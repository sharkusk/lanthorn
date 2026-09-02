//! Slash-command parser and the command registry.
//!
//! `COMMANDS` is the **single source of truth** for every command in the app.
//! Both typed slash input and key presses are dispatched the same way:
//! `parse_in_context(body, prefix, ctx)` looks the command up in `COMMANDS`
//! and runs its `dispatch` closure, producing a [`SlashOutcome`]; the run
//! loop's `dispatch_slash_outcome` (main.rs) then applies it. Key bindings are
//! just stored command-strings (see `crate::keymap`) fed through this same
//! `parse_in_context`. There is no separate command enum — execution flows
//! through [`crate::input::Action`] via `SlashOutcome::Action`.
//!
//! To add a command: add ONE [`CommandSpec`] to `COMMANDS`. That alone gives
//! it slash parsing, `/help` grouping + `help <command>` detail, and Tab
//! autocomplete — there is no second place to register it, so a command can
//! never be missing from `/help`. (Bump the count in the registry
//! well-formedness test when you do.) Names are verb-noun kebab-case; `quit`
//! and `help` are the only one-word exceptions. Directional behavior is
//! expressed by arguments bound to keys, not by separate commands (e.g.
//! `pan-map <dx> <dy>`), so prefer a parametric command over per-direction
//! variants.
//!
//! `parse` receives the input AFTER the leading prefix character has been
//! stripped. It does not know what the prefix was.
//!
//! The registry also carries the **pre-game story browser's** commands
//! (`Category::Library` / `Context::Browser`, SQ-0796). They are the one group
//! that is not typeable — the browser has no command line — so they are left out
//! of `/help` and Tab autocomplete, and reach the picker as
//! `SlashOutcome::Browser` rather than an `Action`, there being no `AppState`
//! that early. The gate is two-way: neither world's commands parse in the
//! other's context. See [`crate::browser`].

use crate::input::Action;
use crate::keymap::Context;

// ── SlashOutcome ──────────────────────────────────────────────────────────────

/// The result of parsing a slash-command body.
#[derive(Debug, Clone, PartialEq)]
pub enum SlashOutcome {
    /// Dispatch an action (pan, zoom, center, tidy, layer, …).
    Action(Action),
    /// Show an informational message on the status line (no effect).
    Message(String),
    /// Show an error message on the status line.
    Error(String),
    /// Print `/help` lines to the transcript as Meta entries.
    Help,
    /// Save the game; optionally to a named slot.
    Save(Option<String>),
    /// Load a save; optionally a named slot.
    Load(Option<String>),
    /// Reset the app; `map: true` also clears the automapper state; `data: true`
    /// also deletes the game's auto persistent data (VFS cache + aux + auto save).
    Reset { map: bool, data: bool },
    /// Quit the application.
    Quit,
    /// Exit the current story and return to the story picker (only meaningful
    /// when lanthorn was launched from a directory). Handled in
    /// `slash_dispatch`: it mirrors `Quit`'s save-prompt path but resolves the
    /// loop to the library instead of exiting. (SQ-0435)
    QuitToLibrary,
    /// Search the transcript; `None` repeats the last search.
    Search(Option<String>),
    /// Filter the transcript by category.
    Filter(TranscriptFilterArg),
    /// Export the visible transcript; `None` uses the default path.
    Export(Option<String>),
    /// Open the Hints panel (caller-handled, like Save/Load). Task D wires the real behavior.
    OpenHints,
    /// Show per-command detail for `help <name>`.
    HelpCommand(String),
    /// Print the resolved color scheme to the transcript. `actual` = render each
    /// selector line in its own style instead of the plain meta color.
    PrintColors { actual: bool },
    /// Diagnostic: dump the live Glk window layout (sizes, borders, per-window
    /// colours) to the transcript as Meta lines. Handled in `slash_dispatch`.
    DumpWindows,
    /// Diagnostic: write the last frame's rendered CELLS — glyphs plus per-cell
    /// colours and attributes — to `~/.lanthorn/dump-cells.log` as plain text
    /// (SQ-0761). Handled in `slash_dispatch`.
    DumpCells,
    /// Diagnostic: what lanthorn detected about this TERMINAL — protocol, cell
    /// size and whether it was measured or guessed, capabilities, whether kitty
    /// uploads are compressed — plus the render state and byte counts that
    /// explain the traffic (SQ-0994). Printed to the transcript and mirrored to
    /// `~/.lanthorn/dump-terminal.log`. Handled in `slash_dispatch`.
    DumpTerminal,
    /// Toggle the Z-machine debug inspector tiled pane. Handled in `slash_dispatch`
    /// (needs AppState + the engine's debugger capability).
    ToggleDebug,
    /// Replay the notification history into the transcript as Meta lines, in case
    /// a toast was missed. Handled in `slash_dispatch`. (SQ-0176)
    DumpNotifications,
    /// Diagnostic: list Blorb `Snd` resources (`None`) or play resource `n`
    /// (`Some(n)`). Handled in `main.rs::dispatch_slash_outcome` because it
    /// needs `AppState` (audio backend + sound blorb).
    PlaySound(Option<u32>),
    /// Load a standalone map file (path argument) into the current session.
    LoadMap(String),
    /// Force this game's `honor_game_colours` on/off (`Some`) or clear the
    /// per-game override (`None` = `auto`, fall back to garglk.ini/global).
    /// Persisted per-game; handled in `slash_dispatch`.
    SetGameColours(Option<bool>),
    /// Set this game's `borderless_windows` override — the payload is the
    /// borderless value: `Some(true)` = abut (no borders), `Some(false)` = keep
    /// Glk borders, `None` = clear the override (default: bordered). Persisted
    /// per-game; applied live to a running Glulx session. (SQ-0341)
    SetGameBorderless(Option<bool>),
    /// Toggle debug-trace sections: `None` shows current state; `Some(list)` sets
    /// the active set (comma list of screen,map,hostio / all / none). (trace feature)
    Trace(Option<String>),
    /// Switch this game's v6 render mode (SQ-1123). Applies live and is persisted
    /// in the per-game `config.toml` sidecar, never the global one. Handled in
    /// `slash_dispatch` (mutates `state.config.v6_render`).
    SetV6Render(V6RenderArg),
    /// Set this game's v6 pixel-lock preference (SQ-0945). Applies live —
    /// `state.config.v6_pixel_lock` is read afresh every frame — and is persisted
    /// in the per-game `config.toml` sidecar, never the global one. Handled in
    /// `slash_dispatch`.
    SetV6PixelLock(V6PixelLockArg),
    /// Switch Lanthorn's Guiding Light for this game (SQ-1045, SQ-1123). Applies
    /// live and is persisted in the per-game `config.toml` sidecar, never the
    /// global one. Handled in `slash_dispatch` (mutates `state.config.guidance`).
    SetGuidance(GuidanceArg),
    SetReturnProbe(ReturnProbeArg),
    /// Light the nouns and named things on screen this story knows, for a moment
    /// (SQ-1107, SQ-1207).
    ///
    /// **The one outcome in this family that is not a setting.** Everything
    /// beside it — the guidance switch, the probe, the render mode — reports a
    /// state you can read at a glance and flips it when asked. This one takes no
    /// argument, has no on state to inherit, persists nothing, and makes
    /// something happen elsewhere on the screen. Handled in `slash_dispatch`,
    /// which is where the engine is, because the story is what decides which
    /// words light.
    RevealWords,
    /// Re-run the first-run font check (SQ-1104): the two-row comparison that
    /// asks whether this terminal's font draws the Nerd Font icon glyphs. Opens
    /// the modal; the answer writes preset names into `style.toml` and reloads
    /// the theme. Handled in `slash_dispatch`.
    RunFontCheck,
    /// Act on the pre-game story browser. The browser has no `AppState`, so it
    /// cannot take an [`Action`]; its verbs are their own type and are applied
    /// by the picker loop. See [`crate::browser`] (SQ-0796).
    Browser(crate::browser::BrowserAction),
}

// ── V6PixelLockArg ────────────────────────────────────────────────────────────

/// Argument for `set-v6-pixel-lock`. Four states rather than the `Option<bool>`
/// its per-game siblings carry, because the bare form is a TOGGLE of the live
/// value and that is a different thing from `Auto` — which clears the override
/// and falls back to the global `v6_pixel_lock`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V6PixelLockArg {
    /// Lock this game's magnification to the ladder.
    On,
    /// Free-scale this game's magnification to fill the pane.
    Off,
    /// Clear the per-game override: inherit the global `v6_pixel_lock`.
    Auto,
    /// Flip whatever is in force, and persist the result for this game.
    Toggle,
}

// ── GuidanceArg / V6RenderArg ─────────────────────────────────────────────────

