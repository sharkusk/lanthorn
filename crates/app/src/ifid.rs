//! Story IFID resolution for the app.
//!
//! Prefers Inform's embedded `UUID://<id>//` marker — the Treaty of Babel string
//! that Inform writes into both Z-machine and Glulx story files, and every babel
//! tool reads. Falling back, a Z-machine story gets its header-derived
//! `ZCODE-<release>-<serial>-<checksum>` (via [`zvm::ifid`]), a Glulx story a
//! stable content hash — so a Glulx game never masquerades as a `ZCODE-…` IFID
//! (SQ-0339) — and, since SQ-1414, a Scott Adams database the same shape of
//! stable content hash, for the same reason and one that bit harder.

/// The story's IFID. See the module docs for the resolution order.
pub fn compute_ifid(story: &[u8]) -> String {
    if let Some(id) = embedded_uuid(story) {
        return id;
    }
    if is_glulx(story) {
        // A Glulx image with no embedded IFID (e.g. an older Inform 6 game like
        // Narcolepsy): a stable content hash, labelled GLULX- so it never reads
        // as a Z-machine IFID. Not the Treaty's MD5, but unique and stable, which
        // is all the app needs it for (per-game styles/config keying).
        return format!("GLULX-{:016X}", fnv1a64(story));
    }
    // A Scott Adams database is not remotely shaped like a Z-machine header,
    // but `zvm::ifid::compute_ifid` does not check that before reading it —
    // and the three binary dialects (SQ-1414) can make that read the SAME
    // bytes twice over. A Commodore 64 *Mysterious Adventures* program file
    // shares its first ~1KB of driver code, byte-for-byte, across all eleven
    // releases — including the offsets `zvm::ifid::compute_ifid` reads for
    // "release", "serial" and "checksum" — so every program on one disk
    // fell through to the identical fabricated `ZCODE-…` id, and
    // `picker::dedupe_within_a_volume`'s ifid-keyed fold (built for two
    // copies of one Z-code build on a hybrid disc) collapsed six distinct
    // games on `MYSTADV1.D64` into one row. A stable content hash, exactly
    // the Glulx branch's own shape and for the same reason, never collides
    // between two different games again.
    if scott::looks_like_scott_bytes(story) {
        return format!("SCOTT-{:016X}", fnv1a64(story));
    }
    zvm::ifid::compute_ifid(story)
}

/// A Glulx image begins with the ASCII magic `Glul` (0x476C_756C).
fn is_glulx(story: &[u8]) -> bool {
    story.starts_with(b"Glul")
}

/// Extract Inform's embedded `UUID://<id>//` IFID, if present. `<id>` is the run
/// of IFID characters (letters, digits, dashes) between the markers.
fn embedded_uuid(story: &[u8]) -> Option<String> {
    const OPEN: &[u8] = b"UUID://";
    let start = story.windows(OPEN.len()).position(|w| w == OPEN)? + OPEN.len();
    let rest = &story[start..];
    let end = rest.windows(2).position(|w| w == b"//")?;
    let id = &rest[..end];
    if id.is_empty() || !id.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    Some(String::from_utf8_lossy(id).into_owned())
}

/// FNV-1a 64-bit hash — deterministic and dependency-free (zvm/gvm stay
/// zero-dep; a crypto MD5 isn't worth a crate for this internal id).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;

    /// A story embedding `UUID://<id>//` (Inform's marker) returns that id
    /// verbatim, whatever the underlying format.
    #[test]
    fn embedded_uuid_wins_for_both_formats() {
        let uuid = "AC0DAF65-F40F-4A41-A4E4-50414F836E14";
        // Glulx image (magic 'Glul') with the marker buried in its ROM.
        let mut glulx = b"Glul\x00\x03\x00\x00".to_vec();
        glulx.extend_from_slice(format!("...UUID://{uuid}//...").as_bytes());
        assert_eq!(compute_ifid(&glulx), uuid);
        // A ZCODE marker (Inform falls back to the ZCODE IFID as the UUID content).
        let mut z = vec![5u8; 0x40];
        z.extend_from_slice(b"UUID://ZCODE-1-070917-994E//");
        assert_eq!(compute_ifid(&z), "ZCODE-1-070917-994E");
    }

    /// A Glulx image with NO embedded IFID gets a stable `GLULX-<hash>`, never a
    /// `ZCODE-…` (the SQ-0339 bug).
    #[test]
    fn glulx_without_uuid_gets_stable_glulx_hash() {
        let mut glulx = b"Glul\x00\x03\x00\x00".to_vec();
        glulx.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let id = compute_ifid(&glulx);
        assert!(id.starts_with("GLULX-"), "labelled Glulx, not ZCODE: {id}");
        assert!(!id.starts_with("ZCODE"), "must not masquerade as Z-machine");
        assert_eq!(compute_ifid(&glulx), id, "deterministic");
        // A one-byte change gives a different id.
        let mut other = glulx.clone();
        *other.last_mut().unwrap() = 9;
        assert_ne!(compute_ifid(&other), id);
    }

    /// A Scott Adams database with no embedded IFID gets a stable
    /// `SCOTT-<hash>`, never the naive `ZCODE-…` reading of whatever happens
    /// to sit at a Z-machine header's fixed byte offsets — the SQ-1414
    /// collision, where every program on a Commodore 64 *Mysterious
    /// Adventures* disk shares identical bytes at exactly those offsets (its
    /// driver code) and fell through to one fabricated id for all eleven
    /// games.
    ///
    /// FALSIFICATION: drop the Scott branch and the second assertion fails —
    /// two DIFFERENT games sharing an identical prefix at those offsets
    /// collide on one `ZCODE-…` id, exactly as `dedupe_within_a_volume`
    /// then folded six rows off `MYSTADV1.D64` into one.
    #[test]
    fn scott_without_uuid_gets_stable_scott_hash() {
        let dat = include_bytes!("../../scott/tests/tiny_cave.dat");
        assert!(scott::looks_like_scott_bytes(dat), "the fixture must sniff as Scott");
        let id = compute_ifid(dat);
        assert!(id.starts_with("SCOTT-"), "labelled Scott, not ZCODE: {id}");
        assert!(!id.starts_with("ZCODE"), "must not masquerade as Z-machine");
        assert_eq!(compute_ifid(dat), id, "deterministic");

        // Two different games sharing an identical prefix at exactly the
        // offsets `zvm::ifid::compute_ifid` reads must not collide.
        let mut same_prefix_different_tail = dat.to_vec();
        same_prefix_different_tail.extend_from_slice(b"\n9999 \"an extra room nobody else has\"\n");
        assert!(scott::looks_like_scott_bytes(&same_prefix_different_tail));
        assert_ne!(compute_ifid(&same_prefix_different_tail), id);
    }

    /// A Z-machine story with no marker keeps its header-derived ZCODE IFID.
    #[test]
    fn zmachine_without_uuid_uses_zcode_header() {
        let mut z = vec![0u8; 0x40];
        z[0] = 5; // version 5 (not the Glulx magic)
        z[0x02] = 0;
        z[0x03] = 42; // release 42
        z[0x12..0x18].copy_from_slice(b"871124");
        z[0x1C] = 0xAB;
        z[0x1D] = 0xCD;
        assert_eq!(compute_ifid(&z), "ZCODE-42-871124-ABCD");
    }
}
