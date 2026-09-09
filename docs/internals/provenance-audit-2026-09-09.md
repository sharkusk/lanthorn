# Provenance audit — 2026-09-09 (SQ-1444)

lanthorn is **BSD-3-Clause** (`LICENSE`, `Copyright (c) 2026, Marcus Kellerman`;
`license = "BSD-3-Clause"` in the workspace `Cargo.toml`, inherited by every
crate). The user's rule, set 2026-09-09: **no code may be derived from GPL
sources.** This document audits every citation of another interpreter in the
engine crates, establishes the licence of each cited project, classifies each
citation, and names what must be re-derived.

This is a **reporting** document. It changes no code. The remediation is filed as
the quests listed at the end.

## Method

Every mention of `frotz`, `bocfel`, `scottfree`, `gargoyle`, `garglk`, `glulxe`,
`cheapglk`, `glkterm`, `scottkit`, `remglk`, `quixe`, `nitfol`, `fizmo` and
`jzip` under `crates/*/src`, `crates/*/tests` and `docs/internals` was collected
and read **in context** — the sentence, not the keyword. The bare grep reports
2,043 hits, but 694 of those are the word "zoom" (the map zoom UI: `zoom-map in`,
`Zoom` in `state.rs`), which has nothing to do with the Zoom interpreter and is
excluded throughout. `scottkit`, `remglk`, `nitfol`, `fizmo` and `jzip` have
**zero** hits.

Two structural checks were run first, because they bound the whole problem:

* **No third-party source is vendored.** `find` for `*.c`, `*.cpp`, `*.h`
  outside `target*` returns **nothing**. The only foreign source text in the
  repository is quoted inside three Rust doc comments (all in `crates/scott`,
  all listed below).
* **Line-number citations are the strongest signal of source-reading**, so
  every `file.c:NNN` form was enumerated: **144 repo-wide, 121 of them in
  `crates/scott`.** That concentration is the audit's central finding.

## 1. Licence of every cited project

Fetched and read 2026-09-08. Where a project is not one licence, the path is
given, because the licence of the *file we cite* is what governs.

| Project / path | Licence | Licence text read at |
|---|---|---|
| **Frotz** (repo root `COPYING`) | **GPL-2.0-or-later** | `raw.githubusercontent.com/DavidGriffith/frotz/master/COPYING` |
| Frotz `src/common/text.c`, `src/common/fastmem.c`, `src/dumb/dumb_init.c` | **GPL-2.0-or-later** (per-file headers) | same repo, each file |
| **ScottFree** (`cspiegel/scottfree-glk`) | **GPL-2.0** | repo README — "supplied subject to the GNU software copyleft (version 2)" |
| **Gargoyle** top level (`License.txt`) | **mixed/aggregate** — "Parts of this package are distributed under the terms of the GNU General Public License" | `raw.githubusercontent.com/garglk/garglk/master/License.txt` |
| **`garglk/window.cpp`**, **`garglk/wintext.cpp`**, `garglk/winmask.cpp` | **GPL-2.0-or-later** (per-file headers; the directory has no licence file of its own) | each file's header block, garglk `master` |
| Gargoyle `terps/scott/` (`scott.c` header) | **GPL-2.0-or-later** | file header; no directory licence file |
| Gargoyle `terps/glulxe/LICENSE` | **MIT** (Plotkin, 1999–2023) | that file |
| **Gargoyle `terps/bocfel/LICENSE`** | **MIT** (Chris Spiegel, 2009–2025) | that file; `dict.cpp` carries `SPDX-License-Identifier: MIT` |
| **Bocfel ≤ v2.2.2** (historic) | GPL-2.0/3.0 dual from v0.6.1 (2012-02-27) | `cspiegel.github.io/bocfel/downloads.html` changelog |
| **Bocfel ≥ v2.2.3** (2025-02-01 onward) | **MIT** — "Bocfel is now under the MIT license" | same changelog |
| **glulxe** | **MIT** (Plotkin) | `raw.githubusercontent.com/erkyrath/glulxe/master/LICENSE` |
| **cheapglk** | **MIT** (Plotkin) | `.../erkyrath/cheapglk/master/LICENSE` |
| **glkterm** | **MIT** (Plotkin) | `.../erkyrath/glkterm/master/LICENSE` |
| **remglk** / **Quixe** | **MIT** (Plotkin) | respective `LICENSE` files (not cited by our code) |
| **ScottKit** | GPL-2.0 | repo page (not cited by our code) |
| **Glk spec** | freely implementable — "anyone can write a Glk library… You naturally retain the copyright on any software you write" | `eblong.com/zarf/glk/freeware.html` |
| **Glulx spec** | same policy inferred (same author, same site); no Glulx-specific grant located | `eblong.com/zarf/glulx/` |
| **Z-Machine Standards Document** | **no explicit copying or permission clause found** in the preface or index | `inform-fiction.org/zmachine/standards/z1point1/preface.html` |