/// Argument for `set-guidance`. The same four states [`V6PixelLockArg`] carries,
/// and for the same reason: the bare form flips the LIVE value, which is not the
/// same request as `Auto` — clearing this game's override so the global setting
/// decides again (SQ-1123).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidanceArg {
    /// Light it for this game.
    On,
    /// Put it out for this game.
    Off,
    /// Clear the per-game override: inherit the global `guidance`.
    Auto,
    /// Flip whatever is in force, and persist the result for this game.
    Toggle,
}

/// Argument for `set-return-probe`. The same four states [`GuidanceArg`] carries,
/// and for the same reason: the bare form flips the LIVE value, which is not the
/// same request as `Auto` — clearing this game's override so the global setting
/// decides again (SQ-0785).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnProbeArg {
    /// Look for the way back, for this game.
    On,
    /// Stop looking, for this game.
    Off,
    /// Clear the per-game override: inherit the global `return_probe`.
    Auto,
    /// Flip whatever is in force, and persist the result for this game.
    Toggle,
}

/// Argument for `set-v6-render`. Three concrete modes, a bare CYCLE through
/// them, and `Auto` — which clears this game's override so the global
/// `v6_render` decides again (SQ-1123).
///
/// A cycle cannot walk through `Auto`: "inherit" has no glyph of its own and
/// would be indistinguishable on screen from whichever mode it resolved to, so
/// the border control reaches every concrete mode and the COMMAND is how you get
/// back to inheriting. That split is uniform across all four persisted controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V6RenderArg {
    /// Use exactly this mode for this game.
    Mode(crate::config::V6RenderMode),
    /// Step to the next mode (hybrid → raster → extended → hybrid).
    Cycle,
    /// Clear the per-game override: inherit the global `v6_render`.
    Auto,
}

// ── TranscriptFilterArg ───────────────────────────────────────────────────────

/// Argument for the `/filter` command. `main.rs` maps this to `state::TranscriptFilter`.
#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptFilterArg {
    Both,
    Story,
    Meta,
}

// ── Category ──────────────────────────────────────────────────────────────────

/// User-facing grouping for `/help` and the hotkey dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Game,
    Map,
    View,
    Transcript,
    Style,
    Export,
    Animation,
    Help,
    /// The pre-game story browser (SQ-0796). Not shown in the game's `/help`,
    /// because the browser has no command line to type them into — see
    /// [`help_text`].
    Library,
}

impl Category {
    pub const ORDER: [Category; 9] = [
        Category::Game,
        Category::Map,
        Category::View,
        Category::Transcript,
        Category::Style,
        Category::Export,
        Category::Animation,
        Category::Help,
        Category::Library,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Game => "Game",
            Category::Map => "Map",
            Category::View => "View",
            Category::Transcript => "Transcript",
            Category::Style => "Style",
            Category::Export => "Export",
            Category::Animation => "Animation",
            Category::Help => "Help",
            Category::Library => "Library",
        }
    }
}

// ── CommandSpec registry ──────────────────────────────────────────────────────

pub struct CommandSpec {
    pub name: &'static str,
    pub category: Category,
    pub context: Context,
    pub usage: &'static str,
    pub description: &'static str,
    pub dispatch: fn(&[&str]) -> SlashOutcome,
}

fn err(s: impl Into<String>) -> SlashOutcome { SlashOutcome::Error(s.into()) }

