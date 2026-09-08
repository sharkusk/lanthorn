//! Glulx disassembler + lazy discovery cache for the debug inspector (SQ-0465).
//!
//! Built on the shared `crate::decode` primitives (same opcode-number decode
//! the interpreter uses). Discovery is cheap and eager (function headers + call
//! graph + a type-validated linear scan of ROM); *rendering* disassembly text is
//! lazy and windowed — an I7 image is multi-MB, so we never format the whole
//! thing, only the address window the inspector asks for.
//!
//! Confidence tiers ([`Tier`], mirroring zvm's `Provenance`):
//!  * `Rd`   — reached from the start function via the constant call graph.
//!  * `Soft` — found only by the type-validated linear scan (an unverified guess).
//!  * `Data` — string/table bytes (E0/E1/E2 objects and gaps), never code.
//!
//! RAM-resident code (legal but rare) is not scanned statically; it is surfaced
//! only when the app seeds executed PCs (see [`DisasmCache::seed_executed`] /
//! [`confirm_pc`](DisasmCache::confirm_pc)), matching zvm's execution-as-truth.

use crate::decode::{decode_opcode, operand_data_len};
use crate::exec::R;
use crate::memory::Memory;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Static confidence tier of a byte range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// Reached from the start function / constant call graph (hard).
    Rd,
    /// Found only by the type-validated linear scan (soft guess).
    Soft,
    /// String/table/data bytes — never code.
    Data,
}

// ─── opcode metadata table ────────────────────────────────────────────────────

/// Role-shaping of an opcode's operand list, for target/annotation resolution.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Special {
    /// Plain: loads first, then stores; no branch/call target.
    None,
    /// Conditional/unconditional relative branch: the last load is the offset.
    Branch,
    /// A call: operand 0 (a load) is the function target.
    Call,
    /// `jumpabs`: operand 0 is an absolute target (no `+off-2` bias).
    JumpAbs,
    /// `glk`: operand 0 is the selector (named when constant).
    Glk,
    /// `streamstr`: operand 0 is a string/callable address (previewed).
    StreamStr,
    /// `catch S1 L1`: operand 0 is the *store*, operand 1 the branch offset.
    Catch,
}

/// Static shape of one opcode: mnemonic, load/store operand counts, role.
#[derive(Clone, Copy)]
struct OpInfo {
    mnemonic: &'static str,
    n_load: u8,
    n_store: u8,
    special: Special,
}

impl OpInfo {
    #[inline]
    fn total(&self) -> usize {
        (self.n_load + self.n_store) as usize
    }
    /// Is operand `i` (encoding order) a store destination?
    #[inline]
    fn is_store(&self, i: usize) -> bool {
        match self.special {
            Special::Catch => i == 0, // catch S1 L1: store first
            _ => i >= self.n_load as usize,
        }
    }
    /// Index of the branch-offset operand, if this opcode branches.
    #[inline]
    fn branch_index(&self) -> Option<usize> {
        match self.special {
            Special::Branch => Some(self.n_load as usize - 1), // last load
            Special::Catch => Some(1),
            _ => None,
        }
    }
}

macro_rules! op {
    ($mn:literal, $l:literal, $s:literal) => {
        OpInfo { mnemonic: $mn, n_load: $l, n_store: $s, special: Special::None }
    };
    ($mn:literal, $l:literal, $s:literal, $sp:expr) => {
        OpInfo { mnemonic: $mn, n_load: $l, n_store: $s, special: $sp }
    };
}

/// The Glulx opcode table, transcribed from `exec::Machine::execute` (the
/// authority). Operand counts are the `read_operands(n_load, n_store)` shapes;
/// `special` marks call/branch/glk/string roles for annotation.
fn opcode_info(opcode: u32) -> Option<OpInfo> {
    use Special::*;
    Some(match opcode {
        0x00 => op!("nop", 0, 0),
        // integer arithmetic
        0x10 => op!("add", 2, 1),
        0x11 => op!("sub", 2, 1),
        0x12 => op!("mul", 2, 1),
        0x13 => op!("div", 2, 1),
        0x14 => op!("mod", 2, 1),
        0x15 => op!("neg", 1, 1),
        0x18 => op!("bitand", 2, 1),
        0x19 => op!("bitor", 2, 1),
        0x1A => op!("bitxor", 2, 1),
        0x1B => op!("bitnot", 1, 1),
        0x1C => op!("shiftl", 2, 1),
        0x1D => op!("sshiftr", 2, 1),
        0x1E => op!("ushiftr", 2, 1),
        // branches
        0x20 => op!("jump", 1, 0, Branch),
        0x22 => op!("jz", 2, 0, Branch),
        0x23 => op!("jnz", 2, 0, Branch),
        0x24 => op!("jeq", 3, 0, Branch),
        0x25 => op!("jne", 3, 0, Branch),
        0x26 => op!("jlt", 3, 0, Branch),
        0x27 => op!("jge", 3, 0, Branch),
        0x28 => op!("jgt", 3, 0, Branch),
        0x29 => op!("jle", 3, 0, Branch),
        0x2A => op!("jltu", 3, 0, Branch),
        0x2B => op!("jgeu", 3, 0, Branch),
        0x2C => op!("jgtu", 3, 0, Branch),
        0x2D => op!("jleu", 3, 0, Branch),
        // calls / return
        0x30 => op!("call", 2, 1, Call),
        0x31 => op!("return", 1, 0),
        0x32 => op!("catch", 1, 1, Catch),
        0x33 => op!("throw", 2, 0),
        0x34 => op!("tailcall", 2, 0, Call),
        // copy / sign-extend
        0x40 => op!("copy", 1, 1),
        0x41 => op!("copys", 1, 1),
        0x42 => op!("copyb", 1, 1),
        0x44 => op!("sexs", 1, 1),
        0x45 => op!("sexb", 1, 1),
        // memory-array load/store
        0x48 => op!("aload", 2, 1),
        0x49 => op!("aloads", 2, 1),
        0x4A => op!("aloadb", 2, 1),
        0x4B => op!("aloadbit", 2, 1),
        0x4C => op!("astore", 3, 0),
        0x4D => op!("astores", 3, 0),
        0x4E => op!("astoreb", 3, 0),
        0x4F => op!("astorebit", 3, 0),
        // stack
        0x50 => op!("stkcount", 0, 1),
        0x51 => op!("stkpeek", 1, 1),
        0x52 => op!("stkswap", 0, 0),
        0x53 => op!("stkroll", 2, 0),
        0x54 => op!("stkcopy", 1, 0),
        // stream output
        0x70 => op!("streamchar", 1, 0),
        0x71 => op!("streamnum", 1, 0),
        0x72 => op!("streamstr", 1, 0, StreamStr),
        0x73 => op!("streamunichar", 1, 0),
        // machine / memory
        0x100 => op!("gestalt", 2, 1),
        0x101 => op!("debugtrap", 1, 0),
        0x102 => op!("getmemsize", 0, 1),
        0x103 => op!("setmemsize", 1, 1),
        0x104 => op!("jumpabs", 1, 0, JumpAbs),
        0x110 => op!("random", 1, 1),
        0x111 => op!("setrandom", 1, 0),
        0x120 => op!("quit", 0, 0),
        0x121 => op!("verify", 0, 1),
        0x122 => op!("restart", 0, 0),
        0x123 => op!("save", 1, 1),
        0x124 => op!("restore", 1, 1),
        0x125 => op!("saveundo", 0, 1),
        0x126 => op!("restoreundo", 0, 1),
        0x127 => op!("protect", 2, 0),
        0x128 => op!("hasundo", 0, 1),
        0x129 => op!("discardundo", 0, 0),
        0x130 => op!("glk", 2, 1, Glk),
        0x140 => op!("getstringtbl", 0, 1),
        0x141 => op!("setstringtbl", 1, 0),
        0x148 => op!("getiosys", 0, 2),
        0x149 => op!("setiosys", 2, 0),
        0x150 => op!("linearsearch", 7, 1),
        0x151 => op!("binarysearch", 7, 1),
        0x152 => op!("linkedsearch", 6, 1),
        // call-with-fixed-args
        0x160 => op!("callf", 1, 1, Call),
        0x161 => op!("callfi", 2, 1, Call),
        0x162 => op!("callfii", 3, 1, Call),
        0x163 => op!("callfiii", 4, 1, Call),
        // block copy / heap / accel
        0x170 => op!("mzero", 2, 0),
        0x171 => op!("mcopy", 3, 0),
        0x178 => op!("malloc", 1, 1),
        0x179 => op!("mfree", 1, 0),
        0x180 => op!("accelfunc", 2, 0),
        0x181 => op!("accelparam", 2, 0),
        // single-precision float
        0x190 => op!("numtof", 1, 1),
        0x191 => op!("ftonumz", 1, 1),
        0x192 => op!("ftonumn", 1, 1),
        0x198 => op!("ceil", 1, 1),
        0x199 => op!("floor", 1, 1),
        0x1A0 => op!("fadd", 2, 1),
        0x1A1 => op!("fsub", 2, 1),
        0x1A2 => op!("fmul", 2, 1),
        0x1A3 => op!("fdiv", 2, 1),
        0x1A4 => op!("fmod", 2, 2),
        0x1A8 => op!("sqrt", 1, 1),
        0x1A9 => op!("exp", 1, 1),
        0x1AA => op!("log", 1, 1),
        0x1AB => op!("pow", 2, 1),
        0x1B0 => op!("sin", 1, 1),
        0x1B1 => op!("cos", 1, 1),
        0x1B2 => op!("tan", 1, 1),
        0x1B3 => op!("asin", 1, 1),
        0x1B4 => op!("acos", 1, 1),
        0x1B5 => op!("atan", 1, 1),
        0x1B6 => op!("atan2", 2, 1),
        0x1C0 => op!("jfeq", 4, 0, Branch),
        0x1C1 => op!("jfne", 4, 0, Branch),
        0x1C2 => op!("jflt", 3, 0, Branch),
        0x1C3 => op!("jfle", 3, 0, Branch),
        0x1C4 => op!("jfgt", 3, 0, Branch),
        0x1C5 => op!("jfge", 3, 0, Branch),
        0x1C8 => op!("jisnan", 2, 0, Branch),
        0x1C9 => op!("jisinf", 2, 0, Branch),
        // double-precision float
        0x200 => op!("numtod", 1, 2),
        0x201 => op!("dtonumz", 2, 1),
        0x202 => op!("dtonumn", 2, 1),
        0x203 => op!("ftod", 1, 2),
        0x204 => op!("dtof", 2, 1),
        0x208 => op!("dceil", 2, 2),
        0x209 => op!("dfloor", 2, 2),
        0x210 => op!("dadd", 4, 2),
        0x211 => op!("dsub", 4, 2),
        0x212 => op!("dmul", 4, 2),
        0x213 => op!("ddiv", 4, 2),
        0x214 => op!("dmodr", 4, 2),
        0x215 => op!("dmodq", 4, 2),
        0x218 => op!("dsqrt", 2, 2),
        0x219 => op!("dexp", 2, 2),
        0x21A => op!("dlog", 2, 2),
        0x21B => op!("dpow", 4, 2),
        0x220 => op!("dsin", 2, 2),
        0x221 => op!("dcos", 2, 2),
        0x222 => op!("dtan", 2, 2),
        0x223 => op!("dasin", 2, 2),
        0x224 => op!("dacos", 2, 2),
        0x225 => op!("datan", 2, 2),
        0x226 => op!("datan2", 4, 2),
        0x230 => op!("jdeq", 7, 0, Branch),
        0x231 => op!("jdne", 7, 0, Branch),
        0x232 => op!("jdlt", 5, 0, Branch),
        0x233 => op!("jdle", 5, 0, Branch),
        0x234 => op!("jdgt", 5, 0, Branch),
        0x235 => op!("jdge", 5, 0, Branch),
        0x238 => op!("jdisnan", 3, 0, Branch),
        0x239 => op!("jdisinf", 3, 0, Branch),
        _ => return Option::None,
    })
}

