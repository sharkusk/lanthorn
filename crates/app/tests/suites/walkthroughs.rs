//! SQ-1600 Phase 1: a headless walkthrough-regression harness, proven here on
//! ONE game (Photopia, `photopia.z5`) before the remaining six titles the
//! quest names get their own lanes. Loads a committed, hand-verified command
//! script (`tests/fixtures/walkthroughs/photopia.txt`), drives it through the
//! library session API (`GameSession`/`Engine`, no terminal) to the game's own
//! real ending, exercises an in-game `@save`/`@restore` round trip, a host
//! Save State/Restore State round trip and a `restart` reboot partway
//! through, checks the mapper and hints machinery don't error while being
//! driven, and pins a normalized reference transcript
//! (`tests/fixtures/walkthroughs/photopia.transcript`) that a future
//! regression must keep matching.
//!
//! Photopia is v5 Z-code (not v6, not Glulx) with **no engine-provided
//! score** (`GameSession::save_summary`'s own doc: score comes only from the
//! Z-machine v1-3 automatic status line) and its own real ending is silent —
//! the last thing it prints is "You turn out the light.", then two blank
//! keypresses later `has_quit()` goes true on its own, with no `quit`+`y`
//! confirmation dance. So the acceptance here is that exact ending TEXT plus
//! the natural `has_quit()` signal, not a score assertion that does not apply
//! to this specimen (see the quest brief for the full reasoning).
//!
//! The command script was reconstructed empirically: booting the real game
//! and confirming every command's actual printed reply, using Graham
//! Pearce's published solution (`photopia.sol`, IF Archive, 02-Jul-2000) as a
//! guide only — the committed script is this project's own derived command
//! list, not a copy of that prose.

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

/// Which script command the mid-script persistence checks fire after: the
/// fifth (and last) `south` in the committed script, back at the Landing
/// site and about to launch into orbit — after the red-planet scene, before
/// the undersea castle. Verified empirically: Photopia accepts `save` there
/// (a Line prompt, not the Char prompts the surrounding cutscenes use).
const CHECKPOINT_AFTER_COMMAND: usize = 18;

/// A generous cap over the committed script's ~130 real commands — large
/// enough that a legitimate run never gets near it, small enough that a
/// script stuck resubmitting the same prompt forever fails loudly instead of
/// hanging CI.
const MAX_TURNS: usize = 500;

