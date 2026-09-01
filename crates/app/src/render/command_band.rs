//! The command band: a persistent bottom dock that composes a command from
//! progressive columns (Journey's command bar, minus the party column).
//!
//! Replaces the old left-edge verb/noun/prep token palette (SQ-0664). The
//! differences that matter:
//!
//! * Every verb carries the **sentence shapes it accepts** (see [`VerbLine`]),
//!   so the band knows which column comes next and can dim the ones that are
//!   not reachable yet.
//! * Those shapes, and the verbs themselves, are the **running story's own**
//!   (SQ-1111): `refresh_verbs` reads the game's grammar table through
//!   `crate::vocab::StoryVocabulary`, so the column that exists to say what is
//!   possible now says what is possible *in this game*. [`BUILTIN_VERBS`]
//!   survives only as the fallback for a story with no readable grammar, and a
//!   column that is not the story's own labels itself ([`VerbSource`]).
//! * The object columns are **live**: they are refreshed from the engine's
//!   object tree every turn the VM runs (`loop_tick::refresh_command_band`,
//!   gated on `AppState::turn_epoch` — SQ-1175), not scraped from the
//!   transcript and snapshotted at open.
//! * It is **not a modal**. The story prompt stays live, paste keeps working,
//!   graphical v6 stays on the pixel path, and only clicks inside the band's
//!   own rect are taken from the game.
//! * A column pick composes directly onto the REAL story input line
//!   (`state.input`), not a band-local phrase row — retired 2026-08-05
//!   (SQ-0667) along with the band's own frame and its VERB column header;
//!   see the module doc in `crate::state` (`CommandBandState`) for how the
//!   mirroring works and `docs/design/2026-08-05-verb-panel-redesign.md` for
//!   why. Nothing auto-submits: composing still never fires a turn by
//!   itself — Enter on the real prompt is the confirm, same as anything
//!   typed by hand. The ONE exception is the quick row: a quick pick fires
//!   at once, no Enter (the immediate-fire half of the same amendment).
//! * **Typing always wins** (SQ-0676, 2026-08-05): the band never owns text
//!   keys. It READS the prompt — the word under construction there highlights
//!   the nearest match in whatever column is current.
//! * **A current column, moved by Tab (SQ-0677, 2026-08-05 — supersedes
//!   SQ-0676's arrow scheme)**: `Tab`/`Shift-Tab` step it across the
//!   reachable columns, `↑`/`↓` highlight a row within it, and `Tab` with a
//!   row highlighted (explicit or the typed nearest match) picks it and
//!   advances — the same gesture a click already was. `Enter` always submits
//!   the prompt as typed; it never picks. `←`/`→` are plain cursor movement on
//!   the edit line. The quick block (rose + flowing words, and the flat-row
//!   fallback) is mouse-click-only, with a hover highlight its one transient
//!   state. See `docs/design/2026-08-05-verb-panel-redesign.md`'s dated
//!   amendments for the full gesture table and why it changed twice in one
//!   day.
//!
//! ```text
//!  NW  N  NE   ↑ │VERB     │WHAT — here │WHAT — carried│WITH…
//!   W  ·  E  ◉ ◎ │ look     │ window     │ brass key    │▸brass key
//!  SW  S  SE   ↓ │▸unlock   │▸iron door  │ lantern      │ lantern
//!  look inventory│
//!  wait again    │
//! ```
//!
//! (The `> unlock iron door with _` prompt line above this strip in the
//! mockup is the ordinary story input, drawn elsewhere — not part of this
//! module anymore.) The quick block STACKS — rose and portal-glyph cluster
//! on top, side by side, the remaining words flowing below both (SQ-0677's
//! geometry, SQ-1218's cluster) — rather than sitting beside the columns;
//! see the "Quick-block layout" section below. Single-cell `│` dividers
//! separate the block from VERB and every column from its neighbour, full
//! band height.
//!
//! The caller (`main.rs`) sizes `area` from the animated `PanelSlide`
//! fraction, so `area` may be shorter than the band's target height while a
//! slide is in flight — everything here clips to `area`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::state::AppState;

// ── Grammar: the story's own syntax, narrowed to what the band can compose ───

/// One sentence shape the band's four columns can compose, derived from a real
/// [`grammar_model::SyntaxLine`].
///
/// This is a **view of** the story's grammar, not a second model of it
/// (SQ-1111). The band used to carry an `Arity` enum — `Solo`/`Object`/
/// `ObjectOpt`/`Pair` plus a `prep` string — which was the grammar table's
/// structure rebuilt as a constant, and rebuilt *worse*: a real verb has
/// SEVERAL syntax lines (`take noun` and `take noun from noun` are two), and
/// one enum value per verb cannot express the alternation at all. A verb here
/// carries a LIST of these.
///
/// # What it deliberately drops
///
/// A syntax line whose literal words sit BEFORE its first object — Zork I's
/// `look at noun`, `stand up noun`, `carry up noun` — is not representable: the
/// band composes `verb object`, so it would emit `look lamp`, a command the
/// game refuses. [`VerbLine::from_syntax`] answers `None` for those, which is
/// also why the object columns stay dimmed on Zork I's `look` — every one of its
/// one-object lines needs a preposition first, and `look lamp` really is not a
/// sentence that story accepts. Offering a *lead* preposition column is a
/// feature, not a fix, and is not in this quest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerbLine {
    /// Object slots the player fills: 0, 1 or 2. (Two is the most any of the
    /// three engines' tables express, and the most the band has columns for.)
    pub nouns: usize,
    /// The words accepted between the first and second object, in table order —
    /// `unlock … **with** …`, `put … **in**/**into**/**inside** …`. The first is
    /// what the band composes with and what the second column labels itself; the
    /// rest are recognised when the player types them. Empty unless
    /// `nouns == 2`, and legitimately empty even then (Zork I's
    /// `throw noun noun`).
    pub joiners: Vec<String>,
}

impl VerbLine {
    /// A line the verb completes on its own — `look`, `wait`, `n`.
    pub fn bare() -> VerbLine {
        VerbLine { nouns: 0, joiners: Vec::new() }
    }

    /// A line taking one object — `take noun`.
    pub fn object() -> VerbLine {
        VerbLine { nouns: 1, joiners: Vec::new() }
    }

    /// A line taking two objects joined by `joiner` — `unlock noun with noun`.
    pub fn pair(joiner: &str) -> VerbLine {
        VerbLine { nouns: 2, joiners: vec![joiner.to_string()] }
    }

    /// Narrow a real syntax line to what the band can compose, or `None` when it
    /// cannot compose it at all (see the type's own doc for which shapes those
    /// are and why dropping them is the honest answer rather than a gap).
    pub fn from_syntax(line: &grammar_model::SyntaxLine) -> Option<VerbLine> {
        let mut nouns = 0usize;
        let mut joiners: Vec<String> = Vec::new();
        let mut literal_slots_after_first_noun = 0usize;
        for slot in &line.slots {
            if slot.is_noun_slot() {
                nouns += 1;
                if nouns > 2 {
                    return None;
                }
                continue;
            }
            // A literal before the first object, or after the second, is a word
            // the band has nowhere to put.
            if nouns != 1 {
                return None;
            }
            literal_slots_after_first_noun += 1;
            if literal_slots_after_first_noun > 1 {
                return None;
            }
            joiners = slot
                .alternatives
                .iter()
                .filter_map(grammar_model::Token::word)
                .map(str::to_lowercase)
                .collect();
            if joiners.is_empty() {
                return None;
            }
        }
        // A joiner with no second object to join to (`search for noun`) is the
        // leading-literal case wearing the other hat.
        if nouns < 2 && !joiners.is_empty() {
            return None;
        }
        Some(VerbLine { nouns, joiners })
    }
}

/// One verb the band offers, with every sentence shape it accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbEntry {
    pub word: String,
    /// The shapes, in table order. **Empty means "shape unknown"**, not "takes
    /// nothing": a word picked off the quick row is not in the table at all, and
    /// a verb whose every line the band cannot compose lands here too. Either
    /// way the band treats it as complete on its own, which is what a quick
    /// action IS.
    pub lines: Vec<VerbLine>,
    /// **The story lets this verb take an object**, whether or not the band
    /// knows how to compose one (SQ-1128).
    ///
    /// Deliberately NOT `max_nouns() > 0`. [`lines`](Self::lines) is what the
    /// band can BUILD, and it drops every shape whose literal precedes the
    /// first object — Zork I's look-verb has twelve syntax lines, eleven of them
    /// `gaze at/under/behind/in OBJ`, and not one survives into `lines`. Asked
    /// through `lines`, that verb answers "takes nothing", which is false about
    /// the story and is what hid `look` from the VERB column.
    ///
    /// [`verbs_from_grammar`] fills this from the RAW
    /// [`grammar_model::SyntaxLine`]s; [`VerbEntry::new`] derives it from the
    /// lines it is handed, which is the right answer for a table (the
    /// built-ins, `[command_band] verbs`) whose shapes ARE its whole grammar.
    pub takes_object: bool,
}

impl VerbEntry {
    pub fn new(word: &str, lines: Vec<VerbLine>) -> Self {
        let takes_object = lines.iter().any(|l| l.nouns > 0);
        VerbEntry { word: word.to_string(), lines, takes_object }
    }

    /// Record that the grammar gives this verb an object slot the band cannot
    /// compose — the `look at noun` case (SQ-1128). The shapes are unchanged:
    /// the object columns stay shut, and only the quick-row exclusion reads it.
    pub fn also_takes_object(mut self) -> Self {
        self.takes_object = true;
        self
    }

    /// The most objects any of this verb's lines takes — 0 for a verb that only
    /// ever stands alone, and what decides how far right the columns open.
    pub fn max_nouns(&self) -> usize {
        self.lines.iter().map(|l| l.nouns).max().unwrap_or(0)
    }

    /// True when some line of this verb takes exactly `nouns` objects — the
    /// question "is the phrase finished?". A verb with no known shape answers
    /// true for a bare phrase, as a quick action does.
    pub fn accepts(&self, nouns: usize) -> bool {
        if self.lines.is_empty() {
            return nouns == 0;
        }
        self.lines.iter().any(|l| l.nouns == nouns)
    }

    /// Every word this verb accepts between its two objects, in table order and
    /// deduplicated. This is the alternation `Arity` could not hold: a verb with
    /// `put noun in noun` and `put noun on noun` names both here, so typing
    /// either one moves the band on to the second-object column.
    pub fn joiners(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for line in self.lines.iter().filter(|l| l.nouns == 2) {
            for j in &line.joiners {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(j)) {
                    out.push(j.as_str());
                }
            }
        }
        out
    }

    /// The word the band composes a two-object phrase with, and the header the
    /// second-object column wears. The first of [`joiners`](Self::joiners) — the
    /// story's own table order decides, not us.
    pub fn joiner(&self) -> Option<&str> {
        self.joiners().first().copied()
    }
}

/// Where the VERB column's words came from.
///
/// The band's whole job is to say what is possible, so a column that is NOT the
/// running story's own grammar has to admit it rather than pass a generic list
/// off as fact — the same rule `here_is_seen` already follows for the object
/// columns (SQ-1111 / SQ-1117).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerbSource {
    /// The running story's own grammar table, read through
    /// [`crate::vocab::StoryVocabulary`]. The column is unlabelled: it is
    /// simply the truth.
    Story,
    /// [`BUILTIN_VERBS`] — the story's grammar could not be read (a menu-driven
    /// Version 6 game, one of SQ-1101's non-Inform files, a Glulx image whose
    /// dictionary→actions→grammar chain will not close). Labelled
    /// `VERB — generic`.
    #[default]
    Builtin,
    /// The player's own `[command_band] verbs` list. Labelled `VERB — yours`:
    /// it is not the story's grammar either, but it is not our guess.
    Configured,
}

impl VerbSource {
    /// The header the VERB column wears, or `None` when it needs none (and can
    /// keep spending that row on one more verb, as it has since SQ-0675).
    pub fn column_label(self) -> Option<&'static str> {
        match self {
            VerbSource::Story => None,
            VerbSource::Builtin => Some("VERB — generic"),
            VerbSource::Configured => Some("VERB — yours"),
        }
    }
}

/// The verb column's words together with where they came from.
///
/// One value rather than two arguments: a caller that has the words without
/// knowing whether they are the story's own will eventually label them wrong,
/// which is exactly the defect this quest fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbTable {
    pub entries: Vec<VerbEntry>,
    pub source: VerbSource,
}

impl VerbTable {
    pub fn new(entries: Vec<VerbEntry>, source: VerbSource) -> VerbTable {
        VerbTable { entries, source }
    }

    /// Drop the words `hidden` names — the adult list, applied to the assembled
    /// column (SQ-1122).
    ///
    /// **Display only.** The story still knows every word taken out: typing one
    /// parses exactly as it did before, and the Guiding Light still offers it
    /// (`crate::vocab` does not read the list). What this removes is the
    /// UNPROMPTED enumeration — a panel putting the word in front of somebody
    /// who only opened a panel.
    ///
    /// Exact, case-insensitive, whole-word. Never a prefix match: see
    /// [`crate::config::DEFAULT_ADULT_WORDS`] for the two real verbs a prefix
    /// rule would have eaten. An empty `hidden` is a no-op, which is how both of
    /// the config's off-switches restore the full column.
    ///
    /// Reached through [`crate::config::Config::resolve_band_verbs`] and
    /// [`layer_band_verbs`](crate::config::Config::layer_band_verbs), which are
    /// the two places a table is assembled; this is `pub` only so they can.
    pub fn hiding(mut self, hidden: &[String]) -> VerbTable {
        if hidden.is_empty() {
            return self;
        }
        self.entries
            .retain(|e| !hidden.iter().any(|h| h.eq_ignore_ascii_case(&e.word)));
        self
    }

    /// Drop the story's own test-harness and diagnostic verbs — every word whose
    /// first character is one of [`SIGILS`] (SQ-1126).
    ///
    /// Zork I r52 shipped `#record`, `#unrecord`, `#command`, `#random` and
    /// `$verify` in its retail grammar, and an alphabetical column put all five
    /// at the very TOP, which is the worst place a player could meet them. They
    /// are Infocom's regression rig — record a playthrough, replay it with the
    /// RNG pinned, diff — plus the §15 checksum check; none is part of the game.
    ///
    /// **A rule, not a list**, which is the whole reason it lives here and not
    /// in `adult_words`. That list is a judgement about words and therefore
    /// ships visibly for the player to edit; this is structure, needs no
    /// maintained vocabulary, and there is nothing to disagree with. Measured
    /// over the 60 stories in `stories/` with a readable Z grammar: 39 hold at
    /// least one such word (19 a `#`, all 39 a `$`), every one of them is a
    /// harness or diagnostic command, and no legitimate verb in the corpus
    /// begins with either character.
    ///
    /// **Display only**, exactly like [`hiding`](Self::hiding): typing `$verify`
    /// still reaches the parser, and it is the easiest way to see which
    /// interpreter number lanthorn reports to a game without a debug build.
    ///
    /// Reached through [`crate::config::Config::resolve_band_verbs`] and
    /// [`layer_band_verbs`](crate::config::Config::layer_band_verbs), which are
    /// the two places a table is assembled; this is `pub` only so they can.
    pub fn without_sigil_verbs(mut self) -> VerbTable {
        self.entries.retain(|e| !e.word.starts_with(SIGILS));
        self
    }
}

/// The two characters that mark a word as Infocom's rather than the player's:
/// `#` is the test harness (`#record`, `#command`, `#random`), `$` is
/// diagnostic (`$verify`, `$refresh`, `$credits`). See
/// [`VerbTable::without_sigil_verbs`].
///
/// The corpus holds two others — `*` in *Violet* and `@new`/`@up`/`@wall` in
/// *The Nameless* — which are author commands of the same kind but are not what
/// SQ-1126 was asked about, and `*` is a single character the column already
/// drops. Widening this is a decision, not an oversight.
const SIGILS: &[char] = &['#', '$'];

