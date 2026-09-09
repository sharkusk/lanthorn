//! Output sink abstraction — ZMSD §7.
//!
//! [`Output`] is the pluggable interface for all text the Z-machine emits.
//! [`BufferOutput`] is a test/headless sink that accumulates output into a
//! `String`; a host rendering a real screen implements the trait itself —
//! lanthorn's `CaptureSink`, which drives a scrolling terminal transcript, is
//! one such implementation.
//!
//! # Why `Output: Any`, and why it stays that way (SQ-1402)
//!
//! The trait requires [`Any`], which in turn requires `'static`: every
//! implementor must own everything it touches rather than borrow it from its
//! host, because [`crate::cpu::exec::Machine::boot`] /
//! [`crate::cpu::exec::Machine::with_output`] take the sink as `Box<dyn
//! Output>` and hold it for the machine's whole lifetime. The bound exists so
//! test code can downcast a `Box<dyn Output>` back to a concrete type — this
//! crate's own tests read [`BufferOutput::buf`] that way, and every other host
//! in this workspace (`zvm-cli`, lanthorn's `app`) downcasts its own sink to
//! reach state the trait doesn't expose. That is a real cost paid by every
//! implementor whether or not it ever downcasts anything, and reviewed and
//! kept as-is rather than redesigned: the fix is not obvious (an associated
//! type or a second, non-`Any` trait both move the cost elsewhere rather than
//! remove it) and no embedder has yet needed a sink that cannot afford to own
//! its state.
//!
//! **The pattern for a host whose sink must reach state it does not own:**
//! don't fight `'static` by borrowing — share the state behind a
//! reference-counted cell instead. [`std::rc::Rc`]`<`[`std::cell::RefCell`]`<_>>`
//! for a single-threaded host, [`std::sync::Arc`]`<`[`std::sync::Mutex`]`<_>>`
//! across threads. The sink owns a clone of the handle (satisfying `'static`
//! honestly, not by unsafely erasing a borrow), the host keeps its own clone,
//! and both sides see the same buffer with no downcast anywhere:
//!
//! ```rust
//! use std::cell::RefCell;
//! use std::rc::Rc;
//!
//! use zvm::io::Output;
//!
//! struct SharedSink(Rc<RefCell<String>>);
//!
//! impl Output for SharedSink {
//!     fn print(&mut self, s: &str) {
//!         self.0.borrow_mut().push_str(s);
//!     }
//!     fn as_any(&self) -> &dyn std::any::Any {
//!         self
//!     }
//!     fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
//!         self
//!     }
//! }
//!
//! // A `Box<dyn Output>` is exactly what `Machine::boot` takes, and exactly
//! // what a real host's step loop would call `.print()` on as the story runs.
//! fn a_steps_worth_of_output(mut sink: Box<dyn Output>) {
//!     sink.print("hello");
//! }
//!
//! let buf = Rc::new(RefCell::new(String::new()));
//! a_steps_worth_of_output(Box::new(SharedSink(Rc::clone(&buf))));
//! // The host reads its OWN clone after the step, no downcast required —
//! // the whole point of sharing the cell instead of asking the sink back for it.
//! assert_eq!(buf.borrow().as_str(), "hello");
//! ```

use std::any::Any;

use crate::screen::ZColour;

/// Text attributes for one styled run (logical colour, pre-reverse-swap).
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct TextAttrs {
    pub style: u8,
    pub fg: ZColour,
    pub bg: ZColour,
}

impl TextAttrs {
    /// Build a `TextAttrs` from its three fields. `#[non_exhaustive]` blocks
    /// the struct-literal form outside this crate (SQ-1404), so this is the
    /// constructor a host reaches for; use `..Default::default()` on a
    /// literal only from within `zvm` itself.
    pub const fn new(style: u8, fg: ZColour, bg: ZColour) -> Self {
        TextAttrs { style, fg, bg }
    }
}

