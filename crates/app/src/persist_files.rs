use std::io;
use std::path::{Path, PathBuf};
use mapper::mapper::Mapper;
use mapper::persist::from_json;

// ── Named save slots ──────────────────────────────────────────────────────────

/// Metadata for one discovered save file.
#[derive(Debug, Clone)]
pub struct SaveInfo {
    /// Absolute path to the `.lanthorn` file.
    pub path: PathBuf,
    /// Human-readable name (slug-form for named saves, "(default)" for the
    /// quick-save slot).
    pub name: String,
    /// Turn counter at save time.
    pub turns: u32,
    /// RFC3339 timestamp string (may be empty for legacy saves).
    pub saved_at: String,
    /// Detected room name at save time (None for legacy saves / no location signal).
    pub location: Option<String>,
    /// Score at save time (None for v4+ Z-machine, Glulx, and legacy saves).
    pub score: Option<i32>,
    /// True for the default (IFID-only) quick-save slot.
    pub is_default: bool,
    /// What wrote this save, and therefore whether its game bytes are portable
    /// (SQ-0531). A bare `.qzl` is always `Ingame` — it IS a standard game save;
    /// a `.lanthorn` reports the trigger recorded in its `meta.json`.
    pub trigger: crate::archive::SaveTrigger,
}

/// List all Save-State files in a game dir (SQ-0284).
///
/// Discovers `default.lanthorn` (default slot) and `<slug>.lanthorn` (named
/// slots) inside `game_dir`, reads their `Meta`, and returns sorted results:
/// default slot first, then named saves sorted by `saved_at` descending (newest
/// first). Files that fail to parse are silently skipped.
pub fn list_saves(game_dir: &Path) -> Vec<SaveInfo> {
    let entries = match std::fs::read_dir(game_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut infos: Vec<SaveInfo> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(fname) = path.file_name().and_then(|n| n.to_str()) else { continue };

        if !fname.ends_with(".lanthorn") {
            continue;
        }
        let is_default = fname == "default.lanthorn";

        // Read only meta.json; skip on failure (corrupt/unsupported → not listed).
        let meta = match crate::archive::read_archive_meta(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let name = if is_default {
            "(default)".to_string()
        } else {
            // The slug is the filename stem (`<slug>.lanthorn`).
            let slug = &fname[..fname.len() - ".lanthorn".len()];
            // Prefer the name stored in Meta, fall back to the slug.
            meta.name.clone().unwrap_or_else(|| slug.to_string())
        };

        infos.push(SaveInfo {
            path,
            name,
            turns: meta.turns,
            saved_at: meta.saved_at.clone(),
            location: meta.location.clone(),
            score: meta.score,
            is_default,
            trigger: meta.trigger,
        });
    }

    // Sort: default first, then by saved_at descending (newer saves sort earlier).
    infos.sort_by(|a, b| {
        match (a.is_default, b.is_default) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.saved_at.cmp(&a.saved_at),
        }
    });

    infos
}

/// Write a named Save State: `<game_dir>/<slug>.lanthorn` (SQ-0284).
///
/// `name` is sanitized into a filesystem-safe slug (lowercase alphanum +
/// hyphens). The IFID is retained only as archive metadata (identity/display).
/// The reserved `default` slug is rejected so a named save never clobbers the
/// auto/singleton slot.
///
/// `trigger` records WHO asked for the file (SQ-0531): the host's Save State, or
/// the game's own `@save`. Both write the same archive; the trigger is what tells
/// a later restore which PC convention `save.bytes` follows — and what lets the
/// saves list say honestly which saves are portable to other interpreters.
#[allow(clippy::too_many_arguments)]
pub fn save_named(
    game_dir: &Path,
    ifid: &str,
    name: &str,
    trigger: crate::archive::SaveTrigger,
    mapper: &Mapper,
    save: &crate::engine::EngineSave,
    screen: Option<&zvm::screen::ScreenState>,
    pics: &[(u8, Vec<u8>)],
    // `display`: the v6 display list + Current Palette (SQ-0588), or None for a
    // non-v6 story. Required rather than defaulted — a save path that silently
    // omits it writes an archive whose restored art can never be recoloured, which
    // is exactly the regression this parameter exists to make impossible.
    display: Option<&crate::archive::DisplayListDto>,
    // `ground`: the v6 painted ground as a PNG (SQ-0787), or None for a non-v6
    // story / a game that has never painted one. Required for the same reason
    // `display` is: a save path that omits it restores a screen still carrying
    // the ground of whatever was on the screen before the restore.
    ground: Option<&[u8]>,
    aux: &std::collections::BTreeMap<String, Vec<u8>>,
    turns: u32,
    location: Option<String>,
    score: Option<i32>,
    // The whole session, as one value. It used to be the five transcript slices
    // and nothing else, which is how the rewind/replay history came to be dropped
    // here — see `SessionRecord` and SQ-1090.
    session: &crate::archive::SessionRecord<'_>,
) -> io::Result<()> {
    let path = named_save_path(game_dir, name)?;

    let saved_at = rfc3339_now();
    let meta = crate::archive::Meta {
        format_version: crate::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(ifid.to_string()),
        name: Some(name.to_string()),
        turns,
        saved_at,
        location,
        score,
        trigger,
    };
    // Command history is per-game, not per-slot, so a named save deliberately
    // writes none — and says so by NAME. The rewind/replay history is not covered
    // by that reasoning and must go in: this line used to read `&[], &[]`
    // positionally, the comment above appeared to explain both, and every named
    // Save State went to disk with no `history/` in it at all (SQ-1090).
    let session = crate::archive::SessionRecord { command_history: &[], ..*session };
    crate::archive::save_archive_meta_pics(&path, mapper, save, screen, aux, meta, &session, pics, display, ground)
}

