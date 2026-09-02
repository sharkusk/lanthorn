//! Who owns the terminal's input between a startup probe and the game's first
//! read (SQ-0769).
//!
//! THE DEFECT. `term_colors` asks the terminal for its default fg/bg (OSC 10 /
//! OSC 11) and terminates the batch with a DSR (`ESC[5n`) so it knows when every
//! answer is in. It drains the answers itself and gives up after a short silence.
//! A terminal that is busy — which is exactly what a picker launch leaves behind,
//! a screenful of kitty graphics and an alternate-screen switch — answers *after*
//! that. Nobody is reading by then, so the replies sit in the tty's input queue
//! until the next reader, which is the game. crossterm has no idea what an OSC
//! reply is (its parser bails on `ESC ]` and treats what follows as ordinary
//! characters), so the player gets
//!
//! ```text
//! 10;rgb:c0c0/c0c0/c0c011;rgb:0000/0000/0000
//! ```
//!
//! typed into the story: the intro's keypress waits are answered, the digits fall
//! through to a "restore a saved position?" prompt, and the terminal beeps.
//!
//! THE FIX, AND WHY THIS SHAPE. Draining the queue at one chosen moment only
//! moves the race — a terminal may answer later still. So the probe does not stop
//! listening at a time; it stops when it reads the answer to the question it
//! asked. `ESC[5n` was sent, so `ESC[0n` is owed; until that arrives the sweep
//! owns the tty and crossterm does not read it, and every byte goes through the
//! classifier below.
//!
//! HOW A REAL KEYSTROKE IS TOLD FROM QUERY TRAFFIC. A terminal's answer is always
//! an escape sequence — CSI, OSC, or a string sequence. A typed character is not.
//! So the classifier is a plain escape-sequence scanner: bytes inside a sequence
//! are the terminal talking back and are dropped; bytes outside one are the
//! player typing and are KEPT, replayed into the event loop as the key events
//! they would have been. Type-ahead during a slow boot survives — which is more
//! than the old drain managed, since it read those bytes into the reply buffer
//! and threw them away with it.
//!
//! WHAT IS DELIBERATELY LOST. While the sweep owns the tty, an escape sequence
//! that is *not* an answer — an arrow key, a mouse report — is discarded with the
//! answers, because nothing distinguishes them at the byte level without knowing
//! what the app asked. The window is short (it closes on the DSR answer, and in
//! any case after [`LATE_WINDOW`]) and only opens at all when the terminal has
//! already failed to answer in time, so the exposure is a fraction of a second at
//! the very start of a game.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// How long the sweep keeps the tty once the game's loop starts, waiting for an
/// answer that has not arrived. Generous, because the cost of waiting is only
/// borne when the terminal was late in the first place, and the alternative is
/// the answer landing in the story.
const LATE_WINDOW: Duration = Duration::from_millis(750);

/// The reply that ends every probe batch: our `ESC[5n` answered. Seeing it means
/// every earlier answer in the batch has already gone past the classifier.
const DSR_REPLY: &[u8] = b"\x1b[0n";

/// The longest escape sequence the scanner will hold before deciding it is not
/// going anywhere and resyncing. Real replies are tens of bytes.
const MAX_SEQ: usize = 512;

/// Owns the terminal's input while a probe answer is still owed, and carries the
/// keystrokes that arrived meanwhile.
#[derive(Default, Debug)]
pub struct QuerySweep {
    /// The probe's DSR answer has not been read yet — its replies are still in
    /// flight and this sweep, not crossterm, reads the tty.
    owed: bool,
    /// When the sweep gives up owning the tty. Set on the first [`Self::pump`],
    /// not at probe time: a story that takes seconds to boot must not have spent
    /// its window before the loop that uses it ever ran.
    until: Option<Instant>,
    scan: EscScan,
    /// Bytes the classifier judged to be the player typing, awaiting replay.
    typed: VecDeque<u8>,
}

impl QuerySweep {
    /// A probe that read its own DSR answer: nothing is owed, and `typed` is
    /// whatever the player typed into the drain while it waited.
    pub fn settled(typed: Vec<u8>) -> QuerySweep {
        QuerySweep { owed: false, until: None, scan: EscScan::default(), typed: typed.into() }
    }

    /// A probe that gave up before its DSR answer arrived: the answers are still
    /// coming, so the sweep holds the tty until they do.
    ///
    pub fn owed(typed: Vec<u8>) -> QuerySweep {
        QuerySweep { owed: true, until: None, scan: EscScan::default(), typed: typed.into() }
    }

