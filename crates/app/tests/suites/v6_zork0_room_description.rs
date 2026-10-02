//! Room description capture on Zork Zero (SQ-1675). Release 393, serial 890714, IBM profile.
//!
//! The room inspector's description used to be gated off for every Version 6 story. Real v6
//! output streams only the game's transcript window, so the capture works; these cases walk the
//! real story. Skip-if-missing (gitignored story).

use std::path::PathBuf;

use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind, TurnResult};

/// Boots the story the way `startup.rs` does (profile -> `MachineBoot` -> `new_for_machine`),
/// colours honoured. `None` (a vacuous skip) when the gitignored story is absent.
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

/// One turn: `cmd` at a line prompt, otherwise a key or an empty event.
fn turn(s: &mut GameSession, cmd: &str) -> TurnResult {
    match s.pending_input() {
        InputKind::Line => s.submit(cmd),
        InputKind::Char => s.submit_char(13),
        InputKind::Event => s.submit(""),
    }
}

#[test]
fn zork0_r393_captures_the_banquet_hall_then_the_entrance_hall_description() {
    let Some(mut s) = boot("zork0-r393-s890714.z6") else { return };
    // Turn 1 (one key past the boot banner): the intro prints the Banquet Hall heading and body.
    let r = turn(&mut s, "");
    assert_eq!(r.location.as_ref().map(|l| l.name.as_str()), Some("Banquet Hall"), "non-vacuity: arrived");
    let d = r.description.expect("turn 1: Banquet Hall description captured");
    assert!(d.contains("The hall is filled to capacity"), "turn 1: {d:?}");
    assert!(!d.starts_with("Banquet Hall"), "the heading is not part of the description: {d:?}");

    // Turn 2 is the game's own "[I beg your pardon?]"; turn 3 is `look`: captured again.
    let _ = turn(&mut s, "");
    let r = turn(&mut s, "look");
    let d = r.description.expect("turn 3 `look`: description captured");
    assert!(d.contains("The hall is filled to capacity"), "turn 3: {d:?}");
    assert!(!d.contains("look"), "no command echo: {d:?}");

    // Turn 4: `w` -> Entrance Hall, whose own description replaces it.
    let r = turn(&mut s, "w");
    assert_eq!(r.location.as_ref().map(|l| l.name.as_str()), Some("Entrance Hall"));
    let d = r.description.expect("turn 4 `w`: Entrance Hall description captured");
    assert!(d.contains("where visitors enter the castle proper"), "turn 4: {d:?}");
    assert!(!d.contains("hall is filled to capacity"), "not the previous room's text: {d:?}");
    assert!(!d.starts_with("Entrance Hall"), "heading excluded: {d:?}");
}
