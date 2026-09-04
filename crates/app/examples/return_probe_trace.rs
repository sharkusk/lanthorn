//! Instrument (SQ-0785): what the return search ASKS and what comes back, one
//! move at a time — beside `return_probe_cost`, which measures what it costs.
//!
//!   cargo run -p lanthorn --example return_probe_trace -- \
//!       --story zork1-invclues-r52-s871125.z5 --walk 'n;n;n'
//!
//! For each crossing it prints the rooms, the gate's two conditions, the
//! candidate order, every attempt with the room it resolved to, and the passage
//! recorded. That is enough to tell the three ways a search can come back
//! empty-handed apart: the gate refused to arm, the candidates were filtered
//! away, or an attempt landed somewhere the map could not identify.
//!
//! **An example rather than a test, deliberately.** `-p lanthorn` links all fourteen
//! test group binaries whatever a filter selects, so reaching one case costs the
//! full ~150s link; this builds the `app` lib and itself, and answered SQ-0785's
//! two field reports in about 26s each.
//!
//! Both of those reports were about a room's IDENTITY rather than about the
//! search, which is why the second half exists — see its comment. `--walk` takes
//! `;`-separated commands and answers whichever prompt the story is waiting on,
//! so a title card costs nothing.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::Engine;
use app::probe::ShadowRecipe;
use app::state::AppState;
use mapper::mapper::Mapper;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut story = "zork1-invclues-r52-s871125.z5".to_string();
    let mut walk = "n;n;n".to_string();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--story" => { story = args[i + 1].clone(); i += 2 }
            "--walk" => { walk = args[i + 1].clone(); i += 2 }
            _ => i += 1,
        }
    }

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(&story);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let inner = match app::hints::extract_story(bytes.clone()).expect("loads") {
        app::hints::LoadedStory::ZCode(b) => b,
        _ => panic!("z-code only"),
    };
    let mut s = app::session::GameSession::new_with_trace(
        inner.clone(), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("boots");
    s.set_strip_prompt(false);
    let mut session: Box<dyn Engine> = Box::new(s);

    let mut state = AppState::default();
    state.config.return_probe = true;
    state.probe.arm(ShadowRecipe {
        story_bytes: Arc::new(inner),
        store: PathBuf::new(),
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    });
    let mut mapper = Mapper::default();
    let mut death = app::session::DeathWatch::default();

    // Settle the opening: answer whichever prompt the game is actually waiting on.
    for _ in 0..4 {
        let r = match session.pending_input() {
            app::session::InputKind::Char => session
                .submit_key(app::engine::KeyInput::Char(' '))
                .unwrap_or_else(|| session.submit("")),
            _ => session.submit(""),
        };
        app::session::apply_turn(&mut mapper, "", &r, &mut death);
    }
    let r = session.submit("look");
    app::session::apply_turn(&mut mapper, "look", &r, &mut death);
    println!("start: {:?}", room(&mapper, mapper.graph.current()));

    for cmd in walk.split(';').map(str::trim).filter(|c| !c.is_empty()) {
        let before = mapper.graph.current();
        let r = session.submit(cmd);
        app::session::apply_turn(&mut mapper, cmd, &r, &mut death);
        let here = mapper.graph.current();
        println!("\n=== {cmd:?}: {:?} -> {:?}", room(&mapper, before), room(&mapper, here));
        println!("    reply: {:?}", r.transcript.trim().lines().next().unwrap_or(""));

        if let (Some(h), Some(o)) = (here, before) {
            let out = mapper.graph.connections().iter().any(|c| c.origin == o && c.dest == h);
            let back = mapper.graph.connections().iter().any(|c| c.origin == h && c.dest == o);
            println!("    gate: outbound-edge={out} return-already-known={back}");
            println!(
                "    candidates: {:?}",
                mapper.graph.probe_candidates(h, mapper::direction::parse_direction(cmd))
            );
        }

        app::return_probe::arm_return_search(
            &mut state, &mapper, &*session, cmd, before,
            &mut app::engine::TurnSave::default(),
        );
        if state.return_search.is_none() {
            println!("    NO SEARCH ARMED");
            continue;
        }

        // settle_return_search, unrolled so every attempt is visible.
        let mut found = None;
        while state.return_search.is_some() {
            if !app::return_probe::pump_return_search(&mut state) {
                if state.return_search.is_some() {
                    println!("    pump refused (shadow busy or queue empty)");
                }
                break;
            }
            let Some(answer) = state.probe.settle() else {
                println!("    settle: NO ANSWER (seam broken)");
                break;
            };
            if !app::return_probe::owns(&state, answer.token) {
                continue;
            }
            if let Some(run) = &answer.run {
                for st in &run.steps {
                    println!(
                        "      try {:<10} -> loc={:?} quit={} escaped={} | {:?}",
                        st.command,
                        st.location,
                        st.quit,
                        st.escaped,
                        st.reply.trim().lines().next().unwrap_or("")
                    );
                }
            } else {
                println!("      try ??? -> run=None");
            }
            if let Some(p) = app::return_probe::deliver(&mut state, &mut mapper, &answer) {
                found = Some(p);
                break;
            }
        }
        println!("    FOUND: {found:?}");
    }

    // ── Live vs a restored shadow, on the SAME candidates ────────────────────
    //
    // The half that found SQ-0785's second defect. The probe seam runs its shadow
    // on a worker thread and owns it, so nothing above can look at that shadow's
    // SCREEN — and on a v4+ story the screen is where `detect_location` reads the
    // room name from. This rebuilds the same situation locally, where the machine
    // is reachable, and prints the three facts side by side: what the live engine
    // resolves, what a restored shadow resolves, and what each of them has in the
    // upper window at the time.
    //
    // A `method` of `StatusName` or `NameOnly` where the live engine says
    // `PlayerParent` is the tell: the ladder fell off the object tree onto the
    // text, and the number beside it is a guess dressed as an answer.
    let Some(here) = mapper.graph.current() else { return };
    let candidates = mapper.graph.probe_candidates(here, None);
    let save = session.save_state();

    println!("\n--- live, from {:?} ---", room(&mapper, Some(here)));
    for d in &candidates {
        let keep = session.save_state();
        let cmd = mapper::direction::long_label(*d);
        let r = session.submit(cmd);
        report(cmd, &r, None);
        session.restore_state(&keep).expect("the live engine takes its own state back");
    }

    println!("--- a fresh shadow, restored to that same instant ---");
    let mut sh = app::session::GameSession::new_with_trace(
        boot_bytes(&story), true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("boots");
    sh.set_strip_prompt(false);
    for d in &candidates {
        sh.restore_state(&save).expect("the shadow takes the live state");
        let cmd = mapper::direction::long_label(*d);
        let r = sh.submit(cmd);
        let upper = zvm::location::status_line_room_name(
            &sh.machine.screen.upper,
            sh.machine.screen.upper_window_rows,
        );
        report(cmd, &r, Some(upper));
    }
}

/// One resolved step: the number, HOW it was arrived at, and — for the shadow —
/// the upper-window text the v4+ ladder read it out of.
fn report(cmd: &str, r: &app::session::TurnResult, upper: Option<Option<String>>) {
    println!(
        "  {cmd:<10} loc={:?} method={:?} | {:?}",
        r.location.as_ref().map(|l| (l.number, l.name.clone())),
        r.location_method,
        r.transcript.trim().lines().next().unwrap_or("")
    );
    if let Some(upper) = upper {
        println!("             upper window = {upper:?}");
    }
}

fn boot_bytes(story: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(story);
    let bytes = std::fs::read(&path).expect("story");
    match app::hints::extract_story(bytes).expect("loads") {
        app::hints::LoadedStory::ZCode(b) => b,
        _ => panic!("z-code only"),
    }
}

fn room(m: &Mapper, id: Option<mapper::graph::RoomId>) -> Option<(mapper::graph::RoomId, String)> {
    id.and_then(|i| m.graph.room(i).map(|r| (i, r.name.clone())))
}
