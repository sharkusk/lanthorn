//! SQ-1600: a headless walkthrough-regression harness for **Anchorhead: the
//! Illustrated Edition** (`stories/Anchorhead.gblorb`, Release 1 / Serial
//! number 171017, Inform 7 build 6M62 — a 2018 ground-up rewrite of Michael
//! Gentry's 1998 classic), the Glulx-graphics lane of this quest. Mirrors
//! `walkthroughs.rs`'s Photopia shape (SQ-1600 Phase 1): a committed,
//! hand-verified command script (`tests/fixtures/walkthroughs/anchorhead.txt`)
//! driven through the library session API (`GlulxSession`/`Engine`, no
//! terminal), an in-game `@save`/`@restore` round trip, a host Save
//! State/Restore State round trip, a `restart`-reboot probe, a
//! mapper/hints sanity check, and a pinned, deterministic reference
//! transcript.
//!
//! The filename keeps the "graphics_sound" name from this lane's original
//! brief even though the "sound" half of that bucket was dropped partway
//! through (see below) — renaming mid-lane was declined on purpose so the
//! module path doesn't churn for a cosmetic reason.
//!
//! # Why Anchorhead, and why it carries no sound
//!
//! `Anchorhead.gblorb` was picked over `Kerkerkruip.gblorb` (a roguelike with
//! genuinely random, non-repeatable runs — unsuitable for a fixed winning
//! script, and excluded from this whole quest for that reason) after
//! confirming empirically, via `crates/blorb::Blorb::parse`, that NONE of the
//! Glulx titles under `stories/` carry both `Pict` and `Snd` resources except
//! Kerkerkruip and this one, and Anchorhead carries only `Pict` (no `Snd` at
//! all). The user's own decision (this session) was to drop the "and sound"
//! half of the bucket entirely rather than force a specimen fit — sound
//! coverage for this wave rests on the sibling Lurking Horror lane
//! (`walkthrough_lurking_horror.rs`, Z-machine) instead.
//!
//! # This is a DELIBERATE, DOCUMENTED PARTIAL result — not the game's win
//!
//! The committed script (629 commands) is an empirically verified, no-fault
//! stretch of Anchorhead covering: Day One (breaking into the real estate
//! office's file room via the fire-escape/umbrella-hook, finding the Verlac
//! file by name after hearing it on the answering machine, meeting Michael at
//! the university library, and walking up to the house after the car breaks
//! down); Day Two (exploring the house — cellar, wine cellar mural, attic,
//! child's bedroom, library — visiting the university library, the
//! courthouse records department, the curiosity shop for the amulet, and the
//! family mausoleum for a dog's skull hidden by a spider web; discovering
//! Michael missing from his study, the secret passage behind the fireplace
//! spheres into "Behind the Walls" up to the Observatory's comet charts, and
//! trading the amulet to a drunken vagrant at the Vacant Lot — after a long
//! dialogue tree about Anna, Edward and William Verlac — for a Public Works
//! key); and Day Three (the sky's "swirling hole," Michael's possessed
//! wandering, and descending through a street manhole into the sewers,
//! finding a second antique key wedged against a locked grating).
//!
//! **It stops there on purpose, not at a technical wall.** Unlike the Zork
//! Zero lane's Double Fanucci (an actual technical blocker — cards rendered
//! only as pictures, no text representation the session API can see), nothing
//! here refused to yield to further play: the walkthrough was still making
//! steady, solvable puzzle progress (the office, the mausoleum, the vagrant's
//! dialogue gate, the manhole) right up to the point it stops. This is
//! purely a time/effort scope decision by the user (this session, following
//! the same pattern as how the Zork Zero lane was finalized): reaching the
//! game's real winning ending — which the game's own IFDB metadata and this
//! session's derivation both confirm is still a long way off (an unsolved
//! church-cellar padlock, an unopened mill gate, an unidentified second
//! antique key, an unexplored sewer grating, the whole zodiac/temple puzzle,
//! the boat trip, the factory capture, and the entire Final Night) — was
//! accepted as out of scope for this lane. The acceptance criterion below is
//! therefore reaching this stopping point with no fault, NOT a game-winning
//! ending — see `anchorhead_walkthrough_reaches_the_sewer_with_no_fault_and_a_populated_map`.
//!
//! Derived empirically turn-by-turn against the real interpreter (never
//! copied from a walkthrough): general shape/beats cross-checked against the
//! IF Archive's "Anchorhead (2017) - Solution" listing page and IFDB/Steam
//! community discussion threads (WebSearch/WebFetch, conceptual guidance
//! only — WebFetch itself refused to reproduce the solution's command text
//! verbatim when asked, which is the intended behaviour) and this session's
//! own knowledge of the 1998 original, where it held (several puzzles in this
//! 2018 ground-up rewrite differ from the original — e.g. the house keys come
//! from a file-cabinet search after the answering machine, not a picked desk
//! lock, which this session's own blind attempt at "something long and thin"
//! discovered was a dead end).
//!
//! # `GlulxSession` input/audio/RNG findings (SQ-1600, this session)
//!
//! - `GlulxSession::submit()` already dispatches to a single keypress
//!   internally when `pending_input() == InputKind::Char` (SQ-1270's
//!   documented contract on the `Engine::submit` impl) — unlike Photopia's
//!   Z-machine `GameSession`, there is no need for a separate
//!   Char-vs-Line `submit_char` dispatch helper here.
//! - Driving a bare `GlulxSession` cannot open a real audio device: grep
//!   confirms `AudioBackend::new` is constructed only in `picker_ui.rs` (the
//!   TUI picker path) and referenced only in a `state.rs` doc comment about
//!   `AppState::default()`'s test guard — `glulx_session.rs` never
//!   constructs one, and `sound_enabled` is purely a VM-side Glk
//!   sound-gestalt flag. No `disable_output_for_tests()` call is needed here
//!   (moot anyway: Anchorhead carries no `Snd` resources to play).
//! - `gvm`'s PRNG (`Machine`, xorshift32) starts from a fixed `DEFAULT_SEED`
//!   unless something calls `set_rng_seed`/`@setrandom` with a nonzero seed
//!   or `@setrandom 0` (real OS entropy). `GlulxSession::new`'s every
//!   non-launcher caller passes `random_seed: None`, leaving the fixed
//!   default untouched. Confirmed empirically for this specimen too: two
//!   independent derivation sessions that rebooted from scratch mid-session
//!   produced byte-identical banners and identical `Engine::rng_seed()`
//!   (2337545247) every time — and the game DOES draw real randomness (the
//!   Twisting Lane's "you take several blind turns and emerge onto..." and
//!   the fireplace's "Behind the Walls" passage both route unpredictably
//!   from the player's perspective), which is exactly why the two-fresh-boot
//!   determinism test below is the actual falsifiable claim, not a
//!   walkthrough author's say-so — a script derived against one seeded
//!   trajectory is NOT safely shortened by re-deriving an equivalent-looking
//!   shorter path from a different point in the RNG stream: this session
//!   tried exactly that against the "Behind the Walls" maze (replacing ~85
//!   lines of empirically-found wandering with the shorter sequence that had
//!   worked when tested from a fresh, shorter prefix) and it broke — the
//!   maze's random room selection depends on how many prior `random` draws
//!   the ENTIRE preceding playthrough made, not just the immediate prefix.
//!   The committed script's maze-traversal section is therefore left as the
//!   verbose, empirically-real sequence, not a hand-optimized one.

