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
