# Getting started

For anyone installing lanthorn for the first time and opening their first story.

## Install

Grab the archive for your platform from the
[latest release](https://github.com/sharkusk/lanthorn/releases) — Linux
(x86_64), macOS (universal) and Windows (x86_64) all ship with every release.
Each archive holds four binaries: `lanthorn` itself, plus the no-map CLI
players `zvm-cli`, `gvm-cli` and `scott-cli`. Extract it and run `lanthorn`
from a terminal.

On Windows, two things are worth knowing up front. Closing the console
window (rather than quitting from inside lanthorn) kills the process before
it can save anything, so if you want to walk away mid-session, save first —
`Ctrl+S` for a Save State — or quit through the app. And if you change your
terminal's font size while a story is open, lanthorn won't notice until you
restart it; on macOS and Linux it picks the change up on the next resize.

## Where story files go

lanthorn doesn't care where your stories live — point it at a single file or
at a whole folder:

```bash
lanthorn zork1.z3           # straight into one game
lanthorn ~/if-games/        # a directory — opens the story picker
```

The second form is the one to get used to: hand lanthorn a folder and it
offers to remember it, so a bare `lanthorn` next time goes straight there. It also opens a URL — `lanthorn https://ifarchive.org/…/curses.z5`
fetches the file, opens it like anything else, and then offers to keep a
copy in your library so the next launch doesn't fetch it again.

## The story picker

Point lanthorn at a folder and you get a browsable library rather than a
single game. Each row names the story and, in the **TYPE** column, its
engine and version at a glance — `Z5`, `Z6 (ADF)`, `G3.1.2`, `Scott` — plus
badges for an existing save and an available hint file. Press `g` to flip
between that list and a **grid of covers**, and `Tab` to open an info panel
for whatever's highlighted: format, release, author, blurb, bundled
resources, saves.

Nothing has a cover or a blurb until you fetch one. Press `r` to sweep the
whole library and pull titles, authors, ratings and cover art from IFDB in
one pass — worth doing before anything else, since there's not much for the
grid to show until you do. Press `/` to search IFDB directly and download a
game straight into your library, or `Shift+U` to paste a web address and
fetch whatever's at the other end of it.

A big library sorts itself into folders, and the picker follows: `Enter` on
a folder descends into it, `Backspace` climbs back out. `Ctrl+F` searches
the *whole* library at once, by title, author, filename or folder, no
matter how deep a story is buried.

**A multi-disk release shows up as one shelf, not a pile of disks.** Point
lanthorn at any volume from a set — the seven Apple II floppies of *The Lost
Treasures of Infocom*, `floppy1.ima` through `floppy5.ima` — and it opens a
single picker across every game the release holds, sorted, searchable, each
with its own saves and cover, instead of playing whichever story happened to
sit on the disk you named. A disk or archive holding just one game still
opens straight into it.

## Opening a disk image directly

lanthorn also plays the *original* release disks — hand it an Amiga, Mac,
Apple II, Atari ST, PC or Commodore floppy image and it mounts the
filesystem, finds the game, and presents the machine that disk came from:
interpreter number, palette and artwork together, exactly as it looked on
that hardware. A disk image is a specific build of a game, not just another
copy of one you already have — a floppy's release and serial can genuinely
differ from the bare story file's — so treat one as its own thing worth
keeping around rather than a redundant format. The mechanics live in
[the interpreter notes](../internals/interpreter.md).

## Going deeper

No terminal you like the look of? See
[Play in a browser](play-in-a-browser.md) — lanthorn runs in a browser tab
too. Once a story's open, [Playing](playing.md) covers the keys, the mouse
and the aids lanthorn gives you while you play.
