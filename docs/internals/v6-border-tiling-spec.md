# Infocom Version 6 side borders: how a decorated column fills a taller screen

**Standing of this document.** This is an independent functional description,
written for the lanthorn project and published under the repository's
BSD-3-Clause licence. It states *what* a Version 6 renderer must do with
Infocom's side-border artwork when the window is taller than the artwork was
drawn for. It contains no code, no function names and no transcription from any
other implementation. Every number in §2 was measured, here, off committed
screenshots of the retail games running on real hardware under emulation; every
number in §3 and §4 was measured off Infocom's own picture archives by
lanthorn's test suite and is reproduced by those screenshots. §7 compares this
model against a GPL-licensed reference implementation at the level of *rules*,
because a project that may not derive from that source still needs to know where
it agrees and where it does not; nothing is quoted from it.

It is written to be implementable by someone who has read nothing else, and it
is deliberately title-agnostic: three Infocom titles ship side borders, the
mechanism knows about none of them, and a fourth would be handled by the same
rules.

---

## 1. The problem, and what the original machines did

Three graphical Version 6 titles frame the story window with artwork down the
left and right edges — *Zork Zero*'s architectural pillars, *Arthur*'s knotwork
poles, and *Shogun*'s lacquer panels. All of it was authored for a 320x200
display, and each machine drew it into a screen of exactly that shape.

**On the original hardware nothing is tiled, because nothing needs to be.** That
is worth stating first, because it is the one fact a screenshot can settle
outright and it bounds the whole problem: the artwork either fills its screen
exactly or falls short of it, and where it falls short the original interpreter
simply leaves the frame open. Measured on the captures in
`machine-screenshots/` (§2), the three titles do all three possible things:

| title, press | flank art | the screen | what the player saw |
|---|---|---|---|
| *Zork Zero*, Amiga | 400 rows | 400 rows | pillar stands on the bottom edge |
| *Zork Zero*, Macintosh mono | 300 rows | 300 rows | pillar stands on the bottom edge |
| *Arthur*, Amiga | 368 rows, ending 21 rows early | 400 rows | frame open below the poles |
| *Arthur*, Macintosh colour | 368 rows, ending 32 rows early | 400 rows | frame open below the poles |
| *Shogun*, Amiga | 400 rows | 400 rows | panel runs off the bottom edge |

So the extension described below is **not** a fidelity requirement. It exists
because a modern terminal pane is an arbitrary number of rows tall, and a border
that stops two thirds of the way down a tall pane looks broken in a way it never
did on a 200-line display. The requirement it *does* carry is that the extension
must be indistinguishable from more of the same artwork: every row it adds is a
row Infocom drew, stamped whole, never stretched and never resampled.

---

## 2. Ground truth: the captures, measured

All measurements below are mine, taken from the PNGs committed in
`machine-screenshots/`. Each capture's game screen was located first (the row
range over which the machine's own display sits inside the capture), and every
row number is then given **relative to the game screen's top row**, in the
artwork's own row units. Three of the five captures turn out to be 1:1 in those
units, which is what makes them usable as an oracle at all.

A capture is a fixture with a machine, a release and a moment in the game. Two
of the DOS captures are non-integer-scaled video grabs and are usable for
*structure* only; that is called out where it applies.

### 2.1 `amiga-zorkzero.png` — Zork Zero, Amiga, Banquet Hall (castle border)

720x485 capture. The Amiga display occupies capture rows **47..447**, i.e. 400
rows, at 1:1 with the doubled 640x400 screen. The left flank's shaft occupies
capture columns 57..105 (48 columns); the banner above it is wider than that and
runs off the crop.

Measured over capture columns 30..130, screen-relative rows, as the run-length of
the flank's painted horizontal span:

| screen rows | count | painted span | what it is |
|---|---|---|---|
| `0..68` | 68 | full width of the top strip | the frame's top strip, drawn across the screen |
| `68..82` | 14 | 74 columns | the capital, and the ring under it |
| `82..374` | **292** | **48 columns** | the plain shaft |
| `374..376` | 2 | 56 | the base's first flare |
| `376..378` | 2 | 64 | |
| `378..380` | 2 | 68 | |
| `380..400` | 20 | 70 | the plinth |

The right flank is the same profile, mirrored, to the row.

So this capture states the three sections directly: **cap `0..82`, repeating
middle `82..374` (292 rows), foot `374..400` (26 rows)**, and the foot's last row
is the screen's last row.

A second measurement on the same columns: the shaft is periodic in the pixels at
a period of **4 rows**, and that periodicity holds from screen row **218** to
row **374**. Both readings are true and they disagree about where the middle
starts; §4.2 says which one wins and why.

### 2.2 `amiga-arthur-church.png` — Arthur, Amiga, Church (St Anne's Day, Compline)

722x570 capture. The Amiga display occupies capture rows **35..435** (400 rows,
1:1). The left flank is capture columns 49..59 (11 columns); its ink runs capture
rows 46..414, i.e. **screen rows 11..379**.

Run-length of the painted span over those 11 columns:

| screen rows | count | span | what it is |
|---|---|---|---|
| `11..15` | 4 | 6 | pole |
| `15..49` | **34** | 11 | ornament |
| `49..79` | 30 | 6 | pole |
| `79..113` | **34** | 11 | ornament |
| `113..145` | 32 | 6 | pole |
| `145..179` | **34** | 11 | ornament |
| `179..379` | **200** | 6 | plain pole, to the art's own end |

Three ornaments of *exactly equal height*, spaced down the column, then two
hundred rows of plain pole — and then **nothing**. Screen rows 379..400 in those
columns are unpainted: on a real Amiga the frame is open for the last 21 rows.

Pixel-exact periodicity: the pole is constant (period 1) from screen row **185**
to 379, 194 rows. The six rows 179..185 are 6 columns wide but not yet
pixel-identical — the taper out of the last ornament — which is why a
silhouette reading puts the plain pole at 179 and a pixel reading puts it at 185.

### 2.3 `mac-arthur.png` — Arthur, Macintosh colour, Churchyard

