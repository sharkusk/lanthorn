# What CI cannot see

`stories/` is gitignored, because it is commercial game media. Every integration
suite that needs a file from it skips — quietly, vacuously, and green — on
GitHub Actions, which is the run that guards a merge. The local gate covers
those suites; the gate that matters does not.

This is the survey SQ-1015 asked for: how big the hole is, which of it can be
closed with fixtures we author ourselves, and which of it cannot be closed at
all. The short answer is that the honest ceiling on synthesis is small, that a
much larger win is sitting in plain sight and needs no synthesis whatsoever, and
that the single most valuable thing to synthesise has now been built.

## The hole, counted

Counted at SQ-1015, and a snapshot rather than a constant — the suite directory
grows most weeks, and every suite added under `crates/app/tests/suites/` that
opens a `stories/` file adds to the first number here. Re-count before quoting
these; the shape of the answer is what is durable, not the totals.

| | suites | `#[test]` fns |
|---|---|---|
| under `crates/app/tests/suites/` | 177 | 1,012 |
| **depending on a `stories/` fixture** | **155** | **885** |
| …of those, tests that actually touch a file | — | 753 |
| v6-related (filename `v6_*`, or v6 by subject) | 96 | 499 |

A further four suites are equally blind on CI but depend on *other* gitignored
media directories rather than `stories/` — `adf_disk_image` (5 tests, off
`$HOME/Downloads`), `masterpieces_sides` (6), `cover_frontispiece` (4),
`font3_shipped_font` (3). They are the same problem wearing a different path.

Two numbers are worth staring at. **134 of the 155 print a `SKIP:` line**, so a
human reading the log can see what did not happen — but **only 32 carry a
non-vacuity guard**, which means 123 suites can pass having executed no
assertion at all and say nothing about it. And there is no shared helper: 125 of
them define their own private `stories_dir()`, so there is no single place to
change how a missing fixture is handled.

(An earlier count said 151 suites and 88 v6. The difference is `cast_manifest`,
which is a false positive — its `"stories/a.z5"` is a string inside a TOML
manifest and no file is opened — against several suites the old grep missed,
including `gallery_manifest`, whose media paths come from
`crates/app/examples/gallery.toml` rather than from a literal in the suite.)

## The finding that outranks the rest

**39 of the 155 suites — 135 tests — depend only on fixtures that are already
freely redistributable.** They sit in `stories/` because that is where story
files go, not because anyone decided they were commercial.

`advent.z6` and `advent.blb`, `scopa.z6`, `sunburst.z6`, `mysterious01`–`11.z6`,
`fmvpoker.z6`, `anchor.z8`, `photopia.z5`, `minizork-r34`, and the modern Glulx
and Scott works — *Kerkerkruip*, *Counterfeit Monkey*, *Cragne Manor*, *The
Wizard Sniffer*, *THE BAT*, the eight `glulx_room_detection` gblorbs,
`golden_baton` / `perseus_andromeda` / `time_machine`.

Moving those files into `unit_tests/` un-skips 135 tests on CI outright. No
fixture needs authoring, no format needs studying, and nothing can be
fabricated wrong because nothing is being fabricated. That is a larger win than
every synthesis below put together, and it is an afternoon of checking licences
rather than a project.

It is not free of judgement — each file's redistribution terms have to be
established one at a time, and "freeware" is not the same as "we may vendor it".
But the work is *verification*, which is a different and much safer activity
than construction.

**SQ-1102 added a case in exactly this shape, and it is worth naming because the
hole is total rather than partial.** `gvm::grammar` locates and reads Inform's
grammar tables in a Glulx image, verified against `glulxdump` across all 22
Glulx stories in `stories/` — 6,911 grammar lines, zero differences. On CI it
proves none of that: `glulxercise.ulx` is the only committed Glulx fixture and
carries no grammar at all, so the single CI-visible case is a **refusal**
(`TablesNotFound` — the dictionary was found and the chain would not close) and
every positive case skips vacuously. The locator is the part most worth guarding
— 889 byte offsets across the corpus pass its pointer-array precondition and
only 22 survive the full walk — and it is precisely the part CI cannot see.
Several of those 22 are already on the redistributable list above
(*Kerkerkruip*, *Counterfeit Monkey*, *Cragne Manor*, *The Wizard Sniffer*), so
this needs no new synthesis either: one file moved into `unit_tests/` turns the
strongest case in the module from invisible to green.