/// Trait for Z-machine text output sinks.
pub trait Output: Any {
    fn print(&mut self, s: &str);
    /// Print `s` carrying the current Z-machine text-style bitmask
    /// (ZMSD §8.7.1: 1=reverse, 2=bold, 4=italic, 8=fixed-pitch). The default
    /// ignores the style and delegates to `print`, so existing sinks are
    /// unaffected until they override this.
    fn print_styled(&mut self, s: &str, _style: u8) {
        self.print(s);
    }
    /// Print `s` carrying full text attributes (style bitmask + logical
    /// colour). The default delegates to `print_styled`, so sinks that do not
    /// render colour are unaffected.
    fn print_attr(&mut self, s: &str, attrs: TextAttrs) {
        self.print_styled(s, attrs.style);
    }
    /// Notify the sink that the Z-machine `buffer_mode` opcode changed the
    /// buffering flag (ZMSD §7.2.1: buffering is on at the start of a game).
    /// When `on` is `false` the interpreter must NOT word-wrap output — text
    /// breaks after the last character that fits (explicit `\n` and paging still
    /// apply).
    ///
    /// The default is a no-op, which is right only for sinks that never wrap
    /// (e.g. `BufferOutput`, which just accumulates a `String`). Any sink that
    /// lays text out in columns MUST override it — a plain stdout host stops
    /// soft-wrapping, and a scrolling-transcript host (lanthorn's
    /// `CaptureSink`) flags the runs it captures so the transcript
    /// char-breaks them.
    fn set_buffer_mode(&mut self, _on: bool) {}
    /// Notify the sink that `erase_window` just cleared the scrolling window it is
    /// capturing (ZMSD §8.7.3.3), AT THIS POINT in the character stream.
    ///
    /// The flag alone (`ScreenState::erase_lower_requested`) says only that an erase
    /// happened somewhere in the turn, so a host that reads it after the turn has to
    /// assume the erase came first. A turn that PRINTS and then erases would keep its
    /// pre-erase text on the cleared screen (SQ-0751). The erase's position in the
    /// stream is knowable only while it executes, and only the sink knows how many
    /// characters it has taken, so the VM tells it rather than counting for it.
    ///
    /// The default is a no-op, right for every sink that does not model a screen
    /// boundary (`BufferOutput`, a plain stdout host that prints straight
    /// through). A sink modelling a scrolling transcript (lanthorn's
    /// `CaptureSink`) overrides it.
    fn screen_cleared(&mut self) {}
    /// Output stream 2 — the game transcript (ZMSD §7.1.1: "number 1 (the
    /// screen) and 2 (the game transcript, usually printed to a printer or a
    /// file)").
    ///
    /// The machine calls this with every run it prints to stream 1 while stream
    /// 2 is selected — and, in Versions 1 to 5, with the player's input line as
    /// typed, because §7.1.1.1 says "the player's input to the `read` opcode
    /// should be echoed to output streams 1 and 2 (if stream 2 is active), so
    /// that text typed in appears in any transcript. In Version 6 input should
    /// be sent only to stream 1 and it is the game's responsibility to write to
    /// the transcript."
    ///
    /// Which windows contribute is decided inside the machine (ZMSD §8.8.3.1
    /// attribute 2, "text copied to output stream 2"), as is the §7.1.2.2 rule
    /// that stream 3 suppresses every other stream while it is selected — a sink
    /// receives only what belongs in the transcript.
    ///
    /// **The text arrives raw, exactly as printed.** ZMSD §7.2 permits
    /// word-wrapping on streams 1 and 2 alike, but does not require it, and
    /// Frotz writes its script file unwrapped (`files.c` `script_char`, which
    /// only breaks a line when the game prints a newline); this crate does the
    /// same, so a sink that wants columns must wrap for itself.
    ///
    /// **zvm never opens a file.** The default is a no-op — an interpreter that
    /// leaves it so is exercising ZMSD §7.6.5 ("Interpreters are allowed to not
    /// support access to external files (such as with `output_stream 2`)"), and
    /// should say so to the player per §7.6.5.2. A host that wants transcripts
    /// implements this and does its own I/O.
    fn transcript(&mut self, _text: &str) {}
    /// Output stream 4 — the command record (ZMSD §7.1.2: "a script file of the
    /// player's whole commands and of individual keypresses as read by
    /// `read_char`"; §7.1.2.3: "Each command is written, in one go, when it has
    /// been finished … Mistypes and uses of 'delete' are not written").
    ///
    /// One call per finished record, carrying the record WITHOUT its trailing
    /// newline; the sink supplies the line separator its file format wants. The
    /// text is already escaped for the stream-4 file format — see
    /// [`encode_command_record`], which is also what [`Output::next_command`]'s
    /// lines are decoded with, since ZMSD §10.2.1 requires that "the format of a
    /// file containing commands must be the same as that written in output
    /// stream 4".
    fn command_record(&mut self, _line: &str) {}
    /// Input stream 1 — the next recorded command (ZMSD §10.2: "the player's
    /// keypresses are drawn from the current 'input stream'. There are two input
    /// streams: numbered 0 (the keyboard) and 1 (a file containing commands)").
    ///
    /// The machine calls this instead of suspending for the player whenever the
    /// game has selected input stream 1 with the `input_stream` opcode, and
    /// decodes the answer with [`decode_command_record`] — the mirror image of
    /// [`Output::command_record`], per §10.2.1.
    ///
    /// Return one record per call, without its trailing newline. `None` means
    /// end of file: the machine then reverts to input stream 0 and suspends for
    /// the keyboard as usual. (The standard does not say what happens at the end
    /// of a command file; reverting is Frotz's behaviour — `files.c`
    /// `replay_close`, reached from every short read — and Bocfel's.)
    ///
    /// The default returns `None`, so a host that has not implemented command
    /// files simply never leaves the keyboard.
    fn next_command(&mut self) -> Option<String> {
        None
    }
    fn as_any(&self) -> &dyn Any;
    /// Mutable downcast support — required to drain sink state a concrete sink
    /// exposes beyond this trait (e.g. a transcript sink's own "take buffered
    /// text" method).
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Encode one finished stream-4 record — a line of ZSCII input codes — into the
/// text form written to a command file (ZMSD §7.1.2.3).
///
/// The standard fixes the file's *content* ("the player's whole commands and of
/// individual keypresses as read by `read_char`") but not its spelling; §7's
/// Remarks give the convention every interpreter follows, a printable character
/// written as itself and anything else as its ZSCII code in square brackets:
///
/// ```text
/// take lamp              an ordinary command
/// turn it on.[154]       command, full stop, then keypad 9
/// [254][10][6]           mouse-click at (10,6)
/// ```
///
/// So: ZSCII 32–126 are written literally, except `[` itself (written `[91]`,
/// or the reader could not tell an escape from a bracket the player typed), and
/// every other code — accented letters, function keys, `read_char`'s arrow keys
/// — as `[N]` in decimal. That is condition for condition Frotz's `record_code`
/// (`src/common/files.c`: `force_encoding || c == '[' || c < 0x20 || c > 0x7e`),
/// so a lanthorn command file and a Frotz one are the same file. The one thing
/// Frotz writes that this cannot is its `1000 + hot-key` range, which encodes
/// Frotz's OWN interpreter hot-keys rather than anything the Z-machine reads;
/// [`decode_command_record`] treats such a code as literal text.
pub fn encode_command_record(codes: &[u8]) -> String {
    let mut out = String::with_capacity(codes.len());
    for &c in codes {
        if (0x20..=0x7e).contains(&c) && c != b'[' {
            out.push(c as char);
        } else {
            out.push('[');
            out.push_str(&c.to_string());
            out.push(']');
        }
    }
    out
}

/// Decode one line of a command file back into ZSCII input codes — the inverse
/// of [`encode_command_record`], and the reason input stream 1 can read what
/// output stream 4 wrote (ZMSD §10.2.1: "The format of a file containing
/// commands must be the same as that written in output stream 4").
///
/// Tolerant by design, because the file is a text file a tester edits by hand: a
/// `[` that does not open a well-formed decimal escape, and a code past 255, are
/// both taken literally rather than rejected. A malformed script should replay
/// as far as it can, not fault the machine.
pub fn decode_command_record(line: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            // Scan for a well-formed `[<decimal>]` that fits in a ZSCII byte.
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == ']') {
                let digits: String = chars[i + 1..i + 1 + close].iter().collect();
                if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                    if let Ok(code) = digits.parse::<u16>() {
                        if code <= 255 {
                            out.push(code as u8);
                            i += close + 2;
                            continue;
                        }
                    }
                }
            }
        }
        // Not an escape: take the character itself. A non-ASCII char in a
        // hand-edited file has no ZSCII code without the story's Unicode table,
        // which this function does not have, so it is dropped rather than
        // truncated into a wrong code.
        let c = chars[i];
        if (c as u32) <= 0xff {
            out.push(c as u8);
        }
        i += 1;
    }
    out
}

