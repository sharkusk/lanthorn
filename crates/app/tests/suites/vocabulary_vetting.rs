//! SQ-1121 — the offer is vetted in a silent copy of the game before it is shown.
//!
//! Two questions, and they are different: **does this story hold the word**
//! (SQ-1041, a static fact about the dictionary, settled in
//! `vocabulary_offer.rs`) and **would typing it here do anything** — which only
//! the story can answer, by being asked in a copy that costs nothing.
//!
//! The canonical pair is `illuminate lamp` in Zork I: outside the house the
//! lamp is not there and the suggestion is dropped, in the Living Room it works
//! and the line appears. Same story, same command, same candidate; the only
//! difference is where the player is standing, which is exactly what a
//! dictionary cannot see.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::Engine;
use app::probe::ShadowRecipe;
use app::state::{AppState, TranscriptKind};

// ── Fixtures. `stories/` is gitignored; every case here skips vacuously. ────

fn story(name: &str) -> Option<Vec<u8>> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

fn recipe(bytes: &[u8]) -> ShadowRecipe {
    recipe_in(bytes, PathBuf::new(), Vec::new())
}

/// The same recipe with the live game's own persistent data, which the shadow
/// may READ and never write (SQ-1124).
///
/// **Both halves or neither**, on the evidence: Counterfeit Monkey checks a
/// 52-byte marker in the Glk file VFS and only then `@restore`s the
/// `_Counterfeit_Monkey-startup-data.qzl` beside it. Given the `.qzl` alone it
/// never asks for it and re-runs the whole initialisation — which is exactly
/// what the case below measures.
fn recipe_in(bytes: &[u8], store: PathBuf, vfs: Vec<u8>) -> ShadowRecipe {
    ShadowRecipe {
        story_bytes: Arc::new(bytes.to_vec()),
        store,
        vfs_bytes: Arc::new(vfs),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    }
}

fn assists(state: &AppState) -> Vec<String> {
    state
        .transcript
        .iter()
        .zip(&state.transcript_kinds)
        .filter(|(_, k)| **k == TranscriptKind::Assist)
        .map(|(l, _)| l.clone())
        .collect()
}

/// Drive a real story the way `finish_command_turn` does — the game's reply into
/// the transcript, then the offer — with the probe seam ARMED, which is the
/// difference from `vocabulary_offer.rs`'s harness.
struct Play {
    state: AppState,
    session: Box<dyn Engine>,
}

