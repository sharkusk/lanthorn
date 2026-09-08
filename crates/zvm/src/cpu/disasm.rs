//! Public Z-machine disassembler: decoded `Instr` → human-readable text.
//! Zero-dependency. Verified against ZMSD §14 + this crate's `execute()` dispatch.

use crate::cpu::decode::{decode, Branch, Instr, Operand, OperandCount};
use crate::memory::Memory;

/// Mnemonic for a decoded instruction, version-aware. Hex fallback (never panics)
/// for opcodes with no assigned name in this version.
pub fn mnemonic(count: &OperandCount, opcode: u8, version: u8) -> &'static str {
    match count {
        OperandCount::Two => match opcode {
            0x01 => "je", 0x02 => "jl", 0x03 => "jg", 0x04 => "dec_chk",
            0x05 => "inc_chk", 0x06 => "jin", 0x07 => "test", 0x08 => "or",
            0x09 => "and", 0x0A => "test_attr", 0x0B => "set_attr",
            0x0C => "clear_attr", 0x0D => "store", 0x0E => "insert_obj",
            0x0F => "loadw", 0x10 => "loadb", 0x11 => "get_prop",
            0x12 => "get_prop_addr", 0x13 => "get_next_prop", 0x14 => "add",
            0x15 => "sub", 0x16 => "mul", 0x17 => "div", 0x18 => "mod",
            0x19 => "call_2s", 0x1A => "call_2n", 0x1B => "set_colour",
            0x1C => "throw",
            _ => "op:2op",
        },
        OperandCount::One => match opcode {
            0x00 => "jz", 0x01 => "get_sibling", 0x02 => "get_child",
            0x03 => "get_parent", 0x04 => "get_prop_len", 0x05 => "inc",
            0x06 => "dec", 0x07 => "print_addr", 0x08 => "call_1s",
            0x09 => "remove_obj", 0x0A => "print_obj", 0x0B => "ret",
            0x0C => "jump", 0x0D => "print_paddr", 0x0E => "load",
            0x0F => if version >= 5 { "call_1n" } else { "not" },
            _ => "op:1op",
        },
        OperandCount::Zero => match opcode {
            0x00 => "rtrue", 0x01 => "rfalse", 0x02 => "print",
            0x03 => "print_ret", 0x04 => "nop", 0x05 => "save",
            0x06 => "restore", 0x07 => "restart", 0x08 => "ret_popped",
            0x09 => if version >= 5 { "catch" } else { "pop" },
            0x0A => "quit", 0x0B => "new_line", 0x0C => "show_status",
            0x0D => "verify", 0x0E => "extended", 0x0F => "piracy",
            _ => "op:0op",
        },
        OperandCount::Var => match opcode {
            0x00 => "call_vs", 0x01 => "storew", 0x02 => "storeb",
            0x03 => "put_prop", 0x04 => if version >= 5 { "aread" } else { "sread" },
            0x05 => "print_char", 0x06 => "print_num", 0x07 => "random",
            0x08 => "push", 0x09 => "pull", 0x0A => "split_window",
            0x0B => "set_window", 0x0C => "call_vs2", 0x0D => "erase_window",
            0x0E => "erase_line", 0x0F => "set_cursor", 0x10 => "get_cursor",
            0x11 => "set_text_style", 0x12 => "buffer_mode", 0x13 => "output_stream",
            0x14 => "input_stream", 0x15 => "sound_effect", 0x16 => "read_char",
            0x17 => "scan_table", 0x18 => "not", 0x19 => "call_vn",
            0x1A => "call_vn2", 0x1B => "tokenise", 0x1C => "encode_text",
            0x1D => "copy_table", 0x1E => "print_table", 0x1F => "check_arg_count",
            _ => "op:var",
        },
        OperandCount::Ext => match opcode {
            0x00 => "save", 0x01 => "restore", 0x02 => "log_shift",
            0x03 => "art_shift", 0x04 => "set_font", 0x05 => "draw_picture",
            0x06 => "picture_data", 0x07 => "erase_picture", 0x08 => "set_margins",
            0x09 => "save_undo", 0x0A => "restore_undo", 0x0B => "print_unicode",
            0x0C => "check_unicode", 0x0D => "set_true_colour", 0x10 => "move_window",
            0x11 => "window_size", 0x12 => "window_style", 0x13 => "get_wind_prop",
            0x14 => "scroll_window", 0x15 => "pop_stack", 0x16 => "read_mouse",
            0x17 => "mouse_window", 0x18 => "push_stack", 0x19 => "put_wind_prop",
            0x1A => "print_form", 0x1B => "make_menu", 0x1C => "picture_table",
            0x1D => "buffer_screen",
            _ => "op:ext",
        },
    }
}

fn fmt_operand(op: &Operand) -> String {
    match op {
        Operand::Large(n) => format!("#{:04x}", n),
        Operand::Small(n) => format!("#{:02x}", n),
        Operand::Var(0) => "sp".to_string(),
        Operand::Var(n @ 1..=15) => format!("local{}", n - 1),
        Operand::Var(n) => format!("g{:02x}", n - 16),
    }
}

fn fmt_var(v: u8) -> String {
    match v {
        0 => "sp".to_string(),
        1..=15 => format!("local{}", v - 1),
        n => format!("g{:02x}", n - 16),
    }
}

fn fmt_branch(b: &Branch, next_pc: u32) -> String {
    let neg = if b.on_true { "" } else { "~" };
    let target = match b.offset {
        0 => "rfalse".to_string(),
        1 => "rtrue".to_string(),
        off => format!("0x{:06x}", (next_pc as i64 + off as i64 - 2) as u32),
    };
    format!(" ?{}{}", neg, target)
}

/// Semantic role of a constant operand, so the app can render/link it
/// appropriately. Only constant operands (`Small`/`Large`) ever carry a
/// non-`Plain` role; a `Var` is a runtime value and always renders as its
/// variable sigil. `VarRef` marks a constant that IS a variable number (an
/// indirect variable reference — `load`/`store`/`inc`/`dec`/`inc_chk`/
/// `dec_chk`/`pull`): rendered as the variable it names (`sp`/`localN`/`gNN`)
/// rather than a bare constant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum OpRole { Plain, Object, MemAddr, Routine, StringAddr, JumpOffset, VarRef }