/// Simple accumulating sink for tests and headless use.
///
/// Streams 2 and 4 accumulate the same way stream 1 does — into strings, not
/// files, since this crate opens none (see [`Output::transcript`]) — and
/// [`BufferOutput::replay`] seeds input stream 1 from a slice of records so a
/// test can drive [`Output::next_command`] without a filesystem.
pub struct BufferOutput {
    pub buf: String,
    /// Everything routed to output stream 2 (ZMSD §7.1.1), concatenated.
    pub transcript: String,
    /// One entry per finished output-stream-4 record (ZMSD §7.1.2.3), in order,
    /// each already in the [`encode_command_record`] text form.
    pub commands: Vec<String>,
    /// Records still to be handed to input stream 1, front first (ZMSD §10.2).
    /// Empty means end of file, which reverts the machine to the keyboard.
    pub replay: std::collections::VecDeque<String>,
}

impl Default for BufferOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl BufferOutput {
    pub fn new() -> Self {
        BufferOutput {
            buf: String::new(),
            transcript: String::new(),
            commands: Vec::new(),
            replay: std::collections::VecDeque::new(),
        }
    }

    /// Seed input stream 1 with `records` (ZMSD §10.2), each in the
    /// [`encode_command_record`] form — the same text a command FILE holds.
    pub fn replay_records<I: IntoIterator<Item = S>, S: Into<String>>(&mut self, records: I) {
        self.replay = records.into_iter().map(Into::into).collect();
    }
}

