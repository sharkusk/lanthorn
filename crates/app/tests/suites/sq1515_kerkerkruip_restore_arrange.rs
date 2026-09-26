//! SQ-1515: a host Save State restore into a FRESH Glulx session must repaint
//! a Glk-windowed game's side panels, not leave them blank.
//!
//! Glulx spec §1.8.5: a save carries window STRUCTURE (the tree, types,
//! pending line/char requests) but never window CONTENTS — the game is
//! expected to repaint on its own next Arrange/Redraw. That is fine
//! restoring into windows the game already had live; it is not fine when a
//! host Save State swaps the whole window MODEL under the game (a restore
//! into a session that never opened those windows itself), because nobody
//! ever delivers the Arrange a real restore-into-live-windows would never
//! have needed. Kerkerkruip's window ids drift upward over a real play
//! session (every panel close/reopen — e.g. its help screen — hands out
//! fresh, higher ids; `all_window_ids` is monotonic, gvm's `alloc_window`
//! never reuses a freed slot), so a Save State taken deep into a session and
//! restored into a freshly booted one names windows the fresh session's own
//! backend has never heard of. The user's real archive showed ids 81..115
//! restoring into a fresh session's 4..45.
//!
//! The fix is two halves: `Machine::restore_state` (gvm) now tells the
//! backend the old windows closed and the new ones opened (so a window-id-
//! keyed backend — AppGlk's grid/buffer/graphics maps — cannot answer for
//! the wrong run's content) and relayouts; `GlulxSession::restore_state`
//! (app) delivers a Glk Arrange the way a resize would, so Kerkerkruip's own
//! Arrange handler repaints its panels from the restored game state.

use std::path::{Path, PathBuf};
use std::time::Duration;

use app::engine::{Engine, KeyInput, WinNode};
use app::glk_backend::GlkStylePairs;
use app::glulx_session::GlulxSession;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

const STORY: &str = "Kerkerkruip.gblorb";
const COLS: u16 = 225;
const ROWS: u16 = 51;
const CELL_PX: (u32, u32) = (13, 29);
const POLL_CAP: usize = 8000;
const TURN_BUDGET: Duration = Duration::from_secs(600);

fn story_path() -> Option<PathBuf> {
    let p = fixture_path(STORY);
    if !p.is_file() {
        eprintln!("SKIP: fixture missing at {}", p.display());
        return None;
    }
    if !p.with_file_name("Kerkerkruip.ini").is_file() {
        eprintln!("SKIP: needs the shipped Kerkerkruip.ini beside the story");
        return None;
    }
    Some(p)
}

fn theme_pairs_for(path: &Path) -> GlkStylePairs {
    let mut cs = app::colors::ColorScheme::default();
    if let Some(ov) = app::garglk_ini::discover(path) {
        ov.apply(&mut cs);
    }
    app::glk_backend::theme_style_colours(&cs)
}

fn boot(path: &Path) -> GlulxSession {
    let bytes = std::fs::read(path).expect("read the story");
    let blorb = blorb::Blorb::parse(bytes).expect("Kerkerkruip is a Blorb");
    let image = blorb.executable().expect("Glulx exec chunk").1.to_vec();
    let mut sess = GlulxSession::new_in(
        PathBuf::new(),
        image,
        COLS as u32,
        ROWS as u32,
        true,
        true,
        false,
        false,
        (CELL_PX.0 as f64, CELL_PX.1 as f64),
        Some(blorb),
        &[],
        theme_pairs_for(path),
        false,
        None,
    )
    .expect("Kerkerkruip boots");
    sess.set_turn_budget(TURN_BUDGET);
    sess
}

fn settle(sess: &mut GlulxSession, done: impl Fn(&GlulxSession) -> bool) -> bool {
    for _ in 0..POLL_CAP {
        if done(sess) {
            return true;
        }
        let can_advance =
            sess.timer_interval().is_some() || Engine::pending_input(sess) == InputKind::Event;
        if !can_advance {
            return done(sess);
        }
        let _ = sess.deliver_timer();
    }
    done(sess)
}

fn into_gameplay() -> Option<GlulxSession> {
    let mut sess = boot(&story_path()?);
    settle(&mut sess, |s| Engine::pending_input(s) != InputKind::Event);
    let _ = Engine::submit_key(&mut sess, KeyInput::Char(' '));
    settle(&mut sess, |s| Engine::pending_input(s) == InputKind::Line);
    assert_eq!(Engine::pending_input(&sess), InputKind::Line, "reached the parser prompt");
    Some(sess)
}

