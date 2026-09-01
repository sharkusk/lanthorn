# Saves and rewind

For anyone unsure what the difference is between pressing Ctrl+S and typing
SAVE at the prompt — this page settles it. lanthorn keeps two different kinds
of save, and they solve two different problems.

**Save State / Restore State** is lanthorn's own snapshot. Press Ctrl+S
(`/save-state`) and it freezes everything — the VM's exact state, the map
you've drawn, every open window, and your scrollback — into one file. Ctrl+R
(`/restore-state`) thaws it back. It's the emulator's save-anywhere: bail out
mid-sentence or mid-puzzle, on any of the three engines, and land right back
in it, exactly as you left it. Keep as many named slots as you like — the
saves manager (Enter to load, `s` to save-as, `d` to delete, `i` to import)
lists every one, with its name, type, turn count, and timestamp.

**The game's own SAVE/RESTORE** is what you type at the prompt — the same
command Infocom and its contemporaries always understood. Under the hood it
writes a standard Quetzal file (Glulx's own Quetzal variant, on Glulx games),
the format every other interpreter reads and writes, so a save made in
`dfrotz` imports straight into lanthorn's saves manager, and a save you make
here works in `dfrotz` right back. Both save families come out of the saves
manager wearing the same wrapper — map, screen and transcript included — so
restoring an in-game save through the manager brings your scrollback back
with it too. The manager's Type column tells the two apart: **Game ↗** means
portable, another interpreter can open it; **State** means it's a Save State,
host-only, and lanthorn marks it that way honestly rather than pretending it
travels.

Turn on auto-save and lanthorn snapshots after every turn; leave auto-load on
(the default) and opening a story drops you straight back where you quit, map
included. Switch auto-load off to start a session fresh while keeping the map
you've already drawn.

**Rewind further than the game's own UNDO.** Switch on `record_turn_history`
and lanthorn keeps a save of every turn you take — the leader key then `h`,
or `/open-history`, opens the replay modal, where you can step or auto-play
through your past turns with the map reconstructed exactly as it looked at
each one, then resume play from any of them. It survives across sessions, so
a game you quit mid-replay is still steppable when you come back to it.

Everything lands under `~/.lanthorn/saves/<story-filename>.save/` by
default; `--data-dir <path>` moves just the saves and sidecars elsewhere
without relocating your config or style.

And it isn't only the full TUI — `zvm-cli`, `gvm-cli` and `scott-cli` play
the same games with the game's own SAVE/RESTORE intact, useful over a slow
link or with a screen reader. See [the command line](command-line.md).

Going deeper: [saves](../internals/saves.md) ·
[the persistence model](../internals/persistence.md)
