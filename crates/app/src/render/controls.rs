//! The pane border's clickable toggle controls (SQ-1123).
//!
//! Guidance, the command band and the two v6 render switches were reachable only
//! by slash command, key or the settings screen — nothing on screen said they
//! existed, let alone whether they were on. A player who turned guidance on and
//! saw nothing had every reason to conclude it was broken. These are the answer:
//! icons riding the story pane's own border, each one saying what state it is in
//! and switching that state when clicked.
//!
//! Four rules shape everything here — and one exception, which arrived last and
//! is stated where it applies: [`BorderControl::Reveal`] is a TRIGGER, not a
//! switch. It has no state to report, remembers nothing, and lights only while
//! the thing it started is still happening.
//!
//! Four rules shape everything here.
//!
//! **A control sits where the thing it governs is, or where it would appear.**
//! The command band opens BELOW the story pane, so its toggle rides the bottom
//! border; the map lives to the RIGHT, so its toggle takes the bottom border's
//! right-hand end, nearest the pane it summons; guidance and the word reveal have
//! no direction of their own — the reveal acts on the story pane's own prose,
//! right there — so they join the band in the centred group; and the two v6 controls
//! govern how the story pane ITSELF is drawn, so they keep that pane's own top
//! border. See [`ControlPlacement`].
//!
//! **The one place that rule was wrong was the return probe** (SQ-0785), which
//! rode the MAP pane's border because the map is what it changes. But the search
//! keeps running when the map is hidden — hiding a view must not degrade the data
//! behind it — and a pane that disappears cannot carry the only switch for
//! something that does not: you could not turn off a feature that was still going.
//! So it sits on the story pane beside the map toggle, immediately inboard of it,
//! and every control lanthorn draws is now on one border of one pane (SQ-1107).
//!
//! **A click runs the command.** Each control names an existing entry in
//! `slash::COMMANDS` and the event loop puts that command string through the
//! ordinary slash pipeline, so clicking is byte-for-byte what typing it does —
//! including whatever the command persists. There is no second implementation of
//! any toggle beside the one the registry already owns.
//!
//! **The state is carried TWICE: by the glyph and by the colour.** The map
//! toggle is an arrow pointing the way the panel would move (the map lives
//! right of the story pane), the Guiding Light is filled when lit and hollow
//! when not, and the two v6 controls draw a distinct glyph per mode — and on
//! top of that, **every control that is ON is lit yellow**, through
//! `panel.control:lit`, which is the `alert` role and so the same slot
//! `transcript_assist` lights up in. The doubling is deliberate: a player who
//! cannot tell the two colours apart still has the shape, and the shape change
//! is legible at a glance without reading the colour.
//!
//! The render mode is a three-way cycle rather than a switch, so "on" needs a
//! reading: **`hybrid` is how the game arrives and is NOT lit; `raster` and
//! `extended` both are**, because either is a choice the player made. The panel
//! cycle (SQ-1237) reads the same way: `none` is idle and not lit, and either
//! panel being open is a choice, so both `command` and `inventory` are.
//!
//! **The v6 pair does not exist off v6.** They are absent from the cluster
//! entirely rather than drawn disabled, so the border of a Zork I never shows a
//! switch that would do nothing — and since they are the only two controls on
//! the top border, a non-v6 story has no top cluster at all and its title strip
//! gets the whole row back.
//!
//! **What a click switches, it also remembers.** Every SWITCH here writes the
//! per-game `config.toml` sidecar, so a preference chosen for one story stays
//! with that story and no other. That is the commands' behaviour, not a second
//! implementation layered under the buttons: a click IS the command. The reveal
//! is exempt and says so through [`BorderControl::persists`], because a light
//! that was on for four seconds has nothing to remember — the `border_controls`
//! suite reads that method rather than a list, so the next control added without
//! a thought about persistence still fails the guard.
//!
//! **There are TWO clusters and one mechanism** (SQ-1148). The map pane carries
//! its own five — room numbers, centre, zoom out, zoom in, view — on its bottom
//! border, and they are the same [`BorderControl`] enum, the same
//! `slash::COMMANDS` dispatch, the same `panel.control{,:lit,:hover}` styles and
//! the same hit-rects as the story pane's. A second enum with its own dispatch
//! would be two places to add the next control, two hint mechanisms and two
//! chances for a hint to advertise a key nothing is bound to. What the pane
//! changes is only which controls are in the list and which pane's border they
//! are placed against: [`BorderControl::pane`] says which, so a control cannot
//! be placed by whichever list it happens to have been put in.
//!
//! **The map cluster CAN live on the map pane, where the return probe could
//! not.** The probe kept running when the map was hidden, so its only switch had
//! to survive the pane; every one of these five acts on a map that is on screen,
//! and there is nothing to switch when there is no map to switch it on.

