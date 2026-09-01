# Command line

For anyone who wants to play without the map and the panes — over a slow
link, with a screen reader, or just because a bare terminal is what you'd
rather have. Every release ships three headless players alongside `lanthorn`
itself: `zvm-cli` for the Z-machine, `gvm-cli` for Glulx, and `scott-cli` for
Scott Adams games — no map, no panes, just your scrollback and the game.

## Screen-reader mode

All three accept **`--screen-reader`** (alias `--plain`), and pick it up
automatically when `TERM=dumb`. It emits no escape sequences at all — no
colour, no cursor addressing, no pinned status line — so a screen reader can
follow the output as plain, linear, append-only text.

What would otherwise be spatial arrives in reading order instead. The status
line comes through only when it *changes* — a move counter that ticks over
every turn would otherwise be read out every single turn, so it's suppressed
and you can ask for it any time with `/status`. A **menu** — InvisiClues
hints, a help list — is read out once, numbered; a marker move after that is
one line rather than a repaint, and typing a number jumps straight to that
item.

**Score changes are announced** the moment they happen — "Score 1, up 1" —
rather than left for you to notice on a status line. `--story-only` drops the
whole status window, menus included, for anyone who wants it gone entirely.

## Paging and scrollback

A turn that prints more than a screenful stops at the bottom of the page with
a `[MORE]` bar and waits for a key, the way the original interpreters did.
`--pager off` turns that off. `--pin bottom` (alias `--scrollback`) moves the
status line to the bottom of the screen instead of the top, so the story
scrolls straight into your terminal's own history — its scroll wheel, its
selection, its search — rather than standing in the way. Swap between the
two mid-game with `/pin`.

## Saving from the command line

`zvm-cli` and `gvm-cli` prompt for a save name at `@save`/`@restore`, and show
you what you already have:

```
saves: 1 cellar   2 troll
Restore from file:
```

A number at the restore prompt picks from that list; at the save prompt a
number isn't a shortcut, because there it would mean "overwrite this one" —
worth typing out in full. Saving over an existing name asks first.

`scott-cli` has no in-game save opcode to answer, so `/save` and `/restore`
(alias `/load`) are its own commands instead — same list, same rules.

## Maintaining a library without the TUI

`--fetch missing` walks a directory of stories and fetches titles, blurbs,
ratings and cover art from IFDB for everything that's missing them, with no
terminal needed — handy for a server or a big library you'd rather populate
in one pass than one keypress at a time in the picker. `--fetch all`
refetches everything already cached. `--import-metadata <file>` applies a
curated TSV of your own for titles IFDB doesn't know or has no cover for.

## Docker: a portable lanthorn

The Docker image runs the full TUI — map, panes, kitty graphics and all — in
any terminal that can run `docker run -it`, with nothing installed locally
but Docker itself:

```sh
docker run -it --rm -v ~/if-games:/stories -v lanthorn-data:/data lanthorn
```

Your terminal's size and capabilities pass straight through the container's
pty, so everything that works locally works here too. See
[play in a browser](play-in-a-browser.md) for the other mode the same image
offers — serving lanthorn to a browser instead of your terminal.

## Going deeper

- [Interpreter](../internals/interpreter.md) — the full screen-reader, paging and save behaviour
- [Docker](../internals/docker.md) — both container modes in full
- [Play in a browser](play-in-a-browser.md) — the browser-facing mode of the same image
