//! Z-machine debug inspector (tiled pane) — panel state + navigation logic.
//! Pure over the `Debugger` trait (engine-neutral); the render code paints the
//! snapshot this holds. No `zvm::` calls here.
//!
//! Model: three tabbed **windows** in a fixed screen layout (left full height;
//! right split top/bottom). `Tab`/`Shift-Tab` cycle which window is focused;
//! `Left`/`Right` switch the focused window's active tab. The disassembly
//! re-anchors to the live PC on every per-turn `refresh` ("PC-follow").

use ratatui::layout::Rect;

use crate::engine::{Debugger, DisasmProvenance};
use crossterm::event::KeyCode;

/// How many instructions / memory rows to pre-render for the address-windowed
/// sections (draw clips to the pane height; over-computing avoids threading
/// height into refresh).
pub const DISASM_WINDOW: usize = 256;
pub const MEM_WINDOW: usize = 256;

/// Disassembly view mode, cycled by `r` in the Disasm tab: **Full** (operand-role
/// sigils, `[obj#N]`/`[word]` annotations, `VarRef`, packed unpacking, variable
/// naming, branch targets), **Basic** (plain mnemonic disassembly — no reference-
/// following), **Raw** (bytes + untranslated decode, no lookups).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DisasmMode { Full, Basic, Raw }

/// A displayable section (one tab's content).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section { Disasm, Globals, Locals, Objects, Dict, CallStack, EvalStack, Memory }

impl Section {
    /// Short label shown on its tab, and used as the on-screen hit target.
    pub fn label(self) -> &'static str {
        match self {
            Section::Disasm => "Disassembly",
            Section::Globals => "Globals",
            Section::Locals => "Locals",
            Section::Objects => "Objects",
            Section::Dict => "Dictionary",
            Section::CallStack => "Call Stack",
            Section::EvalStack => "Stack",
            Section::Memory => "Memory",
        }
    }
}

/// Which tabs each window offers, in order. Window 0 = left (full height),
/// 1 = right-top, 2 = right-bottom. The first tab in each window is the one it
/// opens on (see [`DEFAULT_SECTIONS`]).
pub const WINDOW_TABS: [&[Section]; 3] = [
    &[Section::Disasm],
    &[Section::Globals, Section::Locals, Section::Objects, Section::Dict],
    &[Section::CallStack, Section::EvalStack, Section::Memory],
];

/// The section each window shows by default. Kept as sections (not indices) so
/// reordering [`WINDOW_TABS`] can never silently change which tab opens.
pub const DEFAULT_SECTIONS: [Section; 3] =
    [Section::Disasm, Section::Globals, Section::CallStack];

/// The `(window, tab)` position that renders `section`. Every [`Section`] lives
/// in exactly one window (guarded by `window_tabs_cover_every_section`), so this
/// is total — callers navigate by section and never hard-code tab indices.
pub fn locate_section(section: Section) -> (usize, usize) {
    for (w, tabs) in WINDOW_TABS.iter().enumerate() {
        if let Some(t) = tabs.iter().position(|&s| s == section) {
            return (w, t);
        }
    }
    (0, 0)
}

/// The formatted lines the render code paints, refreshed from the Debugger.
#[derive(Debug, Default, Clone)]
pub struct DebugSnapshot {
    pub disasm: Vec<String>,
    /// Static confidence tier per `disasm` line (SQ-0428), aligned 1:1. The
    /// render layer combines this with `executed` to pick each line's colour
    /// tier. A short/empty vec falls back to the `Rd` (plain) tier per line.
    pub disasm_prov: Vec<DisasmProvenance>,
    pub globals: Vec<String>,
    pub locals: Vec<String>,
    pub objects: Vec<String>,
    pub dict: Vec<String>,
    pub stack: Vec<String>,
    pub eval_stack: Vec<String>,
    pub memory: Vec<String>,
    /// The story's own decoded text for each `memory` row, index-aligned with
    /// it. `None` (or an index past the end) means no string the engine can
    /// vouch for covers that row, and the hex row's raw character column is all
    /// there is to show — never a guess (SQ-0969).
    pub memory_zstrings: Vec<Option<String>>,
    /// Column at which [`memory_zstrings`](Self::memory_zstrings) is drawn: two
    /// past the widest `memory` row in the window. Derived when the window is
    /// loaded rather than pinned to a constant, so an engine whose `memory_hex`
    /// formats rows to a different width needs no agreement with the renderer.
    pub memory_zcol: usize,
    /// Width of the widest Memory row including its decoded-text column — the
    /// bound the horizontal scroll clamps against (SQ-0965).
    pub memory_width: usize,
    /// Instruction start-PCs executed during the last command turn (execution-
    /// coverage marking — a `|` gutter is drawn beside these disasm lines).
    pub executed: std::collections::HashSet<u32>,
    /// Cumulative instruction start-PCs ever executed (plus seeded prior coverage);
    /// never cleared per turn. Drives the permanent "executed" (blue) colour,
    /// decoupled from the last-turn `|` gutter. (SQ-0449)
    pub executed_ever: std::collections::HashSet<u32>,
    /// Detail lines (attributes + properties) for each currently-expanded
    /// object, keyed by object number. Refreshed each turn for whichever
    /// objects are still in `DebugPanelState::expanded_objects`.
    pub object_details: std::collections::HashMap<u16, Vec<String>>,
    /// Detail lines (locals) for each currently-expanded call-stack frame, keyed
    /// by frame index. Unlike `object_details`, this is cleared every turn —
    /// frame indices are ephemeral, so expansion state resets on each `refresh`.
    pub frame_details: std::collections::HashMap<usize, Vec<String>>,
}

impl DebugSnapshot {
    /// The lines for one section, regardless of which window shows it.
    pub fn section(&self, s: Section) -> &[String] {
        match s {
            Section::Disasm => &self.disasm,
            Section::Globals => &self.globals,
            Section::Locals => &self.locals,
            Section::Objects => &self.objects,
            Section::Dict => &self.dict,
            Section::CallStack => &self.stack,
            Section::EvalStack => &self.eval_stack,
            Section::Memory => &self.memory,
        }
    }
}

/// A floating value tooltip: the screen anchor (the hovered token's start col
/// and its row) plus the formatted lines to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverTip { pub col: u16, pub row: u16, pub lines: Vec<String> }

impl HoverTip {
    /// Build the tooltip for variable `var` with current value `value`
    /// (`None` = unavailable, e.g. a local with no frame). `col`/`row` anchor it.
    pub fn for_var(var: u8, value: Option<u16>, col: u16, row: u16) -> Self {
        let label = match var {
            0 => "sp".to_string(),
            1..=15 => format!("local{}", var - 1),
            n => format!("g{:02x}", n - 16),
        };
        let lines = match value {
            Some(v) => vec![format!("{label} = 0x{v:04x}"), format!("{v} / {}", v as i16)],
            None => vec![format!("{label} = (n/a)")],
        };
        HoverTip { col, row, lines }
    }

    /// Build a tooltip from ready-made `lines` anchored at `col`/`row`
    /// (e.g. opcode help from the debugger).
    pub fn for_lines(lines: Vec<String>, col: u16, row: u16) -> Self {
        HoverTip { col, row, lines }
    }
}

#[derive(Debug, Clone)]
pub struct DebugPanelState {
    /// Focused window: 0 = left, 1 = right-top, 2 = right-bottom.
    pub focus: usize,
    /// The visible tabs per window, in order. Defaults to [`WINDOW_TABS`]; an
    /// engine can replace it via [`apply_engine_layout`](Self::apply_engine_layout)
    /// to hide inapplicable sections and reuse slots (Scott). Every navigation /
    /// render site reads this, never the const, so a custom layout is honoured.
    pub tabs: [Vec<Section>; 3],
    /// Per-section tab-label overrides (empty = each section's own `label()`),
    /// set alongside `tabs` for engines that relabel a reused slot.
    labels: std::collections::HashMap<Section, &'static str>,
    /// Active tab index per window (into `tabs[window]`).
    pub tab: [usize; 3],
    /// List-content scroll offset per window (reset on tab change).
    pub scroll: [usize; 3],
    pub disasm_addr: u32,
    /// Disassembly view mode (Full / Basic / Raw), cycled by `r` in the Disasm tab.
    pub disasm_mode: DisasmMode,
    pub mem_addr: u32,
    /// Memory address-input edit buffer (hex digits typed so far). `None`
    /// when not editing; `Some` while the Memory tab's `:`/`/`-opened input
    /// line is active.
    pub mem_input: Option<String>,
    /// Horizontal scroll offset, in columns, for the Memory dump (SQ-0965). A
    /// hex row is 72 columns before its decoded-text column even starts, which
    /// no debug window is ever wide enough for. Clamped in `reload_memory`
    /// against the loaded window's widest row, so it can never sit past the
    /// content; the exact right edge would need the pane width, which this
    /// model does not have (and `less -S` scrolls the same way).
    pub mem_hscroll: usize,
    /// Object numbers currently expanded inline in the Objects tree.
    pub expanded_objects: std::collections::HashSet<u16>,
    /// Frame indices currently expanded inline in the Call Stack. Reset each
    /// turn (frame indices are ephemeral) — see `refresh`.
    pub expanded_frames: std::collections::HashSet<usize>,
    /// Focused-window content height captured by the last draw (for paging).
    pub viewport: usize,
    /// Live PC (for disasm PC-follow + highlight).
    pub pc: u32,
    pub snapshot: DebugSnapshot,
    /// Floating value tooltip for the variable operand under the mouse (set by
    /// the `Moved` handler, cleared on move-off and on each per-turn `refresh`).
    pub hover: Option<HoverTip>,
    /// Active mouse text selection: `(window, range)` in that window's
    /// content-relative coordinates (row/col inside the frame, border excluded).
    /// Cleared whenever the window's content moves (scroll / tab / turn). (SQ-0420)
    pub sel: Option<(usize, crate::clipboard::Selection)>,
    /// Copy text for `sel`, published by the renderer (which reads the drawn
    /// cells) and consumed on mouse-release to emit OSC 52 — mirrors the story
    /// pane's `selection_text`. (SQ-0420)
    pub selection_text: std::cell::RefCell<Option<String>>,
}

/// Result of a keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugKey { Consumed, Ignored, Close }

impl DebugPanelState {
    pub fn new(pc: u32) -> Self {
        // Each window opens on its DEFAULT_SECTIONS entry, resolved to a tab index.
        let tab = [
            locate_section(DEFAULT_SECTIONS[0]).1,
            locate_section(DEFAULT_SECTIONS[1]).1,
            locate_section(DEFAULT_SECTIONS[2]).1,
        ];
        DebugPanelState {
            focus: 0,
            tabs: [WINDOW_TABS[0].to_vec(), WINDOW_TABS[1].to_vec(), WINDOW_TABS[2].to_vec()],
            labels: std::collections::HashMap::new(),
            tab,
            scroll: [0, 0, 0],
            disasm_addr: pc,
            disasm_mode: DisasmMode::Full,
            mem_addr: 0,
            mem_input: None,
            mem_hscroll: 0,
            expanded_objects: std::collections::HashSet::new(),
            expanded_frames: std::collections::HashSet::new(),
            viewport: 1,
            pc,
            snapshot: DebugSnapshot::default(),
            hover: None,
            sel: None,
            selection_text: std::cell::RefCell::new(None),
        }
    }

    /// The section the given window is currently showing (clamped, so an engine
    /// layout with fewer tabs than a stale `tab` index can never index-panic).
    pub fn active_section(&self, window: usize) -> Section {
        let tabs = &self.tabs[window];
        tabs[self.tab[window].min(tabs.len().saturating_sub(1))]
    }