impl Default for VerbTable {
    fn default() -> Self {
        VerbTable { entries: Vec::new(), source: VerbSource::Builtin }
    }
}

/// The built-in verb table — the FALLBACK, no longer the source (SQ-1111).
///
/// A story whose grammar lanthorn can read drives the column itself; this is
/// what a story that has none falls back to, so the column is never empty. It
/// is a generic interactive-fiction verb set and says so on screen.
const BUILTIN_VERBS: &[(&str, &[usize], Option<&str>)] = &[
    ("look", &[0], None),
    ("inventory", &[0], None),
    ("wait", &[0], None),
    ("again", &[0], None),
    ("north", &[0], None),
    ("south", &[0], None),
    ("east", &[0], None),
    ("west", &[0], None),
    ("up", &[0], None),
    ("down", &[0], None),
    ("in", &[0], None),
    ("out", &[0], None),
    ("examine", &[1], None),
    ("take", &[1], None),
    ("drop", &[1], None),
    ("open", &[1], None),
    ("close", &[1], None),
    ("read", &[1], None),
    ("eat", &[1], None),
    ("drink", &[1], None),
    ("wear", &[1], None),
    ("remove", &[1], None),
    ("turn", &[1], None),
    ("enter", &[1], None),
    ("lock", &[2], Some("with")),
    ("search", &[0, 1], None),
    ("push", &[0, 1], None),
    ("pull", &[0, 1], None),
    ("climb", &[0, 1], None),
    ("move", &[0, 1], None),
    ("unlock", &[2], Some("with")),
    ("put", &[2], Some("in")),
    ("give", &[2], Some("to")),
    ("show", &[2], Some("to")),
    ("attack", &[2], Some("with")),
    ("tie", &[2], Some("to")),
];

/// The default one-click quick-action row.
///
/// The four diagonals sit with the rest of the compass (SQ-0676): the rose
/// draws all eight points, and leaving `ne`/`nw`/`se`/`sw` out left four of its
/// cells permanently empty. A game with no diagonal vocabulary (classic Scott
/// Adams) simply answers "I don't understand", exactly as it would to the same
/// word typed by hand — nothing here needs gating on the engine.
///
/// **Deliberately NOT derived from the grammar** (SQ-1111 asked): on the
/// Infocom family the compass is not in the verb table at all — `north` carries
/// the dictionary's "special" bit ($04) and never reaches a syntax line, so
/// Zork I's 134 verbs name no direction between them. The grammar cannot answer
/// the question, and the rose's eight slots are a fixed geometry besides.
pub const DEFAULT_QUICK: &[&str] = &[
    "n", "s", "e", "w", "ne", "nw", "se", "sw", "up", "down", "in", "out", "look", "inventory",
    "wait", "again",
];

/// Built-in verbs whose object arrives through a LEADING preposition, which
/// [`VerbLine`] cannot represent and the band cannot compose (SQ-1128).
///
/// `look at`, `look under`, `look behind`, `look in` — generic interactive
/// fiction accepts all four and none of them is `look noun`, so the entry keeps
/// its bare line and carries [`VerbEntry::takes_object`] beside it. That fact is
/// the whole difference between `look`, which the quick row's one click cannot
/// finish, and `wait`/`again`/`inventory`, which it can.
const BUILTIN_LEADING_PREP_OBJECT: &[&str] = &["look"];

/// The built-in verb table as owned entries — the fallback, labelled as such.
pub fn default_verbs() -> VerbTable {
    let entries = BUILTIN_VERBS
        .iter()
        .map(|&(w, nouns, prep)| {
            let lines = nouns
                .iter()
                .map(|&n| match (n, prep) {
                    (2, Some(p)) => VerbLine::pair(p),
                    (2, None) => VerbLine { nouns: 2, joiners: Vec::new() },
                    (1, _) => VerbLine::object(),
                    _ => VerbLine::bare(),
                })
                .collect();
            let entry = VerbEntry::new(w, lines);
            if BUILTIN_LEADING_PREP_OBJECT.contains(&w) {
                entry.also_takes_object()
            } else {
                entry
            }
        })
        .collect();
    VerbTable::new(entries, VerbSource::Builtin)
}

/// The story's own verb column: one row per dictionary spelling every verb of
/// its grammar answers to, alphabetically.
///
/// **Every spelling, not one per verb**, and that is the whole point. Infocom's
/// tables list a verb's synonyms in DICTIONARY order, so the first spelling is
/// merely the alphabetically-earliest one: Zork I's take-verb is `carry`, its
/// look-verb is `gaze`, its put-verb is `hide`, its throw-verb is `chuck`, and
/// its wave-verb is the truncated key `brandi`. Naming one spelling per verb
/// would hand the player a column of words no one would ever type. Listing them
/// all needs no heuristic, has zero false positives — every word here is one the
/// parser really accepts — and puts `take`, `look`, `put` and `throw` back where
/// the player expects them, beside their oddities.
///
/// One-character spellings (`x`, `g`, `z`, `l`, `q`) are dropped: they are real
/// vocabulary and a wasted row, the same call `vocab::StoryVocabulary`'s synonym
/// offer already makes. Where two verbs claim one spelling the first wins, as
/// both engines' readers do.
pub fn verbs_from_grammar(verbs: &[grammar_model::Verb]) -> Vec<VerbEntry> {
    let mut out: std::collections::BTreeMap<String, VerbEntry> = std::collections::BTreeMap::new();
    for verb in verbs {
        let mut lines: Vec<VerbLine> = Vec::new();
        for line in &verb.lines {
            if let Some(l) = VerbLine::from_syntax(line) {
                if !lines.contains(&l) {
                    lines.push(l);
                }
            }
        }
        // Asked of the RAW syntax lines, not of `lines`: "the story lets this
        // verb take an object" is a different question from "the band knows how
        // to compose one", and only the first decides whether a quick word is
        // redundant in the column (SQ-1128).
        let takes_object =
            verb.lines.iter().any(|l| l.slots.iter().any(grammar_model::Slot::is_noun_slot));
        for word in &verb.words {
            let word = word.to_lowercase();
            if word.chars().count() < 2 {
                continue;
            }
            out.entry(word.clone()).or_insert_with(|| {
                let e = VerbEntry::new(&word, lines.clone());
                if takes_object {
                    e.also_takes_object()
                } else {
                    e
                }
            });
        }
    }
    out.into_values().collect()
}

/// Refill the band's VERB column from the running story's own grammar.
///
/// Called from the same per-tick hook as [`refresh_objects`], and for the same
/// reason the band opens before it can ask: `Action::OpenCommandBand` has the
/// config but no engine, so the band is born on the fallback and swaps to the
/// story's own words on the tick before its first frame. Read ONCE per open —
/// the grammar table is static, so no later turn can change the answer — and
/// never over a `[command_band] verbs` list, which is the player's own.
///
/// Returns `true` when the column actually changed (→ repaint).
pub fn refresh_verbs(state: &mut AppState, session: &dyn crate::engine::Engine) -> bool {
    let Some(band) = state.overlays.command_band.as_ref() else { return false };
    if band.verbs_read || band.verb_source == VerbSource::Configured {
        return false;
    }
    // `VocabState` is the one vocabulary seam (SQ-1117): the same snapshot the
    // guidance offer reads, cached for the session, so this costs one grammar
    // read whichever of the two asks first.
    let mut vocab = std::mem::take(&mut state.vocab);
    let story = vocab.get(session).map(|v| verbs_from_grammar(v.verbs()));
    state.vocab = vocab;
    let table = match story {
        Some(entries) if !entries.is_empty() => {
            Some(state.config.layer_band_verbs(VerbTable::new(entries, VerbSource::Story)))
        }
        _ => None,
    };
    let Some(band) = state.overlays.command_band.as_mut() else { return false };
    band.verbs_read = true;
    match table {
        Some(t) => {
            if band.verbs == t.entries && band.verb_source == t.source {
                return false;
            }
            band.verbs = t.entries;
            band.verb_source = t.source;
            true
        }
        None => false,
    }
}

/// The default quick row as owned strings.
pub fn default_quick() -> Vec<String> {
    DEFAULT_QUICK.iter().map(|s| s.to_string()).collect()
}

// ── Columns ──────────────────────────────────────────────────────────────────

/// Number of columns the band lays out: VERB, WHAT—here, WHAT—carried, and the
/// prepositional second-object column.
pub const BAND_COLS: usize = 4;

/// Column indices, named. The object slot is offered as TWO columns (here /
/// carried) because that split is the whole point of having live objects; both
/// fill the same grammatical slot.
pub const COL_VERB: usize = 0;
pub const COL_HERE: usize = 1;
pub const COL_CARRIED: usize = 2;
pub const COL_SECOND: usize = 3;

// ── Geometry ─────────────────────────────────────────────────────────────────

/// Rows the band occupies when fully open: the configured `height`, clamped
/// to [`MIN_BAND_ROWS`]..=[`MAX_BAND_ROWS`] and then to what the screen can
/// actually spare (never more than half of it, and never so much that the
/// story pane is left with nothing). 0 when the band isn't visible.
///
/// Post-SQ-0667 there is no frame and no phrase line, so every row here is
/// content — unlike the pre-amendment band, which spent 3 of its 8 default
/// rows on chrome (a 2-row border plus the phrase line) before a single verb
/// was visible.
pub fn band_target_height(visible: bool, full_height: u16, rows: u16) -> u16 {
    if !visible {
        return 0;
    }
    let want = rows.clamp(MIN_BAND_ROWS, MAX_BAND_ROWS);
    // Leave the help row plus at least 3 rows of story pane.
    let hi = full_height.saturating_sub(4);
    want.min(hi)
}

/// Smallest useful band: headers + one list row + quick row.
pub const MIN_BAND_ROWS: u16 = 3;
/// Largest band the config may ask for.
pub const MAX_BAND_ROWS: u16 = 11;
/// The shipped default band height, in rows. 5, not the pre-SQ-0667 8 — the
/// band lost its 2-row frame and 1-row phrase line, and this is picked to
/// show exactly as many list rows as the old default did (3).
pub const DEFAULT_BAND_ROWS: u16 = 5;

/// The reserved band height in rows: `target_h` scaled by the slide's current
/// `fraction` (0.0 closed .. 1.0 fully open). Mirrors `inventory_dock_height`.
pub fn band_height(target_h: u16, fraction: f64) -> u16 {
    (target_h as f64 * fraction).round() as u16
}

/// Width of one vertical divider (SQ-0677): a single `│` cell, drawn between
/// the quick block and VERB and between every pair of adjacent columns.
pub const DIVIDER_W: u16 = 1;

/// Minimum width `column_rects` needs to lay out all `BAND_COLS` columns at
/// their own 6-cell floor PLUS a divider between every adjacent pair
/// (`BAND_COLS - 1` of them) — the same number `draw_command_band`'s
/// block-vs-fallback width check adds on top of the quick block's own width.
pub const MIN_COLS_WIDTH: u16 = BAND_COLS as u16 * 6 + (BAND_COLS as u16 - 1) * DIVIDER_W;

/// Split `content` (the band's inner rect, already narrowed past the quick
/// block and its divider if one is showing) into `BAND_COLS` column rects,
/// each separated by a `DIVIDER_W` gap the caller draws a `│` into
/// (`draw_command_band`). Returns an empty vec when the band is too narrow to
/// give each column a usable width — the divider cells are never part of any
/// returned rect, so a click that lands exactly on one hits neither neighbour.
pub fn column_rects(content: Rect) -> Vec<Rect> {
    if content.width < MIN_COLS_WIDTH {
        return Vec::new();
    }
    let dividers = BAND_COLS as u16 - 1;
    let usable = content.width - dividers * DIVIDER_W;
    let each = usable / BAND_COLS as u16;
    let mut rects = Vec::with_capacity(BAND_COLS);
    let mut x = content.x;
    for i in 0..BAND_COLS as u16 {
        rects.push(Rect { x, y: content.y, width: each, height: content.height });
        x += each;
        if i + 1 < BAND_COLS as u16 {
            x += DIVIDER_W;
        }
    }
    rects
}

/// Draw a full-band-height 1-cell vertical divider at column `x`. Reuses
/// `panel.border`'s style — the band draws no frame of its own (SQ-0667), but
/// a plain divider glyph is exactly what that selector already carries
/// elsewhere, and it is the same family `draw_command_band`'s resize-mode
/// highlight already borrows a color from (`panel.border:active`) — so no new
/// selector was needed for this (SQ-0677).
fn draw_divider(buf: &mut Buffer, x: u16, area: Rect, style: Style) {
    if x < area.x || x >= area.right() {
        return;
    }
    for y in area.y..area.bottom() {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.set_symbol("\u{2502}").set_style(style);
        }
    }
}

// ── Live objects ─────────────────────────────────────────────────────────────

/// Refill the band's object columns from the engine.
///
/// The old verb menu snapshotted a transcript scrape at open and never looked
/// again; the band is LIVE. Called once per loop tick (and therefore at least
/// once per turn), so taking an object moves it from *here* to *carried* on the
/// very next frame. Cheap, and skipped entirely while the band is closed.
///
/// Z-machine gets the real object tree through `Introspect`, and each row is the
/// word that story's parser accepts for the object rather than the name it
/// prints — see [`crate::vocab::typeable_name`] for why those are different sets
/// and what happens when a game prints `bird's nest` (SQ-1042). Glulx and Scott
/// have no tree, so `carried` falls back to the parsed-inventory snapshot and
/// `here` is empty.
///
/// **Every engine also gets the SEEN block** (SQ-1135): the words the story has
/// printed that name a THING, settled once a turn in
/// [`crate::input::refresh_seen_words`] and read from `AppState::seen_nouns`
/// here. That block used to be the
/// no-object-tree FALLBACK, which left the inversion the quest is named for —
/// the engine with the most introspection offering the least vocabulary, because
/// the tree looked authoritative enough to use alone. Arthur prints "imbedded in
/// one of the knobs is a sliver of crystal" and `crystal` was reachable in
/// neither column. It is a weaker claim than the tree's and is drawn as one
/// (dimmed, `band.item:seen`), under a header that says only what is true of the
/// whole column ([`crate::state::HereSource`]).
///
/// `here` excludes the player object itself (SQ-0667) — it is structurally a
/// child of whatever room the player is in, so without this it would show up
/// in every room of every game (Zork 1 lists "cretin", the adventurer's own
/// printed name). Excluded by id (`room_objects_excluding`), not by matching
/// the name, which could coincidentally collide with a real scenery object.
/// `examine me` still works by typing it; there is no dedicated row for it.
///
/// Returns `true` when the lists actually changed (→ repaint).
pub fn refresh_objects(state: &mut AppState, session: &dyn crate::engine::Engine) -> bool {
    let Some(band) = state.overlays.command_band.as_ref() else {
        return false;
    };
    // Objects only move when the VM runs, and every path that runs it bumps
    // `turn_epoch` (the three turn finishers via `begin_turn`, and a host
    // restore in `apply_archive_state`). An unchanged epoch means the walk
    // below — location detection, the room/carried object reads, a typeable
    // name per object — would recompute exactly what the band already holds,
    // which is what the ~20 Hz loop tick was doing on every pass (SQ-1175).
    let epoch = state.turn_epoch;
    if band.objects_epoch == Some(epoch) {
        return false;
    }
    let player = state
        .player_obj
        .or_else(|| session.introspect().and_then(|i| i.player_object()));
    let loc = session.current_location().map(|s| s.number).unwrap_or(0);

    // Read the objects first, then their words: the engine borrow above and the
    // vocabulary borrow below are separate, and the second lives on `state`.
    let objects = session.introspect().map(|intro| {
        let here = if loc != 0 { intro.room_objects_excluding(loc, player) } else { Vec::new() };
        // The NESTING read (SQ-1133), the same one `vocab::scope_split` gives
        // the reveal and completion: an opened sack's lunch is a word the parser
        // takes, so the column offers it, while a shut one's is not and the walk
        // stops at the lid. `contents` — the inventory DOCK's flat list of what
        // is in your hands — is a different question and was the wrong one here.
        let carried = player.map(|p| intro.visible_contents(p));
        (here, carried)
    });
    let (here, carried) = {
        let vocab = state.vocab.get(session);
        let typeable = |v: &[crate::engine::ObjectWords]| -> Vec<String> {
            let mut out: Vec<String> = Vec::with_capacity(v.len());
            for o in v {
                // Two things in one room reduce to one word often enough to
                // matter — Zork I's `wooden door` and `trap door` are both
                // `door` — and a row repeated is a row that says nothing extra.
                if let Some(name) = crate::vocab::typeable_name(o, vocab) {
                    if !out.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                        out.push(name);
                    }
                }
            }
            out
        };
        match &objects {
            Some((here, carried)) => (
                typeable(here),
                match carried {
                    Some(c) => typeable(c),
                    None => state.inventory_fallback.clone(),
                },
            ),
            None => (Vec::new(), state.inventory_fallback.clone()),
        }
    };
    // The words the story has PRINTED that name a THING, newest first — already
    // cut down once a turn by `input::refresh_seen_words`, which is where the
    // expensive half of that question lives (it asks the story's own objects).
    // Read in place and cloned only when it actually changed (SQ-1175).
    //
    // SQ-1135: every engine gets this block now, not just the ones with no
    // object tree.
    let seen = &state.seen_nouns;
    // The header claims only what is true of every row (SQ-1135). Read off the
    // LISTS rather than off which engine answered, so a story whose tree says the
    // room is empty labels its column honestly too.
    let source = match (here.is_empty(), seen.is_empty()) {
        (false, false) => crate::state::HereSource::Mixed,
        (false, true) => crate::state::HereSource::Scope,
        (true, false) => crate::state::HereSource::Seen,
        (true, true) => {
            if objects.is_some() {
                crate::state::HereSource::Scope
            } else {
                crate::state::HereSource::Seen
            }
        }
    };

    let Some(band) = state.overlays.command_band.as_mut() else { return false };
    // Recorded whether or not the lists changed: "nothing changed this turn" is
    // as much an answer as a new list, and re-asking every tick until the next
    // turn is the cost SQ-1175 removes.
    band.objects_epoch = Some(epoch);
    if band.here == here
        && band.carried == carried
        && band.here_seen == *seen
        && band.here_source == source
    {
        return false;
    }
    band.here = here;
    band.carried = carried;
    if band.here_seen != *seen {
        band.here_seen = seen.clone();
    }
    band.here_source = source;
    true
}

