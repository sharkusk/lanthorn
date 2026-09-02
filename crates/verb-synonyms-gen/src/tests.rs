//! Tests for the generator's readers and for the committed harvest.
//!
//! The shipped table's own guarantees are tested in `verb-synonyms`; what is
//! tested here is that the two source formats are read the way their
//! documentation says, using fixtures small enough to check by eye against the
//! real files.

use crate::build::{build, GameGroup, IfVerb, Params, Report};
use crate::sources::{Frequency, WordNet};

/// A directory this CALL alone owns.
///
/// Keyed on a counter as well as the pid, and that is the whole point: under
/// `cargo nextest run` every test is its own process, so a pid alone is already
/// unique and the bug below cannot happen. Under `cargo test` — which is what CI
/// runs — one binary's tests share a process and run on threads, so a pid-only
/// key gave every caller of [`wordnet_fixture`] the SAME directory. `fs::write`
/// truncates, so one thread read `index.verb` while another was rewriting it,
/// `WordNet::load` came back empty, `build` returned no groups, and the
/// assertion failed on a fixture that was correct.
///
/// Invisible to the local gate by construction — nextest's process-per-test
/// makes a shared-state race structurally unobservable, and only `cargo test`
/// can see it.
fn scratch(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let nth = NTH.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir()
        .join(format!("verbsyn-test-{name}-{}-{nth}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// Three real lines from `index.verb`, `data.verb` and `verb.exc`, transcribed
/// byte for byte — the `light` synset that carries `illuminate`, plus the
/// `ignite` one, plus one hypernym for the pointer walk.
fn wordnet_fixture() -> WordNet {
    let d = scratch("wn");
    std::fs::write(
        d.join("index.verb"),
        "  1 This software and database is being provided to you\n\
         light v 2 1 @ 2 1 00291873 02759614  \n\
         illuminate v 1 1 @ 1 1 00291873  \n\
         ignite v 1 1 @ 1 0 02759614  \n\
         sprint v 1 1 @ 1 0 02058590  \n\
         run v 1 1 @ 1 1 02091410  \n",
    )
    .unwrap();
    std::fs::write(
        d.join("data.verb"),
        "  1 This software and database is being provided to you\n\
         00291873 30 v 05 light 0 illume 0 illumine 0 light_up 0 illuminate 3 001 @ 00280930 v 0000 | make lighter\n\
         02759614 43 v 02 ignite 0 light 0 001 @ 02762468 v 0000 | cause to start burning\n\
         02058590 38 v 01 sprint 0 001 @ 02091410 v 0000 | run very fast\n\
         02091410 38 v 02 run 0 hurry 0 000 | move fast\n",
    )
    .unwrap();
    std::fs::write(d.join("verb.exc"), "lit light\nran run\nsaw see\nsinging sing singe\n")
        .unwrap();
    std::fs::write(d.join("noun.exc"), "mice mouse\naxes ax axis\nis is\n").unwrap();
    WordNet::load(&d).expect("fixture loads")
}

fn frequency_fixture() -> Frequency {
    let d = scratch("freq");
    let p = d.join("frq.txt");
    std::fs::write(
        &p,
        "----- 1 -----\nup\nlight\n    lighted, lit\nrun\n    ran\n\
         ----- 2 -----\nignite\nsprint\nhurry\n\
         ----- 9 -----\nilluminate\n(April)\nfoo*\n",
    )
    .unwrap();
    Frequency::load(&p).expect("fixture loads")
}

#[test]
fn wordnet_synsets_carry_their_words_and_verb_pointers() {
    let wn = wordnet_fixture();
    assert_eq!(wn.senses["light"], vec![291873, 2759614]);
    assert_eq!(
        wn.words_of(291873),
        ["light", "illume", "illumine", "light up", "illuminate"],
        "underscores must become spaces and the licence header must be skipped"
    );
    assert_eq!(wn.synsets[&2058590].pointers, [("@".to_string(), 2091410)]);
    assert_eq!(wn.exceptions["lit"], ["light"]);
}

/// The two exception lists are read the same way and kept APART.
///
/// Apart because `main.rs`'s inflected-IF-verb measurement counts `exceptions`
/// and would start meaning something else if nouns joined it, and because a
/// spelling can inflect two parts of speech to different lemmas — one map would
/// answer whichever file was read second.
#[test]
fn the_noun_and_verb_exception_lists_stay_apart() {
    let wn = wordnet_fixture();
    assert_eq!(wn.noun_exceptions["mice"], ["mouse"]);
    assert!(!wn.exceptions.contains_key("mice"), "a noun is not in the verb map");
    assert_eq!(
        wn.exceptions["singing"],
        ["sing", "singe"],
        "WordNet puts two bases on some lines and neither may be dropped"
    );
    assert_eq!(wn.noun_exceptions["axes"], ["ax", "axis"]);
    assert_eq!(
        wn.noun_exceptions["is"],
        ["is"],
        "WordNet's lines are kept verbatim — `is is` is it saying `is` inflects nothing, \
         and it is the TABLE WRITER that drops a self-pair, not the reader"
    );
}

/// `noun.exc` is optional: the DB-only WordNet tarball carries no exception
/// lists at all, and a `dict/` without this one still loads for everything else.
#[test]
fn a_dict_without_noun_exc_still_loads() {
    let d = scratch("wn-no-noun");
    std::fs::write(d.join("index.verb"), "light v 1 1 @ 1 1 00291873  \n").unwrap();
    std::fs::write(d.join("data.verb"), "00291873 30 v 01 light 0 000 | make lighter\n").unwrap();
    std::fs::write(d.join("verb.exc"), "lit light\n").unwrap();
    let wn = WordNet::load(&d).expect("a dict with no noun.exc still loads");
    assert_eq!(wn.exceptions["lit"], ["light"]);
    assert!(wn.noun_exceptions.is_empty());
}

#[test]
fn frequency_bands_and_lemmatisation() {
    let f = frequency_fixture();
    assert_eq!(f.band["light"], 1);
    assert_eq!(f.band["illuminate"], 9);
    assert_eq!(
        f.lemma_of["lit"], "light",
        "indented forms belong to the headword above"
    );
    assert_eq!(f.lemma_of["light"], "light", "a headword is its own lemma");
    assert!(
        !f.band.contains_key("April"),
        "parenthesised entries are not words"
    );
    assert_eq!(
        f.top(1),
        ["up", "light", "run"],
        "band order, not alphabetical"
    );
}

#[test]
fn a_synset_becomes_a_group_and_the_rare_words_are_filtered_out() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    let verbs = vec![IfVerb {
        emit: "light".into(),
        lemma: "light".into(),
        stories: 100,
    }];
    let p = Params {
        band_cap: 9,
        ..Params::default()
    };
    let mut r = Report::default();
    let groups = build(&verbs, &[], &wn, &freq, &p, &mut r);
    let light = groups
        .iter()
        .find(|g| g.contains(&"illuminate".to_string()))
        .expect("a group");
    assert_eq!(light[0], "light", "the IF verb leads the line");
    assert!(light.contains(&"light up".to_string()));
    assert!(
        !light.contains(&"illume".to_string()) && !light.contains(&"illumine".to_string()),
        "`illume` is not in the frequency list and must be filtered out: {light:?}"
    );
}

#[test]
fn the_gap_fill_only_rescues_a_synset_no_story_can_match() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    // `run` is the IF verb; `sprint` sits alone in a synset of its own, whose
    // hypernym is `run`.
    let verbs = vec![IfVerb {
        emit: "run".into(),
        lemma: "run".into(),
        stories: 100,
    }];
    let p = Params {
        band_cap: 9,
        ..Params::default()
    };
    let mut r = Report::default();
    let groups = build(&verbs, &[], &wn, &freq, &p, &mut r);
    assert!(
        groups
            .iter()
            .any(|g| g.contains(&"sprint".to_string()) && g.contains(&"run".to_string())),
        "sprint should reach run through its hypernym: {groups:?}"
    );
    let mut off = Params {
        band_cap: 9,
        gap_fill: false,
        ..Params::default()
    };
    off.gap_fill = false;
    let mut r2 = Report::default();
    let plain = build(&verbs, &[], &wn, &freq, &off, &mut r2);
    assert!(
        !plain.iter().any(|g| g.contains(&"sprint".to_string())),
        "with --no-gap-fill the table is pure synsets"
    );
}

#[test]
fn a_corroborated_verb_entry_becomes_a_group_and_outranks_the_synset() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    let verbs = vec![IfVerb {
        emit: "light".into(),
        lemma: "light".into(),
        stories: 100,
    }];
    // Two stories declare `light` and `ignite` to be one verb — which WordNet
    // also happens to say, in a synset `light` reaches only as its SECOND
    // sense. The game-derived group must come first all the same.
    let games = vec![GameGroup {
        words: vec!["ignite".into(), "light".into()],
        stories: 2,
    }];
    let p = Params {
        band_cap: 9,
        ..Params::default()
    };
    let mut r = Report::default();
    let groups = build(&verbs, &games, &wn, &freq, &p, &mut r);
    let first = groups
        .iter()
        .position(|g| g.contains(&"ignite".to_string()))
        .expect("the game group");
    let illuminate = groups
        .iter()
        .position(|g| g.contains(&"illuminate".to_string()))
        .expect("the sense-1 synset");
    assert!(
        first < illuminate,
        "a game-derived group must precede every synset of its members: {groups:?}"
    );
    assert_eq!(r.game_kept, 1);
}

