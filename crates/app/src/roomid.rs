//! RoomId policy for name-only rooms (no backing Z-machine object).
//!
//! RoomIds with the high bit set are synthetic: derived from a room's displayed
//! name when it could not be resolved to a game object. The high bit guarantees
//! no collision with real object numbers (no IF game has >= 32768 objects).

/// Set on a RoomId to mark it a name-only (non-object) room.
pub const SYNTHETIC_ROOM_FLAG: u16 = 0x8000;

/// True when `id` denotes a name-only room (high bit set).
pub fn is_synthetic_room(id: u16) -> bool {
    id & SYNTHETIC_ROOM_FLAG != 0
}

/// Deterministic, save/reload-stable RoomId for a name-only room. Normalizes the
/// name (trim, collapse whitespace, lowercase) then FNV-1a hashes it into the
/// low 15 bits, with the high bit set.
pub fn synthetic_room_id(name: &str) -> u16 {
    let norm: String = name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let mut h: u32 = 0x811c_9dc5;
    for b in norm.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    SYNTHETIC_ROOM_FLAG | (h as u16 & 0x7FFF)
}

/// RoomId for a Glulx room identified by its OBJECT ADDRESS rather than its name
/// (SQ-0526).
///
/// Glulx has no object tree, so rooms were identified by hashing the printed room
/// name — which makes every same-named room one room, and collapses a maze into a
/// single node. Once the `location` global is located
/// ([`crate::glulx_roomlock`]), the room's own address is available and is a true
/// identity. Hashed into the same 15-bit space with the synthetic flag set,
/// because these are not Z-machine object numbers either.
///
/// Two rooms can still collide, as they can under [`synthetic_room_id`] — but
/// that is a remote accident here, where under the name hash it was a certainty
/// for every repeated name.
pub fn glulx_room_id(addr: u32) -> u16 {
    let mut h: u32 = 0x811c_9dc5;
    for b in addr.to_be_bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    SYNTHETIC_ROOM_FLAG | (h as u16 & 0x7FFF)
}

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;

    /// The whole point: addresses that differ give ids that differ, where the name
    /// hash gave one id for every room sharing a name.
    #[test]
    fn distinct_addresses_give_distinct_ids() {
        // Adventure's three maze rooms, all printing the heading "Maze".
        let ids: Vec<u16> = [0x21b0c, 0x21b2c, 0x21b4c].iter().map(|&a| glulx_room_id(a)).collect();
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
}
