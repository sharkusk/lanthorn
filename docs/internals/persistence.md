# The persistence model

> For players, the short version is in [the guide](../guide/saves-and-rewind.md).

[← back to README](../../README.md) · see also [Saves & persistence (feature highlights)](saves.md)

lanthorn persists game progress at three distinct layers. They coexist and serve
different purposes: the game's own save, the host emulator snapshot, and an
automatic per-story layer that needs no explicit save at all. This page explains
what each one captures, when it triggers, and what survives.

## Terminology

- **Save State / Restore State** — the *host* (emulator) snapshot. Engine-neutral,
  save-anywhere, captures the whole machine. Invoked with Ctrl+S / `/save-state`
  and Ctrl+R / `/restore-state`. This is lanthorn's own mechanism, not something
  the game knows about.
- **`@save` / `@restore`** — the *game's* own in-game save, the standard path a
  story invokes when the player types `SAVE` / `RESTORE`. On both engines the VM
  state it captures is a portable, standard **Quetzal** payload — on the Z-machine,
  Quetzal proper; on Glulx, standard **Glulx-Quetzal**.

The two are different *mechanisms*, but in the app they are no longer different
*files*: since SQ-0531 both write the same `.lanthorn` archive, and `meta.json`'s
`trigger` field (`"ingame"` / `"hoststate"`) records which one asked for it. Keep
the names straight anyway — what a restore does with the file depends entirely on
which trigger wrote it (see Layer 1 and Layer 2 below). The `zvm-cli`/`gvm-cli`
front-ends have no archive to write and still emit bare Quetzal files.

**A third engine, Scott Adams, has no Layer 1 at all.** The Scott VM has no
in-game Quetzal `@save`/`@restore` suspension protocol; its in-game `SAVE GAME`
action (opcode 71) instead routes to a host **Save State** snapshot (Layer 2),
and it keeps no Layer 3 sidecar. Wherever this page says "both engines" it means
the Z-machine and Glulx — the two engines with an in-game `@save`/`@restore`
(Layer 1); their in-game saves are Quetzal (`.qzl`) and Glulx (`.glksave`)
respectively.

## Layer 1 — the game's own save (`@save` / `@restore`)

Player-initiated, from inside the story ("Type SAVE to save your position").

**What it captures (SQ-0531).** In the app, `@save` writes a `.lanthorn` archive —
byte for byte the container a host Save State writes, carrying the map, screen,
transcript (inline art included), aux data and turn metadata alongside the VM
state. What makes it a *Layer 1* save is not the wrapper but the **PC convention
of the payload inside**, recorded as `meta.trigger = "ingame"`. The `zvm-cli` and
`gvm-cli` front-ends have no archive layer and still write the bare payload alone.

The payload is unchanged and stays interchange-grade — `game.qzl` / `game.glksave`
inside the ZIP is byte-identical to what the bare file used to be, so unzipping it
hands another interpreter a standard save:

- **Z-machine:** a bare, standard **Quetzal** save — the same format other
  interpreters (e.g. `dfrotz`) read and write, all versions including v3
  branch-form `@save`/`@restore`. Implemented in `crates/zvm/src/quetzal.rs`.
- **Glulx:** a bare, standard **Glulx-Quetzal** save (`machine.save_quetzal()`) —
  `IFhd`/`CMem`/`Stks`/`MAll` only, no `GReg`, no `Glk ` chunk. Per the Glulx
  spec, `@save` pushes a call stub before suspending, so PC and FramePtr are
  recovered from the stack on restore rather than serialized as registers; the
  save is the same shape as the Z-machine's Quetzal. Implemented in
  `crates/gvm/src/exec.rs` (`save_quetzal`/`restore_quetzal`). Round-trip is
  verified internally (gvm unit tests); cross-interpreter golden-file interop is
  tracked separately under SQ-0229. Note this is a *different byte shape* from
  Glulx's host snapshot (`save_state`), which is why the archive writer consults
  the trigger instead of always snapshotting.