/// Structured per-opcode help for the debug inspector's instruction hover
/// tooltip, keyed by opcode NUMBER (Glulx opcode numbers are already
/// encoding-unambiguous, unlike Z-machine mnemonics which need a version to
/// resolve — see `zvm::cpu::opcode_help`, whose depth this mirrors). Grounded
/// in the Glulx Specification AS IMPLEMENTED by `exec::Machine::execute` —
/// where the two disagree, the executor wins (see the discrepancy notes on
/// the double-precision (`0x200..=0x239`) entries below).
pub struct OpcodeHelp {
    /// One-sentence description of what the opcode does. May reference
    /// operand role names (`a`, `b`, …) in prose, matching `roles` below.
    pub summary: &'static str,
    /// Role text for each operand, in ENCODING order — index `i` here
    /// describes `ins.operands[i]` (parallel to `opcode_info`/`decode_instr`'s
    /// operand list). A load operand's text reads standalone ("the addend");
    /// a store operand's text reads after "stores" ("the sum a + b"). The
    /// branch-offset operand of a branching/`catch` opcode (always the LAST
    /// operand — see `OpInfo::branch_index`) has no entry here; its phrasing
    /// comes from `branch` (or, for `catch`, fixed text in `describe`).
    pub roles: &'static [&'static str],
    /// Branch condition, phrased to read after "branches if" — opcodes with
    /// `Special::Branch` only. `None` means an unconditional jump (`jump`);
    /// unused by `Special::Catch`, whose jump is always unconditional.
    pub branch: Option<&'static str>,
}

macro_rules! h {
    ($summary:literal) => {
        OpcodeHelp { summary: $summary, roles: &[], branch: None }
    };
    ($summary:literal, $roles:expr) => {
        OpcodeHelp { summary: $summary, roles: $roles, branch: None }
    };
    ($summary:literal, $roles:expr, branch: $branch:literal) => {
        OpcodeHelp { summary: $summary, roles: $roles, branch: Some($branch) }
    };
}