/// The command registry — the single source of truth for every command.
/// Add a new command by adding one entry here (see the module docs); nothing
/// else needs to change for it to appear in `/help`, Tab autocomplete, and the
/// parser. Keep entries grouped by `Category` in the display order of
/// `Category::ORDER`.
pub static COMMANDS: &[CommandSpec] = &[
    // ── Game ──────────────────────────────────────────────────────────────
    CommandSpec { name: "save-state", category: Category::Game, context: Context::Global,
        usage: "save-state [name]", description: "save an emulator Save State, optionally to a named slot",
        dispatch: |a| SlashOutcome::Save(a.first().map(|s| s.to_string())) },
    CommandSpec { name: "restore-state", category: Category::Game, context: Context::Global,
        usage: "restore-state [name]", description: "restore an emulator Save State — bare opens the saves dialog to pick one; a name restores that slot directly",
        dispatch: |a| match a.first() {
            Some(name) => SlashOutcome::Load(Some(name.to_string())),
            None => SlashOutcome::Action(crate::input::Action::OpenSaves),
        } },
    CommandSpec { name: "reset-game", category: Category::Game, context: Context::Global,
        usage: "reset-game [map] [data]", description: "restart the game — bare opens the options dialog; 'map' also clears the map, 'data' deletes the game's saved progress/cache so it starts fresh",
        dispatch: |a| SlashOutcome::Reset { map: a.contains(&"map"), data: a.contains(&"data") } },
    CommandSpec { name: "quit", category: Category::Game, context: Context::Global,
        usage: "quit", description: "exit lanthorn",
        dispatch: |_| SlashOutcome::Quit },
    CommandSpec { name: "quit-to-library", category: Category::Game, context: Context::Global,
        usage: "quit-to-library", description: "exit the current story and return to the story library",
        dispatch: |_| SlashOutcome::QuitToLibrary },
    CommandSpec { name: "open-hints", category: Category::Game, context: Context::Global,
        usage: "open-hints", description: "open the hints panel",
        dispatch: |_| SlashOutcome::OpenHints },
    CommandSpec { name: "open-history", category: Category::Game, context: Context::Global,
        usage: "open-history", description: "open the rewind/replay history",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenHistory) },
    CommandSpec { name: "open-command-band", category: Category::Game, context: Context::Global,
        usage: "open-command-band", description: "open or close the command band; persisted per-game",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenCommandBand) },
    CommandSpec { name: "toggle-timed-input", category: Category::Game, context: Context::Global,
        usage: "toggle-timed-input", description: "toggle honoring the game's timed-input timers",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleTimedInput) },
    CommandSpec { name: "toggle-sound", category: Category::Game, context: Context::Global,
        usage: "toggle-sound", description: "toggle audio playback (bleeps + sampled sounds)",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleSound) },
    CommandSpec { name: "volume", category: Category::Game, context: Context::Global,
        usage: "volume <0-100>", description: "set the master audio volume (0-100)",
        dispatch: |a| match a.first().and_then(|s| s.parse::<u8>().ok()) {
            Some(v) if v <= 100 => SlashOutcome::Action(crate::input::Action::SetVolume(v)),
            _ => err("volume requires an integer 0-100 (e.g. volume 60)"),
        } },
    CommandSpec { name: "play-sound", category: Category::Game, context: Context::Global,
        usage: "play-sound [n]", description: "diagnostic: list Snd resources, or play resource n",
        dispatch: |a| match a.first() {
            None => SlashOutcome::PlaySound(None),
            Some(s) => match s.parse::<u32>() {
                Ok(n) => SlashOutcome::PlaySound(Some(n)),
                Err(_) => err(format!("play-sound: expected a resource number, got '{s}'")),
            },
        } },

    // ── Map ───────────────────────────────────────────────────────────────
    CommandSpec { name: "pan-map", category: Category::Map, context: Context::Map,
        usage: "pan-map <dx> <dy>", description: "pan the map by dx columns and dy rows",
        dispatch: |a| {
            let dx = a.first().and_then(|s| s.parse::<i32>().ok());
            let dy = a.get(1).and_then(|s| s.parse::<i32>().ok());
            match (dx, dy) {
                (Some(x), Some(y)) => SlashOutcome::Action(crate::input::Action::Pan(x, y)),
                _ => err("pan-map requires two integers (e.g. pan-map -3 0)"),
            }
        } },
    CommandSpec { name: "zoom-map", category: Category::Map, context: Context::Map,
        usage: "zoom-map in|out|reset|<n>", description: "zoom the map in/out, reset, or step by signed n",
        dispatch: |a| {
            use crate::input::Action;
            match a.first().copied() {
                Some("in") => SlashOutcome::Action(Action::ZoomIn),
                Some("out") => SlashOutcome::Action(Action::ZoomOut),
                Some("reset") => SlashOutcome::Action(Action::ZoomReset),
                Some(s) => match s.parse::<i32>() {
                    Ok(0) => SlashOutcome::Action(Action::ZoomReset),
                    // Keep the magnitude (SQ-0355): collapsing every n to a single step made the
                    // usage's "step by signed n" a lie — `zoom-map 5` moved exactly as far as
                    // `zoom-map in`.
                    Ok(n) => SlashOutcome::Action(Action::ZoomBy(n)),
                    Err(_) => err(format!("zoom-map: expected in|out|reset|<integer>, got '{s}'")),
                },
                None => err("zoom-map requires an argument: in|out|reset|<n>"),
            }
        } },
    CommandSpec { name: "center-map", category: Category::Map, context: Context::Map,
        usage: "center-map", description: "re-center the map on the selected room, or the current one",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::Recenter) },
    CommandSpec { name: "tidy-map", category: Category::Map, context: Context::Map,
        usage: "tidy-map", description: "re-run the layout tidy",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::Retidy) },
    CommandSpec { name: "cycle-layer", category: Category::Map, context: Context::Map,
        usage: "cycle-layer next|prev|<n>", description: "switch map layer; n is a signed delta",
        dispatch: |a| {
            use crate::input::Action;
            match a.first().copied() {
                Some("next") => SlashOutcome::Action(Action::CycleLayer(1)),
                Some("prev") => SlashOutcome::Action(Action::CycleLayer(-1)),
                Some(s) => match s.parse::<i32>() {
                    Ok(n) => SlashOutcome::Action(Action::CycleLayer(n)),
                    Err(_) => err(format!("cycle-layer: expected next|prev|<integer delta>, got '{s}'")),
                },
                None => err("cycle-layer requires an argument: next|prev|<n>"),
            }
        } },
    CommandSpec { name: "select-room", category: Category::Map, context: Context::Map,
        usage: "select-room next|prev", description: "move the room selection",
        dispatch: |a| {
            use crate::input::Action;
            match a.first().copied() {
                Some("next") => SlashOutcome::Action(Action::SelectNext),
                Some("prev") => SlashOutcome::Action(Action::SelectPrev),
                _ => err("select-room requires an argument: next|prev"),
            }
        } },
    CommandSpec { name: "rename-room", category: Category::Map, context: Context::Map,
        usage: "rename-room", description: "rename the selected room",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::RenameRoom) },
    CommandSpec { name: "rename-layer", category: Category::Map, context: Context::Map,
        usage: "rename-layer", description: "rename the current layer",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::RenameLayer) },
    CommandSpec { name: "edit-notes", category: Category::Map, context: Context::Map,
        usage: "edit-notes", description: "edit the selected room's notes",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::EditNotes) },
    CommandSpec { name: "delete-connection", category: Category::Map, context: Context::Map,
        usage: "delete-connection", description: "delete the selected connection",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::DeleteSelectedConnection) },
    CommandSpec { name: "relabel-edge", category: Category::Map, context: Context::Map,
        usage: "relabel-edge", description: "relabel the selected edge",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::RelabelSelectedEdge) },
    // SQ-0439: peel and merge were always the same operation — re-home a set of rooms onto a
    // layer — differing only in whether the destination is minted or named. `peel-layer` and
    // `merge-layer` are retired in favour of the one verb that says so.
    CommandSpec { name: "move-region", category: Category::Map, context: Context::Map,
        usage: "move-region [new|parent|layer] [direction]",
        description: "re-home the selected room's region onto a fresh layer, its parent, or any named layer; bare picks both when only one choice is possible",
        // The destination may be a layer NAME with spaces in it ("Dead End"), and the seam is an
        // optional trailing direction, so neither can be split off here: the whole remainder goes
        // to `apply_action`, which has the live layer list to resolve it against. That includes
        // the EMPTY remainder — bare `move-region` auto-picks the seam and the destination when
        // each has only one possibility, which only the graph can answer (SQ-0439).
        dispatch: |a| SlashOutcome::Action(crate::input::Action::MoveRegion(a.join(" "))) },
    CommandSpec { name: "toggle-room-dock", category: Category::Map, context: Context::Map,
        usage: "toggle-room-dock", description: "open or close the room dock under the map",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleRoomDock) },
    CommandSpec { name: "toggle-inspector", category: Category::Map, context: Context::Map,
        usage: "toggle-inspector", description: "show the room dock's diagnostics view (flips back to info when open)",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleRoomDiagnostics) },
    CommandSpec { name: "load-map", category: Category::Map, context: Context::Global,
        usage: "load-map <path>", description: "load a standalone map file into the current session",
        dispatch: |a| match a.first() {
            Some(p) => SlashOutcome::LoadMap(p.to_string()),
            None => SlashOutcome::Error("load-map: a file path is required".into()),
        } },
    CommandSpec { name: "toggle-room-numbers", category: Category::Map, context: Context::Global,
        usage: "toggle-room-numbers", description: "toggle room-number labels",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleRoomNumbers) },
    CommandSpec { name: "view-map", category: Category::Map, context: Context::Global,
        usage: "view-map [drawn|matrix]", description: "how the active layer draws: bare cycles, a name sets it",
        dispatch: |a| {
            use crate::input::Action;
            use mapper::layer::MapView;
            match a.first().copied() {
                None => SlashOutcome::Action(Action::ViewMap(None)),
                Some(s) if s.eq_ignore_ascii_case("drawn") => SlashOutcome::Action(Action::ViewMap(Some(MapView::Drawn))),
                Some(s) if s.eq_ignore_ascii_case("matrix") => SlashOutcome::Action(Action::ViewMap(Some(MapView::Matrix))),
                Some(s) => err(format!("view-map: '{s}' is not a view (drawn | matrix)")),
            }
        } },
    CommandSpec { name: "mark-maze-layer", category: Category::Map, context: Context::Map,
        usage: "mark-maze-layer", description: "flag the active layer as a maze (defaults it to the matrix view)",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::MarkMazeLayer) },
        CommandSpec { name: "toggle-alignment", category: Category::Map, context: Context::Global,
        usage: "toggle-alignment", description: "toggle alignment guides",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleAlignment) },
    CommandSpec { name: "toggle-portal-labels", category: Category::Map, context: Context::Global,
        usage: "toggle-portal-labels", description: "toggle portal labels",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::TogglePortalLabels) },
    // ── View ──────────────────────────────────────────────────────────────
    CommandSpec { name: "toggle-map", category: Category::View, context: Context::Global,
        usage: "toggle-map", description: "show or hide the map panel; persisted per-game",
        dispatch: |_a| SlashOutcome::Action(crate::input::Action::ToggleMap) },
    CommandSpec { name: "toggle-focus", category: Category::View, context: Context::Global,
        usage: "toggle-focus", description: "switch focus between panes",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleFocus) },
    CommandSpec { name: "toggle-inventory", category: Category::View, context: Context::Global,
        usage: "toggle-inventory", description: "toggle the inventory strip",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleInventory) },
    CommandSpec { name: "toggle-status-bar", category: Category::View, context: Context::Global,
        usage: "toggle-status-bar", description: "toggle the status/score bar",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleStatusBar) },
    CommandSpec { name: "resize-panes", category: Category::View, context: Context::Global,
        usage: "resize-panes", description: "enter interactive pane-resize mode",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ResizePanes) },
    CommandSpec { name: "reset-pane-size", category: Category::View, context: Context::Global,
        usage: "reset-pane-size", description: "reset all pane sizes to their defaults",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ResizeReset) },

    // ── Transcript ────────────────────────────────────────────────────────
    CommandSpec { name: "search-transcript", category: Category::Transcript, context: Context::Global,
        usage: "search-transcript [query]", description: "search the transcript; no query repeats the last search",
        dispatch: |a| if a.is_empty() { SlashOutcome::Search(None) } else { SlashOutcome::Search(Some(a.join(" "))) } },
    CommandSpec { name: "filter-transcript", category: Category::Transcript, context: Context::Global,
        usage: "filter-transcript story|meta|both", description: "filter the transcript by category",
        dispatch: |a| match a.first().copied() {
            Some("story") => SlashOutcome::Filter(TranscriptFilterArg::Story),
            Some("meta")  => SlashOutcome::Filter(TranscriptFilterArg::Meta),
            Some("both")  => SlashOutcome::Filter(TranscriptFilterArg::Both),
            _ => err("filter-transcript: use story | meta | both"),
        } },
    CommandSpec { name: "export-transcript", category: Category::Transcript, context: Context::Global,
        usage: "export-transcript [file]", description: "export the visible transcript; default path when omitted",
        dispatch: |a| SlashOutcome::Export(a.first().map(|s| s.to_string())) },

    // ── Style ─────────────────────────────────────────────────────────────
    CommandSpec { name: "open-settings", category: Category::Style, context: Context::Global,
        usage: "open-settings", description: "open the global settings screen",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenConfig) },
    CommandSpec { name: "reload-style", category: Category::Style, context: Context::Global,
        usage: "reload-style", description: "reload style.toml from disk",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ReloadStyle) },
    CommandSpec { name: "toggle-watch", category: Category::Style, context: Context::Global,
        usage: "toggle-watch", description: "toggle live style-file watching",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleWatch) },
    CommandSpec { name: "print-colors", category: Category::Style, context: Context::Global,
        usage: "print-colors [color]", description: "print the current color scheme (color = actual colors)",
        dispatch: |a| SlashOutcome::PrintColors { actual: a.first() == Some(&"color") } },
    CommandSpec { name: "set-game-colours", category: Category::Style, context: Context::Global,
        usage: "set-game-colours on|off|auto", description: "force this game's own colours on/off (auto follows garglk.ini/global); persisted per-game",
        dispatch: |a| match a.first().copied() {
            Some("on")   => SlashOutcome::SetGameColours(Some(true)),
            Some("off")  => SlashOutcome::SetGameColours(Some(false)),
            Some("auto") => SlashOutcome::SetGameColours(None),
            _ => err("set-game-colours requires an argument: on | off | auto"),
        } },
    CommandSpec { name: "set-v6-render", category: Category::Style, context: Context::Global,
        usage: "set-v6-render [hybrid|raster|extended|auto]", description: "switch this game's v6 render mode — bare cycles hybrid → raster → extended, auto inherits the global setting; persisted per-game",
        dispatch: |a| {
            use crate::config::V6RenderMode;
            match a.first().copied() {
                None => SlashOutcome::SetV6Render(V6RenderArg::Cycle),
                Some("hybrid")    => SlashOutcome::SetV6Render(V6RenderArg::Mode(V6RenderMode::Hybrid)),
                Some("raster")    => SlashOutcome::SetV6Render(V6RenderArg::Mode(V6RenderMode::Raster)),
                Some("extended")  => SlashOutcome::SetV6Render(V6RenderArg::Mode(V6RenderMode::Extended)),
                Some("auto")      => SlashOutcome::SetV6Render(V6RenderArg::Auto),
                Some(s) => err(format!("set-v6-render: unknown mode '{s}' (hybrid | raster | extended | auto, or bare to cycle)")),
            }
        } },
    CommandSpec { name: "set-v6-pixel-lock", category: Category::Style, context: Context::Global,
        usage: "set-v6-pixel-lock [on|off|auto]", description: "lock v6 art to a whole number of device pixels per art pixel — bare toggles, auto inherits the global setting; persisted per-game",
        dispatch: |a| match a.first().copied() {
            None         => SlashOutcome::SetV6PixelLock(V6PixelLockArg::Toggle),
            Some("on")   => SlashOutcome::SetV6PixelLock(V6PixelLockArg::On),
            Some("off")  => SlashOutcome::SetV6PixelLock(V6PixelLockArg::Off),
            Some("auto") => SlashOutcome::SetV6PixelLock(V6PixelLockArg::Auto),
            Some(s) => err(format!("set-v6-pixel-lock: unknown argument '{s}' (on | off | auto, or bare to toggle)")),
        } },
    CommandSpec { name: "set-guidance", category: Category::Style, context: Context::Global,
        usage: "set-guidance [on|off|auto]", description: "Lanthorn's Guiding Light: help while you play, marked in the margin — bare toggles, auto inherits the global setting; persisted per-game",
        dispatch: |a| match a.first().copied() {
            None         => SlashOutcome::SetGuidance(GuidanceArg::Toggle),
            Some("on")   => SlashOutcome::SetGuidance(GuidanceArg::On),
            Some("off")  => SlashOutcome::SetGuidance(GuidanceArg::Off),
            Some("auto") => SlashOutcome::SetGuidance(GuidanceArg::Auto),
            Some(s) => err(format!("set-guidance: unknown argument '{s}' (on | off | auto, or bare to toggle)")),
        } },
    CommandSpec { name: "set-return-probe", category: Category::Style, context: Context::Global,
        usage: "set-return-probe [on|off|auto]", description: "after a move, look for the way back in a silent copy of the game and put it on the map — bare toggles, auto inherits the global setting; persisted per-game",
        dispatch: |a| match a.first().copied() {
            None         => SlashOutcome::SetReturnProbe(ReturnProbeArg::Toggle),
            Some("on")   => SlashOutcome::SetReturnProbe(ReturnProbeArg::On),
            Some("off")  => SlashOutcome::SetReturnProbe(ReturnProbeArg::Off),
            Some("auto") => SlashOutcome::SetReturnProbe(ReturnProbeArg::Auto),
            Some(s) => err(format!("set-return-probe: unknown argument '{s}' (on | off | auto, or bare to toggle)")),
        } },
    // A TRIGGER, not a setting — hence `reveal-`, not `set-`, and no argument to
    // take. It is the first of its kind among the border controls; see
    // `SlashOutcome::RevealWords` and `crate::reveal`.
    CommandSpec { name: "reveal-words", category: Category::Style, context: Context::Global,
        usage: "reveal-words", description: "light the nouns and named things on screen this story knows, for a few seconds — under the Guiding Light's switch",
        dispatch: |_| SlashOutcome::RevealWords },
    CommandSpec { name: "run-font-check", category: Category::Style, context: Context::Global,
        usage: "run-font-check", description: "ask which of two glyph rows your terminal's font draws properly, and set the map's arrow, portal and Guiding Light icons from the answer (writes style.toml)",
        // A verb-noun name, per the registry convention — and `run-`, not `set-`,
        // because the command does not set anything: it asks, and the ANSWER
        // sets. The same question `--font-check on` asks at launch, put where
        // someone who has just changed terminal fonts can reach it.
        dispatch: |_| SlashOutcome::RunFontCheck },
    CommandSpec { name: "set-game-borders", category: Category::Style, context: Context::Global,
        usage: "set-game-borders on|off|auto", description: "show this game's Glk window borders (on), or render borderless/abutting (off); auto = default (on); persisted per-game",
        // on = borders shown (default) → borderless=false; off = borderless/abut
        // → borderless=true; auto = clear the override.
        dispatch: |a| match a.first().copied() {
            Some("on")   => SlashOutcome::SetGameBorderless(Some(false)),
            Some("off")  => SlashOutcome::SetGameBorderless(Some(true)),
            Some("auto") => SlashOutcome::SetGameBorderless(None),
            _ => err("set-game-borders requires an argument: on | off | auto"),
        } },

    // ── Export ────────────────────────────────────────────────────────────
    CommandSpec { name: "export-svg", category: Category::Export, context: Context::Global,
        usage: "export-svg [file]", description: "export the map as SVG; default path when omitted",
        dispatch: |a| SlashOutcome::Action(crate::input::Action::ExportSvg(a.first().map(|s| s.to_string()))) },
    CommandSpec { name: "export-dot", category: Category::Export, context: Context::Global,
        usage: "export-dot [file]", description: "export the map as Graphviz DOT; default path when omitted",
        dispatch: |a| SlashOutcome::Action(crate::input::Action::ExportDot(a.first().map(|s| s.to_string()))) },
    CommandSpec { name: "export-map", category: Category::Export, context: Context::Global,
        usage: "export-map [file]", description: "dump the map structure; default path when omitted",
        dispatch: |a| SlashOutcome::Action(crate::input::Action::ExportMap(a.first().map(|s| s.to_string()))) },

    // ── Animation ─────────────────────────────────────────────────────────
    CommandSpec { name: "animate-tidy", category: Category::Animation, context: Context::Global,
        usage: "animate-tidy", description: "animate a tidy pass",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::AnimateTidy) },
    CommandSpec { name: "anim-step", category: Category::Animation, context: Context::Anim,
        usage: "anim-step forward|back", description: "step the animation one frame",
        dispatch: |a| {
            use crate::input::Action;
            match a.first().copied() {
                Some("forward") => SlashOutcome::Action(Action::AnimStep(1)),
                Some("back") => SlashOutcome::Action(Action::AnimStep(-1)),
                _ => err("anim-step requires an argument: forward|back"),
            }
        } },
    CommandSpec { name: "anim-play", category: Category::Animation, context: Context::Anim,
        usage: "anim-play", description: "toggle animation play/pause",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::AnimTogglePlay) },
    CommandSpec { name: "anim-exit", category: Category::Animation, context: Context::Anim,
        usage: "anim-exit", description: "exit the animation view",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::AnimExit) },

    // ── Help ──────────────────────────────────────────────────────────────
    CommandSpec { name: "dump-windows", category: Category::Help, context: Context::Global,
        usage: "dump-windows", description: "dump the last game frame's window layout, here and to ~/.lanthorn/dump-windows.log",
        dispatch: |_| SlashOutcome::DumpWindows },
    CommandSpec { name: "dump-cells", category: Category::Help, context: Context::Global,
        usage: "dump-cells", description: "write the last frame's cells — glyphs, colours and attributes — to ~/.lanthorn/dump-cells.log",
        dispatch: |_| SlashOutcome::DumpCells },
    CommandSpec { name: "dump-terminal", category: Category::Help, context: Context::Global,
        usage: "dump-terminal", description: "dump this terminal's detected protocol, cell size, capabilities and traffic — here and to ~/.lanthorn/dump-terminal.log",
        dispatch: |_| SlashOutcome::DumpTerminal },
    CommandSpec { name: "debug", category: Category::Help, context: Context::Global,
        usage: "debug", description: "toggle the Z-machine debug inspector pane",
        dispatch: |_| SlashOutcome::ToggleDebug },
    CommandSpec {
        name: "trace", category: Category::Help, context: Context::Global,
        usage: "trace [sections|all|none]",
        description: "toggle debug-trace sections (screen, map, hostio, v6) written to trace.log; no arg shows current state",
        dispatch: |a| SlashOutcome::Trace(a.first().map(|s| s.to_string())),
    },
    CommandSpec { name: "dump-notifications", category: Category::Help, context: Context::Global,
        usage: "dump-notifications", description: "print the notification history to the transcript, in case a toast was missed",
        dispatch: |_| SlashOutcome::DumpNotifications },
    CommandSpec { name: "help", category: Category::Help, context: Context::Global,
        usage: "help [command]", description: "list all commands by category; with a name, show one command's detail",
        dispatch: |_| SlashOutcome::Help },

    // ── Library (the pre-game story browser) ──────────────────────────────
    // SQ-0796. These run before there is an `AppState`, so they dispatch to a
    // `BrowserAction` rather than an `Action`, and `Context::Browser` keeps the
    // two sets from crossing. Bind them under `[keymap.browser]`.
    CommandSpec { name: "move-selection", category: Category::Library, context: Context::Browser,
        usage: "move-selection <dx> <dy>", description: "move the browser's selection by dx columns and dy rows (columns exist only in the cover gallery)",
        dispatch: |a| {
            use crate::browser::BrowserAction;
            let dx = a.first().and_then(|s| s.parse::<isize>().ok());
            let dy = a.get(1).and_then(|s| s.parse::<isize>().ok());
            match (dx, dy) {
                (Some(dx), Some(dy)) => SlashOutcome::Browser(BrowserAction::MoveSelection { dx, dy }),
                _ => err("move-selection requires two integers (e.g. move-selection 0 -1)"),
            }
        } },
    CommandSpec { name: "page-selection", category: Category::Library, context: Context::Browser,
        usage: "page-selection <n>", description: "move the browser's selection by n pages",
        dispatch: |a| match a.first().and_then(|s| s.parse::<isize>().ok()) {
            Some(n) => SlashOutcome::Browser(crate::browser::BrowserAction::PageSelection(n)),
            None => err("page-selection requires an integer (e.g. page-selection -1)"),
        } },
    CommandSpec { name: "half-page-selection", category: Category::Library, context: Context::Browser,
        usage: "half-page-selection <n>", description: "move the browser's selection by half a page (vim Ctrl-U/Ctrl-D)",
        dispatch: |a| match a.first().and_then(|s| s.parse::<isize>().ok()) {
            Some(n) => SlashOutcome::Browser(crate::browser::BrowserAction::HalfPageSelection(n)),
            None => err("half-page-selection requires an integer (e.g. half-page-selection -1)"),
        } },
    CommandSpec { name: "select-edge", category: Category::Library, context: Context::Browser,
        usage: "select-edge first|last", description: "jump the browser's selection to the first or last story",
        dispatch: |a| {
            use crate::browser::{BrowserAction, Edge};
            match a.first().copied() {
                Some("first") => SlashOutcome::Browser(BrowserAction::SelectEdge(Edge::First)),
                Some("last") => SlashOutcome::Browser(BrowserAction::SelectEdge(Edge::Last)),
                _ => err("select-edge requires an argument: first | last"),
            }
        } },
    CommandSpec { name: "play-story", category: Category::Library, context: Context::Browser,
        usage: "play-story", description: "launch the selected story",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::PlayStory) },
    CommandSpec { name: "open-launch-options", category: Category::Library, context: Context::Browser,
        usage: "open-launch-options", description: "open the launch-options dialog for the selected story",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::OpenLaunchOptions) },
    CommandSpec { name: "open-story-menu", category: Category::Library, context: Context::Browser,
        usage: "open-story-menu", description: "open the per-story menu beside the selected story",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::OpenStoryMenu) },
    CommandSpec { name: "show-browser-keys", category: Category::Library, context: Context::Browser,
        usage: "show-browser-keys", description: "show the story browser's key reference",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::ShowBrowserKeys) },
    CommandSpec { name: "toggle-info-panel", category: Category::Library, context: Context::Browser,
        usage: "toggle-info-panel", description: "open or close the browser's story info panel",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::ToggleInfoPanel) },
    CommandSpec { name: "toggle-gallery", category: Category::Library, context: Context::Browser,
        usage: "toggle-gallery", description: "switch the browser between the story list and the cover gallery",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::ToggleGallery) },
    CommandSpec { name: "fetch-story", category: Category::Library, context: Context::Browser,
        usage: "fetch-story", description: "re-fetch the selected story's IFDB metadata, ignoring the cache",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::FetchStory) },
    CommandSpec { name: "refresh-library", category: Category::Library, context: Context::Browser,
        usage: "refresh-library", description: "fetch IFDB metadata for every story that is missing or stale",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::RefreshLibrary) },
    CommandSpec { name: "set-ifdb-url", category: Category::Library, context: Context::Browser,
        usage: "set-ifdb-url", description: "point the selected story at an IFDB page by hand",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::SetIfdbUrl) },
    CommandSpec { name: "open-url", category: Category::Library, context: Context::Browser,
        usage: "open-url", description: "download a story from a URL into this library and open it",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::OpenUrl) },
    CommandSpec { name: "search-ifdb", category: Category::Library, context: Context::Browser,
        usage: "search-ifdb", description: "search IFDB by title or author and download a story into this directory",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::SearchIfdb) },
    CommandSpec { name: "download-hints", category: Category::Library, context: Context::Browser,
        usage: "download-hints", description: "download a matching InvisiClues hint file for the selected story",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::DownloadHints) },
    CommandSpec { name: "sort-library", category: Category::Library, context: Context::Browser,
        usage: "sort-library", description: "cycle the browser's sort column, keeping the direction",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::SortLibrary) },
    CommandSpec { name: "reverse-sort", category: Category::Library, context: Context::Browser,
        usage: "reverse-sort", description: "reverse the browser's sort direction, keeping the column",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::ReverseSort) },
    CommandSpec { name: "find-story", category: Category::Library, context: Context::Browser,
        usage: "find-story", description: "type to filter the whole library by title, author, filename or folder",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::FindStory) },
    CommandSpec { name: "parent-folder", category: Category::Library, context: Context::Browser,
        usage: "parent-folder", description: "leave the current library folder for the one above it",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::ParentFolder) },
    CommandSpec { name: "quit-browser", category: Category::Library, context: Context::Browser,
        usage: "quit-browser", description: "leave the story browser",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::QuitBrowser) },
    CommandSpec { name: "cancel-browser", category: Category::Library, context: Context::Browser,
        usage: "cancel-browser", description: "cancel a running fetch, or leave the browser when nothing is in flight",
        dispatch: |_| SlashOutcome::Browser(crate::browser::BrowserAction::CancelBrowser) },
];

