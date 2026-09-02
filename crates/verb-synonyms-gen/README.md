# verb-synonyms-gen

The offline generator behind `crates/verb-synonyms/src/synonym_groups.tsv`.

It exists because of the persistence rule: nothing goes into the tree without
its regeneration inputs, or the table becomes a blob nobody can reproduce when
the corpus grows or the lexical source changes. This crate is a development
tool. It is never linked into `lanthorn`, and it takes no external
dependencies — only the three story readers, so that a checkout that builds the
workspace can rebuild the table.

## The problem it solves

A player types `illuminate lamp`; the story wants `light`. Nothing in the story
file records what `illuminate` *means*, and every mechanism that works on FORM
fails: edit distance is 8 on a 10-letter word, stemming reaches `illuminat-` and
nothing else, and the grammar's shape only narrows to "a verb taking one noun",
which is most verbs. The bridge has to come from outside the story file.

Shipping a general thesaurus and querying it at runtime would be enormous. But
the IF side is BOUNDED — a few thousand verb spellings across every game anyone
has written — so the vocabulary is harvested, WordNet is filtered down to the
synsets that vocabulary actually touches, and the result is shipped as a table
with no runtime dependency and no network.

## The corpus is a thesaurus too

A thesaurus is not the only source, and for this job it is not the best one.
Every story's grammar declares its verbs in GROUPS — `Verb 'examine' 'x'
'watch' 'describe' 'inspect'` is one entry, and it is that game's author saying
those five spellings are one action. The harvest used to flatten that into a
bare word list; now it keeps the grouping, and a set several stories declare
independently is shipped as a group in its own right.

That source is better than WordNet at exactly the job here, because it answers
the parser's question rather than English's. `inspect` is the case that made the
point: WordNet's groups for it are `case`, `visit` and `audit`/`scrutinize` —
the police sense — and not one of them holds `examine`, which is what every
player who types `inspect` means. Fourteen stories in this corpus put the two on
one verb. It is also free (the grammar is already loaded), and it carries no
licence obligation, being read out of behaviour rather than out of a lexicon.

There is **no union across entries**. A group is one verb entry as some author
wrote it; identical entries from different stories are pooled and counted, and
the count is all the merging there is. Taking every pair of spellings that ever
shared a verb and closing it transitively would run one chain through a light
verb like `get` or `turn` and collapse half the table into a single group.

Where the two sources disagree, the games win — by line ORDER, not by deletion.
A game-derived group is placed ahead of every WordNet group holding the same
word, so a consumer walking a word's groups meets it first; WordNet's answer
still follows, one line lower, and still reaches a story that implements a word
no game in the corpus grouped.

## Sources

Neither is vendored; both are downloaded by `fetch-sources.sh`, which pins the
exact releases by SHA-256.

| source | version | what it supplies |
|---|---|---|
| [WordNet](https://wordnet.princeton.edu/) `dict/` | **3.0** (2006), `WordNet-3.0.tar.gz`, sha256 `640db279…d3a52` | synonymy (`data.verb`, `index.verb`), the hypernym pointer graph, and the irregular inflections of verbs (`verb.exc`) and nouns (`noun.exc`) |
| [12dicts](http://wordlist.aspell.net/12dicts/) `Lemmatized/2+2+3frq.txt` | **6.0.2** (June 2016), `12dicts-6.0.2.zip`, sha256 `64ac1d35…780e52` | the frequency ranking (21 bands, commonest first) and a lemmatisation map, headword at column 0 with its inflected and derived forms indented |

Licences: both are permissive and compatible with lanthorn's BSD-3-Clause, and
both require a notice to travel with derived data. That notice is
`THIRD-PARTY-NOTICES.md` at the repository root. Read it before swapping either
source.

Why 12dicts and not a web-scale frequency list: the obvious candidates are not
usable. `first20hours/google-10000-english` says outright "I do not recommend
using this data for commercial purposes without licensing it from the Linguistic
Data Consortium"; `hermitdave/FrequencyWords` is MIT for its code but CC-BY-SA
4.0 for its content, and share-alike on a derived database is not a thing to
take on casually. 12dicts' chain — Beale → AGID → Moby (public domain), ENABLE2K
(public domain), WordNet — is permissive the whole way down, and it has the
property the others lack: it is already **lemmatised**, so the frequency ranking
and the inflection map come from one file.

## Rebuilding

```sh
./crates/verb-synonyms-gen/fetch-sources.sh /tmp/verbsyn      # ~22 MB of downloads

# Step 1 — the IF vocabulary AND the corpus's own verb entries. Needs a corpus
# of story files; both outputs are COMMITTED (if_verbs.tsv, if_groups.tsv), so
# step 2 is reproducible without one.
cargo run -p verb-synonyms-gen -- harvest \
    --corpus stories --corpus unit_tests \
    --wordnet /tmp/verbsyn/WordNet-3.0/dict \
    --freq /tmp/verbsyn/12dicts/Lemmatized/2+2+3frq.txt \
    -o crates/verb-synonyms-gen/if_verbs.tsv \
    --groups crates/verb-synonyms-gen/if_groups.tsv

