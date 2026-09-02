//! SQ-0668: the carried-items source, driven against real Infocom stories.
//!
//! The command panel's *carried* column and the inventory panel read the same two
//! things — `Introspect::player_object()` and `Introspect::contents(player)` —
//! so both are empty forever whenever the avatar is misidentified. Zork 1 (r52)
//! is the case that was wrong: it ships TWO objects with avatar names, and only
//! one of them is the player.
//!
//! The unit-level pin lives in `zvm::location` (a synthetic story with the same
//! topology); this file is the real-game check the original SQ-0212 fix lacked —
//! its hand-built machine encoded the wrong Zork 1 topology, so it passed while
//! the game stayed broken. Commercial stories are gitignored, so every test here
//! skips vacuously when its fixture is absent.


use app::engine::Engine;
use app::graphics::PictSource;
use app::render::transcript::inventory_items;
use app::session::GameSession;

use crate::fixture_paths::fixture_path;


/// Boot a plain Z-machine story from `stories/`, or `None` when the gitignored
/// fixture is absent.
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

fn lower(items: &[String]) -> String {
    items.join(" | ").to_lowercase()
}

/// Zork 1 r52: taking something must show up in the carried source, and the
/// player must never have to type "i" to get there.
///
/// The bug: Zork 1 has #21 "you" — the parser's stand-in for the player as a
/// noun, parked in the "it" globals container — alongside #46 "cretin", the
/// real avatar in the room. Both have a non-zero parent, so "prefer the
/// lowest-numbered situated candidate" picked the noun, whose child chain is
/// empty for the whole game.
#[test]
fn zork1_carried_items_are_live_without_ever_typing_inventory() {
    let Some(mut session) = boot("zork1-invclues-r52-s871125.z5") else { return };

    // The avatar is the object that is actually in the room, not merely one
    // with an avatar-ish name.
    let player = session
        .introspect()
        .and_then(|i| i.player_object())
        .expect("Zork 1 has an identifiable player object");
    let room = session.current_location().expect("Zork 1 opens in West of House");
    assert_eq!(room.name, "West of House");
    assert_eq!(
        zvm::objects::short_name(&session.machine.mem, player),
        "cretin",
        "the avatar is #{player} — must be \"cretin\", never the globals-parked \"you\""
    );
    assert_eq!(
        zvm::objects::get_parent(&session.machine.mem, player),
        room.number,
        "the avatar sits in the detected room"
    );

    // The *here* column source: the opening room really does hold the mailbox
    // and the boarded door.
    let here: Vec<String> = session
        .introspect()
        .unwrap()
        .room_objects(room.number)
        .iter()
        .filter_map(|o| o.display_name())
        .collect();
    let here_l = lower(&here);
    assert!(here_l.contains("mailbox"), "here-column lists the mailbox: {here:?}");
    assert!(here_l.contains("door"), "here-column lists the front door: {here:?}");

    // Nothing carried yet — and this is the honest empty, read from the tree.
    assert!(
        inventory_items(None, &[], session.introspect()).is_empty(),
        "the player starts empty-handed"
    );

    // Take the leaflet. No "i", "inv" or "inventory" is ever submitted, so the
    // transcript-scrape fallback stays empty on purpose: everything below comes
    // from the live object tree.
    session.submit("open mailbox");
    session.submit("take leaflet");

    let items = inventory_items(None, &[], session.introspect());
    assert!(
        lower(&items).contains("leaflet"),
        "the taken leaflet must appear in the carried source: {items:?}"
    );

    // Same answer through the LOCKED path the turn loop uses (`state.player_obj`
    // seeded from `player_object()`), so the dock and the band cannot disagree.
    assert_eq!(
        inventory_items(Some(player), &[], session.introspect()),
        items,
        "the locked avatar and the live lookup must be the same object"
    );

    // …and dropping it takes it back out: the column is the tree, not a log.
    session.submit("drop leaflet");
    assert!(
        !lower(&inventory_items(None, &[], session.introspect())).contains("leaflet"),
        "the dropped leaflet leaves the carried source"
    );
}

/// Planetfall names its avatar plainly "player" and starts the game carrying
/// four things, so a boot-time read is enough to catch a missed avatar. The
/// same name family covers Deadline.
#[test]
fn planetfall_carried_items_are_live_at_boot() {
    let Some(session) = boot("planetfall-invclues-r10-s880531.z5") else { return };
    let items = inventory_items(None, &[], session.introspect());
    let l = lower(&items);
    assert!(
        l.contains("diary") && l.contains("chronometer"),
        "Planetfall opens carrying its diary and chronometer: {items:?}"
    );
}

/// Mini-Zork keeps working: one avatar candidate, no room validation needed.
/// Pins that the extra discrimination did not cost the single-candidate games
/// anything.
#[test]
fn minizork_carried_items_still_track_the_object_tree() {
    let Some(mut session) = boot("minizork-r34-s871124.z3") else { return };
    assert!(inventory_items(None, &[], session.introspect()).is_empty());
    session.submit("open mailbox");
    session.submit("take leaflet");
    assert!(
        lower(&inventory_items(None, &[], session.introspect())).contains("leaflet"),
        "minizork's carried source still follows the tree"
    );
}

// ── SQ-1244: the inventory panel's items click into the prompt ─────────────