/// Per-opcode help, keyed by opcode number. The coverage test
/// `opcode_help_covers_every_implemented_opcode` asserts every opcode
/// `opcode_info` recognizes — itself transcribed from, and round-trip tested
/// against, `exec::Machine::execute` (see `decode_round_trips_with_executor` /
/// `real_image_decode_round_trip` below) — has an entry here, so the tables
/// can't silently drift apart.
pub fn opcode_help(opcode: u32) -> Option<OpcodeHelp> {
    Some(match opcode {
        // ── integer arithmetic ──────────────────────────────────────────────
        0x00 => h!("Do nothing."),
        0x10 => h!("Add two 32-bit values.", &["a", "b", "a + b, with 32-bit wraparound"]),
        0x11 => h!("Subtract two 32-bit values.", &["a", "b", "a − b, with 32-bit wraparound"]),
        0x12 => h!("Multiply two 32-bit values.", &["a", "b", "a × b, the low 32 bits of the product"]),
        0x13 => h!(
            "Signed integer division, truncated toward zero (a zero divisor faults the interpreter).",
            &["dividend", "divisor (nonzero)", "the quotient"]
        ),
        0x14 => h!(
            "Signed integer remainder (a zero divisor faults the interpreter).",
            &["dividend", "divisor (nonzero)", "the remainder"]
        ),
        0x15 => h!("Negate a 32-bit value.", &["a", "−a"]),
        0x18 => h!("Bitwise AND.", &["a", "b", "a & b"]),
        0x19 => h!("Bitwise OR.", &["a", "b", "a | b"]),
        0x1A => h!("Bitwise XOR.", &["a", "b", "a ^ b"]),
        0x1B => h!("Bitwise complement (one's complement NOT).", &["a", "~a"]),
        0x1C => h!(
            "Shift left, zero-filling.",
            &["value", "shift amount", "value shifted left (0 if the shift is ≥ 32)"]
        ),
        0x1D => h!(
            "Arithmetic (sign-preserving) shift right.",
            &["value", "shift amount", "value shifted right, sign-extending (0/-1 fill if the shift is ≥ 32)"]
        ),
        0x1E => h!(
            "Logical (zero-filling) shift right.",
            &["value", "shift amount", "value shifted right, zero-filling (0 if the shift is ≥ 32)"]
        ),
        // ── branches ─────────────────────────────────────────────────────────
        0x20 => h!("Jump unconditionally by a relative offset."),
        0x22 => h!("Branch if a value is zero.", &["value"], branch: "value == 0"),
        0x23 => h!("Branch if a value is nonzero.", &["value"], branch: "value != 0"),
        0x24 => h!("Branch if two values are equal.", &["a", "b"], branch: "a == b"),
        0x25 => h!("Branch if two values differ.", &["a", "b"], branch: "a != b"),
        0x26 => h!("Branch if a is less than b (signed).", &["a", "b"], branch: "a < b (signed 32-bit compare)"),
        0x27 => h!("Branch if a is at least b (signed).", &["a", "b"], branch: "a >= b (signed 32-bit compare)"),
        0x28 => h!("Branch if a is greater than b (signed).", &["a", "b"], branch: "a > b (signed 32-bit compare)"),
        0x29 => h!("Branch if a is at most b (signed).", &["a", "b"], branch: "a <= b (signed 32-bit compare)"),
        0x2A => h!("Branch if a is less than b, an UNSIGNED comparison.", &["a", "b"], branch: "a < b (UNSIGNED 32-bit compare)"),
        0x2B => h!("Branch if a is at least b, an UNSIGNED comparison.", &["a", "b"], branch: "a >= b (UNSIGNED 32-bit compare)"),
        0x2C => h!("Branch if a is greater than b, an UNSIGNED comparison.", &["a", "b"], branch: "a > b (UNSIGNED 32-bit compare)"),
        0x2D => h!("Branch if a is at most b, an UNSIGNED comparison.", &["a", "b"], branch: "a <= b (UNSIGNED 32-bit compare)"),
        // ── calls / return ───────────────────────────────────────────────────
        0x30 => h!(
            "Call a function.",
            &["the function to call", "the number of stack values to pop as arguments", "the function's return value"]
        ),
        0x31 => h!("Return from the current function to its caller.", &["the value to return"]),
        0x32 => h!(
            "Push a call-frame token (usable by @throw to unwind back here), then unconditionally jump — a landing pad @throw resumes at.",
            &["a token identifying this call frame, for a later @throw"]
        ),
        0x33 => h!(
            "Unwind the stack back to a @catch point and resume there as if it had just returned this value.",
            &["the value to deliver to the catch point", "the frame token captured by a matching @catch"]
        ),
        0x34 => h!(
            "Replace the current call frame with a call to another function (a true tail call — no stack growth).",
            &["the function to call", "the number of stack values to pop as arguments"]
        ),
        // ── copy / sign-extend ───────────────────────────────────────────────
        0x40 => h!("Copy a 32-bit value.", &["the value", "a copy of the value"]),
        0x41 => h!(
            "Copy the low 16 bits of a value.",
            &["the value", "the value's low 16 bits, zero-extended (not sign-extended)"]
        ),
        0x42 => h!(
            "Copy the low 8 bits of a value.",
            &["the value", "the value's low 8 bits, zero-extended (not sign-extended)"]
        ),
        0x44 => h!(
            "Sign-extend a 16-bit value to 32 bits.",
            &["the value", "the value's low 16 bits, sign-extended"]
        ),
        0x45 => h!(
            "Sign-extend an 8-bit value to 32 bits.",
            &["the value", "the value's low 8 bits, sign-extended"]
        ),
        // ── memory-array load/store ──────────────────────────────────────────
        0x48 => h!(
            "Load a 32-bit word from a word-indexed array.",
            &["array base address", "word index (signed; 4-byte elements)", "the word at base + 4×index"]
        ),
        0x49 => h!(
            "Load a 16-bit value from a halfword-indexed array.",
            &["array base address", "index (signed; 2-byte elements)", "the 16-bit value at base + 2×index, zero-extended"]
        ),
        0x4A => h!(
            "Load a byte from a byte-indexed array.",
            &["array base address", "index (signed; 1-byte elements)", "the byte at base + index, zero-extended"]
        ),
        0x4B => h!(
            "Load one bit from a bit array.",
            &["bit-array base address", "bit index (signed — negative indexes extend backward from the base)", "the bit's value, 0 or 1"]
        ),
        0x4C => h!(
            "Store a 32-bit word into a word-indexed array.",
            &["array base address", "word index (signed; 4-byte elements)", "value to write at base + 4×index"]
        ),
        0x4D => h!(
            "Store a 16-bit value into a halfword-indexed array.",
            &["array base address", "index (signed; 2-byte elements)", "16-bit value to write at base + 2×index"]
        ),
        0x4E => h!(
            "Store a byte into a byte-indexed array.",
            &["array base address", "index (signed; 1-byte elements)", "byte value to write at base + index"]
        ),
        0x4F => h!(
            "Set or clear one bit in a bit array.",
            &["bit-array base address", "bit index (signed)", "new bit value — 0 clears it, any nonzero value sets it"]
        ),
        // ── stack manipulation ───────────────────────────────────────────────
        0x50 => h!("Count the values on the stack.", &["the number of 32-bit values currently on the stack"]),
        0x51 => h!(
            "Peek at a stack value without removing it.",
            &["depth from the top of the stack (0 = the top value)", "the value at that depth"]
        ),
        0x52 => h!("Swap the top two values on the stack."),
        0x53 => h!(
            "Rotate the top N stack values.",
            &["number of values to rotate (counting from the top)", "rotate amount (positive rotates toward the top; wraps modulo the count)"]
        ),
        0x54 => h!(
            "Push a duplicate of the top N stack values, in their original order.",
            &["number of values to duplicate (counting from the top)"]
        ),
        // ── stream output ────────────────────────────────────────────────────
        0x70 => h!("Print one character to the current output stream.", &["character code (its low byte prints as Latin-1)"]),
        0x71 => h!("Print a signed decimal number to the current output stream.", &["the number to print"]),
        0x72 => h!(
            "Print a string (E0/E1/E2 object) — or invoke a function-as-string-source (C0/C1 object) — to the current output stream.",
            &["the string or function object to print"]
        ),
        0x73 => h!("Print one Unicode character to the current output stream.", &["the code point to print (U+FFFD if invalid)"]),
        // ── machine / memory ─────────────────────────────────────────────────
        0x100 => h!(
            "Query an interpreter capability or limit.",
            &["gestalt selector number", "a selector-specific argument (often unused)", "the query result — meaning depends on the selector"]
        ),
        0x101 => h!(
            "Signal a debug trap. This interpreter has no attached debugger to trap into — it logs the code and continues.",
            &["a diagnostic code, logged verbatim"]
        ),
        0x102 => h!("Read the current memory size.", &["the current extent of memory, in bytes"]),
        0x103 => h!(
            "Resize memory (always fails while the acceleration heap is active).",
            &["the new memory size, in bytes", "the result: 0 on success, nonzero on failure"]
        ),
        0x104 => h!("Jump to an absolute byte address — NOT the 0/1 return shortcut other branches use.", &["the absolute address to jump to"]),
        0x110 => h!(
            "Draw a random number.",
            &["range: 0 = any 32-bit value; N>0 = uniform in [0,N); N<0 = uniform in (N,0]", "the random value"]
        ),
        0x111 => h!(
            "(Re)seed the random-number generator.",
            &["seed (0 reseeds unpredictably from OS entropy; nonzero seeds deterministically)"]
        ),
        0x120 => h!("Stop the program."),
        0x121 => h!("Check the story file's checksum against its header.", &["0 if the checksum matches, nonzero if it doesn't"]),
        0x122 => h!("Reset all state to the story's initial condition and re-enter the start function (the Glk display is not cleared)."),
        0x123 => h!(
            "Suspend for the host to write a standard Glulx-Quetzal save file, then resume.",
            &["a Glk stream open on the destination save file", "the result: 0 on success, 1 on failure"]
        ),
        0x124 => h!(
            "Suspend for the host to read a save file. On success, resumes inside the matching @save's call stub instead (which then stores -1); on failure, stores here.",
            &["a Glk stream open on the source save file", "the result on failure: 1 (never written on success)"]
        ),
        0x125 => h!(
            "Snapshot the VM state for a later @restoreundo. On a later restore, this same destination receives -1 instead.",
            &["the result: 0 on success"]
        ),
        0x126 => h!(
            "Restore the most recent @saveundo snapshot. On success, execution resumes at the matching @saveundo (which then receives -1) — this destination is never written.",
            &["the result on failure (no snapshot to restore): 1"]
        ),
        0x127 => h!(
            "Protect a RAM range from being overwritten by @restore/@restoreundo.",
            &["start address of the range to protect", "length in bytes (0 clears the protection)"]
        ),
        0x128 => h!("Check whether an undo snapshot is available.", &["0 if a @restoreundo would succeed right now, nonzero if none is saved"]),
        0x129 => h!("Discard the most recently saved undo snapshot (a no-op if there is none)."),
        0x130 => h!(
            "Call a Glk API function.",
            &["the Glk selector number to call", "the number of stack values to pop as arguments", "the call's return value"]
        ),
        0x140 => h!("Read the current string-decoding table.", &["address of the current Huffman table (0 = the built-in default)"]),
        0x141 => h!("Set the string-decoding table.", &["address of the new table (0 selects the built-in default)"]),
        0x148 => h!(
            "Read the current I/O system.",
            &["current I/O system mode (0 = null, 1 = filter, 2 = Glk)", "current I/O system rock value"]
        ),
        0x149 => h!(
            "Select the I/O system output goes through.",
            &["mode to select (0 = null — discard output; 1 = filter — call a function; 2 = Glk — normal output)", "rock (in filter mode, the filter function's address)"]
        ),
        0x150 => h!(
            "Linearly search an array of fixed-size records for one with a matching key.",
            &[
                "the search key", "key size, in bytes", "start address of the array", "size of each record, in bytes",
                "number of records to scan (0xffffffff = unbounded)", "byte offset of the key field within each record",
                "option flags (bit0 = return the index instead of the address; bit1 = stop at an all-zero key)",
                "the matching record's address (or index); 0 (or -1) if not found",
            ]
        ),
        0x151 => h!(
            "Binary-search a key-sorted array of fixed-size records for one with a matching key.",
            &[
                "the search key", "key size, in bytes", "start address of the (key-sorted) array", "size of each record, in bytes",
                "number of records", "byte offset of the key field within each record",
                "option flags (bit0 = return the index instead of the address)",
                "the matching record's address (or index); 0 (or -1) if not found",
            ]
        ),
        0x152 => h!(
            "Search a singly-linked list of nodes for one with a matching key.",
            &[
                "the search key", "key size, in bytes", "address of the first node",
                "byte offset of the key field within each node", "byte offset of the 'next node' pointer within each node",
                "option flags (bit1 = stop at an all-zero key)", "the matching node's address; 0 if not found",
            ]
        ),
        // ── call-with-fixed-args ─────────────────────────────────────────────
        0x160 => h!("Call a function with no arguments.", &["the function to call", "the function's return value"]),
        0x161 => h!(
            "Call a function with 1 fixed argument.",
            &["the function to call", "argument 1", "the function's return value"]
        ),
        0x162 => h!(
            "Call a function with 2 fixed arguments.",
            &["the function to call", "argument 1", "argument 2", "the function's return value"]
        ),
        0x163 => h!(
            "Call a function with 3 fixed arguments.",
            &["the function to call", "argument 1", "argument 2", "argument 3", "the function's return value"]
        ),
        // ── block copy / heap / accel ────────────────────────────────────────
        0x170 => h!("Zero a block of memory.", &["number of bytes to zero", "start address"]),
        0x171 => h!("Copy a block of memory (source/destination may overlap).", &["number of bytes to copy", "source address", "destination address"]),
        0x178 => h!("Allocate a block from the acceleration heap.", &["number of bytes to allocate", "address of the new block (0 on failure)"]),
        0x179 => h!("Free a block allocated by @malloc.", &["address of a block returned by @malloc"]),
        0x180 => h!(
            "Mark a function as natively accelerated (or remove that marking) — GLULX_NOTES §17.",
            &["accelerated-function number to assign (0 removes acceleration at the address)", "address of the function to (de)accelerate"]
        ),
        0x181 => h!(
            "Set a parameter used by the accelerated functions.",
            &["acceleration parameter number", "value to assign to that parameter"]
        ),
        // ── single-precision float (GLULX_NOTES §13.1) ───────────────────────
        0x190 => h!("Convert a signed integer to the nearest float32.", &["a signed 32-bit integer", "the value, as a float32 bit pattern"]),
        0x191 => h!(
            "Convert a float32 to an integer, truncating toward zero.",
            &["a float32 bit pattern", "the value as an integer, truncated toward zero (0x7fffffff if NaN)"]
        ),
        0x192 => h!(
            "Convert a float32 to an integer, rounding to nearest (ties away from zero).",
            &["a float32 bit pattern", "the value as an integer, rounded to nearest (0x7fffffff if NaN)"]
        ),
        0x198 => h!("Round a float32 up to the nearest integer value.", &["a float32 bit pattern", "its ceiling, as a float32 bit pattern"]),
        0x199 => h!("Round a float32 down to the nearest integer value.", &["a float32 bit pattern", "its floor, as a float32 bit pattern"]),
        0x1A0 => h!("Add two float32 values.", &["a", "b", "a + b, as a float32 bit pattern"]),
        0x1A1 => h!("Subtract two float32 values.", &["a", "b", "a − b, as a float32 bit pattern"]),
        0x1A2 => h!("Multiply two float32 values.", &["a", "b", "a × b, as a float32 bit pattern"]),
        0x1A3 => h!("Divide two float32 values.", &["a", "b", "a ÷ b, as a float32 bit pattern"]),
        0x1A4 => h!(
            "Float32 division, returning both the remainder and the quotient.",
            &["a (dividend)", "b (divisor)", "the remainder, sign of a, as a float32 bit pattern", "the quotient, truncated toward zero, as a float32 bit pattern"]
        ),
        0x1A8 => h!("Square root of a float32 value.", &["a float32 bit pattern", "its square root, as a float32 bit pattern"]),
        0x1A9 => h!("e raised to a float32 power.", &["a float32 bit pattern", "e^value, as a float32 bit pattern"]),
        0x1AA => h!("Natural logarithm of a float32 value.", &["a float32 bit pattern", "its natural log, as a float32 bit pattern"]),
        0x1AB => h!("Raise a float32 to a float32 power.", &["base", "exponent", "base^exponent, as a float32 bit pattern"]),
        0x1B0 => h!("Sine of a float32 angle in radians.", &["angle in radians (float32)", "its sine, as a float32 bit pattern"]),
        0x1B1 => h!("Cosine of a float32 angle in radians.", &["angle in radians (float32)", "its cosine, as a float32 bit pattern"]),
        0x1B2 => h!("Tangent of a float32 angle in radians.", &["angle in radians (float32)", "its tangent, as a float32 bit pattern"]),
        0x1B3 => h!("Arcsine, result in radians.", &["a float32 bit pattern", "its arcsine in radians, as a float32 bit pattern"]),
        0x1B4 => h!("Arccosine, result in radians.", &["a float32 bit pattern", "its arccosine in radians, as a float32 bit pattern"]),
        0x1B5 => h!("Arctangent, result in radians.", &["a float32 bit pattern", "its arctangent in radians, as a float32 bit pattern"]),
        0x1B6 => h!(
            "Two-argument arctangent (atan2), result in radians.",
            &["y", "x", "atan2(y, x) in radians, as a float32 bit pattern"]
        ),
        // ── float branches ───────────────────────────────────────────────────
        0x1C0 => h!(
            "Branch if two float32 values are equal within a tolerance (\"fuzzy\" equality).",
            &["a", "b", "tolerance"],
            branch: "|a − b| ≤ |tolerance| — false if any operand is NaN, true if a and b are the same infinity"
        ),
        0x1C1 => h!(
            "Branch unless two float32 values are equal within a tolerance — unlike jfeq, this DOES branch on NaN.",
            &["a", "b", "tolerance"],
            branch: "NOT(|a − b| ≤ |tolerance|) — true (branches) if any operand is NaN"
        ),
        0x1C2 => h!("Branch if a is less than b (float32).", &["a", "b"], branch: "a < b"),
        0x1C3 => h!("Branch if a is at most b (float32).", &["a", "b"], branch: "a <= b"),
        0x1C4 => h!("Branch if a is greater than b (float32).", &["a", "b"], branch: "a > b"),
        0x1C5 => h!("Branch if a is at least b (float32).", &["a", "b"], branch: "a >= b"),
        0x1C8 => h!("Branch if a float32 value is NaN.", &["a float32 bit pattern"], branch: "the value is NaN"),
        0x1C9 => h!("Branch if a float32 value is infinite.", &["a float32 bit pattern"], branch: "the value is +/-infinity"),
        // ── double-precision float (GLULX_NOTES §13.1 / spec 3.1.3 §2.13) ────
        // Word order IS the spec's convention (verified against spec §2.13 and
        // glulxe exec.c op_numtod, SQ-0626): "read operands always read the
        // high word first. Write operands always write the LOW word first" —
        // e.g. `dadd Xhi Xlo Yhi Ylo RESlo REShi`. So loads take L1=high,
        // L2=low, while stores write S1=low, S2=high. The asymmetry is
        // deliberate and load-bearing: pushing lo then hi leaves the high word
        // on top, exactly where the next double read's first (high-word) pop
        // looks, so stack round-trips work. Roles below describe this behavior.
        0x200 => h!(
            "Convert a signed integer to the nearest double.",
            &["a signed 32-bit integer", "the double's LOW 32 bits", "the double's HIGH 32 bits"]
        ),
        0x201 => h!(
            "Convert a double to an integer, truncating toward zero.",
            &["HIGH 32 bits of the double", "LOW 32 bits of the double", "the value as an integer, truncated toward zero (0x7fffffff if NaN)"]
        ),
        0x202 => h!(
            "Convert a double to an integer, rounding to nearest (ties away from zero).",
            &["HIGH 32 bits of the double", "LOW 32 bits of the double", "the value as an integer, rounded to nearest (0x7fffffff if NaN)"]
        ),
        0x203 => h!(
            "Widen a float32 to a double (exact — no precision loss).",
            &["a float32 bit pattern", "the double's LOW 32 bits", "the double's HIGH 32 bits"]
        ),
        0x204 => h!(
            "Narrow a double to a float32, rounding to nearest.",
            &["HIGH 32 bits of the double", "LOW 32 bits of the double", "the value, as a float32 bit pattern"]
        ),
        0x208 => h!(
            "Round a double up to the nearest integer value.",
            &["HIGH 32 bits of the operand", "LOW 32 bits of the operand", "LOW 32 bits of the ceiling", "HIGH 32 bits of the ceiling"]
        ),
        0x209 => h!(
            "Round a double down to the nearest integer value.",
            &["HIGH 32 bits of the operand", "LOW 32 bits of the operand", "LOW 32 bits of the floor", "HIGH 32 bits of the floor"]
        ),
        0x210 => h!(
            "Add two doubles.",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b", "LOW 32 bits of a + b", "HIGH 32 bits of a + b"]
        ),
        0x211 => h!(
            "Subtract two doubles.",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b", "LOW 32 bits of a − b", "HIGH 32 bits of a − b"]
        ),
        0x212 => h!(
            "Multiply two doubles.",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b", "LOW 32 bits of a × b", "HIGH 32 bits of a × b"]
        ),
        0x213 => h!(
            "Divide two doubles.",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b", "LOW 32 bits of a ÷ b", "HIGH 32 bits of a ÷ b"]
        ),
        0x214 => h!(
            "Double division, remainder only (sign of the dividend).",
            &[
                "HIGH 32 bits of the dividend", "LOW 32 bits of the dividend", "HIGH 32 bits of the divisor", "LOW 32 bits of the divisor",
                "LOW 32 bits of the remainder", "HIGH 32 bits of the remainder",
            ]
        ),
        0x215 => h!(
            "Double division, quotient only (truncated toward zero).",
            &[
                "HIGH 32 bits of the dividend", "LOW 32 bits of the dividend", "HIGH 32 bits of the divisor", "LOW 32 bits of the divisor",
                "LOW 32 bits of the quotient", "HIGH 32 bits of the quotient",
            ]
        ),
        0x218 => h!(
            "Square root of a double.",
            &["HIGH 32 bits of the operand", "LOW 32 bits of the operand", "LOW 32 bits of the square root", "HIGH 32 bits of the square root"]
        ),
        0x219 => h!(
            "e raised to a double power.",
            &["HIGH 32 bits of the operand", "LOW 32 bits of the operand", "LOW 32 bits of e^value", "HIGH 32 bits of e^value"]
        ),
        0x21A => h!(
            "Natural logarithm of a double.",
            &["HIGH 32 bits of the operand", "LOW 32 bits of the operand", "LOW 32 bits of the log", "HIGH 32 bits of the log"]
        ),
        0x21B => h!(
            "Raise a double to a double power.",
            &[
                "HIGH 32 bits of the base", "LOW 32 bits of the base", "HIGH 32 bits of the exponent", "LOW 32 bits of the exponent",
                "LOW 32 bits of base^exponent", "HIGH 32 bits of base^exponent",
            ]
        ),
        0x220 => h!(
            "Sine of a double angle in radians.",
            &["HIGH 32 bits of the angle", "LOW 32 bits of the angle", "LOW 32 bits of the sine", "HIGH 32 bits of the sine"]
        ),
        0x221 => h!(
            "Cosine of a double angle in radians.",
            &["HIGH 32 bits of the angle", "LOW 32 bits of the angle", "LOW 32 bits of the cosine", "HIGH 32 bits of the cosine"]
        ),
        0x222 => h!(
            "Tangent of a double angle in radians.",
            &["HIGH 32 bits of the angle", "LOW 32 bits of the angle", "LOW 32 bits of the tangent", "HIGH 32 bits of the tangent"]
        ),
        0x223 => h!(
            "Arcsine, result in radians (double).",
            &["HIGH 32 bits of the operand", "LOW 32 bits of the operand", "LOW 32 bits of the arcsine", "HIGH 32 bits of the arcsine"]
        ),
        0x224 => h!(
            "Arccosine, result in radians (double).",
            &["HIGH 32 bits of the operand", "LOW 32 bits of the operand", "LOW 32 bits of the arccosine", "HIGH 32 bits of the arccosine"]
        ),
        0x225 => h!(
            "Arctangent, result in radians (double).",
            &["HIGH 32 bits of the operand", "LOW 32 bits of the operand", "LOW 32 bits of the arctangent", "HIGH 32 bits of the arctangent"]
        ),
        0x226 => h!(
            "Two-argument arctangent (atan2), result in radians (double).",
            &[
                "HIGH 32 bits of y", "LOW 32 bits of y", "HIGH 32 bits of x", "LOW 32 bits of x",
                "LOW 32 bits of atan2(y, x)", "HIGH 32 bits of atan2(y, x)",
            ]
        ),
        // ── double branches ──────────────────────────────────────────────────
        0x230 => h!(
            "Branch if two doubles are equal within a tolerance (\"fuzzy\" equality).",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b", "HIGH 32 bits of the tolerance", "LOW 32 bits of the tolerance"],
            branch: "|a − b| ≤ |tolerance| — false if any operand is NaN, true if a and b are the same infinity"
        ),
        0x231 => h!(
            "Branch unless two doubles are equal within a tolerance — unlike jdeq, this DOES branch on NaN.",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b", "HIGH 32 bits of the tolerance", "LOW 32 bits of the tolerance"],
            branch: "NOT(|a − b| ≤ |tolerance|) — true (branches) if any operand is NaN"
        ),
        0x232 => h!(
            "Branch if a is less than b (double).",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b"],
            branch: "a < b"
        ),
        0x233 => h!(
            "Branch if a is at most b (double).",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b"],
            branch: "a <= b"
        ),
        0x234 => h!(
            "Branch if a is greater than b (double).",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b"],
            branch: "a > b"
        ),
        0x235 => h!(
            "Branch if a is at least b (double).",
            &["HIGH 32 bits of a", "LOW 32 bits of a", "HIGH 32 bits of b", "LOW 32 bits of b"],
            branch: "a >= b"
        ),
        0x238 => h!(
            "Branch if a double value is NaN.",
            &["HIGH 32 bits of the value", "LOW 32 bits of the value"],
            branch: "the value is NaN"
        ),
        0x239 => h!(
            "Branch if a double value is infinite.",
            &["HIGH 32 bits of the value", "LOW 32 bits of the value"],
            branch: "the value is +/-infinity"
        ),
        _ => return None,
    })
}

