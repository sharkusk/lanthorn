# Test fixtures

These fixtures **are tracked in git** — `git ls-files crates/zvm/tests/fixtures`
lists every binary below. (An earlier version of this file said they were
"not committed — gitignored"; that was true once, stopped being true, and the
line was never updated — SQ-1421.) `crate::fixtures::load(name)` still returns
`None` when a name is absent, so a fixture-backed test degrades to a vacuous
skip rather than a compile error if one is ever removed — but nothing here is
gitignored today. All are freely redistributable: the six Z-machine story
files come from the [IF Archive](https://ifarchive.org), whose submission
policy requires everything it hosts to be freely distributable, and the two
`interop/*.qzl` saves and `etude.dfrotz.txt` transcript are ours, generated
from those stories by a reference interpreter and committed for provenance
(see below — regenerating them needs no external download, just `dfrotz`).

## Story files

| file | purpose | source | sha256 | bytes |
|------|---------|--------|--------|-------|
| `czech.z5` | CZECH opcode regression suite (primary acceptance oracle, Task 16) | <https://www.ifarchive.org/if-archive/infocom/interpreters/tools/czech_0_8.zip> (unzip, extract czech.z5) | `9f7e01b9…1ec1bc882b5a` | 13,312 |
| `praxix.z5` | Praxix arithmetic/edge-case checker (Task 16) | <https://ifarchive.org/if-archive/infocom/interpreters/tools/praxix.zip> (unzip, extract praxix.z5) | `bef3bdc2…1347c21b48ac` | 31,744 |
| `minizork.z3` | small real v3 game (smoke tests, save interop, Tasks 5/6/15) | <https://ifarchive.org/if-archive/infocom/demos/minizork.z3> | `c74f01a2…2c69e31ea4a6` | 52,216 |
| `etude.z5` | TerpEtude interpreter exerciser (Andrew Plotkin, Release 2) — all 14 menu options driven (`etude_preload.rs` option 12/SQ-1419, `etude_sections.rs` the other 13/SQ-1421) | <https://ifarchive.org/if-archive/infocom/interpreters/tools/etude.tar.Z> (`.tar.Z` — `uncompress` or `gzip -d` then `tar xf`, extract `etude/etude.z5`) | `bfa2ef69…bb1fb0daface` | 16,896 |
| `gntests.z5` | Graham Nelson's Z-Spec InputCodes/Fonts/Accents/Colours/Header/TimedInput test programs — all six sections driven (`gntests_input_codes.rs`/SQ-1423 at the `zvm-cli` layer, `gntests_sections.rs`/SQ-1421 at the core `zvm` layer) | <https://ifarchive.org/if-archive/infocom/interpreters/tools/etude.tar.Z> (`gzip -dc \| tar xf -`, extract `gntests.z5` — same archive as `etude.z5`, a second file in it) | `56e483ab…4e3d49c545a6` | 7,168 |
| `strictz.z5` | every `@jin`/`@get_child`/`@get_parent`/`@get_sibling`/`@get_prop_addr`/`@get_prop`/`@clear_attr`/`@set_attr`/`@test_attr`/`@insert_obj`/`@remove_obj`/`@get_next_prop` opcode's object-0 edge case (SQ-1421, `regression.rs`) | <https://ifarchive.org/if-archive/infocom/interpreters/tools/strictz.z5> (source `strictz.inf` alongside it, same directory) | `2a15122e…26d6a52d1cfb` | 4,096 |
| `curses.z5` | "Curses" (Graham Nelson, 1993), a real parser game with rooms and its own `save`/`restore` verbs — the second story for `save_interop.rs`'s cross-interpreter matrix (SQ-1421), covering a story that isn't a synthetic opcode-suite/menu exerciser | <https://ifarchive.org/if-archive/games/zcode/curses.z5> | `330100bf…4be4f14fe7da` | 259,072 |

(sha256 truncated in the table for width — the full digests are below, one
`shasum -a 256` line per file, so a fixture can be verified with a single
diff against this block: `shasum -a 256 crates/zvm/tests/fixtures/*.z5 crates/zvm/tests/fixtures/*.z3`.)

```
9f7e01b94353798e1eb8c3b4521f06db4c830a6120f5b3ab7f0d1ec1bc882b5a  czech.z5
bef3bdc2543cc7161833062855aa9bb1682db9eca69d9d36e9101347c21b48ac  praxix.z5
c74f01a232e8df4b05d7ebcba14870143f49b3c9a25f194f7a7d2c69e31ea4a6  minizork.z3
bfa2ef69f2f5ce3796b96f9b073676902e971aedb3ba690b8835bb1fb0daface  etude.z5
56e483ab0049311ce26b4b8639d2c6f8976ad0e7058113099a8e4e3d49c545a6  gntests.z5
2a15122eded657266ea2df07880663c7462d20f263170ede655826d6a52d1cfb  strictz.z5
330100bf1c1ab8ed64065ceb58145ebb87cb6faef5b331fbc6684be4f14fe7da  curses.z5
```

`czech.z5`/`praxix.z5` are driven by `crates/zvm/tests/regression.rs`
(`czech_reports_no_failures`, `praxix_reports_no_failures`,
`strictz_reports_all_correct`). `minizork.z3` also backs
`story_location_verify.rs`, `dict_custom_alphabet.rs` and others — search
`zvm::fixtures::load("minizork.z3")` for the full list.

## `interop/` — cross-interpreter save-format goldens

Reference-interpreter-produced Quetzal saves, checked in so `save_interop.rs`
can prove both directions (zvm reads dfrotz's save; dfrotz reads zvm's save)
without any external binary for the READ direction, and — since SQ-1421 —
without one for the WRITE direction either, as long as `dfrotz` happens to be
on the machine (see below; it skips vacuously otherwise). Regenerate both
goldens with `scripts/gen-interop-goldens.sh` (minizork and curses, since
SQ-1421); the exact prefix/probe commands for each are tabulated below too.

**Reference interpreter for every save/transcript in this directory and
`etude.dfrotz.txt`:** `dfrotz` (FROTZ V2.55, Dumb interface; commit
`acf205585a9472d27c07c0fe62da4b8bc89d1ec7`, 2025-02-01; installed via
`brew install frotz` on macOS). `dfrotz -v` prints this banner — cite it in
any future regeneration so a save format drift shows up as a version bump
here, not a silent divergence.

| file | story | point P (verbatim commands) | resulting state | bytes | sha256 |
|---|---|---|---|---|---|
| `minizork-at-P.qzl` | `minizork.z3` | `open mailbox` → `take leaflet` → `north` | room = *North of House*, leaflet carried | 366 | `4bd24ce6…d7afe2d46adf` |
| `curses-at-P.qzl` | `curses.z5` | `east` → `take scarf` | room = *Servant's Room*, scarf carried (alongside the chocolate biscuit/electric torch/crumpled paper the game starts you with) | 544 | `0d66c832…31e21608c9963e8` |

Both were produced with the game's own `save` verb, writing straight to this
path (`printf '<prefix>\nsave\n<path>\nquit\ny\n' | dfrotz -m <story>` —
`-m` disables `dfrotz`'s own `***MORE***` pager, which otherwise consumes
extra lines from a piped script and desyncs it from the intended input).
Probe for both: `look` (reveals the room) + `inventory` (reveals the carried
item) — a broken restore places the player elsewhere or drops the item, so
the cross-load equivalence assertion in `save_interop.rs` cannot pass
vacuously.

### WRITE-direction (`*_save_read_by_dfrotz` tests)

`save_interop.rs`'s `dfrotz_cmd()` resolves a `dfrotz` binary — the `DFROTZ`
env var if it names a file, else a bare `dfrotz` resolved off `PATH` — and
both WRITE-direction tests skip vacuously (printing why, to stderr) when
neither resolves. **This replaced an `#[ignore]` pair** (SQ-1421): an
`#[ignore]`d test needs `-- --ignored` to ever run, which neither the local
gate nor CI passes, so the WRITE direction was untested by anything that
actually runs regularly. A vacuous skip runs — and proves something — on any
machine that happens to have `dfrotz`, with zero special invocation.

## `etude.dfrotz.txt` — TerpEtude reference transcript

A real `dfrotz` transcript (same version as above) of options 1-9, 13 and 14,
recorded for `etude_sections.rs` (SQ-1421) to assert content parity against.
Regenerate:

```sh
dfrotz -m -w 999 crates/zvm/tests/fixtures/etude.z5 < script.txt \
  > crates/zvm/tests/fixtures/etude.dfrotz.txt
```

where `script.txt` is (one input per line, matching `etude_sections.rs`'s own
driving script exactly):

```
1
2
3
4
5
6
7
.
8
x
.
9
Hi There

13
 
.
14
 
```

**Normalisation — why this is a `.contains(...)` fixture, not an
`assert_eq!` one.** `dfrotz`'s dumb interface word-wraps every paragraph to
its own terminal width (`-w 999` here, chosen wide enough that only a few of
TerpEtude's longest paragraphs still wrap); `zvm`'s headless `BufferOutput`
sink never does — it accumulates exactly the bytes the story printed, with no
simulated terminal to wrap against. So `etude_sections.rs` matches on
short(ish) fragments that don't straddle a `-w 999` wrap point, never on
whole lines or the whole transcript. Every other difference between the two
outputs is either dfrotz's own startup banner (`Using normal formatting.` /
`Loading …`, stripped by not being asserted on) or content this file and
`etude_sections.rs`'s own module doc call out explicitly (options 10/11,
timed input, are NOT diffed against this file at all — see below).