/// The engine state to seal into an archive's `game.<ext>` entry for `trigger`
/// (SQ-0531).
///
/// [`SaveTrigger::Ingame`] picks the GAME's own save convention — the bytes a
/// foreign interpreter can read. For the Z-machine that IS
/// [`crate::engine::Engine::save_state`] (it returns `Machine::save_quetzal()`
/// verbatim, descriptor PC and all, because the VM is suspended on its `@save`).
/// Glulx keeps two shapes apart, so this reaches for the bare standard
/// Glulx-Quetzal (`IFhd+CMem+Stks+MAll`) instead of the host snapshot's
/// `GReg`+`Glk `-bearing one. Scott has no game-native save format at all, so both
/// triggers seal its VM snapshot — the only bytes it has.
pub fn game_save_bytes(session: &dyn crate::engine::Engine, trigger: crate::archive::SaveTrigger) -> crate::engine::EngineSave {
    let host = session.save_state();
    match session.as_any().downcast_ref::<crate::glulx_session::GlulxSession>() {
        Some(g) if trigger.is_portable() => {
            crate::engine::EngineSave::new(host.engine, host.format_version, g.save_quetzal())
        }
        _ => host,
    }
}

/// Remove a save file.
pub fn delete_save(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)
}

/// Resolve `name` to the `.lanthorn` path it would write to inside
/// `game_dir`, without touching the filesystem — the same validation
/// `save_named` applies to its own `path`, factored out so a caller can ask
/// "what file would this name hit?" before deciding to write it (SQ-0648: the
/// overwrite-confirm prompt needs to know the target ahead of the write).
pub fn named_save_path(game_dir: &Path, name: &str) -> io::Result<PathBuf> {
    let slug = slugify(name);
    if slug.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "save name is empty after sanitization"));
    }
    if crate::storage::is_reserved_slug(&slug) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "\"default\" is a reserved save name"));
    }
    Ok(game_dir.join(format!("{}.lanthorn", slug)))
}

/// The display name of the save ALREADY at `path`, if any (SQ-0648).
///
/// Two different typed names can slugify to the same filename (`"Before
/// Troll"` and `"before, troll!"` both land on `before-troll.lanthorn`), so an
/// overwrite-confirm prompt built from the typed name alone would hide exactly
/// the collision it exists to surface. This reads the CURRENT occupant's name
/// instead — `Meta::name` if the archive parses, else the filename stem —
/// mirroring `list_saves`'s own name resolution. `None` when nothing exists at
/// `path` yet, meaning there is nothing to confirm.
pub fn existing_save_display_name(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let name = crate::archive::read_archive_meta(path).ok().and_then(|m| m.name);
    Some(name.unwrap_or_else(|| {
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("this save").to_string()
    }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a human-readable name to a filesystem-safe slug.
///
/// Lowercases, replaces runs of non-alphanumeric chars with a single hyphen,
/// and trims leading/trailing hyphens.
fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_sep = true; // suppress leading hyphens
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('-');
            last_was_sep = true;
        }
    }
    // Trim trailing hyphen.
    if slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Return the current time as an RFC3339 string (UTC, second precision).
fn rfc3339_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_secs(secs)
}

/// A human-friendly default save name from the current time: `YYYY-MM-DD HHMM`
/// (UTC). Used to prefill the save-name dialog's placeholder.
pub fn default_save_name() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let min = (s / 60) % 60;
    let hour = (s / 3600) % 24;
    let days = s / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02} {:02}{:02}", year, month, day, hour, min)
}

