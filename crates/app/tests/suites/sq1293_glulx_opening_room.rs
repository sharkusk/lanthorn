//! SQ-1293: the map must know the room the player starts in.
//!
//! # The report
//!
//! *"At the start of the game, lanthorn doesn't realize I'm in Back Alley. I have to
//! navigate north and then south to get Back Alley to appear on the map."*
//!
//! The player's `/export-map` dump bears it out from the other side: the very first
//! edge the 0.4.2 session recorded was `Sigil Street → S → Back Alley`, the *return*
//! leg, with no `Back Alley → N → Sigil Street` before it. The opening room only
//! exists on the map once you have walked out of it and back.
//!
//! # Why
//!
//! Glulx has no object tree, so `GlulxSession` recovers the room from the `Subheader`
//! room HEADING the story prints (`glk_backend`'s `StoryScan`), and Counterfeit Monkey
//! **prints no heading at all before the first command**. Its prologue is a
//! conversation — "Can you hear me?", "Do you remember our name?", a keypress, the
//! banner — and then it tells the player to type LOOK. So the boot path at
//! `glulx_session.rs` (`GlulxSession::new_in`, the `take_room_heading` after
//! `refresh_screen`) has nothing to resolve and leaves `last_room` at `None`.
//!
//! **This is not a 0.4.3 regression.** The 0.4.2 dump shows exactly the same shape,
//! and neither SQ-1285 (`line_rest_disqualifies`) nor SQ-1286 (the object-table value
//! filter) can reach a turn where no `Subheader` run was written at all.
//!
//! The story's own model cannot supply the name either, and both halves were measured
//! on this fixture: `gvm::objects::ParseNames::find_player` **refuses Counterfeit
//! Monkey outright** (documented in that function — not one of its 2,494 objects
//! answers to `yourself`/`myself`/`self`), so the player's containing room cannot be
//! walked to; and `ParseNames::short_name` of the room the lock points at is the EMPTY
//! STRING, because Inform 7 objects carry no hardware short name. So even a session
//! that boots already locked (the per-game `room-global` sidecar, `RoomLock::locked_at`)
//! knows the room's ADDRESS at boot and still cannot put a name on it.
//!
//! What does work, measured: a plain `look` names the room immediately. So the fix is
//! to ask — `GlulxSession::silent_look` snapshots the VM, types `look` into it, reads
//! the heading off the backend, throws the answer away and restores. It is spent only
//! where there is genuinely no name to be had; see that function for why the shadow in
//! [`app::probe`] could not serve this seam.
//!
//! Note where the first ask can land: `look` is only a COMMAND once the parser is
//! waiting for one, and Counterfeit Monkey's boot is mid-conversation ("Can you hear
//! me?"), so the question is asked on the turn that hands the player the command prompt
//! — the end of the prologue, which is the moment the report calls "the start of the
//! game".
//!
//! # The fixture and the route
//!
//! `stories/CounterfeitMonkey-11.gblorb` — release 11 / serial 230220 / Inform 7 build
//! 6M62. Gitignored, so this skips vacuously without it.
//!
//! Three inputs from a cold boot, and no more, because the report is about the START:
//! `y` (the consent that begins play), `andra` (the name question), then a KEYPRESS —
//! not a command — which is the prologue's `custom-wait for any key` before the banner.
//! The player is standing in the Back Alley for all three.

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::{apply_turn, DeathWatch, InputKind, TurnResult};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

const STORY: &str = "CounterfeitMonkey-11.gblorb";

/// The room Counterfeit Monkey opens in.
const BACK_ALLEY: &str = "Back Alley";

/// The prologue, and nothing else. `None` is the keypress before the banner.
const PROLOGUE: &[Option<&str>] = &[Some("y"), Some("andra"), None];

