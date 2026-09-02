//! Steps 2 and 3 — turn the corpus's own verb entries and WordNet's verb
//! synsets into the shipped table, keeping only the groups a story could ever
//! match, and audit the result against the commonest English verbs.
//!
//! ## Two sources, and which one wins
//!
//! A group comes from one of two places, and they are never merged into each
//! other:
//!
//!   * a **verb entry** some stories declare — the spellings one game's author
//!     wrote on a single verb (`Verb 'examine' 'x' 'inspect' 'describe'`),
//!     harvested by [`crate::harvest`] and read here from `if_groups.tsv`;
//!   * a **WordNet synset**, filtered as it always was.
//!
//! Where they disagree the corpus wins, and the mechanism is the file's line
//! ORDER: a game-derived group ranks ahead of every synset containing the same
//! word, so a consumer walking a word's groups meets it first. Nothing is
//! deleted to make room. WordNet keeps saying what it said, one line lower.
//!
//! The reason is not that WordNet is bad but that it answers a different
//! question. It says how English uses a word; a parser asks which words name
//! one ACTION. On `inspect` the two come apart completely: WordNet's groups for
//! it are `case`, `visit`, `audit`/`scrutinize` — the police-procedural sense —
//! and not one of them contains `examine`, because English does not treat those
//! two as synonyms. Dozens of games do, on the same verb, and that is the fact
//! a player who typed `inspect` needs. The corpus is also free (the grammar is
//! already loaded to read the vocabulary out of) and carries no licence
//! obligation, being read out of behaviour rather than out of a lexicon.
//!
//! ## What is NOT done: no union across entries
//!
//! Two stories may group a word differently, and the tempting move — take every
//! pair of spellings that ever shared a verb and close it transitively — is a
//! catastrophe. One chain through a light verb (`get`, `turn`, `set`) collapses
//! half the table into a single group, which is the same failure the "exactly
//! one hop" rule below exists to prevent. So there is no union at all: a group
//! is ONE verb entry, exactly as some author wrote it. What "across the corpus"
//! means here is only that identical entries are pooled and COUNTED, and the
//! count is the evidence — one story is one author's idiom (the corpus has a
//! game whose `attack` verb carries thirty-three spellings, `vandalise` and
//! `torture` among them), while a set several stories declare independently is
//! an IF convention. See [`Params::game_support`].
//!
//! ## What a row is
//!
//! One line, one **synonym group**, members tab-separated:
//!
//! ```text
//! light   burn    ignite  illuminate
//! pull    draw    drag    tug
//! ```
//!
//! There is no key column. A word may sit in several groups — one per sense —
//! and that is the point: `light` is *illuminate*, *not heavy* and *a lamp*, and
//! keeping the senses apart is what stops `illuminate` from ever reaching
//! `lightweight`. This is WordNet's own structure, kept rather than flattened.
//!
//! Storing groups rather than an inverted `word → words` map also stores each
//! set once instead of once per member, and `grep illuminate` returns the whole
//! group in a single step.
//!
//! ## LINE ORDER IS SIGNIFICANT — do not sort this file
//!
//! WordNet orders a word's senses commonest-first, and that ordering is a real
//! signal: a player who types `draw` at a game that knows neither `pull` nor
//! `sketch` should be shown `pull` first, because pulling is what `draw` most
//! often means. The file preserves it — the groups containing any given word
//! appear in that word's own sense order — so the consumer can walk a word's
//! groups most-common-sense-first and stop after three or four matches, and a
//! rare fifth sense never crowds out the common first one.
//!
//! A linear file cannot always satisfy every word at once (two words can rank
//! two shared senses in opposite orders), so the order is a topological sort of
//! all the per-word constraints, with any genuine cycle broken deterministically
//! and COUNTED in the run report. Sorting this file alphabetically would destroy
//! the signal silently, which is why the header says so too.
//!
//! ## Exactly one hop, and never a transitive closure
//!
//! A group is ONE synset. The only exception is the gap-fill (below), which
//! unions a synset with its immediate hypernym — one pointer, still bounded,
//! and only where the plain synset could match nothing at all. Nothing is
//! chained further, and groups are never merged with each other. Two hops join
//! two senses through a word that carries both, and the table then makes a
//! confident wrong suggestion with nothing left in it to diagnose the mistake.
//! The next person here will be tempted to raise the depth to improve coverage;
//! that is how this table dies.
//!
//! ## What the table means at lookup
//!
//! Two rules the consumer implements, stated here because they define what the
//! data IS:
//!
//!   * **Intersect the group with THIS story's dictionary** before showing
//!     anything. The table proposes; the story disposes. A group holding
//!     `illuminate` is harmless in a game that has never heard of it — and if
//!     some game does implement `illuminate`, the same table suggests it with no
//!     regeneration, which is why members are not pre-filtered to a snapshot of
//!     the harvest.
//!   * **Drop the word the player actually typed.** It is in the group by
//!     construction and it is the one word known to have failed.
//!
//! ## Base forms, with one deliberate exception
//!
//! An IF parser accepts the imperative: you type `take lamp`, never `took lamp`,
//! and the consumer lemmatises before it looks anything up. Members are
//! therefore WordNet lemmas, which are base forms by construction. The exception
//! is a story whose own dictionary spells a verb in a form that looks inflected
//! and which no other story offers as a base — `seen`, in three Infocom
//! mysteries. That spelling is added to its lemma's groups, because the game's
//! vocabulary is ground truth: a suggestion the parser would reject is
//! worthless.
//!
//! ## The three filters, and what they are made of
//!
//! None of them is a list of words someone thought were good or bad.
//!
//! 1. **Register filter.** A member is kept only if every one of its words is a
//!    12dicts headword in band ≤ [`Params::band_cap`]. That is what removes
//!    `illume`, `enkindle` and `conflagrate` while keeping `illuminate` — a
//!    frequency judgement made by Beale's corpus, not here. A word some story's
//!    parser accepts is exempt: a player demonstrably can type it.
//! 2. **Sense cap.** WordNet orders a lemma's senses commonest-first. A synset
//!    survives only if it is among the first [`Params::sense_cap`] senses of
//!    some IF verb in it, which is how a twentieth-sense fringe meaning of a
//!    common word stays out.
//! 3. **The prune.** A group containing no harvested IF verb at all can never
//!    survive the intersection at lookup, so it is general English costing
//!    bytes for nothing.
//!
//! A game-derived group needs none of the first two — every member is a word
//! some parser accepts, so there is nothing to prune and no sense to cap — but
//! it gets the register filter for a different reason: a dictionary TRUNCATES,
//! and `startl`, `procee` and `walkthrou` are what a game's vocabulary actually
//! holds. Offering a player one of those is worse than offering nothing.

