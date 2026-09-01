//! The momentary reveal: light the nouns and named things on screen the story
//! really knows (SQ-1107, SQ-1207).
//!
//! ```text
//! You are in a dimly lit room. Cobwebs hang from the beams, and a
//! ─────────                                       ────────
//! rusted iron lantern sits on the sill beside a faded portrait.
//! ────── ──── ───────           ────
//! ```
//!
//! The oldest frustration in the genre: a room description names a dozen nouns
//! and two of them are implemented. Players learn to probe blindly, and the ones
//! without nostalgia for that simply stop. This is the inverse of
//! [`crate::vocab`]'s offer, which can only help AFTER the parser has rejected a
//! word — it says which of the words already on screen would be accepted in the
//! first place.
//!
//! # It asks the story, twice, and never guesses
//!
//! **Where a word ends is the story's answer.** [`Engine::split_like_parser`]
//! (SQ-1116) is the code path `read` itself calls, so the dictionary's declared
//! separators, the Z-encoding and the §13.3 six / §13.4 nine Z-character
//! truncation all apply exactly as the game applies them. There is no word
//! splitter in this file, and the last one in the codebase was deleted for cause.
//!
//! **Which words light is the story's OBJECTS' answer**, and the question put
//! to them is *"is this one of your parse names?"* — nouns and adjectives, with
//! no scope walk anywhere in it (SQ-1135) and no verb, article or preposition
//! anywhere in the answer (SQ-1207). It used to ask the object tree what was
//! actually HERE wherever an engine had one, and fall back to the dictionary
//! where it did not — that inverted the point, because the engine that could say
//! the most lit the least, and a description naming a sword in the next room lit
//! nothing at all (SQ-1135). It then asked every object GLOBALLY (no scope walk,
//! same as today) but still measured "do you know this word?" by the
//! dictionary's own flag byte, and that is not the same question: on an
//! Inform-family story the "noun" bit means "usable in noun position", not
//! "names a thing", so it lit `a`, `an` and `the` right alongside a real noun
//! (SQ-1207). `ObjectWords::refers_to` / `grammar_model::ObjectWordSet` is the
//! fix — built from every object's own parse names, nouns and adjectives folded
//! together because the parser does not distinguish them either — and it is what
//! every engine with a readable object table answers with today: every retail
//! Z-machine title measured so far, Infocom and Inform-on-Z-code alike (see
//! `zvm::objects::ParseNames::detect`), `word_reveal.rs`'s Zork I specimen
//! among them — and, since SQ-1210, every Inform-compiled Glulx image too
//! (`gvm::objects::ParseNames`, reached through [`Engine::object_word_set`]),
//! which is what put out the `the`/`an` lights this module's fallback lit on
//! `Dr Ludwig and the Devil`.
//!
//! **The dictionary is still asked, but only where the objects cannot be** —
//! Scott today, which has no object table at all, and any Glulx image whose
//! object list fails `gvm`'s validation and honestly answers `None`. Falling
//! back to the dictionary's flag byte there is a decision and not an
//! oversight: an imperfect reveal beats a dark one, and [`arm`]'s `None` arm
//! says why in code. Every word here, either tier, is a word the story has
//! ALREADY PRINTED on the player's own screen, so lighting it reveals nothing
//! that has not been told, and there is no spoiler for a narrower test to
//! defend against. [`CAVEAT`] states the claim rather than leaving the player to
//! infer a stronger one.
//!
//! # Nouns, not verbs — and not articles either
//!
//! A verb never lights, on any engine: the verb panel already answers "what can
//! I do"; this answers "what does this game know about", and they are different
//! questions that would blur into a ransom note if merged. Where the objects can
//! answer, an article or preposition never lights either, because it is never
//! one of their parse names — "You are in an open field west of a white house"
//! lights `white` (an adjective of Zork I's house) and never `open` or `west`.
//! That guarantee only weakens on the dictionary fallback below, and only on an
//! Inform-family title: its dictionary marks a word "usable in noun position",
//! not "is a noun", so `a`, `an` and `the` can still slip through there.
//!
//! # The viewport
//!
//! Exactly what is on screen lights. Scroll and press again to light a different
//! screenful, which answers "how far back do we go?" with the scroll position
//! instead of a constant somebody has to defend.
//!
//! # Momentary, because a terminal cannot do hold-to-reveal
//!
//! Terminals do not report key RELEASE without the kitty keyboard protocol, and
//! lanthorn never pushes keyboard enhancement flags — `input.rs` sees
//! `KeyEventKind::Press` only. Enabling them would work on Ghostty and not on
//! Windows Terminal, and a feature that silently does nothing on one platform is
//! worse than one that behaves the same everywhere. So: one press lights the
//! viewport, and it goes out on the next keystroke, the next turn, or
//! [`REVEAL_HOLD`], whichever comes first. Same feel, no protocol dependency.
//!
//! # Two known false positives, both the parser's own
//!
//! **Truncation.** In a Version 3 game `candle` and `candlesticks` are the same
//! six Z-characters, so a room holding a candle lights the word `candlesticks`
//! wherever it appears. That is not a defect here: `take candlesticks` really
//! does take the candle, because the parser truncates the player's word exactly
//! as the dictionary truncated its own. It is the game's behaviour, shown.
//!
//! **A word inside a word the story would have kept whole.** The words come from
//! the story's tokeniser but they are LOCATED in a drawn row by
//! [`lit_spans`], which accepts a match whose neighbours are not alphanumeric. A
//! story that does not declare `'` a separator holds `bird's` as one word; if
//! `bird` is separately in scope it will light inside `bird's` too. One column of
//! over-lighting, and the alternative is a second word splitter disagreeing with
//! the first.
//!
//! # Two surfaces, one shape (SQ-1138)
//!
//! Version 6 raster draws its text as bitmap glyphs on a canvas rather than as
//! terminal cells, so there is nothing to re-style after the fact — the light has
//! to be applied at the moment each glyph is blitted. It was dark there until
//! SQ-1138, which is a real gap and not a compromise: raster is a destination.
//!
//! What makes one reveal serve both is that neither surface needs a record of
//! which glyphs belonged to which word. **Both draw a row of text from a wrap
//! cache, and [`lit_spans`] locates the words in that row's own string** — the
//! cell path walks it by display column, the raster path by the pen. So the two
//! differ only in what "apply the light" means:
//!
//! | | reads | lights by |
//! |---|---|---|
//! | cell / hybrid | [`crate::render::wrap_cache::CellWrapCache`] | [`paint_row`], re-styling drawn cells |
//! | raster / extended | [`crate::render::wrap_cache::RasterWrapCache`] | [`RasterReveal`], at blit time in `draw_story_text` |
//!
//! [`visible_text`] asks whichever one drew the last frame, and both answers are
//! the same string, so [`arm`] is unchanged by any of it.
//!
//! **And the underline survives the crossing.** `transcript_reveal` is parented on
//! `accent` and UNDERLINED, and the rule is not decoration: this is ink laid over
//! the story's own prose, and a foreground alone cannot promise legibility over a
//! ground the game chose. On the canvas that rule is drawn in the same geometry
//! SQ-1028 gives an emphasised run — the bottom of the TEXT cell, one master row
//! thick, spanning each lit glyph's whole advance so the letters join into one
//! unbroken line under the word.
//!
//! [`Engine::split_like_parser`]: crate::engine::Engine::split_like_parser

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::engine::Engine;
use crate::state::AppState;

