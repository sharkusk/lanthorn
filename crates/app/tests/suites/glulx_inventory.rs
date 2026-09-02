//! SQ-1241: a Glulx game's inventory, read from its own object tree.
//!
//! The reported defect: City of Secrets shows nothing in the inventory dock or
//! in the command panel's *carried* column. Both panels read one seam —
//! `Introspect::player_object()` and `Introspect::contents(player)` — and on
//! Glulx that seam answered `None` outright, so both fell back to scraping the
//! reply to an `i` command. Any game answering `i` in its own prose defeats the
//! scrape, and City of Secrets' does.
//!
//! Three compilers, deliberately, because the object layout is Inform's and the
//! avatar rule is the *library's*:
//!
//! | fixture | compiler | why it is here |
//! |---|---|---|
//! | `CoS.blb` | Inform 6.21 | the reported game, and **not** fingerprinted by `gvm::veneer` — it must work from the image alone |
//! | `King_of_Shreds_and_Patches.gblorb` | Inform 6.31 | fingerprinted, and starts the game carrying things |
//! | `CounterfeitMonkey-11.gblorb` | Inform 7 6M62 | registers its own acceleration; the story that must be REFUSED |
//!
//! `stories/` is gitignored, so every case here skips vacuously without its
//! fixture.

use std::path::PathBuf;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::render::transcript::inventory_items;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The Glulx image inside a Blorb, or a bare `.ulx` passed through. `None`
/// when the gitignored fixture is absent — every case skips on it.
fn glulx_image(name: &str) -> Option<Vec<u8>> {
    let path = stories_dir().join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    if !blorb::Blorb::is_blorb(&bytes) {
        return Some(bytes);
    }
    let b = blorb::Blorb::parse(bytes).ok()?;
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Some(data.to_vec()),
        _ => None,
    }
}

fn boot(name: &str) -> Option<GlulxSession> {
    let image = glulx_image(name)?;
    let mut s = GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[])
        .expect("GlulxSession::new");
    // Past any "press a key" splash to the first command prompt.
    for _ in 0..6 {
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    let _ = s.take_transcript();
    Some(s)
}

fn carried(s: &GlulxSession) -> Vec<String> {
    inventory_items(None, &[], s.introspect())
}

fn lower(items: &[String]) -> String {
    items.join(" | ").to_lowercase()
}

/// A dump of what the seam answers, for a failure message worth reading.
fn describe(s: &GlulxSession) -> String {
    format!(
        "location={:?} player={:?} carried={:?}",
        s.current_location().map(|l| l.name),
        s.introspect().and_then(|i| i.player_object()),
        carried(s)
    )
}

// ── City of Secrets (Inform 6.21) — the reported game ────────────────────────

/// The defect, at the seam the panels read.
///
/// City of Secrets is Inform 6.21, which `gvm::veneer` explicitly refuses to
/// fingerprint (different codegen), so nothing about this can come from a
/// matched veneer: the object list, its stride and the avatar are all derived
/// from the image.
///
/// **And it is the avatar-discrimination case.** Two situated objects here
/// answer to avatar names: Inform 6's own `selfobj` — printed `(self object)`,
/// standing in `City Train Station`, and the real player — and a decoy printed
/// `yourself`, with the richer word list `me/i/name/myself`, parked in a
/// `(ConceptObjs)` bag. Neither name outranks the other (Anchorhead is the same
/// shape with the same two names on the Z-machine), so only the ROOM can say —
/// and on turn one the room-lock has not resolved yet, so the room comes from
/// matching the printed heading against object short names. Picking the decoy
/// gives an empty list, which is why asserting the three real items is the
/// check: it cannot pass on the wrong object.
#[test]
fn city_of_secrets_reads_its_carried_items_from_the_object_tree() {
    let Some(mut s) = boot("CoS.blb") else { return };
    eprintln!("CoS after boot: {}", describe(&s));

    let player = s
        .introspect()
        .expect("City of Secrets has a readable Inform object list")
        .player_object()
        .expect("its avatar is identifiable");

    // Drive a few turns so the tree read below is the RUNNING game's rather
    // than the image's — the panels ask between turns, never at boot.
    for cmd in ["", "look"] {
        s.submit(cmd);
    }
    eprintln!("CoS after two turns: {}", describe(&s));

    let items = carried(&s);
    let l = lower(&items);
    assert!(
        l.contains("travel papers") && l.contains("watch") && l.contains("suitcase"),
        "you arrive at the City Train Station carrying your travel papers, your watch and \
         your suitcase: {}",
        describe(&s)
    );
    // The same answer through the LOCKED path the turn loop uses, so the dock
    // and the band cannot disagree about who the player is.
    assert_eq!(
        inventory_items(Some(player), &[], s.introspect()),
        items,
        "the locked avatar and the live lookup are the same object"
    );
    // And it is the TREE, not the scrape: no `i` reply was ever handed to
    // `parse_inventory_output`, and the fallback list passed above is empty.
}

