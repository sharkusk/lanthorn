//! `verb-synonyms-gen` — rebuild the shipped player-word → IF-verb table.
//!
//! ```text
//! verb-synonyms-gen harvest --corpus stories --corpus unit_tests \
//!     --wordnet <dict> --freq <2+2+3frq.txt> \
//!     -o if_verbs.tsv --groups if_groups.tsv
//! verb-synonyms-gen build --wordnet <dict> --freq <2+2+3frq.txt> \
//!     --if-verbs if_verbs.tsv --if-groups if_groups.tsv \
//!     -o crates/verb-synonyms/src/synonym_groups.tsv
//! verb-synonyms-gen irregulars --wordnet <dict> \
//!     -o crates/verb-synonyms/src/irregular_forms.tsv
//! ```
//!
//! `irregulars` is the odd one out: it reads no corpus and no frequency list,
//! because an irregular inflection is a fact about English and not about
//! interactive fiction. It copies WordNet's own exception lists out, which is
//! the only honest way to hold them — a hand-written table would be a second
//! copy of this data with nothing keeping the two the same.
//!
//! `--groups` and `--if-groups` are the corpus's OWN synonym groups — the
//! spellings each story's author declared to be one verb. Leaving `--if-groups`
//! off builds the WordNet half alone, which is how the two sources are measured
//! apart.
//!
//! Argument parsing is hand-rolled rather than `clap`'d: this crate takes no
//! external dependencies, which keeps it buildable in any checkout of the
//! workspace and makes it obvious that nothing here reaches the shipped binary.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use verb_synonyms_gen::build::{self, GameGroup, IfVerb, Params, Report};
use verb_synonyms_gen::harvest::{self, Harvest};
use verb_synonyms_gen::sources::{Frequency, WordNet};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("harvest") => cmd_harvest(&args[1..]),
        Some("build") => cmd_build(&args[1..]),
        Some("irregulars") => cmd_irregulars(&args[1..]),
        _ => {
            eprintln!(
                "usage: verb-synonyms-gen <harvest|build|irregulars> [options]  (see crate docs)"
            );
            return ExitCode::FAILURE;
        }
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("verb-synonyms-gen: {e}");
            ExitCode::FAILURE
        }
    }
}

/// One `--flag value` scan. Repeated flags collect; a missing value is an error.
fn opts(args: &[String]) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let name = match a.as_str() {
            "-o" => "out",
            _ => a
                .strip_prefix("--")
                .ok_or_else(|| format!("unexpected argument `{a}`"))?,
        };
        if name == "no-gap-fill" {
            m.entry(name.to_string()).or_default().push(String::new());
            i += 1;
            continue;
        }
        let v = args
            .get(i + 1)
            .ok_or_else(|| format!("`--{name}` wants a value"))?;
        m.entry(name.to_string()).or_default().push(v.clone());
        i += 2;
    }
    Ok(m)
}

fn one(m: &BTreeMap<String, Vec<String>>, k: &str) -> Result<String, String> {
    m.get(k)
        .and_then(|v| v.first())
        .cloned()
        .ok_or_else(|| format!("`--{k}` is required"))
}

fn num<T: std::str::FromStr>(
    m: &BTreeMap<String, Vec<String>>,
    k: &str,
    d: T,
) -> Result<T, String> {
    match m.get(k).and_then(|v| v.first()) {
        None => Ok(d),
        Some(s) => s
            .parse()
            .map_err(|_| format!("`--{k}` wants a number, got `{s}`")),
    }
}

// ── harvest ──────────────────────────────────────────────────────────────────

