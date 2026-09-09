# `gvm`'s fileref seam: giving a Glulx game durable files

[← back to README](../../README.md) · see also [The persistence model](persistence.md) (lanthorn's own three-layer scheme, of which this seam is Layer 1's routing plus Layer 3) · [Reviewing `zvm` as a crate someone else depends on](zvm-embedding-review.md) (§11, the sibling review of `gvm`'s public surface)

This page is for someone embedding [`gvm`](../../crates/gvm) — Glulx plus Glk —
in their own host, who is not lanthorn and has never read `crates/app/`. It
answers one question: *what do I have to build so a Glulx story's files, and
its own `@save`/`@restore`, survive between runs of my program?* `gvm` takes
zero external dependencies and does no disk I/O of its own (the crate's own
rule; see `crates/gvm/src/lib.rs`'s crate docs), so every byte that reaches a
disk anywhere in this story does so because a host wrote it. This is that
seam, fully.

lanthorn is used throughout as the worked example, because it is the one host
this seam has actually been built for — but nothing here is lanthorn-specific,
and the SQ-1416 incident quoted below ([§d](#d-the-name-to-disk-boundary)) is
exactly the kind of defect a new embedder will otherwise rediscover: a place
where gvm's naming rule and a host's own disk convention silently drifted
apart.

## a. The model: an in-memory VFS, not a filesystem

Every Glk fileref and file stream a Glulx story opens (`glk_fileref_create_*`,
`glk_stream_open_file*`) is serviced against an **in-memory virtual
filesystem** living entirely inside `gvm`: `Model::files` (`crates/gvm/src/glk.rs`),
a `BTreeMap<String, Vec<u8>>` from a sanitized fileref name to its bytes.
`Model` is the `glk` field of `Machine`, so this VFS's lifetime is the
`Machine`'s — one VFS per game session, not a process-global. The VM never
touches a real disk: a story that thinks it is writing a preferences file, a
score table, or a save is really just mutating this map, and nothing more
happens to those bytes until the host reads `Machine::vfs_bytes()` and writes
them somewhere itself.

**Reset on the story's own `@restart` — verified, and easy to miss.** Glulx's
`restart` opcode (`0x0122`, spec §2.9 — what a compiled game executes for its
own RESTART command, e.g. Inform's library action) runs entirely inside
`Machine::step`, and `Machine::op_restart` (`crates/gvm/src/exec.rs`)
unconditionally does `self.glk = Model::new()`: a fresh, empty VFS, along with
every window, stream and fileref. This happens **mid-step, with no
`StepResult` announcing it** — the host's next `step()` call just continues as
`Continue`, unaware that the VFS it may have been tracking as `vfs_dirty()`
just vanished and came back clean (a fresh `Model`'s dirty flag is `false`).
A host that dirty-gates its sidecar flush (§c below) will not flush after
this — there is nothing "dirty" to flush — so any file the story wrote and the
host had not yet persisted is gone until the *next* time the host itself
reloads a sidecar into a *new* `Machine`. This is a different event from a
host's own external restart (tearing down its `Machine` and building a new
one from scratch, which lanthorn does for its own `/restart` and startup
resume): that path is entirely the host's to control, and lanthorn carries the
live VFS forward into the new `Machine` explicitly
(`session.vfs_bytes()` → the new session's `load_vfs`, `crates/app/src/reset.rs`'s
`carry_vfs`). The in-game opcode gives the host no such chance — it fires
inside the *same* `Machine`, and by the time `step()` returns, the wipe has
already happened.

**Two kinds of session-transient state ride alongside the VFS but are not part
of it.** `Model::saved_game_files` (a name → last-written byte length) and
`Model::vfs_dirty` are both explicitly *not* serialized by
[`Machine::vfs_bytes`](#c-what-a-host-must-persist-the-vfs-sidecar) — they
exist only to answer a story's own existence/size probes
(`glk_fileref_does_file_exist`, a reopen-and-seek-to-end) within one process
run, because a `fileusage_SavedGame` fileref's stream (see [§b](#b-which-stepresults-a-host-must-answer)) is
fully decoupled from the VFS: no `self.files` entry is ever created for it, so
a plain restart or process relaunch would otherwise report a real on-disk
save as absent. `Machine::seed_saved_game_file(name, size)` is how a host
reseeds that index at boot from whatever save files it finds on disk — see
[§d](#d-the-name-to-disk-boundary) for how a host derives `name`.

## b. Which `StepResult`s a host must answer

Three distinct `StepResult` variants belong to this seam. `pending_saveload_request()`
is the router for the middle one — read it before deciding how to service a
save/restore, because "who asked" changes what a host does:

| `StepResult` | Fires when | Host reads | Host resolves with |
|---|---|---|---|
| [`NeedFilename { usage, fmode }`](../../crates/gvm/src/exec.rs) | The story calls `glk_fileref_create_by_prompt` — it wants a name from the *player*, not a fixed one | `usage` (Glk fileusage bits — Data/SavedGame/Transcript/InputRecord + flags), `fmode` | [`Machine::supply_filename(Some(name))`](../../crates/gvm/src/exec.rs) (binds a fileref) or `supply_filename(None)` (player cancelled → the Glk NULL fileref) |
| [`SaveRequest`](../../crates/gvm/src/exec.rs) | The story executed `@save` (spec §1.3.2/§1.8.2) against a fileref/`SavedGame`-usage stream the host must service | [`Machine::pending_saveload_request()`](../../crates/gvm/src/exec.rs) → [`SaveLoadRequest { name, by_prompt, restore: false }`](../../crates/gvm/src/exec.rs) | Write [`Machine::save_quetzal()`](../../crates/gvm/src/exec.rs) wherever `name`/`by_prompt` says it belongs (§d), then [`Machine::complete_save(ok)`](../../crates/gvm/src/exec.rs) |
| [`RestoreRequest`](../../crates/gvm/src/exec.rs) | The story executed `@restore` against a host-serviced target | same `SaveLoadRequest`, `restore: true` | Read the bytes and call [`Machine::complete_restore_quetzal(&bytes)`](../../crates/gvm/src/exec.rs) on success, else [`Machine::complete_restore_failure()`](../../crates/gvm/src/exec.rs) |

[`Machine::is_saveload_pending()`](../../crates/gvm/src/exec.rs) is `true`
between a `SaveRequest`/`RestoreRequest` and its completion — a guard against
firing an unrelated host-side snapshot (e.g. an exit auto-save) mid-suspension,
which would capture an un-popped `@save` call stub.

`complete_restore_quetzal` is for a bare, standard Glulx-Quetzal blob — what
`save_quetzal()`/`@save` itself produces, and the shape any other Glulx
interpreter's save is in too. [`Machine::complete_restore_success(&blob)`](../../crates/gvm/src/exec.rs)
is a *different* entry point for a host's own richer snapshot format (adding
`GReg`/`Glk ` chunks via [`Machine::save_state()`](../../crates/gvm/src/exec.rs)) —
not part of this seam; see [persistence.md](persistence.md)'s Layer 2 for what
that buys a host that wants save-anywhere rather than only the story's own
save points.

**A fourth case never reaches the host at all.** Not every `@save`/`@restore`
targets a fileref — the Glulx spec hands the opcode whatever writable Glk
stream the game opened, and a **memory stream** (`glk_stream_open_memory`) or
a read-only **Blorb resource stream** (`glk_stream_open_resource`) is a legal,
self-contained target: the bytes already live inside the VM (memory) or were
handed to it by the host at open time (a resource chunk), so there is nothing
for the host to read or write. `crates/gvm/src/exec.rs`'s `@save`/`@restore`
opcode handlers detect `StreamKind::Memory`/a resource stream and build/apply
the Quetzal blob **in process**, resolving with plain `StepResult::Continue`
— `pending_saveload_request()` stays `None` throughout. A game shipping a
pre-computed save as a Blorb `Data` chunk to skip an expensive boot
initialization relies on exactly this path. A host implementing this seam
never needs to do anything for it beyond keeping stepping; it is called out
here only so an embedder does not go looking for a `StepResult` that will
never come.

## c. What a host must persist: the VFS sidecar

`@save`/`@restore` (§b) already gives a story's *save files* somewhere to
land. What is left is everything else a story writes through an ordinary Glk
file stream — preferences, score tables, transcripts it opens itself, a
`Data`-usage cache — none of which suspends the VM at all (`glk_stream_open_file`
opens and writes synchronously against the in-memory VFS). Skip this and none
of that survives past the process exiting: the game re-inits from scratch
every launch, silently, because the files it thinks it wrote were never real
outside memory.

- **What's in it.** [`Machine::vfs_bytes()`](../../crates/gvm/src/exec.rs) encodes
  the whole `Model::files` map as a standalone, self-describing blob via
  [`gvm::glk::encode_files`](../../crates/gvm/src/glk.rs): magic `GVFS` + `u32`
  version (currently `1`) + `u32` count, then per entry a length-prefixed name
  and a length-prefixed byte blob, all big-endian. Keys beginning with
  `__temp_` (session-scoped Glk temp files from `glk_fileref_create_temp`) are
  skipped — they're meaningless past this run. [`Machine::load_vfs(&bytes)`](../../crates/gvm/src/exec.rs)
  is the inverse ([`gvm::glk::decode_files`](../../crates/gvm/src/glk.rs)), and is
  **fully tolerant of corruption**: a wrong magic, unknown version, invalid
  UTF-8 name, or truncation all decode to an empty map rather than panicking
  or erroring, so a foreign or damaged sidecar just starts the game with no
  files rather than refusing to boot.
- **When to write it.** [`Machine::vfs_dirty()`](../../crates/gvm/src/exec.rs) is
  set whenever the VFS is mutated (a file created, truncated, written, or
  deleted) and cleared by [`Machine::clear_vfs_dirty()`](../../crates/gvm/src/exec.rs) —
  gate every flush on it rather than writing unconditionally. lanthorn flushes
  at two points: once right after boot (`crates/app/src/startup.rs`) — a
  story can write a file during its own init (Counterfeit Monkey's boot
  cache), and skipping this flush would lose it to an immediate quit before
  the first turn ever runs — and again after every turn, dirty-gated
  (`persist_vfs_after_turn`, `crates/app/src/turn.rs`). `gvm-cli`
  (`crates/gvm-cli/src/main.rs`) checks the same flag once per drive-loop
  iteration. Neither host writes on every `step()` call — only when the flag
  says something changed.
- **When to read it back.** Once, at story-open, **before** the first
  `step()` — `crates/app/src/glulx_session.rs`'s `new_with_store` calls
  `machine.load_vfs(vfs_bytes)` immediately after `Machine::with_glk`
  constructs the machine and before anything runs, because a story may read a
  file it wrote last session as part of its own boot sequence. `load_vfs`
  after execution has begun would silently discard whatever the story had
  already written in this session.
- **What breaks if it's skipped.** Nothing crashes — a Glulx story degrades
  gracefully to an empty VFS on every launch, per the tolerant-decode
  guarantee above — but any story that uses external storage for something
  the player would expect to persist (Kerkerkruip's scores/preferences,
  Counterfeit Monkey's boot-time init cache) quietly loses it every time,
  and a game with a multi-second init pays that cost on every single launch
  instead of once. This is a *silent* failure mode, not a loud one: nothing
  in the story's own protocol tells the host it forgot.
- **A related, easy-to-miss failure mode is the SQ-1416 incident** —
  described in full in [§d](#d-the-name-to-disk-boundary) — where the sanitizer's
  spelling and the host's on-disk convention drifted apart and every
  `SavedGame`-usage `@save`/`@restore` silently missed its file. It is not a
  VFS-sidecar bug (`SavedGame`-usage streams are decoupled from the VFS
  entirely — see [§d](#d-the-name-to-disk-boundary)) but it is the same *class*
  of bug: a host assuming gvm's name and its own filename agree, without a
  single seam enforcing it.

## d. The name-to-disk boundary

`gvm` owns exactly one naming decision — how a story's raw fileref argument
becomes the string that indexes the VFS map (or, for a `SavedGame`-usage
fileref, the string a host keys its own save file on). Everything past that
string — where it lives on disk, what extension it gets, whether it is
exposed to the player as a named save slot or hidden as the game's own private
storage — is the host's, and `gvm` has no opinion on it.

**The sanitizer** ([`Model::sanitize_fileref_name`](../../crates/gvm/src/glk.rs),
the Glk spec §3.7-recommended algorithm, transcribed from cheapglk's
`cgfref.c`): delete every character in `" \ / > < : | ? *` from the raw name,
keep only the part before the first `.`, fall back to `"null"` if that leaves
nothing, then append a suffix that depends solely on the fileusage type bits
(`usage & fileusage_TypeMask`, i.e. the low 4 bits — the `fileusage_TextMode`
flag bit does not affect it):

| `fileusage` type | Suffix |
|---|---|
| `fileusage_Data` (`0x00`) | `.glkdata` |
| `fileusage_SavedGame` (`0x01`) | `.glksave` |
| `fileusage_Transcript` (`0x02`) / `fileusage_InputRecord` (`0x03`) | `.txt` |
| anything else | *(none)* |

Every fileref-creating call (`fileref_create`/`glk_fileref_create_by_name`,
`fileref_create_prompted`/`glk_fileref_create_by_prompt`,
`fileref_create_from`/`glk_fileref_create_from_fileref` inherits the source's
already-sanitized name unchanged) runs this same rule, so two filerefs a
story opens on the same raw name and usage always resolve to the same VFS key
— which is exactly what lets a `create_by_name` game's own `@save` and later
`@restore` find each other across relaunches.

**`fileusage_SavedGame` is the one usage that never touches `self.files` at
all.** A stream opened on such a fileref is `StreamKind::Null`
(`crates/gvm/src/glk.rs`) — a host conduit that discards writes and reads EOF —
because a save's real bytes are meant to go through `@save`/`@restore`'s
suspension (§b), not through the VFS. `Model::note_stream_write` /
`Model::clear_saved_game_file` are how `gvm` keeps `glk_fileref_does_file_exist`
and a reopen-and-seek-to-end truthful for this kind of stream without ever
storing a byte in `self.files`.

**lanthorn's worked example — and the SQ-1416 incident.** lanthorn's own
on-disk convention for a `create_by_name` `SavedGame` slot predates the
`.glksave` suffix above and is unrelated to it: `<game-dir>/<name>.qzl`, where
`<name>` is the *raw* fileref argument the game used (e.g. Counterfeit
Monkey's `_Counterfeit_Monkey-startup-data`). When SQ-1416 added the spec's
usage suffix to the sanitizer, the app code that crosses this boundary —
`drive_auto`'s save/restore path and `seed_saved_games`'s existence-index seed,
both in `crates/app/src/glulx_session.rs` — had been written assuming gvm's
sanitized name and lanthorn's disk stem were the *same string*. They stopped
being the same string the moment the suffix landed: `drive_auto` started
writing/reading `<store>/<name>.glksave.qzl` instead of `<store>/<name>.qzl`,
and `seed_saved_games` seeded the existence index under the bare `<name>`
instead of `<name>.glksave`, so `does_file_exist` never found a real on-disk
save — silently, because both operations "succeed" (a write to the wrong
path, a seed that finds nothing to complain about). Both of the app's own
in-crate tests for this path failed on CI, which is how it was caught.

The fix (`851ed241`) is the shape any embedder should copy: **one pair of
named functions is the entire boundary, crossed in both directions, so it can
never silently drift again**:

```rust
const GLK_SAVEDGAME_SUFFIX: &str = ".glksave";

/// gvm's already-sanitized SavedGame fileref name ("foo.glksave") ->
/// the host's own on-disk basename ("foo").
fn saved_game_disk_stem(glk_name: &str) -> &str {
    glk_name.strip_suffix(GLK_SAVEDGAME_SUFFIX).unwrap_or(glk_name)
}

/// The reverse: an on-disk basename -> the fully-sanitized Glk name
/// gvm's SavedGame existence index is keyed by.
fn saved_game_glk_name(disk_stem: &str) -> String {
    format!("{disk_stem}{GLK_SAVEDGAME_SUFFIX}")
}
```

`saved_game_disk_stem` is what `drive_auto` uses to turn
`pending_saveload_request().name` into `<store>/<stem>.qzl`; `saved_game_glk_name`
is what `seed_saved_games` uses to turn each `<store>/*.qzl` it finds on disk
back into the name `Machine::seed_saved_game_file` should index. **The
suffix is gvm's; the `.qzl` extension and the directory layout are the
host's** — an embedder choosing its own on-disk convention should still
isolate the crossing exactly this way, in one place, rather than assuming the
two strings agree at every call site.

## e. A minimal host

`crates/gvm/examples/run_story.rs` is a complete, compiled, runnable stdin/stdout
host — `cargo run -p lanthorn-gvm --example run_story -- story.gblorb` — that
answers this whole seam: it loads and dirty-flushes a `<story>.glkvfs` VFS
sidecar, prompts on stdin for a non-`SavedGame` `NeedFilename`, and writes/reads
a single fixed `<story>.glksave.qzl` save slot for `SaveRequest`/`RestoreRequest`.
It is a **single-slot simplification** of §d's routing — one save file
regardless of `name`/`by_prompt` — rather than lanthorn's or `gvm-cli`'s full
per-name, prompted-vs-silent scheme; read that file's module doc and its
`StepResult` match arms for the real thing, in real (and tested) code rather
than a doc snippet that can drift. The shape:

```rust
// Load the sidecar once, before the first step() — never after.
if let Ok(bytes) = fs::read(&vfs_path) {
    m.load_vfs(&bytes);
    m.clear_vfs_dirty();
}

loop {
    match m.step() {
        // ...
        StepResult::SaveRequest => {
            let ok = fs::write(&save_path, m.save_quetzal()).is_ok();
            m.complete_save(ok);
        }
        StepResult::RestoreRequest => match fs::read(&save_path) {
            Ok(bytes) if m.complete_restore_quetzal(&bytes) => {}
            _ => m.complete_restore_failure(),
        },
        StepResult::NeedFilename { usage, .. } => {
            if usage & 0x0f == 0x01 {
                m.supply_filename(Some(format!("__prompt_{}__", usage & 0x0f)));
            } else {
                // prompt on stdin; blank cancels -> supply_filename(None)
            }
        }
        // ...
    }
    // Dirty-gated, every iteration — never unconditional.
    if m.vfs_dirty() {
        let _ = fs::write(&vfs_path, m.vfs_bytes());
        m.clear_vfs_dirty();
    }
}
```

## Known gaps

- **The in-game `@restart` VFS wipe (§a) has no host-visible signal at all.**
  A host that wants to survive it (carry the live VFS across a story-initiated
  RESTART, not just its own external one) would need to snapshot `vfs_bytes()`
  before every `step()` and diff it after — there is currently no cheaper way
  to notice.
- **`gvm`'s VFS codec (`GVFS`) records no per-file usage tag**, so a host
  building a `create_by_prompt` read picker over existing files (§b) cannot
  filter it to the fileusage class the story actually asked for; see
  [persistence.md](persistence.md)'s "Known limitations" for lanthorn's
  version of the same gap.