/// A file's modification time as an RFC3339 string (UTC, second precision), or
/// an empty string if it can't be read. Used to timestamp bare `.qzl` game
/// saves, which carry no metadata of their own.
pub fn rfc3339_mtime(path: &Path) -> String {
    use std::time::UNIX_EPOCH;
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| rfc3339_from_secs(d.as_secs()))
        .unwrap_or_default()
}

/// Format seconds-since-Unix-epoch as `YYYY-MM-DDTHH:MM:SSZ` (no external crate).
fn rfc3339_from_secs(s: u64) -> String {
    let sec = s % 60;
    let min = (s / 60) % 60;
    let hour = (s / 3600) % 24;
    let days = s / 86400; // days since 1970-01-01
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month, day, hour, min, sec)
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Algorithm: shift epoch to 1 Mar 0000 for easier leap-year math.
    days += 719468; // days from year 0 to 1970-01-01 (proleptic Gregorian)
    let era = days / 146097;
    let doe = days % 146097; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month prime [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// List `*.qzl` game saves in `game_dir` as SaveInfo rows (for the in-game
/// restore picker). All `.qzl` files in the per-game dir belong to this story
/// (SQ-0284); there is no default `.qzl`, so none is skipped in practice. `.qzl`
/// saves carry no metadata, so they're timestamped from the file's mtime and the
/// slug (filename stem) is the display name.
pub fn list_qzl(game_dir: &Path) -> Vec<SaveInfo> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(game_dir) {
        for e in rd.flatten() {
            let p = e.path();
            let Some(fname) = p.file_name().and_then(|n| n.to_str()) else { continue };
            if let Some(slug) = fname.strip_suffix(".qzl") {
                // Skip the game's OWN internal saves (create_by_name — CM's init
                // cache, undo, autotesting), written by the session to a fixed
                // `<name>.qzl`. Their names begin with `_`; player-save slugs
                // (slugified: lowercase alphanumerics + hyphens) never do, so
                // this cleanly hides internal files from the player saves list.
                if slug.starts_with('_') {
                    continue;
                }
                let name = slug.to_string();
                let saved_at = rfc3339_mtime(&p);
                out.push(SaveInfo {
                    path: p, name, turns: 0, saved_at, location: None, score: None, is_default: false,
                    trigger: crate::archive::SaveTrigger::Ingame,
                });
            }
        }
    }
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    out
}

/// List the game's OWN internal `.qzl` saves in `game_dir` (SQ-0296): the
/// `_`-prefixed fixed-name files `list_qzl` hides from the player saves list
/// (CM's init cache, undo, autotesting, etc). The story-picker info panel
/// shows these so the user can see what's on disk; the in-game restore picker
/// still uses `list_qzl` and never sees them.
pub fn list_qzl_auto(game_dir: &Path) -> Vec<SaveInfo> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(game_dir) {
        for e in rd.flatten() {
            let p = e.path();
            let Some(fname) = p.file_name().and_then(|n| n.to_str()) else { continue };
            if let Some(slug) = fname.strip_suffix(".qzl") {
                if !slug.starts_with('_') {
                    continue;
                }
                let name = slug.to_string();
                let saved_at = rfc3339_mtime(&p);
                out.push(SaveInfo {
                    path: p, name, turns: 0, saved_at, location: None, score: None, is_default: false,
                    trigger: crate::archive::SaveTrigger::Ingame,
                });
            }
        }
    }
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    out
}

pub fn load_map(path: &Path) -> Option<Mapper> {
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return None,
    };
    match from_json(&contents) {
        Ok(mapper) => Some(mapper),
        Err(e) => {
            eprintln!("lanthorn: failed to parse map file {}: {}", path.display(), e);
            None
        }
    }
}

/// Write a bare Quetzal `.qzl` save.
///
/// Atomic (SQ-0644): a `.qzl` is the game's own save slot, and the fixed-name
/// internal ones (`_undo`, `_startup`) are rewritten constantly — truncating the
/// previous save before the new one is durable is how a crash costs the player
/// both.
pub fn save_game(path: &Path, machine: &zvm::cpu::exec::Machine) -> std::io::Result<()> {
    crate::storage::atomic_write(path, &machine.save_quetzal())
}

/// Write a named in-game save: `<game_dir>/<slug>.qzl` (bare standard Quetzal).
///
/// `name` is sanitized into a filesystem-safe slug (lowercase alphanum +
/// hyphens); the reserved `default` slug is rejected (SQ-0284).
pub fn save_game_named(game_dir: &Path, name: &str, machine: &zvm::cpu::exec::Machine) -> io::Result<PathBuf> {
    let path = game_save_path(game_dir, name)?;
    save_game(&path, machine)?;
    Ok(path)
}