## What can be synthesised, and what cannot

Sorting the 155 by what the suite is really *about*:

| bucket | suites | tests | tests touching `stories/` |
|---|---|---|---|
| **A** — a format READER is the subject | 8 | 76 | 61 |
| **B** — a specific commercial release is the subject | 81 | 527 | 491 |
| **C** — engine or VM behaviour, any story would do | 57 | 233 | 169 |
| **D** — the reader already has synthetic cover in `blorb` | 9 | 49 | 32 |

### B is the majority, and it is closed

Eighty-one suites, 527 tests. These assert what a *particular* release does:
Arthur r74's pixel status bar, Journey r83's command menu, Zork Zero's EGA
dither against its own `.eg1`, the Macintosh press's 7x15 cell, the release and
serial each medium carries. `real_media_releases` is the pure case — its whole
content is "this disk is release 83, serial 890706" — and nothing synthetic can
stand in for that by definition.

**Do not try.** A fabricated frame is precisely the SQ-0901 failure mode: two
harnesses omitted `native_std_window`, measured a 560x384 press at 640x400, and
a whole quest was then fixed and tested against the Arthur frame that produced.
The numbers were entirely self-consistent and described a screen the player
never sees. A synthetic *story* engineered to produce a plausible v6 frame is
the same mistake with more effort behind it. These suites stay local-only, and
the right response to them is better non-vacuity guards — so that a green CI run
says "did not run" out loud — not a fake game.

### A is small, and it is where the value is

Eight suites whose subject is a container or archive reader rather than a game:
`disk_set_rows`, `disk_story_rows`, `native_disk_font`, `picture_override`,
`release_enumeration`, `save_key_media`, `story_identity_sweep`,
`volume_chooser`.

`native_disk_font` is the one that raised SQ-1015 and the one to do first,
because of what the crate census says:

> **`amiga_font.rs`, `bitmap_font.rs`, `mac_font.rs` and `resource_fork.rs` had
> zero `#[test]` functions between them.** The only coverage the Macintosh and
> Amiga font readers had anywhere was through real release floppies — that is,
> none on CI.

That gap is now half closed; see below.

### C is large but weakly motivated

Fifty-seven suites, 233 tests, that need *a* story rather than *that* story:
`[more]` pager arming, restore-replay mechanics, `/dump-windows` per engine,
`Introspect::room_objects`. A small authored story would serve — but authoring
one means writing and compiling Inform or ZIL, and twenty-eight of these 57
already depend only on free fixtures and are covered by the move above. The
residue is not worth a compiler toolchain.

### D buys the least

Nine suites whose readers already have working synthetic builders one layer
down in `blorb` — `hfs`, `adf`, `fat12`, `prodos`, `medium`, `infocom_packed`,
`infocom_pics`, `bpal`, `infocom_boot`. Duplicating those upward into `app`
tests the same code twice.

The crate is in better shape than the app suites suggest: **271 tests, 199 of
them synthetic**, with in-test builders for HFS volumes, AmigaDOS volumes,
ProDOS images, ISO9660 discs, DOS sector orders, Blorb files and Infocom picture
archives. Seventy-two are real-media and skip on CI, concentrated in `d64`
(12 of 18), `infocom_boot` (9 of 13) and `prodos` (9 of 22).

## A caveat about `unit_tests/`

`unit_tests/` is not automatically the answer, because most of it is gitignored
too — `.ulx`, `.gblorb`, `.blb`, `.z5`, `.glkdata`, `.blorb` are all in
`.gitignore`, and its README is a re-fetch manifest rather than a vendored
corpus. The Glulx conformance suite is *fetched*, not committed.

So "the Glulx suites test properly on CI and the v6 ones do not" is only true of
`crates/gvm-cli/tests/fixtures/glulxercise.ulx`, which is the one story actually
vendored in-tree. Anything moved into `unit_tests/` under a gitignored extension
lands in exactly the hole it was moved out of. A fixture is only on CI if `git
ls-files` can see it.

## What was built

`unit_tests/macfont.hfs` — a 32 KB synthetic Macintosh volume carrying a bitmap
`FONT`, and `unit_tests/mk_macfont_hfs.py`, the generator that emits it.

