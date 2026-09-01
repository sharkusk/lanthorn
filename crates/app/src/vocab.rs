//! The story's own vocabulary, offered when the parser cannot have understood
//! the player (SQ-1041).
//!
//! ```text
//! > light lanturn
//! I don't know the word "lanturn".
//! ● this story knows — lantern
//! ```
//!
//! This is the first feature to speak through [`crate::assist`]. Guess-the-verb
//! and its quieter cousin, the near-miss spelling, are the canonical way these
//! games fail a player who has no nostalgia for them: the player knows what they
//! want, the story knows the word, and the two never meet.
//!
//! # It OFFERS; it never substitutes
//!
//! The original proposal was a pre-parser that understood a larger vocabulary and
//! rewrote the command before the game saw it. That is the wrong shape, and the
//! quest inverts it. A wrong rewrite does not fail loudly — `light lanturn`
//! silently becoming `light lantern` is fine right up until it becomes `burn
//! lantern`, which costs a turn, possibly a life, and the player never learns it
//! happened. A wrong suggestion costs a keystroke. So the command goes through
//! untouched and lanthorn speaks only afterwards, and only to name words the
//! story itself holds.
//!
//! # How we know the parser rejected a word — without reading the game's prose
//!
//! Every family phrases the refusal differently (`[I don't know the word
//! "lanturn".]`, `You can't see any such thing.`, `You use word(s) I don't
//! know!`), and a detector built on those strings is broken by the next game.
//! It does not have to be one: the story's dictionary is a static table, so **a
//! word absent from it cannot be understood, and that is knowable without the
//! game saying anything at all**. [`Engine::knows_word`] asks the story's own
//! dictionary — through `zvm`'s encoder, so the Z-machine's key truncation is
//! applied the way the game applies it — and the answer is the same whatever the
//! game prints.
//!
//! [`Engine::knows_word`]: crate::engine::Engine::knows_word
//!
//! # Where a candidate may come from
//!
//! Four sources, and [`StoryVocabulary::candidates`] simply concatenates them:
//!
//! 1. **A near miss.** `lanturn` is one keystroke from `lantern`, and a
//!    near-miss against a word this story really holds is strong evidence.
//! 2. **A different ending.** `lighting`, `lights` and `lighted` all stem to
//!    `light`. The dictionary truncates (`lighti` in a Version 3 game), so the
//!    stem has to be built rather than found by prefix.
//! 3. **What the word MEANS.** `illuminate` → `light` is the frustration
//!    everybody names first and the one thing a story file cannot answer: edit
//!    distance puts it eight keystrokes away, stemming reaches nothing, and the
//!    story's own synonym groups only ever group words it already knows. It
//!    takes a corpus, and [`verb_synonyms`] is that corpus — the games' own verb
//!    groupings, then WordNet — shipped as a table and read lazily on the first
//!    rejected word (SQ-1110, SQ-1115, wired in SQ-1119).
//! 4. **The story's own synonyms.** Once a VERB is identified, whichever source
//!    found it, [`grammar_model::Verb::words`] is every spelling the dictionary
//!    gives it — free, and on all three engines.
//!
//! The first two and the fourth are answerable from the story file alone; the
//! third is the only one that is not, and it needed no seam of its own to arrive
//! — which is why the sources are a concatenation and not a chain.
//!
//! Whatever proposes a candidate, [`StoryVocabulary::offer`] intersects it with
//! this story's dictionary before anything is shown. **The player must never be
//! shown a word the parser would reject** — that is the invariant a new source
//! must not be able to break, so the gate is unconditional rather than a promise
//! each source keeps for itself.
//!
//! # And it stays quiet unless it is confident
//!
//! An assist read on its twentieth firing is the test the register sets, and a
//! suggestion that fires on every failed turn is wallpaper. Four gates, in
//! [`offer_vocabulary`] and [`StoryVocabulary::offer`]:
//!
//! * **exactly one** word of the command is unknown — two is a sentence about
//!   things this story has never heard of, or a name typed at a prompt, not a
//!   command with one word wrong in it;
//! * an unknown word in the **opening** position is answered with verbs and
//!   nothing else, and one **inside** the command with words the dictionary marks
//!   as things rather than actions;
//! * the miss is a **single keystroke**, a plain change of ending, or an EXACT
//!   hit in the meaning table — nothing weaker, because a coincidence at
//!   distance two is how a player's name gets answered with `march`, and a fuzzy
//!   match into the table would be a typo guess chained onto a meaning guess;
//! * and a word is answered **once a session**.
//!
//! With nothing that passes, we say nothing. That is still the common answer:
//! meaning is the source most able to find *something*, so it is held to exact
//! lookups, to the opening word, and to verbs the story itself holds.
//!
//! # And then it TRIES them, before you see them (SQ-1121)
//!
//! Everything above establishes that the story's dictionary holds a word. That
//! guarantee is real and weak: `light` being a word does not mean `light lamp`
//! does anything *here*. So each surviving candidate is typed into a silent,
//! disposable copy of this very game — [`crate::probe`] — from exactly where the
//! player is standing, and only the ones that did something are shown. Zork I at
//! the front door says nothing for `illuminate lamp`; five rooms later, with the
//! lantern in sight, it says `try instead — light`.
//!
//! That is what earns the wording. `this story knows` is a fact about a table;
//! `try instead` is a recommendation, and would make this feature's own failure
//! worse if it were made without evidence. With the probe off, unavailable, or
//! busy with the previous turn's question, the line drops back to the fact.
//!
//! # And the trying happens off the main thread (SQ-1124)
//!
//! [`offer_vocabulary`] ASKS and returns; [`poll_vocabulary_offer`] shows the
//! line when the shadow answers, which the event loop does every pass. The
//! game's reply prints immediately and the offer arrives a beat later, through
//! the same insert-above-prompt every assist has always used — the pane is
//! bottom-anchored, so history scrolls up and the cursor does not move, and
//! in-progress input is not in the transcript at all (it lives in
//! `AppState::input` until Enter), so nothing the player has typed is touched.
//!
//! SQ-1121 spent up to 400 ms of the player's turn on this. A budget is a cap on
//! a stall, not a fix for one, and it forced a `too_slow` latch that wrote off
//! Counterfeit Monkey after one measurement. Both are gone.
//!
//! **An answer that arrives after the player has typed again is dropped.** It
//! describes a command that is no longer the last one on screen, and a
//! suggestion under the wrong command is worse than no suggestion. SQ-1125's
//! prompt-anchored hint would have made lateness invisible and is parked, so
//! until it lands lateness is answered with silence.
//!
//! Two consequences that were decided rather than discovered:
//!
//! * **Silence now carries information** — "that would not have worked". The
//!   leak is mild and deliberate: it narrows candidates the player already
//!   provoked by typing something, so they learn nothing about an action they
//!   did not reach for. Contrast the command band's VERB column (SQ-1111), which
//!   is NOT vetted and must not be: a list quietly filtered to what works here
//!   and now is very close to solving the room, which is the hints' job, and the
//!   hints wait to be asked. The two paths deliberately share no filter.
//! * **A vet is evidence, not a guarantee.** A game drawing on randomness can
//!   answer the shadow and the live session differently, and a refusal the
//!   probe's controls never provoke — Inform's "That's not something you can
//!   open." — reads as a success and survives. The offer is still an offer: the
//!   command goes to the game untouched and a wrong suggestion costs a
//!   keystroke.

use std::collections::{BTreeMap, BTreeSet};

use grammar_model::{Verb, WordRoles};

use crate::engine::Engine;
use crate::state::AppState;

// ── The vocabulary a story has, as one engine-neutral value ─────────────────

/// Where in the command the unknown word sat.
///
/// The parser reads the first word as the action, so the two positions want
/// different answers and mixing them is what makes existing interactive-fiction
/// help feel stupid: a noun offered where a verb belongs is not a command, and a
/// verb offered where a noun belongs names nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// The first word — the story reads this as the action.
    Opening,
    /// Any later word — part of a noun phrase.
    Inside,
}

/// What a story's parser will accept, as much of it as an offer needs: the verbs
/// with their sentence shapes, the whole dictionary with its parts of speech, and
/// how much of a word the dictionary keeps.
///
/// Built once per session from [`Engine::story_vocabulary`] and cached in
/// [`VocabState`] — the tables are static, so no later turn can change an answer
/// here.
#[derive(Debug, Clone, Default)]
pub struct StoryVocabulary {
    verbs: Vec<Verb>,
    /// Dictionary spelling → index into `verbs`; the first verb claiming a
    /// spelling wins, as in both engines' readers.
    by_word: BTreeMap<String, usize>,
    /// Every word the dictionary holds, as stored, with its parts of speech.
    words: BTreeMap<String, WordRoles>,
    /// Each stored word cut to [`key_len`](Self::key_len), pointing back at the
    /// spelling it was cut from. A Z-machine or Glulx key is already at most that
    /// long and maps to itself; a Scott Adams database lists its words in full and
    /// then matches only the first `word_length` characters of them, so `score`
    /// and `scoreboard` both have to arrive at the same entry.
    by_trunc: BTreeMap<String, String>,
    /// Every literal word the grammar uses — the story's prepositions.
    prepositions: BTreeSet<String>,
    /// How many CHARACTERS of a word the dictionary keeps; 0 = the whole word.
    /// Six or nine on the Z-machine (§13.3/§13.4's six and nine Z-characters),
    /// `DICT_WORD_SIZE` on Glulx, the header's word length on a Scott Adams
    /// database. `flashlight` is stored as `flashl` in a Version 3 game, and
    /// comparing untruncated forms would report every long word unknown.
    key_len: usize,
}

impl StoryVocabulary {
    /// Assemble the snapshot. The four facts are one subject — a caller holding
    /// the verbs always holds the rest — so they arrive together rather than
    /// being filled in field by field.
    pub fn new(
        verbs: Vec<Verb>,
        words: BTreeMap<String, WordRoles>,
        prepositions: BTreeSet<String>,
        key_len: usize,
    ) -> StoryVocabulary {
        let mut by_word = BTreeMap::new();
        for (i, v) in verbs.iter().enumerate() {
            for w in &v.words {
                by_word.entry(w.to_lowercase()).or_insert(i);
            }
        }
        let cut = |w: &str| -> String {
            if key_len == 0 {
                w.to_string()
            } else {
                w.chars().take(key_len).collect()
            }
        };
        let mut by_trunc = BTreeMap::new();
        for w in words.keys() {
            by_trunc.entry(cut(w)).or_insert_with(|| w.clone());
        }
        StoryVocabulary { verbs, by_word, words, by_trunc, prepositions, key_len }
    }

