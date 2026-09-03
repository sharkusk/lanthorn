# Changelog

All notable changes to lanthorn are recorded here.

**Tag convention.** A release is cut by pushing a `v*` tag (see
[`.github/workflows/release.yml`](.github/workflows/release.yml)). A tag whose
name contains a hyphen — `v0.1.0-beta.1`, `v0.2.0-rc.1` — is published as a
**pre-release**; a bare `vMAJOR.MINOR.PATCH` is a full release. The workspace
version in `Cargo.toml` (currently `0.4.0`) versions every crate and every
binary's `--version` at once, and carries any pre-release suffix so a build
identifies itself without reading its git hash.

**A section here becomes the GitHub release body**, so keep it to what a reader
downloading a build wants: what each feature is, briefly. The reasoning, the
measurements and the history belong in the commit that made the change and in
the quest that tracked it. And use no RELATIVE links — `[x](docs/internals/…)`
resolves against the release page rather than the repository and 404s there.
Absolute URLs or no link.

---

## Unreleased

> *This section is drained when a version is cut. README.md describes the
> RELEASED build; prose for a feature that is in `main` but not yet released
> goes into the README in place, at its normal destination, marked with the
> visible tag `*Next release:*`. `release.yml` refuses to cut a release
> while any such tag, or this Unreleased section, still exists.*

- **lanthorn and its engine crates are on crates.io.** `lanthorn-zvm`,
  `lanthorn-gvm`, `lanthorn-scott`, `lanthorn-blorb`, `lanthorn-mapper` and
  `lanthorn-audio` are usable standalone by anything that wants to run
  Z-machine, Glulx or Scott Adams story files; `cargo install lanthorn` builds
  the player itself from source, and `lanthorn-zvm-cli`, `lanthorn-gvm-cli`
  and `lanthorn-scott-cli` install the three no-map CLI players (the commands
  they install are still `zvm-cli`, `gvm-cli` and `scott-cli`).
- **`gvm-cli --version` names itself `gvm-cli` again**, not the crates.io
  package name it briefly picked up in 0.4.1.
- **The map no longer draws an arrow for a move some games decide at random.**
  Lost Pig's gnome tunnels send you to a different room every time you walk
  the same direction; the room panel and the direction matrix now mark that
  exit `?` — "destination varies" — instead of pointing an arrow at whichever
  room the story happened to pick.

---

## v0.4.1 — 2026-09-02

### Highlights

- **Older Glulx games are much faster.** Inform 6 games and Inform 7 games
  from before 2010 never asked for the acceleration newer ones get; lanthorn
  now recognises their core routines and runs them natively. *King of Shreds
  and Patches*: starting the game, `inventory` and `look` took 3.1 seconds in
  0.4.0 and take 0.34 seconds now; the `inventory` turn alone went from 1.5
  seconds to 0.14. `--accel off` turns it off.
- **Glulx games show their inventory.** The inventory panel and the command
  panel's carried column read the story's own object tree, so games that
  answer `i` in their own words (City of Secrets) no longer leave them empty.
- **A menu for the story under the cursor in the picker** — `Space` or a
  right-click — and a shorter hint bar that lists only the library keys.
- **The browser page (Docker) ships its own Nerd Font**, so icons and the
  map's diagonals draw correctly on any machine.

### Added

- **Story menu in the picker.** `Space`, or a single right-click on a row or
  cover, opens a menu beside the story: open it, launch options, fetch its
  metadata, get its hints, point it at an IFDB page — each with its key shown.
  (A double right-click no longer opens launch options; the menu item does.)
- **`?` in the picker** shows every key the story browser knows.
- **Inventory panel items are clickable**, composing the item onto the prompt
  exactly as a click in the command panel does.
- **The story pane's border control cycles** command panel → inventory panel →
  none, remembered per story.
- **The font check asks about the map's diagonal corners separately**, since
  many fonts that carry the icons lack those four glyphs; each answer stands
  on its own, and skipping the second question changes nothing.
- **Matrix map view:** hovering a room shows its full name, and the name
  column now uses the room the pane has, footnoting only names that still
  don't fit.
- **Ctrl-U / Ctrl-D** scroll half a page in the story list and, when the
  prompt is empty, in the story itself.

### Docker

- The browser page embeds IosevkaTerm Nerd Font Mono; `LANTHORN_WEB_FONT` and
  `LANTHORN_WEB_FONT_SIZE` override the family and size.
- The image no longer carries an `unknown/unknown` platform row on GitHub.

### Changed

- **Panels are called panels everywhere**: commands `toggle-command-panel`,
  `toggle-inventory-panel`, `toggle-room-panel`, `cycle-panel`; config section
  `[command_panel]`; style selectors `command_panel.*`, `inventory_panel`,
  `room_panel.*`. The old names are gone.
- **The picker's hint bar** shows one key per action and only the library-level
  ones: `Enter: open  Space: menu  Tab: info  /: IFDB  g: covers  s: sort
  r: refresh  Ctrl+F: find  ?: keys  q: quit`. Every other key still works.

### Fixed

- City of Secrets' dictionary, and any Inform 6 Glulx game whose first
  dictionary word is empty, is read again — the Guiding Light and the command
  panel were dark on those games.
- Downloading a story from IFDB whose title isn't searchable (City of Secrets
  under "CoS") now keeps its metadata, and a forced refetch can no longer wipe
  it.
- Clicking a command-panel noun after a trailing space, or a verb while a
  partial word is typed, no longer doubles the word (`examine examine rope`).
- A dialog opened over the command panel takes all keys and mouse input.
- The Guiding Light no longer offers `fasten` for `hasten north`, no longer
  credits a phrase like `look sharp` to a game that only knows `look`, and
  vets its suggestions on Curses, suvehnux and other games that don't repaint
  their status line.
- Kerkerkruip's grey status strip (panels off) is filled edge to edge.
- A `parent = "…"` re-root in `style.toml` moves a row's colours even where
  the built-in default pinned them.
- A long notification wraps instead of losing its tail.
- The pixel lock is a real switch in the extended v6 render mode.
- Save State reuses the unchanged history turns of the previous archive
  instead of recompressing them; a palette change in a v6 game re-maps the
  pictures already decoded instead of decoding them again; the cover gallery
  scrolls without re-encoding every visible tile.

## v0.4.0 — 2026-09-01

### Highlights

- **Breaking: the command-line flags changed.** Every `--no-x` flag in all four
  front-ends is now `--x on|off` — `--no-sound` is `--sound off`. The old
  spellings are rejected; see the table below.
- **Lanthorn's Guiding Light** — when the parser rejects a word, lanthorn offers
  the story's own (`try instead — lantern`), having first tried each suggestion
  in a silent throwaway copy of your game. Its lines carry a `●` in the margin:
  lanthorn's voice, never the story's.
- **The word reveal** (the `◈` border control) lights every noun on screen that
  the story actually knows, so you can tell the implemented `lamp` from the
  scenery `field`.
- **Toggle controls on the story pane's border** — click to open the command
  band or map, switch the Guiding Light, or change a v6 story's render mode.
- **The story picker follows your folders**, with `Ctrl+F` to filter the whole
  library as you type.
- **Play in a browser, pictures and sound included** — the Docker image now shows
  in-game graphics and plays the game's audio in the page.
- **A third v6 render mode, `extended`**, which fills a tall terminal with more
  story instead of a letterbox.

### Breaking — the command-line flags

Across `lanthorn`, `zvm-cli`, `gvm-cli` and `scott-cli`. No aliases; the old
spellings are rejected.

| was | is |
|---|---|
| `--no-sound` | `--sound on\|off` |
| `--no-images` | `--images on\|off` |
| `--no-accel` | `--accel on\|off` |
| `--no-game-colours` | `--game-colours on\|off` |
| `--no-aux` | `--aux on\|off` |
| `--no-timed-input` | `--timed-input on\|off` |
| `--no-more` / `--no-page` | `--pager on\|off` |
| `--system-colours` | `--colour machine` |
| `--no-status` | removed (use `--story-only`) |

New: `--colour terminal|theme|machine` chooses where a story's default page and
ink colours come from.

### Lanthorn's Guiding Light

- Mistype a word and the light offers what the story would accept — a near
  spelling, a different ending (`opening` → `open`), or the story's own synonyms
  — and only words this story's parser will take. It never changes what you
  typed or sends anything for you.
- Each suggestion is tried first in a silent copy of your game from where you are
  standing, so `illuminate lamp` at Zork's front door says nothing, and in the
  living room says `try instead — light`. `guidance_probe = false` turns the
  trying-out off.
- `--guidance on|off`, `/set-guidance`, or the `●`/`○` control on the pane
  border — remembered per story. A `guidance` row on the settings screen sets
  the default.
- **The word reveal** — the `◈` border control, or `/reveal-words` — lights
  every noun and adjective already on screen that this story knows, for a few
  seconds. Works for Z-machine and Glulx stories.
- **A first-run font check** shows two rows of glyphs and asks which your
  terminal draws properly; the answer sets the map arrows, portal icons and the
  light's lamp glyph at once. `/run-font-check` asks again after a font change.
- **The map finds its own way back**: after a one-way move, a silent copy probes
  for the return passage and the map records it only if the copy comes out where
  you left. `/set-return-probe` and a border control switch it.

### Toggle controls in the pane border

- Clickable icons on the story pane's frame: command band and Guiding Light on
  the bottom border, the map on the right, and — on a graphical v6 story —
  render mode and pixel lock on the top. Lit controls are yellow; hovering shows
  what a click does and the equivalent command.
- `control_icons = "nerdfont"` swaps the plain glyphs for Nerd Font icons.
  Themeable via `panel.control`, `panel.control:lit`, `panel.control:hover`.
- What you switch here is remembered **for that story**; the settings screen sets
  the default new games inherit. `/set-v6-render` and `/set-guidance` now
  persist per game instead of lasting one session.

### The command band

- The WHAT and WITH columns now also list, dimmed, every thing the story has
  mentioned this session — newest first — not only what the object tree says is
  here. Style it with `band.item:seen`.
- Infocom's verb tables include some strong language; `hide_adult_words`
  (default on) keeps the words in `adult_words` out of the VERB column. The list
  is written into your `config.toml` so you can shorten, extend or empty it. The
  words still parse when typed.
- `up`/`down`/`in`/`out` are drawn as glyphs in a cluster beside the compass
  rose, using the same icons as the map.

### Story picker and library

- A library sorted into sub-folders is browsed folder by folder: `Enter` opens,
  `Backspace` goes up. The cover grid (`g`) shows everything below the current
  folder. `Ctrl+F` filters the whole library by title, author, filename or folder.
- A URL works anywhere a path does; lanthorn downloads it, runs it, and offers to
  keep it. A downloaded zip of release disk images is unpacked into your library.
- `lanthorn <library> --fetch missing|all` fetches IFDB metadata and cover art
  for a whole library headlessly; `--import-metadata rows.tsv` applies your own
  identifications and cover URLs for stories IFDB can't place.
- GIF cover art is accepted. The download cap is 32 MiB.

### Original media

- `.g64` disk images play. A zip is opened like a volume and can carry any
  format lanthorn runs, including a Blorb beside the story.

### Docker image

- The browser mode shows cover art and v6 graphics as pictures (sixel) and plays
  the game's sound. Publish port 7682 alongside 7681; `LANTHORN_WEB_IMAGES=halfblocks`
  and `LANTHORN_WEB_AUDIO=off` turn each off.

### Configuration

- An existing `config.toml` gains the settings added since it was written,
  appended commented in the section they belong to. Nothing you wrote is touched.
  `# lanthorn: no-top-up` in the file opts out.
- `history_turns` (default 500) bounds the opt-in turn history.
- `v6_arrow_keys` now defaults to false: arrows scroll and pan the map in a v6
  game as everywhere else; the game's own arrow bindings are opt-in.

### Version 6 rendering

- `extended` render mode: `v6_render = "extended"`, `--v6-render extended`, or
  `/set-v6-render extended` (bare `/set-v6-render` cycles all three). Zork Zero
  shows 50 rows of prose on a 100x50 terminal where `raster` showed 19.
- Dialogs over a v6 game centre in the pane.
- A fractionally scaled raster frame is no longer stretched by the terminal.

### Performance

Everything that runs per turn and per frame got cheaper: guidance and the return
probe share one snapshot per turn, the word reveal and command band no longer
re-scan the story every frame, hybrid v6 frames are cached between changes and
encoded off the main thread, auto-save writes in the background, and evicted
kitty images are deleted from the terminal instead of leaking.

### Fixed

