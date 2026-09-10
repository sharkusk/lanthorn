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

**The survey below is preserved as written; what was actually done about it is in
"What was done: fetch, do not vendor", further down.** The one place the two
disagree is worth knowing before you read: the survey proposed *moving* the free
fixtures into the tree, and they are **fetched** instead — see that section for
why the IF Archive's own terms make the difference load-bearing rather than
stylistic.

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

Putting those files where CI can reach them un-skips them outright. No fixture
needs authoring, no format needs studying, and nothing can be fabricated wrong
because nothing is being fabricated. That is a larger win than every synthesis
below put together, and it is an afternoon of checking licences rather than a
project.

It is not free of judgement — each file's redistribution terms have to be
established one at a time, and "freeware" is not the same as "we may vendor it".
But the work is *verification*, which is a different and much safer activity
than construction. **And the verification came back saying: do not vendor.**
Almost none of these works carries a licence from its author at all, which under
the Archive's terms makes them ours to download and not ours to republish — so
they are fetched. See "What was done" below; the list above is otherwise
accurate, with the four exceptions named there.

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
Several of those 22 are already on the free list above (*Kerkerkruip*, *Cragne
Manor*, *The Wizard Sniffer* — though not *Counterfeit Monkey*, whose pinned
release is on no upstream), so this needs no new synthesis either: those three
are on the fetch manifest, and reaching for one of them here turns the strongest
case in the module from invisible to green.

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

## What was done: fetch, do not vendor (SQ-1015, decided 2026-08-25)

Recommendation 1 below said "move the free fixtures" into the tree. **That is
not what happened, and the difference is the point.** The IF Archive's own Terms
of Use <https://ifarchive.org/misc/license.html> settle it:

> The contents of the IF Archive … are the intellectual property of their
> original creators. … Any material with no attached license is presumed to be
> licensed for **personal use only**.

Downloading such a file is exactly what the Archive exists for. Committing one
into a repository that is itself published is redistribution, and the Archive
cannot grant that — it says so, in the same paragraph, about anyone who wants to
distribute a subset of its contents. So the repository carries a **manifest** and
never the bytes.

- `scripts/fixtures.manifest` — one record per file: SHA-256, byte count,
  destination name, upstream URL, zip member, and the licence basis, verified
  one file at a time rather than assumed from the directory it sits in.
- `scripts/fetch-fixtures.sh` — fetches and verifies it, into
  `crates/app/tests/fixtures/stories/`, which `fixture_paths::fixture_path`
  already falls back to. `--verify-only` checks a populated directory without
  the network. It exits non-zero on any absence or mismatch: a fixture that
  changed under us is a worse outcome than one that is absent.
- `.github/workflows/test.yml` runs it before `cargo test`, behind an
  `actions/cache` keyed on the manifest's own hash, so editing the manifest is
  the only thing that can invalidate the cache. `release.yml` runs no tests and
  needs nothing.

44 files, ~50 MB. What that turns on: **49 suites (161 tests) that depended only
on manifest fixtures now run on CI instead of skipping**, and 33 more run in
part — every case in them that names a free fixture, with the commercial ones
still skipping beside it.

### What the licences actually say

Only three bases in the whole manifest carry a statement from anyone but a
cataloguer, and saying so plainly is more useful than a table of assumed
freeware:

| basis | files | the statement |
|---|---|---|
| `gpl2-pd` | `scopa.z6`, `scopa.blb` | Aldo Cumani's own README: source under GNU GPL v2, the card artwork public domain or GFDL |
| `cc-by-nc-nd-3.0` | `LostPig.z8` | "Lost Pig by Admiral Jota and Grunk is licensed under a Creative Commons Attribution-NonCommercial-NoDerivs 3.0 Unported License." <https://grunk.org/lostpig/> |
| `author-permission-ifarchive` | the 11 Mysterious Adventures and their Blorbs, `golden_baton.blb`, `perseus_andromeda.blb`, `time_machine.blb`, `ten_indians.blb` | "Brian Howarth gave his permission to upload the games to the IF Archive." (the ReadMe inside `mysterious_blorb.zip`) — permission to *that host* to serve them, which is precisely what a fetch relies on |
| `ifarchive-tou` | the rest | nothing found from the author. The Archive's Terms govern: personal use presumed, freely downloadable, not ours to republish |