fn cmd_harvest(args: &[String]) -> Result<(), String> {
    let m = opts(args)?;
    let corpora = m
        .get("corpus")
        .ok_or("`--corpus <dir>` is required (repeatable)")?;
    let out = PathBuf::from(one(&m, "out")?);
    let wn = WordNet::load(std::path::Path::new(&one(&m, "wordnet")?))
        .map_err(|e| format!("wordnet: {e}"))?;
    let freq = Frequency::load(std::path::Path::new(&one(&m, "freq")?))
        .map_err(|e| format!("freq: {e}"))?;

    let mut h = Harvest::default();
    for dir in corpora {
        harvest::sweep(std::path::Path::new(dir), &mut h).map_err(|e| format!("{dir}: {e}"))?;
    }

    eprintln!(
        "read {} stories ({} z-machine, {} glulx, {} scott)",
        h.read,
        h.by_engine[harvest::ENGINE_Z],
        h.by_engine[harvest::ENGINE_GLULX],
        h.by_engine[harvest::ENGINE_SCOTT]
    );
    eprintln!("{} distinct verb spellings", h.verbs.len());
    let single = h.verbs.iter().filter(|w| !w.contains(' ')).count();
    eprintln!(
        "  {single} single words, {} verb+preposition phrases",
        h.verbs.len() - single
    );
    eprintln!(
        "{} synonym groups declared by the stories themselves ({} distinct sets)",
        h.groups.values().map(BTreeSet::len).sum::<usize>(),
        h.groups.len()
    );
    let corroborated = h.groups.values().filter(|s| s.len() > 1).count();
    eprintln!("  {corroborated} of those sets are declared by more than one story");
    eprintln!(
        "  widest set: {} members",
        h.groups.keys().map(Vec::len).max().unwrap_or(0)
    );
    if h.double_booked.is_empty() {
        eprintln!("  no spelling appears in two verb entries of one story, as expected");
    } else {
        eprintln!(
            "  {} spellings appear in TWO verb entries of one story: {}",
            h.double_booked.len(),
            h.double_booked
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    eprintln!("{} files skipped:", h.skipped.len());
    for (p, why) in &h.skipped {
        eprintln!("  {} — {why}", p.display());
    }

    if let Some(path) = m.get("groups").and_then(|v| v.first()) {
        write_groups(std::path::Path::new(path), &h)?;
    }

    let resolved = lemmatise(&h, &wn, &freq);

    let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
    writeln!(
        f,
        "# Verb vocabulary harvested from a corpus of interactive fiction."
    )
    .unwrap();
    writeln!(f, "#").unwrap();
    writeln!(f, "# Regenerate:").unwrap();
    writeln!(
        f,
        "#   verb-synonyms-gen harvest --corpus stories --corpus unit_tests \\"
    )
    .unwrap();
    writeln!(
        f,
        "#       --wordnet <WordNet-3.0/dict> --freq <12dicts/Lemmatized/2+2+3frq.txt> -o …"
    )
    .unwrap();
    writeln!(f, "#").unwrap();
    writeln!(
        f,
        "# Tab-separated: spelling, the number of stories whose parser accepts it, and"
    )
    .unwrap();
    writeln!(
        f,
        "# the WordNet verb lemma to expand it under (empty when WordNet has no verb"
    )
    .unwrap();
    writeln!(
        f,
        "# entry, which is most of the abbreviations, magic words and game-specific"
    )
    .unwrap();
    writeln!(
        f,
        "# actions).  A spelling containing a space is a verb plus a literal word from"
    )
    .unwrap();
    writeln!(
        f,
        "# one of its syntax lines — `turn on`, `pick up` — which is how English"
    )
    .unwrap();
    writeln!(
        f,
        "# lexicalises them and how a thesaurus indexes them, even though a dictionary"
    )
    .unwrap();
    writeln!(f, "# can only hold the head word.").unwrap();
    writeln!(f, "#").unwrap();
    writeln!(
        f,
        "# These are the STORIES' OWN spellings and are ground truth: a suggestion the"
    )
    .unwrap();
    writeln!(
        f,
        "# parser would reject is worthless.  An inflected spelling is dropped only when"
    )
    .unwrap();
    writeln!(
        f,
        "# every story that accepts it also accepts its base form."
    )
    .unwrap();
    writeln!(f, "#").unwrap();
    writeln!(
        f,
        "# {} stories read; {} spellings.",
        h.read,
        resolved.len()
    )
    .unwrap();
    for (w, stories, lemma) in &resolved {
        writeln!(f, "{w}\t{stories}\t{lemma}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Write the corpus's own synonym sets — one verb entry per line, with the
/// number of stories that declare exactly that set.
///
/// Committed for the same reason `if_verbs.tsv` is: it is what makes `build`
/// reproducible without a corpus of commercial game files. And it carries no
/// more than that list does — de-duplicated vocabulary, no game text, no
/// titles, and no attribution of any word to any story; the per-story detail
/// stays in this command's stderr report.
fn write_groups(path: &std::path::Path, h: &Harvest) -> Result<(), String> {
    let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let head = format!(
        "\
# Synonym groups declared by the stories themselves — the IF-native half of the
# shipped table's evidence.  GENERATED; do not hand-edit.
#
# Regenerate:
#   verb-synonyms-gen harvest --corpus stories --corpus unit_tests \\
#       --wordnet <WordNet-3.0/dict> --freq <12dicts/Lemmatized/2+2+3frq.txt> \\
#       -o crates/verb-synonyms-gen/if_verbs.tsv \\
#       --groups crates/verb-synonyms-gen/if_groups.tsv
#
# One line per DISTINCT verb entry: the number of stories that declare exactly
# that set of spellings, then the spellings, tab-separated and sorted.  A verb
# entry is a game author's own statement that these words are one action — for
# a parser that is a stronger authority than a thesaurus, and it is free, since
# the grammar is already loaded to read the vocabulary out of.
#
# This file is EVIDENCE, not the table: no threshold has been applied.  A set
# declared by one story may be that story's private idiom, which is why the
# count is here and why `build --game-support` decides what to believe.  The
# spellings are the stories' own, filtered only for shape (three or more
# letters, so `x`, `g` and `n` are absent); the verb-plus-preposition phrases
# `if_verbs.tsv` carries are NOT grouped, because a story that calls `turn` and
# `rotate` one verb has said nothing about `rotate on`.
#
# {} stories read; {} distinct sets, {} declarations.
",
        h.read,
        h.groups.len(),
        h.groups.values().map(BTreeSet::len).sum::<usize>(),
    );
    write!(f, "{head}").map_err(|e| e.to_string())?;
    for (set, stories) in &h.groups {
        writeln!(f, "{}\t{}", stories.len(), set.join("\t")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Attach a WordNet lemma to each harvested spelling, and drop the inflected
/// spellings that are genuinely redundant.
///
/// The design assumes IF verbs are imperatives — you type `take lamp`, never
/// `took lamp` — and conversation does not change that, because `ask x about y`
/// and `floyd, take the card` are imperative too. Parsers that accept questions
/// are where a real inflection could appear, so this MEASURES it rather than
/// asserting it, and prints which stories each one came from.
///
/// Two things it deliberately does not do:
///
///   * It never applies a suffix rule. `dress`, `press` and `sing` end in the
///     letters of an inflection and are lemmas; only WordNet's `verb.exc` and
///     12dicts' lemmatisation get a vote.
///   * It never lemmatises a spelling WordNet lists as a lemma in its own
///     right. `saw`, `lay`, `rent`, `wound`, `fell` and `bound` all reach a base
///     form through `verb.exc` and every one of them is also a verb a player
///     types: you saw a log, you lay a rug, you rent a room. Rewriting those
///     would delete a real IF verb.
fn lemmatise(h: &Harvest, wn: &WordNet, freq: &Frequency) -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    let mut inflected = Vec::new();
    let mut dropped = Vec::new();
    let mut homographs = 0usize;
    for w in &h.verbs {
        let lemma = if wn.senses.contains_key(w) {
            if wn.exceptions.contains_key(w) {
                homographs += 1;
            }
            w.clone()
        } else if let Some(b) =
            wn.exceptions.get(w).and_then(|b| b.first()).filter(|b| wn.senses.contains_key(*b))
        {
            inflected.push((w.clone(), b.clone(), "verb.exc"));
            b.clone()
        } else if let Some(b) = freq
            .lemma_of
            .get(w)
            .filter(|b| *b != w && wn.senses.contains_key(*b))
        {
            inflected.push((w.clone(), b.clone(), "12dicts"));
            b.clone()
        } else {
            out.push((w.clone(), h.story_count(w), String::new()));
            continue;
        };
        // Redundant only if every story that accepts the inflection also
        // accepts the base. Otherwise the base is not a spelling that story's
        // parser would take, and dropping this one loses a real verb.
        if lemma != *w {
            let covered = h
                .sources
                .get(&lemma)
                .is_some_and(|base| h.sources[w].iter().all(|s| base.contains(s)));
            if covered {
                dropped.push(format!("{w}→{lemma}"));
                continue;
            }
        }
        out.push((w.clone(), h.story_count(w), lemma));
    }

    eprintln!("\ninflected IF verbs — spellings WordNet knows ONLY as an inflection:");
    for (w, base, via) in &inflected {
        let from: Vec<&str> = h.sources[w].iter().map(String::as_str).take(6).collect();
        eprintln!("  {w} → {base}  ({via})  [{}]", from.join(", "));
    }
    eprintln!(
        "  {} of {} single-word spellings; a further {homographs} look inflected but are \
         lemmas in their own right and are left as the story spells them",
        inflected
            .iter()
            .filter(|(w, _, _)| !w.contains(' '))
            .count(),
        h.verbs.iter().filter(|w| !w.contains(' ')).count()
    );
    eprintln!(
        "  dropped as redundant ({}): {}",
        dropped.len(),
        dropped.join(" ")
    );
    eprintln!(
        "  kept because some story has no base form: {}",
        inflected
            .iter()
            .filter(|(w, l, _)| !dropped.contains(&format!("{w}→{l}")))
            .map(|(w, l, _)| format!("{w}(→{l})"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    out
}

// ── irregulars ───────────────────────────────────────────────────────────────

/// Write `irregular_forms.tsv`: WordNet's exception lists, as `form → base`.
///
/// `vocab::stems` in `app` builds a base by taking an ending off, which reaches
/// every regular inflection in English and none of the irregular ones — `lit`,
/// `took`, `went` and `mice` share no letters with any suffix rule. This is the
/// table that answers those, and it is GENERATED rather than written because
/// WordNet already holds it: a hand table would be a second copy of the same
/// data, to be reconciled with the first every time either one moved.
///
/// NOUNS as well as verbs, which is wider than the quest asked for and
/// deliberate: `stems` is consulted from a position-generic place, so it serves
/// the noun slot of a command as well as the verb slot, and `mice → mouse` is
/// the same case as `lit → light` one slot to the right.
fn cmd_irregulars(args: &[String]) -> Result<(), String> {
    let m = opts(args)?;
    let out = PathBuf::from(one(&m, "out")?);
    let dict = one(&m, "wordnet")?;
    let wn = WordNet::load(std::path::Path::new(&dict)).map_err(|e| format!("wordnet: {e}"))?;
    if wn.noun_exceptions.is_empty() {
        return Err(format!(
            "{dict}/noun.exc is missing or empty — the DB-only WNdb tarball does not carry the \
             exception lists; fetch-sources.sh downloads the full WordNet-3.0.tar.gz"
        ));
    }

    // A SET, so a pair both parts of speech list is written once. Sorted,
    // because unlike `synonym_groups.tsv` nothing here depends on line order.
    let mut rows: BTreeSet<(String, String)> = BTreeSet::new();
    let mut verb_rows = 0usize;
    let mut noun_rows = 0usize;
    for (map, n) in [
        (&wn.exceptions, &mut verb_rows),
        (&wn.noun_exceptions, &mut noun_rows),
    ] {
        for (form, bases) in map {
            for base in bases {
                // WordNet gives ten verb forms and fifteen noun forms back as
                // their own base — `bed bed`, `is is` — which is its way of
                // saying the form is not an inflection at all. As a row here it
                // would offer a player the word they just typed.
                if base == form {
                    continue;
                }
                *n += 1;
                rows.insert((form.clone(), base.clone()));
            }
        }
    }
    let forms: BTreeSet<&str> = rows.iter().map(|(f, _)| f.as_str()).collect();

    let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
    let head = format!(
        "\
# Irregular inflections — the forms no suffix rule can reach.  GENERATED; do
# not hand-edit.
#
# Regenerate:
#   ./crates/verb-synonyms-gen/fetch-sources.sh /tmp/verbsyn
#   cargo run -p verb-synonyms-gen -- irregulars \\
#       --wordnet /tmp/verbsyn/WordNet-3.0/dict \\
#       -o crates/verb-synonyms/src/irregular_forms.tsv
#
# SOURCE: WordNet 3.0 (Princeton University, 2006) — `verb.exc` and `noun.exc`
# out of WordNet-3.0.tar.gz, sha256 640db279…d3a52.  The notice Princeton's
# licence requires is in THIRD-PARTY-NOTICES.md at the repository root.  The
# DB-only WNdb-3.0.tar.gz carries neither file, which is why fetch-sources.sh
# takes the full tarball.
#
# FORMAT: one `form<TAB>base` per line.  A form appears on SEVERAL lines when
# WordNet gives it more than one base — `axes` is `ax` and `axis`, `singing` is
# `sing` and `singe`, `overflown` is `overflow` and `overfly` — which is why the
# reader hands back a slice and lets the story's own dictionary choose.
#
# SORTED, and unlike synonym_groups.tsv that is safe: nothing here depends on
# line order.  A form's bases are alternative readings of one spelling with no
# ranking between them available or wanted, so the file is sorted to be
# greppable and to diff cleanly across a regeneration.
#
# VERBS AND NOUNS BOTH.  `lit` → `light` is the motivating case, but the
# consumer (`vocab::stems` in `app`) is asked about every position in a command,
# so `mice` → `mouse` is the same case one slot to the right.  A pair that both
# lists give is written once.
#
# {} verb rows, {} noun rows, {} lines, {} distinct forms.
",
        verb_rows,
        noun_rows,
        rows.len(),
        forms.len(),
    );
    write!(f, "{head}").map_err(|e| e.to_string())?;
    for (form, base) in &rows {
        writeln!(f, "{form}\t{base}").map_err(|e| e.to_string())?;
    }

    eprintln!("verb.exc rows                 {verb_rows}");
    eprintln!("noun.exc rows                 {noun_rows}");
    eprintln!("lines written                 {}", rows.len());
    eprintln!("  distinct forms              {}", forms.len());
    eprintln!(
        "  extra bases beyond one per form {}",
        rows.len() - forms.len()
    );
    Ok(())
}

// ── build ────────────────────────────────────────────────────────────────────

fn cmd_build(args: &[String]) -> Result<(), String> {
    let m = opts(args)?;
    let p = Params {
        sense_cap: num(&m, "sense-cap", Params::default().sense_cap)?,
        band_cap: num(&m, "band-cap", Params::default().band_cap)?,
        group_cap: num(&m, "group-cap", Params::default().group_cap)?,
        hyponym_cap: num(&m, "hyponym-cap", Params::default().hyponym_cap)?,
        common_bands: num(&m, "common-bands", Params::default().common_bands)?,
        gap_fill: !m.contains_key("no-gap-fill"),
        game_support: num(&m, "game-support", Params::default().game_support)?,
        game_group_cap: num(&m, "game-group-cap", Params::default().game_group_cap)?,
    };
    let wn = WordNet::load(std::path::Path::new(&one(&m, "wordnet")?))
        .map_err(|e| format!("wordnet: {e}"))?;
    let freq = Frequency::load(std::path::Path::new(&one(&m, "freq")?))
        .map_err(|e| format!("freq: {e}"))?;

    let text = std::fs::read_to_string(one(&m, "if-verbs")?).map_err(|e| e.to_string())?;
    let mut rows = 0usize;
    let verbs: Vec<IfVerb> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            rows += 1;
            let mut f = l.split('\t');
            let emit = f.next()?.to_string();
            let stories = f.next()?.trim().parse().ok()?;
            let lemma = f.next()?.trim().to_string();
            (!lemma.is_empty()).then_some(IfVerb {
                emit,
                lemma,
                stories,
            })
        })
        .collect();

    // The corpus's own verb entries. Optional so that `--no-game-groups` is
    // simply leaving the flag off, which is what makes the two halves of the
    // table separable when a measurement needs them apart.
    let games = match m.get("if-groups").and_then(|v| v.first()) {
        None => Vec::new(),
        Some(path) => read_game_groups(std::path::Path::new(path))?,
    };

    let mut report = Report::default();
    let table = build::build(&verbs, &games, &wn, &freq, &p, &mut report);

    let out = PathBuf::from(one(&m, "out")?);
    write_table(&out, &table, &p, rows, verbs.len())?;
    print_report(&report, &table, &p, rows, verbs.len());
    Ok(())
}

/// Read `if_groups.tsv` — a story count, then that entry's spellings.
fn read_game_groups(path: &std::path::Path) -> Result<Vec<GameGroup>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let mut f = line.split('\t');
        let stories: usize = f
            .next()
            .and_then(|n| n.trim().parse().ok())
            .ok_or_else(|| format!("{}: expected a story count: {line:?}", path.display()))?;
        let words: Vec<String> = f.map(str::to_string).collect();
        if words.len() >= 2 {
            out.push(GameGroup { words, stories });
        }
    }
    Ok(out)
}

fn write_table(
    out: &std::path::Path,
    groups: &[Vec<String>],
    p: &Params,
    harvested: usize,
    expanded: usize,
) -> Result<(), String> {
    let mut f = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let members: usize = groups.iter().map(Vec::len).sum();
    let head = format!(
        "\
# Synonym groups for interactive-fiction verbs.  GENERATED; do not hand-edit.
#
# Regenerate with `verb-synonyms-gen` — see crates/verb-synonyms-gen/README.md.
# Derived from WordNet 3.0 (Princeton University) and the 12dicts 6.0.2
# lemmatized frequency list (Alan Beale, under the AGID terms).  The notices
# both licences require are in THIRD-PARTY-NOTICES.md at the repository root.
#
# FORMAT: one group per line, members separated by TABS.  There is no key
# column — every member is equal, and a word may appear in SEVERAL groups, one
# per sense.  A tab is the separator because a member may itself contain a
# space (`turn on`), and no dictionary word contains a tab.
#
# A group is one of two things, and neither is ever merged with the other:
#
#   * ONE VERB ENTRY some stories declare — the spellings a game's own author
#     wrote on a single verb, which is that author saying these words are one
#     action.  Believed only where several stories agree, and greppable in
#     crates/verb-synonyms-gen/if_groups.tsv, which is where a row's provenance
#     can be looked up: this file is members and nothing else.
#   * ONE WORDNET SYNSET, filtered to words a player might type, plus — where a
#     synset no story could match has a hypernym that one can — that synset
#     unioned with its immediate hypernym.  Exactly one hop, never chained.
#
# WHERE THE TWO DISAGREE, THE GAMES WIN.  A game-derived group comes before
# every synset containing the same word, because a game author writing
# `Verb 'examine' 'x' 'inspect'` has stated what a word means IN A PARSER, and
# that is the only question this table asks.  WordNet is not overruled, only
# outranked: its groups still follow, and reach a story that implements a word
# no game in the corpus grouped.
#
# AT LOOKUP, two rules define what this data means:
#   1. Lemmatise the player's word FIRST.  Members are base forms, because an IF
#      parser accepts the imperative; `illuminated` never arrives here, and if
#      the consumer skips this step a miss looks like a hole in the data instead
#      of a missing step in the caller.
#   2. Intersect the group with THIS story's dictionary and show only what
#      survives, then drop the word the player actually typed — it is in the
#      group by construction and it is the one word known to have failed.
#
# LINE ORDER IS SIGNIFICANT — DO NOT SORT THIS FILE.  The groups containing any
# given word appear best-guess first: that word's game-derived groups, then its
# synsets in WordNet's own sense order, commonest sense first.  A consumer can
# therefore walk them and stop after three or four dictionary matches.  Sorting
# the file alphabetically destroys that signal silently.  Member order within a
# line is significant too: verbs the corpus actually uses come first, commonest
# first.
#
# Built with: sense-cap {} band-cap {} group-cap {} hyponym-cap {} gap-fill {}
#             game-support {} game-group-cap {}
# From {} harvested IF spellings, {} of which WordNet knows as verbs.
# {} groups, {} memberships.
",
        p.sense_cap,
        p.band_cap,
        p.group_cap,
        p.hyponym_cap,
        p.gap_fill,
        p.game_support,
        p.game_group_cap,
        harvested,
        expanded,
        groups.len(),
        members,
    );
    write!(f, "{head}").map_err(|e| e.to_string())?;
    for g in groups {
        writeln!(f, "{}", g.join("\t")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn print_report(r: &Report, groups: &[Vec<String>], p: &Params, harvested: usize, expanded: usize) {
    eprintln!("harvested spellings           {harvested}");
    eprintln!("  WordNet knows as a verb     {expanded}");
    eprintln!("verb entries the corpus declares  {}", r.game_sets);
    eprintln!(
        "  declared by >= {} stories    {}",
        p.game_support, r.game_corroborated
    );
    eprintln!("  …kept as groups             {}", r.game_kept);
    eprintln!("  …dropped as too wide        {}", r.game_oversize.len());
    eprintln!("synsets inside the sense cap  {}", r.groups_before_prune);
    eprintln!("  …containing an IF verb      {}", r.groups_after_prune);
    eprintln!("gap-fill groups (synset ∪ hypernym)  {}", r.gap_filled);
    eprintln!("duplicate groups discarded    {}", r.duplicates);
    eprintln!("  …a synset a game already said {}", r.game_agrees);
    eprintln!("groups subsumed by another    {}", r.subsumed);
    eprintln!(
        "bystander members dropped     {} (a WordNet sense the corpus disagrees with)",
        r.bystanders_dropped.len()
    );
    for (w, kept) in r.bystanders_dropped.iter().take(20) {
        eprintln!("  {w} — corpus says that's a different action from {kept}");
    }
    eprintln!("`un-` spellings considered    {}", r.reversal_candidates);
    eprintln!("  …resolved from the corpus's own declaration  {}", r.reversals_from_corpus);
    eprintln!("  …paired with their bare base verb            {}", r.reversals_paired_with_base);
    eprintln!("sense-order constraints broken {}", r.order_conflicts);
    eprintln!("groups written                {}", groups.len());
    let members: usize = groups.iter().map(Vec::len).sum();
    eprintln!("  memberships                 {members}");
    eprintln!(
        "  mean group size             {:.2}",
        members as f64 / groups.len().max(1) as f64
    );
    eprintln!(
        "\ncoverage audit — commonest English verbs (12dicts bands 1..={})",
        p.common_bands
    );
    let n = r.common_verbs.len().max(1);
    eprintln!("  common verbs (lemmatised)   {}", r.common_verbs.len());
    eprintln!(
        "  reached by synonymy         {} ({:.1}%)",
        r.hits_synonymy,
        100.0 * r.hits_synonymy as f64 / n as f64
    );
    eprintln!(
        "  reached after gap-fill      {} ({:.1}%)",
        r.hits_gap_filled,
        100.0 * r.hits_gap_filled as f64 / n as f64
    );
    eprintln!(
        "  reached after game groups   {} ({:.1}%)",
        r.hits_total,
        100.0 * r.hits_total as f64 / n as f64
    );
    eprintln!(
        "  …of which ONLY the games reach ({}): {}",
        r.game_only.len(),
        r.game_only.join(" ")
    );
    eprintln!("\nwidest game-derived groups kept:");
    for (stories, g) in &r.game_widest {
        eprintln!("  [{stories} stories] {}", g.join(" "));
    }
    eprintln!(
        "\ngame-derived sets refused for being wider than {} ({}):",
        p.game_group_cap,
        r.game_oversize.len()
    );
    for (stories, g) in r.game_oversize.iter().take(10) {
        eprintln!("  [{stories} stories, {} members] {}", g.len(), g.join(" "));
    }
    eprintln!("\nwords in the most groups (polysemy check):");
    for (w, n) in &r.widest {
        eprintln!("  {w} ({n})");
    }
    eprintln!("\nstill unreachable ({}):", r.misses.len());
    for chunk in r.misses.chunks(12) {
        eprintln!("  {}", chunk.join(" "));
    }
}