/// How long a reveal holds before it goes out on its own.
///
/// Long enough to read a screenful, short enough that it never feels like a mode
/// the player is stuck in. The other two exits — the next keystroke and the next
/// turn — are what actually ends most reveals; this is the one for a player who
/// pressed it and then did nothing.
pub const REVEAL_HOLD: Duration = Duration::from_millis(4_000);

/// The legend for a reveal, said out loud every time one lights (SQ-1135).
///
/// It is the whole claim, stated: the words are the ones the STORY KNOWS, and
/// knowing a word is not a promise that the thing is within reach. There used to
/// be a second, stronger tier that walked the object tree and lit only what was
/// in scope, with this line reserved for the engines that had no tree to walk —
/// and the tier that could say more was the one that offered less, because a
/// description naming a sword in the next room lit nothing at all. Lighting a
/// word the story has ALREADY PRINTED reveals nothing that has not been told, so
/// there was never a spoiler for the stronger test to defend against.
/// Stated in the control's description rather than on every press since
/// SQ-1214 — a lit reveal itself says nothing, so this is the one wording of
/// the claim, kept here so the description and the docs cannot drift apart.
pub const CAVEAT: &str = "words this story knows — not necessarily things that are here";

/// A reveal that is currently lit.
#[derive(Debug, Clone)]
pub struct Reveal {
    /// The spellings that light, exactly as the story's own tokeniser cut them
    /// out of the prose on screen — so `lantern` and not the `lanter` a Version 3
    /// dictionary stores, because it is the printed spelling the player is
    /// looking at.
    pub words: BTreeSet<String>,
    /// When it goes out on its own.
    pub until: Instant,
}

