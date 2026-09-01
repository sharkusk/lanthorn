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

Nothing here blocks you from playing. Half-blocks needs nothing from the
terminal beyond colour, and every terminal that lacks a graphics protocol
falls back to it automatically — a story is always playable and the map
always draws. Force a particular protocol with `--image-protocol`, or turn
pictures off entirely with `--images off`.

![In-game graphics rendered with the kitty graphics protocol](../kitty-graphics.png)

## Hybrid or raster

Graphical v6 games — *Zork Zero*, *Arthur*, *Journey*, *Shogun*, *Beyond
Zork* — draw an illustrated frame around the story text, and lanthorn can
render that frame two ways. `v6_render` in the config, or `/set-v6-render`
mid-game, picks between them, and the choice sticks to that story rather than
your whole setup.

**hybrid** (the default) puts story text in real terminal cells — crisp,
selectable, scrollable — and the decorative frame around it in real pixels.
**raster** paints the whole pane as one image instead, in the game's own
typeface. Both are first-class; hybrid reads better on most terminals and
most games, but raster is worth trying on a story with a distinctive
proportional font, like *Arthur*'s Amiga release. `/set-v6-render` cycles
between them on the spot.

![Zork Zero drawn in hybrid rendering mode](../zork-zero.png)

## The pixel lock

Scale a picture to fill an arbitrary pane and you get resampled edges — a
line that should be crisp comes out one pixel wide in one place and two
pixels wide next door. `v6_pixel_lock` (off by default; turn it on from the
settings screen, `v6_pixel_lock = true` in `config.toml`, or
`/set-v6-pixel-lock` mid-game) fixes the magnification to a whole number of
device pixels per art pixel instead of whatever fraction happens to fill your
pane. Art comes out crisp and tiled borders repeat on exact boundaries, at
the cost of a wider margin around the picture — it can only grow in
half-picture steps, so it rarely fills the pane exactly.

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
Drop a Mac OS System startup disk, an Amiga Workbench floppy, or a Kickstart
ROM image into `~/.lanthorn/`, and a Version 6 game off that machine's own
media is drawn with the face the machine actually used, rather than the
built-in stand-in. Nothing is shipped or copied — the media stay yours.

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

## Going deeper

- [v6 graphics](../internals/v6-graphics.md) — render modes, the pixel lock, and period fonts in full
- [Platforms](../internals/platforms.md) — the graphics-protocol table and per-OS quirks
- [Interpreter](../internals/interpreter.md) — disk-image support and the period look
- [Missing or corrupted glyphs](../internals/glyphs.md) — boxes or blanks where an icon should be
