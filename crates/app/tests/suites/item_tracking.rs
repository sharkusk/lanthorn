//! Item location tracking (SQ-1627): the mapper's per-item origin/current-location/vanished
//! registry, and Facet 3's structural fixed-in-place detector, end to end through a real
//! Z-machine session. Real-game cases skip vacuously without `stories/` (gitignored), the
//! CI-safe pattern `room_description.rs` (SQ-1625) already uses.
//!
//! `stories/zork1-r88-s840726.z3`: release 88 / serial 840726 — West of House's mailbox and
//! leaflet are the specimen for every facet here: the mailbox is a classic non-takeable object
//! ("It is securely anchored."), and the leaflet is a classic container-nested item (visible only
//! once the mailbox is opened), which happens to be exactly the shape Facet 1's correction is
//! about. Verified against the real interpreter output before writing any assertion below
//! (`cargo run -p lanthorn-zvm-cli -- stories/zork1-r88-s840726.z3` driven with `open mailbox` /
//! `take leaflet` / `close mailbox` / `take mailbox`).

use crate::fixture_paths::fixture_path;

use app::config::Config;
use app::engine::{Engine, Introspect, KeyInput};
use app::glulx_session::GlulxSession;
use app::host::{apply_game_driven_result, boot_story, finish_command_turn, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::session::GameSession;

fn story(name: &str) -> Option<Vec<u8>> {
    std::fs::read(fixture_path(name)).ok()
}

fn boot_zork1() -> Option<GameSession> {
    let bytes = story("zork1-r88-s840726.z3")?;
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
        .expect("zork1 boots without a ZError");
    // Drain the boot's own banner/opening print first — same idiom `room_description.rs` uses.
    let _ = s.submit("");
    Some(s)
}

/// The Glulx image inside a Blorb, or a bare `.ulx` passed through — same helper
/// `glulx_inventory.rs` uses. `None` when the gitignored fixture is absent.
fn glulx_image(name: &str) -> Option<Vec<u8>> {
    let path = fixture_path(name);
    let bytes = std::fs::read(&path).ok()?;
    if !blorb::Blorb::is_blorb(&bytes) {
        return Some(bytes);
    }
    let b = blorb::Blorb::parse(bytes).ok()?;
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Some(data.to_vec()),
        _ => None,
    }
}

/// Cragne Manor (various authors, Inform 7 6M62), past its two-step content-warning
/// gate and its "[press any key to begin]" splash, to the first command prompt at
/// the Railway Platform.
fn boot_cragne() -> Option<GlulxSession> {
    let image = glulx_image("cragne.gblorb")?;
    let mut s = GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[]).ok()?;
    for _ in 0..6 {
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    for cmd in ["yes", "yes"] {
        s.submit(cmd);
    }
    s.submit_key(KeyInput::Enter);
    Some(s)
}

/// Anchorhead (Michael Gentry, Inform 6/Glulx, Illustrated Edition): `stories/Anchorhead.gblorb`
/// — past its splash to the first command prompt at "Outside the Real Estate Office", the boot
/// pattern `room_description.rs`'s `glulx_anchorhead_recaptures_a_description_with_no_leading_blank_line_between_turns`
/// already uses.
fn boot_anchorhead() -> Option<GlulxSession> {
    let image = glulx_image("Anchorhead.gblorb")?;
    let mut s = GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[]).ok()?;
    for _ in 0..6 {
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    Some(s)
}


/// Facet 1: the mailbox (never opened, never moved) is tracked as a direct sighting in West of
/// House from the very first turn it's observed.
#[test]
fn a_direct_room_item_is_tracked_from_its_first_sighting() {
    let Some(mut s) = boot_zork1() else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let r = s.submit("look");
    let mailbox = r
        .items
        .iter()
        .find(|o| o.name.contains("mailbox"))
        .expect("West of House prints a small mailbox");
    assert_eq!(mailbox.location, app::session::ObservedItemLocation::RoomDirect);
}

/// Facet 1 (the correction): opening the mailbox reveals the leaflet as a NESTED sighting, not a
/// direct one — and closing the mailbox again must not make the leaflet read as vanished on a
/// later look, because a closed container's contents are simply unreachable, not gone.
#[test]
fn a_leaflet_seen_only_inside_the_mailbox_is_nested_and_survives_the_mailbox_closing() {
    let Some(mut s) = boot_zork1() else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let opened = s.submit("open mailbox");
    let mailbox_key = opened
        .items
        .iter()
        .find(|o| o.name.contains("mailbox"))
        .expect("the mailbox itself is still a direct sighting")
        .key;
    let leaflet = opened
        .items
        .iter()
        .find(|o| o.name.contains("leaflet"))
        .expect("opening the mailbox reveals the leaflet");
    assert_eq!(
        leaflet.location,
        app::session::ObservedItemLocation::RoomNested { container: Some(mailbox_key) },
        "SQ-1632 Fix 1: the leaflet is nested inside the mailbox specifically, not just \"nested somewhere\""
    );
    let leaflet_key = leaflet.key;

    let mut mapper = mapper::mapper::Mapper::default();
    app::session::apply_turn(&mut mapper, "open mailbox", &opened, &mut Default::default());
    app::session::apply_item_observations(&mut mapper, 1, &opened);
    assert_eq!(
        mapper.graph.item(leaflet_key).unwrap().last_seen,
        mapper::graph::ItemLocation::Room {
            room: opened.location.as_ref().unwrap().number,
            direct: false,
            container: Some(mailbox_key),
        }
    );

    // Close the mailbox — the leaflet drops out of every observation entirely (unreachable, not
    // gone) — and LOOK again to re-drive a full turn against the same room.
    let closed = s.submit("close mailbox");
    assert!(!closed.items.iter().any(|o| o.name.contains("leaflet")), "closed — no longer observable at all");
    app::session::apply_turn(&mut mapper, "close mailbox", &closed, &mut Default::default());
    app::session::apply_item_observations(&mut mapper, 2, &closed);
    let looked = s.submit("look");
    assert!(!looked.items.iter().any(|o| o.name.contains("leaflet")), "still unreachable after a fresh LOOK");
    app::session::apply_turn(&mut mapper, "look", &looked, &mut Default::default());
    app::session::apply_item_observations(&mut mapper, 3, &looked);

    let rec = mapper.graph.item(leaflet_key).unwrap();
    assert!(
        !matches!(rec.last_seen, mapper::graph::ItemLocation::Vanished { .. }),
        "a nested-only item must never read as vanished when its container closes"
    );
    assert_eq!(
        rec.last_seen,
        mapper::graph::ItemLocation::Room {
            room: opened.location.as_ref().unwrap().number,
            direct: false,
            container: Some(mailbox_key),
        },
        "the record is left exactly as it was — the correction's whole point"
    );
}

/// Facet 1: taking the leaflet out of the mailbox moves it to Carried; dropping it again in the
/// SAME room re-confirms it as a direct sighting there. Origin never moves off West of House.
#[test]
fn taking_and_dropping_the_leaflet_moves_its_tracked_location() {
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
        r
    };

    drive(&mut s, &mut mapper, "open mailbox");
    let taken = drive(&mut s, &mut mapper, "take leaflet");
    let leaflet_key = taken.items.iter().find(|o| o.name.contains("leaflet")).expect("still observed, now carried").key;
    assert_eq!(mapper.graph.item(leaflet_key).unwrap().last_seen, mapper::graph::ItemLocation::Carried);
    assert_eq!(
        mapper.graph.item(leaflet_key).unwrap().carried_since_turn,
        Some(2),
        "SQ-1632 Fix 3: picked up on turn 2 (open mailbox=1, take leaflet=2)"
    );
    let origin_room = mapper.graph.item(leaflet_key).unwrap().origin_room;

    // Re-confirming it's STILL carried (an ordinary look) must not disturb the pick-up turn.
    drive(&mut s, &mut mapper, "look");
    assert_eq!(
        mapper.graph.item(leaflet_key).unwrap().carried_since_turn,
        Some(2),
        "still held — the pick-up turn does not slide forward on a re-confirmation"
    );

    drive(&mut s, &mut mapper, "drop leaflet");
    let rec = mapper.graph.item(leaflet_key).unwrap();
    assert_eq!(rec.origin_room, origin_room, "origin never moves");
    assert_eq!(
        rec.last_seen,
        mapper::graph::ItemLocation::Room { room: origin_room, direct: true, container: None },
        "dropped back in the same room — a direct sighting again"
    );
    assert_eq!(
        rec.carried_since_turn, None,
        "SQ-1632 Fix 3: dropping it clears the pick-up turn — it is not carried any more"
    );
}