// ── Hit rects ────────────────────────────────────────────────────────────────

/// Click targets emitted while drawing, for the event loop to hit-test.
#[derive(Default, Clone)]
pub struct CommandBandHits {
    /// The band's whole rect — clicks inside it belong to the band and must not
    /// reach the story pane / the v6 game.
    pub area: Rect,
    /// Column header rects, by column index (focuses that column).
    pub headers: Vec<(usize, Rect)>,
    /// Item rows, as `(column, index-within-the-filtered-list, rect)`.
    pub rows: Vec<(usize, usize, Rect)>,
    /// Quick-action words, as `(index into the quick list, rect)`.
    pub quick: Vec<(usize, Rect)>,
    /// Whole-column rects, for wheel routing.
    pub columns: Vec<(usize, Rect)>,
}

// ── Drawing ──────────────────────────────────────────────────────────────────

/// Draw the command band into `area` (the bottom band carved out by
/// `layout::compute_pane_layout` from the slide fraction).
///
/// Sets `*vp_out` to the ACTIVE column's visible list height so PageUp/PageDown
/// page by the right amount, and publishes EVERY column's height into the
/// band's own `col_viewport` for the wheel, which scrolls the column under the
/// pointer rather than the current one (SQ-0832). No-op when the band is closed
/// or `area` is too small to show anything meaningful (mid-slide).
pub fn draw_command_band(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
    hits: &mut CommandBandHits,
) {
    *hits = CommandBandHits::default();
    let Some(band) = &state.overlays.command_band else { return };
    hits.area = area;
    // Each column republishes its own list height below; clear first so a
    // column this frame does not draw (mid-slide, or the narrow fallback that
    // shows only the current one) reads 0 rather than last frame's height, and
    // a wheel notch over it finds nothing to scroll. (SQ-0832)
    band.col_viewport.set([0; BAND_COLS]);

    if area.width < 8 || area.height == 0 {
        return;
    }

    let theme = &state.colors.theme;
    // Borderless strip (SQ-0667, 2026-08-05): the band's 2-row frame and its
    // "Command" title are gone — the fill below is now the band's ONLY
    // visual separation from the story pane above it. Resize mode still
    // needs some affordance that the band is the live target; with no border
    // left to accent, the whole fill picks up `panel.border:active` instead.
    // …and equally while its top edge is being DRAGGED (SQ-0669). Hover is
    // deliberately excluded here, unlike the bordered panes: with no border to
    // accent, the whole band would flash every time the pointer crossed the row
    // above it on its way in.
    let resize_hl = (state.resize_mode
        && state.resize_target == crate::state::ResizeTarget::CommandBand)
        || matches!(state.pane_drag, Some(d) if d.boundary == crate::layout::Boundary::CommandBandTop);
    let base = if resize_hl {
        theme.get("panel.border:active").style
    } else {
        theme.get("dialog.background").style
    };

    // Opaque fill first, so panes behind never show through mid-slide.
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(base);
            }
        }
    }

    let content = area;
    let divider_style = base.patch(theme.get("panel.border").style);
    let divider_active = base.patch(theme.get("panel.border:active").style);

    // ── Quick actions: a stacked rose+words block on the left edge when
    // there is room (SQ-0675, restacked under SQ-0677), else the flat
    // one-click row along the bottom (the original SQ-0667 layout, kept as
    // the narrow fallback). `use_block` narrows the columns by the block's
    // width plus one divider cell; see `draw_quick_block`'s doc for the
    // height interplay (the block's HEIGHT never steals from the columns,
    // which always get the band's full `content.height`). ─────────────────
    let quick_layout = if band.quick.is_empty() { None } else { Some(quick_block_layout(&band.quick)) };
    let use_block = quick_layout
        .as_ref()
        .is_some_and(|l| content.width >= l.width + DIVIDER_W + MIN_COLS_WIDTH);

    let mut cols_area = content;
    if use_block {
        let layout = quick_layout.as_ref().expect("use_block implies quick_layout is Some");
        let block_area = Rect {
            x: content.x,
            y: content.y,
            width: layout.width,
            height: content.height.min(layout.height),
        };
        draw_quick_block(band, layout, &state.symbols.portal, block_area, buf, base, theme, &mut hits.quick);
        let divider_x = content.x + layout.width;
        // The block/VERB divider is VERB's left flank: it carries the
        // current-column accent when VERB is current (see the column loop).
        let style = if band.focus == 0 { divider_active } else { divider_style };
        draw_divider(buf, divider_x, content, style);
        let claimed = layout.width + DIVIDER_W;
        cols_area = Rect {
            x: content.x + claimed,
            y: content.y,
            width: content.width - claimed,
            height: content.height,
        };
    } else if content.height >= 2 && !band.quick.is_empty() {
        let quick_area =
            Rect { x: content.x, y: content.bottom() - 1, width: content.width, height: 1 };
        draw_quick_row(band, quick_area, buf, base, theme, &mut hits.quick);
        cols_area =
            Rect { x: content.x, y: content.y, width: content.width, height: content.height - 1 };
    }

    // ── Columns ──────────────────────────────────────────────────────────────
    if cols_area.height == 0 || cols_area.width == 0 {
        *vp_out = 0;
        return;
    }
    // The band reads the real prompt: the word under construction there
    // highlights the nearest match in the CURRENT column (SQ-0677 scopes the
    // search to `band.focus`, which `Tab`/`Shift-Tab` move — see
    // `CommandBandState::nearest_match`'s doc for how that differs from the
    // SQ-0676 multi-column hunt it replaced).
    let typed = state.input.value.as_str();

    let rects = column_rects(cols_area);
    if rects.is_empty() {
        // Too narrow for four columns: show only the current one, full width.
        let highlight = band.highlighted_row(typed);
        draw_column(state, band, band.focus, cols_area, buf, base, true, highlight, vp_out, hits);
        return;
    }

    *vp_out = 0;
    for (col, rect) in rects.iter().enumerate() {
        hits.columns.push((col, *rect));
        let is_current = col == band.focus;
        let highlight = if is_current { band.highlighted_row(typed) } else { None };
        draw_column(state, band, col, *rect, buf, base, is_current, highlight, vp_out, hits);
    }
    // Dividers between adjacent columns — drawn last so they sit on top of
    // any column fill, full band height (not just the columns' own rect, in
    // case a shorter mid-slide `cols_area` ever diverges from `content`).
    // The CURRENT column's flanking dividers take the active accent — the
    // uniform current-column hint for all four columns (chrome, never list
    // rows; supersedes VERB's top-row underline, retired on user feedback).
    for (i, pair) in rects.windows(2).enumerate() {
        let flanks_current = band.focus == i || band.focus == i + 1;
        let style = if flanks_current { divider_active } else { divider_style };
        draw_divider(buf, pair[0].right(), content, style);
    }
}

// ── Compass-rose quick block (SQ-0675, restacked SQ-0677, given a portal
// glyph cluster SQ-1218) ────────────────────────────────────────────────────
//
// The flat quick row (below) is the narrow-band fallback; when there is
// room, the quick actions instead draw as a block on the band's left edge:
// the compass rose on top, a cluster of portal-direction glyphs beside it,
// the remaining words flowing below both (SQ-1218; SQ-0677's version flowed
// up/down/in/out as ordinary words under the rose, which spelled out four
// words the map already draws as one glyph apiece):
//
// ```text
//  NW  N  NE   ↑
//   W  ·  E  ◉ ◎
//  SW  S  SE   ↓
//  look inventory
//  wait again
// ```
//
// The 8 compass points (matched by MEANING via `compass_spelling`, not
// spelling — same rule the VERB column's quick exclusion uses) form a 3×3
// rose with an always-inert centre. up/down/in/out are directions too, but
// not compass POINTS, so `ROSE_ORDER` excludes them — they instead fill the
// portal-glyph cluster to the rose's right, drawn with the same
// `PortalGlyphs` icons (`↑`/`↓`/`◉`/`◎` by default, or whatever
// `[symbols].portal` names in `style.toml`) the map uses for an up/down/in/out
// exit, so a click reads as the same glyph the automap would show for it.
// Everything else in the effective quick list flows left-to-right then
// wraps, packed into as many rows as `WORD_ROW_BUDGET` needs at the
// narrowest width that achieves it. Every cell — rose point, cluster glyph or
// word — is a `hits.quick` entry keyed by its index into `band.quick`,
// exactly like the flat row's words: this is a different LAYOUT of the same
// one-click-submit contract (`input::band_quick_pick_command`,
// `main::band_mouse_action`), not a new one, so neither needed to change.
//
// **Height interplay (SQ-0677, revised SQ-1218):** the block's natural
// height (rose rows plus however many word rows `WORD_ROW_BUDGET` needs) may
// exceed the band's actual configured height. With up/down/in/out moved into
// the glyph cluster, the default quick list's word flow is down to 4 words
// (`look`/`inventory`/`wait`/`again`), which packs into exactly 2 rows at
// `WORD_ROW_BUDGET = 2` — so the shipped default band (5 rows: 3-row rose +
// 2 word rows) shows the whole default list with nothing clipped. The clip
// below still applies to a longer CUSTOM quick list, or to a shorter band:
// of the three options on the table (cap the block at the band's height and
// clip the rest; wrap overflow words into an extra COLUMN of the block;
// raise the band's effective minimum height whenever the block is shown),
// the first is what's implemented: `draw_command_band` sizes
// `block_area.height` to `content.height.min(layout.height)`, and
// `draw_quick_block` below silently stops drawing (and registering hits for)
// any row that falls off the bottom — the exact same clip-past-the-edge the
// renderer already did at a fixed 3 rows before the SQ-0677 amendment, just
// against a height that now varies with the word list instead of a
// constant. This was picked over the other two because it costs nothing
// elsewhere (the columns still always get the band's full height;
// `MIN_BAND_ROWS`/`DEFAULT_BAND_ROWS` stay singular, global constants
// unaffected by which quick list happens to be configured) and a taller
// band is one resize away for anyone who wants every word row of a longer
// custom list visible at once.

/// Target row budget the word flow tries to pack into (`word_flow_width`
/// grows the width, not this) — independent of the band's actual configured
/// height; see the height-interplay note above for why. Picked (SQ-1218) so
/// the default quick list's 4 remaining words (`look`/`inventory`/`wait`/
/// `again`, now that up/down/in/out are the glyph cluster's) land in exactly
/// the 2 rows the shipped default band (5 rows: 3-row rose + 2 word rows)
/// shows in full — a longer custom list still wraps to more rows and clips
/// against a short band, per the height-interplay note.
const WORD_ROW_BUDGET: u16 = 2;
/// Rows the compass rose itself occupies.
const ROSE_ROWS: u16 = 3;
/// Width of one rose cell: enough for the two-letter intercardinal labels
/// (`NW`/`NE`/`SW`/`SE`); the one-letter `N`/`S`/`E`/`W` and the centre `·`
/// are right-justified into the same width.
const ROSE_CELL_W: u16 = 2;
/// Columns between adjacent rose cells.
const ROSE_GAP: u16 = 1;
/// Total width of the 3×3 rose grid: three cells plus the two gaps between them.
const ROSE_WIDTH: u16 = ROSE_CELL_W * 3 + ROSE_GAP * 2;
/// Left margin before the block, and the gap separating it from the columns
/// that follow — matches the flat quick row's own one-column left margin.
const BLOCK_MARGIN: u16 = 1;

/// The rose's 8 outer grid slots, reading order (row-major, matching the
/// sketch above): NW N NE / W · E / SW S SE, with the centre omitted (it is
/// never a pick target).
const ROSE_LABELS: [&str; 8] = ["NW", "N", "NE", "W", "E", "SW", "S", "SE"];
const ROSE_ORDER: [mapper::direction::Direction; 8] = {
    use mapper::direction::Direction::*;
    [NW, N, NE, W, E, SW, S, SE]
};

/// The portal-glyph cluster's 4 slots (`CLUSTER_ORDER` order): up alone on
/// row 0, in/out side by side on row 1, down alone on row 2 — see the sketch
/// above `WORD_ROW_BUDGET`.
const CLUSTER_UP: usize = 0;
const CLUSTER_IN: usize = 1;
const CLUSTER_OUT: usize = 2;
const CLUSTER_DOWN: usize = 3;
const CLUSTER_ORDER: [mapper::direction::Direction; 4] = {
    use mapper::direction::Direction::*;
    [Up, In, Out, Down]
};
/// Columns between the "in" and "out" glyphs on the cluster's middle row.
const CLUSTER_INNER_GAP: u16 = 1;
/// Total width of the cluster: one glyph column, the inner gap, one more
/// glyph column — up/down centre on the middle column, directly over the gap.
const CLUSTER_WIDTH: u16 = 1 + CLUSTER_INNER_GAP + 1;
/// Columns between the rose's right edge and the cluster's left edge, when
/// both draw.
const CLUSTER_ROSE_GAP: u16 = 1;
/// X-offset of the "in" glyph within the cluster (its own left edge).
const CLUSTER_IN_X: u16 = 0;
/// X-offset of the "out" glyph within the cluster.
const CLUSTER_OUT_X: u16 = 1 + CLUSTER_INNER_GAP;
/// X-offset of the "up"/"down" glyphs within the cluster: centred on the
/// single column between "in" and "out".
const CLUSTER_UPDOWN_X: u16 = 1;