- The banner and opening room description no longer vanish after the first
  command in games that clear the screen during startup (the Solid Gold
  re-releases, *Zork I* r52, *Hitchhiker's* r31).
- A menu printed below a game's own split — Anchorhead's help, LostPig's — stays
  on screen.
- A Glk text-grid window with no border now has a visible ground (City of
  Secrets' `help` menu); themeable via `glk.grid.background`.
- Arthur's CGA side rule no longer repeats a fragment of the top banner.
- `--help` wraps at 80 columns in every front-end.

### Documentation

`docs/` now has three tiers: a player
[**guide**](https://github.com/sharkusk/lanthorn/blob/main/docs/guide/), the
[**internals**](https://github.com/sharkusk/lanthorn/blob/main/docs/internals/),
and a generated
[**reference**](https://github.com/sharkusk/lanthorn/blob/main/docs/reference/)
of every command, key, setting and style selector.
[`docs/README.md`](https://github.com/sharkusk/lanthorn/blob/main/docs/README.md)
maps all three.


## v0.3.0 — 2026-08-26

### Version 6 typefaces

- *Arthur*'s Amiga floppy is drawn in its own proportional face, at the game's
  per-glyph advances, on the 20-row line the machine used. Raster mode only.
- The Macintosh gets its own 7×15 cell and `FONT` 524 off the game's disk,
  matched per game on a multi-game compilation.
- Geneva from a Mac OS System file, topaz 8 from an Amiga Kickstart dump — drop
  either into `~/.lanthorn/`. Nothing is shipped, copied or licensed.
- A 7-wide cell with no disk font falls back to a public-domain 7-wide face
  rather than an 8-wide one whose letters touch.

### Original media

- A release disk sets its machine's colours before the game runs an instruction.
- v1–v4 stories are dressed as the machine that sold them: nine machines' page,
  ink, status line and cursor, measured off captures of the release disks.
- `--system-colours` opts in when you have named a machine by hand.

### Version 6 rendering

- InvisiClues screens in *Shogun*, *Zork Zero* and *Arthur* read the way the
  machines printed them.
- Side art is one drawing — banner, middle, footer — extended one way, instead of
  a per-game recipe.
- The magnification lock is a per-game switch (`/set-v6-pixel-lock`).

### Command line

- `--story <n|name>` opens one game off a compilation disc.
- `--v6-render hybrid|raster` and `--v6-pixel-lock on|off`.
- `--machines` prints the ZMSD §11.1.3 machine table.

### Saves

- Version 6 saves key by medium — `arthur-r54-s890606-adf` beside `…-hfs`.
  v1–v5 games keep sharing saves across media.
- **Existing v6 save directories are not migrated**; rename them to keep them.

### scott-cli

- `/save` and `/restore`, host-side, as classic ScottFree did it. Files carry
  `.sav`, not `.qzl`.

### Fixed

**Version 6 layout and art**

- Pictures are sized by the text beside them, not by the whole screen.
- *Journey*'s frame is centred again with the magnification lock on.
- *Arthur*'s side poles no longer cut through his status bar; his "crystal ball"
  message appears.
- *Shogun*'s status band is inset between the side ornaments.
- *Zork Zero* and *Shogun*'s InvisiClues menu draws inside its own frame.
- Raster text is a proper face, not a blurred, doubled 8×8 font.
- The v6 caret matches what a real Version 6 interpreter drew.
- A proportional pen measures what it draws, in all five places that measure.
- A restart re-asks the launch's questions instead of answering them itself.

**Colour and half-blocks**

- CGA *Zork Zero* no longer bleeds its artwork into white.
- Half-blocks artwork no longer shows a ghost banner or black holes; text over
  artwork is real glyphs.
- Hint screens and transcript notes no longer show your theme through the page.

**Media and the story browser**

- A floppy with no story of its own checks its release's other disks.
- A disc holding several unrelated games offers the chooser; a bare file path
  mounts the same release the picker would.
- A hybrid Macintosh/DOS CD-ROM reports the right machine per game.
- A `.toast` disc appears in the story list.
- The `(blorb)` tag stops lying in both directions.
- Sorting by TYPE orders by the container the column prints.

**Interface**

- Map portal badges, room-dock markers and `/export-map` use glyphs your font
  has, and your own theme.
- Clicking an InvisiClues topic selects the one under the pointer.
- Quitting a CLI no longer lets your shell draw over the last page, or leaks a
  stray mouse report into it.
- A Windows startup prompt no longer waits on input it cannot hear.
- `/dump-windows` names the face each window is drawn with; the debug
  inspector's memory dump reads the story's own words.
- `--help` wraps to your terminal. It never wrapped at all before — long option
  text ran off the edge, and the few options that looked right were hand-wrapped
  in the source and so stopped short of a wide terminal. Every option now reflows
  to the real width. `--interpreter` no longer repeats the machine table that
  `--machines` prints in full.

---

## v0.2.0 — 2026-08-19

**The project is now called lanthorn**, and this release is mostly about one
idea: playing Infocom's games off the disks they were actually sold on, as the
machines that shipped them played them — artwork, sound, colours and all. The
three command-line players got a lot of attention too, chiefly a fix for
something that had annoyed everyone since the first beta: your terminal's
scrollback works again.

### Renamed from babelmap

The binary is `lanthorn`. Three things that hold your data moved with it, and
there is deliberately no migration shim — the formats themselves are unchanged,
so moving them is two commands:

```sh
mv ~/.babelmap ~/.lanthorn
find ~/.lanthorn -name '*.babelmap' \
  -exec sh -c 'mv "$1" "${1%.babelmap}.lanthorn"' _ {} \;
```

| was | is |
|---|---|
| `~/.babelmap/` | `~/.lanthorn/` |
| `<ifid>.babelmap` archives | `<ifid>.lanthorn` |
| `BABELMAP_HOME` (and `_BIN`, `_DEBUG_TERM`, `_TURN_BUDGET_MS`) | `LANTHORN_*` |

The archive format did not change — the old name was never written inside it,
only on it — so a renamed file loads exactly as before.

### Play the original release media

lanthorn reads the disks Infocom pressed, finds the story *and* everything
shipped beside it, and presents the machine that disk came from.

- **Eight disk and disc formats.** AmigaDOS `.adf`; Macintosh HFS floppies,
  including those behind a DiskCopy 4.2 header; Apple II ProDOS `.po` / `.2mg`
  **and** the raw self-booting DOS 3.3 `.dsk` presses, which have no filesystem
  at all and are found by the story's own checksum; Atari ST `.st`; PC `.ima` /
  `.img`; Commodore 1541 `.d64`, whose stories also sit outside the filesystem
  and span two floppies; and ISO 9660 CD-ROMs, including hybrid discs where the
  Macintosh partition is a second volume inside the same image.
- **A release pressed across five floppies mounts as one disk.** Name any single
  volume and the rest are found beside it — *Arthur*'s Apple press keeps its
  story in five segments and its 168 pictures across four disks. The story
  browser shows one row per game rather than one per platter, so every story on
  a compilation is reachable.
- **The artwork is decoded from the disk's own format**, not from a converted
  Blorb: Amiga, Apple II (8-byte records, RLE and XOR), the PC archives (LZW and
  all), and the Macintosh monochrome plate. EGA and CGA plates are drawn in the
  colours their card fixed, and an EGA plate's half-width pixels land at half
  the width. Where a release offers more than one rendition, a dialog, a flag
  and a key all reach the same choice.
- **And the sound.** *The Lurking Horror* and *Sherlock* shipped sampled effects
  on their release disks years before Blorb existed, in a format nothing else
  reads. lanthorn plays them — from the Amiga floppies and from the Macintosh
  `/MAC/SOUND` layout on the *Lost Treasures* CD — including the **pitch**: each
  effect names a note, each sample states the note it was recorded at, and the
  gap between them is the bend. *Sherlock*'s heartbeat really does beat at three
  speeds from one recording. The model was read out of the 68000 interpreter
  Infocom shipped rather than guessed from the files, and it reproduces two
  independent third-party renderings of these sounds on 27 of the 29 effects
  they carry.
- **A disk always outranks a `.blb` filed beside it** — for sound and for
  graphics alike. The disk is the rendition Infocom pressed; a Blorb is
  somebody's later re-rendering, sometimes at audibly different pitches.
  `/play-sound` says which source answered.
- **The machine comes with the disk.** One table inside `zvm` now carries what
  each ZMSD §11.1.3 machine *is* — the byte it writes into `$1E`, the default
  page and ink it reports, the palette its colour numbers resolve through, and
  the §8.3 screen rules it gets by name — every value sourced out of Infocom's
  own interpreter for that machine. Both front-ends read it, so the same disk
  presents the same machine in either. `zvm-cli --machines` prints the whole
  table, declines included, with each machine's period look beside it.

### The command-line players

`zvm-cli`, `gvm-cli` and `scott-cli` are no longer the poor relations.

- **`--pin bottom` gives you your terminal's scrollback back.** A terminal files
  a line into its history only when that line scrolls off the **top of the
  screen**, so a status line pinned up there — what every interpreter has always
  done, and what `--pin top` still does as the default — means nothing the game
  prints is ever archived. Measured: with the fixed window on top, **zero** rows
  reach history; with it on the bottom they all do. `--pin bottom` puts the fixed
  window under the story, and `Shift-PageUp`, the mouse wheel and `tmux`
  copy-mode then reach what the game printed, with no scrollback buffer of our
  own in the way. `/pin` swaps them mid-game.
- **Exiting leaves your terminal where it should be.** The pinned region is
  released and the prompt lands below it — on `quit`, on Ctrl-D, and on Ctrl-C.
- **The save prompt shows your saves.** It lists what is already there and a
  number picks one, so you no longer have to remember what you called it. Saving
  over an existing file asks first.
- **`zvm-cli` plays release media too**, with the same mount path as the TUI. A
  disk holding several stories asks with a numbered menu, each line labelled
  with its Z-machine version, release and serial — the only thing that tells
  four files called `STORY.DAT` apart — and names them from a bundled titles
  table where it can. Piped stdin never prompts into the void.
- **`--machines`** prints the machine table; **`--interpreter`** (was
  `--interpreter-number`) picks one; **`--no-game-colours`** is now in every
  front-end, lanthorn included.

### Version 6 graphics

- **The hybrid renderer is carved by what the chrome contains**, not by the
  story's complement — so a story window's text region is cut from what the art
  actually leaves, and prose behind artwork has somewhere to go.
- **Side borders tile down the flank** rather than stretching into it, and a
  flank is now identified on both axes.
- **Raster composes the frame at 640×400.**
- **Under the Amiga's interpreter number every window shares one colour pair**,
  as ZMSD §8.3 says of that machine, and the text on screen follows it.
- **The `frameless` render mode is gone**, along with the picture-side "already
  on screen" rule that only it needed. `hybrid` and `raster` remain.

### Mapping

- **`move-region <destination> [direction]`** replaces `peel-layer` and
  `merge-layer`, which were one operation all along. It finds its own seam,
  anchored on the room you picked, and asks rather than guesses.
- **The map sometimes speaks first.** Twice in a game lanthorn notices a set of
  rooms that wants a layer of its own and offers to make one — climbing back out
  of a cellar you could only reach through a trapdoor, and walking into a room
  the game itself calls a *Maze*. It never acts on its own; whatever you answer
  is remembered in the map file.

### Also

- `--interpreter-version` sets header `$1F`, so the byte can be experimented
  with; *Shogun* prints it.
- The story pane names the adventure, and its file when the two differ.
- The scrollbar is colour rather than a glyph, and the story pane's fades away.
- The story browser's keys joined the one command registry.

### In detail

The entries below were written as the work landed and carry the reasoning behind
it; the sections above are the summary.

#### Added

- **`move-region <destination> [direction]`**, and it asks rather than guesses.
  Carving a layer off and folding one back turned out to be the same move — *take
  these rooms and put them on that layer* — so there is now one verb for both,
  anchored on the **selected room** instead of the one you are standing in.
  `move-region new` carves a fresh layer, `move-region main` folds back into Main,
  `move-region parent` sends a region home to whatever it was carved from, and any
  layer name works in place of those. You never point at an edge: it walks the
  compass exits and stops at the portals, and when that swallows the whole layer
  it looks at the passages leading **into** your room and cuts the one real
  boundary, saying which. When more than one way in is a genuine boundary it
  offers them by name and by size — the only answer that always works, because a
  maze happily has two rooms whose **south** exits both land where you are
  standing (Adventure's does) and no direction you could type would tell them
  apart. The destination follows the same rule: one possible answer is taken
  silently, several are offered. Nothing is ever severed — the passage you cut at
  becomes a connection *between* layers — so a room stranded on a maze layer
  finally has a cure: select it and `move-region main`.

#### Changed

- **`--interpreter-number` is now just `--interpreter`.** `zvm-cli` has always
  called this `-I`/`--interpreter`, and one concept under two names across two
  binaries is a thing you have to remember rather than know. Beta, so the old
  spelling is simply gone — no alias. The `interpreter_number` config key is
  untouched; only the command line moved. An audit of every argument the four
  binaries share turned up no other pair like it.
- **`--no-game-colours`, for lanthorn too.** `zvm-cli` and `gvm-cli` both had it
  and the TUI did not, despite owning the `honor_game_colours` setting that backs
  them. It does what the others do — tell the story the interpreter has no
  colours, and let your theme paint everything — for one launch, never written
  back to your config. It outranks a `garglk.ini` beside the story and the game's
  own sidecar, because a flag is an instruction for the launch you typed it on
  and a file sitting beside a story is not; a `/set-game-colours` while you play
  still wins over both, since that is you overriding your own flag.

- **`peel-layer` and `merge-layer` are retired** in favour of the single
  `move-region` above. They were never inverses — one was region-granular and the
  other layer-granular — so a peel that grabbed one room too many could only be
  undone by merging the whole layer back and starting again. Their leader keys are
  unchanged and now run the new verb: `p` is `move-region new`, `m` is
  `move-region parent`. Beta, so the old names are simply gone; nothing on disk
  changes.
- **The story pane names the adventure, and its file when the two differ.** The
  border used to show whatever the file was called. It now shows the resolved
  title, with the filename in parentheses when the two disagree —
  `Journey: The Quest Begins (journey-r83-s890706.z6)` — and just the title when
  the file is already named after its story. The comparison normalises first, so
  a release-stamped filename or an Amiga disk image's container name stays
  visible while `bureaucracy.z4` beside "Bureaucracy" does not earn a redundant
  parenthetical. A story with no resolved title falls back to the bare filename
  rather than an empty `()`.

#### Fixed

- **A content warning is not somewhere you can stand.** Cragne Manor opens on two
  full-page warnings, each with its title set in the same bold the game uses for a
  room, and both were minting nodes on the automap before play began. Front matter
  that ends by asking for a keypress was already rejected; these pages ask you to
  *type* yes or no, which looked exactly like a turn ending at the command prompt.
  It isn't one: only the parser prints a `>`, and a page that reads its own answer
  never does. lanthorn now asks whether the prompt is actually there rather than
  whether the game wants typing, which throws out the whole family — warnings,
  gates, credits pages and title plates that read a line — while every real room
  in the Glulx library, including Adventure's terse `superbrief` rooms, still
  lands on the map.
- **The terminal's answers stop being typed into your story.** Launching a game
  could skip the intro and leave `0;rgb:ffff/ffff/ffff11;rgb:2828/2c2c/3434` on
  the first input line, with a beep and a stack of "restore a saved position?"
  prompts. lanthorn asks the terminal for its default colours and terminates the
  question with a status report it waits on; a terminal busy swallowing a
  screenful of graphics — exactly what a picker launch leaves behind — answers
  *after* that wait gives up, and by then nobody is reading, so the replies sit
  in the tty until the game reads them as keystrokes. The digits and semicolons
  then answer every prompt in their path. The probe now reports whether it
  actually read the answer it asked for, and while one is owed the terminal is
  held until it arrives: bytes inside an escape sequence are discarded, and
  anything outside one is your own type-ahead and is replayed intact. Two probes
  run on a picker launch and both are covered.
- **Type-ahead during a slow boot is no longer binned.** The same drain returned
  an empty string when it gave up waiting, discarding everything it had already
  read — including keys pressed while a large story loaded.
- **Windows stops asking a question it cannot hear the answer to.** The colour
  query was sent on every platform, but the non-blocking drain behind it has
  always been a no-op on Windows, so those replies reached the app as key events
  on *every* launch rather than only on a slow one. The query is now gated on
  having a way to read the reply. The cost is that the v6 raster canvas falls
  back to its built-in default ink and page there, as it did before the query
  existed; restoring it needs a Windows console reader.
- **A draft release with no changelog section says so in its own body.** The
  release workflow already warned when it could not find a heading for the tag,
  then published the draft anyway — and nobody reads a warning on a green run.
  The marker now sits where the release is actually reviewed.

---


## v0.1.0-beta.5 — 2026-08-10

### Added

- **`/dump-cells` writes the screen itself — glyphs *and* colours — as plain
  text.** `/dump-windows` says where each window landed; nothing said what colour
  landed in which cell, which is what a v6 layout defect nearly always turns out to
  be. A panel fill painting rows under a menu, a border cell wearing the fill's
  colour instead of the frame's, a label the buffer holds and the screen does not —
  geometry shows none of the three, and each one cost a round trip through a
  screenshot. The new command writes two lines per terminal row: the glyph row, so
  borders and labels read as text, and under it a style row, one key per cell into a
  legend of the distinct styles with their exact colours, attributes, cell counts and
  extents. Above the grid, the rows owned end to end by a single background are
  listed as ranges — "these nine rows all carry the panel fill" as one line — and
  every region an uploaded image covers is named, its glyphs marked `#` because the
  image draws over them while its cells' colours, untouched by the placement, stay
  in the style row. No escape sequences anywhere: the capture is meant to be copied,
  pasted and diffed. It appends to `~/.lanthorn/dump-cells.log` with the path named
  in the transcript, and describes the last frame drawn with no modal over it — a
  modal paints straight onto the cells, so a capture taken through the palette would
  report the palette's box sitting where the game's picture was. Bind it and no modal
  opens at all: `"ctrl+g" = "dump-cells"` under `[keymap.global]`.

- **The story browser names the container, not just the format.** A game played
  off its original Amiga release floppy now lists as `Z6 (ADF)`, beside the
  existing `Z5 (blorb)` — so a disk image is distinguishable from a loose story
  file at a glance. The suffix comes from the mount that identified the story
  inside the image, so it follows the disk rather than the filename.

- **A story off an Amiga floppy is played on an Amiga.** Header byte `0x1E` was
  always settable, but setting it alone produced a machine that never existed: a
  game told it was running on an Amiga, then told its artwork was IBM PC-sized and
  its default colours were your terminal's. The answers now travel together as a
  named **interpreter profile**. *IBM PC* is the default and is exactly what
  lanthorn has always done, named rather than changed. *Amiga* is the sibling, and
  it selects itself — boot a game out of an `.adf` release floppy and you get
  interpreter number 4, the Amiga's 320×200 standard window, its own default page
  and ink, and the palette Infocom's Amiga interpreter actually loaded. Setting
  `interpreter_number` yourself still wins, and now brings the whole machine with
  it rather than one byte. `honor_game_colours` remains the escape hatch: a
  faithful 1989 colour scheme that reads poorly in a modern terminal is one toggle
  away from your own theme.

- **Play straight off the original Amiga floppy.** Hand lanthorn an `.adf` disk
  image — `lanthorn "Zork Zero_Disk1.adf"` — and it mounts the AmigaDOS filesystem
  (OFS and FFS both), finds the game inside, and boots it. No unpacking step, no
  loose files, nothing to rename. Disk images are listed in the story picker
  alongside everything else. Because AmigaOS has no filename extensions and
  Infocom's `Story.data` convention is a convention rather than a promise, the
  story is identified by its **contents** — a Z-machine header whose version,
  memory map, serial and declared length all agree with the bytes present — so the
  two saved games left on the Zork Zero disk, which begin plausibly enough, are
  correctly passed over. A disk with no game on it (the plain AmigaDOS boot floppy
  that ships as Disk 0) says so rather than feeding a system library to the VM.
  The artwork comes with it: a native `Pic.data` archive on the same image is that
  story's art, no configuration involved, because a shared floppy is as strong a
  guarantee of pairing as a Blorb is. And the original media is the better source
  where the two disagree — five *Zork Zero* pictures are cropped in the
  circulating Blorb, including the full 320×200 pillared and bamboo frames that
  survive there only as a top band, and the floppy has all five whole.
- **v6 pictures land one after another, the way the game drew them.** A single v6
  turn can draw several pictures — Arthur's intro paints the graveyard plate and
  then paints Merlin into the middle of it, fourteen instructions later, without
  pausing in between. Compositing both before anything rendered handed you the
  finished screen instantly; now the renderer walks the screens the turn passed
  through, one per frame, so you watch the graveyard fill the screen and then watch
  Merlin arrive on it. The pause between pictures is proportional to the area each
  one painted, so a full-page plate rests for a beat you can see and a small tile
  barely pauses — roughly what the machines these games were written for imposed.
  The interpreter is not slowed or blocked for any of it: the turn runs straight
  through as before and the composite it settles on is byte-for-byte the one it
  always built. Every v6 game, with nothing to switch on — Zork Zero's border
  assembles itself at startup, Shogun's title arrives in two beats — and **any
  keypress collapses the rest of a sequence at once**, landing on exactly the pixels
  waiting it out would have given you, while still doing whatever you pressed it
  for.
- **Click a room in the matrix and it shows you the way there.** lanthorn finds
  the shortest route it already knows how to walk, from the room you are standing
  in to the one you clicked, and marks one cell per step — the row of the room you
  are in, in the column you leave by — so the marks read top to bottom as walking
  instructions. Each cell keeps its own glyph, so you can still see whether the
  step you are about to take comes back or does not. Passages are only ever walked
  in the direction you walked them, so a one-way corridor is never offered
  backwards: a route lanthorn shows you is a route you can actually walk. The
  search covers the whole map rather than just the layer on screen — steps on
  other layers have no row here, and where the route walks out of this layer the
  `⇱out` cell it leaves by is the one marked — and the view never jumps layers
  behind your back. With no known route the room still selects and lanthorn says
  so. `Esc` clears the route and keeps the room selected; a second `Esc` unpins
  it, a third closes the dock. Styleable as `map.matrix.cell:path`.
- **The room dock — one panel that describes where you are.** The floating Room
  Info popup (left-click) and the diagnostics Inspector (right-click,
  `/toggle-inspector`) are retired: both are now BODIES of a single dock that
  slides in at the bottom of the map pane. It covers nothing, counts as no
  overlay, and stays up while you play — the keyboard never leaves the story
  prompt. With nothing selected it **follows** you, describing the room you are
  standing in and updating every move; clicking a room **pins** it there, and the
  header says which regime it is in. Unpin by clicking the pinned room again,
  clicking empty map space, or pressing `Esc`; a second `Esc` closes the dock.
  `/toggle-room-dock` (leader `k`) opens and closes it; `/toggle-inspector` keeps
  its name and now opens — or flips to — the Diagnostics body, no longer needing
  a room to be selected first. Its top edge drags like every other pane boundary
  (height persisted as `room_dock_pct`), it joins the `F3` resize-mode Tab cycle,
  it docks below the matrix view as happily as below the drawn map, and it is
  styleable through `room_dock`, `room_dock.header` and `room_dock.header:pinned`.
  The exit card spends the dock's WIDTH rather than its height — the twelve
  travel directions lay out in up to three columns (cardinals, diagonals,
  portals, matching the matrix's own grouping), so the card is four rows on a
  normal map pane instead of twelve, and falls back to the single column on a
  narrow one. Its two view names are a real tab strip — the same component, the
  same look and the same click as the map pane's layer tabs.
- **Double-click submits, in the command band** — a second click on the same
  word row within the double-click window fires the composed prompt, so the
  last word of a phrase goes straight into the game: click `open`, double-click
  `mailbox`. The first click of the pair picks the word as always.
- **Tab toggles focus in the IFDB search modal** — `Tab`/`Shift-Tab` hop
  between the `Search:` field and the results list, keeping the half-typed
  query and the list selection intact. Typing over the list already dropped
  you into the query editor; this is the way back that isn't `Enter` (an
  unwanted search) or `Esc` (leaves the modal / falls down its ladder).
- **`merge-layer` takes a target** — `merge-layer <name>` folds the active layer
  into any named layer, not just the one it was peeled from (`merge-layer main`).
  This closes a real trap: a room discovered while exploring a maze layer is
  minted onto the maze layer even when it belongs to the surface, and the bare
  merge could only round-trip it back into the maze. Peel the stranded region,
  then merge it home in one step. Merged rooms keep their map positions where
  free and take the nearest free cell where not; an unknown or ambiguous layer
  name refuses with a message and moves nothing.

### Fixed

- **Shogun's title stays on screen behind its boot menu.** Played from the Amiga
  release floppy, the nine centred lines of Shogun's header — the title, the
  copyrights, the licence — vanished the instant the START/RESTORE/QUIT menu came
  up, and a scroll up to find them snapped straight back down. They were still
  being *printed*; they were being thrown away. A v6 `split_window` tiles windows
  0 and 1 together, which places the story window somewhere new, and the prose a
  window leaves behind when it moves is frozen where it was drawn — but only a
  `move_window` or a `window_size` was doing the freezing. Release 295 moves its
  story window with the split where release 322 uses the other two, so eight of
  the nine lines stayed live text in a window the game erased one instruction
  later, surviving only as scrollback above a screen-clear boundary. The split
  now retires prose exactly as its two siblings do.
- **Shogun's score and moves line up at every window width, not just two of
  them.** Playing off the Amiga release floppy, the status band's `Score:` and
  `Moves:` fields agreed on a column at 82 and 83 columns and drifted apart at
  every other size — while the IBM PC build right-justified them everywhere. The
  game does the justifying itself: both labels are painted at the same pixel, so
  the two rows were aligned before the renderer saw them. Under interpreter 4 it
  paints the band one run per character cell, padding included, and the renderer
  glued each row's padding onto the field behind it — after which the field was
  positioned by the pane's scale and then advanced one terminal column per
  character, two rates that agree only where a column is exactly one of the game's
  own. Row one had more padding in front of it than row two, so the two rows
  disagreed by more and more as the window grew. Padding is no longer glued to
  ink, so every field keeps the column the game gave it. Journey's command menu
  gains the same: on the floppy release its `-->` markers now stand in one column
  instead of stepping left beside the shorter party names.

- **Journey's menu headings are whole again.** On the Amiga release floppy the
  command menu's titles came out chewed — `The P` at one window size, `The Pa` at
  another, `Individual Comm` beside them — with the number of surviving letters
  changing as you resized. Release 30 draws that row by ruling it first and
  printing the titles over it, so the row carries the rule's own dashes *and* one
  run per letter at the same places; the letters split the rule into pieces too
  short to be read as a rule, and each stray dash was then stamped at the column
  its pixel position implied rather than the column the title had reached. A rule
  now begins after everything already drawn on the row, and a lone frame glyph
  will not overwrite a word. Five investigations missed it by measuring the `.z6`
  release, which draws the row differently and was never affected.

- **Journey's picture column stops painting its own borders.** The panel behind
  the illustration filled right through the two frame lines that bound it, so the
  frame's sides carried the picture's background instead of the frame's, while its
  top and bottom carried the game's — the user's *"the border lines around the art
  have the artwork's background color"*. A border is not part of the panel, and the
  fill now stops short of both.

- **`/dump-windows` names every image the v6 ring places**, with the rows of the
  game's screen each one is showing. A picture column is drawn at a rect derived
  from its panel rather than from the strip beside it, and that draw was absent
  from the band list entirely — so the one band an investigation most wanted to
  see was the one band the dump could not name.

- **Key bindings work again — the shipped config was teaching a format the parser
  rejects.** `~/.lanthorn/config.toml`'s own `[keymap.*]` example was written
  backwards and in the wrong case (`quit = "ctrl+q"`, `toggle_map = …`), while the
  parser reads the **key** on the left and the registry's **hyphenated** command on
  the right. TOML is happy with either, so the file loaded cleanly and simply did
  nothing, with one easily-missed warning at game start — which then blamed the
  key. The template now shows entries that work, an inverted line is reported as
  inverted and quotes the corrected version, a snake_case command name is told the
  registry's spelling, and a test uncomments the template's own examples and runs
  them through the real resolvers, so the file can no longer document something
  lanthorn does not accept.

- **Capturing `/dump-windows` no longer disturbs what it reports.** The command
  had to be reached through the command palette, which is a modal: it drops a
  graphical v6 pane off its pixel path for as long as it is open, and coming back
  re-uploads every cached chrome band — so the act of asking added
  `modal overlay open: palette` runs to the render-path history and inflated
  `band uploads since launch`, two of the lines the dump exists to print.
  `dump-windows` is now directly bindable, so a key can take the capture without
  opening anything; bind it with `"ctrl+d" = "dump-windows"` under
  `[keymap.global]`. Ctrl rather than a bare key because a story waiting on a
  single keypress receives every plain key itself.

- **A cleared v6 screen stays cleared.** A graphical v6 game that erased its
  screen was never able to say so: the flag the host watches for is the v1–5 lower
  window's, and the v6 erase path never raised anything. So the transcript went on
  re-rendering every line the game had ever printed into whatever the story window
  happens to be *now*. Journey's boot is the plain case — it prints its title
  block while window 0 is the whole screen, erases, opens the play layout, and
  prints the opening passage into a narrow panel on the right; and the title, the
  copyright and *[Press any key to begin]* all came back with it, pushing the
  passage down the panel and, once the two together overflowed it, hard against
  the command menu. Mysterious Adventures stacked its ASCII banner three deep for
  the same reason. An erase now marks the same screen boundary a story window
  moved out from under its own text already marks: what follows is pinned to the
  top of the window and everything before it stays reachable by scrolling up.
  Which window that is, is asked of the window the game erased rather than
  assumed to be the first one — Journey's Amiga release narrates through window 2
  and leaves window 0 behind as an empty strip off the bottom of the screen, so
  played off its floppy it went on showing its title screen underneath the
  opening passage, with *[Press any key to begin]* twice over, while the IBM PC
  release of the same game cleared correctly. Adventure's boot notice, which had
  been standing above the game's banner in just the same way, goes with it.

- **Journey's picture column keeps to its own frame.** The left-hand panel was
  measured against the whole band between the pane edge and the story text rather
  than against the two borders that bound it, so its background flooded past the
  frame's inner rule and up against the prose, and buried the frame's outer edge —
  which, under the Amiga profile, meant the left border simply did not exist
  between the corner on the top rule and the corner on the bottom one. Both edges
  now come out of one probe, run from each side of the column: the fill stops at
  the rule, and the outer border is drawn in the pane column its corners stand in.
  A border that turns out to be *artwork* is refused, so the IBM PC frame — whose
  illustration runs to the screen edge with no border outside it — is untouched,
  as are Zork Zero's and Shogun's flanks.

- **`/dump-windows` accounts for the whole pane.** The bottom-anchored command
  strip is classified through a different scale from the chrome ring's and was
  filtered out of the strip list entirely, so on Journey's frame eleven rows of
  the pane had nothing in the dump claiming them — an unexplained gap that invited
  the reader to invent a reason for it. The menu band's strips are now listed
  beside the ring's.

- **Journey's frame stops drawing two of its four sides in the wrong alphabet.**
  Played under the Amiga profile, the frame's top, bottom and menu rules came out
  as the crisp line-drawing characters the game prints, while its left and right
  edges came out as fat solid bars — the IBM PC profile's reverse-video idiom,
  standing in the same line as the box glyphs, on a screen where Journey emits no
  reverse-video run at all. The side borders are carried down to the menu as a
  one-pixel-tall slice of the game's own canvas stretched to fill the column, and
  that slice was cropped to the border's *ink*: for a border printed as a
  reverse-video space that is the whole 8-pixel text cell and the stretch is the
  ordinary letterbox scale, but a `│`'s stroke is a single pixel inside its cell,
  so it was blown up sixteenfold into a filled block. The crop is now the native
  columns the band actually covers, so a border is drawn at the width the game
  drew it — whichever characters the game drew it with. The IBM PC frame is
  untouched, as are Zork Zero's and Shogun's flanks, which are genuinely artwork.

- **`/dump-windows` says which of its rectangles are draws.** Under the hybrid
  ring only the story window is drawn at the rectangle beside it; every chrome
  window is rasterised into the ring and reaches the screen through the strips
  listed below. The dump printed both alike, so a chrome grid spanning the whole
  screen read as a second paint over the top border's row — an overlap that does
  not exist, and one that cost two investigations. Those lines now say
  `rasterised into the ring`.

- **`/dump-windows` answers the question it was asked.** The command exists to
  say what the game's last frame looked like, and it could not: it is reached
  through the command palette or a hotkey dialog, both modal overlays that route
  the v6 pane off its pixel path, so the frame it described was always the
  overlay's — every window reporting `NOT DRAWN this frame` with six palette
  frames stacked over the one anybody wanted. It now describes the last frame the
  **game** drew: that frame's render path, pane, story viewport, window
  placements, chrome strips and ring clip, with a `frame described:` line saying
  which frame it is and how many modal frames have passed since. The game's own
  window table and the model built from it are still read live — a modal runs no
  game code — and anything genuinely unrecoverable is reported as `UNAVAILABLE`
  rather than silently replaced by the overlay's numbers.

- **…and you can copy it.** The dump is drawn into a v6 pane made of graphics
  placeholder glyphs, so selecting it took them along: the first real capture came
  back placeholder-dense with fields truncated mid-word, the diagnostic corrupted
  by the protocol it was diagnosing. Every capture is now also appended to
  `~/.lanthorn/dump-windows.log`, timestamped, with the path named in the
  transcript — readable from a second terminal while the game is still running,
  and still there afterwards.

- **Shogun has its artwork back on the Amiga floppy.** Played off its disk image,
  *James Clavell's Shogun* showed no graphics at all — not even a title screen —
  while *Zork Zero*, *Journey* and *Arthur* all drew theirs. Infocom's picture
  archive comes in two shapes, and lanthorn only knew one: the other three games
  share a single compression table for the whole file, while Shogun gives every
  picture its own and needs two more bytes per directory entry to say where it
  is. The file says which it is in its own header, and lanthorn now reads that
  instead of insisting on one layout, so all 48 of Shogun's pictures decode and
  the title screen paints. Held to the same standard as before: of the 39
  pictures Shogun's Blorb also carries, 34 come off the floppy byte-for-byte
  identical, and of the five that do not, two differ only in how the Blorb
  rounded the Amiga's colours, and the rest are places the Blorb cropped or
  retouched what the floppy still has whole. The three games that already worked
  decode bit-for-bit as they did.

- **A room the game never made an object of still reaches the map.** *The
  Impossible Bottle*, *frankenfingers* and *Facility* each printed a room name in
  the top-left corner of the status bar and mapped absolutely nothing, for the
  whole game. The room was never the problem: lanthorn had read every one of those
  names correctly, then thrown them away, because it would not seed an empty map
  with a room it could not also find in the story's object tree. That was a
  sensible-sounding rule with a false premise — it assumed every game eventually
  offers such a room, and these never do. *The Impossible Bottle* is compiled by
  Dialog, whose objects carry no names at all; the others keep their room text
  outside the tree entirely. So the rule was not a delay, it was a permanent mute.
  The test is now corroboration: a room with nothing behind it has to be one the
  **story itself** named, printed as a heading in the prose as well as painted on
  the status bar. Real rooms are named twice, in two independent places. A title
  screen or a character sheet is named once — *Beyond Zork*'s setup still shows
  your character's name where a room name goes, and it still isn't mistaken for a
  room. It is also the rule the Glulx side already used, so both engines now ask
  for the same evidence.

- **The Impossible Stairs maps a place, not a place and a date.** Its status bar
  reads `Year: 2001  Place: Front Lawn`, and lanthorn took the lot as the room's
  name — so every time the story turned a year, the same lawn arrived on the map
  as a brand new room the player had never walked to. A status bar can label
  several things at once, and which label means "room" is not something lanthorn
  gets to assume: it now offers each labelled field to the story's own object
  tree, and maps the one the game recognises as a place. The name that reaches the
  map is the one on screen, since the object behind it is called `FrontLawn` and
  no player should have to read that.
- **Journey stops re-sending its frame to the terminal on every frame.** Playing
  Journey in hybrid mode, lanthorn re-encoded and re-uploaded all three pieces of
  the game's on-screen frame — the picture panel, the right-hand border and the
  bottom rule — every single time the screen was drawn, for pixels the terminal
  already had. The cache that exists to prevent exactly that was being emptied
  each frame by a bookkeeping mismatch: Journey's picture panel is drawn at a rect
  of its own, the cache is keyed on where a band is drawn, and only the rect it was
  *measured* at was being declared still-in-use. One unclaimed key evicts the whole
  cache, so every band went with it. An unchanged frame now sends the terminal
  nothing at all, and the images the terminal holds are released when the frame
  that owns them goes away — including the full-screen composite from the title
  sequence, which no longer has to be argued about because it is now recorded like
  everything else.

- **A band drawn twice on one frame keeps two cached images, not one.** Journey's
  right-hand border is both the frame's flank artwork and the column carried down
  to the menu, and both land on exactly the same cells. lanthorn cached images by
  where they are drawn, so each overwrote the other's cache entry and both were
  re-sent to the terminal on every single frame, forever. They now occupy separate
  cache slots. The earlier attempt at this — skipping one of the two draws — is
  reverted: the two carry different pixels, and dropping either loses part of the
  border.

- **Journey's frame gets its sides back.** Under the Amiga interpreter profile the
  frame around the game drew its top, its bottom and its menu, and then simply
  stopped partway down: below the game's own artwork the left and right borders
  were missing entirely, leaving the frame open down both sides for the whole
  stretch between the picture and the command menu. lanthorn already knew how to
  carry a border column down that reclaimed space — it had been doing it for the
  IBM PC profile all along. The two profiles draw the same frame with different
  ink: IBM PC uses reverse-video blocks that fill their character cell, Amiga uses
  `│` glyphs whose stroke sits in the middle of theirs. lanthorn looked for the
  border in exactly one pixel column, found the glyph's blank margin, and gave up.
  It now looks across the whole character cell, so both profiles frame the gap. The
  right-hand border is also one image lighter per frame: it was being drawn twice,
  once as artwork and once as the extension over the top of it.

- **Zork Zero's compass keeps its colours off the Amiga floppy.** Booted from the
  disk image, the compass arrows and room icons came out bright blue, purple and
  yellow-green, and the colours changed as you walked from room to room — while
  the same game booted from its Blorb drew them properly. Neither archive was at
  fault: both say those pictures have no colours of their own and must borrow the
  palette of the last full illustration drawn, and lanthorn was only listening to
  one of them. A Blorb announces it in a chunk; the original Amiga archive writes
  a plain zero where a picture's palette would go — which, for *Zork Zero*, marks
  exactly the same 172 pictures, id for id. lanthorn now reads that zero as what
  it is, and the native artwork joins the palette machinery the Blorb path has
  always used, so the compass takes the mood of the room it sits beside instead of
  falling back to a stock EGA table. Checked against Infocom's own converter: of
  the 37152 palette combinations it pre-computed and shipped in the Blorb, the
  Amiga archive now reproduces 36980 exactly, and the rest differ only where the
  two archives genuinely hold different art.
- **A screen the game clears and paints nothing into is blank.** Type `BEGIN` on
  Beyond Zork's opening screen, read the prologue, and the game repaints its
  centred title — with the screen you had just typed on still sitting underneath
  it, `[Type BEGIN, RESTORE or QUIT.] >begin` and all. Every character of that
  title is *placed* in the upper window, so the turn prints nothing at all into
  the story window below; the screen-clear boundary therefore landed at the very
  end of the transcript, one row past the last line, and was read as "no boundary"
  rather than "an empty screen". With no boundary the view fell back to sticking
  to the bottom of the scrollback — and the bottom of the scrollback was the exact
  screen the game had just erased. It now reads as what it is, so the title stands
  alone and the erased screen stays where a clear has always left it in lanthorn:
  above the fold, one scroll away. Anchorhead's `* THE FIRST DAY *` quote box, the
  same shape of screen, stops showing the title splash beneath it too.
- **fmvpoker's bet and quit prompts are back on the screen, and so is what you
  type.** Choosing CHANGE CURRENT BET left the whole bottom panel empty: no "Enter
  the new bet:", no Current Bet / Total Winnings totals, and every digit you typed
  invisible — though pressing Enter still applied the bet, so the game was
  listening the whole time. QUIT did the same, which was the giveaway: nothing
  about the bet screen was at fault, only that the game had started reading through
  that panel. lanthorn decides which window is the story's transcript, and it had
  been letting a *read* settle the question. The game says so itself — the
  Z-machine gives every window a "copy this to the transcript" flag, and fmvpoker
  sets it on the table window and clears it on the panel — so a read no longer
  overrules the declaration. The panel stays a panel and keeps its prompt, the
  table stays the story window and keeps its running totals, and the live input
  line now follows the read into whichever window you are actually typing into.
- **Drop caps and room icons survive the trip off an Amiga floppy.** Boot Zork
  Zero from its `.adf` and the illuminated initial that opens each chapter, and the
  little engraved room icons that punctuate the prose, simply were not there — even
  though the very same pictures drew perfectly inside the in-game map. The art was
  decoding fine; it was being filed in the wrong place. Zork Zero doesn't draw its
  inline art exactly at the text cursor: it looks up a tiny placement record in the
  picture file and nudges the picture a pixel or two in from the line. That record
  is `0×0` in the converted Blorb and `2×1` on the original floppy, so with genuine
  Amiga art every drop cap missed the cursor by two pixels and was reclassified as
  a picture the game had placed for itself — painted onto a window nobody was
  showing instead of floated beside its paragraph. lanthorn now also listens for
  the margin the game reserves immediately after such a draw, which is the story
  saying "the text flows around this one" in as many words.
- **Journey's Amiga border reaches the edge of the window, and its menu answers
  the mouse.** Playing Journey with the Amiga interpreter selected, the frame it
  draws around the screen stopped short of the pane — the prose ran straight
  through the right-hand rule — and clicking a command did nothing unless you
  aimed well above the one you wanted. Shrinking the terminal until it matched the
  game's own eighty columns made both problems vanish, which was the clue: they
  were one fault. Journey draws its frame as *text*, and the Amiga profile swaps
  the IBM PC's reverse-video spaces for line-drawing characters. Those characters
  share every row of the story window, and the test for "the game has painted a
  menu over its story" looked only at rows — so an ordinary scene was mistaken for
  a menu takeover and sent down the path that lays the game's screen out one
  terminal column per game column, while the prose and the click map were still
  being placed proportionally across the pane. Two placements for one screen; they
  agree only at eighty columns. The takeover test now asks whether a run is inside
  the story window at all, not merely level with it, and a repeated-glyph rule is
  drawn across the width it was drawn across rather than one cell per character —
  so the border spans the pane at any size, the way the IBM PC profile's
  reverse-video bars already did.
- **Journey's command menu keeps its columns straight.** With that border fixed,
  the panel along the bottom went visibly crooked: the divider between the party
  and their commands stood in one column on the four rows that show a `-->` marker
  and a column or three further right on the row that doesn't, so the menu read as
  a set of columns that never quite lined up — at most window widths. Word
  fragments that touch are glued back together before being drawn, because a game
  with proportional lettering hands over "Churchyard" as three pieces and they have
  to read as one word; but a run, once glued, advances one terminal column per
  letter. Journey sets each marker flush against the divider that follows it, so
  the divider rode the marker's letters instead of standing where the game put it.
  A line-drawing or block character is now never glued to its neighbours: a rule is
  a distance and a divider is a position, and both are placed by the pixels the
  game drew them at.

- **The poker menu can be clicked where it is printed.** *Play Current Bet*,
  *Change Current Bet*, *Save*, *Restore* and *Quit* all ignored the mouse in
  Frobozz Magic VideoPoker — and so did the *Continue* button between hands. The
  clicks were arriving: the game read the coordinates every time and decided they
  had hit nothing, because the labels were being drawn one text row above the
  place the game had put them. A second text window keeps its own lines and they
  are drawn sixteen pixels apart from its top edge, so a cursor move part-way down
  a line had to be resolved to one; it was resolved to the line it fell inside
  rather than the nearest one, and the fifteen-pixel remainder went with it. The
  five labels landed just above the strip the game accepts a click on for them, so
  pressing a label did nothing while pressing the empty row beneath it worked.
  Declared rows now round to the nearest line, which is where fmvpoker meant them.
- **Artwork off a disk image is drawn at full size.** *Zork Zero* booted straight
  from its Amiga floppy painted its pictures at half scale on a full-scale screen,
  so anything the game positioned from a picture's dimensions landed in the wrong
  place. Two correct rules had collided: a Blorb with no `Reso` chunk really is
  declaring its images non-scalable (which is why scopa and mysterious01 rightly
  stay at 1:1), but a native Amiga picture archive has no such chunk because the
  format has no such concept — absence of evidence was being read as evidence of
  absence. The machine now answers where the container cannot: an Amiga's Version
  6 standard window is 320×200, which is precisely what every Infocom Blorb's
  `Reso` chunk declares, so a game off the floppy and the same game off a Blorb
  now scale identically.
- **The poker frame stays up while you type your bet.** Choosing *Change Current
  Bet* in Frobozz Magic VideoPoker made the table, the border and the whole screen
  vanish, leaving a bare page to type into until the bet was entered. The game had
  drawn nothing new: picking that option simply hands the read to the panel at the
  bottom of the screen, which makes that panel the story window and leaves the
  window holding the artwork behind. Hybrid mode carries everything outside the
  story window as pixel bands and everything inside it as terminal text, and a
  picture belonging to the story window is deliberately left to the second half —
  which works right up to the moment the story window moves out from under its own
  picture. With the story down to one bottom panel, the table belonged to neither
  half and was drawn by nobody. Such a frame now goes to the full-screen composite,
  where the picture is drawn as one image, so the screen above the bet panel is
  pixel-for-pixel the screen you were looking at when you chose the option. No
  other v6 title's picture ever leaves its story window, so nothing else moves.

- **HOLD lands under the card you are holding.** Frobozz Magic VideoPoker positions
  everything it says: `HOLD` under each held card, the running totals in the panel
  below the table. lanthorn read its story window as a transcript, so all of it
  scrolled past as narration instead — and the game's own player was right that
  there is not supposed to be a scroll window in it at all. What settles it is not
  what a run *means* — a game moving the cursor before a run usually means "resume
  the story here", which is what Arthur does before every room name — but what kind
  of **surface** the window is. Arthur's story window is a transcript that happens
  to have pictures drawn on it; fmvpoker's is a picture frame that happens to have
  text positioned in it: its own art encloses the window on all four sides without
  filling it. A window like that now renders as what is sitting on it, where the
  game put it, with no transcript at all. HOLD appears under its card, the totals
  sit at the top of the bottom panel, and the hand you drew reads below them. One
  game in the whole v6 corpus answers to this; every other title's story window is
  untouched.

- **The line telling you what you drew stops being written over.** Deal a hand in
  Frobozz Magic VideoPoker and the game announces it in the panel under the table —
  *You draw (a) an Eight, (b) a Three, (c) an Ace…* — which is the only place the
  cards are named, and lanthorn was rasterizing the story scroll's opening banner
  straight across it. The panel was never in the wrong place; the story window is
  the whole screen in this game, and once five cards fill the frame's interior the
  largest clear rectangle left for the transcript drops onto the very box the panel
  occupies. The page already filled *under* labels another window is holding, so
  now the glyphs do too: a transcript is lanthorn's re-reading of everything the
  story window has ever said, while a label another window is holding is on the
  screen right now, and where the two collide the label wins. The money lines the
  transcript owns still print below it.

- **fmvpoker's frame stops having a hole punched in the top of it.** Frobozz Magic
  VideoPoker draws its poker table with Zork Zero's artwork — the original ships that
  picture file renamed — so the frame's top-centre tab natively reads *Double
  Fanucci*, a title belonging to a different game. fmvpoker hides it the way a v6
  game does: it parks a window exactly over the banner and erases it to the blue it
  declared for that window. lanthorn recorded the erase correctly and then flooded
  window 0's page straight over the top of it, and window 0 here is the entire
  screen — so the tab came out as a white gash across an otherwise complete blue
  frame, which is what three passes at this had recorded as artwork being clipped at
  the top edge. Nothing was ever clipped. The story page is now the oldest thing in
  its box: it fills under the game's own `erase_window` fills, exactly as it already
  filled under the labels a game prints inside window 0. Every other v6 title is
  byte-identical.

- **THE BAT's title page stops putting rooms on the map.** The game opens on an act
  list — *Prologue • ACT I • Interlude • …* — and then a prologue headed *Excerpted
  from the New Gothenburg Post:*, and the automap had drawn both as rooms before the
  player had typed a single command. Neither is a place; they only look like one,
  because Inform bolds a room heading and a title with the same style. lanthorn now
  reads the shape of the page rather than the words on it. A room heading is joined
  to the description printed directly beneath it, and the turn that prints one ends
  by handing you the command prompt; a banner stands alone above a blank line on a
  page that ends by asking you to press any key. Both halves have to agree, and each
  is load-bearing: Adventure in `superbrief` prints a room as a bold line, a blank
  line and a list of what's lying about, which the first half alone would discard,
  while a room you really did walk into can perfectly well be followed by a cutscene
  that ends on a keypress. Kerkerkruip's screen-reader question, whose *Enable* was
  bold and at the start of its line, stops being a room too.
- **advent.z6's help bar stops losing letters, and wide terminals stop clipping the
  line.** Opening `help` in Adventure showed a navigation bar reading
  `N   n xt subj ct` and `RETURN = r ad subjec` — the `=`, three lowercase `e`s and
  the tail of "subject", gone. It looked like a font problem and was arithmetic. A
  line of v6 status text is *positioned* by the game's own pixel coordinates but
  *drawn* one terminal column per character, and those two only advance at the same
  rate when the pane happens to be one column per 8-pixel game cell. Widen the
  terminal past that and they drift: at 120 columns a game cell is a column and a
  half, so the blank cells the game paints across the bar — harmless where they
  are, sitting over the label's own spaces — landed on its neighbouring *letters*
  instead and wiped them, and the blank just past the end of a label reached back
  inside it and took the last character with it. Blank runs now paint only the
  cells no text claimed, so the bar reads whole at every terminal width.
- **fmvpoker draws its poker table in `hybrid` mode, instead of nothing at all.**
  Frobozz Magic Videopoker paints a full-screen frame and prints its title, bets
  and winnings inside it; hybrid showed all of that text on a blank white page with
  not one picture anywhere. Hybrid shows artwork as a ring *around* the story text,
  so a game that grows its story window over the whole screen leaves no ring to
  draw in and nothing can be shown — which is why such screens are handed to the
  full-picture composite instead. That handover asked whether the art *filled* the
  screen, and fmvpoker's table is a frame with a hollow middle: 17% painted, and
  missed at every point the test looked. It now also recognises art that *encloses*
  the screen, so the table arrives with the text still on it. Every other v6 title
  renders exactly as before.
- **The pixel composite stopped skipping a second text window.** A v6 game can run
  more than one scrolling text window, and the composite drew graphics and status
  grids and simply ignored those — so fmvpoker's bottom menu and its "Select an
  option with your mouse or by typing the first letter." hint were missing from
  `raster` mode entirely, on a screen where the terminal-cell paths showed both.
  They are drawn now, in the game's own ink where you are honouring game colours
  and the theme's where you are not, and the story page no longer paints over them.
- **A v6 menu bar keeps its columns instead of running into one word.**
  fmvpoker's bottom bar read `PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT`
  — five options with nowhere to break. The game places each label at its own
  pixel column and prints them onto one row, and a second text window kept its
  text as plain lines with no note of where each run began, so every label simply
  followed the last. It now keeps the column the game named for a run and pads the
  line out to it, exactly as the main text window already did, and the bar reads
  `PLAY CURRENT BET  CHANGE CURRENT BET  SAVE  RESTORE  QUIT` again. Anything the
  game centres in such a window — fmvpoker's `CONTINUE` button — lands centred too.
- **The Mysterious Adventures now draw a map instead of nothing at all.** All
  eleven of Brian Howarth's games — Scott Adams adventures rebuilt as v6 Z-code —
  played from start to finish with a completely empty automap, even though every
  turn repaints "I'm in a dense SPOOKY Forest / Obvious exits: NORTH SOUTH" in
  plain sight. They defeat every way lanthorn knows to find a room, all at once:
  the player is never put into the object tree at all, every room object carries
  the same compiled name (`ScottRoom`), and the line you read lives in a property
  where no name match will ever find it. lanthorn now takes the room from the
  variable these games keep it in — but only after confirming that the room it
  points at is carrying, in its own properties, the very words on screen that
  turn. The screen and the object tree have to agree before either is believed,
  which is what makes the answer an exact room rather than a name: these games
  reuse a description across whole mazes, and ten rooms that all read "I'm in a
  Tunnel" now map as ten rooms. Nothing changes for any game that already found
  its rooms — the new check runs only where lanthorn previously found none — and
  the automapper's own probing can no longer fault the story it is reading.
- **Text that vanished from `raster` mode in four v6 games.** Three separate field
  reports, one mistaken assumption: the pixel composite kept using "is this pixel
  opaque?" to mean "is there artwork here?", and rasterized glyphs are opaque too.
  - **Shogun's title showed no prose at all.** Its menu is printed *inside* window
    0's four-row box, and measuring the box against those glyphs shrank it to a
    single row — too little for one line and the caret together. The room a story
    window has is now measured against the artwork alone, so the prompt and the
    menu share the rows the way they do on an Amiga; window 0's page is painted
    *under* the labels other windows put in its box rather than over them. **Journey**
    had the same box shrink to zero height — the screen-wide fill that closes the
    bare cells of a reverse-video bar was running across its text panel — so its
    narration was missing from `raster` too, and is back.
  - **advent's help screen lost its whole navigation bar.** "About Adventure",
    "N = next subject", "RETURN = read subject" render fine as cells and were
    simply absent from the picture. The game paints the bar as one run per label
    plus reversed spacer spaces, and a spacer lands inside "About Adventure" — so
    the header saw the spacer's own highlight block, decided it was sitting on
    frame art (where a block would erase the picture), dropped its block and drew
    itself in the page colour on the page. The over-art test now reads the art
    layer, frozen before a single glyph is stamped.
  - **fmvpoker showed no text whatsoever** — a correct blue frame around a blank
    white interior. Its poker table is a 640×400 picture that is mostly *hole*, and
    a full-window picture in the story window is normally a plate the game draws
    instead of prose (Arthur's illustrated screens). Measured by its bounding box
    the frame owned the screen; measured by the pixels it actually paints, the
    largest clear rectangle it leaves is exactly where the game prints. Arthur's
    plates are dense enough to still own theirs.
  - And a cleared screen now starts at the *top* of the story window in `raster`
    as it always has on the cell paths, so Shogun's four-row box shows the line the
    game printed into it instead of redrawing the tail of the banner it had just
    frozen up top.
- **Shogun's "You may choose to:" now sits beside START/RESTORE/QUIT, not under
  the title.** The game prints its nine centred banner lines while window 0 is the
  whole screen, then moves window 0 down to a four-row box level with — and to the
  left of — its boot menu, and prints the prompt there. lanthorn already froze the
  banner where it was painted and already held the right box; it just started the
  resumed transcript flush under the banner and let it flow, so the prompt landed
  nine rows above the menu it belongs beside and scrolled away with everything
  else. The story window's box now says where its transcript starts on every
  presentation, cell paths included: the gap a game leaves between its chrome and
  its story window carries through into your pane, and a menu painted inside that
  box — items and the ground erased under them — travels with it. Measured against
  the chrome's declared rectangle rather than the text in it, so a status panel
  taller than its own two lines (Zork Zero's) still keeps the transcript exactly
  where it was; Arthur, Journey and Adventure render byte-for-byte as before.
- **Selecting a card in scopa no longer smears the OK button across the table.**
  Choosing a card relabels the confirm button from "Choose" to "OK", and the label's
  white field came with it — running out of the button's rounded outline and off the
  right edge of the screen. scopa prints every button label into one scratch window
  it shoves around for each draw, and by the time the screen is composed that
  window's box is a leftover 1000×1000 measurement clamped to the screen, so it
  describes nothing. A text row that names its own background is padded out to its
  window's edges so a status band printed in pieces (Shogun's location and score
  bar) still reads as one solid bar — but only now when the row's text actually
  reaches those edges. A two-letter label with forty-five pixels of nothing beside
  it is a label, not a bar, and stays the size the game drew it.
- **A v6 game that splits its screen for artwork no longer prints the story over
  the picture.** `mysterious01.z6` reserves the top 260 pixels for its illustration
  and then simply narrates — it never repositions the text window, because the
  Z-machine standard says splitting the screen *tiles* the two windows: the upper
  one takes the height it asked for, and the story window "is placed just below
  it". lanthorn only shortened the story window and left it pinned to the top
  corner, so it sat squarely inside the picture and the prose came out printed
  across the artwork. The split now places the story window where the standard puts
  it, and the picture and the prose each get their own half of the screen. Adventure
  benefits twice over: its library asks the interpreter where the split left the
  text window and positions its own from the answer, so its status bar, its room
  description and its `help` menu all now land where the game intended — the subject
  list used to be buried under a text window that still claimed the whole screen.
  Zork Zero's full-screen title splash is untouched: a split that takes the entire
  screen leaves the story window with no height at all, exactly as the standard
  describes.
- **Shogun's title header stays centred outside raster mode.** The nine centred
  lines the game paints across its title screen are frozen where it printed them,
  and the full-frame `raster` composite placed them perfectly — but `hybrid` (the
  default) and `frameless` route text above the story through the status-bar
  renderer, which sorts a line into a left, centre or right field by where it
  *starts*. That is the right question for a status bar and the wrong one for a
  paragraph: five of the nine lines began far enough left to be flushed against the
  left margin and the shortest ended far enough right to be flushed against the
  right one, so a carefully centred block came out ragged on both edges. A line
  with equal margins on the game's own screen was centred on purpose, so it is now
  centred in your pane too, at any terminal width. Status bars are untouched — a
  field that begins at the screen edge is still anchored there.
- **All three of scopa's card decks now show up, at the size they were drawn.**
  The opening menu invites you to click a card type to begin, and only ever offered
  one: the Milanese deck hardwired into the z-code. The Neapolitan and Sicilian
  decks live in the game's Blorb, and scopa draws every one of those pictures
  through a scratch window it borrows for a single instruction — move it, size it
  to 1000×1000, draw at the corner, move it straight on for the next card. By the
  time the renderer looked, that window had gone somewhere else and shrunk to an
  80×1 sliver, so both photographic decks were clipped out of existence and then
  erased by the next fill. Pictures now record the window box they were drawn into
  and freeze onto the screen where they landed when the window moves on — the same
  rule that already keeps a moved window's prose where it was printed. They are
  also drawn at their real size: scopa's Blorb declares no standard window, which
  the Blorb spec defines as "display at actual size, one image pixel per screen
  pixel", so doubling them (right for every Infocom v6 title, all of which do
  declare one) had told the game its cards were twice as big as they are and
  produced a menu row that overlapped itself and hung off the bottom of the screen.
  Pick a deck and the whole hand — table, hand, backs and all — now deals in it.
- **Turning game colours off no longer deletes half of a v6 game's board.** With
  `honor_game_colours` off, scopa's felt table disappeared and left a black
  card table with two green stripes across it — the only survivors being the bands
  the game had drawn on top of the felt. The table was never a colour preference:
  scopa sizes a window to the whole screen, names an explicit green and erases it,
  which is the same drawing operation that paints its cards, and only reaches the
  renderer as a window background because a full-screen erase is treated as a
  screen clear rather than as paint. So the felt is back whichever way the setting
  is thrown: a window the game has *drawn into* keeps the ground it drew on, while
  a window it merely coloured still defers to your theme, and the story window —
  the surface you actually read prose on — is governed by the setting exactly as
  before. Zork Zero, Arthur, Shogun, Journey and Adventure paint no ground at all
  and are untouched.
- **A v6 game with no story window now draws in hybrid mode too.** scopa's card
  table never streams prose — its whole screen is painted rectangles with a couple
  of buttons on top — and hybrid mode, which builds a picture frame *around* a
  terminal transcript, had no transcript to build around. It fell back to the path
  meant for hint menus, which presents a screen as plain positioned text: the two
  button labels arrived, seven characters in an otherwise empty pane, and the cards
  did not. Now a screen the game has painted goes to the full-picture composite
  whichever render mode you are in, so hybrid shows the table exactly as raster
  does. Genuinely text-only screens — Zork Zero's InvisiClues, Shogun's boot menu —
  are untouched and still come up as crisp terminal text.
- **A v6 game that measures text no longer shrinks its own screen.** Deal a hand
  in scopa and the whole table zoomed out — the cards crammed into a corner with
  big black rectangles beside them. The card game was not drawing any of that: to
  find out how wide a string is, it opens a scratch window 1000×1000 so the string
  cannot wrap, prints into it, and reads the width back. lanthorn sized the
  composite to cover every window the game had open, so that one measuring window
  — two and a half times wider than the screen — decided how big the picture was,
  and everything real shrank to fit inside it. Now a window is drawn only where it
  exists: each box is clipped to the screen the story itself declared before
  anything is composited. What the *game* sees is untouched — it still reads back
  the size it asked for, which is the entire point of the trick it is pulling — so
  the measurement stays correct while the picture goes back to filling the pane.
  `/dump-windows` now says both, the size the game set and how much of it is on
  screen.
- **Shogun's title screen keeps its header where the game painted it.** The nine
  centred banner lines are printed while window 0 still *is* the whole screen;
  the game then drops window 0 to a small box at the bottom, beside its
  START/RESTORE/QUIT menu, and prints "You may choose to:" there. The Z-machine
  standard says moving a window changes nothing already on screen — so on the
  original the banner stays up top. lanthorn streamed both halves into one
  transcript, which jammed the prompt under the banner and then scrolled the
  banner out of a four-row box. Prose now freezes where it was printed the moment
  its window moves out from under it: the banner becomes paint at the exact rows
  and columns the game chose, and the transcript starts again at the window's new
  origin, so the opening reads the way it does on an Amiga. Nothing is deleted —
  the frozen lines stay in scrollback. Prose a window is merely resized *around*
  keeps streaming as before, which is what every turn of Arthur does.
- **A backdrop that fills the screen is no longer mistaken for a drop-cap.**
  Frobozz Magic Videopoker came up with its card table missing — some graphics,
  no outline — and Journey's title illustration never arrived at all. Both games
  clear the screen and then paint a full 640×400 picture at its top-left corner,
  and clearing the screen is also what puts the text cursor there, so the picture
  looked exactly like one of Zork Zero's illuminated drop-caps: drawn on the
  current text line, meant to have prose flowing beside it. It got floated into
  the transcript and the screen never received it. lanthorn now asks the question
  a float actually turns on — *is there room left beside it?* — and a picture
  spanning the window from edge to edge answers no. The table, the JOURNEY splash
  and the Mysterious Adventures' title cards all land on the screen now, with the
  story text over them. Zork Zero's drop-caps and room icons and Shogun's opening
  ship are untouched: the widest of those still leaves nearly half its window free.
- **Arthur's opening illustrations are no longer scribbled over.** The sword in
  the churchyard, and Merlin rising out of it, came up with the previous screen's
  narration rasterized straight across the artwork — a wall of text over the
  picture, unreadable in both directions. Arthur never asks for that: it clears
  the screen, draws the plate, hides the cursor and waits for a key, and its
  narration is a *separate* screen it erases before the next illustration goes up.
  The whole graveyard-to-Merlin turn prints not one character. lanthorn was
  painting its own scrollback onto the plate. Now a placed picture that leaves no
  column wide enough to wrap prose into owns the screen outright — exactly as a
  window-filling picture already did — so the illustration ships alone, in both
  `hybrid` and `raster`. A picture that *does* leave a real column, like a margin
  illustration, still gets prose beside it.

- **Shogun's title screen is centred again — because Shogun centres it.** The
  header that opens Shogun (`SHOGUN`, `A Story of Japan`, the copyright block)
  arrived jammed against the left margin. Shogun does the centring itself, in
  pixels: for every line it reads its own window's width, works out the centred
  column, and moves the cursor there — then prints the text with no leading
  spaces at all. The centring was never in the text, so streaming the text and
  dropping the cursor column lost it entirely. lanthorn now carries a declared
  column into the transcript as an indent; at the v6 cell width of 8 pixels the
  two measurements are the same one, and every line lands exactly where the game
  worked out it should. Journey's title screen, centred the same way, comes
  right with it. Games that declare nothing are untouched: Arthur only ever
  moves the cursor to switch it on and off, and Zork Zero only ever asks for
  column 1.

- **Arthur's intro illustrations actually appear — where Arthur put them.** The
  three plates that open Arthur (the sword in the stone, the churchyard, Merlin)
  never rendered at all. Arthur lays those screens out itself: it clears every
  window, asks window 0 how big it is, centres the 584×392 plate by hand at
  x=29, y=5, and narrates over it. lanthorn treated *every* window-0 picture as
  an inline drop-cap — the Zork Zero idiom, where the art is drawn on the text
  cursor and has to scroll with the paragraph beside it — so Arthur's backdrops
  were pushed into the transcript as floats, no window canvas was ever made, and
  the art never rasterized. The two plates of the Merlin screen would also have
  stacked as separate bands instead of compositing, losing the effect of Merlin
  appearing *on* the graveyard. The engine now records whether a picture was
  placed on the window's current text line or at a position the game chose, and
  a placed one gets a real canvas at the pixel origin the game named, with later
  draws compositing into it. The margin Arthur deliberately left around each
  plate stays the page — the art is not stretched to fill it. Drop-caps, room
  icons and Shogun's margin-parked ship are untouched: all three are drawn on
  the cursor, and still float with the prose.

- **Zork Zero's room icons stop sitting on a black box.** The little compass and
  room icons in Zork Zero's banner are line art on a *clear* ground — 95% of
  each 45×40 picture is fully transparent — and the bottom of every one of them
  hangs below the banner artwork, where the game had painted nothing at all.
  Nothing of ours decided what the player saw there, so the graphics protocol
  decided instead, and its answer was black. The Z-Machine Standard is clear
  that it was ours to decide (§8.8.3.2: every Version 6 window has its OWN
  foreground/background pair) and Zork Zero's banner window says white, like the
  DOS original. lanthorn now paints each chrome window's own page into the
  pixels no layer touched, so the ring it ships is self-contained instead of
  leaving holes for the terminal to colour in. Only untouched pixels are filled:
  artwork, status bands, glyphs and the icons' own ink are left byte for byte
  alone, the story area stays clear for the transcript, and a window the game
  gave no colour keeps exactly today's look. `/set-game-colours off` opts out as
  usual. The same rule gives Scopa its green baize back.
- **The status bar stops painting a black band on a light terminal.** With no
  `style.toml` and no colour scheme configured, lanthorn's UI surfaces — the
  status bar, the v4+ upper window, story info, dialog backgrounds, the Glk grid
  styles — were drawn white-on-**black**, regardless of what colour the terminal
  actually is. It was never the game's doing: Anchorhead, the story it was
  reported on, sets no colours at all. It was ours. "No scheme configured" left
  the theme's `chrome` role with nothing to derive from, so it fell back to a
  hard-coded black page — a guess that happens to be right on a black terminal
  and wrong everywhere else, laying a band across the top of the screen. lanthorn
  already asks the terminal for its real default colours at startup (the OSC
  10/11 probe that keeps the v6 raster canvas honest); that answer now reaches
  the theme as well, so the unconfigured look follows your terminal instead of
  overriding it. Terminals that don't answer the probe keep exactly today's
  behaviour, a half answer is declined whole rather than mixed into a probed ink
  on a guessed page, and a scheme you *did* choose is never second-guessed — as
  is a game that sets its own page colours, which still wins the grid outright.

- **The upper window's frame answers to `style.toml` again.** `upper_window_border`
  could be recoloured but not reshaped: its `style` / `style_top` / … keys were
  read straight past. The one place that applied them was the retired `[colors]`
  table, and the `style.toml` lanthorn seeds has no `[colors]` section — the
  selector lives in `[elements]`, where its border keys parsed into the theme and
  stopped there. So `upper_window_border = { style = "none" }` sat in the file
  doing nothing, on Anchorhead and every other v4+ story. The frame's shape now
  travels from the file to the renderer, which both draws it and reserves its
  rows and columns, and the seeded template finally documents the spelling
  instead of showing only the colour form.

- **Quote boxes are readable again.** The Inform `box` statement — the framed
  reverse-video epigraphs a great many games open with — splits the upper window
  tall, prints into it, then shrinks it back to the status line *before* waiting
  for the keypress that is meant to display it. Truncating the grid at that
  shrink destroyed the quote before it could be read, so Anchorhead's two
  startup quotes (the Lovecraft epigraph beside the title, and
  `* THE FIRST DAY *`) rendered as blank screens waiting for a key. A split now
  shrinks the split height but keeps what was painted, so the box stays in the
  upper window exactly where the game placed it — drawn in the story pane, in
  the CLI's pinned region, and read aloud in `--screen-reader`, where a region
  taller than one row counts as content rather than quietened chrome. It is
  retired when the player next acts, which is the "scroll away over the next few
  command inputs" a real screen gave it. Fixed in the VM, so every front-end
  gets it; the per-turn status-line re-split is untouched.
- **The drawn map's one arrow rule: every arrow on a room border is that room's
  own exit.** A one-way passage used to stamp an inbound arrow on its destination
  (worst at a diagonal, where the side-derived `▶` landed on a box corner and read
  as an exit that does not exist — Zork I's Deep Canyon). The far end of a one-way
  line is now bare; the departure arrow and the line ending on the box carry the
  reading.
- **A passage collapsed into a shared line is stamped, not hidden.** When two
  rooms are joined by both a compass edge and a staircase, one line is drawn and
  the other passage used to vanish entirely — Zork I's Chasm knew its way back
  (`up` to the East-West Passage) and the map showed nothing. Each collapsed
  passage now stamps its own glyph (`↑`/`↓`, or its compass arrow) on the border
  of the room it departs from, beside the line it shares.

### Changed

- **A game's status/upper window is no longer boxed by default.** The single-line
  frame lanthorn drew round it is off out of the box: the status line sits flush
  against the story, and the two rows and two columns the frame was costing go
  back to the game's own screen. Put it back — in any style, or one edge at a
  time — with `upper_window_border = { style = "single" }` in `[elements]`.
- The matrix view's tried-but-pathless cell is `×` rather than `_` — a mark
  centered in the cell instead of one hugging the baseline, where it read as
  an empty cell with an underline artifact. `·` (untried) is unchanged.

---

## v0.1.0-beta.4 — 2026-08-05

### Added

- **The matrix map view** — mazes finally have a representation that tells the
  truth. Any layer can switch between the drawn map and a **direction matrix**
  (`/view-map`): one row per room, a column for every direction, each cell
  saying exactly what is known — mutual passage, goes-there-returns-elsewhere
  (with the return direction), one-way, self-loop, tried-but-flat, or untried
  frontier. Selecting a room bolds its known entrances; identically-named
  rooms number themselves; the table thins its cells before it scrolls. A
  layer marked as a maze (`/mark-maze-layer`, or accept the offer lanthorn
  makes when it notices a tangle) defaults to the matrix. Self-loops — "west
  leads back here" — are now recordable at all, one-way passages grow
  arrowheads on the drawn map, and the room panel gains the full per-direction
  exit card (retiring the explored rose and untried-exits list it replaces).
  Designed against a real player's half-mapped Colossal Cave maze, which now
  lives in the test suite. → [mapping](docs/features/mapping.md)
- **The command band** replaces the verb menu (which was a left-edge token
  palette nobody could drive). Modeled on Journey's clickable menu system: a
  bottom band (`F2`, or `open-command-band`) whose columns fill in
  left-to-right as a phrase narrows — verb, then the objects actually here
  and carried (live from the engine, refreshed every turn), then the
  preposition column when the verb wants one. Everything is clickable, letters
  filter the active column, nothing sends without Enter, and the band is a
  dock rather than a modal: the prompt stays visible, paste works, and
  graphical v6 keeps its pixel path. Verbs and their grammar are configurable
  via `[command_band]`.

- **Screen-reader mode** — all three CLIs take `--screen-reader` (alias
  `--plain`), and select it automatically under `TERM=dumb`. It emits no
  escape sequences at all, hands line editing and echo back to the terminal, and
  drops the `[MORE]` pager. `NO_COLOR` is honoured separately, as colour only.
- **`/status`** — a host command, at any line prompt in any of the three CLIs,
  that repeats the current status without the game seeing the command.
- **Score announcements** — in `--screen-reader`, a score that changes is
  announced above the prompt (`[Score 1, up 1]`), since quietening the status
  line otherwise takes the score with it. Exact for Z-machine v1–v3 (a global the
  standard reserves) and for Scott Adams (treasures deposited); recovered from
  the status text for v4+ and Glulx, where no score is exposed to the
  interpreter at all.
- **`[MORE]` paging in `gvm-cli` and `scott-cli`** — previously only `zvm-cli`
  paused at the bottom of a page, so a Glulx game with a long turn scrolled
  straight past; an ordinary Glk library pauses a text-buffer window the same
  way. All three now take `--no-more` (alias `--no-page`), page only when both
  ends are a terminal, and never page in `--screen-reader`.
- **`--show-status`** — narrate the status line whenever the story updates it.
  Off under `--screen-reader`, because a Z-machine v3 status line carries a move counter
  and so changes on every single turn.
- **Menus read as menus in `--screen-reader` mode.** A menu is a rectangle the
  game repaints, so linearised it used to re-read itself in full on every
  keypress — sixteen lines a press at Planetfall's InvisiClues menu, twenty-three
  at Arthur's, fifteen at Counterfeit Monkey's `ABOUT` — to say that a `>` had
  moved down one row. `zvm-cli` and `gvm-cli` now read a menu out **once**,
  host-numbered under a `[menu — type a number to jump, Enter to select]` line,
  and announce each move as `>3. THE DORMITORY (3 of 12)`. Typing a number jumps
  to that item: the host walks the menu with the game's own keys (`n`/`p` when
  the legend names them, else Down/Up), steering by where the marker actually
  landed rather than by a press count, because Arthur's `N` steps over its
  section headings. **`/menu`** re-reads the open menu on demand — at a menu's
  own prompt as well as a line prompt, since screen-reader mode leaves the
  terminal cooked and a keypress there is a whole line. Detection is a
  mechanical diff: only a block that differs from the last one *solely* in
  marker position is treated as navigation, so a status line that changed, a
  menu that scrolled, or a form that gained a field is still emitted in full.
  Nothing outside `--screen-reader` changes — piped and terminal output are
  byte-identical, verified across 13 Z-machine and 9 Glulx stories.
  → [interpreter](docs/features/interpreter.md)

### Changed

- **`--no-status` is now `--story-only`** (`zvm-cli`; `--lower-only` remains an
  alias, and the old spelling still works with a notice). It reads too much like
  what `--plain` does to the status line while being stronger — it suppresses the
  whole upper window, menus and forms included. `gvm-cli` gains the same flag.
- `gvm-cli` renders Glk grid windows as inline text when there is no TTY; they
  were previously tracked and then dropped, losing the status line from piped
  output entirely.

### Fixed

- **Unknown command-line options are now an error** in all three CLIs, naming
  the option, printing the help, and exiting 2. `zvm-cli` and `gvm-cli`
  previously ignored them — a mistyped `--no-statu` did nothing and exited 0 —
  and `zvm-cli` took an unknown single-dash argument such as `-x` for the story
  path. A missing option value and a second positional argument are errors too.
- **A full-workspace code review closed forty-odd defects** (SQ-0619–SQ-0661), the
  themes being:
  - *Hostile files can no longer crash or hang the host.* Illegal Z-machine
    instructions latch a fault instead of panicking; crafted stories, saves,
    blorbs and dictionaries that used to trigger unbounded recursion, multi-GB
    allocations, out-of-bounds indexing or infinite sibling walks are rejected
    or clamped in all three VMs; restored Glulx save data — stack frames, the
    Glk window tree, the heap block list — is structurally validated.
  - *Nothing overwrites a good file with a bad one.* Every persistence write
    (archive, config, saves, sidecar stores, downloads, exports) goes through
    one atomic temp-and-rename helper; a config that is valid TOML but has a
    wrongly-typed value no longer loads as defaults and then rewrites the
    user's file to defaults on the next save; the exit watchdog waits for an
    in-progress save.
  - *Save/restore honesty across engines.* A host restore over a suspended
    in-game `@save`/`@restore` abandons the old suspension in every engine
    (Glulx replayed the snapshot's last command as a free turn; the Z-machine
    recorded a discarded PC into the next save); a resize or a finishing sound
    no longer silently fails the save dialog the player is sitting in; v6
    `@restart` no longer replays pre-restart art on the next palette change;
    Quetzal v6 saves drop the dummy stack frame per §4.11.
  - *The terminal is treated like the shared resource it is.* Worker-thread
    panics no longer tear down a live session's terminal; kitty image ids are
    deleted when their windows close or resize instead of leaking; layered v6
    chrome art is cached instead of re-uploaded every frame; the CLIs accept
    only key presses (Windows doubled everything), reset the scroll region on
    every exit path, and enable VT processing on Windows.
  - *Text is unicode, everywhere it wasn't.* Typed non-ASCII input reaches the
    Z-machine as ZSCII instead of raw UTF-8 bytes; room notes, save
    timestamps, IFDB titles, selections, caret placement and field editing are
    char-, width- or grapheme-aware instead of byte- or column-indexed.
  - *The map's layers behave.* Peeling cuts only the true reciprocal edge,
    merging survives a deleted parent layer, a room can hold more than one
    non-compass passage, and Scott Adams noun resolution matches ScottFree
    (location-aware auto-get, the two-bottles problem).
  - *Styling is honest.* A whole family of parsed-but-dead style.toml keys
    (border sides, glyphs, dialog shadow/placement) now resolves; choosing a
    colour scheme no longer flips border structure; the modal selection,
    footer, inspector and search-highlight styles are themeable selectors
    instead of hard-coded colours.

---

## v0.1.0-beta.3 — 2026-08-03

Fifty commits, and most of them are about honesty: a saved game that restores into a
different terminal, a different graphics backend or a recoloured scene now shows what
it should rather than something that merely looked right when it was written. The
command-line players got the same treatment — `gvm-cli` learned to render a game's Glk
windows as the panels they are, and `zvm-cli` learned to say no to the v6 stories it
was never going to be able to drive. Along the way the map pane stopped stealing your
keyboard.

### Added

- **The IFDB download chooser tells its candidates apart.** Each file now carries
  IFDB's own description — "Release 16: latest version of the game.", "Competition
  version" — which is frequently the only thing that distinguishes two entries: IFDB
  lists two different *Photopia* builds under the identical filename `photopia.z5`. A
  file the library already holds is marked `✓ … · already downloaded`, and the chooser
  now opens even when a game offers a single file, so that mark is always visible
  before you fetch a duplicate.
- **`glk_pixel_scale`** — a Glulx game asks how big a character cell is in pixels and
  sizes its drawings from the answer. Reporting the terminal's true cell made
  *Adventure*'s toolbar render a third of its intended size on a HiDPI display.
  `native` (the default) keeps the honest answer; `auto` normalises the cell to a
  reference height so a game's pixel space scales with the font; `fixed = n` pins the
  divisor by hand.
- **`gvm-cli` renders each Glk buffer window at its own rect.** Games that lay their
  UI out in several windows — *Kerkerkruip* puts its inventory and status panels in
  six of them — used to have every panel's text dumped into the story stream, so
  "Health: 18 of 18" appeared inline in the prose. Windowed rendering engages only
  when a game actually uses more than one buffer window; every other game keeps the
  streaming path, and the terminal's own scrollback with it.
- **A second v6 prose window gets its own buffer**, so a game that streams narration
  through a window other than the main one keeps both readable.
- **`/dump-windows` describes a v6 story's real layout**, one block per window, and
  the render path is logged and stamped — including *why* the pixel path was skipped
  on a given frame, which is the question that actually comes up.
- **Compass clicks map the direction travelled**, so clicking a room's rose records
  the passage you took rather than the one you aimed at.

### Changed

- **The map pane no longer takes the keyboard.** `Tab` used to hand focus to the map,
  and with the map focused an arrow key panned instead of moving the command-line
  caret — with nothing on screen to say which mode you were in. Every keystroke goes
  to the story now. `Shift+Arrow` pans (as it always did), the mouse pans, zooms and
  selects, and zoom and centring moved onto the `Ctrl+P` leader panel's new **Map**
  group (`+`/`-` zoom, `0` centre). `Tab` still steps the debug inspector's windows,
  and is only advertised when the inspector is open.
- **Manual layout mode and room nudging are gone.** Both were permanent no-ops:
  nothing outside the test suite ever set manual mode, so `nudge-room` and its
  `F6`–`F9` keys could not move a room in any real session, and the refusal was
  silent. `F6`–`F9` now reach a story like any other function key. Room positions
  belong to the layout engine — re-run `tidy-map`.
- **`zvm-cli` declines graphical v6 stories** instead of accepting ones it cannot
  drive. Measured across every v6 story available, each one runs away at its first
  input prompt whatever key it is given: *Zork Zero* and *Arthur* flood the terminal,
  *Shogun* spins silently with nothing to interrupt. `zvm` itself supports v6 fully —
  play those in lanthorn.
- **OS and C-library noise stays off the screen.** ALSA and friends write straight to
  file descriptor 2, which no Rust-side hook can intercept, so their messages landed
  mid-frame and corrupted the display. While the alternate screen is up, fd 2 goes to
  `<user_dir>/stderr.log` instead.

### Fixed

- **A restored game now shows what it should.** Quetzal saves no screen state by
  design — the standard assumes the *story* repaints — but a host Save State swaps
  memory under a game that never learns it happened, so everything the screen needs is
  ours to carry. A v6 archive now stores each graphics window's **display list and
  palette** rather than a snapshot of the pixels, so restored art follows a later
  recolour instead of freezing at the colours it happened to have; it is carried on
  every save path, not just auto-save. A restore **refits the saved screen to the
  terminal you restore into**, which a restore into a different size always was. And
  the archive is backend- and terminal-neutral, so a save moves between kitty,
  half-blocks and sixel.
- **Counterfeit Monkey starts in under a second** (5.4s → 0.76s from the second
  launch). Two faults: `@restore` read a fileref *name* instead of the stream it was
  handed, making a restore from a resource stream impossible for any game; and the
  blorb's own embedded save, whose identity chunk disagrees with the executable beside
  it, was being offered and then rejected. A save belonging to another story is no
  longer advertised, so the game takes its working file-cache path.
- **Graphical v6 rendering**, throughout: a status bar paints inside its own window
  and stays one row deep at any pane scale; prose follows the window the game actually
  streams through rather than window 0; `erase_window`'s background fill is tracked so
  menu panels are opaque; the chrome ring keeps off a secondary prose window's rows and
  re-uploads correctly when a band set changes, a terminal clears, or the pixel path
  resumes; and a full-screen picture takeover no longer mangles the transcript, the
  pager or the composite cache.
- **`gvm-cli` display correctness**: a text grid paints its window background across
  its whole rect rather than only behind the glyphs it drew; `window_clear` redraws a
  screen in place, so a menu updates instead of appending a fresh copy per keypress; a
  grid that shrinks stops repainting the rows it gave up; live input echo carries the
  window's own styling; and the page background is taken from the window tree, which
  is where a game that sets its colours per window actually records them.
- **Neither CLI hangs or panics on input it cannot use.** `zvm-cli`'s line counter was
  a `u16` incremented without check — any story printing 65,536 newlines without a
  pause panicked, which is how *Zork Zero* died in under twenty seconds. `gvm-cli`
  threw away `read_line`'s result, so end-of-input was indistinguishable from a blank
  line and a piped session looped forever.
- **A malformed `config.toml` no longer silently erases itself**, and notification
  toasts anchor to the transcript viewport rather than the story pane rect.

### Save format

- **`.lanthorn` archive `format_version` 5 → 6.** A v6 archive now carries
  `display.json` — each graphics window's display list plus the Blorb §11.3 palette —
  and omits the canvas PNG for any window whose replay reproduced the live canvas at
  save time. Archives written before the bump still load and take the PNG path, which
  is this build's fallback anyway; a version-6 archive is rejected by older builds, as
  the format freeze intends. Bare Quetzal / Glulx-Quetzal interchange files are
  untouched. See
  [`docs/release/save-format-policy.md`](docs/release/save-format-policy.md).

### Known issues

- **`zvm-cli` cannot play graphical v6 stories at all** — it now says so at load
  rather than hanging. Play them in lanthorn, which renders v6 graphics and menus.
- **Room selection lost its keyboard shortcuts.** `select-room next|prev` was bound to
  `n`/`p` only while the map held focus, and with that focus mode removed the command
  is reachable by clicking a room, `/select-room`, or the command palette.
- All beta.2 known issues still stand: **sub-cell buttons in a graphics window can't
  be clicked**, **a v6 game's own erase can take neighbouring art with it**, and the
  three v6 caveats from beta.1 (**Inform-compiled v6 status lines don't paint in
  `raster` mode**, **rasterized v6 text isn't selectable**, **sixel encode latency on
  very large panes**). `hybrid`, the default, avoids the beta.1 three.

---

## v0.1.0-beta.2 — 2026-07-29

Ninety-odd commits on from the first beta, most of them spent making the graphical v6
support that shipped in beta.1 actually behave: its screen model is now rebuilt against
ZMSD §8 rather than approximated, palettes adapt the way the Blorb spec says they
should, and the games that ask for mouse input get it. Alongside that, the map stopped
implying passages it has never seen, the Glulx mapper learned to identify rooms the way
the game itself does, and `config.toml` learned to explain itself.

### Added

- **Switch v6 render modes live** with **`/set-v6-render`** — cycle or name one of
  `hybrid` (crisp terminal story inside a scaled pixel frame), `raster` (the whole pane
  as one image) or `frameless` (no frame at all — full-pane text with a status band and
  inline pictures) without restarting the story. The raster bitfont also gained
  synthesized bold and italic faces, so emphasis survives the pixel path.
- **Adaptive palettes (Blorb §11.3).** A scene that swaps the palette now recolours
  the pictures already on screen, by replaying each window's draws *and* erases in
  order — which is what makes *Arthur*'s churchyard turn brown when you step into the
  church, and its blues invert behind the gravestone.
- **Mouse in v6.** Clicks are delivered during a line read, so *Zork Zero*'s border
  compass rose works while you're mid-command.
- **A map that admits what it hasn't tried.** The mapper records which directions
  you've actually attempted in each room, and an optional `?` overlay marks the ones
  you haven't — verticals included, as `u`/`d`. The room inspector grew a compass rose
  of explored directions that signals exploration by colour and draws real portal
  glyphs.
- **Keep playing with a room panel open.** The room inspector no longer takes the
  prompt hostage: you can read a room's details and keep typing.
- **Ghost-text completion at the story prompt.** Suggestions from the story's own
  vocabulary appear inline as you type, which also stops the prompt bouncing as hints
  appear and vanish.
- **The authentic `[more]` pager**, armed the way the original interpreters armed it —
  on char-input turns, on clears, and at boot.
- **IFDB ratings in the story browser** — the average rating, with its vote count
  beside it, so a 5-star single vote reads as what it is.
- **`--interpreter-number N`** overrides the story header's `0x1E` byte for one run
  (never written back), and **`/print-colors`** reports what the terminal answered to
  the OSC 10/11 colour probe.

### Changed

- **The v6 screen model, rebuilt to spec.** Seven waves of work replaced the beta.1
  approximation with ZMSD §8 behaviour: word wrap, the live cursor, stream 2, line
  counting and `buffer_mode` now do what the spec says. *Zork Zero*, *Arthur*,
  *Journey* and *Shogun* all lay out visibly better for it, and `scroll_window(0)` is
  a silent no-op instead of a player-facing warning.
- **`config.toml` explains itself.** On first run it is seeded like `style.toml`
  already was: every setting lanthorn reads, grouped and commented, with the value
  shown being the default — so uncommenting a line changes nothing and the whole
  surface is browsable from the file. Only settings you actually change are written
  live; section headers stay uncommented; your comments survive later saves.
- **`diagonal_corners` is wired.** The switch the last release said was coming now
  works, under `[map]` in `style.toml` — set it `false` if your font lacks Unicode 13's
  half-diagonals. `[map]` is now the single section driving the map's glyphs, and the
  story-browser badge glyphs became settable too.
- **One line per room pair.** Parallel passages between the same two rooms collapse to
  a single line chosen by priority rather than stacking; staircases keep their own
  vertical slot instead of being folded into the compass line; and an unrelated
  crossing breaks the horizontal instead of drawing a junction that isn't there.
- **Glulx rooms are identified the way the game identifies them** — by its own location
  global rather than by the room's printed name, so two rooms sharing a name stay
  distinct and a renamed room stays itself.
- **One save format, whoever asked for it (SQ-0531).** A story's own `SAVE` now
  writes the same self-contained `.lanthorn` archive Ctrl+S writes — map, screen,
  transcript and inline art included — instead of a bare VM-state-only file. So an
  in-game `restore` finally brings your scrollback back with it, even into a
  freshly launched session. The saves manager's **Type** column is now driven by
  which mechanism wrote the save rather than by its file extension, and marks the
  portable ones (**Game ↗**) — those hold standard save-instruction-PC bytes that
  unzip straight into another interpreter. Host snapshots are taken between turns,
  where no save instruction is executing, so they are honestly left unmarked.
  Restore still accepts a bare `.qzl`/`.sav` carried in from another interpreter,
  in the saves manager and at the game's own `restore` prompt alike.
- **Two new theme selectors** — `saves_portable` (accent + the `↗` glyph) and
  `saves_host_only` (muted) style the saves manager's Type cell.

### Fixed

- **A Glulx game's own `SAVE` now loads from the saves manager (SQ-0556).**
  `SAVE` behaves the same on every engine again: on Z-machine, Glulx and Scott
  Adams alike it writes a `.lanthorn`, the archive appears in the manager, and it
  restores through both the game's own `RESTORE` and the host's. Picking a Glulx
  one from the manager used to answer `Glulx has no game-save (.qzl) format`
  outright. The restore keeps the windows you're looking at exactly as they are —
  the Glulx spec (§1.8.5) keeps Glk's window and stream state out of a save
  deliberately, so nothing in the file can drag a stale screen layout back over a
  live one. No archive format change: the bytes sealed for an in-game save are
  the same standard Glulx-Quetzal as before, and still unzip straight into
  another interpreter.
- **Glulx resume lands in the room you saved in**, not the boot room, and the room
  ids a resume seeds now match the ones a live turn would produce.
- **Toolbar verbs prime the prompt.** Glk's pre-filled line input (§4.2 `initlen`) is
  honoured, so *Adventure*'s graphical toolbar verbs put the word at your cursor
  instead of submitting an empty line — and the player's own edits are mirrored back
  into the game's buffer, so deleting the verb and pressing another button no longer
  re-inserts the first one.
- **The input line and caret stay put** — neither the map taking focus nor a room
  panel opening blanks them any more, and text-entry fields scroll to keep the caret
  visible.
- **v6 layout, a long tail of it.** *Arthur*'s header art no longer moves when the
  `map` command resizes the story window, and its location bar no longer renders as
  sliced half-glyphs at particular pane widths (both were the same class of bug:
  two different roundings of one boundary). *Zork Zero*'s full-screen map is visible
  in hybrid mode instead of being painted over by the transcript. *Journey*'s command
  menu inverts clicks by row, and the width-dependent dark bar under its picture
  column is gone. The v6 status band is found above the story window rather than
  assumed to be at the top of the screen.
- **Graphics are quieter and faster.** Kitty uploads are cached by canvas *content*,
  so a game that repaints an identical frame re-places the image instead of
  re-transmitting it, and image deletion is deferred a generation so animation frames
  no longer flash between steps. *Adventure*'s graphical toolbar renders as a real
  image rather than colour-averaged rule glyphs.
- **`--user-dir` now moves the config read, not just the writes**, so a run with an
  overridden home stops silently discarding everything it saves.

### Save format

- **`.lanthorn` archive `format_version` 4 → 5.** `meta.json` gained
  `trigger: "ingame" | "hoststate"`; restore dispatches on it instead of on the
  file extension. Archives written before the bump still load and read as
  `"hoststate"` — which is exactly what they were — but a version-5 archive is
  rejected by older builds, as the format freeze intends. Bare Quetzal /
  Glulx-Quetzal interchange files are untouched. See
  [`docs/release/save-format-policy.md`](docs/release/save-format-policy.md).

### Known issues

- **Sub-cell buttons in a graphics window can't be clicked.** A game that hit-tests
  its own canvas in pixels — *Adventure*'s graphical toolbar is the case — can place
  buttons smaller than a terminal cell. Its compass rose puts **W** and **E** in a band
  that a cell-centre click can never name, so those two are unreachable however
  carefully you aim. Pixel-precise reporting (DEC mode 1016) was implemented for this
  and withdrawn before release: the cell size it divides by is reported in logical
  points while the protocol reports device pixels, which broke every click on a HiDPI
  display. It needs a `CSI 14t`-derived divisor to land. *Workaround:* type `west` /
  `east`; the toolbar's other buttons all work.
- **A v6 game's own erase can take neighbouring art with it.** Windows share one
  screen (ZMSD §8), so erasing a region clears whatever *any* window plotted there.
  *Arthur*'s map screen erases the columns its side borders occupy, and since the game
  never redraws them they stay gone for the session. This is what a real interpreter
  shows, and lanthorn follows it deliberately rather than second-guessing the game.
- The three v6 caveats from beta.1 still apply: **Inform-compiled v6 status lines
  don't paint in `raster` mode**, **rasterized v6 text isn't selectable**, and **sixel
  encode latency on very large panes**. See their entries below for scope and
  workarounds — `hybrid` (the default) avoids all three.

---

## v0.1.0-beta.1 — first public beta

The first public build of lanthorn: a terminal interactive-fiction interpreter
that draws you a live map as you play. This entry is an inventory of what the
beta ships, not a diff — there's no prior release to diff against.

Everything below has been built and exercised in-repo. Where a claim is scoped
("verified against *Zork Zero*"), that scope is the honest extent of testing — it
is not a promise that every game in a format works.

### Engines

- **Z-machine** (`zvm`, clean-room, zero-dependency) — story-file versions
  **v3–v8**, the Infocom canon and decades of Inform 6. Standard Quetzal
  save/restore (interoperable with Frotz, down to v3 branch-form `@save`), the
  v4+ cursor-addressed upper-window screen model, timed/interrupt input,
  configurable interpreter number, story-dictionary autocomplete, and
  `set_colour` / `set_true_colour` honored at 24-bit RGB.
- **Graphical Z-machine v6** — boots and plays graphical v6 titles, verified in
  depth against ***Zork Zero*** (full banner, side columns, per-room compass,
  illuminated drop-caps), with the same engine and opcode set targeting the wider
  v6 catalogue (*Shogun*, *Journey*, *Arthur*) and Inform-compiled v6 titles.
  Rendered at an **authentic 640×400 screen with an 8×16 cell and 2×-scaled
  art**, matching the DOS/Amiga profile. Three render modes — `hybrid` (crisp
  terminal story text inside a pixel chrome ring, the default), `raster` (the
  whole pane as one pixel image), and `frameless` (no frame; full-pane text with
  inline pictures).
- **Glulx** (`gvm`, clean-room, zero-dependency) — modern Inform 7, targeting
  Glulx spec 3.1.3 with a complete **Glk 0.7.6** layer verified against the
  standard Glulx/Glk test suites. Accelerated-function interception (the Inform
  veneer runs natively, so heavyweights like Counterfeit Monkey skip their long
  startup), the full single- and double-precision float opcode set, external-file
  persistence, line-input terminators, and honest `gestalt` reporting.
- **Scott Adams** (`scott`, ScottFree `.dat`) — the classic 8-bit text
  adventures (*Adventureland*, *Pirate Adventure*, …), played through the same
  TUI and automap. Blorb-bundled PNG artwork renders; the original SAGA
  line-draw format is not decoded.

### Automapping

- **Live, engine-agnostic mapper** — consumes a plain stream of locations and
  movements (never a VM opcode), so one map builder charts all three engines.
  Rooms boxed, exits routed through a lane system with crossing-elimination and
  overlap removal, then continuously re-tidied (configurable eagerness).
- **Room detection across engines** — status-variable (v3), status-line +
  object resolution (v4/v5, including centered custom titles like Beyond Zork /
  Trinity), Inform room-heading parsing (Glulx), and graphical v6. A hideable
  indicator shows *how* the current room was resolved.
- **Layered multi-level areas** — switchable named layer tabs; peel/merge
  regions by hand.
- **Awkward cases understood** — vertical up/down connections (dotted, never
  "distorted"), nautical fore/aft/port/starboard, and redundant multi-direction
  paths collapsed into one shared connector.
- **Hand edits & export** — select / rename rooms and layers, edit notes,
  delete connections, relabel edges; export the map as **SVG**, **Graphviz DOT**,
  or an annotatable text dump; `animate-tidy` steps through the whole layout
  assembly stage by stage.

### Interface

- **Story picker & IFDB** — browse a library as a sortable, badged **list** or a
  `g` cover-gallery **grid**, each with a live info panel (metadata, cover art,
  IFID, resources, saves). On-demand IFDB metadata fetch cached per game, and a
  `/` **IFDB search / browse / download** modal that drops a new story file
  straight into your library.
- **Full TUI cockpit** — mouse support (click a room for info, middle-drag to
  pan, wheel to scroll everything), select-and-copy to the system clipboard via
  OSC 52 (clean even over SSH), a verb/noun menu, dictionary autocomplete,
  readline-style line editing, command history, an inventory strip, and
  notification toasts.
- **Command palette & leader keymap** — a `/`-summoned fuzzy command palette over
  *every* command (reachable even inside modals), plus a tmux-style `Ctrl+P`
  leader panel of mnemonic single-letter map-editing verbs.
- **Transcript tools** — search / filter (story · meta · both) / export, with
  every line category independently themeable.
- **In-game hints** — auto-detected *InvisiClues* files boot in a second
  Z-machine over the story pane; ~50 Infocom titles can fetch a hint file on
  demand with `H`.
- **Sound** — Z-machine bleeps + Blorb sampled audio (AIFF/Ogg/MOD) and Glulx Glk
  sound channels with per-channel volume and finish events, plus a themeable
  border-flash accessibility cue; audio can be routed back from a remote/SSH
  session.
- **Deep theming** — a 7-role palette the whole UI derives from, first-class
  styling for all 11 standard Glk styles, per-game looks, a templated status bar,
  and a fully configurable keymap, all in an auto-seeded, live-reloadable
  `style.toml` (`style.example.toml` mirrors the registry).

### Debugging

- **Built-in debug inspector** (`/debug`, or `--debug` to trace from boot) turns
  the map pane into a live disassembler, retargeted to each engine's model:
  - **Z-machine** — live PC-tracking disassembly; Globals / Locals / Objects /
    Dictionary / Call-Stack / Stack / Memory tabs; opcode hover help;
    click-to-jump operands; execution coverage that persists per story.
  - **Glulx** — routine-discovery disassembly (call-graph + linear scan, tinted
    by confidence, promoted to certain on execution); Functions / Strings / Glk
    tabs; a real call/eval stack and absolute-address memory view with a `<RAM>`
    marker.
  - **Scott Adams** — the action table decompiled one rule per line, fired-action
    coverage, and `✗cond` flags naming the guard that blocked a matched action;
    State / Items / Vocab / World tabs.

### Formats & persistence

- **`.lanthorn` Save States** — one self-contained file freezing the whole
  session (VM state + map + on-screen windows + transcript), with named slots,
  auto-save/auto-load, and an optional per-turn **rewind/replay** history.
- **Standard interchange, in and out** — game-written `@save` produces a portable
  Quetzal `.qzl` (Z-machine, golden-tested against `dfrotz` both directions) and
  a standard Glulx-Quetzal in-game save; other interpreters' saves import through
  the saves manager.
- **Everything else just persists** — Glulx external files (Glk file streams)
  auto-persist per story across sessions; a Glulx game's own fixed-name saves
  (init cache, autosave, undo) are read/written silently so it skips its long
  startup on relaunch.
- **Frozen formats.** For the beta, every persisted byte format is enumerated,
  version-stamped, and pinned by a round-trip freeze test, under three guarantee
  tiers — **Public spec** (Quetzal / Glulx-Quetzal, kept spec-clean and
  interoperable), **Frozen (0.x)** (private binary formats and the `.lanthorn`
  archive: they may only change via a deliberate bump-and-note ritual, and reject
  a newer version marker cleanly), and **Tolerant** (TOML/JSON config &
  metadata: missing fields default, unknown fields ignored). Full inventory and
  policy in [`docs/release/save-format-policy.md`](docs/release/save-format-policy.md).

### Platforms

Runs on **Linux, macOS, and Windows**. Release archives ship four binaries
(`lanthorn` + `zvm-cli` / `gvm-cli` / `scott-cli`) per platform: Linux x86_64
(glibc, needs `libasound2` at runtime), a macOS universal binary (Apple Silicon +
Intel, ad-hoc signed, not notarized), and Windows x86_64 (unsigned).

### Known issues

Honest gaps in the beta. Each is scoped, and carries a workaround where one
exists.

- **Inform-compiled v6 status lines don't paint in `raster` mode.** Inform 6's v6
  library leaves its windows at height 0 and streams prose through the
  transcript; `raster` synthesises a single full-pane buffer for that shape, so
  the game's cursor-positioned status line isn't drawn there. Its prose still
  reads correctly. *Workaround:* play Inform v6 titles in `hybrid` or `frameless`
  mode (Infocom's own v6 titles keep real windows and are unaffected).
- **Rasterized v6 text isn't selectable.** In `raster` mode the story text is
  baked into the pixel image, so mouse select-and-copy can't pick out cells over
  it. *Workaround:* `hybrid` (the default) and `frameless` keep the story as real
  terminal text you can select and copy normally.
- **Sixel encode latency on very large panes.** Sixel is the slowest of the three
  pixel protocols to encode, and the v6 `raster` mode is the heaviest producer;
  encoding runs off the UI thread so input stays responsive, but a full-screen
  raster refresh over sixel can visibly lag. *Workaround:* prefer a
  Kitty/iTerm2 terminal for v6 raster, use `hybrid`/`frameless`, or shrink the
  story pane.
- **Justified text doesn't combine with margin floats, and fully-justified
  ("fill") Glk text falls back to left-flush.** Centered and right-flush
  paragraph layout is honored; the `LeftRight` fill mode currently renders
  left-flush, and justification isn't applied to lines wrapping beside a
  left-margin inline image. Cosmetic — text is never lost.
- **v6 compass-click movement isn't wired end to end.** A mouse click over the v6
  banner compass is mapped to a game pixel and delivered to the VM, but clicking
  a compass spoke doesn't yet reliably issue the corresponding move. *Workaround:*
  type movement commands (the arrow-key and text paths work).
- **v6 proportional fonts aren't honored** — status and chrome text use
  fixed-width metrics, so proportional-font layout is approximated.
- **v6 Save State restore isn't render-verified.** The host Save State captures
  the underlying machine as for any Z-machine game; whether the v6-specific
  render state (window geometry, floats, pictures) comes back pixel-identical
  across a restore isn't verified yet. Standard in-game `@save`/`@restore`
  follows the normal Z-machine path.
- **Glulx cross-interpreter save interop isn't golden-tested.** The Glulx in-game
  save round-trips internally and follows the Glulx-Quetzal spec, but reading our
  Glulx saves in another interpreter (and vice versa) isn't yet pinned by a
  golden test the way the Z-machine `.qzl` interop is (tracked in SQ-0229).
- **v6 menu opcodes are stubs** — `print_form` / `make_menu` are recognized but
  not implemented (tracked in SQ-0457).
