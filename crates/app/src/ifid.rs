//! Story IFID resolution for the app.
//!
//! Prefers Inform's embedded `UUID://<id>//` marker — the Treaty of Babel string
//! that Inform writes into both Z-machine and Glulx story files, and every babel
//! tool reads. Falling back, a Z-machine story gets its header-derived
//! `ZCODE-<release>-<serial>-<checksum>` (via [`zvm::ifid`]) and a Glulx story a
//! stable content hash — so a Glulx game never masquerades as a `ZCODE-…` IFID
//! (SQ-0339).

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