/// Zork 1 r52: take the leaflet, derive the inventory dock's click word for it
/// the same way `render::inventory_dock::refresh_inventory_click_words` does
/// (`inventory_click_words`, the WHAT column's own `typeable_name`
/// derivation), then drive the real `Action::InventoryClickRow` composition
/// path against a real story rather than a synthetic object model.
#[test]
fn zork1_inventory_click_composes_the_parser_word_onto_the_prompt() {
    use app::input::{apply_action, Action};
    use app::render::transcript::inventory_click_words;
    use app::state::AppState;
    use mapper::mapper::Mapper;

    let Some(mut session) = boot("zork1-invclues-r52-s871125.z5") else { return };
    session.submit("open mailbox");
    session.submit("take leaflet");

    // The dock shows the printed name…
    let display = inventory_items(None, &[], session.introspect());
    assert!(lower(&display).contains("leaflet"), "the dock shows the leaflet: {display:?}");

    // …but a click composes the PARSER's word, derived the same way the
    // command panel's WHAT column derives it.
    let words = inventory_click_words(None, &[], session.introspect(), None);
    assert_eq!(words.len(), display.len(), "one click word per drawn row");
    let idx = words
        .iter()
        .position(|w| w.eq_ignore_ascii_case("leaflet"))
        .unwrap_or_else(|| panic!("leaflet not among the click words: {words:?}"));

    let mut state = AppState::default();
    state.inventory_click_words = words;
    state.input.set("examine ".to_string(), true);
    let mut mapper = Mapper::default();
    apply_action(Action::InventoryClickRow(idx), &mut state, &mut mapper);

    assert_eq!(state.input.value, "examine leaflet");
}

// ── SQ-1237 Part 3 audit: does the inventory panel feed the same way the
// command panel does across engines? ────────────────────────────────────────
//
// `inventory_items` reads the live OBJECT TREE (`Introspect::contents`), never
// the story's dictionary (`Engine::story_vocabulary`, the thing SQ-1231 fixed
// for Glulx's Unicode/I6 dictionary shapes) — so a Glulx dictionary shape is
// not the question here. The real finding, discovered driving this case: on
// Glulx, `Engine::introspect` is the trait's own DEFAULT, always `None` —
// `GlulxSession` never overrides it (`glulx_session.rs`'s own
// `introspect_is_none` unit test says so outright: "Glulx introspection is
// SP4", i.e. planned, not built). That is engine-wide, not inventory-specific:
// the command panel's *carried*/*here* columns read the exact same
// `Introspect` trait through the exact same absence, so the inventory panel is
// fed EXACTLY as well (or as poorly) as the command panel is on this engine —
// there is no parity GAP between the two panels here, only a gap between
// engines, and it is not a "small, mechanical fix" (implementing Glulx object
// introspection is its own feature, already tracked as SP4).
//
// Both panels fall back to `state.inventory_fallback` on Glulx: a transcript
// scrape of the reply to a literal `i`/`inv`/`inventory` command
// (`turn.rs`'s `parse_inventory_output`), which is why it stays empty here
// until the player types one — and, on THIS game, stays empty even then: CM
// answers `i` with its own narrative prose rather than the standard Inform
// library reply the parser looks for, which this case also pins.
#[test]
#[ignore = "slow (~2.7s): runs Counterfeit Monkey's intro through the Glulx VM; run with --include-ignored"]
fn counterfeit_monkey_has_no_live_introspection_and_falls_back_to_the_transcript_scrape() {
    use app::glulx_session::GlulxSession;
    use blorb::Blorb;

    let path = fixture_path("CounterfeitMonkey-11.gblorb");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let blorb = Blorb::parse(bytes).expect("parse gblorb");
    let image = blorb.executable().expect("exec chunk").1.to_vec();
    let mut session =
        GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("boot CM");
    // CM's opening asks "Can you hear me?" three times before the game itself
    // starts, then reads a bare keypress (`glk_select` on a char event, which
    // `submit_key` answers and `submit` cannot) and finally its own printed
    // instructions: "type LOOK and press return."
    for cmd in ["yes", "yes", "yes"] {
        session.submit(cmd);
    }
    session.submit_key(app::engine::KeyInput::Enter);
    session.submit("look");
    let _ = session.take_transcript();

    // The real finding: a real, validated Inform 7/Glulx object table still
    // answers no introspection at all.
    assert!(
        session.introspect().is_none(),
        "Glulx introspection is SP4 (not built) — this pins that fact so the day it \
         changes, this whole file's Glulx story updates with it rather than silently \
         going stale",
    );

    // Both panels' fallback path: nothing carried is known without asking.
    assert!(
        inventory_items(None, &[], session.introspect()).is_empty(),
        "no fallback list has been populated yet — the player has typed nothing",
    );

    // Ask, the same way `turn.rs` recognises an inventory command: `i`. A
    // SECOND finding, not the one this case set out to check: CM answers with
    // its own narrative prose ("We are equipped with your R-remover...")
    // rather than the standard "You are carrying:" Inform library reply, and
    // `parse_inventory_output` looks for exactly that header — so the
    // fallback stays empty on THIS game even after asking. Asserted here
    // rather than silently accepted, so a future change to either side
    // (the parser, or CM's own reply) is caught instead of drifting unnoticed.
    let reply = session.submit("i").transcript;
    let fallback = app::inventory::parse_inventory_output(&reply);
    let items = inventory_items(None, &fallback, session.introspect());
    assert!(
        items.is_empty(),
        "CM's custom \"i\" reply does not match parse_inventory_output's \"carrying\" \
         header heuristic — if this now finds items, the heuristic (or CM's reply) \
         changed and the SQ-1237 audit's finding needs re-checking: {reply:?}",
    );

    // Nothing to verify for a click here either (SQ-1244 added hit-rects and
    // composition since this audit was written): an empty click-word list
    // (`inventory_click_words`, mirroring `items` being empty above) composes
    // nothing, exactly as clicking a row nothing was drawn for cannot happen —
    // there is no row to click on a Glulx game with no introspection.
}