// ─── one-instruction decode (cold path; disassembler only) ────────────────────

/// One decoded operand: its addressing `mode`, the raw immediate/address/offset
/// `value` (zero-extended), whether it is a store destination, and the address
/// its data occupies in the stream.
#[derive(Clone, Copy, Debug)]
pub struct Operand {
    pub mode: u8,
    pub value: u32,
    pub is_store: bool,
    pub data_addr: u32,
}

/// A fully decoded instruction spanning `[addr, next)`.
#[derive(Clone, Debug)]
pub struct Instr {
    pub addr: u32,
    pub opcode: u32,
    pub next: u32,
    pub operands: Vec<Operand>,
}

fn fault(kind: &str, a: u32) -> String {
    format!("memory fault: {kind} @{a:#010x}")
}

/// Decode the instruction at `addr` into opcode + operands. Errors on an
/// unknown opcode or a read past the mapped image (so the disassembler never
/// walks off into undecodable bytes).
pub fn decode_instr(mem: &Memory, addr: u32) -> R<Instr> {
    let (opcode, mut pc) = decode_opcode(mem, addr)?;
    let info = opcode_info(opcode).ok_or_else(|| format!("unknown opcode {opcode:#x} @{addr:#010x}"))?;
    let total = info.total();
    let mode_bytes = total.div_ceil(2);
    let mut modes = Vec::with_capacity(total);
    for i in 0..mode_bytes {
        let byte = mem.read8(pc + i as u32).ok_or_else(|| fault("read8", pc + i as u32))?;
        modes.push((byte & 0x0F) as u8);
        modes.push((byte >> 4) as u8);
    }
    pc += mode_bytes as u32;
    let mut operands = Vec::with_capacity(total);
    for (i, &mode) in modes.iter().enumerate().take(total) {
        let dl = operand_data_len(mode);
        let value = match dl {
            0 => 0,
            1 => mem.read8(pc).ok_or_else(|| fault("read8", pc))?,
            2 => mem.read16(pc).ok_or_else(|| fault("read16", pc))?,
            _ => mem.read32(pc).ok_or_else(|| fault("read32", pc))?,
        };
        operands.push(Operand { mode, value, is_store: info.is_store(i), data_addr: pc });
        pc += dl;
    }
    Ok(Instr { addr, opcode, next: pc, operands })
}

/// Length-only decode: the `next_pc` after the instruction at `addr`, without
/// allocating an operand list. Used by the discovery linear scan / tiling.
fn instr_len(mem: &Memory, addr: u32) -> R<u32> {
    let (opcode, mut pc) = decode_opcode(mem, addr)?;
    let info = opcode_info(opcode).ok_or_else(|| format!("unknown opcode {opcode:#x}"))?;
    let total = info.total();
    let mode_bytes = total.div_ceil(2) as u32;
    let mut modes = Vec::with_capacity(total);
    for i in 0..mode_bytes {
        let byte = mem.read8(pc + i).ok_or_else(|| fault("read8", pc + i))?;
        modes.push((byte & 0x0F) as u8);
        modes.push((byte >> 4) as u8);
    }
    pc += mode_bytes;
    for &mode in modes.iter().take(total) {
        pc += operand_data_len(mode);
    }
    Ok(pc)
}

/// Sign-extend an operand `value` per its addressing `mode` (constant modes
/// 0x1/0x2 are signed in the executor; 0x3 and addresses are used as-is).
fn signed_const(mode: u8, value: u32) -> i64 {
    match mode & 0x0F {
        0x1 => value as u8 as i8 as i64,
        0x2 => value as u16 as i16 as i64,
        _ => value as i32 as i64,
    }
}

/// This operand's value, if it's a constant — the only operands whose value
/// is known at disassembly time without executing the program (local/stack
/// operands carry no resolvable value; direct/RAM-relative address modes name
/// a location to read from, not a value in hand). Mode 0x0 counts as the
/// constant 0 for a LOAD (its documented meaning — `resolve_load`/
/// `resolve_load_sized` in exec.rs read it as a literal zero with no data
/// bytes); for a STORE, 0x0 means discard, not a value, so it's excluded.
/// Modes 0x1-0x3 are always constants regardless of load/store.
fn const_operand(o: &Operand) -> Option<u32> {
    match o.mode & 0x0F {
        0x0 if !o.is_store => Some(0),
        0x1..=0x3 => Some(o.value),
        _ => None,
    }
}

/// Where a branch/catch offset operand sends control, applying the shared
/// Glulx branch convention (`exec::Machine::branch`): offset 0/1 return that
/// value from the function instead of jumping; any other constant computes a
/// fixed target; a non-constant offset is only known at runtime.
enum BranchTarget {
    Addr(u32),
    Return(u32),
    Runtime,
}

/// Resolve a branch-offset operand's target per [`BranchTarget`]. `next` is
/// the instruction's `next` address (the branch bias in `exec::Machine::branch`
/// is relative to the PC just past the operands, which `next` always is here —
/// Glulx instructions carry no inline text/data after their operands).
fn resolve_branch_target(o: &Operand, next: u32) -> BranchTarget {
    let Some(v) = const_operand(o) else { return BranchTarget::Runtime };
    match signed_const(o.mode, v) {
        0 => BranchTarget::Return(0),
        1 => BranchTarget::Return(1),
        off => BranchTarget::Addr((next as i64 + off - 2) as u32),
    }
}

/// Spelled-out meaning of an operand's addressing mode, for the hover tooltip
/// (the compact disasm line already conveys this tersely via the `#`/`@`/`L`/`sp`
/// sigils `format_operand` uses).
fn mode_desc(mode: u8, is_store: bool) -> &'static str {
    match (mode & 0x0F, is_store) {
        (0x0, true) => "discard — the result is not kept",
        (0x0, false) => "constant 0",
        (0x1..=0x3, _) => "constant",
        (0x5..=0x7, _) => "direct memory address",
        (0x8, true) => "push onto the stack",
        (0x8, false) => "pop from the stack",
        (0x9..=0xB, _) => "local-variable frame offset",
        (0xD..=0xF, _) => "RAM-relative address (RAMSTART + offset)",
        _ => "unknown addressing mode",
    }
}

// ─── function headers & strings ───────────────────────────────────────────────

/// A validated function header.
struct FuncHeader {
    type_byte: u8,
    locals_count: u32,
    first_instr: u32,
}

/// Validate a function object at `addr`: a `0xC0`/`0xC1` type byte followed by a
/// well-formed locals descriptor (`(type∈{1,2,4}, count)` pairs ending in
/// `(0,0)`). Returns `None` for anything that is not a well-formed function.
fn parse_func_header(mem: &Memory, addr: u32) -> Option<FuncHeader> {
    let tb = mem.read8(addr)?;
    if tb != 0xC0 && tb != 0xC1 {
        return None;
    }
    let mut a = addr + 1;
    let mut count = 0u32;
    let mut pairs = 0u32;
    loop {
        let t = mem.read8(a)?;
        let c = mem.read8(a + 1)?;
        a += 2;
        if t == 0 && c == 0 {
            break;
        }
        if t != 1 && t != 2 && t != 4 {
            return None;
        }
        count += c;
        pairs += 1;
        if pairs > 255 {
            return None; // runaway: not a real descriptor
        }
    }
    Some(FuncHeader { type_byte: tb as u8, locals_count: count, first_instr: a })
}