impl Output for BufferOutput {
    fn print(&mut self, s: &str) {
        self.buf.push_str(s);
    }
    fn transcript(&mut self, text: &str) {
        self.transcript.push_str(text);
    }
    fn command_record(&mut self, line: &str) {
        self.commands.push(line.to_string());
    }
    fn next_command(&mut self) -> Option<String> {
        self.replay.pop_front()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod print_styled_tests {
    use super::*;

    #[test]
    fn default_print_styled_delegates_to_print() {
        let mut a = BufferOutput::new();
        let mut b = BufferOutput::new();
        a.print("hello");
        b.print_styled("hello", 0x02); // style ignored by default impl
        assert_eq!(a.buf, b.buf, "default print_styled must equal print");
    }

    #[test]
    fn default_print_attr_delegates_to_print_styled() {
        use crate::screen::ZColour;
        let mut a = BufferOutput::new();
        let mut b = BufferOutput::new();
        a.print_styled("hi", 0x02);
        b.print_attr("hi", TextAttrs { style: 0x02, fg: ZColour::Standard(3), bg: ZColour::Default });
        assert_eq!(a.buf, b.buf, "default print_attr falls back to print_styled");
    }
}

#[cfg(test)]
mod command_record_tests {
    use super::*;

    #[test]
    fn printable_ascii_is_written_literally() {
        // ZMSD §7 Remarks: "take lamp — an ordinary command".
        assert_eq!(encode_command_record(b"take lamp"), "take lamp");
        assert_eq!(decode_command_record("take lamp"), b"take lamp");
    }

    #[test]
    fn function_key_is_bracketed_by_its_zscii_code() {
        // §7 Remarks: "turn it on.[154] — command, full stop, then keypad 9".
        let mut codes = b"turn it on.".to_vec();
        codes.push(154);
        assert_eq!(encode_command_record(&codes), "turn it on.[154]");
        assert_eq!(decode_command_record("turn it on.[154]"), codes);
    }

    #[test]
    fn a_typed_bracket_is_escaped_so_it_cannot_read_back_as_one() {
        // Frotz's `record_code` forces the encoding for '[' for this reason.
        assert_eq!(encode_command_record(b"say [hi]"), "say [91]hi]");
        assert_eq!(decode_command_record("say [91]hi]"), b"say [hi]");
    }

    #[test]
    fn round_trips_every_zscii_byte() {
        let all: Vec<u8> = (0u8..=255).collect();
        assert_eq!(decode_command_record(&encode_command_record(&all)), all);
    }

    #[test]
    fn malformed_escapes_decode_literally_rather_than_failing() {
        // A hand-edited script should replay as far as it can (see the doc
        // comment): an unterminated bracket, a non-numeric one and an
        // out-of-range code are all just characters.
        assert_eq!(decode_command_record("[abc]"), b"[abc]");
        assert_eq!(decode_command_record("[12"), b"[12");
        assert_eq!(decode_command_record("[300]"), b"[300]");
    }

    #[test]
    fn buffer_output_replay_hands_records_out_in_order_then_eof() {
        let mut b = BufferOutput::new();
        b.replay_records(["north", "take lamp"]);
        assert_eq!(b.next_command().as_deref(), Some("north"));
        assert_eq!(b.next_command().as_deref(), Some("take lamp"));
        assert_eq!(b.next_command(), None, "exhausted replay reports end of file");
    }
}