use std::collections::{BTreeMap, BTreeSet};

use crate::sources::{Frequency, WordNet};

/// Every threshold the generation depends on, so a rebuild can be reproduced or
/// argued with from the command line.
#[derive(Debug, Clone)]
pub struct Params {
    /// A synset survives only if it is among this many senses of some IF verb
    /// in it, counting from WordNet's own commonest-first order.
    pub sense_cap: usize,
    /// The highest 12dicts frequency band a member may sit in.
    pub band_cap: u16,
    /// Discard a group with more members than this. Only the gap-fill can
    /// produce one, by unioning a synset with a very general hypernym.
    pub group_cap: usize,
    /// Bands 1..=this define "common English" for the coverage audit.
    pub common_bands: u16,
    /// Refuse to gap-fill through a hypernym with more hyponyms than this: a
    /// synset with two hundred kinds beneath it is an abstraction (`change`,
    /// `move`, `be`), not a synonym, and unioning every one of those hyponyms
    /// with it produces hundreds of groups that all say the same thing.
    pub hyponym_cap: usize,
    /// Union a synset with its immediate hypernym when the synset alone
    /// contains no IF verb. Off, the table is pure synsets.
    pub gap_fill: bool,
    /// How many stories must declare a verb entry before its spellings are
    /// believed as a group. One story is one author's idiom — the corpus has a
    /// game whose `attack` verb carries thirty-three spellings including
    /// `vandalise` and `torture` — and corroboration is the only signal
    /// available that separates an idiom from an IF convention.
    pub game_support: usize,
    /// Discard a game-derived group wider than this, rather than trimming it:
    /// a set too wide to believe is evidence about that one game, and picking
    /// which of its members to keep would be the editorial judgement this
    /// generator exists to avoid.
    ///
    /// Thirteen, and the number is measured rather than chosen. The widest set
    /// the corpus corroborates strongly is `attack break crack destroy fight
    /// hit kill murder punch smash thump torture wreck` — thirteen spellings,
    /// declared verbatim by TWENTY of the 119 stories, which is the Inform
    /// library's own grammar and the single most authoritative group in the
    /// whole corpus. A cap of twelve refuses exactly that one. Above thirteen
    /// the next candidates are single-game sprees: a fourteen-member `walk`
    /// and a thirty-three-member `attack` carrying `vandalise` and `torture`.
    pub game_group_cap: usize,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            sense_cap: 6,
            band_cap: 16,
            group_cap: 12,
            common_bands: 11,
            hyponym_cap: 25,
            gap_fill: true,
            game_support: 2,
            game_group_cap: 13,
        }
    }
}

/// One IF verb as the harvest recorded it.
#[derive(Debug, Clone)]
pub struct IfVerb {
    /// The story's own spelling — what its parser will accept.
    pub emit: String,
    /// The WordNet verb lemma it was looked up under.
    pub lemma: String,
    /// How many stories in the corpus accept it.
    pub stories: usize,
}

/// One synonym group a story's own grammar declares: the spellings one verb
/// entry carries, and how many stories declare exactly that set.
///
/// Read from `if_groups.tsv`, which the harvest writes and which is committed
/// for the same reason `if_verbs.tsv` is — so a rebuild needs no corpus.
#[derive(Debug, Clone)]
pub struct GameGroup {
    pub words: Vec<String>,
    pub stories: usize,
}

/// One group as the generator holds it, before the members are written out.
struct Group {
    members: Vec<String>,
    /// The synset the group came from, or 0 for a game-derived group.
    origin: u32,
    /// The hypernym it was unioned with, for a gap-fill group.
    via: Option<u32>,
    /// True when the group is a story's own verb entry rather than a synset.
    ///
    /// This is what gives the corpus precedence: a game-derived group ranks
    /// ahead of every WordNet group for each of its members (see
    /// [`order_by_sense`]), and it wins an exact-membership tie. Provenance
    /// does not travel into the shipped table — every row there is members and
    /// nothing else — because the consumer has no use for it; the evidence
    /// stays greppable in `if_groups.tsv`, one line per declared set.
    game: bool,
    /// How many stories declare this EXACT verb entry — the `GameGroup`'s own
    /// `stories`, carried onto the `Group` so [`order_by_sense`] and the
    /// subsumption step can weigh one game-derived group against another.
    /// Unused (0) for a WordNet-origin group, whose precedence comes from
    /// WordNet's own sense order instead.
    support: usize,
}

/// The shortest base a `un`-prefixed spelling may derive from — see
/// [`derive_reversals`].
///
/// Below this, the base is a light verb general enough that "reversing" it
/// means nothing: `un` + `do`/`go`/`be` is not a parser action. Three is the
/// shortest English verb that still names something a game DOES to an object
/// (`tie`, `dye`, `arm`), so the guard costs nothing above three-letter verbs
/// and refuses only the placeholders below it.
const MIN_REVERSAL_BASE: usize = 3;

