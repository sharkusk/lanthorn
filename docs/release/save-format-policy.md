# Save-format policy (beta)

[← back to README](../../README.md) · see also [The persistence model](../internals/persistence.md)

Until the first public beta, lanthorn's rule was *"pre-release: formats may break
freely, no back-compat"*. The beta flips that for the formats that live on a
user's disk between sessions. Every persisted byte format is now (a) enumerated,
(b) version-stamped where it is a private lanthorn format, and (c) pinned by a
round-trip freeze test, so **any change to a persisted format is deliberate** —
never an accident that silently corrupts a user's saves.

## The rule going forward

Changing the wire layout of any format in the table below requires, in the same
change:

1. **Bump its version marker** (the `*_VERSION` constant / `format_version`).
2. **Update the freeze test** that pins the constant (it will fail until you do —
   that is the point).
3. **Add a release-note entry** describing the break, plus a migration path
   (a tolerant reader for the old layout) *or* a documented, accepted break.

Pre-beta there is still **no obligation to read old files** (see the standing
"no back-compat before release" policy); the freeze machinery exists so that
*after* beta, breaks are conscious decisions with a paper trail — not surprises.

## Guarantee tiers

- **Public spec** — a standard interchange format defined outside lanthorn. We
  read and write it to the published spec and it stays interoperable with other
  interpreters. We do not get to "version" it; identity/compatibility is the
  spec's own (e.g. Quetzal `IFhd` release/serial/checksum).
- **Frozen (0.x)** — a private lanthorn format carrying a version marker, pinned
  by a freeze test. It may still break between 0.x versions, but only via the
  bump-and-note ritual above. A reader rejects a *newer* marker cleanly (empty /
  error, never a mis-parse).
- **Tolerant (unversioned by nature)** — TOML/JSON config and metadata. Missing
  fields default; unknown fields are ignored. Not byte-pinned; a `schema`/`format`
  integer guards the shape where one exists.

## Inventory

| Format | File / entry | Defined in | Version marker | Guarantee | Freeze test |
|---|---|---|---|---|---|
| Z-machine Quetzal (`@save`) | `game.qzl` inside `<slug>.lanthorn` (app); bare `<slug>.qzl` (`zvm-cli`) | `zvm/src/quetzal.rs` | none — IFF `FORM IFZS`, identity via `IFhd` | Public spec (Quetzal 1.4) | `quetzal::tests::round_trip_restores_full_state`, `…rejects_serial_mismatch` |
| Glulx-Quetzal (`@save`) | `game.glksave` inside `<slug>.lanthorn` (app); bare `<slug>.qzl` (`gvm-cli`) | `gvm/src/exec.rs` `save_quetzal` | none — spec-defined `FORM IFZS` | Public spec (Glulx §1.8) | `exec::tests::save_quetzal_is_a_wellformed_ifzs_container`, `…omits_greg_and_glk_chunks` |
| Host Save State — Z-machine | inside `.lanthorn` `game.qzl` | `zvm/src/quetzal.rs` (+ archive) | via archive `format_version` | Frozen (0.x) | archive round-trip tests |
| Host Save State — Glulx | inside `.lanthorn` `game.glksave` | `gvm/src/exec.rs` `save_state` (adds `GReg` + `Glk `) | `Glk ` chunk: `GLK_SNAPSHOT_VERSION = 6` | Frozen (0.x) | `glk::tests::snapshot_version_constant_is_frozen`, `…serialize_stamps_current_snapshot_version`, `…deserialize_rejects_future_snapshot_version`, `exec::tests::save_state_is_the_same_container_plus_our_own_chunks` |
| `.lanthorn` archive (map + save + transcript + screen + history + pictures + painted ground) | `<ifid>.lanthorn` (ZIP) | `app/src/archive.rs` | `Meta.format_version = 8` | Frozen (0.x) | `archive::tests::format_version_constant_is_frozen`, `…unknown_format_version_returns_err`, `…save_trigger_wire_names_are_pinned_and_round_trip`, archive round-trip tests |
| Z-machine aux data (v5 `@save`/`@restore` table) | `default.aux` | `app/src/aux_store.rs` + `zvm-cli/src/auxiliary.rs` | `ZAUX` magic + `VERSION = 1` | Frozen (0.x), cross-host | `aux_store::tests::version_constant_is_frozen`, `…decode_rejects_bumped_version`, `…encodes_canonical_zaux_bytes` |
| Glk file VFS sidecar | `default.glkvfs` | `gvm/src/glk.rs` `encode_files`/`decode_files` (path: `app/src/vfs_store.rs`) | `GVFS` magic + `u32` version `1` | Frozen (0.x) | `glk::tests::encode_files_roundtrips_and_skips_temp`, `…decode_files_rejects_bumped_gvfs_version` |
| Debug-coverage PC set | `default.pcs` | `app/src/pcset_store.rs` | `ZPCS` magic + `VERSION = 1` | Frozen (0.x) | `pcset_store::tests::version_constant_is_frozen`, `…decode_rejects_bumped_version`, `…codec_round_trips` |
| Map graph | `map.json` (inside `.lanthorn`) | `mapper/src/persist.rs` | JSON `version: 1` field | Tolerant (JSON) — carried by the archive | `mapper::persist::tests` round-trips |
| Per-story metadata | `info.json` (+ cover) | `app/src/story_info.rs`, `fetch_worker.rs` | JSON `format_version = 1`, `fetch_version = 1` | Tolerant (JSON) | `story_info::tests` |
| Global config | `config.toml` | `app/src/config.rs` | TOML `version` (`CONFIG_SCHEMA_VERSION = 1`) | Tolerant (TOML) | `config::tests` |
| Theme / per-game config | `style.toml`, `<ifid>.config.toml` | `app/src/config.rs`, `styles.rs` | none (TOML, field-tolerant) | Tolerant (TOML) | — |

