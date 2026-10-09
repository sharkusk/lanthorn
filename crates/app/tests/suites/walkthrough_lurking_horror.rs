//! SQ-1600: a headless walkthrough-regression harness for The Lurking Horror,
//! sound edition (`stories/lurkinghorror-r221-s870918-sound.zblorb`, release
//! 221 / serial 870918) — a v3 Z-code game packaged in a Blorb container that
//! also carries real SOUND resources, the "a Blorb with sound" format bucket
//! the quest names explicitly. Mirrors `walkthroughs.rs`'s Photopia shape
//! (SQ-1600 Phase 1): boot via the library session API (`GameSession`/
//! `Engine`, no terminal), extracting the Z-code from the `.zblorb`'s `Exec`
//! chunk via `app::hints::extract_story` (the same helper 20+ other suites
//! use for Blorb-wrapped fixtures), drive a committed, hand-verified command
//! script (`tests/fixtures/walkthroughs/lurking_horror.txt`) to the game's
//! own real winning ending, exercise an in-game `@save`/`@restore` round
//! trip, a host Save State/Restore State round trip and a `restart` reboot
//! partway through, check the mapper and hints machinery don't error while
//! being driven, and pin a normalized reference transcript
//! (`tests/fixtures/walkthroughs/lurking_horror.transcript`) that a future
//! regression must keep matching.
//!
//! **The audio hazard (SQ-1162) does not apply here.** This specimen is a
//! `.zblorb` with real `Snd ` resources, and CLAUDE.md's "thread-affine OS
//! handles" section warns that constructing a real `audio::AudioBackend` in a
//! test can crash or hang the whole binary. Traced the call graph: a plain
//! `GameSession` (`session.rs`) never constructs an `AudioBackend` itself —
//! it only records `@sound_effect` calls into `TurnResult.sounds` for a HOST
//! to act on. The only two call sites of `audio::AudioBackend::new` in this
//! crate are `picker_ui.rs`'s settings-screen sound preview and
//! `host::sound::default_sound_sink`, which `AppState::play_turn_sounds` /
//! `play_glulx_sound_ops` reach only when driving a full `AppState` — never
//! from the bare `GameSession`/`Engine` API this suite uses. So driving this
//! story here, however many `@sound_effect` calls it makes, never opens a
//! real device and needs no `disable_output_for_tests()` guard.
//!
//! v3 Z-code carries the automatic status line ("Wet Tunnel score=85
//! turns=420" — see `status()` below), unlike Photopia's v5 (no engine score
//! at all — see `walkthroughs.rs`'s own doc comment) — so, unlike that
//! suite, the acceptance here pins a real score: **the game's own real
//! ending is a perfect win, "Your score is 100 of a possible 100, in 440
//! moves... President of the Institute."** It does not raise `has_quit()` on
//! its own — like most Infocom endings, it drops into a
//! RESTART/RESTORE/QUIT prompt and waits, which the acceptance test reads as
//! a live `Line` prompt, not a quit.
//!
//! The command script was reconstructed empirically: booting the real game
//! (through its `.zblorb` container, not a bare `.z*` file) and confirming
//! every command's actual printed reply. The IF Archive's own published
//! solutions for this title (`if-archive/infocom/hints/solutions/
//! lurkinghorror.{step1,step2,step3,step4,txt}`, the `.txt` being Scorpia's
//! 1987 prose walkthrough) were used as a GUIDE ONLY, exactly the way
//! `photopia.sol` guided (never transcribed into) that suite's script — every
//! line here is this project's own derived command list, confirmed against
//! this exact release's live parser, not a copy of that prose. Several real,
//! non-obvious mechanics surfaced only by playing it live and are worth
//! flagging for a future maintainer: the maintenance man is evaded (wax the
//! floor, then flee), never fought; the reanimated hand is silently dropped
//! by a carry-capacity limit at Inside Dome if you're over-encumbered when
//! `take hand` is submitted (`take hand` fails with no fault, and everything
//! downstream that depends on carrying the hand — the urchin trade — then
//! silently fails too, which is why the script sheds bulky items before that
//! step); the Subbasement↔Tomb crack rejects the metal flask/fire axe/bolt
//! cutter one at a time ("too tight a fit"), so the script routes around it
//! via the Concrete-Box elevator shaft instead; the elevator-shaft chain
//! needs BOTH its ends anchored in one unbroken sequence (one end locked
//! around the exposed rod, the other never dropped before it reaches the
//! hook) before the tension that snaps the rod free can build at all; and
//! cutting the Inner Lair's power line needs three `cut line with axe`
//! strikes, not one, before it parts.

use std::path::Path;

