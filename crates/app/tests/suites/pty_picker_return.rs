//! SQ-1474: quitting a game back to the story picker restores the picker's
//! position — same folder, same row — instead of snapping to the top of the
//! library.
//!
//! `resolve_picker_position` (`crates/app/src/picker_ui.rs`) is unit-tested
//! directly (`t-picker`) for the identity-match / fallback logic. What only a
//! real terminal can show is the OTHER half: that the real `/quit-to-library`
//! path — raw mode, the picker's own event loop, a real redraw — actually
//! reaches that function with the right position, and that the picker really
//! draws the restored row selected. This drives the real binary under a pty
//! (SQ-0762) to check exactly that.
//!
//! THE FIXTURE IS DELIBERATELY THE TRACKED PAIR: `crates/zvm/tests/fixtures/
//! minizork.z3` and `crates/scott/tests/tiny_cave.dat`, both already used
//! elsewhere as in-repo (non-gitignored) launchable stories, so this always
//! really runs — no `stories/` needed, on CI or in a fresh worktree.
//!
//! HOW THE ASSERTION AVOIDS DECODING COLOUR. The picker draws a literal
//! `▸ ` text marker in front of the selected row (`picker_ui.rs`'s
//! `draw_story_picker`), so which row is selected is legible from the
//! decoded TEXT alone — no need to resolve highlight colours through the
//! oracle. And since `tiny_cave` is a story only the `disks/` sub-folder
//! lists (the library root holds nothing but that one folder), a decoded
//! screen naming it at all already proves the picker is back in that
//! sub-folder; the marker sharing its row proves it is the one selected.

#[cfg(not(unix))]
#[test]
fn the_picker_return_test_is_unix_only() {
    eprintln!("SKIP: driving a real terminal needs a pty, which this platform does not have");
}

#[cfg(unix)]
use super::pty_stream;

#[cfg(unix)]
mod unix {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::pty_stream::{decode, driver};
    use driver::{Key, Spec};

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pty-capture").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// A library whose root holds exactly one sub-folder, `disks/`, which in
    /// turn holds two stories sorted "minizork" then "tiny_cave" (picker
    /// default sort is title, ascending — see `sort_stories`). Neither name
    /// appears anywhere the root itself would render, which is what lets the
    /// assertion below use "is `tiny_cave` on screen at all" as its directory
    /// check.
    fn library(root: &Path) -> (PathBuf, PathBuf) {
        let lib = root.join("library");
        let disks = lib.join("disks");
        let user = root.join("user");
        std::fs::create_dir_all(&disks).unwrap();
        std::fs::create_dir_all(&user).unwrap();

        let minizork = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../zvm/tests/fixtures/minizork.z3");
        let tiny_cave = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scott/tests/tiny_cave.dat");
        assert!(minizork.is_file(), "tracked fixture missing at {}", minizork.display());
        assert!(tiny_cave.is_file(), "tracked fixture missing at {}", tiny_cave.display());
        std::fs::copy(&minizork, disks.join("minizork.z3")).unwrap();
        std::fs::copy(&tiny_cave, disks.join("tiny_cave.dat")).unwrap();

        // A user dir that already knows the library, same as `pty_query_replies`'s
        // `library()`: the launch goes straight to the picker, no first-use prompt.
        std::fs::write(user.join("config.toml"), format!("default_story_dir = '{}'\n", lib.display())).unwrap();
        (lib, user)
    }

    #[test]
    fn quitting_a_game_returns_the_picker_to_the_sub_folder_and_row_it_was_launched_from() {
        let root = scratch("picker-return");
        let (lib, user) = library(&root);

        let mut spec = Spec::new(env!("CARGO_BIN_EXE_lanthorn"), &lib, &user);
        spec.cols = 100;
        spec.rows = 40;
        // The launched path is a directory (the library), not a story, so the
        // per-game `hide_map` sidecar has nothing to key off (mirrors
        // `pty_query_replies::spec`).
        spec.hide_map = false;
        spec.tail = Duration::from_millis(1200);
        spec.keys = vec![
            // The root holds only the `disks/` folder row (nothing else to
            // navigate past): Enter descends into it.
            Key::Wait(Duration::from_millis(1500)),
            Key::Bytes(b"\r".to_vec()),
            Key::Wait(Duration::from_millis(800)),
            // Entering a folder selects its `..` row first (row 0) —
            // `enter_folder` matches the descent's `came_from` against the new
            // listing's own `..` row, which happens to sort first. The full
            // listing is `..`, `Mini-Zork I`, `tiny_cave`, so two Downs reach
            // `tiny_cave` — launching the default top row wouldn't exercise
            // the restore at all, since a fresh scan already lands there.
            Key::Bytes(b"\x1b[B".to_vec()), // Down: `..` -> Mini-Zork
            Key::Wait(Duration::from_millis(500)),
            Key::Bytes(b"\x1b[B".to_vec()), // Down: Mini-Zork -> tiny_cave
            Key::Wait(Duration::from_millis(500)),
            Key::Bytes(b"\r".to_vec()), // launch tiny_cave
            Key::Wait(Duration::from_millis(2500)),
            // `/quit-to-library` is typed into the game's own command line,
            // exactly as `/save` or any other slash command is (docs/guide).
            Key::Bytes(b"/quit-to-library\r".to_vec()),
            Key::Wait(Duration::from_millis(2000)),
        ];

        let cap = driver::run(spec).expect("pty run");
        let text = String::from_utf8_lossy(&cap.bytes);

        // The run really did what the test claims: the scenario is picker,
        // then game, then picker again — three alternate-screen ENTERs (the
        // game's own boot is a fresh `EnterAlternateScreen`, distinct from
        // the picker's), not one. This is a sturdier "did it really run" gate
        // than grepping for the game's own room text: `tiny_cave` carries a
        // picture band (SQ-0876) whose kitty upload can still be in flight
        // when the quit keystroke lands, so its FIRST frame is not always
        // flushed to the wire before the picker's own redraw is — but the
        // screen transitions themselves are not racy the same way.
        let alt_screen_enters = text.matches("\x1b[?1049h").count();
        assert!(
            alt_screen_enters >= 3,
            "the picker → game → picker round trip never completed (only {alt_screen_enters} \
             alternate-screen entries seen), so nothing here was measured"
        );

        // Decode the WHOLE emitted stream into a screen model — the final
        // state is the picker, redrawn after the quit above.
        let mut term = decode::Term::new(cap.spec.cols, cap.spec.rows);
        term.feed(&cap.bytes);

        let mut selected_tiny_cave = false;
        let mut tiny_cave_on_screen = false;
        for row in 0..cap.spec.rows {
            let row_text = term.row_text(row, 0..cap.spec.cols);
            if row_text.contains("tiny_cave") {
                tiny_cave_on_screen = true;
                if row_text.contains('\u{25b8}') {
                    // ▸
                    selected_tiny_cave = true;
                }
            }
        }

        assert!(
            tiny_cave_on_screen,
            "the picker did not return to the `disks/` sub-folder at all (it only lists \
             `tiny_cave` there, not at the library root) — SQ-1474 symptom: the picker \
             reset to the top of the library instead of the folder the game was launched from"
        );
        assert!(
            selected_tiny_cave,
            "the picker returned to the right folder but not the right row: `tiny_cave` is on \
             screen but does not wear the ▸ selection marker — SQ-1474 symptom: the cursor \
             landed elsewhere in the list instead of on the story that was launched"
        );
    }
}