use ratatui::layout::Rect;
use ratatui::style::Style;

use mapper::layer::MapView;

use super::panel::{draw_panel_with_controls, PanelFrame, PanelSpec};
use super::paneframe::{ControlPlacement, HeaderControl};
use crate::config::V6RenderMode;
use crate::state::{AppState, Layout};

/// One border toggle, identified by what it switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderControl {
    /// Show / hide the map pane.
    Map,
    /// Lanthorn's Guiding Light.
    Guidance,
    /// The command band.
    VerbPanel,
    /// The v6 render mode — a three-way cycle, not a toggle. v6 only.
    V6Render,
    /// The v6 pixel lock. v6 only.
    V6PixelLock,
    /// The return probe (SQ-0785).
    ReturnProbe,
    /// The momentary word reveal (SQ-1107).
    ///
    /// **The first TRIGGER in this cluster, and the only one.** Every other
    /// control here reports a state you can read off it at a glance and flips
    /// that state when clicked. This one has no state to report: it makes
    /// something happen on the story pane and is over a few seconds later. Two
    /// consequences, both deliberate — its tooltip carries more weight than its
    /// neighbours', since the glyph alone cannot say what a press does; and it
    /// still LIGHTS for the duration of the reveal, not to report a state but so
    /// that a click visibly did something, because a press that happened to light
    /// no words would otherwise be indistinguishable from a broken button.
    Reveal,

    // ── The MAP pane's cluster (SQ-1148) ─────────────────────────────────────
    // Same enum, same dispatch, same styles — a different pane, which
    // [`BorderControl::pane`] states rather than leaving to be inferred from
    // whichever list a control was put in.
    /// Room-number labels inside the map's room boxes: a boolean.
    RoomNumbers,
    /// Re-centre the map on the selected room, or the current one: a one-shot.
    Centre,
    /// Zoom the map out one step: a one-shot.
    ///
    /// Zoom is the cluster's first SCALAR and is spelled as two adjacent
    /// triggers rather than as one state report, because a single cycling
    /// control has no way back except all the way round and its glyph cannot say
    /// which level you are on. Being two triggers is also what forced
    /// [`BorderControl::command`] to widen: `zoom-map` takes an ARGUMENT, and
    /// `in` and `out` are one registry entry with two of them.
    ZoomOut,
    /// Zoom the map in one step: a one-shot. See [`BorderControl::ZoomOut`].
    ZoomIn,
    /// How the active layer draws — the drawn map or the direction matrix: a
    /// MODE, following the three-state `render` control's precedent that a mode
    /// change reads as a shape change out of one icon family.
    ///
    /// A click writes the PER-LAYER override (`mapper::layer::LayerMeta::view`,
    /// SQ-0666), so the choice sticks for that layer and rides the save. It
    /// never resolves the unruled `None` into a value it merely inherited: a
    /// click always states a view, and only a click does.
    ViewMode,
}

/// Which pane's border a control rides (SQ-1148).
///
/// [`ControlPlacement`] is an anchor on a frame and says nothing about WHICH
/// frame; with one cluster that was unambiguous, and with two it is not — a map
/// control and a story control both asking for `BottomCentre` would compete for
/// one anchor, and the failure would be a layout oddity rather than an error.
/// So the pane is a property of the control, read from the control, not implied
/// by the list it was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPane {
    /// The story pane, whose cluster arrived first (SQ-1123).
    Story,
    /// The map pane (SQ-1148).
    Map,
}