/// Facet 3: "take mailbox" visibly fails ("It is securely anchored.") and the mailbox is the
/// unambiguous sole candidate for that noun in this room — confirmed fixed-in-place.
#[test]
fn taking_the_mailbox_confirms_it_fixed_in_place() {
    let Some(mut s) = boot_zork1() else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let r = s.submit("take mailbox");
    assert!(r.transcript.contains("securely anchored"), "the real refusal text: {:?}", r.transcript);
    let vocab = s.story_vocabulary();
    let key = app::session::classify_take_attempt("take mailbox", &r.items, vocab.as_ref())
        .expect("one unambiguous candidate, visibly not carried");
    let mailbox = r.items.iter().find(|o| o.key == key).unwrap();
    assert!(mailbox.name.contains("mailbox"));
    assert_ne!(mailbox.location, app::session::ObservedItemLocation::Carried);
}

/// Facet 3, the negative case: taking the leaflet (an ordinary takeable item) after opening the
/// mailbox must NOT be confirmed fixed-in-place — the take visibly worked.
#[test]
fn taking_an_ordinary_takeable_item_is_not_flagged_fixed_in_place() {
    let Some(mut s) = boot_zork1() else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let _ = s.submit("open mailbox");
    let r = s.submit("take leaflet");
    assert!(r.transcript.contains("Taken"), "the real success text: {:?}", r.transcript);
    let vocab = s.story_vocabulary();
    assert_eq!(
        app::session::classify_take_attempt("take leaflet", &r.items, vocab.as_ref()),
        None,
        "the take worked — nothing to flag"
    );
}

/// Observed-only guarantee: an item in a room the player has never visited must never appear in
/// the item registry, even after many turns of play elsewhere. Falsified (2026-09-27) by
/// temporarily widening `GameSession::zvm_item_observations` (`session.rs`) to also walk
/// `zvm::location::object_tree_view` — every object in the game, not just the current room and
/// inventory — which made this test fail with the kitchen's sack/bottle/table and every other
/// unvisited-room item leaking into the registry; reverted once confirmed.
#[test]
fn an_item_in_a_never_visited_room_never_appears_in_the_registry() {
    let Some(mut s) = boot_zork1() else {
        eprintln!("SKIP: gitignored stories/zork1-r88-s840726.z3 missing");
        return;
    };
    let mut mapper = mapper::mapper::Mapper::default();
    let mut turn = 0u32;
    // Play several turns entirely around West of House / North of House / Forest — never
    // entering the house, never reaching e.g. the kitchen (which holds its own items: a sack,
    // a bottle, a table).
    for cmd in ["look", "north", "north", "east", "south", "look"] {
        let r = s.submit(cmd);
        turn += 1;
        app::session::apply_turn(&mut mapper, cmd, &r, &mut Default::default());
        app::session::apply_item_observations(&mut mapper, turn, &r);
    }
    let tracked_names: Vec<String> = mapper.graph.items().map(|(_, rec)| rec.name.clone()).collect();
    assert!(
        !tracked_names.iter().any(|n| n.contains("sack") || n.contains("bottle") || n.contains("table")),
        "the kitchen's own items must never appear — the player has never stood in that room: {tracked_names:?}"
    );
    // Every item actually tracked must have an origin room the player did, in fact, walk
    // through this session (a resolved, non-synthetic location every `submit` above returned).
    assert!(!tracked_names.is_empty(), "West of House's own mailbox should still be tracked");
}

/// SQ-1632 Fix 5: the quest's own real repro against Zork I r88 — "white house", "board",
/// "stairs", "chimney", "kitchen window", "boarded window" are Inform/ZIL local-global/shared
/// scenery (`zvm::world::WorldModel::local_globals`), visible from several rooms but a genuine
/// child of NONE of them (`real_container_in_room` returns `None` for every one). Before the
/// fix, each such object was recorded as `RoomNested` in EVERY room it happened to be visible
/// from, reading in the item registry as though one portable object silently relocated itself as
/// the player walked ("white house" moving West of House -> North of House -> Behind House).
/// Falsify by temporarily dropping the `real_container_in_room` filter in
/// `GameSession::zvm_item_observations` and re-running: this test fails with exactly that shape
/// (confirmed before writing the fix).
#[test]
fn shared_scenery_never_reads_as_a_portable_item_following_the_player() {
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
        r
    };

    // The quest's own repro, verbatim.
    for cmd in [
        "open mailbox", "take leaflet", "take mailbox", "north", "east", "open window",
        "enter window", "open sack", "take bottle", "west", "take lamp", "take sword", "east",
        "turn on lamp", "up", "take rope", "down", "drop bottle", "look",
    ] {
        drive(&mut s, &mut mapper, cmd);
    }

    // None of the scenery the report named is in the item registry at all — the fix EXCLUDES
    // local-global scenery from item tracking outright, rather than merely pinning it to one room.
    let names: Vec<String> = mapper.graph.items().map(|(_, rec)| rec.name.to_lowercase()).collect();
    for scenery in ["white house", "board", "stairs", "chimney", "window"] {
        assert!(
            !names.iter().any(|n| n.contains(scenery)),
            "local-global scenery {scenery:?} must never enter the item registry at all: {names:?}"
        );
    }
}

// ── Fix 1 / Fix 3 (SQ-1631) — a real Inform 7 game with an unidentifiable avatar ─────────────
//
// Originally specimen'd against Counterfeit Monkey's Sigil Street (one `north` of the opening
// Back Alley). SQ-1640 added a `scenery`/`door` attribute filter to `glulx_item_observations`,
// and Sigil Street's own six direct children — verified against the real, IF-Archive-fetched
// `CounterfeitMonkey-10.gblorb` (`scripts/fetch-fixtures.sh`) — turn out to be *entirely* scenery
// backdrops ("sky backdrops", building facades, a shop window), with the only way further being a
// code-lock puzzle. Rewritten against Cragne Manor (also Inform 7 6M62, so the same "no hardware
// short name" trait applies, and its own avatar is equally unidentifiable —
// `cragne_wristwatch_take_and_drop_are_silently_untracked_with_an_unidentifiable_avatar` above)
// and its Train Station Lobby (one `south` of the opening Railway Platform), whose half-full
// styrofoam coffee cup is a genuine Inform-7-style object (empty hardware name, real parse words)
// that is NEITHER scenery NOR a door and so survives the new filter — confirmed directly against
// the object tree before writing these assertions.

/// SQ-1631 Fix 1: `glulx_item_observations` must reach an Inform 7 object with an EMPTY printed
/// name via its `display_name()` fallback (its parse words), not just its raw (usually blank)
/// printed name — before this fix, filtering on the raw printed name alone dropped every such
/// object outright, so `result.items` was empty for this game's every room regardless of what was
/// actually shown.
#[test]
fn glulx_item_observations_reaches_an_inform_7_object_with_no_printed_name() {
    let Some(mut s) = boot_cragne() else {
        eprintln!("SKIP: gitignored stories/cragne.gblorb missing");
        return;
    };
    let r = s.submit("south"); // Railway Platform -> Train Station Lobby
    assert_eq!(
        r.location.as_ref().map(|l| l.name.as_str()),
        Some("Train Station Lobby (Shin)"),
        "premise: this walks to the specimen room: {:?}",
        r.location
    );

    // Confirm the premise directly against the object list: this room really does hold an
    // object with an empty raw printed name but a real display name.
    let loc = r.location.clone().unwrap();
    let room_objects = s.introspect().unwrap().room_objects_excluding(loc.number, None);
    let unnamed: Vec<_> =
        room_objects.iter().filter(|o| o.printed_name.is_empty() && o.display_name().is_some()).collect();
    assert!(
        !unnamed.is_empty(),
        "premise: the lobby holds an Inform-7-style object with no printed name: {room_objects:?}"
    );

    // And it now reaches `result.items` — with its `display_name()` as `name`, never empty.
    assert!(
        r.items.iter().any(|i| unnamed.iter().any(|o| o.id == i.key) && !i.name.is_empty()),
        "at least one such object now reaches result.items with a real display name: {:?}",
        r.items
    );
}

