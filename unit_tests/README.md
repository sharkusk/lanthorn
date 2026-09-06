# Glulx / Glk conformance test suite (external oracle)

The canonical Glulx/Glk test stories by Andrew Plotkin (Zarf), used to verify the
gvm/Glk stack (SQ-0312). **The story binaries are not committed** (they are
gitignored — see the root `.gitignore` `unit_tests/*` entries); this README is
the manifest and re-fetch recipe. `crates/gvm-cli/tests/fixtures/glulxercise.ulx`
is the one story that *is* vendored, for the in-tree conformance test.

Downloaded 2026-07-14 via `curl -L`. Two upstreams:

- **Zarf's Glulx page** — <https://www.eblong.com/zarf/glulx/>
- **erkyrath/glk-dev `unittests/`** — <https://github.com/erkyrath/glk-dev/tree/master/unittests>
  (raw base: `https://raw.githubusercontent.com/erkyrath/glk-dev/master/unittests`)

Both hosts serve identical bytes for the shared files; the manifest below is the
source of truth (SHA-256).

## Re-fetch

```sh
cd unit_tests
Z=https://www.eblong.com/zarf/glulx
G=https://raw.githubusercontent.com/erkyrath/glk-dev/master/unittests

# core VM + Glk exerciser (self-checking pass/fail report; drive with "all")
curl -sLO $Z/glulxercise.ulx

# glk-dev unittests (compiled .ulx / .gblorb)
for f in accelfunctest datetimetest extbinaryfile externalfile inputeventtest \
         inputfeaturetest memcopytest memheaptest memstreamtest randomgen \
         resizememstreamtest selectvarianttest statusbufferwin unicasetest \
         unicodetest unidicttest unisourcetest windowtest; do curl -sLO $G/$f.ulx; done
for f in autosavetest graphwintest imagetest resstreamtest startsavetest \
         startsavetest-empty; do curl -sLO $G/$f.gblorb; done

# Zarf-only extras (sound+graphics demo, two-column Glk demo)
curl -sLO $Z/sensory.blb
curl -sLO $Z/sensory.ulx
curl -sLO $Z/twocol.ulx
```

## Manifest (SHA-256, bytes, file)

