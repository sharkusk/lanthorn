# Interpreter (Z-machine, Glulx & Scott Adams)

> For players, the short version is in [the guide](../guide/graphics-and-terminals.md).

[← back to README](../../README.md)

Point lanthorn at a story and it works out the format from the file itself and
boots the right engine — you never choose. Under the hood are three from-scratch,
zero-dependency virtual machines written clean-room in Rust: a Z-machine
(`zvm`), a Glulx engine (`gvm`), and a Scott Adams / ScottFree engine (`scott`).
All three converge on one neutral screen model, so the host features below —
sound, colour, timed input, crash-proofing — light up no matter which you're
playing.

- **Z-machine** (`zvm`) — the Infocom canon and decades of Inform 6, in story-file
  versions **v3/v4/v5/v6/v7/v8**, including graphical **v6** — verified in depth
  against *Zork Zero*, whose pictures and text composite together on
  image-capable terminals, with the same engine targeting the wider v6
  catalogue (*Shogun*, *Journey*, *Arthur*). See [Graphical v6](v6-graphics.md)
  for how. (v1/v2 are not supported.)
- **Glulx** (`gvm`) — modern Inform 7, with a complete **Glk 0.7.6** layer verified
  against the standard Glulx/Glk test suites, an accelerated Inform veneer, and the
  full floating-point opcode set. It targets Glulx spec 3.1.3 and reports every
  capability it does and doesn't have honestly through `gestalt`.
- **Scott Adams** (ScottFree `.dat`) — the classic 8-bit text adventures
  (*Adventureland*, *Pirate Adventure*, …), played through the same TUI and live
  automap as everything else. Room illustrations render when the game ships as a
  Blorb with PNG artwork (drawn by the graphics pipeline); lanthorn plays the
  `.dat` text engine and shows the bundled images — it does **not** decode the
  original SAGA line-draw graphics format.

## What counts as a story file

Point lanthorn at whatever the game arrived in and it digs the story out itself.

- **Bare images** — `.z3`–`.z8`, `.ulx`, and Scott Adams `.dat` are read straight.
- **Blorb containers** — `.zblorb`/`.gblorb`/`.blorb`/`.blb` yield their executable
  chunk, and the same file's `Pict`/`Snd `/`Data` resources become the game's art
  and audio. A resources-only Blorb sitting *beside* the story counts too.
- **ZIP archives** — a zip is opened like a volume, not like a wrapper around one
  file. Entries are classified by their **content**, not by their names, so a zip
  carries anything lanthorn runs — v3–v8 including graphical v6, Glulx, Scott
  Adams, Blorb containers — and a resources Blorb packed in the same zip supplies
  that game's pictures and sounds, as does a hints file. (It used to name three
  extensions and hand back no resources at all, which meant the one format whose
  whole point is that it ships artwork was the one a zip could not carry.) Still
  honest about its limit: a zip holding *two* games plays the first one. And a zip
  is a convenience for what somebody downloaded — the `.lanthorn` archive is the
  container, and the two stay apart.
- **A URL** — anywhere a path is accepted. lanthorn fetches it, hands the file to
  the same loader, so every format above works without a second code path, and
  then offers to keep it in your library so the next launch finds it without
  fetching again.
- **Amiga `.adf` disk images** — the original release floppy, played as it shipped.
- **Macintosh disk images** — a DiskCopy 4.2 `.image` (or a bare HFS volume), the
  Mac release floppy, likewise played as it shipped.
- **Hybrid CD-ROMs** — `.bin`, the raw disc dump, no `.cue` wanted. *Classic Text
  Adventure Masterpieces of Infocom* pressed one disc for two machines, and its
  Macintosh half is an Apple partition three layers down: hand lanthorn the
  354 MB dump and it measures the sector framing, walks the partition map and
  mounts the volume, offering all 83 games on it.
- **DOS floppy images** — `.ima`, `.img`, or any name at all: the PC release disk,
  from a single-game 360 KB floppy to a *Lost Treasures* collection.
- **Atari ST floppy images** — `.st`, the GEMDOS press, which turns out to be the
  same filesystem one machine over.
- **Apple II ProDOS disk images** — `.2mg`, the 800 KB 3.5" press: single-game
  Apple IIgs disks and the seven-volume *Lost Treasures of Infocom* collection.
  And `.dsk`, the 5.25" press, which is the same filesystem with its sectors in
  the order the drive numbers them rather than the order ProDOS does: hand
  lanthorn any one of *Shogun*'s five floppies, *Journey*'s five or *Zork Zero*'s
  four and the whole game opens, because a release is not a platter. And `.po`,
  the same 800 KB volume with no wrapper at all.
- **Apple II raw self-booting disks** — also `.dsk`, also 143,360 bytes, and not
  a filesystem at all. Infocom's earlier retail floppies boot their own loader
  and read the story off known tracks with their own RWTS: no ProDOS volume
  directory in any sector order, no DOS 3.3 VTOC, nothing to enumerate. lanthorn
  finds the game by putting the sectors into DOS 3.3 *logical* order and then
  looking for a run of them that verifies against the story's own header
  checksum — which is an oracle a wrong guess cannot pass. `stories/`'s
  *Planetfall* retail disk is release 29, serial 840118, 426 sectors starting at
  track 3.
- **Commodore 1541 disk images** — `.d64`, 174,848 bytes of 35-track floppy, and
  the first medium here whose game is on **two of them**. Commodore DOS is on all
  three disks in `stories/` and used by none: *Trinity* writes its story straight
  over its own directory sector, and *Hitchhiker's* keeps a decorative directory
  whose only file is a BASIC loader. So the story is raw sectors again — and laid
  out differently by different presses, so lanthorn tries each layout and keeps
  the one that verifies against the story's own checksum.

Those last seven are worth their own paragraphs. Infocom's Amiga releases came on 880 KB
floppies, and the disk images those turned into are still how the graphical
titles circulate in their native form. Hand lanthorn one — `lanthorn "Zork
Zero_Disk1.adf"` — and it mounts the AmigaDOS filesystem (both OFS and FFS),
walks it, and plays what it finds. No unpacking step, no loose files, nothing to
rename.

AmigaOS has no filename extensions to go by, and while Infocom's convention was
`Story.data` beside `Pic.data`, the convention is not a promise — one Zork Zero
disk lists a file in its own manifest that was never written to it. So lanthorn
identifies the story by **content**: a Z-machine header whose version, memory
map, serial, and declared length all agree with the bytes actually present. The
two saved games sitting on the Zork Zero disk look superficially like v3 stories
and are rejected on exactly those grounds. Conventional names only break a tie if
a disk somehow offers two candidates; a disk with none — the plain AmigaDOS boot
floppy that ships as Disk 0 — says so instead of booting a system library.

