//! A terminal that answers our startup questions LATE must not have its answers
//! read as keystrokes by the story (SQ-0769).
//!
//! WHAT THIS PINS. lanthorn asks the terminal for its default fg/bg (OSC 10 /
//! OSC 11) just before a story boots, and ends the batch with a DSR so it knows
//! when the answers are in. In the field the answers sometimes arrived after the
//! probe had stopped listening — a picker launch leaves the terminal busy with a
//! screenful of kitty graphics — and the player got
//!
//! ```text
//! 0;rgb:ffff/ffff/ffff11;rgb:2828/2c2c/3434
//! ```
//!
//! typed into the game: the intro skipped, the restore prompt fired, the terminal
//! beeped. It reproduced perhaps one launch in several, which is why it was once
//! closed as "no longer happening" and had to be reopened.
//!
//! WHY IT IS DETERMINISTIC HERE. The pty harness answers those queries itself
//! (SQ-0762), so it can answer them deliberately late — [`Spec::defer_queries`].
//! The race stops being a race: the answer is guaranteed to arrive after the
//! probe's own patience has run out, every run. A test that only passed because
//! the race was won would be worth nothing here.
//!
//! THE FIXTURE IS DELIBERATELY A FETCHED ONE, NOT `stories/`. `stories/` is
//! gitignored commercial media, so a test that reached for it would skip
//! vacuously in a worktree and in CI — and a silent skip is exactly how this
//! defect survived once already. Mini-Zork is fetched instead
//! (`scripts/fixtures.manifest`, SQ-1453), which CI always populates before
//! `cargo test` runs, so this still always really runs there; a local run
//! wants `scripts/fetch-fixtures.sh` first if `minizork-r34-s871124.z3` is
//! not already present.

#[cfg(not(unix))]
#[test]
fn the_late_reply_test_is_unix_only() {
    eprintln!("SKIP: driving a real terminal needs a pty, which this platform does not have");
}

// Only the driving half of the SQ-0762 harness is wanted here; the decoder is
// another suite's business, and the parts of the driver this file does not call
// are not dead code, just unused by this caller. The harness itself is declared
// once by the group binary (`tests/pty.rs`) and shared.
#[cfg(unix)]
use super::pty_stream::driver;

#[cfg(unix)]
mod unix {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::driver::{self, Key, Spec};

    /// Every byte of an OSC colour answer that must never reach the story. `rgb:`
    /// is the shape of the answer; `c0c0` and `0000` are this harness's own
    /// values, so a hit is provably ours and not the story's prose.
    const LEAKED: [&str; 2] = ["rgb:", "c0c0/c0c0/c0c0"];

    /// Answer the colour queries this long after they are asked — comfortably
    /// past the probe's own silence window, so the answer always lands after it
    /// has stopped listening.
    const LATE: Duration = Duration::from_millis(400);

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pty-capture").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// A one-story library plus a user dir that already knows it, so the launch
    /// goes straight to the picker with no first-use prompt in the way. `None`
    /// if the fetched fixture is absent (caller skips).
    fn library(root: &Path) -> Option<(PathBuf, PathBuf)> {
        let lib = root.join("library");
        let user = root.join("user");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        let story = crate::fixture_paths::fixture_path("minizork-r34-s871124.z3");
        if !story.is_file() {
            return None;
        }
        std::fs::copy(&story, lib.join("minizork.z3")).unwrap();
        std::fs::write(user.join("config.toml"), format!("default_story_dir = '{}'\n", lib.display())).unwrap();
        Some((lib, user))
    }

    fn spec(lib: &Path, user: &Path) -> Spec {
        let mut spec = Spec::new(env!("CARGO_BIN_EXE_lanthorn"), lib, user);
        spec.cols = 100;
        spec.rows = 40;
        // The per-game sidecar `hide_map` writes is keyed off the story path, and
        // here that path is a directory — leave the map alone rather than seed a
        // sidecar for something that is not a story.
        spec.hide_map = false;
        spec.tail = Duration::from_millis(1500);
        spec.defer_queries = vec!["default foreground (OSC 10)", "default background (OSC 11)"];
        spec.defer_by = LATE;
        spec
    }

