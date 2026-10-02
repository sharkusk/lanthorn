//! Room description capture on Journey (SQ-1675). Release 83, serial 890706, IBM profile.
//!
//! Journey prints no room headings into its transcript window, so nothing is captured. The guard:
//! its opening prose and a `look` yield no description. Skip-if-missing (gitignored story).

use std::path::PathBuf;

use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

#[test]
fn journey_r83_prints_no_headings_so_captures_no_description() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/journey-r83-s890706.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
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
        .expect("Journey boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();

    // Turns 1-3: restore question, blank, title card; turn 4 `look` -> the prologue prose.
    let mut last = String::new();
    for (i, cmd) in ["", "", "", "look"].into_iter().enumerate() {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(cmd),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        assert_eq!(r.description, None, "turn {}: Journey captures no description", i + 1);
        last = r.transcript;
    }
    assert!(last.contains("It was a Golden Age"), "non-vacuity: the prologue printed: {last:?}");
}