use std::path::Path;

use app::archive::{self, Meta, SaveTrigger, SessionRecord};
use app::config::Config;
use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::hints;
use app::host::hints::{available, open, HintAvailability};
use app::ifid::compute_ifid;
use app::session::{apply_item_observations, apply_turn, DeathWatch, InputKind, PendingIo, TurnResult};
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// Which script command the mid-script persistence checks fire after: the
/// committed script's LAST command (the 629th, `up`) — climbing back out of
/// the sewer manhole to "Under the Bridge", this walkthrough's final,
/// verified, no-fault stopping point (see the module doc comment). Placed at
/// the very end rather than mid-script (unlike the Photopia/Zork Zero
/// precedents) because this lane's script is itself the full extent of the
/// verified partial walkthrough — there is no later "resume script" segment
/// to come back to, so the checks simply run once the walkthrough is done.
const CHECKPOINT_AFTER_COMMAND: usize = 629;

/// A generous cap over the committed script's 629 real commands.
const MAX_TURNS: usize = 700;

fn boot_anchorhead() -> Option<(Vec<u8>, GlulxSession)> {
    let path = fixture_path("Anchorhead.gblorb");
    let bytes = std::fs::read(&path).ok()?;
    let blorb = blorb::Blorb::parse(bytes.clone()).expect("Anchorhead.gblorb parses as a Blorb");
    let (kind, exec) = blorb.executable().expect("Anchorhead.gblorb carries an executable chunk");
    assert_eq!(kind, blorb::ExecKind::Glulx, "Anchorhead.gblorb is a Glulx blorb");
    let session = GlulxSession::new(exec.to_vec(), 80, 24, true, true, true, (1.0, 1.0), Some(blorb), &[])
        .expect("Anchorhead should boot without a GError");
    Some((bytes, session))
}