/// The `slash::COMMANDS` entry a click runs: the registry's own name for it, and
/// the argument the control supplies when the command needs one (SQ-1148).
///
/// **Both halves are needed and neither can be recovered from the other.** The
/// registry is keyed by `name` alone, so the lookup that resolves a command's
/// `Context` must have it unqualified; and `zoom-map` bare is an ERROR — the two
/// zoom controls are one entry with two arguments, so what a click actually puts
/// through the slash pipeline is the whole line. Returning one string and
/// splitting it at the call site was the alternative, and it puts the same
/// `split_whitespace().next()` in every caller with nothing to fail if one
/// forgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlCommand {
    /// The registry entry's own name — `"zoom-map"`, never `"zoom-map in"`.
    pub name: &'static str,
    /// The argument this control supplies, when it names one.
    pub arg: Option<&'static str>,
}

impl std::fmt::Display for ControlCommand {
    /// The command LINE a click runs, which is what goes through the slash
    /// pipeline and what a hint should print after its `/`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.arg {
            Some(a) => write!(f, "{} {a}", self.name),
            None => f.write_str(self.name),
        }
    }
}

impl BorderControl {
    /// Which of the pane's three border clusters this control belongs to.
    ///
    /// The whole placement rule, in one match: the two panel toggles point at
    /// panels that live below and to the right, and the two v6 switches act on
    /// the story pane itself.
    pub fn placement(self) -> ControlPlacement {
        match self {
            BorderControl::Map => ControlPlacement::BottomRight,
            // Guidance, the command band and the reveal have no direction of their
            // own — the reveal acts on the story pane's own prose, right there —
            // so they ride the bottom border together, centred.
            BorderControl::VerbPanel | BorderControl::Guidance | BorderControl::Reveal => {
                ControlPlacement::BottomCentre
            }
            BorderControl::V6Render | BorderControl::V6PixelLock => ControlPlacement::TopRight,
            // Beside the map toggle, on the STORY pane, immediately inboard of
            // it. It rode the map pane's own bottom border until SQ-1107, which
            // was the placement rule applied to the wrong half of the feature:
            // the search keeps running when the map is hidden — hiding a view
            // must not degrade the data behind it — so its only switch cannot
            // live on a pane that disappears. You could not turn off something
            // that was still running.
            BorderControl::ReturnProbe => ControlPlacement::BottomRight,
            // The map cluster is one centred group on the MAP pane's bottom
            // border (SQ-1148). Centred rather than anchored because none of the
            // five points anywhere — they all act on the pane they are drawn on,
            // which is the same reason guidance and the reveal are centred on the
            // story pane's — and one group so the five read as one cluster and
            // stand or fall together rather than half a cluster surviving a
            // narrowing.
            BorderControl::RoomNumbers
            | BorderControl::Centre
            | BorderControl::ZoomOut
            | BorderControl::ZoomIn
            | BorderControl::ViewMode => ControlPlacement::BottomCentre,
        }
    }

    /// Which PANE's border this control rides (SQ-1148).
    ///
    /// Read from the control rather than from the list it was found in, so the
    /// two clusters cannot compete for one anchor: `BottomCentre` on the story
    /// pane and `BottomCentre` on the map pane are different rows of different
    /// frames, and only this says which.
    pub fn pane(self) -> ControlPane {
        match self {
            BorderControl::Map
            | BorderControl::Guidance
            | BorderControl::VerbPanel
            | BorderControl::V6Render
            | BorderControl::V6PixelLock
            | BorderControl::ReturnProbe
            | BorderControl::Reveal => ControlPane::Story,
            BorderControl::RoomNumbers
            | BorderControl::Centre
            | BorderControl::ZoomOut
            | BorderControl::ZoomIn
            | BorderControl::ViewMode => ControlPane::Map,
        }
    }