#[test]
fn one_story_is_not_evidence() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    let verbs = vec![IfVerb {
        emit: "light".into(),
        lemma: "light".into(),
        stories: 100,
    }];
    // A single game's idiom: `light` and `hurry` are not one action anywhere
    // but in that game.
    let games = vec![GameGroup {
        words: vec!["hurry".into(), "light".into()],
        stories: 1,
    }];
    let mut r = Report::default();
    let groups = build(
        &verbs,
        &games,
        &wn,
        &freq,
        &Params {
            band_cap: 9,
            ..Params::default()
        },
        &mut r,
    );
    assert_eq!(r.game_kept, 0, "one story must not carry a group");
    assert!(!groups
        .iter()
        .any(|g| g.contains(&"hurry".to_string()) && g.contains(&"light".to_string())));
}

#[test]
fn a_truncated_spelling_is_finished_from_the_corpus_or_left_out() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    // `illumi` is what a six-character dictionary holds; the corpus spells the
    // word in full elsewhere, so the group gets the whole word. `zzzzzz` is
    // six characters that finish nothing, and is dropped rather than shown to
    // a player.
    let verbs = vec![
        IfVerb {
            emit: "illuminate".into(),
            lemma: "illuminate".into(),
            stories: 3,
        },
        IfVerb {
            emit: "light".into(),
            lemma: "light".into(),
            stories: 100,
        },
    ];
    let games = vec![GameGroup {
        words: vec!["illumi".into(), "light".into(), "zzzzzz".into()],
        stories: 4,
    }];
    let mut r = Report::default();
    let groups = build(
        &verbs,
        &games,
        &wn,
        &freq,
        &Params {
            band_cap: 9,
            ..Params::default()
        },
        &mut r,
    );
    assert_eq!(r.game_kept, 1, "the entry should have become one group");
    let g = groups
        .iter()
        .find(|g| g.contains(&"illuminate".to_string()) && g.contains(&"light".to_string()))
        .expect("the game group, with the truncation finished");
    assert_eq!(
        g.len(),
        2,
        "`zzzzzz` finishes nothing and must be dropped: {g:?}"
    );
}