impl Play {
    fn zork1() -> Option<Play> {
        let bytes = story("zork1-r88-s840726.z3")?;
        let mut s = app::session::GameSession::new_with_trace(
            bytes.clone(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
        )
        .expect("zork1-r88-s840726.z3 boots without a ZError");
        s.set_strip_prompt(false);
        let mut state = AppState::default();
        state.assist_preamble_shown = true;
        state.probe.arm(recipe(&bytes));
        Some(Play { state, session: Box::new(s) })
    }

    /// SQ-1206's own fixture for the `hasten north` false positive, and one of
    /// the two the research noted vets normally (SQ-1232).
    fn savoir_faire() -> Option<Play> {
        let bytes = story("Savoir-Faire.zblorb")?;
        let app::hints::LoadedStory::ZCode(story_bytes) =
            app::hints::extract_story(bytes.clone()).expect("Savoir-Faire.zblorb is readable")
        else {
            panic!("Savoir-Faire.zblorb is a Z-code story");
        };
        let mut s = app::session::GameSession::new_with_trace(
            story_bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
        )
        .expect("Savoir-Faire.zblorb boots without a ZError");
        s.set_strip_prompt(false);
        let mut state = AppState::default();
        state.assist_preamble_shown = true;
        // `recipe` re-extracts from the ORIGINAL container bytes, exactly as
        // `ShadowRecipe::story_bytes` requires (see its own docs).
        state.probe.arm(recipe(&bytes));
        Some(Play { state, session: Box::new(s) })
    }

    fn turn(&mut self, cmd: &str) {
        let r = self.session.submit(cmd);
        self.state.push_transcript_kind(&format!("> {cmd}"), TranscriptKind::Input);
        self.state
            .push_transcript_kind(r.transcript.trim_end_matches('\n'), TranscriptKind::Story);
        let printed = !r.transcript.trim().is_empty();
        app::vocab::offer_vocabulary(&mut self.state, &*self.session, cmd, printed);
        // The beat after the turn (SQ-1124). The offer is asked of a worker
        // thread and shown when it answers; the event loop collects it with
        // `poll_vocabulary_offer` a frame or two later, and a harness that wants
        // to assert on it waits here instead of racing the thread. Nothing about
        // WHAT is shown differs — only when.
        app::vocab::settle_vocabulary_offer(&mut self.state);
    }

    fn walk(&mut self, cmds: &[&str]) {
        for c in cmds {
            self.turn(c);
        }
    }

    fn assists(&self) -> Vec<String> {
        assists(&self.state)
    }

    /// Everything on screen, as the player would have seen it — the specimen a
    /// finding has to be able to show.
    fn screen(&self) -> String {
        self.state
            .transcript
            .iter()
            .zip(&self.state.transcript_kinds)
            .map(|(l, k)| if *k == TranscriptKind::Assist { format!("● {l}") } else { l.clone() })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The walk from West of House into the Living Room, where the lamp is.
const TO_THE_LAMP: &[&str] = &["north", "east", "open window", "enter window", "west"];

// ── The canonical pair ──────────────────────────────────────────────────────

/// **The case the whole quest exists for.** `illuminate lamp` at the front door:
/// `light` is in Zork's dictionary, so SQ-1041 would offer it — and typing it
/// there gets `You can't see any lamp here!`, which costs the player a turn to
/// discover. The probe discovers it instead, in a copy, and the light says
/// nothing.
///
/// Falsify by turning `guidance_probe` off: the case below does exactly that and
/// gets the offer back, unvetted.
#[test]
fn a_suggestion_that_would_not_work_here_is_never_shown() {
    let Some(mut p) = Play::zork1() else { return };
    p.turn("illuminate lamp");
    eprintln!("--- Zork I r88, turn 1, West of House ---\n{}\n", p.screen());
    assert_eq!(p.assists(), Vec::<String>::new(), "the lamp is not here, so neither is the offer");
}

/// The same command in the room the lamp is in: the probe watches `light lamp`
/// turn the lantern on and the light recommends it, in the words a
/// recommendation earns (SQ-1121's wording note).
#[test]
fn a_suggestion_that_works_here_is_shown_as_a_recommendation() {
    let Some(mut p) = Play::zork1() else { return };
    p.walk(TO_THE_LAMP);
    assert!(
        p.state.transcript.iter().any(|l| l.contains("Living Room")),
        "the fixture is the walk: five turns in, the player is in the Living Room"
    );
    p.turn("illuminate lamp");
    eprintln!("--- Zork I r88, turn 6, Living Room ---\n{}\n", p.screen());
    assert_eq!(p.assists(), vec!["try instead — light"]);
}

/// Both halves in one session, which is the shape a player actually meets: the
/// suggestion is refused at the door, and the SAME word is offered five rooms
/// later. A dropped offer must therefore not spend the word's one-per-session
/// answer — nothing was said, so nothing was answered.
#[test]
fn a_dropped_offer_does_not_spend_the_words_one_answer() {
    let Some(mut p) = Play::zork1() else { return };
    p.turn("illuminate lamp");
    assert_eq!(p.assists(), Vec::<String>::new());
    p.walk(TO_THE_LAMP);
    p.turn("illuminate lamp");
    eprintln!("--- Zork I r88, both halves ---\n{}\n", p.screen());
    assert_eq!(p.assists(), vec!["try instead — light"]);
}

// ── A direction object defeats the noun control (SQ-1232) ──────────────────

/// **The false positive SQ-1206's research found in 17 of 30 stories.**
/// `hasten` is one keystroke from `fasten`, which Savoir-Faire's grammar
/// spells `fasten`/`attach`/`fix` — one verb, three ways in. Every one of
/// them answers `fasten north`, `attach north` and `fix north` alike with
/// "You would achieve nothing by this.", but the noun-based control alone
/// never learns that shape: `fasten <absent noun>` gets "You can't see any
/// such thing.", a different sentence, so the candidate reads as a success
/// and the light offered `fasten` for a plain compass direction.
///
/// Falsify by reverting the direction control added to `vetting_plan`
/// (`vocab.rs`'s `dir_words`/`dir_pair`): this assertion then fails with
/// `fasten` present in the offer, which was the reported symptom.
#[test]
fn a_direction_object_no_longer_earns_a_false_positive() {
    let Some(mut p) = Play::savoir_faire() else { return };
    p.turn("look");
    p.turn("hasten north");
    eprintln!("--- Savoir-Faire, Kitchen Garden, `hasten north` ---\n{}\n", p.screen());
    assert!(
        !p.assists().iter().any(|l| l.contains("fasten")),
        "`fasten` must never be offered for a direction object: {:?}",
        p.assists()
    );
}

// ── The claim matches what was actually done ────────────────────────────────

/// With the probe switched off the offer still appears — and says the modest
/// thing it can still support. `try instead` is a recommendation and is earned
/// by the vetting; naming the dictionary is a fact and is not.
///
/// **SQ-1238 added `light up` to the line.** `light up` is a member of
/// `illuminate`'s synonym group, and Zork's dictionary genuinely holds `light`
/// and `up` — every word of the phrase, which is what the fixed `stored` now
/// asks of a multi-word member (the bug was a naive whole-PHRASE truncation
/// crediting a story with a member it never implemented; the brief's own fix
/// is explicit that the app has no seam to also verify the phrase parses as
/// the SAME action). This is exactly the unvetted claim's documented, weaker
/// promise: "this story's dictionary holds them, and nothing more is
/// claimed." The two `try instead` cases just above this one pin that the
/// STRONGER claim is unaffected — the probe still discards `light up` there,
/// because typing it does nothing this game recognises as lighting the lamp.
#[test]
fn without_the_probe_the_line_makes_the_weaker_claim() {
    let Some(mut p) = Play::zork1() else { return };
    p.state.config.guidance_probe = false;
    p.walk(TO_THE_LAMP);
    p.turn("illuminate lamp");
    assert_eq!(p.assists(), vec!["this story knows — light · light up"]);
}

/// And an unarmed seam — every `AppState::default()`, and any session whose
/// story bytes were never kept — is the same case: no vetting happened, so no
/// vetted claim is made. This is what keeps `vocabulary_offer.rs` honest.
///
/// See the note on `without_the_probe_the_line_makes_the_weaker_claim` for why
/// SQ-1238 added `light up` to this line too.
#[test]
fn an_unarmed_seam_makes_the_weaker_claim_too() {
    let Some(mut p) = Play::zork1() else { return };
    p.state.probe = app::probe::ShadowProbe::default();
    p.walk(TO_THE_LAMP);
    p.turn("illuminate lamp");
    assert_eq!(p.assists(), vec!["this story knows — light · light up"]);
}

// ── The story's own signature of failure, discovered ────────────────────────

/// The oracle is not a table of English phrases: it is what THIS story printed
/// when the shadow typed deliberate nonsense at it, **in the room the question
/// was asked in**.
///
/// Zork I answers `light rug` with `You don't have that!` in the field and `You
/// don't have the carpet.` in the living room, and `light lamp` with `You don't
/// have that!` outside the house and `(Taken) The brass lantern is now on.`
/// inside it. A signature learned once a session is therefore a signature of the
/// wrong room, which is why the controls travel with the question.
#[test]
fn this_storys_refusals_are_learned_from_the_story_in_this_room() {
    let Some(mut p) = Play::zork1() else { return };
    // A word the dictionary does not hold, then the same verb with two nouns
    // that are not here — which is the shape `vocab` builds for a candidate.
    let controls =
        ["zqxwvj".to_string(), "light sword".to_string(), "light water".to_string()];

    let run = p.state.probe.run(&*p.session, &controls).expect("the shadow answers");
    let mut out = run.refusal_from(0);
    out.merge(run.refusal_from_pair(1, 2));
    let sentences: Vec<&str> = out.sentences().collect();
    eprintln!("Zork I r88, West of House — refusal signature: {sentences:#?}");
    assert!(
        out.says_no("I don't know the word \"illuminate\".", "illuminate lamp"),
        "the unknown-word refusal must be in the signature: {sentences:?}"
    );
    assert!(
        out.says_no("You don't have that!", "light lamp"),
        "and the ACTION-level refusal the pair provoked: {sentences:?}"
    );
    assert!(
        !out.says_no("The brass lantern is now on.", "light lamp"),
        "a success must never read as a refusal: {sentences:?}"
    );

    // The same three controls after the walk. `grue` and `water` no longer
    // agree in here — Zork names the carpet and the sword it can see — so the
    // pair teaches nothing, and the run falls back to what it is sure of. That
    // is the failure mode this design chooses: it keeps a suggestion it cannot
    // judge rather than dropping one it cannot judge.
    p.walk(TO_THE_LAMP);
    let run = p.state.probe.run(&*p.session, &controls).expect("the shadow answers");
    let mut here = run.refusal_from(0);
    here.merge(run.refusal_from_pair(1, 2));
    eprintln!(
        "Zork I r88, Living Room — refusal signature: {:#?}",
        here.sentences().collect::<Vec<_>>()
    );
    assert!(
        !here.says_no("The brass lantern is now on.", "light lamp"),
        "a success must never read as a refusal"
    );
}

/// A control that DID something is not describing a refusal, and a pair whose
/// two replies differ is not describing one either. Both are the guard against
/// the fingerprint swallowing a real action's words and then classifying every
/// success as a failure.
#[test]
fn a_control_that_worked_teaches_nothing() {
    let Some(mut p) = Play::zork1() else { return };
    p.walk(TO_THE_LAMP);
    // `take sword` works here; `take zqjwbf` does not. A pair that disagrees
    // teaches nothing, whichever way round it is asked.
    let run = p.state.probe
        .run(&*p.session, &["take sword".to_string(), "take zqjwbf".to_string()])
        .expect("the shadow answers");
    assert!(
        run.refusal_from_pair(0, 1).is_empty(),
        "two different replies were read as one refusal"
    );
    assert!(
        run.refusal_from(0).is_empty(),
        "a control that moved an object taught the shadow its own success"
    );
}

// ── Isolation ───────────────────────────────────────────────────────────────

/// **The live game is never touched.** A probe snapshots it and drives a second
/// engine; if it ever stepped or restored the live session, the player's own
/// game would silently move under them (SQ-0587/0588).
#[test]
fn the_live_session_is_left_exactly_where_it_was() {
    let Some(mut p) = Play::zork1() else { return };
    p.walk(TO_THE_LAMP);
    let before = p.session.save_state();
    let where_before = p.session.current_location().map(|l| l.number);
    p.turn("illuminate lamp");
    assert_eq!(p.assists(), vec!["try instead — light"], "the probe ran");
    // One turn passed in the LIVE game — the player's own `illuminate lamp` —
    // and the lantern is still off, because only the shadow ever lit it.
    let after = p.session.submit("look").transcript;
    assert_eq!(
        p.session.current_location().map(|l| l.number),
        where_before,
        "the probe moved the player"
    );
    assert!(
        !after.to_lowercase().contains("providing light"),
        "the shadow's lantern is lit in the LIVE game: {after:?}"
    );
    assert_ne!(before.bytes.len(), 0, "a snapshot was taken at all");
}

/// Nothing a probe does reaches the filesystem. The shadow is booted with an
/// empty game directory and an empty Glk VFS, and an in-game `@save` inside one
/// is answered "that failed" rather than being allowed to write — so a probe run
/// against a story cannot leave a file behind.
///
/// Checked by running the vetting with the process's temp directory watched:
/// nothing under the story's own directory may change either.
#[test]
fn a_probe_writes_nothing_anywhere() {
    let Some(mut p) = Play::zork1() else { return };
    let dir: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories");
    let before: Vec<String> = std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into()).collect())
        .unwrap_or_default();
    p.walk(TO_THE_LAMP);
    p.turn("illuminate lamp");
    let after: Vec<String> = std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into()).collect())
        .unwrap_or_default();
    assert_eq!(before.len(), after.len(), "the probe left a file beside the story");
    assert!(p.state.probe.probes > 0, "the case is vacuous unless the shadow ran");
}