**Options 10 and 11 (timed input) are absent from this transcript on
purpose.** Confirmed live while producing it: `dfrotz`'s dumb interface does
not implement real timed-read polling when stdin is piped non-interactively
— selecting option 10 under a piped script and pressing a key immediately
prints `The timing interrupt function was not called at all... This aspect
of your interpreter appears to behave WRONG`, dfrotz's own honest report that
no real second elapsed, not a finding about either interpreter. `zvm`
exposes the timed-read protocol as an API instead
(`Machine::pending_timeout`/`run_timed_interrupt`/`abort_timed_input`),
letting `etude_sections.rs` simulate ticks deterministically without a real
wall-clock wait — which is what the SQ-1014 audit meant by "drive them with
the VM's timed-read protocol, not wall clock". Those two options are
asserted against TerpEtude's own Inform 6 source (`timedch.inc`/
`timedstr.inc`, from `etude.tar.Z`) instead, which is unambiguous about what
each tick prints.

## `ziptest-r12`/`ziptest-r13` — not a fixture here, on purpose

Infocom's own in-house ZipTest regression stories for the YZIP (Version 6)
interpreter (June 1989) are **not present anywhere in this repository** —
not in `crates/zvm/tests/fixtures/`, not in `unit_tests/`, not in `stories/`
— and **no suite drives them**. `docs/internals/ci-fixture-coverage.md`
("Two footnotes worth keeping") has the full finding: they are unlicensed
proprietary Infocom/Microsoft material, not redistributable, named for
`unit_tests/` in that doc as an aspiration rather than as a description of
anything that exists on disk. Were they ever obtained, they belong beside
`stories/` (gitignored, skip-vacuously pattern) — never here, where
`crate::fixtures::load` assumes redistributable content. Testing YZIP/v6
render behaviour against them is `crates/app`'s render-path territory (v6
screen geometry, hybrid/raster drawing), not this crate's opcode-level
conformance suite — so SQ-1421 records the gap rather than closing it.

## `bench/` — the timing harness's script

`bench/minizork.script` drives `cargo run --release -p lanthorn-zvm --example
bench` against `minizork.z3` above; see [`bench/README.md`](bench/README.md) for
what the script does and why, and `docs/internals/performance.md` for the
recorded baselines against `dfrotz` (SQ-1428). No story file lives under
`bench/` — it reads `minizork.z3` from this directory.