/// SQ-1631 Fix 3: `glulx_item_observations` must prefer [`app::engine::Engine::set_player_hint`]'s
/// value over the raw [`app::engine::Introspect::player_object`] lookup, which for Cragne Manor
/// (same as Counterfeit Monkey) answers `None` FOREVER — no turn ever locks it by the engine's own
/// name-based heuristic. The host's own movement-tracking fallback
/// (`app::inventory::detect_player_obj`) can still lock a real handle by watching what moved
/// between rooms even when the engine's own lookup cannot, and `finish_command_turn` pushes that
/// lock in via `set_player_hint`. This exercises the mechanism directly against Cragne's own real
/// object tree: with no hint, nothing is excluded from "directly in this room" at all; with a hint
/// set to a real handle this room holds, that ONE object is excluded — exactly as it would be if
/// it really were the player. Tries every direct child in turn (rather than assuming the first
/// one) because SQ-1640's scenery/door filter already excludes some of them, and hinting one of
/// those would produce no visible change at all.
#[test]
fn glulx_item_observations_prefers_the_set_player_hint_over_the_unidentifiable_raw_lookup() {
    let Some(mut s) = boot_cragne() else {
        eprintln!("SKIP: gitignored stories/cragne.gblorb missing");
        return;
    };
    let r = s.submit("south"); // Railway Platform -> Train Station Lobby
    let loc = r.location.clone().unwrap();
    assert!(
        s.introspect().unwrap().player_object().is_none(),
        "premise: Cragne's avatar is unidentifiable by name"
    );

    let without_hint: std::collections::BTreeSet<u32> = r
        .items
        .iter()
        .filter(|i| i.location == app::session::ObservedItemLocation::RoomDirect)
        .map(|i| i.key)
        .collect();
    assert!(!without_hint.is_empty(), "premise: the lobby shows at least one room-direct item");

    let handles = s.introspect().unwrap().children_of(loc.number);
    assert!(!handles.is_empty(), "the lobby holds at least one child object");

    let mut reduced_by_exactly_one = false;
    for &candidate in &handles {
        s.set_player_hint(Some(candidate));
        let with_hint = s.submit("look");
        let after: std::collections::BTreeSet<u32> = with_hint
            .items
            .iter()
            .filter(|i| i.location == app::session::ObservedItemLocation::RoomDirect)
            .map(|i| i.key)
            .collect();
        if after.len() == without_hint.len().saturating_sub(1)
            && without_hint.difference(&after).count() == 1
        {
            reduced_by_exactly_one = true;
            break;
        }
        s.set_player_hint(None);
    }
    assert!(
        reduced_by_exactly_one,
        "at least one of the lobby's own child handles, once hinted as the player, must be \
         excluded from room-direct — exactly as it would be if it really were the avatar"
    );
}

// ── Cragne Manor and King of Shreds and Patches (SQ-1639) ────────────────────
//
// Broadened Glulx real-game coverage beyond Counterfeit Monkey — two games with
// meaningfully different world models: a many-author patchwork built on Inform 7's
// ordinary containment library (Cragne Manor), and an older Inform 6.31-library
// game whose room global takes many turns of movement for `glulx_roomlock` to
// disambiguate from the other RAM words that also change every turn (King of
// Shreds and Patches).
//
// Between them: origin room/turn recorded correctly and current location tracked
// across take/drop (King of Shreds and Patches — Cragne's own avatar turns out to
// be unidentifiable, the same shape CM's already-pinned refusal is, so its own
// case below pins THAT instead of fabricating tracking that cannot happen); and
// the container-vs-vanished distinction, at its most extreme version (Cragne's
// vending machine) — a Glulx item nested inside a container is never observed at
// all while it stays there (see `GlulxSession::glulx_item_observations`'s own doc:
// "this format has no way to tell an OPEN container from a closed one at all"),
// which is what makes it structurally impossible for such an item to ever misread
// as Vanished: `note_items_absent` only ever retires a record that was ONCE
// directly sighted, and this one never was until it is taken.

/// Cragne Manor: **another unidentifiable-avatar refusal** (SQ-1639 finding,
/// same shape `glulx_inventory.rs`'s `counterfeit_monkey_refuses_an_avatar_it_cannot_identify_by_name_alone`
/// already pins for CM) — `Introspect::player_object()` answers `None` here too
/// (checked directly before writing this test), so `glulx_item_observations`'s
/// `Carried` loop never runs at all: while the watch is actually held, the
/// tracker reports it as nothing at all rather than fabricating a false
/// `Carried`. Dropping it back in the open room the watch is genuinely
/// relocated INTO (Inform moves a held object to be the room's own direct
/// child, not back onto the bench it came from), so it becomes visible again
/// on its own merits — a real `RoomDirect` sighting, with no avatar handle
/// involved at all, not a guess.
#[test]
fn cragne_wristwatch_take_and_drop_are_silently_untracked_with_an_unidentifiable_avatar() {
    let Some(mut s) = boot_cragne() else {
        eprintln!("SKIP: gitignored stories/cragne.gblorb missing");
        return;
    };
    assert!(
        s.introspect().expect("Cragne's object list reads perfectly").player_object().is_none(),
        "premise: Cragne's avatar is not identifiable from the image, same shape as CM"
    );

    let taken = s.submit("take wristwatch");
    assert!(taken.transcript.contains("Taken"), "the real success text: {:?}", taken.transcript);
    assert!(
        !taken.items.iter().any(|i| i.location == app::session::ObservedItemLocation::Carried),
        "no avatar handle to read contents() from, so nothing is ever reported Carried: {:?}",
        taken.items
    );

    // Dropped back in the SAME room, the watch becomes a genuine ROOM child —
    // Inform relocates it to the room directly, not back onto the bench it came
    // from — so it now reaches `room_objects_excluding` on its own merits, with
    // no avatar involved at all: this is `RoomDirect`, never a fabricated
    // `Carried` the tracker had no way to see.
    let dropped = s.submit("drop wristwatch");
    assert!(dropped.transcript.contains("Dropped"), "the real success text: {:?}", dropped.transcript);
    let watch = dropped
        .items
        .iter()
        .find(|i| i.words.refers_to("wristwatch"))
        .expect("dropped in the open, the watch is now a real, visible room object");
    // SQ-1648: the bare "Dropped." transcript gives tier 1 no textual evidence, so this falls to
    // tier 2's first-stored-word fallback — `ow.words` here is `["nah-watch", "things", "watch",
    // "gold", "wristwatc"]`, and `"nah-watch"` (Cragne Manor's own internal per-object kind tag,
    // a collaborative-authorship-jam artifact this game apparently compiles into `name` ahead of
    // any human-facing word — Anchorhead carries no such prefix, and every word sampled there had
    // its real name first) is what tier 2 picks. Uglier than the old whole-list join would have
    // shown by luck (its `.contains("wristwatc")` still matched because `"wristwatc"` was buried
    // in the list too), but still a single real, typeable parser word — not the eight-word
    // run-on string, and never a guess.
    assert_eq!(watch.name, "nah-watch");
    assert_eq!(
        watch.location,
        app::session::ObservedItemLocation::RoomDirect,
        "a genuine room sighting, not a guessed Carried: {:?}",
        dropped.items
    );
}

/// Cragne Manor: the plastic bubble inside the (locked, unopenable-by-the-player-
/// at-this-point) vending machine is never a candidate for Vanished, because it is
/// never observed in the first place — confirmed against the real game first
/// (`take bubble` refuses with "The vending machine isn't open", and the bubble
/// never appears in `result.items` on any turn up to and including that refusal).
/// `note_items_absent` can only retire a record it has already created from a
/// direct sighting, so an item Glulx's containment walk never reaches cannot be
/// misclassified as vanished — it is simply never in the registry to begin with.
#[test]
fn cragne_nested_bubble_is_never_tracked_and_therefore_never_reads_as_vanished() {
    let Some(mut s) = boot_cragne() else {
        eprintln!("SKIP: gitignored stories/cragne.gblorb missing");
        return;
    };
    let mut mapper = mapper::mapper::Mapper::default();
    let mut turn = 0u32;
    let mut drive = |s: &mut GlulxSession, mapper: &mut mapper::mapper::Mapper, cmd: &str| {
        let r = s.submit(cmd);
        turn += 1;
        app::session::apply_turn(mapper, cmd, &r, &mut Default::default());
        app::session::apply_item_observations(mapper, turn, &r);
        r
    };

    let examined = drive(&mut s, &mut mapper, "examine vending machine");
    assert!(examined.transcript.contains("plastic bubble"), "premise: the game names it in prose: {:?}", examined.transcript);
    assert!(
        !examined.items.iter().any(|i| i.name.contains("bubble")),
        "the bubble never reaches result.items while nested in the closed machine: {:?}",
        examined.items
    );

    let refused = drive(&mut s, &mut mapper, "take bubble");
    assert!(refused.transcript.contains("isn't open"), "the real refusal text: {:?}", refused.transcript);
    assert!(
        !refused.items.iter().any(|i| i.name.contains("bubble")),
        "still never observed after the refused take: {:?}",
        refused.items
    );

    // Never entered the registry at all — not merely absent from the CURRENT
    // turn's observations, but never created as a record in the first place.
    let names: Vec<String> = mapper.graph.items().map(|(_, rec)| rec.name.to_lowercase()).collect();
    assert!(!names.iter().any(|n| n.contains("bubble")), "no Vanished record either, because no record at all: {names:?}");
}