    /// Drop every dictionary word this story's own tokeniser would never hand
    /// its parser as one token — a word no sequence of keystrokes can reach
    /// (SQ-1151).
    ///
    /// Arthur's dictionary holds both `be` and `be?`, two genuinely distinct
    /// verbs in the game's own data ($c7 and $f7), and `?` is one of the six
    /// input separators Arthur declares in its dictionary header. So the
    /// tokeniser splits `be?` into `be` and `?` before the parser looks anything
    /// up: the entry exists, and **no player can type it**. Infocom used that
    /// deliberately — a separator inside a word makes a slot the game's own code
    /// can reference without a player stumbling into it. Offering it in the verb
    /// column wastes a slot and misleads, and clicking it composes a line the
    /// parser will split.
    ///
    /// **The story is asked where a word ends, not told.**
    /// [`Engine::split_like_parser`] is `zvm::dictionary::tokenise`, the routine
    /// `read` itself calls, so the separator set is the one THIS story declares
    /// at its dictionary header (§13.1) rather than a set assumed here. An engine
    /// that lends no tokeniser answers `None` and nothing is dropped, which is
    /// the right answer: without the story's own splitter there is no authority
    /// to drop a word on.
    ///
    /// A word that IS a separator survives, because it tokenises to itself: the
    /// bare `?` Arthur also files is a real parser token, and a word that
    /// *contains* a separator is not the same thing as a word that *is* one.
    ///
    /// Measured over every story in `stories/` with a readable dictionary,
    /// twelve hold at least one such word and none of them is a word a player
    /// would ever want offered:
    ///
    /// | story | dropped |
    /// |---|---|
    /// | `arthur-r74-s890714.z6` | `be?` `don't` `end.of.` `int.num` `int.tim` `l.g` `no.word` |
    /// | `shogun-r322-s890706.z6` | `be?` `end.of.in` `int.num` `int.tim` `l.g` `no.word` |
    /// | `zork0-r393-s890714.z6` | `end.of.` `int.num` `int.tim` `no.word` |
    /// | `moonmist-r9-s861022.z3` | 19 possessives — `dee's`, `iris'`, `jack'`, … |
    /// | `trinity-r12-s860926.z4` | `p.a.` |
    /// | `enchanter-r29-s860820.z3` | one entry that decodes with spaces in it |
    /// | five Inform stories | `comma,` (and `LostPig.z8` four more of its kind) |
    ///
    /// The two shapes are worth telling apart. Infocom's V6 titles keep private
    /// slots (`int.num`, `no.word`) whose names are unreachable ON PURPOSE, which
    /// is the same trick as `be?`. **Moonmist is the one that proves the set has
    /// to come from the story**: it declares `'` a separator and Enchanter does
    /// not, so `dee's` is one word in Enchanter's parser and three in Moonmist's,
    /// and no fixed table could have got both right.
    ///
    /// Applied at [`VocabState::get`], the one vocabulary seam (SQ-1117), so
    /// every surface reading the snapshot — the verb column, the guidance offer,
    /// the word reveal, completion — is spared it for the same reason.
    pub fn without_untypeable_words(self, engine: &dyn Engine) -> StoryVocabulary {
        // The story's answer, or no answer: one token out means the parser can
        // be handed this word whole.
        let typeable = |w: &str| engine.split_like_parser(w).is_none_or(|toks| toks.len() == 1);

        if self.words.keys().all(|w| typeable(w)) {
            return self;
        }
        let StoryVocabulary { mut verbs, words, prepositions, key_len, .. } = self;
        for v in &mut verbs {
            v.words.retain(|w| typeable(w));
        }
        let words: BTreeMap<String, WordRoles> =
            words.into_iter().filter(|(w, _)| typeable(w)).collect();
        let prepositions: BTreeSet<String> =
            prepositions.into_iter().filter(|w| typeable(w)).collect();
        StoryVocabulary::new(verbs, words, prepositions, key_len)
    }

    /// True when there is nothing here worth consulting. A menu-driven Version 6
    /// game has no grammar at all, and an empty dictionary can answer nothing.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// `word` cut down to what the dictionary would store of it.
    fn truncated(&self, word: &str) -> String {
        if self.key_len == 0 {
            word.to_string()
        } else {
            word.chars().take(self.key_len).collect()
        }
    }

    /// Does the story's dictionary hold this word?
    ///
    /// The fallback for an engine that cannot answer for itself — exact for
    /// Glulx and for a Scott Adams database, both of which truncate by plain
    /// characters. The Z-machine truncates by Z-CHARACTERS, so `GameSession`
    /// overrides [`Engine::knows_word`] with its own encoder and never reaches
    /// this.
    pub fn knows(&self, word: &str) -> bool {
        self.stored(word).is_some()
    }

    /// The dictionary entry a spelling reaches, truncation included: `examine`
    /// finds the `examin` a Version 3 dictionary actually stores.
    fn stored(&self, word: &str) -> Option<(&String, WordRoles)> {
        let w = word.to_lowercase();
        let key = self.by_trunc.get(&self.truncated(&w))?;
        self.words.get_key_value(key).map(|(k, r)| (k, *r))
    }

    /// The verb a spelling reaches, truncation included.
    pub fn verb_named(&self, word: &str) -> Option<&Verb> {
        let (stored, _) = self.stored(word)?;
        self.by_word.get(stored).map(|&i| &self.verbs[i])
    }

    /// Every verb, in grammar-table order.
    pub fn verbs(&self) -> &[Verb] {
        &self.verbs
    }

    /// Every dictionary word the story marks as a thing rather than an action,
    /// in dictionary order. Used to build a control command a probe can be sure
    /// of (SQ-1121) — a real noun the parser will reach for and fail to find.
    pub fn nouns(&self) -> impl Iterator<Item = &str> {
        self.words
            .iter()
            .filter(|(w, r)| {
                (r.noun || r.adjective) && !self.by_word.contains_key(*w) && !self.is_preposition(w)
            })
            .map(|(w, _)| w.as_str())
    }

    /// The parts of speech this story gives `word`, truncation included —
    /// `None` when the dictionary does not hold it at all.
    ///
    /// The one accessor SQ-1116 could not add because another lane held this
    /// file, and the reason its noun scrape shows `the`, `a`, `you` and `my` on
    /// Glulx: those genuinely are dictionary words, and only the story's own
    /// **role** bits separate a thing from a function word. An English stop list
    /// cannot — it was tried, and it also hid `here` from a story that
    /// implements it (SQ-1116). See [`WordRoles`] for which bit each back-end
    /// sets; note that Infocom marks a true adjective and neither Inform
    /// back-end has such a bit, so an Inform adjective arrives as a noun.
    pub fn roles(&self, word: &str) -> Option<WordRoles> {
        self.stored(word).map(|(_, r)| r)
    }

    /// True if the grammar writes this word literally into some line.
    fn is_preposition(&self, word: &str) -> bool {
        self.prepositions.contains(&word.to_lowercase())
    }
}

// ── What a thing may be called ──────────────────────────────────────────────

/// The text to put on the input line for `obj` — a name this story's parser has
/// agreed to accept — or `None` when the story holds no text for the object at
/// all.
///
/// The command band used to offer the object's **printed** name, which is what
/// the game writes when it mentions the thing and not what it answers to. The
/// two barely overlap: Zork I prints `brass lantern` and its `SYNONYM` property
/// holds `lamp`, `lanter` and `light`; it prints `bird's nest`, `ZORK owner's
/// manual` and `number of ghosts`, none of which the parser can even tokenise
/// as written, because `bird's`, `owner's` and `number` are not in its
/// dictionary. A panel offering those is asserting something the story never
/// promised.
///
/// So the name is composed of the printed name's own words, keeping only what
/// the parser will take:
///
/// 1. **The noun** is the LAST printed word the object itself answers to —
///    `coins` in `leather bag of coins`, `ghosts` in `number of ghosts`, `nest`
///    in `bird's nest`. Last, not first, because English puts the head of the
///    phrase there and the parser resolves on it.
/// 2. **The adjectives** are the unbroken run of words before it that the story
///    marks as adjectives, or that are the object's own parse names — `small
///    mailbox`, `brass lantern`. It matters: `take rusty` and `take knife` are
///    both needed when two knives are in the room. The run stops at the first
///    word that is neither, which is what keeps the `of` out of `bag of coins`
///    and the `owner's` out of `owner's manual`.
/// 3. Where the printed name yields nothing — an **Inform 7** object, which has
///    no printed name at all and whose word list is the only text naming it —
///    the story's own first spelling for the object is used. Any one of an
///    object's parse names identifies it on its own; the first is the story's
///    own first answer, and choosing among them on any other grounds would be
///    the guessing this replaces.
/// 4. Where the story keeps no parse names anywhere — Journey has no parser,
///    `advent.z8` brings its own — the printed name stands as it is. That is
///    the old behaviour, kept for exactly the stories that can support nothing
///    better.
///
/// **Where "the story marks it an adjective" comes from, in order.** On an
/// Infocom V4+ story the object's own [`Adjectives`](grammar_model::Adjectives)
/// answer it — `refers_to` covers them — and that is the reliable source, since
/// it is the property the game's own parser reads. The dictionary's DESC bit is
/// the fallback for V1–3, where the adjectives are one-byte numbers `zvm` cannot
/// locate (SQ-1120); every V1–5 Infocom story in `stories/` sets that bit, so
/// the fallback holds there, but the three Infocom **V6** games set it on
/// almost nothing — Zork Zero on 11 of 1624 words, Shogun 17 of 1389, Arthur 9
/// of 1059, and `WordRoles::adjective` is not even decoded for their flag
/// layout. Those three are exactly the games the property now answers for.
///
/// `vocab` is `None` before the story's grammar has been read, and then only
/// the object's own words can qualify a leading adjective; on Inform, where
/// adjectives live in the name property beside the nouns, that loses nothing.
pub fn typeable_name(
    obj: &grammar_model::ObjectWords,
    vocab: Option<&StoryVocabulary>,
) -> Option<String> {
    let tokens = printed_tokens(obj);
    if let Some(noun) = tokens.iter().rposition(|t| obj.refers_to(t)) {
        let qualifies =
            |t: &str| obj.refers_to(t) || vocab.is_some_and(|v| v.roles(t).is_some_and(|r| r.adjective));
        let mut start = noun;
        while start > 0 && qualifies(tokens[start - 1]) {
            start -= 1;
        }
        return Some(tokens[start..=noun].join(" "));
    }
    obj.words.first().cloned().or_else(|| obj.display_name())
}