    /// The `slash::COMMANDS` entry a click runs — which toggles or cycles for
    /// every switch here, and simply HAPPENS for the triggers.
    ///
    /// **Not a bare string** (SQ-1148). It was one while every control's command
    /// took no argument; `zoom-map` takes `in|out|reset|<n>` and errors without
    /// one, so the two zoom controls are a single registry entry with two
    /// different arguments. See [`ControlCommand`] for why the name and the
    /// argument stay apart rather than being one line the callers re-split.
    pub fn command(self) -> ControlCommand {
        let bare = |name| ControlCommand { name, arg: None };
        match self {
            BorderControl::Map => bare("toggle-map"),
            BorderControl::Guidance => bare("set-guidance"),
            // SQ-1237: this control now cycles command panel → inventory panel →
            // none rather than merely toggling the command panel — `cycle-panel`
            // is the registry entry that does both, so a click is still exactly
            // what typing it does.
            BorderControl::VerbPanel => bare("cycle-panel"),
            BorderControl::V6Render => bare("set-v6-render"),
            BorderControl::V6PixelLock => bare("set-v6-pixel-lock"),
            BorderControl::ReturnProbe => bare("set-return-probe"),
            BorderControl::Reveal => bare("reveal-words"),
            BorderControl::RoomNumbers => bare("toggle-room-numbers"),
            BorderControl::Centre => bare("center-map"),
            BorderControl::ZoomOut => ControlCommand { name: "zoom-map", arg: Some("out") },
            BorderControl::ZoomIn => ControlCommand { name: "zoom-map", arg: Some("in") },
            // Bare, which CYCLES between the drawn map and the matrix — the same
            // reading a bare `/view-map` gives, and the one that keeps a click
            // identical to typing it.
            BorderControl::ViewMode => bare("view-map"),
        }
    }

    /// Does this control REMEMBER what it switched, in the per-game sidecar?
    ///
    /// True of every switch here and false of the one trigger, which has nothing
    /// to remember. Stated rather than inferred, because the property it exists
    /// to guard is exactly "a control whose command does not persist" — see the
    /// `border_controls` suite, which walks this and asserts the registry's own
    /// description matches.
    pub fn persists(self) -> bool {
        !matches!(
            self,
            BorderControl::Reveal
                // None of the map cluster writes the per-game sidecar, and each
                // for its own reason rather than as a group (SQ-1148). Centre and
                // the two zooms are one-shots: a viewport is not a preference.
                // Room numbers is a session flag seeded from the global
                // `show_room_numbers` at startup, which the settings screen owns.
                // The view switch DOES remember — in the map archive, per layer
                // (`LayerMeta::view`), which is a different store with a
                // different lifetime, so answering `true` here would point the
                // guard below at a sidecar that will never hold it.
                | BorderControl::RoomNumbers
                | BorderControl::Centre
                | BorderControl::ZoomOut
                | BorderControl::ZoomIn
                | BorderControl::ViewMode
        )
    }
}

/// One control resolved against the live state: which toggle it is, the glyph
/// for the state it is in, the style that state resolves to, and the hover hint.
pub struct ControlView {
    pub id: BorderControl,
    pub glyph: char,
    pub style: Style,
    /// The floating hint's lines: what this is and what a click would do, then
    /// the command or key that does the same thing from the keyboard.
    pub hint: Vec<String>,
}

impl ControlView {
    /// The paint-only half, for `panel::draw_panel_with_controls`.
    pub fn as_header_control(&self) -> HeaderControl {
        HeaderControl { glyph: self.glyph, style: self.style, placement: self.id.placement() }
    }
}

/// The keyboard route to `command`, spelled the way a hint should spell it: the
/// key that runs it when one is bound, then the slash command that always does.
///
/// **Read from the live keymap and leader panel, never written out by hand**
/// (SQ-1142). Two hints here used to name an F-key as a literal — `"F2"` for the
/// command band and `"F4 · /reveal-words"` for the reveal — and when SQ-1142
/// unbound those defaults the hints went on advertising keys that did nothing.
/// A hint that ASKS cannot say that; it also follows a player who rebound the
/// key rather than reciting a default at them, and it picks up the leader route
/// for a command that has one and no direct key, which is exactly what the
/// command band became. `no_hint_advertises_a_key_that_is_not_bound` in the
/// `border_controls` suite fails the hand-written form, because a substring
/// check on the command half is satisfied by a lie about the key half.
fn key_route(state: &AppState, cmd: ControlCommand) -> String {
    // An ARGUMENT-bearing control must match the whole line (SQ-1148): `+` is
    // bound to `zoom-map in` and `-` to `zoom-map out`, and a lookup by name
    // alone answers `+` for both — which is precisely the hint that names a key
    // that does not reach the control, the defect SQ-1142 fixed.
    if let Some(k) = match cmd.arg {
        Some(_) => state.keymap.primary_key_exact(&cmd.to_string()),
        None => state.keymap.primary_key(cmd.name),
    } {
        return format!("{} · /{cmd}", k.label());
    }
    if let Some(letter) = leader_letter(state, cmd) {
        return format!("{} {letter} · /{cmd}", state.hotkeys.prefix.label());
    }
    format!("/{cmd}")
}

