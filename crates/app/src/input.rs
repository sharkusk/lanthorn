//! Input → `Action` mapping and application.
//!
//! # Focus routing
//! `key_to_action` applies bindings in this strict precedence order:
//! 1. Ctrl+Q / Ctrl+C → Quit (always wins, even during a prompt).
//! 2. Prompt active → route to prompt only; all other keys absorbed as None.
//! 3. Tidy-anim sub-mode → KeyMap lookup in Anim context; no fallthrough.
//! 4. Saves-manager sub-mode → saves_key_to_action.
//! 6. Hotkey dialog open → hotkey_dialog_key_to_action.
//!    6.7. Ctrl+A/E/U/K/W at a live story prompt (Game focus, not char_mode/event_wait)
//!    → readline caret/delete ops on the input line.
//! 7. Key == hotkeys.prefix → OpenHotkeyDialog.
//! 8. Tab (no modifiers) → autocomplete-or-ToggleFocus special case.
//! 9. Ctrl modifier → Global KeyMap lookup, filtered by hotkeys.is_direct.
//! 10. Per-focus routing:
//!     - Game: game_key_to_action, then Global fallthrough.
//!     - Map: Map context lookup, filtered by hotkeys.is_direct (direct commands only).
//!
//! The former bottom-bar text-entry prompts are now the `text_entry` /
//! `confirm_delete_save` modals, whose key/mouse input the run-loop intercepts in
//! `main.rs` own directly (like the save-name dialog). This module only OPENS them
//! and provides `apply_text_entry` for the submit. (SQ-0307)
//!
//! # Caller-handled actions
//! `apply_action` handles view/light-correction actions in-process.  The
//! following actions are LEFT FOR THE CALLER (the run loop) to handle and are
//! silently ignored by `apply_action`:
//!   - `SubmitCommand` — caller sends text to the Z-machine.
//!   - `SaveGame` / `RestoreGame` — caller performs I/O.
//!   - `ExportSvg` — caller writes file.
//!   - `Quit` — caller exits the event loop.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use mapper::direction::Direction;
use mapper::mapper::Mapper;

use crate::complete::suggest;
use crate::keymap::{Context, KeySpec};
use crate::state::{AppState, Focus, TextEntryDialog, TextEntryKind};


// ── ResizeNavKind ─────────────────────────────────────────────────────────────

/// Navigation kind for `Action::ResizeNav`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeNavKind {
    /// Switch to the next visible pane (Tab).
    NextTarget,
    /// Switch to the previous visible pane (Shift+Tab).
    PrevTarget,
    /// Shrink the target horizontally (or its width).
    Left,
    /// Grow the target horizontally (or its width).
    Right,
    /// Grow the target vertically (or its height).
    Up,
    /// Shrink the target vertically (or its height).
    Down,
}

// ── Action enum ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Caller: submit the contained command string to the Z-machine.
    ///
    /// The command band composes directly onto `state.input` (SQ-0667,
    /// 2026-08-05) rather than a band-local phrase line, so a composed
    /// command reaches the game through this SAME action as anything typed
    /// by hand — there is no longer a separate `SubmitText` path that skips
    /// the input line (that split existed only because the phrase used to
    /// live apart from it).
    SubmitCommand(String),
    /// Append a character to `state.input`.
    InputChar(char),
    /// Delete the last character from `state.input`.
    Backspace,
    /// Toggle between Game and Map focus.
    ToggleFocus,
    /// Reverse of `ToggleFocus`: cycle window focus one step backward (Shift-Tab).
    CycleFocusBack,
    /// Toggle the map panel on/off (Split ↔ TranscriptFull).
    ToggleMap,
    /// Re-tidy the Auto layout: re-derive room positions (sort) then clean overlaps.
    /// No-op in Manual mode (positions are user-controlled and frozen).
    Retidy,
    /// Re-read style.toml and swap the live colors/symbols (keeps current look on error).
    ReloadStyle,
    /// Toggle the opt-in style.toml file-watcher (handled in the run loop).
    ToggleWatch,
    /// Run the tidy pipeline and start animated playback of its stages (Auto only).
    AnimateTidy,
    /// Step the tidy animation by N frames (negative = back); pauses playback.
    AnimStep(i32),
    /// Toggle the tidy animation between playing and paused.
    AnimTogglePlay,
    /// Exit tidy-animation playback back to the live map.
    AnimExit,
    /// Jump to the next (+1) or previous (-1) stage_start frame in the animation.
    AnimStageJump(i32),
    /// Move the input caret one char left.
    CursorLeft,
    /// Move the input caret one char right — or, at the end of the line with a suggestion
    /// showing, accept that suggestion (SQ-0354). See `apply_action`.
    CursorRight,
    /// Move the input caret to the start of the line.
    CursorHome,
    /// Move the input caret to the end of the line.
    CursorEnd,
    /// Delete the char AT the caret (Backspace deletes the one before it).
    DeleteChar,
    /// Delete from the start of the input line to the caret (readline Ctrl+U).
    DeleteToStart,
    /// Delete from the caret to the end of the input line (readline Ctrl+K —
    /// free at the story prompt now that the leader-dialog prefix is Ctrl+P).
    DeleteToEnd,
    /// Delete the word behind the caret, readline style (Ctrl+W).
    DeleteWordBack,
    /// Put the input caret on the char under a click at this screen column/row.
    CursorToClick(u16, u16),
    /// Zoom the map in one VISIBLE step (more detail). A keypress must move the map.
    ZoomIn,
    /// Zoom the map out one VISIBLE step (less detail).
    ZoomOut,
    /// Zoom by `n` VISIBLE steps: positive in (toward Boxes), negative out (SQ-0355).
    ///
    /// The `zoom-map <n>` form. `ZoomIn`/`ZoomOut` are the one-step keys; this carries the
    /// magnitude the command's usage string has always promised.
    ZoomBy(i32),
    /// Zoom in one FINE step — the wheel's gesture (SQ-0350).
    ///
    /// Three fine steps make one visible step, so a fast ctrl+scroll cannot skip past the middle
    /// view. The keyboard uses `ZoomIn`/`ZoomOut` instead: one press, one visible change.
    ZoomInFine,
    /// Zoom out one FINE step — the wheel's gesture. See `ZoomInFine`.
    ZoomOutFine,
    /// Reset zoom to the default level (Boxes) and clear the char-pan offset.
    ZoomReset,
    /// Pan the map scroll by (dx, dy).
    Pan(i32, i32),
    /// Re-center the map on the selected room.
    Recenter,
    /// Select the next room in sorted order.
    SelectNext,
    /// Select the previous room in sorted order.
    SelectPrev,
    /// Begin a rename-room prompt for the selected room.
    RenameRoom,
    /// Begin a rename-layer prompt for the active layer.
    RenameLayer,
    /// Begin an edit-notes prompt for the selected room.
    EditNotes,
    /// Delete the first outgoing connection of the selected room.
    DeleteSelectedConnection,
    /// Begin a relabel-edge prompt for the first outgoing connection of the
    /// selected room.
    RelabelSelectedEdge,
    /// Caller: save the game.
    SaveGame,
    /// Caller: restore a saved game.
    RestoreGame,
    /// Caller: export the map as SVG. `Some(dest)` is the optional `[file]` arg.
    ExportSvg(Option<String>),
    /// Caller: export the map as a Graphviz DOT graph. `Some(dest)` is the optional `[file]` arg.
    ExportDot(Option<String>),
    /// Caller: write an annotatable text/ASCII map dump. `Some(dest)` is the optional `[file]` arg.
    ExportMap(Option<String>),
    /// Toggle the in-box alignment code overlay (palette-only since SQ-0446).
    ToggleAlignment,
    /// Toggle portal destination name labels beside in-room portal icons
    /// (dialog-only, leader letter `l`).
    TogglePortalLabels,
    /// Toggle room-number (#id) visibility in Boxes-zoom room boxes.
    ToggleRoomNumbers,
    ToggleStatusBar,
    /// Toggle honoring the Z-machine's timed-input (`read`/`read_char` timers).
    ToggleTimedInput,
    /// Toggle audio playback (config.enable_sound).
    ToggleSound,
    /// Set the master audio volume 0..=100 (config.volume).
    SetVolume(u8),
    /// Show the room dock's DIAGNOSTICS body: opens the dock there when it is
    /// closed, and flips Info ↔ Diagnostics when it is already open (SQ-0692).
    /// Still spelled `/toggle-inspector` — the command kept its name when the
    /// floating inspector became the dock's second view.
    ToggleRoomDiagnostics,
    /// Caller: exit the application.
    Quit,
    /// Cycle the viewed layer by `delta` steps over the sorted non-empty layer list (clamped at ends).
    CycleLayer(i32),
    /// Select a specific layer as the viewed one (a click on its map layer tab).
    SetViewedLayer(mapper::layer::LayerId),
    /// Re-home the selected (or current) room's region onto a layer (SQ-0439). Carries
    /// `move-region`'s raw argument — `<new|parent|layer> [direction]` — because only the live
    /// layer list can say where a name with spaces in it ends and a seam direction begins.
    /// This one action is the whole of the retired `peel-layer` and `merge-layer`.
    MoveRegion(String),
    /// Set the active layer's map view (SQ-0666). `None` cycles drawn ⇄ matrix.
    ViewMap(Option<mapper::layer::MapView>),
    /// Toggle the maze flag on the active layer (SQ-0666).
    MarkMazeLayer,
    /// Move the matrix view's row selection by `delta` rows, scrolling to keep it visible
    /// (SQ-0666). Saturating: `i32::MIN`/`i32::MAX` are Home/End. A no-op on a drawn layer.
    MatrixMove(i32),
    /// Scroll the matrix view sideways by `delta` direction columns; the label column stays
    /// pinned. Only reachable at the narrowest density, where the table cannot be read across.
    MatrixPanColumns(i32),
    /// Advance autocomplete to the next suggestion, applying the current one to
    /// the input buffer (game focus, Tab key — only when a partial word is being
    /// typed AND suggestions are available; otherwise Tab keeps its ToggleFocus
    /// role).
    Autocomplete,
    /// Step autocomplete to the previous suggestion, applying the current one to
    /// the input buffer (game focus, Shift-Tab key — the reverse of `Autocomplete`,
    /// active under the same mid-word-with-suggestions condition; otherwise
    /// Shift-Tab keeps its `ToggleFocus` role).
    AutocompletePrev,
    /// Recall the previous (older) command into the input buffer (game focus, Up).
    HistoryPrev,
    /// Recall the next (newer) command into the input buffer (game focus, Down).
    HistoryNext,
    /// Open the hotkey dialog overlay.
    OpenHotkeyDialog,
    /// Close the hotkey dialog overlay.
    CloseHotkeyDialog,
    /// Open the command palette popup (SQ-0419). `from_hotkey` = promoted from the
    /// leader dialog by pressing `/` (Esc returns there); otherwise promoted from
    /// the story prompt or opened cold in a modal/debug view.
    OpenCommandPalette { from_hotkey: bool },
    /// Move the palette selection by `delta` rows, wrapping at the ends.
    PaletteNav(i32),
    /// Append a character to the palette input line.
    PaletteChar(char),
    /// Delete the char before the palette caret.
    PaletteBackspace,
    /// Complete the palette input's first token to the selected command's name
    /// (Tab), preserving any typed arguments.
    PaletteComplete,
    /// Close the palette without executing (Esc / [X] / outside click). Returns to
    /// the hotkey dialog when the palette was promoted from it.
    PaletteClose,
    /// A mouse-wheel notch over whichever selection-list modal is open: scroll
    /// its viewport by `delta` rows and clamp the cursor into the visible
    /// window (SQ-0831). Deliberately ONE action for every list rather than a
    /// wheel twin per modal — the rule is the same everywhere, and it lives in
    /// `ListScroll::scroll_by`. The wheel is not a `*Nav`: a nav key moves the
    /// cursor and the window follows, a notch moves the window and the cursor
    /// rides its top or bottom edge (so it never scrolls off screen), and a
    /// list that fits its window does not move at all.
    ListWheel(i32),
    /// Open the saves-manager modal (loads the save list).
    OpenSaves,
    /// Navigate the saves list by delta (-1 = up, +1 = down).
    SavesNav(i32),
    /// Page the saves list by one viewport (-1 = PageUp, +1 = PageDown).
    SavesPage(i32),
    /// Jump to the first save entry.
    SavesHome,
    /// Jump to the last save entry.
    SavesEnd,
    /// Load the selected save (caller-handled).
    SavesLoad,
    /// Begin a SaveAs prompt for a new named save (sets up the prompt sub-mode).
    SavesSaveAs,
    /// Begin a confirm-delete prompt for the selected save (sets up the prompt sub-mode).
    SavesDelete,
    /// Close the saves-manager modal without acting.
    SavesClose,
    /// Navigate the VFS file-picker list by delta (-1 = up, +1 = down).
    FilePickerNav(i32),
    /// Pick the selected VFS filename (caller-handled).
    FilePickerPick,
    /// Close the VFS file-picker modal without picking.
    FilePickerClose,
    /// Toggle the inventory panel.
    ToggleInventory,
    /// Open a confirmation prompt to reset the game to its opening state (keeps map).
    ResetGame,
    /// Open the bottom command band (its object columns fill from the engine's
    /// live object tree on the next tick).
    OpenCommandBand,
    /// Cycle the story pane's border control: command panel → inventory panel →
    /// none → command panel (SQ-1237). The two panels are mutually exclusive,
    /// so opening one closes the other.
    CyclePanel,
    /// `Tab` (`+1`) / `Shift-Tab` (`-1`) while the band is open and nothing is
    /// highlighted in the current column: step `focus` across the reachable
    /// columns (SQ-0677, 2026-08-05 — supersedes SQ-0676's arrow-drives-quick
    /// scheme). Pure movement, never a pick — see `Action::BandClickRow` for
    /// what Tab does INSTEAD when a row IS highlighted.
    BandColumnStep(i32),
    /// `↑` (`-1`) / `↓` (`+1`) while the band is open: move (or start) the
    /// explicit row highlight within the current column (SQ-0677).
    BandRowNav(i32),
    /// `PageUp` (`-1`) / `PageDown` (`+1`) while the band is open: page the
    /// explicit row highlight within the current column by ~one viewport
    /// (SQ-0682) — the band adopts the same PageUp/PageDown the story picker
    /// and IFDB search modal already have.
    BandRowPage(i32),
    /// `Home` while the band is open: jump the explicit row highlight to the
    /// first item of the current column (SQ-0682).
    BandRowHome,
    /// `End` while the band is open: jump the explicit row highlight to the
    /// last item of the current column (SQ-0682).
    BandRowEnd,
    /// Pick the row at `(column, index)` directly (mouse click on a row).
    BandClickRow(usize, usize),
    /// `Tab` with a row highlighted in the current column (SQ-0677): pick
    /// `(column, index)` exactly like `Action::BandClickRow`, but FIRST strip
    /// the partial word under construction at the prompt (mirroring
    /// `apply_completion`'s word-replace) — so completing `ta` to `take`
    /// lands as `take`, not `ta take`. A mouse click has no "word under
    /// construction" relationship to what it picks, so it keeps the plain
    /// `BandClickRow` compose with no stripping.
    BandTabPick(usize, usize),
    /// Point the band at the given column (mouse click on its header).
    BandFocusCol(usize),
    /// Wheel over a column: scroll it by `delta` rows.
    BandWheel(usize, i32),
    /// Pick the quick row's word at this index. Unlike every other pick, this
    /// fires the command AT ONCE — no Enter (SQ-0667 amendment, 2026-08-05
    /// to decision 2's "always confirm"). Caller-handled: resolving the word
    /// and submitting it needs the session, which `apply_action` does not
    /// have, so the run loop does it (`band_quick_pick_command` +
    /// `Action::SubmitCommand`'s shared submit arm). It leaves any
    /// in-progress phrase untouched — a quick pick is an interjection, not a
    /// composition step, and composes nothing onto the input line either.
    BandQuickPick(usize),
    /// Pick the inventory dock's item at this row index (mouse click, SQ-1244):
    /// composes `AppState::inventory_click_words[idx]` onto the prompt exactly
    /// as a click on the command band's WHAT column composes an item's word —
    /// same one-space rule, same partial-word replacement — via
    /// `compose_word_onto_prompt`, the same low-level composer
    /// (`sync_band_phrase_to_input`) `band_pick_row` itself calls. The command
    /// band is closed whenever the inventory panel shows (the two are
    /// mutually exclusive, `SidePanel`), so there is no `CommandBandState` to
    /// pick FROM — a typed verb stays and the item is simply appended.
    InventoryClickRow(usize),
    /// Esc, one level per press: disarm the quick highlight → close the band
    /// (SQ-0676 — the filter rung retired with type-to-filter, and the phrase
    /// rung with it: the phrase is the prompt's text now, and Esc must never
    /// eat what the player typed).
    BandEscape,
    /// Close the band, leaving `state.input` intact.
    BandClose,
    /// Enter interactive pane-resize mode, selecting the first visible pane.
    ResizePanes,
    /// Exit resize mode without resetting sizes.
    ResizeExit,
    /// Reset all pane sizes to their config defaults.
    ResizeReset,
    /// Navigate resize mode: `Tab`/`Shift+Tab` switches the target pane; arrows adjust it.
    ResizeNav(ResizeNavKind),
    /// Open the config screen modal.
    OpenConfig,
    /// Navigate the config screen by delta (-1 = up, +1 = down).
    ConfigNav(i32),
    /// Page the config screen list by one viewport (-1 = PageUp, +1 = PageDown).
    ConfigPage(i32),
    /// Jump to the first config row.
    ConfigHome,
    /// Jump to the last config row.
    ConfigEnd,
    /// Toggle the selected bool field in the working config.
    ConfigToggle,
    /// Cycle an enum/choice field in the working config by delta (-1 or +1).
    ConfigCycle(i32),
    /// Begin text-editing the selected path field.
    ConfigEdit,
    /// Save the working config: apply to state.config, re-resolve symbols/colors, write file.
    ConfigSave,
    /// Cancel the config screen without saving.
    ConfigCancel,
    /// Open the file browser in PickFile mode to import a .qzl/.sav file.
    SavesImport,
    /// Navigate the file browser by delta (-1 = up, +1 = down).
    FbNav(i32),
    /// Page the file-browser list by one viewport (-1 = PageUp, +1 = PageDown).
    FbPage(i32),
    /// Jump to the first file-browser entry.
    FbHome,
    /// Jump to the last file-browser entry.
    FbEnd,
    /// Activate the selected file-browser entry (cd into dir or import file).
    FbEnter,
    /// Close the file browser without acting.
    FbClose,
    /// No binding found — no-op.
    None,
    // ── Mouse actions ─────────────────────────────────────────────────────────
    /// Activate a specific pane (left-click on pane background).
    ActivatePane(crate::state::Focus),
    /// Pin the room dock to `RoomId` in the given view, opening it if closed
    /// (SQ-0692). Pinning IS selecting: `selected_room` is the pin.
    PinRoomDock(mapper::graph::RoomId, crate::state::RoomDockView),
    /// Unpin the room dock — clear the selection so it follows the player again.
    /// The dock itself stays up.
    UnpinRoomDock,
    /// Clear the highlighted route without dropping the selection (SQ-0693) —
    /// Esc's FIRST rung whenever a route is on screen.
    ClearRoomPath,
    /// Close the room dock (Esc's second rung; the toggle command's off state).
    CloseRoomDock,
    /// Open the room dock in the Info view, or close it if already open.
    ToggleRoomDock,
    /// Show a specific room-dock body — a click on one of its two view tabs.
    SetRoomDockView(crate::state::RoomDockView),
    /// Begin a middle-button drag-pan gesture at terminal cell (col, row).
    BeginDragPan(u16, u16),
    /// Continue a middle-button drag-pan gesture at terminal cell (col, row).
    DragPanTo(u16, u16),
    /// End a middle-button drag-pan gesture.
    EndDragPan,
    /// Begin a story-pane text selection at terminal cell (col, row).
    StartSelection(u16, u16),
    /// Extend the story-pane text selection to terminal cell (col, row).
    ExtendSelection(u16, u16),
    /// End the story-pane text selection (copies it to the clipboard).
    EndSelection,
    /// Scroll the transcript by delta lines (positive = down, negative = up).
    TranscriptScroll(i32),
    /// Page the transcript by one screenful (PageUp/PageDown). `+1` scrolls toward
    /// older lines, `-1` toward newer. Resolved by the run loop, which knows the
    /// last-rendered transcript viewport height and max scroll (see `page_scroll`).
    TranscriptScrollPage(i8),
    /// Half-page the transcript (Ctrl-D/Ctrl-U, vim convention; SQ-1228). `+1`
    /// scrolls toward older lines, `-1` toward newer — same sign convention as
    /// `TranscriptScrollPage`, resolved the same way (see `half_page_scroll`).
    /// Ctrl-D always means half-page down. Ctrl-U means half-page up only when
    /// the story prompt's input line is empty; with text on the line, Ctrl-U
    /// keeps its readline meaning of "delete to start of line"
    /// (`Action::DeleteToStart`), which wins.
    TranscriptScrollHalfPage(i8),
    /// Advance the `[more]` pager one screen toward the bottom; catching up exits
    /// the pager (SQ-0404).
    PagerAdvance,
    /// Dismiss the `[more]` pager, jumping straight to the newest output (SQ-0404).
    PagerDismiss,
    /// Open the Hints panel. Real behavior wired in Task D; stub here keeps match exhaustive.
    OpenHints,
    /// Open the rewind/replay history modal (seeds `replay` at the last turn).
    OpenHistory,
    /// Step the replay selection by delta turns (-1 left, +1 right).
    ReplayStep(isize),
    /// Page the replay selection by one viewport (-1 = PageUp, +1 = PageDown).
    ReplayPage(i8),
    /// Toggle replay auto-play.
    ReplayTogglePlay,
    /// Close the replay modal (back to live, no change).
    ReplayClose,
    /// Resume the live game from the selected turn (caller-handled in main.rs).
    ReplayResume,
}

// ── key_to_command ────────────────────────────────────────────────────────────

/// The result of resolving a `KeyEvent` against the current `AppState`.
///
/// Hardwired keys, modal sub-modes, and per-focus text entry resolve directly
/// to an `Action`. KeyMap lookups resolve to a command-string plus the context
/// it was looked up in, so the run loop can dispatch it through
/// `slash::parse_in_context` exactly as if the user had typed the command.
#[derive(Debug)]
pub enum KeyResolve {
    /// A hardwired / modal / text-entry action to apply directly.
    Action(crate::input::Action),
    /// A keymap-resolved command string to dispatch through the slash parser,
    /// together with the context it was resolved in.
    Command(String, crate::keymap::Context),
    /// The key produced nothing.
    None,
}

/// Resolve a crossterm `KeyEvent` to a `KeyResolve` given the current `AppState`.
///
/// Routing order:
/// 1. Ctrl+Q / Ctrl+C → Quit (hardwired, always wins).
/// 2. (text-entry / confirm-delete modals are intercepted in the run loop.)
/// 3. Tidy-anim active → Anim context lookup (Ctrl+Left/Right stage-jump hardwired).
///    4-6. Modal sub-modes (saves/replay/file-browser/verb-menu/
///    config-screen/hotkey-dialog/room-panel) → their handlers (hardwired Actions).
///    6.7. Ctrl+A/E/U/K/W in Game focus with the line prompt live (not char_mode/
///    event_wait) → readline caret/delete ops on the input line.
///    6.8. Ctrl+D in Game focus → TranscriptScrollHalfPage(-1) (SQ-1228, half
///    page down). Ctrl+U in Game focus → TranscriptScrollHalfPage(1) (half
///    page up) when the input line is EMPTY, else falls through to 6.7's
///    DeleteToStart, which wins whenever there's something to delete. Both are
///    DEFAULTS, not hardwires: either only fires when step 9's Global keymap
///    lookup would find no binding for the key — a user's own Ctrl+D/Ctrl+U
///    binding always wins, so this block falls through to step 9 when one
///    exists.
/// 7. Key == hotkeys.prefix → OpenHotkeyDialog.
/// 8. Tab (no modifiers) → autocomplete-or-ToggleFocus.
/// 9. Ctrl modifier → Global KeyMap lookup, filtered by hotkeys.is_direct_name.
/// 10. Per-focus routing:
///     - Game: game_key_to_action, then Global fallthrough (non-ctrl non-printable).
///     - Map: Map context lookup, filtered by hotkeys.is_direct_name.
pub fn key_to_command(state: &AppState, key: KeyEvent) -> KeyResolve {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // 1. Quit always wins — even while a prompt is active.
    if ctrl && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('c')) {
        return KeyResolve::Action(Action::Quit);
    }

    // 2. The text-entry / confirm-delete modals are intercepted in the run loop
    //    before key routing (like the save-name dialog), so they never reach here.

    // 3. Tidy-animation sub-mode: KeyMap lookup in Anim context; no fallthrough.
    if state.tidy_anim.is_some() {
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Left => return KeyResolve::Action(Action::AnimStageJump(-1)),
                KeyCode::Right => return KeyResolve::Action(Action::AnimStageJump(1)),
                _ => {}
            }
        }
        let spec = KeySpec::from_key_event(key);
        return match state.keymap.lookup(&spec, Context::Anim) {
            Some(s) => KeyResolve::Command(s.to_string(), Context::Anim),
            None => KeyResolve::None,
        };
    }

    // 4-6. Modal sub-modes: route to their handlers (all hardwired Actions).
    if state.overlays.saves.is_some() {
        return KeyResolve::Action(saves_key_to_action(key, state.overlays.dialog_focus));
    }
    if state.overlays.file_picker.is_some() {
        return KeyResolve::Action(file_picker_key_to_action(key));
    }
    if state.overlays.replay.is_some() {
        return KeyResolve::Action(history_key_to_action(key));
    }
    if state.overlays.file_browser.is_some() {
        return KeyResolve::Action(filebrowser_key_to_action(key));
    }
    // 6.5b. Command palette popup (SQ-0419): owns all keys while open. Typing
    // filters; Up/Down (+ Shift-Tab reverse) move the selection; Tab completes the
    // selected name; Enter executes it (with any typed args) through the slash
    // dispatch path; Esc closes. Placed at the top of the modal ladder because it
    // can be summoned over any other view (incl. the debug pane where no prompt
    // exists).
    if state.overlays.palette.is_some() {
        return palette_key_to_command(state, key);
    }
    // 6.6. Resize mode: Tab cycles the target pane, arrows adjust it, 0 resets,
    // Esc/Enter exits. Placed ABOVE the verb-menu intercept (SQ-0238) so resize
    // mode owns Tab/arrows/0/Esc/Enter even when the verb menu is open — the two
    // now coexist, with resize targeting the verb dock and Esc/Enter dropping
    // back to the still-open menu. (config_screen/hotkey_dialog below can never
    // be active together with resize mode, so their relative order is moot.)
    if state.resize_mode {
        return KeyResolve::Action(resize_mode_key_to_action(key));
    }
    // SQ-1236: a modal dialog opened OVER the band (config_screen, hotkey_dialog
    // — the two modal checks below this one) takes all input; the band underneath
    // must intercept nothing until the dialog closes, so it stays gated on
    // `!any_modal_overlay_open()` rather than only on being open itself. The band
    // itself is excluded from `any_modal_overlay_open` (it's a dock, not dialog
    // chrome — see that method's doc comment), so this reads as "band, unless
    // something ACTUALLY modal is stacked on top of it."
    if state.overlays.command_band.is_some() && !state.any_modal_overlay_open() {
        if let Some(a) = command_band_intercept(key, state) {
            return KeyResolve::Action(a);
        }
        // else: fall through — the story input is live, so the key is handled normally.
    }
    if state.overlays.config_screen.is_some() {
        return KeyResolve::Action(config_screen_key_to_action(key, state.overlays.dialog_focus));
    }
    if state.overlays.hotkey_dialog {
        return hotkey_dialog_key_to_action(state, key);
    }

    // 6.5. Room dock Esc ladder (SQ-0692, extended by SQ-0693): drop the route
    // first, then unpin, then close on the next press.
    // Comes after steps 2-6 (prompt/anim/saves/hotkey_dialog checks) so those
    // modes still take priority, but before the prefix key and normal dispatch.
    //
    // Three rungs because there are three states worth leaving: a highlighted
    // route to the room you clicked, the pin on that room, and the dock simply
    // being up. Esc walks out of them in the order you entered them, so one habit
    // ("Esc backs out") covers all of them, and none needs a second key. Clearing
    // the route WITHOUT dropping the selection is the point of the extra rung:
    // the room stays selected and its entrances stay bold, so "stop shouting the
    // way there" and "stop looking at this room" are two separate thoughts.
    //
    // Enter is deliberately NOT a close key. The dock is not a modal — typing
    // reaches the story prompt with it open (a letter resolves to `InputChar`),
    // so stealing Enter would let you compose a command and never submit it.
    if (state.room_dock.open || !state.room_path.is_empty())
        && key.modifiers == KeyModifiers::NONE
        && matches!(key.code, KeyCode::Esc) {
            return KeyResolve::Action(if !state.room_path.is_empty() {
                Action::ClearRoomPath
            } else if state.room_dock_pinned() {
                Action::UnpinRoomDock
            } else {
                Action::CloseRoomDock
            });
        }

    // [more] pager (SQ-0404): while it's showing, keypresses page the transcript
    // instead of reaching the game. Space/PgDn/↓/Enter advance one screen; any
    // other key jumps to the bottom and dismisses.
    //
    // …except with a `read_char` pending (SQ-0539): there, EVERY key advances one
    // screen. Jumping to the bottom would skip output the player never saw — the
    // one thing the pager exists to prevent — and the game is waiting on a single
    // keypress, so the original interpreters' rule applies: the key is consumed by
    // the [MORE] prompt, and only after the view catches up (pager inactive) does
    // a key reach the game (see the char-input gate in main.rs).
    if state.pager.active {
        return match key.code {
            KeyCode::Char(' ') | KeyCode::PageDown | KeyCode::Down | KeyCode::Enter => {
                KeyResolve::Action(Action::PagerAdvance)
            }
            _ if state.char_mode => KeyResolve::Action(Action::PagerAdvance),
            _ => KeyResolve::Action(Action::PagerDismiss),
        };
    }

    // Computed here (ahead of step 7's own use) because step 6.8 below also
    // needs it, to check whether a user keymap binding shadows its default.
    let spec = KeySpec::from_key_event(key);

    // 6.7. Readline-style line-edit shortcuts at the story prompt (SQ-0447):
    // Ctrl+A/E/U/K/W act on the input line instead of falling through to the
    // generic Ctrl handling in step 9. Gated to Game focus with the line prompt
    // actually live — NOT during char_mode/event_wait, which hide the input line
    // and (per main.rs's char-input gate) route Ctrl combos to app dispatch
    // instead. Placed ahead of step 7 too: Ctrl+K used to be the hotkey-dialog
    // prefix, but that moved to Ctrl+P, freeing Ctrl+K for delete-to-end here.
    //
    // Ctrl+U is DeleteToStart only while there's text to delete (SQ-1228): an
    // empty input line falls through — unhandled here — to step 6.8's vim
    // half-page-up, which is the meaning readline's kill-line has nothing to
    // contest on an empty line.
    if key.modifiers == KeyModifiers::CONTROL
        && state.focus == Focus::Game && !state.char_mode && !state.event_wait
    {
        match key.code {
            KeyCode::Char('a') => return KeyResolve::Action(Action::CursorHome),
            KeyCode::Char('e') => return KeyResolve::Action(Action::CursorEnd),
            KeyCode::Char('u') if !state.input.is_empty() => {
                return KeyResolve::Action(Action::DeleteToStart);
            }
            KeyCode::Char('k') => return KeyResolve::Action(Action::DeleteToEnd),
            KeyCode::Char('w') => return KeyResolve::Action(Action::DeleteWordBack),
            // Ctrl+↑/↓ recall command history (SQ-0677): plain ↑/↓ moved to
            // the command band's row navigation while it's open, so history
            // needs a modifier'd alias to stay reachable there too. With the
            // band CLOSED this is simply an alias for the plain arrows below
            // (`game_key_to_action`'s `KeyCode::Up/Down if modifiers ==
            // NONE`) — harmless, and one less thing to remember.
            KeyCode::Up => return KeyResolve::Action(Action::HistoryPrev),
            KeyCode::Down => return KeyResolve::Action(Action::HistoryNext),
            _ => {}
        }
    }

    // 6.8. Ctrl+D in Game focus: half-page the transcript toward newer lines
    // (SQ-1228, the vim convention). Ctrl+U does the same toward older lines,
    // but only when the input line is EMPTY — with text on the line, step 6.7
    // above already returned `Action::DeleteToStart` for it, which wins.
    //
    // This is a DEFAULT, not a hardwire: unlike step 6.7's readline block, a
    // user's own keymap binding for the key always wins. `window_dump_bound_key`
    // (SQ-0759) binds Ctrl+D to `dump-windows` and requires it dispatch that
    // command, not this built-in scroll — so this block only fires the
    // default when `state.keymap` has NO Global binding for the key at all;
    // otherwise it falls through to step 9 below, which resolves (or rejects)
    // that binding the same way every other Ctrl combo does. Not gated to
    // `!char_mode && !event_wait` the way the readline block above is: a Ctrl
    // combo is never forwarded to the VM as game input (see the char-mode gate
    // in main.rs), so there is no game meaning for this key to preempt, and
    // scrolling the transcript is exactly as useful mid-menu as at the prompt.
    if key.modifiers == KeyModifiers::CONTROL && state.focus == Focus::Game {
        let half_page_down = key.code == KeyCode::Char('d');
        let half_page_up = key.code == KeyCode::Char('u') && state.input.is_empty();
        if (half_page_down || half_page_up) && state.keymap.lookup(&spec, Context::Global).is_none() {
            let dir = if half_page_down { -1 } else { 1 };
            return KeyResolve::Action(Action::TranscriptScrollHalfPage(dir));
        }
    }

    // 7. Prefix key → open the hotkey dialog.
    if spec == state.hotkeys.prefix {
        return KeyResolve::Action(Action::OpenHotkeyDialog);
    }

    // 8. Tab (no modifiers): stateful autocomplete-or-ToggleFocus (hardwired).
    if key.modifiers == KeyModifiers::NONE && key.code == KeyCode::Tab {
        // Autocomplete takes priority over focus-toggle when: game is focused,
        // the player is mid-word (non-empty partial), AND suggestions exist.
        // In all other cases Tab keeps its existing ToggleFocus behaviour.
        if state.focus == Focus::Game
            && !state.current_partial().is_empty()
            && !state.suggestions.is_empty()
        {
            return KeyResolve::Action(Action::Autocomplete);
        }
        return KeyResolve::Action(Action::ToggleFocus);
    }

    // 8b. Shift-Tab (BackTab): mid-word with suggestions in game focus →
    //     AutocompletePrev (the reverse of step 8's Autocomplete). Otherwise it
    //     reverses the per-window focus cycle (the mirror of step 8's ToggleFocus).
    if key.code == KeyCode::BackTab
        && state.focus == Focus::Game
        && !state.current_partial().is_empty()
        && !state.suggestions.is_empty()
    {
        return KeyResolve::Action(Action::AutocompletePrev);
    }
    if key.code == KeyCode::BackTab {
        return KeyResolve::Action(Action::CycleFocusBack);
    }

    // 9. Ctrl modifier: Global KeyMap lookup, filtered by is_direct_name — same
    //    rule as Map context. A command is reachable directly iff it is in the
    //    direct set, regardless of whether it uses a Ctrl modifier.
    if ctrl {
        return match state.keymap.lookup(&spec, Context::Global) {
            Some(s) if state.hotkeys.is_direct_name(s) => {
                KeyResolve::Command(s.to_string(), Context::Global)
            }
            _ => KeyResolve::None,
        };
    }

    // 10. Per-focus routing.
    match state.focus {
        Focus::Game => {
            // Text entry is hardwired (printable chars, Enter, Backspace, Shift+Arrows,
            // Home, PageUp/Down). Non-printable / unmatched keys fall through to a
            // Global KeyMap lookup so that non-ctrl global bindings reach Game focus.
            let a = game_key_to_action(state, key);
            if a != Action::None {
                return KeyResolve::Action(a);
            }
            // Global fallthrough for non-ctrl non-Tab non-printable keys.
            match state.keymap.lookup(&spec, Context::Global) {
                Some(s) => KeyResolve::Command(s.to_string(), Context::Global),
                None => KeyResolve::None,
            }
        }
        Focus::Map => {
            // SQ-0666: with the map focused, the arrows drive the matrix view's row selection and
            // its sideways scroll — the list conventions every other pane already uses. They are
            // hardwired rather than bound, because they are the table's own navigation and not a
            // command a player would think to rebind; and they are emitted unconditionally
            // because `key_to_command` cannot see the graph. On a drawn layer the handler is a
            // no-op, which is exactly what these keys did in map focus before.
            match key.code {
                KeyCode::Up => return KeyResolve::Action(Action::MatrixMove(-1)),
                KeyCode::Down => return KeyResolve::Action(Action::MatrixMove(1)),
                KeyCode::PageUp => return KeyResolve::Action(Action::MatrixMove(-10)),
                KeyCode::PageDown => return KeyResolve::Action(Action::MatrixMove(10)),
                KeyCode::Home => return KeyResolve::Action(Action::MatrixMove(i32::MIN)),
                KeyCode::End => return KeyResolve::Action(Action::MatrixMove(i32::MAX)),
                KeyCode::Left => return KeyResolve::Action(Action::MatrixPanColumns(-1)),
                KeyCode::Right => return KeyResolve::Action(Action::MatrixPanColumns(1)),
                _ => {}
            }
            // Map context lookup with direct filter: only return the command if it
            // is in the direct (always-available) set. Dialog-only commands return
            // None when the dialog is closed.
            match state.keymap.lookup(&spec, Context::Map) {
                Some(s) if state.hotkeys.is_direct_name(s) => {
                    KeyResolve::Command(s.to_string(), Context::Map)
                }
                _ => KeyResolve::None,
            }
        }
    }
}

/// Backward-compatible shim: resolve a key straight to an `Action`.
///
/// Production dispatch consumes `key_to_command` directly so command-strings
/// flow through the slash parser. This wrapper is retained for tests and any
/// caller that only needs the `Action` form: command-strings that parse to a
/// plain `Action` are returned as such; Save/Load/Reset/Quit outcomes (and
/// parse errors) collapse to `Action::None`.
pub fn key_to_action(state: &AppState, key: KeyEvent) -> Action {
    match key_to_command(state, key) {
        KeyResolve::Action(a) => a,
        KeyResolve::Command(s, ctx) => {
            match crate::slash::parse_in_context(&s, state.config.command_prefix, ctx) {
                crate::slash::SlashOutcome::Action(a) => a,
                _ => Action::None,
            }
        }
        KeyResolve::None => Action::None,
    }
}

// ── mouse_to_action ───────────────────────────────────────────────────────────

/// Find the first room whose bounding rect contains screen cell `(col, row)`.
fn room_at_screen(
    room_rects: &[(mapper::graph::RoomId, ratatui::layout::Rect)],
    col: u16,
    row: u16,
) -> Option<mapper::graph::RoomId> {
    room_rects
        .iter()
        .find(|(_, rect)| col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom())
        .map(|(id, _)| *id)
}

/// Test whether (col, row) is inside `rect`.
fn hit(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
}

/// Which room-dock action a mouse event inside the dock's rect means (SQ-0694).
///
/// The dock owns every event inside its own rect: it is carved OUT of the map pane, so a click
/// there is neither a map click nor a story selection and must never fall through to either (nor
/// to the v6 mouse-delivery path). A left-click on one of the two view tabs switches the body —
/// the same gesture, on the same shared tab strip, that switches layers on the map pane's tabs;
/// anything else inside the dock is claimed and does nothing.
///
/// Returns `None` when the event is not inside `dock`, which is the caller's cue to route it
/// normally. `tabs` are the hit-rects `draw_room_dock` returned for the frame just drawn.
pub fn room_dock_mouse_action(
    dock: ratatui::layout::Rect,
    tabs: &[(crate::state::RoomDockView, ratatui::layout::Rect)],
    m: &crossterm::event::MouseEvent,
) -> Option<Action> {
    use crossterm::event::{MouseButton, MouseEventKind};
    if dock.width == 0 || dock.height == 0 || !hit(dock, m.column, m.row) {
        return None;
    }
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
        if let Some(&(view, _)) = tabs.iter().find(|(_, r)| {
            r.width > 0 && r.height > 0 && hit(*r, m.column, m.row)
        }) {
            return Some(Action::SetRoomDockView(view));
        }
    }
    Some(Action::None)
}

/// Per-modal button-to-action mapping for the config screen.
/// Maps a `ButtonId` click (or the close [X] hit) to the appropriate `Action`.
fn config_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::ConfigCancel);
        }
    }

    // Check buttons
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Save   => Action::ConfigSave,
                ButtonId::Cancel => Action::ConfigCancel,
                _                => Action::None,
            });
        }
    }

    None
}

/// Per-modal button-to-action mapping for the saves manager.
fn saves_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::SavesClose);
        }
    }

    // Check buttons: Done → SavesClose
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Done => Action::SavesClose,
                _              => Action::None,
            });
        }
    }

    None
}

/// Per-modal button-to-action mapping for the file browser.
fn filebrowser_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::FbClose);
        }
    }

    // Check buttons: Done → FbClose
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Done => Action::FbClose,
                _              => Action::None,
            });
        }
    }

    None
}

/// Per-modal action mapping for the tidy panel ([X] → AnimExit).
fn tidy_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::AnimExit);
        }
    }

    // Check buttons: Ok → AnimExit
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Ok => Action::AnimExit,
                _            => Action::None,
            });
        }
    }

    None
}

/// Per-modal button-to-action mapping for the hotkey dialog.
fn hotkeys_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::CloseHotkeyDialog);
        }
    }

    // Check buttons: Done → CloseHotkeyDialog
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Done => Action::CloseHotkeyDialog,
                _              => Action::None,
            });
        }
    }

    None
}

/// Map a vertical mouse-wheel event to a step: `ScrollUp` → `-1`, `ScrollDown`
/// → `+1`, swapped when `invert`; `None` for non-wheel events. The single place
/// wheel direction and the `mouse_wheel_invert` preference are resolved, so no
/// surface re-implements the invert.
pub fn wheel_delta(kind: MouseEventKind, invert: bool) -> Option<isize> {
    let base = match kind {
        MouseEventKind::ScrollUp => -1,
        MouseEventKind::ScrollDown => 1,
        _ => return None,
    };
    Some(if invert { -base } else { base })
}

/// Map a crossterm `MouseEvent` to an `Action` given the current `AppState`, the
/// bounding rects of the map and story panes, the pre-computed room screen
/// rects (needed for pixel-accurate room hit-testing on left/right clicks), and
/// the active dialog chrome rects (if a dialog is open).
///
/// When `dialog` is `Some`, dialog hit-testing runs FIRST:
/// - close `[X]` click → the active modal's close action
/// - button click → the button's mapped action
/// - any click OUTSIDE the dialog `area` → swallowed (Action::None)
///   Only when no dialog is open does normal map/room routing apply.
///
/// Returns `Action::None` for events outside both panes or with no binding.
pub fn mouse_to_action(
    state: &AppState,
    m: MouseEvent,
    map: ratatui::layout::Rect,
    story: ratatui::layout::Rect,
    room_rects: &[(mapper::graph::RoomId, ratatui::layout::Rect)],
    dialog: &Option<crate::render::dialog::DialogRects>,
) -> Action {
    let col = m.column;
    let row = m.row;
    let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);
    let shift = m.modifiers.contains(KeyModifiers::SHIFT);

    // Honor the user's wheel-direction preference: when mouse_wheel_invert is
    // set, swap scroll up/down (some terminals report "natural" scrolling).
    // Computed once here and reused by both the modal-precedence branch below
    // and the map/story wheel arms; never invert twice for one event.
    let kind = match (m.kind, state.config.mouse_wheel_invert) {
        (MouseEventKind::ScrollUp, true) => MouseEventKind::ScrollDown,
        (MouseEventKind::ScrollDown, true) => MouseEventKind::ScrollUp,
        (k, _) => k,
    };

    // ── Mouse-wheel precedence for open scrollable modals ─────────────────────
    // When a scrollable overlay is open, the wheel drives THAT surface's vertical
    // scrolling, ahead of the underlying map/story and ahead of the dialog
    // chrome hit-testing below. Corner overlays (room panel, tidy) are
    // intentionally absent — the wheel still pans the map under them, as before.
    //
    // A list modal gets `ListWheel`, not its Up/Down nav action (SQ-0831): the
    // wheel scrolls the LIST and pins the cursor to the visible window, where a
    // nav key moves the cursor and drags the window after it. The replay
    // overlay keeps `ReplayStep` — it is a stepper over replay positions, not a
    // list with a cursor in a viewport.
    // `kind` already has the single mouse_wheel_invert applied (above), so map it
    // to a direction with the shared helper and invert=false (never twice).
    let wheel_up = wheel_delta(kind, false).map(|d| d < 0);
    if let Some(up) = wheel_up {
        // Priority mirrors the keyboard modal routing order above.
        let d = if up { -1 } else { 1 };
        if state.overlays.config_screen.is_some() || state.overlays.saves.is_some() {
            return Action::ListWheel(d);
        }
        if state.overlays.replay.is_some() {
            return Action::ReplayStep(d as isize);
        }
        if state.overlays.file_browser.is_some() {
            return Action::ListWheel(d);
        }
    }

    // ── Dialog chrome hit-testing (checked FIRST) ─────────────────────────────
    if let Some(rects) = dialog {
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            // Corner overlays (room panel, tidy panel): only intercept the [X] click;
            // all other clicks fall through to normal map/room routing below.
            // But if a centered modal is also open (stacked on top), it takes
            // priority and must swallow all outside clicks.
            let centered_open = state.overlays.config_screen.is_some()
                || state.overlays.saves.is_some() || state.overlays.file_browser.is_some()
                || state.overlays.hotkey_dialog;
            let is_corner_overlay = !centered_open && state.tidy_anim.is_some();

            if state.overlays.config_screen.is_some() {
                if let Some(action) = config_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.overlays.saves.is_some() {
                if let Some(action) = saves_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.overlays.file_browser.is_some() {
                if let Some(action) = filebrowser_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.overlays.hotkey_dialog {
                if let Some(action) = hotkeys_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.tidy_anim.is_some() {
                if let Some(action) = tidy_dialog_action(rects, col, row) {
                    return action;
                }
            }

            // Corner overlays: don't swallow other clicks — let normal routing handle them.
            if is_corner_overlay {
                // fall through to normal routing below
            } else {
                // Centered modal: swallow all other clicks.
                return Action::None;
            }
        } else {
            // For non-left-click events (wheel/drag): swallow unless a corner overlay
            // is active and no centered modal is stacked on top.
            let centered_open = state.overlays.config_screen.is_some()
                || state.overlays.saves.is_some() || state.overlays.file_browser.is_some()
                || state.overlays.hotkey_dialog;
            let is_corner_overlay = !centered_open && state.tidy_anim.is_some();
            if !is_corner_overlay {
                return Action::None;
            }
        }
    }

    // ── Normal routing (no dialog open) ──────────────────────────────────────

    let in_map = map.width > 0 && map.height > 0
        && col >= map.x && col < map.right()
        && row >= map.y && row < map.bottom();
    let in_story = story.width > 0 && story.height > 0
        && col >= story.x && col < story.right()
        && row >= story.y && row < story.bottom();

    match kind {
        // ── Left-down on the input line: place the caret ──────────────────────
        // Must precede the story arm below: the input line sits inside the story pane, so a click
        // on it would otherwise start a text selection instead of moving the caret (SQ-0354).
        MouseEventKind::Down(MouseButton::Left)
            if state.focus == Focus::Game && state.input_click_index(col, row).is_some() =>
        {
            Action::CursorToClick(col, row)
        }
        // ── Left-down in story: activate game pane + begin text selection ─────
        MouseEventKind::Down(MouseButton::Left) if in_story => {
            Action::StartSelection(col, row)
        }
        // ── Left-drag: extend an in-progress story selection ──────────────────
        MouseEventKind::Drag(MouseButton::Left) => {
            Action::ExtendSelection(col, row)
        }
        // ── Left-up: finish a story selection (copy on release) ───────────────
        MouseEventKind::Up(MouseButton::Left) => {
            Action::EndSelection
        }
        // ── Left-click in map ─────────────────────────────────────────────────
        // Pin, unpin, follow (SQ-0692). A click on a room points the dock at it
        // (opening the dock if it was closed); a second click on the SAME pinned
        // room, or a click on empty map space, unpins and the dock goes back to
        // following the player. Focus deliberately stays on the story pane so you
        // can keep typing.
        MouseEventKind::Down(MouseButton::Left) if in_map => {
            match room_at_screen(room_rects, col, row) {
                Some(id) if state.room_dock.open && state.selected_room == Some(id) => {
                    Action::UnpinRoomDock
                }
                Some(id) => Action::PinRoomDock(id, crate::state::RoomDockView::Info),
                // Empty map gutter: unpin only. This used to hand the keyboard to
                // the map, which is exactly the invisible mode SQ-0599 removed — a
                // stray click in the gutter would silently redirect every
                // subsequent keystroke away from the story.
                None => Action::UnpinRoomDock,
            }
        }
        // ── Right-click in map ────────────────────────────────────────────────
        // Same gestures, but pointed at the DIAGNOSTICS body. Only a click on a
        // room already pinned AND already showing diagnostics unpins — otherwise
        // the click has somewhere to take you.
        MouseEventKind::Down(MouseButton::Right) if in_map => {
            match room_at_screen(room_rects, col, row) {
                Some(id)
                    if state.room_dock.open
                        && state.selected_room == Some(id)
                        && state.room_dock_view == crate::state::RoomDockView::Diagnostics =>
                {
                    Action::UnpinRoomDock
                }
                Some(id) => Action::PinRoomDock(id, crate::state::RoomDockView::Diagnostics),
                None => Action::UnpinRoomDock,
            }
        }
        // ── Middle-button: drag-pan ───────────────────────────────────────────
        MouseEventKind::Down(MouseButton::Middle) if in_map => {
            Action::BeginDragPan(col, row)
        }
        MouseEventKind::Drag(MouseButton::Middle) => {
            Action::DragPanTo(col, row)
        }
        MouseEventKind::Up(MouseButton::Middle) => {
            Action::EndDragPan
        }
        // ── Wheel in map: pan or zoom ─────────────────────────────────────────
        MouseEventKind::ScrollUp if in_map => {
            if ctrl {
                Action::ZoomInFine
            } else if shift {
                Action::Pan(-1, 0)
            } else {
                Action::Pan(0, -1)
            }
        }
        MouseEventKind::ScrollDown if in_map => {
            if ctrl {
                Action::ZoomOutFine
            } else if shift {
                Action::Pan(1, 0)
            } else {
                Action::Pan(0, 1)
            }
        }
        MouseEventKind::ScrollLeft if in_map => Action::Pan(-1, 0),
        MouseEventKind::ScrollRight if in_map => Action::Pan(1, 0),
        // ── Wheel in story: scroll transcript ────────────────────────────────
        // Wheel up = scroll up into older history; wheel down = toward newest.
        MouseEventKind::ScrollUp if in_story => Action::TranscriptScroll(1),
        MouseEventKind::ScrollDown if in_story => Action::TranscriptScroll(-1),
        // ── Everything else ───────────────────────────────────────────────────
        _ => Action::None,
    }
}

// ── Internal: hotkey dialog key routing ───────────────────────────────────────

/// When the hotkey dialog is open, route keys to either close the dialog or
/// fire the bound command. The dialog closes itself when a sub-mode
/// opens (handled in apply_action).
fn hotkey_dialog_key_to_action(state: &AppState, key: KeyEvent) -> KeyResolve {
    // '/' promotes the leader dialog into the command palette (SQ-0419). Checked
    // before the leader-letter lookup below, which would otherwise treat '/' as an
    // unbound letter and just close the dialog.
    if let KeyCode::Char('/') = key.code {
        if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) {
            return KeyResolve::Action(Action::OpenCommandPalette { from_hotkey: true });
        }
    }

    // ESC or Enter always closes the hotkey dialog (same as [X] / [Done]).
    // Enter is handled before the leader-letter lookup to prevent the
    // Anim/AnimExit binding from firing when the hotkey dialog is open.
    if matches!(key.code, KeyCode::Esc | KeyCode::Enter) && key.modifiers == KeyModifiers::NONE {
        return KeyResolve::Action(Action::CloseHotkeyDialog);
    }

    let spec = KeySpec::from_key_event(key);

    // Prefix key closes the dialog.
    if spec == state.hotkeys.prefix {
        return KeyResolve::Action(Action::CloseHotkeyDialog);
    }

    // A bare character (no Ctrl/Alt; Shift allowed) is an authored leader
    // letter: fire its bound command, or close the dialog if unbound. Any
    // other key (Ctrl-combos, Alt-combos, function keys, arrows, etc.) also
    // closes the dialog. This gives tmux-style one-shot semantics: exactly
    // one keypress always resolves the dialog.
    if let KeyCode::Char(c) = key.code {
        if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) {
            return match state.hotkeys.leader_command(c) {
                Some(cmd) => {
                    let name = cmd.split_whitespace().next().unwrap_or("");
                    let ctx = crate::slash::find_command(name)
                        .map(|c| c.context)
                        .unwrap_or(Context::Global);
                    KeyResolve::Command(cmd.to_string(), ctx)
                }
                None => KeyResolve::Action(Action::CloseHotkeyDialog),
            };
        }
    }

    KeyResolve::Action(Action::CloseHotkeyDialog)
}

// ── Internal: command-palette key routing ─────────────────────────────────────

/// Route a key while the command palette is open (SQ-0419). Typing edits the
/// palette's own input line; Up/Down (and Shift-Tab as the reverse) move the
/// selection; Tab completes the selected command name into the line; Enter
/// resolves to the selected command + typed args as a `Command` for the run loop
/// to dispatch (and then close the palette); Esc closes.
fn palette_key_to_command(state: &AppState, key: KeyEvent) -> KeyResolve {
    let Some(palette) = &state.overlays.palette else {
        return KeyResolve::None;
    };
    match key.code {
        KeyCode::Esc => KeyResolve::Action(Action::PaletteClose),
        KeyCode::Up => KeyResolve::Action(Action::PaletteNav(-1)),
        KeyCode::Down => KeyResolve::Action(Action::PaletteNav(1)),
        // Shift-Tab reverses the Up/Down selection cycler (standing convention).
        KeyCode::BackTab => KeyResolve::Action(Action::PaletteNav(-1)),
        KeyCode::Tab => KeyResolve::Action(Action::PaletteComplete),
        KeyCode::Backspace => KeyResolve::Action(Action::PaletteBackspace),
        KeyCode::Enter => {
            // Execute the highlighted candidate with any typed args through the
            // same slash path a typed command uses.
            let cands = crate::complete::palette_candidates(palette.query());
            match cands.get(palette.scroll.selected) {
                Some(cand) => {
                    let spec = &crate::slash::COMMANDS[cand.cmd_index];
                    KeyResolve::Command(palette.command_line(spec.name), spec.context)
                }
                None => KeyResolve::Action(Action::PaletteClose),
            }
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            KeyResolve::Action(Action::PaletteChar(c))
        }
        _ => KeyResolve::None,
    }
}

// ── Internal: saves-manager key routing ───────────────────────────────────────

/// Hardwired saves-manager sub-mode keys (not rebindable, like prompt and anim).
///
/// `focus` is the current button-focus index within the saves button ring:
///   0 = Done (close). Ring length is 1; the [Done] button is the only button.
/// Tab/BackTab are handled upstream (main.rs intercept) and never reach here.
/// The saves dialog has only one button (Done); Enter continues to load the
/// selected save (existing behavior) rather than activating the focused button,
/// since no Load button exists in the button row.
fn saves_key_to_action(key: KeyEvent, _focus: usize) -> Action {
    match key.code {
        KeyCode::Up => Action::SavesNav(-1),
        KeyCode::Down => Action::SavesNav(1),
        KeyCode::PageUp => Action::SavesPage(-1),
        KeyCode::PageDown => Action::SavesPage(1),
        KeyCode::Home => Action::SavesHome,
        KeyCode::End => Action::SavesEnd,
        KeyCode::Enter => Action::SavesLoad,
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => Action::SavesSaveAs,
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => Action::SavesDelete,
        KeyCode::Char('i') if key.modifiers == KeyModifiers::NONE => Action::SavesImport,
        KeyCode::Esc => Action::SavesClose,
        _ => Action::None,
    }
}

/// Hardwired VFS file-picker sub-mode keys (not rebindable, like saves/anim).
fn file_picker_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up => Action::FilePickerNav(-1),
        KeyCode::Down => Action::FilePickerNav(1),
        KeyCode::Enter => Action::FilePickerPick,
        KeyCode::Esc => Action::FilePickerClose,
        _ => Action::None,
    }
}

/// Hardwired replay/rewind sub-mode keys (not rebindable, like saves/anim).
fn history_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Left => Action::ReplayStep(-1),
        KeyCode::Right => Action::ReplayStep(1),
        KeyCode::PageUp => Action::ReplayPage(-1),
        KeyCode::PageDown => Action::ReplayPage(1),
        KeyCode::Home => Action::ReplayStep(i32::MIN as isize),
        KeyCode::End => Action::ReplayStep(i32::MAX as isize),
        KeyCode::Char(' ') => Action::ReplayTogglePlay,
        KeyCode::Enter | KeyCode::Char('r') => Action::ReplayResume,
        KeyCode::Esc | KeyCode::Char('q') => Action::ReplayClose,
        _ => Action::None,
    }
}

// ── Internal: file-browser key routing ───────────────────────────────────────

/// Hardwired file-browser sub-mode keys.
fn filebrowser_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up => Action::FbNav(-1),
        KeyCode::Down => Action::FbNav(1),
        KeyCode::PageUp => Action::FbPage(-1),
        KeyCode::PageDown => Action::FbPage(1),
        KeyCode::Home => Action::FbHome,
        KeyCode::End => Action::FbEnd,
        KeyCode::Enter => Action::FbEnter,
        KeyCode::Esc => Action::FbClose,
        _ => Action::None,
    }
}

// ── Internal: command-band key routing ───────────────────────────────────────

/// Key intercept for the command band. Returns `Some(action)` ONLY for the
/// handful of keys the band consumes, else `None` — the key falls through and
/// is handled by the always-live story input.
///
/// **SQ-0677 (2026-08-05) — supersedes SQ-0676's arrow-drives-quick scheme.**
/// The band still owns no text keys — printable characters and Backspace edit
/// the real prompt exactly as they do with the band closed. What it claims
/// instead:
///
/// | key | effect |
/// |---|---|
/// | `↑`/`↓` (plain) | move (or start) the row highlight within the current column |
/// | `Tab`, nothing highlighted | move the current column forward (pure movement) |
/// | `Tab`, a row highlighted (explicit or the typed nearest match) | pick that row and advance — exactly like a click |
/// | `Shift-Tab` | move the current column back — ALWAYS pure movement, even with a row highlighted |
/// | `Esc` | clear an explicit row highlight, else close the band |
/// | `Enter` | NOT claimed — always falls through to the ordinary prompt submit |
/// | `←`/`→` | NOT claimed — plain cursor movement on the edit line |
/// | Ctrl/Alt chords, the leader prefix | fall through, unchanged (Ctrl+↑/↓ reaches command history — see `key_to_command`'s readline-shortcuts step) |
///
/// Tab unifies completion and column flow: the typed nearest-match highlight
/// counts as "a row highlighted" too, so typing `ta` then Tab picks `take` and
/// advances to the next column in one gesture — there is no separate
/// "complete the word" action left (`Action::BandComplete` retired with it).
fn command_band_intercept(key: KeyEvent, state: &AppState) -> Option<Action> {
    // The hotkey-dialog leader prefix is never swallowed: it must fall through
    // so the player can open the leader palette (and the '/' command palette,
    // home of resize-panes and the rest of the long tail) while the band is up
    // (SQ-0238). First, so it wins even if the prefix is bound to a key the band
    // would otherwise consume.
    if KeySpec::from_key_event(key) == state.hotkeys.prefix {
        return None;
    }
    let band = state.overlays.command_band.as_ref()?;
    // App chords (Ctrl+S, Alt+…) always fall through — including Ctrl+↑/↓,
    // which reach command history instead (SQ-0677 restored it once plain
    // ↑/↓ moved to column row navigation here).
    if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return None;
    }
    let plain = key.modifiers == KeyModifiers::NONE;

    match key.code {
        KeyCode::Up if plain => Some(Action::BandRowNav(-1)),
        KeyCode::Down if plain => Some(Action::BandRowNav(1)),
        // PageUp/PageDown/Home/End (SQ-0682): the same standard list-nav keys
        // the story picker and IFDB search modal already bind, now on the
        // band's current column too.
        KeyCode::PageUp if plain => Some(Action::BandRowPage(-1)),
        KeyCode::PageDown if plain => Some(Action::BandRowPage(1)),
        KeyCode::Home if plain => Some(Action::BandRowHome),
        KeyCode::End if plain => Some(Action::BandRowEnd),
        KeyCode::Esc => Some(Action::BandEscape),
        KeyCode::BackTab => Some(Action::BandColumnStep(-1)),
        KeyCode::Tab => Some(match band.highlighted_row(&state.input.value) {
            Some(idx) => Action::BandTabPick(band.focus, idx),
            None => Action::BandColumnStep(1),
        }),
        _ => None,
    }
}

/// Mirror a band pick's effect onto the real story input line (SQ-0667,
/// 2026-08-05): composing happens directly on the prompt now, not a
/// band-local phrase row. `old_text`/`new_text` are
/// `CommandBandState::phrase_text()` immediately before and after the
/// mutation that triggered the sync.
///
/// The tail of `input`'s value matching `old_text` is swapped for
/// `new_text` — this is what makes a RE-pick (choosing a different verb
/// after an object was already picked, which drops the stale object; see
/// `CommandBandState::set_slot`) correctly shorten the input too, not just
/// the band's internal bookkeeping. If the input's tail has since diverged
/// from what the band's own bookkeeping expects (the player typed something
/// past it — routine since SQ-0676, where typing goes to this very line), the
/// new text is APPENDED instead of clobbering whatever they wrote: a pick
/// never destroys text the player typed themselves.
fn sync_band_phrase_to_input(input: &mut crate::text_field::TextField, old_text: &str, new_text: &str) {
    strip_band_tail(input, old_text);
    if new_text.is_empty() {
        return;
    }
    let mut v = input.value.clone();
    if !v.is_empty() && !v.ends_with(char::is_whitespace) {
        v.push(' ');
    }
    v.push_str(new_text);
    input.set(v, true);
}

/// Remove `old_text` from the end of `input`'s value, if it is still there —
/// leaving whatever text (if any) preceded it untouched. No-op (rather than a
/// wrong, partial delete) when the tail has diverged.
///
/// Matches against the value with any TRAILING whitespace ignored first
/// (`parse_phrase` reads `old_text` off `split_whitespace`, so a space typed
/// after the phrase — e.g. `examine ` — never shows up in it), and the
/// trailing whitespace is discarded along with `old_text` rather than kept:
/// [`sync_band_phrase_to_input`] re-adds exactly one separating space itself.
/// Without this, `old_text` no longer matches the tail at all, and the pick
/// falls through to being APPENDED after the untouched `old_text` instead of
/// replacing it (`examine ` + click `rope` → `examine examine rope`).
fn strip_band_tail(input: &mut crate::text_field::TextField, old_text: &str) {
    if old_text.is_empty() {
        return;
    }
    let trimmed_len = input.value.trim_end().len();
    if input.value[..trimmed_len].ends_with(old_text) {
        let mut v = input.value.clone();
        v.truncate(trimmed_len - old_text.len());
        input.set(v, true);
    }
}

/// Pick row `idx` of `col` and compose it onto the prompt — the shared core
/// of a mouse click (`Action::BandClickRow`) and a Tab-with-highlight pick
/// (`Action::BandTabPick`). `col` becomes the current column; `pick` then
/// advances it again to the NEXT reachable one (SQ-0677: a pick — click or
/// Tab — always moves the current column forward, symmetric with typing a
/// verb/prep advancing it).
///
/// Strips the word under construction FIRST, but ONLY when NOTHING has been
/// picked yet (`phrase_text()` empty) — e.g. completing the very first word,
/// `exa` → `examine`, where the partial verb isn't a recognized token yet so
/// `parse_phrase` doesn't count it toward `phrase_text()` at all, leaving the
/// tail-diff below (`old_text = ""`) nothing to strip on its own (mirrors
/// `apply_completion`'s truncate-then-insert for that case). Once at least
/// one slot IS picked, the partial word for whatever comes next (an object, a
/// second object) already counts toward `phrase_text()` — see
/// `CommandBandState::parse_phrase`'s doc, "the word still under construction
/// counts as a token" — so the tail-diff below replaces it correctly on its
/// own; pre-stripping here TOO would double-strip and leave a stray
/// duplicate (`take take door`, the falsified symptom — see
/// `typing_at_the_prompt_completes_from_the_live_object_columns` in
/// `tests/command_band.rs`). Shared by click and Tab alike (SQ-1230: a click
/// on a partial verb, e.g. `exa` + click `examine`, must replace it exactly
/// like Tab does, not append after it).
fn band_pick_row(state: &mut AppState, col: usize, idx: usize) {
    let old_is_empty =
        state.overlays.command_band.as_ref().is_some_and(|b| b.phrase_text().is_empty());
    if old_is_empty {
        let keep = state.input.char_len() - state.current_partial().chars().count();
        state.input.truncate_chars(keep);
    }

    let vp = state.modal_list_viewport;
    let anim = state.config.animation.clone();
    if let Some(b) = &mut state.overlays.command_band {
        b.focus = col;
        let len = b.items(col).len();
        b.scroll[col].len(len);
        b.scroll[col].select(idx, vp, &anim);
        let old = b.phrase_text();
        b.pick(col, idx);
        let new = b.phrase_text();
        if new != old {
            sync_band_phrase_to_input(&mut state.input, &old, &new);
        }
    }
}

/// Compose `word` onto the prompt exactly as a command-band WHAT-noun pick
/// does — the inventory dock's counterpart of [`band_pick_row`] (SQ-1244),
/// used when there is nothing to pick FROM: the inventory panel shows
/// exactly when the command band is closed (the two are mutually exclusive,
/// [`crate::state::SidePanel`]), so there is no `CommandBandState` whose
/// `items`/`pick` this could index into.
///
/// Routes through the SAME low-level composer `band_pick_row` itself calls
/// ([`sync_band_phrase_to_input`]), not a copy of its logic: `old_text` is
/// the word still under construction at the prompt
/// ([`AppState::current_partial`]), which the composer strips before
/// appending `word` with its one separating space. That is exactly SQ-1230's
/// "a partial word being typed is replaced" rule, read here off the raw
/// prompt text rather than off band picks — a typed verb before the partial
/// word is untouched (`sync_band_phrase_to_input` strips only the tail it is
/// told to), so `examine ` stays and the item lands after it, while a bare
/// unrecognized fragment (`exa`) is replaced outright.
fn compose_word_onto_prompt(state: &mut AppState, word: &str) {
    let partial = state.current_partial().to_string();
    sync_band_phrase_to_input(&mut state.input, &partial, word);
}

/// The command a quick-row pick fires (SQ-0667 amendment, 2026-08-05):
/// clicking or keyboard-picking a quick-row entry submits it AT ONCE, no
/// Enter — the one deliberate exception to "always confirm" (decision 2 in
/// `docs/design/2026-08-05-verb-panel-redesign.md`). Every quick word is
/// already a complete command on its own, so the second confirm bought
/// nothing but an extra step on the row built for single-click speed.
///
/// Pure and read-only on purpose: unlike every other pick, a quick pick must
/// NOT touch the band's in-progress phrase (an unfilled `unlock iron door`
/// stays exactly as it was) — it is an interjection, not a composition step.
/// The run loop (which has the session handle `apply_action` does not) calls
/// this to resolve the word, then submits it exactly like a typed command.
/// `None` when the band is closed or `idx` is stale (e.g. the band was
/// reconfigured between the click landing and this running).
///
/// Direction abbreviations SUBMIT as their full word: the quick row displays
/// `n s e w` for compactness, but Scott Adams vocabularies hold only the
/// spelled-out `NORTH`/`SOUTH`/…, so sending the abbreviation fails there
/// while the full word works in every engine.
///
/// Which words count as abbreviations is
/// [`compass_spelling`](crate::render::command_band::compass_spelling), the
/// band's own table, not `mapper::direction::parse_direction` (SQ-1130). Asked
/// of the parser, a quick row holding `bow` submitted **`north`** — the word
/// the player put on the row replaced by a movement the mapper reads it as, on
/// a story where `bow` is a verb that takes an object. Expanding `n` to `north`
/// is one word spelled two ways; rewriting `bow` is a different word.
pub fn band_quick_pick_command(state: &AppState, idx: usize) -> Option<String> {
    let word = state.overlays.command_band.as_ref()?.quick.get(idx)?;
    Some(match crate::render::command_band::compass_spelling(word) {
        Some(d) => full_direction_word(d).unwrap_or(word).to_string(),
        None => word.clone(),
    })
}

/// Double-click detection for the band's word columns (SQ-0690): a second click on the SAME
/// row within [`BandClickTracker::WINDOW`] completes a double-click, which the run loop turns
/// into a prompt submit — the pair's first click already picked the word, so the double is
/// "pick, then fire", mirroring the story list's select-then-launch double-click.
#[derive(Default)]
pub struct BandClickTracker {
    last: Option<(usize, usize, std::time::Instant)>,
}

impl BandClickTracker {
    /// Same window as the story list's launch double-click.
    pub const WINDOW: std::time::Duration = std::time::Duration::from_millis(400);

    /// Record a click on `(col, idx)`; `true` when it completes a double-click. A completed
    /// double RESETS the tracker: the submit emptied the prompt, so the next click on that row
    /// must read as a fresh pick, not a third beat of the same gesture.
    pub fn observe(&mut self, col: usize, idx: usize, now: std::time::Instant) -> bool {
        let double = self
            .last
            .take()
            .is_some_and(|(lc, li, lt)| lc == col && li == idx && now.duration_since(lt) < Self::WINDOW);
        if !double {
            self.last = Some((col, idx, now));
        }
        double
    }
}

/// The spelled-out command word for a direction, understood by every engine.
fn full_direction_word(d: mapper::direction::Direction) -> Option<&'static str> {
    use mapper::direction::Direction::*;
    Some(match d {
        N => "north",
        S => "south",
        E => "east",
        W => "west",
        NE => "northeast",
        NW => "northwest",
        SE => "southeast",
        SW => "southwest",
        Up => "up",
        Down => "down",
        In => "in",
        Out => "out",
        Unknown => return None,
    })
}

// ── Internal: resize-mode key routing ────────────────────────────────────────

/// Hardwired resize-mode sub-mode keys (not rebindable).
///
/// Tab / Shift+Tab → next/prev visible pane; arrows → grow/shrink the target;
/// `0` → reset to defaults; Esc/Enter → exit.
fn resize_mode_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Tab => Action::ResizeNav(ResizeNavKind::NextTarget),
        KeyCode::BackTab => Action::ResizeNav(ResizeNavKind::PrevTarget),
        KeyCode::Left => Action::ResizeNav(ResizeNavKind::Left),
        KeyCode::Right => Action::ResizeNav(ResizeNavKind::Right),
        KeyCode::Up => Action::ResizeNav(ResizeNavKind::Up),
        KeyCode::Down => Action::ResizeNav(ResizeNavKind::Down),
        KeyCode::Char('0') => Action::ResizeReset,
        KeyCode::Esc | KeyCode::Enter => Action::ResizeExit,
        _ => Action::None,
    }
}

// ── Internal: config-screen key routing ──────────────────────────────────────

/// `focus` is the current button-focus index within the config-screen ring:
///   0 = Save, 1 = Cancel. Ring length is 2.
/// Tab/BackTab are handled upstream (main.rs intercept) and never reach here.
/// Enter activates the focused button; Space still toggles the selected row.
fn config_screen_key_to_action(key: KeyEvent, focus: usize) -> Action {
    match key.code {
        KeyCode::Up => Action::ConfigNav(-1),
        KeyCode::Down => Action::ConfigNav(1),
        KeyCode::PageUp => Action::ConfigPage(-1),
        KeyCode::PageDown => Action::ConfigPage(1),
        KeyCode::Home => Action::ConfigHome,
        KeyCode::End => Action::ConfigEnd,
        KeyCode::Left => Action::ConfigCycle(-1),
        KeyCode::Right => Action::ConfigCycle(1),
        KeyCode::Char(' ') if key.modifiers == KeyModifiers::NONE => Action::ConfigToggle,
        KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
            // Ring: [Save(0), Cancel(1)]. Enter activates the focused button.
            match focus {
                1 => Action::ConfigCancel,
                _ => Action::ConfigSave, // default: Save (focus 0)
            }
        }
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => Action::ConfigSave,
        KeyCode::Esc => Action::ConfigCancel,
        _ => Action::None,
    }
}

// ── Internal: game focus ──────────────────────────────────────────────────────

fn game_key_to_action(state: &AppState, key: KeyEvent) -> Action {
    let shift = key.modifiers == KeyModifiers::SHIFT;
    match key.code {
        // Map navigation is available WITHOUT leaving the story line: Shift+Arrows
        // pan and the non-typeable Home recenters; PageUp/PageDown page the
        // transcript. None of these clash with typing a command (arrows/Home/PageX
        // aren't printable, and Shift+Arrow is distinct from a Shift+letter capital).
        KeyCode::Left if shift => Action::Pan(-1, 0),
        KeyCode::Right if shift => Action::Pan(1, 0),
        KeyCode::Up if shift => Action::Pan(0, -1),
        KeyCode::Down if shift => Action::Pan(0, 1),
        // Plain Up/Down recall command history (shell-style).
        KeyCode::Up if key.modifiers == KeyModifiers::NONE => Action::HistoryPrev,
        KeyCode::Down if key.modifiers == KeyModifiers::NONE => Action::HistoryNext,
        // Caret editing on the command line (SQ-0354). Plain Left/Right were unbound here and fell
        // through to the Global keymap; they are text keys the moment the line has a caret.
        //
        // Safe against story-controlled input: when the story asks for a single keypress the run
        // loop's char-mode gate forwards the key straight to the VM and never reaches this
        // function. Shift+Arrows still pan the map, and plain Up/Down still recall history.
        KeyCode::Left if key.modifiers == KeyModifiers::NONE => Action::CursorLeft,
        KeyCode::Right if key.modifiers == KeyModifiers::NONE => Action::CursorRight,
        KeyCode::Delete => Action::DeleteChar,
        // Home/End are the conventional line-editing pair, matching every other text entry in the
        // app. Home used to recenter the map; that keeps its `center-map` command and hotkey.
        KeyCode::Home => Action::CursorHome,
        KeyCode::End => Action::CursorEnd,
        // PageUp/PageDown page the transcript (toward older / newer). Zoom stays
        // on +/=/-/0, Ctrl+wheel, and /zoom-map.
        KeyCode::PageUp => Action::TranscriptScrollPage(1),
        KeyCode::PageDown => Action::TranscriptScrollPage(-1),
        // Enter submits the current input buffer content as the command.
        KeyCode::Enter => Action::SubmitCommand(state.input.value.clone()),
        KeyCode::Backspace => Action::Backspace,
        // '/' at an EMPTY prompt promotes into the command palette (SQ-0419); '/'
        // mid-line stays a literal character (falls through to InputChar below).
        KeyCode::Char('/') if key.modifiers == KeyModifiers::NONE && state.input.value.is_empty() => {
            Action::OpenCommandPalette { from_hotkey: false }
        }
        KeyCode::Char(c)
            if key.modifiers == KeyModifiers::NONE
                || key.modifiers == KeyModifiers::SHIFT =>
        {
            Action::InputChar(c)
        }
        _ => Action::None,
    }
}

// ── Focus cycling ─────────────────────────────────────────────────────────────

/// Compute the new transcript scroll offset for a one-page step.
///
/// `dir > 0` scrolls toward older lines (increasing the offset); `dir < 0` toward
/// newer. The step is `viewport_rows - 1` (a one-line overlap for reading
/// continuity), floored at 1 so paging always progresses, and the result is
/// clamped to `[0, max_scroll]` — the same bounds the mouse-wheel scroll uses.
pub fn page_scroll(current: u16, dir: i8, viewport_rows: u16, max_scroll: u16) -> u16 {
    let next = crate::list_scroll::page_step(current as usize, dir as i32, viewport_rows as usize);
    (next.min(u16::MAX as usize) as u16).min(max_scroll)
}

/// Compute the new transcript scroll offset for a half-page step (Ctrl-D, vim
/// convention; SQ-1228). Same direction and clamp semantics as `page_scroll`,
/// but the step is `floor(viewport_rows / 2)` (floored at 1) rather than a full
/// page.
pub fn half_page_scroll(current: u16, dir: i8, viewport_rows: u16, max_scroll: u16) -> u16 {
    let next = crate::list_scroll::half_page_step(current as usize, dir as i32, viewport_rows as usize);
    (next.min(u16::MAX as usize) as u16).min(max_scroll)
}

/// Cycle a button-focus index by `delta` (+1 Tab, -1 Shift-Tab), wrapping within
/// `0..len`. Returns 0 when `len` is 0.
pub fn cycle_focus(idx: usize, len: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let next = idx as i32 + delta;
    next.rem_euclid(len as i32) as usize
}

// ── apply_action ──────────────────────────────────────────────────────────────

/// Apply a view or light-correction action to `state` and/or `mapper`.
///
/// **Caller-handled actions** (silently ignored here — the run loop must act on
/// them): `SubmitCommand` (game focus), `SaveGame`, `RestoreGame`, `ExportSvg`,
/// `Quit`.
///
/// The former bottom-bar prompt sub-mode is gone: rename/notes/relabel/layer/
/// config-path/create-file open the `text_entry` modal (submit via
/// [`apply_text_entry`]), and delete-save opens the `confirm_delete_save` modal.
///
/// **Edge rule for DeleteSelectedConnection / RelabelSelectedEdge**: operates on
/// the *first* outgoing connection of the selected room as returned by
/// `mapper.graph.connections()` in iteration order (stable insertion order).
/// If the room has no connections, the operation is a no-op.
///
/// **Recenter**: calls `state.recenter_on(room_pos, 80, 24)` — a default pane
/// size used when the render pane size is not yet available.  The run loop
/// should call `state.recenter_on` with the real pane size when it knows it.
/// Open or close the command band, without persisting anything (SQ-1123).
///
/// The band's open/closed state is a per-game preference now, written by
/// [`Action::OpenCommandBand`]. `startup` opens the band at boot from
/// `[command_panel] auto_open` (or this game's own override), and that must NOT
/// write a sidecar key — a global default that quietly pinned itself to the
/// first game you opened would be a trap. So the state change lives here and the
/// action adds the persistence on top of it.
///
/// `is_none` still guards the fresh-open branch, so a re-press while the band is
/// mid-CLOSE (dock target `false`, content still alive for the slide-out)
/// reopens it rather than double-firing a close, exactly as it always has.
pub fn open_command_band(state: &mut AppState, mapper: &mut Mapper, open: bool) {
    if !open {
        apply_action(Action::BandClose, state, mapper);
        return;
    }
    if state.overlays.command_band.is_none() {
        let (verbs, warnings) = state.config.resolve_band_verbs();
        for w in warnings {
            state.push_transcript_kind(&w, crate::state::TranscriptKind::Warning);
        }
        let mut band = crate::state::CommandBandState::new(
            verbs,
            state.config.command_band.resolve_quick(),
        );
        // The band opens reading whatever is ALREADY on the prompt (SQ-0676): its
        // phrase state follows the typed line, so a half-typed `take ` must light
        // the object columns the moment the band appears, not one keystroke
        // later. Object columns fill from the engine on the next tick.
        band.sync_from_input(&state.input.value);
        state.overlays.command_band = Some(band);
    }
    state.band_dock.toggle_to(true, false);
    state.band_dock.arm(&state.config.animation);
}

/// Open or close the inventory panel, without persisting anything (SQ-1237) —
/// the state-only half `open_command_band` above is, for the same reason: boot
/// and `cycle_panel` both need to change the panel without writing a per-game
/// override on their own behalf, leaving that to the action arm that persists.
pub fn open_inventory_panel(state: &mut AppState, open: bool) {
    state.show_inventory = open;
    state.inv_dock.toggle_to(open, false);
    state.inv_dock.arm(&state.config.animation);
}

/// Cycle the story pane's border control: command panel → inventory panel →
/// none → command panel (SQ-1237). The two panels are mutually exclusive, so
/// landing on one closes the other; landing on `None` closes both. Persists
/// the new state per-game, exactly as a direct `/toggle-command-panel` or
/// `/toggle-inventory-panel` does — a click on the border control runs this
/// through the same `slash::COMMANDS` dispatch every other control uses, so
/// what it remembers is this function's job, not a second one.
pub fn cycle_panel(state: &mut AppState, mapper: &mut Mapper) {
    let next = state.current_side_panel().next();
    match next {
        crate::state::SidePanel::Command => {
            open_inventory_panel(state, false);
            open_command_band(state, mapper, true);
        }
        crate::state::SidePanel::Inventory => {
            open_command_band(state, mapper, false);
            open_inventory_panel(state, true);
        }
        crate::state::SidePanel::None => {
            open_command_band(state, mapper, false);
            open_inventory_panel(state, false);
        }
    }
    if !state.game_dir.as_os_str().is_empty() {
        let _ = crate::styles::write_per_game_panel(&state.game_dir, Some(next));
    }
}

pub fn apply_action(action: Action, state: &mut AppState, mapper: &mut Mapper) {
    // SQ-0676: the command band READS the story input line, so every action
    // that changes that line — typing, Backspace, a delete op, history recall,
    // an autocompletion, a column pick mirroring onto it — has to leave the
    // band pointing somewhere honest afterwards. Snapshotting around the whole
    // dispatch catches all of them in one place, including the arms that
    // return early, instead of asking every future input-touching arm to
    // remember (which is exactly how the two would drift apart).
    let before = state.overlays.command_band.as_ref().map(|_| state.input.value.clone());
    apply_action_inner(action, state, mapper);
    if let Some(before) = before {
        if before != state.input.value {
            band_react_to_input(state);
        }
    }
}

/// Re-point the band at the (just-changed) story input line: clear the
/// explicit row highlight (a text change means the last gesture was TYPING,
/// not `↑`/`↓` — SQ-0677's "the last gesture decides", same rule the retired
/// `quick_sel` used to follow), re-derive the phrase state from what is
/// typed, and scroll the current column so its nearest match is actually on
/// screen.
///
/// `apply_action` and `apply_paste` call this for themselves. It is public for
/// the run loop's submit arm, which empties the line through
/// `AppState::take_input` without going through either — leaving the band
/// showing the columns of a phrase that has already been sent.
pub fn band_react_to_input(state: &mut AppState) {
    let input = state.input.value.clone();
    let vp = state.modal_list_viewport;
    let anim = state.config.animation.clone();
    let Some(b) = state.overlays.command_band.as_mut() else { return };
    b.row_sel = None;
    b.sync_from_input(&input);
    if let Some((col, idx)) = b.nearest_match(&input) {
        let len = b.items(col).len();
        b.scroll[col].len(len);
        b.scroll[col].select(idx, vp, &anim);
    }
}

fn apply_action_inner(action: Action, state: &mut AppState, mapper: &mut Mapper) {
    // ── Normal action dispatch ────────────────────────────────────────────
    match action {
        Action::InputChar(c) => {
            state.push_input_char(c);
            // Recompute suggestions after every character typed in game focus.
            if state.focus == Focus::Game {
                recompute_suggestions(state);
                state.suggestion_idx = 0;
                state.suggestion_active = false;
            }
        }
        Action::Backspace => {
            state.backspace();
            // Recompute suggestions after deletion in game focus.
            if state.focus == Focus::Game {
                recompute_suggestions(state);
                state.suggestion_idx = 0;
                state.suggestion_active = false;
            }
        }
        Action::Autocomplete => {
            // Apply the currently-highlighted suggestion to the input buffer,
            // replacing the partial word being typed. Then advance the index
            // so repeated Tab cycles through candidates.
            if !state.suggestions.is_empty() {
                let len = state.suggestions.len();
                // First Tab applies the highlighted candidate (the preview at
                // `suggestion_idx`); only later presses advance so the bracket
                // stays on the word now in the input.
                if state.suggestion_active {
                    state.suggestion_idx = (state.suggestion_idx + 1) % len;
                } else {
                    state.suggestion_active = true;
                }
                let idx = state.suggestion_idx % len;
                let completion = state.suggestions[idx].clone();
                apply_completion(state, &completion);
            }
        }
        Action::AutocompletePrev => {
            // Inverse of Autocomplete: apply the currently-highlighted suggestion,
            // then step the index BACKWARD (wrapping) so repeated Shift-Tab cycles
            // through candidates in reverse.
            if !state.suggestions.is_empty() {
                let len = state.suggestions.len();
                // First Shift-Tab applies the highlighted candidate; only later
                // presses step backward so the bracket stays on the applied word.
                if state.suggestion_active {
                    state.suggestion_idx = (state.suggestion_idx + len - 1) % len;
                } else {
                    state.suggestion_active = true;
                }
                let idx = state.suggestion_idx % len;
                let completion = state.suggestions[idx].clone();
                apply_completion(state, &completion);
            }
        }
        Action::HistoryPrev => state.history_prev(),
        Action::HistoryNext => state.history_next(),
        Action::ToggleFocus => state.toggle_focus(),
        Action::CycleFocusBack => state.cycle_focus(false),
        Action::ToggleMap => {
            state.toggle_map();
            // Persist the map panel's on/off state per-game so it's restored the
            // next time this story opens (SQ-0304). No game_dir → no sidecar (and
            // keeps unit tests off the filesystem).
            if !state.game_dir.as_os_str().is_empty() {
                let show = state.layout == crate::state::Layout::Split;
                let _ = crate::styles::write_per_game_show_map(&state.game_dir, Some(show));
            }
        }
        Action::CursorLeft => state.input.left(),
        Action::CursorHome => state.input.home(),
        Action::CursorEnd => state.input.end(),
        Action::DeleteChar => {
            state.input.delete();
            recompute_suggestions(state);
        }
        Action::DeleteToStart => {
            state.input.delete_to_start();
            recompute_suggestions(state);
        }
        Action::DeleteToEnd => {
            state.input.delete_to_end();
            recompute_suggestions(state);
        }
        Action::DeleteWordBack => {
            state.input.delete_prev_word();
            recompute_suggestions(state);
        }
        Action::CursorRight => {
            // At the END of the line with a suggestion showing, Right ACCEPTS it (SQ-0354) — the
            // fish/zsh gesture. There is no text to the right to move onto, so the keystroke would
            // otherwise do nothing at all; taking the completion is the only useful reading.
            //
            // Applying it exactly as Tab does keeps the two paths honest: `suggestion_active`
            // flips, so the bracketed preview reads as applied rather than still just a preview.
            let at_end = state.input.cursor >= state.input.char_len();
            match state.suggestions.get(state.suggestion_idx % state.suggestions.len().max(1)) {
                Some(c) if at_end => {
                    let completion = c.clone();
                    state.suggestion_active = true;
                    apply_completion(state, &completion);
                }
                _ => state.input.right(),
            }
        }
        Action::CursorToClick(col, row) => {
            if let Some(idx) = state.input_click_index(col, row) {
                state.input.cursor = idx;
            }
        }
        Action::ZoomIn => state.zoom_in(),
        Action::ZoomOut => state.zoom_out(),
        Action::ZoomBy(n) => state.zoom_by(n),
        Action::ZoomInFine => state.zoom_in_fine(),
        Action::ZoomOutFine => state.zoom_out_fine(),
        Action::ZoomReset => state.zoom_reset(),
        Action::Pan(dx, dy) => {
            // SQ-0666: on a layer showing the matrix, panning IS list scrolling — the wheel and
            // Shift+arrows are the pane-scroll conventions every other surface uses, and the
            // drawn map's grid-cell viewport means nothing to a table. `pan` would otherwise
            // move a viewport nobody is looking at while the table sat still.
            if mapper.graph.layer_view(state.active_layer(&mapper.graph))
                == mapper::layer::MapView::Matrix
            {
                let rows = mapper.graph.rooms_in_layer(state.active_layer(&mapper.graph)).len();
                let max = rows.saturating_sub(1) as i32;
                state.matrix_scroll.1 = (state.matrix_scroll.1 as i32 + dy).clamp(0, max) as u16;
                state.matrix_scroll.0 = (state.matrix_scroll.0 as i32 + dx).clamp(0, 11) as u16;
            } else {
                state.pan(dx, dy);
            }
        }
        Action::Recenter => apply_recenter(state, mapper),
        Action::SelectNext => select_adjacent(state, mapper, 1),
        Action::SelectPrev => select_adjacent(state, mapper, -1),

        Action::MoveRegion(arg) => apply_move_region(state, mapper, &arg),
        // ── SQ-0666: navigating the matrix ──────────────────────────────────────────
        Action::MatrixMove(delta) => {
            let layer = state.active_layer(&mapper.graph);
            if mapper.graph.layer_view(layer) == mapper::layer::MapView::Matrix {
                if let Some(id) =
                    crate::render::matrix::step_selection(&mapper.graph, layer, state.selected_room, delta)
                {
                    // Selection only. Opening the room panel on every arrow press would cover
                    // the very table the player is stepping through; a CLICK still opens it,
                    // because a click is a deliberate "tell me about this one".
                    state.select_room(Some(id));
                    // Any route on screen belonged to the room we just stepped off (SQ-0693).
                    // Recomputing it per arrow instead would fire "no known route from here" for
                    // every unreachable row the selection merely passed over.
                    state.room_path.clear();
                    if let Some((w, h)) = state.map_pane_size.get() {
                        let area = ratatui::layout::Rect::new(0, 0, w, h);
                        state.matrix_scroll.1 = crate::render::matrix::scroll_to_show(
                            &mapper.graph,
                            layer,
                            id,
                            area,
                            state.matrix_scroll.1,
                        );
                    }
                }
            }
        }
        Action::MatrixPanColumns(delta) => {
            let layer = state.active_layer(&mapper.graph);
            if mapper.graph.layer_view(layer) == mapper::layer::MapView::Matrix {
                // Clamped to the twelve columns here; the renderer clamps again to whatever
                // actually fits, which is the only place that knows the column width.
                let next = (state.matrix_scroll.0 as i32 + delta).clamp(0, 11);
                state.matrix_scroll.0 = next as u16;
            }
        }

        // ── SQ-0666: how the active layer draws, and whether it is a maze ────────────
        Action::ViewMap(want) => {
            use mapper::layer::MapView;
            let layer = state.active_layer(&mapper.graph);
            // A bare `/view-map` cycles from what you are LOOKING at, not from what was stored:
            // on a maze-flagged layer that has never been set by hand the stored choice is None,
            // and cycling from the default is what the player means by "the other one".
            let next = want.unwrap_or(match mapper.graph.layer_view(layer) {
                MapView::Drawn => MapView::Matrix,
                MapView::Matrix => MapView::Drawn,
            });
            mapper.graph.set_layer_view(layer, Some(next));
            state.bump_graph_gen(); // the pane draws something else entirely (SQ-0305)
            let label = match next {
                MapView::Drawn => "drawn",
                MapView::Matrix => "matrix",
            };
            state.set_status(format!("{}: {label} view", mapper.graph.layer_name(layer)));
        }
        Action::MarkMazeLayer => {
            use mapper::layer::MapView;
            let layer = state.active_layer(&mapper.graph);
            let maze = !mapper.graph.layer_is_maze(layer);
            mapper.graph.set_layer_maze(layer, maze);
            state.bump_graph_gen();
            // Flagging a maze is how most players will ever reach the matrix, and the flag alone
            // takes them there: it moves the layer's DEFAULT view. Nothing is written to the
            // layer's explicit choice, so an earlier `/view-map` still wins, and unflagging puts
            // an unchosen layer straight back to drawn instead of stranding it on the matrix.
            let name = mapper.graph.layer_name(layer).to_string();
            let showing = mapper.graph.layer_view(layer);
            state.set_status(match (maze, showing) {
                (true, MapView::Matrix) => format!("{name}: maze — showing the direction matrix"),
                (true, MapView::Drawn) => format!("{name}: maze (still drawn — /view-map matrix)"),
                (false, _) => format!("{name}: no longer a maze"),
            });
        }

        // Re-tidy: re-derive the clean Auto layout (constrained stress majorization,
        // or the longest-path sort for very large maps), then nudge rooms so the lane
        // router has no illegal overlaps. Honours compass ordering the greedy per-turn
        // placement can't. No-op in Manual mode — those positions are user-owned.
        Action::Retidy => {
            // Re-tidy off the main thread so the progress bar shows and the UI stays
            // live during a long tidy on a large map — same machinery as `animate-tidy`,
            // but `animate: false` so the run loop applies the tidied graph instantly
            // (no animation playback). Guard against a double-spawn while a build is in
            // flight or an animation is showing. (SQ-0261)
            if state.anim_build_job.is_none() && state.tidy_anim.is_none() {
                let layer = state.active_layer(&mapper.graph);
                // A maze layer's geometry is frozen (SQ-0671), and a tidy the player asked for by
                // hand is no exception: there is no compass arrangement of a maze to find, and
                // the table they are reading would not change if there were.
                if crate::tidy::layer_is_frozen(&mapper.graph, layer) {
                    state.set_status("maze layer: geometry is frozen — the matrix is the view");
                    return;
                }
                let mut g = mapper.graph.clone();
                let gen = state.graph_gen;
                let total = mapper.graph.rooms_in_layer(layer).len() + 8;
                let progress = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let progress_clone = std::sync::Arc::clone(&progress);
                let handle = std::thread::spawn(move || {
                    let frames = crate::tidy::run_tidy_pipeline(&mut g, layer, Some(progress_clone));
                    (frames, g)
                });
                state.anim_build_job = Some(crate::state::AnimBuildJob {
                    handle,
                    layer,
                    gen,
                    started: std::time::Instant::now(),
                    progress,
                    total,
                    animate: false,
                });
                state.set_status("tidying map…");
            }
        }

        Action::ReloadStyle => {
            match crate::reload::reload_style(state) {
                crate::reload::ReloadOutcome::Reloaded { warnings } => {
                    for w in &warnings {
                        state.push_transcript_kind(w, crate::state::TranscriptKind::Warning);
                    }
                    state.set_status("style reloaded");
                }
                crate::reload::ReloadOutcome::Failed { msg } => {
                    state.push_transcript_kind(
                        &format!("style reload failed: {}", msg),
                        crate::state::TranscriptKind::Warning,
                    );
                    state.set_status("reload failed — keeping current style");
                }
            }
        }

        Action::ToggleWatch => { /* handled in the run loop (owns the watcher) */ }

        Action::AnimateTidy => {
            // Build the animation frames on a worker thread so the UI stays responsive
            // during the (potentially long) build. The run loop polls the job, applies the
            // tidied graph, and installs the animation when it finishes. Guard against a
            // double-spawn while one build is in flight or an animation is already showing.
            if state.anim_build_job.is_none() && state.tidy_anim.is_none() {
                let layer = state.active_layer(&mapper.graph);
                let mut g = mapper.graph.clone();
                let gen = state.graph_gen;
                // Estimate the final frame count from the layer's room count (one placement
                // frame per room dominates), plus headroom for the fixed layout/cleanup stages.
                // Only an estimate — the real total isn't known until the build finishes.
                let total = mapper.graph.rooms_in_layer(layer).len() + 8;
                let progress = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let progress_clone = std::sync::Arc::clone(&progress);
                let handle = std::thread::spawn(move || {
                    let frames = crate::tidy::run_tidy_pipeline(&mut g, layer, Some(progress_clone));
                    (frames, g)
                });
                state.anim_build_job = Some(crate::state::AnimBuildJob {
                    handle,
                    layer,
                    gen,
                    started: std::time::Instant::now(),
                    progress,
                    total,
                    animate: true,
                });
                state.set_status("preparing tidy animation…");
            }
        }

        Action::AnimStep(d) => {
            if let Some(anim) = &mut state.tidy_anim {
                anim.step(d as isize);
            }
        }

        Action::AnimTogglePlay => {
            if let Some(anim) = &mut state.tidy_anim {
                anim.toggle_play();
            }
        }

        Action::AnimExit => state.tidy_anim = None,

        Action::AnimStageJump(d) => {
            if let Some(anim) = &mut state.tidy_anim {
                let current = anim.idx;
                let n = anim.frames.len();
                if d > 0 {
                    if let Some(next) = ((current + 1)..n).find(|&i| anim.frames[i].stage_start) {
                        anim.idx = next;
                        anim.playing = false;
                    }
                } else if current > 0 {
                    if let Some(prev) = (0..current).rev().find(|&i| anim.frames[i].stage_start) {
                        anim.idx = prev;
                        anim.playing = false;
                    }
                }
            }
        }

        Action::CycleLayer(delta) => {
            let mut ids: Vec<_> = mapper.graph.layers().keys().copied()
                .filter(|&l| !mapper.graph.rooms_in_layer(l).is_empty())
                .collect();
            ids.sort_unstable();
            if !ids.is_empty() {
                let cur = state.active_layer(&mapper.graph);
                let i = ids.iter().position(|&l| l == cur).unwrap_or(0) as i32;
                let j = (i + delta).clamp(0, ids.len() as i32 - 1) as usize;
                state.set_viewed_layer(Some(ids[j]));
                recenter_for_active_layer(state, &mapper.graph);
            }
        }

        Action::SetViewedLayer(layer) => {
            // A click on a map layer tab selects that layer as the viewed one.
            // set_viewed_layer + active_layer tolerate a stale id (falls back).
            state.set_viewed_layer(Some(layer));
            recenter_for_active_layer(state, &mapper.graph);
        }

        Action::ToggleAlignment => state.show_alignment = !state.show_alignment,
        Action::TogglePortalLabels => state.show_portal_labels = !state.show_portal_labels,
        Action::ToggleRoomNumbers => state.show_room_numbers = !state.show_room_numbers,
        Action::ToggleStatusBar => state.show_status_bar = !state.show_status_bar,
        Action::ToggleTimedInput => {
            state.config.honor_timed_input = !state.config.honor_timed_input;
            state.set_status(if state.config.honor_timed_input { "timed input on" } else { "timed input off" });
        }
        Action::ToggleSound => {
            // Reaching for the key is a decision, so it ends any hold `--sound`
            // had on this run's value — the same rule the settings rows follow
            // (SQ-0807).
            state.config.one_run.release(crate::config::keys::ENABLE_SOUND);
            state.config.enable_sound = !state.config.enable_sound;
            state.set_status(if state.config.enable_sound { "sound on" } else { "sound off" });
            if !state.config.enable_sound {
                state.reset_sound_sidecars();
            } else if state.audio.is_none() {
                state.audio = Some(audio::AudioBackend::new(state.config.volume));
            }
            // Sync the running Glulx VM's Sound gestalt (applied by the event loop).
            state.pending_vm_sound = Some(state.config.enable_sound);
        }
        Action::SetVolume(v) => {
            let v = v.min(100);
            state.config.volume = v;
            state.set_status(format!("volume {v}"));
            if let Some(b) = state.audio.as_mut() { b.set_volume(v); }
        }
        Action::ToggleRoomDiagnostics => {
            // Closed dock: open it straight onto the diagnostics body. Open dock:
            // flip which body it draws (SQ-0692). It no longer needs a SELECTED
            // room — an unpinned dock diagnoses whatever room you are standing in,
            // which is the reading you want most of the time and the one the old
            // `/toggle-inspector` could not give you at all.
            use crate::state::RoomDockView;
            if state.room_dock.open {
                state.room_dock_view = state.room_dock_view.flipped();
            } else {
                state.open_room_dock(RoomDockView::Diagnostics);
            }
        }

        Action::RenameRoom => {
            if let Some(id) = state.selected_room {
                state.overlays.hotkey_dialog = false;
                state.overlays.dialog_focus = 0;
                // Opens empty: an empty submit clears the room's custom label.
                state.overlays.text_entry = Some(TextEntryDialog::new(TextEntryKind::RenameRoom(id), ""));
            }
        }
        Action::RenameLayer => {
            let layer = state.active_layer(&mapper.graph);
            let current_name = mapper.graph.layer_name(layer).to_owned();
            state.overlays.hotkey_dialog = false;
            state.overlays.dialog_focus = 0;
            state.overlays.text_entry =
                Some(TextEntryDialog::new(TextEntryKind::RenameLayer(layer), current_name));
        }
        Action::EditNotes => {
            if let Some(id) = state.selected_room {
                let current_notes =
                    mapper.graph.room(id).map(|r| r.notes.clone()).unwrap_or_default();
                state.overlays.hotkey_dialog = false;
                state.overlays.dialog_focus = 0;
                // Prefilled with the room's existing notes (caret at the end): submit
                // replaces the notes with the field's contents (empty clears them).
                state.overlays.text_entry =
                    Some(TextEntryDialog::new(TextEntryKind::EditNotes(id), current_notes));
            }
        }
        Action::RelabelSelectedEdge => {
            if let Some(id) = state.selected_room {
                // Find the first outgoing connection for this room.
                if let Some(conn) =
                    mapper.graph.connections().iter().find(|c| c.origin == id)
                {
                    let old_dir = conn.dir;
                    state.overlays.hotkey_dialog = false;
                    state.overlays.dialog_focus = 0;
                    state.overlays.text_entry =
                        Some(TextEntryDialog::new(TextEntryKind::RelabelEdge(id, old_dir), ""));
                }
            }
        }
        Action::DeleteSelectedConnection => {
            if let Some(id) = state.selected_room {
                // Delete the first outgoing connection for this room.
                if let Some(conn) =
                    mapper.graph.connections().iter().find(|c| c.origin == id).cloned()
                {
                    mapper.delete_connection(conn.origin, conn.dir);
                    state.bump_graph_gen(); // edge removed → invalidate map memo (SQ-0305)
                }
            }
        }
        Action::OpenHotkeyDialog => {
            state.overlays.hotkey_dialog = true;
            // Close other overlays if open.
            state.overlays.saves = None;
        }

        Action::CloseHotkeyDialog => {
            state.overlays.hotkey_dialog = false;
        }

        // ── Command palette actions (SQ-0419) ─────────────────────────────────

        Action::OpenCommandPalette { from_hotkey } => {
            // The palette owns its own input line; the story prompt underneath is
            // left untouched, so closing restores it unchanged.
            state.overlays.hotkey_dialog = false;
            state.overlays.palette = Some(crate::state::PaletteState::new(from_hotkey));
        }

        Action::PaletteNav(delta) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(p) = &mut state.overlays.palette {
                let len = crate::complete::palette_candidates(p.query()).len();
                if len > 0 {
                    p.scroll.len(len);
                    let next = ((p.scroll.selected as i32 + delta).rem_euclid(len as i32)) as usize;
                    p.scroll.select(next, vp, &anim);
                }
            }
        }

        Action::PaletteChar(c) => {
            if let Some(p) = &mut state.overlays.palette {
                p.input.insert(c);
                // A changed query re-ranks the list; snap the selection to the top.
                let vp = state.modal_list_viewport;
                let anim = state.config.animation.clone();
                let len = crate::complete::palette_candidates(p.query()).len();
                p.scroll.len(len);
                p.scroll.select(0, vp, &anim);
            }
        }

        Action::PaletteBackspace => {
            if let Some(p) = &mut state.overlays.palette {
                p.input.backspace();
                let vp = state.modal_list_viewport;
                let anim = state.config.animation.clone();
                let len = crate::complete::palette_candidates(p.query()).len();
                p.scroll.len(len);
                p.scroll.select(p.scroll.selected.min(len.saturating_sub(1)), vp, &anim);
            }
        }

        Action::PaletteComplete => {
            if let Some(p) = &mut state.overlays.palette {
                let cands = crate::complete::palette_candidates(p.query());
                if let Some(cand) = cands.get(p.scroll.selected) {
                    let name = crate::slash::COMMANDS[cand.cmd_index].name;
                    // Replace the first token with the full command name, keeping
                    // any typed args; append a trailing space so args can follow.
                    let args = p.args();
                    let line = if args.is_empty() {
                        format!("{name} ")
                    } else {
                        format!("{name} {args}")
                    };
                    p.input.set(line, true);
                }
            }
        }

        Action::PaletteClose => {
            let from_hotkey = state.overlays.palette.as_ref().map(|p| p.from_hotkey).unwrap_or(false);
            state.overlays.palette = None;
            if from_hotkey {
                state.overlays.hotkey_dialog = true;
            }
        }

        // ── Saves-manager actions ─────────────────────────────────────────────

        Action::OpenSaves => {
            // The list must be populated by the caller (main.rs has dir + ifid).
            // apply_action only sets up the state; the caller refreshes the list
            // via AppState::open_saves_modal after apply_action returns.
            // If already open, do nothing.
            state.overlays.hotkey_dialog = false;
            state.overlays.dialog_focus = 0;
        }

        // SQ-0831: one wheel rule for every selection list. The notch moves the
        // OFFSET and `ListScroll::scroll_by` clamps the cursor into the visible
        // window — the opposite of the `*Nav` arms below, where the cursor moves
        // and the window chases it. Dispatch order mirrors the wheel precedence
        // in `mouse_to_action` (the palette routes here from the run loop, which
        // intercepts its own mouse events before that function).
        Action::ListWheel(delta) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            let d = delta as isize;
            if let Some(cs) = &mut state.overlays.config_screen {
                cs.scroll.len(CONFIG_ROW_COUNT);
                cs.scroll.scroll_by(d, vp, &anim);
            } else if let Some(s) = &mut state.overlays.saves {
                let len = s.entries.len();
                s.scroll.len(len);
                s.scroll.scroll_by(d, vp, &anim);
            } else if let Some(fb) = &mut state.overlays.file_browser {
                let len = fb.entries.len();
                fb.scroll.len(len);
                fb.scroll.scroll_by(d, vp, &anim);
            } else if let Some(p) = &mut state.overlays.palette {
                let len = crate::complete::palette_candidates(p.query()).len();
                p.scroll.len(len);
                p.scroll.scroll_by(d, vp, &anim);
            }
        }

        Action::SavesNav(delta) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(s) = &mut state.overlays.saves {
                if !s.entries.is_empty() {
                    let len = s.entries.len();
                    s.scroll.len(len);
                    // Preserve the existing wrap-around behavior via select().
                    let next = ((s.scroll.selected as i32 + delta).rem_euclid(len as i32)) as usize;
                    s.scroll.select(next, vp, &anim);
                }
            }
        }

        Action::SavesPage(dir) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(s) = &mut state.overlays.saves {
                let len = s.entries.len();
                s.scroll.len(len);
                s.scroll.page(dir, vp, &anim);
            }
        }

        Action::SavesHome => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(s) = &mut state.overlays.saves {
                s.scroll.len(s.entries.len());
                s.scroll.home(vp, &anim);
            }
        }

        Action::SavesEnd => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(s) = &mut state.overlays.saves {
                let len = s.entries.len();
                s.scroll.len(len);
                s.scroll.end(len, vp, &anim);
            }
        }

        // SavesLoad, SavesSaveAs, SavesDelete: state-only pre-work here;
        // the actual I/O is caller-handled.

        Action::SavesSaveAs => {
            // Open the save-name dialog (a common-dialog modal, not a bottom-bar
            // prompt); on submit the caller performs the host Save State save.
            state.overlays.hotkey_dialog = false;
            state.overlays.dialog_focus = 0;
            state.overlays.save_name_dialog = Some(crate::state::SaveNameDialog::new(
                crate::persist_files::default_save_name(),
                false,
            ));
        }

        Action::SavesDelete => {
            // Open the two-button confirm-delete dialog for the selected entry;
            // focus starts on Cancel (index 1), the safe default.
            if let Some(s) = &state.overlays.saves {
                if let Some(entry) = s.entries.get(s.scroll.selected) {
                    let path = entry.path.clone();
                    state.overlays.hotkey_dialog = false;
                    state.overlays.dialog_focus = 1;
                    state.overlays.confirm_delete_save = Some(path);
                }
            }
        }

        Action::SavesClose => {
            state.overlays.saves = None;
        }

        // SavesLoad is caller-handled.

        // ── VFS file-picker actions ─────────────────────────────────────────────

        Action::FilePickerNav(delta) => {
            if let Some(fp) = &mut state.overlays.file_picker {
                if delta < 0 { fp.move_up() } else { fp.move_down() }
            }
        }
        Action::FilePickerPick => {
            if let Some(fp) = &state.overlays.file_picker {
                if let Some(name) = fp.selected() {
                    state.filename_submitted = Some(Some(name.to_string()));
                }
            }
            state.overlays.file_picker = None;
        }
        Action::FilePickerClose => {
            // Leave pending_filename set: the run loop's resolver treats a closed
            // picker with a still-pending request as a cancel (NULL fileref).
            state.overlays.file_picker = None;
        }

        // ── File-browser actions ──────────────────────────────────────────────

        // SavesImport, FbEnter are caller-handled.

        Action::FbNav(delta) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(fb) = &mut state.overlays.file_browser {
                if !fb.entries.is_empty() {
                    let len = fb.entries.len();
                    fb.scroll.len(len);
                    // Preserve the existing wrap-around behavior via select().
                    let next = ((fb.scroll.selected as i32 + delta).rem_euclid(len as i32)) as usize;
                    fb.scroll.select(next, vp, &anim);
                }
            }
        }

        Action::FbPage(dir) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(fb) = &mut state.overlays.file_browser {
                let len = fb.entries.len();
                fb.scroll.len(len);
                fb.scroll.page(dir, vp, &anim);
            }
        }

        Action::FbHome => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(fb) = &mut state.overlays.file_browser {
                fb.scroll.len(fb.entries.len());
                fb.scroll.home(vp, &anim);
            }
        }

        Action::FbEnd => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(fb) = &mut state.overlays.file_browser {
                let len = fb.entries.len();
                fb.scroll.len(len);
                fb.scroll.end(len, vp, &anim);
            }
        }

        Action::FbClose => {
            state.overlays.file_browser = None;
        }

        // ── Room dock actions (SQ-0692) ───────────────────────────────────────

        Action::ActivatePane(focus) => {
            // Pane focus only. The room dock is not an overlay and deliberately
            // survives a pane switch — it is a readout you keep up while you play,
            // and the two floating dialogs it replaced closing on every pane change
            // was one of the reasons they were never up when you wanted them.
            state.focus = focus;
        }

        Action::PinRoomDock(id, view) => {
            // Pinning IS selecting (SQ-0692): one fact drives the map highlight,
            // the matrix cross-highlight and the dock header, so they cannot drift
            // apart.
            state.selected_room = Some(id);
            state.open_room_dock(view);
            // Focus deliberately STAYS on the story pane. Taking map focus made
            // every letter a map command (so typing reached nothing) and dimmed the
            // story pane on top of that. The selected-room highlight does not need
            // focus — `render/map.rs` reads only `selected_room`.

            // SQ-0693: in the MATRIX view a click also asks "and how do I walk
            // there from here?". Only there — the drawn map has no leave-by cell
            // to mark, so computing a route for it would buy a toast and nothing
            // else. A route to the room you are already standing in is empty and
            // says nothing, which is the correct amount to say.
            state.room_path.clear();
            if state.map_shows_matrix(&mapper.graph) {
                if let Some(here) = mapper.graph.current() {
                    match mapper::path::route(&mapper.graph, here, id) {
                        Some(steps) => state.room_path = steps,
                        // Falling silent here reads as a broken click: the room
                        // selects, its entrances bold, and nothing says why no
                        // route appeared. A partial route to somewhere nearer
                        // would be worse — it answers a question nobody asked.
                        None => state.set_status("no known route from here"),
                    }
                }
            }
        }

        Action::UnpinRoomDock => {
            state.selected_room = None;
            // The route described THAT selection, so it goes with it.
            state.room_path.clear();
        }

        Action::ClearRoomPath => {
            state.room_path.clear();
        }

        Action::CloseRoomDock => {
            state.close_room_dock();
        }

        Action::SetRoomDockView(view) => {
            state.room_dock_view = view;
        }

        Action::ToggleRoomDock => {
            use crate::state::RoomDockView;
            if state.room_dock.open {
                state.close_room_dock();
            } else {
                state.open_room_dock(RoomDockView::Info);
            }
        }

        // ── Mouse drag-pan actions ────────────────────────────────────────────

        Action::BeginDragPan(col, row) => {
            use crate::state::DragState;
            state.drag = Some(DragState { last: (col, row), acc_x: 0, acc_y: 0 });
        }

        Action::DragPanTo(col, row) => {
            if let Some(drag) = &mut state.drag {
                let dx = col as i32 - drag.last.0 as i32;
                let dy = row as i32 - drag.last.1 as i32;
                drag.last = (col, row);
                // Grab-and-drag: the content follows the cursor (dragging right
                // moves the map right). char_pan is added to the draw offset, so
                // add the delta directly. 1-character precision.
                state.char_pan.0 += dx;
                state.char_pan.1 += dy;
            }
        }

        Action::EndDragPan => {
            state.drag = None;
        }

        Action::StartSelection(col, row) => {
            // Left-down in the story also activates the game pane.
            state.focus = Focus::Game;
            if let Some(g) = state.transcript_geom.get() {
                if let Some(p) = screen_to_point(g, col, row) {
                    state.selection = Some(crate::clipboard::Selection::new(p));
                    state.selection_edge = 0;
                }
            }
        }

        Action::ExtendSelection(col, row) => {
            // A left-drag that never started a story selection extends nothing: any
            // left-drag anywhere reaches this arm (the mouse router has no in-story
            // guard on Drag), so press-and-hold on a map room and slide along the
            // story pane's top/bottom boundary row used to autoscroll the transcript
            // with no selection to show for it. No selection → no edge, no scroll.
            // (SQ-0654)
            if state.selection.is_none() {
                state.selection_edge = 0;
            } else if let Some(g) = state.transcript_geom.get() {
                if let Some(sel) = &mut state.selection {
                    if let Some(p) = screen_to_point(g, col, row) { sel.head = p; }
                }
                // Edge detection for auto-scroll: pointer at/above top → -1; at/below bottom → +1.
                state.selection_edge = if row <= g.area.y { -1 }
                    else if row >= g.area.bottom().saturating_sub(1) { 1 }
                    else { 0 };
                // Step once now so a drag that reaches the edge scrolls even without a tick.
                apply_selection_autoscroll(state);
            }
        }

        // Copy is emitted by the run loop from state.selection_text; just clear here.
        Action::EndSelection => {
            state.selection = None;
            state.selection_edge = 0;
        }

        // ── Transcript scroll ─────────────────────────────────────────────────

        Action::TranscriptScroll(delta) => {
            let target = if delta < 0 {
                state.transcript_scroll.saturating_sub((-delta) as u16)
            } else {
                state.transcript_scroll.saturating_add(delta as u16)
            };
            state.scroll_transcript_to(target);
        }

        Action::ToggleInventory => {
            let opening = !state.show_inventory;
            // Mutually exclusive with the command panel (SQ-1237): opening the
            // inventory panel closes the command panel, exactly as `cycle_panel`
            // does when it lands on `Inventory`.
            if opening {
                open_command_band(state, mapper, false);
            }
            open_inventory_panel(state, opening);
            // Persist per-game, the same rule `Action::OpenCommandBand` follows
            // below — a preference chosen for one story stays with that story.
            if !state.game_dir.as_os_str().is_empty() {
                let next = if opening {
                    crate::state::SidePanel::Inventory
                } else {
                    crate::state::SidePanel::None
                };
                let _ = crate::styles::write_per_game_panel(&state.game_dir, Some(next));
            }
        }

        Action::OpenCommandBand => {
            state.overlays.hotkey_dialog = false;
            // F2 / `/toggle-command-panel` is a TOGGLE (bug fix, SQ-0677): with
            // the band already open (its dock target is `open`, whether or
            // not the slide has settled), the SAME key/command closes it —
            // Esc's ladder must never be the only one-key way out.
            let open = !(state.overlays.command_band.is_some() && state.band_dock.open);
            // Mutually exclusive with the inventory panel (SQ-1237).
            if open {
                open_inventory_panel(state, false);
            }
            open_command_band(state, mapper, open);
            // Persist the panel's state per-game so it is restored the next
            // time this story opens (SQ-1123) — the same rule `Action::ToggleMap`
            // has followed since SQ-0304, and the reason `startup` opens the band
            // through `open_command_band` directly instead of through this action:
            // a global `[command_panel] auto_open` must not silently become one
            // game's pinned override. No game_dir → no sidecar (and it keeps unit
            // tests off the filesystem).
            if !state.game_dir.as_os_str().is_empty() {
                let next = if open {
                    crate::state::SidePanel::Command
                } else {
                    crate::state::SidePanel::None
                };
                let _ = crate::styles::write_per_game_panel(&state.game_dir, Some(next));
            }
        }

        Action::CyclePanel => {
            state.overlays.hotkey_dialog = false;
            cycle_panel(state, mapper);
        }

        Action::BandColumnStep(delta) => {
            // Tab/Shift-Tab move the current column (SQ-0677) — pure
            // movement; a pick happens through `Action::BandClickRow`
            // instead, which `command_band_intercept` reaches for on its own
            // when Tab finds a row highlighted.
            if let Some(b) = &mut state.overlays.command_band {
                b.step_column(delta);
            }
        }

        Action::BandRowNav(delta) => {
            // ↑/↓ move (or start) the explicit row highlight within the
            // current column (SQ-0677), scrolling it into view (SQ-0682).
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(b) = &mut state.overlays.command_band {
                let input = state.input.value.clone();
                b.step_row(&input, delta, vp, &anim);
            }
        }

        Action::BandRowPage(dir) => {
            // PageUp/PageDown page the current column by ~one viewport
            // (SQ-0682), the same standard the story picker and IFDB search
            // modal already page their own lists with.
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(b) = &mut state.overlays.command_band {
                let input = state.input.value.clone();
                b.page_row(&input, dir, vp, &anim);
            }
        }

        Action::BandRowHome => {
            // Home jumps the current column's row highlight to the top (SQ-0682).
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(b) = &mut state.overlays.command_band {
                b.home_row(vp, &anim);
            }
        }

        Action::BandRowEnd => {
            // End jumps the current column's row highlight to the bottom (SQ-0682).
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(b) = &mut state.overlays.command_band {
                b.end_row(vp, &anim);
            }
        }

        Action::BandClickRow(col, idx) => band_pick_row(state, col, idx),

        // The partial-word pre-strip (Tab completing `unl` -> `unlock`
        // rather than appending) lives in `band_pick_row` itself now,
        // shared with `Action::BandClickRow` -- see its doc.
        Action::BandTabPick(col, idx) => band_pick_row(state, col, idx),

        Action::BandFocusCol(col) => {
            if let Some(b) = &mut state.overlays.command_band {
                if b.col_reachable(col) {
                    b.focus = col;
                    b.row_sel = None;
                }
            }
        }

        Action::BandWheel(col, delta) => {
            let anim = state.config.animation.clone();
            if let Some(b) = &mut state.overlays.command_band {
                // The wheel scrolls the HOVERED column, which need not be the
                // one the band is pointing at — so this must move a list
                // without moving the band's own attention.
                //
                // …and it scrolls the LIST, clamping the highlight into the
                // visible window, like every other selection list (SQ-0831).
                // The viewport is that column's own measured list height,
                // published by the render each frame (SQ-0832) — the shared
                // `modal_list_viewport` is the CURRENT column's, which is the
                // wrong column and the wrong number the moment VERB (one row
                // taller, having reclaimed its header) is either of the two.
                let vp = b.col_viewport.get().get(col).copied().unwrap_or(0);
                let len = b.items(col).len();
                b.scroll[col].len(len);
                b.scroll[col].scroll_by(delta as isize, vp, &anim);
            }
        }

        Action::BandQuickPick(_) => {
            // Caller-handled (SQ-0667 amendment, 2026-08-05): a quick pick
            // fires immediately, so the run loop resolves the word
            // (`band_quick_pick_command`) and submits it through the session
            // directly — `apply_action` has no session handle to submit
            // with. Deliberately a no-op here: the whole point of the
            // amendment is that a quick pick does NOT touch the band's
            // in-progress phrase (it's an interjection, not a pick), so there
            // is nothing for this arm to do even in principle.
        }

        Action::InventoryClickRow(idx) => {
            if let Some(word) = state.inventory_click_words.get(idx).cloned() {
                compose_word_onto_prompt(state, &word);
            }
        }

        Action::BandEscape => {
            // Two rungs (SQ-0677): clear an EXPLICIT row highlight, then
            // close. This ladder MUST terminate — a bug fixed alongside this
            // amendment had the first rung re-triggering on every press and
            // making the close rung unreachable, because it checked whatever
            // was VISIBLY highlighted, including the passive typed
            // nearest-match highlight, which just recomputes right back the
            // instant it's "cleared" (nothing about pressing Esc changes the
            // typed text). `row_sel` is explicit-only — set ONLY by `↑`/`↓`
            // (`Action::BandRowNav`), never by typing — so checking IT
            // specifically is what makes two Escs from ANY state close the
            // band, guaranteed.
            let mut close = false;
            if let Some(b) = &mut state.overlays.command_band {
                if b.row_sel.is_some() {
                    b.row_sel = None;
                } else {
                    close = true;
                }
            }
            if close {
                apply_action(Action::BandClose, state, mapper);
            }
        }

        Action::BandClose => {
            // Drawer pattern: keep the content alive so the band visibly slides
            // out; `settle_command_band` clears it once the slide finishes.
            state.band_dock.toggle_to(false, false);
            state.band_dock.arm(&state.config.animation);
        }

        // ── Resize mode actions ───────────────────────────────────────────────

        Action::ResizePanes => {
            let visible = state.resize_targets_visible();
            if let Some(first) = visible.first() {
                state.overlays.hotkey_dialog = false;
                state.resize_mode = true;
                state.resize_target = *first;
            }
        }

        Action::ResizeExit => {
            state.resize_mode = false;
            state.pending_config_write = true;
        }

        Action::ResizeReset => {
            state.reset_pane_sizes();
            state.pending_config_write = true;
        }

        Action::ResizeNav(ResizeNavKind::NextTarget) => state.cycle_resize_target(true),
        Action::ResizeNav(ResizeNavKind::PrevTarget) => state.cycle_resize_target(false),

        Action::ResizeNav(dir) => {
            const STEP: u16 = 3;
            // The limits are shared with the mouse drag (SQ-0669) so the two
            // ways of moving a boundary agree about its range.
            use crate::layout::{
                MAX_INV_DOCK_PCT, MAX_ROOM_DOCK_PCT, MAX_SPLIT_PCT, MIN_INV_DOCK_PCT,
                MIN_ROOM_DOCK_PCT, MIN_SPLIT_PCT,
            };
            use ResizeNavKind::*;
            match state.resize_target {
                crate::state::ResizeTarget::StoryMap => match dir {
                    Left => state.pane_sizes.split_ratio = state.pane_sizes.split_ratio.saturating_sub(STEP).max(MIN_SPLIT_PCT),
                    Right => state.pane_sizes.split_ratio = (state.pane_sizes.split_ratio + STEP).min(MAX_SPLIT_PCT),
                    _ => {}
                },
                crate::state::ResizeTarget::InvDock => match dir {
                    Up => state.pane_sizes.inv_dock_pct = (state.pane_sizes.inv_dock_pct + STEP).min(MAX_INV_DOCK_PCT),
                    Down => state.pane_sizes.inv_dock_pct = state.pane_sizes.inv_dock_pct.saturating_sub(STEP).max(MIN_INV_DOCK_PCT),
                    _ => {}
                },
                // The room dock grows upward out of the map pane, sized as a
                // percentage of the frame exactly like the inventory dock — the
                // map's own floor is enforced at layout time, where the pane's
                // real height is known (SQ-0692).
                crate::state::ResizeTarget::RoomDock => match dir {
                    Up => state.pane_sizes.room_dock_pct = (state.pane_sizes.room_dock_pct + STEP).min(MAX_ROOM_DOCK_PCT),
                    Down => state.pane_sizes.room_dock_pct = state.pane_sizes.room_dock_pct.saturating_sub(STEP).max(MIN_ROOM_DOCK_PCT),
                    _ => {}
                },
                // The command band is a bottom band now (SQ-0664), so it resizes
                // by ROWS like the inventory dock: Up grows, Down shrinks. The
                // value is rows, not a percentage — `band_target_height` still
                // clamps it against the screen at layout time.
                crate::state::ResizeTarget::CommandBand => {
                    use crate::render::command_band::{MAX_BAND_ROWS, MIN_BAND_ROWS};
                    match dir {
                        Up => {
                            state.pane_sizes.band_height =
                                (state.pane_sizes.band_height + 1).min(MAX_BAND_ROWS)
                        }
                        Down => {
                            state.pane_sizes.band_height =
                                state.pane_sizes.band_height.saturating_sub(1).max(MIN_BAND_ROWS)
                        }
                        _ => {}
                    }
                }
            }
            state.sync_pane_sizes_to_config();
        }

        // ── Config screen actions ─────────────────────────────────────────────

        Action::OpenConfig => {
            state.overlays.hotkey_dialog = false;
            state.overlays.dialog_focus = 0;
            let working = clone_config(&state.config);
            state.overlays.config_screen = Some(crate::state::ConfigScreenState {
                working,
                scroll: Default::default(),
            });
        }

        Action::ConfigNav(delta) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(cs) = &mut state.overlays.config_screen {
                let n = CONFIG_ROW_COUNT;
                cs.scroll.len(n);
                // Preserve the existing wrap-around behavior via select().
                let next = ((cs.scroll.selected as i32 + delta).rem_euclid(n as i32)) as usize;
                cs.scroll.select(next, vp, &anim);
            }
        }

        Action::ConfigPage(dir) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(cs) = &mut state.overlays.config_screen {
                cs.scroll.len(CONFIG_ROW_COUNT);
                cs.scroll.page(dir, vp, &anim);
            }
        }

        Action::ConfigHome => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(cs) = &mut state.overlays.config_screen {
                cs.scroll.len(CONFIG_ROW_COUNT);
                cs.scroll.home(vp, &anim);
            }
        }

        Action::ConfigEnd => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(cs) = &mut state.overlays.config_screen {
                cs.scroll.len(CONFIG_ROW_COUNT);
                cs.scroll.end(CONFIG_ROW_COUNT, vp, &anim);
            }
        }

        Action::ConfigToggle => {
            if state.overlays.config_screen.is_some() {
                // Split the borrow: take the selected row, then call helper.
                let selected = state.overlays.config_screen.as_ref().map(|cs| cs.scroll.selected).unwrap_or(0);
                config_toggle_or_edit(selected, state);
            }
        }

        Action::ConfigCycle(delta) => {
            if let Some(cs) = &mut state.overlays.config_screen {
                config_cycle(&mut cs.working, cs.scroll.selected, delta);
            }
        }

        Action::ConfigEdit => {
            if let Some(cs) = &state.overlays.config_screen {
                let field = config_path_field(cs.scroll.selected);
                if let Some(f) = field {
                    let current = match &f {
                        crate::state::ConfigPathField::UserDir => cs.working.user_dir.to_string_lossy().to_string(),
                    };
                    state.overlays.dialog_focus = 0;
                    state.overlays.text_entry = Some(TextEntryDialog::new(
                        TextEntryKind::ConfigEditPath { field: f },
                        current,
                    ));
                }
            }
        }

        Action::ConfigSave => {
            if let Some(cs) = state.overlays.config_screen.take() {
                state.config = clone_config(&cs.working);
                // The config screen edits the GLOBAL honor default; keep the
                // SQ-0318 base in sync so a later reload_style doesn't revert it
                // (a per-game override, if any, still wins on the next reload).
                state.honor_game_colours_base = state.config.honor_game_colours;
                // SQ-0860: the base alone is not enough when a one-run source is
                // holding this key, because `reload_style` only falls back to the
                // base when nothing per-story is speaking. Editing the row calls
                // `one_run.release` (see `one_run_key_for_row`), so a missing pin
                // on a key that had one IS the deliberate edit — end the holds that
                // live on `AppState` too, or the next style reload recomputes the
                // user's own choice straight back off. Untouched rows keep their
                // pin, so saving some unrelated setting changes nothing here.
                if !state.config.one_run.holds(crate::config::keys::HONOR_GAME_COLOURS) {
                    state.game_colours_cli = None;
                    state.artwork_declines_colours = false;
                }
                if let Some(b) = state.audio.as_mut() {
                    b.set_volume(state.config.volume);
                } else if state.config.enable_sound {
                    state.audio = Some(audio::AudioBackend::new(state.config.volume));
                }
                if !state.config.enable_sound {
                    state.reset_sound_sidecars();
                }
                // Sync the running Glulx VM's Sound gestalt (applied by the event loop).
                state.pending_vm_sound = Some(state.config.enable_sound);
                // Reconcile the style file-watcher live (the run loop owns it).
                state.pending_watch_style = Some(state.config.watch_style);
                // SQ-1161: two settings are mirrored onto `AppState` at boot and
                // read from THERE by render — `startup.rs` seeds both and the
                // toggle keys drive the mirror, not the config. Saving the row
                // without lowering it wrote config.toml and changed nothing on
                // screen until the next launch, which is exactly the silent
                // half-application the screen's contract forbids.
                state.show_status_bar = state.config.show_status_bar;
                state.show_room_numbers = state.config.show_room_numbers;
                // SQ-1161: and four more keys keep a `_base` on `AppState` — the
                // GLOBAL default a per-story source overrides for one launch, and
                // what `/set-guidance auto` (and its siblings) fall back to. The
                // honour row's base is lowered above for the same reason; without
                // these, saving the row moved the live value and left `auto`
                // pointing at the value the session started with.
                //
                // Only when nothing per-story is pinning the key: a pin means the
                // row was NOT edited (editing releases it, above), so `working`
                // still holds someone else's value for this run and lowering it
                // would turn one game's choice into everyone's (SQ-0807).
                if !state.config.one_run.holds(crate::config::keys::GUIDANCE) {
                    state.guidance_base = state.config.guidance;
                }
                if !state.config.one_run.holds(crate::config::keys::RETURN_PROBE) {
                    state.return_probe_base = state.config.return_probe;
                }
                if !state.config.one_run.holds(crate::config::keys::V6_PIXEL_LOCK) {
                    state.v6_pixel_lock_base = state.config.v6_pixel_lock;
                }
                if !state.config.one_run.holds(crate::config::keys::V6_RENDER) {
                    state.v6_render_base = state.config.v6_render;
                }
                // Re-resolving the live look is caller-handled, and deliberately
                // runs AFTER `write_config_file` (SQ-1161). It used to happen here
                // as a bare global `load_style` + `resolve`, which dropped the
                // per-game style overlay and the garglk overlay from the live look
                // and never recomputed `state.period_look` — so saving the
                // `period_look` row did nothing until something else happened to
                // reload the style. `reload::reload_style` is the ONE place the
                // theme is built and fixes all three, but it also recomputes
                // `honor_game_colours` from this story's sidecar and re-PINS it,
                // and `ConfigDoc::put` skips a pinned key: run here, it would
                // silently drop the honour row's edit from the file it was just
                // asked to write. Ordering is the whole fix — the file first, then
                // the story's own overrides back over the top of the live look.
                // The style-file write + config repoint is caller-handled
                // (main.rs snapshots working before this runs).
            }
        }

        Action::ConfigCancel => {
            state.overlays.config_screen = None;
        }

        Action::ResetGame => {
            // Open the reset dialog; the caller (main.rs) handles confirm/cancel/clear-map.
            state.overlays.hotkey_dialog = false;
            state.overlays.reset_dialog = true;
            state.overlays.reset_clear_map = false;
            state.overlays.reset_delete_data = false;
            state.overlays.dialog_focus = 0;
        }

        // Caller-handled: silently ignored.
        Action::SubmitCommand(_)
        | Action::SaveGame
        | Action::RestoreGame
        | Action::ExportSvg(_)
        | Action::ExportDot(_)
        | Action::ExportMap(_)
        | Action::SavesLoad
        | Action::SavesImport
        | Action::FbEnter
        | Action::TranscriptScrollPage(_)
        | Action::TranscriptScrollHalfPage(_)
        | Action::PagerAdvance
        | Action::PagerDismiss
        | Action::Quit => {}

        // Caller-handled (needs session scope): opens the hints panel in main.rs.
        Action::OpenHints => {}

        // ── Replay / rewind actions ───────────────────────────────────────────

        Action::OpenHistory => {
            // Three outcomes, and the point of SQ-1091 is that two of them used to
            // be one silent no-op: "there is nothing to replay" and "the thing
            // that would have filled it is switched off" look identical from here,
            // and only the second is something the player can act on.
            state.overlays.hotkey_dialog = false;
            if !state.history.is_empty() {
                state.overlays.replay = Some(crate::state::ReplayState::new(state.history.len() - 1));
            } else if !state.config.record_turn_history {
                state.overlays.history_prompt = true;
                state.overlays.dialog_focus = 0;
            } else {
                // Recording IS on and the history is still empty, which is only
                // true before the first move. Saying so beats a dialog offering to
                // switch on what is already on.
                //
                // SQ-1045: this was a bracketed TOAST, and it was the one line in
                // the tree wearing the Z-machine parser's own `[…]` voice while
                // firing mid-play in answer to something the player did — exactly
                // the impersonation the assist register exists to end. It is also
                // help rather than a report: it does not say a thing failed, it
                // says what to do to get what was wanted. So it is an assist, and
                // being one it now persists in the transcript (a toast that
                // expires is the worst surface for advice), carries the marker
                // into a copy-paste and a screen reader, and can be hidden with
                // the rest of them by `/filter story`.
                state.push_assist(&crate::assist::Assist::help(
                    "nothing to rewind yet — there will be after your next move.",
                ));
            }
        }

        Action::ReplayStep(delta) => {
            let len = state.history.len();
            if let Some(r) = &mut state.overlays.replay {
                r.step(delta, len);
            }
        }

        Action::ReplayPage(dir) => {
            let len = state.history.len();
            // Page by one list viewport (1-row overlap), clamped by step().
            let page = (state.modal_list_viewport.max(2) - 1) as isize;
            if let Some(r) = &mut state.overlays.replay {
                r.step(dir as isize * page, len);
            }
        }

        Action::ReplayTogglePlay => {
            if let Some(r) = &mut state.overlays.replay {
                r.toggle_play();
            }
        }

        Action::ReplayClose => {
            state.overlays.replay = None;
        }

        // ReplayResume is caller-handled in main.rs (needs the live session/VM).
        Action::ReplayResume => {}

        Action::None => {}
        // Note: OpenHotkeyDialog and CloseHotkeyDialog are handled above.
    }
}

// ── Story-pane selection helpers (SQ-0197) ────────────────────────────────────

/// Map a story-pane screen cell to an absolute wrapped-transcript Point, clamped to
/// the visible rows and the story band. `None` if geometry is degenerate. (SQ-0197)
pub(crate) fn screen_to_point(g: crate::clipboard::TranscriptGeom, col: u16, row: u16)
    -> Option<crate::clipboard::Point> {
    if g.area.width == 0 || g.area.height == 0 { return None; }
    let dy = row.saturating_sub(g.area.y).min(g.area.height.saturating_sub(1));
    let abs = (g.first_abs_row + dy as usize).min(g.total_rows.saturating_sub(1));
    let c = col.saturating_sub(g.area.x).min(g.area.width.saturating_sub(1));
    Some(crate::clipboard::Point { row: abs, col: c })
}

/// While a selection drag sits at an edge, scroll one wrapped row toward it and
/// advance the head in lockstep so the selection keeps growing. No-op at scroll
/// limits or when not selecting. (SQ-0197)
pub fn apply_selection_autoscroll(state: &mut AppState) {
    // "Not selecting" is a real state the drag path can reach (a drag that began
    // outside the story pane), and scrolling the transcript for a drag that selects
    // nothing is exactly the SQ-0654 symptom — so honour the doc comment here too,
    // not only at the one call site that remembered to check.
    if state.selection_edge == 0 || state.selection.is_none() { return; }
    let Some(g) = state.transcript_geom.get() else { return };
    let max_scroll = g.total_rows.saturating_sub(g.area.height as usize) as u16;
    let cur = state.transcript_scroll;
    let next = if state.selection_edge < 0 { cur.saturating_add(1).min(max_scroll) }
               else { cur.saturating_sub(1) };
    if next == cur { return; } // at a limit
    state.scroll_transcript_to(next);
    if let Some(sel) = &mut state.selection {
        // Top edge reveals an older row above → head.row moves up by 1; bottom edge
        // reveals a newer row below → head.row moves down by 1.
        if state.selection_edge < 0 { sel.head.row = sel.head.row.saturating_sub(1); }
        else { sel.head.row = (sel.head.row + 1).min(g.total_rows.saturating_sub(1)); }
    }
}

// ── Suggestion recompute ──────────────────────────────────────────────────────

/// Extend [`AppState::seen_words`] with the words the story has printed SINCE
/// the last call that its own dictionary holds.
///
/// Called once a turn, and once at boot, from wherever the engine is in hand —
/// the answer needs the engine twice over, to split the prose and to say what a
/// word is, and neither is reachable from a key handler holding only `AppState`.
///
/// # It accumulates, and it does not persist (SQ-1135)
///
/// This used to recompute over a twenty-line window, so a word scrolled OUT of
/// the list as the transcript moved on: Arthur names the sliver of crystal in
/// the torque's knob exactly once, and a few turns later there was no way to
/// reach the word again. Now each call scrapes only the lines past
/// [`AppState::seen_scanned`] and folds them into what is already there, so the
/// per-turn cost is the same and nothing is forgotten.
///
/// New words go on the FRONT — **most recently printed first** — because the
/// case this exists for is the word the story printed a moment ago, and a word
/// printed again moves back to the front rather than keeping its old place.
/// (Order is free for completion, which re-sorts its own tier in
/// [`crate::complete::suggest`]; it is the command band's noun columns that read
/// it as an order.)
///
/// **Nothing new is persisted.** The set is derived from the transcript, which
/// the archive already carries, and
/// [`AppState::reset_transcript_sidecars`](crate::state::AppState::reset_transcript_sidecars)
/// drops it wherever the transcript is replaced — so a restore rebuilds from the
/// transcript it restored, and restoring to before a word was printed correctly
/// takes the word away.
///
/// **Every engine is served, and each says how far it can go:**
///
/// | engine | splitting | "is this a word" |
/// |---|---|---|
/// | Z-machine | its own `dictionary::tokenise`, the routine `read` calls | its own encoder, so §13.3's six / §13.4's nine **Z-character** truncation is exact |
/// | Glulx | [`complete::split_prose`] | the [`StoryVocabulary`] snapshot from `gvm::grammar`, cut to `DICT_WORD_SIZE` — exact, Glulx truncating by plain characters |
/// | Scott Adams | [`complete::split_prose`] | the same snapshot, from the database's own verb/noun lists cut to its header word length |
/// | anything with neither | [`complete::split_prose`] | nothing is known, so nothing is offered |
///
/// The Z-machine reaches this too, even though the command band never uses the
/// result there (it has a live object tree): Tab completion is every engine's.
///
/// [`complete::split_prose`]: crate::complete::split_prose
/// [`StoryVocabulary`]: crate::vocab::StoryVocabulary
pub fn refresh_seen_words(state: &mut AppState, engine: &dyn crate::engine::Engine) {
    // A transcript shorter than the cursor is one that was replaced without the
    // sidecar reset; rebuild rather than index past the end.
    if state.seen_scanned > state.transcript.len() {
        state.seen_words.clear();
        state.seen_nouns.clear();
        state.seen_scanned = 0;
    }
    if state.seen_scanned == state.transcript.len() {
        return;
    }
    let text = state.transcript[state.seen_scanned..].join(" ");
    state.seen_scanned = state.transcript.len();
    let tokens = engine
        .split_like_parser(&text)
        .unwrap_or_else(|| crate::complete::split_prose(&text));
    // The story's whole vocabulary of THINGS, asked once a turn (a walk of the
    // object table, so never per frame) — see
    // [`Introspect::all_object_words`](crate::engine::Introspect::all_object_words)
    // for why the dictionary's noun bit cannot do this job on every story.
    // As the folded any-object SET, not the `Vec<ObjectWords>`: `is_thing`
    // below never asks WHICH object answers, and walking the vec re-truncated
    // the story's whole vocabulary for every fresh word (SQ-1176).
    let objects = engine.object_word_set();
    // Newest first: walk the batch backwards and keep the first sighting of each
    // word, which is its LAST printing.
    let (fresh, fresh_nouns): (Vec<String>, Vec<String>) = {
        // The snapshot is read once a session and cached here; asking for it is
        // free after the first turn.
        let vocab = state.vocab.get(engine);
        // Is this word a THING rather than an action or a joining word?
        //
        // The story's own objects where they can be read, which is exact and
        // needs no flag layout — the Z-machine's, and since SQ-1210 Glulx's
        // too (`Engine::object_word_set`). Otherwise the dictionary's role
        // bits, which is what Scott and an unreadable Glulx image have: a word
        // the story marks a NOUN and does not write literally into a grammar
        // line (SQ-1042). Measured on `stories/advent.blb` at the opening room
        // — back when that title still took this branch — that
        // cuts 20 scraped words to 12 — `at`, `in`, `of`, `to`, `down` and `out`
        // are prepositions the grammar writes down, `don` and `release` are
        // verbs. What it does NOT reach is Inform's `a`, `and` and `the`, which
        // carry the noun bit and NOTHING else (flag byte $80, exactly like
        // `brick`), so no role reading can separate them — which is the second
        // reason the objects are the better question where they exist.
        let is_thing = |w: &str| match &objects {
            Some(set) => set.contains(w),
            None => vocab.is_some_and(|v| v.roles(w).is_some_and(|r| r.noun && !r.preposition)),
        };
        let mut out: Vec<String> = Vec::new();
        let mut things: Vec<String> = Vec::new();
        for w in tokens.iter().rev() {
            let w = w.to_lowercase();
            if !w.chars().any(char::is_alphanumeric) {
                continue;
            }
            if out.contains(&w) {
                continue;
            }
            if engine.knows_word(&w).unwrap_or_else(|| vocab.is_some_and(|v| v.knows(&w))) {
                if is_thing(&w) {
                    things.push(w.clone());
                }
                out.push(w);
            }
        }
        (out, things)
    };
    if fresh.is_empty() {
        return;
    }
    state.seen_words.retain(|w| !fresh.contains(w));
    state.seen_words.splice(0..0, fresh);
    state.seen_nouns.retain(|w| !fresh_nouns.contains(w));
    state.seen_nouns.splice(0..0, fresh_nouns);
}

/// Recompute [`AppState::scope_words`]: the words the parser accepts for the
/// things that are in the room and in the player's hands.
///
/// This is the completion source SQ-1042 was raised for. `Introspect` walks the
/// live object tree from the player's room outward — the floor, an open
/// container standing on it, the room's shared scenery — and hands back each
/// object's own parse names; [`crate::vocab::typeable_words`] spells them out
/// where the printed name can (Zork I stores `lanter` and prints `lantern`) and
/// adds the adjective Infocom keeps in a property of its own.
///
/// **Nothing here is a spoiler.** It names what the game itself would list in
/// answer to `look` and `inventory`, and nothing else: the walk stops at a
/// closed container's lid, so a thing the player has not yet opened is not a
/// thing they can complete. That line is what separates a convenience from a
/// puzzle solver, and it is the object tree's to draw, not ours.
///
/// Empty for an engine with no introspection (Glulx, Scott) — completion there
/// still has the recent-prose scrape and the flat dictionary.
pub fn refresh_scope_words(state: &mut AppState, engine: &dyn crate::engine::Engine) {
    let Some((mut objects, carried)) = crate::vocab::scope_split(engine, state.player_obj) else {
        state.scope_words.clear();
        return;
    };
    objects.extend(carried);
    let vocab = state.vocab.get(engine);
    let mut words: Vec<String> = Vec::new();
    for o in &objects {
        for w in crate::vocab::typeable_words(o, vocab) {
            if !words.iter().any(|x| x.eq_ignore_ascii_case(&w)) {
                words.push(w);
            }
        }
    }
    words.sort_unstable();
    state.scope_words = words;
}

/// Return up to `limit` names from `names` containing `body_token` ANYWHERE (case-insensitive),
/// best match first. A name equal to the token is never offered — it is already typed in full.
///
/// Matching was prefix-only (SQ-0353). But command names are compound — `toggle-room-numbers`,
/// `select-room` — and the part anyone actually remembers is the noun, not the verb the name
/// happens to begin with. Typing the one word you could recall found nothing at all: `room` matched
/// no command, though four contain it.
///
/// Ranking is what keeps substring matching from becoming a free-for-all — the obvious answer must
/// still come first:
///   0. the name starts with the token (a plain prefix hit, the old behaviour),
///   1. a name-PART starts with it (right after a `-`) — `room` in `toggle-room-numbers`,
///   2. it appears mid-word — `map` in `unmappable`.
///      Alphabetical within a rank, so the list is stable and predictable as you type.
pub(crate) fn slash_suggestions(body_token: &str, names: &[String], limit: usize) -> Vec<String> {
    if body_token.is_empty() || limit == 0 {
        return Vec::new();
    }
    let lower = body_token.to_lowercase();
    let mut matches: Vec<(u8, String)> = names
        .iter()
        .filter_map(|n| {
            let ln = n.to_lowercase();
            if ln == lower {
                return None; // already typed in full — suggesting it back is noise
            }
            let at = ln.find(&lower)?;
            // `at` is a byte index; step back by CHAR so a non-ASCII token can't split one.
            let rank = match ln[..at].chars().next_back() {
                None => 0,       // prefix
                Some('-') => 1,  // start of a name-part
                Some(_) => 2,    // mid-word
            };
            Some((rank, n.clone()))
        })
        .collect();
    matches.sort_unstable();
    matches.dedup();
    matches.truncate(limit);
    matches.into_iter().map(|(_, n)| n).collect()
}

/// Apply `completion` to the input line, replacing whatever the caret is currently completing:
/// the whole slash-command name, or the partial word at the end. The caret lands after the
/// inserted text.
///
/// Shared by Tab and Shift-Tab, which carried a verbatim copy of this each (SQ-0354).
///
/// Lengths are counted in CHARS, not bytes. The byte arithmetic this replaces would panic outright
/// on a multi-byte partial word: `String::truncate` rejects a non-char boundary, and subtracting a
/// byte length from a byte length lands on one as soon as the word holds anything non-ASCII.
fn apply_completion(state: &mut AppState, completion: &str) {
    let prefix = state.config.command_prefix;
    // Slash-command suggestions hold the bare name (no prefix). When completing the first token of
    // a slash command, rebuild the line as prefix + name so the leading prefix survives.
    let is_slash_name = state.input.value.starts_with(prefix)
        && !state.input.value[prefix.len_utf8()..].contains(' ');
    if is_slash_name {
        state.input.clear();
        state.input.insert(prefix);
    } else {
        let keep = state.input.char_len() - state.current_partial().chars().count();
        state.input.truncate_chars(keep);
        state.input.end();
    }
    state.input.insert_str(completion);
}

/// Recompute `state.suggestions` from `state.dict_words`, the story's own words
/// in its recent output (`state.seen_words`, refreshed once a turn by
/// [`refresh_seen_words`]), and the current partial word being typed.
/// Called internally after every input character change in game focus.
///
/// When the input starts with `state.config.command_prefix`, completes the
/// first token after the prefix from `slash::slash_names()` instead of the
/// dictionary.
pub(crate) fn recompute_suggestions(state: &mut AppState) {
    const SUGGESTION_LIMIT: usize = 6;
    let prefix = state.config.command_prefix;
    // Check if the whole input starts with the command prefix.
    if state.input.value.starts_with(prefix) {
        // Extract the body (everything after the prefix).
        let body = &state.input.value[prefix.len_utf8()..];
        // Complete only the first token (before any space).
        let first_token = body.split_whitespace().next().unwrap_or("");
        // Only offer completions while the user is still on the first token
        // (no space yet in the body, or trailing chars still form the first word).
        let body_has_space = body.contains(' ');
        if body_has_space {
            // Command name already chosen; no further name completions.
            state.suggestions.clear();
            return;
        }
        let names = crate::slash::slash_names();
        state.suggestions = slash_suggestions(first_token, &names, SUGGESTION_LIMIT);
        return;
    }
    let partial = state.current_partial().to_owned();
    if partial.is_empty() {
        state.suggestions.clear();
        return;
    }
    // Three tiers, best first (SQ-1042): the words for the things that are
    // ACTUALLY HERE, then the words the story has just printed, then the flat
    // dictionary. `suggest` ranks its second argument above its first, so the
    // scope pass runs against no dictionary at all and the prose pass fills
    // whatever room is left — a player typing `lan` in Zork I's Living Room
    // wants the lantern in front of them before the four hundred words the
    // story also knows.
    let mut hits = suggest(&[], &state.scope_words, &partial, SUGGESTION_LIMIT);
    if hits.len() < SUGGESTION_LIMIT {
        for w in suggest(&state.dict_words, &state.seen_words, &partial, SUGGESTION_LIMIT) {
            if !hits.iter().any(|h| h.eq_ignore_ascii_case(&w)) {
                hits.push(w);
            }
            if hits.len() == SUGGESTION_LIMIT {
                break;
            }
        }
    }
    state.suggestions = hits;
}

// ── Bracketed paste (SQ-0653) ─────────────────────────────────────────────────

/// Flatten pasted text into something a single-line field can hold.
///
/// A paste is the terminal handing us a block of text; the fields it can land in
/// are all single-line. Line breaks (`\r\n`, `\n`, `\r`) and tabs collapse to a
/// single space each — a CRLF counts once — and every other control character is
/// dropped. Nothing is trimmed: leading/trailing spaces are the user's, and a
/// paste that is *only* whitespace still inserts that whitespace.
///
/// Newlines deliberately do NOT submit (SQ-0653). Before bracketed paste was
/// enabled the terminal replayed a paste as keystrokes and every newline fired a
/// turn — a multi-line paste played several moves before the player could look at
/// it. The text now lands in the field and waits for Enter.
pub fn sanitize_pasted_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next(); // CRLF is one break
                }
                out.push(' ');
            }
            '\n' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Insert bracketed-paste text into whichever text field currently owns typing,
/// as literal characters (SQ-0653). Returns true when the paste landed somewhere.
///
/// The priority order mirrors the run loop's own intercept ladder, so a paste
/// goes exactly where the next typed character would have gone: the top-most open
/// modal that HAS a field takes it, a modal without one swallows it (typing does
/// nothing there either), and with nothing open it reaches the game's input line.
/// Nothing here submits — the user reviews the text and presses Enter.
pub fn apply_paste(state: &mut AppState, text: &str) -> bool {
    let text = sanitize_pasted_text(text);
    if text.is_empty() {
        return false;
    }
    // Common-dialog ladder, in `overlays::topmost_common_dialog` order:
    // aux ▸ reset/game-over ▸ save-name ▸ text-entry ▸ confirm-delete ▸ quit ▸ launch.
    if state.overlays.aux_prompt || state.overlays.reset_dialog || state.overlays.game_over {
        return false;
    }
    // Field slot only (focus 0) — a button-focused dialog ignores typing too.
    let name_field_focused = state.overlays.dialog_focus == 0;
    if let Some(dlg) = state.overlays.save_name_dialog.as_mut() {
        if !name_field_focused {
            return false;
        }
        if !dlg.active {
            // Pasting onto the greyed placeholder replaces it, exactly as typing does.
            dlg.field.set(String::new(), false);
            dlg.active = true;
        }
        dlg.field.insert_str(&text);
        return true;
    }
    if let Some(dlg) = state.overlays.text_entry.as_mut() {
        dlg.field.insert_str(&text);
        return true;
    }
    if state.overlays.confirm_delete_save.is_some()
        || state.overlays.quit_dialog
        || state.overlays.launch_dialog
    {
        return false;
    }
    // Hints panel: its own line buffer, ahead of the palette in the ladder.
    if let Some(hs) = state.overlays.hints.as_mut() {
        hs.input.push_str(&text);
        return true;
    }
    let vp = state.modal_list_viewport;
    let anim = state.config.animation.clone();
    if let Some(p) = state.overlays.palette.as_mut() {
        p.input.insert_str(&text);
        // A changed query re-ranks the list; snap the selection to the top
        // (mirrors `Action::PaletteChar`).
        let len = crate::complete::palette_candidates(p.query()).len();
        p.scroll.len(len);
        p.scroll.select(0, vp, &anim);
        return true;
    }
    // The command band never intercepts a paste (SQ-0676): it owns no text
    // field anymore — text goes to the story input line, exactly as typing
    // does, and the band re-reads that line afterwards. (SQ-0664 had already
    // stopped the old dock from swallowing pastes outright; SQ-0667's filter
    // took them for a while; now nothing between the paste and the prompt.)
    //
    // Any other MODAL overlay (saves manager, file browser, config screen,
    // replay, hotkey dialog, resize mode…) owns the keyboard but has no text
    // field: swallow, rather than typing behind the modal. The corner overlays the
    // modal test excludes (room panel, tidy animation) leave the story input line
    // live, so a paste still reaches it there — same rule typing follows.
    if state.any_modal_overlay_open() {
        return false;
    }
    state.input.insert_str(&text);
    // Same tail as `Action::InputChar`: a changed line re-ranks the completions,
    // and (SQ-0676) re-points the open command band at it.
    if state.focus == Focus::Game {
        recompute_suggestions(state);
        state.suggestion_idx = 0;
        state.suggestion_active = false;
    }
    band_react_to_input(state);
    true
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Apply a submitted text-entry dialog. Byte-identical to the retired
/// `apply_prompt`, per kind (SQ-0307):
///   - map-edit kinds mutate the mapper and bump `graph_gen` so the edit shows
///     this frame instead of waiting for the next turn (the Wave-1 choke);
///   - `ConfigEditPath` writes the config-screen working copy;
///   - `CreateFile` flag-hops the chosen filename (empty → cancel) to the run
///     loop's `resolve_filename_request`.
///
/// Empty-value semantics are preserved per kind: RenameRoom empty clears the
/// custom label; EditNotes empty clears the notes; RelabelEdge empty (or an
/// unparseable direction) is a no-op; RenameLayer / ConfigEditPath store the
/// empty string; CreateFile empty cancels the request.
pub fn apply_text_entry(dlg: TextEntryDialog, state: &mut AppState, mapper: &mut Mapper) {
    let value = dlg.field.value;
    match dlg.kind {
        TextEntryKind::RenameRoom(id) => {
            let label = if value.is_empty() { None } else { Some(value) };
            mapper.rename_room(id, label);
            state.bump_graph_gen(); // a graph-mutating edit was applied (SQ-0305)
        }
        TextEntryKind::EditNotes(id) => {
            mapper.set_notes(id, value);
            state.bump_graph_gen();
        }
        TextEntryKind::RelabelEdge(id, old_dir) => {
            // Parse the user's input as a direction name.
            if let Some(new_dir) = mapper::direction::parse_direction(&value) {
                mapper.relabel_edge(id, old_dir, new_dir);
            }
            state.bump_graph_gen();
        }
        TextEntryKind::RenameLayer(id) => {
            mapper.graph.set_layer_name(id, value);
            state.bump_graph_gen();
        }
        TextEntryKind::ConfigEditPath { field } => {
            if let Some(cs) = &mut state.overlays.config_screen {
                match field {
                    crate::state::ConfigPathField::UserDir => {
                        // Typing a path is the deliberate act `--user-dir` was not,
                        // so it ends any one-run hold on the key (SQ-0807).
                        cs.working.user_dir = std::path::PathBuf::from(&value);
                        cs.working.one_run.release(crate::config::keys::USER_DIR);
                    }
                }
            }
        }
        TextEntryKind::CreateFile => {
            state.filename_submitted =
                Some(if value.trim().is_empty() { None } else { Some(value) });
        }
    }
}

/// Select the next (+1) or previous (-1) room, cycling through all room ids in
/// ascending sorted order.
fn select_adjacent(state: &mut AppState, mapper: &Mapper, delta: i32) {
    let ids: Vec<_> = mapper.graph.rooms().map(|r| r.id).collect();
    if ids.is_empty() {
        return;
    }
    // ids come from BTreeMap iteration so they are already sorted ascending.
    let new_id = match state.selected_room {
        None => {
            if delta >= 0 {
                ids[0]
            } else {
                *ids.last().unwrap()
            }
        }
        Some(current) => {
            let idx = ids.iter().position(|&id| id == current).unwrap_or(0);
            let len = ids.len() as i32;
            let next = ((idx as i32) + delta).rem_euclid(len) as usize;
            ids[next]
        }
    };
    state.select_room(Some(new_id));
}

/// Re-center the map on the selected room, or — with nothing selected — on the room the player is
/// currently in (SQ-0349).
///
/// The origin is only a last resort now. It used to be the fallback for "nothing selected", which
/// is an arbitrary corner of the map that need not hold a room at all: the common case (no
/// selection, just wanting the view back on yourself) threw the map somewhere useless. A selection
/// that has no position falls through to the current room for the same reason — it cannot be
/// centred on, but the origin is not a better answer than where the player is standing.
///
/// Centres against `state.map_pane_size` — the map pane as the renderer last measured it, since
/// `apply_action` never sees the run loop's `last_panes`. Falls back to 80×24 only before the first
/// frame, or while the map pane is hidden: `recenter_on` divides the pane by the zoom step, so a
/// guessed size puts the target off-centre on any real pane (SQ-0349).
/// Where `move-region` was told to put the rooms, as the player named it (SQ-0439).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDest {
    /// Nothing was named. Auto-pick when only one destination is possible, ask when more are —
    /// the same rule the seam follows, applied to where the rooms land (SQ-0439).
    Auto,
    /// A fresh layer — the old `peel-layer`.
    New,
    /// The layer the region's own was peeled from — the old bare `merge-layer`.
    Parent,
    /// Any layer, by name — the old `merge-layer <name>`.
    Layer(mapper::layer::LayerId),
}

/// Resolve one word or phrase to a destination. `Ok(None)` means "no layer goes by that name",
/// which is a fallthrough rather than an error, because the caller may still read its last word
/// as a seam direction. An AMBIGUOUS name is a hard error: layer names are not unique (a peel
/// names its layer after a room label), so guessing between two would move rooms somewhere the
/// player did not ask for.
fn resolve_move_dest(
    graph: &mapper::graph::MapGraph,
    name: &str,
) -> Result<Option<MoveDest>, String> {
    if name.eq_ignore_ascii_case("new") {
        return Ok(Some(MoveDest::New));
    }
    if name.eq_ignore_ascii_case("parent") {
        return Ok(Some(MoveDest::Parent));
    }
    let ids: Vec<mapper::layer::LayerId> = graph
        .layers()
        .iter()
        .filter(|(_, m)| m.name.eq_ignore_ascii_case(name))
        .map(|(id, _)| *id)
        .collect();
    match ids.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(MoveDest::Layer(*one))),
        many => Err(format!(
            "move-region: {} layers are named '{name}' — rename one first",
            many.len()
        )),
    }
}

/// Split `move-region`'s argument into a destination and the seam to cut at, if one was named.
///
/// A LIVE layer name wins over a speculative trailing direction, so a layer actually called
/// "Dead End North" resolves whole; only when the words name no destination at all is the last
/// one read as a seam. That ordering is the only reason this cannot happen in `slash.rs`, which
/// dispatches without a graph to ask.
///
/// Both halves may be left out. No words at all is the bare command — seam and destination both
/// auto-picked. A lone direction names the seam and leaves the destination to the auto-pick, which
/// is what lets each refusal suggest a command that fixes only the ambiguity it is complaining
/// about (`move-region north`, `move-region new`) instead of demanding both at once.
fn parse_move_region_arg(
    graph: &mapper::graph::MapGraph,
    arg: &str,
) -> Result<(MoveDest, Option<Direction>), String> {
    let words: Vec<&str> = arg.split_whitespace().collect();
    if words.is_empty() {
        return Ok((MoveDest::Auto, None));
    }
    let whole = words.join(" ");
    if let Some(dest) = resolve_move_dest(graph, &whole)? {
        return Ok((dest, None));
    }
    if words.len() == 1 {
        if let Some(dir) = mapper::direction::parse_direction(words[0]) {
            return Ok((MoveDest::Auto, Some(dir)));
        }
    }
    if words.len() >= 2 {
        if let Some(dir) = mapper::direction::parse_direction(words[words.len() - 1]) {
            let head = words[..words.len() - 1].join(" ");
            return match resolve_move_dest(graph, &head)? {
                Some(dest) => Ok((dest, Some(dir))),
                None => Err(format!("move-region: no layer named '{head}'")),
            };
        }
    }
    Err(format!("move-region: no layer named '{whole}'"))
}

/// Why the rooms to move could not be settled — a refusal that belongs to the SEAM, not to the
/// destination (SQ-0439).
enum SeamRefusal {
    /// The seam was named and the region walk turned it down; the pair is the passage tried.
    Region(mapper::layer::RegionRefusal, Option<(mapper::graph::RoomId, Direction)>),
    /// Several passages lead into the room and each cuts a different map. Nothing to auto-pick
    /// from, so the player says which. (Lane D turns this list into a picker; until then it is a
    /// refusal that names the candidates and the command that resolves them.)
    Ambiguous(Vec<mapper::layer::InboundSeam>),
}

/// Which rooms move, in three tiers, anchored on the SELECTED room (SQ-0439).
///
/// The old surface anchored on the room you were STANDING IN and asked for the direction pointing
/// back out of the area you wanted — inverted from how a player thinks about it, and unsayable
/// when the way in was one-way. Anchoring on the selection dissolves both, because the seam can
/// then be found rather than named:
///
/// 1. **Portal boundary.** [`mapper::layer::planar_region`] walks the compass edges and stops at
///    portals, which is a floor, a cellar, a tower — the common case, and it needs no input beyond
///    which room was picked. Accepted whenever it is a proper part of its layer.
/// 2. **The unique inbound bridge.** When the walk covers the whole layer there is no boundary to
///    find that way, so look at the passages leading INTO the room. Exactly one is a real seam →
///    cut it. One-way passages are included, which is precisely the case that had no direction to
///    name. A layer with NO inbound seam falls back to the whole-layer region and lets the move
///    itself answer — folding a whole layer into another is a merge, not an error.
/// 3. **Several inbound bridges.** Ambiguous, so ask.
///
/// A named direction skips the tiers and resolves the seam directly: an inbound passage of that
/// direction first — so the command a tier-3 refusal suggests means what the list said it meant —
/// falling back to the passage of that direction OUT of the room, which is the only way to name a
/// one-way exit.
fn choose_region(
    graph: &mapper::graph::MapGraph,
    room: mapper::graph::RoomId,
    dir: Option<Direction>,
) -> Result<(mapper::layer::Region, Option<(mapper::graph::RoomId, Direction)>), SeamRefusal> {
    if let Some(d) = dir {
        let named: Vec<mapper::layer::InboundSeam> = mapper::layer::inbound_seams(graph, room)
            .into_iter()
            .filter(|s| s.dir == d)
            .collect();
        return match named.as_slice() {
            [one] => Ok((one.region.clone(), Some((one.from, d)))),
            [] => mapper::layer::region_at_edge(graph, room, d)
                .map(|r| (r, Some((room, d))))
                .map_err(|why| SeamRefusal::Region(why, Some((room, d)))),
            _ => Err(SeamRefusal::Ambiguous(named)),
        };
    }
    let region = mapper::layer::planar_region(graph, room);
    if !mapper::layer::is_whole_layer(graph, &region) {
        return Ok((region, None)); // tier 1
    }
    match mapper::layer::inbound_seams(graph, room).as_slice() {
        [one] => Ok((one.region.clone(), Some((one.from, one.dir)))), // tier 2
        // Nothing divides this layer at all. The whole-layer region is still the right answer for
        // a merge, and `move_region` says so better than a seam refusal could.
        [] => Ok((region, None)),
        many => Err(SeamRefusal::Ambiguous(many.to_vec())), // tier 3
    }
}

/// The layers `region` could legally land on, `new` included — what a bare `move-region` picks from
/// when exactly one is possible (SQ-0439).
///
/// Mirrors [`mapper::layer::move_region`]'s own refusals rather than inventing a second rule: a new
/// layer is only a rename when the region is its whole layer, and Main may be moved out of but
/// never emptied.
fn move_targets(
    graph: &mapper::graph::MapGraph,
    region: &mapper::layer::Region,
) -> Vec<mapper::layer::MoveTarget> {
    use mapper::layer::{MoveTarget, MAIN_LAYER};
    let src = graph.layer_of(region.anchor);
    let is_whole = mapper::layer::is_whole_layer(graph, region);
    let mut out = Vec::new();
    if !is_whole {
        out.push(MoveTarget::New);
    }
    if !(src == MAIN_LAYER && is_whole) {
        out.extend(
            graph.layers().keys().copied().filter(|&t| t != src).map(MoveTarget::Existing),
        );
    }
    out
}

/// `move-region`: compute the region, then re-home it (SQ-0439). Those are two steps because they
/// are two concerns — which rooms, and onto what — and each refuses for its own reasons.
///
/// Either step can turn out to be a genuine choice rather than a refusal, and both then open the
/// same prompt: several passages lead in, or several layers could take the rooms. The command does
/// not guess between them and does not make the player retype it with one more word.
fn apply_move_region(state: &mut AppState, mapper: &mut Mapper, arg: &str) {
    let Some(room) = state.selected_room.or_else(|| mapper.graph.current()) else {
        state.notifications.push("move-region: no room selected");
        return;
    };
    let (dest, dir) = match parse_move_region_arg(&mapper.graph, arg) {
        Ok(v) => v,
        Err(msg) => {
            state.notifications.push(msg);
            return;
        }
    };

    let (region, seam) = match choose_region(&mapper.graph, room, dir) {
        Ok(v) => v,
        // A refusal used to be silent, which reads as a broken command — say which of the several
        // quite different reasons it was (SQ-0360).
        Err(SeamRefusal::Region(why, tried)) => {
            state.notifications.push(region_refusal_message(&mapper.graph, room, tried, why));
            return;
        }
        Err(SeamRefusal::Ambiguous(seams)) => {
            open_seam_prompt(state, &mapper.graph, room, dest, &seams);
            return;
        }
    };

    // A seam the player NAMED needs no reporting back; one chosen for them does.
    let cut = if dir.is_some() { None } else { seam };
    move_region_to(state, mapper, region, cut, dest);
}

/// Resolve the destination and make the move — or, when the destination is a real choice rather
/// than a lone possibility, ask (SQ-0439).
///
/// `cut` is the passage that was chosen for the player, reported back once the move goes through;
/// `None` when they picked it themselves or when there was nothing to cut.
fn move_region_to(
    state: &mut AppState,
    mapper: &mut Mapper,
    region: mapper::layer::Region,
    cut: Option<(mapper::graph::RoomId, Direction)>,
    dest: MoveDest,
) {
    use mapper::layer::MoveTarget;
    let src = mapper.graph.layer_of(region.anchor);
    let target = match dest {
        MoveDest::New => MoveTarget::New,
        MoveDest::Parent => MoveTarget::Existing(mapper::layer::parent_layer(&mapper.graph, src)),
        MoveDest::Layer(id) => MoveTarget::Existing(id),
        // Same rule as the seam, one step later: pick when there is nothing to pick between.
        MoveDest::Auto => match move_targets(&mapper.graph, &region).as_slice() {
            [one] => *one,
            [] => {
                state.notifications.push(
                    "move-region: nowhere to put these rooms — they are the whole of their layer, \
                     so a new one would only rename it, and there is no other layer to fold them \
                     into.",
                );
                return;
            }
            many => {
                let many = many.to_vec();
                open_dest_prompt(state, &mapper.graph, region, cut, &many);
                return;
            }
        },
    };
    perform_move(state, mapper, &region, cut, target);
}

/// The move itself, once both halves are settled: re-home the rooms, say what happened, and follow
/// them to where they landed (SQ-0439).
fn perform_move(
    state: &mut AppState,
    mapper: &mut Mapper,
    region: &mapper::layer::Region,
    cut: Option<(mapper::graph::RoomId, Direction)>,
    target: mapper::layer::MoveTarget,
) -> Option<mapper::layer::LayerId> {
    use mapper::layer::MoveTarget;
    let moved = region.rooms.len();
    match mapper::layer::move_region(&mut mapper.graph, region, target) {
        Ok(landed) => {
            state.bump_graph_gen(); // rooms changed layer → invalidate the render memo (SQ-0305)
            // A seam the player did not name was chosen FOR them, so say which passage was cut —
            // otherwise a bare move silently picks a boundary and the map simply changes shape.
            if let Some((from, d)) = cut {
                let name = room_label(&mapper.graph, from);
                state.set_status(format!("move-region: cut the {d:?} passage from {name}"));
            }
            if !matches!(target, MoveTarget::New) {
                let s = if moved == 1 { "" } else { "s" };
                state.set_status(format!(
                    "{moved} room{s} moved into {}",
                    mapper.graph.layer_name(landed)
                ));
            }
            // Follow the rooms to where they landed. Clearing the view instead sent it to whatever
            // layer the PLAYER happens to stand in — usually the top one — so a merge looked like
            // it had dumped the rooms there, when they had gone to the parent all along and were
            // simply off-screen (SQ-0361).
            state.set_viewed_layer(Some(landed));
            recenter_for_active_layer(state, &mapper.graph);
            Some(landed)
        }
        Err(why) => {
            state.notifications.push(move_refusal_message(&mapper.graph, region.anchor, why));
            None
        }
    }
}

/// A room's name, or `#id` when the map has forgotten it.
fn room_label(graph: &mapper::graph::MapGraph, id: mapper::graph::RoomId) -> String {
    graph.room(id).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{id}"))
}

/// Every room a region holds, named, in region order — the bulleted list under the question.
///
/// No eliding here (SQ-0858). This once returned one comma-joined line cut off after four names,
/// which the modal then drew at whatever width the BODY happened to need, so real room names ran
/// off the edge. Handing over the whole list lets the renderer decide how many rows it can spare
/// and say honestly how many it left out.
fn region_rooms_lines(
    graph: &mapper::graph::MapGraph,
    region: &mapper::layer::Region,
) -> Vec<String> {
    region.rooms.iter().map(|&id| room_label(graph, id)).collect()
}

/// Turn a destination list into prompt options, in the order it was ranked.
fn dest_options(
    graph: &mapper::graph::MapGraph,
    targets: &[mapper::layer::MoveTarget],
) -> Vec<crate::state::RegionOption> {
    use mapper::layer::MoveTarget;
    targets
        .iter()
        .map(|&target| crate::state::RegionOption::Dest {
            label: match target {
                MoveTarget::New => "a new layer".to_string(),
                MoveTarget::Existing(id) => graph.layer_name(id).to_string(),
            },
            target,
        })
        .collect()
}

/// Open a prompt, focused on its first option so the ring starts where the answer is.
fn open_region_prompt(state: &mut AppState, prompt: crate::state::RegionPrompt) {
    state.overlays.dialog_focus = 0;
    state.overlays.region_prompt = Some(prompt);
}

/// Pick up whatever the map had to say about the move just made, and put it in front of the player
/// (SQ-0439).
///
/// The detector already ran inside `apply_turn`, so this only TAKES — it never polls. Call it once
/// a turn is finished, never in the middle of one: a modal already up is a modal the player asked
/// for, and a suggestion nobody asked for must not shoulder in front of it. Dropping one costs
/// nothing, because nothing is written down until the player answers, so the same crossing raises
/// it again.
pub fn offer_layer_suggestion(state: &mut AppState, mapper: &mut Mapper) {
    if state.any_modal_overlay_open() {
        return;
    }
    // A player who has turned the map off has said what they think of the map,
    // and a modal about how to LAY IT OUT is the least welcome thing we could
    // put in front of them (SQ-1137). The prompt's whole subject — which layer
    // these rooms belong on — is invisible from here, so answering it is a
    // guess and declining it is an interruption. The map keeps mapping; it just
    // stops asking. Guarded BEFORE `take_suggestion` for the same reason as the
    // modal above: what is not taken is not consumed.
    if state.layout != crate::state::Layout::Split {
        return;
    }
    if let Some(s) = mapper.take_suggestion() {
        open_layer_suggestion(state, &mapper.graph, s);
    }
}

/// The map noticed that a set of rooms wants a layer, so put the choice in front of the player
/// (SQ-0439). Never acts: this is the whole of what "detect and suggest" means.
pub fn open_layer_suggestion(
    state: &mut AppState,
    graph: &mapper::graph::MapGraph,
    suggestion: mapper::suggest::LayerSuggestion,
) {
    use crate::state::{RegionPrompt, RegionPromptKind};
    use mapper::suggest::Trigger;
    let from = room_label(graph, suggestion.seam.from);
    // Spelled out, and upper case so the passage stands apart from the Title Case room name beside
    // it. NEVER `short_label` (SQ-0858): that is `SeamKey`'s persisted ordering key, and printing
    // it produced "You came d out of Living Room."
    let d = mapper::direction::long_label(suggestion.seam.dir).to_uppercase();
    let n = suggestion.region.rooms.len();
    // One trigger, two shapes, and the seam says which (SQ-0858). A structural suggestion is
    // reported the same way whether it was noticed on the way OUT of a region or from inside one
    // there is no way out of, so asking the trigger cannot tell them apart — but the seam's outside
    // end can: it is one of the rooms that would move only when it is the way out. Reading it from
    // the data rather than growing a second label means the sentence cannot drift away from
    // whichever trigger fired, nor need editing when a third moment is found to notice one at.
    let leaving = suggestion.region.rooms.contains(&suggestion.seam.from);
    let (title, body) = match suggestion.trigger {
        Trigger::Structural => (
            "Give these rooms their own layer?".to_string(),
            vec![
                if leaving {
                    // The way OUT, walked just now: `from` is the room inside you have just left.
                    format!("You came {d} out of {from}.")
                } else {
                    // The way IN, and you are still down there: `from` is the room above.
                    format!("You came {d} from {from}.")
                },
                // What `planar_region` actually guarantees, and all it guarantees: no compass edge
                // crosses the boundary. The old line claimed "no other way in", which a cellar with
                // a second trapdoor makes false in either reading.
                format!("No compass passage reaches those {n} rooms."),
            ],
        ),
        // The seam is the way IN, and the region is the maze side of it.
        Trigger::Name => (
            "This looks like a maze.".to_string(),
            vec![
                format!("{} calls itself a maze.", room_label(graph, suggestion.region.anchor)),
                "Separating it also flags the layer as a maze.".to_string(),
            ],
        ),
    };
    let options = dest_options(graph, &suggestion.destinations);
    if options.is_empty() {
        return; // `mapper::suggest` never builds one of these, but an empty list has no question
    }
    open_region_prompt(state, RegionPrompt {
        kind: RegionPromptKind::Suggest {
            trigger: suggestion.trigger,
            seam: suggestion.seam,
            region: suggestion.region.clone(),
        },
        title,
        body,
        rooms: region_rooms_lines(graph, &suggestion.region),
        options,
        choice: 0,
    });
}

/// Tier 3: several passages lead into the selected room and each cuts a different map (SQ-0439).
///
/// This is the case a re-issued command cannot always fix. A maze happily has two rooms whose
/// SOUTH exits both land here — Adventure's does — so `move-region new s` would ask the very same
/// question again, and until this prompt existed the command simply gave up. Picking from the list
/// is the only answer that always exists.
fn open_seam_prompt(
    state: &mut AppState,
    graph: &mapper::graph::MapGraph,
    room: mapper::graph::RoomId,
    dest: MoveDest,
    seams: &[mapper::layer::InboundSeam],
) {
    use crate::state::{RegionOption, RegionPrompt, RegionPromptKind};
    let options: Vec<RegionOption> = seams
        .iter()
        .map(|s| RegionOption::Seam {
            label: format!(
                "{} from {} ({} rooms)",
                mapper::direction::short_label(s.dir),
                room_label(graph, s.from),
                s.region.rooms.len()
            ),
            from: s.from,
            dir: s.dir,
        })
        .collect();
    if options.is_empty() {
        return;
    }
    open_region_prompt(state, RegionPrompt {
        kind: RegionPromptKind::PickSeam { room, dest },
        title: "Which passage should be cut?".to_string(),
        body: vec![
            format!("Several passages lead into {}.", room_label(graph, room)),
            "Each one takes a different set of rooms.".to_string(),
        ],
        // Deliberately blank: the rooms are not settled until a passage is, and each option
        // carries its own count.
        rooms: Vec::new(),
        options,
        choice: 0,
    });
}

/// The destination half of the same question: the rooms are settled and more than one layer could
/// take them (SQ-0439).
fn open_dest_prompt(
    state: &mut AppState,
    graph: &mapper::graph::MapGraph,
    region: mapper::layer::Region,
    cut: Option<(mapper::graph::RoomId, Direction)>,
    targets: &[mapper::layer::MoveTarget],
) {
    use crate::state::{RegionPrompt, RegionPromptKind};
    let options = dest_options(graph, targets);
    if options.is_empty() {
        return;
    }
    let rooms = region_rooms_lines(graph, &region);
    open_region_prompt(state, RegionPrompt {
        kind: RegionPromptKind::PickDest { region, cut },
        title: "Where do these rooms go?".to_string(),
        body: vec!["More than one layer could take them.".to_string()],
        rooms,
        options,
        choice: 0,
    });
}

/// Apply what the player told the region prompt to do, and close it (SQ-0439).
///
/// The three suggestion outcomes are a gradient over one mechanism — separate now / ask again next
/// crossing / never ask about this passage — so only the last two write anything down, and
/// accepting writes nothing at all: the move puts the seam across two layers, which silences it by
/// construction.
pub fn apply_region_prompt(state: &mut AppState, mapper: &mut Mapper, act: crate::state::RegionPromptAct) {
    use crate::state::{RegionOption, RegionPromptAct as A, RegionPromptKind as K};
    use mapper::suggest::{SeamDecision, Trigger};
    let Some(prompt) = state.overlays.region_prompt.take() else { return };
    let chosen = prompt.chosen().cloned();
    match (&prompt.kind, act) {
        (_, A::Dismiss) => {}
        (K::Suggest { seam, .. }, A::Defer) => {
            mapper.graph.set_seam_decision(*seam, SeamDecision::Deferred);
        }
        (K::Suggest { seam, .. }, A::Never) => {
            mapper.graph.set_seam_decision(*seam, SeamDecision::Ignored);
        }
        // A pick has nothing to remember: declining to choose decided nothing.
        (_, A::Defer | A::Never) => {}
        (K::Suggest { trigger, region, .. }, A::Accept) => {
            let Some(RegionOption::Dest { target, .. }) = chosen else { return };
            if let Some(landed) = perform_move(state, mapper, region, None, target) {
                // The player confirmed it is a maze by accepting a prompt that said so. A
                // structural suggestion sets nothing — a cellar is not a maze.
                if *trigger == Trigger::Name {
                    mapper.graph.set_layer_maze(landed, true);
                }
            }
        }
        (K::PickSeam { room, dest }, A::Accept) => {
            let Some(RegionOption::Seam { from, dir, .. }) = chosen else { return };
            // Recomputed with the same walker `inbound_seams` used, so the region cannot disagree
            // with the one the option was labelled from.
            match mapper::layer::region_at_arrival(&mapper.graph, from, dir) {
                // The player picked the passage, so there is nothing to report back.
                Ok(region) => move_region_to(state, mapper, region, None, *dest),
                Err(why) => state.notifications.push(region_refusal_message(
                    &mapper.graph,
                    *room,
                    Some((from, dir)),
                    why,
                )),
            }
        }
        (K::PickDest { region, cut }, A::Accept) => {
            let Some(RegionOption::Dest { target, .. }) = chosen else { return };
            perform_move(state, mapper, region, *cut, target);
        }
    }
}

/// Say which of the several quite different reasons the REGION could not be computed (SQ-0360).
///
/// The refusals are not variations on "no": a layer with no seam in it needs a direction naming
/// one, while a passage that is not a seam means the boundary the player picked is not real. Each
/// message therefore names the room or passage at issue and, where there is a way forward, points
/// at it.
///
/// `seam` is the passage that was actually tried — `(the room it leads out of, its direction)` —
/// which is anchored at the room the passage leads FROM, not at the selected room, because an
/// inbound seam's whole point is that it may have no counterpart out of the selection (SQ-0439).
fn region_refusal_message(
    graph: &mapper::graph::MapGraph,
    room: mapper::graph::RoomId,
    seam: Option<(mapper::graph::RoomId, Direction)>,
    why: mapper::layer::RegionRefusal,
) -> String {
    use mapper::layer::RegionRefusal as R;
    let name = |id: mapper::graph::RoomId| {
        graph.room(id).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{id}"))
    };
    let here = name(room);
    let layer = graph.layer_name(graph.layer_of(room));
    match (why, seam) {
        (R::NoSuchPassage, Some((from, d))) => {
            format!("move-region: {} has no {d:?} passage.", name(from))
        }
        (R::NoSuchPassage, None) => format!("move-region: {here} has no passage to cut."),
        (R::NotASeam, Some((from, d))) => format!(
            "move-region: the {d:?} passage from {} is not a boundary — both sides stay connected \
             another way.",
            name(from)
        ),
        (R::NotASeam, None) => format!(
            "move-region: {here} has no passage that is a boundary — every way out stays connected \
             another way."
        ),
        (R::LeavesLayer, Some((from, d))) => format!(
            "move-region: the {d:?} passage from {} already leaves {layer}. A seam divides one \
             layer.",
            name(from)
        ),
        (R::LeavesLayer, None) => {
            format!("move-region: {here}'s passage already leaves {layer}. A seam divides one layer.")
        }
    }
}

/// Say why the MOVE refused, once the region itself was fine (SQ-0439). Nothing here is about
/// which rooms were chosen — only about where they were headed.
fn move_refusal_message(
    graph: &mapper::graph::MapGraph,
    room: mapper::graph::RoomId,
    why: mapper::layer::MoveRefusal,
) -> String {
    use mapper::layer::MoveRefusal as R;
    let layer = graph.layer_name(graph.layer_of(room));
    match why {
        R::WholeLayer => format!(
            "move-region: {layer} is one connected region — nothing to separate. \
             Use move-region <destination> <direction> to cut at a passage, or name a layer to \
             move it all into."
        ),
        R::EmptiesMain => format!(
            "move-region: that would leave {layer} with no rooms at all. Main is the layer \
             everything else folds into."
        ),
        R::SelfMove => format!("move-region: those rooms are already on {layer}."),
        R::NoSuchLayer => "move-region: that layer no longer exists.".to_string(),
    }
}

fn apply_recenter(state: &mut AppState, mapper: &Mapper) {
    let target = recenter_target(state, &mapper.graph);
    let (pw, ph) = state.map_pane_size.get().unwrap_or((80, 24));
    state.recenter_on(target, pw, ph);
}

/// The cell the map view should sit on: the selected room, else the room the player is in, else the
/// origin (SQ-0349).
///
/// The run loop's own auto-recentres (after a turn, after a tidy) deliberately aim at the current
/// room instead: the map follows the player as you play, and a selection lasts until your next
/// command. That is intended, not an oversight.
fn recenter_target(state: &AppState, graph: &mapper::graph::MapGraph) -> (i32, i32) {
    let pos_of = |id| graph.room(id).and_then(|r| r.pos);
    state
        .selected_room
        .and_then(pos_of)
        .or_else(|| graph.current().and_then(pos_of))
        .unwrap_or((0, 0))
}

/// Recentre the map — or, on a matrix layer, select and scroll to show — the room a layer switch
/// should land on, so switching layers never leaves the viewport sitting on empty scroll space
/// (SQ-0672). Every place `active_layer(graph)` can change value for a reason other than "the
/// player walked" — cycling, a tab click, a peel, a merge, loading a whole new map — calls this
/// right after making the switch.
///
/// The room aimed at, in order:
/// 1. The room the player is currently standing in, if it is on the newly active layer — the same
///    follow-the-player centering the per-turn recenter already uses.
/// 2. Else the last room visited on that layer ([`mapper::graph::MapGraph::last_visited`],
///    recorded every time the current room changes). A dangling id — the room is gone, or has
///    since been peeled/merged to another layer — is treated exactly like "never visited".
/// 3. Else the bounding-box centre of the layer's own rooms (never visited at all, or the one
///    candidate above was dangling).
///
/// A drawn layer just scrolls its viewport to that point. A matrix layer has no scroll grid of
/// its own to centre — rule 1/2's room becomes the SELECTED row and the table scrolls to show it
/// ([`crate::render::matrix::scroll_to_show`]); rule 3's fallback has no room of its own to select,
/// so the room nearest the bounding-box centre stands in for it.
pub fn recenter_for_active_layer(state: &mut AppState, graph: &mapper::graph::MapGraph) {
    let layer = state.active_layer(graph);
    let rooms = graph.rooms_in_layer(layer);

    // Rules 1 and 2: a specific room the switch should land on.
    let focus = graph
        .current()
        .filter(|&id| graph.layer_of(id) == layer)
        .or_else(|| {
            graph
                .last_visited(layer)
                .filter(|&id| graph.room(id).is_some() && graph.layer_of(id) == layer)
        });

    // Rule 3: the layer's own bounding-box centre, when neither rule above named a room.
    let target = focus
        .and_then(|id| graph.room(id).and_then(|r| r.pos))
        .unwrap_or_else(|| layer_bbox_center(graph, &rooms));

    // The matrix still needs SOME row to select even when rule 3 fired with no room of its own —
    // the room nearest the target point stands in.
    let select = focus.or_else(|| nearest_room(graph, &rooms, target));
    if let Some(id) = select {
        state.select_room(Some(id));
    }

    if graph.layer_view(layer) == mapper::layer::MapView::Matrix {
        if let (Some(id), Some((w, h))) = (select, state.map_pane_size.get()) {
            let area = ratatui::layout::Rect::new(0, 0, w, h);
            state.matrix_scroll.1 =
                crate::render::matrix::scroll_to_show(graph, layer, id, area, state.matrix_scroll.1);
        }
    } else {
        let (pw, ph) = state.map_pane_size.get().unwrap_or((80, 24));
        state.recenter_on(target, pw, ph);
    }
}

/// The centre of the smallest box containing every POSITIONED room in `rooms`, or the origin when
/// none of them have a position yet (a brand-new, still-empty layer).
fn layer_bbox_center(
    graph: &mapper::graph::MapGraph,
    rooms: &[mapper::graph::RoomId],
) -> (i32, i32) {
    let mut positions = rooms.iter().filter_map(|&id| graph.room(id).and_then(|r| r.pos));
    let Some(first) = positions.next() else { return (0, 0) };
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first.0, first.0, first.1, first.1);
    for (x, y) in positions {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    ((min_x + max_x) / 2, (min_y + max_y) / 2)
}

/// The positioned room in `rooms` closest to `target`, by squared distance — the matrix's stand-in
/// for a bounding-box centre, which names a POINT rather than a room.
fn nearest_room(
    graph: &mapper::graph::MapGraph,
    rooms: &[mapper::graph::RoomId],
    target: (i32, i32),
) -> Option<mapper::graph::RoomId> {
    rooms
        .iter()
        .filter_map(|&id| graph.room(id).and_then(|r| r.pos).map(|p| (id, p)))
        .min_by_key(|&(_, (x, y))| {
            let (dx, dy) = (x - target.0, y - target.1);
            dx * dx + dy * dy
        })
        .map(|(id, _)| id)
}

// ── Config screen helpers ─────────────────────────────────────────────────────

/// Number of rows in the config screen — derived from the row list so it cannot drift.
pub(crate) const CONFIG_ROW_COUNT: usize = crate::render::config_screen::CONFIG_ROWS.len();

/// Clone a Config (Config derives Clone, this is a convenience wrapper for tests).
pub(crate) fn clone_config(cfg: &crate::config::Config) -> crate::config::Config {
    cfg.clone()
}

/// Return the ConfigPathField for a row, if the row is a path type.
fn config_path_field(row: usize) -> Option<crate::state::ConfigPathField> {
    match row {
        0 => Some(crate::state::ConfigPathField::UserDir),
        _ => None,
    }
}

/// Apply ConfigToggle to the selected row: toggle bool, advance enum by 1, or open path edit.
fn config_toggle_or_edit(selected: usize, state: &mut AppState) {
    if let (Some(key), Some(cs)) =
        (one_run_key_for_row(selected), state.overlays.config_screen.as_mut())
    {
        cs.working.one_run.release(key);
    }
    match selected {
        0 => {
            // user_dir — open the text-entry path edit dialog.
            let current = state.overlays.config_screen.as_ref()
                .map(|cs| cs.working.user_dir.to_string_lossy().to_string())
                .unwrap_or_default();
            state.overlays.dialog_focus = 0;
            state.overlays.text_entry = Some(TextEntryDialog::new(
                TextEntryKind::ConfigEditPath { field: crate::state::ConfigPathField::UserDir },
                current,
            ));
        }
        1 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.auto_load = !cs.working.auto_load; } }
        2 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.auto_save = !cs.working.auto_save; } }
        3 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.prompt_save_on_quit = !cs.working.prompt_save_on_quit; } }
        4 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.prompt_load_on_launch = !cs.working.prompt_load_on_launch; } }
        5 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.show_room_numbers = !cs.working.show_room_numbers; } }
        6 => { if let Some(cs) = &mut state.overlays.config_screen { config_cycle_background_tidy(&mut cs.working.background_tidy, 1); } }
        7 => { if let Some(cs) = &mut state.overlays.config_screen { config_cycle_aux_storage(&mut cs.working.aux_storage, 1); } }
        8 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.honor_game_colours = !cs.working.honor_game_colours; } }
        9 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.period_look = !cs.working.period_look; } }
        10 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.honor_timed_input = !cs.working.honor_timed_input; } }
        11 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.enable_sound = !cs.working.enable_sound; } }
        13 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.mouse = !cs.working.mouse; } }
        14 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.command_bar = !cs.working.command_bar; } }
        15 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.mouse_wheel_invert = !cs.working.mouse_wheel_invert; } }
        16 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.show_status_bar = !cs.working.show_status_bar; } }
        17 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.watch_style = !cs.working.watch_style; } }
        18 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.record_turn_history = !cs.working.record_turn_history; } }
        21 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.hint_skip_screen_warning = !cs.working.hint_skip_screen_warning; } }
        24 => { if let Some(cs) = &mut state.overlays.config_screen { config_cycle_v6_render(&mut cs.working.v6_render, 1); } }
        25 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.v6_arrow_keys = !cs.working.v6_arrow_keys; } }
        26 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.v6_pixel_lock = !cs.working.v6_pixel_lock; } }
        27 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.guidance = !cs.working.guidance; } }
        28 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.guidance_probe = !cs.working.guidance_probe; } }
        29 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.hide_adult_words = !cs.working.hide_adult_words; } }
        30 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.return_probe = !cs.working.return_probe; } }
        _ => {}
    }
}

/// Cycle a BackgroundTidy enum value by delta.
fn config_cycle_background_tidy(val: &mut crate::config::BackgroundTidy, delta: i32) {
    use crate::config::BackgroundTidy::*;
    let variants = [Off, EveryRoom, OnOverlap, Debounced];
    let pos = variants.iter().position(|v| v == val).unwrap_or(0) as i32;
    let n = variants.len() as i32;
    *val = variants[((pos + delta).rem_euclid(n)) as usize];
}

fn config_cycle_aux_storage(val: &mut crate::config::AuxStorage, delta: i32) {
    use crate::config::AuxStorage::*;
    let variants = [Ask, Archive, Global];
    let pos = variants.iter().position(|v| v == val).unwrap_or(0) as i32;
    let n = variants.len() as i32;
    *val = variants[((pos + delta).rem_euclid(n)) as usize];
}

fn config_cycle_v6_render(val: &mut crate::config::V6RenderMode, delta: i32) {
    use crate::config::V6RenderMode::*;
    let variants = [Hybrid, Raster, Extended];
    let pos = variants.iter().position(|v| v == val).unwrap_or(0) as i32;
    let n = variants.len() as i32;
    *val = variants[((pos + delta).rem_euclid(n)) as usize];
}

/// The `config.toml` key a settings-screen row owns, for the rows a one-run source
/// can have pinned (SQ-0807). Editing the row is a deliberate decision, so it ends
/// the one-run hold outright — without that, "toggle away and back" would land on
/// the pinned value again and silently fail to persist.
///
/// Row 0 (`user_dir`) is deliberately absent: it only OPENS a path dialog here, and
/// the user may cancel. Its release lives where the typed path is applied.
fn one_run_key_for_row(row: usize) -> Option<&'static str> {
    use crate::config::keys;
    match row {
        8 => Some(keys::HONOR_GAME_COLOURS),
        11 => Some(keys::ENABLE_SOUND),
        20 => Some(keys::INTERPRETER_NUMBER),
        // SQ-1161: `startup.rs` pins V6_RENDER from this game's sidecar exactly as
        // it pins the pixel lock below, and this row had no arm — so cycling the
        // render mode here released nothing, and `write_config_at` skipped the key
        // as "one-run". The user's global choice was silently not persisted.
        24 => Some(keys::V6_RENDER),
        // SQ-0945: this game's sidecar pins the key at boot; editing the row here
        // is the user speaking about every game, so it ends the hold and the value
        // persists to the global config like any other setting.
        26 => Some(keys::V6_PIXEL_LOCK),
        // SQ-1045: `--guidance off` pins the key for the launch; editing the row
        // is the user overruling their own flag, so it ends the hold and persists.
        27 => Some(keys::GUIDANCE),
        // SQ-0785: this game's sidecar pins the key at boot, exactly as the pixel
        // lock's does; editing the row is the user speaking about every game.
        30 => Some(keys::RETURN_PROBE),
        _ => None,
    }
}

/// Apply ConfigCycle to the selected row.
fn config_cycle(working: &mut crate::config::Config, row: usize, delta: i32) {
    if let Some(key) = one_run_key_for_row(row) {
        working.one_run.release(key);
    }
    match row {
        0 => {} // path: no cycling
        1 => working.auto_load = !working.auto_load,
        2 => working.auto_save = !working.auto_save,
        3 => working.prompt_save_on_quit = !working.prompt_save_on_quit,
        4 => working.prompt_load_on_launch = !working.prompt_load_on_launch,
        5 => working.show_room_numbers = !working.show_room_numbers,
        6 => config_cycle_background_tidy(&mut working.background_tidy, delta),
        7 => config_cycle_aux_storage(&mut working.aux_storage, delta),
        8 => working.honor_game_colours = !working.honor_game_colours,
        9 => working.period_look = !working.period_look,
        10 => working.honor_timed_input = !working.honor_timed_input,
        11 => working.enable_sound = !working.enable_sound,
        12 => working.volume = (working.volume as i32 + delta * 5).clamp(0, 100) as u8,
        13 => working.mouse = !working.mouse,
        14 => working.command_bar = !working.command_bar,
        15 => working.mouse_wheel_invert = !working.mouse_wheel_invert,
        16 => working.show_status_bar = !working.show_status_bar,
        17 => working.watch_style = !working.watch_style,
        18 => working.record_turn_history = !working.record_turn_history,
        19 => working.undo_levels = (working.undo_levels as i32 + delta).clamp(0, 256) as usize,
        // Position 0 == None (lanthorn's default); 1..=10 are explicit interpreter numbers.
        // ← from 1 returns to "default"; → from "default" goes to 1.
        20 => {
            let pos = (working.interpreter_number.map(|n| n as i32).unwrap_or(0) + delta).clamp(0, 10);
            working.set_interpreter_number(if pos == 0 { None } else { Some(pos as u8) });
        }
        21 => working.hint_skip_screen_warning = !working.hint_skip_screen_warning,
        25 => working.v6_arrow_keys = !working.v6_arrow_keys,
        22 => working.text_margin_x = (working.text_margin_x as i32 + delta).clamp(0, 8) as u16,
        23 => working.text_margin_y = (working.text_margin_y as i32 + delta).clamp(0, 8) as u16,
        24 => config_cycle_v6_render(&mut working.v6_render, delta),
        26 => working.v6_pixel_lock = !working.v6_pixel_lock,
        27 => working.guidance = !working.guidance,
        28 => working.guidance_probe = !working.guidance_probe,
        29 => working.hide_adult_words = !working.hide_adult_words,
        30 => working.return_probe = !working.return_probe,
        _ => {}
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use mapper::mapper::Mapper;

    use super::*;
    use crate::state::AppState;

    // ── Test helpers ──────────────────────────────────────────────────────────

    /// Build a plain (no-modifier) Press KeyEvent.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    /// Build a Ctrl+key Press KeyEvent.
    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    /// Build a Shift+key Press KeyEvent.
    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // ── Brief-required tests ──────────────────────────────────────────────────

    #[test]
    fn game_focus_builds_and_submits_command() {
        let mut s = AppState::default(); // Focus::Game
        for c in "north".chars() {
            let a = key_to_action(&s, key(KeyCode::Char(c)));
            assert!(matches!(a, Action::InputChar(_)));
            if let Action::InputChar(ch) = a {
                s.push_input_char(ch);
            }
        }
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(matches!(a, Action::SubmitCommand(ref c) if c == "north"));
    }

    #[test]
    fn game_focus_has_map_shortcuts() {
        let s = AppState::default(); // Game focus (story line)
        // Map navigation works without leaving the story line.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
        // Home is a TEXT key now that the command line has a caret (SQ-0354) — it jumps to the
        // start of the line, matching every other text entry in the app. Recenter keeps its
        // `center-map` command and its hotkey; it just gives up the bare Home key.
        assert!(matches!(key_to_action(&s, key(KeyCode::Home)), Action::CursorHome));
        assert!(matches!(key_to_action(&s, key(KeyCode::End)), Action::CursorEnd));
        // Shift+Arrows still pan, so map nav survives without leaving the story line.
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::CursorLeft));
        // PageUp/PageDown now page the transcript (older/newer), not zoom.
        assert!(matches!(key_to_action(&s, key(KeyCode::PageUp)), Action::TranscriptScrollPage(1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::PageDown)), Action::TranscriptScrollPage(-1)));
        // Retidy (Ctrl+T) is not in the direct set: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::None));
        // Typing still reaches the command line (plain and shifted/capital letters).
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::InputChar('n')));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::InputChar('N')));
    }

    #[test]
    fn active_more_pager_intercepts_keys() {
        // SQ-0404: while the [more] pager is showing, Space/PgDn/Down/Enter page
        // one screen; anything else dismisses to the bottom. Keys never reach the
        // game.
        let mut s = AppState::default();
        s.pager.active = true;
        for code in [KeyCode::Char(' '), KeyCode::PageDown, KeyCode::Down, KeyCode::Enter] {
            assert!(
                matches!(key_to_command(&s, key(code)), KeyResolve::Action(Action::PagerAdvance)),
                "{code:?} should advance the pager"
            );
        }
        for code in [KeyCode::Char('x'), KeyCode::Esc, KeyCode::Char('q')] {
            assert!(
                matches!(key_to_command(&s, key(code)), KeyResolve::Action(Action::PagerDismiss)),
                "{code:?} should dismiss the pager"
            );
        }
        // Inactive: the pager does not intercept — a plain letter reaches input.
        s.pager.active = false;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('x'))), Action::InputChar('x')));
    }

    #[test]
    fn more_pager_pages_on_any_key_while_a_read_char_is_pending() {
        // SQ-0539, per the directive "[more] should work any time output is larger
        // than what fits on the screen … we should behave as the original game
        // intended": with a `read_char` pending, jumping to the bottom would skip
        // text the player never saw AND then hand that key to the game. So while
        // char_mode is live EVERY key advances one screen; nothing dismisses, and
        // nothing reaches the game until the pager has caught up (main.rs's
        // char-input gate is suppressed for the whole time `active` is set).
        let mut s = AppState::default();
        s.pager.active = true;
        s.char_mode = true;
        for code in [
            KeyCode::Char(' '),
            KeyCode::PageDown,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Char('x'),
            KeyCode::Char('y'),
            KeyCode::Esc,
            KeyCode::Char('q'),
        ] {
            assert!(
                matches!(key_to_command(&s, key(code)), KeyResolve::Action(Action::PagerAdvance)),
                "{code:?} must PAGE (never dismiss/deliver) while a read_char is pending"
            );
        }
        // Line input keeps its existing feel: a non-paging key still dismisses.
        s.char_mode = false;
        assert!(matches!(
            key_to_command(&s, key(KeyCode::Char('x'))),
            KeyResolve::Action(Action::PagerDismiss)
        ));
    }

    #[test]
    fn map_letter_keys_are_ordinary_typing() {
        // SQ-0599: h/j/-/c/n/p used to drive the map while it held focus. The
        // map cannot hold focus any more, so they are just characters — typing
        // "hunt" must not pan the map twice on the way.
        let s = AppState::default();
        for c in ['h', 'j', 'k', 'l', 'c', 'n', 'p', '+', '-', '0'] {
            assert!(
                matches!(key_to_action(&s, key(KeyCode::Char(c))), Action::InputChar(x) if x == c),
                "'{c}' must reach the command line, not the map"
            );
        }
    }

    #[test]
    fn ctrl_q_quits_in_any_focus() {
        let s = AppState::default();
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('q'))), Action::Quit));
    }

    // The former "prompt-precedence" tests (Ctrl+Q/Tab/Ctrl+S during a bottom-bar
    // prompt) covered the retired `key_to_command` prompt sub-mode. The text-entry
    // modal is now driven by a run-loop intercept (like the save-name dialog), so
    // key_to_command no longer special-cases it; that intercept is exercised via
    // the `text_entry_dialog_key` unit tests in `render::text_entry_dialog`.

    // ── Additional tests ──────────────────────────────────────────────────────

    #[test]
    fn arrows_always_mean_the_same_thing() {
        // The heart of SQ-0599: an arrow key had two meanings depending on a
        // focus state with no on-screen cue. Now plain arrows are always the
        // command line's, and Shift+Arrow is always the map's.
        let s = AppState::default();
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::CursorLeft));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::CursorRight));
        assert!(matches!(key_to_action(&s, key(KeyCode::Up)), Action::HistoryPrev));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::HistoryNext));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
    }

    #[test]
    fn shift_arrows_pan_from_the_story() {
        let s = AppState::default();
        // Shift+Arrows pan without leaving the command line (SQ-0416).
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
        // F6-F9 were the room-nudge keys until SQ-0600 removed nudging; they are
        // unbound now, not silently doing nothing.
        for f in [6u8, 7, 8, 9] {
            assert!(matches!(key_to_action(&s, key(KeyCode::F(f))), Action::None), "F{f} is unbound");
        }
        // Ctrl+Arrows are unbound too.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Right)), Action::None));
    }

    #[test]
    fn n_starts_edit_notes_in_map_focus() {
        // SQ-0446: 'n' is the Edit-group leader letter for edit-notes now
        // (rename-layer moved to the '/' palette). With the dialog closed, plain
        // 'n' is the direct select-room-next; Shift+N is not a leader letter.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::None));
        // Open dialog: Shift+N is not an authored leader letter, so it closes the
        // dialog; the authored leader letter 'n' fires EditNotes.
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::EditNotes));
    }

    #[test]
    fn global_shortcuts_work_with_the_dialog_closed() {
        let mut s = AppState::default();
        // Direct commands fire without the dialog.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('q'))), Action::Quit));
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-state"));
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('r'))), KeyResolve::Command(c, _) if c == "restore-state"));
        // Non-direct commands return None when dialog is closed. (Ctrl+E is the
        // readline move-to-end at the story prompt, so it is not one of them.
        // Ctrl+D is not one of them either since SQ-1228: it half-pages the
        // transcript in Game focus, hardwired ahead of this dialog lookup.)
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('g'))), Action::None));
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('d'))),
            Action::TranscriptScrollHalfPage(-1)
        ));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('l'))), Action::None));
        // Ctrl-combos always close the dialog now (never fire); the underlying
        // commands fire via their authored leader letters instead (SQ-0446 layout).
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('v'))), Action::OpenCommandBand));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('l'))), Action::TogglePortalLabels));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInventory));
        // 'u' fires the map view-mode cycle (SQ-0666, inheriting SQ-0391's freed letter);
        // 'x' (reset-game's old letter) is still unbound.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('u'))), Action::ViewMap(None)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('x'))), Action::CloseHotkeyDialog));
    }

    #[test]
    fn leader_letter_fires_command() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        // 'r' → rename-room. This used 't' → tidy-map until the Layout group was
        // removed from the panel; the mechanism under test is unchanged.
        match key_to_command(&s, key(KeyCode::Char('r'))) {
            KeyResolve::Command(c, _) => assert_eq!(c, "rename-room"),
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn leader_multiword_letter_fires_full_command() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        match key_to_command(&s, key(KeyCode::Char('c'))) {
            KeyResolve::Command(c, _) => assert_eq!(c, "cycle-layer next"),
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn leader_unbound_letter_closes() {
        // SQ-0446 curated the leader to 15 letters, so most letters are unbound
        // and close the dialog. 'q' is deliberately unbound (quit convention).
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_command(&s, key(KeyCode::Char('q'))), KeyResolve::Action(Action::CloseHotkeyDialog)));
        assert!(matches!(key_to_command(&s, key(KeyCode::Char('1'))), KeyResolve::Action(Action::CloseHotkeyDialog)));
    }

    #[test]
    fn leader_ctrl_combo_closes_not_fires() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Action(Action::CloseHotkeyDialog)));
    }

    #[test]
    fn leader_esc_closes() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_command(&s, key(KeyCode::Esc)), KeyResolve::Action(Action::CloseHotkeyDialog)));
    }

    #[test]
    fn key_resolves_to_command_string() {
        // Ctrl+S resolves to a command string (not an Action).
        let s = AppState::default();
        match key_to_command(&s, ctrl(KeyCode::Char('s'))) {
            KeyResolve::Command(c, ctx) => {
                assert_eq!(c, "save-state");
                assert_eq!(ctx, crate::keymap::Context::Global);
            }
            other => panic!("expected Command, got {other:?}"),
        }
        // Hardwired Ctrl+Q stays an Action.
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('q'))), KeyResolve::Action(Action::Quit)));
    }

    #[test]
    fn tab_toggles_focus() {
        let s = AppState::default();
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
    }

    #[test]
    fn history_keys_step_resume_and_close() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let plain = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert!(matches!(history_key_to_action(plain(KeyCode::Left)), Action::ReplayStep(-1)));
        assert!(matches!(history_key_to_action(plain(KeyCode::Right)), Action::ReplayStep(1)));
        assert!(matches!(history_key_to_action(plain(KeyCode::Char(' '))), Action::ReplayTogglePlay));
        assert!(matches!(history_key_to_action(plain(KeyCode::Enter)), Action::ReplayResume));
        assert!(matches!(history_key_to_action(plain(KeyCode::Char('r'))), Action::ReplayResume));
        assert!(matches!(history_key_to_action(plain(KeyCode::Esc)), Action::ReplayClose));
        assert!(matches!(history_key_to_action(plain(KeyCode::Char('q'))), Action::ReplayClose));
    }

    #[test]
    fn replay_step_moves_idx_and_close_clears() {
        use crate::state::{AppState, ReplayState};
        use mapper::mapper::Mapper;
        let mut s = AppState::default();
        // Three records so idx 0..=2 are valid.
        let m = Mapper::default();
        for t in 1..=3 {
            crate::history::record_turn(&mut s.history, t, "x", vec![t as u8], &m, false, "");
        }
        s.overlays.replay = Some(ReplayState::new(2));
        apply_action(Action::ReplayStep(-1), &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.replay.as_ref().unwrap().idx, 1);
        apply_action(Action::ReplayClose, &mut s, &mut Mapper::default());
        assert!(s.overlays.replay.is_none(), "Esc closes without change");
        assert_eq!(s.history.len(), 3, "close leaves history intact");
    }

    /// SQ-0692: the old single-rung "Esc closes the room panel" became a LADDER,
    /// because the dock has two states worth leaving. Esc unpins first (the dock
    /// stays up, following the player again) and closes it on the next press; with
    /// the dock down it is not Esc's business at all.
    #[test]
    fn esc_unpins_the_room_dock_then_closes_it() {
        let mut s = AppState::default();
        s.room_dock.toggle_to(true, true);
        s.selected_room = Some(1);
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::UnpinRoomDock),
            "Esc with a pinned dock unpins first");
        // q is not a close key (and never was, since the panels stopped being modals).
        assert!(!matches!(key_to_action(&s, key(KeyCode::Char('q'))),
            Action::UnpinRoomDock | Action::CloseRoomDock),
            "q must not touch the dock");

        s.selected_room = None;
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::CloseRoomDock),
            "Esc with an unpinned dock closes it");

        s.room_dock.toggle_to(false, true);
        assert!(!matches!(key_to_action(&s, key(KeyCode::Esc)),
            Action::UnpinRoomDock | Action::CloseRoomDock),
            "Esc with no dock open must not produce a dock action");
    }

    /// The ladder's rungs actually do what they say when applied.
    #[test]
    fn the_esc_ladder_leaves_the_dock_up_until_the_second_press() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.open_room_dock(crate::state::RoomDockView::Info);
        s.selected_room = Some(7);

        apply_action(Action::UnpinRoomDock, &mut s, &mut m);
        assert_eq!(s.selected_room, None, "first Esc unpins");
        assert!(s.room_dock.open, "…and leaves the dock up");

        apply_action(Action::CloseRoomDock, &mut s, &mut m);
        assert!(!s.room_dock.open, "the second Esc closes it");
    }

    #[test]
    fn apply_action_pan_accumulates() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::Pan(2, -1), &mut s, &mut m);
        apply_action(Action::Pan(-1, 3), &mut s, &mut m);
        assert_eq!(s.scroll, (1, 2));
    }

    #[test]
    fn apply_action_toggle_focus() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // SQ-0599: with no inspector open there is nowhere for Tab to go, so
        // ToggleFocus is inert and the story keeps the keyboard.
        apply_action(Action::ToggleFocus, &mut s, &mut m);
        assert_eq!(s.focus, crate::state::Focus::Game);
        apply_action(Action::ToggleFocus, &mut s, &mut m);
        assert_eq!(s.focus, crate::state::Focus::Game);
    }

    #[test]
    fn apply_action_select_cycles_rooms() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(mapper::direction::Direction::N));
        m.observe(3, "C", Some(mapper::direction::Direction::E));

        // No selection yet: SelectNext picks first (id=1).
        apply_action(Action::SelectNext, &mut s, &mut m);
        assert_eq!(s.selected_room, Some(1));

        apply_action(Action::SelectNext, &mut s, &mut m);
        assert_eq!(s.selected_room, Some(2));

        apply_action(Action::SelectPrev, &mut s, &mut m);
        assert_eq!(s.selected_room, Some(1));
    }

    #[test]
    fn rename_fires_only_through_the_leader_dialog() {
        let mut s = AppState::default();
        // With the dialog closed these are ordinary typing (SQ-0599).
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('t'))), Action::InputChar('t')));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::InputChar('r')));
        // With it open, 'r' fires rename-room through its authored leader letter;
        // Shift+R is no longer an authored letter. 't' used to be tidy-map here and
        // is now unauthored, so it closes the dialog instead — see
        // `retidy_has_no_default_key_and_t_is_free`.
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::RenameRoom));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('t'))), Action::CloseHotkeyDialog));
    }

    #[test]
    fn retidy_rederives_clean_layout_in_auto() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(mapper::direction::Direction::E)); // hint: 2 east of 1
        // Scramble so 2 sits WEST of 1, contradicting the hint (mimics greedy drift).
        m.graph.set_pos(1, (5, 5));
        m.graph.set_pos(2, (0, 0));
        apply_action(Action::Retidy, &mut s, &mut m);
        // Retidy now builds off-thread (for the progress bar); drive the worker and
        // apply its tidied graph as the run loop would. (SQ-0261)
        drive_retidy(&mut s, &mut m);
        let p1 = m.graph.room(1).unwrap().pos.unwrap();
        let p2 = m.graph.room(2).unwrap().pos.unwrap();
        assert!(p2.0 > p1.0, "after retidy room 2 must be east of room 1: {p2:?} vs {p1:?}");
    }

    /// Test helper: `Retidy` spawns an off-thread build (SQ-0261). Join it and write
    /// the tidied graph back into `m`, mirroring the run loop's apply step, so tests
    /// can assert on the final layout synchronously. A no-op if no job was spawned
    /// (e.g. Manual mode, where Retidy is a no-op).
    fn drive_retidy(s: &mut AppState, m: &mut Mapper) {
        if let Some(job) = s.anim_build_job.take() {
            assert!(!job.animate, "Retidy must build with animate=false");
            let (_frames, tidied) = job.handle.join().expect("retidy worker completes");
            m.graph = tidied;
        }
    }

    // ── Map-memo invalidation: production paths bump graph_gen (SQ-0305) ──────────
    // The map render model is memoized on (graph_gen, viewed_layer). Any graph edit
    // that reaches the live path with an unchanged graph_gen paints a STALE MAP, so
    // each edit path must bump. These drive the real apply_action code, not a manual
    // bump, so a regression that drops a bump fails here.

    #[test]
    fn rename_room_prompt_submit_bumps_graph_gen() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "Old Name", None);
        s.select_room(Some(1));
        let dlg = crate::state::TextEntryDialog::new(
            crate::state::TextEntryKind::RenameRoom(1),
            "New Name",
        );
        let before = s.graph_gen;
        apply_text_entry(dlg, &mut s, &mut m);
        assert_eq!(m.graph.room(1).unwrap().label_override.as_deref(), Some("New Name"),
            "rename actually applied");
        assert_ne!(s.graph_gen, before, "renaming a room must invalidate the map memo");
    }

    #[test]
    fn delete_connection_bumps_graph_gen() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(mapper::direction::Direction::E)); // edge origin=1 --E--> 2
        assert!(m.graph.connections().iter().any(|c| c.origin == 1),
            "fixture must have an outgoing edge from room 1");
        s.select_room(Some(1));
        let before = s.graph_gen;
        apply_action(Action::DeleteSelectedConnection, &mut s, &mut m);
        assert!(!m.graph.connections().iter().any(|c| c.origin == 1), "edge actually deleted");
        assert_ne!(s.graph_gen, before, "deleting a connection must invalidate the map memo");
    }

    #[test]
    fn animate_tidy_captures_frames_and_lands_on_instant_retidy() {
        use mapper::direction::Direction::E;
        // Two mappers with identical scrambled input: one animated, one instant-tidied.
        let build = || {
            let mut m = Mapper::default();
            m.observe(1, "A", None);
            m.observe(2, "B", Some(E));
            m.observe(3, "C", Some(E));
            m.graph.set_pos(1, (5, 5));
            m.graph.set_pos(2, (0, 0));
            m.graph.set_pos(3, (2, 9));
            m
        };
        let (mut s_anim, mut m_anim) = (AppState::default(), build());
        let (mut s_inst, mut m_inst) = (AppState::default(), build());
        apply_action(Action::AnimateTidy, &mut s_anim, &mut m_anim);
        apply_action(Action::Retidy, &mut s_inst, &mut m_inst);
        // The instant re-tidy also builds off-thread now (progress bar); drive it to
        // completion so m_inst holds the tidied graph as the oracle. (SQ-0261)
        drive_retidy(&mut s_inst, &mut m_inst);

        // The frames are now built on a worker thread: AnimateTidy spawns a build job and
        // does NOT touch tidy_anim or the live graph synchronously.
        assert!(s_anim.tidy_anim.is_none(), "no synchronous animation install");
        let job = s_anim.anim_build_job.expect("build job spawned");
        assert!(job.total > 0, "build job carries a positive progress estimate");
        let progress = std::sync::Arc::clone(&job.progress);
        // Drive the async path by joining the worker (the run loop does this + installs).
        let (frames, tidied) = job.handle.join().expect("worker completes");
        assert!(frames.len() >= 2, "at least before + one layout stage frame");
        assert_eq!(
            progress.load(std::sync::atomic::Ordering::Relaxed),
            frames.len(),
            "progress counter is bumped once per emitted frame",
        );
        // The worker's frames and tidied graph match the instant-tidy result room-for-room.
        for id in [1u16, 2, 3] {
            let inst = m_inst.graph.room(id).unwrap().pos;
            assert_eq!(frames.last().unwrap().graph.room(id).unwrap().pos, inst);
            assert_eq!(tidied.room(id).unwrap().pos, inst);
        }
    }

    #[test]
    fn animate_tidy_is_noop_when_build_or_anim_already_active() {
        use crate::state::{TidyAnim, TidyFrame};
        // A build job already in flight: AnimateTidy does not spawn a second one.
        {
            let mut s = AppState::default();
            let mut m = Mapper::default();
            m.observe(1, "A", None);
            apply_action(Action::AnimateTidy, &mut s, &mut m);
            assert!(s.anim_build_job.is_some(), "first invocation spawns a build job");
            let started = s.anim_build_job.as_ref().unwrap().started;
            apply_action(Action::AnimateTidy, &mut s, &mut m);
            assert_eq!(
                s.anim_build_job.as_ref().unwrap().started,
                started,
                "second invocation is a no-op (same job)"
            );
        }
        // An animation is already playing: AnimateTidy does not spawn a build job.
        {
            let mut s = AppState::default();
            let mut m = Mapper::default();
            m.observe(1, "A", None);
            s.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
                label: "x".into(),
                graph: m.graph.clone(),
                description: String::new(),
                stats: Default::default(),
                stage_start: true,
                manifest: None,
            }], mapper::layer::MAIN_LAYER));
            apply_action(Action::AnimateTidy, &mut s, &mut m);
            assert!(s.anim_build_job.is_none(), "no build job while an animation plays");
        }
    }

    #[test]
    fn anim_submode_routes_transport_keys_and_exits() {
        use crate::state::{TidyAnim, TidyFrame};
        let mut s = AppState::default();
        // No animation: arrows are the command line's (SQ-0599), not stepping.
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::CursorLeft));
        // Animation active: arrows step, Space toggles, Esc exits.
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new(), description: String::new(), stats: mapper::layout::TidyStats::default(), stage_start: false, manifest: None };
        s.tidy_anim = Some(TidyAnim::new(vec![frame("a"), frame("b")], mapper::layer::MAIN_LAYER));
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::AnimStep(-1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::AnimStep(1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(' '))), Action::AnimTogglePlay));
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::AnimExit));
        // The map stays scrollable during playback: hjkl + shift-arrows pan (SQ-0416),
        // +/- zoom. (Plain arrows step stages, so shift+Arrow is the arrow pan path.)
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('-'))), Action::ZoomOut));
        // Exit clears playback.
        apply_action(Action::AnimExit, &mut s, &mut Mapper::default());
        assert!(s.tidy_anim.is_none());
    }

    #[test]
    fn anim_step_clamps_pauses_and_holds_at_end() {
        use crate::state::{TidyAnim, TidyFrame};
        use std::time::Duration;
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new(), description: String::new(), stats: mapper::layout::TidyStats::default(), stage_start: false, manifest: None };
        let mut a = TidyAnim::new(vec![frame("a"), frame("b"), frame("c")], mapper::layer::MAIN_LAYER);
        assert!(a.playing && a.idx == 0);
        a.step(-1); // clamps at 0, and a manual step pauses
        assert_eq!(a.idx, 0);
        assert!(!a.playing, "manual step pauses playback");
        a.step(5); // clamps to last frame
        assert_eq!(a.idx, 2);
        // A paused, end-of-range animation never advances on tick.
        assert!(!a.tick(Duration::from_millis(0)));
        assert_eq!(a.idx, 2);
    }

    #[test]
    fn prompt_flow_rename_room() {
        // Set up a mapper with one room.
        let mut mapper = Mapper::default();
        mapper.observe(1, "Dark Room", None);

        let mut state = AppState::default();
        state.select_room(Some(1));

        // 'r' with the dialog closed is ordinary typing (SQ-0599).
        assert!(matches!(key_to_action(&state, key(KeyCode::Char('r'))), Action::InputChar('r')));

        // With dialog open, 'r' → RenameRoom action → the text-entry dialog opens.
        state.overlays.hotkey_dialog = true;
        let a = key_to_action(&state, key(KeyCode::Char('r')));
        assert!(matches!(a, Action::RenameRoom));
        apply_action(a, &mut state, &mut mapper);
        // apply_action clears hotkey_dialog when opening the dialog.
        assert!(!state.overlays.hotkey_dialog, "hotkey_dialog cleared when the dialog opens");
        let dlg = state.overlays.text_entry.as_ref().expect("text-entry dialog opened");
        assert!(matches!(dlg.kind, crate::state::TextEntryKind::RenameRoom(1)));
        assert_eq!(state.overlays.dialog_focus, 0, "focus starts on the field");

        // Type "Lit Room" into the field via the dialog's key routing.
        for c in "Lit Room".chars() {
            let d = state.overlays.text_entry.as_mut().unwrap();
            crate::render::text_entry_dialog::text_entry_dialog_key(KeyCode::Char(c), &mut d.field, 0);
        }
        assert_eq!(state.overlays.text_entry.as_ref().unwrap().field.value, "Lit Room");

        // Submit (what the run loop does on Enter) → mapper updated, dialog cleared.
        let dlg = state.overlays.text_entry.take().unwrap();
        apply_text_entry(dlg, &mut state, &mut mapper);
        assert!(state.overlays.text_entry.is_none());
        assert_eq!(mapper.graph.room(1).unwrap().label(), "Lit Room");
    }

    #[test]
    fn prompt_esc_cancels_without_applying() {
        use crate::render::text_entry_dialog::{text_entry_dialog_key, TextEntryAction};
        let mut mapper = Mapper::default();
        mapper.observe(1, "Original", None);

        let mut state = AppState::default();
        state.toggle_focus();
        state.select_room(Some(1));

        // Open rename dialog, type something.
        apply_action(Action::RenameRoom, &mut state, &mut mapper);
        let d = state.overlays.text_entry.as_mut().unwrap();
        text_entry_dialog_key(KeyCode::Char('X'), &mut d.field, 0);
        assert_eq!(state.overlays.text_entry.as_ref().unwrap().field.value, "X");

        // Esc resolves to Cancel; the run loop drops the dialog without applying.
        let d = state.overlays.text_entry.as_mut().unwrap();
        let (act, _) = text_entry_dialog_key(KeyCode::Esc, &mut d.field, 0);
        assert!(matches!(act, TextEntryAction::Cancel));
        state.overlays.text_entry = None;
        // Room name unchanged.
        assert_eq!(mapper.graph.room(1).unwrap().label(), "Original");
    }

    #[test]
    fn edit_notes_prefills_with_existing_notes() {
        // SQ-0524: the dialog should open with the room's current notes already
        // in the field (caret at the end) instead of empty.
        let mut mapper = Mapper::default();
        mapper.observe(1, "Dark Room", None);
        mapper.set_notes(1, "watch for the loose brick".to_string());

        let mut state = AppState::default();
        state.toggle_focus();
        state.select_room(Some(1));

        apply_action(Action::EditNotes, &mut state, &mut mapper);
        let dlg = state.overlays.text_entry.as_ref().expect("text-entry dialog opened");
        assert!(matches!(dlg.kind, crate::state::TextEntryKind::EditNotes(1)));
        assert_eq!(dlg.field.value, "watch for the loose brick");
        assert_eq!(
            dlg.field.cursor,
            "watch for the loose brick".chars().count(),
            "caret starts at the end of the prefilled text"
        );
    }

    #[test]
    fn edit_notes_opens_empty_for_a_room_with_no_notes() {
        let mut mapper = Mapper::default();
        mapper.observe(1, "Dark Room", None);

        let mut state = AppState::default();
        state.toggle_focus();
        state.select_room(Some(1));

        apply_action(Action::EditNotes, &mut state, &mut mapper);
        let dlg = state.overlays.text_entry.as_ref().expect("text-entry dialog opened");
        assert_eq!(dlg.field.value, "");
    }

    #[test]
    fn edit_notes_submit_empty_still_clears_notes() {
        let mut mapper = Mapper::default();
        mapper.observe(1, "Dark Room", None);
        mapper.set_notes(1, "old note".to_string());

        let mut state = AppState::default();
        state.toggle_focus();
        state.select_room(Some(1));

        apply_action(Action::EditNotes, &mut state, &mut mapper);
        // Clear the prefilled field (select-all/delete, as a user would).
        let d = state.overlays.text_entry.as_mut().unwrap();
        for _ in 0.."old note".chars().count() {
            d.field.backspace();
        }
        assert_eq!(d.field.value, "");

        let dlg = state.overlays.text_entry.take().unwrap();
        apply_text_entry(dlg, &mut state, &mut mapper);
        assert_eq!(mapper.graph.room(1).unwrap().notes, "");
    }

    #[test]
    fn game_focus_enter_returns_submit_command_with_current_input() {
        let mut s = AppState::default();
        // Pre-populate input.
        s.push_input_char('g');
        s.push_input_char('o');
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(matches!(a, Action::SubmitCommand(ref c) if c == "go"));
    }

    #[test]
    fn ctrl_a_toggles_alignment_overlay() {
        // toggle_alignment is palette-only now (SQ-0446 dropped its leader letter);
        // it has never had a direct key. Ctrl+A is the readline "move to start"
        // shortcut at a live story prompt (SQ-0447), so it resolves to CursorHome
        // there instead of None.
        let s = AppState::default();
        assert!(!s.show_alignment, "off by default");
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('a'))), Action::CursorHome));
        // In Map focus (no line prompt live) Ctrl+A is unbound, same as before.
        let mut s_map = AppState::default();
        s_map.focus = Focus::Map;
        assert!(matches!(key_to_action(&s_map, ctrl(KeyCode::Char('a'))), Action::None));
        // The action itself still works when dispatched directly.
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::ToggleAlignment, &mut s, &mut m);
        assert!(s.show_alignment, "toggled on");
        apply_action(Action::ToggleAlignment, &mut s, &mut m);
        assert!(!s.show_alignment, "toggled off");
    }

    #[test]
    fn ctrl_p_toggles_portal_labels() {
        // toggle_portal_labels is dialog-only (leader letter 'l' after SQ-0446);
        // it has never had a direct key. Ctrl+P is now the leader-dialog prefix
        // itself (moved from Ctrl+K, SQ-0447), so pressing it opens the hotkey dialog.
        let s = AppState::default();
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('p'))),
            Action::OpenHotkeyDialog
        ));
        // The action itself still works when dispatched directly.
        let mut s = AppState::default();
        let mut m = mapper::mapper::Mapper::default();
        assert!(!s.show_portal_labels, "default off");
        apply_action(Action::TogglePortalLabels, &mut s, &mut m);
        assert!(s.show_portal_labels, "TogglePortalLabels turns labels on");
        apply_action(Action::TogglePortalLabels, &mut s, &mut m);
        assert!(!s.show_portal_labels, "TogglePortalLabels toggles back off");
    }

    #[test]
    fn toggle_sound_lazily_builds_backend_when_turned_on() {
        // Regression: launching with enable_sound = false never constructs an
        // AudioBackend (see main.rs), so flipping the config flag alone leaves
        // state.audio == None forever. ToggleSound must build the backend too.
        audio::disable_output_for_tests(); // ToggleSound builds a real backend; keep it silent
        let mut s = AppState::default();
        s.config.enable_sound = false;
        s.audio = None;
        let mut m = Mapper::default();
        apply_action(Action::ToggleSound, &mut s, &mut m);
        assert!(s.config.enable_sound, "ToggleSound should turn sound on");
        assert!(s.audio.is_some(), "ToggleSound should lazily construct the AudioBackend");
        // The toggle also queues a VM Sound-gestalt sync (drained by the event
        // loop) so a running Glulx game that re-checks gestalt_Sound honors it.
        assert_eq!(s.pending_vm_sound, Some(true), "turning sound on queues a gestalt sync to true");
        apply_action(Action::ToggleSound, &mut s, &mut m);
        assert!(!s.config.enable_sound, "second toggle turns sound off");
        assert_eq!(s.pending_vm_sound, Some(false), "turning sound off queues a gestalt sync to false");
    }

    #[test]
    fn config_save_queues_live_watch_style_reconcile() {
        // Saving the settings screen must hand the new watch_style to the run
        // loop (which owns the file-watcher) so the watcher starts/stops live.
        let mut s = AppState::default();
        let mut working = clone_config(&s.config);
        working.watch_style = true;
        s.overlays.config_screen =
            Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
        apply_action(Action::ConfigSave, &mut s, &mut Mapper::default());
        assert_eq!(s.pending_watch_style, Some(true), "ConfigSave should queue a live watch reconcile");
        assert!(s.config.watch_style, "ConfigSave should apply the working config");
    }

    /// SQ-1161: Save applies to the RUNNING session everything it can, not just to
    /// the file. Two of these live on `AppState` rather than on the config the row
    /// edits (render reads the mirror, and the toggle keys drive it), and four more
    /// are the `_base` a per-story source overrides for one launch — what
    /// `/set-guidance auto` and its siblings fall back to. Every one of them was
    /// written to `config.toml` and left untouched on screen until the next launch.
    #[test]
    fn config_save_applies_the_live_mirrors_and_the_global_bases() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // A session booted on the opposite of everything the working copy asks for.
        s.show_status_bar = true;
        s.show_room_numbers = false;
        s.guidance_base = false;
        s.return_probe_base = true;
        s.v6_pixel_lock_base = false;
        s.v6_render_base = crate::config::V6RenderMode::Hybrid;

        let mut working = clone_config(&s.config);
        working.show_status_bar = false;
        working.show_room_numbers = true;
        working.guidance = true;
        working.return_probe = false;
        working.v6_pixel_lock = true;
        working.v6_render = crate::config::V6RenderMode::Raster;
        s.overlays.config_screen =
            Some(crate::state::ConfigScreenState { working, scroll: Default::default() });

        apply_action(Action::ConfigSave, &mut s, &mut m);

        assert!(!s.show_status_bar, "the status bar hides on Save, not on next launch");
        assert!(s.show_room_numbers, "and the room numbers appear on Save");
        assert!(s.guidance_base, "the global guidance default follows the row");
        assert!(!s.return_probe_base, "so does the return probe's");
        assert!(s.v6_pixel_lock_base, "and the pixel lock's");
        assert_eq!(s.v6_render_base, crate::config::V6RenderMode::Raster, "and the render mode's");
    }

    /// …but a key a one-run source is PINNING is not the user speaking (SQ-0807):
    /// the row was not edited (editing releases the pin), so `working` still holds
    /// this story's value and lowering it into the global base would turn one
    /// game's choice into everyone's.
    #[test]
    fn config_save_does_not_lower_a_pinned_value_into_the_global_base() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.guidance_base = true;
        s.config.guidance = false;
        s.config.one_run.pin(crate::config::keys::GUIDANCE, false);

        apply_action(Action::OpenConfig, &mut s, &mut m);
        let unrelated = crate::render::config_screen::CONFIG_ROWS
            .iter()
            .position(|(n, _, _)| *n == "show_room_numbers")
            .expect("the row exists");
        if let Some(cs) = &mut s.overlays.config_screen {
            config_cycle(&mut cs.working, unrelated, 1);
        }
        apply_action(Action::ConfigSave, &mut s, &mut m);
        assert!(
            s.guidance_base,
            "saving an unrelated row must not turn `--guidance off` into the global default"
        );
    }

    /// SQ-1161: `startup.rs` pins `V6_RENDER` from this game's sidecar exactly as it
    /// pins the pixel lock, and the settings row had no arm in `one_run_key_for_row`
    /// — so cycling the render mode released nothing and `ConfigDoc::put` skipped the
    /// key as still-one-run. The user's global choice was reported saved and was not.
    #[test]
    fn cycling_the_v6_render_row_ends_this_games_hold_on_it() {
        use crate::config::{V6RenderMode, keys};
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // What a per-game `v6_render = "raster"` sidecar leaves behind at boot.
        s.config.v6_render = V6RenderMode::Raster;
        s.config.one_run.pin(keys::V6_RENDER, crate::config::v6_render_key(V6RenderMode::Raster));

        apply_action(Action::OpenConfig, &mut s, &mut m);
        let row = crate::render::config_screen::CONFIG_ROWS
            .iter()
            .position(|(n, _, _)| *n == "v6_render")
            .expect("the row exists");
        if let Some(cs) = &mut s.overlays.config_screen {
            config_cycle(&mut cs.working, row, 1);
            assert!(
                !cs.working.one_run.holds(keys::V6_RENDER),
                "editing the row is the user speaking about every game"
            );
        }
        apply_action(Action::ConfigSave, &mut s, &mut m);
        assert_eq!(s.config.v6_render, V6RenderMode::Extended);
        assert_eq!(s.v6_render_base, V6RenderMode::Extended, "and it becomes the global default");
    }

    /// SQ-0807: editing a settings row ends the one-run hold on its key.
    /// `--sound off` pins `enable_sound = false`; toggling the row on and back off again lands on
    /// the very value the flag asked for, and the value-equality rule alone would
    /// read that as "still the flag's" and refuse to save the user's actual choice.
    #[test]
    fn editing_a_settings_row_promotes_a_one_run_value_to_a_persisted_one() {
        audio::disable_output_for_tests();
        let dir = std::env::temp_dir().join(format!("bm-row-promote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "enable_sound = true\n").unwrap();

        // What `--sound off` leaves behind.
        let mut s = AppState::default();
        s.config.config_file = cfg_path.clone();
        s.config.enable_sound = false;
        s.config.one_run.pin(crate::config::keys::ENABLE_SOUND, false);

        // Open the settings screen and work the sound row: on, then off again.
        let mut m = Mapper::default();
        apply_action(Action::OpenConfig, &mut s, &mut m);
        // By NAME: `config_cycle` matches on the row's position, so a row inserted
        // above this one retargets a literal at its neighbour (SQ-0873).
        let sound_row = crate::render::config_screen::CONFIG_ROWS
            .iter()
            .position(|(n, _, _)| *n == "enable_sound")
            .expect("the row exists");
        if let Some(cs) = &mut s.overlays.config_screen {
            config_cycle(&mut cs.working, sound_row, 1);
            config_cycle(&mut cs.working, sound_row, 1);
            assert!(!cs.working.enable_sound, "two toggles land back on off");
        }
        apply_action(Action::ConfigSave, &mut s, &mut m);
        crate::config::write_config_file(&s.config).unwrap();

        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            !toml::from_str::<crate::config::Config>(&back).unwrap().enable_sound,
            "the user's own off must persist even though it matches the flag: {back}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0860: the same rule, one layer further out. A one-run hold on
    /// `honor_game_colours` also lives on `AppState` — the artwork's force-off and
    /// `--game-colours` — and `reload_style` re-applies those on every reload,
    /// so releasing the `one_run` pin alone would let the next style reload
    /// recompute the user's deliberate choice straight back off. Editing the row
    /// must end both holds; saving with the row untouched must end neither.
    #[test]
    fn editing_the_honour_row_ends_the_artworks_hold_too() {
        let dir = std::env::temp_dir().join(format!("bm-honour-row-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("style.toml"), "[colors]\n\"transcript\" = { fg = \"white\" }\n")
            .unwrap();
        let honour_row = crate::render::config_screen::CONFIG_ROWS
            .iter()
            .position(|(n, _, _)| *n == "honor_game_colours")
            .expect("the row exists");

        // What a boot on a two-colour archive leaves behind (SQ-0806/SQ-0846).
        let seed = |dir: &std::path::Path| {
            let mut s = AppState::default();
            s.config.user_dir = dir.to_path_buf();
            s.config.style = Some(dir.join("style.toml").to_string_lossy().to_string());
            s.honor_game_colours_base = true;
            s.artwork_declines_colours = true;
            s.config.honor_game_colours = false;
            s.config.one_run.pin(crate::config::keys::HONOR_GAME_COLOURS, false);
            s
        };

        let mut s = seed(&dir);
        let mut m = Mapper::default();
        apply_action(Action::OpenConfig, &mut s, &mut m);
        if let Some(cs) = &mut s.overlays.config_screen {
            config_cycle(&mut cs.working, honour_row, 1);
            assert!(cs.working.honor_game_colours, "the row turns the game's colours on");
        }
        apply_action(Action::ConfigSave, &mut s, &mut m);
        assert!(!s.artwork_declines_colours, "a deliberate edit outranks a guess about a machine");
        assert!(s.config.honor_game_colours);
        crate::reload::reload_style(&mut s);
        assert!(s.config.honor_game_colours, "and the next style reload does not undo it");

        // The same save with the row untouched changes nothing: the artwork's
        // force-off is still in force for this run.
        let mut s = seed(&dir);
        apply_action(Action::OpenConfig, &mut s, &mut m);
        if let Some(cs) = &mut s.overlays.config_screen {
            config_cycle(&mut cs.working, 5, 1); // show_room_numbers — an unrelated row
        }
        apply_action(Action::ConfigSave, &mut s, &mut m);
        assert!(s.artwork_declines_colours, "saving some other setting is not a choice about this one");
        crate::reload::reload_style(&mut s);
        assert!(!s.config.honor_game_colours, "so the archive still has the last word");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bracket_keys_cycle_layer_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // CycleLayer is dialog-only: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::None));
        s.overlays.hotkey_dialog = true;
        // Brackets are not authored leader letters: they close the dialog now.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::CloseHotkeyDialog));
        // The authored leader letter 'c' fires cycle-layer next instead.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('c'))), Action::CycleLayer(1)));
    }

    #[test]
    fn set_viewed_layer_action_selects_that_layer() {
        // A layer-tab click dispatches Action::SetViewedLayer(id); the handler
        // makes it the viewed layer.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        assert_eq!(s.viewed_layer, None);
        apply_action(Action::SetViewedLayer(2), &mut s, &mut mapper);
        assert_eq!(s.viewed_layer, Some(2), "SetViewedLayer selects the clicked layer");
    }

    #[test]
    fn shift_p_peels_and_shift_m_merges_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // The layer commands are dialog-only: return None when the dialog is closed.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('P'))), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('M'))), Action::None));
        // Open dialog: authored leader letters 'p'/'m' fire the commands
        // (Shift+P/Shift+M are no longer authored leader letters).
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('p'))), Action::MoveRegion(ref a) if a == "new"));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('m'))), Action::MoveRegion(ref a) if a == "parent"));
    }

    // ── Autocomplete / Tab precedence tests ───────────────────────────────────

    #[test]
    fn tab_is_toggle_focus_with_empty_input() {
        // Game focus, empty input, no suggestions → Tab is ToggleFocus.
        let s = AppState::default(); // focus = Game, input = "", suggestions = []
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
    }

    #[test]
    fn tab_is_toggle_focus_with_input_but_no_suggestions() {
        // Game focus, non-empty partial, but no suggestions (dict not loaded) →
        // Tab is still ToggleFocus.
        let mut s = AppState::default();
        s.input.set("nor", true);
        // suggestions is empty by default
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
    }

    #[test]
    fn tab_is_autocomplete_when_suggestions_available() {
        // Game focus, non-empty partial, suggestions populated → Tab is Autocomplete.
        let mut s = AppState::default();
        s.input.set("nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::Autocomplete));
    }

    #[test]
    fn tab_is_toggle_focus_in_map_focus_even_with_suggestions() {
        // Map focus: Tab always toggles focus regardless of suggestions.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        s.suggestions = vec!["north".to_string()]; // not relevant for map focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
    }

    #[test]
    fn autocomplete_action_replaces_partial_word() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::Autocomplete, &mut s, &mut m);
        // "nor" should be replaced with "north" (index 0 suggestion).
        assert_eq!(s.input.value, "go north");
        // The highlight stays on the applied candidate (index 0), so the bracket
        // matches the command line; the next Tab advances.
        assert_eq!(s.suggestion_idx, 0);
        assert!(s.suggestion_active);
    }

    #[test]
    fn autocomplete_slash_command_preserves_prefix() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // Slash suggestions hold the bare command name (no prefix).
        s.input.set("/sav", true);
        s.suggestions = vec!["save".to_string(), "save-as".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::Autocomplete, &mut s, &mut m);
        // The leading prefix must survive completion.
        assert_eq!(s.input.value, "/save");
        assert_eq!(s.suggestion_idx, 0);
    }

    #[test]
    fn autocomplete_cycles_on_repeated_tab() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 0;
        // First Tab: north (applies the highlighted candidate, no advance).
        apply_action(Action::Autocomplete, &mut s, &mut m);
        assert_eq!(s.input.value, "go north");
        assert_eq!(s.suggestion_idx, 0);
        // Second Tab: advances to northeast, replacing the prior completion in
        // place (no need to retype the partial).
        apply_action(Action::Autocomplete, &mut s, &mut m);
        assert_eq!(s.input.value, "go northeast");
        assert_eq!(s.suggestion_idx, 1);
        // Third Tab: wraps back to north.
        apply_action(Action::Autocomplete, &mut s, &mut m);
        assert_eq!(s.input.value, "go north");
        assert_eq!(s.suggestion_idx, 0);
    }

    #[test]
    fn autocomplete_highlight_tracks_command_line() {
        // Regression: the bracketed suggestion must highlight the word that is
        // currently on the command line, not the next one. Cycling forward with
        // repeated Tab keeps the highlight in sync at every step.
        use crate::render::transcript::format_suggestion_line;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string(), "nowhere".to_string()];
        s.suggestion_idx = 0;
        for expected in ["north", "northeast", "nowhere", "north"] {
            apply_action(Action::Autocomplete, &mut s, &mut m);
            assert_eq!(s.input.value, format!("go {expected}"));
            // The bracketed entry on the suggestion line equals the applied word.
            let line = format_suggestion_line(&s.suggestions, s.suggestion_idx);
            assert!(
                line.contains(&format!("[{expected}]")),
                "suggestion line {line:?} must bracket the word now on the command line ({expected})"
            );
        }
    }

    // ── Shift-Tab reverse cycling (feature A) ──────────────────────────────────

    #[test]
    fn shift_tab_is_autocomplete_prev_when_suggestions_available() {
        // Game focus, non-empty partial, suggestions populated → Shift-Tab is
        // AutocompletePrev (the inverse of Tab's Autocomplete).
        let mut s = AppState::default();
        s.input.set("nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        assert!(matches!(key_to_action(&s, key(KeyCode::BackTab)), Action::AutocompletePrev));
    }

    #[test]
    fn shift_tab_cycles_focus_back_without_suggestions() {
        // Game focus, partial typed but no suggestions → no AutocompletePrev, so
        // Shift-Tab reverses the per-window focus cycle.
        let mut s = AppState::default();
        s.input.set("nor", true);
        assert!(matches!(key_to_action(&s, key(KeyCode::BackTab)), Action::CycleFocusBack));
    }

    #[test]
    fn shift_tab_cycles_focus_back_with_empty_input() {
        let s = AppState::default(); // Game focus, empty input, no suggestions
        assert!(matches!(key_to_action(&s, key(KeyCode::BackTab)), Action::CycleFocusBack));
    }

    #[test]
    fn shift_tab_cycles_focus_back_in_map_focus() {
        // Not in Game focus, so the autocomplete intercept does not apply; Shift-Tab
        // reverses the window focus cycle.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        s.suggestions = vec!["north".to_string()];
        assert!(matches!(key_to_action(&s, key(KeyCode::BackTab)), Action::CycleFocusBack));
    }

    #[test]
    fn autocomplete_prev_action_replaces_partial_and_steps_back() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::AutocompletePrev, &mut s, &mut m);
        // Applies the current (index 0) suggestion, like Autocomplete; the
        // highlight stays put so the bracket matches the command line.
        assert_eq!(s.input.value, "go north");
        assert_eq!(s.suggestion_idx, 0);
        assert!(s.suggestion_active);
    }

    #[test]
    fn autocomplete_prev_cycles_backward_with_wrap() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string(), "nowhere".to_string()];
        s.suggestion_idx = 0;
        // First Shift-Tab: applies the highlighted index 0 (north), no step.
        apply_action(Action::AutocompletePrev, &mut s, &mut m);
        assert_eq!(s.input.value, "go north");
        assert_eq!(s.suggestion_idx, 0);
        // Second Shift-Tab: steps backward to index 2 (nowhere), replacing in place.
        apply_action(Action::AutocompletePrev, &mut s, &mut m);
        assert_eq!(s.input.value, "go nowhere");
        assert_eq!(s.suggestion_idx, 2);
    }

    #[test]
    fn autocomplete_prev_slash_command_preserves_prefix() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("/sav", true);
        s.suggestions = vec!["save".to_string(), "save-as".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::AutocompletePrev, &mut s, &mut m);
        assert_eq!(s.input.value, "/save");
        assert_eq!(s.suggestion_idx, 0);
    }

    #[test]
    fn typing_resets_suggestion_index() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // Pre-load some suggestions and set idx > 0.
        s.input.set("no", true);
        s.dict_words = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 1;
        // Type another character: should recompute suggestions and reset idx to 0.
        apply_action(Action::InputChar('r'), &mut s, &mut m);
        assert_eq!(s.suggestion_idx, 0);
        // Suggestions should now match "nor".
        assert!(s.suggestions.iter().any(|w| w.starts_with("nor")));
    }

    // ── Leader-letter 'i' toggle tests ────────────────────────────────────────

    #[test]
    fn i_in_map_focus_yields_toggle_inventory() {
        // SQ-0446: 'i' is the View-group leader letter for toggle-inventory-panel now
        // (toggle-inspector moved to the '/' palette).
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // toggle-inventory-panel is dialog-only: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::None));
        // Returns the action when dialog is open.
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInventory));
    }

    #[test]
    fn i_in_game_focus_is_input_char_not_leader() {
        let s = AppState::default(); // game focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::InputChar('i')));
    }

    /// SQ-0692: `/toggle-inspector` no longer opens a second floating panel for a
    /// SELECTED room — it opens the one dock onto its diagnostics body, and flips
    /// back to Info when the dock is already up. Needing a selection first was the
    /// old command's worst property: the reading you usually want is of the room
    /// you are standing in, which it could never give you.
    #[test]
    fn toggle_inspector_opens_the_dock_in_diagnostics_then_flips_views() {
        use crate::state::RoomDockView;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(!s.room_dock.open, "the dock starts closed");

        // No selection needed: it opens on the followed room.
        apply_action(Action::ToggleRoomDiagnostics, &mut s, &mut m);
        assert!(s.room_dock.open, "a closed dock opens");
        assert_eq!(s.room_dock_view, RoomDockView::Diagnostics, "…onto the diagnostics body");
        assert_eq!(s.selected_room, None, "and does not pin anything");

        // Again with the dock open: flip to Info…
        apply_action(Action::ToggleRoomDiagnostics, &mut s, &mut m);
        assert!(s.room_dock.open, "the dock stays up — this flips views, it does not close");
        assert_eq!(s.room_dock_view, RoomDockView::Info);

        // …and back.
        apply_action(Action::ToggleRoomDiagnostics, &mut s, &mut m);
        assert_eq!(s.room_dock_view, RoomDockView::Diagnostics);
    }

    /// `toggle-room-panel` is the primary open/close command, and it opens on Info.
    #[test]
    fn toggle_room_dock_opens_on_info_and_closes() {
        use crate::state::RoomDockView;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.room_dock_view = RoomDockView::Diagnostics; // a stale view from last time

        apply_action(Action::ToggleRoomDock, &mut s, &mut m);
        assert!(s.room_dock.open);
        assert_eq!(s.room_dock_view, RoomDockView::Info, "the primary toggle opens on Info");

        apply_action(Action::ToggleRoomDock, &mut s, &mut m);
        assert!(!s.room_dock.open, "and the same command closes it");
    }

    /// The dock is NOT an overlay (SQ-0692). It reserves its own rows instead of
    /// covering the map, so it must not suppress the story prompt, the caret, or
    /// anything else gated on `any_overlay_open` — which counting the old room
    /// panel there did.
    #[test]
    fn the_open_room_dock_is_not_an_overlay() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::ToggleRoomDock, &mut s, &mut m);
        assert!(s.room_dock.open);
        assert!(!s.any_overlay_open(), "an open room panel is not an overlay");
        assert!(!s.any_modal_overlay_open(), "…and certainly not a modal one");

        apply_action(Action::PinRoomDock(3, crate::state::RoomDockView::Diagnostics), &mut s, &mut m);
        assert!(!s.any_overlay_open(), "nor is a PINNED dock");
    }

    /// Pin, re-pin, unpin — the click contract, at the action level.
    #[test]
    fn pinning_the_dock_selects_the_room_and_unpinning_clears_it() {
        use crate::state::RoomDockView;
        let mut s = AppState::default();
        let mut m = Mapper::default();

        apply_action(Action::PinRoomDock(5, RoomDockView::Info), &mut s, &mut m);
        assert!(s.room_dock.open, "pinning opens a closed dock");
        assert_eq!(s.selected_room, Some(5), "pin state IS the selection");
        assert!(s.room_dock_pinned());
        assert_eq!(s.room_dock_view, RoomDockView::Info);

        apply_action(Action::PinRoomDock(5, RoomDockView::Diagnostics), &mut s, &mut m);
        assert_eq!(s.selected_room, Some(5));
        assert_eq!(s.room_dock_view, RoomDockView::Diagnostics, "a right-click re-points the view");

        apply_action(Action::UnpinRoomDock, &mut s, &mut m);
        assert_eq!(s.selected_room, None, "unpinned: the dock follows the player again");
        assert!(!s.room_dock_pinned());
        assert!(s.room_dock.open, "…but stays up");
    }

    // ── Equivalence guard for the KeyMap refactor ──────────────────────────────

    /// This test encodes the CURRENT (pre-refactor) behavior of key_to_action for
    /// a representative sample across all contexts. It must pass both before and
    /// after the Task 4 refactor. If it fails after the refactor, the KeyMap
    /// defaults or lookup semantics diverge from today — fix the data, not the test.
    #[test]
    fn key_to_action_equivalence_sample() {
        use crate::state::{TidyAnim, TidyFrame};

        // ── Game focus (default) ──────────────────────────────────────────────
        let s = AppState::default(); // focus = Game
        // Direct ctrl commands work without the dialog.
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-state"));
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('r'))), KeyResolve::Command(c, _) if c == "restore-state"));
        // Non-direct ctrl commands return None when dialog is closed. Ctrl+D is
        // not one of them since SQ-1228 (transcript half-page-down).
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('g'))), Action::None));
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('d'))),
            Action::TranscriptScrollHalfPage(-1)
        ));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('l'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('y'))), Action::None));
        // Ctrl+A/E/P are no longer None in Game focus (SQ-0447): Ctrl+A/E are the
        // readline move-to-start/end shortcuts at the story prompt, and Ctrl+P is
        // now the hotkey-dialog prefix (moved off Ctrl+K).
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('a'))), Action::CursorHome));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::CursorEnd));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('p'))), Action::OpenHotkeyDialog));
        // Ctrl+Left/Right and F6-F9 are unbound (room nudging removed,
        // SQ-0600). Ctrl+Up/Down are the exception (SQ-0677): they recall
        // command history, restoring the plain arrows' shell-style behaviour
        // under a modifier now that the command band claims plain ↑/↓ for
        // its own row navigation while open.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Right)), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Up)), Action::HistoryPrev));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Down)), Action::HistoryNext));
        for f in [6u8, 7, 8, 9] {
            assert!(matches!(key_to_action(&s, key(KeyCode::F(f))), Action::None));
        }
        // Tab → ToggleFocus (no input, no suggestions)
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
        // Text entry
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::InputChar('n')));

        // ── Story focus: the old map-focus key set is typing now (SQ-0599) ────
        let mut s = AppState::default();
        // Every key that used to drive the focused map reaches the command line.
        for c in ['h', 'j', 'k', 'l', 'c', 'n', 'p', '+', '=', '-', '0'] {
            assert!(
                matches!(key_to_action(&s, key(KeyCode::Char(c))), Action::InputChar(x) if x == c),
                "'{c}' must be typing, not a map command"
            );
        }
        // Plain arrows edit the line; Shift+Arrows pan the map from right here.
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::CursorLeft));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::CursorRight));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
        // Direct ctrl globals are unaffected.
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-state"));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::None));

        // ── Leader dialog open ────────────────────────────────────────────────
        s.overlays.hotkey_dialog = true;
        // Dialog-only commands now fire via their authored leader letters (SQ-0446).
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::EditNotes));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('p'))), Action::MoveRegion(ref a) if a == "new"));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('m'))), Action::MoveRegion(ref a) if a == "parent"));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::RenameRoom));
        // 't' was Retidy until the Layout group left the panel; unauthored letters
        // close the dialog rather than firing, so it belongs with the group below.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('t'))), Action::CloseHotkeyDialog));
        // Unauthored keys (shift-modified letters, brackets, dropped letters)
        // close the dialog instead of firing.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::RenameRoom));
        // 'o' (old edit-notes letter) is now unbound → closes the dialog.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('o'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('d'))), Action::DeleteSelectedConnection));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('e'))), Action::RelabelSelectedEdge));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInventory));
        // 'q' is deliberately unassigned → it closes the dialog (quit convention).
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('q'))), Action::CloseHotkeyDialog));
        s.overlays.hotkey_dialog = false;

        // ── Anim sub-mode ─────────────────────────────────────────────────────
        let mut s = AppState::default();
        s.focus = Focus::Map;
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new(), description: String::new(), stats: mapper::layout::TidyStats::default(), stage_start: false, manifest: None };
        s.tidy_anim = Some(TidyAnim::new(vec![frame("a"), frame("b")], mapper::layer::MAIN_LAYER));
        // Step
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::AnimStep(-1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::AnimStep(1)));
        // Play/pause
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(' '))), Action::AnimTogglePlay));
        // Exit
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::AnimExit));
        assert!(matches!(key_to_action(&s, key(KeyCode::Enter)), Action::AnimExit));
        // Pan in anim: hjkl + shift-arrows (SQ-0416). Plain arrows step stages, so
        // shift+Arrow is the arrow-key pan path during playback.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        // Zoom in anim
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('-'))), Action::ZoomOut));
        // Anim does NOT fall through to Global: unknown key → None
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::None));
    }

    // ── Saves-manager sub-mode tests ──────────────────────────────────────────

    fn state_with_saves_open() -> AppState {
        use crate::state::{SavesState};
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState {
            entries: vec![
                SaveInfo {
                    path: PathBuf::from("/tmp/default.lanthorn"),
                    name: "(default)".to_string(),
                    turns: 0,
                    saved_at: String::new(),
                    location: None, score: None, is_default: true, trigger: crate::archive::SaveTrigger::HostState,
                },
                SaveInfo {
                    path: PathBuf::from("/tmp/named.lanthorn"),
                    name: "before-troll".to_string(),
                    turns: 10,
                    saved_at: "2026-06-18T10:00:00Z".to_string(),
                    location: None, score: None, is_default: false, trigger: crate::archive::SaveTrigger::HostState,
                },
            ],
            scroll: Default::default(),
        });
        s
    }

    #[test]
    fn saves_submode_up_down_navigates() {
        let mut s = state_with_saves_open();
        // Down moves selection from 0 to 1.
        let a = key_to_action(&s, key(KeyCode::Down));
        assert!(matches!(a, Action::SavesNav(1)));
        apply_action(a, &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.saves.as_ref().unwrap().scroll.selected, 1);
        // Up moves back to 0.
        let a = key_to_action(&s, key(KeyCode::Up));
        assert!(matches!(a, Action::SavesNav(-1)));
        apply_action(a, &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.saves.as_ref().unwrap().scroll.selected, 0);
    }

    #[test]
    fn saves_submode_s_opens_save_name_dialog() {
        let mut s = state_with_saves_open();
        let a = key_to_action(&s, key(KeyCode::Char('s')));
        assert!(matches!(a, Action::SavesSaveAs));
        apply_action(a, &mut s, &mut Mapper::default());
        // SavesSaveAs now opens the common-dialog save-name modal (not a bottom-bar
        // prompt), prefilled with a greyed date-time default, focused on the field.
        assert!(s.overlays.text_entry.is_none(), "no text-entry dialog is opened for save-as");
        let dlg = s.overlays.save_name_dialog.as_ref().expect("save-name dialog opened");
        assert!(!dlg.active, "opens greyed (placeholder) until edited");
        assert!(!dlg.ingame, "host Save State context");
        assert!(!dlg.field.value.is_empty(), "prefilled with a default name");
        assert_eq!(s.overlays.dialog_focus, 0, "focus starts on the text field");
    }

    #[test]
    fn saves_submode_d_opens_confirm_delete_dialog() {
        let mut s = state_with_saves_open();
        // Select entry 1 (the named save).
        s.overlays.saves.as_mut().unwrap().scroll.selected = 1;
        let a = key_to_action(&s, key(KeyCode::Char('d')));
        assert!(matches!(a, Action::SavesDelete));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.overlays.confirm_delete_save.is_some(), "SavesDelete opens the confirm-delete dialog");
        assert_eq!(s.overlays.dialog_focus, 1, "focus starts on Cancel (the safe default)");
    }

    #[test]
    fn saves_submode_esc_closes_modal() {
        let mut s = state_with_saves_open();
        let a = key_to_action(&s, key(KeyCode::Esc));
        assert!(matches!(a, Action::SavesClose));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.overlays.saves.is_none(), "Esc should close the saves modal");
    }

    #[test]
    fn saves_submode_enter_produces_saves_load() {
        let s = state_with_saves_open();
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(matches!(a, Action::SavesLoad));
    }

    #[test]
    fn ctrl_o_opens_saves_in_game_and_map_focus() {
        // open_saves is not in the direct set: Ctrl+O returns None when dialog is closed.
        let s = AppState::default();
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('o'))), Action::None));
        let mut s = AppState::default();
        s.toggle_focus();
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('o'))), Action::None));
        s.overlays.hotkey_dialog = true;
        // Ctrl-combos always close the dialog now (never fire). open-saves has no
        // leader letter; the SQ-0446 leader letter 's' opens the settings screen
        // (open-settings), reached via the saves dialog / Ctrl+R otherwise.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('o'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('s'))), Action::OpenConfig));
    }

    #[test]
    fn saves_nav_wraps_around() {
        use crate::state::SavesState;
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState {
            entries: vec![
                SaveInfo { path: PathBuf::from("/tmp/a.lanthorn"), name: "a".into(), turns: 0, saved_at: String::new(), location: None, score: None, is_default: false, trigger: crate::archive::SaveTrigger::HostState },
                SaveInfo { path: PathBuf::from("/tmp/b.lanthorn"), name: "b".into(), turns: 0, saved_at: String::new(), location: None, score: None, is_default: false, trigger: crate::archive::SaveTrigger::HostState },
            ],
            scroll: Default::default(),
        });
        s.overlays.saves.as_mut().unwrap().scroll.selected = 1;
        // Down from last wraps to first.
        apply_action(Action::SavesNav(1), &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.saves.as_ref().unwrap().scroll.selected, 0, "should wrap to 0 after last");
        // Up from first wraps to last.
        apply_action(Action::SavesNav(-1), &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.saves.as_ref().unwrap().scroll.selected, 1, "should wrap to last");
    }

    // ── Command palette dispatch tests (SQ-0419) ──────────────────────────────

    #[test]
    fn slash_at_empty_prompt_opens_palette() {
        let mut s = AppState::default(); // Game focus, empty input line.
        let a = key_to_action(&s, key(KeyCode::Char('/')));
        assert!(matches!(a, Action::OpenCommandPalette { from_hotkey: false }));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.overlays.palette.is_some(), "'/' at an empty prompt opens the palette");
    }

    #[test]
    fn slash_midline_stays_a_literal_char() {
        let mut s = AppState::default();
        s.input = crate::text_field::TextField::new("go");
        // '/' with a non-empty line is an ordinary character, not a palette trigger.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('/'))), Action::InputChar('/')));
    }

    #[test]
    fn slash_inside_hotkey_dialog_transitions_to_palette() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        let a = key_to_action(&s, key(KeyCode::Char('/')));
        assert!(matches!(a, Action::OpenCommandPalette { from_hotkey: true }));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.overlays.palette.is_some(), "'/' in the hotkey dialog opens the palette");
        assert!(!s.overlays.hotkey_dialog, "the hotkey dialog closes when the palette opens");
    }

    #[test]
    fn palette_esc_closes_and_preserves_empty_prompt() {
        let mut s = AppState::default();
        // '/' promotes the empty prompt; Esc returns to it unchanged (still empty).
        apply_action(key_to_action(&s, key(KeyCode::Char('/'))), &mut s, &mut Mapper::default());
        let a = key_to_action(&s, key(KeyCode::Esc));
        assert!(matches!(a, Action::PaletteClose));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.overlays.palette.is_none(), "Esc closes the palette");
        assert!(!s.overlays.hotkey_dialog, "prompt-promoted palette does not reopen the hotkey dialog");
        assert!(s.input.value.is_empty(), "the story prompt is preserved (empty)");
    }

    #[test]
    fn palette_esc_returns_to_hotkey_dialog_when_promoted_from_it() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        apply_action(key_to_action(&s, key(KeyCode::Char('/'))), &mut s, &mut Mapper::default());
        apply_action(Action::PaletteClose, &mut s, &mut Mapper::default());
        assert!(s.overlays.palette.is_none());
        assert!(s.overlays.hotkey_dialog, "Esc returns to the hotkey dialog it was promoted from");
    }

    #[test]
    fn palette_shift_tab_reverses_selection_cycler() {
        // Down/Up (and Shift-Tab as Up) cycle the selection with wrap.
        let mut s = AppState::default();
        s.overlays.palette = Some(crate::state::PaletteState::new(false));
        // Empty query → the whole registry is the candidate list.
        let n = crate::slash::COMMANDS.len();
        // Down moves to index 1.
        apply_action(key_to_action(&s, key(KeyCode::Down)), &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.palette.as_ref().unwrap().scroll.selected, 1);
        // Shift-Tab reverses back to 0.
        apply_action(key_to_action(&s, KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)), &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.palette.as_ref().unwrap().scroll.selected, 0);
        // Shift-Tab again wraps to the last entry (reverse of Down's wrap).
        apply_action(key_to_action(&s, KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)), &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.palette.as_ref().unwrap().scroll.selected, n - 1);
    }

    #[test]
    fn palette_typing_filters_and_tab_completes() {
        let mut s = AppState::default();
        s.overlays.palette = Some(crate::state::PaletteState::new(false));
        for c in "zoom".chars() {
            apply_action(Action::PaletteChar(c), &mut s, &mut Mapper::default());
        }
        // Best match for "zoom" is zoom-map; Tab completes the first token.
        apply_action(Action::PaletteComplete, &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.palette.as_ref().unwrap().input.value, "zoom-map ");
    }

    #[test]
    fn palette_enter_executes_selected_command_end_to_end() {
        // Type a safe no-arg toggle, Enter, dispatch as the run loop would, and
        // observe the state mutation.
        let mut s = AppState::default();
        s.overlays.palette = Some(crate::state::PaletteState::new(false));
        for c in "toggle-alignment".chars() {
            apply_action(Action::PaletteChar(c), &mut s, &mut Mapper::default());
        }
        let before = s.show_alignment;
        // Enter resolves to a Command through the palette handler.
        let resolved = key_to_command(&s, key(KeyCode::Enter));
        let (cmd, ctx) = match resolved {
            KeyResolve::Command(c, ctx) => (c, ctx),
            other => panic!("expected a Command, got {other:?}"),
        };
        assert_eq!(cmd, "toggle-alignment");
        // Dispatch it exactly like the run loop's Command arm.
        match crate::slash::parse_in_context(&cmd, '/', ctx) {
            crate::slash::SlashOutcome::Action(a) => apply_action(a, &mut s, &mut Mapper::default()),
            other => panic!("expected an Action outcome, got {other:?}"),
        }
        assert_ne!(s.show_alignment, before, "the toggle command mutated state end-to-end");
    }

    #[test]
    fn palette_enter_passes_typed_args_to_the_command() {
        // "zoom-map in" → the args ride along into the executed command line.
        let mut s = AppState::default();
        s.overlays.palette = Some(crate::state::PaletteState::new(false));
        for c in "zoom-map in".chars() {
            apply_action(Action::PaletteChar(c), &mut s, &mut Mapper::default());
        }
        let resolved = key_to_command(&s, key(KeyCode::Enter));
        match resolved {
            KeyResolve::Command(cmd, _) => assert_eq!(cmd, "zoom-map in"),
            other => panic!("expected a Command, got {other:?}"),
        }
    }

    // ── Hotkey dialog dispatch tests ──────────────────────────────────────────

    #[test]
    fn dialog_closed_dialog_only_cmd_returns_none() {
        // In map focus with dialog closed, a dialog-only command returns None.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // Retidy is bound to Shift+R in Map context but is NOT direct.
        assert!(matches!(
            key_to_action(&s, shift(KeyCode::Char('R'))),
            Action::None
        ));
        // toggle-inventory-panel ('i') is also dialog-only (SQ-0446).
        assert!(matches!(
            key_to_action(&s, key(KeyCode::Char('i'))),
            Action::None
        ));
    }

    #[test]
    fn dialog_closed_direct_cmd_still_works() {
        // With the dialog closed, direct commands still fire on their own keys.
        let s = AppState::default();
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-state"));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('q'))), Action::Quit));
    }

    #[test]
    fn prefix_opens_hotkey_dialog_action() {
        // Ctrl+P in any non-dialog state → OpenHotkeyDialog (prefix moved off
        // Ctrl+K to Ctrl+P, SQ-0447, freeing Ctrl+K for the readline delete-to-end
        // shortcut below).
        let s = AppState::default(); // game focus
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('p'))),
            Action::OpenHotkeyDialog
        ));
        let mut s = AppState::default();
        s.focus = Focus::Map;
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('p'))),
            Action::OpenHotkeyDialog
        ));
    }

    #[test]
    fn ctrl_k_deletes_to_end_at_story_prompt_not_hotkey_dialog() {
        // Ctrl+K used to be the hotkey-dialog prefix; now it's a readline
        // delete-to-end shortcut at the live story prompt (SQ-0447) and must NOT
        // open the palette.
        let mut s = AppState::default(); // game focus, line prompt live
        s.push_input_char('g');
        s.push_input_char('o');
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('k'))),
            Action::DeleteToEnd
        ));
        assert!(!s.overlays.hotkey_dialog);
    }

    #[test]
    fn prefix_closes_hotkey_dialog_action() {
        // Ctrl+P when dialog is open → CloseHotkeyDialog.
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('p'))),
            Action::CloseHotkeyDialog
        ));
    }

    #[test]
    fn q_closes_hotkey_dialog_as_unbound_leader() {
        // SQ-0446 deliberately leaves 'q' unassigned so it restores the universal
        // quit/close convention: an unbound leader letter closes the dialog.
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        let action = key_to_action(&s, key(KeyCode::Char('q')));
        assert!(
            matches!(action, Action::CloseHotkeyDialog),
            "bare 'q' (unbound leader letter) should close the hotkey dialog"
        );
    }

    #[test]
    fn dialog_open_dialog_only_cmd_fires() {
        // With the dialog open, an authored leader letter fires its command. This
        // used 't' for Retidy until the Layout group left the panel; 'r' and 'i'
        // are authored rows that remain.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::RenameRoom));
        // toggle-inventory-panel fires too (SQ-0446 gave 'i' to inventory).
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInventory));
    }

    #[test]
    fn apply_open_hotkey_dialog_sets_flag() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(!s.overlays.hotkey_dialog);
        apply_action(Action::OpenHotkeyDialog, &mut s, &mut m);
        assert!(s.overlays.hotkey_dialog);
        apply_action(Action::CloseHotkeyDialog, &mut s, &mut m);
        assert!(!s.overlays.hotkey_dialog);
    }

    /// `open-history` has THREE outcomes, and two of them used to be one silence.
    ///
    /// `record_turn_history` is opt-in and defaults to false, so for most players
    /// the command opened nothing and explained nothing — which is how it came to
    /// be reported as broken. "Nothing to replay" and "the capture is switched
    /// off" are different situations and only the second is actionable, so they
    /// now get different answers (SQ-1091).
    #[test]
    fn open_history_offers_to_switch_recording_on_rather_than_doing_nothing() {
        let mut m = Mapper::default();

        // Recording off, nothing recorded → offer to turn it on.
        let mut s = AppState::default();
        s.config.record_turn_history = false;
        apply_action(Action::OpenHistory, &mut s, &mut m);
        assert!(s.overlays.history_prompt, "the prompt must open when recording is off");
        assert!(s.overlays.replay.is_none(), "there is nothing to replay");
        assert_eq!(s.overlays.dialog_focus, 0, "focus starts on the affirmative button");

        // Recording ON but nothing yet → say so; do not offer what is already on.
        // SQ-1045: said in the assist voice now, in the transcript, rather than as
        // a bracketed toast that expires before advice can be acted on.
        let mut s = AppState::default();
        s.config.record_turn_history = true;
        s.assist_preamble_shown = true; // the once-per-session introduction has its own case
        apply_action(Action::OpenHistory, &mut s, &mut m);
        assert!(!s.overlays.history_prompt, "no prompt when the setting is already on");
        let told = s.transcript.last().cloned().unwrap_or_default();
        // The line carries no marker of its own — on screen the mark in the gutter
        // is what identifies it, and the kind tag is what `/filter` and the
        // exporter separate on. Both are `assist_voice`'s business; naming the
        // variant here would trip its one-producer guard, which is the guard doing
        // its job.
        assert!(told.contains("rewind"), "the player is told why nothing opened: {told:?}");

        // History present → open the replay, whatever the setting says.
        let mut s = AppState::default();
        s.config.record_turn_history = false;
        for t in 1..=3u32 {
            crate::history::record_turn(&mut s.history, t, "n", vec![t as u8], &m, false, "");
        }
        apply_action(Action::OpenHistory, &mut s, &mut m);
        assert!(!s.overlays.history_prompt, "nothing to offer when there is history to show");
        assert!(s.overlays.replay.is_some(), "the replay opens, seeded at the last turn");
    }

    #[test]
    fn open_saves_clears_hotkey_dialog() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.overlays.hotkey_dialog = true;
        apply_action(Action::OpenSaves, &mut s, &mut m);
        assert!(!s.overlays.hotkey_dialog, "OpenSaves should clear the hotkey dialog");
    }

    // ── is_direct as sole direct-vs-prefix determiner ─────────────────────────

    /// Promoting a command via config makes it reachable directly (dialog closed).
    #[test]
    fn direct_config_promotes_retidy_to_direct() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: Some(vec!["tidy-map".into()]),
            group: vec![HotkeyGroupConfig {
                title: "Layout".into(),
                commands: vec!["tidy-map".into()],
            }],
        };
        let (layout, _) = crate::keymap::HotkeyLayout::resolve(&cfg);
        let mut s = AppState::default();
        s.hotkeys = layout;
        s.focus = Focus::Map;
        // tidy-map has no default keymap binding now (leader-only command), so a
        // user promoting it to direct via config would also bind a key for it.
        s.keymap.bindings.push((
            crate::keymap::KeySpec { code: KeyCode::Char('t'), ctrl: true, shift: false, alt: false },
            "tidy-map".to_string(),
            crate::keymap::Context::Global,
        ));
        // With dialog closed: tidy-map is now direct → fires.
        assert!(
            matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::Retidy),
            "promoted retidy should fire directly (dialog closed)"
        );
    }

    /// Retidy has no default key at all now, and `t` is a free letter.
    ///
    /// It was dialog-only on the authored letter `t` until the Layout group was
    /// removed from the leader panel: the layout re-tidies itself continuously, so
    /// a by-hand pass was not earning a heading of its own. `/tidy-map` still runs
    /// it. This case exists to catch `t` being handed to something else without
    /// anyone noticing it used to mean this.
    #[test]
    fn retidy_has_no_default_key_and_t_is_free() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // Closed dialog: Ctrl+T returns None.
        assert!(
            matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::None),
            "retidy should NOT fire directly with default layout (dialog closed)"
        );
        s.overlays.hotkey_dialog = true;
        // Ctrl-combos close the dialog rather than firing, as they always did.
        assert!(
            matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::CloseHotkeyDialog),
            "Ctrl-combos close the hotkey dialog rather than firing"
        );
        // And a bare `t` now fires nothing: no group authors that letter.
        assert!(
            !matches!(key_to_action(&s, key(KeyCode::Char('t'))), Action::Retidy),
            "'t' must not still fire Retidy — the Layout group that authored it is gone"
        );
    }

    // ── mouse_to_action tests ─────────────────────────────────────────────────

    fn mouse_event(
        kind: crossterm::event::MouseEventKind,
        col: u16,
        row: u16,
        modifiers: KeyModifiers,
    ) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent { kind, column: col, row, modifiers }
    }

    fn map_rect() -> ratatui::layout::Rect {
        ratatui::layout::Rect::new(0, 0, 80, 40)
    }

    fn story_rect() -> ratatui::layout::Rect {
        ratatui::layout::Rect::new(80, 0, 40, 40)
    }

    /// Build a room_rects slice for a single room at a given cell using Compact zoom.
    fn room_rects_for_compact(id: u16, cell: (i32, i32), area: ratatui::layout::Rect) -> Vec<(mapper::graph::RoomId, ratatui::layout::Rect)> {
        use crate::state::{AppState, Zoom};
        use crate::render::map::room_screen_rects;
        use mapper::graph::MapGraph;
        use mapper::render::render_layer;
        use mapper::layer::MAIN_LAYER;

        let mut g = MapGraph::new();
        g.upsert_room(id, "Room".into());
        g.set_pos(id, cell);

        let mut s = AppState::default();
        s.zoom = Zoom::Compact;
        s.scroll = (0, 0);

        let rm = render_layer(&g, MAIN_LAYER);
        room_screen_rects(&rm, &s, area)
    }

    #[test]
    fn mouse_wheel_invert_swaps_story_scroll_direction() {
        use crossterm::event::MouseEventKind;

        let mut s = AppState::default();
        // Default (conventional): wheel up scrolls up into older text (+1).
        let m = mouse_event(MouseEventKind::ScrollUp, 90, 10, KeyModifiers::NONE);
        assert!(matches!(
            mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None),
            Action::TranscriptScroll(1)
        ));
        // Inverted: wheel up scrolls the other way.
        s.config.mouse_wheel_invert = true;
        let m2 = mouse_event(MouseEventKind::ScrollUp, 90, 10, KeyModifiers::NONE);
        assert!(matches!(
            mouse_to_action(&s, m2, map_rect(), story_rect(), &[], &None),
            Action::TranscriptScroll(-1)
        ));
    }

    // ── Mouse-wheel modal precedence tests ─────────────────────────────────────
    //
    // When a scrollable overlay is open, the wheel must drive THAT surface's
    // vertical scrolling (one row per tick) ahead of the underlying map/story. A
    // list modal resolves to the shared `ListWheel` (SQ-0831 — the wheel scrolls
    // the list, it is NOT the modal's Up/Down nav action); the replay overlay,
    // a stepper rather than a list, keeps `ReplayStep`. A wheel position over
    // the MAP area is used so the same events also exercise precedence over map
    // pan/zoom.

    /// One save-list entry, for the wheel/scroll tests — only its presence in
    /// the list matters, so every field but the name is a placeholder.
    fn dummy_save(name: &str) -> crate::persist_files::SaveInfo {
        crate::persist_files::SaveInfo {
            path: std::path::PathBuf::from(format!("/tmp/{name}.lanthorn")),
            name: name.into(),
            turns: 0,
            saved_at: String::new(),
            location: None,
            score: None,
            is_default: false,
            trigger: crate::archive::SaveTrigger::HostState,
        }
    }

    fn wheel_up() -> crossterm::event::MouseEvent {
        // Position (10, 10) is inside map_rect (0,0,80,40).
        mouse_event(crossterm::event::MouseEventKind::ScrollUp, 10, 10, KeyModifiers::NONE)
    }
    fn wheel_down() -> crossterm::event::MouseEvent {
        mouse_event(crossterm::event::MouseEventKind::ScrollDown, 10, 10, KeyModifiers::NONE)
    }

    #[test]
    fn wheel_drives_saves_selection() {
        use crate::state::SavesState;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState { entries: Vec::new(), scroll: Default::default() });
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::ListWheel(-1)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::ListWheel(1)
        ));
    }

    /// The config screen is the other modal that shares the saves list's wheel
    /// precedence slot; pinned so the routing above can't lose one of the two.
    #[test]
    fn wheel_drives_the_config_screen() {
        let mut s = AppState::default();
        apply_action(Action::OpenConfig, &mut s, &mut Mapper::default());
        assert!(s.overlays.config_screen.is_some(), "sanity: the screen opened");
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::ListWheel(-1)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::ListWheel(1)
        ));
    }

    #[test]
    fn wheel_drives_replay_step() {
        use crate::state::ReplayState;
        let mut s = AppState::default();
        s.overlays.replay = Some(ReplayState::new(0));
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::ReplayStep(-1)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::ReplayStep(1)
        ));
    }

    #[test]
    fn wheel_drives_file_browser_selection() {
        use crate::state::{FbMode, FileBrowserState};
        let mut s = AppState::default();
        s.overlays.file_browser = Some(FileBrowserState::build(
            std::env::temp_dir(),
            FbMode::PickFile));
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::ListWheel(-1)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::ListWheel(1)
        ));
    }

    /// The band takes the wheel by HIT RECT (in `main::band_mouse_action`), not
    /// by "an overlay is open" — it is a dock, so a wheel event outside its rect
    /// resolves exactly as it does with the band closed (SQ-0664). Every other
    /// list modal short-circuits `mouse_to_action` the moment it is open; the
    /// band deliberately does not appear there at all.
    #[test]
    fn wheel_outside_the_band_resolves_as_if_it_were_closed() {
        let mut s = AppState::default();
        let closed = mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None);
        open_band(&mut s);
        let open = mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None);
        assert_eq!(open, closed, "the band does not intercept wheels outside its rect");
        assert_ne!(closed, Action::None, "sanity: the wheel does something out there");
    }

    #[test]
    fn wheel_modal_precedence_beats_open_dialog_chrome() {
        // Saves open WITH a dialog present: the wheel must still drive the saves
        // list (not be swallowed by the dialog chrome block).
        use crate::state::SavesState;
        use crate::render::dialog::DialogRects;
        use ratatui::layout::Rect;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState { entries: Vec::new(), scroll: Default::default() });
        let dialog = Some(DialogRects {
            area: Rect::new(10, 5, 40, 15),
            content: Rect::new(11, 7, 38, 10),
            close: Some(Rect::new(48, 5, 1, 1)),
            buttons: Vec::new(),
            field: None,
        });
        // Wheel over the map area, with the dialog open.
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &dialog),
            Action::ListWheel(-1)
        ));
    }

    #[test]
    fn wheel_invert_swaps_modal_nav_direction() {
        // Representative modal (saves): with mouse_wheel_invert set, ScrollUp maps
        // to the DOWN action and ScrollDown to the UP action.
        use crate::state::SavesState;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState { entries: Vec::new(), scroll: Default::default() });
        s.config.mouse_wheel_invert = true;
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::ListWheel(1)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::ListWheel(-1)
        ));
    }

    /// The invert preference is resolved in exactly one place (`wheel_delta`)
    /// and must never be applied twice for one event: a double inversion is
    /// invisible with the setting OFF and silently cancels itself with it ON.
    /// Pinned end-to-end, from the raw event through to the list that moved —
    /// `mouse_to_action` maps `kind` once and then calls `wheel_delta` with
    /// `invert: false`, which only a behavioural test can hold in place.
    #[test]
    fn wheel_invert_is_applied_exactly_once_end_to_end() {
        use crate::state::SavesState;
        let entries: Vec<_> = (0..40).map(|i| dummy_save(&format!("s{i}"))).collect();
        let mut settled = Vec::new();
        for invert in [false, true] {
            let mut s = AppState::default();
            s.config.animation.enabled = false;
            s.config.mouse_wheel_invert = invert;
            s.modal_list_viewport = 5;
            s.overlays.saves = Some(SavesState { entries: entries.clone(), scroll: Default::default() });
            // Start mid-list so both directions have room to move.
            apply_action(Action::SavesNav(20), &mut s, &mut Mapper::default());
            let base = s.overlays.saves.as_ref().unwrap().scroll.target_offset();
            let act = mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None);
            apply_action(act, &mut s, &mut Mapper::default());
            let after = s.overlays.saves.as_ref().unwrap().scroll.target_offset();
            settled.push(after as i64 - base as i64);
        }
        assert_eq!(settled, vec![1, -1], "one ScrollDown scrolls +1 row, or -1 inverted — never 0");
    }

    /// SQ-0831, the whole point: a notch moves the LIST, not the cursor. The
    /// cursor only ever moves to stay inside the window it would otherwise be
    /// scrolled out of — the originally reported symptom was the wheel stepping
    /// the selection while the viewport chased it.
    #[test]
    fn wheel_scrolls_the_saves_list_and_pins_the_cursor_to_the_window() {
        use crate::state::SavesState;
        let mut s = AppState::default();
        s.config.animation.enabled = false;
        s.modal_list_viewport = 5;
        s.overlays.saves = Some(SavesState {
            entries: (0..40).map(|i| dummy_save(&format!("s{i}"))).collect(),
            scroll: Default::default(),
        });
        let mut m = Mapper::default();
        let sel = |s: &AppState| s.overlays.saves.as_ref().unwrap().scroll.selected;
        let off = |s: &AppState| s.overlays.saves.as_ref().unwrap().scroll.target_offset();

        // Park the cursor in the middle of the window, then scroll: the list
        // moves under a cursor that stays exactly where it is.
        apply_action(Action::SavesNav(2), &mut s, &mut m);
        assert_eq!((sel(&s), off(&s)), (2, 0));
        apply_action(Action::ListWheel(1), &mut s, &mut m);
        assert_eq!(off(&s), 1, "the list scrolled one row");
        assert_eq!(sel(&s), 2, "…and the cursor did NOT move with it");

        // Keep scrolling and the cursor eventually rides the window's top edge
        // rather than being scrolled off the screen.
        apply_action(Action::ListWheel(1), &mut s, &mut m);
        apply_action(Action::ListWheel(1), &mut s, &mut m);
        assert_eq!((off(&s), sel(&s)), (3, 3), "cursor pinned to the first visible row");

        // The far end: the offset stops with the last entry on the bottom row.
        for _ in 0..100 {
            apply_action(Action::ListWheel(1), &mut s, &mut m);
        }
        assert_eq!(off(&s), 35, "40 entries, 5 rows → the last window starts at 35");
        assert_eq!(sel(&s), 35);
        for _ in 0..100 {
            apply_action(Action::ListWheel(-1), &mut s, &mut m);
        }
        assert_eq!((off(&s), sel(&s)), (0, 4), "…and the bottom row at the top of the list");
    }

    /// A list shorter than its window has nothing to scroll — and the wheel must
    /// not move the cursor as a consolation prize.
    #[test]
    fn wheel_on_a_list_shorter_than_the_window_does_nothing_at_all() {
        use crate::state::SavesState;
        let mut s = AppState::default();
        s.config.animation.enabled = false;
        s.modal_list_viewport = 10;
        s.overlays.saves = Some(SavesState {
            entries: (0..3).map(|i| dummy_save(&format!("s{i}"))).collect(),
            scroll: Default::default(),
        });
        let mut m = Mapper::default();
        apply_action(Action::SavesNav(1), &mut s, &mut m);
        for d in [1, 1, -1, -1] {
            apply_action(Action::ListWheel(d), &mut s, &mut m);
            let sc = &s.overlays.saves.as_ref().unwrap().scroll;
            assert_eq!((sc.target_offset(), sc.selected), (0, 1), "nothing to scroll, cursor untouched");
        }
    }

    /// The same rule reaches the file browser and the config screen through the
    /// one `ListWheel` arm — no per-modal wheel behaviour to drift apart.
    #[test]
    fn wheel_scrolls_the_file_browser_without_moving_its_cursor() {
        use crate::state::{FbMode, FileBrowserState};
        let mut s = AppState::default();
        s.config.animation.enabled = false;
        s.modal_list_viewport = 2;
        let mut fb = FileBrowserState::build(std::env::temp_dir(), FbMode::PickFile);
        // A synthetic, long-enough list: the temp dir's real contents are not
        // something a test may assume anything about.
        fb.entries = (0..20)
            .map(|i| crate::state::FbEntry { name: format!("e{i}"), is_dir: false })
            .collect();
        s.overlays.file_browser = Some(fb);
        let mut m = Mapper::default();
        apply_action(Action::ListWheel(1), &mut s, &mut m);
        let sc = &s.overlays.file_browser.as_ref().unwrap().scroll;
        assert_eq!(sc.target_offset(), 1, "the browser list scrolled");
        assert_eq!(sc.selected, 1, "cursor pinned to the top of the window, not stepped past it");
    }

    #[test]
    fn wheel_scrolls_the_config_screen_without_moving_its_cursor() {
        let mut s = AppState::default();
        s.config.animation.enabled = false;
        s.modal_list_viewport = 4;
        let mut m = Mapper::default();
        apply_action(Action::OpenConfig, &mut s, &mut m);
        apply_action(Action::ConfigNav(2), &mut s, &mut m);
        assert_eq!(s.overlays.config_screen.as_ref().unwrap().scroll.selected, 2);
        apply_action(Action::ListWheel(1), &mut s, &mut m);
        let sc = &s.overlays.config_screen.as_ref().unwrap().scroll;
        assert_eq!(sc.target_offset(), 1, "the settings list scrolled");
        assert_eq!(sc.selected, 2, "…under a cursor that stayed put");
    }

    /// SQ-0692: a left-click on a room used to open a floating Room Info popup.
    /// It now PINS the room dock to that room — opening the dock if it was closed
    /// — which is the same gesture with a panel that does not cover the map.
    #[test]
    fn left_down_on_room_cell_pins_the_dock_in_info_view() {
        use crossterm::event::MouseEventKind;
        use crate::state::{RoomDockView, Zoom};

        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // step = (12, 5)
        s.scroll = (0, 0);
        assert!(!s.room_dock.open, "the dock starts closed");

        // Room 1 at cell (0,0). Build room_rects using render pipeline.
        let rects = room_rects_for_compact(1, (0, 0), map_rect());

        // Click at (0,0) which is inside the Compact box (8x3).
        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 0, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &rects, &None);
        assert!(
            matches!(action, Action::PinRoomDock(1, RoomDockView::Info)),
            "left-down on a room with the dock CLOSED opens it pinned in Info, got {:?}", action
        );

        // Applying it opens the dock, pinned.
        apply_action(action, &mut s, &mut Mapper::default());
        assert!(s.room_dock.open);
        assert_eq!(s.selected_room, Some(1));

        // With the dock already open, the same click on the SAME room unpins.
        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 0, KeyModifiers::NONE);
        assert!(
            matches!(mouse_to_action(&s, m, map_rect(), story_rect(), &rects, &None), Action::UnpinRoomDock),
            "a click on the already-pinned room unpins"
        );
    }

    #[test]
    fn right_down_on_room_cell_pins_the_dock_in_diagnostics_view() {
        use crossterm::event::MouseEventKind;
        use crate::state::{RoomDockView, Zoom};

        let mut s = AppState::default();
        s.zoom = Zoom::Compact;
        s.scroll = (0, 0);

        let rects = room_rects_for_compact(2, (0, 0), map_rect());

        let m = mouse_event(MouseEventKind::Down(MouseButton::Right), 0, 0, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &rects, &None);
        assert!(
            matches!(action, Action::PinRoomDock(2, RoomDockView::Diagnostics)),
            "right-down on a room pins the dock in Diagnostics, got {:?}", action
        );
        apply_action(action, &mut s, &mut Mapper::default());
        assert_eq!(s.room_dock_view, RoomDockView::Diagnostics);

        // Right-clicking the same room again — pinned AND already diagnostics — unpins.
        let m = mouse_event(MouseEventKind::Down(MouseButton::Right), 0, 0, KeyModifiers::NONE);
        assert!(
            matches!(mouse_to_action(&s, m, map_rect(), story_rect(), &rects, &None), Action::UnpinRoomDock),
            "a right-click on the pinned room already showing diagnostics unpins"
        );

        // …but a LEFT click there re-points it to Info rather than unpinning: the
        // gesture still has somewhere to take you.
        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 0, KeyModifiers::NONE);
        assert!(
            matches!(mouse_to_action(&s, m, map_rect(), story_rect(), &rects, &None), Action::UnpinRoomDock),
            "left-click on the pinned room unpins regardless of view"
        );
    }

    // ── Command history Up/Down (feature D) ────────────────────────────────────

    #[test]
    fn plain_up_down_recall_history_in_game_focus() {
        let s = AppState::default(); // Game focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Up)), Action::HistoryPrev));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::HistoryNext));
    }

    #[test]
    fn shift_up_down_still_pan_in_game_focus() {
        let s = AppState::default(); // Game focus
        assert!(matches!(key_to_action(&s, shift(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
    }

    #[test]
    fn history_actions_apply_through_apply_action() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.command_history = vec!["look".into(), "inventory".into()];
        s.input = "dr".into();
        apply_action(Action::HistoryPrev, &mut s, &mut m);
        assert_eq!(s.input.value, "inventory");
        apply_action(Action::HistoryNext, &mut s, &mut m);
        assert_eq!(s.input.value, "dr"); // draft restored
    }

    // ── PageUp/PageDown transcript paging (feature C) ──────────────────────────

    #[test]
    fn page_scroll_toward_older_advances_by_page_clamped() {
        // viewport 20 rows → page = 19. From 0 toward older (dir > 0): 0 + 19 = 19.
        assert_eq!(page_scroll(0, 1, 20, 100), 19);
        // Next page: 19 + 19 = 38.
        assert_eq!(page_scroll(19, 1, 20, 100), 38);
        // Clamped to max_scroll.
        assert_eq!(page_scroll(90, 1, 20, 100), 100);
        assert_eq!(page_scroll(100, 1, 20, 100), 100);
    }

    #[test]
    fn page_scroll_toward_newer_recedes_by_page_to_zero() {
        // dir < 0 moves toward newer (smaller offset), saturating at 0.
        assert_eq!(page_scroll(38, -1, 20, 100), 19);
        assert_eq!(page_scroll(19, -1, 20, 100), 0);
        assert_eq!(page_scroll(5, -1, 20, 100), 0);
    }

    #[test]
    fn page_scroll_tiny_viewport_steps_at_least_one() {
        // viewport of 0 or 1 → page floors at 1 line so paging still progresses.
        assert_eq!(page_scroll(0, 1, 1, 100), 1);
        assert_eq!(page_scroll(0, 1, 0, 100), 1);
    }

    #[test]
    fn wheel_delta_maps_and_inverts_once() {
        use crossterm::event::MouseEventKind::*;
        assert_eq!(wheel_delta(ScrollUp, false), Some(-1));
        assert_eq!(wheel_delta(ScrollDown, false), Some(1));
        assert_eq!(wheel_delta(ScrollUp, true), Some(1));
        assert_eq!(wheel_delta(ScrollDown, true), Some(-1));
        assert_eq!(wheel_delta(Moved, false), None);
    }

    #[test]
    fn page_up_down_do_not_zoom() {
        // Regression guard: PageUp/PageDown must not produce zoom actions.
        let s = AppState::default();
        let up = key_to_action(&s, key(KeyCode::PageUp));
        let dn = key_to_action(&s, key(KeyCode::PageDown));
        assert!(!matches!(up, Action::ZoomIn | Action::ZoomOut));
        assert!(!matches!(dn, Action::ZoomIn | Action::ZoomOut));
        assert!(matches!(up, Action::TranscriptScrollPage(1)));
        assert!(matches!(dn, Action::TranscriptScrollPage(-1)));
    }

    // ── Ctrl-D/Ctrl-U half-page transcript scrolling (SQ-1228) ─────────────────

    #[test]
    fn half_page_scroll_steps_by_floor_half_viewport() {
        // viewport 20 rows → half page = 10 (floor(20/2), no overlap).
        assert_eq!(half_page_scroll(0, 1, 20, 100), 10);
        assert_eq!(half_page_scroll(10, 1, 20, 100), 20);
        // dir < 0 moves toward newer (smaller offset), saturating at 0.
        assert_eq!(half_page_scroll(10, -1, 20, 100), 0);
        // Clamped to max_scroll.
        assert_eq!(half_page_scroll(95, 1, 20, 100), 100);
    }

    #[test]
    fn half_page_scroll_odd_viewport_floors_and_minimum_is_one() {
        // floor(9/2) = 4.
        assert_eq!(half_page_scroll(0, 1, 9, 100), 4);
        // viewport of 0 or 1 still steps by at least 1.
        assert_eq!(half_page_scroll(0, 1, 1, 100), 1);
        assert_eq!(half_page_scroll(0, 1, 0, 100), 1);
    }

    #[test]
    fn ctrl_d_half_pages_the_transcript_in_game_focus() {
        let s = AppState::default(); // focus = Game
        let dn = key_to_action(&s, ctrl(KeyCode::Char('d')));
        assert!(!matches!(dn, Action::ZoomIn | Action::ZoomOut));
        assert!(matches!(dn, Action::TranscriptScrollHalfPage(-1)));
    }

    /// SQ-1228: Ctrl-U is the vim half-page-up convention, but at the story
    /// prompt Ctrl-U also means "delete to start of line" (SQ-0447's readline
    /// shortcut, step 6.7). The two are disambiguated by whether the input
    /// line has anything to delete: empty → half-page up.
    #[test]
    fn ctrl_u_half_pages_up_when_the_input_line_is_empty() {
        let s = AppState::default(); // focus = Game, input line empty
        assert!(s.input.is_empty());
        let up = key_to_action(&s, ctrl(KeyCode::Char('u')));
        assert!(matches!(up, Action::TranscriptScrollHalfPage(1)));
    }

    /// SQ-1228: with text on the input line, Ctrl-U keeps its readline
    /// meaning of DeleteToStart — that convention wins whenever there's
    /// something to delete.
    #[test]
    fn ctrl_u_keeps_its_readline_meaning_when_input_has_text() {
        let mut s = AppState::default(); // focus = Game, not char_mode/event_wait
        s.input = crate::text_field::TextField::new("look");
        let up = key_to_action(&s, ctrl(KeyCode::Char('u')));
        assert!(matches!(up, Action::DeleteToStart));
    }

    /// SQ-1228: outside Game focus, Ctrl-U isn't bound to either meaning here
    /// — it falls through to whatever the focus's own handling does with it.
    #[test]
    fn ctrl_u_outside_game_focus_is_not_half_paged() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        let up = key_to_action(&s, ctrl(KeyCode::Char('u')));
        assert!(!matches!(up, Action::TranscriptScrollHalfPage(_)));
        assert!(!matches!(up, Action::DeleteToStart));
    }

    /// A state whose keymap is exactly what the user typed into `config.toml`
    /// (mirrors `window_dump_bound_key.rs`'s `state_bound` helper).
    fn state_with_ctrl_binding(key: &str, cmd: &str) -> AppState {
        let mut cfg = crate::config::KeymapConfig::default();
        cfg.global.insert(key.to_string(), cmd.to_string());
        let (keymap, warnings) = crate::keymap::KeyMap::resolve(&cfg);
        assert!(warnings.is_empty(), "{warnings:?}");
        let mut s = AppState::default(); // focus = Game
        s.keymap = keymap;
        s
    }

    /// SQ-1228 (CI fix): the transcript half-page keys are DEFAULTS, not
    /// hardwires — a user's own Ctrl+D binding must win. `window_dump_bound_key`
    /// (SQ-0759) already relies on this for `dump-windows`; this pins the same
    /// contract at the unit level. FALSIFY: reverting the keymap check in step
    /// 6.8 makes this resolve to TranscriptScrollHalfPage(-1) instead.
    #[test]
    fn ctrl_d_bound_in_the_keymap_dispatches_the_command_not_half_page() {
        let s = state_with_ctrl_binding("ctrl+d", "dump-windows");
        assert!(s.hotkeys.is_direct_name("dump-windows"));
        match key_to_command(&s, ctrl(KeyCode::Char('d'))) {
            KeyResolve::Command(cmd, _) => assert_eq!(cmd, "dump-windows"),
            other => panic!("a bound Ctrl+D must dispatch the command, got {other:?}"),
        }
    }

    /// Same contract for the empty-prompt half-page-up default: a user's own
    /// Ctrl+U binding wins over TranscriptScrollHalfPage(1) too. FALSIFY:
    /// reverting the keymap check makes this resolve to the half-page action.
    #[test]
    fn ctrl_u_bound_in_the_keymap_dispatches_the_command_when_prompt_is_empty() {
        let s = state_with_ctrl_binding("ctrl+u", "dump-windows");
        assert!(s.input.is_empty());
        match key_to_command(&s, ctrl(KeyCode::Char('u'))) {
            KeyResolve::Command(cmd, _) => assert_eq!(cmd, "dump-windows"),
            other => panic!("a bound Ctrl+U must dispatch the command, got {other:?}"),
        }
    }

    /// SQ-0692: an empty-space click used to close the popup. It now UNPINS — the
    /// dock stays up and goes back to following the player, which is the state you
    /// wanted when you clicked away from a room in the first place.
    #[test]
    fn left_down_on_gutter_unpins_without_taking_focus() {
        use crossterm::event::MouseEventKind;
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // step = (12, 5)
        s.scroll = (0, 0);
        // Room is at cell (0,0), box is 8 wide so cols 0..8 hit the room.
        // Click at col 50 misses the room entirely.
        let rects = room_rects_for_compact(1, (0, 0), map_rect());

        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 50, 0, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &rects, &None);
        assert!(
            matches!(action, Action::UnpinRoomDock),
            "left-down on the map gutter unpins, and must not hand the keyboard to the map (SQ-0599), got {:?}", action
        );
    }

    #[test]
    fn left_down_in_story_starts_selection_and_activates_game() {
        use crossterm::event::MouseEventKind;
        let mut s = AppState::default();
        // col 85 is inside story_rect (x=80..120).
        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 85, 5, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(
            matches!(action, Action::StartSelection(85, 5)),
            "left-down in story pane should start a selection, got {:?}", action
        );
        // Applying it activates the game pane and sets the selection anchor.
        // Publish geometry so screen cells map to absolute wrapped-row Points.
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: story_rect(), first_abs_row: 0, total_rows: 100,
        }));
        apply_action(action, &mut s, &mut Mapper::default());
        assert_eq!(s.focus, Focus::Game);
        // col 85 → col 5 within the story band (x=80); row 5 → abs row 5.
        assert_eq!(s.selection.map(|sel| sel.anchor), Some(crate::clipboard::Point { row: 5, col: 5 }));
    }

    #[test]
    fn left_drag_then_up_extends_and_ends_selection() {
        use crossterm::event::MouseEventKind;
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: story_rect(), first_abs_row: 0, total_rows: 100,
        }));
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());

        let drag = mouse_event(MouseEventKind::Drag(MouseButton::Left), 90, 7, KeyModifiers::NONE);
        let a = mouse_to_action(&s, drag, map_rect(), story_rect(), &[], &None);
        assert!(matches!(a, Action::ExtendSelection(90, 7)));
        apply_action(a, &mut s, &mut Mapper::default());
        // col 90 → col 10; row 7 (interior) → abs row 7.
        assert_eq!(s.selection.map(|sel| sel.head), Some(crate::clipboard::Point { row: 7, col: 10 }));

        let up = mouse_event(MouseEventKind::Up(MouseButton::Left), 90, 7, KeyModifiers::NONE);
        assert!(matches!(
            mouse_to_action(&s, up, map_rect(), story_rect(), &[], &None),
            Action::EndSelection
        ));
    }

    #[test]
    fn screen_to_point_maps_row_and_col() {
        let g = crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(0, 0, 20, 10), first_abs_row: 5, total_rows: 100,
        };
        assert_eq!(screen_to_point(g, 3, 2), Some(crate::clipboard::Point { row: 7, col: 3 }));
        // row past the bottom clamps to first_abs_row + height - 1.
        assert_eq!(screen_to_point(g, 3, 99), Some(crate::clipboard::Point { row: 14, col: 3 }));
        // col past the right clamps to width - 1.
        assert_eq!(screen_to_point(g, 99, 2), Some(crate::clipboard::Point { row: 7, col: 19 }));
    }

    #[test]
    fn screen_to_point_clamps_to_total_rows() {
        // total_rows smaller than first_abs_row + dy: clamp to total_rows - 1.
        let g = crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(0, 0, 20, 10), first_abs_row: 5, total_rows: 8,
        };
        assert_eq!(screen_to_point(g, 0, 9), Some(crate::clipboard::Point { row: 7, col: 0 }));
    }

    #[test]
    fn start_selection_sets_anchor_from_geom() {
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(80, 0, 40, 40), first_abs_row: 10, total_rows: 100,
        }));
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());
        let sel = s.selection.expect("selection set");
        let expected = crate::clipboard::Point { row: 15, col: 5 };
        assert_eq!(sel.anchor, expected);
        assert_eq!(sel.head, expected);
    }

    #[test]
    fn extend_selection_at_bottom_edge_autoscrolls_and_grows_head() {
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(80, 0, 40, 20), first_abs_row: 30, total_rows: 100,
        }));
        s.transcript_scroll = 10; // mid-range (max = 100 - 20 = 80)
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());
        // Bottom edge row: area.bottom() - 1 = 19 → maps to abs row 30 + 19 = 49;
        // autoscroll then grows the head one more row → 50.
        apply_action(Action::ExtendSelection(90, 19), &mut s, &mut Mapper::default());
        assert_eq!(s.selection_edge, 1);
        assert_eq!(s.transcript_scroll, 9, "bottom edge scrolls toward newer (scroll -1)");
        assert_eq!(s.selection.unwrap().head.row, 50, "head grows downward past the edge row");
    }

    #[test]
    fn extend_selection_at_top_edge_autoscrolls_and_grows_head() {
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(80, 0, 40, 20), first_abs_row: 30, total_rows: 100,
        }));
        s.transcript_scroll = 10;
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());
        // Top edge row: area.y = 0 → maps to abs row 30; autoscroll grows the head
        // one row upward → 29.
        apply_action(Action::ExtendSelection(90, 0), &mut s, &mut Mapper::default());
        assert_eq!(s.selection_edge, -1);
        assert_eq!(s.transcript_scroll, 11, "top edge scrolls toward older (scroll +1)");
        assert_eq!(s.selection.unwrap().head.row, 29, "head grows upward past the edge row");
    }

    #[test]
    fn extend_selection_interior_sets_edge_zero_no_scroll() {
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(80, 0, 40, 20), first_abs_row: 30, total_rows: 100,
        }));
        s.transcript_scroll = 10;
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());
        apply_action(Action::ExtendSelection(90, 10), &mut s, &mut Mapper::default());
        assert_eq!(s.selection_edge, 0);
        assert_eq!(s.transcript_scroll, 10, "interior drag does not scroll");
    }

    #[test]
    fn extend_selection_without_a_selection_neither_tracks_nor_scrolls() {
        // SQ-0654: press-and-hold on a map room, then drag along the story pane's
        // shared boundary row. `mouse_to_action` maps ANY left-drag to
        // ExtendSelection, so this arm runs with `state.selection == None` — and
        // used to set an autoscroll edge and scroll the transcript anyway.
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(80, 0, 40, 20), first_abs_row: 30, total_rows: 100,
        }));
        s.transcript_scroll = 10;
        assert!(s.selection.is_none(), "no selection was ever started");

        // Bottom boundary row, then the top one.
        apply_action(Action::ExtendSelection(90, 19), &mut s, &mut Mapper::default());
        assert_eq!(s.selection_edge, 0, "no selection → no edge tracking");
        assert_eq!(s.transcript_scroll, 10, "no selection → no autoscroll");
        assert!(s.selection.is_none());

        apply_action(Action::ExtendSelection(90, 0), &mut s, &mut Mapper::default());
        assert_eq!(s.selection_edge, 0);
        assert_eq!(s.transcript_scroll, 10);

        // And the tick-driven step is inert too, even if an edge were left set.
        s.selection_edge = 1;
        apply_selection_autoscroll(&mut s);
        assert_eq!(s.transcript_scroll, 10, "autoscroll no-ops without a selection");
    }

    /// SQ-0692 flipped the second half of this test. `ActivatePane` used to CLOSE
    /// the room panel — a floating dialog that had to get out of the way — and
    /// must now leave the dock alone: the dock covers nothing, so a pane switch
    /// closing it just means it is never up when you want it.
    #[test]
    fn apply_activate_pane_sets_focus_and_leaves_the_room_dock_alone() {
        let mut s = AppState::default(); // starts Focus::Game
        let mut m = Mapper::default();

        s.open_room_dock(crate::state::RoomDockView::Diagnostics);
        s.selected_room = Some(1);

        // ActivatePane(Game) sets game focus and leaves the dock exactly as it was.
        apply_action(Action::ActivatePane(Focus::Game), &mut s, &mut m);
        assert_eq!(s.focus, Focus::Game, "ActivatePane(Game) must set focus to Game");
        assert!(s.room_dock.open, "ActivatePane must NOT close the room panel");
        assert_eq!(s.selected_room, Some(1), "…nor unpin it");
        assert_eq!(s.room_dock_view, crate::state::RoomDockView::Diagnostics, "…nor change its view");

        // ActivatePane(Map) sets map focus.
        apply_action(Action::ActivatePane(Focus::Map), &mut s, &mut m);
        assert_eq!(s.focus, Focus::Map, "ActivatePane(Map) must set focus to Map");
    }

    #[test]
    fn scroll_up_in_map_produces_pan_up() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollUp, 10, 10, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action, Action::Pan(0, -1)), "scroll up in map without modifier -> Pan(0,-1)");
    }

    #[test]
    fn scroll_down_in_map_produces_pan_down() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollDown, 10, 10, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action, Action::Pan(0, 1)), "scroll down in map without modifier -> Pan(0,1)");
    }

    #[test]
    fn scroll_up_with_shift_pans_left() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollUp, 10, 10, KeyModifiers::SHIFT);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action, Action::Pan(-1, 0)), "scroll up + Shift -> Pan(-1,0)");
    }

    #[test]
    fn scroll_up_with_ctrl_zooms_in() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollUp, 10, 10, KeyModifiers::CONTROL);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        // The wheel keeps the FINE step (SQ-0350): three notches to a visible change, so a fast
        // ctrl+scroll cannot skip past Compact. The keyboard's `+`/`-` move a whole step instead.
        assert!(matches!(action, Action::ZoomInFine), "scroll up + Ctrl -> ZoomInFine");
    }

    #[test]
    fn scroll_in_story_produces_transcript_scroll() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        // col 85 is inside story_rect (x=80..120).
        let m_up = mouse_event(MouseEventKind::ScrollUp, 85, 5, KeyModifiers::NONE);
        let action_up = mouse_to_action(&s, m_up, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action_up, Action::TranscriptScroll(1)), "scroll up in story -> TranscriptScroll(1) (older)");

        let m_dn = mouse_event(MouseEventKind::ScrollDown, 85, 5, KeyModifiers::NONE);
        let action_dn = mouse_to_action(&s, m_dn, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action_dn, Action::TranscriptScroll(-1)), "scroll down in story -> TranscriptScroll(-1) (newer)");
    }

    #[test]
    fn middle_down_produces_begin_drag_pan() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::Down(MouseButton::Middle), 20, 15, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action, Action::BeginDragPan(20, 15)), "middle-down -> BeginDragPan");
    }

    #[test]
    fn middle_drag_and_up_produce_drag_actions() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let drag = mouse_event(MouseEventKind::Drag(MouseButton::Middle), 25, 18, KeyModifiers::NONE);
        let up = mouse_event(MouseEventKind::Up(MouseButton::Middle), 25, 18, KeyModifiers::NONE);
        assert!(matches!(mouse_to_action(&s, drag, map_rect(), story_rect(), &[], &None), Action::DragPanTo(25, 18)));
        assert!(matches!(mouse_to_action(&s, up, map_rect(), story_rect(), &[], &None), Action::EndDragPan));
    }

    // ── Drag-pan accumulator tests ────────────────────────────────────────────

    #[test]
    fn drag_pan_accumulates_and_pans_at_step_boundary() {
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact;
        let mut m = Mapper::default();

        // Begin at (10, 10).
        apply_action(Action::BeginDragPan(10, 10), &mut s, &mut m);
        assert!(s.drag.is_some(), "drag state should be set after BeginDragPan");

        // New behavior: drag goes directly into char_pan at 1-char precision.
        // Grab-and-drag: drag 11 columns right → char_pan.0 = +11, scroll unchanged.
        apply_action(Action::DragPanTo(21, 10), &mut s, &mut m); // dx=11
        assert_eq!(s.char_pan.0, 11, "11-col drag right should set char_pan.0 to +11");
        assert_eq!(s.scroll, (0, 0), "scroll should not change during drag");

        // Drag 1 more column right: char_pan.0 = +12, scroll still unchanged.
        apply_action(Action::DragPanTo(22, 10), &mut s, &mut m); // dx=1
        assert_eq!(s.char_pan.0, 12, "additional 1-col drag should set char_pan.0 to +12");
        assert_eq!(s.scroll, (0, 0), "scroll must remain unchanged (char_pan handles it)");
    }

    #[test]
    fn drag_pan_sub_step_movement_does_not_pan() {
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Boxes;
        let mut m = Mapper::default();

        apply_action(Action::BeginDragPan(0, 0), &mut s, &mut m);
        // Move 5 cols right: goes into char_pan, scroll unchanged.
        apply_action(Action::DragPanTo(5, 0), &mut s, &mut m);
        assert_eq!(s.scroll, (0, 0), "scroll must not change; char_pan absorbs the delta");
        assert_eq!(s.char_pan.0, 5, "char_pan.0 should be +5 after 5-col drag right (grab)");
    }

    #[test]
    fn drag_pan_grab_and_drag_direction() {
        // Grab-and-drag: dragging LEFT moves content left → char_pan.0 negative.
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact;
        let mut m = Mapper::default();

        apply_action(Action::BeginDragPan(20, 0), &mut s, &mut m);
        // Drag left by 12 columns: dx = -12, char_pan.0 += dx = -12.
        apply_action(Action::DragPanTo(8, 0), &mut s, &mut m);
        assert_eq!(s.char_pan.0, -12, "dragging left moves content left (grab): char_pan.0 = -12");
        assert_eq!(s.scroll.0, 0, "scroll must not change; char_pan handles the delta");
    }

    #[test]
    fn end_drag_pan_clears_state() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::BeginDragPan(0, 0), &mut s, &mut m);
        assert!(s.drag.is_some());
        apply_action(Action::EndDragPan, &mut s, &mut m);
        assert!(s.drag.is_none(), "EndDragPan should clear drag state");
    }

    #[test]
    fn pinning_the_dock_keeps_story_focus_so_you_can_keep_typing() {
        // Opening a room panel used to hand the keyboard to the map, which made
        // every letter a map command and dimmed the story pane — so you could not
        // type while reading it. The room still selects; focus does not move.
        use crate::state::RoomDockView;
        let mut s = AppState::default(); // starts as Focus::Game
        assert_eq!(s.focus, Focus::Game);
        let mut m = Mapper::default();
        apply_action(Action::PinRoomDock(1, RoomDockView::Info), &mut s, &mut m);
        assert_eq!(s.focus, Focus::Game, "pinning the dock must NOT steal keyboard focus");
        assert_eq!(s.selected_room, Some(1), "the room is still selected for rendering");
        // And a letter reaches the story prompt rather than the map.
        assert!(
            matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::InputChar('n')),
            "with the dock open a letter must type, not drive the map"
        );
    }

    #[test]
    fn pinning_the_diagnostics_view_keeps_story_focus() {
        use crate::state::RoomDockView;
        let mut s = AppState::default(); // starts as Focus::Game
        let mut m = Mapper::default();
        apply_action(Action::PinRoomDock(2, RoomDockView::Diagnostics), &mut s, &mut m);
        assert_eq!(s.focus, Focus::Game, "a right-click pin must NOT steal keyboard focus");
        assert_eq!(s.selected_room, Some(2), "the room is still selected for rendering");
    }

    // ── Leaf 1: ToggleMap ─────────────────────────────────────────────────────

    #[test]
    fn apply_action_toggle_map() {
        use crate::state::{AppState, Layout};
        let mut s = AppState::default(); // empty game_dir → no sidecar write
        let mut m = Mapper::default();
        assert!(matches!(s.layout, Layout::Split));
        apply_action(Action::ToggleMap, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::TranscriptFull));
        apply_action(Action::ToggleMap, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::Split));
    }

    #[test]
    fn toggle_map_persists_show_map_to_game_dir() {
        use crate::state::{AppState, Layout};
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let game_dir = std::env::temp_dir()
            .join(format!("bm-togglemap-{}-{}.save", std::process::id(), n));
        std::fs::create_dir_all(&game_dir).unwrap();

        let mut s = AppState::default();
        s.game_dir = game_dir.clone();
        let mut m = Mapper::default();

        // Hide the map → show_map = false persisted.
        apply_action(Action::ToggleMap, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::TranscriptFull));
        assert_eq!(crate::styles::read_per_game_show_map(&game_dir), Some(false));

        // Show it again → show_map = true persisted.
        apply_action(Action::ToggleMap, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::Split));
        assert_eq!(crate::styles::read_per_game_show_map(&game_dir), Some(true));
        let _ = std::fs::remove_dir_all(&game_dir);
    }

    /// SQ-1123: the band toggle is a border control now, and what a control
    /// switches it also remembers — in THIS game's sidecar, the same rule
    /// `toggle-map` has followed since SQ-0304.
    #[test]
    fn open_command_band_persists_its_state_to_game_dir() {
        use crate::state::AppState;
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let game_dir = std::env::temp_dir()
            .join(format!("bm-bandopen-{}-{}.save", std::process::id(), n));
        std::fs::create_dir_all(&game_dir).unwrap();

        let mut s = AppState::default();
        s.game_dir = game_dir.clone();
        let mut m = Mapper::default();

        apply_action(Action::OpenCommandBand, &mut s, &mut m);
        assert!(s.command_band_visible(), "it opens");
        assert_eq!(
            crate::styles::read_per_game_panel(&game_dir),
            Some(crate::state::SidePanel::Command),
        );

        apply_action(Action::OpenCommandBand, &mut s, &mut m);
        assert_eq!(
            crate::styles::read_per_game_panel(&game_dir), Some(crate::state::SidePanel::None),
            "closing is a choice too, and an explicit None is not an absence",
        );

        // …and the boot path does NOT write: a global `[command_panel] auto_open`
        // must not pin itself to whichever story you happened to launch, which is
        // the whole reason `open_command_band` exists beside the action.
        crate::styles::write_per_game_panel(&game_dir, None).unwrap();
        open_command_band(&mut s, &mut m, true);
        assert_eq!(
            crate::styles::read_per_game_panel(&game_dir), None,
            "startup's own open leaves the sidecar alone",
        );
        let _ = std::fs::remove_dir_all(&game_dir);
    }

    // ── SQ-1237: the three-state panel cycle ─────────────────────────────────

    fn cycle_panel_game_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("bm-cyclepanel-{tag}-{}-{}.save", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Command → Inventory → None → Command, driven entirely by
    /// `Action::CyclePanel` (what a click on the border control runs). Each
    /// step is asserted, not just the round trip, so a cycle that skips a state
    /// (e.g. Command → None directly) would fail here even though it returns to
    /// Command eventually.
    #[test]
    fn cycle_panel_visits_command_then_inventory_then_none_then_command() {
        use crate::state::{AppState, SidePanel};
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert_eq!(s.current_side_panel(), SidePanel::None, "starts closed");

        apply_action(Action::CyclePanel, &mut s, &mut m);
        assert_eq!(s.current_side_panel(), SidePanel::Command);

        apply_action(Action::CyclePanel, &mut s, &mut m);
        assert_eq!(s.current_side_panel(), SidePanel::Inventory);

        apply_action(Action::CyclePanel, &mut s, &mut m);
        assert_eq!(s.current_side_panel(), SidePanel::None);

        apply_action(Action::CyclePanel, &mut s, &mut m);
        assert_eq!(s.current_side_panel(), SidePanel::Command, "the cycle wraps");
    }

    /// Falsifies the mutual-exclusion rule: reverting `cycle_panel` to a version
    /// that does not close the panel it is leaving would show this test a
    /// command band still open once the cycle reaches Inventory — which is
    /// exactly what "the two are never open at once" means. Checked at every
    /// step, not just the one transition, since a bug could plausibly appear on
    /// either edge.
    #[test]
    fn the_two_panels_are_never_open_at_once() {
        use crate::state::AppState;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for _ in 0..6 {
            apply_action(Action::CyclePanel, &mut s, &mut m);
            // The band's TARGET (`band_dock.open`), not `command_band_visible()`
            // — the latter stays true through a close's slide-out by design
            // (the drawer's content persists so it can animate away, trimmed
            // only once `settle_command_band` runs on a later tick), which is
            // right for "should this still be drawn this frame" and wrong for
            // "did the cycle actually leave the command panel". The two panels
            // occupy different regions on screen anyway (the command panel
            // below the story pane, the inventory panel carved from the map
            // pane), so this is about state exclusivity, not a visual overlap.
            assert!(
                !(s.band_dock.open && s.show_inventory),
                "both panels open at once after a cycle step",
            );
        }
    }

    /// `Action::ToggleInventory` and `Action::OpenCommandBand` also close the
    /// OTHER panel when they open theirs — not only `cycle_panel` — since a
    /// player can reach either panel directly (leader key, slash command) as
    /// well as through the border control's cycle.
    #[test]
    fn opening_either_panel_directly_closes_the_other() {
        use crate::state::AppState;
        let mut s = AppState::default();
        let mut m = Mapper::default();

        apply_action(Action::OpenCommandBand, &mut s, &mut m);
        assert!(s.band_dock.open);
        apply_action(Action::ToggleInventory, &mut s, &mut m);
        assert!(s.show_inventory, "inventory opened");
        assert!(!s.band_dock.open, "…and closed the command panel");

        apply_action(Action::OpenCommandBand, &mut s, &mut m);
        assert!(s.band_dock.open, "command panel opened");
        assert!(!s.show_inventory, "…and closed the inventory panel");
    }

    /// The three-state value round-trips through the SAME per-game sidecar
    /// mechanism the command band's on/off state already used (SQ-1123) — no
    /// second persistence path was added for the inventory panel.
    #[test]
    fn cycle_panel_persists_the_new_state_to_game_dir() {
        use crate::state::{AppState, SidePanel};
        let game_dir = cycle_panel_game_dir("persist");
        let mut s = AppState::default();
        s.game_dir = game_dir.clone();
        let mut m = Mapper::default();

        apply_action(Action::CyclePanel, &mut s, &mut m);
        assert_eq!(crate::styles::read_per_game_panel(&game_dir), Some(SidePanel::Command));

        apply_action(Action::CyclePanel, &mut s, &mut m);
        assert_eq!(crate::styles::read_per_game_panel(&game_dir), Some(SidePanel::Inventory));

        apply_action(Action::CyclePanel, &mut s, &mut m);
        assert_eq!(crate::styles::read_per_game_panel(&game_dir), Some(SidePanel::None));

        let _ = std::fs::remove_dir_all(&game_dir);
    }

    /// Each of the three states draws its own glyph and its own tooltip line —
    /// falsified by reverting the border control to a plain two-way toggle,
    /// which would make the Inventory-state glyph equal the Command-state glyph
    /// (both would read `band_hide`) and the hint text would still say
    /// Command Panel for a panel that is actually the inventory one.
    #[test]
    fn each_panel_state_draws_its_own_glyph_and_tooltip() {
        use crate::render::controls::{controls_for, BorderControl};
        use crate::state::AppState;

        let find = |state: &AppState| {
            controls_for(state)
                .into_iter()
                .find(|v| v.id == BorderControl::VerbPanel)
                .expect("the panel-cycle control is always drawn")
        };

        let mut s = AppState::default();
        let mut m = Mapper::default();
        let none = find(&s);
        assert!(none.hint[0].to_lowercase().contains("closed"), "{:?}", none.hint);

        apply_action(Action::CyclePanel, &mut s, &mut m);
        let command = find(&s);
        assert!(command.hint[0].to_lowercase().contains("command panel"), "{:?}", command.hint);

        apply_action(Action::CyclePanel, &mut s, &mut m);
        let inventory = find(&s);
        assert!(inventory.hint[0].to_lowercase().contains("inventory panel"), "{:?}", inventory.hint);

        // Three states, three distinct glyphs — not merely three distinct hints
        // over the same shape.
        assert_ne!(none.glyph, command.glyph);
        assert_ne!(command.glyph, inventory.glyph);
        assert_ne!(none.glyph, inventory.glyph);
    }

    // ── Leaf 2: ResetGame opens the dialog ───────────────────────────────────

    #[test]
    fn reset_game_action_opens_reset_dialog() {
        use crate::state::AppState;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(!s.overlays.reset_dialog, "dialog must start closed");
        apply_action(Action::ResetGame, &mut s, &mut m);
        assert!(s.overlays.reset_dialog, "ResetGame must set reset_dialog = true");
        assert!(!s.overlays.reset_clear_map, "checkbox must start unchecked");
        assert!(s.overlays.text_entry.is_none(), "no text-entry dialog should be opened");
    }

    // ── reset-game is now leader-only; F5 has no default binding ──────────────
    // reset-game was demoted out of the always-active default keymap (SQ-0202):
    // it's reached only through the Ctrl+P leader panel now. This test pins that
    // F5 no longer resolves directly, and (still) that Action::ResetGame — however
    // it's triggered — opens the confirmation dialog rather than instant-wiping.
    #[test]
    fn f5_key_no_longer_bound_and_reset_game_still_opens_dialog() {
        use crate::state::AppState;
        let s = AppState::default();
        // (a) F5 has no default binding (reset-game is leader-only now).
        assert!(
            matches!(key_to_command(&s, key(KeyCode::F(5))), KeyResolve::None),
            "F5 must not resolve to a command by default"
        );
        // (b) The from_key Reset branch opens the dialog via Action::ResetGame.
        let mut s2 = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::ResetGame, &mut s2, &mut m);
        assert!(s2.overlays.reset_dialog, "reset-game must open the confirmation dialog, not instant-wipe");
    }

    // ── Leaf 2: minizork fixture reset test ───────────────────────────────────

    #[test]
    fn minizork_reset_restores_opening_room_and_clears_turns() {
        use crate::session::{apply_turn, GameSession, TurnResult};
        use zvm::current_location;

        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");

        // Build the initial session and seed the start room.
        let mut session = GameSession::new(story_bytes.clone(), true, false, None).expect("GameSession::new");
        let mut mapper = Mapper::default();
        let mut state = crate::state::AppState::default();

        let start_loc = current_location(&session.machine);
        let start_room_number = start_loc.as_ref().map(|s| s.number);
        if let Some(snap) = start_loc {
            let snap_number = snap.number;
            let seed_result = TurnResult {
                transcript: String::new(),
                transcript_runs: Vec::new(),
                location: Some(snap),
                quit: false,
                erase_lower: false,
                info: None,
                sounds: Vec::new(),
                glulx_sound_ops: Vec::new(),
                diagnostics: vec![],
                fault: None,
                location_method: None,
                pending_io: None,
                timed_out: false,
                pictures: Vec::new(),
                transcript_elems: Vec::new(),
                prose_retired: None,
            };
            apply_turn(&mut mapper, "", &seed_result, &mut Default::default());
            state.select_room(Some(snap_number as mapper::graph::RoomId));
        }
        let banner = session.take_transcript();
        state.push_transcript(&banner);

        // Simulate some game turns to advance state.
        let r1 = session.submit("look");
        state.push_transcript_runs(&r1.transcript, crate::state::TranscriptKind::Story, &r1.transcript_runs);
        state.turns = 5;

        // Rebuild session from story_bytes (what the reset flow does on confirm).
        let mut new_session = GameSession::new(story_bytes.clone(), true, false, None).expect("GameSession::new for reset");
        let new_start_loc = current_location(&new_session.machine);
        let new_room_number = new_start_loc.as_ref().map(|s| s.number);

        // Reset state fields exactly as the reset flow does.
        state.turns = 0;
        state.input.clear();
        state.suggestions.clear();
        state.suggestion_idx = 0;
        state.transcript.clear();
        state.clear_anchor = None;
        state.transcript_kinds.clear();
        state.transcript_runs.clear();
        state.transcript_scroll = 0;
        let new_banner = new_session.take_transcript();
        state.push_transcript(&new_banner);
        if let Some(snap) = new_start_loc {
            let snap_number = snap.number;
            let seed_result = TurnResult {
                transcript: String::new(),
                transcript_runs: Vec::new(),
                location: Some(snap),
                quit: false,
                erase_lower: false,
                info: None,
                sounds: Vec::new(),
                glulx_sound_ops: Vec::new(),
                diagnostics: vec![],
                fault: None,
                location_method: None,
                pending_io: None,
                timed_out: false,
                pictures: Vec::new(),
                transcript_elems: Vec::new(),
                prose_retired: None,
            };
            apply_turn(&mut mapper, "", &seed_result, &mut Default::default());
            state.select_room(Some(snap_number as mapper::graph::RoomId));
        }

        // Assert post-reset invariants.
        assert_eq!(state.turns, 0, "turn counter must be 0 after reset");
        assert_eq!(
            new_room_number, start_room_number,
            "post-reset current location must equal opening room"
        );
        // Mapper is kept (rooms are still in the graph).
        assert!(mapper.graph.rooms().count() > 0, "mapper must still have rooms after reset");
    }

    // ── Verb menu tests ───────────────────────────────────────────────────────

    // ── SQ-0664: the command band ─────────────────────────────────────────────

    /// Open the band with a known object model.
    fn open_band(state: &mut AppState) {
        let mut band = crate::state::CommandBandState::new(
            crate::render::command_band::default_verbs(),
            crate::render::command_band::default_quick(),
        );
        band.here = vec!["iron door".to_string(), "mailbox".to_string()];
        band.carried = vec!["brass key".to_string(), "lantern".to_string()];
        state.overlays.command_band = Some(band);
        state.band_dock.toggle_to(true, true);
    }

    /// Type `text` at the prompt one key at a time, through the real key
    /// routing — the only honest way to pin "typing always wins" (SQ-0676),
    /// since the whole question is what the band's intercept does with each
    /// keystroke before the story input ever sees it.
    fn type_text(state: &mut AppState, mapper: &mut Mapper, text: &str) {
        for c in text.chars() {
            let a = key_to_action(state, key(KeyCode::Char(c)));
            assert_eq!(a, Action::InputChar(c), "`{c}` must reach the story prompt");
            apply_action(a, state, mapper);
        }
    }

    fn band(state: &AppState) -> &crate::state::CommandBandState {
        state.overlays.command_band.as_ref().expect("band open")
    }

    /// Pick the row whose text is `text` in `col`, the way a click does.
    fn pick_text(state: &mut AppState, mapper: &mut Mapper, col: usize, text: &str) {
        let idx = band(state)
            .items(col)
            .iter()
            .position(|i| i == text)
            .unwrap_or_else(|| panic!("`{text}` not in column {col}"));
        apply_action(Action::BandClickRow(col, idx), state, mapper);
    }

    /// The whole point of the arity table: which columns open, and when.
    #[test]
    fn arity_drives_column_reachability() {
        use crate::render::command_band::{COL_CARRIED, COL_HERE, COL_SECOND, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);

        // Nothing picked: only VERB.
        assert!(band(&s).col_reachable(COL_VERB));
        assert!(!band(&s).col_reachable(COL_HERE));
        assert!(!band(&s).col_reachable(COL_SECOND));

        // solo: still nothing else, and the phrase is already complete.
        // Every default solo verb the quick row can finish on its own is
        // excluded from the VERB column (SQ-0667, direction-aware; narrowed by
        // SQ-1128, which put `look` back), so give the table a synthetic bare
        // verb rather than leaning on which built-ins survive the filter.
        s.overlays
            .command_band
            .as_mut()
            .unwrap()
            .verbs
            .push(crate::render::command_band::VerbEntry::new(
                "pray",
                vec![crate::render::command_band::VerbLine::bare()],
            ));
        pick_text(&mut s, &mut mapper, COL_VERB, "pray");
        assert!(!band(&s).col_reachable(COL_HERE), "a solo verb offers no object");
        assert!(band(&s).complete());
        assert_eq!(
            band(&s).focus, COL_VERB,
            "nothing left to pick — SQ-0667 retired the trailing phrase-line stop, so focus \
             clamps at the last reachable column instead"
        );

        // object: both object columns open; not complete until one is picked.
        band_reset(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "take");
        assert!(band(&s).col_reachable(COL_HERE) && band(&s).col_reachable(COL_CARRIED));
        assert!(!band(&s).col_reachable(COL_SECOND), "no second slot for an object verb");
        assert!(!band(&s).complete(), "`take` alone is not a command");
        pick_text(&mut s, &mut mapper, COL_HERE, "iron door");
        assert!(band(&s).complete());

        // object_opt: complete with the verb alone, object column still offered.
        band_reset(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "search");
        assert!(band(&s).complete(), "`search` alone is valid");
        assert!(band(&s).col_reachable(COL_HERE), "…but an object may still be added");

        // pair: the second column stays shut until the first object is picked.
        band_reset(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "unlock");
        assert!(!band(&s).col_reachable(COL_SECOND), "WITH… is unreachable before WHAT");
        assert!(!band(&s).complete());
        pick_text(&mut s, &mut mapper, COL_HERE, "iron door");
        assert!(band(&s).col_reachable(COL_SECOND), "WITH… opens once WHAT is filled");
        assert!(!band(&s).complete(), "a pair verb needs both objects");
        pick_text(&mut s, &mut mapper, COL_SECOND, "brass key");
        assert!(band(&s).complete());
    }

    /// Start a fresh phrase. Clears the PROMPT too: since SQ-0676 the typed
    /// line is the phrase, so leaving text there would have the band parse the
    /// old verb straight back in.
    fn band_reset(state: &mut AppState) {
        state.input.set(String::new(), true);
        state.overlays.command_band.as_mut().unwrap().clear_phrase();
    }

    /// Materialization is plain words — multi-word object names go in exactly as
    /// a player would type them, with nothing quoted or escaped.
    #[test]
    fn phrase_materializes_as_plain_text() {
        use crate::render::command_band::{COL_HERE, COL_SECOND, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);

        pick_text(&mut s, &mut mapper, COL_VERB, "unlock");
        pick_text(&mut s, &mut mapper, COL_HERE, "iron door");
        pick_text(&mut s, &mut mapper, COL_SECOND, "brass key");
        assert_eq!(band(&s).phrase_text(), "unlock iron door with brass key");

        band_reset(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "put");
        pick_text(&mut s, &mut mapper, COL_HERE, "mailbox");
        pick_text(&mut s, &mut mapper, COL_SECOND, "lantern");
        assert_eq!(band(&s).phrase_text(), "put mailbox in lantern", "the verb's own preposition");
    }

    /// **The SQ-0676 headline.** Typing reaches the story prompt with the band
    /// wide open, and Enter submits exactly what was typed: `w` + Enter sends
    /// `w`. Falsifies against the pre-SQ-0676 band, whose intercept ate the
    /// letter as a column filter and Enter as a column pick (`BandFilterChar`
    /// / `BandEnter`), so the player could type a whole command into the band
    /// and never send a thing.
    #[test]
    fn typing_reaches_the_prompt_and_enter_submits_it() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);

        type_text(&mut s, &mut mapper, "w");
        assert_eq!(s.input.value, "w", "the letter landed on the real prompt");
        assert_eq!(
            key_to_action(&s, key(KeyCode::Enter)),
            Action::SubmitCommand("w".to_string()),
            "unarmed Enter is the ordinary prompt Enter — the band does not consume it"
        );
    }

    /// A column pick still never fires a turn by itself (regression pin,
    /// unchanged since SQ-0664's "always confirm"): composing puts text on the
    /// prompt and stops there.
    #[test]
    fn a_column_pick_never_fires_a_turn() {
        use crate::render::command_band::COL_VERB;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "take");
        assert_eq!(s.input.value, "take", "it composed onto the prompt");
        assert_eq!(s.turns, 0, "…and ran no turn");
    }

    /// SQ-0667 amendment (2026-08-05): a quick-row pick fires immediately —
    /// unlike a column pick, `apply_action` alone must NOT fill the phrase
    /// (or do anything else band-visible), because firing it is the run
    /// loop's job (it needs the session; `apply_action` doesn't have one).
    /// Before this amendment this action FILLED the phrase (decision 2's
    /// original "always confirm" for the whole band) — this test would fail
    /// against that old behaviour, which is the point.
    #[test]
    fn quick_pick_does_not_fill_the_phrase() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        let n = band(&s).quick.iter().position(|q| q == "n").expect("n in the quick row");

        apply_action(Action::BandQuickPick(n), &mut s, &mut mapper);
        assert_eq!(band(&s).phrase_text(), "", "a quick pick fires directly — it never fills the phrase");
        assert_eq!(s.input.value, "", "the story input line is untouched");
    }

    /// The word `main.rs`'s run loop submits for a quick pick — the plumbing
    /// that stands in for the un-testable (private, in the `main` binary)
    /// event-loop wiring that actually calls `session.submit` with it.
    #[test]
    fn quick_pick_command_resolves_the_word() {
        let mut s = AppState::default();
        open_band(&mut s);
        let n = band(&s).quick.iter().position(|q| q == "n").expect("n in the quick row");
        let look = band(&s).quick.iter().position(|q| q == "look").expect("look in the quick row");

        // Direction abbreviations submit spelled out: Scott Adams vocabularies
        // hold only NORTH/SOUTH/…, so sending the displayed `n` fails there.
        assert_eq!(band_quick_pick_command(&s, n), Some("north".to_string()));
        assert_eq!(band_quick_pick_command(&s, look), Some("look".to_string()), "non-directions pass through");
        assert_eq!(band_quick_pick_command(&s, 9999), None, "a stale index is a no-op, not a panic");

        s.overlays.command_band = None;
        assert_eq!(band_quick_pick_command(&s, n), None, "nothing to resolve once the band is closed");
    }

    /// SQ-1130, the sharpest edge of the same reuse: the expansion that turns
    /// the displayed `n` into `north` was asked of
    /// `mapper::direction::parse_direction`, so a quick row holding `bow`
    /// submitted **`north`** — the player's own word replaced by a heading the
    /// mapper reads it as, on a story where `bow` is a verb.
    ///
    /// Falsify by putting `parse_direction` back: `bow` comes out `north` and
    /// `port` comes out `west`.
    #[test]
    fn a_quick_pick_submits_the_word_on_the_row_not_the_heading_it_resembles() {
        let mut s = AppState::default();
        open_band(&mut s);
        let band_mut = s.overlays.command_band.as_mut().unwrap();
        band_mut.quick = ["bow", "port", "starboard", "n"].iter().map(|w| w.to_string()).collect();

        for (idx, word) in [(0, "bow"), (1, "port"), (2, "starboard")] {
            assert_eq!(
                band_quick_pick_command(&s, idx),
                Some(word.to_string()),
                "`{word}` is the command the button dispatches"
            );
        }
        assert_eq!(
            band_quick_pick_command(&s, 3),
            Some("north".to_string()),
            "…and a real abbreviation still spells itself out for Scott vocabularies"
        );
    }

    /// A quick pick is an interjection, not a composition step (SQ-0667's
    /// pinned choice for "what happens to an in-progress phrase"): glancing
    /// with `look` mid-`unlock iron door` must not disturb it.
    #[test]
    fn quick_pick_leaves_an_in_progress_phrase_intact() {
        use crate::render::command_band::{COL_HERE, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "unlock");
        pick_text(&mut s, &mut mapper, COL_HERE, "iron door");
        assert_eq!(band(&s).phrase_text(), "unlock iron door");

        let n = band(&s).quick.iter().position(|q| q == "n").expect("n in the quick row");
        assert_eq!(
            band_quick_pick_command(&s, n),
            Some("north".to_string()),
            "…while still resolving its own word (spelled out for Scott vocabularies)"
        );
        apply_action(Action::BandQuickPick(n), &mut s, &mut mapper);
        assert_eq!(
            band(&s).phrase_text(),
            "unlock iron door",
            "the interjection did not touch the in-progress phrase"
        );
    }

    /// SQ-0676 flips the retired `typing_filters_the_active_column`: typing no
    /// longer narrows anything. The full column stays listed, and the word
    /// being typed picks out the NEAREST MATCH in whichever column the grammar
    /// expects — a passive highlight, not a filtered list.
    #[test]
    fn typing_highlights_the_nearest_match_instead_of_filtering() {
        use crate::render::command_band::COL_VERB;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);

        let all = band(&s).items(COL_VERB).len();
        type_text(&mut s, &mut mapper, "unl");
        assert_eq!(band(&s).items(COL_VERB).len(), all, "the list never narrows now");

        let (col, idx) = band(&s).nearest_match(&s.input.value).expect("`unl` matches `unlock`");
        assert_eq!(col, COL_VERB, "the first word looks in the VERB column");
        assert_eq!(band(&s).items(col)[idx], "unlock");
        assert_eq!(band(&s).scroll[COL_VERB].selected, idx, "…and the column scrolls to it");
    }

    /// The nearest match follows the grammar into the object columns, and
    /// matches a LATER word of a multi-word name (`do` → `iron door`), which is
    /// what makes typing an object's distinctive word work.
    #[test]
    fn the_match_follows_the_phrase_into_the_object_columns() {
        use crate::render::command_band::{COL_HERE, COL_SECOND};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);

        type_text(&mut s, &mut mapper, "unlock do");
        let (col, idx) = band(&s).nearest_match(&s.input.value).expect("`do` matches `iron door`");
        assert_eq!(col, COL_HERE, "after a verb, the object columns are what's expected");
        assert_eq!(band(&s).items(col)[idx], "iron door");

        // Once the pair verb's own preposition is typed, the second-object
        // column is the one being matched against.
        type_text(&mut s, &mut mapper, "or with bra");
        let (col, idx) = band(&s).nearest_match(&s.input.value).expect("`bra` matches `brass key`");
        assert_eq!(col, COL_SECOND, "past the preposition, the WITH… column takes over");
        assert_eq!(band(&s).items(col)[idx], "brass key");
    }

    /// The phrase parse anchors on the FIRST typed word that is a table verb,
    /// so free text in front of it (a greeting, a leftover word) is the
    /// player's own and never mistaken for a verb — the typed counterpart of
    /// `picks_merge_onto_whatever_was_already_typed`.
    #[test]
    fn free_text_before_the_verb_does_not_derail_the_phrase() {
        use crate::render::command_band::COL_HERE;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "well, take mai");

        assert_eq!(band(&s).phrase_text(), "take mai", "anchored on `take`, not on `well,`");
        let (col, idx) = band(&s).nearest_match(&s.input.value).expect("`mai` matches `mailbox`");
        assert_eq!((col, band(&s).items(col)[idx].as_str()), (COL_HERE, "mailbox"));
    }

    /// Backspace edits the prompt exactly as it does with the band closed
    /// (SQ-0676), and the phrase state simply follows the line back down —
    /// flipping the retired `backspace_clears_the_filter_then_unpicks` ladder,
    /// which consumed Backspace inside the band instead.
    #[test]
    fn backspace_edits_the_prompt_and_the_phrase_follows() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "unlock iron door");
        assert_eq!(band(&s).phrase_text(), "unlock iron door", "typed, not picked");

        let a = key_to_action(&s, key(KeyCode::Backspace));
        assert_eq!(a, Action::Backspace, "Backspace belongs to the prompt now");
        for _ in 0..5 {
            apply_action(Action::Backspace, &mut s, &mut mapper);
        }
        assert_eq!(s.input.value, "unlock iron");
        assert_eq!(band(&s).phrase_text(), "unlock iron", "the phrase tracks the line");
    }

    /// Esc's ladder is two rungs (SQ-0677): clear an EXPLICIT row highlight,
    /// then close. Esc must NOT delete text from the prompt, since that text
    /// is the player's, however it got there.
    #[test]
    fn escape_clears_the_row_highlight_then_closes_and_never_eats_the_prompt() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "take");

        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        assert!(band(&s).row_sel.is_some(), "an explicit row highlight");

        apply_action(Action::BandEscape, &mut s, &mut mapper);
        assert_eq!(band(&s).row_sel, None, "rung 1: the row highlight");
        assert!(s.band_dock.open, "…and the band is still open");
        assert_eq!(s.input.value, "take", "…and the prompt is untouched");

        apply_action(Action::BandEscape, &mut s, &mut mapper);
        assert!(!s.band_dock.open, "rung 2: close");
        assert_eq!(s.input.value, "take", "closing never eats the typed line either");
    }

    /// Bug fix (SQ-0677, reported against the shipped build): Esc-Esc must
    /// close the band from EVERY reachable state, not just the explicitly
    /// armed one — the original defect checked whatever was VISIBLY
    /// highlighted, including the passive typed nearest-match highlight,
    /// which just recomputes right back the instant it's "cleared" (Esc
    /// doesn't change the typed text), making rung 1 fire on every press and
    /// the close rung unreachable. Pins all three starting states. Falsifies
    /// against reverting `Action::BandEscape`'s check from `row_sel` back to
    /// something derived from `highlighted_row` (which includes the typed
    /// match).
    #[test]
    fn esc_esc_closes_the_band_from_every_state() {
        // (a) Neutral: nothing typed, nothing highlighted.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        apply_action(Action::BandEscape, &mut s, &mut mapper);
        apply_action(Action::BandEscape, &mut s, &mut mapper);
        assert!(!s.band_dock.open, "(a) neutral: two Escs close");

        // (b) Typed, with a passive nearest-match highlight but no explicit
        // row_sel — the exact state that triggered the reported bug.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "unl");
        assert!(band(&s).nearest_match(&s.input.value).is_some(), "sanity: a passive match exists");
        assert_eq!(band(&s).row_sel, None, "sanity: nothing explicitly armed");
        apply_action(Action::BandEscape, &mut s, &mut mapper);
        apply_action(Action::BandEscape, &mut s, &mut mapper);
        assert!(!s.band_dock.open, "(b) typed-with-match: two Escs close");

        // (c) An explicit row_sel highlight (the two-rung case).
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        assert!(band(&s).row_sel.is_some(), "sanity: armed");
        apply_action(Action::BandEscape, &mut s, &mut mapper);
        apply_action(Action::BandEscape, &mut s, &mut mapper);
        assert!(!s.band_dock.open, "(c) armed: two Escs close");
    }

    /// The other half of the same bug fix: `toggle-command-panel`/F2 is a
    /// TOGGLE, so the band always has a one-key exit independent of Esc.
    /// Falsifies against `Action::OpenCommandBand` always (re-)opening
    /// regardless of the current state.
    #[test]
    fn open_command_band_toggles_closed_when_already_open() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::OpenCommandBand, &mut s, &mut mapper);
        assert!(s.band_dock.open, "first press opens");
        apply_action(Action::OpenCommandBand, &mut s, &mut mapper);
        assert!(!s.band_dock.open, "second press closes");
        assert!(s.overlays.command_band.is_some(), "content persists for the slide-out, same as BandClose");
    }

    /// Re-picking an earlier slot invalidates everything downstream of it —
    /// otherwise the old verb's object is stranded in the new phrase (and,
    /// since SQ-0667, on the real input line too).
    #[test]
    fn repicking_the_verb_drops_the_stale_object() {
        use crate::render::command_band::{COL_HERE, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "take");
        pick_text(&mut s, &mut mapper, COL_HERE, "mailbox");
        assert_eq!(band(&s).phrase_text(), "take mailbox");
        assert_eq!(s.input.value, "take mailbox");

        // `drop`, not `look` — `look` is excluded from the VERB column by
        // SQ-0667, since it's already on the quick row.
        pick_text(&mut s, &mut mapper, COL_VERB, "drop");
        assert_eq!(band(&s).phrase_text(), "drop", "the object went with the old verb");
        assert_eq!(s.input.value, "drop", "…and so did its mirror on the real prompt");
    }

    /// SQ-0667 (2026-08-05): picks compose directly onto the real story input
    /// line now, MERGING with whatever the player already typed rather than
    /// living apart from it in band-local state — the retired pre-amendment
    /// contract was the opposite (`state.input` was never touched at all).
    /// Closing the band leaves the composed text sitting on the prompt: that
    /// is the whole point, it is real input now, indistinguishable from
    /// anything typed by hand.
    #[test]
    fn picks_merge_onto_whatever_was_already_typed() {
        use crate::render::command_band::{COL_HERE, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.input.set("well, ", true);
        open_band(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "take");
        pick_text(&mut s, &mut mapper, COL_HERE, "mailbox");
        assert_eq!(s.input.value, "well, take mailbox", "picks append onto the pre-existing text");

        apply_action(Action::BandClose, &mut s, &mut mapper);
        assert_eq!(s.input.value, "well, take mailbox", "closing the band does not clear it");
    }

    /// SQ-1230, repro 1: autocompleting `examine` and pressing SPACE leaves
    /// `examine ` on the prompt — a trailing space `parse_phrase` never sees
    /// (it reads `split_whitespace`), so `phrase_text()` is still bare
    /// `examine` while the real input line has an extra character
    /// `strip_band_tail`'s old exact `ends_with` match could not see past.
    /// Clicking a WHAT noun must still REPLACE `examine`, appending the noun
    /// with exactly one separating space, not duplicate the verb.
    #[test]
    fn clicking_what_after_a_trailing_space_does_not_duplicate_the_verb() {
        use crate::render::command_band::{COL_HERE, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "examine");
        assert_eq!(s.input.value, "examine");
        type_text(&mut s, &mut mapper, " ");
        assert_eq!(s.input.value, "examine ", "sanity: the trailing space landed");

        pick_text(&mut s, &mut mapper, COL_HERE, "mailbox");
        assert_eq!(s.input.value, "examine mailbox", "not `examine examine mailbox`");
    }

    /// SQ-1230, repro 2: a partial word typed at the VERB column (`exa`,
    /// short of any exact match `parse_phrase` recognizes) must be REPLACED
    /// by a clicked verb, exactly like `Action::BandTabPick` already replaces
    /// it rather than appending after it — a mouse click and a Tab pick are
    /// the same gesture and must leave the same prompt.
    #[test]
    fn clicking_a_verb_replaces_the_partial_word_being_typed() {
        use crate::render::command_band::COL_VERB;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "exa");
        assert_eq!(s.input.value, "exa");

        pick_text(&mut s, &mut mapper, COL_VERB, "examine");
        assert_eq!(s.input.value, "examine", "the partial word was REPLACED, not appended to");
    }

    /// Tab with NOTHING highlighted is pure column movement (SQ-0677):
    /// clamped at the last reachable stop, same as `Shift-Tab`'s own
    /// movement. Falsifies against a Tab that still no-ops when nothing
    /// matches (the retired SQ-0676 behaviour).
    #[test]
    fn tab_moves_the_column_when_nothing_is_highlighted() {
        use crate::render::command_band::{COL_CARRIED, COL_HERE, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        // Only VERB is reachable yet, so Tab clamps in place — but it is
        // still `BandColumnStep`, not a consumed no-op.
        assert_eq!(command_band_intercept(key(KeyCode::Tab), &s), Some(Action::BandColumnStep(1)));
        apply_action(Action::BandColumnStep(1), &mut s, &mut mapper);
        assert_eq!(band(&s).focus, COL_VERB, "clamped: nowhere else reachable yet");

        pick_text(&mut s, &mut mapper, COL_VERB, "unlock");
        assert_eq!(band(&s).focus, COL_HERE, "picking advanced focus here");
        assert_eq!(command_band_intercept(key(KeyCode::Tab), &s), Some(Action::BandColumnStep(1)));
        apply_action(Action::BandColumnStep(1), &mut s, &mut mapper);
        assert_eq!(band(&s).focus, COL_CARRIED, "Tab moved to the next reachable column");
    }

    /// **The SQ-0677 headline pin.** Typing `ta` highlights `take` in the
    /// current column (the passive nearest match); Tab picks it — composing
    /// onto the prompt exactly like a click, replacing the partial word
    /// rather than appending after it — AND advances focus, unifying
    /// Tab-completion and column flow into one gesture.
    #[test]
    fn tab_picks_the_typed_nearest_match_and_advances() {
        use crate::render::command_band::{COL_HERE, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "ta");
        let (col, idx) = band(&s).nearest_match(&s.input.value).expect("`ta` matches `take`");
        assert_eq!(col, COL_VERB);

        assert_eq!(
            command_band_intercept(key(KeyCode::Tab), &s),
            Some(Action::BandTabPick(COL_VERB, idx))
        );
        apply_action(Action::BandTabPick(COL_VERB, idx), &mut s, &mut mapper);
        assert_eq!(s.input.value, "take", "the partial word was REPLACED, not appended to");
        assert_eq!(band(&s).focus, COL_HERE, "…and focus advanced to the next column");
    }

    /// Tab also picks an EXPLICIT row highlight (`↑`/`↓`), the same as the
    /// typed nearest match — the two are the same "highlighted row"
    /// mechanism (`CommandBandState::highlighted_row`), just armed
    /// differently.
    #[test]
    fn tab_picks_an_explicit_row_highlight_and_advances() {
        use crate::render::command_band::{COL_HERE, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        assert_eq!(band(&s).row_sel, Some(0), "armed on the first press");
        // …and moved off row 0 by the second, because the generic column now
        // opens on `look` (SQ-1128), whose shape takes no object and so opens
        // no column to advance INTO. This case is about advancing, so it needs
        // a row that can.
        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        let row = band(&s).row_sel.expect("still armed");
        let word = band(&s).items(COL_VERB)[row].clone();
        assert!(
            band(&s).verb_by_word(&word).is_some_and(|v| v.max_nouns() > 0),
            "`{word}` must be a verb the band can carry an object for"
        );
        assert_eq!(
            command_band_intercept(key(KeyCode::Tab), &s),
            Some(Action::BandTabPick(COL_VERB, row))
        );
        apply_action(Action::BandTabPick(COL_VERB, row), &mut s, &mut mapper);
        assert!(!s.input.value.is_empty(), "the armed verb composed onto the prompt");
        assert_eq!(band(&s).focus, COL_HERE, "…and focus advanced");
    }

    /// Tab-completing a SECOND word (an object, after the verb is already
    /// typed and recognized) must not double the verb. Falsifies against
    /// `Action::BandTabPick` always pre-stripping the partial word — the
    /// verb's own typed text is already counted in `phrase_text()` at that
    /// point (see `CommandBandState::parse_phrase`'s doc), so an
    /// unconditional pre-strip fights `band_pick_row`'s own tail-diff and
    /// produces `take take door` instead of `take door` (the bug this test
    /// reproduces against a real object list; `tests/command_band.rs`'s
    /// `typing_at_the_prompt_completes_from_the_live_object_columns` pins the
    /// same fix against a live engine).
    #[test]
    fn tab_completing_the_second_word_does_not_double_the_verb() {
        use crate::render::command_band::COL_HERE;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "take mai");
        let (col, idx) = band(&s).nearest_match(&s.input.value).expect("`mai` matches `mailbox`");
        assert_eq!(col, COL_HERE);

        let a = key_to_action(&s, key(KeyCode::Tab));
        assert_eq!(a, Action::BandTabPick(COL_HERE, idx));
        apply_action(a, &mut s, &mut mapper);
        assert_eq!(s.input.value, "take mailbox", "not `take take mailbox`");
    }

    /// The band still OWNS Tab while it is open — beating the dictionary
    /// autocomplete, exactly as under SQ-0676 — even though what Tab now
    /// does (pick-and-advance vs. plain word completion) changed underneath.
    /// Closed, Tab reverts to the ordinary dictionary autocomplete.
    #[test]
    fn the_band_still_owns_tab_while_it_is_open() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.dict_words = vec!["unladen".to_string()];
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "unl");
        assert!(!s.suggestions.is_empty(), "the dictionary has a candidate too");

        let a = key_to_action(&s, key(KeyCode::Tab));
        assert!(
            matches!(a, Action::BandTabPick(..)),
            "the open band claims Tab over the dictionary autocomplete: {a:?}"
        );
        apply_action(a, &mut s, &mut mapper);
        assert_eq!(s.input.value, "unlock", "…the band's word, not the dictionary's");

        // Closed, Tab is the dictionary autocomplete again — unchanged.
        s.overlays.command_band = None;
        s.input.set("unl".to_string(), true);
        recompute_suggestions(&mut s);
        assert_eq!(key_to_action(&s, key(KeyCode::Tab)), Action::Autocomplete);
        apply_action(Action::Autocomplete, &mut s, &mut mapper);
        assert_eq!(s.input.value, "unladen", "closed behaviour is untouched");
    }

    /// `Shift-Tab` stays PURE movement even with a row highlighted — it never
    /// picks, unlike `Tab`. Falsifies against a Shift-Tab that fires the
    /// highlighted row the way Tab does.
    #[test]
    fn shift_tab_never_picks_even_with_a_highlight() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "ta");
        assert!(band(&s).nearest_match(&s.input.value).is_some(), "sanity: a match exists");
        assert_eq!(
            command_band_intercept(key(KeyCode::BackTab), &s),
            Some(Action::BandColumnStep(-1)),
            "Shift-Tab is movement regardless of any highlight"
        );
        apply_action(Action::BandColumnStep(-1), &mut s, &mut mapper);
        assert_eq!(s.input.value, "ta", "…and nothing was picked — the typed text is untouched");
    }

    /// `Enter` NEVER picks (SQ-0677 supersedes the SQ-0676 armed-Enter-fires
    /// rule): it always submits the prompt exactly as typed, whether or not
    /// a row is highlighted. Falsifies against an Enter that still fires a
    /// highlighted quick word or column row.
    #[test]
    fn enter_never_picks_always_submits() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        type_text(&mut s, &mut mapper, "take");
        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        assert!(band(&s).row_sel.is_some(), "sanity: a row is highlighted");

        assert_eq!(
            key_to_action(&s, key(KeyCode::Enter)),
            Action::SubmitCommand("take".to_string()),
            "Enter always submits — the band never claims it"
        );
        assert!(command_band_intercept(key(KeyCode::Enter), &s).is_none(), "not consumed by the band");
    }

    /// Left/Right are plain cursor movement on the edit line (SQ-0677) — the
    /// band claims neither, unlike SQ-0676 where every arrow drove the quick
    /// block. Falsifies against the band still intercepting them.
    #[test]
    fn left_right_are_not_claimed_by_the_band() {
        let mut s = AppState::default();
        open_band(&mut s);
        assert!(command_band_intercept(key(KeyCode::Left), &s).is_none(), "falls through to CursorLeft");
        assert!(command_band_intercept(key(KeyCode::Right), &s).is_none(), "falls through to CursorRight");
        assert_eq!(key_to_action(&s, key(KeyCode::Left)), Action::CursorLeft);
        assert_eq!(key_to_action(&s, key(KeyCode::Right)), Action::CursorRight);
    }

    /// `↑`/`↓` move (or start) the row highlight within the CURRENT column —
    /// clamped at the list's ends, never wrapping, mirroring every other list
    /// in the app. The first press only starts it (mirrors the retired
    /// quick-row "arm on the first press" rule, now applied to columns).
    #[test]
    fn up_down_move_the_row_highlight_within_the_current_column() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        assert_eq!(band(&s).row_sel, None, "the band opens with nothing highlighted");

        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        assert_eq!(band(&s).row_sel, Some(0), "first press only starts the highlight");

        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        assert_eq!(band(&s).row_sel, Some(1), "second press actually moves it");

        apply_action(Action::BandRowNav(-1), &mut s, &mut mapper);
        apply_action(Action::BandRowNav(-1), &mut s, &mut mapper);
        assert_eq!(band(&s).row_sel, Some(0), "clamped at the top, not wrapped");
    }

    /// SQ-0682 (reported bug): stepping the row highlight past the visible
    /// window with `↑`/`↓` must scroll it into view — before this fix,
    /// `step_row` moved `row_sel` but never touched `scroll[focus]`, so the
    /// highlight walked off the bottom of the column while the drawn window
    /// (which windows off `scroll[col].display_offset()`, see
    /// `render::command_band::draw_column`) stayed put. Falsifies: reverting
    /// `step_row`'s `self.scroll[self.focus].select(...)` call reproduces the
    /// original symptom — `target_offset()` stays `0` while `row_sel` walks
    /// past the 3-row window.
    #[test]
    fn up_down_scrolls_the_selection_into_view() {
        use crate::render::command_band::COL_HERE;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.modal_list_viewport = 3; // narrow enough that 5 items don't all fit
        open_band(&mut s);
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.here = vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()];
            b.focus = COL_HERE;
        }
        assert_eq!(band(&s).scroll[COL_HERE].target_offset(), 0, "sanity: window starts at the top");

        // Walk down past the 3-row window (first press only arms row 0 —
        // SQ-0677 — so 5 presses reach row 4, the last item).
        for _ in 0..5 {
            apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        }
        assert_eq!(band(&s).row_sel, Some(4), "sanity: stepped to the last item");
        let offset = band(&s).scroll[COL_HERE].target_offset();
        assert!(
            offset <= 4 && 4 < offset + 3,
            "row 4 must be inside the [offset, offset+3) window, got offset {offset}"
        );
        assert!(offset > 0, "the window must have scrolled down from the top");

        // And back up to the top scrolls the window back with it.
        for _ in 0..5 {
            apply_action(Action::BandRowNav(-1), &mut s, &mut mapper);
        }
        assert_eq!(band(&s).row_sel, Some(0));
        assert_eq!(band(&s).scroll[COL_HERE].target_offset(), 0, "scrolled back to the top");
    }

    /// SQ-0682: PageUp/PageDown are new on the band (it previously had none
    /// at all) — pages the current column's row highlight by ~one viewport,
    /// same as `ListScroll::page` (the story picker and IFDB modal's own
    /// mechanism).
    #[test]
    fn page_up_down_page_the_focused_column() {
        use crate::render::command_band::COL_HERE;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.modal_list_viewport = 3;
        open_band(&mut s);
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.here = (0..20).map(|i| format!("item{i}")).collect();
            b.focus = COL_HERE;
        }

        assert_eq!(
            key_to_action(&s, key(KeyCode::PageDown)),
            Action::BandRowPage(1),
            "PageDown must reach the band"
        );
        apply_action(Action::BandRowPage(1), &mut s, &mut mapper);
        let after_one_page = band(&s).row_sel.expect("armed by the page");
        assert!(after_one_page >= 1, "PageDown must advance roughly a viewport");
        let offset = band(&s).scroll[COL_HERE].target_offset();
        assert!(
            offset <= after_one_page && after_one_page < offset + 3,
            "the paged-to row must be inside the window"
        );

        assert_eq!(key_to_action(&s, key(KeyCode::PageUp)), Action::BandRowPage(-1));
        apply_action(Action::BandRowPage(-1), &mut s, &mut mapper);
        assert!(band(&s).row_sel.unwrap() < after_one_page, "PageUp must retreat");
    }

    /// SQ-0682: Home/End are new on the band — jump the current column's row
    /// highlight to the first/last item, scrolling the window with it.
    #[test]
    fn home_end_jump_the_focused_column() {
        use crate::render::command_band::COL_HERE;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.modal_list_viewport = 3;
        open_band(&mut s);
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.here = (0..20).map(|i| format!("item{i}")).collect();
            b.focus = COL_HERE;
        }

        assert_eq!(key_to_action(&s, key(KeyCode::End)), Action::BandRowEnd);
        apply_action(Action::BandRowEnd, &mut s, &mut mapper);
        assert_eq!(band(&s).row_sel, Some(19), "End jumps to the last item");
        let offset = band(&s).scroll[COL_HERE].target_offset();
        assert!(offset <= 19 && 19 < offset + 3, "the last row must be inside the window");

        assert_eq!(key_to_action(&s, key(KeyCode::Home)), Action::BandRowHome);
        apply_action(Action::BandRowHome, &mut s, &mut mapper);
        assert_eq!(band(&s).row_sel, Some(0), "Home jumps back to the first item");
        assert_eq!(band(&s).scroll[COL_HERE].target_offset(), 0, "…and the window follows to the top");
    }

    /// SQ-0682: the passive typed nearest-match highlight must scroll into
    /// view too, not just an explicit `↑`/`↓` selection — `band_react_to_input`
    /// already drove this before the fix (it's what this test pins), so it is
    /// the one surface the fix must NOT regress.
    #[test]
    fn nearest_match_scrolls_into_view_when_typed() {
        use crate::render::command_band::COL_VERB;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.modal_list_viewport = 3;
        open_band(&mut s);
        // `default_verbs()` is long; find a verb past a 3-row window from the
        // top and type a PARTIAL prefix of it — a complete word with no
        // trailing space is a chosen verb to `parse_phrase` (see its doc),
        // which would advance focus off VERB before `nearest_match` ever ran.
        let verbs = band(&s).items(COL_VERB);
        let (idx, word) = verbs
            .iter()
            .enumerate()
            .find(|(i, _)| *i >= 5)
            .map(|(i, w)| (i, w.clone()))
            .expect("default verb table has more than 5 entries");
        let prefix = &word[..word.len() - 1];
        type_text(&mut s, &mut mapper, prefix);
        assert_eq!(band(&s).nearest_match(&s.input.value), Some((COL_VERB, idx)));
        let offset = band(&s).scroll[COL_VERB].target_offset();
        assert!(
            offset <= idx && idx < offset + 3,
            "the typed nearest match at row {idx} must be inside the window, got offset {offset}"
        );
    }

    /// Typing DISARMS the explicit row highlight (SQ-0677's "the last gesture
    /// decides", same rule the retired `quick_sel` used to follow): one
    /// keystroke after arming, the highlight the band shows reverts to
    /// whatever the typed text passively matches (or nothing).
    #[test]
    fn typing_disarms_the_row_highlight() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        assert!(band(&s).row_sel.is_some(), "armed");

        type_text(&mut s, &mut mapper, "x");
        assert_eq!(band(&s).row_sel, None, "typing disarmed it");

        // Backspace is a text change too.
        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        assert!(band(&s).row_sel.is_some());
        apply_action(Action::Backspace, &mut s, &mut mapper);
        assert_eq!(band(&s).row_sel, None, "…and so is a deletion");
    }

    /// The wheel scrolls a column's LIST, not its cursor (SQ-0831's rule, which
    /// the band was wrongly left out of — SQ-0832), and it does it to the column
    /// under the pointer without stealing the keyboard from another.
    #[test]
    fn wheel_scrolls_a_column_without_taking_focus() {
        use crate::render::command_band::{BAND_COLS, COL_CARRIED, COL_HERE, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "take");
        // Cursor is on the HERE column; the pointer will be over CARRIED.
        assert_eq!(band(&s).focus, COL_HERE);
        let anim = s.config.animation.clone();
        {
            // Lists taller than their windows, plus the per-column window
            // heights the render publishes every frame — without one there is
            // no window to scroll and the notch would (correctly) do nothing.
            let b = s.overlays.command_band.as_mut().expect("band open");
            b.here = (0..20).map(|i| format!("here {i}")).collect();
            b.carried = (0..20).map(|i| format!("carried {i}")).collect();
            b.col_viewport.set([4; BAND_COLS]);
            b.scroll[COL_CARRIED].len(20);
            b.scroll[COL_CARRIED].select(6, 4, &anim); // rows 3..7 visible
        }

        // A notch over HERE: the window slides, and the highlight — which was
        // on the row the window just left — rides its top edge rather than
        // being stepped down the list.
        apply_action(Action::BandWheel(COL_HERE, 1), &mut s, &mut mapper);
        assert_eq!(band(&s).scroll[COL_HERE].target_offset(), 1, "the list moved under the cursor");
        assert_eq!(band(&s).scroll[COL_HERE].selected, 1, "the cursor rides the top edge, not off it");
        assert_eq!(band(&s).focus, COL_HERE, "focus unchanged");

        // …and the other direction, on the column the band is NOT pointing at:
        // the highlight sits mid-window, so it holds its row while the list
        // slides, and only starts riding the bottom edge once caught.
        apply_action(Action::BandWheel(COL_CARRIED, -1), &mut s, &mut mapper);
        assert_eq!(band(&s).scroll[COL_CARRIED].target_offset(), 2);
        assert_eq!(band(&s).scroll[COL_CARRIED].selected, 5, "clamped to the window's last row");
        assert_eq!(band(&s).focus, COL_HERE, "…still without moving the band's attention");
        assert_eq!(band(&s).scroll[COL_HERE].target_offset(), 1, "and without touching the other column");
    }

    /// A column the frame never drew has no window, so its wheel is inert —
    /// `ListScroll::scroll_by` refuses to scroll a viewport it cannot measure,
    /// and must NOT fall back to stepping the cursor as a consolation.
    #[test]
    fn wheel_on_an_unmeasured_column_does_nothing_at_all() {
        use crate::render::command_band::{BAND_COLS, COL_HERE, COL_VERB};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "take");
        {
            let b = s.overlays.command_band.as_mut().expect("band open");
            b.here = (0..20).map(|i| format!("here {i}")).collect();
            b.col_viewport.set([0; BAND_COLS]); // mid-slide: nothing measured yet
        }
        apply_action(Action::BandWheel(COL_HERE, 1), &mut s, &mut mapper);
        assert_eq!(band(&s).scroll[COL_HERE].target_offset(), 0);
        assert_eq!(band(&s).scroll[COL_HERE].selected, 0, "not stepped as a consolation");
    }

    /// Opening/closing follows the drawer pattern the inventory dock uses.
    #[test]
    fn open_and_close_arm_the_slide_and_settle_clears_the_content() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::OpenCommandBand, &mut s, &mut mapper);
        assert!(s.band_dock.open, "armed toward open");
        assert!(s.overlays.command_band.is_some());
        assert_eq!(band(&s).row_sel, None, "the band opens with nothing highlighted");

        apply_action(Action::BandClose, &mut s, &mut mapper);
        assert!(!s.band_dock.open, "armed toward closed");
        assert!(s.overlays.command_band.is_some(), "content persists during the slide-out");
        s.settle_command_band();
        assert!(s.overlays.command_band.is_some(), "…including while the slide may still run");

        s.band_dock.toggle_to(false, true);
        s.settle_command_band();
        assert!(s.overlays.command_band.is_none(), "cleared once the slide-out settles");
    }

    /// `toggle-command-panel`/F2 is now a TOGGLE (SQ-0677): re-invoking it while
    /// still mid-close (the content hasn't settled to `None` yet) reopens the
    /// band with its phrase intact, rather than starting fresh — the same
    /// property `reopening_does_not_reset_the_phrase` pinned before the
    /// toggle behaviour existed, now exercised through a close/reopen cycle
    /// instead of a no-op double-open.
    #[test]
    fn reopening_before_the_close_settles_keeps_the_phrase() {
        use crate::render::command_band::COL_VERB;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        pick_text(&mut s, &mut mapper, COL_VERB, "take");

        apply_action(Action::OpenCommandBand, &mut s, &mut mapper); // toggle: closes
        assert!(!s.band_dock.open);
        assert!(s.overlays.command_band.is_some(), "content persists mid-slide-out");

        apply_action(Action::OpenCommandBand, &mut s, &mut mapper); // toggle: reopens
        assert!(s.band_dock.open);
        assert_eq!(band(&s).phrase_text(), "take", "the phrase survived the close/reopen");
        assert_eq!(s.input.value, "take", "…and its mirror on the real prompt");
    }

    /// The verb table comes from config, so a replaced grammar reaches the band.
    #[test]
    fn open_uses_the_configured_verb_table() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.config.command_band.verbs = vec![crate::config::VerbConfig {
            word: "polish".to_string(),
            arity: "object".to_string(),
            prep: None,
        }];
        s.config.command_band.quick = vec!["xyzzy".to_string()];
        apply_action(Action::OpenCommandBand, &mut s, &mut mapper);
        assert_eq!(band(&s).verbs.len(), 1);
        assert_eq!(band(&s).verbs[0].word, "polish");
        assert_eq!(band(&s).quick, vec!["xyzzy".to_string()]);
    }

    /// A bad arity in config is reported in the transcript, not swallowed.
    #[test]
    fn open_reports_a_bad_arity_from_config() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.config.command_band.extra_verbs = vec![crate::config::VerbConfig {
            word: "frob".to_string(),
            arity: "triple".to_string(),
            prep: None,
        }];
        apply_action(Action::OpenCommandBand, &mut s, &mut mapper);
        assert!(
            s.transcript.iter().any(|l| l.contains("frob")),
            "the warning reaches the player: {:?}",
            s.transcript
        );
    }

    /// The band is NOT a modal. This is the property the whole redesign turns
    /// on: the story prompt line stays live, paste keeps working, and the v6
    /// pixel path (gated on `any_modal_overlay_open`) is not dropped.
    #[test]
    fn the_band_is_not_a_modal_overlay() {
        let mut s = AppState::default();
        open_band(&mut s);
        assert!(!s.any_modal_overlay_open(), "not a modal");
        assert!(!s.any_overlay_open(), "not an overlay at all");
        assert!(!s.open_modal_overlays().contains(&"command_band"));
    }

    /// Paste always reaches the story prompt with the band open (SQ-0676),
    /// exactly like typing. The band re-reads the pasted line afterwards, so
    /// the phrase state follows and the row highlight (SQ-0677: `row_sel`,
    /// not the retired `quick_sel`) disarms just like any other text change.
    #[test]
    fn paste_reaches_the_prompt_with_the_band_open() {
        use crate::render::command_band::{COL_CARRIED, COL_HERE};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);

        assert!(apply_paste(&mut s, "take lan"), "consumed");
        assert_eq!(s.input.value, "take lan", "the story input line takes it");
        assert_eq!(band(&s).row_sel, None, "a paste is a text change — it disarms too");

        // `take` opens the object columns; focus lands on HERE (the
        // grammar's first expected column — SQ-0677 no longer auto-jumps to
        // whichever column happens to match best). Tab to CARRIED to find
        // `lantern` there, since the nearest match is scoped to the CURRENT
        // column now.
        assert_eq!(band(&s).focus, COL_HERE, "the first expected column, not a match-driven jump");
        apply_action(Action::BandColumnStep(1), &mut s, &mut mapper);
        assert_eq!(band(&s).focus, COL_CARRIED);
        let (col, idx) = band(&s).nearest_match(&s.input.value).expect("`lan` matches `lantern`");
        assert_eq!((col, band(&s).items(col)[idx].as_str()), (COL_CARRIED, "lantern"));
    }

    // ── SQ-0237: interactive pane resize mode ─────────────────────────────────

    #[test]
    fn resize_panes_enters_mode_targeting_story_map() {
        // Default state is Layout::Split with no verb menu/inventory, so
        // StoryMap is the only (and first) visible target.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert!(s.resize_mode);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::StoryMap);
    }

    #[test]
    fn resize_nav_right_grows_split_ratio_and_clamps_at_80() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.split_ratio, 50);
        apply_action(Action::ResizeNav(ResizeNavKind::Right), &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.split_ratio, 53);
        assert_eq!(s.config.split_ratio, 53, "config must mirror pane_sizes after nav");
        for _ in 0..20 {
            apply_action(Action::ResizeNav(ResizeNavKind::Right), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.split_ratio, 80, "clamped at 80");
        assert_eq!(s.config.split_ratio, 80);
    }

    #[test]
    fn resize_nav_left_shrinks_split_ratio_to_floor_20() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        for _ in 0..20 {
            apply_action(Action::ResizeNav(ResizeNavKind::Left), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.split_ratio, 20, "clamped at floor 20");
        assert_eq!(s.config.split_ratio, 20);
    }

    #[test]
    fn resize_nav_up_on_inv_dock_target_raises_inv_dock_pct_clamped_at_80() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.show_inventory = true;
        s.resize_mode = true;
        s.resize_target = crate::state::ResizeTarget::InvDock;
        assert_eq!(s.pane_sizes.inv_dock_pct, 33);
        apply_action(Action::ResizeNav(ResizeNavKind::Up), &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.inv_dock_pct, 36);
        assert_eq!(s.config.inv_dock_pct, 36);
        for _ in 0..20 {
            apply_action(Action::ResizeNav(ResizeNavKind::Up), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.inv_dock_pct, 80, "clamped at 80");
    }

    #[test]
    fn resize_reset_restores_defaults_and_mirrors_config() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        apply_action(Action::ResizeNav(ResizeNavKind::Right), &mut s, &mut mapper);
        assert_ne!(s.pane_sizes.split_ratio, 50);

        apply_action(Action::ResizeReset, &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.split_ratio, 50);
        assert_eq!(s.pane_sizes.band_height, crate::render::command_band::DEFAULT_BAND_ROWS);
        assert_eq!(s.pane_sizes.inv_dock_pct, 33);
        assert_eq!(s.config.split_ratio, 50);
        assert_eq!(s.config.command_band.height, crate::render::command_band::DEFAULT_BAND_ROWS);
        assert_eq!(s.config.inv_dock_pct, 33);
    }

    #[test]
    fn resize_reset_sets_pending_config_write() {
        // Regression for the reset-pane-size slash command never persisting:
        // the run loop's KeyResolve::Command dispatch path never reached the
        // old resize_persist-guarded write, so it needs this flag to signal
        // the pending write from apply_action instead. See flush_pending_config_write
        // in main.rs, called from both dispatch paths.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        assert!(!s.pending_config_write);
        apply_action(Action::ResizeReset, &mut s, &mut mapper);
        assert!(s.pending_config_write);
    }

    #[test]
    fn resize_exit_clears_resize_mode() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert!(s.resize_mode);
        apply_action(Action::ResizeExit, &mut s, &mut mapper);
        assert!(!s.resize_mode);
    }

    #[test]
    fn resize_exit_sets_pending_config_write() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert!(!s.pending_config_write);
        apply_action(Action::ResizeExit, &mut s, &mut mapper);
        assert!(s.pending_config_write);
    }

    #[test]
    fn resize_panes_is_noop_when_nothing_visible() {
        // TranscriptFull with no verb menu/inventory → no visible targets.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.layout = crate::state::Layout::TranscriptFull;
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert!(!s.resize_mode, "no visible pane → resize mode does not open");
    }

    #[test]
    fn resize_targets_visible_matches_spec_order() {
        let mut s = AppState::default();
        assert_eq!(s.resize_targets_visible(), vec![crate::state::ResizeTarget::StoryMap]);

        s.show_inventory = true;
        assert_eq!(
            s.resize_targets_visible(),
            vec![crate::state::ResizeTarget::StoryMap, crate::state::ResizeTarget::InvDock]
        );

        s.layout = crate::state::Layout::TranscriptFull;
        s.show_inventory = false;
        assert!(s.resize_targets_visible().is_empty());
    }

    #[test]
    fn resize_targets_visible_includes_the_band_when_open() {
        // SQ-0238: resize mode preempts the band's key intercept, so the two
        // coexist and the band becomes a resize target while it is open
        // (appended after StoryMap/InvDock in the Tab-cycle order).
        let mut s = AppState::default();
        open_band(&mut s);
        assert_eq!(
            s.resize_targets_visible(),
            vec![crate::state::ResizeTarget::StoryMap, crate::state::ResizeTarget::CommandBand]
        );

        s.show_inventory = true;
        assert_eq!(
            s.resize_targets_visible(),
            vec![
                crate::state::ResizeTarget::StoryMap,
                crate::state::ResizeTarget::InvDock,
                crate::state::ResizeTarget::CommandBand,
            ]
        );
    }

    /// SQ-0692: an open room dock joins the Tab cycle — but only where it can
    /// actually be drawn. It is carved out of the map pane, so a layout with no
    /// map has no dock edge to move.
    #[test]
    fn resize_targets_visible_includes_the_room_dock_only_with_a_map_pane() {
        use crate::state::ResizeTarget;
        let mut s = AppState::default();
        s.room_dock.toggle_to(true, true);
        assert_eq!(
            s.resize_targets_visible(),
            vec![ResizeTarget::StoryMap, ResizeTarget::RoomDock],
            "an open dock is the last target in the cycle"
        );

        // Tab cycles into it and wraps back out.
        s.resize_target = ResizeTarget::StoryMap;
        s.cycle_resize_target(true);
        assert_eq!(s.resize_target, ResizeTarget::RoomDock);
        s.cycle_resize_target(true);
        assert_eq!(s.resize_target, ResizeTarget::StoryMap, "…and wraps");

        s.layout = crate::state::Layout::TranscriptFull;
        assert!(
            !s.resize_targets_visible().contains(&ResizeTarget::RoomDock),
            "no map pane, no dock edge to drag"
        );

        s.layout = crate::state::Layout::Split;
        s.room_dock.toggle_to(false, true);
        assert!(
            !s.resize_targets_visible().contains(&ResizeTarget::RoomDock),
            "a closed dock is not a target"
        );
    }

    /// The room dock resizes by percentage of the frame, like the inventory
    /// dock, and clamps to the shared limits at both ends.
    #[test]
    fn resize_nav_adjusts_the_room_dock_pct_and_clamps() {
        use crate::layout::{MAX_ROOM_DOCK_PCT, MIN_ROOM_DOCK_PCT};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.room_dock.toggle_to(true, true);
        s.resize_mode = true;
        s.resize_target = crate::state::ResizeTarget::RoomDock;

        assert_eq!(s.pane_sizes.room_dock_pct, crate::config::default_room_dock_pct());
        apply_action(Action::ResizeNav(ResizeNavKind::Up), &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.room_dock_pct, 36);
        assert_eq!(s.config.room_dock_pct, 36, "config mirrors pane_sizes");
        apply_action(Action::ResizeNav(ResizeNavKind::Down), &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.room_dock_pct, 33);

        for _ in 0..40 {
            apply_action(Action::ResizeNav(ResizeNavKind::Up), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.room_dock_pct, MAX_ROOM_DOCK_PCT, "clamped at the top");
        for _ in 0..40 {
            apply_action(Action::ResizeNav(ResizeNavKind::Down), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.room_dock_pct, MIN_ROOM_DOCK_PCT, "clamped at the bottom");

        // And `0` (reset) puts it back to the seeded default.
        apply_action(Action::ResizeReset, &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.room_dock_pct, crate::config::default_room_dock_pct());
    }

    /// SQ-0664: the band resizes by ROWS (up grows, down shrinks), replacing
    /// the retired `verb_dock_pct` percentage-width knob.
    #[test]
    fn resize_nav_adjusts_the_band_height_in_rows() {
        use crate::render::command_band::{DEFAULT_BAND_ROWS, MAX_BAND_ROWS, MIN_BAND_ROWS};
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        s.resize_mode = true;
        s.resize_target = crate::state::ResizeTarget::CommandBand;

        assert_eq!(s.pane_sizes.band_height, DEFAULT_BAND_ROWS);
        apply_action(Action::ResizeNav(ResizeNavKind::Up), &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.band_height, DEFAULT_BAND_ROWS + 1);
        assert_eq!(s.config.command_band.height, DEFAULT_BAND_ROWS + 1, "config mirrors pane_sizes");
        apply_action(Action::ResizeNav(ResizeNavKind::Down), &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.band_height, DEFAULT_BAND_ROWS);

        for _ in 0..30 {
            apply_action(Action::ResizeNav(ResizeNavKind::Up), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.band_height, MAX_BAND_ROWS, "clamped at the top");
        for _ in 0..30 {
            apply_action(Action::ResizeNav(ResizeNavKind::Down), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.band_height, MIN_BAND_ROWS, "clamped at the bottom");
    }

    #[test]
    fn cycle_resize_target_wraps_and_skips_non_visible() {
        let mut s = AppState::default();
        s.show_inventory = true;
        s.resize_target = crate::state::ResizeTarget::StoryMap;

        s.cycle_resize_target(true);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::InvDock, "wraps forward");
        s.cycle_resize_target(true);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::StoryMap, "wraps forward again");

        s.cycle_resize_target(false);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::InvDock, "wraps backward");

        // Current target not visible → snaps to the first visible one.
        s.show_inventory = false;
        s.resize_target = crate::state::ResizeTarget::InvDock;
        s.cycle_resize_target(true);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::StoryMap);
    }

    /// Plain `↑`/`↓` resolve to row navigation while the band is open
    /// (SQ-0677) — `←`/`→` do NOT (they're cursor movement now, unlike
    /// SQ-0676 where every arrow drove the quick block). Shift+Arrow still
    /// pans the map either way.
    #[test]
    fn plain_up_down_resolve_to_row_nav_left_right_do_not() {
        let mut s = AppState::default();
        open_band(&mut s);
        assert_eq!(key_to_action(&s, key(KeyCode::Up)), Action::BandRowNav(-1));
        assert_eq!(key_to_action(&s, key(KeyCode::Down)), Action::BandRowNav(1));
        assert_eq!(key_to_action(&s, key(KeyCode::Left)), Action::CursorLeft, "← is cursor movement");
        assert_eq!(key_to_action(&s, key(KeyCode::Right)), Action::CursorRight, "→ is cursor movement");
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::BandEscape));

        let shift_left = KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT);
        assert!(command_band_intercept(shift_left, &s).is_none(), "Shift+Arrow still pans the map");
    }

    /// SQ-0676 flips the retired
    /// `letters_filter_when_the_band_is_focused_and_type_when_it_is_not`: a
    /// letter ALWAYS types at the prompt, band open or not. There is no focus
    /// state left for it to depend on.
    #[test]
    fn letters_always_type_at_the_prompt() {
        let mut s = AppState::default();
        open_band(&mut s);
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('q'))), Action::InputChar('q')));
        assert!(command_band_intercept(key(KeyCode::Char('q')), &s).is_none(), "not consumed");

        s.overlays.command_band = None;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('q'))), Action::InputChar('q')));
    }

    /// Ctrl/Alt chords are never eaten by the band — app commands keep working
    /// with it up.
    #[test]
    fn ctrl_chords_are_not_swallowed_by_the_filter() {
        let mut s = AppState::default();
        open_band(&mut s);
        assert!(command_band_intercept(ctrl(KeyCode::Char('s')), &s).is_none());
    }

    /// F2 was the direct default binding until SQ-1142 unbound every F-key: a
    /// v4+ story may claim them through its own $2E terminating-characters
    /// table, and Arthur does. The panel's ways in are the leader panel's `v`,
    /// the `/toggle-command-panel` command, and the `≡` control on the pane
    /// border — the palette row here is what this case pins.
    #[test]
    fn f2_no_longer_opens_the_command_band_by_default() {
        use crate::keymap::{KeyMap, KeySpec};
        let km = KeyMap::default();
        let spec = KeySpec { code: KeyCode::F(2), ctrl: false, shift: false, alt: false };
        assert_eq!(km.lookup(&spec, crate::keymap::Context::Global), None);
        assert_eq!(
            km.primary_key("toggle-command-panel"),
            None,
            "toggle-command-panel is leader-, command- and click-reachable: no default key",
        );
    }

    #[test]
    fn open_command_band_is_in_the_view_leader_group() {
        use crate::keymap::HotkeyLayout;
        let layout = HotkeyLayout::default();
        // The group is titled "Map · View" — the panel renders flat headings, so
        // the three map sub-groups spell their parent rather than indenting.
        let (_, cmds) = layout
            .groups
            .iter()
            .find(|(title, _)| title == "Map \u{b7} View")
            .expect("Map \u{b7} View group should exist");
        assert!(cmds.iter().any(|c| c.1 == "toggle-command-panel"));
    }

    // ── File-browser sub-mode key tests ───────────────────────────────────────

    /// Build a state with saves open (for testing e/i dispatch).
    fn state_with_saves_for_fb_tests() -> AppState {
        let mut s = AppState::default();
        s.overlays.saves = Some(crate::state::SavesState { entries: Vec::new(), scroll: Default::default() });
        s
    }

    /// Build a state with the file browser open.
    fn state_with_filebrowser(mode: crate::state::FbMode) -> AppState {
        use crate::state::FileBrowserState;
        let mut s = AppState::default();
        let tmp = std::env::temp_dir();
        s.overlays.file_browser = Some(FileBrowserState::build(tmp, mode));
        s
    }

    #[test]
    fn saves_i_opens_import_browser_action() {
        let s = state_with_saves_for_fb_tests();
        let a = key_to_action(&s, key(KeyCode::Char('i')));
        assert!(matches!(a, Action::SavesImport), "i in saves sub-mode should produce SavesImport");
    }

    #[test]
    fn filebrowser_esc_produces_fb_close() {
        let s = state_with_filebrowser(crate::state::FbMode::PickFile);
        let a = key_to_action(&s, key(KeyCode::Esc));
        assert!(matches!(a, Action::FbClose), "Esc in file browser should produce FbClose");
    }

    #[test]
    fn filebrowser_q_no_longer_closes() {
        // q-close removed from file browser; q now produces None in this sub-mode.
        let s = state_with_filebrowser(crate::state::FbMode::PickFile);
        let a = key_to_action(&s, key(KeyCode::Char('q')));
        assert!(matches!(a, Action::None), "q should no longer close the file browser");
    }

    #[test]
    fn filebrowser_up_down_navigate() {
        let s = state_with_filebrowser(crate::state::FbMode::PickFile);
        assert!(matches!(key_to_action(&s, key(KeyCode::Up)), Action::FbNav(-1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::FbNav(1)));
    }

    #[test]
    fn filebrowser_enter_produces_fb_enter() {
        let s = state_with_filebrowser(crate::state::FbMode::PickFile);
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(matches!(a, Action::FbEnter), "Enter in file browser should produce FbEnter");
    }

    #[test]
    fn fb_close_action_clears_file_browser() {
        let mut s = state_with_filebrowser(crate::state::FbMode::PickFile);
        assert!(s.overlays.file_browser.is_some());
        apply_action(Action::FbClose, &mut s, &mut Mapper::default());
        assert!(s.overlays.file_browser.is_none(), "FbClose should clear file_browser");
    }

    #[test]
    fn fb_nav_wraps_around() {
        let mut s = state_with_filebrowser(crate::state::FbMode::PickFile);
        // We need at least one entry — the tmp dir should have ".." if not root.
        if let Some(fb) = &s.overlays.file_browser {
            if fb.entries.is_empty() {
                return; // nothing to navigate
            }
        }
        // Move up from 0 should wrap to last entry.
        apply_action(Action::FbNav(-1), &mut s, &mut Mapper::default());
        if let Some(fb) = &s.overlays.file_browser {
            assert_eq!(fb.scroll.selected, fb.entries.len() - 1, "nav -1 from 0 should wrap to last");
        }
    }

    // ── Item 1: char-granular drag pan ────────────────────────────────────────

    /// DragPanTo accumulates into char_pan at 1-character resolution.
    /// A drag delta of N columns shifts char_pan.0 by -N (grab-and-drag semantics).
    #[test]
    fn drag_pan_to_accumulates_char_pan() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // Start drag at (10, 10).
        apply_action(Action::BeginDragPan(10, 10), &mut s, &mut m);
        // Drag 5 columns right, 3 rows down (grab: content follows cursor).
        apply_action(Action::DragPanTo(15, 13), &mut s, &mut m);
        assert_eq!(
            s.char_pan,
            (5, 3),
            "drag right+down by (5,3) should set char_pan to (5,3)"
        );
        // Continue dragging 2 columns left.
        apply_action(Action::DragPanTo(13, 13), &mut s, &mut m);
        assert_eq!(
            s.char_pan,
            (3, 3),
            "additional drag left by 2 should update char_pan to (3,3)"
        );
    }

    /// Ending the drag clears state.drag but leaves char_pan intact.
    #[test]
    fn end_drag_pan_leaves_char_pan() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::BeginDragPan(5, 5), &mut s, &mut m);
        apply_action(Action::DragPanTo(8, 5), &mut s, &mut m);
        assert_eq!(s.char_pan, (3, 0));
        apply_action(Action::EndDragPan, &mut s, &mut m);
        assert!(s.drag.is_none(), "EndDragPan should clear drag state");
        assert_eq!(s.char_pan, (3, 0), "EndDragPan must not reset char_pan");
    }

    /// Build a minimal MouseEvent for testing.
    fn mouse_left_click(col: u16, row: u16) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn config_dialog_button_clicks_map_to_actions() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::ConfigScreenState;

        // Build known rects:
        // dialog area at (10, 5, 40, 15)
        // close at (48, 5, 1, 1)  — just inside top-right
        // Save button at (20, 19, 8, 1)
        // Cancel button at (29, 19, 10, 1)
        let rects = DialogRects {
            area:    Rect::new(10, 5, 40, 15),
            content: Rect::new(11, 7, 38, 10),
            close:   Some(Rect::new(48, 5, 1, 1)),
            buttons: vec![
                (ButtonId::Save,   Rect::new(20, 19, 8,  1)),
                (ButtonId::Cancel, Rect::new(29, 19, 10, 1)),
            ],
            field: None,
        };

        // State with config_screen open (so dialog routing knows which modal).
        let mut state = AppState::default();
        let working = crate::input::clone_config(&state.config);
        state.overlays.config_screen = Some(ConfigScreenState { working, scroll: Default::default() });

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → ConfigCancel
        let a = mouse_to_action(&state, mouse_left_click(48, 5), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::ConfigCancel), "close click should produce ConfigCancel, got {:?}", a);

        // Save button → ConfigSave
        let a = mouse_to_action(&state, mouse_left_click(22, 19), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::ConfigSave), "Save button should produce ConfigSave, got {:?}", a);

        // Cancel button → ConfigCancel
        let a = mouse_to_action(&state, mouse_left_click(32, 19), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::ConfigCancel), "Cancel button should produce ConfigCancel, got {:?}", a);

        // Click outside dialog area → swallowed (Action::None)
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::None), "outside-dialog click should be swallowed (None), got {:?}", a);
    }

    #[test]
    fn config_esc_maps_to_config_cancel() {
        // ESC in config screen should produce ConfigCancel (same as [X] and Cancel button).
        let mut s = AppState::default();
        let working = crate::input::clone_config(&s.config);
        s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
        let a = key_to_action(&s, key(KeyCode::Esc));
        assert!(matches!(a, Action::ConfigCancel), "ESC in config screen should produce ConfigCancel");
    }

    #[test]
    fn saves_dialog_x_and_done_produce_saves_close() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::SavesState;
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;

        let rects = DialogRects {
            area:    Rect::new(10, 5, 40, 15),
            content: Rect::new(11, 7, 38, 10),
            close:   Some(Rect::new(48, 5, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(40, 19, 8, 1))],
            field: None,
        };

        let mut state = AppState::default();
        state.overlays.saves = Some(SavesState {
            entries: vec![SaveInfo {
                path: PathBuf::from("/tmp/a.lanthorn"),
                name: "a".into(),
                turns: 0,
                saved_at: String::new(),
                location: None, score: None, is_default: false, trigger: crate::archive::SaveTrigger::HostState,
            }],
            scroll: Default::default(),
        });

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → SavesClose
        let a = mouse_to_action(&state, mouse_left_click(48, 5), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::SavesClose), "saves [X] click should produce SavesClose, got {:?}", a);

        // Done button → SavesClose
        let a = mouse_to_action(&state, mouse_left_click(42, 19), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::SavesClose), "saves [Done] click should produce SavesClose, got {:?}", a);

        // Click outside → swallowed
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::None), "outside saves dialog should be swallowed, got {:?}", a);
    }

    #[test]
    fn filebrowser_dialog_x_and_done_produce_fb_close() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};

        let rects = DialogRects {
            area:    Rect::new(8, 4, 50, 18),
            content: Rect::new(9, 6, 48, 13),
            close:   Some(Rect::new(56, 4, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(48, 21, 8, 1))],
            field: None,
        };

        let mut state = AppState::default();
        let tmp = std::env::temp_dir();
        state.overlays.file_browser = Some(crate::state::FileBrowserState::build(
            tmp,
            crate::state::FbMode::PickFile));

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → FbClose
        let a = mouse_to_action(&state, mouse_left_click(56, 4), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::FbClose), "filebrowser [X] click should produce FbClose, got {:?}", a);

        // Done button → FbClose
        let a = mouse_to_action(&state, mouse_left_click(50, 21), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::FbClose), "filebrowser [Done] click should produce FbClose, got {:?}", a);

        // Click outside → swallowed
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::None), "outside filebrowser dialog should be swallowed, got {:?}", a);
    }

    /// SQ-0690: a second click on the same band row within the window is a double-click (the
    /// run loop submits the prompt on it); a different row, a slow second click, or the click
    /// AFTER a completed double are all fresh picks.
    #[test]
    fn band_double_click_fires_on_the_same_row_within_the_window_only() {
        use std::time::{Duration, Instant};
        let mut t = BandClickTracker::default();
        let t0 = Instant::now();
        let fast = Duration::from_millis(100);

        assert!(!t.observe(1, 3, t0), "a first click is a pick");
        assert!(t.observe(1, 3, t0 + fast), "the same row again, fast: double");
        assert!(
            !t.observe(1, 3, t0 + fast * 2),
            "the click after a completed double is a FRESH pick — the submit emptied the prompt"
        );

        let mut t = BandClickTracker::default();
        assert!(!t.observe(1, 3, t0));
        assert!(!t.observe(1, 4, t0 + fast), "a different row is a pick, not a double");
        assert!(!t.observe(2, 4, t0 + fast * 2), "a different column too");

        let mut t = BandClickTracker::default();
        assert!(!t.observe(1, 3, t0));
        assert!(
            !t.observe(1, 3, t0 + BandClickTracker::WINDOW),
            "at/after the window the pair has expired"
        );
    }

    #[test]
    fn command_band_does_not_swallow_map_clicks() {
        // The band is a dock, not a centered modal — a click in the map pane
        // must route normally instead of being swallowed as modal chrome.
        use ratatui::layout::Rect;

        let mut state = AppState::default();
        open_band(&mut state);

        let map   = Rect::new(30, 0, 50, 24);
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = None;

        let a = mouse_to_action(&state, mouse_left_click(40, 5), map, story, room_rects, &dialog);
        assert!(!matches!(a, Action::None), "click in map should route normally with the band open, got {:?}", a);
    }

    #[test]
    fn hotkey_dialog_x_and_done_produce_close_hotkey_dialog() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};

        let rects = DialogRects {
            area:    Rect::new(10, 5, 60, 30),
            content: Rect::new(11, 7, 58, 26),
            close:   Some(Rect::new(68, 5, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(60, 34, 8, 1))],
            field: None,
        };

        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → CloseHotkeyDialog
        let a = mouse_to_action(&state, mouse_left_click(68, 5), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::CloseHotkeyDialog), "hotkey dialog [X] click should produce CloseHotkeyDialog, got {:?}", a);

        // Done button → CloseHotkeyDialog
        let a = mouse_to_action(&state, mouse_left_click(62, 34), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::CloseHotkeyDialog), "hotkey dialog [Done] click should produce CloseHotkeyDialog, got {:?}", a);
    }

    // ── ESC == [X] sweep ─────────────────────────────────────────────────────────

    /// Table test: for every modal, ESC and a [X] click must yield the SAME close Action.
    ///
    /// Each entry: (modal_name, set-up closure, ESC-Action, close-Action-from-X-click).
    ///
    /// We build a DialogRects with a known close rect at (99, 0) and call
    /// key_to_action for ESC and mouse_to_action for a click at (99, 0).
    /// Both must match the expected close action.
    #[test]
    fn esc_equals_x_click_for_every_modal() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::SavesState;
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];

        // Helper to build a DialogRects with [X] at (99, 0) and one Done button
        let make_rects = || DialogRects {
            area:    Rect::new(5, 0, 70, 24),
            content: Rect::new(6, 1, 68, 20),
            close:   Some(Rect::new(99, 0, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(90, 23, 8, 1))],
            field: None,
        };

        // 2. Saves: ESC → SavesClose, [X] → SavesClose
        {
            let mut s = AppState::default();
            s.overlays.saves = Some(SavesState { entries: vec![SaveInfo {
                path: PathBuf::from("/tmp/a.lanthorn"), name: "a".into(), turns: 0,
                saved_at: String::new(), location: None, score: None, is_default: false, trigger: crate::archive::SaveTrigger::HostState,
            }], scroll: Default::default() });
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::SavesClose),
                "saves ESC should produce SavesClose, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::SavesClose),
                "saves [X] click should produce SavesClose, got {:?}", x_action);
        }

        // 3. File browser: ESC → FbClose, [X] → FbClose
        {
            let mut s = AppState::default();
            s.overlays.file_browser = Some(crate::state::FileBrowserState::build(
                std::env::temp_dir(), crate::state::FbMode::PickFile));
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::FbClose),
                "file browser ESC should produce FbClose, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::FbClose),
                "file browser [X] click should produce FbClose, got {:?}", x_action);
        }

        // 4. The command band is a dock, not a centered modal with [X]/[Done]
        // chrome, so it's out of scope for this ESC == [X]-click sweep. Its Esc
        // ladder is covered by `escape_ladder_filter_then_phrase_then_close`.

        // 5. Config screen: ESC → ConfigCancel, [X] → ConfigCancel
        {
            let mut s = AppState::default();
            let working = clone_config(&s.config);
            s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::ConfigCancel),
                "config screen ESC should produce ConfigCancel, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::ConfigCancel),
                "config screen [X] click should produce ConfigCancel, got {:?}", x_action);
        }

        // 6. Hotkey dialog: ESC → CloseHotkeyDialog, [X] → CloseHotkeyDialog
        {
            let mut s = AppState::default();
            s.overlays.hotkey_dialog = true;
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::CloseHotkeyDialog),
                "hotkey dialog ESC should produce CloseHotkeyDialog, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::CloseHotkeyDialog),
                "hotkey dialog [X] click should produce CloseHotkeyDialog, got {:?}", x_action);
        }
    }

    /// Assert no modal key handler still binds q to a close action.
    #[test]
    fn no_modal_binds_q_to_close() {
        use crate::state::SavesState;
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;

        // Saves: q → not SavesClose
        {
            let mut s = AppState::default();
            s.overlays.saves = Some(SavesState { entries: vec![SaveInfo {
                path: PathBuf::from("/tmp/a.lanthorn"), name: "a".into(), turns: 0,
                saved_at: String::new(), location: None, score: None, is_default: false, trigger: crate::archive::SaveTrigger::HostState,
            }], scroll: Default::default() });
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::SavesClose),
                "q must not close the saves modal");
        }

        // File browser: q → not FbClose
        {
            let mut s = AppState::default();
            s.overlays.file_browser = Some(crate::state::FileBrowserState::build(
                std::env::temp_dir(), crate::state::FbMode::PickFile));
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::FbClose),
                "q must not close the file browser");
        }

        // Command band: q → not BandClose (it filters instead)
        {
            let mut s = AppState::default();
            open_band(&mut s);
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::BandClose | Action::BandEscape),
                "q must not close the command panel");
        }

        // Config screen: q → not ConfigCancel
        {
            let mut s = AppState::default();
            let working = clone_config(&s.config);
            s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::ConfigCancel),
                "q must not cancel the config screen");
        }

        // Room dock: q → not a dock action
        {
            let mut s = AppState::default();
            s.room_dock.toggle_to(true, true);
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::CloseRoomDock | Action::UnpinRoomDock),
                "q must not close or unpin the room panel");
        }
    }

    /// Regression: a centered modal (config screen) stacked over the map must
    /// swallow all outside-dialog clicks. Without the fix, `is_corner_overlay` was
    /// true even when a centered modal was open, so the outside click fell through
    /// to the room-click / ActivatePane path.
    ///
    /// SQ-0692: the corner overlay in the original scenario was the room panel,
    /// which is gone. The open ROOM DOCK stands in for it — the point of the test
    /// is that a click outside a centered modal reaches nothing, whatever else is
    /// on screen underneath.
    #[test]
    fn centered_modal_swallows_outside_clicks_even_with_the_room_dock_open() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::ConfigScreenState;
        use crate::state::Zoom;

        // Build a real map rect and room_rects so a click at (0,0) would normally
        // pin the dock if the dialog were not open.
        let map_r = map_rect();   // Rect::new(0,0,80,40)
        let story_r = story_rect();
        let live_room_rects = room_rects_for_compact(1, (0, 0), map_r);

        // Confirm that without any dialog open, clicking (0,0) hits the room.
        {
            let s = AppState::default();
            let a = mouse_to_action(&s, mouse_left_click(0, 0), map_r, story_r, &live_room_rects, &None);
            assert!(
                matches!(a, Action::PinRoomDock(1, crate::state::RoomDockView::Info)),
                "sanity: without dialog, a click on a room pins the dock to it, got {:?}", a
            );
        }

        // Now open BOTH the room dock AND the config screen (centered modal).
        let mut state = AppState::default();
        state.zoom = Zoom::Compact;
        state.room_dock.toggle_to(true, true);
        let working = clone_config(&state.config);
        state.overlays.config_screen = Some(ConfigScreenState { working, scroll: Default::default() });

        // The dialog rects represent the config screen's centered dialog (not covering (0,0)).
        let dialog = Some(DialogRects {
            area:    Rect::new(5, 3, 70, 24),
            content: Rect::new(6, 5, 68, 19),
            close:   Some(Rect::new(73, 3, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(65, 26, 8, 1))],
            field: None,
        });

        // Click OUTSIDE the config screen dialog (at (0,0), which is on the room).
        // Must be swallowed — NOT ShowRoomInfo or ActivatePane.
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map_r, story_r, &live_room_rects, &dialog);
        assert!(
            matches!(a, Action::None),
            "outside-config-screen click with the room panel also open must be swallowed (None), got {:?}", a
        );
    }

    // ── SQ-0349: center-map target ───────────────────────────────────────────

    /// Two placed rooms; `current` is #1 at (2,2), #2 sits at (7,7).
    fn recenter_fixture() -> (AppState, Mapper) {
        let mut m = Mapper::default();
        m.graph.upsert_room(1, "Here".into());
        m.graph.upsert_room(2, "There".into());
        m.graph.set_pos(1, (2, 2));
        m.graph.set_pos(2, (7, 7));
        m.graph.set_current(1);
        (AppState::default(), m)
    }

    /// SQ-0361: merging showed the layer the PLAYER was standing in, not the one the rooms went
    /// to — so a merge from a nested layer looked like it had dumped everything on the top layer,
    /// when the rooms had gone to the parent and were merely off-screen.
    #[test]
    fn merging_a_layer_shows_where_the_rooms_went_not_where_the_player_is() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let mut s = AppState::default();
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m); // Cellar -> L1
        m.observe(3, "Vault", Some(Direction::E)); // joins L1
        m.observe(2, "Cellar", Some(Direction::W)); // walk back, so the Vault has a way out to cut
        // Peel the Vault by standing IN it and cutting the way back out: a peel takes the selected
        // room's own side (SQ-0364).
        s.select_room(Some(3));
        apply_action(Action::MoveRegion("new west".into()), &mut s, &mut m); // Vault -> L2
        let vault_layer = m.graph.layer_of(3);
        assert_eq!(m.graph.layers()[&vault_layer].parent, Some(1), "L2 was peeled out of L1");

        // The player walks back up: they are on the TOP layer, nowhere near the Vault.
        m.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m.graph.layer_of(1), mapper::layer::MAIN_LAYER);

        s.set_viewed_layer(Some(vault_layer));
        apply_action(Action::MoveRegion("parent".into()), &mut s, &mut m);

        assert_eq!(m.graph.layer_of(3), 1, "the Vault merges into its parent, the Cellar");
        assert_eq!(
            s.active_layer(&m.graph),
            1,
            "and the map shows the Cellar, where the rooms went — not the player's own top layer"
        );
    }

    /// SQ-0687: the stranded-room story. A room discovered while exploring a maze layer inherits
    /// the maze layer even when it belongs to the surface. A bare merge would fold the peeled
    /// room straight back into the maze (the peel's parent); naming the target sends it home.
    #[test]
    fn merge_layer_with_a_name_sends_a_peeled_room_home_not_back_to_its_parent() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Maze", Some(Direction::Down));
        let mut s = AppState::default();
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m); // Maze -> L1
        let maze = m.graph.layer_of(2);
        // The back door: a surface room discovered FROM the maze inherits the maze layer.
        m.observe(3, "Clearing", Some(Direction::E));
        assert_eq!(m.graph.layer_of(3), maze, "the premise: the new room is stranded on the maze");
        m.observe(2, "Maze", Some(Direction::W)); // walk back so the Clearing has a seam to cut

        s.select_room(Some(3));
        apply_action(Action::MoveRegion("new west".into()), &mut s, &mut m); // Clearing -> L2
        let peeled = m.graph.layer_of(3);
        assert_eq!(m.graph.layers()[&peeled].parent, Some(maze), "a bare merge would round-trip");

        s.set_viewed_layer(Some(peeled));
        apply_action(Action::MoveRegion("main".into()), &mut s, &mut m); // case-insensitive

        assert_eq!(m.graph.layer_of(3), mapper::layer::MAIN_LAYER, "the Clearing lands on Main");
        assert_eq!(m.graph.layer_of(2), maze, "the maze keeps its own rooms");
        assert!(!m.graph.layers().contains_key(&peeled), "the peeled layer is gone");
        assert_eq!(s.active_layer(&m.graph), mapper::layer::MAIN_LAYER, "the view follows the rooms");
    }

    /// A merge aimed at a name that resolves to nothing (or to several layers) must refuse with a
    /// message and move nothing — not guess.
    #[test]
    fn merge_layer_refuses_unknown_and_ambiguous_names_without_moving_anything() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let mut s = AppState::default();
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m); // Cellar -> L1
        let cellar = m.graph.layer_of(2);
        s.set_viewed_layer(Some(cellar));

        apply_action(Action::MoveRegion("Attic".into()), &mut s, &mut m);
        assert_eq!(m.graph.layer_of(2), cellar, "an unknown name moves nothing");
        let msg = s.notifications.latest_text().expect("the refusal must not be silent").to_string();
        assert!(msg.contains("no layer named 'Attic'"), "says what was wrong: {msg:?}");

        // Two layers named "Cellar": the peel named L1 after room 2's label, so rename Main too.
        m.graph.set_layer_name(mapper::layer::MAIN_LAYER, "Cellar".into());
        apply_action(Action::MoveRegion("Cellar".into()), &mut s, &mut m);
        assert_eq!(m.graph.layer_of(2), cellar, "an ambiguous name moves nothing");
        let msg = s.notifications.latest_text().expect("the refusal must not be silent").to_string();
        assert!(msg.contains("rename one first"), "says how to fix it: {msg:?}");
    }

    // ── SQ-0439: peel and merge are one verb ─────────────────────────────────

    /// The whole insight, at the command level: the SAME action, told a different destination,
    /// carves a layer or folds one away. Nothing distinguishes them but the argument.
    #[test]
    fn one_verb_both_carves_a_layer_and_folds_it_back() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let mut s = AppState::default();

        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m);
        let cellar = m.graph.layer_of(2);
        assert_ne!(cellar, mapper::layer::MAIN_LAYER, "`new` carved the cellar off");
        assert_eq!(s.active_layer(&m.graph), cellar, "and the view follows the rooms");

        // A whole layer, aimed at an EXISTING one. The old `WholeLayer` refusal blocked exactly
        // this shape; against a named target it was never an error, only a merge.
        apply_action(Action::MoveRegion("main".into()), &mut s, &mut m);
        assert_eq!(m.graph.layer_of(2), mapper::layer::MAIN_LAYER, "`main` folded it back");
        assert!(!m.graph.layers().contains_key(&cellar), "and the emptied layer is gone");
        assert_eq!(s.active_layer(&m.graph), mapper::layer::MAIN_LAYER, "the view follows again");
    }

    /// `MainSource` ("main cannot be a merge source") generalised to "you cannot EMPTY Main".
    /// Moving part of Main out is legal and always was; moving every room out is not.
    #[test]
    fn a_region_may_leave_main_but_main_may_not_be_emptied() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let mut s = AppState::default();
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m); // Cellar -> L1
        let cellar = m.graph.layer_of(2);
        m.observe(1, "Hall", Some(Direction::Up)); // back upstairs, onto Main
        m.observe(3, "Study", Some(Direction::E)); // a second room on Main
        m.observe(1, "Hall", Some(Direction::W)); // and back, so the passage has both ends

        // Part of Main leaves: legal, and always was. The Study's own side of the Hall↔Study
        // passage goes down to the Cellar layer, and Main keeps the Hall.
        s.select_room(Some(3));
        apply_action(Action::MoveRegion("Cellar west".into()), &mut s, &mut m);
        assert_eq!(m.graph.layer_of(3), cellar, "a sub-region of Main moved onto another layer");
        assert_eq!(m.graph.rooms_in_layer(mapper::layer::MAIN_LAYER), vec![1], "Main keeps the Hall");

        // What is LEFT of Main is now its whole contents: refused, and it says why — this is
        // about the MOVE refusing, not about which rooms were chosen.
        s.select_room(Some(1));
        apply_action(Action::MoveRegion("Cellar".into()), &mut s, &mut m);
        assert_eq!(m.graph.layer_of(1), mapper::layer::MAIN_LAYER, "Main was not emptied");
        let msg = s.notifications.latest_text().expect("the refusal must not be silent").to_string();
        assert!(msg.contains("no rooms at all"), "says Main may not be emptied: {msg:?}");
        assert!(m.graph.layers().contains_key(&cellar), "and nothing else changed either");
    }

    /// The destination and the seam share one argument string, so the split must not guess: a
    /// LIVE layer name wins over a trailing word that merely looks like a direction.
    #[test]
    fn a_layer_name_outranks_a_trailing_direction() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        let north = m.graph.new_layer(Some(mapper::layer::MAIN_LAYER), "Dead End North".into());
        let plain = m.graph.new_layer(Some(mapper::layer::MAIN_LAYER), "Dead End".into());

        assert_eq!(
            parse_move_region_arg(&m.graph, "Dead End North"),
            Ok((MoveDest::Layer(north), None)),
            "a layer really called 'Dead End North' resolves whole"
        );
        assert_eq!(
            parse_move_region_arg(&m.graph, "Dead End south"),
            Ok((MoveDest::Layer(plain), Some(Direction::S))),
            "and only when the whole phrase names nothing is the last word a seam"
        );
        assert_eq!(parse_move_region_arg(&m.graph, "new"), Ok((MoveDest::New, None)));
        assert_eq!(
            parse_move_region_arg(&m.graph, "NEW nw"),
            Ok((MoveDest::New, Some(Direction::NW))),
            "destinations are case-insensitive, and the seam takes the game's own vocabulary"
        );
        assert_eq!(parse_move_region_arg(&m.graph, "parent"), Ok((MoveDest::Parent, None)));
        // Either half may be left out, so a refusal can suggest a fix for just the ambiguity it
        // is complaining about instead of demanding both at once (SQ-0439).
        assert_eq!(parse_move_region_arg(&m.graph, ""), Ok((MoveDest::Auto, None)));
        assert_eq!(
            parse_move_region_arg(&m.graph, "west"),
            Ok((MoveDest::Auto, Some(Direction::W))),
            "a lone direction is a seam, with the destination left to the auto-pick"
        );
        assert!(parse_move_region_arg(&m.graph, "Attic").is_err(), "an unknown name is refused");
        assert!(
            parse_move_region_arg(&m.graph, "Attic east").is_err(),
            "and so is an unknown name with a seam after it — never silently a new layer"
        );
    }

    // ── SQ-0360: cut at a named seam, and say why a move refused ─────────────

    /// A layer with no portal seam in it (Zork's Cellar: 35 rooms of solid compass maze) could not
    /// be divided at all, and the refusal was SILENT — the command simply did nothing.
    #[test]
    fn a_direction_cuts_a_layer_that_refuses_the_plain_move() {
        let mut m = Mapper::default();
        m.observe(1, "Round Room", None);
        m.observe(2, "Loud Room", Some(Direction::E));
        m.observe(3, "Damp Cave", Some(Direction::E));
        let mut s = AppState::default();
        s.select_room(Some(1));

        // Plain peel: one connected region, so it refuses — and now explains itself.
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m);
        assert_eq!(m.graph.layers().len(), 1, "nothing peeled");
        let msg = s.notifications.latest_text().expect("a refusal must not be silent").to_string();
        assert!(msg.contains("one connected region"), "says why: {msg:?}");
        assert!(msg.contains("<direction>"), "and points at the way forward: {msg:?}");

        // Naming the seam cuts there — and posts no complaint.
        let before = s.notifications.history().len();
        apply_action(Action::MoveRegion("new east".into()), &mut s, &mut m);
        assert_eq!(s.notifications.history().len(), before, "no complaint when it works");
        let new = s.viewed_layer.expect("the peeled layer is now in view");
        // #1 is selected, so #1's own side leaves — the same side a bare `move-region` would
        // take (SQ-0364). The far side is what stays.
        assert_eq!(m.graph.rooms_in_layer(new), vec![1], "the selected room's side leaves");
        assert_eq!(m.graph.rooms_in_layer(0), vec![2, 3], "the far side stays put");
    }

    #[test]
    fn move_region_explains_a_passage_that_is_not_a_seam() {
        // A→B directly and A→C→B as well: cutting A-B separates nothing.
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        m.graph.add_edge(1, Direction::N, 3);
        m.graph.upsert_room(3, "C".into());
        m.graph.add_edge(3, Direction::E, 2);
        let mut s = AppState::default();
        s.select_room(Some(1));

        apply_action(Action::MoveRegion("new east".into()), &mut s, &mut m);
        let msg = s.notifications.latest_text().expect("refusal must speak").to_string();
        assert!(msg.contains("not a boundary"), "{msg:?}");
        assert_eq!(m.graph.layers().len(), 1, "nothing peeled");

        // And a direction with no passage at all is a different complaint.
        apply_action(Action::MoveRegion("new west".into()), &mut s, &mut m);
        let msg = s.notifications.latest_text().expect("refusal must speak").to_string();
        assert!(msg.contains("no W passage"), "{msg:?}");
    }

    // ── SQ-0439: the three seam tiers, anchored on the selected room ─────────

    /// The user's Adventure map, in miniature: walk SOUTH out of the Long Hall into the maze,
    /// whose only ways out are DOWN back to the hall and an Unknown link to the Inside Building.
    fn advent_maze() -> (AppState, Mapper) {
        let mut m = Mapper::default();
        m.observe(1, "Inside Building", None);
        m.observe(2, "At West End of Long Hall", Some(Direction::Down));
        m.observe(3, "Maze", Some(Direction::S)); // the passage the peel must cut
        m.graph.upsert_room(4, "Maze".into());
        for (a, d, b) in [
            (3, Direction::Down, 2), // the way back: one-way, and a portal at that
            (3, Direction::N, 4),    // maze innards, all sharing the name
            (4, Direction::S, 3),
            (4, Direction::Unknown, 1), // the maze's other exit — also a portal
        ] {
            m.graph.add_edge(a, d, b);
        }
        (AppState::default(), m)
    }

    /// A hall whose ONLY way into the maze is a one-way south passage, with the way back a portal.
    /// The compass walk therefore reaches the hall and covers the whole layer, so this is the
    /// fixture for tier 2 — and for the complaint that started SQ-0439, since no direction OUT of
    /// the maze names the way in.
    fn one_way_maze() -> (AppState, Mapper) {
        let mut m = Mapper::default();
        for (id, n) in [(1, "At West End of Long Hall"), (2, "Maze"), (3, "Maze"), (4, "Maze")] {
            m.graph.upsert_room(id, n.into());
        }
        for (a, d, b) in [
            (1, Direction::S, 2),    // walked in — one way, no reciprocal to name
            (2, Direction::Down, 1), // the way back: a portal, and not the reciprocal
            (2, Direction::N, 3),
            (3, Direction::S, 2),
            (2, Direction::E, 4),
            (4, Direction::N, 3),
        ] {
            m.graph.add_edge(a, d, b);
        }
        m.graph.set_current(2);
        (AppState::default(), m)
    }

    /// A chain with no portal in it: every room's compass walk is the whole layer, so tier 1 can
    /// never fire and tier 2/3 do all the work.
    fn corridor() -> (AppState, Mapper) {
        let mut m = Mapper::default();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C"), (4, "D")] {
            m.graph.upsert_room(id, n.into());
        }
        for (a, b) in [(1, 2), (2, 3), (3, 4)] {
            m.graph.add_edge(a, Direction::E, b);
            m.graph.add_edge(b, Direction::W, a);
        }
        m.graph.set_current(1);
        (AppState::default(), m)
    }

    /// TIER 1. A region the portals already bound is the answer, and the seam search is never
    /// reached — no passage is cut, so nothing is announced as cut. It is also the whole reason a
    /// floor needs no input beyond which room was picked.
    #[test]
    fn a_portal_bounded_region_never_looks_for_a_seam() {
        let (mut s, mut m) = advent_maze();
        s.select_room(Some(3));
        apply_action(Action::MoveRegion(String::new()), &mut s, &mut m);
        let new = s.viewed_layer.expect("the region moved to a layer of its own");
        assert_eq!(
            m.graph.rooms_in_layer(new),
            vec![2, 3, 4],
            "the compass walk stops at the portals, and takes everything inside them"
        );
        assert_eq!(m.graph.rooms_in_layer(0), vec![1], "the building beyond the portal stays");
        assert_eq!(
            s.notifications.latest_text(),
            None,
            "nothing was cut, so nothing is reported cut — and nothing refused"
        );
    }

    /// TIER 2, and the complaint that started this: the way in is ONE-WAY, so no direction out of
    /// the maze names it. Exactly one passage leads in, so it is cut without being asked for — and
    /// said out loud, because a boundary chosen for the player must not be invisible.
    #[test]
    fn the_one_way_passage_in_is_found_cut_and_named() {
        let (mut s, mut m) = one_way_maze();
        s.select_room(Some(2));
        apply_action(Action::MoveRegion(String::new()), &mut s, &mut m);

        let new = s.viewed_layer.expect("the maze moved to a layer of its own");
        assert_eq!(m.graph.rooms_in_layer(new), vec![2, 3, 4], "the maze leaves, all three rooms");
        assert_eq!(m.graph.rooms_in_layer(0), vec![1], "the hall stays put");
        let msg = s.notifications.latest_text().expect("an auto-picked seam must speak").to_string();
        assert!(msg.contains("cut the S passage"), "says which passage: {msg:?}");
        assert!(msg.contains("Long Hall"), "and where it came from: {msg:?}");
    }

    /// TIER 3. Two ways in, each cutting a different half of the map: no auto-pick is honest here,
    /// so the prompt opens on the candidates and nothing moves until one is chosen (SQ-0439).
    #[test]
    fn several_ways_in_are_offered_rather_than_guessed() {
        use crate::state::{RegionOption, RegionPromptKind};
        let (mut s, mut m) = corridor();
        s.select_room(Some(2));
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m);

        assert_eq!(m.graph.layers().len(), 1, "an ambiguous seam moves nothing on its own");
        let p = s.overlays.region_prompt.as_ref().expect("the picker opens");
        assert!(
            matches!(p.kind, RegionPromptKind::PickSeam { room: 2, dest: MoveDest::New }),
            "it asks about the SELECTED room, and carries the destination already named"
        );
        let labels: Vec<&str> = p
            .options
            .iter()
            .map(|o| match o {
                RegionOption::Seam { label, .. } => label.as_str(),
                _ => panic!("a seam pick offers seams"),
            })
            .collect();
        assert!(labels.iter().any(|l| l.starts_with("e from A")), "{labels:?}");
        assert!(labels.iter().any(|l| l.starts_with("w from C")), "{labels:?}");
    }

    /// Choosing from that picker cuts the passage chosen and nothing else — and because the
    /// destination rode along in the prompt, answering the seam question does not reopen the
    /// other one.
    #[test]
    fn choosing_a_seam_from_the_picker_makes_the_move() {
        use crate::state::RegionPromptAct;
        let (mut s, mut m) = corridor();
        s.select_room(Some(2));
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m);
        // Option 0 is `e from A`; option 1 is `w from C`. Choose the second.
        s.overlays.region_prompt.as_mut().unwrap().choice = 1;
        apply_region_prompt(&mut s, &mut m, RegionPromptAct::Accept);

        assert!(s.overlays.region_prompt.is_none(), "answering closes the prompt");
        let new = s.viewed_layer.expect("the chosen seam cut");
        assert_eq!(
            m.graph.rooms_in_layer(new),
            vec![1, 2],
            "cutting C→W→B keeps B's side: A and B travel"
        );
        assert_eq!(m.graph.rooms_in_layer(0), vec![3, 4], "C and D stay behind");
    }

    /// Esc on a seam pick decides nothing and remembers nothing — a pick is not a suggestion.
    #[test]
    fn dismissing_a_seam_pick_moves_nothing() {
        use crate::state::RegionPromptAct;
        let (mut s, mut m) = corridor();
        s.select_room(Some(2));
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m);
        apply_region_prompt(&mut s, &mut m, RegionPromptAct::Dismiss);
        assert!(s.overlays.region_prompt.is_none());
        assert_eq!(m.graph.layers().len(), 1, "nothing moved");
        assert!(m.graph.seam_decisions().is_empty(), "and nothing was written down");
    }

    /// The command that refusal suggests must mean what the list said it meant: a direction picks
    /// the passage IN of that direction, taking the selected room's side — not the passage of the
    /// same name leading out, which cuts the opposite half.
    #[test]
    fn naming_a_way_in_cuts_that_one_and_takes_the_selection_with_it() {
        let (mut s, mut m) = corridor();
        s.select_room(Some(2));
        apply_action(Action::MoveRegion("new e".into()), &mut s, &mut m);
        let new = s.viewed_layer.expect("the named seam cut");
        assert_eq!(m.graph.rooms_in_layer(new), vec![2, 3, 4], "the A→B passage was the seam");
        assert_eq!(m.graph.rooms_in_layer(0), vec![1], "A is what stayed behind");
    }

    /// A direction OUT of the room is still nameable, and is the only way to name a one-way exit.
    /// It is the fallback, reached when no passage leads IN that way.
    #[test]
    fn a_direction_with_no_way_in_falls_back_to_the_passage_out() {
        let (mut s, mut m) = corridor();
        s.select_room(Some(1)); // A: nothing leads in from the east, but A leads out east
        apply_action(Action::MoveRegion("new e".into()), &mut s, &mut m);
        let new = s.viewed_layer.expect("the outbound seam cut");
        assert_eq!(m.graph.rooms_in_layer(new), vec![1], "A's own side leaves, as it always has");
    }

    /// THE REAL DEAD END (SQ-0439). A maze can perfectly well have two rooms whose SOUTH exits
    /// both land here — Adventure's does — and then no direction separates them, so no re-issued
    /// command can resolve it and a refusal that names them is a dead end by construction. The
    /// picker is the only answer that always exists, and it must offer BOTH passages that share
    /// the direction.
    #[test]
    fn two_passages_sharing_a_direction_are_still_pickable() {
        use crate::state::{RegionOption, RegionPromptAct, RegionPromptKind};
        let (mut s, mut m) = advent_maze(); // 2→S→3 and 4→S→3 are both boundaries
        s.select_room(Some(3));
        apply_action(Action::MoveRegion("new south".into()), &mut s, &mut m);
        assert_eq!(m.graph.layers().len(), 1, "nothing moves until one is chosen");
        let p = s.overlays.region_prompt.as_ref().expect("the picker opens");
        assert!(matches!(p.kind, RegionPromptKind::PickSeam { room: 3, .. }));
        let labels: Vec<&str> = p
            .options
            .iter()
            .map(|o| match o {
                RegionOption::Seam { label, .. } => label.as_str(),
                _ => panic!("a seam pick offers seams"),
            })
            .collect();
        assert_eq!(labels.len(), 2, "both S passages are offered: {labels:?}");
        assert!(labels.iter().any(|l| l.starts_with("s from At West End of Long Hall")), "{labels:?}");
        assert!(labels.iter().any(|l| l.starts_with("s from Maze")), "{labels:?}");

        // And picking one of them actually moves rooms, which is the whole point: this case had
        // no route through the command line at all.
        apply_region_prompt(&mut s, &mut m, RegionPromptAct::Accept);
        assert_eq!(m.graph.layers().len(), 2, "the chosen passage was cut");
    }

    /// The consequence the design accepts openly: a room in the MIDDLE of a maze has no inbound
    /// boundary, so there is nothing to auto-cut and the whole-layer region falls through to the
    /// move's own refusal. It never guesses a seam.
    #[test]
    fn a_room_mid_maze_falls_back_instead_of_guessing() {
        let (mut s, mut m) = one_way_maze();
        s.select_room(Some(3)); // every way in has a way round
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m);
        assert_eq!(m.graph.layers().len(), 1, "nothing moved");
        let msg = s.notifications.latest_text().expect("refusal must speak").to_string();
        assert!(msg.contains("one connected region"), "{msg:?}");
    }

    // ── SQ-0439: the destination auto-picks on the same rule as the seam ─────

    /// With Main the only layer there is, `new` is the only place a region can go — so the bare
    /// command says nothing and simply does it.
    #[test]
    fn a_bare_move_picks_the_only_destination_there_is() {
        let (mut s, mut m) = one_way_maze();
        s.select_room(Some(2));
        assert_eq!(m.graph.layers().len(), 1, "the premise: only Main exists");
        apply_action(Action::MoveRegion(String::new()), &mut s, &mut m);
        let new = s.viewed_layer.expect("a layer was minted without being asked for");
        assert_ne!(new, mapper::layer::MAIN_LAYER);
        assert_eq!(m.graph.rooms_in_layer(new), vec![2, 3, 4]);
    }

    /// Add a second layer and `new` is no longer the only answer — the rooms could equally fold
    /// into it — so the bare command asks instead of guessing, and offers what it is choosing
    /// between (SQ-0439).
    #[test]
    fn a_bare_move_asks_once_more_than_one_destination_is_possible() {
        use crate::state::{RegionOption, RegionPromptKind};
        let (mut s, mut m) = one_way_maze();
        m.graph.new_layer(Some(mapper::layer::MAIN_LAYER), "Attic".into());
        s.select_room(Some(2));
        apply_action(Action::MoveRegion(String::new()), &mut s, &mut m);

        assert_eq!(m.graph.layer_of(2), mapper::layer::MAIN_LAYER, "an ambiguous target moves nothing");
        let p = s.overlays.region_prompt.as_ref().expect("the picker opens");
        assert!(matches!(p.kind, RegionPromptKind::PickDest { .. }));
        let labels: Vec<&str> = p
            .options
            .iter()
            .map(|o| match o {
                RegionOption::Dest { label, .. } => label.as_str(),
                _ => panic!("a destination pick offers destinations"),
            })
            .collect();
        assert!(labels.contains(&"a new layer"), "lists the fresh layer: {labels:?}");
        assert!(labels.contains(&"Attic"), "and every layer that could take them: {labels:?}");
        assert_eq!(p.rooms.len(), 3, "and hands over every room that travels: {:?}", p.rooms);
    }

    /// Choosing from the destination picker completes the move the command started.
    #[test]
    fn choosing_a_destination_completes_the_move() {
        use crate::state::{RegionOption, RegionPromptAct};
        let (mut s, mut m) = one_way_maze();
        let attic = m.graph.new_layer(Some(mapper::layer::MAIN_LAYER), "Attic".into());
        s.select_room(Some(2));
        apply_action(Action::MoveRegion(String::new()), &mut s, &mut m);
        let p = s.overlays.region_prompt.as_mut().unwrap();
        p.choice = p
            .options
            .iter()
            .position(|o| matches!(o, RegionOption::Dest { label, .. } if label == "Attic"))
            .expect("Attic is on offer");
        apply_region_prompt(&mut s, &mut m, RegionPromptAct::Accept);
        assert_eq!(m.graph.rooms_in_layer(attic), vec![2, 3, 4], "the rooms went where they were sent");
    }

    /// The two questions stay separate: a seam the player already named resolves the region, and
    /// the destination question is then asked on its own rather than re-opening the seam.
    #[test]
    fn a_named_seam_survives_into_the_destination_question() {
        use crate::state::RegionPromptKind;
        let (mut s, mut m) = corridor();
        m.graph.new_layer(Some(mapper::layer::MAIN_LAYER), "Attic".into());
        s.select_room(Some(2));
        apply_action(Action::MoveRegion("e".into()), &mut s, &mut m);
        let p = s.overlays.region_prompt.as_ref().expect("only the destination is still open");
        let RegionPromptKind::PickDest { region, cut } = &p.kind else {
            panic!("the seam resolved; only the target did not: {:?}", p.kind)
        };
        assert_eq!(region.rooms.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4], "the named seam cut");
        assert_eq!(*cut, None, "and a seam the player named is not reported back at them");
    }

    // ── SQ-0439: the map's own suggestion, and the prompt that carries it ────

    /// A manor whose four-room cellar hangs off one trapdoor, walked down and back up again —
    /// the structural trigger's canonical shape. The player ends at the foot of the stairs, so
    /// one more `Up` is the return crossing the detector waits for.
    fn manor() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Study", Some(Direction::E));
        m.observe(1, "Hall", Some(Direction::W));
        m.observe(3, "Cellar", Some(Direction::Down));
        m.observe(4, "Wine Cellar", Some(Direction::E));
        m.observe(5, "Vault", Some(Direction::E));
        m.observe(6, "Crypt", Some(Direction::E));
        m.observe(5, "Vault", Some(Direction::W));
        m.observe(4, "Wine Cellar", Some(Direction::W));
        m.observe(3, "Cellar", Some(Direction::W));
        m
    }

    /// Zork I's shape, walked to the moment the descent trigger speaks (SQ-0853): five surface
    /// rooms, a trapdoor that bars itself behind you, and four rooms below it with no way back up.
    /// The seam is `Living Room -down-> Cellar`, and the Living Room STAYS PUT — which is the whole
    /// difference between this reading and [`manor`]'s.
    fn barred_trapdoor() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "North of House", Some(Direction::N));
        m.observe(3, "Behind House", Some(Direction::E));
        m.observe(4, "Kitchen", Some(Direction::W));
        m.observe(5, "Living Room", Some(Direction::W));
        m.observe(6, "Cellar", Some(Direction::Down));
        m.observe(7, "East of Chasm", Some(Direction::S));
        m.observe(8, "Gallery", Some(Direction::E));
        m.observe(9, "Studio", Some(Direction::N));
        m
    }

    /// Draw whatever prompt is open into an 80x24 terminal and hand back the screen as one string.
    fn drawn(state: &AppState) -> String {
        use ratatui::{backend::TestBackend, Terminal};
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| {
                crate::render::region_prompt::draw_region_prompt(state, f.area(), f.buffer_mut());
            })
            .unwrap();
        terminal.backend().buffer().content().iter().flat_map(|c| c.symbol().chars()).collect()
    }

    /// A hall with a maze through its south door — the semantic trigger's shape. Stops one step
    /// short so the caller walks in.
    fn maze_doorway() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "At West End of Long Hall", None);
        m.observe(7, "Storeroom", Some(Direction::N));
        m.observe(1, "At West End of Long Hall", Some(Direction::S));
        m
    }

    /// The map speaks: climbing back out of the cellar opens the prompt, and it says what would
    /// move and where it could go. Nothing has moved yet — detect and SUGGEST.
    #[test]
    fn climbing_out_of_the_cellar_opens_the_prompt() {
        use crate::state::{RegionOption, RegionPromptKind};
        use mapper::layer::MoveTarget;
        let mut s = AppState::default();
        let mut m = manor();
        m.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s, &mut m);

        let p = s.overlays.region_prompt.as_ref().expect("the map has something to say");
        let RegionPromptKind::Suggest { trigger, seam, region } = &p.kind else {
            panic!("a structural suggestion: {:?}", p.kind)
        };
        assert_eq!(*trigger, mapper::suggest::Trigger::Structural);
        assert_eq!(seam.from, 3);
        assert_eq!(region.rooms.iter().copied().collect::<Vec<_>>(), vec![3, 4, 5, 6]);
        assert_eq!(
            p.rooms,
            ["Cellar", "Wine Cellar", "Vault", "Crypt"],
            "every room that travels, named and in region order"
        );
        assert_eq!(
            p.options,
            vec![RegionOption::Dest { label: "a new layer".into(), target: MoveTarget::New }],
            "with Main un-emptiable, a fresh layer is the only place they can go"
        );
        assert_eq!(m.graph.layers().len(), 1, "and nothing has moved: it only suggests");
    }

    /// SQ-1137: the same crossing, with the map panel hidden. The prompt asks which LAYER a set of
    /// rooms belongs on, and every word of that question is about a pane the player has closed.
    ///
    /// The second half is the half that pins the guard's POSITION. Showing the map again and
    /// calling once more opens the prompt, which is only possible if the first call declined to
    /// take the suggestion rather than taking it and dropping it on the floor — a distinction
    /// invisible from the first assertion alone, and the difference between "not now" and "never".
    #[test]
    fn a_hidden_map_is_not_asked_how_to_lay_itself_out() {
        use crate::state::Layout;
        let mut s = AppState::default();
        s.layout = Layout::TranscriptFull;
        let mut m = manor();
        m.observe(1, "Hall", Some(Direction::Up));

        offer_layer_suggestion(&mut s, &mut m);
        assert!(
            s.overlays.region_prompt.is_none(),
            "a player who closed the map is not interrupted about how to lay it out"
        );

        s.layout = Layout::Split;
        offer_layer_suggestion(&mut s, &mut m);
        assert!(
            s.overlays.region_prompt.is_some(),
            "and the suggestion was declined, not consumed: it is still there to raise"
        );
    }

    // ── SQ-0858: the sentence, spelled out and pointed the right way round ────

    /// The reported defect, and the headline of it: the prompt read **"You came d out of Living
    /// Room."** — a `SeamKey` ordering tag printed at the player. The direction is a WORD now, and
    /// it reaches the screen.
    #[test]
    fn the_prompt_spells_the_direction_out_instead_of_printing_its_key() {
        let mut s = AppState::default();
        let mut m = barred_trapdoor();
        offer_layer_suggestion(&mut s, &mut m);
        let screen = drawn(&s);
        assert!(screen.contains("DOWN"), "the passage is spelled out: {screen:?}");
        assert!(
            !screen.contains("You came d "),
            "and never as its short tag — that was the report: {screen:?}"
        );
    }

    /// The DESCENT reading. `Trigger::Structural` fires here from inside a region there is no way
    /// out of, so the seam is the way IN and the Living Room is the room ABOVE — it is not one of
    /// the rooms that would move, and the sentence has to say so.
    #[test]
    fn a_descent_prompt_reads_as_coming_down_from_the_room_above() {
        use crate::state::RegionPromptKind;
        let mut s = AppState::default();
        let mut m = barred_trapdoor();
        offer_layer_suggestion(&mut s, &mut m);

        let p = s.overlays.region_prompt.as_ref().expect("the fourth cellar room speaks");
        let RegionPromptKind::Suggest { seam, region, trigger } = &p.kind else {
            panic!("a structural suggestion: {:?}", p.kind)
        };
        assert_eq!(*trigger, mapper::suggest::Trigger::Structural);
        assert_eq!(seam.from, 5, "the trapdoor's OUTSIDE end, the Living Room");
        assert!(!region.rooms.contains(&seam.from), "which does not travel with the region");
        assert_eq!(p.rooms, ["Cellar", "East of Chasm", "Gallery", "Studio"]);

        let screen = drawn(&s);
        assert!(
            screen.contains("You came DOWN from Living Room."),
            "the descent reading: you are still down there, having come in that way: {screen:?}"
        );
        assert!(
            !screen.contains("out of"),
            "and NOT the return reading, which is what it used to say: {screen:?}"
        );
    }

    /// The RETURN reading, from the very same `Trigger::Structural`. Here the seam is the way OUT
    /// and the Cellar is INSIDE the region — one of the rooms that would move — so the sentence
    /// points the other way. Two shapes, one trigger, and the seam is what tells them apart.
    #[test]
    fn a_climb_out_prompt_reads_as_coming_up_out_of_the_room_below() {
        use crate::state::RegionPromptKind;
        let mut s = AppState::default();
        let mut m = manor();
        m.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s, &mut m);

        let p = s.overlays.region_prompt.as_ref().expect("climbing out speaks");
        let RegionPromptKind::Suggest { seam, region, .. } = &p.kind else {
            panic!("a structural suggestion: {:?}", p.kind)
        };
        assert!(region.rooms.contains(&seam.from), "the seam's end is one of the rooms that move");

        let screen = drawn(&s);
        assert!(
            screen.contains("You came UP out of Cellar."),
            "the return reading: you have just left the region by that passage: {screen:?}"
        );
        assert!(!screen.contains("from Cellar"), "not the descent reading: {screen:?}");
    }

    /// Both readings then say the same true thing about the rooms. The line this replaced —
    /// "Those 4 rooms have no other way in" — is false of any cellar with a second trapdoor;
    /// what `planar_region` actually promises is that no COMPASS passage crosses the boundary.
    #[test]
    fn both_structural_readings_claim_only_what_the_region_walk_proves() {
        let mut s = AppState::default();
        let mut m = barred_trapdoor();
        offer_layer_suggestion(&mut s, &mut m);
        let descent = drawn(&s);

        let mut s2 = AppState::default();
        let mut m2 = manor();
        m2.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s2, &mut m2);
        let ret = drawn(&s2);

        for screen in [&descent, &ret] {
            assert!(
                screen.contains("No compass passage reaches those 4 rooms."),
                "the claim the walk supports: {screen:?}"
            );
            assert!(
                !screen.contains("no other way in"),
                "and not the one it does not: {screen:?}"
            );
        }
    }

    /// The rooms reach the screen as BULLETS, one to a row, under a count header — the list the
    /// report said was cut off.
    #[test]
    fn a_suggestion_lists_its_rooms_as_bullets_on_screen() {
        let mut s = AppState::default();
        let mut m = barred_trapdoor();
        offer_layer_suggestion(&mut s, &mut m);
        let screen = drawn(&s);
        assert!(screen.contains("4 rooms:"), "the count: {screen:?}");
        for room in ["Cellar", "East of Chasm", "Gallery", "Studio"] {
            assert!(screen.contains(&format!("• {room}")), "{room} is bulleted: {screen:?}");
        }
    }

    /// Accepting a NAME-triggered suggestion also flags the layer as a maze — the player confirmed
    /// it by accepting a prompt that said so.
    #[test]
    fn accepting_a_maze_suggestion_flags_the_layer() {
        use crate::state::RegionPromptAct;
        let mut s = AppState::default();
        let mut m = maze_doorway();
        m.observe(2, "Maze", Some(Direction::S));
        offer_layer_suggestion(&mut s, &mut m);
        assert!(
            matches!(
                s.overlays.region_prompt.as_ref().map(|p| &p.kind),
                Some(crate::state::RegionPromptKind::Suggest {
                    trigger: mapper::suggest::Trigger::Name,
                    ..
                })
            ),
            "the name is the trigger"
        );
        apply_region_prompt(&mut s, &mut m, RegionPromptAct::Accept);

        let landed = s.viewed_layer.expect("the maze moved to a layer of its own");
        assert_eq!(m.graph.rooms_in_layer(landed), vec![2]);
        assert!(m.graph.layer_is_maze(landed), "accepting a prompt that said 'maze' sets the flag");
    }

    /// A STRUCTURAL accept sets nothing. A cellar is not a maze, and the flag freezes a layer's
    /// geometry and moves its default view — far too much to infer from a trapdoor.
    #[test]
    fn accepting_a_structural_suggestion_flags_nothing() {
        use crate::state::RegionPromptAct;
        let mut s = AppState::default();
        let mut m = manor();
        m.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s, &mut m);
        apply_region_prompt(&mut s, &mut m, RegionPromptAct::Accept);

        let landed = s.viewed_layer.expect("the cellar moved");
        assert_eq!(m.graph.rooms_in_layer(landed), vec![3, 4, 5, 6]);
        assert!(!m.graph.layer_is_maze(landed), "a cellar is not a maze");
    }

    /// "Not now" is not "no": it re-arms, so the very same climb asks again. Esc means this too,
    /// which is why the prompt has no Cancel.
    #[test]
    fn not_now_re_arms_the_seam() {
        use crate::state::RegionPromptAct;
        use mapper::suggest::{SeamDecision, SeamKey};
        let mut s = AppState::default();
        let mut m = manor();
        m.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s, &mut m);
        apply_region_prompt(&mut s, &mut m, RegionPromptAct::Defer);

        let seam = SeamKey { from: 3, dir: Direction::Up };
        assert!(s.overlays.region_prompt.is_none(), "answering closes the prompt");
        assert_eq!(m.graph.seam_decision(seam), SeamDecision::Deferred);
        assert_eq!(m.graph.layers().len(), 1, "and nothing moved");

        // Down and back up again — and it asks a second time, which is the whole difference
        // between "not now" and "never".
        m.observe(3, "Cellar", Some(Direction::Down));
        m.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s, &mut m);
        assert!(s.overlays.region_prompt.is_some(), "a deferred seam speaks up on the next crossing");
    }

    /// "Never" silences that passage for good — a prompt that comes back on the next step teaches
    /// the player to dismiss it blind, which is the failure this whole design exists to avoid.
    #[test]
    fn never_silences_the_seam_for_good() {
        use crate::state::RegionPromptAct;
        use mapper::suggest::{SeamDecision, SeamKey};
        let mut s = AppState::default();
        let mut m = manor();
        m.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s, &mut m);
        apply_region_prompt(&mut s, &mut m, RegionPromptAct::Never);

        assert_eq!(m.graph.seam_decision(SeamKey { from: 3, dir: Direction::Up }), SeamDecision::Ignored);
        m.observe(3, "Cellar", Some(Direction::Down));
        m.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s, &mut m);
        assert!(s.overlays.region_prompt.is_none(), "and it never asks again");
    }

    /// It must not steal focus mid-turn: a modal the player asked for outranks a suggestion nobody
    /// did. Dropping the suggestion costs nothing, because declining to show it writes nothing
    /// down and the same crossing raises it again.
    #[test]
    fn a_suggestion_never_shoulders_in_front_of_an_open_modal() {
        let mut s = AppState::default();
        let mut m = manor();
        s.overlays.quit_dialog = true;
        m.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s, &mut m);
        assert!(s.overlays.region_prompt.is_none(), "the quit dialog keeps the floor");
        assert!(m.graph.seam_decisions().is_empty(), "and nothing was decided on the player's behalf");

        s.overlays.quit_dialog = false;
        m.observe(3, "Cellar", Some(Direction::Down));
        m.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut s, &mut m);
        assert!(s.overlays.region_prompt.is_some(), "so the next crossing still asks");
    }

    #[test]
    fn center_map_uses_the_selected_room() {
        let (mut s, mut m) = recenter_fixture();
        s.select_room(Some(2));
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((7, 7), 80, 24);
        assert_eq!(s.scroll, want.scroll, "centred on the SELECTED room, not the current one");
    }

    #[test]
    fn center_map_measures_the_real_map_pane_not_a_guess() {
        // SQ-0349: `apply_recenter` assumed 80×24, because `apply_action` cannot reach the run
        // loop's pane rects. `recenter_on` divides the pane by the zoom step to place the view,
        // so on any other pane size the target landed off-centre — which is what made pressing
        // `c` look like it centred on something other than the selected room.
        let (mut s, mut m) = recenter_fixture();
        s.select_room(Some(2));
        s.map_pane_size.set(Some((140, 48))); // what the renderer measured this frame
        apply_action(Action::Recenter, &mut s, &mut m);

        let mut want = AppState::default();
        want.recenter_on((7, 7), 140, 48);
        assert_eq!(s.scroll, want.scroll, "centred against the pane that was actually drawn");

        let mut guessed = AppState::default();
        guessed.recenter_on((7, 7), 80, 24);
        assert_ne!(s.scroll, guessed.scroll, "and not against the old 80×24 guess");
    }

    #[test]
    fn center_map_falls_back_to_eighty_by_twentyfour_before_the_first_frame() {
        // No frame has been drawn, so there is no pane to measure. The old constant is still the
        // only answer available — but it is now a fallback, not the assumption.
        let (mut s, mut m) = recenter_fixture();
        s.select_room(Some(2));
        assert!(s.map_pane_size.get().is_none(), "renderer has not run");
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((7, 7), 80, 24);
        assert_eq!(s.scroll, want.scroll);
    }

    #[test]
    fn center_map_falls_back_to_the_current_room_not_the_origin() {
        // SQ-0349: with nothing selected it recentred on (0,0) — an arbitrary corner of the map
        // that need not hold a room at all. The player's own location is the only sensible answer.
        let (mut s, mut m) = recenter_fixture();
        assert!(s.selected_room.is_none(), "nothing selected");
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((2, 2), 80, 24);
        assert_eq!(s.scroll, want.scroll, "centred on the CURRENT room");

        let mut origin = AppState::default();
        origin.recenter_on((0, 0), 80, 24);
        assert_ne!(s.scroll, origin.scroll, "and not on the origin");
    }

    #[test]
    fn center_map_falls_through_an_unplaced_selection_to_the_current_room() {
        // A selected room with no position cannot be centred on. Dropping to the origin would
        // strand the view; the current room is still a real answer.
        let (mut s, mut m) = recenter_fixture();
        m.graph.upsert_room(9, "Unplaced".into()); // never given a pos
        s.select_room(Some(9));
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((2, 2), 80, 24);
        assert_eq!(s.scroll, want.scroll, "fell through to the current room");
    }

    #[test]
    fn center_map_with_nothing_placed_at_all_uses_the_origin() {
        // Last resort, unchanged: no selection, no current room, nothing to aim at.
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((0, 0), 80, 24);
        assert_eq!(s.scroll, want.scroll);
    }

    // ── SQ-0672: recenter the map whenever the active layer changes ──────────

    /// A room on layer 1 and a room on Main, no positions shared, so any wrong-layer centering
    /// shows up immediately as the wrong scroll.
    fn two_layer_fixture() -> (AppState, Mapper) {
        let mut m = Mapper::default();
        m.graph.upsert_room(1, "Hall".into());
        m.graph.set_pos(1, (0, 0));
        m.graph.set_current(1);
        m.graph.upsert_room(2, "Attic".into());
        m.graph.set_pos(2, (5, 5));
        let l = m.graph.new_layer(Some(mapper::layer::MAIN_LAYER), "Attic".into());
        m.graph.set_room_layer(2, l);
        (AppState::default(), m)
    }

    #[test]
    fn switching_layers_centers_on_the_current_room_when_it_is_there() {
        let (mut s, mut m) = two_layer_fixture();
        let attic = m.graph.layer_of(2);
        m.graph.set_current(2); // the player is standing in the layer being switched TO
        apply_action(Action::SetViewedLayer(attic), &mut s, &mut m);

        let mut want = AppState::default();
        want.recenter_on((5, 5), 80, 24);
        assert_eq!(s.scroll, want.scroll, "centred on the current room, which is on the new layer");
        assert_eq!(s.selected_room, Some(2));
    }

    #[test]
    fn switching_layers_centers_on_the_last_room_visited_there_when_the_player_is_elsewhere() {
        let (mut s, mut m) = two_layer_fixture();
        let attic = m.graph.layer_of(2);
        m.graph.set_current(2); // visits the Attic once — recorded as its last-visited room
        m.graph.set_current(1); // then leaves; the player is back on Main
        assert_eq!(m.graph.last_visited(attic), Some(2), "the visit was recorded");

        apply_action(Action::SetViewedLayer(attic), &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((5, 5), 80, 24);
        assert_eq!(
            s.scroll, want.scroll,
            "centred on the last room visited there, not the current (Main) room"
        );
        assert_eq!(s.selected_room, Some(2));
    }

    #[test]
    fn switching_to_a_never_visited_layer_centers_on_its_bounding_box() {
        let mut m = Mapper::default();
        m.graph.upsert_room(1, "Hall".into());
        m.graph.set_pos(1, (0, 0));
        m.graph.set_current(1);
        let l = m.graph.new_layer(Some(mapper::layer::MAIN_LAYER), "Cellar".into());
        // Three rooms whose bounding box centres uniquely on room 3, at (6, 6): the box runs
        // (2,2)-(10,2) x (2,2)-(6,6) → centre (6, 4), and only room 3 sits near it.
        for (id, pos) in [(2u16, (2, 2)), (3, (6, 6)), (4, (10, 2))] {
            m.graph.upsert_room(id, format!("Room {id}"));
            m.graph.set_pos(id, pos);
            m.graph.set_room_layer(id, l);
        }
        assert_eq!(m.graph.last_visited(l), None, "never visited");

        let mut s = AppState::default();
        apply_action(Action::SetViewedLayer(l), &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((6, 4), 80, 24);
        assert_eq!(s.scroll, want.scroll, "centred on the layer's own bounding box");
        assert_eq!(
            s.selected_room, Some(3),
            "the room nearest the box centre stands in for selection"
        );
    }

    /// A room recorded as a layer's last-visited can later be peeled onto ANOTHER layer — the
    /// memory is now stale for the layer that recorded it, and must be treated exactly like a
    /// dangling id: fall through to the bounding-box centre, not a room that lives elsewhere now.
    #[test]
    fn a_last_visited_room_since_moved_to_another_layer_falls_back_to_the_bounding_box() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", Some(Direction::N)); // Main; last_visited(Main) = 2; arrived via N
        let mut s = AppState::default();
        // A bare peel cuts the passage just walked in through, taking room 2 (and nothing else)
        // onto a fresh layer — current stays on room 2, but its LAYER changes.
        apply_action(Action::MoveRegion("new".into()), &mut s, &mut m);
        let new_layer = s.viewed_layer.expect("the peel selected the new layer");
        assert_ne!(new_layer, mapper::layer::MAIN_LAYER);
        assert_eq!(m.graph.layer_of(2), new_layer, "room 2 left Main");
        assert_eq!(m.graph.layer_of(1), mapper::layer::MAIN_LAYER, "room 1 stayed");
        assert_eq!(
            m.graph.last_visited(mapper::layer::MAIN_LAYER), Some(2),
            "Main's last-visited memory still names room 2 — it is now stale"
        );

        apply_action(Action::SetViewedLayer(mapper::layer::MAIN_LAYER), &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on(m.graph.room(1).unwrap().pos.unwrap(), 80, 24);
        assert_eq!(
            s.scroll, want.scroll,
            "the stale last-visited id is skipped — falls back to the bounding box (room 1 alone)"
        );
        assert_eq!(s.selected_room, Some(1));
    }

    #[test]
    fn switching_to_a_matrix_layer_selects_and_scrolls_to_the_target_room() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None); // Main
        let maze = m.graph.new_layer(Some(mapper::layer::MAIN_LAYER), "Maze".into());
        m.graph.set_layer_view(maze, Some(mapper::layer::MapView::Matrix));
        // Ten rooms on the maze layer — more than a small pane can show at once.
        for id in 2..=11u16 {
            m.graph.upsert_room(id, format!("Room {id}"));
            m.graph.set_room_layer(id, maze);
            m.graph.set_pos(id, (0, id as i32));
        }
        m.graph.set_current(9); // visits room 9 — recorded as the maze's last-visited room
        m.graph.set_current(1); // then leaves, back to Main
        assert_eq!(m.graph.last_visited(maze), Some(9));

        let mut s = AppState::default();
        s.map_pane_size.set(Some((80, 10))); // small pane: not all ten rows fit
        apply_action(Action::SetViewedLayer(maze), &mut s, &mut m);

        assert_eq!(s.selected_room, Some(9), "the last-visited room becomes the selected row");
        let area = ratatui::layout::Rect::new(0, 0, 80, 10);
        let want_scroll = crate::render::matrix::scroll_to_show(&m.graph, maze, 9, area, 0);
        assert_eq!(s.matrix_scroll.1, want_scroll, "and the table scrolls to show it");
        assert_ne!(s.matrix_scroll.1, 0, "room 9 does not fit at the top of a ten-row table in this pane");
    }

    // ── SQ-0354: caret editing on the command line ───────────────────────────

    #[test]
    fn arrows_move_the_caret_and_typing_lands_at_it() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "north".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        assert_eq!(s.input.cursor, 5, "typing leaves the caret at the end");

        apply_action(Action::CursorLeft, &mut s, &mut m);
        apply_action(Action::CursorLeft, &mut s, &mut m);
        assert_eq!(s.input.cursor, 3);
        apply_action(Action::InputChar('X'), &mut s, &mut m);
        assert_eq!(s.input.value, "norXth", "a typed char lands AT the caret, not at the end");

        apply_action(Action::CursorHome, &mut s, &mut m);
        assert_eq!(s.input.cursor, 0);
        apply_action(Action::CursorEnd, &mut s, &mut m);
        assert_eq!(s.input.cursor, 6);
        // Clamped at both ends.
        apply_action(Action::CursorRight, &mut s, &mut m);
        assert_eq!(s.input.cursor, 6, "Right at the end does not run off");
        apply_action(Action::CursorHome, &mut s, &mut m);
        apply_action(Action::CursorLeft, &mut s, &mut m);
        assert_eq!(s.input.cursor, 0, "Left at the start does not run off");
    }

    #[test]
    fn backspace_and_delete_cut_opposite_sides_of_the_caret() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "abc".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        apply_action(Action::CursorLeft, &mut s, &mut m); // between b and c
        apply_action(Action::Backspace, &mut s, &mut m);
        assert_eq!(s.input.value, "ac", "Backspace cuts the char BEFORE the caret");
        apply_action(Action::DeleteChar, &mut s, &mut m);
        assert_eq!(s.input.value, "a", "Delete cuts the char AT the caret");
    }

    #[test]
    fn right_at_the_end_accepts_the_suggestion() {
        // SQ-0354: at the end of the line there is nothing to move onto, so Right would be a dead
        // key. Taking the showing suggestion is the only useful reading (the fish/zsh gesture).
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "/toggle-roo".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        assert!(!s.suggestions.is_empty(), "a suggestion is showing: {:?}", s.suggestions);
        let want = format!("/{}", s.suggestions[0]);
        apply_action(Action::CursorRight, &mut s, &mut m);
        assert_eq!(s.input.value, want, "Right at the end accepted the suggestion");
        assert!(s.suggestion_active, "and marks it applied, exactly as Tab does");
    }

    #[test]
    fn right_mid_line_moves_the_caret_even_with_a_suggestion_showing() {
        // The accept gesture must not eat plain caret movement: it fires ONLY at the end.
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "/toggle-roo".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        assert!(!s.suggestions.is_empty());
        let before = s.input.value.clone();
        apply_action(Action::CursorHome, &mut s, &mut m);
        apply_action(Action::CursorRight, &mut s, &mut m);
        assert_eq!(s.input.value, before, "mid-line Right leaves the text alone");
        assert_eq!(s.input.cursor, 1, "it just moves the caret");
    }

    #[test]
    fn a_click_on_the_input_line_places_the_caret() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "north".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        // The renderer records where the text landed; without a frame there is nothing to click.
        assert!(s.input_click_index(10, 5).is_none(), "no origin captured yet -> no mapping");
        s.input_text_origin.set(Some((10, 5)));

        assert_eq!(s.input_click_index(12, 5), Some(2), "click on the 3rd char");
        assert_eq!(s.input_click_index(12, 4), None, "a different row is not the input line");
        assert_eq!(s.input_click_index(9, 5), None, "left of the text is not the input line");
        assert_eq!(s.input_click_index(99, 5), Some(5), "past the end clamps to the end");

        apply_action(Action::CursorToClick(12, 5), &mut s, &mut m);
        assert_eq!(s.input.cursor, 2, "the click moved the caret");
    }

    #[test]
    fn a_click_on_a_wide_glyph_input_line_places_the_caret_by_cell() {
        // SQ-0655: the click offset is a CELL count. "日本語" is 3 chars but 6 cells,
        // so treating the offset as a char index put the caret up to three chars short.
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "日本語x".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        s.input_text_origin.set(Some((10, 5)));
        // Cells: 日 = 10..11, 本 = 12..13, 語 = 14..15, x = 16.
        assert_eq!(s.input_click_index(10, 5), Some(0));
        assert_eq!(s.input_click_index(11, 5), Some(0), "right cell of 日 still selects 日");
        assert_eq!(s.input_click_index(12, 5), Some(1), "the 2nd glyph starts two cells in");
        assert_eq!(s.input_click_index(14, 5), Some(2));
        assert_eq!(s.input_click_index(16, 5), Some(3), "the ASCII char after three wide ones");
        assert_eq!(s.input_click_index(17, 5), Some(4), "past the end clamps to the end");

        apply_action(Action::CursorToClick(12, 5), &mut s, &mut m);
        assert_eq!(s.input.cursor, 1);
        apply_action(Action::InputChar('!'), &mut s, &mut m);
        assert_eq!(s.input.value, "日!本語x", "the caret landed where it was clicked");
    }

    #[test]
    fn multi_byte_input_survives_caret_editing() {
        // The byte arithmetic this replaced would panic outright here: String::truncate rejects a
        // non-char boundary, and every one of these chars is multi-byte.
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "héllo".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        assert_eq!(s.input.char_len(), 5);
        apply_action(Action::CursorHome, &mut s, &mut m);
        apply_action(Action::CursorRight, &mut s, &mut m);
        apply_action(Action::InputChar('X'), &mut s, &mut m);
        assert_eq!(s.input.value, "hXéllo", "insert lands on a char boundary");
        apply_action(Action::DeleteChar, &mut s, &mut m);
        assert_eq!(s.input.value, "hXllo", "delete cuts a whole char, not a byte");
    }

    // ── slash_suggestions tests ───────────────────────────────────────────────

    #[test]
    fn slash_suggestions_filter_by_prefix() {
        let names = vec!["panh".to_string(),"panv".to_string(),"zoom".to_string(),"open-settings".to_string()];
        let s = slash_suggestions("pa", &names, 6);
        assert!(s.contains(&"panh".to_string()) && s.contains(&"panv".to_string()));
        assert!(!s.contains(&"zoom".to_string()));
    }

    #[test]
    fn slash_suggestions_match_any_part_of_the_name() {
        // SQ-0353. Command names are compound (`toggle-room-numbers`), and the part you remember is
        // usually the NOUN, not the verb it happens to start with. Prefix-only matching meant the
        // word you could actually recall found nothing at all.
        let names = crate::slash::slash_names();
        let s = slash_suggestions("room", &names, 10);
        for want in ["toggle-room-numbers", "select-room", "rename-room"] {
            assert!(s.contains(&want.to_string()), "'room' must offer {want}: {s:?}");
        }
        // And it must not turn into a free-for-all: a name without the token stays out.
        assert!(!s.iter().any(|n| !n.contains("room")), "every hit contains the token: {s:?}");
    }

    #[test]
    fn slash_suggestions_rank_prefix_then_word_start_then_middle() {
        // Substring matching is only useful if the obvious answer still comes first. Typing the
        // start of a name must not bury it under incidental mid-word hits elsewhere.
        let names: Vec<String> = ["map-x", "zoom-map", "unmappable", "map-y"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = slash_suggestions("map", &names, 10);
        assert_eq!(
            s,
            vec!["map-x", "map-y", "zoom-map", "unmappable"],
            "prefix first, then a name-part start, then mid-word; alphabetical within each rank",
        );
    }

    #[test]
    fn slash_suggestions_never_offer_what_is_already_typed() {
        // Pre-existing contract, kept: a completion identical to the token is noise.
        let names = vec!["map".to_string(), "zoom-map".to_string()];
        let s = slash_suggestions("map", &names, 6);
        assert!(!s.contains(&"map".to_string()), "exact token is not a suggestion: {s:?}");
        assert!(s.contains(&"zoom-map".to_string()), "but other substring hits still show: {s:?}");
    }

    /// Regression: CONFIG_ROW_COUNT must equal CONFIG_ROWS.len() so every config row is
    /// keyboard-reachable.  If a row is added to CONFIG_ROWS without updating this constant,
    /// Down from the penultimate row wraps to row 0 and the last row becomes unreachable.
    #[test]
    fn config_row_count_matches_config_rows_len() {
        assert_eq!(
            CONFIG_ROW_COUNT,
            crate::render::config_screen::CONFIG_ROWS.len(),
            "CONFIG_ROW_COUNT must equal CONFIG_ROWS.len(); update CONFIG_ROW_COUNT when adding/removing rows"
        );
    }

    #[test]
    fn config_cycle_interpreter_number_reaches_default() {
        // By NAME, not by a literal: `config_cycle` is a match on the row's
        // position, so inserting any row above this one silently retargets a
        // hard-coded index at its neighbour (SQ-0873 inserted `period_look` and
        // this test started cycling `undo_levels` instead).
        let row = crate::render::config_screen::CONFIG_ROWS
            .iter()
            .position(|(n, _, _)| *n == "interpreter_number")
            .expect("the row exists");
        let mut c = crate::config::Config::default();
        c.interpreter_number = None;
        config_cycle(&mut c, row, 1); // default → 1
        assert_eq!(c.interpreter_number, Some(1));
        config_cycle(&mut c, row, -1); // 1 → default
        assert_eq!(c.interpreter_number, None);
        config_cycle(&mut c, row, -1); // default clamps, stays default
        assert_eq!(c.interpreter_number, None);
        for _ in 0..20 {
            config_cycle(&mut c, row, 1); // climbs then clamps at 10
        }
        assert_eq!(c.interpreter_number, Some(10));
    }

    #[test]
    fn cycle_focus_wraps_both_ways() {
        assert_eq!(cycle_focus(0, 3, 1), 1);
        assert_eq!(cycle_focus(2, 3, 1), 0); // wrap forward
        assert_eq!(cycle_focus(0, 3, -1), 2); // wrap backward
        assert_eq!(cycle_focus(5, 0, 1), 0); // empty
    }

    // ── Task 3: config_screen Tab focus + Enter-activates-focused ─────────────

    #[test]
    fn config_screen_tab_then_enter_fires_cancel() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        let working = clone_config(&s.config);
        s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
        s.overlays.dialog_focus = cycle_focus(0, 2, 1); // focus Cancel (index 1)
        let a = config_screen_key_to_action(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            s.overlays.dialog_focus,
        );
        assert!(matches!(a, Action::ConfigCancel),
            "Enter with focus=1 (Cancel) should fire ConfigCancel, got {:?}", a);
    }

    #[test]
    fn config_screen_enter_at_default_focus_fires_save() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        let working = clone_config(&s.config);
        s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
        s.overlays.dialog_focus = 0; // focus Save (default)
        let a = config_screen_key_to_action(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            s.overlays.dialog_focus,
        );
        assert!(matches!(a, Action::ConfigSave),
            "Enter with focus=0 (Save) should fire ConfigSave, got {:?}", a);
    }

    #[test]
    fn config_screen_space_still_toggles_row() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        let working = clone_config(&s.config);
        s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
        // Space must toggle the selected row regardless of focus.
        for focus in [0, 1] {
            let a = config_screen_key_to_action(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                focus,
            );
            assert!(matches!(a, Action::ConfigToggle),
                "Space with focus={focus} should fire ConfigToggle, got {:?}", a);
        }
    }

    #[test]
    fn saves_tab_cycles_done_button_focus() {
        // The saves dialog has a ring of length 1 (Done only). Tab cycles 0 → 0 (stays).
        // Enter still loads the selected save (existing behavior).
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        s.overlays.saves = Some(crate::state::SavesState { entries: Vec::new(), scroll: Default::default() });
        s.overlays.dialog_focus = 0;
        // Tab with ring len 1 stays at 0.
        let after_tab = cycle_focus(s.overlays.dialog_focus, 1, 1);
        assert_eq!(after_tab, 0, "Tab on ring-len-1 should stay at 0");
        // Enter still produces SavesLoad (not SavesClose) regardless of focus.
        let a = saves_key_to_action(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            s.overlays.dialog_focus,
        );
        assert!(matches!(a, Action::SavesLoad),
            "Enter in saves should still fire SavesLoad (not affected by focus), got {:?}", a);
    }

    #[test]
    fn open_config_resets_dialog_focus() {
        let mut s = AppState::default();
        s.overlays.dialog_focus = 5; // non-zero
        let mut m = mapper::mapper::Mapper::default();
        apply_action(Action::OpenConfig, &mut s, &mut m);
        assert_eq!(s.overlays.dialog_focus, 0, "OpenConfig must reset dialog_focus to 0");
    }

    #[test]
    fn open_saves_resets_dialog_focus_in_apply() {
        let mut s = AppState::default();
        s.overlays.dialog_focus = 5; // non-zero
        let mut m = mapper::mapper::Mapper::default();
        apply_action(Action::OpenSaves, &mut s, &mut m);
        assert_eq!(s.overlays.dialog_focus, 0, "OpenSaves must reset dialog_focus to 0");
    }

    // ── Task 5: read-only / single-button panels ──────────────────────────────

    /// Enter belongs to the story prompt, in BOTH dock views. It once closed the
    /// room panel, on the since-expired assumption that a read-only panel meant no
    /// text input underneath it; the dock makes that even plainer, since it never
    /// covers the prompt at all. Esc is the keyboard way out (see the ladder test).
    #[test]
    fn room_dock_enter_submits_and_leaves_the_dock_open() {
        use crate::state::RoomDockView;
        for view in [RoomDockView::Info, RoomDockView::Diagnostics] {
            let mut s = AppState::default();
            s.room_dock.toggle_to(true, true);
            s.room_dock_view = view;
            let a = key_to_action(&s, key(KeyCode::Enter));
            assert!(
                matches!(a, Action::SubmitCommand(_)),
                "Enter must submit the story command, not touch the dock (got {a:?})"
            );
            assert!(
                matches!(key_to_action(&s, key(KeyCode::Esc)), Action::CloseRoomDock),
                "Esc remains the keyboard way out"
            );
        }
    }

    #[test]
    fn hotkey_dialog_enter_closes() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(
            matches!(a, Action::CloseHotkeyDialog),
            "Enter must close the hotkey dialog (got {:?})",
            a
        );
    }

    // ── Task 6: navigation panels — regression guard ──────────────────────────

    /// The band still CONSUMES Tab (it never falls through to the pane focus
    /// toggle while open) — SQ-0677 changed what it does with it: move the
    /// current column when nothing is highlighted, pick-and-advance when
    /// something is. Flipped from `command_band_tab_still_swaps_focus`.
    #[test]
    fn command_band_still_consumes_tab() {
        let mut s = AppState::default();
        open_band(&mut s);
        let a = command_band_intercept(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &s);
        assert_eq!(a, Some(Action::BandColumnStep(1)), "consumed, and with nothing highlighted it moves the column");
    }

    // ── SQ-0237: resize-mode key routing ──────────────────────────────────────

    #[test]
    fn resize_mode_key_to_action_maps_tab_and_backtab() {
        assert!(matches!(
            resize_mode_key_to_action(key(KeyCode::Tab)),
            Action::ResizeNav(ResizeNavKind::NextTarget)
        ));
        assert!(matches!(
            resize_mode_key_to_action(key(KeyCode::BackTab)),
            Action::ResizeNav(ResizeNavKind::PrevTarget)
        ));
    }

    #[test]
    fn resize_mode_key_to_action_maps_arrows() {
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Left)), Action::ResizeNav(ResizeNavKind::Left)));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Right)), Action::ResizeNav(ResizeNavKind::Right)));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Up)), Action::ResizeNav(ResizeNavKind::Up)));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Down)), Action::ResizeNav(ResizeNavKind::Down)));
    }

    #[test]
    fn resize_mode_key_to_action_maps_reset_and_exit() {
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Char('0'))), Action::ResizeReset));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Esc)), Action::ResizeExit));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Enter)), Action::ResizeExit));
    }

    #[test]
    fn resize_mode_intercepts_tab_before_focus_toggle() {
        let mut s = AppState::default();
        s.resize_mode = true;
        let a = key_to_action(&s, key(KeyCode::Tab));
        assert!(matches!(a, Action::ResizeNav(ResizeNavKind::NextTarget)));
    }

    // ── SQ-0238: command band ⇄ resize mode coexistence ───────────────────────

    /// Build a KeyEvent equal to the current hotkey-dialog leader prefix,
    /// regardless of what it is bound to (default ctrl+k, but another lane may
    /// move it) — the prefix must always pass through the band's intercept.
    fn prefix_key(s: &AppState) -> KeyEvent {
        let p = s.hotkeys.prefix;
        let mut m = KeyModifiers::NONE;
        if p.ctrl { m |= KeyModifiers::CONTROL; }
        if p.shift { m |= KeyModifiers::SHIFT; }
        if p.alt { m |= KeyModifiers::ALT; }
        KeyEvent::new(p.code, m)
    }

    #[test]
    fn resize_mode_preempts_the_band_intercept() {
        // (a) With the band open AND resize mode on, arrows/Tab resolve to
        // ResizeNav (resize owns them), not the band's navigation actions.
        let mut s = AppState::default();
        open_band(&mut s);
        s.resize_mode = true;

        assert!(matches!(
            key_to_action(&s, key(KeyCode::Down)),
            Action::ResizeNav(ResizeNavKind::Down)
        ), "Down resizes, not BandNav");
        assert!(matches!(
            key_to_action(&s, key(KeyCode::Tab)),
            Action::ResizeNav(ResizeNavKind::NextTarget)
        ), "Tab cycles resize target, not band focus");
        assert!(matches!(
            key_to_action(&s, key(KeyCode::Esc)),
            Action::ResizeExit
        ), "Esc exits resize mode (back to the still-open band), not BandEscape");
    }

    #[test]
    fn band_intercept_passes_through_the_leader_prefix() {
        // (b) With the band open, the leader prefix is NOT swallowed, so it can
        // still open the leader palette (→ the '/' palette → resize-panes).
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        assert!(command_band_intercept(prefix_key(&s), &s).is_none(), "prefix falls through");
        // …armed or not: the prefix is checked before anything else the band
        // might claim (SQ-0238), and every gesture amendment since has kept
        // that ordering.
        apply_action(Action::BandRowNav(1), &mut s, &mut mapper);
        assert!(command_band_intercept(prefix_key(&s), &s).is_none(), "…even armed");

        assert!(matches!(
            key_to_command(&s, prefix_key(&s)),
            KeyResolve::Action(Action::OpenHotkeyDialog)
        ), "prefix opens the hotkey dialog with the band open");
    }

    #[test]
    fn the_band_is_a_resize_target_only_while_open() {
        let mut s = AppState::default();
        assert!(!s.resize_targets_visible().contains(&crate::state::ResizeTarget::CommandBand));
        open_band(&mut s);
        assert!(s.resize_targets_visible().contains(&crate::state::ResizeTarget::CommandBand));
    }

    #[test]
    fn band_nav_still_resolves_when_resize_mode_off() {
        // (c) With resize mode OFF, band navigation resolves normally.
        let mut s = AppState::default();
        open_band(&mut s);
        assert!(!s.resize_mode);
        assert_eq!(
            key_to_action(&s, key(KeyCode::Down)),
            Action::BandRowNav(1),
            "↓ drives the band's row highlight when not resizing"
        );
    }

    // ── SQ-1236: a modal dialog over the band owns all input ───────────────────

    /// Open Settings with a known `ConfigScreenState`, mirroring
    /// `config_esc_maps_to_config_cancel`'s setup.
    fn open_config_screen(s: &mut AppState) {
        let working = crate::input::clone_config(&s.config);
        s.overlays.config_screen =
            Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
    }

    #[test]
    fn config_screen_over_band_preempts_the_band_intercept() {
        // Falsified by reverting the `!any_modal_overlay_open()` guard on the
        // band's intercept in `key_to_command`: before the fix, Up/Down/Esc
        // resolved to `BandRowNav`/`BandEscape` here instead — the band, not
        // the dialog on top of it, ate the keys.
        let mut s = AppState::default();
        open_band(&mut s);
        open_config_screen(&mut s);

        assert_eq!(
            key_to_action(&s, key(KeyCode::Down)),
            Action::ConfigNav(1),
            "↓ must drive the dialog, not BandRowNav"
        );
        assert_eq!(
            key_to_action(&s, key(KeyCode::Up)),
            Action::ConfigNav(-1),
            "↑ must drive the dialog, not BandRowNav"
        );
        assert_eq!(
            key_to_action(&s, key(KeyCode::Esc)),
            Action::ConfigCancel,
            "Esc must close the dialog, not BandEscape"
        );

        // The band's own selection state is untouched by any of the above —
        // none of those keys ever reached `command_band_intercept`.
        assert_eq!(band(&s).row_sel, None, "band selection must be unchanged");
    }

    #[test]
    fn config_screen_esc_closes_dialog_and_leaves_band_open() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        open_config_screen(&mut s);

        let a = key_to_action(&s, key(KeyCode::Esc));
        apply_action(a, &mut s, &mut mapper);

        assert!(s.overlays.config_screen.is_none(), "Esc must close the dialog");
        assert!(s.overlays.command_band.is_some(), "…and must NOT close the band underneath");
    }

    #[test]
    fn band_mouse_click_is_inert_while_config_screen_is_open() {
        // `band_mouse_action` lives in main.rs and isn't reachable from here,
        // but the guard it now shares with the keyboard path
        // (`state.any_modal_overlay_open()`) is: assert the shared predicate
        // is true exactly when it must gate the band's mouse routing too.
        let mut s = AppState::default();
        open_band(&mut s);
        assert!(!s.any_modal_overlay_open(), "band alone is not modal (SQ-0664)");
        open_config_screen(&mut s);
        assert!(
            s.any_modal_overlay_open(),
            "a dialog stacked over the band must read as modal, so band_mouse_action's \
             any_modal_overlay_open guard fires and the click falls through to the dialog's \
             own hit-testing instead of picking a band row"
        );
    }

    #[test]
    fn band_tab_does_not_fire_while_config_screen_is_open() {
        // Tab's dialog-focus cycling happens upstream in main.rs regardless of
        // the band (unconditional, keyed only on `config_screen.is_some()`), so
        // it is not exercised here. What IS this layer's job: Tab must resolve
        // to the dialog's own (non-)handling of it, not to the band's
        // `BandColumnStep`/`BandTabPick` — before the fix it produced both.
        let mut s = AppState::default();
        open_band(&mut s);
        open_config_screen(&mut s);
        assert_eq!(
            key_to_action(&s, key(KeyCode::Tab)),
            Action::None,
            "Tab must not resolve to a band action while the dialog is open"
        );
    }

    #[test]
    fn band_arrows_resume_after_config_screen_closes() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_band(&mut s);
        open_config_screen(&mut s);
        apply_action(Action::ConfigCancel, &mut s, &mut mapper);
        assert!(s.overlays.config_screen.is_none());

        assert_eq!(
            key_to_action(&s, key(KeyCode::Down)),
            Action::BandRowNav(1),
            "with the dialog gone, ↓ drives the band's row highlight again"
        );
    }

    // ── SQ-1244: the inventory panel's items click into the prompt ─────────────
    //
    // The command panel and the inventory panel are mutually exclusive
    // (`SidePanel`), so the inventory dock's click always lands with
    // `state.overlays.command_band` closed — there is no `CommandBandState`
    // to pick FROM. `Action::InventoryClickRow` resolves the word from
    // `AppState::inventory_click_words` (what a real loop tick refreshes
    // from the engine; these tests seed it directly, mirroring `open_band`'s
    // synthetic object model) and composes it via `compose_word_onto_prompt`
    // — the SAME composer (`sync_band_phrase_to_input`) `band_pick_row` uses.

    /// Seed the inventory panel open with a known, synthetic click-word list
    /// — the panel's counterpart of `open_band`'s synthetic object model.
    fn open_inventory_panel_for_test(state: &mut AppState, words: &[&str]) {
        state.show_inventory = true;
        state.inv_dock.toggle_to(true, true);
        state.inventory_click_words = words.iter().map(|w| w.to_string()).collect();
    }

    /// Falsified by removing the `compose_word_onto_prompt` call from
    /// `Action::InventoryClickRow`'s `apply_action` arm: before the fix the
    /// click did nothing and the prompt stayed exactly `"examine "`.
    #[test]
    fn inventory_click_with_a_typed_verb_and_trailing_space_appends_the_item() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_inventory_panel_for_test(&mut s, &["lamp", "leaflet"]);
        s.input.set("examine ".to_string(), true);

        apply_action(Action::InventoryClickRow(1), &mut s, &mut mapper);

        assert_eq!(s.input.value, "examine leaflet");
    }

    /// Command panel closed, inventory panel open, EMPTY prompt: the WHAT-noun
    /// path's own rule with no verb typed — `compose_word_onto_prompt` strips
    /// nothing (there is no partial word) and composes the bare item.
    #[test]
    fn inventory_click_on_an_empty_prompt_composes_the_bare_item() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_inventory_panel_for_test(&mut s, &["leaflet"]);
        assert_eq!(s.input.value, "");

        apply_action(Action::InventoryClickRow(0), &mut s, &mut mapper);

        assert_eq!(s.input.value, "leaflet");
    }

    /// An unrecognized partial word (`exa`, not yet a complete verb) at the
    /// prompt is REPLACED outright, not appended after — SQ-1230's rule
    /// ("a partial word being typed is replaced"), pinned here for the
    /// no-`CommandBandState` composer exactly as `band_pick_row` already
    /// pins it for a table pick in `arity_drives_column_reachability` and
    /// friends.
    #[test]
    fn inventory_click_replaces_an_unrecognized_partial_word() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_inventory_panel_for_test(&mut s, &["leaflet"]);
        s.input.set("exa".to_string(), true);

        apply_action(Action::InventoryClickRow(0), &mut s, &mut mapper);

        assert_eq!(s.input.value, "leaflet", "the partial word is replaced, not appended after");
    }

    /// A stale/out-of-range index (the click landed after the panel's
    /// contents changed underneath it) composes nothing rather than
    /// panicking or picking the wrong item.
    #[test]
    fn inventory_click_with_a_stale_index_is_a_no_op() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_inventory_panel_for_test(&mut s, &["leaflet"]);
        s.input.set("examine ".to_string(), true);

        apply_action(Action::InventoryClickRow(5), &mut s, &mut mapper);

        assert_eq!(s.input.value, "examine ", "an out-of-range index composes nothing");
    }

    /// SQ-1236's rule extended to the inventory dock: a modal dialog stacked
    /// on top takes all mouse input. `inventory_mouse_action` lives in
    /// main.rs and isn't reachable from here, but the guard it shares with
    /// the band's own mouse routing (`state.any_modal_overlay_open()`) is:
    /// assert the shared predicate is true exactly when it must gate the
    /// inventory dock's mouse routing too.
    #[test]
    fn inventory_panel_alone_is_not_modal_but_a_dialog_over_it_is() {
        let mut s = AppState::default();
        open_inventory_panel_for_test(&mut s, &["leaflet"]);
        assert!(!s.any_modal_overlay_open(), "the inventory panel alone is not modal (SQ-1244)");
        open_config_screen(&mut s);
        assert!(
            s.any_modal_overlay_open(),
            "a dialog stacked over the inventory panel must read as modal, so \
             inventory_mouse_action's any_modal_overlay_open guard fires and the click falls \
             through to the dialog's own hit-testing instead of composing an item"
        );
    }

    /// SQ-0692 deleted this test's subject. `roominfo_ok_button_click_closes_panel`
    /// pinned the ✕/[OK] chrome of the floating Room Info dialog; the dock has no
    /// dialog chrome to click, so the fact worth keeping is the inverse — with the
    /// dock open, a click that hits no dialog and no room is inert rather than
    /// closing anything.
    #[test]
    fn a_click_on_nothing_does_not_close_the_room_dock() {
        use ratatui::layout::Rect;

        let mut state = AppState::default();
        state.room_dock.toggle_to(true, true);

        let map = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];

        let a = mouse_to_action(&state, mouse_left_click(32, 14), map, story, room_rects, &None);
        assert!(
            matches!(a, Action::None),
            "a click outside every pane is inert; nothing closes the dock but Esc or its command, got {:?}", a
        );
    }

    #[test]
    fn tidy_ok_button_click_exits_anim() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::TidyAnim;
        use crate::state::TidyFrame;

        let rects = DialogRects {
            area:    Rect::new(0, 0, 40, 15),
            content: Rect::new(1, 1, 38, 12),
            close:   Some(Rect::new(38, 0, 1, 1)),
            buttons: vec![(ButtonId::Ok, Rect::new(30, 14, 6, 1))],
            field: None,
        };

        let mut state = AppState::default();
        state.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "test".to_string(),
            graph: mapper::graph::MapGraph::new(),
            description: String::new(),
            stats: mapper::layout::TidyStats::default(),
            stage_start: false,
            manifest: None,
        }], mapper::layer::MAIN_LAYER));

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // OK button click → AnimExit
        let a = mouse_to_action(&state, mouse_left_click(32, 14), map, story, room_rects, &dialog);
        assert!(
            matches!(a, Action::AnimExit),
            "tidy [OK] click should produce AnimExit, got {:?}", a
        );
    }


    // ── SQ-0653: bracketed paste lands as literal text in the focused field ────

    #[test]
    fn sanitize_pasted_text_flattens_line_breaks_and_drops_controls() {
        // CRLF counts once; bare \n and \r and tabs each become one space.
        assert_eq!(super::sanitize_pasted_text("go north\r\nget lamp"), "go north get lamp");
        assert_eq!(super::sanitize_pasted_text("a\nb\rc\td"), "a b c d");
        // Other control characters (here: ESC and BEL) are dropped outright — a
        // paste must never smuggle an escape sequence into a field.
        assert_eq!(super::sanitize_pasted_text("ab\u{1b}[31mc\u{7}"), "ab[31mc");
        // Whitespace is the user's; nothing is trimmed.
        assert_eq!(super::sanitize_pasted_text("  x  "), "  x  ");
        assert!(super::sanitize_pasted_text("").is_empty());
    }

    #[test]
    fn paste_inserts_into_the_game_input_line_at_the_caret_without_submitting() {
        let mut s = AppState::default();
        s.input = crate::text_field::TextField::new("go ");
        let turns_before = s.turns;

        assert!(apply_paste(&mut s, "north"));
        assert_eq!(s.input.value, "go north", "pasted text is inserted literally");
        assert_eq!(s.input.cursor, 8, "the caret follows the pasted text");
        assert_eq!(s.turns, turns_before, "a paste must NOT submit a turn");

        // Mid-line insert honours the caret.
        s.input.cursor = 3;
        assert!(apply_paste(&mut s, "far "));
        assert_eq!(s.input.value, "go far north");
    }

    #[test]
    fn paste_with_newlines_never_submits_the_line_to_the_game() {
        // The whole point of enabling bracketed paste: before it, a pasted
        // walkthrough was replayed as keystrokes and every newline fired a turn.
        let mut s = AppState::default();
        assert!(apply_paste(&mut s, "north\nsouth\neast"));
        assert_eq!(s.input.value, "north south east");
        assert_eq!(s.turns, 0, "no turn was taken");
    }

    #[test]
    fn paste_routes_to_the_topmost_modal_text_field() {
        // Text-entry dialog (rename room, notes, …) owns typing while open.
        let mut s = AppState::default();
        s.overlays.text_entry = Some(crate::state::TextEntryDialog::new(
            crate::state::TextEntryKind::RenameLayer(mapper::layer::MAIN_LAYER),
            "Cave",
        ));
        assert!(apply_paste(&mut s, " of Wonders"));
        assert_eq!(s.overlays.text_entry.as_ref().unwrap().field.value, "Cave of Wonders");
        assert!(s.input.value.is_empty(), "the game line must not see the paste");

        // Save-name dialog: pasting onto the greyed placeholder replaces it and
        // activates the field, exactly as typing a character does.
        let mut s = AppState::default();
        s.overlays.save_name_dialog = Some(crate::state::SaveNameDialog::new("2026-01-01".into(), false));
        assert!(apply_paste(&mut s, "before the troll"));
        let d = s.overlays.save_name_dialog.as_ref().unwrap();
        assert_eq!(d.field.value, "before the troll");
        assert!(d.active, "the field is now being edited");

        // Command palette: the paste edits its query and re-ranks the list.
        let mut s = AppState::default();
        s.overlays.palette = Some(crate::state::PaletteState::new(false));
        assert!(apply_paste(&mut s, "tidy"));
        assert_eq!(s.overlays.palette.as_ref().unwrap().input.value, "tidy");
        assert_eq!(s.overlays.palette.as_ref().unwrap().scroll.selected, 0);
    }

    #[test]
    fn paste_is_swallowed_by_a_modal_with_no_text_field() {
        // The saves manager owns the keyboard but has nothing to type into;
        // the paste must not leak onto the game line hidden behind it.
        let mut s = AppState::default();
        s.overlays.saves = Some(crate::state::SavesState { entries: Vec::new(), scroll: Default::default() });
        assert!(!apply_paste(&mut s, "north"));
        assert!(s.input.value.is_empty());

        // Same for the quit confirmation.
        let mut s = AppState::default();
        s.overlays.quit_dialog = true;
        assert!(!apply_paste(&mut s, "north"));
        assert!(s.input.value.is_empty());
    }
}