/// Everything the run learned, for the report and the tests.
#[derive(Default)]
pub struct Report {
    /// Synsets that passed the sense cap and left two or more members.
    pub groups_before_prune: usize,
    /// …and how many of those contain an IF verb, so survive the prune.
    pub groups_after_prune: usize,
    /// Groups whose membership another group already had, exactly.
    pub duplicates: usize,
    /// Groups dropped because another group's membership contains theirs.
    pub subsumed: usize,
    /// Groups the gap-fill produced by unioning a synset with its hypernym.
    pub gap_filled: usize,
    /// Distinct verb entries the corpus declares, before any threshold.
    pub game_sets: usize,
    /// …corroborated by at least `game_support` stories.
    pub game_corroborated: usize,
    /// …and kept, after the member filter, the two-member floor and the cap.
    pub game_kept: usize,
    /// Game-derived sets dropped for being wider than `game_group_cap`, worst
    /// first — the evidence for whether the bound is doing anything.
    pub game_oversize: Vec<(usize, Vec<String>)>,
    /// The widest game-derived groups that WERE kept, worst first. If the
    /// corroboration threshold is too loose, this is where it shows.
    pub game_widest: Vec<(usize, Vec<String>)>,
    /// Synsets whose membership a game-derived group already had, exactly —
    /// where the lexicographer and the game authors agree word for word.
    pub game_agrees: usize,
    /// Common verbs (bands 1..=`common_bands`) after lemmatising and dedup.
    pub common_verbs: Vec<String>,
    /// Common verbs that reach a surviving group by plain synonymy.
    pub hits_synonymy: usize,
    /// …and after the gap-fill.
    pub hits_gap_filled: usize,
    /// …and after the game-derived groups: every channel, which is what the
    /// shipped table reaches.
    pub hits_total: usize,
    /// Common verbs that ONLY the game-derived groups reach.
    pub game_only: Vec<String>,
    /// Common verbs that reach nothing.
    pub misses: Vec<String>,
    /// Per-word sense-order constraints the linear file could not satisfy,
    /// because two words rank two shared senses in opposite orders.
    pub order_conflicts: usize,
    /// The words in the most groups, worst first — a word collecting many
    /// groups is highly polysemous, and the cheapest evidence there is that the
    /// filters are too loose.
    pub widest: Vec<(String, usize)>,
    /// Members dropped from a plain-synonymy WordNet group because the group
    /// was not the reason that word counts as an IF verb (its own sense rank
    /// for this synset falls outside `sense_cap`) AND the corpus corroborates
    /// a completely different, disjoint action for it — `clear` beside
    /// `illuminate`'s "clarify" sense being the motivating case. `(dropped
    /// word, a member that stayed)` per removal, for the report and the tests.
    pub bystanders_dropped: Vec<(String, String)>,
    /// `un`-prefixed IF verbs [`derive_reversals`] looked at.
    pub reversal_candidates: usize,
    /// …resolved using the corpus's own (possibly single-story) declaration
    /// for the `un`-spelling itself.
    pub reversals_from_corpus: usize,
    /// …that had no declaration of their own and were paired with their bare
    /// base verb instead, so the spelling is at least resolvable.
    pub reversals_paired_with_base: usize,
}