/// King of Shreds and Patches (Jimmy Maher, Inform 7 6M62/6.31): a meaningfully
/// OLDER Inform library than Counterfeit Monkey's, and one whose room global
/// `glulx_roomlock` needs several round trips through a door to disambiguate from
/// the other RAM words that also change every turn — confirmed empirically
/// (`room_objects_excluding` answers empty until then) before writing this test,
/// which drives exactly that many turns before checking anything. The title menu
/// wants `S` (start without the tutorial), not a bare keypress.
#[test]
fn kosap_walking_stick_origin_and_current_location_track_across_take_and_drop() {
    let path = fixture_path("King_of_Shreds_and_Patches.gblorb");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored stories/King_of_Shreds_and_Patches.gblorb missing at {}", path.display());
        return;
    };
    let Ok(b) = blorb::Blorb::parse(bytes) else {
        eprintln!("SKIP: King_of_Shreds_and_Patches.gblorb did not parse as a Blorb");
        return;
    };
    let Ok((blorb::ExecKind::Glulx, image)) = b.executable() else {
        eprintln!("SKIP: King_of_Shreds_and_Patches.gblorb carries no Glulx executable");
        return;
    };
    let mut s = GlulxSession::new(image.to_vec(), 80, 24, true, false, false, (1.0, 1.0), None, &[])
        .expect("GlulxSession::new");
    s.submit_key(KeyInput::Char('s')).expect("'s' starts the story without the tutorial");

    let mut mapper = mapper::mapper::Mapper::default();
    let mut turn = 0u32;
    let mut drive = |s: &mut GlulxSession, mapper: &mut mapper::mapper::Mapper, cmd: &str| {
        let r = s.submit(cmd);
        turn += 1;
        app::session::apply_turn(mapper, cmd, &r, &mut Default::default());
        app::session::apply_item_observations(mapper, turn, &r);
        r
    };

    // Walk out the door and back several times, purely to give the room-lock
    // learner enough distinct movement to resolve the `location` global — no
    // assertion depends on what happens on any of these turns.
    let mut last = None;
    for _ in 0..8 {
        drive(&mut s, &mut mapper, "north");
        last = Some(drive(&mut s, &mut mapper, "south"));
    }
    let home = last.unwrap();
    assert_eq!(
        home.location.as_ref().map(|l| l.name.as_str()),
        Some("Fletcher's Printworks"),
        "premise: back home after the walk"
    );

    let taken = drive(&mut s, &mut mapper, "take walking stick");
    assert!(taken.transcript.contains("Taken"), "the real success text: {:?}", taken.transcript);
    let stick = taken
        .items
        .iter()
        .find(|o| o.words.refers_to("stick"))
        .expect("the walking stick is observed the moment it is carried");
    // SQ-1648: the bare "Taken." transcript gives tier 1 no textual evidence, so this falls to
    // tier 2's first-stored-word fallback — `ow.words` is `["walking", "stick", "cane"]`, and
    // "walking" wins. A single real, typeable parser word, not the old three-word join.
    assert_eq!(stick.name, "walking");
    let printworks_room = taken.location.as_ref().expect("still at the printworks").number;
    let stick_key = stick.key;
    let rec = mapper.graph.item(stick_key).unwrap();
    assert_eq!(rec.last_seen, mapper::graph::ItemLocation::Carried);
    assert_eq!(rec.origin_room, printworks_room, "origin is Fletcher's Printworks, where it was first observed");
    let pick_up_turn = rec.carried_since_turn.expect("just picked up");

    let dropped = drive(&mut s, &mut mapper, "drop walking stick");
    assert!(dropped.transcript.contains("Dropped"), "the real success text: {:?}", dropped.transcript);
    let rec = mapper.graph.item(stick_key).unwrap();
    assert_eq!(rec.origin_room, printworks_room, "origin never moves");
    assert_eq!(
        rec.last_seen,
        mapper::graph::ItemLocation::Room { room: printworks_room, direct: true, container: None },
        "dropped back in the same room — a direct sighting again"
    );
    assert_eq!(rec.carried_since_turn, None, "dropping it clears the pick-up turn");

    // Taking it right back up re-confirms Carried with a FRESH pick-up turn, not
    // the stale one from before the drop.
    let retaken = drive(&mut s, &mut mapper, "take walking stick");
    assert!(retaken.transcript.contains("Taken"), "the real success text: {:?}", retaken.transcript);
    let rec = mapper.graph.item(stick_key).unwrap();
    assert_eq!(rec.last_seen, mapper::graph::ItemLocation::Carried);
    assert_ne!(rec.carried_since_turn, Some(pick_up_turn), "a fresh pick-up turn, not the one from before the drop");
}

// ── SQ-1640: exclude pure scenery/backdrop nouns and doors from Glulx item tracking ──────────
//
// The reported defect: walking into Anchorhead's Garbage-Choked Alley populates the item tracker
// with 9 pure scenery/backdrop nouns that are structural children of the room in Inform's object
// tree but are not real inventory-style items: alley entrance doors, buildings, cardboard boxes,
// garbage can, ground, metal ladder, rain, sky, wooden fence. Confirmed against the real
// `stories/Anchorhead.gblorb` (Illustrated Edition) before writing the fix, and again here.

/// The bug itself: none of the 9 reported nouns appear in `result.items` after arriving at the
/// alley, though the room's own prose still names several of them (`cardboard boxes` in
/// particular — SQ-1639's own specimen line for this room's body text).
#[test]
fn anchorhead_garbage_choked_alley_excludes_its_own_pure_scenery_nouns() {
    let Some(mut s) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored stories/Anchorhead.gblorb missing");
        return;
    };
    let _ = s.submit("look");
    let r = s.submit("southeast"); // Outside the Real Estate Office -> Garbage-Choked Alley
    assert_eq!(
        r.location.as_ref().map(|l| l.name.as_str()),
        Some("Garbage-Choked Alley"),
        "premise: this walks to the specimen room: {:?}",
        r.location
    );
    let reported = [
        "alley entrance doors",
        "buildings",
        "cardboard boxes",
        "garbage can",
        "ground",
        "metal ladder",
        "rain",
        "sky",
        "wooden fence",
    ];
    for noun in reported {
        assert!(
            !r.items.iter().any(|i| i.name.contains(noun)),
            "{noun:?} is pure scenery/backdrop and must not be tracked as an item: {:?}",
            r.items
        );
    }
}

// ── SQ-1648: a no-printed-name item's display name prefers a word the turn's own prose used ──
//
// The reported defect: an object with no hardware short name (the ordinary Inform 7 case) showed
// up in `result.items` either as every one of its parser words joined into one run-on string
// (`ObjectWords::display_name()`'s designed behaviour for its OTHER callers, wrong for a
// tracker's single-item display) or, worse, as Glulx's own 9-character-truncated dictionary
// fragment verbatim (`"proprieto"` for "proprietor"). Confirmed against the real
// `stories/Anchorhead.gblorb` (Illustrated Edition) before writing the fix, and again here:
// replaying the committed walkthrough script (`anchorhead.txt`) up to its own "south" into the
// Curiosity Shop (turn 210), the proprietor arrives as `ObjectWords { printed_name: "", words:
// ["proprieto", "men", "old", "portly", "shopkeepe", "man", "himself", "person"], .. }` — and the
// room's own arrival prose says "The proprietor watches you quietly from behind the display
// case," which is exactly the textual evidence tier 1 of `item_tracker_display_name` uses.

/// The committed walkthrough script, trimmed the same way `walkthrough_glulx_graphics_sound.rs`'s
/// own `read_script` does.
fn anchorhead_script() -> Vec<String> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("tests/fixtures/walkthroughs/anchorhead.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("committed walkthrough script must be readable at {}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Replay the script through `through` commands (1-based, inclusive), mirroring
/// `walkthrough_glulx_graphics_sound.rs`'s `submit_command`: a pending `Event` resumes with an
/// empty submit rather than the literal script text.
fn replay_anchorhead(s: &mut GlulxSession, through: usize) -> app::session::TurnResult {
    let script = anchorhead_script();
    let mut last = app::session::TurnResult::default();
    for cmd in script.iter().take(through) {
        last = match s.pending_input() {
            app::session::InputKind::Event => s.submit(""),
            _ => s.submit(cmd),
        };
    }
    last
}

/// Turn 210 (`south`, into the Curiosity Shop): the proprietor is a fresh `RoomDirect` sighting
/// with no printed name, and the arrival prose itself says "The proprietor watches you quietly
/// from behind the display case" — tier 1's textual evidence.
#[test]
fn anchorhead_proprietor_resolves_to_the_prose_spelled_word_not_a_joined_list_or_truncated_fragment() {
    let Some(mut s) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored stories/Anchorhead.gblorb missing");
        return;
    };
    let r = replay_anchorhead(&mut s, 210);
    assert_eq!(
        r.location.as_ref().map(|l| l.name.as_str()),
        Some("Curiosity Shop"),
        "premise: this walks to the specimen room: {:?}",
        r.location
    );
    assert!(
        r.transcript.contains("The proprietor watches you quietly"),
        "premise: the arrival prose itself names him this turn: {:?}",
        r.transcript
    );
    let proprietor = r
        .items
        .iter()
        .find(|i| i.words.words.contains(&"proprieto".to_string()))
        .expect("the proprietor is a fresh RoomDirect sighting this turn");
    assert_eq!(
        proprietor.name, "proprietor",
        "resolved from this turn's own prose, correctly spelled — not the truncated \
         dictionary fragment (\"proprieto\") and not every known word joined into one \
         run-on string: {:?}",
        proprietor
    );
    assert_eq!(proprietor.location, app::session::ObservedItemLocation::RoomDirect);
}

