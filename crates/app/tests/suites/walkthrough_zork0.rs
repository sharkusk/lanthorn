//! SQ-1600: a headless walkthrough-regression harness for **Zork Zero: The
//! Revenge of Megaboz** (`stories/zork0-r393-s890714.z6`, release 393 / serial
//! 890714 — pinned in `real_media_releases.rs`), the v6/graphical Z-code lane
//! of this quest. Mirrors `walkthroughs.rs`'s Photopia shape (SQ-1600 Phase 1)
//! for a structurally much bigger, v6, real-location-tracked specimen: a
//! committed, hand-verified command script (`tests/fixtures/walkthroughs/
//! zork0.txt`) driven through the library session API (`GameSession`/`Engine`,
//! no terminal, no render/pixel/screen-geometry assertions of any kind), an
//! in-game `@save`/`@restore` round trip, a host Save State/Restore State
//! round trip, a `restart`-reboot probe, a populated-mapper check, a
//! hints-machinery sanity check, and a pinned, deterministic reference
//! transcript.
//!
//! # This is a DELIBERATE, DOCUMENTED PARTIAL result — not the game's win
//!
//! The committed script (1266 commands) is the largest contiguous, empirically
//! verified stretch of Zork Zero reachable through this harness: the Prologue,
//! the Secret Wing (including the Tower of Bozbar / Tower of Hanoi puzzle,
//! solved twice — to the right peg then to the left — via an independently
//! derived optimal solution, not a copied one), the East Wing, the West Wing
//! (including Peggleboz and the goggles-assisted shell game), and the
//! Underground (the construction-site maze, the vault, the chessboard/knight
//! puzzle for the hardhat, the Orb Room, the anti-pit bomb, the lantern, the
//! screwdriver) — ending with the walkthrough sitting down at the card table
//! in the Port Foozle Casino, ready to play Double Fanucci.
//!
//! **It stops there on purpose.** Double Fanucci is a full graphical card
//! game: the actual hand of cards is rendered as **pictures**, with no text
//! representation anywhere the session-level API can see. This was confirmed
//! two ways before this scope boundary was accepted (SQ-1600 investigation,
//! 2026-09-27):
//!
//!  1. Every v6 window's text content, dumped at the moment cards are dealt,
//!     carries no card name/rank/suit anywhere — only the action-verb grid
//!     (DRAW/DISCARD/DIVIDE/REVERSE/TRUMP/UNDERTRUMP/COMBINE/PASS/OVERPASS/
//!     SINGLE-PLAY/DOUBLE-PLAY/MUTTONATE/CHEAT/RESIGN/IONIZE) is text at all.
//!  2. Turning on `Engine::set_trace_screen`/`take_screen_trace` (the screen-op
//!     trace, not rendering) during the deal DOES show real signal —
//!     `@draw_picture(number=N, ...)` calls clustered per card position (e.g.
//!     picture numbers 140/141/151/152/106/107 for two adjacent cards) — but
//!     turning that into actual card identities would mean reverse-engineering
//!     an undocumented picture-ID-to-card mapping specific to this game's Blorb
//!     resources: a meaningfully-sized side project on its own, not something
//!     any published walkthrough or format reference documents.
//!
//! This blocks the game's remaining critical path: winning Double Fanucci is
//! required for the broom, which is required for the Cell's cobwebs, which
//! hide the glass flask (1 of the 24 treasures the game's own win condition
//! needs), and the rest of Port Foozle (the Inquisition, the Room of Three
//! Doors, the shovel, the Outer Bailey chest) sits behind the same room. No
//! shortcut was found either — `CHEAT` just prompts an ordinary discard, and
//! `RESIGN` quits the hand rather than winning it.
//!
//! User decision (2026-09-27): accept this partial win-path rather than
//! attempt the picture-ID reverse-engineering. The acceptance criterion below
//! is therefore reaching the Casino table with no fault, NOT a game-winning
//! ending — see `zork0_walkthrough_reaches_the_double_fanucci_table_...`.
//!
//! Derived with the aid of, as a guide only (never transcribed) — every
//! command below was independently confirmed against the real interpreter,
//! turn by turn, before being committed:
//!   "Zork Zero walkthrough", eristic.net, archived 2021-05-08 via the Wayback
//!   Machine: <https://web.archive.org/web/20210508080538/http://www.eristic.net/games/infocom/zorkzero.html>
//! (a literal, command-precise walkthrough with running score totals). Two
//! numeric puzzle "solutions" quoted from it — the Tower of Bozbar's 63-move
//! sequences and the Peggleboz solitaire sequence — were cross-checked by
//! independently deriving the Tower of Bozbar solution from the standard
//! recursive Tower-of-Hanoi algorithm (it matched exactly, confirming it is
//! the unique optimal solution and not creative content) before either was
//! run against the real game.

