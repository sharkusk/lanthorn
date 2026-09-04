//! RoomId policy for name-only rooms (no backing Z-machine object).
//!
//! RoomIds with the high bit set are synthetic: derived from a room's displayed
//! name when it could not be resolved to a game object. The high bit guarantees
//! no collision with real object numbers (no IF game has >= 2^31 objects).

use mapper::graph::{MapGraph, RoomId};

/// Set on a RoomId to mark it a name-only (non-object) room.
pub const SYNTHETIC_ROOM_FLAG: RoomId = 0x8000_0000;

/// True when `id` denotes a name-only room (high bit set).
pub fn is_synthetic_room(id: RoomId) -> bool {
    id & SYNTHETIC_ROOM_FLAG != 0
}

/// The raw hex/decimal spelling of a room id, with no reference to a `MapGraph` and no notion of
/// a per-map ordinal.
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
///
/// SQ-1300: this is no longer "the one spelling a room id is ever shown in" — a synthetic room
/// now shows its small per-map ORDINAL to a player instead (see [`room_label_no`] /
/// [`room_label_full`]), because an opaque hash like `#8000ABCD` reads as noise where the map
/// otherwise shows small, meaningful numbers. This function is what those two fall back to when
/// a synthetic id has no ordinal to show (a destination the graph holds no room for), and it
/// remains the DOT/SVG exporters' node-id spelling — a raw hex/decimal id, not a display label,
/// is what belongs in those.
pub fn display_room_id(id: RoomId) -> String {
    if is_synthetic_room(id) {
        format!("#{id:08X}")
    } else {
        format!("#{id}")
    }
}

/// The short per-map label for `id`, the one shown on the map's boxes, in footnotes, exit lists
/// and manifests (SQ-1300): a real Z-machine object number stays plain decimal (`#136`, unchanged
/// from before this quest — it is already a real, stable identifier a player might reference).
/// A synthetic room (Glulx or name-only — see [`is_synthetic_room`]) shows its small per-map
/// ORDINAL instead of the opaque hex id underneath it — `#12` rather than `#8000ABCD` — because
/// the ordinal is what a player can actually hold in their head. Falls back to
/// [`display_room_id`]'s hex spelling only when `id` names a synthetic room the graph holds no
/// node for (a destination recorded — e.g. as a random-exit pool member — that has never
/// actually been visited, so no ordinal was ever minted for it).
pub fn room_label_no(graph: &MapGraph, id: RoomId) -> String {
    room_label_no_of(id, graph.room(id).map(|r| r.ordinal()))
}

/// [`room_label_no`], for a caller that already has the room's ordinal (or knows it has none) and
/// not a `&MapGraph` to look one up — the drawn map's `RenderRoom::ordinal` (SQ-1300), which
/// exists precisely so the box-drawing code never needs to reach back into the graph.
pub fn room_label_no_of(id: RoomId, ordinal: Option<u64>) -> String {
    if !is_synthetic_room(id) {
        return format!("#{id}");
    }
    match ordinal {
        Some(n) => format!("#{n}"),
        None => display_room_id(id),
    }
}

/// The "both" spelling (SQ-1300): a real Z-machine room still reads as `#136` alone, but a
/// synthetic room carries its ordinal AND the raw id it stands for — `#12 (8000ABCD)` — for the
/// two diagnostic surfaces (the room panel's Diagnostics body, `/export-map`'s ROOM line) where a
/// player reporting a map problem needs the same id `display_room_id` would have shown, without
/// losing the memorable ordinal that now appears everywhere else. Falls back to
/// [`display_room_id`] alone, same as [`room_label_no`], for a synthetic id the graph holds no
/// room for.
pub fn room_label_full(graph: &MapGraph, id: RoomId) -> String {
    if !is_synthetic_room(id) {
        return format!("#{id}");
    }
    match graph.room(id) {
        Some(room) => format!("#{} ({id:08X})", room.ordinal()),
        None => display_room_id(id),
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

    // ── SQ-1300: room_label_no / room_label_full ──────────────────────────────

    #[test]
    fn room_label_no_is_decimal_for_a_zmachine_room_regardless_of_the_graph() {
        let g = mapper::graph::MapGraph::new();
        assert_eq!(room_label_no(&g, 136), "#136", "unchanged from before this quest");
        assert_eq!(room_label_no_of(136, Some(7)), "#136", "an ordinal is never consulted for a real object");
    }

    #[test]
    fn room_label_no_is_the_ordinal_for_a_synthetic_room_in_first_seen_order() {
        let mut g = mapper::graph::MapGraph::new();
        let a = synthetic_room_id("Back Alley");
        let b = synthetic_room_id("Sigil Street");
        g.upsert_room(a, "Back Alley".into());
        g.upsert_room(b, "Sigil Street".into());
        assert_eq!(room_label_no(&g, a), "#1", "first room discovered");
        assert_eq!(room_label_no(&g, b), "#2", "second room discovered");
    }

    #[test]
    fn room_label_no_falls_back_to_hex_for_a_synthetic_id_not_in_the_graph() {
        let g = mapper::graph::MapGraph::new();
        let unvisited = synthetic_room_id("Roundabout");
        assert_eq!(
            room_label_no(&g, unvisited),
            display_room_id(unvisited),
            "no ordinal was ever minted, so the hex spelling is all there is"
        );
    }

    #[test]
    fn room_label_full_pairs_the_ordinal_with_the_raw_hex_id() {
        let mut g = mapper::graph::MapGraph::new();
        let id = 0x8000_ABCD;
        g.upsert_room(id, "Deep Street".into());
        assert_eq!(room_label_full(&g, id), "#1 (8000ABCD)");
    }

    #[test]
    fn room_label_full_is_decimal_alone_for_a_zmachine_room() {
        let g = mapper::graph::MapGraph::new();
        assert_eq!(room_label_full(&g, 136), "#136", "no parenthetical for a real object number");
    }

    #[test]
    fn room_label_no_survives_a_rekey() {
        let mut g = mapper::graph::MapGraph::new();
        let old = synthetic_room_id("Back Alley");
        g.upsert_room(old, "Back Alley".into());
        assert_eq!(room_label_no(&g, old), "#1");
        let real = glulx_room_id(0x21b0c);
        assert!(g.rekey_room(old, real));
        assert_eq!(room_label_no(&g, real), "#1", "the ordinal followed the room to its new id");
    }
}