    /// The label to draw on `section`'s tab (an engine override, else its own
    /// `label()`).
    pub fn tab_label(&self, section: Section) -> &'static str {
        self.labels.get(&section).copied().unwrap_or_else(|| section.label())
    }

    /// Locate `section` within the LIVE `tabs` layout (not the const) so nav
    /// works under a custom engine layout; `(0, 0)` if it isn't visible.
    fn locate(&self, section: Section) -> (usize, usize) {
        for (w, tabs) in self.tabs.iter().enumerate() {
            if let Some(t) = tabs.iter().position(|&s| s == section) {
                return (w, t);
            }
        }
        (0, 0)
    }

    /// Adopt `dbg`'s inspector layout: replace `tabs`/`labels` and reset the
    /// per-window tab/scroll/focus when the engine supplies a custom layout
    /// (Scott). A no-op for engines that don't (the Z-machine), so its panel is
    /// left byte-for-byte identical. Call once, right after `new`, at each open
    /// site (the `/debug` toggle and the `--debug` auto-open).
    pub fn apply_engine_layout(&mut self, dbg: &dyn Debugger) {
        let Some(layout) = dbg.sections() else { return };
        self.tabs = [layout[0].to_vec(), layout[1].to_vec(), layout[2].to_vec()];
        self.tab = [0, 0, 0];
        self.scroll = [0, 0, 0];
        self.focus = 0;
        self.labels = layout
            .iter()
            .flat_map(|w| w.iter())
            .map(|&s| (s, dbg.section_label(s)))
            .collect();
    }

    /// Recompute the whole snapshot for the current cursor positions.
    /// **PC-follow:** re-anchors the disassembly to the live PC, so the
    /// executing instruction is always at the top of the Disassembly tab
    /// after a turn.
    pub fn refresh(&mut self, dbg: &dyn Debugger) {
        // A new turn re-anchors the disassembly, so a stale tooltip (anchored to
        // last turn's rows) must not linger. A selection anchored to those rows
        // is stale for the same reason (SQ-0420).
        self.hover = None;
        self.sel = None;
        self.pc = dbg.pc();
        self.disasm_addr = self.pc;
        self.reload_disasm(dbg);
        self.snapshot.globals = dbg.globals_lines();
        self.snapshot.locals = dbg.locals_lines();
        self.snapshot.objects = dbg.object_tree_lines();
        self.snapshot.dict = dbg.dictionary_lines();
        self.snapshot.stack = dbg.stack_lines();
        // Frame indices are ephemeral across turns, so expansion state cannot
        // carry over the way objects' does — clear it every refresh.
        self.expanded_frames.clear();
        self.snapshot.frame_details = std::collections::HashMap::new();
        self.snapshot.eval_stack = dbg.eval_stack_lines();
        self.reload_memory(dbg);
        self.snapshot.executed = dbg.executed_pcs();
        self.snapshot.executed_ever = dbg.ever_executed_pcs();
        self.snapshot.object_details = self.expanded_objects.iter()
            .map(|&o| (o, dbg.object_detail(o)))
            .collect();
    }

    fn page(&self) -> usize { self.viewport.max(1) }

    /// Build the disassembly for `addr` honoring the current view mode, so every
    /// site that (re)builds `snapshot.disasm` picks up the raw/translated toggle.
    /// Returns the display text plus the per-line confidence tier (SQ-0428).
    /// Provenance is display-format-independent (the tiered accessor's lines
    /// match the basic/raw lines one-for-one), so basic/raw pair their own text
    /// with the tiered provenance.
    fn load_disasm(&self, dbg: &dyn Debugger, addr: u32) -> (Vec<String>, Vec<DisasmProvenance>) {
        match self.disasm_mode {
            DisasmMode::Full => {
                let tiered = dbg.disassemble_tiered(addr, DISASM_WINDOW);
                let prov = tiered.iter().map(|(_, p)| *p).collect();
                let text = tiered.into_iter().map(|(s, _)| s).collect();
                (text, prov)
            }
            DisasmMode::Basic => {
                let text = dbg.disassemble_basic(addr, DISASM_WINDOW);
                let prov = dbg.disassemble_tiered(addr, DISASM_WINDOW).into_iter().map(|(_, p)| p).collect();
                (text, prov)
            }
            DisasmMode::Raw => {
                let text = dbg.disassemble_raw(addr, DISASM_WINDOW);
                let prov = dbg.disassemble_tiered(addr, DISASM_WINDOW).into_iter().map(|(_, p)| p).collect();
                (text, prov)
            }
        }
    }

    /// Re-anchor `snapshot.disasm` (and its provenance) to `self.disasm_addr`
    /// under the current view mode. One helper so every nav/refresh site keeps
    /// the text and tier vectors in lockstep.
    fn reload_disasm(&mut self, dbg: &dyn Debugger) {
        let (text, prov) = self.load_disasm(dbg, self.disasm_addr);
        self.snapshot.disasm = text;
        self.snapshot.disasm_prov = prov;
    }

    /// Load the Memory window at `mem_addr`: the hex rows, the decoded Z-text
    /// beside them, and the two widths the renderer and the horizontal scroll
    /// both read. One helper because the four things must stay in lockstep —
    /// a hex row and a decode from different windows would caption the wrong
    /// bytes, which is the exact failure this column exists to avoid.
    fn reload_memory(&mut self, dbg: &dyn Debugger) {
        self.snapshot.memory = dbg.memory_hex(self.mem_addr, MEM_WINDOW);
        self.snapshot.memory_zstrings = dbg.memory_zstrings(self.mem_addr, MEM_WINDOW);
        let hex_w = self.snapshot.memory.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        // Two spaces of gutter, and only when there is something to put there:
        // an engine with no Z-text (Glulx, Scott) leaves the rows their own width.
        let zwidest = self.snapshot.memory_zstrings.iter().flatten()
            .map(|z| z.chars().count()).max();
        self.snapshot.memory_zcol = hex_w + 2;
        self.snapshot.memory_width = match zwidest {
            Some(w) => self.snapshot.memory_zcol + w,
            None => hex_w,
        };
        // A narrower window than the one that set it must not leave the view
        // scrolled off the end of the content.
        self.mem_hscroll = self.mem_hscroll.min(self.snapshot.memory_width.saturating_sub(1));
    }

    /// Label for the live disassembly mode (for the hint bar `r:` entry).
    pub fn disasm_mode_label(&self) -> &'static str {
        match self.disasm_mode {
            DisasmMode::Full => "full",
            DisasmMode::Basic => "basic",
            DisasmMode::Raw => "raw",
        }
    }

    /// `window`'s active tab index moves by `dir` (wrapping); its scroll resets.
    fn cycle_tab(&mut self, dir: i32) {
        let window = self.focus;
        let n = self.tabs[window].len() as i32;
        self.tab[window] = (self.tab[window] as i32 + dir).rem_euclid(n) as usize;
        self.scroll[window] = 0;
        self.sel = None; // switching sections invalidates the selection (SQ-0420)
    }

    pub fn handle_key(&mut self, code: KeyCode, dbg: &dyn Debugger) -> DebugKey {
        // Memory address-input line intercepts first, so typing hex digits
        // (or cancelling/submitting) never falls through to scroll/tab keys.
        if self.active_section(self.focus) == Section::Memory {
            if let Some(result) = self.handle_memory_input_key(code, dbg) {
                return result;
            }
        }
        // NB: Tab / Shift-Tab (window focus, including the story pane) are handled
        // one level up by `AppState::cycle_focus`, not here — so a debug window is
        // just one stop in the unified per-window cycle.
        match code {
            KeyCode::Left => self.cycle_tab(-1),
            KeyCode::Right => self.cycle_tab(1),
            // Only in the Disasm tab, like `r` below: re-anchoring the
            // disassembly is invisible from any other section, so accepting it
            // there only swallowed the key and did nothing a user could see.
            // The hint bar has listed it under Disassembly alone since SQ-0980;
            // this is the handler catching up (SQ-0984).
            KeyCode::Char('g') if self.active_section(self.focus) == Section::Disasm => {
                self.disasm_addr = self.pc;
                self.reload_disasm(dbg);
            }
            // Only in the Disasm tab, so it doesn't shadow keys in other sections.
            KeyCode::Char('r') if self.active_section(self.focus) == Section::Disasm => {
                self.disasm_mode = match self.disasm_mode {
                    DisasmMode::Full => DisasmMode::Basic,
                    DisasmMode::Basic => DisasmMode::Raw,
                    DisasmMode::Raw => DisasmMode::Full,
                };
                self.reload_disasm(dbg);
            }
            // Pan the hex dump sideways (SQ-0965). `h`/`l` rather than the
            // arrows because Left/Right are the panel's section cycler, and
            // `handle_key` sees no modifiers to hang a Shift-arrow on — the same
            // trade the Anim context already makes, where plain arrows step the
            // playback and hjkl pan the map (SQ-0416). Memory-only, so `h`/`l`
            // stay available to global dispatch in every other tab, and only
            // reachable when the address box is closed (it swallows letters).
            KeyCode::Char('h') if self.active_section(self.focus) == Section::Memory => {
                self.step_memory_h(false);
            }
            KeyCode::Char('l') if self.active_section(self.focus) == Section::Memory => {
                self.step_memory_h(true);
            }
            // Home/End go to the ends, in every section — see `jump_active`.
            KeyCode::Home | KeyCode::End => {
                self.jump_active(self.focus, code == KeyCode::End, dbg);
            }
            KeyCode::Down | KeyCode::Up | KeyCode::PageDown | KeyCode::PageUp => {
                let window = self.focus;
                match self.active_section(window) {
                    Section::Disasm | Section::Memory => {
                        let step = matches!(code, KeyCode::PageDown | KeyCode::PageUp)
                            .then(|| self.page()).unwrap_or(1);
                        let down = matches!(code, KeyCode::Down | KeyCode::PageDown);
                        for _ in 0..step { self.scroll_active(window, down, dbg); }
                    }
                    section => self.scroll_list_key(window, section, code),
                }
            }
            _ => return DebugKey::Ignored,
        }
        DebugKey::Consumed
    }

    /// Handle a key while the focused window's active section is Memory.
    /// Returns `None` when the key isn't part of the address-input flow (so
    /// `handle_key` falls through to the normal scroll/tab dispatch);
    /// `Some(DebugKey::Consumed)` once the input flow handles it — including
    /// swallowing keys while editing, so typing/navigating the buffer never
    /// leaks into scrolling or tab-switching.
    fn handle_memory_input_key(&mut self, code: KeyCode, dbg: &dyn Debugger) -> Option<DebugKey> {
        if self.mem_input.is_none() {
            return match code {
                KeyCode::Char(':') | KeyCode::Char('/') => {
                    self.mem_input = Some(String::new());
                    Some(DebugKey::Consumed)
                }
                _ => None,
            };
        }
        match code {
            // Alphanumerics so variable tokens (`g44`, `local10`, `sp`) type as
            // well as hex addresses; unparseable input is simply a no-op on Enter.
            KeyCode::Char(c) if c.is_ascii_alphanumeric() => {
                self.mem_input.as_mut().expect("checked Some above").push(c);
            }
            KeyCode::Backspace => {
                self.mem_input.as_mut().expect("checked Some above").pop();
            }
            KeyCode::Enter => {
                let buf = self.mem_input.take().expect("checked Some above");
                if let Some(addr) = resolve_mem_target(buf.trim(), dbg) {
                    // Align down to the 16-byte row grid so the jump doesn't
                    // shift every row off the hex dump's column alignment.
                    self.mem_addr = addr.min(dbg.memory_len()) & !0xF;
                    self.reload_memory(dbg);
                }
            }
            KeyCode::Esc => {
                self.mem_input = None;
            }
            _ => {} // swallow anything else while editing
        }
        Some(DebugKey::Consumed)
    }

    /// Scroll `window`'s active section by one step. Used by the key path
    /// (looped for PageUp/PageDown) and directly by the mouse wheel (any
    /// window, regardless of focus). Recomputes only the scrolled section's
    /// lines — never calls `refresh`, which would re-anchor the disassembly
    /// to the PC and fight a manual scroll within the turn.
    pub fn scroll_active(&mut self, window: usize, down: bool, dbg: &dyn Debugger) {
        self.sel = None; // scrolling moves content out from under any selection (SQ-0420)
        match self.active_section(window) {
            Section::Disasm => self.step_disasm(down, dbg),
            Section::Memory => self.step_memory(down, dbg),
            section => self.scroll_list(window, section, down),
        }
    }

    /// Jump `window`'s active section to one end of itself: the top, or as far
    /// as it will go (SQ-0984).
    ///
    /// One meaning for `Home`/`End` in every section, and the meaning is "where
    /// holding the arrow key gets you". They used to be that only in the list
    /// sections; in Disassembly and Memory they were routed through the same
    /// per-step loop as `Down`/`Up` with a step count of ONE, so they moved a
    /// single instruction or a single hex row — which is why SQ-0980 declined to
    /// advertise them at all. A key the hint bar cannot describe in one word is a
    /// key that means two things.
    ///
    /// The two address-anchored sections ask the ENGINE where their ends are
    /// rather than assuming: `prev_instr`/`next_instr` clamp to the first and last
    /// unit the disassembler holds, so handing them the ends of the address space
    /// lands exactly on those units whatever region the story's code occupies, and
    /// Memory's end is `step_memory`'s own clamp — the last row a full 16 bytes
    /// wide. The list sections keep the offsets `scroll_list` converges on.
    fn jump_active(&mut self, window: usize, end: bool, dbg: &dyn Debugger) {
        self.sel = None; // as in `scroll_active`: the content moves out from under it
        match self.active_section(window) {
            Section::Disasm => {
                self.disasm_addr =
                    if end { dbg.next_instr(dbg.memory_len()) } else { dbg.prev_instr(0) };
                self.reload_disasm(dbg);
            }
            Section::Memory => {
                self.mem_addr = if end { dbg.memory_len().saturating_sub(16) } else { 0 };
                self.reload_memory(dbg);
            }
            section => {
                let max = self.snapshot.section(section).len().saturating_sub(1);
                self.scroll[window] = if end { max } else { 0 };
            }
        }
    }

    fn step_disasm(&mut self, down: bool, dbg: &dyn Debugger) {
        if down {
            self.disasm_addr = dbg.next_instr(self.disasm_addr);
        } else {
            self.disasm_addr = dbg.prev_instr(self.disasm_addr);
        }
        self.reload_disasm(dbg);
    }

    fn step_memory(&mut self, down: bool, dbg: &dyn Debugger) {
        let delta = 16u32;
        if down {
            let max = dbg.memory_len().saturating_sub(16);
            self.mem_addr = (self.mem_addr + delta).min(max);
        } else {
            self.mem_addr = self.mem_addr.saturating_sub(delta);
        }
        self.reload_memory(dbg);
    }

    /// Scroll the Memory dump sideways by one step (SQ-0965). Clamped to the
    /// loaded window's widest row, so it can never run off into blank columns.
    fn step_memory_h(&mut self, right: bool) {
        /// Columns per step: two hex bytes plus their spaces, so a step lands on
        /// a byte boundary and eight of them clear the 48-column hex field.
        const STEP: usize = 6;
        let max = self.snapshot.memory_width.saturating_sub(1);
        self.mem_hscroll = if right {
            (self.mem_hscroll + STEP).min(max)
        } else {
            self.mem_hscroll.saturating_sub(STEP)
        };
    }

    /// Pan `window`'s active section sideways by one step, for the mouse wheel
    /// (SQ-0981). `window` is explicit — like [`scroll_active`](Self::scroll_active)
    /// and unlike the `h`/`l` key path, which only ever reaches the FOCUSED
    /// window — so Shift+wheel over the hex dump pans it whether or not it holds
    /// focus, matching the wheel's convention everywhere else in the inspector.
    ///
    /// Returns `false` when `window` shows a section that does not pan, so the
    /// caller can fall back to a plain vertical scroll rather than swallowing
    /// the gesture.
    pub fn pan_active(&mut self, window: usize, right: bool) -> bool {
        if self.active_section(window) != Section::Memory {
            return false;
        }
        self.step_memory_h(right);
        true
    }

    fn scroll_list(&mut self, window: usize, section: Section, down: bool) {
        let max = self.snapshot.section(section).len().saturating_sub(1);
        self.scroll[window] = if down {
            (self.scroll[window] + 1).min(max)
        } else {
            self.scroll[window].saturating_sub(1)
        };
    }

    fn scroll_list_key(&mut self, window: usize, section: Section, code: KeyCode) {
        self.sel = None; // scrolling moves content out from under any selection (SQ-0420)
        let len = self.snapshot.section(section).len();
        let vp = self.page();
        let max = len.saturating_sub(1);
        self.scroll[window] = match code {
            KeyCode::Down => (self.scroll[window] + 1).min(max),
            KeyCode::Up => self.scroll[window].saturating_sub(1),
            KeyCode::PageDown => (self.scroll[window] + vp).min(max),
            KeyCode::PageUp => self.scroll[window].saturating_sub(vp),
            // Home/End never reach here — `handle_key` sends them to
            // `jump_active`, which is the one place that says what they mean.
            _ => self.scroll[window],
        };
    }

    /// Mouse: focus `window` (click in its body).
    pub fn focus_window(&mut self, window: usize) {
        self.focus = window;
    }

    /// Mouse: activate `tab` in `window` and focus it (click on a tab label).
    pub fn activate_tab(&mut self, window: usize, tab: usize) {
        self.tab[window] = tab;
        self.scroll[window] = 0;
        self.focus = window;
        self.sel = None; // switching sections invalidates the selection (SQ-0420)
        // Switching tabs abandons any in-progress memory address input, so it
        // can't be left open-but-hidden (which would swallow Esc's pop-to-story).
        self.mem_input = None;
    }

    /// Focus the window and select the tab that renders `section`. Returns the
    /// window index, so callers can address that window's per-window state
    /// (scroll, etc.) without hard-coding it.
    fn show_section(&mut self, section: Section) -> usize {
        let (w, t) = self.locate(section);
        self.focus = w;
        self.tab[w] = t;
        w
    }

    /// Navigate the disassembly to `addr` (focus the Disassembly tab). Does NOT
    /// call `refresh` — that re-anchors to the live PC, which would instantly
    /// undo the jump. Within-turn nav, like scrolling; the next per-turn refresh
    /// re-anchors to PC as usual.
    pub fn goto(&mut self, addr: u32, dbg: &dyn Debugger) {
        self.disasm_addr = addr;
        self.show_section(Section::Disasm);
        self.reload_disasm(dbg);
    }

    /// Mouse: toggle object `n`'s expansion in the Objects tree (collapse if
    /// already expanded, else expand + fetch its detail lines).
    pub fn toggle_object(&mut self, n: u16, dbg: &dyn Debugger) {
        if self.expanded_objects.remove(&n) {
            self.snapshot.object_details.remove(&n);
        } else {
            self.expanded_objects.insert(n);
            self.snapshot.object_details.insert(n, dbg.object_detail(n));
        }
    }

    /// Mouse: toggle call-stack frame `idx`'s expansion (collapse if already
    /// expanded, else expand + fetch its locals detail lines).
    pub fn toggle_frame(&mut self, idx: usize, dbg: &dyn Debugger) {
        if self.expanded_frames.remove(&idx) {
            self.snapshot.frame_details.remove(&idx);
        } else {
            self.expanded_frames.insert(idx);
            self.snapshot.frame_details.insert(idx, dbg.frame_locals(idx));
        }
    }

    /// Focus the Memory window/tab and point it at `addr`.
    pub fn goto_memory(&mut self, addr: u32, dbg: &dyn Debugger) {
        self.show_section(Section::Memory);
        // Align down to the 16-byte row grid so the jump keeps the hex dump's
        // column alignment (scroll then advances by whole 16-byte rows).
        self.mem_addr = addr.min(dbg.memory_len()) & !0xF;
        self.reload_memory(dbg);
    }

    /// Focus the Objects window/tab, expand object `n`, and scroll it into
    /// view.
    pub fn goto_object(&mut self, n: u16, dbg: &dyn Debugger) {
        let w = self.show_section(Section::Objects);
        if self.expanded_objects.insert(n) {
            self.snapshot.object_details.insert(n, dbg.object_detail(n));
        }
        let rows = objects_rows(&self.snapshot.objects, &self.expanded_objects, &self.snapshot.object_details, 0, usize::MAX);
        if let Some(idx) = rows.iter().position(|r| matches!(r, ObjRow::Tree { obj: Some(id), .. } if *id == n)) {
            self.scroll[w] = idx;
        }
    }
}