// ── King of Shreds and Patches (Inform 6.31) ─────────────────────────────────

/// Robert Fletcher starts the game with a letter and a key in his hands, so a
/// boot-time read is enough — no command has to succeed for this to be a real
/// check of the tree.
#[test]
fn king_of_shreds_and_patches_reads_the_items_it_starts_you_with() {
    let Some(s) = boot("King_of_Shreds_and_Patches.gblorb") else { return };
    eprintln!("KoSaP after boot: {}", describe(&s));
    let items = carried(&s);
    let l = lower(&items);
    assert!(
        l.contains("letter") && l.contains("key"),
        "the game opens with John Croft's letter and the printworks key: {}",
        describe(&s)
    );
}

// ── Counterfeit Monkey (Inform 7 6M62) — the refusal ─────────────────────────

/// **The story that must be refused**, and the reason `find_player` has no
/// "first plausible candidate" fallback.
///
/// Counterfeit Monkey's object list reads perfectly — 2,494 objects, all 1,916
/// containment links consistent — but nothing in it identifies the avatar. Not
/// one of those objects answers to `yourself`, `myself` or `self`, and none
/// carries an avatar-ish printed name, because Inform 7 objects have no
/// hardware short name at all. A conditional or multi-word `Understand`
/// compiles to a `parse_name` ROUTINE rather than to the static `name` array,
/// and machine code is not enumerable from the image in any Inform version.
///
/// The only objects whose word arrays hold anything avatar-ish are conversation
/// quips ("what he thinks of you", "what he kens about me"), parked together in
/// a topics container. An earlier draft of the rule answered with the first of
/// those and would have told the player they were carrying its contents.
///
/// So the avatar is `None` here, the panels keep the transcript scrape, and CM
/// keeps answering `i` in its own prose that the scrape cannot parse. That is
/// the honest outcome, and this case pins it: the day CM's avatar becomes
/// identifiable, this fails and says so rather than going quietly stale.
#[test]
fn counterfeit_monkey_refuses_an_avatar_it_cannot_identify() {
    let Some(mut s) = boot("CounterfeitMonkey-11.gblorb") else { return };
    // CM asks "Can you hear me?" three times, then reads a bare keypress, then
    // prints its own instructions.
    for cmd in ["yes", "yes", "yes"] {
        s.submit(cmd);
    }
    s.submit_key(KeyInput::Enter);
    s.submit("look");
    let _ = s.take_transcript();
    eprintln!("CM after the intro: {}", describe(&s));

    let intro = s.introspect().expect("CM's object list reads perfectly — that is not the problem");
    assert!(
        intro.player_object().is_none(),
        "CM's avatar is not identifiable from the image, and a guess would be worse than \
         nothing: {}",
        describe(&s)
    );
    assert!(carried(&s).is_empty(), "so the carried column is empty rather than wrong");

    // The fallback is unchanged and still cannot read CM's custom `i` reply —
    // pinned so a change to either side is caught rather than drifting.
    let reply = s.submit("i").transcript;
    let fallback = app::inventory::parse_inventory_output(&reply);
    assert!(
        inventory_items(None, &fallback, s.introspect()).is_empty(),
        "CM answers `i` in its own prose, which `parse_inventory_output`'s \"carrying\" \
         header heuristic does not match: {reply:?}"
    );
}