/// Every word the parser accepts for `obj`, spelled the way the story SPELLS
/// it wherever it can be — the completion source for what is in scope.
///
/// Two things a caller cannot get from the word list alone:
///
/// - **The full spelling.** A Version 3 dictionary keeps six Z-characters, so
///   Zork I stores `lanter` and nothing more. Completing a player's `lan` to
///   `lanter` offers them a fragment; the object's printed name holds `lantern`
///   in full, and both reach the same entry because the parser truncates the
///   player's word exactly as the dictionary truncated its own.
/// - **The adjective.** Infocom keeps adjectives in a property of their own, so
///   `brass` is nowhere in the lantern's `SYNONYM` list — but `take brass
///   lantern` works, and a player who can see "brass lantern" written on the
///   screen will type `bra` before they type `lam`.
///
/// So: the printed name's own words where the story answers to them or marks
/// them adjectives, then every stored word that no such spelling already
/// reaches — the nouns, and then the adjectives where the story keeps a list
/// this can be read from. Zork I's lantern gives `brass`, `lantern`, `lamp`,
/// `light`.
///
/// The stored adjectives matter beyond the printed name: Beyond Zork prints one
/// of its keys as `key` and nothing more, while storing `mauve`, `second`,
/// `gray` and `grey` for it — words the screen never shows and the printed name
/// therefore cannot supply. On a story whose adjectives cannot be read —
/// every V1–3 Infocom game — the list is the nouns plus whatever the printed
/// name and the dictionary's DESC bit between them can recover, which is what
/// SQ-1042 shipped and all that is available there.
pub fn typeable_words(
    obj: &grammar_model::ObjectWords,
    vocab: Option<&StoryVocabulary>,
) -> Vec<String> {
    let cut = |w: &str| -> String {
        match obj.truncated_at {
            Some(n) => w.to_lowercase().chars().take(n).collect(),
            None => w.to_lowercase(),
        }
    };
    let mut out: Vec<String> = Vec::new();
    for t in printed_tokens(obj) {
        let known = obj.refers_to(t)
            || vocab.is_some_and(|v| v.roles(t).is_some_and(|r| r.adjective || r.noun));
        if known && !out.iter().any(|w| w.eq_ignore_ascii_case(t)) {
            out.push(t.to_lowercase());
        }
    }
    for w in obj.words.iter().chain(obj.adjectives.words()) {
        if !out.iter().any(|t| cut(t) == cut(w)) {
            out.push(w.clone());
        }
    }
    out
}

/// The printed name split the way a player reads it: words, stripped of the
/// punctuation a story sets around them.
fn printed_tokens(obj: &grammar_model::ObjectWords) -> Vec<&str> {
    obj.printed_name
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-'))
        .filter(|t| !t.is_empty())
        .collect()
}

// ── The offer ───────────────────────────────────────────────────────────────

/// One word this story holds that the player may have meant.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    /// The dictionary spelling, exactly as the story stores it.
    word: String,
    /// 0 for a word the player nearly typed, 1 for one the typed word MEANS,
    /// 2 for another spelling the story gives whatever either of those found.
    ///
    /// Three ranks and not two, because `order` is an index into whichever table
    /// a source read and says nothing across sources: `doff` reaches `remove`
    /// first in the synonym table and `carry` first in Zork's own verb entry,
    /// both at 0, and the tie was settled alphabetically in favour of the aside.
    /// The evidence is what separates them — the form itself, then the meaning,
    /// then what the story calls the answer as well.
    tier: usize,
    /// How far from what was typed — the edit distance, or 0 for a stem.
    distance: usize,
    /// Where the word sits in the table it came from — the dictionary, or the
    /// verb's own list of spellings — so the story's ordering breaks a tie.
    order: usize,
    /// True when this spelling is a WHOLE word in its own right rather than a
    /// dictionary key that may be sitting at the truncation limit.
    ///
    /// `remove` is six characters and a whole word; `leafle` is six characters
    /// and a fragment, and nothing about either string says which — only where
    /// it came from does. Everything drawn from the story's own tables is a key
    /// and goes through [`spell_out`](StoryVocabulary::spell_out); the synonym
    /// table's members are English and go through nothing. Without this, `doff`
    /// answered with `carry · catch · get`, because the one word that was right
    /// looked like a fragment and was dropped in favour of its own asides.
    whole: bool,
}

/// One word an offer line will name, and how it was arrived at.
///
/// [`Candidate`] is this module's private working note; a `Pick` is what leaves,
/// and it carries the one fact about a word that outlives the ranking: whether
/// the player REACHED for it or lanthorn PROPOSED it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    /// The word to show, spelled the way the player should type it.
    pub word: String,
    /// True when nothing about the typed WORD reached this one — the meaning
    /// table proposed it from what the typed word means, so it is a different
    /// word the player never typed and never nearly typed
    /// ([`by_meaning`](StoryVocabulary::by_meaning), tier 1).
    ///
    /// False for a near miss and for a changed ending, which are evidence about
    /// the word actually typed, and for the story's own other spellings of a
    /// verb one of those found — an aside on a word already reached.
    ///
    /// The distinction is not cosmetic. `molst` → `molest` is a correction and
    /// belongs to the player; `sod` → `fuck` is lanthorn saying a word of its
    /// own, and the two cannot be told apart downstream once the line is a list
    /// of strings (SQ-1145).
    pub proposed: bool,
}

/// The most an offer may name. Three, and it is a limit rather than a target:
/// the whole verb list is useless (Zork I knows hundreds) and a list long enough
/// to scan is a list the player reads instead of playing.
pub const MAX_OFFERED: usize = 3;

/// The shortest word a DISTANCE can be evidence about. Below this every
/// dictionary has a neighbour one keystroke away and the evidence is worthless:
/// `cas` is one edit from `case`, `cat`, `car`, `cap` and `gas`, and a rule that
/// answers it is answering the alphabet.
///
/// **It belongs to [`by_near_miss`](StoryVocabulary::by_near_miss) alone**, and
/// SQ-1144 moved it there from the top of [`offer`](StoryVocabulary::offer),
/// where it had been gating every source. The reasoning above is about edit
/// distance and holds nowhere else: an exact hit in WordNet's exception list is
/// not a weak guess that gets weaker as the word shortens — `lit` → `light` is a
/// morphological FACT, and so is `don` → `wear` in the synonym table. Applied to
/// those, length was silencing `lit`, `ate`, `saw`, `won` and `got` on the
/// strength of an argument that was never about them (SQ-1113 found it; the case
/// it left behind is now inverted in `vocabulary_offer.rs`).
///
/// What still protects the exact sources is what always protected them: they
/// look their answer up rather than guessing it, and `offer` then intersects
/// whatever they propose with this story's own dictionary. The table proposes;
/// the story disposes — at three letters exactly as at eight.
const MIN_LEN: usize = 4;

/// Words the parser ignores and a shape count must ignore with it.
const ARTICLES: &[&str] = &["the", "a", "an", "some", "my", "his", "her", "its", "their"];

impl StoryVocabulary {
    /// Every word this story holds that the player may have meant by `typed`.
    ///
    /// The sources are CONCATENATED, not chained: each proposes independently
    /// and the ranking in [`offer`](Self::offer) settles them. SQ-1119's
    /// meaning-driven source joined the list here as one more line, knowing
    /// nothing about the ones beside it — which is what the shape was for.
    fn candidates(&self, typed: &str, position: Position) -> Vec<Candidate> {
        let mut out = Vec::new();
        self.by_near_miss(typed, position, &mut out);
        self.by_ending(typed, position, &mut out);
        self.by_meaning(typed, position, &mut out);
        // Meaning speaks only where FORM reached nothing. A near miss or a
        // changed ending is evidence about the word the player really typed; a
        // synonym is a guess at what they meant, and with the first in hand the
        // second is wallpaper — `opening mailbox` wants `open`, and two games in
        // the corpus put `look` and `read` on that same verb, so the offer read
        // `open · read · look` until this line. It sits here rather than in the
        // ranking below because the aside source builds on whatever it finds: a
        // proposal that is not going to be shown must not leave its asides
        // behind, spelled by a verb nothing else reached.
        if out.iter().any(|c| c.tier == 0) {
            out.retain(|c| c.tier != 1);
        }
        self.by_story_synonym(position, &mut out);
        out
    }

    /// One keystroke wrong: a substitution, an insertion, a deletion or a
    /// transposition. Nothing further — at distance two a six-letter word matches
    /// something in every dictionary, and answering a player's name with `march`
    /// is exactly the wallpaper the register forbids.
    ///
    /// The comparison is between TRUNCATED forms, because that is the comparison
    /// the story's own parser makes. A Version 3 dictionary stores `lanter`, so
    /// `lanturn` is two edits from what is on disk and one edit from what the
    /// parser would have matched — and a rule about keystrokes has to be read in
    /// the space the keystrokes are judged in, or the commonest near miss in the
    /// commonest games is out of reach.
    ///
    /// And this is the ONE source [`MIN_LEN`] governs, on both sides of the
    /// comparison: a dictionary word shorter than it is not worth proposing, and
    /// a TYPED word shorter than it is not worth answering, because at three
    /// characters the whole dictionary is one keystroke away. That second half
    /// used to sit at the top of [`offer`](Self::offer) and gate the exact
    /// sources with it; it is here now because the argument is here (SQ-1144).
    fn by_near_miss(&self, typed: &str, position: Position, out: &mut Vec<Candidate>) {
        let key = self.truncated(typed);
        if key.chars().count() < MIN_LEN {
            return;
        }
        for (order, (word, roles)) in self.words.iter().enumerate() {
            if !self.fills(word, *roles, position) || word.chars().count() < MIN_LEN {
                continue;
            }
            if osa(&key, &self.truncated(word)) == 1 {
                out.push(Candidate { word: word.clone(), tier: 0, distance: 1, order, whole: false });
            }
        }
    }

    /// A different ending on the same word: `lighting`, `lights` and `lighted`
    /// are all `light`. Prefix matching cannot find this — the dictionary stores
    /// `lighti` for the first of them in a Version 3 game — so the stem is built
    /// and then looked up.
    fn by_ending(&self, typed: &str, position: Position, out: &mut Vec<Candidate>) {
        for stem in stems(typed) {
            let Some((word, roles)) = self.stored(&stem) else { continue };
            if !self.fills(word, roles, position) {
                continue;
            }
            let word = word.clone();
            let order = self.words.keys().position(|k| *k == word).unwrap_or(0);
            if !out.iter().any(|c| c.word == word) {
                out.push(Candidate { word, tier: 0, distance: 0, order, whole: false });
            }
        }
    }

    /// What the word MEANS, when nothing about its FORM can reach the story's.
    /// `illuminate` is eight keystrokes from `light`, stems to nothing, and the
    /// story file records no relation between the two — the bridge is
    /// [`verb_synonyms`]'s shipped table, harvested offline from the games' own
    /// verb groupings and from WordNet.
    ///
    /// Three rules the table states, all of them the caller's to keep:
    ///
    /// * **Lemmatise first.** Its keys are base forms, because a parser accepts
    ///   the imperative, so `illuminating` goes through [`stems`] before it is
    ///   looked up. Skip that and a missing morphology step here reads as a hole
    ///   in the data.
    /// * **Exact lookups only.** Fuzzy-matching thousands of table keys would
    ///   chain a typo guess onto a meaning guess with nothing anchoring either;
    ///   the near miss belongs against the story's OWN dictionary, which is
    ///   [`by_near_miss`](Self::by_near_miss)'s job and already done.
    /// * **Walk the groups in order and stop early.** A word is polysemous —
    ///   `draw` is *pull*, *sketch* and *attract*, one group per sense, games'
    ///   own groupings first — and a rare fifth sense must not crowd out the
    ///   common first one. [`verb_synonyms::suggest`] is that walk.
    ///
    /// Verbs only, so the opening word only: the table says what an ACTION is
    /// called, and that is the one place an action stands. What it proposes is
    /// the table's own spelling rather than the key the dictionary stores —
    /// `examine`, not the `examin` a Version 3 game keeps — because the parser
    /// truncates it to the same entry and `examine` is the word to type.
    fn by_meaning(&self, typed: &str, position: Position, out: &mut Vec<Candidate>) {
        if position != Position::Opening {
            return;
        }
        for lemma in std::iter::once(typed.to_string()).chain(stems(typed)) {
            let known = |w: &str| self.stored(w).is_some_and(|(s, r)| self.fills(s, r, position));
            for (order, word) in
                verb_synonyms::suggest(&lemma, known, MAX_OFFERED).into_iter().enumerate()
            {
                if !out.iter().any(|c| c.word == word) {
                    out.push(Candidate {
                        word: word.to_string(),
                        tier: 1,
                        distance: 0,
                        order,
                        whole: true,
                    });
                }
            }
        }
    }