/// Build the groups.
pub fn build(
    verbs: &[IfVerb],
    games: &[GameGroup],
    wn: &WordNet,
    freq: &Frequency,
    p: &Params,
    report: &mut Report,
) -> Vec<Vec<String>> {
    // The stories' own spellings, indexed by the lemma they were looked up
    // under, so a synset naming `see` also offers the `seen` that three Infocom
    // mysteries insist on.
    let mut by_lemma: BTreeMap<&str, Vec<&IfVerb>> = BTreeMap::new();
    for v in verbs {
        by_lemma.entry(v.lemma.as_str()).or_default().push(v);
    }
    let stories = |w: &str| -> usize {
        by_lemma
            .get(w)
            .map_or(0, |vs| vs.iter().map(|v| v.stories).max().unwrap_or(0))
    };
    let is_if_verb = |w: &str| by_lemma.contains_key(w);

    // Which synsets are inside the sense cap of some IF verb.
    let mut wanted: BTreeSet<u32> = BTreeSet::new();
    for lemma in by_lemma.keys() {
        if let Some(senses) = wn.senses.get(*lemma) {
            wanted.extend(senses.iter().take(p.sense_cap));
        }
    }

    let member_ok = |w: &str| {
        is_if_verb(w)
            || w.split(' ')
                .all(|t| freq.band.get(t).is_some_and(|&b| b <= p.band_cap))
    };

    let common: BTreeSet<String> = common_verbs(wn, freq, p).into_iter().collect();

    let mut groups: Vec<Group> = Vec::new();
    let mut seen_sets: BTreeMap<Vec<String>, usize> = BTreeMap::new();
    let mut in_group: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_synonymy: BTreeSet<String> = BTreeSet::new();
    let mut by_gap: BTreeSet<String> = BTreeSet::new();
    let mut by_game: BTreeSet<String> = BTreeSet::new();

    // ── Pass 0: the corpus's own verb entries ────────────────────────────────
    //
    // FIRST, so that where a game entry and a synset have the same membership
    // the game's copy is the one kept — see `keep`, which discards the second
    // of two identical sets.
    //
    // A member has to be a word a person would recognise, and the filter is a
    // dictionary lookup rather than a judgement: `investiga`, `startl`,
    // `walkthrou` and `procee` are what a dictionary holds after truncating to
    // six or nine characters, and offering a player `procee` would be worse
    // than offering nothing.
    //
    // The test is the register filter's own — WordNet knows it as a verb, or
    // 12dicts ranks its headword within `band_cap` — rather than a new axis of
    // judgement, and unlike pass 1 there is no is-an-IF-verb exemption to
    // soften it: every member here is an IF verb by construction, which is
    // exactly why the exemption would let every truncation through. It keeps
    // `hint`, `nope` and `credits`; it drops `bast` and `boll`, a fibre and a
    // cotton pod, which is what a four-letter verb table happens to spell two
    // ruder words as.
    //
    // What it cannot catch is a truncation that IS a common word: `board`
    // truncated to four characters is `boar`, and no filter made of shape or
    // frequency can tell that from the animal. It survives as a member of the
    // `go` group, and can only ever be offered to a player of a game whose
    // dictionary really does hold `boar`.
    let recognised = |w: &str| {
        wn.senses.contains_key(w)
            || freq
                .lemma_of
                .get(w)
                .and_then(|h| freq.band.get(h))
                .is_some_and(|&b| b <= p.band_cap)
    };

    // …except that the corpus can explain most of its own truncations, and a
    // group is where that is worth doing: `examin`, `inspec`, `procee` and
    // `scrutiniz` are not words, but each is the unambiguous prefix of a word
    // that OTHER stories in this corpus spell in full, and the truncating
    // parser accepts the full spelling too — it truncates the player's input by
    // the same rule.
    //
    // Two bounds keep this from guessing. The expansion must be a spelling the
    // CORPUS attests, never one invented out of a dictionary, so a member is
    // always a word some real parser takes; and it must be unique, so `brea`
    // (break / breath / breathe) and `atta` (attach / attack) are left alone.
    //
    // And it applies at SIX or NINE characters only, which is where a
    // Z-machine dictionary truncates (v1–v3 and v4–v8 respectively). That is
    // not a tuning knob but the shape of the defect: Scott Adams tables
    // truncate at their own word length, three to five characters, and a prefix
    // that short does not identify a word. Measured — allowed down to three,
    // the unique expansion of `arr`, in a group that is plainly a pirate's
    // growl, is `arrest`.
    let spellings: Vec<&str> = verbs
        .iter()
        .map(|v| v.emit.as_str())
        .filter(|s| !s.contains(' '))
        .collect();
    let unique_prefix = |w: &str, of: &mut dyn Iterator<Item = &str>| -> Option<String> {
        let mut found: Option<String> = None;
        for s in of {
            if s.len() > w.len() && s.starts_with(w) {
                if found.is_some() {
                    return None;
                }
                found = Some(s.to_string());
            }
        }
        found
    };
    let untruncate = |w: &str| -> Option<String> {
        if w.len() != 6 && w.len() != 9 {
            return None;
        }
        if let Some(attested) =
            unique_prefix(w, &mut spellings.iter().copied().filter(|s| recognised(s)))
        {
            return Some(attested);
        }
        // At NINE characters the true word has at least ten letters, which no
        // Z-machine dictionary can hold — so no story can attest it and the
        // rule above must come up empty for exactly the words most worth
        // recovering: `extinguis`, `interroga`, `deactivat`, `incinerat`. A
        // nine-character prefix is long enough to name one verb, so WordNet is
        // allowed to finish the word where the corpus cannot, and only there.
        // Uniqueness is over VERB lemmas: `extinguisher` and `interrogation`
        // share the prefix and are not verbs, and asking the whole dictionary
        // would refuse every one of these.
        if w.len() == 9 {
            return unique_prefix(w, &mut wn.senses.keys().map(String::as_str));
        }
        None
    };

    // Every RAW verb entry the corpus declares, CANONICALISED the same way
    // Pass 0 canonicalises a kept group's members (`recognised`/`untruncate`)
    // and indexed by the spellings it contains — used only as a
    // member-ORDERING signal (see the "Order the members" step below), never
    // to admit a group: no threshold applied here, because a single-story
    // declaration is still real evidence of which spelling a game's author
    // reached for first when several games agree on the action but not on the
    // ranking.
    //
    // Canonicalising here (not just indexing the raw spellings) matters:
    // `examine` truncates to `examin` in several Z-machine dictionaries, and
    // without this step every one of those entries indexed under the
    // truncated spelling instead — undercounting `examine`'s true corpus
    // support relative to `watch`, which is short enough never to truncate,
    // and flipping the very ranking this signal exists to fix.
    let games_canonical: Vec<(Vec<String>, usize)> = games
        .iter()
        .map(|g| {
            let words: Vec<String> = g
                .words
                .iter()
                .filter_map(|w| if recognised(w) { Some(w.clone()) } else { untruncate(w) })
                .collect();
            (words, g.stories)
        })
        .collect();
    let mut games_by_word: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, (words, _)) in games_canonical.iter().enumerate() {
        for w in words {
            games_by_word.entry(w.as_str()).or_default().push(i);
        }
    }

    report.game_sets = games.len();
    for g in games {
        if g.stories < p.game_support {
            continue;
        }
        report.game_corroborated += 1;
        let mut members: Vec<String> = g
            .words
            .iter()
            .filter_map(|w| {
                if recognised(w) {
                    Some(w.clone())
                } else {
                    untruncate(w)
                }
            })
            .collect();
        members.sort();
        members.dedup();
        if members.len() < 2 {
            continue;
        }
        if members.len() > p.game_group_cap {
            report.game_oversize.push((g.stories, members));
            continue;
        }
        for w in &members {
            by_game.insert(w.clone());
        }
        let widest = (g.stories, members.clone());
        if keep(
            Group {
                members,
                origin: 0,
                via: None,
                game: true,
                support: g.stories,
            },
            &mut groups,
            &mut seen_sets,
            &mut in_group,
            report,
        ) {
            report.game_kept += 1;
            report.game_widest.push(widest);
        }
    }
    report
        .game_widest
        .sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(b.0.cmp(&a.0)));
    report.game_widest.truncate(12);
    report
        .game_oversize
        .sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(b.0.cmp(&a.0)));

    // Every spelling a KEPT (corroborated) game group carries, mapped to the
    // OTHER members of every such group it sits in. Built once, right after
    // Pass 0, and consulted by Pass 1's bystander filter below: it is the
    // corpus's own answer to "does this word already mean something else".
    let mut game_sense_index: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
    for g in groups.iter().filter(|g| g.game) {
        let set: BTreeSet<String> = g.members.iter().cloned().collect();
        for w in &g.members {
            let rest: BTreeSet<String> = set.iter().filter(|x| *x != w).cloned().collect();
            game_sense_index.entry(w.clone()).or_default().push(rest);
        }
    }

    // ── Pass 1: one group per synset ─────────────────────────────────────────
    for (&offset, syn) in &wn.synsets {
        if !wanted.contains(&offset) {
            continue;
        }
        let mut members = assemble(&syn.words, &by_lemma, &member_ok, true);
        if members.len() < 2 {
            continue;
        }
        // Drop a BYSTANDER member: a word whose own WordNet sense rank did not
        // select this synset (some OTHER member's sense list did) and for
        // which the corpus corroborates a completely different action, none
        // of whose declared entries so much as touch this synset's other
        // members. `clear`'s dominant IF sense is "move/push aside" (24
        // stories), never "make plain" — it rides into the illuminate-adjacent
        // "clarify" synset only because `clear up`'s OWN sense list reaches it,
        // and every one of the corpus's `clear` entries is disjoint from that
        // synset's other members. `light` is never touched by this: its own
        // sense list puts the illuminate synset FIRST, so it is never a
        // bystander there regardless of what else the corpus also says about
        // `light` (burning a candle is a related, corroborated, OVERLAPPING
        // reading, not a disjoint one). See `Report::bystanders_dropped`.
        let snapshot = members.clone();
        let mut bystanders: BTreeSet<String> = BTreeSet::new();
        for w in &snapshot {
            let rank = wn.senses.get(w.as_str()).and_then(|s| s.iter().position(|o| *o == offset));
            if rank.is_some_and(|r| r < p.sense_cap) {
                continue; // this synset IS w's own (primary) sense — never a bystander.
            }
            let Some(entries) = game_sense_index.get(w.as_str()) else {
                continue; // no corpus opinion about w at all — nothing to disagree with.
            };
            let rest: BTreeSet<&str> =
                snapshot.iter().filter(|x| *x != w).map(String::as_str).collect();
            let all_disjoint = !entries.is_empty()
                && entries.iter().all(|e| e.iter().all(|m| !rest.contains(m.as_str())));
            if all_disjoint {
                report.bystanders_dropped.push((
                    w.clone(),
                    snapshot.iter().find(|x| *x != w).cloned().unwrap_or_default(),
                ));
                bystanders.insert(w.clone());
            }
        }
        members.retain(|w| !bystanders.contains(w));
        if members.len() < 2 {
            continue;
        }
        report.groups_before_prune += 1;
        if !members.iter().any(|w| is_if_verb(w)) {
            continue;
        }
        report.groups_after_prune += 1;
        for w in &members {
            by_synonymy.insert(w.clone());
        }
        keep(
            Group {
                members,
                origin: offset,
                via: None,
                game: false,
                support: 0,
            },
            &mut groups,
            &mut seen_sets,
            &mut in_group,
            report,
        );
    }

    // ── Pass 2: the gap-fill ─────────────────────────────────────────────────
    //
    // A synset no story can match is dead weight — unless its immediate
    // hypernym CAN be matched, in which case the player's specific word and the
    // general verb the story knows belong together: `sprint` is a kind of `run`.
    // One pointer, only where plain synonymy reached nothing, and never
    // extended: hyponyms are not walked either, because a general word would
    // then drag in every specific verb beneath it (`move` → push, pull, turn,
    // slide, …), which is the over-inclusion this whole design exists to avoid.
    if p.gap_fill {
        for (&offset, syn) in &wn.synsets {
            let members = assemble(&syn.words, &by_lemma, &member_ok, false);
            // ONLY a synset no story can match, and this is the load-bearing
            // condition, not a coverage heuristic. A group is symmetric while
            // hypernymy is not: `sprint` is a kind of `run`, but `run` is not a
            // kind of `sprint`, and a group holding both would suggest `sprint`
            // to a player who typed `run`. Requiring that the CHILD synset
            // contains no IF verb makes that impossible by construction — none
            // of the specific words is in any story's dictionary, so the
            // intersection at lookup can never surface one. Relaxing this to
            // "any synset" was measured: it reached 92.0% instead of 88.8% and
            // put `fish`, `hook` and `net` in a group with `grab`, every one of
            // them a wrong suggestion waiting for a game that implements it.
            if members.iter().any(|w| is_if_verb(w)) {
                continue;
            }
            // And only rescue a synset a PLAYER might reach for. Gap-filling
            // every dead synset in WordNet produces thousands of groups nobody
            // will ever type a member of.
            if !members.iter().any(|w| common.contains(w)) {
                continue;
            }
            for (sym, target) in &syn.pointers {
                if sym != "@" && sym != "@i" {
                    continue;
                }
                let Some(up) = wn.synsets.get(target) else {
                    continue;
                };
                if !wanted.contains(target) {
                    continue;
                }
                if up.pointers.iter().filter(|(s, _)| s == "~").count() > p.hyponym_cap {
                    continue;
                }
                let mut union: Vec<String> = syn.words.clone();
                union.extend(up.words.iter().cloned());
                let union = assemble(&union, &by_lemma, &member_ok, false);
                if union.len() < 2
                    || union.len() > p.group_cap
                    || !union.iter().any(|w| is_if_verb(w))
                {
                    continue;
                }
                report.gap_filled += 1;
                for w in &union {
                    by_gap.insert(w.clone());
                }
                keep(
                    Group {
                        members: union,
                        origin: offset,
                        via: Some(*target),
                        game: false,
                        support: 0,
                    },
                    &mut groups,
                    &mut seen_sets,
                    &mut in_group,
                    report,
                );
            }
        }
    }

    // ── Drop groups another group already contains ───────────────────────────
    //
    // The gap-fill makes families of near-identical unions — a dozen sibling
    // synsets all unioned with the same hypernym. A group whose members are a
    // subset of another group's says nothing the larger one does not, and
    // costs a line. This is not merging: no group gains a member.
    //
    // WITHIN one provenance only. A game-derived group swallowed by a wider
    // synset would lose the precedence the corpus earned it — the whole point
    // of reading the grouping — and a synset swallowed by a game group would
    // lose its position in its other members' sense order, which no larger
    // group can restore. Each source's own redundancy is collapsed on its own
    // terms; the overlap between the two is left standing and counted.
    //
    // For two GAME groups the size test alone is not enough: a wider set is
    // only a strict improvement on a narrower one if it is at least as
    // BELIEVED. `press/push/shove` (5 stories) is a strict subset of
    // `nudge/press/push/shove/stick/thrust` (3 stories) and the size test alone
    // would let the six-member, weaker-evidence set eat the three-member,
    // stronger-evidence one — discarding the very corroboration [`Params::game_support`]
    // exists to weigh. So a game group is swallowed only by a game superset
    // whose OWN support is at least as high; ties still prefer the wider set.
    // WordNet subsumption (`support` unused there, always 0) is unchanged.
    {
        let sets: Vec<BTreeSet<&str>> = groups
            .iter()
            .map(|g| g.members.iter().map(String::as_str).collect())
            .collect();
        let mut drop = vec![false; groups.len()];
        let mut by_member: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, s) in sets.iter().enumerate() {
            for w in s {
                by_member.entry(w).or_default().push(i);
            }
        }
        for (i, s) in sets.iter().enumerate() {
            let Some(anchor) = s.iter().next() else {
                continue;
            };
            for &j in &by_member[*anchor] {
                if i != j
                    && !drop[j]
                    && groups[i].game == groups[j].game
                    && (sets[j].len() > s.len() || (sets[j].len() == s.len() && j < i))
                    && (!groups[i].game || groups[j].support >= groups[i].support)
                    && s.is_subset(&sets[j])
                {
                    drop[i] = true;
                    report.subsumed += 1;
                    break;
                }
            }
        }
        let mut i = 0;
        groups.retain(|_| {
            i += 1;
            !drop[i - 1]
        });
    }

    derive_reversals(verbs, games, wn, &is_if_verb, &recognised, &untruncate, p, &mut groups, &mut seen_sets, &mut in_group, report);

    // ── Order the members ────────────────────────────────────────────────────
    //
    // Verbs the corpus actually uses first, commonest first, so the leading
    // members of a line are the likeliest suggestions and the file diffs
    // stably.
    //
    // "Commonest" is not one number. `cluster_support` sums the RAW (any
    // support level) if_groups.tsv declarations that name this spelling
    // ALONGSIDE another member of the SAME group — how much of the corpus's
    // evidence for this particular action backs this particular spelling —
    // and is tried first. It is what ranks `examine` (named in every one of
    // the corpus's own "look closely" entries) ahead of `watch` (named in
    // fewer of them, despite being IF's more common verb overall, across
    // senses this group is not one of). It is 0 for a WordNet-only group,
    // where no if_groups entry ever names members like `light up` or
    // `illuminate` at all — so it falls straight through to the old
    // overall-popularity tiebreak, unchanged for every group this fix does
    // not touch.
    for g in &mut groups {
        let snapshot: Vec<String> = g.members.clone();
        let cluster_support = |w: &str| -> usize {
            let Some(idxs) = games_by_word.get(w) else {
                return 0;
            };
            let rest: BTreeSet<&str> =
                snapshot.iter().filter(|x| x.as_str() != w).map(String::as_str).collect();
            idxs.iter()
                .filter(|&&i| games_canonical[i].0.iter().any(|m| rest.contains(m.as_str())))
                .map(|&i| games_canonical[i].1)
                .sum()
        };
        g.members.sort_by(|a, b| {
            is_if_verb(b)
                .cmp(&is_if_verb(a))
                .then(cluster_support(b).cmp(&cluster_support(a)))
                .then(stories(b).cmp(&stories(a)))
                .then(a.cmp(b))
        });
    }

    let groups = order_by_sense(groups, wn, report);
    audit(
        &groups,
        &by_synonymy,
        &by_gap,
        &by_game,
        wn,
        freq,
        p,
        report,
    );
    report.widest.extend(
        in_group
            .iter()
            .map(|(w, n)| (w.clone(), *n))
            .filter(|(_, n)| *n > 1),
    );
    report
        .widest
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    report.widest.truncate(15);
    groups
}

