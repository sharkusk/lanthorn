//! A dropped browser connection must not lose the game (SQ-1323).
//!
//! WHAT WENT WRONG IN THE FIELD. lanthorn's Docker image serves the TUI through
//! ttyd, one `lanthorn` process per websocket. A player on an iPad — where a
//! backgrounded tab, a sleeping screen or a roaming Wi-Fi hop drops the socket
//! routinely — reported that "disconnects kill the game and erase progress".
//!
//! WHAT ACTUALLY HAPPENS ON A DROP. ttyd 1.7.7's `LWS_CALLBACK_CLOSED`
//! (`src/protocol.c:373-379`) calls `pty_kill(pss->process, server->sig_code)`;
//! `pty_kill` is `uv_kill(-process->pid, sig)` (`src/pty.c:158-164`) — a signal
//! at the PROCESS GROUP — and `sig_code` defaults to `SIGHUP`
//! (`src/server.c:169`). So the app is asked politely to end, exactly as
//! `install_termination_handlers` (`main.rs`) is built for: the flag is set, the
//! game loop sees it at its next safe point, `exit_if_terminated_saving` restores
//! the terminal, runs `lifecycle::exit_auto_save`, and exits `128 + SIGHUP`.
//!
//! THE MACHINERY WAS NEVER THE PROBLEM. `exit_auto_save` opens with
//! `if !state.config.auto_save { return; }` — and `auto_save` **defaults to
//! false**. The signal arrived, the terminal was restored, the exit code was
//! right, and nothing was written, because nothing was ever meant to be. The
//! resume half was already correct in the other direction: `auto_load` defaults
//! to TRUE, so the next connection silently restores whatever archive it finds.
//! One missing write was the whole defect.
//!
//! WHAT THESE CASES PIN.
//!
//! 1. `a_ttyd_style_hangup_saves_the_turns_that_were_played` — the fix works:
//!    with `--auto-save on`, three turns played and then hung up the way ttyd
//!    hangs up leave an archive that says three turns.
//! 2. `without_auto_save_a_hangup_leaves_nothing_behind` — the FALSIFICATION.
//!    The same run without the flag exits just as cleanly, code and all, and
//!    writes no archive at all. This is the reported symptom, reproduced; if it
//!    ever starts passing an archive back, case 1 has stopped proving anything.
//! 3. `a_reconnect_resumes_the_turns_the_last_connection_saved` — the other half
//!    of "not lost": a fresh process on the same data directory picks the count
//!    up where it was, rather than starting over.
//! 4. `an_uncatchable_kill_loses_at_most_the_turn_in_progress` — the per-turn
//!    cadence, which is what covers the case the signal path cannot: SIGKILL.
//!
//! THE FIXTURE IS THE TRACKED ONE. `stories/` is gitignored, so a case reaching
//! for it would skip vacuously in a worktree and in CI. Mini-Zork ships in
//! `crates/zvm/tests/fixtures/`, so this always really runs.

#[cfg(not(unix))]
#[test]
fn the_hangup_test_is_unix_only() {
    eprintln!("SKIP: hanging a pty up needs a pty, which this platform does not have");
}

// The harness itself is declared once by the group binary (`tests/pty.rs`) and
// shared; only the driving half is wanted here.
#[cfg(unix)]
use super::pty_stream::driver;

#[cfg(unix)]
mod unix {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::driver::{self, Key, Spec};