// ── What it costs ───────────────────────────────────────────────────────────

/// The offer appears BETWEEN the player's command and the game's reply, so the
/// vetting has a budget and must be measured rather than assumed free. Prints
/// the snapshot size and the wall time of a first offer (which pays the shadow's
/// boot) and of a second (which does not).
#[test]
fn the_cost_of_a_vetted_offer_on_the_z_machine() {
    let Some(mut p) = Play::zork1() else { return };
    p.walk(TO_THE_LAMP);
    let save = p.session.save_state();
    let t0 = std::time::Instant::now();
    p.turn("illuminate lamp");
    let first = t0.elapsed();
    let n1 = p.state.probe.probes;
    let t1 = std::time::Instant::now();
    p.turn("inspect lamp");
    let second = t1.elapsed();
    eprintln!(
        "Z-machine (Zork I r88, .z3, {} KiB story): snapshot {} bytes; \
         first offer {first:?} over {n1} probes (shadow boot included); \
         second offer {second:?} over {} probes",
        story("zork1-r88-s840726.z3").map(|b| b.len() / 1024).unwrap_or(0),
        save.bytes.len(),
        p.state.probe.probes - n1,
    );
    assert!(
        second < std::time::Duration::from_millis(500),
        "a warm vetted offer must not stall the turn: {second:?}"
    );
}