/// Filter a synset's words to those that may appear as members, and add the
/// stories' own spellings for any lemma among them.
fn assemble(
    words: &[String],
    by_lemma: &BTreeMap<&str, Vec<&IfVerb>>,
    member_ok: &impl Fn(&str) -> bool,
    story_spellings: bool,
) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for w in words {
        if !member_ok(w) {
            continue;
        }
        out.insert(w.clone());
        if story_spellings {
            for v in by_lemma.get(w.as_str()).map_or(&[][..], Vec::as_slice) {
                out.insert(v.emit.clone());
            }
        }
    }
    out.into_iter().collect()
}

/// Record a group unless an identical one is already there.
///
/// The game-derived pass runs first, so an identical synset arriving later is
/// the one discarded — which is both sources agreeing word for word, counted
/// separately as [`Report::game_agrees`] rather than as noise.
///
/// `seen` maps a member set to its INDEX in `groups`, not merely to whether it
/// was a game group, so that two game-derived entries which happen to reduce
/// to the identical final member set (after `member_ok`/untruncation) do not
/// silently keep whichever one `if_groups.tsv`'s alphabetical order happened
/// to reach first. `describe examine inspect observe study watch` is declared
/// verbatim by 3 stories AND — after a different truncation collapses to the
/// same set — by 2 more elsewhere in the file; without this, `keep` recorded
/// whichever line sorted first and both [`order_by_sense`] and the
/// subsumption step above reasoned from the WRONG (lower) support number for
/// a set the corpus actually corroborates more strongly (SQ-1233). Nothing is
/// unioned: both declarations name the SAME members, so recording the higher
/// support is choosing the stronger of two identical statements, not merging
/// two different ones.
fn keep(
    g: Group,
    groups: &mut Vec<Group>,
    seen: &mut BTreeMap<Vec<String>, usize>,
    in_group: &mut BTreeMap<String, usize>,
    report: &mut Report,
) -> bool {
    if let Some(&idx) = seen.get(&g.members) {
        report.duplicates += 1;
        let was_game = groups[idx].game;
        if was_game && !g.game {
            report.game_agrees += 1;
        }
        if g.game && was_game && g.support > groups[idx].support {
            groups[idx].support = g.support;
        }
        return false;
    }
    let idx = groups.len();
    seen.insert(g.members.clone(), idx);
    for w in &g.members {
        *in_group.entry(w.clone()).or_default() += 1;
    }
    groups.push(g);
    true
}