/// The leader-panel letter that reaches `command`, if the panel offers one.
///
/// Matched on the command NAME for a control that supplies no argument, so a
/// panel row that carries one (`"zoom-map in"`) still answers for `zoom-map` —
/// and on the WHOLE line for a control that does, so the zoom-out control cannot
/// be handed the zoom-in row's letter (SQ-1148).
fn leader_letter(state: &AppState, cmd: ControlCommand) -> Option<char> {
    let line = cmd.to_string();
    state
        .hotkeys
        .groups
        .iter()
        .flat_map(|(_, entries)| entries.iter())
        .find(|(_, entry, _)| match cmd.arg {
            Some(_) => *entry == line,
            None => entry.split_whitespace().next() == Some(cmd.name),
        })
        .map(|(letter, _, _)| *letter)
}

/// Resolve the theme selector for a control's state: lit when it is on, quiet
/// when it is not, and `hover` over everything — so whatever the pointer is on
/// always reads as reachable, on or off.
///
/// Two selectors, not three. There was a `panel.control:active` beside `:lit`
/// while "on" and "lit" were different states; every on-state is lit now, so
/// nothing could ever resolve to it, and a selector a themer can set and never
/// see is worse than one that does not exist.
fn style_for(state: &AppState, id: BorderControl, lit: bool) -> Style {
    let sel = if state.control_hover == Some(id) {
        "panel.control:hover"
    } else if lit {
        "panel.control:lit"
    } else {
        "panel.control"
    };
    state.colors.theme.get(sel).style
}

