//! Local-only Glulx room-detection verification (stories/ is gitignored; these
//! are #[ignore]d). Run: cargo test -p app --test glulx_room_detection -- --ignored --nocapture
use std::path::PathBuf;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

// Extract a Glulx image from a Blorb (or pass through a raw .ulx), like
// gvm/tests/accel_story_equivalence.rs::extract_glulx. Returns None for non-Glulx.
fn glulx_image(name: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(stories_dir().join(name)).ok()?;
    if !blorb::Blorb::is_blorb(&bytes) {
        return Some(bytes);
    }
    let b = blorb::Blorb::parse(bytes).ok()?;
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Some(data.to_vec()),
        _ => None,
    }
}

/// Some Inform 7 Glulx games show a "Press any key to begin" splash (a
/// `NeedChar` prompt) before the opening room heading prints. Dismiss up to
/// `max_keys` of those with an Enter keypress each, stopping as soon as
/// `current_location()` resolves. A no-op when the room is already known
/// (the common case: most games print the room heading during boot, before
/// the first real input prompt).
fn drive_to_room(s: &mut GlulxSession, max_keys: u32) -> Option<app::engine::LocationInfo> {
    for _ in 0..max_keys {
        if s.current_location().is_some() {
            break;
        }
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        s.submit_key(KeyInput::Enter);
    }
    s.current_location()
}

#[test]
#[ignore]
fn starting_rooms_resolve() {
    // Superluminal_Vagrant_Twin renders inline command-hint words ("credits",
    // "land") in `GlkStyle::Subheader` — the same style as the real room heading
    // "Orbiting Boony" — but mid-line, whereas the heading is on its own line.
    // The line-start discriminator in `capture_heading` ignores the inline links,
    // so it resolves correctly.
    let cases = [
        ("FooFoo.gblorb.blorb", "Studio Apartment"),
        ("Superluminal_Vagrant_Twin.gblorb.blorb", "Orbiting Boony"),
        ("Sub_Rosa.gblorb.blorb", "Leathery Cliff"),
        ("Dr Ludwig and the Devil.gblorb", "Laboratory"),
        ("TAKE.gblorb", "War Chest"),
    ];
    for (file, want) in cases {
        let Some(image) = glulx_image(file) else { continue };
        let mut s = GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[]).expect("GlulxSession::new");
        // FooFoo and "Dr Ludwig and the Devil" show a splash / dialogue that
        // needs a few keypresses before the room heading prints; Sub_Rosa and
        // TAKE already have it after boot. 5 is comfortably above the largest
        // observed count (3, for Dr Ludwig).
        let got = drive_to_room(&mut s, 5).map(|r| r.name);
        assert_eq!(got.as_deref(), Some(want), "{file}: starting room");
    }
}

#[test]
#[ignore]
fn pregame_menus_have_no_room() {
    let files = [
        "Zozzled.gblorb.blorb",
        "Brain_Guzzlers_from_Beyond!.gblorb.blorb",
        "And_Then_You_Come_to_a_House.gblorb.blorb",
    ];
    for file in files {
        let Some(image) = glulx_image(file) else { continue };
        let s = GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[]).expect("GlulxSession::new");
        assert_eq!(s.current_location(), None, "{file}: pre-game menu should have no room");
    }
}