```
aa2621d035bf843be8c6bf557182c252c8b83842463c36842d19f7278ce6b829  128256  accelfunctest.ulx
5ccb43f99a6a361b525513448f9bf84fce21d65ee5408c66e8d6bc4b73986f4c  135850  autosavetest.gblorb
b32ec0803c60a31de07c4c23c00bb5d4f8dbe258956392813b8af0847d71a0b5  124160  datetimetest.ulx
1c83b0ea98f2f4c17322bbf1608630dab95fcd1fd6d8749a2f11501ed76a0da0   13312  extbinaryfile.ulx
59813b156e58d7fab97ba6d70b4f46a816bf67a404d3ced4d8e492c9c7ac4509   17664  externalfile.ulx
b732127fee4cb266a5330981c1111fdfaba237134525754e063e6dc5f449b348  231680  glulxercise.ulx
45b942f1d8c995ed225fccbdc336d34bfe96f81cd35a656d2c9c744bdc9f965e  152500  graphwintest.gblorb
06e15822425cfa607a1dd611298a532ef669939fe2926973808b616075876cad  150708  imagetest.gblorb
6887900f0c96a1b63feb2bdae13a6dd302efc30743b69282f62cccd01b61eca6  119040  inputeventtest.ulx
1fe2d4c126dd883abfc0f19c41676d34973f424c58f00b4ab5a0ee24c348552c   10240  inputfeaturetest.ulx
efb6acbbaea4731d5f1930967b6b84575f0234b7accf6092bb24daa0704a4945  114944  memcopytest.ulx
cd78c3a05d334b573ed3d385aa3bf2f61b49da5099c3433d6ac6be478f23d68b  114432  memheaptest.ulx
85a93b8bc1d8ca461757cf50de6f27c8b638a6e2472c97b0fd2241e7506a787b  121856  memstreamtest.ulx
5d7cb773dce372f1555f73af3166f1501a51c22e149152f5dae35f30c9cd0986    7936  randomgen.ulx
0e4a1d813e7c75a5dc2b9bb402eccf784e50e9f88bb2a452212967aec88ba4cd    7936  resizememstreamtest.ulx
70354791fb318c01b33d44a70e1530cb39981f52b5745cafc4451a05addbbcb0   18884  resstreamtest.gblorb
e8bbc090604992c947e32dccee7e10bbf833746d568a8ba3630b056c9abbdd16    8704  selectvarianttest.ulx
a05cd29a71b3200e564a1f33146200f88664aab394b9924acab99381a4001afd  202258  sensory.blb
d188929a7de0af81da3fbf6787f1a85f2933ab371a717c7bbb0c2ce70ffcc6f7  132608  sensory.ulx
7140c221d2285da9488b5526d704916df1982e50e7d623dd6e582ef853e542eb  115264  startsavetest-empty.gblorb
11892834e18375d83f988511c95955ffd7229b003bc7bc6392ddfebd66ef29a1  115818  startsavetest.gblorb
ac5a2eda83012409c5ef504cded782de918be8352775ded3c78f0f297a47667e  115456  statusbufferwin.ulx
23b2acb6ba725236b9db010342f607a41e9fc5e44758a7360b12d102418b0e6d  129024  twocol.ulx
e4b2da7fe1a894913421ba87cf26551f18fa158294df2333bbb79bc39b2f219c   18432  unicasetest.ulx
ff57ddb9bacfc2ecff22257b4ddc77f08e85e644d7174e6bb83008a7e95bd210  116224  unicodetest.ulx
3162dfcaf9f5d96be94122873fc29ab7ba3125d26a7feec4a85fb369e7cd1001    9472  unidicttest.ulx
87d1ca3231f55373206ff2a9169db57002915dc1454ac1e343dcc001549eb2a0    9216  unisourcetest.ulx
f2cfd00107ee49b344a1a184ede31a6b914bd064104faefc664f4705da7b6757   17152  windowtest.ulx
```

## Driving them headlessly

`gvm-cli <story>` reads cooked lines from stdin and writes the transcript to
stdout; diagnostics + the `gvm:` diagnostic dump go to stderr on clean exit.
There is no dedicated headless flag — pipe a scripted input and cap wall-time
(the VM loops on empty-line input at EOF; kill it once the test has reported).
Use `--data-dir <dir>` to sandbox file/fileref writes.

- **Self-checking** (print their own PASS/FAIL): `glulxercise` (`all`),
  `unicasetest` (`all`), `resizememstreamtest`, `unisourcetest`,
  `extbinaryfile`, `externalfile`, `resstreamtest`, `memstreamtest`
  (`pos`/`read`/…). These are the automated oracles.
- **Interactive Inform exercisers** (visual verification; headless-checkable
  only for faults/diagnostics/sensible output): `datetimetest`, `windowtest`,
  `inputeventtest`, `inputfeaturetest`, `twocol`, `sensory`, `statusbufferwin`,
  `selectvarianttest`, `accelfunctest`, `memcopytest`, `memheaptest`,
  `randomgen`, `unicodetest`, `unidicttest`.
- **Need graphics / sound / a real TTY** (out of terminal scope):
  `imagetest`, `graphwintest` (report "does not support graphics"),
  `startsavetest*`, `autosavetest` (kill-and-restart autosave protocol).

## Map fixtures

`advent_maze_map.json` — a player's real, partial mapping of Colossal Cave
(`advent.blb`), lifted verbatim from the `map.json` inside a lanthorn archive.
30 rooms across two layers; layer 1 is the hand-peeled "all alike" maze (12
rooms, 47 in-layer edges, 11 of them sharing the name "Maze"). Player-generated
data, freely redistributable, and the calibration set behind the matrix view and
its tangle threshold (SQ-0666): 2 of the 47 edges are reciprocal, 18 return by a
different direction, 27 have no known return, and 29 are marked distorted.

