//! Room description capture on Arthur (SQ-1675). Release 74, serial 890714, IBM profile.
//!
//! Arthur prints an upper-case heading only when you ARRIVE (`CHURCH`, `CHURCHYARD`); a bare
//! `look` prints headingless prose (the room's name lives only in the status bar), so it captures
//! nothing. Skip-if-missing (gitignored story).

use std::path::PathBuf;

use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind, TurnResult};

/// Boots `file` the way `startup.rs` does (profile -> `MachineBoot` -> `new_for_machine`), colours
/// honoured. `None` (a vacuous skip) when the gitignored story is absent.
fn boot(file: &str) -> Option<GameSession> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let profile = InterpreterProfile::resolve(&path, None, PictureOverride::Unset.flavour(), None);
    let mut picts = PictSource::resolve_with_override(&path, PictureOverride::Unset, None);
    let dims = picts.all_pict_dims();
    let honoured = !picts.declines_game_colours(profile.default_colours());
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, honoured, false, false, dims, None, None, &boot)
        .expect("story boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// One turn: `cmd` at a line prompt, otherwise a key (`key`) or an empty event.
fn turn(s: &mut GameSession, cmd: &str, key: u8) -> TurnResult {
    match s.pending_input() {
        InputKind::Line => s.submit(cmd),
        InputKind::Char => s.submit_char(key),
        InputKind::Event => s.submit(""),
    }
}

#[test]
fn arthur_r74_captures_arrival_headings_and_a_bare_look_captures_nothing() {
    let Some(mut s) = boot("arthur-r74-s890714.z6") else { return };
    // Turns 1-14: decline the restore question (n), then key through the intro to the churchyard.
    for _ in 0..14 {
        let _ = turn(&mut s, "", b'n');
    }
    assert_eq!(s.pending_input(), InputKind::Line, "non-vacuity: at the command prompt after 14 keys");

    // Turn 15: `e` -> CHURCH.
    let r = turn(&mut s, "e", b'n');
    assert_eq!(r.location.as_ref().map(|l| l.name.as_str()), Some("church"), "non-vacuity: arrived");
    let d = r.description.expect("`e` -> CHURCH: description captured");
    assert!(d.contains("simple, one-room building"), "church: {d:?}");
    assert!(!d.contains("CHURCH"), "heading excluded: {d:?}");

    // Turn 16: `w` -> CHURCHYARD.
    let r = turn(&mut s, "w", b'n');
    assert_eq!(r.location.as_ref().map(|l| l.name.as_str()), Some("churchyard"));
    let d = r.description.expect("`w` -> CHURCHYARD: description captured");
    assert!(d.contains("You return to the churchyard"), "churchyard: {d:?}");
    assert!(!d.contains("CHURCHYARD"), "heading excluded: {d:?}");

    // Turn 17: a bare `look` prints headingless prose, so nothing is captured (it must not be
    // mistaken for a description).
    let r = turn(&mut s, "look", b'n');
    assert!(r.transcript.contains("bright moonlight"), "non-vacuity: look printed the room: {:?}", r.transcript);
    assert_eq!(r.description, None, "a headingless `look` captures nothing");
}