pub fn find_command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.name == name)
}

// ── parse ─────────────────────────────────────────────────────────────────────

/// Parse a slash-command body (the text AFTER the leading prefix, e.g. `/`).
///
/// `prefix` is the configured command prefix character, used only in user-facing
/// error/help display strings. Routing and matching logic is unaffected.
///
/// Routes entirely through the `COMMANDS` registry. Special case:
/// `search-transcript` passes the raw remainder of the line preserving internal
/// whitespace. All other commands receive split tokens.
pub fn parse(body: &str, prefix: char) -> SlashOutcome {
    parse_in_context(body, prefix, Context::Global)
}

pub fn parse_in_context(body: &str, prefix: char, ctx: Context) -> SlashOutcome {
    let Some(t0) = body.split_whitespace().next() else {
        return SlashOutcome::Error(format!("type {prefix}help for commands"));
    };

    // help: bare `help` → Help; `help <name>` → HelpCommand.
    if t0 == "help" {
        let rest = body.split_whitespace().nth(1);
        return match rest {
            Some(name) => SlashOutcome::HelpCommand(name.to_string()),
            None => SlashOutcome::Help,
        };
    }

    // search-transcript: preserve internal whitespace in the query.
    if t0 == "search-transcript" {
        // Slice from where the token actually STARTS, not from byte 0: the body may
        // carry leading whitespace (a custom prefix typed with a space after it, or a
        // key bound to a command string with one). Measuring from 0 both mangled the
        // query ("t twisty") and, with enough multi-byte leading space, sliced through
        // the middle of a char and panicked. (SQ-0654)
        let t0_at = body.len() - body.trim_start().len(); // the first token starts after the leading whitespace
        let remainder = body[t0_at + t0.len()..].trim_start().trim_end();
        return if remainder.is_empty() { SlashOutcome::Search(None) }
               else { SlashOutcome::Search(Some(remainder.to_string())) };
    }

    let Some(spec) = find_command(t0) else {
        return SlashOutcome::Error(format!("unknown command: {prefix}{t0} — try {prefix}help"));
    };

    if spec.context == Context::Anim && ctx != Context::Anim {
        return SlashOutcome::Error(format!("{} is only available during animation playback", spec.name));
    }

    // The browser's commands and the game's are disjoint worlds, so the gate runs
    // both ways (SQ-0796). A browser command in the game has no browser to act on;
    // a game command in the browser has no `AppState` to act on, and would
    // otherwise hand the picker an outcome it cannot apply.
    if spec.context == Context::Browser && ctx != Context::Browser {
        return SlashOutcome::Error(format!("{} is only available in the story browser", spec.name));
    }
    if ctx == Context::Browser && spec.context != Context::Browser {
        return SlashOutcome::Error(format!("{} is not available in the story browser", spec.name));
    }

    let tokens: Vec<&str> = body.split_whitespace().collect();
    (spec.dispatch)(&tokens[1..])
}