// ── The panels, driven the way the app drives them (SQ-1241, reopened) ───────
//
// The three cases above ask the SESSION. They passed while the app still drew
// two empty panels, because the session they booted is not the one the app
// boots and, more to the point, they never asked at the moment the player
// first sees the screen — they asked after a `look`, and `look` is what
// repaired the defect.
//
// What the app actually does at the first prompt is this: one keypress past
// "PRESS ANY KEY TO BEGIN", then every loop tick calls
// `render::transcript::inventory_items` (the inventory panel, `main.rs`),
// `render::inventory_dock::refresh_inventory_click_words` (its click words) and
// `render::command_band::refresh_objects` (the command panel's *carried*
// column). All three read `Introspect::player_object()`, and on City of
// Secrets that answered `None` for the first four turns of play.
//
// The cause was not in the object walk at all: City of Secrets is a GWindows
// game, and it prints its whole prologue — title, the `Subheader` heading
// "City Train Station", the room description and the read prompt — into a
// SECOND buffer window it opens mid-turn, while `AppGlk::primary` is still the
// splash window opened first. The room-heading detector was fed only for the
// window that was primary AT WRITE TIME, so the opening heading was never seen,
// `current_location()` stayed `None`, and `find_player`'s room-decides rule had
// no room to decide with — City of Secrets ships two situated avatar
// candidates, `(self object)` in City Train Station and a decoy `yourself` in
// `(ConceptObjs)`, and without the room neither outranks the other, so the
// avatar was refused. King of Shreds and Patches has ONE situated candidate and
// never needs the room, which is exactly why it worked throughout.

use app::render::command_band::{default_quick, default_verbs, refresh_objects, COL_CARRIED};
use app::render::inventory_dock::{
    draw_inventory_dock, inventory_dock_target_height, refresh_inventory_click_words,
    InventoryDockHits,
};
use app::render::transcript::inventory_click_words;
use app::state::{AppState, CommandBandState};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Boot a Glulx story the way `startup.rs` does: a writable per-game store,
/// graphics and sound on, a real char cell, a fixed PRNG seed. `None` when the
/// gitignored fixture is absent.
fn boot_like_the_app(name: &str) -> Option<GlulxSession> {
    let image = glulx_image(name)?;
    GlulxSession::new_in(
        app::scratch_dir("sq1241-panels"),
        image,
        80,
        24,
        true,
        true,
        true,
        false,
        (8, 16),
        None,
        &[],
        [[(None, None); 11]; 2],
        false,
        Some(1),
    )
    .ok()
}