    /// The conventional exit code for a process that ended on SIGHUP — which is
    /// what lanthorn's own signal path reports (`term_exit_code`, SQ-0502). It
    /// arrives as a NORMAL exit with this code, never as a signal death: the
    /// difference is the whole point, because a signal death means the default
    /// disposition ran and no save was ever attempted.
    const EXIT_SIGHUP: i32 = 128 + 1;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pty-capture").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn story() -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../zvm/tests/fixtures/minizork.z3");
        assert!(p.is_file(), "tracked fixture missing at {}", p.display());
        p
    }

    /// Where this launch's auto-resume archive lands — the same three calls
    /// `startup.rs` makes to bind `arc_file`.
    fn archive_path(user: &Path) -> PathBuf {
        let s = story();
        let game_dir = app::storage::game_dir(&user.join("saves"), &app::storage::story_key_at(&s));
        app::storage::default_state_path(&game_dir)
    }

    /// A launch on Mini-Zork that plays `turns` commands and then ends the way
    /// `end` says. `auto_save` is the flag under test; `None` runs the stock
    /// default, which is off.
    fn play(user: &Path, auto_save: Option<&str>, turns: usize, hangup: bool) -> driver::Capture {
        let mut spec = Spec::new(env!("CARGO_BIN_EXE_lanthorn"), story(), user);
        spec.cols = 100;
        spec.rows = 40;
        spec.tail = Duration::from_millis(1500);
        spec.hangup = hangup;
        if let Some(v) = auto_save {
            spec.extra_args = vec!["--auto-save".into(), v.into()];
        }
        let mut keys = vec![Key::Wait(Duration::from_millis(1500))];
        for _ in 0..turns {
            // `look` is a turn whatever the parser makes of the room, and every
            // submitted line counts one (`state.turns += 1` at the submit sites in
            // `main.rs`) — so the count under test does not depend on the game
            // understanding anything in particular.
            keys.push(Key::Bytes(b"look\r".to_vec()));
            keys.push(Key::Wait(Duration::from_millis(600)));
        }
        spec.keys = keys;
        driver::run(spec).expect("pty run")
    }

    /// The run really booted the story and really typed at it — without this a
    /// case that never got past the boot would "prove" whatever it liked about
    /// an archive that was never going to be written.
    fn assert_the_scenario_ran(cap: &driver::Capture) {
        let text = String::from_utf8_lossy(&cap.bytes).to_string();
        assert!(
            text.contains("Mini-Zork"),
            "the story never booted, so nothing here was measured; got {} bytes",
            cap.bytes.len()
        );
    }

    fn turns_in(path: &Path) -> u32 {
        app::archive::read_archive_meta(path)
            .unwrap_or_else(|e| panic!("archive at {} unreadable: {e}", path.display()))
            .turns
    }

    #[test]
    fn a_ttyd_style_hangup_saves_the_turns_that_were_played() {
        let user = scratch("hangup-autosave-on");
        let cap = play(&user, Some("on"), 3, true);
        assert_the_scenario_ran(&cap);

        let status = cap.exit.expect("a hangup run reports how the child ended");
        assert_eq!(
            status.code(),
            Some(EXIT_SIGHUP),
            "the app must end through its OWN signal path (a normal exit of {EXIT_SIGHUP}), \
             not on SIGHUP's default disposition — a signal death means the save never ran: {status:?}"
        );

        let arc = archive_path(&user);
        assert!(
            arc.is_file(),
            "a dropped connection left no resume state at {} — this is the reported defect",
            arc.display()
        );
        assert_eq!(turns_in(&arc), 3, "the archive must carry the turns that were actually played");
    }

    #[test]
    fn without_auto_save_a_hangup_leaves_nothing_behind() {
        let user = scratch("hangup-autosave-off");
        let cap = play(&user, None, 3, true);
        assert_the_scenario_ran(&cap);

        let status = cap.exit.expect("a hangup run reports how the child ended");
        assert_eq!(
            status.code(),
            Some(EXIT_SIGHUP),
            "the signal path itself was never broken — it runs either way: {status:?}"
        );

        let arc = archive_path(&user);
        assert!(
            !arc.exists(),
            "the stock default writes NOTHING on a hangup, which is the whole of SQ-1323; \
             an archive appearing at {} means the falsification has stopped falsifying and \
             the sibling case above no longer proves the flag did anything",
            arc.display()
        );
    }

    #[test]
    fn a_reconnect_resumes_the_turns_the_last_connection_saved() {
        let user = scratch("hangup-reconnect");

        let first = play(&user, Some("on"), 3, true);
        assert_the_scenario_ran(&first);
        let arc = archive_path(&user);
        assert_eq!(turns_in(&arc), 3, "the first connection's three turns");

        // What the NEXT websocket runs: a brand-new process on the same data
        // directory. `auto_load` defaults to true, so this must pick the count up
        // rather than start over.
        let second = play(&user, Some("on"), 2, true);
        assert_the_scenario_ran(&second);
        assert_eq!(
            turns_in(&arc),
            5,
            "the reconnect resumed at 3 and played 2 more; a 2 here means it silently \
             started a fresh game and the player's progress is gone even though a save exists"
        );
    }

    #[test]
    fn an_uncatchable_kill_loses_at_most_the_turn_in_progress() {
        // SIGKILL is what a container OOM, a `docker stop` timeout or a crashed
        // host does, and no handler can answer it. The per-turn cadence is the
        // only thing standing between that and a lost session — the same
        // `auto_save` key, writing after every turn through the coalescing
        // background worker.
        let user = scratch("hangup-sigkill");
        let cap = play(&user, Some("on"), 3, false);
        assert_the_scenario_ran(&cap);
        assert!(cap.exit.is_none(), "this run is closed with SIGKILL, so there is no orderly status");

        let arc = archive_path(&user);
        assert!(
            arc.is_file(),
            "nothing survived an uncatchable kill at {} — the per-turn cadence is not running",
            arc.display()
        );
        // Not `== 3`: the last turn's write is enqueued on a background worker and
        // a SIGKILL can land between the enqueue and the rename. "At most one turn"
        // is exactly the guarantee being claimed, so it is exactly what is asserted.
        let turns = turns_in(&arc);
        assert!(
            turns >= 2,
            "three turns were played and the archive says {turns}: more than the turn in \
             progress was lost, so the writes are not landing per turn"
        );
    }
}