/// Semantic role of operand `index` for this opcode (version-aware). `Plain`
/// for every operand not in the table below. Verified against this file's
/// `mnemonic` table AND the crate's `execute()` operand semantics.
pub fn operand_role(count: &OperandCount, opcode: u8, version: u8, index: usize) -> OpRole {
    use OpRole::*;
    match count {
        OperandCount::One => match opcode {
            0x01 | 0x02 | 0x03 | 0x09 | 0x0A if index == 0 => Object,
            0x07 if index == 0 => MemAddr,
            0x08 if index == 0 => Routine,
            0x0F if index == 0 && version >= 5 => Routine,
            0x0D if index == 0 => StringAddr,
            0x0C if index == 0 => JumpOffset,
            // load / inc / dec: operand 0 is a variable number.
            0x0E | 0x05 | 0x06 if index == 0 => VarRef,
            _ => Plain,
        },
        OperandCount::Two => match opcode {
            0x06 | 0x0E if index == 0 || index == 1 => Object,
            0x0A | 0x0B | 0x0C | 0x11 | 0x12 | 0x13 if index == 0 => Object,
            0x0F | 0x10 if index == 0 => MemAddr,
            0x19 | 0x1A if index == 0 => Routine,
            // store / dec_chk / inc_chk: operand 0 is a variable number.
            0x0D | 0x04 | 0x05 if index == 0 => VarRef,
            _ => Plain,
        },
        OperandCount::Var => match opcode {
            0x03 if index == 0 => Object,
            0x01 | 0x02 if index == 0 => MemAddr,
            0x00 | 0x0C | 0x19 | 0x1A if index == 0 => Routine,
            // pull: operand 0 is the variable number to pull into.
            0x09 if index == 0 => VarRef,
            _ => Plain,
        },
        _ => Plain,
    }
}

/// One-line summary and full per-opcode help live in [`crate::cpu::opcode_help`];
/// re-exported here so the disassembler's traditional path still resolves them.
pub use crate::cpu::opcode_help::{describe_mnemonic, opcode_help};

/// Numeric value of a constant operand (`None` for a variable operand).
fn const_operand(op: &Operand) -> Option<u16> {
    match op {
        Operand::Small(n) => Some(*n as u16),
        Operand::Large(n) => Some(*n),
        Operand::Var(_) => None,
    }
}

/// Render an operand's value for a help tooltip: unpack packed routine/string
/// constants to their real target address, and name a by-number variable
/// reference (`local3`/`g0f`/`sp`). Everything else uses the normal operand form.
fn operand_value(op: &Operand, role: OpRole, unpack: &Unpack) -> String {
    match role {
        OpRole::Routine => {
            if let Some(p) = const_operand(op).filter(|&p| p != 0) {
                return format!("0x{:06x}", unpack.routine(p));
            }
        }
        OpRole::StringAddr => {
            if let Some(p) = const_operand(op) {
                return format!("0x{:06x}", unpack.string(p));
            }
        }
        OpRole::VarRef => {
            if let Some(n) = const_operand(op) {
                return fmt_var(n as u8);
            }
        }
        _ => {}
    }
    fmt_operand(op)
}

/// Instruction-aware help lines for a hover tooltip: the opcode's purpose, then
/// each operand's meaning (with its rendered value), what the stored result
/// means, the branch condition, and any inline text. Engine-neutral display text
/// (the app renders these lines verbatim).
pub fn describe_instruction(instr: &Instr, version: u8, unpack: &Unpack) -> Vec<String> {
    let name = mnemonic(&instr.operand_count, instr.opcode, version);
    let help = opcode_help(name);
    let mut lines = Vec::new();
    match &help {
        Some(h) => lines.push(format!("{name} — {}", h.summary)),
        None => lines.push(name.to_string()),
    }
    for (i, op) in instr.operands.iter().enumerate() {
        let role = operand_role(&instr.operand_count, instr.opcode, version, i);
        let val = operand_value(op, role, unpack);
        match help.as_ref().and_then(|h| h.params.get(i).copied().or(h.rest)) {
            Some(desc) => lines.push(format!("  arg{} = {}: {}", i + 1, val, desc)),
            None => lines.push(format!("  arg{} = {}", i + 1, val)),
        }
    }
    if let Some(v) = instr.store {
        match help.as_ref().and_then(|h| h.returns) {
            Some(r) => lines.push(format!("  → stores {} in {}", r, fmt_var(v))),
            None => lines.push(format!("  → result stored in {}", fmt_var(v))),
        }
    }
    if let Some(b) = &instr.branch {
        // A `~` in the disassembly's `?~` branch sigil inverts the sense, so an
        // inverted branch reads "unless" (not "if"); the note names the cause.
        let (kw, note) = if b.on_true { ("if", "") } else { ("unless", " (the ~ inverts the branch)") };
        match help.as_ref().and_then(|h| h.branch) {
            Some(c) => lines.push(format!("  → branches {kw} {c}{note}")),
            None => {
                let sense = if b.on_true { "true" } else { "false" };
                lines.push(format!("  → branches when the test is {sense}{note}"));
            }
        }
    }
    if instr.text.is_some() {
        lines.push("  → prints the inline text shown".to_string());
    }
    lines
}

/// Header-derived context for unpacking packed routine/string addresses.
/// `routine_off`/`string_off` are only consulted for versions 6 and 7.
#[derive(Clone, Copy)]
pub struct Unpack { pub version: u8, pub routine_off: u16, pub string_off: u16 }

impl Unpack {
    /// Build from a story's header. Reads the routine-offset word at 0x28 and
    /// the string-offset word at 0x2A (big-endian) only for v6/7; 0 otherwise.
    pub fn from_mem(mem: &Memory) -> Self {
        let version = mem.version();
        let (routine_off, string_off) = if version == 6 || version == 7 {
            (mem.read_word(0x28), mem.read_word(0x2A))
        } else {
            (0, 0)
        };
        Unpack { version, routine_off, string_off }
    }
    /// Version-only context (offsets 0) — convenience for tests / v<6.
    pub fn plain(version: u8) -> Self { Unpack { version, routine_off: 0, string_off: 0 } }
    /// Byte address of a packed routine address (ZMSD §1.2.3).
    pub fn routine(&self, p: u16) -> u32 {
        let p = p as u32;
        match self.version {
            1..=3 => 2 * p,
            4 | 5 => 4 * p,
            6 | 7 => 4 * p + 8 * self.routine_off as u32,
            8 => 8 * p,
            _ => 2 * p,
        }
    }
    /// Byte address of a packed string address (ZMSD §1.2.3).
    pub fn string(&self, p: u16) -> u32 {
        let p = p as u32;
        match self.version {
            1..=3 => 2 * p,
            4 | 5 => 4 * p,
            6 | 7 => 4 * p + 8 * self.string_off as u32,
            8 => 8 * p,
            _ => 2 * p,
        }
    }
}