### The two licence facts that decide most of this audit

1. **Bocfel has been MIT since v2.2.3 (2025-02-01).** Every Bocfel read in this
   repository postdates that: the repository's first commit is **2026-06-18**,
   `font3_translate` landed **2026-06-30**, the `ega_colormap` citation
   **2026-08-12**. So **no Bocfel citation in lanthorn is a GPL citation.** This
   moves the font-3 table and the encoder citations from "must rewrite" to
   "permissive — record and attribute".
   *Caveat:* Gargoyle's own top-level `License.txt` still lists Bocfel as GPL.
   That text is stale relative to `terps/bocfel/LICENSE`, which is MIT. We rely
   on the per-file/per-directory licence, which is the governing one.
2. **`garglk/window.cpp` and `garglk/wintext.cpp` are GPL-2.0-or-later**, read
   from their own header blocks. The "portions of Gargoyle are MIT" fact is
   true — `terps/glulxe` and `terps/bocfel` are MIT — but it does **not** extend
   to the `garglk/` Glk library itself, which is exactly where our three
   Gargoyle citations point. Those three are therefore GPL citations and are
   judged individually below.

### Also cited, outside the brief's list

| Project / path | Licence | Cited at |
|---|---|---|
| **inform6lib** `english.h`, `verblib.h` | **dual: traditional Inform/DM4 licence OR Artistic-2.0**, implementer's choice (per-file headers, Nelson 1993–2004 + Griffith 2012–2018) | `crates/zvm/src/world.rs:552-570` |
| Spatterlight (top level) | GPL-3.0 | — |
| Spatterlight `terps/bocfel/LICENSE` | MIT (the vendored bocfel tree) | — |
| **Spatterlight `terps/bocfel/z6/draw_border.cpp`** | **GPL-2.0-or-3.0** — its own header, "Copyright 2010-**2026** Chris Spiegel", a deliberate current choice, *not* a stale pre-relicence snapshot | `crates/app/src/graphics.rs:1995` |
| **Spatterlight `terps/bocfel/z6/draw_image.cpp`** | **no licence header at all** — no SPDX, no copyright; ambiguous between the parent MIT `LICENSE` and the `COPYING.GPLv2`/`COPYING.GPLv3` sitting beside it | `crates/blorb/src/infocom_pics.rs:341`, `crates/app/src/graphics.rs:828` |
| ttyd / dtach (`protocol.c`, `pty.c`, `master.c`) | not audited — no code derived | `docs/internals/docker.md:151-158` (prose describing an external tool's signal behaviour; lanthorn links neither) |

**The `z6/` subdirectory is the one place where "portions of X are MIT" actively
misleads.** Its parent directory's `LICENSE` is MIT, but `draw_border.cpp`
carries an explicit GPL-2-or-3 header of its own, and `draw_image.cpp` carries
nothing. Treat both as GPL until a maintainer says otherwise — and note that
`ega_colormap` itself, in `draw_image.cpp`, cites *Wikipedia's Enhanced Graphics
Adapter article* as its origin, so the table is a hardware fact in that file too,
not that file's invention. Our `EGA_PALETTE` reached the same sixteen entries
through four witnesses (§7), so nothing turns on it — but the citation should
stop pointing at a GPL file.

**inform6lib is not a problem.** Artistic-2.0 is available at the implementer's
choice, and `world.rs` took no constants from it in any case.

## 2. Classification

* **(a) SPEC / standard / file format** — fine.
* **(b) Permissive source read for design** — fine; licence recorded, attribution owed.
* **(c) GPL source cited for BEHAVIOUR** observed by running the binary or reading its documentation — fine.
* **(d) GPL source cited for CODE** — a function name, a line number, an algorithm "ported from", a table "taken from". **Flagged.**

### Counts

| Class | Distinct sites | Where |
|---|---|---|
| (a) spec / format / API constants | **~180** | Glk & Glulx spec cites, `garglk_set_zcolors` selector numbers, Quetzal, Blorb chunk names, the `garglk.ini` config format (~140 of these are `garglk.ini` alone) |
| (b) permissive read (Bocfel MIT, glulxe/cheapglk MIT, inform6lib Artistic-2.0) | **~28** | `zvm/src/text/encode.rs`, the font-3 table, `gvm/src/exec.rs` float ops, `docs/internals/gvm-fileref-seam.md` (cheapglk `cgfref.c`) |
| (c) GPL behaviour, black-box or documented | **~70 distinct** (~120 raw, incl. golden-test repeats) | `zvm/src/cpu/exec.rs` (~50 Frotz), `zvm/src/screen.rs`, `scott` message wording, every `dfrotz` parity fixture |
| **(d) GPL source cited for code** | **~76 raw sites, resolving to 9 findings** | see below |

**`crates/gvm/**` outside `exec.rs`/`glk.rs` contains zero GPL mentions at all** —
every hit is glulxe, cheapglk or Quixe, all MIT.

### The (d) population splits in two, and the distinction is the whole remediation plan

The raw count of ~76 flagged sites is dominated by *citation hygiene*, not
derivation. Sorting them by what a fix actually requires:

* **(d-i) Reproduced GPL source text, or a ported algorithm — REWRITE.**
  **Three** findings: D1, D2, D7. These are the only places where lanthorn's
  code cannot be defended as an independent implementation.
* **(d-ii) A GPL file, function or line cited as the authority for a fact
  lanthorn had another route to — RE-WORD.** The remaining ~70 sites. The code
  is independently written (often demonstrably so, with its own oracle); the
  defect is that a comment records a GPL source as the origin. These are real
  and must be fixed, but they are a documentation pass, not a re-implementation.

Judging each of the ~76 individually is what separates these; the nine findings
below are the result.

## 3. The (d) findings, judged

Ordered by severity.

### D1 — `crates/scott/src/loader.rs:204-217`, `extract_auto_noun` — **rewritten from spec §2.5 (SQ-1445)**

The doc comment says "porting ScottFree 1.14's item-load loop **verbatim**
(`ScottCurses.c:319-327`)" and then reproduces **ten lines of ScottFree's GPL C
source in a fenced block**, including the original's own comment
(`/* Some games use // to mean no auto get/drop word! */`). The Rust body is a
direct transliteration of that block: same first-`/` search, same two
whole-string comparisons against `"//"` and `"/*"`, same "find the next `/`,
tolerate `t==NULL`" tail, in the same order.

This is the audit's worst finding on both counts — it is the only place where
GPL source *text* is reproduced in the repository, and the surrounding code is a
port rather than an independent implementation. **Must be re-derived, and the C
block must go.**

### D2 — `crates/scott/src/loader.rs:153-166`, `next_str` / `ReadString` — **rewritten from spec §2.6 (SQ-1447)**

"porting ScottFree 1.14's `ReadString` (`ScottCurses.c:189-224`) **byte
rule-for-rule**", including a note that the doubled-quote test is checked
*before* the backtick substitution "matching the source's order". No C is
quoted, but the comment asserts order-for-order correspondence with GPL code,
and the function implements exactly that order.

Mitigating: the underlying rules (backtick → `"`, doubled `""` → literal `"`)
are **facts about the `.dat` file format** and are observable from any `.dat`
file plus ScottFree's output. The two documented *additions* (`\r` dropped,
non-ASCII → `?`) are lanthorn's own. So the substance is re-derivable cheaply;
what must go is the claim of, and the reliance on, rule-for-rule
correspondence.