The artwork comes along for free: a native Infocom picture archive on the *same*
image is that story's art, because a shared floppy is as strong a guarantee of
pairing as a Blorb is. Loose archives are a different matter — lanthorn will use
one, but only if you name it, and it never guesses from a filename. See
[Choosing which artwork a game draws](v6-graphics.md#choosing-which-artwork-a-game-draws).

The Macintosh floppy is the same story one filesystem over, and a good deal more
work: a DiskCopy 4.2 image is an 84-byte header wrapped around an HFS volume,
with 12 bytes of sector tag per block trailing behind that are *not* part of the
filesystem. Inside is a B\*-tree catalog — the most structure any medium here
asks for — and lanthorn walks it, extents overflow file and all. macOS is no help
whatsoever: `hdiutil attach` has refused HFS-standard images since 10.14, so
every layer of that chain is hand-rolled, with the same zero dependencies the
rest of the container reading takes.

The same reader opens a **CD**, because a CD is that volume in two more
containers rather than a new filesystem. A raw disc dump keeps each sector's
whole 2352-byte frame, so the 2048 bytes of user data have to be gathered out of
it — and lanthorn *measures* that frame rather than assuming it, by finding the
sync pattern at the front and the next one after it, which reads a 2448-byte
subchannel dump with nothing taught about the number and reads a cooked `.iso`
by noticing there is no sync at all. Inside is an Apple Partition Map, and on a
hybrid disc it names three: the map itself, the ISO9660 side the PC release
lives on, and `Apple_HFS`, which is the Macintosh volume. Two signatures the
mount never looked for while deriving any of that confirm it — Apple's `ER` at
logical block 0 and ISO9660's `CD001` at logical sector 16 — and both land.

That disc taught the reader one other thing, which was a bug rather than a
feature: **a volume need not fill its container.** The *Masterpieces* partition
is sized for the whole disc and claims 634.8 MB of allocation blocks on a disc
whose entire payload is 308 MB, because it shares the platter with the ISO9660
half and its free tail was simply never written. Every block any file actually
uses is there. lanthorn used to compare that nominal size against the image and
decline the volume outright, which meant the disc — and the partition, even when
you extracted it by hand — read as "Z-machine version 0 is not supported". The
bound is now on what a reader *follows*: the catalogue, the extents overflow
file, and each file's own extents must be present, and a missing tail nobody
follows costs nothing. A genuinely truncated volume is still refused, and refused
harder — a file whose extents run off the end yields no story rather than the
front half of one.

And the same content-first rule decides what to run, because the `.image`
extension means nothing in particular and the Mac disk carries a story, an
application, the Finder's desktop database and **two** picture archives — one for
the colour screen and one for the black-and-white one. lanthorn draws the colour
archive; the monochrome one packs its directory differently and is not yet
decoded.

The PC and the Atari ST are one paragraph, not two, and that is the interesting
part. GEMDOS put its BIOS Parameter Block at **exactly** the DOS offsets — bytes
per sector at `0x0B`, sectors per cluster at `0x0D`, and so on down the block —
so a plain FAT12 reader opens an Atari compilation with no Atari-specific code in
it whatsoever. What differs is the machine, and the machine is a question for the
boot sector: DOS's own load protocol requires the sector to begin with an x86
jump over the BPB (`EB xx 90`, or `E9 xx xx`), because the BIOS executes it from
offset 0. TOS has no such rule. Across all twenty-four floppies in the reference
collection the test is unanimous — fifteen DOS images open with that jump under
four *different* OEM strings, and nine Atari ones open with `00 00 4E` or three
zeros and an OEM field that is blank. Nothing else was usable: the extension is
worthless (`.ima` and `.img` are one format), the OEM string names a formatter
rather than a machine, and the `55 AA` at the end of the sector is a boot
signature on one machine and a checksum word on the other.

These disks push the content-first rule harder than any other medium, because
their filenames give up entirely. Every story on an Atari ST compilation is
called `STORY.DAT` — four of them, on four different games — so the *directory*
is what names the game, and lanthorn lists them as `HITCHHIK/STORY.DAT` and
`BUREAUCR.ACY/STORY.DAT`. Subdirectories are not optional here: a root-only walk
would find nothing at all on that disk, and would miss the `DEMO` folder on the
standalone DOS *Hitchhiker's*. Beside the games sit somebody's 1996 saved
positions (`BILL1.SAV`, `STEVE1.SAV`) and a pile of `.COM`, `.EXE`, `.PRG` and
`.SYS` files, and the header check throws out every one of them. One more piece
of Infocom trivia falls out for free: `ZORK0.ZIP` is **not** a PKZIP archive but
Infocom's DOS name for a bare Z-machine story — byte-identical to the loose
`zork0.z6` — so it needs no unwrapping and never did.

The one thing a PC disk cannot do is be a whole release by itself. *Zork Zero*'s
story lives on *Lost Treasures* floppy 5 with its EGA artwork, while its CGA
artwork is one disk over on floppy 4; the standalone 360 KB release spreads
installer, story and EGA art across three floppies. lanthorn mounts **one image**
and offers what that image holds, so pick the disk with the game on it. Joining
several disks into one release is a set model that does not exist yet.

The Apple II arrives wrapped. A `.2mg` is a 64-byte little-endian header bolted
onto an 800 KB ProDOS volume, and every image in the reference collection carries
a small trap in that header: the field that says how long the data is reads
**zero**. That is a known quirk of the tool that wrote them — CiderPress signs
its images `WOOF` — so lanthorn takes the declared length when there is one, the
block count when there is not, and the tail of the file only as a last resort,
and insists in every case that what it lands on is a whole number of 512-byte
blocks that are actually present. A bare ProDOS volume with no wrapper reads the
same way.

Underneath, ProDOS is the tidiest filesystem here and the one that nests deepest.
Files come in four shapes and lanthorn reads all of them: a *seedling* is a
single block, a *sapling* points at an index block of 256 pointers, a *tree*
points at a master index of index blocks, and a GS/OS *extended* file keeps a
mini-entry per fork in an extended key block — of which lanthorn reads the data
fork, exactly as it does on the Macintosh. Holes are real: a zero pointer means a
block ProDOS never allocated, and it reads back as 512 zero bytes rather than as
an error. Directories nest two deep on the GS/OS disks, so files are named by
path — `SYSTEM/SYSTEM.SETUP/TOOL.SETUP` — which the launcher volume insists on,
since it carries three different files called `FINDER.DATA`.

Two of these disks are worth knowing about before you open them. *Arthur* and
*Journey* on the Apple II are the ProDOS **8** press, and they do not contain a
story file at all: the game is split across `ARTHUR.D1`–`D5` and
`JOURNEY.D1`–`D4`, none of which begins with a Z-machine header. lanthorn reads
them anyway — see [The packed Apple volume](#the-packed-apple-volume) below,
where the 5.25" presses of *Shogun* and *Zork Zero* do the same thing across
five and four separate floppies. And
*Lost Treasures* volume 1 is the GS/OS launcher — fifty-three
files of system software and not one game. Volumes 2–7 carry thirty games
between them, and since no ProDOS release uses a conventional story name, opening
one of those disks gives you the largest game on it while the picker and
`--story` offer the whole list.

The thirtieth of those games took an extra quest to find. Deciding what is a
story means reading a Z-machine header, and one of the things a header carries is
a six-character serial — `871214`, or `------` on some builds — which is a fine
sanity check right up until you meet a disk written on a machine that sets the
high bit on every character it stores. `LEATHRGODDESSES` on volume `INFOCOM6` is
a perfectly good Version 3 story whose serial reads `C2 EC EF F7 EE A1`; take bit
7 off and it spells **`Blown!`**, somebody's joke, not damage. lanthorn now masks
that bit before it judges a serial, so *Leather Goddesses of Phobos* is on the
list where it always belonged — and the check keeps doing the job it was there
for, because what it is really guarding against is the saved games sitting beside
the games on these disks, whose serial field is binary rather than text either
way.

### The packed Apple volume

*Arthur* and *Journey* on the Apple II are the awkward case, and they are awkward
for an interesting reason: the ProDOS volume they sit on is not where the game
lives. Those `.D1`–`.D5` segments are not chapters, not chunks, not a split
archive. They are a **second container with its own block space**, and the story
is a paging image scattered across all of them — page 34 can be on the fourth
floppy while page 35 is back on the first. Infocom did that because a 5.25" disk
holds 140 KB and *Arthur* is 265 KB, and because an interpreter that pages by
block does not care where a block is as long as something tells it.

Something does. Block 0 of the first segment is an index: how many floppies the
release was pressed on, and then, per floppy, a list of runs saying "story pages
*first* through *last* live here, starting at block *n*". Put the runs in page
order and the game comes back. The runs tile the story exactly — every page named
once, no gaps — which is also the check that keeps a `.D1` full of something else
from being mistaken for one.

Reassembly is not something to be confident about by eye, though: the pages are
opaque, and a wrong map produces a file that looks every bit as much like Z-code
as a right one. So lanthorn does not trust its own work. It assembles, then
checks the story's own header checksum — the sum of every byte from `$40` to the
declared length, which Infocom put there for exactly this kind of doubt — and
hands back nothing that fails. *Arthur* release 63, serial 890622, 271,304 bytes,
checksum `$45EB`: that is a game, and you can play it.

`Journey.2mg` is the one that cannot be. Its index declares five segments and
that image carries four, so ninety-two of its five hundred and fifty-two pages
are not on the image at all. Nothing is wrong with the reader and nothing is
wrong with the disk's ProDOS filesystem; the pressing is simply incomplete. The
honest answer to four fifths of a game is no game, so lanthorn mounts the volume,
lists its files and declines to offer a story — the same answer it gave before,
now for a reason it can state.

It can state one more thing about it, though. Page 0 of the story is on
`JOURNEY.D1`, which the image *does* have, and a Z-machine header is where a build
writes its name: release 77, serial 890616. So this disk knows what it is even
though it cannot be played, which is enough to keep the release 83 `Journey.blb`
next to it from drawing another build's pictures into it — see
[v6 graphics](v6-graphics.md).

And release 77 *is* playable, off two other pressings of it that are complete:
the five-floppy `journey_s1.dsk`…`s5` set and the consolidated 3.5" `Journey.po`.
Both reassemble to 282,176 bytes, checksum `$B136`, and both draw the release's
own 135 pictures. That is what turns "this image is short" from a suspicion into
a measurement — the same build, off media that have every segment, behaves.

`Journey.po` is also the reason `.po` is a spelling the library scan knows. A
`.po` is usually a **bare** ProDOS volume with nothing wrapped round it, which this
reader has always been able to open; until these images arrived nothing in the
reference collection was one, so the extension was in no format's list and the
disks were openable by name and invisible in the story picker. Three of the four
here are bare volumes and mount (`Arthur.po`, `Journey.po`, `ZorkZero.po`).

The fourth, `Shogun.po`, wears the same extension over a **DiskCopy 4.2** image,
and it used to be declined on exactly the grounds this page keeps repeating —
recognition is by content, never by extension. That was right about the name and
wrong about the file: the bytes are readable, and refusing them cost the Apple
*Shogun* press for two quests. The wrapper states its own geometry and the
arithmetic closes to the byte — `dataSize` 819,200, `tagSize` 19,200, and
84 + 819,200 + 19,200 = 838,484, the file's length — so an ordinary 800 KB
`SHOGUN` volume is sitting 84 bytes in, with a volume directory header at offset
1108 as textbook as `Journey.po`'s at 1024. SQ-0889 mounts it, and not through a
new decoder: the unwrap is the **Macintosh** reader's, shared rather than
rewritten, because DiskCopy is a wrapper and not a filesystem. Each reader runs
its own volume sniff 84 bytes in and declines what is not its own, so a Macintosh
DiskCopy image is unwrapped by the ProDOS reader just as willingly and then
turned away — the same way a DOS 3.3 floppy is de-interleaved and then turned
away. What comes out is the segmented Apple II press on one disk instead of five:
`SHOGUN.D1`…`D5` beside `INFOCOM.SYSTEM`, reassembling to **release 311, serial
890510, checksum `$E200`** — the same build the five-floppy set gives, which is
the outside evidence that the unwrap landed on the right bytes and not merely on
well-formed ones.

#### …and the same container, one floppy per disk

*Shogun* and *Zork Zero* shipped the same way and did not get the convenience of
a single image. Their Apple II press is five and four **separate 5.25" floppies**,
and the packed volume above spans all of them: `SHOGUN.D1` is alone on the first
disk, `SHOGUN.D2` on the second, and the pages interleave across the lot. Open
one of those files by itself and the honest answer is the *Journey* answer — a
mounted ProDOS volume called `SHOGUN.1`, four files on it, and no game.

So opening a disk had to stop meaning opening a file. Name any volume of the set
and lanthorn finds the rest the way the picker already grouped them — by their
filenames, in one directory, without opening anything — and asks the container
the question spanning them. The same header checksum settles it, so a pile of
floppies that are not one release is refused rather than spliced: *Shogun*
release 311, serial 890510, 344,224 bytes, checksum `$E200`, and *Zork Zero*
release 383, serial 890602, 299,392 bytes, checksum `$6F7F`. Both are builds no
other medium in the collection carries — a fifth *Shogun* and a fourth *Zork
Zero*.

In the picker they are two games and not nine disks: every volume reports the
same reassembled build, and the fold that already existed for multi-disk
collections keeps the first one and drops the rest.

The artwork on those disks was a separate piece of work and it is done. It was in
there all along — four picture archives on *Arthur*, in the space the story pages
leave free at the end of each segment, with the familiar Infocom header and a
directory of 140×192 and 62×72 pictures at the Apple II's own hi-res dimensions.
The Apple wrote a directory record eight bytes wide where the Amiga, Macintosh and
PC wrote twelve, fourteen or sixteen, and it packs pixels the way Apple hi-res
packs them, which is like nothing else in the crate; the same segment index that
says where the story pages are says where each archive begins. lanthorn reads all
of it now, and every Apple II release here draws its own plates — *Arthur*'s 168,
*Journey*'s 135, *Shogun*'s 55, *Zork Zero*'s 496. That art comes with a screen of
its own, 560×384 rather than the 640×400 a Version 6 story gets when nothing
declares a picture space: see [v6 graphics](v6-graphics.md#apple-ii-artwork).

Disk images are first-class in the library too: point lanthorn at a directory of
them and the picker's TYPE column names the container alongside the format —
`Z6 (ADF)` off an Amiga disk, `Z6 (HFS)` off a Macintosh one, `Z6 (DOS)` off a PC
floppy, `Z3 (ST)` off an Atari one and `Z5 (ProDOS)` off an Apple II disk — from the same content-based
identification, so a floppy is never listed as a bare story file, and one
machine's media is never labelled as another's. See
[Story picker](interface.md#story-picker).

### One road in, whatever the disk is

Two filesystems this far apart could easily have grown two of everything, and for
a while they did: the "is this a disk, and what is on it" question was written
out three separate times — once for artwork, once for story loading, once in
`zvm-cli` — and the third copy had never learned about Macintosh disks at all. So
the command-line player mounted an Amiga floppy happily and refused a Mac one a
month after lanthorn had learned to read it. Nobody wrote that rule; it was what
you get when three places each answer the same question separately.

There is one road now. A single table inside `blorb` lists the formats, and
everything that touches a disk — the picker, story loading, artwork, the CLI's
menu, the interpreter number the medium implies — asks that table rather than
naming a filesystem. **Whatever lanthorn can recognise as a disk, it can open**,
because recognising and opening are the same lookup. A format added to the table
arrives everywhere at once, and DOS and the Atari ST proved it: they landed as
two rows and one reader, and the picker, the CLI menu and the launch dialog all
gained them without a line changed. Apple ProDOS then landed on exactly those
terms — one row, one new reader, and not a line of the picker, the launch dialog
or the command-line player touched.

The packed Apple volume above landed on gentler terms still — no row at all,
because it is not a disk format. It is a container that happens to live on one,
so it plugged into the seam's "every story on the volume" answer and *Arthur*
appeared in the picker, in the launch dialog and in `zvm-cli`'s menu without any
of them being told a thing.

Apple II 5.25" `.dsk` media was going to be the exception — the one format a row
could not express, because a `.dsk` is one 140 KB floppy and *Shogun* is five of
them. It turned out to need no row at all, and for a better reason than expected.
Look at one of those images the way ProDOS does, with its sectors put back in
block order, and it is simply **a ProDOS volume**: block 2 is an ordinary volume
directory calling itself `SHOGUN.1`, and the file on it is `SHOGUN.D1`. The
5.25" press is the 3.5" press one de-interleave away, so it wears the same row,
uses the same reader and announces the same Apple IIgs. `.dsk` became a spelling
that row claims, and the picker, the launch dialog and the CLI's menu gained it
with nothing edited anywhere.

And then a `.dsk` turned up that really did need a row. *Planetfall*'s retail
disk is the same 143,360 bytes in the same sector order, and there is no
filesystem under it in any order — no ProDOS volume directory, no DOS 3.3 VTOC,
just Infocom's own loader and 426 sectors of Z-code it reads with its own RWTS.
Nothing comes out the other side of a de-interleave here the way a ProDOS volume
does, so this one is a format of its own, and it is the only one lanthorn reads
whose bytes are not a volume at all. What finds the game is the **story's own
header checksum**: put the sectors into DOS 3.3 logical order, walk every sector
boundary, and take the run that verifies. Under the two wrong orders the same
disk yields a story that is right about its version, its release and its serial
and wrong about its checksum — `$529D` and `$97D5` against the `$842E` the header
declares — which is exactly the trap a signature-matching reader walks into and a
checksum cannot.

Two rows now claim `.dsk`, and they stay apart by construction rather than by
table order: a raw disk is one only when the image is *not* a ProDOS volume. And
the format matters past one game. `zvm-cli` declines Version 6 by design, and
every Apple release above — Arthur, Journey, Shogun, Zork Zero — is v6, so until
this disk arrived, "lanthorn reads Apple II media" had never once meant "and
plays a single-game Apple disk from the command line". *Planetfall* is v3. It
boots, prints its banner and names release 29.

What was genuinely missing was not a format but a **set**. The story is paged
across every floppy in the release and no single one carries a game, so opening a
disk had to become a question a release could answer. It did, format-neutrally:
name any volume and the others are offered alongside, and nobody else pays for it
— the siblings are read only when the disk you named has no story of its own,
which is true of a *Shogun* floppy and false of every compilation disk there is.
Both sets verify against their own header checksums on the way out, *Shogun*
release 311 serial 890510 and *Zork Zero* release 383 serial 890602, so a
mismatched pile of floppies is refused rather than reassembled into plausible
nonsense. And which files are one release was already known — the picker had
been grouping multi-disk sets by name for a quest already, reading its list of
disk spellings off the same table, so it recognised the two presses the day the
reader landed without a line changed.

The proof was not free, mind. One function had been missed — the one that reads
an archive you name *inside* a disk, which predated the table and still carried a
hand-written two-reader chain. It was merely stale while two formats existed, and
became a defect the instant a third arrived: the launch dialog enumerated a PC
floppy's `ZORK0.EG1` through the table and offered it, and the loader had no arm
that could open it. Offered, picked, nothing drawn. It goes through the one table
now, which is exactly the failure mode the table exists to make impossible.

### The Commodore disks, where both of those problems arrive together

The 1541 press is the raw-sectors problem and the multi-disk problem in one
medium, and neither of them the way the Apple posed it.

Commodore DOS is present on all three `.d64`s in `stories/` and is a decoration
on every one. *Trinity*'s directory sector holds story data — the game is written
straight across it — and its Block Availability Map cheerfully reports the entire
disk free while 387 sectors are written. *Hitchhiker's* keeps a directory whose
one file, `THE HITCHHIKER'S`, is three blocks of BASIC loader, and stamps its DOS
version bytes `TG` instead of the standard `2A`. So there is again nothing to
enumerate, and the story has to be found rather than opened.

The new part is that the two presses do not agree on where to put it. The 1984
*Hitchhiker's* spends **sixteen** of each track's twenty-one sectors and leaves
the other five formatted-blank, skipping the loader and directory tracks whole;
the 1986 *Trinity* spends every sector it can reach and skips only the BAM.
lanthorn does not guess between them: it tries each, and keeps the one whose
reassembly verifies against the story's own header checksum. Where a press stops
on a disk needs no table either, because a 1541 `FORMAT` leaves every block as
`$4B` and then 255 × `$01` — so the reader simply stops where the disk stops
having been written to, and moves to the next floppy.

Which it has to, because *Trinity* is on two. This is arithmetic rather than
packaging: Version 4 counts its length field in units of four, so *Trinity* is
262,064 bytes, and a 1541 floppy holds 174,848 including the interpreter. Side 1
carries the header and 344 sectors; side 2 carries the other 680 and no header at
all — it cannot say what game it is, what release, or even that it is Infocom.
The set model from the Apple II work joined them the day `.d64` became a spelling
the table claimed, and open either side and the whole game comes up.

The checksum settled *which* sectors, and — as the Apple disks had already taught
— a byte sum cannot settle what **order** they go in. So the layout was pinned
three more ways: the dictionary at each header's own pointer decodes as a textbook
one (`, . "` with 7-byte entries and 969 words for *Hitchhiker's*; `. , " ! ?`
with 9-byte entries and 2,120 for *Trinity*), a fingerprint over every sector in
order is recorded so a later change cannot move it quietly, and — best of all —
*Trinity*'s Commodore press is release 12 serial 860926, the same build as the
`trinity-r12-s860926.z4` beside it. What comes off the two floppies is
byte-identical to that file from `$40` to the end.

Three bytes below `$40` are not identical, and they are the nicest detail on the
disk. This press declares its high-memory mark at 22,527 where the reference
build says 63,423 — a third as much resident — because it was pressed for a
machine with 64 KB of RAM and a 256 KB story to page through it. The header
checksum starts at `$40` precisely so the interpreter-facing head of the header
may differ, and all three bytes are inside that exemption.

### A floppy is a different release

Worth knowing before you compare two runs: the disk image is not the same story
as the `.z6` sitting beside it. It is a different **build** of the game, and the
builds do not always behave alike. *Journey*'s floppy is release 30; the bare
story file is release 83 — and where r83 narrates through window 0, r30 narrates
through window 2. A screen rule that is right on one of them can be wrong on the
other, which is exactly what happened once.

What each medium carries, measured across the collection:

| Title | Amiga floppy | Bare story file |
| --- | --- | --- |
| Journey | release 30, serial 890322 | release 83, serial 890706 |
| Zork Zero | release 366, serial 890323 | release 393, serial 890714 |
| Shogun | release 295, serial 890321 | release 322, serial 890706 |
| Arthur | release 54, serial 890606 | release 74, serial 890714 |
| Beyond Zork | release 57, serial 871221 | release 57, serial 871221 |
| Zork I | release 88, serial 840726 | release 88, serial 840726 |
| Zork II | release 48, serial 840904 | release 48, serial 840904 |
| Zork III | release 17, serial 840727 | release 17, serial 840727 |
| Zork: The Undiscovered Underground | release 16, serial 970828 | — |

Zork Zero has a third medium, and it is the outlier of the whole collection: the
Macintosh floppy carries **release 296, serial 881019** — October 1988, where the
Amiga disk is March 1989 and the bare story file July 1989. Ninety-seven releases
separate the Mac build from the PC one. Treat a finding made on it as describing
that build and no other. It will also tell you which machine it thinks it is on
if you ask — `version` off that disk answers *"Macintosh Interpreter version
6.65"*, which is the game reading header byte `0x1E` back to you.

It is not the only Macintosh press on the shelf, and the second one went unnamed
here for months while it played perfectly well. `Shogun.toast` is a bare HFS
volume — the Macintosh signature `BD` sits 1,024 bytes in, and the volume calls
itself `Shogun` — wearing a Toast CD's extension, which lanthorn never reads:
formats are recognised by content, so the name was never evidence about anything.
It carries **release 292, serial 890314**, the earliest *Shogun* in the
collection and a fourth build of it:

| *Shogun* medium | Release |
| --- | --- |
| `Shogun.toast` (Macintosh, HFS) | v6, release 292, serial 890314 |
| `James Clavell's Shogun.adf` (Amiga) | v6, release 295, serial 890321 |
| `shogun_s1.dsk`…`s5` / `Shogun.po` (Apple II) | v6, release 311, serial 890510 |
| `shogun-r322-s890706.z6` (bare) | v6, release 322, serial 890706 |

*Sherlock* makes the opposite point on four media at once: the Amiga floppy, the
Macintosh `Sherlock.img` and the bare `.z5` are all release 26, serial 880127 —
three media, one build — while the DOS and Apple IIgs collections carry release
21, serial 871214. The medium decides, and it decides differently per title.

And the *Masterpieces of Infocom* CD-ROM (`InfocomMasterpieces.img`) mounts as
the 12 MB HFS volume it is and opens with *Zork Zero* release 296, serial 881019
— the same build as the Macintosh floppy. That agreement is the finding: the
outlying October-1988 build is the *Macintosh's*, not one disk's.

And *Hitchhiker's* takes the rule to its limit, now that the PC and Atari presses
are readable. Three media, three releases, and **two different Z-machine
versions**:

| Medium | Release |
| --- | --- |
| Atari ST compilation (`STORY.DAT`) | v3, release 56, serial 841221 |
| DOS standalone 360 KB floppy | v3, release 58, serial 851002 |
| DOS *Lost Treasures* collection | **v5**, release 31, serial 871119 |

The collection ships the later "Solid Gold" edition — a different engine version,
45 KB more story, and built-in hints the other two do not have. A result measured
on one of those describes exactly one of them.

The Apple II press makes the same point once more, and this time with a game that
is *not* Hitchhiker's. *Trinity* is release 12, serial 860926 on the Apple IIgs
*Lost Treasures* volume 5 and release 11, serial 860509 on `Infocom Compilation 8`
for the Atari ST — two floppies, two builds, six months apart. What each ProDOS
volume opens with:

| Volume | Opens |
| --- | --- |
| `Beyond Zork (1988)(Infocom).2mg` | Beyond Zork, v5 release 57, serial 871221 |
| *Lost Treasures* 1 (`INFOCOM1`) | — the GS/OS launcher, no game on it |
| *Lost Treasures* 2 (`INFOCOM2`) | Beyond Zork, v5 release 57, serial 871221 |
| *Lost Treasures* 3 (`INFOCOM3`) | Stationfall, v3 release 107, serial 870430 |
| *Lost Treasures* 4 (`INFOCOM4`) | The Lurking Horror, v3 release 203, serial 870506 |
| *Lost Treasures* 5 (`INFOCOM5`) | Trinity, v4 release 12, serial 860926 |
| *Lost Treasures* 6 (`INFOCOM6`) | Sherlock, v5 release 21, serial 871214 |
| *Lost Treasures* 7 (`INFOCOM7`) | Wishbringer, v3 release 69, serial 850920 |
| `Arthur Quest 4 Excalibur.2mg` | Arthur, v6 release 63, serial 890622 — packed |
| `Arthur.po` (bare, 3.5") | the same press again — same story, same 168 pictures |
| `Journey.2mg` | — declares five segments, carries four; no game (its header still says release 77, serial 890616) |
| `Journey.po` (bare, 3.5") | Journey, v6 release 77, serial 890616 — packed, and complete |
| `journey_s1.dsk`…`s5` (5.25") | Journey, v6 release 77, serial 890616 — packed across five |
| `shogun_s1.dsk`…`s5` (5.25") | Shogun, v6 release 311, serial 890510 — packed across five |
| `Shogun.po` (DiskCopy 4.2, 3.5") | Shogun, v6 release 311, serial 890510 — the same press as the five floppies, packed on one disk behind an 84-byte wrapper |
| `zork_zero_1.dsk`…`_4` (5.25") | Zork Zero, v6 release 383, serial 890602 — packed across four |
| `ZorkZero.po` (bare, 3.5") | Zork Zero, v6 release 383, serial 890602 — packed across four subdirectories |
| `Planetfall r29 …dsk` (5.25", raw) | Planetfall, v3 release 29, serial 840118 — no filesystem; 426 sectors from track 3 |

Each of volumes 2–7 carries three to seven games; the one listed is the largest,
which is what opening the disk gives you when nothing on it wears a conventional
story name. Ask the picker or `--story` for the rest. The Apple IIgs *Beyond
Zork* is a happier note to end on than the trio above: it is the **same build**
as the Amiga floppy and the bare `.z5`, so for once all three media agree.

The PC disks add a smaller trap worth naming: *the same release can be a
different file size on different media*. `LURKING` is 153,600 bytes on one Atari
compilation and 129,024 on another, and both are v3 release 203 serial 870506 —
identical builds with different trailing padding. Size is never a release
identifier. Read the header.

Every graphical title ships a *different* build on its floppy; the v3/v5 ones
ship the same build on both media. A resource `.blb` beside a story is never a
third build — it holds artwork and no executable, so the release you play is
decided entirely by the file you open.

The practical rule, and the one the interpreter's own tests follow: a report made
on a disk image is reproduced on that disk image, and a finding names the release
it was measured on. `crates/app/tests/suites/real_media_releases.rs` pins this whole
table, plus the frame each build lays out, so an upgraded fixture announces
itself instead of quietly rebasing someone's investigation.

**And the table now guards itself.** A hand-written table has one failure mode a
test cannot normally see: what is *not* in it. An agent went to that file for a
Macintosh *Shogun*, found no row, and concluded there was no such press — a
statement about a table that read as a statement about the world. So a case
walks `stories/`, asks the format table what each file actually is, and fails on
any medium neither pinned above nor listed with a reason for its absence. It
found `Shogun.toast` and twenty-one more the day it was written: six Atari ST
compilations, nine DOS floppies, two Macintosh volumes, an Amiga *Sherlock*, a
Commodore GCR bitstream, and two ProDOS volumes that carry no whole game and now
say so out loud.

### The command-line player takes a floppy too

`zvm-cli` — the no-map DOS-style player — mounts a disk image exactly the way the
TUI does, and it cost nothing to give it: `blorb` hand-rolls every one of these
readers with zero dependencies, and `zvm-cli` already linked it. So
`zvm-cli "Zork I - The Great Underground Empire.adf"` drops you at *West of
House* off the original floppy, no unpacking, no rename, and the same
content-based identification decides what on the disk is a story.

**Exactly the way** is meant literally: the CLI opens every format the TUI does,
Macintosh disks included, because both go through the same table. Point it at a
graphical v6 disk of either kind and you get the v6 refusal — the one every
graphical Amiga floppy already gets, telling you to run it in lanthorn — rather
than a complaint about the disk. That distinction matters: it says the mount
worked and only the renderer is missing.

One thing the CLI needs that a single-game floppy never asks for: **which one**.
Amiga releases came one game to a disk, but the compilations did not — an Atari
ST or PC collection carries four to six stories on a single image — so when more
than one turns up you get a menu. Here is a real Atari one:

```
This disk holds 4 stories:
  1) The Hitchhiker's Guide to the Galaxy  (v3 r56 s841221)  HITCHHIK/STORY.DAT
  2) Bureaucracy  (v4 r86 s870212)  BUREAUCR.ACY/STORY.DAT
  3) Cutthroats  (v3 r23 s840809)  CUTHROAT/STORY.DAT
  4) Leather Goddesses of Phobos  (v3 r59 s860730)  LEATHER.GOD/STORY.DAT
Which one? [1-4] 3
Opening 3) Cutthroats  (v3 r23 s840809)  CUTHROAT/STORY.DAT
```

Three things are on every line, and each earns its place.

**The game's name comes first, and no medium supplied it** (SQ-0884). All four
files here are called `STORY.DAT`; on *Lost Treasures* they are `MAC/BALLYHOO`
and `PC/DATA/BEYONDZO.DAT`, and on an Amiga floppy every one of them is
`Story.data`. None of those is a title. What *is* one is the release and serial
in the header, so the menu looks the build up in the bundled title table
(`crates/cli-host/src/known_titles.tsv`) — the same table and the same key the
story picker names its rows with and the per-game save directory is built from,
so a game reads the same in all three places. A build the table does not carry
falls back to the name the disc stored, which is the honest failure: a missing
row costs a filename, a wrong one mislabels a game.

**The version, release and serial come next**, and they are not decoration
either — the collection holds three different builds of *Hitchhiker's* alone, and
*Lost Treasures I* carries the Solid Gold v5 r31 alongside the v3. Two rows with
one title are told apart here and nowhere else.

**The stored name stays on the end.** *Masterpieces* presses *Ballyhoo* three
times — `MAC/BALLYHOO`, `PC/BALLYHOO/DATA/BALLYHOO.DAT`, `PC/DATA/BALLYHOO.DAT`,
one build in three files — so a menu that stopped at the title would print three
identical lines and no choice anybody could make. It is also what keeps the
folder visible on the Atari press, where the directory is the only thing telling
four `STORY.DAT`s apart.

`--story` answers to either name: `--story cuthroat` picks the folder as it
always did, and `--story "leather goddesses"` now picks the title the menu shows.
When a title matches two builds — both copies of *Hitchhiker's*, say — that is
reported as ambiguous rather than guessed at, and the menu number always decides.

A disk with one story opens straight into it and asks nothing. A disk with none
says what it mounted instead of failing later as a corrupt story file. And
nothing here ever blocks a script: pipe stdin, and rather than prompt at a
terminal that isn't there, `zvm-cli` lists the candidates and tells you to pass
**`--story <n|name>`** — a menu number, or any part of a name that picks out one
story.

**The TUI asks the same question a different way** (SQ-0859). It has a list
already, so a compilation contributes one *row per game* to the story picker
rather than a menu: same mount, same enumeration, same names, and the row carries
which story it stands for straight into the launch. A menu is what a front-end
with nothing on screen needs; a picker that can already sort and search by title
does better by putting the games in it. Both front-ends reach every story on
every image, and — because the save key is the story's own release and serial —
`--story 4` and the picker row land in the same directory.

**And lanthorn takes `--story` too, for when nobody is watching** (SQ-1078). A
picker is the right answer for a player and the wrong one for everything else:
until this flag, the only way to reach a game on a compilation disc was to launch
it and move a cursor, so no capture, no harness and no bug report could name one.
`stories/InfocomMasterpieces.img` opens *Zork Zero* by the volume's own tiebreak,
and the Macintosh *Arthur* press sitting beside it on the same platter could not
be measured at all — SQ-1063 worked around it with a StuffIt archive unpacked
into a directory, which is not a medium, so the interpreter profile resolved
wrong and every number described a screen no player sees.

```sh
lanthorn stories/InfocomMasterpieces.img --story arthur
lanthorn stories/InfocomMasterpieces.img --story 7
```

Same flag, same spelling, same matching rule — literally the same code
(`cli_host::story_pick`), because a `--story arthur` that found a game at the
prompt and nothing in the TUI would be its own defect. A number is a position in
the list the picker would have shown; a name is matched case-insensitively
against both the stored name and the title, a fragment is enough, and something
that fits two games is refused *with the list* rather than guessed at. Nothing
that fails to match ever falls back to booting an arbitrary game.

Naming a story goes straight into it: no picker on the way in, and none on the
way out either — the launch reads as the single-file launch it is, so it exits
when the game does rather than depositing you in a list you asked not to see. And
because the flag names a story ON something, it requires a path, exactly as
`--pictures` does.

**And both front-ends read the whole set** (SQ-0844, SQ-0961). These collections
were pressed as sets — seven Apple II volumes, nine Atari ST floppies,
`floppy1.ima` through `floppy5.ima` — and a set is one shelf of games rather than
a pile of disks. lanthorn works out which files belong together from their names
alone: one directory, one disk-image extension, identical but for a run of digits
counting 1, 2, 3…. Name any single volume and the picker opens on the entire
release, which is what finally makes *Lost Treasures* volume 1 useful — it is the
GS/OS launcher with no game on it, so `lanthorn "…(Disk 1 of 7).2mg"` used to be
an error message and now lists all thirty games.

The menu above learned the same thing later, and the gap was visible: point
`zvm-cli` at the Amiga *Lost Treasures* disk 1 and it offered the six games on
that platter while lanthorn, pointed at the same file, listed all twenty. Nothing
was wrong with the CLI's mount — it asked a narrower question, because there was
no wider one to ask. There is now, one function beside the mount, and both
front-ends ask it. The CLI's menu lists the release; a build that two volumes
both carry is listed once, so the three-floppy DOS *Zork Zero* still opens
straight into its one game without a menu at all.

That fix needed a second one first, and it is a nice reminder that a naming rule
is only ever as good as the shelf it was written against. The Macintosh DiskCopy
press of *Lost Treasures* spells its volumes
`The Lost Treasures of Infocom - Disk 1 - Beyond Zork, Lurking Horror.dc42` — the
number in the middle, and **every volume naming its own games after it**, so the
five stems agree on nothing at all past `Disk N`. "Identical but for a run of
digits" grouped none of them. The suffix is now dropped from the comparison, but
only when the number is introduced by a word that says it is a disk number, which
is what keeps `Ultima 1`, `Ultima 2 - Revenge`, `Ultima 3` three games rather than
one release.

That also settles the overlap these sets carry. `Infocom Compilation 5` and
`Compilation 8` both hold *Trinity* release 11, serial 860509 — one stored flat
as `TRINITY.T`, one as `TRINITY/STORY.DAT` — and the shelf repeats *Lurking
Horror*, *Moonmist*, *Stationfall*, *Cutthroats* and *Hitchhiker's* the same way:
39 rows for 33 games. Matching on the IFID lists each build once, off the first
disk that offers it. The folding is narrow by construction — only within one
release, and only between the *same build* — so *Zork Zero*'s 296, 366 and 393
stay three rows, and that 393 stays one row per medium across `floppy5.ima`, the
360K DOS press and the loose `.z6`, because those are three separate things
rather than volumes of one release. What it refuses is in
`crates/cli-host/src/disk_set.rs`; the sharpest case is *Zork Zero*'s 360K and 720K
DOS presses, which both spell their disks `(Disk 1)` and `(Disk 2)` and differ
only at `360`/`720` — a capacity, not a disk number, and therefore two sets.

**And each of those six games gets its own saves** (SQ-0850). A per-game save
directory used to be named after the story file, which was fine while one image
meant one game and quietly catastrophic once it did not: all six stories on an ST
compilation shared one `<image>.save/`, one `default.lanthorn`, and whichever you
played last owned it. A story taken off a disk image is now keyed by its own
**release and serial** — `hitchhikers-guide-r56-s841221` — so two games on one
disk cannot collide, renaming the image keeps your saves, and the Amiga, DOS and
Atari ST presses of *Zork I* r88/840726 all reach the same directory because they
are the same build. A loose story file still keys on its filename, exactly as
before, so nothing you already have moves. `zvm-cli` and the TUI read one helper
for this, which is why `--story 3` off a compilation and the same game opened in
lanthorn find each other's saves.

**A zip is the third case, and it had to be settled before a zip could offer a
choice at all** (SQ-1098). A story taken out of an archive has no release-and-
serial identity to be keyed by — that key is defined in terms of the disk format
a story was pressed onto, and a zip is somebody's download rather than a press —
so both games in a two-game archive fell through to the zip's own filename and
would have shared one directory. They are keyed by the **entry's own basename**
instead: `if-archive-pack.zip` holding `amber.z5` and `beacon.z5` names
`amber.z5.save` and `beacon.z5.save`, which is exactly what those two games key
on when they are loose. That is why lanthorn shipped a zip that could carry two
games *before* it would list them: enumerating first would have traded a visible
limitation — one game reachable — for an invisible one, two games overwriting
each other's saves. All three rules now read one value, `StoryOrigin`, so a
caller holding two of the three facts gets a compile error instead of a
plausible-looking key.

And the floppy now tells the CLI which *machine* it is, not merely which story.
A disk format is evidence, and evidence that only reaches one front-end is half
an answer: for a while the TUI took an `.adf` for an Amiga while `zvm-cli`
mounted the same floppy and then ran it as an IBM PC. Both now ask the same
question of the same code — `blorb::medium`, the one crate that recognises these
filesystems and the only one both front-ends share — so
`zvm-cli "Zork - The Undiscovered Underground.adf"` answers VERSION with
*Interpreter 4* where the bare story file answers *Interpreter 1*. It is a
**default**, never a verdict: `-I 6` still puts you on the IBM PC, off the
Amiga floppy or anywhere else.

## Z-machine

- **Standard Quetzal save/restore** — the game's own SAVE/RESTORE writes and reads
  the interchange Quetzal format, so a save you make here opens in Frotz and vice
  versa.
- **Story-dictionary introspection** — lanthorn reads the game's built-in word list
  and turns it into live verb/noun autocomplete, so you type `exam` and the game's
  actual vocabulary completes it.
- **v4+ upper-window screen model** — cursor-addressed status lines and full-screen
  forms (Bureaucracy's infamous licence application, for one) render in a fixed
  grid pinned atop the transcript, and `read_char` keystrokes are forwarded so you
  fill those forms in place. The game is told the story pane's **real** size — the
  standard asks the interpreter to keep the current height and width in the header
  and lets it change them whenever it likes, so lanthorn measures the pane and
  re-measures it on every terminal resize. A game's full-width form therefore lines
  up column-for-column with the prose beside it instead of floating in a fixed
  80-column box. Pin a fixed screen with `virtual_screen_cols`/`virtual_screen_rows`
  if you want a game's original layout back; when the pane is smaller than a pinned
  screen, the viewport auto-follows the cursor. The virtual window is themeable
  from `[elements]`: `upper_window` inks its cells, and `upper_window_border`
  both colours and shapes the frame around them. That frame is off by default —
  the bar sits flush against the story and the game keeps every row and column
  of the pane — so set `style = "single"` (or `double`/`thick`/`rounded`) if you
  want it boxed.
  During a `read_char` prompt keystrokes go to the game; only the hotkey prefix
  (default `Ctrl+P`) stays reserved.
- **Timed / interrupt input** — v4+ `read` and `read_char` honor their `time`+
  `routine` operands, so real-time games keep ticking while you think: the game's
  interrupt routine fires every N tenths of a second (countdowns and clocks — the
  bomb in Border Zone) and can cut the read short. Controlled by
  `honor_timed_input` (default on), the `/toggle-timed-input` command, and the
  settings row; `zvm-cli` takes `--timed-input off`. The VM stays zero-dependency —
  the wall clock lives in the hosts, not the interpreter.
- **A different game every time you sit down** — every engine here runs the same
  xorshift generator, and a VM core built in isolation seeds it from a fixed
  constant so its own tests mean something. That is exactly the wrong thing to
  hand a *player*: a story that never calls the seeding opcode would deal the
  identical sequence on every launch, and a roguelike would be the same dungeon
  forever. So the app seeds each engine from the OS before the story boots —
  before, because a game's initialisation routine is precisely where the
  shuffling is done, and a seed installed after the first prompt changes nothing
  the player will ever see. Set `random_seed` in `config.toml` to pin it instead
  and the run becomes reproducible end to end; lanthorn names the seed it used on
  the console at startup, so an interesting run can be asked for again. The VM
  crates stay dependency-free through all of it — the entropy comes from std's
  own OS-seeded hasher, not a crate.
- **Interpreter number** — the story header's interpreter number (byte `0x1E`)
  defaults to **1 (DECSystem-20)**, following Frotz's rule (6 / IBM PC only for
  v6) — unless you opened a release disk image, in which case the medium picks
  the number instead (an `.adf` is an Amiga's 4, an HFS volume a Macintosh's 3,
  a `.st` floppy an Atari ST's 5, a `.2mg` ProDOS volume an Apple IIgs's 10), in
  every front-end alike.
  One medium deliberately does **not** move it. A DOS floppy is an IBM PC, and
  the IBM PC's honest number is version-dependent — that *is* Frotz's rule — so
  it is already in force and there is nothing for the disk to add; pinning a flat
  6 would quietly flip *Beyond Zork* on the *Lost Treasures* disk over to CP437
  character graphics, which is a rendering decision and not a container one.
  The Atari ST used to be the second such abstention, and it is worth saying why
  it no longer is, because the reasoning is the useful part. The worry was that a
  number here travels with a palette, a screen and a set of default colours, and
  that announcing a machine we could not fully describe would produce an
  incoherent one. But the thing that goes wrong in that scenario is a number
  *contradicting* the artwork — and there is no ST artwork to contradict. Infocom
  never wrote a version-6 interpreter for the ST, so all thirty-nine stories
  across the nine ST compilations are v3, v4 or v5, and the collision cannot
  happen. Meanwhile the ST's own interpreters turned out to answer the rest of
  the questions outright: `INTWRD DC.B 5 — MACHINE ID FOR ATARI ST`, a white page
  under black text, and a colour table that is the standard's own eight colours.
  So the ST profile states what it knows, declines the one thing it does not (a
  version-6 screen, which the machine never had), and *Trinity* off an ST disk now
  answers VERSION with *Interpreter 5*.
  **Apple ProDOS was the second abstention, and is no longer one** — the
  reasoning is worth keeping, because the premise survived and the conclusion did
  not. ProDOS is the only medium here that names a *family* rather than a
  machine: it is the Apple II's filesystem from the IIe onward, and §11.1.3 gives
  that family three numbers — 2 Apple IIe, 9 Apple IIc, 10 Apple IIgs — with
  nothing on the volume to choose between them. That is not pedantry, and
  Infocom's own code proves it rather than merely permitting it. The Apple II
  YZIP — their version-6 interpreter for the machine, and the program sitting on
  the *Arthur* and *Journey* disks as `INFOCOM.SYSTEM` — picks between all three
  *at boot*:

  ```text
    ; Make sure we are on a good machine, like a ][c or ][e+/][gs
    MACHINE:  lda MACHID1 / cmp #6 / bne BADMACH
              lda MACHID2 / bne MACH1
              lda #IIcID              ; Apple ][c thank you
    MACH1:    sec / jsr MACHCHK / bcs OLDMACH
              lda #IIgsID             ; this is a ][gs
    OLDMACH:  lda #IIeID              ; this is IIe
  ```

  and hands the result straight to the story (`lda ARG2+LO { get machine id! } /
  sta ZBEGIN+ZINTWD`). One disk, pressed for the whole family, with the machine
  identified at run time. So the volume genuinely cannot name the press.

  What changed the answer is the realisation that **abstaining is not neutral
  here**. The DOS floppy can abstain because Frotz's rule *is* the IBM PC's rule,
  so a DOS disk describes itself correctly by saying nothing. On a ProDOS volume
  that same silence lands on 1 — the DECSystem-20 — or, for version 6, on 6, the
  IBM PC, which is also the one value that switches lanthorn into CP437 character
  graphics. Saying nothing does not leave the Apple II unnamed; it names it
  something else, on another continent. And §11.1.3 asks for exactly the
  judgement being ducked: *"an interpreter should choose the interpreter number
  most suitable for the machine it will run on."* Of the three machines that YZIP
  will start on at all — it refuses anything below an enhanced IIe outright — the
  one a modern terminal with colour and a large screen resembles is the IIgs. So
  a ProDOS disk is now a **10**, and `--interpreter 2` or `9` still asks
  for the other two.

  It is measured rather than asserted, the same way the ST's 5 was. All
  thirty-one stories on the ten ProDOS images were traced under the old rule and
  under 10: twenty-four are byte-identical, five simply print the new number in
  their VERSION block, and one behaves differently — *Beyond Zork*, on both of
  its ProDOS presses, stops asking whether the terminal is a VT220 and draws its
  box-drawn interface unprompted, because an Apple IIgs is not a terminal that
  might or might not have line-drawing characters. It also signs itself **"Apple
  //gs Color Version A"** where it used to say *"DEC-20"* — Infocom's spelling,
  not ours.
  This byte is what unlocks colour on several Infocom games: Beyond Zork, for
  instance, only emits colour to a non-IBM interpreter and falls back to
  reverse-video under IBM PC. Override it with the app's `interpreter_number` config
  key, or `--interpreter N` — one spelling across `lanthorn` and `zvm-cli` alike,
  where it is also `-I N` —
  e.g. `6` selects the IBM PC path, which draws Beyond Zork's map box and cursor
  arrows as CP437 character graphics instead of Font 3. The `--interpreter`
  flag applies to one run only and is never written back to your config, so probing
  a game's behaviour can't quietly pin one machine for every story afterwards —
  unless you then set the value in the settings screen, which is a decision rather
  than a flag and persists like any other setting. Setting that row back to
  **default** removes the key, restoring the per-version rule on the next launch. The
  values are ZMSD §11.1.3's:

  | | | | |
  |---|---|---|---|
  | 1 DECSystem-20 | 4 Amiga | 7 Commodore 128 | 10 Apple IIgs |
  | 2 Apple IIe | 5 Atari ST | 8 Commodore 64 | 11 Tandy Color |
  | 3 Macintosh | 6 IBM PC | 9 Apple IIc | |
- **Interpreter profiles — the whole machine, not one byte.** Byte `0x1E` is not
  the only thing that makes a machine. A Version 6 game that reads it goes on to
  ask about the screen it has, the colours the interpreter calls default, and what
  "red" looks like here — and answering one of those as an Amiga while answering
  the rest as an IBM PC produces a machine that never existed. So the answers
  travel together as a named **profile**.

  **IBM PC** is the default and is simply what lanthorn has always done: the
  Frotz interpreter-number rule above, the resource file's own declared art
  resolution, your terminal's colours reported as the interpreter defaults, and
  ZMSD §8.3.1's colour table.

  **Amiga** is the sibling, and it selects itself: a story booted straight out of
  an `.adf` release floppy came off an Amiga, so lanthorn presents one — 
  interpreter number 4, the Amiga's 320×200 standard window (which is what makes
  the artwork in a native `Pic.data` archive scale onto the 640×400 screen, since
  that format has no `Reso` chunk to declare it), a dark grey page and white ink
  reported as the interpreter's defaults, and the palette Infocom's own Amiga
  interpreter loaded — a slightly darker green and blue than the standard's, a
  warmer yellow, and its own three Version 6 greys. Whatever you name outright
  still wins: a number set in config, `--interpreter`, or `-I` outranks
  the medium every time, and only the *default* moves.

  **Macintosh** is the third, and it was the last one to arrive because it was
  the last one anybody could *prove*. A Mac release floppy has mounted and played
  for a while, but what a Mac's page, palette and screen looked like was not
  something the media in hand could settle, and a bundle guessed from memory is
  exactly the incoherent half-machine profiles exist to prevent. Infocom's own
  Macintosh interpreter settles all of it, so the bundle now ships: interpreter
  number 3, black ink on a **white** page — the Mac's whole visual signature, and
  the exact opposite of the Amiga's dark grey — and the standard colour table,
  because the Mac's own colour mapping *is* that table and nothing more.

  It hangs on the **medium**, and it has to. The Amiga and the Macintosh wrote
  the same colour archive, byte for byte indistinguishable, and the Mac release
  disk proves it by carrying one. A volume cannot be mistaken that way: HFS is
  Apple's filesystem and nobody else wrote one.

  And the Macintosh is the one machine with **two screens**, which is the part
  worth knowing about. Infocom's Mac interpreter sized its window and picked its
  picture file in a single decision — a big colour Mac got a 640×400 window and
  the colour archive drawn at double size, and a standard compact Mac got a
  480×300 window and the *monochrome* archive drawn 1:1. So on a Mac disk the
  artwork you choose is the screen you get, and
  [the artwork's own page](v6-graphics.md#two-macintosh-screens) has the numbers.
  (512×342, the compact Mac's famous screen, is the *hardware* — the game window
  sits inside it under the menu bar, and the story is told about the window.)

  **Atari ST** is the fourth, and it is the one that shows a profile is allowed
  to say *"I don't know"* about part of itself. It answers interpreter number 5,
  black ink on a white page, and the standard colour table — all of it read out
  of Infocom's own ST interpreters, where `INTWRD DC.B 5` is labelled `MACHINE ID
  FOR ATARI ST`, `DEF_BACK 9`/`DEF_FORE 2` are commented *"default ST background
  id = white"* and *"foreground id = black"*, and the ST's colour table asks for
  the standard's own eight colours at full saturation. It states **no standard
  window at all**, and that absence is a fact rather than a gap: Infocom never
  wrote a version-6 interpreter for the ST, so there is no ST artwork for a
  standard window to be the resolution of. (The machine could show only four of
  its eight colours at once in 80-column mode, one of them always the background
  — a display ceiling a terminal does not have, so there is nothing to express.)

  The ST is also the clearest demonstration that this byte is not decoration.
  *Beyond Zork* on an ST compilation, told it was a DECSystem-20, opened by
  asking **"Is this a VT220?"** — a question about a 1983 DEC terminal, put to
  someone who has just inserted an Atari floppy — and a player who answered *no*
  got a stripped-down screen: no box around the room description, the compass
  rose drawn as `\` and `@-`, and *"use the UP and DOWN arrow keys"* spelled out
  in words. Told it is an Atari ST, the game never asks, because an ST is not a
  terminal that might or might not have line-drawing characters. It goes straight
  to the boxed layout with its block-graphic compass and real `↑`/`↓` arrows —
  the same screen the DEC-20 player only reached by answering *yes* — and it
  signs itself *"Atari ST Color Version A"* where it used to say *"DEC-20"*. That
  "Version A" is corroboration in its own right: Infocom's ST version-5
  interpreter is stamped **FROZEN Version A** in its source.

  Across the rest of that corpus the change is quiet, which is the point of
  having measured it: of the thirty-nine stories on the nine ST compilations,
  thirty-two behave identically, six merely print the new number in their VERSION
  block, and only *Beyond Zork* does anything differently. The version-3 stories
  cannot notice at all — byte `0x1E` has no meaning before version 4, which is
  why the ST's own version-3 interpreter leaves it zero and comments it
  *"(UNUSED)"*.

  **Apple IIgs** is the fifth, and it is the one where declining a member of the
  bundle became a *judgement* rather than a shortage of evidence. Its number, its
  page and its palette all come out of the Apple II YZIP — Infocom's version-6
  interpreter for the machine, which is not merely in the leaked source archive
  but sitting on two of the disks in the reference collection. It answers
  interpreter number 10, white ink on a **black** page (the YZIP's own boot code:
  *"black is the background color"*, *"the color white is the foreground
  color"*), and the standard colour table — because the Apple's colour map and
  its inverse, side by side in the interpreter's tables, close on exactly the
  standard's eight colours and mark the machine's other eight *"no Z-machine
  colour"*. That black page is the darkest of the five; the Amiga's is a dark
  grey rather than black, and the Macintosh and the ST both boot white.

  It states **no standard window**, and this is the interesting decline, because
  unlike the ST the machine *has* a version-6 screen and lanthorn can quote it:
  140×192 on a 3×9 character cell, 46 columns by 21 lines, the 560-dot double
  hi-res display counted in four-dot colour pixels. That is a different screen
  *model*, not a different resolution — this knob holds a picture space that gets
  doubled onto the 640×400 unit screen and cut into 8×16 cells, so claiming
  140×192 would tell a game its screen was 280×384 and 35×24 characters, a
  machine that never existed and further from the Apple's real 46×21 than the
  80×25 it gets by declining. Honouring it properly means making the character
  cell run-time state, which is the same refactor the Macintosh's real 7×15 cell
  was declined for. There is also nothing yet for it to size: *Arthur*'s and
  *Journey*'s pictures live inside a segmented container that has no reader.

  **Commodore 128** is the sixth, and it is deliberately the thinnest of them:
  the number, and an explicit "not established" on everything else. It exists
  because the number would otherwise be dropped on the floor — `blorb`'s `.d64`
  row answers 7, `zvm-cli` takes that straight off the medium, and a profile is
  how the TUI takes it, so a Commodore disk with no profile would have the two
  front-ends disagreeing about which machine the player is on.

  Choosing 7 over 8 is the ProDOS argument with better evidence. A `.d64` is a
  1541 image and §11.1.3 numbers two Commodores — 7 the 128, 8 the 64 — so the
  geometry cannot say which. The **disks** can, though, and they disagree with
  each other: *Hitchhiker's* boots from a Commodore 64 BASIC stub, `SYS(2063)`,
  while *Trinity* opens with `CBM`, the Commodore 128's autoboot signature, and
  runs an interpreter that touches the C128's own memory-management register
  `$FF00` forty times — a register a 64 does not have. It could not boot on a 64
  even if it wanted to, since its directory sector is full of story. And byte
  `0x1E` means nothing before Version 4, so the Version 3 *Hitchhiker's* is
  exactly the disk with no opinion: the only Commodore story here that **reads**
  the byte is on the 128 disk. Declining is not the neutral option — it lands a
  Commodore story on the DECSystem-20 — and `--interpreter 8` still names the 64.

  What it does *not* claim is the rest of a machine. No standard window, because
  Infocom never wrote a Version 6 interpreter for the Commodore at all; no
  palette and no default colour pair, because none has been read out of Infocom's
  Commodore interpreter, and the C64's sixteen famous hardware colours are the
  machine's reputation rather than the interpreter's evidence — the same call the
  ST's profile makes about its 512 colours and the Apple's about double hi-res.
  Filling those in wants a source, not an afternoon.

  **Apple IIe** and **Apple IIc** are the seventh and eighth, and they cost
  almost nothing to add because they are the Apple IIgs's bundle with a different
  byte. Infocom's Apple II interpreter is one program for all three machines: it
  seeds the black page and white ink first, then works out at boot which Apple it
  is standing on and writes 2, 9 or 10 accordingly. A ProDOS *disk* still selects
  the IIgs, because the medium genuinely cannot name the press — but a player who
  names 2 or 9 outright now gets an Apple instead of an IBM PC wearing an Apple's
  number, which is what the fallback used to hand them.

  **Commodore 64** is the ninth, and it arrived last because it was wrongly
  refused. It states nothing a story can read — no palette, no `$2C`/`$2D` pair,
  for exactly the reason the Commodore 128 states none — and it exists for the one
  thing that *is* measured, its [period look](#the-period-look). The ground for
  leaving it out had been that a `.d64` is a 1541 image both Commodore machines
  read, so the disk cannot choose between 7 and 8. True, and the same thing is
  true of ProDOS and the three Apples, all of which have profiles. A medium that
  names a family rather than a machine means the number gets **asked for** instead
  of inferred; it does not mean the machine goes unmodelled. So a `.d64` still
  selects the 128, and `--interpreter 8` now gets a Commodore 64 rather than an
  IBM PC wearing its number.

  **Two numbers still have no profile, and each is a decline rather than a
  gap**: 1 DECSystem-20 (what declining already falls through to — whether it
  deserves a bundle of its own or is honestly "a terminal, the same as the IBM
  PC" is a decision, not a datum), and 11 Tandy Color (no fixture, no sourced
  constant — better absent than invented). Naming either of them
  still writes it into `0x1E`, because the story asked and §11.1.3 has an answer,
  but everything else about the presentation is the IBM PC's — and `zvm-cli`
  **says so** on stderr rather than letting the substitution pass unremarked.

- **One machine table, two front-ends.** Everything above that a *story* can read
  — the interpreter number, the default page and ink in `$2C`/`$2D`, the palette
  colour numbers resolve through, and the §8.3 screen rules a machine gets by
  name — lives in one table inside `zvm`, keyed by the §11.1.3 number. Both
  `lanthorn` and `zvm-cli` read it, so opening the same disk in either presents
  the same machine.

  That is new, and it fixed a real half-wiring: `zvm-cli` used to set the
  interpreter number and nothing else, so a story off a Macintosh or Atari ST
  press was told which machine it was on and left to work out what that machine
  looked like from the Z-machine's own generic default. The number and the page
  disagreed. (The Apple presses were the one place it never showed, because the
  Apple's black-page-white-ink pair happens to *be* that generic default.)

  What stays in `lanthorn` stays for a reason: reading a disk to work out which
  machine pressed it is file I/O, which the VM core deliberately has none of; the
  artwork-flavour preference needs the resource reader; and a standard window is
  a Version 6 picture space stated by an *archive* rather than by a machine.

  One of those machine facts decides what the *artwork* does, not just the text.
  The Amiga has a single set of colour registers, so a scene's palette is the
  whole screen's: on a real Amiga, *Shogun*'s ornate side panels are
  blue-and-white in the storm on deck and red-on-cream below decks, though the
  border is drawn only once. The same game's DOS press leaves them one colour
  throughout — and that is equally right, because the MCGA's DAC holds 256
  entries and Infocom used them, which is how *Arthur*'s map screen manages
  three palettes at once. One story, one border, two machines, two behaviours.
  lanthorn follows whichever machine you are presenting as; `one screen palette`
  in the table above is the column that says which.

  **Another decides where a line of text ends**, and it is the one place the
  standard admits Infocom's own interpreters did not do what the standard says.
  A Version 6 window has a *wrapping* attribute; §8.8.3.1.1 says that with it off,
  characters print until the right margin and everything past that is ignored. The
  Macintosh and Amiga interpreters never read it. §8.8.3.1.2.2's own commentary
  tabulates them and puts a dash in every attribute row — "the interpreter ignores
  the given state" — because both follow the `buffer_mode` opcode instead, which
  is on by default. So on those two machines a window word-wraps whether or not
  the game asked it to.

  That is not a footnote. *Shogun*'s InvisiClues turns wrapping **off** to paint
  its topic list, then prints a hint into the same 500-pixel window — and the hint
  is longer than the window. Read the attribute and the clue runs across the frame
  art and off the screen, cut mid-word at "…keep your ship from sinking bef". A
  real Amiga and a real Macintosh both wrap it onto a second line, at their own
  break points, and `machine-screenshots/amiga-shogun-hintshown.png` and
  `mac-shogun-hintshown.png` are the proof. `v6 wrap` in the machine table is the
  column that says which rule a press gets: `attributes` for the standard's own
  reading, `buffer_mode/…` for the two machines that ignore it.

  **One byte in that neighbourhood is still unsourced**, and it is one a story
  can print. Header `$1F` is the interpreter *version*, and lanthorn writes `A`
  for every machine — a value that arrived in the same early commit as the
  interpreter number's since-replaced "6, a common neutral value" and was never
  revisited. *Shogun* renders it as a decimal, so its Amiga credits read
  `Amiga Interpreter version 6.65` where a real Amiga read `6.8`. Until it is
  settled you can set it yourself:

  ```sh
  lanthorn "stories/James Clavell's Shogun.adf" --interpreter-version 8
  ```

  A number or a single character (`A` is taken as its ASCII code), because games
  render the byte both ways — *Nord and Bert* prints a letter where *Shogun*
  prints a decimal, so you can type what you saw. It is an experiment knob, not
  a setting: there is no config key and nothing is written back. Whether any
  story *branches* on the byte rather than merely printing it is exactly what it
  exists to find out.

  You can read the whole table without opening the source or starting a game,
  from either front-end:

  ```sh
  lanthorn --machines
  zvm-cli --machines
  ```

  Both print the same string, because both ask `zvm` for it — the reporter lives
  beside the table it reports (`zvm::machines`), so there is no second copy to go
  stale. It prints every machine `zvm` models, in number order, with every
  setting each one carries — the number it writes into `$1E`, the default page
  and ink it reports in `$2C`/`$2D`, the palette those colour numbers resolve
  through, and the two §8.3 screen rules — followed by the numbers that have no
  row and the argument for each absence. The colours come out **resolved through
  each machine's own palette**, which is the only rendering in which the page and
  ink columns mean what they say: colour 12 is `#5A5A5A` under §8.3.1 and
  `#424242` on the Amiga, where it happens to be the Amiga's own page. The output
  is generated from the table itself, so a machine added to `zvm` appears there
  with no second copy to keep in step.

  **A machine is not a screen, and a third block says where the two come apart.**
  Ask for the IBM PC's look and the answer depends on the story's *Version*,
  because Infocom shipped two IBM interpreters that disagree about white — XZIP
  (v1–v5) sends it to EGA attribute 7 and YZIP (v6) to attribute 15. So after the
  per-machine blocks comes a list of exactly the rows a Version moves, generated
  by asking `period_look_for` and `palette_for` both ways: the IBM PC's graphics
  mode and its ink, and the caret on both the IBM PC and the Amiga, which Version
  6 turned into the pair on screen reversed. Everything else answers the same for
  any story and so has no row there. The card that is *not* a Version — CGA,
  which a `--pictures foo.cg1` installs from the archive rather than from the
  story — is named in that block's legend for the same reason: a reader comparing
  two EGA rows would otherwise conclude EGA is all there is.

  One number in those blocks is worth knowing about before it surprises you. The
  IBM PC's page prints as `#0000AD` where the adapter's own byte is `#0000AA` —
  because a colour number reaches a story through the Z-machine's 15-bit colour
  space, where `0xAA` truncates to 21/31 and comes back bit-replicated as `0xAD`.
  Three parts in 255, an artifact of the colour space rather than an error in
  either value, and `#0000AD` is what your screen gets. Both blocks print it,
  because both ask `period_look_for` — the same question lanthorn asks.

  In a terminal, note that a machine's page is what the story is *told*, not
  something painted over your theme. Where a game names no colour, `zvm-cli` and
  `lanthorn` both still show your terminal's own — a machine's colours reach the
  screen only where the game actually asks for them.

  The Apple is also the profile whose *number* had to be argued rather than read
  — the Amiga, the Macintosh and the ST each write one byte and mean it, while
  the Apple II YZIP detects the machine at boot and writes 2, 9 or 10 accordingly.
  The **Interpreter number** entry above has that argument in full, and the
  thirty-one-story trace behind it.

  The artwork can select the machine too, and it sits between the two. If you
  name a picture archive for a game — the `pictures` key described under
  [Choosing which artwork a game draws](v6-graphics.md#choosing-which-artwork-a-game-draws) —
  then you have said which machine's rendition you want to look at, and lanthorn
  presents that machine: a `Pic.data` is an Amiga, an `.MG1`/`.EG1`/`.CG1` is an
  IBM PC. It reads that from the file's *contents*, never its extension, since
  the two containers are structurally different and a renamed file would
  otherwise lie about which machine you asked for. (The Macintosh wrote the same
  container as the Amiga and cannot be told apart from it in general, so it is
  not claimed to be — naming an archive off a Mac *disk* still gets you the
  Macintosh, from the disk underneath it.) MCGA, EGA and CGA are three video
  cards in one machine, so
  all three name the IBM PC and none of them moves byte `0x1E`; what a card does
  change is how densely its artwork was stored, which is
  [the art's business rather than the machine's](v6-graphics.md#choosing-which-artwork-a-game-draws).
  No *rendition* alters the screen a game is handed — EGA's own 640×200 mode on
  an 8×8 cell is the same 80×25 grid the 8×16 one gives. The **machine** can,
  though, and two of them do. The Macintosh's interpreter typeset Version 6 in
  12-point Geneva on a **7×15** cell (`mac/xzip.lst`: `colWidth := 7;
  lineHeight := 15`), so a standard-Mac screen is 68×20 characters rather than
  60×19. And where a release disk carries a proportional typeface of its own,
  the cell follows the *face*: Arthur's Amiga floppy ships a 10-row `char.data`,
  and its art doubles onto the 640×400 unit screen, so the story is told a
  **20-row line** and lays out the 20 text rows a real Amiga showed instead of
  25. Only the DECLARED height moves — a proportional face has no single advance
  to declare, so header `$27` stays the machine's 8 and the story's own column
  grid with it. What goes proportional is every *measurement*: the cursor
  advances by the glyph the face draws, a line breaks at the window's real pixel
  width, and the width a game reads back out of header `$30` after measuring a
  string through output stream 3 is the width that string will occupy. Games
  right-align from that number, so declaring one thing and drawing another put
  Arthur's date field thirty pixels past the end of its own score bar. Setting
  `interpreter_number` yourself names the machine outright and outranks both, so
  `interpreter_number = 4` gets you the Amiga's palette, its standard window and
  its §8.3 screen rules rather than just the byte — a number that changed what
  games did without changing the machine it implied was never a useful thing to
  be able to set.

  **Its default page and ink are the one part that waits to be asked for**
  (SQ-0928). A machine's `$2C`/`$2D` pair describes a *machine*, and running a
  story off its release disk makes that description true of the launch — so off
  media it applies with no flag at all. Typing a number does not: add
  **`--colour machine`** (or `system_colours = true`) when you have named one and
  mean the whole machine. The reason is the IBM PC, which states blue under white
  and is also what every story with no medium falls through to; without the
  distinction, opening any modern Inform game would paint it blue.

  **And the flag runs the other way too** (SQ-1154). `--colour` names which
  *regime* a launch is in rather than which rung of a chain it starts at, and the
  two halves are mirror images: `--colour machine` gives a bare story file the
  media path, and `--colour theme` or `--colour terminal` gives a release floppy
  the raw one. Ask for your terminal's colours off an Amiga disk and the launch
  simply does not present its machine — no §8.3.3 pair, no two-colour card, and
  colour numbers resolving through §8.3.1's own table rather than the Amiga's,
  which is the whole of what a bare file does. It has to be all of those together:
  snapping your terminal's grey to the nearest *standard* number and then reading
  that number back through the *machine's* palette reports a colour that is not
  your terminal's, which is exactly what the flag was asked for. What does not
  move is the ARTWORK — a plate resolves through the palette stored beside it in
  the archive, so the disk's pictures look like the disk's pictures whichever
  regime you are in — and the story's own `set_colour`, which is still obeyed
  unless you turn `--game-colours off`; it simply resolves through the table the
  regime names.

  **A regime withholds the machine's screen RULES, not only its colour values.**
  Two machines treat their `$2C`/`$2D` pair as the screen itself rather than as
  advice about one — the Amiga, whose two "pens" are shared by every window
  (§8.3), and the Macintosh, whose white page under black ink is what a Mac window
  *was*. Leave those rules live under a host regime and the pair becomes the
  ground, and a pair can only be a colour *number*: your terminal's `#1A1B26`
  snaps to the nearest standard one, which is pure black, and that is what gets
  painted. So a host regime turns the rules off with the values, and the pane
  keeps your terminal's real background — exactly as it does on the Atari ST and
  the IBM PC, which claim no screen page of their own and never had this problem.
  A host Save State does not carry a regime across, either: restore a
  `--colour machine` save under `--colour terminal` and the page and ink are the
  regime *this* run was launched in, because `--colour` is a flag of the run doing
  the showing.

  **And the card it is showing is not the machine.** The IBM PC's blue belongs to
  a full-colour screen; put a CGA plate in front of it and the display is two
  states, black under light grey — the exact inverse of what *Zork Zero* asks for,
  and visible in `machine-screenshots/dos-zorkzero-cga.png`. So the profile states
  a second pair for its two-colour display, differing from the first in the page
  alone, and *that* is what a launch off a `.CG1` reports in `$2C`/`$2D`: black 2
  under white 9, with white resolving to the card's `#AAAAAA` rather than the
  full-colour screen's `#FFFFFF`. The Macintosh's monochrome plate states the same
  pair its machine already states, so nothing about it moves.

  Which card is showing comes from the ARCHIVE's container, because nothing else
  can say: a `.CG1` is a card, an Amiga/Mac `Pic.data` is not, and the same
  `EF_MONO` flag is set on both. See
  [a two-colour card takes one bit](v6-graphics.md#a-two-colour-card-takes-one-bit).

  You can set it per game as well as globally. The
  [launch-options dialog](v6-graphics.md#three-ways-to-say-it) — **Shift-Enter**
  on a story in the picker — shows the number your art choice implies *and where
  it came from*, lets you pin a different one for that launch, and will write
  `interpreter_number` into the game's own `config.toml` if you tick the box.
  Most specific first: the dialog's choice for this launch, then
  `--interpreter`, then the game's sidecar, then the global config, then
  the inference above. It belongs in a *launch* dialog rather than the settings
  screen because header byte `$1E` is read by the story itself at boot — a game
  that has already started has already made decisions from it, so offering to
  change it mid-session would be offering something lanthorn cannot deliver.

  Authenticity can cost readability — *Zork Zero* under an Amiga picks a colour
  scheme that was easy on a 1989 monitor and is merely adequate in a modern
  terminal. There is no separate switch for that on purpose: `honor_game_colours`
  already decides whether the game's colour choices are honoured at all, so
  turning it off hands the screen back to your theme, profile or no profile.
  (`period_look` is *not* that switch. It governs a v1–v4 story, which has no
  colour choices for this paragraph to be about — see
  [The period look](#the-period-look).)

  **The Amiga had two pens, and moving one repaints the screen.** This is the one
  place where claiming to be an Amiga changes not just what a game is *told* but
  what happens when it acts on it, and the standard is blunt about it. Version 6
  normally gives every window its own foreground and background — eight windows,
  eight pairs — but ZMSD §8.3 carves out this machine: a Version 6 interpreter
  going under the Amiga interpreter number "must use the same pair of colours for
  all windows when running Infocom's games", and if either colour changes it "must
  change the colour of all text on the screen to match". The reason is hardware.
  The Amiga drew text through two colour *registers* and changed a colour by
  reloading the register, so every glyph already on the display changed with it —
  there was no way to give one window, or one word, a colour of its own.

  lanthorn does exactly that. Under interpreter 4 a `set_colour` **from window 0**
  loads the machine's two pens, every window adopts them, and every glyph already
  drawn — status grids, the pixel-positioned labels on *Zork Zero*'s banner
  ribbons, the prose a window has scrolled, even prose left frozen behind a window
  that has since moved — is repainted in them. *Zork Zero* is the title that shows
  it off: it boots black-on-light-grey on its story window and the whole screen
  goes with it.

  **And a `set_colour` from any other window is ignored** — which is the one place
  lanthorn deliberately departs from the letter of §8.3, so it is worth saying why.
  The standard does not mention such a gate; Infocom's own released Amiga
  interpreter does, in as many words: it changes text colours *"only in window 0,
  and ignore[s] requests in other windows (except for the special case of
  bg = -1)"*. §8.3's stated purpose is to **simulate the Amiga hardware**, so a
  reading of it that makes lanthorn diverge from that hardware defeats the rule's
  own reason for existing — and Infocom's interpreter is the better authority on
  how Infocom's games looked on Infocom's machine. *Journey* settles it: its Amiga
  release (30 / 890322) makes exactly one `set_colour`, asking for white on black,
  and makes it on window 3. Applied globally that paints the game black; real
  Amiga captures show *Journey* on grey with white text instead — the Amiga's
  *default* pair, `DEF_BACK` over `DEF_FORE 9`. The real machine dropped the call,
  and so does lanthorn. (If you are ever tempted to "correct" this back to the bare
  text of the standard: that is the change, and this is the paragraph explaining
  why it was not made.)

  **And the floppy outranks the leaked source.** lanthorn took the Amiga's numbers
  from `amiga/yzip1.c` and `amiga/yzip.h` in Infocom's leaked interpreter sources,
  which are a *development* snapshot. In two places they disagree with what
  Infocom actually pressed onto the disks, and the second of the two is the whole
  screen:

  | constant | leaked source | on every release floppy |
  |---|---|---|
  | `colortable[5]` — standard colour 5, yellow | `$0EE0` | **`$0FD0`** |
  | `DEF_BACK` — the page every Amiga game is played on | 11, medium grey `$777` | **12, dark grey `$444`** |

  Each Amiga disk in `stories/` carries its own 68000 interpreter beside the
  story, and those programs are the authority: they are what painted the screens.
  `set_back()` opens `if (id == 1) id = DEF_BACK;` and compiles to
  `cmpi.w #1,d7` / `bne.s` / `moveq #12,d7` in all four; `set_color()`'s
  `return ((DEF_BACK << 8) | DEF_FORE)` assembles to `move.w #$0C09,d0` in all
  four; `$0B09` occurs in none of them. Real captures agree — a *Journey*
  release‑30 screen tallies 173,994 pixels of `#444444` under 25,878 of `#FFFFFF`,
  and an *Arthur* church screen is `#444444` under `#FFFFFF` with the status bar
  *reversed* to `#444444` on `#FFFFFF`, which is pens 0 and 1 swapped and so proves
  the page is the text background register rather than artwork.
  `crates/app/tests/suites/v6_amiga_shipped_interpreter.rs` reads all of this back
  off the disks on every run, precisely so that a future reader who reaches for
  `yzip.h` is told by a failing test that the machine disagrees. (SQ-0822.)

  **On this machine, a bracketed line is not a message from the interpreter.**
  lanthorn normally mutes a whole line in `[brackets]` in the transcript, on the
  reasonable guess that it came from the interpreter rather than the story. Under
  §8.3's Amiga that guess is wrong twice: *Arthur*'s
  `[You have earned ten chivalry points.]` is the game's own prose in the game's
  own pens, and the muted colour was chosen to recede against your *theme's* page,
  not against the machine's dark grey — where it reads as grey on grey. So the
  rule stands down while the machine owns the ink. Your own `[transcript.rules]`
  entries are unaffected (they are explicit, and they always win), and so is the
  room-heading highlight, which paints an accent rather than a mute and stays
  legible on any page.

  **The machine's default pair is painted, not merely advertised.** §8.3.3 has an
  interpreter write its own default background and foreground into header bytes
  `$2C`/`$2D` so the story can read them, and lanthorn has always written the
  Amiga's. Under interpreter 4 those two bytes are also the *screen*: on real
  hardware they are the registers, so every pixel no picture and no `set_colour`
  claimed is the background pen. So they are what lanthorn paints with too — the
  page under the frame, the ink of any text that named no colour of its own. That
  is what makes an Amiga *look* like an Amiga rather than merely report as one:
  *Journey* on its release floppy is white text on the machine's dark grey, frame
  and menu and prose alike, instead of your terminal's own colours.

  **The Macintosh needed the same thing, and found out the same way.** *Zork
  Zero* off its Mac disk never calls `set_colour` even once — the game asks a
  Macintosh for nothing — so every window sat at "default", and with nothing
  painting `$2C`/`$2D` the whole screen fell through to your theme. The visible
  symptom was the status banner: location and score drawn in the theme's grey on
  the game's own white plate, on a two-colour machine that has no grey in it
  anywhere, which reads as text that failed to render. A Mac window was white
  with black type, Infocom's own interpreter says so in one line, and that is
  now the page lanthorn paints. There is no claim about shared pens here — that
  part is the Amiga's alone; a Mac `set_colour` still colours one window, exactly
  as §8.3 describes. This is only the ground beneath a window that asked for
  nothing, and `honor_game_colours = false` still hands it back to your theme.

  **What you are typing stands on that page too.** The line you are composing is
  drawn by lanthorn rather than by the story, and it used to resolve its ink from
  your theme alone — which on a machine page is a coin toss. On the Amiga it won
  the toss, because the theme's body ink is white and so is `DEF_FORE`; on the
  Macintosh it lost it completely, and typing into a white Mac page was typing in
  white on white. You could not see a word until you pressed Enter, whereupon the
  game echoed the command back as prose and it appeared, in black. So the live
  echo now stands on the same ground the committed text does: the machine's own
  pair, the same characters rendering the same way whether you have pressed Enter
  or not. A game that *asks* for colours with `set_colour` still wins over the
  machine's defaults, exactly as it always did — and so does a `style.toml` that
  names `input_text` or `input_prompt` by hand, because the machine's page is a
  default and anything you declare outranks a default.

  Two things the rule deliberately does *not* do. Colour **-1**, "the colour of the
  pixel under the cursor", names no colour, so it loads no pen — it stays a
  request to draw over what is already there, which is how *Zork Zero* prints its
  banner labels straight onto the ribbon artwork (and it is the one request a
  window other than 0 may still make). And a pen carries ink and page both, but a
  page nobody ever laid down is not a pixel a pen can reach: a window the game
  never gave a background keeps painting nothing behind its glyphs, or a single
  black `set_colour` would paint *Journey*'s own illustration out of its frame.
  Everything else — every non-Amiga profile, and any profile at all with
  `honor_game_colours` off, where lanthorn has told the story it has no colours to
  offer — keeps one pair per window and the host theme's own page, exactly as §8.3
  describes for every other machine.
- **v6 graphical stories** — lanthorn boots and plays graphical v6 titles,
  verified against *Zork Zero*'s full frame. On an image-capable terminal
  (Kitty / iTerm2 / Sixel) the game's chrome — the decorative frame, status
  line, and per-room compass — renders as one scaled, **pixel-aspect-accurate**
  image (uniform scaling, letterboxed, never stretched); the game itself lays
  this out by querying invisible "placement" pictures, which lanthorn answers
  from the Blorb's own dimension data. The `v6_render` setting (see
  Customization) picks how the story text is drawn: the default `hybrid` mode
  keeps it as real, crisp terminal text inside the chrome; `raster` bakes it
  into the pixel image instead, bitmap-font style. Without an image protocol,
  v6 falls back to a character-cell rendering. Full depth — the three render
  modes, inline drop-caps, pixel-positioned status text and colour — is in
  [Graphical v6](v6-graphics.md). (v6's menu and mouse opcodes are not yet
  wired up.)

## Glulx

- **External files** — Glulx games persist their own data through Glk file streams;
  a game's fixed-name saves and caches are read and written for it silently. (See
  [saves](saves.md) for how this dovetails with lanthorn's Save States.)
- **Accelerated-function interception** — big Glulx games reach the first prompt
  dramatically faster. Well-known Inform veneer functions the game registers via
  `accelfunc` are recognized and run natively instead of grinding through full VM
  dispatch, so a heavyweight like Counterfeit Monkey stops making you wait through
  its startup. On by default; disable with `--accel off` (`gvm-cli` and the app).
- **Fingerprinted acceleration for games that never ask** — a game only gets the
  interception above if it *registers* its veneer, which Inform 7 has done since
  build 6E59 (2010) and nothing older ever does. Every plain Inform 6 game, and
  every Inform 7 game before that, interprets those routines one opcode at a
  time. `crates/gvm/src/veneer.rs` finds them anyway, by matching the story's own
  ROM against a committed template of the seven routines before the first opcode
  runs. King of Shreds and Patches' `inventory` turn goes from 43.0M dispatched
  opcodes to 3.5M — **12.5x**, and 10.5x in wall time.

  What is matched, and what is checked:

  - **The bytecode, byte for byte**, outside a mask that covers exactly the
    operands whose bytes are image-specific — memory references and RAM-relative
    operands (addressing modes `5/6/7` and `D/E/F`), call targets, run-time-error
    message addresses, and the operands carrying the nine `accelparam` constants.
    Opcode numbers, addressing-mode nibbles, branch offsets, local offsets and
    every genuine constant must be identical. Nothing fuzzy; no partial matches.
  - **Uniqueness** — a routine that matches at two addresses in ROM is an
    ambiguity, and acceleration is refused rather than guessed at.
  - **Call-graph closure** — the seven must call *each other*: the `RA__Pr` we
    matched has to call the `CP__Tab` and `OC__Cl` we matched, `RV__Pr` has to
    call that `RA__Pr`, and all three `RT__Err` call sites have to name one
    function. This is what makes the match a statement about the game's veneer
    rather than about a body that merely looks like one.
  - **The nine parameters**, read out of the matched operands, must agree wherever
    the same one appears twice (`class_metaclass` is read in four routines), and
    the `indiv_prop_start + 5/6/7/8` constants must be exactly that.
  - **Cross-checks against the object table** — facts the bytecode cannot supply:
    `classes_table`'s first four entries must be the four metaclasses in order;
    those must be objects by `Z__Region`'s own test and evenly spaced; the stride
    must leave room for the class-chain field at `13 + num_attr_bytes`; and
    `classes_table[4]`, the first genuine class, must carry `class_metaclass` in
    exactly that field. That last one is what validates `num_attr_bytes` against
    the object record rather than against the `aload` index it was read from.

  If anything fails, **nothing** is installed and the routines are interpreted
  exactly as before. `--accel off` disables the whole thing, fingerprinted or
  declared.

  **Reading the result.** `Machine::veneer_accel()` returns a `VeneerReport`;
  `report.summary()` is one line naming the template, each routine and its
  address, and every derived parameter — for example

  ```
  veneer acceleration: Inform 6.31-6.41 (BlueLacuna.gblorb, serial 100717)
  [Z__Region@0xed746 CP__Tab@0xed840 RA__Pr@0xeceeb RL__Pr@0xecf6f OC__Cl@0xed05c
   RV__Pr@0xecc6b OP__Pr@0xecff9] params classes_table=0x2420c3 indiv_prop_start=256
   class/object/routine/string_metaclass=0x1e8880/0x1e88a0/0x1e88c0/0x1e88e0
   self=0x1c9910 num_attr_bytes=7 cpv__start=0x241fdf
  ```

  — and when nothing was installed it says why (`not applied (no template match
  for Z__Region, …)`, `cross-check failed: …`). It is deliberately *not* pushed
  to `Machine::diagnostics`, which the app turns into Warning lines in the
  player's transcript. The fingerprinted assignments also flow into
  `accel_funcs()`, so the disassembler badges those routines as accelerated
  exactly as it does a game's own.

  **Provenance and coverage.** The template is the veneer of `BlueLacuna.gblorb`
  (Inform 6.31, serial 100717), which registers its own and so states the ground
  truth. Across the 35 stories in `stories/` that register — Inform 6.31 through
  6.41 — the seven routines are identical instruction for instruction, and
  `fingerprint_agrees_with_every_story_that_declares_its_own` re-derives every
  address and every parameter from bytecode alone and checks the answer against
  what each game goes on to announce.

  **Inform 6.21 is not covered, on purpose.** City of Secrets, `advent.blb`,
  `narco.blorb`, `photo201.blb` and `sensory.blorb` all use a different codegen —
  no `jgeu` or `callfi`, and `Z__Region` calls an `Unsigned__Compare` helper —
  and, decisively, their `CP__Tab` omits the `Z__Region` guard that Glulxe's
  `accel.c` performs, so the native routine is *not* a drop-in for the
  interpreted one on a non-object argument. There is also no 6.21 story in the
  corpus that registers, so there would be no ground truth to check a template
  against. Matching refuses them, which is the correct outcome.
- **Floating-point math** — the complete float opcode set is implemented, in both
  single **and** double precision: conversions, arithmetic, `sqrt`/`exp`/`log`/
  `pow`, trigonometry, and the fuzzy comparisons `jfeq`…`jisinf`. Games that
  compute with floats — Counterfeit Monkey's in-game graphics scaling, say — run
  instead of faulting, and the `gestalt` opcode answers `Float` and `Double`
  truthfully so a game can probe first.
- **Line-input terminators** — lanthorn honors `glk_set_terminators_line_event`, so
  a game can register special keys (Escape and the function keys `Func1`–`Func12`)
  that end a line of input; the terminating keycode comes back in the line event's
  second value (`val2`; `0` for a normal Enter).
  `glk_gestalt(gestalt_LineTerminators/LineTerminatorKey)` answers truthfully so
  games can check before relying on it.

## Sound

- **Z-machine** — the `sound_effect` opcode's two built-in bleeps (#1 high / #2 low)
  play as real synthesized tones, and Blorb `Snd ` resources (#≥3) play as sampled
  audio (AIFF, Ogg, or ProTracker MOD), in both the `app` TUI and `zvm-cli`. Sound
  resources come from the story file itself if it's a Blorb, else from a sibling
  `.blb`/`.blorb` next to it. On every bleep the story-pane border also flashes in
  a distinct, themeable colour (`sound_beep_high` / `sound_beep_low`) — a
  complementary and accessibility cue, and the *only* cue when sound is off.
  Controlled by `enable_sound` (default on) and `volume` (0–100, default 100);
  toggle it with `/toggle-sound` or the `F2` settings row, adjust it with
  `/volume <0-100>`, and use `/play-sound <resource-id>` to fire a Blorb `Snd `
  resource on demand for verifying the audio path. Both the `app` and `zvm-cli`
  take `--sound off` to start muted for a single run (leaving `enable_sound`
  untouched); `zvm-cli` also takes `--volume <0-100>`.
- **Straight off the original floppy** — the two Infocom games that ever used sound,
  *The Lurking Horror* and *Sherlock*, shipped their effects as raw Infocom sample
  files on the release disk, years before Blorb existed. Mount one of those disks and
  lanthorn plays them: no `.blb` beside the story, no conversion step, nothing to
  fetch. **The disk wins over a `.blb` filed beside it** — the same way artwork
  already resolves, and for the same reason: the disk is the rendition Infocom
  pressed, and a Blorb is somebody's later re-rendering of it, sometimes at
  audibly different pitches. `/play-sound` says which source answered, and names
  a Blorb that is present but outranked rather than leaving you wondering.
  It reads the disk's own index rather than guessing from filenames — which
  matters, because *Sherlock*'s samples are called `armor`, `growl` and `violin.bin`,
  and three separate effects share one `heart` recording. Both the Amiga floppies and
  the Macintosh `/MAC/SOUND` layout of the *Lost Treasures* CD are understood, and
  `/play-sound <n>` fires them the same way it fires a Blorb resource.
  **And the pitch comes with them.** Each effect names a tiny MIDI file saying which
  note to sound, and each sample states in its own header the note it was recorded at;
  the gap between the two is the bend, in equal temperament. That is why *Sherlock*'s
  heartbeat beats at three different speeds from one recording — the model was read out
  of the 68000 interpreter Infocom shipped, and it reproduces two independent
  third-party renderings of these sounds on 27 of the 29 effects they carry.
  The two machines also disagree about where silence sits in a sample byte, and the
  header does not say which — so the layout decides it, checked against the one effect
  both discs press from the same master. The Mac goes further and fades each sample in
  and out from its speaker's rest position, which lanthorn unwinds rather than
  reproduces: played back literally on a modern output that ramp is a click at each
  end.
- **Glulx** — Glk sound channels (`glk_schannel_*`) play a Blorb's AIFF/Ogg/MOD
  `Snd ` resources with per-channel volume (including gradual volume ramps) and
  sound-finished notify events, so music and effects behave the way the author
  wired them.

Sound always plays on the local device lanthorn runs on; to route audio from a
remote/SSH session back to your own machine, see
[`docs/internals/remote-sound.md`](remote-sound.md). Unimplemented-opcode warnings
surface in the transcript as meta lines (hidden by `/filter story`) rather than
spilling onto stderr.

## Game-driven colour

When a game asks for colour, lanthorn gives it colour — on your terms. The
Z-machine's v5+ `set_colour` and `set_true_colour` are honored: the standard
palette (black/red/green/…) maps onto *your* colour scheme, so a game's "red" is
your red rather than a hard-coded shade, while greys and true-colour render as
exact 24-bit RGB. Colour and reverse-video apply in both the transcript and the
upper-window grid. **Glulx/Glk** games get the same treatment —
`stylehint_TextColor`/`BackColor`/`ReverseColor` render at full 24-bit fidelity.

It all sits under one switch, `honor_game_colours` (default **on**): flip it in the
F2 settings screen to let your theme own every colour instead, per game with
`/set-game-colours on|off|auto`, or for a single launch with **`--game-colours off`** —
one spelling across all three players, since `zvm-cli` and `gvm-cli` render the same
colours as ANSI SGR and have always accepted it (they also honour `NO_COLOR` set to
a non-empty value).

The flag is an instruction for the launch you typed it on, so it outranks the two
things that otherwise speak for a story — a `garglk.ini` sitting beside it, and an
`honor` key in the game's own sidecar — exactly as `--interpreter` outranks that
same sidecar. Nothing is written back to your config: probing one game with colours
off cannot leave every later launch monochrome. And it is not a lock — a
`/set-game-colours` while you play is you overriding your own flag, and wins.

One thing turns it off for you, and only when nothing else can speak. A game
drawing **two-colour (CGA) artwork** off no medium at all — a bare `.z6` with
`--pictures zork0.cg1` — is told the interpreter has no colours, because in that
launch it genuinely has none to state: that artwork is a stencil whose own paint
is opaque and whose transparency is meant to show your background through, and a
story that thinks it is on a colour display paints over both. Off a real DOS press
the card states a screen instead and the colours stay ON, which is what *Zork
Zero*'s in-game `color` command needs — see
[a two-colour card takes one bit](v6-graphics.md#a-two-colour-card-takes-one-bit).
Either way it applies to that story only and is never written back to your config,
so choosing a `.cg1` once cannot quietly strip the colours from every other game.

## The period look

Colour arrives with **Version 5**. `set_colour` and the `$2C`/`$2D` header bytes
are v5-and-up, so a Version 1–4 story has no colour concept at all: it never
sets one, never reads one, never branches on one. Everything above this section
is about a fact a story can read. This is about the other thing — what the
*screen* looked like — and for a v1–v4 story that is all there is.

Open *Zork I* off a Commodore disk or *Spellbreaker* off an Amiga floppy and
lanthorn dresses the story pane as that machine's own interpreter dressed its
screen: its page and its ink, its status line, and the shape of its cursor. It is
on by default (`period_look`, in the F2 settings screen right below
`honor_game_colours`), and it applies only where a machine is actually named —
off a release disk, or when you set `interpreter_number` yourself.

**Nine machines across seven rows, and not one of the three decisions follows
from the others.**

| # | machine | page / ink | status line | cursor |
|---|---|---|---|---|
| 2, 9, 10 | Apple II | `#000000` / `#FFFFFF` | full-width reverse | block |
| 3 | Macintosh | `#FFFFFF` / `#000000` | no ground at all — **rules** | 1px bar, between glyphs |
| 4 | Amiga | `#074BA1` / `#FFFFFF` | full-width reverse — measured per run | block, `#FF7E1C` |
| 5 | Atari ST | `#FFFFFF` / `#000000` | full-width reverse | block |
| 6 | IBM PC | `#0000AD` / `#ADADAD` — *resolved, see below* | full-width reverse — measured per run | underscore |
| 7 | Commodore 128 | `#000000` / `#55FFFF` | full-width reverse | underscore |
| 8 | Commodore 64 | `#000000` / `#FFFFFF` | full-width reverse | underscore |

One caret in that column is a colour that is neither its machine's page nor its
ink — the Amiga's orange — so the cursor cannot be built out of the pair. Neither
can the status line: the Macintosh sets its row apart with no ground at all where
every other machine here reverses the body pair, and the captures are wider still
than the table, the 1984 Commodore 64 drawing a band of grey on black that is
neither the body pair nor its reverse.

**And two of these machines do not reverse the whole row.** On
`amiga-spellbreaker.png` the reversal sits behind "Council Chamber" and behind
"Score: 0/0" with 376 pixels of plain blue page between them, and
`dos-hitchhiker.png` runs 611 pixels of page through the middle of its own.
lanthorn draws both bands whole, on the user's ruling: a band broken into pieces
reads as damage in a terminal where it read as design on a 1989 monitor. The
column above is what lanthorn draws — the measurement keeps its own record, in
`StatusBand::PerRun`, which no row now uses.

**These are observations, not sources, and that is a real difference.** Every
other value on this page is quoted at its constant out of Infocom's own
interpreter — `st/stx1.s`, `zboot.asm`, `mac/xzip.lst`. These were measured off
emulator captures in `machine-screenshots/`, row by row. Two of them are values a
palette choice could move (the Amiga's `#074BA1` is almost certainly Workbench's
register `$05A`, which bit-replicates to `#0055AA`; the Commodore 64's greys
depend on which VIC-II palette you believe). Three cannot move at all: the Mac
Plus is 1-bit, the C128's VDC is RGBI, and the Apple II is monochrome — though
"monochrome" there means the *white* monitor, and green and amber were as common.
And two are the capture corrected back to the colour the machine actually names,
because the emulator dimmed it: the Atari ST's page is plain white rather than
`st-zork1.png`'s `#EBEBEB`, which is a scanline filter and not a shade the ST has,
and the IBM PC's pair is its adapter's own digital entries rather than
`dos-hitchhiker.png`'s scaled `#0F009E` — a blue carrying `0x0F` of red that no
entry on that card has.

**And one row is not stored at all.** The IBM PC's screen *is* its own palette
rendering the pair it reports in `$2C`/`$2D`, so lanthorn resolves it rather than
keeping a second copy — the page a v1–v4 story is painted on and the colour a
later one gets from `@set_colour(6)` are then one lookup, and cannot come out as
two blues three parts apart where two windows meet. The adapter's own entries are
EGA 1 `#0000AA` and EGA 7 `#AAAAAA`, and that is still the truth about the
hardware; the table above shows them as a *story* gets them, through the 15-bit
colour space described under **One machine table, two front-ends** in the
Z-machine section above. The row could not have held a constant in any case,
because that ink depends on the story's Version — see the caret note below.

**The caret in that table is the v1–v5 one, and on two machines Version 6 moved
it.** Infocom's later interpreters draw the caret as the pair *on screen*
reversed — it follows whatever colour the story has set rather than having one of
its own — and the captures show it twice over on one machine:
`amiga-zorkzero.png` draws a black block after `[MORE]` on *Zork Zero*'s grey
page, and `amiga-shogun.png` a white one after the `>` on *Shogun*'s dark page.
Neither is the `#FF7E1C` orange the same machine's Version 3 interpreter draws.
`dos-arthur.png` says the same about the IBM PC, and changes shape rather than
colour: a solid white cell after `>exam` where its v3 capture draws an
underscore. So the Amiga and the IBM PC get lanthorn's ordinary reverse-video
caret on a v6 story — which is not a fallback here but the exact behaviour — and
every other machine keeps the caret its own capture measured. The Macintosh is
the control that makes this per-machine rather than per-version:
`mac-zorkzero.png` and `mac-shogun.jpg` draw the same 1px bar its v3 frame does.
(Version 6 moves the IBM PC's *ink* as well as its caret: its Version 6
interpreter renders white as a true `#FFFFFF`, where the v1–v5 one draws the
`#ADADAD` the table above records. That is the other half of why the row stores
no pair — one constant cannot be two colours, so the ink is resolved for the
Version in hand.)

**A period look is a property of the interpreter build as much as of the
machine.** Two Commodore 64 captures three years apart disagree on all four
decisions: the 1984 *Hitchhiker's* is white on a grey page with a status band of
grey on black — neither the body pair nor its reverse — and the 1987 Solid Gold
*Zork I*, whose banner reads "Interpreter 8 Version J", is white on black with a
plain reverse. The table has one row per machine and takes the later press.

### What it will not do

- **Anything you styled yourself wins.** A selector named in your `style.toml`,
  in a `garglk.ini` found beside the story, or in the game's own sidecar is a
  *choice*, and a choice outranks a machine default — per selector, and counting
  the role it inherits from. Theme your transcript and the Amiga floppy will not
  take it back.
- **Nothing outside the story pane.** The map, the dialogs and the rest of the
  chrome are lanthorn's, not the machine's. A Commodore's page across the whole
  application would be dressing up rather than presenting.
- **`honor_game_colours = false` takes it with it.** That switch is the broad
  one: you have said "keep my terminal's colours", and painting a blue Amiga page
  over you would be arguing. It does not work the other way — `period_look` is the
  narrow key precisely so that declining the *presentation* never costs a v5+
  story the colours it asked for.
- **Never for v7 and v8.** No Infocom machine ever shipped an interpreter for
  them, so there is no period screen to have measured. Everything from v1 to v6
  *is* dressed: the machine's measured RGB and the `$2C`/`$2D` numbers a v5+ story
  reads are two spellings of one row, so painting the first while answering the
  second is not a lie about the screen — it is the screen. (The **status line** is
  the one part that still stops at v4, because that is where the game gains
  `set_colour` and starts naming that row's colours itself.)

### What a terminal cannot say

The measurements are in pixels and lanthorn draws in cells. The bar and the
underscore become the glyph that occupies the same part of the cell — `▏` and
`▁` — and where the caret sits *on* a character the shape stands down entirely,
because the character has to stay readable while you edit it. The Macintosh's
rules become one row of underline, which is a terminal's whole horizontal-rule
vocabulary.

`zvm-cli` has the opposite trade and does better on the cursor. Ask for it with
**`--period-look`** — **off** by default, because that is your terminal and not a
pane lanthorn owns, which is the same reasoning that makes the IBM PC decline a
default colour pair. The page and the ink go through OSC 11 and OSC 10, setting
the terminal's *own* defaults so that every `ESC[0m` a styled run ends with
returns to the machine's pair instead of dropping it; the cursor goes through
DECSCUSR, which states the real shape rather than an approximation. What the CLI
cannot say is the cursor's *colour*: DECSCUSR carries a shape and nothing else.
`--game-colours off` and `NO_COLOR` suppress the whole thing, as they should.

## Plain text, for screen readers

All three CLIs accept **`--screen-reader`** (alias `--plain`), and select it
automatically under `TERM=dumb`. It emits no escape sequences at all: no colour,
no cursor addressing, no scroll region, no pinned status line, no alternate
screen — just linear, append-only text a screen reader can follow and scrollback
can review. `[MORE]` paging goes too, since a blocking prompt that hides the rest
of the output behind a keypress is the shape a reader copes with worst. Line
editing and echo go back to the terminal, so the reader announces typed
characters and the user's familiar editing keys work.

What would otherwise be spatial arrives in reading order instead: the Z-machine
status line and upper window come through as ordinary lines, and Glk TextGrid
windows stream inline, deduped so an unchanged status bar doesn't repeat every
turn. **Menus** get more than that — see below.

**The status line is not narrated every turn.** A Z-machine v3 status line
carries a move counter, so it differs on every single turn and no amount of
change-detection will suppress it — measured, Ballyhoo repeats it on four turns
out of four. Screen-reader mode therefore leaves it out and lets you ask with `/status`.
`--show-status` puts it back if you would rather have it whenever the story
updates it.

The suppression goes by *size*, and only a one-row region is treated as chrome.
Anything taller is content the game means you to read: the Infocom releases with
integrated InvisiClues draw their hint menus in the upper window — Planetfall's
is twelve chapter headings and a `RETURN = See hint / Q = Resume story` legend —
and Lost Pig's HELP menu and Bureaucracy's licence-application form are the same
shape. Those always come through. **`--story-only`** is the blunt instrument for
anyone who wants the whole upper window gone, menus included — it is deliberately
a separate, stronger switch, and it works with or without `--plain`. `gvm-cli`
takes it too, where it suppresses every Glk grid window. (Scott has no status
window to suppress: its room block *is* the story.)

The status also lands in the right place. A game writes its prompt last and
without a trailing newline, and the host only learns the turn is over when the
game asks for input — so a naive host can only append the status *after* the
prompt, giving `> In the Wings   Score: 0`, which reads as though the prompt were
showing you a room. In this mode the prompt is held back until the status has
gone out, so a turn reads description, then status, then prompt. `/status`
answers the same way, and puts the prompt back after itself.

`scott-cli` drops its em-dash divider rule in this mode. It stands in for the
boundary a real Scott terminal drew between its two windows, and a reader either
announces thirty-odd em-dashes one at a time or swallows the line — neither of
which conveys a boundary.

### Menus are numbered, and a move is one line

A menu is a rectangle the game repaints: a list with a `>` parked on the current
item and a legend saying which keys move it. Sighted, the marker jumps and
nothing else happens. Linearised, *every repaint is a fresh block of text* —
measured, `N` at Planetfall's InvisiClues menu read out sixteen lines, and
Arthur's read out twenty-three, on every single press, to say that a `>` had
moved down one row. Followable, but not usable.

So in screen-reader mode the host recognises the repaint. A menu is read out
**once**, host-numbered:

```
                               INVISICLUES (tm)
 N = Next                                                     P = Previous
 RETURN = See hint                                        Q = Resume story

[menu — type a number to jump, Enter to select]
>1. THE FEINSTEIN
 2. THE POD TRIP
 3. THE DORMITORY
 …
```

and after that a marker move is announced in one line:

```
>3. THE DORMITORY (3 of 12)
```

**Detection is a mechanical diff, not a guess about content.** The host keeps the
last block emitted from each source (the Z-machine upper window; each Glk grid;
the Glk story stream) and compares. If two blocks differ *only* in where the
marker sits — same items, same headers, same legend — that is navigation. Any
other difference is content and is emitted in full, unchanged. A status line
whose text changed, a menu that scrolled, a form that gained a field: all differ
somewhere other than the marker column, so none of them is ever swallowed. This
is the whole safety argument, and it is pinned by tests on both engines.

**Which lines get numbers** is decided by shape: an item is a non-blank line
whose text begins at the same column as the marked line's, with nothing but
blanks and marker characters in front of it. That is exactly the items in all
three measured menus and none of their furniture — Arthur's centred title
(column 20), its `N = next item` legend (column 1) and its `(more)` pagination
hint (column 4) are all left unnumbered, as are Planetfall's title and two
legend rows. The rule errs towards numbering more lines rather than fewer: an
over-numbered header is an annoyance, an unreachable item is a dead end. (The
alternative — numbering only the lines the marker has been seen on — renumbers
the menu under the player as they explore it.) A list the game repaints twice
into one block, as Counterfeit Monkey's does, counts once.

**Typing a number jumps to that item.** The host cannot teleport the marker — the
game owns it — so it walks the menu with the game's own keys: `n`/`p` when the
legend names them (`N = Next`, `P = previous item`), else Down/Up (ZSCII 129/130
for the Z-machine, `keycode_Up`/`keycode_Down` for Glk). It steers rather than
counting: press, read where the marker actually landed, decide again — because
Arthur's `N` steps straight over its unselectable section headings, and a
press-count worked out in advance would sail past the item you asked for. The
landing is announced in the move format; the intermediate steps are silent; and
an ordinal the marker will not stop on gives up and reports where it ended
instead of pressing forever.

Numbers are only intercepted while a menu is open, and only for an ordinal the
menu actually has. Everything else — `n`, `p`, Enter, `q`, a digit at an ordinary
prompt — reaches the game untouched.

**`/menu`** re-reads the open menu, numbered, on demand, and says
`[no menu is open]` when there isn't one. It is the `/status` precedent, and
because screen-reader mode leaves the terminal cooked, a menu "keypress" is
really a whole line terminated by Enter — so `/menu` and multi-digit jumps work
at a menu's own prompt, not just at a line prompt. (That termination rule is not
a choice: it is the shape of the read. Raw mode would deliver `1` then `2` with
no way to tell `12` from item 1 followed by item 2.)

None of this applies outside `--screen-reader`. On a terminal the menu is painted
in place and nothing repeats; on a plain pipe the output is a transcript that
stays byte-identical.

### Score changes are announced

Quietening the status line takes the score with it, and the score is the part
that carries news — a sighted player watches it tick over, a listener would have
to keep asking. So in screen-reader mode a score that *moves* is announced above
the prompt:

```
You put the gold idol on the pedestal.

[Score 1, up 1]
>
```

Only on change, never on the first sighting (the score you started with is not an
event), and words rather than `+1`, because a reader announces "plus" only at
higher punctuation settings.

Where the number comes from differs sharply by format, and two of the three are
exact while the rest is pattern-matching:

| | source | |
|---|---|---|
| Z-machine v1–v3 | global 2, which the standard reserves for the score (ZMSD §8.2) | exact |
| Z-machine v4+ | the status line the game drew | recovered from text |
| Glulx | the Glk grid window — Glk has no concept of a score at all | recovered from text |
| Scott Adams | treasures deposited in the treasure room, recounted each turn | exact |

The text-recovery cases look for a `Score: N` field and take the last one on the
line, so a room called "Score Board" doesn't become your score. A game that words
it differently — "Points", a bare number, a translated status line — simply isn't
matched, and the announcement stays silent rather than reporting a wrong figure.
A Z-machine *time* game has a clock where the score would be, and is correctly
never announced.

### `/status`

Status text reaches a listener only when the game chooses to write it, and then
it scrolls away; a sighted player re-reads a pinned line for free. All three CLIs
answer **`/status`** at any line prompt with the current status — the Z-machine
status line or upper window, the Glk grid windows, or a Scott room block — and
the game never sees the command. The leading slash is what makes intercepting it
safe: no interactive-fiction parser gives `/` a meaning, so no game verb is
shadowed, and lanthorn's own TUI already spells host commands that way. (A `char`
prompt — "press any key" — takes the keypress as itself; `/status` is a line
command. `/menu` is the exception, because a menu *is* a char prompt and a
command that could not be typed at one would be useless.)

This is the same output path piped/redirected use has always taken — kept honest
by the test harnesses that read it — so `--screen-reader` mostly makes it *selectable*
without giving up an interactive terminal. `NO_COLOR` deliberately does **not**
imply this mode: [the convention](https://no-color.org/) is about colour, and
someone who sets it has not asked to lose their status line.

> **Not yet validated with a real screen reader.** The escape output, input
> paths, and TTY gating are measured; NVDA/Orca/VoiceOver behaviour is not. If
> you use one, we would like to hear how this goes.

## `[MORE]` paging in the CLIs

A turn that prints more than a screenful used to scroll its own beginning away in
`gvm-cli` and `scott-cli`; only `zvm-cli` paused. All three now stop at the
bottom of a page with a reverse-video `[MORE]` bar and wait for a key, the way
the original interpreters did and the way the TUI already did. `--pager off`
turns it off.

Paging requires **both** ends to be a terminal — pausing for a key that a pipe
will never send is a hang, which is why the headless harnesses never see it — and
is off in `--screen-reader` by choice. `gvm-cli` pages only its streaming story
window; a game using several buffer windows is painted as fixed panels with their
own scrollback, so there is no bottom of the page to stop at.

## Scrollback in the CLIs, and where the status line sits

`zvm-cli` and `gvm-cli` keep the status line and any grid window fixed while the
story scrolls beneath — and until recently that quietly cost you the ability to
scroll back over anything you had read. The reason is a detail of how terminals
work: **a line is only kept in history when it scrolls off the top of the screen**,
and a terminal decides that from the top edge of the scrolling region. Pin the
status bar at the top and the region starts one row down, so nothing ever leaves
the screen and every line is discarded as it passes.

`--pin bottom` (or its alias **`--scrollback`**, which is what you actually want it
for) moves the same fixed rows to the bottom of the screen. The region then starts
at row 1 again and the story scrolls into your terminal's own history — with its
scroll wheel, its text selection and its search, none of which lanthorn could give
you as convincingly. Nothing is buffered on our side either way; the history was
always the terminal's, and the only question was whether we were standing in the
way of it. Swap it mid-game with **`/pin`** (`/pin top`, `/pin bottom`, or bare
`/pin` to toggle) when you want to read back over what just happened.

The default is `top`, where Infocom put the status line and where a v3 game's
`Score`/`Moves` belong. Hint menus, forms and BeyondZork's stats-and-compass panel
move with it and keep working either way — they simply resize the region from the
other edge.

`gvm-cli` honours it for the ordinary Glk layout: grid windows stacked above one
full-width story window. Anything a game arranges differently — a side-by-side
split, a graphics window — has no obvious top and bottom to exchange, so those
stay exactly where the game put them. `scott-cli` needs none of this: it draws no
fixed rows at all, so it has always scrolled straight into your terminal's
history.

## Saving from the command line

The CLIs prompt for a save name, and now show you what you already have rather
than expecting you to remember:

```
saves: 1 cellar   2 troll
Restore from file:
```

At the **restore** prompt a number picks from that list; anything else is a
filename exactly as before, so a save you genuinely called `2` is still reachable
by name. At the **save** prompt a number is *not* a shortcut — there it would mean
"overwrite that one", and that is a thing worth typing out. The list is shown
anyway, as a reminder of what you would collide with.

And saving over a name that already exists asks first, naming the save you would
lose. Anything but an explicit `y` is a no, including a bare Enter, so the
destructive answer is never the one you get by hesitating.

### Scott Adams saves too, and it is the host that does it

`zvm-cli` and `gvm-cli` reach those prompts because the *game* asked — `@save` is
an opcode, and the host is answering it. A Scott Adams adventure has no such
opcode and no save format of its own, which is true and is not the same as saying
a Scott session cannot be preserved. Classic ScottFree made SAVE GAME and LOAD
GAME its own interpreter commands rather than the adventure's, and `scott-cli`
now does the same:

```
Tell me what to do ? /save cellar
Saved to 'tiny_cave.dat.save/cellar.sav'.

Tell me what to do ? /restore

saves: 1 cellar   2 troll
Restore which ? 1
```

`/save` and `/restore` (alias `/load`) take a name, or ask for one if you leave it
off — and asking is what shows you the list. Everything else is the behaviour
above, shared code and all: numbers pick at the restore prompt and not at the save
prompt, and an existing name is never overwritten without a `y`. `--data-dir`
works as it does in the other two.

**The leading slash is what makes this safe.** `save` and `restore` are perfectly
ordinary things to type at a Scott prompt, and a host that swallowed them would be
worse than no feature; no adventure parser assigns meaning to `/`.

**And the saves are `.sav`, not `.qzl`.** A Quetzal file is the Z-machine's own
standard format and a Scott snapshot is nothing of the kind — it is item
locations, flags, counters and the lamp, written by `scott::Vm`. Naming it `.qzl`
would be a claim about the bytes that is simply false. A save from a different
adventure is refused rather than half-applied, too: the item count is checked
against the loaded game before anything is written.

## Robustness

When a story faults — out-of-bounds memory, stack under/overflow, an unimplemented
opcode — it doesn't take the interpreter down with it. The game halts with a
call-frame stack trace (the faulting PC and opcode, plus each frame's return
address and locals). In the app the trace appears inline in the transcript and the
app **stays interactive**: the map, scrollback, and a deliberate quit all keep
working, and a durable copy lands in `~/.lanthorn/crash.log`. `zvm-cli`/`gvm-cli`
print the trace to stderr and exit 70.