`zork1_maze_map.json` — the same thing for Zork I (`zork1-invclues-r52-s871125.z5`),
and the counter-case: nothing here is hand-peeled. 31 rooms across two layers;
layer 2 ("Cellar", NOT flagged a maze) holds the maze *and* the ordinary rooms
around it — 5 "Maze", 2 "Dead End", plus Cellar, The Troll Room, Gallery, Studio
and East of Chasm, all one connected component. That is the shape that defeated
component-based tangle detection (SQ-0683): the component scores 0.45 asymmetry
while the 7 maze rooms inside it score 0.83. The above-ground layer 0 (19 rooms)
is the negative control, and its ring of rooms around the white house is the
closest a real overworld comes to a maze — 4 rooms, 1.00 asymmetry, held off by
the room-count bar alone. Loaded by `crates/mapper/tests/zork1_maze.rs`.

`zork1_walked_map.json` — a third snapshot of the same story, and the one the
LAYOUT is argued from rather than the matrix view (SQ-1364). 26 rooms across
three layers, 66 connections, 19 rooms above ground; the player had walked the
white house, the forest and as far as the Troll Room. What it carries that no
synthetic graph carries as economically is a three-way conflict over one cell:
the `Forest` #230 is reciprocally south of the `Clearing` #134 (walked both
ways), and `South of House` #217 makes two separate ONE-WAY claims on the same
room — an `S` into it, answered by a `NW` back out. The solve used to split the
difference and drop the Forest an extra row, leaving the reciprocated pair
aligned but two cells apart and still flagged undistorted. Loaded by
`crates/mapper/tests/sq1364_clearing_gap.rs`.

All three are committed as-is rather than regenerated, because the point of them is
that they are real: an actual snapshot of what a player knew mid-game, and no
synthetic graph reproduces the particular mess. The first two are also loaded by
`crates/app/tests/matrix_view.rs` and `crates/mapper/tests/advent_maze.rs`.

## Authored fixtures

Everything above is fetched. What follows is **built here**, from a published
format description, and committed — which is the whole point of it: a fixture we
authored is redistributable by construction, so it is present on CI, where
`stories/` is not (SQ-1015).

`macfont.hfs` — a minimal Macintosh volume carrying a bitmap `FONT`. 32,768
bytes: a 64-block HFS volume with a Master Directory Block, a two-node catalog
B*-tree, and a `Test Folder` holding two files.

| file | type | data fork | resource fork |
|---|---|---|---|
| `Test Folder/Story.data` | `INdf` | 512 bytes, a bare Version 6 header | none |
| `Test Folder/TestApp` | `APPL` | **zero bytes** | 525 bytes, two `FONT`s |

The zero-byte data fork is the case that matters: that is how an Infocom
Macintosh release ships, and a reader that can only reach data forks sees an
empty file rather than a font. The resource fork holds `FONT` 524 (7x15, ascent
12, `A`–`D`) and `FONT` 1033 (7x12, ascent 9, `0`–`1`) — the two ids and the
7x15 cell a real release uses, with glyphs this repository drew.

Regenerate with:

```sh
python3 unit_tests/mk_macfont_hfs.py unit_tests/macfont.hfs
```

The generator is committed beside it and is the documentation: it cites the
Inside Macintosh chapters each structure comes from, and it shares no code with
the Rust readers it exists to test. That independence is deliberate. `blorb`'s
other HFS tests build volumes with an in-test builder, which is a mirror — a
writer and a reader developed together agree with each other whether or not
either agrees with HFS — and the resource-fork path had no coverage at all
before this. Every expected value in `crates/blorb/src/mac_font.rs`'s tests is
written out by hand, including all 15 rows of every glyph.

It does **not** replace `crates/app/tests/suites/native_disk_font.rs`, which
pins the *real* face off a real floppy — 7x15, baseline 12, 200-odd glyphs.
Synthetic proves the machinery; real media proves the data. Both are needed, and
the real one still skips on CI.