**Every `ifarchive-tou` entry was looked for and not found** — Photopia,
Anchorhead's 1998 release, Spider and Web, Adventure, Sunburst, Kerkerkruip,
Cragne Manor, The Wizard Sniffer, THE BAT, Chlorophyll and the rest. IFDB's
"License: Freeware" is a cataloguer's metadata, not the author's grant, and is
not recorded as one. That is not an obstacle to fetching; it is the reason not
to vendor.

### Four things deliberately NOT fetched

- **`fmvpoker.z6`.** Its author states the story file "may be freely copied and
  distributed" — and in the same breath that "the graphics file is the
  intellectual property of Activision". He is right: `stories/fmvpoker.blb` is
  byte-identical to Infocom's `ZorkZero.blb`, which the game's own instructions
  tell you to rename. Half a title is not a fixture. `v6_fmvpoker_hybrid`
  asserts render paths that exist only once the art is there, so fetching the
  story alone would not add coverage — it would fabricate a frame, which is
  bucket B's mistake wearing different clothes.
- **`Anchorhead.gblorb`** — the 2018 Special Edition is a paid product
  (mikegentry5.itch.io/anchorhead). The 1998 `anchor.z8` that IS fetched is a
  different work, not a different copy.
- **`CounterfeitMonkey-11.gblorb`** — no longer fully true; see "Counterfeit
  Monkey: fetched at a different release" below. The suites pin release 11
  (serial 230220), and the Archive's one copy has not moved since 12-Mar-2021 —
  it is release 10 (serial 210312), which no digest against "release 11" could
  ever match. The cases that assert release-11-specific facts stay local-only
  for that reason; the release-agnostic cases were repointed to what the
  Archive actually serves (SQ-1454).
- **`Alias 'The Magpie'.gblorb`, `frankenfingers_260330.z5`** — same shape: the
  local copies are releases the Archive no longer carries.

Release drift is the recurring hazard here, and the manifest's digests are what
make it visible instead of silent. A fixture that "is on the IF Archive" under
the right filename is not necessarily the fixture a suite was written against.

### A skip is no longer ambiguous

Once CI fetches some fixtures and not others, a silent skip stops meaning "CI has
no media" and might mean "the fetch quietly failed" — which would report a broken
step as a green run full of skips, the exact failure mode this whole document is
about. `LANTHORN_FIXTURES_REQUIRED=1`, which the workflow sets on the test step
and nothing else sets, makes `fixture_path` **panic** rather than answer with a
path that is not there — for the names the manifest promises and those only. A
developer's run is unchanged. Falsified by deleting one fetched file: the case
fails with the manifest name, both directories tried, and the command to run.

Two more sources of vacuous skip were closed on the way, both of them a name
rather than a licence: `sq1372_adventure_maze` and `sq1389_lostpig_gnome_room`
still built their own `stories/`-only path, and `sq1302_wizard_sniffer_rooms`
and `sq1303_glulx_static_world` asked for `The_Wizard_Sniffer.gblorb` where the
file on the shelf — and in `wizard_sniffer.rs`, which ran — is
`The_Wizard_Sniffer.gblorb.blorb`. Those four skipped *locally* too, past a
`stories/` directory that had the file.

**Cost.** `sq1372_adventure_maze` builds Adventure's whole map twice and takes
46–91 s per case on a 12-core machine; those three cases used to skip on CI and
now run. That is the largest single addition to the CI run, and it is real
coverage, but it is worth knowing before wondering where the minutes went.

### Still owed

1. ~~Move the free fixtures.~~ Done differently — fetched, above.
2. **The remaining unguarded suites.** 106 still define a private
   `stories_dir()`, and every one of them is commercial-only, so the guard above
   cannot reach them: `LANTHORN_FIXTURES_REQUIRED` only speaks for names the
   manifest promises. Consolidating them onto `fixture_path` costs little and
   buys nothing today; it buys something the day a fixture of theirs becomes
   fetchable.
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

## Counterfeit Monkey: fetched at a different release (SQ-1454)

