//! SQ-0656: what the host may and may not do to a Glulx session while the GAME's
//! own `@save`/`@restore` is suspended on one of lanthorn's dialogs.
//!
//! Three things converge on that window: the player can answer the game's RESTORE
//! dialog with a host **Save State** instead (a full machine swap under a
//! suspended call), the terminal can be **resized**, and a **sound can finish**.
//! Every one of them used to drive the VM, and every non-interactive drive
//! auto-FAILS a suspended save/restore — so the player's in-flight save was
//! recorded as failed while its name prompt was still open, and the abandoned
//! restore kept re-reporting itself so the dialog reopened out of nowhere.
//!
//! Driven against a real Inform-compiled Glulx story (Adventure), because this is
//! the game's own SAVE/RESTORE verb plumbing end to end. The tests skip vacuously
//! when the gitignored fixture is absent.


use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::session::PendingIo;

use crate::fixture_paths::fixture_path;

/// The Glulx executable inside `advent.blb` (gitignored; the test skips loudly
/// when it is absent).
fn advent_image() -> Option<Vec<u8>> {
    let p = fixture_path("advent.blb");
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", p.display());
            return None;
        }
    };
    let blorb = blorb::Blorb::parse(bytes).expect("advent.blb parses as a Blorb");
    let (_kind, exec) = blorb.executable().expect("advent.blb carries an executable chunk");
    Some(exec.to_vec())
}

fn booted() -> Option<GlulxSession> {
    let image = advent_image()?;
    let mut sess = GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[])
        .expect("Adventure (Glulx) boots");
    let _ = sess.take_transcript(); // drain the banner
    Some(sess)
}

#[test]
fn a_host_save_state_answers_the_games_restore_without_leaving_it_pending() {
    let Some(mut sess) = booted() else { return };

    // ── A reference run, so the restored session can be held to it ────────────
    sess.submit("in");
    let save = sess.save_state();
    let ref_look = sess.submit("look").transcript;
    let ref_inv = sess.submit("inventory").transcript;
    assert!(ref_look.contains("Inside Building"), "reference look: {ref_look:?}");

    // Play PAST the save point so a restore is observable.
    sess.submit("get lamp");
    assert!(
        sess.submit("inventory").transcript.contains("lantern"),
        "the lamp is carried after the save point",
    );

    // ── The game's own RESTORE verb suspends, waiting on the host's dialog ────
    let r = sess.submit("restore");
    assert_eq!(r.pending_io, Some(PendingIo::Restore), "the RESTORE verb bubbles to the host");
    assert!(sess.is_saveload_pending(), "the VM is suspended inside @restore");

    // The player answers that dialog with a host Save State (main.rs's
    // `RestoreOutcome::Resumed`, explicitly reachable while an @restore pends).
    sess.restore_state(&save).expect("the host snapshot restores");
    assert!(
        !sess.is_saveload_pending(),
        "the abandoned @restore must not survive the machine swap — completing it later would \
         store through a dest record describing a stack that no longer exists",
    );

    // ── PERTURB, then assert: the bug shows on the NEXT turn, not here ────────
    let look = sess.submit("look");
    assert_eq!(
        look.pending_io, None,
        "the restore dialog must not reopen from nowhere on the turn after a host restore",
    );
    assert_eq!(
        look.transcript, ref_look,
        "the restored session must replay the reference run exactly — a resumed PC with no live \
         glk_select re-reads the snapshot's event_t and replays its last command as a free turn",
    );
    let inv = sess.submit("inventory");
    assert_eq!(inv.pending_io, None, "and still no dialog a turn later");
    assert_eq!(
        inv.transcript, ref_inv,
        "the lamp taken after the save point is gone: the snapshot really was installed",
    );
}

#[test]
fn a_terminal_resize_while_a_game_save_is_suspended_defers_instead_of_failing_it() {
    let Some(mut sess) = booted() else { return };
    sess.submit("in");

    let r = sess.submit("save");
    assert_eq!(r.pending_io, Some(PendingIo::Save), "the SAVE verb bubbles to the host");
    assert!(sess.is_saveload_pending(), "the VM is suspended inside @save");

    // The player is still typing a name into the save dialog when the terminal
    // resizes. The resize poller records the size as delivered and never retries
    // it, so the session has to carry it rather than drop it.
    sess.resize(60, 20);
    assert!(
        sess.is_saveload_pending(),
        "a resize must not answer the player's in-flight @save — driving the VM auto-fails it, \
         and the dialog's later confirm would then write the archive over a game that already \
         recorded a failure",
    );

    // The dialog confirms: the game hears SUCCESS, and the queued size lands.
    let resumed = sess.resume_save(true);
    assert_eq!(resumed.pending_io, None, "@save completes and the game runs on");
    assert!(
        resumed.transcript.contains("Ok."),
        "the game must see the save it actually got: {:?}",
        resumed.transcript,
    );
    assert_eq!(
        sess.screen().content_size.0,
        60,
        "the resize deferred over the dialog is applied on resume, not lost",
    );

    // And the session is playable at the new size.
    assert!(sess.submit("look").transcript.contains("Inside Building"));
}

#[test]
fn a_sound_finishing_while_a_game_save_is_suspended_does_not_fail_it() {
    let Some(mut sess) = booted() else { return };
    sess.submit("in");
    assert_eq!(sess.submit("save").pending_io, Some(PendingIo::Save));

    // A sound channel finishing fires an Evtype_SoundNotify from the host's audio
    // poller — on its own schedule, with no regard for what dialog is open.
    let _ = sess.sound_notify(3, 7);
    assert!(
        sess.is_saveload_pending(),
        "a sound-notify must not answer the player's in-flight @save",
    );
    // Same for a Glk timer tick and a volume-ramp completion, which share the gate.
    let _ = sess.deliver_timer();
    let _ = sess.volume_notify(1);
    assert!(sess.is_saveload_pending(), "nor a timer tick or a volume notify");

    let resumed = sess.resume_save(true);
    assert!(
        resumed.transcript.contains("Ok."),
        "the game sees the success the player confirmed: {:?}",
        resumed.transcript,
    );
}