/// Every non-primary Buffer/Grid/Graphics window in the tree, found by kind
/// rather than a hard-coded id (ids drift, see the module doc). The primary
/// buffer is excluded: the app renders it from `state.transcript`, not from
/// `WinNode::Buffer::lines` (see `render_state`/`render` elsewhere in this
/// suite family), so it is never populated through this path either way.
fn panel_windows(sess: &GlulxSession) -> Vec<WinNode> {
    fn walk(node: &WinNode, out: &mut Vec<WinNode>) {
        match node {
            WinNode::Buffer(b) if !b.primary => out.push(node.clone()),
            WinNode::Grid(_) | WinNode::Graphics(_) => out.push(node.clone()),
            WinNode::Pair { first, second, .. } => {
                walk(first, out);
                walk(second, out);
            }
            _ => {}
        }
    }
    let mut v = Vec::new();
    walk(&Engine::screen(sess).root, &mut v);
    v
}

/// Assert every panel window found by [`panel_windows`] actually carries
/// content: a buffer has printed lines, a grid has non-blank cells (unless
/// `require_grid` is false), a graphics window has opaque pixels. Panics
/// with the offending window's id and kind so a failure names exactly what
/// stayed blank.
///
/// `require_grid`: Kerkerkruip's status grid is only painted on a full game
/// turn, not merely on opening — a session that has never taken a turn
/// legitimately has a blank grid with no restore involved, so the
/// pre-restore sanity check on the archive's SOURCE session (which has only
/// been through the menu, never a turn) skips it; the post-restore check
/// does not, since by then a turn (or an Arrange-triggered repaint) has run.
fn assert_panels_populated(label: &str, sess: &GlulxSession, require_grid: bool) {
    let panels = panel_windows(sess);
    assert!(!panels.is_empty(), "{label}: expected panel windows in the tree, found none");
    for w in &panels {
        match w {
            WinNode::Buffer(b) => assert!(
                !b.lines.is_empty(),
                "{label}: buffer window {} has no lines (panel stayed blank)",
                b.win
            ),
            WinNode::Grid(g) => {
                let nonblank = g.cells.iter().filter(|c| c.ch != ' ' && c.ch != '\0').count();
                if require_grid {
                    assert!(nonblank > 0, "{label}: grid window {} has no non-blank cells", g.win);
                }
            }
            WinNode::Graphics(gw) => {
                let opaque = gw.canvas.pixels().filter(|p| p[3] >= 128).count();
                assert!(opaque > 0, "{label}: graphics window {} canvas has no opaque pixels", gw.win);
            }
            _ => unreachable!("panel_windows only collects Buffer/Grid/Graphics"),
        }
    }
}

/// Drive a session into gameplay, then open and close Kerkerkruip's HELP
/// screen once: it closes every side panel down to a single help pane, and
/// on dismissal reopens them all under FRESH, higher window ids — the same
/// drift a long real play session produces, condensed into one predictable,
/// condition-driven step (no fixed tick counts, no hard-coded ids). Returns
/// a save of that state, taken once the reopened panels are confirmed alive
/// in THIS session — so the archive is known-good before it is trusted to
/// exercise a restore.
fn id_shifted_archive() -> app::engine::EngineSave {
    let mut sess = into_gameplay().unwrap();
    let _ = Engine::submit(&mut sess, "help");
    settle(&mut sess, |s| {
        Engine::pending_input(s) == InputKind::Char || Engine::pending_input(s) == InputKind::Line
    });
    // Escape backs out of the help topic menu (Kerkerkruip's "Go back") and
    // reopens the side panels under new, higher ids; the game then parks on
    // a further char-wait, which is fine — the archive just needs to
    // capture live, populated, higher-numbered windows, not reach a fresh
    // Line prompt.
    let _ = Engine::submit_key(&mut sess, KeyInput::Escape);
    settle(&mut sess, |s| {
        Engine::pending_input(s) == InputKind::Char || Engine::pending_input(s) == InputKind::Line
    });
    assert_panels_populated("id_shifted_archive source session", &sess, false);
    Engine::save_state(&sess)
}