    /// What else this story calls the verbs already found. `Verb::words` is every
    /// dictionary spelling of one verb, so a story that groups `take` with `get`
    /// and `hold` teaches the player its own vocabulary at no cost — and after
    /// every other source, whichever one found the verb, because it is an aside.
    fn by_story_synonym(&self, position: Position, out: &mut Vec<Candidate>) {
        if position != Position::Opening {
            return;
        }
        let found: Vec<String> = out.iter().map(|c| c.word.clone()).collect();
        for w in &found {
            let Some(verb) = self.verb_named(w) else { continue };
            for (order, other) in verb.words.iter().enumerate() {
                // A one-letter abbreviation (`q`, `x`, `g`) is real vocabulary
                // and a wasted slot: the offer has three, and a player reading
                // `quit · q` learned one word.
                if other.chars().count() < 2 {
                    continue;
                }
                if other != w && !out.iter().any(|c| c.word == *other) {
                    out.push(Candidate {
                        word: other.clone(),
                        tier: 2,
                        distance: 0,
                        order,
                        whole: false,
                    });
                }
            }
        }
    }

    /// Can this dictionary word stand where the unknown one stood? The opening
    /// word of a command is the action, so only a verb belongs there; anywhere
    /// else it is part of a noun phrase, and a verb is not.
    fn fills(&self, word: &str, roles: WordRoles, position: Position) -> bool {
        match position {
            Position::Opening => self.by_word.contains_key(word),
            Position::Inside => {
                !self.by_word.contains_key(word) && (roles.noun || roles.adjective)
            }
        }
    }

    /// The words to offer for `typed`, which sat at `position` and was followed
    /// by `rest`. `prose` is what the story has printed so far — see
    /// [`spell_out`](Self::spell_out).
    ///
    /// Empty when nothing is confident enough to say, which is the common answer
    /// and the important one.
    pub fn offer(
        &self,
        typed: &str,
        position: Position,
        rest: &[&str],
        prose: &[String],
    ) -> Vec<String> {
        self.offer_picks(typed, position, rest, prose).into_iter().map(|p| p.word).collect()
    }

    /// [`offer`](Self::offer), with each pick's provenance still attached.
    ///
    /// The words are identical and in the same order; all this adds is
    /// [`Pick::proposed`], because *where* a word came from is the whole of what
    /// separates a correction from a proposal — and that distinction is gone the
    /// moment the line is a `Vec<String>`. Nothing here consults it: what a
    /// caller does with a proposal is the caller's judgement, and this file
    /// deliberately holds none (SQ-1145).
    pub fn offer_picks(
        &self,
        typed: &str,
        position: Position,
        rest: &[&str],
        prose: &[String],
    ) -> Vec<Pick> {
        let typed = typed.to_lowercase();
        // No length gate here. It lived at this line until SQ-1144 and refused
        // every word under four characters before a single source ran — which
        // meant an argument about EDIT DISTANCE was deciding the fate of two
        // sources that measure no distance at all. It now sits inside
        // [`by_near_miss`](Self::by_near_miss), which is the only source it was
        // ever reasoning about; see [`MIN_LEN`].
        if self.is_empty() {
            return Vec::new();
        }
        let mut found = self.candidates(&typed, position);

        // The invariant, applied once and to everything: only words THIS story
        // holds are ever shown. Every source above already draws from the
        // dictionary, so today this removes nothing — it is here so that a source
        // added later cannot put a word on screen that the parser would refuse.
        found.retain(|c| self.knows(&c.word) && c.word != typed);

        // The sentence the player typed, as the last tie-break and no more than
        // that. `SyntaxLine::accepts` matches on the NUMBER of noun phrases and
        // the literal prepositions, never on which object — whether a verb
        // applies to *that* lantern is decided by the game at runtime and is not
        // in the tables — so it separates candidates that are otherwise equal and
        // is not evidence on its own.
        let (nouns, preps) = self.shape(rest);
        let preps: Vec<&str> = preps.iter().map(String::as_str).collect();
        let misfits = |c: &Candidate| match self.verb_named(&c.word) {
            Some(v) => !v.accepts(nouns, &preps),
            None => true,
        };

        found.sort_by_key(|c| (c.tier, c.distance, misfits(c), c.order, c.word.clone()));
        let mut seen = BTreeSet::new();
        let mut picks = Vec::new();
        for c in found {
            // A dictionary KEY still sitting at the truncation limit may be a
            // fragment — `exam`, `leafle` — and the story would accept it typed
            // back. As the ANSWER that is worth showing; as an aside beside a
            // word we did spell out it is only noise, so `look · exam · desc`
            // becomes `look`. A word that is whole by construction is neither:
            // it is English, and the story's prose has no say in how it is
            // spelled.
            let word = if c.whole {
                c.word.clone()
            } else {
                match (self.spell_out(&c.word, prose), c.tier) {
                    (Some(w), _) => w,
                    (None, 0) => c.word.clone(),
                    (None, _) => continue,
                }
            };
            if seen.insert(word.clone()) {
                picks.push(Pick { word, proposed: c.tier == 1 });
            }
            if picks.len() == MAX_OFFERED {
                break;
            }
        }
        picks
    }

    /// A dictionary key, spelled the way the story spells it — `None` when it
    /// sits at the truncation limit and the story has never printed it whole.
    ///
    /// A Version 3 dictionary keeps six characters, so it holds `leafle` for the
    /// leaflet and `mailbo` for the mailbox. Both are what the parser matches and
    /// both would be typed back successfully — and `● this story knows — leafle`
    /// reads as our bug, which is exactly the misattribution the register exists
    /// to prevent.
    ///
    /// The story has already printed the whole word, so that is where it is
    /// recovered from: a word in the transcript that truncates to this key is the
    /// key's full spelling, and typing it reaches the same entry. The SHORTEST
    /// such word wins, so `bottled` never stands in for `bottle`; a key the prose
    /// spells exactly is already whole; and a key the prose has never carried is
    /// offered as stored, because a truncated key is still a word that works.
    ///
    /// Only the NEWEST [`SPELLING_LOOKBACK`] transcript lines are read
    /// (SQ-1180). This runs per candidate of a rejected word, and lowercasing
    /// every word of an unbounded transcript per candidate grows without limit
    /// over a session; the spelling on offer is about what the player has been
    /// reading, and a word last printed hundreds of lines ago is offered as
    /// stored — still typeable — exactly as one the prose never carried.
    fn spell_out(&self, stored: &str, prose: &[String]) -> Option<String> {
        if self.key_len == 0 || stored.chars().count() != self.key_len {
            return Some(stored.to_string());
        }
        let mut best: Option<String> = None;
        for line in prose.iter().rev().take(SPELLING_LOOKBACK) {
            // Split on the hyphen too: `lantern-bearer` is a compound of the
            // story's prose, not a spelling of the key, and it would otherwise
            // outrank the word itself.
            for w in line.split(|c: char| !c.is_alphanumeric() && c != '\'') {
                let w = w.to_lowercase();
                if w == stored {
                    return Some(stored.to_string());
                }
                if w.chars().count() > self.key_len
                    && self.truncated(&w) == stored
                    && best.as_ref().is_none_or(|b| w.len() < b.len())
                {
                    best = Some(w);
                }
            }
        }
        best
    }

    /// How many noun phrases the player supplied and which prepositions they
    /// used, reading their words the way the parser splits them: a run of words
    /// between prepositions is one noun phrase, and an article is not a word.
    fn shape(&self, rest: &[&str]) -> (usize, Vec<String>) {
        let mut nouns = 0;
        let mut preps = Vec::new();
        let mut in_phrase = false;
        for w in rest {
            let w = w.to_lowercase();
            if ARTICLES.contains(&w.as_str()) {
                continue;
            }
            if self.is_preposition(&w) {
                preps.push(w);
                in_phrase = false;
            } else if !in_phrase {
                nouns += 1;
                in_phrase = true;
            }
        }
        (nouns, preps)
    }
}

/// The words `w` might be an inflected form of.
///
/// Two sources, and they answer the two halves of English morphology.
///
/// The regular endings are a RULE — strip `ing`, `ed`, `es`, `s` and put back
/// the letter the spelling dropped — and it is applied generously, because every
/// stem is looked up in the story's dictionary afterwards and a stem that is not
/// a word costs nothing.
///
/// The irregulars are a TABLE, because no rule can reach them: `lit` shares no
/// letters with the ending that would have made it from `light`, and neither do
/// `took`, `went` or `mice`. That table is WordNet's own exception list, shipped
/// by [`verb_synonyms::irregular_bases`] (SQ-1113) — and it is consulted for
/// every word rather than only where the rule came up empty, because it is one
/// hash lookup and some words are reached by both.
///
/// Nouns as well as verbs: this is asked about every position in a command, and
/// `mice` → `mouse` is the same case as `lit` → `light` one slot to the right.
fn stems(w: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if s.chars().count() >= 2 && !out.contains(&s) {
            out.push(s);
        }
    };
    for suffix in ["ing", "ed", "es", "s", "en", "er", "ly"] {
        let Some(base) = w.strip_suffix(suffix) else { continue };
        let ch: Vec<char> = base.chars().collect();
        if ch.len() < 2 {
            continue;
        }
        push(base.to_string());
        push(format!("{base}e")); // taking → take
        if ch[ch.len() - 1] == ch[ch.len() - 2] && !"aeiou".contains(ch[ch.len() - 1]) {
            push(ch[..ch.len() - 1].iter().collect()); // running → run
        }
        if ch[ch.len() - 1] == 'i' {
            let mut y: String = ch[..ch.len() - 1].iter().collect();
            y.push('y');
            push(y); // carries → carry
        }
    }
    // A form can be an inflection of two different words — `axes` is `ax` and
    // `axis` — so every base is proposed and the dictionary lookup settles it,
    // exactly as it settles the endings above.
    for base in verb_synonyms::irregular_bases(w) {
        push((*base).to_string());
    }
    out
}

