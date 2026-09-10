# Graphics and terminals

For anyone wondering why a game looks like pixels in one terminal and coloured
blocks in another, or who wants to squeeze the best possible picture out of
the terminal they already have.

## Which terminal gives what

lanthorn draws cover art, in-game pictures and graphical v6's illustrated
frame with real pixels wherever your terminal supports a graphics protocol —
and it auto-detects which one, so you rarely have to set anything.

| Protocol | Terminals | Platforms |
|---|---|---|
| kitty graphics | kitty, Ghostty, WezTerm | Linux, macOS, Windows (via WezTerm) |
| iTerm2 inline images | iTerm2 | macOS |
| sixel | Windows Terminal 1.22+, foot, xterm | Windows 11, Linux, macOS |
| Unicode half-blocks (automatic fallback) | anything, including SSH and tmux | everywhere |

kitty the terminal only runs on Linux and macOS; on Windows you reach the
kitty protocol through WezTerm instead. sixel on Windows needs Windows
Terminal **1.22 or newer**, which in practice means Windows 11.

Nothing here blocks you from playing. The half-block fallback needs nothing
from the terminal beyond colour, and every terminal that lacks a graphics protocol
falls back to it automatically — a story is always playable and the map
always draws. Force a particular protocol with `--image-protocol`, or turn
pictures off entirely with `--images off`.

![In-game graphics rendered with the kitty graphics protocol](../kitty-graphics.png)

## Hybrid, raster or extended

Graphical v6 games — *Zork Zero*, *Arthur*, *Journey*, *Shogun*, *Beyond
Zork* — draw an illustrated frame around the story text, and lanthorn can
render that frame three ways. `v6_render` in the config, or `/set-v6-render`
mid-game, picks between them, and the choice sticks to that story rather than
your whole setup.

**hybrid** (the default) puts story text in real terminal cells — crisp,
selectable, scrollable — and the decorative frame around it in real pixels.
**raster** paints the whole pane as one image instead, in the game's own
typeface. **extended** keeps raster's typeface and grows the frame *downward*
rather than letterboxing it, so a tall terminal buys you extra rows of story
instead of empty margin — the game is told nothing, and keeps the screen it
always had at the top of a taller one. All three are first-class; hybrid reads
better on most terminals and most games, but raster and extended are worth
trying on a story with a distinctive proportional font, like *Arthur*'s Amiga
release. `/set-v6-render` cycles through all three on the spot.

![Zork Zero drawn in hybrid rendering mode](../zork-zero.png)

## The pixel lock

Scale a picture to fill an arbitrary pane and you get resampled edges — a
line that should be crisp comes out one pixel wide in one place and two
pixels wide next door. `v6_pixel_lock` (off by default; turn it on from the
settings screen, `v6_pixel_lock = true` in `config.toml`, or
`/set-v6-pixel-lock` mid-game) fixes the magnification to a whole number of
device pixels per art pixel instead of whatever fraction happens to fill your
pane. Art comes out crisp and tiled borders repeat on exact boundaries, at
the cost of a wider margin around the picture — the steps come from the
artwork itself, and it can only grow a whole step at a time, so it rarely
fills the pane exactly.

## The authentic screen, in period dress

Infocom's v6 games were authored against a 640×400 screen with an 8×16 pixel
font cell, doubling their 320×200 artwork on the way to the display — that
2× is what makes the text read at the right size relative to the picture.
lanthorn reproduces that screen exactly, rather than taking the art
dimensions at face value.

Open a game off its original release disk and lanthorn goes further: it
dresses the story pane as that machine's own interpreter dressed its screen
— page and ink, status line, cursor shape, all nine machines' worth. It's on
by default (`period_look`) and applies only where a machine is actually
named, off a release disk or a chosen `interpreter_number`.

**Your own boot media, your machine's own typeface.** Neither the Macintosh
nor the Amiga kept its body typeface on a game disk — the Macintosh drew with
Geneva out of its System file, the Amiga with topaz out of Kickstart ROM.
Drop a Mac OS System startup disk or an Amiga Kickstart ROM image into
`~/.lanthorn/`, and a Version 6 game off that machine's own media is drawn
with the face the machine actually used, rather than the built-in stand-in. (A
Workbench floppy doesn't help: the topaz the interpreter drew with lives in
the ROM, not on any disk Commodore shipped.) Nothing is shipped or copied —
the media stay yours.