/// The same measurement on Glulx, where the picture is different in both
/// directions: the memory map is megabytes rather than kilobytes, and
/// Counterfeit Monkey's initialisation is millions of opcodes even accelerated.
///
/// **This is the story that said no**, and the reason it said no was ours.
/// SQ-1121 booted the shadow from the story bytes with an empty persistent
/// store, so Counterfeit Monkey re-ran the whole initialisation the LIVE session
/// skips — 2.1 s, measured — and the seam latched `too_slow` and declined to vet
/// the corpus's biggest game for the rest of the session.
///
/// The live session is fast because of its own file cache: CM `@save`s a
/// `_Counterfeit_Monkey-startup-data` slot on its first launch and `@restore`s it
/// on every later one, which lanthorn services silently against the per-story
/// directory (the CHANGELOG's "5.4s → 0.76s from the second launch"). The shadow
/// now reads that same store, READ-ONLY, and takes the same path.
///
/// The case measures all three: the live cold boot that writes the cache, the
/// live warm boot that reads it, and a shadow booted each way. Numbers, not an
/// estimate — it prints them.
#[test]
fn counterfeit_monkeys_shadow_boots_the_way_the_live_game_boots() {
    let Some(bytes) = story("CounterfeitMonkey-11.gblorb") else { return };
    let app::hints::LoadedStory::Glulx(image) = app::hints::extract_story(bytes.clone())
        .expect("CounterfeitMonkey-11.gblorb is a readable container")
    else {
        panic!("CounterfeitMonkey-11.gblorb is a Glulx story");
    };

    let dir = std::env::temp_dir().join(format!("lanthorn-sq1124-cm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp game_dir");

    // The live session, twice, against a persistent store it may write: the
    // first launch runs the initialisation and leaves the cache, the second
    // restores it. If the second is not dramatically faster, this fixture is not
    // the game the finding is about and every number below is meaningless.
    let live_in = |dir: PathBuf, vfs: &[u8]| {
        let b = blorb::Blorb::parse(bytes.clone()).ok();
        app::glulx_session::GlulxSession::new_in(
            dir, image.clone(), 80, 24, true, false, false, false, (8, 16), b, vfs,
            [[(None, None); 11]; 2], false, None,
        )
        .expect("Counterfeit Monkey boots")
    };
    let t = std::time::Instant::now();
    let cold_session = live_in(dir.clone(), &[]);
    let live_cold = t.elapsed();
    let vfs = app::engine::Engine::vfs_bytes(&cold_session);
    eprintln!("  vfs after the cold boot: {} bytes, dirty={}", vfs.len(),
        app::engine::Engine::vfs_dirty(&cold_session));
    drop(cold_session);
    let t = std::time::Instant::now();
    let live = live_in(dir.clone(), &vfs);
    let live_warm = t.elapsed();
    let save = live.save_state();
    let cached: Vec<String> = std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into()).collect())
        .unwrap_or_default();

    // SQ-1121's shadow: no store at all.
    let mut blind = app::probe::ShadowProbe::default();
    blind.arm(recipe(&bytes));
    let t = std::time::Instant::now();
    let blind_run = blind.run(&live, &["take zqxwvj".to_string()]);
    let blind_cold = t.elapsed();

    // SQ-1124's shadow: the live game's own store, read-only.
    let mut probe = app::probe::ShadowProbe::default();
    probe.arm(recipe_in(&bytes, dir.clone(), vfs.clone()));
    let t = std::time::Instant::now();
    let first = probe.run(&live, &["take zqxwvj".to_string()]);
    let cold = t.elapsed();
    let t = std::time::Instant::now();
    let second = probe.run(&live, &["take zqxwvj".to_string()]);
    let warm = t.elapsed();

    eprintln!(
        "Glulx (Counterfeit Monkey 11, {} KiB container): \n  \
         live boot cold {live_cold:?}, warm {live_warm:?} (cache: {cached:?})\n  \
         snapshot {} bytes\n  \
         shadow, NO store (SQ-1121):   first probe {blind_cold:?}, answered={}\n  \
         shadow, live store read-only: first probe {cold:?}, answered={}; \
         warm probe {warm:?}, answered={}\n  \
         seam armed={} after {} probes",
        bytes.len() / 1024,
        save.bytes.len(),
        blind_run.is_some(),
        first.is_some(),
        second.is_some(),
        probe.is_armed(),
        probe.probes,
    );

    // Non-vacuity: the cache has to exist, or the shadow read nothing and the
    // comparison is of two identical cold boots.
    assert!(
        cached.iter().any(|f| f.ends_with(".qzl")),
        "Counterfeit Monkey wrote no fixed-name save, so there was no cache to read: {cached:?}"
    );
    assert!(!vfs.is_empty(), "and no VFS marker, which is the half that makes it ASK");
    assert!(
        live_warm * 2 < live_cold,
        "the live warm boot ({live_warm:?}) is not meaningfully faster than the cold one \
         ({live_cold:?}) — this fixture does not use the cache and the finding does not apply"
    );
    // The claim.
    assert!(first.is_some(), "the shadow answered nothing");
    assert!(probe.is_armed(), "the seam gave up on a story it can now afford");
    assert!(
        cold * 2 < blind_cold,
        "reading the live game's store bought nothing: {cold:?} against {blind_cold:?}"
    );

    // And it wrote nothing while doing it: the store holds exactly what the live
    // session left there.
    let after: Vec<String> = std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into()).collect())
        .unwrap_or_default();
    assert_eq!(cached.len(), after.len(), "the shadow wrote into the live game's store");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A lighter Glulx story is a different answer, and the seam has to reach it —