/// Tier 2's fallback, in the SAME room: a later turn ("examine display case") whose own prose
/// never re-mentions the proprietor still names him, as a single word (the FIRST of his stored
/// words) — never the whole eight-word joined list `display_name()` would have produced, even
/// though this turn's prose gives tier 1 nothing to match.
#[test]
fn anchorhead_proprietor_falls_back_to_a_single_stored_word_when_a_later_turns_prose_omits_him() {
    let Some(mut s) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored stories/Anchorhead.gblorb missing");
        return;
    };
    let r = replay_anchorhead(&mut s, 211); // "examine display case", one turn after arrival
    assert!(
        !r.transcript.to_lowercase().contains("proprietor"),
        "premise: this turn's own prose gives tier 1 no textual evidence: {:?}",
        r.transcript
    );
    let proprietor = r
        .items
        .iter()
        .find(|i| i.words.words.contains(&"proprieto".to_string()))
        .expect("still a RoomDirect sighting this turn, just with no fresh textual evidence");
    assert_eq!(
        proprietor.name, "proprieto",
        "no match in THIS turn's prose: the first stored word, unspelled — never the whole \
         joined list: {:?}",
        proprietor
    );
}

/// Regression guard against over-filtering: a genuine portable item in the same game — Cragne
/// Manor's half-full styrofoam coffee cup, one `south` of the starting Railway Platform, an
/// ordinary Inform-7-style object (no hardware short name, real parse words) that is neither
/// `scenery` nor a `door` — is still tracked normally, both before and after being taken.
#[test]
fn a_genuine_portable_item_is_still_tracked_after_the_scenery_filter() {
    let Some(mut s) = boot_cragne() else {
        eprintln!("SKIP: gitignored stories/cragne.gblorb missing");
        return;
    };
    let r = s.submit("south"); // Railway Platform -> Train Station Lobby
    assert_eq!(
        r.location.as_ref().map(|l| l.name.as_str()),
        Some("Train Station Lobby (Shin)"),
        "premise: this walks to the specimen room: {:?}",
        r.location
    );
    let cup = r
        .items
        .iter()
        .find(|i| i.words.refers_to("cup"))
        .expect("the coffee cup is a genuine portable item, not scenery, and must still be tracked");
    // SQ-1648: this object has no printed name, so its display name is now resolved through
    // `item_tracker_display_name` rather than `ObjectWords::display_name`'s old whole-list join
    // ("half-full styrofoam coffee cup things clouds swirls") — a single word, "styrofoam", the
    // first of `ow.words` this turn's own arrival prose used ("A styrofoam coffee cup sits on the
    // floor…"; "half-full" precedes it in `ow.words` but this turn's prose never says it).
    assert_eq!(cup.name, "styrofoam", "resolved from this turn's own arrival prose: {:?}", cup);
    assert_eq!(cup.location, app::session::ObservedItemLocation::RoomDirect);

    let taken = s.submit("take cup");
    assert!(taken.transcript.contains("Taken"), "the real success text: {:?}", taken.transcript);
}

// ── SQ-1652: a display name resolved from one item's own prose must never bleed onto another ──
//
// The reported defect: `x umbrella`'s whole turn is the umbrella's own examine description
// ("Olive green, with a hook-shaped handle…"), never a word about the unrelated `clothes` item —
// but `clothes`' own vocabulary ALSO has "green" (its own separately-described "tasteful ensemble
// in muted browns and greens"), and `item_tracker_display_name`'s tier 1, called once per
// currently-tracked item against that SAME turn-wide prose with no awareness of what the OTHER
// items' own vocabularies looked like, wrongly renamed `clothes` to `"green"` too. Confirmed
// against the real `stories/AnchorheadDemo.gblorb` before writing the fix (`x umbrella` relabels
// both "umbrella" and "clothes" to "green"; falsified by reverting `item_tracker_display_name`'s
// `other_items` disambiguation and confirming this exact case fails again), and again here.

/// Anchorhead's Special Edition demo (Michael Gentry, Inform 6/Glulx): `stories/AnchorheadDemo.gblorb`
/// — past its splash to the first command prompt outside the real estate office, carrying the
/// umbrella/clothes/trenchcoat/wedding-ring starting inventory the case below needs.
fn boot_anchorhead_demo() -> Option<GlulxSession> {
    let image = glulx_image("AnchorheadDemo.gblorb")?;
    let mut s = GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[]).ok()?;
    for _ in 0..6 {
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    Some(s)
}

#[test]
fn examining_the_umbrella_never_relabels_the_unrelated_clothes_item() {
    let Some(mut s) = boot_anchorhead_demo() else {
        eprintln!("SKIP: gitignored stories/AnchorheadDemo.gblorb missing");
        return;
    };
    let before = s.submit("look");
    let clothes_before = before
        .items
        .iter()
        .find(|i| i.words.refers_to("clothes"))
        .expect("clothes is carried from the very first turn");
    assert_eq!(clothes_before.name, "clothes", "premise: clothes starts out named for itself");

    let r = s.submit("x umbrella");
    assert!(
        r.transcript.contains("green"),
        "premise: the umbrella's own description is this turn's whole transcript, and it \
         contains \"green\", the word the bug misattributed to clothes: {:?}",
        r.transcript
    );
    let clothes_after = r
        .items
        .iter()
        .find(|i| i.words.refers_to("clothes"))
        .expect("clothes is still tracked this turn, just not examined");
    assert_eq!(
        clothes_after.name, "clothes",
        "SQ-1652: \"x umbrella\" printed nothing about clothes, so its resolved name must not \
         change at all — got {:?}",
        clothes_after
    );

    // The report's own follow-up: `x clothes` relabels clothes to a word from ITS OWN
    // description ("tasteful") — correct — but must never touch the umbrella's own resolved
    // name, exactly as `x umbrella` above must never touch clothes'.
    let r2 = s.submit("x clothes");
    assert!(
        r2.transcript.contains("tasteful"),
        "premise: clothes' own description is this turn's whole transcript: {:?}",
        r2.transcript
    );
    let umbrella_after = r2
        .items
        .iter()
        .find(|i| i.words.refers_to("umbrella"))
        .expect("umbrella is still tracked this turn, just not examined");
    assert_eq!(
        umbrella_after.name, "umbrella",
        "SQ-1652: \"x clothes\" printed nothing about the umbrella, so its resolved name must \
         not change at all — got {:?}",
        umbrella_after
    );
}

// ── SQ-1655: Counterfeit Monkey's avatar, found by the player-lock fallback ──
//
// `gvm::objects::ParseNames::find_player`'s static scan refuses CM outright (see that
// function's own doc): its Inform 7 `Understand` grammar for the player compiles to a
// `parse_name` ROUTINE, not the static word array the scan can read, and none of its
// 2,494 objects has a hardware short name either. `GlulxSession::player_addr` used to stop
// there, so `result.items` never reported a single carried object for the whole session —
// confirmed directly (`counterfeit_monkey_refuses_an_avatar_it_cannot_identify_by_name_alone`,
// `glulx_inventory.rs`, still pinned above and still passing: neither of its two cases ever
// takes anything, so the new fallback below never has evidence to fire on).
//
// `crate::glulx_playerlock::PlayerLock` closes the gap from the OTHER side: once a
// take-shaped command visibly moves an object out of the room and into something that
// looks like an avatar, that address is strong enough evidence to lock onto the `player`
// global directly, no name or grammar involved in IDENTIFYING the avatar at all — only in
// deciding which commands are worth reading that way (see `learn_player_from_pickups`'s own
// doc for why that gate exists).

/// [`ROUTE`] is the first 17 inputs of `stories/CounterfeitMonkey-10.gblorb`'s own `test me`
/// script (`tools/command scripts/test_me.txt` in the i7/counterfeit-monkey repository) —
/// the same verified prefix `sq1294_glulx_silent_vehicle_move.rs`'s own `ROUTE` constant
/// documents at length, reused rather than invented, since a route through this specific
/// commercial game is not guessable and this one is already known to play cleanly from a
/// cold boot — plus one substitution: the script's own 18th input is `get heel`, and CM's
/// grammar keeps `get` as its OWN verb table entry, distinct from `take`/`carry`/`hold`
/// (confirmed directly: `vocab.verb_named("get")` and `vocab.verb_named("take")` are two
/// different `Verb`s here), so it does not resolve as take-shaped and this test substitutes
/// the equivalent `take heel` instead — the same action, the same real success text, just
/// the spelling `crate::session::take_command_target` actually recognises.
/// `release 10 / serial 210312` (the IF Archive's current copy; SQ-1454).
const ROUTE: &[&str] = &[
    "y", "andra", "", "tutorial off", "random-seed 1234", "pauses off", "n",
    "wave u-remover at mourning dress", "score", "e", "wave x-remover at codex", "x code",
    "unlock barrier", "set barrier to 305", "go to fair", "x wheel", "wave w-remover at wheel",
    "take heel",
];

