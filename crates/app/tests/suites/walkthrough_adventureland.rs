//! SQ-1600: a headless walkthrough-regression harness for **Adventureland**
//! (`stories/adv01.dat`), the first title in Adventure International's
//! classic Scott Adams series -- the dedicated "Scott Adams" format bucket
//! the quest names. Mirrors the SHAPE of `walkthroughs.rs`'s Phase 1
//! (Photopia, Z-code): boots headless via the library session API (no
//! terminal), drives a committed, hand-verified command script
//! (`tests/fixtures/walkthroughs/adventureland.txt`) to the game's own real
//! winning ending, exercises a host Save State/Restore State round trip
//! partway through, checks the mapper and hints machinery don't error while
//! being driven, and pins a normalized reference transcript
//! (`tests/fixtures/walkthroughs/adventureland.transcript`) that a future
//! regression must keep matching.
//!
//! **The concrete API differs from Photopia's Z-machine shape in several
//! ways worth recording:**
//!
//! - The session type is [`app::scott_session::ScottSession`], not
//!   `GameSession` -- adapts `crates/scott`'s headless `scott::Vm` (a
//!   ScottFree-format database) to the same engine-neutral [`Engine`] trait.
//!   `ScottSession::new(bytes, None)` is the loader for a plain `.dat` with
//!   no companion Blorb graphics container (`scott_mapper.rs`'s own boot
//!   idiom) -- `adv01.dat` is a raw Scott Adams database, not a
//!   `.z*`/`.gblorb` file, so there is no `GameSession::new_with_trace`
//!   analogue to reach for.
//! - Scott is **line-only** (no `read_char`) outside a US S.A.G.A.
//!   picture-show sequence (SQ-1487) -- Adventureland is a plain reference
//!   database with no picture files, so `pending_input()` never answers
//!   `Char` here, but `submit_command` below still routes through it for
//!   the same reason the Photopia harness does: staying engine-neutral
//!   costs nothing and matches how the app itself drives any session.
//! - **No engine-provided score accessor reaches an integration test.**
//!   `scott::Vm::treasures_stored()` is `pub fn` on the VM, but
//!   `ScottSession`'s `vm` field is private and exposes no score/treasure
//!   accessor of its own (only `item_loc`, for the binary crate's restore
//!   tests). Score is therefore read the way a PLAYER reads it: Scott's own
//!   `SCORE` verb prints "I've stored N  treasures...", and the real
//!   ending prints "Well done." immediately before "The game is now over."
//!   and `has_quit()` goes true -- those exact printed strings are the
//!   acceptance here, the same shape Photopia's silent-ending acceptance
//!   uses for a fact the engine does not expose structurally.
//! - **Scott's PRNG is deterministically seeded at boot, no OS entropy.**
//!   `scott::Vm::DEFAULT_RNG_SEED` (`0x1234_5678`) is a fixed constant, and
//!   `ScottSession::new`/`new_with_trace` pass `None` for `random_seed`,
//!   which falls through to that default -- confirmed empirically, not
//!   merely by reading the constant: two independent full playthroughs of
//!   the script below (an early throwaway run and the final committed run)
//!   produced byte-identical transcripts. Adventureland uses this PRNG for
//!   several real per-turn occurrence rolls this walkthrough had to route
//!   around (a chigger-bite chance in the swamp, a mud-drying chance, a
//!   bees-suffocating chance, and the fish's own escape/death chances at
//!   the lake) -- unlike the `zvm` fixed-seed finding this project already
//!   has on record, this is `crates/scott`'s own, entirely separate RNG.
//! - Scott has **no in-game restart verb** (`adv01.dat`'s vocabulary has no
//!   `RESTART`/`RES` entry) and no analogue to the Z-machine's `@restart`
//!   opcode; the only restart path is the HOST's `app::host::reset::reset_game`
//!   (engine-generic, rebuilds a fresh `ScottSession` and downcasts it in),
//!   which needs `AppState`/`game_dir`/`ResetOptions` scaffolding no
//!   existing Scott suite exercises and `restart_reboots_in_place.rs` never
//!   reaches for Scott either (it drives the Z-machine's own in-game
//!   `restart` LIBRARY VERB, which Scott has no equivalent of at all). This
//!   harness deliberately omits a restart-reboot check for that reason.

