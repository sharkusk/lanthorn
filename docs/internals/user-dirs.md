# Where lanthorn keeps the user's files

Design for SQ-1721, SQ-1722 and SQ-1723, settled with the user on 2026-10-06.
Lane A builds the roots (SQ-1721 + SQ-1722); lane B builds the shared IFDB
store on top of them (SQ-1723).

## The three roots

Everything lanthorn writes for a user hangs off three roots, carried together
as one value, `UserDirs { config, data, cache }` — the same "facts that travel
together" rule as `DataRoots` and `MachineBoot`. `DataRoots` (catalogue /
player / documents / cache) is derived from a `UserDirs`, never from a bare
path.

| root | holds |
|---|---|
| **config** | `config.toml`, `style.toml` |
| **data** | `saves/` (the catalogue and the default player), `users/<name>/`, `documents/`, the user's system disks (`system_fonts::user_media_dir`) |
| **cache** | only what can be regenerated without network traffic or user action (today: `miss_cache`). Fetched IFDB records and covers are **data**, not cache: the OS and users purge caches, and refetching means unprompted IFDB traffic. |

### Platform defaults (new installs only)

| | config | data | cache |
|---|---|---|---|
| macOS | `~/Library/Application Support/lanthorn` | same as config | `~/Library/Caches/lanthorn` |
| Linux / other Unix | `$XDG_CONFIG_HOME/lanthorn` (`~/.config/lanthorn`) | `$XDG_DATA_HOME/lanthorn` (`~/.local/share/lanthorn`) | `$XDG_CACHE_HOME/lanthorn` (`~/.cache/lanthorn`) |
| Windows | `%APPDATA%\lanthorn` | same as config | `%LOCALAPPDATA%\lanthorn` |

The folder name is plain `lanthorn` (no reverse-DNS bundle id). Resolved with
the `dirs` crate's base directories plus our own name — not `ProjectDirs`,
whose names differ per platform. Linux deliberately splits config from data:
that is what XDG asks for, and a user who wants one folder keeps (or creates)
`~/.lanthorn`.

### Precedence

1. **Explicit roots** win: a `UserDirs` handed in by an embedding host
   through the library API (mobile containers, store builds with their own
   sync), or `--user-dir` / `--data-dir` on the command line. `--user-dir X`
   keeps meaning today's single-folder layout under X (config, `saves/`,
   `documents/`, `cache/` all inside X). `--data-dir` keeps standing in for the
   catalogue base, as `DataRoots::resolve` documents.
2. **Legacy**: if `~/.lanthorn` exists, it is used whole, exactly as today
   (single-folder layout, cache inside it). No migration, no prompt, no
   opt-in move command.
3. **Platform defaults** above.

The TUI and every embedding host go through the same resolver, so on one
machine they share config and saves.

The Docker image passes `--user-dir /data/.lanthorn` explicitly, so container
volumes keep the single-folder layout `docs/internals/docker.md` describes.

### The home directory (SQ-1721)

There is exactly one "home directory" helper, built on `dirs::home_dir()`
(on Windows that is the profile from the Known Folder API, so an unset `HOME`
no longer matters). Every former `std::env::var("HOME")` in production code —
`config.rs` `default_user_dir`, `system_fonts.rs` `user_media_dir`,
`colors.rs` `expand_path` (`~` expansion), `main.rs` — goes through it or
through `UserDirs`. Nothing falls back to `.` (the working directory).

## The shared IFDB store (SQ-1723)

A story's fetched IFDB record and cover are kept **once per IFDB entry**, not
once per copy:

```
<catalogue>/ifdb/<tuid>.json        the record (formerly <copy>.save/info.json)
<catalogue>/ifdb/<tuid>.<ext>       the fetched cover, in the format IFDB served
                                    (.png, .jpg, …; formerly <copy>.save/cover.png)
```

* Flat: an entry holds exactly two files, so no per-entry folder. `ifdb/`
  cannot collide with a story folder, which always ends in `.save`.
* The cover keeps its original bytes and takes the extension of its real
  format; a lookup tries the known image extensions.
* It lives in the **catalogue**, shared by every player (SQ-1676), as does each
  copy's IFDB link.
* Each copy's own `<copy>.save/` keeps only its link state — the tuid it is
  linked to, not-found, or a user's "Wrong game?" choice — plus saves, map,
  notes and per-game settings. Refreshing an entry updates every copy linked
  to it; relinking a copy just points it at another entry.
* A blorb's own frontispiece still outranks a fetched cover, as today.

### Adoption of existing per-copy files

An existing `<copy>.save/info.json` / `cover.png` is read as before. The first
time one is used (or on the next fetch), it is **moved** into `ifdb/` under the
copy's tuid, so copies cannot drift apart again. If the shared store already
holds that entry, the newer fetch wins, and the other copy's stale files are
removed. Nothing is refetched to do this.

## Docs

README describes the released build: the new locations go in with
`*Next release:*` tags until the release that ships them. The guide, the
config template comments and `docs/internals/persistence.md` /
`saves.md` / `docker.md` are updated in the same lanes.
