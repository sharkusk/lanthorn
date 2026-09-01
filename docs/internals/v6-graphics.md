# Graphical Z-machine v6

> For players, the short version is in [the guide](../guide/graphics-and-terminals.md).

[← back to README](../../README.md) · see also [Interpreter](interpreter.md) · [Customization](customization.md)

Z-machine **v6** is Infocom's graphical story format — the one behind *Zork
Zero*, *Shogun*, *Journey*, and *Arthur*. It splits the screen into pixel-
addressed windows, draws pictures at exact coordinates, and expects the
interpreter to composite it all into one illustrated page. lanthorn's `zvm`
implements the v6 windowing and picture opcodes needed to run that model, and
the app renders the result. The depth below is verified against *Zork Zero*,
whose full frame — banner, side columns, the per-room exit compass, and
in-text illustrations — renders faithfully; the same engine and opcode set
targets the format's other titles, but they haven't been played through end
to end in this repo yet.

## The game lays itself out — you just answer its questions

v6 games don't hard-code their own layout. At boot, Zork Zero (and its
siblings) queries `picture_data` on a handful of pictures that are never
actually drawn — they exist purely to answer "how big is this thing," and the
game uses the answer to position its banner, columns, and compass. Those
pictures are Blorb `Rect` chunks: an 8-byte, dimension-only placeholder (width
then height, big-endian) with no pixel data at all. lanthorn recognizes a
`Rect` chunk and answers `picture_data` straight from it, which is exactly the
mechanism these games rely on — it isn't a general Blorb image feature, it's a
placement protocol these specific titles speak.

## Where the pictures come from

Most of the time: a Blorb. Either the story file *is* one, or a `.blb`/`.blorb`
sibling beside it carries the `Pict` resources, and lanthorn resolves that on its
own.

There is a second source, for anyone playing from original media. Infocom's Amiga
releases stored their artwork in a single `Pic.data` archive on the game disk — a
big-endian Huffman + run-length + per-scanline-XOR codec of Infocom's own design,
nothing to do with PNG — and lanthorn decodes it directly. Launch a game from its
[`.adf` disk image](interpreter.md#what-counts-as-a-story-file) and the archive
that shipped on that same floppy becomes the game's art. Nothing to configure:
the story and the pictures came off one disk, so the pairing is guaranteed by the
medium rather than guessed from a filename.

The Macintosh releases wrote the *same* container, so a Mac disk image works the
same way — with one wrinkle worth knowing about. Apple sold two screens, and
Infocom shipped an archive for each: a colour `CPic.data` and a monochrome
`Pic.data`. **lanthorn reads both.** It draws the colour one, because that is
what every other medium here supplies and nothing on the disk argues otherwise;
the black-and-white artwork is a thing you ask for:

```sh
lanthorn "Zork Zero Disk.image" --pictures Pic.data
```

That name is looked up *on the volume*. A story mounted out of a disk image has
no folder for a loose archive to sit beside it in, so `--pictures` reaches into
the medium when the name it was given is not on your filesystem — and every
format gets that door by the same code, which is a claim worth checking rather
than assuming. It was not true when the DOS and Atari ST readers arrived: this
one lookup still named `.adf` and HFS by hand, so a PC floppy's `ZORK0.EG1` was
offered in the dialog and would not load when picked. It goes through the same
one table as everything else now.

On a format with directories the name may carry one, because that is how the
volume spells it: an Atari ST compilation's files are `HITCHHIK/STORY.DAT` and
`CUTHROAT/STORY.DAT`, and what the dialog shows you is what `--pictures` accepts.
Amiga and Macintosh volumes are flat and behave exactly as they always did.

The PC and Atari ST floppies join the same road, with one caveat that is the
disk's fault rather than lanthorn's. A DOS release stores its art as `.MG1`
(MCGA), `.EG1`/`.EG2` (EGA) or `.CG1` (CGA) — three video cards, one machine —
and lanthorn will draw whichever of them is on the image you opened. But a PC
release is often **several** images, and the artwork does not always travel with
the story: *Zork Zero*'s story and its EGA art are on *Lost Treasures* floppy 5,
while its CGA art is alone on floppy 4. One mount is one disk, so open the one
that has the game on it and you get that disk's rendition. (The Atari
compilations are all text-only, so this never comes up there.)

You do not have to know the name, either. **The launch-options dialog lists what
is on the disk**, so opening *Zork Zero Disk.image* offers you both of its
archives by name, with the two-colour one labelled `Mac B&W` and both marked *on
disk* — because they are not in the folder you are looking at, they are inside
the image. Pick a row and that is what the game draws.

What comes back is not a recoloured copy of the colour art. It is a different
screen: the mono archive's plates are **480×300** where the colour ones are
320×200, drawn for the standard Macintosh's black-and-white display, and its
sprites are redrawn rather than scaled — the same 483 picture numbers, the same
386 of them carrying pixels, at sizes that mostly do not divide. Infocom's own
Mac interpreter says so in the flag's own definition: *"this pic is mono, and
scaled for a 480x300 screen (std Mac)"*. It also displayed that art 1:1 where it
scaled the colour art by 1.5 or 2, and lanthorn does the same — 480×300 is the
one picture space in this whole format that does not double onto the 640×400
screen.

#### Two Macintosh screens

Which raises the obvious question, and the answer turns out to be the whole
Macintosh screen model: if the mono plates are 480×300 and the colour ones are
640×400 once doubled, **what is the screen?** Both, depending on which archive
is in hand — and that is not a compromise lanthorn invented, it is one decision
in Infocom's own interpreter. It sized its window and chose its picture file on
the same test: a Mac big enough for colour got a 640×400 window and `CPic.Data`,
and everything else got a 480×300 window and `Pic.Data`. The source says it in
one breath: *"for a small window use mono gfx, for a big window use color gfx"*.

| you asked for | picture space | drawn at | the screen the story is told about |
|---|---|---|---|
| `CPic.data` (the default) | 320×200 | 2× | 640×400 — the Amiga's own unit space |
| `Pic.data` | 480×300 | 1:1 | 480×300 |

So the colour path needs nothing new: 320×200 doubled is the space every other
Infocom rendition already lands in, which is also why a Macintosh colour archive
is indistinguishable from an Amiga one — there was nothing to distinguish. And
neither path has stretched pixels. Every scaling arm in the Mac interpreter
moves both axes by the same factor, unlike the IBM PC's EGA rendition, whose
pixels really are half as wide. Both Mac archives are 1.6:1 and stay 1.6:1.

**512×342 is the hardware, and never the screen the game hears about.** The
compact Mac's screen appears in the interpreter only as the thing the game
window is centred *inside* — a 300-pixel-tall window placed at y=38 on a
342-pixel screen, which is exactly a menu bar plus a title bar. A photograph of a
real Mac *Zork Zero* is 512×342 with the game filling nearly all of it, while the
game itself is being told 480×300. The interpreter computes what it reports
straight off its own window rect, in pixels, with the divide-by-font-size
commented out.

One rounding the Mac did not have to do: 300 is not a whole number of lanthorn's
16-pixel Version 6 cells. A real Mac fitted 20 rows of its own 15-pixel Geneva
into exactly 300; lanthorn rounds to the nearest cell, 19 rows and 304 pixels, so
the screen *contains* the 480×300 plate with four pixels to spare. Rounding the
other way would have handed the game a 288-pixel screen and clipped the bottom
twelve pixels off its own artwork, which is the sort of thing that turns into a
missing pillar base.

Underneath, the monochrome archive is not a new codec at all, which was the
surprise. It runs the same Huffman + run-length + XOR as everything else on this
side of the format and lands one byte per pixel, using colour numbers 2 and 3 —
white and black — where a colour picture uses all sixteen. What it does differently
is its *directory*: 12-byte records rather than 14, because a two-colour screen
has no palette to point at. That is the same twelve bytes an EGA or CGA archive
spends for the same reason, so the PC and the Macintosh turn out to be one case
and not two.

The two sources are close but not identical, and where they disagree the original
media wins. Five *Zork Zero* pictures are cropped in the circulating Blorb —
ids 5, 6 and 7 keep only a 29–39 row band of what are full 320×200 decorative
frames, id 8 is flattened to a plain rectangle, and id 33 loses most of a
"Four Fantastic Flies of Famathria" plate. The floppy has all five whole. The
other 383 pictures decode byte-for-byte identically to the Blorb's, which is its
own quiet confirmation that those Blorbs were converted from the Amiga release.

*Shogun*'s floppy tells the same story from the other side of the format. Its
archive is built the second way the format allows — every picture carrying its
own compression table rather than sharing one for the whole file, which costs two
extra bytes in each directory entry. The header says which shape a file is, and
lanthorn reads both. Of the 39 pictures *Shogun*'s Blorb also holds, 34 come off
the floppy byte-for-byte identical; two of the rest differ only in how the Blorb
rounded the Amiga's 4-bit colours, and the others are places the Blorb kept a
band, or a retouched version, of art the floppy still has whole.