/// Past the "press any key" splash to the story's FIRST command prompt, and no
/// further — no `look`, no move. This is the frame the player is looking at
/// when they first see the panels, and the frame the defect was reported on.
fn to_first_prompt(s: &mut GlulxSession) {
    for _ in 0..6 {
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
}

/// What the two panels draw, read through the very calls the loop tick makes.
///
/// Returns `(inventory panel rows, the text actually painted into the dock,
/// the command panel's *carried* column)`.
fn panels(s: &GlulxSession) -> (Vec<String>, String, Vec<String>) {
    let mut state = AppState::default();
    // Both panels open at once, which the app never does (they are mutually
    // exclusive) — but each reads its own source, and one case asserting both
    // cannot let the two drift apart.
    app::input::open_inventory_panel(&mut state, true);
    state.overlays.command_band = Some(CommandBandState::new(default_verbs(), default_quick()));

    // `main.rs`'s own line, argument for argument.
    let rows = inventory_items(state.player_obj, &state.inventory_fallback, s.introspect());
    refresh_inventory_click_words(&mut state, s);
    refresh_objects(&mut state, s);

    // …and the rows really reach the screen, not just the list.
    let area = Rect::new(0, 0, 40, inventory_dock_target_height(rows.len(), 40, 100));
    let mut buf = Buffer::empty(area);
    draw_inventory_dock(
        &rows,
        area,
        &state.colors,
        false,
        &mut buf,
        &mut InventoryDockHits::default(),
    );
    let painted: String = buf.content().iter().map(|c| c.symbol().to_owned()).collect();

    let carried = state.overlays.command_band.as_ref().unwrap().items(COL_CARRIED);
    // SQ-1244's click words track the rows one-for-one; a panel that draws rows
    // it cannot compose a word for is half-broken.
    assert_eq!(
        state.inventory_click_words.len(),
        rows.len(),
        "one clickable word per drawn row: {:?} vs {rows:?}",
        state.inventory_click_words
    );
    assert_eq!(
        state.inventory_click_words,
        inventory_click_words(state.player_obj, &state.inventory_fallback, s.introspect(), None),
        "the dock's stored words are the ones the panel would derive now"
    );
    (rows, painted, carried)
}

/// **The reported defect, at the layer that was broken.**
///
/// One keypress past the splash — the very first command prompt — and both
/// panels must already name what you are carrying. Falsify by restoring
/// `put_text_attr`'s `if Some(win) == self.primary` guard around the heading
/// scan: `current_location()` goes back to `None` here, `player_object()` with
/// it, and both panels draw empty until the player thinks to type `look`.
#[test]
fn city_of_secrets_panels_are_filled_at_the_first_prompt() {
    let Some(mut s) = boot_like_the_app("CoS.blb") else { return };
    to_first_prompt(&mut s);

    // Non-vacuity: the story really did reach its first command prompt, in the
    // room it opens in. A capture taken before this point is of a screen the
    // player never plays from.
    assert_eq!(
        s.pending_input(),
        app::session::InputKind::Line,
        "one keypress past PRESS ANY KEY TO BEGIN leaves the parser reading a command"
    );

    // The reported symptom first, so a regression fails saying what the player
    // saw…
    let (rows, painted, carried) = panels(&s);
    let l = lower(&rows);
    assert!(
        l.contains("travel papers") && l.contains("watch") && l.contains("suitcase"),
        "the inventory panel names what you stepped off the train with: {rows:?}"
    );
    // …and the cause immediately after, so it also says why.
    assert_eq!(
        s.current_location().map(|l| l.name).as_deref(),
        Some("City Train Station"),
        "the prologue's own room heading — printed into the second buffer window \
         GWindows opens mid-turn, which is the whole of this defect"
    );
    for word in ["papers", "watch", "suitcase"] {
        assert!(painted.contains(word), "the dock paints {word:?}: {painted:?}");
    }
    let c = lower(&carried);
    assert!(
        c.contains("watch") && c.contains("suitcase"),
        "the command panel's carried column too: {carried:?}"
    );
}

/// The engine's live answer and the turn loop's LOCKED one are the same object,
/// so the two panels cannot disagree about who the player is.
///
/// `turn.rs` locks `AppState::player_obj` on the first turn that reports a
/// location and every panel prefers the lock; this pins that the lock taken at
/// the first prompt is the avatar and not the `(ConceptObjs)` decoy.
#[test]
fn city_of_secrets_locked_avatar_matches_the_live_one() {
    let Some(mut s) = boot_like_the_app("CoS.blb") else { return };
    to_first_prompt(&mut s);
    let locked = s
        .introspect()
        .expect("City of Secrets has a readable Inform object list")
        .player_object()
        .expect("its avatar is identifiable at the first prompt");
    assert_eq!(
        inventory_items(Some(locked), &[], s.introspect()),
        inventory_items(None, &[], s.introspect()),
        "the locked avatar and the live lookup are the same object"
    );
}

/// The engine that always worked, held to the same standard: King of Shreds and
/// Patches is found by avatar words alone, so its panels were filled before
/// this change and must still be.
#[test]
fn king_of_shreds_panels_are_filled_at_the_first_prompt() {
    let Some(mut s) = boot_like_the_app("King_of_Shreds_and_Patches.gblorb") else { return };
    to_first_prompt(&mut s);
    let (rows, painted, carried) = panels(&s);
    let l = lower(&rows);
    assert!(
        l.contains("letter") && l.contains("key"),
        "John Croft's letter and the printworks key: {rows:?}"
    );
    assert!(painted.contains("letter"), "the dock paints it: {painted:?}");
    assert!(!carried.is_empty(), "and the carried column is filled: {carried:?}");
}

/// The refusal, at the panel layer: Counterfeit Monkey's avatar is not
/// identifiable from the image, so the panels stay EMPTY rather than showing a
/// conversation quip's contents. A change that made `find_player` guess would
/// pass every case above and fail this one.
#[test]
fn counterfeit_monkey_panels_stay_empty_rather_than_wrong() {
    let Some(mut s) = boot_like_the_app("CounterfeitMonkey-11.gblorb") else { return };
    for cmd in ["yes", "yes", "yes"] {
        s.submit(cmd);
    }
    s.submit_key(KeyInput::Enter);
    s.submit("look");
    let _ = s.take_transcript();
    // Non-vacuity: it is at a command prompt with a readable object list — the
    // emptiness below is a refusal, not a story that never booted.
    assert_eq!(s.pending_input(), app::session::InputKind::Line);
    assert!(s.introspect().is_some(), "CM's object list reads perfectly");

    let (rows, painted, carried) = panels(&s);
    assert!(rows.is_empty(), "no avatar, no rows: {rows:?}");
    assert!(painted.contains("(empty)"), "the dock says so plainly: {painted:?}");
    assert!(carried.is_empty(), "and the carried column is empty: {carried:?}");
}