/// The committed harvest is what makes `build` reproducible without a corpus of
/// commercial game files, so its shape is worth pinning.
#[test]
fn the_committed_harvest_is_well_formed() {
    let text = include_str!("../if_verbs.tsv");
    let mut n = 0;
    let mut previous = String::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(f.len(), 3, "expected spelling/stories/lemma: {line:?}");
        assert!(
            f[0].bytes()
                .all(|c| c.is_ascii_lowercase() || c == b' ' || c == b'-'),
            "not a lower-case English word: {:?}",
            f[0]
        );
        assert!(
            f[1].parse::<usize>().is_ok_and(|n| n > 0),
            "bad story count: {line:?}"
        );
        assert!(
            f[0] > previous.as_str(),
            "the harvest must be sorted: {line:?}"
        );
        previous = f[0].to_string();
        n += 1;
    }
    assert!(
        n > 2000,
        "only {n} spellings — did the harvest run against an empty corpus?"
    );
    for expected in ["take", "drop", "open", "light", "examine", "turn on"] {
        assert!(
            text.lines()
                .any(|l| l.starts_with(&format!("{expected}\t"))),
            "`{expected}` is missing from the harvest"
        );
    }
}

/// The committed verb ENTRIES, likewise — and this one carries the evidence for
/// the quest: `inspect` and `examine` are one verb in game after game, which is
/// the fact WordNet does not have.
#[test]
fn the_committed_verb_entries_are_well_formed() {
    let text = include_str!("../if_groups.tsv");
    let mut n = 0;
    let mut corroborating_inspect_examine = 0;
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let f: Vec<&str> = line.split('\t').collect();
        assert!(f.len() >= 3, "a count and two or more spellings: {line:?}");
        let stories: usize = f[0].parse().expect("a story count");
        assert!(stories > 0, "a set nobody declares: {line:?}");
        let mut members = f[1..].to_vec();
        for w in &members {
            assert!(
                w.len() >= 2 && w.bytes().all(|c| c.is_ascii_lowercase() || c == b'-'),
                "not a dictionary spelling: {w:?}"
            );
        }
        let sorted = {
            let mut m = members.clone();
            m.sort_unstable();
            m
        };
        assert_eq!(members, sorted, "members must be sorted: {line:?}");
        members.dedup();
        assert_eq!(members.len(), f.len() - 1, "repeated member: {line:?}");
        if members.contains(&"examine") && members.contains(&"inspect") {
            corroborating_inspect_examine += stories;
        }
        n += 1;
    }
    assert!(
        n > 1000,
        "only {n} verb entries — did the harvest run against an empty corpus?"
    );
    assert!(
        corroborating_inspect_examine >= 10,
        "only {corroborating_inspect_examine} stories put `inspect` and `examine` on one verb"
    );
}

