//! SQ-1600: a headless walkthrough-regression harness for **The
//! Hitchhiker's Guide to the Galaxy** (`hitchhiker-r59-s851108.z3`, v3
//! Z-code), the second lane of the multi-game quest that
//! `crates/app/tests/suites/walkthroughs.rs` (Photopia, Phase 1) started.
//! Same shape: boots the real story via the library session API
//! (`GameSession`/`Engine`, no terminal), drives a committed, hand-verified
//! command script (`tests/fixtures/walkthroughs/hhgg.txt`) to the game's own
//! real winning ending, exercises an in-game `@save`/`@restore` round trip
//! through the host archive path, a host Save State/Restore State round
//! trip and a `restart` reboot partway through, checks the mapper and hints
//! machinery don't error while being driven, and pins a normalized
//! reference transcript (`tests/fixtures/walkthroughs/hhgg.transcript`) that
//! a future regression must keep matching.
//!
//! HHGG is v3 Z-code with the automatic status line (score + moves), so
//! unlike Photopia this specimen DOES have an engine-provided score: the
//! real winning playthrough below reaches the maximum, 400 of a possible
//! 400 (`GameSession::save_summary`'s own doc: score comes from the
//! Z-machine v1-3 automatic status line). The real ending is the game's own
//! — stepping down the hatch onto Magrathea, printing the score report, and
//! quitting on its own with no explicit `quit`+confirm dance needed (the
//! same natural `has_quit()` shape Photopia's ending has).
//!
//! The command script was reconstructed empirically: booting the real game
//! and confirming every command's actual printed reply. See the script
//! file's own header for the two IF Archive solutions consulted as a guide
//! only (never transcribed) and — importantly — for why the script is long
//! and what it does to dodge a scripted, unavoidable death.

use std::path::Path;

use app::archive::{self, Meta, SaveTrigger, SessionRecord};
use app::config::Config;
use app::engine::{Engine, StatusField, StatusModel};
use app::hints;
use app::host::hints::{available, open, HintAvailability};
use app::ifid::compute_ifid;
use app::session::{
    apply_item_observations, apply_turn, status_model_from_machine, DeathWatch, GameSession, InputKind, PendingIo,
    TurnResult,
};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// Which script command the mid-script persistence checks fire after: the
/// `eat fruit` that reveals the vision (turn 571 of the committed script),
/// well past every random-scenario draw the middle act makes — verified
/// empirically that nothing after this point in the script depends on the
/// Z-machine `random` opcode, so the extra turns the checks themselves
/// spend (a `save`, a `restore`, a `down`/restore drift) cannot perturb any
/// later command's outcome. Deliberately NOT placed earlier: the whole
/// middle act (the five flashback vignettes, the war-room/maze repeat-visit
/// death trap, the poetry code word) is `random`-sensitive turn-for-turn,
/// and inserting extra turns there would have meant re-deriving the entire
/// back half of the script by hand. At this point the story is on the
/// Bridge with a Line prompt, which is what the checks need.
const CHECKPOINT_AFTER_COMMAND: usize = 571;

/// A generous cap over the committed script's ~600 real commands — large
/// enough that a legitimate run never gets near it, small enough that a
/// script stuck resubmitting the same prompt forever fails loudly instead of
/// hanging CI.
const MAX_TURNS: usize = 900;

