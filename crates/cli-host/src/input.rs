//! Reading input, and knowing when there is none left.
//!
//! The whole module exists for one distinction the CLIs each had to learn
//! separately: **a blank line is not EOF**. `read_line` returning `Ok(0)` means
//! the stream is finished; returning `Ok(1)` with `"\n"` means the player
//! pressed Enter on an empty command, which is a perfectly good thing to hand a
//! game. Confuse the two and the host synthesizes a newline forever — the game
//! reprints its prompt, asks again, gets another fabricated newline, and the
//! process has to be killed.
//!
//! That bug shipped twice. `zvm-cli` fixed it in its line path long ago;
//! `gvm-cli` carried the identical defect until SQ-0604, where piping
//! `/dev/null` to Kerkerkruip repeated its "no destination yet" reply until the
//! process was killed. `zvm-cli`'s *char* path, meanwhile, got the EOF check
//! right but exited without restoring the terminal.
//!
//! So: one pair of readers that report EOF honestly, and one exit that always
//! puts the terminal back.

use std::io::{self, BufRead};

use crossterm::event::{Event, KeyEvent, KeyEventKind};

use crate::term::restore_and_exit;

/// The key event inside `ev` — but only when it is a *press*.
///
/// Crossterm delivers Release and Repeat key events too: on Windows it always
/// has (the console reports both edges), and kitty-protocol terminals can on
/// unix. A host that treats every key event as a keystroke doubles typed
/// characters, lets the Enter *release* terminate the next line read as a
/// phantom empty command, and lets a queued release dismiss a `[MORE]` prompt
/// (SQ-0633). Every place one of these hosts reads a key event must come
/// through this filter.
pub fn key_press(ev: &Event) -> Option<&KeyEvent> {
    match ev {
        Event::Key(k) if k.kind == KeyEventKind::Press => Some(k),
        _ => None,
    }
}

/// Read one line, or `None` at true EOF.
///
/// A 0-byte read is EOF. A blank line still yields one byte (`"\n"`) and is a
/// legitimate empty command. A read error is not recoverable input either, so it
/// reports EOF rather than pretending.
pub fn read_line_or_eof<R: BufRead>(r: &mut R) -> Option<String> {
    let mut line = String::new();
    match r.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line),
        Err(_) => None,
    }
}

/// The ZSCII code a `read_char` request should see for a line already read
/// (its line terminator, if any, still attached).
///
/// A `read_char` byte must be a legal ZSCII input code (ZMSD §3.8): Return is
/// ZSCII **13**, and 10 (LF) is not a legal input code at all. A bare Enter on
/// a piped/cooked line arrives as `"\n"` (or `"\r\n"`) with no content before
/// the terminator — that must be read back as 13, not as the raw LF byte, or a
/// game asking `read_char` for a keypress (rather than a line) receives a code
/// it is never allowed to see (SQ-1423). A line with real content still
/// reports its first character as before.
///
/// Shared by [`read_byte_or_eof`] and any host that already owns the whole
/// line — a cooked-terminal char read that borrows the line editor to get
/// escape-free digits, say — so the mapping cannot drift between the two.
pub fn read_char_from_line(line: &str) -> u8 {
    let content = line.trim_end_matches(['\n', '\r']);
    content.bytes().next().unwrap_or(13)
}

/// Read one line and return the ZSCII code of the key it represents (see
/// [`read_char_from_line`]), or `None` at true EOF.
pub fn read_byte_or_eof<R: BufRead>(r: &mut R) -> Option<u8> {
    read_line_or_eof(r).map(|line| read_char_from_line(&line))
}

/// [`read_line_or_eof`] against the real stdin.
pub fn read_line_stdin() -> Option<String> {
    read_line_or_eof(&mut io::stdin().lock())
}

/// [`read_byte_or_eof`] against the real stdin.
pub fn read_byte_stdin() -> Option<u8> {
    read_byte_or_eof(&mut io::stdin().lock())
}

/// Stop cleanly at end of piped input: restore the terminal and exit 0.
///
/// The game asked for input and there is none; the only honest answers are to
/// stop or to hang. `prefix` carries any renderer teardown (a scroll-region
/// reset, say) that must go out before the terminal is handed back.
///
/// The restore matters even here, where stdin is a pipe: stdout may still be a
/// terminal — `echo commands | zvm-cli story.z5` — and it is wearing the game's
/// page background and a block cursor.
pub fn exit_at_eof(prefix: &str) -> ! {
    restore_and_exit(prefix, 0)
}

/// The line that asks the host — not the game — to repeat the status.
pub const STATUS_COMMAND: &str = "/status";