652x423 capture. The flank profile is the same drawing at a different width:
**6x4, 20x34, 6x30, 20x34, 6x32, 20x34, 6x200**, 368 rows in total, ending at
capture row 399 with nothing below it inside the captured area. Placed on the
Macintosh colour press's 400-row screen this is screen rows `0..368` — the poles
start at the very top edge and stop **32** rows above the bottom.

Two things follow, and both matter to an implementer. The ornament *heights* are
identical to the Amiga's (34/30/34/32/34/200) while the ornament *widths* are
not (20 against 11) — so a rule keyed on how wide a flank is will not transfer
between presses, and a rule keyed on the vertical rhythm will. And the art's
top inset differs between presses (11 on the Amiga, 0 here) while its 368-row
extent does not.

### 2.4 `mac-zorkzero-game.png` — Zork Zero, Macintosh monochrome, Banquet Hall

552x385 capture. The monochrome picture space is 480x300, and the pillar's
300 rows map to capture rows **59..359** at 1:1 — confirmed three ways, in that
the capital boundary, the ring and the base all land on the same offset.

| picture rows | what it is |
|---|---|
| `0..63` | capital |
| `63..285` | shaft — one span, **except** for a ring at `163..173` that departs by exactly one column at each edge |
| `285..300` | base, flaring, standing on the last row |

This is the case that stops "the shaft is the longest run of rows holding one
constant span" from being the whole story: measured that way this shaft is only
112 of 300 rows, because the ring interrupts it.

Cropped and magnified, the capture shows it plainly: capital, shaft, a decorative
ring roughly a third of the way down, more shaft, flared base on the bottom row.

### 2.5 `amiga-shogun-game.png` — Shogun, Amiga, Bridge of the *Erasmus*

720x570 capture. The Amiga display occupies capture rows **32..432** (400 rows,
1:1). Each flank is 46 columns (capture 36..82 and 640..676) and is painted for
**all 400 rows** — it runs off the bottom edge of the screen.

Periodicity over those columns: the longest run of rows equal to the row *p*
above them is **11 rows**, at *p* = 1. In other words there is **no vertical
period at all**. This flank is one drawing, top to bottom: no cap that can be
told from the rest, no foot, and no repeating unit smaller than the whole.

### 2.6 The DOS captures

`dos-zorkzero.png`, `dos-shogun.png` and `dos-arthur.png` are 1642x924 video
grabs of a 320x200 screen — 5.13x horizontally and 4.62x vertically, neither
integer — so row boundaries in them are worth ±3 rows at best and span
measurements wobble by several columns from scaling alone. They corroborate the
*structure* (the DOS Zork Zero pillar has the same capital / shaft / flared base
and stands on the screen's bottom edge) and they must not be used to pin a row
number. `dos-zorkzero-cga.png` is 507x317, likewise fractional.

They do settle one thing the Amiga captures cannot: the DOS renditions are
*separately drawn*, not the same drawing at another density, so their section
boundaries are their own. Our archive measurements put the Zork Zero castle
banner at 34 raw rows on the 256-colour art, 37 on EGA and 39 on CGA, against
pillars that are 166 rows on all three. **A section boundary is therefore a
property of the picture archive in front of you, not of the title.**

---

## 3. The model

A side flank is **three sections stacked vertically**, any of which may be
absent:

* a **cap** — everything above the repeating part, drawn once, at the top,
  never repeated. Typically the frame's top strip plus a capital, a banner or a
  run of ornaments.
* a **middle** — the only part that repeats. It may be a *lattice*, which
  repeats in the pixels at some period; or a *shaft*, a plain length of one
  width between a capital and a base, which has no meaningful period of its own
  but a definite extent; or, in the limit, one drawing standing in for a middle.
* a **foot** — everything below the middle, drawn once, and always at the very
  bottom of the finished column.

Every combination occurs in the corpus:

| shape | seen on |
|---|---|
| nothing | some of *Arthur*'s function-key screens |
| cap only | some of *Arthur*'s screens |
| middle only | *Shogun* (§2.5) |
| cap + middle | *Arthur* (§2.2, §2.3); the InvisiClues screens |
| cap + middle + foot | *Zork Zero* (§2.1, §2.4) |

**Extending a flank is one operation and the title never enters into it**: keep
the cap where it is, re-anchor the foot to the *new* bottom, and tile the middle
to fill whatever is between. The three sections are not constants; they are
measured from the flank's own pixels every time, which is what lets one
mechanism serve four picture archives per title and a fifth nobody has met yet.

### 3.1 The coordinate space

Everything in this document is stated in the **artwork's own row units** — the
game's picture space, vertically doubled on any press whose picture space is
half the screen's height. No section boundary, unit height or stride is ever
computed in device pixels or in text cells.

The composed column is scaled **once**, together with the rest of the frame,
after it is complete. Two consequences:

* **Art density does not enter the composition.** Where the artwork is stored at
  half the screen's density and doubled onto it (the 320x200 archives), every
  number here is in doubled rows and the 4-row inset of §4.4 is two *drawn*
  rows. Where the artwork is stored at the screen's own density (the 480x300
  monochrome archive), they are the same thing. Nothing branches on it.
* **The frame keeps its corners.** Because the side art and the top strip are
  scaled by the same factor, they still meet exactly at the corners at every
  window size. A flank that was composed at some other factor — or, worse,
  stretched vertically to fit — breaks the corner and is visibly a different
  aspect from the strip above it.

### 3.2 The two flanks are one drawing

A frame's left and right flanks are two crops of one symmetric picture. They
**must** be sectioned once and given the same answer.

They do not measure alike on their own. Macintosh *Arthur*'s poles differ by a
single pixel — the tail below the ornaments is five columns on the left and four
on the right — and that is enough for a pixel-exact period search to find a
repeat on one side and nothing on the other. The side that finds nothing falls
back to treating the whole drawing as the middle and mirrors its *cap* down the
column, which is visible as "the right side copies portions of the banner when
tiling, the left side is correct".

Combine the two readings conservatively, and symmetrically, so that each side
computes the same answer from the same pair without knowing which side it is:

* the **cap** is the *later* of the two — neither side repeats what the other
  calls cap;
* the **foot** is the *earlier* of the two — neither side tiles over what the
  other calls foot;
* a **measured period** from either side is believed, and the smaller wins when
  both found one. Failing to find a repeat is absence of evidence, not evidence
  that the drawing does not repeat.

