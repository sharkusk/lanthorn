//! SCRATCH instrument (SQ-0785): what one return probe COSTS, per move, on the
//! three stories the quest named — Zork I, Coloratura, Counterfeit Monkey.
//!
//! The feature is off by default because it runs the player's game a few extra
//! turns in private after every move that opens a gap, and "a few" is the number
//! that decides whether that is affordable. Two figures are wanted and they are
//! very different:
//!
//! * **at 12 candidates** — the worst case, a room from which nothing leads back,
//!   so the search walks its whole list;
//! * **after filtering** — what actually happens in play, where
//!   `probe_candidates` has already dropped everything the player walked and
//!   everything an earlier search covered, and the priority order usually
//!   answers in one or two.
//!
//! Both are WORKER time, not the player's: `ask` returns immediately and the
//! answer is collected a beat later. What the player's thread pays is the
//! `save_state()` per attempt, which this measures separately for exactly that
//! reason — it is the one part of the cost that is not free.
//!
//! Usage:
//!   cargo run -p lanthorn --example return_probe_cost -- [--story <path>] [--walk 'n;e;s']

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use app::engine::Engine;
use app::probe::ShadowRecipe;
use app::state::AppState;
use mapper::direction::PROBE_DIRS;
use mapper::mapper::Mapper;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(path: &Path, bytes: Vec<u8>) -> Option<(Box<dyn Engine>, ShadowRecipe)> {
    // `<story-file>.save`, appended — NOT `with_extension`, which replaces
    // `.gblorb` and misses `CounterfeitMonkey-11.gblorb.save` entirely. That is
    // the difference between a shadow that reads the game's init cache and one
    // that re-runs the initialisation from cold.
    let store = PathBuf::from(format!("{}.save", path.display()));
    // The live game's own Glk file VFS, which lives INSIDE its save directory as
    // `default.glkvfs` — Counterfeit Monkey checks a 52-byte marker in it and
    // only then `@restore`s the startup cache beside it, so the store without the
    // VFS is the same cold boot as neither (SQ-1124).
    let vfs = std::fs::read(store.join("default.glkvfs")).unwrap_or_default();
    let recipe = ShadowRecipe {
        story_bytes: Arc::new(bytes.clone()),
        // The live game's own persistent data, which the shadow READS and never
        // writes — the SQ-1124 boot fix, and the whole of why Counterfeit Monkey
        // is a fraction of a second here rather than two.
        store: if store.is_dir() { store } else { PathBuf::new() },
        vfs_bytes: Arc::new(vfs),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    };
    match app::hints::extract_story(bytes).ok()? {
        app::hints::LoadedStory::ZCode(b) => {
            let mut s = app::session::GameSession::new_with_trace(
                b, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
            )
            .ok()?;
            s.set_strip_prompt(false);
            Some((Box::new(s), recipe))
        }
        app::hints::LoadedStory::Glulx(b) => {
            // The LIVE constructor, with the game's own writable store — the
            // shadow's read-only twin is what the seam builds for itself.
            let s = app::glulx_session::GlulxSession::new_in(
                recipe.store.clone(),
                b,
                80,
                24,
                true,
                false,
                false,
                false,
                (8, 16),
                None,
                &recipe.vfs_bytes,
                Default::default(),
                false,
                None,
            )
            .ok()?;
            Some((Box::new(s), recipe))
        }
        app::hints::LoadedStory::Scott(_) => None,
    }
}

