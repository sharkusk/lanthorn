//! SQ-1548: the shadow-probe answers a headless host would otherwise never
//! collect.
//!
//! `host::turn::finish_command_turn` arms all three probes exactly as the TUI
//! does — `vocab::offer_vocabulary` (a vetted "try instead" offer),
//! `return_probe::arm_return_search` and `random_exit_probe::arm_for_finished_turn`
//! (host/turn.rs) — but arming is not collecting. Before SQ-1548 the only thing
//! that ever called `state.probe.poll()` and routed what came back was the
//! binary's own `loop_tick::poll_shadow_answers`, so a host driving sessions
//! through `app::host` alone got none of it: an offer sat in `vocab_pending`
//! forever and a return-probe/random-exit discovery never reached the `Mapper`.
//! `app::host::probe::poll` is that routing, moved into the library.
//!
//! Both cases below drive `boot_story` + `finish_command_turn` — no TUI, no
//! event loop, no terminal — and collect purely by calling `host::probe::poll`
//! on a bounded loop, the way a headless host's own tick would.
//!
//! | fixture | what |
//! |---|---|
//! | `zork1-r88-s840726.z3` | `illuminate lamp` at the door (dropped) then in the Living Room (vetted "try instead — light") |
//! | `zork1-r88-s840726.z3` | `north` from West of House, and the return-probe edge it discovers |
//!
//! `stories/` is gitignored commercial media; both cases skip vacuously without it.

use std::path::{Path, PathBuf};

use app::config::Config;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::state::TranscriptKind;

use mapper::direction::Direction;

use crate::fixture_paths::fixture_path;

fn headless_config(home: &Path) -> Config {
    Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        enable_sound: false,
        ..Config::default()
    }
}

fn boot(story: PathBuf, home: &Path) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg: headless_config(home),
        data_base: home.join("saves"),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

/// Submit `cmd` and apply the turn exactly as the TUI's submit path does —
/// this is what arms the probes (`host::turn::finish_command_turn`, which
/// calls `vocab::offer_vocabulary` / `return_probe::arm_return_search` /
/// `random_exit_probe::arm_for_finished_turn` internally).
fn command(b: &mut BootedStory, cmd: &str, tidy: &mut u32) {
    let result = b.session.submit(cmd);
    let _ = app::host::finish_command_turn(
        cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid,
        &b.arc_file, None, tidy,
    );
}

/// Collect through `host::probe::poll` alone, on a bounded loop, until nothing
/// is left outstanding — the shape a headless host's own idle tick takes,
/// since `ShadowProbe::poll` is a nonblocking channel read and not a deadline
/// (see `host::probe`'s own docs). Panics rather than reporting a false
/// negative if the worker never answers within the deadline.
fn drain_probe(b: &mut BootedStory, tidy: &mut u32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app::host::probe::poll(&mut b.state, &mut b.mapper, tidy);
        let outstanding = b.state.probe.is_busy()
            || b.state.vocab_pending.is_some()
            || b.state.return_search.is_some()
            || b.state.random_exit_search.is_some();
        if !outstanding {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "host::probe::poll never drained the shadow within 10s"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn assists(b: &BootedStory) -> Vec<String> {
    b.state
        .transcript
        .iter()
        .zip(&b.state.transcript_kinds)
        .filter(|(_, k)| **k == TranscriptKind::Assist)
        .map(|(l, _)| l.clone())
        .collect()
}

fn room_named(b: &BootedStory, name: &str) -> Option<mapper::graph::RoomId> {
    b.mapper.graph.rooms().find(|r| r.name == name).map(|r| r.id)
}

// ── The vetted vocabulary offer ─────────────────────────────────────────────

/// **The case SQ-1121 exists for, reached through the host alone.**
/// `illuminate lamp` at the front door does nothing (the lamp is not there);
/// the SAME command five rooms later, in the Living Room, lights it — and the
/// probe watches that happen in a silent copy and recommends `light`. Nothing
/// here is TUI code: `boot_story`, `finish_command_turn` and `host::probe::poll`
/// are the whole of it.
#[test]
fn a_mistyped_word_the_probe_can_vet_yields_the_try_instead_line_through_the_host_alone() {
    let story = fixture_path("zork1-r88-s840726.z3");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-probe-vocab");
    let mut b = boot(story, &home);
    // Fresh AppState: skip the once-per-session preamble line so the assist
    // filter below reads only the offer itself.
    b.state.assist_preamble_shown = true;
    let mut tidy = 0u32;

    // At the door: `light` is a Zork word, but the lamp is not here, so the
    // probe watches it fail and the offer is dropped.
    command(&mut b, "illuminate lamp", &mut tidy);
    drain_probe(&mut b, &mut tidy);
    assert_eq!(assists(&b), Vec::<String>::new(), "the lamp is not here — nothing is recommended");

    // Walk to the Living Room, where the lamp is.
    for cmd in ["north", "east", "open window", "enter window", "west"] {
        command(&mut b, cmd, &mut tidy);
        drain_probe(&mut b, &mut tidy);
    }
    assert_eq!(
        room_named(&b, "Living Room"),
        b.mapper.graph.current(),
        "five turns in, the player is in the Living Room"
    );

    command(&mut b, "illuminate lamp", &mut tidy);
    drain_probe(&mut b, &mut tidy);
    assert_eq!(
        assists(&b),
        vec!["try instead — light"],
        "the shadow watched `light lamp` turn the lantern on, through the host alone: {}",
        b.state.transcript.join("\n")
    );
}

// ── The return-probe map edge ───────────────────────────────────────────────

/// **The walk `return_probe.rs`'s TUI-path suite already covers**
/// (`zork1_learns_the_way_back_which_is_not_the_way_it_came`), reached here
/// through `boot_story` + `finish_command_turn` + `host::probe::poll` alone.
/// West of House, `north` into North of House: south is boarded, east reaches
/// Behind House (unmapped, so its answer is dropped), and west is the way
/// back — the edge a headless host would otherwise never see minted.
#[test]
fn a_return_probe_walk_adds_the_same_map_edge_through_the_host_alone() {
    let story = fixture_path("zork1-r88-s840726.z3");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-probe-return");
    let mut b = boot(story, &home);
    let mut tidy = 0u32;
    assert!(b.state.config.return_probe, "on by default");

    let west = b.mapper.graph.current().expect("the map starts West of House");
    command(&mut b, "north", &mut tidy);
    let north = b.mapper.graph.current().expect("north of house");
    assert_ne!(north, west, "the move actually crossed something");

    // Nothing but `host::probe::poll`, bounded, collects the search's answers.
    drain_probe(&mut b, &mut tidy);

    let edge = b
        .mapper
        .graph
        .connections()
        .iter()
        .find(|c| c.origin == north && c.dir == Direction::W)
        .map(|c| c.dest);
    assert_eq!(
        edge,
        Some(west),
        "west is the way back, discovered and minted through the host alone: rooms {:?}",
        b.mapper.graph.rooms().map(|r| (r.id, r.name.clone())).collect::<Vec<_>>()
    );
    assert!(b.state.probe.probes >= 2, "south refused, then west succeeded — at least two attempts");
}
