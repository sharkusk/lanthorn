// SQ-0196: a Glulx game that styles its Normal text for a light interpreter
// (CounterfeitMonkey sets black-on-white via glk_stylehint_set) must paint the
// story pane with that background, not leave white text-islands on the dark
// theme (which looked like bad reversed/inverse text on the opening screen).
//
// Uses the real CM gblorb (git-ignored, kept locally under `stories/`); skips
// gracefully when absent, mirroring the zvm `story_location_verify` tests.
//
// Pinned to release 11 on purpose (SQ-1454's disposition table): booted cold
// against the IF Archive's release 10, this style hint reads back as the
// theme's Default rather than the game's white-on-black — release 10 does not
// set it the same way at boot, so `stories/`-only is correct here, not a gap.
use app::engine::Engine;
use app::glulx_session::GlulxSession;
use blorb::Blorb;

#[test]
#[ignore = "slow (~2.7s): runs Counterfeit Monkey's intro through the Glulx VM; run with --include-ignored"]
fn cm_intro_pane_adopts_the_games_black_on_white() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../stories/CounterfeitMonkey-11.gblorb");
    if !path.exists() {
        eprintln!("SKIP: stories/CounterfeitMonkey-11.gblorb absent");
        return;
    }
    let blorb = Blorb::parse(std::fs::read(&path).unwrap()).expect("parse gblorb");
    let image = blorb.executable().expect("exec chunk").1.to_vec();
    let sess = GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("boot CM");

    let model = sess.screen();
    let bg = app::state::unpack_zcolour(model.bg);
    let fg = app::state::unpack_zcolour(model.fg);
    assert!(
        matches!(bg, zvm::screen::ZColour::True24(0x00FF_FFFF)),
        "story pane must adopt the game's white Normal background, got {bg:?}"
    );
    assert!(
        matches!(fg, zvm::screen::ZColour::True24(0)),
        "story pane must adopt the game's black Normal foreground, got {fg:?}"
    );
}