### D3 — `crates/scott/src/scottfree_save.rs:10-18` — **RE-WORD (code is clean)**

Seven lines of ScottFree's `LoadGame` `fscanf` calls quoted verbatim to document
the save format's field order.

The Rust reader is **not** a port: it is a `SplitAsciiWhitespace` tokenizer
(`Tokens`) that shares no structure with `LoadGame`. And a save-file layout read
by a third party is an **interoperability fact** — the whole point of the module
is importing a user's existing `.sav`. The finding is confined to the quoted C:
the same field order can be stated in prose, or derived from a `.sav` file
ScottFree wrote. **Delete the block, keep the code.**

### D4 — `crates/scott/src/options.rs:8-13` — **RE-WORD (code is clean)**

Four lines of ScottFree's `main` argument switch quoted verbatim
(`case 'y': Options|=YOUARE; break;` …).

`Options` is a Rust struct of four bools on the `Vm`, deliberately *not* the
process-global bitmask ScottFree uses — the module doc says so. Nothing is
ported. The quoted `case` lines are near-*de minimis*, but they are still
literal GPL source, and the same fact ("`-y` selects second-person wording") is
in ScottFree's own usage message. **Delete the block, keep the code.**

### D5 — `crates/scott/src/{vm,options,loader,scottfree_save,lib}.rs` — 92 `ScottCurses.c:NNN` citations — **RE-WORD**

`vm.rs` 43, `options.rs` 42, `loader.rs` 3, `scottfree_save.rs` 3, `lib.rs` 1,
plus 23 more in `tests/scottfree_parity.rs` and 4 in `tests/golden.rs`.

Read in bulk, **the overwhelming majority annotate observable behaviour, not
structure** — and in `options.rs` they are almost entirely **message wording**:
`"O.K. "`, `"I'm carrying:\n"`, `"I am dead.\n"`, `"What ? "`, `"Light runs out
in "`. Those strings are what a Scott Adams `.dat` was authored and tested
against; they are recoverable in full by *running* ScottFree, and reproducing
them is required for parity. The **facts** are fine.