// ── slash_names ───────────────────────────────────────────────────────────────

/// All known slash-command names (for Tab autocomplete).
///
/// Returns the registry command names, minus the story browser's: that surface
/// has no command line, so completing a name there is impossible and offering it
/// here would complete a command the game then refuses (SQ-0796).
pub fn slash_names() -> Vec<String> {
    COMMANDS.iter()
        .filter(|c| c.context != Context::Browser)
        .map(|c| c.name.to_string())
        .collect()
}

// ── help_text / help_for_command ──────────────────────────────────────────────

/// Lines to display when the user types the help command.
///
/// `prefix` is the configured command prefix character used in all display strings.
/// Commands are grouped by category in `Category::ORDER` order, sorted by name
/// within each group.
///
/// The story browser's commands are omitted: this list is what you can *type*,
/// and the browser is a pre-game loop with no command line (SQ-0796). They are
/// documented as key bindings instead — see `docs/internals/customization.md`.
pub fn help_text(prefix: char) -> Vec<String> {
    let mut lines = vec![
        format!("Slash commands (type {prefix}<command> [args]):"),
        String::new(),
    ];
    for cat in Category::ORDER {
        let mut group: Vec<&CommandSpec> = COMMANDS.iter()
            .filter(|c| c.category == cat && c.context != Context::Browser)
            .collect();
        if group.is_empty() { continue; }
        group.sort_by_key(|c| c.name);
        lines.push(format!("{}:", cat.title()));
        for c in group {
            lines.push(format!("  {prefix}{}  — {}", c.usage, c.description));
        }
        lines.push(String::new());
    }
    lines
}