It exercises, end to end and on CI: `Hfs::mount` on a real volume structure →
the `APPL` catalog entry → `read_resource` pulling a fork out of a file whose
**data fork is zero bytes** → `ResourceFork::parse` → `mac_font::parse`. That
zero-byte data fork is the case worth having: it is how every Infocom Macintosh
release ships, and a reader that can only reach data forks sees an empty file
rather than a font.

Fifteen new tests in `crates/blorb/src/{mac_font,resource_fork}.rs`, in modules
that had none.

Two things about how it was built, both of which are the point rather than
housekeeping:

**It is not a mirror.** `blorb`'s existing HFS tests build volumes with an
in-test builder, and a writer and a reader developed together agree with each
other whether or not either agrees with HFS. The generator is a separate
implementation, written from Inside Macintosh — *Files* for the volume, *More
Macintosh Toolbox* for the resource fork, *Text* for the `FontRec` — sharing no
code with what it tests. Writing it from the spec caught a real error in the
first draft: `drNxtCNID` is at MDB offset 30, not 32, and a volume with it in
the wrong place still mounts perfectly well.

**Every expected value is written out by hand**, including all fifteen rows of
every glyph, with the ASCII art beside them in a comment. The fixture's left
side bearing is deliberately non-zero — `kernMax` −1 plus an offset byte of 2 —
so the whole glyph table doubles as the falsification test for the bearing
arithmetic that SQ-0916 got wrong.

Four falsifications were run, and all four fail loudly:

| break | result |
|---|---|
| fixture deleted | **compile error** — `include_bytes!` cannot skip |
| one strike byte flipped | `parses_the_font_resource_glyph_for_glyph` fails |
| `parse` ignores `kernMax` (the SQ-0916 bug) | same test fails |
| a resource's declared length changed | the non-vacuity guard plus two others fail |

**It does not replace `native_disk_font.rs`**, and that suite was not touched.
That one pins the *real* face — 7x15, baseline 12, 200-odd glyphs — and is what
proves we read Infocom's data correctly. Synthetic proves the machinery; real
media proves the data. Both are needed, and the real one still skips on CI.

## Recommended order

1. **Move the free fixtures.** 135 tests, no authoring, no fabrication risk.
   Verify each licence individually and commit them under extensions
   `.gitignore` does not swallow — or amend `.gitignore`. Biggest win by a wide
   margin.
2. **Give the 123 unguarded suites a non-vacuity guard**, or one shared helper
   that provides it. This does not add coverage; it stops CI green from meaning
   two different things. Cheap, and it makes every number above self-reporting.
3. **The Amiga font, mirroring the Macintosh one.** An authored ADF carrying a
   `DFH_ID` font would close the other half of the `native_disk_font` gap and
   give `amiga_font.rs` its first test. `adf.rs` already has a synthetic volume
   builder to read the format against — though the same
   independent-implementation rule applies to the font itself.
4. **The `infocom_pics` flavours**, if anything. A tiny authored archive per
   flavour (`Pic.data`, `CPic.data`, `.mg1`, `.cg1`, `.eg1`) would test the
   readers — but not the artwork, and `infocom_pics.rs` already carries 22
   synthetic tests with three builders. Low value; listed for completeness.
5. **Nothing for bucket B, ever.**

## Two footnotes worth keeping

`unit_tests/ziptest-r12-s890607.z6` and `unit_tests/ziptest-r13-s890619.z6` are
**Infocom's own in-house ZipTest regression stories for the YZIP (Version 6)
interpreter**, June 1989, and are **not redistributable** — unlicensed
proprietary material, copyright now Microsoft. Their menu table names `YZIP
windows`; their strings are written in ZIL/ZAP opcode vocabulary (`DIROUT`,
`CURGET`, `IGRTR?`, and the interpreter's internal `LMRG`/`RMRG` globals); their
headers are pre-Inform in shape. They belong in `stories/`, with any suite using
them written to skip.

`.gitignore` covers `/unit_tests/*.z5` but **not** `*.z6`, so those two files are
untracked-yet-unignored and one careless `git add unit_tests/` commits them.
The hard rule against `git add -A` is the only thing that has prevented it.