**Two kinds of Glulx `@save`/`@restore`, routed by how the game made the fileref
(SQ-0296).** The VM carries the target file's name and a `by_prompt` flag to the
host on each save/restore request (`Machine::pending_saveload_request` →
`SaveLoadRequest { name, by_prompt, restore }`):

  - **Player SAVE/RESTORE verb** — the game opens the file with
    `glk_fileref_create_by_prompt` (`by_prompt = true`). The host surfaces its
    save UI: `gvm-cli` prompts `Save to file:` / `Restore from file:`; the app
    opens its saves dialog. Lands as `<slug>.lanthorn` in the app, `<slug>.qzl`
    from `gvm-cli`.
  - **The game's OWN fixed-name saves** — `glk_fileref_create_by_name` /
    `create_by_usage` (`by_prompt = false`), e.g. Counterfeit Monkey's
    `_Counterfeit_Monkey-startup-data` init cache, its autosave, and undo slots.
    These are serviced **silently and automatically** — no prompt, no UI. `@save`
    writes `<game-dir>/<name>.qzl`; `@restore` reads that fixed path if present,
    else fails cleanly so the game runs its init. Because the file persists in the
    per-game `.save` dir, the next launch's boot `@restore` finds it and the game
    skips its (multi-second) init — measured for CM: ~3.5s first launch vs ~0.9s
    on relaunch. These internal `_`-prefixed files are hidden from the player
    saves list (app) and never prompt (both hosts). These stay bare `.qzl` files:
    they are the game's private storage, not player-facing save slots.