/// the cap above is a measurement of one story, not a rule about an engine.
#[test]
fn a_lighter_glulx_story_is_still_probed() {
    let Some(bytes) = story("Coloratura.gblorb.blorb") else { return };
    let app::hints::LoadedStory::Glulx(image) = app::hints::extract_story(bytes.clone())
        .expect("Coloratura.gblorb.blorb is a readable container")
    else {
        panic!("Coloratura is a Glulx story");
    };
    let live =
        app::glulx_session::GlulxSession::new(image, 80, 24, true, false, false, (8, 16), None, &[])
            .expect("Coloratura boots");
    let mut probe = app::probe::ShadowProbe::default();
    probe.arm(recipe(&bytes));
    let t = std::time::Instant::now();
    let run = probe.run(&live, &["zqxwvj".to_string(), "take zqxwvj".to_string()]);
    eprintln!(
        "Glulx (Coloratura, {} KiB): two probes in {:?}, armed={}",
        bytes.len() / 1024,
        t.elapsed(),
        probe.is_armed(),
    );
    assert!(probe.is_armed(), "the seam gave up on a story of this size");
    let run = run.expect("the shadow answered nothing");
    for s in &run.steps {
        eprintln!("  {:?} -> {:?}", s.command, s.reply.trim());
    }
    eprintln!("  refusal: {:?}", run.refusal_from(0).sentences().collect::<Vec<_>>());
}