/// One story: boot it, walk it a little, and time the two shapes of search.
fn measure(name: &str, file: &str, walk: &[&str]) {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        println!("{name:22} SKIP (no {file})");
        return;
    };
    let Some((mut session, recipe)) = boot(&path, bytes) else {
        println!("{name:22} SKIP (would not boot)");
        return;
    };

    let mut state = AppState::default();
    state.config.return_probe = true;
    state.probe.arm(recipe);
    let mut mapper = Mapper::default();
    let mut death = app::session::DeathWatch::default();

    // Settle the opening, then walk in, so the measurement is of a game in play
    // rather than of a title screen.
    // A blank line or two first: these open on a title card and a "press any
    // key", and a harness that skips them measures a screen with no room on it.
    // These open on a title card and a "press any key", so the drive has to
    // answer whichever input the game is actually waiting on — a line typed at a
    // char prompt is swallowed and the harness measures a screen with no room on
    // it, which is how this instrument first reported "no room detected" for both
    // Glulx stories.
    let mut drive = |session: &mut Box<dyn Engine>, mapper: &mut Mapper, cmd: &str| {
        let r = match session.pending_input() {
            app::session::InputKind::Char => session
                .submit_key(app::engine::KeyInput::Char(' '))
                .unwrap_or_else(|| session.submit(cmd)),
            _ => session.submit(cmd),
        };
        if std::env::var("RP_TRACE").is_ok() {
            eprintln!(
                "--- {cmd:?} loc={:?} ---\n{}",
                r.location.as_ref().map(|l| (l.number, l.name.clone())),
                r.transcript.chars().take(300).collect::<String>()
            );
        }
        app::session::apply_turn(mapper, cmd, &r, &mut death);
    };
    for _ in 0..6 {
        drive(&mut session, &mut mapper, "");
    }
    // Counterfeit Monkey opens by asking "Can you hear me?" and will not move on
    // until it is answered; a harness that types `look` at it measures a screen
    // with no room on it forever.
    for cmd in ["yes", "andra", "look", "look"] {
        drive(&mut session, &mut mapper, cmd);
    }
    // Walk in until something actually crosses — a move that does not move the
    // player arms no search, and the gate rightly refuses a crossing whose way
    // back the walk itself has already recorded.
    let mut crossed = None;
    for cmd in walk.iter().copied().chain(["north", "south", "east", "west", "up", "down", "in", "out"])
    {
        let room_before = mapper.graph.current();
        let r = session.submit(cmd);
        app::session::apply_turn(&mut mapper, cmd, &r, &mut death);
        if room_before.is_some() && mapper.graph.current() != room_before {
            crossed = Some((cmd, room_before));
            break;
        }
    }
    let here = mapper.graph.current();

    // The one cost the PLAYER's thread pays, and it is paid ONCE per search
    // rather than once per attempt (see `ShadowProbe::snapshot`).
    let t = Instant::now();
    for _ in 0..4 {
        let _ = state.probe.snapshot(&*session);
    }
    let per_save = t.elapsed() / 4;

    // (a) The worst case: twelve candidates, none of which is the way back.
    // Driven straight through the seam so nothing is filtered.
    let cmds: Vec<String> =
        PROBE_DIRS.iter().map(|d| mapper::direction::long_label(*d).to_string()).collect();
    let boot_t = Instant::now();
    let _ = state.probe.run(&*session, &cmds[..1]); // first question boots the shadow
    let first = boot_t.elapsed();
    let before = state.probe.spent;
    let _ = state.probe.run(&*session, &cmds);
    let twelve = state.probe.spent - before;

    // (b) What play actually asks for, after both records have filtered — the
    // real number, since `probe_candidates` has already dropped everything the
    // player walked and everything an earlier search covered.
    let n = here
        .map(|h| {
            mapper
                .graph
                .probe_candidates(h, mapper::direction::parse_direction(walk.last().copied().unwrap_or("")))
                .len()
        })
        .unwrap_or(PROBE_DIRS.len());
    let n = n.min(app::probe::MAX_PROBES);
    let asked: Vec<String> = cmds[..n].to_vec();
    let before = state.probe.spent;
    let _ = state.probe.run(&*session, &asked);
    let filtered = state.probe.spent - before;

    println!(
        "{name:22} room={:<6} shadow boot {:>9.1?}   12 candidates {:>9.1?} ({:>8.1?}/cmd)   \
         after filtering {n:>2} {:>9.1?}   snapshot/search {:>8.1?}",
        here.is_some(),
        first,
        twelve,
        twelve / 12,
        filtered,
        per_save,
    );

    // (c) And what an actual crossing costs, end to end: the search armed by the
    // last move, run to whatever answer it reaches. This is the number that
    // matters in play — the priority order stops at the FIRST success, so it is
    // usually a small fraction of (a).
    let Some((cmd, room_before)) = crossed else {
        println!("{:22} in play: nothing crossed from here", "");
        return;
    };
    let before_probes = state.probe.probes;
    let before_spent = state.probe.spent;
    app::return_probe::arm_return_search(
        &mut state, &mapper, &*session, cmd, room_before,
        &mut app::engine::TurnSave::default(),
    );
    let found = app::return_probe::settle_return_search(&mut state, &mut mapper);
    println!(
        "{:22} in play: `{cmd}` → {:>2} command(s), {:>9.1?}, found {}",
        "",
        state.probe.probes - before_probes,
        state.probe.spent - before_spent,
        found.map(|p| format!("{:?}", p.dir)).unwrap_or_else(|| "nothing".into()),
    );
}

fn main() {
    let mut only: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--story" {
            only = args.next();
        }
    }
    println!("Return probe cost — worker time per question, and the one figure the main thread pays.\n");
    let corpus: [(&str, &str, &[&str]); 3] = [
        ("Zork I", "zork1-r88-s840726.z3", &["north"]),
        ("Coloratura", "Coloratura.gblorb.blorb", &["out"]),
        ("Counterfeit Monkey", "CounterfeitMonkey-11.gblorb", &["north"]),
    ];
    for (name, file, walk) in corpus {
        if only.as_deref().is_some_and(|o| !file.contains(o) && !name.contains(o)) {
            continue;
        }
        measure(name, file, walk);
    }
}
