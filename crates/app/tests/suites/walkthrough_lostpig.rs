//! SQ-1600: a headless walkthrough-regression harness for Lost Pig
//! (`LostPig.z8`, Eric Eve, Release 2 / Serial 080406, Inform v6.30 6/11),
//! mirroring `walkthroughs.rs` (SQ-1600 Phase 1, Photopia) for a different
//! story-file format bucket: z8 Z-code with a standard Inform status line
//! (v4+, so the engine itself is `StatusModel::HostManaged` and reports no
//! score of its own -- see `session::status_model_from_machine`). Loads a
//! committed, hand-verified command script
//! (`tests/fixtures/walkthroughs/lostpig.txt`), drives it through the library
//! session API (`GameSession`/`Engine`, no terminal) to the game's own real
//! BEST ending, exercises an in-game `@save`/`@restore` round trip, a host
//! Save State/Restore State round trip and a `restart` reboot partway
//! through, checks the mapper and hints machinery don't error while being
//! driven, and pins a normalized reference transcript
//! (`tests/fixtures/walkthroughs/lostpig.transcript`) that a future
//! regression must keep matching.
//!
//! Lost Pig has a real, in-game maximum score of 7 (confirmed via the game's
//! own SCORE/FULL commands, not from memory) and a two-tier ending: reaching
//! 6/7 prints "*** Grunk bring pig back to farm ***", while the full 7/7 --
//! earned only by returning BOTH the key and the color-magnet pole to the
//! store room (Shelf Room) AND closing the secret door behind you in the
//! Windy Cave before leaving through the maze -- prints the best ending,
//! "*** Grunk bring pig back to farm and make new friend ***". The
//! acceptance below is that best-ending text plus the full 7/7 tally, not
//! merely reaching a game-over.
//!
//! The command script was reconstructed empirically: booting the real game
//! and confirming every command's actual printed reply. A web search turned
//! up no IF-Archive `.sol` solution file for this title, only vague and
//! partly-inaccurate prose summaries (used only to point at which puzzle
//! areas exist); the committed script is this project's own derived command
//! list, checked command-by-command against the real interpreter, not a
//! transcription of that prose (several of its specific claims, e.g. a
//! literal "coin" on the stairs, turned out to be wrong once checked -- the
//! item there is a whistle, and the coin is later found by searching the dry
//! fountain bowl).

use std::path::Path;

use app::archive::{self, Meta, SaveTrigger, SessionRecord};
use app::config::Config;
use app::engine::Engine;
use app::hints;
use app::hints::HintStory;
use app::host::hints::{available, open, HintAvailability};
use app::ifid::compute_ifid;
use app::session::{apply_item_observations, apply_turn, DeathWatch, GameSession, InputKind, PendingIo, TurnResult};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// Which script command the mid-script persistence checks fire after: `give
/// paper to gnome`, mid-conversation in the Gnome Room, back at a Line
/// prompt. Verified empirically: Lost Pig accepts `save` there (bubbles a
/// host Save request exactly like everywhere else in this game).
const CHECKPOINT_AFTER_COMMAND: usize = 59;

/// A generous cap over the committed script's 142 real commands -- large
/// enough that a legitimate run never gets near it, small enough that a
/// script stuck resubmitting the same prompt forever fails loudly instead of
/// hanging CI.
const MAX_TURNS: usize = 400;