use std::path::{Path, PathBuf};

use app::archive::{self, Meta, SaveTrigger, SessionRecord};
use app::config::Config;
use app::engine::Engine;
use app::engine_helpers::{apply_v6_pictures, v6_save_payload};
use app::hints;
use app::host::hints::{available, open, HintAvailability};
use app::ifid::compute_ifid;
use app::interpreter::InterpreterProfile;
use app::machine_boot::MachineBoot;
use app::native_font::FaceSet;
use app::session::{
    apply_item_observations, apply_turn, restore_screen, DeathWatch, GameSession, InputKind, PendingIo, TurnResult,
};
use mapper::mapper::Mapper;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn story_path() -> PathBuf {
    stories_dir().join("zork0-r393-s890714.z6")
}

/// Which script command the mid-script persistence checks fire after: the
/// "west" (the 1262nd command) that arrives at the Wharf, one screen short of
/// the Port Foozle Casino. Placed deliberately late — past every random
/// jester event the script encounters (the last is an alligator
/// transformation several hundred commands earlier, at the Great Underground
/// Highway's Exit) — so the round trip cannot land on a turn whose outcome
/// still depends on the PRNG. `rng_state` (`zvm::cpu::exec::Machine`) is a
/// plain Rust field, not part of the Quetzal-shaped save/restore payload, so
/// it is untouched by a same-session restore either way — but this checkpoint
/// choice means that fact never has to be relied on.
const CHECKPOINT_AFTER_COMMAND: usize = 1262;

/// A generous cap over the committed script's 1266 real commands.
const MAX_TURNS: usize = 1400;

/// Boot Zork Zero the way `startup.rs` boots a bare story file (no medium):
/// the profile resolves with no disk/medium context, and `MachineBoot::resolve`
/// derives the standard window/art-scale/cell together — mirrors
/// `real_media_releases.rs`'s `boot()` and `v6_zork0_hints.rs`'s Macintosh-disk
/// variant, for the bare-file case.
fn boot_zork0(seed: Option<u32>) -> Option<(Vec<u8>, GameSession)> {
    let path = story_path();
    let bytes = std::fs::read(&path).ok()?;
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = app::graphics::PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let boot = MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut session = GameSession::new_for_machine(bytes.clone(), true, false, false, picture_dims, None, seed, &boot)
        .expect("Zork Zero should boot without a ZError");
    assert!(!session.quit, "Zork Zero quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork Zero faulted during boot: {:?}", session.machine.fault_trace);
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some((bytes, session))
}