/// A resolved rose+cluster+words quick block: which `band.quick` index (if
/// any) fills each of the 8 rose slots, whether the rose draws at all, the word
/// flow's row assignment, and the total width/height the whole block needs —
/// computed once so the width-vs-fallback decision in `draw_command_band` and
/// the actual drawing in `draw_quick_block` always agree exactly.
struct QuickBlockLayout {
    /// `band.quick` index for each of the 8 rose slots (`ROSE_LABELS` order).
    rose: [Option<usize>; 8],
    has_rose: bool,
    /// `band.quick` index for each of the 4 portal-glyph cluster slots
    /// (`CLUSTER_ORDER` order: up, in, out, down).
    cluster: [Option<usize>; 4],
    has_cluster: bool,
    /// X-offset (past `BLOCK_MARGIN`) of the cluster's own left edge:
    /// `ROSE_WIDTH + CLUSTER_ROSE_GAP` when the rose also draws, else 0 (the
    /// cluster then starts flush with the block's margin, same as the rose
    /// does when it is alone). Meaningless when `has_cluster` is false.
    cluster_x: u16,
    /// One row per line of the word flow, each a list of `(index into
    /// band.quick, x-offset within the block, past `BLOCK_MARGIN`)`. Empty
    /// when the effective quick list has no non-compass, non-portal words
    /// at all.
    word_rows: Vec<Vec<(usize, u16)>>,
    /// Row (from the block's top) where the word flow starts: `ROSE_ROWS`
    /// when the rose or the cluster draws (words stack UNDER them), else 0
    /// (neither to stack under).
    words_y: u16,
    /// Total width the block needs, margins included: `BLOCK_MARGIN` +
    /// `max(rose width + gap + cluster width, widest word row)` +
    /// `BLOCK_MARGIN` — "as narrow as the widest word row" whenever that's
    /// wider than the rose and cluster together.
    width: u16,
    /// Total height the block wants: `words_y` + the word flow's row count.
    /// May exceed the band's actual content height; see the height-interplay
    /// note above `WORD_ROW_BUDGET` for how the caller handles that.
    height: u16,
}

/// The compass direction `word` is another SPELLING of — the band's own
/// question, and **not** `mapper::direction::parse_direction`'s (SQ-1130).
///
/// The two look alike and are not the same. A direction parser answers a
/// question about MOVEMENT: a mapper watching a ship has to read `GO BOW` as
/// north, because a bow points forward, and `PORT` as west. Those aliases are
/// facts about navigation. The band is asking something narrower — *is this
/// word already one click away on the quick row?* — which is a question about
/// vocabulary, and there `bow` is a verb.
///
/// Borrowing the parser for it hid `bow` from the VERB column in the twelve
/// corpus stories that have it (Sherlock, Trinity, Plundered Hearts, Beyond
/// Zork, Arthur, Shogun, Moonmist, Wishbringer ×2, Leather Goddesses ×2,
/// Bureaucracy), and routed a quick row spelling `port` into the rose's W
/// slot — drawn `W`, dispatching `port`. SQ-1128's rule (a quick word only
/// leaves the column when it cannot take an object) cured the first symptom
/// without touching the reuse that caused it; `stern`, `port` and `starboard`
/// are verbs in no story of the corpus **today**, which is the whole of why
/// this was latent rather than urgent.
///
/// `enter` and `exit` are gone for the same reason. The parser reads them as
/// in/out because a mapper must; `enter the boat` is a verb taking an object,
/// not another way to spell `in`.
///
/// What is left is one word per direction with the abbreviation the quick row
/// usually carries — two spellings of one thing, which is the whole of what
/// the band needs to know. The `go` prefix stays: `go north` is a spelling of
/// north, not an alias for something else.
pub(crate) fn compass_spelling(word: &str) -> Option<mapper::direction::Direction> {
    use mapper::direction::Direction as D;
    let lower = word.trim().to_lowercase();
    let mut tokens = lower.split_whitespace();
    let first = tokens.next()?;
    let w = if first == "go" { tokens.next()? } else { first };
    if tokens.next().is_some() {
        return None;
    }
    Some(match w {
        "n" | "north" => D::N,
        "s" | "south" => D::S,
        "e" | "east" => D::E,
        "w" | "west" => D::W,
        "ne" | "northeast" => D::NE,
        "nw" | "northwest" => D::NW,
        "se" | "southeast" => D::SE,
        "sw" | "southwest" => D::SW,
        "u" | "up" => D::Up,
        "d" | "down" => D::Down,
        "in" | "inside" => D::In,
        "out" | "outside" => D::Out,
        _ => return None,
    })
}

/// Split the effective quick list into the 8 compass-rose slots, the 4
/// portal-glyph cluster slots (each by index, so a click can resolve through
/// the same `band.quick.get(idx)` every other pick does), and everything
/// else, in original list order. A word is routed by the DIRECTION it names
/// ([`compass_spelling`]), not its spelling, so a custom quick row spelling
/// out `"north"` still lands in the rose's N slot rather than the word flow —
/// the same rule `CommandBandState::items`'s VERB exclusion already uses.
/// `up`/`down`/`in`/`out` are directions too but not compass POINTS, so
/// `ROSE_ORDER` excludes them on purpose — they route to `CLUSTER_ORDER`
/// instead (SQ-1218), drawn as the map's own portal glyphs rather than
/// spelled-out words.
fn split_quick_rose(quick: &[String]) -> ([Option<usize>; 8], [Option<usize>; 4], Vec<usize>) {
    let mut rose: [Option<usize>; 8] = [None; 8];
    let mut cluster: [Option<usize>; 4] = [None; 4];
    let mut words = Vec::new();
    for (i, w) in quick.iter().enumerate() {
        match compass_spelling(w) {
            Some(d) if ROSE_ORDER.contains(&d) => {
                let s = ROSE_ORDER.iter().position(|&r| r == d).expect("just checked");
                rose[s] = Some(i);
            }
            Some(d) if CLUSTER_ORDER.contains(&d) => {
                let c = CLUSTER_ORDER.iter().position(|&r| r == d).expect("just checked");
                cluster[c] = Some(i);
            }
            // Unreachable in practice — `compass_spelling`'s range is exactly
            // `ROSE_ORDER` plus `CLUSTER_ORDER` — but a vocabulary question is
            // never worth a panic, so an impossible third case just flows.
            Some(_) | None => words.push(i),
        }
    }
    (rose, cluster, words)
}

/// Rows a greedy, row-major left-to-right wrap of `words` needs at `width`: a
/// word starts a new row only when appending it (with a one-column separator)
/// would overflow the current row.
fn rows_needed(words: &[&str], width: u16) -> u16 {
    let mut rows = 1u16;
    let mut cur = 0u16;
    for word in words {
        let wl = word.chars().count() as u16;
        let extended = if cur == 0 { wl } else { cur + 1 + wl };
        if extended > width && cur > 0 {
            rows += 1;
            cur = wl;
        } else {
            cur = extended;
        }
    }
    rows
}

/// The narrowest width that packs `words` into `budget` rows under
/// `rows_needed`'s wrap — grown one column at a time from the longest single
/// word (the floor: nothing narrower could ever place that word). Small `n`
/// (the quick list is a handful of words at most) makes the linear search
/// cheap enough to redo on every draw rather than cache. By construction the
/// width this settles on IS the widest packed row's rendered width — which is
/// what makes it "as narrow as the widest word row" (the block-width doc
/// above `WORD_ROW_BUDGET` promises).
fn word_flow_width(words: &[&str], budget: u16) -> u16 {
    let Some(mut w) = words.iter().map(|s| s.chars().count() as u16).max() else { return 0 };
    while rows_needed(words, w) > budget {
        w += 1;
    }
    w
}

/// Assign each word (given as `band.quick` indices) to a row and an x-offset
/// within the word-flow area, using the exact wrap `word_flow_width` sized
/// the area for. Empty `idxs` produces zero rows (not one empty row) — the
/// height math above (`words_y + word_rows.len()`) depends on that: a
/// compass-only custom quick list must not pay for a phantom blank row.
fn flow_words(quick: &[String], idxs: &[usize], width: u16) -> Vec<Vec<(usize, u16)>> {
    let mut rows: Vec<Vec<(usize, u16)>> = Vec::new();
    let mut cur = 0u16;
    for &i in idxs {
        let wl = quick[i].chars().count() as u16;
        let extended = if cur == 0 { wl } else { cur + 1 + wl };
        if rows.is_empty() || (extended > width && cur > 0) {
            rows.push(Vec::new());
            cur = 0;
        }
        let x = if cur == 0 { 0 } else { cur + 1 };
        rows.last_mut().expect("just pushed if empty").push((i, x));
        cur = x + wl;
    }
    rows
}

/// Resolve `quick` into a [`QuickBlockLayout`]. Pure and cheap (see
/// `word_flow_width`'s doc) — called from `draw_command_band` purely to
/// answer the width-vs-fallback question, then again (necessarily the same
/// answer) to actually draw.
fn quick_block_layout(quick: &[String]) -> QuickBlockLayout {
    let (rose, cluster, word_idxs) = split_quick_rose(quick);
    let has_rose = rose.iter().any(Option::is_some);
    let has_cluster = cluster.iter().any(Option::is_some);
    let word_strs: Vec<&str> = word_idxs.iter().map(|&i| quick[i].as_str()).collect();
    let words_width = word_flow_width(&word_strs, WORD_ROW_BUDGET);
    let word_rows = flow_words(quick, &word_idxs, words_width);

    // The rose and the portal-glyph cluster sit side by side on the rose's
    // own 3 rows (SQ-1218); the word flow stacks under both, sharing their
    // left edge — so the block's width is whichever of "rose + gap + cluster"
    // and "widest word row" needs more room, never the two added together.
    let rose_w = if has_rose { ROSE_WIDTH } else { 0 };
    let cluster_gap = if has_rose && has_cluster { CLUSTER_ROSE_GAP } else { 0 };
    let cluster_w = if has_cluster { CLUSTER_WIDTH } else { 0 };
    let cluster_x = rose_w + cluster_gap;
    let top_w = rose_w + cluster_gap + cluster_w;
    let content_w = top_w.max(words_width);
    let width = BLOCK_MARGIN + content_w + BLOCK_MARGIN;
    let words_y = if has_rose || has_cluster { ROSE_ROWS } else { 0 };
    let height = words_y + word_rows.len() as u16;
    QuickBlockLayout { rose, has_rose, cluster, has_cluster, cluster_x, word_rows, words_y, width, height }
}

/// Draw the rose+cluster+words quick block into `area` (the band's left
/// strip, sized by the caller to `layout.width` ×
/// `content.height.min(layout.height)` — see the height-interplay note above
/// `WORD_ROW_BUDGET`).
///
/// Any row — rose, cluster or word — that falls at or past `area.bottom()` is
/// simply not drawn and registers no `hits.quick` entry: a short band shows
/// the rose and cluster and clips the word rows it has no room for, exactly
/// the clip-past-the-edge behaviour the pre-SQ-0677 renderer already had at a
/// fixed 3 rows.
///
/// Every rose point, cluster glyph and word registers the exact same `hits`
/// entry shape the flat row's words did (`(index into band.quick, rect)`), so
/// `input::band_quick_pick_command` and `main::band_mouse_action` need no
/// changes — a click resolves identically regardless of which layout drew it.
#[allow(clippy::too_many_arguments)]
fn draw_quick_block(
    band: &crate::state::CommandBandState,
    layout: &QuickBlockLayout,
    portal: &crate::symbols::PortalGlyphs,
    area: Rect,
    buf: &mut Buffer,
    base: Style,
    theme: &crate::theme::resolve::Theme,
    hits: &mut Vec<(usize, Rect)>,
) {
    let style = base.patch(theme.get("band.quick").style);
    // The centre is always inert decoration — never a pick target — styled
    // like the map matrix's own frontier dot rather than a new selector.
    let dim = base.patch(theme.get("map.matrix.cell:frontier").style);
    // Hover is the quick block's ONLY transient highlight now (SQ-0677 made
    // it mouse-click-only, retiring the arrow-armed selection this cell style
    // used to carry) — reversed video, deliberately distinct from the column
    // rows' `dialog.list_selected` (a fg/bg swap, not REVERSED — see the
    // design doc amendment for why reversed was safe to pick).
    let cell_style = |qi: usize| {
        if band.quick_hover == Some(qi) {
            style.patch(theme.get("band.quick:hover").style)
        } else {
            style
        }
    };

    if layout.has_rose {
        for (cell, label) in ROSE_LABELS.iter().enumerate() {
            // Reading order is row-major over the 3×3 grid with the centre
            // (grid index 4) skipped, so cells 0..4 map to grid cells 0..4
            // and cells 4..8 map to grid cells 5..9.
            let grid = if cell < 4 { cell } else { cell + 1 };
            let (row, gcol) = ((grid / 3) as u16, (grid % 3) as u16);
            let y = area.y + row;
            if y >= area.bottom() {
                continue;
            }
            let x = area.x + BLOCK_MARGIN + gcol * (ROSE_CELL_W + ROSE_GAP);
            let cell_area = Rect { x, y, width: ROSE_CELL_W, height: 1 };
            let Some(qi) = layout.rose[cell] else { continue };
            let lx = x + ROSE_CELL_W.saturating_sub(label.chars().count() as u16);
            let st = cell_style(qi);
            if band.quick_hover == Some(qi) {
                for cx in cell_area.x..cell_area.right() {
                    if let Some(c) = buf.cell_mut((cx, y)) {
                        c.set_symbol(" ").set_style(st);
                    }
                }
            }
            crate::render::draw_str_clipped(buf, lx, y, label, st, cell_area);
            hits.push((qi, cell_area));
        }
        // The centre dot, row 1 / col 1 of the grid.
        let cy = area.y + 1;
        if cy < area.bottom() {
            let cx = area.x + BLOCK_MARGIN + (ROSE_CELL_W + ROSE_GAP) + ROSE_CELL_W - 1;
            if let Some(c) = buf.cell_mut((cx, cy)) {
                c.set_symbol("\u{b7}").set_style(dim);
            }
        }
    }

    if layout.has_cluster {
        // Up alone on row 0 (centred), in/out side by side on row 1, down
        // alone on row 2 (centred) — the map's own portal glyphs, so a click
        // here reads as the same icon the automap draws for that exit.
        let cx0 = area.x + BLOCK_MARGIN + layout.cluster_x;
        let slots: [(usize, u16, u16, char); 4] = [
            (CLUSTER_UP, 0, CLUSTER_UPDOWN_X, portal.up),
            (CLUSTER_IN, 1, CLUSTER_IN_X, portal.in_),
            (CLUSTER_OUT, 1, CLUSTER_OUT_X, portal.out),
            (CLUSTER_DOWN, 2, CLUSTER_UPDOWN_X, portal.down),
        ];
        for (slot, row, x_off, glyph) in slots {
            let y = area.y + row;
            if y >= area.bottom() {
                continue;
            }
            let Some(qi) = layout.cluster[slot] else { continue };
            let x = cx0 + x_off;
            let cell_area = Rect { x, y, width: 1, height: 1 };
            crate::render::draw_char_clipped(buf, x, y, glyph, cell_style(qi), cell_area);
            hits.push((qi, cell_area));
        }
    }

    for (row_i, row) in layout.word_rows.iter().enumerate() {
        let y = area.y + layout.words_y + row_i as u16;
        if y >= area.bottom() {
            break;
        }
        for &(qi, x_off) in row {
            let word = &band.quick[qi];
            let x = area.x + BLOCK_MARGIN + x_off;
            if x >= area.right() {
                continue;
            }
            let w = (word.chars().count() as u16).min(area.right() - x);
            let r = Rect { x, y, width: w, height: 1 };
            crate::render::draw_str_clipped(buf, x, y, word, cell_style(qi), area);
            hits.push((qi, r));
        }
    }
}

