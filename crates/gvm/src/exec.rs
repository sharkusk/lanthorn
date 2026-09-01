// Glulx execution engine — GLULX_NOTES.md §4 (stack, call frames, calling
// convention). Instruction decode/dispatch and the run loop are layered on in
// later tasks.
//
// The stack is a single byte-addressed buffer (`stack`) sized to the header's
// stack size, with a stack pointer `sp` (bytes used) and a frame pointer `fp`.
// A call frame is laid out exactly as the spec's diagram: FrameLen, LocalsPos,
// the locals-format list, the locals (each at natural alignment), then the
// value-stack region. A four-word "call stub" sits just below each non-start
// frame so a return can restore the caller.

use crate::error::GError;
use crate::glk::{self, GlkBackend, GlkEvent, GlkStyle, Model, StreamKind, WinType};
use crate::memory::Memory;
use crate::unicode_norm;

/// A recoverable runtime fault. Carries a human-readable diagnostic; the run
/// loop records it and Quits rather than panicking.
pub(crate) type R<T> = Result<T, String>;

/// The outcome of a single [`Machine::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// Execution should continue with the next instruction.
    Continue,
    /// Execution has ended (`quit`, an outer return, or a recorded fault).
    Quit,
    /// A `glk_select` is pending a **line**-input event on window `win`. The host
    /// supplies the typed line via [`Machine::supply_line`], then resumes.
    NeedLine {
        /// The window awaiting line input.
        win: u32,
    },
    /// A `glk_select` is pending a **character**-input event on window `win`. The
    /// host supplies the keystroke via [`Machine::supply_char`], then resumes.
    /// `unicode` is set for the `_uni` request (a full code point is accepted).
    NeedChar {
        /// The window awaiting a keystroke.
        win: u32,
        /// Whether the request was the Unicode (`_uni`) variant.
        unicode: bool,
    },
    /// The game executed `@save`: the host should capture [`Machine::save_state`],
    /// write it to a file, then call [`Machine::complete_save`] with the result.
    SaveRequest,
    /// The game executed `@restore`: the host should read a save file and call
    /// [`Machine::complete_restore_success`] (or [`Machine::complete_restore_failure`]).
    RestoreRequest,
    /// The game executed `glk_fileref_create_by_prompt`: the host should prompt for
    /// a filename (write modes) or let the player pick an existing VFS file (read
    /// mode), then call [`Machine::supply_filename`]. `usage` is the Glk fileusage,
    /// `fmode` the Glk filemode.
    NeedFilename {
        /// The Glk fileusage (Data / SavedGame / Transcript / InputRecord + flags).
        usage: u32,
        /// The Glk filemode (Read `0x02`, Write `0x01`, ReadWrite `0x03`, WriteAppend `0x05`).
        fmode: u32,
    },
    /// A `glk_select` is pending a **non-input** event: the game armed only a
    /// timer, mouse, and/or hyperlink request (no line/char input) and is blocked
    /// until one fires (Glk spec §4.4 — such a `glk_select` must block, never
    /// return `evtype_None`). The host resolves it by delivering the event: a
    /// timer tick on its own clock ([`Machine::deliver_timer`]) when `timer_ms`
    /// is set, or a mouse/hyperlink event on a user action
    /// ([`Machine::deliver_mouse`]/[`Machine::deliver_hyperlink`]) — then the VM
    /// resumes.
    NeedEvent {
        /// The armed timer interval in ms, or `None` when no timer is running.
        timer_ms: Option<u32>,
        /// Whether any window has a pending mouse request.
        mouse: bool,
        /// Whether any window has a pending hyperlink request.
        hyperlink: bool,
    },
}

/// Where a produced value (a function's return value, or an opcode's store
/// operand) should go. Mirrors the spec's call-stub DestType encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dest {
    /// Throw the value away (DestType 0).
    Discard,
    /// Push onto the value stack (DestType 3).
    Push,
    /// Store to main memory at this address (DestType 1).
    Mem(u32),
    /// Store to a call-frame local at this byte offset (DestType 2).
    Local(u32),
}

/// The widest operand shape any Glulx opcode decodes: the search family
/// (`linearsearch`/`binarysearch` at 7 loads + 1 store) tops the ISA, and every
/// `read_operands` call site names its counts statically — so 8 is a bound, not
/// a guess. Decoding runs once per instruction; giving it a fixed ceiling is
/// what lets [`Operands`] live on the stack instead of the heap (SQ-1208).
const MAX_OPERANDS: usize = 8;

/// A decoded operand list with a fixed capacity of [`MAX_OPERANDS`] — the
/// zero-allocation replacement for the per-instruction `Vec`s that put the
/// allocator at half of VM time (SQ-1208). Derefs to a slice, so call sites
/// index and subslice it exactly as they did the `Vec`.
pub(crate) struct Operands<T: Copy> {
    buf: [T; MAX_OPERANDS],
    len: usize,
}

impl<T: Copy> Operands<T> {
    fn new(fill: T) -> Self {
        Self { buf: [fill; MAX_OPERANDS], len: 0 }
    }

    /// Append one operand. Indexing panics past [`MAX_OPERANDS`] — that is a
    /// new opcode outgrowing the bound above, which must be raised with it.
    fn push(&mut self, v: T) {
        self.buf[self.len] = v;
        self.len += 1;
    }
}

impl<T: Copy> std::ops::Deref for Operands<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.buf[..self.len]
    }
}

/// Which case fold a `glk_buffer_to_*_case_uni` selector performs.
#[derive(Clone, Copy)]
enum CaseOp {
    /// `glk_buffer_to_lower_case_uni`.
    Lower,
    /// `glk_buffer_to_upper_case_uni`.
    Upper,
    /// `glk_buffer_to_title_case_uni` (`lower_rest` = lowercase the tail).
    Title {
        /// Whether to lowercase everything after the first character.
        lower_rest: bool,
    },
}

/// Which Unicode normalization a `glk_buffer_canon_*_uni` selector performs.
#[derive(Clone, Copy)]
enum NormForm {
    /// `glk_buffer_canon_decompose_uni` — canonical decomposition (NFD).
    Nfd,
    /// `glk_buffer_canon_normalize_uni` — canonical composition (NFC).
    Nfc,
}

/// A suspended `glk_select` awaiting a host-supplied event.
struct PendingInput {
    /// Glulx address of the `event_t` to fill when the event arrives.
    event_addr: u32,
    /// The window the input was requested on.
    win: u32,
    /// `true` = line input awaited; `false` = char input.
    line: bool,
    /// Whether the request was the Unicode (`_uni`) variant.
    unicode: bool,
}

/// A suspended `glk_select` awaiting a host-supplied **non-input** event: the
/// game armed only a timer/mouse/hyperlink request (no line/char input) and is
/// blocked until one fires (see [`StepResult::NeedEvent`]). Mirrors
/// [`PendingInput`]; mutually exclusive with it (a `glk_select` sets exactly one).
struct PendingEvent {
    /// Glulx address of the `event_t` to fill when the event arrives.
    event_addr: u32,
    /// The armed timer interval in ms at suspend time, or `None`.
    timer_ms: Option<u32>,
    /// Whether any window had a pending mouse request at suspend time.
    mouse: bool,
    /// Whether any window had a pending hyperlink request at suspend time.
    hyperlink: bool,
}

/// A suspended game-initiated `@save`/`@restore` awaiting the host's file I/O.
/// The store destination is the opcode's S1 operand, resolved by the host-facing
/// completion methods once the file operation is done.
struct PendingSaveLoad {
    /// Where the `@save`/`@restore` result (0/1/-1) is written.
    dest: Dest,
    /// `false` = `@save` (SaveRequest); `true` = `@restore` (RestoreRequest).
    restore: bool,
    /// The (sanitized) fileref name the game opened the save stream on — the
    /// host's fixed file for a game-managed save. Empty if the operand stream
    /// wasn't a `SavedGame` file stream.
    name: String,
    /// `true` when the fileref was `create_by_prompt` (the player's SAVE/RESTORE
    /// verb): the host surfaces its save UI. `false` for the game's own
    /// fixed-name saves (CM's init cache, undo, autotesting): the host services
    /// them silently against `name`.
    by_prompt: bool,
    /// The game's save stream id (`@save`'s L1). Credited only on the success
    /// path (`complete_save(true)`) via `note_stream_write`, so a cancelled save
    /// leaves the library's write-count/existence checks reading failure
    /// (SQ-0301). Unused for `@restore` (0).
    save_stream: u32,
    /// The would-be save byte count (`save_quetzal().len()`), carried so the
    /// success path can credit it without recomputing. Unused for `@restore` (0).
    save_bytes: u32,
    /// `true` when this `@save` targets a brand-new SavedGame slot (no prior save
    /// recorded this session). A cancel of a fresh save clears its open-for-write
    /// existence marker so `does_file_exist` reads false (SQ-0301). Unused for
    /// `@restore` (false).
    fresh: bool,
}

/// What the host needs to service a suspended `@save`/`@restore`
/// ([`StepResult::SaveRequest`]/[`RestoreRequest`]): the game's target file
/// `name` and whether it is the player's prompted SAVE/RESTORE verb.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SaveLoadRequest {
    /// The (sanitized) fixed file name for a game-managed save (empty if unknown).
    pub name: String,
    /// `true` = the player's SAVE/RESTORE verb (surface the host UI); `false` =
    /// the game's own fixed-name save (service silently against `name`).
    pub by_prompt: bool,
    /// `true` = `@restore`; `false` = `@save`.
    pub restore: bool,
}

/// A suspended `glk_fileref_create_by_prompt` awaiting the host's chosen filename.
/// `dest` (the `@glk` store operand) is filled by `op_glk` right after the arm sets
/// this; `supply_filename` binds the fileref and stores its id there.
struct PendingFileref {
    dest: Dest,
    usage: u32,
    fmode: u32,
    rock: u32,
}

impl Dest {
    fn to_stub(self) -> (u32, u32) {
        match self {
            Dest::Discard => (0, 0),
            Dest::Mem(a) => (1, a),
            Dest::Local(a) => (2, a),
            Dest::Push => (3, 0),
        }
    }
}

/// The Glulx virtual machine.
pub struct Machine {
    pub(crate) mem: Memory,
    /// Byte-addressed stack (length = header stack size).
    stack: Vec<u8>,
    /// Stack pointer: number of bytes currently in use.
    sp: usize,
    /// Frame pointer: byte offset of the current call frame.
    fp: usize,
    /// Program counter (address in main memory).
    pub(crate) pc: u32,
    /// Current I/O system mode (0 null, 1 filter, 2 Glk).
    pub(crate) iosys_mode: u32,
    /// Current I/O system rock.
    pub(crate) iosys_rock: u32,
    /// Recursion depth of filter-iosys (mode 1) callbacks, guarding against a
    /// filter function whose own output recurses back into `emit`.
    filter_depth: u32,
    /// A fault raised inside a filter-iosys callback, latched because `emit`
    /// has no error channel (printing is fire-and-forget for the opcodes). The
    /// aborted callee's frame was abandoned mid-flight, so execution must not
    /// continue: `step`/`run_call_to_return` drain this and surface it as the
    /// current instruction's fault (SQ-0625).
    pending_fault: Option<String>,
    /// Recursion depth of echo-stream forwarding in `glk_stream_put`, bounding a
    /// pathological echo loop (windows echoing to each other's streams, which the
    /// Glk spec calls illegal) so output can never infinite-loop.
    echo_depth: u32,
    /// When `Some`, `emit` diverts all output into this buffer instead of routing
    /// it through the I/O system. Used to decode a compressed (E1) string object
    /// handed to `glk_put_string` into a `String` (capturing embedded string- and
    /// function-node output too) so it can then be written straight to the Glk
    /// stream, bypassing the iosys — a direct Glk call must ignore it. Transient
    /// (set and cleared within one `decode_glk_string`); never live across a save.
    emit_capture: Option<String>,
    /// Current string-decoding-table address (0 = none). Initialized from the
    /// header's decode_table; overridable by `setstringtbl`.
    pub(crate) cur_stringtbl: u32,
    /// Allocation-heap start address (0 = heap inactive). Set on the first
    /// `malloc` to the memsize at that moment.
    pub(crate) heap_start: u32,
    /// Extant allocated blocks `(addr, size)`, kept sorted by address.
    heap_blocks: Vec<(u32, u32)>,
    /// The Glk window/stream model (the output target for all printing).
    pub(crate) glk: Model,
    /// A suspended `glk_select`: the event address + which window/kind of input
    /// is awaited. Set when `glk_select` finds a pending request; cleared when
    /// the host supplies the event ([`Machine::supply_line`]/[`supply_char`]).
    pending_input: Option<PendingInput>,
    /// A suspended `glk_select` awaiting a non-input event (timer/mouse/hyperlink;
    /// see [`StepResult::NeedEvent`]). Set when `glk_select` finds no line/char
    /// request but a timer/mouse/hyperlink is armed; cleared when the host
    /// delivers the event ([`Machine::deliver_timer`] etc.). Mutually exclusive
    /// with `pending_input`.
    pending_event: Option<PendingEvent>,
    /// A suspended game `@save`/`@restore` (see [`StepResult::SaveRequest`]).
    /// Set when the opcode fires; consumed by the host-facing `complete_*` methods.
    pending_saveload: Option<PendingSaveLoad>,
    /// A suspended `glk_fileref_create_by_prompt` (see [`StepResult::NeedFilename`]).
    /// Set by the `0x0062` @glk arm; consumed by [`Machine::supply_filename`].
    pending_fileref: Option<PendingFileref>,
    /// The display backend the Glk model drives.
    pub(crate) backend: Box<dyn GlkBackend>,
    /// Recorded runtime faults / deferred-feature notices.
    pub diagnostics: Vec<String>,
    /// When true, every structural Glk call (windows, styles, streams, colours,
    /// garglk extensions — but not the high-volume put/get text I/O) is recorded
    /// to `screen_trace`. A debug aid for seeing exactly what a story instructs
    /// the interpreter to do.
    pub trace_screen: bool,
    /// Structural Glk/garglk call lines recorded while `trace_screen` is set,
    /// drained by the host each turn. Separate from `diagnostics`.
    pub screen_trace: Vec<String>,
    /// Pending coalesced story-text run for the `screen` trace: `(window, window
    /// type, text)`. Flushed as a `win N [buf|grid] <- "…"` line before the next
    /// structural call, so the trace shows WHERE story text is printed (which
    /// window and whether it's a buffer that accumulates or a grid). (SQ-0405)
    text_run: Option<(u32, &'static str, String)>,
    /// What the host's editable input line should hold for the latest line-input
    /// request: the `initlen` characters the game pre-loaded into the buffer (Glk
    /// spec §4.2 — "already in the input line"), or an EMPTY string when it
    /// pre-loaded nothing. One-shot per request.
    ///
    /// `Some("")` is meaningful, not a miss: a fresh request with an empty buffer
    /// means the previous line is gone — consumed or cancelled — so the host must
    /// clear its own line too. advent.blb's toolbar relies on both halves: a verb
    /// button primes "Examine ", and clicking a verb while a noun is typed runs the
    /// whole command itself and re-requests empty, which must not leave the noun
    /// stranded at the prompt.
    line_seed: Option<String>,
    /// Set once execution has ended (outer return or quit/fault).
    pub(crate) halted: bool,
    /// Protected RAM range `(addr, len)` preserved across restore/restoreundo;
    /// `len == 0` means no protection (set by the `protect` opcode).
    protect: (u32, u32),
    /// Bounded stack of undo snapshots (oldest first); see [`Machine::UNDO_CAP`].
    undo_stack: Vec<Vec<u8>>,
    /// Accelerated-function assignments: VM function address → accel number
    /// (`accelfunc`). Stored only; interception is deferred (see GLULX_NOTES §17).
    accel_funcs: std::collections::HashMap<u32, u32>,
    /// Acceleration parameter table: index → value (`accelparam`).
    accel_params: std::collections::HashMap<u32, u32>,
    /// Whether accelerated-function interception is active (default true).
    pub(crate) acceleration: bool,
    /// Whether Glk graphics windows are enabled (default false; hosts opt in).
    pub(crate) graphics_enabled: bool,
    /// Whether Glk sound channels are enabled (default false; hosts opt in).
    pub(crate) sound_enabled: bool,
    /// The current Glk timer interval in milliseconds, or `None` when timer
    /// events are off. Set by `glk_request_timer_events`; the host reads it via
    /// [`Machine::glk_timer_interval`] to arm its clock and calls
    /// [`Machine::deliver_timer`] on each tick.
    timer_interval_ms: Option<u32>,
    /// Total number of opcodes dispatched since the machine was built.
    pub(crate) insn_count: u64,
    /// PRNG state (xorshift32); seeded by `setrandom`.
    rng: u32,
    /// PC at the start of the instruction currently executing (captured before
    /// operand reads); used as the fault site if this instruction faults.
    instr_start_pc: u32,
    /// Stack trace captured when a fault converted to Quit. Host drains it.
    pub fault_trace: Option<crate::trace::StackTrace>,

    // Cached layout of the current frame (recomputed whenever `fp` changes).
    cur_frame_len: u32,
    cur_localspos: u32,
    /// `(offset_within_locals, size_bytes)` for each local of the current frame.
    cur_locals: Vec<(u32, u8)>,
    /// Reused argument buffer for `call`/`tailcall`'s popped args (argc is
    /// dynamic, so this can't be a fixed array like [`Operands`]). Taken with
    /// `mem::take` for the duration of one call setup and put back after, so a
    /// re-entrant user would merely allocate afresh, never alias (SQ-1208).
    call_args_scratch: Vec<u32>,

    /// When true, `step_once` records each instruction's start PC into
    /// `executed_pcs`/`ever_executed` (the debug inspector's execution coverage).
    /// Default false; guarded so it is a single predictable branch — and zero
    /// set-touching / allocation — on the hot path when off (SQ-0465).
    pub trace_exec: bool,
    /// Instruction-start PCs executed since the host last cleared them (per-turn
    /// coverage). The host drains/clears this each turn via `clear_executed_pcs`.
    pub executed_pcs: std::collections::HashSet<u32>,
    /// Cumulative instruction-start PCs ever executed while tracing was on —
    /// NEVER cleared per turn. Feeds the disassembler as dynamic discovery
    /// (executed bytes are ground-truth code) and can be pre-seeded from
    /// host-persisted coverage via [`Machine::seed_ever_executed`].
    pub ever_executed: std::collections::HashSet<u32>,
}

fn align_up(v: u32, to: u32) -> u32 {
    v.div_ceil(to) * to
}

/// Pre-allocation hint for a count read from the story image or VM memory:
/// clamp it so a hostile 32-bit count (e.g. `@call` with argc 0xFFFFFFFF)
/// cannot reserve gigabytes up front. Pushes past the hint simply grow the
/// collection, and the per-element pops/reads fault long before any genuine
/// multi-billion-entry collection could materialize. (SQ-0624)
fn cap_hint(n: u32) -> usize {
    (n as usize).min(1024)
}

/// Best-effort Glulx opcode mnemonic; hex fallback when unknown.
fn opcode_name(opcode: u32) -> String {
    let name = match opcode {
        0x30 => "call",
        0x31 => "return",
        0x40 => "copy",
        0x48 => "aload",
        0x4C => "astore",
        0x70 => "streamchar",
        0x160..=0x163 => "callf",
        _ => return format!("0x{opcode:x}"),
    };
    name.to_string()
}

/// `glk_char_to_lower`: Latin-1 lowercasing (ASCII A–Z and the Latin-1 uppercase
/// accented letters 0xC0–0xDE except 0xD7); other code points are unchanged.
fn glk_char_to_lower(c: u32) -> u32 {
    match c {
        0x41..=0x5A => c + 0x20,
        0xC0..=0xD6 | 0xD8..=0xDE => c + 0x20,
        _ => c,
    }
}

/// `glk_char_to_upper`: Latin-1 uppercasing (ASCII a–z and the Latin-1 lowercase
/// accented letters 0xE0–0xFE except 0xF7); other code points are unchanged.
fn glk_char_to_upper(c: u32) -> u32 {
    match c {
        0x61..=0x7A => c - 0x20,
        0xE0..=0xF6 | 0xF8..=0xFE => c - 0x20,
        _ => c,
    }
}

/// True for high-volume per-character Glk selectors — text I/O (put/get
/// char/string/buffer, Latin-1 and Unicode) plus the per-character case/normalize
/// helpers. The debug trace skips these so a story's structural window/style/
/// colour instructions aren't buried under per-character output.
fn is_glk_text_io(sel: u32) -> bool {
    matches!(sel,
        0x0080..=0x0085 // put_char / _stream / put_string / _stream / put_buffer / _stream
        | 0x0090..=0x0092 // get_char_stream / get_line_stream / get_buffer_stream
        | 0x00A0..=0x00A1 // char_to_lower / char_to_upper
        | 0x0120..=0x0124 // buffer_to_lower/upper/title_case_uni, canon_decompose/normalize_uni
        | 0x0128..=0x012D // put_char_uni / put_string_uni / put_buffer_uni / _stream
        | 0x0130..=0x0132 // get_char_stream_uni / get_buffer / get_line (uni)
    )
}

/// A human-readable name for a Glk / garglk selector, for the debug trace. The
/// mapping is taken from this interpreter's own `glk_dispatch` arms (the
/// authoritative source — not the Glk spec constants, which differ), so it stays
/// correct as the dispatch evolves. Unknown selectors fall back to hex.
pub(crate) fn glk_selector_name(sel: u32) -> String {
    let name = match sel {
        0x0001 => "glk_exit",
        0x0002 => "glk_tick",
        0x0004 => "glk_gestalt",
        0x0005 => "glk_gestalt_ext",
        0x0020 => "glk_window_iterate",
        0x0021 => "glk_window_get_rock",
        0x0022 => "glk_window_get_root",
        0x0023 => "glk_window_open",
        0x0024 => "glk_window_close",
        0x0025 => "glk_window_get_size",
        0x0026 => "glk_window_set_arrangement",
        0x0027 => "glk_window_get_arrangement",
        0x0028 => "glk_window_get_type",
        0x0029 => "glk_window_get_parent",
        0x002A => "glk_window_clear",
        0x002B => "glk_window_move_cursor",
        0x002C => "glk_window_get_stream",
        0x002D => "glk_window_set_echo_stream",
        0x002E => "glk_window_get_echo_stream",
        0x002F => "glk_set_window",
        0x0030 => "glk_window_get_sibling",
        0x0040 => "glk_stream_iterate",
        0x0041 => "glk_stream_get_rock",
        0x0042 => "glk_stream_open_file",
        0x0043 => "glk_stream_open_memory",
        0x0044 => "glk_stream_close",
        0x0045 => "glk_stream_set_position",
        0x0046 => "glk_stream_get_position",
        0x0047 => "glk_stream_set_current",
        0x0048 => "glk_stream_get_current",
        0x0049 => "glk_stream_open_resource",
        0x0060 => "glk_fileref_create_temp",
        0x0061 => "glk_fileref_create_by_name",
        0x0062 => "glk_fileref_create_by_prompt",
        0x0063 => "glk_fileref_destroy",
        0x0064 => "glk_fileref_iterate",
        0x0065 => "glk_fileref_get_rock",
        0x0066 => "glk_fileref_delete_file",
        0x0067 => "glk_fileref_does_file_exist",
        0x0068 => "glk_fileref_create_from_fileref",
        0x0080 => "glk_put_char",
        0x0081 => "glk_put_char_stream",
        0x0082 => "glk_put_string",
        0x0083 => "glk_put_string_stream",
        0x0084 => "glk_put_buffer",
        0x0085 => "glk_put_buffer_stream",
        0x0086 => "glk_set_style",
        0x0087 => "glk_set_style_stream",
        0x0090 => "glk_get_char_stream",
        0x0091 => "glk_get_line_stream",
        0x0092 => "glk_get_buffer_stream",
        0x00A0 => "glk_char_to_lower",
        0x00A1 => "glk_char_to_upper",
        0x00B0 => "glk_stylehint_set",
        0x00B1 => "glk_stylehint_clear",
        0x00B2 => "glk_style_distinguish",
        0x00B3 => "glk_style_measure",
        0x00C0 => "glk_select",
        0x00C1 => "glk_select_poll",
        0x00D0 => "glk_request_line_event",
        0x00D1 => "glk_cancel_line_event",
        0x00D2 => "glk_request_char_event",
        0x00D3 => "glk_cancel_char_event",
        0x00D4 => "glk_request_mouse_event",
        0x00D5 => "glk_cancel_mouse_event",
        0x00D6 => "glk_request_timer_events",
        0x00E0 => "glk_image_get_info",
        0x00E1 => "glk_image_draw",
        0x00E2 => "glk_image_draw_scaled",
        0x00E8 => "glk_window_flow_break",
        0x00E9 => "glk_window_erase_rect",
        0x00EA => "glk_window_fill_rect",
        0x00EB => "glk_window_set_background_color",
        0x00F0 => "glk_schannel_iterate",
        0x00F1 => "glk_schannel_get_rock",
        0x00F2 => "glk_schannel_create",
        0x00F3 => "glk_schannel_destroy",
        0x00F4 => "glk_schannel_create_ext",
        0x00F7 => "glk_schannel_play_multi",
        0x00F8 => "glk_schannel_play",
        0x00F9 => "glk_schannel_play_ext",
        0x00FA => "glk_schannel_stop",
        0x00FB => "glk_schannel_set_volume",
        0x00FC => "glk_sound_load_hint",
        0x00FD => "glk_schannel_set_volume_ext",
        0x00FE => "glk_schannel_pause",
        0x00FF => "glk_schannel_unpause",
        0x0100 => "glk_set_hyperlink",
        0x0101 => "glk_set_hyperlink_stream",
        0x0102 => "glk_request_hyperlink_event",
        0x0103 => "glk_cancel_hyperlink_event",
        0x0120 => "glk_buffer_to_lower_case_uni",
        0x0121 => "glk_buffer_to_upper_case_uni",
        0x0122 => "glk_buffer_to_title_case_uni",
        0x0123 => "glk_buffer_canon_decompose_uni",
        0x0124 => "glk_buffer_canon_normalize_uni",
        0x0128 => "glk_put_char_uni",
        0x0129 => "glk_put_string_uni",
        0x012A => "glk_put_buffer_uni",
        0x012B => "glk_put_char_stream_uni",
        0x012C => "glk_put_string_stream_uni",
        0x012D => "glk_put_buffer_stream_uni",
        0x0130 => "glk_get_char_stream_uni",
        0x0131 => "glk_get_buffer_stream_uni",
        0x0132 => "glk_get_line_stream_uni",
        0x0138 => "glk_stream_open_file_uni",
        0x0139 => "glk_stream_open_memory_uni",
        0x013A => "glk_stream_open_resource_uni",
        0x0140 => "glk_request_char_event_uni",
        0x0141 => "glk_request_line_event_uni",
        0x0150 => "glk_set_echo_line_event",
        0x0151 => "glk_set_terminators_line_event",
        0x0160 => "glk_current_time",
        0x0161 => "glk_current_simple_time",
        0x0168 => "glk_time_to_date_utc",
        0x0169 => "glk_time_to_date_local",
        0x016A => "glk_simple_time_to_date_utc",
        0x016B => "glk_simple_time_to_date_local",
        0x016C => "glk_date_to_time_utc",
        0x016D => "glk_date_to_time_local",
        0x016E => "glk_date_to_simple_time_utc",
        0x016F => "glk_date_to_simple_time_local",
        0x1100 => "garglk_set_zcolors",
        0x1101 => "garglk_set_zcolors_stream",
        0x1102 => "garglk_set_reversevideo",
        0x1103 => "garglk_set_reversevideo_stream",
        _ => return format!("glk[{sel:#06x}]"),
    };
    name.to_string()
}

/// Glk `wintype_*` name, for the trace.
fn glk_wintype_name(v: u32) -> &'static str {
    match v { 0 => "AllTypes", 1 => "Pair", 2 => "Blank", 3 => "TextBuffer", 4 => "TextGrid", 5 => "Graphics", _ => "?" }
}

/// Glk `style_*` name (0..=10), for the trace.
fn glk_style_name(v: u32) -> &'static str {
    match v {
        0 => "Normal", 1 => "Emphasized", 2 => "Preformatted", 3 => "Header", 4 => "Subheader",
        5 => "Alert", 6 => "Note", 7 => "BlockQuote", 8 => "Input", 9 => "User1", 10 => "User2", _ => "?",
    }
}

/// Glk `stylehint_*` name, for the trace.
fn glk_hint_name(v: u32) -> &'static str {
    match v {
        0 => "Indentation", 1 => "ParaIndentation", 2 => "Justification", 3 => "Size", 4 => "Weight",
        5 => "Oblique", 6 => "Proportional", 7 => "TextColor", 8 => "BackColor", 9 => "ReverseColor", _ => "?",
    }
}

/// Format a Glk/garglk colour value: the garglk sentinels by name, else `#RRGGBB`.
fn glk_color_hex(v: u32) -> String {
    match v {
        0xffff_ffff => "default".to_string(),
        0xffff_fffe => "current".to_string(),
        0xffff_fffd => "cursor".to_string(),
        0xffff_fffc => "transparent".to_string(),
        rgb => format!("#{:06X}", rgb & 0x00FF_FFFF),
    }
}

/// Format a Glk call's arguments for the trace, decoding the colour/style calls
/// (stylehints, window/graphics background, garglk zcolors, set_style) into
/// readable names + `#RRGGBB`; everything else prints its raw args. (SQ-0403)
fn glk_trace_args(sel: u32, args: &[u32]) -> String {
    let a = |i: usize| args.get(i).copied().unwrap_or(0);
    match sel {
        0x00B0 => {
            // glk_stylehint_set(wintype, style, hint, value)
            let hint = a(2);
            let val = if hint == 7 || hint == 8 { glk_color_hex(a(3)) } else { a(3).to_string() };
            format!("{}, {}, {}, {val}", glk_wintype_name(a(0)), glk_style_name(a(1)), glk_hint_name(hint))
        }
        0x00B1 => format!("{}, {}, {}", glk_wintype_name(a(0)), glk_style_name(a(1)), glk_hint_name(a(2))),
        0x0086 => glk_style_name(a(0)).to_string(),                         // glk_set_style(style)
        0x0087 => format!("str={}, {}", a(0), glk_style_name(a(1))),        // glk_set_style_stream
        0x00EB => format!("win={}, {}", a(0), glk_color_hex(a(1))),         // window_set_background_color(win, color)
        0x00EA => format!("win={}, {}, l={}, t={}, w={}, h={}", a(0), glk_color_hex(a(1)), a(2), a(3), a(4), a(5)), // fill_rect
        0x1100 => format!("fg={}, bg={}", glk_color_hex(a(0)), glk_color_hex(a(1))),           // garglk_set_zcolors(fg, bg)
        0x1101 => format!("str={}, fg={}, bg={}", a(0), glk_color_hex(a(1)), glk_color_hex(a(2))), // _stream
        0x0004 => format!("{}, {}", glk_gestalt_name(a(0)), a(1)),          // glk_gestalt(sel, val)
        0x0005 => format!("{}, {}", glk_gestalt_name(a(0)), a(1)),          // glk_gestalt_ext(sel, val, ...)
        // The `glk_select` arg is the event_t out-pointer (where the result is
        // written); the decoded event follows on the next `event -> …` line. (SQ-0405)
        0x00C0 | 0x00C1 => format!("event=0x{:X}", a(0)),                   // glk_select[_poll](&event)
        0x00D0 | 0x0141 => format!("win={}, buf=0x{:X}, maxlen={}, initlen={}", a(0), a(1), a(2), a(3)), // request_line_event[_uni]
        0x00D2 | 0x0140 => format!("win={}", a(0)),                         // request_char_event[_uni]
        _ => args.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "),
    }
}

/// A human-readable result suffix for the value-returning Glk selectors, so the
/// trace can show `call(...) -> <result>` on one line. `None` for void returns.
/// (SQ-0405)
fn glk_trace_ret(sel: u32, ret: u32) -> Option<String> {
    let s = match sel {
        0x0020 | 0x0022 | 0x0023 | 0x0029 | 0x0030 => format!("win {ret}"), // window open/root/parent/sibling/iterate
        0x0028 => glk_wintype_name(ret).to_string(),                        // window_get_type
        0x002C | 0x002E | 0x0040 | 0x0042 | 0x0043 | 0x0048 | 0x0049 | 0x0138 | 0x0139 | 0x013A => format!("stream {ret}"),
        0x0060 | 0x0061 | 0x0062 | 0x0064 | 0x0068 => format!("fileref {ret}"),
        0x0067 => (if ret != 0 { "exists" } else { "absent" }).to_string(), // fileref_does_file_exist
        0x0004 | 0x0005 => ret.to_string(),                                 // gestalt value
        0x00F2 | 0x00F4 => format!("channel {ret}"),                        // schannel_create[_ext]
        0x00E1 | 0x00E2 => (if ret != 0 { "drawn" } else { "no" }).to_string(), // image_draw[_scaled]
        _ => return None,
    };
    Some(s)
}

/// Glk `evtype_*` name, for the event trace. (SQ-0405)
fn glk_evtype_name(t: u32) -> &'static str {
    match t {
        0 => "None", 1 => "Timer", 2 => "CharInput", 3 => "LineInput", 4 => "MouseInput",
        5 => "Arrange", 6 => "Redraw", 7 => "SoundNotify", 8 => "Hyperlink", 9 => "VolumeNotify",
        _ => "Event?",
    }
}

/// Decode a Glk character-event `val1` — a special `keycode_*` name, a printable
/// char, or a hex code point. (SQ-0405)
fn glk_keycode_name(k: u32) -> String {
    use glk::keycode::*;
    match k {
        UNKNOWN => "Unknown".into(),
        LEFT => "Left".into(),
        RIGHT => "Right".into(),
        UP => "Up".into(),
        DOWN => "Down".into(),
        RETURN => "Return".into(),
        DELETE => "Delete".into(),
        ESCAPE => "Escape".into(),
        TAB => "Tab".into(),
        PAGE_UP => "PageUp".into(),
        PAGE_DOWN => "PageDown".into(),
        HOME => "Home".into(),
        END => "End".into(),
        // Function keys occupy FUNC12..=FUNC1 (keycode_FuncN = FUNC1 - (N-1)).
        f if (FUNC12..=FUNC1).contains(&f) => format!("Func{}", FUNC1 - f + 1),
        c if c <= 0x0010_FFFF => {
            char::from_u32(c).map(|ch| format!("'{ch}'")).unwrap_or_else(|| format!("{c:#x}"))
        }
        _ => format!("{k:#010x}"),
    }
}

/// One-line description of a delivered Glk event, decoding the type + payload
/// (keycode for char input, length for line input). (SQ-0405)
fn glk_event_desc(ev: &GlkEvent) -> String {
    let name = glk_evtype_name(ev.etype);
    match ev.etype {
        glk::evtype::NONE => name.to_string(),
        glk::evtype::LINE_INPUT => format!("{name} win={} len={}", ev.win, ev.val1),
        glk::evtype::CHAR_INPUT => format!("{name} win={} key={}", ev.win, glk_keycode_name(ev.val1)),
        glk::evtype::MOUSE_INPUT => format!("{name} win={} x={} y={}", ev.win, ev.val1, ev.val2),
        glk::evtype::HYPERLINK => format!("{name} win={} link={}", ev.win, ev.val1),
        _ => format!("{name} win={} val1={} val2={}", ev.win, ev.val1, ev.val2),
    }
}

/// A human-readable name for a Glk `gestalt_*` selector. (SQ-0405)
fn glk_gestalt_name(sel: u32) -> String {
    let name = match sel {
        0 => "Version", 1 => "CharInput", 2 => "LineInput", 3 => "CharOutput",
        4 => "MouseInput", 5 => "Timer", 6 => "Graphics", 7 => "DrawImage", 8 => "Sound",
        9 => "SoundVolume", 10 => "SoundNotify", 11 => "Hyperlinks", 12 => "HyperlinkInput",
        13 => "SoundMusic", 14 => "GraphicsTransparency", 15 => "Unicode", 16 => "UnicodeNorm",
        17 => "LineInputEcho", 18 => "LineTerminators", 19 => "LineTerminatorKey", 20 => "DateTime",
        21 => "Sound2", 22 => "ResourceStream", 23 => "GraphicsCharInput",
        0x1100 => "GarglkText",
        _ => return format!("gestalt[{sel:#x}]"),
    };
    name.to_string()
}

/// Clamp a code point for a byte-oriented stream read: values above 0xFF are
/// returned as '?' (0x3F), everything else unchanged (Glk spec §5.4).
fn clamp_byte(cp: u32) -> u32 {
    if cp > 0xFF {
        0x3F
    } else {
        cp
    }
}

/// Append one IFF chunk (`id`, 4-byte big-endian length, data, even-pad).
fn push_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0); // IFF chunks are padded to an even length
    }
}

/// Parsed IFZS chunks: each is `(4-byte id, chunk data slice)`.
type IfzsChunks<'a> = Vec<([u8; 4], &'a [u8])>;

/// Parse a `FORM IFZS` container into its `(id, data)` chunks. Never panics;
/// returns a [`GError::BadSave`] on any structural problem.
fn parse_ifzs<'a>(data: &'a [u8]) -> Result<IfzsChunks<'a>, GError> {
    let bad = |m: &str| GError::BadSave(m.to_string());
    if data.len() < 12 || &data[0..4] != b"FORM" || &data[8..12] != b"IFZS" {
        return Err(bad("not a FORM IFZS container"));
    }
    let form_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let end = (8 + form_len).min(data.len());
    let mut chunks = Vec::new();
    let mut p = 12;
    while p + 8 <= end {
        let id = [data[p], data[p + 1], data[p + 2], data[p + 3]];
        let len = u32::from_be_bytes([data[p + 4], data[p + 5], data[p + 6], data[p + 7]]) as usize;
        let start = p + 8;
        if start + len > data.len() {
            return Err(bad("chunk length overruns the data"));
        }
        chunks.push((id, &data[start..start + len]));
        p = start + len + (len & 1); // skip the even-pad byte
    }
    Ok(chunks)
}

impl Machine {
    /// Ceiling on the memory-map size; `malloc` fails rather than grow past
    /// it, and `parse_header` rejects a story whose ENDMEM starts above it.
    const MAX_MEMSIZE: u32 = crate::header::MAX_MEMSIZE; // 256 MiB

    /// Maximum number of in-memory undo snapshots retained (oldest dropped).
    const UNDO_CAP: usize = 16;

    /// The PRNG seed a bare `Machine` starts from: fixed, so a machine nobody
    /// seeds replays one sequence (every unit test relies on that). A HOST that
    /// wants a fresh game per launch calls [`Machine::set_rng_seed`] before the
    /// boot drive — lanthorn seeds from the `random_seed` config key, or from
    /// entropy when that key is unset (SQ-0811). Also the nonzero fallback for
    /// [`Machine::entropy_seed`] and for a state that reached 0.
    pub const DEFAULT_SEED: u32 = 0x2BAD_C0DE;

    /// Build a machine over `mem`, entering the start function (no arguments).
    /// Text output flows through the Glk model to `backend`.
    pub fn with_glk(mem: Memory, backend: Box<dyn GlkBackend>) -> Machine {
        let stack_len = (mem.stack_size().max(0x100)) as usize;
        let start = mem.start_func();
        let decode_table = mem.decode_table();
        let mut m = Machine {
            mem,
            stack: vec![0u8; stack_len],
            sp: 0,
            fp: 0,
            pc: 0,
            iosys_mode: 0,
            iosys_rock: 0,
            filter_depth: 0,
            pending_fault: None,
            echo_depth: 0,
            emit_capture: None,
            cur_stringtbl: decode_table,
            heap_start: 0,
            heap_blocks: Vec::new(),
            glk: Model::new(),
            pending_input: None,
            pending_event: None,
            pending_saveload: None,
            pending_fileref: None,
            backend,
            diagnostics: Vec::new(),
            trace_screen: false,
            screen_trace: Vec::new(),
            text_run: None,
            line_seed: None,
            halted: false,
            protect: (0, 0),
            undo_stack: Vec::new(),
            accel_funcs: std::collections::HashMap::new(),
            accel_params: std::collections::HashMap::new(),
            acceleration: true,
            graphics_enabled: false,
            sound_enabled: false,
            timer_interval_ms: None,
            insn_count: 0,
            rng: Self::DEFAULT_SEED,
            instr_start_pc: 0,
            fault_trace: None,
            cur_frame_len: 0,
            cur_localspos: 0,
            cur_locals: Vec::new(),
            call_args_scratch: Vec::new(),
            trace_exec: false,
            executed_pcs: std::collections::HashSet::new(),
            ever_executed: std::collections::HashSet::new(),
        };
        // Enter the start function directly (no call stub beneath it; its fp is 0).
        if let Err(msg) = m.build_frame_and_enter(start, &[]) {
            m.diagnostics.push(msg);
            m.halted = true;
        }
        m
    }

    // ── main-memory read helpers (fault on out-of-range) ──────────────────────

    pub(crate) fn m8(&self, a: u32) -> R<u32> {
        self.mem.read8(a).ok_or_else(|| format!("memory fault: read8 @{a:#010x}"))
    }
    pub(crate) fn m16(&self, a: u32) -> R<u32> {
        self.mem.read16(a).ok_or_else(|| format!("memory fault: read16 @{a:#010x}"))
    }
    pub(crate) fn m32(&self, a: u32) -> R<u32> {
        self.mem.read32(a).ok_or_else(|| format!("memory fault: read32 @{a:#010x}"))
    }

    // ── instruction-stream readers (advance pc) ───────────────────────────────

    fn take8(&mut self) -> R<u32> {
        let v = self.m8(self.pc)?;
        self.pc += 1;
        Ok(v)
    }
    fn take16(&mut self) -> R<u32> {
        let v = self.m16(self.pc)?;
        self.pc += 2;
        Ok(v)
    }
    fn take32(&mut self) -> R<u32> {
        let v = self.m32(self.pc)?;
        self.pc += 4;
        Ok(v)
    }

    // ── opcode + operand decode (GLULX_NOTES §3) ──────────────────────────────

    /// Decode the variable-length opcode number at `pc`, advancing past it.
    /// Delegates the layout to the shared [`crate::decode::decode_opcode`] so the
    /// interpreter and disassembler decode opcode numbers identically.
    #[inline]
    pub(crate) fn decode_opcode(&mut self) -> R<u32> {
        let (opcode, next) = crate::decode::decode_opcode(&self.mem, self.pc)?;
        self.pc = next;
        Ok(opcode)
    }

    /// Decode `n_load` load values then `n_store` store destinations from the
    /// packed mode nibbles + operand data following `pc`. Everything decoded
    /// lives on the stack — this runs once per instruction, and heap-allocating
    /// here made the allocator the single largest line in a VM profile
    /// (SQ-1208).
    pub(crate) fn read_operands(&mut self, n_load: usize, n_store: usize) -> R<(Operands<u32>, Operands<Dest>)> {
        let total = n_load + n_store;
        debug_assert!(total <= MAX_OPERANDS, "operand count {total} outgrew MAX_OPERANDS");
        let mode_bytes = total.div_ceil(2);
        // Slurp the mode nibbles first; operand data follows them, in order.
        // (+1: an odd total still unpacks its final byte's padding high nibble,
        // which `take(total)` below then ignores, exactly as the spec allows.)
        let mut modes = [0u8; MAX_OPERANDS + 1];
        let start = self.pc;
        for i in 0..mode_bytes {
            let byte = self.m8(start + i as u32)?;
            modes[2 * i] = (byte & 0x0F) as u8;
            modes[2 * i + 1] = (byte >> 4) as u8;
        }
        self.pc += mode_bytes as u32;

        let mut loads = Operands::new(0u32);
        let mut stores = Operands::new(Dest::Discard);
        for (i, &mode) in modes.iter().enumerate().take(total) {
            if i < n_load {
                loads.push(self.resolve_load(mode)?);
            } else {
                stores.push(self.resolve_store(mode)?);
            }
        }
        Ok((loads, stores))
    }

    /// Resolve one LOAD operand of the given addressing mode to its value.
    fn resolve_load(&mut self, mode: u8) -> R<u32> {
        let ramstart = self.mem.ramstart();
        Ok(match mode {
            0x0 => 0,
            0x1 => self.take8()? as u8 as i8 as i32 as u32, // sign-extend
            0x2 => self.take16()? as u16 as i16 as i32 as u32,
            0x3 => self.take32()?,
            0x5 => {
                let a = self.take8()?;
                self.m32(a)?
            }
            0x6 => {
                let a = self.take16()?;
                self.m32(a)?
            }
            0x7 => {
                let a = self.take32()?;
                self.m32(a)?
            }
            0x8 => self.pop32()?,
            0x9 => {
                let o = self.take8()?;
                self.local_load(o)?
            }
            0xA => {
                let o = self.take16()?;
                self.local_load(o)?
            }
            0xB => {
                let o = self.take32()?;
                self.local_load(o)?
            }
            // RAM-relative modes wrap (32-bit address arithmetic, as glulxe's C
            // uint32 sum does): a huge operand must reach the normal memory
            // bounds check, not a debug overflow panic (SQ-0627).
            0xD => {
                let a = self.take8()?.wrapping_add(ramstart);
                self.m32(a)?
            }
            0xE => {
                let a = self.take16()?.wrapping_add(ramstart);
                self.m32(a)?
            }
            0xF => {
                let a = self.take32()?.wrapping_add(ramstart);
                self.m32(a)?
            }
            other => return Err(format!("illegal load operand mode {other:#x}")),
        })
    }

    /// Resolve one STORE operand of the given addressing mode to a destination.
    fn resolve_store(&mut self, mode: u8) -> R<Dest> {
        let ramstart = self.mem.ramstart();
        Ok(match mode {
            0x0 => Dest::Discard,
            0x8 => Dest::Push,
            0x5 => Dest::Mem(self.take8()?),
            0x6 => Dest::Mem(self.take16()?),
            0x7 => Dest::Mem(self.take32()?),
            0x9 => Dest::Local(self.take8()?),
            0xA => Dest::Local(self.take16()?),
            0xB => Dest::Local(self.take32()?),
            // Wrapping for the same reason as resolve_load's 0xD..0xF arms.
            0xD => Dest::Mem(self.take8()?.wrapping_add(ramstart)),
            0xE => Dest::Mem(self.take16()?.wrapping_add(ramstart)),
            0xF => Dest::Mem(self.take32()?.wrapping_add(ramstart)),
            other => return Err(format!("illegal store operand mode {other:#x}")),
        })
    }

    /// Write `v` to a resolved destination.
    pub(crate) fn store(&mut self, dest: Dest, v: u32) -> R<()> {
        match dest {
            Dest::Discard => Ok(()),
            Dest::Push => self.push32(v),
            Dest::Mem(a) => self.store_mem(a, v),
            Dest::Local(a) => self.local_store(a, v),
        }
    }

    // ── single-instruction execution ──────────────────────────────────────────

    /// Decode and execute one instruction. Grown task-by-task; `step()`/`run()`
    /// (the public loop with fault handling) wrap this in Task 6.
    pub(crate) fn step_once(&mut self) -> R<()> {
        self.insn_count += 1;
        self.instr_start_pc = self.pc;
        // Debug execution-coverage trace. Off by default: a single predictable
        // branch with no set access / allocation on the hot path (SQ-0465).
        if self.trace_exec {
            self.executed_pcs.insert(self.pc);
            self.ever_executed.insert(self.pc);
        }
        let opcode = self.decode_opcode()?;
        self.execute(opcode)
    }

    /// Clear the per-turn `executed_pcs` set (call at the start/end of each turn).
    /// Leaves the cumulative `ever_executed` intact.
    pub fn clear_executed_pcs(&mut self) {
        self.executed_pcs.clear();
    }

    /// Pre-seed the cumulative `ever_executed` set from host-persisted coverage
    /// (the debug PC-set sidecar) so prior runs' disassembly tiers light up
    /// immediately. Independent of `trace_exec`.
    pub fn seed_ever_executed(&mut self, pcs: &std::collections::HashSet<u32>) {
        self.ever_executed.extend(pcs.iter().copied());
    }

    /// Dispatch a decoded opcode to its handler.
    fn execute(&mut self, opcode: u32) -> R<()> {
        match opcode {
            // Control / I/O system.
            0x00 => Ok(()), // nop
            0x120 => {
                self.halted = true;
                Ok(())
            }
            0x148 => {
                let (_, s) = self.read_operands(0, 2)?;
                let (mode, rock) = (self.iosys_mode, self.iosys_rock);
                self.store(s[0], mode)?;
                self.store(s[1], rock)
            }
            0x149 => {
                let (l, _) = self.read_operands(2, 0)?;
                self.iosys_mode = l[0];
                self.iosys_rock = l[1];
                Ok(())
            }
            // Stream output.
            0x70 => self.op_streamchar(),
            0x71 => self.op_streamnum(),
            0x72 => self.op_streamstr(),
            0x73 => self.op_streamunichar(),
            0x130 => self.op_glk(),
            // String-decoding table (get/set the current table address).
            0x140 => {
                let (_, s) = self.read_operands(0, 1)?;
                let v = self.cur_stringtbl;
                self.store(s[0], v)
            }
            0x141 => {
                let (l, _) = self.read_operands(1, 0)?;
                self.cur_stringtbl = l[0];
                Ok(())
            }
            // Arithmetic (2 load, 1 store) — 32-bit two's complement.
            0x10 => self.binop(|a, b| a.wrapping_add(b)),
            0x11 => self.binop(|a, b| a.wrapping_sub(b)),
            0x12 => self.binop(|a, b| a.wrapping_mul(b)),
            0x13 => self.divop(false),
            0x14 => self.divop(true),
            0x15 => self.unop(|a| (a as i32).wrapping_neg() as u32),
            // Bitwise.
            0x18 => self.binop(|a, b| a & b),
            0x19 => self.binop(|a, b| a | b),
            0x1A => self.binop(|a, b| a ^ b),
            0x1B => self.unop(|a| !a),
            0x1C => self.binop(|a, b| a.checked_shl(b).unwrap_or(0)),
            0x1D => self.binop(|a, b| (a as i32).checked_shr(b).unwrap_or((a as i32) >> 31) as u32),
            0x1E => self.binop(|a, b| a.checked_shr(b).unwrap_or(0)),
            // Branches.
            0x20 => {
                let (l, _) = self.read_operands(1, 0)?;
                self.branch(l[0], true)
            }
            0x104 => {
                // jumpabs L1 — jump to the absolute address L1 (no offset bias).
                let (l, _) = self.read_operands(1, 0)?;
                self.pc = l[0];
                Ok(())
            }
            0x22 => self.branch1(|v| v == 0),
            0x23 => self.branch1(|v| v != 0),
            0x24 => self.branch2(|a, b| a == b),
            0x25 => self.branch2(|a, b| a != b),
            0x26 => self.branch2(|a, b| (a as i32) < (b as i32)),
            0x27 => self.branch2(|a, b| (a as i32) >= (b as i32)),
            0x28 => self.branch2(|a, b| (a as i32) > (b as i32)),
            0x29 => self.branch2(|a, b| (a as i32) <= (b as i32)),
            0x2A => self.branch2(|a, b| a < b),
            0x2B => self.branch2(|a, b| a >= b),
            0x2C => self.branch2(|a, b| a > b),
            0x2D => self.branch2(|a, b| a <= b),
            // Memory-array load/store (L2 is a signed index).
            0x48 => self.op_aload(4),
            0x49 => self.op_aload(2),
            0x4A => self.op_aload(1),
            0x4B => self.op_aloadbit(),
            0x4C => self.op_astore(4),
            0x4D => self.op_astore(2),
            0x4E => self.op_astore(1),
            0x4F => self.op_astorebit(),
            // Block zero / copy.
            0x170 => self.op_mzero(),
            0x171 => self.op_mcopy(),
            // Search opcodes.
            0x150 => self.op_linearsearch(),
            0x151 => self.op_binarysearch(),
            0x152 => self.op_linkedsearch(),
            // Capability query / image verify.
            0x100 => {
                let (l, s) = self.read_operands(2, 1)?;
                let v = self.gestalt(l[0], l[1]);
                self.store(s[0], v)
            }
            0x121 => {
                let (_, s) = self.read_operands(0, 1)?;
                let v = u32::from(!self.mem.checksum_ok()); // 0 = good, 1 = problem
                self.store(s[0], v)
            }
            // PRNG.
            0x110 => {
                let (l, s) = self.read_operands(1, 1)?;
                let v = self.rand_range(l[0]);
                self.store(s[0], v)
            }
            0x111 => {
                let (l, _) = self.read_operands(1, 0)?;
                // setrandom(0) seeds from a genuinely unpredictable source; a nonzero
                // argument seeds deterministically (reproducible runs / testing).
                self.rng = if l[0] == 0 { Self::entropy_seed() } else { l[0] };
                Ok(())
            }
            // Acceleration (storage only; interception deferred — GLULX_NOTES §17).
            0x180 => {
                let (l, _) = self.read_operands(2, 0)?;
                let (funcnum, addr) = (l[0], l[1]);
                if funcnum == 0 {
                    self.accel_funcs.remove(&addr);
                } else {
                    self.accel_funcs.insert(addr, funcnum);
                }
                Ok(())
            }
            0x181 => {
                let (l, _) = self.read_operands(2, 0)?;
                self.accel_params.insert(l[0], l[1]);
                Ok(())
            }
            // Undo (in-memory; @save/@restore stream opcodes are sub-project 3).
            0x125 => self.op_saveundo(),
            0x126 => self.op_restoreundo(),
            // ExtUndo (Glulx 3.1.3): query / discard the temporary undo state.
            0x128 => {
                // hasundo — S1 = 0 if a restoreundo would succeed (a state is saved), else 1.
                let (_, s) = self.read_operands(0, 1)?;
                self.store(s[0], u32::from(self.undo_stack.is_empty()))
            }
            // discardundo — drop the most recent saved state; a no-op if none. Zero operands.
            0x129 => {
                self.undo_stack.pop();
                Ok(())
            }
            // Protect a RAM range across restore/restoreundo (L2 == 0 clears).
            0x127 => {
                let (l, _) = self.read_operands(2, 0)?;
                self.protect = (l[0], l[1]);
                Ok(())
            }
            // Copy / sign-extend.
            0x40 => self.unop(|a| a),                                  // copy
            0x41 => self.copy_sized(2),                                // copys
            0x42 => self.copy_sized(1),                                // copyb
            0x44 => self.unop(|a| a as u16 as i16 as i32 as u32),      // sexs
            0x45 => self.unop(|a| a as u8 as i8 as i32 as u32),        // sexb
            // Stack manipulation.
            0x50 => {
                let (_, s) = self.read_operands(0, 1)?;
                let c = self.value_count();
                self.store(s[0], c)
            }
            0x51 => self.op_stkpeek(),
            0x52 => self.op_stkswap(),
            0x53 => self.op_stkroll(),
            0x54 => self.op_stkcopy(),
            // Memory size.
            0x102 => {
                let (_, s) = self.read_operands(0, 1)?;
                let v = self.mem.mem_size();
                self.store(s[0], v)
            }
            0x103 => {
                let (l, s) = self.read_operands(1, 1)?;
                let r = if self.heap_start != 0 {
                    self.diagnostics
                        .push("setmemsize while the heap is active is illegal".to_string());
                    1
                } else {
                    u32::from(!self.mem.set_mem_size(l[0]))
                };
                self.store(s[0], r)
            }
            // Allocation heap.
            0x178 => {
                let (l, s) = self.read_operands(1, 1)?;
                let a = self.heap_malloc(l[0]);
                self.store(s[0], a)
            }
            0x179 => {
                let (l, _) = self.read_operands(1, 0)?;
                self.heap_free(l[0])
            }
            // Catch / throw (exception-style stack unwinding).
            0x32 => self.op_catch(),
            0x33 => self.op_throw(),
            // Calls / return.
            0x30 => self.op_call(),
            0x31 => {
                let (l, _) = self.read_operands(1, 0)?;
                self.return_value(l[0])
            }
            0x34 => self.op_tailcall(),
            0x160 => self.op_callf(0),
            0x161 => self.op_callf(1),
            0x162 => self.op_callf(2),
            0x163 => self.op_callf(3),
            0x0101 => {
                // debugtrap L1 — log the value and continue.
                let (l, _) = self.read_operands(1, 0)?;
                self.diagnostics.push(format!("debugtrap: {:#x}", l[0]));
                Ok(())
            }
            0x0122 => self.op_restart(),
            0x0123 => {
                // save L1 S1 — suspend for the host to write a standard
                // Glulx-Quetzal save (spec §1.3.2, §1.8.2). Push a call stub for
                // S1 so PC/FramePtr self-describe the resume point instead of
                // baking a value in now; complete_save() pops the stub and
                // stores the current-run result (0 success / 1 failure). A
                // restore of the saved blob instead pops the stub storing -1
                // (the "just restored" sentinel, spec §2.9) — see restore_quetzal.
                let (l, s) = self.read_operands(1, 1)?;
                let (dtype, daddr) = s[0].to_stub();
                let ret_pc = self.pc;
                let cur_fp = self.fp as u32;
                self.push32(dtype)?;
                self.push32(daddr)?;
                self.push32(ret_pc)?;
                self.push32(cur_fp)?;
                // Carry the target fileref's name + prompt-ness (L1 is the save
                // stream) so the host can service a game-managed save silently or
                // surface the player's SAVE-verb UI (see SaveLoadRequest).
                let (name, by_prompt) = self.glk.stream_saveload_info(l[0]).unwrap_or_default();
                // Whether this targets a brand-new slot (no prior save recorded)
                // so a cancel can revert it to "absent" (SQ-0301).
                let fresh = self.glk.saved_game_size(&name).unwrap_or(0) == 0;
                // The bytes the game's save stream would receive if delivery
                // weren't host-intercepted (spec §1.8.2's save goes to a host
                // file, not this Glk stream). Carried, NOT credited yet: crediting
                // eagerly would make a cancelled save's write-count/existence
                // checks report success (SQ-0301). complete_save(true) credits it.
                let n = self.save_quetzal().len() as u32;
                self.pending_saveload =
                    Some(PendingSaveLoad { dest: s[0], restore: false, name, by_prompt, save_stream: l[0], save_bytes: n, fresh });
                Ok(())
            }
            0x0124 => {
                // restore L1 S1 — suspend for the host to read a save file. On
                // success execution resumes inside the original @save's call
                // stub (which stores -1, the "just restored" sentinel); on
                // failure complete_restore_failure() stores 1 here.
                let (l, s) = self.read_operands(1, 1)?;
                // A stream whose bytes are already inside the VM — a Blorb resource
                // stream — restores IN PROCESS. The spec restores from the stream it
                // is handed (§1.8.2); only a saved-game/file stream needs the host,
                // because only that stream's bytes live outside us.
                //
                // Games ship a pre-computed save as a Data chunk and restore it at
                // boot to skip an expensive initialisation. Routing that through the
                // host's by-name path silently fails — a resource stream has no
                // fileref name, so the lookup below yields "" and finds nothing — and
                // the game recomputes on every launch instead (Counterfeit Monkey:
                // 5.4s rather than 0.7s, SQ-0595).
                if let Some(blob) = self.glk.stream_restore_bytes(l[0]) {
                    // `complete_restore_*` write the result through `pending_saveload`,
                    // so stage it exactly as the suspend path would.
                    self.pending_saveload = Some(PendingSaveLoad {
                        dest: s[0],
                        restore: true,
                        name: String::new(),
                        by_prompt: false,
                        save_stream: 0,
                        save_bytes: 0,
                        fresh: false,
                    });
                    // `restore_quetzal`, NOT `restore_state`: a blob a game carries in
                    // its own Blorb is a spec-standard Glulx-Quetzal (§1.8.4), with
                    // none of the `GReg`/`Glk ` chunks our host snapshots add — and
                    // `restore_state` rejects it outright for the missing `GReg`.
                    // It also pops the original @save's call stub storing -1, the
                    // "just restored" sentinel (§2.9).
                    match self.restore_quetzal(&blob) {
                        Ok(()) => {
                            self.pending_saveload = None;
                            self.undo_stack.clear();
                        }
                        Err(e) => {
                            self.diagnostics.push(format!("@restore from stream failed: {e:?}"));
                            self.complete_restore_failure();
                        }
                    }
                    return Ok(());
                }
                // L1 is the restore stream; carry its fileref name + prompt-ness so
                // the host reads the game's fixed file silently (failing cleanly if
                // absent) or surfaces the player's RESTORE-verb picker.
                let (name, by_prompt) = self.glk.stream_saveload_info(l[0]).unwrap_or_default();
                self.pending_saveload =
                    Some(PendingSaveLoad { dest: s[0], restore: true, name, by_prompt, save_stream: 0, save_bytes: 0, fresh: false });
                Ok(())
            }
            // Floating point (single-precision, GLULX_NOTES §13.1).
            0x190 => {
                // numtof L1 S1 — signed int -> nearest float.
                let (l, s) = self.read_operands(1, 1)?;
                self.store(s[0], (l[0] as i32 as f32).to_bits())
            }
            0x191 => {
                // ftonumz L1 S1 — float -> int, truncating toward zero.
                let (l, s) = self.read_operands(1, 1)?;
                let v = Self::dec(l[0]);
                let r = if v.is_nan() { 0x7FFF_FFFF } else { v as i32 as u32 };
                self.store(s[0], r)
            }
            0x192 => {
                // ftonumn L1 S1 — float -> int, rounding to nearest (half away from zero).
                let (l, s) = self.read_operands(1, 1)?;
                let v = Self::dec(l[0]);
                let r = if v.is_nan() { 0x7FFF_FFFF } else { v.round() as i32 as u32 };
                self.store(s[0], r)
            }
            0x198 => self.funop(f32::ceil),
            0x199 => self.funop(f32::floor),
            0x1A0 => self.fbinop(|a, b| a + b),
            0x1A1 => self.fbinop(|a, b| a - b),
            0x1A2 => self.fbinop(|a, b| a * b),
            0x1A3 => self.fbinop(|a, b| a / b),
            0x1A4 => {
                // fmod L1 L2 S1 S2 — S1 = remainder (sign of L1), S2 = quotient
                // truncated toward zero (as a float).
                let (l, s) = self.read_operands(2, 2)?;
                let (a, b) = (Self::dec(l[0]), Self::dec(l[1]));
                let q = (a / b).trunc();
                let r = a - q * b;
                self.store(s[0], r.to_bits())?;
                self.store(s[1], q.to_bits())
            }
            0x1A8 => self.funop(f32::sqrt),
            0x1A9 => self.funop(f32::exp),
            0x1AA => self.funop(f32::ln),
            0x1AB => self.fbinop(f32::powf),
            0x1B0 => self.funop(f32::sin),
            0x1B1 => self.funop(f32::cos),
            0x1B2 => self.funop(f32::tan),
            0x1B3 => self.funop(f32::asin),
            0x1B4 => self.funop(f32::acos),
            0x1B5 => self.funop(f32::atan),
            0x1B6 => self.fbinop(f32::atan2),
            0x1C0 => {
                // jfeq L1 L2 L3 offset — fuzzy float equality (see Self::feq).
                let (l, _) = self.read_operands(4, 0)?;
                let taken = Self::feq(Self::dec(l[0]), Self::dec(l[1]), Self::dec(l[2]));
                self.branch(l[3], taken)
            }
            0x1C1 => {
                // jfne L1 L2 L3 offset — inverse of jfeq (branches on NaN input).
                let (l, _) = self.read_operands(4, 0)?;
                let taken = !Self::feq(Self::dec(l[0]), Self::dec(l[1]), Self::dec(l[2]));
                self.branch(l[3], taken)
            }
            0x1C2 => self.branch2(|x, y| Self::dec(x) < Self::dec(y)),
            0x1C3 => self.branch2(|x, y| Self::dec(x) <= Self::dec(y)),
            0x1C4 => self.branch2(|x, y| Self::dec(x) > Self::dec(y)),
            0x1C5 => self.branch2(|x, y| Self::dec(x) >= Self::dec(y)),
            0x1C8 => self.branch1(|x| Self::dec(x).is_nan()),
            0x1C9 => self.branch1(|x| Self::dec(x).is_infinite()),
            // Floating point (double-precision, GLULX spec 3.1.3 §2.13). A double
            // occupies two words; reads take the high word first, stores write the
            // low word first (see dec64 / store64).
            0x200 => {
                // numtod L1 -> S1:S2 — signed int to nearest double.
                let (l, s) = self.read_operands(1, 2)?;
                self.store64(&s, l[0] as i32 as f64)
            }
            0x201 => {
                // dtonumz L1:L2 -> S1 — double to int, truncating toward zero.
                let (l, s) = self.read_operands(2, 1)?;
                let v = Self::dec64(l[0], l[1]);
                let r = if v.is_nan() { 0x7FFF_FFFF } else { v as i32 as u32 };
                self.store(s[0], r)
            }
            0x202 => {
                // dtonumn L1:L2 -> S1 — double to int, rounding to nearest.
                let (l, s) = self.read_operands(2, 1)?;
                let v = Self::dec64(l[0], l[1]);
                let r = if v.is_nan() { 0x7FFF_FFFF } else { v.round() as i32 as u32 };
                self.store(s[0], r)
            }
            0x203 => {
                // ftod L1 (float) -> S1:S2 (double) — exact widening.
                let (l, s) = self.read_operands(1, 2)?;
                self.store64(&s, Self::dec(l[0]) as f64)
            }
            0x204 => {
                // dtof L1:L2 -> S1 (float) — round to nearest.
                let (l, s) = self.read_operands(2, 1)?;
                self.store(s[0], (Self::dec64(l[0], l[1]) as f32).to_bits())
            }
            0x208 => self.dunop(f64::ceil),
            0x209 => self.dunop(f64::floor),
            0x210 => self.dbinop(|a, b| a + b),
            0x211 => self.dbinop(|a, b| a - b),
            0x212 => self.dbinop(|a, b| a * b),
            0x213 => self.dbinop(|a, b| a / b),
            0x214 => {
                // dmodr L1:L2 L3:L4 -> S1:S2 — remainder (sign of the dividend).
                let (l, s) = self.read_operands(4, 2)?;
                let (a, b) = (Self::dec64(l[0], l[1]), Self::dec64(l[2], l[3]));
                self.store64(&s, a - (a / b).trunc() * b)
            }
            0x215 => {
                // dmodq L1:L2 L3:L4 -> S1:S2 — quotient truncated toward zero.
                let (l, s) = self.read_operands(4, 2)?;
                let (a, b) = (Self::dec64(l[0], l[1]), Self::dec64(l[2], l[3]));
                self.store64(&s, (a / b).trunc())
            }
            0x218 => self.dunop(f64::sqrt),
            0x219 => self.dunop(f64::exp),
            0x21A => self.dunop(f64::ln),
            0x21B => self.dbinop(f64::powf),
            0x220 => self.dunop(f64::sin),
            0x221 => self.dunop(f64::cos),
            0x222 => self.dunop(f64::tan),
            0x223 => self.dunop(f64::asin),
            0x224 => self.dunop(f64::acos),
            0x225 => self.dunop(f64::atan),
            0x226 => self.dbinop(f64::atan2),
            0x230 => {
                // jdeq L1:L2 L3:L4 L5:L6(eps) L7(offset) — fuzzy double equality.
                let (l, _) = self.read_operands(7, 0)?;
                let taken = Self::deq(Self::dec64(l[0], l[1]), Self::dec64(l[2], l[3]), Self::dec64(l[4], l[5]));
                self.branch(l[6], taken)
            }
            0x231 => {
                // jdne — inverse of jdeq (branches on any NaN input).
                let (l, _) = self.read_operands(7, 0)?;
                let taken = !Self::deq(Self::dec64(l[0], l[1]), Self::dec64(l[2], l[3]), Self::dec64(l[4], l[5]));
                self.branch(l[6], taken)
            }
            0x232 => {
                let (l, _) = self.read_operands(5, 0)?;
                let taken = Self::dec64(l[0], l[1]) < Self::dec64(l[2], l[3]);
                self.branch(l[4], taken)
            }
            0x233 => {
                let (l, _) = self.read_operands(5, 0)?;
                let taken = Self::dec64(l[0], l[1]) <= Self::dec64(l[2], l[3]);
                self.branch(l[4], taken)
            }
            0x234 => {
                let (l, _) = self.read_operands(5, 0)?;
                let taken = Self::dec64(l[0], l[1]) > Self::dec64(l[2], l[3]);
                self.branch(l[4], taken)
            }
            0x235 => {
                let (l, _) = self.read_operands(5, 0)?;
                let taken = Self::dec64(l[0], l[1]) >= Self::dec64(l[2], l[3]);
                self.branch(l[4], taken)
            }
            0x238 => {
                // jdisnan L1:L2 L3(offset).
                let (l, _) = self.read_operands(3, 0)?;
                let taken = Self::dec64(l[0], l[1]).is_nan();
                self.branch(l[2], taken)
            }
            0x239 => {
                // jdisinf L1:L2 L3(offset).
                let (l, _) = self.read_operands(3, 0)?;
                let taken = Self::dec64(l[0], l[1]).is_infinite();
                self.branch(l[2], taken)
            }
            other => Err(format!("illegal/unimplemented opcode {other:#x}")),
        }
    }

    fn binop(&mut self, f: impl Fn(u32, u32) -> u32) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let v = f(l[0], l[1]);
        self.store(s[0], v)
    }

    fn unop(&mut self, f: impl Fn(u32) -> u32) -> R<()> {
        let (l, s) = self.read_operands(1, 1)?;
        let v = f(l[0]);
        self.store(s[0], v)
    }

    /// Decode a Glulx word as the IEEE-754 single-precision float it holds.
    fn dec(v: u32) -> f32 {
        f32::from_bits(v)
    }

    /// A 1-value float op (ceil, sqrt, sin, …): decode, apply, re-encode.
    fn funop(&mut self, f: impl Fn(f32) -> f32) -> R<()> {
        let (l, s) = self.read_operands(1, 1)?;
        let v = f(Self::dec(l[0]));
        self.store(s[0], v.to_bits())
    }

    /// A 2-value float op (fadd…fdiv, pow, atan2): decode, apply, re-encode.
    fn fbinop(&mut self, f: impl Fn(f32, f32) -> f32) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let v = f(Self::dec(l[0]), Self::dec(l[1]));
        self.store(s[0], v.to_bits())
    }

    /// `jfeq`'s fuzzy float equality: any NaN input is never equal; same-signed
    /// infinities are equal (opposite-signed are not); otherwise the values are
    /// equal if `|a - b| <= |tolerance|`.
    fn feq(a: f32, b: f32, c: f32) -> bool {
        if a.is_nan() || b.is_nan() || c.is_nan() {
            return false;
        }
        if a.is_infinite() && b.is_infinite() {
            return a == b;
        }
        (a - b).abs() <= c.abs()
    }

    /// Decode a Glulx (high, low) word pair as the IEEE-754 double it holds.
    /// Glulx reads doubles high word first (spec 3.1.3 §2.13: "read operands
    /// always read the high word first").
    fn dec64(hi: u32, lo: u32) -> f64 {
        f64::from_bits((u64::from(hi) << 32) | u64::from(lo))
    }

    /// Store a double to two dests, LOW word first (S1 = low, S2 = high) —
    /// the spec's write convention, deliberately the REVERSE of reads: "write
    /// operands always write the low word first" ("the result is stored as
    /// S2:S1"; `dadd Xhi Xlo Yhi Ylo RESlo REShi`, spec 3.1.3 §2.13; glulxe
    /// op_numtod stores lo→inst[1], hi→inst[2]). The asymmetry is what makes
    /// stack round-trips work: pushing lo then hi leaves the HIGH word on
    /// top, exactly where the next double read's first (high-word) pop looks,
    /// so `@numtod Stack Stack` → `@dtonumz Stack Stack` is the identity
    /// (pinned by `numtod_dtonumz_round_trip_through_the_stack`). (SQ-0626)
    fn store64(&mut self, s: &[Dest], v: f64) -> R<()> {
        let b = v.to_bits();
        self.store(s[0], b as u32)?;
        self.store(s[1], (b >> 32) as u32)
    }

    /// A 1-double op (dsqrt, dsin, …): read (2,2), decode hi:lo, apply, store lo:hi.
    fn dunop(&mut self, f: impl Fn(f64) -> f64) -> R<()> {
        let (l, s) = self.read_operands(2, 2)?;
        let v = f(Self::dec64(l[0], l[1]));
        self.store64(&s, v)
    }

    /// A 2-double op (dadd…ddiv, dpow, datan2): read (4,2), decode both, store lo:hi.
    fn dbinop(&mut self, f: impl Fn(f64, f64) -> f64) -> R<()> {
        let (l, s) = self.read_operands(4, 2)?;
        let v = f(Self::dec64(l[0], l[1]), Self::dec64(l[2], l[3]));
        self.store64(&s, v)
    }

    /// `jdeq`'s fuzzy double equality — the f64 analog of [`Self::feq`].
    fn deq(a: f64, b: f64, c: f64) -> bool {
        if a.is_nan() || b.is_nan() || c.is_nan() {
            return false;
        }
        if a.is_infinite() && b.is_infinite() {
            return a == b;
        }
        (a - b).abs() <= c.abs()
    }

    /// `div` (false) / `mod` (true): signed, truncating toward zero, faulting on
    /// a zero divisor.
    fn divop(&mut self, is_mod: bool) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let (a, b) = (l[0] as i32, l[1] as i32);
        if b == 0 {
            return Err(format!("division by zero ({})", if is_mod { "mod" } else { "div" }));
        }
        let v = if is_mod { a.wrapping_rem(b) } else { a.wrapping_div(b) };
        self.store(s[0], v as u32)
    }

    /// A 1-value conditional branch (jz/jnz): value then offset.
    fn branch1(&mut self, cond: impl Fn(u32) -> bool) -> R<()> {
        let (l, _) = self.read_operands(2, 0)?;
        let taken = cond(l[0]);
        self.branch(l[1], taken)
    }

    /// A 2-value conditional branch (jeq…jleu): two values then offset.
    fn branch2(&mut self, cond: impl Fn(u32, u32) -> bool) -> R<()> {
        let (l, _) = self.read_operands(3, 0)?;
        let taken = cond(l[0], l[1]);
        self.branch(l[2], taken)
    }

    /// Apply the branch convention: offset 0 → return 0, 1 → return 1, else
    /// `pc = pc_after_operands + offset - 2`. `pc` is already past the operands.
    fn branch(&mut self, offset: u32, taken: bool) -> R<()> {
        if !taken {
            return Ok(());
        }
        match offset {
            0 => self.return_value(0),
            1 => self.return_value(1),
            _ => {
                self.pc = self.pc.wrapping_add(offset).wrapping_sub(2);
                Ok(())
            }
        }
    }

    // ── stack byte accessors (offsets are guaranteed in-range by callers) ─────

    fn st_w32(&mut self, off: usize, v: u32) {
        self.stack[off..off + 4].copy_from_slice(&v.to_be_bytes());
    }
    fn st_r32(&self, off: usize) -> u32 {
        u32::from_be_bytes([
            self.stack[off],
            self.stack[off + 1],
            self.stack[off + 2],
            self.stack[off + 3],
        ])
    }

    /// Bounds-checked big-endian u32 read from the stack for trace-building.
    /// Returns None instead of panicking on an out-of-range offset.
    fn st_r32_opt(&self, off: usize) -> Option<u32> {
        let b = self.stack.get(off..off + 4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    // ── value stack (operands above the current frame) ────────────────────────

    /// Byte offset where the current frame's value stack begins.
    fn value_base(&self) -> usize {
        self.fp + self.cur_frame_len as usize
    }

    /// Push a 32-bit value onto the value stack.
    pub(crate) fn push32(&mut self, v: u32) -> R<()> {
        if self.sp + 4 > self.stack.len() {
            return Err("stack overflow".to_string());
        }
        let off = self.sp;
        self.st_w32(off, v);
        self.sp += 4;
        Ok(())
    }

    /// Pop a 32-bit value off the value stack.
    pub(crate) fn pop32(&mut self) -> R<u32> {
        if self.sp < self.value_base() + 4 {
            return Err("stack underflow".to_string());
        }
        self.sp -= 4;
        Ok(self.st_r32(self.sp))
    }

    /// Number of 32-bit values currently on the frame's value stack.
    /// `reload_frame_meta` guarantees `value_base() <= sp` for any installed
    /// frame; the saturation is belt-and-braces so a corrupt frame can never
    /// turn this into a debug panic / huge wrapped count (SQ-0623).
    pub(crate) fn value_count(&self) -> u32 {
        (self.sp.saturating_sub(self.value_base()) / 4) as u32
    }

    // ── call-frame locals (size-aware, per the locals format) ─────────────────

    fn local_size(&self, offset: u32) -> R<u8> {
        self.cur_locals
            .iter()
            .find(|(o, _)| *o == offset)
            .map(|(_, s)| *s)
            .ok_or_else(|| format!("bad local offset {offset:#x}"))
    }

    /// Read a local at byte `offset` within the locals region (size from format).
    pub(crate) fn local_load(&self, offset: u32) -> R<u32> {
        let size = self.local_size(offset)?;
        let base = self.fp + self.cur_localspos as usize + offset as usize;
        Ok(match size {
            1 => self.stack[base] as u32,
            2 => ((self.stack[base] as u32) << 8) | self.stack[base + 1] as u32,
            _ => self.st_r32(base),
        })
    }

    /// Write a local at byte `offset` (truncated to the local's declared size).
    pub(crate) fn local_store(&mut self, offset: u32, v: u32) -> R<()> {
        let size = self.local_size(offset)?;
        let base = self.fp + self.cur_localspos as usize + offset as usize;
        match size {
            1 => self.stack[base] = v as u8,
            2 => {
                self.stack[base] = (v >> 8) as u8;
                self.stack[base + 1] = v as u8;
            }
            _ => self.st_w32(base, v),
        }
        Ok(())
    }

    // ── frame construction ────────────────────────────────────────────────────

    /// Recompute `cur_frame_len`/`cur_localspos`/`cur_locals` from the frame
    /// header bytes at `fp`, VALIDATING them first. The header may come off a
    /// restored `Stks` chunk — untrusted input — so accept only what
    /// [`Machine::build_frame_and_enter`] could have produced: the format list
    /// between the header and `LocalsPos`, the locals inside `FrameLen`, and
    /// the whole frame inside the stack and at-or-below `sp`. Without this, a
    /// crafted `LocalsPos`/`FrameLen` sent `local_load`/`local_store` and
    /// `value_count` out of bounds (SQ-0623).
    fn reload_frame_meta(&mut self) -> R<()> {
        let fp = self.fp;
        if fp + 8 > self.stack.len() {
            return Err("corrupt frame pointer".to_string());
        }
        self.cur_frame_len = self.st_r32(fp);
        self.cur_localspos = self.st_r32(fp + 4);
        let frame_len = self.cur_frame_len as usize;
        let localspos = self.cur_localspos as usize;
        let frame_end = fp as u64 + frame_len as u64;
        if localspos < 8
            || localspos > frame_len
            || frame_end > self.stack.len() as u64
            || frame_end > self.sp as u64
        {
            return Err("corrupt frame header".to_string());
        }
        // Parse the locals-format pairs that begin at fp+8; a valid frame's
        // terminator sits before LocalsPos (LocalsPos = align4(8 + format)).
        // Reuse `cur_locals`' allocation — this runs on every function return,
        // and a fresh Vec per return fed the allocator hot spot (SQ-1208). An
        // error path leaves it empty, which is fine: every caller's error
        // channel halts the machine.
        let mut locals = std::mem::take(&mut self.cur_locals);
        locals.clear();
        let mut p = fp + 8;
        let fmt_end = fp + localspos;
        let mut off = 0u32;
        loop {
            if p + 2 > fmt_end {
                return Err("corrupt locals format".to_string());
            }
            let t = self.stack[p];
            let c = self.stack[p + 1];
            p += 2;
            if t == 0 && c == 0 {
                break;
            }
            if !matches!(t, 1 | 2 | 4) {
                return Err("corrupt locals format".to_string());
            }
            for _ in 0..c {
                off = align_up(off, t as u32);
                // Every local must lie inside the frame's locals region.
                if localspos as u64 + off as u64 + t as u64 > frame_len as u64 {
                    return Err("corrupt locals format".to_string());
                }
                locals.push((off, t));
                off += t as u32;
            }
        }
        self.cur_locals = locals;
        Ok(())
    }

    /// Build a call frame for the function at `func_addr` at the current `sp`,
    /// install it as the current frame, copy/push `args` per the function type,
    /// and set `pc` to the first instruction. Does NOT push a call stub — the
    /// caller does that for non-start calls.
    fn build_frame_and_enter(&mut self, func_addr: u32, args: &[u32]) -> R<()> {
        let func_type = self.m8(func_addr)?;
        if func_type != 0xC0 && func_type != 0xC1 {
            return Err(format!("not a function: type byte {func_type:#x} @{func_addr:#x}"));
        }

        // Parse the locals format in one pass: count the (type, count) pairs
        // and lay out the locals (each at its natural alignment). The layout
        // builds directly into `cur_locals`' reused allocation — this runs on
        // every call, and a fresh Vec (plus a scratch pairs Vec) per call fed
        // the allocator hot spot (SQ-1208). An error path leaves it empty,
        // which is fine: every caller's error channel halts the machine.
        let mut locals_layout = std::mem::take(&mut self.cur_locals);
        locals_layout.clear();
        let mut addr = func_addr + 1;
        let mut n_pairs = 0usize;
        let mut off = 0u32;
        loop {
            let t = self.m8(addr)? as u8;
            let c = self.m8(addr + 1)? as u8;
            addr += 2;
            if t == 0 && c == 0 {
                break;
            }
            if t != 1 && t != 2 && t != 4 {
                return Err(format!("bad local type {t} @{func_addr:#x}"));
            }
            n_pairs += 1;
            for _ in 0..c {
                off = align_up(off, t as u32);
                locals_layout.push((off, t));
                off += t as u32;
            }
        }
        let format_len = (n_pairs + 1) * 2; // includes the (0,0) terminator
        let localspos = align_up(8 + format_len as u32, 4);
        let locals_size = off;
        let frame_len = align_up(localspos + locals_size, 4);

        let frameptr = self.sp;
        if frameptr + frame_len as usize > self.stack.len() {
            return Err("stack overflow building call frame".to_string());
        }

        // Zero the whole frame region, then write the header + format list.
        for b in &mut self.stack[frameptr..frameptr + frame_len as usize] {
            *b = 0;
        }
        self.st_w32(frameptr, frame_len);
        self.st_w32(frameptr + 4, localspos);
        // Copy the format pairs into the frame, re-reading the bytes the parse
        // above already validated (cannot fault: same addresses, same memory).
        for i in 0..n_pairs {
            let a = func_addr + 1 + 2 * i as u32;
            self.stack[frameptr + 8 + 2 * i] = self.m8(a)? as u8;
            self.stack[frameptr + 8 + 2 * i + 1] = self.m8(a + 1)? as u8;
        }
        // (terminator pair already zero from the wipe above)

        self.fp = frameptr;
        self.sp = frameptr + frame_len as usize;
        self.pc = addr;
        self.cur_frame_len = frame_len;
        self.cur_localspos = localspos;
        self.cur_locals = locals_layout;

        // Pass the arguments.
        if func_type == 0xC1 {
            // Copy args into locals in order, truncated to each local's size.
            // (Looked up per index so `local_store` can borrow `self` — no
            // layout clone.)
            for (i, &arg) in args.iter().enumerate() {
                let Some(&(loff, _sz)) = self.cur_locals.get(i) else { break };
                self.local_store(loff, arg)?;
            }
        } else {
            // C0: push args (last first → first on top), then the count.
            for &a in args.iter().rev() {
                self.push32(a)?;
            }
            self.push32(args.len() as u32)?;
        }
        Ok(())
    }

    /// Call the function at `func_addr` with `args`, storing its eventual return
    /// value per `dest`. Pushes a call stub, then builds the callee frame.
    pub(crate) fn call_function(&mut self, func_addr: u32, args: &[u32], dest: Dest) -> R<()> {
        if self.acceleration {
            if let Some(num) = self.accel_func_for(func_addr) {
                if crate::accel::accel_impl_supported(num) {
                    let result = self.accel_dispatch(num, args)?;
                    return self.store(dest, result);
                }
            }
        }
        let (dtype, daddr) = dest.to_stub();
        let ret_pc = self.pc;
        let caller_fp = self.fp as u32;
        // Push the stub: DestType, DestAddr, PC, FramePtr (FramePtr ends on top).
        self.push32(dtype)?;
        self.push32(daddr)?;
        self.push32(ret_pc)?;
        self.push32(caller_fp)?;
        self.build_frame_and_enter(func_addr, args)
    }

    /// Return `v` from the current function. Pops the frame and its call stub,
    /// restores the caller, and stores `v` per the stub's destination. Returning
    /// from the start frame (`fp == 0`) halts the machine.
    pub(crate) fn return_value(&mut self, v: u32) -> R<()> {
        if self.fp == 0 {
            self.halted = true;
            return Ok(());
        }
        // Discard the current frame (and any pushed values).
        self.sp = self.fp;
        self.pop_save_stub_and_store(v)
    }

    /// Pop a 4-word call stub (`DestType, DestAddr, PC, FramePtr` — reverse of
    /// the push) off the top of the stack, restore the caller's `fp`/`pc`, and
    /// store `v` per the stub's destination. Shared tail of [`Machine::return_value`]
    /// (which first discards the current frame via `sp = fp`) and `@save`'s
    /// `complete_save` (which must NOT discard a frame — `@save` pushed only a
    /// stub, not a new one).
    fn pop_save_stub_and_store(&mut self, v: u32) -> R<()> {
        if self.sp < 16 {
            return Err("corrupt call stub on return".to_string());
        }
        self.sp -= 4;
        let caller_fp = self.st_r32(self.sp);
        self.sp -= 4;
        let ret_pc = self.st_r32(self.sp);
        self.sp -= 4;
        let daddr = self.st_r32(self.sp);
        self.sp -= 4;
        let dtype = self.st_r32(self.sp);

        self.fp = caller_fp as usize;
        self.pc = ret_pc;
        self.reload_frame_meta()?;

        match dtype {
            0 => {} // discard
            1 => self.store_mem(daddr, v)?,
            2 => self.local_store(daddr, v)?,
            3 => self.push32(v)?,
            other => return Err(format!("bad call-stub DestType {other}")),
        }
        Ok(())
    }

    /// `catch S L`: push a catch stub (so `@throw` can unwind here), store the
    /// resulting **catch token** (the stack pointer) into `S`, then branch by `L`.
    /// A later `@throw token` resumes just past this instruction, with the thrown
    /// value stored into `S` instead. Operands are `store` then `load` (branch),
    /// the reverse of the usual order, so decode them by hand.
    fn op_catch(&mut self) -> R<()> {
        let mode_byte = self.m8(self.pc)?;
        self.pc += 1;
        let smode = (mode_byte & 0x0F) as u8;
        let lmode = (mode_byte >> 4) as u8;
        let dest = self.resolve_store(smode)?;
        let offset = self.resolve_load(lmode)?;
        // Push the catch stub: DestType, DestAddr, PC (the no-branch resume),
        // FramePtr — identical layout to a call stub.
        let (dtype, daddr) = dest.to_stub();
        let ret_pc = self.pc;
        let caller_fp = self.fp as u32;
        self.push32(dtype)?;
        self.push32(daddr)?;
        self.push32(ret_pc)?;
        self.push32(caller_fp)?;
        let token = self.sp as u32; // the catch token = sp just above the stub
        self.store(dest, token)?;
        self.branch(offset, true)
    }

    /// `throw value token`: restore the stack to `token` (the catch token), pop
    /// the catch stub, store `value` per the stub's destination, and resume at the
    /// stub's PC (just past the matching `@catch`). Faults on a corrupt token.
    fn op_throw(&mut self) -> R<()> {
        let (l, _) = self.read_operands(2, 0)?;
        let (value, token) = (l[0], l[1]);
        let tok = token as usize;
        if tok < 16 || tok > self.stack.len() {
            return Err(format!("throw: invalid catch token {token:#x}"));
        }
        // Unwind, then pop the stub exactly as a return does.
        self.sp = tok;
        self.sp -= 4;
        let caller_fp = self.st_r32(self.sp);
        self.sp -= 4;
        let ret_pc = self.st_r32(self.sp);
        self.sp -= 4;
        let daddr = self.st_r32(self.sp);
        self.sp -= 4;
        let dtype = self.st_r32(self.sp);

        self.fp = caller_fp as usize;
        self.pc = ret_pc;
        self.reload_frame_meta()?;
        match dtype {
            0 => Ok(()),
            1 => self.store_mem(daddr, value),
            2 => self.local_store(daddr, value),
            3 => self.push32(value),
            other => Err(format!("throw: bad catch-stub DestType {other}")),
        }
    }

    /// Write a 32-bit `v` to main memory at `addr`.
    pub(crate) fn store_mem(&mut self, addr: u32, v: u32) -> R<()> {
        self.store_mem_sized(addr, v, 4)
    }

    /// Write the low `width` bytes of `v` to main memory at `addr`, mapping
    /// ROM/out-of-range faults to a diagnostic (ROM) or a fault (out of range).
    fn store_mem_sized(&mut self, addr: u32, v: u32, width: u32) -> R<()> {
        use crate::memory::WriteFault;
        let res = match width {
            1 => self.mem.write8(addr, v),
            2 => self.mem.write16(addr, v),
            _ => self.mem.write32(addr, v),
        };
        match res {
            Ok(()) => Ok(()),
            Err(WriteFault::Rom) => {
                self.diagnostics.push(format!("ignored ROM write @{addr:#010x}"));
                Ok(())
            }
            Err(WriteFault::OutOfRange) => Err(format!("memory fault: write @{addr:#010x}")),
        }
    }

    fn read_width(&self, addr: u32, width: u32) -> R<u32> {
        match width {
            1 => self.m8(addr),
            2 => self.m16(addr),
            _ => self.m32(addr),
        }
    }

    // ── copys/copyb (width-sized copies; no sign extension) ───────────────────

    /// `copys` (width 2) / `copyb` (width 1): copy a sub-word value. Memory
    /// operands access `width` bytes; stack operands stay 32-bit. (Local
    /// operands — deprecated — are read/written at the local's declared size.)
    fn copy_sized(&mut self, width: u32) -> R<()> {
        let mode_byte = self.take8()? as u8;
        let lmode = mode_byte & 0x0F;
        let smode = mode_byte >> 4;
        let v = self.resolve_load_sized(lmode, width)?;
        let dest = self.resolve_store(smode)?;
        match dest {
            Dest::Mem(a) => self.store_mem_sized(a, v, width),
            other => self.store(other, v),
        }
    }

    fn resolve_load_sized(&mut self, mode: u8, width: u32) -> R<u32> {
        let ramstart = self.mem.ramstart();
        let mask = if width >= 4 { u32::MAX } else { (1u32 << (width * 8)) - 1 };
        let v = match mode {
            0x0 => 0,
            0x1 => self.take8()?, // zero-extended (no sign extension for copys/copyb)
            0x2 => self.take16()?,
            0x3 => self.take32()?,
            0x5 => {
                let a = self.take8()?;
                self.read_width(a, width)?
            }
            0x6 => {
                let a = self.take16()?;
                self.read_width(a, width)?
            }
            0x7 => {
                let a = self.take32()?;
                self.read_width(a, width)?
            }
            0x8 => self.pop32()?,
            0x9 => {
                let o = self.take8()?;
                self.local_load(o)?
            }
            0xA => {
                let o = self.take16()?;
                self.local_load(o)?
            }
            0xB => {
                let o = self.take32()?;
                self.local_load(o)?
            }
            // Wrapping for the same reason as resolve_load's 0xD..0xF arms.
            0xD => {
                let a = self.take8()?.wrapping_add(ramstart);
                self.read_width(a, width)?
            }
            0xE => {
                let a = self.take16()?.wrapping_add(ramstart);
                self.read_width(a, width)?
            }
            0xF => {
                let a = self.take32()?.wrapping_add(ramstart);
                self.read_width(a, width)?
            }
            other => return Err(format!("illegal load operand mode {other:#x}")),
        };
        Ok(v & mask)
    }

    // ── memory-array opcodes (GLULX_NOTES; L2 is a signed index) ──────────────

    /// `aload`/`aloads`/`aloadb` (width 4/2/1): load from `L1 + width*L2`
    /// (L2 signed), zero-extended to 32 bits.
    fn op_aload(&mut self, width: u32) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let addr = Self::array_addr(l[0], l[1], width);
        let v = self.read_width(addr, width)?;
        self.store(s[0], v)
    }

    /// `astore`/`astores`/`astoreb` (width 4/2/1): store the low `width` bytes
    /// of L3 at `L1 + width*L2` (L2 signed).
    fn op_astore(&mut self, width: u32) -> R<()> {
        let (l, _) = self.read_operands(3, 0)?;
        let addr = Self::array_addr(l[0], l[1], width);
        self.store_mem_sized(addr, l[2], width)
    }

    /// `aloadbit`: store bit `(L2 mod 8)` of byte `(L1 + L2/8)` (L2 signed,
    /// flooring division) as 0/1.
    fn op_aloadbit(&mut self) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let (addr, bit) = Self::bit_addr(l[0], l[1]);
        let byte = self.m8(addr)?;
        self.store(s[0], (byte >> bit) & 1)
    }

    /// `astorebit`: set (L3 nonzero) or clear (L3 zero) bit `(L2 mod 8)` of byte
    /// `(L1 + L2/8)` (L2 signed).
    fn op_astorebit(&mut self) -> R<()> {
        let (l, _) = self.read_operands(3, 0)?;
        let (addr, bit) = Self::bit_addr(l[0], l[1]);
        let byte = self.m8(addr)?;
        let v = if l[2] != 0 { byte | (1 << bit) } else { byte & !(1 << bit) };
        self.store_mem_sized(addr, v, 1)
    }

    /// Compute `base + scale*index` with `index` taken as signed.
    fn array_addr(base: u32, index: u32, scale: u32) -> u32 {
        (base as i64 + scale as i64 * (index as i32 as i64)) as u32
    }

    /// Compute `(byte_address, bit_number)` for the signed bit index `L2`.
    fn bit_addr(base: u32, index: u32) -> (u32, u32) {
        let idx = index as i32;
        let byte = (base as i64 + idx.div_euclid(8) as i64) as u32;
        (byte, idx.rem_euclid(8) as u32)
    }

    /// `mzero count addr`: zero `count` bytes starting at `addr`.
    fn op_mzero(&mut self) -> R<()> {
        let (l, _) = self.read_operands(2, 0)?;
        let (count, addr) = (l[0], l[1]);
        for i in 0..count {
            self.store_mem_sized(addr + i, 0, 1)?;
        }
        Ok(())
    }

    /// `mcopy count from to`: copy `count` bytes, choosing the copy direction so
    /// overlapping ranges move correctly (spec §2.6).
    fn op_mcopy(&mut self) -> R<()> {
        let (l, _) = self.read_operands(3, 0)?;
        let (count, from, to) = (l[0], l[1], l[2]);
        if to < from {
            for i in 0..count {
                let b = self.m8(from + i)?;
                self.store_mem_sized(to + i, b, 1)?;
            }
        } else {
            for i in (0..count).rev() {
                let b = self.m8(from + i)?;
                self.store_mem_sized(to + i, b, 1)?;
            }
        }
        Ok(())
    }

    // ── gestalt (GLULX_NOTES §13) ─────────────────────────────────────────────

    /// Version of the Glulx spec this VM targets (3.1.3).
    const GLULX_VERSION: u32 = 0x0003_0103;
    /// This interpreter's own version (0.1.0).
    const TERP_VERSION: u32 = 0x0000_0100;

    /// Report the capability for gestalt selector `sel` (with argument `arg`).
    /// Returned values reflect what this VM actually implements; unimplemented
    /// selectors and deferred features return 0.
    fn gestalt(&self, sel: u32, arg: u32) -> u32 {
        match sel {
            0 => Self::GLULX_VERSION,                  // GlulxVersion
            1 => Self::TERP_VERSION,                   // TerpVersion
            2 => 1,                                    // ResizeMem
            3 => 1,                                    // Undo (saveundo/restoreundo)
            4 => u32::from(arg == 0 || arg == 1 || arg == 2), // IOSystem: null + filter + Glk
            5 => 1,                                    // Unicode
            6 => 1,                                    // MemCopy
            7 => 1,                                    // MAlloc
            8 => self.heap_start,                      // MAllocHeap (0 if inactive)
            9 => 1,                                    // Acceleration: interception implemented
            10 => u32::from(crate::accel::accel_impl_supported(arg)), // AccelFunc: implemented function numbers
            11 => 1,                                   // Float
            12 => 1,                                   // ExtUndo (hasundo / discardundo)
            13 => 1,                                   // Double (double-precision opcodes)
            _ => 0,
        }
    }

    // ── save / restore serialization core (GLULX_NOTES §14) ───────────────────

    /// Serialize the VM's mutable state as Glulx-Quetzal bytes (`FORM IFZS`:
    /// `IFhd` identity, `CMem` compressed RAM, `Stks` stack, `MAll` heap, a
    /// `GReg` register chunk, and a `Glk ` window/stream-model chunk). Round-trips
    /// exactly via [`Machine::restore_state`]; the `Glk ` chunk makes the snapshot
    /// self-contained so a cross-session restore reinstalls the display state.
    pub fn save_state(&self) -> Vec<u8> {
        let mut body = Vec::new();

        // IFhd: the first 128 bytes of memory (identity).
        let mut ifhd = Vec::with_capacity(128);
        for a in 0..128 {
            ifhd.push(self.mem.read8(a).unwrap_or(0) as u8);
        }
        push_chunk(&mut body, b"IFhd", &ifhd);

        // CMem: current memsize, then the RLE-compressed diff against the
        // original image over [RAMSTART, memsize).
        push_chunk(&mut body, b"CMem", &self.compress_ram());

        // Stks: the live stack bytes [0, sp).
        push_chunk(&mut body, b"Stks", &self.stack[..self.sp]);

        // MAll: heap-start, block count, then (addr, len) per block.
        let mut mall = Vec::new();
        mall.extend_from_slice(&self.heap_start.to_be_bytes());
        mall.extend_from_slice(&(self.heap_blocks.len() as u32).to_be_bytes());
        for &(a, sz) in &self.heap_blocks {
            mall.extend_from_slice(&a.to_be_bytes());
            mall.extend_from_slice(&sz.to_be_bytes());
        }
        push_chunk(&mut body, b"MAll", &mall);

        // GReg: registers not derivable from the stack (sp, fp, pc, iosys,
        // string table, protect range).
        let mut greg = Vec::with_capacity(32);
        for v in [
            self.sp as u32,
            self.fp as u32,
            self.pc,
            self.iosys_mode,
            self.iosys_rock,
            self.cur_stringtbl,
            self.protect.0,
            self.protect.1,
        ] {
            greg.extend_from_slice(&v.to_be_bytes());
        }
        push_chunk(&mut body, b"GReg", &greg);

        // Glk: the window/stream model, so a snapshot is self-contained and a
        // cross-session restore (a fresh Machine) reinstalls the display state.
        push_chunk(&mut body, b"Glk ", &self.glk.serialize());

        let mut out = Vec::with_capacity(body.len() + 12);
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(b"IFZS");
        out.extend_from_slice(&body);
        out
    }

    /// Serialize a **spec-conformant standard Glulx-Quetzal** in-game save
    /// (Glulx spec §1.8): `FORM IFZS` with `IFhd + CMem + Stks + MAll` only — no
    /// `GReg`, no `Glk ` chunk. Unlike [`Machine::save_state`], PC/FP/SP are not
    /// serialized as registers; they are recovered on restore from the saved
    /// stack together with the call stub `@save` pushes for its result operand
    /// (§1.3.2, §1.8.2). This is the format `@save` hands to the host.
    pub fn save_quetzal(&self) -> Vec<u8> {
        let mut body = Vec::new();

        // IFhd: the first 128 bytes of memory (identity).
        let mut ifhd = Vec::with_capacity(128);
        for a in 0..128 {
            ifhd.push(self.mem.read8(a).unwrap_or(0) as u8);
        }
        push_chunk(&mut body, b"IFhd", &ifhd);

        // CMem: current memsize, then the RLE-compressed diff against the
        // original image over [RAMSTART, memsize).
        push_chunk(&mut body, b"CMem", &self.compress_ram());

        // Stks: the live stack bytes [0, sp), including the pending @save call
        // stub — restore_quetzal (Task 2) pops it to resume.
        push_chunk(&mut body, b"Stks", &self.stack[..self.sp]);

        // MAll: heap-start, block count, then (addr, len) per block.
        let mut mall = Vec::new();
        mall.extend_from_slice(&self.heap_start.to_be_bytes());
        mall.extend_from_slice(&(self.heap_blocks.len() as u32).to_be_bytes());
        for &(a, sz) in &self.heap_blocks {
            mall.extend_from_slice(&a.to_be_bytes());
            mall.extend_from_slice(&sz.to_be_bytes());
        }
        push_chunk(&mut body, b"MAll", &mall);

        let mut out = Vec::with_capacity(body.len() + 12);
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(b"IFZS");
        out.extend_from_slice(&body);
        out
    }

    /// RLE-compress the RAM diff `[RAMSTART, memsize)` against the original
    /// image (`CMem` body: 4-byte memsize then the compressed bytes). A run of
    /// 1..=256 zero bytes is `0x00` followed by `(count-1)`; non-zero diff bytes
    /// are literal (spec §1.8 / Quetzal).
    fn compress_ram(&self) -> Vec<u8> {
        // Slices, not a bounds-checked `read8` per byte: this runs on the
        // player's thread for every host snapshot (a probe question, the
        // per-turn auto-save), and the per-byte `Option` call was the whole
        // cost (SQ-1177). Same diff, same RLE, byte-identical output —
        // `compress_ram_matches_the_per_byte_reference` holds the two side
        // by side.
        let ramstart = self.mem.ramstart() as usize;
        let memsize = self.mem.mem_size() as usize;
        let cur = &self.mem.raw_bytes()[ramstart..memsize];
        let orig = self.mem.orig_bytes();
        let mut diff = vec![0u8; memsize - ramstart];
        // `orig` covers [0, EXTSTART) and is zero by definition above it, so
        // the diff is an XOR over the overlap and the current bytes verbatim
        // past it. `ramstart <= EXTSTART <= memsize` always; the clamps keep
        // this correct rather than panicking if that ever moved.
        let overlap = orig.len().clamp(ramstart, memsize) - ramstart;
        diff.copy_from_slice(cur);
        for (d, o) in diff[..overlap].iter_mut().zip(&orig[ramstart..]) {
            *d ^= *o;
        }
        let mut out = Vec::new();
        out.extend_from_slice(&(memsize as u32).to_be_bytes());
        let mut i = 0;
        while i < diff.len() {
            if diff[i] == 0 {
                // A zero run: up to 256 zeros become `00 (run-1)`.
                let cap = (i + 256).min(diff.len());
                let run = diff[i..cap].iter().take_while(|&&b| b == 0).count();
                out.push(0);
                out.push((run - 1) as u8);
                i += run;
            } else {
                // A literal stretch: copy whole, up to the next zero.
                let end = i + diff[i..].iter().take_while(|&&b| b != 0).count();
                out.extend_from_slice(&diff[i..end]);
                i = end;
            }
        }
        out
    }

    /// Restore VM state from [`Machine::save_state`] bytes. Resets RAM to the
    /// original image, applies the saved diff, rebuilds the stack/heap/registers,
    /// preserves the protected range, and recomputes the frame cache. Corrupt or
    /// truncated data yields a [`GError`] (never a panic).
    pub fn restore_state(&mut self, data: &[u8]) -> Result<(), GError> {
        let chunks = parse_ifzs(data)?;
        let find = |id: &[u8; 4]| chunks.iter().find(|(cid, _)| cid == id).map(|(_, d)| *d);
        // Preserve the original missing-chunk error precedence (CMem, Stks,
        // GReg) even though the shared core re-derives CMem/Stks itself.
        find(b"CMem").ok_or_else(|| GError::BadSave("missing CMem chunk".into()))?;
        find(b"Stks").ok_or_else(|| GError::BadSave("missing Stks chunk".into()))?;
        let greg = find(b"GReg").ok_or_else(|| GError::BadSave("missing GReg chunk".into()))?;
        if greg.len() != 32 {
            return Err(GError::BadSave("GReg chunk has wrong length".into()));
        }
        let g = |i: usize| u32::from_be_bytes([greg[i], greg[i + 1], greg[i + 2], greg[i + 3]]);
        let (sp, fp, pc) = (g(0), g(4), g(8));
        let (iosys_mode, iosys_rock, stringtbl) = (g(12), g(16), g(20));
        let (paddr, plen) = (g(24), g(28));

        self.restore_vm_core(&chunks, Some(sp))?;

        self.fp = fp as usize;
        self.pc = pc;
        self.iosys_mode = iosys_mode;
        self.iosys_rock = iosys_rock;
        self.cur_stringtbl = stringtbl;
        self.protect = (paddr, plen);
        self.pending_fileref = None;

        // Recompute the current-frame cache from the restored stack.
        self.reload_frame_meta().map_err(GError::BadSave)?;

        // Glk model: reinstall the window/stream tree from the "Glk " chunk so a
        // restore into a fresh Machine has live windows. An older snapshot with
        // no such chunk restores with an empty model (back-compat, no panic).
        // The per-game borderless mode is a HOST/runtime setting, not game
        // state: it is deliberately absent from the snapshot, and the live
        // value survives the model swap — @restoreundo runs through here with
        // no host callback to re-apply it (SQ-0627).
        let borderless = self.glk.borderless();
        self.glk = match find(b"Glk ") {
            Some(d) => Model::deserialize(d).map_err(GError::BadSave)?,
            None => Model::new(),
        };
        self.glk.set_borderless(borderless);
        // A snapshot never carries a suspended `@save`/`@restore`: §1.8.5 keeps
        // the interpreter's own suspensions out of the file, and the host guards
        // its snapshot trigger on `is_saveload_pending` precisely so an un-popped
        // `@save` call stub is never serialized. So whatever is pending here
        // belongs to the run we just REPLACED, and must not survive it (SQ-0656):
        // left set, the next `step` re-reports a SaveRequest/RestoreRequest out of
        // nowhere (the host's restore dialog reopens by itself), and completing it
        // would store 0/1 through a `dest` record describing a stack that no
        // longer exists — a push-type dest silently imbalances the restored stack.
        // Cleared only once the restore has COMMITTED, so a blob that fails partway
        // still leaves the game's own `@restore` answerable by the caller's
        // `complete_restore_failure`.
        self.pending_saveload = None;
        // The snapshot carries the game's input REQUESTS (in the `Glk ` chunk) but
        // not the interpreter's own `glk_select` suspension — §1.8.5 keeps that out
        // of the file, and its `event_addr` belongs to the live run we are resuming
        // into, so it is the one thing here we must keep. Re-point that suspension
        // at the request the restored state actually holds (SQ-0656): a snapshot
        // captured at a "press any key" prompt, restored while the session sits at
        // a line prompt, otherwise stays in LINE mode — the host shows an input bar
        // and the game gets a LineInput event it never asked for. With no request
        // at all in the snapshot the live suspension is left untouched, so the host
        // can still resolve it rather than being stranded.
        if let Some(pi) = self.pending_input.as_mut() {
            if let Some((win, unicode)) = self.glk.first_line_request() {
                *pi = PendingInput { event_addr: pi.event_addr, win, line: true, unicode };
            } else if let Some((win, unicode)) = self.glk.first_char_request() {
                *pi = PendingInput { event_addr: pi.event_addr, win, line: false, unicode };
            }
        }
        Ok(())
    }

    /// Is `data` a Glulx-Quetzal save whose `IFhd` names a DIFFERENT story than the
    /// one running? False for anything that is not a save at all, and false for a
    /// save that matches — only a foreign one is true.
    ///
    /// A game probes `glk_stream_open_resource` to ask "is there an embedded save I
    /// can restore?", then hands the stream straight to `@restore`. When that save
    /// is for another build, the honest answer to the probe is NO: we would reject
    /// the restore on the §1.8.4 identity check anyway, so opening it only invites a
    /// guaranteed failure. Counterfeit Monkey r11 ships exactly that — a startup
    /// save whose RAMSTART/EXTSTART disagree with the story chunk beside it in the
    /// same blorb — and answering YES cost every launch a full re-initialisation
    /// (5.4s against 0.77s) because the game does not fall back to its own on-disk
    /// cache once it believes an embedded one exists. Bypassing the identity check
    /// instead is not an option: the restored state faults on the first command.
    ///
    /// The test MUST agree with [`Machine::restore_quetzal`]'s: hide exactly the
    /// saves we would refuse, never a restorable one.
    /// `quetzal_resource_hidden_iff_restore_would_reject_it` pins that.
    fn quetzal_for_another_story(&self, data: &[u8]) -> bool {
        if data.len() < 12 || &data[0..4] != b"FORM" || &data[8..12] != b"IFZS" {
            return false; // not a save: an ordinary data resource, always offered
        }
        let mut i = 12usize;
        while i + 8 <= data.len() {
            let len =
                u32::from_be_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
            if &data[i..i + 4] == b"IFhd" {
                let Some(ifhd) = data.get(i + 8..i + 8 + len) else { return false };
                return ifhd.len() != 128
                    || (0..128).any(|a| ifhd[a] != self.mem.read8(a as u32).unwrap_or(0) as u8);
            }
            i = i + 8 + len + (len & 1);
        }
        false // a save with no IFhd names no story; leave it to `@restore` to refuse
    }

    /// Restore VM state from a [`Machine::save_quetzal`] blob (a standard,
    /// spec-conformant Glulx-Quetzal save — Glulx spec §1.8). Restores
    /// RAM/stack/heap exactly like [`Machine::restore_state`], then **pops the
    /// `@save` call stub off the restored stack and stores the "just restored"
    /// sentinel `-1` into it** (§1.8.2, §2.9) — recovering PC/FramePtr from the
    /// stub rather than from a serialized register, since the bare format
    /// carries none.
    ///
    /// Per §1.8.5, this leaves all other live interpreter state **untouched**:
    /// the Glk model (windows/streams/VFS), `iosys_mode`/`iosys_rock`, the
    /// current string-decoding table, and the protect range keep their current
    /// live values. This is the correct semantics for a mid-session in-game
    /// `@restore`; contrast [`Machine::restore_state`], which *replaces* all of
    /// that from `GReg`/`Glk ` (correct only for a cold, cross-session Save
    /// State restore into a fresh machine).
    pub fn restore_quetzal(&mut self, blob: &[u8]) -> Result<(), GError> {
        let chunks = parse_ifzs(blob)?;

        // Identity check (Glulx spec §1.8.4): IFhd is the first 128 bytes of
        // memory as of the save, invariant ROM that names the story. Reject a
        // save from a different story before any state mutation.
        let ifhd = chunks
            .iter()
            .find(|(cid, _)| cid == b"IFhd")
            .map(|(_, d)| *d)
            .ok_or_else(|| GError::BadSave("missing IFhd chunk".into()))?;
        if ifhd.len() != 128 || (0..128).any(|a| ifhd[a] != self.mem.read8(a as u32).unwrap_or(0) as u8) {
            return Err(GError::BadSave("save is for a different story".into()));
        }

        self.restore_vm_core(&chunks, None)?;
        self.pop_save_stub_and_store((-1i32) as u32).map_err(GError::BadSave)
    }

    /// Shared memory/stack/heap restore core for both [`Machine::restore_state`]
    /// and [`Machine::restore_quetzal`]: resets RAM to the original image and
    /// applies the saved `CMem` (RLE diff, spec §1.8) or `UMem` (literal, no
    /// diff) chunk — honoring the *live* protect range so protected bytes keep
    /// their pre-restore values — then loads the stack from `Stks` and rebuilds
    /// the heap from `MAll`. Does **not** touch fp/pc/iosys/stringtbl/protect/Glk;
    /// callers apply those from their own register source (or leave them live)
    /// afterward, and callers own `reload_frame_meta()`/the stub pop once fp is
    /// known.
    ///
    /// `expected_sp`, when `Some`, cross-checks the `Stks` chunk's length against
    /// a register-carried `sp` the way `restore_state`'s `GReg` does;
    /// `restore_quetzal` has no such register (sp is simply the `Stks` length)
    /// and passes `None`.
    fn restore_vm_core(&mut self, chunks: &IfzsChunks, expected_sp: Option<u32>) -> Result<(), GError> {
        let find = |id: &[u8; 4]| chunks.iter().find(|(cid, _)| cid == id).map(|(_, d)| *d);
        let stks = find(b"Stks").ok_or_else(|| GError::BadSave("missing Stks chunk".into()))?;

        // Snapshot the currently-protected bytes (preserved across restore).
        // Capture 0 for any address currently out of bounds (memory was shrunk
        // below the protected range): the restore re-extends memory and would
        // otherwise leak the saved diff into the protected range, so the
        // re-impose loop must zero it back out.
        let (cur_paddr, cur_plen) = self.protect;
        let mut protected: Vec<(u32, u8)> = Vec::new();
        for a in cur_paddr..cur_paddr.saturating_add(cur_plen) {
            protected.push((a, self.mem.read8(a).unwrap_or(0) as u8));
        }

        // Reset RAM to the original image and apply the saved memory chunk —
        // CMem (RLE diff) or UMem (literal), whichever is present.
        if let Some(cmem) = find(b"CMem") {
            self.decompress_ram(cmem)?;
        } else if let Some(umem) = find(b"UMem") {
            self.load_umem(umem)?;
        } else {
            return Err(GError::BadSave("missing CMem/UMem chunk".into()));
        }

        // Re-impose the protected bytes' pre-restore values.
        for (a, b) in protected {
            if a >= self.mem.ramstart() && a < self.mem.mem_size() {
                self.mem.write_byte_raw(a, b);
            }
        }

        // Stack: the Stks bytes must fit the buffer and (for restore_state)
        // must agree with the GReg-carried sp.
        if let Some(sp) = expected_sp {
            if stks.len() != sp as usize {
                return Err(GError::BadSave("Stks length disagrees with sp".into()));
            }
        }
        if stks.len() > self.stack.len() {
            return Err(GError::BadSave("saved sp exceeds the stack size".into()));
        }
        self.stack[..stks.len()].copy_from_slice(stks);
        self.sp = stks.len();

        // Heap: rebuild from MAll (absent/empty → inactive).
        self.heap_start = 0;
        self.heap_blocks.clear();
        if let Some(mall) = find(b"MAll") {
            self.restore_heap(mall)?;
        }

        Ok(())
    }

    /// Decompress a `CMem` body into RAM: read the saved memsize, resize memory,
    /// and rebuild `[RAMSTART, memsize)` as `original XOR diff`. Faults on a
    /// truncated/over-long stream.
    fn decompress_ram(&mut self, cmem: &[u8]) -> Result<(), GError> {
        if cmem.len() < 4 {
            return Err(GError::BadSave("CMem chunk too short".into()));
        }
        let memsize = u32::from_be_bytes([cmem[0], cmem[1], cmem[2], cmem[3]]);
        let ramstart = self.mem.ramstart();
        if memsize < ramstart || memsize > Self::MAX_MEMSIZE {
            return Err(GError::BadSave("CMem memsize out of range".into()));
        }
        self.mem.set_raw_size(memsize);
        let mut addr = ramstart;
        let mut i = 4;
        while addr < memsize {
            if i >= cmem.len() {
                return Err(GError::BadSave("CMem data truncated".into()));
            }
            let b = cmem[i];
            i += 1;
            if b == 0 {
                if i >= cmem.len() {
                    return Err(GError::BadSave("CMem zero-run truncated".into()));
                }
                let run = cmem[i] as u32 + 1;
                i += 1;
                for _ in 0..run {
                    if addr >= memsize {
                        return Err(GError::BadSave("CMem data overruns memory".into()));
                    }
                    let base = self.mem.orig_byte(addr);
                    self.mem.write_byte_raw(addr, base);
                    addr += 1;
                }
            } else {
                let base = self.mem.orig_byte(addr);
                self.mem.write_byte_raw(addr, base ^ b);
                addr += 1;
            }
        }
        Ok(())
    }

    /// Load a `UMem` (uncompressed) body into RAM: read the saved memsize,
    /// resize memory, and copy `[RAMSTART, memsize)` literally — no XOR diff,
    /// no RLE (spec §1.8, the `CMem` alternative). Faults on a
    /// truncated/mismatched stream.
    fn load_umem(&mut self, umem: &[u8]) -> Result<(), GError> {
        if umem.len() < 4 {
            return Err(GError::BadSave("UMem chunk too short".into()));
        }
        let memsize = u32::from_be_bytes([umem[0], umem[1], umem[2], umem[3]]);
        let ramstart = self.mem.ramstart();
        if memsize < ramstart || memsize > Self::MAX_MEMSIZE {
            return Err(GError::BadSave("UMem memsize out of range".into()));
        }
        if umem.len() != 4 + (memsize - ramstart) as usize {
            return Err(GError::BadSave("UMem data length disagrees with memsize".into()));
        }
        self.mem.set_raw_size(memsize);
        for (i, addr) in (ramstart..memsize).enumerate() {
            self.mem.write_byte_raw(addr, umem[4 + i]);
        }
        Ok(())
    }

    /// Rebuild the allocation heap from a `MAll` chunk body.
    fn restore_heap(&mut self, mall: &[u8]) -> Result<(), GError> {
        if mall.len() < 8 {
            if mall.is_empty() {
                return Ok(()); // omitted/empty heap → inactive
            }
            return Err(GError::BadSave("MAll chunk too short".into()));
        }
        let rd = |i: usize| u32::from_be_bytes([mall[i], mall[i + 1], mall[i + 2], mall[i + 3]]);
        let heap_start = rd(0);
        let nblocks = rd(4) as usize;
        if mall.len() != 8 + nblocks * 8 {
            return Err(GError::BadSave("MAll block count disagrees with length".into()));
        }
        if heap_start == 0 && nblocks == 0 {
            return Ok(()); // inactive
        }
        // The chunk is untrusted input; accept only a heap the live
        // malloc/free path could have built (SQ-0623). heap_start is the
        // memsize at first-malloc time, so it can never sit below the header
        // ENDMEM floor — a bogus low value would let @mfree truncate memory
        // below RAMSTART (then compress_ram underflows). Blocks must be
        // sorted, non-overlapping, non-empty, and inside [heap_start,
        // memsize]: out-of-order blocks make @malloc's gap walk compute
        // `a - cursor` with a < cursor (debug panic; release wrap →
        // overlapping allocations, silent heap corruption).
        let memsize = self.mem.mem_size() as u64;
        if heap_start < self.mem.endmem() || heap_start as u64 > memsize {
            return Err(GError::BadSave("MAll heap-start out of range".into()));
        }
        let mut blocks = Vec::with_capacity(nblocks.min(1024));
        let mut cursor = heap_start as u64;
        for k in 0..nblocks {
            let base = 8 + k * 8;
            let (a, sz) = (rd(base), rd(base + 4));
            if sz == 0 || (a as u64) < cursor || a as u64 + sz as u64 > memsize {
                return Err(GError::BadSave("MAll blocks unsorted, overlapping, or out of range".into()));
            }
            cursor = a as u64 + sz as u64;
            blocks.push((a, sz));
        }
        self.heap_start = heap_start;
        self.heap_blocks = blocks;
        Ok(())
    }

    // ── undo (GLULX_NOTES §15) ────────────────────────────────────────────────

    /// `saveundo S1`: snapshot the VM state for a later `restoreundo`. A
    /// four-value call stub (the result destination S1, the resume PC, and the
    /// frame pointer) is pushed before snapshotting so `restoreundo` can resume
    /// here; the snapshot is bounded to [`Machine::UNDO_CAP`]. Stores 0 (success).
    fn op_saveundo(&mut self) -> R<()> {
        let (_, s) = self.read_operands(0, 1)?;
        let (dtype, daddr) = s[0].to_stub();
        // Push the call stub (FramePtr ends on top), snapshot, then pop it off.
        self.push32(dtype)?;
        self.push32(daddr)?;
        self.push32(self.pc)?;
        self.push32(self.fp as u32)?;
        let snap = self.save_state();
        self.sp -= 16;

        if self.undo_stack.len() >= Self::UNDO_CAP {
            self.undo_stack.remove(0); // drop the oldest
        }
        self.undo_stack.push(snap);
        self.store(s[0], 0) // success
    }

    /// `restoreundo S1`: pop the newest snapshot and restore it. On success the
    /// snapshot's call stub is consumed and the original `saveundo`'s destination
    /// receives -1 (per the spec, the `saveundo` "returns again"); `restoreundo`
    /// itself stores nothing. With no snapshot, stores 1 (failure), state intact.
    fn op_restoreundo(&mut self) -> R<()> {
        let (_, s) = self.read_operands(0, 1)?;
        let snap = match self.undo_stack.pop() {
            None => return self.store(s[0], 1), // failure
            Some(snap) => snap,
        };
        self.restore_state(&snap).map_err(|e| format!("restoreundo: {e:?}"))?;
        // Consume the four-value call stub the snapshot left on top.
        if self.sp < self.value_base() + 16 {
            return Err("restoreundo: snapshot is missing its call stub".to_string());
        }
        self.sp -= 4;
        let caller_fp = self.st_r32(self.sp);
        self.sp -= 4;
        let ret_pc = self.st_r32(self.sp);
        self.sp -= 4;
        let daddr = self.st_r32(self.sp);
        self.sp -= 4;
        let dtype = self.st_r32(self.sp);
        self.fp = caller_fp as usize;
        self.pc = ret_pc;
        self.reload_frame_meta()?;
        // The original saveundo destination receives -1.
        match dtype {
            0 => Ok(()),
            1 => self.store_mem(daddr, 0xFFFF_FFFF),
            2 => self.local_store(daddr, 0xFFFF_FFFF),
            3 => self.push32(0xFFFF_FFFF),
            other => Err(format!("bad restoreundo stub DestType {other}")),
        }
    }

    // ── restart ───────────────────────────────────────────────────────────────

    /// `@restart`: reset all VM state to its initial condition and re-enter the
    /// start function. The Glk backend is preserved (the display is not cleared by
    /// the spec). No operands.
    fn op_restart(&mut self) -> R<()> {
        let start = self.mem.start_func();
        let decode_table = self.mem.decode_table();
        self.mem.reset_ram();
        self.stack.fill(0);
        self.sp = 0;
        self.fp = 0;
        self.pc = 0;
        self.iosys_mode = 0;
        self.iosys_rock = 0;
        self.cur_stringtbl = decode_table;
        self.heap_start = 0;
        self.heap_blocks.clear();
        // The model reset invalidates every live window id, and the fresh
        // model will hand out the SAME ids again — tell the backend each old
        // window closed first, so a backend keyed by id cannot splice
        // pre-restart window state (grid cells, buffer logs) into the
        // restarted game's windows. The borderless mode is a host/runtime
        // setting, not game state: it survives the reset (SQ-0627).
        for id in self.glk.all_window_ids() {
            self.backend.window_close(id);
        }
        let borderless = self.glk.borderless();
        self.glk = Model::new();
        self.glk.set_borderless(borderless);
        self.pending_input = None;
        self.pending_event = None;
        self.pending_fileref = None;
        self.halted = false;
        self.protect = (0, 0);
        self.undo_stack.clear();
        self.accel_funcs.clear();
        self.accel_params.clear();
        // The PRNG is NOT reset. Snapping back to `DEFAULT_SEED` would throw away
        // the seed the host installed at launch and hand every `@restart` of a
        // randomised game the same opening it gave the last one — the SQ-0811
        // defect, one restart later. zvm's restart leaves its state alone for the
        // same reason, and so does Glulxe.
        self.cur_frame_len = 0;
        self.cur_localspos = 0;
        self.cur_locals.clear();
        // Push the now-empty layout/tree so the backend stops rendering the
        // torn-down windows until the game reopens its own (SQ-0627).
        self.relayout_glk();
        self.build_frame_and_enter(start, &[])
    }

    // ── acceleration storage + PRNG (GLULX_NOTES §17, §18) ────────────────────

    /// The accelerated-function number assigned to the VM function at `addr` via
    /// `accelfunc`, or `None`.
    pub fn accel_func_for(&self, addr: u32) -> Option<u32> {
        self.accel_funcs.get(&addr).copied()
    }

    /// The acceleration parameter stored at `index` via `accelparam`, or `None`.
    pub fn accel_param(&self, index: u32) -> Option<u32> {
        self.accel_params.get(&index).copied()
    }

    /// The full VM-function-address → accelerated-function-number map. Passed to
    /// the disassembler so it can badge accelerated functions in their headers.
    pub fn accel_funcs(&self) -> &std::collections::HashMap<u32, u32> {
        &self.accel_funcs
    }

    /// Borrow the loaded image memory (for the debug disassembler / inspector).
    pub fn mem(&self) -> &Memory {
        &self.mem
    }

    /// PC of the instruction the machine is parked at — the start PC of the last
    /// instruction whose execution began (captured before its operand reads). The
    /// debug inspector uses this as its "current PC" anchor; when the machine is
    /// settled at a Glk input request this is the `glk` instruction that suspended,
    /// which is a real instruction boundary the disassembly can key its divider on.
    pub fn instr_start_pc(&self) -> u32 {
        self.instr_start_pc
    }

    /// The live call stack, innermost (current) frame first — each frame's return
    /// PC, locals, and working value-stack operands. Read-only (never mutates the
    /// machine); the debug inspector renders it, and [`Machine::build_trace`] wraps
    /// it into a crash [`StackTrace`](crate::trace::StackTrace).
    pub fn call_frames(&self) -> Vec<crate::trace::TraceFrame> {
        use crate::trace::TraceFrame;
        let mut frames = Vec::new();
        let mut f = self.fp; // innermost frame offset
        let mut inner_bottom = self.sp; // top of innermost value region
        loop {
            // A frame header needs 8 bytes at [f, f+8) for FrameLen/LocalsPos,
            // and (if not the start frame) a 16-byte call stub at [f-16, f).
            // On a corrupt/attacker-influenced fp, bail out with whatever
            // frames we've collected rather than risk a panic below.
            if f + 8 > self.stack.len() || (f != 0 && f < 16) {
                break;
            }
            let (frame_len, localspos) = match (self.st_r32_opt(f), self.st_r32_opt(f + 4)) {
                (Some(fl), Some(lp)) => (fl as usize, lp as usize),
                _ => break,
            };
            // Walk the locals-format list at f+8 to read each local value.
            let locals = self.read_frame_locals(f, localspos);
            // Value/operand region: above this frame's frame_len, up to inner_bottom.
            let val_lo = f + frame_len;
            let operands = self.read_stack_words(val_lo, inner_bottom);
            let (caller_fp, this_ret_pc) = if f == 0 {
                (0usize, 0u32) // start frame: no stub beneath it
            } else {
                match (self.st_r32_opt(f - 4), self.st_r32_opt(f - 8)) {
                    (Some(caller_fp), Some(ret_pc)) => (caller_fp as usize, ret_pc),
                    _ => break, // corrupt stub: stop, keeping frames collected so far
                }
            };
            frames.push(TraceFrame {
                func_addr: 0, // Glulx does not store per-frame entry addresses
                return_pc: this_ret_pc,
                locals,
                operands,
            });
            if f == 0 {
                break;
            }
            inner_bottom = f.saturating_sub(16); // stub sits at [f-16, f)
            f = caller_fp;
            if frames.len() > 256 {
                break; // guard against a corrupt chain
            }
        }
        frames
    }

    /// Test-only: set an acceleration parameter directly, bypassing `accelparam`.
    #[cfg(test)]
    pub(crate) fn set_accel_param(&mut self, index: u32, value: u32) {
        self.accel_params.insert(index, value);
    }

    /// Test-only: assign an accelerated-function number directly, bypassing `accelfunc`.
    #[cfg(test)]
    pub(crate) fn set_accel_func(&mut self, addr: u32, num: u32) {
        self.accel_funcs.insert(addr, num);
    }

    /// Enable/disable accelerated-function interception (debug escape hatch).
    pub fn set_acceleration(&mut self, on: bool) {
        self.acceleration = on;
    }

    /// Enable/disable Glk graphics windows (gestalt + graphics-window open).
    pub fn set_graphics(&mut self, on: bool) {
        self.graphics_enabled = on;
    }

    /// Whether Glk graphics windows are currently enabled.
    pub fn graphics_enabled(&self) -> bool {
        self.graphics_enabled
    }

    /// Set the per-game borderless-windows mode: `true` makes all window splits
    /// abut with no reserved gutter cell and no reported border (SQ-0341).
    pub fn set_borderless(&mut self, on: bool) {
        self.glk.set_borderless(on);
    }

    /// Enable/disable Glk sound (gestalt + schannel opcodes).
    pub fn set_sound(&mut self, on: bool) {
        self.sound_enabled = on;
    }

    /// The resolved colour a `style` renders in for a `wintype` window, from the
    /// game's `glk_stylehint_set` table. Lets the host paint a window background
    /// that matches a game's own scheme (e.g. a game that styles Normal text
    /// black-on-white for a light interpreter). (SQ-0196)
    pub fn style_colour(&self, wintype: WinType, style: GlkStyle) -> glk::StyleColour {
        self.glk.style_colour(wintype, style)
    }

    /// The Glk file VFS as a standalone sidecar blob, for the host to persist
    /// to disk between sessions (mirrors the Z-machine's aux store). (SQ-0278)
    pub fn vfs_bytes(&self) -> Vec<u8> {
        self.glk.vfs_bytes()
    }

    /// Replace the Glk file VFS from a sidecar blob loaded at story-open; a
    /// corrupt/foreign blob yields an empty VFS. (SQ-0278)
    pub fn load_vfs(&mut self, bytes: &[u8]) {
        self.glk.load_vfs(bytes);
    }

    /// Seed a host-managed `SavedGame` slot's existence + size so a
    /// `create_by_name` game that probes `glk_fileref_does_file_exist` before
    /// `@restore` sees its on-disk save across launches. The host calls this at
    /// boot for each `<game_dir>/<name>.qzl` (raw basename minus `.qzl`) before
    /// `drive_settled`, mirroring the VFS-before-boot ordering. (SQ-0301)
    pub fn seed_saved_game_file(&mut self, name: String, size: u32) {
        self.glk.seed_saved_game_file(name, size);
    }

    /// Whether the Glk file VFS changed since the last [`Machine::clear_vfs_dirty`],
    /// so the host can flush the sidecar only when it changed. (SQ-0278)
    pub fn vfs_dirty(&self) -> bool {
        self.glk.vfs_dirty()
    }

    /// Clear the VFS-dirty flag after a host flush or the initial load. (SQ-0278)
    pub fn clear_vfs_dirty(&mut self) {
        self.glk.clear_vfs_dirty();
    }

    /// Total number of opcodes dispatched since the machine was built.
    /// Accelerated calls bypass the opcode dispatcher, so this undercounts
    /// work done by intercepted functions when acceleration is enabled.
    pub fn insn_count(&self) -> u64 {
        self.insn_count
    }

    /// Seed the PRNG (Glulx spec §2.14, the `setrandom` semantics). Call this
    /// BEFORE the boot drive: a game's initialisation may already draw from it,
    /// so seeding after the first `glk_select` is too late to change the game the
    /// player is handed. `0` is coerced to [`Self::DEFAULT_SEED`] — xorshift32 is
    /// an absorbing state at zero.
    pub fn set_rng_seed(&mut self, seed: u32) {
        self.rng = if seed == 0 { Self::DEFAULT_SEED } else { seed };
    }

    /// The current PRNG state — the seed the next `random` draw advances from.
    /// Diagnostic only (the startup seed report); the game never sees it.
    pub fn rng_seed(&self) -> u32 {
        self.rng
    }

    /// A best-effort entropy seed for `setrandom(0)`, drawn from std's
    /// OS-seeded `RandomState` so gvm stays dependency-free and cross-platform.
    /// Guaranteed nonzero so xorshift32 can never get stuck at 0.
    fn entropy_seed() -> u32 {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u32(0x9E37_79B9); // fixed salt; the entropy lives in the random keys
        let s = h.finish() as u32;
        if s == 0 { Self::DEFAULT_SEED } else { s }
    }

    /// Advance the xorshift32 PRNG and return the next 32-bit value.
    fn next_rand(&mut self) -> u32 {
        let mut x = if self.rng == 0 { Self::DEFAULT_SEED } else { self.rng };
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// `random(L1)` per the spec: `[0, L1)` for `L1 > 0`, `(L1, 0]` for `L1 < 0`,
    /// any 32-bit value for `L1 == 0`.
    fn rand_range(&mut self, l1: u32) -> u32 {
        let r = self.next_rand();
        let n = l1 as i32;
        if n == 0 {
            r
        } else if n > 0 {
            r % n as u32
        } else {
            // (L1, 0] → -(r mod |L1|), so values from L1+1 up to 0.
            0i32.wrapping_sub((r % n.unsigned_abs()) as i32) as u32
        }
    }

    // ── allocation heap (GLULX_NOTES §11) ─────────────────────────────────────

    /// Allocate `size` bytes in the heap, returning the block address or 0 on
    /// failure. The first allocation activates the heap at the current memsize.
    fn heap_malloc(&mut self, size: u32) -> u32 {
        if size == 0 {
            return 0; // malloc requires a positive size
        }
        let heap_start = if self.heap_start == 0 { self.mem.mem_size() } else { self.heap_start };

        // Walk the free gaps from heap_start, reusing the first that fits.
        let mut cursor = heap_start;
        let mut addr = None;
        for &(a, sz) in &self.heap_blocks {
            if a - cursor >= size {
                addr = Some(cursor);
                break;
            }
            cursor = a + sz;
        }
        // No internal gap fit: append at the end of the last block (or
        // heap_start if empty), reusing committed tail space and growing only
        // if necessary.
        let addr = addr.unwrap_or(cursor);

        let top = addr as u64 + size as u64;
        if top > Self::MAX_MEMSIZE as u64 {
            return 0; // would exceed the heap ceiling
        }
        // Grow the map to fit the block, keeping ENDMEM a multiple of 256 (the
        // spec invariant; see GLULX_NOTES §1). The alignment slack also lets
        // Inform's memory-stream idiom write its result struct at buf+len, one
        // word past the block, without faulting — as on other interpreters.
        let top = (top as u32).next_multiple_of(256);
        if top > self.mem.mem_size() {
            self.mem.set_raw_size(top);
        }
        // Insert keeping the block list sorted by address.
        let pos = self.heap_blocks.partition_point(|&(a, _)| a < addr);
        self.heap_blocks.insert(pos, (addr, size));
        self.heap_start = heap_start;
        addr
    }

    /// Free the extant block at `addr`; faults if it is not a current block.
    /// Freeing the last block deactivates the heap and shrinks memory back to
    /// the heap-start address.
    fn heap_free(&mut self, addr: u32) -> R<()> {
        let pos = self
            .heap_blocks
            .iter()
            .position(|&(a, _)| a == addr)
            .ok_or_else(|| format!("mfree of non-extant block @{addr:#010x}"))?;
        self.heap_blocks.remove(pos);
        if self.heap_blocks.is_empty() {
            self.mem.set_raw_size(self.heap_start);
            self.heap_start = 0;
        }
        Ok(())
    }

    // ── search opcodes (GLULX_NOTES §12) ──────────────────────────────────────

    const KEY_INDIRECT: u32 = 0x01;
    const ZERO_KEY_TERMINATES: u32 = 0x02;
    const RETURN_INDEX: u32 = 0x04;

    /// Build the search key as a byte vector: either read indirectly from the
    /// key address, or take the low `size` bytes of the immediate value.
    fn search_key(&self, key: u32, size: u32, options: u32) -> R<Vec<u8>> {
        if options & Self::KEY_INDIRECT != 0 {
            self.read_bytes(key, size)
        } else {
            match size {
                1 | 2 | 4 => Ok(key.to_be_bytes()[(4 - size as usize)..].to_vec()),
                _ => Err(format!("search: KeySize {size} must be 1, 2, or 4 for a direct key")),
            }
        }
    }

    /// Read `n` bytes from main memory into a vector (bounds-checked).
    fn read_bytes(&self, addr: u32, n: u32) -> R<Vec<u8>> {
        let mut v = Vec::with_capacity(cap_hint(n));
        for i in 0..n {
            v.push(self.m8(addr + i)? as u8);
        }
        Ok(v)
    }

    fn op_linearsearch(&mut self) -> R<()> {
        let (l, s) = self.read_operands(7, 1)?;
        let (key, key_size, start, struct_size, num, key_off, opts) =
            (l[0], l[1], l[2], l[3], l[4], l[5], l[6]);
        let want = self.search_key(key, key_size, opts)?;
        let mut found: Option<(u32, u32)> = None; // (addr, index)
        let mut index = 0u32;
        loop {
            if num != 0xFFFF_FFFF && index >= num {
                break;
            }
            let saddr = start.wrapping_add(index.wrapping_mul(struct_size));
            let have = self.read_bytes(saddr + key_off, key_size)?;
            if have == want {
                found = Some((saddr, index));
                break;
            }
            if opts & Self::ZERO_KEY_TERMINATES != 0 && have.iter().all(|&b| b == 0) {
                break;
            }
            index += 1;
        }
        let r = Self::search_result(found, opts);
        self.store(s[0], r)
    }

    fn op_binarysearch(&mut self) -> R<()> {
        let (l, s) = self.read_operands(7, 1)?;
        let (key, key_size, start, struct_size, num, key_off, opts) =
            (l[0], l[1], l[2], l[3], l[4], l[5], l[6]);
        let want = self.search_key(key, key_size, opts)?;
        let mut found: Option<(u32, u32)> = None;
        let (mut lo, mut hi) = (0u32, num);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let saddr = start.wrapping_add(mid.wrapping_mul(struct_size));
            let have = self.read_bytes(saddr + key_off, key_size)?;
            match have.cmp(&want) {
                std::cmp::Ordering::Equal => {
                    found = Some((saddr, mid));
                    break;
                }
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        let r = Self::search_result(found, opts);
        self.store(s[0], r)
    }

    fn op_linkedsearch(&mut self) -> R<()> {
        let (l, s) = self.read_operands(6, 1)?;
        let (key, key_size, start, key_off, next_off, opts) =
            (l[0], l[1], l[2], l[3], l[4], l[5]);
        let want = self.search_key(key, key_size, opts)?;
        let mut node = start;
        let mut result = 0u32;
        while node != 0 {
            let have = self.read_bytes(node + key_off, key_size)?;
            if have == want {
                result = node;
                break;
            }
            if opts & Self::ZERO_KEY_TERMINATES != 0 && have.iter().all(|&b| b == 0) {
                break;
            }
            node = self.m32(node + next_off)?;
        }
        self.store(s[0], result)
    }

    /// Map a found `(addr, index)` (or `None`) to the result value per the
    /// ReturnIndex option.
    fn search_result(found: Option<(u32, u32)>, options: u32) -> u32 {
        match (found, options & Self::RETURN_INDEX != 0) {
            (Some((_, idx)), true) => idx,
            (Some((addr, _)), false) => addr,
            (None, true) => 0xFFFF_FFFF,
            (None, false) => 0,
        }
    }

    // ── stack-manipulation opcodes ────────────────────────────────────────────

    fn op_stkpeek(&mut self) -> R<()> {
        let (l, s) = self.read_operands(1, 1)?;
        let i = l[0];
        if i >= self.value_count() {
            return Err(format!("stkpeek index {i} out of range"));
        }
        let off = self.sp - 4 * (i as usize + 1);
        let v = self.st_r32(off);
        self.store(s[0], v)
    }

    fn op_stkswap(&mut self) -> R<()> {
        if self.value_count() < 2 {
            return Err("stkswap underflow".to_string());
        }
        let (a, b) = (self.sp - 4, self.sp - 8);
        let (va, vb) = (self.st_r32(a), self.st_r32(b));
        self.st_w32(a, vb);
        self.st_w32(b, va);
        Ok(())
    }

    fn op_stkcopy(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        let n = l[0] as usize;
        if l[0] > self.value_count() {
            return Err("stkcopy out of range".to_string());
        }
        let base = self.sp;
        for k in 0..n {
            let v = self.st_r32(base - 4 * n + 4 * k);
            self.push32(v)?;
        }
        Ok(())
    }

    fn op_stkroll(&mut self) -> R<()> {
        let (l, _) = self.read_operands(2, 0)?;
        let count = l[0];
        let places = l[1] as i32;
        if count > self.value_count() {
            return Err("stkroll out of range".to_string());
        }
        if count == 0 {
            return Ok(());
        }
        let count = count as usize;
        // Positive places rotate "up" (topmost moves deeper) → rotate_right on the
        // bottom→top ordering.
        let eff = (((places % count as i32) + count as i32) % count as i32) as usize;
        let base = self.sp - 4 * count;
        let mut vals: Vec<u32> = (0..count).map(|k| self.st_r32(base + 4 * k)).collect();
        vals.rotate_right(eff);
        for (k, v) in vals.iter().enumerate() {
            self.st_w32(base + 4 * k, *v);
        }
        Ok(())
    }

    // ── call variants ─────────────────────────────────────────────────────────

    /// Pop `argc` call arguments (first arg topmost) into the reused
    /// `call_args_scratch` buffer, returning it for the caller to hand back
    /// via [`Machine::return_args_scratch`] once the call setup is done.
    fn pop_args_scratch(&mut self, argc: u32) -> R<Vec<u32>> {
        let mut args = std::mem::take(&mut self.call_args_scratch);
        args.clear();
        for _ in 0..argc {
            match self.pop32() {
                Ok(v) => args.push(v),
                Err(e) => {
                    self.call_args_scratch = args;
                    return Err(e);
                }
            }
        }
        Ok(args)
    }

    /// Put the buffer from [`Machine::pop_args_scratch`] back for reuse.
    fn return_args_scratch(&mut self, args: Vec<u32>) {
        self.call_args_scratch = args;
    }

    fn op_call(&mut self) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let (func, argc) = (l[0], l[1]);
        let args = self.pop_args_scratch(argc)?;
        let res = self.call_function(func, &args, s[0]);
        self.return_args_scratch(args);
        res
    }

    fn op_callf(&mut self, nargs: usize) -> R<()> {
        let (l, s) = self.read_operands(1 + nargs, 1)?;
        self.call_function(l[0], &l[1..], s[0])
    }

    fn op_tailcall(&mut self) -> R<()> {
        let (l, _) = self.read_operands(2, 0)?;
        let (func, argc) = (l[0], l[1]);
        let args = self.pop_args_scratch(argc)?;
        let res = self.tailcall_with(func, &args);
        self.return_args_scratch(args);
        res
    }

    fn tailcall_with(&mut self, func: u32, args: &[u32]) -> R<()> {
        if self.acceleration {
            if let Some(num) = self.accel_func_for(func) {
                if crate::accel::accel_impl_supported(num) {
                    let result = self.accel_dispatch(num, args)?;
                    return self.return_value(result);
                }
            }
        }
        // Destroy the current frame but keep the call stub beneath it, so the new
        // function returns to the current function's caller.
        self.sp = self.fp;
        self.build_frame_and_enter(func, args)
    }

    // ── stream output (GLULX_NOTES §7) ────────────────────────────────────────

    /// Route `streamchar`/`streamnum`/`streamstr` output, honoring the current
    /// I/O system: the Glk system (mode 2) prints to the current Glk stream;
    /// the filter system (mode 1) calls `iosys_rock` once per character, with
    /// that character's code point as the sole argument (GLULX_NOTES §7.2);
    /// the null system (mode 0, and any unrecognized mode) discards.
    fn emit(&mut self, s: &str) {
        // Capturing (decoding an E1 object for glk_put_string): divert output into
        // the buffer instead of routing it through the I/O system.
        if let Some(buf) = self.emit_capture.as_mut() {
            buf.push_str(s);
            return;
        }
        match self.iosys_mode {
            2 => {
                let sid = self.glk.current_stream();
                self.glk_stream_put(sid, s);
            }
            1 => {
                // Guard against a filter function whose own output recurses
                // back into `emit` (e.g. via streamchar): each nested level
                // costs several native frames (this call chain runs a full
                // nested opcode-dispatch loop), so the bound is much lower
                // than the lighter-weight string-decode recursion guard.
                const FILTER_MAX_DEPTH: u32 = 32;
                if self.filter_depth > FILTER_MAX_DEPTH {
                    return;
                }
                self.filter_depth += 1;
                let mut chars = s.chars();
                while let Some(ch) = chars.next() {
                    if self.iosys_mode != 1 {
                        // The filter switched the I/O system mid-string (e.g.
                        // via @setiosys): hand the remaining characters off
                        // under the now-current mode instead of continuing to
                        // call the stale filter rock.
                        self.filter_depth -= 1;
                        let rest: String = std::iter::once(ch).chain(chars).collect();
                        return self.emit(&rest);
                    }
                    if let Err(e) = self.run_call_to_return(self.iosys_rock, &[ch as u32]) {
                        // The filter function faulted: its frame was abandoned
                        // mid-flight, so the machine must not keep printing or
                        // executing. Latch the fault (first one wins) —
                        // `step`/`run_call_to_return` surface it as this
                        // instruction's fault (SQ-0625).
                        self.pending_fault.get_or_insert(e);
                        self.filter_depth -= 1;
                        return;
                    }
                    if self.pending_fault.is_some() {
                        // A nested emit latched a fault below us: stop too.
                        self.filter_depth -= 1;
                        return;
                    }
                }
                self.filter_depth -= 1;
            }
            _ => {} // null system (mode 0) and unrecognized modes: discard
        }
    }

    /// Write `s` to Glk stream `sid` (its current style). A window stream routes
    /// to that window via the backend; a memory stream writes Glulx memory; an
    /// invalid/zero stream is safely discarded (no panic). This is the single
    /// output funnel for both the `@glk` put selectors and the stream opcodes.
    fn glk_stream_put(&mut self, sid: u32, s: &str) {
        // While capturing an E1 decode for glk_put_string, direct Glk output from
        // an embedded function-node lands in the buffer too, in decode order,
        // rather than escaping to a real stream mid-decode. (`emit` returns before
        // reaching here during capture, so string-machinery output isn't
        // double-counted — only direct Glk put selectors funnel through this.)
        if let Some(buf) = self.emit_capture.as_mut() {
            buf.push_str(s);
            return;
        }
        if sid == 0 {
            return; // no current stream → discard
        }
        let Some((kind, style, link)) = self.glk.stream_kind_style(sid) else {
            return; // bad stream id → discard
        };
        match kind {
            StreamKind::Window(win) => {
                if self.trace_screen {
                    let ty = match self.glk.window_type(win) {
                        Some(WinType::TextBuffer) => "buf",
                        Some(WinType::TextGrid) => "grid",
                        _ => "?",
                    };
                    self.trace_text_append(win, ty, s);
                }
                match self.glk.window_type(win) {
                    Some(WinType::TextBuffer) => {
                        let colour = self.glk.window_stream_style_colour(win, sid, style);
                        let attrs = self.glk.style_attrs(WinType::TextBuffer, style);
                        self.backend.put_text_attr(win, style, colour, attrs, link, s);
                    }
                    Some(WinType::TextGrid) => self.grid_put_str(sid, win, style, link, s),
                    _ => {} // pair window or stale: nothing to display
                }
                self.glk.window_stream_advance(sid, s.chars().count() as u32);
                // Echo: text printed to a window is also written to its echo
                // stream (Glk spec §3.6). `window_set_echo_stream` refuses a
                // self-loop; `echo_depth` bounds any longer loop the game builds.
                let echo = self.glk.window_echo_stream(win);
                if echo != 0 && echo != sid && self.echo_depth < 8 {
                    self.echo_depth += 1;
                    self.glk_stream_put(echo, s);
                    self.echo_depth -= 1;
                }
            }
            StreamKind::Memory { addr, len, pos, unicode, .. } => {
                let elsize = if unicode { 4 } else { 1 };
                let mut p = pos;
                for ch in s.chars() {
                    if p < len {
                        let ea = addr + p * elsize;
                        let v = ch as u32;
                        if unicode {
                            let _ = self.store_mem_sized(ea, v, 4);
                        } else {
                            // Byte stream: a code point > 0xFF stores as '?'
                            // (0x3F), not its low byte (Glk spec §5.4).
                            let _ = self.store_mem_sized(ea, if v > 0xFF { 0x3F } else { v }, 1);
                        }
                    }
                    p = p.saturating_add(1);
                }
                self.glk.memory_stream_advance(sid, s.chars().count() as u32);
            }
            StreamKind::File { .. } => self.glk.file_stream_write(sid, s),
            // A SavedGame host conduit: writes discard (the `@save` shim credits
            // its write count separately via `note_stream_write`).
            StreamKind::Null => {}
            // A resource stream is read-only: writes are silently discarded.
            StreamKind::Resource { .. } => {}
        }
    }

    /// Write `s` to a text-grid window starting at its cursor, advancing the
    /// cursor and wrapping at the window edge (output past the bottom is
    /// discarded). `\n` moves to the next row, column 0.
    fn grid_put_str(&mut self, sid: u32, win: u32, style: GlkStyle, link: u32, s: &str) {
        let Some((w, h, mut cx, mut cy)) = self.glk.grid_state(win) else { return };
        let colour = self.glk.window_stream_style_colour(win, sid, style);
        let attrs = self.glk.style_attrs(WinType::TextGrid, style);
        for ch in s.chars() {
            if ch == '\n' {
                cx = 0;
                cy += 1;
                continue;
            }
            if cx >= w {
                cx = 0;
                cy += 1;
            }
            if cy < h && cx < w {
                self.backend.grid_put_attr(win, cx, cy, style, colour, attrs, link, &ch.to_string());
            }
            cx += 1;
        }
        self.glk.set_grid_cursor(win, cx, cy);
    }

    fn op_streamchar(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        let s = ((l[0] & 0xFF) as u8 as char).to_string();
        self.emit(&s);
        Ok(())
    }

    fn op_streamunichar(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        let s = char::from_u32(l[0]).unwrap_or('\u{FFFD}').to_string();
        self.emit(&s);
        Ok(())
    }

    fn op_streamnum(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        let s = (l[0] as i32).to_string();
        self.emit(&s);
        Ok(())
    }

    fn op_streamstr(&mut self) -> R<()> {
        let (l, _) = self.read_operands(1, 0)?;
        self.print_object(l[0], &[], 0)
    }

    /// Print a "typable object" at `addr`: a string (E0/E1/E2) is decoded and
    /// streamed; a function (C0/C1) is called with `args` and its output
    /// streamed in its place. `depth` bounds indirect/recursive references.
    fn print_object(&mut self, addr: u32, args: &[u32], depth: u32) -> R<()> {
        if depth > 256 {
            return Err(format!("string decode recursion too deep @{addr:#010x}"));
        }
        match self.m8(addr)? {
            0xE0 => {
                let s = self.read_cstring(addr + 1)?;
                self.emit(&s);
                Ok(())
            }
            0xE2 => {
                let s = self.read_ustring(addr + 4)?;
                self.emit(&s);
                Ok(())
            }
            0xE1 => self.decode_compressed(addr + 1, depth),
            0xC0 | 0xC1 => self.run_call_to_return(addr, args),
            other => Err(format!("bad string/function type {other:#x} @{addr:#010x}")),
        }
    }

    /// Decode a compressed (E1) bit stream beginning at `start`, walking the
    /// current string-decoding table. Bits are read low-bit-first (GLULX_NOTES
    /// §9). Never panics: bad addresses/node types fault.
    fn decode_compressed(&mut self, start: u32, depth: u32) -> R<()> {
        let table = self.cur_stringtbl;
        if table == 0 {
            return Err("compressed string (E1) with no string-decoding table set".to_string());
        }
        let root = self.m32(table + 8)?;
        let mut node = root;
        let mut addr = start;
        let mut bit = 0u32;
        loop {
            match self.m8(node)? {
                0x00 => {
                    // Branch: read one bit, go left (0) or right (1).
                    let byte = self.m8(addr)?;
                    let b = (byte >> bit) & 1;
                    bit += 1;
                    if bit == 8 {
                        bit = 0;
                        addr += 1;
                    }
                    node = if b == 0 { self.m32(node + 1)? } else { self.m32(node + 5)? };
                }
                0x01 => return Ok(()), // string terminator
                0x02 => {
                    let c = self.m8(node + 1)?;
                    self.emit_latin1(c);
                    node = root;
                }
                0x03 => {
                    let s = self.read_cstring(node + 1)?;
                    self.emit(&s);
                    node = root;
                }
                0x04 => {
                    let cp = self.m32(node + 1)?;
                    self.emit_uni(cp);
                    node = root;
                }
                0x05 => {
                    // C-style Unicode string: 32-bit chars until a zero word.
                    let s = self.read_ustring(node + 1)?;
                    self.emit(&s);
                    node = root;
                }
                0x08 => {
                    let a = self.m32(node + 1)?;
                    self.print_object(a, &[], depth + 1)?;
                    node = root;
                }
                0x09 => {
                    let a = self.m32(self.m32(node + 1)?)?;
                    self.print_object(a, &[], depth + 1)?;
                    node = root;
                }
                0x0A => {
                    let a = self.m32(node + 1)?;
                    let args = self.read_node_args(node + 5)?;
                    self.print_object(a, &args, depth + 1)?;
                    node = root;
                }
                0x0B => {
                    let a = self.m32(self.m32(node + 1)?)?;
                    let args = self.read_node_args(node + 5)?;
                    self.print_object(a, &args, depth + 1)?;
                    node = root;
                }
                other => return Err(format!("bad string node type {other:#x} @{node:#010x}")),
            }
        }
    }

    /// Read an argument list for a 0x0A/0x0B node: a 32-bit count then that many
    /// 32-bit arguments.
    fn read_node_args(&self, at: u32) -> R<Vec<u32>> {
        let argc = self.m32(at)?;
        let mut args = Vec::with_capacity(cap_hint(argc));
        for i in 0..argc {
            args.push(self.m32(at + 4 + 4 * i)?);
        }
        Ok(args)
    }

    /// Call the function at `func` with `args`, running the VM run-loop until
    /// that frame returns, then resume the caller. Used for string-embedded
    /// function nodes (GLULX_NOTES §9); the return value is discarded.
    fn run_call_to_return(&mut self, func: u32, args: &[u32]) -> R<()> {
        let resume_fp = self.fp;
        self.call_function(func, args, Dest::Discard)?;
        // call_function installed a deeper callee frame; run until it returns.
        while self.fp != resume_fp {
            if self.halted {
                return Err("function called within a string halted the machine".to_string());
            }
            self.step_once()?;
            if let Some(fault) = self.pending_fault.take() {
                // A filter callback nested under this frame faulted (latched
                // by `emit`, which has no error channel): stop running in the
                // abandoned state and propagate (SQ-0625).
                return Err(fault);
            }
        }
        Ok(())
    }

    fn emit_latin1(&mut self, v: u32) {
        let s = ((v & 0xFF) as u8 as char).to_string();
        self.emit(&s);
    }
    fn emit_uni(&mut self, v: u32) {
        let s = char::from_u32(v).unwrap_or('\u{FFFD}').to_string();
        self.emit(&s);
    }

    /// Decode a Glulx string OBJECT at `addr` into a String for the Glk
    /// `put_string`/`put_string_uni` dispatch arms. The Glk string argument is a
    /// type-tagged Inform string (the reference decodes it via `DecodeVMString`),
    /// not a raw C array: `0xE0` unencoded Latin-1, `0xE2` unencoded Unicode, and
    /// `0xE1` compressed. E0/E2 read directly; a compressed E1 object is decoded
    /// through the string machinery with `emit`/`glk_stream_put` diverted into a
    /// capture buffer (so embedded string- and function-node output is captured in
    /// order and bypasses the VM iosys, like E0/E2), then returned for the caller
    /// to write to the Glk stream. An E1 decode failure (no string table set, a
    /// bad node, or a memory fault while decoding) records a diagnostic and yields
    /// `None` rather than faulting the VM; for E0/E2, a genuine memory fault from
    /// `m8`/the readers still propagates. A malformed tag yields `None` with a
    /// diagnostic. Returns `Ok(None)` when nothing should be emitted.
    fn decode_glk_string(&mut self, addr: u32) -> R<Option<String>> {
        Ok(match self.m8(addr)? {
            0xE0 => Some(self.read_cstring(addr + 1)?),
            0xE2 => Some(self.read_ustring(addr + 4)?),
            0xE1 => {
                // Compressed object: decode it through the string machinery
                // (which handles embedded string- and function-nodes), capturing
                // all output into a buffer, then return that so the caller writes
                // it straight to the Glk stream like E0/E2. A decode failure
                // (e.g. no string table set, or a bad node) records a diagnostic
                // and skips rather than faulting the VM.
                let prev = self.emit_capture.replace(String::new());
                let decoded = self.decode_compressed(addr + 1, 0);
                let captured = self.emit_capture.take().unwrap_or_default();
                self.emit_capture = prev;
                match decoded {
                    Ok(()) => Some(captured),
                    Err(e) => {
                        self.diagnostics.push(format!(
                            "glk_put_string: compressed (E1) string @{addr:#010x} decode failed: {e}"
                        ));
                        None
                    }
                }
            }
            other => {
                self.diagnostics.push(format!(
                    "glk_put_string: bad string type {other:#x} @{addr:#010x}; skipped"
                ));
                None
            }
        })
    }

    /// Read a zero-terminated Latin-1 string from main memory.
    fn read_cstring(&self, mut addr: u32) -> R<String> {
        let mut s = String::new();
        loop {
            let b = self.m8(addr)?;
            if b == 0 {
                break;
            }
            s.push(b as u8 as char);
            addr += 1;
        }
        Ok(s)
    }

    /// Read a zero-terminated array of big-endian 32-bit Unicode code points.
    fn read_ustring(&self, mut addr: u32) -> R<String> {
        let mut s = String::new();
        loop {
            let cp = self.m32(addr)?;
            if cp == 0 {
                break;
            }
            s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
            addr += 4;
        }
        Ok(s)
    }

    // ── @glk dispatch (GLULX_NOTES §19) ───────────────────────────────────────

    fn op_glk(&mut self) -> R<()> {
        let (l, s) = self.read_operands(2, 1)?;
        let (selector, argc) = (l[0], l[1]);
        let mut args = Vec::with_capacity(cap_hint(argc));
        for _ in 0..argc {
            args.push(self.pop32()?); // first @glk arg is topmost
        }
        let result = self.glk_dispatch(selector, &args)?;
        // glk_fileref_create_by_prompt suspends: the real fileref id is produced
        // later by supply_filename, which stores it into this @glk's S1. Capture the
        // destination and do not store the placeholder result now.
        if let Some(pf) = self.pending_fileref.as_mut() {
            pf.dest = s[0];
            return Ok(());
        }
        self.store(s[0], result)
    }

    /// For selectors that return their result through out-pointers (not the
    /// return value), read the values back from memory after the call for the
    /// trace's `-> …` suffix. Skips the Glulx dispatch sentinels (`0` = value not
    /// wanted, `-1` = written to the stack), showing `?` for those. (SQ-0405)
    fn glk_out_result(&self, sel: u32, args: &[u32]) -> Option<String> {
        let a = |i: usize| args.get(i).copied().unwrap_or(0);
        let rd = |addr: u32| -> Option<u32> {
            if addr == 0 || addr == 0xffff_ffff { None } else { self.mem.read32(addr) }
        };
        let num = |o: Option<u32>| o.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
        match sel {
            // glk_window_get_size(win, awidthptr, aheightptr)
            0x0025 => Some(format!("{}×{}", num(rd(a(1))), num(rd(a(2))))),
            _ => None,
        }
    }

    /// Coalesce a run of story text going to `win` (of type `ty`) for the trace,
    /// so a stream of per-char puts becomes one readable line. (SQ-0405)
    fn trace_text_append(&mut self, win: u32, ty: &'static str, s: &str) {
        if self.text_run.as_ref().is_some_and(|(w, _, _)| *w != win) {
            self.flush_text_run();
        }
        let run = self.text_run.get_or_insert_with(|| (win, ty, String::new()));
        run.2.push_str(s);
        if run.2.len() > 4096 {
            self.flush_text_run();
        }
    }

    /// Emit any pending coalesced text run as a `win N [ty] <- "…"` trace line.
    fn flush_text_run(&mut self) {
        if let Some((win, ty, buf)) = self.text_run.take() {
            if !buf.is_empty() {
                let preview: String = buf.chars().take(200).collect();
                self.screen_trace.push(format!("win {win} [{ty}] <- {preview:?}"));
            }
        }
    }

    /// Dispatch one `@glk` selector against the Glk model + backend. Output-side
    /// selectors only (input/events are phase 3a-2). Unknown selectors record a
    /// diagnostic and return 0; nothing here panics on bad ids.
    fn glk_dispatch(&mut self, selector: u32, args: &[u32]) -> R<u32> {
        let a = |i: usize| args.get(i).copied().unwrap_or(0);
        // Debug trace: record structural Glk/garglk calls so a story's
        // window/style/colour instructions are visible. The high-volume text I/O
        // (put/get char/string/buffer) is skipped so the trace stays readable.
        // Flush any pending story-text run before a non-text (structural) call, so
        // the `win N [ty] <- "…"` line lands in order ahead of this call. (SQ-0405)
        if self.trace_screen && !matches!(selector, 0x0080..=0x0085 | 0x0128..=0x012D) {
            self.flush_text_run();
        }
        // Captured now (args), pushed after the result is known so the return
        // value can ride on the same line as `-> …`. (SQ-0405)
        let trace_call = (self.trace_screen && !is_glk_text_io(selector))
            .then(|| format!("{}({})", glk_selector_name(selector), glk_trace_args(selector, args)));
        let ret = match selector {
            // ── output (put) ──────────────────────────────────────────────────
            0x0080 => {
                // glk_put_char(ch)
                let s = ((a(0) & 0xFF) as u8 as char).to_string();
                self.glk_put_current(&s);
                0
            }
            0x0081 => {
                // glk_put_char_stream(str, ch)
                let s = ((a(1) & 0xFF) as u8 as char).to_string();
                self.glk_stream_put(a(0), &s);
                0
            }
            0x0082 => {
                // glk_put_string(addr): addr is a type-tagged Inform string object.
                if let Some(s) = self.decode_glk_string(a(0))? {
                    self.glk_put_current(&s);
                }
                0
            }
            0x0083 => {
                // glk_put_string_stream(str, addr)
                if let Some(s) = self.decode_glk_string(a(1))? {
                    self.glk_stream_put(a(0), &s);
                }
                0
            }
            0x0084 => {
                // glk_put_buffer(addr, len)
                let s = self.read_latin1_buffer(a(0), a(1))?;
                self.glk_put_current(&s);
                0
            }
            0x0085 => {
                // glk_put_buffer_stream(str, addr, len)
                let s = self.read_latin1_buffer(a(1), a(2))?;
                self.glk_stream_put(a(0), &s);
                0
            }
            0x0128 => {
                // glk_put_char_uni(ch)
                let s = char::from_u32(a(0)).unwrap_or('\u{FFFD}').to_string();
                self.glk_put_current(&s);
                0
            }
            0x0129 => {
                // glk_put_string_uni(addr): addr is a type-tagged Inform string object.
                if let Some(s) = self.decode_glk_string(a(0))? {
                    self.glk_put_current(&s);
                }
                0
            }
            0x012A => {
                // glk_put_buffer_uni(addr, len)
                let s = self.read_uni_buffer(a(0), a(1))?;
                self.glk_put_current(&s);
                0
            }
            0x012B => {
                // glk_put_char_stream_uni(str, ch)
                let s = char::from_u32(a(1)).unwrap_or('\u{FFFD}').to_string();
                self.glk_stream_put(a(0), &s);
                0
            }
            0x012C => {
                // glk_put_string_stream_uni(str, addr)
                if let Some(s) = self.decode_glk_string(a(1))? {
                    self.glk_stream_put(a(0), &s);
                }
                0
            }
            0x012D => {
                // glk_put_buffer_stream_uni(str, addr, len)
                let s = self.read_uni_buffer(a(1), a(2))?;
                self.glk_stream_put(a(0), &s);
                0
            }
            // ── windows ───────────────────────────────────────────────────────
            0x0020 => {
                // glk_window_iterate(win, rockptr) -> next window id
                let (next, rock) = self.glk.window_iterate(a(0));
                self.glk_store_ptr(a(1), rock)?;
                next
            }
            0x0021 => self.glk.window_rock(a(0)).unwrap_or(0), // glk_window_get_rock
            0x0022 => self.glk.root(),                         // glk_window_get_root
            0x0023 => self.glk_open_window(a(0), a(1), a(2), a(3), a(4)), // glk_window_open
            0x0024 => {
                // glk_window_close(win, streamresultptr{readcount, writecount})
                match self.glk.window_close(a(0)) {
                    Some((r, w)) => {
                        self.glk_out_ref(a(1), &[r, w])?;
                        self.backend.window_close(a(0));
                        self.relayout_glk();
                    }
                    None => self.diagnostics.push(format!("glk_window_close: bad window {}", a(0))),
                }
                0
            }
            0x0025 => {
                // glk_window_get_size(win, awidthptr, aheightptr) — PIXELS for a
                // graphics window, cells (unchanged) for other window types.
                let cp = self.backend.char_pixels();
                let size = self.glk.window_pixel_size(a(0), cp).or_else(|| self.glk.window_size(a(0)));
                if let Some((w, h)) = size {
                    self.glk_store_ptr(a(1), w)?;
                    self.glk_store_ptr(a(2), h)?;
                }
                0
            }
            0x0026 => {
                // glk_window_set_arrangement(win, method, size, keywin)
                self.glk.window_set_arrangement(a(0), a(1), a(2), a(3));
                self.relayout_glk();
                // A program-driven rearrangement must NOT synthesize an
                // evtype_Arrange event: per the Glk spec, Arrange notifies the game
                // of EXTERNAL display-size changes (the player resizing), not the
                // game's own set_arrangement. Emitting one made any game whose
                // arrange handler re-arranges its windows spin forever — it re-lays
                // out, we queue another Arrange, glk_select returns it, repeat (The
                // Wizard Sniffer's status-window auto-sizer, SQ-0272). A Redraw is
                // still warranted for a graphics window whose pixels the rearrange
                // may have resized.
                if self.glk.has_graphics_window() {
                    self.glk.push_event(GlkEvent { etype: glk::evtype::REDRAW, win: 0, val1: 0, val2: 0 });
                }
                0
            }
            0x0027 => {
                // glk_window_get_arrangement(win, methodptr, sizeptr, keywinptr)
                if let Some((m, s, k)) = self.glk.window_arrangement(a(0)) {
                    self.glk_store_ptr(a(1), m)?;
                    self.glk_store_ptr(a(2), s)?;
                    self.glk_store_ptr(a(3), k)?;
                }
                0
            }
            0x0028 => self.glk.window_type(a(0)).map(|t| t.to_arg()).unwrap_or(0), // get_type
            0x0029 => self.glk.window_parent(a(0)).unwrap_or(0),                   // get_parent
            0x002A => {
                // glk_window_clear(win)
                match self.glk.window_clear(a(0)) {
                    Some(WinType::TextGrid) => self.backend.grid_clear(a(0)),
                    Some(WinType::TextBuffer) => self.backend.window_clear(a(0)),
                    // A graphics window clears to its background colour
                    // (glk_window_set_background_color): erase the whole canvas to
                    // that colour. Without this the clear was a no-op, so a game
                    // that set a black background (Counterfeit Monkey's help)
                    // rendered as the transparent default → the white page showed
                    // through. (SQ-0403)
                    Some(WinType::Graphics) => self.backend.graphics_erase_rect(a(0), 0, 0, u32::MAX, u32::MAX),
                    _ => {}
                }
                0
            }
            0x002B => {
                // glk_window_move_cursor(win, xpos, ypos)
                self.glk.window_move_cursor(a(0), a(1), a(2));
                0
            }
            0x002C => self.glk.window_stream(a(0)).unwrap_or(0), // glk_window_get_stream
            0x002D => {
                // glk_window_set_echo_stream(win, str)
                self.glk.window_set_echo_stream(a(0), a(1));
                0
            }
            0x002E => self.glk.window_echo_stream(a(0)), // glk_window_get_echo_stream
            0x0030 => self.glk.window_sibling(a(0)).unwrap_or(0), // glk_window_get_sibling
            0x002F => {
                // glk_set_window(win)
                let win = a(0);
                let sid = if win == 0 { 0 } else { self.glk.window_stream(win).unwrap_or(0) };
                self.glk.set_current_stream(sid);
                0
            }
            // ── streams ───────────────────────────────────────────────────────
            0x0040 => {
                // glk_stream_iterate(str, rockptr) -> next stream id
                let (next, rock) = self.glk.stream_iterate(a(0));
                self.glk_store_ptr(a(1), rock)?;
                next
            }
            0x0041 => self.glk.stream_rock(a(0)).unwrap_or(0), // glk_stream_get_rock
            0x0043 => self.glk.stream_open_memory(a(0), a(1), false, a(2), a(3)), // open_memory(addr,len,fmode,rock)
            0x0139 => self.glk.stream_open_memory(a(0), a(1), true, a(2), a(3)),  // open_memory_uni
            0x0042 => self.glk.stream_open_file(a(0), a(1), false, a(2)),   // open_file(fref,fmode,rock)
            0x0138 => self.glk.stream_open_file(a(0), a(1), true, a(2)),    // open_file_uni
            0x0049 => {
                // glk_stream_open_resource(filenum, rock) — read-only Blorb Data
                // chunk; the host resolves the bytes + text/binary flag. Missing
                // resource → open fails (0), and so does a save for another story
                // (see `quetzal_for_another_story`).
                match self.backend.data_resource(a(0)) {
                    Some((data, is_text)) if !self.quetzal_for_another_story(&data) => {
                        self.glk.stream_open_resource(data, false, is_text, a(1))
                    }
                    _ => 0,
                }
            }
            0x013A => {
                // glk_stream_open_resource_uni(filenum, rock)
                match self.backend.data_resource(a(0)) {
                    Some((data, is_text)) if !self.quetzal_for_another_story(&data) => {
                        self.glk.stream_open_resource(data, true, is_text, a(1))
                    }
                    _ => 0,
                }
            }
            0x0044 => {
                // glk_stream_close(str, resultptr{readcount, writecount})
                match self.glk.stream_close(a(0)) {
                    Some((r, w)) => self.glk_out_ref(a(1), &[r, w])?,
                    None => self.diagnostics.push(format!("glk_stream_close: bad stream {}", a(0))),
                }
                0
            }
            0x0045 => {
                // glk_stream_set_position(str, pos, seekmode)
                self.glk.stream_set_position(a(0), a(1) as i32, a(2));
                0
            }
            0x0046 => self.glk.stream_position(a(0)).unwrap_or(0), // glk_stream_get_position
            0x0047 => {
                // glk_stream_set_current(str)
                self.glk.set_current_stream(a(0));
                0
            }
            0x0048 => self.glk.current_stream(), // glk_stream_get_current
            // ── stream reads ──────────────────────────────────────────────────
            0x0090 => {
                // glk_get_char_stream(str) — read one Latin-1 byte, or 0xFFFFFFFF = EOF
                let sid = a(0);
                match self.glk.memory_stream_read_info(sid) {
                    // A byte-oriented read works on a Unicode memory stream too:
                    // each 32-bit element comes back clamped to a byte
                    // (> 0xFF → '?'), not EOF (cheapglk gli_get_char).
                    Some((addr, len, pos, unicode)) if pos < len => {
                        let v = if unicode {
                            clamp_byte(self.m32(addr + pos * 4)?)
                        } else {
                            self.m8(addr + pos)?
                        };
                        self.glk.memory_stream_read_advance(sid, 1);
                        v
                    }
                    // A byte-oriented read works on a Unicode file/resource
                    // stream too: it returns each code point clamped to a byte
                    // (> 0xFF → '?'), not EOF (Glk spec §5.4).
                    _ if self.glk.file_stream_is_unicode(sid).is_some() => {
                        match self.glk.file_stream_read_char(sid) {
                            Some(cp) => clamp_byte(cp),
                            None => 0xFFFF_FFFF,
                        }
                    }
                    _ if self.glk.resource_stream_is_unicode(sid).is_some() => {
                        match self.glk.resource_stream_read_char(sid) {
                            Some(cp) => clamp_byte(cp),
                            None => 0xFFFF_FFFF,
                        }
                    }
                    _ => 0xFFFF_FFFF, // EOF or not a byte stream
                }
            }
            0x0091 => {
                // glk_get_line_stream(str, buf, maxlen) — read up to maxlen-1 bytes
                let (sid, buf, maxlen) = (a(0), a(1), a(2));
                let mut count = 0u32;
                if let Some((addr, len, pos, unicode)) = self.glk.memory_stream_read_info(sid) {
                    let mut p = pos;
                    while count + 1 < maxlen && p < len {
                        // Unicode memory stream: clamp each 32-bit element to a
                        // byte (> 0xFF → '?') — cheapglk gli_get_line.
                        let byte = if unicode {
                            clamp_byte(self.m32(addr + p * 4)?)
                        } else {
                            self.m8(addr + p)?
                        };
                        p += 1;
                        self.store_mem_sized(buf + count, byte, 1)?;
                        count += 1;
                        if byte == b'\n' as u32 {
                            break;
                        }
                    }
                    self.glk.memory_stream_read_advance(sid, count);
                } else if self.glk.file_stream_is_unicode(sid).is_some() {
                    while count + 1 < maxlen {
                        match self.glk.file_stream_read_char(sid) {
                            Some(cp) => {
                                let byte = clamp_byte(cp);
                                self.store_mem_sized(buf + count, byte, 1)?;
                                count += 1;
                                if byte == b'\n' as u32 {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                } else if self.glk.resource_stream_is_unicode(sid).is_some() {
                    while count + 1 < maxlen {
                        match self.glk.resource_stream_read_char(sid) {
                            Some(cp) => {
                                let byte = clamp_byte(cp);
                                self.store_mem_sized(buf + count, byte, 1)?;
                                count += 1;
                                if byte == b'\n' as u32 {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
                // always NUL-terminate if room
                if maxlen > 0 {
                    self.store_mem_sized(buf + count, 0, 1)?;
                }
                count
            }
            0x0092 => {
                // glk_get_buffer_stream(str, buf, len) — read up to len bytes
                let (sid, buf, maxlen) = (a(0), a(1), a(2));
                let mut count = 0u32;
                if let Some((addr, len, pos, unicode)) = self.glk.memory_stream_read_info(sid) {
                    // pos can sit PAST len: writes past a memory stream's end
                    // advance the cursor without storing (Glk §5.3), so a
                    // later read must see EOF, not a wrapped `len - pos`
                    // (SQ-0625 — the 0x0090/0x0091 arms guard with their
                    // `pos < len` matches; mirror them).
                    let available = len.saturating_sub(pos).min(maxlen);
                    for i in 0..available {
                        // Unicode memory stream: clamp each 32-bit element to a
                        // byte (> 0xFF → '?') — cheapglk gli_get_buffer.
                        let byte = if unicode {
                            clamp_byte(self.m32(addr + (pos + i) * 4)?)
                        } else {
                            self.m8(addr + pos + i)?
                        };
                        self.store_mem_sized(buf + i, byte, 1)?;
                    }
                    count = available;
                    self.glk.memory_stream_read_advance(sid, count);
                } else if self.glk.file_stream_is_unicode(sid).is_some() {
                    while count < maxlen {
                        match self.glk.file_stream_read_char(sid) {
                            Some(cp) => {
                                self.store_mem_sized(buf + count, clamp_byte(cp), 1)?;
                                count += 1;
                            }
                            None => break,
                        }
                    }
                } else if self.glk.resource_stream_is_unicode(sid).is_some() {
                    while count < maxlen {
                        match self.glk.resource_stream_read_char(sid) {
                            Some(cp) => {
                                self.store_mem_sized(buf + count, clamp_byte(cp), 1)?;
                                count += 1;
                            }
                            None => break,
                        }
                    }
                }
                count
            }
            0x0130 => {
                // glk_get_char_stream_uni(str) — read one codepoint, or 0xFFFFFFFF = EOF
                let sid = a(0);
                match self.glk.memory_stream_read_info(sid) {
                    Some((addr, len, pos, unicode)) if pos < len => {
                        let v = if unicode { self.m32(addr + pos * 4)? } else { self.m8(addr + pos)? };
                        self.glk.memory_stream_read_advance(sid, 1);
                        v
                    }
                    _ if self.glk.file_stream_is_unicode(sid).is_some() => {
                        self.glk.file_stream_read_char(sid).unwrap_or(0xFFFF_FFFF)
                    }
                    _ if self.glk.resource_stream_is_unicode(sid).is_some() => {
                        self.glk.resource_stream_read_char(sid).unwrap_or(0xFFFF_FFFF)
                    }
                    _ => 0xFFFF_FFFF,
                }
            }
            0x0131 => {
                // glk_get_buffer_stream_uni(str, buf, len) — read up to len codepoints
                let (sid, buf, maxlen) = (a(0), a(1), a(2));
                let mut count = 0u32;
                if let Some((addr, len, pos, unicode)) = self.glk.memory_stream_read_info(sid) {
                    // pos past len → EOF, exactly as in the 0x0092 arm (SQ-0625).
                    let available = len.saturating_sub(pos).min(maxlen);
                    for i in 0..available {
                        let cp = if unicode {
                            self.m32(addr + (pos + i) * 4)?
                        } else {
                            self.m8(addr + pos + i)?
                        };
                        self.store_mem_sized(buf + i * 4, cp, 4)?;
                    }
                    count = available;
                    self.glk.memory_stream_read_advance(sid, count);
                } else if self.glk.file_stream_is_unicode(sid).is_some() {
                    while count < maxlen {
                        match self.glk.file_stream_read_char(sid) {
                            Some(cp) => {
                                self.store_mem_sized(buf + count * 4, cp, 4)?;
                                count += 1;
                            }
                            None => break,
                        }
                    }
                } else if self.glk.resource_stream_is_unicode(sid).is_some() {
                    while count < maxlen {
                        match self.glk.resource_stream_read_char(sid) {
                            Some(cp) => {
                                self.store_mem_sized(buf + count * 4, cp, 4)?;
                                count += 1;
                            }
                            None => break,
                        }
                    }
                }
                count
            }
            0x0132 => {
                // glk_get_line_stream_uni(str, buf, maxlen) — read up to maxlen-1 codepoints
                let (sid, buf, maxlen) = (a(0), a(1), a(2));
                let mut count = 0u32;
                if let Some((addr, len, pos, unicode)) = self.glk.memory_stream_read_info(sid) {
                    let mut p = pos;
                    while count + 1 < maxlen && p < len {
                        let cp = if unicode {
                            self.m32(addr + p * 4)?
                        } else {
                            self.m8(addr + p)?
                        };
                        p += 1;
                        self.store_mem_sized(buf + count * 4, cp, 4)?;
                        count += 1;
                        if cp == b'\n' as u32 {
                            break;
                        }
                    }
                    self.glk.memory_stream_read_advance(sid, count);
                } else if self.glk.file_stream_is_unicode(sid).is_some() {
                    while count + 1 < maxlen {
                        match self.glk.file_stream_read_char(sid) {
                            Some(cp) => {
                                self.store_mem_sized(buf + count * 4, cp, 4)?;
                                count += 1;
                                if cp == b'\n' as u32 {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                } else if self.glk.resource_stream_is_unicode(sid).is_some() {
                    while count + 1 < maxlen {
                        match self.glk.resource_stream_read_char(sid) {
                            Some(cp) => {
                                self.store_mem_sized(buf + count * 4, cp, 4)?;
                                count += 1;
                                if cp == b'\n' as u32 {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
                // always NUL-terminate if room
                if maxlen > 0 {
                    self.store_mem_sized(buf + count * 4, 0, 4)?;
                }
                count
            }
            // ── styles / gestalt / control ────────────────────────────────────
            0x0086 => {
                // glk_set_style(style) — on the current stream
                let sid = self.glk.current_stream();
                self.glk.set_stream_style(sid, GlkStyle::from_num(a(0)));
                0
            }
            0x0087 => {
                // glk_set_style_stream(str, style)
                self.glk.set_stream_style(a(0), GlkStyle::from_num(a(1)));
                0
            }
            0x00A0 => glk_char_to_lower(a(0)), // glk_char_to_lower(ch)
            0x00A1 => glk_char_to_upper(a(0)), // glk_char_to_upper(ch)
            0x0120 => self.glk_buffer_case_uni(a(0), a(1), a(2), CaseOp::Lower)?, // _to_lower_case_uni
            0x0121 => self.glk_buffer_case_uni(a(0), a(1), a(2), CaseOp::Upper)?, // _to_upper_case_uni
            0x0122 => self.glk_buffer_case_uni(a(0), a(1), a(2), CaseOp::Title { lower_rest: a(3) != 0 })?,
            0x0123 => self.glk_buffer_norm_uni(a(0), a(1), a(2), NormForm::Nfd)?, // _canon_decompose_uni
            0x0124 => self.glk_buffer_norm_uni(a(0), a(1), a(2), NormForm::Nfc)?, // _canon_normalize_uni
            0x00B0 => { self.glk.set_style_hint(a(0), a(1), a(2), a(3)); 0 } // glk_stylehint_set
            0x00B1 => { self.glk.clear_style_hint(a(0), a(1), a(2)); 0 }     // glk_stylehint_clear
            // glk_style_distinguish: compares the rendered stylehints the model
            // records — colour/reverse plus Weight/Oblique (SQ-0317). Since SQ-0803
            // the backend's default colours are per STYLE CLASS, not one pair per
            // window type, so two styles the game never hinted can still be told
            // apart by what the host paints (a garglk.ini that colours only
            // style_User2 is exactly that case) — hence the second arm. The model's
            // hint-based answer still wins when it says yes: a hint the game set is
            // what gets rendered, overriding the host default the backend reports.
            0x00B2 => {
                let distinguishable = self.glk.style_distinguish(a(0), a(1), a(2))
                    || (a(1) != a(2)
                        && a(1) < glk::NUMSTYLES
                        && a(2) < glk::NUMSTYLES
                        && match self.glk.window_type(a(0)) {
                            Some(wt) => {
                                self.backend.default_style_colours(wt, a(1))
                                    != self.backend.default_style_colours(wt, a(2))
                            }
                            None => false,
                        });
                distinguishable as u32
            }
            0x00B3 => {
                // glk_style_measure(win, styl, hint, result*) -> 1 if known + stored.
                // Answer order (SQ-0315): a game-set stylehint colour wins (it IS
                // what's rendered when set); else the backend's actually-rendered
                // default colour for that style; else unknown (0). Only the two
                // colour hints consult the backend — TextColor (7) / BackColor (8).
                let measured = self.glk.style_measure(a(0), a(1), a(2)).or_else(|| {
                    if a(1) >= glk::NUMSTYLES {
                        return None;
                    }
                    let wintype = self.glk.window_type(a(0))?;
                    let (fg, bg) = self.backend.default_style_colours(wintype, a(1))?;
                    match a(2) {
                        7 => fg, // stylehint_TextColor
                        8 => bg, // stylehint_BackColor
                        _ => None,
                    }
                });
                match measured {
                    Some(v) => {
                        self.glk_store_ptr(a(3), v)?;
                        1
                    }
                    None => 0,
                }
            }
            // Gargoyle garglk_* colour extensions (gestalt_GarglkText, selector 0x1100).
            0x1100 => {
                // garglk_set_zcolors(fg, bg) — on the current stream
                let sid = self.glk.current_stream();
                self.glk.set_stream_zcolors(sid, a(0), a(1));
                0
            }
            0x1101 => { self.glk.set_stream_zcolors(a(0), a(1), a(2)); 0 } // garglk_set_zcolors_stream(str, fg, bg)
            0x1102 => {
                // garglk_set_reversevideo(reverse) — on the current stream
                let sid = self.glk.current_stream();
                self.glk.set_stream_reversevideo(sid, a(0));
                0
            }
            0x1103 => { self.glk.set_stream_reversevideo(a(0), a(1)); 0 } // garglk_set_reversevideo_stream(str, reverse)
            // ── input requests + select (3a-2) ────────────────────────────────
            0x00D0 => {
                // glk_request_line_event(win, buf, maxlen, initlen)
                self.glk_request_line(a(0), a(1), a(2), a(3), false);
                0
            }
            0x0141 => {
                // glk_request_line_event_uni(win, buf, maxlen, initlen)
                self.glk_request_line(a(0), a(1), a(2), a(3), true);
                0
            }
            0x00D2 => {
                // glk_request_char_event(win)
                self.glk_request_char(a(0), false);
                0
            }
            0x0140 => {
                // glk_request_char_event_uni(win)
                self.glk_request_char(a(0), true);
                0
            }
            0x00D1 => {
                // glk_cancel_line_event(win, event)
                self.glk_cancel_line(a(0), a(1))?;
                0
            }
            0x00D3 => {
                // glk_cancel_char_event(win)
                self.glk.take_char_request(a(0));
                self.clear_pending_input_for(a(0), false);
                0
            }
            0x00C0 => {
                // glk_select(event) — suspend until an event arrives
                self.glk_select(a(0))?;
                0
            }
            0x00C1 => {
                // glk_select_poll(event) — internal events only, never suspends
                let ev = self.glk.pop_event().unwrap_or_else(GlkEvent::none);
                self.write_event(a(0), ev)?;
                0
            }
            0x00D6 => {
                // glk_request_timer_events(millisecs): arm a periodic timer, or
                // cancel it when millisecs == 0. The host reads the interval via
                // `glk_timer_interval` and calls `deliver_timer` on each tick.
                self.timer_interval_ms = if a(0) != 0 { Some(a(0)) } else { None };
                0
            }
            0x00D4 => {
                // glk_request_mouse_event(win): arm a mouse-click watch. The host
                // reads mouse_windows() and calls deliver_mouse on a real click.
                self.glk.set_mouse_request(a(0));
                0
            }
            0x00D5 => {
                // glk_cancel_mouse_event(win): disarm the watch (one-shot take).
                self.glk.take_mouse_request(a(0));
                0
            }
            0x0100 => {
                // glk_set_hyperlink(linkval): set the current stream's link value.
                let sid = self.glk.current_stream();
                self.glk.set_stream_link(sid, a(0));
                0
            }
            0x0101 => {
                // glk_set_hyperlink_stream(str, linkval)
                self.glk.set_stream_link(a(0), a(1));
                0
            }
            0x0102 => {
                // glk_request_hyperlink_event(win): arm a hyperlink-click watch.
                self.glk.set_hyperlink_request(a(0));
                0
            }
            0x0103 => {
                // glk_cancel_hyperlink_event(win): disarm the watch (one-shot take).
                self.glk.take_hyperlink_request(a(0));
                0
            }
            0x0150 => {
                // glk_set_echo_line_event(win, val): record the window's echo flag
                // so scrolling-terminal hosts can avoid double-echoing a game that
                // echoes its own input line.
                self.glk.set_window_echo_line(a(0), a(1) != 0);
                0
            }
            0x0151 => {
                // glk_set_terminators_line_event(win, keycodes, count)
                self.glk_set_terminators(a(0), a(1), a(2))?;
                0
            }
            0x0004 => self.glk_gestalt(a(0), a(1)), // glk_gestalt(sel, val)
            0x0005 => {
                // glk_gestalt_ext(sel, val, arr, arrlen). Only gestalt_CharOutput (3)
                // writes the output array — arr[0] = glyphs printed for the char. We
                // report ExactPrint for every char, so that is exactly one glyph. A
                // null or zero-length array is skipped, per spec.
                let (sel, val, arr, arrlen) = (a(0), a(1), a(2), a(3));
                if sel == 3 && arr != 0 && arrlen >= 1 {
                    self.store_mem(arr, 1)?;
                }
                self.glk_gestalt(sel, val)
            }
            0x0001 => {
                // glk_exit — end the program
                self.halted = true;
                0
            }
            // glk_set_interrupt_handler — unusable from Glulx (a C function pointer
            // can't cross the dispatch layer), so it is a no-op here. Explicit arm so
            // the call doesn't spam the diagnostics log.
            0x0002 => 0,
            // glk_tick — a periodic housekeeping hook with nothing to do on a modern
            // host. Explicit no-op so the constant calls don't spam the diagnostics log.
            0x0003 => 0,
            // ── filerefs (in-memory VFS) ────────────────────────────────────────
            0x0060 => self.glk.fileref_create_temp(a(0), a(1)), // glk_fileref_create_temp(usage, rock)
            0x0061 => {
                // glk_fileref_create_by_name(usage, nameptr, rock): nameptr is a
                // NUL-terminated byte string in Glulx memory.
                let name = self.read_cstring(a(1))?;
                self.glk.fileref_create(a(0), name, a(2))
            }
            0x0062 => {
                // glk_fileref_create_by_prompt(usage, fmode, rock): no synchronous
                // name. Suspend so the host can prompt (write) or pick (read);
                // supply_filename() resumes with the choice. op_glk fills in `dest`
                // (this @glk's S1) and returns the suspend instead of storing.
                self.pending_fileref =
                    Some(PendingFileref { dest: Dest::Discard, usage: a(0), fmode: a(1), rock: a(2) });
                0 // placeholder; not stored (op_glk suspends instead)
            }
            0x0063 => {
                // glk_fileref_destroy(fref)
                self.glk.fileref_destroy(a(0));
                0
            }
            0x0064 => {
                // glk_fileref_iterate(fref, rockptr) -> next fileref id
                let (next, rock) = self.glk.fileref_iterate(a(0));
                self.glk_store_ptr(a(1), rock)?;
                next
            }
            0x0065 => self.glk.fileref_rock(a(0)), // glk_fileref_get_rock(fref)
            0x0066 => {
                // glk_fileref_delete_file(fref)
                self.glk.fileref_delete(a(0));
                0
            }
            0x0067 => self.glk.fileref_exists(a(0)) as u32, // glk_fileref_does_file_exist(fref)
            0x0068 => self.glk.fileref_create_from(a(0), a(1), a(2)), // glk_fileref_create_from_fileref(usage, oldfref, rock)
            // ── graphics (GLULX_NOTES §21) ──────────────────────────────────────
            0x00E0 => {
                // glk_image_get_info(image, widthptr, heightptr) -> 1 if it exists
                if self.graphics_enabled {
                    if let Some((w, h)) = self.backend.image_info(a(0)) {
                        self.glk_store_ptr(a(1), w)?;
                        self.glk_store_ptr(a(2), h)?;
                        1
                    } else {
                        0
                    }
                } else {
                    0
                }
            }
            0x00E1 => {
                // glk_image_draw(win, image, val1=x, val2=y) -> 1 if actually drawn
                if self.graphics_enabled {
                    self.backend.graphics_draw_image(a(0), a(1), a(2) as i32, a(3) as i32, None) as u32
                } else {
                    0
                }
            }
            0x00E2 => {
                // glk_image_draw_scaled(win, image, val1=x, val2=y, width, height)
                // -> 1 if actually drawn
                if self.graphics_enabled {
                    self.backend.graphics_draw_image(a(0), a(1), a(2) as i32, a(3) as i32, Some((a(4), a(5)))) as u32
                } else {
                    0
                }
            }
            0x00E8 => {
                // glk_window_flow_break(win) — block-mode inline images already
                // break the text flow; nothing to do.
                let _win = a(0);
                0
            }
            0x00E9 => {
                // glk_window_erase_rect(win, left, top, width, height)
                if self.graphics_enabled {
                    self.backend.graphics_erase_rect(a(0), a(1) as i32, a(2) as i32, a(3), a(4));
                }
                0
            }
            0x00EA => {
                // glk_window_fill_rect(win, color, left, top, width, height)
                if self.graphics_enabled {
                    self.backend.graphics_fill_rect(a(0), a(1), a(2) as i32, a(3) as i32, a(4), a(5));
                }
                0
            }
            0x00EB => {
                // glk_window_set_background_color(win, color)
                if self.graphics_enabled {
                    self.backend.graphics_set_background(a(0), a(1));
                }
                0
            }
            // ── sound channels (Glk Sound; GLULX_NOTES) ─────────────────────────
            0x00F0 => {
                // glk_schannel_iterate(chan, &rock) -> next chan
                if self.sound_enabled {
                    let (next, rock) = self.backend.schannel_iterate(a(0));
                    self.glk_store_ptr(a(1), rock)?;
                    next
                } else {
                    self.glk_store_ptr(a(1), 0)?;
                    0
                }
            }
            0x00F1 => {
                // glk_schannel_get_rock(chan) -> rock
                if self.sound_enabled { self.backend.schannel_get_rock(a(0)) } else { 0 }
            }
            0x00F2 => {
                // glk_schannel_create(rock) -> chan
                if self.sound_enabled { self.backend.schannel_create(a(0)) } else { 0 }
            }
            0x00F3 => {
                // glk_schannel_destroy(chan)
                if self.sound_enabled { self.backend.schannel_destroy(a(0)); }
                0
            }
            0x00F4 => {
                // glk_schannel_create_ext(rock, volume) -> chan  (Sound2)
                if self.sound_enabled { self.backend.schannel_create_ext(a(0), a(1)) } else { 0 }
            }
            0x00F7 => {
                // glk_schannel_play_multi(chanarray, chancount, sndarray, soundcount,
                // notify) -> count started (Sound2). Each sound plays once with the
                // shared notify; each finished sound posts its own SoundNotify
                // (val1 = its resource, val2 = notify), so this reuses schannel_play.
                if self.sound_enabled {
                    let (chan_ptr, count, snd_ptr) = (a(0), a(1), a(2));
                    // chancount and soundcount are required to be equal (spec §8.3);
                    // clamp to the shorter to stay memory-safe if a game disagrees.
                    let count = count.min(a(3));
                    let notify = a(4);
                    let mut started = 0u32;
                    for i in 0..count {
                        let chan = self.m32(chan_ptr + i * 4)?;
                        let snd = self.m32(snd_ptr + i * 4)?;
                        started += self.backend.schannel_play(chan, snd, 1, notify);
                    }
                    started
                } else {
                    0
                }
            }
            0x00F8 => {
                // glk_schannel_play(chan, snd) -> 1/0  (repeats=1, no notify)
                if self.sound_enabled { self.backend.schannel_play(a(0), a(1), 1, 0) } else { 0 }
            }
            0x00F9 => {
                // glk_schannel_play_ext(chan, snd, repeats, notify) -> 1/0
                if self.sound_enabled { self.backend.schannel_play(a(0), a(1), a(2), a(3)) } else { 0 }
            }
            0x00FA => {
                // glk_schannel_stop(chan)
                if self.sound_enabled { self.backend.schannel_stop(a(0)); }
                0
            }
            0x00FB => {
                // glk_schannel_set_volume(chan, vol)
                if self.sound_enabled { self.backend.schannel_set_volume(a(0), a(1)); }
                0
            }
            0x00FC => {
                // glk_sound_load_hint(snd, flag) — decoding is on-demand; accept + ignore.
                0
            }
            0x00FD => {
                // glk_schannel_set_volume_ext(chan, vol, duration, notify) (Sound2):
                // ramped volume change; the host fires evtype_VolumeNotify when done.
                if self.sound_enabled { self.backend.schannel_set_volume_ext(a(0), a(1), a(2), a(3)); }
                0
            }
            0x00FE => {
                // glk_schannel_pause(chan) (Sound2)
                if self.sound_enabled { self.backend.schannel_pause(a(0)); }
                0
            }
            0x00FF => {
                // glk_schannel_unpause(chan) (Sound2)
                if self.sound_enabled { self.backend.schannel_unpause(a(0)); }
                0
            }
            // ── date/time (Glk 0.7.6; _local via GlkBackend — see glk::datetime) ─
            0x0160 => {
                // glk_current_time(timeval*)
                let (secs, micro) = Self::now_seconds();
                self.write_timeval(a(0), glk::datetime::timestamp_to_timeval(secs, micro))?;
                0
            }
            0x0161 => {
                // glk_current_simple_time(factor) -> simplified time
                let (secs, _) = Self::now_seconds();
                glk::datetime::simplify_time(secs, a(0)) as u32
            }
            // glk_time_to_date_utc (timeval* -> date*).
            0x0168 => {
                let tv = self.read_timeval(a(0))?;
                let secs = glk::datetime::timeval_to_timestamp(tv);
                self.write_glkdate(a(1), glk::datetime::time_to_date(secs, tv.microsec))?;
                0
            }
            // glk_time_to_date_local (timeval* -> date*).
            0x0169 => {
                let tv = self.read_timeval(a(0))?;
                let secs = glk::datetime::timeval_to_timestamp(tv);
                let date = glk::datetime::time_to_date_local(secs, tv.microsec, |t| {
                    self.backend.local_utc_offset_seconds(t)
                });
                self.write_glkdate(a(1), date)?;
                0
            }
            // glk_simple_time_to_date_utc (time, factor -> date*).
            0x016A => {
                let secs = a(0) as i32 as i64 * a(1) as i64;
                self.write_glkdate(a(2), glk::datetime::time_to_date(secs, 0))?;
                0
            }
            // glk_simple_time_to_date_local (time, factor -> date*).
            0x016B => {
                let secs = a(0) as i32 as i64 * a(1) as i64;
                let date = glk::datetime::time_to_date_local(secs, 0, |t| {
                    self.backend.local_utc_offset_seconds(t)
                });
                self.write_glkdate(a(2), date)?;
                0
            }
            // glk_date_to_time_utc (date* -> timeval*).
            0x016C => {
                let date = self.read_glkdate(a(0))?;
                let (secs, micro) = glk::datetime::date_to_time(date);
                self.write_timeval(a(1), glk::datetime::timestamp_to_timeval(secs, micro))?;
                0
            }
            // glk_date_to_time_local (date* -> timeval*).
            0x016D => {
                let date = self.read_glkdate(a(0))?;
                let (secs, micro) = glk::datetime::date_to_time_local(date, |t| {
                    self.backend.local_utc_offset_seconds(t)
                });
                self.write_timeval(a(1), glk::datetime::timestamp_to_timeval(secs, micro))?;
                0
            }
            // glk_date_to_simple_time_utc (date*, factor -> time).
            0x016E => {
                let date = self.read_glkdate(a(0))?;
                let (secs, _) = glk::datetime::date_to_time(date);
                glk::datetime::simplify_time(secs, a(1)) as u32
            }
            // glk_date_to_simple_time_local (date*, factor -> time).
            0x016F => {
                let date = self.read_glkdate(a(0))?;
                let (secs, _) = glk::datetime::date_to_time_local(date, |t| {
                    self.backend.local_utc_offset_seconds(t)
                });
                glk::datetime::simplify_time(secs, a(1)) as u32
            }
            other => {
                self.diagnostics
                    .push(format!("unhandled @glk selector {other:#06x} (returning 0)"));
                0
            }
        };
        if let Some(call) = trace_call {
            // Prefer an out-pointer result (read back from memory), else the
            // return value; either rides on the call line as `-> …`. (SQ-0405)
            let result = self.glk_out_result(selector, args).or_else(|| glk_trace_ret(selector, ret));
            match result {
                Some(r) => self.screen_trace.push(format!("{call} -> {r}")),
                None => self.screen_trace.push(call),
            }
        }
        Ok(ret)
    }

    /// Write `s` to the current Glk stream (used by the put-to-current
    /// selectors). Glk output is independent of the VM's I/O system.
    fn glk_put_current(&mut self, s: &str) {
        let sid = self.glk.current_stream();
        self.glk_stream_put(sid, s);
    }

    /// Read `len` Latin-1 bytes at `addr` into a String.
    fn read_latin1_buffer(&self, addr: u32, len: u32) -> R<String> {
        let mut s = String::with_capacity(cap_hint(len));
        for i in 0..len {
            s.push(self.m8(addr + i)? as u8 as char);
        }
        Ok(s)
    }
    /// Read `len` big-endian 32-bit code points at `addr` into a String.
    fn read_uni_buffer(&self, addr: u32, len: u32) -> R<String> {
        let mut s = String::with_capacity(cap_hint(len));
        for i in 0..len {
            let cp = self.m32(addr + i * 4)?;
            s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
        }
        Ok(s)
    }

    /// `glk_window_open`: open a window in the model, tell the backend, and
    /// recompute the layout. Returns the new window id (0 on a malformed call).
    fn glk_open_window(&mut self, split: u32, method: u32, size: u32, wintype: u32, rock: u32) -> u32 {
        if wintype == 5 && !self.graphics_enabled {
            self.diagnostics
                .push("glk_window_open(Graphics) rejected — graphics disabled".to_string());
            return 0;
        }
        match self.glk.window_open(split, method, size, wintype, rock) {
            Some(id) => {
                if let Some(ty) = self.glk.window_type(id) {
                    self.backend.window_open(id, ty);
                }
                self.relayout_glk();
                if self.glk.window_type(id) == Some(WinType::Graphics) {
                    self.glk.push_event(GlkEvent { etype: glk::evtype::REDRAW, win: id, val1: 0, val2: 0 });
                }
                id
            }
            None => {
                self.diagnostics
                    .push(format!("glk_window_open(split={split}, wintype={wintype}) failed"));
                0
            }
        }
    }

    /// Recompute the window layout from the backend's screen size and notify it.
    fn relayout_glk(&mut self) {
        let (w, h) = self.backend.screen_size();
        let cp = self.backend.char_pixels();
        let layout = self.glk.relayout(w, h, cp);
        self.backend.window_layout(&layout);
        let tree = self.glk.window_tree();
        self.backend.window_tree(tree);
    }

    /// Re-push the current window tree to the backend WITHOUT recomputing
    /// geometry. A window's per-window colours come from the live style hints
    /// (`window_style_colour`), which a game may set *after* the window opens
    /// (e.g. Kerkerkruip paints its panel backgrounds, and Counterfeit Monkey its
    /// black-on-white page, once the windows exist). The tree is otherwise only
    /// re-pushed on a structural `relayout_glk`, so between relayouts the renderer
    /// keeps each window's open-time colour snapshot and paints panes with
    /// stale/absent backgrounds. The host calls this each time it rebuilds its
    /// screen model so the render always reflects the live colours. Cheap: a pure
    /// walk of the live window tree, no layout math.
    pub fn sync_window_tree(&mut self) {
        let tree = self.glk.window_tree();
        self.backend.window_tree(tree);
    }

    /// Deliver Glk output reference/struct values `vals` for the pointer argument
    /// `ptr`, following the Glulx Glk dispatch convention:
    /// * `ptr == 0` → a NULL pointer: discard (no result wanted).
    /// * `ptr == 0xFFFFFFFF` (-1) → push each value onto the VM stack, in order,
    ///   so the **last** field ends up on top (the game pops them back).
    ///   Verified against glulxe glkop.c `WriteStructField` (SQ-0627): the -1
    ///   convention writes field 0 at the lowest stack address and ascends,
    ///   i.e. field 0 is pushed first and ends deepest — same as here.
    /// * otherwise → write the values consecutively to memory at `ptr`, `ptr+4`, …
    fn glk_out_ref(&mut self, ptr: u32, vals: &[u32]) -> R<()> {
        if ptr == 0 {
            return Ok(());
        }
        if ptr == 0xFFFF_FFFF {
            for &v in vals {
                self.push32(v)?;
            }
            return Ok(());
        }
        for (i, &v) in vals.iter().enumerate() {
            self.store_mem(ptr + 4 * i as u32, v)?;
        }
        Ok(())
    }

    /// Deliver a single Glk output reference value (see [`Machine::glk_out_ref`]).
    fn glk_store_ptr(&mut self, ptr: u32, v: u32) -> R<()> {
        self.glk_out_ref(ptr, &[v])
    }

    // ── date/time (Glk 0.7.6) ─────────────────────────────────────────────────

    /// The wall-clock "now" as `(epoch seconds, microsec)` from
    /// [`std::time::SystemTime`]. Times before the Unix epoch (the host clock is
    /// set wrong) yield a negative second count with a normalized `microsec`.
    fn now_seconds() -> (i64, i32) {
        use std::time::{SystemTime, UNIX_EPOCH};
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => (d.as_secs() as i64, d.subsec_micros() as i32),
            Err(e) => {
                // Clock is before 1970: report the negative offset (borrowing a
                // second when there's a sub-second remainder, so microsec >= 0).
                let d = e.duration();
                let micros = d.subsec_micros();
                if micros == 0 {
                    (-(d.as_secs() as i64), 0)
                } else {
                    (-(d.as_secs() as i64) - 1, (1_000_000 - micros) as i32)
                }
            }
        }
    }

    /// Read a Glk `glktimeval_t` (3 words: high_sec, low_sec, microsec) at `addr`.
    fn read_timeval(&self, addr: u32) -> R<glk::datetime::GlkTimeVal> {
        Ok(glk::datetime::GlkTimeVal {
            high_sec: self.m32(addr)? as i32,
            low_sec: self.m32(addr + 4)?,
            microsec: self.m32(addr + 8)? as i32,
        })
    }

    /// Write a Glk `glktimeval_t` (3 words) to `addr` (honors NULL / push-to-stack).
    fn write_timeval(&mut self, addr: u32, tv: glk::datetime::GlkTimeVal) -> R<()> {
        self.glk_out_ref(addr, &[tv.high_sec as u32, tv.low_sec, tv.microsec as u32])
    }

    /// Read a Glk `glkdate_t` (8 words) at `addr`.
    fn read_glkdate(&self, addr: u32) -> R<glk::datetime::GlkDate> {
        Ok(glk::datetime::GlkDate {
            year: self.m32(addr)? as i32,
            month: self.m32(addr + 4)? as i32,
            day: self.m32(addr + 8)? as i32,
            weekday: self.m32(addr + 12)? as i32,
            hour: self.m32(addr + 16)? as i32,
            minute: self.m32(addr + 20)? as i32,
            second: self.m32(addr + 24)? as i32,
            microsec: self.m32(addr + 28)? as i32,
        })
    }

    /// Write a Glk `glkdate_t` (8 words) to `addr` (honors NULL / push-to-stack).
    fn write_glkdate(&mut self, addr: u32, d: glk::datetime::GlkDate) -> R<()> {
        self.glk_out_ref(
            addr,
            &[
                d.year as u32,
                d.month as u32,
                d.day as u32,
                d.weekday as u32,
                d.hour as u32,
                d.minute as u32,
                d.second as u32,
                d.microsec as u32,
            ],
        )
    }

    // ── input requests + glk_select suspend/resume (3a-2) ─────────────────────

    /// A Unicode buffer case conversion (`glk_buffer_to_*_case_uni`): read
    /// `numchars` code points from the 32-bit array at `buf`, case-fold them
    /// (Unicode-aware; a fold may change the character count), write the result
    /// back (clamped to `buflen` elements), and return the full result length.
    fn glk_buffer_case_uni(&mut self, buf: u32, buflen: u32, numchars: u32, op: CaseOp) -> R<u32> {
        let mut s = String::new();
        for i in 0..numchars {
            let cp = self.m32(buf + i * 4)?;
            s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
        }
        let result: Vec<char> = match op {
            CaseOp::Lower => s.chars().flat_map(char::to_lowercase).collect(),
            CaseOp::Upper => s.chars().flat_map(char::to_uppercase).collect(),
            CaseOp::Title { lower_rest } => {
                let mut out = Vec::new();
                for (i, c) in s.chars().enumerate() {
                    if i == 0 {
                        // Titlecase the first char: a real Unicode titlecase
                        // mapping (digraphs/Greek/ligatures where it differs
                        // from uppercase), falling back to uppercase otherwise.
                        match unicode_norm::titlecase(c as u32) {
                            Some(seq) => out.extend(seq.iter().filter_map(|&cp| char::from_u32(cp))),
                            None => out.extend(c.to_uppercase()),
                        }
                    } else if lower_rest {
                        out.extend(c.to_lowercase());
                    } else {
                        out.push(c);
                    }
                }
                out
            }
        };
        for (i, &c) in result.iter().enumerate() {
            if i as u32 >= buflen {
                break;
            }
            self.store_mem_sized(buf + i as u32 * 4, c as u32, 4)?;
        }
        Ok(result.len() as u32)
    }

    /// A Unicode buffer normalization (`glk_buffer_canon_decompose_uni` / NFD,
    /// `glk_buffer_canon_normalize_uni` / NFC): read `numchars` code points from
    /// the 32-bit array at `buf`, normalize them (the count may change), write
    /// the result back clamped to `buflen` elements, and return the FULL result
    /// length — the same buffer-function contract as [`Self::glk_buffer_case_uni`]
    /// and cheapglk's `cgunicod.c` (truncated on write, untruncated in the value).
    fn glk_buffer_norm_uni(&mut self, buf: u32, buflen: u32, numchars: u32, form: NormForm) -> R<u32> {
        let mut input = Vec::with_capacity(cap_hint(numchars));
        for i in 0..numchars {
            input.push(self.m32(buf + i * 4)?);
        }
        let result = match form {
            NormForm::Nfd => unicode_norm::canonical_decompose(&input),
            NormForm::Nfc => unicode_norm::canonical_compose(&input),
        };
        for (i, &c) in result.iter().enumerate() {
            if i as u32 >= buflen {
                break;
            }
            self.store_mem_sized(buf + i as u32 * 4, c, 4)?;
        }
        Ok(result.len() as u32)
    }

    /// Record a pending line-input request; diagnose a bad window. When the game
    /// pre-loaded the buffer (`initlen` > 0, Glk spec §4.2) the pre-filled text is
    /// read out for [`Self::take_line_seed`] so the host can start its input
    /// line with it — read HERE, while the buffer still holds what the game just
    /// wrote, rather than lazily later.
    fn glk_request_line(&mut self, win: u32, buf: u32, maxlen: u32, initlen: u32, unicode: bool) {
        if !self.glk.request_line_event(win, buf, maxlen, initlen, unicode) {
            self.diagnostics
                .push(format!("glk_request_line_event: bad window {win}"));
            return;
        }
        self.line_seed =
            Some(self.read_line_prefill(buf, initlen.min(maxlen), unicode).unwrap_or_default());
    }

    /// Read `n` characters of pre-filled line input from `buf` — Latin-1 bytes, or
    /// 32-bit code points for a `_uni` request. `None` for an empty prefill or an
    /// unreadable buffer (a bad address is the game's bug, not worth faulting the
    /// turn over: the prompt just starts empty).
    fn read_line_prefill(&self, buf: u32, n: u32, unicode: bool) -> Option<String> {
        if n == 0 {
            return None;
        }
        let mut s = String::with_capacity(n as usize);
        for i in 0..n {
            let cp = if unicode { self.m32(buf + i * 4) } else { self.m8(buf + i) }.ok()?;
            s.push(char::from_u32(cp)?);
        }
        (!s.is_empty()).then_some(s)
    }

    /// Take what the host's input line should hold for the newest line-input
    /// request, if one has appeared since the last call. One-shot per request, so
    /// re-polling never re-inserts it. `Some("")` means "start empty" — see
    /// [`Self::line_seed`].
    pub fn take_line_seed(&mut self) -> Option<String> {
        self.line_seed.take()
    }

    /// Mirror the host's in-progress input line into the pending line request's
    /// buffer, updating the count of characters "already in the buffer".
    ///
    /// Glk hands the interpreter that buffer for the whole duration of the request
    /// (spec §4.2), which is what lets `glk_cancel_line_event` report how much has
    /// been typed so far. Writing only at submit time left the buffer holding
    /// whatever was last in it, so a game that cancels mid-edit read stale text:
    /// advent.blb's toolbar cancels on every button press and PRESERVES the partial
    /// input it finds, so after clicking Examine and deleting the word, every other
    /// verb button kept re-inserting "Examine " — the game was faithfully restoring
    /// text the player had already erased. A no-op when no line request is pending.
    pub fn set_line_input_text(&mut self, text: &str) {
        let Some((win, unicode)) = self.glk.first_line_request() else { return };
        let Some(req) = self.glk.line_request(win) else { return };
        let n = self.write_line_buffer(req.buf, req.maxlen, unicode, text).len() as u32;
        self.glk.set_line_initlen(win, n);
    }

    /// Write `text` into a line-input buffer — Latin-1 bytes, or 32-bit code points
    /// for a `_uni` request — truncated to `maxlen`, and return the characters
    /// actually stored. A bad address is diagnosed, not fatal.
    fn write_line_buffer(&mut self, buf: u32, maxlen: u32, unicode: bool, text: &str) -> Vec<char> {
        let chars: Vec<char> = text.chars().take(maxlen as usize).collect();
        for (i, &ch) in chars.iter().enumerate() {
            let cp = ch as u32;
            let res = if unicode {
                self.store_mem_sized(buf + i as u32 * 4, cp, 4)
            } else {
                self.store_mem_sized(buf + i as u32, cp & 0xFF, 1)
            };
            if let Err(e) = res {
                self.diagnostics.push(e);
                break;
            }
        }
        chars
    }

    /// `glk_set_terminators_line_event(win, keycodes, count)`: record the special
    /// keys that terminate line input on `win`. A null pointer or zero count
    /// clears the set (back to Enter-only); invalid keycodes are dropped by the
    /// model (Glk spec §11.2). Diagnose a bad window.
    fn glk_set_terminators(&mut self, win: u32, keycodes: u32, count: u32) -> R<()> {
        let mut keys = Vec::with_capacity(count as usize);
        if keycodes != 0 {
            for i in 0..count {
                keys.push(self.m32(keycodes + i * 4)?);
            }
        }
        if !self.glk.set_line_terminators(win, &keys) {
            self.diagnostics
                .push(format!("glk_set_terminators_line_event: bad window {win}"));
        }
        Ok(())
    }

    /// Record a pending char-input request; diagnose a bad window.
    fn glk_request_char(&mut self, win: u32, unicode: bool) {
        if !self.glk.request_char_event(win, unicode) {
            self.diagnostics
                .push(format!("glk_request_char_event: bad window {win}"));
        }
    }

    /// `glk_select(event_addr)`: deliver a queued non-input event immediately, or
    /// suspend on the first pending input request, or — with nothing to wait for
    /// — write `evtype_None` and continue (a malformed program would otherwise
    /// deadlock). Suspension is signaled to [`Machine::step`] via `pending_input`.
    fn glk_select(&mut self, event_addr: u32) -> R<()> {
        if event_addr == 0 {
            self.diagnostics.push("glk_select with a null event pointer".to_string());
            return Ok(());
        }
        if let Some(ev) = self.glk.pop_event() {
            return self.write_event(event_addr, ev); // arrange/redraw, no suspend
        }
        if let Some((win, unicode)) = self.glk.first_line_request() {
            self.pending_input = Some(PendingInput { event_addr, win, line: true, unicode });
        } else if let Some((win, unicode)) = self.glk.first_char_request() {
            self.pending_input = Some(PendingInput { event_addr, win, line: false, unicode });
        } else {
            // No line/char request. If a timer/mouse/hyperlink is armed, this is a
            // legal blocking select on a non-input event (Glk §4.4): suspend and
            // let the host deliver the event, rather than spinning on evtype_None.
            let timer_ms = self.glk_timer_interval();
            let mouse = self.glk.any_mouse_requested();
            let hyperlink = self.glk.any_hyperlink_requested();
            if timer_ms.is_some() || mouse || hyperlink {
                self.pending_event = Some(PendingEvent { event_addr, timer_ms, mouse, hyperlink });
            } else {
                // Nothing at all is armed: a malformed program that would otherwise
                // deadlock. Preserve the diagnostic + evtype_None escape hatch.
                self.diagnostics
                    .push("glk_select with no pending input request (returning evtype_None)".to_string());
                self.write_event(event_addr, GlkEvent::none())?;
            }
        }
        Ok(())
    }

    /// `glk_cancel_line_event(win, event)`: drop the pending line request and
    /// report what would have been the line event — `evtype_LineInput` with the
    /// count of characters already in the buffer (the `initlen`), or `evtype_None`
    /// if there was no request.
    fn glk_cancel_line(&mut self, win: u32, event_addr: u32) -> R<()> {
        let ev = match self.glk.take_line_request(win) {
            Some(r) => GlkEvent { etype: glk::evtype::LINE_INPUT, win, val1: r.initlen, val2: 0 },
            None => GlkEvent::none(),
        };
        self.clear_pending_input_for(win, true);
        if event_addr != 0 {
            self.write_event(event_addr, ev)?;
        }
        Ok(())
    }

    /// Clear a suspended `glk_select` if it was waiting on this window for the
    /// given input kind (line = `true`), so a cancel during suspension is safe.
    fn clear_pending_input_for(&mut self, win: u32, line: bool) {
        if matches!(&self.pending_input, Some(pi) if pi.win == win && pi.line == line) {
            self.pending_input = None;
        }
    }

    /// Deliver the 4-word Glk `event_t` (`type`, `win`, `val1`, `val2`) for the
    /// pointer `addr` — to memory, NULL-discarded, or pushed to the stack for the
    /// -1 convention (see [`Machine::glk_out_ref`]).
    fn write_event(&mut self, addr: u32, ev: GlkEvent) -> R<()> {
        // Trace the delivered event, decoded (SQ-0405). This is the single choke
        // point for every event — the async `glk_select` result (written on
        // resume) and the synchronous arrange/redraw/cancel events — so its
        // "meaning" surfaces here even though `glk_select` itself only suspends.
        if self.trace_screen {
            self.screen_trace.push(format!("event -> {}", glk_event_desc(&ev)));
        }
        self.glk_out_ref(addr, &[ev.etype, ev.win, ev.val1, ev.val2])
    }

    /// The [`StepResult`] for the current suspended `glk_select`, if any.
    fn suspend_result(&self) -> Option<StepResult> {
        if let Some(pi) = self.pending_input.as_ref() {
            return Some(if pi.line {
                StepResult::NeedLine { win: pi.win }
            } else {
                StepResult::NeedChar { win: pi.win, unicode: pi.unicode }
            });
        }
        self.pending_event.as_ref().map(|pe| StepResult::NeedEvent {
            timer_ms: pe.timer_ms,
            mouse: pe.mouse,
            hyperlink: pe.hyperlink,
        })
    }

    /// The [`StepResult`] for a suspended game `@save`/`@restore`, if any. The host
    /// resolves it via `complete_save`/`complete_restore_success`/`_failure`.
    fn saveload_result(&self) -> Option<StepResult> {
        self.pending_saveload.as_ref().map(|p| {
            if p.restore {
                StepResult::RestoreRequest
            } else {
                StepResult::SaveRequest
            }
        })
    }

    /// The [`StepResult`] for a suspended `glk_fileref_create_by_prompt`, if any.
    /// The host resolves it via [`Machine::supply_filename`].
    fn fileref_prompt_result(&self) -> Option<StepResult> {
        self.pending_fileref
            .as_ref()
            .map(|p| StepResult::NeedFilename { usage: p.usage, fmode: p.fmode })
    }

    /// Whether a game-initiated `@save`/`@restore` is currently suspended,
    /// awaiting the host's file I/O (`complete_save`/`complete_restore_success`/
    /// `complete_restore_quetzal`/`complete_restore_failure`). Hosts can use
    /// this to guard a snapshot trigger (e.g. an exit auto-save) against firing
    /// mid-suspension, which would capture an un-popped `@save` call stub.
    pub fn is_saveload_pending(&self) -> bool {
        self.pending_saveload.is_some()
    }

    /// The suspended `@save`/`@restore`'s routing info — the game's target file
    /// name and whether it is the player's prompted SAVE/RESTORE verb — for the
    /// host to decide between silent game-managed I/O and its own save UI.
    /// `None` when no save/restore is pending.
    pub fn pending_saveload_request(&self) -> Option<SaveLoadRequest> {
        self.pending_saveload.as_ref().map(|p| SaveLoadRequest {
            name: p.name.clone(),
            by_prompt: p.by_prompt,
            restore: p.restore,
        })
    }

    /// Deliver the result of a game-initiated `@save` back to the machine, then
    /// resume. Pops the call stub `@save` pushed for its S1 operand and stores
    /// the current-run result: 0 (success) or 1 (failure). Glulx spec §1.8.2,
    /// §2.9. A no-op if no `@save` is pending.
    pub fn complete_save(&mut self, ok: bool) {
        if let Some(p) = self.pending_saveload.take() {
            if ok {
                // Now that the host has committed the file, credit the game's
                // save stream — bumping its write_count and marking the SavedGame
                // slot's existence at the true byte count for the library's
                // post-@save verification (SQ-0301).
                self.glk.note_stream_write(p.save_stream, p.save_bytes);
            } else if p.fresh {
                // A cancelled brand-new save wrote no `.qzl`: undo the
                // open-for-write existence marking so does_file_exist reads false
                // (SQ-0301). Re-saves over a pre-existing slot keep their marker.
                self.glk.clear_saved_game_file(&p.name);
            }
            let _ = self.pop_save_stub_and_store(if ok { 0 } else { 1 });
        }
    }

    /// Complete a game-initiated `@restore` with the supplied save bytes.
    ///
    /// On success the machine state is replaced by the snapshot, whose PC sits just
    /// after the original `@save` — so execution simply resumes there and the
    /// `@restore`'s own S1 is discarded (this is the full host-snapshot path;
    /// it restores the raw stack byte for byte, including any not-yet-popped
    /// `@save` call stub, rather than understanding and popping it — see
    /// `restore_quetzal` for the stub-aware standard-save path). Returns `true`
    /// when the state was applied. On failure the machine is left as-is and the
    /// caller should call `complete_restore_failure`.
    pub fn complete_restore_success(&mut self, blob: &[u8]) -> bool {
        match self.restore_state(blob) {
            Ok(()) => {
                self.pending_saveload = None;
                self.undo_stack.clear();
                true
            }
            Err(e) => {
                self.diagnostics.push(format!("@restore failed: {e:?}"));
                false
            }
        }
    }

    /// Complete a game-initiated `@restore` of a standard `.qzl` (Glulx-Quetzal)
    /// blob written by [`Machine::save_quetzal`]/`@save`. Mirrors
    /// [`Machine::complete_restore_success`] but calls
    /// [`Machine::restore_quetzal`] (stub-based, live-state-preserving) instead
    /// of `restore_state`. Returns `true` when the state was applied. On
    /// failure the machine is left as-is and the caller should call
    /// `complete_restore_failure`.
    pub fn complete_restore_quetzal(&mut self, blob: &[u8]) -> bool {
        match self.restore_quetzal(blob) {
            Ok(()) => {
                self.pending_saveload = None;
                self.undo_stack.clear();
                true
            }
            Err(e) => {
                self.diagnostics.push(format!("@restore failed: {e:?}"));
                false
            }
        }
    }

    /// Signal that a game-initiated `@restore` failed (no data / invalid save):
    /// store 1 into the `@restore`'s S1 and resume just after the opcode.
    pub fn complete_restore_failure(&mut self) {
        if let Some(p) = self.pending_saveload.take() {
            let _ = self.store(p.dest, 1);
        }
    }

    /// Abandon a live `glk_select` suspension (and the Glk input request behind
    /// it) **without** delivering an event to the game, so the next
    /// [`Machine::step`] executes at the current PC instead of re-reporting the
    /// suspension.
    ///
    /// This exists for one caller: a HOST restore of a standard Glulx-Quetzal
    /// blob (`restore_quetzal`) into a session that is parked at its input
    /// prompt. Glulx spec §1.8.5 keeps Glk library state — windows, streams,
    /// I/O state — **out** of the save file, so a save blob can say nothing
    /// about the interpreter's own suspension; the host owns it. Leaving it in
    /// place would wedge the restore twice over: `step` short-circuits on the
    /// suspension and never reaches the restored PC, and a later `supply_line`
    /// would post its event to the event address of the run that just went
    /// away.
    ///
    /// The game's own `@restore` never needs this — it runs *between* selects,
    /// with no suspension live — and neither does [`Machine::restore_state`],
    /// which replaces the whole Glk model wholesale.
    pub fn abandon_pending_input(&mut self) {
        if let Some(pi) = self.pending_input.take() {
            self.glk.take_line_request(pi.win);
            self.glk.take_char_request(pi.win);
        }
        self.pending_event = None;
    }

    /// Complete a suspended `glk_fileref_create_by_prompt`. `Some(name)` binds a
    /// fileref to that (sanitized) name and stores its id into the `@glk`'s S1;
    /// `None` (the player cancelled) stores 0 (the Glk NULL fileref). No-op if none
    /// pending.
    pub fn supply_filename(&mut self, name: Option<String>) {
        if let Some(p) = self.pending_fileref.take() {
            let id = match name {
                // `by_prompt`: this fileref came from glk_fileref_create_by_prompt
                // (the player's SAVE/RESTORE verb), so its @save/@restore surfaces
                // the host save UI rather than a silent game-managed file.
                Some(n) => self.glk.fileref_create_prompted(p.usage, n, p.rock),
                None => 0,
            };
            let _ = self.store(p.dest, id);
        }
    }

    /// The user-visible VFS filenames (for a host `create_by_prompt` read picker).
    pub fn file_names(&self) -> Vec<String> {
        self.glk.file_names()
    }

    /// Complete a suspended line-input `glk_select` ended by the normal Enter
    /// key (terminator `val2` = 0). See [`Machine::supply_line_terminated`].
    pub fn supply_line(&mut self, text: &str) {
        self.supply_line_terminated(text, 0);
    }

    /// Complete a suspended line-input `glk_select`: write `text` into the
    /// request's Glulx buffer (truncated to `maxlen`, Latin-1 or 32-bit), fill
    /// the `event_t` with `evtype_LineInput` + the character count, and resume.
    /// If `terminator` is a special keycode the game registered for this window
    /// via `glk_set_terminators_line_event`, it is delivered in the event's
    /// second value (`val2`); otherwise `val2` is 0 (Glk spec §4.2 / §11.2).
    /// A no-op (with a diagnostic) if no line request is pending.
    pub fn supply_line_terminated(&mut self, text: &str, terminator: u32) {
        let pi = match self.pending_input.take() {
            Some(pi) if pi.line => pi,
            Some(pi) => {
                self.diagnostics
                    .push("supply_line called while a char event is pending".to_string());
                self.pending_input = Some(pi);
                return;
            }
            None => {
                self.diagnostics.push("supply_line with no pending line request".to_string());
                return;
            }
        };
        let req = self.glk.take_line_request(pi.win);
        let (buf, maxlen, unicode) = match req {
            Some(r) => (r.buf, r.maxlen, r.unicode),
            None => (0, 0, pi.unicode), // request vanished; still close the event safely
        };
        let chars = self.write_line_buffer(buf, maxlen, unicode, text);
        let n = chars.len() as u32;
        // Echo the completed line to the window's echo stream, if any: the input
        // text (as stored, truncated to `maxlen`) followed by a newline, written
        // straight to the echo stream — never to the window's own backend, so
        // there is no visible double echo (Glk spec §3.6; cheapglk
        // `gli_stream_echo_line`). Routed through `glk_stream_put` so a
        // memory/file echo stream records it and any window echo stream forwards
        // through the same depth-bounded machinery. Independent of
        // `glk_set_echo_line_event`, which the spec notes is "unrelated to the
        // window's echo stream".
        let echo = self.glk.window_echo_stream(pi.win);
        if echo != 0 {
            let mut echoed: String = chars.iter().collect();
            echoed.push('\n');
            self.glk_stream_put(echo, &echoed);
        }
        // Deliver the terminator keycode in val2 only if the game actually
        // registered it for this window; a normal Enter (or any other key)
        // reports 0 (Glk spec §4.2).
        let val2 = if terminator != 0 && self.glk.is_line_terminator(pi.win, terminator) {
            terminator
        } else {
            0
        };
        let ev = GlkEvent { etype: glk::evtype::LINE_INPUT, win: pi.win, val1: n, val2 };
        if let Err(e) = self.write_event(pi.event_addr, ev) {
            self.diagnostics.push(e);
        }
    }

    /// Complete a suspended char-input `glk_select`: fill the `event_t` with
    /// `evtype_CharInput` + the key code (mapped for a non-Unicode request: a
    /// Latin-1 code or a special keycode passes through; anything else becomes
    /// `keycode_Unknown`), and resume. A no-op (with a diagnostic) if no char
    /// request is pending.
    pub fn supply_char(&mut self, key: u32) {
        let pi = match self.pending_input.take() {
            Some(pi) if !pi.line => pi,
            Some(pi) => {
                self.diagnostics
                    .push("supply_char called while a line event is pending".to_string());
                self.pending_input = Some(pi);
                return;
            }
            None => {
                self.diagnostics.push("supply_char with no pending char request".to_string());
                return;
            }
        };
        let _ = self.glk.take_char_request(pi.win);
        // A Unicode request carries any code point; a non-Unicode request carries
        // only Latin-1 (<=0xFF) or special keycodes, and maps anything else to
        // keycode_Unknown.
        let val = if pi.unicode || key <= 0xFF || key >= glk::keycode::SPECIAL_FLOOR {
            key
        } else {
            glk::keycode::UNKNOWN
        };
        let ev = GlkEvent { etype: glk::evtype::CHAR_INPUT, win: pi.win, val1: val, val2: 0 };
        if let Err(e) = self.write_event(pi.event_addr, ev) {
            self.diagnostics.push(e);
        }
    }

    /// Deliver an `evtype_Arrange` into a suspended `glk_select`, if one is
    /// pending. Unlike a line/char event, an Arrange does **not** consume the
    /// window's input request: the event resolves this `glk_select`, the game
    /// runs its arrange handler (redrawing graphics windows to their new size),
    /// then loops back to `glk_select` and re-suspends on the still-pending
    /// request. A no-op when the VM is not currently waiting on input (e.g. it
    /// has quit) — an Arrange is only meaningful at a blocked `glk_select`.
    pub fn deliver_arrange(&mut self) {
        let ev = GlkEvent { etype: glk::evtype::ARRANGE, win: 0, val1: 0, val2: 0 };
        if let Some(pi) = self.pending_input.take() {
            if let Err(e) = self.write_event(pi.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else if let Some(pe) = self.pending_event.take() {
            if let Err(e) = self.write_event(pe.event_addr, ev) {
                self.diagnostics.push(e);
            }
        }
        // else: no-op — an Arrange is only meaningful at a blocked `glk_select`.
    }

    /// Deliver a Glk `Evtype_SoundNotify` for a finished sound: `sound` is the
    /// resource number, `notify` the value the game passed to
    /// `glk_schannel_play_ext`. Mirrors [`Machine::deliver_arrange`] — written
    /// directly into a suspended `glk_select` (without consuming the window's
    /// input request, so the game handles it and re-suspends), or queued for the
    /// next select when the VM is not currently blocked.
    pub fn deliver_sound_notify(&mut self, sound: u32, notify: u32) {
        let ev = GlkEvent { etype: glk::evtype::SOUND_NOTIFY, win: 0, val1: sound, val2: notify };
        if let Some(pi) = self.pending_input.take() {
            if let Err(e) = self.write_event(pi.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else if let Some(pe) = self.pending_event.take() {
            if let Err(e) = self.write_event(pe.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else {
            self.glk.push_event(ev);
        }
    }

    /// Deliver a Glk `Evtype_VolumeNotify` for a completed gradual volume change
    /// (`glk_schannel_set_volume_ext` with a nonzero `notify`). The host owns the
    /// ramp clock and calls this when the duration elapses. The event is
    /// `{ type: VolumeNotify, win: 0, val1: 0, val2: notify }` (Glk spec §8.3).
    /// Delivery mirrors [`Machine::deliver_sound_notify`].
    pub fn deliver_volume_notify(&mut self, notify: u32) {
        let ev = GlkEvent { etype: glk::evtype::VOLUME_NOTIFY, win: 0, val1: 0, val2: notify };
        if let Some(pi) = self.pending_input.take() {
            if let Err(e) = self.write_event(pi.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else if let Some(pe) = self.pending_event.take() {
            if let Err(e) = self.write_event(pe.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else {
            self.glk.push_event(ev);
        }
    }

    /// The currently armed Glk timer interval in milliseconds, or `None` when
    /// timer events are off. The host reads this to arm its own clock and fires
    /// [`Machine::deliver_timer`] once per interval. Set by
    /// `glk_request_timer_events`.
    pub fn glk_timer_interval(&self) -> Option<u32> {
        self.timer_interval_ms
    }

    /// Deliver a Glk `Evtype_Timer` event (a fired timer tick). Mirrors
    /// [`Machine::deliver_sound_notify`] — written directly into a suspended
    /// `glk_select` (without consuming the window's input request, so the game
    /// handles it and re-suspends), or queued for the next select when the VM is
    /// not currently blocked. The event is `{ type: Timer, win: 0, val1: 0,
    /// val2: 0 }` (Glk spec §4.4).
    pub fn deliver_timer(&mut self) {
        let ev = GlkEvent { etype: glk::evtype::TIMER, win: 0, val1: 0, val2: 0 };
        if let Some(pi) = self.pending_input.take() {
            if let Err(e) = self.write_event(pi.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else if let Some(pe) = self.pending_event.take() {
            if let Err(e) = self.write_event(pe.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else {
            self.glk.push_event(ev);
        }
    }

    /// Deliver a Glk `Evtype_MouseInput` for a terminal click at window-relative
    /// `(x, y)` (char col/row for a grid window, pixels for a graphics window).
    /// **One-shot**, unlike timer/sound/arrange: `take_mouse_request` both gates
    /// and clears the request, so a second `deliver_mouse` with no fresh
    /// `glk_request_mouse_event` is a no-op. Written directly into a suspended
    /// `glk_select`, or queued for the next select when the VM is not blocked.
    pub fn deliver_mouse(&mut self, win: u32, x: u32, y: u32) {
        if !self.glk.take_mouse_request(win) {
            return;
        }
        let ev = GlkEvent { etype: glk::evtype::MOUSE_INPUT, win, val1: x, val2: y };
        if let Some(pi) = self.pending_input.take() {
            if let Err(e) = self.write_event(pi.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else if let Some(pe) = self.pending_event.take() {
            if let Err(e) = self.write_event(pe.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else {
            self.glk.push_event(ev);
        }
    }

    /// Whether `win` currently has a pending Glk mouse request. The host reads
    /// this (per window in its layout) to decide whether a terminal click lands
    /// inside a mouse-watching window before calling [`Machine::deliver_mouse`].
    pub fn mouse_requested(&self, win: u32) -> bool {
        self.glk.mouse_requested(win)
    }

    /// Deliver a Glk `Evtype_Hyperlink` for a click on a hyperlink with value
    /// `linkval` in `win`. **One-shot**, like [`Machine::deliver_mouse`]:
    /// `take_hyperlink_request` both gates and clears the request, so a second
    /// `deliver_hyperlink` with no fresh `glk_request_hyperlink_event` is a no-op.
    /// Written directly into a suspended `glk_select`, or queued for the next
    /// select when the VM is not blocked.
    pub fn deliver_hyperlink(&mut self, win: u32, linkval: u32) {
        if !self.glk.take_hyperlink_request(win) {
            return;
        }
        let ev = GlkEvent { etype: glk::evtype::HYPERLINK, win, val1: linkval, val2: 0 };
        if let Some(pi) = self.pending_input.take() {
            if let Err(e) = self.write_event(pi.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else if let Some(pe) = self.pending_event.take() {
            if let Err(e) = self.write_event(pe.event_addr, ev) {
                self.diagnostics.push(e);
            }
        } else {
            self.glk.push_event(ev);
        }
    }

    /// Whether `win` currently has a pending Glk hyperlink request. The host reads
    /// this to decide whether a click on a link inside `win` should be delivered
    /// via [`Machine::deliver_hyperlink`].
    pub fn hyperlink_requested(&self, win: u32) -> bool {
        self.glk.hyperlink_requested(win)
    }

    /// The line-echo flag of window `win` (`glk_set_echo_line_event`; default true).
    pub fn window_line_echo(&self, win: u32) -> bool {
        self.glk.window_echo_line(win)
    }

    /// The window awaiting line input (lowest id if several), or `None`. The host
    /// routes its inline prompt/echo and scrollback to this window — the one the
    /// player is actually typing into — rather than assuming the first-opened
    /// buffer. (SQ-0337)
    pub fn line_request_window(&self) -> Option<u32> {
        self.glk.first_line_request().map(|(w, _)| w)
    }

    /// The resolved `GlkStyle::Input` colour for window `win` (for host input echo).
    pub fn window_input_colour(&self, win: u32) -> crate::glk::StyleColour {
        self.glk.window_input_colour(win)
    }

    /// Recompute the window layout from the backend's (freshly updated) screen
    /// size and notify a suspended game via an Arrange event. Call after the
    /// host reports a new display size: the relayout resizes graphics canvases
    /// to the new geometry, and the Arrange lets the game redraw into them.
    pub fn rearrange(&mut self) {
        self.relayout_glk();
        self.deliver_arrange();
    }

    /// The Glk version this layer implements (0.7.6), reported by
    /// `glk_gestalt(gestalt_Version)`.
    const GLK_VERSION: u32 = 0x0000_0706;

    /// Answer a `glk_gestalt` query. Truthful for what 3a-1 implements: output +
    /// Unicode are supported; graphics is supported conditionally (per
    /// `graphics_enabled`, see the selector 6/7/14 arms below); sound is
    /// supported conditionally (per `sound_enabled`, see the selector
    /// 8/9/10 arms below); timer and mouse input are supported.
    fn glk_gestalt(&self, sel: u32, val: u32) -> u32 {
        match sel {
            0 => Self::GLK_VERSION, // gestalt_Version
            1 => 1,                 // gestalt_CharInput → supported
            2 => 1,                 // gestalt_LineInput → supported
            3 => 2,                 // gestalt_CharOutput → ExactPrint for any char
            15 => 1,                // gestalt_Unicode
            16 => 1,                // gestalt_UnicodeNorm → canon_decompose/normalize supported
            17 => 1,                // gestalt_LineInputEcho → set_echo_line_event supported
            18 => 1,                // gestalt_LineTerminators → set_terminators_line_event supported
            19 => glk::keycode::is_terminator(val) as u32, // gestalt_LineTerminatorKey(keycode)
            // gestalt_MouseInput(wintype): grid + graphics support mouse; a generic
            // val==0 (AllTypes) probe answers "supported". TextBuffer/Pair/Blank do not.
            4 => matches!(val, 0 | 4 | 5) as u32,
            5 => 1,                 // gestalt_Timer → supported
            6 => self.graphics_enabled as u32,                // gestalt_Graphics
            7 => (self.graphics_enabled && (val == 5 || val == 3)) as u32, // gestalt_DrawImage(wintype): Graphics + TextBuffer (inline images)
            14 => self.graphics_enabled as u32,               // gestalt_GraphicsTransparency
            8 => self.sound_enabled as u32,  // gestalt_Sound
            9 => self.sound_enabled as u32,  // gestalt_SoundVolume
            10 => self.sound_enabled as u32, // gestalt_SoundNotify
            21 => self.sound_enabled as u32, // gestalt_Sound2 (0.7.3 extended sound suite)
            11 => 1,                 // gestalt_Hyperlinks → supported
            // gestalt_HyperlinkInput(wintype): the text windows (buffer + grid) carry
            // hyperlinks; a generic val==0 probe answers "supported". Graphics/Pair/Blank do not.
            12 => matches!(val, 0 | 3 | 4) as u32,
            0x1100 => 1,             // gestalt_GarglkText → garglk_set_zcolors/reversevideo supported
            20 => 1,                 // gestalt_DateTime → clock + date/time conversions supported
            22 => 1,                 // gestalt_ResourceStream → glk_stream_open_resource[_uni] supported
            // the rest are not supported.
            _ => 0,
        }
    }

    // ── the run loop ──────────────────────────────────────────────────────────

    /// Execute one instruction. Returns [`StepResult::Quit`] on `quit`, an outer
    /// return, or any fault (which is recorded in `diagnostics`); otherwise
    /// [`StepResult::Continue`]. Never panics.
    pub fn step(&mut self) -> StepResult {
        if self.halted {
            return StepResult::Quit;
        }
        // Still suspended on a prior glk_select: re-report until the host supplies.
        if let Some(sr) = self.suspend_result() {
            return sr;
        }
        // Still suspended on a prior @save/@restore: re-report until completed.
        if let Some(sr) = self.saveload_result() {
            return sr;
        }
        // Still suspended on a prior create_by_prompt: re-report until supplied.
        if let Some(sr) = self.fileref_prompt_result() {
            return sr;
        }
        // A fault latched inside a filter-iosys callback (see `pending_fault`)
        // counts as this instruction's fault — the machine abandoned a frame
        // mid-flight and must record-the-fault-and-quit (SQ-0625).
        let stepped = self.step_once().and_then(|()| match self.pending_fault.take() {
            Some(fault) => Err(fault),
            None => Ok(()),
        });
        match stepped {
            Ok(()) if self.halted => StepResult::Quit,
            // A glk_select this step may have suspended for input, or an
            // @save/@restore this step may have suspended for host file I/O.
            Ok(()) => self
                .suspend_result()
                .or_else(|| self.saveload_result())
                .or_else(|| self.fileref_prompt_result())
                .unwrap_or(StepResult::Continue),
            Err(msg) => {
                self.fault_trace = Some(self.build_trace(msg.clone()));
                self.diagnostics.push(msg);
                self.halted = true;
                StepResult::Quit
            }
        }
    }

    /// Drain the trace captured by the last fault, if any.
    pub fn take_fault_trace(&mut self) -> Option<crate::trace::StackTrace> {
        self.fault_trace.take()
    }

    /// Halt the machine with a synthetic recoverable fault, as if a runtime error
    /// had occurred. Used by the host to abort a runaway turn (an unbounded game
    /// loop) so the app can survive instead of hard-hanging. Records the same
    /// fault trace + diagnostic a real fault would, and halts; the next `step`
    /// returns `Quit`.
    pub fn abort_with_fault(&mut self, msg: String) {
        if self.halted {
            return;
        }
        self.fault_trace = Some(self.build_trace(msg.clone()));
        self.diagnostics.push(msg);
        self.halted = true;
    }

    /// Build a [`crate::trace::StackTrace`] by walking the frame-pointer chain
    /// from the current (innermost) frame down to the start frame (`fp == 0`).
    /// Read-only: never mutates the machine.
    fn build_trace(&self, fault: String) -> crate::trace::StackTrace {
        use crate::trace::StackTrace;
        let fault_op = self.opcode_name_at(self.instr_start_pc);
        let frames = self.call_frames();
        StackTrace { fault, fault_pc: self.instr_start_pc, fault_op, width: 4, frames }
    }

    /// Read a frame's local values by walking its (type,count) format list.
    fn read_frame_locals(&self, f: usize, localspos: usize) -> Vec<i64> {
        let mut out = Vec::new();
        let mut fmt = f + 8;
        let mut off = 0usize;
        'walk: loop {
            let ty = self.stack_byte(fmt);
            let count = self.stack_byte(fmt + 1);
            if ty == 0 && count == 0 {
                break;
            }
            let size = ty as usize; // Glulx local sizes: 1, 2, or 4 bytes
            if !matches!(size, 1 | 2 | 4) { break; }
            for _ in 0..count {
                off = align_up(off as u32, size as u32) as usize;
                let base = f + localspos + off;
                let v = match size {
                    1 => self.stack_byte(base) as i64,
                    2 => (((self.stack_byte(base) as u32) << 8) | self.stack_byte(base + 1) as u32) as i64,
                    _ => match self.st_r32_opt(base) {
                        Some(v) => v as i64,
                        None => break 'walk,
                    },
                };
                out.push(v);
                off += size.max(1);
            }
            fmt += 2;
            if out.len() > 256 {
                break;
            }
        }
        out
    }

    fn read_stack_words(&self, lo: usize, hi: usize) -> Vec<i64> {
        let mut out = Vec::new();
        let hi = hi.min(self.stack.len());
        if lo > hi {
            return out;
        }
        let mut a = lo;
        while a + 4 <= hi {
            match self.st_r32_opt(a) {
                Some(v) => out.push(v as i64),
                None => break,
            }
            a += 4;
        }
        out
    }

    fn stack_byte(&self, off: usize) -> u8 {
        self.stack.get(off).copied().unwrap_or(0)
    }

    fn opcode_name_at(&self, pc: u32) -> String {
        // Best-effort: decode just the opcode number at pc for a name.
        match self.mem.read8(pc) {
            Some(b) => opcode_name(b),
            None => "<unknown>".to_string(),
        }
    }

    /// Run until the machine quits.
    pub fn run(&mut self) {
        while self.step() == StepResult::Continue {}
    }

    /// Flush the display backend (e.g. at the end of a run).
    pub fn flush(&mut self) {
        self.backend.flush();
    }

    /// Mutable access to the display backend (e.g. to downcast in tests/host).
    pub fn backend_mut(&mut self) -> &mut dyn GlkBackend {
        &mut *self.backend
    }

    /// Notify the machine that the terminal was resized. Re-reads the display
    /// size from the backend (the caller must update the backend first), lays
    /// out all windows at the new size, and queues a Glk `evtype_Arrange`
    /// event so the game can redraw its layout on the next `glk_select`.
    pub fn notify_resize(&mut self) {
        self.relayout_glk();
        self.glk.push_event(GlkEvent {
            etype: glk::evtype::ARRANGE,
            win: 0,
            val1: 0,
            val2: 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm;
    use crate::glk::TestBackend;

    /// Nibble-order + padding edges of `read_operands` (Glulx spec §1.4 /
    /// GLULX_NOTES §3): modes pack two per byte with the LOW nibble naming the
    /// EARLIER operand, and an odd operand count leaves a final high nibble
    /// whose value decode must ignore. Hand-laid bytes, so a decoder that reads
    /// the nibbles high-first (or honours the padding) fails here rather than
    /// three opcodes into a story (SQ-1208's zero-allocation rewrite).
    #[test]
    fn read_operands_nibble_order_and_odd_padding() {
        // Body: raw operand block for a (2 load, 1 store) shape — never
        // executed, only decoded. 3 operands → 2 mode bytes.
        //   byte 0 = 0x21: op0 mode 0x1 (low nibble), op1 mode 0x2 (high)
        //   byte 1 = 0x78: op2 mode 0x8 (low), padding nibble 0x7 (ignored —
        //            0x7 would otherwise decode as a 4-byte memory operand)
        // Data: op0 = 0xFF (sign-extends), op1 = 0x1234; mode 0x8 has none.
        let body = [0x21, 0x78, 0xFF, 0x12, 0x34];
        let start = asm::func(0xC0, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = Machine::with_glk(Memory::new(built.image).unwrap(), Box::new(TestBackend::new()));
        let pc0 = m.pc; // frame entry left pc at the body's first byte

        let (l, s) = m.read_operands(2, 1).expect("decodes");
        assert_eq!(&l[..], &[0xFFFF_FFFF, 0x1234], "low nibble is the earlier operand");
        assert_eq!(&s[..], &[Dest::Push]);
        assert_eq!(m.pc, pc0 + 5, "2 mode bytes + 1 + 2 data bytes consumed");

        // Zero operands: no mode bytes, nothing consumed.
        let (l, s) = m.read_operands(0, 0).expect("decodes");
        assert!(l.is_empty() && s.is_empty());
        assert_eq!(m.pc, pc0 + 5);
    }

    /// Release-mode hot-loop micro-benchmark (SQ-0465 perf guard). Executes a
    /// tight decrement/branch loop of ~20M instructions and prints throughput.
    /// `#[ignore]`d (run via `cargo test --release -p gvm -- --ignored perf_hot_loop
    /// --nocapture`); used only to compare decode-split before/after numbers.
    #[test]
    #[ignore]
    fn perf_hot_loop() {
        use asm::Op::*;
        const N: u32 = 10_000_000;
        let body = [
            asm::ins(0x40, &[C32(N), Local8(0)]),         // copy #N -> L0
            asm::ins(0x11, &[Local8(0), C8(1), Local8(0)]), // sub L0,1 -> L0
            asm::ins(0x23, &[Local8(0), C16(-9)]),        // jnz L0, back-to-sub
            asm::ins(0x31, &[C8(0)]),                     // return 0
        ]
        .concat();
        let start = asm::func(0xC1, &[(4, 1)], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = Machine::with_glk(Memory::new(built.image).unwrap(), Box::new(TestBackend::new()));
        let t0 = std::time::Instant::now();
        m.run();
        let dt = t0.elapsed();
        let n = m.insn_count();
        eprintln!(
            "perf_hot_loop: {n} insns in {:.3}s = {:.1} Minsn/s",
            dt.as_secs_f64(),
            n as f64 / dt.as_secs_f64() / 1e6
        );
    }

    #[test]
    fn trace_exec_records_instruction_start_pcs() {
        // Off by default: no PCs recorded.
        let body = [
            asm::ins(0x40, &[asm::Op::C32(5), asm::Op::Local8(0)]),
            asm::ins(0x11, &[asm::Op::Local8(0), asm::Op::C8(1), asm::Op::Local8(0)]),
            asm::ins(0x31, &[asm::Op::Local8(0)]),
        ]
        .concat();
        let start = asm::func(0xC1, &[(4, 1)], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = Machine::with_glk(Memory::new(built.image.clone()).unwrap(), Box::new(TestBackend::new()));
        while m.step() == StepResult::Continue {}
        assert!(m.ever_executed.is_empty(), "tracing off records nothing");

        // On: every instruction-start PC is captured in both sets.
        let mut m = Machine::with_glk(Memory::new(built.image).unwrap(), Box::new(TestBackend::new()));
        m.trace_exec = true;
        let mut expected = std::collections::HashSet::new();
        loop {
            expected.insert(m.pc);
            if m.step() != StepResult::Continue {
                break;
            }
        }
        assert_eq!(m.executed_pcs, expected);
        assert_eq!(m.ever_executed, expected);

        // Per-turn clear empties executed_pcs but not ever_executed.
        m.clear_executed_pcs();
        assert!(m.executed_pcs.is_empty());
        assert_eq!(m.ever_executed, expected);
    }

    #[test]
    fn seed_ever_executed_merges_persisted_coverage() {
        let mut m = super::tests::machine_with_glk(&[]);
        let mut seed = std::collections::HashSet::new();
        seed.insert(0x1234);
        seed.insert(0x5678);
        m.seed_ever_executed(&seed);
        assert!(m.ever_executed.contains(&0x1234) && m.ever_executed.contains(&0x5678));
    }

    #[test]
    fn glk_trace_names_structural_selectors_and_skips_text_io() {
        // Structural calls get readable names; unknowns fall back to hex.
        assert_eq!(glk_selector_name(0x002A), "glk_window_clear");
        assert_eq!(glk_selector_name(0x1100), "garglk_set_zcolors");
        assert_eq!(glk_selector_name(0x0023), "glk_window_open");
        assert_eq!(glk_selector_name(0xDEAD), "glk[0xdead]");
        // The high-volume text I/O family is skipped so the trace stays readable;
        // structural calls are traced.
        assert!(is_glk_text_io(0x0080), "put_char is text I/O");
        assert!(is_glk_text_io(0x0082), "put_string is text I/O");
        assert!(is_glk_text_io(0x012A), "put_string_uni is text I/O");
        assert!(!is_glk_text_io(0x002A), "window_clear is structural");
        assert!(!is_glk_text_io(0x1100), "garglk_set_zcolors is structural");
        // Names come from the dispatch, not Glk-spec constants: tick is 0x0002 here.
        assert_eq!(glk_selector_name(0x0002), "glk_tick");
        assert_eq!(glk_selector_name(0x0042), "glk_stream_open_file");
        assert_eq!(glk_selector_name(0x0043), "glk_stream_open_memory");
        // Per-char case helpers are named but skipped from the trace (noise).
        assert_eq!(glk_selector_name(0x00A0), "glk_char_to_lower");
        assert!(is_glk_text_io(0x00A0));
    }

    #[test]
    fn glk_trace_args_decodes_colour_and_style_calls() {
        // stylehint_set(TextGrid, Normal, ReverseColor, 1)
        assert_eq!(glk_trace_args(0x00B0, &[4, 0, 9, 1]), "TextGrid, Normal, ReverseColor, 1");
        // stylehint_set(TextBuffer, Normal, BackColor, #FFFFFF)
        assert_eq!(glk_trace_args(0x00B0, &[3, 0, 8, 0xFFFFFF]), "TextBuffer, Normal, BackColor, #FFFFFF");
        // window_set_background_color(6, #000000)
        assert_eq!(glk_trace_args(0x00EB, &[6, 0]), "win=6, #000000");
        // garglk_set_zcolors: sentinels named, rgb as hex
        assert_eq!(glk_trace_args(0x1100, &[0x00FF0000, 0xFFFF_FFFF]), "fg=#FF0000, bg=default");
        // set_style(Header)
        assert_eq!(glk_trace_args(0x0086, &[3]), "Header");
        // gestalt selector decoded to its name
        assert_eq!(glk_trace_args(0x0004, &[15, 0]), "Unicode, 0");
        // glk_select's event pointer is labelled (the decoded event follows separately)
        assert_eq!(glk_trace_args(0x00C0, &[0x36E5DA]), "event=0x36E5DA");
        // request_line_event args are named
        assert_eq!(glk_trace_args(0x00D0, &[1, 0x1000, 256, 0]), "win=1, buf=0x1000, maxlen=256, initlen=0");
    }

    #[test]
    fn screen_trace_text_run_coalesces_and_labels_window_type() {
        // SQ-0405: story text is captured per target window (buffer vs grid) so
        // the trace shows WHERE output goes — e.g. a menu redraw into the primary
        // buffer accumulates, a grid does not.
        let mut m = super::tests::machine_with_glk(&[]);
        m.trace_screen = true;
        m.trace_text_append(1, "buf", "Basic ");
        m.trace_text_append(1, "buf", "Commands");
        assert!(m.screen_trace.is_empty(), "same-window text coalesces, not yet flushed");
        m.flush_text_run();
        assert_eq!(m.screen_trace, vec!["win 1 [buf] <- \"Basic Commands\"".to_string()]);

        // A run to a DIFFERENT window flushes the pending one first.
        m.screen_trace.clear();
        m.trace_text_append(2, "grid", "menu");
        m.trace_text_append(1, "buf", "body");
        assert_eq!(m.screen_trace, vec!["win 2 [grid] <- \"menu\"".to_string()]);
    }

    #[test]
    fn glk_out_result_reads_get_size_out_pointers() {
        // SQ-0405: glk_window_get_size returns its width/height through the
        // pointer args; the trace reads them back after the call for `-> W×H`.
        let mut m = super::tests::machine_with_glk(&[]);
        // Write 80/25 to two RAM cells, then decode as the get_size out-params.
        let (wp, hp) = (0x0120u32, 0x0124u32);
        if m.mem.write32(wp, 80).is_ok() && m.mem.write32(hp, 25).is_ok() {
            assert_eq!(m.glk_out_result(0x0025, &[2, wp, hp]).as_deref(), Some("80×25"));
        }
        // Null (value-not-wanted) and the -1 stack sentinel both show `?`.
        assert_eq!(m.glk_out_result(0x0025, &[2, 0, 0xffff_ffff]).as_deref(), Some("?×?"));
        // A non-out-pointer selector is not handled here (falls back to glk_trace_ret).
        assert_eq!(m.glk_out_result(0x0023, &[0, 0, 3]), None);
    }

    #[test]
    fn glk_trace_ret_labels_return_values() {
        // SQ-0405: value-returning selectors get a labelled `-> …` suffix.
        assert_eq!(glk_trace_ret(0x0023, 5), Some("win 5".to_string()));      // window_open
        assert_eq!(glk_trace_ret(0x0043, 3), Some("stream 3".to_string()));   // stream_open_memory
        assert_eq!(glk_trace_ret(0x0061, 2), Some("fileref 2".to_string()));  // fileref_create_by_name
        assert_eq!(glk_trace_ret(0x0028, 3), Some("TextBuffer".to_string())); // window_get_type
        assert_eq!(glk_trace_ret(0x0067, 1), Some("exists".to_string()));     // does_file_exist
        assert_eq!(glk_trace_ret(0x0067, 0), Some("absent".to_string()));
        assert_eq!(glk_trace_ret(0x0004, 1), Some("1".to_string()));          // gestalt value
        // A void-returning selector (window_clear) gets no suffix.
        assert_eq!(glk_trace_ret(0x002A, 0), None);
    }

    #[test]
    fn glk_event_desc_decodes_event_types_and_keycodes() {
        use crate::glk::{evtype, keycode, GlkEvent};
        let ev = |etype, win, val1, val2| GlkEvent { etype, win, val1, val2 };
        assert_eq!(glk_event_desc(&ev(evtype::LINE_INPUT, 5, 8, 0)), "LineInput win=5 len=8");
        assert_eq!(glk_event_desc(&ev(evtype::CHAR_INPUT, 5, keycode::RETURN, 0)), "CharInput win=5 key=Return");
        assert_eq!(glk_event_desc(&ev(evtype::CHAR_INPUT, 5, 'x' as u32, 0)), "CharInput win=5 key='x'");
        assert_eq!(glk_event_desc(&ev(evtype::ARRANGE, 0, 0, 0)), "Arrange win=0 val1=0 val2=0");
        // Function keys decode by number.
        assert_eq!(glk_keycode_name(keycode::FUNC1), "Func1");
        assert_eq!(glk_gestalt_name(0), "Version");
        assert_eq!(glk_gestalt_name(0x9999), "gestalt[0x9999]");
    }

    /// Build a test machine over `built` with a [`TestBackend`], then run the
    /// Glk prelude: open a TextBuffer window and make its stream current, so the
    /// hand-assembled programs (which print via `streamchar`/`glk_put_*`) have a
    /// window to print into — exactly as a real game does at startup.
    fn machine(built: asm::Built) -> Machine {
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        let win = m.glk_open_window(0, 0, 0, 3, 0); // wintype_TextBuffer = 3
        let sid = m.glk.window_stream(win).expect("buffer window has a stream");
        m.glk.set_current_stream(sid);
        m
    }

    /// A machine whose start function has `locals` and whose body is `body`,
    /// with `pc` positioned at the first body byte. RAMSTART is 0x100 (tiny
    /// code), so tests can hardcode RAM addresses.
    fn machine_with_body(locals: &[(u8, u8)], body: Vec<u8>) -> Machine {
        let start = asm::func(0xC1, locals, &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let m = machine(built);
        assert_eq!(m.mem.ramstart(), 0x100, "test assumes RAMSTART == 0x100");
        m
    }


    // ── SQ-0229: Glulx-Quetzal structural conformance ────────────────────────
    //
    // Nothing guarded the save FORMAT. The round-trip tests all restore through our OWN reader, so
    // any layout drift they introduce cancels out — a save that no other interpreter could read
    // would still pass them. These assert the BYTES against the Glulx spec (§1.8) instead.
    //
    // The limit is worth stating: this cannot catch a spec misreading shared by both the writer and
    // the reader. Only a cross-load against a foreign interpreter can, and that is still open on
    // SQ-0229 (it needs a headless glulxe built against cheapglk plus an authored fixture).

    /// Walk an IFF chunk list, asserting the container and even-padding as it goes.
    /// Returns `(id, data)` per chunk, in file order.
    fn iff_chunks(blob: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        assert!(blob.len() >= 12, "too short to be a FORM: {} bytes", blob.len());
        assert_eq!(&blob[0..4], b"FORM", "container is a FORM");
        assert_eq!(&blob[8..12], b"IFZS", "of type IFZS");
        let form_len = u32::from_be_bytes([blob[4], blob[5], blob[6], blob[7]]) as usize;
        assert_eq!(
            form_len + 8,
            blob.len(),
            "FORM length must cover exactly the rest of the file (a short/long count is what makes \
             a foreign reader give up)",
        );

        let mut out = Vec::new();
        let mut p = 12;
        while p < blob.len() {
            assert!(p + 8 <= blob.len(), "chunk header runs off the end at {p}");
            assert_eq!(p % 2, 0, "every chunk must start on an even offset; {p} is odd");
            let id = [blob[p], blob[p + 1], blob[p + 2], blob[p + 3]];
            let len =
                u32::from_be_bytes([blob[p + 4], blob[p + 5], blob[p + 6], blob[p + 7]]) as usize;
            let start = p + 8;
            assert!(start + len <= blob.len(), "chunk {:?} overruns the file", String::from_utf8_lossy(&id));
            out.push((id, blob[start..start + len].to_vec()));
            if len % 2 == 1 {
                assert_eq!(blob[start + len], 0, "the pad byte after an odd chunk must be zero");
            }
            p = start + len + (len & 1);
        }
        assert_eq!(p, blob.len(), "the chunk walk must land exactly on the end");
        out
    }

    fn chunk_ids(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<String> {
        chunks.iter().map(|(id, _)| String::from_utf8_lossy(id).to_string()).collect()
    }

    /// Decompress a CMem body: 4-byte memsize, then RLE where `00 n` is (n+1) zero bytes and any
    /// other byte is a literal (spec §1.8). Returns `(memsize, diff_bytes)`.
    fn decompress_cmem(body: &[u8]) -> (u32, Vec<u8>) {
        assert!(body.len() >= 4, "CMem must start with a 4-byte memsize");
        let memsize = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
        let mut out = Vec::new();
        let mut i = 4;
        while i < body.len() {
            if body[i] == 0 {
                assert!(i + 1 < body.len(), "a zero-run marker needs its count byte");
                let run = body[i + 1] as usize + 1;
                out.extend(std::iter::repeat_n(0u8, run));
                i += 2;
            } else {
                out.push(body[i]);
                i += 1;
            }
        }
        (memsize, out)
    }

    /// A machine that has actually RUN: RAM dirtied, a call frame live on the stack.
    ///
    /// Stepping matters — a freshly built machine has a pristine RAM image, so its CMem diff is all
    /// zeroes and every assertion about the diff would hold vacuously.
    fn conformance_machine() -> Machine {
        let body = [
            asm::ins(0x40, &[asm::Op::C32(0xDEAD_BEEF), asm::Op::Mem32(0x110)]), // copy → dirty RAM
            asm::ins(0x40, &[asm::Op::C32(0x0BAD_F00D), asm::Op::Mem32(0x118)]),
            asm::ins(0x120, &[]), // quit (never reached: we stop stepping before it)
        ]
        .concat();
        let mut m = machine_with_body(&[(4, 2)], body);
        for _ in 0..2 {
            assert_eq!(m.step(), StepResult::Continue, "the fixture's copies must execute");
        }
        assert_eq!(m.mem.read32(0x110), Some(0xDEAD_BEEF), "RAM is genuinely dirty now");
        m
    }

    #[test]
    fn an_odd_length_chunk_is_padded_to_even() {
        // IFF requires every chunk to occupy an even number of bytes, with the LENGTH field still
        // reporting the unpadded size. Drop the pad and every following chunk starts one byte late
        // — a foreign reader desyncs and gives up on the file.
        //
        // Tested on the writer directly, because no chunk a real save emits is odd-length: IFhd is
        // 128, MAll is 8+8n, Stks is 4-aligned, and CMem is only incidentally odd. Asserting this
        // through a save blob would pass whether or not the padding existed.
        let mut out = Vec::new();
        push_chunk(&mut out, b"TEST", &[1, 2, 3]);
        assert_eq!(out.len(), 12, "4 id + 4 len + 3 data + 1 pad");
        assert_eq!(&out[0..4], b"TEST");
        assert_eq!(
            u32::from_be_bytes([out[4], out[5], out[6], out[7]]),
            3,
            "the length field reports the UNPADDED size — the pad is not part of the chunk",
        );
        assert_eq!(out[11], 0, "the pad byte is zero");

        let mut even = Vec::new();
        push_chunk(&mut even, b"TEST", &[1, 2]);
        assert_eq!(even.len(), 10, "an even-length chunk takes no pad");

        // And the reader must skip the pad it just wrote — writer and reader agree.
        let mut blob = Vec::new();
        blob.extend_from_slice(b"FORM");
        let mut body = Vec::new();
        push_chunk(&mut body, b"AAAA", &[9]); // odd
        push_chunk(&mut body, b"BBBB", &[7, 7]);
        blob.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        blob.extend_from_slice(b"IFZS");
        blob.extend_from_slice(&body);
        let chunks = parse_ifzs(&blob).expect("round-trips through our own reader");
        assert_eq!(chunks.len(), 2, "the pad byte must not be mistaken for a chunk header");
        assert_eq!(chunks[0].1, &[9], "odd chunk keeps its exact data");
        assert_eq!(chunks[1].1, &[7, 7], "and the next chunk is found at the right offset");
    }

    #[test]
    fn save_quetzal_is_a_wellformed_ifzs_container() {
        let m = conformance_machine();
        let blob = m.save_quetzal();
        let chunks = iff_chunks(&blob); // asserts FORM/IFZS, lengths, even-padding

        // The spec-conformant @save format is EXACTLY these four, in this order. A foreign
        // interpreter reads this blob, so our own extensions must not leak into it.
        assert_eq!(
            chunk_ids(&chunks),
            vec!["IFhd", "CMem", "Stks", "MAll"],
            "@save emits the standard four chunks and nothing else",
        );
        for extra in ["GReg", "Glk "] {
            assert!(
                !chunk_ids(&chunks).contains(&extra.to_string()),
                "{extra} is a lanthorn extension and must stay out of the interop format",
            );
        }
    }

    #[test]
    fn save_state_is_the_same_container_plus_our_own_chunks() {
        // The host snapshot may carry extensions — a foreign terp is required to skip unknown
        // chunks — but it must still be a well-formed IFZS, and the standard four must lead.
        let m = conformance_machine();
        let blob = m.save_state();
        let chunks = iff_chunks(&blob);
        assert_eq!(
            chunk_ids(&chunks),
            vec!["IFhd", "CMem", "Stks", "MAll", "GReg", "Glk "],
            "the snapshot is the standard four, then our extensions",
        );
    }

    #[test]
    fn ifhd_is_the_first_128_bytes_of_memory() {
        let m = conformance_machine();
        for blob in [m.save_quetzal(), m.save_state()] {
            let chunks = iff_chunks(&blob);
            let (_, ifhd) = chunks.iter().find(|(id, _)| id == b"IFhd").expect("IFhd present");
            assert_eq!(ifhd.len(), 128, "IFhd is exactly the 128-byte header");
            let want: Vec<u8> = (0..128).map(|a| m.mem.read8(a).unwrap_or(0) as u8).collect();
            assert_eq!(*ifhd, want, "IFhd must BE the header — it is the identity a restore checks");
        }
    }

    #[test]
    fn cmem_decompresses_to_exactly_the_ram_extent() {
        let m = conformance_machine();
        let blob = m.save_quetzal();
        let chunks = iff_chunks(&blob);
        let (_, cmem) = chunks.iter().find(|(id, _)| id == b"CMem").expect("CMem present");
        let (memsize, diff) = decompress_cmem(cmem);

        assert_eq!(memsize, m.mem.mem_size(), "CMem's memsize prefix is the live memory size");
        assert_eq!(
            diff.len() as u32,
            memsize - m.mem.ramstart(),
            "the RLE must decompress to precisely [RAMSTART, memsize) — a length mismatch is the \
             classic silent corruption: a foreign terp restores a truncated or over-long RAM image",
        );

        // The diff is XOR-against-original, so it must reconstruct live RAM exactly.
        for (i, d) in diff.iter().enumerate() {
            let a = m.mem.ramstart() + i as u32;
            let want = m.mem.read8(a).unwrap_or(0) as u8;
            assert_eq!(d ^ m.mem.orig_byte(a), want, "RAM byte {a:#x} must round-trip through the diff");
        }
        assert!(diff.iter().any(|&b| b != 0), "the fixture actually dirtied RAM, so the test is not vacuous");
    }

    /// The slice-based `compress_ram` (SQ-1177) must be byte-identical to the
    /// per-byte original it replaced — the CMem body is an on-disk format, and
    /// "faster" is only allowed to mean faster. The reference below IS the old
    /// implementation, verbatim: `read8`/`orig_byte` a byte at a time, then the
    /// one-zero-at-a-time RLE.
    #[test]
    fn compress_ram_matches_the_per_byte_reference() {
        fn reference(m: &Machine) -> Vec<u8> {
            let ramstart = m.mem.ramstart();
            let memsize = m.mem.mem_size();
            let mut diff = Vec::with_capacity((memsize - ramstart) as usize);
            for a in ramstart..memsize {
                let cur = m.mem.read8(a).unwrap_or(0) as u8;
                diff.push(cur ^ m.mem.orig_byte(a));
            }
            let mut out = Vec::new();
            out.extend_from_slice(&memsize.to_be_bytes());
            let mut i = 0;
            while i < diff.len() {
                if diff[i] == 0 {
                    let mut run = 1usize;
                    while i + run < diff.len() && diff[i + run] == 0 && run < 256 {
                        run += 1;
                    }
                    out.push(0);
                    out.push((run - 1) as u8);
                    i += run;
                } else {
                    out.push(diff[i]);
                    i += 1;
                }
            }
            out
        }

        // A machine that has run, so the diff holds real dirt…
        let mut m = conformance_machine();
        assert_eq!(m.compress_ram(), reference(&m), "the fixture's own dirt");

        // …then every shape the encoder can meet: a literal stretch, a zero run
        // longer than one marker can carry (>256), single zeros between
        // literals, RAM grown past the original ENDMEM (where the diff base is
        // zeros rather than image bytes), and a literal on the very last byte.
        let ramstart = m.mem.ramstart();
        for (i, b) in [0xAA, 0xBB, 0xCC, 0x00, 0xDD].into_iter().enumerate() {
            m.mem.write_byte_raw(ramstart + 0x40 + i as u32, b);
        }
        assert!(m.mem.set_mem_size(m.mem.mem_size() + 512), "growable");
        m.mem.write_byte_raw(m.mem.mem_size() - 300, 0x11); // zero run > 256 after it
        m.mem.write_byte_raw(m.mem.mem_size() - 1, 0x22); // run ends on a literal
        assert_eq!(m.compress_ram(), reference(&m), "adversarial run shapes");

        // And the trailing-zero-run shape, which the loop above just removed.
        m.mem.write_byte_raw(m.mem.mem_size() - 1, 0x00);
        assert_eq!(m.compress_ram(), reference(&m), "a zero run to the very end");
    }

    #[test]
    fn stks_is_the_live_stack_and_its_bottom_frame_is_wellformed() {
        let m = conformance_machine();
        let blob = m.save_quetzal();
        let chunks = iff_chunks(&blob);
        let (_, stks) = chunks.iter().find(|(id, _)| id == b"Stks").expect("Stks present");

        assert_eq!(stks.len(), m.sp, "Stks is exactly the live stack [0, sp)");
        assert!(!stks.is_empty(), "the fixture is mid-call, so there is a frame to check");

        // Bottom call frame, per the spec's diagram: FrameLen, LocalsPos, the locals-format list
        // (pairs terminated by 0,0), the locals, then the value-stack region.
        let r32 = |o: usize| u32::from_be_bytes([stks[o], stks[o + 1], stks[o + 2], stks[o + 3]]);
        let frame_len = r32(0) as usize;
        let locals_pos = r32(4) as usize;
        assert!(frame_len >= 8, "FrameLen covers at least its own two header words: {frame_len}");
        assert!(frame_len <= stks.len(), "FrameLen {frame_len} overruns the saved stack");
        assert_eq!(frame_len % 4, 0, "FrameLen is 4-aligned (spec §1.3.2)");
        assert!(locals_pos >= 8 && locals_pos <= frame_len, "LocalsPos {locals_pos} sits inside the frame");

        // The format list must terminate with a (0,0) pair before the locals begin.
        let mut p = 8;
        let mut saw_terminator = false;
        while p + 1 < locals_pos {
            let (ty, count) = (stks[p], stks[p + 1]);
            if ty == 0 && count == 0 {
                saw_terminator = true;
                break;
            }
            assert!(matches!(ty, 1 | 2 | 4), "a locals-format type is 1, 2 or 4; got {ty}");
            p += 2;
        }
        assert!(saw_terminator, "the locals-format list must end with a (0,0) pair");
    }

    #[test]
    fn mall_is_self_consistent() {
        let m = conformance_machine();
        let blob = m.save_quetzal();
        let chunks = iff_chunks(&blob);
        let (_, mall) = chunks.iter().find(|(id, _)| id == b"MAll").expect("MAll present");

        assert!(mall.len() >= 8, "MAll carries at least heap-start and a block count");
        let heap_start = u32::from_be_bytes([mall[0], mall[1], mall[2], mall[3]]);
        let count = u32::from_be_bytes([mall[4], mall[5], mall[6], mall[7]]) as usize;
        assert_eq!(heap_start, m.heap_start, "MAll's heap-start is the live one");
        assert_eq!(
            mall.len(),
            8 + count * 8,
            "MAll is heap-start + count + exactly count (addr,len) pairs — a count that disagrees \
             with the body length is unrecoverable for a reader",
        );
    }

    #[test]
    fn a_save_survives_a_restore_byte_for_byte() {
        // The strongest drift guard available without a foreign interpreter: save → restore → save
        // must reproduce the identical blob. It catches any field the writer emits that the reader
        // drops on the floor (or vice versa) — the two halves cannot quietly disagree.
        let m = conformance_machine();
        let first = m.save_state();
        let mut m2 = conformance_machine();
        m2.restore_state(&first).expect("our own snapshot restores");
        let second = m2.save_state();
        assert_eq!(first, second, "save → restore → save is a fixed point");
    }

    /// A bare machine over a minimal image with a [`TestBackend`] and no
    /// windows opened — for tests that exercise `glk_gestalt`/`glk_open_window`
    /// directly without needing a printable window or a running program.
    fn machine_with_glk(body: &[u8]) -> Machine {
        let start = asm::func(0xC1, &[], body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mem = Memory::new(built.image).expect("valid image");
        Machine::with_glk(mem, Box::new(TestBackend::new()))
    }

    /// A bare machine (no windows opened) whose [`TestBackend`] reports a
    /// `(cols, rows)` screen and `(cw, ch)`-pixel character cells — for tests
    /// exercising graphics-window pixel↔cell layout conversion.
    fn machine_with_glk_charpx(cols: u32, rows: u32, cw: u32, ch: u32) -> Machine {
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x100);
        let mem = Memory::new(built.image).expect("valid image");
        let backend = TestBackend::with_screen(cols, rows).with_char_pixels(cw, ch);
        Machine::with_glk(mem, Box::new(backend))
    }

    impl Machine {
        /// Test accessor: a graphics window's `(width, height)` in pixels, per
        /// the backend's current `char_pixels()`.
        fn graphics_window_pixels(&self, win: u32) -> Option<(u32, u32)> {
            self.glk.window_pixel_size(win, self.backend.char_pixels())
        }
    }

    #[test]
    fn oob_load_captures_fault_trace() {
        use asm::Op::{Mem32, Stack};
        // copy from a wildly OOB main-memory address -> push (faults on the load).
        let body = asm::ins(0x40, &[Mem32(0x7FFF_FFFF), Stack]);
        let mut m = machine_with_body(&[], body);
        let mut steps = 0;
        loop {
            match m.step() {
                StepResult::Continue => {
                    steps += 1;
                    assert!(steps < 1000, "runaway");
                }
                StepResult::Quit => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        let t = m.take_fault_trace().expect("fault trace present");
        assert!(t.fault.starts_with("memory fault: "), "fault: {}", t.fault);
        assert_eq!(t.width, 4);
        assert!(!t.frames.is_empty());
        assert_eq!(t.frames[0].func_addr, 0, "gvm func_addr is always 0 (unknown)");
    }

    #[test]
    fn clean_quit_has_no_fault_trace() {
        let body = asm::ins(0x120, &[]); // quit
        let mut m = machine_with_body(&[], body);
        loop {
            match m.step() {
                StepResult::Quit => break,
                StepResult::Continue => {}
                o => panic!("{o:?}"),
            }
        }
        assert!(m.take_fault_trace().is_none());
    }

    #[test]
    fn game_save_restore_round_trips_and_stores_results() {
        // @save L1=0 S1->[0x110]; @restore L1=0 S1->[0x118]; quit.
        let body = [
            asm::ins(0x0123, &[asm::Op::Zero, asm::Op::Mem32(0x110)]),
            asm::ins(0x0124, &[asm::Op::Zero, asm::Op::Mem32(0x118)]),
            asm::ins(0x120, &[]),
        ]
        .concat();
        let mut m = machine_with_body(&[], body);

        // @save suspends with SaveRequest without writing S1 at all — it pushes
        // a call stub instead of baking a value, so S1 still reads fresh RAM (0)
        // at snapshot time.
        assert_eq!(m.step(), StepResult::SaveRequest);
        assert_eq!(m.mem.read32(0x110), Some(0), "@save no longer bakes a value into S1 pre-snapshot");
        let blob = m.save_state();

        // complete_save(true) pops the stub and stores the current-run success
        // code (0) into S1.
        m.complete_save(true);
        assert_eq!(m.mem.read32(0x110), Some(0), "complete_save(true) stores 0 into S1");

        // Mutate a witness RAM byte so a successful restore is observable.
        m.mem.write_byte_raw(0x120, 0xAB);
        assert_eq!(m.mem.read8(0x120), Some(0xAB));

        // Step to @restore -> RestoreRequest, then apply the captured blob.
        assert_eq!(m.step(), StepResult::RestoreRequest);
        assert!(m.complete_restore_success(&blob), "restore of our own blob must succeed");

        // State is back to the @save snapshot: S1 reads its pre-snapshot value
        // again (0), the witness byte reverted, and the pending state cleared.
        assert_eq!(m.mem.read32(0x110), Some(0), "restore reverts S1 to its pre-snapshot value");
        assert_eq!(m.mem.read8(0x120), Some(0), "restore reverts the mutated witness byte");
        assert!(m.pending_saveload.is_none(), "restore clears the pending save/restore");

        // The resumed PC sits just after the original @save (i.e. at the @restore),
        // so driving one more step re-suspends with RestoreRequest.
        assert_eq!(m.step(), StepResult::RestoreRequest, "restore resumes just after the original @save");
    }

    #[test]
    fn game_restore_failure_stores_one() {
        let body = [
            asm::ins(0x0124, &[asm::Op::Zero, asm::Op::Mem32(0x110)]), // @restore
            asm::ins(0x120, &[]),                                      // quit
        ]
        .concat();
        let mut m = machine_with_body(&[], body);
        assert_eq!(m.step(), StepResult::RestoreRequest);
        // A corrupt blob leaves state untouched and reports failure.
        assert!(!m.complete_restore_success(b"not a save"), "corrupt blob must fail");
        // The host then reports failure: S1 gets 1 and execution resumes to quit.
        m.complete_restore_failure();
        assert_eq!(m.mem.read32(0x110), Some(1), "complete_restore_failure stores 1 into S1");
        assert_eq!(m.step(), StepResult::Quit);
    }

    #[test]
    fn save_push_dest_leaves_single_result_on_stack() {
        // @save L1=0 S1->stack; quit. The store destination is Push (mode 0x8):
        // @save pushes a 4-word call stub (16 bytes) while suspended; complete_save
        // pops it and pushes exactly the current-run result, leaving one net
        // value on the stack — not the stub underneath it.
        let body = [
            asm::ins(0x0123, &[asm::Op::Zero, asm::Op::Stack]),
            asm::ins(0x120, &[]),
        ]
        .concat();
        let mut m = machine_with_body(&[], body);

        let before = m.value_count();
        assert_eq!(m.step(), StepResult::SaveRequest);
        assert_eq!(m.value_count(), before + 4, "a 4-word call stub was pushed for the Push destination");

        m.complete_save(true);
        assert_eq!(m.value_count(), before + 1, "complete_save pops the stub and pushes exactly the result");
        assert_eq!(m.pop32().unwrap(), 0, "complete_save(true) pushes the success code 0, not a stray stub underneath");
    }

    #[test]
    fn save_push_dest_failure() {
        let body = [
            asm::ins(0x0123, &[asm::Op::Zero, asm::Op::Stack]),
            asm::ins(0x120, &[]),
        ]
        .concat();
        let mut m = machine_with_body(&[], body);

        let before = m.value_count();
        assert_eq!(m.step(), StepResult::SaveRequest);
        m.complete_save(false);
        assert_eq!(m.value_count(), before + 1, "complete_save pops the stub and pushes exactly the result");
        assert_eq!(m.pop32().unwrap(), 1, "complete_save(false) pushes the failure code 1, not a stray stub underneath");
    }

    #[test]
    fn save_push_dest_restore_preserves_raw_stub_bytes() {
        // Mirrors game_save_restore_round_trips_and_stores_results but with a
        // Push store destination. save_state()/restore_state() operate BELOW
        // @save's own semantics: they restore the raw stack bytes exactly as
        // captured, including a not-yet-popped call stub if the snapshot was
        // taken mid-suspension. (Only save_quetzal()/restore_quetzal() — Task 2
        // — understand and pop that stub to resume with the -1 sentinel.)
        let body = [
            asm::ins(0x0123, &[asm::Op::Zero, asm::Op::Stack]),
            asm::ins(0x120, &[]),
        ]
        .concat();
        let mut m = machine_with_body(&[], body);

        let before = m.value_count();
        assert_eq!(m.step(), StepResult::SaveRequest);
        assert_eq!(m.value_count(), before + 4, "the pushed call stub is 4 words");
        let blob = m.save_state();

        m.complete_save(true);
        assert_eq!(m.pop32().unwrap(), 0, "current run: complete_save(true) pushes the success code 0");

        // Restore the snapshot captured at @save time (pre-complete_save): the
        // raw, still-unpopped stub reappears on the stack byte for byte.
        assert!(m.complete_restore_success(&blob), "restore of our own blob must succeed");
        assert_eq!(m.value_count(), before + 4, "restore reinstates the unpopped stub, byte for byte");
        // The stub's top (last-pushed) word is the caller frame pointer
        // captured at @save time — 0 for the top-level start frame — not a
        // semantic -1 sentinel (that convention belongs to restore_quetzal).
        assert_eq!(m.pop32().unwrap(), 0, "raw stub top word (caller fp), not a semantic sentinel");
    }

    #[test]
    fn build_trace_handles_corrupt_fp_without_panicking() {
        // Simulate the op_throw hazard: self.fp set to an attacker-influenced,
        // out-of-range stack offset before reload_frame_meta() had a chance to
        // validate it. build_trace() must never panic while reporting a fault.
        let mut m = machine_with_body(&[], asm::ins(0x120, &[])); // body unused
        m.fp = m.stack.len() + 64; // corrupt frame pointer, wildly out of range
        m.sp = m.fp;
        let t = m.build_trace("memory fault: test".to_string());
        assert_eq!(t.fault, "memory fault: test");
        assert_eq!(t.width, 4);
        // Frame-chain walk must bail out gracefully (truncated/empty), not panic.
        assert!(t.frames.len() <= 1, "expected a truncated trace, got {} frames", t.frames.len());
    }

    #[test]
    fn build_trace_handles_corrupt_frame_format_without_panicking() {
        // In-range corrupt fp: passes build_trace's length/alignment guards,
        // but the locals-format bytes at [fp+8, fp+9] decode to (ty=0, count>0),
        // which previously reached align_up(_, 0) and panicked (div by zero).
        let mut m = machine_with_body(&[], asm::ins(0x120, &[])); // body unused
        let f = 0usize;
        m.stack[f + 8] = 0; // ty = 0
        m.stack[f + 9] = 1; // count = 1 (nonzero, so the ty==0 && count==0 exit doesn't fire)
        m.fp = f;
        m.sp = m.stack.len();
        let t = m.build_trace("memory fault: test".to_string());
        assert_eq!(t.fault, "memory fault: test");
        assert_eq!(t.width, 4);
    }

    #[test]
    fn decode_opcode_1_2_4_byte_forms() {
        let mut m = machine_with_body(&[], asm::opcode_bytes(0x10));
        assert_eq!(m.decode_opcode().unwrap(), 0x10);
        let mut m = machine_with_body(&[], asm::opcode_bytes(0x130));
        assert_eq!(m.decode_opcode().unwrap(), 0x130);
        let mut m = machine_with_body(&[], asm::opcode_bytes(0x0100_0000));
        assert_eq!(m.decode_opcode().unwrap(), 0x0100_0000);
    }

    fn one_load(locals: &[(u8, u8)], op: asm::Op, setup: impl FnOnce(&mut Machine)) -> u32 {
        let mut m = machine_with_body(locals, asm::operands(&[op]));
        setup(&mut m);
        let (loads, _) = m.read_operands(1, 0).unwrap();
        loads[0]
    }

    #[test]
    fn load_mode_constants_sign_extend() {
        assert_eq!(one_load(&[], asm::Op::Zero, |_| {}), 0);
        assert_eq!(one_load(&[], asm::Op::C8(-5), |_| {}), (-5i32) as u32);
        assert_eq!(one_load(&[], asm::Op::C16(-1000), |_| {}), (-1000i32) as u32);
        assert_eq!(one_load(&[], asm::Op::C32(0xDEAD_BEEF), |_| {}), 0xDEAD_BEEF);
    }

    #[test]
    fn load_mode_contents_of_address() {
        // Address 0 holds the "Glul" magic.
        assert_eq!(one_load(&[], asm::Op::Mem8(0), |_| {}), 0x476C_756C);
        // 2-/4-byte address forms read a value we plant in RAM at 0x100.
        let plant = |m: &mut Machine| m.mem.write32(0x100, 0x0102_0304).unwrap();
        assert_eq!(one_load(&[], asm::Op::Mem16(0x0100), plant), 0x0102_0304);
        assert_eq!(one_load(&[], asm::Op::Mem32(0x0100), plant), 0x0102_0304);
    }

    #[test]
    fn load_mode_ram_relative() {
        let plant = |m: &mut Machine| m.mem.write32(0x100, 0x1111_2222).unwrap();
        assert_eq!(one_load(&[], asm::Op::Ram8(0), plant), 0x1111_2222);
        assert_eq!(one_load(&[], asm::Op::Ram16(0), plant), 0x1111_2222);
        assert_eq!(one_load(&[], asm::Op::Ram32(0), plant), 0x1111_2222);
    }

    #[test]
    fn load_mode_local_each_offset_size() {
        let set = |m: &mut Machine| m.local_store(4, 0x55AA).unwrap();
        assert_eq!(one_load(&[(4, 4)], asm::Op::Local8(4), set), 0x55AA);
        assert_eq!(one_load(&[(4, 4)], asm::Op::Local16(4), set), 0x55AA);
        assert_eq!(one_load(&[(4, 4)], asm::Op::Local32(4), set), 0x55AA);
    }

    #[test]
    fn load_mode_stack_pops() {
        let v = one_load(&[], asm::Op::Stack, |m| m.push32(0xFEED_FACE).unwrap());
        assert_eq!(v, 0xFEED_FACE);
    }

    #[test]
    fn store_mode_discard_and_push() {
        let mut m = machine_with_body(&[], asm::operands(&[asm::Op::Zero]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Discard);
        m.store(dests[0], 0x1234).unwrap(); // no-op

        let mut m = machine_with_body(&[], asm::operands(&[asm::Op::Stack]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Push);
        m.store(dests[0], 0x9).unwrap();
        assert_eq!(m.pop32().unwrap(), 0x9);
    }

    #[test]
    fn store_mode_memory_and_local() {
        // Memory (2-byte addr form) → RAM at 0x100.
        let mut m = machine_with_body(&[], asm::operands(&[asm::Op::Mem16(0x0100)]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Mem(0x100));
        m.store(dests[0], 0xABCD_1234).unwrap();
        assert_eq!(m.mem.read32(0x100), Some(0xABCD_1234));

        // RAM-relative store maps to the same address.
        let mut m = machine_with_body(&[], asm::operands(&[asm::Op::Ram8(0)]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Mem(0x100));

        // Local store.
        let mut m = machine_with_body(&[(4, 2)], asm::operands(&[asm::Op::Local8(4)]));
        let (_, dests) = m.read_operands(0, 1).unwrap();
        assert_eq!(dests[0], Dest::Local(4));
        m.store(dests[0], 0x7777).unwrap();
        assert_eq!(m.local_load(4).unwrap(), 0x7777);
    }

    // ── Task 4: arithmetic / bitwise / branch ─────────────────────────────────

    /// Execute one binary-op instruction storing to RAM 0x100; return the result.
    fn arith2(op: u32, a: asm::Op, b: asm::Op) -> u32 {
        let body = asm::ins(op, &[a, b, asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        m.mem.read32(0x100).unwrap()
    }
    fn arith1(op: u32, a: asm::Op) -> u32 {
        let body = asm::ins(op, &[a, asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        m.mem.read32(0x100).unwrap()
    }

    #[test]
    fn arithmetic_core() {
        use asm::Op::{C16, C32, C8};
        assert_eq!(arith2(0x10, C8(5), C8(7)), 12); // add
        assert_eq!(arith2(0x11, C8(5), C8(7)), (-2i32) as u32); // sub
        assert_eq!(arith2(0x12, C16(1000), C16(1000)), 1_000_000); // mul
        assert_eq!(arith2(0x12, C32(0x10000), C32(0x10000)), 0); // mul truncates
        assert_eq!(arith2(0x13, C8(-7), C8(2)), (-3i32) as u32); // div trunc toward 0
        assert_eq!(arith2(0x14, C8(-7), C8(2)), (-1i32) as u32); // mod sign of dividend
        assert_eq!(arith1(0x15, C8(5)), (-5i32) as u32); // neg
        assert_eq!(arith1(0x15, C8(-128)), 128); // neg of -128
    }

    #[test]
    fn div_mod_by_zero_faults() {
        let body = asm::ins(0x13, &[asm::Op::C8(5), asm::Op::C8(0), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        assert!(m.step_once().is_err());
        let body = asm::ins(0x14, &[asm::Op::C8(5), asm::Op::C8(0), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        assert!(m.step_once().is_err());
    }

    #[test]
    fn bitwise_and_shifts() {
        use asm::Op::{C32, C8};
        assert_eq!(arith2(0x18, C32(0xF0F0), C32(0xFF00)), 0xF000); // bitand
        assert_eq!(arith2(0x19, C32(0xF0F0), C32(0x0F0F)), 0xFFFF); // bitor
        assert_eq!(arith2(0x1A, C32(0xFF00), C32(0x0FF0)), 0xF0F0); // bitxor
        assert_eq!(arith1(0x1B, C32(0x0000_FFFF)), 0xFFFF_0000); // bitnot
        assert_eq!(arith2(0x1C, C8(1), C8(4)), 0x10); // shiftl
        assert_eq!(arith2(0x1C, C8(1), C8(32)), 0); // shiftl >= 32 → 0
        assert_eq!(arith2(0x1E, C32(0x8000_0000), C8(4)), 0x0800_0000); // ushiftr
        assert_eq!(arith2(0x1E, C32(0x8000_0000), C8(40)), 0); // ushiftr >= 32 → 0
        assert_eq!(arith2(0x1D, C32(0x8000_0000), C8(4)), 0xF800_0000); // sshiftr arith
        assert_eq!(arith2(0x1D, C32(0x8000_0000), C8(40)), 0xFFFF_FFFF); // sshiftr >= 32 → sign
    }

    #[test]
    fn jump_offset_math() {
        // jump +5: [0x20, modes(0x01), data(0x05)] = 3 bytes → pc = pc0+3+5-2.
        let body = asm::ins(0x20, &[asm::Op::C8(5)]);
        let mut m = machine_with_body(&[], body);
        let pc0 = m.pc;
        m.step_once().unwrap();
        assert_eq!(m.pc, pc0 + 6);
    }

    #[test]
    fn conditional_branch_taken_and_not() {
        // jz value=0 → taken (jumps forward).
        let body = asm::ins(0x22, &[asm::Op::C8(0), asm::Op::C8(20)]);
        let mut m = machine_with_body(&[], body);
        let pc0 = m.pc;
        let instr_len = body_len(0x22, 2);
        m.step_once().unwrap();
        assert_eq!(m.pc, pc0 + instr_len + 20 - 2);

        // jz value=1 → not taken (falls through past the instruction).
        let body = asm::ins(0x22, &[asm::Op::C8(1), asm::Op::C8(20)]);
        let mut m = machine_with_body(&[], body);
        let pc0 = m.pc;
        m.step_once().unwrap();
        assert_eq!(m.pc, pc0 + body_len(0x22, 2));
    }

    #[test]
    fn signed_vs_unsigned_compares() {
        // jlt(-1, 1) signed → taken; jltu(0xFFFFFFFF, 1) unsigned → not taken.
        let taken = |op: u32, a: asm::Op, b: asm::Op| -> bool {
            let body = asm::ins(op, &[a, b, asm::Op::C8(40)]);
            let blen = body.len() as u32;
            let mut m = machine_with_body(&[], body);
            let pc0 = m.pc;
            m.step_once().unwrap();
            // Not taken → pc falls through to pc0+blen; taken → pc differs.
            m.pc != pc0 + blen
        };
        assert!(taken(0x26, asm::Op::C8(-1), asm::Op::C8(1))); // jlt signed
        assert!(!taken(0x2A, asm::Op::C32(0xFFFF_FFFF), asm::Op::C8(1))); // jltu unsigned
        assert!(taken(0x2B, asm::Op::C32(0xFFFF_FFFF), asm::Op::C8(1))); // jgeu unsigned
    }

    #[test]
    fn branch_offset_0_and_1_return_convention() {
        // Inside a callee, `jump 0` returns 0 and `jump 1` returns 1 to the
        // caller's destination (here, the value stack).
        for (offset, expect) in [(0u32, 0u32), (1, 1)] {
            let start = asm::func(0xC1, &[], &[]);
            let off_op = if offset == 0 { asm::Op::Zero } else { asm::Op::C8(1) };
            let callee = asm::func(0xC1, &[], &asm::ins(0x20, &[off_op]));
            let built = asm::assemble(&[start, callee], 0, 0x100);
            let callee_addr = built.addrs[1];
            let mut m = machine(built);
            m.call_function(callee_addr, &[], Dest::Push).unwrap();
            m.step_once().unwrap(); // executes the jump → return
            assert_eq!(m.pop32().unwrap(), expect);
        }
    }

    /// Byte length of an instruction: 1-byte opcode (< 0x80) + ceil(n/2) mode
    /// bytes + n single-byte (C8) operands.
    fn body_len(_op: u32, n: u32) -> u32 {
        1 + n.div_ceil(2) + n
    }

    #[test]
    fn illegal_operand_mode_faults() {
        // Mode 0x4 is unused/illegal for load.
        let mut m = machine_with_body(&[], vec![0x04]);
        assert!(m.read_operands(1, 0).is_err());
        // Mode 0x1 (constant) is illegal as a store target.
        let mut m = machine_with_body(&[], vec![0x01, 0x00]);
        assert!(m.read_operands(0, 1).is_err());
    }

    // ── Task 5: stack ops, copy, memsize, call variants ───────────────────────

    #[test]
    fn copy_and_sign_extend() {
        // copy: full 32-bit through RAM.
        let body = asm::ins(0x40, &[asm::Op::C32(0xDEAD_BEEF), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0xDEAD_BEEF);

        // copys: write only 2 bytes to memory (no sign extension).
        let body = asm::ins(0x41, &[asm::Op::C32(0x1234_5678), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.mem.write32(0x100, 0xFFFF_FFFF).unwrap();
        m.step_once().unwrap();
        // Big-endian: the 2-byte write at 0x100 lands in the high-order bytes
        // (0x5678), leaving the low half (0x102..0x104) unchanged (FFFF).
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x5678_FFFF);

        // copyb: write only 1 byte (the top byte at 0x100).
        let body = asm::ins(0x42, &[asm::Op::C32(0x1234_5678), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.mem.write32(0x100, 0).unwrap();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x7800_0000);

        // sexs / sexb sign-extend.
        let sx = |op: u32, v: u32| {
            let body = asm::ins(op, &[asm::Op::C32(v), asm::Op::Mem16(0x0100)]);
            let mut m = machine_with_body(&[], body);
            m.step_once().unwrap();
            m.mem.read32(0x100).unwrap()
        };
        assert_eq!(sx(0x44, 0x0000_8000), 0xFFFF_8000); // sexs negative
        assert_eq!(sx(0x44, 0x0000_7FFF), 0x0000_7FFF); // sexs positive
        assert_eq!(sx(0x45, 0x0000_0080), 0xFFFF_FF80); // sexb negative
        assert_eq!(sx(0x45, 0x0000_007F), 0x0000_007F); // sexb positive
    }

    #[test]
    fn stkcount_peek_swap() {
        // stkcount → RAM.
        let body = asm::ins(0x50, &[asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.push32(1).unwrap();
        m.push32(2).unwrap();
        m.push32(3).unwrap();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 3);

        // stkpeek index 1 (does not pop).
        let body = asm::ins(0x51, &[asm::Op::C8(1), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.push32(0xAA).unwrap();
        m.push32(0xBB).unwrap();
        m.push32(0xCC).unwrap(); // top
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0xBB);
        assert_eq!(m.value_count(), 3);

        // stkswap.
        let body = asm::ins(0x52, &[]);
        let mut m = machine_with_body(&[], body);
        m.push32(1).unwrap();
        m.push32(2).unwrap();
        m.step_once().unwrap();
        assert_eq!(m.pop32().unwrap(), 1);
        assert_eq!(m.pop32().unwrap(), 2);
    }

    #[test]
    fn stkcopy_duplicates_top_n_in_order() {
        let body = asm::ins(0x54, &[asm::Op::C8(2)]);
        let mut m = machine_with_body(&[], body);
        m.push32(10).unwrap();
        m.push32(20).unwrap();
        m.push32(30).unwrap();
        m.step_once().unwrap();
        assert_eq!(m.value_count(), 5);
        // ...10,20,30,20,30 (top).
        for expect in [30, 20, 30, 20, 10] {
            assert_eq!(m.pop32().unwrap(), expect);
        }
    }

    #[test]
    fn stkroll_rotates_per_spec_example() {
        // Spec example: 8 7 6 5 4 3 2 1 0<top>; stkroll 5 1 → 8 7 6 5 0 4 3 2 1<top>.
        let body = asm::ins(0x53, &[asm::Op::C8(5), asm::Op::C8(1)]);
        let mut m = machine_with_body(&[], body);
        for v in [8u32, 7, 6, 5, 4, 3, 2, 1, 0] {
            m.push32(v).unwrap();
        }
        m.step_once().unwrap();
        for expect in [1, 2, 3, 4, 0, 5, 6, 7, 8] {
            assert_eq!(m.pop32().unwrap(), expect);
        }
    }

    #[test]
    fn get_and_set_memsize() {
        let body = asm::ins(0x102, &[asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        let size = m.mem.mem_size();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), size);

        // setmemsize success → result 0, size grows.
        let body = asm::ins(0x103, &[asm::Op::C16(0x300), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0); // success
        assert_eq!(m.mem.mem_size(), 0x300);

        // setmemsize failure (unaligned) → result 1, size unchanged.
        let body = asm::ins(0x103, &[asm::Op::C16(0x250), asm::Op::Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        let before = m.mem.mem_size();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 1); // failure
        assert_eq!(m.mem.mem_size(), before);
    }

    #[test]
    fn call_takes_args_from_stack_and_returns() {
        // callee (index 0, addr 0x24) returns the constant 0x123.
        let callee = asm::func(0xC1, &[], &asm::ins(0x31, &[asm::Op::C16(0x123)]));
        let start = asm::func(
            0xC1,
            &[],
            &asm::ins(0x30, &[asm::Op::C32(0x24), asm::Op::C8(0), asm::Op::Mem16(0x0100)]),
        );
        let built = asm::assemble(&[callee, start], 1, 0x100);
        assert_eq!(built.addrs[0], 0x24);
        let mut m = machine(built);
        m.step_once().unwrap(); // call
        m.step_once().unwrap(); // return
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x123);
    }

    #[test]
    fn callfi_passes_one_arg() {
        // callee returns its single local (the argument).
        let callee = asm::func(0xC1, &[(4, 1)], &asm::ins(0x31, &[asm::Op::Local8(0)]));
        let start = asm::func(
            0xC1,
            &[],
            &asm::ins(0x161, &[asm::Op::C32(0x24), asm::Op::C16(0x99), asm::Op::Mem16(0x0100)]),
        );
        let built = asm::assemble(&[callee, start], 1, 0x100);
        let mut m = machine(built);
        m.step_once().unwrap(); // callfi
        m.step_once().unwrap(); // return
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x99);
    }

    #[test]
    fn callfii_sums_two_args() {
        // callee: add local0 + local1 → stack, then return it.
        let mut body = asm::ins(0x10, &[asm::Op::Local8(0), asm::Op::Local8(4), asm::Op::Stack]);
        body.extend(asm::ins(0x31, &[asm::Op::Stack]));
        let callee = asm::func(0xC1, &[(4, 2)], &body);
        let start = asm::func(
            0xC1,
            &[],
            &asm::ins(
                0x162,
                &[asm::Op::C32(0x24), asm::Op::C8(10), asm::Op::C8(20), asm::Op::Mem16(0x0100)],
            ),
        );
        let built = asm::assemble(&[callee, start], 1, 0x100);
        let mut m = machine(built);
        m.step_once().unwrap(); // callfii
        m.step_once().unwrap(); // add
        m.step_once().unwrap(); // return
        assert_eq!(m.mem.read32(0x100).unwrap(), 30);
    }

    #[test]
    fn tailcall_reuses_caller_stub() {
        // f2 (addr 0x24) returns 0x77. f1 tailcalls f2. start calls f1 storing to
        // RAM 0x100. Because tailcall reuses f1's stub, 0x77 lands at 0x100.
        let f2 = asm::func(0xC1, &[], &asm::ins(0x31, &[asm::Op::C16(0x77)]));
        let f1_addr = 0x24 + f2.len() as u32;
        let f1 = asm::func(0xC1, &[], &asm::ins(0x34, &[asm::Op::C32(0x24), asm::Op::C8(0)]));
        let start = asm::func(
            0xC1,
            &[],
            &asm::ins(0x30, &[asm::Op::C32(f1_addr), asm::Op::C8(0), asm::Op::Mem16(0x0100)]),
        );
        let built = asm::assemble(&[f2, f1, start], 2, 0x100);
        assert_eq!(built.addrs[1], f1_addr);
        let mut m = machine(built);
        m.step_once().unwrap(); // start: call f1
        m.step_once().unwrap(); // f1: tailcall f2
        m.step_once().unwrap(); // f2: return 0x77
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x77);
        assert!(!m.halted); // returned into start, not halted
    }

    // ── Task 4: accelerated-function interception ─────────────────────────────

    /// A RAM address (within [ramstart, endmem), ≥36) marked as a routine
    /// (type byte 0xC0), so `Z__Region(ROUTINE_ADDR) == 2`.
    const ROUTINE_ADDR: u32 = 0x104;

    /// Builds a machine with a function `FADDR` whose *bytecode* returns the
    /// sentinel `0xBAD`, assigned accel number 1 (`Z__Region`) via
    /// `set_accel_func`. Returns the machine and `FADDR`.
    fn accel_installed_machine() -> (Machine, u32) {
        let faddr_func = asm::func(0xC1, &[], &asm::ins(0x31, &[asm::Op::C16(0x0BAD)]));
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[faddr_func, start], 1, 0x100);
        let faddr = built.addrs[0];
        let mut m = machine(built);
        m.mem.write8(ROUTINE_ADDR, 0xC0).unwrap();
        m.set_accel_func(faddr, 1); // Z__Region
        (m, faddr)
    }

    #[test]
    fn call_uses_accelerated_function_when_installed() {
        let (mut m, faddr) = accel_installed_machine();
        m.call_function(faddr, &[ROUTINE_ADDR], Dest::Push).unwrap();
        assert_eq!(m.pop32().unwrap(), 2); // Z__Region(ROUTINE_ADDR) == 2, not 0xBAD
    }

    #[test]
    fn no_accel_runs_the_bytecode() {
        let (mut m, faddr) = accel_installed_machine();
        m.set_acceleration(false);
        m.call_function(faddr, &[ROUTINE_ADDR], Dest::Push).unwrap();
        m.step_once().unwrap(); // interpreted path: execute the callee's `return 0xBAD`
        assert_eq!(m.pop32().unwrap(), 0x0BAD);
    }

    #[test]
    fn tailcall_uses_accelerated_function() {
        // f2 (FADDR, addr 0x24): bytecode returns 0xBAD if actually run.
        let f2 = asm::func(0xC1, &[], &asm::ins(0x31, &[asm::Op::C16(0x0BAD)]));
        let f1_addr = 0x24 + f2.len() as u32;
        // f1: push ROUTINE_ADDR (the arg), then tailcall f2(ROUTINE_ADDR).
        let mut f1_body = asm::ins(0x40, &[asm::Op::C32(ROUTINE_ADDR), asm::Op::Stack]);
        f1_body.extend(asm::ins(0x34, &[asm::Op::C32(0x24), asm::Op::C8(1)]));
        let f1 = asm::func(0xC1, &[], &f1_body);
        let start = asm::func(
            0xC1,
            &[],
            &asm::ins(0x30, &[asm::Op::C32(f1_addr), asm::Op::C8(0), asm::Op::Mem16(0x0100)]),
        );
        let built = asm::assemble(&[f2, f1, start], 2, 0x100);
        assert_eq!(built.addrs[1], f1_addr);
        let mut m = machine(built);
        m.mem.write8(ROUTINE_ADDR, 0xC0).unwrap();
        m.set_accel_func(0x24, 1); // Z__Region

        m.step_once().unwrap(); // start: call f1
        m.step_once().unwrap(); // f1: push arg
        m.step_once().unwrap(); // f1: tailcall f2 (accelerated → delivers to start's stub)
        assert_eq!(m.mem.read32(0x100).unwrap(), 2); // Z__Region(ROUTINE_ADDR) == 2, not 0xBAD
        assert!(!m.halted); // returned into start, not halted
    }

    // ── Task 5 (accel): differential native-vs-interpreted Z__Region ──────────
    //
    // Hand-transcribes algorithms.md §1's Z__Region into Glulx bytecode (a
    // "veneer"), runs it *interpreted* (acceleration off), and checks it agrees
    // with `accel_dispatch(1, ..)` (the native path) for the same inputs. This
    // is the differential harness from spec Task 5: best-effort, Z__Region only
    // (the property functions' veneers are far more involved and are left to
    // Task 8's full-story on/off equivalence, the primary anti-divergence
    // guarantee).
    //
    // Veneer control flow (local0 = addr, local1(byte) = tb = mem[addr]):
    //   if addr < 36            -> return 0   (branch-offset-0 shortcut)
    //   if addr >= endmem       -> return 0   (branch-offset-0 shortcut)
    //   tb = aloadb(addr, 0)
    //   if tb < 0xE0 goto L1 else return 3
    //   L1: if tb < 0xC0 goto L2 else return 2
    //   L2: if tb < 0x70 goto RET0
    //   if tb > 0x7F goto RET0
    //   if addr < ramstart goto RET0
    //   return 1
    //   RET0: return 0
    //
    // ramstart/endmem are read live from the header (Mem32(0x08)/Mem32(0x10))
    // rather than hardcoded, so the veneer tracks whatever image it runs in.

    /// The Z__Region veneer body. All conditional branches use a fixed-width
    /// `C16` offset operand, so each branch instruction's byte length doesn't
    /// depend on the (forward-only) offset value it carries — lengths of the
    /// pieces already emitted are enough to compute every jump target.
    fn z_region_veneer_body() -> Vec<u8> {
        use asm::Op::{C8, C16, C32, Local8, Mem32, Zero};

        let addr = Local8(0);
        let tb = Local8(4);

        let guard_lt36 = asm::ins(0x2A, &[addr, C32(36), Zero]); // jltu addr,36 -> return 0
        let guard_endmem = asm::ins(0x2B, &[addr, Mem32(0x10), Zero]); // jgeu addr,endmem -> return 0
        let load_tb = asm::ins(0x4A, &[addr, C8(0), tb]); // aloadb: tb = mem[addr]
        let ret3 = asm::ins(0x31, &[C8(3)]);
        let ret2 = asm::ins(0x31, &[C8(2)]);
        let ret1 = asm::ins(0x31, &[C8(1)]);
        let ret0 = asm::ins(0x31, &[Zero]);

        // Every branch below shares this shape (Local8 + 4-byte constant/mem +
        // C16 offset), so they're all the same length.
        let branch_len = asm::ins(0x2A, &[addr, C32(0), C16(0)]).len() as i16;

        let off_skip_ret3 = ret3.len() as i16 + 2; // jltu tb,0xE0 -> just past ret3
        let off_skip_ret2 = ret2.len() as i16 + 2; // jltu tb,0xC0 -> just past ret2
        let off_to_ret0_from_g = ret1.len() as i16 + 2; // jltu addr,ramstart -> ret0
        let off_to_ret0_from_f = branch_len + off_to_ret0_from_g; // jgtu tb,0x7F -> ret0 (over G)
        let off_to_ret0_from_e = branch_len + off_to_ret0_from_f; // jltu tb,0x70 -> ret0 (over F,G)

        let branch_e = asm::ins(0x2A, &[tb, C32(0x70), C16(off_to_ret0_from_e)]);
        let branch_f = asm::ins(0x2C, &[tb, C32(0x7F), C16(off_to_ret0_from_f)]);
        let branch_g = asm::ins(0x2A, &[addr, Mem32(0x08), C16(off_to_ret0_from_g)]); // ramstart

        let mut body = Vec::new();
        body.extend(guard_lt36);
        body.extend(guard_endmem);
        body.extend(load_tb);
        body.extend(asm::ins(0x2A, &[tb, C32(0xE0), C16(off_skip_ret3)])); // tb<0xE0 -> skip ret3
        body.extend(ret3);
        body.extend(asm::ins(0x2A, &[tb, C32(0xC0), C16(off_skip_ret2)])); // tb<0xC0 -> skip ret2
        body.extend(ret2);
        body.extend(branch_e);
        body.extend(branch_f);
        body.extend(branch_g);
        body.extend(ret1);
        body.extend(ret0);
        body
    }

    /// Builds a machine whose function `FADDR` is the Z__Region veneer
    /// (locals: local0 = addr (word), local1 = tb (byte)), plus a trivial
    /// `start` function. Returns the machine, `FADDR`, and object/routine/
    /// string RAM addresses (type bytes 0x70/0xC0/0xE0) at `ramstart..`.
    fn z_region_differential_machine() -> (Machine, u32, u32, u32, u32) {
        let veneer = asm::func(0xC1, &[(4, 1), (1, 1)], &z_region_veneer_body());
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[veneer, start], 1, 0x100);
        let faddr = built.addrs[0];
        let mut m = machine(built);
        assert_eq!(m.mem.ramstart(), 0x100, "test assumes RAMSTART == 0x100");

        let obj_addr = m.mem.ramstart();
        let routine_addr = obj_addr + 4;
        let string_addr = obj_addr + 8;
        m.mem.write8(obj_addr, 0x70).unwrap();
        m.mem.write8(routine_addr, 0xC0).unwrap();
        m.mem.write8(string_addr, 0xE0).unwrap();

        (m, faddr, obj_addr, routine_addr, string_addr)
    }

    #[test]
    fn differential_z_region_matches_interpreter() {
        let (mut m, faddr, obj_addr, routine_addr, string_addr) = z_region_differential_machine();
        let endmem = m.mem.endmem();
        let inputs = [0u32, 35, 36, obj_addr, routine_addr, string_addr, endmem];

        for &addr in &inputs {
            m.set_acceleration(true);
            let native = m.accel_dispatch(1, &[addr]).unwrap();

            m.set_acceleration(false);
            let entry_fp = m.fp;
            m.call_function(faddr, &[addr], Dest::Push).unwrap();
            let mut steps = 0;
            while m.fp != entry_fp {
                m.step_once().unwrap();
                steps += 1;
                assert!(steps < 100, "veneer did not return for addr {addr:#x}");
            }
            assert!(!m.halted);
            let interp = m.pop32().unwrap();

            assert_eq!(native, interp, "Z__Region diverged at {addr:#x}: native={native} interp={interp}");
        }
    }

    // ── Task 6: output, iosys, @glk, run loop ─────────────────────────────────

    /// All text the program printed: the migrated `TestBackend`'s accumulated
    /// text-buffer content (the prelude opens exactly one window).
    fn out_str(m: &Machine) -> String {
        m.backend.as_any().downcast_ref::<TestBackend>().unwrap().all_text()
    }

    /// Build + run a start function with body `body`; return its output.
    fn run_program(body: Vec<u8>) -> Machine {
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        m.run();
        m
    }

    #[test]
    fn streamnum_and_streamchar_under_glk_iosys() {
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]); // setiosys glk
        body.extend(asm::ins(0x71, &[asm::Op::C8(42)])); // streamnum 42
        body.extend(asm::ins(0x71, &[asm::Op::C8(-7)])); // streamnum -7
        body.extend(asm::ins(0x70, &[asm::Op::C8(65)])); // streamchar 'A'
        body.extend(asm::ins(0x120, &[])); // quit
        let m = run_program(body);
        assert_eq!(out_str(&m), "42-7A");
        assert!(m.diagnostics.is_empty());
    }

    #[test]
    fn null_iosys_discards_output() {
        // Default iosys is null (0); streamchar produces nothing.
        let mut body = asm::ins(0x70, &[asm::Op::C8(88)]); // streamchar 'X'
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(out_str(&m), "");
    }

    #[test]
    fn filter_iosys_calls_function_per_char() {
        use asm::Op::{C8, C32, Local32, Mem32};
        const COUNTER_ADDR: u32 = 0x100;
        const LOG_BASE: u32 = 0x110;
        // The filter function is assembled first, so its address is always
        // the fixed offset right after the 0x24-byte header — known up front,
        // letting the start function's `setiosys` embed it as a constant.
        const FILT_ADDR: u32 = 0x24;

        // Filter function (rock = FILT_ADDR): records each received code
        // point into a growing array at LOG_BASE, indexed by a counter at
        // COUNTER_ADDR.
        let mut filt_body = asm::ins(0x4C, &[C32(LOG_BASE), Mem32(COUNTER_ADDR), Local32(0)]); // astore
        filt_body.extend(asm::ins(0x10, &[Mem32(COUNTER_ADDR), C8(1), Mem32(COUNTER_ADDR)])); // counter += 1
        filt_body.extend(asm::ins(0x31, &[C8(0)])); // return 0
        let filt = asm::func(0xC1, &[(4, 1)], &filt_body);

        let mut body = asm::ins(0x149, &[C8(1), C32(FILT_ADDR)]); // setiosys filter, rock=filt
        body.extend(asm::ins(0x70, &[C8(b'H' as i8)])); // streamchar 'H'
        body.extend(asm::ins(0x70, &[C8(b'i' as i8)])); // streamchar 'i'
        body.extend(asm::ins(0x71, &[C32(42)])); // streamnum 42 → chars '4','2'
        body.extend(asm::ins(0x120, &[])); // quit
        let start = asm::func(0xC1, &[], &body);

        let built = asm::assemble(&[filt, start], 1, 0x200);
        assert_eq!(built.addrs[0], FILT_ADDR, "test assumes filt is assembled first");
        let mut m = machine(built);
        m.run();

        // One filter call per output character: 'H', 'i', '4', '2'.
        assert_eq!(m.mem.read32(COUNTER_ADDR), Some(4));
        let codes: Vec<u32> = (0..4).map(|i| m.mem.read32(LOG_BASE + i * 4).unwrap()).collect();
        assert_eq!(codes, vec!['H' as u32, 'i' as u32, '4' as u32, '2' as u32]);
        assert_eq!(out_str(&m), ""); // filter mode does not print to Glk
    }

    #[test]
    fn filter_iosys_recursion_is_depth_guarded() {
        use asm::Op::{C8, C32, Mem32};
        const COUNTER_ADDR: u32 = 0x100;
        const FILT_ADDR: u32 = 0x24;

        // A filter function whose own output (streamchar) recurses back into
        // `emit`/the filter. Left unguarded, this would overflow the native
        // stack; the depth guard must bound it instead.
        let mut filt_body = asm::ins(0x10, &[Mem32(COUNTER_ADDR), C8(1), Mem32(COUNTER_ADDR)]); // counter += 1
        filt_body.extend(asm::ins(0x70, &[C8(b'X' as i8)])); // streamchar 'X' → recurses
        filt_body.extend(asm::ins(0x31, &[C8(0)])); // return 0
        let filt = asm::func(0xC1, &[], &filt_body);

        let mut body = asm::ins(0x149, &[C8(1), C32(FILT_ADDR)]); // setiosys filter
        body.extend(asm::ins(0x70, &[C8(b'X' as i8)])); // kick off the recursion
        body.extend(asm::ins(0x120, &[])); // quit
        let start = asm::func(0xC1, &[], &body);

        let built = asm::assemble(&[filt, start], 1, 0x200);
        assert_eq!(built.addrs[0], FILT_ADDR, "test assumes filt is assembled first");
        let mut m = machine(built);
        // Grow the VM's own byte stack well past what 256+ nested call frames
        // need, so the depth guard (not a VM stack overflow) is what bounds
        // the recursion here.
        m.stack.resize(1 << 20, 0);
        m.run(); // must terminate rather than stack-overflow or hang

        // Bounded by the depth guard: the initial call plus 32 further
        // recursive calls before deeper output is discarded.
        assert_eq!(m.mem.read32(COUNTER_ADDR), Some(33));
    }

    #[test]
    fn filter_switches_iosys_mid_string_routes_remainder() {
        use asm::Op::{C8, C16, C32, Local32, Mem16, Zero};
        const FILT_ADDR: u32 = 0x24;
        const MEMBUF: u32 = 0x180;
        const MEMBUF_LEN: i8 = 16;
        const SID_ADDR: u16 = 0x1A0;
        const CLOSE_ADDR: u16 = 0x1A8;

        // Filter function `surroundonce`(ch): switches iosys to Glk (mode 2,
        // rock 0) then emits '<', ch, '>'. Only the first char ('1' of "123")
        // is meant to reach the filter; '<', ch, and '>' go out under the
        // switched-to mode, and the remaining '2'/'3' must then also be
        // routed under that new mode rather than dropped or fed to the
        // now-stale filter rock.
        let mut filt_body = asm::ins(0x149, &[C8(2), C8(0)]); // setiosys glk, rock 0
        filt_body.extend(asm::ins(0x70, &[C8(b'<' as i8)])); // streamchar '<'
        filt_body.extend(asm::ins(0x70, &[Local32(0)])); // streamchar ch
        filt_body.extend(asm::ins(0x70, &[C8(b'>' as i8)])); // streamchar '>'
        filt_body.extend(asm::ins(0x31, &[C8(0)])); // return 0
        let filt = asm::func(0xC1, &[(4, 1)], &filt_body);

        // Open a memory stream and make it current, so glk-mode output (mode
        // 2, which the filter switches to) lands there rather than a window.
        let mut body = glk_call(0x43, &[C16(MEMBUF as i16), C8(MEMBUF_LEN), C8(1), C8(0)], Mem16(SID_ADDR));
        body.extend(glk_call(0x47, &[Mem16(SID_ADDR)], Zero)); // set_current(memory stream)
        body.extend(asm::ins(0x149, &[C8(1), C32(FILT_ADDR)])); // setiosys filter, rock=filt
        body.extend(asm::ins(0x71, &[C32(123)])); // streamnum 123 -> '1','2','3'
        body.extend(glk_call(0x44, &[Mem16(SID_ADDR), C16(CLOSE_ADDR as i16)], Zero)); // stream_close -> counts
        body.extend(asm::ins(0x120, &[])); // quit
        let start = asm::func(0xC1, &[], &body);

        let built = asm::assemble(&[filt, start], 1, 0x300);
        assert_eq!(built.addrs[0], FILT_ADDR, "test assumes filt is assembled first");
        let mut m = machine(built);
        m.run();

        let bytes: Vec<u8> = (0..5).map(|i| m.mem.read8(MEMBUF + i).unwrap() as u8).collect();
        assert_eq!(bytes, b"<1>23", "remainder routed under the switched-to mode, not dropped");
        assert_eq!(m.mem.read32((CLOSE_ADDR + 4) as u32), Some(5), "write count");
    }

    #[test]
    fn glk_put_char_and_put_buffer() {
        // glk_put_char('B').
        let mut body = asm::ins(0x40, &[asm::Op::C8(66), asm::Op::Stack]); // push 'B'
        body.extend(asm::ins(0x130, &[asm::Op::C16(0x80), asm::Op::C8(1), asm::Op::Zero]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(out_str(&m), "B");

        // glk_put_buffer(addr=0x100, len=2) over "Hi" planted via copyb.
        let mut body = asm::ins(0x42, &[asm::Op::C8(b'H' as i8), asm::Op::Mem16(0x0100)]);
        body.extend(asm::ins(0x42, &[asm::Op::C8(b'i' as i8), asm::Op::Mem16(0x0101)]));
        body.extend(asm::ins(0x40, &[asm::Op::C8(2), asm::Op::Stack])); // push len (below)
        body.extend(asm::ins(0x40, &[asm::Op::C16(0x0100), asm::Op::Stack])); // push addr (top)
        body.extend(asm::ins(0x130, &[asm::Op::C16(0x84), asm::Op::C8(2), asm::Op::Zero]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(out_str(&m), "Hi");
    }

    #[test]
    fn streamstr_prints_e0_cstring() {
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0100)])); // streamstr @0x100
        body.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        for (i, b) in [0xE0u8, b'H', b'i', b'!', 0].iter().enumerate() {
            m.mem.write8(0x100 + i as u32, *b as u32).unwrap();
        }
        m.run();
        assert_eq!(out_str(&m), "Hi!");
    }

    #[test]
    fn nop_and_quit() {
        let mut body = asm::ins(0x00, &[]); // nop
        body.extend(asm::ins(0x120, &[])); // quit
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        assert_eq!(m.step(), StepResult::Continue); // nop
        assert_eq!(m.step(), StepResult::Quit); // quit
        assert_eq!(m.step(), StepResult::Quit); // stays quit
    }

    #[test]
    fn unknown_opcode_records_diagnostic_and_quits() {
        let m = run_program(asm::ins(0x1FF, &[])); // 0x1FF unimplemented
        assert!(m.halted);
        assert!(!m.diagnostics.is_empty());
    }

    #[test]
    fn memory_fault_records_diagnostic_and_quits() {
        // Load contents of a wildly out-of-range address → fault, no panic.
        let m = run_program(asm::ins(0x40, &[asm::Op::Mem32(0xFFFF_FFF0), asm::Op::Zero]));
        assert!(m.halted);
        assert!(m.diagnostics.iter().any(|d| d.contains("memory fault")));
    }

    #[test]
    fn end_to_end_arith_call_output_branch_quit() {
        // double(arg) = arg + arg (function at 0x24).
        let mut dbody = asm::ins(0x10, &[asm::Op::Local8(0), asm::Op::Local8(0), asm::Op::Stack]);
        dbody.extend(asm::ins(0x31, &[asm::Op::Stack])); // return
        let double = asm::func(0xC1, &[(4, 1)], &dbody);

        // start: setiosys glk; push 3+4; call double; streamnum result; a taken
        // branch (jz 0, offset 1) returns from start (halts) before the poison.
        let mut sbody = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        sbody.extend(asm::ins(0x10, &[asm::Op::C8(3), asm::Op::C8(4), asm::Op::Stack])); // 7
        sbody.extend(asm::ins(0x30, &[asm::Op::C32(0x24), asm::Op::C8(1), asm::Op::Stack])); // 14
        sbody.extend(asm::ins(0x71, &[asm::Op::Stack])); // streamnum 14 → "14"
        sbody.extend(asm::ins(0x22, &[asm::Op::C8(0), asm::Op::C8(1)])); // jz 0,1 → return 1
        sbody.extend(asm::ins(0x70, &[asm::Op::C8(b'!' as i8)])); // poison, unreached
        let start = asm::func(0xC1, &[], &sbody);

        let built = asm::assemble(&[double, start], 1, 0x100);
        assert_eq!(built.addrs[0], 0x24);
        let mut m = machine(built);
        m.run();
        assert_eq!(out_str(&m), "14");
        assert!(m.halted);
    }

    #[test]
    fn start_frame_entered_at_fp_zero() {
        // A C1 start function with three 4-byte locals.
        let start = asm::func(0xC1, &[(4, 3)], &[]);
        let built = asm::assemble(&[start], 0, 0x100);
        let m = machine(built);
        assert!(!m.halted);
        assert_eq!(m.fp, 0);
        // Locals are zero-initialized.
        assert_eq!(m.local_load(0).unwrap(), 0);
        assert_eq!(m.local_load(8).unwrap(), 0);
    }

    #[test]
    fn c1_call_copies_args_into_locals_extras_zeroed() {
        // start (C1, no locals) and a callee (C1, four 4-byte locals).
        let start = asm::func(0xC1, &[], &[]);
        let callee = asm::func(0xC1, &[(4, 4)], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        m.call_function(callee_addr, &[11, 22, 33], Dest::Discard).unwrap();
        assert_eq!(m.local_load(0).unwrap(), 11);
        assert_eq!(m.local_load(4).unwrap(), 22);
        assert_eq!(m.local_load(8).unwrap(), 33);
        assert_eq!(m.local_load(12).unwrap(), 0); // extra local zeroed
    }

    #[test]
    fn c1_call_truncates_to_local_width() {
        let start = asm::func(0xC1, &[], &[]);
        // locals: one 1-byte, one 2-byte, one 4-byte.
        let callee = asm::func(0xC1, &[(1, 1), (2, 1), (4, 1)], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        m.call_function(callee_addr, &[0x1234_5678, 0xABCD, 0xDEAD_BEEF], Dest::Discard)
            .unwrap();
        assert_eq!(m.local_load(0).unwrap(), 0x78); // 1-byte truncation
        assert_eq!(m.local_load(2).unwrap(), 0xABCD); // 2-byte, aligned to even
        assert_eq!(m.local_load(4).unwrap(), 0xDEAD_BEEF); // 4-byte, aligned to 4
    }

    #[test]
    fn c0_call_pushes_args_and_count() {
        let start = asm::func(0xC1, &[], &[]);
        let callee = asm::func(0xC0, &[], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        m.call_function(callee_addr, &[100, 200, 300], Dest::Discard).unwrap();
        // Count on top, then first arg, then second, then third.
        assert_eq!(m.value_count(), 4);
        assert_eq!(m.pop32().unwrap(), 3); // count
        assert_eq!(m.pop32().unwrap(), 100); // first arg topmost
        assert_eq!(m.pop32().unwrap(), 200);
        assert_eq!(m.pop32().unwrap(), 300);
    }

    #[test]
    fn return_writes_value_to_local_dest() {
        let start = asm::func(0xC1, &[(4, 1)], &[]); // start has one local
        let callee = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        // Call storing the return value into start's local at offset 0.
        m.call_function(callee_addr, &[], Dest::Local(0)).unwrap();
        m.return_value(0x4242).unwrap();
        assert_eq!(m.fp, 0); // back in the start frame
        assert_eq!(m.local_load(0).unwrap(), 0x4242);
    }

    #[test]
    fn return_writes_value_to_stack_dest() {
        let start = asm::func(0xC1, &[], &[]);
        let callee = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let mut m = machine(built);
        m.call_function(callee_addr, &[], Dest::Push).unwrap();
        m.return_value(0x99).unwrap();
        assert_eq!(m.value_count(), 1);
        assert_eq!(m.pop32().unwrap(), 0x99);
    }

    #[test]
    fn return_writes_value_to_memory_dest() {
        let start = asm::func(0xC1, &[], &[]);
        let callee = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start, callee], 0, 0x100);
        let callee_addr = built.addrs[1];
        let ramstart = {
            let mem = Memory::new(built.image.clone()).unwrap();
            mem.ramstart()
        };
        let mut m = machine(built);
        m.call_function(callee_addr, &[], Dest::Mem(ramstart)).unwrap();
        m.return_value(0xCAFE_F00D).unwrap();
        assert_eq!(m.mem.read32(ramstart), Some(0xCAFE_F00D));
    }

    #[test]
    fn returning_from_outermost_halts() {
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        assert!(!m.halted);
        m.return_value(0).unwrap();
        assert!(m.halted);
    }

    #[test]
    fn nested_calls_restore_pc_and_fp() {
        let start = asm::func(0xC1, &[], &[]);
        let f1 = asm::func(0xC1, &[(4, 1)], &[]);
        let f2 = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start, f1, f2], 0, 0x100);
        let (a1, a2) = (built.addrs[1], built.addrs[2]);
        let mut m = machine(built);
        let start_pc = m.pc;
        m.pc = 0xDEAD; // pretend the caller is mid-instruction
        m.call_function(a1, &[], Dest::Discard).unwrap();
        let f1_fp = m.fp;
        m.pc = 0xBEEF;
        m.call_function(a2, &[], Dest::Discard).unwrap();
        m.return_value(0).unwrap();
        assert_eq!(m.fp, f1_fp); // back to f1
        assert_eq!(m.pc, 0xBEEF); // f1's resume pc
        m.return_value(0).unwrap();
        assert_eq!(m.fp, 0); // back to start
        assert_eq!(m.pc, 0xDEAD);
        let _ = start_pc;
    }

    // ── Task 1 (2b): compressed-string decoding + full streamstr ──────────────

    /// Write consecutive bytes into memory starting at `addr` (RAM).
    fn poke(m: &mut Machine, addr: u32, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            m.mem.write8(addr + i as u32, b as u32).unwrap();
        }
    }

    /// Run a start-function body that has already had its RAM populated by
    /// `setup`; returns the machine. iosys is left to the body.
    fn run_with_ram(body: Vec<u8>, ram_bytes: u32, setup: impl FnOnce(&mut Machine)) -> Machine {
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, ram_bytes);
        let mut m = machine(built);
        setup(&mut m);
        m.run();
        m
    }

    /// A standard tiny decoding table at RAM base `t` that decodes the bit
    /// sequence 0,0,0,1,1 (packed byte 0x18) to "Hi": root branches
    /// left→inner / right→terminator; inner branches left→'H' / right→'i'.
    fn build_hi_table(m: &mut Machine, t: u32) {
        // Header: length, numnodes, root.
        let root = t + 12;
        let inner = root + 9;
        let h = inner + 9;
        let i = h + 2;
        let term = i + 2;
        let len = (term + 1) - t;
        poke(m, t, &len.to_be_bytes());
        poke(m, t + 4, &5u32.to_be_bytes());
        poke(m, t + 8, &root.to_be_bytes());
        // root: branch left=inner right=term
        poke(m, root, &[0x00]);
        poke(m, root + 1, &inner.to_be_bytes());
        poke(m, root + 5, &term.to_be_bytes());
        // inner: branch left='H' right='i'
        poke(m, inner, &[0x00]);
        poke(m, inner + 1, &h.to_be_bytes());
        poke(m, inner + 5, &i.to_be_bytes());
        poke(m, h, &[0x02, b'H']);
        poke(m, i, &[0x02, b'i']);
        poke(m, term, &[0x01]);
    }

    #[test]
    fn compressed_string_decodes_against_table() {
        // setstringtbl 0x100; streamstr 0x140; quit.
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]); // setiosys glk
        body.extend(asm::ins(0x141, &[asm::Op::C16(0x0100)])); // setstringtbl 0x100
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0140)])); // streamstr 0x140
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            build_hi_table(m, 0x100);
            poke(m, 0x140, &[0xE1, 0x18]); // compressed "Hi": bits 0,0,0,1,1
        });
        assert_eq!(out_str(&m), "Hi");
        assert!(m.diagnostics.is_empty(), "diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn getstringtbl_reports_current_table() {
        // setstringtbl 0x100; getstringtbl → RAM 0x160.
        let mut body = asm::ins(0x141, &[asm::Op::C16(0x0100)]);
        body.extend(asm::ins(0x140, &[asm::Op::Mem16(0x0160)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x160).unwrap(), 0x100);
    }

    #[test]
    fn compressed_string_indirect_node_prints_e0() {
        // root: left → 0x08 node (→ E0 "Ok"), right → terminator. bits 0,1 → 0x02.
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        body.extend(asm::ins(0x141, &[asm::Op::C16(0x0100)]));
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0150)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            let t = 0x100u32;
            let root = t + 12;
            let indir = root + 9;
            let term = indir + 5;
            let estr = 0x170u32;
            let len = (term + 1) - t;
            poke(m, t, &len.to_be_bytes());
            poke(m, t + 4, &3u32.to_be_bytes());
            poke(m, t + 8, &root.to_be_bytes());
            poke(m, root, &[0x00]);
            poke(m, root + 1, &indir.to_be_bytes());
            poke(m, root + 5, &term.to_be_bytes());
            poke(m, indir, &[0x08]);
            poke(m, indir + 1, &estr.to_be_bytes());
            poke(m, term, &[0x01]);
            poke(m, estr, &[0xE0, b'O', b'k', 0]);
            poke(m, 0x150, &[0xE1, 0x02]); // bits 0,1
        });
        assert_eq!(out_str(&m), "Ok");
    }

    #[test]
    fn compressed_string_function_node_calls_function() {
        // A printer function (C1, no locals) that streams 'Z' and returns.
        let printer = asm::func(
            0xC1,
            &[],
            &{
                let mut b = asm::ins(0x70, &[asm::Op::C8(b'Z' as i8)]); // streamchar 'Z'
                b.extend(asm::ins(0x31, &[asm::Op::Zero])); // return 0
                b
            },
        );
        // start: setiosys glk; setstringtbl 0x100; streamstr 0x150; quit.
        let mut sbody = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        sbody.extend(asm::ins(0x141, &[asm::Op::C16(0x0100)]));
        sbody.extend(asm::ins(0x72, &[asm::Op::C16(0x0150)]));
        sbody.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &sbody);
        let built = asm::assemble(&[printer, start], 1, 0x200);
        let printer_addr = built.addrs[0];
        let mut m = machine(built);
        // root: left → 0x08 node (→ printer fn), right → terminator. bits 0,1.
        let t = 0x100u32;
        let root = t + 12;
        let fnode = root + 9;
        let term = fnode + 5;
        let len = (term + 1) - t;
        poke(&mut m, t, &len.to_be_bytes());
        poke(&mut m, t + 4, &3u32.to_be_bytes());
        poke(&mut m, t + 8, &root.to_be_bytes());
        poke(&mut m, root, &[0x00]);
        poke(&mut m, root + 1, &fnode.to_be_bytes());
        poke(&mut m, root + 5, &term.to_be_bytes());
        poke(&mut m, fnode, &[0x08]);
        poke(&mut m, fnode + 1, &printer_addr.to_be_bytes());
        poke(&mut m, term, &[0x01]);
        poke(&mut m, 0x150, &[0xE1, 0x02]); // bits 0,1 → call printer, then terminate
        m.run();
        assert_eq!(out_str(&m), "Z");
        assert!(m.diagnostics.is_empty(), "diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn streamstr_e2_unicode_still_works() {
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            // E2 + 3 pad bytes + 'A'(0x41) + 'λ'(0x3BB) + terminator word.
            poke(m, 0x100, &[0xE2, 0, 0, 0]);
            poke(m, 0x104, &0x41u32.to_be_bytes());
            poke(m, 0x108, &0x3BBu32.to_be_bytes());
            poke(m, 0x10C, &0u32.to_be_bytes());
        });
        assert_eq!(out_str(&m), "Aλ");
    }

    #[test]
    fn compressed_string_with_no_table_faults() {
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]);
        body.extend(asm::ins(0x72, &[asm::Op::C16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x100, &[0xE1, 0x00]));
        assert!(m.halted);
        assert!(m.diagnostics.iter().any(|d| d.contains("decoding table")));
    }

    // ── Task 2 (2b): memory-array opcodes + mzero/mcopy ───────────────────────

    #[test]
    fn aload_astore_32bit_roundtrip_signed_index() {
        use asm::Op::{C16, C32, C8, Mem16};
        // astore base=0x140 index=2 → 0x148; aload reads it back to 0x100.
        let mut body = asm::ins(0x4C, &[C16(0x0140), C8(2), C32(0xDEAD_BEEF)]);
        body.extend(asm::ins(0x48, &[C16(0x0140), C8(2), Mem16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x148).unwrap(), 0xDEAD_BEEF);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0xDEAD_BEEF);

        // Negative index: astore base=0x148 index=-1 → 0x144.
        let mut body = asm::ins(0x4C, &[C16(0x0148), C8(-1), C32(0x1234_5678)]);
        body.extend(asm::ins(0x48, &[C16(0x0148), C8(-1), Mem16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x144).unwrap(), 0x1234_5678);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x1234_5678);
    }

    #[test]
    fn aloads_astores_16bit_and_aloadb_astoreb_8bit() {
        use asm::Op::{C16, C8, Mem16};
        // 16-bit: astores base=0x140 index=3 → 0x146; loads expand WITHOUT sign.
        let mut body = asm::ins(0x4D, &[C16(0x0140), C8(3), C16(-2)]); // 0xFFFE truncated
        body.extend(asm::ins(0x49, &[C16(0x0140), C8(3), Mem16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read16(0x146).unwrap(), 0xFFFE);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0000_FFFE); // zero-extended

        // 8-bit at a negative index.
        let mut body = asm::ins(0x4E, &[C16(0x0150), C8(-1), C16(0xAB)]); // → 0x14F
        body.extend(asm::ins(0x4A, &[C16(0x0150), C8(-1), Mem16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read8(0x14F).unwrap(), 0xAB);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0000_00AB);
    }

    #[test]
    fn aloadbit_astorebit_signed_bit_index() {
        use asm::Op::{C16, C8, Mem16};
        // Set bit 0 of 0x142, bit 7 of 0x142, bit 0 of 0x143 (index 8), bit 7 of
        // 0x141 (index -1) — mirrors the spec's astorebit examples.
        let sets: &[(u16, i8)] = &[(0x142, 0), (0x142, 7), (0x142, 8), (0x142, -1)];
        let mut body = Vec::new();
        for &(base, idx) in sets {
            body.extend(asm::ins(0x4F, &[C16(base as i16), C8(idx), C8(1)]));
        }
        // Read them back: bit 7 of 0x141 should be 1; bit 0 of 0x142 should be 1.
        body.extend(asm::ins(0x4B, &[C16(0x0142), C8(0), Mem16(0x0100)]));
        body.extend(asm::ins(0x4B, &[C16(0x0142), C8(-1), Mem16(0x0104)])); // bit7 of 0x141
        body.extend(asm::ins(0x4B, &[C16(0x0142), C8(8), Mem16(0x0108)])); // bit0 of 0x143
        body.extend(asm::ins(0x4B, &[C16(0x0142), C8(1), Mem16(0x010C)])); // clear bit → 0
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read8(0x142).unwrap(), 0b1000_0001); // bits 0 and 7 set
        assert_eq!(m.mem.read8(0x143).unwrap(), 0b0000_0001); // bit 0 set
        assert_eq!(m.mem.read8(0x141).unwrap(), 0b1000_0000); // bit 7 set
        assert_eq!(m.mem.read32(0x100).unwrap(), 1);
        assert_eq!(m.mem.read32(0x104).unwrap(), 1);
        assert_eq!(m.mem.read32(0x108).unwrap(), 1);
        assert_eq!(m.mem.read32(0x10C).unwrap(), 0);
    }

    #[test]
    fn aload_out_of_range_faults() {
        use asm::Op::{C32, Zero};
        // base near the top of memory + a big index → out of range.
        let m = run_program(asm::ins(0x48, &[C32(0xFFFF_FFF0), Zero, Zero]));
        assert!(m.halted);
        assert!(m.diagnostics.iter().any(|d| d.contains("memory fault")));
    }

    #[test]
    fn mzero_clears_a_span() {
        use asm::Op::{C16, C32, C8};
        // Plant a value, then mzero 4 bytes over it.
        let mut body = asm::ins(0x4C, &[C16(0x0140), C8(0), C32(0xFFFF_FFFF)]);
        body.extend(asm::ins(0x170, &[asm::Op::C8(4), C16(0x0140)])); // mzero count=4 addr=0x140
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x140).unwrap(), 0);
    }

    #[test]
    fn mcopy_handles_forward_and_backward_overlap() {
        use asm::Op::{C16, C8};
        // Source bytes 1,2,3,4 at 0x140. mcopy 4 from 0x140 to 0x142 (to > from →
        // backward copy) must yield 1,2,1,2,3,4 across 0x140..0x146.
        let plant = |body: &mut Vec<u8>| {
            for (i, v) in [1u8, 2, 3, 4].iter().enumerate() {
                body.extend(asm::ins(0x4E, &[C16(0x0140), C8(i as i8), C8(*v as i8)]));
            }
        };
        let mut body = Vec::new();
        plant(&mut body);
        body.extend(asm::ins(0x171, &[C8(4), C16(0x0140), C16(0x0142)])); // count,from,to
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(
            [
                m.mem.read8(0x140).unwrap(),
                m.mem.read8(0x141).unwrap(),
                m.mem.read8(0x142).unwrap(),
                m.mem.read8(0x143).unwrap(),
                m.mem.read8(0x144).unwrap(),
                m.mem.read8(0x145).unwrap(),
            ],
            [1, 2, 1, 2, 3, 4]
        );

        // to < from → forward copy. 1,2,3,4 at 0x142; copy to 0x140 → 1,2,3,4.
        let mut body = Vec::new();
        for (i, v) in [1u8, 2, 3, 4].iter().enumerate() {
            body.extend(asm::ins(0x4E, &[C16(0x0142), C8(i as i8), C8(*v as i8)]));
        }
        body.extend(asm::ins(0x171, &[C8(4), C16(0x0142), C16(0x0140)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(
            [
                m.mem.read8(0x140).unwrap(),
                m.mem.read8(0x141).unwrap(),
                m.mem.read8(0x142).unwrap(),
                m.mem.read8(0x143).unwrap(),
            ],
            [1, 2, 3, 4]
        );
    }

    // ── Task 3 (2b): malloc / mfree heap ──────────────────────────────────────

    #[test]
    fn malloc_returns_distinct_in_range_blocks() {
        use asm::Op::{C8, Mem16};
        let mut body = asm::ins(0x178, &[C8(16), Mem16(0x0100)]); // malloc 16 → 0x100
        body.extend(asm::ins(0x178, &[C8(16), Mem16(0x0104)])); // malloc 16 → 0x104
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        let a = m.mem.read32(0x100).unwrap();
        let b = m.mem.read32(0x104).unwrap();
        let floor = m.mem.endmem();
        assert!(a >= floor, "block a {a:#x} below endmem {floor:#x}");
        assert_ne!(a, b);
        assert_eq!(b, a + 16, "blocks must be adjacent and non-overlapping");
        assert!(a + 16 <= m.mem.mem_size() && b + 16 <= m.mem.mem_size());
    }

    #[test]
    fn mfree_then_malloc_reuses_freed_space() {
        use asm::Op::{C8, Mem16};
        // malloc a, malloc b, free a, malloc c → c reuses a's address.
        let mut body = asm::ins(0x178, &[C8(16), Mem16(0x0100)]); // a
        body.extend(asm::ins(0x178, &[C8(16), Mem16(0x0104)])); // b
        body.extend(asm::ins(0x179, &[Mem16(0x0100)])); // mfree a (contents of 0x100)
        body.extend(asm::ins(0x178, &[C8(16), Mem16(0x0108)])); // c
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        let a = m.mem.read32(0x100).unwrap();
        let c = m.mem.read32(0x108).unwrap();
        assert_eq!(c, a, "freed block should be reused");
    }

    #[test]
    fn setmemsize_while_heap_active_fails() {
        use asm::Op::{C32, C8, Mem16, Zero};
        let mut body = asm::ins(0x178, &[C8(16), Zero]); // malloc 16 (activate heap)
        body.extend(asm::ins(0x103, &[C32(0x1_0000), Mem16(0x0100)])); // setmemsize → result
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "setmemsize must fail");
        assert!(m.diagnostics.iter().any(|d| d.contains("heap")));
    }

    #[test]
    fn malloc_too_large_returns_zero() {
        use asm::Op::{C32, Mem16};
        // A request beyond the heap ceiling fails without allocating.
        let mut body = asm::ins(0x178, &[C32(0x2000_0000), Mem16(0x0100)]);
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0);
    }

    #[test]
    fn malloc_growth_keeps_memsize_256_aligned() {
        use asm::Op::{C8, Mem16};
        // ENDMEM (memsize) is a multiple of 256 (GLULX_NOTES §1). Heap growth
        // must preserve that invariant, which also leaves slack above the block.
        // Inform's memory-stream idiom relies on this: it opens a memory stream
        // over a malloc'd buffer, then closes it with resultptr = buf + len,
        // writing the 8-byte stream_result struct one word past the buffer end.
        let mut body = asm::ins(0x178, &[C8(24), Mem16(0x0100)]); // malloc 24 → block @ 0x100
        body.extend(asm::ins(0x120, &[]));
        let mut m = run_program(body);
        let block = m.mem.read32(0x100).unwrap();
        assert_eq!(m.mem.mem_size() % 256, 0, "heap growth must keep ENDMEM 256-aligned");
        assert!(
            m.mem.write32(block + 24, 0xDEAD_BEEF).is_ok(),
            "a write just past the block end must be in-range (alignment slack)"
        );
    }

    #[test]
    fn freeing_last_block_deactivates_and_shrinks() {
        use asm::Op::{C8, Mem16};
        let mut body = asm::ins(0x178, &[C8(32), Mem16(0x0100)]); // malloc → activates
        body.extend(asm::ins(0x179, &[Mem16(0x0100)])); // free the only block
        body.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        let floor = m.mem.mem_size();
        m.run();
        assert_eq!(m.heap_start, 0, "heap should be inactive again");
        assert_eq!(m.mem.mem_size(), floor, "memory shrinks back to heap start");
    }

    // ── Task 4 (2b): search opcodes ───────────────────────────────────────────

    /// Run `linearsearch` with the given operands; return the stored result.
    fn linear(
        key: asm::Op,
        keysize: i8,
        start: u16,
        structsize: i8,
        numstructs: asm::Op,
        keyoffset: i8,
        options: i8,
        setup: impl FnOnce(&mut Machine),
    ) -> u32 {
        use asm::Op::{C16, C8, Mem16};
        let mut body = asm::ins(
            0x150,
            &[
                key,
                C8(keysize),
                C16(start as i16),
                C8(structsize),
                numstructs,
                C8(keyoffset),
                C8(options),
                Mem16(0x0100),
            ],
        );
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x300, setup);
        m.mem.read32(0x100).unwrap()
    }

    /// Three 8-byte structs at 0x140 with 4-byte keys at offset 0.
    fn three_structs(m: &mut Machine) {
        poke(m, 0x140, &0x1111u32.to_be_bytes());
        poke(m, 0x148, &0x2222u32.to_be_bytes());
        poke(m, 0x150, &0x3333u32.to_be_bytes());
    }

    #[test]
    fn linearsearch_finds_and_reports_absence() {
        use asm::Op::{C32, C8};
        // Present key → struct address.
        assert_eq!(
            linear(C32(0x2222), 4, 0x140, 8, C8(3), 0, 0, three_structs),
            0x148
        );
        // Absent key → 0.
        assert_eq!(linear(C32(0x9999), 4, 0x140, 8, C8(3), 0, 0, three_structs), 0);
        // ReturnIndex (0x04): present → index 2.
        assert_eq!(
            linear(C32(0x3333), 4, 0x140, 8, C8(3), 0, 0x04, three_structs),
            2
        );
        // ReturnIndex + absent → 0xFFFFFFFF.
        assert_eq!(
            linear(C32(0x9999), 4, 0x140, 8, C8(3), 0, 0x04, three_structs),
            0xFFFF_FFFF
        );
    }

    #[test]
    fn linearsearch_key_indirect_and_zero_terminates() {
        use asm::Op::{C32, C8};
        // KeyIndirect (0x01): key bytes live at 0x110.
        let r = linear(C32(0x0110), 4, 0x140, 8, C8(3), 0, 0x01, |m| {
            three_structs(m);
            poke(m, 0x110, &0x2222u32.to_be_bytes());
        });
        assert_eq!(r, 0x148);

        // ZeroKeyTerminates (0x02): a zero key at index 1 halts before 0x3333.
        // NumStructs -1 (no limit) so only the zero key stops it.
        let r = linear(C32(0x3333), 4, 0x140, 8, C8(-1), 0, 0x02, |m| {
            poke(m, 0x140, &0x1111u32.to_be_bytes());
            poke(m, 0x148, &0u32.to_be_bytes()); // zero key terminates here
            poke(m, 0x150, &0x3333u32.to_be_bytes());
        });
        assert_eq!(r, 0, "search should fail at the zero key");
    }

    #[test]
    fn binarysearch_on_sorted_table() {
        use asm::Op::{C16, C32, C8, Mem16};
        // Sorted keys 0x10,0x20,0x30,0x40 in 8-byte structs at 0x140.
        let setup = |m: &mut Machine| {
            for (i, k) in [0x10u32, 0x20, 0x30, 0x40].iter().enumerate() {
                poke(m, 0x140 + 8 * i as u32, &k.to_be_bytes());
            }
        };
        let run = |key: u32, options: i8| -> u32 {
            let mut body = asm::ins(
                0x151,
                &[
                    C32(key),
                    C8(4),
                    C16(0x0140),
                    C8(8),
                    C8(4),
                    C8(0),
                    C8(options),
                    Mem16(0x0100),
                ],
            );
            body.extend(asm::ins(0x120, &[]));
            run_with_ram(body, 0x300, setup).mem.read32(0x100).unwrap()
        };
        assert_eq!(run(0x30, 0), 0x150); // address of the 3rd struct
        assert_eq!(run(0x30, 0x04), 2); // ReturnIndex
        assert_eq!(run(0x25, 0), 0); // absent
        assert_eq!(run(0x10, 0x04), 0); // first element, index 0
        assert_eq!(run(0x40, 0x04), 3); // last element
        assert_eq!(run(0x50, 0x04), 0xFFFF_FFFF); // absent past end
    }

    #[test]
    fn linkedsearch_over_a_linked_list() {
        use asm::Op::{C16, C32, C8, Mem16};
        // Nodes: key at offset 0, next pointer at offset 4.
        let setup = |m: &mut Machine| {
            poke(m, 0x140, &0xAAAAu32.to_be_bytes());
            poke(m, 0x144, &0x0150u32.to_be_bytes());
            poke(m, 0x150, &0xBBBBu32.to_be_bytes());
            poke(m, 0x154, &0x0160u32.to_be_bytes());
            poke(m, 0x160, &0xCCCCu32.to_be_bytes());
            poke(m, 0x164, &0u32.to_be_bytes()); // end of list
        };
        let run = |key: u32| -> u32 {
            let mut body = asm::ins(
                0x152,
                &[C32(key), C8(4), C16(0x0140), C8(0), C8(4), C8(0), Mem16(0x0100)],
            );
            body.extend(asm::ins(0x120, &[]));
            run_with_ram(body, 0x300, setup).mem.read32(0x100).unwrap()
        };
        assert_eq!(run(0xBBBB), 0x150); // middle node
        assert_eq!(run(0xAAAA), 0x140); // head
        assert_eq!(run(0xCCCC), 0x160); // tail
        assert_eq!(run(0x9999), 0); // absent → 0
    }

    // ── Task 5 (2b): gestalt + verify ─────────────────────────────────────────

    #[test]
    fn gestalt_reports_capabilities() {
        let m = machine_with_body(&[], vec![]);
        assert_eq!(m.gestalt(0, 0), 0x0003_0103); // GlulxVersion 3.1.3
        assert_eq!(m.gestalt(1, 0), 0x0000_0100); // TerpVersion 0.1.0
        assert_eq!(m.gestalt(2, 0), 1); // ResizeMem
        assert_eq!(m.gestalt(3, 0), 1); // Undo (saveundo/restoreundo)
        assert_eq!(m.gestalt(4, 0), 1); // IOSystem null
        assert_eq!(m.gestalt(4, 1), 1); // IOSystem filter
        assert_eq!(m.gestalt(4, 2), 1); // IOSystem Glk
        assert_eq!(m.gestalt(4, 3), 0); // IOSystem unrecognized
        assert_eq!(m.gestalt(5, 0), 1); // Unicode
        assert_eq!(m.gestalt(6, 0), 1); // MemCopy
        assert_eq!(m.gestalt(7, 0), 1); // MAlloc
        assert_eq!(m.gestalt(8, 0), 0); // MAllocHeap inactive → 0
        assert_eq!(m.gestalt(9, 0), 1); // Acceleration: interception implemented
        assert_eq!(m.gestalt(10, 0), 0); // AccelFunc 0 is "cancel", not a function
        assert_eq!(m.gestalt(11, 0), 1); // Float (single-precision implemented)
        assert_eq!(m.gestalt(999, 0), 0); // unknown selector
    }

    #[test]
    fn gestalt_opcode_and_mallocheap() {
        use asm::Op::{C8, Mem16, Zero};
        // gestalt(0,0) via the opcode → 0x00030103.
        let mut body = asm::ins(0x100, &[Zero, Zero, Mem16(0x0100)]);
        // malloc 16 (activates heap), then gestalt(8) → heap-start address.
        body.extend(asm::ins(0x178, &[C8(16), Mem16(0x0104)]));
        body.extend(asm::ins(0x100, &[C8(8), Zero, Mem16(0x0108)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0003_0103);
        let heap_start = m.mem.read32(0x108).unwrap();
        assert_eq!(heap_start, m.mem.read32(0x104).unwrap()); // == first block addr
        assert_eq!(heap_start, m.heap_start);
    }

    #[test]
    fn verify_returns_success() {
        let mut body = asm::ins(0x121, &[asm::Op::Mem16(0x0100)]);
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0);
    }

    // ── Task 1 (2c): save / restore serialization core ────────────────────────

    #[test]
    fn save_restore_roundtrips_ram_stack_heap_registers() {
        let mut m = machine_with_body(&[], vec![]);
        m.mem.write32(0x100, 0xCAFE_BABE).unwrap();
        m.push32(0x1111_2222).unwrap();
        m.push32(0x3333_4444).unwrap();
        m.iosys_mode = 2;
        m.iosys_rock = 0x99;
        m.cur_stringtbl = 0x1234;
        m.pc = 0x40;
        let blk = m.heap_malloc(16);
        assert_ne!(blk, 0);

        let snap = m.save_state();

        // Diverge from the saved state.
        m.mem.write32(0x100, 0).unwrap();
        m.pop32().unwrap();
        m.iosys_mode = 0;
        m.iosys_rock = 0;
        m.cur_stringtbl = 0;
        m.pc = 0;
        m.heap_free(blk).unwrap();

        m.restore_state(&snap).unwrap();

        assert_eq!(m.mem.read32(0x100).unwrap(), 0xCAFE_BABE);
        assert_eq!(m.iosys_mode, 2);
        assert_eq!(m.iosys_rock, 0x99);
        assert_eq!(m.cur_stringtbl, 0x1234);
        assert_eq!(m.pc, 0x40);
        assert_eq!(m.heap_start, blk);
        assert_eq!(m.heap_blocks, vec![(blk, 16)]);
        assert_eq!(m.value_count(), 2);
        assert_eq!(m.pop32().unwrap(), 0x3333_4444);
        assert_eq!(m.pop32().unwrap(), 0x1111_2222);
    }

    /// `save_quetzal()` is the standard, spec-conformant bare save (Glulx spec
    /// §1.8): `IFhd`/`CMem`/`Stks`/`MAll` only — no `GReg`, no `Glk `. `save_state()`
    /// (the host `.lanthorn` snapshot) still carries both.
    #[test]
    fn save_quetzal_omits_greg_and_glk_chunks() {
        fn has_chunk(save: &[u8], id: &[u8; 4]) -> bool {
            save.windows(4).any(|w| w == id)
        }
        let mut m = machine_with_body(&[], vec![]);
        m.mem.write32(0x100, 0xCAFE_BABE).unwrap();

        let bare = m.save_quetzal();
        assert!(has_chunk(&bare, b"IFhd"), "save_quetzal carries IFhd");
        assert!(has_chunk(&bare, b"CMem"), "save_quetzal carries CMem");
        assert!(has_chunk(&bare, b"Stks"), "save_quetzal carries Stks");
        assert!(has_chunk(&bare, b"MAll"), "save_quetzal carries MAll");
        assert!(!has_chunk(&bare, b"GReg"), "save_quetzal omits GReg");
        assert!(!has_chunk(&bare, b"Glk "), "save_quetzal omits the Glk chunk");

        let full = m.save_state();
        assert!(has_chunk(&full, b"GReg"), "save_state still carries GReg");
        assert!(has_chunk(&full, b"Glk "), "save_state still carries the Glk chunk");
    }

    // ── Task 2 (SQ-0283): restore_quetzal (live-state preserving) ─────────────

    /// The rule behind hiding a foreign embedded save must match the rule
    /// `@restore` refuses one by, or we would either hide a save the game could
    /// have used or offer one it cannot (SQ-0595).
    #[test]
    fn quetzal_resource_hidden_iff_restore_would_reject_it() {
        use asm::Op::{Mem32, Zero};
        let body = asm::ins(0x0123, &[Zero, Mem32(0x180)]);
        let mut m = machine_with_body(&[], body);
        assert_eq!(m.step(), StepResult::SaveRequest);
        let blob = m.save_quetzal();
        m.complete_save(true);

        // Our own save: offered, and restorable.
        assert!(!m.quetzal_for_another_story(&blob), "a matching save stays visible");

        // A save for another story: hidden, AND refused by restore. Flip a byte of
        // the IFhd's RAMSTART — a field no running game can change, which is what
        // made Counterfeit Monkey's embedded save provably foreign.
        let mut foreign = blob.clone();
        let mut i = 12usize;
        let ifhd_at = loop {
            let len = u32::from_be_bytes([foreign[i + 4], foreign[i + 5], foreign[i + 6], foreign[i + 7]]) as usize;
            if &foreign[i..i + 4] == b"IFhd" {
                break i + 8;
            }
            i += 8 + len + (len & 1);
        };
        foreign[ifhd_at + 9] ^= 0xFF; // inside RAMSTART
        assert!(m.quetzal_for_another_story(&foreign), "a foreign save is hidden");
        assert!(
            m.restore_quetzal(&foreign).is_err(),
            "...and is exactly what restore would refuse — the two rules must agree"
        );

        // Anything that is not a save is always offered: an ordinary data resource
        // must not be second-guessed.
        assert!(!m.quetzal_for_another_story(b"\x89PNG\r\n\x1a\n and then some pixels"));
        assert!(!m.quetzal_for_another_story(b""));
        assert!(!m.quetzal_for_another_story(b"FORM\0\0\0\x04IFRS"), "a Blorb is not a save");
    }

    /// `@restore L1 S1` restores from the STREAM it is handed (Glulx §1.8.2). A
    /// **resource stream** has no fileref name, so routing it through the host's
    /// by-name path silently found nothing and the restore failed — invisibly,
    /// because a game that ships a pre-computed save as a Blorb `Data` chunk and
    /// restores it at boot just falls back to recomputing (SQ-0595).
    ///
    /// The blob must go through `restore_quetzal`, not `restore_state`: a save a
    /// game carries in its own Blorb is spec-standard, with none of the
    /// `GReg`/`Glk ` chunks our host snapshots add, and `restore_state` rejects it
    /// for the missing `GReg`.
    #[test]
    fn restore_reads_the_blob_from_a_resource_stream_with_no_fileref_name() {
        use asm::Op::{C8, Mem32, Zero};
        // @save 0 -> mem[0x180]; copy 0x7F -> mem[0x184]; quit.
        let mut body = asm::ins(0x0123, &[Zero, Mem32(0x180)]);
        body.extend(asm::ins(0x40, &[C8(0x7F), Mem32(0x184)]));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);

        assert_eq!(m.step(), StepResult::SaveRequest);
        let blob = m.save_quetzal();
        m.complete_save(true);
        while m.step() == StepResult::Continue {}
        assert_eq!(m.mem.read32(0x184).unwrap(), 0x7F, "the post-save sentinel ran");

        // Replay the same story, restoring from a RESOURCE stream — the shape a
        // game's embedded startup save takes. No fileref, no host involvement.
        let mut body2 = asm::ins(0x0123, &[Zero, Mem32(0x180)]);
        body2.extend(asm::ins(0x40, &[C8(0x7F), Mem32(0x184)]));
        body2.extend(asm::ins(0x120, &[]));
        let mut m2 = machine_with_body(&[], body2);
        let sid = m2.glk.stream_open_resource(blob.clone(), false, false, 0);
        assert!(sid != 0, "resource stream opens");
        assert_eq!(
            m2.glk.stream_saveload_info(sid),
            None,
            "a resource stream has no fileref name — the case the host path cannot serve"
        );
        assert_eq!(
            m2.glk.stream_restore_bytes(sid).map(|b| b.len()),
            Some(blob.len()),
            "its bytes are readable in-process"
        );

        // @restore sid -> mem[0x188]
        let restore = asm::ins(0x0124, &[asm::Op::C8(sid as i8), Mem32(0x188)]);
        let mut m3 = machine_with_body(&[], restore);
        let sid3 = m3.glk.stream_open_resource(blob, false, false, 0);
        assert_eq!(sid3, sid, "same stream id in the fresh machine");
        // Executing @restore must NOT suspend for the host: it completes inline.
        let step = m3.step();
        assert_ne!(step, StepResult::RestoreRequest, "a resource stream never suspends for the host");
        assert_eq!(
            m3.mem.read32(0x184).unwrap(),
            0,
            "restored to the SAVE point, before the sentinel was written"
        );
    }

    /// The key §1.8.5 property: `restore_quetzal` reverts VM memory/stack/heap to
    /// the save point and resumes with `-1` in `@save`'s S1, but leaves the live
    /// Glk model (windows/streams/VFS) and `iosys_mode`/`iosys_rock` completely
    /// untouched — including further mutations made to them AFTER the save. This
    /// is what distinguishes it from `restore_state`, which replaces that state
    /// from `GReg`/`Glk `.
    ///
    /// (SQ-1065: this sentence was split in half by a test inserted between its
    /// two paragraphs, so each half read as a claim about the wrong case.)
    #[test]
    fn restore_quetzal_reverts_vm_state_but_preserves_live_glk_and_iosys() {
        use asm::Op::{C8, Mem32, Zero};
        // @save 0, -> mem[0x180]; copy 0x7F -> mem[0x184] (sentinel); quit.
        let mut body = asm::ins(0x0123, &[Zero, Mem32(0x180)]);
        body.extend(asm::ins(0x40, &[C8(0x7F), Mem32(0x184)]));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);

        // Live state that restore_quetzal must leave alone.
        let root = m.glk.root();
        let fref = m.glk.fileref_create(0x00, "witness".to_string(), 0);
        let sid = m.glk.stream_open_file(fref, 0x01, false, 0); // Write
        m.glk.file_stream_write(sid, "A");
        m.iosys_mode = 2;
        m.iosys_rock = 0x55;

        // VM state that restore_quetzal must revert to the save point.
        m.mem.write32(0x190, 0xCAFE_BABE).unwrap();

        assert_eq!(m.step(), StepResult::SaveRequest);
        let blob = m.save_quetzal();

        // Mutate everything after the save point.
        m.mem.write32(0x190, 0).unwrap();
        m.iosys_mode = 9;
        m.iosys_rock = 0x77;
        m.glk.file_stream_write(sid, "B"); // more writes to the SAME live stream

        m.restore_quetzal(&blob).unwrap();
        // restore_quetzal is the low-level primitive (mirrors restore_state);
        // clearing the pending @save/@restore suspension is complete_restore_quetzal's
        // job (mirrors complete_restore_success), so do it by hand here to
        // check that stepping resumes normally afterward.
        m.pending_saveload = None;

        // VM memory reverted to save time.
        assert_eq!(m.mem.read32(0x190).unwrap(), 0xCAFE_BABE, "restore_quetzal reverts VM RAM to save time");
        // @save's S1 is stored with -1 (the "just restored" sentinel) directly
        // by the stub pop, before any further stepping.
        assert_eq!(m.mem.read32(0x180).unwrap(), 0xFFFF_FFFF, "restore_quetzal stores -1 into S1");

        // Live Glk/iosys untouched by restore_quetzal (§1.8.5) — the
        // post-save-mutation values survive; the bare format has no saved
        // register/model state to revert them to in the first place.
        assert_eq!(m.glk.root(), root, "the live window tree survives untouched");
        assert_eq!(m.iosys_mode, 9, "iosys is left at its live value, not reverted");
        assert_eq!(m.iosys_rock, 0x77, "iosys rock is left at its live value, not reverted");

        // The live stream (opened before the save) kept writing after the save
        // and survives the restore with everything it holds.
        m.glk.stream_close(sid);
        let rf2 = m.glk.fileref_create(0x00, "witness".to_string(), 0);
        let rsid = m.glk.stream_open_file(rf2, 0x02, false, 0); // Read
        let mut got = String::new();
        while let Some(c) = m.glk.file_stream_read_char(rsid) {
            got.push(c as u8 as char);
        }
        assert_eq!(got, "AB", "the live VFS stream survives restore_quetzal with post-save writes intact");

        // Resuming steps into the trailing sentinel, just after the original @save.
        assert_eq!(m.step(), StepResult::Continue, "resumes at the stub's PC, just after @save");
        assert_eq!(m.mem.read32(0x184).unwrap(), 0x7F, "execution reached the trailing sentinel");
    }

    /// SQ-0556, the question the whole quest hung on: does `restore_quetzal`
    /// TOLERATE the non-standard `GReg`/`Glk ` chunks — skipping them, the way
    /// Quetzal §8.9 says any reader should treat an unknown chunk — rather than
    /// applying them?
    ///
    /// It has to, and for a sharper reason than tidiness: a `Glk ` chunk carries
    /// a window tree that was current at save time, and reinstating it over a
    /// live session would hand the game a STALE window model — windows it has
    /// since split or closed, snapping back. Glulx §1.8.5 puts Glk library state
    /// (windows, filerefs, streams, output stream, window contents, cursor
    /// positions) deliberately outside the save for exactly that reason.
    ///
    /// So: feed `restore_quetzal` the RICH `save_state()` blob, having first
    /// moved the live model somewhere the chunk knows nothing about. The VM half
    /// must revert; the display half must not budge.
    #[test]
    fn restore_quetzal_skips_greg_and_glk_chunks_and_never_reinstates_a_stale_window_model() {
        use asm::Op::{C8, Mem32, Zero};
        fn has_chunk(save: &[u8], id: &[u8; 4]) -> bool {
            save.windows(4).any(|w| w == id)
        }
        // @save 0, -> mem[0x180]; copy 0x7F -> mem[0x184] (sentinel); quit.
        let mut body = asm::ins(0x0123, &[Zero, Mem32(0x180)]);
        body.extend(asm::ins(0x40, &[C8(0x7F), Mem32(0x184)]));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);

        // One window at save time; that is what the `Glk ` chunk will describe.
        let first = match m.glk.root() {
            0 => m.glk.window_open(0, 0, 0, 3, 0).expect("root buffer window opens"),
            existing => existing,
        };
        assert_eq!(m.glk.root(), first, "the lone window is the root at save time");
        m.iosys_mode = 2;
        m.mem.write32(0x190, 0xCAFE_BABE).unwrap();

        assert_eq!(m.step(), StepResult::SaveRequest);
        let rich = m.save_state();
        assert!(has_chunk(&rich, b"GReg"), "the blob under test really does carry GReg");
        assert!(has_chunk(&rich, b"Glk "), "the blob under test really does carry a Glk chunk");

        // Move the LIVE display state on, past anything the blob describes: split
        // a status window in (so the root is now a pair window the chunk has
        // never heard of) and change the I/O system.
        let status = m.glk.window_open(first, 0x12, 2, 4, 0).expect("status window splits in");
        let live_root = m.glk.root();
        assert_ne!(live_root, first, "the split moved the root to a pair window");
        m.iosys_mode = 9;
        m.mem.write32(0x190, 0).unwrap();

        m.restore_quetzal(&rich).expect("restore_quetzal accepts a blob carrying GReg + Glk chunks");
        m.pending_saveload = None;

        // The VM half came back from the blob.
        assert_eq!(m.mem.read32(0x190).unwrap(), 0xCAFE_BABE, "RAM reverted to the save point");
        assert_eq!(m.mem.read32(0x180).unwrap(), 0xFFFF_FFFF, "@save's S1 got the -1 sentinel");

        // The display half did NOT: the stale one-window tree in `Glk ` was
        // skipped, not applied. Were it applied, `root` would snap back to
        // `first` and the status window would vanish.
        assert_eq!(m.glk.root(), live_root, "the live (post-save) window tree survives; no stale model applied");
        assert!(m.glk.window_type(status).is_some(), "the window split in AFTER the save is still open");
        // GReg was skipped too — iosys keeps its live value instead of the saved one.
        assert_eq!(m.iosys_mode, 9, "GReg was skipped: iosys keeps its live value");

        // And the machine really is runnable afterwards.
        assert_eq!(m.step(), StepResult::Continue, "resumes at the stub's PC, just after @save");
        assert_eq!(m.mem.read32(0x184).unwrap(), 0x7F, "execution reached the trailing sentinel");
    }

    /// `restore_quetzal` must accept a `UMem` (uncompressed) memory chunk, not
    /// just `CMem` — spec §1.8.
    #[test]
    fn restore_quetzal_accepts_umem_uncompressed_chunk() {
        use asm::Op::{Mem32, Zero};
        let mut body = asm::ins(0x0123, &[Zero, Mem32(0x180)]);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);
        m.mem.write32(0x190, 0x1111_2222).unwrap();
        assert_eq!(m.step(), StepResult::SaveRequest);

        // Hand-build a FORM IFZS with a literal UMem chunk (the entire
        // [RAMSTART, memsize) image) instead of a CMem diff.
        let memsize = m.mem.mem_size();
        let ramstart = m.mem.ramstart();
        let mut umem_body = Vec::new();
        umem_body.extend_from_slice(&memsize.to_be_bytes());
        for a in ramstart..memsize {
            umem_body.push(m.mem.read8(a).unwrap() as u8);
        }
        // Overwrite the target word in the desired image so the assertion
        // proves UMem was actually applied, not just left as-is.
        let idx = 4 + (0x190 - ramstart) as usize;
        umem_body[idx..idx + 4].copy_from_slice(&0xAAAA_BBBBu32.to_be_bytes());

        let mut ifhd = Vec::with_capacity(128);
        for a in 0..128 {
            ifhd.push(m.mem.read8(a).unwrap_or(0) as u8);
        }
        let mut body_chunks = Vec::new();
        push_chunk(&mut body_chunks, b"IFhd", &ifhd);
        push_chunk(&mut body_chunks, b"UMem", &umem_body);
        push_chunk(&mut body_chunks, b"Stks", &m.stack[..m.sp]);
        push_chunk(&mut body_chunks, b"MAll", &[]);
        let mut blob = Vec::new();
        blob.extend_from_slice(b"FORM");
        blob.extend_from_slice(&((body_chunks.len() + 4) as u32).to_be_bytes());
        blob.extend_from_slice(b"IFZS");
        blob.extend_from_slice(&body_chunks);

        m.restore_quetzal(&blob).unwrap();
        assert_eq!(m.mem.read32(0x190).unwrap(), 0xAAAA_BBBB, "UMem chunk applied literally");
    }

    /// `restore_quetzal` must reject a save whose `IFhd` chunk (the first 128
    /// bytes of memory, captured at save time — spec §1.8.4) doesn't match the
    /// running story's current header, and must leave VM state untouched
    /// (no partial mutation) rather than applying it anyway.
    #[test]
    fn restore_quetzal_rejects_a_save_from_a_different_story() {
        use asm::Op::{Mem32, Zero};
        let mut body = asm::ins(0x0123, &[Zero, Mem32(0x180)]);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);
        m.mem.write32(0x190, 0xCAFE_BABE).unwrap();
        assert_eq!(m.step(), StepResult::SaveRequest);
        let mut blob = m.save_quetzal();

        // Corrupt a byte inside the blob's IFhd chunk payload. Layout:
        // FORM(4) + size(4) + "IFZS"(4) + chunk-id(4) + chunk-len(4) = 20
        // bytes before the 128-byte IFhd data begins.
        blob[20] ^= 0xFF;

        // Witness: mutate RAM after taking the blob. A wrongly-applied
        // restore would revert this to the saved 0xCAFE_BABE; a correctly
        // rejected restore must leave it at the post-save value.
        m.mem.write32(0x190, 0).unwrap();

        let err = m.restore_quetzal(&blob).unwrap_err();
        assert!(matches!(err, GError::BadSave(_)), "mismatched IFhd is rejected: {err:?}");
        assert_eq!(m.mem.read32(0x190).unwrap(), 0, "rejected restore leaves VM state untouched");
    }

    /// The live protect range (not part of the bare-save format) is honored
    /// during `restore_quetzal`'s memory diff: protected bytes keep their
    /// pre-restore (live) values instead of the blob's (spec §1.8.5).
    #[test]
    fn restore_quetzal_honors_the_live_protect_range() {
        use asm::Op::{Mem32, Zero};
        let mut body = asm::ins(0x0123, &[Zero, Mem32(0x180)]);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);
        assert_eq!(m.step(), StepResult::SaveRequest);
        let blob = m.save_quetzal(); // save-time: 0x190 and 0x194 both read 0

        // Protect [0x190, 0x194) live, then mutate inside and outside it.
        m.protect = (0x190, 4);
        m.mem.write32(0x190, 0x2222_2222).unwrap(); // inside protect
        m.mem.write32(0x194, 0x3333_3333).unwrap(); // outside protect

        m.restore_quetzal(&blob).unwrap();

        assert_eq!(m.mem.read32(0x190).unwrap(), 0x2222_2222, "protected bytes keep their live pre-restore value");
        assert_eq!(m.mem.read32(0x194).unwrap(), 0, "unprotected bytes are restored from the blob");
    }

    #[test]
    fn is_saveload_pending_reflects_a_suspended_save_or_restore() {
        use asm::Op::{Mem32, Zero};
        let body = asm::ins(0x0123, &[Zero, Mem32(0x180)]); // @save 0, -> mem[0x180]
        let mut m = machine_with_body(&[], body);
        assert!(!m.is_saveload_pending(), "nothing pending at start");
        assert_eq!(m.step(), StepResult::SaveRequest);
        assert!(m.is_saveload_pending(), "pending while suspended on @save");
        m.complete_save(true);
        assert!(!m.is_saveload_pending(), "cleared once complete_save resolves it");
    }

    #[test]
    fn cmem_resets_to_original_then_applies_diff() {
        let mut m = machine_with_body(&[], vec![]);
        m.mem.write8(0x150, 0xAB).unwrap();
        let snap = m.save_state();
        // A byte changed after the save must be reset to the original image (0).
        m.mem.write8(0x150, 0x00).unwrap();
        m.mem.write8(0x108, 0xFF).unwrap();
        m.restore_state(&snap).unwrap();
        assert_eq!(m.mem.read8(0x150).unwrap(), 0xAB); // saved diff re-applied
        assert_eq!(m.mem.read8(0x108).unwrap(), 0x00); // not in the save → original
    }

    #[test]
    fn restore_rejects_corrupt_save_without_panic() {
        let mut m = machine_with_body(&[], vec![]);
        assert!(matches!(m.restore_state(b"not a save"), Err(GError::BadSave(_))));
        assert!(matches!(m.restore_state(&[]), Err(GError::BadSave(_))));
        let mut good = m.save_state();
        good.truncate(good.len() - 10); // sever a chunk
        assert!(matches!(m.restore_state(&good), Err(GError::BadSave(_))));
    }

    // ── Glk-model snapshot ("Glk " chunk): cross-session + back-compat ─────────

    /// Rebuild a `FORM IFZS` save omitting one chunk (simulates an older gvm
    /// snapshot that predates a given chunk). Test helper.
    fn strip_chunk(save: &[u8], target: &[u8; 4]) -> Vec<u8> {
        let mut body = Vec::new();
        let mut p = 12;
        while p + 8 <= save.len() {
            let len = u32::from_be_bytes([save[p + 4], save[p + 5], save[p + 6], save[p + 7]]) as usize;
            let total = 8 + len + (len & 1);
            if &save[p..p + 4] != target {
                body.extend_from_slice(&save[p..(p + total).min(save.len())]);
            }
            p += total;
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(b"IFZS");
        out.extend_from_slice(&body);
        out
    }

    /// A snapshot carries the Glk window/stream model in a `Glk ` chunk, so
    /// restoring into a FRESH machine reinstalls the window tree, streams, and
    /// current state — and subsequent output routes to the right window.
    #[test]
    fn glk_model_survives_cross_session_restore() {
        use crate::glk::{GlkStyle, StreamKind};
        let start = asm::func(0xC1, &[], &asm::ins(0x120, &[])); // body: quit
        let built = asm::assemble(&[start], 0, 0x100);
        let image = built.image.clone();
        let mut m = Machine::with_glk(Memory::new(built.image).unwrap(), Box::new(TestBackend::new()));

        // A non-trivial model: a buffer root split into a grid (so the root is a
        // pair window), a memory stream with a moved position, a styled+current
        // grid stream, and a positioned grid cursor.
        let buf = m.glk.window_open(0, 0, 0, 3, 0xB0).unwrap(); // root TextBuffer
        let grid = m.glk.window_open(buf, 0x12, 3, 4, 0x61).unwrap(); // grid above, fixed 3
        m.glk.relayout(80, 24, (1, 1));
        let mem_stream = m.glk.stream_open_memory(0x180, 16, false, 3, 0x5E); // ReadWrite: seekable
        m.glk.stream_set_position(mem_stream, 5, 0);
        let grid_stream = m.glk.window_stream(grid).unwrap();
        let buf_stream = m.glk.window_stream(buf).unwrap();
        m.glk.set_stream_style(grid_stream, GlkStyle::Header);
        m.glk.set_grid_cursor(grid, 2, 1);
        m.glk.set_current_stream(grid_stream);
        let root = m.glk.root();

        let snap = m.save_state();

        // A FRESH machine over the same image starts with an EMPTY model.
        let mut m2 = Machine::with_glk(Memory::new(image).unwrap(), Box::new(TestBackend::new()));
        assert_eq!(m2.glk.root(), 0, "fresh machine has no windows");
        m2.restore_state(&snap).unwrap();

        // Window tree: ids, types, rocks, and the pair split are all restored.
        assert_eq!(m2.glk.root(), root);
        assert_eq!(m2.glk.window_type(buf), Some(WinType::TextBuffer));
        assert_eq!(m2.glk.window_rock(buf), Some(0xB0));
        assert_eq!(m2.glk.window_type(grid), Some(WinType::TextGrid));
        assert_eq!(m2.glk.window_rock(grid), Some(0x61));
        assert_eq!(m2.glk.window_type(root), Some(WinType::Pair));
        assert_eq!(m2.glk.window_parent(grid), Some(root));
        assert_eq!(m2.glk.window_sibling(grid), Some(buf));
        // Text-grid dimensions + cursor restored.
        assert_eq!(m2.glk.grid_state(grid), Some((80, 3, 2, 1)));
        // Streams: memory addr/len/pos/rock + current stream + style.
        assert_eq!(m2.glk.stream_position(mem_stream), Some(5));
        assert_eq!(m2.glk.stream_rock(mem_stream), Some(0x5E));
        assert_eq!(m2.glk.current_stream(), grid_stream);
        let (kind, style, _link) = m2.glk.stream_kind_style(grid_stream).unwrap();
        assert_eq!(style, GlkStyle::Header);
        assert!(matches!(kind, StreamKind::Window(w) if w == grid));

        // Routing after a cross-session restore: a put on the current (grid)
        // stream lands in the grid window at its restored cursor (row 1, col 2+).
        // (The host re-lays the restored tree out to its fresh backend first.)
        let layout = m2.glk.relayout(80, 24, (1, 1));
        m2.backend.window_layout(&layout);
        m2.glk_stream_put(grid_stream, "Hi");
        assert_eq!(backend_of(&m2).grid_line(grid, 1), "  Hi");
        // And a put on the buffer window's stream routes to the buffer window.
        m2.glk_stream_put(buf_stream, "Z");
        assert_eq!(backend_of(&m2).text(buf), "Z");
    }

    /// A snapshot WITHOUT a `Glk ` chunk (an older gvm save) restores with an
    /// empty model and returns Ok (no panic) — back-compat.
    #[test]
    fn restore_without_glk_chunk_leaves_empty_model() {
        let mut m = machine_with_body(&[], vec![]); // prelude opened window 1
        assert_ne!(m.glk.root(), 0);
        let snap = m.save_state();
        let stripped = strip_chunk(&snap, b"Glk ");
        assert!(stripped.len() < snap.len(), "a Glk chunk was present and removed");
        m.restore_state(&stripped).unwrap(); // no panic
        assert_eq!(m.glk.root(), 0, "missing Glk chunk -> empty model");
        assert_eq!(m.glk.current_stream(), 0);
    }

    /// Same-session restore reinstalls the saved model exactly: windows/streams
    /// opened after the save are gone after restoring it.
    #[test]
    fn same_session_restore_reinstalls_saved_model() {
        let mut m = machine_with_body(&[], vec![]); // prelude: buffer window 1 current
        let buf = m.glk.root();
        let buf_stream = m.glk.window_stream(buf).unwrap();
        let snap = m.save_state();
        // Diverge the model: split a grid, open a memory stream, change current.
        let grid = m.glk.window_open(buf, 0x12, 2, 4, 0).unwrap();
        let extra = m.glk.stream_open_memory(0x180, 8, false, 1, 0);
        m.glk.set_current_stream(extra);
        assert_ne!(m.glk.current_stream(), buf_stream);
        assert!(m.glk.window_type(grid).is_some());
        m.restore_state(&snap).unwrap();
        // Back to the saved single-window state.
        assert_eq!(m.glk.root(), buf);
        assert_eq!(m.glk.current_stream(), buf_stream);
        assert_eq!(m.glk.window_type(grid), None, "post-save window removed by restore");
        assert_eq!(m.glk.stream_rock(extra), None, "post-save stream removed by restore");
    }

    // ── Task 2 (2c): saveundo / restoreundo ───────────────────────────────────

    #[test]
    fn saveundo_then_restoreundo_restores_and_returns_minus_one() {
        let mut body = asm::ins(0x125, &[asm::Op::Local8(0)]); // saveundo -> local0
        body.extend(asm::ins(0x40, &[asm::Op::C32(0xDEAD), asm::Op::Mem16(0x0110)])); // mutate
        body.extend(asm::ins(0x126, &[asm::Op::Mem16(0x0104)])); // restoreundo -> mem 0x104
        body.extend(asm::ins(0x120, &[])); // quit
        let mut m = machine_with_body(&[(4, 1)], body);

        m.step_once().unwrap(); // saveundo
        assert_eq!(m.local_load(0).unwrap(), 0); // success stores 0

        m.step_once().unwrap(); // mutate
        assert_eq!(m.mem.read32(0x110).unwrap(), 0xDEAD);

        m.step_once().unwrap(); // restoreundo → resumes just after saveundo
        assert_eq!(m.mem.read32(0x110).unwrap(), 0); // prior state restored
        assert_eq!(m.local_load(0).unwrap(), 0xFFFF_FFFF); // saveundo "returns" -1
        assert_eq!(m.mem.read32(0x104).unwrap(), 0); // restoreundo stored nothing on success
    }

    #[test]
    fn hasundo_and_discardundo_track_the_undo_stack() {
        // hasundo→mem (empty), saveundo, hasundo→mem (available), discardundo,
        // hasundo→mem (empty again), quit.
        let mut body = asm::ins(0x128, &[asm::Op::Mem16(0x0100)]);   // hasundo → mem[0x100]
        body.extend(asm::ins(0x125, &[asm::Op::Mem16(0x0104)]));     // saveundo → mem[0x104]
        body.extend(asm::ins(0x128, &[asm::Op::Mem16(0x0108)]));     // hasundo → mem[0x108]
        body.extend(asm::ins(0x129, &[]));                           // discardundo
        body.extend(asm::ins(0x128, &[asm::Op::Mem16(0x010C)]));     // hasundo → mem[0x10C]
        body.extend(asm::ins(0x120, &[]));                           // quit
        let mut m = machine_with_body(&[(4, 1)], body);

        m.step_once().unwrap(); // hasundo (empty)
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "no undo state yet → 1");

        m.step_once().unwrap(); // saveundo
        m.step_once().unwrap(); // hasundo (available)
        assert_eq!(m.mem.read32(0x108).unwrap(), 0, "undo state available → 0");

        m.step_once().unwrap(); // discardundo
        m.step_once().unwrap(); // hasundo (empty again)
        assert_eq!(m.mem.read32(0x10C).unwrap(), 1, "undo state discarded → 1");
    }

    #[test]
    fn restoreundo_empty_fails_with_one() {
        let mut body = asm::ins(0x126, &[asm::Op::Mem16(0x0100)]); // no prior saveundo
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);
        m.mem.write32(0x110, 0x1234).unwrap();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 1); // failure stores 1
        assert_eq!(m.mem.read32(0x110).unwrap(), 0x1234); // state unchanged
    }

    #[test]
    fn undo_stack_is_bounded() {
        let mut body = Vec::new();
        for _ in 0..(Machine::UNDO_CAP + 3) {
            body.extend(asm::ins(0x125, &[asm::Op::Zero])); // saveundo, discard result
        }
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);
        m.run();
        assert_eq!(m.undo_stack.len(), Machine::UNDO_CAP);
    }

    // ── Task 3 (2c): protect ──────────────────────────────────────────────────

    #[test]
    fn protect_opcode_sets_and_clears() {
        let mut body = asm::ins(0x127, &[asm::Op::C16(0x0110), asm::Op::C8(4)]);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        assert_eq!(m.protect, (0x110, 4));

        // protect(_, 0) clears protection.
        let mut body = asm::ins(0x127, &[asm::Op::C16(0x0110), asm::Op::C8(0)]);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);
        m.protect = (0x110, 4);
        m.step_once().unwrap();
        assert_eq!(m.protect, (0x110, 0));
    }

    #[test]
    fn protect_preserves_range_across_restore() {
        let mut m = machine_with_body(&[], vec![]);
        m.protect = (0x110, 4);
        m.mem.write32(0x110, 0xAAAA).unwrap();
        let snap = m.save_state();
        m.mem.write32(0x110, 0xBEEF).unwrap(); // change the protected word
        m.restore_state(&snap).unwrap();
        assert_eq!(m.mem.read32(0x110).unwrap(), 0xBEEF); // kept current, not restored 0xAAAA
        assert_eq!(m.protect, (0x110, 4)); // range survives
    }

    #[test]
    fn protect_survives_restoreundo() {
        let mut body = asm::ins(0x127, &[asm::Op::C16(0x0110), asm::Op::C8(4)]); // protect
        body.extend(asm::ins(0x125, &[asm::Op::Zero])); // saveundo
        body.extend(asm::ins(0x40, &[asm::Op::C32(0xBEEF), asm::Op::Mem16(0x0110)])); // change
        body.extend(asm::ins(0x126, &[asm::Op::Mem16(0x0104)])); // restoreundo
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap(); // protect
        m.step_once().unwrap(); // saveundo
        m.step_once().unwrap(); // change → 0xBEEF
        assert_eq!(m.mem.read32(0x110).unwrap(), 0xBEEF);
        m.step_once().unwrap(); // restoreundo, resumes just after saveundo
        assert_eq!(m.mem.read32(0x110).unwrap(), 0xBEEF); // protected → kept current value
        assert_eq!(m.protect, (0x110, 4));
    }

    /// SQ-0320: `@protect` + a restore that re-extends memory. When the protected
    /// range lies above the (shrunken) memory top at restore time, the restore
    /// must bring those bytes back as 0 — not leak the saved diff into them. The
    /// pre-resize snapshot must capture out-of-bounds protected addresses as 0.
    #[test]
    fn restore_zeroes_a_protected_range_deallocated_by_a_shrink() {
        let mut m = machine_with_body(&[], vec![]);
        let base = m.mem.mem_size(); // grow beyond the current top
        let grown = base + 0x100;
        m.mem.set_raw_size(grown);
        // Distinct non-zero bytes across the grown region; protect [base+1, base+6).
        for i in 0..12u32 {
            m.mem.write8(base + i, i + 1).unwrap();
        }
        m.protect = (base + 1, 5);
        let snap = m.save_state(); // captures memsize=grown incl. bytes 1..=12

        m.mem.set_raw_size(base); // shrink below the protected range (deallocates it)
        m.restore_state(&snap).unwrap(); // re-extends to grown, applies the diff

        assert_eq!(m.mem.mem_size(), grown);
        assert_eq!(m.mem.read8(base).unwrap(), 1, "outside protect → restored");
        for a in base + 1..base + 6 {
            assert_eq!(m.mem.read8(a).unwrap(), 0, "protected byte {a:#x} must not leak the saved diff");
        }
        assert_eq!(m.mem.read8(base + 6).unwrap(), 7, "outside protect → restored");
    }

    // ── Task 4 (2c): accel storage, PRNG, verify, gestalt ─────────────────────

    #[test]
    fn accelfunc_accelparam_store_assignments() {
        let mut body = asm::ins(0x180, &[asm::Op::C8(7), asm::Op::C32(0x24)]); // accelfunc 7 @0x24
        body.extend(asm::ins(0x181, &[asm::Op::C8(3), asm::Op::C16(0x99)])); // accelparam 3 = 0x99
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.accel_func_for(0x24), Some(7));
        assert_eq!(m.accel_param(3), Some(0x99));

        // accelfunc(0, addr) cancels the assignment.
        let mut body = asm::ins(0x180, &[asm::Op::C8(7), asm::Op::C32(0x24)]);
        body.extend(asm::ins(0x180, &[asm::Op::C8(0), asm::Op::C32(0x24)]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(m.accel_func_for(0x24), None);
    }

    #[test]
    fn state_roundtrips_with_accel_assignments() {
        let mut m = machine_with_body(&[], vec![]);
        m.accel_funcs.insert(0x24, 7);
        m.accel_params.insert(3, 0x99);
        let snap = m.save_state();
        m.mem.write32(0x110, 0xDEAD).unwrap();
        m.restore_state(&snap).unwrap();
        // Accel assignments are interpreter config, untouched by save/restore.
        assert_eq!(m.accel_func_for(0x24), Some(7));
        assert_eq!(m.accel_param(3), Some(0x99));
        assert_eq!(m.mem.read32(0x110).unwrap(), 0); // RAM restored
    }

    #[test]
    fn random_honors_range_bounds() {
        let mut m = machine_with_body(&[], vec![]);
        for _ in 0..2000 {
            assert!((0..10).contains(&(m.rand_range(10) as i32))); // [0, 10)
            assert!((-9..=0).contains(&(m.rand_range((-10i32) as u32) as i32))); // (-10, 0]
        }
        let _ = m.rand_range(0); // full 32-bit range: just exercise (no panic)
    }

    #[test]
    fn setrandom_seed_is_reproducible() {
        let run = |seed: u32| {
            let mut body = asm::ins(0x111, &[asm::Op::C32(seed)]); // setrandom
            body.extend(asm::ins(0x110, &[asm::Op::C32(1_000_000), asm::Op::Mem16(0x0100)]));
            body.extend(asm::ins(0x110, &[asm::Op::C32(1_000_000), asm::Op::Mem16(0x0104)]));
            body.extend(asm::ins(0x120, &[]));
            let m = run_program(body);
            (m.mem.read32(0x100).unwrap(), m.mem.read32(0x104).unwrap())
        };
        assert_eq!(run(0x1234), run(0x1234)); // same seed → same sequence
        assert_ne!(run(0x1234), run(0x4321)); // different seed → different sequence
        assert_ne!(run(0), run(0)); // setrandom(0): entropy-seeded, non-reproducible
    }

    /// Draw two `random` values from a machine the HOST seeded before the program
    /// ran — the way `GlulxSession` seeds one before the boot drive (SQ-0811) —
    /// with no `setrandom` in the program at all. A game that never seeds itself
    /// is exactly the case the fixed construction seed used to decide forever.
    fn host_seeded_draws(seed: Option<u32>) -> (u32, u32) {
        let mut body = asm::ins(0x110, &[asm::Op::C32(1_000_000), asm::Op::Mem16(0x0100)]);
        body.extend(asm::ins(0x110, &[asm::Op::C32(1_000_000), asm::Op::Mem16(0x0104)]));
        body.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mut m = machine(built);
        if let Some(s) = seed {
            m.set_rng_seed(s);
        }
        m.run();
        (m.mem.read32(0x100).unwrap(), m.mem.read32(0x104).unwrap())
    }

    #[test]
    fn a_host_seed_decides_a_game_that_never_calls_setrandom() {
        // Reproducible under a pinned seed…
        assert_eq!(host_seeded_draws(Some(0xC0FF_EE00)), host_seeded_draws(Some(0xC0FF_EE00)));
        // …and genuinely steered by it, so seeding cannot be silently a no-op.
        assert_ne!(host_seeded_draws(Some(0xC0FF_EE00)), host_seeded_draws(Some(0x0BAD_1DEA)));
    }

    #[test]
    fn an_unseeded_machine_is_the_fixed_default_and_seed_zero_joins_it() {
        // A bare `Machine` stays deterministic — the rest of this file depends on
        // it — and a zero seed is coerced, never absorbed (xorshift32 at 0 is stuck).
        assert_eq!(host_seeded_draws(None), host_seeded_draws(Some(Machine::DEFAULT_SEED)));
        assert_eq!(host_seeded_draws(Some(0)), host_seeded_draws(None));
    }

    #[test]
    fn restart_keeps_the_host_seed_instead_of_snapping_back_to_the_default() {
        // A restart that reset the PRNG to `DEFAULT_SEED` would hand a restarted
        // roguelike the same opening as the last one, whatever the host seeded —
        // SQ-0811's defect, one `@restart` later.
        let mut m = machine_with_body(&[], asm::ins(0x120, &[]));
        m.set_rng_seed(0xC0FF_EE00);
        let _ = m.rand_range(1_000_000); // advance the stream, as a played turn would
        let advanced = m.rng_seed();
        m.op_restart().expect("restart re-enters the start function");
        assert_ne!(m.rng_seed(), Machine::DEFAULT_SEED, "restart must not re-impose the default");
        // And the stream CARRIES ON rather than rewinding, so the restarted game
        // is a new deal and not the one just abandoned.
        assert_eq!(m.rng_seed(), advanced);
    }

    #[test]
    fn verify_detects_a_bad_checksum() {
        let mut vbody = asm::ins(0x121, &[asm::Op::Mem16(0x0100)]);
        vbody.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &vbody);
        let mut built = asm::assemble(&[start], 0, 0x100);
        built.image[0x20] ^= 0xFF; // corrupt the stored checksum
        let mut m = machine(built);
        m.run();
        assert_eq!(m.mem.read32(0x100).unwrap(), 1); // problem detected
    }

    #[test]
    fn gestalt_reports_undo_and_accel_supported() {
        let m = machine_with_body(&[], vec![]);
        assert_eq!(m.gestalt(3, 0), 1); // Undo now supported
        assert_eq!(m.gestalt(9, 0), 1); // Acceleration: interception implemented
        assert_eq!(m.gestalt(10, 0), 0); // AccelFunc 0 is "cancel", not a function
    }

    #[test]
    fn gestalt_reports_acceleration_supported() {
        let m = machine_with_body(&[], vec![]);
        assert_eq!(m.gestalt(9, 0), 1); // Acceleration: interception implemented
        assert_eq!(m.gestalt(10, 0), 0); // AccelFunc 0 is "cancel", not a function
        assert_eq!(m.gestalt(10, 1), 1); // Z__Region implemented
        assert_eq!(m.gestalt(10, 13), 1); // last implemented
        assert_eq!(m.gestalt(10, 14), 0); // beyond the set
    }

    #[test]
    fn acceleration_defaults_on_and_toggles() {
        let mut m = machine_with_body(&[], vec![]);
        assert!(m.acceleration);
        m.set_acceleration(false);
        assert!(!m.acceleration);
    }

    // ── Task 1 (Glk model): seam migration behaviors ──────────────────────────

    /// With no window open and no current stream, Glk output is safely discarded
    /// (no panic). Build a raw machine WITHOUT the test prelude.
    #[test]
    fn glk_output_with_no_current_stream_is_discarded() {
        let mut body = asm::ins(0x149, &[asm::Op::C8(2), asm::Op::C8(0)]); // setiosys glk
        body.extend(asm::ins(0x71, &[asm::Op::C8(42)])); // streamnum 42 (no window!)
        // glk_put_char('B') directly — also routes to the (absent) current stream.
        body.extend(asm::ins(0x40, &[asm::Op::C8(66), asm::Op::Stack]));
        body.extend(asm::ins(0x130, &[asm::Op::C16(0x80), asm::Op::C8(1), asm::Op::Zero]));
        body.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x100);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new())); // no prelude
        m.run();
        assert!(m.halted);
        let backend = m.backend.as_any().downcast_ref::<TestBackend>().unwrap();
        assert_eq!(backend.all_text(), ""); // nothing printed, no panic
    }

    /// `glk_window_open` builds a TextBuffer window whose stream `glk_set_window`
    /// makes current; subsequent `glk_put_char` lands in that window.
    #[test]
    fn glk_window_open_and_set_window_route_output() {
        // The shared `machine()` prelude already opens window 1 + sets it current,
        // so a bare glk_put_char prints there.
        let mut body = asm::ins(0x40, &[asm::Op::C8(b'Z' as i8), asm::Op::Stack]); // push 'Z'
        body.extend(asm::ins(0x130, &[asm::Op::C16(0x80), asm::Op::C8(1), asm::Op::Zero]));
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        // Window 1 is the prelude TextBuffer; its recorded text is "Z".
        let backend = m.backend.as_any().downcast_ref::<TestBackend>().unwrap();
        assert_eq!(backend.text(1), "Z");
        assert_eq!(m.glk.root(), 1);
        assert_eq!(m.glk.window_type(1), Some(WinType::TextBuffer));
    }

    // ── Task 2 (Glk window tree, sizing, TextGrid) ────────────────────────────

    /// Emit code calling `@glk(selector, args)` storing the result via `store`.
    /// Args are pushed so `args[0]` is topmost (the first value `@glk` pops).
    fn glk_call(selector: u32, args: &[asm::Op], store: asm::Op) -> Vec<u8> {
        let mut body = Vec::new();
        for &arg in args.iter().rev() {
            body.extend(asm::ins(0x40, &[arg, asm::Op::Stack])); // copy arg -> push
        }
        body.extend(asm::ins(0x130, &[asm::Op::C32(selector), asm::Op::C8(args.len() as i8), store]));
        body
    }

    fn backend_of(m: &Machine) -> &TestBackend {
        m.backend.as_any().downcast_ref::<TestBackend>().unwrap()
    }

    #[test]
    fn glk_fileref_iterate_walks_live_filerefs() {
        use asm::Op::{C16, C8, Mem16};
        // Create one fileref by name, then iterate. Iteration now yields the live
        // fileref (with its rock) and then NULL -- and, crucially, emits no
        // "unhandled selector" diagnostic.
        // Name string "s" at 0x0120 (NUL-terminated).
        let mut body = glk_call(0x61, &[C8(0x00), C16(0x0120), C8(0x07)], Mem16(0x0100)); // create_by_name(usage, name, rock=7)
        body.extend(glk_call(0x64, &[C8(0), C16(0x0114)], Mem16(0x0104))); // iterate(0) -> first
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            poke(m, 0x0120, b"s\0");
        });
        let fref = m.mem.read32(0x100).unwrap();
        assert_ne!(fref, 0, "create_by_name -> live fileref");
        assert_eq!(m.mem.read32(0x104).unwrap(), fref, "iterate(0) yields the created fileref");
        assert_eq!(m.mem.read32(0x114).unwrap(), 7, "iterate reports the fileref's rock");
        assert!(m.diagnostics.is_empty(), "fileref_iterate must be a handled selector");
    }

    #[test]
    fn glk_fileref_create_by_name_returns_live_ref() {
        use asm::Op::{C16, C8, Mem16};
        // create_by_name returns a NON-zero fileref; does_file_exist on a
        // never-written fileref is still false; destroy is a silent no-op.
        let mut body = glk_call(0x61, &[C8(0x00), C16(0x0120), C8(0x00)], Mem16(0x0100)); // create_by_name
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            poke(m, 0x0120, b"save\0");
        });
        let fref = m.mem.read32(0x100).unwrap();
        assert_ne!(fref, 0, "create_by_name -> live fileref id");
        assert!(m.diagnostics.is_empty(), "the fileref group must not emit diagnostics");
    }

    #[test]
    fn glk_fileref_group_is_silent() {
        use asm::Op::{C16, C8, Mem16};
        // The startup probe games run: create a fileref by name, check existence,
        // destroy it. does_file_exist on a never-written fileref is false, destroy
        // is a no-op -- and crucially no diagnostics (the spam reported on
        // CounterfeitMonkey's start).
        let mut body = glk_call(0x61, &[C8(0x02), C16(0x0120), C8(0x00)], Mem16(0x0100)); // create_by_name
        body.extend(glk_call(0x67, &[Mem16(0x0100)], Mem16(0x0104))); // does_file_exist(fref)
        body.extend(glk_call(0x63, &[Mem16(0x0100)], Mem16(0x0108))); // destroy(fref)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            poke(m, 0x0120, b"save\0");
        });
        assert_ne!(m.mem.read32(0x100).unwrap(), 0, "create_by_name -> live fileref");
        assert_eq!(m.mem.read32(0x104).unwrap(), 0, "does_file_exist -> false (never written)");
        assert!(m.diagnostics.is_empty(), "the fileref group must not emit diagnostics");
    }

    #[test]
    fn glk_fileref_create_by_prompt_suspends_then_supply_binds_name() {
        use asm::Op::{C8, Mem16};
        // create_by_prompt(usage=0x00, fmode=0x01 Write, rock=0) -> store fileref id @ 0x0100.
        let mut body = glk_call(0x62, &[C8(0x00), C8(0x01), C8(0x00)], Mem16(0x0100));
        body.extend(asm::ins(0x120, &[])); // quit (runs only after the prompt resumes)
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x200);
        let mut m = machine(built);
        // Drive to the suspension.
        let mut sr = m.step();
        while sr == StepResult::Continue {
            sr = m.step();
        }
        assert_eq!(sr, StepResult::NeedFilename { usage: 0x00, fmode: 0x01 });
        // Re-reported while pending (idempotent, like @save/@restore).
        assert_eq!(m.step(), StepResult::NeedFilename { usage: 0x00, fmode: 0x01 });
        // Player names it; execution resumes and runs to quit.
        m.supply_filename(Some("mydata".to_string()));
        let mut sr = m.step();
        while sr == StepResult::Continue {
            sr = m.step();
        }
        assert_eq!(sr, StepResult::Quit);
        assert_ne!(m.mem.read32(0x100).unwrap(), 0, "supply_filename bound a live (non-NULL) fileref");
    }

    #[test]
    fn glk_fileref_create_by_prompt_cancel_stores_null() {
        use asm::Op::{C8, Mem16};
        let mut body = glk_call(0x62, &[C8(0x00), C8(0x01), C8(0x00)], Mem16(0x0100));
        body.extend(asm::ins(0x120, &[]));
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x200);
        let mut m = machine(built);
        let mut sr = m.step();
        while sr == StepResult::Continue {
            sr = m.step();
        }
        assert_eq!(sr, StepResult::NeedFilename { usage: 0x00, fmode: 0x01 });
        m.supply_filename(None); // player cancelled
        let mut sr = m.step();
        while sr == StepResult::Continue {
            sr = m.step();
        }
        assert_eq!(sr, StepResult::Quit);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0, "cancel -> NULL fileref (0)");
    }

    #[test]
    fn glk_split_builds_pair_tree_with_fixed_sizes() {
        use asm::Op::{C16, C8, Mem16, Zero};
        // Prelude opens window 1 (TextBuffer root, 80x24). Split it: a TextGrid
        // ABOVE | FIXED (0x12), 3 rows. New grid = window 2; pair = window 3.
        let mut body = glk_call(0x23, &[C8(1), C8(0x12), C8(3), C8(4), C8(0)], Mem16(0x0100));
        body.extend(glk_call(0x22, &[], Mem16(0x0104))); // get_root
        body.extend(glk_call(0x25, &[C8(2), C16(0x0108), C16(0x010C)], Zero)); // get_size(grid)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "open returns the new grid window id");
        assert_eq!(m.mem.read32(0x104).unwrap(), 3, "root is the pair window");
        assert_eq!(m.mem.read32(0x108).unwrap(), 80, "grid width = full screen");
        assert_eq!(m.mem.read32(0x10C).unwrap(), 3, "grid height = fixed 3 rows");
    }

    #[test]
    fn glk_window_tree_queries_and_proportional_split() {
        use asm::Op::{C16, C8, Zero};
        // Split window 1 RIGHT | PROPORTIONAL | NoBorder (0x121), 25% of 80 = 20
        // cols. NoBorder keeps the full 80-col content (a bordered split reserves
        // a separator column, so 25% would be of 79); this test only cares about
        // the proportional math and the tree-navigation queries below.
        let mut body =
            glk_call(0x23, &[C8(1), C16(0x121), C8(25), C8(4), C8(0)], asm::Op::Mem16(0x0100));
        // get_size(grid=2) -> 0x108,0x10C ; get_size(buf=1) -> 0x110,0x114
        body.extend(glk_call(0x25, &[C8(2), C16(0x0108), C16(0x010C)], Zero));
        body.extend(glk_call(0x25, &[C8(1), C16(0x0110), C16(0x0114)], Zero));
        body.extend(glk_call(0x28, &[C8(2)], asm::Op::Mem16(0x0118))); // get_type(2)
        body.extend(glk_call(0x29, &[C8(2)], asm::Op::Mem16(0x011C))); // get_parent(2)
        body.extend(glk_call(0x30, &[C8(2)], asm::Op::Mem16(0x0120))); // get_sibling(2)
        body.extend(glk_call(0x28, &[C8(3)], asm::Op::Mem16(0x0124))); // get_type(pair)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x108).unwrap(), 20, "grid width = 25% of 80");
        assert_eq!(m.mem.read32(0x10C).unwrap(), 24, "grid height = full");
        assert_eq!(m.mem.read32(0x110).unwrap(), 60, "buffer width = remainder");
        assert_eq!(m.mem.read32(0x118).unwrap(), 4, "grid type = wintype_TextGrid");
        assert_eq!(m.mem.read32(0x11C).unwrap(), 3, "grid parent = pair window");
        assert_eq!(m.mem.read32(0x120).unwrap(), 1, "grid sibling = buffer window");
        assert_eq!(m.mem.read32(0x124).unwrap(), 1, "pair type = wintype_Pair");
    }

    #[test]
    fn glk_textgrid_move_cursor_and_put_writes_cells() {
        use asm::Op::{C16, C8, Mem16, Zero};
        // Open a grid above; move its cursor to (x=2,y=1); make it current; print.
        let mut body = glk_call(0x23, &[C8(1), C8(0x12), C8(3), C8(4), C8(0)], Mem16(0x0100));
        body.extend(glk_call(0x2B, &[C8(2), C8(2), C8(1)], Zero)); // move_cursor(2, 2,1)
        body.extend(glk_call(0x2F, &[C8(2)], Zero)); // set_window(2)
        body.extend(glk_call(0x82, &[C16(0x0200)], Zero)); // glk_put_string("Hi")
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x200, &[0xE0, b'H', b'i', 0]));
        assert_eq!(backend_of(&m).grid_line(2, 1), "  Hi"); // cols 2,3 hold "Hi"
    }

    #[test]
    fn glk_window_clear_buffer_and_grid() {
        use asm::Op::{C8, Zero};
        // Buffer (window 1, current via prelude): put 'X', clear, put 'Y' -> "Y".
        let mut body = glk_call(0x80, &[C8(b'X' as i8)], Zero);
        body.extend(glk_call(0x2A, &[C8(1)], Zero)); // glk_window_clear(1)
        body.extend(glk_call(0x80, &[C8(b'Y' as i8)], Zero));
        // Grid: open above, write at (0,0), clear it -> grid line empty.
        body.extend(glk_call(0x23, &[C8(1), C8(0x12), C8(3), C8(4), C8(0)], Zero)); // grid=2
        body.extend(glk_call(0x2F, &[C8(2)], Zero)); // set_window(2)
        body.extend(glk_call(0x80, &[C8(b'Q' as i8)], Zero)); // grid (0,0)='Q'
        body.extend(glk_call(0x2A, &[C8(2)], Zero)); // glk_window_clear(2)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x100, |_| {});
        assert_eq!(backend_of(&m).text(1), "Y", "buffer clear wiped 'X'");
        assert_eq!(backend_of(&m).grid_line(2, 0), "", "grid clear wiped 'Q'");
    }

    // ── Task 3 (Glk streams: window + memory) ─────────────────────────────────

    #[test]
    fn glk_memory_stream_write_position_and_close() {
        use asm::Op::{C16, C8, Mem16, Zero};
        // Prelude made window stream 1; open_memory -> stream 2 over [0x180,0x188).
        let mut body = glk_call(0x43, &[C16(0x0180), C8(8), C8(1), C8(0)], Mem16(0x0100));
        body.extend(glk_call(0x47, &[C8(2)], Zero)); // stream_set_current(2)
        body.extend(glk_call(0x82, &[C16(0x0200)], Zero)); // glk_put_string("Hi") -> memory
        body.extend(glk_call(0x46, &[C8(2)], Mem16(0x0104))); // stream_get_position(2)
        body.extend(glk_call(0x44, &[C8(2), C16(0x0108)], Zero)); // stream_close -> result@0x108
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x200, &[0xE0, b'H', b'i', 0]));
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "open_memory returns stream id 2");
        assert_eq!(m.mem.read8(0x180).unwrap(), b'H' as u32);
        assert_eq!(m.mem.read8(0x181).unwrap(), b'i' as u32);
        assert_eq!(m.mem.read32(0x104).unwrap(), 2, "position advanced by 2");
        assert_eq!(m.mem.read32(0x108).unwrap(), 0, "read count");
        assert_eq!(m.mem.read32(0x10C).unwrap(), 2, "write count");
    }

    #[test]
    fn glk_memory_stream_uni_writes_words_and_seeks() {
        use asm::Op::{C16, C8, Mem16, Zero};
        // open_memory_uni -> stream 2 over 4 words at 0x180; write 'A','B'.
        let mut body = glk_call(0x139, &[C16(0x0180), C8(4), C8(1), C8(0)], Zero);
        body.extend(glk_call(0x47, &[C8(2)], Zero)); // set current
        body.extend(glk_call(0x128, &[C8(b'A' as i8)], Zero)); // put_char_uni 'A'
        body.extend(glk_call(0x128, &[C8(b'B' as i8)], Zero)); // put_char_uni 'B'
        body.extend(glk_call(0x46, &[C8(2)], Mem16(0x0100))); // position -> 0x100
        // seek back to word 0 (seekmode 0), overwrite with 'C'.
        body.extend(glk_call(0x45, &[C8(2), C8(0), C8(0)], Zero)); // set_position(2, 0, start)
        body.extend(glk_call(0x128, &[C8(b'C' as i8)], Zero)); // put_char_uni 'C'
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x180).unwrap(), b'C' as u32, "word 0 reseeked + overwritten");
        assert_eq!(m.mem.read32(0x184).unwrap(), b'B' as u32, "word 1 = 'B'");
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "uni position counts words");
    }

    #[test]
    fn memory_stream_seek_end_uses_write_high_water_mark() {
        // SQ-0324: seekmode_End (and the seek clamp) on a memory stream is
        // relative to the write high-water mark (`bufeof`), not the buffer
        // capacity (cheapglk `cgstream.c`).
        let mut m = resource_machine(glk::TestBackend::new(), 0x400);
        // open_memory over a 128-element byte buffer, fmode = Write (hiwater 0).
        let sid = m.glk_dispatch(0x0043, &[0x300, 128, 1, 0]).unwrap();
        for _ in 0..29 {
            m.glk_dispatch(0x0081, &[sid, b'x' as u32]).unwrap(); // put_char_stream
        }
        assert_eq!(m.glk_dispatch(0x0046, &[sid]).unwrap(), 29, "wrote 29 bytes");

        // seekmode_End (2), offset -1 → high-water(29) - 1 = 28, NOT len(128) - 1.
        m.glk_dispatch(0x0045, &[sid, (-1i32) as u32, 2]).unwrap();
        assert_eq!(m.glk_dispatch(0x0046, &[sid]).unwrap(), 28, "End is relative to hiwater, not capacity");

        // seekmode_End, offset 0 → exactly the high-water mark.
        m.glk_dispatch(0x0045, &[sid, 0, 2]).unwrap();
        assert_eq!(m.glk_dispatch(0x0046, &[sid]).unwrap(), 29);

        // A positive End offset clamps to hiwater, not to the buffer capacity.
        m.glk_dispatch(0x0045, &[sid, 50, 2]).unwrap();
        assert_eq!(m.glk_dispatch(0x0046, &[sid]).unwrap(), 29, "clamped to hiwater(29), not len(128)");

        // seekmode_Start past the high-water mark clamps there too.
        m.glk_dispatch(0x0045, &[sid, 100, 0]).unwrap();
        assert_eq!(m.glk_dispatch(0x0046, &[sid]).unwrap(), 29, "Start clamps to hiwater");

        // A Read-mode memory stream treats the whole buffer as content, so
        // seekmode_End lands at len (cheapglk seeds `bufeof = bufend`).
        let rsid = m.glk_dispatch(0x0043, &[0x300, 128, 2, 0]).unwrap(); // fmode = Read
        m.glk_dispatch(0x0045, &[rsid, 0, 2]).unwrap();
        assert_eq!(m.glk_dispatch(0x0046, &[rsid]).unwrap(), 128, "Read-mode End is the full buffer");
    }

    #[test]
    fn glk_stream_current_and_explicit_routing() {
        use asm::Op::{C16, C8, Mem16, Zero};
        // open_memory -> stream 2; current stays the window (stream 1).
        let mut body = glk_call(0x43, &[C16(0x0180), C8(8), C8(1), C8(0)], Mem16(0x0100));
        body.extend(glk_call(0x48, &[], Mem16(0x0104))); // get_current (expect 1, the window)
        body.extend(glk_call(0x81, &[C8(2), C8(b'Z' as i8)], Zero)); // put_char_stream(2,'Z') -> memory
        body.extend(glk_call(0x80, &[C8(b'Y' as i8)], Zero)); // put_char 'Y' -> current window
        body.extend(glk_call(0x47, &[C8(2)], Zero)); // set_current(2)
        body.extend(glk_call(0x48, &[], Mem16(0x0108))); // get_current (expect 2)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "memory stream id");
        assert_eq!(m.mem.read32(0x104).unwrap(), 1, "current stream is the window");
        assert_eq!(m.mem.read8(0x180).unwrap(), b'Z' as u32, "explicit stream routing");
        assert_eq!(backend_of(&m).text(1), "Y", "current routing untouched");
        assert_eq!(m.mem.read32(0x108).unwrap(), 2, "set_current took effect");
    }

    // ── Task 4 (Glk styles, gestalt, output-selector completeness) ────────────

    #[test]
    fn glk_set_style_tags_subsequent_output() {
        use asm::Op::{C16, C8, Zero};
        let mut body = glk_call(0x86, &[C8(3)], Zero); // glk_set_style(Header)
        body.extend(glk_call(0x82, &[C16(0x0200)], Zero)); // glk_put_string("Hi")
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x200, &[0xE0, b'H', b'i', 0]));
        assert_eq!(backend_of(&m).runs(1), vec![(GlkStyle::Header, "Hi".to_string())]);
    }

    #[test]
    fn glk_put_string_decodes_e0_string_object() {
        use asm::Op::{C16, Zero};
        // glk_put_string is handed an Inform string OBJECT (E0 type byte + text),
        // not a raw C array. The E0 tag must be decoded away, not streamed as 'à'.
        let mut body = glk_call(0x82, &[C16(0x0200)], Zero);
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x200, &[0xE0, b'H', b'i', b'!', 0]));
        assert_eq!(backend_of(&m).text(1), "Hi!");
        assert!(m.diagnostics.is_empty(), "diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_put_string_uni_decodes_e2_string_object() {
        use asm::Op::{C16, Zero};
        // glk_put_string_uni is handed an E2 Unicode string object (E2 + 3 pad
        // bytes, then 32-bit code points). The tag must be decoded, not streamed
        // as a leading U+FFFD.
        let mut body = glk_call(0x129, &[C16(0x0200)], Zero);
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            poke(m, 0x200, &[0xE2, 0, 0, 0]);
            poke(m, 0x204, &0x41u32.to_be_bytes()); // 'A'
            poke(m, 0x208, &0x3BBu32.to_be_bytes()); // 'λ'
            poke(m, 0x20C, &0u32.to_be_bytes());
        });
        assert_eq!(backend_of(&m).text(1), "Aλ");
        assert!(m.diagnostics.is_empty(), "diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_put_string_e1_skips_without_halting() {
        use asm::Op::{C16, Zero};
        // A compressed (E1) object whose decode FAILS (here: no string table is
        // set) must NOT fault the VM — it records a diagnostic, emits nothing, and
        // execution continues (regression: an early impl returned Err → Quit).
        let mut body = glk_call(0x82, &[C16(0x0200)], Zero);
        body.extend(glk_call(0x82, &[C16(0x0210)], Zero)); // a following E0 call still runs
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x220, |m| {
            poke(m, 0x200, &[0xE1, 0x02]); // compressed, but no string table → decode fails
            poke(m, 0x210, &[0xE0, b'O', b'k', 0]);
        });
        assert!(m.fault_trace.is_none(), "E1 put_string must not fault the VM");
        assert_eq!(backend_of(&m).text(1), "Ok", "failed E1 skipped, later E0 still printed");
        assert!(
            m.diagnostics.iter().any(|d| d.contains("compressed (E1)")),
            "expected an E1 decode-failed diagnostic, got: {:?}",
            m.diagnostics
        );
    }

    #[test]
    fn glk_put_string_decodes_e1_compressed_object() {
        use asm::Op::{C16, Zero};
        // glk_put_string handed a COMPRESSED (E1) Inform string object decodes it
        // against the current string table and writes the text to the Glk stream
        // (the string machinery's output is captured, then written straight to the
        // stream — bypassing the iosys, like the E0/E2 paths).
        let mut body = asm::ins(0x141, &[C16(0x0100)]); // setstringtbl 0x100
        body.extend(glk_call(0x82, &[C16(0x0140)], Zero)); // glk_put_string(0x140)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            build_hi_table(m, 0x100);
            poke(m, 0x140, &[0xE1, 0x18]); // compressed "Hi": bits 0,0,0,1,1
        });
        assert_eq!(backend_of(&m).text(1), "Hi");
        assert!(m.diagnostics.is_empty(), "diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_set_style_stream_targets_a_specific_stream() {
        use asm::Op::{C8, Zero};
        // window 1's stream is stream 1; tag it Alert, then print to current.
        let mut body = glk_call(0x87, &[C8(1), C8(5)], Zero); // glk_set_style_stream(1, Alert)
        body.extend(glk_call(0x80, &[C8(b'A' as i8)], Zero)); // glk_put_char('A')
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(backend_of(&m).runs(1), vec![(GlkStyle::Alert, "A".to_string())]);
    }

    #[test]
    fn glk_gestalt_reports_output_capabilities() {
        use asm::Op::{C8, Mem16, Zero};
        let mut body = glk_call(0x04, &[C8(0), C8(0)], Mem16(0x0100)); // Version
        body.extend(glk_call(0x04, &[C8(3), C8(b'A' as i8)], Mem16(0x0104))); // CharOutput 'A'
        body.extend(glk_call(0x04, &[C8(15), C8(0)], Mem16(0x0108))); // Unicode
        body.extend(glk_call(0x04, &[C8(2), C8(0)], Mem16(0x010C))); // LineInput (3a-2)
        body.extend(glk_call(0x05, &[C8(0), C8(0), Zero, C8(0)], Mem16(0x0110))); // gestalt_ext Version
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0000_0706, "glk version 0.7.6");
        assert_eq!(m.mem.read32(0x104).unwrap(), 2, "CharOutput = ExactPrint");
        assert_eq!(m.mem.read32(0x108).unwrap(), 1, "Unicode supported");
        assert_eq!(m.mem.read32(0x10C).unwrap(), 1, "LineInput supported (3a-2)");
        assert_eq!(m.mem.read32(0x110).unwrap(), 0x0000_0706, "gestalt_ext mirrors gestalt");
    }

    #[test]
    fn glk_stylehint_calls_are_accepted_silently() {
        use asm::Op::{C8, Zero};
        let mut body = glk_call(0xB0, &[C8(3), C8(3), C8(0), C8(1)], Zero); // stylehint_set
        body.extend(glk_call(0xB1, &[C8(3), C8(3), C8(0)], Zero)); // stylehint_clear
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert!(m.diagnostics.is_empty(), "stylehints accepted: {:?}", m.diagnostics);
    }

    // A Weight/Oblique stylehint flows to the backend's per-run rendered
    // attributes (SQ-0317); clearing it restores the class default (no hint).
    #[test]
    fn glk_stylehint_weight_flows_to_run_attrs_and_clears() {
        use asm::Op::{C16, C8, Zero};
        // stylehint_set(TextBuffer=3, Normal=0, Weight=4, 1); put_string("Hi");
        // stylehint_clear(3, 0, 4); put_string("Hi") again.
        let mut body = glk_call(0xB0, &[C8(3), C8(0), C8(4), C8(1)], Zero);
        body.extend(glk_call(0x82, &[C16(0x0200)], Zero));
        body.extend(glk_call(0xB1, &[C8(3), C8(0), C8(4)], Zero));
        body.extend(glk_call(0x82, &[C16(0x0200)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x200, &[0xE0, b'H', b'i', 0]));
        let runs = backend_of(&m).attr_runs(1);
        assert_eq!(runs.len(), 2);
        assert_eq!(
            runs[0].1,
            crate::glk::StyleAttrs { weight: Some(1), ..Default::default() },
            "weight hint carried on the run"
        );
        assert_eq!(runs[1].1, crate::glk::StyleAttrs::default(), "cleared hint → class default");
    }

    // glk_gestalt(gestalt_GarglkText=0x1100) reports the garglk_* colour
    // extensions as supported, so games that gate on it actually call them.
    #[test]
    fn glk_garglk_text_gestalt_reports_supported() {
        use asm::Op::{C32, C8, Mem16};
        let mut body = glk_call(0x0004, &[C32(0x1100), C8(0)], Mem16(0x0100)); // glk_gestalt(0x1100, 0)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "gestalt_GarglkText -> supported");
    }

    // garglk_set_zcolors(fg, bg) on the current stream colours subsequent output.
    #[test]
    fn glk_garglk_set_zcolors_colours_subsequent_output() {
        use asm::Op::{C16, C32, Zero};
        let mut body = glk_call(0x1100, &[C32(0x00AA_BBCC), C32(0x0011_2233)], Zero); // garglk_set_zcolors
        body.extend(glk_call(0x82, &[C16(0x0200)], Zero)); // glk_put_string("Hi")
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x200, &[0xE0, b'H', b'i', 0]));
        assert!(m.diagnostics.is_empty(), "garglk_set_zcolors handled: {:?}", m.diagnostics);
        assert_eq!(
            backend_of(&m).colour_runs(1),
            vec![(
                GlkStyle::Normal,
                crate::glk::StyleColour { fg: Some(0x00AA_BBCC), bg: Some(0x0011_2233), reverse: false },
                "Hi".to_string(),
            )],
        );
    }

    // garglk_set_reversevideo(1) on the current stream reverse-flags output.
    #[test]
    fn glk_garglk_set_reversevideo_marks_output_reverse() {
        use asm::Op::{C16, C8, Zero};
        let mut body = glk_call(0x1102, &[C8(1)], Zero); // garglk_set_reversevideo(1)
        body.extend(glk_call(0x82, &[C16(0x0200)], Zero)); // glk_put_string("Hi")
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x200, &[0xE0, b'H', b'i', 0]));
        assert!(m.diagnostics.is_empty(), "garglk_set_reversevideo handled: {:?}", m.diagnostics);
        let runs = backend_of(&m).colour_runs(1);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].1.reverse, "output run is reverse-video");
    }

    #[test]
    fn glk_exit_halts_the_machine() {
        use asm::Op::{C8, Zero};
        let mut body = glk_call(0x01, &[], Zero); // glk_exit
        body.extend(glk_call(0x80, &[C8(b'X' as i8)], Zero)); // poison: must not run
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert!(m.halted, "glk_exit halted the machine");
        assert_eq!(backend_of(&m).text(1), "", "nothing printed after glk_exit");
    }

    // ── Task 1 (3a-2): input requests + glk_select suspend/resume ─────────────

    /// Build (but do not run) a start function over `body` with `ram_bytes` of
    /// RAM, with the Glk prelude window already current.
    fn machine_ram(body: Vec<u8>, ram_bytes: u32) -> Machine {
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, ram_bytes);
        machine(built)
    }

    /// Step until the machine suspends for input or quits.
    fn step_to_event(m: &mut Machine) -> StepResult {
        loop {
            match m.step() {
                StepResult::Continue => {}
                other => return other,
            }
        }
    }

    /// Read a 4-word Glk event struct `(type, win, val1, val2)` at `addr`.
    fn read_event(m: &Machine, addr: u32) -> (u32, u32, u32, u32) {
        (
            m.mem.read32(addr).unwrap(),
            m.mem.read32(addr + 4).unwrap(),
            m.mem.read32(addr + 8).unwrap(),
            m.mem.read32(addr + 12).unwrap(),
        )
    }

    #[test]
    fn glk_line_input_suspends_resumes_and_writes_event() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event(win=1, buf=0x180, maxlen=10, initlen=0); select(@0x100).
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // glk_select(event @0x100)
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "select suspends");
        m.supply_line("north");
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "resumes and quits");

        // Buffer holds the Latin-1 line; event = LineInput, win 1, val1 = 5 chars.
        let buf: String = (0..5).map(|i| m.mem.read8(0x180 + i).unwrap() as u8 as char).collect();
        assert_eq!(buf, "north");
        assert_eq!(read_event(&m, 0x100), (3, 1, 5, 0), "evtype_LineInput, win, count");
    }

    #[test]
    fn line_input_echoes_line_and_newline_to_echo_stream() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event(win=1, buf=0x180, maxlen=10); select(@0x100).
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x400);
        // Attach a Latin-1 memory echo stream at 0x300 to window 1.
        let echo = m.glk.stream_open_memory(0x300, 64, false, 1, 0);
        m.glk.window_set_echo_stream(1, echo);
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        m.supply_line("look");
        step_to_event(&mut m);
        // The echo stream recorded the input line plus a newline (Glk §3.6).
        let echoed: String = (0..5).map(|i| m.mem.read8(0x300 + i).unwrap() as u8 as char).collect();
        assert_eq!(echoed, "look\n", "echo stream records the input line + newline");
    }

    #[test]
    fn line_input_uni_echoes_to_echo_stream() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event_uni(win=1, buf=0x180, maxlen=8); select(@0x100).
        let mut body = glk_call(0x141, &[C8(1), C16(0x0180), C8(8), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x400);
        // A Unicode (32-bit element) memory echo stream at 0x300.
        let echo = m.glk.stream_open_memory(0x300, 32, true, 1, 0);
        m.glk.window_set_echo_stream(1, echo);
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        m.supply_line("AB");
        step_to_event(&mut m);
        assert_eq!(m.mem.read32(0x300).unwrap(), b'A' as u32, "echo word 0 = 'A'");
        assert_eq!(m.mem.read32(0x304).unwrap(), b'B' as u32, "echo word 1 = 'B'");
        assert_eq!(m.mem.read32(0x308).unwrap(), '\n' as u32, "echo trailing newline");
    }

    #[test]
    fn line_input_without_echo_stream_records_nothing() {
        use asm::Op::{C16, C8, Zero};
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x400);
        // No echo stream set: 0x300 stays zero after input.
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        m.supply_line("look");
        step_to_event(&mut m);
        assert_eq!(m.mem.read8(0x300).unwrap(), 0, "no echo stream → nothing recorded");
    }

    #[test]
    fn glk_line_input_truncates_to_maxlen() {
        use asm::Op::{C16, C8, Zero};
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(3), C8(0)], Zero); // maxlen 3
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        m.supply_line("verbose");
        step_to_event(&mut m);
        let buf: String = (0..3).map(|i| m.mem.read8(0x180 + i).unwrap() as u8 as char).collect();
        assert_eq!(buf, "ver", "truncated to maxlen");
        assert_eq!(read_event(&m, 0x100).2, 3, "val1 = chars actually stored");
    }

    #[test]
    fn glk_line_input_uni_writes_words() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event_uni(0x0141): 32-bit elements at 0x180.
        let mut body = glk_call(0x141, &[C8(1), C16(0x0180), C8(8), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        m.supply_line("AB");
        step_to_event(&mut m);
        assert_eq!(m.mem.read32(0x180).unwrap(), b'A' as u32, "word 0 = 'A'");
        assert_eq!(m.mem.read32(0x184).unwrap(), b'B' as u32, "word 1 = 'B'");
        assert_eq!(read_event(&m, 0x100), (3, 1, 2, 0));
    }

    #[test]
    fn glk_gestalt_reports_line_terminator_support() {
        use crate::glk::keycode;
        let m = machine_with_body(&[], vec![]);
        // gestalt_LineInputEcho (17): echo control is supported.
        assert_eq!(m.glk_gestalt(17, 0), 1, "gestalt_LineInputEcho supported");
        // gestalt_LineTerminators (18): the call is supported.
        assert_eq!(m.glk_gestalt(18, 0), 1, "gestalt_LineTerminators supported");
        // gestalt_LineTerminatorKey (19): TRUE only for Escape + the function keys.
        assert_eq!(m.glk_gestalt(19, keycode::FUNC1), 1, "Func1 is a valid terminator");
        assert_eq!(m.glk_gestalt(19, keycode::FUNC12), 1, "Func12 is a valid terminator");
        assert_eq!(m.glk_gestalt(19, keycode::ESCAPE), 1, "Escape is a valid terminator");
        assert_eq!(m.glk_gestalt(19, keycode::RETURN), 0, "Return is never a terminator");
        assert_eq!(m.glk_gestalt(19, keycode::LEFT), 0, "arrow keys are not terminators");
        assert_eq!(m.glk_gestalt(19, b'a' as u32), 0, "a printable char is not a terminator");
    }

    #[test]
    fn glk_tick_is_a_silent_no_op() {
        let mut m = machine_with_body(&[], vec![]);
        // glk_tick returns 0 and, being an explicit arm, records no diagnostic —
        // unlike the unknown-selector default which would log on every call.
        assert_eq!(m.glk_dispatch(0x0003, &[]).unwrap(), 0);
        assert!(m.diagnostics.is_empty(), "glk_tick must not touch the diagnostics log");
    }

    #[test]
    fn glk_set_interrupt_handler_is_a_silent_no_op() {
        let mut m = machine_with_body(&[], vec![]);
        // Unusable from Glulx, so a no-op — but an explicit arm, not the warning default.
        assert_eq!(m.glk_dispatch(0x0002, &[0x1234]).unwrap(), 0);
        assert!(m.diagnostics.is_empty(), "set_interrupt_handler must not warn");
    }

    #[test]
    fn gestalt_mouse_and_hyperlink_report_per_window_type() {
        let m = machine_with_body(&[], vec![]);
        // gestalt_MouseInput (4): grid (4) + graphics (5), plus the generic (0) probe.
        assert_eq!(m.glk_gestalt(4, 4), 1, "grid supports mouse");
        assert_eq!(m.glk_gestalt(4, 5), 1, "graphics supports mouse");
        assert_eq!(m.glk_gestalt(4, 0), 1, "generic mouse probe supported");
        assert_eq!(m.glk_gestalt(4, 3), 0, "text buffer does not support mouse");
        assert_eq!(m.glk_gestalt(4, 1), 0, "pair windows do not support mouse");
        // gestalt_HyperlinkInput (12): text windows (buffer 3 + grid 4), plus generic (0).
        assert_eq!(m.glk_gestalt(12, 3), 1, "buffer supports hyperlinks");
        assert_eq!(m.glk_gestalt(12, 4), 1, "grid supports hyperlinks");
        assert_eq!(m.glk_gestalt(12, 0), 1, "generic hyperlink probe supported");
        assert_eq!(m.glk_gestalt(12, 5), 0, "graphics does not support hyperlinks");
    }

    #[test]
    fn glk_gestalt_ext_fills_charoutput_glyph_count() {
        let mut m = machine_with_body(&[], vec![]);
        // gestalt_CharOutput (3) for 'A' with a 1-element output array: returns
        // ExactPrint (2) and writes arr[0] = 1 (one glyph).
        let r = m.glk_dispatch(0x0005, &[3, b'A' as u32, 0x0110, 1]).unwrap();
        assert_eq!(r, 2, "ExactPrint");
        assert_eq!(m.mem.read32(0x0110).unwrap(), 1, "one glyph written to arr[0]");
        // A non-CharOutput selector still returns its scalar but leaves the array alone.
        m.mem.write32(0x0110, 0x0000_DEAD).unwrap();
        assert_eq!(m.glk_dispatch(0x0005, &[0, 0, 0x0110, 1]).unwrap(), 0x0000_0706); // Version
        assert_eq!(m.mem.read32(0x0110).unwrap(), 0x0000_DEAD, "version query leaves the array untouched");
    }

    #[test]
    fn glk_line_terminator_delivered_in_val2() {
        use asm::Op::{C16, C8, Zero};
        use crate::glk::keycode;
        // set_terminators_line_event(win=1, keycodes=@0x0190, count=1) with Func1,
        // then request_line_event + select.
        let mut body = glk_call(0x151, &[C8(1), C16(0x0190), C8(1)], Zero);
        body.extend(glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero));
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        m.mem.write32(0x190, keycode::FUNC1).unwrap(); // the registered terminator key

        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        m.supply_line_terminated("look", keycode::FUNC1);
        assert_eq!(step_to_event(&mut m), StepResult::Quit);
        // val1 = char count; val2 = the terminator keycode that ended input.
        assert_eq!(read_event(&m, 0x100), (3, 1, 4, keycode::FUNC1));
    }

    #[test]
    fn glk_line_terminator_val2_zero_for_unregistered_key() {
        use asm::Op::{C16, C8, Zero};
        use crate::glk::keycode;
        // Register Func1, but end the line with Func2 — a valid terminator keycode
        // the game did NOT register. Per spec, val2 reports only registered keys.
        let mut body = glk_call(0x151, &[C8(1), C16(0x0190), C8(1)], Zero);
        body.extend(glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero));
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        m.mem.write32(0x190, keycode::FUNC1).unwrap();

        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        m.supply_line_terminated("look", keycode::FUNC1 - 1); // Func2, not registered
        assert_eq!(step_to_event(&mut m), StepResult::Quit);
        assert_eq!(read_event(&m, 0x100).3, 0, "unregistered terminator -> val2 = 0");
    }

    #[test]
    fn glk_char_input_suspends_resumes_and_writes_event() {
        use asm::Op::{C16, C8, Zero};
        // request_char_event(0x00D2, win=1); select.
        let mut body = glk_call(0xD2, &[C8(1)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::NeedChar { win: 1, unicode: false }, "char suspend");
        m.supply_char(b'Z' as u32);
        assert_eq!(step_to_event(&mut m), StepResult::Quit);
        assert_eq!(read_event(&m, 0x100), (2, 1, b'Z' as u32, 0), "evtype_CharInput, win, key");
    }

    #[test]
    fn glk_char_input_non_uni_maps_special_and_unknown() {
        use asm::Op::{C16, C8, Zero};
        // Two char requests back-to-back: deliver a special key, then a high
        // Unicode code point (which a non-Unicode request cannot represent).
        let mut body = glk_call(0xD2, &[C8(1)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(glk_call(0xD2, &[C8(1)], Zero));
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::NeedChar { win: 1, unicode: false });
        m.supply_char(crate::glk::keycode::LEFT); // a special keycode passes through
        assert_eq!(step_to_event(&mut m), StepResult::NeedChar { win: 1, unicode: false });
        m.supply_char(0x1F600); // emoji → not Latin-1, non-uni request → Unknown
        step_to_event(&mut m);
        assert_eq!(read_event(&m, 0x100).2, crate::glk::keycode::LEFT, "special key preserved");
        assert_eq!(read_event(&m, 0x110).2, crate::glk::keycode::UNKNOWN, "non-latin1 → Unknown");
    }

    #[test]
    fn glk_char_input_uni_passes_full_codepoint() {
        use asm::Op::{C16, C8, Zero};
        // request_char_event_uni(0x0140).
        let mut body = glk_call(0x140, &[C8(1)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::NeedChar { win: 1, unicode: true });
        m.supply_char(0x1F600); // a Unicode request preserves the full code point
        step_to_event(&mut m);
        assert_eq!(read_event(&m, 0x100), (2, 1, 0x1F600, 0));
    }

    #[test]
    fn glk_select_with_no_request_is_safe() {
        use asm::Op::{C16, Zero};
        // select with nothing requested: deliver evtype_None, diagnostic, continue.
        let mut body = glk_call(0xC0, &[C16(0x0100)], Zero);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "no suspend without a request");
        assert_eq!(read_event(&m, 0x100).0, 0, "evtype_None written");
        assert!(!m.diagnostics.is_empty(), "diagnostic recorded");
    }

    #[test]
    fn supply_without_pending_request_is_safe() {
        let mut m = machine_ram(asm::ins(0x120, &[]), 0x200);
        m.supply_line("ignored"); // no panic, no effect
        m.supply_char(b'x' as u32);
        assert!(!m.diagnostics.is_empty(), "diagnostics noted the stray supply");
    }

    // ── Task 2 (3a-2): cancel, arrange, select_poll, gestalt, timer/mouse ─────

    #[test]
    fn glk_cancel_line_event_reports_partial_and_clears() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event(win=1, buf=0x180, maxlen=10, initlen=2), then cancel.
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(2)], Zero);
        body.extend(glk_call(0xD1, &[C8(1), C16(0x0100)], Zero)); // glk_cancel_line_event
        // A following select has nothing to wait for → evtype_None (request gone).
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "cancel does not suspend");
        assert_eq!(read_event(&m, 0x100), (3, 1, 2, 0), "LineInput, win, initlen chars");
        assert_eq!(read_event(&m, 0x110).0, 0, "request was cleared → None");
    }

    // ── Pre-filled line input (Glk §4.2 `initlen`) ────────────────────────────

    /// A game may pre-load the input buffer and pass a non-zero `initlen`, meaning
    /// those characters are ALREADY in the input line. The host takes that text to
    /// seed its own editable prompt (advent.blb's toolbar primes verbs this way:
    /// clicking Examine asks for a line with `"Examine "` waiting). (SQ-0562)
    #[test]
    fn line_prefill_is_read_from_the_buffer_and_taken_once() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event(win=1, buf=0x180, maxlen=10, initlen=8) over "Examine ".
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(8)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        let mut m = machine_ram(body, 0x200);
        poke(&mut m, 0x180, b"Examine ");
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        assert_eq!(m.take_line_seed().as_deref(), Some("Examine "));
        assert_eq!(m.take_line_seed(), None, "one-shot: a second poll must not re-seed");
    }

    /// The prefill is capped at `maxlen` (the model clamps `initlen` the same way),
    /// so a game over-declaring its prefill cannot make the host read past the
    /// buffer it offered.
    #[test]
    fn line_prefill_is_capped_at_maxlen() {
        use asm::Op::{C16, C8, Zero};
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(4), C8(8)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        let mut m = machine_ram(body, 0x200);
        poke(&mut m, 0x180, b"Examine ");
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        assert_eq!(m.take_line_seed().as_deref(), Some("Exam"));
    }

    /// A `_uni` request pre-loads 32-bit code points, not bytes.
    #[test]
    fn line_prefill_reads_unicode_code_points() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event_uni(win=1, buf=0x180, maxlen=10, initlen=2)
        let mut body = glk_call(0x141, &[C8(1), C16(0x0180), C8(10), C8(2)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        let mut m = machine_ram(body, 0x200);
        m.mem.write32(0x180, 'é' as u32).unwrap();
        m.mem.write32(0x184, '—' as u32).unwrap();
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        assert_eq!(m.take_line_seed().as_deref(), Some("é—"));
    }

    /// The overwhelmingly common request (`initlen` 0) leaves nothing to seed, so
    /// an ordinary prompt never gains stray text.
    #[test]
    fn no_prefill_for_an_ordinary_line_request() {
        use asm::Op::{C16, C8, Zero};
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        let mut m = machine_ram(body, 0x200);
        poke(&mut m, 0x180, b"stale bytes");
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        assert_eq!(
            m.take_line_seed().as_deref(),
            Some(""),
            "initlen 0 → seed the host EMPTY (the stale bytes stay out, and any line the host \
             still shows is cleared), not `None`, which would mean no new request at all",
        );
    }

    /// Committing the line writes from index 0, so a host that submits the whole
    /// line (prefill + what the player appended) does not double the verb.
    #[test]
    fn supplying_the_seeded_line_replaces_the_prefill_in_the_buffer() {
        use asm::Op::{C16, C8, Zero};
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(20), C8(8)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        poke(&mut m, 0x180, b"Examine ");
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        m.supply_line("Examine lamp");
        step_to_event(&mut m);
        assert_eq!(read_event(&m, 0x100), (3, 1, 12, 0), "LineInput, win, 12 chars");
        let got: String =
            (0..12).map(|i| m.mem.read8(0x180 + i).unwrap() as u8 as char).collect();
        assert_eq!(got, "Examine lamp", "written from index 0 — no doubled verb");
    }

    /// SQ-0565: Glk lends the interpreter the line-input buffer for the whole
    /// request, which is what lets `glk_cancel_line_event` report how much has been
    /// typed. The host mirrors the player's in-progress line into it, so a game that
    /// cancels mid-edit reads the real partial input instead of whatever the buffer
    /// last held.
    #[test]
    fn mirroring_the_host_line_replaces_what_a_cancel_would_report() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event(win=1, buf=0x180, maxlen=20, initlen=8) over "Examine " —
        // the state advent.blb's toolbar leaves behind after priming a verb.
        let body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(20), C8(8)], Zero);
        let mut m = machine_ram(body, 0x200);
        poke(&mut m, 0x180, b"Examine ");
        step_to_event(&mut m);
        assert_eq!(m.take_line_seed().as_deref(), Some("Examine "), "seeded from the game");

        // The player erases it and types something else. `glk_cancel_line_event`
        // reports the request's `initlen` and leaves the buffer for the game to read
        // (pinned by `glk_cancel_line_event_reports_partial_and_clears`), so BOTH
        // must now describe the real line — otherwise the game reads back text the
        // player deleted, which is how every toolbar verb kept re-inserting the
        // first one.
        m.set_line_input_text("go west");
        assert_eq!(
            m.glk.line_request(1).map(|r| r.initlen),
            Some(7),
            "the count a cancel would report is the player's line, not the prefill's 8"
        );
        let got: String = (0..7).map(|i| m.mem.read8(0x180 + i).unwrap() as u8 as char).collect();
        assert_eq!(got, "go west", "and the buffer holds it, not the stale prefill");

        // Shrinking it must shrink the report too — the tail of the old text stays
        // in memory but is no longer claimed as input.
        m.set_line_input_text("go");
        assert_eq!(m.glk.line_request(1).map(|r| r.initlen), Some(2), "deleting shortens it");
    }

    /// Mirroring is bounded by the buffer the game offered and is a no-op when the
    /// game is not awaiting a line at all.
    #[test]
    fn mirroring_the_host_line_respects_maxlen_and_needs_a_request() {
        use asm::Op::{C16, C8, Zero};
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(4), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 });
        m.set_line_input_text("far too long");
        let got: String = (0..4).map(|i| m.mem.read8(0x180 + i).unwrap() as u8 as char).collect();
        assert_eq!(got, "far ", "truncated to maxlen");
        assert_eq!(m.glk.line_request(1).map(|r| r.initlen), Some(4), "and reported as 4");

        // With no request pending, mirroring writes nothing anywhere.
        let mut m2 = machine_ram(asm::ins(0x120, &[]), 0x200);
        m2.set_line_input_text("ignored");
        assert!(m2.diagnostics.is_empty(), "a no-op, not a diagnosed fault");
    }

    #[test]
    fn glk_cancel_char_event_clears_request() {
        use asm::Op::{C16, C8, Zero};
        let mut body = glk_call(0xD2, &[C8(1)], Zero); // request_char_event(1)
        body.extend(glk_call(0xD3, &[C8(1)], Zero)); // glk_cancel_char_event(1)
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → None
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "cancelled char does not suspend");
        assert_eq!(read_event(&m, 0x100).0, 0, "no pending request after cancel");
    }

    #[test]
    fn glk_arrange_event_delivered_before_input() {
        use asm::Op::{C16, C8, Zero};
        // Open a TextGrid split (grid=2, pair=3), request a char, then select
        // twice. A host-queued Arrange (a real terminal resize via notify_resize)
        // is delivered first, then the select suspends on the char.
        let mut body = glk_call(0x23, &[C8(1), C8(0x12), C8(3), C8(4), C8(0)], Zero);
        body.extend(glk_call(0xD2, &[C8(1)], Zero)); // request_char_event(1)
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → Arrange
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero)); // select → suspend (char)
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        m.notify_resize(); // a real terminal resize queues an Arrange event
        assert_eq!(step_to_event(&mut m), StepResult::NeedChar { win: 1, unicode: false });
        assert_eq!(read_event(&m, 0x100), (5, 0, 0, 0), "evtype_Arrange delivered first");
        m.supply_char(b'q' as u32);
        step_to_event(&mut m);
        assert_eq!(read_event(&m, 0x110), (2, 1, b'q' as u32, 0), "then the char event");
    }

    #[test]
    fn deliver_arrange_interrupts_suspended_select_without_consuming_request() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event(win=1, buf=0x180, maxlen=10, initlen=0), then select
        // twice. The first select suspends on the line request; the host injects
        // an Arrange into it. The game loops and selects again, re-suspending on
        // the SAME still-pending request, which the real line input then resolves.
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → suspend @0x100
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero)); // select again → re-suspend @0x110
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "first select suspends");
        m.deliver_arrange();
        assert_eq!(read_event(&m, 0x100), (5, 0, 0, 0), "evtype_Arrange written to the suspended select");

        // The line request was NOT consumed: the next select re-suspends on it.
        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedLine { win: 1 },
            "re-suspends; the line request persisted across the Arrange"
        );
        m.supply_line("north");
        assert_eq!(step_to_event(&mut m), StepResult::Quit);
        assert_eq!(read_event(&m, 0x110), (3, 1, 5, 0), "the real line event on the second select");
    }

    #[test]
    fn deliver_sound_notify_writes_into_a_suspended_select() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event then select: the select suspends on the line request;
        // a sound-notify is written into it WITHOUT consuming the line request, so
        // the next select re-suspends on the still-pending read.
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → suspend @0x100
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero)); // select again → re-suspend @0x110
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "first select suspends");
        m.deliver_sound_notify(6, 42);
        // evtype_SoundNotify = 7, win = 0, val1 = sound (6), val2 = notify (42).
        assert_eq!(read_event(&m, 0x100), (7, 0, 6, 42), "sound-notify written to the suspended select");
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "line request persisted across the notify");
    }

    #[test]
    fn deliver_sound_notify_queues_when_not_suspended() {
        use asm::Op::{C16, Zero};
        // With nothing waiting, a notify is queued and delivered by the NEXT select.
        let body = {
            let mut b = glk_call(0xC0, &[C16(0x0100)], Zero); // select → drains the queued event
            b.extend(asm::ins(0x120, &[]));
            b
        };
        let mut m = machine_ram(body, 0x200);
        m.deliver_sound_notify(3, 99); // not suspended yet → queue
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "select consumes the queued event and runs to quit");
        assert_eq!(read_event(&m, 0x100), (7, 0, 3, 99), "queued sound-notify delivered by the select");
    }

    #[test]
    fn glk_timer_interval_set_and_cancel() {
        use asm::Op::{C16, Zero};
        // request_timer_events(100) arms the interval; the host reads it back.
        let mut body = glk_call(0xD6, &[C16(100)], Zero);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::Quit);
        assert_eq!(m.glk_timer_interval(), Some(100), "timer armed at 100ms");
        assert!(m.diagnostics.is_empty(), "no diagnostic: {:?}", m.diagnostics);

        // request_timer_events(0) cancels it.
        let mut body = glk_call(0xD6, &[C16(100)], Zero);
        body.extend(glk_call(0xD6, &[asm::Op::C8(0)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        assert_eq!(step_to_event(&mut m), StepResult::Quit);
        assert_eq!(m.glk_timer_interval(), None, "0ms cancels the timer");
    }

    #[test]
    fn deliver_timer_into_suspended_select() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event then select: the select suspends on the line request;
        // a timer tick is written into it WITHOUT consuming the line request, so
        // the next select re-suspends on the still-pending read.
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → suspend @0x100
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero)); // select again → re-suspend @0x110
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "first select suspends");
        m.deliver_timer();
        // evtype_Timer = 1, win = 0, val1 = 0, val2 = 0.
        assert_eq!(read_event(&m, 0x100), (1, 0, 0, 0), "timer written to the suspended select");
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "line request persisted across the timer");
    }

    #[test]
    fn deliver_timer_queues_when_not_suspended() {
        use asm::Op::{C16, Zero};
        // With nothing waiting, a timer tick is queued and delivered by the NEXT select.
        let body = {
            let mut b = glk_call(0xC0, &[C16(0x0100)], Zero); // select → drains the queued event
            b.extend(asm::ins(0x120, &[]));
            b
        };
        let mut m = machine_ram(body, 0x200);
        m.deliver_timer(); // not suspended yet → queue
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "select consumes the queued timer and runs to quit");
        assert_eq!(read_event(&m, 0x100), (1, 0, 0, 0), "queued timer delivered by the select");
    }

    #[test]
    fn glk_gestalt_timer_supported() {
        use asm::Op::{C8, Mem16};
        let body = glk_call(0x04, &[C8(5), C8(0)], Mem16(0x0100)); // gestalt_Timer
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "gestalt_Timer supported");
    }

    // ── SQ-0299: glk_select on a non-input event (timer/mouse/hyperlink) ──────

    #[test]
    fn glk_select_timer_only_suspends_as_needevent() {
        use asm::Op::{C16, Zero};
        // request_timer_events(130) then select with NO line/char request: a legal
        // blocking select on a timer (Glk §4.4). Must suspend as NeedEvent, never
        // fall through to the evtype_None spin.
        let mut body = glk_call(0xD6, &[C16(130)], Zero); // request_timer_events(130)
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select @0x100
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedEvent { timer_ms: Some(130), mouse: false, hyperlink: false },
            "timer-only select suspends as NeedEvent"
        );
        assert!(m.diagnostics.is_empty(), "no evtype_None diagnostic: {:?}", m.diagnostics);
        // The host's clock delivers a tick, resolving the suspended select.
        m.deliver_timer();
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "resumes and quits");
        assert_eq!(read_event(&m, 0x100), (1, 0, 0, 0), "evtype_Timer written");
    }

    #[test]
    fn glk_select_mouse_only_suspends_as_needevent() {
        use asm::Op::{C16, C8, Zero};
        // request_mouse_event(win=1) then select with no line/char request →
        // NeedEvent{mouse}; a delivered click resolves it.
        let mut body = glk_call(0xD4, &[C8(1)], Zero); // request_mouse_event(1)
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select @0x100
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedEvent { timer_ms: None, mouse: true, hyperlink: false },
            "mouse-only select suspends as NeedEvent"
        );
        assert!(m.diagnostics.is_empty(), "no diagnostic: {:?}", m.diagnostics);
        m.deliver_mouse(1, 3, 4);
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "resumes and quits");
        // evtype_MouseInput = 4, win 1, val1 = x (3), val2 = y (4).
        assert_eq!(read_event(&m, 0x100), (4, 1, 3, 4), "mouse event written");
    }

    #[test]
    fn glk_select_hyperlink_only_suspends_as_needevent() {
        use asm::Op::{C16, C8, Zero};
        // request_hyperlink_event(win=1) then select with no line/char request →
        // NeedEvent{hyperlink}; a delivered link click resolves it.
        let mut body = glk_call(0x102, &[C8(1)], Zero); // request_hyperlink_event(1)
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select @0x100
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedEvent { timer_ms: None, mouse: false, hyperlink: true },
            "hyperlink-only select suspends as NeedEvent"
        );
        assert!(m.diagnostics.is_empty(), "no diagnostic: {:?}", m.diagnostics);
        m.deliver_hyperlink(1, 77);
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "resumes and quits");
        // evtype_Hyperlink = 8, win 1, val1 = linkval (77).
        assert_eq!(read_event(&m, 0x100), (8, 1, 77, 0), "hyperlink event written");
    }

    // ── SQ-0302: any deliverable event wakes a pending_event-suspended select ──

    #[test]
    fn deliver_arrange_wakes_a_suspended_pending_event_select() {
        use asm::Op::{C16, C8, Zero};
        // A select suspended on a pending_event (mouse-only wait, no line/char)
        // must be woken by an Arrange too — not just by a timer (SQ-0299 only
        // wired deliver_timer). request_mouse_event(1) then select with no
        // line/char → NeedEvent{mouse}; a terminal resize resolves it.
        let mut body = glk_call(0xD4, &[C8(1)], Zero); // request_mouse_event(1)
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select @0x100
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedEvent { timer_ms: None, mouse: true, hyperlink: false },
            "mouse-only select suspends as NeedEvent"
        );
        m.deliver_arrange();
        // evtype_Arrange = 5, win 0.
        assert_eq!(read_event(&m, 0x100), (5, 0, 0, 0), "Arrange written to the suspended pending_event select");
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "suspension cleared; VM resumes and quits");
    }

    #[test]
    fn deliver_sound_notify_wakes_a_suspended_pending_event_select() {
        use asm::Op::{C16, C8, Zero};
        // A select suspended on a pending_event (hyperlink-only wait) must be
        // woken by a sound-finish notify too. request_hyperlink_event(1) then
        // select with no line/char → NeedEvent{hyperlink}; a sound-notify resolves it.
        let mut body = glk_call(0x102, &[C8(1)], Zero); // request_hyperlink_event(1)
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select @0x100
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedEvent { timer_ms: None, mouse: false, hyperlink: true },
            "hyperlink-only select suspends as NeedEvent"
        );
        m.deliver_sound_notify(6, 42);
        // evtype_SoundNotify = 7, win 0, val1 = sound (6), val2 = notify (42).
        assert_eq!(read_event(&m, 0x100), (7, 0, 6, 42), "sound-notify written to the suspended pending_event select");
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "suspension cleared; VM resumes and quits");
    }

    #[test]
    fn glk_select_with_nothing_armed_still_returns_evtype_none() {
        use asm::Op::{C16, Zero};
        // No line/char AND no timer/mouse/hyperlink: a genuinely malformed program.
        // The evtype_None + diagnostic escape hatch must survive (no deadlock).
        let mut body = glk_call(0xC0, &[C16(0x0100)], Zero); // select @0x100
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(step_to_event(&mut m), StepResult::Quit, "does not suspend; runs to quit");
        assert_eq!(read_event(&m, 0x100), (0, 0, 0, 0), "evtype_None written");
        assert!(
            m.diagnostics.iter().any(|d| d.contains("no pending input request")),
            "the malformed-program diagnostic is still emitted: {:?}",
            m.diagnostics
        );
    }

    #[test]
    fn abort_with_fault_halts_and_records_a_recoverable_fault() {
        // The host watchdog calls this to end a runaway turn. It must halt the VM
        // (next step → Quit) and record a fault trace + diagnostic, exactly like a
        // real runtime fault, so the app's survival path keeps it interactive.
        let mut m = machine_ram(asm::ins(0x00, &[]), 0x200); // a nop program, not run
        m.abort_with_fault("runaway game loop (test)".to_string());
        assert_eq!(m.step(), StepResult::Quit, "halted after abort");
        assert!(m.take_fault_trace().is_some(), "fault trace recorded");
        assert!(
            m.diagnostics.iter().any(|d| d.contains("runaway")),
            "diagnostic explains the abort: {:?}",
            m.diagnostics
        );
    }

    #[test]
    fn deliver_arrange_is_a_noop_when_not_suspended() {
        // With nothing waiting on input, deliver_arrange must not touch memory or
        // push diagnostics — an Arrange is only meaningful at a blocked select.
        let mut m = machine_ram(asm::ins(0x120, &[]), 0x200);
        m.mem.write32(0x0100, 0xDEAD_BEEF).unwrap();
        m.deliver_arrange();
        assert_eq!(m.mem.read32(0x0100).unwrap(), 0xDEAD_BEEF, "memory untouched when not suspended");
        assert!(m.diagnostics.is_empty(), "no diagnostic when there is no pending select");
    }

    #[test]
    fn glk_select_poll_returns_internal_events_not_input() {
        use asm::Op::{C16, C8, Zero};
        // Arrange queued (a real resize) + a char requested. poll returns arrange,
        // then None — never the char, and never suspends.
        let mut body = glk_call(0x23, &[C8(1), C8(0x12), C8(3), C8(4), C8(0)], Zero);
        body.extend(glk_call(0xD2, &[C8(1)], Zero)); // request_char_event(1)
        body.extend(glk_call(0xC1, &[C16(0x0100)], Zero)); // select_poll → Arrange
        body.extend(glk_call(0xC1, &[C16(0x0110)], Zero)); // select_poll → None
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        m.notify_resize(); // a real terminal resize queues an Arrange event
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "poll never suspends");
        assert_eq!(read_event(&m, 0x100).0, 5, "poll returns the Arrange event");
        assert_eq!(read_event(&m, 0x110).0, 0, "poll never returns input → None");
    }

    #[test]
    fn glk_gestalt_reports_input_capabilities() {
        use asm::Op::{C8, Mem16};
        let mut body = glk_call(0x04, &[C8(1), C8(0)], Mem16(0x0100)); // CharInput
        body.extend(glk_call(0x04, &[C8(2), C8(0)], Mem16(0x0104))); // LineInput
        body.extend(glk_call(0x04, &[C8(5), C8(0)], Mem16(0x0108))); // Timer
        body.extend(glk_call(0x04, &[C8(4), C8(0)], Mem16(0x010C))); // MouseInput
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "CharInput supported");
        assert_eq!(m.mem.read32(0x104).unwrap(), 1, "LineInput supported");
        assert_eq!(m.mem.read32(0x108).unwrap(), 1, "Timer supported");
        assert_eq!(m.mem.read32(0x10C).unwrap(), 1, "MouseInput supported");
    }

    #[test]
    fn glk_gestalt_reports_sound_capabilities() {
        // gestalt_Sound(8)/SoundVolume(9)/SoundNotify(10)/Sound2(21) all follow
        // sound_enabled. Sound2 (0.7.3) advertises the extended-suite selectors
        // (create_ext/play_multi/pause/unpause/set_volume_ext).
        let mut m = machine_with_glk(&[]);
        assert_eq!(m.glk_gestalt(8, 0), 0, "Sound off by default");
        assert_eq!(m.glk_gestalt(9, 0), 0);
        assert_eq!(m.glk_gestalt(10, 0), 0);
        assert_eq!(m.glk_gestalt(21, 0), 0, "Sound2 off by default");
        m.set_sound(true);
        assert_eq!(m.glk_gestalt(8, 0), 1, "Sound supported once enabled");
        assert_eq!(m.glk_gestalt(9, 0), 1, "SoundVolume supported");
        assert_eq!(m.glk_gestalt(10, 0), 1, "SoundNotify supported");
        assert_eq!(m.glk_gestalt(21, 0), 1, "Sound2 supported once enabled");
    }

    #[test]
    fn schannel_dispatch_routes_to_backend_when_enabled() {
        use asm::Op::{C16, C32, C8, Mem16};
        // Glk dispatch selector numbers (gi_dispa.c), same iterate/get_rock/create/
        // destroy block layout as window (0x0020) and stream (0x0040):
        //   F0 iterate, F1 get_rock, F2 create, F3 destroy;
        //   F8 play, F9 play_ext, FA stop, FB set_volume, FC sound_load_hint.
        let mut body = glk_call(0xF2, &[C8(7)], Mem16(0x0100)); // create(rock=7)
        body.extend(glk_call(0xF1, &[C8(1)], Mem16(0x0104)));   // get_rock(1)
        body.extend(glk_call(0xF0, &[C8(0), C16(0x0120)], Mem16(0x0108))); // iterate(0,&rock)
        body.extend(glk_call(0xF0, &[C8(1), C16(0x0124)], Mem16(0x010C))); // iterate(1,&rock)
        body.extend(glk_call(0xF8, &[C8(1), C8(5)], Mem16(0x0110))); // play(1, snd=5)
        body.extend(glk_call(0xF9, &[C8(1), C8(6), C8(3), C8(9)], Mem16(0x0114))); // play_ext
        body.extend(glk_call(0xFB, &[C8(1), C32(0x8000)], asm::Op::Zero)); // set_volume
        body.extend(glk_call(0xFA, &[C8(1)], asm::Op::Zero)); // stop
        body.extend(glk_call(0xF3, &[C8(1)], asm::Op::Zero)); // destroy
        body.extend(asm::ins(0x120, &[]));                    // quit
        let m = run_with_ram(body, 0x200, |m| m.set_sound(true));

        assert_eq!(m.mem.read32(0x0100).unwrap(), 1, "create returns the first channel ref");
        assert_eq!(m.mem.read32(0x0104).unwrap(), 7, "get_rock returns the stored rock");
        // iterate(0) yields the first channel and writes its rock; iterate(that
        // channel) yields 0 — the loop MUST terminate (regression: a create/iterate
        // selector swap made iterate hand out endless fresh channels, hanging the game).
        assert_eq!(m.mem.read32(0x0108).unwrap(), 1, "iterate(0) returns the first channel");
        assert_eq!(m.mem.read32(0x0120).unwrap(), 7, "iterate writes the channel's rock to the out-ref");
        assert_eq!(m.mem.read32(0x010C).unwrap(), 0, "iterate past the last channel returns 0 (terminates)");
        assert_eq!(m.mem.read32(0x0110).unwrap(), 1, "play returns success");
        assert_eq!(m.mem.read32(0x0114).unwrap(), 1, "play_ext returns success");

        let log = backend_of(&m).sound_log();
        assert!(log.iter().any(|l| l == "play chan=1 snd=5 repeats=1 notify=0"),
            "plain play forwards repeats=1 notify=0: {log:?}");
        assert!(log.iter().any(|l| l == "play chan=1 snd=6 repeats=3 notify=9"),
            "play_ext threads repeats+notify: {log:?}");
        assert!(log.iter().any(|l| l == "setvol chan=1 vol=32768"), "set_volume forwarded: {log:?}");
        assert!(log.iter().any(|l| l == "stop chan=1"), "stop forwarded: {log:?}");
        assert!(log.iter().any(|l| l == "destroy chan=1"), "destroy forwarded: {log:?}");
        assert!(!m.diagnostics.iter().any(|d| d.contains("unhandled")),
            "no unhandled-selector diagnostic: {:?}", m.diagnostics);
    }

    #[test]
    fn schannel_dispatch_is_inert_when_sound_disabled() {
        use asm::Op::{C8, Mem16};
        // With sound disabled, create (0xF2) returns 0 (NULL channel) and nothing is
        // recorded. (A spec-correct game won't call these — gestalt reports 0 —
        // but a probe must get a safe 0, not a diagnostic-spamming fallthrough.)
        let mut body = glk_call(0xF2, &[C8(7)], Mem16(0x0100)); // schannel_create
        body.extend(glk_call(0xFC, &[C8(5), C8(1)], asm::Op::Zero)); // sound_load_hint
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {}); // sound left disabled
        assert_eq!(m.mem.read32(0x0100).unwrap(), 0, "create returns NULL when sound is off");
        assert!(backend_of(&m).sound_log().is_empty(), "no backend calls when sound is off");
    }

    #[test]
    fn schannel_sound2_dispatch_routes_to_backend() {
        use asm::Op::{C16, C32, C8, Mem16, Zero};
        // Sound2 (0.7.3) selectors (gi_dispa.c): F4 create_ext, F7 play_multi,
        // FD set_volume_ext, FE pause, FF unpause.
        // play_multi reads two same-length arrays from RAM: chanarray=[1,2] at
        // 0x0180, sndarray=[5,6] at 0x0190; notify=9 applies to both sounds.
        let mut body = glk_call(0xF4, &[C8(7), C32(0x8000)], Mem16(0x0100)); // create_ext(rock=7, vol=0x8000)
        body.extend(glk_call(0xF7, &[C16(0x0180), C8(2), C16(0x0190), C8(2), C8(9)], Mem16(0x0104))); // play_multi
        body.extend(glk_call(0xFD, &[C8(1), C32(0x4000), C16(500), C8(3)], Zero)); // set_volume_ext(chan=1, vol=0x4000, 500ms, notify=3)
        body.extend(glk_call(0xFE, &[C8(1)], Zero)); // pause(1)
        body.extend(glk_call(0xFF, &[C8(1)], Zero)); // unpause(1)
        body.extend(asm::ins(0x120, &[]));           // quit
        let m = run_with_ram(body, 0x200, |m| {
            m.set_sound(true);
            poke(m, 0x0180, &1u32.to_be_bytes());
            poke(m, 0x0184, &2u32.to_be_bytes());
            poke(m, 0x0190, &5u32.to_be_bytes());
            poke(m, 0x0194, &6u32.to_be_bytes());
        });

        assert_ne!(m.mem.read32(0x0100).unwrap(), 0, "create_ext returns a channel ref");
        assert_eq!(m.mem.read32(0x0104).unwrap(), 2, "play_multi returns the number of sounds started");

        let log = backend_of(&m).sound_log();
        assert!(log.iter().any(|l| l == "create_ext rock=7 vol=32768 -> 1"),
            "create_ext threads rock + initial volume: {log:?}");
        assert!(log.iter().any(|l| l == "play chan=1 snd=5 repeats=1 notify=9"),
            "play_multi starts the first pair with shared notify + repeats=1: {log:?}");
        assert!(log.iter().any(|l| l == "play chan=2 snd=6 repeats=1 notify=9"),
            "play_multi starts the second pair: {log:?}");
        assert!(log.iter().any(|l| l == "setvol_ext chan=1 vol=16384 dur=500 notify=3"),
            "set_volume_ext threads vol/duration/notify: {log:?}");
        assert!(log.iter().any(|l| l == "pause chan=1"), "pause forwarded: {log:?}");
        assert!(log.iter().any(|l| l == "unpause chan=1"), "unpause forwarded: {log:?}");
        assert!(!m.diagnostics.iter().any(|d| d.contains("unhandled")),
            "no unhandled-selector diagnostic: {:?}", m.diagnostics);
    }

    #[test]
    fn schannel_sound2_dispatch_is_inert_when_sound_disabled() {
        use asm::Op::{C32, C8, Mem16, Zero};
        // With sound off, create_ext (0xF4) returns 0 and pause/set_volume_ext
        // record nothing — a probe gets a safe 0, not a diagnostic fallthrough.
        let mut body = glk_call(0xF4, &[C8(7), C32(0x8000)], Mem16(0x0100)); // create_ext
        body.extend(glk_call(0xFD, &[C8(1), C32(0x4000), C8(0), C8(0)], Zero)); // set_volume_ext
        body.extend(glk_call(0xFE, &[C8(1)], Zero)); // pause
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {}); // sound left disabled
        assert_eq!(m.mem.read32(0x0100).unwrap(), 0, "create_ext returns NULL when sound is off");
        assert!(backend_of(&m).sound_log().is_empty(), "no backend calls when sound is off");
    }

    #[test]
    fn deliver_volume_notify_writes_into_a_suspended_select() {
        use asm::Op::{C16, C8, Zero};
        // request_line_event then select: the select suspends on the line request;
        // a volume-notify is written into it WITHOUT consuming the line request, so
        // the next select re-suspends on the still-pending read.
        let mut body = glk_call(0xD0, &[C8(1), C16(0x0180), C8(10), C8(0)], Zero);
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → suspend @0x100
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero)); // select again → re-suspend @0x110
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "first select suspends");
        m.deliver_volume_notify(42);
        // evtype_VolumeNotify = 9, win = 0, val1 = 0, val2 = notify (42).
        assert_eq!(read_event(&m, 0x100), (9, 0, 0, 42), "volume-notify written to the suspended select");
        assert_eq!(step_to_event(&mut m), StepResult::NeedLine { win: 1 }, "line request persisted across the notify");
    }

    #[test]
    fn deliver_mouse_into_suspended_select_is_one_shot() {
        use asm::Op::{C16, C8, Zero};
        // Arm a mouse watch AND a char request, then select: the select suspends
        // on the char request; a click is written into it WITHOUT consuming the
        // char request, but the mouse request itself is one-shot — a second
        // deliver_mouse with no fresh request is a no-op.
        let mut body = glk_call(0xD4, &[C8(1)], Zero); // request_mouse_event(1)
        body.extend(glk_call(0xD2, &[C8(1)], Zero)); // request_char_event(1) → select suspends on this
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → suspend @0x100
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero)); // select again → re-suspend @0x110
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedChar { win: 1, unicode: false },
            "select suspends on the char request",
        );
        m.deliver_mouse(1, 7, 3);
        // evtype_MouseInput = 4, win = 1, val1 = col (7), val2 = row (3).
        assert_eq!(read_event(&m, 0x100), (4, 1, 7, 3), "click written to the suspended select");
        // One-shot: the request was consumed on delivery, so a second click is dropped.
        m.deliver_mouse(1, 99, 99);
        assert_eq!(read_event(&m, 0x100), (4, 1, 7, 3), "second deliver_mouse is a no-op (request consumed)");
        assert!(!m.glk.mouse_requested(1), "mouse request cleared after delivery");
        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedChar { win: 1, unicode: false },
            "char request persisted across the click",
        );
    }

    #[test]
    fn glk_cancel_mouse_suppresses_delivery() {
        use asm::Op::{C16, C8, Zero};
        // request then cancel the mouse watch: with no armed request, a click is a
        // no-op and never overwrites the suspended select's event slot.
        let mut body = glk_call(0xD4, &[C8(1)], Zero); // request_mouse_event(1)
        body.extend(glk_call(0xD5, &[C8(1)], Zero)); // cancel_mouse_event(1)
        body.extend(glk_call(0xD2, &[C8(1)], Zero)); // request_char_event(1) → suspend
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → suspend @0x100
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        m.mem.write32(0x0100, 0xDEAD_BEEF).unwrap();

        assert_eq!(step_to_event(&mut m), StepResult::NeedChar { win: 1, unicode: false });
        m.deliver_mouse(1, 5, 5); // request cancelled → dropped
        assert_eq!(m.mem.read32(0x100).unwrap(), 0xDEAD_BEEF, "cancelled mouse request suppresses delivery");
    }

    #[test]
    fn deliver_mouse_queues_when_not_suspended() {
        use asm::Op::{C16, Zero};
        // With nothing waiting, a click is queued and delivered by the NEXT select.
        let body = {
            let mut b = glk_call(0xC0, &[C16(0x0100)], Zero); // select → drains the queued event
            b.extend(asm::ins(0x120, &[]));
            b
        };
        let mut m = machine_ram(body, 0x200);
        m.glk.set_mouse_request(1); // window 1 exists via the prelude
        m.deliver_mouse(1, 4, 8); // not suspended yet → queue (and consume the request)
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "select drains the queued click and quits");
        assert_eq!(read_event(&m, 0x100), (4, 1, 4, 8), "queued click delivered by the select");
    }

    // ── Glk hyperlinks (SQ-0257) ──────────────────────────────────────────────

    #[test]
    fn glk_set_hyperlink_tags_subsequent_output() {
        use asm::Op::{C8, Zero};
        // glk_set_hyperlink(42) on the current stream, then print: the emitted run
        // carries link 42 to the backend. Clearing it (0) drops the link again.
        let mut body = glk_call(0x100, &[C8(42)], Zero); // glk_set_hyperlink(42)
        body.extend(glk_call(0x80, &[C8(b'A' as i8)], Zero)); // put_char('A') → linked
        body.extend(glk_call(0x100, &[C8(0)], Zero)); // glk_set_hyperlink(0) → clear
        body.extend(glk_call(0x80, &[C8(b'B' as i8)], Zero)); // put_char('B') → unlinked
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert_eq!(
            backend_of(&m).linked_runs(1),
            vec![
                (GlkStyle::Normal, 42, "A".to_string()),
                (GlkStyle::Normal, 0, "B".to_string()),
            ],
            "the link value rides each output run and clears on set_hyperlink(0)",
        );
    }

    #[test]
    fn deliver_hyperlink_into_suspended_select_is_one_shot() {
        use asm::Op::{C16, C8, Zero};
        // Arm a hyperlink watch AND a char request, then select: the select
        // suspends on the char request; a link click is written into it WITHOUT
        // consuming the char request, but the hyperlink request is one-shot — a
        // second deliver_hyperlink with no fresh request is a no-op.
        let mut body = glk_call(0x102, &[C8(1)], Zero); // request_hyperlink_event(1)
        body.extend(glk_call(0xD2, &[C8(1)], Zero)); // request_char_event(1) → select suspends on this
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → suspend @0x100
        body.extend(glk_call(0xC0, &[C16(0x0110)], Zero)); // select again → re-suspend @0x110
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);

        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedChar { win: 1, unicode: false },
            "select suspends on the char request",
        );
        m.deliver_hyperlink(1, 99);
        // evtype_Hyperlink = 8, win = 1, val1 = linkval (99), val2 = 0.
        assert_eq!(read_event(&m, 0x100), (8, 1, 99, 0), "link click written to the suspended select");
        // One-shot: the request was consumed on delivery, so a second click drops.
        m.deliver_hyperlink(1, 7);
        assert_eq!(read_event(&m, 0x100), (8, 1, 99, 0), "second deliver_hyperlink is a no-op (request consumed)");
        assert!(!m.glk.hyperlink_requested(1), "hyperlink request cleared after delivery");
        assert_eq!(
            step_to_event(&mut m),
            StepResult::NeedChar { win: 1, unicode: false },
            "char request persisted across the click",
        );
    }

    #[test]
    fn glk_cancel_hyperlink_suppresses_delivery() {
        use asm::Op::{C16, C8, Zero};
        // request then cancel the hyperlink watch: with no armed request, a click
        // is a no-op and never overwrites the suspended select's event slot.
        let mut body = glk_call(0x102, &[C8(1)], Zero); // request_hyperlink_event(1)
        body.extend(glk_call(0x103, &[C8(1)], Zero)); // cancel_hyperlink_event(1)
        body.extend(glk_call(0xD2, &[C8(1)], Zero)); // request_char_event(1) → suspend
        body.extend(glk_call(0xC0, &[C16(0x0100)], Zero)); // select → suspend @0x100
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_ram(body, 0x200);
        m.mem.write32(0x0100, 0xDEAD_BEEF).unwrap();

        assert_eq!(step_to_event(&mut m), StepResult::NeedChar { win: 1, unicode: false });
        m.deliver_hyperlink(1, 5); // request cancelled → dropped
        assert_eq!(m.mem.read32(0x100).unwrap(), 0xDEAD_BEEF, "cancelled hyperlink request suppresses delivery");
    }

    #[test]
    fn deliver_hyperlink_queues_when_not_suspended() {
        use asm::Op::{C16, Zero};
        // With nothing waiting, a link click is queued and delivered by the NEXT select.
        let body = {
            let mut b = glk_call(0xC0, &[C16(0x0100)], Zero); // select → drains the queued event
            b.extend(asm::ins(0x120, &[]));
            b
        };
        let mut m = machine_ram(body, 0x200);
        m.glk.set_hyperlink_request(1); // window 1 exists via the prelude
        m.deliver_hyperlink(1, 77); // not suspended yet → queue (and consume the request)
        assert_eq!(step_to_event(&mut m), StepResult::Quit, "select drains the queued click and quits");
        assert_eq!(read_event(&m, 0x100), (8, 1, 77, 0), "queued link click delivered by the select");
    }

    #[test]
    fn glk_gestalt_reports_hyperlink_support() {
        use asm::Op::{C8, Mem16};
        let mut body = glk_call(0x04, &[C8(11), C8(0)], Mem16(0x0100)); // gestalt_Hyperlinks
        body.extend(glk_call(0x04, &[C8(12), C8(3)], Mem16(0x0104))); // gestalt_HyperlinkInput(TextBuffer)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "gestalt_Hyperlinks supported");
        assert_eq!(m.mem.read32(0x104).unwrap(), 1, "gestalt_HyperlinkInput supported");
    }

    #[test]
    fn glk_set_echo_and_terminators_accepted_silently() {
        use asm::Op::{C8, Zero};
        let mut body = glk_call(0x150, &[C8(1), C8(0)], Zero); // set_echo_line_event(1, 0)
        body.extend(glk_call(0x151, &[C8(1), C8(0), C8(0)], Zero)); // set_terminators_line_event
        body.extend(asm::ins(0x120, &[]));
        let m = run_program(body);
        assert!(m.diagnostics.is_empty(), "accepted as best-effort: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_set_echo_line_event_toggles_the_window_flag() {
        use asm::Op::{C8, Zero};
        // A fresh window defaults to echo-on.
        let mut on = glk_call(0x150, &[C8(1), C8(0)], Zero); // set_echo_line_event(win=1, 0)
        on.extend(asm::ins(0x120, &[])); // quit
        let m = run_program(on);
        assert!(!m.window_line_echo(1), "echo turned off for window 1");

        // A window that never set the flag reports the default (true).
        let base = run_program(asm::ins(0x120, &[]));
        assert!(base.window_line_echo(1), "default echo is on");
    }

    // ── Task 4 (glulxercise conformance fixes) ────────────────────────────────

    #[test]
    fn jumpabs_sets_pc_to_the_absolute_target() {
        use asm::Op::{C32, C8, Mem16};
        // jumpabs over a poisoned store; only the post-jump store should run.
        let poison = asm::ins(0x40, &[C8(0x55), Mem16(0x0100)]); // copy 0x55 -> mem[0x100]
        let body_start = 0x27u32; // start func @0x24: C1 + (0,0) header = 3 bytes
        let jumpabs_len = 7u32; // 2-byte opcode + 1 mode + 4-byte C32
        let target = body_start + jumpabs_len + poison.len() as u32;
        let mut body = asm::ins(0x104, &[C32(target)]); // jumpabs target
        body.extend(poison); // skipped
        body.extend(asm::ins(0x40, &[C8(0x42), Mem16(0x0104)])); // copy 0x42 -> mem[0x104]
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 0, "poisoned store was jumped over");
        assert_eq!(m.mem.read32(0x104).unwrap(), 0x42, "landed on the post-jump store");
    }

    #[test]
    fn catch_and_throw_unwind_to_the_handler() {
        use asm::Op::{C16, C8, Local8, Mem16};
        // catch L0, <branch past the handler to the try-block>; the handler (the
        // fall-through after catch) stores the thrown value; the try-block throws.
        let handler = {
            let mut h = asm::ins(0x40, &[Local8(0), Mem16(0x0100)]); // copy L0 -> mem[0x100]
            h.extend(asm::ins(0x120, &[])); // quit
            h
        };
        let tryblock = asm::ins(0x33, &[C8(42), Local8(0)]); // throw 42, token=L0
        // Branch convention: pc = pc_after_operands + offset - 2. The try-block
        // sits right after the handler, so offset = handler.len() + 2.
        let offset = handler.len() as u32 + 2;
        let mut body = asm::ins(0x32, &[Local8(0), C16(offset as i16)]); // catch L0, offset
        body.extend(handler);
        body.extend(tryblock);
        let start = asm::func(0xC1, &[(4, 1)], &body);
        let built = asm::assemble(&[start], 0, 0x200);
        let mut m = machine(built);
        m.run();
        assert_eq!(m.mem.read32(0x100).unwrap(), 42, "throw delivered 42 to the catch dest");
    }

    #[test]
    fn glk_char_to_lower_and_upper() {
        use asm::Op::{C8, Mem16};
        let mut body = glk_call(0xA0, &[C8(b'A' as i8)], Mem16(0x0100)); // char_to_lower('A')
        body.extend(glk_call(0xA1, &[C8(b'z' as i8)], Mem16(0x0104))); // char_to_upper('z')
        body.extend(glk_call(0xA0, &[C8(b'5' as i8)], Mem16(0x0108))); // non-letter unchanged
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), b'a' as u32);
        assert_eq!(m.mem.read32(0x104).unwrap(), b'Z' as u32);
        assert_eq!(m.mem.read32(0x108).unwrap(), b'5' as u32);
    }

    #[test]
    fn glk_buffer_to_lower_case_uni_folds_in_place() {
        use asm::Op::{C16, C8, Mem16};
        // Buffer of 4 uni chars "HÉLO" at 0x180; lower-case the first 4.
        let mut body = glk_call(0x120, &[C16(0x0180), C8(8), C8(4)], Mem16(0x0100));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            for (i, c) in ['H', 'É', 'L', 'O'].iter().enumerate() {
                m.mem.write32(0x180 + i as u32 * 4, *c as u32).unwrap();
            }
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 4, "result length");
        let got: String = (0..4).map(|i| char::from_u32(m.mem.read32(0x180 + i * 4).unwrap()).unwrap()).collect();
        assert_eq!(got, "hélo", "lower-cased in place (Unicode-aware)");
    }

    // SQ-0321: glk_buffer_to_title_case_uni (selector 0x0122) must titlecase the
    // first char with a real Unicode titlecase mapping, not the uppercase one:
    //   Ǆ (U+01C4) → ǅ (U+01C5), a titlecase digraph distinct from uppercase Ǆ;
    //   ﬄ (U+FB04) → "Ffl" (multi-char, first-upper-rest-lower), not "FFL".
    #[test]
    fn glk_buffer_to_title_case_uni_uses_titlecase_not_uppercase() {
        use asm::Op::{C16, C8, Mem16};
        // "Ǆz": titlecase (rest unchanged) → "ǅz".
        let mut body = glk_call(0x0122, &[C16(0x0180), C8(8), C8(2), C8(0)], Mem16(0x0100));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            m.mem.write32(0x180, 0x01C4).unwrap(); // Ǆ
            m.mem.write32(0x184, 0x007A).unwrap(); // z
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "length unchanged");
        assert_eq!(m.mem.read32(0x180).unwrap(), 0x01C5, "Ǆ → titlecase ǅ, not uppercase Ǆ");
        assert_eq!(m.mem.read32(0x184).unwrap(), 0x007A, "rest unchanged");

        // "ﬄ": titlecase → "Ffl" (single ligature expands to three chars).
        let mut body = glk_call(0x0122, &[C16(0x0180), C8(8), C8(1), C8(0)], Mem16(0x0100));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            m.mem.write32(0x180, 0xFB04).unwrap(); // ﬄ
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 3, "ligature titlecases to 3 chars");
        assert_eq!(m.mem.read32(0x180).unwrap(), 0x0046, "F");
        assert_eq!(m.mem.read32(0x184).unwrap(), 0x0066, "f (lower), not uppercase F");
        assert_eq!(m.mem.read32(0x188).unwrap(), 0x006C, "l (lower)");
    }

    // glk_buffer_canon_decompose_uni (selector 0x0123): é (U+00E9) decomposes to
    // U+0065 U+0301 (NFD), writing 2 chars back and returning the new count.
    #[test]
    fn glk_buffer_canon_decompose_uni_dispatch() {
        use asm::Op::{C16, C8, Mem16};
        let mut body = glk_call(0x0123, &[C16(0x0180), C8(8), C8(1)], Mem16(0x0100));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            m.mem.write32(0x180, 0x00E9).unwrap();
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "NFD length");
        assert_eq!(m.mem.read32(0x180).unwrap(), 0x0065, "base 'e'");
        assert_eq!(m.mem.read32(0x184).unwrap(), 0x0301, "combining acute");
    }

    // glk_buffer_canon_normalize_uni (selector 0x0124): U+0065 U+0301 composes to
    // é (U+00E9) under NFC, returning length 1.
    #[test]
    fn glk_buffer_canon_normalize_uni_dispatch() {
        use asm::Op::{C16, C8, Mem16};
        let mut body = glk_call(0x0124, &[C16(0x0180), C8(8), C8(2)], Mem16(0x0100));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            m.mem.write32(0x180, 0x0065).unwrap();
            m.mem.write32(0x184, 0x0301).unwrap();
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "NFC length");
        assert_eq!(m.mem.read32(0x180).unwrap(), 0x00E9, "composed é");
    }

    // Buffer-function contract: when the result is longer than buflen it is
    // truncated on write, but the FULL untruncated length is returned (mirrors
    // glk_buffer_to_lower_case_uni / cheapglk cgunicod.c).
    #[test]
    fn glk_buffer_canon_decompose_uni_truncates_but_returns_full_len() {
        use asm::Op::{C16, C8, Mem16};
        // buflen = 1, but é decomposes to 2 chars.
        let mut body = glk_call(0x0123, &[C16(0x0180), C8(1), C8(1)], Mem16(0x0100));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            m.mem.write32(0x180, 0x00E9).unwrap();
            m.mem.write32(0x184, 0xDEAD).unwrap(); // sentinel past buflen
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "full untruncated length returned");
        assert_eq!(m.mem.read32(0x180).unwrap(), 0x0065, "first char written");
        assert_eq!(m.mem.read32(0x184).unwrap(), 0xDEAD, "past buflen left untouched");
    }

    // gestalt_UnicodeNorm (16) now reports the normalization selectors supported.
    #[test]
    fn glk_gestalt_unicode_norm_reports_supported() {
        use asm::Op::{C8, Mem16};
        let mut body = glk_call(0x0004, &[C8(16), C8(0)], Mem16(0x0100));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "gestalt_UnicodeNorm -> supported");
    }

    #[test]
    fn glk_stream_close_minus_one_pushes_counts_to_stack() {
        use asm::Op::{C16, C8, Mem16, Stack, Zero};
        // The Glulx Glk dispatch -1 convention: glk_stream_close(str, -1) pushes
        // (readcount, writecount) so writecount is on top.
        let mut body = glk_call(0x43, &[C16(0x0180), C8(8), C8(1), C8(0)], Zero); // open_memory -> 2
        body.extend(glk_call(0x47, &[C8(2)], Zero)); // set_current(2)
        body.extend(glk_call(0x80, &[C8(b'A' as i8)], Zero)); // put 'A' -> memory
        body.extend(glk_call(0x80, &[C8(b'B' as i8)], Zero)); // put 'B'
        body.extend(glk_call(0x47, &[C8(1)], Zero)); // restore the window stream
        body.extend(glk_call(0x44, &[C8(2), C8(-1)], Zero)); // close(2, -1): pushes 0 then 2
        body.extend(asm::ins(0x40, &[Stack, Mem16(0x0100)])); // pop top (writecount) -> mem[0x100]
        body.extend(asm::ins(0x40, &[Stack, Mem16(0x0104)])); // pop next (readcount) -> mem[0x104]
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "writecount on top");
        assert_eq!(m.mem.read32(0x104).unwrap(), 0, "readcount beneath");
    }

    #[test]
    fn glk_select_poll_with_minus_one_pushes_event_words() {
        use asm::Op::{C8, Mem16, Stack};
        // glk_select_poll(-1) with no queued event pushes the 4 evtype_None words
        // (type, win, val1, val2 = 0,0,0,0) onto the stack.
        let mut body = glk_call(0xC1, &[C8(-1)], asm::Op::Zero);
        for addr in [0x0100u16, 0x0104, 0x0108, 0x010C] {
            body.extend(asm::ins(0x40, &[Stack, Mem16(addr)]));
        }
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        for addr in [0x0100u32, 0x0104, 0x0108, 0x010C] {
            assert_eq!(m.mem.read32(addr).unwrap(), 0, "evtype_None word @{addr:#x}");
        }
    }

    // ── @restart, @save/@restore stubs, debugtrap, glk_style_* ──────────────

    #[test]
    fn restart_resets_ram_and_reenters_start_func() {
        // The start function is simply `quit`. We create the machine, mutate RAM,
        // call op_restart directly, then verify RAM is reset and the machine can
        // run to halt cleanly.
        let body = asm::ins(0x120, &[]); // quit
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x200);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        // Mutate RAM after initial setup.
        m.mem.write32(0x100, 0xDEAD_BEEF).unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0xDEAD_BEEF, "sanity: RAM written");
        // op_restart resets RAM and re-enters start func.
        m.op_restart().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0, "RAM reset by restart");
        // Machine is now positioned at start_func again; run to completion.
        m.run();
        assert!(m.halted, "machine halted after restart");
    }

    #[test]
    fn restart_opcode_via_assembly() {
        use asm::Op::{C32, Mem16};
        // Program: store sentinel to RAM, @restart (0x0122), then quit.
        // After restart the machine re-enters this same function and runs to quit.
        // We check that RAM[0x100] is zero at the end (reset, then the restart loop
        // would store again — but we want only *one* restart, so after the second
        // entry the @restart is reached again… which would loop forever).
        //
        // Instead: write to a scratch address *before* the restart, and check it's
        // reset on the way through. To avoid the infinite-restart loop we use a
        // two-function program: start_func writes a sentinel and calls `quit`, and
        // a second function issues restart — but the start_func IS the restart entry.
        //
        // Simplest safe approach: test op_restart() directly (as above). This test
        // just verifies @restart doesn't *halt* the machine with an illegal-opcode error.
        let body = {
            let mut b = asm::ins(0x40, &[C32(0xABCD), Mem16(0x0100)]); // copy sentinel
            b.extend(asm::ins(0x122, &[])); // @restart (re-enters this body)
            // The second time through, copy falls through to restart again →
            // to avoid infinite loop in CI we keep it simple and just check
            // that @restart itself doesn't fault. We'll test memory reset with
            // the direct call above.
            b
        };
        // Build and run for a limited number of steps to confirm no illegal-opcode error.
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x200);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        // Step a few times (enough to hit @restart at least once).
        for _ in 0..6 {
            m.step();
        }
        // Confirm no illegal-opcode diagnostic for 0x122.
        assert!(
            !m.diagnostics.iter().any(|d| d.contains("0x122")),
            "restart must not produce an illegal-opcode error: {:?}",
            m.diagnostics
        );
    }

    #[test]
    fn save_suspends_and_failure_stores_one() {
        use asm::Op::{Mem16, Zero};
        // @save L1 S1: suspends with SaveRequest (does not halt); the host reports
        // failure via complete_save(false), which stores 1 into S1 and resumes.
        let mut body = asm::ins(0x123, &[Zero, Mem16(0x0100)]); // @save 0, -> mem[0x100]
        body.extend(asm::ins(0x120, &[])); // quit
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x200);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        // run() stops at the SaveRequest (not Continue) without halting or looping.
        m.run();
        assert!(!m.halted, "@save suspends rather than halting");
        assert_eq!(m.step(), StepResult::SaveRequest, "run() left the VM at the @save request");
        m.complete_save(false);
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "@save failure stores 1 into S1");
        assert_eq!(m.step(), StepResult::Quit, "resumes to the trailing quit");
    }

    #[test]
    fn save_pushes_call_stub_and_complete_save_resumes_via_stub() {
        use asm::Op::{C8, Mem16, Zero};
        // @save 0, -> mem[0x100]; copy 0x7F -> mem[0x104] (sentinel); quit.
        let mut body = asm::ins(0x123, &[Zero, Mem16(0x0100)]);
        body.extend(asm::ins(0x40, &[C8(0x7F), Mem16(0x0104)]));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);

        let before = m.value_count();
        assert_eq!(m.step(), StepResult::SaveRequest);
        assert_eq!(m.value_count(), before + 4, "a 4-word (16-byte) call stub was pushed");
        assert_eq!(m.mem.read32(0x100).unwrap(), 0, "@save no longer bakes anything into S1 pre-snapshot");

        m.complete_save(true);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0, "complete_save(true) stores the success code via the popped stub");
        assert_eq!(m.value_count(), before, "the stub was popped (S1 is Mem, not Push), nothing left on the stack");

        assert_eq!(m.step(), StepResult::Continue, "resumes at the stub's PC, just after @save");
        assert_eq!(m.mem.read32(0x104).unwrap(), 0x7F, "execution reached the trailing sentinel");
    }

    #[test]
    fn save_credits_its_stream_write_count_with_the_quetzal_length() {
        use asm::Op::{C8, Mem16};
        // @save 2, -> mem[0x100] — L1 = the file stream opened below. Stream
        // id 1 is the main window's stream, opened by machine_with_body's
        // default window; the file stream opened after it is id 2.
        let body = asm::ins(0x123, &[C8(2), Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);

        let fref = m.glk.fileref_create(0x00, "save".to_string(), 0);
        let sid = m.glk.stream_open_file(fref, 0x01, false, 0); // Write
        assert_eq!(sid, 2, "the file stream opened after the main window is id 2, matching @save's L1 operand");

        assert_eq!(m.step(), StepResult::SaveRequest);

        // SQ-0301: the stream is credited on the SUCCESS path (complete_save(true)),
        // not eagerly at suspend — so a cancelled save leaves the write-count check
        // reading failure. Once the host commits the save, the stream the game
        // opened has been credited with the bytes the real (host-intercepted) save
        // would have delivered.
        let expected = m.save_quetzal().len() as u32;
        m.complete_save(true);
        assert_eq!(
            m.glk.stream_close(sid),
            Some((0, expected)),
            "the stream's write_count equals save_quetzal()'s length, with no bytes actually stored"
        );

        // No bytes were actually written into the VFS file (a fresh read
        // stream over it is immediately at EOF).
        let rf = m.glk.fileref_create(0x00, "save".to_string(), 0);
        let rsid = m.glk.stream_open_file(rf, 0x02, false, 0); // Read
        assert_eq!(m.glk.file_stream_read_char(rsid), None, "the VFS file is still empty");
    }

    #[test]
    fn save_request_routes_create_by_name_as_silent_game_managed() {
        use asm::Op::{C8, Mem16};
        // @save 2 -> mem[0x100]; L1 = the SavedGame stream opened below (id 2).
        let body = asm::ins(0x123, &[C8(2), Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        // The game's OWN fixed-name save slot: create_by_name, SavedGame usage.
        let fref = m.glk.fileref_create(0x01, "startup-data".to_string(), 0);
        let sid = m.glk.stream_open_file(fref, 0x01, false, 0); // Write -> Null stream
        assert_eq!(sid, 2);
        assert_eq!(m.step(), StepResult::SaveRequest);
        let req = m.pending_saveload_request().expect("a save is pending");
        assert_eq!(
            req,
            SaveLoadRequest { name: "startup-data".to_string(), by_prompt: false, restore: false },
            "a create_by_name @save carries its fixed name and is NOT by_prompt (host writes it silently)"
        );
    }

    #[test]
    fn save_request_routes_create_by_prompt_to_host_ui() {
        use asm::Op::{C8, Mem16};
        let body = asm::ins(0x123, &[C8(2), Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        // The player's SAVE verb: create_by_prompt binds the name via
        // supply_filename -> fileref_create_prompted (by_prompt = true).
        let fref = m.glk.fileref_create_prompted(0x01, "myslot".to_string(), 0);
        let sid = m.glk.stream_open_file(fref, 0x01, false, 0);
        assert_eq!(sid, 2);
        assert_eq!(m.step(), StepResult::SaveRequest);
        let req = m.pending_saveload_request().expect("a save is pending");
        assert!(req.by_prompt, "a create_by_prompt @save is by_prompt (host surfaces the save UI)");
        assert_eq!(req.name, "myslot");
        assert!(!req.restore);
    }

    #[test]
    fn cancelled_fresh_save_reports_absent_and_leaves_no_credit() {
        use asm::Op::{C8, Mem16};
        // A brand-new by_prompt SAVE that the player cancels must report failure
        // on BOTH result channels: S1 = 1, AND the secondary channel a library
        // verifies through — glk_fileref_does_file_exist (→ false) + write_count
        // (→ uncredited). Before SQ-0301 @save credited eagerly at suspend, so a
        // cancel still reported the slot present at the full save size.
        let body = asm::ins(0x123, &[C8(2), Mem16(0x0100)]); // @save 2 -> mem[0x100]
        let mut m = machine_with_body(&[], body);
        let fref = m.glk.fileref_create_prompted(0x01, "myslot".to_string(), 0);
        let sid = m.glk.stream_open_file(fref, 0x01, false, 0); // Write -> Null stream
        assert_eq!(sid, 2);
        assert!(m.glk.fileref_exists(fref), "open-for-write marks existence at 0 (mirrors real Glk)");
        assert_eq!(m.step(), StepResult::SaveRequest);

        // The player cancels the SAVE-verb prompt.
        m.complete_save(false);

        // (a) primary channel: S1 = 1 (failure).
        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "cancelled @save stores 1 into S1");
        // (b) secondary channel: a cancelled fresh save leaves no file.
        assert!(!m.glk.fileref_exists(fref), "a cancelled brand-new save reports absent");
        // (c) the save stream was never credited the save's bytes.
        assert_eq!(m.glk.stream_close(2), Some((0, 0)), "no eager write-count credit on cancel");
    }

    #[test]
    fn cancelled_resave_over_an_existing_slot_keeps_it() {
        use asm::Op::{C8, Mem16};
        // The other half of the cancel rule (SQ-0301): the revert is gated on
        // `fresh`, so a cancelled save over a slot that ALREADY holds a save must
        // leave that save alone — clearing it would lose a good save to a
        // cancelled overwrite. (Write/0x01 truncates the recorded size to 0, so a
        // re-save reaches complete_save with fresh=false only via an append/
        // read-write open, which preserves the prior size.)
        let body = asm::ins(0x123, &[C8(2), Mem16(0x0100)]); // @save 2 -> mem[0x100]
        let mut m = machine_with_body(&[], body);
        // A previous launch's save already occupies the slot.
        m.glk.seed_saved_game_file("startup-data".to_string(), 4321);
        let fref = m.glk.fileref_create(0x01, "startup-data".to_string(), 0);
        let sid = m.glk.stream_open_file(fref, 0x05, false, 0); // ReadWrite -> keeps size
        assert_eq!(sid, 2);
        assert_eq!(m.glk.saved_game_size("startup-data"), Some(4321), "the open kept the prior size");
        assert_eq!(m.step(), StepResult::SaveRequest);

        // The player cancels the overwrite.
        m.complete_save(false);

        assert_eq!(m.mem.read32(0x100).unwrap(), 1, "cancelled @save stores 1 into S1");
        assert!(m.glk.fileref_exists(fref), "the pre-existing save survives a cancelled re-save");
        assert_eq!(
            m.glk.saved_game_size("startup-data"),
            Some(4321),
            "and keeps its original byte count — not reverted to absent like a fresh save",
        );
    }

    #[test]
    fn completed_save_credits_stream_and_marks_existence_at_true_size() {
        use asm::Op::{C8, Mem16};
        // The success path (complete_save(true)) must credit the save stream now
        // (not eagerly at suspend): write_count bumped and the SavedGame slot
        // marked present at the real save size for the library's verification.
        let body = asm::ins(0x123, &[C8(2), Mem16(0x0100)]); // @save 2 -> mem[0x100]
        let mut m = machine_with_body(&[], body);
        let fref = m.glk.fileref_create(0x01, "startup-data".to_string(), 0);
        let sid = m.glk.stream_open_file(fref, 0x01, false, 0); // Write -> Null stream
        assert_eq!(sid, 2);
        assert_eq!(m.step(), StepResult::SaveRequest);
        let expected = m.save_quetzal().len() as u32;
        assert_ne!(expected, 0, "a real Glulx-Quetzal save is non-empty");

        m.complete_save(true);

        assert_eq!(m.mem.read32(0x100).unwrap(), 0, "completed @save stores 0 into S1");
        assert!(m.glk.fileref_exists(fref), "a committed save marks the slot present");
        assert_eq!(
            m.glk.stream_close(2),
            Some((0, expected)),
            "the save stream is credited the true save byte count on success",
        );
    }

    #[test]
    fn restore_request_routes_create_by_name_as_silent_game_managed() {
        use asm::Op::{C8, Mem16};
        // @restore 2 -> mem[0x100]; L1 = the SavedGame read stream (id 2).
        let body = asm::ins(0x124, &[C8(2), Mem16(0x0100)]);
        let mut m = machine_with_body(&[], body);
        let fref = m.glk.fileref_create(0x01, "startup-data".to_string(), 0);
        let sid = m.glk.stream_open_file(fref, 0x02, false, 0); // Read -> Null stream
        assert_eq!(sid, 2);
        assert_eq!(m.step(), StepResult::RestoreRequest);
        let req = m.pending_saveload_request().expect("a restore is pending");
        assert_eq!(
            req,
            SaveLoadRequest { name: "startup-data".to_string(), by_prompt: false, restore: true },
            "a create_by_name @restore is silent + game-managed (host reads its fixed file, clean-fails if absent)"
        );
    }

    #[test]
    fn debugtrap_logs_value_and_continues() {
        use asm::Op::{C8, Mem16};
        // @debugtrap 0x42; copy a sentinel into RAM to prove execution continues.
        let mut body = asm::ins(0x101, &[C8(0x42)]); // debugtrap 0x42
        body.extend(asm::ins(0x40, &[C8(0x7F), Mem16(0x0100)])); // copy 0x7F -> RAM
        body.extend(asm::ins(0x120, &[])); // quit
        let start = asm::func(0xC1, &[], &body);
        let built = asm::assemble(&[start], 0, 0x200);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        m.run();
        assert!(m.halted, "machine halted normally");
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x7F, "execution continued past debugtrap");
        assert!(
            m.diagnostics.iter().any(|d| d.contains("debugtrap")),
            "debugtrap emitted a diagnostic: {:?}",
            m.diagnostics
        );
    }

    #[test]
    fn glk_style_distinguish_and_measure_return_zero_silently() {
        use asm::Op::{C8, Mem16, Zero};
        // glk_style_distinguish(win, style1, style2) → 0
        let mut body = glk_call(0xB2, &[C8(1), C8(0), C8(1)], Mem16(0x0100));
        // glk_style_measure(win, style, hint, result) → 0
        body.extend(glk_call(0xB3, &[C8(1), C8(0), C8(0), Zero], Mem16(0x0104)));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 0, "style_distinguish returns 0");
        assert_eq!(m.mem.read32(0x104).unwrap(), 0, "style_measure returns 0");
        assert!(
            m.diagnostics.is_empty(),
            "no diagnostic noise from style_distinguish/measure: {:?}",
            m.diagnostics
        );
    }

    // ── Glk stream reads (glk_get_*_stream) ──────────────────────────────────

    #[test]
    fn glk_get_char_stream_reads_bytes_and_returns_eof() {
        use asm::Op::{C8, C16, Mem16, Zero};
        // open_memory_stream over RAM bytes 0x180..0x183 ("AB\n"), then read 4 chars.
        // First 3 should be 'A','B','\n'; 4th is EOF (0xFFFFFFFF).
        let mut body = glk_call(0x43, &[C16(0x0180), C8(3), C8(1), C8(0)], Zero); // open_memory -> sid=2
        body.extend(glk_call(0x0090, &[C8(2)], Mem16(0x0100))); // get_char_stream(2) -> 'A'
        body.extend(glk_call(0x0090, &[C8(2)], Mem16(0x0104))); // -> 'B'
        body.extend(glk_call(0x0090, &[C8(2)], Mem16(0x0108))); // -> '\n'
        body.extend(glk_call(0x0090, &[C8(2)], Mem16(0x010C))); // -> EOF
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            m.mem.write8(0x180, b'A' as u32).unwrap();
            m.mem.write8(0x181, b'B' as u32).unwrap();
            m.mem.write8(0x182, b'\n' as u32).unwrap();
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), b'A' as u32, "first char");
        assert_eq!(m.mem.read32(0x104).unwrap(), b'B' as u32, "second char");
        assert_eq!(m.mem.read32(0x108).unwrap(), b'\n' as u32, "third char");
        assert_eq!(m.mem.read32(0x10C).unwrap(), 0xFFFF_FFFF, "EOF");
    }

    #[test]
    fn glk_get_buffer_stream_reads_up_to_len() {
        use asm::Op::{C8, C16, Mem16, Zero};
        // Stream over 4 bytes "RUST" at 0x180; read 3 into buf at 0x190, store count.
        let mut body = glk_call(0x43, &[C16(0x0180), C8(4), C8(1), C8(0)], Zero); // open
        body.extend(glk_call(0x0092, &[C8(2), C16(0x0190), C8(3)], Mem16(0x0100))); // get_buffer_stream
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            for (i, &ch) in b"RUST".iter().enumerate() {
                m.mem.write8(0x180 + i as u32, ch as u32).unwrap();
            }
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 3, "count = 3 (maxlen)");
        assert_eq!(m.mem.read8(0x190).unwrap(), b'R' as u32, "buf[0]");
        assert_eq!(m.mem.read8(0x191).unwrap(), b'U' as u32, "buf[1]");
        assert_eq!(m.mem.read8(0x192).unwrap(), b'S' as u32, "buf[2]");
    }

    #[test]
    fn glk_get_line_stream_stops_at_newline_and_nul_terminates() {
        use asm::Op::{C8, C16, Mem16, Zero};
        // Stream over "Hi\nWorld" (8 bytes); read line into buf at 0x190 with maxlen 6.
        let data = b"Hi\nWorld";
        let mut body = glk_call(0x43, &[C16(0x0180), C8(data.len() as i8), C8(1), C8(0)], Zero);
        body.extend(glk_call(0x0091, &[C8(2), C16(0x0190), C8(6)], Mem16(0x0100)));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            for (i, &b) in data.iter().enumerate() {
                m.mem.write8(0x180 + i as u32, b as u32).unwrap();
            }
        });
        // Should have read "Hi\n" (3 chars) and NUL-terminated.
        assert_eq!(m.mem.read32(0x100).unwrap(), 3, "3 chars read (including newline)");
        assert_eq!(m.mem.read8(0x190).unwrap(), b'H' as u32);
        assert_eq!(m.mem.read8(0x191).unwrap(), b'i' as u32);
        assert_eq!(m.mem.read8(0x192).unwrap(), b'\n' as u32);
        assert_eq!(m.mem.read8(0x193).unwrap(), 0, "NUL terminator");
    }

    #[test]
    fn glk_get_char_stream_uni_reads_unicode_stream() {
        use asm::Op::{C8, C16, Mem16, Zero};
        // open_memory_uni over 2 codepoints at 0x180: U+0048 ('H'), U+1F600 (emoji).
        let mut body = glk_call(0x0139, &[C16(0x0180), C8(2), C8(1), C8(0)], Zero); // open_memory_uni -> 2
        body.extend(glk_call(0x0130, &[C8(2)], Mem16(0x0100))); // get_char_stream_uni
        body.extend(glk_call(0x0130, &[C8(2)], Mem16(0x0104)));
        body.extend(glk_call(0x0130, &[C8(2)], Mem16(0x0108))); // EOF
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            m.mem.write32(0x180, 0x0048).unwrap();     // 'H'
            m.mem.write32(0x184, 0x1F600).unwrap();    // emoji
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0048, "first codepoint 'H'");
        assert_eq!(m.mem.read32(0x104).unwrap(), 0x1F600, "second codepoint emoji");
        assert_eq!(m.mem.read32(0x108).unwrap(), 0xFFFF_FFFF, "EOF");
    }

    #[test]
    fn glk_get_buffer_stream_uni_reads_codepoints() {
        use asm::Op::{C8, C16, Mem16, Zero};
        // 3-codepoint unicode stream at 0x180; read 2 into buf at 0x1C0.
        let mut body = glk_call(0x0139, &[C16(0x0180), C8(3), C8(1), C8(0)], Zero);
        body.extend(glk_call(0x0131, &[C8(2), C16(0x01C0), C8(2)], Mem16(0x0100)));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            m.mem.write32(0x180, b'A' as u32).unwrap();
            m.mem.write32(0x184, b'B' as u32).unwrap();
            m.mem.write32(0x188, b'C' as u32).unwrap();
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "count = 2");
        assert_eq!(m.mem.read32(0x1C0).unwrap(), b'A' as u32, "cp[0]");
        assert_eq!(m.mem.read32(0x1C4).unwrap(), b'B' as u32, "cp[1]");
    }

    // ── Glk file streams (glk_stream_open_file over the VFS) ─────────────────

    #[test]
    fn glk_file_stream_write_then_read_roundtrips() {
        use asm::Op::{C8, C16, Mem16, Zero};
        // create_by_name("save") -> fref@0x100; open it Write -> sid@0x104; write
        // "Hi" via put_char_stream and "!!" via put_buffer_stream; close; confirm
        // does_file_exist==1; reopen Read -> sid2@0x10C; read back "Hi!!" then EOF.
        let mut body = glk_call(0x61, &[C8(0), C16(0x0220), C8(0)], Mem16(0x0300)); // create_by_name
        body.extend(glk_call(0x42, &[Mem16(0x0300), C8(1), C8(0)], Mem16(0x0304))); // open_file(Write)
        body.extend(glk_call(0x81, &[Mem16(0x0304), C8(b'H' as i8)], Zero)); // put_char_stream 'H'
        body.extend(glk_call(0x81, &[Mem16(0x0304), C8(b'i' as i8)], Zero)); // put_char_stream 'i'
        body.extend(glk_call(0x85, &[Mem16(0x0304), C16(0x0280), C8(2)], Zero)); // put_buffer_stream "!!"
        body.extend(glk_call(0x44, &[Mem16(0x0304), C16(0x0330)], Zero)); // stream_close
        body.extend(glk_call(0x67, &[Mem16(0x0300)], Mem16(0x0308))); // does_file_exist -> 1
        body.extend(glk_call(0x42, &[Mem16(0x0300), C8(2), C8(0)], Mem16(0x030C))); // open_file(Read)
        body.extend(glk_call(0x90, &[Mem16(0x030C)], Mem16(0x0340))); // get_char 'H'
        body.extend(glk_call(0x90, &[Mem16(0x030C)], Mem16(0x0344))); // 'i'
        body.extend(glk_call(0x90, &[Mem16(0x030C)], Mem16(0x0348))); // '!'
        body.extend(glk_call(0x90, &[Mem16(0x030C)], Mem16(0x034C))); // '!'
        body.extend(glk_call(0x90, &[Mem16(0x030C)], Mem16(0x0350))); // EOF
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x400, |m| {
            poke(m, 0x0220, b"save\0");
            poke(m, 0x0280, b"!!");
        });
        assert_ne!(m.mem.read32(0x300).unwrap(), 0, "create_by_name -> live fileref");
        assert_ne!(m.mem.read32(0x304).unwrap(), 0, "open_file(Write) -> live stream");
        assert_eq!(m.mem.read32(0x308).unwrap(), 1, "file exists after write+close");
        assert_ne!(m.mem.read32(0x30C).unwrap(), 0, "open_file(Read) -> live stream");
        assert_eq!(m.mem.read32(0x340).unwrap(), b'H' as u32, "byte 0");
        assert_eq!(m.mem.read32(0x344).unwrap(), b'i' as u32, "byte 1");
        assert_eq!(m.mem.read32(0x348).unwrap(), b'!' as u32, "byte 2");
        assert_eq!(m.mem.read32(0x34C).unwrap(), b'!' as u32, "byte 3");
        assert_eq!(m.mem.read32(0x350).unwrap(), 0xFFFF_FFFF, "EOF");
        assert!(m.diagnostics.is_empty(), "no diagnostics: {:?}", m.diagnostics);
    }

    // ── Glk resource streams (glk_stream_open_resource over a Blorb Data chunk) ──

    /// A machine over `ram` bytes of RAM whose backend serves the given canned
    /// Data resources, for resource-stream dispatch tests (no window needed).
    fn resource_machine(backend: glk::TestBackend, ram: u32) -> Machine {
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, ram);
        let mem = Memory::new(built.image).expect("valid image");
        Machine::with_glk(mem, Box::new(backend))
    }

    #[test]
    fn glk_stream_open_resource_reads_bytes_and_reports_gestalt() {
        // Data resource #1 is a TEXT chunk "Hi\n"; #2 a binary chunk of 3 bytes.
        // Read them a byte at a time via glk_get_char_stream (0x0090) through the
        // non-uni resource path, then confirm EOF and a clean close.
        let backend = glk::TestBackend::new()
            .with_data_resource(1, b"Hi\n".to_vec(), true)
            .with_data_resource(2, vec![0xDE, 0xAD, 0xBE], false);
        let mut m = resource_machine(backend, 0x100);

        // gestalt_ResourceStream (22) is advertised.
        assert_eq!(m.glk_dispatch(0x0004, &[22, 0]).unwrap(), 1, "gestalt_ResourceStream");

        let sid = m.glk_dispatch(0x0049, &[1, 0]).unwrap(); // open_resource(filenum=1, rock=0)
        assert_ne!(sid, 0, "text resource opened");
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), b'H' as u32);
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), b'i' as u32);
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), b'\n' as u32);
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), 0xFFFF_FFFF, "EOF");
        // get_position reports the byte cursor (3), then close cleanly.
        assert_eq!(m.glk_dispatch(0x0046, &[sid]).unwrap(), 3, "position after reads");
        m.glk_dispatch(0x0044, &[sid, 0]).unwrap(); // stream_close, NULL result ptr

        // Binary resource: raw bytes, no decoding, in the non-uni path.
        let bin = m.glk_dispatch(0x0049, &[2, 0]).unwrap();
        assert_ne!(bin, 0);
        assert_eq!(m.glk_dispatch(0x0090, &[bin]).unwrap(), 0xDE);
        assert_eq!(m.glk_dispatch(0x0090, &[bin]).unwrap(), 0xAD);
        assert_eq!(m.glk_dispatch(0x0090, &[bin]).unwrap(), 0xBE);
        assert_eq!(m.glk_dispatch(0x0090, &[bin]).unwrap(), 0xFFFF_FFFF, "EOF");
        assert!(m.diagnostics.is_empty(), "no diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_stream_open_resource_nonuni_utf8_decodes_text_chunk_with_clamp() {
        // cheapglk cgstream.c: a TEXT chunk is UTF-8-decoded even on a
        // NON-unicode stream; a decoded code point above 0xFF clamps to '?'.
        // "é←" = U+00E9 (C3 A9, fits Latin-1) then U+2190 (E2 86 90, clamps).
        let backend =
            glk::TestBackend::new().with_data_resource(1, "é←".as_bytes().to_vec(), true);
        let mut m = resource_machine(backend, 0x400);
        let sid = m.glk_dispatch(0x0049, &[1, 0]).unwrap(); // open_resource (non-uni)
        assert_ne!(sid, 0);
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), 0xE9, "é decoded from UTF-8");
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), b'?' as u32, "U+2190 clamps to '?'");
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), 0xFFFF_FFFF, "EOF");
        // Position is the BYTE cursor: 2 (é) + 3 (←) = 5 bytes consumed.
        assert_eq!(m.glk_dispatch(0x0046, &[sid]).unwrap(), 5, "byte cursor after decode");

        // Buffer-read variant decodes + clamps the same way.
        let sid2 = m.glk_dispatch(0x0049, &[1, 0]).unwrap();
        let n = m.glk_dispatch(0x0092, &[sid2, 0x300, 8]).unwrap(); // get_buffer_stream
        assert_eq!(n, 2, "two decoded chars");
        assert_eq!(m.mem.read8(0x300).unwrap(), 0xE9);
        assert_eq!(m.mem.read8(0x301).unwrap(), b'?' as u32);
        assert!(m.diagnostics.is_empty(), "no diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_stream_open_resource_uni_decodes_per_chunk_type() {
        // A TEXT chunk read as unicode decodes UTF-8; a BINA chunk read as
        // unicode takes 4 big-endian bytes per code point (cheapglk gtstream.c).
        // Text: "Aλ" = 0x41, then the 2-byte UTF-8 for U+03BB.
        let text = "Aλ".as_bytes().to_vec();
        // Binary: one 32-bit BE code point U+1F600.
        let bin = 0x0001_F600u32.to_be_bytes().to_vec();
        let backend = glk::TestBackend::new()
            .with_data_resource(1, text, true)
            .with_data_resource(2, bin, false);
        let mut m = resource_machine(backend, 0x100);

        let tsid = m.glk_dispatch(0x013A, &[1, 0]).unwrap(); // open_resource_uni(1)
        assert_ne!(tsid, 0);
        assert_eq!(m.glk_dispatch(0x0130, &[tsid]).unwrap(), 0x41, "'A'");
        assert_eq!(m.glk_dispatch(0x0130, &[tsid]).unwrap(), 0x03BB, "λ via UTF-8");
        assert_eq!(m.glk_dispatch(0x0130, &[tsid]).unwrap(), 0xFFFF_FFFF, "EOF");

        let bsid = m.glk_dispatch(0x013A, &[2, 0]).unwrap();
        assert_ne!(bsid, 0);
        assert_eq!(m.glk_dispatch(0x0130, &[bsid]).unwrap(), 0x0001_F600, "4-byte BE code point");
        assert_eq!(m.glk_dispatch(0x0130, &[bsid]).unwrap(), 0xFFFF_FFFF, "EOF");
        assert!(m.diagnostics.is_empty(), "no diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_byte_read_on_a_unicode_resource_clamps_instead_of_eof() {
        // SQ-0322: a byte-oriented read (glk_get_char_stream / _line / _buffer)
        // on a UNICODE stream returns each code point clamped to a byte
        // (> 0xFF → '?'), not EOF (Glk spec §5.4). Binary chunk of two 32-bit
        // BE code points: 'A' (in range) and U+1F600 (clamps).
        let mut bin = 0x0000_0041u32.to_be_bytes().to_vec();
        bin.extend_from_slice(&0x0001_F600u32.to_be_bytes());
        let backend = glk::TestBackend::new().with_data_resource(1, bin, false);
        let mut m = resource_machine(backend, 0x400);

        let sid = m.glk_dispatch(0x013A, &[1, 0]).unwrap(); // open_resource_uni(1)
        assert_ne!(sid, 0);
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), 0x41, "'A' via a byte read on a uni stream");
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), b'?' as u32, "U+1F600 clamps to '?', not EOF");
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), 0xFFFF_FFFF, "EOF");

        // Buffer-read variant clamps identically.
        let sid2 = m.glk_dispatch(0x013A, &[1, 0]).unwrap();
        let n = m.glk_dispatch(0x0092, &[sid2, 0x300, 8]).unwrap();
        assert_eq!(n, 2, "two code points read (not EOF)");
        assert_eq!(m.mem.read8(0x300).unwrap(), 0x41);
        assert_eq!(m.mem.read8(0x301).unwrap(), b'?' as u32);
        assert!(m.diagnostics.is_empty(), "no diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_byte_read_on_a_unicode_memory_stream_clamps_instead_of_eof() {
        // SQ-0322: a byte-oriented read on a Unicode MEMORY stream returns each
        // 32-bit element clamped to a byte (> 0xFF → '?'), not EOF (cheapglk
        // gli_get_char / gli_get_buffer, strtype_Memory unicode arm).
        let mut m = resource_machine(glk::TestBackend::new(), 0x400);
        // Two 32-bit elements at 0x300: 'A' (in range) and U+2190 (clamps).
        m.mem.write32(0x300, 0x41).unwrap();
        m.mem.write32(0x304, 0x2190).unwrap();
        // open_memory_uni over 2 words, fmode = Read (buffer is content).
        let sid = m.glk_dispatch(0x0139, &[0x300, 2, 2, 0]).unwrap();
        assert_ne!(sid, 0);
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), 0x41, "'A' passes through");
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), b'?' as u32, "U+2190 clamps to '?', not EOF");
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), 0xFFFF_FFFF, "EOF");

        // Buffer-read variant clamps identically (seek back to the start).
        m.glk_dispatch(0x0045, &[sid, 0, 0]).unwrap();
        let n = m.glk_dispatch(0x0092, &[sid, 0x380, 8]).unwrap();
        assert_eq!(n, 2, "two elements read (not EOF)");
        assert_eq!(m.mem.read8(0x380).unwrap(), 0x41);
        assert_eq!(m.mem.read8(0x381).unwrap(), b'?' as u32);
        assert!(m.diagnostics.is_empty(), "no diagnostics: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_byte_write_above_0xff_stores_question_mark() {
        // SQ-0322: writing a code point > 0xFF to a byte (Latin-1) stream
        // stores '?' (0x3F), not the low byte (Glk spec §5.4).
        let mut m = resource_machine(glk::TestBackend::new(), 0x400);
        // open_memory over [0x300, 0x304) as a BYTE stream (fmode write).
        let sid = m.glk_dispatch(0x0043, &[0x300, 4, 1, 0]).unwrap();
        assert_ne!(sid, 0);
        m.glk_dispatch(0x012B, &[sid, 0x01C4]).unwrap(); // put_char_stream_uni(U+01C4)
        m.glk_dispatch(0x012B, &[sid, b'x' as u32]).unwrap(); // put_char_stream_uni('x')
        assert_eq!(m.mem.read8(0x300).unwrap(), 0x3F, "U+01C4 > 0xFF stored as '?', not low byte 0xC4");
        assert_eq!(m.mem.read8(0x301).unwrap(), b'x' as u32, "in-range char stored verbatim");
    }

    #[test]
    fn glk_stream_open_resource_buffer_read_and_missing() {
        // glk_get_buffer_stream (0x0092) fills Glulx memory from a resource; a
        // missing resource number opens as 0 (failure) per the Glk spec.
        let backend = glk::TestBackend::new().with_data_resource(5, b"ABCDE".to_vec(), false);
        let mut m = resource_machine(backend, 0x400);
        let sid = m.glk_dispatch(0x0049, &[5, 0]).unwrap();
        // Read 3 bytes into RAM at 0x300.
        let n = m.glk_dispatch(0x0092, &[sid, 0x300, 3]).unwrap();
        assert_eq!(n, 3, "buffer read count");
        assert_eq!(m.mem.read8(0x300).unwrap(), b'A' as u32);
        assert_eq!(m.mem.read8(0x301).unwrap(), b'B' as u32);
        assert_eq!(m.mem.read8(0x302).unwrap(), b'C' as u32);
        // Seek back to start (seekmode 0) then re-read the first byte.
        m.glk_dispatch(0x0045, &[sid, 0, 0]).unwrap();
        assert_eq!(m.glk_dispatch(0x0090, &[sid]).unwrap(), b'A' as u32, "seek to start");

        // A resource number the backend has no chunk for → open fails.
        assert_eq!(m.glk_dispatch(0x0049, &[99, 0]).unwrap(), 0, "missing resource → 0");
        assert_eq!(m.glk_dispatch(0x013A, &[99, 0]).unwrap(), 0, "missing resource (uni) → 0");
    }

    #[test]
    fn resource_stream_survives_snapshot_roundtrip() {
        // An open resource stream round-trips through the Glk snapshot: after a
        // partial read, serialize + deserialize the model and confirm the cursor
        // and remaining bytes are preserved (the host isn't consulted on restore).
        let mut model = glk::Model::new();
        let sid = model.stream_open_resource(b"WXYZ".to_vec(), false, false, 0);
        assert_eq!(model.resource_stream_read_char(sid), Some(b'W' as u32));
        let blob = model.serialize();
        let mut restored = glk::Model::deserialize(&blob).expect("snapshot round-trips");
        assert_eq!(restored.resource_stream_read_char(sid), Some(b'X' as u32), "cursor preserved");
        assert_eq!(restored.resource_stream_read_char(sid), Some(b'Y' as u32));
        assert_eq!(restored.stream_position(sid), Some(3));
    }

    #[test]
    fn machine_vfs_roundtrip() {
        // Write a file into one machine's VFS, snapshot it with the Machine
        // sidecar accessor, then load it into a fresh machine and read it back.
        let mut m = machine_with_glk(&[]);
        assert!(!m.vfs_dirty(), "a fresh machine's VFS is not dirty");
        let f = m.glk.fileref_create(0x00, "greeting".to_string(), 0);
        let sid = m.glk.stream_open_file(f, 0x01, false, 0); // Write
        m.glk.file_stream_write(sid, "Hi");
        m.glk.stream_close(sid);
        assert!(m.vfs_dirty(), "writing the VFS marks the machine dirty");
        let blob = m.vfs_bytes();

        let mut m2 = machine_with_glk(&[]);
        m2.load_vfs(&blob);
        let rf = m2.glk.fileref_create(0x00, "greeting".to_string(), 0);
        assert!(m2.glk.fileref_exists(rf), "the loaded file is present");
        let rsid = m2.glk.stream_open_file(rf, 0x02, false, 0); // Read
        assert_ne!(rsid, 0, "the loaded file opens for reading");
        let mut got = String::new();
        while let Some(c) = m2.glk.file_stream_read_char(rsid) {
            got.push(c as u8 as char);
        }
        assert_eq!(got, "Hi", "the bytes survive the sidecar round-trip");
    }

    // ── gestalt truthfulness ─────────────────────────────────────────────────

    #[test]
    fn glk_gestalt_all_known_selectors_are_accurate() {
        use asm::Op::{C8, Mem16};
        let mut body = glk_call(0x04, &[C8(0), C8(0)], Mem16(0x0100));  // Version
        body.extend(glk_call(0x04, &[C8(1), C8(0)], Mem16(0x0104)));    // CharInput
        body.extend(glk_call(0x04, &[C8(2), C8(0)], Mem16(0x0108)));    // LineInput
        body.extend(glk_call(0x04, &[C8(3), C8(65)], Mem16(0x010C)));   // CharOutput 'A'
        body.extend(glk_call(0x04, &[C8(4), C8(0)], Mem16(0x0110)));    // MouseInput
        body.extend(glk_call(0x04, &[C8(5), C8(0)], Mem16(0x0114)));    // Timer
        body.extend(glk_call(0x04, &[C8(6), C8(0)], Mem16(0x0118)));    // Graphics
        body.extend(glk_call(0x04, &[C8(7), C8(0)], Mem16(0x011C)));    // DrawImage
        body.extend(glk_call(0x04, &[C8(8), C8(0)], Mem16(0x0120)));    // Sound
        body.extend(glk_call(0x04, &[C8(15), C8(0)], Mem16(0x0124)));   // Unicode
        body.extend(glk_call(0x04, &[C8(22), C8(0)], Mem16(0x0128)));   // ResourceStream
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0000_0706, "Version 0.7.6");
        assert_eq!(m.mem.read32(0x104).unwrap(), 1, "CharInput supported");
        assert_eq!(m.mem.read32(0x108).unwrap(), 1, "LineInput supported");
        assert_eq!(m.mem.read32(0x10C).unwrap(), 2, "CharOutput = ExactPrint");
        assert_eq!(m.mem.read32(0x110).unwrap(), 1, "MouseInput supported");
        assert_eq!(m.mem.read32(0x114).unwrap(), 1, "Timer supported");
        assert_eq!(m.mem.read32(0x118).unwrap(), 0, "Graphics not supported");
        assert_eq!(m.mem.read32(0x11C).unwrap(), 0, "DrawImage not supported");
        assert_eq!(m.mem.read32(0x120).unwrap(), 0, "Sound not supported");
        assert_eq!(m.mem.read32(0x124).unwrap(), 1, "Unicode supported");
        assert_eq!(m.mem.read32(0x128).unwrap(), 1, "ResourceStream supported (SQ-0308)");
        assert!(m.diagnostics.is_empty(), "no noise: {:?}", m.diagnostics);
    }

    #[test]
    fn graphics_gestalt_gated_on_flag() {
        let mut m = super::tests::machine_with_glk(&[]); // helper that builds a Machine over minimal mem + TestBackend
        // Default: graphics OFF → gestalt reports none.
        assert_eq!(m.glk_gestalt(6, 0), 0, "gestalt_Graphics off by default");
        assert_eq!(m.glk_gestalt(7, 5), 0, "gestalt_DrawImage(Graphics) off");
        m.set_graphics(true);
        assert_eq!(m.glk_gestalt(6, 0), 1, "gestalt_Graphics on");
        assert_eq!(m.glk_gestalt(7, 5), 1, "gestalt_DrawImage(wintype_Graphics=5) on");
        assert_eq!(m.glk_gestalt(7, 3), 1, "gestalt_DrawImage(wintype_TextBuffer=3) on — inline images (Surface A)");
        assert_eq!(m.glk_gestalt(7, 4), 0, "gestalt_DrawImage(wintype_TextGrid=4) off — grids can't draw images");
        assert_eq!(m.glk_gestalt(14, 0), 1, "gestalt_GraphicsTransparency on");
    }

    #[test]
    fn graphics_enabled_getter_reflects_set_graphics() {
        let mut m = super::tests::machine_with_glk(&[]);
        assert!(!m.graphics_enabled(), "graphics off by default");
        m.set_graphics(true);
        assert!(m.graphics_enabled(), "getter reflects set_graphics(true)");
    }

    #[test]
    fn graphics_window_open_gated_on_flag() {
        let mut m = super::tests::machine_with_glk(&[]);
        // wintype_Graphics = 5; open a root graphics window.
        assert_eq!(m.glk_open_window(0, 0, 0, 5, 0), 0, "graphics window rejected when disabled");
        m.set_graphics(true);
        assert_ne!(m.glk_open_window(0, 0, 0, 5, 0), 0, "graphics window opens when enabled");
    }

    #[test]
    fn graphics_ops_dispatch_to_backend() {
        let mut m = super::tests::machine_with_glk(&[]);
        m.set_graphics(true);
        let win = m.glk_open_window(0, 0, 0, 5, 0); // graphics root
        assert_ne!(win, 0);

        // Verified arg order: glk_dispatch's `a(i)` reads args[i] as the i-th Glk
        // parameter in natural left-to-right order (confirmed against the
        // glk_window_get_size arm at selector 0x0025, which does
        // a(0)=win, a(1)=awidthptr, a(2)=aheightptr — and op_glk pops args off
        // the stack in that same first-arg-first order before calling
        // glk_dispatch). So fill_rect(win, color, left, top, w, h) takes
        // &[win, color, left, top, w, h].
        m.glk_dispatch(0x00EA, &[win, 0x00FF_0000, 1, 2, 3, 4]).unwrap(); // fill_rect
        m.glk_dispatch(0x00EB, &[win, 0x0000_00FF]).unwrap(); // set_background_color
        let drew = m.glk_dispatch(0x00E1, &[win, 7, 5, 6]).unwrap(); // image_draw(win, resnum=7, x=5, y=6)
        assert_eq!(drew, 1, "backend resolved resnum 7 -> glk_image_draw reports success");

        let tb = m.backend.as_any().downcast_ref::<glk::TestBackend>().unwrap();
        assert_eq!(tb.fills(win), vec![(0x00FF_0000, 1, 2, 3, 4)]);
        assert_eq!(tb.background(win), Some(0x0000_00FF));
        assert_eq!(tb.draws(win), vec![(7, 5, 6, None)]);
    }

    #[test]
    fn graphics_window_clear_erases_canvas_to_background() {
        // SQ-0403: glk_window_clear on a GRAPHICS window must erase the canvas to
        // its background colour (glk_window_set_background_color). It used to fall
        // through the clear dispatch's `_ => {}` (only text windows were handled),
        // so a game that set a black background — Counterfeit Monkey's help window
        // — left the canvas transparent and the white page showed through.
        let mut m = super::tests::machine_with_glk(&[]);
        m.set_graphics(true);
        let win = m.glk_open_window(0, 0, 0, 5, 0); // graphics root
        assert_ne!(win, 0);

        m.glk_dispatch(0x00EB, &[win, 0]).unwrap(); // set_background_color(win, 0 = black)
        m.glk_dispatch(0x002A, &[win]).unwrap();    // window_clear(win)

        let tb = m.backend.as_any().downcast_ref::<glk::TestBackend>().unwrap();
        // The clear reached the backend as a full-canvas erase to the background.
        assert_eq!(
            tb.fills(win),
            vec![(0, 0, 0, u32::MAX, u32::MAX)],
            "graphics clear must erase the whole canvas to the set background colour"
        );
        assert_eq!(tb.background(win), Some(0));
    }

    #[test]
    fn screen_trace_records_glk_calls_when_enabled_and_skips_text_io() {
        let mut m = super::tests::machine_with_glk(&[]);
        m.trace_screen = true;
        let win = m.glk_dispatch(0x0023, &[0, 0, 3]).unwrap(); // glk_window_open (traced, returns win id)
        m.glk_dispatch(0x0080, &[b'x' as u32]).ok(); // glk_put_char (text I/O → skipped)
        // SQ-0405: the return value rides on the same line as `-> win N`.
        assert!(
            m.screen_trace.iter().any(|l| l.starts_with("glk_window_open(") && l.ends_with(&format!("-> win {win}"))),
            "{:?}", m.screen_trace
        );
        assert!(!m.screen_trace.iter().any(|l| l.starts_with("glk_put_char")), "text I/O skipped");

        let mut off = super::tests::machine_with_glk(&[]);
        off.trace_screen = false;
        off.glk_dispatch(0x0023, &[0, 0, 3]).ok();
        assert!(off.screen_trace.is_empty());
    }

    #[test]
    fn graphics_draw_image_reports_backend_failure() {
        // glk_image_draw/glk_image_draw_scaled must reflect whether the
        // backend actually resolved and drew the image, not just whether
        // graphics is enabled (SQ-0175 part A).
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x100);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(glk::TestBackend::new().with_missing_image(99)));
        m.set_graphics(true);
        let win = m.glk_open_window(0, 0, 0, 5, 0);
        assert_ne!(win, 0);

        let drew = m.glk_dispatch(0x00E1, &[win, 99, 5, 6]).unwrap();
        assert_eq!(drew, 0, "missing resnum -> glk_image_draw reports failure");
        let drew_scaled = m.glk_dispatch(0x00E2, &[win, 99, 5, 6, 10, 20]).unwrap();
        assert_eq!(drew_scaled, 0, "missing resnum -> glk_image_draw_scaled reports failure");

        let tb = m.backend.as_any().downcast_ref::<glk::TestBackend>().unwrap();
        assert_eq!(tb.draws(win), Vec::new(), "nothing recorded for a failed draw");

        // A resnum the backend doesn't consider missing still succeeds.
        let drew_ok = m.glk_dispatch(0x00E1, &[win, 7, 1, 2]).unwrap();
        assert_eq!(drew_ok, 1);
    }

    #[test]
    fn glk_window_flow_break_is_silent_noop() {
        // glk_window_flow_break(win) — selector 0x00E8 per the Glk spec's
        // selector table (falls between 0x00E2 glk_image_draw_scaled and
        // 0x00E9 glk_window_erase_rect). Block-mode inline images already
        // break the text flow, so this must be accepted as a no-op rather
        // than falling into the "unhandled @glk selector" diagnostic arm.
        let mut m = super::tests::machine_with_glk(&[]);
        assert_eq!(m.glk_dispatch(0x00E8, &[1]).unwrap(), 0);
        assert!(m.diagnostics.is_empty(), "flow_break must be a handled selector: {:?}", m.diagnostics);
    }

    #[test]
    fn graphics_ops_noop_when_disabled() {
        let mut m = super::tests::machine_with_glk(&[]);
        // graphics_enabled stays false (default); no window exists, but the
        // selectors must still no-op silently and return 0 rather than panic.
        assert_eq!(m.glk_dispatch(0x00EA, &[1, 0x00FF_0000, 1, 2, 3, 4]).unwrap(), 0);
        assert_eq!(m.glk_dispatch(0x00EB, &[1, 0x0000_00FF]).unwrap(), 0);
        assert_eq!(m.glk_dispatch(0x00E1, &[1, 7, 5, 6]).unwrap(), 0);
        assert_eq!(m.glk_dispatch(0x00E9, &[1, 1, 2, 3, 4]).unwrap(), 0);
        assert_eq!(m.glk_dispatch(0x00E2, &[1, 7, 5, 6, 10, 20]).unwrap(), 0);
        assert_eq!(m.glk_dispatch(0x00E0, &[7, 0, 0]).unwrap(), 0, "image_get_info returns 0/false");

        let tb = m.backend.as_any().downcast_ref::<glk::TestBackend>().unwrap();
        assert_eq!(tb.fills(1), Vec::new(), "no backend calls recorded when disabled");
        assert_eq!(tb.draws(1), Vec::new());
        assert_eq!(tb.background(1), None);
    }

    /// Assemble a start routine that opens a graphics root window, fills a
    /// rect, and draws image #1 — all via hand-assembled `@glk` (0x130)
    /// instructions decoded and executed by the real opcode-dispatch loop
    /// (`op_glk` pops the args off the VM stack and calls `glk_dispatch`),
    /// rather than calling `glk_dispatch`/`glk_open_window` directly as the
    /// tests above do. `win` is stashed at RAM address 0x0100 so later calls
    /// can pass it back in and the driving test can read it out afterward.
    fn synthetic_graphics_story_body() -> Vec<u8> {
        use asm::Op::{C8, C32, Mem16, Zero};
        // glk_window_open(split=0, method=0, size=0, wintype_Graphics=5, rock=0)
        let mut body = glk_call(0x0023, &[C8(0), C8(0), C8(0), C8(5), C8(0)], Mem16(0x0100));
        // glk_window_fill_rect(win, color=0x00112233, left=1, top=2, w=3, h=4)
        body.extend(glk_call(0x00EA, &[Mem16(0x0100), C32(0x0011_2233), C8(1), C8(2), C8(3), C8(4)], Zero));
        // glk_image_draw(win, image=1, x=5, y=6)
        body.extend(glk_call(0x00E1, &[Mem16(0x0100), C8(1), C8(5), C8(6)], Zero));
        body.extend(asm::ins(0x120, &[])); // quit
        body
    }

    /// End-to-end confidence check: with graphics enabled, the assembled
    /// story's `@glk` calls flow through opcode decode -> `op_glk` -> arg
    /// popping -> `glk_dispatch` -> the backend, landing the exact fill/draw
    /// ops on the exact window the story opened.
    #[test]
    fn synthetic_graphics_story_records_ops_when_enabled() {
        let start = asm::func(0xC1, &[], &synthetic_graphics_story_body());
        let built = asm::assemble(&[start], 0, 0x100);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        m.set_graphics(true);
        m.run();

        let win = m.mem.read32(0x0100).unwrap();
        assert_ne!(win, 0, "graphics root window opened when enabled");

        let tb = backend_of(&m);
        assert_eq!(tb.fills(win), vec![(0x0011_2233, 1, 2, 3, 4)]);
        assert_eq!(tb.draws(win), vec![(1, 5, 6, None)]);
        assert!(m.diagnostics.is_empty(), "no noise: {:?}", m.diagnostics);
    }

    /// The same assembled story, but graphics never gets enabled (the
    /// default): the gate holds end-to-end — `glk_window_open` rejects the
    /// graphics wintype (win stays 0), and the subsequent fill/draw selectors
    /// no-op rather than recording anything against window 0.
    #[test]
    fn synthetic_graphics_story_gated_off_by_default() {
        let start = asm::func(0xC1, &[], &synthetic_graphics_story_body());
        let built = asm::assemble(&[start], 0, 0x100);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        // graphics_enabled left at its default (false) — no `set_graphics` call.
        m.run();

        let win = m.mem.read32(0x0100).unwrap();
        assert_eq!(win, 0, "graphics window rejected when disabled");

        let tb = backend_of(&m);
        assert_eq!(tb.fills(0), Vec::new(), "no fill recorded with graphics off");
        assert_eq!(tb.draws(0), Vec::new(), "no draw recorded with graphics off");
    }

    #[test]
    fn graphics_fixed_split_converts_pixels_to_cells() {
        // A backend reporting 8x16 px cells; a 150px-tall fixed graphics window
        // below a text buffer. The terminal footprint rounds up to ceil(150/16)
        // = 10 cells, but get_size reports the EXACT 150px the game requested (a
        // rounded 160 would throw off layout code that echoes its request).
        let mut m = super::tests::machine_with_glk_charpx(80, 24, 8, 16);
        m.set_graphics(true);
        let buf = m.glk_open_window(0, 0, 0, 3, 0); // text buffer root
        // winmethod: BELOW(0x03) | FIXED(0x10) = 0x13, size=150 px, wintype_Graphics=5
        let gfx = m.glk_open_window(buf, 0x13, 150, 5, 0);
        assert_ne!(gfx, 0);
        // Fixed axis (height): exact requested 150px. Free axis (width): 80 cells * 8 = 640.
        let (w_px, h_px) = m.graphics_window_pixels(gfx).unwrap();
        assert_eq!(h_px, 150, "fixed height reports the exact requested pixels");
        assert_eq!(w_px, 640, "free width is cells × char_px");
    }

    #[test]
    fn graphics_window_open_pushes_redraw() {
        let mut m = super::tests::machine_with_glk_charpx(80, 24, 8, 16);
        m.set_graphics(true);
        let _gfx = m.glk_open_window(0, 0, 0, 5, 0);
        assert!(
            m.glk.take_pending_events().iter().any(|e| e.etype == glk::evtype::REDRAW),
            "opening a graphics window queues a Redraw"
        );
    }

    #[test]
    fn arrangement_never_pushes_arrange_and_redraws_only_with_graphics() {
        // A program-driven set_arrangement must NEVER synthesize an evtype_Arrange
        // (that would spin a game whose arrange handler re-arranges — SQ-0272). A
        // pair of text windows only also needs no redraw.
        let mut m = super::tests::machine_with_glk_charpx(80, 24, 8, 16);
        let buf = m.glk_open_window(0, 0, 0, 3, 0); // text buffer root
        let grid = m.glk_open_window(buf, 0x12, 3, 4, 0); // grid above, fixed 3
        m.glk_dispatch(0x0026, &[m.glk.window_parent(grid).unwrap(), 0x12, 5, grid]).unwrap(); // set_arrangement
        let evs = m.glk.take_pending_events();
        assert!(!evs.iter().any(|e| e.etype == glk::evtype::ARRANGE),
            "program-driven rearrangement must not queue an Arrange event");
        assert!(
            !evs.iter().any(|e| e.etype == glk::evtype::REDRAW),
            "no graphics window in the tree — no redraw needed"
        );

        // Add a graphics window to the tree: now rearranging queues a redraw (but
        // still no Arrange) so the game repaints the resized graphics.
        m.set_graphics(true);
        let gfx = m.glk_open_window(buf, 0x13, 150, 5, 0); // graphics below, fixed 150px
        m.glk.take_pending_events(); // drain the open-triggered redraw
        m.glk_dispatch(0x0026, &[m.glk.window_parent(gfx).unwrap(), 0x13, 100, gfx]).unwrap(); // set_arrangement
        let evs = m.glk.take_pending_events();
        assert!(!evs.iter().any(|e| e.etype == glk::evtype::ARRANGE),
            "still no Arrange even with a graphics window");
        assert!(
            evs.iter().any(|e| e.etype == glk::evtype::REDRAW),
            "a graphics window is in the tree — arrangement queues a redraw"
        );
    }

    // ── Floating point (single-precision, GLULX_NOTES §13.1) ──────────────────

    fn f32c(v: f32) -> asm::Op {
        asm::Op::C32(v.to_bits())
    }

    /// Execute a 1-load, 1-store float op; `a` is encoded as its float bits.
    fn farith1(op: u32, a: f32) -> u32 {
        arith1(op, f32c(a))
    }

    /// Execute a 2-load, 1-store float op; `a`/`b` are encoded as float bits.
    fn farith2(op: u32, a: f32, b: f32) -> u32 {
        arith2(op, f32c(a), f32c(b))
    }

    /// fmod L1 L2 S1 S2 — returns (S1 = remainder, S2 = quotient).
    fn fmod2(a: f32, b: f32) -> (u32, u32) {
        let body = asm::ins(0x1A4, &[f32c(a), f32c(b), asm::Op::Mem16(0x0100), asm::Op::Mem16(0x0104)]);
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        (m.mem.read32(0x100).unwrap(), m.mem.read32(0x104).unwrap())
    }

    /// Whether a float branch opcode taken with the given float load operands
    /// (offset 40 appended) actually jumps.
    fn ftaken(op: u32, loads: &[f32]) -> bool {
        let mut ops: Vec<asm::Op> = loads.iter().map(|&v| f32c(v)).collect();
        ops.push(asm::Op::C8(40));
        let body = asm::ins(op, &ops);
        let blen = body.len() as u32;
        let mut m = machine_with_body(&[], body);
        let pc0 = m.pc;
        m.step_once().unwrap();
        m.pc != pc0 + blen
    }

    // ── double-precision (3.1.3) test helpers ─────────────────────────────────
    /// A double as its two operand words in Glulx read order: high, then low.
    fn d64c(v: f64) -> [asm::Op; 2] {
        let b = v.to_bits();
        [asm::Op::C32((b >> 32) as u32), asm::Op::C32(b as u32)]
    }

    /// Run a double op taking `loads` doubles and storing one double; return it.
    fn deval(op: u32, loads: &[f64]) -> f64 {
        let mut ops: Vec<asm::Op> = loads.iter().flat_map(|&v| d64c(v)).collect();
        ops.push(asm::Op::Mem16(0x0100)); // low word
        ops.push(asm::Op::Mem16(0x0104)); // high word
        let mut body = asm::ins(op, &ops);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[(4, 1)], body);
        m.step_once().unwrap();
        let lo = m.mem.read32(0x100).unwrap();
        let hi = m.mem.read32(0x104).unwrap();
        f64::from_bits((u64::from(hi) << 32) | u64::from(lo))
    }

    /// Run a double→int op (one store word); return the stored word.
    fn dtonum(op: u32, v: f64) -> u32 {
        let mut ops = d64c(v).to_vec();
        ops.push(asm::Op::Mem16(0x0100));
        let mut body = asm::ins(op, &ops);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[(4, 1)], body);
        m.step_once().unwrap();
        m.mem.read32(0x100).unwrap()
    }

    /// Whether a double jump op branches for `loads` doubles (mirror of ftaken).
    fn dtaken(op: u32, loads: &[f64]) -> bool {
        let mut ops: Vec<asm::Op> = loads.iter().flat_map(|&v| d64c(v)).collect();
        ops.push(asm::Op::C8(40));
        let body = asm::ins(op, &ops);
        let blen = body.len() as u32;
        let mut m = machine_with_body(&[], body);
        let pc0 = m.pc;
        m.step_once().unwrap();
        m.pc != pc0 + blen
    }

    #[test]
    fn numtod_encodes_low_word_first() {
        // The spec's write convention (3.1.3 §2.13): "the result is stored as
        // S2:S1" — S1 receives the LOW word, S2 the high word (glulxe
        // op_numtod agrees: lo→inst[1], hi→inst[2]). Reads are the reverse
        // (high word first); see numtod_dtonumz_round_trip_through_the_stack
        // for why the asymmetry is load-bearing. (SQ-0626)
        // numtod 3 -> mem[0x100]=low, mem[0x104]=high. 3.0 = 0x4008_0000_0000_0000.
        let mut body = asm::ins(0x200, &[asm::Op::C32(3), asm::Op::Mem16(0x0100), asm::Op::Mem16(0x0104)]);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[(4, 1)], body);
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x0000_0000, "low word stored first");
        assert_eq!(m.mem.read32(0x104).unwrap(), 0x4008_0000, "high word stored second");
    }

    #[test]
    fn double_to_int_conversions() {
        assert_eq!(dtonum(0x201, 3.75), 3, "dtonumz truncates toward zero");
        assert_eq!(dtonum(0x201, -3.75) as i32, -3, "dtonumz truncates negatives toward zero");
        assert_eq!(dtonum(0x202, 3.75), 4, "dtonumn rounds to nearest");
        assert_eq!(dtonum(0x201, f64::NAN), 0x7FFF_FFFF, "NaN -> 0x7FFFFFFF");
    }

    #[test]
    fn double_arithmetic_reads_hi_first_stores_lo_first() {
        assert_eq!(deval(0x210, &[2.0, 3.0]), 5.0, "dadd");
        assert_eq!(deval(0x211, &[5.0, 3.0]), 2.0, "dsub");
        assert_eq!(deval(0x212, &[2.0, 3.0]), 6.0, "dmul");
        assert_eq!(deval(0x213, &[6.0, 3.0]), 2.0, "ddiv");
        assert_eq!(deval(0x218, &[9.0]), 3.0, "dsqrt");
        assert_eq!(deval(0x21B, &[2.0, 10.0]), 1024.0, "dpow");
        assert_eq!(deval(0x208, &[2.3]), 3.0, "dceil");
        assert_eq!(deval(0x209, &[2.7]), 2.0, "dfloor");
        assert_eq!(deval(0x214, &[7.0, 3.0]), 1.0, "dmodr remainder");
        assert_eq!(deval(0x215, &[7.0, 3.0]), 2.0, "dmodq quotient");
    }

    #[test]
    fn float_double_conversions_round_trip() {
        // ftod(2.5f32) -> double 2.5.
        let f = 2.5f32;
        let mut body = asm::ins(0x203, &[asm::Op::C32(f.to_bits()), asm::Op::Mem16(0x0100), asm::Op::Mem16(0x0104)]);
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[(4, 1)], body);
        m.step_once().unwrap();
        let lo = m.mem.read32(0x100).unwrap();
        let hi = m.mem.read32(0x104).unwrap();
        assert_eq!(f64::from_bits((u64::from(hi) << 32) | u64::from(lo)), 2.5, "ftod widens exactly");
        // dtof(2.5) -> float 2.5.
        assert_eq!(f32::from_bits(dtonum(0x204, 2.5)), 2.5, "dtof narrows exactly");
    }

    #[test]
    fn double_comparisons_branch_correctly() {
        assert!(dtaken(0x232, &[2.0, 3.0]), "jdlt 2<3");
        assert!(!dtaken(0x232, &[3.0, 2.0]), "jdlt 3<2 is false");
        assert!(dtaken(0x233, &[3.0, 3.0]), "jdle 3<=3");
        assert!(dtaken(0x234, &[3.0, 2.0]), "jdgt 3>2");
        assert!(dtaken(0x235, &[3.0, 3.0]), "jdge 3>=3");
        assert!(!dtaken(0x232, &[f64::NAN, 3.0]), "NaN makes ordered comparison false");
        // jdeq fuzzy equality within tolerance; jdne is its inverse.
        assert!(dtaken(0x230, &[2.0, 2.05, 0.1]), "jdeq within tolerance");
        assert!(!dtaken(0x230, &[2.0, 3.0, 0.1]), "jdeq outside tolerance");
        assert!(dtaken(0x231, &[2.0, 3.0, 0.1]), "jdne outside tolerance");
        // jdisnan / jdisinf.
        assert!(dtaken(0x238, &[f64::NAN]), "jdisnan on NaN");
        assert!(!dtaken(0x238, &[1.0]), "jdisnan on a number is false");
        assert!(dtaken(0x239, &[f64::INFINITY]), "jdisinf on inf");
        assert!(!dtaken(0x239, &[1.0]), "jdisinf on a number is false");
    }

    #[test]
    fn gestalt_reports_double_supported() {
        let m = machine_with_body(&[], vec![]);
        assert_eq!(m.gestalt(13, 0), 1, "Double gestalt supported");
        assert_eq!(m.gestalt(0, 0), 0x0003_0103, "version bumped to 3.1.3");
    }

    #[test]
    fn numtof_converts_signed_int_to_nearest_float() {
        use asm::Op::C8;
        assert_eq!(arith1(0x190, C8(3)), 3.0f32.to_bits());
        assert_eq!(arith1(0x190, C8(-2)), (-2.0f32).to_bits());
    }

    #[test]
    fn ftonumz_truncates_toward_zero_with_overflow_and_nan() {
        assert_eq!(farith1(0x191, 3.7), 3u32);
        assert_eq!(farith1(0x191, -3.7), (-3i32) as u32);
        assert_eq!(farith1(0x191, 1e30), 0x7FFF_FFFF);
        assert_eq!(farith1(0x191, -1e30), 0x8000_0000);
        assert_eq!(farith1(0x191, f32::INFINITY), 0x7FFF_FFFF);
        assert_eq!(farith1(0x191, f32::NAN), 0x7FFF_FFFF);
    }

    #[test]
    fn ftonumn_rounds_to_nearest_half_away_from_zero() {
        assert_eq!(farith1(0x192, 3.5), 4u32);
        assert_eq!(farith1(0x192, 2.5), 3u32);
        assert_eq!(farith1(0x192, -2.5), (-3i32) as u32);
    }

    #[test]
    fn ceil_and_floor_round_toward_infinity() {
        assert_eq!(farith1(0x198, 2.3), 3.0f32.to_bits());
        assert_eq!(farith1(0x199, 2.7), 2.0f32.to_bits());
        assert_eq!(farith1(0x199, -2.1), (-3.0f32).to_bits());
        let ceil_neg_half = f32::from_bits(farith1(0x198, -0.5));
        assert_eq!(ceil_neg_half, 0.0);
        assert!(ceil_neg_half.is_sign_negative(), "ceil(-0.5) should be -0.0");
    }

    #[test]
    fn fadd_fsub_fmul_fdiv_basic_and_special_values() {
        assert_eq!(farith2(0x1A0, 1.5, 2.5), 4.0f32.to_bits());
        assert_eq!(farith2(0x1A1, 5.0, 2.0), 3.0f32.to_bits());
        assert_eq!(farith2(0x1A2, 3.0, 4.0), 12.0f32.to_bits());
        assert_eq!(farith2(0x1A3, 10.0, 2.0), 5.0f32.to_bits());
        assert_eq!(farith2(0x1A3, 1.0, 0.0), f32::INFINITY.to_bits());
        assert!(f32::from_bits(farith2(0x1A3, 0.0, 0.0)).is_nan());
    }

    #[test]
    fn fmod_remainder_and_quotient_signs() {
        let (rem, quot) = fmod2(7.0, 3.0);
        assert_eq!(rem, 1.0f32.to_bits());
        assert_eq!(quot, 2.0f32.to_bits());
        let (rem, quot) = fmod2(-7.0, 3.0);
        assert_eq!(quot, (-2.0f32).to_bits());
        assert_eq!(rem, (-1.0f32).to_bits());
    }

    #[test]
    fn sqrt_exp_log_pow_domain_and_values() {
        assert_eq!(farith1(0x1A8, 4.0), 2.0f32.to_bits());
        assert!(f32::from_bits(farith1(0x1A8, -1.0)).is_nan());
        let exp1 = f32::from_bits(farith1(0x1A9, 1.0));
        assert!((exp1 - std::f32::consts::E).abs() < 1e-5);
        let ln_e = f32::from_bits(farith1(0x1AA, std::f32::consts::E));
        assert!((ln_e - 1.0).abs() < 1e-5);
        let p = f32::from_bits(farith2(0x1AB, 2.0, 10.0));
        assert!((p - 1024.0).abs() < 1e-5);
    }

    #[test]
    fn trig_known_values() {
        let sin90 = f32::from_bits(farith1(0x1B0, std::f32::consts::FRAC_PI_2));
        assert!((sin90 - 1.0).abs() < 1e-5);
        let cos0 = f32::from_bits(farith1(0x1B1, 0.0));
        assert!((cos0 - 1.0).abs() < 1e-5);
        let tan0 = f32::from_bits(farith1(0x1B2, 0.0));
        assert!(tan0.abs() < 1e-5);
        let asin1 = f32::from_bits(farith1(0x1B3, 1.0));
        assert!((asin1 - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        let acos1 = f32::from_bits(farith1(0x1B4, 1.0));
        assert!(acos1.abs() < 1e-5);
        let atan1 = f32::from_bits(farith1(0x1B5, 1.0));
        assert!((atan1 - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
        let atan2_11 = f32::from_bits(farith2(0x1B6, 1.0, 1.0));
        assert!((atan2_11 - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
    }

    #[test]
    fn jfeq_fuzzy_equality_and_special_cases() {
        assert!(ftaken(0x1C0, &[1.0, 1.0, 0.0]));
        assert!(ftaken(0x1C0, &[1.0, 1.05, 0.1]));
        assert!(!ftaken(0x1C0, &[1.0, 2.0, 0.1]));
        assert!(!ftaken(0x1C0, &[f32::NAN, 1.0, 0.1]));
        assert!(!ftaken(0x1C0, &[1.0, f32::NAN, 0.1]));
        assert!(!ftaken(0x1C0, &[1.0, 1.0, f32::NAN]));
        assert!(ftaken(0x1C0, &[f32::INFINITY, f32::INFINITY, 0.0]));
        assert!(!ftaken(0x1C0, &[f32::INFINITY, f32::NEG_INFINITY, 0.0]));
    }

    #[test]
    fn jfne_is_the_inverse_of_jfeq_including_nan() {
        assert!(!ftaken(0x1C1, &[1.0, 1.0, 0.0]));
        assert!(ftaken(0x1C1, &[1.0, 2.0, 0.1]));
        // Spec: jfne DOES branch when any argument is NaN (feq is false → !feq is true).
        assert!(ftaken(0x1C1, &[f32::NAN, 1.0, 0.1]));
    }

    #[test]
    fn jflt_jfle_jfgt_jfge_basics_and_nan() {
        assert!(ftaken(0x1C2, &[1.0, 2.0])); // jflt
        assert!(!ftaken(0x1C2, &[2.0, 1.0]));
        assert!(ftaken(0x1C3, &[1.0, 1.0])); // jfle
        assert!(ftaken(0x1C4, &[2.0, 1.0])); // jfgt
        assert!(!ftaken(0x1C4, &[1.0, 2.0]));
        assert!(ftaken(0x1C5, &[1.0, 1.0])); // jfge
        assert!(!ftaken(0x1C2, &[f32::NAN, 1.0]), "NaN compares false");
    }

    #[test]
    fn jisnan_jisinf_true_and_false() {
        assert!(ftaken(0x1C8, &[f32::NAN]));
        assert!(!ftaken(0x1C8, &[1.0]));
        assert!(ftaken(0x1C9, &[f32::INFINITY]));
        assert!(ftaken(0x1C9, &[f32::NEG_INFINITY]));
        assert!(!ftaken(0x1C9, &[1.0]));
    }

    #[test]
    fn gestalt_reports_float_support() {
        let m = machine_with_body(&[], vec![]);
        assert_eq!(m.gestalt(11, 0), 1);
    }

    // ── Glk 0.7.6: date/time, echo streams, uni stream-puts, style measure ─────

    #[test]
    fn glk_gestalt_reports_datetime_support() {
        let m = machine_with_body(&[], vec![]);
        assert_eq!(m.glk_gestalt(20, 0), 1, "gestalt_DateTime supported");
    }

    #[test]
    fn glk_date_time_selectors_round_trip_through_at_glk() {
        use asm::Op::{C16, Zero};
        // A known timeval at 0x180 = { high=0, low=1_784_030_400, micro=0 } is
        // 2026-07-14 12:00:00 UTC. Convert to a date, then back to a timeval.
        let mut body = glk_call(0x160, &[C16(0x0140)], Zero); // glk_current_time -> exercises the clock
        body.extend(glk_call(0x168, &[C16(0x0180), C16(0x01A0)], Zero)); // time_to_date_utc
        body.extend(glk_call(0x16C, &[C16(0x01A0), C16(0x01C0)], Zero)); // date_to_time_utc (inverse)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| {
            poke(m, 0x180, &0u32.to_be_bytes()); // high_sec
            poke(m, 0x184, &1_784_030_400u32.to_be_bytes()); // low_sec
            poke(m, 0x188, &0u32.to_be_bytes()); // microsec
        });
        // The glkdate written at 0x1A0 (year, month, day, weekday, hour, ...).
        assert_eq!(m.mem.read32(0x1A0).unwrap(), 2026, "year");
        assert_eq!(m.mem.read32(0x1A4).unwrap(), 7, "month");
        assert_eq!(m.mem.read32(0x1A8).unwrap(), 14, "day");
        assert_eq!(m.mem.read32(0x1AC).unwrap(), 2, "weekday = Tuesday");
        assert_eq!(m.mem.read32(0x1B0).unwrap(), 12, "hour");
        // date_to_time_utc inverts the conversion back to the original timeval.
        assert_eq!(m.mem.read32(0x1C0).unwrap(), 0, "high_sec restored");
        assert_eq!(m.mem.read32(0x1C4).unwrap(), 1_784_030_400, "low_sec restored");
        // glk_current_time wrote a nonzero (modern) low_sec at 0x144.
        assert_ne!(m.mem.read32(0x144).unwrap(), 0, "current_time wrote the live clock");
        assert!(m.diagnostics.is_empty(), "no unhandled selector: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_echo_stream_duplicates_window_output_to_a_second_stream() {
        use asm::Op::{C16, C8, Mem16, Zero};
        // Prelude made window 1 current. Open a memory stream (id 2) over
        // [0x180,0x188), set it as window 1's echo, then print to the window.
        let mut body = glk_call(0x43, &[C16(0x0180), C8(8), C8(1), C8(0)], Mem16(0x0100)); // open_memory -> 2
        body.extend(glk_call(0x2D, &[C8(1), C8(2)], Zero)); // window_set_echo_stream(1, 2)
        body.extend(glk_call(0x2E, &[C8(1)], Mem16(0x0104))); // window_get_echo_stream(1) -> 0x104
        body.extend(glk_call(0x82, &[C16(0x0200)], Zero)); // glk_put_string("Hi") -> window (+ echo)
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |m| poke(m, 0x200, &[0xE0, b'H', b'i', 0]));
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "open_memory returns stream 2");
        assert_eq!(m.mem.read32(0x104).unwrap(), 2, "get_echo_stream returns the set stream");
        assert_eq!(backend_of(&m).text(1), "Hi", "the window still receives the text");
        assert_eq!(m.mem.read8(0x180).unwrap(), b'H' as u32, "echo stream also got 'H'");
        assert_eq!(m.mem.read8(0x181).unwrap(), b'i' as u32, "echo stream also got 'i'");
        assert!(m.diagnostics.is_empty(), "no unhandled selector: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_uni_stream_put_variants_write_words_including_astral() {
        use asm::Op::{C16, C32, C8, Mem16, Zero};
        // open_memory_uni -> stream 2 over 8 words at 0x180. Write via all three
        // uni stream-put variants: char (astral emoji), buffer, and string.
        let mut body = glk_call(0x139, &[C16(0x0180), C8(8), C8(1), C8(0)], Mem16(0x0100));
        // put_char_stream_uni(2, U+1F600) at word 0.
        body.extend(glk_call(0x12B, &[C8(2), C32(0x1F600)], Zero));
        // put_buffer_stream_uni(2, 0x220, 2): the 32-bit code points at 0x220.
        body.extend(glk_call(0x12D, &[C8(2), C16(0x0220), C8(2)], Zero));
        // put_string_stream_uni(2, 0x240): an E2 Unicode string object "λ".
        body.extend(glk_call(0x12C, &[C8(2), C16(0x0240)], Zero));
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x300, |m| {
            poke(m, 0x220, &0x41u32.to_be_bytes()); // 'A'
            poke(m, 0x224, &0x42u32.to_be_bytes()); // 'B'
            poke(m, 0x240, &[0xE2, 0, 0, 0]); // E2 tag + 3 pad bytes
            poke(m, 0x244, &0x3BBu32.to_be_bytes()); // 'λ'
            poke(m, 0x248, &0u32.to_be_bytes()); // terminator
        });
        assert_eq!(m.mem.read32(0x100).unwrap(), 2, "open_memory_uni -> stream 2");
        assert_eq!(m.mem.read32(0x180).unwrap(), 0x1F600, "put_char_stream_uni wrote the astral char");
        assert_eq!(m.mem.read32(0x184).unwrap(), b'A' as u32, "put_buffer_stream_uni word 0");
        assert_eq!(m.mem.read32(0x188).unwrap(), b'B' as u32, "put_buffer_stream_uni word 1");
        assert_eq!(m.mem.read32(0x18C).unwrap(), 0x3BB, "put_string_stream_uni decoded 'λ'");
        assert!(m.diagnostics.is_empty(), "no unhandled selector: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_style_measure_and_distinguish_through_at_glk() {
        use asm::Op::{C16, C32, C8, Mem16, Zero};
        // Set style_Note (6) TextColor (7) = 0x00FF00 on the text-buffer window (3),
        // then measure it and distinguish it from the unhinted style_Normal (0).
        let mut body = glk_call(0xB0, &[C8(3), C8(6), C8(7), C32(0x00FF00)], Zero); // stylehint_set
        body.extend(glk_call(0xB3, &[C8(1), C8(6), C8(7), C16(0x0100)], Mem16(0x0104))); // style_measure -> result@0x100, ret@0x104
        body.extend(glk_call(0xB3, &[C8(1), C8(6), C8(8), C16(0x0110)], Mem16(0x0114))); // measure unset BackColor
        body.extend(glk_call(0xB2, &[C8(1), C8(6), C8(0)], Mem16(0x0118))); // distinguish Note vs Normal
        body.extend(glk_call(0xB2, &[C8(1), C8(0), C8(0)], Mem16(0x011C))); // distinguish Normal vs Normal
        body.extend(asm::ins(0x120, &[]));
        let m = run_with_ram(body, 0x200, |_| {});
        assert_eq!(m.mem.read32(0x104).unwrap(), 1, "style_measure(TextColor) succeeded");
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x00FF00, "measured colour stored in result");
        assert_eq!(m.mem.read32(0x114).unwrap(), 0, "unset BackColor -> measure fails (0)");
        assert_eq!(m.mem.read32(0x118).unwrap(), 1, "Note (hinted) distinguishable from Normal");
        assert_eq!(m.mem.read32(0x11C).unwrap(), 0, "a style is not distinguishable from itself");
        assert!(m.diagnostics.is_empty(), "no unhandled selector: {:?}", m.diagnostics);
    }

    #[test]
    fn glk_style_measure_falls_back_to_backend_default_colours() {
        // SQ-0315: with no game-set hint, style_measure answers with the host's
        // actually-rendered default colours; a game-set hint takes precedence;
        // a channel the host reports as terminal-default (None) stays unknown.
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x200);
        let mem = Memory::new(built.image).expect("valid image");
        let backend =
            glk::TestBackend::new().with_default_colours(Some(0x00C5C8C6), Some(0x001D1F21));
        let mut m = Machine::with_glk(mem, Box::new(backend));
        let win = m.glk_open_window(0, 0, 0, 3, 0); // text buffer
        assert_ne!(win, 0);

        // No hint set → backend colours reported for TextColor (7) / BackColor (8).
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 0, 7, 0x100]).unwrap(), 1, "fg known via backend");
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x00C5C8C6, "backend fg");
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 0, 8, 0x104]).unwrap(), 1, "bg known via backend");
        assert_eq!(m.mem.read32(0x104).unwrap(), 0x001D1F21, "backend bg");
        // Non-colour hints never consult the backend (still honestly unknown).
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 0, 4, 0x108]).unwrap(), 0, "Size stays unknown");

        // A game-set hint wins over the backend's default.
        m.glk_dispatch(0x00B0, &[3, 0, 8, 0x00FF0000]).unwrap(); // stylehint_set BackColor red
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 0, 8, 0x10C]).unwrap(), 1);
        assert_eq!(m.mem.read32(0x10C).unwrap(), 0x00FF0000, "game hint wins over backend");
        // ...and clearing it falls back to the backend again.
        m.glk_dispatch(0x00B1, &[3, 0, 8]).unwrap(); // stylehint_clear
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 0, 8, 0x110]).unwrap(), 1);
        assert_eq!(m.mem.read32(0x110).unwrap(), 0x001D1F21, "cleared hint -> backend bg again");
    }

    #[test]
    fn glk_style_measure_reports_the_backend_per_style_colour() {
        // SQ-0803: a host that paints one style class in its own colour (the app's
        // per-Glk-style theme slot, e.g. a garglk.ini `tcolor 10`) must report THAT
        // colour for that style and the window base for every other — this is the
        // Kerkerkruip 0xF400A1 style_User2 sentinel. Constants verified against
        // glk.h (eblong.com): style_Normal 0, style_User2 10, stylehint_TextColor 7,
        // stylehint_BackColor 8, wintype_TextBuffer 3.
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x200);
        let mem = Memory::new(built.image).expect("valid image");
        let backend = glk::TestBackend::new()
            .with_default_colours(Some(0x00C5C8C6), Some(0x001D1F21))
            .with_style_colours(10, Some(0x00F400A1), Some(0x00FFFFFF));
        let mut m = Machine::with_glk(mem, Box::new(backend));
        let win = m.glk_open_window(0, 0, 0, 3, 0); // wintype_TextBuffer
        assert_ne!(win, 0);

        // style_User2 reports the slot, not the window base.
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 10, 7, 0x100]).unwrap(), 1);
        assert_eq!(m.mem.read32(0x100).unwrap(), 0x00F400A1, "User2 fg is the style slot");
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 10, 8, 0x104]).unwrap(), 1);
        assert_eq!(m.mem.read32(0x104).unwrap(), 0x00FFFFFF, "User2 bg is the style slot");
        // Every other style still reports the window base.
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 0, 7, 0x108]).unwrap(), 1);
        assert_eq!(m.mem.read32(0x108).unwrap(), 0x00C5C8C6, "Normal fg is the base");
        // And a game-set hint still wins over the host's per-style colour.
        m.glk_dispatch(0x00B0, &[3, 10, 7, 0x0000FF00]).unwrap(); // stylehint_set
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 10, 7, 0x10C]).unwrap(), 1);
        assert_eq!(m.mem.read32(0x10C).unwrap(), 0x0000FF00, "game hint wins over the slot");
    }

    #[test]
    fn glk_style_distinguish_sees_the_backend_per_style_colours() {
        // SQ-0803: with no game hints at all, two styles the HOST paints
        // differently are distinguishable; two it paints alike are not.
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x200);
        let mem = Memory::new(built.image).expect("valid image");
        let backend = glk::TestBackend::new()
            .with_default_colours(Some(0x00C5C8C6), Some(0x001D1F21))
            .with_style_colours(10, Some(0x00F400A1), Some(0x00FFFFFF));
        let mut m = Machine::with_glk(mem, Box::new(backend));
        let win = m.glk_open_window(0, 0, 0, 3, 0);
        assert_eq!(m.glk_dispatch(0x00B2, &[win, 0, 10]).unwrap(), 1, "Normal vs User2 differ");
        assert_eq!(m.glk_dispatch(0x00B2, &[win, 0, 3]).unwrap(), 0, "Normal vs Header alike");
        assert_eq!(m.glk_dispatch(0x00B2, &[win, 10, 10]).unwrap(), 0, "a style equals itself");
        // An out-of-range style class is never distinguishable (glk.h: NUMSTYLES 11).
        assert_eq!(m.glk_dispatch(0x00B2, &[win, 0, 11]).unwrap(), 0, "style 11 is out of range");
        // No window, no answer.
        assert_eq!(m.glk_dispatch(0x00B2, &[0, 0, 10]).unwrap(), 0, "invalid window");
    }

    #[test]
    fn glk_style_measure_backend_partial_and_absent_info() {
        // A host that knows only its bg (fg is terminal-default) reports just
        // that channel; a host with no info at all (trait default) yields 0.
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x200);
        let mem = Memory::new(built.image).expect("valid image");
        let backend = glk::TestBackend::new().with_default_colours(None, Some(0x00101010));
        let mut m = Machine::with_glk(mem, Box::new(backend));
        let win = m.glk_open_window(0, 0, 0, 3, 0);
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 0, 7, 0x100]).unwrap(), 0, "fg unknown (terminal default)");
        assert_eq!(m.glk_dispatch(0x00B3, &[win, 0, 8, 0x104]).unwrap(), 1, "bg known");
        assert_eq!(m.mem.read32(0x104).unwrap(), 0x00101010);

        // Backend with no colour info at all: measure of an unhinted colour → 0.
        let mut plain = machine_with_glk(&[]);
        let win2 = plain.glk.root();
        assert_eq!(plain.glk_dispatch(0x00B3, &[win2, 0, 7, 0x100]).unwrap(), 0, "no info -> unknown");
    }

    // ── SQ-0623..0627: reviewed-defect regressions ────────────────────────────

    /// Reassemble an IFZS container from `(id, data)` chunks (inverse of
    /// [`iff_chunks`]) so a test can corrupt one chunk of a real save.
    fn rebuild_ifzs(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (id, data) in chunks {
            push_chunk(&mut body, id, data);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(b"IFZS");
        out.extend_from_slice(&body);
        out
    }

    /// A copy of `blob` with the data of chunk `id` rewritten by `f`.
    fn corrupt_chunk(blob: &[u8], id: &[u8; 4], f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut chunks = iff_chunks(blob);
        let (_, data) = chunks.iter_mut().find(|(cid, _)| cid == id).expect("chunk present");
        f(data);
        rebuild_ifzs(&chunks)
    }

    #[test]
    fn restore_rejects_crafted_stks_localspos() {
        // SQ-0623: LocalsPos comes off the restored stack; before validation a
        // crafted value passed reload_frame_meta and the first local access
        // indexed stack[fp + 0xFFFF_FFF0 + off] — a panic, not a load error.
        let body =
            [asm::ins(0x40, &[asm::Op::Local8(0), asm::Op::Zero]), asm::ins(0x120, &[])].concat();
        let mut m = machine_with_body(&[(4, 1)], body);
        let blob = m.save_state();
        let bad = corrupt_chunk(&blob, b"Stks", |d| {
            d[4..8].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes()); // start frame's LocalsPos
        });
        assert!(m.restore_state(&bad).is_err(), "crafted LocalsPos must be a BadSave");
    }

    #[test]
    fn restore_rejects_frame_len_past_sp() {
        // SQ-0623: a restored FrameLen that puts value_base() above sp makes
        // value_count() underflow on the next @stkcount/@stkpeek (debug panic,
        // release OOB index). Must be rejected at restore.
        let mut m = machine_with_body(&[], asm::ins(0x120, &[]));
        let blob = m.save_state();
        let bad = corrupt_chunk(&blob, b"Stks", |d| {
            let huge = (d.len() as u32 + 64) & !3;
            d[0..4].copy_from_slice(&huge.to_be_bytes()); // start frame's FrameLen
        });
        assert!(m.restore_state(&bad).is_err(), "FrameLen past sp must be a BadSave");
    }

    #[test]
    fn restore_rejects_corrupt_glk_snapshot_root() {
        // SQ-0623: a bogus root id in the "Glk " chunk previously installed
        // fine and panicked later in build_win_tree/layout_window.
        let mut m = machine_with_glk(&[]);
        assert_ne!(m.glk_open_window(0, 0, 0, 3, 0), 0);
        let blob = m.save_state();
        let bad = corrupt_chunk(&blob, b"Glk ", |d| {
            d[4..8].copy_from_slice(&99u32.to_be_bytes()); // root follows the version word
        });
        assert!(m.restore_state(&bad).is_err(), "a dead Glk root must be a BadSave");
    }

    #[test]
    fn restore_rejects_hostile_heap_chunks() {
        // SQ-0623: MAll blocks restored out of order made @malloc's gap walk
        // compute `a - cursor` with a < cursor (debug panic; release wrap →
        // overlapping blocks), and a bogus low heap_start would let @mfree
        // truncate memory below RAMSTART (compress_ram then underflows).
        let mut m = machine_with_body(&[], asm::ins(0x120, &[]));
        let blob = m.save_state();
        let hs = m.mem.endmem();

        // (a) out-of-order blocks, inside a memory map grown to hold them.
        let grown = corrupt_chunk(&blob, b"CMem", |d| {
            let memsize = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) + 0x100;
            d[0..4].copy_from_slice(&memsize.to_be_bytes());
            d.extend_from_slice(&[0x00, 0xFF]); // 256 more zero diff bytes
        });
        let unsorted = corrupt_chunk(&grown, b"MAll", |d| {
            d.clear();
            d.extend_from_slice(&hs.to_be_bytes()); // heap_start: valid
            d.extend_from_slice(&2u32.to_be_bytes());
            for (a, sz) in [(hs + 0x20, 0x10u32), (hs, 0x10)] {
                d.extend_from_slice(&a.to_be_bytes());
                d.extend_from_slice(&sz.to_be_bytes());
            }
        });
        assert!(m.restore_state(&unsorted).is_err(), "unsorted heap blocks must be a BadSave");

        // (b) blocks reaching past the restored memory map.
        let past_end = corrupt_chunk(&blob, b"MAll", |d| {
            d.clear();
            d.extend_from_slice(&hs.to_be_bytes());
            d.extend_from_slice(&1u32.to_be_bytes());
            d.extend_from_slice(&hs.to_be_bytes());
            d.extend_from_slice(&0x1000u32.to_be_bytes()); // block overruns memsize
        });
        assert!(m.restore_state(&past_end).is_err(), "a block past memsize must be a BadSave");

        // (c) heap_start below the ENDMEM floor (would truncate RAM on @mfree).
        let low = corrupt_chunk(&blob, b"MAll", |d| {
            d.clear();
            d.extend_from_slice(&0x40u32.to_be_bytes()); // heap_start below RAMSTART
            d.extend_from_slice(&1u32.to_be_bytes());
            d.extend_from_slice(&0x40u32.to_be_bytes());
            d.extend_from_slice(&0x10u32.to_be_bytes());
        });
        assert!(m.restore_state(&low).is_err(), "heap_start below ENDMEM must be a BadSave");
    }

    #[test]
    fn loader_rejects_absurd_header_resource_requests() {
        // SQ-0624: a 36-byte file could demand ~4 GiB of zeroed ENDMEM and
        // another ~4 GiB of stack before the first opcode ran.
        let huge_mem = asm::image_with_map(0x100, 0x200, 0xFFFF_FF00, 0x1000, 0x124, 0);
        assert!(Memory::new(huge_mem).is_err(), "ENDMEM beyond the cap must fail the load");
        let huge_stack = asm::image_with_map(0x100, 0x200, 0x300, 0xFFFF_FF00, 0x124, 0);
        assert!(Memory::new(huge_stack).is_err(), "a huge stack request must fail the load");
        // The caps must not reject a normal story.
        assert!(Memory::new(asm::image_with_map(0x100, 0x200, 0x300, 0x1000, 0x124, 0)).is_ok());
    }

    #[test]
    fn get_buffer_stream_past_end_cursor_reads_eof() {
        // SQ-0625: writes past a memory stream's end advance its cursor beyond
        // len (Glk §5.3); glk_get_buffer_stream[_uni] then computed `len - pos`
        // — a debug subtract-overflow (release: a huge wrapped count clamped to
        // maxlen, copying from past the declared buffer).
        let mut m = resource_machine(glk::TestBackend::new(), 0x400);
        let sid = m.glk_dispatch(0x0043, &[0x300, 4, 1, 0]).unwrap(); // byte stream, len 4
        assert_ne!(sid, 0);
        for _ in 0..6 {
            m.glk_dispatch(0x0081, &[sid, b'x' as u32]).unwrap(); // put_char_stream → pos ends at 6
        }
        assert_eq!(m.glk_dispatch(0x0092, &[sid, 0x380, 8]).unwrap(), 0, "past-end cursor reads EOF");
        assert_eq!(m.glk_dispatch(0x0131, &[sid, 0x380, 8]).unwrap(), 0, "uni variant guards too");
    }

    #[test]
    fn filter_function_fault_halts_instead_of_being_swallowed() {
        // SQ-0625: a fault inside the filter-iosys function was discarded
        // (`let _ =`), leaving the machine executing inside the aborted
        // callee's frame as if nothing happened.
        let body = [
            asm::ins(0x149, &[asm::Op::C8(1), asm::Op::C32(0x0100)]), // setiosys(filter, rock→RAM, not a function)
            asm::ins(0x70, &[asm::Op::C8(b'A' as i8)]),               // streamchar 'A' → filter call faults
            asm::ins(0x120, &[]),                                     // quit (must not be reached cleanly)
        ]
        .concat();
        let mut m = machine_with_body(&[], body);
        assert_eq!(m.step(), StepResult::Continue, "setiosys");
        assert_eq!(m.step(), StepResult::Quit, "the filter fault surfaces as this instruction's fault");
        assert!(m.halted);
        assert!(
            m.diagnostics.iter().any(|d| d.contains("not a function")),
            "the underlying fault is reported: {:?}",
            m.diagnostics
        );
    }

    #[test]
    fn numtod_dtonumz_round_trip_through_the_stack() {
        // SQ-0626 (verification): spec §2.13 — reads take the high word first,
        // stores write the LOW word first ("stored as S2:S1"), so numtod's two
        // pushes leave the high word on top, exactly where dtonumz's first
        // (high-word) pop looks. The asymmetry is what makes this round-trip.
        let n = -123_456_789i32;
        let mut body = asm::ins(0x200, &[asm::Op::C32(n as u32), asm::Op::Stack, asm::Op::Stack]);
        body.extend(asm::ins(0x201, &[asm::Op::Stack, asm::Op::Stack, asm::Op::Mem16(0x0100)]));
        body.extend(asm::ins(0x120, &[]));
        let mut m = machine_with_body(&[], body);
        m.step_once().unwrap();
        m.step_once().unwrap();
        assert_eq!(m.mem.read32(0x100).unwrap() as i32, n, "numtod → dtonumz is the identity");
    }

    #[test]
    fn datetime_selector_names_match_the_dispatch_table() {
        // SQ-0627: the trace-name table was shifted one selector against the
        // dispatch arms (authoritative ids: cheapglk gi_dispa.c, 0x0168..0x016F).
        assert_eq!(glk_selector_name(0x0168), "glk_time_to_date_utc");
        assert_eq!(glk_selector_name(0x0169), "glk_time_to_date_local");
        assert_eq!(glk_selector_name(0x016A), "glk_simple_time_to_date_utc");
        assert_eq!(glk_selector_name(0x016B), "glk_simple_time_to_date_local");
        assert_eq!(glk_selector_name(0x016C), "glk_date_to_time_utc");
        assert_eq!(glk_selector_name(0x016D), "glk_date_to_time_local");
        assert_eq!(glk_selector_name(0x016E), "glk_date_to_simple_time_utc");
        assert_eq!(glk_selector_name(0x016F), "glk_date_to_simple_time_local");
    }

    #[test]
    fn restart_preserves_borderless_and_closes_backend_windows() {
        // SQ-0627: @restart swapped in a fresh Model with no backend
        // window_close notifications (per-id backend state would splice into
        // the restarted game's colliding fresh ids) and silently dropped the
        // per-game borderless mode (a host/runtime setting, not game state).
        let mut m = machine_with_glk(&[]);
        m.set_borderless(true);
        let win = m.glk_open_window(0, 0, 0, 3, 0);
        assert_ne!(win, 0);
        m.glk_dispatch(0x002F, &[win]).unwrap(); // set_window
        m.glk_dispatch(0x0080, &[b'A' as u32]).unwrap(); // put_char
        assert_eq!(backend_of(&m).runs(win).len(), 1, "backend holds the old window's text");

        m.op_restart().unwrap();
        assert!(m.glk.borderless(), "borderless survives @restart");
        assert_eq!(m.glk.root(), 0, "the model itself is fresh");
        assert!(backend_of(&m).runs(win).is_empty(), "the backend was told the old window closed");
    }

    #[test]
    fn restore_state_preserves_borderless() {
        // SQ-0627: restore_state replaces the Glk model with the deserialized
        // snapshot, which deliberately never carries the host's borderless
        // mode — the LIVE value must survive the swap. @restoreundo runs
        // through this path with no host callback to re-apply it.
        let mut m = machine_with_glk(&[]);
        m.set_borderless(true);
        let blob = m.save_state();
        m.restore_state(&blob).unwrap();
        assert!(m.glk.borderless(), "borderless survives restore_state/@restoreundo");
    }

    #[test]
    fn restore_state_abandons_a_suspended_game_restore() {
        // SQ-0656: a HOST Save State restore performed while the game's OWN
        // @restore was suspended left `pending_saveload` set. The state it
        // described was gone, but the next `step` re-reported RestoreRequest
        // (the host's restore dialog reopening from nowhere) and cancelling it
        // stored the failure code through the abandoned dest record into the
        // freshly restored state.
        let body = [
            asm::ins(0x0124, &[asm::Op::Zero, asm::Op::Mem32(0x110)]), // @restore -> S1 = *0x110
            asm::ins(0x120, &[]),                                      // quit
        ]
        .concat();
        let mut m = machine_with_body(&[], body);
        // A clean host snapshot, taken before the @restore opcode fires.
        let blob = m.save_state();
        assert_eq!(m.step(), StepResult::RestoreRequest);
        assert!(m.is_saveload_pending(), "the game's own @restore is suspended");

        m.restore_state(&blob).expect("our own snapshot restores");
        assert!(
            !m.is_saveload_pending(),
            "a host restore must abandon the suspended @restore — the state it belonged to is gone",
        );

        // Cancelling the dialog that is no longer open must not write through the
        // abandoned dest record into the state we just installed.
        m.complete_restore_failure();
        assert_eq!(
            m.mem.read32(0x110),
            Some(0),
            "no stale store into the restored state (1 = the abandoned @restore's failure code)",
        );
    }

    #[test]
    fn restore_state_repoints_the_suspension_at_the_restored_request() {
        // SQ-0656: the snapshot carries the game's input REQUESTS but not the
        // interpreter's `glk_select` suspension. Restoring a snapshot captured at
        // a "press any key" prompt while the live session sat at a LINE prompt
        // left the suspension in line mode: the host showed an input bar and the
        // game got a LineInput event it never asked for.
        let mut m = machine_with_glk(&[]);
        let win = m.glk_open_window(0, 0, 0, 3, 0); // wintype_TextBuffer
        assert_ne!(win, 0);

        // Park the game on a CHAR request and snapshot it there.
        assert!(m.glk.request_char_event(win, false));
        m.glk_select(0x140).expect("select suspends on the char request");
        assert!(matches!(m.step(), StepResult::NeedChar { .. }), "snapshot taken at a char prompt");
        let blob = m.save_state();

        // Move the LIVE session to a line prompt instead.
        m.supply_char('x' as u32);
        assert!(m.glk.request_line_event(win, 0x180, 16, 0, false));
        m.glk_select(0x140).expect("select suspends on the line request");
        assert!(matches!(m.step(), StepResult::NeedLine { .. }), "live session is at a line prompt");

        m.restore_state(&blob).expect("our own snapshot restores");
        assert!(
            matches!(m.step(), StepResult::NeedChar { .. }),
            "the restored state wants a KEY, so the suspension must too (not the stale line mode)",
        );
    }

    #[test]
    fn ram_relative_operand_overflow_is_not_a_panic() {
        // SQ-0627: mode 0xD/0xE/0xF added the operand to RAMSTART unwrapped —
        // a debug overflow panic on a 32-bit wrap. It must wrap (32-bit address
        // arithmetic, as glulxe's C uint32 does) and take the normal
        // bounds-check/ROM-write path.
        let body = [
            asm::ins(0x40, &[asm::Op::Ram32(0xFFFF_FFFF), asm::Op::Zero]), // load: wraps low
            asm::ins(0x40, &[asm::Op::C32(7), asm::Op::Ram32(0xFFFF_FFFF)]), // store: wraps into ROM
            asm::ins(0x120, &[]),
        ]
        .concat();
        let mut m = machine_with_body(&[], body);
        for _ in 0..8 {
            if m.step() == StepResult::Quit {
                break;
            }
        }
        assert!(m.halted, "the program must fault or quit cleanly, never panic");
    }
}
