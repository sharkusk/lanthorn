//! SQ-0678: the *here* column against real Infocom stories.
//!
//! `Introspect::room_objects` used to list the room object's direct children and
//! nothing else, which is wrong in two directions at once:
//!
//! - things sitting on a supporter or inside an open container are children of
//!   *that furniture*, not of the room, so Zork I's kitchen showed a table and
//!   neither the sack nor the bottle on it;
//! - shared scenery (the window at Behind House, the chimney, the forest) is
//!   never a child of any room at all — ZIL parks it in one bucket object and
//!   each room names what it can see in a property.
//!
//! Fixing the first without care is how a play-aid starts cheating: descend one
//! level unconditionally and the here column announces the lunch and the clove
//! of garlic inside a sack the player has never opened. Every assertion below
//! that starts with `!` is the real subject of this file — the positive ones
//! only prove the walk runs at all.
//!
//! Commercial stories are gitignored, so every test skips vacuously when its
//! fixture is absent.


use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;

use crate::fixture_paths::fixture_path;


fn boot(file: &str) -> Option<GameSession> {
    let path = fixture_path(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
            .expect("story should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// The objects the command band's *here* column is built from: the current
/// room's visible objects with the player excluded, named the way the game
/// prints them. (What the column then OFFERS is the word the parser accepts for
/// each — `crate::scope_words` pins that; every case here is about WHICH
/// objects the walk reaches, which is a question about the tree.)
fn here(session: &GameSession) -> Vec<String> {
    let player = session.introspect().and_then(|i| i.player_object());
    let loc = session.current_location().expect("a located room");
    session
        .introspect()
        .unwrap()
        .room_objects_excluding(loc.number, player)
        .iter()
        .filter_map(|o| o.display_name())
        .collect()
}

fn has(list: &[String], needle: &str) -> bool {
    list.iter().any(|n| n.to_lowercase().contains(needle))
}

/// Zork I r52. The conventions this story was compiled with, recovered from its
/// own object table — pinned because every behaviour below rests on them and a
/// silent `None` here would turn the assertions into vacuous ones (the walk
/// would fall back to direct children and the *negative* assertions would all
/// still pass).
#[test]
fn zork1_object_table_conventions_are_recovered() {
    let Some(session) = boot("zork1-invclues-r52-s871125.z5") else { return };
    let m = session.world_model();
    assert_eq!(m.container_attr, Some(34), "Zork I marks containers with attribute 34");
    assert_eq!(
        m.open_attr,
        Some(28),
        "…and openness with 28 — measured by diffing every attribute across an `open mailbox` turn"
    );
    assert_eq!(m.globals_prop, Some(37), "shared scenery is listed in property 37 (ZIL GLOBAL)");
    assert_eq!(m.globals_holder, Some(50), "…naming objects parked in bucket object #50");
}

/// The model is a property of the STORY, not of when it was first asked for.
///
/// It is derived lazily, and the inference reads attribute populations — which
/// drift the moment anything is opened or taken. Deriving it from live memory
/// made Zork I's openness bit stop being unique after four moves, so the kitchen
/// silently lost the sack and bottle depending on the route taken to get there.
/// Deriving it from the boot image fixes that; this pins it.
#[test]
fn zork1_conventions_do_not_depend_on_when_they_were_first_asked_for() {
    let Some(at_boot) = boot("zork1-invclues-r52-s871125.z5") else { return };
    let expected = at_boot.world_model().clone();

    let mut later = boot("zork1-invclues-r52-s871125.z5").unwrap();
    // The exact route that broke it: opening the window flips an attribute that
    // the openness inference counts.
    for cmd in ["north", "east", "open window", "enter window"] {
        later.submit(cmd);
    }
    assert_eq!(
        *later.world_model(),
        expected,
        "four moves of play must not change what the story's attributes mean"
    );
}

/// West of House, the very first room: both halves of the open/closed rule.
///
/// The mailbox is shut, and the leaflet inside it is something the player has
/// not seen. It appears the moment the game opens the mailbox and disappears
/// again when it is shut — the column reads the live attribute every refresh,
/// it does not snapshot a visible set at boot.
#[test]
fn zork1_a_shut_mailbox_hides_its_leaflet_and_opening_it_reveals_it() {
    let Some(mut session) = boot("zork1-invclues-r52-s871125.z5") else { return };
    assert_eq!(session.current_location().unwrap().name, "West of House");

    let shut = here(&session);
    assert!(has(&shut, "mailbox"), "the mailbox is on the lawn: {shut:?}");
    assert!(
        !has(&shut, "leaflet"),
        "the leaflet is inside a SHUT mailbox — listing it is cheating: {shut:?}"
    );

    session.submit("open mailbox");
    let open = here(&session);
    assert!(has(&open, "leaflet"), "an opened mailbox shows what is in it: {open:?}");

    session.submit("close mailbox");
    let reshut = here(&session);
    assert!(
        !has(&reshut, "leaflet"),
        "closing it hides the leaflet again — the column is the live tree, not a log: {reshut:?}"
    );
}

/// Behind House: the window is a local-global. It is a child of the scenery
/// bucket, not of the room, so no child walk of any depth can ever reach it —
/// it comes from the room's own GLOBAL property.
#[test]
fn zork1_behind_house_lists_the_window_which_is_no_rooms_child() {
    let Some(mut session) = boot("zork1-invclues-r52-s871125.z5") else { return };
    session.submit("north");
    session.submit("east");
    let loc = session.current_location().unwrap();
    assert_eq!(loc.name, "Behind House");

    let list = here(&session);
    assert!(has(&list, "window"), "the window is what you use to get in: {list:?}");
    assert!(
        zvm::objects::get_parent(&session.machine.mem, 253) as mapper::graph::RoomId != loc.number,
        "…and it is emphatically not a child of the room, so this cannot be a child walk"
    );
}

/// The kitchen: two levels of nesting on one shelf, and the leak sitting right
/// next to it. The sack and the bottle rest on the table (children of the
/// *table*); the lunch and the garlic are inside the sack, which is shut.
#[test]
fn zork1_kitchen_lists_what_is_on_the_table_but_not_what_is_in_the_shut_sack() {
    let Some(mut session) = boot("zork1-invclues-r52-s871125.z5") else { return };
    for cmd in ["north", "east", "open window", "enter window"] {
        session.submit(cmd);
    }
    assert_eq!(session.current_location().unwrap().name, "Kitchen");

    let list = here(&session);
    for want in ["table", "sack", "bottle", "window"] {
        assert!(has(&list, want), "the kitchen shows the {want}: {list:?}");
    }
    assert!(!has(&list, "lunch"), "the lunch is inside the SHUT sack: {list:?}");
    assert!(!has(&list, "garlic"), "the garlic is inside the SHUT sack: {list:?}");

    // The sack's children really are there to be leaked — this is not a test
    // that passes because the tree is empty.
    let sack_children = session.introspect().unwrap().children_of(103);
    assert!(!sack_children.is_empty(), "the sack #103 does hold things");

    session.submit("open sack");
    let opened = here(&session);
    assert!(has(&opened, "lunch"), "opening the sack may reveal its contents: {opened:?}");
    assert!(has(&opened, "garlic"), "opening the sack may reveal its contents: {opened:?}");
    assert!(has(&opened, "table"), "…without losing anything that was already there: {opened:?}");
}

/// The player is a holder too, and their pockets are the *carried* column. The
/// nested walk must not turn the inventory into scenery.
#[test]
fn zork1_carried_items_never_leak_into_the_here_column() {
    let Some(mut session) = boot("zork1-invclues-r52-s871125.z5") else { return };
    session.submit("open mailbox");
    session.submit("take leaflet");
    let list = here(&session);
    assert!(
        !has(&list, "leaflet"),
        "a taken leaflet is CARRIED, not here — the player's subtree is excluded: {list:?}"
    );
    assert!(!has(&list, "cretin"), "…and the avatar itself stays out (SQ-0667): {list:?}");
}

/// Mini-Zork r34 uses different numbers for all of it (container 9, openness
/// 10, scenery property 12) — proof the recovery is per-story and nothing is
/// hard-coded to Zork I. Its here column is strictly improved: same mailbox,
/// plus the scenery it always could have shown, and still no leaflet until the
/// mailbox is opened.
#[test]
fn minizork_gets_the_same_treatment_from_entirely_different_numbers() {
    let Some(mut session) = boot("minizork-r34-s871124.z3") else { return };
    let m = session.world_model();
    assert_eq!((m.container_attr, m.open_attr), (Some(9), Some(10)));
    assert_eq!(m.globals_prop, Some(12));

    let shut = here(&session);
    assert!(has(&shut, "mailbox"), "{shut:?}");
    assert!(has(&shut, "white house"), "shared scenery the child walk never reached: {shut:?}");
    assert!(!has(&shut, "leaflet"), "shut mailbox keeps its leaflet: {shut:?}");

    session.submit("open mailbox");
    assert!(has(&here(&session), "leaflet"), "{:?}", here(&session));
}

/// The fail-toward-less contract on a real story. Planetfall's table gives no
/// unambiguous openness bit, so nesting is off there — the column falls back to
/// direct children plus shared scenery, which is what lanthorn listed before
/// this work. A story we cannot read is a story we do not guess about.
#[test]
fn planetfall_declines_to_guess_at_openness_and_nests_nothing() {
    let Some(session) = boot("planetfall-invclues-r10-s880531.z5") else { return };
    let m = session.world_model();
    assert_eq!(m.open_attr, None, "ambiguous openness candidates must resolve to None");
    assert_eq!(m.globals_prop, Some(41), "…while the scenery walk still works");

    // With no openness bit, nothing in the room can nest, whatever it holds.
    let loc = session.current_location().unwrap();
    let mem = &session.machine.mem;
    let model = session.world_model();
    for &obj in &model.visible_room_objects(mem, loc.number.try_into().unwrap(), 0) {
        assert!(
            !model.shows_contents(mem, obj),
            "no object may be treated as open when the bit is unknown (#{obj})"
        );
    }
}