Loaded by `crates/blorb/src/resource_fork.rs` and `crates/blorb/src/mac_font.rs`
via `include_bytes!`, so a vanished fixture is a compile error rather than a
vacuous skip.

`sysfont.hfs` and `relfont.hfs` — the two halves of the **face cascade**
(SQ-1037), built by one generator from the same primitives. The cascade ranks a
release's own typeface against the machine's system typeface off a boot disk the
player supplies, and testing it needs one volume of each kind.

| volume | plays | resources |
|---|---|---|
| `sysfont.hfs` | a Mac OS System startup disk | `FONT` 12, 394, 396 — a family of proportional faces |
| `relfont.hfs` | an Infocom Macintosh release | `FONT` 524 — one fixed-pitch face, 7x15 |

The System disk carries the three discriminators a real one carries, which is why
it carries three rather than one: `FONT` 12 is the RIGHT HEIGHT in the WRONG
FAMILY (family 0, fifteen rows), `FONT` 394 is the right family at the WRONG
HEIGHT (Geneva at 10pt, twelve rows), and `FONT` 396 is the one the machine drew
with (Geneva 12, fifteen rows — the `lineHeight := 15` `mac/xzip.lst` declares).
A `FONT` id is family × 128 + point size, so family 3 owns 384–511. The real
`MacOS_6.0.8_System_Startup.img` lists exactly those three at 14x15, 12x12 and
15x15.

`relfont.hfs` exists because `macfont.hfs` cannot play the release here: its
`FONT` 524 carries SQ-0916's deliberately narrow `D`, so it reads as a *typeface*
rather than as the cell — the opposite of what the real resource does, whose
printable set is uniformly 7. `relfont.hfs` is the same resource without that
trap, so `native_font::fit` calls it `FaceFit::Cell` and it fills the
fixed-pitch role the Macintosh's real `FONT` 524 fills.

Regenerate BOTH with:

```sh
python3 unit_tests/mk_sysfont_hfs.py unit_tests
```

The generator imports `mk_macfont_hfs.py` rather than restating the formats, so
there is still one description of HFS and one of the resource fork in this
directory. Loaded by `crates/app/tests/suites/system_face_cascade.rs`, which
copies them into a temp directory standing in for `~/.lanthorn/` — never the
real one, since a case that read the tester's own disks would pass or fail on
what they happen to own.

`kickfont.rom` — the **Amiga** side of the same cascade (SQ-1053). The Amiga's
Version 6 body face is topaz 8, which is in Kickstart ROM and on no floppy
Commodore shipped, so exercising that rung needs a ROM. **A real Kickstart is
copyrighted Commodore code and is never committed here**; this is a 256 KiB
image whose every byte is invented, shaped like one: the length that maps it at
`$FC0000`, the `JMP` a Kickstart opens with, and three `TextFont` records.

| record | plays | why it is there |
|---|---|---|
| `topaz/8` | the face the machine drew with | 8x8, which at the Amiga's system-face scale of (1, 2) IS the 8x16 cell |
| `topaz/9` | the right family at the wrong size | 10x9 — eighteen native rows against sixteen, so the cascade declines it (Kickstart 1.2 really carries a second topaz of that geometry) |
| `ruby/8` | a Workbench display face | byte-identical geometry to `topaz/8`, so it passes every fitness test there is and **only the name** keeps it out |

It is also a shape test for the finder: the two topazes put their name pointers
in *different* slots of the uninitialised `tf_Message` preamble, because a ROM
image does not set those link fields and `blorb::amiga_font` looks for the name
rather than indexing a fixed offset. Glyph bitmaps are reproducible without a
table — row `y` of code `c` is the byte `(c + y) & 0xFF` — so a test can assert
on the pixels.

```sh
python3 unit_tests/mk_kickfont_rom.py
```

Loaded by `crates/app/tests/suites/amiga_rom_face.rs`, alongside one case that
reads the player's own `~/.lanthorn/*.rom` and skips vacuously without one — the
only thing in the suite that can say whether the *question* was right rather
than whether the code answers it.