use app::archive::{self, Meta, SaveTrigger, SessionRecord};
use app::config::Config;
use app::engine::{Engine, StatusField, StatusModel};
use app::hints::{self, extract_story, LoadedStory};
use app::hints::HintStory;
use app::host::hints::{available, open, HintAvailability};
use app::ifid::compute_ifid;
use app::session::{
    apply_item_observations, apply_turn, status_model_from_machine, DeathWatch, GameSession, InputKind, PendingIo,
    TurnResult,
};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// Which script command the mid-script persistence checks fire after: the
/// game's own `save` verb is confirmed to accept there (a `Line` prompt) and
/// the game is past every turn-critical stretch (the Inside Dome creature
/// ambush, the Concrete Box darkness/crowbar trap, the pentagram-escape
/// timing, the Inner Lair endgame) — a genuinely quiet moment deep in the
/// Wet Tunnel maze, just past the wire-cut, before the slime-curtain flask
/// trick. Verified empirically: `save` there yields `PendingIo::Save`.
const CHECKPOINT_AFTER_COMMAND: usize = 415;

/// A generous cap over the committed script's 436 real commands — large
/// enough that a legitimate run never gets near it, small enough that a
/// script stuck resubmitting the same prompt forever fails loudly instead of
/// hanging CI.
const MAX_TURNS: usize = 600;