fn boot() -> Option<GlulxSession> {
    let path = fixture_path(STORY);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let pict_blorb = blorb::Blorb::parse(bytes.clone()).ok();
    let app::hints::LoadedStory::Glulx(image) =
        app::hints::extract_story(bytes).expect("CounterfeitMonkey-11.gblorb is a readable container")
    else {
        panic!("{STORY} is a Glulx story");
    };
    let mut s = GlulxSession::new(image, 80, 30, true, false, false, (8, 16), pict_blorb, &[])
        .expect("Counterfeit Monkey boots");
    let _ = s.take_transcript();
    Some(s)
}

/// Play [`PROLOGUE`], returning the session and each turn's result.
fn prologue() -> Option<(GlulxSession, Vec<TurnResult>)> {
    let mut s = boot()?;
    let mut turns = Vec::new();
    for (i, step) in PROLOGUE.iter().enumerate() {
        let r = match step {
            Some(cmd) => {
                assert_eq!(s.pending_input(), InputKind::Line, "prologue step {i} ({cmd:?}) wants a line");
                s.submit(cmd)
            }
            None => {
                assert_eq!(s.pending_input(), InputKind::Char, "prologue step {i} wants the keypress");
                s.submit_key(KeyInput::Enter).expect("Glulx takes keys")
            }
        };
        turns.push(r);
    }
    Some((s, turns))
}


/// One route, every assertion: play the prologue, ask the map where we are, then walk
/// out of the room and back and check the map put us where the story does.
#[test]
fn the_opening_room_is_on_the_map_before_the_player_leaves_it() {
    let Some((mut s, prologue_turns)) = prologue() else { return };

    // ── Non-vacuity: the story really does print no heading in its prologue ──
    // This is what makes the assertion below about the OPENING ROOM rather than
    // about a heading we failed to read.
    assert!(
        prologue_turns.iter().all(|t| t.location.is_none() || t.location.as_ref().unwrap().name == BACK_ALLEY),
        "Counterfeit Monkey prints no room heading during its prologue; if another room \
         appears here the fixture or the route has changed"
    );

    // ── The report: the player is in the Back Alley from the first moment ────
    let here = s
        .current_location()
        .expect("the player is standing in a room; the map must know which one");
    assert_eq!(here.name, BACK_ALLEY);

    // ── The map's half: the FIRST move out records its own edge ──────────────
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    for (step, r) in PROLOGUE.iter().zip(&prologue_turns) {
        apply_turn(&mut mapper, step.unwrap_or("<key>"), r, &mut death);
    }
    let north = s.submit("n");
    assert_eq!(
        north.location.as_ref().map(|l| l.name.as_str()),
        Some("Sigil Street"),
        "north from the Back Alley is Sigil Street"
    );
    apply_turn(&mut mapper, "n", &north, &mut death);

    // ── And the strongest non-vacuity there is: walking back in lands on the
    //    SAME node the boot recorded, named by the story itself this time. The
    //    reported dump's whole complaint is that these were two different things.
    let south = s.submit("s");
    let back = south.location.as_ref().expect("a room after walking back south");
    assert_eq!(back.name, BACK_ALLEY, "south from Sigil Street is the Back Alley");
    assert_eq!(
        back.number, here.number,
        "the room the map recorded before the first command is the room the player \
         walks back into, not a second node beside it"
    );
    apply_turn(&mut mapper, "s", &south, &mut death);

    let names: Vec<String> = mapper.graph.rooms().map(|r| r.label().to_string()).collect();
    assert_eq!(
        names.iter().filter(|n| n.as_str() == BACK_ALLEY).count(),
        1,
        "one Back Alley: {names:?}"
    );
    let alley = mapper
        .graph
        .rooms()
        .find(|r| r.label() == BACK_ALLEY)
        .map(|r| r.id)
        .expect("the Back Alley is mapped");
    let outbound: Vec<_> = mapper
        .graph
        .connections()
        .iter()
        .filter(|c| c.origin == alley && c.dir == mapper::direction::Direction::N)
        .collect();
    assert!(
        !outbound.is_empty(),
        "the first move out of the opening room must record its own edge, not only the \
         return leg: {:?}",
        mapper.graph.connections()
    );
}