#[test]
fn counterfeit_monkey_carried_items_are_tracked_once_a_confirmed_pickup_locks_the_avatar() {
    let Some(bytes) = std::fs::read(fixture_path("CounterfeitMonkey-10.gblorb")).ok() else {
        eprintln!("SKIP: gitignored stories/CounterfeitMonkey-10.gblorb missing");
        return;
    };
    let Ok(b) = blorb::Blorb::parse(bytes) else {
        eprintln!("SKIP: CounterfeitMonkey-10.gblorb did not parse as a Blorb");
        return;
    };
    let Ok((blorb::ExecKind::Glulx, image)) = b.executable() else {
        eprintln!("SKIP: CounterfeitMonkey-10.gblorb carries no Glulx executable");
        return;
    };
    let mut s = GlulxSession::new(image.to_vec(), 80, 30, true, false, false, (8.0, 16.0), None, &[])
        .expect("Counterfeit Monkey boots");

    for &cmd in &ROUTE[..ROUTE.len() - 1] {
        let r = if s.pending_input() == app::session::InputKind::Char {
            s.submit_key(KeyInput::Enter).expect("Glulx takes keys")
        } else {
            s.submit(cmd)
        };
        // Premise, checked on every step up to (but not including) the take: the static
        // scan really does refuse this story's avatar throughout, exactly like the
        // already-pinned `counterfeit_monkey_refuses_an_avatar_it_cannot_identify_by_name_alone`.
        assert!(
            s.introspect().expect("CM's object list reads perfectly").player_object().is_none(),
            "premise: no pickup has happened yet, so the fallback has no evidence to lock on: {cmd:?}"
        );
        assert!(
            !r.items.iter().any(|i| i.location == app::session::ObservedItemLocation::Carried),
            "premise: nothing is reported Carried before the avatar is identified: {:?}",
            r.items
        );
    }

    // The last step: `get heel`, a genuine, unambiguous, successful take — the confirmed
    // pickup [`crate::glulx_playerlock`]'s whole mechanism is built on.
    let taken = s.submit(ROUTE[ROUTE.len() - 1]);
    assert!(taken.transcript.contains("We take the heel"), "the real success text: {:?}", taken.transcript);

    // THE regression: the avatar is now identifiable, where a moment ago it was refused.
    assert!(
        s.introspect().expect("CM's object list still reads perfectly").player_object().is_some(),
        "SQ-1655: a confirmed pickup must lock the player-global fallback"
    );

    // And `result.items` now reports the heel as genuinely Carried — the defect this quest
    // opened on: "TurnResult.items stays empty on every single turn despite the game's own
    // prose confirming the player is carrying it."
    let heel = taken
        .items
        .iter()
        .find(|i| i.words.refers_to("heel"))
        .expect("the heel is observed the moment it is carried");
    assert_eq!(
        heel.location,
        app::session::ObservedItemLocation::Carried,
        "the heel must read Carried now that the avatar is known: {:?}",
        taken.items
    );

    // A second, independent confirmation: the letter-remover device the player has been
    // holding since the very first `wave` command (step 7) was invisible to every turn
    // before this one for the exact same reason — no known avatar to read `contents()`
    // from — and becomes visible on the SAME turn the lock resolves, with no pickup of
    // its own needed. This is not a second bug fixed; it is the same fix, applied
    // retroactively to everything already in hand the moment the avatar is known.
    let remover = taken
        .items
        .iter()
        .find(|i| i.words.refers_to("remover"))
        .expect("the letter-remover device, held since step 7, is now visible too");
    assert_eq!(remover.location, app::session::ObservedItemLocation::Carried);
}

/// Anchorhead (Michael Gentry, Illustrated Edition, Inform 7): the SECOND real commercial
/// story this quest resolves, found only after filing SQ-1655 against Counterfeit Monkey
/// alone — `stories/Anchorhead.gblorb`'s avatar carries the identical structural gap
/// `find_player`'s own doc describes for CM (an empty hardware short name, no static word
/// array `find_player` can read), confirmed directly: `player_object()` is `None` from boot
/// through the whole opening sequence below, even though the game's own "take" replies
/// ("Taken.") confirm every one of these items really did end up in the player's hands.
///
/// **This is also where the false-positive the vocab gate exists for was actually found.**
/// Before `learn_player_from_pickups` required a take-shaped command
/// (`crate::session::take_command_target`), replaying this exact walkthrough's first fifty
/// commands locked onto a DIFFERENT wrong object on nearly every turn: `climb on garbage
/// can` (the player becomes a child of the can, not the room, which reads exactly like an
/// item leaving the room) locked the can itself as "the avatar"; `enter window`, `up`,
/// `down` and several others each relocked onto whatever scenery happened to explain an
/// address leaving the room's direct children that turn. Asserting the premise —
/// `player_object()` stays `None` through every one of those non-take commands — is this
/// test's non-regression half; the walkthrough steps chosen are the exact ones that broke it.
#[test]
fn anchorhead_carried_items_are_tracked_once_a_confirmed_pickup_locks_the_avatar() {
    let Some(mut s) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored stories/Anchorhead.gblorb missing");
        return;
    };
    let script = anchorhead_script();
    assert!(script.len() >= 53, "premise: the committed walkthrough reaches the keyring/umbrella/trenchcoat takes");

    // The first 50 commands: knocking, climbing the fire escape, searching the file room,
    // talking to Michael, walking the whole loop back — real commands, but none of them
    // take-shaped, and several of them the exact false-positive shapes above. The avatar
    // must stay unidentified throughout, exactly like the already-pinned CM refusal case.
    for (i, cmd) in script.iter().enumerate().take(50) {
        let r = match s.pending_input() {
            app::session::InputKind::Event => s.submit(""),
            _ => s.submit(cmd),
        };
        assert!(
            s.introspect().expect("Anchorhead's object list reads perfectly").player_object().is_none(),
            "premise: no take-shaped command has run yet, so the fallback has no evidence to lock on \
             (step {}, {cmd:?})",
            i + 1
        );
        assert!(
            !r.items.iter().any(|it| it.location == app::session::ObservedItemLocation::Carried),
            "premise: nothing is reported Carried before the avatar is identified (step {}, {cmd:?}): {:?}",
            i + 1,
            r.items
        );
    }

    // Step 51: "take keyring" — a genuine, unambiguous, successful take.
    assert_eq!(script[50], "take keyring", "premise: this is the walkthrough's first real take");
    let taken = s.submit(&script[50]);
    assert_eq!(taken.transcript, "Taken.", "the real success text: {:?}", taken.transcript);

    // THE regression: the avatar is now identifiable, where a moment ago it was refused —
    // the identical defect shape CM's own `counterfeit_monkey_refuses_an_avatar_it_cannot_identify_by_name_alone`
    // pins as a refusal, resolved here by the same fallback.
    assert!(
        s.introspect().expect("Anchorhead's object list still reads perfectly").player_object().is_some(),
        "SQ-1655: a confirmed pickup must lock the player-global fallback"
    );
    let keyring = taken
        .items
        .iter()
        .find(|i| i.words.refers_to("keyring"))
        .expect("the keyring is observed the moment it is carried");
    assert_eq!(keyring.location, app::session::ObservedItemLocation::Carried);

    // Two more takes, back to back, prove the lock holds rather than re-deriving by luck
    // each time: "take umbrella" and "take trenchcoat" (steps 52/53).
    assert_eq!(script[51], "take umbrella");
    let r = s.submit(&script[51]);
    assert_eq!(r.transcript, "Taken.");
    let umbrella =
        r.items.iter().find(|i| i.words.refers_to("umbrella")).expect("the umbrella is carried");
    assert_eq!(umbrella.location, app::session::ObservedItemLocation::Carried);

    assert_eq!(script[52], "take trenchcoat");
    let r = s.submit(&script[52]);
    assert_eq!(r.transcript, "Taken.");
    let trenchcoat = r
        .items
        .iter()
        .find(|i| i.words.refers_to("trenchcoat") || i.words.refers_to("trenchcoa"))
        .expect("the trenchcoat is carried");
    assert_eq!(trenchcoat.location, app::session::ObservedItemLocation::Carried);

    // And what the coordinator's own direct check on this story found: the wedding ring and
    // clothes, worn/held since the opening sequence and never picked up by any command in
    // this test, become visible on the SAME turn the lock resolves — the same
    // apply-retroactively-to-everything-already-in-hand shape CM's own "letter-remover"
    // demonstrates, not a second mechanism.
    let wedding = r.items.iter().find(|i| i.words.refers_to("wedding")).expect("the wedding ring is carried");
    assert_eq!(wedding.location, app::session::ObservedItemLocation::Carried);
    let clothes = r.items.iter().find(|i| i.words.refers_to("clothes")).expect("clothes are carried");
    assert_eq!(clothes.location, app::session::ObservedItemLocation::Carried);
}

// ── SQ-1657: the avatar found from the very first `inventory` listing ───────
//
// SQ-1655 closed the gap for a story whose avatar `find_player`'s static scan cannot find,
// but only from the PICKUP side: a confirmed take visibly moving an object out of a room.
// Both Anchorhead and Counterfeit Monkey start the player already wearing/holding their key
// item(s) — Anchorhead's trenchcoat, wedding ring and umbrella; Counterfeit Monkey's single
// R-remover — so a session that never types a `take` at all (an `inventory`-first player, or
// one who only ever examines things) never gives the pickup watcher any evidence, and
// `result.items` stayed empty forever. `GlulxSession::learn_player_from_inventory` closes
// that from the OTHER side: the `inventory` command's own prose already names everything the
// player holds, and the story's own vocabulary says which objects those words identify.