---

## 4. The rules

Throughout: the artwork occupies rows `[0, H)` of the flank's own columns; the
cap is `[0, a)`, the middle `[a, e)` of height `b = e − a`, the foot `[e, H)` of
height `c = H − e`; and `S` is the number of rows the finished column must
cover.

### 4.1 The guard

**Extend only when `S > H`.** If the target is no taller than the artwork, do
nothing: the column is the artwork as drawn, cropped by whoever is placing it.
A window shorter than the art shows the art's top and loses its bottom, which is
exactly what a small window did on the original machines.

This is one guard, not three; it does not vary by title, and a flank that needs
no extension must not be pushed through the machinery anyway, because every arm
below assumes there is somewhere to put a copy.

### 4.2 Finding the middle

The middle is found in the **pixels**, not in the silhouette, and it is found two
ways because there are two kinds of middle.

**A lattice** is found by periodicity: a stretch of rows for which every row is
identical to the row `p` rows above it. Search `p` from a quarter of a text row
up to eight text rows, and take the longest qualifying stretch; among equal
lengths take the smallest period, because a stretch periodic at `p` is also
periodic at `2p` and the smaller unit tiles with fewer seams. Three conditions
qualify a candidate, and all three are load-bearing:

1. **It must reach the foot.** Measure where the foot starts *first*, and accept
   a candidate only if it comes within one period of it — one period, because
   the last copy before the foot need not be whole. The model is cap, then
   middle, then foot, in that order, and only the middle repeats; so the
   repeating stretch that abuts the foot is the middle and everything above it
   is cap, *however much of the cap also repeats*. A cap may repeat: *Arthur*'s
   three ornaments are one drawing stamped three times and agree at a period of
   64 over 128 rows, where the plain pole beneath them agrees at period 4 over
   110. Ranked by length alone the ornaments win, which makes the pole into cap,
   the pole's taper into the middle, and prints the ornament motif down the side
   of the screen.
2. **It must hold for two whole copies.** A stretch shorter than `2p` is not a
   repeat, it is a coincidence. Apply this while searching, not to the winner
   afterwards: an invalid candidate that wins the search and is then discarded
   takes every valid candidate with it. Measured on *Arthur* at the Churchyard
   frame, the left pole's unfiltered winner was a 172-row stretch at period 112
   — not two copies of anything — and the period-4 pole beneath it was never
   reported at all.
3. **It must be at least one text row tall.** A repeat thinner than that is
   texture, not structure. It also has a mechanical edge: the inset of §4.4
   removes two drawn rows from each end of the unit, so a middle of eight rows
   or fewer yields no unit and the extension silently draws nothing. *Shogun*'s
   lacquer is where that bites — it agrees with itself for eight rows in two or
   three places, some of them at the very bottom where the reach test cannot
   exclude them.

**A shaft** is found from the silhouette: the longest run of consecutive rows
whose opaque column span (first and last painted column) is identical. It counts
as a shaft only when

* that span is strictly **narrower** than the flank's widest painted row — there
  is a capital or a base wider than it, which is what makes the shape a pillar
  rather than a slab;
* something is painted above it and something below it, so there is a cap to cut
  beneath and a base to put back; and
* it is **most of the flank** — more than half. A pillar is mostly shaft; a run
  that holds for less than half the art is a coincidence in a textured surface.

Compare rows by *span*, not by ink count: a dithered rendition's shaft wobbles in
pixel count row to row while its edges do not move at all.

The majority test is what keeps a border symmetric. Measured over the four native
*Zork Zero* archives, with each scene's flanks composed the way the game composes
them, the longest constant-span run is 280–292 rows of 400 for the castle border
(**70–73%** on every rendition and both flanks), and at most 180 of 400
(**36%**) for the underground and jungle borders, which are alternating stone
blocks and foliage. The two families are separated by the gap 36%..70%, so the
cut at half the flank is in it with margin at both ends and needs no fitting.
Without it, six of the eight non-castle flank *pairs* measured derived different
recipes for the two halves of one symmetric border.

**A banded shaft** is the same idea with a tolerance, for the Macintosh
monochrome column of §2.4: the longest run of rows whose span stays within **one
column at each edge** of some reference row's, the reference being scanned over
every row rather than taken from the run's first. One column, not two: at two,
the underground border's masonry starts declaring shafts again and the flanks
stop agreeing. Anchoring to the run's first row instead of scanning finds 102 and
122 rows on the two flanks — neither a majority — where scanning finds all 225.

**When both a period and a shaft exist, the shaft wins.** A shaft is
architecture the drawing *declares* — a capital above it, a base below it —
where a period is *inferred* from rows that happen to agree, and a uniform
capital agrees with itself. This is the §2.1 disagreement, and it is exactly why
it matters: the capture's shaft runs from row 82 and its pixel period runs from
row 218, and preferring the inferred reading puts the middle 136 rows *below*
where the shaft starts, so the ring under the capital lands inside the repeat
unit and is tiled down the whole column as a horizontal seam. Take 82.

Conversely a lattice has no shaft to declare — *Shogun*'s runs to the bottom at
one width, so there is no base under it and no shaft to find — and there
periodicity is the only thing that answers.

**When neither answers**, the flank is one drawing: the middle is the whole of
it, the cap and the foot are empty, and the period is its own height. That is
§2.5's case and the model still describes it.

### 4.3 Finding the foot

**A foot is found by its flare, not by the period.** The "usual" span of a
column is the *modal* one — whatever the most rows of that column are — which
for a border is its shaft or its lattice. A foot departs from that mode and a
cap does not exist at the bottom, so the trailing run of rows whose span is not
the modal span **is** the foot; if the last row is already modal, the flank has
no foot.

*Zork Zero*'s is the shape that names it: §2.1's base flares to 70 columns
against a 48-column shaft, and the departure is exactly 26 rows deep.
*Shogun*'s lattice runs to the bottom at one width and correctly reports no
foot at all.

Getting this wrong is wrong twice over: a repeat that runs to the last row tiles
the foot away, *and* the band then ends on a fragment of shaft instead of on the
art's own last row — bare shaft below the feet.

### 4.4 Extending a flank that HAS a foot