## Version history

- **`.lanthorn` archive 4 → 5 (SQ-0531).** `meta.json` gained
  `trigger: "ingame" | "hoststate"`, recording whether the game's own `@save` or
  the host's Save State wrote the archive — and therefore which PC convention the
  `game.<ext>` bytes inside follow. Restore dispatches on it instead of on the
  file extension, because `@save` now writes an archive too.
  *Accepted break, no migration:* a pre-5 archive still loads (the field defaults
  to `"hoststate"`, which is what every archive written before the bump actually
  was), but a v5 archive is rejected by older builds, as the freeze machinery
  intends. Bare `.qzl`/`.glksave` interchange files are untouched.

- **`.lanthorn` archive 5 → 6 (SQ-0588).** A v6 (graphical Z-machine) archive now
  carries `display.json` — each graphics window's DISPLAY LIST plus the Blorb §11.3
  Current Palette it was drawn under — and **omits** `pictures/win-N.png` for every
  window whose replay reproduced the live canvas at save time. Storing what the story
  drew, rather than a picture of the result, is what lets restored art follow a later
  palette change: a canvas restored as pixels cannot be recoloured, only a replay of
  its ops under the new palette can.
  *Accepted break, no migration:* a pre-6 archive still loads and restores exactly as
  before (no `display.json` → the canvas PNGs are used, and those windows are not
  replayable, so their colours stay as saved — there are no ops to migrate). A v6
  archive is rejected by older builds, which matters more here than usual: an older
  build would find neither a PNG nor a list it understands for the omitted windows and
  would restore them **blank**. `pictures/win-N.png` remains as the per-window
  fallback, written whenever the save-time self-check finds that a window's recorded
  ops do not rebuild it.