fn read_script() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("tests/fixtures/walkthroughs/anchorhead.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("committed walkthrough script must be readable at {}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Submit one script command. `GlulxSession::submit` already delivers a
/// single keypress when `pending_input()` is `Char` (see the module doc
/// comment), so the only special case left is `Event` — an empty resume,
/// never the literal script text.
fn submit_command(session: &mut GlulxSession, cmd: &str) -> TurnResult {
    match session.pending_input() {
        InputKind::Event => session.submit(""),
        _ => session.submit(cmd),
    }
}

/// Collapse whitespace so the determinism/golden-file comparisons below don't
/// care about incidental line wrapping — matches the repo's existing
/// precedent (`restart_reboots_in_place.rs`'s `norm`, `walkthroughs.rs`'s).
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True if `look`'s reply shows "Under the Bridge" — the mid-script
/// checkpoint's own precondition/postcondition probe, exactly like
/// `walkthroughs.rs`'s `looking_shows_landing_site`.
fn looking_shows_under_the_bridge(session: &mut GlulxSession) -> bool {
    session.submit("look").transcript.contains("Under the Bridge")
}

/// Exercises the in-game `@save`/`@restore` round trip (through the host
/// archive path, mirroring `glulx_ingame_save_host_restore.rs`) and the host
/// Save State/Restore State round trip (mirroring `walkthroughs.rs`'s
/// non-v6 shape: bare `Engine::save_state`/`archive::save_archive_meta_pics`/
/// `archive::load_archive`/`Engine::restore_state`, no v6 screen table or
/// pictures payload — Glulx has neither of those Z-machine-v6-specific
/// concepts). Leaves `session` exactly where it found it — Under the Bridge,
/// pending a Line command — so the caller can resume as if this had never
/// run (a no-op here in practice, since this is the script's last command).
fn mid_script_persistence_checks(session: &mut GlulxSession) {
    assert!(
        looking_shows_under_the_bridge(session),
        "checkpoint precondition: back Under the Bridge with the manhole open"
    );

    // ---- In-game @save / @restore, through the host archive path ------------
    let r = session.submit("save");
    assert_eq!(
        r.pending_io,
        Some(PendingIo::Save),
        "Anchorhead's SAVE verb must bubble a host Save request: {:?}",
        r.transcript
    );

    let ingame = app::persist_files::game_save_bytes(session, SaveTrigger::Ingame);
    let dir = app::scratch_dir("sq1600-anchorhead-ingame-save");
    app::persist_files::save_named(
        &dir,
        "SQ1600-ANCHORHEAD",
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
        Some("Under the Bridge".to_string()),
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

    let bytes = archive::read_quetzal_from_file(&path).expect("archive's game.glksave reads back");
    session
        .restore_game_save(&bytes)
        .expect("a real Anchorhead @save archive must restore through the host restore path");
    assert!(!session.has_quit(), "the restore left the session alive");
    assert!(looking_shows_under_the_bridge(session), "restored to the save point");
    let _ = std::fs::remove_dir_all(&dir);

    // ---- Host Save State / Restore State --------------------------------------
    let es = Engine::save_state(session);
    let ss_dir = app::scratch_dir("sq1600-anchorhead-host-save");
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
            location: Some("Under the Bridge".to_string()),
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

    // Drift far enough that the restore below proves something real: climb
    // back down into the sewer, which leaves "Under the Bridge" entirely.
    let drift = session.submit("down");
    assert!(
        !drift.transcript.contains("Under the Bridge"),
        "the drift really left the checkpoint: {:?}",
        drift.transcript
    );

    let ac = archive::load_archive(&ss_path).expect("load_archive");
    let _ = std::fs::remove_dir_all(&ss_dir);
    Engine::restore_state(session, &ac.engine_save()).expect("restore_state (Quetzal memory/stack/pc)");
    let _ = session.take_transcript();
    assert!(!session.has_quit());
    assert!(
        looking_shows_under_the_bridge(session),
        "the host Save State/Restore State round trip lands back at the checkpoint"
    );
}

/// After a `restart` command, answer the game's Line confirmation
/// ("Are you sure you want to restart?") and absorb the title-card replay —
/// `GlulxSession::submit`'s own Char dispatch means the SAME "yes" text
/// works whether the current prompt is the Line confirmation or one of the
/// Char-prompt title cards that follow it (delivered as its first
/// character). Mirrors `restart_reboots_in_place.rs`'s `confirm_restart`
/// shape, simplified by that Glulx dispatch behaviour.
fn confirm_restart(session: &mut GlulxSession) -> String {
    let mut collected = String::new();
    for _ in 0..6 {
        let result = submit_command(session, "yes");
        assert!(!result.quit, "restart must NOT quit the app: {:?}", result.transcript);
        assert!(result.fault.is_none(), "restart faulted: {:?}", result.fault);
        collected.push_str(&result.transcript);
        if !result.transcript.trim().is_empty() {
            break;
        }
    }
    collected
}

/// Boots a completely SEPARATE Anchorhead session (never the one the main
/// walkthrough is driving), presses through the opening title cards into
/// real gameplay, and proves `restart` reboots in place rather than quitting
/// the app — mirrors `restart_reboots_in_place.rs`'s pattern.
fn check_restart_reboots() {
    let Some((_bytes, mut session)) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored story missing at {}", fixture_path("Anchorhead.gblorb").display());
        return;
    };
    let _ = session.take_transcript(); // the boot banner itself is empty for this title-card sequence
    let opening = norm(&submit_command(&mut session, "a").transcript);
    assert!(
        opening.contains("Welcome to Anchorhead"),
        "sanity: the real opening card's closing line: {opening:?}"
    );
    let _ = submit_command(&mut session, "s"); // the epigraph card
    let gameplay = norm(&submit_command(&mut session, "v").transcript);
    assert!(
        gameplay.contains("Outside the Real Estate Office"),
        "sanity: the real Day One opening room: {gameplay:?}"
    );

    let r = session.submit("restart");
    assert!(!r.quit, "the restart command itself must not quit");
    assert!(r.fault.is_none(), "restart faulted: {:?}", r.fault);
    let mut reboot_text = norm(&r.transcript);
    reboot_text.push(' ');
    reboot_text.push_str(&norm(&confirm_restart(&mut session)));

    assert!(!session.has_quit(), "session must stay alive after restart");
    assert!(
        reboot_text.contains("Welcome to Anchorhead"),
        "restart must re-run Anchorhead from the opening (the opening card's closing line should reappear)\n  \
         reboot: {reboot_text:?}"
    );
}

/// Mirrors `walkthroughs.rs`'s "no hint sidecar" case: Anchorhead carries no
/// InvisiClues-style hint file anywhere lanthorn looks (`IZM_HINTS`/
/// `SLAG_HINTS` in `hints.rs` name no entry for it), so the point here is
/// only that the check machinery itself does not error when asked about it.
fn check_hints_machinery_agrees(story_path: &Path, story_bytes: &[u8]) {
    let ifid = compute_ifid(story_bytes);
    let home = app::scratch_dir("sq1600-anchorhead-empty-hint-index");
    let index = hints::load_hint_index(&home);

    assert_eq!(
        available(story_path, &ifid, "", &index),
        HintAvailability::None,
        "Anchorhead carries no hint sidecar anywhere lanthorn looks"
    );
    let cfg = Config::default();
    let result = open(story_path, &ifid, "", &index, &[], &cfg).expect("no hint source is not an error");
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
/// Anchorhead is a real Inform 7 location-tracked game, so the mapper is
/// expected to populate a genuine room graph.
fn play_full_script(mut session: GlulxSession, commands: &[String]) -> PlayOutcome {
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
            "turn {} ({cmd:?}) faulted: {:?}\ntranscript so far (last 2000 chars): {}",
            i + 1,
            result.fault,
            &transcript[transcript.len().saturating_sub(2000)..]
        );
        assert!(
            !result.quit,
            "turn {} ({cmd:?}) quit the game — the committed script must never reach a death/quit prompt",
            i + 1
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
fn anchorhead_walkthrough_reaches_the_sewer_with_no_fault_and_a_populated_map() {
    let Some((bytes, session)) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/Anchorhead.gblorb");
        return;
    };
    let path = fixture_path("Anchorhead.gblorb");
    let commands = read_script();
    assert!(commands.len() > 500, "premise: the committed script is the full verified partial walkthrough, not a stub");

    let outcome = play_full_script(session, &commands);

    // NOT a win — see the module doc comment. The acceptance here is reaching
    // this session's documented stopping point, with no fault and no quit
    // anywhere along the way.
    assert!(!outcome.quit, "the partial walkthrough must not quit — it deliberately stops short of the real ending, not at a death/game-over");
    assert!(
        norm(&outcome.transcript).contains("Under the Bridge"),
        "the script's last real move must climb back out of the sewer to Under the Bridge: {:?}",
        outcome.transcript
    );

    // Mapper: Anchorhead is a real Inform 7 location-tracked game (unlike
    // Photopia's Phase-1 empty-graph shape) — driving the full script
    // populates a genuine room graph.
    let room_count = outcome.mapper.graph.rooms().count();
    assert!(room_count > 20, "expected a well-populated map after 629 turns across two days of the town and house, got {room_count} rooms");
    for name in ["Outside the Real Estate Office", "Foyer", "Town Square", "Curiosity Shop", "Under the Bridge"] {
        assert!(
            outcome.mapper.graph.rooms().any(|r| r.name == name),
            "mapper never discovered {name:?} — rooms seen: {:?}",
            outcome.mapper.graph.rooms().map(|r| r.name.as_str()).collect::<Vec<_>>()
        );
    }

    check_restart_reboots();
    check_hints_machinery_agrees(&path, &bytes);

    // ---- SQ-1647: item-tracking sanity (deliberately no exact item list/count pinning —
    // see the quest; this repo's own `synonym_groups.tsv` precedent is that pinned lines
    // break on unrelated changes) ----
    //
    // Empirically (SQ-1647): the full 629-command partial playthrough tracks 30 distinct
    // items. A generous floor well under that, not the exact figure, so an unrelated future
    // script edit doesn't need to touch this assertion — the point is only that item
    // detection didn't quietly stop working.
    let items: Vec<_> = outcome.mapper.graph.items().collect();
    assert!(
        items.len() > 10,
        "Anchorhead is a real Inform 7 game with plenty of inventory objects; a partial \
         playthrough across two days of the town and house should track well over a dozen, got {}",
        items.len()
    );

    // SQ-1640 regression, exercised across the WHOLE walkthrough rather than the single-turn
    // arrival `item_tracking.rs`'s own case covers: the script revisits the Garbage-Choked
    // Alley repeatedly (`southeast` appears 16 times in anchorhead.txt), so this is a much
    // longer-running exercise of the same `gvm::objects::ParseNames::is_scenery_or_door`
    // filter. None of the alley's own reported pure-scenery/backdrop nouns should ever have
    // entered the registry.
    let reported_scenery = [
        "alley entrance doors",
        "buildings",
        "cardboard boxes",
        "garbage can",
        "ground",
        "metal ladder",
        "rain",
        "sky",
        "wooden fence",
    ];
    for noun in reported_scenery {
        assert!(
            !items.iter().any(|(_, r)| r.name.contains(noun)),
            "{noun:?} is pure scenery/backdrop (SQ-1640) and must never enter the item registry: {:?}",
            items.iter().map(|(_, r)| r.name.as_str()).collect::<Vec<_>>()
        );
    }

    let vanished = items
        .iter()
        .filter(|(_, r)| matches!(r.last_seen, mapper::graph::ItemLocation::Vanished { .. }))
        .count();
    assert!(
        vanished * 10 < items.len() * 9,
        "{vanished} of {} tracked items ended Vanished — that's suspiciously close to \"everything\", \
         the shape a regression that vanishes on every sweep would produce (empirically this specimen sees \
         ~47%, the highest of any walkthrough lane — Anchorhead's inventory churns heavily: keys get used, \
         clothes get changed, tools get consumed)",
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
fn anchorhead_walkthrough_is_deterministic_and_matches_the_pinned_transcript() {
    let Some((_, session1)) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/Anchorhead.gblorb");
        return;
    };
    let commands = read_script();

    let run1 = norm(&play_full_script(session1, &commands).transcript);

    let Some((_, session2)) = boot_anchorhead() else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/Anchorhead.gblorb");
        return;
    };
    let run2 = norm(&play_full_script(session2, &commands).transcript);

    assert_eq!(
        run1, run2,
        "two fresh runs of the same script must produce byte-identical (normalized) transcripts — this is the \
         falsifiable claim that gvm's RNG is deterministically seeded here, not the walkthrough author's say-so. \
         Anchorhead DOES draw real randomness (the Twisting Lane and the 'Behind the Walls' passage both route \
         unpredictably from the player's perspective) — this confirms both runs draw the SAME sequence."
    );

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest.join("tests/fixtures/walkthroughs/anchorhead.transcript");
    if std::env::var_os("SQ1600_REGEN_GOLDEN").is_some() {
        std::fs::write(&golden_path, format!("{run1}\n")).expect("regenerate the pinned transcript");
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("committed reference transcript must be readable at {}: {e}", golden_path.display()));
    assert_eq!(
        run1,
        golden.trim_end(),
        "the current run's normalized transcript no longer matches the pinned reference \
         (crates/app/tests/fixtures/walkthroughs/anchorhead.transcript) — if this is an intentional \
         behaviour change, regenerate the pinned file and explain why in the commit"
    );
}