/// The one-click quick-action row. SQ-0667 amendment (2026-08-05): unlike
/// every other pick, these fire AT ONCE — no Enter, and they don't compose
/// onto the input line either (see `input::band_quick_pick_command`).
fn draw_quick_row(
    band: &crate::state::CommandBandState,
    area: Rect,
    buf: &mut Buffer,
    base: Style,
    theme: &crate::theme::resolve::Theme,
    hits: &mut Vec<(usize, Rect)>,
) {
    let style = base.patch(theme.get("band.quick").style);
    let hover_style = theme.get("band.quick:hover").style;
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_symbol(" ").set_style(style);
        }
    }
    let mut x = area.x + 1;
    for (i, word) in band.quick.iter().enumerate() {
        let w = word.chars().count() as u16;
        if x + w > area.right() {
            break;
        }
        let r = Rect { x, y: area.y, width: w, height: 1 };
        // Hover reads here too (SQ-0677) — same reversed style the rose+words
        // block uses; the flat row is only ever a fallback for the same block.
        let st = if band.quick_hover == Some(i) { style.patch(hover_style) } else { style };
        crate::render::draw_str_clipped(buf, x, area.y, word, st, area);
        hits.push((i, r));
        x += w + 1;
    }
}

/// One column: its header row plus the (scrolled) item list.
///
/// `is_current` is whether `col` is `band.focus` — the ONE column `Tab`/
/// `Shift-Tab` point at (SQ-0677) — which is what lights the header (or, for
/// VERB, underlines its top row; see below) and drives `vp_out`. `highlight`
/// is the row to mark with `▸` and the selected style: the caller only ever
/// passes one for the current column (`CommandBandState::highlighted_row`),
/// so a non-current column never shows a highlight even if it happens to
/// have a passing nearest-match text.
#[allow(clippy::too_many_arguments)]
fn draw_column(
    state: &AppState,
    band: &crate::state::CommandBandState,
    col: usize,
    area: Rect,
    buf: &mut Buffer,
    base: Style,
    is_current: bool,
    highlight: Option<usize>,
    vp_out: &mut usize,
    hits: &mut CommandBandHits,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let theme = &state.colors.theme;
    let reachable = band.col_reachable(col);

    // SQ-0675: the VERB column's header row carried no text at all (SQ-0667
    // dropped the "VERB" label, leaving the row visually blank) — pure
    // wasted space in a deliberately compact band. It is reclaimed here as
    // the column's first LIST row instead of being drawn separately, so the
    // VERB column shows exactly ONE MORE visible verb than the object/prep
    // columns show items, at the same overall column height. Every other
    // column still gets a real header row: WHAT — here / WHAT — carried /
    // WITH… all carry real information (here vs. carried, and the pair
    // verb's preposition), so their row stays worth spending on a label.
    // VERB also contributes no `hits.headers` entry — there is no separate
    // header region left to focus; clicking its reclaimed top row just picks
    // the first verb there, exactly like clicking any other row.
    //
    // The current-column hint decorates CHROME, not list rows: the dividers
    // flanking the current column take `panel.border:active` (see the divider
    // loop in `draw_command_band`), uniformly for all four columns. VERB
    // therefore needs nothing special here despite having no header row —
    // its earlier top-row underline read as "this entry is underlined for
    // some reason" and was retired on user feedback (2026-08-05).
    //
    // SQ-1111 gives the row back for the ONE thing worth saying there: when the
    // column is NOT the story's own grammar — the generic fallback, or the
    // player's own `[command_band] verbs` list — it says so
    // (`VerbSource::column_label`), the same way `here_is_seen` relabels the
    // object column rather than passing a scrape off as the room's contents. A
    // story whose grammar we CAN read pays nothing: no label, no header, one
    // more verb visible, exactly as before.
    let header_h: u16 =
        if col == COL_VERB && band.verb_source.column_label().is_none() { 0 } else { 1 };
    if header_h > 0 {
        let header_style = base.patch(
            theme.get(if is_current { "band.column_header:active" } else { "band.column_header" }).style,
        );
        let header_area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
        for x in header_area.x..header_area.right() {
            if let Some(cell) = buf.cell_mut((x, header_area.y)) {
                cell.set_symbol(" ").set_style(header_style);
            }
        }
        let label = format!("{}{}", if is_current { "▸" } else { " " }, band.column_label(col));
        crate::render::draw_str_clipped(buf, header_area.x, header_area.y, &label, header_style, header_area);
        hits.headers.push((col, header_area));
    }
    let list_h = area.height.saturating_sub(header_h);
    if is_current {
        *vp_out = list_h as usize;
    }
    // …and the same height per column, for the wheel: it scrolls whichever
    // column the pointer is over, which need not be the current one, and VERB's
    // reclaimed header row makes that a genuinely different number. (SQ-0832)
    let mut vps = band.col_viewport.get();
    if let Some(slot) = vps.get_mut(col) {
        *slot = list_h as usize;
        band.col_viewport.set(vps);
    }
    if list_h == 0 {
        return;
    }
    let list_area = Rect { x: area.x, y: area.y + header_h, width: area.width, height: list_h };

    // A column that is not reachable yet renders dimmed and empty — the grammar
    // is the point, so showing pickable-looking rows there would lie.
    if !reachable {
        return;
    }

    let items = band.rows(col);
    let label_style = base.patch(theme.get("band.group_label").style);
    if items.is_empty() {
        // Column-specific wording (SQ-0667, following the SQ-0668 data fix
        // that made an empty carried/here column a real possibility rather
        // than blank space that reads as broken): "(nothing visible)" stays
        // the fallback for VERB/SECOND, which are never truly this empty in
        // practice (VERB always has the built-in table; SECOND only opens
        // once a pair verb's first object is picked, from the union of the
        // very lists this message would otherwise be reporting on for).
        let msg = match col {
            COL_HERE => "(nothing here)",
            COL_CARRIED => "(nothing carried)",
            _ => "(nothing visible)",
        };
        crate::render::draw_str_clipped(buf, list_area.x, list_area.y, msg, label_style, list_area);
        return;
    }

    let scroll = &band.scroll[col];
    let visible = list_area.height as usize;
    let total = items.len();
    // Only the nearest match highlights (SQ-0676). `usize::MAX` = nothing in
    // this column matches, so no row wears the marker at all — an unmatched
    // column must not look like it is offering a pick.
    let selected = highlight.unwrap_or(usize::MAX);
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total, visible) && list_area.width >= 2;
    let row_w = if scrollbar_visible { list_area.width.saturating_sub(1) } else { list_area.width };
    let offset = scroll.display_offset().min(total.saturating_sub(1));

    for row in 0..visible {
        let idx = offset + row;
        let y = list_area.y + row as u16;
        if y >= list_area.bottom() || idx >= total {
            break;
        }
        let is_selected = idx == selected;
        // A row the story merely PRINTED is dimmed (SQ-1135): it shares the
        // column with the object tree's rows because it fills the same slot, and
        // it is dimmed because it is a weaker claim — the story knows the word,
        // which is not a promise that the thing is here. The selection still
        // wins, so a dim row that is armed reads as armed.
        let style = if is_selected {
            theme.get("dialog.list_selected").style
        } else if items[idx].seen {
            base.patch(theme.get("band.item:seen").style)
        } else {
            base
        };
        let marker = if is_selected { "▸" } else { " " };
        let line = format!("{}{}", marker, items[idx].text);
        let row_area = Rect::new(list_area.x, y, row_w, 1);
        hits.rows.push((col, idx, row_area));
        for x in row_area.x..row_area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        crate::render::draw_str_clipped(buf, row_area.x, y, &line, style, row_area);
    }

    if scrollbar_visible {
        let sb = Rect::new(list_area.right().saturating_sub(1), list_area.y, 1, list_area.height);
        crate::render::scroll::draw_scrollbar(
            buf,
            sb,
            total,
            visible,
            scroll.target_offset(),
            crate::render::scroll::ScrollbarLook::from_theme(theme),
        );
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, CommandBandState};

    // Wide enough that the default quick list's compass-rose block (SQ-0675)
    // fits alongside four columns with real breathing room (so header/list
    // text like "(nothing carried)" never clips) — most of this module's
    // tests care about column content, not the quick block's own layout;
    // `narrow_band_falls_back_to_the_flat_quick_row` below covers the
    // narrower case on purpose.
    const BAND: Rect = Rect { x: 0, y: 0, width: 120, height: 8 };

    /// A band as a real session has one: the built-in words, but flying the
    /// story's own flag, which is the case every render test below is about.
    /// The FALLBACK's extra header row is the exception and is pinned by its
    /// own two cases (`verb_column_header_carries_no_text` and
    /// `the_verb_column_spends_a_row_only_when_it_has_something_to_admit`).
    fn story_table() -> VerbTable {
        VerbTable::new(default_verbs().entries, VerbSource::Story)
    }

    fn state_with_band() -> AppState {
        let mut s = AppState::default();
        let mut band = CommandBandState::new(story_table(), default_quick());
        band.here = vec!["iron door".to_string(), "mailbox".to_string()];
        band.carried = vec!["brass key".to_string(), "lantern".to_string()];
        s.overlays.command_band = Some(band);
        s
    }

    fn dump(buf: &Buffer) -> String {
        buf.content().iter().map(|c| c.symbol().to_owned()).collect()
    }

    #[test]
    fn band_shows_verbs_and_live_objects() {
        let mut buf = Buffer::empty(BAND);
        let s = state_with_band();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        // `take` — an ordinary object verb, unaffected by the quick-row
        // exclusion in either direction (see
        // `verb_column_excludes_quick_words_that_cannot_take_an_object`).
        assert!(out.contains("take"), "a verb");
        assert!(out.contains("carried"), "the carried column header");
    }

    #[test]
    fn unreachable_columns_render_no_rows() {
        // With no verb picked, only the VERB column is reachable, so no object
        // rows may be emitted as click targets.
        let mut buf = Buffer::empty(BAND);
        let s = state_with_band();
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        assert!(hits.rows.iter().all(|(c, _, _)| *c == COL_VERB), "only verb rows are pickable");
        assert!(!dump(&buf).contains("iron door"), "objects are not offered before a verb");
    }

    #[test]
    fn object_columns_open_after_an_object_verb() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("take");
        }
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);
        assert!(out.contains("iron door"), "here objects become pickable");
        assert!(out.contains("brass key"), "carried objects become pickable");
        assert!(hits.rows.iter().any(|(c, _, _)| *c == COL_HERE));
    }

    /// The wheel's viewport is a real per-column MEASUREMENT taken by this
    /// draw (SQ-0832), so this drives the actual wheel action through an
    /// actual frame instead of setting a height by hand. Drop the publish and
    /// every column reads 0, at which point `ListScroll::scroll_by` — quite
    /// correctly — declines to scroll a window whose size it does not know:
    /// the band's wheel goes DEAD rather than scrolling, which is exactly the
    /// silent failure this test exists to catch.
    #[test]
    fn the_draw_publishes_a_viewport_per_column_that_the_wheel_scrolls_in() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("take");
            b.here = (0..20).map(|i| format!("thing {i}")).collect();
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());

        let vps = s.overlays.command_band.as_ref().unwrap().col_viewport.get();
        assert!(vps[COL_HERE] > 0, "a drawn column publishes its list height");
        assert_eq!(
            vps[COL_VERB],
            vps[COL_HERE] + 1,
            "VERB is a row taller, having reclaimed its header row as a list row — which is \
             why one shared viewport could never have been right for all four columns"
        );

        // One notch over HERE scrolls THAT column's window by one row.
        let mut mapper = ::mapper::mapper::Mapper::default();
        crate::input::apply_action(
            crate::input::Action::BandWheel(COL_HERE, 1),
            &mut s,
            &mut mapper,
        );
        let b = s.overlays.command_band.as_ref().unwrap();
        assert_eq!(b.scroll[COL_HERE].target_offset(), 1, "the list scrolled a row");
        assert_eq!(b.scroll[COL_HERE].selected, 1, "…and the highlight rides its top edge");
    }

    /// SQ-0667 (2026-08-05): the band no longer draws its own frame, title, or
    /// phrase line — composing now happens on the real story input line
    /// (`state.input`), drawn elsewhere. Falsifies against the pre-amendment
    /// band, which drew "Command" in a title strip and "Enter: send" on a
    /// dedicated phrase row.
    #[test]
    fn band_no_longer_draws_a_frame_or_phrase_line() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("look");
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        assert!(!out.contains("Command"), "the title strip is retired along with the frame");
        assert!(!out.contains("Enter: send"), "the armed/send affordance moved to the real prompt");
        assert!(!out.contains('┌') && !out.contains('└'), "no border corners — a borderless strip");
    }

    /// SQ-0667: the VERB column shows no header text (it's self-evident); the
    /// object columns keep theirs, since WHAT/WITH carry real information.
    ///
    /// SQ-1111 leaves that intact for the column that IS the story's grammar,
    /// and only spends the row on the label when there is something to admit —
    /// checked in the second half here.
    #[test]
    fn verb_column_header_carries_no_text() {
        let s = state_with_band();
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        assert!(!dump(&buf).contains("VERB"), "the VERB label is gone");
        // SQ-0675: the header ROW itself is gone for VERB too now — reclaimed
        // as an extra list row (see `verb_column_shows_one_more_row...`
        // below) — so only the other three columns still contribute a
        // `hits.headers` entry.
        assert_eq!(
            hits.headers.len(),
            BAND_COLS - 1,
            "every column but VERB still has a clickable header row"
        );
    }

    /// SQ-0675: the VERB column's blank header row is reclaimed as its FIRST
    /// list row, so at the SAME column height it shows exactly one more
    /// visible item than a column that still spends a row on a real header
    /// (WHAT — here). Both columns are seeded with far more items than the
    /// area can show, so each is "full" and its row count is governed purely
    /// by how many rows `draw_column` gives its list. Falsifies against
    /// reverting the `header_h` special-case in `draw_column` back to an
    /// unconditional 1, which would make the two counts equal.
    #[test]
    fn verb_column_shows_one_more_row_than_an_object_column_at_the_same_height() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("take"); // opens HERE/CARRIED
            b.here = (0..10).map(|i| format!("thing{i}")).collect();
        }
        let area = Rect { x: 0, y: 0, width: 120, height: 6 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);

        let verb_rows = hits.rows.iter().filter(|(c, _, _)| *c == COL_VERB).count();
        let here_rows = hits.rows.iter().filter(|(c, _, _)| *c == COL_HERE).count();
        assert!(verb_rows > 0 && here_rows > 0, "both lists are long enough to fill every row");
        assert_eq!(
            verb_rows,
            here_rows + 1,
            "VERB's reclaimed header row is one extra visible verb: verb={verb_rows} here={here_rows}"
        );
    }

    #[test]
    fn prep_column_header_names_the_preposition() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("unlock");
            b.pick(COL_HERE, 0);
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(dump(&buf).contains("WITH"), "the second-object column names its preposition");
    }

    /// The quick actions are hit-testable regardless of which layout drew
    /// them — the compass-rose block (SQ-0675, `BAND` is wide enough to fit
    /// it) or the flat row it falls back to (see
    /// `narrow_band_falls_back_to_the_flat_quick_row` for that path
    /// specifically).
    #[test]
    fn quick_row_is_hit_testable() {
        let mut buf = Buffer::empty(BAND);
        let s = state_with_band();
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        assert!(!hits.quick.is_empty(), "quick words emit hit rects");
        assert_eq!(hits.headers.len(), BAND_COLS - 1, "every column but VERB has a clickable header row");
    }

    /// SQ-1111: the label is the ONLY thing that costs VERB its reclaimed list
    /// row, and it costs it exactly one. Falsify by making the header
    /// unconditional — the story's own column then shows one verb fewer for no
    /// reason.
    #[test]
    fn the_verb_column_spends_a_row_only_when_it_has_something_to_admit() {
        let mut s = state_with_band();
        let rows = |s: &AppState| -> usize {
            let mut buf = Buffer::empty(BAND);
            let mut hits = CommandBandHits::default();
            draw_command_band(s, BAND, &mut buf, &mut 0, &mut hits);
            hits.rows.iter().filter(|(c, _, _)| *c == COL_VERB).count()
        };
        let story = rows(&s);

        for (source, label) in
            [(VerbSource::Builtin, "VERB — generic"), (VerbSource::Configured, "VERB — yours")]
        {
            s.overlays.command_band.as_mut().unwrap().verb_source = source;
            assert_eq!(rows(&s), story - 1, "{label} costs exactly the reclaimed row");
            let mut buf = Buffer::empty(BAND);
            draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
            assert!(dump(&buf).contains(label), "{label} reaches the screen");
        }
    }

    /// SQ-0675: too narrow for the rose+words block (plus a usable four
    /// columns), the quick actions fall back to the original flat one-click
    /// row along the bottom — the pre-SQ-0675 layout, still exercised here.
    /// Falsifies against a `use_block` that ignores width entirely (which
    /// would draw the rose even into a sliver this narrow, clipping it).
    #[test]
    fn narrow_band_falls_back_to_the_flat_quick_row() {
        let s = state_with_band(); // default quick: n s e w up down in out look inventory wait again
        let area = Rect { x: 0, y: 0, width: 40, height: 8 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);

        assert!(!hits.quick.is_empty(), "quick words still emit hit rects");
        let out = dump(&buf);
        assert!(!out.contains("NW") && !out.contains("SE"), "too narrow for the rose diagram: {out}");
        // Early quick words only — width=40 isn't enough for the whole flat
        // row (the flat row breaks off once it runs out of space, same as it
        // always has), so don't assert on the tail end of the list.
        assert!(out.contains("down"), "the flat row still spells out the quick words: {out}");
        assert!(
            hits.quick.iter().all(|(_, r)| r.y == area.bottom() - 1),
            "the flat row is a single strip along the very bottom: {:?}",
            hits.quick
        );
    }

    /// SQ-0675: with room, the quick actions draw as a compass rose (8 outer
    /// cells, matched by MEANING via `parse_direction`) plus a word block —
    /// every cell is its own click target, resolving through the same
    /// `band_quick_pick_command` lookup the flat row always used (directions
    /// spell out in full on submission; the rose only abbreviates the
    /// DISPLAY). Falsifies against `draw_quick_block` not registering a hit
    /// for a rose cell (would leave `hits.quick.len()` short of `quick.len()`).
    #[test]
    fn rose_cells_are_present_and_clickable_when_width_allows() {
        let quick: Vec<String> =
            ["nw", "n", "ne", "w", "e", "sw", "s", "se", "look"].iter().map(|s| s.to_string()).collect();
        let mut s = AppState::default();
        s.overlays.command_band = Some(CommandBandState::new(default_verbs(), quick.clone()));
        let area = Rect { x: 0, y: 0, width: 120, height: 6 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);

        assert_eq!(hits.quick.len(), quick.len(), "every rose cell and word is clickable: {:?}", hits.quick);

        // No two quick hit rects overlap.
        for (i, (_, a)) in hits.quick.iter().enumerate() {
            for (_, b) in hits.quick.iter().skip(i + 1) {
                let overlap = a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom();
                assert!(!overlap, "quick hit rects must not overlap: {a:?} vs {b:?}");
            }
        }

        // Each hit resolves through the real submission lookup to the right
        // command — directions spell out in full, "look" passes through.
        let mut resolved: Vec<String> = hits
            .quick
            .iter()
            .map(|(idx, _)| crate::input::band_quick_pick_command(&s, *idx).expect("valid index"))
            .collect();
        resolved.sort();
        let mut want = vec![
            "north", "south", "east", "west", "northeast", "northwest", "southeast", "southwest", "look",
        ];
        want.sort();
        assert_eq!(resolved, want);
    }

    /// The word flow holds exactly the effective quick words that are NOT one
    /// of the 8 compass points AND NOT one of the 4 portal directions —
    /// `up`/`down`/`in`/`out` are directions too, but not compass POINTS, so
    /// they leave the word flow for the glyph cluster instead (SQ-1218; see
    /// `split_quick_rose`'s doc). Falsifies against a rose that (wrongly)
    /// swallows them, or a cluster that (wrongly) leaves them in the word
    /// flow — either would change this list.
    #[test]
    fn word_block_holds_exactly_the_non_compass_non_portal_quick_words() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        assert!(layout.has_rose, "n/s/e/w are compass words — the rose shows");
        assert!(layout.has_cluster, "up/down/in/out are portal words — the cluster shows");

        let mut got: Vec<&str> =
            layout.word_rows.iter().flatten().map(|&(i, _)| quick[i].as_str()).collect();
        got.sort_unstable();
        let mut want = vec!["look", "inventory", "wait", "again"];
        want.sort_unstable();
        assert_eq!(got, want, "the word flow holds exactly the non-compass, non-portal quick words");
    }

    /// The glyph cluster holds exactly `up`/`down`/`in`/`out`, each mapped to
    /// its own `band.quick` index in `CLUSTER_ORDER` order (up, in, out,
    /// down). Falsifies against a cluster that mismatches a direction to the
    /// wrong slot or leaves one out.
    #[test]
    fn glyph_cluster_holds_exactly_the_four_portal_words_in_order() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        let up = quick.iter().position(|q| q == "up").unwrap();
        let down = quick.iter().position(|q| q == "down").unwrap();
        let in_ = quick.iter().position(|q| q == "in").unwrap();
        let out = quick.iter().position(|q| q == "out").unwrap();
        assert_eq!(layout.cluster, [Some(up), Some(in_), Some(out), Some(down)]);
    }

    /// The glyph cluster draws the map's own portal icons (SQ-1218) — default
    /// `↑`/`◉`/`◎`/`↓`, one per cell, each its own click target that resolves
    /// through `band_quick_pick_command` to the bare direction, same as the
    /// rose's own cells. Falsifies against the cluster drawing text (`"up"`
    /// spelled out) instead of the glyph, or against a click landing on the
    /// wrong word.
    #[test]
    fn glyph_cluster_draws_portal_glyphs_and_is_clickable() {
        let s = state_with_band(); // default quick, default (ascii) portal glyphs
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);

        for glyph in ['\u{2191}', '\u{2193}', '\u{25c9}', '\u{25ce}'] {
            assert!(out.contains(glyph), "the cluster draws the default portal glyph {glyph:?}: {out}");
        }
        // The word flow no longer spells these out — only the compass words
        // and the plain quick words remain as text.
        assert!(!out.contains("up") && !out.contains("down"), "no spelled-out up/down: {out}");

        let quick = &s.overlays.command_band.as_ref().unwrap().quick;
        for word in ["up", "down", "in", "out"] {
            let qi = quick.iter().position(|q| q == word).unwrap();
            let (_, rect) = *hits.quick.iter().find(|(i, _)| *i == qi).unwrap_or_else(|| {
                panic!("{word} (idx {qi}) should have a hit rect: {:?}", hits.quick)
            });
            assert_eq!(rect.width, 1, "a glyph cell is exactly one column wide");
            assert_eq!(
                crate::input::band_quick_pick_command(&s, qi),
                Some(word.to_string()),
                "clicking the {word} glyph submits {word}"
            );
        }
    }

    /// A `style.toml` portal-glyph preset override reaches the cluster the
    /// same way it reaches the map (SQ-1218): both read
    /// `state.symbols.portal`. Falsifies against the cluster hard-coding the
    /// default `PortalGlyphs::preset("ascii")` instead of reading the active
    /// symbol set.
    #[test]
    fn glyph_cluster_honors_a_custom_portal_glyph_preset() {
        let mut s = state_with_band();
        s.symbols = crate::symbols::SymbolSet::from_preset_names("ascii", "filled", "nerdfont-stairs", "light");
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);

        let portal = &s.symbols.portal;
        assert_ne!(portal.up, '\u{2191}', "sanity: nerdfont-stairs does not reuse the ascii glyph");
        for glyph in [portal.up, portal.down, portal.in_, portal.out] {
            assert!(out.contains(glyph), "the cluster draws the configured preset's glyph {glyph:?}: {out}");
        }
        assert!(!out.contains('\u{2191}'), "the default ascii glyph must not leak through: {out}");
    }

    /// A custom quick list with no compass words at all draws no rose —
    /// just the flowing word list, starting where the rose's margin would
    /// otherwise have been. Falsifies against a rose that always draws (even
    /// empty), which would waste the rose's width for nothing.
    #[test]
    fn quick_list_without_compass_words_shows_no_rose() {
        let quick: Vec<String> = ["look", "wait", "again"].iter().map(|s| s.to_string()).collect();
        let layout = quick_block_layout(&quick);
        assert!(!layout.has_rose, "no compass word in quick -> no rose");
        assert_eq!(layout.words_y, 0, "no rose to stack the word flow under, so it starts at row 0");

        let s = {
            let mut s = AppState::default();
            s.overlays.command_band = Some(CommandBandState::new(default_verbs(), quick));
            s
        };
        let area = Rect { x: 0, y: 0, width: 120, height: 6 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);
        assert!(!out.contains('·'), "no rose -> no inert centre dot either: {out}");
        assert_eq!(hits.quick.len(), 3, "just the three words: {:?}", hits.quick);
    }

    /// SQ-0676 added the four diagonals to the built-in quick list, so the
    /// default rose has no empty cells left. Falsifies against the pre-SQ-0676
    /// `DEFAULT_QUICK`, where NW/NE/SW/SE were blank.
    #[test]
    fn the_default_quick_list_fills_all_eight_rose_cells() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        assert!(layout.rose.iter().all(Option::is_some), "every compass point has a word");

        let mut s = AppState::default();
        s.overlays.command_band = Some(CommandBandState::new(default_verbs(), quick.clone()));
        let area = Rect { x: 0, y: 0, width: 120, height: 6 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);
        for label in ["NW", "NE", "SW", "SE"] {
            assert!(out.contains(label), "the rose draws {label}: {out}");
        }
        // …and a diagonal submits spelled out, like every other direction.
        let ne = quick.iter().position(|q| q == "ne").expect("ne is in the default quick list");
        assert_eq!(crate::input::band_quick_pick_command(&s, ne), Some("northeast".to_string()));
    }

    /// SQ-0676: the word being typed at the PROMPT highlights the nearest match
    /// — the band reads the input line rather than owning a filter of its own.
    /// Falsifies against a band that highlights `scroll.selected` regardless of
    /// what is typed (the pre-SQ-0676 behaviour), which marked row 0 of the
    /// focused column no matter what.
    #[test]
    fn the_typed_word_highlights_the_nearest_match() {
        let mut s = state_with_band();
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(!dump(&buf).contains('▸'), "nothing typed -> nothing highlighted");

        // `examine` heads the VERB column, so it is on screen without relying
        // on the scroll-to-match `apply_action` does for the live app.
        s.input.set("exa".to_string(), true);
        s.overlays.command_band.as_mut().unwrap().sync_from_input(&s.input.value);
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(dump(&buf).contains("▸examine"), "the nearest verb wears the marker");

        s.input.set("zzzz".to_string(), true);
        s.overlays.command_band.as_mut().unwrap().sync_from_input(&s.input.value);
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(!dump(&buf).contains('▸'), "no match -> no highlight at all");
    }

    /// The hovered quick word is visibly restyled (SQ-0677: quick lost its
    /// arrow-armed keyboard state and gained mouse hover as its one
    /// transient highlight instead). Falsifies against drawing every quick
    /// cell in the same `band.quick` style regardless of `quick_hover`.
    #[test]
    fn the_hovered_quick_word_is_drawn_differently() {
        let s_plain = state_with_band();
        let mut buf_plain = Buffer::empty(BAND);
        draw_command_band(&s_plain, BAND, &mut buf_plain, &mut 0, &mut CommandBandHits::default());

        let mut s_hover = state_with_band();
        s_hover.overlays.command_band.as_mut().unwrap().quick_hover = Some(0);
        let mut buf_hover = Buffer::empty(BAND);
        draw_command_band(&s_hover, BAND, &mut buf_hover, &mut 0, &mut CommandBandHits::default());

        assert_ne!(
            buf_plain.content(),
            buf_hover.content(),
            "hovering a quick word must show somewhere on screen"
        );
    }

    /// The hover style is REVERSED video — deliberately distinct from the
    /// column rows' `dialog.list_selected` (a fg/bg swap, not a REVERSED
    /// modifier — see `theme::registry`), so hovering a quick cell can never
    /// be mistaken for a column row's picked/armed highlight even though
    /// both can be visible at once (hover in the quick block, the highlight
    /// in a column). Falsifies against a hover style that happens to collide
    /// with `dialog.list_selected`'s own modifiers.
    #[test]
    fn hover_style_is_reversed_and_distinct_from_the_column_selected_style() {
        use ratatui::style::Modifier;
        let mut s = state_with_band();
        s.overlays.command_band.as_mut().unwrap().quick_hover = Some(0);
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);

        let (_, r) = hits.quick.iter().find(|(i, _)| *i == 0).expect("hovered cell has a hit rect");
        let cell = buf.cell((r.x, r.y)).expect("hovered cell drawn");
        assert!(
            cell.style().add_modifier.contains(Modifier::REVERSED),
            "the hovered cell is reversed video: {:?}",
            cell.style()
        );

        let sel_style = s.colors.theme.get("dialog.list_selected").style;
        assert!(
            !sel_style.add_modifier.contains(Modifier::REVERSED),
            "sanity: the column-row selected style is a colour swap, not REVERSED — \
             otherwise hover and armed would be indistinguishable"
        );
    }

    /// The flat-row fallback hovers too (SQ-0677) — the block is the mouse's
    /// only path to the quick actions when it's showing, so hover must not
    /// be a rose-only affordance.
    #[test]
    fn the_flat_row_hovers_too() {
        let s = state_with_band();
        let area = Rect { x: 0, y: 0, width: 40, height: 8 };
        let mut buf_plain = Buffer::empty(area);
        draw_command_band(&s, area, &mut buf_plain, &mut 0, &mut CommandBandHits::default());

        let mut s_hover = state_with_band();
        s_hover.overlays.command_band.as_mut().unwrap().quick_hover = Some(0);
        let mut buf_hover = Buffer::empty(area);
        draw_command_band(&s_hover, area, &mut buf_hover, &mut 0, &mut CommandBandHits::default());

        assert_ne!(buf_plain.content(), buf_hover.content(), "the flat row's hover shows too");
    }

    /// The WHAT header's three states (SQ-1135). The rule is that the label has
    /// to be true of the WHOLE column, so a column that mixes the object tree's
    /// rows with the story's printed words can claim neither qualifier.
    #[test]
    fn the_what_header_says_only_what_is_true_of_the_whole_column() {
        let header = |source: crate::state::HereSource, seen: Vec<String>| {
            let mut s = state_with_band();
            {
                let b = s.overlays.command_band.as_mut().unwrap();
                b.here_source = source;
                b.here_seen = seen;
                b.pick_word("take");
            }
            let mut buf = Buffer::empty(BAND);
            draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
            dump(&buf)
        };
        use crate::state::HereSource;

        let scope = header(HereSource::Scope, Vec::new());
        assert!(scope.contains("WHAT — here"), "the object tree's own rows: {scope}");

        // Glulx and Scott, unchanged: no object tree, so every row is a scrape
        // and the column labels itself rather than passing one off as the other.
        let seen = header(HereSource::Seen, vec!["lamp".to_string()]);
        assert!(seen.contains("WHAT — seen"), "a column that is all scrape: {seen}");
        assert!(!seen.contains("WHAT — here"), "{seen}");

        // `▸` is the current-column marker, so this is the WHAT header itself
        // and not the neighbouring `WHAT — carried`, which is untouched.
        let mixed = header(HereSource::Mixed, vec!["crystal".to_string()]);
        assert!(mixed.contains("▸WHAT "), "{mixed}");
        assert!(!mixed.contains("WHAT — here"), "neither qualifier is true of all of it: {mixed}");
        assert!(!mixed.contains("WHAT — seen"), "{mixed}");
    }

    /// A printed-word row is drawn dimmed and a scope row is not (SQ-1135) —
    /// same column, weaker claim, visibly so.
    #[test]
    fn a_seen_row_is_dimmed_and_a_scope_row_is_not() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.here = vec!["torque".to_string()];
            b.here_seen = vec!["crystal".to_string()];
            b.here_source = crate::state::HereSource::Mixed;
            b.pick_word("take");
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());

        // The style of the cell each word starts in.
        let style_of = |word: &str| {
            for y in BAND.y..BAND.bottom() {
                let row: String =
                    (BAND.x..BAND.right()).map(|x| buf.cell((x, y)).unwrap().symbol()).collect();
                if let Some(at) = row.find(word) {
                    return buf.cell((BAND.x + at as u16, y)).unwrap().style();
                }
            }
            panic!("{word:?} is not on screen:\n{}", dump(&buf));
        };
        let scope = style_of("torque");
        let seen = style_of("crystal");
        println!("scope row {scope:?}\nseen row  {seen:?}");
        assert_ne!(scope, seen, "the weaker claim must look weaker");
        assert_eq!(
            seen.fg,
            s.colors.theme.get("band.item:seen").style.fg,
            "and it must take its colour from the selector, not a literal",
        );
    }

    /// A genuinely empty carried/here column must say so explicitly, not
    /// render as blank space that reads as broken (SQ-0668 made an empty
    /// column, with real data now behind it, an actual possibility — not
    /// just a scrape-fallback artifact).
    #[test]
    fn empty_object_columns_say_so() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.here = Vec::new();
            b.carried = Vec::new();
            b.pick_word("take");
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        assert!(out.contains("nothing here"), "the here column says it's empty: {out}");
        assert!(out.contains("nothing carried"), "the carried column says it's empty: {out}");
    }

    /// …and the message is gone the moment either column actually has
    /// something in it — falsifies against always showing the empty row.
    #[test]
    fn nonempty_object_columns_do_not_say_empty() {
        let mut s = state_with_band(); // here/carried both non-empty by construction
        s.overlays.command_band.as_mut().unwrap().pick_word("take");
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        assert!(!out.contains("nothing here"), "the here column has objects: {out}");
        assert!(!out.contains("nothing carried"), "the carried column has objects: {out}");
    }

    #[test]
    fn band_is_opaque() {
        let mut buf = Buffer::empty(BAND);
        for y in 0..BAND.height {
            for x in 0..BAND.width {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_symbol("X").set_style(
                        Style::new()
                            .fg(ratatui::style::Color::Red)
                            .bg(ratatui::style::Color::Green),
                    );
                }
            }
        }
        let s = state_with_band();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(
            !buf.content().iter().any(|c| c.style().bg == Some(ratatui::style::Color::Green)),
            "the band paints over whatever was behind it"
        );
    }

    #[test]
    fn tiny_band_does_not_panic() {
        for (w, h) in [(4u16, 8u16), (78, 2), (10, 3), (0, 0)] {
            let area = Rect { x: 0, y: 0, width: w, height: h };
            let mut buf = Buffer::empty(Rect { x: 0, y: 0, width: w.max(1), height: h.max(1) });
            let s = state_with_band();
            draw_command_band(&s, area, &mut buf, &mut 0, &mut CommandBandHits::default());
        }
    }

    #[test]
    fn closed_band_is_a_noop() {
        let s = AppState::default();
        assert!(s.overlays.command_band.is_none());
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        assert!(hits.headers.is_empty());
    }

    /// SQ-0667: the band no longer draws `panel.border` — there is no frame
    /// left for it to style. Falsifies against the pre-amendment band, which
    /// drew a real box in whatever `panel.border` configured (a custom
    /// `style = "double"` used to paint a "╔" at the top-left corner; there
    /// is no corner left to paint).
    #[test]
    fn band_no_longer_draws_a_border() {
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed =
            crate::theme::toml_schema::parse("[panel]\nborder = { style = \"double\" }\n").unwrap();
        let mut s = state_with_band();
        s.colors.theme = crate::theme::resolve::resolve_theme(&scheme, &parsed);
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert_ne!(buf.cell((0, 0)).unwrap().symbol(), "╔", "no border corner is drawn anymore");
    }

    /// Resize mode still needs SOME affordance that the band is the live
    /// target now that there's no border left to accent (SQ-0667) — the
    /// whole fill tints with `panel.border:active` instead.
    #[test]
    fn resize_mode_tints_the_whole_fill_with_no_border_left_to_accent() {
        let s_normal = state_with_band();
        let mut buf_normal = Buffer::empty(BAND);
        draw_command_band(&s_normal, BAND, &mut buf_normal, &mut 0, &mut CommandBandHits::default());

        let mut s_resize = state_with_band();
        s_resize.resize_mode = true;
        s_resize.resize_target = crate::state::ResizeTarget::CommandBand;
        let mut buf_resize = Buffer::empty(BAND);
        draw_command_band(&s_resize, BAND, &mut buf_resize, &mut 0, &mut CommandBandHits::default());

        assert_ne!(
            buf_normal.cell((0, 0)).unwrap().style(),
            buf_resize.cell((0, 0)).unwrap().style(),
            "resize mode visibly tints the band's fill, with no border left to accent instead"
        );
    }

    #[test]
    fn target_height_clamps_and_respects_the_screen() {
        assert_eq!(band_target_height(false, 40, 8), 0);
        assert_eq!(band_target_height(true, 40, 8), 8);
        assert_eq!(band_target_height(true, 40, 2), MIN_BAND_ROWS);
        assert_eq!(band_target_height(true, 40, 99), MAX_BAND_ROWS);
        // A tiny screen wins over the configured height.
        assert_eq!(band_target_height(true, 9, 8), 5);
    }

    #[test]
    fn height_scales_with_the_slide_fraction() {
        assert_eq!(band_height(8, 0.0), 0);
        assert_eq!(band_height(8, 1.0), 8);
        assert_eq!(band_height(8, 0.5), 4);
    }

    #[test]
    fn every_two_object_fallback_verb_declares_a_joining_word() {
        for v in default_verbs().entries {
            if v.max_nouns() == 2 {
                assert!(v.joiner().is_some(), "two-object verb `{}` needs a joiner", v.word);
            }
        }
    }

    // ── The grammar view (SQ-1111) ───────────────────────────────────────────

    fn noun() -> grammar_model::Slot {
        grammar_model::Slot::one(grammar_model::Token::Noun(grammar_model::NounKind::Noun))
    }

    fn word(w: &str) -> grammar_model::Slot {
        grammar_model::Slot::one(grammar_model::Token::Word(w.to_string()))
    }

    fn line(slots: Vec<grammar_model::Slot>) -> grammar_model::SyntaxLine {
        grammar_model::SyntaxLine::new(1, false, slots)
    }

    /// The three shapes the band can compose, read off real syntax lines
    /// instead of declared by hand.
    #[test]
    fn a_syntax_line_narrows_to_the_shape_the_columns_can_build() {
        assert_eq!(VerbLine::from_syntax(&line(vec![])), Some(VerbLine::bare()));
        assert_eq!(VerbLine::from_syntax(&line(vec![noun()])), Some(VerbLine::object()));
        assert_eq!(
            VerbLine::from_syntax(&line(vec![noun(), word("with"), noun()])),
            Some(VerbLine::pair("with"))
        );
        // Two objects with nothing between them is Zork I's `throw noun noun`,
        // and is composable — the second column simply has no word to wear.
        assert_eq!(
            VerbLine::from_syntax(&line(vec![noun(), noun()])),
            Some(VerbLine { nouns: 2, joiners: Vec::new() })
        );
    }

    /// Alternatives at one slot are one slot (`'in' / 'into' / 'inside'`), and
    /// all of them are kept: the first composes, the rest are recognised.
    #[test]
    fn a_slot_s_alternatives_all_reach_the_second_column() {
        let mut slot = word("in");
        slot.alternatives.push(grammar_model::Token::Word("into".into()));
        slot.alternatives.push(grammar_model::Token::Word("inside".into()));
        let l = VerbLine::from_syntax(&line(vec![noun(), slot, noun()])).expect("composable");
        assert_eq!(l.joiners, vec!["in", "into", "inside"]);
        let v = VerbEntry::new("put", vec![l]);
        assert_eq!(v.joiner(), Some("in"));
        assert_eq!(v.joiners(), vec!["in", "into", "inside"]);
    }

    /// A literal word the band has nowhere to put is refused rather than
    /// silently dropped — composing `look lamp` off Zork I's `look at noun`
    /// would offer a command that story really does reject.
    #[test]
    fn a_line_the_band_cannot_type_back_is_refused() {
        assert_eq!(VerbLine::from_syntax(&line(vec![word("at"), noun()])), None);
        assert_eq!(VerbLine::from_syntax(&line(vec![word("for"), noun()])), None);
        assert_eq!(VerbLine::from_syntax(&line(vec![noun(), word("up")])), None);
        assert_eq!(VerbLine::from_syntax(&line(vec![noun(), noun(), noun()])), None);
    }

    /// The alternation `Arity` could not hold: one verb, two shapes, and the
    /// phrase is finished at either.
    #[test]
    fn a_verb_keeps_every_shape_it_accepts() {
        let v = VerbEntry::new("take", vec![VerbLine::object(), VerbLine::pair("from")]);
        assert!(v.accepts(1) && v.accepts(2));
        assert!(!v.accepts(0));
        assert_eq!(v.max_nouns(), 2);
        assert_eq!(v.joiner(), Some("from"));
    }

    /// Every dictionary spelling gets a row, one-letter abbreviations do not,
    /// and the column comes out alphabetical. This is what puts `take` and
    /// `look` in a Zork I column whose verbs are internally named `carry` and
    /// `gaze`.
    #[test]
    fn the_story_s_column_is_every_spelling_alphabetically() {
        let verbs = vec![
            grammar_model::Verb::new(
                255,
                0,
                vec!["carry".into(), "get".into(), "take".into()],
                vec![line(vec![noun()]), line(vec![noun(), word("from"), noun()])],
            ),
            grammar_model::Verb::new(
                254,
                0,
                vec!["gaze".into(), "l".into(), "look".into()],
                vec![line(vec![]), line(vec![word("at"), noun()])],
            ),
        ];
        let entries = verbs_from_grammar(&verbs);
        let words: Vec<&str> = entries.iter().map(|e| e.word.as_str()).collect();
        assert_eq!(words, vec!["carry", "gaze", "get", "look", "take"], "no `l`, alphabetical");
        let take = entries.iter().find(|e| e.word == "take").expect("take");
        assert_eq!(take.max_nouns(), 2, "a synonym carries the verb's own shapes");
        assert_eq!(take.joiner(), Some("from"));
        let look = entries.iter().find(|e| e.word == "look").expect("look");
        assert_eq!(look.max_nouns(), 0, "`look at noun` is not `look noun`");
        assert!(look.accepts(0));
        // …and yet the STORY takes an object here, which is the distinction
        // SQ-1128 turns on: the band cannot compose `look at OBJ`, but the
        // sentence exists, so a quick-row `look` is not the whole of `look`.
        assert!(look.takes_object, "`look at OBJ` is a noun-slot line the raw grammar has");
        let take = entries.iter().find(|e| e.word == "take").expect("take");
        assert!(take.takes_object, "and an ordinary object verb agrees both ways");
    }

    /// A verb with no noun slot ANYWHERE in its raw grammar answers false, which
    /// is what keeps `wait`/`again`/`inventory` off the column (SQ-1128).
    /// Falsifies against reading `takes_object` off `lines` instead of the raw
    /// syntax — which would answer the same here and wrongly for `look` above.
    #[test]
    fn a_genuinely_bare_verb_takes_no_object() {
        let verbs = vec![grammar_model::Verb::new(
            253,
            0,
            vec!["wait".into(), "z".into()],
            vec![line(vec![])],
        )];
        let entries = verbs_from_grammar(&verbs);
        let wait = entries.iter().find(|e| e.word == "wait").expect("wait");
        assert!(!wait.takes_object, "one bare line and nothing else");
        assert_eq!(wait.max_nouns(), 0);
    }

    /// SQ-1126: `#` marks Infocom's test harness and `$` its diagnostics, and
    /// neither is something to offer a player browsing for what to try. A RULE,
    /// on the first character — no vocabulary to maintain and no switch, unlike
    /// the adult list beside it. Falsifies against a `VerbTable` that keeps
    /// everything the grammar named.
    #[test]
    fn sigil_verbs_are_dropped_from_the_column() {
        let entries = ["#record", "$verify", "#random", "take", "dollar", "hash"]
            .iter()
            .map(|w| VerbEntry::new(w, vec![VerbLine::object()]))
            .collect();
        let table = VerbTable::new(entries, VerbSource::Story).without_sigil_verbs();
        let words: Vec<&str> = table.entries.iter().map(|e| e.word.as_str()).collect();
        assert_eq!(words, vec!["take", "dollar", "hash"], "the sigil is a PREFIX, not a substring");
    }

    /// The rule is the first character only, so a verb that merely contains one
    /// is untouched — and a table with no sigil words is unchanged.
    #[test]
    fn a_column_without_sigils_is_left_alone() {
        let before = default_verbs();
        let after = before.clone().without_sigil_verbs();
        assert_eq!(before, after, "the built-in table has nothing to drop");
    }

    /// The fallback labels itself and the story's own column does not — the
    /// `here_is_seen` rule, applied to the VERB column.
    #[test]
    fn only_a_column_that_is_not_the_story_s_wears_a_label() {
        assert_eq!(VerbSource::Story.column_label(), None);
        assert_eq!(VerbSource::Builtin.column_label(), Some("VERB — generic"));
        assert_eq!(VerbSource::Configured.column_label(), Some("VERB — yours"));
    }

    /// SQ-0667, narrowed by SQ-1128: the VERB column excludes a quick word only
    /// when the quick row's one click is the WHOLE of what that word can do.
    /// `wait`, `again` and `inventory` are complete in themselves; the compass
    /// is too. Falsifies against HEAD, where `items(COL_VERB)` is just the raw
    /// verb table.
    #[test]
    fn verb_column_excludes_quick_words_that_cannot_take_an_object() {
        let band = CommandBandState::new(default_verbs(), default_quick());
        let items = band.items(COL_VERB);
        assert!(!items.contains(&"wait".to_string()));
        assert!(!items.contains(&"again".to_string()));
        assert!(!items.contains(&"inventory".to_string()));
        // Direction words compare by the direction they name, not by spelling:
        // the quick row says `n s e w` while the table spells them out, and the
        // compass must still be excluded (it appeared in both places).
        for dir in ["north", "south", "east", "west", "up", "down", "in", "out"] {
            assert!(!items.contains(&dir.to_string()), "`{dir}` is one click away on the quick row");
        }
        assert!(items.contains(&"take".to_string()), "an ordinary verb is unaffected");
    }

    /// The reported defect (SQ-1128): the column jumped from `lock` to `lose`
    /// because `look` sat on the quick row — but the button only ever fires the
    /// bare word, and `look at`/`under`/`behind`/`in` are the column's alone.
    /// The same argument returns `enter`, which the quick row's `in` excluded by
    /// direction equivalence. Falsifies against the pre-SQ-1128 filter.
    #[test]
    fn a_quick_word_that_takes_an_object_stays_in_the_verb_column() {
        let band = CommandBandState::new(default_verbs(), default_quick());
        let items = band.items(COL_VERB);
        assert!(items.contains(&"look".to_string()), "the word the user went looking for: {items:?}");
        assert!(
            items.contains(&"enter".to_string()),
            "`in` is direction-equivalent to `enter`, which takes an object: {items:?}"
        );
    }

    /// Direction equivalence follows the EFFECTIVE quick list too: a custom
    /// quick row without `n` puts `north` back in the VERB column.
    #[test]
    fn a_direction_dropped_from_quick_returns_to_the_verb_column() {
        let band = CommandBandState::new(default_verbs(), vec!["inventory".to_string()]);
        let items = band.items(COL_VERB);
        assert!(items.contains(&"north".to_string()), "`n` no longer in quick -> `north` returns");
        assert!(!items.contains(&"inventory".to_string()), "…and the one word still on it stays out");
    }

    /// The exclusion is config-aware: it follows the EFFECTIVE `quick` list
    /// (the user's custom one when set), not the built-in row. Removing a
    /// word from a custom `quick` puts it back in the VERB column.
    #[test]
    fn verb_column_exclusion_follows_a_custom_quick_list() {
        let mut band = CommandBandState::new(default_verbs(), vec!["wait".to_string()]);
        assert!(!band.items(COL_VERB).contains(&"wait".to_string()), "custom quick excludes `wait`");
        assert!(
            band.items(COL_VERB).contains(&"again".to_string()),
            "…and un-excludes `again`, no longer in quick"
        );

        band.quick.clear();
        assert!(band.items(COL_VERB).contains(&"wait".to_string()), "removed from quick -> back in VERB");
    }

    /// …and a custom quick list cannot hide an object-taking verb either: the
    /// rule is about what the word can DO, not about which list it is on
    /// (SQ-1128). Somebody who puts `take` on their quick row still needs the
    /// column to say `take lantern`.
    #[test]
    fn a_custom_quick_list_still_cannot_hide_an_object_verb() {
        let band = CommandBandState::new(default_verbs(), vec!["take".to_string()]);
        assert!(band.items(COL_VERB).contains(&"take".to_string()), "one click cannot finish `take`");
    }

    // ── SQ-1130: the band's own word equivalence ──────────────────────────────

    /// A direction PARSER's aliases are facts about movement. The band is
    /// asking whether two words are the same word, and there they are not.
    ///
    /// Falsify by pointing `compass_spelling` at
    /// `mapper::direction::parse_direction`: every nautical line below flips.
    #[test]
    fn a_nautical_alias_is_not_another_spelling_of_a_compass_point() {
        use mapper::direction::{parse_direction, Direction as D};
        for (word, sailing) in
            [("bow", D::N), ("fore", D::N), ("aft", D::S), ("stern", D::S), ("port", D::W), ("starboard", D::E)]
        {
            assert_eq!(
                parse_direction(word),
                Some(sailing),
                "the MAPPER still reads `{word}` as a heading — that is its job"
            );
            assert_eq!(
                compass_spelling(word),
                None,
                "`{word}` is a word of its own to the band, not a spelling of {sailing:?}"
            );
        }
        // `enter`/`exit` are verbs that take an object, not spellings of in/out.
        assert_eq!(compass_spelling("enter"), None);
        assert_eq!(compass_spelling("exit"), None);
        // …and the real spellings still pair up, which is the whole point of
        // having a table at all.
        for (short, long) in [
            ("n", "north"),
            ("s", "south"),
            ("e", "east"),
            ("w", "west"),
            ("ne", "northeast"),
            ("nw", "northwest"),
            ("se", "southeast"),
            ("sw", "southwest"),
            ("u", "up"),
            ("d", "down"),
            ("in", "inside"),
            ("out", "outside"),
        ] {
            assert!(compass_spelling(short).is_some(), "`{short}` is a direction");
            assert_eq!(compass_spelling(short), compass_spelling(long), "{short}/{long}");
        }
        assert_eq!(compass_spelling("go north"), compass_spelling("n"), "the `go` prefix survives");
        assert_eq!(compass_spelling("look under"), None, "two words that are not a heading");
    }

    /// SQ-1130's reported symptom: `bow` is a verb in twelve stories of the
    /// corpus and was excluded from the VERB column because the quick row has
    /// `n`. SQ-1128's rule returns it whenever it takes an object; this pins
    /// the case that rule cannot reach — a `bow` with no object at all.
    ///
    /// Falsify by restoring `parse_direction` in `items`: `bow` vanishes.
    #[test]
    fn a_verb_the_mapper_would_call_north_is_not_excluded_by_the_quick_row() {
        let mut verbs = default_verbs();
        verbs.entries.push(VerbEntry::new("bow", vec![VerbLine::bare()]));
        verbs.entries.push(VerbEntry::new("port", vec![VerbLine::bare()]));
        let band = CommandBandState::new(verbs, default_quick());
        let items = band.items(COL_VERB);
        assert!(items.contains(&"bow".to_string()), "`bow` is not `n`: {items:?}");
        assert!(items.contains(&"port".to_string()), "`port` is not `w`: {items:?}");
        // The compass is still excluded — this must not become "no exclusion".
        for dir in ["north", "south", "east", "west", "up", "down", "in", "out"] {
            assert!(!items.contains(&dir.to_string()), "`{dir}` is still one click away");
        }
    }

    /// The same fact reaches the ROSE: a quick row holding `port` used to fill
    /// the W slot, drawn `W`, firing `port`. It is an ordinary word now.
    #[test]
    fn a_nautical_alias_flows_as_a_word_instead_of_filling_a_rose_slot() {
        let quick: Vec<String> =
            ["port", "bow", "north", "e"].iter().map(|s| s.to_string()).collect();
        let (rose, _cluster, words) = split_quick_rose(&quick);
        assert_eq!(rose[ROSE_LABELS.iter().position(|l| *l == "W").unwrap()], None, "`port` is not W");
        assert_eq!(rose[ROSE_LABELS.iter().position(|l| *l == "N").unwrap()], Some(2), "`north` is N");
        assert_eq!(rose[ROSE_LABELS.iter().position(|l| *l == "E").unwrap()], Some(3), "`e` is E");
        assert_eq!(words, vec![0, 1], "`port` and `bow` flow as words");
    }

    // ── SQ-0677: stacked quick block, dividers, current-column hint ────────────

    /// The word flow now stacks UNDER the rose (SQ-0677) rather than beside
    /// it: every word row's y-offset is at or past `ROSE_ROWS`, and the block
    /// is "as narrow as the widest word row" — its width is the max of the
    /// rose's own width and the widest packed word row, not the two added
    /// together the way a side-by-side layout would need. Falsifies against
    /// reverting to the SQ-0675 side-by-side layout, where the words shared
    /// the rose's rows and the block was `rose + gap + words` wide.
    #[test]
    fn quick_words_stack_under_the_rose_not_beside_it() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        assert!(layout.has_rose);
        assert!(layout.has_cluster, "up/down/in/out are portal words — the cluster shows");
        assert_eq!(
            layout.words_y, ROSE_ROWS,
            "the word flow starts right under the 3-row rose+cluster"
        );
        assert!(!layout.word_rows.is_empty(), "the default quick list has non-compass, non-portal words");

        let widest_row: u16 = layout
            .word_rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|&(qi, x_off)| x_off + quick[qi].chars().count() as u16)
                    .max()
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0);
        // The rose and the glyph cluster sit side by side (SQ-1218): the
        // block's top width is rose + gap + cluster, not either alone.
        let top_w = ROSE_WIDTH + CLUSTER_ROSE_GAP + CLUSTER_WIDTH;
        assert!(widest_row <= top_w.max(widest_row), "sanity");
        assert_eq!(
            layout.width,
            BLOCK_MARGIN + top_w.max(widest_row) + BLOCK_MARGIN,
            "block width is margins plus max(rose+gap+cluster width, widest word row) — never everything added"
        );
    }

    /// The stacked block's total height is the rose's rows plus however many
    /// word rows the flow needs — "as many rows as needed" (SQ-0677), not
    /// capped at a fixed constant the way the pre-amendment side-by-side
    /// block was (always exactly 3, regardless of the word list).
    #[test]
    fn block_height_is_rose_rows_plus_word_rows() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        assert_eq!(layout.height, ROSE_ROWS + layout.word_rows.len() as u16);
        assert!(layout.word_rows.len() > 1, "the default quick list needs more than one word row");
    }

    /// SQ-1218: with up/down/in/out moved to the glyph cluster, the default
    /// quick list's word flow is down to 4 words, which `WORD_ROW_BUDGET = 2`
    /// packs into exactly `look inventory` / `wait again` — TWO rows, not
    /// three — so the shipped default band (5 rows: 3-row rose + 2 word rows,
    /// `DEFAULT_BAND_ROWS`) shows the whole list with nothing clipped.
    /// Falsifies against a `WORD_ROW_BUDGET` that still packs `look` and
    /// `inventory` onto separate rows (the pre-amendment budget of 3, which
    /// finds the narrowest width satisfying 3 rows rather than 2 and lands on
    /// a width too narrow to hold `look inventory` together).
    #[test]
    fn the_default_word_flow_packs_into_exactly_two_rows() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        assert_eq!(layout.word_rows.len(), 2, "look/inventory/wait/again pack into exactly 2 rows");

        let row_words = |row: &[(usize, u16)]| -> Vec<&str> {
            row.iter().map(|&(i, _)| quick[i].as_str()).collect()
        };
        assert_eq!(row_words(&layout.word_rows[0]), vec!["look", "inventory"]);
        assert_eq!(row_words(&layout.word_rows[1]), vec!["wait", "again"]);

        // The shipped default band height fits rose + both word rows with
        // nothing clipped.
        assert_eq!(
            layout.height, DEFAULT_BAND_ROWS,
            "rose (3) + 2 word rows == the default band height"
        );

        let s = state_with_band();
        let area = Rect { x: 0, y: 0, width: 120, height: DEFAULT_BAND_ROWS };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);
        assert!(out.contains("look") && out.contains("inventory"), "row 0: {out}");
        assert!(out.contains("wait") && out.contains("again"), "row 1: {out}");

        // Every word hit lands on one of the two word rows, none past them —
        // no third row exists to clip against, at any band height.
        let word_idxs: std::collections::HashSet<usize> =
            layout.word_rows.iter().flatten().map(|&(i, _)| i).collect();
        let word_rows_y: std::collections::HashSet<u16> = hits
            .quick
            .iter()
            .filter(|(i, _)| word_idxs.contains(i))
            .map(|(_, r)| r.y)
            .collect();
        assert_eq!(word_rows_y.len(), 2, "word hits land on exactly two rows: {word_rows_y:?}");
    }

    /// The height interplay (SQ-0677, documented above `WORD_ROW_BUDGET`):
    /// a band shorter than the block's natural height still draws the rose
    /// in full and clips whatever word rows don't fit, rather than shrinking
    /// the rose or refusing to show the block at all. Falsifies against a
    /// block that either panics or silently drops the rose when the band is
    /// short.
    #[test]
    fn a_short_band_clips_word_rows_but_keeps_the_rose() {
        let s = state_with_band();
        // Tall enough to fit the rose+columns, short of the full word flow.
        let area = Rect { x: 0, y: 0, width: 120, height: 4 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);
        assert!(out.contains('N') && out.contains('S'), "the rose still draws in full: {out}");
        // Every registered quick hit stays within the drawn area — no hits
        // for clipped rows.
        assert!(hits.quick.iter().all(|(_, r)| r.y < area.bottom()), "no hits below the band: {:?}", hits.quick);
    }

    /// Single-cell `│` dividers separate the quick block from VERB and every
    /// column from its neighbour (SQ-0677), full band height. Falsifies
    /// against removing the `draw_divider` calls in `draw_command_band`.
    #[test]
    fn dividers_separate_the_block_and_every_column() {
        let s = state_with_band();
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);

        // The block-to-VERB divider sits right after the block's own width.
        let layout = quick_block_layout(&s.overlays.command_band.as_ref().unwrap().quick);
        let block_divider_x = BAND.x + layout.width;
        assert_eq!(
            buf.cell((block_divider_x, BAND.y)).unwrap().symbol(),
            "\u{2502}",
            "a divider separates the quick block from VERB"
        );

        // One divider between every pair of adjacent columns.
        assert_eq!(hits.columns.len(), BAND_COLS, "sanity: four columns drew");
        for pair in hits.columns.windows(2) {
            let dx = pair[0].1.right();
            assert_eq!(
                buf.cell((dx, BAND.y)).unwrap().symbol(),
                "\u{2502}",
                "a divider separates column {} from column {}",
                pair[0].0,
                pair[1].0
            );
        }

        // Dividers span the full band height, not just the columns' own rows.
        assert_eq!(
            buf.cell((block_divider_x, BAND.bottom() - 1)).unwrap().symbol(),
            "\u{2502}",
            "the divider reaches the band's bottom row"
        );
    }

    /// A divider cell belongs to neither of its neighbouring columns' hit
    /// rects — clicking exactly on the line between two columns must not be
    /// silently attributed to either one.
    #[test]
    fn divider_cells_are_excluded_from_both_neighbouring_column_rects() {
        let s = state_with_band();
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        for pair in hits.columns.windows(2) {
            let dx = pair[0].1.right();
            assert!(dx < pair[1].1.x, "a gap (the divider) separates {:?} and {:?}", pair[0], pair[1]);
        }
    }

    /// The current column (`band.focus`, Tab/Shift-Tab-driven — SQ-0677)
    /// gets a visible hint: `band.column_header:active` on its header row for
    /// the object/prep columns, whose header moves when Tab moves focus.
    #[test]
    fn the_current_column_header_follows_focus() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("take"); // opens HERE (focus lands there) and CARRIED
        }
        assert_eq!(s.overlays.command_band.as_ref().unwrap().focus, COL_HERE);
        let mut buf_here = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf_here, &mut 0, &mut CommandBandHits::default());

        s.overlays.command_band.as_mut().unwrap().step_column(1); // -> CARRIED
        assert_eq!(s.overlays.command_band.as_ref().unwrap().focus, COL_CARRIED);
        let mut buf_carried = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf_carried, &mut 0, &mut CommandBandHits::default());

        assert_ne!(
            buf_here.content(),
            buf_carried.content(),
            "moving the current column with Tab visibly moves the header hint"
        );
    }

    /// The current-column hint decorates chrome, not list rows (user
    /// feedback 2026-08-05, retiring VERB's top-row underline which read as
    /// a mysteriously decorated entry): the dividers flanking the current
    /// column take the `panel.border:active` accent, uniformly for every
    /// column, and NO list row anywhere gains a text decoration from column
    /// focus. Falsifies against the underline scheme and against unstyled
    /// dividers alike.
    #[test]
    fn the_current_columns_flanking_dividers_carry_the_accent_not_any_row() {
        use ratatui::style::Modifier;

        let style_at = |buf: &Buffer, x: u16, y: u16| buf.cell((x, y)).unwrap().style();

        // VERB current (the band's default focus): the divider on VERB's
        // right flank is styled DIFFERENTLY from the far divider between
        // CARRIED and WITH… (which flanks neither side of the current
        // column) — the accent is a visible difference, whatever the theme
        // resolves it to.
        let s = state_with_band();
        assert_eq!(s.overlays.command_band.as_ref().unwrap().focus, COL_VERB);
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        let rect_of = |hits: &CommandBandHits, col: usize| {
            hits.columns.iter().find(|(c, _)| *c == col).expect("column drew").1
        };
        let verb = rect_of(&hits, COL_VERB);
        let carried = rect_of(&hits, COL_CARRIED);
        assert_ne!(
            style_at(&buf, verb.right(), verb.y),
            style_at(&buf, carried.right(), carried.y),
            "the divider flanking the current column differs from a far divider"
        );
        // And no list row anywhere is underlined by column focus.
        for x in verb.x..carried.right() {
            for y in BAND.y..BAND.bottom() {
                assert!(
                    !buf.cell((x, y)).unwrap().style().add_modifier.contains(Modifier::UNDERLINED),
                    "no row wears an underline from column focus (cell {x},{y})"
                );
            }
        }

        // Move focus: the accent follows — the VERB-flank divider goes plain
        // and the divider beside the new current column takes the accent.
        let mut s2 = state_with_band();
        s2.overlays.command_band.as_mut().unwrap().pick_word("take");
        let focus = s2.overlays.command_band.as_ref().unwrap().focus;
        assert_ne!(focus, COL_VERB);
        let mut buf2 = Buffer::empty(BAND);
        let mut hits2 = CommandBandHits::default();
        draw_command_band(&s2, BAND, &mut buf2, &mut 0, &mut hits2);
        let cur = rect_of(&hits2, focus);
        assert_ne!(
            style_at(&buf2, cur.x.saturating_sub(1), cur.y),
            style_at(&buf2, carried.right(), cur.y),
            "the accent moved with the current column"
        );
    }
}