/// Format one decoded instruction as "mnemonic op, op -> store ?branch [\"text\"]".
pub fn format_instr(instr: &Instr, unpack: &Unpack) -> String {
    let version = unpack.version;
    let mut s = mnemonic(&instr.operand_count, instr.opcode, version).to_string();
    if !instr.operands.is_empty() {
        let is_print_char = matches!(instr.operand_count, OperandCount::Var) && instr.opcode == 0x05;
        let ops: Vec<String> = instr.operands.iter().enumerate().map(|(i, op)| {
            if is_print_char {
                let ch = match op {
                    Operand::Small(n) => Some(*n as u32),
                    Operand::Large(n) => Some(*n as u32),
                    Operand::Var(_) => None,
                };
                if let Some(c) = ch.filter(|c| (32..=126).contains(c)) {
                    return format!("'{}'", c as u8 as char);
                }
            }
            // A variable holds a runtime value, not a static reference — render
            // its sigil (sp/localN/gNN), except that a variable used AS a memory
            // address gets an `@` prefix (`@local5`) so the debugger can link it
            // to memory at the address it currently holds.
            let value = match op {
                Operand::Small(n) => *n as u16,
                Operand::Large(n) => *n,
                Operand::Var(_) => {
                    return match operand_role(&instr.operand_count, instr.opcode, version, i) {
                        OpRole::MemAddr => format!("@{}", fmt_operand(op)),
                        OpRole::Object => format!("obj#{}", fmt_operand(op)),
                        _ => fmt_operand(op),
                    };
                }
            };
            match operand_role(&instr.operand_count, instr.opcode, version, i) {
                OpRole::Object => format!("obj#{}", value),
                OpRole::MemAddr => format!("@0x{:06x}", value as u32),
                OpRole::Routine => format!("0x{:06x}", unpack.routine(value)),
                OpRole::StringAddr => format!("@0x{:06x}", unpack.string(value)),
                OpRole::JumpOffset => {
                    let offset = match op {
                        Operand::Large(n) => *n as i16,
                        Operand::Small(n) => *n as i8 as i16,
                        Operand::Var(_) => unreachable!(),
                    };
                    let target = (instr.next_pc as i64 + offset as i64 - 2) as u32;
                    format!("0x{:06x}", target)
                }
                // The constant IS a variable number — render the variable it
                // names (sp/localN/gNN) rather than a bare `#nn`.
                OpRole::VarRef => fmt_var(value as u8),
                OpRole::Plain => fmt_operand(op),
            }
        }).collect();
        s.push(' ');
        s.push_str(&ops.join(", "));
    }
    if let Some(v) = instr.store {
        s.push_str(&format!(" -> {}", fmt_var(v)));
    }
    if let Some(b) = &instr.branch {
        s.push_str(&fmt_branch(b, instr.next_pc));
    }
    if let Some((text, _)) = &instr.text {
        s.push_str(&format!(" {:?}", text));
    }
    s
}

/// Plain mnemonic disassembly: no operand-role sigils, no packed-address
/// unpacking, no annotations — just mnemonic, `#hex`/named-variable operands,
/// computed branch targets, and inline text. (Version only affects mnemonics.)
pub fn format_instr_basic(instr: &Instr, version: u8) -> String {
    let mut s = mnemonic(&instr.operand_count, instr.opcode, version).to_string();
    if !instr.operands.is_empty() {
        let is_print_char = matches!(instr.operand_count, OperandCount::Var) && instr.opcode == 0x05;
        let ops: Vec<String> = instr.operands.iter().map(|op| {
            if is_print_char {
                let ch = match op { Operand::Small(n) => Some(*n as u32), Operand::Large(n) => Some(*n as u32), Operand::Var(_) => None };
                if let Some(c) = ch.filter(|c| (32..=126).contains(c)) { return format!("'{}'", c as u8 as char); }
            }
            fmt_operand(op)
        }).collect();
        s.push(' ');
        s.push_str(&ops.join(", "));
    }
    if let Some(v) = instr.store { s.push_str(&format!(" -> {}", fmt_var(v))); }
    if let Some(b) = &instr.branch { s.push_str(&fmt_branch(b, instr.next_pc)); }
    if let Some((text, _)) = &instr.text { s.push_str(&format!(" {:?}", text)); }
    s
}

/// Basic-mode disassembly (see [`format_instr_basic`]). Mirrors [`disassemble`]
/// but emits no annotations or reference sigils. Stops early at end of memory.
pub fn disassemble_basic(mem: &Memory, start: u32, version: u8, lines: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(lines);
    let mut pc = start.min(mem.len() as u32);
    for _ in 0..lines {
        if pc >= mem.len() as u32 {
            break;
        }
        let instr = decode(mem, pc, version);
        out.push(format!("{:06x}  {}", pc, format_instr_basic(&instr, version)));
        // Guard against a decoder that fails to advance (malformed bytes).
        pc = if instr.next_pc > pc { instr.next_pc } else { pc + 1 };
    }
    out
}

/// Disassemble `lines` instructions starting at `start`, each prefixed with its
/// address. Stops early at the end of memory (never reads out of range).
pub fn disassemble(mem: &Memory, start: u32, version: u8, lines: usize) -> Vec<String> {
    let unpack = Unpack::from_mem(mem);
    let mut out = Vec::with_capacity(lines);
    let mut pc = start.min(mem.len() as u32);
    for _ in 0..lines {
        if pc >= mem.len() as u32 {
            break;
        }
        let instr = decode(mem, pc, version);
        out.push(format!("{:06x}  {}", pc, format_instr(&instr, &unpack)));
        // Guard against a decoder that fails to advance (malformed bytes).
        pc = if instr.next_pc > pc { instr.next_pc } else { pc + 1 };
    }
    out
}

