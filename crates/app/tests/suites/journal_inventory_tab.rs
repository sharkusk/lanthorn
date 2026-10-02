//! SQ-1630: the Journal Inventory tab's (formerly the inventory dock's) "Carrying"/"Elsewhere" split, end to end through a real
//! Z-machine session — `render::inventory_dock::build_inventory_dock_rows` fed the SAME two
//! inputs `main.rs` builds it from: `render::transcript::inventory_items_with_keys` (the live
//! carried list, with ids) and `mapper::graph::MapGraph` (the whole-game item registry built by
//! `app::session::apply_item_observations`, exactly as `item_tracking.rs`'s own cases drive it).
//!
//! `stories/zork1-r88-s840726.z3`: release 88 / serial 840726, the same specimen
//! `item_tracking.rs` uses — West of House's mailbox/leaflet for a direct sighting and a
//! container-nested one, and the Living Room's brass lantern for something the player actually
//! walks off with. Skips vacuously without `stories/` (gitignored), the CI-safe pattern
//! `room_description.rs` uses.

use crate::fixture_paths::fixture_path;

use app::engine::{Engine, Introspect};
use app::render::inventory_dock::build_inventory_dock_rows;
use app::render::transcript::inventory_items_with_keys;
use app::session::GameSession;

fn boot_zork1() -> Option<GameSession> {
    let bytes = std::fs::read(fixture_path("zork1-r88-s840726.z3")).ok()?;
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
        .expect("zork1 boots without a ZError");
    let _ = s.submit("");
    Some(s)
}

/// Take-and-drop the mailbox's leaflet back in West of House (never picked back up — an
/// "Elsewhere" item with a real Room sighting), leave the mailbox itself behind (a takeable-
/// looking but "securely anchored" object — `fixed_in_place`), and walk in through the kitchen
/// window to carry off the Living Room's brass lantern.
///
/// Turn-by-turn (verified against the real interpreter output the way `item_tracking.rs`'s own
/// header describes, `cargo run -p lanthorn-zvm-cli -- stories/zork1-r88-s840726.z3`):
/// 1. open mailbox   — the leaflet becomes visible (RoomNested)
/// 2. take leaflet   — carried
/// 3. take mailbox   — refused, "It is securely anchored." → Facet 3's fixed_in_place
/// 4. drop leaflet   — back in West of House, a real Room sighting (Elsewhere)
/// 5. north          — North of House, leaving the leaflet behind
/// 6. east           — Behind House
/// 7. open window    — the kitchen window
/// 8. enter window   — into the Kitchen
/// 9. west           — into the Living Room, where the brass lantern sits
/// 10. take lamp     — carried (the parser accepts "lamp" as a synonym of "lantern")
#[test]
fn carrying_and_elsewhere_sections_reflect_a_real_walkthrough() {
    let Some(mut s) = boot_zork1() else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let mut mapper = mapper::mapper::Mapper::default();
    let mut turn = 0u32;
    let mut drive = |s: &mut GameSession, mapper: &mut mapper::mapper::Mapper, cmd: &str| {
        let r = s.submit(cmd);
        turn += 1;
        app::session::apply_turn(mapper, cmd, &r, &mut Default::default());
        app::session::apply_item_observations(mapper, turn, &r);
        // Facet 3 (`host::turn`'s own wiring, reproduced here since this test drives the
        // session directly rather than through the host loop): a structurally unambiguous
        // take-refusal marks the item fixed-in-place.
        let vocab = s.story_vocabulary();
        if let Some(key) = app::session::classify_take_attempt(cmd, &r.items, vocab.as_ref()) {
            mapper.graph.note_item_fixed_in_place(key);
        }
        r
    };

    for cmd in [
        "open mailbox", "take leaflet", "take mailbox", "drop leaflet", "north", "east",
        "open window", "enter window", "west", "take lamp",
    ] {
        drive(&mut s, &mut mapper, cmd);
    }

    let carried = inventory_items_with_keys(None, &[], s.introspect(), None);
    // The lamp really is carried, through the very same call `main.rs` makes.
    assert!(
        carried.iter().any(|(_, name)| name.to_lowercase().contains("lantern")),
        "the live carried list has the lamp: {carried:?}"
    );

    let rows = build_inventory_dock_rows(&carried, &mapper.graph, None);
    let texts: Vec<String> = rows
        .iter()
        .map(|r| match r {
            app::render::inventory_dock::ItemDockRow::Header(t) => t.clone(),
            app::render::inventory_dock::ItemDockRow::Carried { text, .. } => text.clone(),
            app::render::inventory_dock::ItemDockRow::Elsewhere { text, .. } => text.clone(),
        })
        .collect();
    let joined = texts.join("\n").to_lowercase();

    assert!(joined.contains("carrying:"), "a Carrying header, since the lamp is held: {texts:?}");
    assert!(
        joined.contains("lantern") && joined.contains("found"),
        "the carried lantern cross-references its origin room/turn: {texts:?}"
    );
    assert!(joined.contains("elsewhere:"), "an Elsewhere header, since the leaflet/mailbox are tracked but not carried: {texts:?}");
    assert!(
        joined.contains("mailbox") && joined.contains("fixed in place"),
        "the mailbox reads fixed in place, per Facet 3's structural take-refusal signal: {texts:?}"
    );
    assert!(
        joined.contains("leaflet") && joined.contains("last seen"),
        "the dropped leaflet shows up Elsewhere with a real last-seen room/turn: {texts:?}"
    );
    // Nothing carried shows up twice in Elsewhere.
    let lantern_elsewhere = rows.iter().any(|r| matches!(
        r,
        app::render::inventory_dock::ItemDockRow::Elsewhere { text, .. } if text.to_lowercase().contains("lantern")
    ));
    assert!(!lantern_elsewhere, "the carried lantern must not also appear in Elsewhere: {texts:?}");
}
