# Saves & persistence

> For players, the short version is in [the guide](../guide/saves-and-rewind.md).

[← back to README](../../README.md)

Quit mid-dungeon and come back to exactly where you stood — same room, same
inventory, same map, same screen. lanthorn layers a few different kinds of save
on top of each other so you never lose progress, whether you save deliberately,
let the game save itself, or never save at all.

- **`.lanthorn` Save States — freeze the whole session, not just the game.**
  Ctrl+S (`/save-state`) snapshots everything into one self-contained file: the
  VM's exact state, the map you've drawn, the on-screen windows, and the
  transcript. Ctrl+R (`/restore-state`) thaws it back. It's the emulator's own
  save-anywhere snapshot — engine-neutral, and the game never knows it happened,
  so you can bail out mid-sentence or mid-puzzle and land right back in it.
- **Named slots.** Keep as many Save States as you like. The saves-manager modal
  lists them (Enter to load, `s` to save-as, `d` to delete, `i` to import), each
  slot showing its name, type, turn count, and timestamp.
- **One file format, whoever asked for it.** When a story runs its own `SAVE`,
  lanthorn writes the *same* `.lanthorn` archive Ctrl+S writes — map, screen,
  transcript and all. The old split, where an in-game save was a lesser file that
  held VM state and nothing else, is gone: `restore` from inside the game now
  brings your scrollback and its inline artwork back with it, even into a freshly
  launched session. The archive quietly records which mechanism asked for it, and
  that is the only difference between the two.

  And it's the same deal on every engine. Whether you're playing a Z-machine
  story, a Glulx one, or a Scott Adams adventure, typing `SAVE` leaves a
  `.lanthorn` that shows up in the saves manager and loads from either
  direction — the manager's Enter, or the game's own `RESTORE`. Glulx used to be
  the odd one out here, answering the saves manager with a flat "no such format";
  it isn't any more. A Glulx restore still leaves the windows you're looking at
  exactly as they are, which is what the Glulx spec asks for and what stops a
  save from dragging a stale screen layout back with it.
- **Bring saves in from other interpreters — standard Quetzal.** Point the saves
  manager's built-in file browser at a `.qzl`/`.sav` game save from `dfrotz` (or
  any other interpreter), import it, and keep the map you've already accumulated.
  Drop one into the story's save folder and the in-game `restore` picker lists it
  beside your own saves. That interoperability is golden-tested against `dfrotz`
  in both directions (`scripts/gen-interop-goldens.sh`, or
  `cargo test -p zvm --test save_interop -- --ignored`).

  It works the other way too, and this is what the **Type** column in the saves
  manager is telling you. A save marked **Game ↗** was written by the story's own
  `SAVE`, which happens while the VM is suspended *inside* the save instruction —
  exactly where the Quetzal standard says a saved program counter should point.
  Unzip that archive, pull out `game.qzl` (or `game.glksave` for Glulx), and any
  other interpreter reads it, for every Z-machine version right down to v3
  (Zork-era) branch-form `@save`/`@restore`.

  A save marked **State** is a host snapshot, taken *between* turns so you can
  bail out mid-puzzle. There is no save instruction at that program counter for
  another interpreter to hand a result back to, so lanthorn doesn't pretend those
  travel — the mark is there to be honest about it rather than to leave you
  guessing. (Glulx *cross-interpreter* interop isn't golden-tested yet either;
  tracked in SQ-0229.)
- **Auto-save and auto-load.** Turn on auto-save and lanthorn snapshots after
  every turn; leave auto-load on (the default) and launching a story drops you
  straight back where you quit, map and all. Both are configurable — start fresh
  while keeping the accumulated map by switching auto-load off.
- **Glulx external files just persist.** A Glulx game's own Glk files — its
  transcripts, command recordings, and data files — live in an in-memory VFS that
  auto-persists per story across sessions, so they survive a plain quit with no
  explicit save (Kerkerkruip's scores and preferences stick, for instance). When
  a game asks the *player* where to write (`create_by_prompt`), lanthorn prompts
  for a name; when it asks which file to read, it shows a picker of the story's
  existing files. These files ride inside `.lanthorn` Save States too. A Glulx
  game's **own** fixed-name saves (`create_by_name` — e.g. Counterfeit Monkey's
  init cache, autosave, and undo slots) are written and read **silently**, with no
  prompt, and stay hidden from the player saves list; because they persist per
  story, a relaunch auto-restores them so the game skips its long init (SQ-0296).
  → [persistence model](persistence.md)
- **Rewind, replay, resume.** Switch on `record_turn_history` and lanthorn keeps
  a per-turn history — each turn's game save plus a snapshot of the map and
  transcript — inside the `.lanthorn` archive. Open the replay modal (the leader
  key then `h`, or `/open-history`) and step or auto-play through every past turn
  with the map reconstructed exactly as it looked at that moment, then resume the
  game from any earlier turn. It's undo that reaches back further than the game's
  own UNDO — and survives across sessions.