The problem is evidentiary, and it is real: a file:line citation into GPL source
is a written record that the source was read, attached to code that had another
route to the same fact. It invites exactly the derivation claim the user's rule
exists to prevent. **Re-point every one of these at the observable behaviour —
the parity fixture, the transcript, ScottFree's usage message — or at the
Swansea Definition where it is not silent.**

### D7 — `crates/app/src/render/v6_border.rs` — Spatterlight bocfel `z6/draw_border.cpp` (GPL-2-or-3) — **REWRITE `extend_pillars`**

The second-worst finding, and the one the brief did not anticipate, because it
hinges on the licence split inside Bocfel that §1 uncovered: **every other
Bocfel citation in lanthorn is MIT, but `z6/draw_border.cpp` is explicitly
GPL-2.0-or-3.0** by its own header, copyright through 2026. This module cites
that file, and says plainly what it did with it:

* `:13-16` — "This module TILES instead, **the way Spatterlight's Bocfel does**
  (`terps/bocfel/z6/draw_border.cpp`) … **Read for MECHANISM**, not policy".
* `:23` — "plus **a port of Bocfel's** [`extend_pillars`]".
* `:872` — "Bocfel's `extend_pillars()`, **ported**: capital → tiled shaft → foot".
* `:880-884` — "**The ordering caveat is Bocfel's own, and it is not optional**:
  snapshot the repeat unit BEFORE erasing the foot."
* `:862` — `erase_below` glossed as "Bocfel's `erase_lines_in_bitmap`".
* `:887` and `:913` — two quoted C fragments
  (`if (is_spatterlight_arthur) foot_top -= (foot_top & 1);` and
  `bool initial_parity = flip;`).

**What is already clean, and it is most of it.** The constants were the larger
exposure and they have *already been remediated*, by SQ-1063's refactor rather
than by this audit: `:481-483` and `:594-599` record that the three section
boundaries "are not lost, they are **MEASURED**" off Infocom's own artwork —
`zork0.mg1`'s castle border yields a top cut of 82 and a 26-row foot where "the
constants said 86 and 26". An independent measurement that reproduces the
borrowed numbers is the strongest possible answer to a derived-constants claim,
and it is already in the tree. The module also documents a **deliberate
divergence** from Bocfel at `:886-893` (it does not nudge Arthur's foot onto an
even row, and explains why lanthorn cannot), which is evidence of engineering
rather than transcription.