fn wide_frequency_fixture() -> Frequency {
    let d = scratch("freq-wide");
    let p = d.join("frq.txt");
    std::fs::write(
        &p,
        "----- 1 -----\n\
         check\ndescribe\nexamine\ninspect\nobserve\nsee\nstudy\nsurvey\nwatch\ntrace\nlight\n",
    )
    .unwrap();
    Frequency::load(&p).expect("fixture loads")
}

/// Rule 1a (SQ-1233): a game-derived group is only swallowed by a WIDER
/// game-derived superset if that superset is at least as well corroborated.
/// `describe/examine/inspect/observe/study/watch` (3 stories) is a strict
/// subset of `check/describe/examine/inspect/observe/see/study/survey/watch`
/// (2 stories) — the unmodified subsumption step drops the narrower, MORE
/// corroborated set purely for being smaller. Falsified by reverting the
/// `groups[j].support >= groups[i].support` gate in the subsumption step: the
/// 6-member group then disappears and this test fails.
#[test]
fn a_narrower_game_group_survives_a_wider_but_weaker_superset() {
    let wn = wordnet_fixture();
    let freq = wide_frequency_fixture();
    let verbs = vec![IfVerb { emit: "light".into(), lemma: "light".into(), stories: 100 }];
    let games = vec![
        GameGroup {
            words: ["check", "describe", "examine", "inspect", "observe", "see", "study", "survey", "watch"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            stories: 2,
        },
        GameGroup {
            words: ["describe", "examine", "inspect", "observe", "study", "watch"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            stories: 3,
        },
    ];
    let p = Params { band_cap: 16, ..Params::default() };
    let mut r = Report::default();
    let groups = build(&verbs, &games, &wn, &freq, &p, &mut r);
    assert!(
        groups.iter().any(|g| g.len() == 6 && g.contains(&"observe".to_string())),
        "the 3-story, 6-member set must survive as its own row: {groups:?}"
    );
    assert_eq!(r.subsumed, 0, "neither set should swallow the other here");
}

/// The companion half of the same bug: two DIFFERENT raw `if_groups.tsv`
/// entries can reduce to the identical final member set after filtering (a
/// truncated spelling untruncating the same way, say), and `keep` used to
/// remember whichever one processed first — not the more corroborated one.
/// Falsified by reverting `keep`'s `seen` map to a bare `game: bool` (losing
/// the index/support update): the kept group's support then reads 1, and this
/// test fails.
#[test]
fn keep_remembers_the_stronger_of_two_identical_declarations() {
    let wn = wordnet_fixture();
    let freq = wide_frequency_fixture();
    let verbs = vec![IfVerb { emit: "light".into(), lemma: "light".into(), stories: 100 }];
    let games = vec![
        GameGroup { words: ["check", "watch"].iter().map(|s| s.to_string()).collect(), stories: 2 },
        GameGroup { words: ["check", "watch"].iter().map(|s| s.to_string()).collect(), stories: 9 },
    ];
    let p = Params { band_cap: 16, ..Params::default() };
    let mut r = Report::default();
    build(&verbs, &games, &wn, &freq, &p, &mut r);
    assert_eq!(r.duplicates, 1, "the second declaration is a duplicate set");
    // Corroborate it against a wider, weaker set that only the STRONGER
    // support number should be able to resist being swallowed by.
    let games2 = vec![
        GameGroup { words: ["check", "watch"].iter().map(|s| s.to_string()).collect(), stories: 2 },
        GameGroup { words: ["check", "watch"].iter().map(|s| s.to_string()).collect(), stories: 9 },
        GameGroup {
            words: ["check", "describe", "watch"].iter().map(|s| s.to_string()).collect(),
            stories: 2,
        },
    ];
    let mut r2 = Report::default();
    let groups = build(&verbs, &games2, &wn, &freq, &p, &mut r2);
    assert!(
        groups.iter().any(|g| g.len() == 2 && g.contains(&"check".to_string())),
        "the 9-story pair must have survived the weaker 2-story triple: {groups:?}"
    );
}

/// Rule 2 (SQ-1233): a WordNet group's member is dropped when (a) the
/// synset was not that word's OWN reason for being selected — its sense rank
/// for this offset falls outside `sense_cap` — and (b) the corpus
/// corroborates a completely different, disjoint action for it.
///
/// `clear`'s synonym-of-`enlighten` sense mirrors `illuminate`'s real
/// "clarify" collision: `clear`'s dominant corpus sense (grouped here with
/// `push`) shares nothing with `enlighten`/`elucidate`, so `clear` must be
/// dropped from that group while it survives with its other members intact.
/// `light`, by contrast, sits in ITS OWN group at its OWN top sense and must
/// never be touched, mirroring `illuminate`'s "light/light up" group.
///
/// Falsified by skipping the bystander filter entirely (comment it out in
/// `build`): `clear` then stays in the clarify group and this test fails.
fn bystander_fixture() -> (WordNet, Frequency) {
    let d = scratch("wn-bystander");
    std::fs::write(
        d.join("index.verb"),
        "light v 1 0 1 0 00000001  \n\
         enlighten v 1 0 1 0 00000002  \n\
         elucidate v 1 0 1 0 00000002  \n\
         clear v 2 0 2 0 00000003 00000002  \n",
    )
    .unwrap();
    std::fs::write(
        d.join("data.verb"),
        "00000001 30 v 02 light 0 illuminate 0 000 | make lighter\n\
         00000002 30 v 03 clear 0 enlighten 0 elucidate 0 000 | make clear\n",
    )
    .unwrap();
    std::fs::write(d.join("verb.exc"), "").unwrap();
    std::fs::write(d.join("noun.exc"), "").unwrap();
    let wn = WordNet::load(&d).expect("fixture loads");

    let d2 = scratch("freq-bystander");
    let p = d2.join("frq.txt");
    std::fs::write(
        &p,
        "----- 1 -----\nlight\nilluminate\nclear\nenlighten\nelucidate\npush\n",
    )
    .unwrap();
    let freq = Frequency::load(&p).expect("fixture loads");
    (wn, freq)
}

#[test]
fn a_bystander_member_is_dropped_when_the_corpus_disagrees() {
    let (wn, freq) = bystander_fixture();
    let verbs = vec![
        IfVerb { emit: "light".into(), lemma: "light".into(), stories: 100 },
        IfVerb { emit: "clear".into(), lemma: "clear".into(), stories: 50 },
        IfVerb { emit: "enlighten".into(), lemma: "enlighten".into(), stories: 3 },
        IfVerb { emit: "elucidate".into(), lemma: "elucidate".into(), stories: 3 },
    ];
    // `clear`'s attested corpus action: pushing something aside — nothing to
    // do with clarifying, and disjoint from the clarify synset's members.
    let games = vec![GameGroup {
        words: ["clear", "push"].iter().map(|s| s.to_string()).collect(),
        stories: 2,
    }];
    let p = Params { band_cap: 16, sense_cap: 1, ..Params::default() };
    let mut r = Report::default();
    let groups = build(&verbs, &games, &wn, &freq, &p, &mut r);

    let clarify = groups
        .iter()
        .find(|g| g.contains(&"enlighten".to_string()))
        .expect("the clarify group survives with its other members");
    assert!(
        !clarify.contains(&"clear".to_string()),
        "`clear` must be dropped as a bystander: {clarify:?}"
    );
    assert!(clarify.contains(&"elucidate".to_string()));
    assert_eq!(r.bystanders_dropped.len(), 1);
    assert_eq!(r.bystanders_dropped[0].0, "clear");

    let light = groups
        .iter()
        .find(|g| g.contains(&"illuminate".to_string()))
        .expect("light's own group is untouched");
    assert!(
        light.contains(&"light".to_string()),
        "light is its OWN top sense here and must never be treated as a bystander: {light:?}"
    );
}

/// Rule 3 (SQ-1233): within a group, members are ordered by how much of the
/// corpus's OWN evidence for this cluster backs each spelling — not by each
/// spelling's overall popularity across every sense it happens to have.
/// `watch` has more raw IF-verb story support than `examine` here (an
/// unrelated, un-corroborating declaration inflates it), but `examine` is
/// named alongside this group's OTHER members more often, and must lead.
///
/// Falsified by reverting the sort to plain `stories(w)`: `watch` (60) then
/// outranks `examine` (50) and this test fails.
#[test]
fn member_order_prefers_the_spelling_the_corpus_clusters_around() {
    let wn = wordnet_fixture();
    let d = scratch("freq-cluster");
    let p = d.join("frq.txt");
    std::fs::write(&p, "----- 1 -----\nexamine\nwatch\ndescribe\n").unwrap();
    let freq = Frequency::load(&p).expect("fixture loads");
    let verbs = vec![
        IfVerb { emit: "examine".into(), lemma: "examine".into(), stories: 50 },
        IfVerb { emit: "watch".into(), lemma: "watch".into(), stories: 60 },
        IfVerb { emit: "describe".into(), lemma: "describe".into(), stories: 10 },
    ];
    let games = vec![
        GameGroup {
            words: ["examine", "watch", "describe"].iter().map(|s| s.to_string()).collect(),
            stories: 3,
        },
        // Below `game_support`, so it never becomes a row of its own, but its
        // raw declaration still counts as corpus evidence for member order.
        GameGroup {
            words: ["examine", "describe"].iter().map(|s| s.to_string()).collect(),
            stories: 1,
        },
    ];
    let p = Params { band_cap: 16, ..Params::default() };
    let mut r = Report::default();
    let groups = build(&verbs, &games, &wn, &freq, &p, &mut r);
    let g = groups
        .iter()
        .find(|g| g.len() == 3 && g.contains(&"watch".to_string()))
        .expect("the three-member group");
    assert_eq!(g[0], "examine", "the corpus-clustered spelling must lead: {g:?}");
}

/// Rule 4 (SQ-1233): an `un`-prefixed spelling that reaches no group through
/// any earlier pass gets one — from the corpus's own (possibly single-story)
/// declaration for it when there is one, or paired with its bare base verb
/// otherwise. Neither channel exists for these words before this pass runs.
///
/// Falsified by deleting the call to `derive_reversals` in `build`: both
/// assertions below fail because neither spelling reaches any group.
#[test]
fn un_prefixed_spellings_reach_a_group() {
    let wn = wordnet_fixture();
    let d = scratch("freq-reversal");
    let p = d.join("frq.txt");
    std::fs::write(&p, "----- 1 -----\nmask\npin\nstrip\nunmask\nunpin\n").unwrap();
    let freq = Frequency::load(&p).expect("fixture loads");
    let verbs = vec![
        IfVerb { emit: "mask".into(), lemma: "mask".into(), stories: 20 },
        IfVerb { emit: "pin".into(), lemma: "pin".into(), stories: 5 },
        IfVerb { emit: "strip".into(), lemma: "strip".into(), stories: 8 },
        IfVerb { emit: "unmask".into(), lemma: "unmask".into(), stories: 2 },
        IfVerb { emit: "unpin".into(), lemma: "unpin".into(), stories: 2 },
    ];
    // The corpus declares `unmask` itself, at one story — below
    // `game_support` on its own, which is exactly the case this pass exists
    // for. It declares nothing at all for `unpin`.
    let games = vec![GameGroup {
        words: ["strip", "unmask"].iter().map(|s| s.to_string()).collect(),
        stories: 1,
    }];
    let p = Params { band_cap: 16, ..Params::default() };
    let mut r = Report::default();
    let groups = build(&verbs, &games, &wn, &freq, &p, &mut r);

    let unmask = groups
        .iter()
        .find(|g| g.contains(&"unmask".to_string()))
        .expect("unmask reaches the corpus's own declaration");
    assert!(unmask.contains(&"strip".to_string()));
    assert_eq!(unmask.len(), 2, "exactly the corpus's own declared pair: {unmask:?}");

    let unpin = groups
        .iter()
        .find(|g| g.contains(&"unpin".to_string()))
        .expect("unpin falls back to pairing with its base verb");
    assert!(unpin.contains(&"pin".to_string()));
    assert_eq!(unpin.len(), 2, "the fallback pairing is minimal: {unpin:?}");

    assert_eq!(r.reversal_candidates, 2);
    assert_eq!(r.reversals_from_corpus, 1);
    assert_eq!(r.reversals_paired_with_base, 1);
}

/// A well-corroborated `un`-cluster that Pass 0 already built normally is
/// left exactly as it is — this pass only reaches spellings that are
/// otherwise unreachable, never a word that already has a home.
#[test]
fn un_prefixed_derivation_does_not_touch_an_already_reachable_spelling() {
    let wn = wordnet_fixture();
    let d = scratch("freq-reversal-noop");
    let p = d.join("frq.txt");
    std::fs::write(&p, "----- 1 -----\nhook\nfree\nuntie\nunhook\n").unwrap();
    let freq = Frequency::load(&p).expect("fixture loads");
    let verbs = vec![
        IfVerb { emit: "hook".into(), lemma: "hook".into(), stories: 10 },
        IfVerb { emit: "free".into(), lemma: "free".into(), stories: 30 },
        IfVerb { emit: "untie".into(), lemma: "untie".into(), stories: 40 },
        IfVerb { emit: "unhook".into(), lemma: "unhook".into(), stories: 25 },
    ];
    let games = vec![GameGroup {
        words: ["free", "untie", "unhook"].iter().map(|s| s.to_string()).collect(),
        stories: 7,
    }];
    let p = Params { band_cap: 16, ..Params::default() };
    let mut r = Report::default();
    let groups = build(&verbs, &games, &wn, &freq, &p, &mut r);
    assert_eq!(r.reversal_candidates, 0, "unhook already reaches a group through Pass 0");
    assert_eq!(
        groups.iter().filter(|g| g.contains(&"unhook".to_string())).count(),
        1,
        "no duplicate group should appear: {groups:?}"
    );
}

/// Rule 1b (SQ-1233): among DISJOINT game-derived groups sharing a word (not
/// a subset/superset pair — that is the subsumption test above), the one
/// with MORE support must be offered first. `push/press/shove` (5 stories)
/// must precede `pull/drag/tug/yank/shove` (4 stories) for `shove`, which is
/// the exact SQ-1206 finding.
///
/// Falsified by reverting `order_by_sense`'s `tie` to always 0: the two
/// groups then keep whatever order the alphabetical/offset tie-break gives
/// them, independent of support, and this test fails (the pull group's first
/// member sorts alphabetically before the push group's).
#[test]
fn game_groups_sharing_a_word_are_ordered_by_support() {
    let wn = wordnet_fixture();
    let d = scratch("freq-shove");
    let p = d.join("frq.txt");
    std::fs::write(&p, "----- 1 -----\npush\npress\nshove\npull\ndrag\ntug\nyank\n").unwrap();
    let freq = Frequency::load(&p).expect("fixture loads");
    let verbs = vec![
        IfVerb { emit: "push".into(), lemma: "push".into(), stories: 10 },
        IfVerb { emit: "press".into(), lemma: "press".into(), stories: 10 },
        IfVerb { emit: "shove".into(), lemma: "shove".into(), stories: 10 },
        IfVerb { emit: "pull".into(), lemma: "pull".into(), stories: 10 },
        IfVerb { emit: "drag".into(), lemma: "drag".into(), stories: 10 },
        IfVerb { emit: "tug".into(), lemma: "tug".into(), stories: 10 },
        IfVerb { emit: "yank".into(), lemma: "yank".into(), stories: 10 },
    ];
    let games = vec![
        GameGroup {
            words: ["pull", "drag", "tug", "yank", "shove"].iter().map(|s| s.to_string()).collect(),
            stories: 4,
        },
        GameGroup {
            words: ["push", "press", "shove"].iter().map(|s| s.to_string()).collect(),
            stories: 5,
        },
    ];
    let p = Params { band_cap: 16, ..Params::default() };
    let mut r = Report::default();
    let groups = build(&verbs, &games, &wn, &freq, &p, &mut r);
    let push_group = groups.iter().position(|g| g.contains(&"push".to_string())).expect("push group");
    let pull_group = groups.iter().position(|g| g.contains(&"pull".to_string())).expect("pull group");
    assert!(
        push_group < pull_group,
        "the 5-story push group must precede the 4-story pull group: {groups:?}"
    );
}