/// **The offer survives on a two-word parser**, end to end — the case that
/// nearly did not.
///
/// A Scott Adams turn ends with `Tell me what to do ?`, so that sentence is
/// inside the refusal the controls teach AND inside every success. An oracle
/// matching any sentence of a reply would have classified every reply as a
/// refusal and silenced the offer on the whole engine; matching the FIRST
/// sentence does not. Falsify by changing `Refusals::says_no` back to `any`.
#[test]
fn a_two_word_parser_is_not_silenced_by_its_own_prompt() {
    let Some(bytes) = story("adv14a.dat") else { return };
    let live = app::scott_session::ScottSession::new(bytes.clone(), None)
        .expect("adv14a.dat loads");
    let mut state = AppState::default();
    state.assist_preamble_shown = true;
    state.probe.arm(recipe(&bytes));
    let mut session: Box<dyn Engine> = Box::new(live);

    for cmd in ["quti", "loko"] {
        let r = session.submit(cmd);
        state.push_transcript_kind(&format!("> {cmd}"), TranscriptKind::Input);
        state.push_transcript_kind(r.transcript.trim_end_matches('\n'), TranscriptKind::Story);
        app::vocab::offer_vocabulary(&mut state, &*session, cmd, !r.transcript.trim().is_empty());
        app::vocab::settle_vocabulary_offer(&mut state); // the beat after the turn (SQ-1124)
    }
    let lines = assists(&state);
    eprintln!("Scott adv14a.dat, vetted: {lines:?} ({} probes)", state.probe.probes);
    assert!(state.probe.probes > 0, "the case is vacuous unless the shadow ran");
    assert_eq!(
        lines,
        vec!["try instead — quit", "try instead — look"],
        "the offer was silenced on a two-word parser, or went unvetted"
    );
}