/// Raw class tag for an `OperandCount` — the untranslated instruction class.
fn class_tag(count: &OperandCount) -> &'static str {
    match count {
        OperandCount::Zero => "0OP",
        OperandCount::One => "1OP",
        OperandCount::Two => "2OP",
        OperandCount::Var => "VAR",
        OperandCount::Ext => "EXT",
    }
}

/// Format one decoded instruction in RAW form — the part AFTER the address:
/// `{bytes}   {CLASS}:0x{opcode}  {operands} -> V{store} ?{T|F}{offset} "text"`.
/// Deliberately performs NO lookups (a diagnostic to catch bugs in the
/// translation layer): no mnemonic name, no operand-role sigils, no variable
/// naming (`sp`/`localN`/`gNN`), no packed-address unpacking, no annotations.
/// `bytes` is the instruction's raw bytes (already extracted + capped by the
/// caller); `truncated` appends `…` to the byte column when they were clipped.
pub fn format_instr_raw(instr: &Instr, bytes: &[u8], truncated: bool) -> String {
    let mut byte_col = bytes.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
    if truncated {
        byte_col.push_str(" …");
    }
    let mut s = format!("{}   {}:0x{:02x}", byte_col, class_tag(&instr.operand_count), instr.opcode);
    if !instr.operands.is_empty() {
        let ops: Vec<String> = instr.operands.iter().map(|op| match op {
            Operand::Large(n) => format!("L#{:04x}", n),
            Operand::Small(n) => format!("S#{:02x}", n),
            Operand::Var(n) => format!("V{:02x}", n),
        }).collect();
        s.push_str("  ");
        s.push_str(&ops.join(", "));
    }
    if let Some(v) = instr.store {
        s.push_str(&format!(" -> V{:02x}", v));
    }
    if let Some(b) = &instr.branch {
        let tf = if b.on_true { "T" } else { "F" };
        s.push_str(&format!(" ?{}{}", tf, b.offset));
    }
    if let Some((text, _)) = &instr.text {
        s.push_str(&format!(" {:?}", text));
    }
    s
}

/// Disassemble `lines` instructions from `start` in RAW form — mirrors
/// [`disassemble`] but shows instruction bytes + untranslated structure with no
/// lookups (see [`format_instr_raw`]). Stops early at the end of memory.
pub fn disassemble_raw(mem: &Memory, start: u32, version: u8, lines: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(lines);
    let mut pc = start.min(mem.len() as u32);
    for _ in 0..lines {
        if pc >= mem.len() as u32 {
            break;
        }
        let instr = decode(mem, pc, version);
        let end = instr.next_pc.min(mem.len() as u32);
        let n = end.saturating_sub(pc).min(12);
        let truncated = end.saturating_sub(pc) > 12;
        let bytes: Vec<u8> = (0..n).map(|i| mem.read_byte(pc + i)).collect();
        out.push(format!("{:06x}: {}", pc, format_instr_raw(&instr, &bytes, truncated)));
        // Guard against a decoder that fails to advance (malformed bytes).
        pc = if instr.next_pc > pc { instr.next_pc } else { pc + 1 };
    }
    out
}

/// Address of the instruction following the one at `addr` (clamped to memory).
pub fn next_instr(mem: &Memory, addr: u32, version: u8) -> u32 {
    if addr >= mem.len() as u32 {
        return mem.len() as u32;
    }
    let n = decode(mem, addr, version).next_pc;
    n.min(mem.len() as u32).max(addr + 1)
}

/// Byte window scanned to find the previous instruction (> max instruction length).
const PREV_WINDOW: u32 = 24;

