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
pub struct TextAttrs {
    pub style: u8,
    pub fg: ZColour,
    pub bg: ZColour,
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
    fn as_any(&self) -> &dyn Any;
    /// Mutable downcast support — required to drain sink state a concrete sink
    /// exposes beyond this trait (e.g. a transcript sink's own "take buffered
    /// text" method).
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Simple accumulating sink for tests and headless use.
pub struct BufferOutput {
    pub buf: String,
}

impl Default for BufferOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl BufferOutput {
    pub fn new() -> Self {
        BufferOutput { buf: String::new() }
    }
}

impl Output for BufferOutput {
    fn print(&mut self, s: &str) {
        self.buf.push_str(s);
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