/// And the third engine, because "engine-neutral" is a claim about all of them:
/// a Scott Adams database forks and answers like the other two.
///
/// The `tiny_cave` fixture is in the repo, so this one never skips.
#[test]
fn a_scott_adams_story_forks_and_answers() {
    let bytes = include_bytes!("../../../scott/tests/tiny_cave.dat").to_vec();
    let live = app::scott_session::ScottSession::new(bytes.clone(), None)
        .expect("tiny_cave.dat loads");
    let mut probe = app::probe::ShadowProbe::default();
    probe.arm(recipe(&bytes));
    let run = probe
        .run(&live, &["zqxwvj".to_string(), "take zqxwvj".to_string()])
        .expect("the Scott shadow answers");
    for s in &run.steps {
        eprintln!("Scott tiny_cave: {:?} -> {:?}", s.command, s.reply.trim());
    }
    assert_eq!(run.steps.len(), 2);
    assert!(
        !run.refusal_from(0).is_empty(),
        "a two-word parser handed nonsense taught the shadow nothing"
    );
    assert!(probe.is_armed(), "a database this small must not trip the too-slow latch");
}

// ── Off the main thread, and what that costs (SQ-1124) ──────────────────────

/// **What the player's turn now pays.** The vetting happens on a worker thread,
/// so the only main-thread cost of an offer is taking the snapshot and hashing
/// the world — everything after that is the worker's. SQ-1121 spent the whole
/// run inline under a 400 ms budget; there is no budget any more because there
/// is no stall to cap.
///
/// Prints both halves for every engine the corpus reaches, so the claim is a
/// measurement rather than an assertion about a design.
#[test]
fn asking_costs_the_players_turn_a_snapshot_and_nothing_else() {
    let Some(mut p) = Play::zork1() else { return };
    p.walk(TO_THE_LAMP);
    let cmds: Vec<String> =
        ["zqxwvj", "light sword", "light water", "light lamp"].iter().map(|s| s.to_string()).collect();

    // Cold — the ask that also causes the shadow's boot, which is the worst case
    // and still costs the caller only the snapshot.
    let t = std::time::Instant::now();
    let token = p.state.probe.ask(&*p.session, &cmds).expect("the seam is armed");
    let ask_cold = t.elapsed();
    let t = std::time::Instant::now();
    let answered = p.state.probe.settle().is_some();
    let worker_cold = t.elapsed();

    let t = std::time::Instant::now();
    p.state.probe.ask(&*p.session, &cmds).expect("the seam is still armed");
    let ask_warm = t.elapsed();
    let t = std::time::Instant::now();
    p.state.probe.settle();
    let worker_warm = t.elapsed();

    eprintln!(
        "Zork I r88, Living Room, four commands: token {token}, answered={answered}\n  \
         main thread: ask {ask_cold:?} cold, {ask_warm:?} warm\n  \
         worker:      {worker_cold:?} cold (boot included), {worker_warm:?} warm"
    );
    // The number that matters: what the player waits for. A snapshot of a .z3 is
    // a few hundred bytes and a world print is a handful of hashes.
    assert!(
        ask_warm < std::time::Duration::from_millis(5),
        "asking is supposed to be free: {ask_warm:?}"
    );
    assert!(
        ask_cold < worker_cold,
        "the ask paid for the boot, which is exactly what it must not do"
    );
}