    /// The run really did what the test claims: the story booted from the picker,
    /// and the colour answers really were held back until after the boot.
    fn assert_the_scenario_ran(cap: &driver::Capture, text: &str) {
        assert!(
            text.contains("Mini-Zork"),
            "the story never booted from the picker, so nothing here was measured"
        );
        let late: Vec<&str> = cap
            .answered
            .iter()
            .filter(|a| a.query.starts_with("default f") || a.query.starts_with("default b"))
            .map(|a| a.query)
            .collect();
        assert_eq!(late.len(), 2, "both colour queries must have been answered (late): {late:?}");
    }

    #[test]
    fn a_late_colour_answer_never_reaches_the_story() {
        let root = scratch("query-replies-leak");
        let Some((lib, user)) = library(&root) else {
            eprintln!("SKIP: minizork-r34-s871124.z3 fixture absent");
            return;
        };
        let mut spec = spec(&lib, &user);
        spec.keys = vec![
            Key::Wait(Duration::from_millis(1000)),
            Key::Bytes(b"\r".to_vec()), // launch the selected story from the picker
            Key::Wait(Duration::from_millis(2500)),
        ];

        let cap = driver::run(spec).expect("pty run");
        let text = String::from_utf8_lossy(&cap.bytes).to_string();
        assert_the_scenario_ran(&cap, &text);

        for needle in LEAKED {
            assert!(
                !text.contains(needle),
                "the terminal's colour answer was typed into the story: {needle:?} appears in \
                 what lanthorn painted, which is the SQ-0769 symptom verbatim"
            );
        }
    }

    #[test]
    fn a_keystroke_during_the_wait_still_reaches_the_story() {
        // The other half of the bargain: dropping the terminal's answers must not
        // cost the player a key. This types a command while the answers are still
        // owed — the window in which the fix owns the terminal — and asks the
        // story to prove it arrived.
        let root = scratch("query-replies-typeahead");
        let Some((lib, user)) = library(&root) else {
            eprintln!("SKIP: minizork-r34-s871124.z3 fixture absent");
            return;
        };
        let mut spec = spec(&lib, &user);
        // Later than the colour answers' own lateness would allow, so the typing
        // lands while the fix is holding the terminal rather than after it.
        spec.defer_by = Duration::from_millis(900);
        // **The typing is keyed to the PHASE, not to a stopwatch** (SQ-1162).
        // This used to be `Wait(320)` — 320 ms of quiet after the launch key —
        // and it reddened main once on a macOS runner and passed on a rerun of
        // the very same commit. That is what a deadline standing in for a phase
        // does: 320 ms of silence is "inside the boot probe" on an idle machine
        // and can be "before it started" on a loaded one, and typing outside the
        // window means the case asserts a property it never exercised. The
        // harness SEES the colour query go out, so waiting for it puts the
        // keystroke at the very start of the 900 ms the answers are held back
        // for, on any machine at any load. A failure here now means the
        // keystroke really was lost.
        spec.keys = vec![
            Key::Wait(Duration::from_millis(1000)),
            Key::Bytes(b"\r".to_vec()),
            Key::AwaitQuery {
                query: "default foreground (OSC 10)",
                cap: Duration::from_secs(20),
            },
            Key::Bytes(b"open mailbox\r".to_vec()),
            Key::Wait(Duration::from_millis(2500)),
        ];

        let cap = driver::run(spec).expect("pty run");
        let text = String::from_utf8_lossy(&cap.bytes).to_string();
        assert_the_scenario_ran(&cap, &text);

        // **And it really was typed DURING the wait** (SQ-1162). A command that
        // goes out after the answers have landed reaches the story for the dull
        // reason that nothing was holding the terminal any more, and passes this
        // case without touching the property it names. The window opens when the
        // app asks and closes when the harness finally answers, and both moments
        // are in the capture, so say so rather than trusting the schedule.
        let typed = cap
            .typed
            .iter()
            .find(|t| t.bytes.starts_with(b"open mailbox"))
            .expect("the command was typed");
        let answered = cap
            .answered
            .iter()
            .filter(|a| a.query.starts_with("default f") || a.query.starts_with("default b"))
            .map(|a| a.at)
            .min()
            .expect("a colour answer was sent");
        assert!(
            typed.at < answered,
            "the command went out at {:?}, after the first colour answer at {:?} — the terminal \
             was no longer being held, so this run proved nothing about typeahead",
            typed.at,
            answered
        );

        assert!(
            text.contains("leaflet"),
            "the typed command never reached the story — Mini-Zork answers `open mailbox` by \
             revealing a leaflet, and that never appeared"
        );
        for needle in LEAKED {
            assert!(!text.contains(needle), "and the answers still must not leak: {needle:?}");
        }
    }
}