impl Reveal {
    /// Is this reveal still lit?
    pub fn is_lit(&self) -> bool {
        Instant::now() < self.until
    }
}

// ── Arming ──────────────────────────────────────────────────────────────────

/// What [`arm`] did, so the caller can say it out loud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Armed {
    /// `n` words lit. [`CAVEAT`] is the caller's to relay.
    Lit { words: usize },
    /// Nothing on screen is a word this story would accept. A real answer — a
    /// room of pure scenery gives it — and said plainly rather than silently.
    ///
    /// It is a claim about the ROOM, so only the tier that actually asked the
    /// story may make it. When neither tier can be asked at all the answer is
    /// [`NoVocabulary`](Self::NoVocabulary), not this (SQ-1150).
    Nothing,
    /// The story's words could not be read AT ALL — its object tree will not
    /// answer and it has no dictionary snapshot to fall back on, so there was
    /// nothing to test the prose against (SQ-1150).
    ///
    /// Deliberately not [`Nothing`](Self::Nothing), which this used to be: a
    /// Dialog story (`stories/ImpossibleStairs.z8`,
    /// `stories/frankenfingers_260330.z5`) and `stories/advent.z8`, whose
    /// dictionary declares zero entries, take plenty of the words on screen —
    /// we simply cannot say which, and "nothing on screen is a word this story
    /// takes" is a claim about the STORY that the app is in no position to make.
    NoVocabulary,
    /// There is no drawn text to read — no frame has been rendered yet, or the
    /// one that was carries no prose. Not a statement about the SURFACE: both the
    /// cell path and the v6 raster path answer [`visible_text`] (SQ-1138).
    NoText,
    /// The Guiding Light is out, and this is one of its lamps.
    GuidanceOff,
}