fn read_script() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("tests/fixtures/walkthroughs/zork0.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("committed walkthrough script must be readable at {}: {e}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Submit one script command, routing it to a keypress when the game is at a
/// single-key (Char) prompt (the Tower of Bozbar's digit/peg-letter moves,
/// the Peggleboz board, the shell-game point, every "Hit any key" pause) and
/// as an ordinary typed line otherwise. The committed script never needs
/// arrow-key/mouse-only input, so unlike the exploration scaffolding this
/// harness grew from, no special `@`-prefixed tokens remain.
fn submit_command(session: &mut GameSession, cmd: &str) -> TurnResult {
    match session.pending_input() {
        InputKind::Char => session.submit_char(cmd.as_bytes().first().copied().unwrap_or(b' ')),
        InputKind::Line => session.submit(cmd),
        InputKind::Event => session.submit(""),
    }
}

/// Collapse whitespace so the determinism/golden-file comparisons below don't
/// care about incidental v6-style line wrapping — matches the repo's existing
/// precedent (`restart_reboots_in_place.rs`'s `norm`, `walkthroughs.rs`'s).
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True if `look`'s reply shows the Wharf — the mid-script checkpoint's own
/// precondition/postcondition probe, exactly like `walkthroughs.rs`'s
/// `looking_shows_landing_site`.
fn looking_shows_wharf(session: &mut GameSession) -> bool {
    session.submit("look").transcript.contains("Wharf")
}

/// Exercises the in-game `@save`/`@restore` round trip (through the host
/// archive path) and the host Save State/Restore State round trip, both
/// v6-aware per `zork0_v6_persistence.rs`'s shape: the v6 window table rides
/// `screen.bin` (via `restore_screen`, same as `main.rs`'s real host-restore
/// arm) and the graphics canvases/display-list/painted-ground ride the
/// archive's picture payload (`v6_save_payload` in, `apply_v6_pictures` out —
/// the exact pair the real app's save/restore call sites use). Leaves
/// `session` exactly where it found it — back at the Wharf, pending a Line
/// command — so the caller's script loop can resume as if this had never run.
fn mid_script_persistence_checks(session: &mut GameSession) {
    assert!(looking_shows_wharf(session), "checkpoint precondition: back at the Wharf, one screen short of the Casino");

    // ---- In-game @save / @restore, through the host archive path ------------
    let r = session.submit("save");
    assert_eq!(
        r.pending_io,
        Some(PendingIo::Save),
        "Zork Zero's SAVE verb must bubble a host Save request: {:?}",
        r.transcript
    );

    let ingame = app::persist_files::game_save_bytes(session, SaveTrigger::Ingame);
    let (ig_pics, ig_display, ig_ground, _ig_diags) = v6_save_payload(session);
    let dir = app::scratch_dir("sq1600-zork0-ingame-save");
    app::persist_files::save_named(
        &dir,
        "SQ1600-ZORK0",
        "slot",
        SaveTrigger::Ingame,
        &Mapper::default(),
        &ingame,
        Some(&session.machine.screen),
        &ig_pics,
        ig_display.as_ref(),
        ig_ground.as_deref(),
        session.aux_data(),
        CHECKPOINT_AFTER_COMMAND as u32,
        Some("Wharf".to_string()),
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
        .expect("a real Zork Zero @save archive must restore through the host restore path");
    assert!(!session.has_quit(), "the restore left the session alive");
    assert!(looking_shows_wharf(session), "restored to the save point");
    let _ = std::fs::remove_dir_all(&dir);

    // ---- Host Save State / Restore State (v6-aware) --------------------------
    let es = Engine::save_state(session);
    let (v6_pics, v6_display, v6_ground, _diags) = v6_save_payload(session);
    let ss_dir = app::scratch_dir("sq1600-zork0-host-save");
    let ss_path = ss_dir.join("hoststate.lanthorn");
    archive::save_archive_meta_pics(
        &ss_path,
        &Mapper::default(),
        &es,
        Some(&session.machine.screen),
        session.aux_data(),
        Meta {
            format_version: archive::CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: CHECKPOINT_AFTER_COMMAND as u32,
            saved_at: String::new(),
            location: Some("Wharf".to_string()),
            score: None,
            trigger: SaveTrigger::HostState,
            source: archive::SaveSource::default(),
        },
        &SessionRecord::empty(),
        &v6_pics,
        v6_display.as_ref(),
        v6_ground.as_deref(),
    )
    .expect("save_archive_meta_pics");

    // Drift far enough that the restore below proves something real: step
    // into the Casino itself, which leaves the Wharf entirely and triggers
    // the jester's Double-Fanucci intro (a real, distinct state change).
    let drift = session.submit("north");
    assert!(
        !drift.transcript.contains("Wharf"),
        "the drift really left the checkpoint: {:?}",
        drift.transcript
    );

    let ac = archive::load_archive(&ss_path).expect("load_archive");
    let _ = std::fs::remove_dir_all(&ss_dir);
    Engine::restore_state(session, &ac.engine_save()).expect("restore_state (Quetzal memory/stack/pc)");
    if let Some(scr) = ac.screen.clone() {
        restore_screen(session, scr);
    }
    apply_v6_pictures(session, &ac);
    let _ = session.take_transcript();
    assert!(!session.has_quit());
    assert!(
        looking_shows_wharf(session),
        "the host Save State/Restore State round trip lands back at the checkpoint"
    );
}

/// After a `restart` command, answer the game's Y/N confirmation, then absorb
/// the "Restarting." pause. Mirrors `restart_reboots_in_place.rs`'s
/// `confirm_restart`, adapted for Zork Zero's own confirmation shape (a
/// direct "(y or n)?" — not the multi-turn "yes"-typing dance some games use).
fn confirm_restart(session: &mut GameSession) -> String {
    let mut collected = String::new();
    for _ in 0..6 {
        let result = match session.pending_input() {
            InputKind::Line => session.submit("y"),
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

/// Boots a completely SEPARATE Zork Zero session (never the one the main
/// walkthrough is driving), drives past the Prologue into the "94 years
/// later" Great Hall, and proves `restart` reboots all the way back to the
/// very start (the waiter Prologue) rather than quitting the app — mirrors
/// `restart_reboots_in_place.rs`'s pattern. Verified empirically (SQ-1600):
/// Zork Zero's `@restart` re-runs the whole program from its first
/// instruction, per ZMSD §2.4, so the reboot text is the Prologue's opening
/// banner, not the "94 YEARS LATER..." mid-game one.
fn check_restart_reboots() {
    let Some((_bytes, mut session)) = boot_zork0(None) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path().display());
        return;
    };
    let boot_banner = norm(&session.take_transcript());
    assert!(
        boot_banner.contains("frantic day at the castle"),
        "sanity: the real Prologue opening: {boot_banner:?}"
    );

    let mut transition = String::new();
    for cmd in ["northeast", "south", "west", "wait", "wait", "dive under table", "wait", "wait", "wait", "stand"] {
        transition.push_str(&submit_command(&mut session, cmd).transcript);
    }
    let transition = norm(&transition);
    assert!(
        transition.contains("94 YEARS LATER") && transition.contains("Great Hall"),
        "sanity: the Prologue-to-mainline transition ('stand' after diving under the table) must land in the \
         post-Prologue Great Hall: {transition:?}"
    );

    let r = session.submit("restart");
    assert!(!r.quit, "the restart command itself must not quit");
    assert!(r.fault.is_none(), "restart faulted: {:?}", r.fault);
    let mut reboot_text = norm(&r.transcript);
    reboot_text.push(' ');
    reboot_text.push_str(&norm(&confirm_restart(&mut session)));

    assert!(!session.has_quit(), "session must stay alive after restart");
    assert!(session.machine.fault_trace.is_none(), "no fault after restart");
    assert!(
        reboot_text.contains("frantic day at the castle"),
        "restart must re-run Zork Zero from the Prologue's opening (boot banner should reappear)\n  reboot: {reboot_text:?}"
    );
}

/// Unlike `walkthroughs.rs`'s Photopia precedent (no hint sidecar anywhere,
/// so the point there is only that the machinery doesn't error on absence),
/// `stories/zork0izm.z5` is a REAL external InvisiClues hint program for this
/// exact game (the "waitingforgo" IZM naming, `zork0izm` → `zork0` in
/// `IZM_HINTS`, `hints.rs`) — confirmed present empirically (SQ-1600) rather
/// than assumed from the Photopia shape. So this checks the stronger, more
/// interesting path: the host's external-hint resolution (`app::host::hints`)
/// finds it and boots it cleanly. This is entirely separate from Zork Zero's
/// own SEPARATE, built-in `HINT` verb (an in-fiction InvisiClues-style menu
/// explored directly in-game during this quest's investigation: topics like
/// "SECRET WING"/"EAST WING"/"GENERAL QUESTIONS", graduated reveals,
/// arrow-key navigation) — that in-game menu is not what this function
/// exercises at all.
fn check_hints_machinery_agrees(story_path: &Path, story_bytes: &[u8]) {
    let ifid = compute_ifid(story_bytes);
    let home = app::scratch_dir("sq1600-zork0-empty-hint-index");
    let index = hints::load_hint_index(&home);

    assert_eq!(
        available(story_path, &ifid, "", &index),
        HintAvailability::Available,
        "stories/zork0izm.z5 is a real InvisiClues sidecar for this game and must resolve"
    );
    let cfg = Config::default();
    let session = open(story_path, &ifid, "", &index, &[], &cfg)
        .expect("resolving to Available must mean open() can actually boot it")
        .expect("open() must return a session when available() said Available");
    assert!(
        !session.transcript.is_empty(),
        "the booted hint program must have printed its own opening screen"
    );
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
/// injecting [`mid_script_persistence_checks`] at [`CHECKPOINT_AFTER_COMMAND`],
/// and feeding every turn through `apply_turn` — Zork Zero is a real
/// Inform/ZIL location-tracked game (unlike Photopia), so the mapper is
/// expected to populate a genuine room graph, not stay empty.
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
fn zork0_walkthrough_reaches_the_double_fanucci_table_with_no_fault_and_a_populated_map() {
    let Some((bytes, session)) = boot_zork0(None) else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/zork0-r393-s890714.z6");
        return;
    };
    let path = story_path();
    let commands = read_script();
    assert!(commands.len() > 1000, "premise: the committed script is the full verified partial walkthrough, not a stub");

    let outcome = play_full_script(session, &commands);

    // NOT a win — see the module doc comment. The acceptance here is reaching
    // the Casino table, sitting down, with no fault and no quit anywhere
    // along the way.
    assert!(!outcome.quit, "the partial walkthrough must not quit — it deliberately stops short of Double Fanucci, not at a death/game-over");
    assert!(
        norm(&outcome.transcript).contains("You are now sitting at the card table"),
        "the script's last real move must be sitting down for Double Fanucci: {:?}",
        outcome.transcript
    );

    // Mapper: Zork Zero is a real Inform/ZIL location-tracked game (unlike
    // Photopia's Phase-1 empty-graph shape) — checked empirically (SQ-1600):
    // driving the full script populates a genuine room graph.
    let room_count = outcome.mapper.graph.rooms().count();
    assert!(room_count > 50, "expected a well-populated map after 1266 turns across most of the castle, got {room_count} rooms");
    for name in ["Great Hall", "Banquet Hall", "Casino", "Toll Plaza", "Secret Passage"] {
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
    // Empirically (SQ-1647): the full 1266-command partial playthrough tracks 111 distinct
    // items. A generous floor well under that, not the exact figure, so an unrelated future
    // script edit doesn't need to touch this assertion — the point is only that item
    // detection didn't quietly stop working across a v6/graphical game's much bigger object
    // tree.
    let items: Vec<_> = outcome.mapper.graph.items().collect();
    assert!(
        items.len() > 50,
        "Zork Zero is a huge, v6 Inform/ZIL-shaped game with plenty of scenery/inventory objects; \
         a full partial playthrough across most of the castle should track well over 50, got {}",
        items.len()
    );
    let vanished = items
        .iter()
        .filter(|(_, r)| matches!(r.last_seen, mapper::graph::ItemLocation::Vanished { .. }))
        .count();
    assert!(
        vanished * 10 < items.len() * 9,
        "{vanished} of {} tracked items ended Vanished — that's suspiciously close to \"everything\", \
         the shape a regression that vanishes on every sweep would produce (empirically this specimen sees ~31%)",
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
fn zork0_walkthrough_is_deterministic_and_matches_the_pinned_transcript() {
    let Some((_, session1)) = boot_zork0(None) else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/zork0-r393-s890714.z6");
        return;
    };
    let commands = read_script();

    let run1 = norm(&play_full_script(session1, &commands).transcript);

    let Some((_, session2)) = boot_zork0(None) else {
        eprintln!("SKIP: gitignored/fetched story missing at stories/zork0-r393-s890714.z6");
        return;
    };
    let run2 = norm(&play_full_script(session2, &commands).transcript);

    assert_eq!(
        run1, run2,
        "two fresh runs of the same script must produce byte-identical (normalized) transcripts — this is the \
         falsifiable claim that Zork Zero's randomness is seeded deterministically here, not the walkthrough \
         author's say-so. (Confirmed during SQ-1600: the jester's random events — bat teleports, an alligator \
         transformation, clown noses — fire at the SAME points in both runs.)"
    );

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let golden_path = manifest.join("tests/fixtures/walkthroughs/zork0.transcript");
    if std::env::var_os("SQ1600_REGEN_GOLDEN").is_some() {
        std::fs::write(&golden_path, format!("{run1}\n")).expect("regenerate the pinned transcript");
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("committed reference transcript must be readable at {}: {e}", golden_path.display()));
    assert_eq!(
        run1,
        golden.trim_end(),
        "the current run's normalized transcript no longer matches the pinned reference \
         (crates/app/tests/fixtures/walkthroughs/zork0.transcript) — if this is an intentional \
         behaviour change, regenerate the pinned file and explain why in the commit"
    );
}