/// The controls to draw in the STORY pane's border, left to right. The map
/// pane's own five are [`map_controls_for`].
///
/// Always the five that apply to every story; the two v6 ones only when the
/// story really is v6 (header version 6, as `startup` recorded it), so they
/// appear and vanish with the game rather than being greyed out.
///
/// **Order is placement, within a group.** The groups are filtered out of this
/// one list in index order, so the probe standing ahead of the map toggle is what
/// puts it inboard — and what makes it the one that goes first when the pane
/// narrows, since an anchored group sheds from its left.
pub fn controls_for(state: &AppState) -> Vec<ControlView> {
    let g = &state.symbols.controls;
    let mut out = Vec::with_capacity(7);

    // ── Return probe ─────────────────────────────────────────────────────────
    // First, so it takes the INBOARD slot of the right-hand pair and the map
    // toggle keeps the corner. Within that pair the probe gives way first as the
    // pane narrows: the map toggle moves a whole pane and is the only way back to
    // a hidden map, so it survives longest (SQ-1107).
    //
    // **Drawn in both states, never hidden.** Every other switch here governs
    // something already on by default or already visible, so it is discovered by
    // being used. This one is off out of the box, and a switch nobody has ever
    // seen lit is a switch nobody finds: muted through the plain `panel.control`
    // when off, lit yellow when on, same glyph either way (see
    // [`crate::symbols::ControlGlyphs::return_probe`]).
    let probe_on = state.config.return_probe;
    out.push(ControlView {
        id: BorderControl::ReturnProbe,
        glyph: g.return_probe,
        style: style_for(state, BorderControl::ReturnProbe, probe_on),
        hint: vec![
            if probe_on {
                "Return probe: on — click to stop looking for the way back"
            } else {
                "Return probe: off — click to look for the way back after a move"
            }
            .to_string(),
            "/set-return-probe".to_string(),
        ],
    });

    // ── Map ──────────────────────────────────────────────────────────────────
    let map_on = state.layout == Layout::Split;
    out.push(ControlView {
        id: BorderControl::Map,
        glyph: if map_on { g.map_hide } else { g.map_show },
        style: style_for(state, BorderControl::Map, map_on),
        hint: vec![
            if map_on { "Map: shown — click to hide" } else { "Map: hidden — click to show" }
                .to_string(),
            "/toggle-map".to_string(),
        ],
    });

    // ── Guidance ─────────────────────────────────────────────────────────────
    let guide_on = state.config.guidance;
    out.push(ControlView {
        id: BorderControl::Guidance,
        glyph: if guide_on { g.guidance_on } else { g.guidance_off },
        style: style_for(state, BorderControl::Guidance, guide_on),
        hint: vec![
            if guide_on {
                "Guiding Light: on — click to put it out"
            } else {
                "Guiding Light: off — click to light it"
            }
            .to_string(),
            "/set-guidance".to_string(),
        ],
    });

    // ── The panel cycle: command panel → inventory panel → none (SQ-1237) ────
    // Three states, one control, so the glyph and the hint both name the state
    // it is IN (not the state a click reaches, as the two-way toggles above do)
    // and the hint's second line says what a click does next. `None` is the
    // only unlit reading — the other two are a panel actually open, which is
    // exactly what "lit" means everywhere else in this cluster.
    let panel = state.current_side_panel();
    let (panel_glyph, panel_hint) = match panel {
        crate::state::SidePanel::Command => {
            (g.band_hide, "Command panel: open — click for the inventory panel")
        }
        crate::state::SidePanel::Inventory => {
            (g.inventory_open, "Inventory panel: open — click to close")
        }
        crate::state::SidePanel::None => {
            (g.band_show, "Closed — click for the command panel")
        }
    };
    out.push(ControlView {
        id: BorderControl::VerbPanel,
        glyph: panel_glyph,
        style: style_for(state, BorderControl::VerbPanel, panel != crate::state::SidePanel::None),
        hint: vec![panel_hint.to_string(), key_route(state, BorderControl::VerbPanel.command())],
    });

    // ── The reveal (a trigger, not a switch) ─────────────────────────────────
    // Lit while a reveal is up — which is not a state report, since there is no
    // state: it is the click's own acknowledgement, so a press that lights no
    // words still reads as a press that worked.
    //
    // Its hint does more work than the others'. Theirs need only name a state and
    // its opposite, because the glyph has already said which one is in force; a
    // lamp on a border says nothing at all about WHAT it lights, so this one has
    // to say it — and, when the Guiding Light is out, has to say why a press will
    // do nothing rather than leaving the player to conclude it is broken.
    let lit = state.reveal.as_ref().is_some_and(|r| r.is_lit());
    out.push(ControlView {
        id: BorderControl::Reveal,
        glyph: g.reveal,
        style: style_for(state, BorderControl::Reveal, lit),
        hint: vec![
            "Reveal: light the nouns and named things on screen the story knows".to_string(),
            if state.config.guidance {
                "click for a moment — it goes out on your next key"
            } else {
                "needs the Guiding Light, which is out — the lamp beside this one"
            }
            .to_string(),
            // Whatever actually reaches it, unlike its neighbours: the other
            // controls' toggles can be found by clicking the control that names
            // them, and this one cannot be found at all without being told the
            // route. It had a direct key until SQ-1142 took it away, so the
            // route is READ rather than written — see `key_route`.
            key_route(state, BorderControl::Reveal.command()),
        ],
    });

    if state.story_zversion != Some(6) {
        return out;
    }

    // ── v6 render mode (a cycle, so the hint names what is next) ─────────────
    let (mode_glyph, mode_name, next) = match state.config.v6_render {
        V6RenderMode::Hybrid => (g.render_hybrid, "hybrid", "raster"),
        V6RenderMode::Raster => (g.render_raster, "raster", "extended"),
        V6RenderMode::Extended => (g.render_extended, "extended", "hybrid"),
    };
    out.push(ControlView {
        id: BorderControl::V6Render,
        glyph: mode_glyph,
        // `hybrid` is how the game arrives, so it reads as the idle state and the
        // other two as a choice the player made — which is what "on" means for a
        // cycle, and so what is lit.
        style: style_for(
            state,
            BorderControl::V6Render,
            state.config.v6_render != V6RenderMode::Hybrid,
        ),
        hint: vec![
            format!("Render: {mode_name} — click for {next}"),
            "/set-v6-render".to_string(),
        ],
    });

    // ── v6 pixel lock ────────────────────────────────────────────────────────
    let lock_on = state.config.v6_pixel_lock;
    out.push(ControlView {
        id: BorderControl::V6PixelLock,
        glyph: if lock_on { g.lock_on } else { g.lock_off },
        style: style_for(state, BorderControl::V6PixelLock, lock_on),
        hint: vec![
            if lock_on {
                "Pixel lock: on — click to unlock"
            } else {
                "Pixel lock: off — click to lock"
            }
            .to_string(),
            "/set-v6-pixel-lock".to_string(),
        ],
    });

    out
}