/// Lines to display for a single command's detail.
///
/// Returns the command's usage and description, or an unknown-command message.
pub fn help_for_command(prefix: char, name: &str) -> Vec<String> {
    match find_command(name) {
        Some(c) => vec![format!("  {prefix}{}  — {}", c.usage, c.description)],
        None => vec![format!("unknown command: {prefix}{name} — try {prefix}help")],
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// SQ-0439: one verb carries the whole remainder — destination and optional seam alike —
    /// because a layer name may contain spaces ("Dead End") and only the live layer list can
    /// say where the name ends and a direction begins.
    #[test]
    fn move_region_passes_its_whole_argument_through() {
        use crate::input::Action;
        assert!(matches!(
            parse("move-region new", '/'),
            SlashOutcome::Action(Action::MoveRegion(ref s)) if s == "new"
        ));
        assert!(matches!(
            parse("move-region new east", '/'),
            SlashOutcome::Action(Action::MoveRegion(ref s)) if s == "new east"
        ));
        assert!(matches!(
            parse("move-region main", '/'),
            SlashOutcome::Action(Action::MoveRegion(ref s)) if s == "main"
        ));
        assert!(matches!(
            parse("move-region Dead End", '/'),
            SlashOutcome::Action(Action::MoveRegion(ref s)) if s == "Dead End"
        ));
        // Bare is a real form, not an error: the destination auto-picks when only one is
        // possible and asks otherwise, and only the live graph can tell which (SQ-0439). So the
        // empty remainder goes through like any other.
        assert!(matches!(
            parse("move-region", '/'),
            SlashOutcome::Action(Action::MoveRegion(ref s)) if s.is_empty()
        ));
    }

    /// The retired verbs are gone outright — pre-release, so no aliases (SQ-0439).
    #[test]
    fn peel_layer_and_merge_layer_are_retired() {
        assert!(find_command("peel-layer").is_none());
        assert!(find_command("merge-layer").is_none());
        assert!(find_command("move-region").is_some());
    }

    #[test]
    fn parse_registry_and_errors() {
        use crate::input::Action;
        assert!(matches!(parse("pan-map -1 0", '/'), SlashOutcome::Action(Action::Pan(-1, 0))));
        assert!(matches!(parse("pan-map 0 2", '/'), SlashOutcome::Action(Action::Pan(0, 2))));
        assert!(matches!(parse("zoom-map reset", '/'), SlashOutcome::Action(Action::ZoomReset)));
        // SQ-0355: the usage promises "step by signed n", so the magnitude must survive parsing.
        // It used to collapse to a single ZoomIn/ZoomOut, making `zoom-map 5` a synonym for
        // `zoom-map in`.
        assert!(matches!(parse("zoom-map 2", '/'), SlashOutcome::Action(Action::ZoomBy(2))));
        assert!(matches!(parse("zoom-map -3", '/'), SlashOutcome::Action(Action::ZoomBy(-3))));
        assert!(matches!(parse("zoom-map 1", '/'), SlashOutcome::Action(Action::ZoomBy(1))));
        // 0 still means "reset", and a non-integer is still an error.
        assert!(matches!(parse("zoom-map 0", '/'), SlashOutcome::Action(Action::ZoomReset)));
        assert!(matches!(parse("zoom-map wat", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("cycle-layer next", '/'), SlashOutcome::Action(Action::CycleLayer(1))));
        assert!(matches!(parse("save-state foo", '/'), SlashOutcome::Save(Some(_))));
        assert!(matches!(parse("save-state", '/'), SlashOutcome::Save(None)));
        // restore-state: a name restores that slot directly; bare opens the saves dialog.
        assert!(matches!(parse("restore-state foo", '/'), SlashOutcome::Load(Some(_))));
        assert!(matches!(parse("restore-state", '/'), SlashOutcome::Action(Action::OpenSaves)));
        assert!(matches!(parse("reset-game map", '/'), SlashOutcome::Reset { map: true, data: false }));
        assert!(matches!(parse("reset-game", '/'), SlashOutcome::Reset { map: false, data: false }));
        assert!(matches!(parse("reset-game data", '/'), SlashOutcome::Reset { map: false, data: true }));
        assert!(matches!(parse("reset-game map data", '/'), SlashOutcome::Reset { map: true, data: true }));
        assert!(matches!(parse("reset-game data map", '/'), SlashOutcome::Reset { map: true, data: true }));
        assert!(matches!(parse("quit", '/'), SlashOutcome::Quit));
        assert!(matches!(parse("help", '/'), SlashOutcome::Help));
        // in registry:
        assert!(matches!(parse("open-settings", '/'), SlashOutcome::Action(_)));
        // errors:
        assert!(matches!(parse("panh", '/'), SlashOutcome::Error(_)));   // no longer in registry
        assert!(matches!(parse("nope", '/'), SlashOutcome::Error(_)));   // unknown
        assert!(matches!(parse("", '/'), SlashOutcome::Error(_)));       // bare prefix
        // old short names now error (clean break):
        assert!(matches!(parse("save", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("pan", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("zoom", '/'), SlashOutcome::Error(_)));
    }

    #[test]
    fn slash_names_returns_registry() {
        let n = slash_names();
        assert!(n.iter().any(|s| s == "pan-map")); // registry name
        assert!(n.iter().any(|s| s == "open-settings")); // registry name
        assert!(!n.iter().any(|s| s == "panh")); // old curated name, not in registry
    }

    #[test]
    fn help_text_uses_prefix() {
        let lines = help_text('/');
        assert!(lines[0].contains('/'));
        let lines_semi = help_text(';');
        assert!(lines_semi[0].contains(';'));
        assert!(!lines_semi[0].contains('/'));
    }

    #[test]
    fn help_text_lists_registry_commands() {
        // Every registry command's usage must appear in /help.
        let lines = help_text('/');
        assert!(
            lines.iter().any(|l| l.contains("/open-settings")),
            "open-settings should appear in /help"
        );
        assert!(
            lines.iter().any(|l| l.contains("/save-state")),
            "save-state should appear in /help"
        );
        assert!(
            lines.iter().any(|l| l.contains("/zoom-map")),
            "zoom-map should appear in /help"
        );
    }

    #[test]
    fn quit_to_library_parses_to_quit_to_library() {
        assert!(matches!(parse("quit-to-library", '/'), SlashOutcome::QuitToLibrary));
        assert_eq!(find_command("quit-to-library").expect("quit-to-library").category, Category::Game);
    }

    #[test]
    fn slash_hint_parses_to_open_hints() {
        assert!(matches!(crate::slash::parse("open-hints", '/'), crate::slash::SlashOutcome::OpenHints));
        // old short names no longer resolve (clean break):
        assert!(matches!(crate::slash::parse("hint", '/'), crate::slash::SlashOutcome::Error(_)));
        assert!(matches!(crate::slash::parse("hints", '/'), crate::slash::SlashOutcome::Error(_)));
    }

    #[test]
    fn load_map_parses_path_argument() {
        assert!(matches!(parse("load-map ~/Downloads/map.json", '/'),
            SlashOutcome::LoadMap(p) if p == "~/Downloads/map.json"));
    }

    #[test]
    fn load_map_without_path_is_an_error() {
        assert!(matches!(parse("load-map", '/'), SlashOutcome::Error(_)));
    }

    #[test]
    fn parse_search_filter_export() {
        assert!(matches!(parse("search-transcript twisty maze", '/'), SlashOutcome::Search(Some(q)) if q == "twisty maze"));
        assert!(matches!(parse("search-transcript a  b", '/'), SlashOutcome::Search(Some(q)) if q == "a  b"));
        assert!(matches!(parse("search-transcript", '/'), SlashOutcome::Search(None)));
        assert!(matches!(parse("filter-transcript meta", '/'), SlashOutcome::Filter(TranscriptFilterArg::Meta)));
        assert!(matches!(parse("filter-transcript both", '/'), SlashOutcome::Filter(TranscriptFilterArg::Both)));
        assert!(matches!(parse("filter-transcript nope", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("export-transcript", '/'), SlashOutcome::Export(None)));
        assert!(matches!(parse("export-transcript out.txt", '/'), SlashOutcome::Export(Some(f)) if f == "out.txt"));
        // old short names now error (clean break):
        assert!(matches!(parse("search", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("filter", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("export", '/'), SlashOutcome::Error(_)));
    }

    #[test]
    fn search_transcript_slices_from_the_token_not_byte_zero() {
        // SQ-0654: a body with LEADING whitespace (a prefix typed with a space after
        // it, or a key bound to " search-transcript twisty") must still yield the
        // query itself — not the tail of the command name shifted by the leading run.
        assert!(matches!(parse(" search-transcript twisty", '/'),
            SlashOutcome::Search(Some(q)) if q == "twisty"), "leading space must not shift the slice");
        assert!(matches!(parse("   search-transcript twisty maze", '/'),
            SlashOutcome::Search(Some(q)) if q == "twisty maze"));
        assert!(matches!(parse("\tsearch-transcript a  b", '/'),
            SlashOutcome::Search(Some(q)) if q == "a  b"), "internal whitespace still preserved");
        assert!(matches!(parse("  search-transcript  ", '/'), SlashOutcome::Search(None)));
        // Multi-byte leading whitespace: six U+3000 ideographic spaces are 18 bytes,
        // so slicing from byte 0 by the token's LENGTH landed mid-char and panicked.
        assert!(matches!(parse("\u{3000}\u{3000}\u{3000}\u{3000}\u{3000}\u{3000}search-transcript twisty", '/'),
            SlashOutcome::Search(Some(q)) if q == "twisty"), "ideographic leading space must not panic or mangle");
        // And with the query itself starting past a multi-byte separator.
        assert!(matches!(parse("\u{3000}search-transcript\u{3000}twisty", '/'),
            SlashOutcome::Search(Some(q)) if q == "twisty"));
    }

    #[test]
    fn parse_routes_registry_and_gates_anim() {
        use crate::input::Action;
        use crate::keymap::Context;
        assert!(matches!(parse("pan-map -1 0", '/'), SlashOutcome::Action(Action::Pan(-1, 0))));
        assert!(matches!(parse("zoom-map in", '/'), SlashOutcome::Action(Action::ZoomIn)));
        assert!(matches!(parse("select-room next", '/'), SlashOutcome::Action(Action::SelectNext)));
        assert!(matches!(parse("save-state foo", '/'), SlashOutcome::Save(Some(_))));
        assert!(matches!(parse("reset-game map", '/'), SlashOutcome::Reset { map: true, data: false }));
        assert!(matches!(parse("quit", '/'), SlashOutcome::Quit));
        // Old short names no longer resolve (clean break).
        assert!(matches!(parse("center", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("panh", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("nope", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("", '/'), SlashOutcome::Error(_)));
        // Context gating: anim-step outside Anim errors; inside Anim it fires.
        assert!(matches!(parse_in_context("anim-step forward", '/', Context::Global), SlashOutcome::Error(_)));
        assert!(matches!(parse_in_context("anim-step forward", '/', Context::Anim), SlashOutcome::Action(Action::AnimStep(1))));
        // search-transcript preserves internal whitespace.
        assert!(matches!(parse("search-transcript a  b", '/'), SlashOutcome::Search(Some(q)) if q == "a  b"));
    }

    #[test]
    fn category_order_and_titles() {
        assert_eq!(Category::ORDER.len(), 9);
        assert_eq!(Category::ORDER[0], Category::Game);
        assert_eq!(Category::ORDER[7], Category::Help);
        assert_eq!(Category::ORDER[8], Category::Library);
        assert_eq!(Category::Game.title(), "Game");
        assert_eq!(Category::Animation.title(), "Animation");
    }

    #[test]
    fn registry_is_complete_and_well_formed() {
        use std::collections::HashSet;
        // Names unique.
        let mut seen = HashSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.name), "duplicate command name: {}", c.name);
            assert!(!c.usage.is_empty(), "{} has empty usage", c.name);
            assert!(!c.description.is_empty(), "{} has empty description", c.name);
        }
        // Verb-noun lint: every name contains '-' except the whitelist.
        for c in COMMANDS {
            if c.name == "quit" || c.name == "help" || c.name == "volume" || c.name == "trace" || c.name == "debug" { continue; }
            assert!(c.name.contains('-'), "non-verb-noun command name: {}", c.name);
        }
        // Spot-check representative commands exist with the right category.
        let by = |n: &str| COMMANDS.iter().find(|c| c.name == n).expect(n);
        assert_eq!(by("save-state").category, Category::Game);
        assert_eq!(by("zoom-map").category, Category::Map);
        assert_eq!(by("anim-step").context, Context::Anim);
        // Total count matches the spec table (Game 12, Map 21, View 6,
        // Transcript 3, Style 10, Export 3, Animation 4, Help 3). `open-saves`
        // was removed — `restore-state` (bare) opens the saves dialog instead.
        // `debug` (SQ-0169) opens the Z-machine debug inspector. `open-gallery`
        // and `open-style-editor` were removed (SQ-0309): the interactive
        // gallery/style-editor UIs are gone; `reload-style` remains.
        // `quit-to-library` (SQ-0435) exits to the story picker.
        // `set-v6-render` switches/cycles the v6 render mode live.
        // SQ-0945 added `set-v6-pixel-lock`, the runtime switch for the
        // magnification ladder, persisted in the per-game sidecar.
        // `nudge-room` was removed with Manual layout mode (SQ-0600).
        // `toggle-untried-exits` was retired with the overlay it drove (SQ-0666); `view-map`
        // and `mark-maze-layer` arrived with the matrix view.
        // SQ-0692 added `toggle-room-dock`; `toggle-inspector` kept its name and
        // now flips the SAME dock to its diagnostics body.
        // SQ-0761 added `dump-cells`, the cell-buffer half of `dump-windows`.
        // SQ-0994 added `dump-terminal`, the terminal-and-traffic half of the same
        // family: what was detected about the terminal, and which of those numbers
        // lanthorn measured rather than guessed.
        // SQ-0796 added the 16-command Library group — the pre-game story
        // browser's own keys, which used to be hardcoded match arms outside the
        // registry entirely.
        // SQ-0439 retired `peel-layer` and `merge-layer` for the one verb they
        // always were, `move-region` — two entries out, one in.
        // SQ-1086 added `open-url`: a URL is accepted wherever a story path is,
        // and this is the browser's door to one.
        // SQ-1045 added `set-guidance`, the switch for Lanthorn's Guiding Light —
        // the assist set as a whole, not one feature of it.
        // SQ-1104 added `run-font-check`, the re-runnable first-run font check.
        // SQ-0785 added `set-return-probe`, the switch for the automap's search
        // for the way back — the sixth border control and the first off-by-default.
        // SQ-1107 added `reveal-words`, the momentary word reveal — the seventh
        // border control and the first that is a TRIGGER rather than a switch:
        // nothing to read off it, nothing persisted, it just happens.
        // `find-story` and `parent-folder` arrived with the picker's folder
        // navigation and its in-memory library find: two more Library commands.
        // SQ-1228 added `half-page-selection`, the vim Ctrl-U/Ctrl-D convention
        // for the browser's list view.
        // SQ-1227 added `open-story-menu` and `show-browser-keys` — the browser's
        // per-story menu and its own key reference, which between them are what
        // let the footer shrink to one key per hint.
        assert_eq!(COMMANDS.len(), 89, "registry must match the spec's Full command table");
    }

    /// SQ-0796: `Category::ORDER` must list every category, or a whole group of
    /// commands silently vanishes from `/help`.
    #[test]
    fn category_order_covers_every_category_in_use() {
        for c in COMMANDS {
            assert!(
                Category::ORDER.contains(&c.category),
                "{} is in a category missing from Category::ORDER",
                c.name
            );
        }
    }

    #[test]
    fn sound_commands_present() {
        let by = |n: &str| COMMANDS.iter().find(|c| c.name == n).expect(n);
        assert_eq!(by("toggle-sound").category, Category::Game);
        assert_eq!(by("volume").category, Category::Game);
    }

    #[test]
    fn help_text_grouped_and_per_command() {
        let lines = help_text('/');
        // Category headers appear in order.
        let game_at = lines.iter().position(|l| l.contains("Game")).unwrap();
        let map_at = lines.iter().position(|l| l.contains("Map")).unwrap();
        assert!(game_at < map_at, "Game group precedes Map group");
        // Every command's usage shows up.
        assert!(lines.iter().any(|l| l.contains("/zoom-map")));

        // Per-command detail.
        let one = help_for_command('/', "zoom-map");
        assert!(one.iter().any(|l| l.contains("zoom-map in|out|reset")));
        assert!(one.iter().any(|l| l.contains("zoom the map")));
        let bad = help_for_command('/', "nope");
        assert!(bad.iter().any(|l| l.contains("unknown command")));

        // `help <command>` parses to HelpCommand; bare help to Help.
        assert!(matches!(parse("help", '/'), SlashOutcome::Help));
        assert!(matches!(parse("help zoom-map", '/'), SlashOutcome::HelpCommand(n) if n == "zoom-map"));
    }

    #[test]
    fn print_colors_command_parses_flag() {
        assert!(find_command("print-colors").is_some());
        assert!(matches!(parse("print-colors", '/'), SlashOutcome::PrintColors { actual: false }));
        assert!(matches!(parse("print-colors color", '/'), SlashOutcome::PrintColors { actual: true }));
    }

    #[test]
    fn dump_windows_command_parses() {
        assert!(find_command("dump-windows").is_some());
        assert!(matches!(parse("dump-windows", '/'), SlashOutcome::DumpWindows));
    }

    /// SQ-0994. The description names BOTH destinations, because the on-screen
    /// copy of a v6 pane cannot be selected (SQ-0756) and the file is what a bug
    /// report actually carries — a command that writes a log the user is never
    /// told about is a log nobody reads.
    #[test]
    fn dump_terminal_command_parses_and_names_its_log() {
        let spec = find_command("dump-terminal").expect("dump-terminal is registered");
        assert!(matches!(parse("dump-terminal", '/'), SlashOutcome::DumpTerminal));
        assert!(spec.description.contains("dump-terminal.log"), "{}", spec.description);
        assert_eq!(spec.category, Category::Help);
    }

    #[test]
    fn debug_command_parses_to_toggle_debug() {
        assert!(find_command("debug").is_some());
        assert!(matches!(parse("debug", '/'), SlashOutcome::ToggleDebug));
    }

    #[test]
    fn trace_command_parses_set_and_show() {
        assert!(find_command("trace").is_some());
        let set = parse("trace screen,map", '/');
        assert!(matches!(set, SlashOutcome::Trace(Some(ref s)) if s == "screen,map"));
        let show = parse("trace", '/');
        assert!(matches!(show, SlashOutcome::Trace(None)));
    }

    #[test]
    fn play_sound_command_parses_optional_number() {
        assert!(matches!(parse("play-sound", '/'), SlashOutcome::PlaySound(None)));
        assert!(matches!(parse("play-sound 3", '/'), SlashOutcome::PlaySound(Some(3))));
        assert!(matches!(parse("play-sound nope", '/'), SlashOutcome::Error(_)));
    }

    #[test]
    fn play_sound_command_present() {
        assert_eq!(find_command("play-sound").expect("play-sound").category, Category::Game);
    }

    #[test]
    fn help_for_command_round_trip() {
        // The run loop calls help_for_command on a HelpCommand(name); verify the
        // function exists with the expected signature and returns non-empty lines.
        assert!(!help_for_command('/', "save-state").is_empty());
    }

    #[test]
    fn set_game_colours_parses_on_off_auto() {
        assert!(matches!(parse("set-game-colours on", '/'), SlashOutcome::SetGameColours(Some(true))));
        assert!(matches!(parse("set-game-colours off", '/'), SlashOutcome::SetGameColours(Some(false))));
        assert!(matches!(parse("set-game-colours auto", '/'), SlashOutcome::SetGameColours(None)));
        assert!(matches!(parse("set-game-colours", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("set-game-colours maybe", '/'), SlashOutcome::Error(_)));
        assert_eq!(find_command("set-game-colours").expect("set-game-colours").category, Category::Style);
    }

    #[test]
    fn set_game_borders_parses_on_off_auto() {
        // on = show borders (borderless=false); off = borderless (true); auto = clear.
        assert!(matches!(parse("set-game-borders on", '/'), SlashOutcome::SetGameBorderless(Some(false))));
        assert!(matches!(parse("set-game-borders off", '/'), SlashOutcome::SetGameBorderless(Some(true))));
        assert!(matches!(parse("set-game-borders auto", '/'), SlashOutcome::SetGameBorderless(None)));
        assert!(matches!(parse("set-game-borders", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("set-game-borders maybe", '/'), SlashOutcome::Error(_)));
        assert_eq!(find_command("set-game-borders").expect("set-game-borders").category, Category::Style);
    }

    #[test]
    fn set_v6_render_parses_modes_and_bare_cycles() {
        use crate::config::V6RenderMode;
        assert!(matches!(parse("set-v6-render hybrid", '/'), SlashOutcome::SetV6Render(V6RenderArg::Mode(V6RenderMode::Hybrid))));
        assert!(matches!(parse("set-v6-render raster", '/'), SlashOutcome::SetV6Render(V6RenderArg::Mode(V6RenderMode::Raster))));
        assert!(matches!(parse("set-v6-render extended", '/'), SlashOutcome::SetV6Render(V6RenderArg::Mode(V6RenderMode::Extended))));
        // SQ-0895 removed `frameless`; it must now be rejected like any other
        // unknown token rather than silently parsing to a mode.
        assert!(matches!(parse("set-v6-render frameless", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("set-v6-render", '/'), SlashOutcome::SetV6Render(V6RenderArg::Cycle)));
        assert!(matches!(parse("set-v6-render auto", '/'), SlashOutcome::SetV6Render(V6RenderArg::Auto)));
        assert!(matches!(parse("set-v6-render sepia", '/'), SlashOutcome::Error(_)));
        assert_eq!(find_command("set-v6-render").expect("set-v6-render").category, Category::Style);
    }

    /// SQ-0945: the runtime switch for SQ-0936's magnification ladder. Bare is a
    /// TOGGLE — the form a key binding wants — and is distinct from `auto`, which
    /// clears this game's override rather than flipping it.
    #[test]
    fn set_v6_pixel_lock_parses_on_off_auto_and_bare_toggles() {
        assert!(matches!(parse("set-v6-pixel-lock on", '/'), SlashOutcome::SetV6PixelLock(V6PixelLockArg::On)));
        assert!(matches!(parse("set-v6-pixel-lock off", '/'), SlashOutcome::SetV6PixelLock(V6PixelLockArg::Off)));
        assert!(matches!(parse("set-v6-pixel-lock auto", '/'), SlashOutcome::SetV6PixelLock(V6PixelLockArg::Auto)));
        assert!(matches!(parse("set-v6-pixel-lock", '/'), SlashOutcome::SetV6PixelLock(V6PixelLockArg::Toggle)));
        assert!(matches!(parse("set-v6-pixel-lock maybe", '/'), SlashOutcome::Error(_)));
        // SQ-1045: on/off, bare toggles, anything else is an error rather than a
        // silent no-op — the same shape, one state shorter (there is no `auto`:
        // guidance is one global setting with nothing per-game to inherit from).
        assert!(matches!(parse("set-guidance on", '/'), SlashOutcome::SetGuidance(GuidanceArg::On)));
        assert!(matches!(parse("set-guidance off", '/'), SlashOutcome::SetGuidance(GuidanceArg::Off)));
        assert!(matches!(parse("set-guidance", '/'), SlashOutcome::SetGuidance(GuidanceArg::Toggle)));
        assert!(matches!(parse("set-guidance auto", '/'), SlashOutcome::SetGuidance(GuidanceArg::Auto)));
        assert!(matches!(parse("set-guidance maybe", '/'), SlashOutcome::Error(_)));
        assert_eq!(find_command("set-guidance").expect("set-guidance").category, Category::Style);
        // SQ-1104: no arguments at all — the dialog is the question, so anything
        // typed after the name is ignored rather than being a second grammar to
        // keep in step with the two buttons.
        assert!(matches!(parse("run-font-check", '/'), SlashOutcome::RunFontCheck));
        assert_eq!(find_command("run-font-check").expect("run-font-check").category, Category::Style);
        assert_eq!(find_command("set-v6-pixel-lock").expect("set-v6-pixel-lock").category, Category::Style);
    }

    #[test]
    fn create_game_style_command_removed() {
        assert!(find_command("create-game-style").is_none(),
            "create-game-style is replaced by the Save Game Style button");
    }
}