/// Resolve a named `.qzl` game-save path inside `game_dir`, rejecting an empty
/// or reserved (`default`) slug.
fn game_save_path(game_dir: &Path, name: &str) -> io::Result<PathBuf> {
    let slug = slugify(name);
    if slug.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "save name is empty after sanitization"));
    }
    if crate::storage::is_reserved_slug(&slug) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "\"default\" is a reserved save name"));
    }
    Ok(game_dir.join(format!("{}.qzl", slug)))
}

/// A `.qzl` file is a BARE game save: standard Quetzal with no wrapper, carried in
/// from another interpreter (lanthorn's own `@save` writes a `.lanthorn` archive
/// now — SQ-0531). Anything else is a `.lanthorn`, whose PC convention is recorded
/// in its `Meta::trigger`, not in its extension. Restore sites use this only to
/// tell "raw bytes" from "archive"; which convention the bytes follow comes from
/// the trigger.
pub fn is_game_save(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "qzl")
}

pub fn restore_game(path: &Path, machine: &mut zvm::cpu::exec::Machine) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    machine.complete_restore_success(&bytes).map_err(|e| match e {
        zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
        other => format!("restore failed: {:?}", other),
    })
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;
    use mapper::mapper::Mapper;
    use mapper::direction::Direction;
    use mapper::persist::to_json;

    /// A named Save State carries the rewind/replay history (SQ-1090).
    ///
    /// It did not, and nothing failed: `save_named` passed `&[], &[]` positionally
    /// for `history` and `command_history` under a comment that explained only the
    /// second. Every named save went to disk with no `history/` entry at all, and
    /// restoring one came back with nothing to rewind through — 22 turns played,
    /// no history, no error. `SessionRecord` is why that spelling is gone; this is
    /// the case that fails if it comes back.
    #[test]
    fn a_named_save_keeps_the_turn_history_and_drops_only_the_command_history() {
        let Some(machine) = fake_machine() else { return };
        let dir = std::env::temp_dir().join(format!("bm-named-hist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mapper = mapper::mapper::Mapper::default();

        let mut history = Vec::new();
        for t in 1..=3u32 {
            crate::history::record_turn(&mut history, t, "north", vec![t as u8; 4], &mapper, false, "You are here.");
        }
        assert_eq!(history.len(), 3, "three turns recorded before saving");
        let commands = vec!["north".to_string(), "look".to_string()];

        super::save_named(
            &dir, "TEST-IFID", "slot", crate::archive::SaveTrigger::HostState, &mapper,
            &es(&machine), Some(&machine.screen), &[], None, None, &machine.aux_data, 3, None, None,
            &crate::archive::SessionRecord {
                history: &history,
                command_history: &commands,
                ..crate::archive::SessionRecord::empty()
            },
        )
        .expect("save_named");

        let ac = crate::archive::load_archive(&dir.join("slot.lanthorn")).expect("load");
        assert_eq!(ac.history.len(), 3, "the rewind/replay history must survive a named save");
        assert_eq!(ac.history[0].save, vec![1, 1, 1, 1], "and byte-identically");
        assert_eq!(ac.history[2].save, vec![3, 3, 3, 3]);
        // The other blank IS deliberate: command history is per-game, not per-slot.
        assert!(ac.command_history.is_empty(), "a named slot carries no command history");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_round_trips() {
        // `lanthorn-test-<pid>` was ALSO `turn.rs`'s name for its own scratch, and
        // under `cargo test` the two tests share a process — so each was deleting
        // the other's directory at the end of its case (SQ-1131).
        let dir = crate::scratch_dir("map-round-trip");
        let path = dir.join("ZCODE-1-x-0.map.json");
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        std::fs::write(&path, to_json(&m)).unwrap();
        let loaded = load_map(&path).expect("loads");
        assert_eq!(loaded.graph.connections(), m.graph.connections());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_is_none() {
        assert!(load_map(Path::new("/no/such/lanthorn.map.json")).is_none());
    }

    #[test]
    fn load_corrupt_is_none() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("lanthorn-test-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.map.json");
        std::fs::write(&path, b"this is not valid json {{{").unwrap();
        assert!(load_map(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_round_trips_layers_and_names() {
        use mapper::direction::Direction;
        let mut dir = std::env::temp_dir();
        dir.push(format!("lanthorn-layers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ZCODE-1-x-0.map.json");
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let region = mapper::layer::planar_region(&m.graph, 2);
        let l = mapper::layer::move_region(&mut m.graph, &region, mapper::layer::MoveTarget::New)
            .expect("peel");
        m.graph.set_layer_name(l, "Basement".into());
        std::fs::write(&path, to_json(&m)).unwrap();
        let loaded = load_map(&path).expect("loads");
        assert_eq!(loaded.graph.layer_of(2), l);
        assert_eq!(loaded.graph.layer_name(l), "Basement");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn game_save_restore_round_trips_with_czech() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return /* skip */ };
        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        // step a few instructions so dynamic memory differs from the pristine image
        for _ in 0..50 { let _ = machine.step(); }
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("lanthorn-save-{}.qzl", std::process::id()));
        save_game(&tmp, &machine).unwrap();
        let mut m2 = zvm::cpu::exec::Machine::new(
            zvm::memory::Memory::new(std::fs::read(&fixture).unwrap()).unwrap()
        );
        m2.init_caps();
        restore_game(&tmp, &mut m2).expect("restore ok");
        let _ = std::fs::remove_file(&tmp);
    }

    // ── slugify ───────────────────────────────────────────────────────────────

    #[test]
    fn slugify_produces_fs_safe_names() {
        assert_eq!(super::slugify("Before Troll"), "before-troll");
        assert_eq!(super::slugify("  hello  world  "), "hello-world");
        assert_eq!(super::slugify("CAPS and Symbols!!"), "caps-and-symbols");
        assert_eq!(super::slugify("a--b"), "a-b");
        assert_eq!(super::slugify("   "), "");
    }

    // ── rfc3339_now ───────────────────────────────────────────────────────────

    #[test]
    fn rfc3339_now_looks_like_timestamp() {
        let ts = super::rfc3339_now();
        // Format: 2026-06-18T12:34:56Z (20 chars)
        assert_eq!(ts.len(), 20, "expected 20-char RFC3339 timestamp, got '{ts}'");
        assert!(ts.ends_with('Z'), "should end with Z");
        assert!(ts.contains('T'), "should contain T separator");
        // Year should be plausible (>= 2024).
        let year: u32 = ts[0..4].parse().unwrap();
        assert!(year >= 2024, "year {year} looks wrong");
    }

    // ── list_saves / save_named / delete_save ─────────────────────────────────

    fn make_temp_dir(tag: &str) -> std::path::PathBuf {
        crate::scratch_dir(&format!("saves-test-{tag}"))
    }

    /// The Z-machine `EngineSave` for `m` (Quetzal bytes, `"zmachine"` tag).
    fn es(m: &zvm::cpu::exec::Machine) -> crate::engine::EngineSave {
        crate::engine::EngineSave::new(crate::archive::DEFAULT_ENGINE, 1, m.save_quetzal())
    }

    /// Build a machine from the czech.z5 fixture, or return None to skip.
    fn fake_machine() -> Option<zvm::cpu::exec::Machine> {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return None };
        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut m = zvm::cpu::exec::Machine::new(mem);
        m.init_caps();
        for _ in 0..50 {
            let _ = m.step();
        }
        Some(m)
    }

    #[test]
    fn save_named_round_trip() {
        let Some(machine) = fake_machine() else { return };
        let dir = make_temp_dir("named");
        let mut mapper = Mapper::default();
        mapper.observe(1, "Foyer", None);

        let ifid = "ZCODE-1-TEST00-0001";
        super::save_named(&dir, ifid, "before-troll", crate::archive::SaveTrigger::HostState, &mapper, &es(&machine), Some(&machine.screen), &[], None, None, &machine.aux_data, 42, None, None, &crate::archive::SessionRecord::empty())
            .expect("save_named ok");

        // Path is `<slug>.lanthorn` inside the game dir (no ifid in the name).
        assert!(dir.join("before-troll.lanthorn").exists(), "named save lands at <slug>.lanthorn");

        let saves = super::list_saves(&dir);
        assert_eq!(saves.len(), 1, "should have 1 save");
        let s = &saves[0];
        assert_eq!(s.name, "before-troll");
        assert_eq!(s.turns, 42);
        assert!(!s.is_default);
        assert!(!s.saved_at.is_empty(), "saved_at should be set");

        // The file should be loadable as an archive; the ifid is retained as meta.
        let ac = crate::archive::load_archive(&s.path).expect("load_archive ok");
        assert_eq!(ac.meta.turns, 42);
        assert_eq!(ac.meta.name.as_deref(), Some("before-troll"));
        assert_eq!(ac.meta.ifid.as_deref(), Some(ifid), "ifid retained as archive metadata");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_named_persists_location_and_score() {
        // SQ-0411: the save summary (location + score) round-trips through Meta into
        // list_saves and a full archive load.
        let Some(machine) = fake_machine() else { return };
        let dir = make_temp_dir("summary");
        let mapper = Mapper::default();
        let ifid = "ZCODE-1-TEST00-0411";
        super::save_named(&dir, ifid, "at-troll", crate::archive::SaveTrigger::HostState, &mapper, &es(&machine), Some(&machine.screen), &[], None, None, &machine.aux_data, 7, Some("The Troll Room".into()), Some(10), &crate::archive::SessionRecord::empty())
            .expect("save_named ok");

        let saves = super::list_saves(&dir);
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0].location.as_deref(), Some("The Troll Room"));
        assert_eq!(saves[0].score, Some(10));

        let ac = crate::archive::load_archive(&saves[0].path).expect("load_archive ok");
        assert_eq!(ac.meta.location.as_deref(), Some("The Troll Room"));
        assert_eq!(ac.meta.score, Some(10));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_named_rejects_reserved_default_slug() {
        let Some(machine) = fake_machine() else { return };
        let dir = make_temp_dir("reserved-lanthorn");
        let mapper = Mapper::default();
        let ifid = "ZCODE-1-TEST00-0009";

        // "Default" slugifies to "default" — reserved for the auto/singleton slot.
        let err = super::save_named(&dir, ifid, "Default", crate::archive::SaveTrigger::HostState, &mapper, &es(&machine), Some(&machine.screen), &[], None, None, &machine.aux_data, 1, None, None, &crate::archive::SessionRecord::empty())
            .expect_err("reserved slug must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(!dir.join("default.lanthorn").exists(), "must not clobber the default slot");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_saves_ordering_default_first() {
        let Some(machine) = fake_machine() else { return };
        let dir = make_temp_dir("order");
        let mapper = Mapper::default();
        let ifid = "ZCODE-1-TEST00-0002";

        // Write the default archive (`default.lanthorn`).
        let default_path = crate::storage::default_state_path(&dir);
        crate::archive::save_archive(&default_path, &mapper, &es(&machine), Some(&machine.screen), &machine.aux_data, &[], &[], &[], &[], &[], &[])
            .expect("default save ok");

        // Write two named saves.
        super::save_named(&dir, ifid, "save-a", crate::archive::SaveTrigger::HostState, &mapper, &es(&machine), Some(&machine.screen), &[], None, None, &machine.aux_data, 10, None, None, &crate::archive::SessionRecord::empty()).unwrap();
        // Small sleep between named saves so timestamps differ, but since we
        // can't sleep in tests, we directly patch the timestamps via the archive
        // — instead, just verify ordering constraint is maintained.
        super::save_named(&dir, ifid, "save-b", crate::archive::SaveTrigger::HostState, &mapper, &es(&machine), Some(&machine.screen), &[], None, None, &machine.aux_data, 20, None, None, &crate::archive::SessionRecord::empty()).unwrap();

        let saves = super::list_saves(&dir);
        assert_eq!(saves.len(), 3, "should find 3 saves (1 default + 2 named)");
        assert!(saves[0].is_default, "default save must be first");
        // Remaining two are named; order between them is by saved_at desc (both
        // written in the same second in tests, so we just check they are present).
        let names: Vec<&str> = saves[1..].iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"save-a"), "save-a should be present");
        assert!(names.contains(&"save-b"), "save-b should be present");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_saves_skips_non_archive_files() {
        let dir = make_temp_dir("skip");

        // Write a non-archive file matching the extension.
        std::fs::write(dir.join("notanarchive.lanthorn"), b"garbage")
            .unwrap();

        let saves = super::list_saves(&dir);
        assert!(saves.is_empty(), "garbage file should be skipped");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_qzl_lists_qzl_by_stem_newest_first_and_skips_lanthorn() {
        let dir = make_temp_dir("list-qzl");
        std::fs::write(dir.join("default.lanthorn"), b"x").unwrap();
        std::fs::write(dir.join("quick.qzl"), b"x").unwrap();
        std::fs::write(dir.join("older.qzl"), b"x").unwrap();
        let out = super::list_qzl(&dir);
        assert_eq!(out.len(), 2); // .lanthorn excluded
        assert!(out.iter().all(|q| q.path.extension().unwrap() == "qzl"));
        assert!(out.iter().all(|q| !q.is_default && q.turns == 0));
        assert!(out.iter().any(|q| q.name == "quick")); // name = stem

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_qzl_hides_game_managed_underscore_saves() {
        // The game's OWN fixed-name saves (create_by_name: CM's init cache, undo,
        // autotesting) land as `_<name>.qzl` and must NOT appear in the player's
        // saves list. Player-save slugs (slugified: no leading `_`) still do.
        let dir = make_temp_dir("list-qzl-underscore");
        std::fs::write(dir.join("myslot.qzl"), b"x").unwrap();
        std::fs::write(dir.join("_Counterfeit_Monkey-startup-data.qzl"), b"x").unwrap();
        std::fs::write(dir.join("_autotesting.qzl"), b"x").unwrap();
        let out = super::list_qzl(&dir);
        assert_eq!(out.len(), 1, "only the player save is listed");
        assert_eq!(out[0].name, "myslot");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_qzl_auto_lists_only_underscore_saves() {
        // The mirror of list_qzl: game dir with a player save + two auto
        // (game-managed) saves. list_qzl returns only the player save;
        // list_qzl_auto returns only the two `_`-prefixed ones.
        let dir = make_temp_dir("list-qzl-auto");
        std::fs::write(dir.join("default.lanthorn"), b"x").unwrap();
        std::fs::write(dir.join("quicksave.qzl"), b"x").unwrap();
        std::fs::write(dir.join("_undo-x.qzl"), b"x").unwrap();
        std::fs::write(dir.join("_startup.qzl"), b"x").unwrap();

        let player = super::list_qzl(&dir);
        assert_eq!(player.len(), 1, "player list excludes underscore saves");
        assert_eq!(player[0].name, "quicksave");

        let auto = super::list_qzl_auto(&dir);
        assert_eq!(auto.len(), 2, "auto list has only the two underscore saves");
        let names: Vec<&str> = auto.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"_undo-x"));
        assert!(names.contains(&"_startup"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_save_removes_file() {
        let Some(machine) = fake_machine() else { return };
        let dir = make_temp_dir("delete");
        let mapper = Mapper::default();
        let ifid = "ZCODE-1-TEST00-0004";

        super::save_named(&dir, ifid, "to-delete", crate::archive::SaveTrigger::HostState, &mapper, &es(&machine), Some(&machine.screen), &[], None, None, &machine.aux_data, 5, None, None, &crate::archive::SessionRecord::empty()).unwrap();
        let saves = super::list_saves(&dir);
        assert_eq!(saves.len(), 1);
        let path = saves[0].path.clone();

        super::delete_save(&path).expect("delete ok");
        let saves_after = super::list_saves(&dir);
        assert!(saves_after.is_empty(), "save should be gone after delete");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── export round-trip (save_game / restore_game) ──────────────────────────

    /// SQ-0644: `.qzl` writes went through `fs::write`, which truncates the target
    /// before producing a byte — and the fixed-name internal saves (`_undo`,
    /// `_startup`) are rewritten constantly, so a crash mid-write cost the player both
    /// the new save and the old one. The write is temp-then-rename now: a directory
    /// that admits no new files makes it fail outright, where the in-place write would
    /// have truncated a perfectly good save first.
    #[test]
    fn an_interrupted_qzl_write_keeps_the_previous_save() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return /* skip */ };
        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        for _ in 0..50 { let _ = machine.step(); }

        let dir = std::env::temp_dir().join(format!("lanthorn-qzl-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("quick.qzl");
        save_game(&path, &machine).unwrap();
        let first = std::fs::read(&path).unwrap();

        // Move the machine on so the next save would differ.
        for _ in 0..50 { let _ = machine.step(); }
        if !crate::storage::deny_new_files_in(&dir) {
            let _ = std::fs::remove_dir_all(&dir);
            return; // platform can't enforce it (or we're root) — skip
        }
        let result = save_game(&path, &machine);
        crate::storage::allow_new_files_in(&dir);

        assert!(result.is_err(), "a write that cannot complete must fail, not half-happen");
        assert_eq!(std::fs::read(&path).unwrap(), first, "the previous save is byte-intact");
        assert!(crate::storage::leftover_temps(&dir).is_empty(), "no temp left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_round_trip_bytes_match_save_quetzal() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return };
        let mem = zvm::memory::Memory::new(story.clone()).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        for _ in 0..50 { let _ = machine.step(); }

        let tmp = std::env::temp_dir().join(format!("lanthorn-export-rt-{}.qzl", std::process::id()));
        save_game(&tmp, &machine).unwrap();

        // Bytes on disk should equal machine.save_quetzal().
        let on_disk = std::fs::read(&tmp).unwrap();
        assert_eq!(on_disk, machine.save_quetzal(), "exported file bytes must match save_quetzal()");

        // restore_game into a fresh machine should succeed.
        let mem2 = zvm::memory::Memory::new(story).unwrap();
        let mut machine2 = zvm::cpu::exec::Machine::new(mem2);
        machine2.init_caps();
        restore_game(&tmp, &mut machine2).expect("restore ok");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn import_keeps_mapper_unchanged() {
        // After restore_game the mapper is unmodified (standard saves have no map).
        use mapper::mapper::Mapper;
        use mapper::direction::Direction;

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return };
        let mem = zvm::memory::Memory::new(story.clone()).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        for _ in 0..50 { let _ = machine.step(); }

        let tmp = std::env::temp_dir().join(format!("lanthorn-import-map-{}.qzl", std::process::id()));
        save_game(&tmp, &machine).unwrap();

        // Build a mapper with rooms.
        let mut mapper = Mapper::default();
        mapper.observe(1, "West of House", None);
        mapper.observe(2, "Forest", Some(Direction::N));

        let room_count_before = mapper.graph.rooms().count();
        let connections_before = mapper.graph.connections().len();

        // restore_game should NOT touch the mapper.
        let mem2 = zvm::memory::Memory::new(story).unwrap();
        let mut machine2 = zvm::cpu::exec::Machine::new(mem2);
        machine2.init_caps();
        restore_game(&tmp, &mut machine2).expect("restore ok");

        assert_eq!(mapper.graph.rooms().count(), room_count_before, "mapper rooms unchanged after import");
        assert_eq!(mapper.graph.connections().len(), connections_before, "mapper connections unchanged after import");

        let _ = std::fs::remove_file(&tmp);
    }

    // Minimal valid v4 story buffer, matching the layout of zvm's own
    // (crate-private, cfg(test)-only) `header::tests_support::sample_story`.
    fn sample_story_v4() -> Vec<u8> {
        let mut buf = vec![0u8; 0x400];
        buf[0x00] = 4; // version
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        buf[0x06] = 0x00; buf[0x07] = 0x40; // initial_pc = 0x0040
        buf[0x08] = 0x02; buf[0x09] = 0x00; // dictionary = 0x0200
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x40; // abbrev_table = 0x0040
        buf
    }

    #[test]
    fn restore_game_completes_descriptor_of_a_gamesave_qzl() {
        // Build a v4 machine that @saves G0; capture the game-save .qzl (pending_save set
        // => descriptor PC), then restore_game() must complete it: G0==2, pc past the save.
        use zvm::cpu::exec::{Machine, StepResult};
        use zvm::memory::Memory;
        let mut buf = sample_story_v4();
        buf[0x40] = 0xB5; buf[0x41] = 0x10; buf[0x42] = 0xBA; // save->G0 ; quit
        let mut m = Machine::new(Memory::new(buf).unwrap());
        m.state.pc = 0x40;
        assert_eq!(m.step(), StepResult::SaveRequest);
        let blob = m.save_quetzal();               // descriptor PC (0x41), pending_save set
        m.complete_save(true);
        // Persist the game save and restore it via the game-save path.
        let tmp = std::env::temp_dir().join(format!("bm-gs-{}.qzl", std::process::id()));
        std::fs::write(&tmp, &blob).unwrap();
        m.do_store(Some(0x10), 0x99); m.state.pc = 0x00AB;
        super::restore_game(&tmp, &mut m).expect("restore game save");
        assert_eq!(m.global(0), 2, "game-save restore completes the @save descriptor (store 2)");
        assert_eq!(m.state.pc, 0x42, "resumes at the post-@save address");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn save_game_named_writes_bare_qzl() {
        // Build a v4 machine at an @save SaveRequest (pending_save set => descriptor PC),
        // mirroring the Task 1 fixture but without completing the save.
        use zvm::cpu::exec::{Machine, StepResult};
        use zvm::memory::Memory;
        let mut buf = sample_story_v4();
        buf[0x40] = 0xB5; buf[0x41] = 0x10; buf[0x42] = 0xBA; // save->G0 ; quit
        let mut m = Machine::new(Memory::new(buf).unwrap());
        m.state.pc = 0x40;
        assert_eq!(m.step(), StepResult::SaveRequest);

        // Own directory, not the bare temp root: the slug fixes the FILE name, so
        // writing it beside every other run's copy is a race (SQ-0812).
        let dir = std::env::temp_dir().join(format!("bm-named-qzl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = super::save_game_named(&dir, "slot one", &m).unwrap();
        assert!(path.to_string_lossy().ends_with("slot-one.qzl"));
        assert_eq!(std::fs::read(&path).unwrap(), m.save_quetzal(), "bare Quetzal bytes");

        // The reserved `default` slug is rejected for game saves too.
        let err = super::save_game_named(&dir, "default", &m).expect_err("reserved slug rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_game_fails_gracefully_on_wrong_story() {
        // Restoring a save from czech.z5 into a machine loaded with a *different* story
        // (or same story but different state check) should return an error, not panic.
        // We test by saving from czech and attempting to restore into a fresh czech machine
        // after corrupting the bytes.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return };

        // Write clearly-invalid bytes.
        let tmp = std::env::temp_dir().join(format!("lanthorn-bad-save-{}.qzl", std::process::id()));
        std::fs::write(&tmp, b"this is not a quetzal save at all").unwrap();

        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();

        let result = restore_game(&tmp, &mut machine);
        assert!(result.is_err(), "restore of corrupt file should return Err");

        let _ = std::fs::remove_file(&tmp);
    }
}