/// If `addr` begins a string object, return `(type_byte, end)` where `end` is
/// one past the last string byte. E1 (compressed) extents are found by walking
/// the Huffman bitstream against the header decode table.
fn string_extent(mem: &Memory, decode_table: u32, addr: u32) -> Option<(u8, u32)> {
    match mem.read8(addr)? {
        0xE0 => {
            let mut a = addr + 1;
            loop {
                let b = mem.read8(a)?;
                a += 1;
                if b == 0 {
                    break;
                }
            }
            Some((0xE0, a))
        }
        0xE2 => {
            let mut a = addr + 4; // type byte + 3 pad, then 32-bit chars
            loop {
                let w = mem.read32(a)?;
                a += 4;
                if w == 0 {
                    break;
                }
            }
            Some((0xE2, a))
        }
        0xE1 => walk_e1(mem, decode_table, addr + 1, None).map(|(end, _)| (0xE1, end)),
        _ => None,
    }
}

/// Walk a compressed (E1) bit stream from `start` against the decode table at
/// `table` (root at `table+8`), returning `(end_byte, text)`. Bits are read
/// low-bit-first (matching `exec::decode_compressed`). When `cap` is `Some(n)`,
/// at most `n` chars of preview text are collected (indirect/complex leaf nodes
/// are elided as `…`); when `None`, no text is built (extent-only). Bounded so a
/// malformed stream can't loop forever.
fn walk_e1(mem: &Memory, table: u32, start: u32, cap: Option<usize>) -> Option<(u32, String)> {
    if table == 0 {
        return None;
    }
    let root = mem.read32(table + 8)?;
    let mut node = root;
    let mut addr = start;
    let mut bit = 0u32;
    let mut text = String::new();
    let mut steps = 0u32;
    loop {
        steps += 1;
        if steps > 1_000_000 {
            return None; // runaway guard
        }
        match mem.read8(node)? {
            0x00 => {
                // branch: consume one bit
                let byte = mem.read8(addr)?;
                let b = (byte >> bit) & 1;
                bit += 1;
                if bit == 8 {
                    bit = 0;
                    addr += 1;
                }
                node = if b == 0 { mem.read32(node + 1)? } else { mem.read32(node + 5)? };
            }
            0x01 => {
                // terminator: end is the byte after the last consumed one
                let end = if bit == 0 { addr } else { addr + 1 };
                return Some((end, text));
            }
            0x02 => {
                if let Some(n) = cap {
                    if text.chars().count() < n {
                        text.push(mem.read8(node + 1)? as u8 as char);
                    }
                }
                node = root;
            }
            0x03 => {
                if let Some(n) = cap {
                    let mut a = node + 1;
                    while text.chars().count() < n {
                        let b = mem.read8(a)?;
                        if b == 0 {
                            break;
                        }
                        text.push(b as u8 as char);
                        a += 1;
                    }
                }
                node = root;
            }
            // Complex/indirect leaves (unichar, unicode string, indirect refs):
            // not previewed — elide, but keep walking so the extent stays correct.
            0x04 | 0x05 | 0x08 | 0x09 | 0x0A | 0x0B => {
                if let Some(n) = cap {
                    if text.chars().count() < n {
                        text.push('…');
                    }
                }
                node = root;
            }
            _ => return None, // bad node type
        }
    }
}

// ─── discovered units ─────────────────────────────────────────────────────────

/// A discovered function.
#[derive(Clone, Copy)]
struct FuncUnit {
    addr: u32,
    end: u32,
    type_byte: u8,
    locals_count: u32,
    first_instr: u32,
    tier: Tier, // Rd or Soft
}

/// A tiled unit of the code region.
#[derive(Clone, Copy)]
enum Unit {
    Func(FuncUnit),
    /// A string object `[addr, end)` with its E0/E1/E2 type byte.
    Str { addr: u32, end: u32, type_byte: u8 },
    /// An opaque data/padding run `[addr, end)`.
    Data { addr: u32, end: u32 },
}

impl Unit {
    fn addr(&self) -> u32 {
        match self {
            Unit::Func(f) => f.addr,
            Unit::Str { addr, .. } | Unit::Data { addr, .. } => *addr,
        }
    }
    fn end(&self) -> u32 {
        match self {
            Unit::Func(f) => f.end,
            Unit::Str { end, .. } | Unit::Data { end, .. } => *end,
        }
    }
}

/// Public per-string summary for the inspector's Strings list.
#[derive(Clone, Debug)]
pub struct StringInfo {
    pub addr: u32,
    /// `0xE0` (C string), `0xE1` (compressed), or `0xE2` (Unicode).
    pub type_byte: u8,
    /// A short, lossy decoded preview of the string's text.
    pub preview: String,
}

/// Public per-function summary for the inspector's function list.
#[derive(Clone, Debug)]
pub struct FuncInfo {
    pub addr: u32,
    /// `0xC0` (stack-args) or `0xC1` (locals-args).
    pub type_byte: u8,
    pub locals_count: u32,
    pub tier: Tier,
    /// Accelerated-function number if the VM has assigned one, else `None`.
    pub accel: Option<u32>,
}

// ─── the cache ────────────────────────────────────────────────────────────────

/// Lazy disassembly + discovery cache. Build it once (cheap discovery), then
/// render address windows on demand.
pub struct DisasmCache {
    /// Sorted, gapless tiling of `[region_start, region_end)`.
    units: Vec<Unit>,
    region_start: u32,
    region_end: u32,
    decode_table: u32,
    ramstart: u32,
    /// Executed instruction-start PCs seeded from the VM (ground-truth code;
    /// upgrades tier and enables RAM-code rendering). See `seed_executed`.
    executed: HashSet<u32>,
    /// Memoized per-function instruction-start boundaries (ROM is immutable, so
    /// these are stable). Keyed by function entry address.
    boundaries: RefCell<HashMap<u32, Vec<u32>>>,
}

impl DisasmCache {
    /// Discover functions and tile the ROM code region. Cheap: headers + a
    /// constant call graph + a type-validated linear scan. Does NOT render text.
    pub fn build(mem: &Memory) -> DisasmCache {
        let region_start = 36; // just past the 36-byte header
        let ramstart = mem.ramstart();
        let region_end = ramstart;
        let decode_table = mem.decode_table();

        // Pass 1 — RD: BFS the constant call graph from the start function.
        let rd = discover_rd(mem, mem.start_func(), region_start, region_end);

        // Pass 2 — linear tiling: walk [region_start, region_end), classifying
        // each address as a function, a string, or opaque data.
        let mut units: Vec<Unit> = Vec::new();
        let mut addr = region_start;
        while addr < region_end {
            if let Some(fh) = parse_func_header(mem, addr) {
                let end = scan_func_end(mem, decode_table, fh.first_instr, region_end);
                let tier = if rd.contains(&addr) { Tier::Rd } else { Tier::Soft };
                units.push(Unit::Func(FuncUnit {
                    addr,
                    end,
                    type_byte: fh.type_byte,
                    locals_count: fh.locals_count,
                    first_instr: fh.first_instr,
                    tier,
                }));
                addr = end;
            } else if let Some((ty, end)) = string_extent(mem, decode_table, addr) {
                units.push(Unit::Str { addr, end, type_byte: ty });
                addr = end.max(addr + 1);
            } else {
                // Data/padding: accumulate to the next func/string start.
                let start = addr;
                addr += 1;
                while addr < region_end
                    && parse_func_header(mem, addr).is_none()
                    && string_extent(mem, decode_table, addr).is_none()
                {
                    addr += 1;
                }
                units.push(Unit::Data { addr: start, end: addr });
            }
        }

        DisasmCache {
            units,
            region_start,
            region_end,
            decode_table,
            ramstart,
            executed: HashSet::new(),
            boundaries: RefCell::new(HashMap::new()),
        }
    }

    /// The `[start, end)` code region this cache tiles (ROM: header end → RAMSTART).
    pub fn region(&self) -> (u32, u32) {
        (self.region_start, self.region_end)
    }

    /// Seed the cumulative executed-PC set (ground truth: these bytes are code).
    /// Upgrades tier rendering and lets RAM-resident code be disassembled.
    pub fn seed_executed(&mut self, pcs: impl IntoIterator<Item = u32>) {
        self.executed.extend(pcs);
    }

    /// Record a single executed instruction-start PC (dynamic discovery). Returns
    /// true iff it was newly added. Mirrors zvm's `confirm_pc` in intent:
    /// execution is ground truth about where instructions start.
    pub fn confirm_pc(&mut self, pc: u32) -> bool {
        self.executed.insert(pc)
    }

    /// Every discovered function, sorted by address, tagged with its accel number
    /// (looked up in `accel`, the VM's function→accel-number map).
    pub fn functions(&self, accel: &HashMap<u32, u32>) -> Vec<FuncInfo> {
        self.units
            .iter()
            .filter_map(|u| match u {
                Unit::Func(f) => Some(FuncInfo {
                    addr: f.addr,
                    type_byte: f.type_byte,
                    locals_count: f.locals_count,
                    tier: f.tier,
                    accel: accel.get(&f.addr).copied(),
                }),
                _ => None,
            })
            .collect()
    }

    /// Count of discovered string objects (cheap: the static tiling, no preview
    /// decoding). Lets a caller size/paginate its Strings list without paying to
    /// decode every preview.
    pub fn string_count(&self) -> usize {
        self.units.iter().filter(|u| matches!(u, Unit::Str { .. })).count()
    }

    /// Up to `limit` discovered string objects, in address order, each with a
    /// short preview — the inspector's Strings list. Discovery is static (the ROM
    /// tiling); previews are decoded here, only for the returned entries, so the
    /// cost is bounded by `limit` (a huge I7 image can hold tens of thousands of
    /// strings, and preview decoding an E1 stream is not free). Cold: only ever
    /// called while the inspector is open.
    pub fn strings(&self, mem: &Memory, limit: usize) -> Vec<StringInfo> {
        self.units
            .iter()
            .filter_map(|u| match u {
                Unit::Str { addr, type_byte, .. } => Some((*addr, *type_byte)),
                _ => None,
            })
            .take(limit)
            .map(|(addr, type_byte)| StringInfo {
                addr,
                type_byte,
                preview: self.string_preview(mem, addr).unwrap_or_default(),
            })
            .collect()
    }

    /// A short decoded preview of the string object at `addr` (E0/E1/E2), capped
    /// and lossy. `None` if `addr` is not a string object.
    pub fn string_preview(&self, mem: &Memory, addr: u32) -> Option<String> {
        string_text(mem, self.decode_table, addr, Some(40))
    }
}

/// Decode the string object at `addr` — `E0` (C string), `E1` (compressed
/// against the header's decoding table) or `E2` (Unicode) — stopping after
/// `cap` characters when one is given.
///
/// `None` if `addr` does not begin a string object. Free-standing because a
/// caller that has an address and a decoding table should not have to build a
/// whole disassembly to read one string: `gvm::objects` reads an object's
/// hardware name this way.
pub fn string_text(mem: &Memory, decode_table: u32, addr: u32, cap: Option<usize>) -> Option<String> {
    {
        let cap = cap.unwrap_or(usize::MAX);
        match mem.read8(addr)? {
            0xE0 => {
                let mut s = String::new();
                let mut a = addr + 1;
                while s.chars().count() < cap {
                    let b = mem.read8(a)?;
                    if b == 0 {
                        break;
                    }
                    s.push(b as u8 as char);
                    a += 1;
                }
                Some(s)
            }
            0xE2 => {
                let mut s = String::new();
                let mut a = addr + 4;
                while s.chars().count() < cap {
                    let w = mem.read32(a)?;
                    if w == 0 {
                        break;
                    }
                    s.push(char::from_u32(w).unwrap_or('\u{FFFD}'));
                    a += 4;
                }
                Some(s)
            }
            0xE1 => walk_e1(mem, decode_table, addr + 1, Some(cap)).map(|(_, t)| t),
            _ => None,
        }
    }
}