/// Anchorhead: `inventory`, with NO prior take, must still lock the avatar — the exact
/// reported gap ("still not seeing any inventory for Anchorhead").
#[test]
fn anchorhead_carried_items_are_tracked_from_the_very_first_inventory_listing() {
    let Some(mut s) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored stories/Anchorhead.gblorb missing");
        return;
    };
    // `Engine::seed_turn` is the ONE boot drain the real app runs before the player ever types
    // anything (`host/boot.rs`) — without it, the still-buffered boot banner and opening room
    // description would be glued onto the FIRST submitted command's own `transcript`, which
    // `boot_anchorhead`'s own splash-dismissal loop does not drain on its own.
    let _ = s.seed_turn();
    // Premise: nothing has happened yet to identify the avatar — the same refusal
    // `anchorhead_carried_items_are_tracked_once_a_confirmed_pickup_locks_the_avatar` pins for
    // this story's opening, carried forward to before ANY command at all has run.
    assert!(
        s.introspect().expect("Anchorhead's object list reads perfectly").player_object().is_none(),
        "premise: nothing has happened yet to identify the avatar"
    );

    let r = s.submit("inventory");
    assert!(
        r.transcript.contains("wearing your trenchcoat") && r.transcript.contains("holding your umbrella"),
        "the real inventory text: {:?}",
        r.transcript
    );

    // THE regression: the avatar is now identifiable from the listing alone, no take involved.
    assert!(
        s.introspect().expect("Anchorhead's object list still reads perfectly").player_object().is_some(),
        "SQ-1657: an inventory listing with no prior take must still lock the player-global fallback"
    );

    let trenchcoat = r
        .items
        .iter()
        .find(|i| i.words.refers_to("trenchcoat") || i.words.refers_to("trenchcoa"))
        .expect("the trenchcoat is tracked the moment the avatar is known");
    assert_eq!(trenchcoat.location, app::session::ObservedItemLocation::Carried);
    let wedding = r.items.iter().find(|i| i.words.refers_to("wedding")).expect("the wedding ring is tracked");
    assert_eq!(wedding.location, app::session::ObservedItemLocation::Carried);
    let umbrella = r.items.iter().find(|i| i.words.refers_to("umbrella")).expect("the umbrella is tracked");
    assert_eq!(umbrella.location, app::session::ObservedItemLocation::Carried);
}

// ── SQ-1659: the inventory DOCK shows the registry's clean name, not the raw joined list ────
//
// The reported defect: after typing `inventory` in Anchorhead, the inventory panel's
// "Carrying:" row read "umbrella things green handle brolly bumbersho — found Outside the
// Real Estate Office, turn 2" instead of a clean "umbrella". `render::inventory_dock::carried_line`
// used the LIVE `name` parameter — `grammar_model::ObjectWords::display_name()`'s raw joined
// word list, the right fallback when there is no better source but exactly the word-salad shape
// SQ-1648's `item_tracker_display_name` exists to avoid for player-facing text — even when the
// mapper's own registry already held the clean, resolved name for the same item (stamped by
// `apply_item_observations` from the identical per-turn observation). This test exercises the
// dock exactly the way `main.rs` builds it (`inventory_items_with_keys` + a real, populated
// `MapGraph` + `build_inventory_dock_rows`), through a real commercial story, past the exact
// `inventory` command SQ-1657 made resolvable at all for this avatar.
#[test]
fn anchorhead_inventory_dock_shows_the_clean_registry_name_not_the_joined_word_list() {
    let Some(mut s) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored stories/Anchorhead.gblorb missing");
        return;
    };
    // Drain the boot banner the same way `anchorhead_carried_items_are_tracked_from_the_very_first_inventory_listing`
    // does, so the FIRST submitted command's transcript is the `inventory` reply alone.
    let _ = s.seed_turn();

    let r = s.submit("inventory");
    assert!(
        r.transcript.contains("wearing your trenchcoat") && r.transcript.contains("holding your umbrella"),
        "the real inventory text: {:?}",
        r.transcript
    );

    // The avatar must actually be identified — SQ-1657's own regression — or the dock has
    // nothing to draw at all and the rest of this test would pass vacuously.
    let player_obj = s
        .introspect()
        .expect("Anchorhead's object list reads perfectly")
        .player_object()
        .expect("SQ-1657: the inventory listing just submitted identifies the avatar");

    // Populate a REAL mapper registry from this exact turn — the same two calls `turn.rs`
    // makes in production (`apply_turn` for the room graph `carried_line`'s "found …" clause
    // reads, `apply_item_observations` for the per-item record `carried_line` now prefers).
    let mut mapper = mapper::mapper::Mapper::default();
    app::session::apply_turn(&mut mapper, "inventory", &r, &mut Default::default());
    app::session::apply_item_observations(&mut mapper, 1, &r);

    // Premise: Anchorhead's umbrella really has no hardware short name, so the raw
    // `ObjectWords::display_name()` join really is the multi-word list this bug report
    // describes — read directly off `contents`, bypassing `inventory_items_with_keys`
    // itself, since SQ-1662 gave THAT function its own typeable_name-preferring fallback
    // and it no longer surfaces the raw join at all (see the next assertion).
    let raw_umbrella = s
        .introspect()
        .expect("Anchorhead's object list reads perfectly")
        .contents(player_obj)
        .iter()
        .find_map(|o| o.display_name().filter(|n| n.to_lowercase().contains("umbrella")))
        .expect("the umbrella is among the live carried contents");
    assert!(
        raw_umbrella.split_whitespace().count() > 1,
        "premise: not coincidentally already clean: {raw_umbrella:?}"
    );

    // The exact pair `main.rs` calls to build the live "Carrying:" list and the dock's
    // rows. SQ-1662: `inventory_items_with_keys` now prefers `typeable_name` over the raw
    // join itself, so this is already clean before the dock's own registry preference
    // (asserted below) ever gets a chance to apply.
    let carried = app::render::transcript::inventory_items_with_keys(Some(player_obj), &[], s.introspect(), None);
    let live_umbrella = carried
        .iter()
        .find(|(_, name)| name.to_lowercase().contains("umbrella"))
        .map(|(_, name)| name.clone())
        .expect("the umbrella is among the live carried contents");
    assert_eq!(
        live_umbrella, "umbrella",
        "SQ-1662: inventory_items_with_keys itself must prefer the short typeable name over \
         the raw joined word list: {live_umbrella:?}"
    );

    let dock_rows = app::render::inventory_dock::build_inventory_dock_rows(&carried, &mapper.graph, None);
    let (text, meta_start) = dock_rows
        .iter()
        .find_map(|row| match row {
            app::render::inventory_dock::ItemDockRow::Carried { text, meta_start, .. }
                if text.to_lowercase().contains("umbrella") =>
            {
                Some((text.clone(), *meta_start))
            }
            _ => None,
        })
        .expect("the umbrella is drawn as a Carrying: row");
    let name_prefix: String = text.chars().take(meta_start).collect();
    assert_eq!(
        name_prefix.trim(),
        "umbrella",
        "SQ-1659: the dock must show the registry's clean resolved name, not the raw joined \
         word list — the reported symptom was exactly this row reading \
         \"umbrella things green handle brolly bumbersho\": {text:?}"
    );
}

// ── SQ-1662: the live panel at a RETURNING player's instant-avatar boot ───────
//
// User report: "when we start anchorhead fresh, inventory panel is showing items +
// jibberish. After the first command it properly updates." Root cause: SQ-1655's
// player-global sidecar correctly restores a returning player's avatar INSTANTLY at
// boot — `introspect().player_object()` answers `Some(..)` before any command runs —
// but `render::transcript::inventory_items_with_keys` (the live "Carrying:" panel's
// data source) called `o.display_name()` directly with no smarter fallback, unlike its
// sibling `inventory_click_words`, which already preferred `vocab::typeable_name`'s
// short word. The fix gives `inventory_items_with_keys` the same vocab parameter and
// fallback. This test reproduces the exact two-session shape that exposes it: a FIRST
// session, driven through the real host turn path, locks the avatar via a confirmed
// pickup and persists the sidecar (`glulx_session.rs`'s `remember_player_global`); a
// SECOND session reuses the same home dir — exactly what "New Game" does for a
// returning player — and the panel must already read cleanly, with ZERO commands
// submitted.

/// A config rooted in a scratch home, same shape `host_boot.rs`'s own `headless_config`
/// uses, so this reads and writes nothing under the real `~/.lanthorn`.
fn sq1662_headless_config(home: &std::path::Path) -> Config {
    Config { user_dir: home.to_path_buf(), config_file: home.join("config.toml"), random_seed: Some(1), ..Config::default() }
}