/// Light the words on screen that this story would accept, right now.
///
/// The order is the whole design: read what is DRAWN, cut it into words with the
/// STORY's tokeniser, and keep the ones the story's own world model answers to.
/// Nothing here consults English.
pub fn arm(state: &mut AppState, engine: &dyn Engine) -> Armed {
    // Under the Guiding Light's switch, like every other assist: a player who has
    // put the light out has said they do not want this kind of help (SQ-1045).
    // The control stays on the border beside the lamp, so the answer to "why did
    // nothing happen" is one click away from the question.
    if !state.config.guidance {
        state.reveal = None;
        return Armed::GuidanceOff;
    }

    let visible = visible_text(state);
    if visible.trim().is_empty() {
        state.reveal = None;
        return Armed::NoText;
    }

    // The story's own tokeniser where it lends one; `split_prose` is the
    // documented last resort for an engine that does not, and costs only an
    // unusual separator set — whatever comes out is still filtered below by the
    // story's own world model or its own dictionary.
    let tokens = engine
        .split_like_parser(&visible)
        .unwrap_or_else(|| crate::complete::split_prose(&visible));

    // ASK THE OBJECTS FIRST, and fall back to the flag byte only where they
    // cannot answer (SQ-1153).
    //
    // The role filter below is a proxy: a dictionary bit that USUALLY means "this
    // word names a thing". On Infocom's V6 titles it does not — `decode_roles`
    // reads that flag byte with Inform's layout, and Infocom V6 keeps the bits
    // somewhere else. Measured: the noun bit selects `a all and of the then` on
    // Zork Zero and `are is was were will` on Arthur, while missing `crystal`,
    // `torque` and `sword`. So the reveal lit punctuation and articles on exactly
    // the three titles the graphical work showcases.
    //
    // An object's parse names are not a proxy at all — they are the words the
    // parser files that thing under, which is the claim the reveal is making.
    // `Introspect::all_object_words` exists for this reason and its own doc says
    // so ("need no flag layout"); SQ-1135 added it for the command band and left
    // this second consumer on the flag byte. `refers_to` truncates both sides, so
    // a printed `lantern` matches Zork I's stored `lanter`, and it answers over
    // adjectives too — `dirigible` names Zork Zero's hangar exactly as `hangar`
    // does, and the parser does not distinguish them.
    //
    // `None` means the question could not be ASKED — Glulx and Scott today — and
    // is NOT the same as a story with no parse names. Only the first falls back.
    //
    // The dictionary is fetched INSIDE that fallback and not above the match
    // (SQ-1150): it is the second tier's input, so a story whose objects answer
    // must not be turned away for want of one, and the `NoVocabulary` below then
    // means exactly what it says — neither tier could be asked.
    // `object_word_set`, not `all_object_words` + a `refers_to` walk: this asks
    // only "does ANY object answer", for every token on screen, and the walk
    // re-truncated the story's whole vocabulary per token (SQ-1176). Same
    // answers — the set is `any(refers_to)` by construction, minus the articles
    // Inform 7 folds into multi-word names, which the story's own parser never
    // lets stand as a typed word (`grammar_model::ARTICLES`, SQ-1210) — and
    // `None` still means the question could not be asked, never a story with no
    // names.
    let words = match engine.object_word_set() {
        Some(set) => tokens
            .iter()
            .filter(|t| set.contains(t))
            .cloned()
            .collect::<BTreeSet<String>>(),
        None => {
            // Reached only by an engine this crate cannot ask about its own
            // objects at all — Scott today, plus any Glulx image whose object
            // list fails `gvm::objects::ParseNames`' validation (`GameSession`
            // answers `Some` for every Z-machine title with a readable object
            // table, and the Glulx adapter for every Inform-compiled image,
            // which between them is essentially the whole corpus — SQ-1210).
            // Going dark here — lighting nothing, ever, on the remainder — was
            // the other option, and was rejected: an imperfect reveal a Scott
            // player can still lean on beats a silently absent one (SQ-1207
            // decision, stated here since there is nowhere else for a reader of
            // THIS branch to find it).
            //
            // The dictionary, filtered to the words that NAME things — nouns and
            // adjectives, minus the buzzword bit ($04), which is `the`, `a`,
            // `please` and their kin ON AN INFOCOM TITLE.
            //
            // A word carrying both the noun and the VERB bit — `light` in most of
            // Infocom's catalogue — does light, because the claim being made
            // about it here is the noun one.
            //
            // **And it inherits whatever the dictionary thinks a word is, which
            // is a WEAKER claim on an Inform title than an Infocom one.** Neither
            // Inform back-end has a buzzword bit or a distinct adjective bit at
            // all (see `WordRoles`), and Inform's "noun" bit really means "usable
            // in noun position" rather than "names a thing" — measured on a real
            // Glulx game (`Dr Ludwig and the Devil.gblorb`) back when this arm
            // was Glulx's only tier, `a`, `an` and `the` all decoded with the
            // noun bit set and lit right alongside a real noun. That was the
            // whole of SQ-1210, and the fix was the one this comment used to
            // call for: `Engine::object_word_set` now answers on Glulx from the
            // story's own objects, so an Inform-family image lands here only
            // when its object list fails validation. There is still no rescuing
            // the flag bits from inside this arm without consulting English,
            // which this file does not do (see the module doc) — so [`CAVEAT`]
            // says what the reveal is rather than pretending otherwise.
            let Some(v) = state.vocab.get(engine) else {
                state.reveal = None;
                return Armed::NoVocabulary;
            };
            tokens
                .iter()
                .filter(|t| v.roles(t).is_some_and(|r| (r.noun || r.adjective) && !r.special))
                .cloned()
                .collect::<BTreeSet<String>>()
        }
    };

    if words.is_empty() {
        state.reveal = None;
        return Armed::Nothing;
    }
    let n = words.len();
    state.reveal = Some(Reveal { words, until: Instant::now() + REVEAL_HOLD });
    Armed::Lit { words: n }
}