impl DisasmCache {

    /// A hover/help description for the instruction at `addr`: the opcode's
    /// one-sentence summary, then a line per operand — its resolved value,
    /// spelled-out addressing mode (constant/local offset/stack/RAM address),
    /// and named role — with store operands phrased as "stores … in Sn" and
    /// the branch/catch offset resolved to a concrete target (or the 0/1
    /// return-instead-of-branch shortcut, spelled out) rather than shown as a
    /// plain operand. A constant operand that happens to address a string
    /// object gets its decoded preview appended (reusing `string_preview`),
    /// and a `call`-family target that resolves to an accelerated function
    /// gets its accelerated name appended (reusing the same lookup `annotate`
    /// badges function headers with). Never empty: an address that fails to
    /// decode as an instruction gets a one-line diagnostic.
    pub fn describe(&self, mem: &Memory, accel: &HashMap<u32, u32>, addr: u32) -> Vec<String> {
        let Ok(ins) = decode_instr(mem, addr) else {
            return vec![format!("{addr:#010x}: does not decode as an instruction")];
        };
        let info = opcode_info(ins.opcode);
        let mnemonic = info.map(|i| i.mnemonic).unwrap_or("?");
        let help = opcode_help(ins.opcode);

        let mut lines = Vec::new();
        match &help {
            Some(h) => lines.push(format!("{mnemonic} — {}", h.summary)),
            None => lines.push(format!("{mnemonic} ({:#x}) — no description available.", ins.opcode)),
        }

        // Decode always implies a recognized opcode (decode_instr requires
        // `opcode_info` to succeed), but stay defensive rather than assume it.
        let Some(info) = info else { return lines };
        let branch_idx = info.branch_index();

        let mut load_n = 0usize;
        let mut store_n = 0usize;
        for (i, op) in ins.operands.iter().enumerate() {
            if Some(i) == branch_idx {
                continue; // described by the branch/jump line below instead
            }
            let role = help.as_ref().and_then(|h| h.roles.get(i)).copied().unwrap_or("");
            let val = self.format_operand(op);
            let mode = mode_desc(op.mode, op.is_store);
            let mut line = if info.is_store(i) {
                store_n += 1;
                let what = if role.is_empty() { "the result" } else { role };
                format!("  → S{store_n} stores {what} — writes to {val} ({mode})")
            } else {
                load_n += 1;
                if role.is_empty() {
                    format!("  L{load_n} = {val} ({mode})")
                } else {
                    format!("  L{load_n} = {val} ({mode}): {role}")
                }
            };
            if let Some(s) = const_operand(op).and_then(|a| self.string_preview(mem, a)) {
                line.push_str(&format!("  [string: \"{}\"]", escape(&s)));
            }
            lines.push(line);
        }

        if let Some(idx) = branch_idx {
            if let Some(op) = ins.operands.get(idx) {
                let target = resolve_branch_target(op, ins.next);
                match info.special {
                    Special::Catch => {
                        let where_ = match target {
                            BranchTarget::Return(v) => format!("returning {v} from the function"),
                            BranchTarget::Addr(a) => format!("{a:#010x}"),
                            BranchTarget::Runtime => "a runtime-computed address".to_string(),
                        };
                        lines.push(format!(
                            "  → after storing the frame token, unconditionally jumps to {where_} \
                             (this jump is not itself a branch — @throw is what later returns here)"
                        ));
                    }
                    Special::Branch => {
                        let cond = help.as_ref().and_then(|h| h.branch);
                        let line = match (cond, target) {
                            (Some(c), BranchTarget::Addr(a)) => format!("  → branches to {a:#010x} if {c}"),
                            (Some(c), BranchTarget::Return(v)) => format!(
                                "  → if {c}, returns {v} from the function instead of jumping (the 0/1 shortcut convention)"
                            ),
                            (Some(c), BranchTarget::Runtime) => format!("  → branches to a runtime-computed address if {c}"),
                            (None, BranchTarget::Addr(a)) => format!("  → unconditionally jumps to {a:#010x}"),
                            (None, BranchTarget::Return(v)) => format!(
                                "  → unconditionally returns {v} from the function (the 0/1 shortcut convention)"
                            ),
                            (None, BranchTarget::Runtime) => "  → unconditionally jumps to a runtime-computed address".to_string(),
                        };
                        lines.push(line);
                    }
                    _ => {}
                }
            }
        }

        if matches!(info.special, Special::Call) {
            if let Some(t) = ins.operands.first().and_then(const_operand) {
                if let Some(&num) = accel.get(&t) {
                    lines.push(format!("  → target {t:#010x} is accelerated: {}", crate::accel::accel_name(num)));
                }
            }
        }
        if matches!(info.special, Special::JumpAbs) {
            if let Some(t) = ins.operands.first().and_then(const_operand) {
                lines.push(format!("  → jumps to {t:#010x} (no branch bias, and NOT the 0/1 return shortcut)"));
            }
        }
        if matches!(info.special, Special::Glk) {
            if let Some(sel) = ins.operands.first().and_then(const_operand) {
                lines.push(format!("  → selector: {}", crate::exec::glk_selector_name(sel)));
            }
        }

        lines
    }

    /// Start address of the instruction after the one at `addr`.
    pub fn next_instr(&self, mem: &Memory, addr: u32) -> u32 {
        instr_len(mem, addr).unwrap_or(addr.saturating_add(1))
    }

    /// Start address of the instruction before `addr`. Found by re-scanning
    /// forward from the enclosing function's first instruction (a known
    /// boundary) — never by byte-guessing backward.
    pub fn prev_instr(&self, mem: &Memory, addr: u32) -> u32 {
        if let Some(f) = self.enclosing_func(addr) {
            let bounds = self.func_boundaries(mem, &f);
            let mut prev = f.addr; // fall back to the function entry
            for &b in &bounds {
                if b >= addr {
                    break;
                }
                prev = b;
            }
            return prev;
        }
        addr.saturating_sub(1)
    }

    /// Disassemble up to `lines` rows starting at `addr` (text only).
    pub fn disassemble(
        &self,
        mem: &Memory,
        accel: &HashMap<u32, u32>,
        addr: u32,
        lines: usize,
    ) -> Vec<String> {
        self.disassemble_tiered(mem, accel, addr, lines)
            .into_iter()
            .map(|(row, _)| row)
            .collect()
    }

    /// Disassemble up to `lines` rows starting at `addr`, each tagged with its
    /// [`Tier`]. Rendering is lazy: only this window is formatted. An executed
    /// PC (seeded) always renders as an instruction tier `Rd`, overriding a
    /// static Data/Soft classification.
    pub fn disassemble_tiered(
        &self,
        mem: &Memory,
        accel: &HashMap<u32, u32>,
        addr: u32,
        lines: usize,
    ) -> Vec<(String, Tier)> {
        let mut out = Vec::with_capacity(lines);
        if lines == 0 {
            return out;
        }
        // Inside a known function: render header (if at entry) + instructions.
        if let Some(f) = self.enclosing_func(addr) {
            self.render_from_func(mem, accel, &f, addr, lines, &mut out);
            // Continue into following units if room remains.
            let mut cursor = f.end;
            while out.len() < lines && cursor < self.region_end {
                self.render_unit_at(mem, accel, cursor, lines, &mut out);
                cursor = self.next_unit_start(cursor);
            }
            return out;
        }
        // Outside functions: walk units (string/data), or decode executed RAM code.
        let mut cursor = addr;
        while out.len() < lines {
            if cursor >= self.region_end && !self.executed.contains(&cursor) {
                break;
            }
            let advanced = self.render_unit_at(mem, accel, cursor, lines, &mut out);
            cursor = advanced;
            if cursor <= addr && out.len() >= lines {
                break;
            }
        }
        out
    }

    // ── internal rendering helpers ────────────────────────────────────────────

    /// Function whose `[addr, end)` contains `addr`.
    fn enclosing_func(&self, addr: u32) -> Option<FuncUnit> {
        // units are sorted; find the last unit starting at/<= addr.
        let idx = self.units.partition_point(|u| u.addr() <= addr);
        if idx == 0 {
            return None;
        }
        if let Unit::Func(f) = self.units[idx - 1] {
            if addr >= f.addr && addr < f.end {
                return Some(f);
            }
        }
        None
    }

    /// Start of the unit at/after `at` (clamped to region_end).
    fn next_unit_start(&self, at: u32) -> u32 {
        let idx = self.units.partition_point(|u| u.addr() <= at);
        self.units.get(idx).map(|u| u.addr()).unwrap_or(self.region_end)
    }

    /// Memoized instruction-start boundaries of a function (first_instr..end).
    fn func_boundaries(&self, mem: &Memory, f: &FuncUnit) -> Vec<u32> {
        if let Some(b) = self.boundaries.borrow().get(&f.addr) {
            return b.clone();
        }
        let mut bounds = Vec::new();
        let mut pc = f.first_instr;
        while pc < f.end {
            bounds.push(pc);
            match instr_len(mem, pc) {
                Ok(next) if next > pc => pc = next,
                _ => break,
            }
        }
        self.boundaries.borrow_mut().insert(f.addr, bounds.clone());
        bounds
    }

    /// Render a function starting at `from` (its header if `from == f.addr`,
    /// else its instructions from the boundary at/after `from`).
    fn render_from_func(
        &self,
        mem: &Memory,
        accel: &HashMap<u32, u32>,
        f: &FuncUnit,
        from: u32,
        lines: usize,
        out: &mut Vec<(String, Tier)>,
    ) {
        if from <= f.addr {
            out.push((self.func_header_line(f, accel), f.tier));
            if out.len() >= lines {
                return;
            }
        }
        for b in self.func_boundaries(mem, f) {
            if b < from {
                continue;
            }
            if out.len() >= lines {
                return;
            }
            let tier = if self.executed.contains(&b) { Tier::Rd } else { f.tier };
            out.push((self.instr_line(mem, accel, b), tier));
        }
    }

    /// Render whatever unit starts at `at` (or an executed RAM instruction),
    /// returning the address just past what was rendered.
    fn render_unit_at(
        &self,
        mem: &Memory,
        accel: &HashMap<u32, u32>,
        at: u32,
        lines: usize,
        out: &mut Vec<(String, Tier)>,
    ) -> u32 {
        // A known unit?
        let idx = self.units.partition_point(|u| u.addr() <= at);
        if idx > 0 {
            let u = self.units[idx - 1];
            if at >= u.addr() && at < u.end() {
                return match u {
                    Unit::Func(f) => {
                        self.render_from_func(mem, accel, &f, at, lines, out);
                        f.end
                    }
                    Unit::Str { addr, end, type_byte } => {
                        let preview = self.string_preview(mem, addr).unwrap_or_default();
                        let kind = match type_byte {
                            0xE0 => "string",
                            0xE1 => "string(E1)",
                            _ => "string(uni)",
                        };
                        out.push((format!("{addr:06x}  ; {kind} \"{}\"", escape(&preview)), Tier::Data));
                        end
                    }
                    Unit::Data { addr, end } => {
                        self.render_data(mem, addr, end, lines, out);
                        end
                    }
                };
            }
        }
        // Not in any unit: an executed RAM instruction, or unknown → decode/byte.
        if self.executed.contains(&at) || at >= self.ramstart {
            if let Ok(ins) = decode_instr(mem, at) {
                let tier = if self.executed.contains(&at) { Tier::Rd } else { Tier::Soft };
                out.push((self.instr_line(mem, accel, at), tier));
                return ins.next;
            }
        }
        let b = mem.read8(at).unwrap_or(0);
        out.push((format!("{at:06x}  .byte {b:#04x}"), Tier::Data));
        at + 1
    }