![Arthur's Amiga floppy drawn in its own proportional typeface](../native-font.png)

## Play the original disks

Hand lanthorn an Amiga, Macintosh, Apple II, Atari ST, PC or Commodore floppy
image and it mounts the filesystem, finds the story and everything shipped
beside it, and plays the exact build that disk carries — interpreter number,
palette, default colours and screen rules together.

A disk image is a different **release**, not the same story on other media —
*Journey*'s floppy is release 30, the bare story file release 83, and they
narrate through different windows. Treat a floppy as its own build, and if
you're comparing behaviour across releases, name the exact medium.

![Zork Zero off its Macintosh floppy, dithered stone columns drawn on the machine's own cell](../zork-zero-mac.png)

## Choosing which artwork a game draws

A game's pictures can come from three places, and lanthorn picks the most
certain one available: a Blorb bundled with the story, a disk image (the
whole release, so the pairing between art and story is guaranteed), or a
`pictures` line you set yourself in the story's `config.toml` sidecar, which
always wins outright. Where a release shipped more than one rendition — MCGA,
EGA, CGA, the Macintosh's monochrome plates — you can pick among them.

**Choosing before you launch.** Select the story in the picker and press `o`
(or `Shift+Enter`, or pick **Launch options…** from the story menu that `Space`
and a right-click open) instead of `Enter`, and the
launch-options dialog opens. It lists every rendition it found for *that*
story — beside it in the folder or inside its disk image — with the picture
count and where each one lives, and a second row picks which machine the game
presents itself as. Pick a row and that is what the game draws; the dialog
also shows the interpreter number your choice implies and where it came from,
because prettier art can quietly mean a different machine. Tick the box at the
bottom and the choice is written into the story's sidecar so it holds for
every later launch; leave it clear and it lasts for this one only. Plain
`Enter` never opens it — you only meet the dialog when you ask.

**Scott Adams games with their own pictures.** The Commodore 64 pressing of
Brian Howarth's *Mysterious Adventures* series draws its own line-art room
pictures straight off the disk — no Blorb needed. The picker's info panel
says so (`Pictures: native C64 (vector, N rooms)`), and opening launch
options for one of these adds a **Picture resolution** row: `hi-res` (the
default, drawn as sharp as the picture band your terminal cell allows) or
`original`, the release's own unscaled canvas. It works exactly like the
rows above it — pick one, tick the box to keep it for every later launch. A
story whose pictures ship pre-rendered in a Blorb, or one of the American
S.A.G.A. releases below, shows no such row: those are bitmaps at a fixed size,
so there is nothing to choose a resolution for.

The **ZX Spectrum** pressings of those same eleven titles carry the same
line-art, and lanthorn draws it too — open a `.z80` snapshot straight up, no
Blorb needed, and the info panel says so
(`Pictures: native ZX Spectrum (vector, N rooms)`). It gets the same
**Picture resolution** row as the Commodore 64 pressing, `hi-res` or
`original`, because both platforms store the identical artwork and differ
only in their colours.


**The American *Questprobe* releases, on two very different machines.** *The
Hulk* was sold for the Commodore 64 on a disk whose artwork sits in seventy
separate files beside the game, and for the IBM PC as a folder of DOS files
with sixty-eight `.PAK` pictures in it. lanthorn draws both. Hand it the
Commodore disk and the info panel reads `Pictures: S.A.G.A. (C64 strips, 70
pictures)`; hand it the MS-DOS download — zip and all, no unpacking — and it
reads `Pictures: S.A.G.A. (MS-DOS CGA, 68 pictures)`, the story list titles the
row *The Hulk (MS-DOS)* rather than whatever the archive was called, and the
TYPE column says `zip`.

They are the same drawings by the same artist and they do not look the same:
the Commodore version is painted in that machine's own colours, picked per
picture, while the PC version is locked to the four colours a CGA card could
show at once — black, cyan, magenta and white — and about half of its pictures
are drawn at twice the horizontal detail to make up for it.

**The American S.A.G.A. disks draw theirs too, and each machine drew it
differently.** The Commodore 64 *Hulk* is full-colour comic panels
(`Pictures: S.A.G.A. (C64 strips, N pictures)`); the **Apple II** editions of
*Adventureland*, *Pirate Adventure*, *Mission Impossible* and *Strange
Odyssey* are white hi-res line drawings (`Pictures: S.A.G.A. (Apple II
hi-res, N pictures)`), and they are worth a word because of where they live:
an Apple II release is **two floppies**, the game on one and the pictures on
the other. Hand lanthorn the boot side — the one with the game on it — and it
finds the picture disk beside it in the same folder and draws from both, so
there is nothing for you to do but open the game. Move one of the two
somewhere else and the info panel says `S.A.G.A. (not on this file)` rather
than quietly playing as text.

Three Apple II titles — *Voodoo Castle*, *The Count* and *Claymorgue Castle* —
keep their room pictures in a scrambled form on a disk with no filesystem on
it at all, and lanthorn can't read those yet. It says so
(`S.A.G.A. (scrambled, not readable yet)`) instead of pretending the artwork
is missing. The Atari 8-bit disks are in the same position for a different
reason, and report the same way.

## Going deeper

- [v6 graphics](../internals/v6-graphics.md) — render modes, the pixel lock, and period fonts in full
- [Platforms](../internals/platforms.md) — the graphics-protocol table and per-OS quirks
- [Interpreter](../internals/interpreter.md) — disk-image support and the period look
- [Missing or corrupted glyphs](../internals/glyphs.md) — boxes or blanks where an icon should be
