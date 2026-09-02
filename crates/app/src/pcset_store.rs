//! Executed-PC coverage sidecar codec + per-game file backend (SQ-0449). The
//! debug inspector's cumulative "ever executed" set persists here so coverage
//! (the blue disassembly lines) survives across `--debug` runs. Mirrors
//! `aux_store.rs`'s length-prefixed, tolerant-decode format.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Sidecar format: `ZPCS` magic + version + little-endian `u32` count, then each
/// PC as a little-endian `u32`. A set has no order, so the on-disk order is
/// unspecified.
const MAGIC: &[u8; 4] = b"ZPCS";
const VERSION: u8 = 1;

/// Encode the PC set as the `ZPCS` v1 blob: `"ZPCS", u8 version, u32 count`,
/// then `count` × `u32` PC (all little-endian).
pub fn encode(set: &HashSet<u32>) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + set.len() * 4);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(set.len() as u32).to_le_bytes());
    for &pc in set {
        out.extend_from_slice(&pc.to_le_bytes());
    }
    out
}

/// Decode PC-set bytes. Tolerant: a missing/foreign magic yields an empty set,
/// and any truncation yields whatever parsed so far — never panics.
pub fn decode(bytes: &[u8]) -> HashSet<u32> {
    let mut out = HashSet::new();
    if bytes.len() < 4 || &bytes[..4] != MAGIC {
        return out;
    }
    // Reject an unknown version (freeze policy, docs/release/save-format-policy.md):
    // a future format bump yields an empty set rather than a v1 mis-parse.
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
        match take(&mut p, 4) {
            Some(b) => { out.insert(u32::from_le_bytes([b[0], b[1], b[2], b[3]])); }
            None => break,
        }
    }
    out
}

/// `<game_dir>/default.pcs` — the game's singleton executed-PC coverage sidecar.
pub fn pcs_path(game_dir: &Path) -> PathBuf {
    game_dir.join("default.pcs")
}

/// Read the per-game executed-PC sidecar (empty set if absent or unreadable).
pub fn read_pcs(game_dir: &Path) -> HashSet<u32> {
    match std::fs::read(pcs_path(game_dir)) {
        Ok(bytes) => decode(&bytes),
        Err(_) => HashSet::new(),
    }
}

/// Write the per-game executed-PC sidecar (creating `game_dir` if needed).
///
/// Atomic (SQ-0644): temp-then-rename, so an interrupted write cannot leave a
/// truncated `default.pcs` behind (the decoder would silently read it as partial
/// coverage, quietly losing lines the inspector had already marked executed).
pub fn write_pcs(game_dir: &Path, set: &HashSet<u32>) -> std::io::Result<()> {
    crate::storage::atomic_write(&pcs_path(game_dir), &encode(set))
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;

    fn sample() -> HashSet<u32> {
        HashSet::from([0x0400u32, 0x0500, 0xDEADBEEF, 0])
    }

    #[test]
    fn codec_round_trips() {
        assert_eq!(decode(&encode(&sample())), sample());
    }

    #[test]
    fn empty_round_trips() {
        let empty = HashSet::new();
        assert_eq!(decode(&encode(&empty)), empty);
    }

    // ── format freeze (docs/release/save-format-policy.md) ──
    #[test]
    fn version_constant_is_frozen() {
        assert_eq!(VERSION, 1, "ZPCS version changed — see docs/release/save-format-policy.md");
    }

    #[test]
    fn decode_rejects_bumped_version() {
        // A future (v2) ZPCS file is rejected by today's reader (empty set),
        // never mis-parsed as v1.
        let mut b = encode(&sample());
        b[4] = 0x02; // bump the version byte
        assert!(decode(&b).is_empty(), "a bumped-version ZPCS must not decode as v1");
    }

    #[test]
    fn decode_tolerates_foreign_or_truncated_bytes() {
        assert!(decode(b"\xff\xff\xffnonsense").is_empty()); // wrong magic
        assert!(decode(&[]).is_empty()); // empty
        // Valid header claiming 3 PCs but only 1.5 present → whatever parsed.
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.push(VERSION);
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&0x1234u32.to_le_bytes());
        b.extend_from_slice(&[0x00, 0x11]); // partial second PC
        assert_eq!(decode(&b), HashSet::from([0x1234u32]));
    }

    #[test]
    fn pcs_path_is_default_pcs_in_game_dir() {
        let dir = Path::new("/tmp/saves/Zork1.z5.save");
        let p = pcs_path(dir);
        assert_eq!(p, PathBuf::from("/tmp/saves/Zork1.z5.save/default.pcs"));
        assert_eq!(p.parent(), Some(dir), "stays in the game dir");
    }

    #[test]
    fn missing_file_reads_empty() {
        let dir = std::env::temp_dir().join(format!("lanthorn-pcs-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_pcs(&dir).is_empty(), "absent file → empty");
    }

    #[test]
    fn global_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("lanthorn-pcs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_pcs(&dir).is_empty(), "absent file → empty");
        write_pcs(&dir, &sample()).unwrap();
        assert_eq!(read_pcs(&dir), sample());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0644: the decoder is deliberately tolerant of truncation, so a half-written
    /// `default.pcs` reads back as PARTIAL coverage — silently losing lines the
    /// inspector had already marked executed, with nothing to say it happened. The
    /// write is temp-then-rename, so an interrupted one keeps the old set instead.
    #[test]
    fn an_interrupted_pcs_write_keeps_the_previous_set() {
        let dir = std::env::temp_dir().join(format!("lanthorn-pcs-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_pcs(&dir, &sample()).unwrap();

        if !crate::storage::deny_new_files_in(&dir) {
            let _ = std::fs::remove_dir_all(&dir);
            return; // platform can't enforce it (or we're root) — skip
        }
        let result = write_pcs(&dir, &HashSet::from([0x99u32]));
        crate::storage::allow_new_files_in(&dir);

        assert!(result.is_err(), "a write that cannot complete must fail, not half-happen");
        assert_eq!(read_pcs(&dir), sample(), "the previous coverage set is still readable");
        assert!(crate::storage::leftover_temps(&dir).is_empty(), "no temp left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