This is the *Zork Zero* case, and it is the one where the composition's end is
fixed: **a flank with a foot always ends on its foot.**

1. **The repeat unit** is the middle less an inset of two *drawn* rows at each
   end: source rows `[a + i, e − i)` where `i` is two rows in the artwork's own
   density (four rows in a doubled space). The inset is not decoration. A
   shaft's first and last rows are *transitions* — into the capital above and
   the base below — and repeating a transition steps the shading at every join.
   The inset is measurable rather than assumed: on the 256-colour castle art the
   shaft runs `82..374`, and the unit that tiles without a visible join is
   `86..370`.
2. **Keep the artwork's rows `[0, e)` exactly as drawn**, and clear everything
   from row `e` down. The foot's original rows must be tiled over, not left
   where the game drew them, or the foot appears twice — once part of the way
   down and once at the re-anchored bottom.
3. **Tile downward from row `e`**, stamping the unit whole every `u` rows
   (`u` = the unit's height; the stride is the unit, with no overlap), for as
   long as a copy's top row is at or before `S`. A copy is always stamped
   *whole*; it may overshoot.
4. **Stamp the foot at `[S − c, S)`**, having first cleared everything from
   `S − c` down. This is what truncates the last copy, and it is why the copies
   may overshoot in step 3: the foot's erase is the only cut in the band, and it
   is at the bottom. Nothing is clipped from the top and nothing is centred.
5. **Orientation.** Alternate each copy's vertical flip when the middle was
   *not* found periodic, and stamp every copy upright when it was — see §4.6.
   When alternating, the copy **adjacent to the artwork** is the flipped one.

An implementation detail that is worth stating as a rule because it is invisible
until it bites: **snapshot the repeat unit before clearing anything.** If the
unit's source rows are read out of the same buffer that step 2 clears, and the
two overlap, the extension comes out blank. In the geometry above they do not
overlap — the unit ends `i` rows above `e` and the clear starts at `e` — but the
constraint costs nothing to honour and a future arm that cuts its unit lower
would depend on it.

### 4.5 Extending a flank with a BANDED shaft

A shaft with a decorative band in it (§2.4) cannot be composed by §4.4, and both
reasons are visible on screen:

* **The remainder.** A fixed stride ends wherever it happens to reach, so the
  gap between the last band and the foot is whatever is left over — never the
  gap the artwork was drawn with. Measured on the Macintosh monochrome column: a
  last band standing 183 rows above the foot where the picture itself stands
  123.
* **The mirror.** On a plain shaft a mirrored copy is indistinguishable from a
  translated one. On a banded one it *moves the band*, by twice the band's
  offset from the unit's centre.

So compose it the other way round. The repeat unit is the whole of the pillar
**below its capital** — shaft, band and foot together, rows `[a + i, H)` — and
`k` further copies are laid at a stride that divides the extension exactly:

* `x = S − H` is what the copies below the first must cover;
* `k = ceil(x / u)` is the fewest copies that can, so the stride is as long as
  the artwork allows and never longer;
* copy `j` (for `j` = 1..`k`) is stamped with its top at `a + i + (j·x)/k`.

Each copy overwrites the one above it from its own top down, so only the last
copy's foot survives; the rest contribute shaft and bands. Copy `k` sits at
`a + i + x`, so its foot ends exactly on row `S`, and nothing may follow it.

The rhythm this keeps is the artwork's own at every window height: the
capital-to-first-band distance and the last-band-to-foot distance are both
exactly what the picture was drawn with, because both come from an unmodified
copy of it, and the band-to-band distance is uniform to within one row (the
extension divided into `k` whole rows leaves a remainder that has to land
somewhere). A taller pane gets **more** bands, not longer gaps.

### 4.6 Extending a flank with NO foot

This is *Arthur* (cap + middle) and *Shogun* (middle only).

1. **The repeat unit** is the largest whole number of periods that fits in the
   middle, taken from the middle's **end**: rows `[e − k·p, e)` where
   `k = floor(b / p)`. Anchoring at the end continues the phase already on
   screen, and taking whole periods rather than one period keeps the section's
   texture — a 292-row middle that reads as a 16-row period would otherwise
   stamp the same sixteen rows eighteen times where the artwork itself varies
   down its length. Where the middle has no period of its own, `p` is its whole
   height and the unit is the whole middle.
2. **Fill downward from row `e`** — not upward from the bottom. There is no foot
   to land on, so the row the band ends on is whatever the fill leaves at the
   window's edge, and that is correct: a foot-less flank was *measured* to end
   on nothing in particular, and every flank that reaches this arm runs off the
   bottom of the screen with nothing closing it.
3. **Whole copies, then a fragment.** `g = S − e` rows to fill; `w = g / u`
   whole copies at `e`, `e + u`, …; then a fragment of `r = g − w·u` rows.
4. **The fragment is the leading part of the copy that would have come next, in
   that copy's own orientation** — the unit's first `r` rows for an upright
   copy, its last `r` rows read upward for a flipped one. The phase therefore
   carries on through it and the band's one cut is the window's own bottom edge.
5. **Orientation**, as in §4.4: alternate the flip when the period was not
   measured, starting with the copy adjacent to the artwork; never flip when it
   was.

**Why filling downward, and why the fragment's end matters, is the whole of what
one title shows.** *Shogun*'s lacquer has no sub-period, so its unit is the whole
400 rows; a 586-row target leaves 186 — less than one copy — so `w` is zero and
**the fragment is the entire extension**. Cut from the wrong end it puts art row
214 directly under art row 399: measured on the Amiga press at the gameplay
frame, 60 columns in, an **18.18** luminance step across that join against the
6.93 the drawing typically steps by itself. Cut as rule 4 says, with the flipped
copy adjacent to the art, the step is **0.00** — a flipped copy opens on the row
the artwork closed with, so the vine reverses at the join and reads as a
symmetric blossom rather than as a break.

### 4.7 The orientation rule, stated once

Whether a copy is mirrored follows from whether the period was **measured from
repeating pixels** or is merely the section's own height standing in for one.
The two cases are opposite and neither is a preference:

* **A measured period is a lattice.** The copy meets its own motifs exactly, so
  there is no seam to hide, and a flip only turns the motifs upside down —
  which on a directional ornament means it hangs inverted for hundreds of rows.
  Stamp every copy upright.
* **An unmeasured period is a shaft or a single drawing.** The copy does *not*
  meet itself. A pillar under light is the sharp case: one rendition's masonry
  is *lit*, mean row luminance falling steadily from capital to base, so a
  translated repeat butts its darkest row against its brightest and resets the
  shading at every join — measured at a **17.54** step against the **9.39** the
  shaft itself ever steps between adjacent rows. Reversing alternate copies makes
  each join an exact duplicated row instead, and the shading folds back on
  itself with nothing to show. On the flat renditions of the same border a
  mirror and a translation are indistinguishable, so nothing is lost by applying
  the rule uniformly.

Where the rule applies, **the copy adjacent to the artwork is the flipped one**,
in both the with-foot and the without-foot arm. That is what makes the first
join — the one where the drawing has to pick itself up again — continuous.

### 4.8 Do the two flanks differ?

**No, and they must not.** Left and right are two crops of one symmetric
picture; they are sectioned together (§3.2) and composed by the same rules with
the same numbers. A one-row difference in a detected foot shifts each side's
tiling phase independently and the two flanks visibly drift apart down a tall
pane.

The one asymmetry in the corpus is in the *artwork*, not the mechanism: one
*Zork Zero* scene plate's right crop begins at picture row 2 where its left crop
begins at 0 — two blank rows in the drawing. That is why the recognition
tolerances in §5.1 are tolerances rather than exact tests.

---

## 5. Two things a renderer must get right before any of this applies

### 5.1 What counts as a border flank at all

Extending something that is not a border reprints a *picture* down the side of
the screen. Two measurements separate a border flank from everything else, and
the corpus pins both cuts with margin:

* **Shape.** A pillar narrows below its banner; a slab holds one width from top
  to bottom. Narrowest ÷ widest painted row is **0.96–1.00** across every
  *Shogun* rendition and both flanks, and **0.02–0.81** across every *Zork Zero*
  rendition and all three of its scene borders. The cut is at **9/10**, in the
  gap. Note that "reaches the bottom of the screen" is *not* sufficient on its
  own: *Shogun*'s DOS artwork is drawn for the full 200-line screen where its
  Amiga artwork stops at 168, so a bottom-reaching test alone hands a Japanese
  lacquer frame the masonry recipe.
* **Inset.** A pillar under a banner *spans* its screen — the banner is painted
  to the top edge and the pillar stands on the bottom one — and the only thing
  between the art and the frame is the rounding of the screen height up to a
  whole text cell. Charge that rounding **once**, against
  `top + (screen_height − bottom)`. Measured in-game: *Zork Zero* 0 or 4;
  *Shogun* (DOS) 0; *Arthur* 29 or 32 on every press it has. A factor of seven,
  and measuring only the bottom leaves the top free — which is how an *Arthur*
  plate that is fifteen rows short of the bottom, one row inside a bottom-only
  tolerance, and narrows like a pillar, got the masonry recipe stamped down its
  side.

A flank also has to **run the height of the frame**. Artwork that leaves more
than a quarter of the frame unpainted is a picture that happens to reach the
screen's edge, and extending it lengthens a picture instead of a border.

Finally, **a band thinner than a text row is dither, not architecture.** A
rounded corner where a top panel meets a flank is eight rows deep; counted as a
capital it turns a plain slab into a pillar by twelve pixels, and the slab is
then handed a foot it does not have.

### 5.2 Which buffer the repeat unit is cut from

A renderer that draws some of the frame as text cells and the rest as artwork
holds two canvases: the artwork as the game painted it, and the composite it is
about to ship, from which the cell-drawn parts have been cleared. **The repeat
unit must be cut from the artwork canvas, and stamped into the composite.**

Cut from the composite instead and the unit repeats the *holes*. One title's
two-row status line is 32 rows of artwork that its border sits behind; cutting
from the composite copies that hole into every copy, and where two copies meet
the two holes meet as a single band of nothing. Measured at a 120x90 terminal:
64 transparent rows centred on the join, which the frame's scale put on screen
as a 94-pixel black band across the border.

---

## 6. Worked examples

All rows are in the artwork's own units. `H` = the artwork's height, `a` / `b` /
`c` = cap / middle / foot heights, `S` = the rows the finished column must
cover, `i` = 4 (a two-drawn-row inset at each end of the unit, in a doubled
space).

### 6.1 A flank with a foot — `H` = 400, `a` = 82, `b` = 292, `c` = 26

Cap `[0, 82)`, middle `[82, 374)`, foot `[374, 400)`. No measured period (a
declared shaft), so copies alternate and the first is flipped. Unit = rows
`[86, 370)`, `u` = 284.

**`S` = 400 — the original screen.** `S` is not greater than `H`: nothing is
extended. The column is the artwork, foot on the bottom row. This is exactly
what §2.1 measures on real hardware.

**`S` = 300 — a window SHORTER than the artwork.** Again `S ≤ H`, so nothing is
extended. The column is the artwork, cropped by the caller to its top 300 rows:
cap `[0, 82)` and 218 rows of shaft. The foot is not re-anchored and is not
drawn — it lies below the visible band. The frame's bottom edge is simply
off-screen, as it was in a short window on the original machine.

**`S` = 800 — a whole number of copies plus a remainder.**

| rows | content |
|---|---|
| `[0, 374)` | the artwork as drawn — cap and the original middle |
| `[374, 658)` | copy 1, **flipped**: unit rows 283 down to 0 |
| `[658, 774)` | copy 2, upright, truncated: unit rows 0..116 |
| `[774, 800)` | the foot, exactly the artwork's rows `[374, 400)` |

Copy 2 was stamped whole at row 658 and would have covered `[658, 942)`; the
clear at row 774 cut it. A third copy would have started at 942, past `S`, so
none was stamped. Remainder: 116 of 284 rows.

**`S` = 1000 — three copies, a different remainder.**

| rows | content |
|---|---|
| `[0, 374)` | the artwork as drawn |
| `[374, 658)` | copy 1, flipped |
| `[658, 942)` | copy 2, upright |
| `[942, 974)` | copy 3, flipped, truncated to 32 rows: unit rows 283 down to 252 |
| `[974, 1000)` | the foot |

**`S` = 660 — a copy that is stamped and then wholly discarded.** Copies are
stamped at 374 and at 658 (658 ≤ 660). The clear at row 634 removes the second
copy entirely.

| rows | content |
|---|---|
| `[0, 374)` | the artwork as drawn |
| `[374, 634)` | copy 1, flipped, truncated to 260 of its 284 rows |
| `[634, 660)` | the foot |

This is the case that makes the ordering of "stamp, then clear from `S − c`"
non-negotiable: a routine that decided how many copies to stamp by dividing
`S − c − e` by `u` would stamp one here and be right, but the general rule is
simpler and always right — stamp while the top row is at or before `S`, then let
the foot's clear decide where the band ends.

### 6.2 A flank with no foot and no measured period — `H` = 400, middle `[0, 400)`

Cap and foot both empty; `u` = 400; copies alternate, the first flipped.

**`S` = 586.** `g` = 186, `w` = 0, `r` = 186. The first copy would have been
flipped, so the fragment is the unit's **last** 186 rows read upward:

| rows | content |
|---|---|
| `[0, 400)` | the artwork as drawn |
| `[400, 586)` | art rows 399 down to 214 |

Screen row 400 carries art row 399, so the join is an exact duplicated row.

**`S` = 900.** `g` = 500, `w` = 1, `r` = 100.

| rows | content |
|---|---|
| `[0, 400)` | the artwork |
| `[400, 800)` | copy 1, flipped: art rows 399 down to 0 |
| `[800, 900)` | fragment of the next (upright) copy: art rows 0..100 |

Again both joins are duplicated rows: 399 meets 399 at row 400, and 0 meets 0 at
row 800.

### 6.3 A flank with a cap, a measured period and no foot — `H` = 400, art `[11, 379)`

Cap `[11, 185)` (four rows of pole, three 34-row ornaments and the gaps between
them, plus the six-row taper), middle `[185, 379)` at a measured period of 1.
Because the period was measured, **no copy is flipped**. `u` = 194.

**`S` = 600.** `g` = 600 − 379 = 221, `w` = 1, `r` = 27.

| rows | content |
|---|---|
| `[0, 11)` | unpainted, as the artwork is |
| `[11, 379)` | the artwork: ornamented cap, then 194 rows of plain pole |
| `[379, 573)` | copy 1, upright |
| `[573, 600)` | fragment: the unit's first 27 rows |

Since this middle is a constant texture, the visible result is a plain pole from
row 185 to the bottom of the window and no join anywhere. That is the point: the
mechanism does not need to know it was *Arthur*.

### 6.4 A banded shaft — picture space 300 rows, cap `[0, 63)`, band at `163..173`, foot `[285, 300)`

`H` = 300, `a` = 63, unit = rows `[67, 300)`, `u` = 233. The band sits 96 rows
into the unit; the artwork's own capital-to-band distance is 100 and its
band-to-foot distance is 122.

**`S` = 500.** `x` = 200, `k` = ceil(200/233) = 1. One further copy, at
`67 + 200` = 267, covering `[267, 500)`.

| rows | content |
|---|---|
| `[0, 267)` | the artwork's cap, shaft and first band (at `163..173`) |
| `[267, 500)` | copy 1 — shaft, band at `363..373`, foot at `[485, 500)` |

Band-to-band = 200. Band-to-foot = 122, the artwork's own. Foot ends exactly on
row 499.

**`S` = 800.** `x` = 500, `k` = ceil(500/233) = 3. Copies at
`67 + (1·500)/3` = 233, `67 + (2·500)/3` = 400, and `67 + 500` = 567.

| rows | content |
|---|---|
| `[0, 233)` | the artwork's cap and first band (at `163..173`) |
| `[233, 400)` | copy 1 (band at 329), overwritten from row 400 by copy 2 |
| `[400, 567)` | copy 2 (band at 496), overwritten from row 567 by copy 3 |
| `[567, 800)` | copy 3 — band at 663, foot at `[785, 800)` |

Bands at 163, 329, 496, 663: gaps of 166, 167, 167 — uniform to within one row,
which is the remainder of 500 ÷ 3 landing somewhere. A taller window gets a
fourth band, not longer gaps.

---

## 7. Divergence from the GPL reference

lanthorn's border extension began close to Spatterlight's Bocfel
(`terps/bocfel/z6/draw_border.cpp` and its Zork Zero caller, both
**GPL-2.0-or-3.0**) and has since been rewritten and re-derived from
measurement. This section compares the two **models**, rule by rule, so that the
provenance audit's D7 verdict can be settled on evidence rather than on the word
"port" in a comment. Nothing below is quoted; the reference was read to
establish what its rules *are*, and each row states whether ours agrees, differs,
and which of the two has an independent source.

The reference's shape, in one paragraph and in its own terms: section boundaries
are **compile-time constants**, chosen per title *and* per graphics format, and a
routine is selected by that pair. Its generic pillar extender takes a top cut, a
foot height, a total height, a repeat height, an overlap and a flip flag; it
snapshots the repeat strip and the foot, clears the foot's rows, tiles from the
old foot position with a stride of *repeat height minus overlap*, alternating the
flip if asked, then clears from the new foot position down and stamps the foot
there — and finally trims the whole pixmap to the target height so the last
tile's overshoot is never displayed. Four further bespoke routines handle a
Macintosh monochrome castle (upper / ring / lower / foot, tiled and with the ring
re-placed), an underground border (alternating left and right rectangles, with a
single "stone" erased near the bottom so the foot lands flush), a jungle border
(one overlapped strip, no foot at all), and a hint border whose foot is drawn
with white treated as transparent.

| # | Rule | This model | The reference | Verdict |
|---|---|---|---|---|
| 1 | Where the three sections are | measured from the flank's own pixels, every time | fixed constants, chosen per title **and** per graphics format | **differ** — ours measured, theirs not |
| 2 | The castle border's numbers | measured: cap `0..82`, middle `82..374`, foot 26 (§2.1, §4.2) | 43 / 13 / 200 / 142 raw rows, i.e. cut 86, foot 26, total 400, unit 284 doubled | **agree to the row** — and ours is independently confirmed by a real Amiga capture, so the agreement is corroboration, not derivation |
| 3 | Inset at each end of the repeat unit | two drawn rows, because a shaft's first and last rows are transitions (§4.4) | the same two rows, implied by its cut being 4 above the measured shaft top and its unit 8 shorter than the measured shaft | **agree in value**, differently sourced: ours states the reason and can be re-measured on any archive |
| 4 | Seam-hiding by overlap | none — the stride is the unit | per-art overlaps (0, 9, 11, 20, 33, 59 depending on the asset) | **differ** |
| 5 | Seam-hiding by mirroring | derived: mirror exactly when the period was *not* measured; the copy adjacent to the artwork is the flipped one (§4.7) | hard-coded per format: one rendition alternates, another forces only its first tile flipped | **differ** — same device, opposite sourcing |
| 6 | The guard | extend only when the target exceeds the artwork | the same | **agree** — and it is the only composition either could have |
| 7 | Snapshot the unit before clearing | honoured, but the unit and the cleared region do not overlap in this geometry (§4.4) | load-bearing: its cut for one title sits *inside* the region cleared immediately below | **differ in necessity** — for us it is a cheap invariant, for them a correctness requirement |
| 8 | Where the remainder lands | bottom: the foot's clear truncates the last copy, or the fragment carries the phase to the window's edge | the last tile is stamped whole, overshoots, and the pixmap is then trimmed to the target height | **agree in effect, differ in mechanism** — we cannot trim, because the band occupies a rectangle the caller has already fixed |
| 9 | *Arthur* | cap + middle, **no foot**; the measured middle repeats (§6.3) | cut at 90% of the artwork's height, repeat a **two-row** strip, treat the last 10% as a foot, and nudge that foot up onto an even row | **differ substantially**; the even-row nudge is deliberately not done here — it would leave an unpainted sliver at the bottom of a band we cannot trim |
| 10 | *Shogun* | one drawing, no cap, no foot; mirrored copy adjacent to the artwork, fragment cut so the phase carries (§6.2) | stamp one further whole copy of the border (flipped for one border picture, plain for the other) at platform-specific offsets, then a generic strip repeater | **differ** |
| 11 | Macintosh monochrome castle | `k` whole copies at a stride dividing the extension, every band kept, the artwork's own capital-to-band and band-to-foot distances preserved (§6.4) | tile an *upper* and a *lower* section alternately, then re-place the **single** ring at the vertical centre of the column, then the foot | **differ** — ours yields `k` evenly spaced bands, theirs exactly one, wherever the middle happens to be |
| 12 | Other *Zork Zero* scene borders | no special arm: they decline a shaft and take the generic reading | two further bespoke routines (alternating side rectangles with an erased "stone"; a foot-less overlapped tiler) | **differ** — we have no equivalent of either |
| 13 | Left/right agreement | the two crops are sectioned together and their readings combined conservatively (§3.2) | not applicable: one constant per art asset, so both sides agree by construction | **different route to the same requirement** |
| 14 | Clipping | the band is exactly the requested rows; nothing may overshoot it | the composed pixmap is trimmed after the fact | **differ** |
| 15 | Transparent foot | none | a dedicated mode for one Macintosh hint border, drawing the foot with white as transparent | **absent here** |

**What this adds up to.** Of fifteen rules, three coincide: the guard (6), the
three-section shape, and the castle border's numbers (2, 3). The guard is the
only sensible one. The three-section shape is a property of the *artwork* and
§2.1 measures it off a photograph of an Amiga. The castle's numbers are the same
in both, and the reason is now settled: they are what the drawing is, and both
implementations arrived at them — one by hard-coding a measurement, the other by
taking one. **Every other rule differs**, several of them in ways that produce a
visibly different screen (5, 9, 10, 11, 12).

So the correct reading of the current code is that it is **a re-derivation, not a
port** — the "port of" language and the ordering caveat attributed to the
reference are the last residue of an ancestry the algorithm has otherwise left
behind. Two facts should nonetheless be re-sourced rather than merely re-worded,
because the reference is currently their only stated authority:

* the **inset** of two drawn rows — re-source to §4.4's reason and to the
  measurement that 82 + 4 and 292 − 8 are what tiles without a join;
* the **snapshot-before-clear** ordering — re-source to §4.4's own statement of
  it as an invariant, and note explicitly that it is not load-bearing in this
  geometry.

---

## 8. Verification

### 8.1 What to compare against a screenshot

An implementer can check the *sections* directly, without any of lanthorn's own
machinery, by measuring these files:

| file | what to measure | expected |
|---|---|---|
| `machine-screenshots/amiga-zorkzero.png` | run-length of the flank's painted span, capture columns 30..130, over capture rows 47..447 | 68 rows full width · 14 rows at 74 · **292 rows at 48** · 6 rows flaring 56/64/68 · 20 rows at 70 |
| the same file | longest run of rows equal to the row *p* above, same columns | *p* = 4, holding capture rows 265..421 (screen 218..374) |
| `machine-screenshots/amiga-arthur-church.png` | painted span over capture columns 49..59, capture rows 46..414 | 6x4 · 11x34 · 6x30 · 11x34 · 6x32 · 11x34 · **6x200**, then nothing for 21 rows |
| `machine-screenshots/mac-arthur.png` | painted span over capture columns 14..34, capture rows 31..399 | 6x4 · 20x34 · 6x30 · 20x34 · 6x32 · 20x34 · **6x200**, then nothing |
| `machine-screenshots/mac-zorkzero-game.png` | painted span over capture columns 40..90, capture rows 59..359 | capital to capture row 122 · shaft at a constant span · **ring at capture 222..232, one column wider at each edge** · base flaring from capture 344 to the last row |
| `machine-screenshots/amiga-shogun-game.png` | longest run of rows equal to the row *p* above, capture columns 36..82, rows 32..432 | **11 rows at most** — no period; 400 painted rows |

The two things these settle that nothing internal can: that the section
boundaries our code measures are the drawing's own, and that on the original
machines the artwork is never repeated — so any join a player sees is ours.

### 8.2 Which existing cases pin the current behaviour

A rewrite must keep all of these green. They are the measured record and, for
several of them, the only record.

**In-crate, in the border module** (`cargo nextest run -p lanthorn --lib
--features t-render v6_border`):

| case | what it pins |
|---|---|
| `recognize_separates_the_measured_shapes` | the three layouts separate on the measured extents |
| `a_slab_that_reaches_the_bottom_is_not_a_pillar` | §5.1's 9/10 shape cut, both ends |
| `an_inset_plate_that_all_but_reaches_the_bottom_is_not_a_pillar` | §5.1's whole-inset rule, and the DOS press's one-row margin |
| `tile_down_strides_by_height_less_overlap` | stride arithmetic |
| `tile_down_alternates_the_flip_only_when_asked` | §4.7's alternation, including the initial parity |
| `extend_pillars_snapshots_the_unit_before_erasing_the_foot` | §4.4's ordering invariant, on a synthetic overlap |
| `nothing_is_extended_when_the_art_already_covers_the_band` | §4.1's guard |
| `an_extended_flank_is_painted_to_its_last_row` | no hole at the bottom |
| `shoguns_repeats_come_from_the_art_not_the_status_cleared_canvas` | §5.2 |
| `the_pillar_shaft_is_measured_from_the_art_not_pinned_to_one_banner_height` | §4.2's shaft, and that the measurement reproduces 86 / 26 / 284 |
| `a_run_shorter_than_half_the_flank_is_not_a_shaft` | §4.2's majority test |
| `a_constant_width_slab_declares_no_shaft` | a slab declines |
| `no_tile_boundary_repeats_the_ring_under_the_capital` | §4.2's "the shaft wins over an inferred period", across three banner heights |
| `arthurs_extension_keeps_his_foot_at_the_bottom` | the band's last row is the section that belongs there |

**Integration suites** (each skips vacuously without its `stories/` fixture, so
symlink `stories/` into the worktree — a vacuous skip reads exactly like a pass):

| suite | what it pins |
|---|---|
| `v6_side_border_tiling` | every flank is tiled and none is stretched; the frame reaches the viewport's bottom; the side art and the top strip share one horizontal scale at every pane width; no join steps harder than the artwork does by itself (§4.7's luminance numbers); every rendition is recognised as its own title's layout; raster mode ships the same frame; a picture column over a command menu is not a border; all three *Zork Zero* scene borders |
| `v6_mac_pillar_feet` | §4.5 in full — the foot is the bottom-most ink at every pane; the bands are evenly spaced to the row and the pane decides how many; **and that the colour rendition on the same disk is unmoved**, its shaft holding one span from row 82 to the foot |
| `v6_archive_border_sweep` | the six properties over 68 flanks discovered from the archives themselves — the band is filled exactly; no hole below the artwork; every extension row's span occurs among the artwork's own row spans (so a stretch, a shift or a resample cannot pass); four rows of cell rounding change nothing; a flank that has a foot ends on it; a symmetric border comes out symmetric |
| `v6_arthur_status` | the neighbour a border change is most likely to disturb: *Arthur*'s status ribbon is solid across its own window — native columns `28..612` of 640 — and the 28 native columns it leaves at each edge are exactly where his poles stand, with the frame's rule running past the ribbon unbroken |