fn boot_lostpig() -> Option<(Vec<u8>, GameSession)> {
    let path = fixture_path("LostPig.z8");
    let bytes = std::fs::read(&path).ok()?;
    let session = GameSession::new_with_trace(
        bytes.clone(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("Lost Pig should boot without a ZError");
    Some((bytes, session))
}

fn read_script() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("tests/fixtures/walkthroughs/lostpig.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("committed walkthrough script must be readable at {}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Submit one script command, routing it to a keypress when the game is at a
/// single-key (Char) prompt -- exactly what the exploration used to derive
/// the script did -- and as an ordinary typed line otherwise. The committed
/// script never actually lands on a Char prompt (Lost Pig's main playthrough
/// is Line input throughout), but this mirrors `walkthroughs.rs`'s helper for
/// the same reason: safety if that ever changes.
fn submit_command(session: &mut GameSession, cmd: &str) -> TurnResult {
    match session.pending_input() {
        InputKind::Char => session.submit_char(cmd.as_bytes().first().copied().unwrap_or(b' ')),
        _ => session.submit(cmd),
    }
}

/// Collapse whitespace so the determinism/golden-file comparisons below don't
/// care about incidental line wrapping -- matches the repo's existing
/// precedent (`walkthroughs.rs`'s `norm`, `restart_reboots_in_place.rs`'s).
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lost Pig is a standard Inform 6 game with real room objects, so unlike
/// Photopia this game's `look` always names the room it's set in -- "are we
/// at the checkpoint" is read off that printed room name, the way a player
/// would confirm it.
fn looking_shows_gnome_room(session: &mut GameSession) -> bool {
    session.submit("look").transcript.contains("Gnome Room")
}

/// Exercises the in-game `@save`/`@restore` round trip (through the host
/// archive path, mirroring `glulx_ingame_save_host_restore.rs`) and the host
/// Save State/Restore State round trip (mirroring `zork0_v6_persistence.rs`'s
/// non-v6 shape: bare `Engine::save_state`/`archive::save_archive_meta_pics`/
/// `archive::load_archive`/`Engine::restore_state`, no v6 screen table or
/// pictures -- Lost Pig has neither, being z8 but not a graphical v6 game).
/// Leaves `session` exactly where it found it -- back in the Gnome Room,
/// pending a Line command -- so the caller's script loop can resume as if
/// this had never run.
fn mid_script_persistence_checks(session: &mut GameSession) {
    assert!(
        looking_shows_gnome_room(session),
        "checkpoint precondition: back in the Gnome Room, mid-conversation, right after giving the gnome the repaired book"
    );

    // ---- In-game @save / @restore, through the host archive path ------------
    let r = session.submit("save");
    assert_eq!(
        r.pending_io,
        Some(PendingIo::Save),
        "Lost Pig's SAVE verb must bubble a host Save request: {:?}",
        r.transcript
    );

    let ingame = app::persist_files::game_save_bytes(session, SaveTrigger::Ingame);
    let dir = app::scratch_dir("sq1600-lostpig-ingame-save");
    app::persist_files::save_named(
        &dir,
        "SQ1600-LOSTPIG",
        "slot",
        SaveTrigger::Ingame,
        &Mapper::default(),
        &ingame,
        None,
        &[],
        None,
        None,
        session.aux_data(),
        CHECKPOINT_AFTER_COMMAND as u32,
        Some("Gnome Room".to_string()),
        None,
        &SessionRecord::empty(),
        &archive::SaveSource::default(),
    )
    .expect("the @save archive writes");
    let path = dir.join("slot.lanthorn");
    assert!(path.exists(), "an in-game @save must produce a .lanthorn the saves manager can list");
    assert_eq!(
        archive::read_archive_meta(&path).expect("meta").trigger,
        SaveTrigger::Ingame,
        "recorded as an in-game save"
    );
    assert_eq!(session.resume_save(true).pending_io, None, "@save completes and the game runs on");

    let bytes = archive::read_quetzal_from_file(&path).expect("archive's game.z8 reads back");
    session
        .restore_game_save(&bytes)
        .expect("a real Lost Pig @save archive must restore through the host restore path");
    assert!(!session.has_quit(), "the restore left the session alive");
    assert!(looking_shows_gnome_room(session), "restored to the save point");
    let _ = std::fs::remove_dir_all(&dir);

    // ---- Host Save State / Restore State --------------------------------------
    let es = Engine::save_state(session);
    let ss_dir = app::scratch_dir("sq1600-lostpig-host-save");
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
            location: Some("Gnome Room".to_string()),
            score: None,
            trigger: SaveTrigger::HostState,
            source: archive::SaveSource::default(),
        },
        &SessionRecord::empty(),
        &[],
        None,
        None,
    )
    .expect("save_archive_meta_pics");

    // Drift far enough that the restore below proves something real: leave
    // the Gnome Room entirely (east, into the Shelf Room).
    let drift = session.submit("east");
    assert!(
        !drift.transcript.contains("Gnome Room"),
        "the drift really left the checkpoint: {:?}",
        drift.transcript
    );

    let ac = archive::load_archive(&ss_path).expect("load_archive");
    let _ = std::fs::remove_dir_all(&ss_dir);
    Engine::restore_state(session, &ac.engine_save()).expect("restore_state (Quetzal memory/stack/pc)");
    let _ = session.take_transcript();
    assert!(!session.has_quit());
    assert!(
        looking_shows_gnome_room(session),
        "the host Save State/Restore State round trip lands back at the checkpoint"
    );
}

/// After a `restart` command, answer the game's confirmation up to a few
/// turns, collecting the reboot output. Bounded -- never loops forever.
/// Mirrors `restart_reboots_in_place.rs`'s `confirm_restart`.
fn confirm_restart(session: &mut GameSession) -> String {
    let mut collected = String::new();
    for _ in 0..6 {
        let result = match session.pending_input() {
            InputKind::Line => session.submit("yes"),
            InputKind::Char => session.submit_char(b'y'),
            InputKind::Event => session.submit(""),
        };
        assert!(!result.quit, "restart must NOT quit the app: {:?}", result.transcript);
        assert!(result.fault.is_none(), "restart faulted: {:?}", result.fault);
        collected.push_str(&result.transcript);
        if !result.transcript.trim().is_empty() {
            break;
        }
    }
    collected
}

/// Boots a completely SEPARATE Lost Pig session (never the one the main
/// walkthrough is driving) and proves `restart` reboots in place rather than
/// quitting the app -- mirrors `restart_reboots_in_place.rs`'s pattern.
fn check_restart_reboots(bytes: &[u8]) {
    let mut session = GameSession::new_with_trace(
        bytes.to_vec(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("Lost Pig should boot for the restart probe");
    let boot_banner = norm(&session.take_transcript());
    assert!(
        boot_banner.contains("Lost Pig"),
        "sanity: the real boot banner: {boot_banner:?}"
    );

    let _ = session.submit("listen");
    let _ = session.submit("search bushes");

    let r = session.submit("restart");
    assert!(!r.quit, "the restart command itself must not quit");
    assert!(r.fault.is_none(), "restart faulted: {:?}", r.fault);
    let mut reboot_text = norm(&r.transcript);
    reboot_text.push(' ');
    reboot_text.push_str(&norm(&confirm_restart(&mut session)));

    assert!(!session.has_quit(), "session must stay alive after restart");
    assert!(session.machine.fault_trace.is_none(), "no fault after restart");
    let probe: String = boot_banner.chars().take(30).collect();
    assert!(
        reboot_text.contains(&probe),
        "restart must re-run Lost Pig from the opening (boot banner should reappear)\n  probe: {probe:?}\n  reboot: {reboot_text:?}"
    );
}

/// Mirrors `sq1586_host_hints_open.rs`'s "no hint sidecar" case: Lost Pig
/// carries no InvisiClues-style hint file anywhere lanthorn looks (its own
/// rich in-game HINT menu is a separate, in-story mechanism this harness
/// never touches), so the point here is only that the check machinery itself
/// does not error when asked about it.
fn check_hints_machinery_agrees(story_path: &Path, story_bytes: &[u8]) {
    let ifid = compute_ifid(story_bytes);
    let home = app::scratch_dir("sq1600-lostpig-empty-hint-index");
    let index = hints::load_hint_index(&home);

    assert_eq!(
        available(story_path, HintStory::new(&ifid, ""), &index),
        HintAvailability::None,
        "Lost Pig carries no hint sidecar anywhere lanthorn looks"
    );
    let cfg = Config::default();
    let result = open(story_path, HintStory::new(&ifid, ""), &index, &[], &cfg, None).expect("no hint source is not an error");
    assert!(result.is_none(), "open() finds nothing, exactly as available() said");
}

struct PlayOutcome {
    /// The ordinary walkthrough transcript -- every script command's printed
    /// reply, in order. The mid-script persistence checks' OWN turns are
    /// deliberately not folded in here, so this stays a clean read of the
    /// walkthrough itself for the determinism/golden-file comparisons below.
    transcript: String,
    mapper: Mapper,
}

/// Drives the full committed script through a freshly booted session,
/// injecting [`mid_script_persistence_checks`] at [`CHECKPOINT_AFTER_COMMAND`].
fn play_full_script(mut session: GameSession, commands: &[String]) -> PlayOutcome {
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

    PlayOutcome { transcript, mapper }
}

#[test]
fn lostpig_reaches_its_best_ending_with_no_fault_and_a_sane_mapper_and_hints_state() {
    let Some((bytes, session)) = boot_lostpig() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/LostPig.z8 (run scripts/fetch-fixtures.sh)");
        return;
    };
    let story_path = fixture_path("LostPig.z8");
    let commands = read_script();
    assert!(commands.len() > 100, "premise: the committed script is the full walkthrough, not a stub");

    let outcome = play_full_script(session, &commands);
    let normalized = norm(&outcome.transcript);

    assert!(
        normalized.contains("Grunk bring pig back to farm and make new friend"),
        "Lost Pig's real BEST ending (all 7 points, key+pole returned and the secret door closed) -- got: {:?}",
        outcome.transcript
    );
    assert!(
        normalized.contains("Grunk have 7 out of 7"),
        "the full 7/7 score tally must be on screen at the ending -- got: {:?}",
        outcome.transcript
    );

    // Mapper: unlike Photopia (SQ-1600 Phase 1, which gives the mapper no
    // location signal at all), Lost Pig is a standard Inform 6 game with real
    // room objects, so `apply_turn`'s heuristic location detection has
    // something to seed rooms from. The acceptance here is just that driving
    // the whole walkthrough through `apply_turn` never panicked and left a
    // sane (non-empty) graph -- not a specific room count, which would pin
    // heuristic detection detail this suite isn't about.
    assert!(
        outcome.mapper.graph.rooms().count() > 0,
        "Lost Pig is a standard Inform game; apply_turn's location heuristic should seed at least one room"
    );

    check_restart_reboots(&bytes);
    check_hints_machinery_agrees(&story_path, &bytes);

    // ---- SQ-1647: item-tracking sanity (deliberately no exact item list/count pinning --
    // see the quest; this repo's own `synonym_groups.tsv` precedent is that pinned lines
    // break on unrelated changes) ----
    //
    // Empirically (SQ-1647): the full 142-command playthrough tracks 99 distinct items. A
    // generous floor well under that, not the exact figure, so an unrelated future script
    // edit doesn't need to touch this assertion -- the point is only that item detection
    // didn't quietly stop working.
    let items: Vec<_> = outcome.mapper.graph.items().collect();
    assert!(
        items.len() > 40,
        "Lost Pig is a real Inform game with plenty of scenery/inventory objects; \
         a full playthrough should track well over a few dozen, got {}",
        items.len()
    );
    let vanished = items
        .iter()
        .filter(|(_, r)| matches!(r.last_seen, mapper::graph::ItemLocation::Vanished { .. }))
        .count();
    assert!(
        vanished * 10 < items.len() * 9,
        "{vanished} of {} tracked items ended Vanished -- that's suspiciously close to \"everything\", \
         the shape a regression that vanishes on every sweep would produce (empirically this specimen sees ~11%)",
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

    // ---- SQ-1649: no compiler-internal Inform identifier ever reaches the tracker as an item's
    // display name ----
    //
    // Lost Pig gives several objects no explicit name at all: a parenthesised placeholder
    // ("(missingOutside)", "(whistle)", "(missingBR)", "(key)" -- Inform's own convention for an
    // object the author never gave a printed name, verified via `zvm::objects::printed_name`'s
    // own doc for the identical `MazeRoom` shape) and six auto-multiplied brick instances
    // ("Brick_1" .. "Brick_6"). `item_tracker_display_name` must fall through to a real word from
    // each object's own vocabulary instead of showing the raw identifier.
    for (key, rec) in &items {
        let looks_like_an_identifier = (rec.name.starts_with('(') && rec.name.ends_with(')'))
            || rec
                .name
                .rsplit_once('_')
                .is_some_and(|(head, tail)| {
                    !tail.is_empty()
                        && tail.bytes().all(|b| b.is_ascii_digit())
                        && head.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                });
        assert!(
            !looks_like_an_identifier,
            "item {key} tracked under the compiler-internal identifier {:?} -- item_tracker_display_name \
             should have fallen through to a real word from the object's own vocabulary instead",
            rec.name
        );
    }
}

#[test]
fn lostpig_walkthrough_is_deterministic_and_matches_the_pinned_transcript() {
    let Some((_, session1)) = boot_lostpig() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/LostPig.z8 (run scripts/fetch-fixtures.sh)");
        return;
    };
    let commands = read_script();

    let run1 = norm(&play_full_script(session1, &commands).transcript);

    let Some((_, session2)) = boot_lostpig() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/LostPig.z8 (run scripts/fetch-fixtures.sh)");
        return;
    };
    let run2 = norm(&play_full_script(session2, &commands).transcript);

    assert_eq!(
        run1, run2,
        "two fresh runs of the same script must produce byte-identical (normalized) transcripts -- \
         this is the falsifiable claim that zvm's fixed-seed RNG makes Lost Pig's pig-catching and \
         other flavor text reproducible turn-for-turn, not the walkthrough author's say-so"
    );

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest.join("tests/fixtures/walkthroughs/lostpig.transcript");
    if std::env::var_os("SQ1600_REGEN_GOLDEN").is_some() {
        std::fs::write(&golden_path, format!("{run1}\n")).expect("regenerate the pinned transcript");
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("committed reference transcript must be readable at {}: {e}", golden_path.display()));
    assert_eq!(
        run1,
        golden.trim_end(),
        "the current run's normalized transcript no longer matches the pinned reference \
         (crates/app/tests/fixtures/walkthroughs/lostpig.transcript) -- if this is an intentional \
         behaviour change, regenerate the pinned file and explain why in the commit"
    );
}