#[test]
fn restore_into_fresh_session_repaints_orphaned_panels() {
    let Some(_p) = story_path() else { return };
    let archive = id_shifted_archive();

    let mut fresh = into_gameplay().unwrap();
    Engine::restore_state(&mut fresh, &archive).expect("restore the id-shifted archive");

    // Deliberate departure from the general "perturb before asserting"
    // convention: that rule exists because MOST restore bugs surface only on
    // the next repaint/palette-change/resize, so a same-frame check would be
    // fooled by a coincidentally-correct first frame. SQ-1515 is the inverse
    // — the reported symptom (blank panels) is the FRAME IMMEDIATELY AFTER
    // restore, no further action needed to see it, and the fix
    // (`GlulxSession::restore_state`) delivers its Arrange SYNCHRONOUSLY
    // before returning. Checking right here is checking the exact mechanism
    // under test.
    assert!(!Engine::has_quit(&fresh), "restored session must still be playable");
    assert_panels_populated("immediately after restore into a fresh session", &fresh, true);

    // Perturb with a TURN, not a resize: `GlulxSession::resize` calls
    // `Machine::rearrange()` on its own (pre-existing, unrelated to this fix),
    // so a resize-based perturb would deliver the very same Arrange the fix
    // adds and could never falsify anything here — confirmed while writing
    // this suite (a resize perturb passed even with the restore fix
    // reverted). A plain `look` delivers no Arrange at all: the diagnostic
    // that opened SQ-1515 showed `look` alone does NOT repopulate panels on
    // the unfixed code, so this perturb stays genuinely falsifiable — it
    // re-checks that the Arrange the restore already delivered held, without
    // itself being able to deliver a second one.
    let tr = Engine::submit(&mut fresh, "look");
    assert!(!tr.transcript.is_empty(), "a `look` after restore must produce room text");
    settle(&mut fresh, |s| Engine::pending_input(s) != InputKind::Event);
    assert!(!Engine::has_quit(&fresh), "session must survive a post-restore turn");
    assert_panels_populated("after restore + a `look` turn", &fresh, true);
}

/// The inverse of [`assert_panels_populated`]: every panel window found by
/// [`panel_windows`] carries NO content — the shape an un-arranged restore
/// leaves behind (SQ-1515's original bug report, and now the deliberate
/// behaviour of a `headless` restore — see `GlulxSession::restore_state`).
fn assert_panels_blank(label: &str, sess: &GlulxSession) {
    let panels = panel_windows(sess);
    assert!(!panels.is_empty(), "{label}: expected panel windows in the tree, found none");
    for w in &panels {
        match w {
            WinNode::Buffer(b) => assert!(
                b.lines.iter().all(|l| l.is_empty()),
                "{label}: buffer window {} has content (expected blank): {:?}",
                b.win,
                b.lines
            ),
            WinNode::Grid(g) => {
                let nonblank = g.cells.iter().filter(|c| c.ch != ' ' && c.ch != '\0').count();
                assert_eq!(nonblank, 0, "{label}: grid window {} has non-blank cells (expected blank)", g.win);
            }
            WinNode::Graphics(gw) => {
                let opaque = gw.canvas.pixels().filter(|p| p[3] >= 128).count();
                assert_eq!(opaque, 0, "{label}: graphics window {} has opaque pixels (expected blank)", gw.win);
            }
            _ => unreachable!("panel_windows only collects Buffer/Grid/Graphics"),
        }
    }
}

/// Build the same shape of session `probe::serve` restores its shadow into:
/// `GlulxSession::new_shadow` (read-only store, graphics/sound off, no
/// picture Blorb) — settled to its own first select, the way `boot_shadow`
/// leaves it before the first restore.
fn boot_shadow() -> Option<GlulxSession> {
    let path = story_path()?;
    let bytes = std::fs::read(&path).expect("read the story");
    let blorb = blorb::Blorb::parse(bytes).expect("Kerkerkruip is a Blorb");
    let image = blorb.executable().expect("Glulx exec chunk").1.to_vec();
    let mut shadow =
        GlulxSession::new_shadow(PathBuf::new(), image, COLS as u32, ROWS as u32, true, &[], None)
            .expect("shadow boots");
    settle(&mut shadow, |s| Engine::pending_input(s) != InputKind::Event);
    Some(shadow)
}

/// SQ-1515 follow-up: the `headless` guard on `GlulxSession::restore_state`
/// skips the Arrange delivery for the `probe` shadow (measured cost: a
/// shadow restore of this same archive went from 22ms to 737ms once the fix
/// started delivering it — see the comment in `restore_state`). A headless
/// restore of the id-shifted archive must therefore leave every panel
/// exactly as blank as an un-arranged restore does (the shape this suite's
/// other test asserts is now WRONG for a rendered session); the identical
/// archive restored into a normal, rendered session repaints, confirming the
/// contrast is the `headless` flag and nothing else about the archive.
#[test]
fn headless_restore_skips_the_arrange_a_rendered_restore_still_gets() {
    let Some(_p) = story_path() else { return };
    let archive = id_shifted_archive();

    let Some(mut shadow) = boot_shadow() else { return };
    Engine::restore_state(&mut shadow, &archive).expect("headless restore");
    assert!(!Engine::has_quit(&shadow), "a headless restore must not itself end the game");
    assert_panels_blank("headless restore (probe shadow shape)", &shadow);

    let mut normal = into_gameplay().unwrap();
    Engine::restore_state(&mut normal, &archive).expect("normal (rendered) restore");
    assert_panels_populated("normal (rendered) restore of the SAME archive", &normal, true);
}