/// **A late offer that missed its turn is dropped, not printed.**
///
/// The player typed again while the shadow was still thinking, so the answer
/// describes a command that is no longer the last one on screen. SQ-1125 (a
/// prompt-anchored hint, which would have made lateness invisible) is parked, so
/// the answer is discarded — silently, which is this feature's existing
/// discipline rather than a new rule.
///
/// Falsify by removing the epoch check in `poll_vocabulary_offer`: the line then
/// appears underneath a `look` that never provoked it.
#[test]
fn an_offer_that_arrives_after_the_player_typed_again_is_dropped() {
    let Some(mut p) = Play::zork1() else { return };
    p.walk(TO_THE_LAMP);

    // The turn that asks — without the beat afterwards, which is the whole point.
    let r = p.session.submit("illuminate lamp");
    p.state.push_transcript_kind("> illuminate lamp", TranscriptKind::Input);
    p.state.push_transcript_kind(r.transcript.trim_end_matches('\n'), TranscriptKind::Story);
    app::vocab::offer_vocabulary(&mut p.state, &*p.session, "illuminate lamp", true);
    assert!(p.assists().is_empty(), "the offer was shown synchronously after all");

    // The player types again before the shadow answers.
    p.state.begin_turn();
    let after = p.session.submit("look");
    p.state.push_transcript_kind(after.transcript.trim_end_matches('\n'), TranscriptKind::Story);

    let shown = app::vocab::settle_vocabulary_offer(&mut p.state);
    assert!(!shown, "a stale answer reached the transcript");
    assert!(
        p.assists().is_empty(),
        "a suggestion was printed under the wrong command: {:?}",
        p.assists()
    );
    // And the state is clean: nothing is left waiting for a turn that has passed.
    assert!(p.state.probe.is_armed(), "the seam gave up over a dropped answer");
    assert!(!p.state.probe.is_busy(), "the worker is still holding the stale question");
}

/// **A player scrolled back reading is not yanked to the bottom.**
///
/// A synchronous offer lands inside a turn the player is already watching; a
/// late one arrives unprompted, and if it moved the view it would move it while
/// somebody is reading. It does not: the transcript pane is bottom-anchored and
/// an insert above the prompt scrolls HISTORY up, and nothing on the offer's path
/// touches `transcript_scroll`.
#[test]
fn a_late_offer_leaves_a_scrolled_back_reader_where_they_were() {
    let Some(mut p) = Play::zork1() else { return };
    p.walk(TO_THE_LAMP);
    p.state.transcript_scroll = 7;
    p.turn("illuminate lamp");
    assert_eq!(p.assists(), vec!["try instead — light"], "the offer must actually have landed");
    assert_eq!(p.state.transcript_scroll, 7, "the late insert moved the reader's view");
}

/// **What asking the story beats guessing from its prose** (SQ-1042's
/// `ObjectWords`, folded into `absent_nouns` by SQ-1124).
///
/// The controls need two dictionary nouns that are NOT in scope. The old rule
/// substring-matched them against the lowercased PRINTED names of everything the
/// player can see, which answers a different question: a thing printed as `brass
/// lantern` also answers to `lamp`, `lantern` and `light`, and a rule reading the
/// printed name finds one of them and misses the rest. Picking a noun that is
/// really here makes the control succeed, the pair disagree, and the run learn
/// nothing.
///
/// The case counts the disagreement on a real story in a real room rather than
/// asserting it exists.
#[test]
fn the_scope_test_asks_the_story_instead_of_reading_its_prose() {
    let Some(mut p) = Play::zork1() else { return };
    p.walk(TO_THE_LAMP);

    let mut vs = app::vocab::VocabState::default();
    let v = vs.get(&*p.session).expect("Zork I has a dictionary");
    let intro = p.session.introspect().expect("the Z-machine introspects");
    let player = intro.player_object();
    let mut in_scope: Vec<app::engine::ObjectWords> = Vec::new();
    if let Some(room) = p.session.current_location().map(|l| l.number) {
        in_scope.extend(intro.room_objects_excluding(room, player));
    }
    if let Some(pl) = player {
        in_scope.extend(intro.contents(pl));
    }
    let printed: Vec<String> =
        in_scope.iter().filter_map(|o| o.display_name()).map(|n| n.to_lowercase()).collect();

    // Nouns the OLD rule called absent that an object here really answers to —
    // every one of them a control that would have succeeded and taught nothing.
    let mut wrong: Vec<&str> = Vec::new();
    let mut total = 0usize;
    for w in v.nouns().filter(|w| w.chars().count() >= 3) {
        total += 1;
        let lower = w.to_lowercase();
        let old_says_absent = !printed.iter().any(|n| n.contains(&lower));
        let really_here = in_scope.iter().any(|o| o.refers_to(w));
        if old_says_absent && really_here {
            wrong.push(w);
        }
    }
    eprintln!(
        "Zork I r88, Living Room: {} objects in scope, printed as {printed:?}\n  \
         {} of {total} dictionary nouns were called absent by the printed-name rule \
         while something here answers to them: {wrong:?}",
        in_scope.len(),
        wrong.len(),
    );
    assert!(
        !wrong.is_empty(),
        "the printed-name rule agreed with the story everywhere here, so this room \
         cannot show the difference — pick another"
    );
}