SQ-1015 (above) left `CounterfeitMonkey-11.gblorb` unfetched because "the
Archive carries release 12 and releases 5–9" and no digest could be pinned
against the suites' release 11. Checked again directly against the Archive
(2026-09-09): that was imprecise. `games/glulx/CounterfeitMonkey.gblorb` is
the Archive's only copy, its directory listing has said "Release 10 / Serial
number 210312" since 12-Mar-2021, and the file's own embedded `IFhd`/iFiction
chunks agree — 11,314,624 bytes, sha256
`f9544d3111b2db43c4c7ab12a07109c6e84bb623e92ded796e9dfc7e3558874d`. IFDB's
"Current Version 11" field is a crowd-edited pointer at a GitHub release
(11.1) the Archive does not carry, not a description of the archived file;
trust the bytes over the catalogue entry. Licence: the author's own
`LICENSE` in the `i7/counterfeit-monkey` GitHub repo states CC BY-SA 4.0
(Attribution-ShareAlike, no NonCommercial clause) — narrower research had
assumed CC BY-NC-SA, which is wrong.

Sixteen suites across `crates/app/tests/suites/`, `crates/gvm/tests/` and two
in-crate `app` tests (`glulx_session.rs`, `render/screen.rs`) pin
`CounterfeitMonkey-11.gblorb`. Rather than leave all of them local-only, each
was run against release 10 (symlinked into the worktree's fetched-fixtures
directory) to sort release-specific assertions from structural ones:

- **Release-specific — stay pinned to 11, `stories/`-only**, the same shape as
  `real_media_releases.rs`: cases asserting an exact release/serial
  (`sq1306_mapgen`'s `counterfeit_monkey_static_map_covers_every_walked_room`),
  an exact compiled address or count (`grammar_tables`'s table address,
  `object_words`'s object-tree head, `i7_map`'s `Map_Storage` address), an
  exact map-geometry count tied to release 11's room layout
  (`sq1316_connector_overlaps`'s diagonal-yield count), or a style hint release
  10 does not set the same way at boot (`glulx_game_colours`).
- **Release-agnostic — repointed to `CounterfeitMonkey-10.gblorb`**, fetched
  via the manifest below: the rest — opening-room keying, the room lock, the
  bolded-name and flashback-heading rules, silent vehicle moves, the stale
  sidecar recovery, distorted-flag geometry, the avatar-refusal seam, odd-pane
  layout, the shadow-boot cache seam, the accel on/off equivalence, and most of
  the compiled-map reader (room/exit membership, live `Map_Storage` writes, the
  compass-column count) — all hold on release 10 exactly as they do on 11.

Where a suite's fixture-resolution helper was a private `stories/`-only join
(most were), it was migrated to `fixture_paths::fixture_path` (or the
equivalent two-directory fallback the zero-dependency `gvm` crate's tests
reimplement locally) so the repointed cases actually reach the fetched copy on
CI rather than continuing to skip. `scripts/fixtures.manifest` carries
`CounterfeitMonkey-10.gblorb` under licence `cc-by-sa-4.0`.

## A fixture that is allowed to differ in bytes (SQ-1461)

`Kerkerkruip.ini` looked, at first, like the case the digest pin exists to
catch: a local copy at 877 bytes against the manifest's pinned 875, reported by
`--verify-only` as a MISMATCH while a fresh CI fetch stayed pristine. The
obvious read — "the game rewrites its own settings file when played, so a
played local copy drifts from a never-played CI one" — turned out to be wrong.
Checked against the code rather than assumed: `Kerkerkruip.ini` is not a save
or a preferences file at all. It is a **Gargoyle interpreter config**, in
Gargoyle's own `.ini` format (`# Gargoyle Glk configuration for Kerkerkruip!`),
that lanthorn reads directly off disk once at boot
(`app::garglk_ini::discover`, `std::fs::read_to_string`) and never writes back.
The game's *own* preferences live behind a completely different name —
Kerkerkruip's `KerkerkruipStorage` Glk-file extension (`crates/gvm/tests/
kerkerkruip_boots.rs`) — which is virtualised through gvm's in-memory Glk VFS
and, when persisted at all, lands in the app's per-game save directory
(`<data_base>/<story-key>.save/`, see `storage::game_dir` and
`startup.rs`'s `story_game_dir` call), never beside the story in `stories/`.
No suite that boots Kerkerkruip through the app (`sq1303_glulx_static_world.rs`,
`glulx_garglk_style_sentinel.rs`) passes the story's own directory as the
persistent store — both use `app::scratch_dir(...)` or no store at all — so
nothing in a normal test run, or in ordinary play, ever touches the `.ini`
file on disk.

The actual difference was two bytes: one extra trailing CRLF at end-of-file
(`data[:-2]` of the local copy hashes to exactly the manifest's pinned digest),
almost certainly a byproduct of however the local copy was extracted or last
opened, not of anything lanthorn or the game did. And the one suite that reads
this file, `glulx_garglk_style_sentinel.rs`, only cares about its *parsed*
content — the `tcolor 10 F400A1 ffffff` line the sentinel test looks for — which
a trailing newline cannot change either way.

So digest-pinning `Kerkerkruip.ini` byte-for-byte was enforcing a stricter
invariant than any reader needs, on a file that (unlike every other row in the
manifest) genuinely isn't lanthorn's to author or reproduce exactly — its
provenance is "however you last got a copy of the Gargoyle config", not "the
release archive's canonical bytes". The fix is a new `scripts/fixtures.manifest`
column: this one row is marked `presence`, meaning `scripts/fetch-fixtures.sh`
still fetches it and still requires it to exist and be non-empty, but does not
fail the run over a content mismatch. `scripts/fetch-fixtures.sh --self-test`
covers the new branching (there is no Rust suite that reads this shell script,
so this is the shell-native equivalent). See the manifest's own header, next to
the `Kerkerkruip.ini` row, for the column's exact contract.

The general lesson generalises further than this one file: not every fixture
this manifest names is a work whose bytes lanthorn must reproduce exactly. A
digest pin is right for a story file or a picture archive, where a single
changed byte is exactly the failure this manifest exists to catch. It is wrong
for a companion config file no code path ever regenerates or depends on
byte-for-byte — pinning it anyway just turns an inconsequential difference in
provenance into a false CI-vs-local disagreement that a suite reading the file
would never itself observe.

## One footnote worth keeping, and one resolved

`unit_tests/ziptest-r12-s890607.z6` and `unit_tests/ziptest-r13-s890619.z6` are
**Infocom's own in-house ZipTest regression stories for the YZIP (Version 6)
interpreter**, June 1989, and are **not redistributable** — unlicensed
proprietary material, copyright now Microsoft. Their menu table names `YZIP
windows`; their strings are written in ZIL/ZAP opcode vocabulary (`DIROUT`,
`CURGET`, `IGRTR?`, and the interpreter's internal `LMRG`/`RMRG` globals); their
headers are pre-Inform in shape. They belong in `stories/`, with any suite using
them written to skip. `.gitignore` covered `/unit_tests/*.z5` but **not** `*.z6`
until 19fc75f6, so those two files were untracked-yet-unignored and one careless
whole-directory stage would have committed them; every Z-machine version is
covered now.

**Resolved (SQ-1453, 2026-09-10).** `crates/zvm/tests/fixtures/minizork.z3`,
Infocom's Mini-Zork I demo, used to be vendored rather than fetched — the one
committed file in the tree this document's rule did not cover, since it
predates SQ-1015. It is now a manifest row like every other free fixture
(`minizork-r34-s871124.z3` in `scripts/fixtures.manifest`, same IF Archive
URL, `if-archive/infocom/demos/minizork.z3`, under `ifarchive-tou` — no
stronger permission statement from Infocom, Activision or Microsoft was
found, and the November 2025 MIT release of the Zork sources names specific
full releases and not this cut-down demo, so it rests on the same Terms-of-
Use basis as everything else fetched rather than committed here). `zvm`
takes zero external dependencies and so has no fetch machinery of its own;
`crate::fixtures::load("minizork.z3")` falls back to the app crate's fetched
copy (`crates/app/tests/fixtures/stories/minizork-r34-s871124.z3`) when its
own `tests/fixtures/` does not have it, which is now always, on CI and in a
fresh worktree that has run `scripts/fetch-fixtures.sh`.
