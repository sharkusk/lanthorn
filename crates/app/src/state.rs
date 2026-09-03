use std::time::{Duration, Instant};

// ── Hint system state ─────────────────────────────────────────────────────────

/// The source driving the open Hints panel.
///
/// `Zcode` wraps a second Z-machine session running the companion Invisiclues
/// (or any hint `.z5`) file.  The enum is a seam for future sources (e.g. UHS).
pub enum HintSource {
    /// A companion Invisiclues / hint program run as a second Z-machine session.
    Zcode(crate::session::GameSession),
}

// GameSession does not implement Debug, so we implement Debug manually for HintSource.
impl std::fmt::Debug for HintSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HintSource::Zcode(_) => write!(f, "HintSource::Zcode(<GameSession>)"),
        }
    }
}

/// Transient state for the Hints panel modal.
///
/// Held in `AppState.hints: Option<HintSession>` — `Some` while the panel is
/// open, `None` when closed.  The session is NOT persisted into the `.lanthorn`
/// archive; only the per-IFID hint-file association is saved (Task A).
pub struct HintSession {
    /// The active hint source (currently always `Zcode`).
    pub source: HintSource,
    /// The hint program's own output (its scrollback transcript).
    pub transcript: Vec<String>,
    /// Scroll offset within the hint transcript (logical target).
    pub scroll: u16,
    /// Transcript index to truncate back to when the companion clears its
    /// screen for a menu redraw (menu-reprint collapse anchor). `None` until
    /// the first screen clear establishes an anchor.
    pub clear_anchor: Option<usize>,
    /// Smooth-scroll animation easing the *displayed* offset toward `scroll`.
    /// `None` when settled or animation is disabled (the instant path).
    pub scroll_anim: Option<ScrollAnim>,
    /// The hint panel's own input line (typed by the player).
    pub input: String,
    /// Dialog title, e.g. "Invisiclues: Zork I".
    pub label: String,
    /// When true, show the suggestion "This game has its own hints — type HINT".
    pub builtin_hint: bool,
}

impl HintSession {
    /// Scroll the transcript by `delta` rows, clamped to `[0, max]`, easing the
    /// displayed offset per the `[animation]` config (instant when disabled).
    ///
    /// `delta > 0` scrolls toward older content (matching the story transcript's
    /// wheel-up direction); `max` is the last-rendered maximum scroll offset.
    pub fn scroll_by(&mut self, delta: i32, max: u16, anim: &crate::config::AnimationConfig) {
        let from = self.effective_scroll() as usize;
        let next = (self.scroll as i32 + delta).clamp(0, max as i32) as u16;
        self.scroll = next;
        self.scroll_anim = ScrollAnim::to(from, next as usize, anim);
    }

    /// The displayed scroll offset this frame: the eased value while animating,
    /// else the logical target.
    pub fn effective_scroll(&self) -> u16 {
        self.scroll_anim
            .as_ref()
            .map(|a| a.current().round() as u16)
            .unwrap_or(self.scroll)
    }

    /// Drop a completed scroll animation (called from the run loop). Returns
    /// `true` iff a running animation was cleared this call, so the loop can
    /// force the one redraw that paints the settled offset. (SQ-0305)
    pub fn finalize_scroll_if_done(&mut self) -> bool {
        if self.scroll_anim.as_ref().is_some_and(|a| a.done()) {
            self.scroll_anim = None;
            true
        } else {
            false
        }
    }

    /// True while the displayed scroll offset is still easing.
    pub fn has_active_animation(&self) -> bool {
        self.scroll_anim.as_ref().is_some_and(|a| !a.done())
    }

    /// Fold one companion-VM turn's output into the panel transcript, mirroring the
    /// main session's game-driven handling: on a screen clear (menu redraw), collapse
    /// the previous reprint by truncating back to the anchor, then re-anchor; then
    /// append this turn's lines.
    ///
    /// A turn with no lower-window output (e.g. an upper-window menu keystroke that
    /// only moves the highlight) adds NOTHING — `"".split('\n')` yields one empty
    /// string, so a naive push would drop a blank line into the clue window on every
    /// keypress. Scroll only snaps to the newest content when something actually
    /// changed, so paging up to reread clues survives menu navigation.
    pub fn apply_turn(&mut self, result: &crate::session::TurnResult) {
        let mut changed = false;
        if result.erase_lower {
            let anchor = self.clear_anchor.unwrap_or(self.transcript.len());
            self.transcript.truncate(anchor);
            self.clear_anchor = Some(self.transcript.len());
            changed = true;
        }
        if !result.transcript.is_empty() {
            for line in result.transcript.split('\n') {
                self.transcript.push(line.to_owned());
            }
            changed = true;
        }
        if changed {
            self.scroll = 0;
            self.scroll_anim = None;
        }
    }
}

// GameSession does not implement Debug, so we implement Debug manually for HintSession.
impl std::fmt::Debug for HintSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HintSession")
            .field("source", &self.source)
            .field("transcript", &self.transcript)
            .field("scroll", &self.scroll)
            .field("input", &self.input)
            .field("label", &self.label)
            .field("builtin_hint", &self.builtin_hint)
            .finish()
    }
}

// ── Room dock ─────────────────────────────────────────────────────────────────

/// Which body the room dock is showing (SQ-0692). One dock, two views — the
/// story-facing Info body and the layout Diagnostics body, for the SAME room.
///
/// Replaces `RoomPanelMode`, which chose between two floating corner dialogs;
/// the dock is not a dialog, so the mode is now just which body it draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoomDockView {
    /// Story/game info body (notes, the exit card, objects for the current room).
    #[default]
    Info,
    /// Layout diagnostics body (id, layer, grid pos, edges, discovery method).
    Diagnostics,
}

impl RoomDockView {
    /// The other view — what `/toggle-inspector` flips an open dock to.
    pub fn flipped(self) -> Self {
        match self {
            RoomDockView::Info => RoomDockView::Diagnostics,
            RoomDockView::Diagnostics => RoomDockView::Info,
        }
    }
}

// ── Side panel cycle (SQ-1237) ──────────────────────────────────────────────

/// Which of the two mutually-exclusive panels the story pane's border control
/// summons is open: the command panel, the inventory panel, or neither.
///
/// The two panels never show at once — opening one closes the other — so one
/// value, not two independent booleans, describes the pair. `/cycle-panel`
/// (and a click on the border control) walks [`SidePanel::next`]; the value is
/// what the per-game sidecar persists (`styles::PerGameConfig::panel`), the
/// same single mechanism the command band's on/off state already used before
/// the inventory panel joined the cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidePanel {
    Command,
    Inventory,
    None,
}

impl SidePanel {
    /// The next state in the cycle: Command → Inventory → None → Command.
    pub fn next(self) -> SidePanel {
        match self {
            SidePanel::Command => SidePanel::Inventory,
            SidePanel::Inventory => SidePanel::None,
            SidePanel::None => SidePanel::Command,
        }
    }

    /// The sidecar's own spelling, read by [`SidePanel::from_key`].
    pub fn key(self) -> &'static str {
        match self {
            SidePanel::Command => "command",
            SidePanel::Inventory => "inventory",
            SidePanel::None => "none",
        }
    }

    /// Parse the sidecar's spelling. An unrecognised token is `None` — the same
    /// "a corrupt sidecar inherits the default" rule every other per-game key
    /// follows (`styles::PerGameConfig::read`).
    pub fn from_key(s: &str) -> Option<SidePanel> {
        match s {
            "command" => Some(SidePanel::Command),
            "inventory" => Some(SidePanel::Inventory),
            "none" => Some(SidePanel::None),
            _ => Option::None,
        }
    }
}

// ── Drag-pan state ────────────────────────────────────────────────────────────

/// Middle-button drag-pan accumulator state.
#[derive(Debug, Clone, Copy)]
pub struct DragState {
    /// Terminal cell position of the last drag event.
    pub last: (u16, u16),
    /// Sub-cell accumulator for x (in terminal columns).
    pub acc_x: i32,
    /// Sub-cell accumulator for y (in terminal rows).
    pub acc_y: i32,
}

// ── Command band state ────────────────────────────────────────────────────────

use crate::render::command_band::{
    VerbEntry, VerbSource, VerbTable, BAND_COLS, COL_CARRIED, COL_HERE, COL_SECOND, COL_VERB,
};
use crossterm::event::KeyCode;

/// Which grammatical slot a picked token fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandSlot {
    Verb,
    Object,
    Second,
}

/// One token the player has picked, in pick order — so Backspace is a pop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandPick {
    pub slot: BandSlot,
    pub text: String,
}

/// The token still under construction at the end of `input`: its trailing
/// whitespace-delimited word, or `""` when the line ends in whitespace (the
/// player finished that word, so the band should be looking at the NEXT slot
/// rather than still matching the last one).
pub fn band_typed_token(input: &str) -> &str {
    if input.ends_with(char::is_whitespace) {
        return "";
    }
    input.split_whitespace().next_back().unwrap_or("")
}

/// How well `item` matches the word being typed (already lowercased):
/// `0` = the item starts with it, `1` = one of the item's words starts with it
/// (so `do` finds `iron door`), `2` = it appears anywhere. `None` = no match at
/// all, which is what makes Tab a no-op rather than a wrong guess.
fn band_match_rank(item: &str, token_lower: &str) -> Option<u8> {
    let lower = item.to_lowercase();
    if lower.starts_with(token_lower) {
        return Some(0);
    }
    if lower.split_whitespace().any(|w| w.starts_with(token_lower)) {
        return Some(1);
    }
    if lower.contains(token_lower) {
        return Some(2);
    }
    None
}

/// One parse of the real input line into the band's grammar state.
struct ParsedPhrase {
    picks: Vec<BandPick>,
    /// The columns the grammar expects the NEXT word to come from — where the
    /// nearest-match highlight looks. Empty when nothing more is expected (a
    /// solo verb).
    expected: Vec<usize>,
}

/// The command band's whole state. `None` in `OverlayState.command_band` means
/// the band is closed.
///
/// `picks` is the grammar's bookkeeping — which slot has what, for arity and
/// column reachability — but the composed phrase ITSELF lives on the real
/// story input line (`state.input`), not a band-local phrase row (retired
/// 2026-08-05, SQ-0667): every successful pick mirrors onto `state.input`'s
/// tail (see `input::sync_band_phrase_to_input`, called from `apply_action`,
/// which is why that mirroring isn't done here — this type has no sibling
/// field access to `state.input`).
///
/// **SQ-0676 (2026-08-05) inverted the focus model.** The band never owns the
/// keyboard now: typing always reaches the story prompt, and `picks` is
/// re-derived from what is typed there ([`CommandBandState::sync_from_input`])
/// rather than only from clicks. There is therefore no type-to-filter state
/// and no story-focus flag left.
///
/// **SQ-0677 (2026-08-05) replaced the SQ-0676 arrow scheme.** A current
/// column (`focus`) always exists while the band is open; `Tab`/`Shift-Tab`
/// move it across the reachable columns, `↑`/`↓` highlight a row within it
/// (`row_sel`), and typing drives a passive nearest-match highlight in that
/// SAME column ([`CommandBandState::nearest_match`], now scoped to `focus`
/// rather than hunting every grammatically-live column). `Tab` unifies the
/// two: with a row highlighted (explicit or typed) it picks that row and
/// advances, exactly like a click; with nothing highlighted it is pure
/// column movement. The quick block (rose + flowing words, and the flat-row
/// fallback) lost its keyboard entirely in the same amendment — it is
/// mouse-click-only now, with `quick_hover` its one transient highlight.
#[derive(Debug, Clone, Default)]
pub struct CommandBandState {
    /// The verb table in force: the running story's own grammar, the built-in
    /// fallback, or the config's replacement — with `extra_verbs` layered on
    /// whichever it is.
    pub verbs: Vec<VerbEntry>,
    /// Where [`Self::verbs`] came from, so the column can label itself when it
    /// is NOT the story's own grammar (SQ-1111).
    pub verb_source: VerbSource,
    /// True once this open has asked the engine for the story's grammar —
    /// whatever the answer. The tables are static, so one read per open is all
    /// there is; without this a story with no readable grammar would re-ask on
    /// every tick forever. (`render::command_band::refresh_verbs`.)
    pub verbs_read: bool,
    /// The one-click quick-action row.
    pub quick: Vec<String>,
    /// Tokens picked so far, in order.
    pub picks: Vec<BandPick>,
    /// The column the band currently points at. `Tab`/`Shift-Tab` step it
    /// across [`Self::focus_stops`] ([`Self::step_column`]); typing re-points
    /// it only when the grammar no longer expects the current column
    /// ([`Self::sync_from_input`]) — so a manual Tab survives further typing
    /// in the same slot.
    pub focus: usize,
    /// The row explicitly highlighted within `focus` by `↑`/`↓`
    /// ([`Self::step_row`]) — distinct from the passive nearest-match
    /// highlight typing drives. `None` when nothing is explicitly selected,
    /// in which case [`Self::highlighted_row`] falls back to the typed match.
    /// Cleared by any change to the typed line and by moving `focus`.
    pub row_sel: Option<usize>,
    /// The quick word (a rose cell, a flowing word, or a flat-row entry) the
    /// mouse currently sits over, as an index into `quick` — `None` off the
    /// block. Purely a hover cosmetic (SQ-0677): quick is mouse-click-only,
    /// so hover is its only transient highlight and plays no role in what a
    /// click fires.
    pub quick_hover: Option<usize>,
    /// Per-column selection + animated scroll.
    pub scroll: [crate::list_scroll::ListScroll; BAND_COLS],
    /// Each column's visible list height this frame, published by the render
    /// (`draw_command_band`) for the wheel to scroll within — a `Cell` for the
    /// same reason `transcript_geom` is one: a draw only ever sees `&AppState`.
    /// A column the frame did not draw (mid-slide, or the narrow single-column
    /// fallback) reads 0, and `ListScroll::scroll_by` correctly refuses to
    /// scroll a window whose size it does not know. (SQ-0832)
    ///
    /// PER COLUMN, and deliberately not the shared `modal_list_viewport`: the
    /// VERB column reclaims its header row as a list row (see `draw_column`)
    /// and so is one row taller than its neighbours, and a modal drawn later
    /// in the same frame overwrites that one global slot while the band is
    /// still on screen underneath it.
    pub col_viewport: std::cell::Cell<[usize; BAND_COLS]>,
    /// Objects in the current room, refreshed from the engine whenever the VM
    /// has run (see [`Self::objects_epoch`]).
    /// The object tree's answer, and empty for an engine that has none.
    pub here: Vec<String>,
    /// Objects the player carries, refreshed alongside [`Self::here`].
    pub carried: Vec<String>,
    /// The [`AppState::turn_epoch`] the object columns were last refreshed at,
    /// `None` for a fresh open (SQ-1175). Objects only move when the VM runs,
    /// and every turn finisher (and a host restore) bumps the epoch — so a
    /// matching epoch means `refresh_objects` has nothing new to read, and the
    /// ~20 Hz loop tick skips the whole object-tree walk (on v4+ the location
    /// detection behind it Z-decodes every short name in the game).
    pub objects_epoch: Option<u64>,
    /// The nouns the story has PRINTED, most recently first — the second block
    /// of the noun columns, under whatever the object tree could say (SQ-1135).
    ///
    /// Drawn dimmed (`band.item:seen`), because it is a weaker claim: the story
    /// knows the word, which is not the same as the thing being here. Every
    /// engine gets this block; before SQ-1135 only an engine with NO object tree
    /// did, so the Z-machine — the one that can say most — offered least.
    pub here_seen: Vec<String>,
    /// What the WHAT column's rows actually are, which is what its header may
    /// claim. Recomputed with the lists themselves; see [`HereSource`].
    pub here_source: HereSource,
}

/// Where the WHAT column's rows came from, and therefore what its header is
/// allowed to say (SQ-1135).
///
/// The rule is that the label must be true of the WHOLE column. With scope rows
/// and printed-word rows in one column, neither "here" nor "seen" is true of all
/// of it, and the honest header is the bare noun.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HereSource {
    /// Every row is something the object tree says is here.
    #[default]
    Scope,
    /// Every row is a word the story has printed (an engine with no object
    /// tree, or a story whose tree says nothing is here).
    Seen,
    /// Both, in one column: scope first, then the printed words.
    Mixed,
}

/// One row of a noun column, and how strong a claim it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandRow {
    /// The word as the column shows it.
    pub text: String,
    /// True for a row from [`CommandBandState::here_seen`] — the story knows the
    /// word, which is not a promise that the thing is here.
    pub seen: bool,
}

impl CommandBandState {
    pub fn new(verbs: VerbTable, quick: Vec<String>) -> Self {
        CommandBandState {
            verbs: verbs.entries,
            verb_source: verbs.source,
            quick,
            ..Default::default()
        }
    }

    // ── Phrase ───────────────────────────────────────────────────────────────

    fn slot_text(&self, slot: BandSlot) -> Option<&str> {
        self.picks.iter().find(|p| p.slot == slot).map(|p| p.text.as_str())
    }

    /// The table entry for `word`, matched case-insensitively (the player types
    /// the prompt now, and `Take` is the same verb as `take`).
    pub fn verb_by_word(&self, word: &str) -> Option<&VerbEntry> {
        self.verbs.iter().find(|v| v.word.eq_ignore_ascii_case(word))
    }

    /// The picked verb's table entry (grammar for everything downstream).
    pub fn verb_entry(&self) -> Option<&VerbEntry> {
        let w = self.slot_text(BandSlot::Verb)?;
        self.verb_by_word(w)
    }

    /// The picked verb's sentence shapes, if a verb is picked.
    ///
    /// A picked word that is NOT in the table has no known shape — an empty
    /// list, which [`VerbEntry::accepts`] and [`VerbEntry::max_nouns`] both read
    /// as "complete on its own". That can only come from the quick row (every
    /// column pick is drawn from the table), and a quick action IS the whole
    /// command (`n`, `again`), so it is complete as it stands. Without this, a
    /// quick row holding `n` (which the story's grammar spells `north`) could
    /// never arm.
    fn shape(&self) -> Option<&VerbEntry> {
        let word = self.slot_text(BandSlot::Verb)?;
        self.verb_by_word(word)
    }

    /// How many objects the picked verb can take at most — 0 when it only ever
    /// stands alone, and 0 for a picked word with no table entry, which is what
    /// closes the object columns behind a quick action.
    fn max_nouns(&self) -> usize {
        self.shape().map_or(0, VerbEntry::max_nouns)
    }

    /// The word joining the two objects of the picked verb — `unlock … WITH …`.
    /// The first of the verb's own joiners; see [`VerbEntry::joiners`] for the
    /// alternation that lives behind it.
    pub fn prep(&self) -> Option<&str> {
        self.verb_entry().and_then(|v| v.joiner())
    }

    /// Materialize the phrase as the plain text the game will parse. Multi-word
    /// object names go in as-is: `unlock iron door with brass key` is exactly
    /// what a player would type, so nothing is quoted or escaped.
    pub fn phrase_text(&self) -> String {
        let mut out: Vec<&str> = Vec::new();
        if let Some(v) = self.slot_text(BandSlot::Verb) {
            out.push(v);
        }
        if let Some(o) = self.slot_text(BandSlot::Object) {
            out.push(o);
        }
        if let Some(s) = self.slot_text(BandSlot::Second) {
            if let Some(p) = self.prep() {
                out.push(p);
            }
            out.push(s);
        }
        out.join(" ")
    }

    /// Whether the phrase is grammatically complete — i.e. whether the text
    /// now sitting on the real story input line is a valid command on its
    /// own (Enter there sends it, exactly like anything typed by hand).
    pub fn complete(&self) -> bool {
        if self.slot_text(BandSlot::Verb).is_none() {
            return false;
        }
        let mut filled = 0;
        if self.slot_text(BandSlot::Object).is_some() {
            filled += 1;
        }
        if self.slot_text(BandSlot::Second).is_some() {
            filled += 1;
        }
        // ANY of the verb's lines taking exactly this many objects settles it —
        // which is the alternation the old single-arity model could not hold:
        // a story that accepts both `take noun` and `take noun from noun` arms
        // at one object AND at two, instead of having to pick a shape.
        match self.shape() {
            Some(v) => v.accepts(filled),
            None => filled == 0,
        }
    }

    // ── Columns ──────────────────────────────────────────────────────────────

    /// Whether `col` can be picked from yet, given what the phrase already has.
    /// Columns to the right of the active one stay unreachable until the
    /// grammar opens them.
    pub fn col_reachable(&self, col: usize) -> bool {
        match col {
            COL_VERB => true,
            COL_HERE | COL_CARRIED => self.max_nouns() >= 1,
            COL_SECOND => self.max_nouns() >= 2 && self.slot_text(BandSlot::Object).is_some(),
            _ => false,
        }
    }

    /// Whether `col`'s grammatical slot is already filled.
    pub fn col_filled(&self, col: usize) -> bool {
        match col {
            COL_VERB => self.slot_text(BandSlot::Verb).is_some(),
            COL_HERE | COL_CARRIED => self.slot_text(BandSlot::Object).is_some(),
            COL_SECOND => self.slot_text(BandSlot::Second).is_some(),
            _ => false,
        }
    }

    /// Which slot a pick in `col` fills.
    pub fn col_slot(col: usize) -> BandSlot {
        match col {
            COL_VERB => BandSlot::Verb,
            COL_SECOND => BandSlot::Second,
            _ => BandSlot::Object,
        }
    }

    /// The column's header text.
    pub fn column_label(&self, col: usize) -> String {
        match col {
            // Unlabelled when the words ARE the story's own grammar — see
            // `VerbSource::column_label`, and `draw_column` for the row that
            // header only costs when there is something to admit.
            COL_VERB => self.verb_source.column_label().unwrap_or("VERB").to_string(),
            // The label has to be true of the WHOLE column (SQ-1135): with the
            // object tree's rows and the story's printed words stacked in one
            // list, neither qualifier is, and the bare noun is what is left.
            COL_HERE => match self.here_source {
                HereSource::Scope => "WHAT — here".to_string(),
                HereSource::Seen => "WHAT — seen".to_string(),
                HereSource::Mixed => "WHAT".to_string(),
            },
            COL_CARRIED => "WHAT — carried".to_string(),
            COL_SECOND => match self.prep() {
                Some(p) => format!("{}…", p.to_uppercase()),
                None => "WITH…".to_string(),
            },
            _ => String::new(),
        }
    }

    /// The column's items before filtering.
    ///
    /// `COL_VERB` excludes a quick word only when the quick row's one click
    /// really is the whole of what that word can do (SQ-0667, narrowed by
    /// SQ-1128).
    ///
    /// A quick pick fires the BARE word at once. For `n`, `wait`, `again` and
    /// `inventory` that is the entire sentence and a second row for it is pure
    /// duplication. For a verb that takes an object it is not: `look at the
    /// mirror`, `enter the boat`, `bow to the king` and Deadline's `wait for
    /// the coroner` are all sentences the button cannot reach, and the column
    /// is the only place they can be started from. So the test is
    /// [`VerbEntry::takes_object`] — the STORY's answer, read off its raw
    /// syntax lines rather than off the shapes the band can compose, because
    /// every one of Zork I's `look` lines is a shape the band drops.
    ///
    /// Measured over the corpus, that returns `look` (49 of 60 stories with a
    /// readable grammar), `enter` (49) and `exit` (47) — excluded as
    /// direction-equivalents of `in`/`out` — Deadline's `wait` (29), and `bow`
    /// (12). `inventory` and `again` take an object in no story in the corpus
    /// and stay out of the column everywhere.
    ///
    /// The exclusion still follows `self.quick` — the *effective* list (the
    /// user's configured `quick` when set, else the built-in row) — so removing
    /// a word from a custom `quick` puts it back here. Direction words still
    /// compare by the direction they name, not by spelling, so the quick row's
    /// `n` excludes the table's `north`.
    ///
    /// That comparison is the band's OWN ([`crate::render::command_band::compass_spelling`],
    /// SQ-1130), not `mapper::direction::parse_direction`. The parser answers a
    /// question about movement, where `bow` is north because a ship's bow points
    /// forward; this one asks whether two words are the same word, where `bow` is
    /// a verb twelve stories take an object with. The rule above already returns
    /// `bow` on its own merits — the reuse is what is fixed here, before the next
    /// story spells a verb `port`.
    pub fn items(&self, col: usize) -> Vec<String> {
        self.rows(col).into_iter().map(|r| r.text).collect()
    }

    /// [`Self::items`] with each row's provenance still attached — the list the
    /// renderer draws, since a printed-word row is dimmed and a scope row is not
    /// (SQ-1135).
    ///
    /// **One construction, not two.** `items` is this list with the flags
    /// dropped, so the order the renderer dims by and the order every picker
    /// indexes into cannot drift — the hand-maintained cross-file invariant
    /// CLAUDE.md's refactoring policy names, which here would show up as the
    /// wrong rows greyed out.
    pub fn rows(&self, col: usize) -> Vec<BandRow> {
        use crate::render::command_band::compass_spelling;
        let same_word = |q: &str, w: &str| {
            q.eq_ignore_ascii_case(w)
                || matches!(
                    (compass_spelling(q), compass_spelling(w)),
                    (Some(a), Some(b)) if a == b
                )
        };
        // Append `src`'s words, skipping anything already listed under any
        // spelling — one word, one row, whichever block first claimed it.
        let push = |out: &mut Vec<BandRow>, src: &[String], seen: bool| {
            for w in src {
                if !out.iter().any(|r| r.text.eq_ignore_ascii_case(w)) {
                    out.push(BandRow { text: w.clone(), seen });
                }
            }
        };
        match col {
            COL_VERB => self
                .verbs
                .iter()
                .filter(|v| {
                    v.takes_object || !self.quick.iter().any(|q| same_word(q, &v.word))
                })
                .map(|v| BandRow { text: v.word.clone(), seen: false })
                .collect(),
            // Here first, then what the story has merely PRINTED (SQ-1135). One
            // column and not two: the second block is the same question asked
            // more weakly, and a player scanning for a noun should not have to
            // know which of two lists to scan.
            COL_HERE => {
                let mut out = Vec::new();
                push(&mut out, &self.here, false);
                push(&mut out, &self.here_seen, true);
                out
            }
            COL_CARRIED => {
                let mut out = Vec::new();
                push(&mut out, &self.carried, false);
                out
            }
            COL_SECOND => {
                // Carried first (the usual instrument), then anything here that
                // isn't already listed, then the printed words — the same noun
                // slot as WHAT, so the same three tiers in the same order.
                let mut out = Vec::new();
                push(&mut out, &self.carried, false);
                push(&mut out, &self.here, false);
                push(&mut out, &self.here_seen, true);
                out
            }
            _ => Vec::new(),
        }
    }

    // ── Reading the typed line (SQ-0676) ─────────────────────────────────────

    /// Parse the real input line into the band's grammar state.
    ///
    /// The phrase is anchored on the FIRST typed token that is a table verb;
    /// anything before it is free text the player wrote (`well, take mailbox`)
    /// and is deliberately ignored rather than mistaken for a verb. Everything
    /// after it fills the object slot — split at the pair verb's preposition
    /// when one has been typed — which is also why the parse never counts
    /// tokens: object names are routinely multi-word (`iron door`).
    ///
    /// The word still under construction counts as a token here, unlike in
    /// [`Self::nearest_match`]: a bare `take` with no trailing space is a
    /// chosen verb (it is exactly what a click on the VERB column leaves on the
    /// prompt), so the object columns must open on it rather than waiting for a
    /// space that a mouse-only player would never type.
    fn parse_phrase(&self, input: &str) -> ParsedPhrase {
        let toks: Vec<&str> = input.split_whitespace().collect();
        let Some(vi) = toks.iter().position(|t| self.verb_by_word(t).is_some()) else {
            return ParsedPhrase { picks: Vec::new(), expected: vec![COL_VERB] };
        };
        let entry = self.verb_by_word(toks[vi]).expect("just matched");
        let (max_nouns, joiners) = (entry.max_nouns(), entry.joiners());
        // Store the TABLE's spelling, so downstream lookups (`prep`,
        // `max_nouns`) and `phrase_text` are canonical regardless of how it was
        // typed.
        let mut picks = vec![BandPick { slot: BandSlot::Verb, text: entry.word.clone() }];
        let rest = &toks[vi + 1..];
        let push = |picks: &mut Vec<BandPick>, slot, text: String| {
            if !text.is_empty() {
                picks.push(BandPick { slot, text });
            }
        };
        // The split point is ANY of the verb's own joiners, not one canonical
        // preposition: a story that writes `put noun in noun` and
        // `put noun on noun` as two lines accepts both, and reading the line
        // back has to accept both too (SQ-1111 — `Arity` had room for one).
        let split =
            if max_nouns >= 2 {
                rest.iter().position(|t| joiners.iter().any(|j| t.eq_ignore_ascii_case(j)))
            } else {
                None
            };
        let expected = if max_nouns == 0 {
            Vec::new()
        } else if let Some(j) = split {
            push(&mut picks, BandSlot::Object, rest[..j].join(" "));
            push(&mut picks, BandSlot::Second, rest[j + 1..].join(" "));
            vec![COL_SECOND]
        } else {
            push(&mut picks, BandSlot::Object, rest.join(" "));
            vec![COL_HERE, COL_CARRIED]
        };
        ParsedPhrase { picks, expected }
    }

    /// Re-derive the phrase state from the real input line and point the band
    /// at whatever it should be suggesting (SQ-0676). Called after every change
    /// to `state.input`, so the columns follow typing exactly as they follow
    /// clicking — a click composes onto the prompt, and the prompt is what this
    /// reads back.
    pub fn sync_from_input(&mut self, input: &str) {
        let parsed = self.parse_phrase(input);
        self.picks = parsed.picks;
        // SQ-0677: `focus` is now Tab/Shift-Tab-driven and persists across
        // typing — only re-pointed when the grammar no longer expects the
        // column it is currently on (a verb just got typed, the preposition
        // just got typed, …), mirroring what `advance()` already does for a
        // click/pick. A manual Tab survives further typing in the same slot;
        // `row_sel` is dropped alongside it since a stale row index would
        // belong to the column focus is leaving.
        if !parsed.expected.contains(&self.focus) {
            self.focus = parsed.expected.first().copied().unwrap_or(COL_VERB);
            self.row_sel = None;
        }
    }

    /// The columns the grammar expects the next word to come from: VERB for the
    /// first word, the object columns after an object/pair verb, the
    /// prepositional column once that verb's preposition has been typed. Empty
    /// once nothing more is expected (a solo verb).
    pub fn expected_cols(&self, input: &str) -> Vec<usize> {
        self.parse_phrase(input).expected
    }

    /// The nearest match for the word being typed, as `(focus, index)`:
    /// prefix first, then a prefix on any word of a multi-word name, then a
    /// substring — searching ONLY `self.focus`'s items (SQ-0677: the current
    /// column, not every grammatically-live one), earliest row winning ties.
    /// `None` when nothing is typed, `focus` isn't reachable, or nothing in
    /// it matches (no highlight, and Tab is pure column movement).
    pub fn nearest_match(&self, input: &str) -> Option<(usize, usize)> {
        let token = band_typed_token(input).to_lowercase();
        if token.is_empty() || !self.col_reachable(self.focus) {
            return None;
        }
        let mut best: Option<(u8, usize)> = None;
        for (idx, item) in self.items(self.focus).iter().enumerate() {
            let Some(rank) = band_match_rank(item, &token) else { continue };
            if best.is_none_or(|(r, _)| rank < r) {
                best = Some((rank, idx));
            }
        }
        best.map(|(_, idx)| (self.focus, idx))
    }

    /// The text of [`Self::nearest_match`] — what Tab completes the current
    /// word to while the band is open.
    pub fn nearest_match_text(&self, input: &str) -> Option<String> {
        let (col, idx) = self.nearest_match(input)?;
        self.items(col).get(idx).cloned()
    }

    /// The row highlighted in `focus` right now: an explicit `↑`/`↓`
    /// selection if there is one, else the passive typed nearest match
    /// (SQ-0677 unifies the two — Tab picks whichever of these is showing).
    /// `None` when neither applies.
    pub fn highlighted_row(&self, input: &str) -> Option<usize> {
        self.row_sel.or_else(|| self.nearest_match(input).map(|(_, idx)| idx))
    }

    // ── Focus ────────────────────────────────────────────────────────────────

    /// Every reachable column, left to right — what `Tab`/`Shift-Tab`
    /// ([`Self::step_column`]) step across, and what `advance` clamps
    /// against, so the band never points at a column `col_reachable`
    /// disagrees with.
    pub fn focus_stops(&self) -> Vec<usize> {
        (0..BAND_COLS).filter(|&c| self.col_reachable(c)).collect()
    }

    /// Step `focus` by one stop (`+1` = Tab, `-1` = Shift-Tab) along
    /// [`Self::focus_stops`], clamped rather than wrapped — a left-to-right
    /// grammar has a real first and last column. Clears `row_sel`: a row
    /// highlight belongs to the column it was raised in, not wherever focus
    /// lands next.
    pub fn step_column(&mut self, delta: i32) {
        let stops = self.focus_stops();
        let cur = stops.iter().position(|&s| s == self.focus).unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, stops.len() as i32 - 1) as usize;
        if let Some(&stop) = stops.get(next) {
            self.focus = stop;
        }
        self.row_sel = None;
    }

    /// Move (or start) the explicit row highlight within `focus` by `delta`
    /// (`-1` = Up, `+1` = Down), clamped at the list's ends — never wraps.
    /// The FIRST press starts the highlight — at the typed nearest match if
    /// there is one, else row 0 — WITHOUT moving further, mirroring the old
    /// quick-row "arm on the first press" rule, now applied to columns.
    ///
    /// Keeps `scroll[focus]`'s viewport following the highlight (SQ-0682):
    /// every new `row_sel` is fed straight into [`ListScroll::select`] — the
    /// same primitive the story picker and the IFDB search modal drive their
    /// own lists with — so a selection that steps past the visible window
    /// scrolls into view instead of walking off it. The arm-on-first-press
    /// clamp math itself stays bespoke to the band (neither of the other two
    /// consumers has a "first press only arms" rule), which is why this
    /// isn't just a call to `list_scroll::nav_key`.
    pub fn step_row(&mut self, input: &str, delta: i32, viewport: usize, anim: &crate::config::AnimationConfig) {
        let len = self.items(self.focus).len();
        if len == 0 {
            return;
        }
        let idx = match self.row_sel {
            None => self.nearest_match(input).map(|(_, idx)| idx).unwrap_or(0),
            Some(cur) => (cur as i32 + delta).clamp(0, len as i32 - 1) as usize,
        };
        self.row_sel = Some(idx);
        self.scroll[self.focus].len(len);
        self.scroll[self.focus].select(idx, viewport, anim);
    }

    /// Seed `scroll[focus]`'s selection from whatever is currently
    /// highlighted — the explicit `row_sel` if there is one, else the typed
    /// nearest match, else row 0 — before a page/home/end jump. Without this,
    /// paging would start from wherever `scroll[focus]` last happened to
    /// land (e.g. a mouse-wheel pan, which deliberately does NOT move
    /// `row_sel` — see `Action::BandWheel`'s doc), rather than from the row
    /// actually wearing the `▸` marker.
    fn seed_scroll_selection(&mut self, input: &str) {
        let idx = self
            .row_sel
            .or_else(|| self.nearest_match(input).map(|(_, idx)| idx))
            .unwrap_or(0);
        self.scroll[self.focus].selected = idx;
    }

    /// Page the explicit row highlight within `focus` by ~one viewport
    /// (`dir > 0` = PageDown), scrolling it into view. Routes through the
    /// same shared [`crate::list_scroll::nav_key`] the story picker and IFDB
    /// modal page with (SQ-0682) — the band adopts standard PageUp/PageDown
    /// here, where it previously had none at all.
    pub fn page_row(&mut self, input: &str, dir: i32, viewport: usize, anim: &crate::config::AnimationConfig) {
        let len = self.items(self.focus).len();
        if len == 0 {
            return;
        }
        self.seed_scroll_selection(input);
        let code = if dir > 0 { KeyCode::PageDown } else { KeyCode::PageUp };
        crate::list_scroll::nav_key(&mut self.scroll[self.focus], code, len, viewport, anim);
        self.row_sel = Some(self.scroll[self.focus].selected);
    }

    /// Jump the explicit row highlight within `focus` to the first item
    /// (SQ-0682) — the band adopts standard Home, same as the other two
    /// `list_scroll::nav_key` consumers.
    pub fn home_row(&mut self, viewport: usize, anim: &crate::config::AnimationConfig) {
        let len = self.items(self.focus).len();
        if len == 0 {
            return;
        }
        crate::list_scroll::nav_key(&mut self.scroll[self.focus], KeyCode::Home, len, viewport, anim);
        self.row_sel = Some(0);
    }

    /// Jump the explicit row highlight within `focus` to the last item
    /// (SQ-0682) — the band adopts standard End, same as the other two
    /// `list_scroll::nav_key` consumers.
    pub fn end_row(&mut self, viewport: usize, anim: &crate::config::AnimationConfig) {
        let len = self.items(self.focus).len();
        if len == 0 {
            return;
        }
        crate::list_scroll::nav_key(&mut self.scroll[self.focus], KeyCode::End, len, viewport, anim);
        self.row_sel = Some(len - 1);
    }

    /// Move focus to the next reachable UNFILLED column, or — when there is
    /// nothing left to pick — the last reachable column, so the cursor never
    /// parks somewhere `col_reachable` no longer agrees with (e.g. picking a
    /// Solo verb collapses the ring back down to just VERB). Always clears
    /// `row_sel`, whether or not `focus` actually moved.
    pub fn advance(&mut self) {
        self.row_sel = None;
        for c in 0..BAND_COLS {
            if self.col_reachable(c) && !self.col_filled(c) {
                self.focus = c;
                return;
            }
        }
        self.focus = self.focus_stops().into_iter().next_back().unwrap_or(COL_VERB);
    }

    // ── Picking ──────────────────────────────────────────────────────────────

    fn set_slot(&mut self, slot: BandSlot, text: String) {
        // Filling a slot invalidates everything picked after it: choosing a new
        // verb must not leave the old verb's object stranded in the phrase.
        if let Some(pos) = self.picks.iter().position(|p| p.slot == slot) {
            self.picks.truncate(pos);
        }
        self.picks.push(BandPick { slot, text });
    }

    /// Pick row `idx` of column `col`. No-op when the column is unreachable or
    /// the index is out of range.
    pub fn pick(&mut self, col: usize, idx: usize) {
        if !self.col_reachable(col) {
            return;
        }
        let Some(text) = self.items(col).get(idx).cloned() else { return };
        self.set_slot(Self::col_slot(col), text);
        self.advance();
    }

    /// Pick a verb by word (the quick row, and tests). Replaces the phrase
    /// wholesale: a quick action is a fresh command, not an addition.
    pub fn pick_word(&mut self, word: &str) {
        self.picks.clear();
        self.set_slot(BandSlot::Verb, word.to_string());
        self.advance();
    }

    /// Drop the whole phrase and return to the verb column. (Backspace's
    /// un-pick ladder retired with the band's keyboard ownership — SQ-0676:
    /// Backspace edits the real prompt, and `sync_from_input` re-derives the
    /// phrase from what is left there.)
    pub fn clear_phrase(&mut self) {
        self.picks.clear();
        self.focus = COL_VERB;
        self.row_sel = None;
    }

    /// Whether any of the band's per-column scrolls is animating.
    pub fn has_active_animation(&self) -> bool {
        self.scroll.iter().any(|s| s.has_active_animation())
    }
}

/// Maximum number of submitted command lines retained in `command_history`.
pub const COMMAND_HISTORY_CAP: usize = 500;

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::LayerId;

/// How many walked rooms the maze breadcrumb remembers (SQ-0666). Eight is about as far back as
/// "which way did I come in?" is still a live question in a maze; past that the trail is just a
/// second highlight competing with the here-marker.
pub const MAP_TRAIL_LEN: usize = 8;

// ── Transcript kind ───────────────────────────────────────────────────────────

/// Category tag for each transcript entry.
///
/// `Story` = game output. `Input` = the player's echoed command. `Meta` =
/// app/slash output. `Warning` = VM diagnostics. `Assist` = lanthorn helping the
/// player play (SQ-1045). The `/filter` view is coarse (story = Story+Input, meta
/// = everything lanthorn wrote); the styling is per-variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TranscriptKind {
    Story,
    Input,
    Meta,
    Warning,
    /// A line from the assist set — a vocabulary offer, a completion, a caution
    /// before an irreversible move, a pointer at the hints that exist.
    ///
    /// This variant is what makes an assist distinguishable to the CODE and to
    /// `/filter`, rather than distinguishable only by its wording. It is produced
    /// by [`AppState::push_assist`] and nothing else: see [`crate::assist`] for
    /// the register these lines are written in, and why every one of them also
    /// carries the marker in its text.
    Assist,
}

/// Push a turn's ordered elements into the transcript: text runs via
/// `push_transcript_runs`, images via `push_transcript_image`, in order. The
/// Glulx run-loop path uses this when a `TurnResult` carries interleaved inline
/// images; the plain-text path stays on `push_transcript_runs`.
pub fn apply_transcript_elems(state: &mut AppState, elems: &[crate::session::TranscriptElem]) {
    use crate::session::TranscriptElem;
    for e in elems {
        match e {
            TranscriptElem::Text { text, runs } => {
                state.push_transcript_runs(text, TranscriptKind::Story, runs);
            }
            TranscriptElem::Image(img) => state.push_transcript_image(img.clone()),
            // The window moved out from under the prose above, which the engine
            // froze in place (SQ-0697). Everything so far stays in scrollback;
            // the live screen restarts here, at the window's new origin.
            TranscriptElem::ScreenClear => state.mark_screen_clear(),
        }
    }
}

/// A run of characters in a transcript line carrying Z-machine text-style bits and colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct StyleRun {
    pub start: usize, // char offset within the line, inclusive
    pub end: usize,   // char offset, exclusive
    pub bits: u8,     // 1=reverse, 2=bold, 4=italic, 8=fixed
    #[serde(default)]
    pub fg: u32, // packed ZColour (see pack_zcolour / unpack_zcolour); 0 = Default
    #[serde(default)]
    pub bg: u32, // packed ZColour; 0 = Default
    #[serde(default)]
    pub link: u32, // Glk hyperlink value for this span (0 = no link)
    #[serde(default)]
    pub glk_style: u8, // Glk style class (0=Normal .. 10=User2); indexes the theme's per-style colour slot (SQ-0331)
}

/// Per-paragraph layout formatting for one transcript line, derived from the Glk
/// paragraph stylehints (SQ-0330). A logical transcript line is one paragraph, so
/// this is stored parallel to `transcript`. `Default` = left-flush, no indent —
/// the Z-machine and any buffer that set no layout hints render exactly as before.
/// The renderer turns these into LEADING-SPACE padding in the wrap (so
/// selection/copy/search coordinates stay consistent with what is drawn).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ParaFmt {
    /// Left indent applied to every wrapped row of the paragraph, in cells.
    #[serde(default)]
    pub indent: u16,
    /// Extra indent applied to the FIRST wrapped row only (Glk ParaIndentation);
    /// may be negative for a hanging indent.
    #[serde(default)]
    pub para_indent: i16,
    /// Justification: 0=left-flush (default), 1=fill (rendered as left for now),
    /// 2=centered, 3=right-flush.
    #[serde(default)]
    pub justify: u8,
    /// Char offset within the line at/after which the game had output BUFFERING
    /// OFF (`buffer_mode 0`, ZMSD §7.2/§7.2.1): from there on the text must break
    /// after the last character that fits, never word-wrap. `None` (the default)
    /// = fully buffered, i.e. ordinary word-wrap.
    ///
    /// Per-line (a logical transcript line is one paragraph), derived by
    /// [`AppState::push_transcript_runs`] from the per-run flag `CaptureSink`
    /// records. It is a wrap POLICY, not a style, which is why it lives here
    /// beside the other paragraph layout rather than in [`StyleRun`].
    #[serde(default)]
    pub nowrap_from: Option<u32>,
}

/// Encode a [`zvm::screen::ZColour`] as a packed `u32` for serde-safe storage in
/// [`StyleRun`] (zvm is zero-dep and cannot derive serde).
///
/// Scheme (tag in the high byte): `Default` → 0, `Standard(n)` → `(1 << 24) | n`,
/// `True(v)` → `(2 << 24) | v`, `True24(rgb)` → `(3 << 24) | rgb` (24-bit RGB
/// occupies the low 24 bits exactly).
pub fn pack_zcolour(c: zvm::screen::ZColour) -> u32 {
    use zvm::screen::ZColour;
    match c {
        ZColour::Default    => 0,
        ZColour::Standard(n) => (1 << 24) | n as u32,
        ZColour::True(v)    => (2 << 24) | v as u32,
        ZColour::True24(v)  => (3 << 24) | (v & 0x00FF_FFFF),
    }
}

/// Decode a packed `u32` back to a [`zvm::screen::ZColour`].  Unknown tags → `Default`.
pub fn unpack_zcolour(p: u32) -> zvm::screen::ZColour {
    use zvm::screen::ZColour;
    match p >> 24 {
        1 => ZColour::Standard((p & 0xFF) as u8),
        2 => ZColour::True((p & 0xFFFF) as u16),
        3 => ZColour::True24(p & 0x00FF_FFFF),
        _ => ZColour::Default,
    }
}

/// Map a Blorb sound kind to a backend format, or None for unsupported kinds.
pub fn sound_kind_to_format(k: blorb::SoundKind) -> Option<audio::SoundFormat> {
    match k {
        blorb::SoundKind::Aiff => Some(audio::SoundFormat::Aiff),
        blorb::SoundKind::Ogg => Some(audio::SoundFormat::Ogg),
        blorb::SoundKind::Mod => Some(audio::SoundFormat::Mod),
        blorb::SoundKind::Other => None,
    }
}

/// Map a Glk channel volume (`0x10000` = full; may exceed for amplification) to a
/// linear pre-master gain fraction for the audio backend.
pub fn glk_volume_to_gain(vol: u32) -> f32 {
    vol as f32 / 65536.0
}

/// One in-flight Sound2 volume ramp (`glk_schannel_set_volume_ext` with a nonzero
/// duration): the host interpolates the channel's linear gain from `start_gain`
/// to `target_gain` over `duration_ms`, starting at `start`.
#[derive(Clone, Copy, Debug)]
pub struct VolumeRamp {
    pub start: std::time::Instant,
    pub duration_ms: u32,
    pub start_gain: f32,
    pub target_gain: f32,
}

/// Linearly interpolate a ramping gain: `start_gain` at `elapsed_ms == 0`,
/// `target_gain` once `elapsed_ms >= duration_ms` (and for a zero duration).
pub fn ramp_gain(start_gain: f32, target_gain: f32, elapsed_ms: u32, duration_ms: u32) -> f32 {
    if duration_ms == 0 || elapsed_ms >= duration_ms {
        return target_gain;
    }
    let t = elapsed_ms as f32 / duration_ms as f32;
    start_gain + (target_gain - start_gain) * t
}

/// Map Glk `repeats` to the audio backend's `repeats` byte, or `None` to skip
/// playing entirely. Glk: `0xFFFFFFFF` = loop forever; `0` = play zero times;
/// `N` = N plays. The audio byte reserves `255` for "forever", so finite counts
/// are clamped to `254`.
fn glk_repeats_to_audio(repeats: u32) -> Option<u8> {
    match repeats {
        0 => None,
        0xFFFF_FFFF => Some(255),
        n if n >= 255 => Some(254),
        n => Some(n as u8),
    }
}

/// Short display label for a [`blorb::SoundKind`].
pub fn sound_kind_label(k: blorb::SoundKind) -> &'static str {
    match k {
        blorb::SoundKind::Aiff => "AIFF",
        blorb::SoundKind::Ogg => "OGG",
        blorb::SoundKind::Mod => "MOD",
        blorb::SoundKind::Other => "other",
    }
}

/// Format the `Snd ` resources of a (possibly absent) sound Blorb for the
/// `/play-sound` diagnostic's list mode (no argument).
pub fn format_sound_resource_list(
    blorb: Option<&blorb::Blorb>,
    disk: &std::collections::HashMap<u16, crate::native_sound::DiskSound>,
) -> Vec<String> {
    // The MEDIUM is listed when it has anything, because it is what will play
    // (SQ-0914) — and on the two games that have sounds at all it is the whole
    // inventory. A Blorb that is present but outranked is still worth a line: a
    // listing that silently omits it would leave a person wondering why the `.blb`
    // they filed made no difference.
    if !disk.is_empty() {
        let mut effects: Vec<&crate::native_sound::DiskSound> = disk.values().collect();
        effects.sort_by_key(|s| s.effect);
        let mut lines = vec![format!("{} sound effect(s) on the medium:", effects.len())];
        for s in effects {
            lines.push(format!(
                "  #{}  {} — {} Hz, {} frames  playable",
                s.effect, s.name, s.rate, s.frames
            ));
        }
        if let Some(b) = blorb {
            let n = b.resources().iter().filter(|r| &r.usage == b"Snd ").count();
            if n > 0 {
                lines.push(format!(
                    "a sound blorb is also resolved ({n} resource(s)), and is NOT used:                      the disk's own sounds outrank it",
                ));
            }
        }
        return lines;
    }
    let Some(blorb) = blorb else {
        return vec!["no sound blorb resolved, and none on the medium".to_string()];
    };
    let sounds: Vec<_> = blorb.resources().iter().filter(|r| &r.usage == b"Snd ").collect();
    if sounds.is_empty() {
        return vec!["no Snd resources".to_string()];
    }
    let mut lines = vec![format!("{} sound resource(s):", sounds.len())];
    for r in sounds {
        let kind = match &r.chunk_type {
            b"FORM" => blorb::SoundKind::Aiff,
            b"OGGV" => blorb::SoundKind::Ogg,
            b"MOD " => blorb::SoundKind::Mod,
            _ => blorb::SoundKind::Other,
        };
        let playable = if sound_kind_to_format(kind).is_some() { "playable" } else { "not decodable" };
        lines.push(format!("  #{}  {}  {} bytes  {playable}", r.number, sound_kind_label(kind), r.len));
    }
    lines
}

/// Which source answers for sound effect `n`, and what to hand the mixer.
///
/// **The medium outranks a Blorb (SQ-0914).** A release disk is the rendition
/// Infocom pressed; a `.blb` beside the story is somebody's later re-rendering of
/// it, and on the two games that have both we have measured the Blorb disagreeing
/// with the disk's own header — `Sherlock.blb` states 13032 Hz for effect 11 where
/// the floppy states 18430, because its author baked in a pitch model that is not
/// the interpreter's. Graphics has always resolved this way round:
/// [`crate::graphics::PictSource::resolve`] takes the disk's release art and only
/// falls back to a resource Blorb. Sound was the one inconsistency.
///
/// A free function rather than a method because `play_turn_sounds` is already
/// holding `&mut self.audio`, and disjoint field borrows are what let it read the
/// other two fields at the same time. One definition, so the play path and the
/// `/play-sound` diagnostic cannot drift — a diagnostic that resolves differently
/// from the thing it diagnoses is worse than none.
///
/// The third element is the sample's own name when the medium answered, which is
/// what the report prints and what tells a person which source they are hearing.
///
/// The KIND comes back rather than a decodable format, and a Blorb resource is
/// returned even when nothing here can decode it: `/play-sound` distinguishes
/// "found, but not decodable" from "not found at all", and collapsing the two would
/// blind the diagnostic to a real and reportable state. The medium's own sounds are
/// always AIFF, having been wrapped as one on the way out of the container.
pub fn resolve_sound<'a>(
    disk: &'a std::collections::HashMap<u16, crate::native_sound::DiskSound>,
    blorb: Option<&'a blorb::Blorb>,
    n: u16,
) -> Option<(&'a [u8], blorb::SoundKind, Option<&'a str>)> {
    if let Some(s) = disk.get(&n) {
        return Some((&s.aiff, blorb::SoundKind::Aiff, Some(s.name.as_str())));
    }
    let (bytes, kind) = blorb?.sound(u32::from(n))?;
    Some((bytes, kind, None))
}

/// Step-by-step report for the `/play-sound <n>` diagnostic, built by the
/// (impure) glue in `main.rs` and rendered here as plain transcript lines.
#[derive(Debug, Default, Clone)]
pub struct PlaySoundReport {
    pub number: u32,
    pub enable_sound: bool,
    pub backend_present: bool,
    pub blorb_present: bool,
    /// How many sounds the story's own MEDIUM offers (SQ-0907), so the report says
    /// where a sound came from — or that neither source has one.
    pub disk_sounds: usize,
    /// The medium's own sample answered, rather than a Blorb resource.
    pub from_medium: Option<String>,
    pub resource: Option<(blorb::SoundKind, usize)>,
    pub format: Option<audio::SoundFormat>,
    pub sound_id: Option<audio::SoundId>,
}

/// Render a [`PlaySoundReport`] as transcript lines, stopping early at the
/// first failed stage (resource not found, or format not decodable).
pub fn format_play_sound_report(r: &PlaySoundReport) -> Vec<String> {
    let mut lines = vec![format!("/play-sound {}", r.number)];
    lines.push(format!(
        "enable_sound: {}",
        if r.enable_sound { "on" } else { "off (attempting playback anyway — diagnostic)" }
    ));
    lines.push(format!("audio backend: {}", if r.backend_present { "present" } else { "NONE" }));
    // The medium is listed FIRST because it is consulted first (SQ-0914), and a
    // report whose order disagrees with the resolution order is a report that
    // misleads — which is the same defect SQ-0907 fixed by mentioning the medium at
    // all, over a story whose disk carries fourteen effects.
    lines.push(format!(
        "medium sounds: {}",
        if r.disk_sounds == 0 { "none".to_string() } else { format!("{} effect(s)", r.disk_sounds) }
    ));
    lines.push(format!(
        "sound blorb: {}",
        match (r.blorb_present, r.disk_sounds > 0) {
            (true, true) => "resolved, outranked by the medium",
            (true, false) => "resolved",
            (false, _) => "NONE",
        }
    ));
    let Some((kind, len)) = r.resource else {
        lines.push(format!("resource #{}: NOT FOUND in either source", r.number));
        return lines;
    };
    match &r.from_medium {
        Some(name) => lines.push(format!(
            "effect #{}: found on the medium as {name}, {len} bytes of {kind:?}",
            r.number
        )),
        None => lines.push(format!("resource #{}: found, kind={:?}, {len} bytes", r.number, kind)),
    }
    let Some(format) = r.format else {
        lines.push("format: not decodable".to_string());
        return lines;
    };
    lines.push(format!("format: {format:?} — decodable"));
    match r.sound_id {
        Some(id) => lines.push(format!("playback: started (sound id {id})")),
        None => lines.push("playback: backend returned None".to_string()),
    }
    lines
}

// ── Transcript edits ──────────────────────────────────────────────────────────

/// What a transcript mutation did to the lines that were ALREADY there.
///
/// The wrap cache (SQ-1034) can extend its wrapped rows only while every line it
/// has already wrapped still says what it said. That is a property of the EDIT,
/// not of the resulting buffer — a same-length rewrite and a pure append are
/// indistinguishable afterwards — so every mutator states it at the call site
/// (`AppState::touch_transcript`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptEdit {
    /// Lines were added at the END and nothing before the old end moved. The
    /// wrapped rows for `[..old_len]` are still exactly right, so the cache wraps
    /// the new lines and appends them.
    Appended,
    /// An existing line was edited, merged, removed, truncated away, or the
    /// whole buffer was replaced. Everything wrapped so far is suspect; the
    /// cache rebuilds.
    Rewrote,
    /// `count` lines were inserted starting at raw transcript index `at`,
    /// pushing everything from `at` onward later without touching it —
    /// `push_transcript_internal`/`_styled`'s insert-above-the-prompt
    /// (SQ-0270), the only mutator that does this. Distinct from `Rewrote`
    /// because the wrap cache can REPAIR through it (SQ-1179): re-wrap only
    /// the disturbed tail rather than rebuild from line zero, since every line
    /// before `at` provably did not move. See
    /// [`crate::render::wrap_cache::WrapKey::plan`].
    Inserted { at: usize, count: usize },
}

/// One unbroken run of [`TranscriptEdit::Inserted`] edits since the last
/// opaque [`TranscriptEdit::Rewrote`] (SQ-1179).
///
/// `min_at` only ever needs to be the SMALLEST `at` seen: within an unbroken
/// run the transcript only grows (an opaque rewrite — the only thing that can
/// shrink or otherwise disturb it — resets the run), so every later insert's
/// `at` is at or past the run's start. That means the earliest one is also the
/// only one the wrap cache needs to compare against its own `content.len` —
/// see `WrapKey::plan`, which resumes any repair from there regardless of how
/// many inserts or plain appends followed it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TailInsertRun {
    /// `transcript_edits` value the run started from — i.e. right after the
    /// last opaque rewrite. A repair is only offered when this matches the
    /// wrap cache's OWN synced `content.edits`; otherwise some rewrite the run
    /// doesn't account for happened after the cache was last built.
    pub since_edits: u64,
    /// Smallest raw transcript index any insert in this run targeted.
    pub min_at: usize,
}

// ── Transcript filter ─────────────────────────────────────────────────────────

/// Which categories of transcript entries are currently visible.
///
/// `Both` (the default) shows all entries. `Story` shows only game output.
/// `Meta` shows only app-generated output (slash commands, /help, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TranscriptFilter {
    #[default]
    Both,
    Story,
    Meta,
}

/// Does `kind` pass `filter`? The one predicate
/// [`AppState::visible_transcript_indices_from`] filters the transcript with,
/// pulled out so the wrap cache's SQ-1179 repair can ask the same question of
/// a single line (whether the transcript's cached tail passed the filter at
/// its last sync) without re-deriving the rule.
pub(crate) fn transcript_filter_matches(filter: TranscriptFilter, kind: TranscriptKind) -> bool {
    match filter {
        TranscriptFilter::Both => true,
        TranscriptFilter::Story => matches!(kind, TranscriptKind::Story | TranscriptKind::Input),
        // Assist joins the META bucket, not the story one: `/filter story` is
        // the player asking for 1982, and an assist is exactly what 1982 did
        // not have (SQ-1045).
        TranscriptFilter::Meta => matches!(kind, TranscriptKind::Meta | TranscriptKind::Warning | TranscriptKind::Assist),
    }
}

// ── Tidy animation ────────────────────────────────────────────────────────────

/// One captured stage of the tidy pipeline, held for playback. `graph` is a clone
/// of the layout as it stood after the named stage ran.
#[derive(Debug, Clone)]
pub struct TidyFrame {
    pub label: String,
    pub graph: MapGraph,
    pub description: String,
    pub stats: mapper::layout::TidyStats,
    pub stage_start: bool,
    /// When `Some`, the map pane renders these lines as text (the Build frame's
    /// connection manifest) instead of drawing rooms. `None` for every layout stage.
    pub manifest: Option<Vec<String>>,
}

/// Transient playback state for the tidy animation. While this is `Some`, the map
/// pane renders the current frame's graph instead of the live one. Playback holds
/// on the final frame; `Esc` clears it back to the live map.
#[derive(Debug)]
pub struct TidyAnim {
    pub frames: Vec<TidyFrame>,
    pub idx: usize,
    pub playing: bool,
    /// The layer these frames tidy. Carried explicitly because a frame's graph CANNOT answer it:
    /// every frame is a `layer_subgraph`, whose rooms keep their real layer while its `layers()`
    /// map always reports main-only — so asking the frame yields `MAIN_LAYER`, a layer it holds no
    /// rooms for, and the map draws blank (SQ-0359).
    pub layer: LayerId,
    last_advance: Instant,
}

impl TidyAnim {
    pub fn new(frames: Vec<TidyFrame>, layer: LayerId) -> Self {
        Self { frames, idx: 0, playing: true, layer, last_advance: Instant::now() }
    }

    pub fn current(&self) -> &TidyFrame {
        &self.frames[self.idx]
    }

    fn at_end(&self) -> bool {
        self.idx + 1 >= self.frames.len()
    }

    /// Step `delta` frames (clamped to range) and pause — manual control overrides playback.
    pub fn step(&mut self, delta: isize) {
        let last = self.frames.len().saturating_sub(1) as isize;
        self.idx = (self.idx as isize + delta).clamp(0, last) as usize;
        self.playing = false;
    }

    /// Toggle play/pause; resuming restarts the dwell clock so the current frame holds full time.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.last_advance = Instant::now();
    }

    /// Advance one frame if playing and `dwell` has elapsed since the last advance. Stops (holds)
    /// at the final frame. Returns true if the frame index changed.
    pub fn tick(&mut self, dwell: Duration) -> bool {
        if !self.playing || self.at_end() {
            self.playing = false;
            return false;
        }
        if self.last_advance.elapsed() < dwell {
            return false;
        }
        self.idx += 1;
        self.last_advance = Instant::now();
        if self.at_end() {
            self.playing = false;
        }
        true
    }
}

// ── Replay / rewind ───────────────────────────────────────────────────────────

/// Transient state for the rewind/replay modal. While `Some`, the map pane
/// renders the reconstructed snapshot for `idx` instead of the live graph
/// (like `TidyAnim`). `Esc`/`q` clears it back to the live game with no change.
#[derive(Debug)]
pub struct ReplayState {
    /// Selected turn index into `AppState.history`. The source of truth that
    /// drives map-snapshot reconstruction; `scroll` mirrors it for list display.
    pub idx: usize,
    pub playing: bool,
    last_advance: Instant,
    /// Animated list scroll, synced to `idx` each frame for windowing + scrollbar.
    pub scroll: crate::list_scroll::ListScroll,
}

impl ReplayState {
    /// Open seeded at the last turn (`last_idx`), paused.
    pub fn new(last_idx: usize) -> Self {
        let mut scroll = crate::list_scroll::ListScroll::new();
        scroll.selected = last_idx;
        Self { idx: last_idx, playing: false, last_advance: Instant::now(), scroll }
    }

    /// Step `delta` turns (clamped to `[0, len-1]`) and pause.
    pub fn step(&mut self, delta: isize, len: usize) {
        if len == 0 { self.idx = 0; self.playing = false; return; }
        let last = (len - 1) as isize;
        self.idx = (self.idx as isize + delta).clamp(0, last) as usize;
        self.playing = false;
    }

    /// Toggle auto-play; resuming restarts the dwell clock.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.last_advance = Instant::now();
    }

    /// Advance one turn if playing and `dwell` elapsed; holds at the last turn.
    /// Returns true if `idx` changed.
    pub fn tick(&mut self, dwell: Duration, len: usize) -> bool {
        if !self.playing || len == 0 || self.idx + 1 >= len {
            self.playing = false;
            return false;
        }
        if self.last_advance.elapsed() < dwell {
            return false;
        }
        self.idx += 1;
        self.last_advance = Instant::now();
        if self.idx + 1 >= len {
            self.playing = false;
        }
        true
    }
}

// ── Sound pulse ──────────────────────────────────────────────────────────────

/// A host-side bleep classification for the border-pulse visual cue.
#[derive(Debug, Clone, Copy)]
pub enum BeepKind {
    High,
    Low,
}

/// An in-flight one-shot story-border flash triggered by a `sound_effect` bleep.
#[derive(Debug)]
pub struct SoundPulse {
    pub kind: BeepKind,
    pub started: std::time::Instant,
}

// ── Smooth transcript scroll ─────────────────────────────────────────────────

/// An in-flight smooth-scroll animation over a `usize` row offset. The logical
/// target (e.g. `transcript_scroll`, a `ListScroll` offset) is updated
/// immediately by the caller; this eases the *displayed* offset from `from` to
/// `to` over the tween. Driven by the run loop, which snaps to `to` and clears
/// this once the tween is `done()`. The single animated-offset type, reused by
/// the transcript and by `ListScroll`.
#[derive(Debug, Clone)]
pub struct ScrollAnim {
    /// Displayed offset (rows) when the animation was armed.
    pub from: usize,
    /// Target offset (rows) the animation eases toward.
    pub to: usize,
    /// The timing curve.
    pub tween: crate::anim::Tween,
}

impl ScrollAnim {
    /// Arm an animation easing the displayed offset from `from` to `to` per the
    /// `[animation]` config. Returns `None` when animation is disabled or
    /// `scroll_ms == 0` (the caller should jump instantly and clear any anim) —
    /// the byte-for-byte instant path.
    pub fn to(from: usize, to: usize, cfg: &crate::config::AnimationConfig) -> Option<Self> {
        if !cfg.enabled || cfg.scroll_ms == 0 {
            return None;
        }
        Some(Self {
            from,
            to,
            tween: crate::anim::Tween::new(Duration::from_millis(cfg.scroll_ms), cfg.easing),
        })
    }

    /// The current displayed offset: `lerp(from, to, tween.progress())`.
    pub fn current(&self) -> f64 {
        crate::anim::lerp(self.from as f64, self.to as f64, self.tween.progress())
    }

    /// The settled target offset.
    pub fn target(&self) -> usize {
        self.to
    }

    /// True once the tween has reached its duration.
    pub fn done(&self) -> bool {
        self.tween.done()
    }
}

// ── Background tidy job ───────────────────────────────────────────────────────

/// What a background [`TidyJob`] does. Both run off the main thread and pulse the
/// map border; they differ only in how much they move rooms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TidyKind {
    /// Full relayout + overlap cleanup + hint repair + compaction. Config-driven
    /// (the `background_tidy` setting) — rearranges the whole layer for aesthetics.
    Full,
    /// Overlap cleanup only — nudges rooms just enough to remove rendered overlaps,
    /// preserving the existing layout. Runs on EVERY geometry change regardless of
    /// the `background_tidy` setting, so the map is never left showing an overlap
    /// (the guarantee the old synchronous `apply_turn` cleanup used to provide).
    Cleanup,
}

/// An in-flight background tidy job. The worker thread runs the relayout on a
/// clone of the graph and returns the tidied clone. The run loop polls
/// `handle.is_finished()` each iteration and joins when done.
pub struct TidyJob {
    /// Worker thread handle. Returns the tidied graph clone on success.
    pub handle: std::thread::JoinHandle<mapper::graph::MapGraph>,
    /// The layer being tidied.
    pub layer: mapper::layer::LayerId,
    /// Graph generation recorded at spawn time. Used to detect stale results.
    pub gen: u64,
    /// Instant the job was spawned. Used to compute the pulse phase for the border color.
    pub started: std::time::Instant,
    /// Full relayout vs. overlap-cleanup-only — governs the stale re-trigger.
    pub kind: TidyKind,
}

impl std::fmt::Debug for TidyJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TidyJob")
            .field("layer", &self.layer)
            .field("gen", &self.gen)
            .field("started", &self.started)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// An in-flight background map-render job (SQ-0379). The worker builds the routed
/// `RenderMap` for `(gen, layer)` off the main thread — the routing is the
/// expensive part — and pushes a short label into `steps` at the start of each
/// phase so the map pane can show a live progress trace. The run loop polls
/// `handle.is_finished()` and installs the result when `gen` still matches.
pub struct RenderJob {
    /// Worker thread handle. Returns the routed `RenderMap`.
    pub handle: std::thread::JoinHandle<mapper::render::RenderMap>,
    /// The layer this model is being routed for.
    pub layer: mapper::layer::LayerId,
    /// Graph generation recorded at spawn; a mismatch on completion means the
    /// map changed again mid-build, so the result is stale and a fresh job runs.
    pub gen: u64,
    /// Instant the job was spawned, for the border pulse phase.
    pub started: std::time::Instant,
}

impl std::fmt::Debug for RenderJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderJob")
            .field("layer", &self.layer)
            .field("gen", &self.gen)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

/// An in-flight background job that builds a tidy *animation* off the main thread.
/// The worker runs `run_tidy_pipeline` on a clone of the graph and returns both the
/// captured frames and the mutated (tidied) clone. The run loop polls
/// `handle.is_finished()`, then applies the tidied graph (with a staleness check) and
/// installs the animation. Unlike `TidyJob`, this is NOT an overlay — input keeps
/// flowing while the frames are built.
pub struct AnimBuildJob {
    /// Worker thread handle. Returns the captured frames and the tidied graph clone.
    pub handle: std::thread::JoinHandle<(Vec<TidyFrame>, mapper::graph::MapGraph)>,
    /// The layer being tidied.
    pub layer: mapper::layer::LayerId,
    /// Graph generation recorded at spawn time. Used to detect stale results.
    pub gen: u64,
    /// Instant the job was spawned.
    pub started: std::time::Instant,
    /// Frames captured so far, bumped once per emitted frame by the worker. Read by the
    /// renderer to drive the progress bar while the build runs.
    pub progress: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Estimated final frame count (room count + headroom). The bar is approximate: the
    /// real frame total isn't known until the build finishes, so this is only an estimate.
    pub total: usize,
    /// Whether to install the tidy animation when the build finishes. `animate-tidy`
    /// sets this `true` (play the frames); the instant `tidy-map`/`Retidy` re-tidy sets
    /// it `false` — it reuses the same off-thread build purely to surface the progress
    /// bar, then applies the tidied graph without an animation. (SQ-0261)
    pub animate: bool,
}

impl std::fmt::Debug for AnimBuildJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnimBuildJob")
            .field("layer", &self.layer)
            .field("gen", &self.gen)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

// ── Saves manager state ───────────────────────────────────────────────────────

/// Transient state for the saves-manager modal.
/// `None` in `AppState.saves` = modal closed.
#[derive(Debug, Clone)]
pub struct SavesState {
    /// All discovered save files for the current story (default first, then named).
    pub entries: Vec<crate::persist_files::SaveInfo>,
    /// Selection + animated scroll offset for the entry list.
    pub scroll: crate::list_scroll::ListScroll,
}

// ── File picker state (read-mode create_by_prompt) ─────────────────────────────

/// A minimal list picker over existing VFS filenames, for a read-mode
/// `create_by_prompt`. Rendered with the shared dialog chrome. `None` in
/// `AppState.file_picker` = closed.
#[derive(Debug, Clone)]
pub struct FilePickerState {
    pub names: Vec<String>,
    pub scroll: crate::list_scroll::ListScroll,
}

impl FilePickerState {
    pub fn new(names: Vec<String>) -> Self {
        FilePickerState { names, scroll: Default::default() }
    }
    pub fn selected(&self) -> Option<&str> {
        self.names.get(self.scroll.selected).map(|s| s.as_str())
    }
    pub fn move_up(&mut self) {
        self.scroll.selected = self.scroll.selected.saturating_sub(1);
    }
    pub fn move_down(&mut self) {
        if self.scroll.selected + 1 < self.names.len() {
            self.scroll.selected += 1;
        }
    }
}

// ── Command palette state ───────────────────────────────────────────────────────

/// Transient state for the command-palette popup (SQ-0419): a fuzzy-searchable
/// list of every registry command, usable even where no text prompt exists
/// (modal / debug views). `None` in `AppState.overlays.palette` = closed.
///
/// The palette owns its own input line ([`input`](Self::input)): its first
/// whitespace token is the fuzzy *query* that filters commands; anything after it
/// is passed to the chosen command as *arguments*. Enter runs the selected
/// candidate with those args through the normal slash dispatch path; Esc closes.
#[derive(Debug, Clone)]
pub struct PaletteState {
    /// The palette's own editable line (query + optional args), with a caret.
    pub input: crate::text_field::TextField,
    /// Selection + animated scroll over the current filtered candidate list.
    pub scroll: crate::list_scroll::ListScroll,
    /// True when the palette was promoted from the hotkey (leader) dialog by
    /// pressing `/`; Esc then returns to that dialog instead of closing outright.
    pub from_hotkey: bool,
}

impl PaletteState {
    /// Open an empty palette. `from_hotkey` records whether it was promoted from
    /// the leader dialog (so Esc returns there).
    pub fn new(from_hotkey: bool) -> PaletteState {
        PaletteState {
            input: crate::text_field::TextField::new(String::new()),
            scroll: crate::list_scroll::ListScroll::new(),
            from_hotkey,
        }
    }

    /// The fuzzy query: the first whitespace-delimited token of the input line.
    pub fn query(&self) -> &str {
        self.input.value.split_whitespace().next().unwrap_or("")
    }

    /// The argument string passed to the chosen command: everything after the
    /// first token (leading whitespace trimmed). Empty when only a query is typed.
    pub fn args(&self) -> &str {
        let v = &self.input.value;
        match v.find(char::is_whitespace) {
            Some(sp) => v[sp..].trim_start(),
            None => "",
        }
    }

    /// The command line to execute for command `name`: the command name plus any
    /// typed arguments (`"name"` when there are none).
    pub fn command_line(&self, name: &str) -> String {
        let args = self.args();
        if args.is_empty() {
            name.to_string()
        } else {
            format!("{name} {args}")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Game,
    Map,
}

// ── Text-entry prompt kinds ─────────────────────────────────────────────────

/// Which path field is being edited in the config screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigPathField {
    UserDir,
}

/// Which text prompt a [`TextEntryDialog`] serves, carrying the target room (and
/// edge direction where applicable) or config context each submit needs. Used by
/// `apply_text_entry` to know which mapper/config method to call on submit.
/// (SQ-0307 — replaces the retired bottom-bar `PromptKind`; the y/n
/// `ConfirmDeleteSave` moved to a two-button confirm dialog.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextEntryKind {
    RenameRoom(RoomId),
    EditNotes(RoomId),
    /// Relabel the edge that exits `RoomId` in the given direction.
    RelabelEdge(RoomId, Direction),
    /// Rename the layer with the given id.
    RenameLayer(LayerId),
    /// Edit a config path field (user_dir) from the config screen.
    ConfigEditPath { field: ConfigPathField },
    /// Enter a filename for a game `create_by_prompt` (write modes). The pending
    /// request lives on `AppState.pending_filename`.
    CreateFile,
}

// ── Save-name dialog ──────────────────────────────────────────────────────────

/// State for the save-name modal (a common-dialog with a caret text field). Opened
/// for both the host "Save State as named slot" and an in-game `@save` (which one is
/// tracked by `AppState.ingame_io`). The `field` is prefilled with a date-time
/// default shown greyed until `active`; see the field-behavior state machine in
/// `main.rs`'s save-name intercept.
#[derive(Debug, Clone)]
pub struct SaveNameDialog {
    /// The editable buffer + caret.
    pub field: crate::text_field::TextField,
    /// false = the greyed default placeholder; true = live editing.
    pub active: bool,
    /// True when opened for an in-game `@save` (vs a host Save State slot). Kept for
    /// clarity; the actual save branch is chosen by `AppState.ingame_io`.
    pub ingame: bool,
}

impl SaveNameDialog {
    /// Open with the given date-time default name, greyed and inactive.
    pub fn new(default_name: String, ingame: bool) -> SaveNameDialog {
        SaveNameDialog {
            field: crate::text_field::TextField::new(default_name),
            active: false,
            ingame,
        }
    }
}

// ── Overwrite-confirm dialog (SQ-0648) ──────────────────────────────────────────

/// What a confirmed [`ConfirmOverwriteSave`] resumes.
///
/// The save-as dialog path and the slash `/save <name>` path both write named
/// saves through `save_named`, but only the dialog path has a modal to fall
/// back into on Cancel — the slash path has nothing to reopen, so its pending
/// name has to ride along here instead of being re-read off a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingOverwrite {
    /// The save-as dialog path (host Save State slot or in-game `@save`). The
    /// save-name dialog is left open BEHIND the confirm overlay the whole
    /// time it's up, so a confirm reads the typed name back off it and a
    /// cancel needs no recovery at all — the dialog is exactly as the player
    /// left it.
    SaveAs,
    /// The slash `/save <name>` path: there is no dialog to read the name
    /// back from, so it travels with the pending state instead.
    Slash(String),
}

/// Pending overwrite-confirmation state (SQ-0648): a save-as target whose file
/// already exists. Confirm writes, replacing it; cancel leaves it untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmOverwriteSave {
    /// Absolute path to the target `.lanthorn` file.
    pub path: std::path::PathBuf,
    /// Display name of the EXISTING save at `path` — may differ from the name
    /// the player just typed when two names slugify to the same file.
    pub existing_name: String,
    /// What to do once the player answers.
    pub pending: PendingOverwrite,
}

/// The "keep this download in your library?" prompt (SQ-1086).
///
/// Raised once, right after a story fetched from a URL has booted, because that
/// is the moment the player knows whether they want it: a fetch that is not kept
/// plays from the temp directory it landed in and is forgotten, while keeping it
/// writes it into the library the picker reads, so the next launch finds it
/// without a second fetch (and the IFDB metadata/cover sweep can attach to it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchKeepPrompt {
    /// What came off the network, and the local file the session was booted from.
    pub fetched: crate::story_url::FetchedStory,
    /// The library directory the copy would land in — `default_story_dir`, the
    /// same directory the story picker lists. The prompt is never raised without
    /// one, because there would be nowhere to keep it.
    pub library_dir: std::path::PathBuf,
    /// True when the library already holds a file of that name. This is what
    /// turns the prompt's two buttons into three: replacing and keeping both are
    /// different answers and neither may happen silently.
    pub collision: bool,
    /// SQ-1096: when the download is a ZIP of release **disk images** rather
    /// than a story, the names they would land in the library under. Empty for
    /// the ordinary SQ-1086 case, and that emptiness is what the dialog reads to
    /// choose its wording.
    ///
    /// The two cases reach the screen from different places and must not be
    /// confused: a story prompt is raised inside `boot_story`, after the game is
    /// running, and answering it copies one file. An archive prompt is raised
    /// BEFORE any boot — nothing is running, `fetched.path` is the zip rather
    /// than a story, and answering it unpacks several files or ends the launch.
    /// Only the first ever reaches the game loop's overlay ladder.
    pub disk_images: Vec<String>,
}

// ── Text-entry dialog ─────────────────────────────────────────────────────────

/// A single-field text-entry modal (title, one caret field, OK/Cancel) — the
/// common home for the former bottom-bar map-edit / config-path / create-file
/// prompts (SQ-0307). Unlike the save-name dialog, the field opens ACTIVE with
/// the prompt's initial value and the caret at the end (normal caret editing, no
/// greyed-placeholder adopt semantics). Held in `AppState.text_entry`.
#[derive(Debug, Clone)]
pub struct TextEntryDialog {
    /// Which prompt this dialog serves (carries the submit context).
    pub kind: TextEntryKind,
    /// The editable buffer + caret, prefilled with the prompt's initial value.
    pub field: crate::text_field::TextField,
}

impl TextEntryDialog {
    /// Open for `kind`, prefilled with `initial` (caret at the end).
    pub fn new(kind: TextEntryKind, initial: impl Into<String>) -> TextEntryDialog {
        TextEntryDialog { kind, field: crate::text_field::TextField::new(initial) }
    }
}

// ── The region prompt ─────────────────────────────────────────────────────────

/// One row of the region prompt's choice list — a radio button, not a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionOption {
    /// Send the rooms here. The label is `new` or a layer's name.
    Dest { label: String, target: mapper::layer::MoveTarget },
    /// Cut this passage, and take the side the selected room is on. The label reads as the
    /// passage does — `s from At West End of Long Hall`.
    Seam { label: String, from: RoomId, dir: mapper::direction::Direction },
}

/// What the region prompt is asking, and everything the answer needs (SQ-0439).
///
/// Three questions arrive here, and they are the same question twice over — *which rooms*, and
/// *onto what* — which is why one modal answers all of them. The map's own suggestion has already
/// settled the rooms and asks only where they go; a manual `move-region` may have either half
/// unsettled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionPromptKind {
    /// The map spoke first. The options are destinations, and the three buttons are the three
    /// outcomes the design settled on: separate now / ask again next crossing / never ask.
    Suggest {
        trigger: mapper::suggest::Trigger,
        /// The crossing that fired it — the key a deferral or a refusal is remembered under.
        seam: mapper::suggest::SeamKey,
        region: mapper::layer::Region,
    },
    /// Several passages lead into the selected room and each cuts a different map, so the options
    /// are those passages. `dest` is whatever the player already named, carried through so
    /// answering this question does not silently reopen the other one.
    PickSeam { room: RoomId, dest: crate::input::MoveDest },
    /// The rooms are settled and the destination is not, so the options are the layers they could
    /// land on. `cut` is the passage the seam step chose, kept only to report it once the move
    /// goes through.
    PickDest {
        region: mapper::layer::Region,
        cut: Option<(RoomId, mapper::direction::Direction)>,
    },
}

/// An open region prompt (SQ-0439). Held in [`OverlayState::region_prompt`].
///
/// The focus ring is `options.len()` option rows followed by the buttons, cycled by Tab/Shift-Tab
/// through the shared `dialog_focus`; moving focus onto an option CHOOSES it, so `choice` is
/// simply the last option the ring rested on and survives focus moving down to the buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionPrompt {
    pub kind: RegionPromptKind,
    /// The modal's title bar.
    pub title: String,
    /// What is being asked, in one or two sentences.
    pub body: Vec<String>,
    /// The rooms that would move, named, one per entry and in region order — drawn as a bulleted
    /// list under a count header (SQ-0858). Empty when the rooms are not settled yet (a seam pick,
    /// where each option takes a different set).
    ///
    /// ALL of them, however many: the count header and the "…and N more" tail are the renderer's,
    /// because how many names a modal can afford to show is a question about the terminal and not
    /// about the map.
    pub rooms: Vec<String>,
    /// The choices, best first. Never empty — a prompt with nothing to offer is not opened.
    pub options: Vec<RegionOption>,
    /// Index into `options` of the chosen row.
    pub choice: usize,
}

impl RegionPrompt {
    /// How many slots the focus ring has: one per option, then one per button.
    pub fn focus_slots(&self) -> usize {
        self.options.len() + self.buttons()
    }

    /// How many buttons this prompt shows — three outcomes for a suggestion, Move/Cancel for a
    /// pick.
    pub fn buttons(&self) -> usize {
        if matches!(self.kind, RegionPromptKind::Suggest { .. }) { 3 } else { 2 }
    }

    /// The chosen option, or `None` when the list is somehow empty.
    pub fn chosen(&self) -> Option<&RegionOption> {
        self.options.get(self.choice)
    }
}

/// What the player told the region prompt to do (SQ-0439).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionPromptAct {
    /// Do it, with the chosen option.
    Accept,
    /// Put it off: re-arm this seam for the next crossing. Also what Esc means on a suggestion,
    /// because declining to answer is not the same as saying no.
    Defer,
    /// Never ask about this passage again.
    Never,
    /// Close a manual pick without moving anything.
    Dismiss,
}

/// Which modal the run loop opens for a `create_by_prompt` filename request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilenameModal {
    /// Read mode with existing files: let the player pick one (Task 5).
    Picker,
    /// Write / WriteAppend / ReadWrite: prompt for a new name.
    NamePrompt,
    /// Read mode with no existing files: nothing to pick — cancel immediately.
    AutoCancel,
}

/// Decide the modal for a filename request. Read mode (`fmode == 0x02`) picks from
/// existing VFS files (or auto-cancels when there are none); every other mode
/// prompts for a name.
pub fn filename_modal_for(req: crate::session::FilenameReq, existing_files: usize) -> FilenameModal {
    if req.fmode == 0x02 {
        if existing_files == 0 { FilenameModal::AutoCancel } else { FilenameModal::Picker }
    } else {
        FilenameModal::NamePrompt
    }
}

// ── File browser state ────────────────────────────────────────────────────────

/// Mode for the file browser: picking a file to import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbMode {
    /// Import: browse and pick a `.qzl`/`.sav` file.
    PickFile,
}

/// One entry in the file browser listing.
#[derive(Debug, Clone)]
pub struct FbEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Transient state for the file-browser modal.
/// `None` in `AppState.file_browser` = modal closed.
#[derive(Debug, Clone)]
pub struct FileBrowserState {
    /// Current working directory shown by the browser.
    pub cwd: std::path::PathBuf,
    /// Sorted entries: `..` (if not root), then dirs, then matching files.
    pub entries: Vec<FbEntry>,
    /// Selection + animated scroll offset for the entry list.
    pub scroll: crate::list_scroll::ListScroll,
    /// The file-browser mode (currently always import).
    pub mode: FbMode,
}

impl FileBrowserState {
    /// Build a new `FileBrowserState` for `cwd`, reading the filesystem.
    /// Entries: `..` when not at root, then dirs sorted, then `.qzl`/`.sav` files sorted
    /// (PickFile only).  Entries that fail to read are silently omitted.
    pub fn build(cwd: std::path::PathBuf, mode: FbMode) -> Self {
        let entries = Self::read_entries(&cwd, mode);
        FileBrowserState { cwd, entries, scroll: Default::default(), mode }
    }

    /// (Re)build entries for the current `cwd` and `mode`.
    pub fn refresh(&mut self) {
        self.entries = Self::read_entries(&self.cwd, self.mode);
        self.scroll = Default::default();
    }

    /// Navigate into a subdirectory or parent.
    pub fn cd(&mut self, dir: std::path::PathBuf) {
        self.cwd = dir;
        self.refresh();
    }

    fn read_entries(cwd: &std::path::Path, mode: FbMode) -> Vec<FbEntry> {
        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<String> = Vec::new();

        if let Ok(iter) = std::fs::read_dir(cwd) {
            for entry in iter.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                // Skip hidden files (starting with '.') except we add '..' explicitly.
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    dirs.push(name.to_owned());
                } else if mode == FbMode::PickFile {
                    let lower = name.to_lowercase();
                    if lower.ends_with(".qzl") || lower.ends_with(".sav") {
                        files.push(name.to_owned());
                    }
                }
            }
        }

        dirs.sort_unstable();
        files.sort_unstable();

        let mut entries: Vec<FbEntry> = Vec::new();
        // Prepend ".." if not at root.
        if cwd.parent().is_some() {
            entries.push(FbEntry { name: "..".to_owned(), is_dir: true });
        }
        for d in dirs {
            entries.push(FbEntry { name: d, is_dir: true });
        }
        for f in files {
            entries.push(FbEntry { name: f, is_dir: false });
        }
        entries
    }
}

// ── Config screen state ───────────────────────────────────────────────────────

/// Transient state for the config-screen modal.
/// `None` in `AppState.config_screen` = modal closed.
#[derive(Debug, Clone)]
pub struct ConfigScreenState {
    /// A working copy of the config, edited in the modal.
    /// On Save this is copied to `state.config`; on Cancel it is dropped.
    pub working: crate::config::Config,
    /// Selection + animated scroll offset for the settings list.
    pub scroll: crate::list_scroll::ListScroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Story and map panes side by side.
    Split,
    /// Story only; the map panel is hidden.
    TranscriptFull,
}

/// Which pane the interactive resize mode is currently adjusting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeTarget {
    /// The story/map split ratio (`Layout::Split` only).
    StoryMap,
    /// The inventory dock height.
    InvDock,
    /// The command band's height (only while the band is open; SQ-0238).
    CommandBand,
    /// The room dock's height, carved from the map pane's bottom (only while the
    /// dock is open; SQ-0692).
    RoomDock,
}

/// Where the event loop should go when the current story ends. `Exit` leaves
/// lanthorn entirely (the classic quit); `Library` exits the story but returns
/// to the story picker (only meaningful when launched from a directory). Set by
/// the quit / `/quit-to-library` dispatch and read at the loop's break sites, where
/// the binary maps it to its own `RunOutcome`. (SQ-0435)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExitTarget {
    #[default]
    Exit,
    Library,
}

/// Percentage-based pane sizes, seeded from `Config` at startup and mirrored
/// here for the layout code to consume. `config` stays the persisted source
/// of truth; this is the runtime-facing copy.
#[derive(Debug, Clone, Copy)]
pub struct PaneSizes {
    /// Story's % of the story/map Split (default 50).
    pub split_ratio: u16,
    /// Command band height in rows, including its frame (default 8).
    pub band_height: u16,
    /// Inventory dock height cap as % of screen height (default 33).
    pub inv_dock_pct: u16,
    /// Room dock height as % of screen height (default 33). Measured against the
    /// FRAME, like `inv_dock_pct`, even though the dock is carved out of the map
    /// pane — one unit for both docks, so `dock_pct_for_rows` inverts either.
    pub room_dock_pct: u16,
}

/// Zoom levels for the map pane. `Boxes` is the closest/most-detailed view;
/// `Overview` is the most zoomed-out view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    Boxes,
    Compact,
    Overview,
}

/// Map a fine zoom level (0–8) to the three-variant `Zoom` enum used for rendering.
///
/// Fine levels 0–2 → Overview, 3–5 → Compact, 6–8 → Boxes.
/// The fine level slows down zoom transitions so the middle (Compact) level is
/// reachable without accidentally skipping it when scrolling quickly.
pub(crate) fn zoom_from_level(level: u8) -> Zoom {
    match level {
        0..=2 => Zoom::Overview,
        3..=5 => Zoom::Compact,
        _ => Zoom::Boxes,
    }
}

impl Zoom {
    /// Returns (step_w, step_h): the terminal cell stride per map-grid cell.
    ///
    /// The stride is larger than the box size (see `zoom_box_size`), and the
    /// difference is gutter where connectors route. The Boxes-zoom box is 11×5
    /// (a ~2:1 width:height ratio so it looks square given the ~1:2 terminal cell
    /// aspect; both odd so side anchors land on the exact box centre). The stride
    /// adds an 8-col / 6-row gutter for the direction-aware router's clearance and
    /// perpendicular-crossing lanes.
    pub fn steps(self) -> (i32, i32) {
        match self {
            Zoom::Boxes => (19, 11),
            Zoom::Compact => (12, 5),
            Zoom::Overview => (2, 2),
        }
    }
}

/// Stashed restore data for the launch dialog: (engine save, transcript lines,
/// transcript kinds, screen state).
pub type PendingResume =
    Option<(crate::engine::EngineSave, Vec<String>, Vec<TranscriptKind>, Option<zvm::screen::ScreenState>)>;

/// Transcript bundle loaded from an archive at startup: (lines, kinds, per-line
/// style runs, per-line paragraph layout, per-line inline image).
pub type LoadedTranscript = Option<(
    Vec<String>,
    Vec<TranscriptKind>,
    Vec<Vec<StyleRun>>,
    Vec<ParaFmt>,
    Vec<Option<crate::inline_image::InlineImage>>,
)>;

/// The cached map render model for the live graph, keyed by graph generation and
/// viewed layer. `render_layer` re-runs chain detection + edge routing every call,
/// so re-doing it on an animation / transcript / mouse-move redraw of an otherwise
/// unchanged map is pure waste. The live graph only changes on a turn / tidy apply /
/// map edit — each bumps `graph_gen` — so an unchanged `(gen, layer)` reuses this.
/// Only the live map is cached; replay and tidy-animation graphs are not tracked by
/// `graph_gen` and are rebuilt per frame (see `AppState::cached_map_render`). (SQ-0305)
#[derive(Debug)]
pub(crate) struct MapRenderCache {
    /// Graph generation (`AppState::graph_gen`) this model was routed for.
    pub gen: u64,
    /// Viewed layer this model was routed for.
    pub layer: LayerId,
    /// The routed, zoom-independent render model.
    pub rm: mapper::render::RenderMap,
}

/// Modal / overlay UI state carved off `AppState` (SQ-0307). Each field's
/// presence (`true` / `Some`) means the corresponding modal, dialog, or
/// full-screen overlay is open; `dialog_focus` is the shared button-focus
/// index for the currently-open modal. Grouping only — no behaviour change.
#[derive(Debug, Default)]
pub struct OverlayState {
    /// When true, show the hotkey dialog overlay. Opened by the prefix key (Ctrl+P),
    /// closed by the prefix key again or 'q'.
    pub hotkey_dialog: bool,
    /// Active saves-manager modal state. `None` means the modal is closed.
    pub saves: Option<SavesState>,
    /// Active file-browser modal state. `None` means the browser is closed.
    pub file_browser: Option<FileBrowserState>,
    /// Active VFS file-picker modal state (read-mode `create_by_prompt`).
    /// `None` means the picker is closed.
    pub file_picker: Option<FilePickerState>,
    /// Active command-band dock state. `None` means the band is closed.
    /// Deliberately NOT a modal — see [`AppState::any_modal_overlay_open`].
    pub command_band: Option<CommandBandState>,
    /// Active command-palette popup state (fuzzy slash-command search). `None`
    /// means the palette is closed. (SQ-0419)
    pub palette: Option<PaletteState>,
    /// Active config-screen modal state. `None` means the screen is closed.
    pub config_screen: Option<ConfigScreenState>,
    /// Active rewind/replay modal state. `None` means the modal is closed.
    pub replay: Option<ReplayState>,
    /// When true, the reset-confirmation dialog is open.
    pub reset_dialog: bool,
    /// When true, the Scott-only "game is now over" dialog is open. Set when a
    /// Scott engine's turn quits cleanly (win/loss) instead of exiting the app.
    pub game_over: bool,
    /// When true, the "Also clear the map" checkbox is checked in the reset dialog.
    pub reset_clear_map: bool,
    /// When true, the "Delete saved progress" checkbox is checked in the reset
    /// dialog: on confirm, the game's auto persistent data (VFS cache + aux + auto
    /// Save State) is deleted so the game re-initializes from scratch.
    pub reset_delete_data: bool,
    /// When `Some`, the save-name modal is open (host Save State slot or in-game
    /// `@save`). Replaces the old bottom-bar `PromptKind::SaveAs` overlay so the
    /// prompt renders in the graphics-free dialog area instead of hiding behind a
    /// Glulx graphics window.
    pub save_name_dialog: Option<SaveNameDialog>,
    /// Active single-field text-entry dialog (rename room / edit notes / relabel
    /// edge / rename layer / config path / create-file), if any. A modal drawn in
    /// the graphics-free dialog area; its run-loop intercept owns key/mouse input
    /// while open. (SQ-0307)
    pub text_entry: Option<TextEntryDialog>,
    /// When `Some`, the two-button "delete this save?" confirm dialog is open for
    /// the named save at this path. Confirm deletes it; cancel keeps it. (SQ-0307)
    pub confirm_delete_save: Option<std::path::PathBuf>,
    /// When `Some`, the two-button "overwrite existing save?" confirm dialog is
    /// open: a save-as target that already exists. Confirm writes, replacing
    /// it; cancel leaves it untouched. See [`PendingOverwrite`] for what a
    /// confirm resumes. (SQ-0648)
    pub confirm_overwrite_save: Option<ConfirmOverwriteSave>,
    /// The "keep this download in your library?" prompt (SQ-1086), or `None`
    /// when this launch did not come off a URL (or has already been answered).
    pub fetch_keep: Option<FetchKeepPrompt>,
    /// When true, the "turn history is not being recorded — switch it on?"
    /// prompt is open (SQ-1091). Raised by `open-history` when there is nothing
    /// to replay AND the capture that would have filled it is off, which used to
    /// be a silent no-op.
    pub history_prompt: bool,
    /// When true, the font check is open (SQ-1104): the two-row comparison that
    /// asks whether this terminal's font draws the Nerd Font icon glyphs. Raised
    /// by `/run-font-check` and by the settings screen's `font_check` row; the
    /// FIRST-run ask happens before this state exists at all, in
    /// `startup::ask_font_check`, and both drive the same
    /// `render::font_check_dialog`.
    pub font_check: bool,
    /// `None` while the font check is showing stage one (the icon glyphs, or
    /// not open at all); `Some(nerdfont)` once stage one is answered and the
    /// check has moved on to stage two, the diagonal corner stubs (SQ-1245) —
    /// carrying stage one's answer until stage two closes and both are written
    /// together. Reset to `None` whenever `font_check` closes.
    pub font_check_icon_answer: Option<bool>,
    /// When true, the first-use aux-storage prompt is open.
    pub aux_prompt: bool,
    /// When true, the "Save state before quitting?" confirmation dialog is open.
    pub quit_dialog: bool,
    /// When true, the "Resume saved game?" dialog is shown at startup.
    pub launch_dialog: bool,
    /// Active Hints panel session. `None` means the panel is closed.
    pub hints: Option<HintSession>,
    /// The open `move-region` prompt, if any (SQ-0439): the map's own suggestion that a set of
    /// rooms wants a layer, or the half of a manual `move-region` the command could not settle by
    /// itself. `None` means nothing is being asked.
    pub region_prompt: Option<RegionPrompt>,
    /// Index of the currently focused button in an open modal dialog. Reset to
    /// a button index when a modal opens; cycled by Tab/Shift-Tab.
    pub dialog_focus: usize,
}

/// Where the last v6 frame put one thing on the terminal, for `/dump-windows`
/// (SQ-0585). `native` is the game-pixel rect it came from — zeroed for entries that
/// are not a game window (the pane, the viewport, a carved strip) — so the engine can
/// match each record back to the window it belongs to and report both halves
/// together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V6CellRect {
    pub label: String,
    pub native: (u16, u16, u16, u16),
    pub cells: (u16, u16, u16, u16),
}

/// One finished non-modal v6 frame, held for `/dump-windows` (SQ-0756).
///
/// Only what the FRAME owns is snapshotted. The game's window table and the model
/// built from it are live state and are read live at dump time: a modal overlay runs
/// no game code, so they still describe the same frame. Where the renderer PUT each
/// window does not survive — the next frame overwrites it — so it is captured here,
/// with the ring's own plan/clip for that frame beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V6GameFrame {
    /// This frame's `v6_cell_map`: its render path, pane, story viewport, per-window
    /// placements and chrome strips.
    pub cells: Vec<V6CellRect>,
    pub ring_plan: &'static str,
    pub ring_clip: Option<(u16, u16)>,
    /// Frames drawn since, all of them with a modal overlay up — how stale this is.
    pub modal_frames_since: u32,
}

#[derive(Debug)]
pub struct AppState {
    pub focus: Focus,
    /// Active debug-inspector pane. Tiled into the map slot while `Some`; `None` = closed.
    pub debug: Option<crate::debug_panel::DebugPanelState>,
    pub layout: Layout,
    pub zoom: Zoom,
    /// Fine zoom level (0–8): 0–2 = Overview, 3–5 = Compact, 6–8 = Boxes.
    /// Derived from this level; `zoom_in`/`zoom_out`/`zoom_reset` update both.
    pub zoom_level: u8,
    /// Map scroll offset in grid cells: (x, y).
    pub scroll: (i32, i32),
    pub selected_room: Option<RoomId>,
    /// The route the matrix view is highlighting: the shortest KNOWN walk from the room the player
    /// is standing in to the room they clicked (SQ-0693). Empty = no route on screen.
    ///
    /// Held rather than recomputed each frame because it is an ANSWER, not a view of the graph: it
    /// is minted by one deliberate click, and it survives the player walking on, so they can read
    /// the next direction off the table with the route still in front of them. It is also what the
    /// first rung of the Esc ladder clears — a derived highlight would have nothing to clear.
    /// Session state; nothing about it belongs in a save.
    pub room_path: Vec<mapper::path::Step>,
    /// Matrix-view scroll offset: `(first direction column, first room row)` (SQ-0666).
    ///
    /// Separate from `scroll`, which is the drawn map's viewport in grid cells. They measure
    /// different things in different units, and sharing one field would make a pan of one view
    /// silently derange the other every time `/view-map` was pressed.
    pub matrix_scroll: (u16, u16),
    /// The last few rooms the player walked into, most recent LAST (SQ-0666). Bounded at
    /// [`MAP_TRAIL_LEN`]; used only on maze-flagged layers, where "how did I get here" is the
    /// question a drawn map would have answered by itself.
    pub map_trail: std::collections::VecDeque<RoomId>,
    /// What the mapper still owes a death the game has not finished with (SQ-0671, SQ-0673): the
    /// `tried` record a fatal move may have to take back, and whether a reported death is still
    /// waiting to be resolved by a resurrection. See [`crate::session::DeathWatch`] for the
    /// lifetime of each. Session state; nothing about it is worth persisting, and a restart or
    /// restore replaces it wholesale.
    pub death_watch: crate::session::DeathWatch,
    pub transcript: Vec<String>,
    /// Parallel kind tag for each entry in `transcript` (always same length).
    pub transcript_kinds: Vec<TranscriptKind>,
    /// Whether this session has already shown [`crate::assist::preamble`], the
    /// one-time flourish above the first assist. Session state, deliberately not
    /// persisted: a restore into a fresh run should introduce the voice again,
    /// because the player of that run may never have seen it.
    pub assist_preamble_shown: bool,
    /// The story's own vocabulary, and which unknown words this session has
    /// already answered (SQ-1041). Read from the engine the first time an offer
    /// is considered and cached — the tables are static — and deliberately not
    /// persisted: a restore into a fresh run is a session that has said nothing
    /// yet.
    pub vocab: crate::vocab::VocabState,
    /// The momentary word reveal that is lit right now, if any (SQ-1107).
    ///
    /// Session state of the most fleeting kind — it goes out on the next
    /// keystroke, the next turn or a four-second hold — so nothing persists it
    /// and nothing needs to: a restore that brought a reveal back would be
    /// restoring a keypress.
    pub reveal: Option<crate::reveal::Reveal>,
    /// A silent, disposable copy of the live game, kept between questions
    /// (SQ-1121). Armed at boot with the story's own bytes; the shadow inside it
    /// is booted the first time anything asks a question and reused after that.
    /// Session state and never persisted — an archive carries the recipe already,
    /// in the story file it names.
    pub probe: crate::probe::ShadowProbe,
    /// The search for the way back that is running right now, if any (SQ-0785).
    ///
    /// Session state, never persisted: what it LEARNS is persisted, in the
    /// graph's own `tried` and `probed` records, so a restore starts with no
    /// search running and picks up wherever the last one left off. At most one —
    /// a new move ends whatever was in flight, because the move may itself be the
    /// walk back. See [`crate::return_probe`].
    pub return_search: Option<crate::return_probe::ReturnSearch>,
    /// A vocabulary offer that has been asked of the shadow and not yet answered
    /// (SQ-1124). At most one: a second question while this is outstanding is not
    /// asked at all, and that offer falls back to what it can say unvetted.
    pub vocab_pending: Option<crate::vocab::PendingOffer>,
    /// Which player turn the session is on, counted by lanthorn rather than by
    /// the story (SQ-1124).
    ///
    /// Bumped by [`Self::begin_turn`] at the head of every turn finisher, and
    /// used for exactly one thing: an answer the shadow hands back late belongs
    /// to the turn that asked for it, and printing it against a later one would
    /// attach a suggestion to a command that never provoked it. Distinct from
    /// [`Self::turns`], which counts the STORY's turns, is written into a save's
    /// `Meta`, and comes back from a restore at whatever it was — none of which
    /// is what a staleness test wants.
    pub turn_epoch: u64,
    /// Optional per-line render-style override, parallel to `transcript`. In-memory
    /// only (not persisted). `None` = use the line's per-kind style. Kept length-
    /// synced by `push_transcript_kind`; read defensively by the renderer.
    pub transcript_styles: Vec<Option<ratatui::style::Style>>,
    /// Per-line Z-machine text-style runs, parallel to `transcript` (always same
    /// length). Empty for the common unstyled line. Populated only by game-turn
    /// output via `push_transcript_runs`; persisted in `transcript.json`.
    pub transcript_runs: Vec<Vec<StyleRun>>,
    /// Per-line paragraph layout format, parallel to `transcript` (always same
    /// length). `ParaFmt::default()` = left-flush, no indent (the Z-machine and
    /// any buffer that set no Glk layout hints). Populated by `push_transcript_runs`
    /// from each line's first content run; persisted in `transcript.json` (SQ-0330).
    pub transcript_para: Vec<ParaFmt>,
    /// Optional inline image parallel to `transcript` (always same length).
    /// `Some` marks a logical unit that renders as an image band instead of
    /// text; its `transcript` entry is an empty placeholder. In-memory only
    /// (not persisted — pixels don't serialize).
    pub transcript_images: Vec<Option<crate::inline_image::InlineImage>>,
    /// Which categories of transcript entries are currently visible.
    pub transcript_filter: TranscriptFilter,
    pub transcript_scroll: u16,
    /// The `[more]` pager (SQ-0404): runtime state for paging a single command's
    /// overflowing output one screen at a time instead of jumping to the bottom.
    pub pager: crate::pager::Pager,
    /// The transcript's total wrapped-row count from the last rendered frame,
    /// cached so a command turn can measure how many rows it added (the pager arm
    /// needs the pre-turn total, which is only known at render time). (SQ-0404)
    pub last_transcript_total_rows: u16,
    /// Transcript length at the most recent game screen-clear (`erase_window`),
    /// or `None`. When set and the view is at the bottom, the renderer pins the
    /// post-clear lines to the top of the pane so a screen clear looks fresh
    /// while older scrollback stays reachable above it. See `mark_screen_clear`.
    pub clear_anchor: Option<usize>,
    /// Monotonic transcript-content generation, bumped by every mutation of the
    /// transcript vecs (append / insert / merge / in-place edit / wholesale
    /// reset). Distinguishes a same-length content replacement (rewind / restore)
    /// from an unchanged buffer, which a length check alone cannot. Read by the
    /// transcript wrap cache. (SQ-0305)
    pub transcript_gen: u64,
    /// Monotonic count of transcript mutations that were NOT pure appends —
    /// every [`TranscriptEdit::Rewrote`] or [`TranscriptEdit::Inserted`]
    /// (in-place edit, insert-above-prompt, merge, truncate, wholesale
    /// replacement). `transcript_gen` moves on every mutation and so can only
    /// say "something changed"; this says "something that was already WRAPPED
    /// changed", which is the difference between the wrap cache appending and
    /// rebuilding. (SQ-1034)
    pub transcript_edits: u64,
    /// The current unbroken run of [`TranscriptEdit::Inserted`] edits, if any
    /// (SQ-1179) — what lets the wrap cache REPAIR through an
    /// insert-above-the-prompt instead of rebuilding. A `Cell` because the
    /// render path only holds `&AppState`: it clears this once a sync (of
    /// whichever kind) has caught the wrap cache up to the current
    /// `transcript_edits`, so the NEXT insert starts a fresh run correctly
    /// anchored at that new baseline rather than extending a stale one.
    pub(crate) transcript_tail_insert: std::cell::Cell<Option<TailInsertRun>>,
    /// Cache of the fully wrapped transcript rows, keyed by
    /// [`crate::render::wrap_cache::WrapKey`],
    /// so an unchanged transcript (idle redraw / scroll) is not re-wrapped and the
    /// per-line filter+clone waterfall is skipped. Published/consumed by render.
    /// (SQ-0305)
    pub(crate) transcript_wrap: std::cell::RefCell<Option<crate::render::wrap_cache::CellWrapCache>>,
    /// The RASTER path's twin of `transcript_wrap` (SQ-1034): the v6 pixel
    /// composite's wrapped story rows, under the same key type and the same
    /// append-or-rebuild rule. Two caches because the two products are different
    /// types — glyph rows against `WrappedRow`s — but ONE owner of the question
    /// "has the wrap moved?", because two copies of that rule are what drifted.
    pub(crate) raster_wrap: std::cell::RefCell<Option<crate::render::wrap_cache::RasterWrapCache>>,
    /// Cache of the live map's routed render model, keyed by graph generation +
    /// viewed layer, so an animation / transcript / mouse-move redraw of an
    /// unchanged map reuses the routed model instead of re-running `render_layer`.
    /// See [`MapRenderCache`] and [`AppState::cached_map_render`]. (SQ-0305)
    pub(crate) map_render: std::cell::RefCell<Option<MapRenderCache>>,
    /// The scroll-independent tables `render_map` derives from the LIVE model at
    /// the current zoom — room placement, Boxes-zoom position tables, edge kinds
    /// — keyed `(gen, layer)` plus the zoom stored inside (SQ-1182). Cleared
    /// whenever `map_render` is replaced, so tables derived from a superseded
    /// model (the empty placeholder a first draw seeds, at the SAME `(gen,
    /// layer)` as the real route that follows it) can never be served for the
    /// new one. See `render::map::derived_tables`.
    pub(crate) map_derived:
        std::cell::RefCell<Option<(u64, LayerId, crate::render::map::MapDerived)>>,
    /// In-flight background map-render job (SQ-0379): rebuilds `map_render` for a
    /// new `(graph_gen, layer)` off the main thread so a re-route never blocks the
    /// interpreter. `RefCell` so it can be spawned from within the draw closure
    /// (which holds only `&self`); polled/installed from the loop body.
    pub(crate) render_job: std::cell::RefCell<Option<RenderJob>>,
    /// Live progress trace for the in-flight `render_job`, shared with the worker
    /// thread. The worker pushes a phase label as each starts; the map pane shows
    /// them top-right and they are cleared when the job completes (SQ-0379).
    pub(crate) render_steps: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// Screen origin `(col, row)` of the input line's TEXT (just past the `"> "` prompt), captured
    /// by the renderer each frame so a click can be mapped back to a caret position (SQ-0354).
    ///
    /// Only the renderer knows where the line landed: the prompt's width and the row both depend on
    /// layout the input code cannot see. `None` before the first frame, or when the command bar is
    /// hidden.
    pub(crate) input_text_origin: std::cell::Cell<Option<(u16, u16)>>,
    /// Size `(cols, rows)` of the map pane's inner content, captured by the renderer each frame
    /// so `Action::Recenter` can centre against the pane the player is actually looking at
    /// (SQ-0349).
    ///
    /// Every other recentre path runs in the event loop, which holds the pane rects `draw_frame`
    /// returns; a key action reaches `apply_action`, which does not. It used to assume 80×24, and
    /// `recenter_on` divides the pane by the zoom step to place the view — so on any other pane
    /// size the target landed off-centre. `None` before the first frame.
    pub(crate) map_pane_size: std::cell::Cell<Option<(u16, u16)>>,
    /// List-row viewport (rows) of the currently-open selection-list modal,
    /// captured from the last render so `apply_action` nav can keep the
    /// selection visible and arm scroll animations (mirrors the transcript's
    /// `transcript_viewport_rows`). 0 when no list modal is open.
    pub modal_list_viewport: usize,
    /// The player's command line, with a caret (SQ-0354).
    ///
    /// A `TextField` like every other text entry in the app, so the caret arithmetic — char vs
    /// byte indexing above all — lives in one tested place instead of being rewritten here.
    /// Story-controlled input never lands in this buffer: when the story asks for a single
    /// keypress the run loop's char-mode gate forwards the key straight to the VM, before app
    /// routing sees it.
    pub input: crate::text_field::TextField,
    /// Transient top-right notification popups (save/restore/export banners,
    /// slash results, VM faults, map-command refusals). Fed by [`set_status`] and
    /// [`push_notice`]; they slide in, hold a few seconds, and slide out without
    /// ever touching the score bar or the transcript. `/dump-notifications`
    /// replays the retained history into the story. (SQ-0176)
    ///
    /// [`set_status`]: AppState::set_status
    /// [`push_notice`]: AppState::push_notice
    pub notifications: crate::notify::Notifications,
    /// Modal / overlay UI cluster: every field whose presence means a
    /// modal, dialog, or full-screen overlay is open, plus the shared
    /// modal button-focus index. Grouped off `AppState` in SQ-0307.
    pub overlays: OverlayState,
    /// When true, draw each chained room's alignment code (`R{id}` / `C{id}`) in
    /// its box interior (Boxes zoom only). Palette-only since SQ-0446 (reached
    /// through the `/` command palette) — `Ctrl+A` is a readline caret shortcut
    /// at the story prompt now, not this toggle's direct key.
    pub show_alignment: bool,
    /// When true, portal icons additionally show their destination room name (Boxes zoom only).
    /// Dialog-only; toggled via the leader panel (default group "View", letter
    /// `l`) — `Ctrl+P` is the leader-dialog prefix now, not this toggle's direct key.
    pub show_portal_labels: bool,
    /// Active tidy-animation playback, if any. While `Some`, the map renders the current
    /// captured stage instead of the live graph. Started by `Ctrl+Y`, cleared by `Esc`.
    pub tidy_anim: Option<TidyAnim>,
    /// In-flight background tidy job, if any. The worker runs the relayout on a clone
    /// of the graph and returns the tidied clone. Driven by the run loop (spawn, poll, apply).
    pub tidy_job: Option<TidyJob>,
    /// The map grew while its pane was hidden, so its layout owes a tidy (SQ-1136).
    ///
    /// Set by `turn::schedule_map_maintenance` each time it declines to spawn a job
    /// because nobody can see the result, and cleared by the one catch-up
    /// `loop_tick::catch_up_deferred_map_layout` schedules when the pane comes back.
    /// It is a debt marker, not a queue: any number of deferred turns settle with a
    /// single relayout, because a relayout derives every position from the graph
    /// rather than from the turns that built it.
    pub map_layout_deferred: bool,
    /// In-flight job building a tidy *animation* off-thread, if any. The worker runs the
    /// tidy pipeline on a clone and returns the frames + tidied graph; the run loop installs
    /// the animation when it finishes. Not an overlay — input stays live during the build.
    pub anim_build_job: Option<AnimBuildJob>,
    /// In-flight one-shot story-border flash, if any. Armed by a beep event; expires after SOUND_PULSE_MS.
    pub sound_pulse: Option<SoundPulse>,
    /// Host audio backend, present when audio was enabled at launch. `None` when
    /// disabled or when construction was skipped.
    pub audio: Option<audio::AudioBackend>,
    /// The Blorb holding this story's `Snd ` resources, resolved at launch.
    pub sound_blorb: Option<blorb::Blorb>,
    /// Sounds the story's own MEDIUM carries, by effect number (SQ-0907). Populated
    /// at launch for a story mounted off a release disk; empty for a loose file.
    /// Consulted only when no Blorb answers, so a `.blb` beside the story still wins.
    pub disk_sounds: std::collections::HashMap<u16, crate::native_sound::DiskSound>,
    /// Playing sampled sounds keyed by Z-machine sound number (for `effect` 3 stop).
    pub sound_ids: std::collections::HashMap<u16, audio::SoundId>,
    /// Finish-routines to fire when a sampled sound ends, keyed by its SoundId.
    pub sound_routines: std::collections::HashMap<audio::SoundId, u16>,
    /// Playing Glulx sounds keyed by Glk channel ref (for stop / replace).
    pub glulx_channels: std::collections::HashMap<u32, audio::SoundId>,
    /// Pending sound-notify per playing SoundId: `(sound resource, notify value)`.
    pub glulx_sound_notify: std::collections::HashMap<audio::SoundId, (u32, u32)>,
    /// Pending volume-notify per Glk channel ref (Sound2 `set_volume_ext`):
    /// `(ramp-completion deadline, notify value)`. The host owns the ramp clock;
    /// when the deadline passes the event loop delivers an `evtype_VolumeNotify`.
    /// At most one entry per channel (a new volume change interrupts the prior).
    pub glulx_volume_notify: std::collections::HashMap<u32, (std::time::Instant, u32)>,
    /// Current linear pre-master gain per Glk channel ref, tracking the value a
    /// live volume ramp is interpolating toward (and where a fresh ramp starts).
    /// Seeded on play / set_volume; stepped by [`AppState::advance_volume_ramps`].
    pub glulx_gain: std::collections::HashMap<u32, f32>,
    /// Active Sound2 volume ramps keyed by Glk channel ref (Sound2
    /// `set_volume_ext` with a nonzero duration). The event loop steps these each
    /// pass via [`AppState::advance_volume_ramps`], interpolating the sink gain
    /// linearly from start to target; a completed ramp is removed. At most one per
    /// channel (a new volume change interrupts the prior).
    pub glulx_volume_ramp: std::collections::HashMap<u32, VolumeRamp>,
    /// A pending change to the running Glulx VM's Sound gestalt, set when the
    /// sound toggle / config save flips `enable_sound`. The event loop drains it
    /// (it holds the session) and calls `GlulxSession::set_sound`, so a game that
    /// re-queries `gestalt_Sound` per play honors the toggle. `None` = nothing pending.
    pub pending_vm_sound: Option<bool>,
    /// Set by a config-screen Save when `watch_style` changed; the run loop
    /// (which owns the file-watcher) reconciles it live and clears this.
    pub pending_watch_style: Option<bool>,
    /// In-flight smooth transcript-scroll animation, if any. `transcript_scroll`
    /// holds the target; this eases the displayed offset toward it.
    pub scroll_anim: Option<ScrollAnim>,
    /// When the story pane's scrollbar was last summoned by an actual scroll
    /// (SQ-0782). It holds for `animation.scrollbar_hide_ms`, then fades over
    /// `animation.scrollbar_fade_ms`. `None` = never scrolled this session, so
    /// the bar has never been shown. New game text deliberately does NOT set
    /// this: the bar would flash on every turn.
    pub scrollbar_shown_at: Option<Instant>,
    /// When the transcript viewport last moved — wheel, `PageUp`/`PageDown`, or a
    /// selection auto-scroll at an edge, all funneled through
    /// [`AppState::scroll_transcript_to`] exactly like `scrollbar_shown_at` above.
    /// Used only to gate the sixel-backend scroll-settle debounce (SQ-1198): while
    /// [`AppState::transcript_scroll_in_motion`] reads true, an inline sixel image
    /// renders as a background-filled footprint instead of re-emitting its full
    /// payload, so a scroll past it does not re-send hundreds of KB per step.
    /// `None` = never scrolled this session.
    pub sixel_scroll_motion_at: Option<Instant>,
    /// Monotonically increasing generation counter. Bumped each time the real graph is mutated
    /// by an applied turn. Used to detect stale tidy results (job's gen vs current gen).
    pub graph_gen: u64,
    /// Explicit layer override for the map view. `None` means follow the current room's layer.
    pub viewed_layer: Option<LayerId>,
    /// Which body the room dock draws (SQ-0692). Meaningful only while
    /// `room_dock` is open; kept across a close so reopening returns to the view
    /// you left — except an explicit open, which names the view it wants.
    pub room_dock_view: RoomDockView,
    /// Middle-button drag-pan state. `Some` while a drag gesture is in progress.
    pub drag: Option<DragState>,
    /// Story-pane text selection (left-drag). `Some` while selecting; the
    /// highlight is shown during the drag and copied on release.
    pub selection: Option<crate::clipboard::Selection>,
    /// Auto-scroll direction while a story-pane selection drag sits at an edge:
    /// -1 = top edge (reveal older), +1 = bottom edge (reveal newer), 0 = interior. (SQ-0197)
    pub selection_edge: i32,
    /// This frame's transcript geometry, published by render for the mouse/copy paths. (SQ-0197)
    pub transcript_geom: std::cell::Cell<Option<crate::clipboard::TranscriptGeom>>,
    /// v6 hybrid letterbox scale — the magnification the ART is drawn at,
    /// published per-frame by the render's Layered arm. 0.0 (the Default) and 1.0
    /// both mean "no scaling".
    ///
    /// It is the ART's factor and NOTHING else since SQ-1002; inline story
    /// pictures used to be scaled by it and must not be (see [`Self::v6_hybrid_ring`]).
    /// `/dump-terminal` reports it as the frame's magnification, which is the one
    /// remaining reader that wants exactly this number.
    pub v6_image_scale: std::cell::Cell<f32>,
    /// This frame drew the v6 chrome ring in HYBRID mode — art as pixels, text as
    /// terminal glyphs (SQ-1002).
    ///
    /// Published where [`Self::v6_image_scale`] is, and read by `render_transcript`
    /// for one decision: how big an inline story picture (a drop-cap, a room icon)
    /// should be. In hybrid the two halves of the frame are mapped out of the
    /// game's native pixel space at DIFFERENT rates — art by the letterbox scale,
    /// text at one native 8x16 cell per terminal cell — and a picture drawn inside
    /// the text flow has to follow the text, which is the thing it was authored to
    /// sit beside. Following the art instead is what made Zork Zero's drop-cap
    /// eight rows tall beside the four-line paragraph it opens.
    pub v6_hybrid_ring: std::cell::Cell<bool>,
    /// The per-axis density of the artwork this launch mounted —
    /// [`crate::graphics::PictSource::art_scale`], resolved once at boot by
    /// `startup.rs` (and again by an `@restart`, `reset.rs`) from the archive's own
    /// declared picture space.
    ///
    /// The render path needs it for one decision: `v6_pixel_lock`'s magnification
    /// ladder, whose step is `1 / gcd` of this pair
    /// ([`crate::render::v6_layout::scale_ladder_step`]). It is a fact about the
    /// mounted ARCHIVE, which the screen model does not carry and the render cannot
    /// re-derive, so it is published here beside the other boot facts rather than
    /// threaded through `ScreenModel`. `(2, 2)` — the uniform
    /// [`crate::session::V6_ART_SCALE`] — for every Blorb-sourced and non-v6 story,
    /// which is the rule that has always applied to them. (SQ-0936)
    pub v6_art_scale: (u32, u32),
    /// This frame asked for `v6_pixel_lock` and could not have it: the pane is too
    /// small for even the smallest rung of the ladder, so the frame fell back to
    /// free scaling (SQ-0936).
    ///
    /// Published for diagnostics — the eventual `/info` — and deliberately NOT
    /// surfaced on the game screen: every other too-small decision in this app
    /// degrades rather than blocks, and a notification that fires on every frame of
    /// a small terminal would be worse than the softness it warns about.
    pub v6_scale_lock_fallback: std::cell::Cell<bool>,
    /// This frame asked for `v6_pixel_lock` on a backend that has no rung to snap
    /// to, so the lock was inert (SQ-0978).
    ///
    /// Distinct from [`Self::v6_scale_lock_fallback`], and the distinction is the
    /// whole point: that flag means *the pane is too small*, which a player can fix
    /// by resizing. This one means *half-blocks resolves the picture into cells and
    /// has no device pixel to put an art pixel a whole number of*, which no pane
    /// size changes. Reporting the second as the first would send a reader hunting
    /// for a bigger terminal. See `crate::render::graphics::v6_pixel_lock_applies`.
    ///
    /// Published by both v6 arms, hybrid and raster, because both consult the lock.
    pub v6_scale_lock_inapplicable: std::cell::Cell<bool>,
    /// The page the v6 story window declared for the CURRENT frame, published by
    /// the render's Layered arm alongside the pane flood that paints it (SQ-0704).
    ///
    /// Inline story pictures (Zork Zero's drop-caps and room icons) carry alpha,
    /// and the image protocol hands that alpha to the terminal to resolve — kitty
    /// composites against the terminal's own background and never consults the
    /// cells underneath. So a transparent icon must be flattened onto this page
    /// before it is encoded, or it sits on the terminal's colour instead of the
    /// game's. `None` when the game named no page, or outside a v6 frame; the
    /// theme's `inline_image` style is then the fallback.
    ///
    /// Published rather than read back out of the buffer: the band's own cells
    /// hold what the PREVIOUS frame drew there, so sampling them would feed the
    /// picture its own colours.
    pub v6_story_page: std::cell::Cell<Option<(u8, u8, u8)>>,
    /// The MACHINE's own screen pair — packed `(foreground, background)` — for the
    /// current frame, when the interpreter it presents as has one. Published by
    /// `render_story_pane` from `ScreenModel.fg`/`bg` and read by
    /// `render::screen::v6_host_pair`, so every v6 surface resolves its default ink
    /// and page from one place. (SQ-0740)
    ///
    /// Only ZMSD §8.3's Amiga machine sets it: there one pair paints the whole
    /// screen and the story's own windows sit on it (see
    /// `zvm::screen::amiga_screen_pair`). `None` — every other profile, every
    /// non-v6 frame, and any frame with `honor_game_colours` off — leaves the host
    /// theme owning the page exactly as before.
    pub v6_page_pair: std::cell::Cell<Option<(u32, u32)>>,
    /// The v6 painted ground for the CURRENT frame, republished by `draw_frame`
    /// from [`crate::engine::Engine::paint_surface`] (SQ-0706). `None` whenever the
    /// game has painted none, which is every game that does not draw with
    /// `erase_window`.
    pub v6_paint: std::cell::RefCell<Option<std::sync::Arc<image::RgbaImage>>>,
    /// Where the last v6 frame actually PUT each window, in terminal cells, for
    /// `/dump-windows`. The engine can report the game's pixel rects but has no idea
    /// what the renderer did with them, and a v6 layout defect is nearly always in
    /// that mapping — art scales by pixel, text by cell, and the two disagree.
    /// Rebuilt every frame by the v6 render paths.
    pub v6_cell_map: std::cell::RefCell<Vec<V6CellRect>>,
    /// The last frame the GAME drew — the one `/dump-windows` describes (SQ-0756).
    ///
    /// `v6_cell_map` above is the frame just rendered, and by the time the command
    /// runs that is always the command's own: it is reached through the palette or a
    /// hotkey dialog, both modal overlays, and a modal routes the v6 pane away from
    /// the pixel path. Every window then reports `NOT DRAWN this frame`, which is
    /// precisely the question the command exists to answer. So each non-modal frame
    /// puts its mapping here as it finishes ([`AppState::note_v6_frame_end`]), and
    /// the dump reads THIS.
    pub v6_last_game_frame: std::cell::RefCell<Option<V6GameFrame>>,
    /// The last frame's rendered CELLS — glyphs and styling — for `/dump-cells`
    /// (SQ-0761).
    ///
    /// `v6_last_game_frame` above records where each window LANDED; this records
    /// what was actually painted into every cell of the terminal, which is a
    /// different question and the one every Journey colour defect turned out to be.
    /// Filed at the end of each frame that no modal covers
    /// ([`AppState::note_frame_cells`]) for the same reason the v6 mapping is: the
    /// command may be reached through the palette, and the palette's own frame is
    /// not the one anybody wants to look at.
    pub last_frame_cells: std::cell::RefCell<Option<crate::cell_dump::FrameCells>>,
    /// Recent v6 render paths, newest last, consecutive repeats collapsed to a count
    /// (SQ-0587). `/dump-windows` cannot observe the steady state on its own: typing
    /// `/` opens the command palette, a modal overlay, which itself routes the frame
    /// away from the pixel path — so the last frame before the command runs is always
    /// a palette frame. This history shows what the frames BEFORE it did.
    pub v6_path_log: std::cell::RefCell<Vec<(String, u32)>>,
    /// Save-time display-list diagnostics (SQ-0588), newest last, shown by
    /// `/dump-windows`. Written by `note_v6_save` when a window's recorded ops do
    /// not reproduce its live canvas.
    pub v6_save_log: std::cell::RefCell<Vec<String>>,
    /// The hybrid ring's bottom PLAN for the last frame, and the clip it applied
    /// (SQ-0587): `(lowest opaque native art row, the terminal row the ring was cut
    /// at)`, or `None` when that plan does not clip. The side-border flank bands are
    /// what this drops, so when a game's surrounding art goes missing while its
    /// centre picture stays, these two numbers say whether the ring was cut and how
    /// far up — i.e. whether the graphics canvas still has art down there at all.
    pub v6_ring_plan: std::cell::Cell<&'static str>,
    pub v6_ring_clip: std::cell::Cell<Option<(u16, u16)>>,
    /// WHICH `picture_takeover_reason` arm decided the last hybrid frame's route,
    /// or `None` for "no takeover — the ring drew it" (SQ-0994).
    ///
    /// Published by the hybrid branch of `render_story_pane`, which already
    /// computes it to make the routing decision — a `Cell` store of an
    /// `Option<&'static str>` it was about to discard, so this costs the frame
    /// path nothing. `/dump-terminal` cannot recompute it: the arms are over a
    /// frame's positioned windows, and by the time a command runs the live frame
    /// is the palette's.
    ///
    /// Meaningless in RASTER mode, where the hatch is never reached; the report
    /// says "not evaluated" there rather than reading a stale answer, which is
    /// why the mode is checked beside it.
    pub v6_takeover_reason: std::cell::Cell<Option<&'static str>>,
    /// Last v6 raster story metrics (SQ-0469), cached so a frame that skips the
    /// canvas rebuild (unchanged generation) can still republish the scroll/pager
    /// geometry the render arm returns. Valid across skipped frames because every
    /// input that alters these metrics also bumps the v6 raster generation.
    pub v6_raster_metrics: std::cell::Cell<Option<crate::render::screen::RasterMetrics>>,
    /// Horizontal text margin actually inset on each side of the story text this
    /// frame (SQ-0345), published by `reserve_text_margin` so `render_middle` can
    /// draw the scrollbar flush against the pane border rather than inside the
    /// margin band — only the text gets the margin.
    pub text_margin_applied: std::cell::Cell<u16>,
    /// The selection's extracted copy text, published by render, read on mouse-release. (SQ-0197)
    pub selection_text: std::cell::RefCell<Option<String>>,
    /// Sub-character pan offset in terminal columns/rows, applied on top of `scroll`.
    /// Allows 1-character precision drag panning without changing the cell-unit scroll.
    /// Cleared by `recenter_on`.
    pub char_pan: (i32, i32),

    /// Resolved glyph set for the map renderer.  Defaults to today's hardcoded glyphs;
    /// overwritten at startup (and on `/reload`) from `style.toml` via `style::resolve`.
    pub symbols: crate::symbols::SymbolSet,

    /// Resolved color scheme.  Defaults to `ColorScheme::terminal_default()` (today's exact
    /// ANSI colors); overwritten at startup (and on `/reload`) from `style.toml` via `style::resolve`.
    pub colors: crate::colors::ColorScheme,

    /// Per-game garglk.ini colour overlay (SQ-0319), discovered beside the story
    /// at boot. Kept here so `reload_style` can re-apply it over the freshly
    /// resolved `colors` (garglk sits between the global theme and the user's
    /// per-game `<game_dir>/style.toml`). `None` when no sidecar was found.
    pub garglk_overlay: Option<crate::garglk_ini::GarglkOverlay>,

    /// The global `honor_game_colours` default (from config.toml/CLI, before any
    /// garglk.ini or per-game override), captured at boot. `reload_style` recomputes
    /// the live `config.honor_game_colours` as `per_game > garglk.ini > this base`
    /// (SQ-0318), so `auto` (no per-game override) falls back here.
    pub honor_game_colours_base: bool,

    /// The global `v6_pixel_lock` default (from config.toml, before this game's
    /// own sidecar override), captured at boot (SQ-0945).
    ///
    /// `set-v6-pixel-lock auto` clears the per-game key and has to put the live
    /// `config.v6_pixel_lock` back to what the global file says — and by then the
    /// boot-time override has already overwritten that field, exactly as
    /// `honor_game_colours_base` exists to survive.
    pub v6_pixel_lock_base: bool,

    /// The global `guidance` default (from config.toml, before this game's own
    /// sidecar override), captured at boot (SQ-1123). `set-guidance auto` clears
    /// the per-game key and puts the live value back to this, for exactly the
    /// reason `v6_pixel_lock_base` exists.
    pub guidance_base: bool,

    /// The global `return_probe` default (from config.toml, before this game's
    /// own sidecar override), captured at boot (SQ-0785). `set-return-probe auto`
    /// clears the per-game key and puts the live value back to this — which is a
    /// meaningful thing to do here rather than a synonym for `off`, since the
    /// global default is off and a user who set it globally ON wants that back.
    pub return_probe_base: bool,

    /// The global `v6_render` default (from config.toml, before this game's own
    /// sidecar override), captured at boot (SQ-1123). `set-v6-render auto` clears
    /// the per-game key and puts the live mode back to this.
    pub v6_render_base: crate::config::V6RenderMode,

    /// What `--game-colours` was typed as on this launch, if it was (SQ-0855).
    ///
    /// `None` is the third answer SQ-1082 gave every one of these switches: the
    /// flag was not typed, so nothing on the command line has an opinion and the
    /// per-story sources below speak. `Some(true)` is as much an instruction as
    /// `Some(false)` — while this was a `bool` named `no_game_colours_cli` there
    /// was no way to ask for the colours a sidecar had turned off.
    ///
    /// The base above is already `false` in the `Some(false)` case, but a base is only the
    /// FALLBACK — `reload_style` re-reads the two per-story sources from disk on
    /// every reload, so without this a `garglk.ini` beside the story or a sidecar
    /// `honor` key written on some earlier run would quietly turn the flag off
    /// again. A flag is an instruction for the launch you typed it on and outranks
    /// both, exactly as `--interpreter` outranks this game's sidecar.
    ///
    /// Cleared by `/set-game-colours`, which is the user overriding their own flag
    /// in session — the same "a deliberate edit ends the one-run hold" rule
    /// `--interpreter` follows (SQ-0646).
    pub game_colours_cli: Option<bool>,

    /// True when the artwork this launch loaded is a two-colour rendition with no
    /// machine behind it to state a screen, so `startup.rs` declared the
    /// interpreter colourless (SQ-0806/SQ-0846, narrowed by SQ-0956 to exactly
    /// that launch — see [`crate::graphics::PictSource::declines_game_colours`]).
    ///
    /// Recorded here for exactly the reason `game_colours_cli` above is: the
    /// force-off happens on `Config` before the engine is built, and the boot
    /// `reload_style` that runs a few lines later recomputes the key from the two
    /// per-story FILES and lands on the global base — which was captured *before*
    /// the force-off. Without this the whole thing is undone the moment it is
    /// done: the honour flag reads `true` again from the boot reload onward, so
    /// `poll_zvm_default_colours` starts writing header $2C/$2D that §8.3.2 says
    /// to leave alone, and an `@restart` rebuilds the session honouring the very
    /// colours that paint a two-colour stencil out (SQ-0860).
    ///
    /// A launch-wide fact about the artwork, like the flag above is a launch-wide
    /// fact about the command line, so it sits beside it and is folded into the
    /// same per-story answer rather than lowering the base — which is what keeps
    /// the value PINNED and out of the user's global `config.toml`.
    ///
    /// Cleared by the same events that clear the flag: `/set-game-colours` and a
    /// settings-panel edit of the row. The archive's half of
    /// `declines_game_colours` is expressly a *guess* about a machine, and a
    /// deliberate choice outranks a guess.
    pub artwork_declines_colours: bool,

    /// The story's Z-machine Version, or `None` for an engine that has no such
    /// byte (Glulx, Scott Adams). Captured at startup (SQ-0873).
    ///
    /// Read by `reload_style` for one question: does this launch get its
    /// machine's period look? Colour arrives with v5, so the look belongs to a
    /// v1-v4 story and to nothing else — see [`crate::period`]. On state rather
    /// than re-read from the engine because the reload runs on `AppState` alone,
    /// exactly as `honor_game_colours_base` above does.
    pub story_zversion: Option<u8>,

    /// The machine's period look for this launch, or `None` — the resolved
    /// answer, recomputed by `reload_style` whenever the theme is (SQ-0873).
    ///
    /// The colours of it are already folded into `colors.theme`; what stays here
    /// is what a theme cannot carry: the status band's *behaviour* (the Amiga
    /// reverses behind each run and lets the page show between) and the input
    /// cursor's shape.
    pub period_look: Option<zvm::interpreter::PeriodLook>,
    /// The Version 6 character cell this session's machine quantizes by
    /// (SQ-0917) — the render path's copy of [`zvm::cpu::Machine::v6_cell`].
    ///
    /// Every native-pixel-to-terminal-column step in `render/` divides by this,
    /// so it has to be the SAME number the engine placed the runs with: the
    /// engine emits a run at a multiple of `cell.w` and the renderer recovers
    /// the column by dividing by it. A renderer dividing by 8 while the engine
    /// stepped by 7 puts every run in the wrong column.
    ///
    /// Threaded from here into the geometry helpers rather than read ambiently.
    /// A module-level or thread-local cell would be SQ-0958's shape exactly — a
    /// case that renders without setting one inherits whatever the last case
    /// left, which is invisible under nextest's per-test processes and wrong
    /// under `cargo test`.
    ///
    /// [`zvm::screen::V6Cell::DEFAULT`] (8x16) until a profile says otherwise.
    ///
    /// # …and the FACE that draws it, in the same value (SQ-1009)
    ///
    /// [`crate::native_font::TextFace`] carries the cell, the typeface the
    /// RELEASE shipped (`None` on every medium that ships none), how that face may
    /// be drawn, and the pen. They were two fields and were read together at every
    /// render call site, which is CLAUDE.md's refactoring-policy tell — and the
    /// cell is now DERIVED from the face on a machine whose release shipped a
    /// proportional one, so holding them apart would let a frame be drawn with a
    /// cell from one boot and a face from another.
    pub v6_text: crate::native_font::TextFace,

    /// Resolved keymap.  Defaults to `KeyMap::default()` (today's hardcoded bindings);
    /// overwritten at startup via `KeyMap::resolve(&cfg.keymap)` when a config is present.
    pub keymap: crate::keymap::KeyMap,

    /// Hotkey layout: prefix key, direct command set, dialog groups.
    /// Defaults to the built-in layout; overwritten at startup from config.
    pub hotkeys: crate::keymap::HotkeyLayout,



    /// Set while a game-initiated (v4+) `@save`/`@restore` is awaiting the host's
    /// file I/O. The saves dialog runs in "in-game" mode: its confirm/cancel call
    /// `session.resume_save`/`resume_restore` instead of the Ctrl+S/Ctrl+R path.
    pub ingame_io: Option<crate::session::PendingIo>,

    /// Flag-hop: set by `handle_save_as` after a successful in-game SAVE so
    /// the run loop (where `session`/`mapper`/`last_panes` are in scope) performs
    /// the VM resume + recenter. `Some(true)` = file written. Cleared on resume.
    pub ingame_resume_save: Option<bool>,




    /// The resolved runtime config. Set at startup; updated on config-screen Save.
    pub config: crate::config::Config,

    /// Percentage-based pane sizes, seeded from `config` at startup.
    pub pane_sizes: PaneSizes,

    /// Set (by `apply_action`) whenever `config` has changed in a way that
    /// should be persisted to `config.toml` on the next loop iteration,
    /// regardless of which dispatch path (`KeyResolve::Action` or
    /// `KeyResolve::Command`) handled the key. The run loop checks this after
    /// every event and writes + clears it. See `Action::ResizeExit` /
    /// `Action::ResizeReset`.
    pub pending_config_write: bool,

    /// True while the interactive pane-resize mode is active (leader `z`).
    pub resize_mode: bool,
    /// Which visible pane resize mode is currently adjusting.
    pub resize_target: ResizeTarget,

    /// The pane boundary the mouse is currently dragging, if any (SQ-0669).
    /// While this is set the drag owns every mouse event — see
    /// [`crate::pane_drag`].
    pub pane_drag: Option<crate::pane_drag::PaneDrag>,
    /// The pane boundary the pointer is hovering, if any. Drives the grab
    /// affordance only; `pane_drag` outranks it while a boundary is held.
    pub pane_hover: Option<crate::layout::Boundary>,
    /// The border toggle control the pointer is on, if any (SQ-1123). Drives
    /// the `:hover` style and the floating hint, and nothing else: it is set
    /// from `Moved` events and never claims one, because typing always wins.
    pub control_hover: Option<crate::render::controls::BorderControl>,

    /// The matrix-view room the pointer is on, if any (SQ-1246), paired with
    /// the exact rect it was found under — a row label or a destination cell.
    /// Set from `Moved` events, only while the active layer is drawn as a
    /// matrix; the drawn map view has no equivalent and must not populate
    /// this. Carrying the rect alongside the room (rather than re-resolving
    /// it against a fresh hit-list at draw time, as `control_hover` does)
    /// sidesteps the ambiguity a `BorderControl` never has: one room can be
    /// the destination of several cells, so an id alone would not say which
    /// occurrence the pointer was actually over.
    pub matrix_hover: Option<(RoomId, ratatui::layout::Rect)>,



    /// Session turn counter; incremented on each non-empty `SubmitCommand`.
    /// Written into `Meta` on every save (quick-save and named).
    pub turns: u32,

    /// True when the game has advanced since the last Save State was written
    /// (or since load/restart) — i.e. there is progress not captured in a Save
    /// State. Drives the "unsaved Save State" quit prompt. Set on each turn,
    /// cleared by a Save State save, a restore/load, and restart.
    pub unsaved_progress: bool,

    /// Where the event loop should resume once this story ends: `Exit` (quit
    /// lanthorn) or `Library` (return to the story picker). Set by the quit /
    /// `/quit-to-library` dispatch; read at the loop's break sites. (SQ-0435)
    pub exit_target: ExitTarget,

    /// True when lanthorn was launched against a directory (a story library),
    /// so a picker exists to return to. Set once at startup; gates
    /// `/quit-to-library`. (SQ-0435)
    pub launched_from_library: bool,

    /// True when launched with `--debug`: the cumulative executed-PC coverage set
    /// is written to the per-story `.pcs` sidecar on story-end. Set once at
    /// startup. (SQ-0449)
    pub persist_debug_trace: bool,

    /// Per-turn rewind/replay history. Filled when `config.record_turn_history`
    /// is on; persisted into the `.lanthorn` archive. Empty otherwise. `Arc`-
    /// wrapped (SQ-1184) so handing a snapshot to the background archive
    /// writer is a pointer-copy per turn, not a copy of every retained VM
    /// snapshot.
    pub history: Vec<std::sync::Arc<crate::history::TurnRecord>>,


    /// A game `create_by_prompt` awaiting a host filename (its modal is open).
    pub pending_filename: Option<crate::session::FilenameReq>,
    /// Flag-hop: the chosen filename (`Some(name)`) or cancel (`None`) from the
    /// CreateFile prompt / file picker, drained by the run loop to call
    /// `resume_filename`. Outer `Some` = a decision is ready.
    pub filename_submitted: Option<Option<String>>,

    // ── Autocomplete state ────────────────────────────────────────────────────

    /// Cached parser-vocabulary words from the Z-machine dictionary.
    /// Populated once by the run loop after session creation via
    /// `zvm::dictionary::load(&session.machine.mem).words(&session.machine.mem)`.
    /// If empty, autocomplete draws only from room-description words.
    pub dict_words: Vec<String>,
    /// The words the story has PRINTED that its own dictionary holds — deduped,
    /// **most recently printed first**, extended once a turn by
    /// [`crate::input::refresh_seen_words`] (SQ-1116, accumulated by SQ-1135).
    ///
    /// Cached rather than recomputed per keystroke because it is the ENGINE that
    /// answers what a word is: the story's tokeniser splits the prose and the
    /// story's dictionary decides what survives, and neither is reachable from a
    /// key handler holding only `AppState`. The transcript changes once a turn
    /// anyway, so this is also strictly less work than the per-keystroke scrape it
    /// replaces.
    ///
    /// **It ACCUMULATES over the session** (SQ-1135). It used to be a sliding
    /// twenty-line window, so a word walked out of the list as the transcript
    /// moved on — Arthur names the crystal in the torque once, and three turns
    /// later there was no way to reach the word again. Reverse recency because
    /// the motivating case is exactly the word just printed.
    ///
    /// **And it is DERIVED, never persisted.** The archive already carries the
    /// transcript this is scraped from, so storing the scrape beside it would be
    /// keeping a result next to its own input. Deriving it gives the correct
    /// per-save answer for free: a restore replaces the transcript wholesale, so
    /// restoring to before the crystal was read takes `crystal` away again.
    /// [`Self::reset_transcript_sidecars`] is the one place that invalidation
    /// lives, because it is already called at every site that replaces the
    /// transcript.
    pub seen_words: Vec<String>,
    /// The subset of [`Self::seen_words`] that names a THING, in the same
    /// order — the command band's printed-word block (SQ-1135).
    ///
    /// Two lists rather than one filtered at draw time, because the two answer
    /// different questions and the expensive one is settled once a turn.
    /// Completion wants every word the story printed, including its verbs;
    /// a noun COLUMN wants only the things, and deciding which is which means
    /// asking the story's own objects
    /// ([`Introspect::all_object_words`](crate::engine::Introspect::all_object_words)),
    /// which is a walk of the object table and has no business running per
    /// frame. Both are built in one pass in
    /// [`crate::input::refresh_seen_words`], so they cannot fall out of step.
    pub seen_nouns: Vec<String>,
    /// How many transcript lines have already been folded into [`Self::seen_words`].
    ///
    /// The accumulator's cursor: a turn scrapes only the lines past it, rather
    /// than re-tokenising the whole session every turn. Zeroed together with
    /// `seen_words` by [`Self::reset_transcript_sidecars`], which is what makes a
    /// restore rebuild from the transcript it restored.
    pub seen_scanned: usize,
    /// The words the parser accepts for the objects that are ACTUALLY HERE —
    /// the room's visible contents and what the player carries — refreshed once
    /// a turn by [`crate::input::refresh_scope_words`] (SQ-1042).
    ///
    /// Completion's first tier, ahead of the recent prose and the flat
    /// dictionary, because the noun set changes every room and is where the
    /// friction is: you can see the thing described and cannot find the word for
    /// it. Nothing here is a secret — it names only what the game would list if
    /// asked, and a closed container's contents never enter it (the same walk
    /// the *here* column uses, which stops at a lid).
    pub scope_words: Vec<String>,
    /// Current list of completion candidates, recomputed whenever `input` changes
    /// while in Game focus. Empty means no suggestions are shown.
    pub suggestions: Vec<String>,
    /// Index into `suggestions` of the currently-highlighted candidate.
    /// `Tab` advances this (cycling); typing resets it to 0.
    pub suggestion_idx: usize,
    /// Whether a suggestion has been applied to `input` in the current cycle.
    /// `false` after typing (the candidate at `suggestion_idx` is only a preview);
    /// the first Tab/Shift-Tab applies that candidate and sets this `true`, and
    /// only subsequent presses advance `suggestion_idx`. This keeps the bracketed
    /// highlight aligned with the word actually on the command line.
    pub suggestion_active: bool,

    // ── Command history (shell-style Up/Down recall) ──────────────────────────

    /// Every non-empty submitted line (game commands and slash commands), oldest
    /// first. Capped at `COMMAND_HISTORY_CAP`; consecutive duplicates are skipped.
    /// Persisted per-game in the `.lanthorn` archive.
    pub command_history: Vec<String>,
    /// Navigation cursor into `command_history`. `None` means "not navigating"
    /// (the input line holds the live draft); `Some(i)` means the input shows
    /// `command_history[i]`.
    pub history_cursor: Option<usize>,
    /// In-progress input saved on the first Up press, restored when the player
    /// pages Down past the newest entry.
    pub history_draft: String,

    // ── Adventure title ───────────────────────────────────────────────────────

    /// Resolved adventure title (override > banner > filename stem).
    /// Set once at startup. Used bare by the statusbar `{title}` placeholder;
    /// the pane border uses `pane_title` instead.
    pub title: String,

    /// The story pane's border-title text (SQ-0766): `title`, with the story's
    /// filename appended in parentheses when it differs from `title` (see
    /// [`crate::session::format_pane_title`]). Set once at startup alongside
    /// `title`.
    pub pane_title: String,

    /// The current story's IFID (set at session creation). Used for
    /// title/hint lookup. Empty until set.
    pub ifid: String,

    /// The per-game storage directory (`<data_base>/<story-key>.save/`) holding
    /// this story's saves and sidecars, including the per-game `style.toml` and
    /// `config.toml` overrides. Set once at startup; empty until then (no
    /// per-game reads/writes happen against an empty path).
    pub game_dir: std::path::PathBuf,

    // ── Inventory panel state ─────────────────────────────────────────────────

    /// When true, the inventory strip is shown above the input line.
    pub show_inventory: bool,
    /// Locked player object number once detected by the heuristic. None until
    /// the player moves between two rooms and exactly one object follows.
    pub player_obj: Option<u16>,
    /// Last parsed output from an inventory command (parse fallback when player_obj
    /// is not yet locked).
    pub inventory_fallback: Vec<String>,
    /// The word a click on each inventory dock row composes into the prompt,
    /// in the SAME order as the dock's own display list
    /// (`render::transcript::inventory_items`) — refreshed once per loop tick
    /// by `render::inventory_dock::refresh_inventory_click_words` (SQ-1244).
    /// Empty whenever the panel is neither shown nor sliding.
    pub inventory_click_words: Vec<String>,
    /// The player's previous room (global 0 value from the previous turn).
    pub prev_location: Option<u16>,
    /// Objects whose parent was prev_location at the end of the previous turn.
    pub prev_objects_here: std::collections::BTreeSet<u16>,

    // ── Reset dialog state ────────────────────────────────────────────────────


    // ── Save-name dialog state ────────────────────────────────────────────────


    // ── Aux-storage prompt state ──────────────────────────────────────────────


    // ── Quit dialog state ─────────────────────────────────────────────────────


    // ── Launch dialog state ───────────────────────────────────────────────────

    /// Stashed restore data shown while the launch dialog is open.
    /// Tuple is (engine-tagged save, transcript lines, transcript kinds, screen).
    pub pending_resume: PendingResume,
    /// When true, room numbers (#id) are shown in Boxes-zoom room boxes.
    pub show_room_numbers: bool,
    /// How the current room was detected (for the map indicator). Retained
    /// across turns; updated when a turn reports a method.
    pub loc_method: Option<zvm::location::LocationMethod>,
    /// The current room's display name (from `TurnResult.location`), retained
    /// across turns. Drives the built-in `transcript:location` story rule.
    pub current_room_name: Option<String>,
    /// Whether the detection-method indicator is shown. Default false.
    /// Whether the status/score bar (top row of the story pane) is shown.
    /// Default true; toggled by ToggleStatusBar. Hidden, the row collapses into
    /// the transcript but still pops up briefly for a transient status message.
    pub show_status_bar: bool,

    // ── Hints panel state ─────────────────────────────────────────────────────


    // ── Search state ──────────────────────────────────────────────────────────

    /// The active search query, if any. `None` means no search is active.
    pub search_query: Option<String>,
    /// Positions (0-based) within the visible-index list of lines that match the query.
    pub search_matches: Vec<usize>,
    /// Index into `search_matches` of the current match.
    pub search_idx: usize,

    // ── Dialog focus state ────────────────────────────────────────────────────


    // ── Char-input mode ───────────────────────────────────────────────────────

    /// True when the Z-machine is awaiting a single keypress (`read_char`).
    /// Set each frame by the run loop from `session.pending_input()`.
    /// Used by the renderer to hide the bottom input prompt.
    pub char_mode: bool,

    /// True when a Glulx game is blocked on a non-input Glk event only (a
    /// timer/mouse/hyperlink `glk_select` with no line/char request; see
    /// [`InputKind::Event`]). Set each frame from `session.pending_input()`. Like
    /// `char_mode` it hides the typed-input prompt/cursor, but keystrokes are NOT
    /// forwarded to the game (there is no request to satisfy) — the host delivers
    /// the timer tick / click instead.
    pub event_wait: bool,

    // ── Timed input ───────────────────────────────────────────────────────────

    /// When a timed read/read_char is active and honored (`config.honor_timed_input`),
    /// the wall-clock instant the next interrupt tick is due. `None` when no timer
    /// is armed (untimed read, honor disabled, or an overlay/dialog is open).
    pub input_deadline: Option<std::time::Instant>,

    /// When a Glulx game has armed Glk timer events (`glk_request_timer_events`),
    /// the wall-clock instant the next `evtype_Timer` tick is due. `None` when no
    /// Glk timer is armed. Independent of `input_deadline` (which is the
    /// Z-machine timed-input clock).
    pub glulx_timer_next_fire: Option<std::time::Instant>,

    /// When a v6 turn's picture sequence is being paced out (SQ-0708), the
    /// wall-clock instant the frame now on screen gives way to the next. `None`
    /// whenever nothing is playing.
    ///
    /// This is the CLOCK only — the frames themselves live on the session, beside
    /// the canvases they are snapshots of. Pacing is presentation, so it rides the
    /// loop's existing deadline-driven wakeups exactly like the game clocks do:
    /// the story interpreter is never blocked, nothing sleeps, and input stays
    /// responsive right through the sequence.
    pub picture_pace_next: Option<std::time::Instant>,

    /// The in-game graphics Picker (None when images are disabled or unbuilt).
    pub game_picker: Option<ratatui_image::picker::Picker>,
    /// Bytes and frame flushes the ratatui backend has written to the terminal,
    /// for `/dump-terminal` (SQ-0994). `None` in every headless harness, which
    /// builds no terminal at all — and the report says "unavailable" rather than
    /// printing a zero, because a zero reads as "this session emitted nothing".
    ///
    /// The counters live inside the backend's writer and are read from here, so
    /// the two share one `Arc`. See [`crate::terminal_dump::CountingWriter`] for
    /// why this adds nothing to the frame path.
    pub term_traffic: Option<crate::terminal_dump::TrafficHandle>,
    /// The terminal's own default fg/bg, probed once at startup (SQ-0510). Seeds
    /// the v6 raster canvas's default ink/page when the theme leaves them at
    /// "terminal default"; each field is `None` when the terminal didn't answer.
    pub term_default_colors: crate::term_colors::TermDefaultColors,
    /// Unfinished business from that probe (SQ-0769): while the terminal still
    /// owes an answer this owns the tty, so a late reply is read as the reply it
    /// is instead of reaching the story as a burst of keystrokes.
    pub query_sweep: crate::query_sweep::QuerySweep,
    /// Cached graphics-window protocols (interior-mutable for the render pass).
    pub graphics_render: std::cell::RefCell<crate::render::graphics::GraphicsRender>,
    /// Inline-image band blitter (interior-mutable for the render pass).
    pub inline_image_render: std::cell::RefCell<crate::render::inline_image::InlineImageRender>,

    /// Set when a gvm runtime fault has halted the VM (as opposed to a clean
    /// `glk_exit`). While true, the run loop no longer exits on `TurnResult::quit`
    /// so the fault stays visible and the user can review/save before quitting
    /// deliberately. Reset to `false` on game restart.
    pub vm_halted: bool,

    /// Slide-in inventory dock (bottom). Session-only; starts closed.
    pub inv_dock: crate::anim::PanelSlide,
    /// Slide-in command band (bottom). Session-only; starts closed.
    pub band_dock: crate::anim::PanelSlide,
    /// Slide-in room dock, at the bottom of the MAP pane (SQ-0692).
    /// Session-only; starts closed. Deliberately NOT an overlay: the map above
    /// it stays fully interactive and the story prompt keeps the keyboard.
    pub room_dock: crate::anim::PanelSlide,

    /// Background archive writer (SQ-1184): the per-turn auto-save builds and
    /// writes the `.lanthorn` archive off the main thread. Lazily spawns its
    /// thread on first use, so the hundreds of tests that build `AppState`
    /// and never touch persistence pay nothing for it.
    pub archive_worker: crate::archive_worker::ArchiveWorker,
}

impl Default for AppState {
    fn default() -> Self {
        // **No `app` test ever wants a real audio device, and several production
        // paths a test drives build one lazily** — `ConfigSave`, `ToggleSound`,
        // the picker's sound preview — so the rule cannot live at the call
        // sites: a settings-screen case reaches `AudioBackend::new` without
        // naming audio anywhere in its own body, and its author has no reason
        // to suspect it. Saying it here, once, on the constructor every one of
        // them starts from, is the guard that replaces that convention.
        //
        // It is not cosmetic. `cargo test` gives a binary's tests ONE process
        // and many threads, and a real device opened on a libtest thread is
        // bound to that thread: on macOS three of them together took 491s
        // against 0.76s for the same four cases run with `--test-threads=1`,
        // and on Windows the same shape is a hard `0xc0000005` — cpal's WASAPI
        // backend keeps a process-global `IMMDeviceEnumerator` while
        // initialising COM per THREAD, so a finished test thread's
        // `CoUninitialize` can pull MMDevAPI out from under the next one. That
        // killed the whole `-p app --lib` binary with no test reporting a
        // failure, and nextest could not see it: one process per test means
        // never two threads (SQ-1162).
        //
        // The lazy construction itself still runs and is still asserted on —
        // `TEST_SILENCE` only skips opening the device — so nothing about the
        // production path goes untested. Invisible to the shipped binary,
        // which is built without `cfg(test)`.
        #[cfg(test)]
        audio::disable_output_for_tests();

        Self {
            focus: Focus::Game,
            debug: None,
            layout: Layout::Split,
            zoom: Zoom::Boxes,
            zoom_level: 7, // default = Boxes (level 7)
            scroll: (0, 0),
            selected_room: None,
            room_path: Vec::new(),
            matrix_scroll: (0, 0),
            map_trail: std::collections::VecDeque::new(),
            death_watch: crate::session::DeathWatch::default(),
            transcript: Vec::new(),
            transcript_kinds: Vec::new(),
            assist_preamble_shown: false,
            vocab: crate::vocab::VocabState::default(),
            reveal: None,
            probe: crate::probe::ShadowProbe::default(),
            return_search: None,
            vocab_pending: None,
            turn_epoch: 0,
            transcript_styles: Vec::new(),
            transcript_runs: Vec::new(),
            transcript_para: Vec::new(),
            transcript_images: Vec::new(),
            transcript_filter: TranscriptFilter::Both,
            garglk_overlay: None,
            honor_game_colours_base: true,
            v6_pixel_lock_base: false,
            guidance_base: false,
            return_probe_base: false,
            v6_render_base: crate::config::V6RenderMode::default(),
            game_colours_cli: None,
            artwork_declines_colours: false,
            story_zversion: None,
            period_look: None,
            v6_text: crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
            transcript_scroll: 0,
            pager: crate::pager::Pager::default(),
            last_transcript_total_rows: 0,
            clear_anchor: None,
            transcript_gen: 0,
            transcript_edits: 0,
            transcript_tail_insert: std::cell::Cell::new(None),
            transcript_wrap: std::cell::RefCell::new(None),
            raster_wrap: std::cell::RefCell::new(None),
            map_render: std::cell::RefCell::new(None),
            map_derived: std::cell::RefCell::new(None),
            render_job: std::cell::RefCell::new(None),
            render_steps: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            input_text_origin: std::cell::Cell::new(None),
            map_pane_size: std::cell::Cell::new(None),
            modal_list_viewport: 0,
            input: crate::text_field::TextField::default(),
            notifications: crate::notify::Notifications::default(),
            overlays: OverlayState::default(),
            show_alignment: false,
            show_portal_labels: false,
            tidy_anim: None,
            tidy_job: None,
            map_layout_deferred: false,
            anim_build_job: None,
            sound_pulse: None,
            audio: None,
            sound_blorb: None,
            disk_sounds: Default::default(),
            sound_ids: std::collections::HashMap::new(),
            sound_routines: std::collections::HashMap::new(),
            glulx_channels: std::collections::HashMap::new(),
            glulx_sound_notify: std::collections::HashMap::new(),
            glulx_volume_notify: std::collections::HashMap::new(),
            glulx_gain: std::collections::HashMap::new(),
            glulx_volume_ramp: std::collections::HashMap::new(),
            pending_vm_sound: None,
            pending_watch_style: None,
            scroll_anim: None,
            scrollbar_shown_at: None,
            sixel_scroll_motion_at: None,
            graph_gen: 0,
            viewed_layer: None,
            room_dock_view: RoomDockView::Info,
            drag: None,
            selection: None,
            selection_edge: 0,
            transcript_geom: std::cell::Cell::new(None),
            v6_image_scale: std::cell::Cell::new(1.0),
            v6_hybrid_ring: std::cell::Cell::new(false),
            v6_art_scale: (crate::session::V6_ART_SCALE, crate::session::V6_ART_SCALE),
            v6_scale_lock_fallback: std::cell::Cell::new(false),
            v6_scale_lock_inapplicable: std::cell::Cell::new(false),
            v6_story_page: std::cell::Cell::new(None),
            v6_page_pair: std::cell::Cell::new(None),
            v6_paint: std::cell::RefCell::new(None),
            v6_cell_map: std::cell::RefCell::new(Vec::new()),
            v6_last_game_frame: std::cell::RefCell::new(None),
            last_frame_cells: std::cell::RefCell::new(None),
            v6_path_log: std::cell::RefCell::new(Vec::new()),
            v6_save_log: std::cell::RefCell::new(Vec::new()),
            v6_ring_plan: std::cell::Cell::new("—"),
            v6_ring_clip: std::cell::Cell::new(None),
            v6_takeover_reason: std::cell::Cell::new(None),
            v6_raster_metrics: std::cell::Cell::new(None),
            text_margin_applied: std::cell::Cell::new(0),
            selection_text: std::cell::RefCell::new(None),
            char_pan: (0, 0),
            symbols: crate::symbols::SymbolSet::default(),
            colors: crate::colors::ColorScheme::terminal_default(),
            keymap: crate::keymap::KeyMap::default(),
            hotkeys: crate::keymap::HotkeyLayout::default(),
            ingame_io: None,
            ingame_resume_save: None,
            config: crate::config::Config::default(),
            pane_sizes: PaneSizes {
                split_ratio: 50,
                band_height: crate::render::command_band::DEFAULT_BAND_ROWS,
                inv_dock_pct: 33,
                room_dock_pct: 33,
            },
            pending_config_write: false,
            resize_mode: false,
            resize_target: ResizeTarget::StoryMap,
            pane_drag: None,
            pane_hover: None,
            control_hover: None,
            matrix_hover: None,
            turns: 0,
            unsaved_progress: false,
            exit_target: ExitTarget::Exit,
            launched_from_library: false,
            persist_debug_trace: false,
            history: Vec::new(),
            pending_filename: None,
            filename_submitted: None,
            dict_words: Vec::new(),
            seen_words: Vec::new(),
            seen_nouns: Vec::new(),
            seen_scanned: 0,
            scope_words: Vec::new(),
            suggestions: Vec::new(),
            suggestion_idx: 0,
            suggestion_active: false,
            command_history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            title: String::new(),
            pane_title: String::new(),
            ifid: String::new(),
            game_dir: std::path::PathBuf::new(),
            show_inventory: false,
            player_obj: None,
            inventory_fallback: Vec::new(),
            inventory_click_words: Vec::new(),
            prev_location: None,
            prev_objects_here: std::collections::BTreeSet::new(),
            pending_resume: None,
            show_room_numbers: false,
            loc_method: None,
            current_room_name: None,
            show_status_bar: true,
            search_query: None,
            search_matches: Vec::new(),
            search_idx: 0,
            char_mode: false,
            event_wait: false,
            input_deadline: None,
            glulx_timer_next_fire: None,
            picture_pace_next: None,
            game_picker: None,
            term_traffic: None,
            term_default_colors: crate::term_colors::TermDefaultColors::default(),
            query_sweep: crate::query_sweep::QuerySweep::default(),
            graphics_render: std::cell::RefCell::new(Default::default()),
            inline_image_render: std::cell::RefCell::new(Default::default()),
            vm_halted: false,
            inv_dock: crate::anim::PanelSlide::closed(),
            band_dock: crate::anim::PanelSlide::closed(),
            room_dock: crate::anim::PanelSlide::closed(),
            archive_worker: crate::archive_worker::ArchiveWorker::new(),
        }
    }
}

impl AppState {
    /// Return true if any time-based animation/effect is in flight, so the run
    /// loop should fast-poll (and redraw) to advance it without input. Covers the
    /// tidy border pulse, the sound-beep flash, and smooth transcript scroll.
    pub fn has_active_animation(&self) -> bool {
        self.tidy_job.is_some()
            || self.render_job.borrow().is_some()
            || self.anim_build_job.is_some()
            || self.sound_pulse.is_some()
            || self.scroll_anim.is_some()
            || self.transcript_scrollbar_animating()
            || self.transcript_scroll_in_motion()
            || self.overlays.saves.as_ref().is_some_and(|s| s.scroll.has_active_animation())
            || self.overlays.file_browser.as_ref().is_some_and(|fb| fb.scroll.has_active_animation())
            || self.overlays.config_screen.as_ref().is_some_and(|cs| cs.scroll.has_active_animation())
            || self.overlays.command_band.as_ref().is_some_and(|b| b.has_active_animation())
            || self.overlays.replay.as_ref().is_some_and(|r| r.scroll.has_active_animation())
            || self.overlays.hints.as_ref().is_some_and(|h| h.has_active_animation())
            || self.inv_dock.active()
            || self.band_dock.active()
            || self.room_dock.active()
            || self.notifications.needs_tick()
    }

    /// Clear `command_band` once its slide-out has fully settled. The band is a
    /// "drawer": content persists while `band_dock` animates closed (so the
    /// panel visibly slides out instead of vanishing instantly), and is only
    /// dropped once the slide is done and it's logically closed.
    pub fn settle_command_band(&mut self) {
        if self.overlays.command_band.is_some() && !self.band_dock.open && !self.band_dock.active()
        {
            self.overlays.command_band = None;
        }
    }

    /// True while the command band is on screen (open, or still sliding).
    pub fn command_band_visible(&self) -> bool {
        self.overlays.command_band.is_some() || self.band_dock.active()
    }

    /// Which of the two mutually-exclusive panels is open right now (SQ-1237).
    ///
    /// Reads `band_dock.open` rather than `command_band_visible()` DELIBERATELY:
    /// the dock's `open` flag is the TARGET the last click/command set, set
    /// synchronously; `command_band_visible()` also answers true while a closed
    /// band's content is still sliding out (`settle_command_band` has not yet
    /// trimmed it), which is right for deciding whether to draw the band at all
    /// but wrong for deciding what the player's next click on the cycle control
    /// should do — a click mid-slide-out must not re-open the band it just
    /// asked to close. `show_inventory` has no such lag (it is set directly, no
    /// drawer-content field to trim), so both halves read the same kind of
    /// fact: intent, not on-screen visibility. `Command` wins over `Inventory`
    /// on the (should-not-happen) case both are somehow set, since the command
    /// panel is the cycle's first stop.
    pub fn current_side_panel(&self) -> SidePanel {
        if self.band_dock.open {
            SidePanel::Command
        } else if self.show_inventory {
            SidePanel::Inventory
        } else {
            SidePanel::None
        }
    }

    /// True while the room dock is on screen — open, or still sliding out
    /// (SQ-0692). The layout reserves rows for it in both cases, so a close
    /// animates instead of snapping.
    pub fn room_dock_visible(&self) -> bool {
        self.room_dock.open || self.room_dock.active()
    }

    /// True when the room dock is PINNED — which is exactly "a room is
    /// selected". Pin state is the room selection (SQ-0692): one fact, so the
    /// map highlight, the matrix cross-highlight and the dock header can never
    /// disagree about which room is being described.
    pub fn room_dock_pinned(&self) -> bool {
        self.selected_room.is_some()
    }

    /// Open (or re-point) the room dock in `view`, animating the slide.
    pub fn open_room_dock(&mut self, view: RoomDockView) {
        self.room_dock_view = view;
        if !self.room_dock.open {
            self.room_dock.toggle_to(true, false);
            self.room_dock.arm(&self.config.animation);
        }
    }

    /// Close the room dock, animating the slide. The view is remembered.
    pub fn close_room_dock(&mut self) {
        if self.room_dock.open {
            self.room_dock.toggle_to(false, false);
            self.room_dock.arm(&self.config.animation);
        }
    }

    /// Play the turn's sound events through the backend (gated on config +
    /// backend availability). Bleeps (#1/#2) → tones; samples (#>=3) → Blorb
    /// resource playback, remembering the SoundId (and finish routine) per number.
    /// `effect`: 2/default = start, 3 = stop, 1 = prepare (no-op).
    pub fn play_turn_sounds(&mut self, sounds: &[zvm::cpu::exec::SoundEvent]) {
        if !self.config.enable_sound {
            return;
        }
        let Some(backend) = self.audio.as_mut() else { return };
        for ev in sounds {
            match ev.number {
                0 => {}
                1 | 2 => {
                    if ev.effect == 0 || ev.effect == 2 {
                        let freq = if ev.number == 1 { 800.0 } else { 400.0 };
                        backend.play_tone(freq, 150, ev.volume);
                    }
                }
                n => match ev.effect {
                    3 => {
                        if let Some(id) = self.sound_ids.remove(&n) {
                            backend.stop(id);
                        }
                    }
                    1 => {} // prepare: decode on start
                    _ => {
                        // One definition of which source wins, shared with the
                        // `/play-sound` diagnostic — see `resolve_sound`.
                        let played = resolve_sound(&self.disk_sounds, self.sound_blorb.as_ref(), n)
                            .and_then(|(bytes, kind, _)| {
                                sound_kind_to_format(kind)
                                    .and_then(|fmt| backend.play_sample(bytes, fmt, ev.volume, ev.repeats))
                            });
                        if let Some(id) = played {
                            self.sound_ids.insert(n, id);
                            if ev.routine != 0 {
                                self.sound_routines.insert(id, ev.routine);
                            }
                        }
                    }
                },
            }
        }
    }

    /// Apply this turn's Glk sound-channel operations to the shared audio backend
    /// (Glulx). Mirrors `play_turn_sounds`: gated on the sound config flag and a
    /// present backend; resolves sound resources from `sound_blorb` and tracks the
    /// channel→SoundId and SoundId→notify maps.
    pub fn play_glulx_sound_ops(&mut self, ops: &[crate::session::SchannelOp]) {
        use crate::session::SchannelOp;
        if !self.config.enable_sound {
            return;
        }
        let Some(backend) = self.audio.as_mut() else { return };
        for op in ops {
            match *op {
                SchannelOp::Play { chan, snd, repeats, notify, volume, paused } => {
                    // Playing on a busy channel stops the old sound first; the
                    // replaced sound fires no notify.
                    if let Some(old) = self.glulx_channels.remove(&chan) {
                        backend.stop(old);
                        self.glulx_sound_notify.remove(&old);
                    }
                    let Some(reps) = glk_repeats_to_audio(repeats) else { continue };
                    if let Some(blorb) = &self.sound_blorb {
                        if let Some((bytes, kind)) = blorb.sound(snd) {
                            if let Some(fmt) = sound_kind_to_format(kind) {
                                let gain = glk_volume_to_gain(volume);
                                if let Some(id) = backend.play_sample_gain(bytes, fmt, gain, reps) {
                                    self.glulx_channels.insert(chan, id);
                                    // A fresh sound starts at the channel's snapshot
                                    // volume; cancel any ramp left over from a prior
                                    // sound and seed the current gain.
                                    self.glulx_gain.insert(chan, gain);
                                    self.glulx_volume_ramp.remove(&chan);
                                    if notify != 0 {
                                        self.glulx_sound_notify.insert(id, (snd, notify));
                                    }
                                    // A sound played on a channel paused while empty
                                    // starts paused (Glk 0.7.3 §8.3); a paused sink is
                                    // not "empty", so no finish-notify fires until it
                                    // is unpaused and actually plays out.
                                    if paused {
                                        backend.pause(id);
                                    }
                                }
                            }
                        }
                    }
                }
                SchannelOp::Stop { chan } => {
                    if let Some(id) = self.glulx_channels.remove(&chan) {
                        backend.stop(id);
                        self.glulx_sound_notify.remove(&id);
                    }
                }
                SchannelOp::Destroy { chan } => {
                    if let Some(id) = self.glulx_channels.remove(&chan) {
                        backend.stop(id);
                        self.glulx_sound_notify.remove(&id);
                    }
                    // A destroyed channel can never complete a pending ramp.
                    self.glulx_volume_notify.remove(&chan);
                    self.glulx_volume_ramp.remove(&chan);
                    self.glulx_gain.remove(&chan);
                }
                SchannelOp::SetVolume { chan, vol } => {
                    let gain = glk_volume_to_gain(vol);
                    if let Some(&id) = self.glulx_channels.get(&chan) {
                        backend.set_sample_gain(id, gain);
                    }
                    // A plain set_volume is an immediate change; it interrupts any
                    // in-progress ramp (whose notify is then dropped, per spec §8.3).
                    self.glulx_volume_notify.remove(&chan);
                    self.glulx_volume_ramp.remove(&chan);
                    self.glulx_gain.insert(chan, gain);
                }
                SchannelOp::Pause { chan } => {
                    if let Some(&id) = self.glulx_channels.get(&chan) {
                        backend.pause(id);
                    }
                }
                SchannelOp::Unpause { chan } => {
                    if let Some(&id) = self.glulx_channels.get(&chan) {
                        backend.unpause(id);
                    }
                }
                SchannelOp::SetVolumeExt { chan, vol, duration_ms, notify } => {
                    let target = glk_volume_to_gain(vol);
                    // A new volume change interrupts any prior one on this channel:
                    // the prior notify AND ramp are dropped (spec §8.3).
                    self.glulx_volume_notify.remove(&chan);
                    self.glulx_volume_ramp.remove(&chan);
                    if duration_ms == 0 {
                        // Immediate change: jump the sink and current gain to target.
                        if let Some(&id) = self.glulx_channels.get(&chan) {
                            backend.set_sample_gain(id, target);
                        }
                        self.glulx_gain.insert(chan, target);
                    } else {
                        // Gradual ramp: interpolate from the channel's current gain to
                        // target over duration_ms. The event loop steps the sink each
                        // pass via advance_volume_ramps; the sink stays at start until
                        // the first step (no jump). Interrupting a live ramp starts the
                        // new one from wherever the gain currently sits.
                        let start_gain = self.glulx_gain.get(&chan).copied().unwrap_or(target);
                        self.glulx_volume_ramp.insert(chan, VolumeRamp {
                            start: std::time::Instant::now(),
                            duration_ms,
                            start_gain,
                            target_gain: target,
                        });
                    }
                    // Schedule the notify at now + duration, regardless of whether a
                    // sound is playing (a volume change may occur between sounds).
                    if notify != 0 {
                        let deadline = std::time::Instant::now()
                            + std::time::Duration::from_millis(duration_ms as u64);
                        self.glulx_volume_notify.insert(chan, (deadline, notify));
                    }
                }
            }
        }
    }

    /// Stop all playing audio and clear the per-game sound-tracking maps. Call on
    /// game restart and when sound is turned off, so a finished/blorb SoundId from a
    /// prior game can never misfire a finish-routine or Glk sound-notify into a new
    /// game state, and no sink keeps playing across a restart.
    pub fn reset_sound_sidecars(&mut self) {
        if let Some(b) = self.audio.as_mut() {
            b.stop_all();
        }
        self.sound_ids.clear();
        self.sound_routines.clear();
        self.glulx_channels.clear();
        self.glulx_sound_notify.clear();
        self.glulx_volume_notify.clear();
        self.glulx_gain.clear();
        self.glulx_volume_ramp.clear();
    }

    /// Step every active Sound2 volume ramp to time `now`: interpolate the
    /// channel's linear gain, apply it to the live sink, and drop the ramp once
    /// it completes. Called each event-loop pass (host owns the ramp clock).
    pub fn advance_volume_ramps(&mut self, now: std::time::Instant) {
        if self.glulx_volume_ramp.is_empty() {
            return;
        }
        let mut done: Vec<u32> = Vec::new();
        for (&chan, ramp) in self.glulx_volume_ramp.iter() {
            let elapsed = now.saturating_duration_since(ramp.start).as_millis() as u32;
            let gain = ramp_gain(ramp.start_gain, ramp.target_gain, elapsed, ramp.duration_ms);
            self.glulx_gain.insert(chan, gain);
            if let Some(&id) = self.glulx_channels.get(&chan) {
                if let Some(b) = self.audio.as_mut() {
                    b.set_sample_gain(id, gain);
                }
            }
            if elapsed >= ramp.duration_ms {
                done.push(chan);
            }
        }
        for chan in done {
            self.glulx_volume_ramp.remove(&chan);
        }
    }

    /// Set the transcript scroll target to `target`. When animation is enabled
    /// and `scroll_ms > 0`, arm (or retarget) a smooth-scroll animation from the
    /// current displayed offset toward `target`; otherwise jump instantly and
    /// clear any in-flight animation (exactly today's instant scroll).
    pub fn scroll_transcript_to(&mut self, target: u16) {
        let from = self.effective_transcript_scroll() as usize;
        self.transcript_scroll = target;
        self.scroll_anim = ScrollAnim::to(from, target as usize, &self.config.animation);
        // Every story-pane scroll comes through here — wheel, PgUp/PgDn and the
        // other scroll keys, the [more] pager, a selection drag at an edge — and
        // nothing else does, which is exactly the auto-hide trigger set the bar
        // wants (SQ-0782). New game text sets `transcript_scroll` directly.
        self.scrollbar_shown_at = Some(Instant::now());
        // Same funnel, for the sixel scroll-settle debounce (SQ-1198): every real
        // scroll motion — wheel, page, drag-autoscroll — restarts the window.
        self.sixel_scroll_motion_at = Some(Instant::now());
    }

    /// How long the transcript viewport is considered "in motion" after the last
    /// scroll (SQ-1198), for [`AppState::transcript_scroll_in_motion`].
    ///
    /// `default_scroll_ms()` (120ms, `config.animation.scroll_ms`) is the length
    /// of ONE smooth-scroll tween — so a lone wheel notch is still tweening for
    /// the whole of it — plus one `TIDY_POLL_MS` tick (33ms, the fast-poll cadence
    /// `has_active_animation()` already earns) as margin, so the tween's own
    /// settle frame is never mistaken for the debounce's. A flurry of scroll
    /// steps each restarts the window before it closes, so it stays open for the
    /// whole flurry and only opens the settle frame once the wheel actually stops.
    const SIXEL_SCROLL_SETTLE_MS: u64 = 150;

    /// True while the transcript viewport is still "in motion" from a recent
    /// scroll (SQ-1198) — see [`Self::SIXEL_SCROLL_SETTLE_MS`]. Read by the sixel
    /// backend only: kitty re-places by id and half-blocks are ordinary cells, so
    /// neither needs this.
    pub fn transcript_scroll_in_motion(&self) -> bool {
        self.sixel_scroll_motion_at
            .is_some_and(|t| t.elapsed().as_millis() < Self::SIXEL_SCROLL_SETTLE_MS as u128)
    }

    /// Drop a fully-elapsed scroll-motion window (called from the run loop,
    /// mirroring [`Self::finalize_scrollbar_if_done`]). Returns `true` iff this
    /// call just closed the window, so the loop forces the one redraw where a
    /// sixel image goes from its footprint back to its full payload at the
    /// now-settled position — the frame `transcript_scroll_in_motion` flipping
    /// false is itself the content change, so it needs its own settle frame
    /// exactly as the scrollbar fade does.
    pub fn finalize_sixel_scroll_motion_if_done(&mut self) -> bool {
        if self.sixel_scroll_motion_at.is_some() && !self.transcript_scroll_in_motion() {
            self.sixel_scroll_motion_at = None;
            return true;
        }
        false
    }

    /// How opaque the story pane's scrollbar is this frame, in `[0,1]`
    /// (SQ-0782). `1.0` while the reveal window is open, easing to `0.0` across
    /// the fade, then staying hidden until the next scroll.
    ///
    /// `scrollbar_hide_ms == 0` disables the auto-hide entirely (always `1.0`);
    /// `scrollbar_fade_ms == 0`, or animation turned off, pops instead of fading.
    /// Before the first scroll of a session the bar has never been summoned, so
    /// it is hidden.
    pub fn transcript_scrollbar_opacity(&self) -> f64 {
        let anim = &self.config.animation;
        if anim.scrollbar_hide_ms == 0 {
            return 1.0;
        }
        let Some(shown) = self.scrollbar_shown_at else { return 0.0 };
        let fade_ms = if anim.enabled { anim.scrollbar_fade_ms } else { 0 };
        let elapsed = shown.elapsed().as_millis() as u64;
        if elapsed < anim.scrollbar_hide_ms {
            return 1.0;
        }
        if fade_ms == 0 {
            return 0.0;
        }
        let t = (elapsed - anim.scrollbar_hide_ms) as f64 / fade_ms as f64;
        1.0 - crate::anim::ease(anim.easing, t)
    }

    /// Drop a fully-elapsed reveal (called from the run loop). Returns `true`
    /// iff this call cleared one, so the loop forces the single redraw that
    /// paints the bar AWAY: the last frame the fade itself asks for is the one
    /// at the dregs of its opacity, which still puts a (near-backdrop) colour
    /// in the gutter. The same settle-frame problem, and the same fix, as
    /// `finalize_scroll_if_done`. (SQ-0782)
    pub fn finalize_scrollbar_if_done(&mut self) -> bool {
        if self.config.animation.scrollbar_hide_ms == 0 {
            return false; // pinned on: there is nothing to settle
        }
        if self.scrollbar_shown_at.is_some() && !self.transcript_scrollbar_animating() {
            self.scrollbar_shown_at = None;
            return true;
        }
        false
    }

    /// True while the story pane's scrollbar still has a change coming — it is
    /// holding before its fade, or fading — so the run loop keeps drawing frames
    /// without input. False once it has settled (visible forever, or gone).
    pub fn transcript_scrollbar_animating(&self) -> bool {
        let anim = &self.config.animation;
        if anim.scrollbar_hide_ms == 0 {
            return false;
        }
        let fade_ms = if anim.enabled { anim.scrollbar_fade_ms } else { 0 };
        self.scrollbar_shown_at
            .is_some_and(|t| (t.elapsed().as_millis() as u64) < anim.scrollbar_hide_ms + fade_ms)
    }

    /// The transcript offset to render this frame: the animated displayed offset
    /// (line-rounded) while a smooth scroll is in flight, else the logical target.
    /// Still clamped to `[0, max_scroll]` by the renderer.
    pub fn effective_transcript_scroll(&self) -> u16 {
        self.scroll_anim
            .as_ref()
            .map(|a| a.current().round() as u16)
            .unwrap_or(self.transcript_scroll)
    }

    /// Return true if any modal, dialog, or overlay is currently open — including
    /// the CORNER overlay (the tidy animation) that lets you keep playing
    /// underneath it.
    ///
    /// The room dock is deliberately NOT here (SQ-0692). It replaced two floating
    /// corner dialogs with a docked panel that reserves its own rows out of the
    /// map pane: it covers nothing, swallows nothing, and stays up while you
    /// play, so counting it as an overlay would suppress the story prompt for as
    /// long as it is open.
    ///
    /// For anything about the story pane's live input — the line, its caret, its
    /// suggestions — use [`any_modal_overlay_open`](Self::any_modal_overlay_open)
    /// instead: a modal genuinely means you are not typing at the prompt, but a
    /// corner panel does not.
    pub fn any_overlay_open(&self) -> bool {
        self.any_modal_overlay_open() || self.tidy_anim.is_some()
    }

    /// [`any_overlay_open`](Self::any_overlay_open) minus the corner overlays.
    ///
    /// The tidy animation lives in the MAP pane and deliberately does not swallow
    /// input, so it must not suppress the story pane's live input line or caret —
    /// doing so hid a half-typed command with no sign it was still buffered, and
    /// Enter would then run something the player could not see.
    pub fn any_modal_overlay_open(&self) -> bool {
        self.overlays.saves.is_some()
            || self.overlays.file_browser.is_some()
            || self.overlays.file_picker.is_some()
            || self.overlays.config_screen.is_some()
            // NOT the command band (SQ-0664): it is a dock, not dialog chrome.
            // Counting it here is what made the old verb menu degrade the
            // session — it hid the story prompt line, swallowed paste, blocked
            // v6/Glk click delivery, and dropped graphical v6 off the pixel
            // path for as long as it was open.
            || self.overlays.palette.is_some()
            || self.overlays.hotkey_dialog
            || self.overlays.text_entry.is_some()
            || self.overlays.confirm_delete_save.is_some()
            || self.overlays.confirm_overwrite_save.is_some()
            || self.overlays.fetch_keep.is_some()
            || self.overlays.reset_dialog
            || self.overlays.game_over
            || self.overlays.save_name_dialog.is_some()
            || self.overlays.history_prompt
            || self.overlays.font_check
            || self.overlays.aux_prompt
            || self.overlays.quit_dialog
            || self.overlays.launch_dialog
            || self.overlays.hints.is_some()
            || self.overlays.replay.is_some()
            || self.overlays.region_prompt.is_some()
            || self.resize_mode
    }

    /// Note a v6 save-time display-list diagnostic for `/dump-windows` (SQ-0588).
    ///
    /// The save-time self-check replays each window's ops and compares against the
    /// live canvas; a window that fails names itself here. That is the whole reason
    /// the archive stores ops rather than pixels — a pixel snapshot would restore
    /// correctly and leave the recording gap invisible until it surfaced later as
    /// missing or mis-coloured art. `&self` because the auto-save paths hold an
    /// immutable `AppState` (same reason as `note_v6_path`).
    pub fn note_v6_save(&self, msg: &str) {
        let mut log = self.v6_save_log.borrow_mut();
        if log.iter().any(|m| m == msg) {
            return; // one line per distinct problem, not one per save
        }
        log.push(msg.to_string());
        if log.len() > 8 {
            log.remove(0);
        }
    }

    /// Note one v6 render path for the `/dump-windows` history (SQ-0587). Consecutive
    /// repeats collapse into a count, and only the last few distinct runs are kept.
    pub fn note_v6_path(&self, label: &str) {
        let mut log = self.v6_path_log.borrow_mut();
        match log.last_mut() {
            Some((last, n)) if last == label => *n += 1,
            _ => {
                log.push((label.to_string(), 1));
                if log.len() > 6 {
                    log.remove(0);
                }
            }
        }
    }

    /// The frame `/dump-windows` describes, and the line that says which frame that
    /// is (SQ-0756).
    ///
    /// `None` means no game frame has been recorded — the placements and the ring
    /// plan are then genuinely unknown, and the caller must report them as
    /// unavailable rather than fall back to the modal frame standing in
    /// `v6_cell_map`, which is the defect this exists to fix.
    pub fn v6_dump_frame(&self) -> (Option<V6GameFrame>, String) {
        match self.v6_last_game_frame.borrow().clone() {
            Some(f) => {
                let age = match f.modal_frames_since {
                    0 => "the frame on screen now".to_string(),
                    n => format!("{n} modal frame(s) ago — NOT the palette/dialog frame this command runs in"),
                };
                (Some(f), format!("  frame described: the last frame the game drew, {age}"))
            }
            None => (
                None,
                "  frame described: none — no frame has been drawn without a modal overlay up, so \
                 the per-window cells, the story viewport and the ring plan are UNAVAILABLE"
                    .to_string(),
            ),
        }
    }

    /// Close a story-pane frame: if the GAME drew it, keep its mapping for
    /// `/dump-windows` (SQ-0756).
    ///
    /// Called once per frame by `render_story_pane`. A frame drawn under a modal
    /// overlay is the command's own, not the game's, so it only ages the snapshot —
    /// the count it bumps is what the dump reports as "modal frames since". Frames
    /// that recorded no v6 mapping at all (every non-v6 game) are not frames of the
    /// kind this describes and leave it alone.
    ///
    /// `&self` because the render pass holds an immutable `AppState` (same reason as
    /// [`note_v6_path`](Self::note_v6_path)).
    pub fn note_v6_frame_end(&self) {
        let cells = self.v6_cell_map.borrow();
        if cells.is_empty() {
            return;
        }
        let mut slot = self.v6_last_game_frame.borrow_mut();
        if self.any_modal_overlay_open() {
            if let Some(f) = slot.as_mut() {
                f.modal_frames_since = f.modal_frames_since.saturating_add(1);
            }
            return;
        }
        *slot = Some(V6GameFrame {
            cells: cells.clone(),
            ring_plan: self.v6_ring_plan.get(),
            ring_clip: self.v6_ring_clip.get(),
            modal_frames_since: 0,
        });
    }

    /// Close a rendered frame: if no modal covered it, keep its CELLS for
    /// `/dump-cells` (SQ-0761).
    ///
    /// Called once per frame from the terminal draw closure, at the point every
    /// widget and the whole overlay ladder have finished writing — so what is kept
    /// is the frame as the terminal received it, not an intermediate state.
    ///
    /// A modal frame only ages the snapshot, exactly as [`note_v6_frame_end`] ages
    /// its own: a dump taken from the palette must describe the game's frame, not
    /// the palette's. Bound to a key no modal ever opens and this is simply the
    /// frame on screen.
    ///
    /// The v6 render path and this frame's art placements ride along, because both
    /// are rebuilt by the next frame and the grid is far easier to read when the
    /// regions an image covers are named beside it.
    ///
    /// `&self` for the same reason as [`note_v6_frame_end`]: the render pass holds
    /// an immutable `AppState`.
    pub fn note_frame_cells(&self, buf: &ratatui::buffer::Buffer) {
        let mut slot = self.last_frame_cells.borrow_mut();
        if self.any_modal_overlay_open() {
            if let Some(f) = slot.as_mut() {
                f.modal_frames_since = f.modal_frames_since.saturating_add(1);
            }
            return;
        }
        let map = self.v6_cell_map.borrow();
        let path = map.iter().find(|e| e.label.starts_with("path:")).map(|e| e.label.clone());
        // An art strip is an uploaded image: under kitty it draws ABOVE the cells,
        // so a region it covers can hold intact text that nobody can see (SQ-0747).
        // Naming those rects beside the grid is what keeps such a region from
        // reading as ordinary cells.
        let images = map
            .iter()
            .filter(|e| e.label.contains("art") && !e.label.contains("rasterised"))
            .map(|e| crate::cell_dump::ImagePlacement { label: e.label.clone(), rect: e.cells })
            .collect();
        *slot = Some(crate::cell_dump::FrameCells {
            buf: buf.clone(),
            modal_frames_since: 0,
            path,
            images,
        });
    }

    /// The modal overlays currently open, by name (SQ-0587). A v6 story drops its
    /// pixel path while one is up, so `/dump-windows` can say WHICH — "the ring did
    /// not run" is only half an answer.
    pub fn open_modal_overlays(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.overlays.saves.is_some() { v.push("saves"); }
        if self.overlays.file_browser.is_some() { v.push("file_browser"); }
        if self.overlays.file_picker.is_some() { v.push("file_picker"); }
        if self.overlays.config_screen.is_some() { v.push("config_screen"); }
        if self.overlays.palette.is_some() { v.push("palette"); }
        if self.overlays.hotkey_dialog { v.push("hotkey_dialog"); }
        if self.overlays.text_entry.is_some() { v.push("text_entry"); }
        if self.overlays.confirm_delete_save.is_some() { v.push("confirm_delete_save"); }
        if self.overlays.confirm_overwrite_save.is_some() { v.push("confirm_overwrite_save"); }
        if self.overlays.fetch_keep.is_some() { v.push("fetch_keep"); }
        if self.overlays.reset_dialog { v.push("reset_dialog"); }
        if self.overlays.game_over { v.push("game_over"); }
        if self.overlays.save_name_dialog.is_some() { v.push("save_name_dialog"); }
        if self.overlays.history_prompt { v.push("history_prompt"); }
        if self.overlays.font_check { v.push("font_check"); }
        if self.overlays.aux_prompt { v.push("aux_prompt"); }
        if self.overlays.quit_dialog { v.push("quit_dialog"); }
        if self.overlays.launch_dialog { v.push("launch_dialog"); }
        if self.overlays.hints.is_some() { v.push("hints"); }
        if self.overlays.replay.is_some() { v.push("replay"); }
        if self.resize_mode { v.push("resize_mode"); }
        v
    }

    /// Set the explicit layer override. `None` means follow the current room's layer.
    pub fn set_viewed_layer(&mut self, layer: Option<LayerId>) {
        self.viewed_layer = layer;
    }

    /// The layer the map pane must draw THIS frame, for the graph it is drawing.
    ///
    /// A tidy animation is the case [`active_layer`](Self::active_layer) cannot serve. Its frames
    /// are `layer_subgraph`s, and a subgraph reports `layers()` as main-only whatever layer it
    /// holds — so `active_layer` finds `viewed_layer` "absent", falls back to `MAIN_LAYER`, and the
    /// map draws every room of a layer the subgraph has none of: blank. The animation therefore
    /// states its own layer rather than letting the frame be asked (SQ-0359).
    ///
    /// `replay` takes precedence over an animation, matching the map pane's own order.
    pub fn frame_layer(&self, live: &MapGraph, replay: Option<&MapGraph>) -> LayerId {
        if let Some(g) = replay {
            return self.active_layer(g);
        }
        match &self.tidy_anim {
            Some(anim) => anim.layer,
            None => self.active_layer(live),
        }
    }

    /// Return the layer to render the map with.
    /// Priority: `viewed_layer` (if set and still present), else the current room's layer, else `MAIN_LAYER`.
    pub fn active_layer(&self, graph: &MapGraph) -> LayerId {
        use mapper::layer::MAIN_LAYER;
        if let Some(l) = self.viewed_layer {
            if graph.layers().contains_key(&l) {
                return l;
            }
        }
        graph.current().map(|id| graph.layer_of(id)).unwrap_or(MAIN_LAYER)
    }

    /// Borrow the live map's routed render model for `layer`. The routing in
    /// `render_layer` is the dominant map cost, so it is kept OFF the main thread
    /// (SQ-0379): the very first model is built synchronously (small, at game
    /// start), but every later rebuild — triggered by a `graph_gen` change —
    /// happens on a background worker while this keeps returning the last-ready
    /// model. Only the current-room highlight and labels are refreshed here, live
    /// and cheaply, so the highlight follows the player with no re-route (SQ-0378).
    ///
    /// Only the live map uses this — replay and tidy-animation graphs are not
    /// tracked by `graph_gen`, so their models are built fresh each frame. (SQ-0305)
    pub fn cached_map_render(
        &self,
        layer: LayerId,
        graph: &mapper::graph::MapGraph,
    ) -> std::cell::Ref<'_, mapper::render::RenderMap> {
        let gen = self.graph_gen;
        let fresh = matches!(self.map_render.borrow().as_ref(), Some(c) if c.gen == gen && c.layer == layer);
        if !fresh {
            // No routing ever runs on the main thread (SQ-0379). If there is no
            // model yet, seed an empty one so the pane can draw (blank) this frame;
            // otherwise keep drawing the stale model. Either way the real re-route
            // runs on the background worker, with the pulse + step overlay showing.
            if self.map_render.borrow().is_none() {
                *self.map_render.borrow_mut() = Some(MapRenderCache {
                    gen,
                    layer,
                    rm: mapper::render::render(&mapper::graph::MapGraph::new()),
                });
                // A new model means new derived tables (SQ-1182).
                *self.map_derived.borrow_mut() = None;
            }
            self.spawn_render_job(layer, graph, gen);
        }
        // Live, cheap refresh of the per-move-changeable fields on whatever model
        // is currently shown — no re-route (SQ-0378).
        if let Some(c) = self.map_render.borrow_mut().as_mut() {
            let current = graph.current();
            for r in &mut c.rm.rooms {
                r.is_current = Some(r.id) == current;
                if let Some(room) = graph.room(r.id) {
                    // Compare before writing: labels rarely change, and this
                    // runs per room per frame — the unconditional to_string was
                    // one allocation per room per drawn frame (SQ-1182).
                    if r.label != room.label() {
                        r.label = room.label().to_string();
                    }
                }
            }
        }
        std::cell::Ref::map(self.map_render.borrow(), |c| {
            &c.as_ref().expect("populated above").rm
        })
    }

    /// True when the map pane draws the MATRIX for the layer it is showing (SQ-0666).
    ///
    /// The matrix is built from the graph at draw time — rooms, edges, `tried` — and owes nothing
    /// to the routed layout model or to the background jobs that produce it.
    pub fn map_shows_matrix(&self, graph: &mapper::graph::MapGraph) -> bool {
        graph.layer_view(self.active_layer(graph)) == mapper::layer::MapView::Matrix
    }

    /// The routed model for the LIVE map this frame, or `None` when the pane is showing the
    /// matrix and there is no map to route (SQ-0671).
    ///
    /// This is the churn the player saw as a cycling map pane: [`AppState::cached_map_render`]
    /// re-routes whenever `graph_gen` moves, and every arriving tidy result bumps that generation,
    /// so a background job for ANY layer spawned a render worker whose in-flight pulse restyled
    /// the border of a pane drawing a table it never touched. Asking for no model at all is what
    /// makes the matrix independent of the layout pipeline, rather than merely ignoring its
    /// output.
    pub fn live_map_render(
        &self,
        layer: LayerId,
        graph: &mapper::graph::MapGraph,
    ) -> Option<std::cell::Ref<'_, mapper::render::RenderMap>> {
        if self.map_shows_matrix(graph) {
            return None;
        }
        Some(self.cached_map_render(layer, graph))
    }

    /// Spawn the background map-render worker for `(gen, layer)` unless one is
    /// already in flight (coalesced — like the tidy worker). The worker routes a
    /// clone of the graph and reports each phase into `render_steps`. (SQ-0379)
    fn spawn_render_job(&self, layer: LayerId, graph: &mapper::graph::MapGraph, gen: u64) {
        let mut job = self.render_job.borrow_mut();
        if job.is_some() {
            // A job is already running; let it finish. `poll_render_job` discards
            // it if it turns out stale, and the next frame respawns for `gen`.
            return;
        }
        let g = graph.clone();
        let steps = self.render_steps.clone();
        if let Ok(mut s) = steps.lock() {
            s.clear();
        }
        let handle = std::thread::spawn(move || {
            let mut push = |name: &str| {
                if let Ok(mut s) = steps.lock() {
                    s.push(name.to_string());
                }
            };
            mapper::render::render_layer_traced(&g, layer, &mut push)
        });
        *job = Some(RenderJob { handle, layer, gen, started: std::time::Instant::now() });
    }

    /// Poll the background map-render worker: if it has finished, install its
    /// model as the new last-ready render (when its generation still matches) or
    /// discard it as stale (a fresh job then spawns on the next draw). Returns
    /// true when a completed job was handled (the caller should redraw). (SQ-0379)
    pub fn poll_render_job(&mut self) -> bool {
        let done = self
            .render_job
            .borrow()
            .as_ref()
            .is_some_and(|j| j.handle.is_finished());
        if !done {
            return false;
        }
        let job = self.render_job.borrow_mut().take().expect("checked above");
        match job.handle.join() {
            Ok(rm) => {
                if job.gen == self.graph_gen {
                    if self.config.trace.map {
                        let steps = self.render_steps_snapshot();
                        write_map_trace(&self.config.user_dir, &steps, true);
                    }
                    *self.map_render.borrow_mut() =
                        Some(MapRenderCache { gen: job.gen, layer: job.layer, rm });
                    // The derived tables described the model this replaces — and
                    // the first real route lands at the SAME `(gen, layer)` as
                    // the empty placeholder it supersedes, so the key alone
                    // cannot tell them apart (SQ-1182).
                    *self.map_derived.borrow_mut() = None;
                    if let Ok(mut s) = self.render_steps.lock() {
                        s.clear();
                    }
                }
                // else: stale — a newer geometry arrived mid-build; drop it and
                // let the next draw respawn for the current generation.
            }
            Err(_) => {
                // Worker panicked: keep the last-ready model, drop the trace.
                if let Ok(mut s) = self.render_steps.lock() {
                    s.clear();
                }
            }
        }
        true
    }

    /// True while the background map-render worker (SQ-0379) is in flight.
    pub fn map_render_in_flight(&self) -> bool {
        self.render_job.borrow().is_some()
    }

    /// Poll the background v6 raster encode (SQ-0469): install a completed
    /// protocol and report whether a redraw is warranted. Called once per loop
    /// tick so an off-thread encode surfaces within a poll interval.
    pub fn poll_v6_encode_job(&self) -> bool {
        self.graphics_render.borrow_mut().poll_v6_job()
    }

    /// How long the in-flight background map job (tidy relayout or render worker)
    /// has been running, for the border-pulse phase — `None` when neither runs.
    ///
    /// Also `None` while the pane is showing the matrix (SQ-0671). The pulse announces "the
    /// layout you are looking at is being rebuilt"; over a table read straight from the graph it
    /// announces nothing, and a job for some other layer finishing mid-turn made the map pane's
    /// border cycle red/green while the player walked a maze.
    pub fn map_job_pulse_elapsed(
        &self,
        graph: &mapper::graph::MapGraph,
    ) -> Option<std::time::Duration> {
        if self.map_shows_matrix(graph) {
            return None;
        }
        self.tidy_job
            .as_ref()
            .map(|j| j.started.elapsed())
            .or_else(|| self.render_job.borrow().as_ref().map(|j| j.started.elapsed()))
    }

    /// Snapshot the in-flight render worker's phase trace, for the map's top-right
    /// progress overlay (SQ-0379).
    pub fn render_steps_snapshot(&self) -> Vec<String> {
        self.render_steps.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Advance keyboard focus one step (Tab). See [`cycle_focus`].
    pub fn toggle_focus(&mut self) {
        self.cycle_focus(true);
    }

    /// Cycle keyboard focus one step forward (`forward = true`, Tab) or back
    /// (Shift-Tab). The stops are **per window**, not per sub-tab: the story
    /// pane, then — when the debug inspector is open — each of its windows in
    /// turn (story → debug 0 → 1 → 2 → story). With the inspector closed there
    /// is nowhere else to go and Tab does nothing.
    ///
    /// The map pane is deliberately NOT a stop (SQ-0599). It used to be, and
    /// that made the same keystroke mean two different things depending on a
    /// focus state with no obvious on-screen cue — press an arrow and you were
    /// either editing the command line or panning the map, with nothing to say
    /// which. The map is now driven entirely modelessly: Shift+Arrow pans and
    /// the mouse does the rest, from wherever you are.
    pub fn cycle_focus(&mut self, forward: bool) {
        // Focus stops after the story pane (position 0) — the inspector's
        // windows, and nothing else.
        let extra = if self.debug.is_some() {
            crate::debug_panel::WINDOW_TABS.len() // one stop per debug window
        } else {
            0
        };
        let total = extra + 1;
        let cur = match self.focus {
            Focus::Game => 0,
            Focus::Map => 1 + self.debug.as_ref().map_or(0, |p| p.focus),
        };
        let next = if forward { (cur + 1) % total } else { (cur + total - 1) % total };
        if next == 0 {
            self.focus = Focus::Game;
        } else {
            self.focus = Focus::Map;
            if let Some(p) = &mut self.debug {
                p.focus = next - 1;
            }
        }
    }

    /// Toggle the map panel on (`Split`) / off (`TranscriptFull`).
    ///
    /// No longer touches focus: since SQ-0599 the map is not a focus stop at
    /// all, so hiding it cannot strand the keyboard on a hidden pane (the
    /// SQ-0333 hazard this used to guard against).
    pub fn toggle_map(&mut self) {
        self.layout = match self.layout {
            Layout::Split => Layout::TranscriptFull,
            Layout::TranscriptFull => Layout::Split,
        };
    }

    /// Which panes are currently visible and eligible for resize mode, in
    /// Tab-cycle order: StoryMap (Split layout only), InvDock (inventory
    /// shown), CommandBand (band open).
    ///
    /// The band is only a resize target while it is open — resize mode preempts
    /// the band's key intercept, so the two can be active at once (SQ-0238). Its
    /// height (`command_band.height`) is also a persisted, resettable config
    /// value.
    pub fn resize_targets_visible(&self) -> Vec<ResizeTarget> {
        let mut targets = Vec::new();
        if self.layout == Layout::Split {
            targets.push(ResizeTarget::StoryMap);
        }
        if self.show_inventory {
            targets.push(ResizeTarget::InvDock);
        }
        if self.overlays.command_band.is_some() {
            targets.push(ResizeTarget::CommandBand);
        }
        // The room dock is carved out of the map pane, so it is only a target
        // when the map is on screen AND the dock is open (SQ-0692).
        if self.room_dock.open && self.layout == Layout::Split && self.debug.is_none() {
            targets.push(ResizeTarget::RoomDock);
        }
        targets
    }

    /// Move `resize_target` to the next (`forward`) or previous visible target,
    /// wrapping. Snaps to the first visible target if the current one isn't
    /// visible; leaves it unchanged if nothing is visible.
    pub fn cycle_resize_target(&mut self, forward: bool) {
        let targets = self.resize_targets_visible();
        let Some(first) = targets.first() else { return };
        match targets.iter().position(|t| *t == self.resize_target) {
            Some(idx) => {
                let n = targets.len();
                let next = if forward { (idx + 1) % n } else { (idx + n - 1) % n };
                self.resize_target = targets[next];
            }
            None => self.resize_target = *first,
        }
    }

    /// Mirror the live pane sizes into `config`, the persisted source of truth.
    ///
    /// The one place the three runtime sizes meet their config keys, so resize
    /// mode's arrows and the mouse drag (SQ-0669) persist through identical
    /// paths; the actual write still waits for `pending_config_write`.
    pub fn sync_pane_sizes_to_config(&mut self) {
        self.config.split_ratio = self.pane_sizes.split_ratio;
        self.config.command_band.height = self.pane_sizes.band_height;
        self.config.inv_dock_pct = self.pane_sizes.inv_dock_pct;
        self.config.room_dock_pct = self.pane_sizes.room_dock_pct;
    }

    /// Reset all pane sizes to their config defaults and mirror into `config`.
    pub fn reset_pane_sizes(&mut self) {
        self.pane_sizes = PaneSizes {
            split_ratio: crate::config::default_split_ratio(),
            band_height: crate::config::default_band_height(),
            inv_dock_pct: crate::config::default_inv_dock_pct(),
            room_dock_pct: crate::config::default_room_dock_pct(),
        };
        self.sync_pane_sizes_to_config();
    }

    /// Whether `b` should draw with the resize accent: the boundary being
    /// dragged, or — when nothing is held — the one under the pointer (SQ-0669).
    pub fn boundary_active(&self, b: crate::layout::Boundary) -> bool {
        match self.pane_drag {
            Some(d) => d.boundary == b,
            None => self.pane_hover == Some(b),
        }
    }

    /// Zoom in one VISIBLE step (toward Boxes). Already at Boxes → no change; it is the most
    /// detailed view there is.
    pub fn zoom_in(&mut self) {
        self.zoom_to(match self.zoom {
            Zoom::Overview => Zoom::Compact,
            Zoom::Compact | Zoom::Boxes => Zoom::Boxes,
        });
    }

    /// Zoom out one VISIBLE step (toward Overview). Already at Overview → no change.
    pub fn zoom_out(&mut self) {
        self.zoom_to(match self.zoom {
            Zoom::Boxes => Zoom::Compact,
            Zoom::Compact | Zoom::Overview => Zoom::Overview,
        });
    }

    /// Zoom by `n` VISIBLE steps: positive in (toward Boxes), negative out (SQ-0355).
    ///
    /// Repeats the one-step move rather than doing arithmetic on the fine level, so `zoom-map 5`
    /// clamps at the most detailed view exactly as five presses of `+` would — the command and the
    /// key cannot drift apart.
    pub fn zoom_by(&mut self, n: i32) {
        for _ in 0..n.unsigned_abs() {
            if n > 0 {
                self.zoom_in();
            } else {
                self.zoom_out();
            }
        }
    }

    /// Jump to `z`, landing in the MIDDLE of its fine band (SQ-0350).
    ///
    /// A keypress must move the map, and only whole bands are visible: the nine fine levels
    /// collapse to three views (0–2 Overview, 3–5 Compact, 6–8 Boxes). Stepping one fine level per
    /// press meant the default (level 7, mid-Boxes) needed TWO presses of `-` before anything
    /// happened, and `+` did nothing at all, ever — 7→8 is still Boxes. That is the whole of "the
    /// zoom keys are not responsive": the keys always fired, they just moved a counter nobody
    /// could see.
    ///
    /// Landing mid-band rather than on its edge keeps `+` and `-` exact inverses, and leaves the
    /// wheel (which still steps one fine level at a time, see `zoom_in_fine`) equal room either way
    /// before it tips into the next view.
    fn zoom_to(&mut self, z: Zoom) {
        self.zoom_level = match z {
            Zoom::Overview => 1,
            Zoom::Compact => 4,
            Zoom::Boxes => 7,
        };
        self.zoom = z;
    }

    /// Zoom in one FINE step (toward Boxes). Clamps at level 8.
    ///
    /// The wheel's step, not the keyboard's: fine levels exist so a fast ctrl+scroll cannot skip
    /// straight past Compact. A keypress uses `zoom_in`, which moves a whole band.
    pub fn zoom_in_fine(&mut self) {
        self.zoom_level = self.zoom_level.saturating_add(1).min(8);
        self.zoom = zoom_from_level(self.zoom_level);
    }

    /// Zoom out one FINE step (toward Overview). Clamps at level 0. See `zoom_in_fine`.
    pub fn zoom_out_fine(&mut self) {
        self.zoom_level = self.zoom_level.saturating_sub(1);
        self.zoom = zoom_from_level(self.zoom_level);
    }

    /// Reset zoom to the default level (7 = Boxes) and clear char_pan.
    pub fn zoom_reset(&mut self) {
        self.zoom_level = 7;
        self.zoom = Zoom::Boxes;
        self.char_pan = (0, 0);
    }

    /// Pan the map scroll by (dx, dy).
    pub fn pan(&mut self, dx: i32, dy: i32) {
        self.scroll = (self.scroll.0 + dx, self.scroll.1 + dy);
    }

    /// Set scroll so that `cell` is centered in a pane of size `pane_w` × `pane_h`.
    ///
    /// `pane_w` and `pane_h` are in terminal characters; this method converts
    /// them to map-grid cells using the current zoom step before centering,
    /// so that `scroll` stays in cell units (matching `cell_to_screen`).
    ///
    /// For Boxes zoom the non-uniform layout places rooms at roughly
    /// `(BOX_W + MIN_GUTTER)` × `(BOX_H + MIN_GUTTER)` pixels per cell, which
    /// is smaller than `zoom.steps()` (19×11). Using the actual cell footprint
    /// keeps the target room near the pane centre rather than at the top edge.
    pub fn recenter_on(&mut self, cell: (i32, i32), pane_w: u16, pane_h: u16) {
        use crate::render::map::{BOX_W, BOX_H, MIN_GUTTER};
        let (sw, sh) = match self.zoom {
            Zoom::Boxes => (BOX_W + MIN_GUTTER, BOX_H + MIN_GUTTER), // 13 × 7
            _ => self.zoom.steps(),
        };
        let cells_w = (pane_w as i32 / sw).max(1);
        let cells_h = (pane_h as i32 / sh).max(1);
        self.scroll = (cell.0 - cells_w / 2, cell.1 - cells_h / 2);
        // Reset char-granular pan offset when re-centering the view.
        self.char_pan = (0, 0);
    }

    /// Return the indices (into `self.transcript`) of entries that pass the active
    /// `transcript_filter`, in order. `Both` returns all indices; `Story`/`Meta`
    /// return only indices whose kind matches. Defensively tolerates any length
    /// mismatch between `transcript` and `transcript_kinds` by defaulting to `Story`.
    pub fn visible_transcript_indices(&self) -> Vec<usize> {
        self.visible_transcript_indices_from(0)
    }

    /// [`visible_transcript_indices`](Self::visible_transcript_indices) over the
    /// source lines from `from` onwards.
    ///
    /// The filter is decided per line from that line's own kind, so the visible
    /// indices of a suffix are exactly the tail of the whole transcript's — which
    /// is what lets the wrap cache wrap only the lines that just arrived
    /// (SQ-1034). Anything that made the filter depend on a line's NEIGHBOURS
    /// would break that silently, so it lives here rather than being open-coded.
    pub fn visible_transcript_indices_from(&self, from: usize) -> Vec<usize> {
        (from.min(self.transcript.len())..self.transcript.len())
            .filter(|&i| {
                let kind = self.transcript_kinds.get(i).copied().unwrap_or(TranscriptKind::Story);
                transcript_filter_matches(self.transcript_filter, kind)
            })
            .collect()
    }

    /// Split `text` on `'\n'` and append each line to the transcript, tagged as `Story`.
    /// Record a screen-clear boundary (game `erase_window`, ZMSD §8.7.3) at the
    /// current end of the transcript, WITHOUT deleting scrollback. The renderer
    /// pins post-clear output to the top of a fresh screen (blanks below) while
    /// everything above the boundary stays reachable by scrolling up — a
    /// scrollback-preserving "clear" rather than a destructive wipe. Also snaps
    /// the view to the bottom so the cleared screen is what's shown.
    pub fn mark_screen_clear(&mut self) {
        self.clear_anchor = Some(self.transcript.len());
        self.transcript_scroll = 0;
        self.scroll_anim = None;
    }

    /// Truncate the transcript — and every parallel sidecar vec — back to `len`,
    /// collapsing a menu-redraw reprint to a screen-clear boundary so consecutive
    /// reprints replace each other instead of piling up in scrollback. A no-op if
    /// `len` is already at/beyond the end. (SQ-0407)
    pub fn truncate_transcript(&mut self, len: usize) {
        if len >= self.transcript.len() {
            return;
        }
        self.transcript.truncate(len);
        self.transcript_kinds.truncate(len);
        self.transcript_styles.truncate(len);
        self.transcript_runs.truncate(len);
        self.transcript_para.truncate(len);
        self.transcript_images.truncate(len);
        self.touch_transcript(TranscriptEdit::Rewrote);
    }

    pub fn push_transcript(&mut self, text: &str) {
        self.push_transcript_kind(text, TranscriptKind::Story);
    }

    /// Whether app-internal transcript output (status/slash/save-restore messages,
    /// pushed via [`push_transcript_kind`](Self::push_transcript_kind) /
    /// [`push_transcript_styled`](Self::push_transcript_styled)) should be inserted
    /// just ABOVE a trailing game `>` prompt rather than appended after it. True in
    /// inline-prompt mode when the game's prompt is the last line, so these messages
    /// don't bury the prompt the caret sits at (SQ-0270). Game turn output goes
    /// through `push_transcript_runs`/`apply_transcript_elems` and is unaffected.
    fn insert_above_prompt_at(&self) -> Option<usize> {
        (!self.config.command_bar && self.last_transcript_line_is_story())
            .then(|| self.transcript.len().saturating_sub(1))
    }

    /// Record a mutation of the transcript vecs (content, kinds, runs, styles,
    /// para, or images). Call from every method that touches them. (SQ-0305)
    ///
    /// The argument is not decoration: it is the difference between the wrap
    /// cache extending its rows and throwing them away (SQ-1034). It is an
    /// argument rather than something inferred here BECAUSE it cannot be inferred
    /// here — "did anything before the old end move?" is not visible from a
    /// `&mut self` after the fact — and because a new mutator that has to pick one
    /// is a mutator whose author had to think about it. A wrong pick is still
    /// caught: the cache also fingerprints the last line it consumed
    /// ([`crate::render::wrap_cache::WrapContent`]), and every in-place mutator
    /// here touches that line.
    fn touch_transcript(&mut self, edit: TranscriptEdit) {
        self.transcript_gen = self.transcript_gen.wrapping_add(1);
        match edit {
            TranscriptEdit::Appended => {}
            TranscriptEdit::Rewrote => {
                self.transcript_edits = self.transcript_edits.wrapping_add(1);
                // An opaque rewrite is exactly what a repair cannot see through
                // (SQ-1179): whatever run of inserts preceded it no longer
                // accounts for everything that moved.
                self.transcript_tail_insert.set(None);
            }
            TranscriptEdit::Inserted { at, count } => {
                debug_assert!(count > 0, "an insert of zero lines is a no-op mischaracterized as Inserted");
                self.transcript_edits = self.transcript_edits.wrapping_add(1);
                let since_edits = self.transcript_edits - 1;
                let run = match self.transcript_tail_insert.take() {
                    Some(prev) => TailInsertRun { since_edits: prev.since_edits, min_at: prev.min_at.min(at) },
                    None => TailInsertRun { since_edits, min_at: at },
                };
                self.transcript_tail_insert.set(Some(run));
            }
        }
    }

    /// Bump the graph generation, invalidating the map render memo. Call after ANY
    /// mutation of the mapper graph or its layout/labels — rename/notes/relabel/nudge,
    /// tidy applies, room reassignment (restore/import/reset). The map render memo is
    /// keyed on this (`cached_map_render`), so a missed bump paints a STALE MAP.
    /// Double-bumping is harmless (`wrapping_add`); a missing bump is a wrong map. (SQ-0305)
    pub fn bump_graph_gen(&mut self) {
        self.graph_gen = self.graph_gen.wrapping_add(1);
    }

    /// Split `text` on `'\n'` and append each line to the transcript with the given kind tag.
    pub fn push_transcript_kind(&mut self, text: &str, kind: TranscriptKind) {
        self.touch_transcript(TranscriptEdit::Appended);
        self.transcript_styles.resize(self.transcript.len(), None); // self-heal alignment
        self.transcript_runs.resize(self.transcript.len(), Vec::new()); // self-heal alignment
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default()); // self-heal alignment
        self.transcript_images.resize(self.transcript.len(), None); // self-heal alignment
        for line in text.split('\n') {
            self.transcript.push(line.to_owned());
            self.transcript_kinds.push(kind);
            self.transcript_styles.push(None);
            self.transcript_runs.push(Vec::new());
            self.transcript_para.push(ParaFmt::default());
            self.transcript_images.push(None);
        }
    }

    /// Add app-internal output (a `[…]` status line, a slash-command dump, a
    /// save/restore/copy banner) to the transcript. Like [`push_transcript_kind`],
    /// but in inline-prompt mode it inserts the line(s) ABOVE a trailing game `>`
    /// prompt so the prompt the caret sits at is never buried (SQ-0270).
    pub fn push_transcript_internal(&mut self, text: &str, kind: TranscriptKind) {
        let base = self.insert_above_prompt_at();
        // Inline-prompt mode INSERTS above the trailing prompt, which moves a line
        // the wrap cache has already wrapped — but every line before that prompt
        // provably did not move, so this is `Inserted`, not the opaque `Rewrote`,
        // and the cache can repair through it instead of rebuilding (SQ-1179).
        let count = text.split('\n').count();
        self.touch_transcript(match base {
            Some(b) => TranscriptEdit::Inserted { at: b, count },
            None => TranscriptEdit::Appended,
        });
        self.transcript_styles.resize(self.transcript.len(), None);
        self.transcript_runs.resize(self.transcript.len(), Vec::new());
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default());
        self.transcript_images.resize(self.transcript.len(), None);
        for (k, line) in text.split('\n').enumerate() {
            match base {
                Some(b) => {
                    let idx = b + k;
                    self.transcript.insert(idx, line.to_owned());
                    self.transcript_kinds.insert(idx, kind);
                    self.transcript_styles.insert(idx, None);
                    self.transcript_runs.insert(idx, Vec::new());
                    self.transcript_para.insert(idx, ParaFmt::default());
                    self.transcript_images.insert(idx, None);
                }
                None => {
                    self.transcript.push(line.to_owned());
                    self.transcript_kinds.push(kind);
                    self.transcript_styles.push(None);
                    self.transcript_runs.push(Vec::new());
                    self.transcript_para.push(ParaFmt::default());
                    self.transcript_images.push(None);
                }
            }
        }
    }

    /// The visible transcript as a FILE should carry it (SQ-1045).
    ///
    /// Identical to what is on screen except for one thing: an assist gets its
    /// `Lanthorn: ` back. On screen the mark in the gutter says whose the line is;
    /// an exported file has no gutter, no colour and nothing a screen reader can
    /// voice, so the words are the only carrier left — and this is the surface the
    /// original all-lines-marked design was really about. See [`crate::assist`].
    pub fn transcript_for_export(&self) -> Vec<String> {
        self.visible_transcript_indices()
            .into_iter()
            .map(|i| match self.transcript_kinds.get(i) {
                Some(TranscriptKind::Assist) => crate::assist::export_line(&self.transcript[i]),
                _ => self.transcript[i].clone(),
            })
            .collect()
    }

    /// Say something to the player in the assist voice — Lanthorn's Guiding Light
    /// (SQ-1045).
    ///
    /// **This is the only door.** It honours the player's `guidance` switch, tags
    /// the line [`TranscriptKind::Assist`] so `/filter`, the exporter and the
    /// renderer can tell it from the story without reading it, resolves the tone's
    /// `style.toml` selector, and shows the once-per-session introduction above
    /// the first assist a session displays. A caller that assembles any of that
    /// itself has reinvented the register, which is the thing this quest exists to
    /// stop — `tests/suites/assist_voice.rs` fails a source file that tries.
    ///
    /// The introduction is built around the glyph actually in force
    /// ([`crate::symbols::SymbolSet::assist_gutter`], which is what the renderer
    /// draws in the gutter), because that sentence is also the glyph's self-test:
    /// a font without the mark shows a box in the one line that says what the box
    /// means and where to switch it off.
    ///
    /// See [`crate::assist`] for what an assist line may and may not say.
    /// A new player turn is starting: everything the previous one was still
    /// waiting on is now stale (SQ-1124).
    ///
    /// Called at the head of every turn finisher — a typed command, a resumed
    /// read, a game-driven turn — because "the player has moved on" is what
    /// makes a late offer wrong, and each of those three is a way for that to
    /// happen. Bumping twice for one turn would cost nothing; missing one prints
    /// a suggestion against the wrong command.
    pub fn begin_turn(&mut self) {
        self.turn_epoch = self.turn_epoch.wrapping_add(1);
        // A reveal is an answer about one moment, and the turn has just ended it
        // (SQ-1107). The keystroke path clears it too, so this is the exit for a
        // turn nobody typed — a timed read firing, a game-driven redraw — where
        // the words on screen would otherwise stay lit against a world that had
        // moved on.
        self.reveal = None;
    }

    pub fn push_assist(&mut self, assist: &crate::assist::Assist) {
        // The switch is checked HERE rather than at each of the five call sites,
        // for the reason everything else about the register is: a feature that
        // forgets to ask is a feature the player cannot turn off.
        if !self.config.guidance {
            return;
        }
        if !self.assist_preamble_shown {
            self.assist_preamble_shown = true;
            let intro = self.colors.theme.get(crate::assist::AssistTone::Help.selector()).style;
            let line = crate::assist::preamble(self.symbols.assist_gutter);
            self.push_transcript_internal_styled(&line, TranscriptKind::Assist, intro);
        }
        let style = self.colors.theme.get(assist.tone().selector()).style;
        for line in assist.lines() {
            self.push_transcript_internal_styled(&line, TranscriptKind::Assist, style);
        }
    }

    /// Surface an app-internal `[…]` bracketed notice as a top-right toast
    /// (SQ-0176). It slides in, holds a few seconds, and slides out — it is NOT
    /// written to the transcript; `/dump-notifications` replays the history.
    pub fn push_notice(&mut self, text: &str) {
        self.notifications.push(text);
    }

    /// Append `text` to the last transcript line in place (used in inline-prompt
    /// mode so the typed command joins the game's `>` line). If the transcript is
    /// empty, push `text` as a new line instead. Only the last line's String is
    /// edited; its parallel runs/kinds/images/styles are left as-is (the appended
    /// chars carry no style runs and render in the input style).
    pub fn append_to_last_transcript_line(&mut self, text: &str) {
        if self.transcript.is_empty() {
            self.push_transcript_kind(text, TranscriptKind::Input);
            return;
        }
        self.touch_transcript(TranscriptEdit::Rewrote);
        let start = self.transcript.last().unwrap().chars().count();
        self.transcript.last_mut().unwrap().push_str(text);
        let end = start + text.chars().count();
        // Inherit the colour of the line's trailing style run so the echoed command
        // is drawn in the game's prompt colours instead of the uncoloured theme base,
        // and the SQ-0263 background band stays continuous across it. Only fg/bg are
        // carried (not reverse/bold bits or a hyperlink). No coloured run → plain
        // append (the theme case), unchanged. (SQ-0269)
        if let Some(runs) = self.transcript_runs.last_mut() {
            if let Some(&StyleRun { fg, bg, .. }) = runs.last() {
                if fg != 0 || bg != 0 {
                    runs.push(StyleRun { start, end, bits: 0, fg, bg, link: 0, glk_style: 0 });
                }
            }
        }
    }

    /// Merge transcript line `idx` into line `idx - 1`, concatenating the text and
    /// appending `idx`'s style runs shifted by the previous line's length, then
    /// removing line `idx`. Used to fold a game's own command echo (the first line
    /// of its turn output) onto the `>` prompt line, preserving the game's styling
    /// (e.g. CounterfeitMonkey's bold echo) so it reads as `>look` at the prompt
    /// rather than on a detached line below (SQ-0274). A no-op for `idx == 0` or
    /// out of range. Assumes the parallel arrays are length-aligned (the push
    /// methods maintain this).
    pub fn merge_line_into_previous(&mut self, idx: usize) {
        if idx == 0 || idx >= self.transcript.len() {
            return;
        }
        self.touch_transcript(TranscriptEdit::Rewrote);
        let base = self.transcript[idx - 1].chars().count();
        let moved = std::mem::take(&mut self.transcript[idx]);
        self.transcript[idx - 1].push_str(&moved);
        let shifted: Vec<StyleRun> = self
            .transcript_runs
            .get(idx)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut r| {
                r.start += base;
                r.end += base;
                r
            })
            .collect();
        if let Some(prev) = self.transcript_runs.get_mut(idx - 1) {
            prev.extend(shifted);
        }
        self.transcript.remove(idx);
        if idx < self.transcript_kinds.len() {
            self.transcript_kinds.remove(idx);
        }
        if idx < self.transcript_styles.len() {
            self.transcript_styles.remove(idx);
        }
        if idx < self.transcript_runs.len() {
            self.transcript_runs.remove(idx);
        }
        if idx < self.transcript_para.len() {
            self.transcript_para.remove(idx);
        }
        if idx < self.transcript_images.len() {
            self.transcript_images.remove(idx);
        }
    }

    /// The fg/bg of the most recent coloured style run in transcript lines before
    /// `upto` (scanning lines then runs from the end), or `None` if none is set.
    /// Approximates the game's "current" colour state at that point so a Default
    /// (unset) colour can be resolved to it rather than reset to the theme.
    pub fn prevailing_run_colour_before(&self, upto: usize) -> Option<(u32, u32)> {
        let end = upto.min(self.transcript_runs.len());
        for runs in self.transcript_runs[..end].iter().rev() {
            if let Some(r) = runs.iter().rev().find(|r| r.fg != 0 || r.bg != 0) {
                return Some((r.fg, r.bg));
            }
        }
        None
    }

    /// For transcript line `idx`, resolve every Default (0) fg/bg channel — in
    /// existing runs and on chars that carry no run — to `fg`/`bg`, preserving
    /// style bits, links, and any colour the game DID set. Used so a folded
    /// self-echo (which a game like CounterfeitMonkey prints in the default
    /// colour) keeps the game's current page colours instead of resetting to the
    /// theme (SQ-0274). No-op when both `fg` and `bg` are Default.
    pub fn fill_line_default_colours(&mut self, idx: usize, fg: u32, bg: u32) {
        if (fg == 0 && bg == 0) || idx >= self.transcript.len() {
            return;
        }
        let len = self.transcript[idx].chars().count();
        if len == 0 {
            return;
        }
        self.touch_transcript(TranscriptEdit::Rewrote);
        // Resolve per char: (bits, fg, bg, link, glk_style). Unstyled chars take the fill.
        let mut per: Vec<(u8, u32, u32, u32, u8)> = vec![(0, fg, bg, 0, 0); len];
        for r in self.transcript_runs.get(idx).cloned().unwrap_or_default() {
            for item in per.iter_mut().take(r.end.min(len)).skip(r.start) {
                *item = (
                    r.bits,
                    if r.fg != 0 { r.fg } else { fg },
                    if r.bg != 0 { r.bg } else { bg },
                    r.link,
                    r.glk_style,
                );
            }
        }
        // Coalesce adjacent identical cells back into runs.
        let mut runs: Vec<StyleRun> = Vec::new();
        for (c, &(bits, cf, cb, link, gs)) in per.iter().enumerate() {
            match runs.last_mut() {
                Some(last) if last.end == c && last.bits == bits && last.fg == cf && last.bg == cb && last.link == link && last.glk_style == gs => {
                    last.end = c + 1;
                }
                _ => runs.push(StyleRun { start: c, end: c + 1, bits, fg: cf, bg: cb, link, glk_style: gs }),
            }
        }
        if idx < self.transcript_runs.len() {
            self.transcript_runs[idx] = runs;
        }
    }

    /// Whether the last transcript line is game (Story) output — i.e. the game's
    /// inline `>` prompt is the last line, so the typed command can be appended to
    /// it. False when a non-game line (e.g. a `/help` Meta dump) is last, in which
    /// case the inline echo must go on its own line rather than corrupt that text.
    pub fn last_transcript_line_is_story(&self) -> bool {
        matches!(self.transcript_kinds.last(), Some(TranscriptKind::Story))
    }

    /// Append lines with the given kind and an explicit per-line render style.
    pub fn push_transcript_styled(&mut self, text: &str, kind: TranscriptKind, style: ratatui::style::Style) {
        self.touch_transcript(TranscriptEdit::Appended);
        self.transcript_styles.resize(self.transcript.len(), None); // self-heal alignment
        self.transcript_runs.resize(self.transcript.len(), Vec::new()); // self-heal alignment
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default()); // self-heal alignment
        self.transcript_images.resize(self.transcript.len(), None); // self-heal alignment
        for line in text.split('\n') {
            self.transcript.push(line.to_owned());
            self.transcript_kinds.push(kind);
            self.transcript_styles.push(Some(style));
            self.transcript_runs.push(Vec::new());
            self.transcript_para.push(ParaFmt::default());
            self.transcript_images.push(None);
        }
    }

    /// Like [`push_transcript_styled`], but inserts app-internal styled output
    /// above a trailing game prompt in inline-prompt mode (SQ-0270).
    pub fn push_transcript_internal_styled(&mut self, text: &str, kind: TranscriptKind, style: ratatui::style::Style) {
        let base = self.insert_above_prompt_at();
        // See `push_transcript_internal`: an insert above the prompt is a
        // repairable `Inserted`, not the opaque `Rewrote` (SQ-1179).
        let count = text.split('\n').count();
        self.touch_transcript(match base {
            Some(b) => TranscriptEdit::Inserted { at: b, count },
            None => TranscriptEdit::Appended,
        });
        self.transcript_styles.resize(self.transcript.len(), None);
        self.transcript_runs.resize(self.transcript.len(), Vec::new());
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default());
        self.transcript_images.resize(self.transcript.len(), None);
        for (k, line) in text.split('\n').enumerate() {
            match base {
                Some(b) => {
                    let idx = b + k;
                    self.transcript.insert(idx, line.to_owned());
                    self.transcript_kinds.insert(idx, kind);
                    self.transcript_styles.insert(idx, Some(style));
                    self.transcript_runs.insert(idx, Vec::new());
                    self.transcript_para.insert(idx, ParaFmt::default());
                    self.transcript_images.insert(idx, None);
                }
                None => {
                    self.transcript.push(line.to_owned());
                    self.transcript_kinds.push(kind);
                    self.transcript_styles.push(Some(style));
                    self.transcript_runs.push(Vec::new());
                    self.transcript_para.push(ParaFmt::default());
                    self.transcript_images.push(None);
                }
            }
        }
    }

    /// Split `text` on `'\n'` and append each line tagged with `kind`, deriving a
    /// per-line `Vec<StyleRun>` from `chunks` — a `(char_count, bits, fg, bg)` list
    /// whose total char-count covers every char of `text` INCLUDING the `'\n'`
    /// separators (as recorded by `CaptureSink`). Adjacent equal-attribute spans
    /// merge; zero-bits/default-colour spans are omitted (so an unstyled line
    /// yields an empty run vec).
    ///
    /// A chunk's `nowrap` flag (printed while `buffer_mode` was off, ZMSD §7.2.1)
    /// is folded per line into [`ParaFmt::nowrap_from`] — the first char offset on
    /// that line that was emitted unbuffered — so the wrap can char-break from
    /// exactly there.
    pub fn push_transcript_runs(
        &mut self,
        text: &str,
        kind: TranscriptKind,
        chunks: &[crate::session::CaptureRun],
    ) {
        // A turn with no new lower-window output (e.g. a read_char keypress that
        // only redrew the upper window) yields an empty string; appending it would
        // add a spurious blank line (`"".split('\n')` yields one empty element),
        // scrolling the transcript up one row per keypress. Skip it.
        if text.is_empty() {
            return;
        }
        self.touch_transcript(TranscriptEdit::Appended);
        self.transcript_styles.resize(self.transcript.len(), None);
        self.transcript_runs.resize(self.transcript.len(), Vec::new());
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default());
        self.transcript_images.resize(self.transcript.len(), None);

        // Walk `text` char-by-char while consuming the chunk list in lockstep.
        let mut chunk_iter = chunks.iter().copied();
        let mut rem: usize = 0;
        let mut bits: u8 = 0;
        let mut fg = zvm::screen::ZColour::Default;
        let mut bg = zvm::screen::ZColour::Default;
        let mut link: u32 = 0;
        let mut para = ParaFmt::default();
        let mut glk_style: u8 = 0;
        let mut nowrap = false;
        // Advance to the next non-exhausted chunk when the current one is spent.
        // When chunks run out, treat the remainder as plain (bits 0, default
        // colours, no link, default layout, Normal style, buffered).
        let mut refill = |rem: &mut usize,
                          bits: &mut u8,
                          fg: &mut zvm::screen::ZColour,
                          bg: &mut zvm::screen::ZColour,
                          link: &mut u32,
                          para: &mut ParaFmt,
                          glk_style: &mut u8,
                          nowrap: &mut bool| {
            while *rem == 0 {
                match chunk_iter.next() {
                    Some((c, b, f, bk, lk, pf, gs, nw)) => {
                        *rem = c;
                        *bits = b;
                        *fg = f;
                        *bg = bk;
                        *link = lk;
                        *para = pf;
                        *glk_style = gs;
                        *nowrap = nw;
                    }
                    None => {
                        *rem = usize::MAX;
                        *bits = 0;
                        *fg = zvm::screen::ZColour::Default;
                        *bg = zvm::screen::ZColour::Default;
                        *link = 0;
                        *para = ParaFmt::default();
                        *glk_style = 0;
                        *nowrap = false;
                        break;
                    }
                }
            }
        };

        let mut first = true;
        for line in text.split('\n') {
            // Consume the '\n' separator's chunk char (one per separator, i.e.
            // before every line except the first).
            if !first {
                refill(&mut rem, &mut bits, &mut fg, &mut bg, &mut link, &mut para, &mut glk_style, &mut nowrap);
                rem = rem.saturating_sub(1);
            }
            first = false;

            // The paragraph's layout comes from the FIRST content run on the line
            // (a paragraph is normally one style). `None` until the first char.
            let mut line_para: Option<ParaFmt> = None;
            // First column on this line printed with buffering off (§7.2.1).
            let mut line_nowrap: Option<u32> = None;
            let mut runs: Vec<StyleRun> = Vec::new();
            for (col, _ch) in line.chars().enumerate() {
                refill(&mut rem, &mut bits, &mut fg, &mut bg, &mut link, &mut para, &mut glk_style, &mut nowrap);
                if line_para.is_none() {
                    line_para = Some(para);
                }
                if nowrap && line_nowrap.is_none() {
                    line_nowrap = Some(col as u32);
                }
                let pfg = pack_zcolour(fg);
                let pbg = pack_zcolour(bg);
                // A run is also emitted for a non-Normal Glk style with no other
                // styling, so the theme's per-style colour slot can apply at render
                // (SQ-0331). A Normal (glk_style 0) unstyled char still yields no run.
                let has_style = bits != 0 || pfg != 0 || pbg != 0 || link != 0 || glk_style != 0;
                if has_style {
                    match runs.last_mut() {
                        Some(r) if r.end == col && r.bits == bits && r.fg == pfg && r.bg == pbg && r.link == link && r.glk_style == glk_style => {
                            r.end = col + 1;
                        }
                        _ => runs.push(StyleRun { start: col, end: col + 1, bits, fg: pfg, bg: pbg, link, glk_style }),
                    }
                }
                rem = rem.saturating_sub(1);
            }

            self.transcript.push(line.to_owned());
            self.transcript_kinds.push(kind);
            self.transcript_styles.push(None);
            self.transcript_runs.push(runs);
            self.transcript_para.push(ParaFmt { nowrap_from: line_nowrap, ..line_para.unwrap_or_default() });
            self.transcript_images.push(None);
        }
    }

    /// [`push_transcript_runs`](Self::push_transcript_runs) for a `read_char` turn,
    /// keeping output the game printed WHERE THE CURSOR ALREADY WAS on the line it
    /// was already on, instead of opening a new one (SQ-0726, generalised by
    /// SQ-0804).
    ///
    /// Every push starts a new transcript line, which is how the host supplies the
    /// newline an interpreter echoes after a `read` (ZMSD §7.1.1.1): the typed
    /// command is appended to the game's `>` prompt and the reply opens the line
    /// below. `read_char` echoes nothing at all (§10.7), so a keypress turn inherits
    /// no newline and that implicit one is a fabrication. sunburst.z6 is the report:
    /// it has no line reader of its own — it runs `read_char` in a loop and
    /// `print_char`s each keystroke straight back — so the player's word arrived one
    /// character per transcript line, the first of them a line below the prompt.
    ///
    /// `continues` is [`Engine::output_continued_line`](crate::engine::Engine::output_continued_line):
    /// the game's own cursor, which is the only thing that knows. SQ-0726 shipped a
    /// stand-in — "the turn's whole output is one character, so it must be an echo"
    /// — because the printed text alone cannot decide it and folding every keypress
    /// turn was measured to move six other titles, joining Arthur's, Shogun's,
    /// Journey's, advent's and fmvpoker's menu repaints to the line above and
    /// concatenating four of mysterious01's re-asked prompts into one. Those games
    /// reposition between reprints, which the text does not show and the cursor
    /// does: swept across the v6 corpus the cursor rule leaves every one of them
    /// byte-identical and moves sunburst alone — onto the shape sunburst's own
    /// screen has, `>look.` on one line where the host used to break after `>look`.
    ///
    /// Two guards ride with it, and both are load-bearing:
    ///
    /// * text that OPENS with a newline is the game's own line break, so the split
    ///   is the game's and not the host's.
    /// * a fold needs a live line to fold onto. A last line that is not game output
    ///   (a `/help` dump, the host's own command echo) is left alone per
    ///   [`last_transcript_line_is_story`](Self::last_transcript_line_is_story), and
    ///   so is a transcript sitting exactly on a screen-clear boundary — everything
    ///   above `clear_anchor` belongs to a screen the game has wiped, and the cursor
    ///   agreeing there is a coincidence of both being at the window's origin.
    ///
    /// The fold is [`merge_line_into_previous`](Self::merge_line_into_previous) —
    /// the same call the game-self-echo path uses — so the moved text's style runs
    /// shift onto the line they join rather than being dropped.
    ///
    /// Returns whether the fold happened, which the `[more]` pager needs: a folded
    /// turn's output starts on a row that was already on screen, so that row is
    /// partly this turn's and the pager's baseline has to step back onto it. See
    /// [`app::pager`](crate::pager) — getting this wrong scrolled Arthur's hint
    /// pages one line too far (SQ-0823).
    pub fn push_transcript_runs_char_echo(
        &mut self,
        text: &str,
        kind: TranscriptKind,
        chunks: &[crate::session::CaptureRun],
        continues: bool,
    ) -> bool {
        let fold = continues
            && !text.starts_with('\n')
            && self.last_transcript_line_is_story()
            && self.clear_anchor != Some(self.transcript.len());
        let before = self.transcript.len();
        self.push_transcript_runs(text, kind, chunks);
        let folded = fold && self.transcript.len() > before;
        if folded {
            self.merge_line_into_previous(before);
        }
        folded
    }

    /// Append a logical image unit: an empty placeholder line tagged `Story`
    /// carrying an inline image, keeping the parallel Vecs length-synced.
    pub fn push_transcript_image(&mut self, img: crate::inline_image::InlineImage) {
        self.touch_transcript(TranscriptEdit::Appended);
        self.transcript_styles.resize(self.transcript.len(), None);
        self.transcript_runs.resize(self.transcript.len(), Vec::new());
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default());
        self.transcript_images.resize(self.transcript.len(), None);
        self.transcript.push(String::new());
        self.transcript_kinds.push(TranscriptKind::Story);
        self.transcript_styles.push(None);
        self.transcript_runs.push(Vec::new());
        self.transcript_para.push(ParaFmt::default());
        self.transcript_images.push(Some(img));
    }

    /// Reset the in-memory transcript sidecars (`transcript_styles`,
    /// `transcript_images`) to match a freshly reassigned `transcript`. Call this
    /// after replacing `transcript` wholesale (load / restore / reset / history
    /// jump). These sidecars carry no persisted data — the `.lanthorn` archive
    /// stores only lines/kinds/runs (see `archive::TranscriptData`) — so the
    /// correct post-reassignment state is all-`None`, sized to the new transcript.
    /// A plain `resize` cannot do this: it only truncates the tail, leaving stale
    /// `Some(..)` at retained head indices (e.g. an inline image a Glulx game drew
    /// before the load, now indexing a different, shorter transcript).
    pub fn reset_transcript_sidecars(&mut self) {
        self.touch_transcript(TranscriptEdit::Rewrote);
        self.transcript_styles = vec![None; self.transcript.len()];
        self.transcript_images = vec![None; self.transcript.len()];
        // The scraped word set is a sidecar too, and the most derived one of the
        // lot: it is the transcript read through the story's dictionary
        // (SQ-1135). A wholesale replacement — a restore, a resume — is exactly
        // the moment it stops describing what is on the page, so it is dropped
        // here and rebuilt from the NEW transcript by the next
        // `input::refresh_seen_words`. That is also what gives a restore its
        // per-save semantics: restoring to before a word was printed takes the
        // word away.
        self.seen_words.clear();
        self.seen_nouns.clear();
        self.seen_scanned = 0;
    }

    /// Surface a transient message as a top-right notification toast (SQ-0176).
    /// No longer reuses the score bar — the message slides in, holds a few
    /// seconds, and slides out; `/dump-notifications` replays the history.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.notifications.push(msg);
    }

    /// Set the selected room.
    pub fn select_room(&mut self, room: Option<RoomId>) {
        self.selected_room = room;
    }

    /// Note that the player just walked into `room`, for the maze breadcrumb (SQ-0666).
    ///
    /// A repeat of the room already at the head is dropped: a `look` or a failed move is not a
    /// step, and letting either one in would flush the trail with eight copies of where you are
    /// standing. Bounded at [`MAP_TRAIL_LEN`].
    pub fn push_trail(&mut self, room: RoomId) {
        if self.map_trail.back() == Some(&room) {
            return;
        }
        self.map_trail.push_back(room);
        while self.map_trail.len() > MAP_TRAIL_LEN {
            self.map_trail.pop_front();
        }
    }

    /// How far back in the trail `room` is: 0 for the room just entered, `None` when it is not on
    /// the trail at all. Drives the breadcrumb's fade.
    pub fn trail_age(&self, room: RoomId) -> Option<usize> {
        self.map_trail.iter().rev().position(|&r| r == room)
    }

    /// Insert a character at the caret.
    pub fn push_input_char(&mut self, c: char) {
        self.input.insert(c);
    }

    /// Delete the character before the caret, if any.
    pub fn backspace(&mut self) {
        self.input.backspace();
    }

    /// Return the current input line and clear it. Also clears autocomplete state.
    pub fn take_input(&mut self) -> String {
        self.suggestions.clear();
        self.suggestion_idx = 0;
        self.suggestion_active = false;
        self.input.take()
    }

    /// Char index in the input line under a click at screen `(col, row)`, or `None` when the click
    /// is not on that line (SQ-0354).
    ///
    /// A click past the last character clamps to the end, which is what every other text field
    /// does — clicking the empty space after a line puts the caret at its end.
    ///
    /// `col - x0` is a CELL offset, and a cell is not a char: a double-width glyph
    /// earlier in the line pushes everything after it one column right, so the offset
    /// has to be converted through the line's display widths or the caret lands short
    /// of the click (SQ-0655). Either cell of a wide glyph puts the caret before it.
    pub fn input_click_index(&self, col: u16, row: u16) -> Option<usize> {
        let (x0, y0) = self.input_text_origin.get()?;
        if row != y0 || col < x0 {
            return None;
        }
        let cell = (col - x0) as usize;
        Some(crate::textwidth::col_to_char_idx(self.input.as_str(), cell).min(self.input.char_len()))
    }

    /// Clear the current autocomplete suggestions.
    pub fn clear_suggestions(&mut self) {
        self.suggestions.clear();
        self.suggestion_idx = 0;
        self.suggestion_active = false;
    }

    /// Return the partial word the player is currently typing (the last
    /// whitespace-delimited token in `input`).
    pub fn current_partial(&self) -> &str {
        // Find the last space; if none, the whole input is the partial word.
        match self.input.value.rfind(' ') {
            Some(pos) => &self.input.value[pos + 1..],
            None => &self.input.value,
        }
    }

    // ── Command history (Up/Down recall) ──────────────────────────────────────

    /// Record a submitted command line into `command_history`.
    ///
    /// No-op when `line.trim()` is empty. A line equal to the current last entry
    /// is skipped (consecutive-duplicate dedupe). The list is capped at
    /// `COMMAND_HISTORY_CAP` (oldest dropped). Always resets the navigation cursor
    /// and clears the saved draft.
    pub fn record_command(&mut self, line: &str) {
        self.history_cursor = None;
        self.history_draft.clear();
        if line.trim().is_empty() {
            return;
        }
        if self.command_history.last().map(String::as_str) == Some(line) {
            return;
        }
        self.command_history.push(line.to_string());
        if self.command_history.len() > COMMAND_HISTORY_CAP {
            let overflow = self.command_history.len() - COMMAND_HISTORY_CAP;
            self.command_history.drain(0..overflow);
        }
    }

    /// Recall the previous (older) command into the input buffer (Up arrow).
    ///
    /// The first Up saves the in-progress input as the draft. At the oldest entry
    /// further Up is a no-op. No-op when the history is empty.
    pub fn history_prev(&mut self) {
        if self.command_history.is_empty() {
            return;
        }
        let next = match self.history_cursor {
            None => {
                self.history_draft = self.input.value.clone();
                self.command_history.len() - 1
            }
            Some(0) => return, // oldest: stay
            Some(i) => i - 1,
        };
        self.history_cursor = Some(next);
        self.input.set(self.command_history[next].clone(), true);
        self.clear_suggestions();
    }

    /// Recall the next (newer) command into the input buffer (Down arrow).
    ///
    /// Stepping past the newest entry restores the saved draft and leaves
    /// navigation. No-op when not currently navigating.
    pub fn history_next(&mut self) {
        let i = match self.history_cursor {
            None => return,
            Some(i) => i,
        };
        if i + 1 < self.command_history.len() {
            self.history_cursor = Some(i + 1);
            self.input.set(self.command_history[i + 1].clone(), true);
        } else {
            // Past the newest entry: restore the draft.
            self.history_cursor = None;
            self.input.set(std::mem::take(&mut self.history_draft), true);
        }
        self.clear_suggestions();
    }

    // ── Search helpers ────────────────────────────────────────────────────────

    /// Run a case-insensitive substring search over the visible transcript lines.
    ///
    /// Fills `search_matches` with the 0-based positions (within the visible list)
    /// of lines that contain `query`. Sets `search_idx` to the last match index
    /// when `start_backward` is true (landing on the most recent match), or the
    /// first (0) when false. Sets `search_query` to the query string regardless of
    /// whether matches were found (so the status line can show "no matches").
    /// Returns the number of matches.
    pub fn run_search(&mut self, query: &str, start_backward: bool) -> usize {
        let query_lower = query.to_lowercase();
        let visible = self.visible_transcript_indices();
        self.search_matches = visible
            .iter()
            .enumerate()
            .filter(|&(_, &raw_idx)| {
                self.transcript[raw_idx].to_lowercase().contains(&query_lower)
            })
            .map(|(pos, _)| pos)
            .collect();
        let count = self.search_matches.len();
        self.search_idx = if start_backward && count > 0 { count - 1 } else { 0 };
        self.search_query = Some(query.to_string());
        count
    }

    /// Advance the current match by one step and return the new match's visible-list position.
    ///
    /// `forward = true` moves toward the end (newer lines); `forward = false` moves
    /// toward the start (older lines). Both directions wrap around. Returns `None` if
    /// there are no matches.
    pub fn search_next(&mut self, forward: bool) -> Option<usize> {
        let count = self.search_matches.len();
        if count == 0 {
            return None;
        }
        if forward {
            self.search_idx = (self.search_idx + 1) % count;
        } else {
            self.search_idx = self.search_idx.checked_sub(1).unwrap_or(count - 1);
        }
        Some(self.search_matches[self.search_idx])
    }

    /// Clear all search state: query, matches, and index.
    pub fn clear_search(&mut self) {
        self.search_query = None;
        self.search_matches.clear();
        self.search_idx = 0;
    }
}

/// Append this render pass's pipeline stage labels to trace.log when `on`. (trace feature)
pub(crate) fn write_map_trace(user_dir: &std::path::Path, steps: &[String], on: bool) {
    if on {
        crate::trace::write(user_dir, crate::trace::Section::Map, steps);
    }
}

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;

    #[test]
    fn appstate_history_defaults_empty() {
        let s = AppState::default();
        assert!(s.history.is_empty(), "history starts empty");
    }

    #[test]
    fn filename_modal_for_picks_prompt_picker_or_autocancel() {
        use super::{filename_modal_for, FilenameModal};
        use crate::session::FilenameReq;
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x01 }, 3), FilenameModal::NamePrompt);
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x05 }, 0), FilenameModal::NamePrompt);
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x03 }, 0), FilenameModal::NamePrompt);
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x02 }, 2), FilenameModal::Picker);
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x02 }, 0), FilenameModal::AutoCancel);
    }

    #[test]
    fn file_picker_navigation_clamps_and_selects() {
        use super::FilePickerState;
        let mut p = FilePickerState::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(p.selected(), Some("a"));
        p.move_up(); // clamps at top
        assert_eq!(p.selected(), Some("a"));
        p.move_down();
        p.move_down();
        assert_eq!(p.selected(), Some("c"));
        p.move_down(); // clamps at bottom
        assert_eq!(p.selected(), Some("c"));
    }

    #[test]
    fn reset_sound_sidecars_clears_tracking_maps() {
        let mut state = AppState::default();
        state.sound_ids.insert(3, 1);
        state.sound_routines.insert(1, 0x1234);
        state.glulx_channels.insert(1, 5);
        state.glulx_sound_notify.insert(5, (7, 42));
        state.glulx_volume_notify.insert(1, (std::time::Instant::now(), 9));
        state.glulx_gain.insert(1, 0.5);
        state.glulx_volume_ramp.insert(1, VolumeRamp {
            start: std::time::Instant::now(), duration_ms: 500, start_gain: 1.0, target_gain: 0.0,
        });
        state.reset_sound_sidecars();
        assert!(state.sound_ids.is_empty());
        assert!(state.sound_routines.is_empty());
        assert!(state.glulx_channels.is_empty());
        assert!(state.glulx_sound_notify.is_empty());
        assert!(state.glulx_volume_notify.is_empty());
        assert!(state.glulx_gain.is_empty());
        assert!(state.glulx_volume_ramp.is_empty());
    }

    #[test]
    fn ramp_gain_interpolates_at_endpoints_and_midpoint() {
        // t=0 → start; t=mid → linear midpoint; t>=end (and zero duration) → target.
        assert_eq!(ramp_gain(1.0, 0.0, 0, 1000), 1.0, "t=0 sits at the start gain");
        assert_eq!(ramp_gain(1.0, 0.0, 500, 1000), 0.5, "halfway is the midpoint");
        assert_eq!(ramp_gain(1.0, 0.0, 1000, 1000), 0.0, "t=end lands on target");
        assert_eq!(ramp_gain(1.0, 0.0, 5000, 1000), 0.0, "past the end clamps to target");
        assert_eq!(ramp_gain(0.2, 0.8, 0, 0), 0.8, "zero duration is an immediate jump");
        // Rising ramp, quarter point.
        assert!((ramp_gain(0.0, 1.0, 250, 1000) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn set_volume_ext_ramps_interpolates_interrupts_and_completes() {
        use crate::session::SchannelOp;
        let mut state = AppState::default();
        state.config.enable_sound = true;
        audio::disable_output_for_tests(); // silent backend: no real device to open/tear down
        state.audio = Some(audio::AudioBackend::new(50));
        // Seed the channel's current gain (as a prior play/set_volume would).
        state.glulx_gain.insert(1, 1.0);
        // A ramped set_volume_ext installs a ramp from the current gain to target
        // (vol 0 → gain 0) and does NOT jump the stored gain immediately.
        let t0 = std::time::Instant::now();
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0, duration_ms: 1000, notify: 7 }]);
        let ramp = *state.glulx_volume_ramp.get(&1).expect("ramp installed");
        assert_eq!(ramp.start_gain, 1.0, "ramp starts from the channel's current gain");
        assert_eq!(ramp.target_gain, 0.0, "ramp targets the new volume's gain");
        assert_eq!(state.glulx_volume_notify.get(&1).map(|&(_, n)| n), Some(7), "notify still scheduled at the deadline");
        // Halfway through, the interpolated gain is ~0.5.
        state.advance_volume_ramps(t0 + std::time::Duration::from_millis(500));
        let mid = *state.glulx_gain.get(&1).unwrap();
        assert!((mid - 0.5).abs() < 0.1, "gain interpolates to ~0.5 at half-time, got {mid}");
        assert!(state.glulx_volume_ramp.contains_key(&1), "ramp still active mid-way");
        // A new change interrupts: the old ramp is replaced and heads to the new
        // target from wherever the gain currently sits (spec §8.3).
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0x10000, duration_ms: 1000, notify: 9 }]);
        let ramp2 = *state.glulx_volume_ramp.get(&1).expect("new ramp installed");
        assert_eq!(ramp2.target_gain, 1.0, "interruption retargets to the new volume");
        assert!((ramp2.start_gain - mid).abs() < 1e-6, "new ramp starts from the current gain");
        assert_eq!(state.glulx_volume_notify.get(&1).map(|&(_, n)| n), Some(9), "the prior notify was dropped, the new one scheduled");
        // Stepping past the duration completes the ramp: gain lands on target and
        // the ramp is removed.
        state.advance_volume_ramps(ramp2.start + std::time::Duration::from_millis(1000));
        assert_eq!(*state.glulx_gain.get(&1).unwrap(), 1.0, "completed ramp lands on target");
        assert!(!state.glulx_volume_ramp.contains_key(&1), "completed ramp is dropped");
        // An immediate (duration 0) change jumps and installs no ramp.
        state.play_glulx_sound_ops(&[SchannelOp::SetVolume { chan: 1, vol: 0x8000 }]);
        assert_eq!(*state.glulx_gain.get(&1).unwrap(), 0.5, "plain set_volume jumps the gain");
        assert!(!state.glulx_volume_ramp.contains_key(&1), "plain set_volume leaves no ramp");
    }

    #[test]
    fn play_glulx_sound_ops_schedules_and_interrupts_volume_notify() {
        use crate::session::SchannelOp;
        let mut state = AppState::default();
        state.config.enable_sound = true;
        audio::disable_output_for_tests(); // silent backend: no real device to open/tear down
        state.audio = Some(audio::AudioBackend::new(50));
        // A ramped set_volume_ext with a nonzero notify schedules a pending
        // volume-notify keyed by channel (no live sound needed).
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0x8000, duration_ms: 1000, notify: 7 }]);
        assert_eq!(state.glulx_volume_notify.get(&1).map(|&(_, n)| n), Some(7), "notify scheduled");
        // A second change on the same channel interrupts the first (spec §8.3):
        // the prior notify is dropped and replaced.
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0x2000, duration_ms: 500, notify: 9 }]);
        assert_eq!(state.glulx_volume_notify.get(&1).map(|&(_, n)| n), Some(9), "new change replaces the prior notify");
        // notify == 0 requests no event → schedules nothing and clears the channel.
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0, duration_ms: 0, notify: 0 }]);
        assert!(state.glulx_volume_notify.is_empty(), "notify=0 schedules no event and clears the channel");
        // Destroy cancels a channel's pending ramp notify.
        state.play_glulx_sound_ops(&[
            SchannelOp::SetVolumeExt { chan: 2, vol: 0x8000, duration_ms: 1000, notify: 3 },
            SchannelOp::Destroy { chan: 2 },
        ]);
        assert!(state.glulx_volume_notify.is_empty(), "destroy cancels the pending volume-notify");
    }

    #[test]
    fn turn_elems_interleave_text_and_image_in_transcript() {
        use crate::session::TranscriptElem;
        let mut st = AppState::default();
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(4, 4)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let elems = vec![
            TranscriptElem::Text { text: "a".into(), runs: vec![(1, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0, ParaFmt::default(), 0, false)] },
            TranscriptElem::Image(dummy),
            TranscriptElem::Text { text: "b".into(), runs: vec![(1, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0, ParaFmt::default(), 0, false)] },
        ];
        apply_transcript_elems(&mut st, &elems);
        assert_eq!(st.transcript, vec!["a".to_string(), "".to_string(), "b".to_string()]);
        assert!(st.transcript_images[1].is_some());
        assert!(st.transcript_images[0].is_none() && st.transcript_images[2].is_none());
    }

    // ── Command history (feature D) ────────────────────────────────────────────

    #[test]
    fn record_command_skips_empty_and_consecutive_duplicates() {
        let mut s = AppState::default();
        s.record_command("north");
        s.record_command("north"); // consecutive dup → skipped
        s.record_command("   ");    // blank → skipped
        s.record_command("south");
        s.record_command("north"); // not consecutive → recorded
        assert_eq!(s.command_history, vec!["north", "south", "north"]);
    }

    #[test]
    fn record_command_caps_at_500_dropping_oldest() {
        let mut s = AppState::default();
        for i in 0..600 {
            s.record_command(&format!("cmd{i}"));
        }
        assert_eq!(s.command_history.len(), COMMAND_HISTORY_CAP);
        assert_eq!(s.command_history.first().unwrap(), "cmd100");
        assert_eq!(s.command_history.last().unwrap(), "cmd599");
    }

    #[test]
    fn record_command_resets_cursor_and_draft() {
        let mut s = AppState::default();
        s.command_history = vec!["a".into(), "b".into()];
        s.history_cursor = Some(0);
        s.history_draft = "draft".into();
        s.record_command("c");
        assert_eq!(s.history_cursor, None);
        assert!(s.history_draft.is_empty());
    }

    #[test]
    fn history_up_down_recall_with_draft_save_and_restore() {
        let mut s = AppState::default();
        s.command_history = vec!["one".into(), "two".into(), "three".into()];
        s.input = "partial".into(); // in-progress draft

        // First Up: save draft, recall newest ("three").
        s.history_prev();
        assert_eq!(s.input.value, "three");
        // Up again: "two".
        s.history_prev();
        assert_eq!(s.input.value, "two");
        // Up again: "one" (oldest).
        s.history_prev();
        assert_eq!(s.input.value, "one");
        // Up at oldest: no-op (stays).
        s.history_prev();
        assert_eq!(s.input.value, "one");
        // Down: "two".
        s.history_next();
        assert_eq!(s.input.value, "two");
        // Down: "three".
        s.history_next();
        assert_eq!(s.input.value, "three");
        // Down past newest: restore the saved draft.
        s.history_next();
        assert_eq!(s.input.value, "partial");
        assert_eq!(s.history_cursor, None);
        // Down again while not navigating: no-op.
        s.history_next();
        assert_eq!(s.input.value, "partial");
    }

    #[test]
    fn history_prev_on_empty_history_is_noop() {
        let mut s = AppState::default();
        s.input = "x".into();
        s.history_prev();
        assert_eq!(s.input.value, "x");
        assert_eq!(s.history_cursor, None);
    }

    #[test]
    fn replay_state_step_clamps_and_pauses() {
        let mut r = ReplayState::new(4); // start at last idx
        assert_eq!(r.idx, 4);
        r.step(-1, 5);
        assert_eq!(r.idx, 3);
        assert!(!r.playing, "manual step pauses");
        r.step(-10, 5);
        assert_eq!(r.idx, 0, "clamped at 0");
        r.step(10, 5);
        assert_eq!(r.idx, 4, "clamped at len-1");
    }

    #[test]
    fn replay_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.overlays.replay = Some(ReplayState::new(0));
        assert!(s.any_overlay_open(), "replay open => any_overlay_open true");
    }

    #[test]
    fn filter_maps_input_with_story_and_warning_with_meta() {
        let mut s = AppState::default();
        s.push_transcript("story0");
        s.push_transcript_kind("> go north", TranscriptKind::Input);
        s.push_transcript_kind("meta", TranscriptKind::Meta);
        s.push_transcript_kind("warn", TranscriptKind::Warning);
        s.transcript_filter = TranscriptFilter::Story;
        assert_eq!(s.visible_transcript_indices(), vec![0, 1]); // Story + Input
        s.transcript_filter = TranscriptFilter::Meta;
        assert_eq!(s.visible_transcript_indices(), vec![2, 3]); // Meta + Warning
        s.transcript_filter = TranscriptFilter::Both;
        assert_eq!(s.visible_transcript_indices(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn current_room_name_defaults_none() {
        let s = AppState::default();
        assert_eq!(s.current_room_name, None);
    }

    #[test]
    fn visible_transcript_indices_respects_filter() {
        let mut s = AppState::default();
        s.push_transcript("story0");
        s.push_transcript_kind("meta1", TranscriptKind::Meta);
        s.push_transcript("story2");
        s.transcript_filter = TranscriptFilter::Both;
        assert_eq!(s.visible_transcript_indices(), vec![0, 1, 2]);
        s.transcript_filter = TranscriptFilter::Story;
        assert_eq!(s.visible_transcript_indices(), vec![0, 2]);
        s.transcript_filter = TranscriptFilter::Meta;
        assert_eq!(s.visible_transcript_indices(), vec![1]);
    }

    #[test]
    fn transcript_tags_story_and_meta() {
        let mut s = AppState::default();
        s.push_transcript("West of House");
        s.push_transcript_kind("/help line", TranscriptKind::Meta);
        // last entry is Meta, prior is Story
        assert_eq!(s.transcript_kinds.len(), 2);
        assert!(matches!(s.transcript_kinds[0], TranscriptKind::Story));
        assert!(matches!(s.transcript_kinds[1], TranscriptKind::Meta));
    }

    #[test]
    fn append_to_last_transcript_line_appends_to_existing_last_line() {
        let mut s = AppState::default();
        s.push_transcript_kind(">", TranscriptKind::Story);
        s.append_to_last_transcript_line("look");
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript[0], ">look");
    }

    #[test]
    fn append_to_last_transcript_line_pushes_new_line_when_empty_and_keeps_arrays_aligned() {
        let mut s = AppState::default();
        assert!(s.transcript.is_empty());
        s.append_to_last_transcript_line("hi");
        assert_eq!(s.transcript, vec!["hi".to_string()]);
        assert_eq!(s.transcript.len(), s.transcript_kinds.len());
        assert_eq!(s.transcript.len(), s.transcript_styles.len());
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert_eq!(s.transcript.len(), s.transcript_images.len());
    }

    #[test]
    fn append_to_last_transcript_line_inherits_trailing_run_colour() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        // A coloured game prompt line: ">" drawn on a white page background.
        s.push_transcript_runs(
            ">",
            TranscriptKind::Story,
            &[(1, 0, ZColour::Default, ZColour::True24(0x00FF_FFFF), 0, ParaFmt::default(), 0, false)],
        );
        s.append_to_last_transcript_line("look");
        assert_eq!(s.transcript.last().unwrap(), ">look");
        let tail = s.transcript_runs.last().unwrap().last().unwrap();
        assert_eq!((tail.start, tail.end), (1, 5), "run covers the appended 'look'");
        assert_eq!(tail.bg, pack_zcolour(ZColour::True24(0x00FF_FFFF)),
            "echo inherits the prompt's white background (keeps the SQ-0263 band)");
        assert_eq!(tail.bits, 0, "reverse/bold bits are not carried onto the echo");
    }

    #[test]
    fn append_to_last_transcript_line_adds_no_run_when_prompt_is_uncoloured() {
        let mut s = AppState::default();
        s.push_transcript_kind(">", TranscriptKind::Story); // no coloured runs
        let before = s.transcript_runs.last().map(|r| r.len()).unwrap_or(0);
        s.append_to_last_transcript_line("look");
        assert_eq!(s.transcript.last().unwrap(), ">look");
        let after = s.transcript_runs.last().map(|r| r.len()).unwrap_or(0);
        assert_eq!(before, after, "no coloured trailing run → plain append (theme case unchanged)");
    }

    #[test]
    fn merge_line_into_previous_folds_game_echo_onto_the_prompt() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        // The `>` prompt line (bare, no runs), then the game's own bold echo "look"
        // as a separate pushed line — CounterfeitMonkey's shape.
        s.push_transcript_kind(">", TranscriptKind::Story);
        s.push_transcript_runs("look", TranscriptKind::Story,
            &[(4, 2 /* bold */, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        assert_eq!(s.transcript.len(), 2);
        s.merge_line_into_previous(1);
        // One line now: ">look", and the bold run is shifted past the ">".
        assert_eq!(s.transcript, vec![">look".to_string()]);
        assert_eq!(s.transcript.len(), s.transcript_kinds.len());
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert_eq!(s.transcript.len(), s.transcript_images.len());
        let run = s.transcript_runs[0].iter().find(|r| r.bits == 2).expect("bold run kept");
        assert_eq!((run.start, run.end), (1, 5), "bold echo run shifted past the `>`");
    }

    #[test]
    fn fill_line_default_colours_preserves_current_colour_and_keeps_game_overrides() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        let white = pack_zcolour(ZColour::True24(0x00FF_FFFF));
        let black = pack_zcolour(ZColour::True24(0));
        let red = pack_zcolour(ZColour::True24(0x00FF_0000));
        // A `>look` line where `>` has no run and "look" is bold with DEFAULT colour,
        // except a single char CM coloured red explicitly.
        s.push_transcript_runs(">look", TranscriptKind::Story, &[
            (1, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),       // '>' — unstyled
            (1, 2, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),       // 'l' — bold, default colour
            (1, 2, ZColour::True24(0x00FF_0000), ZColour::Default, 0, ParaFmt::default(), 0, false), // 'o' — bold + explicit red fg
            (2, 2, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),       // 'ok' — bold, default colour
        ]);
        s.fill_line_default_colours(0, black, white);
        // Every char now has fg/bg resolved; the '>' and default chars take black/white,
        // the explicit red fg is kept, bold bits preserved.
        let runs = &s.transcript_runs[0];
        // Char 0 ('>'): no bits, black on white.
        let r0 = runs.iter().find(|r| r.start == 0).unwrap();
        assert_eq!((r0.bits, r0.fg, r0.bg), (0, black, white), "bare '>' gets current colours");
        // The explicit-red char keeps red fg but gains the current white bg.
        let red_run = runs.iter().find(|r| r.fg == red).expect("explicit red preserved");
        assert_eq!((red_run.bits, red_run.bg), (2, white), "override kept, default bg filled, bold kept");
        // A default bold char: bold + black on white.
        assert!(runs.iter().any(|r| r.bits == 2 && r.fg == black && r.bg == white),
            "default bold echo chars take the current black-on-white");
    }

    /// SQ-0823: the char-echo push reports whether it folded, because a folded
    /// turn's output starts on a row that was already on screen and the `[more]`
    /// pager has to step its baseline back onto that row. Every reason the fold is
    /// declined has to report `false` — a `true` there parks the view one row too
    /// far BACK, showing a row of the previous screen.
    #[test]
    fn the_char_echo_push_reports_whether_it_folded() {
        let page = "SENIOR PROGRAMMER\nDuane Beck";
        let prompt = |s: &mut AppState| s.push_transcript_runs("1> ", TranscriptKind::Story, &[]);

        // The Arthur shape: the game's cursor was still on the prompt's line.
        let mut s = AppState::default();
        prompt(&mut s);
        assert!(s.push_transcript_runs_char_echo(page, TranscriptKind::Story, &[], true));
        assert_eq!(s.transcript[0], "1> SENIOR PROGRAMMER");

        // The cursor says otherwise — the game moved before printing.
        let mut s = AppState::default();
        prompt(&mut s);
        assert!(!s.push_transcript_runs_char_echo(page, TranscriptKind::Story, &[], false));

        // The game's own leading newline is the game's line break, not the host's.
        let mut s = AppState::default();
        prompt(&mut s);
        assert!(!s.push_transcript_runs_char_echo(&format!("\n{page}"), TranscriptKind::Story, &[], true));

        // Nothing to fold onto: the last line is not game output.
        let mut s = AppState::default();
        s.push_transcript_kind("help: N, S, LOOK", TranscriptKind::Meta);
        assert!(!s.push_transcript_runs_char_echo(page, TranscriptKind::Story, &[], true));

        // …nor when the transcript sits exactly on a screen-clear boundary.
        let mut s = AppState::default();
        prompt(&mut s);
        s.mark_screen_clear();
        assert!(!s.push_transcript_runs_char_echo(page, TranscriptKind::Story, &[], true));

        // An empty turn pushes nothing, so there is nothing to have folded.
        let mut s = AppState::default();
        prompt(&mut s);
        assert!(!s.push_transcript_runs_char_echo("", TranscriptKind::Story, &[], true));
    }

    #[test]
    fn merge_line_into_previous_is_a_noop_at_zero_or_out_of_range() {
        let mut s = AppState::default();
        s.push_transcript_kind("only", TranscriptKind::Story);
        s.merge_line_into_previous(0);
        s.merge_line_into_previous(5);
        assert_eq!(s.transcript, vec!["only".to_string()]);
    }

    #[test]
    fn internal_messages_insert_above_the_inline_prompt() {
        let mut s = AppState::default();
        s.config.command_bar = false; // inline-prompt mode
        // Game output goes through push_transcript_runs (appends); it ends with the
        // kept `>` prompt.
        s.push_transcript_runs("You are in a hall.", TranscriptKind::Story, &[]);
        s.push_transcript_runs(">", TranscriptKind::Story, &[]);
        // A /help-style internal dump must land ABOVE the `>`, keeping it last.
        s.push_transcript_internal("help: N, S, LOOK, X", TranscriptKind::Meta);
        assert_eq!(
            s.transcript,
            vec!["You are in a hall.".to_string(), "help: N, S, LOOK, X".to_string(), ">".to_string()],
        );
        assert!(s.last_transcript_line_is_story(), "the `>` prompt stays the last line");
        assert_eq!(s.transcript.len(), s.transcript_kinds.len());
        assert_eq!(s.transcript.len(), s.transcript_styles.len());
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert_eq!(s.transcript.len(), s.transcript_images.len());
        assert!(matches!(s.transcript_kinds[1], TranscriptKind::Meta), "inserted line keeps its kind");

        // Multi-line internal output preserves order above the prompt.
        s.push_transcript_internal("a\nb", TranscriptKind::Meta);
        assert_eq!(s.transcript.last().unwrap(), ">");
        assert_eq!(&s.transcript[s.transcript.len() - 3..], &["a".to_string(), "b".to_string(), ">".to_string()]);
    }

    #[test]
    fn internal_messages_append_in_command_bar_mode_or_without_a_prompt() {
        // Command-bar mode: the `>` isn't in the transcript, so append normally.
        let mut s = AppState::default();
        s.config.command_bar = true;
        s.push_transcript_runs(">", TranscriptKind::Story, &[]);
        s.push_transcript_internal("help", TranscriptKind::Meta);
        assert_eq!(s.transcript, vec![">".to_string(), "help".to_string()]);
        // Inline mode but the last line isn't a game prompt (Meta) → append.
        let mut s2 = AppState::default();
        s2.config.command_bar = false;
        s2.push_transcript_internal("note", TranscriptKind::Meta);
        s2.push_transcript_internal("more", TranscriptKind::Meta);
        assert_eq!(s2.transcript, vec!["note".to_string(), "more".to_string()]);
    }

    #[test]
    fn last_transcript_line_is_story_distinguishes_game_output_from_meta() {
        let mut s = AppState::default();
        assert!(!s.last_transcript_line_is_story(), "empty transcript is not a story prompt");
        s.push_transcript_kind(">", TranscriptKind::Story);
        assert!(s.last_transcript_line_is_story(), "game `>` prompt is a story line");
        // A /help-style meta dump lands after the prompt: no longer a story last line.
        s.push_transcript_kind("help: available commands", TranscriptKind::Meta);
        assert!(!s.last_transcript_line_is_story(), "meta dump is not the game prompt");
    }

    #[test]
    fn transcript_styles_track_and_self_heal() {
        use ratatui::style::{Color, Style};
        let mut s = AppState::default();
        s.push_transcript_kind("a", TranscriptKind::Meta);
        let cyan = Style::new().fg(Color::Cyan);
        s.push_transcript_styled("b", TranscriptKind::Meta, cyan);
        assert_eq!(s.transcript.len(), s.transcript_styles.len(), "lengths stay equal");
        assert_eq!(s.transcript_styles[0], None, "plain push has no override");
        assert_eq!(s.transcript_styles[1], Some(cyan), "styled push records the style");

        // Simulate a wholesale reassignment that leaves transcript_styles short.
        s.transcript = vec!["x".into(), "y".into(), "z".into()];
        s.transcript_kinds = vec![TranscriptKind::Story; 3];
        s.push_transcript_kind("w", TranscriptKind::Meta); // must self-heal
        assert_eq!(s.transcript.len(), s.transcript_styles.len(), "self-heal re-aligns lengths");
    }

    #[test]
    fn push_transcript_image_keeps_parallel_vecs_synced() {
        let mut st = AppState::default();
        st.push_transcript("hello");
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(4, 4)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        st.push_transcript_image(dummy);
        st.push_transcript("world");
        let n = st.transcript.len();
        assert_eq!(st.transcript_kinds.len(), n);
        assert_eq!(st.transcript_styles.len(), n);
        assert_eq!(st.transcript_runs.len(), n);
        assert_eq!(st.transcript_images.len(), n);
        // The image unit sits between the two text lines.
        let img_idx = st.transcript_images.iter().position(|o| o.is_some()).unwrap();
        assert_eq!(st.transcript[img_idx], "");
        assert_eq!(st.transcript_kinds[img_idx], TranscriptKind::Story);
        assert!(st.transcript_images.iter().filter(|o| o.is_some()).count() == 1);
        assert_eq!(st.transcript_para.len(), n, "transcript_para stays length-synced");
    }

    #[test]
    fn push_transcript_runs_captures_paragraph_layout_from_first_run() {
        // SQ-0330: a chunk carrying a Centered ParaFmt sets the line's layout, and
        // the parallel `transcript_para` stays length-synced. A default-layout line
        // keeps the default ParaFmt (the Z-machine path).
        let mut st = AppState::default();
        let centered = ParaFmt { indent: 0, para_indent: 0, justify: 2, nowrap_from: None };
        st.push_transcript_runs(
            "plain\ncentered",
            TranscriptKind::Story,
            &[
                (6, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0, ParaFmt::default(), 0, false), // "plain\n"
                (8, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0, centered, 0, false),        // "centered"
            ],
        );
        assert_eq!(st.transcript, vec!["plain".to_string(), "centered".to_string()]);
        assert_eq!(st.transcript_para.len(), st.transcript.len());
        assert_eq!(st.transcript_para[0], ParaFmt::default(), "first line is left/no-indent");
        assert_eq!(st.transcript_para[1], centered, "second line takes its run's Centered layout");
    }

    #[test]
    fn truncate_transcript_trims_all_sidecars_and_bumps_gen() {
        // SQ-0407: collapsing a menu reprint truncates the transcript AND every
        // parallel sidecar vec (they must stay length-synced) and invalidates the
        // wrap cache via the generation bump.
        let mut s = AppState::default();
        s.push_transcript("line1");
        s.push_transcript("line2");
        s.push_transcript("line3");
        let gen = s.transcript_gen;

        s.truncate_transcript(1);
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript_kinds.len(), 1);
        assert_eq!(s.transcript_styles.len(), 1);
        assert_eq!(s.transcript_runs.len(), 1);
        assert_eq!(s.transcript_para.len(), 1);
        assert_eq!(s.transcript_images.len(), 1);
        assert_ne!(s.transcript_gen, gen, "truncation must bump the wrap-cache generation");

        // No-op when the length is already at/beyond the end.
        let gen2 = s.transcript_gen;
        s.truncate_transcript(5);
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript_gen, gen2, "a no-op truncate must not bump the generation");
    }

    #[test]
    fn reset_transcript_sidecars_clears_stale_head_entries() {
        // Reproduce the load/restore/reset bug: a Glulx game drew an inline image
        // (sidecar holds `Some` at a head index), then the transcript is replaced
        // wholesale with a SHORTER one. `resize` alone would keep the stale `Some`
        // at index 0 indexing the wrong line; `reset_transcript_sidecars` must wipe
        // it and length-match the new transcript.
        let mut s = AppState::default();
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(4, 4)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        s.push_transcript_image(dummy); // transcript_images[0] = Some, len 1
        s.push_transcript("a\nb"); // grow to len 3
        assert!(s.transcript_images[0].is_some(), "precondition: stale image present");

        // Wholesale reassign to a shorter transcript, then apply the fix.
        s.transcript = vec!["only".into()];
        s.reset_transcript_sidecars();

        assert_eq!(s.transcript_images.len(), s.transcript.len(), "length matches");
        assert_eq!(s.transcript_styles.len(), s.transcript.len(), "length matches");
        assert!(
            s.transcript_images.iter().all(Option::is_none),
            "no stale Some survives within range",
        );
    }

    #[test]
    fn push_runs_extracts_per_line_spans() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        s.push_transcript_runs("ab cd", TranscriptKind::Story,
            &[(2, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false), (3, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        assert_eq!(s.transcript.last().unwrap(), "ab cd");
        assert_eq!(s.transcript_runs.last().unwrap(), &vec![StyleRun { start: 0, end: 2, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert_eq!(s.transcript.len(), s.transcript_kinds.len());
    }

    #[test]
    fn push_runs_splits_across_newlines() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        s.push_transcript_runs("A\nB", TranscriptKind::Story,
            &[(1, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),
              (1, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),
              (1, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        let n = s.transcript.len();
        assert_eq!(s.transcript[n - 2], "A");
        assert_eq!(s.transcript[n - 1], "B");
        assert_eq!(s.transcript_runs[n - 2], vec![StyleRun { start: 0, end: 1, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
        assert_eq!(s.transcript_runs[n - 1], vec![StyleRun { start: 0, end: 1, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
    }

    #[test]
    fn push_runs_carries_hyperlink_value() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        // Two adjacent spans with DIFFERENT Glk hyperlink values (42 then 99):
        // the link value must be carried AND be part of the coalescing key, so
        // the two spans stay separate runs. (Unstyled/default spans emit no run,
        // which is why both spans here carry a nonzero link.)
        s.push_transcript_runs("link more", TranscriptKind::Story,
            &[(4, 0, ZColour::Default, ZColour::Default, 42, ParaFmt::default(), 0, false),
              (5, 0, ZColour::Default, ZColour::Default, 99, ParaFmt::default(), 0, false)]);
        let runs = s.transcript_runs.last().unwrap();
        assert_eq!(runs.len(), 2, "differing links do not coalesce");
        assert_eq!((runs[0].start, runs[0].end, runs[0].link), (0, 4, 42));
        assert_eq!((runs[1].start, runs[1].end, runs[1].link), (4, 9, 99));
    }

    #[test]
    fn style_run_link_defaults_to_zero_from_old_json() {
        // Old transcript.json has no `link` field; #[serde(default)] must load it as 0.
        let old: StyleRun =
            serde_json::from_str(r#"{"start":0,"end":3,"bits":2,"fg":0,"bg":0}"#).unwrap();
        assert_eq!(old.link, 0, "a missing link field defaults to 0");
    }

    #[test]
    fn push_runs_all_plain_is_empty() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        s.push_transcript_runs("hello", TranscriptKind::Story,
            &[(5, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        assert!(s.transcript_runs.last().unwrap().is_empty());
    }

    /// ZMSD §7.2.1: a run captured with `buffer_mode` off marks its line's
    /// `nowrap_from` at the first unbuffered column, per line (offsets are
    /// line-relative, and a fully buffered line stays `None`).
    #[test]
    fn push_runs_folds_buffering_off_into_para_nowrap_from() {
        use zvm::screen::ZColour;
        let plain = |n: usize, nw: bool| (n, 0u8, ZColour::Default, ZColour::Default, 0u32, ParaFmt::default(), 0u8, nw);
        let mut s = AppState::default();
        // "buffered\nmixed" + "DOTS" printed unbuffered on the second line.
        s.push_transcript_runs(
            "buffered\nmixedDOTS",
            TranscriptKind::Story,
            &[plain(14, false), plain(4, true)], // 8 + '\n' + 5 chars, then "DOTS"
        );
        let n = s.transcript_para.len();
        assert_eq!(s.transcript_para[n - 2].nowrap_from, None, "fully buffered line");
        assert_eq!(s.transcript_para[n - 1].nowrap_from, Some(5), "offset is line-relative");
    }

    #[test]
    fn push_runs_empty_text_adds_no_line() {
        // Regression (BeyondZork v4+ upper-window menu): a read_char turn that only
        // redraws the upper window emits NO new lower-window text. Pushing that
        // empty transcript must append nothing — otherwise each keypress added a
        // blank line and scrolled the (bottom-anchored) transcript up one row.
        let mut s = AppState::default();
        let before = s.transcript.len();
        s.push_transcript_runs("", TranscriptKind::Story, &[]);
        assert_eq!(s.transcript.len(), before, "empty transcript must add no line");
    }

    #[test]
    fn push_kind_keeps_runs_synced_empty() {
        let mut s = AppState::default();
        s.push_transcript_kind("x", TranscriptKind::Meta);
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert!(s.transcript_runs.last().unwrap().is_empty());
    }

    #[test]
    fn pack_roundtrip_and_run_carries_colour() {
        use zvm::screen::ZColour;
        // Round-trip all cases.
        for c in [
            ZColour::Default,
            ZColour::Standard(3),
            ZColour::Standard(12),
            ZColour::True(0x1234),
            ZColour::True24(0x00AB_CDEF),
        ] {
            assert_eq!(unpack_zcolour(pack_zcolour(c)), c,
                "pack/unpack round-trip failed for {c:?}");
        }
        // Thread colour through push_transcript_runs → StyleRun.
        let mut s = AppState::default();
        s.push_transcript_runs("ab", TranscriptKind::Story,
            &[(2, 0x02, ZColour::Standard(3), ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        let run = s.transcript_runs.last().unwrap().first()
            .expect("coloured chunk must produce a StyleRun");
        assert_eq!(unpack_zcolour(run.fg), ZColour::Standard(3),
            "fg colour must survive the push → StyleRun round-trip");
        assert_eq!(unpack_zcolour(run.bg), ZColour::Default,
            "bg colour must survive the push → StyleRun round-trip");
    }

    #[test]
    fn play_turn_sounds_never_panics_without_device() {
        use zvm::cpu::exec::SoundEvent;
        let mut s = AppState::default();       // audio = None, sound_blorb = None
        s.config.enable_sound = true;
        // A #1 bleep event: play_turn_sounds must not panic with no backend.
        let ev = SoundEvent { number: 1, effect: 2, volume: 8, repeats: 0, routine: 0 };
        s.play_turn_sounds(&[ev]);             // no device -> silent, no panic
        // A #3 sampled start with no blorb loaded: no id remembered, no panic.
        let ev3 = SoundEvent { number: 3, effect: 2, volume: 8, repeats: 1, routine: 0 };
        s.play_turn_sounds(&[ev3]);
        assert!(s.sound_ids.is_empty(), "no sound id remembered without a blorb");
    }

    #[test]
    fn glk_volume_to_gain_is_linear_over_0x10000() {
        assert_eq!(glk_volume_to_gain(0), 0.0);
        assert_eq!(glk_volume_to_gain(0x10000), 1.0);   // Glk full
        assert_eq!(glk_volume_to_gain(0x8000), 0.5);    // half
        assert!(glk_volume_to_gain(0x20000) > 1.0, "amplification passes through");
    }

    #[test]
    fn glk_repeats_to_audio_maps_counts_and_forever() {
        assert_eq!(glk_repeats_to_audio(0), None);            // play zero times → skip
        assert_eq!(glk_repeats_to_audio(1), Some(1));         // once
        assert_eq!(glk_repeats_to_audio(5), Some(5));         // N times
        assert_eq!(glk_repeats_to_audio(0xFFFF_FFFF), Some(255)); // -1 → forever
        assert_eq!(glk_repeats_to_audio(300), Some(254));     // clamp below the forever sentinel
    }

    #[test]
    fn sound_pulse_defaults_none_and_holds_kind() {
        use crate::state::BeepKind;
        let mut s = AppState::default();
        assert!(s.sound_pulse.is_none(), "no pulse by default");
        s.sound_pulse = Some(SoundPulse { kind: BeepKind::High, started: std::time::Instant::now() });
        assert!(matches!(s.sound_pulse.as_ref().map(|p| p.kind), Some(BeepKind::High)));
    }

    #[test]
    fn has_active_animation_reflects_sources() {
        use crate::state::BeepKind;
        let mut s = AppState::default();
        assert!(!s.has_active_animation(), "idle state has no active animation");

        s.sound_pulse = Some(SoundPulse { kind: BeepKind::High, started: std::time::Instant::now() });
        assert!(s.has_active_animation(), "sound pulse counts as active");
        s.sound_pulse = None;

        s.scroll_anim = Some(ScrollAnim {
            from: 0,
            to: 5,
            tween: crate::anim::Tween::new(
                std::time::Duration::from_millis(100),
                crate::anim::Easing::EaseOut,
            ),
        });
        assert!(s.has_active_animation(), "scroll anim counts as active");
        s.scroll_anim = None;
        assert!(!s.has_active_animation());

        // An open selection-list modal's ListScroll animation also counts.
        let cfg = crate::config::AnimationConfig {
            enabled: true,
            easing: crate::anim::Easing::Linear,
            scroll_ms: 80,
            ..Default::default()
        };
        let mut cs = ConfigScreenState {
            working: crate::config::Config::default(),
            scroll: Default::default(),
        };
        cs.scroll.len(100);
        cs.scroll.move_by(40, 5, &cfg); // arms a scroll animation
        s.overlays.config_screen = Some(cs);
        assert!(s.has_active_animation(), "an open modal's list scroll anim counts as active");
        s.overlays.config_screen = None;
        assert!(!s.has_active_animation());
    }

    #[test]
    fn has_active_animation_true_while_dock_slides() {
        let mut s = AppState::default();
        assert!(!s.has_active_animation(), "fresh default state has no active animation");

        let cfg = crate::config::AnimationConfig {
            enabled: true,
            easing: crate::anim::Easing::Linear,
            scroll_ms: 100,
            ..Default::default()
        };
        s.inv_dock.toggle_to(true, false);
        s.inv_dock.arm(&cfg);
        assert!(s.has_active_animation(), "arming inv_dock open counts as active");
    }

    #[test]
    fn scroll_transcript_to_arms_when_enabled() {
        let mut s = AppState::default();
        s.transcript_scroll = 3;
        s.scroll_transcript_to(8);
        assert_eq!(s.transcript_scroll, 8, "logical target updated immediately");
        let a = s.scroll_anim.as_ref().expect("animation armed when enabled");
        assert_eq!(a.from, 3, "from = previous displayed offset");
        assert_eq!(a.target(), 8, "to = new target");
    }

    // ── Story-pane scrollbar auto-hide (SQ-0782) ─────────────────────────────

    #[test]
    fn scrollbar_is_hidden_until_a_scroll_then_fades_out() {
        let mut s = AppState::default();
        s.config.animation.scrollbar_hide_ms = 1000;
        s.config.animation.scrollbar_fade_ms = 200;
        s.config.animation.easing = crate::anim::Easing::Linear;
        assert_eq!(s.transcript_scrollbar_opacity(), 0.0, "never scrolled = never shown");
        assert!(!s.transcript_scrollbar_animating());

        s.scroll_transcript_to(4);
        assert_eq!(s.transcript_scrollbar_opacity(), 1.0, "a scroll summons it, fully opaque");
        assert!(s.transcript_scrollbar_animating(), "a hide is still to come");

        // Half way through the fade.
        let now = Instant::now();
        s.scrollbar_shown_at = Some(now - Duration::from_millis(1100));
        let mid = s.transcript_scrollbar_opacity();
        assert!((0.1..0.9).contains(&mid), "mid-fade opacity should be partial, got {mid}");

        s.scrollbar_shown_at = Some(now - Duration::from_millis(2000));
        assert_eq!(s.transcript_scrollbar_opacity(), 0.0, "gone after delay + fade");
        assert!(!s.transcript_scrollbar_animating(), "settled: no more frames needed");
    }

    /// The fade's own last frame still paints the bar at the dregs of its
    /// opacity, so the run loop needs one settle frame to take it off screen.
    #[test]
    fn finalize_scrollbar_forces_exactly_one_settle_frame() {
        let mut s = AppState::default();
        s.config.animation.scrollbar_hide_ms = 100;
        s.config.animation.scrollbar_fade_ms = 100;
        s.scroll_transcript_to(4);
        assert!(!s.finalize_scrollbar_if_done(), "nothing to settle while it is still up");

        s.scrollbar_shown_at = Some(Instant::now() - Duration::from_millis(500));
        assert!(s.finalize_scrollbar_if_done(), "a finished fade asks for the settle frame");
        assert!(s.scrollbar_shown_at.is_none(), "the reveal is cleared");
        assert!(!s.finalize_scrollbar_if_done(), "and only ever asks once");

        // A bar pinned on by config never settles (there is nothing to take away).
        s.config.animation.scrollbar_hide_ms = 0;
        s.scrollbar_shown_at = Some(Instant::now() - Duration::from_secs(60));
        assert!(!s.finalize_scrollbar_if_done());
    }

    /// SQ-1198: `transcript_scroll_in_motion` is what the sixel backend reads to
    /// decide footprint-vs-payload. It must be true immediately after a scroll
    /// (so the render suppresses), keep the run loop fast-polling while it is
    /// (`has_active_animation`), and read false once the settle window elapses.
    #[test]
    fn transcript_scroll_in_motion_follows_the_settle_window() {
        let mut s = AppState::default();
        assert!(!s.transcript_scroll_in_motion(), "never scrolled = not in motion");
        assert!(!s.has_active_animation());

        s.scroll_transcript_to(5);
        assert!(s.transcript_scroll_in_motion(), "a fresh scroll is in motion");
        assert!(s.has_active_animation(), "in-motion state keeps the run loop polling without input");

        // Past the settle window (mirrors how the scrollbar tests fake elapsed
        // time above, since neither test can literally sleep for it).
        s.sixel_scroll_motion_at = Some(Instant::now() - Duration::from_millis(200));
        assert!(!s.transcript_scroll_in_motion(), "past the settle window");
    }

    /// The window closing is itself the content change (a suppressed sixel band
    /// goes back to its full payload at the same offset), so it needs the same
    /// one-settle-frame treatment as the scrollbar fade above.
    #[test]
    fn finalize_sixel_scroll_motion_forces_exactly_one_settle_frame() {
        let mut s = AppState::default();
        assert!(!s.finalize_sixel_scroll_motion_if_done(), "nothing to settle when never scrolled");

        s.scroll_transcript_to(5);
        assert!(!s.finalize_sixel_scroll_motion_if_done(), "still in motion: not done yet");

        s.sixel_scroll_motion_at = Some(Instant::now() - Duration::from_millis(200));
        assert!(s.finalize_sixel_scroll_motion_if_done(), "an elapsed window asks for the settle frame");
        assert!(s.sixel_scroll_motion_at.is_none(), "the window is cleared, mirroring finalize_scrollbar_if_done");
        assert!(!s.finalize_sixel_scroll_motion_if_done(), "and only ever asks once");
    }

    #[test]
    fn scrollbar_hide_ms_zero_pins_it_on_and_fade_ms_zero_pops_it() {
        let mut s = AppState::default();
        s.config.animation.scrollbar_hide_ms = 0;
        assert_eq!(s.transcript_scrollbar_opacity(), 1.0, "0 = never auto-hide, even unscrolled");
        assert!(!s.transcript_scrollbar_animating(), "a permanent bar needs no frames");

        s.config.animation.scrollbar_hide_ms = 100;
        s.config.animation.scrollbar_fade_ms = 0;
        s.scrollbar_shown_at = Some(Instant::now() - Duration::from_millis(101));
        assert_eq!(s.transcript_scrollbar_opacity(), 0.0, "no fade = it pops");

        // Animation off pops it too, whatever the fade is set to.
        s.config.animation.scrollbar_fade_ms = 5_000;
        s.config.animation.enabled = false;
        assert_eq!(s.transcript_scrollbar_opacity(), 0.0);
    }

    #[test]
    fn scrollbar_reveal_survives_new_output_without_being_retriggered() {
        let mut s = AppState::default();
        s.scroll_transcript_to(2);
        let shown = s.scrollbar_shown_at.expect("a scroll records the reveal");
        s.push_transcript("The troll shrugs and ambles away.");
        s.mark_screen_clear();
        assert_eq!(s.scrollbar_shown_at, Some(shown), "game output must not re-summon the bar");
    }

    #[test]
    fn scroll_transcript_to_jumps_when_disabled() {
        let mut s = AppState::default();
        s.config.animation.enabled = false;
        s.transcript_scroll = 3;
        s.scroll_transcript_to(8);
        assert_eq!(s.transcript_scroll, 8);
        assert!(s.scroll_anim.is_none(), "disabled = instant, no animation");
    }

    #[test]
    fn scroll_transcript_to_zero_ms_jumps() {
        let mut s = AppState::default();
        s.config.animation.scroll_ms = 0;
        s.transcript_scroll = 2;
        s.scroll_transcript_to(6);
        assert_eq!(s.transcript_scroll, 6);
        assert!(s.scroll_anim.is_none(), "scroll_ms = 0 = instant, no animation");
    }

    #[test]
    fn scroll_transcript_to_retargets_from_current_displayed() {
        let mut s = AppState::default();
        s.transcript_scroll = 0;
        s.scroll_transcript_to(10); // arm 0 -> 10
        // Immediately retarget: progress is ~0, so the new `from` is ~current (~0).
        s.scroll_transcript_to(4);
        let a = s.scroll_anim.as_ref().unwrap();
        assert!(a.from < 1, "retarget starts from current displayed offset, got {}", a.from);
        assert_eq!(a.target(), 4);
    }

    #[test]
    fn effective_transcript_scroll_uses_target_or_rounded_anim() {
        let mut s = AppState::default();
        s.transcript_scroll = 9;
        assert_eq!(s.effective_transcript_scroll(), 9, "no anim = logical target");
        // A done tween reports current() == to; the offset is line-rounded.
        s.scroll_anim = Some(ScrollAnim {
            from: 0,
            to: 4,
            tween: crate::anim::Tween::new(std::time::Duration::ZERO, crate::anim::Easing::Linear),
        });
        assert_eq!(s.effective_transcript_scroll(), 4, "done tween shows rounded target");
    }

    #[test]
    fn scroll_anim_current_interpolates() {
        let a = ScrollAnim {
            from: 2,
            to: 10,
            tween: crate::anim::Tween::new(
                std::time::Duration::from_millis(100),
                crate::anim::Easing::Linear,
            ),
        };
        // Right after construction progress is ~0, so current() is near `from`.
        let c = a.current();
        assert!((2.0..3.0).contains(&c), "current near from at start, got {c}");
    }

    #[test]
    fn scroll_anim_instant_when_disabled() {
        use crate::anim::Easing;
        let cfg = crate::config::AnimationConfig { enabled: false, easing: Easing::EaseOut, scroll_ms: 120, ..Default::default() };
        assert!(ScrollAnim::to(0, 10, &cfg).is_none(), "disabled animation arms nothing");
        let cfg0 = crate::config::AnimationConfig { enabled: true, easing: Easing::EaseOut, scroll_ms: 0, ..Default::default() };
        assert!(ScrollAnim::to(0, 10, &cfg0).is_none(), "scroll_ms = 0 arms nothing");
    }

    #[test]
    fn scroll_anim_interpolates_then_settles() {
        use crate::anim::Easing;
        let cfg = crate::config::AnimationConfig { enabled: true, easing: Easing::Linear, scroll_ms: 40, ..Default::default() };
        let a = ScrollAnim::to(0, 10, &cfg).expect("armed");
        assert_eq!(a.target(), 10);
        let c = a.current();
        assert!((0.0..=10.0).contains(&c), "current within range during ease: {c}");
    }

    #[test]
    fn any_overlay_open_reflects_state() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open(), "default AppState must have no overlay open");

        // saves
        s.overlays.saves = Some(SavesState { entries: vec![], scroll: Default::default() });
        assert!(s.any_overlay_open(), "saves open => any_overlay_open true");
        s.overlays.saves = None;

        // file_browser
        s.overlays.file_browser = Some(FileBrowserState::build(
            std::path::PathBuf::from("/tmp"),
            FbMode::PickFile));
        assert!(s.any_overlay_open(), "file_browser open => any_overlay_open true");
        s.overlays.file_browser = None;

        // config_screen
        s.overlays.config_screen = Some(ConfigScreenState {
            working: crate::config::Config::default(),
            scroll: Default::default(),
        });
        assert!(s.any_overlay_open(), "config_screen open => any_overlay_open true");
        s.overlays.config_screen = None;

        // The command band is deliberately absent from this list: it is a dock,
        // not a modal, and must NOT register as an overlay at all (SQ-0664).
        s.overlays.command_band = Some(CommandBandState::default());
        assert!(!s.any_overlay_open(), "the command panel is not an overlay");
        assert!(!s.any_modal_overlay_open(), "…and certainly not a modal one");
        s.overlays.command_band = None;

        // hotkey_dialog
        s.overlays.hotkey_dialog = true;
        assert!(s.any_overlay_open(), "hotkey_dialog true => any_overlay_open true");
        s.overlays.hotkey_dialog = false;

        // room dock — deliberately NOT an overlay (SQ-0692). It reserves rows out
        // of the map pane instead of covering it, so the story prompt and caret
        // must survive it; the room panel it replaced counted here and blanked
        // both.
        s.room_dock.toggle_to(true, true);
        s.selected_room = Some(1);
        assert!(!s.any_overlay_open(), "the room panel is not an overlay, pinned or not");
        s.selected_room = None;
        assert!(!s.any_overlay_open());
        s.room_dock.toggle_to(false, true);

        // tidy_anim
        s.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "test".to_string(),
            graph: mapper::graph::MapGraph::new(),
            description: String::new(),
            stats: mapper::layout::TidyStats::default(),
            stage_start: false,
            manifest: None,
        }], mapper::layer::MAIN_LAYER));
        assert!(s.any_overlay_open(), "tidy_anim active => any_overlay_open true");
        s.tidy_anim = None;

        // text_entry
        s.overlays.text_entry = Some(TextEntryDialog::new(TextEntryKind::CreateFile, ""));
        assert!(s.any_overlay_open(), "text_entry active => any_overlay_open true");
        s.overlays.text_entry = None;

        // confirm_delete_save
        s.overlays.confirm_delete_save = Some(std::path::PathBuf::from("/x.lanthorn"));
        assert!(s.any_overlay_open(), "confirm_delete_save active => any_overlay_open true");
        s.overlays.confirm_delete_save = None;

        // confirm_overwrite_save
        s.overlays.confirm_overwrite_save = Some(ConfirmOverwriteSave {
            path: std::path::PathBuf::from("/x.lanthorn"),
            existing_name: "x".to_string(),
            pending: PendingOverwrite::SaveAs,
        });
        assert!(s.any_overlay_open(), "confirm_overwrite_save active => any_overlay_open true");
        s.overlays.confirm_overwrite_save = None;

        // launch_dialog
        s.overlays.launch_dialog = true;
        assert!(s.any_overlay_open(), "launch_dialog true => any_overlay_open true");
        s.overlays.launch_dialog = false;

        // debug: a tiled pane, not a modal overlay
        s.debug = Some(crate::debug_panel::DebugPanelState::new(0));
        assert!(!s.any_overlay_open(), "debug pane open => any_overlay_open still false");
        s.debug = None;

        assert!(!s.any_overlay_open(), "all cleared => any_overlay_open false again");
    }

    #[test]
    fn toggle_map_flips_the_panel_and_never_moves_focus() {
        let mut s = AppState::default();
        assert!(matches!(s.layout, Layout::Split));
        // SQ-0599: Tab does not reach the map, so focus is on the game before
        // and after — there is no hidden-pane hazard left to guard against.
        s.toggle_focus();
        assert!(matches!(s.focus, Focus::Game), "Tab has nowhere to go without the inspector");
        s.toggle_map();
        assert!(matches!(s.layout, Layout::TranscriptFull));
        assert!(matches!(s.focus, Focus::Game));
        s.toggle_map();
        assert!(matches!(s.layout, Layout::Split));
        assert!(matches!(s.focus, Focus::Game));
    }

    /// The whole point of SQ-0599: with the inspector closed, Tab is inert and
    /// every keystroke keeps going to the story. It used to hand the keyboard
    /// to the map, silently changing what the next arrow key did.
    #[test]
    fn tab_does_not_hand_the_keyboard_to_the_map() {
        let mut s = AppState::default();
        assert!(matches!(s.layout, Layout::Split), "map is visible");
        for _ in 0..4 {
            s.toggle_focus();
            assert!(matches!(s.focus, Focus::Game), "Tab must never land on the map pane");
        }
        // Shift-Tab likewise.
        for _ in 0..4 {
            s.cycle_focus(false);
            assert!(matches!(s.focus, Focus::Game));
        }
    }

    #[test]
    fn toggle_focus_stays_on_game_when_map_hidden() {
        let mut s = AppState::default();
        s.toggle_map(); // → TranscriptFull, map hidden
        assert!(matches!(s.layout, Layout::TranscriptFull));
        s.toggle_focus();
        assert!(matches!(s.focus, Focus::Game), "Tab can't focus a hidden map");
    }

    #[test]
    fn toggle_focus_reaches_debug_region_when_map_hidden() {
        let mut s = AppState::default();
        s.toggle_map(); // → TranscriptFull, map hidden
        assert!(matches!(s.layout, Layout::TranscriptFull));
        s.debug = Some(crate::debug_panel::DebugPanelState::new(0));
        s.toggle_focus();
        assert!(
            matches!(s.focus, Focus::Map),
            "Tab reaches the debug region even though the map itself is hidden"
        );
        // 4 focus stops (story + 3 debug windows): three more Tabs return to story.
        s.toggle_focus();
        s.toggle_focus();
        s.toggle_focus();
        assert!(matches!(s.focus, Focus::Game));
    }

    #[test]
    fn cycle_focus_steps_per_window_including_the_story() {
        let mut s = AppState::default();
        s.layout = Layout::Split;
        s.debug = Some(crate::debug_panel::DebugPanelState::new(0));
        // Forward: story → debug win 0 → 1 → 2 → story.
        assert!(matches!(s.focus, Focus::Game));
        s.cycle_focus(true);
        assert!(matches!(s.focus, Focus::Map));
        assert_eq!(s.debug.as_ref().unwrap().focus, 0);
        s.cycle_focus(true);
        assert_eq!(s.debug.as_ref().unwrap().focus, 1);
        s.cycle_focus(true);
        assert_eq!(s.debug.as_ref().unwrap().focus, 2);
        s.cycle_focus(true);
        assert!(matches!(s.focus, Focus::Game), "wraps from the last window back to the story");
        // Backward from the story lands on the last debug window.
        s.cycle_focus(false);
        assert!(matches!(s.focus, Focus::Map));
        assert_eq!(s.debug.as_ref().unwrap().focus, 2);
    }

    #[test]
    fn focus_layout_zoom_transitions() {
        let mut s = AppState::default();
        assert!(matches!(s.focus, Focus::Game));
        s.toggle_focus();
        assert!(matches!(s.focus, Focus::Game), "no map focus stop (SQ-0599)");
        s.toggle_map();
        assert!(matches!(s.layout, Layout::TranscriptFull));
        s.toggle_map();
        assert!(matches!(s.layout, Layout::Split));
        // Zoom clamps. A keypress moves one VISIBLE step (SQ-0350); the fine 0-8 counter is
        // the wheel's, not the keyboard's.
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Compact));
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview));
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview)); // clamped
        s.zoom_in();
        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Boxes));
        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Boxes)); // clamped
    }

    #[test]
    fn input_line_and_transcript() {
        let mut s = AppState::default();
        s.push_input_char('g');
        s.push_input_char('o');
        s.backspace();
        assert_eq!(s.input.value, "g");
        let cmd = s.take_input();
        assert_eq!(cmd, "g");
        assert_eq!(s.input.value, "");
        s.push_transcript("line1\nline2");
        assert_eq!(s.transcript.len(), 2);
    }

    #[test]
    fn recenter_on_centers_cell() {
        let mut s = AppState::default(); // Boxes zoom: effective step 13×7
        // Centering cell (5, 5) in a 20×10 character pane:
        // cells_w = 20 / 13 = 1, cells_h = 10 / 7 = 1
        // scroll = (5 - 1/2, 5 - 1/2) = (5 - 0, 5 - 0) = (5, 5)
        s.recenter_on((5, 5), 20, 10);
        assert_eq!(s.scroll, (5, 5));
    }

    #[test]
    fn recenter_on_boxes_larger_pane() {
        let mut s = AppState::default(); // Boxes zoom: effective step 13×7
        // Centering cell (0, 0) in a 80×24 character pane:
        // cells_w = 80 / 13 = 6, cells_h = 24 / 7 = 3
        // scroll = (0 - 6/2, 0 - 3/2) = (0 - 3, 0 - 1) = (-3, -1)
        s.recenter_on((0, 0), 80, 24);
        assert_eq!(s.scroll, (-3, -1));
    }

    #[test]
    fn recenter_on_compact_zoom() {
        use crate::state::Zoom;
        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // steps = (12, 5)
        // Centering cell (4, 4) in a 48×20 pane:
        // cells_w = 48 / 12 = 4, cells_h = 20 / 5 = 4
        // scroll = (4 - 4/2, 4 - 4/2) = (4 - 2, 4 - 2) = (2, 2)
        s.recenter_on((4, 4), 48, 20);
        assert_eq!(s.scroll, (2, 2));
    }

    #[test]
    fn pan_accumulates() {
        let mut s = AppState::default();
        s.pan(3, -2);
        s.pan(1, 4);
        assert_eq!(s.scroll, (4, 2));
    }

    #[test]
    fn select_room_roundtrip() {
        let mut s = AppState::default();
        assert_eq!(s.selected_room, None);
        s.select_room(Some(42));
        assert_eq!(s.selected_room, Some(42));
        s.select_room(None);
        assert_eq!(s.selected_room, None);
    }

    #[test]
    fn active_layer_follows_current_then_view_override() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_current(1);
        let l = g.new_layer(Some(0), "B".into());
        let mut s = AppState::default();
        assert_eq!(s.active_layer(&g), 0, "defaults to current room's layer");
        s.set_viewed_layer(Some(l));
        assert_eq!(s.active_layer(&g), l, "explicit view wins");
        s.set_viewed_layer(Some(999)); // stale id (no such layer)
        assert_eq!(s.active_layer(&g), 0, "stale view falls back to current room's layer");
    }

    /// SQ-0359: the bug in one assertion. A tidy animation's frames are `layer_subgraph`s, and a
    /// subgraph reports `layers()` as main-only however it was built — so asking the FRAME which
    /// layer to draw answers `MAIN_LAYER`, which it holds no rooms for, and the map goes blank.
    #[test]
    fn frame_layer_takes_an_animations_layer_from_the_animation_not_its_subgraph() {
        use mapper::graph::MapGraph;
        use mapper::layer::MAIN_LAYER;

        let mut live = MapGraph::new();
        live.upsert_room(1, "Hall".into());
        live.set_current(1);
        live.set_pos(1, (0, 0));
        let cellar = live.new_layer(Some(0), "Cellar".into());
        live.upsert_room(2, "Cellar".into());
        live.set_room_layer(2, cellar);
        live.set_pos(2, (0, 0)); // placed: `render` only emits rooms that have a position

        let mut s = AppState::default();
        s.set_viewed_layer(Some(cellar));

        // The frame the animation would carry: the Cellar layer, extracted.
        let sub = live.layer_subgraph(cellar);
        assert!(
            !sub.layers().contains_key(&cellar),
            "a subgraph does not admit which layer it is — the whole reason for this field"
        );
        assert_eq!(
            s.active_layer(&sub),
            MAIN_LAYER,
            "so asking the frame yields main: the blank-map bug"
        );
        // And that is not a cosmetic mislabel — it is the blank map the user reported.
        assert!(
            mapper::render::render_layer(&sub, MAIN_LAYER).rooms.is_empty(),
            "the frame holds no main-layer rooms, so drawing it as main draws nothing"
        );
        assert!(
            !mapper::render::render_layer(&sub, cellar).rooms.is_empty(),
            "drawn as the layer it actually is, the frame has rooms"
        );

        s.tidy_anim = Some(TidyAnim::new(
            vec![TidyFrame {
                label: "Build".into(),
                graph: sub,
                description: String::new(),
                stats: Default::default(),
                stage_start: true,
                manifest: None,
            }],
            cellar,
        ));
        assert_eq!(
            s.frame_layer(&live, None),
            cellar,
            "the animation states its own layer, so the map draws the rooms being tidied"
        );

        // With no animation, nothing changes: the live graph still answers for itself.
        s.tidy_anim = None;
        assert_eq!(s.frame_layer(&live, None), cellar, "viewed layer still wins on the live graph");
        s.set_viewed_layer(None);
        assert_eq!(s.frame_layer(&live, None), MAIN_LAYER, "falls back to the current room's layer");
    }

    /// Replay outranks an animation, matching the map pane's own order.
    #[test]
    fn frame_layer_prefers_a_replay_graph_over_an_animation() {
        use mapper::graph::MapGraph;
        use mapper::layer::MAIN_LAYER;

        let mut live = MapGraph::new();
        live.upsert_room(1, "Hall".into());
        live.set_current(1);
        let cellar = live.new_layer(Some(0), "Cellar".into());

        let mut replay = MapGraph::new();
        replay.upsert_room(1, "Hall".into());
        replay.set_current(1);

        let mut s = AppState::default();
        s.tidy_anim = Some(TidyAnim::new(
            vec![TidyFrame {
                label: "Build".into(),
                graph: MapGraph::new(),
                description: String::new(),
                stats: Default::default(),
                stage_start: true,
                manifest: None,
            }],
            cellar,
        ));
        assert_eq!(
            s.frame_layer(&live, Some(&replay)),
            MAIN_LAYER,
            "replay is what's on screen, so it picks the layer — not the stale animation"
        );
    }

    #[test]
    fn appstate_default_symbols_are_default_set() {
        let st = AppState::default();
        assert_eq!(st.symbols, crate::symbols::SymbolSet::default());
    }

    /// Wait for the in-flight render worker to finish, then install it.
    fn drain_render_job(s: &mut AppState) {
        while s
            .render_job
            .borrow()
            .as_ref()
            .is_some_and(|j| !j.handle.is_finished())
        {
            std::thread::yield_now();
        }
        s.poll_render_job();
    }

    /// SQ-0391, flipped by SQ-0666: a direction that goes NOWHERE adds no room and no
    /// connection, so `graph_gen` deliberately does not bump for it (SQ-0378 keeps a plain step
    /// from re-routing the whole map). The untried-exits OVERLAY was memoised on that generation
    /// and needed a hand-written refresh to notice a foiled move; the matrix view that replaced
    /// it reads the graph directly on every frame and so cannot go stale at all. Same fact, same
    /// awkward turn, one fewer cache to keep honest.
    #[test]
    fn a_foiled_move_shows_up_immediately_without_a_geometry_change() {
        use mapper::direction::Direction;
        use mapper::mapper::Mapper;
        use mapper::matrix::{classify, MatrixCell};

        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        let mut s = AppState::default();
        let _ = s.cached_map_render(0, &m.graph);
        drain_render_job(&mut s);

        assert_eq!(classify(&m.graph, 1, Direction::N), MatrixCell::Untried, "north starts `·`");

        let (gen, rooms, conns) =
            (s.graph_gen, m.graph.rooms().count(), m.graph.connections().len());
        m.observe(1, "Hall", Some(Direction::N)); // typed, went nowhere
        assert_eq!(m.graph.rooms().count(), rooms, "a foiled move adds no room");
        assert_eq!(m.graph.connections().len(), conns, "and no connection");
        assert_eq!(s.graph_gen, gen, "so the map memo is NOT invalidated");

        assert_eq!(
            classify(&m.graph, 1, Direction::N),
            MatrixCell::Probed,
            "the cell becomes `×` on the same turn — nothing waits for the geometry to change"
        );
    }

    /// SQ-0379: no routing runs on the main thread — even the first build. The
    /// first draw serves an empty placeholder while the real model routes on a
    /// worker; a later geometry change re-routes off-thread while the last-ready
    /// model keeps being served, so the interpreter never blocks.
    #[test]
    fn cached_map_render_routes_off_thread_including_first_build() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        let mut s = AppState::default();

        // First call: empty placeholder served, real model routes off-thread.
        {
            let rm = s.cached_map_render(0, &g);
            assert_eq!(rm.rooms.len(), 0, "empty placeholder while the first route runs");
        }
        assert!(s.render_job.borrow().is_some(), "even the first build is off-thread");
        drain_render_job(&mut s);
        {
            let rm = s.cached_map_render(0, &g);
            assert_eq!(rm.rooms.len(), 1, "routed model served once it lands");
        }
        // Same (gen, layer): reused, no new worker.
        let _ = s.cached_map_render(0, &g);
        assert!(s.render_job.borrow().is_none());

        // A geometry change (new room + gen bump): re-route OFF-thread, the STALE
        // model (still 1 room) served meanwhile.
        g.upsert_room(2, "B".into());
        g.set_pos(2, (1, 0));
        s.graph_gen = s.graph_gen.wrapping_add(1);
        {
            let rm = s.cached_map_render(0, &g);
            assert_eq!(rm.rooms.len(), 1, "last-ready model served while routing");
        }
        assert!(s.render_job.borrow().is_some(), "a stale model re-routes off-thread");
        drain_render_job(&mut s);
        {
            let rm = s.cached_map_render(0, &g);
            assert_eq!(rm.rooms.len(), 2, "the freshly routed model is now served");
        }
    }

    /// SQ-1182: `render_map`'s scroll-independent derived tables (room
    /// placement, position tables, edge kinds) are cached beside the live model
    /// and dropped whenever that model is replaced — INCLUDING the
    /// same-`(gen, layer)` replacement of the first draw's empty placeholder by
    /// the first real route, which the key alone cannot tell apart. That
    /// replacement is the stale-cache hazard this pins: falsify by removing the
    /// `map_derived` clear in `poll_render_job`, and the third block serves
    /// placement for zero rooms against a one-room model.
    #[test]
    fn map_derived_tables_are_dropped_when_the_live_model_is_replaced() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        let mut g = mapper::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        let mut s = AppState::default();
        let area = Rect::new(0, 0, 40, 20);

        // First draw: the empty placeholder is the live model, and the derived
        // tables cached for it describe zero rooms.
        {
            let rm = s.cached_map_render(0, &g);
            let mut buf = Buffer::empty(area);
            crate::render::map::render_map(&rm, &s, area, &mut buf);
        }
        {
            let d = s.map_derived.borrow();
            let (_, _, tables) = d.as_ref().expect("the live model's tables are cached");
            assert_eq!(tables.rooms_placed(), 0, "…derived from the placeholder");
        }

        // The routed model lands at the SAME (gen, layer): the cache must drop.
        drain_render_job(&mut s);
        assert!(
            s.map_derived.borrow().is_none(),
            "installing the routed model drops the placeholder's tables"
        );

        // The next draw derives fresh tables from the real model.
        {
            let rm = s.cached_map_render(0, &g);
            let mut buf = Buffer::empty(area);
            crate::render::map::render_map(&rm, &s, area, &mut buf);
        }
        let d = s.map_derived.borrow();
        let (_, _, tables) = d.as_ref().expect("rebuilt on the next draw");
        assert_eq!(tables.rooms_placed(), 1, "…and they describe the routed model");
    }

    /// SQ-0378: a step between already-placed rooms changes the current-room
    /// highlight but not the routed geometry, so `graph_gen` does not bump. The
    /// cache must follow the player WITHOUT re-routing (no worker spawns).
    #[test]
    fn cached_map_render_refreshes_current_without_rerouting() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        g.upsert_room(2, "B".into());
        g.set_pos(2, (1, 0));
        g.set_current(1);
        let mut s = AppState::default();

        // Build + install the initial model.
        let _ = s.cached_map_render(0, &g);
        drain_render_job(&mut s);
        {
            let rm = s.cached_map_render(0, &g);
            assert!(rm.rooms.iter().find(|r| r.id == 1).unwrap().is_current);
            assert!(!rm.rooms.iter().find(|r| r.id == 2).unwrap().is_current);
        }

        // Move the player to room 2 WITHOUT bumping graph_gen.
        g.set_current(2);
        {
            let rm = s.cached_map_render(0, &g);
            assert!(!rm.rooms.iter().find(|r| r.id == 1).unwrap().is_current);
            assert!(
                rm.rooms.iter().find(|r| r.id == 2).unwrap().is_current,
                "the highlight follows the player"
            );
        }
        assert!(
            s.render_job.borrow().is_none(),
            "a current-room change must NOT spawn a re-route worker"
        );
    }

    // ── FileBrowserState tests ────────────────────────────────────────────────

    /// Create a temporary directory with a unique tag.
    /// Contents: subdir/, save.qzl, notes.txt.
    fn make_test_fb_dir(tag: &str) -> std::path::PathBuf {
        let dir = crate::scratch_dir(&format!("fb-{tag}"));
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        std::fs::write(dir.join("save.qzl"), b"fake quetzal").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a save").unwrap();
        dir
    }

    #[test]
    fn filebrowser_pickfile_shows_dirs_and_qzl_not_txt() {
        let dir = make_test_fb_dir("pickfile");
        let fb = FileBrowserState::build(dir.clone(), FbMode::PickFile);
        let names: Vec<&str> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&".."), "should contain parent link");
        assert!(names.contains(&"subdir"), "should contain subdir");
        assert!(names.contains(&"save.qzl"), "should contain .qzl file");
        assert!(!names.contains(&"notes.txt"), ".txt file must not appear in PickFile mode");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filebrowser_dotdot_absent_at_root() {
        // Synthesize a state rooted at "/" (or the filesystem root on this OS).
        let root = std::path::Path::new("/");
        let fb = FileBrowserState::build(root.to_path_buf(), FbMode::PickFile);
        let has_dotdot = fb.entries.iter().any(|e| e.name == "..");
        assert!(!has_dotdot, "'..' must not appear when at filesystem root");
    }

    #[test]
    fn filebrowser_cd_into_subdir_and_refresh() {
        let dir = make_test_fb_dir("cd");
        let mut fb = FileBrowserState::build(dir.clone(), FbMode::PickFile);
        let subdir = dir.join("subdir");
        fb.cd(subdir.clone());
        assert_eq!(fb.cwd, subdir, "cwd should update after cd");
        assert_eq!(fb.scroll.selected, 0, "selection should reset to 0 after cd");
        // subdir is empty (no qzl files), but ".." should be present.
        let names: Vec<&str> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&".."), "subdir should show '..'");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filebrowser_entries_sorted_dirs_before_files() {
        let dir = make_test_fb_dir("sorted");
        let fb = FileBrowserState::build(dir.clone(), FbMode::PickFile);
        // Verify: ".." first, then dirs, then files.
        let mut saw_dir = false;
        let mut saw_file = false;
        for e in &fb.entries {
            if e.is_dir {
                assert!(!saw_file, "dirs should appear before files, but saw a file first");
                saw_dir = true;
            } else {
                saw_file = true;
            }
        }
        assert!(saw_dir, "should have at least one dir");
        assert!(saw_file, "should have at least one file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── zoom_level / fine zoom tests (item 2) ────────────────────────────────

    #[test]
    fn zoom_level_default_is_boxes() {
        let s = AppState::default();
        assert_eq!(s.zoom_level, 7);
        assert!(matches!(s.zoom, Zoom::Boxes));
    }

    #[test]
    fn a_zoom_keypress_always_moves_the_map() {
        // SQ-0350: the zoom keys "did not respond". They always fired — they just moved a fine
        // counter nobody could see. Nine fine levels collapse to three views (0-2 Overview,
        // 3-5 Compact, 6-8 Boxes) and the default sits at 7, mid-Boxes: so `-` needed TWO presses
        // for the first visible change, and `+` never did anything at all (7 -> 8 is still Boxes).
        // One press must now equal one visible step.
        let mut s = AppState::default();
        assert!(matches!(s.zoom, Zoom::Boxes), "default is the most detailed view");

        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Compact), "one press of `-` moves the map");
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview), "and again");
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview), "clamped at the least detailed view");

        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Compact), "one press of `+` moves it back");
        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Boxes));
        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Boxes), "clamped at the most detailed view");
    }

    #[test]
    fn zoom_keypresses_are_exact_inverses() {
        // Landing mid-band is what makes this hold: an edge landing would let `+` then `-` drift
        // into a different view than it started in, and the wheel would tip over a boundary on its
        // very first notch.
        let mut s = AppState::default();
        for _ in 0..2 {
            s.zoom_out();
        }
        let (lvl, z) = (s.zoom_level, s.zoom);
        s.zoom_in();
        s.zoom_out();
        assert_eq!(s.zoom_level, lvl, "+ then - returns to the same fine level");
        assert_eq!(s.zoom, z);
    }

    #[test]
    fn zoom_by_honours_its_magnitude_and_clamps() {
        // SQ-0355: `zoom-map <n>` promises "step by signed n". Every n used to collapse to one
        // step, so `zoom-map 5` moved exactly as far as `zoom-map in`.
        let mut s = AppState::default(); // Boxes
        s.zoom_by(-1);
        assert!(matches!(s.zoom, Zoom::Compact), "-1 is one step out");

        let mut s = AppState::default();
        s.zoom_by(-2);
        assert!(matches!(s.zoom, Zoom::Overview), "-2 goes two steps, not one");

        // Clamps exactly as the equivalent run of keypresses would.
        let mut s = AppState::default();
        s.zoom_by(-99);
        assert!(matches!(s.zoom, Zoom::Overview), "over-stepping clamps, it does not wrap");
        s.zoom_by(99);
        assert!(matches!(s.zoom, Zoom::Boxes));

        // n and a run of single steps must agree — the command and the key cannot drift apart.
        let (mut by, mut steps) = (AppState::default(), AppState::default());
        by.zoom_by(-2);
        steps.zoom_out();
        steps.zoom_out();
        assert_eq!(by.zoom_level, steps.zoom_level);
        assert_eq!(by.zoom, steps.zoom);

        // 0 is a no-op here; the parser maps `zoom-map 0` to ZoomReset before it reaches this.
        let mut s = AppState::default();
        let before = s.zoom_level;
        s.zoom_by(0);
        assert_eq!(s.zoom_level, before);
    }

    #[test]
    fn the_wheel_still_steps_finely() {
        // The fine levels exist so a fast ctrl+scroll cannot skip straight past Compact. Fixing the
        // KEYBOARD must not flatten the wheel into the same coarse step.
        let mut s = AppState::default(); // level 7, Boxes
        s.zoom_out_fine();
        assert_eq!(s.zoom_level, 6);
        assert!(matches!(s.zoom, Zoom::Boxes), "one notch stays inside the band");
        s.zoom_out_fine();
        assert_eq!(s.zoom_level, 5);
        assert!(matches!(s.zoom, Zoom::Compact), "the third notch tips into the next view");
        for _ in 0..5 {
            s.zoom_out_fine();
        }
        assert_eq!(s.zoom_level, 0);
        assert!(matches!(s.zoom, Zoom::Overview));
        s.zoom_out_fine();
        assert_eq!(s.zoom_level, 0, "clamped at 0");

        for _ in 0..8 {
            s.zoom_in_fine();
        }
        assert_eq!(s.zoom_level, 8);
        assert!(matches!(s.zoom, Zoom::Boxes));
        s.zoom_in_fine();
        assert_eq!(s.zoom_level, 8, "clamped at 8");
    }

    #[test]
    fn zoom_reset_returns_to_default_level() {
        let mut s = AppState::default();
        // Go to Overview (two visible steps from Boxes).
        for _ in 0..2 {
            s.zoom_out();
        }
        assert!(matches!(s.zoom, Zoom::Overview));
        // Also set char_pan to something non-zero
        s.char_pan = (4, -2);
        // Reset
        s.zoom_reset();
        assert_eq!(s.zoom_level, 7, "zoom_reset must restore level to 7");
        assert!(matches!(s.zoom, Zoom::Boxes), "zoom_reset must restore Zoom::Boxes");
        assert_eq!(s.char_pan, (0, 0), "zoom_reset must clear char_pan");
    }

    #[test]
    fn zoom_from_level_maps_correctly() {
        use super::zoom_from_level;
        assert!(matches!(zoom_from_level(0), Zoom::Overview));
        assert!(matches!(zoom_from_level(1), Zoom::Overview));
        assert!(matches!(zoom_from_level(2), Zoom::Overview));
        assert!(matches!(zoom_from_level(3), Zoom::Compact));
        assert!(matches!(zoom_from_level(4), Zoom::Compact));
        assert!(matches!(zoom_from_level(5), Zoom::Compact));
        assert!(matches!(zoom_from_level(6), Zoom::Boxes));
        assert!(matches!(zoom_from_level(7), Zoom::Boxes));
        assert!(matches!(zoom_from_level(8), Zoom::Boxes));
    }

    // ── char_pan / drag-pan tests (item 1) ───────────────────────────────────

    #[test]
    fn char_pan_default_is_zero() {
        let s = AppState::default();
        assert_eq!(s.char_pan, (0, 0));
    }

    #[test]
    fn recenter_on_clears_char_pan() {
        let mut s = AppState::default();
        s.char_pan = (5, -3);
        s.recenter_on((0, 0), 80, 24);
        assert_eq!(s.char_pan, (0, 0), "recenter_on must reset char_pan to (0,0)");
    }

    #[test]
    fn reset_dialog_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.overlays.reset_dialog = true;
        assert!(s.any_overlay_open(), "reset_dialog open => any_overlay_open true");
    }

    #[test]
    fn game_over_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.overlays.game_over = true;
        assert!(s.any_overlay_open(), "game_over open => any_overlay_open true");
        s.overlays.game_over = false;
        assert!(!s.any_overlay_open(), "game_over false => any_overlay_open false");
    }

    #[test]
    fn quit_dialog_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.overlays.quit_dialog = true;
        assert!(s.any_overlay_open(), "quit_dialog open => any_overlay_open true");
        s.overlays.quit_dialog = false;
        assert!(!s.any_overlay_open(), "quit_dialog false => any_overlay_open false");
    }

    #[test]
    fn hints_panel_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());

        // Build a minimal HintSession using the minizork fixture (same approach as
        // the reset test in input.rs). If the fixture is absent we skip.
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes, true, false, None).expect("GameSession::new");
        s.overlays.hints = Some(HintSession {
            source: HintSource::Zcode(session),
            transcript: vec![],
            scroll: 0,
            clear_anchor: None,
            scroll_anim: None,
            input: String::new(),
            label: "Hints: Test".to_string(),
            builtin_hint: false,
        });
        assert!(s.any_overlay_open(), "hints open => any_overlay_open true");
        s.overlays.hints = None;
        assert!(!s.any_overlay_open(), "hints closed => any_overlay_open false");
    }

    #[test]
    fn hint_session_scroll_by_clamps_to_range() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes, true, false, None).expect("GameSession::new");
        let mut hs = HintSession {
            source: HintSource::Zcode(session),
            transcript: vec![],
            scroll: 0,
            clear_anchor: None,
            scroll_anim: None,
            input: String::new(),
            label: "Hints: Test".to_string(),
            builtin_hint: false,
        };
        // Instant (animation disabled) so the logical offset settles immediately.
        let anim = crate::config::AnimationConfig { enabled: false, easing: crate::anim::Easing::EaseOut, scroll_ms: 0, ..Default::default() };
        // Scrolling down (negative) at the top is clamped to 0.
        hs.scroll_by(-1, 5, &anim);
        assert_eq!(hs.scroll, 0, "scroll cannot go below 0");
        // Scrolling up (positive) advances within range.
        hs.scroll_by(3, 5, &anim);
        assert_eq!(hs.scroll, 3);
        // Scrolling past max is clamped to max.
        hs.scroll_by(10, 5, &anim);
        assert_eq!(hs.scroll, 5, "scroll clamps to max");
        // A max of 0 (nothing to scroll) pins scroll at 0.
        hs.scroll_by(4, 0, &anim);
        assert_eq!(hs.scroll, 0);
    }

    /// Build a minimal `HintSession` off the minizork fixture (its transcript
    /// starts empty so `apply_turn` math is easy to assert), or `None` if the
    /// fixture is absent (caller skips).
    fn make_hint_session() -> Option<HintSession> {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return None;
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes, true, false, None)
            .expect("GameSession::new");
        Some(HintSession {
            source: HintSource::Zcode(session),
            transcript: vec![],
            scroll: 0,
            clear_anchor: None,
            scroll_anim: None,
            input: String::new(),
            label: "Hints: Test".to_string(),
            builtin_hint: false,
        })
    }

    /// A `TurnResult` carrying just the fields `apply_turn` reads; everything
    /// else is empty/default.
    fn turn_result(transcript: &str, erase_lower: bool) -> crate::session::TurnResult {
        crate::session::TurnResult {
            transcript: transcript.to_string(),
            transcript_runs: vec![],
            location: None,
            quit: false,
            erase_lower,
            info: None,
            sounds: vec![],
            glulx_sound_ops: vec![],
            diagnostics: vec![],
            location_method: None,
            pending_io: None,
            timed_out: false,
            fault: None,
            pictures: Vec::new(),
            transcript_elems: vec![],
            prose_retired: None,
            declared_exit: None,
        }
    }

    #[test]
    fn apply_turn_collapses_successive_menu_reprints() {
        let Some(mut hs) = make_hint_session() else { return };
        hs.transcript.clear(); // fixture opening irrelevant; start clean

        // First menu redraw (screen clear): anchors at 0, appends its 3 lines.
        hs.apply_turn(&turn_result("m1a\nm1b\nm1c", true));
        assert_eq!(hs.transcript.len(), 3);
        assert_eq!(hs.clear_anchor, Some(0));

        // Second menu redraw: must collapse the first reprint, not stack on it.
        hs.apply_turn(&turn_result("m2a\nm2b", true));
        assert_eq!(
            hs.transcript.len(),
            2,
            "second erase_lower collapses the prior reprint back to the anchor"
        );
        assert_eq!(hs.transcript, vec!["m2a".to_string(), "m2b".to_string()]);
        assert_eq!(hs.scroll, 0);
    }

    #[test]
    fn apply_turn_normal_turn_appends() {
        let Some(mut hs) = make_hint_session() else { return };
        hs.transcript.clear();
        hs.transcript.push("existing".to_string());

        // A normal (non-clearing) turn appends its lines and leaves the anchor unset.
        hs.apply_turn(&turn_result("clue line 1\nclue line 2", false));
        assert_eq!(
            hs.transcript,
            vec![
                "existing".to_string(),
                "clue line 1".to_string(),
                "clue line 2".to_string()
            ]
        );
        assert_eq!(hs.clear_anchor, None, "no screen clear => no anchor");
        assert_eq!(hs.scroll, 0);
    }

    #[test]
    fn apply_turn_empty_turn_adds_no_blank_line_and_keeps_scroll() {
        let Some(mut hs) = make_hint_session() else { return };
        hs.transcript.clear();
        hs.transcript.push("clue A".to_string());
        hs.transcript.push("clue B".to_string());
        hs.scroll = 1; // user paged up to reread

        // A menu keystroke that only moves the upper-window highlight produces no
        // lower-window text: it must NOT append a blank line, and must NOT yank the
        // scroll back to the bottom.
        hs.apply_turn(&turn_result("", false));
        assert_eq!(
            hs.transcript,
            vec!["clue A".to_string(), "clue B".to_string()],
            "an empty turn must not append a blank line to the clue window"
        );
        assert_eq!(hs.scroll, 1, "an empty turn must not reset the scroll position");
    }

    #[test]
    fn run_search_direction_and_next_wrap() {
        let mut s = AppState::default();
        for t in ["alpha", "beta", "alpha again", "gamma", "ALPHA"] { s.push_transcript(t); }
        // matches for "alpha" at visible positions 0, 2, 4 (case-insensitive)
        let n = s.run_search("alpha", true); // start backward → last match
        assert_eq!(n, 3);
        assert_eq!(s.search_matches, vec![0, 2, 4]);
        assert_eq!(s.search_idx, 2); // index into search_matches → position 4
        // n = back
        assert_eq!(s.search_next(false), Some(2)); // now at match position 2
        // forward wraps from 2 → 4 → back to 0
        let _ = s.search_next(true); // → 4
        assert_eq!(s.search_next(true), Some(0)); // wrap to first
        let f = s.run_search("alpha", false); // start forward → first match
        assert_eq!(f, 3);
        assert_eq!(s.search_idx, 0);
        s.clear_search();
        assert!(s.search_query.is_none() && s.search_matches.is_empty());
    }

    #[test]
    fn map_trace_routes_render_steps_to_log_only_when_on() {
        let dir = std::env::temp_dir().join(format!("bm-maptrace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let steps = vec!["detect chains".to_string(), "route lanes".to_string()];
        // helper under test:
        write_map_trace(&dir, &steps, /* on = */ true);
        let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(body.contains("[map]    detect chains"), "{body:?}");

        let dir2 = std::env::temp_dir().join(format!("bm-maptrace-off-{}", std::process::id()));
        std::fs::create_dir_all(&dir2).unwrap();
        write_map_trace(&dir2, &steps, false);
        assert!(!dir2.join("trace.log").exists() || std::fs::read_to_string(dir2.join("trace.log")).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok(); std::fs::remove_dir_all(&dir2).ok();
    }
}

#[cfg(all(test, feature = "t-state"))]
mod play_sound_tests {
    use super::*;

    // Build an IFF chunk: type + BE len + data + pad-to-even. Mirrors
    // blorb::tests::chunk (not exported, so duplicated here).
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    // Build a Blorb with the given resources. Mirrors blorb::tests::build_blorb
    // (not exported, so duplicated here).
    type BlorbRes<'a> = (&'a [u8; 4], u32, &'a [u8; 4], &'a [u8]);
    fn build_blorb(res: &[BlorbRes]) -> Vec<u8> {
        let count = res.len() as u32;
        let ridx_data_len = 4 + 12 * res.len();
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut offsets = Vec::new();
        let mut cursor = first_res_off;
        let mut body = Vec::new();
        for (_u, _n, ty, data) in res {
            offsets.push(cursor as u32);
            let c = chunk(ty, data);
            cursor += c.len();
            body.extend_from_slice(&c);
        }
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&count.to_be_bytes());
        for (i, (usage, number, _ty, _d)) in res.iter().enumerate() {
            ridx.extend_from_slice(*usage);
            ridx.extend_from_slice(&number.to_be_bytes());
            ridx.extend_from_slice(&offsets[i].to_be_bytes());
        }
        let ridx_chunk = chunk(b"RIdx", &ridx);
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    // ── sound_kind_label ────────────────────────────────────────────────────

    #[test]
    fn sound_kind_label_maps_all_variants() {
        assert_eq!(sound_kind_label(blorb::SoundKind::Aiff), "AIFF");
        assert_eq!(sound_kind_label(blorb::SoundKind::Ogg), "OGG");
        assert_eq!(sound_kind_label(blorb::SoundKind::Mod), "MOD");
        assert_eq!(sound_kind_label(blorb::SoundKind::Other), "other");
    }

    // ── format_sound_resource_list ──────────────────────────────────────────

    #[test]
    fn resource_list_no_blorb_reports_none_resolved() {
        let lines = format_sound_resource_list(None, &Default::default());
        assert_eq!(lines, vec!["no sound blorb resolved, and none on the medium".to_string()]);
    }

    /// One effect on the medium, named the way the disk names it.
    fn disk_with(effect: u16, name: &str, rate: u32, aiff: Vec<u8>)
        -> std::collections::HashMap<u16, crate::native_sound::DiskSound>
    {
        let mut disk = std::collections::HashMap::new();
        disk.insert(
            effect,
            crate::native_sound::DiskSound {
                effect,
                name: name.to_string(),
                rate,
                frames: 33280,
                aiff,
            },
        );
        disk
    }

    /// With no Blorb, the list is the medium's — which on the two Infocom games that
    /// use sound is the whole inventory (SQ-0907). Saying only "no sound blorb
    /// resolved" over a disk holding fourteen effects is a report that misleads, and
    /// that is what the user saw.
    #[test]
    fn resource_list_leads_with_the_medium() {
        let disk = disk_with(3, "armor", 15360, Vec::new());
        let lines = format_sound_resource_list(None, &disk);
        assert_eq!(lines[0], "1 sound effect(s) on the medium:");
        assert!(lines[1].contains("#3"), "{:?}", lines[1]);
        assert!(lines[1].contains("armor"), "the sample's own name is what a person recognises");
        assert!(lines[1].contains("15360 Hz"));
    }

    /// With BOTH, the medium is the inventory and the Blorb is named as outranked
    /// (SQ-0914).
    ///
    /// The second half matters as much as the first: `stories/Sherlock.blb` sits
    /// beside the Sherlock floppy, so this is the shipped configuration, and a
    /// listing that silently omitted the Blorb would leave a person wondering why
    /// the file they filed made no difference.
    #[test]
    fn resource_list_says_when_a_blorb_is_present_but_outranked() {
        let disk = disk_with(3, "armor", 15360, Vec::new());
        let bytes = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd"), (b"Snd ", 3, b"FORM", b"aiffbytes")]);
        let blorb = blorb::Blorb::parse(bytes).unwrap();
        let lines = format_sound_resource_list(Some(&blorb), &disk);
        assert_eq!(lines[0], "1 sound effect(s) on the medium:");
        assert!(lines[1].contains("armor"));
        let last = lines.last().expect("a line about the blorb");
        assert!(last.contains("NOT used"), "{last:?}");
        assert!(last.contains("outrank"), "{last:?}");
        assert!(
            !lines.iter().any(|l| l.contains("sound resource(s):")),
            "the Blorb's inventory is not the inventory: {lines:?}",
        );
    }

    // ── resolve_sound ───────────────────────────────────────────────────────

    /// **The medium outranks a Blorb** (SQ-0914), which is the whole policy.
    ///
    /// Both sources carry effect 3 here, as they really do for *Sherlock* and *The
    /// Lurking Horror*, and the disk's bytes are the ones that come back. Falsified
    /// by swapping the two arms of `resolve_sound`, which returns the Blorb's.
    #[test]
    fn resolve_sound_prefers_the_medium_over_a_blorb() {
        let disk = disk_with(3, "armor", 15360, b"DISKAIFF".to_vec());
        let bytes = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd"), (b"Snd ", 3, b"FORM", b"blorbaiff")]);
        let blorb = blorb::Blorb::parse(bytes).unwrap();
        let (got, kind, name) = resolve_sound(&disk, Some(&blorb), 3).expect("effect 3 resolves");
        assert_eq!(got, b"DISKAIFF", "the disk is the rendition Infocom pressed");
        assert_eq!(kind, blorb::SoundKind::Aiff);
        assert_eq!(name, Some("armor"), "the report says which source answered");
    }

    /// A Blorb still answers where the disk has nothing — the policy is precedence,
    /// not exclusion, and a story with no medium at all is the ordinary case.
    #[test]
    fn resolve_sound_falls_through_to_a_blorb_and_reports_nothing_for_neither() {
        let disk = disk_with(3, "armor", 15360, b"DISKAIFF".to_vec());
        let bytes = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd"), (b"Snd ", 5, b"OGGV", b"oggdata!")]);
        let blorb = blorb::Blorb::parse(bytes).unwrap();
        let (got, kind, name) = resolve_sound(&disk, Some(&blorb), 5).expect("effect 5 resolves");
        assert_eq!(got, b"oggdata!");
        assert_eq!(kind, blorb::SoundKind::Ogg);
        assert_eq!(name, None, "no medium name, because the medium did not answer");

        assert!(resolve_sound(&disk, Some(&blorb), 9).is_none(), "in neither source");
        assert!(resolve_sound(&Default::default(), None, 3).is_none(), "no sources at all");
    }

    /// An undecodable Blorb resource comes back rather than reading as absent, so
    /// `/play-sound` can tell "found, but not decodable" from "not found at all".
    #[test]
    fn resolve_sound_returns_a_resource_it_cannot_decode() {
        let bytes = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd"), (b"Snd ", 9, b"WEIR", b"whatever")]);
        let blorb = blorb::Blorb::parse(bytes).unwrap();
        let (_, kind, _) = resolve_sound(&Default::default(), Some(&blorb), 9).expect("still found");
        assert_eq!(kind, blorb::SoundKind::Other);
        assert!(sound_kind_to_format(kind).is_none(), "and the caller is what decides it is unplayable");
    }

    #[test]
    fn resource_list_no_snd_resources() {
        let bytes = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")]);
        let blorb = blorb::Blorb::parse(bytes).unwrap();
        let lines = format_sound_resource_list(Some(&blorb), &Default::default());
        assert_eq!(lines, vec!["no Snd resources".to_string()]);
    }

    #[test]
    fn resource_list_enumerates_and_marks_playability() {
        let bytes = build_blorb(&[
            (b"Exec", 0, b"ZCOD", b"abcd"),
            (b"Snd ", 3, b"FORM", b"aiffbytes"),
            (b"Snd ", 5, b"OGGV", b"oggdata!"),
            (b"Snd ", 9, b"WEIR", b"whatever"),
        ]);
        let blorb = blorb::Blorb::parse(bytes).unwrap();
        let lines = format_sound_resource_list(Some(&blorb), &Default::default());
        assert_eq!(lines[0], "3 sound resource(s):");
        let three = lines.iter().find(|l| l.contains("#3")).unwrap();
        assert!(three.contains("AIFF") && three.contains("playable") && !three.contains("not decodable"));
        let five = lines.iter().find(|l| l.contains("#5")).unwrap();
        assert!(five.contains("OGG") && five.contains("playable") && !five.contains("not decodable"));
        let nine = lines.iter().find(|l| l.contains("#9")).unwrap();
        assert!(nine.contains("other") && nine.contains("not decodable"));
    }

    // ── format_play_sound_report ────────────────────────────────────────────

    #[test]
    fn report_resource_not_found() {
        let r = PlaySoundReport {
            disk_sounds: 0,
            from_medium: None,
            number: 42,
            enable_sound: true,
            backend_present: true,
            blorb_present: true,
            resource: None,
            format: None,
            sound_id: None,
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("NOT FOUND")));
    }

    #[test]
    fn report_undecodable_kind_stops_before_playback() {
        let r = PlaySoundReport {
            disk_sounds: 0,
            from_medium: None,
            number: 9,
            enable_sound: true,
            backend_present: true,
            blorb_present: true,
            resource: Some((blorb::SoundKind::Other, 128)),
            format: None,
            sound_id: None,
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("not decodable")));
        assert!(!lines.iter().any(|l| l.contains("playback")));
    }

    #[test]
    fn report_success_shows_sound_id() {
        let r = PlaySoundReport {
            disk_sounds: 0,
            from_medium: None,
            number: 3,
            enable_sound: true,
            backend_present: true,
            blorb_present: true,
            resource: Some((blorb::SoundKind::Aiff, 256)),
            format: Some(audio::SoundFormat::Aiff),
            sound_id: Some(7),
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("sound id 7")));
    }

    #[test]
    fn report_backend_none_when_playback_fails() {
        let r = PlaySoundReport {
            disk_sounds: 0,
            from_medium: None,
            number: 3,
            enable_sound: true,
            backend_present: true,
            blorb_present: true,
            resource: Some((blorb::SoundKind::Aiff, 256)),
            format: Some(audio::SoundFormat::Aiff),
            sound_id: None,
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("playback: backend returned None")));
    }

    #[test]
    fn report_notes_disabled_gate() {
        let r = PlaySoundReport {
            disk_sounds: 0,
            from_medium: None,
            number: 3,
            enable_sound: false,
            backend_present: true,
            blorb_present: true,
            resource: None,
            format: None,
            sound_id: None,
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("off (attempting playback anyway")));
    }
}