// ── Geometry (pure; shared by render and mouse hit-testing) ───────────────────

/// One screen row of the Disassembly section: either the `▼── PC ──▼` divider
/// or a disasm line, identified by its index into the snapshot's `disasm` Vec.
#[derive(Clone, Copy)]
pub struct DisasmRow { pub divider: bool, pub line_idx: usize }

/// The screen rows to draw for the disassembly, inserting a PC-divider row
/// above the line at `pc`. `disasm` is the pre-windowed snapshot (index 0 =
/// top). Shared by the renderer and the click hit-test so they never disagree
/// on which screen row is which disasm line.
pub fn disasm_rows(disasm: &[String], pc: u32, height: usize) -> Vec<DisasmRow> {
    let pc_prefix = format!("{:06x}", pc);
    let mut rows = Vec::with_capacity(height);
    for (i, line) in disasm.iter().enumerate() {
        if line.starts_with(&pc_prefix) && rows.len() < height {
            rows.push(DisasmRow { divider: true, line_idx: i });
        }
        if rows.len() < height { rows.push(DisasmRow { divider: false, line_idx: i }); }
        if rows.len() >= height { break; }
    }
    rows
}

/// One screen row of the Objects section: either a tree line (`obj` is its
/// parsed `[N]` id, if any) or one of an expanded object's detail lines
/// (`di` indexes into that object's `object_details` entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjRow { Tree { line_idx: usize, obj: Option<u16> }, Detail { obj: u16, di: usize } }

/// One screen row of the Call Stack section: either a frame line (`frame` is its
/// parsed `#N` index, if any) or one of an expanded frame's locals detail lines
/// (`di` indexes into that frame's `frame_details` entry). Mirrors `ObjRow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackRow { Frame { line_idx: usize, frame: Option<usize> }, Detail { frame: usize, di: usize } }

/// Resolve a Memory-jump input to a target address. A variable token (`sp`,
/// `localN`, `gNN` — matching the disassembly's rendering) dereferences to the
/// variable's current value used AS an address; anything else parses as a
/// literal hex address.
fn resolve_mem_target(s: &str, dbg: &dyn Debugger) -> Option<u32> {
    if let Some(var) = parse_var_token(s) {
        return dbg.var_value(var).map(|v| v as u32);
    }
    u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

/// Parse a variable token into a Z-machine variable number: `sp` → 0,
/// `localN` → N+1 (N decimal, 0..=14), `gNN` → 16+NN (NN hex, 0..=0xef, matching
/// the disassembly's `g{:02x}`). `None` if `s` is not a variable token.
fn parse_var_token(s: &str) -> Option<u8> {
    if s == "sp" {
        return Some(0);
    }
    if let Some(n) = s.strip_prefix("local") {
        let idx: u8 = n.parse().ok()?;
        return (idx <= 14).then_some(idx + 1);
    }
    if let Some(n) = s.strip_prefix('g') {
        let idx = u8::from_str_radix(n, 16).ok()?;
        return (idx <= 0xef).then_some(idx + 16);
    }
    None
}

/// Parse the leading `[N]` object id from a tree line (e.g. `"  [12] lamp"`).
fn parse_obj_id(line: &str) -> Option<u16> {
    let start = line.find('[')?;
    let rest = line.get(start + 1..)?;
    let end = rest.find(']')?;
    rest.get(..end)?.parse().ok()
}

/// Interleave each object tree line with its expanded detail lines (if any),
/// apply `scroll` (display rows, not tree lines) and cap at `height`. Pure;
/// shared by the renderer and the click hit-test so scroll offset and
/// click-row→object mapping never drift (same discipline as `disasm_rows`).
pub fn objects_rows(
    objects: &[String],
    expanded: &std::collections::HashSet<u16>,
    details: &std::collections::HashMap<u16, Vec<String>>,
    scroll: usize,
    height: usize,
) -> Vec<ObjRow> {
    let mut all = Vec::new();
    for (i, line) in objects.iter().enumerate() {
        let obj = parse_obj_id(line);
        all.push(ObjRow::Tree { line_idx: i, obj });
        if let Some(n) = obj {
            if expanded.contains(&n) {
                if let Some(det) = details.get(&n) {
                    for di in 0..det.len() {
                        all.push(ObjRow::Detail { obj: n, di });
                    }
                }
            }
        }
    }
    all.into_iter().skip(scroll).take(height).collect()
}

/// Parse the leading `#N` frame index from a stack line (`"#0  fn@…"`). `None`
/// for a line without one (e.g. the `(no frames)` placeholder).
fn parse_frame_idx(line: &str) -> Option<usize> {
    let rest = line.strip_prefix('#')?;
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest.get(..end)?.parse().ok()
}

/// Interleave each frame line with its expanded locals detail lines (if any),
/// apply `scroll` (display rows, not frame lines) and cap at `height`. Pure;
/// shared by the renderer and both hit-tests so scroll offset and click-row→line
/// mapping never drift (same discipline as `objects_rows`).
pub fn stack_rows(
    stack: &[String],
    expanded: &std::collections::HashSet<usize>,
    details: &std::collections::HashMap<usize, Vec<String>>,
    scroll: usize,
    height: usize,
) -> Vec<StackRow> {
    let mut all = Vec::new();
    for (i, line) in stack.iter().enumerate() {
        let frame = parse_frame_idx(line);
        all.push(StackRow::Frame { line_idx: i, frame });
        if let Some(n) = frame {
            if expanded.contains(&n) {
                if let Some(det) = details.get(&n) {
                    for di in 0..det.len() {
                        all.push(StackRow::Detail { frame: n, di });
                    }
                }
            }
        }
    }
    all.into_iter().skip(scroll).take(height).collect()
}

/// A resolved click destination, tagged by the operand-role sigil it came from.
/// The disassembler (Phase 6b-1) emits these sigils; the app classifies each
/// clickable token to one of these and jumps to its referent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClickTarget {
    Code(u32),    // → Disassembly at address
    Memory(u32),  // → Memory window at address
    Object(u16),  // → Objects tab, expand object
    Global(u8),   // → Globals tab, scroll to global index (0..=239)
    Local(u8),    // → Locals tab, scroll to local index (0-based)
    Stack,        // → Stack (eval) tab
    MemVia(u8),   // → Memory window at the ADDRESS held by variable `u8`
                  //   (var-value convention: 0 = sp, 1..=15 = locals, 16.. = globals);
                  //   dereferenced at click time.
    ObjVia(u8),   // → Objects tab, the OBJECT whose number variable `u8` holds
                  //   (same var-value convention); dereferenced at click time.
}

/// Var-value number (0 = sp, 1..=15 = locals, 16.. = globals) for a variable
/// `ClickTarget`, or `None` for a non-variable target.
fn var_number(t: &ClickTarget) -> Option<u8> {
    match t {
        ClickTarget::Stack => Some(0),
        ClickTarget::Local(i) => Some(i + 1),
        ClickTarget::Global(i) => Some(i + 16),
        _ => None,
    }
}

/// Clickable operand-reference spans within a rendered line: the char range and
/// the tagged target. In Disasm, every role sigil the disassembler emits
/// (`@0x……` memory, `0x……`/`?0x……` code, `obj#N`, `gNN`, `localN`, `sp`) is
/// clickable; in Call Stack, the frame entry address (`fn@……`, non-zero). Pure;
/// shared by the render-underline pass and the mouse hit-test so they never drift.
pub fn clickable_spans(section: Section, line: &str) -> Vec<(core::ops::Range<usize>, ClickTarget)> {
    match section {
        // Variables (`gNN`/`localN`/`sp`) are NOT clickable — they show a hover
        // tooltip instead (see `hover_var_at`). Only memory/object/code
        // references keep their click-jump + underline. `classify_disasm_tokens`
        // itself still emits the variable variants for the hover path.
        Section::Disasm => classify_disasm_tokens(line).into_iter()
            .filter(|(_, t)| matches!(t,
                ClickTarget::Code(_) | ClickTarget::Memory(_) | ClickTarget::Object(_)
                | ClickTarget::MemVia(_) | ClickTarget::ObjVia(_)))
            .collect(),
        Section::CallStack => {
            // Both the routine entry (`fn@……`) and the return PC (`ret=……`) are
            // clickable code addresses → jump to the Disassembly.
            let mut spans = find_hex_spans(line, "fn@", 6);
            spans.extend(find_hex_spans(line, "ret=", 6));
            spans.into_iter()
                .filter(|(_, addr)| *addr != 0)
                .map(|(range, addr)| (range, ClickTarget::Code(addr)))
                .collect()
        }
        // Objects/Dict rows lead with their entry byte address as an `@0x……`
        // token → a Memory-pane jump (the same Memory target a disasm `@0x`
        // operand yields). Only the address token is clickable; the object
        // name / dictionary word is inert.
        Section::Objects | Section::Dict => find_hex_spans(line, "@0x", 6)
            .into_iter()
            .map(|(range, addr)| (range, ClickTarget::Memory(addr)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Whole-token classifier for a Disassembly line. Splits on ASCII whitespace and
/// commas (both are single-byte, so token ranges stay on valid `str` boundaries
/// even when a trailing `print`-family story-text token is multi-byte), then
/// classifies each complete token to a `ClickTarget`. Whole-token matching (not
/// substring scanning) is what keeps mnemonics like `get_prop`/`jg` from
/// false-matching a bare `g`/`0x`.
fn classify_disasm_tokens(line: &str) -> Vec<(core::ops::Range<usize>, ClickTarget)> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let is_sep = |b: u8| b == b',' || b.is_ascii_whitespace();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && is_sep(bytes[i]) { i += 1; }
        if i >= bytes.len() { break; }
        let start = i;
        while i < bytes.len() && !is_sep(bytes[i]) { i += 1; }
        let token = &line[start..i];
        if let Some((sub, target)) = classify_token(token) {
            out.push((start + sub.start..start + sub.end, target));
        }
    }
    out
}

/// Classify one whole token to a `ClickTarget`, returning the sub-range within
/// the token that its underline/hit-test span should cover (the whole token,
/// except the code case which drops a leading `?`/`~` branch prefix). Order
/// matters: `@0x` (memory) is checked before `0x` (code) so a memory sigil is
/// never misread as a code address.
fn classify_token(tok: &str) -> Option<(core::ops::Range<usize>, ClickTarget)> {
    // Annotation wrapper: `[obj#5]` (clickable) / `[lamp]` (dictionary, no match →
    // non-clickable). Classify the inner token and shift the span by +1 for the `[`.
    if let Some(inner) = tok.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        return classify_token(inner).map(|(r, t)| (r.start + 1..r.end + 1, t));
    }
    let is_hex = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
    let is_dec = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());

    // Memory: `@0x` + exactly 6 hex, nothing more.
    if let Some(rest) = tok.strip_prefix("@0x") {
        return (rest.len() == 6 && is_hex(rest))
            .then(|| u32::from_str_radix(rest, 16).ok().map(|a| (0..tok.len(), ClickTarget::Memory(a))))
            .flatten();
    }
    // Memory-via-variable: `@` + a variable sigil (`@localN`/`@gHH`/`@sp`) — a
    // variable used AS a memory address. Jumps to memory at the value the
    // variable currently holds, read at click time (a variable is a runtime
    // value). The variable part is still hover-classified for its value.
    if let Some(varpart) = tok.strip_prefix('@') {
        return classify_token(varpart)
            .and_then(|(_, t)| var_number(&t))
            .map(|var| (0..tok.len(), ClickTarget::MemVia(var)));
    }
    // Code: `0x` + 6 hex, optionally behind a `?` / `?~` branch prefix. Span
    // covers the `0x……` core (skip the prefix).
    {
        let prefix = tok.len() - tok.trim_start_matches(['?', '~']).len();
        let core = &tok[prefix..];
        if let Some(hex) = core.strip_prefix("0x") {
            if hex.len() == 6 && is_hex(hex) {
                return u32::from_str_radix(hex, 16).ok()
                    .map(|a| (prefix..tok.len(), ClickTarget::Code(a)));
            }
        }
    }
    // Object: `obj#` + decimal digits (constant), or `obj#` + a variable sigil
    // (`obj#local5`/`obj#g0f`/`obj#sp`) → the object whose number the variable
    // currently holds, resolved on click.
    if let Some(n) = tok.strip_prefix("obj#") {
        if is_dec(n) {
            return n.parse().ok().map(|num| (0..tok.len(), ClickTarget::Object(num)));
        }
        return classify_token(n)
            .and_then(|(_, t)| var_number(&t))
            .map(|var| (0..tok.len(), ClickTarget::ObjVia(var)));
    }
    // Global: exactly `g` + 2 hex digits.
    if let Some(n) = tok.strip_prefix('g') {
        if n.len() == 2 && is_hex(n) {
            return u8::from_str_radix(n, 16).ok().map(|g| (0..tok.len(), ClickTarget::Global(g)));
        }
    }
    // Local: `local` + decimal digits.
    if let Some(n) = tok.strip_prefix("local") {
        return (is_dec(n)).then(|| n.parse().ok().map(|l| (0..tok.len(), ClickTarget::Local(l)))).flatten();
    }
    // Stack: exactly `sp`.
    if tok == "sp" {
        return Some((0..tok.len(), ClickTarget::Stack));
    }
    None
}