    fn render_data(&self, mem: &Memory, addr: u32, end: u32, lines: usize, out: &mut Vec<(String, Tier)>) {
        let mut row = addr;
        while row < end && out.len() < lines {
            let row_end = (row + 16).min(end);
            let hex = (row..row_end)
                .map(|a| format!("{:02x}", mem.read8(a).unwrap_or(0)))
                .collect::<Vec<_>>()
                .join(" ");
            out.push((format!("{row:06x}  .byte {hex}"), Tier::Data));
            row += 16;
        }
    }

    fn func_header_line(&self, f: &FuncUnit, accel: &HashMap<u32, u32>) -> String {
        let kind = if f.type_byte == 0xC0 { "C0/stack" } else { "C1/locals" };
        let mut line = format!(
            "{:06x}  ; function {kind}, {} local{}",
            f.addr,
            f.locals_count,
            if f.locals_count == 1 { "" } else { "s" }
        );
        if let Some(&num) = accel.get(&f.addr) {
            line.push_str(&format!("  [accel: {}]", crate::accel::accel_name(num)));
        }
        line
    }

    /// Render one instruction row: `addr  mnemonic  ops   ; annotations`.
    fn instr_line(&self, mem: &Memory, accel: &HashMap<u32, u32>, addr: u32) -> String {
        let ins = match decode_instr(mem, addr) {
            Ok(i) => i,
            Err(_) => {
                let b = mem.read8(addr).unwrap_or(0);
                return format!("{addr:06x}  .byte {b:#04x}");
            }
        };
        let info = opcode_info(ins.opcode);
        let mnemonic = info.map(|i| i.mnemonic).unwrap_or("?");
        let ops = ins
            .operands
            .iter()
            .map(|o| self.format_operand(o))
            .collect::<Vec<_>>()
            .join(", ");
        let mut line = if ops.is_empty() {
            format!("{addr:06x}  {mnemonic}")
        } else {
            format!("{addr:06x}  {mnemonic} {ops}")
        };
        if let Some(ann) = self.annotate(mem, accel, &ins, info) {
            line.push_str(&format!("   ; {ann}"));
        }
        line
    }

    /// Format one operand mode-aware: `#const`, `Ln`, `sp`, `@addr`, `_`.
    fn format_operand(&self, o: &Operand) -> String {
        match o.mode & 0x0F {
            0x0 if o.is_store => "_".to_string(),
            0x0 => "#0".to_string(),
            0x8 => "sp".to_string(),
            0x1..=0x3 => format!("#{:#x}", o.value),
            0x5..=0x7 => format!("@{:#x}", o.value),
            0x9..=0xB => format!("L{}", o.value),
            0xD..=0xF => format!("@{:#x}", self.ramstart.wrapping_add(o.value)),
            _ => format!("?{:#x}", o.value),
        }
    }

    /// Build the trailing `; …` annotation for call/branch/glk/string sites.
    fn annotate(
        &self,
        mem: &Memory,
        accel: &HashMap<u32, u32>,
        ins: &Instr,
        info: Option<OpInfo>,
    ) -> Option<String> {
        let info = info?;
        match info.special {
            Special::Call => {
                let t = const_operand(ins.operands.first()?)?;
                match accel.get(&t) {
                    Some(&num) => Some(format!("-> fn@{t:#x} [accel: {}]", crate::accel::accel_name(num))),
                    None => Some(format!("-> fn@{t:#x}")),
                }
            }
            Special::JumpAbs => {
                let t = const_operand(ins.operands.first()?)?;
                Some(format!("-> {t:#x}"))
            }
            Special::Glk => {
                let sel = const_operand(ins.operands.first()?)?;
                Some(crate::exec::glk_selector_name(sel))
            }
            Special::StreamStr => {
                let a = const_operand(ins.operands.first()?)?;
                self.string_preview(mem, a).map(|p| format!("\"{}\"", escape(&p)))
            }
            Special::Branch | Special::Catch => {
                let idx = info.branch_index()?;
                let o = ins.operands.get(idx)?;
                match resolve_branch_target(o, ins.next) {
                    BranchTarget::Return(v) => Some(format!("-> return {v}")),
                    BranchTarget::Addr(a) => Some(format!("-> {a:#x}")),
                    BranchTarget::Runtime => None,
                }
            }
            Special::None => None,
        }
    }
}

