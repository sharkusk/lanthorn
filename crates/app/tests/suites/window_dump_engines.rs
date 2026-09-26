//! `/dump-windows` must describe the engine it is actually looking at
//! (SQ-0699 follow-up).
//!
//! The command reaches each engine through `Engine::window_dump`. The Z-machine
//! and Glulx both override it; **Scott did not**, so it fell through to the
//! trait default — which is written for the Z-machine. That default looks for a
//! GRID window, and Scott's layout is buffer-over-buffer, so every Scott game
//! reported the same two lies: a size of `Grid 0x0`, and the engine name
//! "Z-machine simple path".
//!
//! The Scott case runs everywhere (`tiny_cave.dat` is a checked-in fixture).
//! The Glulx case needs a gitignored story and skips vacuously without it.

use app::engine::Engine;
use app::scott_session::ScottSession;

use crate::fixture_paths::fixture_path;

fn tiny_cave() -> Vec<u8> {
    include_bytes!("../../../scott/tests/tiny_cave.dat").to_vec()
}

#[test]
fn scott_dump_describes_a_scott_screen() {
    let session = ScottSession::new(tiny_cave(), None).expect("tiny_cave.dat loads");
    let dump = session.window_dump();
    let text = dump.join("\n");

    // The two defects of the inherited Z-machine default.
    assert!(
        !text.contains("Z-machine"),
        "a Scott game must not be described as a Z-machine one:\n{text}"
    );
    assert!(
        !text.contains("Grid 0x0"),
        "the Z-machine default reported a grid Scott does not have:\n{text}"
    );

    // What a Scott screen actually is.
    assert!(text.contains("Scott Adams"), "names its own engine:\n{text}");
    assert!(text.contains("picture:"), "reports the room-picture band:\n{text}");
    assert!(text.contains("room panel:"), "reports the room panel:\n{text}");
    assert!(text.contains("transcript:"), "reports the transcript below it:\n{text}");

    // The panel's live lines are the point — a dump that says "6 lines" without
    // showing them cannot tell a blank panel from a populated one.
    assert!(
        dump.iter().any(|l| l.contains("line ")),
        "the panel's live lines are quoted:\n{text}"
    );
    // tiny_cave's start room and its exits, straight from the VM's room block —
    // proof the lines are live state, not a placeholder.
    assert!(
        text.contains("sunlit forest clearing"),
        "the quoted lines are the REAL room block:\n{text}"
    );
    assert!(text.contains("Obvious exits: Down."), "…including its exit list:\n{text}");
}

/// Glulx already had a real dump (a window tree with ids, rects and per-canvas
/// opacity). This pins that it stays real — the same regression Scott suffered
/// would be invisible otherwise, since a wrong dump still returns lines.
#[test]
fn glulx_dump_describes_the_window_tree() {
    let path = fixture_path("advent.blb");
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let blorb = blorb::Blorb::parse(raw.clone()).expect("valid blorb");
    let bytes = match app::hints::extract_story(raw).expect("extract") {
        app::hints::LoadedStory::Glulx(b) => b,
        _ => panic!("expected Glulx"),
    };
    let session =
        app::glulx_session::GlulxSession::new(bytes, 138, 51, true, true, false, (8.0, 18.0), Some(blorb), &[])
            .expect("session");
    let text = session.window_dump().join("\n");

    assert!(!text.contains("Z-machine"), "a Glulx game is not a Z-machine one:\n{text}");
    assert!(text.contains("Window layout"), "reports the tree:\n{text}");
    assert!(text.contains("Buffer") && text.contains("(primary)"), "names the primary buffer:\n{text}");
    assert!(
        text.contains("canvas=") && text.contains("opaque="),
        "advent.blb opens a graphics toolbar — its canvas diagnostics are what make \
         'the game never painted this window' visible:\n{text}"
    );
}