/// Put out whatever is lit. `true` when something actually went out (→ repaint).
pub fn clear(state: &mut AppState) -> bool {
    state.reveal.take().is_some()
}

/// Drop a reveal whose time is up. `true` when one did (→ repaint).
///
/// Called from the loop's expiry tick beside the sound pulse and the toasts,
/// which is what makes the hold a wall-clock hold rather than "until the next
/// event happens to arrive".
pub fn expire(state: &mut AppState) -> bool {
    if state.reveal.as_ref().is_some_and(|r| !r.is_lit()) {
        state.reveal = None;
        return true;
    }
    false
}

/// The text that is actually drawn in the story pane this frame, one string per
/// visible row.
///
/// Read from the wrap cache of whichever path drew, windowed by the geometry that
/// frame recorded — not from `AppState::transcript`, which is the whole scrollback
/// and would light words the player cannot see, and not from a re-wrap, which
/// would have to guess at a width the renderer already knows.
///
/// The two caches are twins ([`crate::render::wrap_cache`]) and the slice is
/// windowed the same way out of both, so the string is the same shape whichever
/// answered and everything downstream is surface-blind.
///
/// Empty before the first frame.
fn visible_text(state: &AppState) -> String {
    raster_visible_text(state).unwrap_or_else(|| cell_visible_text(state))
}