/// Pass 3 — `un`-prefixed spellings reach at least one group.
///
/// Neither earlier pass has a channel for this. WordNet has no synset relating
/// an English verb to its `un`-form — that is a productive morphological rule,
/// not a lexical fact, so no amount of sense-cap tuning will ever surface one —
/// and a rare spelling like `unmask` or `unzip` is exactly the kind of word one
/// or two stories declare, which [`Params::game_support`] exists to distrust.
/// But the `un`-morphology is itself independent evidence: nobody accidentally
/// spells a game's own dictionary word `unpin`, so a single declaration is
/// enough HERE where it would not be enough for an unrelated pair of spellings.
///
/// Only spellings that reach NO group at all after every earlier pass are
/// considered (`in_group` is checked, not reasoned about) — a well-corroborated
/// `unhook`/`untie`/`unfasten` cluster that Pass 0 already built normally is
/// left exactly as it is; this pass exists only for the words that pipeline
/// never reaches, and it is not run against anything else.
///
/// Two tiers, tried in order, per `unX`:
///
///   1. The corpus's OWN raw declaration for `unX` (`if_groups.tsv`, ANY
///      support level, not gated by `game_support`) — `unpin` reaches
///      `unblock`/`uncover`/`unplug` this way, the reversal cluster a game
///      author actually wrote, at one story.
///   2. No declaration at all: pair `unX` with its bare base verb (`unmask`
///      with `mask`) so the spelling is at least resolvable, per the module
///      docs' "at lookup" rule — a suggestion nobody can act on is worthless,
///      and a two-member group naming the base action is not nobody.
fn derive_reversals(
    verbs: &[IfVerb],
    games: &[GameGroup],
    wn: &WordNet,
    is_if_verb: &impl Fn(&str) -> bool,
    recognised: &impl Fn(&str) -> bool,
    untruncate: &impl Fn(&str) -> Option<String>,
    p: &Params,
    groups: &mut Vec<Group>,
    seen: &mut BTreeMap<Vec<String>, usize>,
    in_group: &mut BTreeMap<String, usize>,
    report: &mut Report,
) {
    let mut candidates: Vec<&str> =
        verbs.iter().map(|v| v.emit.as_str()).filter(|w| !w.contains(' ')).collect();
    candidates.sort_unstable();
    candidates.dedup();
    for w in candidates {
        let Some(base) = w.strip_prefix("un") else { continue };
        if base.chars().count() < MIN_REVERSAL_BASE {
            continue;
        }
        if !(is_if_verb(base) || wn.senses.contains_key(base)) {
            continue;
        }
        if in_group.contains_key(w) {
            continue; // already reachable through an earlier pass.
        }
        report.reversal_candidates += 1;

        let mut resolved = false;
        for g in games.iter().filter(|g| g.words.iter().any(|m| m == w)) {
            let mut members: Vec<String> = g
                .words
                .iter()
                .filter_map(|m| if recognised(m) { Some(m.clone()) } else { untruncate(m) })
                .collect();
            members.sort();
            members.dedup();
            if members.len() < 2 || members.len() > p.game_group_cap {
                continue;
            }
            if keep(
                Group { members, origin: 0, via: None, game: true, support: g.stories },
                groups,
                seen,
                in_group,
                report,
            ) {
                report.reversals_from_corpus += 1;
                resolved = true;
            }
        }
        if !resolved && !in_group.contains_key(w) {
            let stories = verbs.iter().find(|v| v.emit == w).map_or(1, |v| v.stories);
            let mut members = vec![base.to_string(), w.to_string()];
            members.sort();
            if keep(
                Group { members, origin: 0, via: None, game: true, support: stories },
                groups,
                seen,
                in_group,
                report,
            ) {
                report.reversals_paired_with_base += 1;
            }
        }
    }
}