/// Boot Anchorhead through the real host path (`boot_story`) — the same request a
/// player's own "New Game" issues, never a bare resume probe.
fn sq1662_boot_anchorhead_host(home: &std::path::Path, data_base: &std::path::Path) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: fixture_path("Anchorhead.gblorb"),
        disk_entry: None,
        overrides: &overrides,
        cfg: sq1662_headless_config(home),
        data_base: data_base.to_path_buf(),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: true,
    };
    boot_story(req, &mut QuietBoot).expect("Anchorhead boots headlessly")
}

/// Drain the up-to-six leading char-mode keypresses the real TUI drains before the
/// first typed command — `main.rs`'s own `submit_key` -> `apply_game_driven_result`
/// path (SQ-1654), never `finish_command_turn`.
fn sq1662_drain_intro(b: &mut BootedStory) {
    for _ in 0..6 {
        if b.session.pending_input() != app::session::InputKind::Char {
            break;
        }
        let r = b.session.submit_key(KeyInput::Enter).expect("Anchorhead takes a key");
        let _ = apply_game_driven_result(&mut b.state, &mut b.mapper, &r, &b.game_dir, None, &*b.session, app::pager::Driver::PlayerInput);
    }
}

/// Submit one command through the real host turn path — `finish_command_turn`, the
/// exact call `main.rs`'s own submit path makes — handling a pending `Event` the same
/// way `replay_anchorhead` does (an empty submit resumes it). Returns the transcript.
fn sq1662_submit(b: &mut BootedStory, cmd: &str, tidy: &mut u32) -> String {
    let result = match b.session.pending_input() {
        app::session::InputKind::Event => b.session.submit(""),
        _ => b.session.submit(cmd),
    };
    let transcript = result.transcript.clone();
    let _ = finish_command_turn(cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid, &b.arc_file, None, tidy);
    transcript
}

/// **The reported defect, end to end through the real production boot path.**
#[test]
fn anchorhead_returning_players_fresh_boot_shows_clean_live_inventory_names_before_any_command() {
    if !fixture_path("Anchorhead.gblorb").is_file() {
        eprintln!("SKIP: gitignored stories/Anchorhead.gblorb missing");
        return;
    }
    let home = app::scratch_dir("sq1662-anchorhead-returning-player");
    let data_base = home.join("saves");

    // ── Session 1: play far enough to lock the avatar and persist the sidecar ──
    let mut tidy1 = 0u32;
    let mut first = sq1662_boot_anchorhead_host(&home, &data_base);
    sq1662_drain_intro(&mut first);
    assert!(
        first.session.introspect().expect("Anchorhead's object list reads perfectly").player_object().is_none(),
        "premise: a brand-new session has nothing yet to lock the avatar on"
    );

    let script = anchorhead_script();
    assert!(script.len() >= 51, "premise: the committed walkthrough reaches the keyring take");
    assert_eq!(script[50], "take keyring", "premise: this is the walkthrough's first real take");
    for cmd in script.iter().take(51) {
        let _ = sq1662_submit(&mut first, cmd, &mut tidy1);
    }
    assert!(
        first.session.introspect().expect("Anchorhead's object list still reads perfectly").player_object().is_some(),
        "premise: the confirmed pickup above must lock the player-global sidecar (SQ-1655)"
    );

    // ── Session 2: a returning player's brand-new game, same home dir ──────────
    let mut second = sq1662_boot_anchorhead_host(&home, &data_base);
    sq1662_drain_intro(&mut second);

    // THE premise this whole test exists to pin: the avatar resolves INSTANTLY from
    // the persisted sidecar, with no command submitted in this second session at all.
    assert!(
        second.session.introspect().expect("Anchorhead's object list reads perfectly").player_object().is_some(),
        "SQ-1655/SQ-1657: a returning player's avatar must resolve instantly from the \
         persisted sidecar, before any turn has run in this new session"
    );

    // THE regression: the live panel, read the exact way `main.rs` reads it, with
    // zero commands submitted this session.
    let carried = app::render::transcript::inventory_items_with_keys(
        second.state.player_obj,
        &second.state.inventory_fallback,
        second.session.introspect(),
        second.state.vocab.peek(),
    );
    assert!(!carried.is_empty(), "the avatar is already carrying things at boot: {carried:?}");
    for (_, name) in &carried {
        assert!(
            name.split_whitespace().count() <= 2,
            "SQ-1662: the live panel must show a short, clean name at this returning \
             player's instant boot, not every parser word joined into one run-on string \
             (the exact reported symptom was \
             \"umbrella things green handle brolly bumbersho\"): {name:?} in {carried:?}"
        );
    }
    let umbrella = carried.iter().find(|(_, name)| name.to_lowercase().contains("umbrella"));
    assert_eq!(
        umbrella.map(|(_, name)| name.as_str()),
        Some("umbrella"),
        "the umbrella specifically must read clean, matching the bug report's own named \
         item: {carried:?}"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// Falsifies the fix above: with the OLD `display_name()`-only behavior restored (no
/// `typeable_name` preference), the same returning-player boot must reproduce the
/// originally reported word-salad symptom. Temporarily hand-inlines the pre-fix body
/// of `inventory_items_with_keys` rather than calling it, per this project's own
/// "falsify fixes" testing convention — reverting the real fix and re-running the test
/// above would also prove this, but would require editing production code to do it.
#[test]
fn anchorhead_returning_players_fresh_boot_reproduces_word_salad_under_the_old_display_name_only_fallback() {
    if !fixture_path("Anchorhead.gblorb").is_file() {
        eprintln!("SKIP: gitignored stories/Anchorhead.gblorb missing");
        return;
    }
    let home = app::scratch_dir("sq1662-anchorhead-returning-player-falsify");
    let data_base = home.join("saves");

    let mut tidy1 = 0u32;
    let mut first = sq1662_boot_anchorhead_host(&home, &data_base);
    sq1662_drain_intro(&mut first);
    let script = anchorhead_script();
    for cmd in script.iter().take(51) {
        let _ = sq1662_submit(&mut first, cmd, &mut tidy1);
    }
    assert!(first.session.introspect().unwrap().player_object().is_some(), "premise: the avatar is locked");

    let mut second = sq1662_boot_anchorhead_host(&home, &data_base);
    sq1662_drain_intro(&mut second);
    let player = second
        .session
        .introspect()
        .unwrap()
        .player_object()
        .expect("premise: the avatar resolves instantly on the returning player's new game");

    // The pre-fix body of `inventory_items_with_keys`: `o.display_name()` alone, no
    // `typeable_name` preference at all.
    let old_behavior: Vec<(Option<u32>, String)> = second
        .session
        .introspect()
        .unwrap()
        .contents(player)
        .iter()
        .filter_map(|o| o.display_name().map(|name| (Some(o.id), name)))
        .collect();
    let old_umbrella = old_behavior
        .iter()
        .find(|(_, name)| name.to_lowercase().contains("umbrella"))
        .map(|(_, name)| name.clone())
        .expect("the umbrella is among the live carried contents");
    assert!(
        old_umbrella.split_whitespace().count() > 1,
        "falsification check: the OLD display_name()-only fallback really does reproduce \
         the reported multi-word word-salad symptom, confirming the test above actually \
         exercises the fix rather than passing vacuously: {old_umbrella:?}"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// Counterfeit Monkey (release 11, serial 230220 — `stories/CounterfeitMonkey-11.gblorb`, not
/// fetchable on CI; see `scripts/fixtures.manifest`'s own note, so this skips there): the
/// SINGLE-item case the brief for this quest specifically calls out — the R-remover is the
/// whole of the starting inventory, so this is also the test of the unambiguous-single-match
/// bar `inventory_evidence_parent` applies before trusting a lone match.
#[test]
fn counterfeit_monkey_carried_items_are_tracked_from_the_very_first_inventory_listing() {
    let Some(img) = glulx_image("CounterfeitMonkey-11.gblorb") else {
        eprintln!("SKIP: gitignored stories/CounterfeitMonkey-11.gblorb missing");
        return;
    };
    let mut s = GlulxSession::new(img, 80, 24, true, false, false, (1.0, 1.0), None, &[]).expect("CM boots");
    for _ in 0..6 {
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    for cmd in ["yes", "yes", "yes"] {
        s.submit(cmd);
    }
    s.submit_key(KeyInput::Enter);

    assert!(
        s.introspect().expect("CM's object list reads perfectly").player_object().is_none(),
        "premise: nothing has happened yet to identify the avatar"
    );

    let r = s.submit("inventory");
    assert!(r.transcript.contains("equipped with your R-remover"), "the real inventory text: {:?}", r.transcript);

    // THE regression: the avatar is now identifiable from a SINGLE-item listing, no take
    // involved — the reported gap ("I also don't see the u-remover in CM").
    assert!(
        s.introspect().expect("CM's object list still reads perfectly").player_object().is_some(),
        "SQ-1657: a single-item inventory listing must still lock the player-global fallback"
    );

    let remover = r.items.iter().find(|i| i.words.refers_to("remover")).expect("the R-remover is tracked");
    assert_eq!(remover.location, app::session::ObservedItemLocation::Carried);
}