/// Find every occurrence of `marker` followed by exactly `hex_len` hex digits,
/// returning the char range covering `marker` + the hex digits, and the
/// parsed address. Byte-indexed but panic-safe: a disasm line can carry
/// embedded multi-byte story text (from `print`-family instructions) after
/// the part we actually search, so every slice goes through `str::get`
/// (returns `None` off a char boundary) rather than direct indexing.
fn find_hex_spans(line: &str, marker: &str, hex_len: usize) -> Vec<(core::ops::Range<usize>, u32)> {
    let mut out = Vec::new();
    let mlen = marker.len();
    let mut i = 0;
    while i + mlen + hex_len <= line.len() {
        if line.get(i..i + mlen) == Some(marker) {
            let hex_start = i + mlen;
            let hex_end = hex_start + hex_len;
            if let Some(candidate) = line.get(hex_start..hex_end) {
                if candidate.bytes().all(|b| b.is_ascii_hexdigit()) {
                    if let Ok(addr) = u32::from_str_radix(candidate, 16) {
                        out.push((i..hex_end, addr));
                    }
                    i = hex_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Map a click at `(col, row)` inside the debug region to a jump target
/// address, if it landed on an underlined clickable span. Uses the same
/// `window_rects` / `disasm_rows` / scroll math the renderer uses, so it
/// never drifts. Only Disasm (window 0, tab 0) and Call Stack (window 2, tab
/// 0) are clickable in v1.
pub fn clickable_at(region: Rect, panel: &DebugPanelState, col: u16, row: u16) -> Option<ClickTarget> {
    let windows = window_rects(region);
    for (w, window_rect) in windows.iter().enumerate() {
        if col < window_rect.x || col >= window_rect.right()
            || row < window_rect.y || row >= window_rect.bottom() {
            continue;
        }
        // Content rect: border inset by one on every side (matches
        // `draw_pane_frame`'s frame.content for a Single/Double border).
        let content = Rect::new(
            window_rect.x + 1, window_rect.y + 1,
            window_rect.width.saturating_sub(2), window_rect.height.saturating_sub(2),
        );
        if col < content.x || col >= content.right() || row < content.y || row >= content.bottom() {
            return None;
        }
        let section = panel.active_section(w);
        return match (w, section) {
            (0, Section::Disasm) => {
                let r = (row - content.y) as usize;
                let rows = disasm_rows(&panel.snapshot.disasm, panel.pc, content.height as usize);
                let row_entry = rows.get(r)?;
                if row_entry.divider { return None; }
                let line = panel.snapshot.disasm.get(row_entry.line_idx)?;
                let off = (col.checked_sub(content.x + 1))? as usize;
                clickable_spans(section, line).into_iter()
                    .find(|(range, _)| range.contains(&off))
                    .map(|(_, target)| target)
            }
            (2, Section::CallStack) => {
                let r = (row - content.y) as usize;
                let rows = stack_rows(&panel.snapshot.stack, &panel.expanded_frames,
                                      &panel.snapshot.frame_details, panel.scroll[2], content.height as usize);
                match rows.get(r)? {
                    StackRow::Frame { line_idx, .. } => {
                        let line = panel.snapshot.stack.get(*line_idx)?;
                        let off = (col.checked_sub(content.x + 2))? as usize; // +2 for the "▶ " marker
                        clickable_spans(section, line).into_iter()
                            .find(|(range, _)| range.contains(&off)).map(|(_, t)| t)
                    }
                    StackRow::Detail { .. } => None,
                }
            }
            // Objects tree rows carry their entry-address `@0x……` link; a click
            // on it jumps the Memory pane (a click elsewhere on the row falls
            // through to `objects_click_at`'s expand/collapse). The frame text
            // draws past a 2-col "▶ " marker, like the Call Stack.
            //
            // Glulx repurposes this same slot for its Functions list (SQ-0472):
            // there, the entry address should jump the Disassembly instead — the
            // panel's relabelled tab (`tab_label`) is the mechanical signal, not
            // the clicked token's text, so a real Objects tree (still labelled
            // "Objects") keeps its Memory jump unchanged.
            (1, Section::Objects) => {
                let r = (row - content.y) as usize;
                let rows = objects_rows(&panel.snapshot.objects, &panel.expanded_objects,
                                        &panel.snapshot.object_details, panel.scroll[1], content.height as usize);
                let functions_mode = panel.tab_label(Section::Objects) == "Functions";
                match rows.get(r)? {
                    ObjRow::Tree { line_idx, .. } => {
                        let line = panel.snapshot.objects.get(*line_idx)?;
                        let off = (col.checked_sub(content.x + 2))? as usize; // +2 for the "▶ " marker
                        clickable_spans(section, line).into_iter()
                            .find(|(range, _)| range.contains(&off))
                            .map(|(_, t)| match t {
                                ClickTarget::Memory(a) if functions_mode => ClickTarget::Code(a),
                                other => other,
                            })
                    }
                    // Detail rows carry the object ENTRY's own `@0x……` link
                    // (SQ-0975), so the §12.3 entry stays reachable now that the
                    // tree row jumps to the property table instead. They draw
                    // under a 4-column indent (see `draw_objects`).
                    ObjRow::Detail { obj, di } => {
                        let line = panel.snapshot.object_details.get(obj)?.get(*di)?;
                        let off = (col.checked_sub(content.x + 4))? as usize;
                        clickable_spans(section, line).into_iter()
                            .find(|(range, _)| range.contains(&off))
                            .map(|(_, t)| match t {
                                ClickTarget::Memory(a) if functions_mode => ClickTarget::Code(a),
                                other => other,
                            })
                    }
                }
            }
            // Dictionary rows draw with no marker (the plain list path), so the
            // entry-address `@0x……` link sits at the content's left edge.
            (1, Section::Dict) => {
                let r = (row - content.y) as usize;
                let line = panel.snapshot.dict.get(panel.scroll[1] + r)?;
                let off = (col.checked_sub(content.x))? as usize;
                clickable_spans(section, line).into_iter()
                    .find(|(range, _)| range.contains(&off)).map(|(_, t)| t)
            }
            _ => None,
        };
    }
    None
}

/// If `(col,row)` lands on a variable operand (`gNN`/`localN`/`sp`) in the
/// Disassembly window, return its Z-machine variable number and the screen
/// anchor (the token's start col, its row) for a tooltip. Uses the same
/// window_rects / content-inset / disasm_rows math as clickable_at so it
/// never drifts. Runs `classify_disasm_tokens` (not the filtered
/// `clickable_spans`, which drops variables) to find the variable spans.
pub fn hover_var_at(region: Rect, panel: &DebugPanelState, col: u16, row: u16) -> Option<(u8, u16, u16)> {
    let windows = window_rects(region);
    for (w, window_rect) in windows.iter().enumerate() {
        if col < window_rect.x || col >= window_rect.right()
            || row < window_rect.y || row >= window_rect.bottom() {
            continue;
        }
        let content = Rect::new(
            window_rect.x + 1, window_rect.y + 1,
            window_rect.width.saturating_sub(2), window_rect.height.saturating_sub(2),
        );
        if col < content.x || col >= content.right() || row < content.y || row >= content.bottom() {
            return None;
        }
        if w != 0 || panel.active_section(0) != Section::Disasm {
            return None;
        }
        let r = (row - content.y) as usize;
        let rows = disasm_rows(&panel.snapshot.disasm, panel.pc, content.height as usize);
        let row_entry = rows.get(r)?;
        if row_entry.divider { return None; }
        let line = panel.snapshot.disasm.get(row_entry.line_idx)?;
        let off = (col.checked_sub(content.x + 1))? as usize;
        let (range, target) = classify_disasm_tokens(line).into_iter()
            .find(|(range, _)| range.contains(&off))?;
        let var = match target {
            ClickTarget::Stack => 0,
            ClickTarget::Local(i) => i + 1,
            ClickTarget::Global(i) => i + 16,
            // `@local5` / `obj#local5` etc. — hovering the address- or object-
            // holding variable still shows its current value (`v` is already the
            // var-value number).
            ClickTarget::MemVia(v) | ClickTarget::ObjVia(v) => v,
            _ => return None,
        };
        return Some((var, content.x + 1 + range.start as u16, row));
    }
    None
}

/// If `(col, row)` is over the mnemonic (opcode) token of a Disassembly
/// instruction line, returns `(instruction_address, tooltip_col, row)` for an
/// opcode-help tooltip. Returns `None` over the address, operands, a
/// header/`.byte`/Raw line, or outside the Disassembly window — mirrors
/// [`hover_var_at`]'s geometry.
pub fn hover_help_at(region: Rect, panel: &DebugPanelState, col: u16, row: u16) -> Option<(u32, u16, u16)> {
    let windows = window_rects(region);
    for (w, window_rect) in windows.iter().enumerate() {
        if col < window_rect.x || col >= window_rect.right()
            || row < window_rect.y || row >= window_rect.bottom() {
            continue;
        }
        let content = Rect::new(
            window_rect.x + 1, window_rect.y + 1,
            window_rect.width.saturating_sub(2), window_rect.height.saturating_sub(2),
        );
        if col < content.x || col >= content.right() || row < content.y || row >= content.bottom() {
            return None;
        }
        if w != 0 || panel.active_section(0) != Section::Disasm {
            return None;
        }
        let r = (row - content.y) as usize;
        let rows = disasm_rows(&panel.snapshot.disasm, panel.pc, content.height as usize);
        let row_entry = rows.get(r)?;
        if row_entry.divider {
            return None;
        }
        let line = panel.snapshot.disasm.get(row_entry.line_idx)?;
        // Lines are "AAAAAA  mnemonic operands…" (Full/Basic) — 6 hex address, two
        // spaces, then the mnemonic at column 8. Raw ("AAAAAA: …"), header
        // ("…  ; routine") and data ("…  .byte") lines have no lowercase-letter
        // token at column 8 and are rejected below.
        let addr = u32::from_str_radix(line.get(0..6)?, 16).ok()?;
        let after = line.get(8..)?;
        let tok_len = after.find([' ', ',']).unwrap_or(after.len());
        if !after.starts_with(|c: char| c.is_ascii_lowercase()) {
            return None;
        }
        let off = col.checked_sub(content.x + 1)? as usize;
        if off < 8 || off >= 8 + tok_len {
            return None;
        }
        return Some((addr, content.x + 1 + 8, row));
    }
    None
}

/// Hit-test a click at `(col, row)` against the Objects section's tree rows
/// (window 1, right-top). Returns the object id if the click landed on a
/// `Tree` row that carries one (a toggle target). Uses the same
/// `window_rects` / `objects_rows` geometry the renderer uses, so it never
/// drifts.
pub fn objects_click_at(region: Rect, panel: &DebugPanelState, col: u16, row: u16) -> Option<u16> {
    let window_rect = window_rects(region)[1];
    if col < window_rect.x || col >= window_rect.right()
        || row < window_rect.y || row >= window_rect.bottom() {
        return None;
    }
    if panel.active_section(1) != Section::Objects {
        return None;
    }
    let content = Rect::new(
        window_rect.x + 1, window_rect.y + 1,
        window_rect.width.saturating_sub(2), window_rect.height.saturating_sub(2),
    );
    if col < content.x || col >= content.right() || row < content.y || row >= content.bottom() {
        return None;
    }
    let r = (row - content.y) as usize;
    let rows = objects_rows(
        &panel.snapshot.objects, &panel.expanded_objects, &panel.snapshot.object_details,
        panel.scroll[1], content.height as usize,
    );
    match rows.get(r)? {
        ObjRow::Tree { obj: Some(n), .. } => Some(*n),
        _ => None,
    }
}

/// Hit-test a click at `(col, row)` against the Call Stack frame rows (window 2,
/// tab 0). Returns the frame index if the click landed on a `Frame` row that
/// carries one (a toggle target). Uses the same `window_rects` / content-inset /
/// `stack_rows` geometry the renderer uses, so it never drifts.
pub fn stack_click_at(region: Rect, panel: &DebugPanelState, col: u16, row: u16) -> Option<usize> {
    let window_rect = window_rects(region)[2];
    if col < window_rect.x || col >= window_rect.right()
        || row < window_rect.y || row >= window_rect.bottom() {
        return None;
    }
    if panel.active_section(2) != Section::CallStack {
        return None;
    }
    let content = Rect::new(
        window_rect.x + 1, window_rect.y + 1,
        window_rect.width.saturating_sub(2), window_rect.height.saturating_sub(2),
    );
    if col < content.x || col >= content.right() || row < content.y || row >= content.bottom() {
        return None;
    }
    let r = (row - content.y) as usize;
    let rows = stack_rows(
        &panel.snapshot.stack, &panel.expanded_frames, &panel.snapshot.frame_details,
        panel.scroll[2], content.height as usize,
    );
    match rows.get(r)? {
        StackRow::Frame { frame: Some(n), .. } => Some(*n),
        _ => None,
    }
}

/// Tile `region` into the three window rects: left full-height, right column
/// split top/bottom. Must match exactly what `render/debug_panel.rs` draws.
pub fn window_rects(region: Rect) -> [Rect; 3] {
    let left_w = region.width / 2;
    let right_x = region.x + left_w;
    let right_w = region.width - left_w;
    let top_h = region.height / 2;
    let left = Rect::new(region.x, region.y, left_w, region.height);
    let r_top = Rect::new(right_x, region.y, right_w, top_h);
    let r_bot = Rect::new(right_x, region.y + top_h, right_w, region.height - top_h);
    [left, r_top, r_bot]
}

/// The content rect of debug window `i` in `region` — the frame inset by its
/// 1-cell border. `None` if the window is too small to hold content. This is the
/// SAME inset the click hit-tests use, so a selection's coordinates line up with
/// what the renderer draws. (SQ-0420)
pub fn window_content(region: Rect, i: usize) -> Option<Rect> {
    let w = window_rects(region).get(i).copied()?;
    if w.width < 3 || w.height < 3 { return None; }
    Some(Rect::new(w.x + 1, w.y + 1, w.width - 2, w.height - 2))
}

/// Map a screen `(col, row)` to `(window, content-relative point)` — `None` if it
/// is not over any window's content area (borders/tabs excluded). Used to START a
/// mouse selection. (SQ-0420)
pub fn debug_point_at(region: Rect, col: u16, row: u16) -> Option<(usize, crate::clipboard::Point)> {
    for i in 0..3 {
        if let Some(c) = window_content(region, i) {
            if col >= c.x && col < c.x + c.width && row >= c.y && row < c.y + c.height {
                return Some((i, crate::clipboard::Point { row: (row - c.y) as usize, col: col - c.x }));
            }
        }
    }
    None
}

/// Clamp a screen `(col, row)` to window `win`'s content rect and return the
/// content-relative point — so a drag that strays outside the window clings to its
/// edge instead of jumping to another window. (SQ-0420)
pub fn debug_point_clamped(region: Rect, win: usize, col: u16, row: u16) -> crate::clipboard::Point {
    match window_content(region, win) {
        Some(c) => crate::clipboard::Point {
            row: (row.clamp(c.y, c.y + c.height - 1) - c.y) as usize,
            col: col.clamp(c.x, c.x + c.width - 1) - c.x,
        },
        None => crate::clipboard::Point { row: 0, col: 0 },
    }
}

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    // Minimal mock: 4-byte fixed instructions, 0x10000 bytes of memory.
    struct MockDbg;
    impl crate::engine::Debugger for MockDbg {
        fn pc(&self) -> u32 { 0x1000 }
        fn disassemble(&self, addr: u32, n: usize) -> Vec<String> {
            (0..n).map(|i| format!("{:06x}  add", addr + i as u32 * 4)).collect()
        }
        // Raw form is distinguishable from the translated `add` above (carries a
        // class tag) so the `r` cycle is testable.
        fn disassemble_raw(&self, addr: u32, n: usize) -> Vec<String> {
            (0..n).map(|i| format!("{:06x}: 54  2OP:0x14", addr + i as u32 * 4)).collect()
        }
        // Basic form: plain mnemonic, no `@0x` sigil, no `2OP:` class tag —
        // distinct from both `disassemble` and `disassemble_raw` above.
        fn disassemble_basic(&self, addr: u32, n: usize) -> Vec<String> {
            (0..n).map(|i| format!("{:06x}  loadw #0abc", addr + i as u32 * 4)).collect()
        }
        fn next_instr(&self, a: u32) -> u32 { a + 4 }
        fn prev_instr(&self, a: u32) -> u32 { a.saturating_sub(4) }
        fn executed_pcs(&self) -> std::collections::HashSet<u32> { std::collections::HashSet::new() }
        fn stack_lines(&self) -> Vec<String> { vec!["#0 main".into()] }
        fn eval_stack_lines(&self) -> Vec<String> { vec!["[  0] 0000  (0)".into()] }
        fn locals_lines(&self) -> Vec<String> { vec!["(none)".into()] }
        fn globals_lines(&self) -> Vec<String> { (0..240).map(|i| format!("g{i:02x}")).collect() }
        fn object_tree_lines(&self) -> Vec<String> { vec!["[1] thing".into()] }
        fn dictionary_lines(&self) -> Vec<String> { vec!["word".into()] }
        // The real row shape: a 6-digit address, 48 columns of hex and a
        // 16-column char column — 72 columns before the decoded-text column can
        // even start, which is what makes the horizontal scroll load-bearing at
        // any pane width the inspector actually gets (SQ-0965).
        fn memory_hex(&self, a: u32, r: usize) -> Vec<String> {
            (0..r)
                .map(|i| format!("{:06x}  {:<48}{}", a + i as u32 * 16, "00 ".repeat(16), ".".repeat(16)))
                .collect()
        }
        fn memory_len(&self) -> u32 { 0x10000 }
        fn object_detail(&self, _obj: u16) -> Vec<String> { vec!["attrs: (none)".into()] }
        fn frame_locals(&self, _idx: usize) -> Vec<String> { vec!["local0 = 0x0001  (1)".to_string()] }
        // Deterministic, distinct-per-var value so deref jumps are testable.
        fn var_value(&self, var: u8) -> Option<u16> { Some(0x1000 + var as u16 * 0x10) }
        // One "dictionary entry" whose text lands on the 0x2000 row and nowhere
        // else — the shape a real entry (`base + i * entry_length`) takes: it
        // starts at an odd address inside the row, so the row-aligned `mem_addr`
        // could never have been asked about it directly.
        fn memory_zstrings(&self, a: u32, r: usize) -> Vec<Option<String>> {
            (0..r).map(|i| (a + i as u32 * 16 == 0x2000).then(|| "lantern".to_string())).collect()
        }
    }

    #[test]
    fn r_cycles_disasm_mode_full_basic_raw_in_the_disasm_tab() {
        let mut p = DebugPanelState::new(0x1000);
        p.refresh(&MockDbg); // focus 0 / tab 0 = Disasm, Full view first.
        assert_eq!(p.disasm_mode, DisasmMode::Full);
        // Full: `add`, no basic `loadw`, no raw class tag.
        assert!(p.snapshot.disasm.iter().all(|l| l.contains("add") && !l.contains("loadw") && !l.contains("2OP:0x14")),
            "full view: {:?}", p.snapshot.disasm.first());
        // Full → Basic.
        assert_eq!(p.handle_key(KeyCode::Char('r'), &MockDbg), DebugKey::Consumed);
        assert_eq!(p.disasm_mode, DisasmMode::Basic);
        assert!(p.snapshot.disasm.iter().all(|l| l.contains("loadw") && !l.contains("2OP:0x14")),
            "basic view: {:?}", p.snapshot.disasm.first());
        // Basic → Raw.
        assert_eq!(p.handle_key(KeyCode::Char('r'), &MockDbg), DebugKey::Consumed);
        assert_eq!(p.disasm_mode, DisasmMode::Raw);
        assert!(p.snapshot.disasm.iter().all(|l| l.contains("2OP:0x14")),
            "raw view: {:?}", p.snapshot.disasm.first());
        // Raw → Full (wraps).
        assert_eq!(p.handle_key(KeyCode::Char('r'), &MockDbg), DebugKey::Consumed);
        assert_eq!(p.disasm_mode, DisasmMode::Full);
        assert!(p.snapshot.disasm.iter().all(|l| l.contains("add") && !l.contains("2OP:0x14")),
            "full view restored: {:?}", p.snapshot.disasm.first());
    }

    #[test]
    fn r_is_ignored_outside_the_disasm_tab() {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = 1; // Locals | Objects | Dictionary — not Disasm.
        assert_eq!(p.handle_key(KeyCode::Char('r'), &MockDbg), DebugKey::Ignored);
        assert_eq!(p.disasm_mode, DisasmMode::Full, "no cycle outside the Disasm tab");
    }

    /// SQ-0984: `g` was accepted from every tab and only ever re-anchored the
    /// disassembly, so outside Disassembly it swallowed the key and did nothing
    /// the user could see — while the hint bar had already listed it under
    /// Disassembly alone.
    #[test]
    fn g_is_ignored_outside_the_disasm_tab() {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = 2;
        p.tab[2] = 0; // Call Stack — not Disasm.
        p.disasm_addr = 0x3000;
        assert_eq!(p.handle_key(KeyCode::Char('g'), &MockDbg), DebugKey::Ignored);
        assert_eq!(p.disasm_addr, 0x3000, "no re-anchor from a tab that shows no disassembly");

        // And the direction that stops that passing for a `g` nobody handles:
        // in the Disassembly it still jumps to the PC.
        p.focus = 0;
        assert_eq!(p.handle_key(KeyCode::Char('g'), &MockDbg), DebugKey::Consumed);
        assert_eq!(p.disasm_addr, 0x1000, "MockDbg parks the PC at 0x1000");
    }

    #[test]
    fn disasm_mode_label_names_each_variant() {
        let mut p = DebugPanelState::new(0x1000);
        assert_eq!(p.disasm_mode_label(), "full");
        p.disasm_mode = DisasmMode::Basic;
        assert_eq!(p.disasm_mode_label(), "basic");
        p.disasm_mode = DisasmMode::Raw;
        assert_eq!(p.disasm_mode_label(), "raw");
    }

    // Window focus (Tab / Shift-Tab, including the story pane) is handled by
    // AppState::cycle_focus, not the panel — see state::tests::cycle_focus_*.

    #[test]
    fn left_right_cycle_focused_tab_with_wrap_and_reset_scroll() {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = 1; // a 4-tab window (Globals | Locals | Objects | Dictionary)
        p.tab[1] = 0; // start from the first tab
        p.scroll[1] = 5;
        p.handle_key(KeyCode::Right, &MockDbg);
        assert_eq!(p.tab[1], 1);
        assert_eq!(p.scroll[1], 0, "tab switch resets scroll");
        p.scroll[1] = 3;
        p.handle_key(KeyCode::Right, &MockDbg);
        assert_eq!(p.tab[1], 2);
        p.handle_key(KeyCode::Right, &MockDbg);
        assert_eq!(p.tab[1], 3); // Globals
        p.handle_key(KeyCode::Right, &MockDbg); // wraps
        assert_eq!(p.tab[1], 0);
        p.handle_key(KeyCode::Left, &MockDbg); // wraps the other way
        assert_eq!(p.tab[1], 3);
    }

    #[test]
    fn disasm_scroll_advances_and_retreats_by_instruction_symmetrically() {
        let mut p = DebugPanelState::new(0x1000);
        // focus 0 / tab 0 is Disasm by default. MockDbg's next_instr/prev_instr
        // are inverses (+4/-4), so scrolling down then up round-trips exactly —
        // no history buffer needed (unlike the old disasm_history model).
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1004);
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1008);
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1004);
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1000);
        // Scrolling up before ever scrolling down still retreats — Feature B:
        // backward scroll is not gated on scroll-down history.
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x0ffc);
    }

    #[test]
    fn memory_scroll_clamps_at_memory_len() {
        let mut p = memory_focused_panel();
        p.mem_addr = 0x10000 - 16;
        p.handle_key(KeyCode::Down, &MockDbg);
        // Was `tab[2] = 1`, which is the Stack — so this drove a LIST scroll and
        // asserted `mem_addr` had not changed, which it never could (SQ-0984).
        assert_eq!(p.mem_addr, 0x10000 - 16, "the last full row is as far down as it goes");
    }

    /// SQ-0984: `Home`/`End` mean "the ends" in every section.
    ///
    /// They already did in the list sections. In Disassembly and Memory they were
    /// routed through the same per-step loop as the arrows with a step count of
    /// one, so they moved a SINGLE instruction or a single hex row — which is why
    /// the hint bar could not name them. Both directions here, in all three kinds
    /// of section, because "jumps to the end" and "steps once" agree whenever the
    /// cursor happens to start one step from the end.
    #[test]
    fn home_and_end_go_to_the_ends_of_every_section() {
        // Disassembly: the engine is asked where its ends are — `prev_instr`/
        // `next_instr` clamp to the first and last unit a real cache holds, and
        // MockDbg's unbounded ±4 shows the question being put to it.
        let mut p = DebugPanelState::new(0x1000);
        p.handle_key(KeyCode::Home, &MockDbg);
        assert_eq!(p.disasm_addr, 0, "prev_instr(0), not one instruction back from the PC");
        p.handle_key(KeyCode::End, &MockDbg);
        assert_eq!(p.disasm_addr, 0x10004, "next_instr(memory_len), not one instruction on");

        // Memory: the top of the dump, and the last row a full sixteen bytes wide —
        // which is exactly where holding Down ends up (`memory_scroll_clamps…`).
        let mut p = memory_focused_panel();
        p.mem_addr = 0x8000;
        p.handle_key(KeyCode::Home, &MockDbg);
        assert_eq!(p.mem_addr, 0, "the start of memory, not 16 bytes back");
        p.handle_key(KeyCode::End, &MockDbg);
        assert_eq!(p.mem_addr, 0x10000 - 16, "the last full row, not 16 bytes on");

        // A list section: unchanged, and pinned here so the three read as one rule.
        let mut p = DebugPanelState::new(0x1000);
        p.refresh(&MockDbg);
        p.focus = 1; // Globals, MockDbg's 240 lines.
        let last = p.snapshot.globals.len() - 1;
        assert!(last > 1, "the mock must supply a list worth scrolling: {last}");
        p.handle_key(KeyCode::End, &MockDbg);
        assert_eq!(p.scroll[1], last);
        p.handle_key(KeyCode::Home, &MockDbg);
        assert_eq!(p.scroll[1], 0);
    }

    #[test]
    fn refresh_re_anchors_disasm_to_pc() {
        let mut p = DebugPanelState::new(0x2000);
        p.disasm_addr = 0x3000;
        p.refresh(&MockDbg);
        assert_eq!(p.pc, 0x1000);
        assert_eq!(p.disasm_addr, 0x1000);
        assert!(!p.snapshot.disasm.is_empty());
    }

    #[test]
    fn active_section_mapping() {
        let mut p = DebugPanelState::new(0x1000);
        // Each window opens on its DEFAULT_SECTIONS entry.
        assert_eq!(p.active_section(0), Section::Disasm);
        assert_eq!(p.active_section(1), Section::Globals);
        assert_eq!(p.active_section(2), Section::CallStack);
        // Selecting any section's tab makes active_section report it, whatever
        // the tab order (locate_section is the single source of truth).
        for sec in [
            Section::Disasm, Section::Globals, Section::Locals, Section::Objects,
            Section::Dict, Section::CallStack, Section::EvalStack, Section::Memory,
        ] {
            let (w, t) = locate_section(sec);
            p.tab[w] = t;
            assert_eq!(p.active_section(w), sec);
        }
    }

    #[test]
    fn window_tabs_cover_every_section() {
        // locate_section / DEFAULT_SECTIONS are total only if every Section lives
        // in exactly one window — guard that invariant so a future edit can't
        // silently drop or duplicate a tab.
        for sec in [
            Section::Disasm, Section::Globals, Section::Locals, Section::Objects,
            Section::Dict, Section::CallStack, Section::EvalStack, Section::Memory,
        ] {
            let count = WINDOW_TABS.iter().flat_map(|w| w.iter()).filter(|&&s| s == sec).count();
            assert_eq!(count, 1, "{sec:?} must appear in exactly one window");
        }
    }

    #[test]
    fn window_rects_tiles_the_region_without_overlap() {
        let region = Rect::new(0, 0, 61, 40);
        let [left, top, bot] = window_rects(region);
        // Left is full height; right column split top/bottom, same x/width.
        assert_eq!(left.height, region.height);
        assert_eq!(top.x, bot.x);
        assert_eq!(top.width, bot.width);
        assert_eq!(top.y, region.y);
        assert_eq!(top.y + top.height, bot.y);
        assert_eq!(bot.y + bot.height, region.y + region.height);
        // Left + right widths cover the region with no gap or overlap.
        assert_eq!(left.width + top.width, region.width);
        assert_eq!(left.x, region.x);
        assert_eq!(top.x, left.x + left.width);
    }

    // ── disasm_rows (Feature B: shared row model) ──────────────────────────────

    #[test]
    fn disasm_rows_inserts_a_pc_divider_above_the_pc_line() {
        let disasm = vec!["001000  add".to_string(), "001004  sub".to_string(), "001008  mul".to_string()];
        let rows = disasm_rows(&disasm, 0x1004, 10);
        assert_eq!(rows.len(), 4);
        assert!(!rows[0].divider && rows[0].line_idx == 0);
        assert!(rows[1].divider && rows[1].line_idx == 1);
        assert!(!rows[2].divider && rows[2].line_idx == 1);
        assert!(!rows[3].divider && rows[3].line_idx == 2);
    }

    #[test]
    fn disasm_rows_caps_at_height_including_the_divider() {
        let disasm = vec!["001000  add".to_string(), "001004  sub".to_string()];
        let rows = disasm_rows(&disasm, 0x1000, 2);
        // The divider consumes one of the 2 available rows, so only the PC
        // line itself fits — not also the next disasm line.
        assert_eq!(rows.len(), 2);
        assert!(rows[0].divider);
        assert!(!rows[1].divider && rows[1].line_idx == 0);
    }

    #[test]
    fn disasm_rows_no_divider_when_pc_is_off_screen() {
        let disasm = vec!["001000  add".to_string(), "001004  sub".to_string()];
        let rows = disasm_rows(&disasm, 0x9999, 10);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| !r.divider));
    }

    // ── clickable_spans / clickable_at (Feature C: shared click model) ─────────

    #[test]
    fn clickable_spans_finds_a_disasm_branch_target() {
        let line = "001000  je local0, #01 ?0x001234";
        let spans = clickable_spans(Section::Disasm, line);
        // The branch target is the Code span (the `local0` operand also
        // classifies now — a Local — but this test is about the branch target).
        let (range, target) = spans.iter()
            .find(|(_, t)| matches!(t, ClickTarget::Code(_)))
            .expect("a Code target");
        assert_eq!(*target, ClickTarget::Code(0x1234));
        assert_eq!(&line[range.clone()], "0x001234");
    }

    #[test]
    fn clickable_spans_classifies_every_operand_sigil_in_order() {
        let line = "004a2f  loadw @0x001234, g0f -> local2  ?0x004b00";
        let spans = clickable_spans(Section::Disasm, line);
        let targets: Vec<ClickTarget> = spans.iter().map(|(_, t)| *t).collect();
        // Variables (`g0f`, `local2`) are filtered out of the clickable set —
        // they're hover-only now; only memory and code references remain, in order.
        assert_eq!(targets, vec![
            ClickTarget::Memory(0x1234),
            ClickTarget::Code(0x4b00),
        ]);
        assert_eq!(&line[spans[0].0.clone()], "@0x001234");
        assert_eq!(&line[spans[1].0.clone()], "0x004b00"); // span skips the `?`
    }

    #[test]
    fn clickable_spans_classifies_object_and_stack_sigils() {
        let line = "004a2f  get_prop obj#5 -> sp";
        let spans = clickable_spans(Section::Disasm, line);
        let targets: Vec<ClickTarget> = spans.iter().map(|(_, t)| *t).collect();
        // Whole-token classification: `get_prop` must NOT false-match a `g..`
        // global or an embedded `0x`. The `sp` operand is a variable, so it's
        // filtered out of the clickable set — only Object(5) remains.
        assert_eq!(targets, vec![ClickTarget::Object(5)]);
        assert_eq!(&line[spans[0].0.clone()], "obj#5");
    }

    #[test]
    fn classify_token_unwraps_object_annotation_bracket() {
        // `[obj#5]` → Object(5), span covering the inner `obj#5` (shifted +1).
        let tok = "[obj#5]";
        let (span, target) = classify_token(tok).expect("[obj#5] classifies");
        assert_eq!(target, ClickTarget::Object(5));
        assert_eq!(&tok[span], "obj#5");
    }

    #[test]
    fn classify_token_dictionary_bracket_is_not_clickable() {
        // `[lamp]` → inner `lamp` matches nothing → None (dictionary is informational).
        assert_eq!(classify_token("[lamp]"), None);
    }

    #[test]
    fn classify_token_memory_via_variable() {
        // `@localN`/`@gHH`/`@sp` → MemVia in the var-value convention
        // (0 = sp, 1..=15 = locals, 16.. = globals), span over the whole token.
        assert_eq!(classify_token("@sp"), Some((0..3, ClickTarget::MemVia(0))));
        assert_eq!(classify_token("@local5"), Some((0..7, ClickTarget::MemVia(6))));
        // g0f → global index 15 → var 15 + 16 = 31.
        assert_eq!(classify_token("@g0f"), Some((0..4, ClickTarget::MemVia(31))));
        // A constant memory address is still the plain Memory target, not MemVia.
        assert_eq!(classify_token("@0x001234"), Some((0..9, ClickTarget::Memory(0x1234))));
        // `@` on a non-variable is not a link.
        assert_eq!(classify_token("@nonsense"), None);
    }

    #[test]
    fn classify_token_object_via_variable() {
        // `obj#<var>` → ObjVia in the var-value convention; `obj#<decimal>` stays Object.
        assert_eq!(classify_token("obj#sp"), Some((0..6, ClickTarget::ObjVia(0))));
        assert_eq!(classify_token("obj#local0"), Some((0..10, ClickTarget::ObjVia(1))));
        assert_eq!(classify_token("obj#g05"), Some((0..7, ClickTarget::ObjVia(21))));
        assert_eq!(classify_token("obj#5"), Some((0..5, ClickTarget::Object(5))));
        assert_eq!(classify_token("obj#bogus"), None);
    }

    #[test]
    fn object_via_variable_is_clickable_and_still_hoverable() {
        let line = "001000  get_child obj#local0 -> sp ?0x001010";
        let spans = clickable_spans(Section::Disasm, line);
        assert!(spans.iter().any(|(_, t)| matches!(t, ClickTarget::ObjVia(1))),
            "obj#local0 should be a clickable ObjVia span: {spans:?}");
        let (p, region, content, row_y) = hover_panel(line);
        let col = content.x + 1 + line.find("obj#local0").unwrap() as u16 + 4; // over "local0"
        assert_eq!(hover_var_at(region, &p, col, row_y).map(|(v, ..)| v), Some(1));
    }

    #[test]
    fn memory_via_variable_is_clickable_and_still_hoverable() {
        // The `@local5` operand underlines + click-jumps (MemVia) AND the inner
        // variable stays hover-classified for its value.
        let line = "001000  loadw @local5, #00 -> sp";
        let spans = clickable_spans(Section::Disasm, line);
        assert!(spans.iter().any(|(_, t)| matches!(t, ClickTarget::MemVia(6))),
            "@local5 should be a clickable MemVia span: {spans:?}");
        // Hover over it still yields the variable (value tooltip).
        let (p, region, content, row_y) = hover_panel(line);
        let col = content.x + 1 + line.find("@local5").unwrap() as u16 + 1; // over "local5"
        assert_eq!(hover_var_at(region, &p, col, row_y).map(|(v, ..)| v), Some(6));
    }

    #[test]
    fn clickable_spans_makes_object_annotation_clickable_but_not_dictionary() {
        let line = "004a2f  loadw @0x00abcd [obj#5], #00";
        let spans = clickable_spans(Section::Disasm, line);
        // The `[obj#5]` annotation yields a clickable Object(5) span.
        let obj = spans
            .iter()
            .find(|(_, t)| matches!(t, ClickTarget::Object(_)))
            .expect("[obj#5] annotation is clickable");
        assert_eq!(obj.1, ClickTarget::Object(5));
        assert_eq!(&line[obj.0.clone()], "obj#5");

        // A dictionary annotation yields no clickable span.
        let dict_line = "004a2f  storeb @0x001a2c [lamp], #01";
        let dict_spans = clickable_spans(Section::Disasm, dict_line);
        assert!(dict_spans.iter().all(|(_, t)| !matches!(t, ClickTarget::Object(_))));
    }

    #[test]
    fn clickable_spans_finds_a_call_stack_frame_address_and_skips_a_zero_address() {
        let line = "#0  fn@004a00  ret=004a35  args=2";
        let spans = clickable_spans(Section::CallStack, line);
        // Both the entry (fn@) and the return PC (ret=) are clickable.
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].1, ClickTarget::Code(0x4a00));
        assert_eq!(&line[spans[0].0.clone()], "fn@004a00");
        assert_eq!(spans[1].1, ClickTarget::Code(0x4a35));
        assert_eq!(&line[spans[1].0.clone()], "ret=004a35");

        // A zero entry address is skipped, but the non-zero return PC still clicks.
        let zero_line = "#0  fn@000000  ret=004a35  args=2";
        let spans = clickable_spans(Section::CallStack, zero_line);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].1, ClickTarget::Code(0x4a35));
    }

    #[test]
    fn clickable_spans_empty_for_other_sections() {
        assert!(clickable_spans(Section::Globals, "g00=0012").is_empty());
    }

    #[test]
    fn clickable_spans_finds_the_object_entry_address_link() {
        // An Objects tree row leads with its entry byte address as an `@0x……`
        // Memory-jump token; the `[N] name` text is inert.
        let line = "@0x000110 [1] lamp";
        let spans = clickable_spans(Section::Objects, line);
        assert_eq!(spans.len(), 1, "only the entry address is clickable: {spans:?}");
        assert_eq!(spans[0].1, ClickTarget::Memory(0x110));
        assert_eq!(&line[spans[0].0.clone()], "@0x000110");
    }

    #[test]
    fn clickable_spans_finds_the_dict_entry_address_link() {
        let line = "@0x000abc open";
        let spans = clickable_spans(Section::Dict, line);
        assert_eq!(spans.len(), 1, "only the entry address is clickable: {spans:?}");
        assert_eq!(spans[0].1, ClickTarget::Memory(0xabc));
        assert_eq!(&line[spans[0].0.clone()], "@0x000abc");
    }

    #[test]
    fn clickable_at_resolves_an_object_entry_address_click_to_a_memory_jump() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        let (ow, ot) = locate_section(Section::Objects);
        p.focus = ow;
        p.tab[ow] = ot;
        p.snapshot.objects = vec!["@0x000110 [1] lamp".to_string()];
        let wrect = window_rects(region)[ow];
        let content = Rect::new(wrect.x + 1, wrect.y + 1, wrect.width.saturating_sub(2), wrect.height.saturating_sub(2));
        // Line text draws past the 2-col "▶ " marker; the `@0x` sits at its start.
        let col = content.x + 2 + p.snapshot.objects[0].find("@0x").unwrap() as u16;
        assert_eq!(clickable_at(region, &p, col, content.y), Some(ClickTarget::Memory(0x110)));
    }

    /// SQ-0975: the tree row now points at the property table (where the name
    /// is), so the §12.3 entry lives on the expanded detail's `entry @0x……`
    /// line — and that line has to be clickable, or the entry stops being
    /// reachable at all. Detail rows draw under a 4-column indent.
    #[test]
    fn clickable_at_resolves_an_expanded_detail_entry_address_click_to_a_memory_jump() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        let (ow, ot) = locate_section(Section::Objects);
        p.focus = ow;
        p.tab[ow] = ot;
        // The tree row carries the PROPERTY TABLE address, the detail the entry.
        p.snapshot.objects = vec!["@0x000340 [1] lamp".to_string()];
        p.expanded_objects.insert(1);
        p.snapshot.object_details.insert(1, vec!["entry @0x000110".into(), "attrs: (none)".into()]);
        let wrect = window_rects(region)[ow];
        let content = Rect::new(wrect.x + 1, wrect.y + 1, wrect.width.saturating_sub(2), wrect.height.saturating_sub(2));
        // Row 0 is the tree line → the property table.
        let tree_col = content.x + 2 + p.snapshot.objects[0].find("@0x").unwrap() as u16;
        assert_eq!(
            clickable_at(region, &p, tree_col, content.y),
            Some(ClickTarget::Memory(0x340)),
            "the row itself jumps to the name",
        );
        // Row 1 is the first detail line → the entry, still one click away.
        let detail = &p.snapshot.object_details[&1][0];
        let det_col = content.x + 4 + detail.find("@0x").unwrap() as u16;
        assert_eq!(
            clickable_at(region, &p, det_col, content.y + 1),
            Some(ClickTarget::Memory(0x110)),
            "the entry stays reachable from the expanded detail",
        );
        // A detail row with no address is still inert (the old behaviour).
        assert_eq!(clickable_at(region, &p, content.x + 4, content.y + 2), None);
    }

    #[test]
    fn clickable_at_resolves_a_functions_row_entry_address_click_to_a_disasm_jump() {
        // Glulx relabels the Objects slot "Functions" (SQ-0472); a click on a
        // Functions row's entry address should jump the Disassembly, not Memory
        // — driven mechanically by the relabelled tab, same row/column geometry
        // as the plain Objects case above.
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        let (ow, ot) = locate_section(Section::Objects);
        p.focus = ow;
        p.tab[ow] = ot;
        p.labels.insert(Section::Objects, "Functions");
        p.snapshot.objects = vec!["@0x004a00  C0  2 locals  [Rd]".to_string()];
        let wrect = window_rects(region)[ow];
        let content = Rect::new(wrect.x + 1, wrect.y + 1, wrect.width.saturating_sub(2), wrect.height.saturating_sub(2));
        let col = content.x + 2 + p.snapshot.objects[0].find("@0x").unwrap() as u16;
        assert_eq!(clickable_at(region, &p, col, content.y), Some(ClickTarget::Code(0x4a00)));
    }

    #[test]
    fn clickable_at_resolves_a_dict_entry_address_click_to_a_memory_jump() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        let (dw, dt) = locate_section(Section::Dict);
        p.focus = dw;
        p.tab[dw] = dt;
        p.snapshot.dict = vec!["@0x000abc open".to_string()];
        let wrect = window_rects(region)[dw];
        let content = Rect::new(wrect.x + 1, wrect.y + 1, wrect.width.saturating_sub(2), wrect.height.saturating_sub(2));
        // The plain list path draws at the content's left edge (no marker).
        let col = content.x + p.snapshot.dict[0].find("@0x").unwrap() as u16;
        assert_eq!(clickable_at(region, &p, col, content.y), Some(ClickTarget::Memory(0xabc)));
    }

    #[test]
    fn clickable_spans_does_not_panic_on_embedded_multi_byte_story_text() {
        // `print`-family instructions can embed arbitrary (multi-byte UTF-8)
        // story text right in the disasm line, near a real `0x……` marker.
        // find_hex_spans is byte-indexed; every slice must go through
        // `str::get` (never direct indexing) or a byte offset landing mid
        // char boundary panics. Spans may come back empty or correct — the
        // only requirement is no panic.
        let line = "004a2f  print \"café ➤ 0x1234\"";
        let spans = clickable_spans(Section::Disasm, line);
        // If a span was found, its range must still slice cleanly.
        for (range, _) in &spans {
            let _ = &line[range.clone()];
        }
    }

    #[test]
    fn clickable_at_resolves_a_branch_target_click_in_disasm() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        p.pc = 0x1000;
        p.snapshot.disasm = vec![
            "001000  je local0, #01 ?0x001234".to_string(),
            "001004  add".to_string(),
        ];
        let [left, ..] = window_rects(region);
        let content = Rect::new(left.x + 1, left.y + 1, left.width.saturating_sub(2), left.height.saturating_sub(2));
        // Row 0 is the PC divider (line 0 IS the PC line); row 1 is line_idx 0.
        let row_y = content.y + 1;
        let line = &p.snapshot.disasm[0];
        let off = line.find("0x").unwrap();
        let col = content.x + 1 + off as u16; // +1 for the execution-mark gutter
        let hit = clickable_at(region, &p, col, row_y);
        assert_eq!(hit, Some(ClickTarget::Code(0x1234)));
    }

    #[test]
    fn clickable_at_resolves_memory_but_not_variable_clicks_in_disasm() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        p.pc = 0x1000;
        p.snapshot.disasm = vec![
            "001000  loadw @0x001234, g0f -> sp".to_string(),
            "001004  add".to_string(),
        ];
        let [left, ..] = window_rects(region);
        let content = Rect::new(left.x + 1, left.y + 1, left.width.saturating_sub(2), left.height.saturating_sub(2));
        // Row 0 is the PC divider (line 0 IS the PC line); row 1 is line_idx 0.
        let row_y = content.y + 1;
        let line = &p.snapshot.disasm[0];
        let mem_col = content.x + 1 + line.find("@0x").unwrap() as u16;
        assert_eq!(clickable_at(region, &p, mem_col, row_y), Some(ClickTarget::Memory(0x1234)));
        // A variable operand is no longer clickable — it's hover-only.
        let g_col = content.x + 1 + line.find("g0f").unwrap() as u16;
        assert_eq!(clickable_at(region, &p, g_col, row_y), None);
    }

    #[test]
    fn clickable_at_returns_none_on_the_divider_row() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        p.pc = 0x1000;
        p.snapshot.disasm = vec!["001000  add".to_string()];
        let [left, ..] = window_rects(region);
        let content_y = left.y + 1;
        let hit = clickable_at(region, &p, left.x + 3, content_y);
        assert_eq!(hit, None);
    }

    #[test]
    fn clickable_at_resolves_a_call_stack_frame_address_click() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        p.snapshot.stack = vec!["#0  fn@004a00  ret=004a35  args=2".to_string()];
        let [_, _, bot] = window_rects(region);
        let content = Rect::new(bot.x + 1, bot.y + 1, bot.width.saturating_sub(2), bot.height.saturating_sub(2));
        let line = &p.snapshot.stack[0];
        let off = line.find("fn@").unwrap();
        // +2 for the "▶ " disclosure marker prefixing the frame text.
        let col = content.x + 2 + off as u16;
        let hit = clickable_at(region, &p, col, content.y);
        assert_eq!(hit, Some(ClickTarget::Code(0x4a00)));

        // A click on an expanded frame's detail row resolves to nothing.
        p.expanded_frames.insert(0);
        p.snapshot.frame_details.insert(0, vec!["local0 = 0x0001  (1)".to_string()]);
        let detail_col = content.x + 2 + off as u16;
        assert_eq!(clickable_at(region, &p, detail_col, content.y + 1), None,
            "detail row carries no clickable address");
    }

    #[test]
    fn clickable_at_resolves_a_call_stack_return_address_click_to_a_disasm_jump() {
        // The `ret=` token (the frame's return PC) is clickable separately from
        // `fn@` (the frame's entry) — both are Code targets, but this exercises
        // `ret=` specifically (SQ-0472).
        let region = Rect::new(0, 0, 61, 40);
        let p = {
            let mut p = DebugPanelState::new(0x1000);
            p.snapshot.stack = vec!["#0  fn@004a00  ret=004a35  args=2".to_string()];
            p
        };
        let [_, _, bot] = window_rects(region);
        let content = Rect::new(bot.x + 1, bot.y + 1, bot.width.saturating_sub(2), bot.height.saturating_sub(2));
        let line = &p.snapshot.stack[0];
        let off = line.find("ret=").unwrap();
        // +2 for the "▶ " disclosure marker prefixing the frame text.
        let col = content.x + 2 + off as u16;
        assert_eq!(clickable_at(region, &p, col, content.y), Some(ClickTarget::Code(0x4a35)));
    }

    #[test]
    fn goto_navigates_disasm_without_touching_pc_or_calling_refresh() {
        let mut p = DebugPanelState::new(0x1000);
        p.pc = 0x1000;
        p.focus = 2;
        p.goto(0x2000, &MockDbg);
        assert_eq!(p.disasm_addr, 0x2000);
        assert_eq!(p.focus, 0);
        assert_eq!(p.tab[0], 0);
        assert_eq!(p.pc, 0x1000, "goto must not re-anchor to PC (that's refresh's job)");
        assert_eq!(p.snapshot.disasm[0], format!("{:06x}  add", 0x2000));
    }

    #[test]
    fn g_recenters_the_disasm_on_the_live_pc_after_a_jump() {
        // The existing 'g' hotkey (hint bar: "g: PC") is the back-to-PC
        // affordance for a jump landed via a Functions/Call-Stack click —
        // verify it round-trips after `goto` moves `disasm_addr` away from `pc`.
        let mut p = DebugPanelState::new(0x1000);
        p.pc = 0x1000;
        p.goto(0x4a00, &MockDbg);
        assert_eq!(p.disasm_addr, 0x4a00);
        p.handle_key(KeyCode::Char('g'), &MockDbg);
        assert_eq!(p.disasm_addr, 0x1000, "'g' must recenter on the live PC");
        assert_eq!(p.snapshot.disasm[0], format!("{:06x}  add", 0x1000));
    }

    // ── Memory address-input line ───────────────────────────────────────────

    fn memory_focused_panel() -> DebugPanelState {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = 2;
        p.tab[2] = 2; // Memory
        p
    }

    #[test]
    fn colon_opens_the_memory_input_and_digits_edit_it() {
        let mut p = memory_focused_panel();
        assert_eq!(p.handle_key(KeyCode::Char(':'), &MockDbg), DebugKey::Consumed);
        assert_eq!(p.mem_input.as_deref(), Some(""));
        p.handle_key(KeyCode::Char('1'), &MockDbg);
        p.handle_key(KeyCode::Char('a'), &MockDbg);
        assert_eq!(p.mem_input.as_deref(), Some("1a"));
        p.handle_key(KeyCode::Backspace, &MockDbg);
        assert_eq!(p.mem_input.as_deref(), Some("1"));
    }

    #[test]
    fn enter_parses_the_memory_input_as_hex_and_jumps_mem_addr() {
        let mut p = memory_focused_panel();
        p.handle_key(KeyCode::Char(':'), &MockDbg);
        for c in "2000".chars() { p.handle_key(KeyCode::Char(c), &MockDbg); }
        assert_eq!(p.handle_key(KeyCode::Enter, &MockDbg), DebugKey::Consumed);
        assert_eq!(p.mem_addr, 0x2000);
        assert!(p.mem_input.is_none());
        assert!(p.snapshot.memory[0].starts_with(&format!("{:06x}", 0x2000)));
    }

    /// The decoded text beside memory row `row` of the loaded window.
    fn zrow(p: &DebugPanelState, row: usize) -> Option<&str> {
        p.snapshot.memory_zstrings.get(row)?.as_deref()
    }

    #[test]
    fn a_jump_decodes_onto_the_row_the_entry_lives_on_not_a_caption_above_it() {
        // SQ-0969: the view snaps down to the 16-byte grid, and the entry starts
        // somewhere inside that row. The text belongs beside its own bytes — a
        // caption over the dump only restated the tab you clicked from.
        let mut p = memory_focused_panel();
        p.goto_memory(0x2005, &MockDbg);
        assert_eq!(p.mem_addr, 0x2000, "the dump still starts on the row boundary");
        assert_eq!(zrow(&p, 0), Some("lantern"), "the entry's own row carries its text");
        assert_eq!(zrow(&p, 1), None, "and the row after it claims nothing");
    }

    #[test]
    fn a_row_no_table_accounts_for_shows_no_decode_at_all() {
        // The whole point: an unanchored row falls back to the hex row's own
        // char column rather than a plausible-looking wrong decode.
        let mut p = memory_focused_panel();
        p.goto_memory(0x2005, &MockDbg);
        p.goto_memory(0x3000, &MockDbg);
        assert!(
            p.snapshot.memory_zstrings.iter().all(|z| z.is_none()),
            "and the previous jump's text does not linger",
        );
    }

    #[test]
    fn the_decode_travels_with_its_row_when_the_dump_scrolls() {
        // Scrolling moves the window, so the entry's text moves up a row with
        // the bytes that produced it — it is never left labelling a row that no
        // longer holds it.
        let mut p = memory_focused_panel();
        p.goto_memory(0x2005, &MockDbg);
        p.scroll_active(2, true, &MockDbg);
        assert_eq!(p.mem_addr, 0x2010);
        assert_eq!(zrow(&p, 0), None, "0x2010 is not the entry's row");
        p.scroll_active(2, false, &MockDbg);
        assert_eq!(zrow(&p, 0), Some("lantern"), "and comes back on return");
    }

    #[test]
    fn the_memory_input_box_loads_the_decode_for_its_jump_too() {
        let mut p = memory_focused_panel();
        p.handle_key(KeyCode::Char(':'), &MockDbg);
        for c in "2005".chars() { p.handle_key(KeyCode::Char(c), &MockDbg); }
        p.handle_key(KeyCode::Enter, &MockDbg);
        assert_eq!(p.mem_addr, 0x2000);
        assert_eq!(zrow(&p, 0), Some("lantern"));
    }

    // ── Horizontal scrolling (SQ-0965) ─────────────────────────────────────

    #[test]
    fn h_and_l_pan_the_hex_dump_and_clamp_at_both_ends() {
        // MockDbg's rows are the real 72 columns wide, plus a 2-column gutter
        // and "lantern" — 81 in all, so the far column is unreachable at any
        // width the inspector gets without this.
        let mut p = memory_focused_panel();
        p.goto_memory(0x2005, &MockDbg);
        assert_eq!(p.snapshot.memory_zcol, 74, "hex row width + a 2-column gutter");
        assert_eq!(p.snapshot.memory_width, 81, "…and the widest row reaches past it");

        assert_eq!(p.mem_hscroll, 0);
        assert_eq!(p.handle_key(KeyCode::Char('l'), &MockDbg), DebugKey::Consumed);
        assert_eq!(p.mem_hscroll, 6, "one step is two hex bytes and their spaces");
        p.handle_key(KeyCode::Char('h'), &MockDbg);
        assert_eq!(p.mem_hscroll, 0);
        p.handle_key(KeyCode::Char('h'), &MockDbg);
        assert_eq!(p.mem_hscroll, 0, "and cannot go negative");

        for _ in 0..40 { p.handle_key(KeyCode::Char('l'), &MockDbg); }
        assert_eq!(p.mem_hscroll, 80, "clamped to the widest row, never past it");
    }

    #[test]
    fn panning_the_dump_reaches_the_decoded_text_column() {
        // Non-vacuity for the clamp above: the scroll must actually be able to
        // put the Z-text column at the left edge of the pane.
        let mut p = memory_focused_panel();
        p.goto_memory(0x2005, &MockDbg);
        while p.mem_hscroll < p.snapshot.memory_zcol {
            p.handle_key(KeyCode::Char('l'), &MockDbg);
        }
        assert!(p.mem_hscroll >= 74 && p.mem_hscroll < p.snapshot.memory_width);
    }

    #[test]
    fn a_narrower_window_pulls_the_scroll_back_onto_the_content() {
        // Jumping to a region with no decoded text shortens the longest row from
        // 81 to 72; a scroll left at 80 would be looking at nothing.
        let mut p = memory_focused_panel();
        p.goto_memory(0x2005, &MockDbg);
        for _ in 0..40 { p.handle_key(KeyCode::Char('l'), &MockDbg); }
        assert_eq!(p.mem_hscroll, 80);
        p.goto_memory(0x3000, &MockDbg);
        assert_eq!(p.snapshot.memory_width, 72, "no Z-text column here");
        assert_eq!(p.mem_hscroll, 71, "so the scroll is pulled back onto the hex");
    }

    // ── Wheel pan (SQ-0981) ────────────────────────────────────────────────

    /// The whole point: `h`/`l` only ever reach the FOCUSED window, but the
    /// wheel addresses whatever is under the cursor. Shift+wheel over the hex
    /// dump must pan it while the focus sits somewhere else entirely.
    #[test]
    fn the_wheel_pans_a_memory_window_that_does_not_have_focus() {
        let mut p = memory_focused_panel();
        p.goto_memory(0x2005, &MockDbg);
        p.focus = 0; // Disassembly — the hex dump is visible but not focused.
        assert_ne!(p.active_section(p.focus), Section::Memory, "focus is elsewhere");
        // The key path is inert from here, exactly as before.
        assert_eq!(p.handle_key(KeyCode::Char('l'), &MockDbg), DebugKey::Ignored);
        assert_eq!(p.mem_hscroll, 0);
        // The wheel is not.
        assert!(p.pan_active(2, true), "window 2 shows Memory, so it pans");
        assert_eq!(p.mem_hscroll, 6, "one step is two hex bytes and their spaces");
        assert!(p.pan_active(2, false));
        assert_eq!(p.mem_hscroll, 0);
    }

    #[test]
    fn the_wheel_pan_clamps_exactly_as_h_and_l_do() {
        let mut by_key = memory_focused_panel();
        by_key.goto_memory(0x2005, &MockDbg);
        for _ in 0..40 { by_key.handle_key(KeyCode::Char('l'), &MockDbg); }
        let mut by_wheel = memory_focused_panel();
        by_wheel.goto_memory(0x2005, &MockDbg);
        by_wheel.focus = 0;
        for _ in 0..40 { by_wheel.pan_active(2, true); }
        assert_eq!(by_wheel.mem_hscroll, 80, "clamped to the widest row");
        assert_eq!(by_wheel.mem_hscroll, by_key.mem_hscroll, "the same clamp as the keys");
        for _ in 0..40 { by_wheel.pan_active(2, false); }
        assert_eq!(by_wheel.mem_hscroll, 0, "and cannot go negative");
    }

    /// A window showing anything else reports "not mine", so the caller falls
    /// back to a plain vertical scroll instead of eating the gesture.
    #[test]
    fn the_wheel_pan_declines_every_section_but_memory() {
        let mut p = memory_focused_panel();
        for w in 0..3 {
            for tab in 0..p.tabs[w].len() {
                p.tab[w] = tab;
                let section = p.active_section(w);
                let panned = p.pan_active(w, true);
                assert_eq!(
                    panned,
                    section == Section::Memory,
                    "{section:?} in window {w} should pan == {}",
                    section == Section::Memory
                );
            }
        }
    }

    /// The unmodified wheel is untouched: it still steps the dump's ADDRESS
    /// down a row and leaves the sideways position alone.
    #[test]
    fn an_unmodified_wheel_still_scrolls_the_memory_window_vertically() {
        let mut p = memory_focused_panel();
        p.goto_memory(0x2005, &MockDbg);
        p.focus = 0;
        let addr = p.mem_addr;
        p.pan_active(2, true); // put the pan somewhere non-zero first
        assert_eq!(p.mem_hscroll, 6);
        p.scroll_active(2, true, &MockDbg);
        assert_eq!(p.mem_addr, addr + 16, "the wheel still moves a row down");
        assert_eq!(p.mem_hscroll, 6, "and leaves the sideways pan where it was");
    }

    #[test]
    fn h_and_l_are_ignored_outside_the_memory_tab() {
        // They must fall through to global dispatch everywhere else — the panel
        // does not own two plain letters across the whole inspector.
        let mut p = DebugPanelState::new(0x1000);
        p.focus = 0; // Disassembly
        assert_eq!(p.handle_key(KeyCode::Char('l'), &MockDbg), DebugKey::Ignored);
        assert_eq!(p.handle_key(KeyCode::Char('h'), &MockDbg), DebugKey::Ignored);
        assert_eq!(p.mem_hscroll, 0);
    }

    #[test]
    fn typing_h_into_the_memory_address_box_does_not_pan_the_dump() {
        // The address box takes alphanumerics (`sp`, `g44`, `local10`), so it
        // must swallow `h` and `l` while it is open.
        let mut p = memory_focused_panel();
        p.handle_key(KeyCode::Char(':'), &MockDbg);
        p.handle_key(KeyCode::Char('l'), &MockDbg);
        p.handle_key(KeyCode::Char('h'), &MockDbg);
        assert_eq!(p.mem_input.as_deref(), Some("lh"));
        assert_eq!(p.mem_hscroll, 0, "the dump did not move");
    }

    #[test]
    fn esc_cancels_the_memory_input_without_changing_mem_addr() {
        let mut p = memory_focused_panel();
        p.mem_addr = 0x40;
        p.handle_key(KeyCode::Char(':'), &MockDbg);
        p.handle_key(KeyCode::Char('9'), &MockDbg);
        assert_eq!(p.handle_key(KeyCode::Esc, &MockDbg), DebugKey::Consumed);
        assert!(p.mem_input.is_none());
        assert_eq!(p.mem_addr, 0x40, "Esc must not commit the in-progress buffer");
    }

    #[test]
    fn typing_while_editing_the_memory_input_does_not_scroll_or_switch_tabs() {
        let mut p = memory_focused_panel();
        p.mem_addr = 0x100;
        p.handle_key(KeyCode::Char(':'), &MockDbg);
        // Down/Left would normally scroll memory / switch the window's tab —
        // while editing they must be swallowed, not fall through.
        p.handle_key(KeyCode::Down, &MockDbg);
        p.handle_key(KeyCode::Left, &MockDbg);
        assert_eq!(p.mem_addr, 0x100, "arrow keys must not scroll while editing");
        assert_eq!(p.tab[2], 2, "arrow keys must not switch tabs while editing");
        assert!(p.mem_input.is_some(), "still editing");
    }

    #[test]
    fn slash_also_opens_the_memory_input() {
        let mut p = memory_focused_panel();
        assert_eq!(p.handle_key(KeyCode::Char('/'), &MockDbg), DebugKey::Consumed);
        assert_eq!(p.mem_input.as_deref(), Some(""));
    }

    #[test]
    fn switching_tabs_clears_an_open_memory_input() {
        // Opening the address input then switching tabs (e.g. a mouse click on
        // the Stack tab) must not leave it open-but-hidden — a stale mem_input
        // swallows Esc's pop-to-story shortcut.
        let mut p = memory_focused_panel();
        p.handle_key(KeyCode::Char(':'), &MockDbg);
        assert!(p.mem_input.is_some());
        p.activate_tab(2, 0); // switch window 2 to the Call Stack tab
        assert!(p.mem_input.is_none(), "tab switch must abandon the input");
    }

    #[test]
    fn colon_does_not_open_input_outside_the_memory_section() {
        let mut p = DebugPanelState::new(0x1000); // focus 0 = Disasm
        assert_eq!(p.handle_key(KeyCode::Char(':'), &MockDbg), DebugKey::Ignored);
        assert!(p.mem_input.is_none());
    }

    // ── objects_rows (shared Objects display model) ─────────────────────────

    #[test]
    fn objects_rows_interleaves_detail_lines_after_an_expanded_object() {
        let objects = vec!["[1] lamp".to_string(), "[2] rock".to_string()];
        let expanded = std::collections::HashSet::from([1u16]);
        let details = std::collections::HashMap::from([
            (1u16, vec!["attrs: 5".to_string(), "  prop 1: 01 02".to_string()]),
        ]);
        let rows = objects_rows(&objects, &expanded, &details, 0, 10);
        assert_eq!(rows.len(), 4); // tree[1] + 2 detail lines + tree[2]
        assert!(matches!(rows[0], ObjRow::Tree { line_idx: 0, obj: Some(1) }));
        assert!(matches!(rows[1], ObjRow::Detail { obj: 1, di: 0 }));
        assert!(matches!(rows[2], ObjRow::Detail { obj: 1, di: 1 }));
        assert!(matches!(rows[3], ObjRow::Tree { line_idx: 1, obj: Some(2) }));
    }

    #[test]
    fn objects_rows_applies_scroll_and_height_over_the_interleaved_rows() {
        let objects = vec!["[1] lamp".to_string(), "[2] rock".to_string()];
        let expanded = std::collections::HashSet::from([1u16]);
        let details = std::collections::HashMap::from([(1u16, vec!["attrs: 5".to_string()])]);
        let rows = objects_rows(&objects, &expanded, &details, 1, 1);
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0], ObjRow::Detail { obj: 1, di: 0 }));
    }

    #[test]
    fn parse_obj_id_reads_the_leading_bracketed_number() {
        assert_eq!(parse_obj_id("[12] West of House"), Some(12));
        assert_eq!(parse_obj_id("  [3] lamp"), Some(3));
        assert_eq!(parse_obj_id("no brackets here"), None);
    }

    #[test]
    fn toggle_object_expands_then_collapses() {
        let mut p = DebugPanelState::new(0x1000);
        p.snapshot.objects = vec!["[1] lamp".to_string()];
        p.toggle_object(1, &MockDbg);
        assert!(p.expanded_objects.contains(&1));
        assert!(p.snapshot.object_details.contains_key(&1));
        p.toggle_object(1, &MockDbg);
        assert!(!p.expanded_objects.contains(&1));
        assert!(!p.snapshot.object_details.contains_key(&1));
    }

    #[test]
    fn objects_click_at_resolves_a_tree_row_click() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        let (ow, ot) = locate_section(Section::Objects);
        p.focus = ow;
        p.tab[ow] = ot;
        p.snapshot.objects = vec!["[1] lamp".to_string()];
        let wrect = window_rects(region)[ow];
        let content = Rect::new(wrect.x + 1, wrect.y + 1, wrect.width.saturating_sub(2), wrect.height.saturating_sub(2));
        let hit = objects_click_at(region, &p, content.x, content.y);
        assert_eq!(hit, Some(1));
    }

    #[test]
    fn objects_click_at_ignores_clicks_when_objects_is_not_the_active_tab() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        p.tab[1] = 0; // Locals, not Objects
        p.snapshot.objects = vec!["[1] lamp".to_string()];
        let [_, top, _] = window_rects(region);
        let content = Rect::new(top.x + 1, top.y + 1, top.width.saturating_sub(2), top.height.saturating_sub(2));
        assert_eq!(objects_click_at(region, &p, content.x, content.y), None);
    }

    // ── stack_rows / toggle_frame / stack_click_at (Call Stack expansion) ───

    #[test]
    fn stack_rows_interleaves_detail_lines_after_an_expanded_frame() {
        let stack = vec!["#0  fn@004a00  ret=004a35  args=2".to_string(),
                         "#1  fn@005000  ret=005035  args=0".to_string()];
        let expanded = std::collections::HashSet::from([0usize]);
        let details = std::collections::HashMap::from([
            (0usize, vec!["local0 = 0x0001  (1)".to_string(), "local1 = 0x0002  (2)".to_string()]),
        ]);
        let rows = stack_rows(&stack, &expanded, &details, 0, 10);
        assert_eq!(rows.len(), 4); // frame#0 + 2 detail lines + frame#1
        assert!(matches!(rows[0], StackRow::Frame { line_idx: 0, frame: Some(0) }));
        assert!(matches!(rows[1], StackRow::Detail { frame: 0, di: 0 }));
        assert!(matches!(rows[2], StackRow::Detail { frame: 0, di: 1 }));
        assert!(matches!(rows[3], StackRow::Frame { line_idx: 1, frame: Some(1) }));
    }

    #[test]
    fn parse_frame_idx_reads_the_leading_hash_number() {
        assert_eq!(parse_frame_idx("#0  fn@004a00  ret=004a35  args=2"), Some(0));
        assert_eq!(parse_frame_idx("#12  fn@…"), Some(12));
        assert_eq!(parse_frame_idx("(no frames)"), None);
    }

    #[test]
    fn toggle_frame_expands_then_collapses() {
        let mut p = DebugPanelState::new(0x1000);
        p.snapshot.stack = vec!["#0  fn@004a00  ret=004a35  args=2".to_string()];
        p.toggle_frame(0, &MockDbg);
        assert!(p.expanded_frames.contains(&0));
        assert!(p.snapshot.frame_details.contains_key(&0));
        p.toggle_frame(0, &MockDbg);
        assert!(!p.expanded_frames.contains(&0));
        assert!(!p.snapshot.frame_details.contains_key(&0));
    }

    #[test]
    fn stack_click_at_resolves_a_frame_row_and_ignores_a_detail_row() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        // focus/tab default: window 2 tab 0 = Call Stack.
        p.snapshot.stack = vec!["#0  fn@004a00  ret=004a35  args=2".to_string()];
        p.expanded_frames.insert(0);
        p.snapshot.frame_details.insert(0, vec!["local0 = 0x0001  (1)".to_string()]);
        let [_, _, bot] = window_rects(region);
        let content = Rect::new(bot.x + 1, bot.y + 1, bot.width.saturating_sub(2), bot.height.saturating_sub(2));
        // Row 0 is the frame row → toggle target.
        assert_eq!(stack_click_at(region, &p, content.x, content.y), Some(0));
        // Row 1 is the detail row → not a toggle target.
        assert_eq!(stack_click_at(region, &p, content.x, content.y + 1), None);
    }

    #[test]
    fn stack_click_at_ignores_clicks_when_call_stack_is_not_the_active_tab() {
        let region = Rect::new(0, 0, 61, 40);
        let mut p = DebugPanelState::new(0x1000);
        p.tab[2] = 1; // Stack (eval), not Call Stack
        p.snapshot.stack = vec!["#0  fn@004a00  ret=004a35  args=2".to_string()];
        let [_, _, bot] = window_rects(region);
        let content = Rect::new(bot.x + 1, bot.y + 1, bot.width.saturating_sub(2), bot.height.saturating_sub(2));
        assert_eq!(stack_click_at(region, &p, content.x, content.y), None);
    }

    // ── Navigation primitives (goto_memory / goto_object) ───────────────────

    #[test]
    fn goto_memory_focuses_the_memory_tab_and_jumps_mem_addr() {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = 0;
        p.goto_memory(0x300, &MockDbg);
        assert_eq!(p.focus, 2);
        assert_eq!(p.tab[2], 2);
        assert_eq!(p.mem_addr, 0x300);
        assert!(p.snapshot.memory[0].starts_with(&format!("{:06x}", 0x300)));
    }

    #[test]
    fn memory_input_dereferences_a_global_token_to_its_value_as_an_address() {
        // MockDbg::var_value returns 0x1000 + var*0x10. `g00` = global index 0
        // = variable 16 → 0x1000 + 16*0x10 = 0x1100, used as the jump address.
        let mut p = memory_focused_panel();
        p.handle_key(KeyCode::Char(':'), &MockDbg);
        for c in "g00".chars() { p.handle_key(KeyCode::Char(c), &MockDbg); }
        p.handle_key(KeyCode::Enter, &MockDbg);
        assert_eq!(p.mem_addr, 0x1100, "jumps to the value held in the variable");
        assert!(p.mem_input.is_none(), "input closes on Enter");
    }

    #[test]
    fn parse_var_token_maps_the_variable_families() {
        assert_eq!(parse_var_token("sp"), Some(0));
        assert_eq!(parse_var_token("local0"), Some(1));
        assert_eq!(parse_var_token("local10"), Some(11));
        assert_eq!(parse_var_token("g00"), Some(16));
        assert_eq!(parse_var_token("g44"), Some(0x44 + 16)); // NN is hex, matches g{:02x}
        assert_eq!(parse_var_token("local15"), None, "only 0..=14 locals");
        assert_eq!(parse_var_token("1234"), None, "a bare hex address is not a var");
    }

    #[test]
    fn goto_memory_aligns_an_unaligned_jump_down_to_the_row_grid() {
        let mut p = DebugPanelState::new(0x1000);
        p.goto_memory(0x30b, &MockDbg);
        assert_eq!(p.mem_addr, 0x300, "jump aligns down to the 16-byte row boundary");
    }

    #[test]
    fn goto_object_focuses_objects_tab_expands_and_scrolls_to_the_object() {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = 0;
        p.snapshot.objects = vec!["[1] lamp".to_string(), "[2] rock".to_string()];
        p.goto_object(2, &MockDbg);
        let (ow, _) = locate_section(Section::Objects);
        assert_eq!(p.focus, ow);
        assert_eq!(p.active_section(ow), Section::Objects);
        assert!(p.expanded_objects.contains(&2));
        assert!(p.snapshot.object_details.contains_key(&2));
        assert_eq!(p.scroll[ow], 1, "object [2]'s display row is index 1");
    }

    // ── Variable hover tooltips ─────────────────────────────────────────────

    /// Build a Disasm-focused panel with a single known line at the PC divider's
    /// row, using the same Rect the `clickable_at` tests use. Returns the panel,
    /// the content rect, and the row_y of the disasm line (row 1, under the
    /// divider at row 0).
    fn hover_panel(line: &str) -> (DebugPanelState, Rect, Rect, u16) {
        // Wide enough that every operand of the test lines fits the left window.
        let region = Rect::new(0, 0, 120, 40);
        let mut p = DebugPanelState::new(0x1000);
        p.pc = 0x1000;
        p.snapshot.disasm = vec![line.to_string(), "001004  add".to_string()];
        let [left, ..] = window_rects(region);
        let content = Rect::new(left.x + 1, left.y + 1, left.width.saturating_sub(2), left.height.saturating_sub(2));
        let row_y = content.y + 1; // row 0 is the PC divider; row 1 is line 0
        (p, region, content, row_y)
    }

    #[test]
    fn hover_var_at_resolves_global_local_and_stack_operands() {
        let line = "001000  loadw g0f, local0 -> sp";
        let (p, region, content, row_y) = hover_panel(line);
        // g0f (global index 0x0f) → var 0x0f + 16 = 0x1f (31).
        let g_col = content.x + 1 + line.find("g0f").unwrap() as u16;
        assert_eq!(hover_var_at(region, &p, g_col, row_y), Some((0x1f, g_col, row_y)));
        // local0 → var 1.
        let l_col = content.x + 1 + line.find("local0").unwrap() as u16;
        assert_eq!(hover_var_at(region, &p, l_col, row_y), Some((1, l_col, row_y)));
        // sp → var 0.
        let sp_col = content.x + 1 + line.rfind("sp").unwrap() as u16;
        assert_eq!(hover_var_at(region, &p, sp_col, row_y), Some((0, sp_col, row_y)));
    }

    #[test]
    fn hover_var_at_returns_none_over_non_variable_tokens() {
        let line = "001000  storew @0x001234, obj#5 -> #01";
        let (p, region, content, row_y) = hover_panel(line);
        let mem_col = content.x + 1 + line.find("@0x").unwrap() as u16;
        assert_eq!(hover_var_at(region, &p, mem_col, row_y), None);
        let obj_col = content.x + 1 + line.find("obj#").unwrap() as u16;
        assert_eq!(hover_var_at(region, &p, obj_col, row_y), None);
        let const_col = content.x + 1 + line.find("#01").unwrap() as u16;
        assert_eq!(hover_var_at(region, &p, const_col, row_y), None);
    }

    #[test]
    fn hover_help_at_resolves_the_mnemonic_token_only() {
        let line = "001000  loadw g0f, local0 -> sp";
        let (p, region, content, row_y) = hover_panel(line);
        // Over the mnemonic (column 8) → the instruction address + anchor col.
        let mcol = content.x + 1 + 8;
        assert_eq!(hover_help_at(region, &p, mcol, row_y), Some((0x1000, mcol, row_y)));
        // Over the address prefix → None.
        assert_eq!(hover_help_at(region, &p, content.x + 1 + 2, row_y), None);
        // Over an operand → None (opcode help is mnemonic-only; operands use hover_var_at).
        let op_col = content.x + 1 + line.find("g0f").unwrap() as u16;
        assert_eq!(hover_help_at(region, &p, op_col, row_y), None);
    }

    #[test]
    fn hover_help_at_ignores_header_data_and_raw_lines() {
        // Address 001000 matches hover_panel's pc so the divider lands at row 0
        // and row_y hits this line (row 1).
        for line in [
            "001000  ; routine, 1 local",   // Full/Basic header marker
            "001000  .byte 01 02 03",       // data row
            "001000: 88 1b82 01   1OP:0x08", // Raw line (colon prefix, hex token)
        ] {
            let (p, region, content, row_y) = hover_panel(line);
            let col = content.x + 1 + 8; // column 8 (where a mnemonic would be)
            assert_eq!(hover_help_at(region, &p, col, row_y), None, "line {line:?}");
        }
    }

    #[test]
    fn hover_var_at_returns_none_outside_window_0() {
        let line = "001000  loadw g0f -> sp";
        let (p, region, _content, row_y) = hover_panel(line);
        // Far right (window 1/2 territory) is not the Disasm window.
        let [_, top, _] = window_rects(region);
        assert_eq!(hover_var_at(region, &p, top.x + 3, row_y), None);
    }

    #[test]
    fn hover_tip_for_var_formats_hex_signed_and_na() {
        // var 0x1f = global index 0x0f → label "g0f"; value 0x1234 = 4660.
        let tip = HoverTip::for_var(0x1f, Some(0x1234), 5, 7);
        assert_eq!(tip.lines, vec!["g0f = 0x1234".to_string(), "4660 / 4660".to_string()]);
        assert_eq!((tip.col, tip.row), (5, 7));
        // Signed rendering: 0xffff → -1.
        let neg = HoverTip::for_var(1, Some(0xffff), 0, 0);
        assert_eq!(neg.lines, vec!["local0 = 0xffff".to_string(), "65535 / -1".to_string()]);
        // Unavailable value.
        let na = HoverTip::for_var(0, None, 0, 0);
        assert_eq!(na.lines, vec!["sp = (n/a)".to_string()]);
    }

    #[test]
    fn clickable_spans_drops_variable_targets_in_disasm() {
        let line = "004a2f  loadw @0x001234, g0f, local2, sp  ?0x004b00";
        let spans = clickable_spans(Section::Disasm, line);
        let targets: Vec<ClickTarget> = spans.iter().map(|(_, t)| *t).collect();
        // Only memory/object/code survive; g0f/local2/sp are gone.
        assert_eq!(targets, vec![ClickTarget::Memory(0x1234), ClickTarget::Code(0x4b00)]);
        assert!(!targets.iter().any(|t| matches!(t,
            ClickTarget::Global(_) | ClickTarget::Local(_) | ClickTarget::Stack)));
    }

    #[test]
    fn debug_point_maps_and_clamps_to_window_content() {
        // 80x24: window 0 content = (1,1,38,22); window 1 content = (41,1,38,10).
        let region = Rect::new(0, 0, 80, 24);
        assert_eq!(debug_point_at(region, 5, 3),
            Some((0, crate::clipboard::Point { row: 2, col: 4 })));
        assert_eq!(debug_point_at(region, 45, 2),
            Some((1, crate::clipboard::Point { row: 1, col: 4 })));
        // A click on the border/gap is not over any content area.
        assert_eq!(debug_point_at(region, 0, 0), None);
        // A drag past window 1's far corner clings to its last content cell.
        let c1 = window_content(region, 1).unwrap();
        assert_eq!(debug_point_clamped(region, 1, 200, 200),
            crate::clipboard::Point { row: (c1.height - 1) as usize, col: c1.width - 1 });
    }
}
