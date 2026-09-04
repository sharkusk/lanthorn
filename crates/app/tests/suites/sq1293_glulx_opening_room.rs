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

    // ── …and the question was asked BEHIND the player ───────────────────────
    // `silent_look` restores gvm's state, and the BACKEND is a second copy of the
    // game's state that the app renders from. Restoring only the VM leaves the
    // question's prose sitting in the buffer's log with the drain pointer moved
    // past it, so the player is owed the room description on the NEXT turn and
    // reads it twice. Look for the description rather than the heading: the
    // heading is the thing we asked for.
    const ONLY_IN_THE_DESCRIPTION: &str = "peeling yellow paint";
    for turn in &prologue_turns {
        assert!(
            !turn.transcript.contains(ONLY_IN_THE_DESCRIPTION),
            "the prologue prints no room description; this is the silent look's, \
             leaking into the player's transcript: {:?}",
            turn.transcript
        );
    }

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
    assert!(
        !north.transcript.contains(ONLY_IN_THE_DESCRIPTION),
        "the turn after the question is where a half-restored backend hands the \
         player the answer they never asked for: {:?}",
        north.transcript
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

/// SQ-1300: the Back Alley and Sigil Street must keep reading "1" and "2" — their small per-map
/// ordinals — in `/export-map`'s dump even after the Glulx room lock lands and re-keys them from
/// name-derived ids onto their real object addresses (`app::turn`'s `take_room_remap` /
/// `Mapper::rekey_room`, exercised here the same way `turn.rs` drives it every real turn). The
/// ordinal is a property of the room NODE, so a re-key must carry it along unchanged — this is
/// the real-game half of the coverage `roomid::tests::room_label_no_survives_a_rekey` and
/// `graph::tests::rekey_room_carries_the_ordinal_with_it` give it synthetically.
#[test]
fn opening_rooms_keep_their_ordinals_across_the_lock_rekey() {
    let Some((mut s, prologue_turns)) = prologue() else { return };

    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    for (step, r) in PROLOGUE.iter().zip(&prologue_turns) {
        apply_turn(&mut mapper, step.unwrap_or("<key>"), r, &mut death);
    }

    // Drive the same rekey the app's own turn loop performs every turn (`app::turn`), so a
    // room the lock renames mid-walk lands on the same node afterward rather than a duplicate.
    // Returns how many rooms it actually re-keyed, for the non-vacuity check below.
    let rekey_after = |s: &mut GlulxSession, mapper: &mut Mapper| -> usize {
        let mut done = 0;
        for (name, addr) in s.take_room_remap() {
            let old_id = app::roomid::synthetic_room_id(&name);
            let new_id = app::roomid::glulx_room_id(addr);
            if mapper.rekey_room(old_id, new_id) {
                done += 1;
            }
        }
        done
    };

    // Walk back and forth between the Back Alley and Sigil Street enough times to give the
    // room-lock learner the repeated room-change evidence it needs to resolve (SQ-1286).
    const WALK: [&str; 8] = ["n", "s", "n", "s", "n", "s", "n", "s"];
    let mut rekeys = 0;
    for cmd in WALK {
        let r = s.submit(cmd);
        rekeys += rekey_after(&mut s, &mut mapper);
        apply_turn(&mut mapper, cmd, &r, &mut death);
    }

    assert!(
        s.locked_room_global().is_some(),
        "non-vacuity: the walk must resolve the room lock, or this test never reaches a re-key"
    );
    assert!(
        rekeys > 0,
        "non-vacuity: the lock resolving is not enough — the walk must actually trigger at \
         least one re-key, or this test never exercises what SQ-1300 is about"
    );

    let names: Vec<String> = mapper.graph.rooms().map(|r| r.label().to_string()).collect();
    assert_eq!(
        names.iter().filter(|n| n.as_str() == BACK_ALLEY).count(),
        1,
        "the re-key must not leave a duplicate Back Alley beside the renamed node: {names:?}"
    );

    let dump = app::map_dump::render_dump(&mapper.graph, &app::symbols::SymbolSet::default());
    let alley_line = dump
        .lines()
        .find(|l| l.starts_with("ROOM ") && l.contains(&format!("{BACK_ALLEY:?}")))
        .unwrap_or_else(|| panic!("Back Alley's ROOM line: {dump}"));
    assert!(
        alley_line.starts_with("ROOM #1 "),
        "Back Alley is the first room discovered and must still read #1 after the lock's \
         re-key: {alley_line}"
    );

    let sigil_line = dump
        .lines()
        .find(|l| l.starts_with("ROOM ") && l.contains("\"Sigil Street\""))
        .unwrap_or_else(|| panic!("Sigil Street's ROOM line: {dump}"));
    assert!(
        sigil_line.starts_with("ROOM #2 "),
        "Sigil Street is the second room discovered and must still read #2 after the lock's \
         re-key: {sigil_line}"
    );
}
