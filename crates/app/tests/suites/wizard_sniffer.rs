use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

#[test]
fn wizard_sniffer_reaches_input() {
    let path = fixture_path("The_Wizard_Sniffer.gblorb.blorb");
    let Ok(raw) = std::fs::read(&path) else { eprintln!("SKIP: no WS"); return; };
    let bytes = match app::hints::extract_story(raw).expect("extract") {
        app::hints::LoadedStory::Glulx(b) => b,
        app::hints::LoadedStory::ZCode(_) => panic!("expected Glulx"),
        app::hints::LoadedStory::Scott(_) => panic!("expected Glulx"),
    };
    let mut sess = GlulxSession::new(bytes, 80, 24, true, true, false, (8.0, 16.0), None, &[]).expect("ws session");
    let _ = sess.take_transcript();
    // Two "press any key" char inputs then a directional command; the 2nd char
    // used to spin in a status-window resize loop and abort with quit=true.
    for (i, cmd) in ["", "", "look"].iter().enumerate() {
        let pending = sess.pending_input();
        let result = match pending {
            InputKind::Char => sess.submit_key(KeyInput::Enter).expect("char"),
            _ => sess.submit(cmd),
        };
        eprintln!("step {i} cmd={cmd:?} pending_before={pending:?} -> {} chars, quit={}, diag={}",
            result.transcript.chars().count(), result.quit, result.diagnostics.len());
        for d in &result.diagnostics { eprintln!("   DIAG: {d}"); }
        assert!(!result.quit, "WS must not quit (runaway) at step {i}");
    }
    eprintln!("pending after intro+look: {:?}", sess.pending_input());
}