/// Escape control chars in a preview for single-line display.
fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' => "\\n".to_string(),
            '\t' => "\\t".to_string(),
            '\r' => "\\r".to_string(),
            c if (c as u32) < 0x20 => format!("\\x{:02x}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

/// RD discovery: BFS the constant call graph from `start_func`, returning every
/// validated function entry reached. Each function's body is scanned linearly
/// (stopping at the next header / a decode error / the region end) to harvest
/// constant call targets.
fn discover_rd(mem: &Memory, start_func: u32, region_start: u32, region_end: u32) -> BTreeSet<u32> {
    let mut found = BTreeSet::new();
    let mut work = vec![start_func];
    while let Some(a) = work.pop() {
        if a < region_start || a >= region_end || found.contains(&a) {
            continue;
        }
        let Some(fh) = parse_func_header(mem, a) else { continue };
        found.insert(a);
        // Scan this function's body for constant call targets.
        let mut pc = fh.first_instr;
        while pc < region_end {
            if pc != fh.first_instr && parse_func_header(mem, pc).is_some() {
                break; // reached the next function
            }
            let Ok(ins) = decode_instr(mem, pc) else { break };
            if let Some(info) = opcode_info(ins.opcode) {
                if info.special == Special::Call {
                    if let Some(o) = ins.operands.first() {
                        if matches!(o.mode & 0x0F, 0x1..=0x3) {
                            work.push(o.value);
                        }
                    }
                }
            }
            pc = ins.next;
        }
    }
    found
}

/// Scan a function body from `first_instr`, returning the end address (one past
/// the last instruction) — where the next function/string starts, a decode
/// error occurs, or the region ends.
fn scan_func_end(mem: &Memory, decode_table: u32, first_instr: u32, region_end: u32) -> u32 {
    let mut pc = first_instr;
    while pc < region_end {
        if pc != first_instr
            && (parse_func_header(mem, pc).is_some()
                || string_extent(mem, decode_table, pc).is_some())
        {
            break;
        }
        match instr_len(mem, pc) {
            Ok(next) if next > pc => pc = next,
            _ => {
                pc += 1;
                break;
            }
        }
    }
    pc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::{self, Op::*};
    use crate::exec::{Machine, StepResult};
    use crate::glk::TestBackend;
    use crate::memory::Memory;

    fn machine(funcs: &[Vec<u8>], start: usize) -> Machine {
        let built = asm::assemble(funcs, start, 0x100);
        Machine::with_glk(Memory::new(built.image).unwrap(), Box::new(TestBackend::new()))
    }

    fn mem_of(funcs: &[Vec<u8>], start: usize) -> (Memory, Vec<u32>) {
        let built = asm::assemble(funcs, start, 0x100);
        (Memory::new(built.image).unwrap(), built.addrs)
    }

    /// A straight-line function: copy #5→L0, add L0 #3→L0, sub L0 #1→L0, return L0.
    fn straight_line() -> Vec<u8> {
        let body = [
            asm::ins(0x40, &[C32(5), Local8(0)]),
            asm::ins(0x10, &[Local8(0), C8(3), Local8(0)]),
            asm::ins(0x11, &[Local8(0), C8(1), Local8(0)]),
            asm::ins(0x31, &[Local8(0)]),
        ]
        .concat();
        asm::func(0xC1, &[(4, 1)], &body)
    }

    #[test]
    fn decode_round_trips_with_executor() {
        // The executor is the oracle: every PC it starts an instruction at must
        // decode to an instruction the disassembler agrees ends at the next PC.
        let mut m = machine(&[straight_line()], 0);
        let mut pcs = Vec::new();
        loop {
            pcs.push(m.pc);
            if m.step() != StepResult::Continue {
                break;
            }
        }
        assert_eq!(pcs.len(), 4, "copy, add, sub, return");
        for w in pcs.windows(2) {
            let ins = decode_instr(m.mem(), w[0]).expect("decodes");
            assert_eq!(ins.next, w[1], "disasm next must match executor advance");
        }
        assert!(decode_instr(m.mem(), pcs[3]).is_ok(), "return decodes");
    }

    #[test]
    fn discovers_functions_and_assigns_tiers() {
        // helper@0x24 (called by main), main (start), orphan (only scan-found).
        let helper = asm::func(0xC1, &[], &asm::ins(0x31, &[C8(7)]));
        let main = asm::func(
            0xC1,
            &[],
            &[asm::ins(0x160, &[C32(0x24), Zero]), asm::ins(0x31, &[C8(0)])].concat(),
        );
        let orphan = asm::func(0xC1, &[], &asm::ins(0x31, &[C8(9)]));
        let (mem, addrs) = mem_of(&[helper, main, orphan], 1);
        let cache = DisasmCache::build(&mem);
        let funcs = cache.functions(&HashMap::new());
        assert_eq!(funcs.len(), 3, "all three discovered");
        let tier = |a: u32| funcs.iter().find(|f| f.addr == a).unwrap().tier;
        assert_eq!(tier(addrs[0]), Tier::Rd, "helper is called → Rd");
        assert_eq!(tier(addrs[1]), Tier::Rd, "main is the start func → Rd");
        assert_eq!(tier(addrs[2]), Tier::Soft, "orphan only scan-found → Soft");
        // type byte + locals surfaced
        let helper_info = funcs.iter().find(|f| f.addr == addrs[0]).unwrap();
        assert_eq!(helper_info.type_byte, 0xC1);
        assert_eq!(helper_info.locals_count, 0);
    }

    #[test]
    fn annotates_glk_selector_name() {
        // glk #0x2A #0 -> _   (0x2A = glk_window_clear); then return #0.
        let body = [
            asm::ins(0x130, &[C16(0x2A), C8(0), Zero]),
            asm::ins(0x31, &[C8(0)]),
        ]
        .concat();
        let (mem, addrs) = mem_of(&[asm::func(0xC1, &[], &body)], 0);
        let cache = DisasmCache::build(&mem);
        let text = cache.disassemble(&mem, &HashMap::new(), addrs[0], 8).join("\n");
        assert!(text.contains("glk_window_clear"), "got:\n{text}");
    }

    #[test]
    fn badges_accelerated_functions() {
        let (mem, addrs) = mem_of(&[straight_line()], 0);
        let cache = DisasmCache::build(&mem);
        let mut accel = HashMap::new();
        accel.insert(addrs[0], 1u32); // Z__Region
        let header = cache.disassemble(&mem, &accel, addrs[0], 1).remove(0);
        assert!(header.contains("[accel: Z__Region]"), "got: {header}");
    }

    #[test]
    fn string_preview_e0_e1_absent_and_present() {
        // Write an E0 C-string into writable RAM and preview it.
        let mut m = machine(&[straight_line()], 0);
        let base = m.mem().ramstart();
        m.mem.write8(base, 0xE0).unwrap();
        for (i, b) in b"Hi!".iter().enumerate() {
            m.mem.write8(base + 1 + i as u32, *b as u32).unwrap();
        }
        m.mem.write8(base + 4, 0).unwrap();
        let cache = DisasmCache::build(m.mem());
        assert_eq!(cache.string_preview(m.mem(), base).as_deref(), Some("Hi!"));
        // A non-string address previews as None.
        assert_eq!(cache.string_preview(m.mem(), base + 1), None);
    }

    #[test]
    fn lazy_window_renders_only_requested_lines() {
        let (mem, addrs) = mem_of(&[straight_line()], 0);
        let cache = DisasmCache::build(&mem);
        // From the function entry: header + N instruction rows, capped at `lines`.
        assert_eq!(cache.disassemble(&mem, &HashMap::new(), addrs[0], 1).len(), 1);
        assert_eq!(cache.disassemble(&mem, &HashMap::new(), addrs[0], 2).len(), 2);
        let three = cache.disassemble(&mem, &HashMap::new(), addrs[0], 3);
        assert!(three[0].contains("; function"), "first row is the header: {}", three[0]);
    }

    #[test]
    fn next_and_prev_instr_round_trip() {
        let mut m = machine(&[straight_line()], 0);
        let mut pcs = Vec::new();
        loop {
            pcs.push(m.pc);
            if m.step() != StepResult::Continue {
                break;
            }
        }
        let cache = DisasmCache::build(m.mem());
        for w in pcs.windows(2) {
            assert_eq!(cache.next_instr(m.mem(), w[0]), w[1]);
            assert_eq!(cache.prev_instr(m.mem(), w[1]), w[0]);
        }
    }

    #[test]
    fn describe_reports_opcode_help() {
        let (mem, _addrs) = mem_of(&[straight_line()], 0);
        let cache = DisasmCache::build(&mem);
        // The copy is the first instruction; find it by stepping the executor.
        let mut m = machine(&[straight_line()], 0);
        let copy_pc = m.pc;
        m.step(); // execute copy
        let add_pc = m.pc;
        let d_copy = cache.describe(&mem, &HashMap::new(), copy_pc);
        assert!(d_copy[0].starts_with("copy — "), "got: {d_copy:?}");
        let d_add = cache.describe(&mem, &HashMap::new(), add_pc);
        assert!(d_add[0].starts_with("add — "), "got: {d_add:?}");
        // Named operand roles + the stored-in-Sn phrasing (SQ-0476).
        assert!(d_add.iter().any(|l| l.contains(": a")), "missing role for L1: {d_add:?}");
        assert!(d_add.iter().any(|l| l.contains(": b")), "missing role for L2: {d_add:?}");
        assert!(
            d_add.iter().any(|l| l.contains("→ S1 stores") && l.contains("a + b")),
            "missing S1 store phrasing: {d_add:?}"
        );
    }

    #[test]
    fn branch_target_resolves() {
        // jnz L0, back-by-9 → target = next_pc - 9 - 2. Build a self-loop body.
        let body = [
            asm::ins(0x40, &[C32(3), Local8(0)]),          // copy #3 -> L0  (7 bytes)
            asm::ins(0x11, &[Local8(0), C8(1), Local8(0)]), // sub           (6 bytes)
            asm::ins(0x23, &[Local8(0), C16(-9)]),         // jnz L0, -9
            asm::ins(0x31, &[C8(0)]),
        ]
        .concat();
        let (mem, addrs) = mem_of(&[asm::func(0xC1, &[(4, 1)], &body)], 0);
        let cache = DisasmCache::build(&mem);
        let text = cache.disassemble(&mem, &HashMap::new(), addrs[0], 8).join("\n");
        // The sub sits at first_instr+7; jnz should resolve back to it.
        // Header = type byte + one (4,1) locals pair + (0,0) terminator = 5 bytes.
        let first_instr = addrs[0] + 1 + 2 + 2;
        let sub_addr = first_instr + 7;
        assert!(
            text.contains(&format!("-> {sub_addr:#x}")),
            "jnz target should resolve to the sub at {sub_addr:#x}\n{text}"
        );
    }

    #[test]
    fn real_image_decode_round_trip() {
        // Skip cleanly when the fixture isn't present.
        let path = "../gvm-cli/tests/fixtures/glulxercise.ulx";
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("skip: {path} not found");
            return;
        };
        let mem = Memory::new(bytes).expect("valid Glulx image");
        let start_func = mem.start_func();
        let cache = DisasmCache::build(&mem);
        let funcs = cache.functions(&HashMap::new());
        assert!(!funcs.is_empty(), "discovery found functions");
        assert!(
            funcs.iter().any(|f| f.addr == start_func && f.tier == Tier::Rd),
            "start function is discovered and Rd"
        );
        // Executor as oracle: every PC it starts must decode as a valid start.
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        let mut pcs = std::collections::HashSet::new();
        for _ in 0..300_000 {
            pcs.insert(m.pc);
            if m.step() != StepResult::Continue {
                break;
            }
        }
        assert!(pcs.len() > 50, "exercised a meaningful number of instructions");
        let bad: Vec<u32> = pcs
            .iter()
            .copied()
            .filter(|&pc| decode_instr(m.mem(), pc).is_err())
            .collect();
        assert!(
            bad.is_empty(),
            "{} executor PCs failed to decode (first: {:?})",
            bad.len(),
            bad.first().map(|a| format!("{a:#x}"))
        );
    }

    // ── SQ-0476: opcode-help / describe() coverage & quality ──────────────────

    #[test]
    fn opcode_help_covers_every_implemented_opcode() {
        // `opcode_info` is transcribed from — and round-trip tested against, via
        // `decode_round_trips_with_executor` / `real_image_decode_round_trip`
        // above — `exec::Machine::execute`'s dispatch, the authoritative set of
        // opcodes this VM implements. Every one of them must have a help entry,
        // or the hover tooltip silently falls back to "no description available."
        // The range comfortably covers the current max opcode number (0x239)
        // with headroom for opcodes added later.
        let missing: Vec<u32> =
            (0u32..=0x1000).filter(|&op| opcode_info(op).is_some() && opcode_help(op).is_none()).collect();
        assert!(
            missing.is_empty(),
            "opcode_help missing entries for: {:?}",
            missing.iter().map(|o| format!("{o:#x}")).collect::<Vec<_>>()
        );
    }

    #[test]
    fn describe_branch_resolves_target_and_names_the_unsigned_sense() {
        // jltu #3 #5 -> +9, then two `return`s (the second is the branch target).
        let body = [
            asm::ins(0x2A, &[C32(3), C32(5), C16(9)]), // jltu #3 #5 +9
            asm::ins(0x31, &[C8(0)]),                   // return 0 (fallthrough)
            asm::ins(0x31, &[C8(1)]),                   // return 1 (branch target)
        ]
        .concat();
        let (mem, addrs) = mem_of(&[asm::func(0xC1, &[], &body)], 0);
        let cache = DisasmCache::build(&mem);
        // Header with no locals: type byte + immediate (0,0) terminator = 3 bytes.
        let jltu_pc = addrs[0] + 3;
        let lines = cache.describe(&mem, &HashMap::new(), jltu_pc);
        assert!(lines[0].starts_with("jltu — "), "got: {lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("  L1 = ") && l.contains(": a")), "missing a: {lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("  L2 = ") && l.contains(": b")), "missing b: {lines:?}");
        let branch_line = lines
            .iter()
            .find(|l| l.contains("branches to"))
            .unwrap_or_else(|| panic!("no resolved branch line: {lines:?}"));
        assert!(branch_line.contains("UNSIGNED"), "missing unsigned-compare note: {branch_line}");
        assert!(branch_line.contains("0x"), "target address not resolved: {branch_line}");
    }

    #[test]
    fn describe_names_the_glk_selector_on_a_glk_call() {
        // glk #0x2A (glk_window_clear) #0 -> _.
        let body = [asm::ins(0x130, &[C16(0x2A), C8(0), Zero]), asm::ins(0x31, &[C8(0)])].concat();
        let (mem, addrs) = mem_of(&[asm::func(0xC1, &[], &body)], 0);
        let cache = DisasmCache::build(&mem);
        let glk_pc = addrs[0] + 3;
        let lines = cache.describe(&mem, &HashMap::new(), glk_pc).join("\n");
        assert!(lines.starts_with("glk — "), "got:\n{lines}");
        assert!(lines.contains("glk_window_clear"), "selector not named:\n{lines}");
        assert!(lines.contains("→ S1 stores"), "missing S1 store phrasing:\n{lines}");
    }

    #[test]
    fn describe_catch_shows_store_first_then_the_unconditional_jump() {
        // catch S1 -> +9 (store-first operand order: S1 in the encoding, THEN
        // the branch offset — GLULX_NOTES's documented quirk), then two returns.
        let body = [
            asm::ins(0x32, &[Local8(0), C16(9)]), // catch local0, +9
            asm::ins(0x31, &[C8(0)]),               // return 0 (fallthrough)
            asm::ins(0x31, &[C8(1)]),               // return 1 (catch's jump target)
        ]
        .concat();
        let (mem, addrs) = mem_of(&[asm::func(0xC1, &[(4, 1)], &body)], 0);
        let cache = DisasmCache::build(&mem);
        // Header: type byte + one (4,1) locals pair + (0,0) terminator = 5 bytes.
        let catch_pc = addrs[0] + 5;
        let lines = cache.describe(&mem, &HashMap::new(), catch_pc);
        assert!(lines[0].starts_with("catch — "), "got: {lines:?}");
        // The store (the frame token, S1) is described before the jump line —
        // catch's store-first encoding order, correctly reflected.
        let store_idx = lines.iter().position(|l| l.starts_with("  → S1 stores")).expect("no S1 store line");
        let jump_idx = lines.iter().position(|l| l.contains("unconditionally jumps")).expect("no jump line");
        assert!(store_idx < jump_idx, "catch must describe its store before its jump: {lines:?}");
        assert!(lines[jump_idx].contains("0x"), "catch's jump target not resolved: {lines:?}");
    }

    #[test]
    fn describe_previews_a_string_operand() {
        // streamstr pointing at an E0 C-string written into RAM. Two-pass: the
        // operand embeds RAMSTART's address, which isn't known until the image
        // is built — but a function's byte length (and thus RAMSTART) doesn't
        // depend on a C32 operand's VALUE, only its fixed 4-byte width, so the
        // first pass's RAMSTART is exactly the second pass's too.
        let placeholder = asm::func(0xC1, &[], &asm::ins(0x72, &[C32(0)]));
        let (mem0, _) = mem_of(&[placeholder], 0);
        let rs = mem0.ramstart();

        let body = asm::ins(0x72, &[C32(rs)]); // streamstr @ramstart
        let (mut mem, addrs) = mem_of(&[asm::func(0xC1, &[], &body)], 0);
        assert_eq!(mem.ramstart(), rs, "RAMSTART must be stable across the two passes");
        mem.write8(rs, 0xE0).unwrap();
        for (i, b) in b"Hi!".iter().enumerate() {
            mem.write8(rs + 1 + i as u32, *b as u32).unwrap();
        }
        mem.write8(rs + 4, 0).unwrap();

        let cache = DisasmCache::build(&mem);
        let streamstr_pc = addrs[0] + 3;
        let lines = cache.describe(&mem, &HashMap::new(), streamstr_pc);
        assert!(lines[0].starts_with("streamstr — "), "got: {lines:?}");
        assert!(
            lines.iter().any(|l| l.contains("[string: \"Hi!\"]")),
            "missing decoded string preview: {lines:?}"
        );
    }

    #[test]
    fn describe_spells_out_the_offset_0_1_return_convention() {
        // jz L1 offset=0 (returns 0 instead of branching), and a second
        // instruction with offset=1 (returns 1).
        let body0 = asm::ins(0x22, &[C32(0), Zero]); // jz #0, offset 0 -> "return 0"
        let (mem0, addrs0) = mem_of(&[asm::func(0xC1, &[], &body0)], 0);
        let cache0 = DisasmCache::build(&mem0);
        let lines0 = cache0.describe(&mem0, &HashMap::new(), addrs0[0] + 3);
        assert!(
            lines0.iter().any(|l| l.contains("returns 0") && l.contains("0/1 shortcut")),
            "missing offset-0 return convention: {lines0:?}"
        );

        let body1 = asm::ins(0x22, &[C32(0), C8(1)]); // jz #0, offset 1 -> "return 1"
        let (mem1, addrs1) = mem_of(&[asm::func(0xC1, &[], &body1)], 0);
        let cache1 = DisasmCache::build(&mem1);
        let lines1 = cache1.describe(&mem1, &HashMap::new(), addrs1[0] + 3);
        assert!(
            lines1.iter().any(|l| l.contains("returns 1") && l.contains("0/1 shortcut")),
            "missing offset-1 return convention: {lines1:?}"
        );
    }
}
