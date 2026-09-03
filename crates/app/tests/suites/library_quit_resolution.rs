//! SQ-1258: a story launched from the story list returns to the list on EVERY
//! way its run can end — the game's own quit, the player's `quit` command, and
//! Ctrl-Q alike — instead of only `/quit-to-library` doing so. A launch off the
//! command line still leaves lanthorn entirely either way, since there is no
//! list to return to.
//!
//! The whole rule lives in one place: `app::state::ExitTarget::for_launch`,
//! seeded onto `AppState.exit_target` once at boot (`run_event_loop`,
//! `crates/app/src/main.rs`) from `launched_from_library`, and left untouched
//! by a clean game quit — `should_exit_on_turn` (`crates/app/src/turn.rs`)
//! decides only WHETHER a turn ends the run, never WHERE it resolves to. The
//! player's own `quit` (`SlashOutcome::Quit` in `slash_dispatch.rs`) and Ctrl-Q
//! (`Action::Quit` in `main.rs`) both re-resolve through the same
//! `ExitTarget::for_launch` call rather than hardcoding `Exit`.
//!
//! That split is exactly why this real-game case only needs to pin the VM half
//! — that a game's own `quit` actually reaches `has_quit()`, which is what
//! `should_exit_on_turn` reads — while the launch-context half is checked
//! directly against `ExitTarget::for_launch` right here, and independently in
//! `crates/app/src/state.rs` (`exit_target_for_launch_follows_whether_a_library_exists`),
//! `crates/app/src/main.rs` (`library_launch_always_resolves_to_the_library` /
//! `command_line_launch_always_resolves_to_exit`), and `slash_dispatch.rs`'s
//! `quit_from_a_library_launch_resolves_to_library` /
//! `quit_from_a_command_line_launch_resolves_to_exit`. None of those need a
//! real VM to falsify; this one confirms the VM signal they all assume is real.

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;
use app::state::ExitTarget;

use crate::fixture_paths::fixture_path;

/// Mini-Zork I r34/s871124 — routed by `fixture_path` to the tracked,
/// sha256-verified copy at `crates/zvm/tests/fixtures/minizork.z3` (IF Archive
/// `demos/minizork.z3`), so this case never skips vacuously in CI or a fresh
/// worktree with no `stories/`.
#[test]
fn minizork_quit_reaches_has_quit_and_the_launch_context_resolves_where_to_go() {
    let path = fixture_path("minizork-r34-s871124.z3");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("tracked fixture must be present at {}: {e}", path.display()));

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
            .expect("minizork should load and boot without a ZError");
    assert!(!session.has_quit(), "minizork must not quit during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Mini-Zork's `quit` verb asks "Do you wish to leave the game? (Y is
    // affirmative): " before it actually quits — drive both turns, exactly the
    // sequence a player who confirms produces.
    let mut result = session.submit("quit");
    if !result.quit {
        result = session.submit("y");
    }
    assert!(result.quit, "quit (+ a y confirmation) must end the game: {:?}", result.transcript);
    assert!(
        session.has_quit(),
        "the session must record the quit — should_exit_on_turn (turn.rs) reads exactly this"
    );

    // The turn that just ended never touched Exit-vs-Library — only WHETHER to
    // end the run. Where it resolves to is entirely the launch context,
    // seeded once at boot and otherwise left alone:
    assert_eq!(
        ExitTarget::for_launch(true),
        ExitTarget::Library,
        "launched from the picker + the game quit -> back to the list"
    );
    assert_eq!(
        ExitTarget::for_launch(false),
        ExitTarget::Exit,
        "launched from the command line + the game quit -> leave lanthorn (no list to return to)"
    );
}