/// The drawn rows of the v6 RASTER composite, or `None` when that is not the
/// surface the player is looking at (SQ-1138).
///
/// Asked FIRST, and gated on two facts rather than one. `v6_render` is the mode
/// the player chose, but it is set on a config that a v3 game or a text-only
/// terminal never reaches the raster arm under; `v6_raster_metrics` is set only by
/// that arm, and only when it found a story window to measure. Neither alone is
/// the question — a hybrid session leaves the mode saying "hybrid" while a
/// raster-configured session on a terminal with no image protocol leaves the
/// metrics unset forever — so both must agree before the raster cache is believed
/// over the cell one.
fn raster_visible_text(state: &AppState) -> Option<String> {
    if state.config.v6_render == crate::config::V6RenderMode::Hybrid {
        return None;
    }
    let metrics = state.v6_raster_metrics.get()?;
    let cache = state.raster_wrap.borrow();
    let cache = cache.as_ref()?;
    Some(
        cache
            .rows
            .iter()
            .skip(metrics.first_visible_row as usize)
            .take(metrics.viewport_rows as usize)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The drawn rows of the cell/hybrid transcript — the original path, unchanged.
fn cell_visible_text(state: &AppState) -> String {
    let Some(geom) = state.transcript_geom.get() else {
        return String::new();
    };
    let cache = state.transcript_wrap.borrow();
    let Some(entry) = cache.as_ref() else {
        return String::new();
    };
    entry
        .rows
        .iter()
        .skip(geom.first_abs_row)
        .take(geom.area.height as usize)
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Painting ────────────────────────────────────────────────────────────────

/// Where in `text` each lit word was printed, as CHAR ranges, in order.
///
/// **This locates; it does not split.** The words were cut out of the prose by
/// the story's own tokeniser before they ever reached here, so the only question
/// left is where on this row the story printed them. A match counts when it is
/// bounded by non-alphanumeric characters on both sides — which is what stops
/// `rug` lighting inside `shrug`, the same anchoring
/// [`crate::vocab`]'s absent-noun check already uses.
///
/// Case-insensitive, per character rather than by lowercasing the whole row,
/// because a lowercase mapping may change a string's length and every offset
/// here has to stay an index into the ORIGINAL text.
pub fn lit_spans(text: &str, words: &BTreeSet<String>) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let mut out: Vec<(usize, usize)> = Vec::new();
    for w in words {
        let pat: Vec<char> = w.chars().map(fold).collect();
        if pat.is_empty() || pat.len() > chars.len() {
            continue;
        }
        for start in 0..=(chars.len() - pat.len()) {
            let end = start + pat.len();
            if !chars[start..end].iter().map(|&c| fold(c)).eq(pat.iter().copied()) {
                continue;
            }
            let left_clear = start == 0 || !chars[start - 1].is_alphanumeric();
            let right_clear = end == chars.len() || !chars[end].is_alphanumeric();
            if left_clear && right_clear {
                out.push((start, end));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Re-style the lit words of one already-drawn row.
///
/// A pass OVER the drawn cells rather than a change to how they are drawn: the
/// reveal is a property of the moment, not of the text, and the transcript's
/// style runs are the game's own output (persisted in the archive, restored with
/// it). Folding a momentary highlight into them would mean writing a decoration
/// into a save file and then having to take it out again.
///
/// `x` is the row's first text column and `text` the string that was drawn
/// there; columns advance by each glyph's DISPLAY width, the same walk
/// `draw_str_runs` made, so a CJK glyph's two cells both light.
pub(crate) fn paint_row(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    area: ratatui::layout::Rect,
    state: &AppState,
) {
    let Some(reveal) = state.reveal.as_ref().filter(|r| r.is_lit()) else {
        return;
    };
    if y < area.y || y >= area.bottom() {
        return;
    }
    let spans = lit_spans(text, &reveal.words);
    if spans.is_empty() {
        return;
    }
    let style = state.colors.theme.get("transcript_reveal").style;
    let mut col = x;
    for (i, ch) in text.chars().enumerate() {
        if col >= area.right() {
            break;
        }
        let w = crate::textwidth::char_cells(ch) as u16;
        if spans.iter().any(|&(s, e)| i >= s && i < e) {
            for c in col..(col + w.max(1)).min(area.right()) {
                if let Some(cell) = buf.cell_mut((c, y)) {
                    let patched = cell.style().patch(style);
                    cell.set_style(patched);
                }
            }
        }
        col += w;
    }
}

// ── Painting, on the pixel canvas ────────────────────────────────────────────

/// A lit reveal, resolved for the RASTER canvas (SQ-1138).
///
/// The two facts travel together because they are only ever used together and
/// only ever come from the same place — the lit words and the ink they light in,
/// resolved once per frame by [`raster_reveal`] and carried into
/// [`crate::render::v6_layout::draw_story_text`] as ONE value. Splitting them
/// across two parameters is how a caller ends up supplying a word set at the
/// story's own ink and drawing a reveal nobody can see.
///
/// There is no `rule` field: the underline's geometry is the TEXT CELL's, which
/// the draw path already holds and this value must never second-guess. A rule
/// sized from anywhere else is the density trap — on a Macintosh colour press one
/// art pixel is two native pixels while one text pixel is one, so a number
/// resolved in the wrong space is half-size on exactly one machine and correct on
/// every other.
pub struct RasterReveal<'a> {
    /// The spellings that light, borrowed from the live [`Reveal`].
    pub words: &'a BTreeSet<String>,
    /// The ink they light in — `transcript_reveal`'s foreground, which is
    /// `accent` unless the player's `style.toml` says otherwise.
    pub ink: image::Rgba<u8>,
    /// Whether to rule under them, from the SAME `transcript_reveal` modifier the
    /// cell path's [`paint_row`] patches onto its cells.
    ///
    /// Read rather than assumed, so a player who restyles the selector gets the
    /// same reveal on both surfaces. It ships true, and the default is the part
    /// that matters: a foreground alone cannot promise legibility over a ground
    /// the game chose, which is why the registry sets it and not why a theme may
    /// not.
    pub rule: bool,
}

/// Resolve the lit reveal for the pixel canvas, or `None` when nothing is lit.
///
/// `fallback` is what the ink falls back to when the theme names a colour the
/// canvas cannot resolve to concrete bytes (`Reset`, an `Indexed`) — the same
/// fallback every other themed colour on this path takes. The raster caller passes
/// the STORY's own ink, so a theme that cannot resolve draws the prose exactly as
/// it already was rather than in some colour nobody chose.
pub fn raster_reveal(state: &AppState, fallback: image::Rgba<u8>) -> Option<RasterReveal<'_>> {
    let reveal = state.reveal.as_ref().filter(|r| r.is_lit())?;
    let style = state.colors.theme.get("transcript_reveal").style;
    let ink = style
        .fg
        .map_or(fallback, |c| crate::render::v6_layout::color_to_rgba(c, fallback));
    let rule = style.add_modifier.contains(ratatui::style::Modifier::UNDERLINED);
    Some(RasterReveal { words: &reveal.words, ink, rule })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set<const N: usize>(words: [&str; N]) -> BTreeSet<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn a_lit_word_is_found_where_the_story_printed_it() {
        let spans = lit_spans("A brass lantern sits here.", &set(["lantern"]));
        assert_eq!(spans, vec![(8, 15)]);
    }

    #[test]
    fn matching_is_case_insensitive_and_offsets_index_the_original() {
        let spans = lit_spans("The Brass Lantern.", &set(["lantern", "brass"]));
        assert_eq!(spans, vec![(4, 9), (10, 17)]);
    }

    /// The anchoring that stops `rug` lighting inside `shrug` — the same rule
    /// `vocab::absent_nouns` uses, for the same reason.
    #[test]
    fn a_word_inside_another_word_does_not_light() {
        assert!(lit_spans("You shrug.", &set(["rug"])).is_empty());
        assert!(lit_spans("mailboxes", &set(["mailbox"])).is_empty());
        // …but the same word standing on its own does.
        assert_eq!(lit_spans("a rug", &set(["rug"])), vec![(2, 5)]);
    }

    #[test]
    fn every_occurrence_on_the_row_lights() {
        assert_eq!(lit_spans("door, door", &set(["door"])), vec![(0, 4), (6, 10)]);
    }

    #[test]
    fn overlapping_words_are_reported_once_each_and_in_order() {
        let spans = lit_spans("iron door", &set(["door", "iron"]));
        assert_eq!(spans, vec![(0, 4), (5, 9)], "sorted by position, not by word");
    }

    #[test]
    fn nothing_lit_is_no_spans() {
        assert!(lit_spans("You are in an open field.", &set([])).is_empty());
        assert!(lit_spans("", &set(["door"])).is_empty());
    }

    /// The reveal states its own claim rather than letting a player infer a
    /// stronger one (SQ-1135): these are the story's WORDS, not its scope.
    #[test]
    fn the_caveat_says_what_the_reveal_cannot_promise() {
        assert!(CAVEAT.contains("not necessarily"), "{CAVEAT:?}");
    }
}
