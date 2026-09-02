//! Auxiliary save-data codec + global-file backend (v5 `save/restore table`).
//! See docs/superpowers/specs/2026-06-26-aux-save-data-design.md.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Shared cross-host aux format: `ZAUX` magic + version + little-endian widths,
/// byte-identical to zvm-cli's `encode_aux` so one `default.aux` is readable by
/// both hosts (SQ-0300).
const MAGIC: &[u8; 4] = b"ZAUX";
const VERSION: u8 = 1;

/// Encode the aux table as the length-prefixed `ZAUX` v1 blob: `"ZAUX",
/// u8 version, u32 count`, then per entry `u32 name_len, name, u32 data_len,
/// data` (all little-endian). Byte-identical to zvm-cli's `encode_aux` so a
/// shared `default.aux` is cross-host readable. Deterministic (BTreeMap input).
pub fn encode_aux(table: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(table.len() as u32).to_le_bytes());
    for (name, data) in table {
        let nb = name.as_bytes();
        out.extend_from_slice(&(nb.len() as u32).to_le_bytes());
        out.extend_from_slice(nb);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// Decode aux bytes into the table. Tolerant on two axes: it accepts both the
/// current `ZAUX` format (magic present) and the legacy app format (no magic,
/// big-endian, u16 name length) for back-compat, and any truncation/overflow
/// yields whatever parsed so far (empty for non-aux bytes) — never panics.
pub fn decode_aux(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    if bytes.len() >= 4 && &bytes[..4] == MAGIC {
        decode_zaux(bytes)
    } else {
        decode_legacy(bytes)
    }
}

/// Parse the current `ZAUX` format (little-endian, u32 name length). Tolerant.
fn decode_zaux(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    // Reject an unknown version (matches zvm-cli's reference decoder): a future
    // format bump yields an empty table rather than a v1 mis-parse. Freeze policy
    // (docs/release/save-format-policy.md) — a bump is deliberate, never silent.
    if bytes.get(4) != Some(&VERSION) {
        return out;
    }
    let mut p = 5usize; // MAGIC (4) + VERSION (1), already validated
    let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
        let end = p.checked_add(n)?;
        let s = bytes.get(*p..end)?;
        *p = end;
        Some(s)
    };
    let count = match take(&mut p, 4) {
        Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        None => return out,
    };
    for _ in 0..count {
        let nl = match take(&mut p, 4) { Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize, None => break };
        let name = match take(&mut p, nl) { Some(b) => String::from_utf8_lossy(b).into_owned(), None => break };
        let dl = match take(&mut p, 4) { Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize, None => break };
        let data = match take(&mut p, dl) { Some(b) => b.to_vec(), None => break };
        out.insert(name, data);
    }
    out
}

/// Parse the legacy app format (no magic, big-endian, u16 name length). Tolerant.
fn decode_legacy(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut p = 0usize;
    let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
        let end = p.checked_add(n)?;
        let s = bytes.get(*p..end)?;
        *p = end;
        Some(s)
    };
    let count = match take(&mut p, 4) {
        Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        None => return out,
    };
    for _ in 0..count {
        let nl = match take(&mut p, 2) { Some(b) => u16::from_be_bytes([b[0], b[1]]) as usize, None => break };
        let name = match take(&mut p, nl) { Some(b) => String::from_utf8_lossy(b).into_owned(), None => break };
        let dl = match take(&mut p, 4) { Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize, None => break };
        let data = match take(&mut p, dl) { Some(b) => b.to_vec(), None => break };
        out.insert(name, data);
    }
    out
}

/// `<game_dir>/default.aux` (SQ-0284). The aux table is the game's singleton
/// side data, stored under the per-game directory keyed by story filename.
pub fn aux_path(game_dir: &Path) -> PathBuf {
    game_dir.join("default.aux")
}

/// Read the per-game global aux file (empty map if absent or unreadable).
pub fn read_global_aux(game_dir: &Path) -> BTreeMap<String, Vec<u8>> {
    match std::fs::read(aux_path(game_dir)) {
        Ok(bytes) => decode_aux(&bytes),
        Err(_) => BTreeMap::new(),
    }
}