    /// True while the sweep — not crossterm — is the tty's reader. The caller
    /// must not `read()` the terminal in this state; [`Self::pump`] is doing it.
    pub fn owns_input(&self) -> bool {
        self.owed
    }

    /// A replayable keystroke is waiting.
    pub fn has_event(&self) -> bool {
        !self.typed.is_empty()
    }

    /// Read whatever the terminal has sent, drop the answers, keep the typing.
    /// Non-blocking and cheap; a no-op once nothing is owed.
    pub fn pump(&mut self) {
        if !self.owed {
            return;
        }
        if !cfg!(unix) {
            // No non-blocking console read to hold the tty WITH (see
            // `read_available`), and holding it deaf would be worse than the leak.
            self.give_up();
            return;
        }
        let now = Instant::now();
        let until = *self.until.get_or_insert(now + LATE_WINDOW);
        let mut chunk = [0u8; 256];
        while let Some(n) = read_available(&mut chunk) {
            self.feed(&chunk[..n]);
            if !self.owed {
                return; // the answer arrived — the tty is crossterm's again
            }
        }
        if now >= until {
            self.give_up();
        }
    }

    /// Push read bytes through the classifier.
    fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.scan.push(b) {
                Classified::Typed(b) => self.typed.push_back(b),
                Classified::Partial => {}
                Classified::Sequence(seq) => {
                    if seq == DSR_REPLY {
                        // The question we asked has been answered: every reply in
                        // the batch is now behind us, so hand the tty back.
                        self.owed = false;
                    }
                }
            }
        }
    }

    /// Stop owning the tty. A sequence half-read at this point is abandoned,
    /// except a bare ESC, which is far more likely to be the player's Esc key
    /// than the start of an answer that never came.
    fn give_up(&mut self) {
        if self.scan.is_bare_escape() {
            self.typed.push_back(0x1b);
        }
        self.scan = EscScan::default();
        self.owed = false;
    }

    /// The next keystroke the sweep held back, as the event crossterm would have
    /// produced for it. Mirrors crossterm's non-escape parsing so a replayed key
    /// is indistinguishable from a typed one.
    pub fn next_event(&mut self) -> Option<Event> {
        let b = *self.typed.front()?;
        // Multi-byte UTF-8: wait for the whole character, and drop a truncated
        // one rather than emitting a replacement glyph into the story.
        let want = utf8_len(b);
        if want > 1 {
            if self.typed.len() < want {
                self.typed.clear();
                return None;
            }
            let buf: Vec<u8> = self.typed.drain(..want).collect();
            let ch = std::str::from_utf8(&buf).ok()?.chars().next()?;
            return Some(Event::Key(char_event(ch)));
        }
        self.typed.pop_front();
        Some(Event::Key(match b {
            b'\r' | b'\n' => KeyCode::Enter.into(),
            b'\t' => KeyCode::Tab.into(),
            0x7f => KeyCode::Backspace.into(),
            0x1b => KeyCode::Esc.into(),
            0x00 => KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL),
            0x01..=0x1a => KeyEvent::new(KeyCode::Char((b - 1 + b'a') as char), KeyModifiers::CONTROL),
            0x1c..=0x1f => KeyEvent::new(KeyCode::Char((b - 0x1c + b'4') as char), KeyModifiers::CONTROL),
            other => char_event(other as char),
        }))
    }
}

/// The bytes of `buf` that lie outside any escape sequence — the player typing,
/// as opposed to the terminal answering. Used on a probe's finished reply buffer,
/// which before SQ-0769 was discarded whole, type-ahead and all.
pub(crate) fn typed_bytes(buf: &[u8]) -> Vec<u8> {
    let mut scan = EscScan::default();
    let mut typed = Vec::new();
    for &b in buf {
        if let Classified::Typed(b) = scan.push(b) {
            typed.push(b);
        }
    }
    typed
}

/// crossterm adds SHIFT for an uppercase character; match it exactly.
fn char_event(c: char) -> KeyEvent {
    let m = if c.is_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::empty() };
    KeyEvent::new(KeyCode::Char(c), m)
}

/// Byte length of the UTF-8 character starting with `b` (1 for anything that is
/// not a well-formed leading byte, so the caller always makes progress).
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

/// What the classifier made of one byte.
enum Classified {
    /// Outside any escape sequence: the player typed this.
    Typed(u8),
    /// Inside a sequence that has not ended yet.
    Partial,
    /// A complete escape sequence, terminal-to-app traffic.
    Sequence(Vec<u8>),
}