/// Put the groups in an order that, for every word, presents that word's groups
/// in WordNet's own sense order — commonest sense first.
///
/// Each word contributes a chain of "this group before that one" constraints
/// over the groups it belongs to. Satisfying all of them at once is a
/// topological sort; two words CAN rank two shared senses in opposite orders,
/// which makes a cycle, so the sort is Kahn's algorithm with an alphabetical
/// tie-break and any surviving cycle broken by taking the alphabetically first
/// remaining group. Every constraint broken that way is counted, because a large
/// number would mean the ordering claim in the file header is not worth much.
fn order_by_sense(groups: Vec<Group>, wn: &WordNet, report: &mut Report) -> Vec<Vec<String>> {
    // For each word, its groups in that word's sense order — with every
    // game-derived group ahead of every synset.
    //
    // THIS IS THE PRECEDENCE RULE, and it is the whole of it. Where the two
    // sources disagree about what a word means, the corpus wins, because a
    // game author writing `Verb 'examine' 'x' 'inspect'` has stated what the
    // word means IN A PARSER, which is the only question this table asks.
    // WordNet's answer is not deleted — it sits below, and reaches a story
    // that implements a word no game in the corpus grouped.
    // The tuple is `(rank, tie, group index)`. `rank` alone gives every game
    // group 0 and every synset >=1 — the precedence rule above — but that
    // leaves every game group for a word tied with every other, which is
    // resolved arbitrarily (see SQ-1233: `press/push/shove`, the 5-story
    // set, was landing BEHIND `nudge/press/push/shove/stick/thrust`, the
    // 3-story one). `tie` breaks that tie by support, descending, and is 0
    // for a synset (whose own `rank` already orders it, and whose `support`
    // is always 0 regardless), so nothing about WordNet ordering changes.
    let mut by_word: BTreeMap<&str, Vec<(usize, usize, usize)>> = BTreeMap::new();
    for (i, g) in groups.iter().enumerate() {
        for w in &g.members {
            let senses = wn.senses.get(w.as_str()).map_or(&[][..], Vec::as_slice);
            let rank = if g.game {
                0
            } else {
                senses
                    .iter()
                    .position(|o| *o == g.origin)
                    .or_else(|| g.via.and_then(|v| senses.iter().position(|o| *o == v)))
                    .map_or(usize::MAX, |r| r + 1)
            };
            let tie = if g.game { usize::MAX - g.support } else { 0 };
            by_word.entry(w.as_str()).or_default().push((rank, tie, i));
        }
    }

    let n = groups.len();
    let mut edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    for chain in by_word.values_mut() {
        chain.sort();
        for pair in chain.windows(2) {
            if pair[0].2 != pair[1].2 {
                edges.insert((pair[0].2, pair[1].2));
            }
        }
    }

    let mut indegree = vec![0usize; n];
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in &edges {
        out[a].push(b);
        indegree[b] += 1;
    }
    // Deterministic tie-break: the group's first member, then its offset.
    let mut key: Vec<(&str, usize)> = Vec::with_capacity(n);
    for g in &groups {
        key.push((
            g.members.first().map_or("", String::as_str),
            g.origin as usize,
        ));
    }
    let mut ready: BTreeSet<(&str, usize, usize)> = (0..n)
        .filter(|&i| indegree[i] == 0)
        .map(|i| (key[i].0, key[i].1, i))
        .collect();
    let mut remaining: BTreeSet<(&str, usize, usize)> = (0..n)
        .filter(|&i| indegree[i] != 0)
        .map(|i| (key[i].0, key[i].1, i))
        .collect();
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let next = match ready.iter().next().copied() {
            Some(x) => {
                ready.remove(&x);
                x
            }
            None => {
                // A cycle: two words disagree about which sense comes first.
                let x = *remaining.iter().next().expect("nodes remain");
                remaining.remove(&x);
                report.order_conflicts += indegree[x.2];
                indegree[x.2] = 0;
                x
            }
        };
        order.push(next.2);
        for &b in &out[next.2] {
            // Saturating because breaking a cycle zeroes a node's indegree
            // while its predecessors still hold edges to it.
            indegree[b] = indegree[b].saturating_sub(1);
            if indegree[b] == 0 && remaining.remove(&(key[b].0, key[b].1, b)) {
                ready.insert((key[b].0, key[b].1, b));
            }
        }
    }

    let mut members: Vec<Option<Vec<String>>> =
        groups.into_iter().map(|g| Some(g.members)).collect();
    order
        .into_iter()
        .map(|i| members[i].take().expect("each group once"))
        .collect()
}