**Restoring one.** The extension no longer decides: `restore_from_file`
(`crates/app/src/engine_helpers.rs`) reads `meta.trigger` and, for `"ingame"`,
completes the suspended `@save`/`@restore` descriptor (`Engine::restore_game_save`
for a host load, `resume_restore` for the game's own `@restore`) and then reinstates
the archive's map/transcript/screen around it. A bare `.qzl`/`.sav` carried in from
another interpreter has no `meta.json` at all and takes the same descriptor path
with nothing to reinstate — that interchange route is untouched.

**A restore carries a layout width (SQ-0681).** A v4/v5 status routine lays its
bar out ONCE, from header byte $21 as it stood at boot, and thereafter only
re-cursors to the field columns it computed back then; declaring a narrower
screen later makes those moves illegal (ZMSD §8.7.2.3) and the digits land on the
room name. The app therefore floors the declared width at the width the running
story was laid out for (`GameSession::boot_screen_cols`, SQ-0679/0680) — and a
restore replaces the running story with one *another* session booted, at its own
width. Every restore that brings a screen with it (`restore_screen`, so: host
Save State resume, auto-resume at launch, and the in-game `@restore` of a
`.lanthorn`) raises that floor to the restored upper window's grid width, which
is the saved session's own frame of reference; the floor only ever grows, so
restoring a narrow save into a wide session changes nothing.
`reconcile_restored_screen_size` applies the same floor, so the restored grid
follows a *wider* pane and holds its own width in a narrower one, where the pane
simply clips the right of the bar. A bare `.qzl`/`.sav` carries no screen and,
per Quetzal, no usable header dimensions either, so its layout width is
unknowable: that path assumes the conventional 80 columns
(`note_bare_quetzal_width`) — wrong at worst by a clipped bar, versus a garbled
one for assuming this session's width.

**Every engine, the same deal (SQ-0556).** `@save` behaves identically wherever you
meet it: it writes a `.lanthorn`, the archive shows up in the saves manager, and it
comes back through *both* the game's own `restore` and the host restore path.
Glulx used to be the exception — the saves manager answered
`Glulx has no game-save (.qzl) format` — and no longer is. Its host restore now
takes the same road the game's own `@restore` takes: `restore_quetzal`, which
reverts RAM/stack/heap, pops `@save`'s call stub and stores the `-1` "just
restored" sentinel, and leaves the **live** Glk window model exactly where it
found it (Glulx spec §1.8.5 keeps windows, streams and I/O state out of a save
on purpose). That is why the bare `IFhd`/`CMem`/`Stks`/`MAll` shape is the right
thing to seal: it carries no serialized window tree, so there is nothing for a
restore to snap a *stale* set of windows back from.

One wrinkle is the host's alone. A saves-manager restore arrives while the session
is parked inside a `glk_select` belonging to the run you are leaving, and the save
file — by that same §1.8.5 — cannot say a word about it. Left in place it wedges
the restore twice: the VM re-reports the old suspension instead of resuming at the
restored PC, and your next command gets swallowed answering it. So the host
retires it (`Machine::abandon_pending_input`) and runs the save-verb tail out to
the next prompt, discarding that tail's output the way the Z-machine path does —
the archive's own transcript is about to be laid down over it. Verified against
real Adventure (Glulx) in `crates/app/tests/suites/glulx_ingame_save_host_restore.rs`:
save, play on, host-restore, and the game replays the reference run move for move
with an inventory that has forgotten everything picked up afterwards.

## Layer 2 — host Save State / Restore State (emulator snapshot)

lanthorn's own save-anywhere snapshot, explicit and per-slot. Triggered by Ctrl+S
/ `/save-state`, the named-slot saves manager, and the "Save State & quit" prompt.

It captures the **entire machine plus lanthorn's session context**: VM state, the
Glk window/stream tree and screen, the map, the transcript, turn history, and
metadata. Crucially it **includes the entire Glk file VFS** — every file a Glulx
game has written through Glk file streams — embedded in the `Glk ` snapshot
(`crates/gvm/src/glk.rs`, `GLK_SNAPSHOT_VERSION = 6`; the VFS has been embedded
since v4, SQ-0277, and restore still accepts v4 onward).

Save States are bundled into a self-contained `.lanthorn` archive
(`crates/app/src/archive.rs`). Inside the archive the engine-tagged VM save is
`game.glksave` for Glulx and `game.qzl` otherwise (the `save_ext` fallback, so
the Z-machine's Quetzal and the Scott VM's `Vm::snapshot` blob both land as
`game.qzl` — the recorded engine tag, not the extension, tells them apart on
restore). This is Scott Adams' **only** persistence layer: with no in-game
Quetzal save and no sidecar, its in-game `SAVE GAME` and the host Ctrl+S both
write here. Named slots, auto-save (per turn) and auto-load (resume on launch)
all operate on this layer.

## Layer 3 — automatic per-story persistence (no explicit save)

This layer needs **no player action and no Save State**. lanthorn keeps a small
per-story sidecar that it loads when the story opens and flushes after each turn
(only when it changed). It is what makes a game's own external-storage files
survive a plain quit — quit the game normally, relaunch, and the data is still
there. For example, Kerkerkruip's persistent scores/preferences stick across
sessions.

- **Z-machine — aux data.** Games that use the v5 `@save` / `@restore`
  auxiliary-file mechanism (save/restore of a memory table to a named external
  file) persist to `<base>/<story-key>.save/default.aux` — in the app
  (`crates/app/src/aux_store.rs`) and in `zvm-cli` (`ZAUX` format,
  `crates/zvm-cli/src/auxiliary.rs`), each keyed by the story key, not IFID.
- **Glulx — the Glk file VFS (new, SQ-0278).** Every file a Glulx game writes
  through Glk file streams now auto-persists to
  `<base>/<story-key>.save/default.glkvfs` — in the app
  (`crates/app/src/vfs_store.rs`) and in `gvm-cli`
  (`crates/gvm-cli/src/main.rs`), both keyed by the story key. The blob is
  the files-only `GVFS` codec (`gvm::glk::encode_files` / `decode_files`): magic
  `GVFS` + version `1` + length-prefixed name→bytes entries, big-endian, fully
  tolerant of a corrupt or foreign file (it just resets to empty, never panics).
  Session-scoped Glk temp files (VFS keys beginning with `__temp_`) are
  deliberately **not** persisted.

  Loaded at story-open (`main.rs`, alongside the aux load) and flushed per-turn
  dirty-gated (`persist_vfs_after_turn`), exactly mirroring the aux store. This is
  the automatic, no-explicit-save counterpart to Layer 2: Save State already
  embeds the full VFS per-slot, but Layer 3 is what preserves those files when the
  player never saves at all.

Deleting the sidecar (or `--aux off` in the CLIs) resets the game's stored data.

## Storage layout (SQ-0284)

All three hosts — app, `zvm-cli`, `gvm-cli` — store saves and sidecars in a
flat **per-game directory**, one directory per story, holding everything for
that game side by side:

```
<base>/<story-key>.save/
    default.aux        # Z-machine aux sidecar (Layer 3)
    default.glkvfs     # Glulx VFS sidecar (Layer 3)
    default.lanthorn   # the auto/singleton Save State slot (Layer 2)
    <slug>.lanthorn     # named saves — Save States AND in-game @save (app only);
                        #   meta.json's `trigger` says which wrote each one
    <slug>.qzl           # bare in-game @save files: the CLI hosts, a game's own
                        #   fixed-name storage (`_`-prefixed), and saves carried
                        #   in from other interpreters
    style.toml          # per-game style override (app only, layered over global)
    config.toml         # per-game non-style overrides (honor/borders/map panel)
```

`<story-key>` has **two rules**, because one disk image is no longer one game
(SQ-0850):

- A **loose story file** keys on its own **filename** (basename including
  extension, sanitized to filesystem-safe characters) — *not* the IFID. The
  same story file always maps to the same directory, and different files (even
  the same game shipped as `.z5` vs `.zblorb`) get separate directories.
- A story **mounted out of a disk image** keys on that story's own **release
  and serial** instead: `<slug>-r<release>-s<serial>`, e.g.
  `hitchhikers-guide-r59-s851108`. The slug is the canonical title from
  `cli_host::titles`, cut at its subtitle and truncated on a word boundary; it
  is there to be read, and the release and serial are what identify the build.
  A build the title table does not name slugs as `story`, which is still
  unique.

The image's filename cannot answer for a compilation: `Infocom Compilation 1
(19xx)(-).st` carries six games and `floppy2.ima` six more, and under a
filename key all of them shared one `default.lanthorn` and overwrote each
other in turn. Keying on the build gives three properties a filename never
had — renaming the image keeps the saves, a game that moves between disks in a
set keeps them, and two games on one disk cannot collide — and it is the same
identity this project already uses to say that *a disk image is a different
release*, not the same story on other media. So the Amiga, DOS and Atari ST
presses of Zork I r88/840726 share one directory, while Zork Zero's r296, r366
and r393 presses get three.

One helper answers for every host — `cli_host::storage::story_key_for` /
`story_key_at`, which `app` re-exports through `app::storage` — so the TUI and
`zvm-cli` cannot name one game's directory two ways.

The IFID is still computed and used for the story's *title* and for
interpreter-hint association, but it no longer keys any storage path.

The directory name carries a **`.save` suffix** (`<story-key>.save`, e.g.
`Zork1.z5.save/`) so it can never collide with the story file itself — this
matters for `zvm-cli`/`gvm-cli`, whose default `<base>` is the story's own
directory, where a directory named exactly `Zork1.z5` would collide with the
file `Zork1.z5` (SQ-0294).

`<base>` — the directory containing all per-game directories — defaults
differently per host, and every host accepts `--data-dir <path>` to override
it:

- **app** — `~/.lanthorn/saves` (i.e. `<user_dir>/saves`; follows
  `--user-dir` unless `--data-dir` is also given).
- **`zvm-cli` / `gvm-cli`** — the story file's own directory (so a story run
  from `~/games/zork1.z5` gets `~/games/zork1.z5/...`).

A save named `default` is a **reserved slug** — the app rejects an attempt to
create a named Save State or in-game save called `default`, since that name
is claimed by the auto/singleton slot. (The rejection re-opens the save-name
dialog so an in-game `SAVE` can be retried rather than lost.)

### Interactive `@save` / `@restore` in the CLIs

When `zvm-cli` / `gvm-cli` prompt for a filename on the **player's** SAVE /
RESTORE verb (a `glk_fileref_create_by_prompt` fileref in Glulx; always in the
Z-machine), a **bare name** (no path separator, e.g. `@save quick`) resolves
into the per-game directory — `<base>/<story-key>.save/quick.qzl` — matching the
`.qzl` extension automatically. A **path-bearing value** (e.g.
`@save /tmp/x.qzl`) is honored verbatim, bypassing the per-game directory
entirely.

A Glulx game's **own** fixed-name saves (`glk_fileref_create_by_name`, e.g. CM's
init cache) do **not** prompt: `gvm-cli` writes/reads `<story-key>.save/<name>.qzl`
silently (see Layer 1, SQ-0296).

### Map/transcript exports (SQ-0288)

The app's `/export-svg`, `/export-dot`, `/export-map`, and `/export-transcript`
commands write into the same per-game directory, using fixed default names —
`map.svg`, `map.dot`, `map.txt`, `transcript.txt` — overwriting on repeat
export. Each takes an optional `[file]` argument that resolves the same way as
`@save`/`@restore` above: a **bare name** lands in `<base>/<story-key>.save/`
(the format's extension is appended if the name has none), a **path-bearing
value** is honored verbatim.

### No migration (alpha)

There is **no migration** from the old IFID-keyed layout. Saves and sidecars
previously written as `<save_dir>/<ifid>.lanthorn`, `<ifid>.aux`, `<ifid>.gvfs`,
etc. are orphaned — lanthorn will not find or move them automatically. If you
have saves from before this change, either re-create them under the new
layout or manually move the files into the new `<base>/<story-key>.save/`
directory (renaming to the `default.*` / `<slug>.*` names above as needed).

## Where each thing lands

| Layer | Engine | Host | File |
|-------|--------|------|------|
| 1 — game's `@save`/`@restore` | Z-machine | app | `<base>/<story-key>.save/<slug>.lanthorn` (`trigger = "ingame"`; `game.qzl` inside is bare standard Quetzal) |
| 1 — game's `@save`/`@restore` | Z-machine | `zvm-cli` | `<base>/<story-key>.save/<slug>.qzl` (bare name) or verbatim path |
| 1 — player SAVE verb (`create_by_prompt`) | Glulx | app | `<base>/<story-key>.save/<slug>.lanthorn` (`trigger = "ingame"`; `game.glksave` inside is bare standard Glulx-Quetzal) |
| 1 — player SAVE verb (`create_by_prompt`) | Glulx | `gvm-cli` | `<base>/<story-key>.save/<slug>.qzl` (bare name) or verbatim path |
| 1 — game's own save (`create_by_name`, SQ-0296) | Glulx | app & `gvm-cli` | `<base>/<story-key>.save/<name>.qzl` — silent, no prompt; hidden from the saves list |
| 2 — Save State / Restore State | Z-machine | app | `<base>/<story-key>.save/default.lanthorn` or `<slug>.lanthorn` (`game.qzl` inside) |
| 2 — Save State / Restore State | Glulx | app | `<base>/<story-key>.save/default.lanthorn` or `<slug>.lanthorn` (`game.glksave` inside; embeds full Glk VFS) |
| 2 — Save State / Restore State | Scott Adams | app | `<base>/<story-key>.save/default.lanthorn` or `<slug>.lanthorn` (`game.qzl` inside = `Vm::snapshot` blob; Scott's only layer) |
| 3 — auto per-story (aux) | Z-machine | app | `<base>/<story-key>.save/default.aux` |
| 3 — auto per-story (aux) | Z-machine | `zvm-cli` | `<base>/<story-key>.save/default.aux` (`ZAUX`) |
| 3 — auto per-story (Glk VFS) | Glulx | app | `<base>/<story-key>.save/default.glkvfs` (`GVFS`) |
| 3 — auto per-story (Glk VFS) | Glulx | `gvm-cli` | `<base>/<story-key>.save/default.glkvfs` (`GVFS`) |
| export — `/export-svg`\|`-dot`\|`-dump`\|`-transcript` | either | app | `<base>/<story-key>.save/map.svg`\|`map.dot`\|`map.txt`\|`transcript.txt` (bare `[file]` arg) or verbatim path |

`<base>` and `<story-key>` are as defined in [Storage layout](#storage-layout-sq-0284)
above.

## `create_by_prompt` naming (SQ-0279)

`glk_fileref_create_by_prompt` suspends the VM for a host-chosen name rather than
resolving to a fixed per-usage slot. Write / append / read-write modes open a
name-entry prompt; read mode opens a picker over the story's existing Glk files.
The named file lives in the VFS like any other Glk file, so it auto-persists
per-story through the Layer 3 sidecar (`default.glkvfs`) and is embedded in
Layer 2 Save States — there is no separate on-disk file. `gvm-cli` prompts for the
name on stdin (blank cancels). This matches the layering above: a game reaching for
`create_by_prompt` is writing an *external named file*, which by the game's own
choice belongs in the automatic per-story (global) layer, not a save slot.

**Exception — `fileusage_SavedGame`.** A `create_by_prompt` stream opened for
saved-game usage does **not** resolve into a VFS slot at all: it's a host
conduit (`StreamKind::Null`) that discards writes and reads EOF, with no
`self.files` entry and nothing persisted to `default.glkvfs` or embedded in a
Save State. The library's post-`@save` verification is satisfied without storing
bytes: `note_stream_write` credits the stream (and records the slot's byte
length), so `glk_fileref_does_file_exist` reports the slot exists and a reopen +
seek-to-end reports the true save size — CM's SAVE verb otherwise printed "Save
failed." (SQ-0292). The game's `@save`/`@restore` always reaches the opcode —
even on a first-ever restore, with no prior save this session — and the *host*
decides success by writing/reading the actual `.qzl` (Layer 1, above). Net: the
VFS (Layer 3) now holds only the game's genuine external files — transcripts,
command recordings, and data files — never saves.

**Game-managed vs. player-prompted (SQ-0296).** The above concerns the
*player's* verb (`create_by_prompt`). A game that saves to a **fixed-name**
fileref (`create_by_name`/`create_by_usage` — CM's `_Counterfeit_Monkey-startup-data`
init cache, its autosave, undo slots) is routed differently: the host writes/reads
`<game-dir>/<name>.qzl` **silently, with no prompt**, keyed by the fileref name
the VM now reports (`SaveLoadRequest.by_prompt = false`). This is what makes CM's
boot cache auto-restore on relaunch (skipping its long init) and removes the
spurious boot prompts. Note a Glulx game may open such a slot as a `Data`-usage
VFS `File` stream rather than a `SavedGame` `Null` stream — CM does — so the
name/`by_prompt` routing covers both stream kinds.

## Known limitations (Glk file VFS)

- **The read picker is not usage-filtered** — it lists *all* of the story's VFS
  files, not only those matching the requested Glk usage class, because the `GVFS`
  codec does not record a per-file usage tag.
- **Text-mode newline translation is omitted** — Glk text-mode file streams are
  stored verbatim, with no platform newline translation.