Two habits that the record shows are worth keeping. **Falsify each case**:
temporarily undo the rule it pins and confirm it fails with the originally
reported symptom — most of the cases above carry a note saying exactly which
edit falsifies them. And **run the archive sweep on any change to the
sectioning**, because it is the only thing that reaches art no play session
visits: five of six earlier attempts at this mechanism rendered the reported
frame correctly and were caught by that sweep alone.

---

## 9. Sources

| source | what was taken from it | licence | version read |
|---|---|---|---|
| `machine-screenshots/amiga-zorkzero.png`, `amiga-arthur-church.png`, `amiga-shogun-game.png`, `mac-arthur.png`, `mac-zorkzero-game.png`, `dos-zorkzero.png`, `dos-zorkzero-cga.png`, `dos-shogun.png`, `dos-arthur.png` | every measurement in §2; the fact that the original interpreters do not tile | committed in this repository (BSD-3-Clause); the captures are of Infocom's retail games running under emulation | tree at `a8797b6b` |
| `crates/app/src/render/v6_border.rs` and its in-crate cases | the corpus measurements quoted in §4 and §5 (span ratios, insets, luminance steps, the 68-flank tallies) — lanthorn's own measurements, taken off Infocom's picture archives | BSD-3-Clause (this repository) | tree at `a8797b6b` |
| `crates/app/tests/suites/v6_side_border_tiling.rs`, `v6_mac_pillar_feet.rs`, `v6_archive_border_sweep.rs` | §8.2; the per-rendition numbers in §4.5 and §5.1 | BSD-3-Clause (this repository) | tree at `a8797b6b` |
| SQ-1063's quest record | the three-section framing as the reporter stated it, the 68-flank section table, and the history of six rejected discriminators | project tracker (not published) | read 2026-09-08 |
| Spatterlight, `terps/bocfel/z6/draw_border.cpp` | §7 only — the reference's **rules**, read to compare models. No code, constant table or comment was copied. | **GPL-2.0-or-3.0** (its own file header, "Copyright 2010-2026 Chris Spiegel") — note that the enclosing `terps/bocfel/LICENSE` is MIT and does **not** govern this file | last commit touching the file `a40650e4225a2c832c71873c3389b34dcb7463dd` (2026-07-20); repository HEAD `c315f5bf7bcd6d485951fbff5e623bc160673da2` (2026-09-08) |
| Spatterlight, `terps/bocfel/z6/zorkzero.cpp` | §7 only — which routine each border and graphics format selects, and with what section constants | **GPL-2.0-or-3.0** (same project; per-file GPL headers) | last commit touching the file `dd4fdb01ec35962989a3209f5dda3705ef94d2cc` (2026-07-25); same HEAD |

**Why the reference was read at all.** No published standard describes Version 6
border tiling — the Z-Machine Standards Document is silent, because this is a
rendering decision each interpreter makes for itself and the original games
never made it. The reference was therefore consulted to answer one question that
screenshots cannot: *what does an interpreter do when the window is taller than
any screen the artwork was drawn for?* Everything it contributed is in §7, stated
as a rule and, where it bears on this implementation, with a worked example on
numbers constructed here.