/// How much of the commonest English verb vocabulary the table reaches.
///
/// This is the quality metric for the whole exercise: a table that misses the
/// words players actually reach for is not useful however many rows it has.
fn audit(
    groups: &[Vec<String>],
    by_synonymy: &BTreeSet<String>,
    by_gap: &BTreeSet<String>,
    by_game: &BTreeSet<String>,
    wn: &WordNet,
    freq: &Frequency,
    p: &Params,
    report: &mut Report,
) {
    let all: BTreeSet<&str> = groups.iter().flatten().map(String::as_str).collect();
    let common = common_verbs(wn, freq, p);
    report.hits_synonymy = common
        .iter()
        .filter(|w| by_synonymy.contains(*w) && all.contains(w.as_str()))
        .count();
    // The three channels are cumulative on the SAME basis, so every number is
    // comparable with the run before the corpus grouping existed: plain
    // synonymy, then what the gap-fill adds, then what the games add.
    report.hits_gap_filled = common
        .iter()
        .filter(|w| (by_synonymy.contains(*w) || by_gap.contains(*w)) && all.contains(w.as_str()))
        .count();
    report.game_only = common
        .iter()
        .filter(|w| {
            by_game.contains(*w)
                && !by_synonymy.contains(*w)
                && !by_gap.contains(*w)
                && all.contains(w.as_str())
        })
        .cloned()
        .collect();
    report.hits_total = common.iter().filter(|w| all.contains(w.as_str())).count();
    report.misses = common
        .iter()
        .filter(|w| !all.contains(w.as_str()))
        .cloned()
        .collect();
    report.common_verbs = common;
}

/// The commonest English verbs: 12dicts bands 1..=`common_bands`, lemmatised,
/// deduplicated, and reduced to those WordNet knows as verbs.
///
/// Lemmatising BEFORE the dedup is what makes the count honest — `go`, `going`
/// and `went` are three entries for one verb, and counting them separately gets
/// the hit rate wrong in both directions.
fn common_verbs(wn: &WordNet, freq: &Frequency, p: &Params) -> Vec<String> {
    let mut common = Vec::new();
    let mut seen = BTreeSet::new();
    for w in freq.top(p.common_bands) {
        let lemma = freq.lemma_of.get(w).map_or(w, String::as_str);
        let lemma = if wn.senses.contains_key(lemma) {
            lemma.to_string()
        } else if let Some(b) = wn
            .exceptions
            .get(lemma)
            .and_then(|b| b.first())
            .filter(|b| wn.senses.contains_key(*b))
        {
            b.clone()
        } else {
            continue; // not a verb at all: a noun, adjective or function word.
        };
        if seen.insert(lemma.clone()) {
            common.push(lemma);
        }
    }
    common
}