/// Optimal string alignment distance: a substitution, an insertion, a deletion
/// or a TRANSPOSITION each cost one.
///
/// Transpositions have to cost one or the commonest typo of all is out of reach
/// — plain Levenshtein scores `tkae` against `take` at two, the same as an
/// unrelated word, so the offer would either miss every swapped pair or have to
/// admit coincidences at distance two in order to catch them.
fn osa(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    if n.abs_diff(m) > 1 {
        return 2; // more than one keystroke apart; the exact figure is unused
    }
    let mut prev2 = vec![0usize; m + 1];
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (cur[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(prev2[j - 2] + 1);
            }
            cur[j] = v;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

// ── Session state ───────────────────────────────────────────────────────────

/// The story's vocabulary as this session has it, and what has already been
/// offered.
///
/// Deliberately not persisted: a saved game restored a week later is a session
/// that has said nothing yet, and the player will have forgotten anyway.
#[derive(Debug, Clone, Default)]
pub struct VocabState {
    loaded: bool,
    story: Option<StoryVocabulary>,
    /// Unknown words already answered, so the twentieth `lanturn` is silent.
    offered: BTreeSet<String>,
}

impl VocabState {
    /// The story's vocabulary, read from the engine the first time it is asked
    /// for. `None` for a story with no readable dictionary — a menu-driven
    /// Version 6 game, or an engine that has none to give.
    ///
    /// The words no player can type are dropped here, once, for every surface at
    /// the same time — see
    /// [`StoryVocabulary::without_untypeable_words`] (SQ-1151).
    pub fn get(&mut self, engine: &dyn Engine) -> Option<&StoryVocabulary> {
        if !self.loaded {
            self.loaded = true;
            self.story = engine
                .story_vocabulary()
                .map(|v| v.without_untypeable_words(engine))
                .filter(|v| !v.is_empty());
        }
        self.story.as_ref()
    }

    /// The story's vocabulary and the words already answered, in ONE borrow.
    ///
    /// Both are needed to decide whether to speak, and the story is borrowed out
    /// of the same value the set lives in — so asking for them separately is a
    /// borrow error rather than a design choice (SQ-1121).
    fn story_and_answered(
        &mut self,
        engine: &dyn Engine,
    ) -> Option<(&StoryVocabulary, &BTreeSet<String>)> {
        self.get(engine)?;
        Some((self.story.as_ref()?, &self.offered))
    }

    /// Record `word` as answered, so the twentieth `lanturn` is silent.
    ///
    /// Split from the ASKING half (SQ-1121), which
    /// [`story_and_answered`](Self::story_and_answered) serves, because the two
    /// no longer happen together: the question is asked before a shadow is
    /// woken, and the answer is recorded only if a line was actually shown.
    fn mark_offered(&mut self, word: &str) {
        self.offered.insert(word.to_string());
    }
}

// ── Reading the command ─────────────────────────────────────────────────────

/// Split a typed command the way a parser would: words, lowercased, with the
/// punctuation a player sprinkles on stripped off.
fn words_of(cmd: &str) -> Vec<String> {
    cmd.split(|c: char| c.is_whitespace() || c == ',' || c == '.' || c == ';')
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'').to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

// ── The vetting, and the claim it earns ─────────────────────────────────────

/// What the light says about words it has only LOOKED UP: this story's
/// dictionary holds them, and nothing more is claimed.
pub const LEAD_DICTIONARY: &str = "this story knows — ";

/// What it says about words it has watched WORK, in a copy of this game, from
/// where the player is standing (SQ-1121).
///
/// A recommendation rather than a fact, so it is only ever used for an offer
/// that came back through [`crate::probe`]. `try instead`, not "you may want to
/// try": the story owns the second person (see [`crate::assist`]), and the
/// imperative reads at one suggestion as well as at four.
pub const LEAD_VETTED: &str = "try instead — ";

/// The nonsense a shadow types so this story will show how it says no.
///
/// Six characters, so a Version 3 dictionary's truncation cannot land it on a
/// real word by accident, and three of them so a story that happens to hold one
/// still has a spare.
const NONSENSE: [&str; 3] = ["zqxwvj", "vprkxz", "jwqzbf"];

/// Everything the player can reach right now, as [`ObjectWords`]: what is in
/// the room, and what they are carrying.
///
/// The room's contents come through `room_objects_excluding`, which drops the
/// player object — structurally a child of whatever room they are standing in,
/// so without that every room of every game would contain the adventurer
/// (SQ-0667).
///
/// **Both halves nest and neither opens a shut container** (SQ-1133). The
/// carried half was the player's DIRECT children for as long as the room half
/// had been descending into open holders, so the same brown sack listed its
/// lunch on Zork I's kitchen table and hid it the moment you picked the sack up.
/// One walk answers both now — see
/// [`crate::engine::Introspect::visible_contents`] — and its guarantee is the
/// old one: a container the player has not opened contributes nothing, at any
/// depth.
///
/// **`None` is not the same answer as an empty list, and the difference decides
/// what a caller may claim.** `None` means the question could not be ASKED —
/// the engine has no [`Introspect`](crate::engine::Introspect) at all (Glulx
/// and Scott Adams today), or it has one that cannot say where the player is or
/// who they are. An empty `Some` means it was asked and the answer is nothing:
/// an empty room, carrying nothing. A caller that flattens the two reports "no
/// objects are in scope" about a story it never managed to read, which is the
/// sort of confident wrong answer this crate keeps having to un-tell (see
/// [`crate::state::HereSource`], which turns exactly this distinction into what
/// the command band's WHAT column is allowed to call itself).
pub fn objects_in_scope(engine: &dyn Engine) -> Option<Vec<crate::engine::ObjectWords>> {
    let (mut here, carried) = scope_split(engine, None)?;
    here.extend(carried);
    Some(here)
}

/// [`objects_in_scope`] with the two halves still apart: what is HERE, and what
/// is CARRIED, in that order.
///
/// The command band draws them as two columns and needs them separately; every
/// other caller wants them concatenated. **One function either way** — this used
/// to be written out four times (here, `command_band::refresh_objects`,
/// `input::refresh_scope_words`, and the vetting harness), and the four spellings
/// were the hand-maintained invariant CLAUDE.md's refactoring policy names: each
/// looked entirely reasonable alone, and SQ-1133 found the carried half reading
/// direct children in all of them while the room half had nested since SQ-0678.
///
/// **Both halves nest, and neither opens a shut container.** The room's contents
/// come through `room_objects_excluding` and the player's through
/// [`crate::engine::Introspect::visible_contents`]; those are the same walk, so
/// Zork I's brown sack lists its lunch on the kitchen table and in your hands
/// under exactly one rule, and lists it in neither place while it is shut.
///
/// `player_hint` is the app's own locked-in player object
/// ([`crate::state::AppState::player_obj`]) where the caller has one: for a
/// story whose player object is not NAMED, that id came from watching what moved
/// between rooms and is the only answer there is — `player_object()` returns
/// `None` there and the carried half would come back empty. Pass `None` to ask
/// the engine.
///
/// `None` (returned) carries the same meaning it does for
/// [`objects_in_scope`].
pub fn scope_split(
    engine: &dyn Engine,
    player_hint: Option<u16>,
) -> Option<(Vec<crate::engine::ObjectWords>, Vec<crate::engine::ObjectWords>)> {
    let intro = engine.introspect()?;
    let player = player_hint.or_else(|| intro.player_object());
    let room = engine.current_location().map(|l| l.number);
    // Neither half readable: the seam exists but this story is not answering, so
    // the question was not really asked.
    if player.is_none() && room.is_none() {
        return None;
    }
    let here = match room {
        Some(r) => intro.room_objects_excluding(r, player),
        None => Vec::new(),
    };
    let carried = match player {
        Some(p) => intro.visible_contents(p),
        None => Vec::new(),
    };
    Some((here, carried))
}

/// Two dictionary nouns that are not here, for the control commands — see
/// [`crate::probe`] for what a pair of them is worth and why one is worth
/// nothing.
///
/// Two questions, and only one of them is still a guess.
///
/// **Is anything in scope called this?** Exact, since SQ-1042 landed
/// [`ObjectWords`] through the `Introspect` seam (folded in here by SQ-1124).
/// Every object the player can see is asked [`ObjectWords::refers_to`], which
/// compares against the words the STORY files the object under — truncated the
/// way the story's own dictionary truncates them, so Zork I's stored `lanter`
/// answers to `lantern`. It replaces a substring match over lowercased *printed*
/// names, which is a different question with the same shape: `You can't see any
/// black book here` names a book the parser calls `black`, `book`, `prayer`,
/// `bible` and `works`, and matching on what it PRINTS finds one of the five.
/// Picking a noun that is really here makes the control succeed, the pair
/// disagree, and the run learn nothing — the failure mode that keeps an
/// unjudgeable suggestion rather than dropping a bad one.
///
/// **Has the story MENTIONED it lately?** Still a guess, and unavoidably so:
/// nothing hands us the scope of a room's description. The tail of the
/// transcript is what the player can see on screen, and a thing named there is
/// very likely still around. Word-anchored rather than a bare substring — a
/// dictionary noun matches a prose word it is the front of (which is what
/// truncation and plurals need) and not a word it merely sits inside, where
/// `rug` disqualified itself on `shrug`.
///
/// The remaining guess is exactly why the probe believes a pair only when both
/// answer identically.
///
/// # Two callers, one spelling of "here"
///
/// [`objects_in_scope`] is the reader; this asks it the negative question and
/// [`crate::reveal`] asks it the positive one. One function because "what can
/// the player see?" answered two ways in two files is the hand-maintained
/// invariant CLAUDE.md's refactoring policy names — and the two answers would
/// diverge silently, since each looks entirely reasonable on its own.
fn absent_nouns(v: &StoryVocabulary, engine: &dyn Engine, prose: &[String], avoid: &[String]) -> Vec<String> {
    let in_scope = objects_in_scope(engine).unwrap_or_default();
    // The tail of the transcript, split into words, which is what the player can
    // see on screen and therefore roughly what is around them.
    let mentioned: BTreeSet<String> = prose
        .iter()
        .rev()
        .take(PROSE_LOOKBACK)
        .flat_map(|line| line.split(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();

    v.nouns()
        .filter(|w| w.chars().count() >= 3)
        .filter(|w| !avoid.iter().any(|a| a == w))
        .filter(|w| !in_scope.iter().any(|o| o.refers_to(w)))
        .filter(|w| {
            let lower = w.to_lowercase();
            !mentioned.iter().any(|m| m.starts_with(&lower))
        })
        .take(2)
        .map(str::to_string)
        .collect()
}

/// How many transcript lines back count as "what the player can see".
const PROSE_LOOKBACK: usize = 40;

/// How many transcript lines back [`StoryVocabulary::spell_out`] reads for a
/// truncated key's full spelling (SQ-1180).
///
/// Five times [`PROSE_LOOKBACK`]: the spelling question reaches further than
/// "on screen right now" — the player may be retyping a word from a room
/// description a few screens up — but not into the whole session, whose
/// transcript this scan lowercased word by word, per candidate, per rejection.
/// Two hundred lines is several screenfuls of prose; a word the story has not
/// printed in that long is offered as stored, which the parser accepts anyway.
const SPELLING_LOOKBACK: usize = 200;

/// Which command in a `run` answered a candidate, and which pair of controls (if
/// any) judges it. Indices into the run's steps, in the order they were typed.
type Slot = (usize, Option<(usize, usize)>);

/// A vocabulary offer that has been asked of the shadow and is waiting for its
/// answer (SQ-1124).
///
/// Everything the answer needs in order to be turned into a line, held on
/// [`AppState`] rather than on a thread: which words were offered, which step of
/// the run judges each of them, and — the two that decide whether the answer is
/// still wanted at all — the shadow's own token and the turn it belongs to.
#[derive(Debug)]
pub struct PendingOffer {
    /// The question this offer is waiting on. An answer carrying any other token
    /// belongs to a question this offer did not ask.
    token: u64,
    /// The turn it was asked on ([`AppState::turn_epoch`]). If the player has
    /// typed again the offer is stale, and a stale offer is dropped rather than
    /// printed under a command that never provoked it.
    epoch: u64,
    /// The unknown word the player typed, recorded as answered only if a line is
    /// actually shown.
    word: String,
    /// The candidates, in the order they will be named.
    picks: Vec<String>,
    /// One entry per candidate, parallel to `picks`.
    plan: Vec<Slot>,
    /// How many commands were sent. A run that came back shorter left the tail
    /// unjudged, and a partly-vetted offer cannot make the vetted claim.
    commands: usize,
}

/// Lay out the commands that would vet `picks`, and the plan for reading the
/// answer back.
///
/// `None` when this story cannot be asked the question at all: no nonsense word
/// its dictionary lacks, or fewer than two nouns believably out of scope for the
/// controls (see [`absent_nouns`]).
///
/// Every control is laid out in the SAME run as the question it judges, from the
/// same snapshot and therefore the same room. See [`crate::probe`]'s module docs
/// for why a signature learned once a session is a signature of the wrong room.
fn vetting_plan(
    engine: &dyn Engine,
    v: &StoryVocabulary,
    prose: &[String],
    words: &[String],
    at: usize,
    picks: &[String],
) -> Option<(Vec<String>, Vec<Slot>)> {
    let knows = |w: &str| engine.knows_word(w).unwrap_or_else(|| v.knows(w));
    let nonsense = NONSENSE.iter().find(|w| !knows(w))?.to_string();
    let absent = absent_nouns(v, engine, prose, words);
    let [absent_a, absent_b] = absent.as_slice() else { return None };

    // Which word of the command the controls swap out. The player's substituted
    // word when it is a NOUN; the command's last word when they substituted the
    // VERB, so the control keeps the sentence's shape and only loses its object.
    let noun_slot = if at > 0 { Some(at) } else { words.len().checked_sub(1).filter(|&i| i > 0) };

    let mut cmds: Vec<String> = vec![nonsense];
    let mut plan: Vec<Slot> = Vec::new();
    let mut controls: BTreeMap<String, usize> = BTreeMap::new();
    for pick in picks {
        let mut w = words.to_vec();
        w[at] = pick.clone();
        let candidate = w.join(" ");
        let pair = noun_slot.map(|slot| {
            let mut idx = [0usize; 2];
            for (k, noun) in [absent_a, absent_b].into_iter().enumerate() {
                let mut c = w.clone();
                c[slot] = noun.clone();
                let text = c.join(" ");
                idx[k] = *controls.entry(text.clone()).or_insert_with(|| {
                    cmds.push(text);
                    cmds.len() - 1
                });
            }
            (idx[0], idx[1])
        });
        cmds.push(candidate);
        plan.push((cmds.len() - 1, pair));
    }
    (cmds.len() <= crate::probe::MAX_PROBES).then_some((cmds, plan))
}

/// Read a finished run against the plan that produced it, and keep only the
/// candidates that did something.
///
/// `None` means **no vetting happened** — the run came back short, or this story
/// showed nothing believable about how it refuses. That is not "everything
/// failed": the caller falls back to the modest claim it can still support.
/// `Some(vec![])` means the vetting ran and nothing survived it, and then the
/// light says nothing at all.
fn judge(run: &crate::probe::ProbeRun, offer: &PendingOffer) -> Option<Vec<String>> {
    // A run that came back short would leave the tail unjudged, and a
    // partly-vetted offer cannot make the vetted claim.
    if run.steps.len() != offer.commands {
        return None;
    }
    // The unknown word is the one control every story answers, so if even that
    // taught nothing there is no signature to judge against.
    let base = run.refusal_from(0);
    if base.is_empty() {
        return None;
    }
    Some(
        offer
            .picks
            .iter()
            .zip(&offer.plan)
            .filter(|(_, (cand, pair))| {
                let mut refusals = base.clone();
                if let Some((a, b)) = *pair {
                    refusals.merge(run.refusal_from_pair(a, b));
                }
                run.did_something(*cand, &refusals)
            })
            .map(|(p, _)| p.clone())
            .collect(),
    )
}

// ── The turn hook ───────────────────────────────────────────────────────────

/// What one call to [`offer_vocabulary`] decided.
enum Outcome {
    /// Say this now: nothing was asked of the shadow, so the claim is the modest
    /// one the dictionary alone supports.
    Now(String),
    /// The shadow was asked. Nothing is said until [`poll_vocabulary_offer`]
    /// collects the answer — or drops it, if the player has moved on.
    Asked(PendingOffer),
}

/// Offer the story's own vocabulary, if there is anything worth offering, for a
/// command holding exactly one word the story's dictionary does not have.
///
/// Called once per completed line-input turn, after the game's own reply is in
/// the transcript, so the offer reads underneath the refusal it answers.
/// `printed` says whether the turn produced any output at all: a turn that
/// printed nothing rejected nothing.
///
/// With `guidance_probe` on, every candidate is tried in a silent copy of the
/// game (SQ-1121) and only the ones that did something are shown — which is what
/// lets the line say `try instead` rather than `this story knows`. The count can
/// now shrink to nothing for that second reason, and then nothing is said and the
/// word is NOT recorded as answered: the lamp may be in the next room, and the
/// same suggestion may be right there.
///
/// **The probing does not happen here** (SQ-1124). This function asks and
/// returns; [`poll_vocabulary_offer`] shows the line a beat later, from the event
/// loop. A turn that cannot ask — the seam unarmed, `guidance_probe` off, the
/// shadow already busy with the previous turn's question — says the unvetted
/// thing immediately, exactly as before.
pub fn offer_vocabulary(state: &mut AppState, engine: &dyn Engine, cmd: &str, printed: bool) {
    // Whatever the previous turn was still waiting on cannot be printed under
    // this one. Dropped here as well as at collection, so a pending offer whose
    // answer never comes does not sit on the state for the rest of the session.
    if state.vocab_pending.as_ref().is_some_and(|p| p.epoch != state.turn_epoch) {
        state.vocab_pending = None;
    }
    // Asked HERE as well as at `push_assist`'s door, and only as an early exit:
    // with the light off there is no reason to read a story's grammar tables, and
    // no word may be recorded as answered by a line nobody was shown.
    if !state.config.guidance || !printed {
        return;
    }
    let words = words_of(cmd);
    if words.is_empty() {
        return;
    }

    // What the story has printed, so a truncated dictionary key can be shown the
    // way the story itself spells it. Taken from the transcript rather than the
    // last turn alone: the leaflet is named the turn it is revealed and mistyped
    // several turns later.
    let prose = std::mem::take(&mut state.transcript);
    let mut vocab = std::mem::take(&mut state.vocab);
    let mut probe = std::mem::take(&mut state.probe);
    let may_probe = state.config.guidance_probe;
    let epoch = state.turn_epoch;
    // The one thing the offer takes from configuration, and it is taken HERE
    // rather than inside [`StoryVocabulary`] (SQ-1145). The tables know which
    // picks lanthorn PROPOSED rather than corrected; what may be said out loud is
    // the config's judgement, and `Config::spoken_offer` is where the two meet.
    let config = &state.config;
    let outcome = (|| {
        let prose = &prose;
        let (v, answered) = vocab.story_and_answered(engine)?;
        let knows = |w: &str| engine.knows_word(w).unwrap_or_else(|| v.knows(w));
        // EXACTLY one word wrong. Two is a sentence about things this story has
        // never heard of, or a name typed at a prompt — and speaking into a name
        // prompt is a far worse mistake than staying quiet.
        let mut unknown = words.iter().enumerate().filter(|(_, w)| !knows(w));
        let (at, word) = unknown.next()?;
        if unknown.next().is_some() {
            return None;
        }
        // A bare number is a menu answer or a disambiguation, never a word
        // somebody meant to spell.
        if word.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let position = if at == 0 { Position::Opening } else { Position::Inside };
        let rest: Vec<&str> = words[at + 1..].iter().map(String::as_str).collect();
        let picks = config.spoken_offer(v.offer_picks(word, position, &rest, prose));
        // Empty because nothing was confident enough, or because everything that
        // was got filtered — one answer either way, and it is silence. The word
        // is NOT recorded as answered: nothing was said, so nothing was spent.
        if picks.is_empty() {
            return None;
        }
        // The one-per-session question is asked before anything is planned: a
        // word already answered is not worth a single silent turn in the shadow,
        // let alone nine.
        if answered.contains(word) {
            return None;
        }
        let asked = may_probe
            .then(|| vetting_plan(engine, v, prose, &words, at, &picks))
            .flatten()
            .and_then(|(cmds, plan)| {
                let token = probe.ask(engine, &cmds)?;
                Some(PendingOffer {
                    token,
                    epoch,
                    word: word.clone(),
                    picks: picks.clone(),
                    plan,
                    commands: cmds.len(),
                })
            });
        Some(match asked {
            Some(pending) => Outcome::Asked(pending),
            None => {
                vocab.mark_offered(word);
                Outcome::Now(format!("{LEAD_DICTIONARY}{}", picks.join(" · ")))
            }
        })
    })();
    state.vocab = vocab;
    state.probe = probe;
    state.transcript = prose;

    match outcome {
        Some(Outcome::Now(line)) => state.push_assist(&crate::assist::Assist::help(line)),
        Some(Outcome::Asked(pending)) => state.vocab_pending = Some(pending),
        None => {}
    }
}

/// Collect an answer the shadow has finished with and show the offer it earned —
/// or drop it (SQ-1124).
///
/// Called from the event loop every pass. Returns whether the transcript changed,
/// which is the caller's redraw contribution.
///
/// Three ways this says nothing, and they are different:
///
/// * **stale** — the player has typed again since the question was asked, so the
///   answer describes a command that is no longer the last one. Printing it would
///   attach a suggestion to the wrong turn; SQ-1125's prompt-anchored hint would
///   have made lateness invisible and is parked, so the answer is discarded.
///   Silently, which is this feature's existing discipline rather than a new
///   rule.
/// * **nothing survived** — the vetting ran and every candidate did nothing here.
///   The word is deliberately NOT recorded as answered: the lamp may be in the
///   next room.
/// * **nothing was learned** — the run could not answer (it came back short, or
///   the story showed nothing believable about how it refuses), and then the
///   offer falls back to the claim the dictionary alone supports.
pub fn poll_vocabulary_offer(state: &mut AppState) -> bool {
    let Some(answer) = state.probe.poll() else { return false };
    deliver(state, answer)
}

/// True when `token` answers a question THIS consumer asked.
///
/// The shadow is shared — [`crate::return_probe`] asks it too (SQ-0785) — and it
/// hands back one answer at a time with no idea who wanted it. So the event
/// loop's single collector routes by token
/// (`loop_tick::poll_shadow_answers`) rather than letting each consumer
/// poll in turn: a consumer that polls and finds an answer it does not own has
/// already taken it off the channel, and the one that did want it never sees it.
pub fn owns(state: &AppState, token: u64) -> bool {
    state.vocab_pending.as_ref().is_some_and(|p| p.token == token)
}

/// [`deliver`] for the router, which has already matched the token.
pub fn deliver_answer(state: &mut AppState, answer: crate::probe::Answer) -> bool {
    deliver(state, answer)
}

// The wrap cache, and why the late insert does not defer to a keystroke gap.
//
// An insert above the prompt moves the one line the cache has already
// wrapped — the trailing prompt itself — so before SQ-1179 it was a
// `TranscriptEdit::Rewrote` and the next frame rebuilt the whole wrap. Measured
// at 40 columns in a debug build
// (`render::transcript::tests::a_late_insert_above_the_prompt_repairs_the_wrap_exactly_once`,
// before the fix): 1.3 ms at 200 transcript lines, 4.0 ms at 1,000, 18.4 ms at
// 5,000 and 71.8 ms at 20,000, against a flat 0.43 ms for a cached frame —
// linear in scrollback, and not free.
//
// SQ-1179 gave the edit its own `TranscriptEdit::Inserted { at, count }`, which
// the wrap cache can REPAIR through instead: every line before `at` provably
// did not move, so only the (typically one-line) tail is re-wrapped. What was
// a rebuild is now flat again, like the cached-frame number above rather than
// the scrollback-linear ones beside it.
//
// The cost this comment used to describe is nevertheless still paid where it
// always was for anything that ISN'T an insert-above-the-prompt — a resize, a
// filter, a theme, or any other `Rewrote`. Every `push_transcript_internal` in
// inline-prompt mode used to be the same edit — every `/help`, every save
// banner, every other assist — so before the fix this was the register's
// standing cost rather than a new one, and the SYNCHRONOUS offer paid it too.
// What changed with SQ-1124 alone (the deferred offer, prior to this fix) was
// only that the frame it landed on might be one the player is typing into; and
// the event loop already coalesces an input burst (`skip_draw` defers the draw
// and leaves `needs_redraw` set), so even the pre-SQ-1179 rebuild was paid ONCE
// per burst rather than per keystroke.

/// [`poll_vocabulary_offer`], but waits for the answer instead of collecting one
/// that has already arrived.
///
/// **Not for the event loop.** It is what a test harness and a measurement
/// harness need: a turn plus the beat afterwards, in one call, so a case can
/// assert on what the player would eventually have seen without racing it.
pub fn settle_vocabulary_offer(state: &mut AppState) -> bool {
    let Some(answer) = state.probe.settle() else { return false };
    deliver(state, answer)
}

/// Turn one collected answer into a line, or into silence. See
/// [`poll_vocabulary_offer`] for the three ways it says nothing.
fn deliver(state: &mut AppState, answer: crate::probe::Answer) -> bool {
    let Some(pending) = state.vocab_pending.take() else { return false };
    if pending.token != answer.token {
        // Not this offer's answer. Put it back: the one it is waiting for may
        // still be coming.
        state.vocab_pending = Some(pending);
        return false;
    }
    if pending.epoch != state.turn_epoch {
        return false; // stale — the player typed again
    }
    let vetted = answer.run.as_ref().and_then(|run| judge(run, &pending));
    let (picks, lead) = match vetted {
        Some(kept) => (kept, LEAD_VETTED),
        None => (pending.picks, LEAD_DICTIONARY),
    };
    // Vetting can empty the list, and then there is nothing to recommend.
    if picks.is_empty() {
        return false;
    }
    state.vocab.mark_offered(&pending.word);
    let before = state.transcript.len();
    state.push_assist(&crate::assist::Assist::help(format!("{lead}{}", picks.join(" · "))));
    state.transcript.len() != before
}


#[cfg(test)]
mod tests {
    use super::*;
    use grammar_model::{NounKind, Slot, SyntaxLine, Token};

    fn noun() -> Slot {
        Slot::one(Token::Noun(NounKind::Noun))
    }

    fn word(w: &str) -> Slot {
        Slot::one(Token::Word(w.to_string()))
    }

    fn roles(verb: bool, noun: bool) -> WordRoles {
        let mut r = WordRoles::default();
        r.verb = verb;
        r.noun = noun;
        r
    }

    /// A pocket Zork with a Version 3 dictionary — six-character keys, so
    /// `examine` is stored as `examin` and `lantern` fits whole.
    fn pocket_zork() -> StoryVocabulary {
        let verbs = vec![
            Verb::new(
                255,
                0,
                vec!["light".into(), "burn".into()],
                vec![SyntaxLine::new(1, false, vec![noun()])],
            ),
            Verb::new(
                254,
                0,
                vec!["take".into(), "get".into(), "hold".into()],
                vec![
                    SyntaxLine::new(5, false, vec![noun()]),
                    SyntaxLine::new(6, false, vec![noun(), word("from"), noun()]),
                ],
            ),
            Verb::new(253, 0, vec!["examin".into()], vec![SyntaxLine::new(7, false, vec![noun()])]),
        ];
        let mut words = BTreeMap::new();
        for w in ["light", "burn", "take", "get", "hold", "examin"] {
            words.insert(w.to_string(), roles(true, false));
        }
        // A Version 3 dictionary cannot store a key longer than six characters,
        // so the lantern is on disk as `lanter` — which is what makes the prose
        // the only place its whole spelling exists.
        for w in ["lanter", "lamp", "sword", "case", "the"] {
            words.insert(w.to_string(), roles(false, true));
        }
        let preps: BTreeSet<String> = ["from"].iter().map(|s| s.to_string()).collect();
        StoryVocabulary::new(verbs, words, preps, 6)
    }

    /// The headline case: one keystroke wrong on a noun, answered with the word
    /// the story actually holds.
    ///
    /// `lanturn` is TWO edits from the `lanter` on disk and one from what the
    /// parser would have matched, so this also pins that the comparison happens
    /// in the parser's own truncated space — falsify by comparing untruncated
    /// forms and the offer disappears.
    #[test]
    fn a_near_miss_is_answered_with_the_word_the_story_holds() {
        let v = pocket_zork();
        let prose = vec!["A battery-powered brass lantern is on the trophy case.".to_string()];
        assert_eq!(v.offer("lanturn", Position::Inside, &[], &prose), vec!["lantern"]);
        assert_eq!(v.offer("swrod", Position::Inside, &[], &[]), vec!["sword"]);
    }

    /// The story printed the whole word, so that is where the whole word comes
    /// from. Without the prose the key is offered as stored — still a word the
    /// parser accepts, which is why it is shown rather than swallowed.
    #[test]
    fn a_truncated_key_is_spelled_out_of_the_storys_own_prose() {
        let v = pocket_zork();
        assert_eq!(v.offer("lanturn", Position::Inside, &[], &[]), vec!["lanter"]);
        let prose = vec!["The lanterns and the lantern-bearer are here.".to_string()];
        assert_eq!(
            v.offer("lanturn", Position::Inside, &[], &prose),
            vec!["lantern"],
            "the shortest spelling wins, so `lanterns` never stands in for `lantern`"
        );
    }

    /// The spelling scan reads the newest [`SPELLING_LOOKBACK`] lines and no
    /// more (SQ-1180): a sighting the session has since scrolled that far past
    /// resolves as stored, exactly like one the prose never carried, while the
    /// same sighting inside the window still answers.
    #[test]
    fn the_spelling_scan_reads_the_newest_lines_and_stops() {
        let v = pocket_zork();
        let mut prose = vec!["A battery-powered brass lantern is on the trophy case.".to_string()];
        prose.extend(vec!["Time passes.".to_string(); SPELLING_LOOKBACK]);
        assert_eq!(
            v.offer("lanturn", Position::Inside, &[], &prose),
            vec!["lanter"],
            "a sighting pushed past the lookback no longer spells the key out"
        );
        prose.push("The lantern flickers in the draught.".to_string());
        assert_eq!(v.offer("lanturn", Position::Inside, &[], &prose), vec!["lantern"]);
    }

    /// A transposition is one keystroke, and the commonest typo there is — and
    /// identifying the VERB brings the story's own synonyms with it, free.
    #[test]
    fn a_mistyped_verb_brings_the_storys_own_synonyms_with_it() {
        let v = pocket_zork();
        assert_eq!(v.offer("tkae", Position::Opening, &["lamp"], &[]), vec!["take", "get", "hold"]);
        assert_eq!(v.offer("ligth", Position::Opening, &["lamp"], &[]), vec!["light", "burn"]);
    }

    /// An ending the story does not inflect. The dictionary stores `lighti` for
    /// `lighting` in a Version 3 game, so a prefix match cannot find this.
    #[test]
    fn a_different_ending_stems_back_to_the_word_the_story_knows() {
        let v = pocket_zork();
        assert_eq!(v.offer("lighting", Position::Opening, &["lamp"], &[]), vec!["light", "burn"]);
        assert_eq!(v.offer("lanterns", Position::Inside, &[], &[]), vec!["lanter"]);
        assert_eq!(v.offer("taking", Position::Opening, &["lamp"], &[]), vec!["take", "get", "hold"]);
    }

    /// A verb where a noun belongs is not a noun, and a noun where a verb belongs
    /// is not a command. The two positions never trade answers.
    #[test]
    fn the_position_of_the_unknown_word_decides_what_may_answer_it() {
        let v = pocket_zork();
        // `lanturn` is one keystroke from a noun, and nothing a command opens with.
        assert!(v.offer("lanturn", Position::Opening, &[], &[]).is_empty());
        // `tkae` is one keystroke from a verb, and no kind of thing.
        assert!(v.offer("tkae", Position::Inside, &[], &[]).is_empty());
    }

    /// Never more than three, however much the story knows.
    #[test]
    fn an_offer_is_at_most_three_words_long() {
        let v = pocket_zork();
        assert!(v.offer("tkae", Position::Opening, &["lamp"], &[]).len() <= MAX_OFFERED);
    }

    /// Nothing confident, nothing said — the common answer, and the important one.
    #[test]
    fn silence_is_the_common_answer() {
        let v = pocket_zork();
        assert!(v.offer("xyzzy", Position::Opening, &["lamp"], &[]).is_empty());
        assert!(
            v.offer("cas", Position::Inside, &[], &[]).is_empty(),
            "three letters is no evidence of a DISTANCE — `cas` is one keystroke from `case`, \
             and from `car`, `cat`, `cap` and `gas` besides"
        );
        assert!(StoryVocabulary::default().offer("lanturn", Position::Inside, &[], &[]).is_empty());
    }

    /// A story that spells its verbs plainly and keeps whole words — a Glulx
    /// game rather than a Version 3 one, so nothing here is about truncation.
    /// One spelling per verb, so what an offer names came from MEANING and not
    /// from the story's own synonym list.
    fn a_plainly_spelled_story() -> StoryVocabulary {
        let mut verbs = Vec::new();
        let mut words = BTreeMap::new();
        for (i, w) in ["light", "examine", "hide", "wear", "remove", "buy", "help"]
            .iter()
            .enumerate()
        {
            verbs.push(Verb::new(
                200 + i as u32,
                0,
                vec![(*w).to_string()],
                vec![SyntaxLine::new(i as u16, false, vec![noun()])],
            ));
            words.insert((*w).to_string(), roles(true, false));
        }
        words.insert("lamp".to_string(), roles(false, true));
        StoryVocabulary::new(verbs, words, BTreeSet::new(), 0)
    }

    /// **The case the whole synonym effort was for** (SQ-1041 left the seam,
    /// SQ-1110/1115 built the table, SQ-1119 ran the wire). `illuminate` is
    /// eight keystrokes from `light` and stems to nothing, so no source that
    /// reads FORM can reach it; the story's own grammar then throws in `burn`,
    /// which is what makes the sources a concatenation rather than a chain.
    ///
    /// Falsify by dropping `by_meaning` from `candidates`: the offer vanishes,
    /// which is exactly what this assertion said before the wire was run.
    #[test]
    fn a_word_the_story_never_heard_is_answered_by_what_it_means() {
        let v = pocket_zork();
        assert_eq!(v.offer("illuminate", Position::Opening, &["lamp"], &[]), vec!["light", "burn"]);
    }

    /// The mappings SQ-1115 pinned, each on a story that holds exactly one
    /// spelling of the target — so the answer is the table's and nothing else's.
    #[test]
    fn the_canonical_meanings_reach_the_word_the_story_holds() {
        let v = a_plainly_spelled_story();
        for (typed, wanted) in [
            ("illuminate", "light"),
            ("inspect", "examine"), // the one the player reported
            ("conceal", "hide"),
            ("doff", "remove"),
            ("purchase", "buy"),
            ("hint", "help"),
        ] {
            assert_eq!(
                v.offer(typed, Position::Opening, &["lamp"], &[]),
                vec![wanted.to_string()],
                "{typed} means {wanted}"
            );
        }
        // `don` -> `wear` is three characters, and answered (SQ-1144). It was
        // pinned as REFUSED here until then, on `MIN_LEN`'s reasoning that at
        // three letters every dictionary has a neighbour one keystroke away —
        // true, and about a neighbour. This is a LOOKUP: nothing measured a
        // distance, `don` means `wear` in the table whatever its length, and the
        // story is asked whether it holds the answer exactly as it is at eight
        // characters. The gate is now `by_near_miss`'s alone.
        assert_eq!(v.offer("don", Position::Opening, &["lamp"], &[]), vec!["wear"]);
    }

    /// And the silences that still hold at three letters, which is the half
    /// SQ-1144 could most easily have lost. The exact sources were let through;
    /// nothing else was.
    ///
    /// Falsify by deleting the `MIN_LEN` guard from `by_near_miss`: `lam` starts
    /// answering `lamp`, which is the wallpaper the constant exists to prevent —
    /// `lam` is equally one keystroke from `jam`, `ram`, `lab` and `am`.
    #[test]
    fn three_letters_is_still_no_evidence_of_a_near_miss() {
        let v = a_plainly_spelled_story();
        // One keystroke from `lamp`, and from a dozen words this story does not
        // happen to hold. A distance is not evidence at this length.
        assert!(v.offer("lam", Position::Inside, &[], &[]).is_empty());
        // In no table at all, and near nothing: the ordinary answer.
        assert!(v.offer("zug", Position::Opening, &["lamp"], &[]).is_empty());
        // The table PROPOSES and the story DISPOSES, at three letters as at
        // eight: `ate` reaches `eat` in WordNet's exception list, and this story
        // has no `eat` for it to reach.
        assert_eq!(verb_synonyms::irregular_bases("ate"), ["eat"], "the table does reach it");
        assert!(!v.knows("eat"), "and this story does not hold what it reaches");
        assert!(v.offer("ate", Position::Opening, &["lamp"], &[]).is_empty());
    }

    /// The table's keys are BASE FORMS, so an inflected word has to be reduced
    /// before it is looked up — `illuminating` reaches nothing on its own and
    /// `illuminate` reaches `light`. Falsify by looking up only what was typed:
    /// the miss looks like a hole in the data rather than a missing step here.
    #[test]
    fn an_inflected_word_is_lemmatized_before_the_table_is_asked() {
        let v = a_plainly_spelled_story();
        assert_eq!(v.offer("illuminating", Position::Opening, &["lamp"], &[]), vec!["light"]);
        assert_eq!(v.offer("purchased", Position::Opening, &["lamp"], &[]), vec!["buy"]);
    }

    /// Meaning proposes VERBS, and the opening word is the only place a verb
    /// stands. A synonym of an action offered inside a noun phrase names nothing.
    #[test]
    fn meaning_never_answers_a_word_inside_a_noun_phrase() {
        let v = a_plainly_spelled_story();
        assert!(v.offer("illuminate", Position::Inside, &[], &[]).is_empty());
        assert!(v.offer("purchase", Position::Inside, &[], &[]).is_empty());
    }

    /// The table is large and this story is small: a word the story cannot spell
    /// is not offered, however well the table knows it. `enlighten` shares a
    /// group with `illuminate`, `disrobe` with `doff` — neither is here.
    #[test]
    fn meaning_is_still_intersected_with_this_storys_dictionary() {
        let v = a_plainly_spelled_story();
        for typed in ["illuminate", "inspect", "conceal", "doff", "purchase", "hint", "xyzzy"] {
            for w in v.offer(typed, Position::Opening, &["lamp"], &[]) {
                assert!(v.knows(&w), "{w:?} is not in this story's dictionary");
            }
        }
        assert!(
            v.offer("scrutinize", Position::Opening, &["lamp"], &[]).is_empty(),
            "`scrutinize` groups with `audit` and `inspect`, and this story has neither"
        );
    }

    /// Meaning is the answer to what FORM could not reach, and never an addition
    /// to it. `lighting` stems straight to `light`, so the offer is `light` and
    /// the story's own `burn` — and not the four further things three thousand
    /// groups can find to say about a lamp.
    ///
    /// Falsify by dropping the tier-1 filter from `candidates`: `opening
    /// mailbox` starts reading `open · read · look` at Zork I, because two games
    /// in the corpus declared those one verb.
    #[test]
    fn meaning_answers_only_where_the_word_itself_reached_nothing() {
        let v = pocket_zork();
        assert_eq!(v.offer("lighting", Position::Opening, &["lamp"], &[]), vec!["light", "burn"]);
        assert_eq!(v.offer("ligth", Position::Opening, &["lamp"], &[]), vec!["light", "burn"]);
        // …and with no near miss and no stem, it is the only thing left.
        assert_eq!(v.offer("illuminate", Position::Opening, &["lamp"], &[]), vec!["light", "burn"]);
    }

    /// Only words THIS story holds, whatever proposed them. Falsified by pushing
    /// a candidate the dictionary lacks: the gate in `offer` drops it.
    #[test]
    fn nothing_is_offered_that_the_parser_would_reject() {
        let v = pocket_zork();
        for typed in ["lanturn", "swrod", "lighting", "tkae"] {
            for pos in [Position::Opening, Position::Inside] {
                for w in v.offer(typed, pos, &[], &[]) {
                    assert!(v.knows(&w), "{w:?} is not in this story's dictionary");
                }
            }
        }
    }

    /// Truncation is by the dictionary's key length, so a long word finds the key
    /// the story really stores.
    #[test]
    fn a_long_word_is_matched_against_what_the_dictionary_kept_of_it() {
        let v = pocket_zork();
        assert!(v.knows("examination"), "`examin` is what a v3 dictionary stores");
        assert!(!v.knows("xyzzy"));
        assert_eq!(v.verb_named("examine").and_then(Verb::word), Some("examin"));
    }

    #[test]
    fn optimal_string_alignment_counts_a_swap_as_one() {
        assert_eq!(osa("take", "take"), 0);
        assert_eq!(osa("tkae", "take"), 1);
        assert_eq!(osa("takes", "take"), 1);
        assert_eq!(osa("tae", "take"), 1);
        assert_eq!(osa("rake", "take"), 1);
        assert!(osa("illuminate", "light") > 1);
    }

    /// Both halves of English morphology, in the one place they meet: the rule
    /// for the regular endings, and WordNet's exception list for the irregulars
    /// that no rule can produce.
    ///
    /// `stems("lit")` was pinned as EMPTY until SQ-1113 shipped the table — the
    /// limitation was recorded rather than papered over with a rule that
    /// half-works. Falsify the fix by dropping the `irregular_bases` loop from
    /// `stems`: every assertion below `lights` fails, `lit` reaching nothing
    /// first, which is the symptom the quest was filed on.
    #[test]
    fn stems_reach_the_regular_endings_and_the_irregular_ones_too() {
        assert!(stems("lighting").contains(&"light".to_string()));
        assert!(stems("taking").contains(&"take".to_string()));
        assert!(stems("running").contains(&"run".to_string()));
        assert!(stems("carries").contains(&"carry".to_string()));
        assert!(stems("lights").contains(&"light".to_string()));
        // Regular words gain nothing spurious from the table: it holds only the
        // forms a rule cannot make, so a word the rule already handles is not in
        // it at all.
        assert_eq!(stems("lights"), ["light", "lighte"], "an ending, and the `e` it may have lost");
        // The irregulars, which no ending in the loop above can reach.
        assert_eq!(stems("lit"), ["light"]);
        assert_eq!(stems("took"), ["take"]);
        assert_eq!(stems("went"), ["go"]);
        assert_eq!(stems("mice"), ["mouse"], "a NOUN: `stems` serves every position");
        // A form that inflects two ways proposes both, and the story's own
        // dictionary is what settles which one it meant.
        let axes = stems("axes");
        assert!(axes.contains(&"ax".to_string()) && axes.contains(&"axis".to_string()), "{axes:?}");
    }

    #[test]
    fn a_command_is_split_the_way_a_parser_splits_it() {
        assert_eq!(
            words_of("  Light  the LANTURN, please. "),
            ["light", "the", "lanturn", "please"]
        );
        assert!(words_of("   ").is_empty());
    }
}