fn boot_hhgg() -> Option<(Vec<u8>, GameSession)> {
    let path = fixture_path("hitchhiker-r59-s851108.z3");
    let bytes = std::fs::read(&path).ok()?;
    let session = GameSession::new_with_trace(
        bytes.clone(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("Hitchhiker's Guide should boot without a ZError");
    Some((bytes, session))
}

fn read_script() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("tests/fixtures/walkthroughs/hhgg.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("committed walkthrough script must be readable at {}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Submit one script command, routing it to a keypress when the game is at a
/// single-key (Char) prompt — exactly what the exploration used to derive the
/// script did — and as an ordinary typed line otherwise.
fn submit_command(session: &mut GameSession, cmd: &str) -> TurnResult {
    match session.pending_input() {
        InputKind::Char => session.submit_char(cmd.as_bytes().first().copied().unwrap_or(b' ')),
        _ => session.submit(cmd),
    }
}

/// Collapse whitespace so the determinism/golden-file comparisons below don't
/// care about incidental line wrapping — matches the repo's existing
/// precedent (`restart_reboots_in_place.rs`'s `norm`, `walkthroughs.rs`'s own).
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether we're at the checkpoint: back on the Heart of Gold's Bridge,
/// right after eating the Tree of Foreknowledge's fruit.
fn looking_shows_bridge(session: &mut GameSession) -> bool {
    session.submit("look").transcript.contains("Bridge")
}

/// Exercises the in-game `@save`/`@restore` round trip (through the host
/// archive path, mirroring `walkthroughs.rs`'s own) and the host Save
/// State/Restore State round trip. Leaves `session` exactly where it found
/// it — back on the Bridge, pending a Line command — so the caller's script
/// loop can resume as if this had never run.
fn mid_script_persistence_checks(session: &mut GameSession) {
    assert!(
        looking_shows_bridge(session),
        "checkpoint precondition: back on the Bridge after eating the Tree of Foreknowledge's fruit"
    );

    // ---- In-game @save / @restore, through the host archive path ------------
    let r = session.submit("save");
    assert_eq!(
        r.pending_io,
        Some(PendingIo::Save),
        "Hitchhiker's SAVE verb must bubble a host Save request: {:?}",
        r.transcript
    );

    let ingame = app::persist_files::game_save_bytes(session, SaveTrigger::Ingame);
    let dir = app::scratch_dir("sq1600-hhgg-ingame-save");
    app::persist_files::save_named(
        &dir,
        "SQ1600-HHGG",
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
        Some("Bridge".to_string()),
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

    let bytes = archive::read_quetzal_from_file(&path).expect("archive's game.z3 reads back");
    session
        .restore_game_save(&bytes)
        .expect("a real Hitchhiker's @save archive must restore through the host restore path");
    assert!(!session.has_quit(), "the restore left the session alive");
    assert!(looking_shows_bridge(session), "restored to the save point");
    let _ = std::fs::remove_dir_all(&dir);

    // ---- Host Save State / Restore State --------------------------------------
    let es = Engine::save_state(session);
    let ss_dir = app::scratch_dir("sq1600-hhgg-host-save");
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
            location: Some("Bridge".to_string()),
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

    // Drift far enough that the restore below proves something real: go down
    // to the Fore End corridor, which leaves the Bridge entirely.
    let drift = session.submit("down");
    assert!(
        !drift.transcript.contains("Bridge"),
        "the drift really left the checkpoint: {:?}",
        drift.transcript
    );

    let ac = archive::load_archive(&ss_path).expect("load_archive");
    let _ = std::fs::remove_dir_all(&ss_dir);
    Engine::restore_state(session, &ac.engine_save()).expect("restore_state (Quetzal memory/stack/pc)");
    let _ = session.take_transcript();
    assert!(!session.has_quit());
    assert!(
        looking_shows_bridge(session),
        "the host Save State/Restore State round trip lands back at the checkpoint"
    );
}

/// After a `restart` command, answer the game's Line confirmation(s) up to
/// several turns, collecting the reboot output. Bounded — never loops
/// forever. Mirrors `walkthroughs.rs`'s `confirm_restart`, but runs its full
/// iteration count rather than breaking on the first non-empty reply:
/// Hitchhiker's `restart` verb is a TWO-step dance ("Hit RETURN or ENTER
/// when ready" to see the score, THEN "Do you wish to restart? (Y is
/// affirmative):"), unlike Photopia's one-step confirmation, so breaking
/// early here would stop right at the second prompt without ever answering
/// it — verified empirically (see `zzscratch_restart.rs`'s exploration,
/// since deleted).
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
        collected.push(' ');
    }
    collected
}

/// Boots a completely SEPARATE Hitchhiker's session (never the one the main
/// walkthrough is driving) and proves `restart` reboots in place rather than
/// quitting the app — mirrors `walkthroughs.rs`'s `check_restart_reboots`.
fn check_restart_reboots(bytes: &[u8]) {
    let mut session = GameSession::new_with_trace(
        bytes.to_vec(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("Hitchhiker's Guide should boot for the restart probe");
    let boot_banner = norm(&session.take_transcript());
    assert!(
        boot_banner.contains("THE HITCHHIKER'S GUIDE TO THE GALAXY"),
        "sanity: the real boot banner: {boot_banner:?}"
    );

    let _ = session.submit("stand");
    let _ = session.submit("turn on light");

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
        "restart must re-run Hitchhiker's from the opening (boot banner should reappear)\n  probe: {probe:?}\n  reboot: {reboot_text:?}"
    );
}

/// Mirrors `sq1586_host_hints_open.rs`'s SLAG-naming case: `hhgginv.z5`
/// sits beside `hitchhiker-r59-s851108.z3` in the gitignored `stories/`
/// checkout, so — unlike Photopia, which has no hint sidecar anywhere —
/// this specimen's hints ARE available, and the point here is that
/// `available`/`open` agree and the hint VM boots cleanly.
fn check_hints_machinery_agrees(story_path: &Path, story_bytes: &[u8]) {
    let ifid = compute_ifid(story_bytes);
    let home = app::scratch_dir("sq1600-hhgg-hint-index");
    let index = hints::load_hint_index(&home);

    if !story_path
        .parent()
        .map(|p| p.join("hhgginv.z5").is_file())
        .unwrap_or(false)
    {
        // The hint sidecar is only ever present via a developer's own
        // `stories/` checkout (never fetched) — skip vacuously without it,
        // exactly like the rest of this suite does for the story itself.
        eprintln!("SKIP: hhgginv.z5 absent beside the story — hints check skipped");
        return;
    }

    assert_eq!(
        available(story_path, &ifid, "", &index),
        HintAvailability::Available,
        "hhgginv.z5 sits beside hitchhiker-r59-s851108.z3, so a hint source resolves"
    );
    let cfg = Config::default();
    let session = open(story_path, &ifid, "", &index, &[], &cfg)
        .expect("a resolved hint source boots")
        .expect("available() said yes, so open() must find the same source");
    let opening = session.transcript.join("\n");
    assert!(!opening.trim().is_empty(), "the hint program's first screen is not blank");
    assert!(
        opening.contains("InvisiClues"),
        "the first screen names the InvisiClues booklet, got: {opening}"
    );
    assert_eq!(session.label, "hhgginv.z5");
}

struct PlayOutcome {
    /// The ordinary walkthrough transcript — every script command's printed
    /// reply, in order. The mid-script persistence checks' OWN turns are
    /// deliberately not folded in here, so this stays a clean read of the
    /// walkthrough itself for the determinism/golden-file comparisons below.
    transcript: String,
    mapper: Mapper,
    quit: bool,
}

/// Drives the full committed script through a freshly booted session,
/// injecting [`mid_script_persistence_checks`] at [`CHECKPOINT_AFTER_COMMAND`].
fn play_full_script(mut session: GameSession, commands: &[String]) -> PlayOutcome {
    assert!(
        commands.len() < MAX_TURNS,
        "the committed script ({} lines) exceeds the sanity cap ({MAX_TURNS}) — inspect it before raising the cap",
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

        if session.has_quit() {
            break;
        }
    }

    PlayOutcome { transcript, mapper, quit: session.has_quit() }
}

#[test]
fn hhgg_reaches_its_ending_with_no_fault_and_a_sane_mapper_and_hints_state() {
    let Some((bytes, session)) = boot_hhgg() else {
        eprintln!("SKIP: gitignored story missing at stories/hitchhiker-r59-s851108.z3");
        return;
    };
    let story_path = fixture_path("hitchhiker-r59-s851108.z3");
    let commands = read_script();
    assert!(commands.len() > 400, "premise: the committed script is the full walkthrough, not a stub");

    let outcome = play_full_script(session, &commands);

    assert!(
        outcome.quit,
        "Hitchhiker's own ending must reach has_quit() naturally, with no explicit quit+confirm needed"
    );
    assert!(
        norm(&outcome.transcript).contains(
            "You set one single foot on the ancient dust -- and almost instantly the most incredible adventure starts"
        ),
        "Hitchhiker's real ending: stepping onto Magrathea, to be continued in the sequel — got: {:?}",
        outcome.transcript
    );
    assert!(
        norm(&outcome.transcript).contains("Your score is 400 of a possible 400"),
        "the real winning playthrough reaches the maximum score — got: {:?}",
        outcome.transcript
    );

    // Mapper: unlike Photopia, HHGG is a classic Inform-library-shaped game
    // with a real location global the standard heuristic can read (the
    // playthrough visits ~28 distinct rooms/vignette locations end to end),
    // so the acceptance here is a POPULATED graph, not an empty one.
    assert!(
        outcome.mapper.graph.rooms().count() > 10,
        "Hitchhiker's gives the mapper a real location signal throughout — got {} rooms, expected a populated graph",
        outcome.mapper.graph.rooms().count()
    );

    check_restart_reboots(&bytes);
    check_hints_machinery_agrees(&story_path, &bytes);

    // ---- SQ-1647: item-tracking sanity (deliberately no exact item list/count pinning —
    // see the quest; this repo's own `synonym_groups.tsv` precedent is that pinned lines
    // break on unrelated changes) ----
    //
    // Empirically (SQ-1647): the full 600+-command playthrough tracks 106 distinct items.
    // A generous floor well under that, not the exact figure, so an unrelated future script
    // edit doesn't need to touch this assertion — the point is only that item detection
    // didn't quietly stop working.
    let items: Vec<_> = outcome.mapper.graph.items().collect();
    assert!(
        items.len() > 50,
        "Hitchhiker's is a real Inform-shaped game with plenty of scenery/inventory objects; \
         a full playthrough should track well over a few dozen, got {}",
        items.len()
    );
    let vanished = items
        .iter()
        .filter(|(_, r)| matches!(r.last_seen, mapper::graph::ItemLocation::Vanished { .. }))
        .count();
    assert!(
        vanished * 10 < items.len() * 9,
        "{vanished} of {} tracked items ended Vanished — that's suspiciously close to \"everything\", \
         the shape a regression that vanishes on every sweep would produce (empirically this specimen sees ~18%)",
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

    // ---- SQ-1649: no bare pronoun/generic ever reaches the tracker as an item's display name,
    // and the player's own avatar object is never tracked as if it were an item of itself ----
    //
    // HHGG's own avatar object (#31) prints the header short name "it" and its only parse word is
    // "protag" ("PROTAGONIST" truncated to 6 chars) — before `zvm::location`'s `PLAYER_WORDS`
    // recognised that word, this object was never identified as the player, so it followed the
    // player into every room as a persistent fake "it" item, AND — the bigger half of the same
    // gap — the story's carried inventory was never tracked at all (no `player_obj` to walk),
    // empirically 0 of 3,319 item observations `Carried` in the full walkthrough.
    for (key, rec) in &items {
        let lower = rec.name.to_lowercase();
        assert!(
            !["it", "them", "this", "that", "thing"].contains(&lower.as_str()),
            "item {key} tracked under the bare pronoun {:?} — item_tracker_display_name should have \
             fallen through to a real word from the object's own vocabulary instead",
            rec.name
        );
    }
    assert!(
        items.iter().any(|(_, r)| matches!(r.last_seen, mapper::graph::ItemLocation::Carried)),
        "HHGG's avatar (\"protag\") must now be recognised as the player, so the playthrough's own \
         inventory (screwdriver, towel, satchel, …) should show up as Carried at least once"
    );
}

#[test]
fn hhgg_walkthrough_is_deterministic_and_matches_the_pinned_transcript() {
    let Some((_, session1)) = boot_hhgg() else {
        eprintln!("SKIP: gitignored story missing at stories/hitchhiker-r59-s851108.z3");
        return;
    };
    let commands = read_script();

    let run1 = norm(&play_full_script(session1, &commands).transcript);

    let Some((_, session2)) = boot_hhgg() else {
        eprintln!("SKIP: gitignored story missing at stories/hitchhiker-r59-s851108.z3");
        return;
    };
    let run2 = norm(&play_full_script(session2, &commands).transcript);

    assert_eq!(
        run1, run2,
        "two fresh runs of the same script must produce byte-identical (normalized) transcripts — \
         this is the falsifiable claim that zvm's `random` opcode is deterministic from a fixed boot \
         seed, not the walkthrough author's say-so. HHGG's own middle act is heavily `random`-driven \
         (see the script file's header), so this is the sharpest test of that claim in the quest."
    );

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest.join("tests/fixtures/walkthroughs/hhgg.transcript");
    if std::env::var_os("SQ1600_REGEN_GOLDEN").is_some() {
        std::fs::write(&golden_path, format!("{run1}\n")).expect("regenerate the pinned transcript");
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("committed reference transcript must be readable at {}: {e}", golden_path.display()));
    assert_eq!(
        run1,
        golden.trim_end(),
        "the current run's normalized transcript no longer matches the pinned reference \
         (crates/app/tests/fixtures/walkthroughs/hhgg.transcript) — if this is an intentional \
         behaviour change, regenerate the pinned file and explain why in the commit"
    );
}

/// Sanity that reading the real automatic status line agrees with the
/// ending's printed score, once the game has quit. Not a `#[test]` on its
/// own — folded into a normal turn of exploration during development, kept
/// here as a comment for future readers: `status_model_from_machine`
/// returns `StatusModel::Classic { right: StatusField::ScoreTurns { .. }, .. }`
/// for a v1-3 game like this one, matching `engine_helpers::save_summary`'s
/// own doc on where a v1-3 game's score lives.
#[allow(dead_code)]
fn _score_reads_through_status_model(session: &GameSession) -> Option<i16> {
    match status_model_from_machine(&session.machine) {
        StatusModel::Classic { right: StatusField::ScoreTurns { score, .. }, .. } => Some(score),
        _ => None,
    }
}