**What remains** is the named port itself. `extend_pillars` is a generic raster
operation — snapshot a repeat unit, tile it downward, stamp a foot, erase below
— and lanthorn has an independent reason to want each step. But it is described
twice as a port of a named function in a GPL file, with that file's ordering
constraint carried across as a stated requirement, so it cannot stand as
written. One surviving constant, `INSET = 4` at `:590-591` ("Bocfel's two raw
rows at each end, doubled"), needs re-sourcing too — though `:598` already notes
the measurement corroborates it ("86 is 82 plus this inset").

The two quoted C fragments must go regardless.

### D8 — `crates/blorb/src/infocom_pics.rs` — Frotz `src/dos/bcpic.c` (GPL) — **NOT DERIVED; re-word (12 sites)**

The largest single (d) cluster after `v6_border.rs`, and on inspection the least
alarming of the big three. `:86-87` says "**Every LZW constant here is quoted
from** [Frotz's `bcpic.c`] below", and `:1710-1721` reproduces a **verbatim block
of Frotz's comment prose** describing the codec. There is also a quoted x86
fragment (`xor ah,1; ror ah,1`) and the `ega_colormap` / `draw_image.cpp:58`
citations already discussed.

**Verdict: independent implementation; the quoted prose must go.** The decisive
sentence is lanthorn's own, one line above the quote: "This is **GIF's**
variable-width LZW with the minimum code size fixed at 8: 256 clears the table,
257 ends the stream, assignable codes start at 258, and the code width runs 9 to
12 bits… packed least-significant bit first." Every "Frotz constant" in that
list is a **GIF89a specification** constant. Frotz did not invent them and
neither did we; Frotz's comments merely *explain* them, which is why they were
quoted.

And the module does not rest on Frotz at all. `:88-93` names its real authority:
an **oracle** — Zork Zero's MCGA archive and its Amiga `Pic.data` carry the same
artwork, and "for all 383 pictures whose two directories agree on dimensions the
LZW output is **byte-for-byte identical**". A decoder validated against 383
independently-encoded pictures is not a transcription of anyone's decoder.

Fix: delete the quoted Frotz comment block, re-source the constants to the GIF89a
specification, and let the 383-picture oracle stand as the authority it already
is. No code changes.

### D9 — `crates/zvm/src/io.rs`, `text/decode.rs`, `cpu/decode.rs`, `text/mod.rs`, `tests/v1_v2.rs` — Frotz (GPL) — **NOT DERIVED; re-word (~12 sites)**

The `zvm` sites my own pass missed, because they sit outside the files the brief
named. Several quote short C expressions verbatim:

* **`io.rs:230-239`** — the strongest. Frotz's `record_code` condition is quoted
  (`force_encoding || c == '[' || c < 0x20 || c > 0x7e`) and the comment claims
  lanthorn's encoder is "**condition for condition**" the same.
* `text/decode.rs:185-192` — `else if (h_version==V1 && c==1) new_line();`
* `tests/v1_v2.rs:237-249` — `if (shift_state==2 && c==6)`
* `cpu/decode.rs:182-192` — "Following Frotz here" on `op0_opcodes`; `:486-497`,
  `:854-861` on `z_pull` storing unconditionally.

**Verdict on all of them: independent implementation of a rule the standard or
the format forces.** Taking `io.rs` as the representative, because it is the
worst-looking: the command-file format is defined by **ZMSD §10.2.1** ("The
format of a file containing commands must be the same as that written in output
stream 4"), and the doc derives the rule from its own worked example three lines
earlier. Once you decide to write printable ASCII literally and everything else
as `[N]`, the escape character itself must also be escaped — there is essentially
one expressible condition, and `(0x20..=0x7e).contains(&c) && c != b'['` is it.
Byte-level agreement with Frotz is the module's stated *purpose*, not evidence of
copying: "so a lanthorn command file and a Frotz one are the same file."

Fix: replace each quoted C expression with the standard's own statement of the
rule, and keep Frotz as a named interoperability target rather than as the
authority for the code. No code changes.

### D6 — `crates/gvm/src/glk.rs:29, 97, 159` — garglk (GPL) — **NOT DERIVED; re-word**

Three citations into `garglk/window.cpp` and `garglk/wintext.cpp`, both
confirmed GPL-2.0-or-later.

**Verdict: independent implementation.** All three annotate a rule the **Glk
0.7.6 spec states outright** and which the comment quotes from the spec ("You
must supply one of each when calling this function"; "glk_image_draw() is
equivalent to imagerule_WidthOrig|imagerule_HeightOrig, maxwidth=$10000").
garglk is named as the *reference library that agrees*, in the same role a
conformance test would play. Decisively, `ImageRule::resolve` **deliberately
computes differently from garglk**: the comment notes garglk resolves in
`double` and calls `std::round`, and lanthorn instead does `u64` round-half-up
division precisely because plain truncation would disagree. Code that documents
why it differs from a source is not a transcription of it.

No rewrite. Re-word to lead with the spec section and name garglk as a
conformance witness (see the re-wording quest).

### The (d-ii) re-wording inventory

Every remaining flagged site, by file, so the re-wording quest has a worklist.
None of these needs a code change.

| File | Sites | What to re-point at |
|---|---|---|
| `crates/scott/src/{vm,options,loader,scottfree_save,lib}.rs` | 92 `ScottCurses.c:NNN` | the parity fixture / ScottFree's usage message / the Definition (D5) |
| `crates/scott/tests/{scottfree_parity,golden}.rs` | ~27 `ScottCurses.c:NNN` | the transcript each one pins |
| `crates/scott/src/database.rs:20-23, 104-108` | 2, incl. "porting ScottFree's own split verbatim" | the format; cross-refs D1 |
| `crates/scott-cli/src/main.rs:145, 698` | 2 | the `Options` docs |
| `crates/app/src/scott_session.rs:26-29` | verbatim ScottFree source line `Output("\nTell me what to do ? ")` | observed output |
| `crates/app/src/engine_helpers.rs:704-708` | `ScottCurses.c:653-706` | the save format (D3) |
| `crates/blorb/src/infocom_pics.rs` | 12 (D8) | GIF89a spec + the 383-picture oracle |
| `crates/zvm/src/{io,text/decode,cpu/decode,text/mod}.rs`, `tests/v1_v2.rs` | ~12 (D9) | ZMSD sections |
| `crates/zvm/src/cpu/exec.rs`, `screen.rs` | ~58, incl. 2 quoted C conditionals | ZMSD §8.8.3.2.2 etc. (§6) |
| `crates/app/src/render/v6_border.rs` | ~18 (D7) | the measurement already in the file |
| `crates/app/tests/suites/{v6_side_border_tiling,v6_mac_pillar_feet,v6_arthur_status,v6_ega_dither_blend,v6_macintosh_profile,picture_override,transcript_stream_files,font3_shipped_font}.rs` | ~19 | the archive's own picture IDs; the shipped-font oracle |
| `crates/app/src/{graphics,interpreter,launch_options,inline_image,session}.rs`, `render/screen.rs` | ~9 | the corpus measurements already beside them |
| `docs/internals/v6-graphics.md:1531-1554, 545-584` | 2 clusters — restates the `draw_border.cpp` port in prose | must change **with** D7 |
| `docs/internals/performance.md:365-368` | 1 — attributes an unimplemented dispatch idea to dfrotz/glulxe | SQ-1441's own measurements |

Two notes on that list. `crates/app/tests/suites/v6_side_border_tiling.rs`'s
"Bocfel's `zorkzero.hpp`: `CASTLE_BORDER` 5 / `OUTSIDE_BORDER` 6 …" cites
**Zork Zero's own picture numbers** — facts about the archive, readable by
opening it — so only the attribution is wrong, not the constants. And
`font3_shipped_font.rs:3-6` contains an explicit "the table came from bocfel …
we copied that", which is accurate, permissively licensed (§5), and sits in the
one suite that **overrode** Bocfel from an independent oracle; it needs the
licence recorded, not an apology.

## 4. The ScottFree question, settled by history

**The `scott` crate was NOT written from ScottFree's source.** The evidence is
in the repository and is unambiguous:

* `git show ece616b3:crates/scott/src/loader.rs` (the founding loader,
  **2026-07-16**) contains **zero** matches for `ScottCurses`, `.c:NNN`,
  `porting` or `verbatim`. The founding `vm.rs` (`2c3f5aff`) likewise: **zero**.
* `git log -S "ScottCurses.c" -- crates/scott` returns exactly **two** commits,
  both dated **2026-09-08**: `6f1cf057` (SQ-1412, "match ScottFree 1.14's
  format-required behaviours") and `f1f63db0` (SQ-1413, "ScottFree option flags,
  wording table, and reference fidelity").
* Those two commits are the "2026-09-08 reference audit of `crates/scott`
  against a ScottFree 1.14 oracle" named in SQ-1412's own context field. They
  total ~2,400 insertions, essentially all inside `crates/scott`.
* No ScottFree material is vendored as a fixture: `crates/scott/tests/` holds
  only `tiny_cave.dat`, a fixture this project authored, and the golden test's
  header documents each transcript line by *rule*, not by comparison with a
  binary's output dump.

So the original engine — the loader, the condition evaluator, the action table,
the turn loop — was written from the format description and observed behaviour,
and is clean. **The contamination is a single-day, two-commit event with a known
boundary.** That is the good news in this audit: remediation does not touch the
crate's foundations, only what those two commits added.

The crate-level doc in `crates/scott/src/lib.rs:1-18` must change with them. It
currently states that "this crate's behaviour is checked against ScottFree's own
C source wherever the two could disagree, and each such site says so in its doc
comment" — an accurate description of the post-2026-09-08 state, and precisely
the claim the rule forbids making.

## 5. Permissive citations — fine, but attribution is owed

Not defects. Recorded because MIT's terms attach.

* **`crates/gvm/src/exec.rs`** says "**Ported** from glulxe's `op_fmod`
  (`exec.c`), not re-derived" (also `op_dmodr`/`op_dmodq`, `op_ftonumz`, and the
  `floatexp` wrapper), and quotes a glulxe comment ("the sign has been lost in
  the shuffle"). glulxe is **MIT**, so a port is *permitted* — but MIT requires
  the copyright notice and permission text to travel with "substantial portions
  of the Software". lanthorn currently ships no third-party notice file.
* **`crates/zvm/src/cpu/exec.rs:6672-6745`**, `font3_translate` — a 60-entry
  table whose doc says "Source: Bocfel interpreter … function
  `build_zscii_to_character_graphics_table`". Bocfel is MIT as of v2.2.3, and
  our read is from 2026 — permissive. Two further mitigations worth recording:
  the table implements **ZMSD §16**, and `crates/app/tests/suites/font3_shipped_font.rs`
  already checks it against **the font Infocom actually shipped** (`Graphic.Data`,
  an 8×8 Amiga disk font on *Lost Treasures* disk 5) — an oracle independent of
  every interpreter, which already **overrode** Bocfel at codes 71–74 (SQ-0915).
  This table is better evidenced than Bocfel's own.
* **`crates/zvm/src/text/encode.rs:120-148`** — Bocfel's `dict.cpp` (MIT) named
  as the reference for shift-lock encoding, with the ZMSD §3.5.4 text quoted
  first and a falsifying case given (Zork I's "pdp10"). Spec-led; Bocfel is the
  witness. Also records that **Frotz does *not* implement the rule** — which is
  itself a behavioural observation, not a reading of Frotz's code.

## 6. Frotz citations in `zvm` — class (c), no ports found

`crates/zvm/src/cpu/exec.rs` carries ~50 Frotz mentions and `screen.rs` ~8.
Every one was read. They name Frotz functions (`restart_screen`,
`screen_new_line`, `z_pull`, `winarg0`, `record_char`, `countdown`,
`init_memory`) as the *reference interpreter that behaves this way*, almost
always beside a ZMSD section number. **No `file.c:NNN` line-number citation
appears anywhere in `zvm/src` except `world.rs`** (inform6lib, Artistic-2.0 —
cleared in §7), and no comment in `exec.rs` or `screen.rs` claims a port.
Representative:

* `header.rs:87-88` — Frotz's `init_memory` "seek to end of file" when a v1–v2
  header lacks a file-size entry. A one-line interoperability *fact* about old
  story files, not expression.
* `screen.rs:786-793`, `807-814` — the `[MORE]` threshold and line-count reload,
  cited to Frotz beside **ZMSD §8.8.3.2.2**, which is quoted.

Two sites quote a short C conditional inline — `screen.rs:819-821`
(`if (y_cursor + 2 * font_height - 1 > y_size)`) and `exec.rs:5210`
(`if countdown != 0 { if --countdown == 0 { call }}`). Both express arithmetic
that is forced by the situation (does one more line fit? decrement-and-fire),
and both sit beside the standard's own statement of the rule. They are the
weakest members of class (c) rather than members of (d), but they are the two
Frotz comments worth rewriting first, because a quoted conditional reads as
transcription even when it is not.

Verdict for `zvm`: **nothing to rewrite.** The Frotz citations are re-wording
work, at lower priority than `scott`'s.

## 7. Good practice worth keeping

Two sites show the pattern the rest should move toward, and should be left alone:

* **`crates/blorb/src/infocom_pics.rs:325-375`** (`EGA_PALETTE`) — a 16-entry
  colour table "verified against **four sources that agree entry for entry**",
  of which Bocfel is one and three are independent of any interpreter (the EGA's
  own 2-bits-per-channel encoding, ModdingWiki, int10h.org). It even records
  where Bocfel's own comment is *wrong*. A table with four witnesses and a
  hardware derivation is not taken from any of them.
* **`crates/app/src/graphics.rs:815-840`** — Bocfel's `pixelwidth` and Frotz's
  `x_scale` cited as two witnesses *agreeing with a corpus measurement* the code
  made itself (125 Arthur pictures identical across renditions; 446 of 503 for
  Zork Zero). The comment says it outright: "the corpus agrees, which is what
  makes this a measurement rather than a reading."

`crates/zvm/src/world.rs:552-575` deserves the same credit — it quotes two short
`verblib.h` expressions but states plainly that "every number below is recovered
from the STORY, never assumed from the library source", so no constants were
taken. inform6lib is dual-licensed with Artistic-2.0 available at our choice, so
this site is clear on both counts and needs no change.

`crates/app/src/garglk_ini.rs` is named after Gargoyle but implements
**interoperability with the `garglk.ini` configuration file format** — reading a
file format is class (a), and the name signals compatibility, not derivation.

## 8. What must be rewritten, and how it would be verified

The Swansea "Definition" document is, by `crates/scott/src/lib.rs`'s own
account, **silent or wrong** on several of these points — so the standard alone
is thin, and the honest route for D1 and D2 is the **clean-room protocol** the
user approved (2026-09-09): one agent reads the GPL source and writes a
*functional specification* — formats, behaviours, worked examples, **no code** —
and a **separate agent that has never seen the source** implements from that
spec.

| Item | What the spec says | Clean-room needed? | Verification |
|---|---|---|---|
| **D1** `extract_auto_noun` | Definition describes item text carrying a `/NOUN/` auto-get word, but not the `//`, `/*`, or missing-close cases | **Yes** — spec must cover: first-`/` split, whole-remainder equality with `//` and `/*` meaning "no auto-noun, text untouched", noun runs to next `/` or end, uppercased | `scottfree_parity.rs` cases incl. `secret.dat` item 33 (`"Luger/LUGER/GUN/"` → display `Luger`, bind `LUGER`); black-box parity against ScottFree's own item listing |
| **D2** `next_str` / `ReadString` | Format-level: backtick → `"`, doubled `""` → literal `"` | **Borderline** — the two escapes are derivable from `.dat` files plus ScottFree output; use the clean-room pair only if the ordering question (which test comes first) cannot be settled by a fixture. Keep lanthorn's own `\r` and non-ASCII rules | Golden transcript + a fixture `.dat` exercising both escapes and a string containing `` ` `` and `""` adjacent |
| **D7** `extend_pillars` | Nothing — no standard covers border tiling; this is a rendering choice | **Yes** — spec must cover: the three-section model (capital / repeat unit / foot), that the unit is snapshotted **before** the foot region is cleared, the inset at each end, and the extend-only-when-taller guard | `v6_side_border_tiling`, `v6_mac_pillar_feet`, `v6_arthur_status`; the constants are already measured off `zork0.mg1` (82 / 26 rows) and that measurement is the oracle |
| **D3** `scottfree_save.rs` | Field order is an interoperability fact | No — restate in prose | Round-trip a real ScottFree-written `.sav` |
| **D4** `options.rs` | `-y/-s/-t/-p` documented in ScottFree's usage message | No — restate in prose | Existing `Options` tests |
| **D5** 92 `ScottCurses.c:NNN` citations | Message wording is observable output | No — re-point at the parity fixture | Suites already pin every string |
| **D6** `gvm/src/glk.rs` | Glk 0.7.6 §7.2 states the rules | No — code already independent | `glk_conformance_corpus` |
| **D8** `infocom_pics.rs` LZW | GIF89a defines every constant | No — re-source to the GIF spec | The 383-picture MCGA-vs-Amiga oracle already in the module |
| **D9** `zvm` Frotz quotes | ZMSD §10.2.1 and the v1/v2 text sections | No — quote the standard instead of the C | `save_interop`, `etude_sections`, `v1_v2` — all already `dfrotz`-parity |

**On the clean-room pairs.** For D1 and D7 the standard is genuinely thin —
the Swansea Definition is silent on the auto-noun edge cases (`crates/scott/src/lib.rs`
says so itself), and *no* standard describes v6 border tiling, which is a
rendering decision Infocom's own interpreters made differently. In both cases
the route the user approved (2026-09-09) applies: **one agent reads the GPL
source and writes a functional specification — formats, behaviours, worked
examples, no code — and a separate agent that has never seen the source
implements from that spec.** The spec content each pair must cover is in the
"Clean-room needed?" column above.

D2 is marked borderline deliberately: the two escape rules are plainly
observable, and only the claim about *test ordering* came from reading the
source. If a fixture can distinguish the orders, no clean-room is needed; if it
cannot, the ordering is unobservable and therefore not a fact worth preserving.

## 9. Quests filed

| Quest | Covers | Priority |
|---|---|---|
| **SQ-1445** | **D1** — re-derive `extract_auto_noun`. **Clean-room pair.** | high |
| **SQ-1446** | **D7** — re-derive `extend_pillars`. **Clean-room pair.** | high |
| **SQ-1447** | **D2** — re-derive `next_str`; clean-room only if a fixture cannot settle the ordering | high |
| **SQ-1448** | **D3, D4, D5, D6, D8, D9** and the whole (d-ii) inventory — the re-wording batch, including deleting the three verbatim C blocks and rewriting `crates/scott/src/lib.rs`'s crate doc | high |
| **SQ-1449** | MIT attribution — a `THIRD-PARTY-NOTICES` file for the glulxe ports and the Bocfel font-3 table | low |
| **SQ-1450** | Ask angstsmurf/spatterlight to clarify `terps/bocfel/z6/`'s licence | low |

`docs/internals/v6-graphics.md:1531-1554` restates D7's port in prose and must
be rewritten **in the same effort as SQ-1446**, not in the re-wording batch — a
doc that still calls the mechanism "a port of Bocfel's `draw_border.cpp`" would
outlive the code it describes.

## 10. Summary for the impatient

* lanthorn is BSD-3-Clause. **Nothing GPL is vendored**; the only foreign source
  text in the repository is quoted inside four doc comments, all identified here.
* **Bocfel has been MIT since February 2025** and every read of it postdates
  that — with **one exception**, Spatterlight's `z6/draw_border.cpp`, which
  carries its own GPL header. That single file is the source of the second-worst
  finding, and it is exactly the file "portions of Gargoyle are MIT" would have
  let us wave through.
* **Three things are genuinely derived from GPL code** and need re-deriving:
  `scott`'s `extract_auto_noun` (D1) and `next_str` (D2), and `v6_border.rs`'s
  `extend_pillars` (D7). Two of the three warrant the clean-room protocol
  because no standard covers the behaviour.
* **Everything else — ~70 sites — is citation hygiene.** The code is
  independently written, and in the strongest cases (the 383-picture LZW oracle,
  the four-witness EGA palette, the shipped-Amiga-font check that *overrode*
  Bocfel, the 125-picture corpus agreement) it is better evidenced than the
  source it cites. Those comments should say so.
* **The `scott` crate's foundations are clean.** It was written 2026-07-16 from
  the format and observed behaviour with zero source citations; all 121 arrived
  on 2026-09-08 in two commits. The remediation does not reach the original
  engine.