/// Start address of the instruction immediately BEFORE `addr`. Z-machine
/// instructions are variable-length and can't be decoded backwards, and a
/// single decode from an arbitrary byte offset can coincidentally land on
/// `addr` even when that offset isn't a real instruction boundary. So this
/// tries a decode-chain from every byte offset in `addr - PREV_WINDOW..addr`
/// and returns the candidate start most of those chains resync onto before
/// reaching `addr` (nearest-to-`addr` breaks ties). Guarantees the round-trip
/// `next_instr(prev_instr(addr)) == addr` whenever any chain lands on `addr`
/// at all. Falls back to `addr - 1` when nothing aligns (data region / start
/// of memory). Never returns `>= addr`.
pub fn prev_instr(mem: &Memory, addr: u32, version: u8) -> u32 {
    if addr == 0 { return 0; }
    let start = addr.saturating_sub(PREV_WINDOW);
    // A single decode from an arbitrary byte offset can coincidentally land on
    // `addr` even when that offset isn't a real instruction boundary (variable-
    // length Z-code is ambiguous when read out of alignment). To filter those
    // false positives out, try a decode-chain from EVERY byte offset in the
    // window, not just one: a chain starting on a real instruction boundary
    // resyncs onto the true instruction stream almost immediately and keeps
    // agreeing with every other chain that also resyncs, while a spurious
    // match from a misaligned offset is very unlikely to line up with more
    // than one starting offset. So take whichever predecessor the most
    // starting offsets agree on (nearest wins ties).
    let mut votes: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for s0 in start..addr {
        let mut pc = s0;
        loop {
            let n = decode(mem, pc, version).next_pc;
            if n == addr {
                *votes.entry(pc).or_insert(0) += 1;
                break;
            }
            if n >= addr {
                break; // overshot addr without landing on it exactly
            }
            pc = if n > pc { n } else { pc + 1 };
        }
    }
    if let Some((&best, _)) = votes.iter().max_by_key(|(&s, &count)| (count, s)) {
        return best;
    }
    addr.saturating_sub(1) // fallback: nothing aligned (data region / start of memory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::decode::OperandCount;

    #[test]
    fn mnemonics_cover_each_class() {
        assert_eq!(mnemonic(&OperandCount::Two, 0x14, 5), "add");
        assert_eq!(mnemonic(&OperandCount::Two, 0x0F, 5), "loadw");
        assert_eq!(mnemonic(&OperandCount::One, 0x00, 5), "jz");
        assert_eq!(mnemonic(&OperandCount::One, 0x0B, 5), "ret");
        assert_eq!(mnemonic(&OperandCount::Zero, 0x00, 5), "rtrue");
        assert_eq!(mnemonic(&OperandCount::Zero, 0x02, 5), "print");
        assert_eq!(mnemonic(&OperandCount::Var, 0x00, 5), "call_vs");
        assert_eq!(mnemonic(&OperandCount::Var, 0x04, 5), "aread");
        assert_eq!(mnemonic(&OperandCount::Var, 0x04, 3), "sread");
        assert_eq!(mnemonic(&OperandCount::One, 0x0F, 5), "call_1n");
        assert_eq!(mnemonic(&OperandCount::One, 0x0F, 4), "not");
        assert_eq!(mnemonic(&OperandCount::Ext, 0x09, 5), "save_undo");
        // Unknown falls back to a hex label, never panics.
        assert!(mnemonic(&OperandCount::Two, 0x7F, 5).starts_with("op:"));
    }

    #[test]
    fn formats_operands_store_and_branch() {
        use crate::cpu::decode::Form;
        // add local1, #05 -> sp   (2OP:0x14, store to var 0)
        let instr = Instr {
            opcode: 0x14, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Var(1), Operand::Small(5)],
            store: Some(0), branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&instr, &Unpack::plain(5));
        assert!(s.starts_with("add "), "got {s:?}");
        assert!(s.contains("local0"), "got {s:?}");
        assert!(s.contains("#05"), "got {s:?}");
        assert!(s.contains("-> sp"), "got {s:?}");
    }

    #[test]
    fn print_char_renders_a_printable_constant_operand_as_a_character() {
        use crate::cpu::decode::Form;
        // print_char #0x41 (VAR:0x05, one small-constant operand)
        let instr = Instr {
            opcode: 0x05, form: Form::Variable, operand_count: OperandCount::Var,
            operands: vec![Operand::Small(0x41)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&instr, &Unpack::plain(5));
        assert!(s.contains("'A'"), "got {s:?}");
    }

    #[test]
    fn print_char_keeps_hex_for_a_variable_or_non_printable_operand() {
        use crate::cpu::decode::Form;
        let var_op = Instr {
            opcode: 0x05, form: Form::Variable, operand_count: OperandCount::Var,
            operands: vec![Operand::Var(1)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        assert!(format_instr(&var_op, &Unpack::plain(5)).contains("local0"), "got {:?}", format_instr(&var_op, &Unpack::plain(5)));
        let non_printable = Instr {
            opcode: 0x05, form: Form::Variable, operand_count: OperandCount::Var,
            operands: vec![Operand::Small(0x0a)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&non_printable, &Unpack::plain(5));
        assert!(s.contains("#0a"), "got {s:?}");
    }

    #[test]
    fn operand_role_classifies_each_family() {
        // Object: test_attr (2OP:0x0A) index 0 is the object, index 1 is an attribute #.
        assert_eq!(operand_role(&OperandCount::Two, 0x0A, 5, 0), OpRole::Object);
        assert_eq!(operand_role(&OperandCount::Two, 0x0A, 5, 1), OpRole::Plain);
        // MemAddr: loadw (2OP:0x0F) base address is index 0; the word index is Plain.
        assert_eq!(operand_role(&OperandCount::Two, 0x0F, 5, 0), OpRole::MemAddr);
        assert_eq!(operand_role(&OperandCount::Two, 0x0F, 5, 1), OpRole::Plain);
        // Routine: call_vs (VAR:0x00) packed routine is index 0; args are Plain.
        assert_eq!(operand_role(&OperandCount::Var, 0x00, 5, 0), OpRole::Routine);
        assert_eq!(operand_role(&OperandCount::Var, 0x00, 5, 1), OpRole::Plain);
        // StringAddr: print_paddr (1OP:0x0D).
        assert_eq!(operand_role(&OperandCount::One, 0x0D, 5, 0), OpRole::StringAddr);
        // JumpOffset: jump (1OP:0x0C).
        assert_eq!(operand_role(&OperandCount::One, 0x0C, 5, 0), OpRole::JumpOffset);
        // 1OP:0x0F is call_1n (Routine) at v5+, but `not` (Plain) at v3.
        assert_eq!(operand_role(&OperandCount::One, 0x0F, 5, 0), OpRole::Routine);
        assert_eq!(operand_role(&OperandCount::One, 0x0F, 3, 0), OpRole::Plain);
        // jin (2OP:0x06) and insert_obj (2OP:0x0E) take an object in BOTH operands.
        assert_eq!(operand_role(&OperandCount::Two, 0x06, 5, 0), OpRole::Object);
        assert_eq!(operand_role(&OperandCount::Two, 0x06, 5, 1), OpRole::Object);
        assert_eq!(operand_role(&OperandCount::Two, 0x0E, 5, 1), OpRole::Object);
    }

    #[test]
    fn packed_address_unpacking_scales_by_version() {
        // Routine/string both double in v1-3, quadruple in v4/5, x8 in v8.
        assert_eq!(Unpack::plain(3).routine(0x0a2f), 0x00145e);
        assert_eq!(Unpack::plain(3).string(0x0a2f), 0x00145e);
        assert_eq!(Unpack::plain(5).routine(0x0050), 0x000140);
        assert_eq!(Unpack::plain(8).routine(0x0010), 0x000080);
        // v6/7 add 8*offset.
        let u = Unpack { version: 7, routine_off: 2, string_off: 4 };
        assert_eq!(u.routine(0x0010), 4 * 0x0010 + 8 * 2);
        assert_eq!(u.string(0x0010), 4 * 0x0010 + 8 * 4);
    }

    #[test]
    fn format_instr_renders_role_sigils() {
        use crate::cpu::decode::Form;
        // loadw #0x1234, #0x00 -> the base address renders as a memory sigil,
        // the word index stays a plain constant.
        let loadw = Instr {
            opcode: 0x0F, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Large(0x1234), Operand::Small(0x00)],
            store: Some(0), branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&loadw, &Unpack::plain(5));
        assert!(s.contains("@0x001234"), "got {s:?}");
        assert!(s.contains("#00"), "got {s:?}");

        // call_vs with a packed routine constant unpacks (v3 doubling).
        let call = Instr {
            opcode: 0x00, form: Form::Variable, operand_count: OperandCount::Var,
            operands: vec![Operand::Large(0x0a2f)],
            store: Some(0), branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&call, &Unpack::plain(3));
        assert!(s.contains("0x00145e"), "got {s:?}");
        assert!(!s.contains("@0x00145e"), "routine is a code addr, not @-prefixed: {s:?}");

        // print_paddr packed string renders with the @-prefixed memory sigil.
        let pp = Instr {
            opcode: 0x0D, form: Form::Short, operand_count: OperandCount::One,
            operands: vec![Operand::Large(0x0a2f)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&pp, &Unpack::plain(3));
        assert!(s.contains("@0x00145e"), "got {s:?}");

        // test_attr #5, #10 -> object sigil for operand 0, plain for the attribute.
        let ta = Instr {
            opcode: 0x0A, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(5), Operand::Small(10)],
            store: None, branch: Some(Branch { on_true: true, offset: 4, len: 1 }),
            text: None, next_pc: 0x1000,
        };
        let s = format_instr(&ta, &Unpack::plain(5));
        assert!(s.contains("obj#5"), "got {s:?}");
        assert!(s.contains("#0a"), "attribute stays a plain constant: {s:?}");

        // jump with a signed offset renders the resolved code target.
        let jump = Instr {
            opcode: 0x0C, form: Form::Short, operand_count: OperandCount::One,
            operands: vec![Operand::Large(0x0010)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&jump, &Unpack::plain(5));
        // target = next_pc + offset - 2 = 0x1000 + 0x10 - 2 = 0x100e
        assert!(s.contains("0x00100e"), "got {s:?}");
    }

    #[test]
    fn describe_mnemonic_covers_the_opcode_set_and_rejects_unknowns() {
        // Every mnemonic the disassembler can emit (except the op:* placeholders)
        // must have a help string — otherwise a hover shows a bare mnemonic.
        for count in [
            OperandCount::Two, OperandCount::One,
            OperandCount::Zero, OperandCount::Var, OperandCount::Ext,
        ] {
            for opcode in 0u8..=0x1f {
                for &version in &[3u8, 5, 8] {
                    let name = mnemonic(&count, opcode, version);
                    if name.starts_with("op:") {
                        continue; // unassigned opcode — no help expected
                    }
                    assert!(
                        describe_mnemonic(name).is_some(),
                        "no help for {name} ({count:?} 0x{opcode:02x} v{version})"
                    );
                }
            }
        }
        // A couple of spot checks on the content.
        assert!(describe_mnemonic("loadw").unwrap().contains("array"));
        assert!(describe_mnemonic("je").unwrap().to_lowercase().contains("equal"));
        // Unknown / placeholder mnemonics have no help.
        assert_eq!(describe_mnemonic("op:2op"), None);
        assert_eq!(describe_mnemonic("nonsense"), None);
    }

    #[test]
    fn describe_instruction_folds_in_operand_roles_store_and_branch() {
        use crate::cpu::decode::Form;
        // call_1s #0x0a2f -> local0 : header + routine role (unpacked) + store.
        let call = Instr {
            opcode: 0x08, form: Form::Short, operand_count: OperandCount::One,
            operands: vec![Operand::Large(0x0a2f)],
            store: Some(1), branch: None, text: None, next_pc: 0x1000,
        };
        let lines = describe_instruction(&call, 3, &Unpack::plain(3));
        assert!(lines[0].starts_with("call_1s — "), "got {:?}", lines[0]);
        assert!(lines.iter().any(|l| l.contains("routine to call") && l.contains("0x00145e")),
            "missing unpacked routine role: {lines:?}");
        assert!(lines.iter().any(|l| l.contains("stores the routine's return value in local0")),
            "missing store line: {lines:?}");

        // je #01, #02 ?true : header + two plain operands + branch sense.
        let je = Instr {
            opcode: 0x01, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(1), Operand::Small(2)],
            store: None, branch: Some(Branch { on_true: true, offset: 4, len: 1 }),
            text: None, next_pc: 0x1000,
        };
        let lines = describe_instruction(&je, 5, &Unpack::plain(5));
        assert!(lines[0].starts_with("je — "), "got {:?}", lines[0]);
        assert!(lines.iter().any(|l| l.starts_with("  → branches if ")),
            "missing branch line: {lines:?}");

        // je … ?~ : an inverted branch reads "unless …" and names the `~` as the
        // cause, so the opcode help alone explains the `?~` sigil.
        let je_inv = Instr {
            opcode: 0x01, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(1), Operand::Small(2)],
            store: None, branch: Some(Branch { on_true: false, offset: 4, len: 1 }),
            text: None, next_pc: 0x1000,
        };
        let lines = describe_instruction(&je_inv, 5, &Unpack::plain(5));
        assert!(
            lines.iter().any(|l| l.starts_with("  → branches unless ") && l.contains("(the ~ inverts the branch)")),
            "inverted branch should read 'unless …' and name the ~: {lines:?}"
        );

        // store #10, #01 : VarRef operand 0 names the target variable.
        let store = Instr {
            opcode: 0x0D, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(0x10), Operand::Small(0x01)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        let lines = describe_instruction(&store, 5, &Unpack::plain(5));
        // variable 0x10 = global g00 (16 → g00); the value renders as the named var.
        assert!(lines.iter().any(|l| l.contains("arg1 = g00:")),
            "VarRef operand should name the variable: {lines:?}");

        // A variable used AS a memory address (loadw base) renders `@sp` — the
        // `@` marks it as an address the debugger links to memory[sp]; it is NOT
        // the `@0x……` constant-address form.
        let loadw_var = Instr {
            opcode: 0x0F, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Var(0), Operand::Small(0x00)],
            store: Some(0), branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&loadw_var, &Unpack::plain(5));
        assert!(s.contains("@sp"), "var memory base should render @sp: {s:?}");
        assert!(!s.contains("@0x"), "not the constant-address form: {s:?}");

        // A variable in a NON-address role stays a plain sigil (no `@`).
        // add sp, #01 (2OP:0x14) — operand 0 is a Plain value, not an address.
        let add_var = Instr {
            opcode: 0x14, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Var(0), Operand::Small(0x01)],
            store: Some(1), branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&add_var, &Unpack::plain(5));
        assert!(s.contains("sp") && !s.contains("@sp"), "plain var must not be @-linked: {s:?}");

        // A variable used AS an object (get_child operand) renders `obj#local0`
        // — an object link the debugger resolves to object[local0].
        let getchild_var = Instr {
            opcode: 0x02, form: Form::Short, operand_count: OperandCount::One,
            operands: vec![Operand::Var(1)],
            store: Some(0), branch: Some(Branch { on_true: true, offset: 4, len: 1 }),
            text: None, next_pc: 0x1000,
        };
        let s = format_instr(&getchild_var, &Unpack::plain(5));
        assert!(s.contains("obj#local0"), "var object should render obj#local0: {s:?}");
    }

    #[test]
    fn operand_role_marks_indirect_variable_references() {
        // load/inc/dec (1OP), store/dec_chk/inc_chk (2OP), pull (VAR): operand 0
        // is a variable number, not a value.
        assert_eq!(operand_role(&OperandCount::One, 0x0E, 5, 0), OpRole::VarRef); // load
        assert_eq!(operand_role(&OperandCount::One, 0x05, 5, 0), OpRole::VarRef); // inc
        assert_eq!(operand_role(&OperandCount::One, 0x06, 5, 0), OpRole::VarRef); // dec
        assert_eq!(operand_role(&OperandCount::Two, 0x0D, 5, 0), OpRole::VarRef); // store
        assert_eq!(operand_role(&OperandCount::Two, 0x0D, 5, 1), OpRole::Plain);  // store's value
        assert_eq!(operand_role(&OperandCount::Two, 0x04, 5, 0), OpRole::VarRef); // dec_chk
        assert_eq!(operand_role(&OperandCount::Two, 0x05, 5, 0), OpRole::VarRef); // inc_chk
        assert_eq!(operand_role(&OperandCount::Var, 0x09, 5, 0), OpRole::VarRef); // pull
        // push (VAR:0x08) pushes a VALUE — not an indirect variable reference.
        assert_eq!(operand_role(&OperandCount::Var, 0x08, 5, 0), OpRole::Plain);
    }

    #[test]
    fn format_instr_renders_varref_as_the_named_variable() {
        use crate::cpu::decode::Form;
        // store #0x10, #0x05 -> target is variable 0x10 = g00; the value stays plain.
        let store = Instr {
            opcode: 0x0D, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(0x10), Operand::Small(0x05)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr(&store, &Unpack::plain(5));
        assert!(s.contains("g00"), "target variable named, not a constant: {s:?}");
        assert!(s.contains("#05"), "the value operand stays a plain constant: {s:?}");
        // load #0x00 -> reads variable 0 = the stack (store target set elsewhere).
        let load = Instr {
            opcode: 0x0E, form: Form::Short, operand_count: OperandCount::One,
            operands: vec![Operand::Small(0x00)],
            store: Some(0x10), branch: None, text: None, next_pc: 0x1000,
        };
        assert!(format_instr(&load, &Unpack::plain(5)).contains("load sp"), "var 0 = stack");
        // inc #0x01 -> variable 1 = local0.
        let inc = Instr {
            opcode: 0x05, form: Form::Short, operand_count: OperandCount::One,
            operands: vec![Operand::Small(0x01)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        assert!(format_instr(&inc, &Unpack::plain(5)).contains("local0"), "var 1 = local0");
    }

    #[test]
    fn format_instr_basic_uses_plain_operands_no_reference_following() {
        use crate::cpu::decode::Form;
        // loadw #0x0abc, #00 -> V01: base is a plain `#0abc` (NO `@0x`), store named.
        let loadw = Instr {
            opcode: 0x0F, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Large(0x0abc), Operand::Small(0x00)],
            store: Some(1), branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr_basic(&loadw, 5);
        assert_eq!(s, "loadw #0abc, #00 -> local0", "got {s:?}");
        assert!(!s.contains("@0x"), "no memory sigil in basic: {s:?}");

        // store #0x10, #01: operand 0 stays a plain `#10` (NO VarRef `g00`).
        let store = Instr {
            opcode: 0x0D, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(0x10), Operand::Small(0x01)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr_basic(&store, 5);
        assert_eq!(s, "store #10, #01", "got {s:?}");
        assert!(!s.contains("g00"), "no VarRef naming in basic: {s:?}");

        // An actual Var(0) operand still renders `sp` (runtime variable sigil).
        let loadw_var = Instr {
            opcode: 0x0F, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Var(0), Operand::Small(0x00)],
            store: Some(0), branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr_basic(&loadw_var, 5);
        assert!(s.contains("sp"), "got {s:?}");

        // A branch renders the computed `?0x……` target (same as Full).
        let je = Instr {
            opcode: 0x01, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(1), Operand::Small(2)],
            store: None, branch: Some(Branch { on_true: true, offset: 5, len: 1 }),
            text: None, next_pc: 0x1000,
        };
        let s = format_instr_basic(&je, 5);
        // target = next_pc + offset - 2 = 0x1000 + 5 - 2 = 0x1003
        assert!(s.contains("?0x001003"), "got {s:?}");

        // print_char #0x41 -> 'A' (char special-case preserved).
        let pc = Instr {
            opcode: 0x05, form: Form::Variable, operand_count: OperandCount::Var,
            operands: vec![Operand::Small(0x41)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        assert!(format_instr_basic(&pc, 5).contains("'A'"), "got {:?}", format_instr_basic(&pc, 5));
    }

    #[test]
    fn disassemble_basic_prefixes_address_and_names_mnemonics() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let start = mem.initial_pc();
        let lines = disassemble_basic(&mem, start, mem.version(), 8);
        assert!(!lines.is_empty());
        // Basic names mnemonics (like Full) but never emits role sigils/annotations.
        assert!(lines.iter().any(|l| !l.contains("op:")), "all fallbacks: {lines:?}");
        assert!(lines.iter().all(|l| !l.contains("@0x") && !l.contains("obj#") && !l.contains('[')),
            "basic view leaked a reference sigil/annotation: {lines:?}");
        // Translated-path address separator (two spaces), not the raw `:` form.
        assert!(lines[0].as_bytes()[6] == b' ', "addr sep: {:?}", lines[0]);
    }

    #[test]
    fn disassembles_a_real_routine_without_all_fallbacks() {
        // Real story fixture — sample_story() has all-zero code memory, which
        // would trivially fall back and defeat the point of this oracle test.
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let start = mem.initial_pc();
        let lines = disassemble(&mem, start, mem.version(), 8);
        assert!(!lines.is_empty());
        assert!(lines.iter().any(|l| !l.contains("op:")), "all fallbacks: {lines:?}");
    }

    #[test]
    fn prev_instr_round_trips_with_next_instr() {
        // Z-code is variable-length and reading it out of alignment can decode
        // *something* plausible-looking, so `prev_instr` isn't guaranteed to
        // recover the bit-exact original start in every case (see its doc
        // comment). What IS guaranteed by construction: whatever start it
        // returns is a real instruction boundary that `next_instr` maps
        // straight back to `addr` — scrolling up then down never leaves the
        // disasm view stuck or jumps to an unrelated line.
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let mut pc = mem.initial_pc();
        let mut exact_matches = 0;
        for _ in 0..16 {
            let next = next_instr(&mem, pc, version);
            let back = prev_instr(&mem, next, version);
            assert_eq!(next_instr(&mem, back, version), next, "round-trip broke at next={next:#x}");
            if back == pc { exact_matches += 1; }
            pc = next;
        }
        // Spot-check: in real code (not just isolated bytes) it recovers the
        // exact original predecessor the large majority of the time.
        assert!(exact_matches >= 12, "only {exact_matches}/16 exact matches");
    }

    #[test]
    fn prev_instr_of_zero_is_zero() {
        let bytes = crate::header::tests_support::sample_story(5);
        let mem = Memory::new(bytes).unwrap();
        assert_eq!(prev_instr(&mem, 0, mem.version()), 0);
    }

    #[test]
    fn class_tag_covers_each_operand_count() {
        assert_eq!(class_tag(&OperandCount::Zero), "0OP");
        assert_eq!(class_tag(&OperandCount::One), "1OP");
        assert_eq!(class_tag(&OperandCount::Two), "2OP");
        assert_eq!(class_tag(&OperandCount::Var), "VAR");
        assert_eq!(class_tag(&OperandCount::Ext), "EXT");
    }

    #[test]
    fn format_instr_raw_shows_bytes_and_untranslated_decode() {
        use crate::cpu::decode::Form;
        // add S#05, S#03 -> V05  (2OP:0x14, store var 5). Raw: NO mnemonic name.
        let instr = Instr {
            opcode: 0x14, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(5), Operand::Small(3)],
            store: Some(5), branch: None, text: None, next_pc: 0x1004,
        };
        let s = format_instr_raw(&instr, &[0x54, 0x05, 0x03, 0x05], false);
        assert_eq!(s, "54 05 03 05   2OP:0x14  S#05, S#03 -> V05");
        assert!(!s.contains("add"), "raw view must not name the mnemonic: {s:?}");
        // A Large operand renders `L#{:04x}`.
        let big = Instr {
            opcode: 0x00, form: Form::Variable, operand_count: OperandCount::Var,
            operands: vec![Operand::Large(0x0a2f)],
            store: None, branch: None, text: None, next_pc: 0x1000,
        };
        let s = format_instr_raw(&big, &[], false);
        assert!(s.contains("VAR:0x00"), "got {s:?}");
        assert!(s.contains("L#0a2f"), "raw large is not unpacked: {s:?}");
        assert!(!s.contains("0x00145e"), "raw must not unpack packed addresses: {s:?}");
    }

    #[test]
    fn format_instr_raw_never_names_variables() {
        use crate::cpu::decode::Form;
        // Var(0) is the stack, but raw shows V00 — never `sp`; store shows raw V01.
        let instr = Instr {
            opcode: 0x0F, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Var(0), Operand::Small(0)],
            store: Some(1), branch: None, text: None, next_pc: 0x1004,
        };
        let s = format_instr_raw(&instr, &[0x6f, 0x00, 0x00, 0x01], false);
        assert!(s.contains("V00"), "got {s:?}");
        assert!(s.contains("-> V01"), "raw store is a bare variable number: {s:?}");
        assert!(!s.contains("sp"), "raw view must not name variables: {s:?}");
        assert!(!s.contains("local"), "got {s:?}");
    }

    #[test]
    fn format_instr_raw_renders_raw_branch_offset() {
        use crate::cpu::decode::Form;
        let on_true = Instr {
            opcode: 0x01, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(1), Operand::Small(2)],
            store: None, branch: Some(Branch { on_true: true, offset: 5, len: 1 }),
            text: None, next_pc: 0x1000,
        };
        assert!(format_instr_raw(&on_true, &[], false).ends_with("?T5"),
            "raw branch is the signed offset, no target address: {:?}", format_instr_raw(&on_true, &[], false));
        let on_false = Instr {
            opcode: 0x01, form: Form::Long, operand_count: OperandCount::Two,
            operands: vec![Operand::Small(1), Operand::Small(2)],
            store: None, branch: Some(Branch { on_true: false, offset: -3, len: 2 }),
            text: None, next_pc: 0x1000,
        };
        assert!(format_instr_raw(&on_false, &[], false).ends_with("?F-3"),
            "got {:?}", format_instr_raw(&on_false, &[], false));
    }

    #[test]
    fn disassemble_raw_prefixes_address_and_caps_the_byte_column() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let start = mem.initial_pc();
        let lines = disassemble_raw(&mem, start, mem.version(), 8);
        assert!(!lines.is_empty());
        // Raw lines carry a class tag, never a mnemonic.
        assert!(lines.iter().any(|l| l.contains(":0x")), "no class:opcode: {lines:?}");
        assert!(lines.iter().all(|l| !l.contains("add") && !l.contains("call")),
            "raw view leaked a mnemonic: {lines:?}");
        // Address column uses `{:06x}: ` (colon), distinct from the translated path.
        assert!(lines[0].as_bytes()[6] == b':', "addr sep: {:?}", lines[0]);
    }

    #[test]
    fn prev_instr_never_returns_at_or_after_addr() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let start = mem.initial_pc();
        for addr in start..start + 64 {
            assert!(prev_instr(&mem, addr, version) < addr, "addr={addr:#x}");
        }
    }
}
