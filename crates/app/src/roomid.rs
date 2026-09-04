//! RoomId policy for name-only rooms (no backing Z-machine object).
//!
//! RoomIds with the high bit set are synthetic: derived from a room's displayed
//! name when it could not be resolved to a game object. The high bit guarantees
//! no collision with real object numbers (no IF game has >= 2^31 objects).

use mapper::graph::RoomId;

/// Set on a RoomId to mark it a name-only (non-object) room.
pub const SYNTHETIC_ROOM_FLAG: RoomId = 0x8000_0000;

/// True when `id` denotes a name-only room (high bit set).
pub fn is_synthetic_room(id: RoomId) -> bool {
    id & SYNTHETIC_ROOM_FLAG != 0
}

/// The one spelling a room id is ever shown in, everywhere it reaches a player
/// or a file written for one — the map's boxes, the room panel, `/export-map`'s
/// dump, the DOT/SVG exporters (SQ-1297).
///
/// A real Z-machine object number stays plain decimal (`#123`): small, and
/// nothing about it is a hash. A synthetic id (the high bit set — see
/// [`is_synthetic_room`]) prints as `#` plus its 8 uppercase hex digits
/// (`#8000ABCD`) instead of decimal: decimal would run to 10 digits since every
/// synthetic id is now >= 2^31, and `#` + 9 or 10 digits no longer fits the
/// 9-character interior Boxes-zoom centres a room's id in — the truncation that
/// motivated this function, since two different large ids can share a leading
/// 9 digits and would otherwise draw identically. Hex is also the more natural
/// reading for a Glulx id: `glulx_room_id` folds an object ADDRESS into it.
pub fn display_room_id(id: RoomId) -> String {
    if is_synthetic_room(id) {
        format!("#{id:08X}")
    } else {
        format!("#{id}")
    }
}

/// Deterministic, save/reload-stable RoomId for a name-only room. Normalizes the
/// name (trim, collapse whitespace, lowercase) then FNV-1a hashes it into the
/// low 31 bits, with the high bit set.
pub fn synthetic_room_id(name: &str) -> RoomId {
    let norm: String = name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let mut h: u32 = 0x811c_9dc5;
    for b in norm.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    SYNTHETIC_ROOM_FLAG | (h & 0x7FFF_FFFF)
}

/// RoomId for a Glulx room identified by its OBJECT ADDRESS rather than its name
/// (SQ-0526).
///
/// Glulx has no object tree, so rooms were identified by hashing the printed room
/// name — which makes every same-named room one room, and collapses a maze into a
/// single node. Once the `location` global is located
/// ([`crate::glulx_roomlock`]), the room's own address is available and is a true
/// identity. Hashed into the same 31-bit space with the synthetic flag set,
/// because these are not Z-machine object numbers either.
///
/// Two rooms can still collide, as they can under [`synthetic_room_id`] — but
/// that is a remote accident here, where under the name hash it was a certainty
/// for every repeated name.
pub fn glulx_room_id(addr: u32) -> RoomId {
    let mut h: u32 = 0x811c_9dc5;
    for b in addr.to_be_bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    SYNTHETIC_ROOM_FLAG | (h & 0x7FFF_FFFF)
}

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;

    /// The whole point: addresses that differ give ids that differ, where the name
    /// hash gave one id for every room sharing a name.
    #[test]
    fn distinct_addresses_give_distinct_ids() {
        // Adventure's three maze rooms, all printing the heading "Maze".
        let ids: Vec<RoomId> = [0x21b0c, 0x21b2c, 0x21b4c].iter().map(|&a| glulx_room_id(a)).collect();
        assert_eq!(
            ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
            3,
            "three maze rooms must be three ids, got {ids:?}"
        );
        assert_eq!(
            synthetic_room_id("Maze"),
            synthetic_room_id("Maze"),
            "whereas the name hash gives them all the same id — the bug"
        );
    }

    /// SQ-1297: with a 15-bit synthetic space, two differently-named Counterfeit
    /// Monkey rooms hashed to the same id (43044) and were merged into one room on
    /// the map. Widening to 31 usable bits must separate them.
    #[test]
    fn sq1297_observed_collision_your_bunk_vs_language_studies() {
        assert_ne!(
            synthetic_room_id("Your Bunk"),
            synthetic_room_id("Language Studies Seminar Room"),
            "these two real Counterfeit Monkey rooms collided at 43044 under the old 15-bit fold"
        );
    }

    /// SQ-1297: the other observed collision was a Glulx address-hashed room
    /// (Private Beach) landing on the same id as a name-hashed one (Roundabout,
    /// 49352). We don't know Private Beach's real address, so sweep a handful of
    /// plausible ones and confirm none lands on Roundabout's new (wider) id.
    #[test]
    fn sq1297_glulx_and_synthetic_ids_stay_apart_over_a_plausible_range() {
        let roundabout = synthetic_room_id("Roundabout");
        for addr in (0x1000u32..0x20000).step_by(0x101) {
            assert_ne!(
                glulx_room_id(addr),
                roundabout,
                "addr {addr:#x} collided with \"Roundabout\" in the widened space"
            );
        }
    }

    #[test]
    fn glulx_ids_are_marked_synthetic_and_stable() {
        let id = glulx_room_id(0x21b0c);
        assert!(is_synthetic_room(id), "not a Z-machine object number");
        assert_eq!(id, glulx_room_id(0x21b0c), "stable across calls, so saves reload");
    }

    #[test]
    fn synthetic_id_high_bit_set_and_deterministic() {
        let a = synthetic_room_id("Bedroom");
        assert_eq!(a & SYNTHETIC_ROOM_FLAG, SYNTHETIC_ROOM_FLAG, "high bit set");
        assert_eq!(a, synthetic_room_id("Bedroom"), "deterministic");
        assert!(is_synthetic_room(a));
        assert!(!is_synthetic_room(150)); // a real object number
    }

    #[test]
    fn synthetic_id_normalizes_name() {
        assert_eq!(synthetic_room_id("Bedroom"), synthetic_room_id("  bedroom  "));
        assert_eq!(synthetic_room_id("Foo Bar"), synthetic_room_id("foo   bar"));
    }

    #[test]
    fn synthetic_id_differs_for_distinct_names() {
        assert_ne!(synthetic_room_id("Bedroom"), synthetic_room_id("Kitchen"));
    }

    /// SQ-1297: a real object number is small enough that decimal never overflows
    /// the 9-char box interior, so it stays decimal.
    #[test]
    fn display_room_id_real_object_stays_decimal() {
        assert_eq!(display_room_id(57), "#57");
        assert_eq!(display_room_id(65535), "#65535");
    }

    /// SQ-1297: a synthetic id is always >= 2^31 (10 decimal digits), which
    /// overflows the 9-char box interior; hex holds it in exactly `#` + 8 digits.
    #[test]
    fn display_room_id_synthetic_is_9_char_hex() {
        let s = display_room_id(synthetic_room_id("Your Bunk"));
        assert_eq!(s.len(), 9, "# plus 8 hex digits: {s}");
        assert!(s.starts_with('#'));
        assert_eq!(s, s.to_uppercase(), "hex digits are uppercase");
    }

    #[test]
    fn display_room_id_is_exact_hex_of_the_id() {
        assert_eq!(display_room_id(0x8000_ABCD), "#8000ABCD");
    }
}