/// The controls to draw in the MAP pane's border, left to right: room numbers,
/// centre, zoom out, zoom in, view (SQ-1148).
///
/// `view` is the view the ACTIVE LAYER actually draws in
/// (`MapGraph::layer_view`, i.e. `LayerMeta::effective_view`), which is the one
/// fact this cluster needs and the one `AppState` cannot answer on its own — the
/// map's per-layer state lives on the graph. Passed in rather than looked up so
/// this stays a pure function of what is on screen, exactly as `controls_for` is.
///
/// **`None` versus `Some(the derived value)` is not this function's business.**
/// It draws what the layer resolves to; a click runs `/view-map`, which is the
/// only thing that ever writes the override, and it always writes `Some(_)`. So
/// the unruled/ruled distinction cannot be flattened by looking at the map.
pub fn map_controls_for(state: &AppState, view: MapView) -> Vec<ControlView> {
    let g = &state.symbols.map_controls;
    let mut out = Vec::with_capacity(5);

    // ── Room numbers ─────────────────────────────────────────────────────────
    // The one control in either cluster whose PLAIN glyph is the same in both
    // states (`#`), leaving colour to say which — a degradation forced by ASCII
    // having no off-shape for a `#`, not a new house pattern. The patched preset
    // obeys the shape rule outright. See [`crate::symbols::MapControlGlyphs`].
    let numbers_on = state.show_room_numbers;
    out.push(ControlView {
        id: BorderControl::RoomNumbers,
        glyph: if numbers_on { g.room_numbers_on } else { g.room_numbers_off },
        style: style_for(state, BorderControl::RoomNumbers, numbers_on),
        hint: vec![
            if numbers_on {
                "Room numbers: shown — click to hide"
            } else {
                "Room numbers: hidden — click to show"
            }
            .to_string(),
            key_route(state, BorderControl::RoomNumbers.command()),
        ],
    });

    // ── Centre (a one-shot) ──────────────────────────────────────────────────
    // No state to report, so never lit and its hint says what a press DOES —
    // the reveal's rule, for the same reason.
    out.push(ControlView {
        id: BorderControl::Centre,
        glyph: g.centre,
        style: style_for(state, BorderControl::Centre, false),
        hint: vec![
            "Centre the map on the selected room, or the current one".to_string(),
            key_route(state, BorderControl::Centre.command()),
        ],
    });

    // ── Zoom, as two one-shots ───────────────────────────────────────────────
    // The set's first SCALAR, and it reports no level: a single cycling control
    // has no way back except all the way round, and no glyph can say which rung
    // you are on. Two adjacent triggers are directly manipulable and reversible.
    out.push(ControlView {
        id: BorderControl::ZoomOut,
        glyph: g.zoom_out,
        style: style_for(state, BorderControl::ZoomOut, false),
        hint: vec![
            "Zoom out — more of the map, drawn smaller".to_string(),
            key_route(state, BorderControl::ZoomOut.command()),
        ],
    });
    out.push(ControlView {
        id: BorderControl::ZoomIn,
        glyph: g.zoom_in,
        style: style_for(state, BorderControl::ZoomIn, false),
        hint: vec![
            "Zoom in — less of the map, drawn larger".to_string(),
            key_route(state, BorderControl::ZoomIn.command()),
        ],
    });

    // ── View (a mode) ────────────────────────────────────────────────────────
    // Lit on the matrix, which is the view a player chooses; the drawn map is
    // how a layer arrives, so it reads as the idle state — the same reading the
    // v6 render cycle's `hybrid` gets.
    let matrix = view == MapView::Matrix;
    out.push(ControlView {
        id: BorderControl::ViewMode,
        glyph: if matrix { g.view_matrix } else { g.view_drawn },
        style: style_for(state, BorderControl::ViewMode, matrix),
        hint: vec![
            if matrix {
                "View: matrix — click for the drawn map"
            } else {
                "View: drawn — click for the direction matrix"
            }
            .to_string(),
            key_route(state, BorderControl::ViewMode.command()),
        ],
    });

    out
}

