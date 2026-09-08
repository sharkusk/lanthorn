//! Other-v6-title headless smokes (SQ-0453, follow-up to SQ-0186/Lane S of
//! `docs/superpowers/plans/2026-07-22-v6-completion.md`).
//!
//! The original version of this file booted `stories/Arthur.blb` /
//! `stories/Shogun.blb` / `stories/Journey.blb`, but discovered (and recorded)
//! that all three `.blb` files in this worktree's (gitignored, local-only)
//! `stories/` are resources-only Blorb sidecars with no `ZCOD` executable —
//! every test skipped. The real bootable executables are the bare `.z6`
//! files sitting next to those sidecars (`arthur-r74-s890714.z6`,
//! `journey-r83-s890706.z6`), each with its picture/sound resources resolved
//! via `blorb::resolve_resource_blorb` — exactly the setup
//! `v6_shogun_gameplay.rs` uses for `shogun-r322-s890706.z6`. Shogun itself is
//! dropped from this file since it already has dedicated gameplay coverage
//! there.
//!
//! For each title: boot headless, assert no fault + no quit at the first
//! input and a `Layered` screen-model root, then drive 3 inputs (whatever
//! `pending_input()` asks for) asserting no fault/quit and no leaked control
//! characters in the transcript on each turn.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// All control chars (except newline) in `s` — must always be empty for
/// anything the renderer will touch.
fn ctrl_chars(s: &str) -> Vec<char> {
    s.chars().filter(|c| c.is_control() && *c != '\n').collect()
}

/// Boot `story_path` as a bare v6 Z-machine executable (mirroring
/// `v6_shogun_gameplay.rs`'s setup) and drive 3 turns, asserting no fault, no
/// quit, a `Layered` screen-model root, and no control-character leakage into
/// the transcript on any turn.
fn smoke_v6_title(title: &str, story_path: &PathBuf) {
    let Ok(story_bytes) = std::fs::read(story_path) else {
        eprintln!("SKIP {title}: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window(), None, None)
            .unwrap_or_else(|e| panic!("{title} should load and boot without a ZError, got {e:?}"));

    assert!(!session.quit, "{title} quit during boot (fault or premature quit), before reaching the first prompt");
    assert!(
        session.machine.fault_trace.is_none(),
        "{title} faulted during boot: {:?}",
        session.machine.fault_trace.as_ref().map(|t| t.to_lines())
    );

    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    let screen = session.screen();
    let WinNode::Layered(items) = &screen.root else {
        panic!("{title}'s v6 screen() root must be WinNode::Layered, got {:?}", screen.root);
    };
    eprintln!("{title}: {} layered window(s) at first input, content_size={:?}", items.len(), screen.content_size);
    for it in items {
        eprintln!("  window x_px={} y_px={} w_px={} h_px={}", it.x_px, it.y_px, it.w_px, it.h_px);
    }

    let _ = session.take_transcript();
    for turn in 0..3 {
        let result = match session.pending_input() {
            InputKind::Line => session.submit("look"),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        assert!(!result.quit, "{title} quit on turn {turn}");
        assert!(result.fault.is_none(), "{title} faulted on turn {turn}: {:?}", result.fault);
        assert_eq!(
            ctrl_chars(&result.transcript),
            Vec::<char>::new(),
            "{title} turn {turn}: control chars leaked into the transcript: {:?}",
            result.transcript
        );
    }

    eprintln!("{title} diagnostics: {:?}", session.machine.diagnostics());
}

#[test]
fn arthur_v6_boot_smoke() {
    smoke_v6_title("Arthur", &stories_dir().join("arthur-r74-s890714.z6"));
}

#[test]
fn journey_v6_boot_smoke() {
    smoke_v6_title("Journey", &stories_dir().join("journey-r83-s890706.z6"));
}