use std::path::Path;

use app::archive::{self, Meta, SaveTrigger};
use app::engine::Engine;
use app::hints;
use app::host::hints::{available, open, HintAvailability};
use app::config::Config;
use app::ifid::compute_ifid;
use app::scott_session::ScottSession;
use app::session::{apply_item_observations, apply_turn, DeathWatch, InputKind, TurnResult};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// Which script command the mid-script persistence check fires after: the
/// 31st (`drop keys`) -- back in the treasure room having just deposited the
/// Pot of Rubies and the skeleton keys, holding nothing more than the unlit
/// lamp and a full water bottle. Verified empirically: no pending I/O, no
/// picture-show sequence, a clean moment to snapshot and perturb.
const CHECKPOINT_AFTER_COMMAND: usize = 31;

/// A generous cap over the committed script's ~150 real commands.
const MAX_TURNS: usize = 400;

fn boot_adventureland() -> Option<(Vec<u8>, ScottSession)> {
    let path = fixture_path("adv01.dat");
    let bytes = std::fs::read(&path).ok()?;
    let session = ScottSession::new(bytes.clone(), None).expect("Adventureland should boot without error");
    Some((bytes, session))
}

fn read_script() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("tests/fixtures/walkthroughs/adventureland.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("committed walkthrough script must be readable at {}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Submit one script command, routing it to a keypress when the game is at a
/// single-key (Char) prompt (never happens for this reference-format title,
/// but keeps this harness engine-neutral the way `walkthroughs.rs`'s
/// `submit_command` is) and as an ordinary typed line otherwise.
fn submit_command(session: &mut ScottSession, cmd: &str) -> TurnResult {
    match session.pending_input() {
        InputKind::Char => {
            let c = cmd.chars().next().unwrap_or(' ');
            session
                .submit_key(app::engine::KeyInput::Char(c))
                .expect("a Char-pending Scott session must answer submit_key")
        }
        _ => session.submit(cmd),
    }
}

/// Collapse whitespace so the determinism/golden-file comparisons below don't
/// care about incidental formatting.
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "Are we at the checkpoint" is read directly off `current_location()`,
/// which Scott's `TurnResult`/`Engine::current_location()` always supplies
/// (unlike Photopia's Z-code, which never seeds one) -- no need to submit an
/// extra `look` to confirm the room the way the Photopia harness does.
fn at_treasure_room(session: &ScottSession) -> bool {
    session
        .current_location()
        .is_some_and(|loc| loc.name.contains("damp hollow stump in the swamp"))
}

/// Exercises the host Save State/Restore State round trip: bare
/// `Engine::save_state`/`archive::save_archive_meta_pics`/
/// `archive::load_archive`/`Engine::restore_state`, no v6 screen table or
/// pictures -- Scott has neither, matching `walkthroughs.rs`'s Photopia
/// shape for the same reason. Scott's persistence is ENTIRELY this
/// mechanism (`ScottSession`'s own module doc: "no in-game @save/@restore
/// suspension protocol -- persistence is entirely the host-driven Save
/// State snapshot"), so there is no separate in-game-`@save` sub-case to
/// exercise the way Photopia's Z-machine one has. Leaves `session` exactly
/// where it found it -- back in the treasure room -- so the caller's script
/// loop can resume as if this had never run.
fn mid_script_persistence_checks(session: &mut ScottSession) {
    assert!(at_treasure_room(session), "checkpoint precondition: back in the treasure room after depositing the keys");

    let es = Engine::save_state(session);
    let ss_dir = app::scratch_dir("sq1600-adventureland-host-save");
    let ss_path = ss_dir.join("hoststate.lanthorn");
    archive::save_archive_meta_pics(
        &ss_path,
        &Mapper::default(),
        &es,
        None,
        session.aux_data(),
        Meta {
            format_version: archive::CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: CHECKPOINT_AFTER_COMMAND as u32,
            saved_at: String::new(),
            location: Some("damp hollow stump in the swamp".to_string()),
            score: None,
            trigger: SaveTrigger::HostState,
            source: archive::SaveSource::default(),
        },
        &archive::SessionRecord::empty(),
        &[],
        None,
        None,
    )
    .expect("save_archive_meta_pics");

    // Drift far enough that the restore below proves something real: leave
    // the treasure room entirely.
    let drift = session.submit("up");
    assert!(!at_treasure_room(session), "the drift really left the checkpoint: {:?}", drift.transcript);

    let ac = archive::load_archive(&ss_path).expect("load_archive");
    let _ = std::fs::remove_dir_all(&ss_dir);
    Engine::restore_state(session, &ac.engine_save()).expect("restore_state (Scott VM snapshot)");
    assert!(!session.has_quit());
    assert!(at_treasure_room(session), "the host Save State/Restore State round trip lands back at the checkpoint");
}

/// Mirrors `sq1586_host_hints_open.rs`'s "no hint sidecar" case: Adventureland
/// carries no InvisiClues-style hint file anywhere lanthorn looks, so the
/// point here is only that the check machinery itself does not error when
/// asked about it.
fn check_hints_machinery_agrees(story_path: &Path, story_bytes: &[u8]) {
    let ifid = compute_ifid(story_bytes);
    let home = app::scratch_dir("sq1600-adventureland-empty-hint-index");
    let index = hints::load_hint_index(&home);

    assert_eq!(
        available(story_path, &ifid, "", &index),
        HintAvailability::None,
        "Adventureland carries no hint sidecar anywhere lanthorn looks"
    );
    let cfg = Config::default();
    let result = open(story_path, &ifid, "", &index, &[], &cfg).expect("no hint source is not an error");
    assert!(result.is_none(), "open() finds nothing, exactly as available() said");
}

struct PlayOutcome {
    /// The ordinary walkthrough transcript -- every script command's printed
    /// reply, in order. The mid-script persistence check's OWN turns are
    /// deliberately not folded in here, so this stays a clean read of the
    /// walkthrough itself for the determinism/golden-file comparisons below.
    transcript: String,
    mapper: Mapper,
    quit: bool,
}

/// Drives the full committed script through a freshly booted session,
/// injecting [`mid_script_persistence_checks`] at [`CHECKPOINT_AFTER_COMMAND`].
fn play_full_script(mut session: ScottSession, commands: &[String]) -> PlayOutcome {
    assert!(
        commands.len() < MAX_TURNS,
        "the committed script ({} lines) exceeds the sanity cap ({MAX_TURNS}) -- inspect it before raising the cap",
        commands.len()
    );
    let mut transcript = String::new();
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();

    for (i, cmd) in commands.iter().enumerate() {
        let result = submit_command(&mut session, cmd);
        assert!(
            result.fault.is_none(),
            "turn {} ({cmd:?}) faulted: {:?}\ntranscript so far: {transcript}",
            i + 1,
            result.fault
        );
        transcript.push_str(&result.transcript);
        transcript.push('\n');
        apply_turn(&mut mapper, cmd, &result, &mut death);
        apply_item_observations(&mut mapper, (i + 1) as u32, &result);

        if i + 1 == CHECKPOINT_AFTER_COMMAND {
            mid_script_persistence_checks(&mut session);
        }
    }

    PlayOutcome { transcript, mapper, quit: session.has_quit() }
}

#[test]
fn adventureland_reaches_its_ending_with_no_fault_and_a_sane_mapper_and_hints_state() {
    let Some((bytes, session)) = boot_adventureland() else {
        eprintln!("SKIP: gitignored story missing at stories/adv01.dat");
        return;
    };
    let story_path = fixture_path("adv01.dat");
    let commands = read_script();
    assert!(commands.len() > 100, "premise: the committed script is the full walkthrough, not a stub");

    let outcome = play_full_script(session, &commands);

    assert!(
        norm(&outcome.transcript).contains("I've stored 13 treasures"),
        "Adventureland's own SCORE verb must report all 13 treasures stored: {:?}",
        outcome.transcript
    );
    assert!(
        norm(&outcome.transcript).contains("Well done."),
        "Adventureland's real winning ending prints \"Well done.\" once every treasure is stored: {:?}",
        outcome.transcript
    );
    assert!(
        norm(&outcome.transcript).contains("The game is now over."),
        "the SCORE verb's win check falls through to the same ending text QUIT prints: {:?}",
        outcome.transcript
    );
    assert!(outcome.quit, "the real winning ending must reach has_quit()");

    // Mapper: a real Inform-style location signal every turn (unlike
    // Photopia's Z-code, which gives none) -- the walkthrough visits every
    // room reachable without dying, so the graph should be populated, not
    // empty. Deliberately a loose lower bound rather than the exact final
    // count: the point is that driving ~150 turns through `apply_turn`
    // discovers a real map and never panics, not pinning adv01.dat's exact
    // room graph (already covered by `scott_mapper.rs`'s dedicated case).
    assert!(
        outcome.mapper.graph.rooms().count() > 15,
        "Adventureland gives the mapper a location every turn; a walkthrough covering this much of the \
         game should have discovered well over a dozen distinct rooms, got {}",
        outcome.mapper.graph.rooms().count()
    );

    check_hints_machinery_agrees(&story_path, &bytes);

    // ---- SQ-1647: item-tracking sanity (deliberately no exact item list/count pinning --
    // see the quest; this repo's own `synonym_groups.tsv` precedent is that pinned lines
    // break on unrelated changes) ----
    //
    // Empirically (SQ-1647): the full ~150-command playthrough tracks 27 distinct items --
    // Adventureland is a small, sparse Scott Adams database, so the bar here is
    // deliberately lower than the Z-machine/Glulx specimens'. No scenery-filter check here
    // (unlike Anchorhead's, below): `is_scenery_or_door` is `gvm`-specific (SQ-1640), and
    // `crates/scott` has no analogous filter to exercise -- see this quest's report for that
    // as a possible follow-up, not built here (would be new production scope).
    let items: Vec<_> = outcome.mapper.graph.items().collect();
    assert!(
        items.len() > 10,
        "Adventureland gives the mapper an item observation every turn; a walkthrough covering \
         this much of the game should have tracked well over a dozen distinct items, got {}",
        items.len()
    );
    let vanished = items
        .iter()
        .filter(|(_, r)| matches!(r.last_seen, mapper::graph::ItemLocation::Vanished { .. }))
        .count();
    assert!(
        vanished * 10 < items.len() * 9,
        "{vanished} of {} tracked items ended Vanished -- that's suspiciously close to \"everything\", \
         the shape a regression that vanishes on every sweep would produce (empirically this specimen sees ~19%)",
        items.len()
    );
    for (key, rec) in &items {
        if let mapper::graph::ItemLocation::Vanished { room, .. } = rec.last_seen {
            assert!(
                outcome.mapper.graph.rooms().any(|r| r.id == room),
                "item {key} ({:?}) is marked Vanished from room {room}, which the graph never actually mapped",
                rec.name
            );
        }
    }
}

#[test]
fn adventureland_walkthrough_is_deterministic_and_matches_the_pinned_transcript() {
    let Some((_, session1)) = boot_adventureland() else {
        eprintln!("SKIP: gitignored story missing at stories/adv01.dat");
        return;
    };
    let commands = read_script();

    let run1 = norm(&play_full_script(session1, &commands).transcript);

    let Some((_, session2)) = boot_adventureland() else {
        eprintln!("SKIP: gitignored story missing at stories/adv01.dat");
        return;
    };
    let run2 = norm(&play_full_script(session2, &commands).transcript);

    assert_eq!(
        run1, run2,
        "two fresh runs of the same script must produce byte-identical (normalized) transcripts -- \
         this is the falsifiable claim that Adventureland's PRNG is deterministically seeded, not the \
         walkthrough author's say-so (crates/scott's occurrence rolls: chigger bites, mud drying, bees \
         suffocating, the fish's escape/death chances)"
    );

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest.join("tests/fixtures/walkthroughs/adventureland.transcript");
    if std::env::var_os("SQ1600_REGEN_GOLDEN").is_some() {
        std::fs::write(&golden_path, format!("{run1}\n")).expect("regenerate the pinned transcript");
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("committed reference transcript must be readable at {}: {e}", golden_path.display()));
    assert_eq!(
        run1,
        golden.trim_end(),
        "the current run's normalized transcript no longer matches the pinned reference \
         (crates/app/tests/fixtures/walkthroughs/adventureland.transcript) -- if this is an intentional \
         behaviour change, regenerate the pinned file and explain why in the commit"
    );
}