/// An incremental scanner that splits a byte stream into escape sequences and
/// everything else. Deliberately structural — it recognises the SHAPE of a
/// sequence rather than any particular one, so a reply the app never asked for is
/// still recognised as the terminal talking rather than as typing.
#[derive(Default, PartialEq, Debug)]
enum EscState {
    #[default]
    Text,
    /// Seen ESC, waiting to learn which kind of sequence this is.
    Esc,
    /// `ESC [` … a final byte in 0x40..=0x7e.
    Csi,
    /// `ESC ]` … BEL or ST.
    Osc,
    /// `ESC P/X/^/_` … ST.
    StringSeq,
    /// An ESC inside an OSC/string sequence: `ESC \` ends it, anything else does
    /// not.
    StringEsc,
}

#[derive(Default, Debug)]
struct EscScan {
    state: EscState,
    seq: Vec<u8>,
}

impl EscScan {
    /// True when the scanner is holding a lone ESC and nothing more.
    fn is_bare_escape(&self) -> bool {
        self.state == EscState::Esc && self.seq.len() == 1
    }

    fn finish(&mut self) -> Classified {
        self.state = EscState::Text;
        Classified::Sequence(std::mem::take(&mut self.seq))
    }

    fn push(&mut self, b: u8) -> Classified {
        // A sequence that never ends is a stream we have lost the thread of;
        // resync rather than buffer for ever.
        if self.seq.len() >= MAX_SEQ {
            self.state = EscState::Text;
            self.seq.clear();
        }
        match self.state {
            EscState::Text => {
                if b == 0x1b {
                    self.state = EscState::Esc;
                    self.seq.push(b);
                    Classified::Partial
                } else {
                    Classified::Typed(b)
                }
            }
            EscState::Esc => {
                self.seq.push(b);
                match b {
                    b'[' => self.state = EscState::Csi,
                    b']' => self.state = EscState::Osc,
                    b'P' | b'X' | b'^' | b'_' => self.state = EscState::StringSeq,
                    // Any other byte completes a two-character escape sequence.
                    _ => return self.finish(),
                }
                Classified::Partial
            }
            EscState::Csi => {
                self.seq.push(b);
                // Parameter and intermediate bytes run until a final byte.
                if (0x40..=0x7e).contains(&b) { self.finish() } else { Classified::Partial }
            }
            EscState::Osc | EscState::StringSeq => {
                self.seq.push(b);
                match b {
                    0x07 => self.finish(), // BEL terminates an OSC
                    0x1b => {
                        self.state = EscState::StringEsc;
                        Classified::Partial
                    }
                    _ => Classified::Partial,
                }
            }
            EscState::StringEsc => {
                self.seq.push(b);
                if b == b'\\' {
                    self.finish()
                } else {
                    // Not a string terminator after all — still inside the string.
                    self.state = EscState::Osc;
                    Classified::Partial
                }
            }
        }
    }
}

/// Read whatever stdin has right now without blocking. `None` when there is
/// nothing (or nothing can be read); `Some(0)` never happens on a live tty and is
/// treated as end-of-input by the caller.
#[cfg(unix)]
fn read_available(chunk: &mut [u8; 256]) -> Option<usize> {
    let fd = libc::STDIN_FILENO;
    // SAFETY: fcntl F_GETFL/F_SETFL on a valid fd; no memory is involved.
    let old = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if old < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, old | libc::O_NONBLOCK) } < 0 {
        return None;
    }
    // SAFETY: reading at most `chunk.len()` bytes into a valid buffer.
    let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
    // SAFETY: restore the original flags whatever the read did.
    unsafe { libc::fcntl(fd, libc::F_SETFL, old) };
    (n > 0).then_some(n as usize)
}