fn boot_photopia() -> Option<(Vec<u8>, GameSession)> {
    let path = fixture_path("photopia.z5");
    let bytes = std::fs::read(&path).ok()?;
    let session = GameSession::new_with_trace(
        bytes.clone(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("Photopia should boot without a ZError");
    Some((bytes, session))
}

fn read_script() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("tests/fixtures/walkthroughs/photopia.txt");
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
/// care about incidental v6-style line wrapping (Photopia has none, but this
/// matches the repo's existing precedent — `restart_reboots_in_place.rs`'s
/// `norm`).
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Photopia's `TurnResult::location` (and therefore `current_location()`) is
/// always `None` — it is not an Inform-library game with a conventional
/// location global for the standard heuristic to read, and this suite's own
/// exploration never saw a location signal at any point in the whole
/// playthrough. So "are we at the checkpoint" is read off the printed room
/// name instead, exactly the way a player would confirm it: `look`.
fn looking_shows_landing_site(session: &mut GameSession) -> bool {
    session.submit("look").transcript.contains("Landing site")
}

/// Exercises the in-game `@save`/`@restore` round trip (through the host
/// archive path, mirroring `glulx_ingame_save_host_restore.rs`) and the host
/// Save State/Restore State round trip (mirroring `zork0_v6_persistence.rs`'s
/// non-v6 shape: bare `Engine::save_state`/`archive::save_archive_meta_pics`/
/// `archive::load_archive`/`Engine::restore_state`, no v6 screen table or
/// pictures — Photopia has neither). Leaves `session` exactly where it found
/// it — back at the Landing site, pending a Line command — so the caller's
/// script loop can resume as if this had never run.
fn mid_script_persistence_checks(session: &mut GameSession) {
    assert!(
        looking_shows_landing_site(session),
        "checkpoint precondition: back at the Landing site before launching into orbit"
    );

    // ---- In-game @save / @restore, through the host archive path ------------
    let r = session.submit("save");
    assert_eq!(
        r.pending_io,
        Some(PendingIo::Save),
        "Photopia's SAVE verb must bubble a host Save request: {:?}",
        r.transcript
    );

    let ingame = app::persist_files::game_save_bytes(session, SaveTrigger::Ingame);
    let dir = app::scratch_dir("sq1600-photopia-ingame-save");
    app::persist_files::save_named(
        &dir,
        "SQ1600-PHOTOPIA",
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
        Some("Landing site".to_string()),
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

    let bytes = archive::read_quetzal_from_file(&path).expect("archive's game.z5 reads back");
    session
        .restore_game_save(&bytes)
        .expect("a real Photopia @save archive must restore through the host restore path");
    assert!(!session.has_quit(), "the restore left the session alive");
    assert!(looking_shows_landing_site(session), "restored to the save point");
    let _ = std::fs::remove_dir_all(&dir);

    // ---- Host Save State / Restore State --------------------------------------
    let es = Engine::save_state(session);
    let ss_dir = app::scratch_dir("sq1600-photopia-host-save");
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
            location: Some("Landing site".to_string()),
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

    // Drift far enough that the restore below proves something real: launch
    // back into orbit, which leaves the Landing site entirely.
    let drift = session.submit("up");
    assert!(
        !drift.transcript.contains("Landing site"),
        "the drift really left the checkpoint: {:?}",
        drift.transcript
    );

    let ac = archive::load_archive(&ss_path).expect("load_archive");
    let _ = std::fs::remove_dir_all(&ss_dir);
    Engine::restore_state(session, &ac.engine_save()).expect("restore_state (Quetzal memory/stack/pc)");
    let _ = session.take_transcript();
    assert!(!session.has_quit());
    assert!(
        looking_shows_landing_site(session),
        "the host Save State/Restore State round trip lands back at the checkpoint"
    );
}

/// After a `restart` command, answer the game's Line confirmation up to a
/// few turns, collecting the reboot output. Bounded — never loops forever.
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

/// Boots a completely SEPARATE Photopia session (never the one the main
/// walkthrough is driving) and proves `restart` reboots in place rather than
/// quitting the app — mirrors `restart_reboots_in_place.rs`'s pattern.
fn check_restart_reboots(bytes: &[u8]) {
    let mut session = GameSession::new_with_trace(
        bytes.to_vec(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("Photopia should boot for the restart probe");
    let boot_banner = norm(&session.take_transcript());
    assert!(
        boot_banner.contains("Would you like instructions?"),
        "sanity: the real boot banner: {boot_banner:?}"
    );

    let _ = session.submit("no");
    let _ = session.submit_char(b'x');

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
        "restart must re-run Photopia from the opening (boot banner should reappear)\n  probe: {probe:?}\n  reboot: {reboot_text:?}"
    );
}

/// Mirrors `sq1586_host_hints_open.rs`'s "no hint sidecar" case: Photopia
/// carries no InvisiClues-style hint file anywhere lanthorn looks, so the
/// point here is only that the check machinery itself does not error when
/// asked about it.
fn check_hints_machinery_agrees(story_path: &Path, story_bytes: &[u8]) {
    let ifid = compute_ifid(story_bytes);
    let home = app::scratch_dir("sq1600-photopia-empty-hint-index");
    let index = hints::load_hint_index(&home);

    assert_eq!(
        available(story_path, HintStory::new(&ifid, ""), &index),
        HintAvailability::None,
        "Photopia carries no hint sidecar anywhere lanthorn looks"
    );
    let cfg = Config::default();
    let result = open(story_path, HintStory::new(&ifid, ""), &index, &[], &cfg).expect("no hint source is not an error");
    assert!(result.is_none(), "open() finds nothing, exactly as available() said");
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
    }

    PlayOutcome { transcript, mapper, quit: session.has_quit() }
}

#[test]
fn photopia_reaches_its_ending_with_no_fault_and_a_sane_mapper_and_hints_state() {
    let Some((bytes, session)) = boot_photopia() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/photopia.z5 (run scripts/fetch-fixtures.sh)");
        return;
    };
    let story_path = fixture_path("photopia.z5");
    let commands = read_script();
    assert!(commands.len() > 100, "premise: the committed script is the full walkthrough, not a stub");

    let outcome = play_full_script(session, &commands);

    assert!(
        outcome.quit,
        "Photopia's own ending must reach has_quit() naturally, with no explicit quit+confirm needed"
    );
    assert!(
        norm(&outcome.transcript).contains("You turn out the light."),
        "Photopia's real ending: the nursery light goes out, with no further printed text — got: {:?}",
        outcome.transcript
    );

    // Mapper: light touch (SQ-1600 Phase 1). Checked empirically: Photopia's
    // `TurnResult::location` is `None` on every single turn of this whole
    // playthrough (it is not an Inform-library game with a location global
    // the standard heuristic can read), so `apply_turn` never has anything to
    // seed a room from — the correct, EXPECTED shape here is an empty graph,
    // not a populated one. The acceptance is just that driving all 128 turns
    // through `apply_turn` never panicked.
    assert_eq!(
        outcome.mapper.graph.rooms().count(),
        0,
        "Photopia gives the mapper no location signal at all — an empty graph is the expected shape here, \
         not a bug; if this ever becomes nonzero, something upstream started detecting a location for it"
    );

    check_restart_reboots(&bytes);
    check_hints_machinery_agrees(&story_path, &bytes);

    // ---- SQ-1647: item-tracking sanity (see the module's own quest for why this
    // deliberately does NOT pin an exact item list/count — this repo's own
    // `synonym_groups.tsv` precedent is that pinned lines break on unrelated changes) ----
    //
    // Photopia gives the mapper no location signal at all (the room-count assertion
    // above), so `apply_item_observations` never has a room to attach a RoomDirect/
    // RoomNested sighting to — and this narrative work's minimal object tree gives it no
    // conventional player-object inventory for a Carried sighting either (checked
    // empirically, SQ-1647). An empty item registry is therefore the expected, correct
    // shape here, not a bug, exactly like the empty room graph above.
    assert_eq!(
        outcome.mapper.graph.items().count(),
        0,
        "Photopia gives the mapper no item signal at all — an empty item registry is the expected shape \
         here, not a bug; if this ever becomes nonzero, something upstream started detecting items for it"
    );
}

#[test]
fn photopia_walkthrough_is_deterministic_and_matches_the_pinned_transcript() {
    let Some((_, session1)) = boot_photopia() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/photopia.z5 (run scripts/fetch-fixtures.sh)");
        return;
    };
    let commands = read_script();

    let run1 = norm(&play_full_script(session1, &commands).transcript);

    let Some((_, session2)) = boot_photopia() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/photopia.z5 (run scripts/fetch-fixtures.sh)");
        return;
    };
    let run2 = norm(&play_full_script(session2, &commands).transcript);

    assert_eq!(
        run1, run2,
        "two fresh runs of the same script must produce byte-identical (normalized) transcripts — \
         this is the falsifiable claim that Photopia has no randomness, not the walkthrough author's say-so"
    );

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest.join("tests/fixtures/walkthroughs/photopia.transcript");
    if std::env::var_os("SQ1600_REGEN_GOLDEN").is_some() {
        std::fs::write(&golden_path, format!("{run1}\n")).expect("regenerate the pinned transcript");
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("committed reference transcript must be readable at {}: {e}", golden_path.display()));
    assert_eq!(
        run1,
        golden.trim_end(),
        "the current run's normalized transcript no longer matches the pinned reference \
         (crates/app/tests/fixtures/walkthroughs/photopia.transcript) — if this is an intentional \
         behaviour change, regenerate the pinned file and explain why in the commit"
    );
}