- **`.lanthorn` archive — `pictures/ground.png` added, no version bump (SQ-0787).**
  A v6 archive now also carries the screen's PAINTED GROUND: the surface
  `erase_window` fills and stranded canvases accumulate on, under every window. It is
  the layer that made a resumed scopa show its main-menu cards beneath the restored
  hand, and a resumed Shogun lose its backdrop.
  *No bump, because the entry is purely additive:* nothing else is omitted for it, so
  an older build ignores the extra ZIP entry and restores exactly as it does today —
  the case that forced the 5 → 6 bump (an older reader finding neither a PNG nor a
  list it understood) does not arise. In the other direction a newer build reading an
  older archive finds no ground and **resets** the ground rather than inheriting the
  pre-restore screen's, which is the correct answer for every story that paints none.
  Stored as pixels rather than as a recipe: the ground's inputs are an unbounded
  stream of fills, so there is no bounded recipe to store — the justification the
  "persist the recipe" rule requires for a derived artifact.

- **`.lanthorn` archive 6 → 7 (SQ-0814).** `display.json` now also carries the two v6
  screen layers that ride beside the window canvases: the surviving `erase_window`
  FILLS (what each window's last erase painted, and in what order) and the canvas
  ANCHORS (where each window's art was painted, so a later window move strands it
  where the hardware left it). They are the ground's siblings — the same per-session
  fields no restore path touched — and the same defect: a resumed Journey wore three
  opaque bands from the screen it replaced, or lost the three the save carried.
  A RECIPE, not pixels, because unlike the ground they are bounded at one small struct
  per window however long the session runs.
  *Accepted break, no migration:* a pre-7 archive still loads and simply carries no
  layers, which restores them EMPTY — the correct answer, and the same reset every
  non-v6 archive gets. The bump is because the break runs the other way: an older
  build reading a version-7 archive would drop the layers silently and restore a
  screen still wearing the previous session's fills, which is precisely the bug.

- **`.lanthorn` archive 7 → 8 (SQ-0820).** `screen.json` now carries all three of a v6
  window's pixel-run layers instead of one. Beside `texts` (the window's own paint) go
  `streamed` — where the prose the window sent to the host transcript is currently
  SITTING on the glass (SQ-0697/SQ-0729) — and `retired`, the prose a `move_window` or
  `window_size` froze at coordinates the window no longer covers. Both are live screen
  state that only the story repaints, and a host Save State swaps memory under a story
  that never learns it happened: fmvpoker's "Current Bet:"/"10" legends live only in
  `streamed`, so a resumed hand came back with them missing from the pixel raster
  (invisible in cell mode, because the `cells` grid was archived all along), and Shogun
  one keypress in holds its whole nine-line title header in `retired` and lost it.
  A RECIPE, not pixels: the game's own runs in zvm's native pixel space, exactly as
  `texts` travels, so the archive stays terminal- and backend-neutral. The per-burst
  `stream_origin` is deliberately NOT carried — it is only meaningful between a clear
  and the read that follows it.
  *Accepted break, no migration:* a pre-8 archive still loads and simply restores the
  two layers EMPTY, which is what every archive written before the bump actually meant.
  The bump is for the other direction, as in 6 → 7: an older build reading a version-8
  archive would drop them silently and resume the game with prose missing from its
  screen, which is precisely the bug.

## Notes on identity vs. version

Quetzal and Glulx-Quetzal carry **no lanthorn version** — they are public
interchange formats. Their safety net is the `IFhd` identity chunk (story
release + serial + checksum): restoring a save into the wrong story is rejected
(`ZError::SaveMismatch` / `GError::BadSave`), which is the standard's own
compatibility mechanism. We deliberately keep these formats spec-clean so other
interpreters can read our `@save` files and vice-versa.

The three private binary sidecars (`ZAUX`, `ZPCS`, `GVFS`) and the two versioned
containers (the `.lanthorn` archive, the `Glk ` snapshot chunk) all reject a
**newer** version marker cleanly — an empty table/set/map, or a clean error —
rather than mis-parsing future bytes as the current layout. That reject behavior
is itself pinned by a freeze test, so a future bump has to consciously decide how
old readers see the new file.