/// Non-Unix (Windows): there is no portable non-blocking console read, and a
/// blocking one is exactly what swallowed the player's keystrokes in SQ-0642.
/// The sweep never owns the tty there — `term_colors` does not drain on Windows
/// either, so nothing is ever owed.
#[cfg(not(unix))]
fn read_available(_chunk: &mut [u8; 256]) -> Option<usize> {
    None
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    /// Push a whole byte string through the classifier and report what came out.
    fn classify(bytes: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut scan = EscScan::default();
        let (mut typed, mut seqs) = (Vec::new(), Vec::new());
        for &b in bytes {
            match scan.push(b) {
                Classified::Typed(b) => typed.push(b),
                Classified::Partial => {}
                Classified::Sequence(s) => seqs.push(s),
            }
        }
        (typed, seqs)
    }

    #[test]
    fn the_reported_leak_is_all_sequence_and_no_typing() {
        // The exact payload the player was shown (SQ-0769), reply-for-reply.
        let (typed, seqs) =
            classify(b"\x1b]10;rgb:c0c0/c0c0/c0c0\x07\x1b]11;rgb:0000/0000/0000\x07\x1b[0n");
        assert!(typed.is_empty(), "no byte of a colour reply is typing: {typed:?}");
        assert_eq!(seqs.len(), 3);
        assert_eq!(seqs[2], DSR_REPLY);
    }

    #[test]
    fn st_terminated_replies_are_one_sequence_each() {
        let (typed, seqs) = classify(b"\x1b]11;rgb:2828/2c2c/3434\x1b\\\x1b[0n");
        assert!(typed.is_empty(), "{typed:?}");
        assert_eq!(seqs.len(), 2);
    }

    #[test]
    fn typing_around_a_reply_survives_it() {
        // A player typing "look" while the terminal answers late: the answer goes,
        // the word stays, in order.
        let (typed, seqs) = classify(b"lo\x1b]10;rgb:c0c0/c0c0/c0c0\x07ok\x1b[0n");
        assert_eq!(typed, b"look");
        assert_eq!(seqs.len(), 2);
    }

    #[test]
    fn a_reply_split_across_reads_is_still_one_sequence() {
        let mut scan = EscScan::default();
        let (mut typed, mut seqs) = (Vec::new(), Vec::new());
        for part in [&b"\x1b]10;rgb:c0"[..], &b"c0/c0c0/c0c0\x07"[..]] {
            for &b in part {
                match scan.push(b) {
                    Classified::Typed(b) => typed.push(b),
                    Classified::Partial => {}
                    Classified::Sequence(s) => seqs.push(s),
                }
            }
        }
        assert!(typed.is_empty(), "{typed:?}");
        assert_eq!(seqs.len(), 1);
    }

    #[test]
    fn the_dsr_answer_hands_the_tty_back() {
        let mut sweep = QuerySweep::owed(Vec::new());
        assert!(sweep.owns_input());
        sweep.feed(b"\x1b]10;rgb:c0c0/c0c0/c0c0\x07");
        assert!(sweep.owns_input(), "still owed until the DSR answer arrives");
        sweep.feed(b"\x1b[0n");
        assert!(!sweep.owns_input());
    }

    #[test]
    fn held_keystrokes_replay_as_the_events_crossterm_would_have_made() {
        let mut sweep = QuerySweep::owed(Vec::new());
        sweep.feed(b"\x1b[0nHi!\r");
        let mut got = Vec::new();
        while let Some(e) = sweep.next_event() {
            got.push(e);
        }
        assert_eq!(
            got,
            vec![
                Event::Key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT)),
                Event::Key(KeyCode::Char('i').into()),
                Event::Key(KeyCode::Char('!').into()),
                Event::Key(KeyCode::Enter.into()),
            ]
        );
    }

    #[test]
    fn a_multibyte_character_replays_whole() {
        let mut sweep = QuerySweep::settled("é".as_bytes().to_vec());
        assert_eq!(sweep.next_event(), Some(Event::Key(KeyCode::Char('é').into())));
        assert_eq!(sweep.next_event(), None);
    }

    #[test]
    fn giving_up_keeps_a_bare_escape_as_the_esc_key() {
        let mut sweep = QuerySweep::owed(Vec::new());
        sweep.feed(b"\x1b");
        sweep.give_up();
        assert_eq!(sweep.next_event(), Some(Event::Key(KeyCode::Esc.into())));
        assert!(!sweep.owns_input());
    }

    #[test]
    fn a_settled_probe_never_takes_the_tty() {
        let mut sweep = QuerySweep::settled(Vec::new());
        assert!(!sweep.owns_input());
        sweep.pump(); // must not read, must not panic
        assert!(!sweep.has_event());
    }

    #[test]
    fn an_endless_sequence_resyncs_instead_of_growing() {
        let mut scan = EscScan::default();
        let mut typed = Vec::new();
        scan.push(0x1b);
        scan.push(b']');
        for _ in 0..MAX_SEQ * 2 {
            if let Classified::Typed(b) = scan.push(b'x') {
                typed.push(b);
            }
        }
        assert!(!typed.is_empty(), "the scanner must resync rather than buffer for ever");
        assert!(scan.seq.len() < MAX_SEQ);
    }
}