/// Is this input line a status request rather than a game command?
///
/// Status information reaches a plain-mode reader only when the game chooses to
/// write it, and then it scrolls away. A sighted player re-reads a pinned status
/// line for free; a listener would have to scroll back through the transcript to
/// answer "where am I, what is my score?" (SQ-0610).
///
/// The leading slash is what makes interception safe. Any bare word — `status`,
/// `score`, `look` — risks shadowing a verb the game itself defines, and a host
/// that silently eats a real command is worse than no feature. No interactive
/// fiction parser assigns meaning to a leading `/`, and lanthorn's own TUI
/// already spells its host commands that way, so the vocabulary is not new.
pub fn is_status_request(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case(STATUS_COMMAND)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_line_is_input_not_eof() {
        let mut r = io::Cursor::new(b"\n".to_vec());
        assert_eq!(read_line_or_eof(&mut r), Some("\n".to_string()));
        assert_eq!(read_line_or_eof(&mut r), None, "now it really is EOF");

        // The byte reader must draw the same line as real input, not EOF —
        // reading it as EOF (or reading EOF as a keypress) is the loop that
        // hung Kerkerkruip on /dev/null (SQ-0604). But the byte it reports for
        // a bare Enter is ZSCII 13, not the raw LF: see
        // `blank_line_reads_as_return_not_lf` below (SQ-1423).
        let mut r = io::Cursor::new(b"\n".to_vec());
        assert_eq!(read_byte_or_eof(&mut r), Some(13));
        assert_eq!(read_byte_or_eof(&mut r), None);
    }

    #[test]
    fn blank_line_reads_as_return_not_lf() {
        // ZMSD §3.8: Return is ZSCII 13; 10 (LF) is not a legal `read_char`
        // input code at all. A bare Enter on piped stdin is a blank line, and
        // `read_line` hands that back as the raw terminator byte — feeding it
        // straight to `@read_char` is exactly what gntests' InputCodes section
        // catches as "code 10 should not have been returned" (SQ-1423).
        let mut lf = io::Cursor::new(b"\n".to_vec());
        assert_eq!(read_byte_or_eof(&mut lf), Some(13), "bare LF must read back as Return");

        let mut crlf = io::Cursor::new(b"\r\n".to_vec());
        assert_eq!(read_byte_or_eof(&mut crlf), Some(13), "CRLF must read back as Return too");

        let mut lone_cr = io::Cursor::new(b"\r".to_vec());
        assert_eq!(read_byte_or_eof(&mut lone_cr), Some(13), "a lone unterminated CR is Return");

        // Real content is unaffected: the first character still wins, whatever
        // the line ending is.
        let mut content = io::Cursor::new(b"yes\n".to_vec());
        assert_eq!(read_byte_or_eof(&mut content), Some(b'y'));
        let mut content_crlf = io::Cursor::new(b"north\r\n".to_vec());
        assert_eq!(read_byte_or_eof(&mut content_crlf), Some(b'n'));
    }

    #[test]
    fn empty_stream_is_eof_immediately() {
        assert_eq!(read_line_or_eof(&mut io::Cursor::new(Vec::new())), None);
        assert_eq!(read_byte_or_eof(&mut io::Cursor::new(Vec::new())), None);
    }

    #[test]
    fn lines_are_returned_with_their_terminator_and_first_byte() {
        let mut r = io::Cursor::new(b"look\nnorth\n".to_vec());
        assert_eq!(read_line_or_eof(&mut r), Some("look\n".to_string()));
        assert_eq!(read_line_or_eof(&mut r), Some("north\n".to_string()));
        assert_eq!(read_line_or_eof(&mut r), None);

        let mut r = io::Cursor::new(b"yes\n".to_vec());
        assert_eq!(read_byte_or_eof(&mut r), Some(b'y'));
    }

    #[test]
    fn status_request_is_recognised_leniently_but_never_greedily() {
        assert!(is_status_request("/status"));
        assert!(is_status_request("  /status  \n"), "trimmed");
        assert!(is_status_request("/STATUS"), "case-insensitive");
        // Anything that could be a real game command must pass straight through:
        // eating a verb the game defines is worse than not having the feature.
        for line in ["status", "score", "look", "/status now", "x /status", "/", ""] {
            assert!(!is_status_request(line), "{line:?} must reach the game");
        }
    }

    #[test]
    fn only_a_press_counts_as_a_keystroke() {
        use crossterm::event::{KeyCode, KeyEventState, KeyModifiers};
        let ev = |kind| {
            Event::Key(KeyEvent {
                code: KeyCode::Char('x'),
                modifiers: KeyModifiers::NONE,
                kind,
                state: KeyEventState::NONE,
            })
        };
        assert!(key_press(&ev(KeyEventKind::Press)).is_some());
        // SQ-0633: Windows delivers both edges; the release of the Enter that
        // submitted the previous command must not be read as a new keystroke.
        assert!(key_press(&ev(KeyEventKind::Release)).is_none(), "a release is not a keystroke");
        assert!(key_press(&ev(KeyEventKind::Repeat)).is_none(), "nor is an autorepeat");
        assert!(key_press(&Event::Resize(80, 24)).is_none(), "nor a resize");
    }

    #[test]
    fn key_press_hands_back_the_event_it_accepted() {
        use crossterm::event::{KeyCode, KeyEventState, KeyModifiers};
        let k = KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let ev = Event::Key(k);
        let got = key_press(&ev).expect("a press passes through");
        // The caller needs code AND modifiers — Ctrl-C handling reads both.
        assert_eq!(got.code, KeyCode::Char('c'));
        assert!(got.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn unterminated_final_line_is_input_then_eof() {
        // A pipe whose last line has no trailing newline still carries a command.
        let mut r = io::Cursor::new(b"quit".to_vec());
        assert_eq!(read_line_or_eof(&mut r), Some("quit".to_string()));
        assert_eq!(read_line_or_eof(&mut r), None);
    }
}