The PC releases shipped the *same* pictures in a different wrapper, and lanthorn
reads that too. `.MG1` (MCGA), `.EG1`/`.EG2` (EGA) and `.CG1` (CGA) are the same
sixteen-byte header and the same directory written the other way round —
little-endian, x86-style — but the pixels inside are GIF's LZW rather than
Infocom's Huffman, with no run-length pass and no XOR. One picture, two codecs
that share nothing: decode *Zork Zero*'s MCGA archive and its Amiga floppy side
by side and all 383 pictures whose directories agree on size come out
byte-for-byte identical, which is a nicer proof than any spec. Arthur and Journey
split their EGA art across two files — see [Two files, one
archive](#two-files-one-archive) below; CGA keeps its big pictures as one bit per pixel,
so *Arthur*, *Journey* and *Shogun* have 228 pictures between them that are
literally black-and-white. Only MCGA stores palettes — EGA and CGA had theirs
soldered in, so their directory records are two bytes shorter with nowhere to put
one.

#### Two files, one archive

EGA art is 640 pixels wide and did not fit on a 360K floppy, so *Arthur* and
*Journey* shipped their EGA renditions on two disks — `.EG1` and `.EG2` — and the
header's first byte says which one you are holding. **Name the first and lanthorn
loads both.** You do not have to know the set is split, and you cannot pick half
of it by accident: the launch dialog and the info panel show a two-disk set as a
single row, counting the whole thing.

That matters more than it sounds. The split is not a partition — each disk had to
stand alone for its stretch of the game, so a picture wanted on both sides is
stored on *both*, and 55 of *Arthur*'s ids live in two places. Read only disk one
and you get 97 of *Arthur*'s 137 pictures and 80 of *Journey*'s 135; the rest are
simply absent, including two of *Arthur*'s largest plates. Merged, the two disks
come to exactly what `arthur.mg1` holds undivided — 171 entries, 137 of them with
pixels — which is the nicest possible check that nothing is being lost or
invented. (*Journey*'s EGA set carries one picture MCGA does not: id 59, a
220×126 rectangle of solid black and the only single-colour plate in the archive,
which looks very much like an EGA-only way of blanking the illustration window.)

Following the part number to the next file is *not* the guess-the-pairing-from-a-
filename rule the tier list below rejects, and the difference is worth being
precise about: you have already told lanthorn which archive this story uses, and
this only follows that archive's own in-band part number to the rest of itself.
The header is then checked — a file under the next part's name that says it is
some other part, or was written by another codec, or adds no picture the set
lacks, is **refused and reported**, never merged on the strength of its name.
lanthorn keeps looking until a part is missing, so a title that shipped on three
disks would work too.

*Zork Zero* is unaffected: its 360K release gave EGA a whole disk, so `zork0.eg1`
is complete on its own and stays at 396 pictures.

#### Apple II artwork

The Apple II press is the fourth machine, and the one that hides its artwork
best. There is no archive *file* on any of these disks: the pictures live inside
the same opaque `.D1`…`.D5` segments as the story, and the segment index says
where — one block number per segment, zero for a segment carrying no art. Open
any of these and lanthorn merges what it finds into one archive:

| release | opens from | pictures |
|---|---|---|
| *Arthur* r63 | `Arthur Quest 4 Excalibur.2mg`, `Arthur.po` | 168 |
| *Journey* r77 | `Journey.po`, `journey_s1.dsk`…`s5` | 135 |
| *Shogun* r311 | `shogun_s1.dsk`…`s5` | 55 |
| *Zork Zero* r383 | `zork_zero_1.dsk`…`_4` | 496 |

Three of those four are pressed on 5.25" floppies with the artwork spread across
the whole set, so naming any one volume merges the plates off all of them —
exactly the way naming any one volume opens the whole game. A set with a floppy
missing draws nothing rather than a picture space with rooms knocked out of it,
which is why `Journey.2mg` — an 800K image that genuinely shipped without its
fifth segment — is the one Apple release in the corpus that still plays no art
and, for the same absence, does not play at all.

**And this is a different screen.** The Apple's picture space is 140×192, stated
by Infocom's own Apple interpreter in the dots it is counted in: `MAXWIDTH EQU
140 ; 560 / 4 = max "pixels"` and `MAXHEIGHT EQU 192 ; 192 screen lines`. One
Apple picture pixel is four dots of the 560-dot double-hi-res display and one
scan line tall, and on the 4:3 monitor the machine drove a scan line measures
about 2.19 dots — so lanthorn presents that art at (4, 2) and the story is told
its screen is **560×384**. Not 640×400: that was what *Arthur* got while its
artwork was unreadable and nothing declared a picture space, and an archive
outranks a machine default for the same reason a Macintosh mono `Pic.data` lays
*Zork Zero* out on 480×300. The Apple press of *Arthur* is a third build beside
the Amiga's r54 and the DOS r74, and it is entitled to its own screen.

| rendition | picture space | drawn at | the screen the story is told about |
|---|---|---|---|
| Amiga / Mac colour, MCGA | 320×200 | 2× | 640×400 |
| EGA, CGA | 640×200 | 1× / 2× | 640×400 |
| Macintosh mono | 480×300 | 1:1 | 480×300 |
| Apple II | 140×192 | 4× / 2× | 560×384 |

560×384 is exactly 70×24 of lanthorn's 8×16 Version 6 cells, so nothing rounds.
(The Apple's *own* character grid was 46×21 on a 3×9 cell, which is a screen
model rather than a resolution and is a separate thing lanthorn does not yet
express — see [`interpreter.md`](interpreter.md).)

### Choosing which artwork a game draws

Three sources, in decreasing order of how sure lanthorn can be that the art and
the story belong together:

1. **A Blorb.** The container validates its own contents. Nothing to configure.
2. **A disk image — the whole release, not just the platter.** Story and archive
   came out of one box, so the medium guarantees the pairing. Nothing to
   configure. A multi-disk press can split them: the 360K DOS *Zork Zero* puts
   CGA on disk 1, the story **alone** on disk 2 and EGA on disk 3, so booting the
   story disk used to draw no artwork at all and offer none to pick. lanthorn now
   reads every volume of the release, and prefers the rendition that kept its
   colour when the story's own disk carries none — EGA over CGA, on a terminal
   with rather more than two.

   It only does that when the release is *one game*. A twenty-game shelf like the
   DOS `floppy1.ima`…`floppy5.ima` press of *The Lost Treasures of Infocom* keeps
   each disk's artwork on that disk, because "in the same box" stops being
   evidence the moment the box holds Zork I as well — and *Zork Zero*'s plates
   drawn into *Zork I* would look like art rather than like a mistake.
3. **You say so.** Put a `pictures` line in the game's own `config.toml` — the
   small per-game sidecar in `<save-dir>/<story>.save/`, alongside the per-game
   `style.toml`:

   ```toml
   pictures = "zork0.mg1"
   ```

   The name is relative to the story file (an absolute path works too), and a
   named archive **wins outright** — over a Blorb sitting right beside the story,
   over a floppy's own `Pic.data`, over everything. Naming it is an instruction,
   not a hint. If the name is a bare filename that is not on your filesystem and
   the story was mounted out of a disk image, it is looked for *on the release* —
   the story's own disk first, then its siblings — which is how you reach the
   Macintosh's monochrome `Pic.data`, either archive on an Amiga floppy, or a PC
   disk's `ZORK0.EG1` while you are booted off the disk *next* to it, without
   unpacking anything first.

#### A Blorb that belongs to another release is not used

Tier 1's confidence has a limit, and the Apple IIgs *Arthur* found it. That disk
is release 63 / serial 890622 and carries 168 pictures; lanthorn cannot yet read
its artwork off the platter, so it fell through to the resource Blorb beside the
story — and `Arthur.blb` is the **DOS** press, release 74 / serial 890714, with
**326**. The two games number their pictures differently, so *Arthur* asked for
its own plates and got another build's. That is the corruption, and it came from
a six-character filename match.

Blorb has a chunk for exactly this: the optional `IFhd` **game identifier**, which
records the release, serial and checksum the resources were made for, and which
the spec says an interpreter *"can check… If they don't [match], the interpreter
should display an error."* lanthorn now checks it, and where the check fails it
draws nothing and says why:

```
lanthorn: warning: Arthur.blb is the artwork for release 74, serial 890714, but
this disk is release 63, serial 890622 — a different build's pictures are not
being drawn
```

The launch dialog agrees, because it asks the same question the boot does: its
first row reads *Automatic — no artwork found*. *Arthur* is perfectly playable as
text meanwhile, and that is better than nonsense.

The rule is deliberately narrow, because most of the corpus pairs a story with a
neighbouring file quite legitimately and all of it must keep working. A Blorb is
only refused when it **contradicts** the story: it carries an `IFhd`, the story
came off a disk image, and the identifier matches no build on that release. Three
things are therefore *not* contradictions —

- **A Blorb that says nothing.** `IFhd` is optional and most containers omit it —
  every modern `.zblorb`, `Sherlock.blb`, `beyondzork.blb`, all eleven *Mysterious
  Adventures* sidecars. Silence is not disagreement.
- **A loose story file.** Its folder was assembled by a person, and that placement
  *is* the pairing. *Frobozz Magic Video Poker* is the case that proves it: its
  Blorb is a byte-for-byte copy of *Zork Zero*'s, so it claims to be release 393
  while the game is release 60 — and its own readme tells you to do that, because
  borrowing *Zork Zero*'s plates is the entire design of the game. lanthorn does
  not overrule someone who has already answered the question.
- **A disk whose story lanthorn cannot identify.** Nothing to compare, so nothing
  is proven. The check simply gets sharper as the disk readers learn more, and it
  has: see below.

#### Asking the release, not the platter

The Apple II presses of the graphical Version 6 games do not put a story *file*
anywhere. They page one game across the whole set as opaque `.D1`…`.D5` segments,
so the five-volume `shogun_s*.dsk` press is five floppies of which not one carries
a story — and the identity check, which asked each volume on its own, came back
empty however plainly the release stated its build. It kept `Shogun.blb`, release
322 / serial 890706, and drew its 48 pictures into a game that is release **311**,
serial 890510.

The check now asks the release as a whole. The segment index names which block
holds story page 0, and that page is where the release, serial and checksum live
(Quetzal §5.4) — so *which build is this?* is a question **one 512-byte block**
answers, without reassembling the 344 KB story around it. On the presses where
lanthorn can do both, the page and the full checksum-verified reassembly name the
identical build, which is what licenses trusting the page on its own.

Two releases changed, and nothing else in a corpus of 269 files did:

| release | its build | the Blorb beside it | now |
| --- | --- | --- | --- |
| `shogun_s1.dsk` … `s5.dsk` | 311 / 890510 | `Shogun.blb`, 322 / 890706 | refused |
| `Journey.2mg` | 77 / 890616 | `Journey.blb`, 83 / 890706 | refused |

*Journey*'s image is the interesting one, because it is an **incomplete pressing**:
its index declares five segments and the disk carries four, so 92 of the story's
552 pages are missing and the game cannot be reassembled or played off that image
at all. It can still say what it is — page 0 survives on `JOURNEY.D1` — and a
release that cannot be played is still a release whose plates lanthorn will not
guess at. The four-volume Apple II *Zork Zero* press became identifiable in the
same breath (release 383 / serial 890602) and nothing about it moved, because no
Blorb in the corpus stem-matches it.

Refusing the wrong plates left these releases drawing nothing at all, and that
was always meant to be temporary: their own plates *are* on their platters,
inside the same packed segments. lanthorn reads them now — see [Apple II
artwork](#apple-ii-artwork) below — so *Shogun*'s press draws its own 55, the
`.2mg` *Arthur* its own 168, and the refusal costs a player nothing. The one
release still dark is `Journey.2mg`, and for the same reason it cannot be played:
the segment it is missing carries a quarter of its pictures.

One more thing moved with it: `IFhd` describes the **container**, not its picture
chunks, and a Blorb built for another build numbers its sounds every bit as
build-specifically as its pictures. So the boot resolves the sound container
through the same check. On today's corpus that changes nothing audible — no Blorb
the rule refuses holds a single `Snd` resource — but it does end a boot that
refused a release's artwork out loud and then reported, one line later, that it had
loaded 48 images from it. The one genuine sound-path mismatch, `Lurking.blb` at
release 221 / serial 870918 against a release 219 / serial 870912 story, is
untouched for the reason *Frobozz Magic Video Poker* is: that story is a loose
file, and somebody put those two in a folder together on purpose.

And note that a contradiction only ever *matters* for a disk carrying no artwork
of its own. The Amiga *Arthur* and *Journey* floppies are contradicted by the same
DOS Blorbs sitting beside them, and both draw their own `Pic.data` regardless,
because the medium is consulted first and never reaches this question.

Naming an archive yourself (tier 3, below) still wins outright, mismatch or not.
That is the point of naming one.

Tier 3 is how you *pick a rendition*, not just how you rescue a game. *Zork Zero*
alone can be played four ways from the files that survive — the Amiga `zork0.pic`,
the MCGA `zork0.mg1`, the EGA `zork0.eg1`, the CGA `zork0.cg1` — and they are
genuinely different pictures, not the same art at different sizes. Point the key
at whichever one you want and restart. *Arthur*, *Journey* and *Shogun* offer the
same choice.

#### Three ways to say it

The config key is the *durable* form, and editing a file before you can see the
result is a strange way to choose something you can only judge by looking at it.
So there are two more doors into the same mechanism, and all three end in the
same place:

| You are… | Say it with |
| --- | --- |
| launching one story from a shell | `--pictures <path>` |
| browsing the library | **Shift-Enter** (or `o`, or a double right-click) on the story |
| setting it once and forgetting it | `pictures = "…"` in the game's `config.toml` |

`--pictures` is the try-it-once path — `lanthorn zork0.z6 --pictures zork0.mg1` —
and it composes with your shell, so you can flip between renditions in successive
launches without touching any config. It **outranks** the config key: the more
specific and more recent instruction wins. It also *requires* a story on the
command line, and says so immediately rather than starting the picker and
quietly discarding the flag — the flag names art for a story, so it has no
meaning without one.

**The launch-options dialog** is the richest door, because it can show you what
you have before you choose. Select a story in the picker and press **Shift-Enter**
— plain Enter launches as it always has, so you meet this only when you ask for
it. (`o` does the same thing on terminals that can't tell Shift-Enter from Enter,
and so does double-right-clicking a row.) It lists the archives detected **for
that story** — flavour, picture count, and a "2 disks" note on a split set — and it shows you the
interpreter number your choices imply *and where that number came from*, because
picking prettier art can quietly change the machine you're emulating and that is
not a thing to discover later.

"Detected for that story" means two things, because a story's art can live in two
places.

**Beside it**, the name has to match — in either direction, once both names are
reduced to their letters and digits. That is enough to connect `zork0.mg1` to
`zork0-r393-s890714.z6`, `beyondzo.mg1` to *Beyond Zork* under either of its
filenames, and `shogun.*` to a floppy called *James Clavell's Shogun* — across
every game in a real library it finds each one's art and nobody else's, so *Zork
Zero*'s dialog offers four renditions rather than a folder. An archive under a
name that resembles nothing simply isn't in the list; you reach it the way you
always could, by naming it — `--pictures`, or the `pictures` key — and the dialog
says so on its last line rather than leaving you to wonder.

**Inside it**, when you launched a disk image, no name test applies at all: every
archive on the release is offered, because the medium itself is the pairing.
There is nothing to guess — the story and the art came out of one box. On a
single-game multi-disk press that means the siblings too, which is what puts CGA
and EGA in front of you when you boot the 360K *Zork Zero* story disk that
carries neither. Each of those rows says **which** disk it came off — *from disk
3* for EGA on that press, *from game disk* when the release is a single platter
and there is no number to give. That matters precisely because of the siblings:
once an archive can live on a disk you never put in the drive, "on disk" stops
being vague and starts being misleading. The number is the release's own, read
off the set the filenames form, not counted from a position in a list. This is what makes the Macintosh's two archives pickable: a directory
scan cannot see inside a disk image, so before this the dialog could offer a Mac
disk nothing at all, and its black-and-white artwork could only be reached by
typing `--pictures Pic.data`. Every archive is identified by *parsing* it rather
than by its name, which matters here more than anywhere: `CPic.data` and
`Pic.data` are one codec under two names that tell you nothing, and only the
file's own two-colour flag says which is which.

The same list appears, read-only, in the picker's **info panel**, so you can see
what a game has without opening anything: each detected archive with its flavour
and picture count, and an arrow against the one the game's `config.toml` actually
names. Panel and dialog run the same detection, so they cannot tell you two
different stories about what you own. If your `pictures` key names something the
detector would never have found — that renamed `FMVPOKER.EG1` — the panel names
it anyway, because it is what the game will draw.

Everything in that dialog applies to **this launch only** — until you tick *Save
as this game's default*, which writes it to the game's `config.toml`. That is the
point of the checkbox: try the EGA art, look at it, and only write it down if you
keep it. It writes only what you actually changed, so a setting you left alone
stays inherited rather than being pinned at today's value.

Two options are in there and no others, and the rule is not arbitrary: **only
choices that cannot be changed after boot**. The artwork is opened as the story
starts; the interpreter number is read out of the story header by the *game*.
Anything the running app can already change — colours, the v6 render mode,
map behaviour — belongs in the settings screen, and putting it in both places
would give you two editors for one value.

It also rescues games nothing could pair automatically. `fmvpoker.z6`, a fan-made
video-poker cabinet, ships a readme telling you to rename one of *Zork Zero*'s
graphics files to `FMVPOKER.EG1` and drop it alongside. No rule could ever have
guessed that; one line of config says it outright.

What lanthorn deliberately will **not** do is find an archive by name. It sounds
harmless and it is not. These files carry no release number and no serial —
nothing that ties one to a story — and every Infocom Amiga release names its
archive the identical `Pic.data`, so a name-based rule needs you to rename things
anyway. Get it wrong and there is no error: *Arthur*'s illuminated plates simply
appear in *Zork Zero*, looking exactly like artwork. Better to be asked than to
guess wrong silently. (Listing what it *finds*, so you can pick, is a different
and perfectly safe thing — that just hands the choice back to the person who
knows which game they own. Which is exactly why the name matching described
above is allowed to exist: it decides which rows you are *shown*, never which
file gets opened. Nothing downstream of that list acts on it.)

And when the key names a file lanthorn can't use — missing, truncated, or not a
picture archive at all — it says so, out loud, naming the file and the reason,
before falling back to the Blorb. The one outcome worth ruling out is a player
who believes they're looking at original artwork and isn't.

Naming an archive also picks the machine. Ask for a game's EGA rendition and you
are asking for the IBM PC that drew it; ask for its `Pic.data` and you are asking
for the Amiga, colours and all — see
[the interpreter profile](interpreter.md#the-interpreter-profile). lanthorn works
out which from the file's *contents*, never its extension, because the two codecs
are structurally different and a filename can lie. An explicit
`interpreter_number` still overrules it.

With one honest caveat: the Amiga and the Macintosh wrote the *same* container,
so an archive of that flavour names a codec and not a machine. When the story
came off a disk, the disk settles it — pick either archive on a Macintosh volume
and you are still on a Macintosh, which is what the dialog's provenance line says
while you are choosing. A rendition that *is* unambiguous still wins outright: an
`.mg1` asks for the IBM PC and gets it, whatever it is sitting on.

Native archives carry no `Reso` chunk — the format has no such concept — so the
archive states the picture space its own coordinates use, and the screen is that
space at the scale the machine drew it in. For every rendition but one that
works out to the same 320×200-doubled 640×400, which is precisely what every
Infocom v6 Blorb's `Reso` declares anyway, so the geometry below is unchanged.
The exception is the standard Macintosh, [above](#two-macintosh-screens).

What *does* differ between renditions is how densely the art is stored. EGA and
CGA addressed a 640-column screen with pixels half as wide, so their plates are
640 across where MCGA's are 320 — the same picture, twice the samples, each one
half the width. Both cover the same rectangle, so both land on the same 640×400
screen: an MCGA or Amiga plate doubles on both axes, an EGA or CGA plate doubles
only vertically, and the Macintosh's 480×300 monochrome plate doubles on neither
— which is why it is a different screen rather than a denser drawing of this one.
*Arthur* is the clean proof — all 125 pictures its `.mg1` and
`.eg1` share come out at byte-identical sizes once each is mapped that way, and
*Zork Zero* agrees on 446 of its 503 (the rest differ by a pixel or two, because
these are separately drawn renditions rather than one scaled copy). Frotz reads
the same header bit as `x_scale = (flags & 0x08) ? 640 : 320`; Spatterlight's
bocfel calls it `pixelwidth` and sets it to 0.5.

The character grid never moves between *cards*. EGA ran 640×200 on an 8×8 cell,
which is 80×25 characters — the very grid the 640×400 screen already lays out on
its 8×16 cell — so choosing a rendition changes the artwork you are looking at
and nothing about the machine underneath it. Choosing a *machine's other screen*
does move it, and only the Macintosh has one: 480×300 on the same 8×16 cell is
60×19 characters, a genuinely smaller grid, because that really was a smaller
screen.

### The colours come with the card

An MCGA or Amiga picture arrives with its own sixteen colours attached. An EGA or
CGA one does not, and there is nowhere in its directory record to put them —
those cards had their palettes soldered in, so Infocom stored the pixels and let
the hardware supply the rest. lanthorn now supplies it too, reading the rendition
straight out of the directory: nobody carrying a palette means the colours came
from somewhere else, and every picture flagged two-colour means two colours.
(Never the file extension. A `.CG1` that somebody renamed is still a `.CG1`.)
The Macintosh's monochrome archive answers that second question exactly as a
`.CG1` does — bocfel handles Mac black-and-white and CGA in a single branch — but
it does not get the same two colours, because the machines disagree about the lit
one: the card's is light grey and the Mac's is a real white. The container is what
tells them apart, since the flag cannot.

**EGA** gets the card's sixteen: each channel off, a third, two thirds or full —
0, 85, 170, 255 — with one famous exception. Colour 6 should arithmetically be a
dark yellow, and the hardware halves its green and shows **brown** instead,
`#AA5500`, because IBM thought brown more useful than mustard and wired in the
extra circuitry to get it. That single entry is not a footnote: *Zork Zero*'s
proscenium arch is drawn as brown dithered against bright red, and getting it
wrong turned the whole frame pink and olive.

And getting it *right* is only half the arch, because EGA has no bronze at all —
the artist made one. Look closely at the original and the arch is not brown, and
not red: it is brown and bright red in alternating **columns**, one pixel wide,
and on a 640×200 screen those pixels are half as wide as an MCGA one, so the card
fused each pair into a colour the palette does not contain. Bocfel puts it
perfectly: no single pixel of the artwork is the colour the eye actually sees.
lanthorn keeps all 640 columns — that is what makes an EGA plate cover exactly
the rectangle a 320-wide one does — so it has to do the fusing itself, with a
three-tap tent across columns as the art comes out of the archive. Do it there
and bronze is a property of the artwork; leave it to the scale onto your terminal
and it becomes a property of *your terminal*, since that scale is
nearest-neighbour on purpose and blends at no width at all. Measured on *Zork
Zero*'s border, the fused EGA frame's neighbour-to-neighbour variation falls from
49.1 to 8.4, against the MCGA rendition's own 4.3, and it now reads the same at a
pane of 320 pixels or 1280.

**CGA is deliberately left alone**, and it is the reason the rule is written the
way it is. A `.CG1` is 640 wide exactly as an `.EG1` is, so a rule keyed on width
would soften it too — and there is nothing in it to fuse. Its 640-wide art is
genuine one-bit line work, and blending line work only makes it grey. What the
fusing asks is not "how wide?" but "how many colours?", off the archive's own
two-colour flags.

**And if you would rather see the pixels Infocom shipped**, set
`fuse_art_dither = false` and every column comes back distinct, dither and all.
The default is on, because on is what the card did to the eye — but the archive's
own bytes are a perfectly reasonable thing to want to look at, and this is the
only setting that changes them. It cannot make CGA blend; that answer belongs to
the artwork, not to you.

#### Where the fusing stops, and why it stays there

The tent is a *notch*, not a blur. It zeroes an alternation of exactly two
columns — which is what the arch is, and why the frame's flat interior comes out
at a neighbour-to-neighbour variation of 0.00 — and it barely touches anything
coarser. *Zork Zero*'s **pillars** are dithered the other way: not a clean
two-colour alternation but error diffusion over seven EGA entries in irregular
runs, the sort of thing an automatic colour reducer produces on a smooth bronze
gradient. Broadband noise has energy at every frequency, so the notch removes
only the top of it. Across the flank columns the fusing takes the pillars from
62.9 to 12.7, against 12.3 for the MCGA pillars measured in their own 320-wide
space — much better, and still visibly a weave where MCGA is smooth metal.

Widening the kernel does finish the job: `[1, 2, 2, 2, 1] / 8` has zeros at both
of the frequencies a 320-wide plate cannot carry, and it takes the flank to 6.98
against MCGA's 6.05 while pulling the whole frame's distance to the MCGA
rendition from 27.79 down to 26.04. Every number improves. It is still not what
lanthorn does, because the same frame carries the **compass rose**, whose N, W, E
and S are 640-wide line art the card resolved perfectly well — and at that width
they stop being letters and become smudges. One plate, two kinds of detail at the
same frequency, and no single linear filter tells them apart. The tent keeps the
lettering; the pillars keep some of their weave. That is the trade, made
deliberately.

**CGA** gets two colours, and that surprises people who remember CGA's cyan and
magenta. Those belong to its 320-wide four-colour mode; the 640-wide mode these
archives are stored for — mode 6, the only 640-wide one the card had — is one bit
per pixel. So *Zork Zero*'s CGA rendition really is crisp two-colour line art,
exactly as it was in 1988, and not a washed-out version of the EGA one. The two
colours are black and the card's **light grey**, `#AAAAAA` — not pure white:
`machine-screenshots/dos-zorkzero-cga.png` measures the same shade for the
artwork and the text, and it is EGA entry 7, the value the IBM PC's row has always
recorded. A Macintosh's monochrome plate keeps a real white, and it gets its own
table for exactly that reason.

Two colours also make it a **stencil**, which is the part worth knowing. Count
the border: 46,336 pixels of opaque paint, 17,152 of opaque black — and 192,512
transparent. The paint is the lit face of the pillars; the transparency is
deliberate, and whatever sits behind it becomes a colour the artwork never had to
store. Both are lost the moment something paints a page underneath, and *Zork
Zero* asks for one — it sets black-on-white at boot and does so for every video
card alike, because the story file cannot see which archive you loaded.

### A two-colour card takes one bit

**A display with two states takes one bit from a pair of colours, and that bit is
which channel wants the lit one.** So lanthorn does not turn a game's colours off
when it draws a `.CG1`; it hands the pair to the card, and the card shows its own
two. *Zork Zero* asks for black ink on a white page, the card shows light ink on
a black one — `machine-screenshots/dos-zorkzero-cga.png`, the Banquet Hall in CGA
mode, is 48.3% pure black, the exact inverse of the page the story asked for — and
the stencil reads against it the way it was drawn to.

Leaving the game's colours ON is the point rather than an implementation detail:
*Zork Zero*'s own in-game **`color`** command works through them, and on a CGA
machine it offers exactly what the card has, a swap of the two states. Take the
colours away and the command has nothing to act through; the game checks for them
before it does any colour work at all, which is also why declining produced the
right-looking screen for the wrong reason. Choose the swap and the plates wash out
— light line work on a light ground — and they do that on the real machine too.
It is your choice to make, and lanthorn now lets the game offer it.

**A machine whose own screen is that display needs no card.** A Macintosh's is:
the volume names the machine, Infocom's own Mac interpreter names its white page
under black ink, and the *same* interpreter picked the monochrome `Pic.data`
**for** that page, in one decision. Its plate is not a video card and never
collapses — which is what it took to stop the status banner's location and score
coming out grey on the game's own white plate.

**And with no machine at all, there is still nothing to state.** Open a bare
`.z6` with `--pictures zork0.cg1` and no medium has named a machine, so no card
has named a screen; lanthorn tells that story the interpreter has no colours, your
theme owns the page, and the stencil reveals it. That applies to that story only —
it never touches your saved settings, so opening a `.cg1` once does not quietly
strip the colours from everything else you play.

Neither is *adaptive*, which matters more than it sounds. A picture that carries
no palette normally means "draw me with whatever palette is current" (below), and
an EGA picture carries none for an entirely different reason — it has no say in
its colours at all. lanthorn keeps those out of the Current-Palette machinery
altogether, so nothing can tint a rendition whose colours were decided by a chip.

## Splitting the screen TILES it

A v6 game reserves room for artwork by splitting the screen, and the standard is
precise about what that means (§8.8.4.1): the opcode "tiles windows 0 and 1
together to fill the screen, so that window 1 has the given height and is placed
at the top left, while window 0 is placed just below it (with its height suitably
shortened, possibly making it disappear altogether if window 1 occupies the whole
screen)". The split *places* the story window; it does not merely shrink it.

That matters because most games never move the story window themselves — they
don't have to. `mysterious01.z6` splits off 260 pixels, draws its illustration up
there, and starts narrating; if the story window is left in the top-left corner it
sits inside the picture and the prose prints across the artwork. And Adventure's
Inform 6 library goes further: it splits, asks the interpreter where the split left
window 0, and positions its own prose window at the answer. A game reads the
tiling back, so getting it wrong misplaces everything downstream of it — bar, room
description, and menus alike.

The spec's own escape hatch is worth naming: a split that takes the entire screen
leaves the story window with zero height, which is exactly what Zork Zero's
full-screen title splash relies on. Nothing is carved over the picture, and the
game re-places the window itself when the splash goes.

Some games never lay out at all. Inform 6's v6 library leaves *every* window at
height zero and flows its prose through the transcript, so the screen model would
otherwise come out completely empty and the composite would ship a blank page. For
that — and only that — lanthorn synthesises a full-screen story window out of the
header's own character dimensions, so the streamed text still has somewhere to
live. The question it asks is "did *nothing at all* survive?", which sounds
obvious and was not: it used to ask whether the surviving windows had a zero
*character grid*, and a game that never resizes window 0 off its boot rect never
sets one. `sunburst.z6` is that game — a real 640×400 story window with a 0×0
char grid — so it got a phantom twin at the same rect, filed away as frame
furniture. One screen, one story window.

## A window keeps its own text style

Bold, italic and reverse video are *per window* in Version 6. The standard lists
the style as window property 10 (§8.8.3.2) and says it "is set just as in Version
4, using `set_text_style` (which sets that for the current window)" — so selecting
a window makes that window's style live, exactly as it makes that window's colour
pair live. A game can leave the status bar reversed indefinitely and go on printing
plain prose below it, and on a conforming interpreter it never has to say so.

Shogun does precisely that, but only when it thinks it is on an Amiga: it selects
window 1, turns reverse video on, paints the status line, and returns to window 0
without turning it off. Reading the style as one global setting therefore left the
Amiga release printing everything in inverse from its second turn onwards — the `>`
prompt, the room headings, the death notice. It is the kind of bug that only one
build shows, so it is worth saying which: `James Clavell's Shogun.adf`, release 295
/ serial 890321, which is a different build from the `shogun-r322-s890706.z6`
sitting beside it and the only title in the corpus the fix moves at all.

## The authentic screen: 640×400, an 8×16 cell, art doubled

There's a subtlety in "how big is this thing" that decides whether the whole
frame looks right. Infocom's v6 artwork is 320×200 MCGA, but the games were
authored and tested against the Amiga/DOS interpreter, which presents them on a
**640×400** screen with a **non-square 8×16 pixel font cell** — 80 columns × 25
rows of text — and scales every picture **2×** on the way to the screen. That
2× is the whole trick: 80 columns spread across 640px of doubled art make the
text read at its period-screenshot size *relative to the picture*, instead of
the oversized 40-columns-over-320px look you get if you take the art dimensions
at face value.

lanthorn now does exactly that (matching Frotz's DOS/Amiga profile). The engine
reports a 640×400 screen (2× the Blorb `Reso` standard window, or a plain
640×400 when a story ships no `Reso`), an 8-wide-by-16-tall font cell, and
answers `picture_data` with the **doubled** dimensions — so the game lays its
banner, columns, and compass out on the same 640-wide grid the original did.
The 320×200 pictures themselves stay art-native in storage and are blitted 2×
(crisp nearest-neighbour, DOS-authentic) into the composite; the bitmap text is
drawn with a natively 8×16 face — Uni-VGA, the IBM PC text font the profile's own
blue and white belong to — so it fills the cell 1:1 with no resampling at all.
Until SQ-0932 it was an 8×8 master doubled vertically, which spent half the cell's
height on duplicated scanlines and gave no glyph a descender. **Font 3 still is**
that doubled 8×8, deliberately: box drawing and block elements are a graphics
character set rather than a typeface, and Uni-VGA's `│` is CP437's two-pixel
vertical where every v6 rule in lanthorn is a one-pixel hairline.
Screen size and picture size double *together*, so the frame-vs-content picture
classification (which is pure ratios) lands exactly where it did before.

The doubling follows the **`Reso` chunk**, though, not the version number. Blorb
§11 is explicit that a resource file without one has no scalable images at all —
"non-scalable images are always displayed at their actual size. (One image pixel
per screen pixel.)" Every Infocom v6 blorb declares a 320×200 standard window, so
they all double. scopa.blb declares nothing: its card art is drawn for the 640×400
screen already, the same 52×84 as the vector deck hardwired into its own z-code.
Doubling *that* told the game its cards were 104×168, and it dutifully laid out a
menu whose sample cards overlapped each other and hung off the bottom of the
screen. So the screen is still 640×400 either way, and the art scales only when the
story says what it should scale against.

And that screen is a **hard edge**. A v6 game may size a window far past it,
because `window_size` doubles as a measuring instrument: scopa opens a scratch
window 1000×1000 so a string it is about to print cannot wrap, reads the width
back, and moves on. Taken literally that one window is bigger than the screen,
and since the composite spans every window the game has open, the whole picture
would shrink to fit it — the table crammed into a corner with black bands where
the oversized window's page fell off the world. lanthorn draws the part of a
window that exists: each box is clipped to the screen the header declares
(§8.4.3's width and height words) before anything is composited. The clip is
purely what gets *drawn* — the interpreter still reports the size the game wrote
when the game asks for it back, which is the whole point of the trick scopa is
pulling. `/dump-windows` shows both: the size the game set, and what of it is on
screen.

## Art grows with a hard filter and shrinks with a soft one

Everything above lands the artwork on a 640×400 game screen. Getting *that* onto
your terminal is one more scale, and which way it goes changes what the right
answer is.

Growing is easy and it is the case pixel art is famous for: nearest-neighbour,
which replicates whole source pixels and invents no colours. Journey's canyon
plate is 222×254 native pixels drawn from a palette of fourteen; magnified 1.48×
it still holds exactly fourteen. Run the same plate through a smoothing filter
and you get 1,636 — every one of them a blend nobody painted. That is why the
scale caps out (`MAX_V6_UPSCALE`) rather than reaching for your pane's full
device resolution, and why it has always been nearest.

That cap is a *PNG-encode budget*, though, and so it only binds a backend that
spends one. Kitty, sixel and iTerm2 build and ship encoded pixels for every frame
the picture changes on, and every extra factor of magnification is bytes to make
and bytes to write. **Half-blocks encodes nothing** — the image is resolved
straight into terminal cells, one pixel per column and two per row — so it has no
budget to protect and no ceiling any more. It used to have one, and the cost was
plain: because the fit that follows only ever *shrinks*, the magnification is what
decides how many cells the picture occupies, so a picture pinned at 2× kept a fixed
number of cells while a smaller font kept handing the pane more of them. Shrinking
the font made the game window smaller instead of the picture sharper. It now climbs
as far as the pane allows — and where the v6 pixel lock is on, as far up the
artwork'''s own integer ladder as the pane allows.

Lifting that ceiling then made something else visible, and it is worth spelling out
because it reads like a contradiction. The pre-scale exists to work around an API:
`Resize::Fit` never grows, so a pane bigger than the composite gets nothing at all
unless the picture is magnified *before* it is handed over. But half-blocks does not
want device pixels — it resolves whatever it is given to exactly one sample per
column and two per row, and throws the font size away — so on a 458×144 pane the
pre-scale was magnifying a 640×400 canvas to 4580×2862 (50 MB of nearest-neighbour,
155 ms) purely so the crate could take it straight back down to 458×288. Two
resamples in opposite directions, to land on a grid *narrower* than the artwork
started. Half-blocks therefore skips the pre-scale entirely now and resamples once,
straight onto the sample grid: 0.50 MB and 2.3 ms for the same frame, the same cell
rect on screen, and one filter chosen by direction rather than nearest-up followed
by a smoothing pass down. The encoding backends keep the pre-scale, because for them
it was never a workaround — the pixels they build are the pixels they ship.

That argument was never about v6, and the same pair was still standing at the four
places a picture is *fitted into a cell box*: Glulx graphics windows, the cover panel
in the story picker, its cover-grid tiles, and the resource preview. All
four now make one call, and on half-blocks it resamples once onto the sample grid too.
The win here is smaller and honestly so — a jacket scan into a twenty-cell tile
pre-scaled to 190×220 device pixels, not to 50 MB — but it is paid per *tile* and on
every scroll: a screenful of thirty-six covers rebuilt in 43 ms instead of 76, for 132
MB of allocation instead of 211. The Glulx window is where it is dramatic, because
that one *magnifies*: a 320×200 canvas filling a 100×40-cell window went up to
1000×640 so the backend could take it down to 100×64, and skipping that is 23× faster
for a tenth of the bytes. A picture already at native size in its window is a wash —
marginally slower, in fact, and what the extra microseconds buy is a cut-out edge that
no longer averages toward black. Every one of those sites lands on the same cells it
did before; nothing on screen moves, it just gets there once.

Shrinking is the same rule read backwards, and that is the trap. The instruction
"take the source pixel nearest this destination pixel" *replicates* one on the way
up and **drops** one on the way down. At a 60×24 pane Journey's plate is asked for
168×198, and 54 of its 222 columns and 56 of its 254 rows are then never sampled
at all. On a flat wall you would not notice. On a dithered one — a checkerboard of
two inks standing in for a third — every surviving pixel is a coin toss about which
ink you keep, and the shadow that should read as a smooth gradient breaks into
noise. Which is precisely the report this section exists because of: distortion in
the artwork *only when the artwork is smaller*, worst in the foreground rocks and
the dithered shadow.

So the filter is now chosen by direction, per axis, at every one of the places art
is resampled: the raster composite, the hybrid ring's bands, and the two pictures
that are fitted to a box of their own rather than to the frame's letterbox grid — a
menu flank's panel and a divider extended down a reclaimed gap. An axis that grows
gets nearest; an axis that shrinks gets an area filter
whose kernel is as wide as the ratio, so the dither *fuses* into the colour it was
always standing in for. The two axes are decided separately because a band can grow
on one while it shrinks on the other — that is exactly what an elongated frame
column is — and a pass at 1:1 is a bit-exact identity, so the ordinary case still
costs a single resize.

Measured against the honest ideal (an area average where an axis shrinks,
replication where it grows), on Journey's plate at the sizes the pane sweep
actually produces:

| filter | RMS on a shrink | what it does to a dithered gradient |
|---|---:|---|
| Nearest | 9.9–10.7 | drops rows and columns; the reported aliasing |
| **Triangle** | **0.4–1.6** | fuses the dither — the area filter |
| CatmullRom | 2.1–2.6 | over-sharpens; raises contrast *above* the ideal |
| Lanczos3 | 3.8–4.1 | over-sharpens harder, and rings |
| Gaussian | 2.4–3.5 | over-blurs |

There is a second, quieter fix folded in. The raster composite's own pre-scale was
clamped at 1.0, so a pane smaller than the composite made a full identity copy of
it that bought nothing at all, and then left the actual shrink to the image
protocol's *default* filter — nearest again. It now hands over the native canvas
and names the filter, which is one resample from the best source there is instead
of two from a worse one.

`/dump-windows` reports the decision, since a band's cell rect never could: every
band's log line ends with `resample 222x254->200x234 x:area y:area`. If art ever
looks wrong at a particular size again, that line says which direction it moved and
which filter it went through.

Nothing changes at or above native size. A magnifying resample is still exact pixel
replication, and the corpus tests pin it that way.

The rule outgrew v6. It arrived here because Journey's canyon needed it, but nothing
about "filter by the direction the axis moves" is Z-machine-shaped, and every other
place lanthorn scaled a picture had quietly picked its own answer: Glulx's
`glk_image_draw_scaled` smoothed art it was *enlarging*, and cover art, gallery
tiles, the resource preview and the non-Kitty graphics-window blit all deferred to
the image crate's default filter, which is nearest — a decimation, at exactly the
several-fold reductions a jacket scan into an info panel goes through. They now all
call the same resampler (`resize_directional`, and `fitted_protocol` for the ones
that fit into a cell box), so the answer to "what happens to this picture" is one
answer and not six.

### The seam that came with it

An area filter averages neighbours, and it will happily average a pixel that is not
there. lanthorn's canvases are RGBA, and a *transparent* pixel carries the colour
`(0,0,0)` behind its zero alpha — a colour no game ever drew. Filter the four
channels independently and every place opaque art meets clear canvas comes out with
its colour dragged toward black and its alpha dropped to match; composited, that
reads as a dark hairline exactly one pixel wide.

It was reported on the Amiga Zork Zero floppy as *a very thin dark line down both
edges of the story pane*, which went away when the terminal was made wider — wider
means the flanks grow rather than shrink, which puts them back on the nearest arm,
which never blends. At an 83-column terminal the flank shrinks 95 native pixels to
84, and the pixel where its story page meets clear canvas went out as
`(38,38,38,57)`: over the page the band is drawn on, 142 against that page's own
173.

Every blending pass now runs on *associated* (premultiplied) colour, so the average
is one of light-with-nothing rather than light-with-black, and the seam comes back
out as page. It costs nothing where it is not needed: at full opacity both
directions round-trip exactly, so a fully opaque plate — Journey's canyon, every
number in the table above — is bit-identical either way, and a magnifying pass skips
the conversion entirely.

### One resample, and the 2× that is not a second one

Journey's canyon plate lives in unit space as 222×254 pixels, and its *artwork* is
111×127: the plate is that art replicated exactly 2×, the uniform `V6_ART_SCALE`
every v6 picture reaches the 640×400 unit screen through. Which naturally raises
the question — is the picture being scaled twice, once to double it and again to
fit the pane, with the second scale sampling the first one's guesses?

No. It is two *calls*, and that is not the same thing as two samplings. Doubling at
an integer ratio is pure replication: output pixel `i` takes source pixel `i/2`,
rounded down, and nothing is invented. A nearest resample of *that* takes
`floor((o+0.5)·2N/T)` of it, and `floor(floor(2u)/2) = floor(u)` for every `u` — so
the pair **is** the single resample `floor((o+0.5)·N/T)` straight from the artwork's
own resolution. Not approximately; bit for bit, which is what
`the_art_scale_predouble_composes_away_under_nearest` asserts at the ratios the pane
sweep produces. Wherever a band magnifies — every pane from about 80 columns up —
the pixels on screen are already one resample from the native artwork, and swapping
the source for the artwork itself cannot change a single one of them.

What it *would* change is the direction decision above, and only in one narrow band:
a target between the artwork's size and its double magnifies from the artwork while
it minifies from the unit-space plate, so the same target picks nearest one way and
the area filter the other. Measured on the real plate against the artwork's own area
average, at the sizes the small panes ask for:

| target | from the unit plate (shipped) | from the artwork |
|---|---:|---:|
| 160×180 | **RMS 1.96** | RMS 11.26 |
| 168×198 | **RMS 1.64** | RMS 10.68 |
| 200×234 | **RMS 0.80** | RMS 9.93 |

Those right-hand numbers are the aliasing this whole section is about, back again:
sampling the artwork means *magnifying* it 1.4×, and nearest at 1.4× keeps some art
pixels twice and others once. So the pipeline stays as it is. The unit-space plate
is the better source precisely because it is bigger, and the pre-double costs
nothing to compose through.

What the non-integer magnification *does* cost is uniformity. At a 166×44 terminal
one art pixel is 3.69 device pixels wide, so the emitted image draws it as 3 pixels
sometimes and 4 others (measured across the plate: 7,757 runs of 4 against 3,372 of
3). Snapping that factor to a whole number would make every art pixel exactly 4 wide
— but the factor is the uniform letterbox scale the story viewport is mapped through
as well, so it cannot be moved without moving the text with it. That is the trade
`v6_pixel_lock` now offers, and the next section is what it costs and what it buys.

### `v6_pixel_lock` — a whole number of device pixels per art pixel

Off by default. Turn it on (settings screen, `v6_pixel_lock = true` in
`config.toml`, or `/set-v6-pixel-lock` mid-game — see below) and the letterbox
magnification stops being whatever fraction fills
your pane and becomes a rung of a ladder, chosen so that **one art pixel is always a
whole number of device pixels**. The 3-and-4-wide runs above become 3s or 4s and
nothing in between; a resampled edge meeting a font glyph on a shared boundary stops
landing half a pixel off; and every tiled side border repeats on an exact boundary,
because a tile is cut at whole art-pixel boundaries and an integral art pixel makes
its height integral too. Crisp art and seamless flanks turn out to be the same
constraint, so there is only the one switch.

**The ladder is derived from the artwork, not chosen.** This matters more than it
sounds, because the obvious ladder — 1×, 1.5×, 2× — is right for most of the corpus
and wrong for two of its presses. An art pixel is `art_scale` unit pixels (the
per-axis density lanthorn computes at boot from the archive's own declared picture
space) and a unit pixel is `s` device pixels, so both axes need `art_scale · s` to be
a whole number, and the coarsest step satisfying both is `1 / gcd(art_scale)`:

| press | art space | `art_scale` | step | ladder |
|---|---|---|---:|---|
| most v6 — Blorb, Amiga `Pic.data`, MCGA `.mg1` | 320×200 | (2, 2) | ½ | 0.5×, 1×, 1.5×, 2× … |
| Macintosh monochrome `Pic.data` | 480×300 | (1, 1) | 1 | 1×, 2×, 3× … |
| Macintosh colour `CPic.data` | 320×200 | (2, 2) | ½ | half-steps |
| EGA / CGA `.eg1` / `.cg1` | 640×200 | (1, 2) | 1 | 1×, 2×, 3× … |
| Apple II | 140×192 | (4, 2) | ½ | half-steps |

So the familiar half-step ladder falls out for the common case rather than being
assumed, while the standard Macintosh's mono plate — already 1:1 on its own screen —
and the 640-wide EGA and CGA renditions get whole steps only. A half step on EGA
would put its half-width art pixels on half a device pixel, which is the very thing
the mode exists to prevent. Pick the same rendition of one game on two different
media and the ladder changes with it, because the artwork did.

**A half step is whole for the ART and half for the TEXT, and that is a real cost.**
The ladder above is derived from the artwork, and it has to be — but raster text is
not drawn at the artwork's density. It is drawn on the machine's character cell,
which lives in *native* pixels. So on any press where one art pixel is already two
native pixels, a rung that is whole in art terms is a **half step in native terms**,
and a 7-pixel-wide Macintosh glyph asked for 1.5× gets ten and a half device pixels:
its strokes come out alternating one and two pixels wide, and `l` and `i` go ragged
while the compass rose in the same frame stays perfectly crisp. That contrast inside
one image is the signature.

It is a property of the *press*, not of the font. The Macintosh's monochrome
`Pic.data` is 480×300 at (1, 1) — art pixel and text pixel are the same size — so
every rung there is a whole native step and its text is never touched. Its colour
`CPic.data` is 320×200 doubled, so the half rungs are where the text breaks. Sit on
a whole multiple and it is sharp again. Skipping the half rungs on the presses that
cannot use them is tracked as a fix; until then, whole rungs are the workaround, and
the mode remains off by default.

The magnification stays **uniform** — horizontal equals vertical. The non-squareness
of EGA and Apple II pixels is already carried by `art_scale` itself, so a uniform
factor on top of it preserves the shape the artist drew.

**One factor for the whole screen, never one per picture.** Journey settles that.
Its illustration sits in a window of its own with a drawn divider rule beside it, so
quantizing each picture separately would stop the art short of its own frame and open
a gap between the picture and the rule. Quantizing the screen's single factor moves
the window and the artwork in it together, and the art still fills it exactly. Journey
is treated like every other title: its frame letterboxes horizontally rather than
spanning the pane.

**What it costs is screen area.** The picture stops at the rung below your pane
instead of filling it, so the margin around it gets wider — sometimes considerably,
since the gap between rungs is up to half the picture. That margin is not dead space:
it is painted with the story's own page, or the machine's when the story names none,
exactly as it already was.

**A pane too small for even the smallest rung falls back to free scaling.** It never
blocks and never says anything on the game screen — the same way every other
too-small decision in lanthorn degrades rather than refuses. (The fallback is
diagnostic state, destined for `/info`.)

**And it does nothing under half-blocks, on purpose.** The whole promise is stated in
*device pixels*, and half-blocks has none: it paints `▀` in two colours per cell, so
the picture resolves onto one sample per column and two per row, and the "font size"
the rung was counted in is a hardcoded 10×20 that `ratatui-image` itself calls
completely arbitrary. Quantizing onto the sample grid instead — the honest analogue,
and computable from the cell grid alone — was tried and measured, and it buys nothing,
because half-blocks never magnifies at any size you would run: a 640×400 canvas has
more pixels than a terminal has samples until the pane reaches 640×200 *cells*, so the
picture is always being shrunk, and shrinking goes through a smoothing filter. On a
640×400 plate of 2×2 art pixels in hard black-and-white stripes:

| sample grid | ratio | samples that are a pure art-pixel colour |
|---|---:|---:|
| 640×400 | 1:1 | 640 / 640 |
| 458×288 | 1.4:1 | 50 / 458 |
| **320×200** | **2:1** | **0 / 320** |
| 160×100 | 4:1 | 0 / 160 |

The 320×200 row is the honest ladder's own rung — one art pixel onto exactly one
sample — and every sample still lands on a 25/75 blend of two art pixels. Meanwhile at
1:1 and above the art already comes out pure without any lock, and the lock could only
move the factor *down* off that plateau. So there is no pane size at which it helps and
a measured 17–20% of linear resolution to lose where it acts (at a 120×40 pane a
640×400 canvas free-scales to 120×38 cells; the old device-pixel rung cut it to 96×30).
Turning the switch on under half-blocks therefore changes nothing, and `/dump-terminal`
reports the lock as `INERT` rather than claiming a snap that did not happen. This is not
a magnification ceiling — half-blocks has had none since it stopped needing one, and the
free scale still climbs the whole way to your pane.

**And it is a per-game preference, not a global one.** Which side of that trade you
want depends on the press you mounted — a 320-wide Amiga rendition has half-steps to
land on and gives up little, while the Macintosh mono plate's whole-number ladder can
cost you half the picture. So `/set-v6-pixel-lock` writes its answer into *this*
game's `config.toml` sidecar and nowhere else:

| | |
|---|---|
| `/set-v6-pixel-lock` | flip whatever is in force, and remember it for this game |
| `/set-v6-pixel-lock on` / `off` | say it outright |
| `/set-v6-pixel-lock auto` | forget this game's answer and inherit your global `v6_pixel_lock` |
| `lanthorn story.z6 --v6-pixel-lock on` / `off` | before the game boots, for this launch only |

It takes effect on the next frame — there is no reload, and no restart. The bare
form is a toggle precisely so it can be bound to a key and used to flip back and
forth while you decide. Your global `config.toml` is never touched by any of this,
including when you have the settings screen open at the time: a per-game value is
held apart from the file exactly as `--game-colours off` is, and only an edit to the
settings screen's own **v6 pixel lock** row changes the global default. The flag
is the strongest of the three: it outranks the file *and* this game's sidecar,
because a flag is an instruction for the launch you typed it on and a file
beside the story is not.

### A picture is not stretched by the grid it sits on

A flank panel's art is placed at *cell* granularity — whole terminal columns and
rows — and cells are 8 wide against 18 tall. Rounding each axis up on its own
therefore rounds them by quite different amounts: Journey's 222×254 plate at an
80×24 pane, where the uniform scale is exactly 1.0 and the art wants its own size
back, went into a 224×270 box. That is ×1.0090 horizontally against ×1.0630
vertically — the picture drawn **5.3% taller than it is wide**, for no reason anyone
chose.

Both axes are now picked together, against the exact criterion `cols·cw·dh ==
rows·ch·dw`, over the four boxes the ideal falls between. Every candidate is within
one cell of the ideal on each axis, so this can neither inflate the art to fill its
column nor starve it — it only chooses which corner of the grid to land on. The
80×24 answer becomes 224×252, a 1.7% error, and that is the floor: fourteen rows of
18 pixels cannot express 254/222 any better. The pane sweep improves everywhere it
was wrong and moves nothing that was already right.

## Render modes

Set `v6_render` in the config (or cycle it from the settings screen) to pick
how a v6 story's pane is drawn on an image-capable terminal (Kitty, iTerm2, or
Sixel). Want to compare looks mid-game? `/set-v6-render` hops to the next mode
on the spot (or jumps straight to one: `/set-v6-render raster`), and the answer
sticks **to that story** — written into its own `config.toml` sidecar, alongside
`v6_pixel_lock`, never into your global config. `/set-v6-render auto` hands the
game back to your global default.

That is a deliberate reversal, worth saying plainly: the switch *was*
session-only, and that was right for what it then was. Raster began as a
**fallback** — the mode you escaped to when hybrid could not cope — and a
temporary escape hatch should not outlive the session that needed it. Raster is
a destination now, with `extended` beside it, and a player may genuinely prefer
raster for one game and hybrid for another. A preference about one story's
artwork belongs with that story.

`--v6-render hybrid` /
`--v6-render raster` says the same thing one moment earlier, before the game
boots — so the *first* frame is already the one you meant, which is what a
screenshot, a bug report and a headless capture all want:

- **`hybrid`** (the default) — the decorative chrome (banner, borders, the
  compass) renders as a single scaled pixel image forming a **ring** around an
  inset viewport, and the story text inside that viewport is real terminal
  text: crisp, selectable, scrollable, and styled exactly like any other
  lanthorn transcript — including its own inline images (see below). The ring
  is tiled into up to four non-overlapping bands (top/bottom/left/right)
  around the viewport; a band flush against the pane edge is simply omitted.
  Each top/bottom band is then decomposed further: a horizontal strip that is
  **pure chrome text** — status/menu runs with *no* opaque frame art behind it —
  drops out of the pixel ring and paints as **real terminal cells** (crisp,
  selectable, themed via the game-colour resolver, with solid reverse-video
  bars), while a strip that sits over actual artwork keeps the scaled pixel
  image. So Journey's bottom command menu ("Proceed / Back / Game", the party
  column, the verb columns — a full-width window sized to zero height that paints
  fixed pixel runs) becomes terminal text while its left picture column (and the
  reversed vertical divider the game paints between picture and text) stays
  imaged; Arthur's location/date status row becomes a crisp reverse bar sitting
  between the graphics panel above it and the story below; and Zork Zero's
  status, painted directly *onto* its banner art, stays in the ring.
  On the **half-block** backend that last case is drawn with real glyphs anyway,
  in a background sampled from the picture behind each cell, so "Banquet Hall" and
  "Score: 0" sit *in* the ribbon rather than in a box. It is not a preference and
  has no setting: a half-block cell is `▀` with a foreground and a background —
  two vertical samples — so a rasterised 8×16 glyph arrives as 8×2 and is
  unreadable rather than merely coarse. That is the backend a terminal with no
  graphics protocol falls back to, the one tmux gets, and the one an asciinema
  cast records, so the difference is between a legible game and coloured mush with
  a picture in it. Under **kitty** the same text stays rasterised, and that is
  faithful — art and characters composite exactly as the game drew them, at the
  game's own resolution. It is also the only thing that works there: lanthorn's
  placements are *virtual* (`U=1`), positioned by `U+10EEEE` placeholder
  characters, so the image is the cell's content and printing a glyph into a
  covered cell deletes the image instead of layering over it — and truncates the
  rest of that row's run. Sixel and iTerm2 have no Z index at all. The capability
  is therefore asked of the picker that negotiated rather than configured.
  Two things follow that are easy to miss and both were reported from a real
  screen. The rasterised copy of that text has to leave the band, or the crisp
  glyph lands beside a blurred twin of itself — the banner's runs sit at native
  rows 10 and 26, a text row off the cell grid's 0 and 16, so a canvas that
  re-derives their position from the grid paints them back one row up. And the
  frame art's **holes** have to be filled before the band is encoded: a
  half-block cell has no alpha, so a transparent pixel arrives as black, which
  put a black gutter down either side of Zork Zero's pillars where kitty shows
  the white page the story declared. Those holes now resolve to the same page
  the raster composite has flattened onto since it shipped, so the two modes
  agree about the same frame. A pure
  reverse-video row (a status/menu bar) fills **edge to edge across the full pane
  width**, so a bar the game drew as separate runs with bare cells between and
  around them reads as one solid block. A **rule** — three or more abutting
  fragments of the same symbol glyph, which is how a game draws a horizontal line —
  gets the same treatment one layer up: it is drawn across the width its own pixels
  span, not one terminal cell per fragment, and it closes the seams the scale opens
  around its corners and titles. That is what lets Journey's line-drawing border
  (which is what the Amiga interpreter profile makes it draw instead of reverse-video
  spaces) reach both edges of the pane at any window size, with the prose wrapping
  inside it. Prose is untouched by the rule: a label's character count is its width,
  because it has to stay legible. The predicate is narrow on purpose — a game with
  proportional metrics emits one run per glyph, so "two equal abutting fragments"
  would read every doubled letter in the corpus as a rule, and Arthur's status bar
  loses its character's name the moment it does. A **lone** line-drawing or block
  glyph gets the other half of the same idea: it is a column **divider**, so it is
  kept out of the fragment merge and stamped at its own scaled column. Abutting
  fragments are otherwise glued back together, because a game with proportional
  lettering hands over one word as several runs and they must read as one word — but
  a glued run then advances one terminal cell per character, and Journey sets each
  party member's `-->` marker flush against the divider after it, so the divider rode
  the marker's letters and stood in a different column on every row that had one. A
  rule is a distance, a divider is a position, and both are placed by pixels; prose
  is neither. A game that never reserves a band and
  instead **overlays** its bar on the top row of a full-screen prose window
  (advent.z6) is given one: a full-width strip of at most two rows, pinned to the
  top of the screen, has its rows reserved off the story viewport so it decomposes
  into an ordinary text strip — a solid bar with the transcript starting beneath it,
  rather than glyphs stamped over scrolling prose. Such a bar need not be
  reverse-video to fill the row; a window that shape *is* the status bar.
  A ring is also nothing when the story window covers the **whole screen**: there
  are no bands left to carve, so no artwork behind it can be shown at all. Such a
  frame is handed to the full-picture composite instead — and the rule is simply
  that, with no ring to draw in, a story window whose own picture paints *anything*
  has nowhere else to put it. That was arrived at the long way round, by asking
  first whether the art *filled* the screen (Zork Zero's map, Arthur's illustrated
  intro plates, Journey's title) and then whether it merely *enclosed* it. fmvpoker's
  poker table is the second kind: a 640×400 frame with a hollow middle that the game
  prints its whole title inside, only 17% of its pixels painted, which the fill test
  missed at every point that mattered. The Mysterious Adventures are neither kind —
  their boot stacks two 512×192 title cards down the left of the screen, leaving the
  right-hand quarter bare — and for a while lanthorn drew neither card, because two
  tests for two particular shapes had quietly stood in for the one fact that
  mattered. Both shapes are special cases of it, and the general rule moves no other
  title: crisp terminal cells are worth having, but not at the price of the picture.
  **Art the game paints inside its story window now has somewhere to go.** For a
  long time hybrid's story region was terminal cells and nothing else: a picture
  drawn into window 0 belonged to neither half of the screen — no band carried it,
  because the ring is what lies *outside* the story viewport, and no viewport showed
  it, because a viewport shows text. Prose was drawn straight over artwork the
  player could not see. The story viewport is now cut from what the art *leaves*:
  the window is inset past any frame artwork touching its edges, then reduced to the
  largest rectangle the window's own picture painted no pixel of. Everything outside
  the viewport already belongs to the ring, so the picture is drawn there, by the
  same machinery that draws the frame — and where the picture leaves no room for
  text at all, the whole pane becomes ring and no transcript is drawn on that frame,
  which is what the game itself intended when it erased the screen, drew, and waited
  for a keypress. A picture in one corner of the story window costs the prose that
  corner and nothing else; the rest of the region stays crisp glyphs.

  That capability retired the fourth way a frame could have no ring. fmvpoker used
  to lose its whole frame for as long as the player took to type a bet: choosing
  *Change Current Bet* hands the read to its bottom panel, and while lanthorn
  treated that panel as the story window, the window still holding the poker table
  stopped being one and the table was drawn by nobody. Two separate fixes have since
  removed that — a display panel the game declares is not its transcript never
  becomes the story window, and a picture reaching outside its own window now lands
  in the ring like any other pixel — so the special case for it is gone.
  Not every v6 game *has* a story window to ring, though. scopa's card table
  streams no prose at all — its screen is three grid windows and a table drawn out
  of filled rectangles, with two button labels on top — and a ring around nothing
  is nothing. A screen with no story window is presented whole instead: as crisp
  positioned terminal text when it really is only text (a hint menu, a boot menu),
  and as the **full-picture composite** when the game has painted pixels onto it,
  because those pixels *are* the screen and the composite draws the labels over
  them anyway. So hybrid shows scopa's table exactly as raster does, and Zork
  Zero's InvisiClues stays the readable full-pane text screen it has always been.
  **A takeover screen that keeps its story window is sorted by what is behind it.**
  A game can print a menu *inside* the box its story window occupies — Shogun's
  boot menu, advent's help popup. advent has no artwork in it anywhere, so its
  popup is the coherent all-text page it has always been. Shogun's boot menu sits
  on the machine's own ground between two panels of gold filigree, and sending it
  to that text page lost every one of those pixels — the panels gone and a
  full-width black block where the frame belongs, on the Amiga floppy, the IBM
  Blorb and the Apple ProDOS set alike — so it takes the **ring**, which draws the
  panels as artwork and the credits and the menu as ordinary terminal glyphs.

  That screen used to take the composite instead, and the reason it could not take
  the ring is worth keeping, because it is a nice illustration of what hybrid is
  actually doing. Shogun prints its menu one character at a time through a
  one-pixel-wide caret window: `START the game` arrives as fourteen separate runs,
  each eight game pixels along from the last. Each run used to be placed on its
  own — its game pixel mapped through the picture's scale and rounded to the
  nearest cell — and at a pane where a cell is 1.2 game characters wide, rounding
  fourteen neighbours independently makes some of them collide into one column and
  others skip one. `SI(RT th e ga me`. The same arithmetic on the other axis put
  the three menu items on rows 26, 28 and 29 and pushed the first one off the top
  of the story window altogether. Runs the game printed as one stream are now
  grouped and placed together, and a block of centred lines — the credits — is
  laid out in the game's *own* text grid rather than line by line through the
  picture's scale, so the nine lines share one centre again instead of drifting up
  to five columns apart.
  Grouping the stream fixed the scattering and left one tail. A run is placed
  through the picture's scale but then advances one terminal *column* per
  character, and the two rates only agree where a column is exactly one eight-pixel
  game character. Below that — any pane smaller than the game's own 640×400 screen —
  a group of glyphs is wider in columns than the game meant it to be, so the blank
  the game painted immediately after it, which in game pixels touches it without
  covering any of it, maps to a column *inside* it. `START the game` came out
  `START the gam`, and `RESTORE a saved gam`, and `QUIT the gam`, with the final
  letter painted over by a space. A blank run carries no glyphs and in game pixels
  only ever covers whitespace the glyph run drew itself, so it now paints the cells
  no glyph claimed — the parts of a selection bar that reach past its label — and
  skips the rest. Above 1:1 the group *under*-runs instead and the blank lands
  clear, which is why this only ever showed on a small terminal.
- **More than one scrolling text window.** A v6 game may run several flowing-prose
  windows at once — advent.z6's `style` opens one across the top of the screen and
  keeps playing in another below it. Both are wrap+scroll, so both stream through
  the same text path, and splicing them into one transcript scrolled the top
  window's text away with the story (the game warns about exactly that). Which
  window carries the narrative is the game's own declaration: ZMSD §8.8.3.1's
  attribute 2, "text copied to output stream 2", is set on the transcript's window
  and cleared on a display window. lanthorn follows that — with the window the game
  reads input through as the fallback for a game that declares nothing — and gives
  every other prose window its own buffer, drawn in its own rect. A **read does not
  overrule the declaration**: fmvpoker prints "Enter the new bet:" into its bottom
  panel and reads the answer through that panel, and treating the read as the
  answer split one screen across two sinks — the prompt stayed behind in the
  panel's buffer while the panel was published as the story window, whose lines are
  empty by construction, so the player got a blank panel with no prompt, no running
  totals and no echo of what they typed. The **live input line follows the read**
  rather than the story window, so it appears after the prompt in the window the
  player is actually typing into. A secondary window is **live
  screen state**: what it currently shows, with no scrollback, cleared when the game
  erases it — but persisted with the rest of the screen, because a game that splits
  the display does not necessarily repaint it after a restore (advent doesn't). Its
  lines are drawn on the **pixel composite** too, stacked from the window's own
  origin one text row each — fmvpoker prints its bottom menu and its "Select an
  option…" hint into one, and the composite used to draw graphics and grid windows
  and nothing else, so a screen the cell paths showed in full came out with that
  strip blank.
- **Erased windows are opaque.** On a real interpreter every v6 window is a
  clipping region over one shared screen bitmap, so erasing a window paints its
  rect with that window's background — which is what makes a hint menu hide the
  story behind it. lanthorn composites layers instead, so it tracks the erase:
  a window stays an opaque field until the story prints another character, at
  which point the prose is the newer paint and the fill stops covering. That one
  rule keeps both cases right — advent.z6's `help` (erase the screen, split a
  160px window, erase it, paint the menu, and print no prose) reads as a solid
  panel on blank background, while Zork Zero's full-screen decorative window,
  erased to white during boot *before* a word of story has printed, never
  blankets the transcript. The rows that become cells are carved out
  of the pixel bands entirely — their rasterized ink never reaches an uploaded
  band image (no raster bar showing through behind the cells), and because a
  band's image no longer depends on that text, navigating the menu re-encodes only
  the genuinely changed artwork rather than every band. The whole cell strip is
  first flooded with the chrome background so the panel reads as one solid block —
  no theme backdrop peeks through the cells between the runs — and when the
  letterbox scale spreads the menu's native rows across *more* terminal rows than it
  has (leaving a blank row mid-menu), that gap row is folded back into the panel and
  its reversed vertical column dividers are carried through, so the lines never
  break. Text that a game positions with proportional (sub-cell) pixel metrics —
  Arthur emits its status words as separate abutting single-glyph runs — is
  reassembled: fragments whose pixel start touches the previous run's end merge into
  one word stamped from a single cell (so "Churchyard" stays whole instead of
  scattering into "Chu rch yard"), while runs held apart by a real pixel gap (menu
  items, column dividers) keep their spacing and never fuse. The glue stops at
  **padding**: a merged run is positioned once by the scale and then advances one
  terminal cell per character, so a field glued to the blank cells in front of it
  inherits their starting column and drifts away from its own as the pane grows.
  One blank cell is a word space and still merges — Arthur's "St Anne's Day,
  Compline" is one phrase — but a wider blank stretch is layout, and what follows
  it is a field with a column of its own. Shogun off the Amiga floppy paints its
  whole status band one run per cell, padding included, and that is why its `Score:`
  and `Moves:` used to line up only at an 80-column story pane and drift apart at
  every other width; Journey's `-->` party markers, glued to the names in front of
  them, stepped left beside the shorter ones for the same reason.
  A game that prints a **label over its own rule** leaves stray fragments of that
  rule buried inside the label's pixels — Journey's release-30 menu header has one
  under each of its two titles — and those are the label's, not dividers: they are
  dropped in the game's own coordinates, native against native, so a title's rule
  closes up against it at every pane. Judged in terminal columns instead, the answer
  moved with the pane: a stray 80 native pixels into a 19-character title fell one
  column past the title's last cell once the pane passed about 1.9 columns per native
  cell, and pushed the rule behind it one further right again — a single blank cell
  after `Individual Commands`, at 155 columns, and 157 and up, but not at 154 or 156.
  The ring layout is also **dynamic**: on a pane taller than the game's native
  aspect there is vertical letterbox dead space, and hybrid mode reclaims it rather
  than centring the frame in it. When nothing sits below the story — header art,
  side borders, a status bar on top, but an open bottom (Arthur) — the ring is
  anchored to the pane top and the story viewport grows all the way to the pane
  bottom at its exact inset width; where the side art runs out, the border is
  **tiled** down the rest of the flank (below). When the game has a bottom text
  chrome instead (Journey's command menu), that strip is anchored to the pane
  *bottom* edge and the story fills the space between the top chrome and the menu.
  A game whose frame *encloses* the story to the screen bottom (Zork Zero's full
  frame) keeps the centred letterbox untouched, and a pane at or below the scaled
  native height (no dead space) degrades to that same centred layout.
  **What the story reclaims is dead space, and a row the game is using is not
  dead.** "An open bottom" is a claim about the frame, and the planner reached it
  by forgiving the last native text row — a story window ending within one row of
  the screen bottom counted as reaching it. Arthur spends exactly that row. Ask
  him for a hint in play and he lays window 3 across the foot of the screen and
  prints *"If only you had a crystal ball...."* into it; answer him with a blank
  line and he puts *"I beg your pardon?"* in the same place. At 80×25 — his own
  screen, no slack, centred letterbox — the box was drawn all along, and at every
  taller terminal the story viewport grew straight over it and the message was on
  no screen at all. Raster never lost it, which is the tell: its composite is
  built at native size, so a pane cannot take a row out of it. So the reclaim now
  stops above whatever the game keeps below its story window and bottom-anchors it
  to the pane's last rows — the same treatment Journey's command menu has always
  had, for the same reason. The transcript keeps everything else: window 0 is a
  scrolling buffer and those rows are its history, so at 80×34 it is 20 rows deep
  against its 11 native ones.
  **What the pane is shaped like decides how much is reclaimed; it never decides
  what the game has.** Those are two questions and the planner used to run them
  together, taking its "no dead space, centre it" shortcut *before* it had so much
  as looked for a command menu. So on any pane whose vertical edge is the binding
  one — a wide, short terminal, where the frame already fills the height exactly —
  Journey stopped counting as a menu game at all, and everything that follows from
  being one went out with it: its picture column lost the panel it sits in, lost its
  fill sampled from the art's own edge, lost its centring, and fell through to the
  side-border tiler, which happily tiled canyon wall down a column that was never a
  border. It is a frame-shaped fact about the game, so it is asked first now, and
  slack gates only the reclaim. A menu game at zero slack simply gets "menu, no
  reclaim", which costs nothing to arrange: with no dead space the centred offset
  and the top-anchored one are the same offset, so not a band moves — only the
  flank's treatment changes, which was the whole of the complaint. 166×44 was one of
  these panes, and one of the commonest: 164×41 cells of v6 area on an 8×18 cell is
  1312×738 device pixels, and 738⁄400 = 1.845 lands on the scale exactly.
  **One window is fixed-height and the rest take what remains — and in Journey it
  is the one at the bottom.** Nearly every v6 title puts its fixed window on top
  (Arthur's status bar, Zork Zero's banner) and lets the story grow downward;
  Journey inverts it. Its artwork sits left, its story right, and its command menu
  runs along the foot of the screen, and it is the *menu* whose height is a property
  of itself: fixed in y, dynamic in x, with the art and the story taking the width
  and whatever height is left above it. Since hybrid draws chrome text as text —
  one game row to one terminal row — that fixed height is simply the span of the
  game rows the menu carries, which is how "native pixels → terminal rows" cashes
  out for a strip made of characters. The planner used to compute it the other way
  round, deriving the story viewport from the letterbox and handing the menu the
  remainder, so the band's height wandered with the pane (nine rows at one scale,
  eleven at another) while its content stayed a constant seven game rows. The rows
  the menu never reached were painted by nothing, which stranded Journey's own
  `└────┘` three rows above the pane's last row and trailed an empty upload after
  the band; at a short pane the same arithmetic ran the other way and clipped the
  menu's last line off the screen entirely. The menu now ends exactly where the
  screen does, at every pane shape and on both releases.
  **A flank is where the frame's side artwork is, not what the story box left
  over.** The ring used to be defined as *pane minus story viewport*, which meant a
  flank had no existence of its own: it was whatever fell in the third or fourth
  rectangle of that subtraction, so its vertical extent was the story window's by
  definition and the same column was drawn in up to three pieces by two different
  routines off two different canvases. Zork Zero's left pillar came out as six rows
  of a full-width top band cropped at 1.2250 and thirty-one rows of a side band
  resampled at 1.2308 by 1.2237 — half a pixel of shear, on a join the eye missed
  only because both halves happened to be reading the same continuous artwork.
  Now the ring is carved from what the chrome *contains*. A flank's rows are every
  contiguous run it may own, stopping at chrome text that stands in *its own
  columns*, at a bar the game draws edge to edge, at a secondary prose panel, and at
  a bottom command menu — and the frame's two flanks are then held to the same row
  set, because they are one object drawn twice and nothing about the pane may make
  them disagree. (Shogun is why: his status band sits at native x 46..594, exactly
  between the two ornaments, and its first glyph is at 49. At one pane size the left
  ornament's last column and that glyph round into the *same* terminal column and the
  text wins it, while the right ornament's columns are clear — one top corner
  ornament, the other bare.) Its columns come from the artwork itself — for each
  native row, the first contiguous opaque run in from each edge, taking the
  narrowest such run over the whole picture, so that a banner or a capital above the
  column is never mistaken for the column. The wider of that and the story box's
  leftover wins, and where the artwork claims columns the story window declared, the
  window gives them up.
  And *at the edge* is part of that sentence. A run counts only if it starts within
  one of the game's own text cells of the edge it is claimed for, and a run stopped
  by the canvas's own midpoint — which is where the scan is bounded, because two
  flanks meeting would leave no screen between them — is not a measurement of a
  column at all. Arthur's Apple II press is the frame that made the omission
  visible: release 63 shows a single illustration painting native columns 250–389
  and nothing else, so the run in from the left and the run in from the right were
  the same picture read from opposite sides. Each flank came back 253 native pixels
  wide — 39 terminal columns at a 98×37 pane, 67 at 169×62 — and the sliver of
  picture inside it was then tiled down the whole column, so the artwork repeated
  down both flanks at every pane size and a tall pane simply showed more copies of
  it. That frame now has no flanks: its illustration is drawn once, in the band
  above the story, at the frame's own magnification. Arthur's four-pixel gutter
  still counts as a pole at the edge, because a frame is authored on the game's text
  grid and its ink may sit anywhere inside its own eight pixels.
  Two things fall out that used to be impossible. Zork Zero's pillar is one image at
  one scale down the whole pane, and the seam is simply gone. And the same title on
  two presses now lays out the same: Shogun's credits screen puts window 0 at
  `548×64` on the IBM Blorb and at the full `640×64` on the Amiga floppy, which used
  to mean ornaments on one and none at all on the other — the frame is identical in
  both, and now so is the ring. (Both of those screens still take the pixel
  composite for an unrelated reason — the game prints its menu one character at a
  time through a one-pixel caret window — so the ring's answer for them is measured,
  not yet on screen.)
  **Side border art is TILED down the flank, never stretched into it.** Three
  titles frame their story window with artwork drawn for a 320×200 screen —
  Arthur's poles, Shogun's single-piece border, Zork Zero's pillars — and a modern
  pane is taller than all of them. Stretching the band to fit elongates the art by
  whatever the slack happens to be: measured at 1.8× vertical against 1.3×
  horizontal on Shogun at a 100×40 terminal, and 2.2× against 1.0× on Zork Zero at
  117×64. So each flank is now composed in the game's own native pixels — capital,
  a repeated shaft, then the art's own foot back on at the bottom — and the whole
  strip is scaled once, at the same factor as everything else. Two consequences
  worth knowing: the side art keeps the header plate's horizontal factor, so the
  frame still meets exactly at its corners at every pane width; and Arthur's flanks
  are no longer cut off at the row his poles happen to stop (native 379 of 400,
  which on a 64-row pane left the frame standing open down its lower half).
  "At every pane width" took a second pass to earn. A tiled flank asks for the
  native box its band covers, and there is nothing outside the game's screen to
  ask for — so on a pane wide enough that the letterbox leaves a margin at the
  edges, the flank got a source narrower than it wanted and the resize quietly made
  up the difference by changing the magnification. Measured on Arthur at a 70×19
  pane, where the letterbox leaves six device pixels down each side: the pole below
  his status bar drew 30 native columns across all 32 pixels of its band, 1.07 per
  native pixel where the banner beside it was at 0.855, and six pixels to the left
  of it — a fragment in each top corner, at a slightly wrong size, not quite lined
  up with anything. Now the destination travels with the source: a flank lands on
  exactly the device pixels its native box maps to, and the margin beside it stays
  margin, the same margin the banner above it already leaves. Every band on a frame
  is within one native pixel of the frame's one scale, and there is a test that says
  so at panes above *and* below 1:1 — the defect lived for as long as it did because
  every pane anyone had checked magnified, and above 1:1 the arithmetic agrees by
  luck. "Never stretched into it" took a third pass, because one stretch was still
  live and the test could not see it: the exemption that lets a menu panel and an
  extended divider off the one-magnification rule was keyed on the *function* that
  draws them, so a third caller of that function — the last vertical stretch, kept
  for a case nobody could reach — inherited the exemption in silence. It fires
  wherever a flank needs no extension at all, which on Arthur is the piece of pole
  above his status bar, every pane from 0.8:1 to 2:1, on both presses: 400 native
  rows crushed into the banner's 252 pixels, 0.63 vertical against the frame's 1.35,
  which is why his top corners showed a whole squashed pole while the pole below
  them was fine. That arm is gone; those flanks are a plain crop of the same scaled
  screen everything else is cut from, and the exemption now names the two sites that
  hold it rather than the routine they happen to share. The
  three recipes are per title, because the artwork is: the mechanism is a port of
  Bocfel's `draw_border.cpp`, which Spatterlight ships, and which hard-codes per
  game *and* per platform for the same reason. It can afford to, because it draws
  one rendition per run; lanthorn lets you switch archives mid-library, and Zork
  Zero's renditions **disagree about where its pillars start** — the banner above
  them is 34 raw rows on MCGA, 37 on EGA and 39 on CGA, while the pillars are 166
  rows in all three. A repeat unit pinned to one of those layouts lands inside the
  ring beneath the capital on the other two and tiles that ring down the whole
  column as a horizontal seam. So Zork Zero's pillars are **measured, not pinned**:
  the shaft is the longest run of rows holding one opaque width, the capital and
  base are what flare out above and below it, and the cut, the repeat and the foot
  all come off that. On the MCGA and Amiga art the measurement returns Bocfel's own
  four constants to the row, which is what makes it a derivation of them rather
  than a replacement. **Alternate tiles are drawn mirrored**, which is what finally
  killed the CGA seam. Cutting in the plain shaft is not enough on its own, because
  Zork Zero's CGA pillar is a *lit* column: mean row luminance runs 97 down to 82
  from its capital to its base, where MCGA holds a flat 54 and EGA a flat 51. A
  repeat that merely translates such a strip butts its darkest row against its
  brightest and resets the shading at every join — a step of 22.98 against the 14.08
  the art's own shaft ever manages, plainly visible once SQ-0806 stopped painting the
  page white behind it. Flipping every other tile makes each join an exact duplicated
  row instead, so the shading folds back on itself and the seam has nothing to show;
  on the two flat renditions a mirror and a translation are indistinguishable, so
  they are untouched. Spatterlight reaches the same place by hard-coding
  `flip = true` for CGA (and forcing the first EGA tile flipped) with an 11-row
  overlap to hide what is left; a duplicated row needs no overlap at all.
  Which title a flank belongs to is measured too, and **reaching the bottom of the
  screen is not what makes a flank Zork Zero's**. Shogun's DOS artwork is drawn for
  the full 200-line screen where its Amiga art stops at 168, so `.MG1`, `.EG1`,
  `.CG1` and the Blorb all reached the last row and were handed Zork Zero's masonry
  recipe — cut at the shaft, repeat, stamp a foot — applied to a Japanese lacquer
  frame, with `.CG1`'s two flanks disagreeing with each other for good measure. The
  second measurement is the flank's *shape*: a pillar narrows below its banner, a
  slab holds one width top to bottom. Narrowest ÷ widest painted row is 0.96–1.00
  across every Shogun rendition and both flanks, and 0.02–0.81 across every Zork Zero
  rendition and all three of its scene borders, so the cut sits at 9/10 in the gap
  between them.
  **A shaft has to be most of the flank, or it isn't one.** Zork Zero has three
  scene borders — the castle, the underground and the jungle — and Spatterlight
  picks between them by reading the game's own border global, which lanthorn has no
  path to: picture numbers do not survive the engine boundary, and the renderer is
  handed a flattened canvas. Measuring the repeat unit rather than pinning it was
  right for the castle and wrong for the other two, and wrong in an unusually
  visible way: the underground is alternating stone blocks and the jungle is
  foliage, so the longest run of rows holding one width in them is a coincidence —
  and a *different* coincidence in each flank. Composed from each archive's own
  pictures the way the game draws them, `.CG1`'s underground cut its left flank at
  row 78 and its right at row 296, and `.MG1`'s jungle derived a 14-row repeat unit
  on the left while the right fell back to the castle's 284. Six of the eight
  non-castle flank pairs got different recipes from the two halves of one symmetric
  border. The castle holds one span for 70–73% of the flank on every rendition and
  both flanks; nothing else measured manages more than 45%. So a run has to be at
  least half the flank to count — the definition of a pillar rather than a number
  fitted to the corpus — and the underground and the jungle now take the castle's
  constants uniformly, which is what they were getting before the measurement
  existed and is still the right answer for them. The mirrored repeat covers the
  rest: Spatterlight's per-scene overlaps (37 rows underground, 59 in the jungle)
  exist to hide the seam a duplicated row already has nothing to show. What is
  genuinely out of reach is the underground's *stone alternation* — Spatterlight
  swaps the two flanks' 37-pixel stone blocks on alternate tiles so the courses
  trade sides, and that is a statement about the pair which only the scene identity
  justifies.
  **The standard Macintosh's pillars have a ring in them, and a ring will not
  tile.** Every rendition above has a featureless shaft, which is the only reason a
  fixed stride was ever invisible: cut anywhere, repeat forever, stamp the foot over
  the overshoot, and nobody can tell you where the joins are. The monochrome
  `Pic.data` on *Zork Zero Disk.image* (release 296) is drawn differently — capital,
  shaft, a banded ring at mid-height, more shaft, then a flared base — and it broke
  the flank path in two places at once. First it was not recognised as a pillar at
  all: a v6 screen is the archive's picture space rounded up to a whole 8×16 cell,
  the mono space is 480×300, and 300 is four pixels short of the 304-row screen that
  makes, so "does the art reach the bottom?" answered *no* and handed a pillar to
  Shogun's single-piece recipe — which extends a slab by mirroring the whole thing
  below itself. On screen that reads as a doubled foot at the art's own bottom and
  then bare shaft, and at 120×90 a second capital, running past the base and down to
  the pane's last row. Reaching the bottom now means reaching it *to within one text
  row*, which is the largest a full-height plate can ever miss by. Second, and once
  it was recognised: repeating a plain length of a *banded* shaft puts the ring back
  wherever the stride lands, leaves whatever is left over as the gap above the foot,
  and — because alternate tiles are mirrored — moves the ring by twice its offset
  from the unit's centre every other time. So a banded column is composed the other
  way round. The repeat unit is everything below the capital, foot included, and the
  copies are laid at a stride that divides the extension exactly, so the last one's
  base lands on the bottom row and every ring is one stride from the next. The
  rhythm that keeps is the picture's own at every pane height: capital to first ring
  and last ring to base are both exactly what the artist drew, because both come
  from an unmodified copy of the art. A taller pane gets *more* rings rather than
  longer gaps — one at 40 rows, three at 64, four at 90 — which is the articulation
  a 480×300 window never had room to show.
  **Every picture archive in the library is now swept for flank art, so the next
  one cannot hide.** That ring was caught by eye, in one room, on one border set,
  on the single format whose picture space does not divide by the text cell — and
  the test it broke had never been wrong in its life, because every other
  rendition divides exactly. But a flank's composition is a pure function of the
  art and the height it has to fill, so the art no longer has to come off a screen
  somebody walked to. Every archive is opened, every picture in it measured, and
  the ones that can be a flank are composed at three pane heights and checked.
  Nothing names a picture number anywhere: a border is found by *shape* — either a
  plate covering the whole picture space, whose left and right crops are the very
  flanks the renderer takes, or a full-width strip and a narrow column whose
  heights tile that space exactly, which is how the PC renditions ship theirs and
  is enough to turn up the castle, the underground and the jungle without knowing
  that any of them exists. What is then asserted is what has to be true of *any*
  composition: the band is painted to its last row, nothing below the art is
  transparent, every row in the extension is a row the art itself has — so a
  stretch or a shift cannot pass, while a mirrored tile can, because flipping a
  strip does not move a single row's span — a layout that stamps a foot ends on
  that foot, and, the one that would have caught the ring in the first place, four
  pixels of cell rounding decide nothing: not which layout a flank is, and not one
  pixel of what it composes to. Alongside runs a tally of which layout each
  archive's art falls into, pinned, because that is the number which says a newly
  supported format has arrived carrying art that no recipe handles. Arthur is the
  one border the sweep cannot rebuild — his poles stop short of the bottom by
  design and hang at a height the archive never states — so his flank stays pinned
  where it always was, in a suite that boots him.
  The obvious escape from all of this — autocorrelate the flank down its own y
  axis, take the strongest period as the tile height, and never ask which scene is
  on screen — was measured across the corpus and **does not work**, for a reason
  that is structural rather than a matter of tuning: *a pillar shaft has no
  period*. `.MG1`'s is uniform, its rows pixel-identical, so every lag scores
  exactly alike and the search answers with the smallest one it is offered — a
  4-row repeat unit against the 284 the shape measurement gives. `.CG1`'s is a lit
  column shading 97 down to 82, and a gradient is no more periodic than a flat
  wall, so its best lag scores *worse* than an average one. Meanwhile the two
  scenes the idea was meant to rescue fare worse still: the underground's stone
  course does turn up at 74 rows, but with no more confidence than a coin toss, and
  on `.CG1` the two flanks disagree about it — 76 left against 74 right, the very
  asymmetry the majority test had just finished removing. The statistic rewards
  self-similarity, and a plain shaft is more self-similar than patterned masonry,
  which makes it anti-correlated with the thing it was asked to detect. No
  threshold anywhere in the corpus admits the underground and the jungle without
  also admitting the castle, Arthur's poles and Shogun's slab, whose repeat units
  are not periods in the art at all but choices about how much of it to reuse. The
  per-scene dispatch stays, and the corpus measurement is pinned so that a future
  statistic which *does* separate it will say so out loud.
  One trap the recipe has to dodge:
  the canvas a band ships is the artwork *minus* whatever the renderer draws as
  terminal cells instead, so a repeat cut from it copies the holes those cells
  left. Shogun's status line is two 16-pixel rows the top of its border sits
  behind, and cutting the repeat there put a 64-row hole at the join between the
  tiled pieces — 94 screen pixels of black between two ornate gold panels at
  120×90. Its repeats come off the graphics-only canvas instead, which is the
  order Spatterlight works in too: it covers the status bar *after* extending, not
  before. `/dump-windows` labels a band `[Art, tiled]`, reports the native size of
  the source it was composed from, and counts the rows in it that carry no art at
  all — the longest run and where it starts, since a hole is invisible in the
  band's rectangle and shows up only on screen.
  **Raster mode gets the same frame**, because it builds the whole thing at the
  640×400 native screen and hands the finished canvas to a single scale — the same
  way Spatterlight composes at native resolution and stretch-blits once. The flanks
  are extended before that scale rather than at draw time, so raster's corners
  agree structurally instead of by arrangement. It had been left behind when tiling
  landed, and the two pixel modes were drawing different screens from the same turn:
  Shogun's Amiga border ends at native row 336 of 400 and Arthur's poles at 379, and
  raster showed those last rows as one flat colour inside the frame's own lower edge
  — 64 native rows on Shogun, 21 on Arthur. Zork Zero was unaffected either way; its
  pillars already reach the bottom.
  **A picture column over a command menu is not a border, and raster leaves it
  alone.** The hybrid ring had always known this — it builds no tiled flank at all
  for a game with a text strip under its story window, because that flank is a
  picture seated in a panel rather than a frame to extend — and the raster
  extension arrived without the exclusion. Journey paid for it. On the Amiga disk
  (release 30, serial 890322) its illustration paints native rows 25–279 of columns
  0–264, its story window ends at row 288, and "The Party" is printed at row 289;
  recognised as nobody's border in particular, the column fell through to Arthur's
  pole handler, which cut four rows of canyon wall at 90% of the art's height and
  tiled them to row 400 with a 28-row "foot" stamped on the end. The player got
  "Individual Commands" alone on a menu strip half-buried in scenery, and an
  illustration reading a third taller than the artist drew it. Release 83 has the
  same shape and was showing the same thing, so it was never a quirk of one medium.
  Now the two modes agree with the machine again: the art stops where the picture
  stops, and both labels sit side by side on the strip below it.
  In hybrid, **nothing the game printed as a character is ever rasterised**. A strip
  is classified by what is *in* it, never by where it sits: a side column whose
  pixels the game's own paint runs fully account for is drawn with those characters,
  and only pixels the runs cannot explain — genuine artwork — go up as a bitmap.
  Journey draws its frame as text under both interpreter profiles (box-drawing
  glyphs on the Amiga, reverse-video spaces on the IBM PC), so its four vertical
  rules are now stamped in the terminal's own font, standing in the same columns as
  the `┌` and `┐` on the rule above them, instead of arriving as four RGBA uploads —
  about 192 KB a frame to draw two hundred `│`s, in a different renderer from the
  corners they hang off. Zork Zero's, Shogun's and Arthur's side columns are
  pictures, the runs cannot account for them, and they stay pictures. The half-cell
  a story window's edge rounds away goes to the flanks too — **both** edges, since a
  story box has two of them: the frame closes at its top corner instead of leaving an
  unwritten row between the top rule and the first line of prose, and its side rules
  run down to the menu instead of stopping a row short with a pane-wide band painted
  across them. (A band spans the whole pane by definition, so a leftover one under the
  story paints over both side rules at once; the flanks own those columns and take the
  row.) A band that carries the game's own chrome there — a frame closing along the
  pane's last row, as Zork Zero's does — keeps it: the test is whether there is
  artwork *between* the flanks, not where the row sits.
  Clicks follow the same seam. The command menu a game like Journey puts at the foot
  of the screen is a bottom-anchored strip of its own now that the menu is recognised
  at every pane shape, but it used to be an ordinary ring strip whenever the layout
  had no slack to reclaim; both are drawn by packing the game's rows onto consecutive
  terminal rows, so the click map inverts by row index in either, rather than
  inverting the pane linearly and landing a row off in the second.
  The same seam runs through the **InvisiClues hint menu** Zork Zero and Shogun
  share — the screen that says "(Or use mouse.)" and means it. Its topic list is a
  grid printed into the ring's clear middle, drawn as glyphs, and a glyph is
  wherever the renderer put it: the grid is the *game's* screen, so a pane wider
  than the game centres it, and a click map that assumed the first topic sat on the
  viewport's first column looked forty columns to its left at a 190-column pane.
  The map asks the drawing where it put that column instead. Both of its origins are
  native **pixels** now, not cell indices — Zork Zero's box begins at y=78 and prints
  its first topic at y=79, so a row index rounds to a slot the text is not in and
  clicking `GENERAL QUESTIONS` selected `THE JESTER`; Shogun's box happens to fall
  the other way, which is exactly why one specimen is never enough. And a region's
  rows govern the pane's whole width even where its columns do not: the flank beside
  a menu row is tiled artwork, not on the letterbox grid at all, so inverting *its*
  row proportionally reported a y from elsewhere on the screen and a click just left
  of a topic landed several items away.
  And this holds at *every* pane shape, reclaimed layout or
  centred letterbox alike — a short, wide pane leaves no dead space to reclaim and
  used to hand the whole flank, border columns included, to one uploaded band, which
  swallowed the frame's rules into the picture beside them.
  **If a game draws a border, the artwork does not overlap it**: the picture's
  allocated span stops where the rule's column begins, and the rule is stamped as
  the character the game printed. Nothing is lost in the trade — the column was
  already established to hold no artwork before the rule can claim it.
  The border's unit is the game's own **text cell**, not one terminal column, and
  that matters as soon as the scale exceeds a column per native cell (2.93 at a
  236-column pane). A band's crop is *where it is placed* mapped back through the
  letterbox scale, so a destination trimmed by whole columns still starts a native
  pixel or two inside the rule's cell — and Journey inks its `│` three pixels in,
  which is how the game's own rule ended up rasterised *beside* the glyph stamped
  for it: three lines down the left edge, the innermost visibly fatter, and only at
  the wider panes. So the rule's extension spans every column its native cell falls
  in, those columns carry the cell's own ground, and the cell's pixels are erased
  from the canvas the bands are built from — the column-wise twin of the row-wise
  carve that has always kept a text strip out of the bands. The character itself
  still stands in exactly one column; stamping it across the span would be the
  doubled rule this whole rule exists to avoid — and *which* column is decided by
  the game's own screen: a glyph in the screen's edge cell aligns outward, so the
  frame's `┐`, its `┘` and the rule down that side all reach the pane's last column
  instead of leaving a blank one beside it. Everything inside the screen, every
  interior divider included, keeps the column its own run maps to.
  A rule is found by **ink**, not by opacity. The probe that locates one grows an
  opaque run outward from the story window's edge, and a window's *page* — the
  colour the game asked to present on, flooded behind everything while game colours
  are honoured — is opaque everywhere. Journey's Apple II press (release 77, serial
  890616) is where that told: its rule stands at native columns 72–80, the run
  reported 0–80, and what came back was an 83-pixel-wide crop *through the
  illustration*, replicated down the entire left column — 738 device pixels out of
  a single native row at the 171×50 terminal it was reported from, and drawn after
  the picture, so on top of it. The player got vertical bands where the artwork
  belongs. The probe now reads the canvas as the game *painted* it, before any page
  is flooded: the rule comes back as the character it is and is stamped, and the
  picture beside it survives. With game colours declined the same frame was correct
  throughout, which is the kind of split a single-mode test cannot see.
- **`raster`** — the whole pane, story text included, bakes into one
  device-resolution pixel image with a bitmap font, the way the original v6
  engine drew it natively. Its default ink/page follow the theme; where the
  theme leaves them at "terminal default", lanthorn probes the terminal's own
  foreground/background at startup (OSC 10/11) and paints in those, so raster
  text stays readable on a light-background terminal instead of forcing a
  fixed light-grey-on-black.
  - **The word reveal lights here too.** Pressing it (`◈`, or
    `/reveal-words`) underlines the nouns and named things on screen this story
    knows, and it was dark on every graphical v6 title for as long as it read
    the *cell* wrap cache — which raster never fills, because raster's text is
    bitmap glyphs on a canvas rather than terminal cells. It now reads the
    canvas's own wrap and applies the light as each glyph is blitted: the same
    words, from the same object tree, in the same accent ink, ruled under in the
    same geometry the game's own emphasised runs use. The rule is the point
    rather than the polish — this is host ink laid over prose the *game*
    coloured, and a foreground alone cannot promise legibility over a ground
    somebody else chose.
- **`extended`** — raster, pinned to a **whole** magnification, spending the
  height that buys on *content* rather than on empty margin: the canvas grows
  downward and the surplus becomes whole extra text rows of prose in the game's
  own bitmap face. The game is told nothing — it lays its windows out on exactly
  the screen it always had, which is now the top of a taller picture. A frame
  with nowhere to put the rows (a title card, a picture that owns the screen, a
  hint menu) declines and draws exactly as `raster` does.
  - **Anything the game anchored below its story window keeps its distance from
    the frame's bottom edge.** Arthur prints his parser errors into a boxed
    window across the last text row of the screen, shrinking the story window by
    a row to make space — so that band appears and vanishes with the turn. It
    travels down with the frame instead of shortening it, which is what the
    extension's own arithmetic leaves room for, and the message lands on the
    bottom line of the taller frame exactly as it lands on the bottom line of the
    game's screen. (It used to make the whole screen shrink back to plain raster
    size for one turn and grow again on the next command the parser understood.)
    **However tall that band is** — Arthur's wraps to two lines for a message as
    long as *"Sorry, but I don't understand. Please rephrase that, or try
    something else."* — the frame keeps its height and the artwork its size, in
    every mode, and the extra line comes out of the story text and nothing else.
    (A wrapped message used to be read as Journey's command menu, which cost
    `hybrid` its side art and `extended` its surplus height until the next
    command.) Journey itself is the real exception, and declines: its command
    menu sits under the story with frame art beside it that cannot be carried
    down past it, so that title stays on the `raster` picture.
- **Cell fallback** — without an image protocol (a remote or text-only terminal),
  while a menu or dialog is open over the story pane, or on a painted menu
  takeover, everything — graphics windows, status grids, and story text —
  composites as terminal cells instead of pixels, so the game stays playable
  everywhere. It draws no game art at all: the story runs as a normal full-pane
  terminal transcript at full size with native scrollback, and the game's
  chrome/status text collapses to compact terminal bands.
  - A dialog *has* to land here, because image placements draw **above** terminal
    cells in the classic protocols — a menu rendered as cells over a v6 image
    would simply be invisible underneath it. The command band is the deliberate
    exception: it is a dock rather than dialog chrome, and counting it as an
    overlay used to hide the story prompt and drop the whole pixel path for as
    long as it was open.
  - The pane is laid out by **relation to the story window**, never by absolute
    pixel row — because v6 games put their chrome wherever their artwork leaves
    room. Chrome text *above* the story becomes the status band and pins to the
    **top** (Zork Zero and Shogun paint theirs on native row 0; Arthur paints his
    on row 12, under a twelve-row art panel this path doesn't draw — and it still
    lands on line one, not a quarter of the way down an empty pane). Chrome text
    *below* the story becomes a command band pinned to the **bottom**, so
    Journey's verb menu stays welded to the last row at any pane height instead
    of floating over the prose. Chrome text *inside* the story box — Shogun's
    boot menu, a hint screen — paints over the transcript where the game put it.
    "Inside" means inside on **both axes**: a run merely level with the story is
    frame, not a takeover. Journey under the Amiga profile is the case that proves
    it — its border rules are line-drawing glyphs beside the story on every one of
    its rows, and a row-only test called an ordinary scene a menu screen and sent
    the whole frame down this path, where the game's eighty columns are laid out
    one per terminal column while the prose and the mouse map are still placed
    proportionally across the pane. The two agree only at eighty columns wide.
    All three are painted *after* the windows' erase fills go down, because a
    window's erase is the ground its own text is written on, not a lid over it:
    paint the band first and Adventure's status bar disappears under the very
    window that drew it. The band's height is measured up front (it decides where
    the transcript starts) and drawn at the end, with the rest of the text.
  - A graphics window sitting wholly **beside** the story is story content, not
    frame, so it keeps its column: Journey's half-screen character portrait
    renders at its native proportion with the prose inset alongside it. Art that
    spans or overlaps the story stays undrawn.
  - Clicks still work here even though no image is drawn: the pane stands for the
    game's screen, so a click maps into the game pixel at the same fraction
    across and down it.

> **Removed: `frameless`.** A third mode used to make this presentation the
> *deliberate, always-on* choice even on an image-capable terminal, trading the
> compass and border art for full-size text and native scrollback, and resizing
> inline pictures to suit (drop-caps to ~3–4 rows, band art upscaled by a crisp
> 2×/3×). It is gone as of the next release. A `config.toml` still naming it
> falls back to `hybrid` without complaint; `/set-v6-render frameless` reports an
> unknown mode. The layout above is unchanged — it was always the cell path's,
> and `frameless` was only one of four ways in.

The status and command bands on the cell path are themed by
the `upper_window` style selector (the same one that colours a v4+ status line);
a beside-the-story picture column letterboxes in the `graphics` selector's style.

## Illuminated drop-caps and room icons, inline

Window 0's own pictures — Zork Zero's illuminated drop-caps and small room
icons — aren't separate chrome; they're story content. lanthorn floats them
at the left margin of the story text and wraps the surrounding lines beside
them, so they scroll naturally with the transcript instead of sitting in a
fixed frame.

**A story picture is sized by the text it sits beside, not by the frame around
it.** Hybrid maps the game's native pixel space onto your terminal at two rates
at once: the chrome ring is artwork, magnified to fill the pane, while the prose
is terminal glyphs — one native 8×16 character cell per terminal cell, whatever
the art is doing. A drop-cap lives in the prose. Zork Zero draws its illuminated
capital four of its own text lines tall and its room icons three, and those are
the numbers that have to survive, so the float is laid out at the *text's* rate:
`ceil(width/8) × ceil(height/16)` cells, the footprint the game drew it for.

Matching the ring instead is the obvious-looking mistake, and it was ours for a
while. At a magnification of 2 the cap claimed eight terminal rows beside the
four-row paragraph it opens, and it grew further every time the window did — the
cap was not too big, everything else was too small. Raster mode never had the
problem, because there the glyphs are painted into the same native canvas as the
art and the whole finished frame is scaled once; hybrid now agrees with it, and
both agree with a real DOS press.

The tell is where the game put the picture: **on the current text line, or
somewhere it chose for itself.** A drop-cap is drawn at window 0's text cursor —
it belongs to the paragraph beside it and has to travel with it. Ask for a
picture at a row the cursor is nowhere near and you mean something else
entirely: you have placed it. (An inline float's horizontal position is a margin
choice — Shogun parks its ship at the right edge and still means "beside this
paragraph".)

**And a game may paint the float from a different window entirely.** Shogun's
Apple IIgs press (`shogun_s1.dsk`, release 311) states the Bridge exactly as its
Amiga sibling does and spells it differently: where the Amiga draws the ship into
window 0 and calls `set_margins` on the window it just drew into, the Apple gives
window 0 a 320-pixel right margin and paints the same ship from a *graphics*
window laid over the story. Read as a difference in kind, that cost the picture
its place in the text — it went onto a window canvas, where hybrid's ring (only
ever pane-minus-viewport) threw away 316 of its 348 rows and left the rest as a
three-row sliver above the prose. It is a difference in spelling. A picture a
window laid over the story paints **entirely inside the column window 0's own
margin reserved**, and nowhere else, is that window's margin picture: it floats
in the prose, it scrolls up with it, and the text reclaims the full width once it
has passed the picture's bottom edge — which is what the original does. The three
conditions are what keep the rule narrow: the painting window has to *contain*
window 0, the margin has to be in force at the time of the draw, and the picture
has to fit inside the column it gave up. Across the whole v6 corpus exactly one
picture on one press answers to all three.

A game can also say it in words, and Zork Zero does. It follows an inline draw
with `set_margins`, reserving the column its prose is about to flow in, and that
declaration counts as much as landing on the cursor — which matters because the
cursor test is pixel-exact and Zork Zero does not always hit it. Booted off its
original Amiga floppy, the game reads a tiny placement record out of the native
picture archive and nudges each drop-cap a couple of pixels in from the line; the
converted Blorb records that same placeholder as zero-sized, so the same story
lands exactly on the cursor there and two pixels off it here. The reserved margin
is the same claim either way.

**And the reserved margin is page, not chrome.** The columns a float holds back
are drawn as leading spaces on every row of prose beside the picture, and those
spaces used to inherit the transcript's base style while the prose an inch to
their right sat on the background its own text run named. Nothing showed while
the two were the same colour — which is every machine but one. Under the Amiga
interpreter the base is the machine's screen pair (§8.3, and the same pair the
pixel ring around the viewport is drawn in), whose page is dark grey, while Zork
Zero's window 0 declares a light grey page of its own; the difference turned up
as a dark stripe down the right-hand edge of every drop-cap and every room icon.
The margin now takes the ground the prose beside it sits on — its background and
nothing else, no bold, no reverse, no hyperlink — so the picture, its gutter and
the paragraph read as one sheet of paper. A paragraph that names no background of
its own copies nothing and inherits exactly as before, which is why the IBM PC
profile and both `honor_game_colours` settings render byte-for-byte what they
always did.

**So is the picture's own ground.** These are cut-out PNGs — an ornate letter and
a little room icon, mostly holes — and the image protocol keeps that transparency
and hands it to the *terminal* to resolve, which never consults the cells
underneath. So lanthorn resolves it first, against the page the picture is
standing on, and the question is only which page that is. A window that named a
background with `set_colour` has answered it; failing that, the **machine** has,
if it is one of the two whose §8.3.3 defaults are its screen rather than advice
about a terminal. Zork Zero off its Macintosh release disk never calls
`set_colour` even once, so until lanthorn asked the machine, every drop-cap and
every room icon on that disk was cut out against the theme's own dark chrome
while the page around it was the Mac's white — a black tile under each picture on
a white sheet. The Amiga's grey page answers for Arthur, Shogun and Journey the
same way. Everywhere else nothing has changed: no machine states a page, so the
theme is still the last word, and declining the game's colours declines the
machine's page with them.

There is a second question, because clearing the screen also puts the cursor
back at its top-left corner: **is there any room left beside the picture?** A
float, by definition, has prose flowing next to it. A picture that spans window
0 from edge to edge leaves no column for that prose, so it cannot be one — it is
a backdrop, and it goes on the window's own canvas with the story text drawn
over it. Frobozz Magic Videopoker paints its whole card table that way, and
Journey its title illustration; both draw at (1,1) immediately after erasing the
screen and would otherwise be mistaken for the world's largest drop-cap. The
margin there is not a fine one: the widest genuine float in the Infocom v6
catalogue — Shogun's ship — covers 58% of its window, and both of those
backdrops cover all of it.

The Mysterious Adventures are the reason there is a third question. Their title
cards are 512 pixels wide on a 640-pixel screen, so they span neither the window
nor any threshold worth arguing about, and no reading of "how wide is it?" was
ever going to place them. What settles them instead is asking what the cursor
test is actually worth on that frame: landing on the text cursor means the
picture belongs to the line being written, and at boot **nothing has been written
at all**. The cursor is simply where the screen-clear left it, so a picture that
matches it matches nothing. lanthorn now counts the characters window 0 has
streamed, and treats hitting the cursor as evidence only when there were some.
Every genuine float in the catalogue is drawn into a window that has already
printed something; every coincidence is not.

## Full-page plates — art the game placed itself

Arthur opens on three illustrated screens, and it lays each one out by hand.
It clears every window, asks window 0 how big it is, does the centring
arithmetic itself — a 584×392 plate in a 640×400 window lands at x=29, y=5 —
and draws the plate there. The Merlin screen redraws the graveyard at that same
origin and composites Merlin *inside* it, so the wizard appears on the graveyard
in a single frame rather than beneath it in a second one.

lanthorn honours that arithmetic. A window-0 picture the game placed rather than
inlined gets a real window canvas, at the pixel origin the game named, and later
draws composite into the same canvas exactly as they would on an Amiga. The
centring margin the game deliberately left around the plate stays the story
window's own page — we don't stretch art to fill space the author left empty.
Because such a screen has no frame ring at all, `hybrid` hands the frame to the
full-canvas compositor (the same path Zork Zero's map takes), which ships the
plate as one image.

**A plate is drawn *instead of* prose, not underneath it.** Arthur's illustrated
screens carry no text at all: the game erases the screen, draws the plate, hides
the cursor and waits for a key — the whole graveyard→Merlin turn is thirty-one
instructions and prints not one character. Its narration is a *separate*,
picture-less screen, erased before the next plate goes up. So when a placed plate
leaves no column wide enough to wrap prose into, the picture owns the screen and
lanthorn draws no story text on that frame — the same rule a window-filling
picture like Zork Zero's rebus already followed. A plate that *does* leave a real
column — a margin illustration, a corner logo — still gets prose beside it.

**"No room" means no room among the pixels the plate actually painted**, not
inside the rectangle it happens to span. fmvpoker draws a 640×400 poker table into
window 0 and then prints its title *inside* it — because the table is a frame with
a hollow middle, barely a sixth of it opaque. Measured by its bounding box it looked
like a plate that owned the screen and every line of the game's text disappeared;
measured by its ink, the largest clear rectangle it leaves is exactly the hole the
author meant to print in. Arthur's plates are dense enough to leave only their own
centring margin, so they still own their screens.

## What crowds the story window is *art*, not text

The story window has to be seated inside whatever the game drew around it: Zork
Zero rings it with a carved frame, Arthur hangs a graphics panel above it, Journey
puts an illustration down its left side. lanthorn finds the room by shrinking the
window's rectangle, edge by edge, until no edge touches anything opaque — and for
a long time "opaque" meant *any* pixel already on the canvas, which includes the
rasterized glyphs of the game's own menus.

That is the wrong question, because a v6 game routinely prints *over* window 0.
Shogun's title puts "You may choose to:" at the left of a four-row window 0 and
its START/RESTORE/QUIT menu into a second window sitting inside the same four
rows; on an Amiga both are simply on the screen. Measured against the menu's
glyphs, window 0's 548×64 box shrank to 548×16 — one row, which leaves no room for
a line of prose *and* the input caret, so the title showed no text at all. Journey
fared worse: the screen-wide fill that closes the bare cells of a reverse-video bar
ran straight across its 392×304 text panel, and the panel measured 392×**0**.

So the shrink is measured against the artwork alone. Everything the game printed
still reaches the screen, and now so does the prose beside it: window 0's page is
painted *under* the labels other windows put inside its box, in the order the game
drew them — page first, then the menu, then the prose.

**And the transcript's own glyphs yield to those labels as well.** Sparing the
page was only half of it: the story text was still rasterized straight over the
labels the page had carefully filled under. fmvpoker is the case. Its story window
is the whole screen, so once five dealt cards fill the frame's interior the largest
clear rectangle left for the transcript drops onto the very box the game gave its
bottom panel — and the panel is where the hand is announced, *You draw (a) an
Eight, (b) a Three, (c) an Ace…*, the only place the cards are named. The boot
banner was written across it. The rule that settles it is a difference in kind: a
transcript is lanthorn's re-reading of everything the story window has ever said,
while a label another window is holding is on the screen *right now*. Where they
land on the same cell, the live label wins, and everything the transcript owns
outside those cells still prints.

The same distinction settles how a reverse-video run is drawn. Highlighting a run
means painting a solid block and cutting the glyph out of it — except over frame
art, where a block would erase the picture, so lanthorn draws dark ink directly on
the art instead (that is how Zork Zero's ribbon labels sit on their banner). The
"is there art here?" test also used to read the live canvas, where an earlier run's
own highlight block looks exactly like artwork. advent's help screen is drawn as
one run per label plus reversed spacer spaces, and one of those spacers lands in
the middle of "About Adventure" — so the header concluded it was sitting on a
picture, dropped its block, and drew itself in the page colour on the page. The
whole navigation bar was invisible in `raster` while reading perfectly as cells.
Both tests now consult the art layer, frozen before a single glyph is stamped.

## Pictures land one after another, the way they were drawn

A v6 turn can draw more than one picture, and the order matters to how the screen
reads. Arthur's graveyard→Merlin screen is the case: **one** turn erases every
window, paints the 584×392 graveyard plate, and fourteen instructions later paints
Merlin into the middle of it. Compositing both before anything reaches the terminal
hands you the finished picture instantly — correct, and completely flat. On the
machines these games were written for you watched the graveyard fill the screen and
*then* watched Merlin arrive on top of it, because each `draw_picture` blitted as
its opcode ran.

lanthorn plays that back. The turn still runs straight through — the interpreter
never blocks, never yields mid-picture, and the composite it ends on is exactly the
one it built before — but the renderer walks the screens the turn passed through on
the way there, one per frame. The wait between them is proportional to the area
each picture painted, so a full-page plate rests for a beat you can see and a small
compass tile barely pauses at all; that is roughly what the original hardware
imposed, for the same reason.

It is not an Arthur rule and there is nothing to switch on. Any v6 turn that draws
more than one picture paces, so Zork Zero's border assembles itself at startup,
Shogun's title screen arrives in two beats, and Journey's scene art lands after the
frame it sits in. And you are never made to wait: **any keypress collapses the rest
of the sequence instantly**, landing on precisely the pixels waiting it out would
have given you. The key still does whatever you pressed it for — pacing is
decoration over a turn that already finished, so it never swallows a keystroke.

There is no Z-machine construct for any of this. Nothing in those turns busy-waits
or sleeps, and the `read_char` timers on Arthur's illustrated screens are an
auto-advance for a player who has wandered off, not an animation clock. This is a
presentation choice, made deliberately, because the games were written for
machines that painted at a visible speed.

## Prose the game positions itself

A v6 window that wraps and scrolls streams its text, and lanthorn renders that
stream as real terminal text — selectable, scrollable, reflowing to your pane.
But a game can still position that prose horizontally, and some do. Shogun's
title screen is the case: for every header line it reads its own window's width,
computes the centred column, moves the cursor there, and prints the line with no
leading spaces whatsoever. The centring lives entirely in the cursor move.

lanthorn carries that declaration into the text stream as an indent. The v6 cell
is 8 pixels wide, so the pixel column and the character column are the same
measurement — column 297 is character 37, which is exactly where a six-letter
title centres on an eighty-column screen. Every line of Shogun's header lands on
the column it asked for, and Journey's title screen, which centres itself the
same way, comes out right for the same reason. A game that never declares a
column never gains an indent.

**A second text window keeps its columns too.** A v6 game may run more than one
wrapping, scrolling window, and the one that is not the transcript keeps its own
lines rather than joining the stream. Those lines used to be recorded as plain
text with no note of where each run began, so a game that placed several runs
across one row got them back butted together: fmvpoker prints its five menu
options at pixel columns 1, 178, 372, 454 and 557 and read back as
`PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT`. Such a window now honours a
declared column the same way the stream does — with one difference that matters.
The line is padded out **to** the column, not indented **by** it: a run has to
land where the game named it, not that far past wherever the previous run
happened to end, or five labels at fixed columns drift into a ragged row. A
column already behind the line's end cannot be reached by appending and is
ignored; a line buffer only moves right. The declared *row* is honoured the same
way, with blank lines — the buffer is padded out to it, and a row already behind
its end is ignored.

The row is taken to the **nearest** text line, not the line it happens to fall
inside. A line buffer's only vertical unit is the line, and it is drawn at the
window's top plus sixteen pixels per line, so the question is which line the game
meant — and rounding down can lose a whole row of it. fmvpoker places its menu bar
and its *Continue* button at pixel row 80 of a bottom panel, which is five
sixteen-pixel lines down if you count from zero, the way the game did; taking the
line it falls *inside* gave four, drew the five labels fifteen pixels high, and
put them clear of the band the game's own mouse handler accepts for them. The
labels were visible and dead: clicking one did nothing, while clicking the blank
row beneath it played the hand.

**A keypress turn's output does not automatically open a line, either.** The
transcript puts each turn's output on a fresh line, and for a typed command that is
right: an interpreter echoes a `read` together with its terminating newline
(§7.1.1.1), so lanthorn appends what you typed to the game's `>` and lets the reply
start below. `read_char` echoes nothing at all (§10.7), so for a keypress turn that
line break is lanthorn's own invention — and whether it belongs cannot be read off
the text, because a game redrawing a menu moves its cursor and prints no newline
either way. The game's cursor is asked instead: output whose first character lands
exactly where the last output left the cursor continues that line, and everything
else opens a new one. `sunburst.z6` is what it buys — a game with no line reader
that runs `read_char` in a loop and echoes each key back, so typing `look` and
pressing Enter used to arrive as `>look` and then a lone `.` a line lower, where the
game's own screen has `>look.` on one row. Games that reposition between reprints —
the Mysterious Adventures re-asking `Resume play on a game ?`, Journey's and
fmvpoker's menu repaints — keep their line breaks, because their cursor says so.

## A message in a box takes the long way round

Some of what a v6 game says to you it does not say in the story window at all.
Ask Arthur for a hint while you are simply standing in the churchyard and it
answers *"If only you had a crystal ball...."* in a box across the bottom of the
screen; type something it cannot parse and *"I beg your pardon?"* arrives the
same way. Neither line is narration, and neither joins the scrollback — a box is
paint, sitting in its own window at its own pixels, the way a status line does.

Getting it there is a two-step the Z-machine reserves for exactly this. The game
first sends the sentence to **output stream 3**, which swallows text into a
memory table instead of printing it, and hands the interpreter a *width* to
justify against — "as if it were in the window with that number", which for
Arthur is the story window's 584 pixels. Closing the stream leaves the sentence
in the table already broken into lines, and the total width it came to in the
header, so the game can read back how tall a box it needs before it opens one.
Arthur does precisely that: it counts the lines, lays a window out that many rows
high across the foot of the screen, erases it, colours it, and only then prints
the table into it with **`print_form`**.

Both halves have to be right or you see nothing. lanthorn used to close the
stream with the plain layout — one count and the text after it — where a width
calls for the formatted one, a run of per-line records ending in a zero word; and
`print_form` was a stub that printed none of them. The visible result was a box
Arthur had carefully sized for **six** lines, because it had walked the wrong
layout as if it were the right one, and then left completely empty.

## A window the game drew a frame around is a canvas, not a page

The story window is a transcript in every Infocom v6 title: text streams into it,
lanthorn keeps the scrollback, and you can page back through it. Frobozz Magic
VideoPoker is not built that way. It draws a poker table across the whole screen,
grows its story window to the whole screen behind it, and then *positions*
everything it has to say — `HOLD` under each card you are holding, the running
totals in the panel at the bottom. Read as a transcript, all of it arrived as
narration: `HOLD` scrolled past in the story text instead of appearing under a
card, and the running totals stacked up as prose.

The tempting rule — "a run the game moved the cursor before is paint" — does not
work, and it was measured rather than argued. Arthur positions every room headline
in the story window, one character at a time, with only the first character
carrying the cursor move; Shogun and Journey centre each line of their title
headers the same way; the Mysterious Adventures re-home the cursor before every
prompt. All of them mean *resume the story here*, with the identical signal
fmvpoker uses to mean *paint this under that card*. Under that rule Arthur's
`CHURCH` came out as a painted `C` and a streamed `HURCH`.

So lanthorn asks what kind of **surface** the window is, not what a run means.
Arthur's story window is a transcript that happens to have plates drawn on it;
fmvpoker's is a picture frame that happens to have text positioned in it. The
discriminator is that the window's own art **encloses** it — painted pixels within
a text row of all four edges — while *not* filling it: a solid full-page plate is
something a game narrates over, and a frame with a hole in the middle is something
a game positions text inside. A window like that renders as what is sitting on it,
at the coordinates the game named, and carries no transcript at all — which is
exactly how a real interpreter shows it, and it is the same idea as a window
keeping the ground it painted, applied to the text on that ground. Measured across
every v6 title lanthorn is tested against, one game answers to it.

## Prose freezes where it was printed when its window moves

The Z-machine standard is blunt about it: moving or resizing a window "does not
change the current display". Text already printed is pixels, and pixels do not
follow a box around. Shogun's opening depends on it — the whole nine-line title
header is printed while window 0 *is* the screen, and then window 0 drops to a
tiny box at the bottom beside the menu and prints "You may choose to:" there. On
an Amiga the header simply stays up top; lanthorn streamed both halves into one
transcript, so the prompt came out jammed under the banner and the banner
promptly scrolled out of a four-row box.

So a scrolling window's prose is now frozen the moment its window moves out from
under it: the lines become paint, at the exact rows and columns the game printed
them at, and the transcript starts again at the window's new origin. Shogun's
title now reads the way it does on the original: the header centred across the
top, "You may choose to:" down beside START/RESTORE/QUIT.

**Frozen means frozen — the transcript lets go of it.** When the freeze takes the
whole of what a window had on screen, those characters stop being transcript at
all. They are on the glass, drawn as paint, and a transcript copy of them is a
second rendition of characters that are already there — which is exactly what went
wrong once the boot menu stopped drawing them as transcript: the credits were
drawn correctly across the top *and* replayed into the four-row prose box at
the bottom, colliding mid-line with the menu ("Copyright (c) 1988 by
InfocomQUIT the game"). It holds whichever way that screen is drawn, because
either way the frozen lines reach the glass by another route — as composited
pixels in `raster`, as a band of chrome glyphs on the ring. Pictures have had this rule since the splash art learned
it: an image the canvas already carries is marked as such, and the modes that
draw the canvas skip it rather than draw it twice. Canvas-painted text now has
the same provenance. A *partial* freeze — some lines stranded, the rest still
inside the window's new box — keeps every line, because there the frozen and the
live text are interleaved in one stream and no single boundary separates them.

**A partial freeze draws no boundary either**, and that is the same statement as
the paragraph above rather than a second rule. Saying "the live screen begins
here" is only true when the window walked away from everything it had; when it
kept most of its lines, they are still on the glass, and anchoring the transcript
past them shows you a blank page. Arthur reaches that case on an ordinary turn:
type a word he does not know and he shrinks his story window by exactly one row to
open the one-line message window at the foot of the screen, stranding the bottom
line of narration and nothing else — then prints "You don't need to use the word
'wa.'" into that new window rather than into the story. The screen went blank with
the rejection alone at the bottom (SQ-1155). Type the same unknown word twice and
only the first wipes, because by the second the window has already scrolled its
content clear of the row it is about to give up.

**Only prose the window walks away from freezes.** A window resized *around* the
text it just printed still covers it, so that text is still the window's own and
keeps streaming — which is what Arthur does on nearly every turn of play, and
what makes the difference between a faithful title screen and a transcript that
quietly stops scrolling.

**And the transcript restarts in the box the game moved the window to.** Freezing
the old half was only half the job: the live half has to land somewhere, and
somewhere is the story window's own box. In the pixel composite that was always
true — the transcript is drawn inside the window's rectangle — and it is true on
the ring, which insets a real terminal viewport at that same rectangle. The cell
presentations (a dialog over the pane, a terminal with no image protocol, and
`hybrid` on an art-less menu screen) build the pane by relation instead: the chrome above the story packs against your pane's top edge,
the chrome below packs against its bottom, and the transcript fills between. That
packing used to start the transcript flush under the band, which is right for
every game that puts its story window directly under its status bar — and wrong
for Shogun, whose window 0 sits nine rows further down, level with the menu. The
prompt came out on the line below the banner instead of beside START/RESTORE/QUIT.

Now the story window's box says where its transcript starts: the gap the game left
between its chrome and its story window carries through into the pane, and anything
painted *inside* that box — a menu's items, and the ground erased under them —
travels with it. The gap is measured against the chrome's declared rectangle rather
than the text in it, so a status panel taller than its own two lines (Zork Zero's
is 78 pixels of which two rows carry text) does not push the transcript down for
art the cell path has deliberately dropped. Nothing above the story window at all
means nothing to sit below, and the transcript keeps the top of your pane.

**A cleared screen starts at the top of its box, in every mode.** When a game
clears the screen, lanthorn pins what it prints next to the *top* of the story
window and leaves the rest blank, rather than sticking to the bottom and dragging
pre-clear history back into view — your scrollback is all still there, one scroll
up. The cell paths have always done this; `raster` now does too, which is what
keeps Shogun's four-row box showing the one line the game printed into it instead
of redrawing the tail of the banner it had just frozen up top.

**Frozen prose keeps the columns it was given.** `raster` composites the frozen
layer as pixels, so it lands exactly where the game put it. The ring keeps the
same columns by a different route — it lays the frozen block out in the game's own
text grid, one terminal column per game character, so Shogun's nine centred credit
lines come back sharing the centre the game gave them. Where a
cell path draws the screen there are no pixels to composite: text above the story window
is drawn by the anchored status-band renderer, which stretches a game's 40- or
80-column bar across whatever width your terminal is by sorting each run into a
left, centre or right field. It decides by where the run *starts*, which is how a
location name finds the left margin and a score finds the right one — and which
would tear a centred paragraph apart, since a longer line starts further left. So
a run whose margins are equal on the game's own screen is taken as deliberately
centred and is centred again in your pane, however far left it begins. A field
that starts at the screen's edge is not centred text and stays anchored where it
was.

## …and so do pictures

Same rule, one layer over. lanthorn keeps each v6 window's art on a canvas of its
own and paints that canvas wherever the window currently is, which tells the truth
right up until the game moves the window. scopa never stops moving it. Every
picture it draws goes through a scratch window it borrows for exactly one
operation — move it to the corner the card belongs in, size it to 1000×1000 so
nothing can clip, draw at (1,1), and immediately move it again for the next card
or the next fill. Its Neapolitan and Sicilian decks were being drawn into a window
that had already left, clipped to whatever sliver it had been shrunk to and then
erased outright by the following fill, so the only deck that ever reached the
opening menu was the vector one the z-code draws with fills instead of pictures.

Two things fix it, and both are the standard read literally. The engine now
records the window's **box at the moment of the call** on the picture event
itself, the same way it already records the rect an `erase_window` painted — a
scratch window's geometry is only meaningful at the instant it is used. And when a
window with art on it moves, that art is frozen onto the screen's painted ground
at the coordinates it was drawn at, exactly as prose is. Picture draws and erase
fills also drain as one ordered timeline now rather than one queue after the
other: scopa's boot fills its green table, draws two card pictures and *then* fills
the menu buttons over the top, and replaying the fills last let the opening
full-screen clear wipe cards that had already been painted.

### The ground is a screen, not a window

A v6 game erases in *screen* coordinates — `erase_window` hands over an absolute
rectangle, because the window that drew it has usually been moved and resized for
that one fill and will move again before the next. The ground it lands on was cut
to window 0's box instead, on the standard's word that window 0 opens as the whole
screen: true at boot, false from the first `window_size`, which every v6 game
issues within a keypress or two. Anything past that box was dropped without a
trace. Shogun (r322, IBM PC) paints its score bar at native (46,0) 548×32,
reaching to x=594 on a surface 548 wide, so the bar's right end simply stopped
existing 46 columns short of the flank it belongs to — one half of a symptom whose
other half was a different layer entirely, which is why fixing that layer cured one
side of the bar and left the other standing. Journey's ProDOS press allocated
304×288 for a 560×384 screen for the same reason. The ground is now allocated at
the screen the header states, which is the same extent and the same coordinate
space as the canvas the ring composites onto: 640×400 for an IBM PC press, 560×384
for an Apple IIgs one.

### A window's own ground is its page, not an obstruction of it

The ground is *paint*, and the raster path used to hand it to the probe that decides
where a story window's prose can go. That probe — `story_clear_native` — walks the
story window's own edges inward until nothing under them is opaque, so the only
pixels it can ever read are the ones **inside that same window**. Give it a ground
covering the window and it insets past all of it and reports nothing left.

Which is exactly what a game does when it erases its own story window. Macintosh
*Shogun* (release 292) leaves InvisiClues with an `erase_window` on window 0 —
548×370 at native (46, 30), the story window to the pixel — and the story box went
from the declared 548×370 to a degenerate 180×0. A degenerate interior trips the
floor that exists so a full-screen picture can own the screen, so the frame shipped
with its score bar and both ornaments and an empty page between them. Nothing short
of `restart` brought the prose back, because nothing else clears the ground.

The window *pages* had known this since they were introduced — `fill_pages_where`
skips every window overlapping the story box for precisely this reason — and the
painted ground simply had no counterpart rule. Hybrid was never affected: it measures
against the chrome art alone, and art is the only thing that question was ever asking
about. Both paths now put it to the same canvas, which is the only thing that makes
them reliably agree. A page is a colour a window was told to present on; a ground is
a rectangle the game filled. Neither is artwork, and only artwork moves the prose.

### The ground has to survive a restore too

The painted ground rides *beside* the window tree rather than inside it, and for a
while that meant it was the one v6 screen layer no restore touched. A Save State
swaps VM memory under a game that never learns it happened, so the story issues no
repaint — and `auto_load` fires only after the story has already booted and painted
its opening screen. Resuming scopa therefore came back with the main menu's cards and
buttons still on the ground, the restored hand's own text drawn over the top of them;
the model was perfectly correct underneath, so clicking where the *real* cards should
be played the right card. Shogun showed the mirror image of the same hole: it lays its
backdrop down one keypress into the boot, so a resumed Shogun arrived with no ground at
all and lost the backdrop.

The archive now carries the ground as `pictures/ground.png`, and every restore path
replaces it — including with *nothing*, when the archive has none. Pixels rather than a
recipe, which is the exception the "persist the recipe" rule allows for a derived
artifact that is genuinely authoritative: the ground's inputs are an unbounded stream
of `erase_window` fills (scopa repaints its table hundreds of times per card), which is
why it is a surface and not a list of rectangles in the first place. It is stored in
the story's own native pixels, so it stays as backend- and terminal-neutral as the rest
of the archive — a save taken on kitty at 117×64 restores unchanged onto half-blocks at
80×24.

### …and so do the ground's two siblings

The ground was not travelling alone in that gap; it was simply the one anybody
noticed. Two more layers ride beside the window tree, and neither was archived nor
reset either.

The **erase fills** are the first. The standard makes erasing a window a fill of its
rect with the window's background colour, and on a v6 screen that is opaque paint —
it is what makes advent's help panel a solid panel rather than text hovering over the
story. Journey two steps into its boot is covering three windows that way; one step
later it is covering none. Restore the later save onto the earlier screen and all
three bands used to stay, three opaque slabs over a game that had moved on; restore
it the other way and the three the save *did* carry never arrived at all.

The **canvas anchors** are the second. An anchor is what remembers where a window's
art was painted, so that when the window moves the pixels stay behind (the standard is
explicit: "subsequent movements of the window do not move what was printed"). A
restored session used to inherit the *previous* game's anchors, so the first window
move after a restore stranded the restored art at coordinates belonging to a screen
that no longer existed. Journey, Zork Zero, Shogun and fmvpoker all hold live anchors
within three steps of boot, so this was not a corner case.

Both now travel inside `display.json` — and as a **recipe**, not as pixels, because
unlike the ground they are bounded: one small struct per window however long the
session runs. Two session-local numbers are deliberately left behind, since neither
means anything in the session that reads them back. A fill's draw stamp comes from a
process-global counter, so only the *order* of the fills travels and the restore
re-stamps them from the live counter, exactly as it does for restored canvases. And a
fill's character stamp decides exactly one thing — whether any prose has printed since,
which is what stops it covering — so only the fills that still cover travel at all; the
counter never runs backwards, so a fill the story has printed past can never cover
again.

`@restart` gets the same treatment, for the same reason and by the same argument the
reboot path already makes about the canvases and the display list. A rebooted story
inherits neither the dead screen's anchors nor its ground.

### …and so does the prose that is sitting on the glass

Three more runs were missing, and this time from *inside* the window tree. A v6
window keeps its text as three layers in the same pixel space: what it has painted,
what it has **streamed** — where the prose it sent to the transcript is currently
sitting — and what a move or resize has **retired**, frozen at coordinates the window
has since walked away from ([above](#prose-freezes-where-it-was-printed-when-its-window-moves)).
Only the first of the three was in the save.

The one game that renders from the streamed layer is the one game that keeps its text
inside its own picture frame: fmvpoker's "Current Bet: 10" and "Total Winnings: 990"
live nowhere else a save was carrying, so a resumed hand came back with its legends
gone from the table. It hid for a long time because the *character* grid was archived
all along — the bet was there in cell mode and missing from the pixel composite, which
is the mode almost everybody plays in. Shogun lost the other layer: one keypress into
its boot it is holding all nine frozen title lines, and a restore used to hand them
back blank or leave the previous screen's standing over the new one.

All three layers now travel together in `screen.json`, as the game's own runs in its
own native pixels — a recipe like `texts` beside them, with no cell coordinate, font
metric or picker state anywhere in it, so one archive restores identically into an
80×24 terminal and a 200×80 one and draws the same on either graphics backend. The one
thing deliberately left behind is the per-burst *stream origin*, which only means
anything between one keypress and the next and nothing at all across a save.

## Margin pictures — text that flows past the art

Some v6 scenes put the picture on one side and let the prose flow past it.
Shogun's opening is the classic: the game draws its harbour illustration at the
**right** of window 0 and calls `set_margins` to shrink the text's right edge
back past the art (that's the Z-Machine Standard's margin-picture idiom — a
picture parked at a window edge with the margins pulled in around it). The story
text fills the narrower **left** column beside the picture, then reclaims the
full width the moment it scrolls below the art. lanthorn honours the game's own
margins: the engine records them (and snaps the cursor home on either edge, per
§15), and both `raster` and `hybrid` float the picture to its placed side —
right for Shogun, left for a drop-cap — wrapping the prose in the column the game
left for it. A picture too wide to leave a readable column falls back to a
full-width band.

## Splash art (removed: the inline echo)

Some v6 titles paint a big picture straight into a graphics window: Shogun's
320×200 title screen, Zork Zero's cutscene illustrations. `hybrid` and `raster`
draw those windows directly. The removed `frameless` mode dropped the graphics
windows that frame the story, so it would have lost the splash entirely — and to
save it, lanthorn used to recognise a *content-sized* draw and re-emit it as a
one-off inline image band in the transcript, anchored where the game drew it,
which every other mode then had to skip so the art was not drawn twice.

That echo is gone with the mode. Both surviving modes render the window canvas
itself, which is where the picture always was; nothing is lost on screen, and the
"is this image already on the screen?" bookkeeping it required — a provenance tag
on every transcript image, a skip in the raster path, a drop in the transcript,
and a per-window redraw dedupe — is gone with it.

The size classifier stayed, because it answers a second question too: whether a
window-0 picture floats inline in the prose or reserves a margin beside it.

The catch is telling a splash from decoration. lanthorn classifies a
graphics-window picture by its size against the reported screen: a picture is
**content** when it covers ≥ 40% of the screen area, or is ≥ 60% of the screen
width *and* ≥ 30% of its height; a narrow strip (≤ 15% of screen width, like
Shogun's 23-pixel side borders) or any small tile stays **frame** and is left
undrawn. On the real games this lands cleanly — Shogun's title (320×200) and
Zork Zero's full-screen cutscenes come through, while their borders, banners,
and 45×40 compass tiles do not. A repeated redraw of the same splash into the
same window is de-duplicated, so a per-turn refresh can't stamp the same
picture down the page twice; clearing the window resets that, so a genuinely
new splash shows again.

## Pixel-faithful status text and colour

Status and chrome text isn't drawn in character cells — it's drawn at the
exact pixel position the game specified, matching the source game's actual
layout instead of an approximated grid. Colour is honored too: a text run's
packed foreground/background (from `set_colour`/`set_true_colour`) resolves
to real RGB, and the reverse-video style bit swaps fg/bg — which is what
makes Zork Zero's scroll ribbons come out dark-on-tan instead of inverted.
On the pixel canvas the Standard palette colours (2–9) resolve to the
Z-Machine Standard's own recommended true-colour RGB (ZMSD §8.3.1) — so
white is real white (255,255,255) and black is black (0,0,0), rather than the
dim VGA base values the terminal ANSI palette would give (white → 170,170,170
"light grey"). The terminal *cell* path still routes Standard colours through
the theme's ANSI palette (so a user's Ghostty colours apply); only the pixel
paths take the authoritative RGB directly.
The story page itself fills with the window's own background colour (when
the game set one) rather than leaving the terminal's theme backdrop showing
through.

**A row that names a background is filled behind its runs — and bridges the gaps
between them only when it is a bar.** A status band a game paints as several
separate runs has to read as one solid strip, gaps and all: Shogun prints
`Erasmus :`, `SHOGUN` and `Score:` black-on-white across a window whose two ends
they all but touch, so the band floods that white from one window edge to the
other and the bare cells between the labels come out the same colour as the ones
under them. A row whose runs stop well short of both edges is not a band, though —
it is two labels the game happened to print on one line, and what lies between
them belongs to the window. Scopa's end-of-hand score screen is the case: it
prints its whole board into a single 640×400 grid, and `Denari` and `Primiera`
(with the two pairs of totals below them) sit either side of a green divider the
game leaves between its two blue card panels. Filling each of those rows from its
first label to its last painted three blue bridges straight through the divider.
Each run's own cells are filled; the table between them stays the table.

Every *other* window's page follows the same rule, because ZMSD §8.8.3.2 gives
each Version 6 window its own pair — not one page shared by the screen. It
matters most where the art is mostly holes: Zork Zero's compass and room icons
are line art on a clear ground (95% transparent) and hang below the banner
artwork, so the pixels behind them are pixels nobody painted. Left transparent,
the graphics protocol picks the colour, and it picks black. lanthorn paints each
chrome window's own page into its untouched pixels instead, so the ring it
uploads is self-contained. Only `alpha == 0` pixels are filled — artwork, status
bands, glyphs and the icons' own ink are untouched, the story box stays clear for
the terminal transcript, and a window the game gave no colour keeps the host
page. It is the whole screen's look for Scopa, whose green baize is a window
background and nothing else.

**A window the game has drawn into keeps its page even when you decline game
colours.** That exception exists because Scopa's baize is not a preference at all:
read from the screen ops, it sizes window 1 to the whole 640×400 screen, names an
explicit true-colour green and issues `erase_window` — the same fill opcode that
draws its cards. A fill spanning the entire screen is treated as a screen clear
rather than as paint (otherwise every game that merely erases would gain a
backdrop it never asked for), so the window's background is the only surviving
record of that drawing. Gating that record on `honor_game_colours` while leaving
the smaller fills ungated split one picture in half: turn game colours off and you
got a *black* table carrying the green bands and the cards the game had drawn onto
it. The discriminator is the painted ground — a window with the game's own pixels
inside it is a canvas and keeps its page either way; a window with none is
presentation, and your theme still owns it. The story window is never in scope:
its page and ink are the surface prose is read on, and those are exactly what the
setting is for. Nothing else in the v6 corpus paints a ground at all, so Zork
Zero, Arthur, Shogun, Journey and Adventure are untouched.

**The story page fills UNDER the game's own fills.** Window 0's page is the oldest
thing in its box — the game filled the window, then everything else was drawn on
top — so the page yields both to the labels other windows print inside that box
and to any rectangle the game itself painted with `erase_window`. fmvpoker is why
the second half matters. It draws its poker table with Zork Zero's picture file
(the original release ships that file renamed to `FMVPOKER.EG1`), so the frame's
top-centre tab natively reads *Double Fanucci* — and the game hides that title the
way a v6 game does, by parking a window over the banner and erasing it to the
colour it declared for that window. It never prints a title of its own there; the
banner is erased, not overwritten. With window 0 covering the entire 640×400
screen, a page fill that ignored the erase repainted the tab in window 0's white
and the frame appeared to have its top cut off — an artefact of the fill order, not
of the artwork, which is neither clipped nor mis-placed.

This colour honoring now spans *every* v6 presentation, not just the pixel
raster: the cell path's classic status band, the painted menu/hint overlays,
the hybrid story-strip overlay, and the plain cell fallback all resolve a
run's game colours the same way. The rule is the shared one every engine
follows — a channel the game explicitly set (a real palette entry or a true
colour) wins; a "current"/"default" sentinel is inheritance, so the theme
keeps that channel — and it's gated on `honor_game_colours` like the rest.
A game that sets no colours (Shogun) is untouched: its runs stay theme-styled
in every mode.

## Adaptive palettes: overlays that borrow their colours

Some of Zork Zero's pictures — the compass rose overlays, the little scene
tiles — don't carry a real palette of their own. They ship with a placeholder
(a stock 16-colour table, close to the EGA card's but not it — see above) and are
flagged in the Blorb's `APal` chunk as
*adaptive*: the interpreter is meant to draw them with the "Current Palette"
established by the last ordinary picture it plotted (Blorb spec §11.3). Zork
Zero leans on this hard — it paints a base illustration to set the mood's
colours, then stamps adaptive overlays on top expecting them to inherit that
mood. Decode each one with its own placeholder instead and the compass comes
out in garish primary EGA, clashing with everything around it.

lanthorn now tracks that Current Palette as it draws, and when an adaptive
picture comes up it splices the current colours into the picture before
decoding — keeping the overlay's own transparency intact, so the arrow still
cuts a clean hole in the rose. Because the *same* overlay can legally be drawn
under different base palettes as a game moves between scenes, the decoded result
is cached per palette, not just per picture, so a palette change re-tints it
rather than serving a stale copy. All the v6 render paths — ring, raster, cell
fallback — share this decode path, so the fix lands everywhere at once.

The original Amiga archive has no `APal` chunk, because it never needed one: it
writes a plain **zero** where each picture's palette would go, and a picture with
no palette can only be drawn through the one that is current. That is the same
statement, made per picture rather than in a list — and for *Zork Zero* it marks
exactly the same 172 pictures the Blorb's `APal` names, id for id, which is
another sign of where those Blorbs came from. So native artwork goes through the
machinery above rather than beside it: one Current Palette, in the same colours a
Blorb `PLTE` holds, tracked the same way and carried into a save the same way.
The check is Infocom's own: their converter pre-computed every
(illustration, overlay) pairing and shipped the results inside the Blorb, and the
Amiga archive reproduces 36980 of those 37152 answers exactly. The remainder are
all one illustration — picture 8, one of the five the Blorb replaced — where the
floppy is the source that is right.

## Arrow keys: movement or map panning, your call

Several v6 titles bind the arrow keys straight to movement — press ↑ and your
character walks north. That's authentic, but it collides with lanthorn's own use
of arrows for scrollback recall and map panning, and a v6 game was the one place
in lanthorn where ↑ stopped scrolling the transcript. That reads less like a
setting doing its job than like the app going deaf, so arrows are **withheld by
default** — but only at the `>` prompt, where the movement-vs-panning clash
actually happens. There, instead of being delivered as a ZSCII cursor code, the
keypress falls through to whatever lanthorn would do with it if no game input
were pending — command-history recall or map panning, depending on focus.

Want a game's own arrow bindings? Set `v6_arrow_keys = true` in the config, or
flip it right in the settings screen. The trade isn't symmetric, which is why the
default falls where it does: withholding costs you a shortcut for a movement
command you can still type, while forwarding costs a key that works everywhere
else.

Menus are the deliberate exception. Whenever a v6 story is waiting on a single
keypress rather than a line — Shogun's startup menu, hint menus, a "press any
key" pause — arrows always reach the game, setting or no setting, because those
screens are unnavigable without them. So the rule is simply: arrows drive
lanthorn at the prompt, and drive the game everywhere else.

Enter and every other key are untouched, and v1–v5/Glulx stories keep getting
arrows regardless of this setting; it only ever withholds them from a v6 prompt.

## Click the compass, walk the map

A click inside the game image is mapped back through the letterbox to the game
pixel it landed on and delivered the way the original interpreters did it: the
coordinates go into the header extension table and the click terminates the
pending read (ZSCII 254, §3.8) — at a `>` prompt too, when the story asks for
click terminators, which is exactly how Zork Zero's banner compass works. Click
a spoke and you walk.

The automapper comes along for the ride. A click types nothing, so there is no
command to parse a direction from — but the game echoes the command it
synthesized (`north`, alone on the first output line), and lanthorn adopts that
echo as the turn's movement command. A compass-clicked move draws the same
directional edge on the map, and records the direction as tried, as if you had
typed it.

## Artwork you stop looking at is handed back

An image sent to a kitty terminal stays there until something says otherwise.
Placing a new one over it does not free it; closing the window it belonged to does
not free it; clearing the screen does not free it. Only an explicit delete does,
and lanthorn now sends one for every picture it walks away from — a chrome band
whose art changed, a band the ring no longer draws, and the full-pane raster
composite, which is the largest single thing the app ever uploads (2.8 MB of
Journey's opening screen, at a 117×64 terminal).

This is not tidiness. Kitty terminals evict by least-recently-used when their image
memory fills, and they will happily evict a picture that is *currently on screen* —
so a long session that keeps sending art and never takes any back can blank the very
frame you are looking at. Journey's first five keypresses used to upload 4.1 MB and
free none of it; a game like scopa, whose whole screen is one image, stranded a
fresh copy of it on every move. Now each one is handed back in the same breath as
its replacement goes out, and `band uploads since launch` counts against a terminal
that is no longer quietly accumulating everything it was ever shown.

Order matters more than it looks. A picture being *replaced* is the one your
terminal is drawing at that moment, and its replacement can be most of a megabyte
away — Zork Zero's banner used to be 618 KB every time a compass arrow changed.
Free it first and those cells have nothing to draw until the new upload lands, which
reads on screen as a flicker: the compass blinking as it composites, the on-screen
map blinking as it updates its corner, Arthur's graveyard blinking as Merlin appears
in it. So a picture nothing is showing any more is freed immediately, and a picture
being replaced in place is freed *after* its replacement is on screen. Same frame,
same batch, nothing held longer than the width of one placement.

## A band is cut into tiles, so a small change is a small upload

That 618 KB is worth staring at. Zork Zero's banner is one 920×126 image, and a
compass arrow is about 45×40 pixels of it — a third of a percent. Every band already
hashes only its own native footprint, so a change under *another* band never disturbs
it; but a change under *this* band re-encoded and re-transmitted all 151 chunks of it.
Eight arrows over the boot animation, and 4.9 MB had gone down the wire to redraw a
compass.

There is no way to ask a terminal to patch pixels into a picture it already has.
Kitty cannot, iterm2 cannot, and building a patch-over-base layer on virtual
placements only trades the bandwidth for bookkeeping and drift. What you *can* do is
send a **smaller picture**, so the ring's full-width bands now go up as a row of
8-column tiles rather than one strip: fifteen images across Zork Zero's banner
instead of one, and a compass arrow re-sends the one or two it lands in.

The tiles are the same pixels. Every band crops its rectangle out of one scaled
canvas the frame builds once, at whole device pixels, so column 41 reads exactly the
same source however the band around it is cut — no resampling boundary at a seam, no
ceil-versus-round trap, and the first frame's transmitted payload is byte for byte
what it always was (618,240 bytes: fourteen tiles of 43,008 and one of 16,128). The
partition is exact by construction — no gap that would leave a column of the ring
unwritten, no overlap that would put two images on one cell.

Eight columns is arithmetic rather than taste. Kitty takes 4096 base64 characters per
chunk, so every tile rounds its last chunk up and wastes about 2 KB; cut finer and
that fixed cost eats the win (115 one-column tiles would add 230 KB to every first
frame, and leave a terminal that evicts by LRU juggling 115 resident images per band);
cut coarser and the re-send climbs straight back. Measured on the real binary under a
pty at 117×64, the same three frames on either side:

| frame | one strip | 15 tiles | |
|---|---:|---:|---|
| first frame | 2,089,630 B | 2,093,195 B | +0.17% |
| compass, one tile | 629,280 B | 43,947 B | **14.3×** |
| compass, two tiles | 628,566 B | 88,349 B | **7.1×** |
| whole three-key boot | 4,604,778 B | 2,358,042 B | **1.95×** |

Granularity is per backend, because the trade is not the same everywhere. Kitty and
iterm2 tile. **Sixel does not** — every sixel image carries its own palette
definition, so fifteen tiles would mean fifteen palettes where the strip had one,
which is a real first-frame regression bought for a redraw win. Half-blocks does not
either, and does not need to: it draws glyphs, and ratatui's own cell diff has always
sent it just the cells that changed. Side flanks are left whole as well — they are
tall and thin, and column tiles would buy them nothing.

None of this is visible. With the flicker fixed, this is purely how much goes down
the wire; on a local terminal you would never notice, and over ssh it is the
difference between a boot animation that feels snappy and one that does not.

## Graphics-window uploads are compressed

The kitty graphics protocol accepts a zlib-deflated payload (`o=z`); the
compression happens before base64, `f=32` still names the format the terminal
finds after inflating, and `s`/`v` still name the *uncompressed* image's pixel
dimensions. Sixteen-colour artwork is what deflate is best at, so this is not a
marginal saving. Measured on real canvases at level 6:

| canvas | raw base64 | compressed | |
|---|---:|---:|---|
| Zork Zero r393 window 7, 640×400 | 1,365,336 B | 6,580 B | **207×** |
| Shogun r322 window 7, 640×400 | 1,365,336 B | 6,532 B | **209×** |
| Journey r83 window 3, 232×304 | 376,152 B | 10,884 B | **34.6×** |

Level 6 rather than 1 or 9: level 1 leaves three to five times as much on the
wire to save about a millisecond, and level 9 buys 5–8% more for two to four
times level 6's cost — and on one canvas it came out *larger*. The 1.4–3.3 ms
this spends lands on the render worker, which is nothing beside a megabyte of
base64.

**That path is graphics *windows*** — Glulx's clickable toolbars, Scott room
pictures, and any v6 graphics window drawn as an image rather than as cells.
Measured on advent.blb under a pty, the whole capture went from ~314 KB to 54 KB.
Compressing the image is only half that path's bill, though — see *A graphics
window's image id never moves* below for the other half, which was larger.

**And it asks the terminal first, on both paths** (SQ-0997). This one did not,
for a while: SQ-0976 taught it `o=z` before `Capability::KittyCompression`
existed, so it stated `o=z` whatever the probe said — and on a terminal that
speaks kitty graphics but cannot inflate, that is not a slow upload but an absent
one. The transmission is refused, the image is never stored, and every
placeholder cell naming it draws nothing: no error, no fallback, just windows
with no pictures in them, while the chrome ring beside them (which *did* ask)
drew perfectly. Both encoders now read the same answer, and an empty capability
list means raw — see the paragraph on what *cannot* ask, below.

The v6 chrome ring's bands and the full-pane raster composite go through
`ratatui-image`, which is a layer down and was the larger prize — the ring alone
was emitting more than the windows ever did. It compresses too now (SQ-0991),
and the crate **asks the terminal first** rather than assuming:

| capture (117×64, under a pty) | before | after | |
|---|---:|---:|---|
| Journey r30 hybrid, kitty payload | 3,431,392 B | 67,735 B | **50.7×** |
| Zork Zero r393 raster, kitty payload | 14,151,821 B | 159,630 B | **88.7×** |
| one 920×575 composite frame | 2,821,336 B | 32,452 B | 86.9× in 5.5 ms |

The asking matters more than the ratio. `Capability::KittyCompression` rides the
capability query already sent at startup, as one extra probe using the protocol's
own query action (`a=q`) — sixteen base64 characters, no extra round trip, the
same shape `RectangularOps` already had. Compression happens only when that probe
came back `OK`.

Everything that *cannot* ask — `Picker::halfblocks()`, `from_fontsize`, the
default picker returned when the query gets no answer, tmux without passthrough,
the WezTerm/Konsole blacklist — leaves the capability absent and therefore
transmits raw. That is the safe direction and worth stating plainly: an
uncompressed image is merely slow, while an image the terminal cannot inflate is
**invisible**, because the transmit fails and every placement naming it draws
nothing. Our own placement oracle demonstrated exactly that when it met `o=z`
without a zlib decoder linked in.

A capability is also something you can *lose* without noticing. lanthorn
re-measures the terminal's cell size on every resize (SQ-0988), because a cell is
two roundings at different rates and changing font size mid-session leaves the
composite fitted with an aspect ratio up to ~29% wrong. That refresh used to
build a *replacement* picker around the new cell with `from_fontsize` and copy
the protocol across — which preserved the protocol and nothing else, so the
capability list went with the discarded picker and a queried kitty session
dropped back to raw transmits until the app was relaunched. It fails safe, which
is exactly why it went unseen. The refresh now hands the new cell size to the
picker it already has (`Picker::set_font_size`, added to the fork for it) and
touches nothing else (SQ-0992).

## A graphics window's image id never moves, so a changed picture costs the picture

Compressing the payload only helps if the payload is what you are paying for. On
a kitty terminal a graphics window is drawn as a **virtual placement**: the canvas
is transmitted once, and every cell of the window's rect gets a `U+10EEEE`
placeholder that names the image — its id's low 24 bits as the cell's foreground
colour, the high byte as a third combining diacritic. The cells are ordinary
buffer cells, so they go through ratatui's diff like any other content, and a
frame whose picture did not change emits nothing at all. That is the design
working.

**The id is a per-cell value, and that is the trap.** Change the id and every one
of those cells differs, so the diff emits all of them. Until SQ-0995 a new id was
allocated whenever a canvas-content hash missed, which meant one changed pixel
repainted the whole grid — and, worse, so did a canvas the terminal *already had*,
because re-placing a cached upload swaps the id back. Measured under a pty:

| capture, one frame in which the picture changed | before | after | |
|---|---:|---:|---|
| golden_baton.blb at 117×50, a 115×16-cell room picture | 21,867 B | 3,271 B | **6.7×** |
| golden_baton.blb at 230×64, a 228×16-cell room picture | 42,207 B | 3,723 B | **11.3×** |
| waxworks.blb at 160×50, a 158×16-cell room picture | 29,419 B | 3,251 B | **9.0×** |
| golden_baton at 230×64, whole session over three moves | 129,461 B | 55,229 B | 2.3× |

The compressed image in the 230×64 row is 2,208 bytes. Everything else in the
42,207 was placeholder cells being repainted for no reason.

The fix is to hold the id still and replace the data behind it. The protocol
licenses this directly — *"When re-transmitting image data for a specific id, the
existing image and all its placements must be deleted"* — and our transmit is
`a=T,U=1,r,c`, which re-creates the window's placement in the same command that
replaces its data, so the cells never stop resolving. Two details make it safe:

- **The transmit now names its placement (`p=1`).** `p=0` means "assign me an
  internal placement id", and Ghostty's storage replaces the *image* on a
  re-transmit while leaving placements alone — so an unnamed placement would leave
  one dead duplicate behind per frame of animation. A named one is replaced in
  place. The placeholder cells still encode placement 0, which resolves to "the
  first virtual placement of this image" and therefore to the only one.
- **Nothing blanks mid-transfer.** A chunked transmit commits only on its final
  chunk, so the old picture stays on screen for the whole of the new one's
  journey.

What this replaced is the SQ-0564 upload cache: eight canvases per window kept in
the terminal, so a game flipping between a resting and a pressed toolbar re-placed
an id instead of re-uploading it. That cache cannot coexist with a stable id — an
id you might place is an id in every cell — and the measurement above is why it
does not deserve to. It saved the *image* on a flip-back and paid the whole grid
in cells to do it: the golden_baton frame that returned to a room it had already
drawn transmitted zero bytes of picture and still cost 39,859 bytes. A window now
holds exactly one image in the terminal however long it animates, which is a
tighter bound than the cap ever gave, with no eviction to get wrong.

The half of SQ-0564 that survives is the content hash itself: a game that repaints
its window from scratch onto identical pixels (advent.blb's toolbar does this to
release a button) still transmits nothing.

Resize is the one thing that still churns the id, and must: the placement's `r×c`
grid is baked into the transmission, so an upload at the old size can never be
re-placed at the new one. It is deleted in the same batch as its replacement, as
it has been since SQ-0637.

## …and neither does a chrome band's, nor the raster composite's

The section above is about *graphics windows* — Glulx toolbars, Scott room
pictures, the odd v6 window drawn as an image. **The v6 pane is not drawn through
that emitter at all.** Journey r83, Zork Zero r393, Shogun r322 and Arthur r74
emit no ids from lanthorn's own range: their art is a chrome ring of bands and,
in raster mode, one full-pane composite, and both go through `ratatui-image`. So
the whole win above landed one layer away from the pictures most players are
looking at.

The crate has the same defect, from the other direction. It draws a fresh
`rand::random()` id for every `Protocol` it builds, and lanthorn builds a new one
on every content change — so a band that changed by one pixel, and a composite
that changed by one pixel, each repainted their entire placeholder rect. A
composite covers the pane: 3,680 cells at 117×64.

The fix is the same fix, and it needed one addition to the fork:
`Picker::new_protocol_with_id`, which is `new_protocol` with the kitty id handed
in instead of drawn. Lanthorn passes back the id the band or composite is
*already placed as* — read off the placement it last wrote, which is also what
makes it `None` under half-blocks and sixel, where there are no ids and none are
wanted. The crate's own transmit now names its placement (`p=1`) for the same
reason lanthorn's does, and for one more: `StatefulKitty::resize_encode` already
re-transmitted to a live id on every resize, so the duplicate-placement pile-up
was reachable in the crate without lanthorn's help.

Measured under a pty at 117×64, comparing frames by their image payload so the
two runs are describing the same frame:

| frame (Journey r83, raster mode) | before | after | |
|---|---:|---:|---|
| composite, 7,668 B image | 48,742 B / 3,680 cells | 7,806 B / **1 cell** | 6.2× |
| composite, 25,452 B image | 62,911 B / 3,680 cells | 24,622 B / **1 cell** | 2.6× |
| composite, 42,084 B image | 83,274 B / 3,680 cells | 42,339 B / **1 cell** | 2.0× |

| frame (Zork Zero r393, hybrid ring) | before | after | |
|---|---:|---:|---|
| one 64×126 band, 588 B image | 1,346 B / 56 cells | 729 B / **1 cell** | 1.85× |
| one 64×126 band, 604 B image | 1,419 B / 56 cells | 745 B / **1 cell** | 1.90× |
| two bands, 600 + 592 B images | 3,272 B / 116 cells | 1,580 B / **4 cells** | 2.07× |

Every changed frame now costs its picture and a single cell. The floor is the
image, which is what it should be.

**One frame shape does not improve, and it is worth knowing why.** Journey r83's
39×20-cell illustration band went 27,751 B → 26,943 B on the frames where the
picture changes, and still emitted all 780 placeholders. The id holds still; the
*background* underneath does not. Those cells carry a background colour taken from
the game's window, Journey changes it when it changes scene, and a cell's style is
part of ratatui's cell equality — so the rect is dirty for a reason that has
nothing to do with the id, on exactly the frames the art moves. The colour is
invisible (the image covers it) and its repaint is pure cost; not painting a
ground into a cell a placement is about to cover is a separate change, and not
this one.

SQ-0637 is untouched in both paths: a band evicted from the ring, a ring
invalidated wholesale, and a composite abandoned at the raster→ring transition are
all still deleted in the terminal, and all come back under a **new** id. They have
to — the `a=d` for the old one is queued and rides out on whichever placement goes
next, which could be after a transmit that revived it.

## Half-blocks pays by the cell, and no palette can help it

Compression is a kitty story. Half-blocks emits no image at all — it emits
*cells*, one `▀` per cell with a truecolor foreground and background — so `o=z`
does nothing for it and its bill is written entirely in SGR. Measured under a pty
with `--image-protocol halfblocks`, four fifths of everything lanthorn writes is
colour changes:

| capture | full repaint | SGR share | distinct colours |
|---|---:|---:|---:|
| Zork Zero r393, 200×100 cells at 4×9 px | 180 KB | 84.7% | 1,083 |
| Zork Zero r393, 458×144 cells at 4×9 px | 489 KB | 81.7% | 1,419 |
| Zork Zero r393, 700×220 cells at 4×9 px | 936 KB | 78.3% | 1,712 |
| Journey r83, 117×64 cells at 8×18 px | 150 KB | 82.8% | 4,746 |

`ESC[38;2;R;G;B;48;2;R;G;Bm` is about thirty bytes; the indexed form
`ESC[38;5;N;48;5;Nm` is about eighteen, and OSC 4 can program a terminal palette
entry to an exact RGB — so an obvious idea is to index the colour instead of
spelling it. Infocom artwork is famously sixteen colours: a decoded picture is one
palette index per pixel through a `[Rgb; 16]`, and 640×400 Amiga hires is four
bitplanes, which is where the ceiling comes from. Sixteen entries, sixteen colours,
done.

**Except the artwork's colours are not what goes on the wire.** Half-blocks
resolves one sample per column and two per row, so the composite is resampled onto
that sample grid on the way out, and a *shrinking* resample averages neighbours
into colours that were never in the picture. Aspect is preserved, so both axes
always travel together and the whole thing turns on one comparison — the sample
grid against the canvas:

| Zork Zero picture 41 (15 colours) on its 640×400 screen | sample grid | colours emitted |
|---|---|---:|
| 117×64 pane | 117×74 | 665 |
| 200×60 pane | 192×120 | 614 |
| 458×144 pane | 458×288 | 1,165 |
| **640×200 pane** | **640×400** | **15** |

Shogun's richest illustration reaches 15,539. The grid stops shrinking only at a
**640-column terminal**, and only there does an indexed encoding of the artwork
become exact — which is not a width anybody has. Mapping the real numbers into the
standard 256-colour cube instead costs a mean RGB error of 21–26 with 5–6% of
emissions landing exactly: visible posterisation, on precisely the dithered art
where the saving would have been concentrated. So neither half was built, and
`v6_halfblocks_colour_depth` is the measurement kept executable so the idea can be
re-opened on evidence rather than on arithmetic.

## `/dump-windows` reports the last frame the *game* drew

When a v6 layout looks wrong, `/dump-windows` is how you say what you saw: one
block per window, merging the game's own window table, the model lanthorn built
from it, and where the renderer actually put each one on the terminal — the three
things that have to agree.

There is a catch built into asking. Reach the command through the command palette
or a hotkey dialog and you are opening a modal overlay, which routes the v6 pane
off its pixel path — so the frame *most recently drawn* when the dump runs is the
palette's, in which every one of the game's windows is honestly reported as
`NOT DRAWN this frame`. That is the one thing nobody opened the dump to learn. So
lanthorn keeps the mapping from each frame the **game** drew, and the dump
describes that one: its render path, its pane, its story viewport, its per-window
cells and chrome strips, and the ring's own plan and clip for that frame. A
`frame described:` line says which frame it is and how many modal frames have
gone by since. The game-side halves — the window table and the model — are read
live, because a modal overlay runs no game code and they still describe the frame
being reported. If no frame has ever been drawn without an overlay up, the
placements are reported as `UNAVAILABLE` rather than quietly swapped for the
overlay's.

Better still, don't open a modal at all. Reporting the right frame stops the dump
*lying*; it does not stop the palette **churning** the very numbers the dump
prints. Opening it costs a run of `cell — modal overlay open: palette` entries in
the render-path history, and coming back out invalidates every cached chrome band
so they all re-upload — visibly moving `band uploads since launch` and pushing the
frame of interest further into the past. Bind the command to a key instead and the
capture reads the counters without touching them:

```toml
[keymap.global]
"ctrl+d" = "dump-windows"
```

Ctrl rather than a bare key on purpose: while a v6 story waits on a single
keypress — Journey's menus, any "press any key" — plain keys go to the game, and
`F9` would answer the prompt instead of dumping. The Ctrl binding fires from map
focus too.

Under the window blocks the **band list** names every image the ring placed on that
frame: its cell rect, where it was placed, whether it re-encoded or came out of the
cache, and — the part that matters when a picture turns up somewhere it shouldn't —
the **native crop** it is showing, so you can read which rows of the game's own
screen an image is painting. A flank's picture is drawn at a rect the panel derives
rather than at the strip's, and it used to be missing from this list entirely: two
investigations of Journey's picture column reasoned about it from the strip beside
it, because the one band they wanted to see was the one band the dump could not
name. A flank's two border columns say which **medium** they came out in — a
`flank-divider (glyph '│' style=00)` line is a rule stamped in the terminal's font,
carrying no crop because there is nothing to crop, while a plain `flank-divider`
with a native crop beside it is still a bitmap. "The frame's sides are a picture of
a character" is a sentence this dump can now say in one line.

The dump also lands in **`~/.lanthorn/dump-windows.log`**, appended, with a
timestamp per capture, and the transcript line names the path. Selecting the
on-screen copy off a v6 pane drags the graphics protocol's own placeholder glyphs
along with it — the diagnostic corrupted by the thing it is diagnosing — and the
file is the same text with nothing composited over it. Read it from a second
terminal while the game is still running, take several captures across a turn,
and paste any of them intact.

## `/dump-cells` writes the rendered screen — glyphs *and* colours — as plain text

`/dump-windows` answers *where did each window land*. It cannot answer the
question a v6 layout defect nearly always turns out to be: **which colour landed
in which cell**. A panel fill painting rows underneath a menu, a border cell
wearing the fill's colour instead of the frame's, a label the cell buffer holds
and the screen does not — geometry shows none of the three, and each one used to
cost a round trip through a screenshot.

`/dump-cells` writes the frame itself. Two lines per terminal row: the **glyph**
row, so borders and labels read as text, and directly under it the **style** row,
one key character per cell indexing a legend of the distinct styles. No ANSI
escapes anywhere — the whole point is text you can copy, paste and diff.

```
 52 g|│──────────────────────────The P──────────────────────────────────Individual Comm──│
 52 s|ccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
```

Above the grid, three summaries do the counting for you:

- **`graphics:`** — every region an uploaded image covers, by rect. Those cells
  read `#` in the glyph row, because an image draws *over* the terminal's text
  layer and whatever character is underneath is not on screen. Their **style** row
  survives, though: placing an image does not touch the colours of the cells it
  covers, which is exactly how a fill painted beneath an art strip stays visible.
  Placements the renderer recorded are listed beside the rects recovered from the
  buffer, because halfblock and sixel backends paint without leaving escape cells
  behind at all.
- **`row backgrounds:`** — the rows every cell of which shares one background,
  as ranges. "These nine rows all carry the panel fill's colour" is one line here
  instead of a count done by eye off a screenshot.
- **`styles:`** — the distinct styles, commonest first, each with its exact
  foreground, background, attributes, cell count and bounding box, plus the rows
  it owns end to end. A picture rendered *into* cells can run to hundreds of
  styles (one per pixel pair); past the first 48 the tail is bucketed under `*`
  with its own count and extent, so the legend never buries the dozen that matter.

The whole capture goes to **`~/.lanthorn/dump-cells.log`**, appended and
timestamped, and the transcript line names the path — only the path, because the
grid is two lines per row and echoing it would scroll the very frame your next
capture is meant to describe. Like the window dump, it lands in a file because a
selection dragged off a v6 pane brings the graphics protocol's placeholder glyphs
with it.

It describes the last frame drawn with **no modal over it**, for a reason sharper
than the window dump's: a modal is painted straight onto the cells, so a capture
taken through the palette would not report a stale frame — it would report the
palette's box sitting where the game's picture was. Bind it to a Ctrl key and no
modal ever opens:

```toml
[keymap.global]
"ctrl+g" = "dump-cells"
```

A bound-key capture moves neither the render-path history nor the band-upload
count; the palette route moves both.

## `/dump-terminal` separates what lanthorn *measured* from what it *guessed*

The other two dumps describe the frame. This one describes the **terminal it is
being drawn on**, and its organising principle is a distinction nothing else in
the app makes: several numbers the whole graphics path is computed from are
guesses that look exactly like measurements.

The cell size is the one that bites. `ratatui-image` asks the terminal with
`CSI 16 t`; when nobody answers it falls back to the tty's `TIOCGWINSZ` pixel
geometry, and when that answers nothing either it uses a hardcoded 10x20 the crate
itself calls "completely arbitrary" — and on Windows there is no ioctl to fall
back to at all. `cell 10x20` and `cell 10x20 (ASSUMED)` mean entirely different
things, and every device box downstream is derived from whichever one you have.
So the report names the source outright, and colours a guess differently from a
measurement:

```
  cell size: 9x20 px — DERIVED (from the tty's TIOCGWINSZ pixel geometry)
  cell aspect: 2.222 (height/width), +0.222 from the 2.000 that makes a half-block sample square
    CSI 16 t answered: 8x18 px
    TIOCGWINSZ says now: 9x20 px
```

That capture was taken after a font-size change, which is exactly when the two
disagree: the cell in force is the ioctl's, the `CSI 16 t` answer is the one the
terminal gave at launch, and calling the first "measured" would be the conflation
this command exists to end. The **signed** distance from 2.000 is there because
the sign says which of Ghostty's `adjust-cell-height` / `adjust-cell-width` to
reach for — real cells swing 1.75 to 2.25 even for a face whose design ratio is
exactly 2.002, because a cell is `round(advance·px)` by `round(line·px)` and the
two round at different rates.

An empty capability list gets the same treatment, because it is three different
facts: the terminal was asked and refused, `--image-protocol halfblocks` built a
picker that asks nothing, or `--images off` built no picker at all. And the
question the command was written for:

```
  kitty transmission compression (o=z):
    ratatui-image uploads (v6 chrome bands, the raster composite, cover + inline art): COMPRESSED — the terminal answered the o=z probe
    graphics-window uploads (lanthorn's own transmit — Glulx toolbars, Scott room pictures, v6 graphics windows): COMPRESSED — the same o=z probe governs both encoders
```

Two lines because there are two encoders, and only one of them asks. Compression
fails silently in both directions — a terminal that cannot inflate simply draws
nothing, and a quiet reversion to raw looks like nothing at all — which is why
having it stated anywhere at all is the point.

Then the render state and the byte counts that explain each other:

```
render
  story pane: 115x61 cells = 7,015 cell(s)
  v6 mode: hybrid
  picture takeover: none — the hybrid ring drew the last frame
  native screen: 640x400 game pixels, art_scale 2x2
  magnification: 1.617x, pixel lock off (free scaling)
  recent render paths (oldest first): raster x2 · hybrid-ring x6
traffic
  bytes written to the terminal: 210,012 in 10 frame flush(es) since launch
  last drawn frame: 44,014 bytes
  graphics ops on the last recorded frame: 1 upload(s), 0 reuse(s), 1 placement(s), 0 drop(s)
  placeholder cells under those placements: 840
  chrome-band / composite uploads since launch: 3
```

Those numbers cost the frame path nothing. The bytes are counted at the writer —
one add per `write`, one per `flush`, never looking at a byte — and every other
figure is read from something the render already tracked for its own reasons. The
graphics-op counts come from the render's own log rather than from the wire for
the same reason: finding `\x1b_G` in the stream would mean scanning every byte of
every frame. Anything that would have needed a new counter on the frame path is
reported as **unavailable**, with the reason, rather than instrumented into
existence.

Like its two siblings the report is appended to a file —
**`~/.lanthorn/dump-terminal.log`**, timestamped, path named in the transcript —
and this is the one you actually want in a bug report. It also takes a Ctrl
binding, and for a sharper reason than the others: reaching it through the palette
is itself traffic, so bytes-per-frame taken that way describes the palette's frame.

```toml
[keymap.global]
"ctrl+t" = "dump-terminal"
```

## Your own boot media, your machine's own typeface

Neither machine Infocom wrote a Version 6 interpreter for kept its body face on
a game disk. The Macintosh kept Geneva in the System file; the Amiga kept topaz
in Kickstart ROM. Both are recoverable, and both are yours to supply.

Infocom's Macintosh games ship one face, and it isn't the one the machine drew
with. `FONT` 524 is **Monaco 12** — the fixed-pitch alternate `mac/xzip.lst`
selects as `ZMONO` — while the body text it declares is `stdFont := geneva`, and
Geneva lives in the System file that came with every Macintosh and no game. Zoom
in on `machine-screenshots/mac-zorkzero-game.png` and both faces are on one
screen: the status bar's `Banquet Hall` steps a metronome 7 pixels a character,
and the prose two lines below advances 7, 7, 5 through `n`, `o`, `t`. Nothing on
a game disk can draw that second line.

So lanthorn asks you. Drop a **Mac OS System startup disk** or an **Amiga
Kickstart ROM** into `~/.lanthorn/` — an `.img`, a `.rom`, any image the mounter
already reads — and a Version 6 game off that
machine's own media is drawn with the face the machine really used. Nothing is
shipped, nothing is copied, nothing is licensed: the media stay yours, exactly the
arrangement `stories/` has always run on, and a player with none there sees the
built-in face answering as it always did.

The order is one sentence, and it lives in one function:

1. **the release's own face**, off the story's own medium — Arthur's Amiga
   `char.data`, the Macintosh's `FONT` 524;
2. **the machine's system face**, off a boot medium you supplied;
3. **the built-in**, which is what CI and an empty `~/.lanthorn/` get.

And the built-in is **two faces, picked by the cell**. Uni-VGA is drawn for an
8-pixel advance — 76 of its 94 printable glyphs ink out to column 6, so column 7
*is* their inter-character gap — and a Macintosh cell is 7 wide, which drops it.
Measured over all 52×52 ordered pairs of ASCII letters blitted into adjacent 7×15
cells, 1649 pairs came out touching their neighbour. So a 7-wide cell with no disk
face behind it — a bare `.z6` under `--interpreter 3`, and every CI run, since disk
fonts live on media that cannot be committed — is drawn with **X11 misc-fixed 7×14**
instead, public domain and 7 pixels wide by construction: 19 pairs touch, every one
of them a `T`, the only glyph in that face which inks its last column. Every other
machine declares an 8-wide cell and nothing about it moves; font 3 still comes off
the 8×8 masters, because a character-graphics set has to tile edge to edge and no
text face does.

On the Macintosh those first two rungs answer at once and land in *different*
jobs, which is the whole point: Geneva 12 off your System disk becomes the body
face, and the game's own Monaco keeps the fixed-pitch role it was drawn for. A
story asks for that role two ways and means one thing — `@set_text_style 8`, or
`@set_font 4` — and *Zork Zero* uses the second, bracketing its entire status bar
in `@set_font 4` / `@set_font 1` without ever touching the style word. So the bar
keeps its columns while the prose beside it steps Geneva's own advances, which is
what the capture shows and what no single face can do.

What the story is **told** does not move an inch. `mac/xzip.lst` declares
`colWidth := 7; lineHeight := 15`, Geneva 12 is fifteen rows tall, and the
declared cell comes out 7×15 either way — the machine's grid is exactly the grid
it always was, and only the drawing changed. That is not luck: a Macintosh paints
text at one native pixel per face pixel however dense its artwork is, so the
colour press can double `CPic.data` onto the unit screen without doubling
Geneva's line into thirty rows. (`zvm::interpreter::V6FaceSpace` is where that
rule lives; the Amiga is the row that answers the other way.)

The **size** is the machine's too, not a guess. A System disk carries a whole
family — Geneva at 9, 10, 12, 14, 18, 20 and 24 point on a System 6.0.8 startup
disk — and the declared line height says which one was painted. Fifteen rows,
so Geneva 12. Ask for a family the disk doesn't carry, or a size the machine
never declared a line for, and the cascade falls through rather than
improvising.

Several disks **compose**. Keep Workbench 1.2 and 1.3 side by side and both are
read; the faces pool, and the request — family, drawer, size — picks out of the
pool rather than one disk winning. When two disks carry the same face and you
care which answers (a System 7 Geneva is not the 1988 one), name it:

```toml
system_font_disk = "1.3"      # any case-insensitive piece of the filename
                              # ("Kick" would promote your Kickstart ROM)
```

It only breaks a tie. A file named there that doesn't carry the face being asked
for falls through to the others, because a naming preference must never lose you
a face; with no preference the pool is ordered by filename, which is stable and
visible. The picker's info panel lists every face it found, grouped by the medium
it came off, so you can see what a disk is worth before a game is even launched.

### The Amiga's face is in the ROM

The Amiga's half went the same way and for a while drew nothing, which was honest
rather than broken — and honestly wrong. A Workbench floppy's `fonts/topaz/11` is
fixed-pitch at 8×11 against that machine's 8×16 cell, so it is neither the cell
nor a typeface and the fitness test declines it. The seven display faces beside it
(`ruby`, `garnet`, `emerald`…) are *not* candidates either: the machine names
`topaz` and only `topaz`, which is what stops an Amiga game being quietly drawn in
Ruby. The trouble is that the topaz the interpreter actually painted with is
**topaz 8**, and topaz 8 is on no floppy Commodore ever shipped. It is in
**Kickstart**.

So point lanthorn at a Kickstart dump — `Kick12.rom`, `kick13.rom`, whatever you
call it, as long as it ends in `.rom` — and *Shogun* and *Zork Zero* on the Amiga
draw in the face the machine drew in. Zoom into
`machine-screenshots/amiga-shogun-game.png` over `Erasmus` in "This is the bridge
of the Erasmus" and the measurement is right there: ten distinct scanlines across
a twenty-row line, so every face row is drawn **twice**, and sixty pixels of
underline across seven characters, so the pen steps **eight**. An 8×8 face at a
text scale of (1, 2) — which is exactly the 8×16 cell the machine declares, and
exactly the Amiga's 640×200 hires mode, where a text pixel is 1:1 across and a
square-pixel screen doubles the 200 rows.

That (1, 2) is the interesting part, because *Arthur*'s own `char.data` on the
same machine wants (2, 2): it is authored in the game's 320-wide picture space and
doubles with the artwork, which is why its ten face rows become a twenty-row
declared line. **Two faces on one machine wanting two different scales**, so which
space a face is drawn in follows where the face came from rather than which
machine is drawing it — a release's face from the release, the system's face from
the system. Arthur keeps its own face regardless: rung 1 is the release's medium
and a ROM is rung 2.

Nothing is pinned to a Kickstart revision. The image is identified by its length
(256 KiB maps at `$FC0000`, 512 KiB at `$F80000` — every Kickstart ends at
`$1000000`, so the base is just that minus the size) and by the `JMP` every
Kickstart opens with; then the whole image is swept for `TextFont`-shaped records
that name themselves `<something>.font`. On Kickstart 1.2 that finds exactly two,
`topaz/8` and `topaz/9`, with no false positives, and it is the same parser and
the same name rule that read a font out of a `FONTS:` drawer — so the machine's
"topaz, and the size whose line matches my cell" picks topaz 8 without a single
rule written twice.

And a file in `~/.lanthorn/` is whatever you put there. A truncated image, a
header claiming an enormous glyph, a ROM with a pointer into hyperspace, a file
only pretending to be a volume — every one of them is refused quietly, without a
panic, an unbounded allocation, or a game that won't start.

### And `/dump-windows` says which face won

All of that resolves once, at launch, out of sight — so when a line breaks in the
wrong place the first question is whether the wrong face was admitted or the right
one was measured wrong, and until now the dump could not tell you. It reports the
answer above the windows, once, because the face is a launch fact and every window
shares it:

```
  face: one launch fact — every window below is set in it
    body: 10x10px from the release's own medium · fit Metric
    fixed: none — a fixed-pitch run takes the body face
    declared cell 8x20px · text scale 2x2 native px per face px
    pen: proportional 4–18px over printable ASCII · bold +2px (smear 1)
```

That is *Arthur* off its Amiga floppy. Point it at the Macintosh compilation
volume with a System disk in `~/.lanthorn/` and the same five lines name two
different faces off two different media — `body: 15x15px from
MacOS_6.0.8_System_Startup.img · FONT 396`, `fixed: 7x15px from the release's own
medium` — which is the split described above, stated rather than inferred. With no
face at all the body line says so and names the built-in.

The three sizes are on one line on purpose, because they are the three that get
confused for one another: what the face *is*, what the story was *told*, and the
scale between them. A Macintosh colour press doubles its artwork and not its text,
so `2x2` on that line would be a real defect wearing a plausible number.

Each window then reports the font properties the *game* can read back — §8.8.3.2's
properties 12 and 13 — against that declared cell, so a window a story re-sized for
itself stands out from seven that never touched theirs.

And the declared cell is a launch fact that outlives a reboot. `@restart` reloads
dynamic memory to the story's pristine image and builds a fresh window model, both
of which carry the story's own idea of the cell rather than the host's — so the
reboot re-states the metric it was launched with, and hands the screen back in the
pixels it was reported in rather than reconstituting them from the character grid
(which is lossy on any cell that does not divide the screen: a Macintosh 640×400
comes back 637×390 on a 7×15 cell). Without the first, Arthur's Amiga floppy laid
its score bar out on a 16-row line after a restart where its launch declared 20,
and hybrid — which draws a strip with glyphs only while the game's own runs explain
it — rasterized the bar (SQ-1156).

## Not yet there
- **Proportional fonts, one machine so far.** Arthur's Amiga floppy carries a
  real proportional typeface, and lanthorn draws it at the face's own per-glyph
  advances instead of a fixed cell — the only Version 6 release that does,
  since *Journey*, *Beyond Zork* and *Shogun*'s Amiga releases ship a fixed 8×8
  graphics set instead of a typeface and keep the old fixed-cell path
  untouched. The engine measures with the same pen the renderer draws with:
  the cursor advances by the glyph, a line breaks at the window's real pixel
  width, and header `$30` — the width a game reads back after measuring a
  string through output stream 3 — reports what the machine would have drawn.
  That is what puts Arthur's date field ten pixels inside its own score bar
  and wraps the F5 crystal-ball description exactly where a real Amiga wraps
  it, instead of thirty pixels past the bar's end and a word and a half late
  (SQ-1009). The half-block and kitty backends take the same break rather
  than re-wrapping to the story's declared column count, so a line reads the
  same in hybrid as in raster and its right edge is honestly ragged; a run
  carries the grid cell it was written at, and the cell and the pixel are
  measured in one pass so they cannot disagree about which word ends a line
  or which blank a wrap swallowed.
- **Save State across v6** — the host Save State snapshot captures the
  underlying machine as it does for any Z-machine game; carrying the v6-
  specific render state (window geometry, floats, pictures) across a restore
  so the chrome comes back pixel-identical isn't verified yet. Standard
  in-game `@save`/`@restore` follows the normal Z-machine path (see
  [the persistence model](persistence.md)).
