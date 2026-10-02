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

`lanthorn` itself has an accessibility option `zvm-cli` and friends don't
need, being screen-reader-friendly by design: `--transcript-file <path>`
appends every turn — the game's text, your own commands — to a plain-text
file live, as it happens, so a screen reader or a second terminal's
`tail -f` can follow along outside the app itself. See
[playing](playing.md#keeping-a-transcript).

## Paging and scrollback

A turn that prints more than a screenful stops at the bottom of the page with
a `[MORE]` bar and waits for a key, the way the original interpreters did.
`--pager off` turns that off. `--pin bottom` (alias `--scrollback`) moves the
status line to the bottom of the screen instead of the top, so the story
scrolls straight into your terminal's own history — its scroll wheel, its
selection, its search — rather than standing in the way. Swap between the
two mid-game with `/pin`.

## Saving from the command line

`zvm-cli` and `gvm-cli` prompt for a save name when the game's own
`save`/`restore` runs, and show you what you already have:

```
saves: 1 cellar   2 troll
Restore from file:
```

A number at the restore prompt picks from that list; at the save prompt a
number isn't a shortcut, because there it would mean "overwrite this one" —
worth typing out in full. Saving over an existing name asks first.

Scott Adams games have no save of their own to answer, so `scott-cli` puts
`/save` and `/restore` (alias `/load`) on its own prompt instead — same list,
same rules.

## Transcripts, recording and replay

`zvm-cli` keeps the Z-machine's own script files, and each one is off until
you name a path for it:

```sh
zvm-cli --transcript zork.txt zork1.z3       # what the game's SCRIPT writes
zvm-cli --record walkthrough.txt zork1.z3    # every command you type
zvm-cli --replay walkthrough.txt zork1.z3    # play it back, then hand over
```

`--transcript` gives the game's own `SCRIPT` command somewhere to write;
without it, `SCRIPT` honestly reports that it failed. `--record` logs your
commands, and the individual keys a game reads one at a time, one record per
line; `--replay` reads that file back in place of the keyboard and returns
you to the keys the moment it runs out. The file format is the one other
interpreters use, so a script recorded in Frotz replays here and the reverse.

All three take a filename you choose and overwrite it at each launch — this
run's transcript, this run's script. (`lanthorn` itself keeps the same two
files per game instead, and appends to them; see
[playing](playing.md).)

## Maintaining a library without the TUI

`--fetch missing` walks a directory of stories and fetches titles, blurbs,
ratings and cover art from IFDB for everything that's missing them, with no
terminal needed — handy for a server or a big library you'd rather populate
in one pass than one keypress at a time in the picker. `--fetch all`
refetches everything already cached. `--import-metadata <file>` applies a
curated TSV of your own for titles IFDB doesn't know or has no cover for.

## Sharing one install between players

A household, a classroom or a shared server can run one lanthorn for several
people, each with their own saves, map, settings and look, while the library's
titles, blurbs and cover art are fetched once and shared. Say who is playing
with `--player <name>`, or set `LANTHORN_PLAYER=<name>` in the environment (the
flag wins). Leave both out, or leave them empty, and you are the default
player, exactly as lanthorn has always worked.

```sh
lanthorn --player amy ~/if-games
```

Names are 1 to 29 characters of letters, digits, `.`, `_` and `-`, and may not
start with a dot; anything else is refused with an error before lanthorn
touches a file.

**What is shared.** Everything IFDB gave a story (title, author, blurb, rating,
cover), what lanthorn has learned about a Glulx story's insides, your
`config.toml` and `style.toml` as the starting point for every player, and the
install-wide extras (hint files, logs, system fonts and disks). One player's
metadata fetch shows up for everyone.

**What is per player.** Everything under `~/.lanthorn/users/<name>/`: their
saves and quick-saves, in-game saves, auto-resume, map and turn history,
transcripts and scripts, and each game's own settings. A game one player has
saved shows as played only for them; deleting or resetting your saves never
removes the shared story information.

**Settings layer.** A player's own `config.toml` (and `style.toml`) sits on top
of the shared one. It starts empty, and lanthorn writes only the settings that player
actually changed, so anything they never touched keeps following the shared
file: change the shared volume later and every player who never set theirs
hears it. `[keymap]` and `[hotkeys]` layer the same way. The default player's
`config.toml` *is* the shared one, so their changes become everyone's defaults
unless a player has set their own.

`--user-dir` moves the whole `.lanthorn`, player trees included. `--data-dir`
stands in for `~/.lanthorn/saves`, the shared catalogue (and the default
player's saves); a named player's files always sit under `users/` in the user
directory.

**Lanthorn does not check who you are.** `--player amy` means "act as amy", and
anyone who can run lanthorn can say it. If players must not be able to read each
other's games, put something in front that knows who is who: a login proxy that
passes the username through, or a "who's playing?" picker behind a shared
password for a household that trusts each other.

## Docker: a portable lanthorn

The Docker image runs the full TUI — map, panes, kitty graphics and all — in
any terminal that can run `docker run -it`, with nothing installed locally
but Docker itself:

```sh
docker run -it --rm -v ~/if-games:/stories -v lanthorn-data:/data lanthorn
```

Your terminal's size and capabilities pass straight through to the container,
so everything that works locally works here too. See
[play in a browser](play-in-a-browser.md) for the other mode the same image
offers — serving lanthorn to a browser instead of your terminal.

## Going deeper

- [Interpreter](../internals/interpreter.md) — the full screen-reader, paging and save behaviour
- [Docker](../internals/docker.md) — both container modes in full
- [Play in a browser](play-in-a-browser.md) — the browser-facing mode of the same image