# Step 2 — the shipped table.
cargo run -p verb-synonyms-gen -- build \
    --wordnet /tmp/verbsyn/WordNet-3.0/dict \
    --freq /tmp/verbsyn/12dicts/Lemmatized/2+2+3frq.txt \
    --if-verbs crates/verb-synonyms-gen/if_verbs.tsv \
    --if-groups crates/verb-synonyms-gen/if_groups.tsv \
    -o crates/verb-synonyms/src/synonym_groups.tsv

# Step 3 — the irregular inflections. Independent of the other two: it reads no
# corpus and no frequency list, because `lit` is `light` whoever is playing.
cargo run -p verb-synonyms-gen -- irregulars \
    --wordnet /tmp/verbsyn/WordNet-3.0/dict \
    -o crates/verb-synonyms/src/irregular_forms.tsv

cargo nextest run -p verb-synonyms   # the canonical mappings must survive
```

Leaving `--if-groups` off builds the WordNet half alone, which is how the two
sources are measured apart.

Both steps print a report to stderr. The number to look at is the coverage
audit: what fraction of the commonest English verbs (12dicts bands 1–11,
lemmatised and reduced to those WordNet knows as verbs) reach a surviving group.
That is the quality metric for the whole exercise — far more meaningful than the
row count — and it is what tells you whether a change to the filters helped.

### When the corpus grows

Three things, in order. Re-run the **harvest** above into scratch files and diff
them against the committed `if_verbs.tsv` / `if_groups.tsv`: the new lines are
the verbs and verb entries the tables have never seen, and a group whose count
has climbed past `--game-support` is one the shipped table would now believe.
Re-run the **build** into a scratch file and diff that against
`crates/verb-synonyms/src/synonym_groups.tsv` — the number to read is the
coverage audit on stderr, not the row count. Then look at what the new stories
actually get offered:

```sh
cargo run -p app --example guidance_scan          # stories/ + unit_tests/
cargo run -p app --example guidance_scan -- --only curses.z5,vespers.z8 --json
```

The harvest diff answers "which verbs are missing"; `guidance_scan` answers
"are the offers we already make any good" — it drives the real vocabulary offer
and its shadow-probe vetting over every story it can read and prints each
suggestion with its verdict. A wrong offer (`shove` → `pull · drag`) is a
line-order or sense problem in this table; a silent story is usually a grammar
this generator could not read at all, and the harvest's own skip report names it.

### The knobs

`build` takes `--sense-cap`, `--band-cap`, `--group-cap`, `--hyponym-cap`,
`--common-bands`, `--no-gap-fill`, `--game-support` and `--game-group-cap`; the
defaults are in `Params::default` and each is documented where it is declared.
They are on the command line so that a retune can be argued with rather than
recompiled.

Measured for `--game-support` (the number of stories that must declare a verb
entry before it is believed), on the 1,365-verb basis:

| support | game groups kept | rows | table | coverage |
|---|---|---|---|---|
| — (WordNet only) | 0 | 2,759 | 74 KB | 88.8% |
| 1 | 1,425 | 3,463 | 90 KB | 90.5% |
| **2** | **546** | **3,068** | **81 KB** | **90.0%** |
| 3 | 306 | 2,946 | 78 KB | 89.5% |

One story is one author's idiom: at support 1 the corpus contributes a
33-member `attack` group carrying `vandalise` and `torture`, and a 21-member
`cut`. Two is where those disappear and the survivors are IF conventions.

This table predates the four SQ-1233 rules below and is left as measured —
it is what argues for the KNOB, and none of the four rules moves it: at
`--game-support 2` the row count they leave the table at is 3,099 (79 KB),
546 game groups kept (unchanged — the rules reorder and prune MEMBERS, they
do not change which verb entries clear the threshold), and coverage 89.7%,
down three tenths of a point from dropping some bystander memberships that
happened to be a common verb's only channel into the table. That is the
measured cost of correctness here, not a regression to chase back up.

### Four more rules (SQ-1233)

A 30-story guidance-scan audit found four systematic ways the table (and the
mechanisms above) still misled a player. All four are in `build.rs`, and none
of them is a list of words — every one reads its answer out of the harvest.

1. **Order game-derived groups by support.** Two game-derived groups sharing a
   member used to break their tie arbitrarily (alphabetically, in practice):
   `shove` offered `pull · drag` (the `pull/drag/tug/yank/shove` group, 4
   stories) before `push · nudge` (`push/press/stick/thrust/shove/nudge`, 3
   stories) even though a THIRD, narrower group — `press/push/shove` — has 5.
   That third group was also the deeper half of the bug: the subsumption step
   (above) swallowed it into the 3-story six-member set purely for being
   smaller, discarding the stronger evidence entirely. Both halves are fixed
   together: subsumption between two GAME groups now requires the wider set to
   be at least as well supported as the narrower one it would eat (`build.rs`,
   the "Drop groups another group already contains" step), and
   `order_by_sense`'s tie-break among a word's game-derived groups is support,
   descending, ties kept in file order. A companion fix in `keep` matters here
   too: two raw entries that reduce to the SAME final members after
   filtering used to remember whichever processed first regardless of its
   support; now the higher one wins, so `order_by_sense` reasons from the
   corpus's true belief in a set rather than an accident of file order.
2. **Drop WordNet senses no parser wants.** `illuminate` was pulling `clear`
   into its "clarify" sense (`clear up / elucidate / illuminate / …`) even
   though the corpus's OWN, heavily-corroborated sense of `clear` is
   "push/move aside" (24 stories) and has nothing to do with clarifying. The
   rule: a member is a "bystander" in a synset if that synset was not the
   reason WordNet counts the word as an IF verb in the first place — its own
   sense rank for this offset falls outside `sense_cap` — which is true of
   `clear` here (its top sense is `push`; "clarify" is its 10th) and false of
   `light` in the neighbouring "illuminate" group (that group IS `light`'s
   #1 sense). A bystander is dropped only if the corpus ALSO corroborates a
   different action for it that shares nothing with the rest of the synset —
   so a word with no corpus opinion, or one whose corpus entries actually
   overlap the synset, is left alone. See `Report::bystanders_dropped`.
3. **Rank the canonical parser verb first.** `inspect` was offering
   `watch · check · examine` — `examine` last, because member order used each
   spelling's OVERALL if_verbs.tsv popularity, and `watch` is IF's more common
   verb across every sense it has, most of them nothing to do with inspecting.
   Members are now ordered first by how much of the corpus's OWN evidence for
   THIS group backs each spelling — the sum of every if_groups.tsv
   declaration (any support level; a single story is still real evidence of a
   ranking, if not of a group's existence) that names the spelling alongside
   one of the group's other members — falling back to the old overall-count
   tiebreak only where that is zero (a WordNet-only group, where no
   if_groups.tsv entry ever names a member like `light up`). This is also what
   makes `find` disappear from `obtain`'s `get/find/incur/receive` group
   without a special case: `find`'s own dominant corpus sense is "search"
   (`find/seek`, a dozen-plus stories, sharing nothing with `get`/`obtain`), so
   rule 2 removes it as a bystander before ordering ever sees it.
4. **Derive `un-X`.** `unmask`, `unpin` and `unzip` reached no group at all —
   too rare to pass `--game-support` on their own, and WordNet has no synset
   relating an English verb to its `un`-form (that is a live morphological
   rule, not a fact any lexicon states). A new pass, `derive_reversals`, gives
   every `un`-prefixed spelling that reaches NOTHING through every earlier
   pass one of two homes: the corpus's own raw declaration for the `un`-word
   itself, at ANY support level (`unpin` reaches `unblock`/`uncover`/`unplug`
   this way, one story, the reversal cluster a game author actually wrote);
   or, failing that, a minimal pairing with its bare base verb (`unmask` with
   `mask`), so the spelling is at least resolvable. The base must itself be a
   known verb (an IF verb or a WordNet lemma) at least `MIN_REVERSAL_BASE`
   (3) letters long — short enough to exclude only light verbs like `do`/`go`,
   whose "reversal" means nothing, and specifically what keeps this pass from
   ever touching `undo` (base `do`, 2 letters). A word that already reaches a
   group through an earlier pass — `unhook`, corroborated normally — is left
   untouched.

## The second table

`irregular_forms.tsv` is the other half of English morphology, and it is a
straight copy of WordNet's exception lists rather than anything derived. The
consumer, `app`'s `vocab::stems`, reaches every REGULAR inflection with a rule —
strip `ing`, `ed`, `es`, `s`, put back the letter the spelling dropped — and
cannot ever reach the irregular ones, because `lit` shares no letters with the
ending that would have made it from `light`. Nor can the near miss: `lit` is two
keystrokes away and that threshold is one, deliberately.

Two things about it differ from the synonym table and are worth stating.

**It is SORTED, and that is safe here.** Nothing about an irregular form's bases
is ranked — they are alternative readings of one spelling, and only the story's
own dictionary can choose between them — so the file is sorted to be greppable
and to diff cleanly. `synonym_groups.tsv` must never be, because its line order
carries WordNet's sense ranking.

**It carries NOUNS as well as verbs.** `vocab::stems` is asked about every
position in a command, not only the opening word, so `mice` → `mouse` is the
identical case to `lit` → `light` one slot to the right. The two exception lists
are read into two maps and never merged: a form can inflect two parts of speech
to two different lemmas, and one map would have to drop one of them.

## Why `if_verbs.tsv` and `if_groups.tsv` are committed

One is a sorted list of ordinary English verb spellings with a count of how many
stories accept each — `take 118`, `xyzzy 9`; the other is a sorted list of the
verb entries those stories declare, with a count of how many declare each —
`29 awake awaken wake`. Neither carries game text, game titles or any
attribution of a word to a story: the per-story detail exists only in the
harvest's stderr report. `stories/` is gitignored because it holds commercial
game files; a de-duplicated vocabulary list drawn across 119 of them is not one,
and committing both is what makes step 2 — and therefore the shipped table —
reproducible by CI and by anyone without the corpus.