/// Write the per-game global aux file (creating `game_dir` if needed).
///
/// Atomic (SQ-0644): the aux table is the game's singleton side data, rewritten
/// whenever the story calls `save table`, so a crash mid-write must leave the
/// previous table readable rather than a truncated one that decodes to garbage.
pub fn write_global_aux(game_dir: &Path, table: &BTreeMap<String, Vec<u8>>) -> std::io::Result<()> {
    crate::storage::atomic_write(&aux_path(game_dir), &encode_aux(table))
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;

    fn sample() -> BTreeMap<String, Vec<u8>> {
        let mut m = BTreeMap::new();
        m.insert("AB".to_string(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
        m.insert("".to_string(), vec![]); // empty key + empty value
        m
    }

    /// A one-entry table with the exact `ZAUX` v1 bytes both hosts must emit.
    fn cross_host_sample() -> BTreeMap<String, Vec<u8>> {
        let mut m = BTreeMap::new();
        m.insert("AB".to_string(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
        m
    }
    const ZAUX_BYTES: &[u8] = &[
        b'Z', b'A', b'U', b'X', 0x01, // magic + version
        0x01, 0x00, 0x00, 0x00, // count = 1 (LE)
        0x02, 0x00, 0x00, 0x00, b'A', b'B', // name_len 2 (LE) + "AB"
        0x04, 0x00, 0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, // data_len 4 (LE) + data
    ];

    /// Legacy app format: no magic, big-endian, u16 name length. Test-only
    /// helper reproducing the pre-SQ-0300 encoder to exercise back-compat.
    fn encode_legacy(table: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(table.len() as u32).to_be_bytes());
        for (name, data) in table {
            let nb = name.as_bytes();
            out.extend_from_slice(&(nb.len() as u16).to_be_bytes());
            out.extend_from_slice(nb);
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    #[test]
    fn codec_round_trips() {
        let m = sample();
        assert_eq!(decode_aux(&encode_aux(&m)), m);
    }

    // ── format freeze (docs/release/save-format-policy.md) ──
    // The ZAUX version is frozen at 1. Changing this constant must be a
    // deliberate format bump (update this pin + a migration/release note), never
    // an accidental drift — the assert forces the decision to be conscious.
    #[test]
    fn version_constant_is_frozen() {
        assert_eq!(VERSION, 1, "ZAUX version changed — see docs/release/save-format-policy.md");
    }

    #[test]
    fn decode_rejects_bumped_version() {
        // A future (v2) ZAUX file is rejected by today's reader (empty table),
        // never mis-parsed as v1. Symmetric with zvm-cli's reference decoder.
        let mut v2 = ZAUX_BYTES.to_vec();
        v2[4] = 0x02; // bump the version byte
        assert!(decode_aux(&v2).is_empty(), "a bumped-version ZAUX must not decode as v1");
    }

    #[test]
    fn encodes_canonical_zaux_bytes() {
        // Byte-identity with zvm-cli: both hosts assert against this same literal.
        assert_eq!(encode_aux(&cross_host_sample()), ZAUX_BYTES);
    }

    #[test]
    fn decodes_zaux_from_other_host() {
        assert_eq!(decode_aux(ZAUX_BYTES), cross_host_sample());
    }

    #[test]
    fn decodes_legacy_app_format() {
        // Back-compat: a pre-SQ-0300 (no-magic/BE/u16) file still loads.
        let m = sample();
        assert_eq!(decode_aux(&encode_legacy(&m)), m);
    }

    #[test]
    fn decode_tolerates_garbage() {
        assert!(decode_aux(b"\xff\xff\xffnonsense").is_empty());
        assert!(decode_aux(&[]).is_empty());
    }

    #[test]
    fn aux_path_is_default_aux_in_game_dir() {
        let dir = Path::new("/tmp/saves/Zork1.z5");
        let p = aux_path(dir);
        assert_eq!(p, PathBuf::from("/tmp/saves/Zork1.z5/default.aux"));
        assert_eq!(p.parent(), Some(dir), "stays in the game dir");
    }

    #[test]
    fn global_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("lanthorn-aux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_global_aux(&dir).is_empty(), "absent file → empty");
        write_global_aux(&dir, &sample()).unwrap();
        assert_eq!(read_global_aux(&dir), sample());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0644: a write that cannot complete must leave the PREVIOUS aux table on
    /// disk. `fs::write` truncated `default.aux` first, so a crash between truncate
    /// and write cost the story every table it had ever saved. A directory that
    /// admits no new files proves the temp-then-rename path: the write fails outright
    /// rather than half-happening (an in-place `fs::write` would have succeeded).
    #[test]
    fn an_interrupted_aux_write_keeps_the_previous_table() {
        let dir = std::env::temp_dir().join(format!("lanthorn-aux-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_global_aux(&dir, &sample()).unwrap();

        if !crate::storage::deny_new_files_in(&dir) {
            let _ = std::fs::remove_dir_all(&dir);
            return; // platform can't enforce it (or we're root) — skip
        }
        let mut next = BTreeMap::new();
        next.insert("XY".to_string(), vec![1, 2, 3]);
        let result = write_global_aux(&dir, &next);
        crate::storage::allow_new_files_in(&dir);

        assert!(result.is_err(), "a write that cannot complete must fail, not half-happen");
        assert_eq!(read_global_aux(&dir), sample(), "the previous table is still readable");
        assert!(crate::storage::leftover_temps(&dir).is_empty(), "no temp left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