fn boot_lurking_horror() -> Option<(Vec<u8>, GameSession)> {
    let path = fixture_path("lurkinghorror-r221-s870918-sound.zblorb");
    let raw = std::fs::read(&path).ok()?;
    let loaded = extract_story(raw).expect("the sound-edition zblorb must extract a runnable executable");
    let bytes = match loaded {
        LoadedStory::ZCode(b) => b,
        other => panic!("expected a Z-code executable inside the zblorb, got {other:?}"),
    };
    let session = GameSession::new_with_trace(
        bytes.clone(), true, true, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("The Lurking Horror should boot without a ZError");
    Some((bytes, session))
}

fn read_script() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("tests/fixtures/walkthroughs/lurking_horror.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("committed walkthrough script must be readable at {}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Submit one script command, routing it to a keypress when the game is at a
/// single-key (Char) prompt, and as an ordinary typed line otherwise —
/// mirrors `walkthroughs.rs`'s `submit_command`.
fn submit_command(session: &mut GameSession, cmd: &str) -> TurnResult {
    match session.pending_input() {
        InputKind::Char => session.submit_char(cmd.as_bytes().first().copied().unwrap_or(b' ')),
        _ => session.submit(cmd),
    }
}

/// The v3 automatic status line, read live off the machine (never inferred
/// from printed text) — `status_model_from_machine` for v1-3 always answers
/// `StatusModel::Classic`.
fn status(session: &GameSession) -> String {
    match status_model_from_machine(&session.machine) {
        StatusModel::Classic { location, right: StatusField::ScoreTurns { score, turns } } => {
            format!("[{location} | score={score} turns={turns}]")
        }
        StatusModel::Classic { location, right: StatusField::Time { hours, minutes } } => {
            format!("[{location} | time={hours}:{minutes:02}]")
        }
        StatusModel::HostManaged => "[hostmanaged]".to_string(),
    }
}

/// The live turn counter off the v3 status line, used to prove a "drift"
/// between save and restore really advanced the game (mirrors the intent of
/// `walkthroughs.rs`'s room-name drift check, but reads the engine's own
/// score/turns fact instead of a room name — the Wet Tunnel maze plausibly
/// reuses one room description for several distinct rooms, which would make
/// a printed-name comparison unreliable here).
fn turns(session: &GameSession) -> u16 {
    match status_model_from_machine(&session.machine) {
        StatusModel::Classic { right: StatusField::ScoreTurns { turns, .. }, .. } => turns,
        other => panic!("expected a classic score/turns status line, got {other:?}"),
    }
}

/// Collapse whitespace so the determinism/golden-file comparisons don't care
/// about incidental line wrapping — matches `walkthroughs.rs`'s `norm`.
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Exercises the in-game `@save`/`@restore` round trip (through the host
/// archive path) and the host Save State/Restore State round trip, at the
/// Wet Tunnel checkpoint. Leaves `session` exactly where it found it, so the
/// caller's script loop can resume as if this had never run.
fn mid_script_persistence_checks(session: &mut GameSession) {
    let before_turns = turns(session);

    // ---- In-game @save / @restore, through the host archive path ------------
    let r = session.submit("save");
    assert_eq!(
        r.pending_io,
        Some(PendingIo::Save),
        "The Lurking Horror's SAVE verb must bubble a host Save request: {:?}",
        r.transcript
    );

    let ingame = app::persist_files::game_save_bytes(session, SaveTrigger::Ingame);
    let dir = app::scratch_dir("sq1600-lurkinghorror-ingame-save");
    app::persist_files::save_named(
        &dir,
        "SQ1600-LURKINGHORROR",
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
        Some("Wet Tunnel".to_string()),
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
        .expect("a real Lurking Horror @save archive must restore through the host restore path");
    assert!(!session.has_quit(), "the restore left the session alive");
    assert_eq!(turns(session), before_turns, "restored to the save point's turn count");
    let _ = std::fs::remove_dir_all(&dir);

    // ---- Host Save State / Restore State --------------------------------------
    let es = Engine::save_state(session);
    let ss_dir = app::scratch_dir("sq1600-lurkinghorror-host-save");
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
            location: Some("Wet Tunnel".to_string()),
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

    // Drift far enough that the restore below proves something real: a `wait`
    // always advances the v3 turn counter by one, regardless of whether the
    // maze's room NAME changes (it may not — a maze commonly reuses one
    // description for several distinct rooms).
    let drift = session.submit("wait");
    assert!(drift.fault.is_none(), "the drift turn must not fault: {:?}", drift.fault);
    let drift_turns = turns(session);
    assert!(
        drift_turns > before_turns,
        "the drift really advanced the game: {before_turns} -> {drift_turns}"
    );

    let ac = archive::load_archive(&ss_path).expect("load_archive");
    let _ = std::fs::remove_dir_all(&ss_dir);
    Engine::restore_state(session, &ac.engine_save()).expect("restore_state (Quetzal memory/stack/pc)");
    let _ = session.take_transcript();
    assert!(!session.has_quit());
    assert_eq!(
        turns(session),
        before_turns,
        "the host Save State/Restore State round trip lands back at the checkpoint's turn count"
    );
}

/// After a `restart` command, answer the game's Line confirmation up to a
/// few turns, collecting the reboot output. Bounded — never loops forever.
/// Mirrors `walkthroughs.rs`'s `confirm_restart`.
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

/// Boots a completely SEPARATE Lurking Horror session (never the one the
/// main walkthrough is driving) and proves `restart` reboots in place rather
/// than quitting the app — mirrors `walkthroughs.rs`'s `check_restart_reboots`.
/// Verified empirically: `restart` here itself prints the closing score line
/// and "Do you wish to restart? (Y is affirmative):", a `Line` prompt that
/// accepts "yes".
fn check_restart_reboots(bytes: &[u8]) {
    let mut session = GameSession::new_with_trace(
        bytes.to_vec(), true, true, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("The Lurking Horror should boot for the restart probe");
    let boot_banner = norm(&session.take_transcript());
    assert!(
        boot_banner.contains("THE LURKING HORROR"),
        "sanity: the real boot banner: {boot_banner:?}"
    );

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
        "restart must re-run The Lurking Horror from the opening (boot banner should reappear)\n  probe: {probe:?}\n  reboot: {reboot_text:?}"
    );
}

/// Confirms the hints machinery does not error while being asked about this
/// story. Unlike Photopia's suite (which pins `HintAvailability::None`
/// because it carries no hint sidecar anywhere), this fixture's answer
/// depends on what else happens to sit in the local, gitignored `stories/`
/// directory beside the `.zblorb` — `stories/lurkinghorror-r219-s870912.z3`
/// (a different, non-sound release of the SAME game) resolves as a
/// same-stem sibling, which is real, accurate behaviour of THIS machine's
/// `stories/` snapshot, not a genuine bundled InvisiClues file for the
/// title, and not guaranteed to be present on every machine that runs this
/// suite. So the assertion here is deliberately weaker than Photopia's: only
/// that `available()` and `open()` — which SQ-1586's own doc promises can
/// never disagree — actually agree, and that neither panics.
fn check_hints_machinery_agrees(story_path: &Path, story_bytes: &[u8]) {
    let ifid = compute_ifid(story_bytes);
    let home = app::scratch_dir("sq1600-lurkinghorror-hint-index");
    let index = hints::load_hint_index(&home);

    let avail = available(story_path, HintStory::new(&ifid, ""), &index);
    let cfg = Config::default();
    let opened = open(story_path, HintStory::new(&ifid, ""), &index, &[], &cfg, None);
    match avail {
        HintAvailability::None | HintAvailability::Choose(_) => {
            let result = opened.expect("no hint source is not an error");
            assert!(result.is_none(), "open() finds nothing, exactly as available() said");
        }
        HintAvailability::Available => {
            // A source resolved (a same-stem sibling release on this
            // machine's `stories/`, per the doc comment above) — `open()`
            // booting it, declining it, or erroring on it are all legitimate
            // outcomes for a file that is not really an InvisiClues hint
            // file; the only thing under test is that neither call panics.
            eprintln!("hints: available() said Available; open() -> {opened:?}");
        }
    }
}

struct PlayOutcome {
    /// The ordinary walkthrough transcript — every script command's printed
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

    PlayOutcome { transcript, mapper }
}

#[test]
fn lurking_horror_reaches_its_ending_with_no_fault_and_a_sane_mapper_and_hints_state() {
    let Some((bytes, session)) = boot_lurking_horror() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/lurkinghorror-r221-s870918-sound.zblorb");
        return;
    };
    let story_path = fixture_path("lurkinghorror-r221-s870918-sound.zblorb");
    let commands = read_script();
    assert!(commands.len() > 400, "premise: the committed script is the full walkthrough, not a stub");

    let outcome = play_full_script(session, &commands);
    let normalized = norm(&outcome.transcript);

    assert!(
        normalized.contains("Your score is 100 of a possible 100"),
        "The Lurking Horror's real ending is a perfect score — got tail: {:?}",
        normalized.chars().rev().take(400).collect::<String>().chars().rev().collect::<String>()
    );
    assert!(
        normalized.contains("President of the Institute"),
        "the perfect-score grading line names the top rank: {normalized:?}"
    );

    // Mapper: unlike Photopia (a non-Inform narrative work with no location
    // global at all), The Lurking Horror is a conventional adventure game
    // with a real player-object/room-parent structure, so the mapper's
    // per-turn location detection has real signal to work with throughout —
    // the acceptance here is that driving all 436 turns through `apply_turn`
    // never panicked AND produced a genuinely populated graph.
    assert!(
        outcome.mapper.graph.rooms().count() > 0,
        "The Lurking Horror gives the mapper real location signal; an empty graph would mean detection broke"
    );

    check_restart_reboots(&bytes);
    check_hints_machinery_agrees(&story_path, &bytes);

    // ---- SQ-1647: item-tracking sanity (deliberately no exact item list/count pinning —
    // see the quest; this repo's own `synonym_groups.tsv` precedent is that pinned lines
    // break on unrelated changes) ----
    //
    // Empirically (SQ-1647): the full 436-command playthrough tracks 79 distinct items. A
    // generous floor well under that, not the exact figure, so an unrelated future script
    // edit doesn't need to touch this assertion — the point is only that item detection
    // didn't quietly stop working.
    let items: Vec<_> = outcome.mapper.graph.items().collect();
    assert!(
        items.len() > 30,
        "The Lurking Horror is a real Inform-shaped game with plenty of scenery/inventory objects; \
         a full playthrough should track well over a couple dozen, got {}",
        items.len()
    );
    let vanished = items
        .iter()
        .filter(|(_, r)| matches!(r.last_seen, mapper::graph::ItemLocation::Vanished { .. }))
        .count();
    assert!(
        vanished * 10 < items.len() * 9,
        "{vanished} of {} tracked items ended Vanished — that's suspiciously close to \"everything\", \
         the shape a regression that vanishes on every sweep would produce (empirically this specimen sees ~4%)",
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
fn lurking_horror_walkthrough_is_deterministic_and_matches_the_pinned_transcript() {
    let Some((_, session1)) = boot_lurking_horror() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/lurkinghorror-r221-s870918-sound.zblorb");
        return;
    };
    let commands = read_script();

    let run1 = norm(&play_full_script(session1, &commands).transcript);

    let Some((_, session2)) = boot_lurking_horror() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/lurkinghorror-r221-s870918-sound.zblorb");
        return;
    };
    let run2 = norm(&play_full_script(session2, &commands).transcript);

    assert_eq!(
        run1, run2,
        "two fresh runs of the same script must produce byte-identical (normalized) transcripts — \
         this is the falsifiable claim that this walkthrough's win path has no `@random` dependency \
         zvm's fixed-seed boot doesn't already cover, not the walkthrough author's say-so"
    );

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest.join("tests/fixtures/walkthroughs/lurking_horror.transcript");
    if std::env::var_os("SQ1600_REGEN_GOLDEN").is_some() {
        std::fs::write(&golden_path, format!("{run1}\n")).expect("regenerate the pinned transcript");
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("committed reference transcript must be readable at {}: {e}", golden_path.display()));
    assert_eq!(
        run1,
        golden.trim_end(),
        "the current run's normalized transcript no longer matches the pinned reference \
         (crates/app/tests/fixtures/walkthroughs/lurking_horror.transcript) — if this is an intentional \
         behaviour change, regenerate the pinned file and explain why in the commit"
    );
}