/// Draw a pane with its control cluster, and hand back the frame plus the
/// hit-rects that are actually ON SCREEN (SQ-1148).
///
/// **The one seam both clusters go through.** It was inlined in `main` while
/// there was one cluster; with two, a pane that resolved its own would be a
/// second place for the zero-area filter to be forgotten, and — the reason it
/// is here rather than there — a cluster drawn only from `main` is a cluster no
/// test can render, which is exactly how a quest shipped this feature's whole
/// vocabulary and none of its controls.
///
/// A group the pane was too narrow to hold leaves zero-area rects behind; they
/// are dropped here, so what comes back is only what a pointer can reach.
pub fn draw_pane_with_controls(
    buf: &mut ratatui::buffer::Buffer,
    spec: &PanelSpec,
    theme: &crate::theme::resolve::Theme,
    views: &[ControlView],
) -> (PanelFrame, Vec<(BorderControl, Rect)>) {
    let ctls: Vec<_> = views.iter().map(|v| v.as_header_control()).collect();
    let (frame, rects) = draw_panel_with_controls(buf, spec, &ctls, theme);
    let hits = views
        .iter()
        .map(|v| v.id)
        .zip(rects)
        .filter(|(_, r)| r.width > 0 && r.height > 0)
        .collect();
    (frame, hits)
}

/// Draw the hover hint for whichever control the pointer is on, if any.
///
/// `hits` are this frame's control rects (the same ones a click resolves
/// against, so hint and click can never disagree about what is under the
/// pointer); `state.control_hover` is what the last `Moved` event resolved.
///
/// The box goes INTO the pane, whichever border its control rides: down from the
/// top one, up from the bottom one. It never covers the icon being pointed at,
/// and `tooltip::draw_tip_on` slides it left of the right edge and flips it to
/// the other side of the anchor rather than letting it run off. It paints and
/// nothing else: no focus, no keyboard, no event.
pub fn draw_control_hint(
    buf: &mut ratatui::buffer::Buffer,
    area: Rect,
    state: &AppState,
    views: &[ControlView],
    hits: &[(BorderControl, Rect)],
) -> Option<Rect> {
    // A modal owning the screen also owns the pointer: the hint is drawn after
    // the overlay ladder, so without this it would float on top of a dialog if
    // one opened while the pointer sat on a control and never moved after.
    if state.any_modal_overlay_open() {
        return None;
    }
    let id = state.control_hover?;
    let (_, rect) = hits.iter().find(|(i, _)| *i == id)?;
    // A group the pane was too narrow to draw leaves a zero-area rect; there is
    // no icon on screen to explain, so there is no hint either.
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let view = views.iter().find(|v| v.id == id)?;
    let side = match id.placement() {
        ControlPlacement::TopRight => super::tooltip::TipSide::Below,
        ControlPlacement::BottomCentre | ControlPlacement::BottomRight => {
            super::tooltip::TipSide::Above
        }
    };
    super::tooltip::draw_tip_on(
        buf,
        area,
        rect.x,
        rect.y,
        &view.hint,
        &state.colors.theme,
        &state.symbols,
        side,
    )
}

/// Resolve a pointer position against this frame's control rects.
///
/// One function for both the click path and the `Moved` hover path, so the two
/// always agree; returns `None` over anything else, including while a modal
/// overlay owns the screen.
pub fn control_at(
    state: &AppState,
    hits: &[(BorderControl, Rect)],
    col: u16,
    row: u16,
) -> Option<BorderControl> {
    if state.any_modal_overlay_open() {
        return None;
    }
    hits.iter()
        .find(|(_, r)| {
            r.width > 0 && r.height > 0 && col >= r.x && col < r.right() && row >= r.y
                && row < r.bottom()
        })
        .map(|(id, _)| *id)
}
