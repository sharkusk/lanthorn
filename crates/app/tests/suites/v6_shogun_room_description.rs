//! Room description capture on Shogun (SQ-1675). Release 322, serial 890706, IBM profile.
//! Skip-if-missing (gitignored story).

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
fn shogun_r322_captures_the_bridge_description_on_turn_2_and_on_look() {
    let Some(mut s) = boot("shogun-r322-s890706.z6") else { return };
    let _ = turn(&mut s, "", 13); // turn 1: boot menu
    // Turn 2: START -> the Bridge heading and description.
    let r = turn(&mut s, "", 13);
    assert_eq!(r.location.as_ref().map(|l| l.name.as_str()), Some("Bridge"), "non-vacuity: arrived");
    let d = r.description.expect("turn 2: Bridge description captured");
    assert!(d.contains("the bridge of the Erasmus"), "turn 2: {d:?}");
    assert!(!d.starts_with("Bridge"), "heading excluded: {d:?}");

    // Turn 3: `look` reprints the heading, so the capture is the Bridge's again.
    let r = turn(&mut s, "look", 13);
    let d = r.description.expect("turn 3 `look`: description captured");
    assert!(d.contains("the bridge of the Erasmus"), "turn 3: {d:?}");
    assert!(!d.contains("look"), "no command echo: {d:?}");
}
