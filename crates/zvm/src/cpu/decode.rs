//! Z-machine instruction decoder — ZMSD §4, §14.
//!
//! Decodes one instruction at `pc` into a structured [`Instr`] value.
//! Four instruction forms: Long, Short, Variable, Extended.
//! After operands, reads store/branch/text bytes per the opcode's signature.

use crate::memory::Memory;
use crate::text::decode::decode_string;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single operand value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Operand {
    /// 2-byte large constant.
    Large(u16),
    /// 1-byte small constant.
    Small(u8),
    /// 1-byte variable reference.
    Var(u8),
}

/// Instruction encoding form (ZMSD §4.3).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Form {
    Long,
    Short,
    Variable,
    Extended,
}

/// Operand count class — disambiguates same-numbered opcodes in different classes.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum OperandCount {
    Zero,
    One,
    Two,
    Var,
    Ext,
}

/// Decoded branch target.
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    /// Branch when condition is true (bit 7 of first branch byte).
    pub on_true: bool,
    /// Signed offset. 0 = return false, 1 = return true, else PC-relative.
    pub offset: i16,
    /// Number of bytes the branch descriptor occupies (1 for short form, 2 for long).
    pub len: u8,
}

/// A fully decoded Z-machine instruction.
#[derive(Debug, Clone)]
pub struct Instr {
    /// Raw opcode number within its operand-count class.
    pub opcode: u8,
    /// Encoding form.
    pub form: Form,
    /// Operand count class.
    pub operand_count: OperandCount,
    /// Decoded operands (0–8).
    pub operands: Vec<Operand>,
    /// Store variable, if this opcode stores a result.
    pub store: Option<u8>,
    /// Branch info, if this opcode branches.
    pub branch: Option<Branch>,
    /// Inline text (string, address-past-end), if this opcode carries text.
    pub text: Option<(String, u32)>,
    /// Address of the first byte after this instruction.
    pub next_pc: u32,
}

// ---------------------------------------------------------------------------
// Opcode signature table — (stores, branches, has_text)
// Based on ZMSD §14.
// ---------------------------------------------------------------------------

/// Returns (stores, branches, has_text) for 2OP opcodes.
fn two_op_sig(opcode: u8) -> (bool, bool, bool) {
    match opcode {
        0x01 => (false, true, false),  // je
        0x02 => (false, true, false),  // jl
        0x03 => (false, true, false),  // jg
        0x04 => (false, true, false),  // dec_chk
        0x05 => (false, true, false),  // inc_chk
        0x06 => (false, true, false),  // jin
        0x07 => (false, true, false),  // test
        0x08 => (true, false, false),  // or
        0x09 => (true, false, false),  // and
        0x0A => (false, true, false),  // test_attr
        0x0B => (false, false, false), // set_attr
        0x0C => (false, false, false), // clear_attr
        0x0D => (false, false, false), // store
        0x0E => (false, false, false), // insert_obj
        0x0F => (true, false, false),  // loadw
        0x10 => (true, false, false),  // loadb
        0x11 => (true, false, false),  // get_prop
        0x12 => (true, false, false),  // get_prop_addr
        0x13 => (true, false, false),  // get_next_prop
        0x14 => (true, false, false),  // add
        0x15 => (true, false, false),  // sub
        0x16 => (true, false, false),  // mul
        0x17 => (true, false, false),  // div
        0x18 => (true, false, false),  // mod
        0x19 => (true, false, false),  // call_2s (v4+)
        0x1A => (false, false, false), // call_2n (v5+)
        0x1B => (false, false, false), // set_colour (v5+)
        0x1C => (false, false, false), // throw (v5+)
        _ => (false, false, false),
    }
}

/// Returns (stores, branches, has_text) for 1OP opcodes.
/// Version-dependent: 1OP:0x0F is `not` (stores) in v1-4, `call_1n` (no store) in v5+.
fn one_op_sig(opcode: u8, version: u8) -> (bool, bool, bool) {
    match opcode {
        0x00 => (false, true, false),  // jz (branches only)
        0x01 => (true, true, false),   // get_sibling (stores + branches)
        0x02 => (true, true, false),   // get_child (stores + branches)
        0x03 => (true, false, false),  // get_parent (stores)
        0x04 => (true, false, false),  // get_prop_len (stores)
        0x05 => (false, false, false), // inc
        0x06 => (false, false, false), // dec
        0x07 => (false, false, false), // print_addr
        0x08 => (true, false, false),  // call_1s (v4+, stores)
        0x09 => (false, false, false), // remove_obj
        0x0A => (false, false, false), // print_obj
        0x0B => (false, false, false), // ret
        0x0C => (false, false, false), // jump
        0x0D => (false, false, false), // print_paddr
        0x0E => (true, false, false),  // load (stores)
        0x0F
            // `not` (stores) in v1–4; `call_1n` (no store) in v5+
            if version <= 4 => { (true, false, false) }
        _ => (false, false, false),
    }
}

/// Returns (stores, branches, has_text) for 0OP opcodes.
/// Version-dependent:
///   0x02 print / 0x03 print_ret carry inline text.
///   0x05 save / 0x06 restore: branch in v3, store in v4+.
///   0x09: pop (nothing) in v1-4, catch (stores) in v5+.
fn zero_op_sig(opcode: u8, version: u8) -> (bool, bool, bool) {
    match opcode {
        0x00 => (false, false, false), // rtrue
        0x01 => (false, false, false), // rfalse
        0x02 => (false, false, true),  // print (has inline text)
        0x03 => (false, false, true),  // print_ret (has inline text)
        0x04 => (false, false, false), // nop
        0x05 => {
            // save: branch in v3, store in v4+
            if version <= 3 { (false, true, false) } else { (true, false, false) }
        }
        0x06 => {
            // restore: branch in v3, store in v4+
            if version <= 3 { (false, true, false) } else { (true, false, false) }
        }
        0x07 => (false, false, false), // restart
        0x08 => (false, false, false), // ret_popped
        0x09 => {
            // pop in v1-4; catch (stores) in v5+
            if version <= 4 { (false, false, false) } else { (true, false, false) }
        }
        0x0A => (false, false, false), // quit
        0x0B => (false, false, false), // new_line
        0x0C => (false, false, false), // show_status (v3) / illegal (v4+)
        0x0D => (false, true, false),  // verify (branches)
        0x0E => (false, false, false), // [extended prefix — should not appear here]
        0x0F => (false, true, false),  // piracy (branches)
        _ => (false, false, false),
    }
}

/// Returns (stores, branches, has_text) for VAR opcodes.
fn var_op_sig(opcode: u8, version: u8) -> (bool, bool, bool) {
    match opcode {
        0x00 => (true, false, false),  // call / call_vs (stores)
        0x01 => (false, false, false), // storew
        0x02 => (false, false, false), // storeb
        0x03 => (false, false, false), // put_prop
        0x04 => {
            // sread/read: stores in v5+ (returns terminator char)
            if version >= 5 { (true, false, false) } else { (false, false, false) }
        }
        0x05 => (false, false, false), // print_char
        0x06 => (false, false, false), // print_num
        0x07 => (true, false, false),  // random (stores)
        0x08 => (false, false, false), // push
        0x09 => (false, false, false), // pull (v1-5). v6 always stores — see the
                                       // dedicated disambiguation after this lookup.
        0x0A => (false, false, false), // split_window
        0x0B => (false, false, false), // set_window
        0x0C => (true, false, false),  // call_vs2 (v4+, stores, uses 2 type bytes)
        0x0D => (false, false, false), // erase_window (v4+)
        0x0E => (false, false, false), // erase_line (v4+)
        0x0F => (false, false, false), // set_cursor (v4+)
        0x10 => (false, false, false), // get_cursor (v4+)
        0x11 => (false, false, false), // set_text_style (v4+)
        0x12 => (false, false, false), // buffer_mode (v4+)
        0x13 => (false, false, false), // output_stream (v3+)
        0x14 => (false, false, false), // input_stream (v3+)
        0x15 => (false, false, false), // sound_effect (v3+)
        0x16 => (true, false, false),  // read_char (v4+, stores)
        0x17 => (true, true, false),   // scan_table (v4+, stores AND branches)
        0x18 => (true, false, false),  // not (v5+, stores)
        0x19 => (false, false, false), // call_vn (v5+)
        0x1A => (false, false, false), // call_vn2 (v5+, uses 2 type bytes)
        0x1B => (false, false, false), // tokenise (v5+)
        0x1C => (false, false, false), // encode_text (v5+)
        0x1D => (false, false, false), // copy_table (v5+)
        0x1E => (false, false, false), // print_table (v5+)
        0x1F => (false, true, false),  // check_arg_count (v5+, branches)
        _ => (false, false, false),
    }
}

/// Returns (stores, branches, has_text) for EXT opcodes (v5+).
fn ext_op_sig(opcode: u8) -> (bool, bool, bool) {
    match opcode {
        0x00 => (true, false, false),  // save (stores)
        0x01 => (true, false, false),  // restore (stores)
        0x02 => (true, false, false),  // log_shift (stores)
        0x03 => (true, false, false),  // art_shift (stores)
        0x04 => (true, false, false),  // set_font (stores)
        0x06 => (false, true, false),  // picture_data (branches) — v6
        0x09 => (true, false, false),  // save_undo (stores)
        0x0A => (true, false, false),  // restore_undo (stores)
        0x0B => (false, false, false), // print_unicode (v5+)
        0x0C => (true, false, false),  // check_unicode (v5+, stores)
        0x13 => (true, false, false),  // get_wind_prop (stores) — v6
        0x18 => (false, true, false),  // push_stack (branches) — v6
        0x1B => (false, true, false),  // make_menu (branches) — v6
        0x1D => (true, false, false),  // buffer_screen (stores) — v6
        // v6 no-store/no-branch: draw_picture 0x05, erase_picture 0x07,
        // set_margins 0x08, move_window 0x10, window_size 0x11,
        // window_style 0x12, scroll_window 0x14, pop_stack 0x15,
        // read_mouse 0x16, mouse_window 0x17, put_wind_prop 0x19,
        // print_form 0x1A, picture_table 0x1C
        _ => (false, false, false),
    }
}

/// Dispatch to the appropriate signature function.
fn get_signature(oc: &OperandCount, opcode: u8, version: u8) -> (bool, bool, bool) {
    match oc {
        OperandCount::Two  => two_op_sig(opcode),
        OperandCount::One  => one_op_sig(opcode, version),
        OperandCount::Zero => zero_op_sig(opcode, version),
        OperandCount::Var  => var_op_sig(opcode, version),
        OperandCount::Ext  => ext_op_sig(opcode),
    }
}

// ---------------------------------------------------------------------------
// Operand reading helpers
// ---------------------------------------------------------------------------

/// Read one operand from `*pc` given a 2-bit type code.
/// Returns `None` for type 0b11 (omitted).
fn read_operand(mem: &Memory, typ: u8, pc: &mut u32) -> Option<Operand> {
    match typ & 0b11 {
        0b00 => {
            let v = mem.read_word(*pc);
            *pc += 2;
            Some(Operand::Large(v))
        }
        0b01 => {
            let v = mem.read_byte(*pc);
            *pc += 1;
            Some(Operand::Small(v))
        }
        0b10 => {
            let v = mem.read_byte(*pc);
            *pc += 1;
            Some(Operand::Var(v))
        }
        _ => None, // 0b11 = omitted; stop
    }
}

/// Read up to 4 operands from the given type byte (MSB pair first).
/// Stops at the first omitted slot.
fn read_operands_from_type_byte(
    mem: &Memory,
    type_byte: u8,
    pc: &mut u32,
    operands: &mut Vec<Operand>,
) {
    for shift in [6u8, 4, 2, 0] {
        let typ = (type_byte >> shift) & 0b11;
        match read_operand(mem, typ, pc) {
            Some(op) => operands.push(op),
            None => break, // omitted means no further operands in this byte
        }
    }
}

// ---------------------------------------------------------------------------
// Form / opcode-class decoding
// ---------------------------------------------------------------------------

/// Determine encoding form, operand-count class, and raw opcode number.
/// Advances `cursor` past the extended opcode byte if needed.
fn decode_form(
    mem: &Memory,
    opcode_byte: u8,
    cursor: &mut u32,
    version: u8,
) -> (Form, OperandCount, u8) {
    // Extended form: opcode byte 0xBE in v5+
    if opcode_byte == 0xBE && version >= 5 {
        let ext_op = mem.read_byte(*cursor);
        *cursor += 1;
        return (Form::Extended, OperandCount::Ext, ext_op);
    }

    match opcode_byte >> 6 {
        // Long form: top bit 0 (0x00–0x7F)
        0b00 | 0b01 => {
            let opcode = opcode_byte & 0x1F;
            (Form::Long, OperandCount::Two, opcode)
        }
        // Short form: top two bits 10 (0x80–0xBF)
        0b10 => {
            let typ = (opcode_byte >> 4) & 0b11;
            let opcode = opcode_byte & 0x0F;
            let oc = if typ == 0b11 { OperandCount::Zero } else { OperandCount::One };
            (Form::Short, oc, opcode)
        }
        // Variable form: top two bits 11 (0xC0–0xFF)
        _ => {
            let opcode = opcode_byte & 0x1F;
            // Bit 5 distinguishes VAR from 2OP
            let oc = if (opcode_byte & 0x20) != 0 { OperandCount::Var } else { OperandCount::Two };
            (Form::Variable, oc, opcode)
        }
    }
}

// ---------------------------------------------------------------------------
// Branch decoding
// ---------------------------------------------------------------------------

/// Decode a branch descriptor at `addr`. The Z-machine short form (bit 6 set) is
/// one byte with a 6-bit unsigned offset; the long form is two bytes with a
/// 14-bit signed offset. `Branch::len` reports how many bytes were consumed.
pub fn decode_branch_at(mem: &crate::memory::Memory, addr: u32) -> Branch {
    let b0 = mem.read_byte(addr);
    let on_true = (b0 & 0x80) != 0;
    if (b0 & 0x40) != 0 {
        Branch { on_true, offset: (b0 & 0x3F) as i16, len: 1 }
    } else {
        let b1 = mem.read_byte(addr + 1);
        let raw = (((b0 & 0x3F) as u16) << 8) | (b1 as u16);
        let offset = if raw & 0x2000 != 0 { (raw | 0xC000) as i16 } else { raw as i16 };
        Branch { on_true, offset, len: 2 }
    }
}

// ---------------------------------------------------------------------------
// Main decode entry point
// ---------------------------------------------------------------------------

/// Decode the instruction at `pc` in `mem` for Z-machine `version`.
pub fn decode(mem: &Memory, pc: u32, version: u8) -> Instr {
    let mut cursor = pc;
    let opcode_byte = mem.read_byte(cursor);
    cursor += 1;

    let (form, operand_count, opcode) = decode_form(mem, opcode_byte, &mut cursor, version);

    // Read operands based on form
    let mut operands: Vec<Operand> = Vec::new();
    match form {
        Form::Long => {
            // Bits 6 and 5 of opcode_byte give operand types:
            //   0 = small constant, 1 = variable
            let typ1: u8 = if (opcode_byte & 0b0100_0000) != 0 { 0b10 } else { 0b01 };
            let typ2: u8 = if (opcode_byte & 0b0010_0000) != 0 { 0b10 } else { 0b01 };
            if let Some(op) = read_operand(mem, typ1, &mut cursor) { operands.push(op); }
            if let Some(op) = read_operand(mem, typ2, &mut cursor) { operands.push(op); }
        }
        Form::Short => {
            // Bits 5-4 give the single operand's type; 0b11 = 0OP (no operand)
            let typ = (opcode_byte >> 4) & 0b11;
            if typ != 0b11 {
                if let Some(op) = read_operand(mem, typ, &mut cursor) { operands.push(op); }
            }
        }
        Form::Variable | Form::Extended => {
            // call_vs2 (VAR:0x0C) and call_vn2 (VAR:0x1A) use two type bytes.
            // The two-type-byte rule only applies to the VAR opcode class, NOT to
            // 2OP opcodes encoded in variable form (e.g. call_2n = 2OP:0x1A).
            let two_type_bytes = form == Form::Variable
                && operand_count == OperandCount::Var
                && (opcode == 0x0C || opcode == 0x1A);

            if two_type_bytes {
                // Read BOTH type bytes first (before reading any operands), then decode
                // operands for both.  This is necessary because the second type byte
                // immediately follows the first, and operands come after both.
                let type_byte1 = mem.read_byte(cursor);
                cursor += 1;
                let type_byte2 = mem.read_byte(cursor);
                cursor += 1;
                read_operands_from_type_byte(mem, type_byte1, &mut cursor, &mut operands);
                read_operands_from_type_byte(mem, type_byte2, &mut cursor, &mut operands);
            } else {
                let type_byte1 = mem.read_byte(cursor);
                cursor += 1;
                read_operands_from_type_byte(mem, type_byte1, &mut cursor, &mut operands);
            }
        }
    }

    // Look up signature: does this opcode store / branch / carry text?
    let (mut stores, branches, has_text) = get_signature(&operand_count, opcode, version);

    // v6 `pull` (ZMSD §15): the instruction is `pull stack -> (result)` — it
    // ALWAYS carries a store byte in v6, whatever the operand encoding (frotz
    // z_pull calls store() unconditionally in its V6 branch; user vs game stack
    // is picked by argument count at execution, not operand type). Missing the
    // store byte mis-reads it as the next opcode, silently corrupting all
    // following decode — the failure behind the SQ-0452 parser soft-lock. An
    // earlier fix keyed this off a Large-constant operand only, which repaired
    // direction parsing but left Zork Zero's verb path (Var-operand encoding)
    // corrupted.
    if version == 6 && operand_count == OperandCount::Var && opcode == 0x09 {
        stores = true;
    }

    // Store variable byte (read after operands)
    let store = if stores {
        let v = mem.read_byte(cursor);
        cursor += 1;
        Some(v)
    } else {
        None
    };

    // Branch bytes (read after store)
    let branch = if branches {
        let br = decode_branch_at(mem, cursor);
        cursor += br.len as u32;
        Some(br)
    } else {
        None
    };

    // Inline text (read after branch)
    let text = if has_text {
        let (s, end) = decode_string(mem, cursor);
        cursor = end;
        Some((s, end))
    } else {
        None
    };

    Instr {
        opcode,
        form,
        operand_count,
        operands,
        store,
        branch,
        text,
        next_pc: cursor,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;

    // (a) Long-form 2OP add: opcode 0x14, small+small operands, store byte.
    // From brief: exact test case required.
    #[test]
    fn decodes_long_form_add() {
        // 0x14 = 0b0001_0100: long form (bit7=0), bits6,5=00 → both small constants, opcode=20
        let mut m = Memory::new(sample_story(5)).unwrap();
        m.write_byte(0x10, 0x14); // add, small, small
        m.write_byte(0x11, 0x02); // operand a = 2
        m.write_byte(0x12, 0x03); // operand b = 3
        m.write_byte(0x13, 0x05); // store -> var 5
        let ins = decode(&m, 0x10, 5);
        assert_eq!(ins.opcode, 20);
        assert_eq!(ins.form, Form::Long);
        assert_eq!(ins.operand_count, OperandCount::Two);
        assert_eq!(ins.operands.len(), 2);
        assert_eq!(ins.operands[0], Operand::Small(2));
        assert_eq!(ins.operands[1], Operand::Small(3));
        assert_eq!(ins.store, Some(5));
        assert_eq!(ins.branch, None);
        assert_eq!(ins.next_pc, 0x14);
    }

    // (b) Short-form 1OP jz (opcode=0) with large-constant operand and two-byte branch.
    #[test]
    fn decodes_short_form_jz_with_two_byte_branch() {
        // 0x80 = 0b1000_0000: short form (bits7-6=10), bits5-4=00 → large constant, opcode=0 (jz)
        // Offset = 100 in 14-bit form:
        //   100 = 0b00_0000_0110_0100; high6=0b000000, low8=0x64
        //   b0 = on_true(1) | two-byte(0) | high6 = 0b1000_0000 | 0x00 = 0x80
        //   b1 = 0x64
        let mut m = Memory::new(sample_story(3)).unwrap();
        m.write_byte(0x10, 0x80); // jz, large constant
        m.write_word(0x11, 0xABCD); // operand value
        m.write_byte(0x13, 0x80);   // branch: on_true=1, two-byte form, high6=0
        m.write_byte(0x14, 0x64);   // low8 = 100
        let ins = decode(&m, 0x10, 3);
        assert_eq!(ins.opcode, 0);
        assert_eq!(ins.form, Form::Short);
        assert_eq!(ins.operand_count, OperandCount::One);
        assert_eq!(ins.operands.len(), 1);
        assert_eq!(ins.operands[0], Operand::Large(0xABCD));
        let br = ins.branch.as_ref().unwrap();
        assert!(br.on_true);
        assert_eq!(br.offset, 100);
        assert_eq!(ins.next_pc, 0x15);
    }

    // (c) Short-form 0OP print (0xB2) with inline Z-string text.
    #[test]
    fn decodes_short_form_print_with_text() {
        // 0xB2 = 0b1011_0010: short form (bits7-6=10), bits5-4=11 → 0OP, opcode=2 (print)
        // Z-string "abc": Z-chars [6,7,8] packed into one word with high bit set.
        let mut m = Memory::new(sample_story(3)).unwrap();
        m.write_byte(0x10, 0xB2); // print
        let z_word: u16 = 0x8000 | (6 << 10) | (7 << 5) | 8; // "abc"
        m.write_word(0x11, z_word);
        let ins = decode(&m, 0x10, 3);
        assert_eq!(ins.opcode, 2);
        assert_eq!(ins.form, Form::Short);
        assert_eq!(ins.operand_count, OperandCount::Zero);
        assert_eq!(ins.operands.len(), 0);
        assert_eq!(ins.store, None);
        assert_eq!(ins.branch, None);
        let (text, end_addr) = ins.text.as_ref().unwrap();
        assert_eq!(text, "abc");
        assert_eq!(*end_addr, 0x13); // 0x11 + 2 bytes for the Z-word
        assert_eq!(ins.next_pc, 0x13);
    }

    // (d) Variable-form VAR call_vs (0xE0) with type byte and several operands, plus store.
    #[test]
    fn decodes_variable_form_call_vs() {
        // 0xE0 = 0b1110_0000: variable form (bits7-6=11), bit5=1 → VAR, opcode=0 (call_vs)
        // Type byte 0b00_01_10_11 → large, small, var, omitted → 3 operands
        // Operands: large=0x1234 (2 bytes), small=0x05 (1 byte), var=0x10 (1 byte)
        // Store byte: 0x02
        let mut m = Memory::new(sample_story(5)).unwrap();
        m.write_byte(0x10, 0xE0);            // call_vs
        m.write_byte(0x11, 0b00_01_10_11);   // type byte
        m.write_word(0x12, 0x1234);           // large operand
        m.write_byte(0x14, 0x05);             // small operand
        m.write_byte(0x15, 0x10);             // var operand
        m.write_byte(0x16, 0x02);             // store into var 2
        let ins = decode(&m, 0x10, 5);
        assert_eq!(ins.opcode, 0);
        assert_eq!(ins.form, Form::Variable);
        assert_eq!(ins.operand_count, OperandCount::Var);
        assert_eq!(ins.operands.len(), 3);
        assert_eq!(ins.operands[0], Operand::Large(0x1234));
        assert_eq!(ins.operands[1], Operand::Small(0x05));
        assert_eq!(ins.operands[2], Operand::Var(0x10));
        assert_eq!(ins.store, Some(0x02));
        assert_eq!(ins.branch, None);
        assert_eq!(ins.next_pc, 0x17);
    }

    // (e) 14-bit signed negative branch offset decodes correctly.
    #[test]
    fn decodes_14bit_negative_branch_offset() {
        // jz with small constant operand (0x90 = bits5-4=01 → small, opcode=0).
        // Offset = -1 in 14-bit two's complement = 0b11_1111_1111_1111
        //   high6 = 0b111111 = 0x3F, low8 = 0xFF
        //   b0 = on_false(0) | two-byte(0) | 0x3F = 0x3F
        //   b1 = 0xFF
        let mut m = Memory::new(sample_story(3)).unwrap();
        m.write_byte(0x10, 0x90); // jz with small constant
        m.write_byte(0x11, 0x00); // operand = 0
        m.write_byte(0x12, 0x3F); // on_false, two-byte, high6=0x3F
        m.write_byte(0x13, 0xFF); // low8=0xFF → offset = -1
        let ins = decode(&m, 0x10, 3);
        assert_eq!(ins.opcode, 0);
        let br = ins.branch.as_ref().unwrap();
        assert!(!br.on_true);
        assert_eq!(br.offset, -1);
        assert_eq!(ins.next_pc, 0x14);
    }

    // Supplementary: 6-bit (single-byte) branch offset.
    #[test]
    fn decodes_6bit_single_byte_branch_offset() {
        // jz with small const, branch on true, single-byte form, offset=42
        // 0x90 = jz with small const; b0 = on_true | single_byte | 42 = 0x80|0x40|0x2A = 0xEA
        let mut m = Memory::new(sample_story(3)).unwrap();
        m.write_byte(0x10, 0x90); // jz with small constant
        m.write_byte(0x11, 0x05); // operand
        m.write_byte(0x12, 0x80 | 0x40 | 42); // on_true, single-byte, offset=42
        let ins = decode(&m, 0x10, 3);
        let br = ins.branch.as_ref().unwrap();
        assert!(br.on_true);
        assert_eq!(br.offset, 42);
        assert_eq!(ins.next_pc, 0x13);
    }

    // Supplementary: version-dependent 1OP:0x0F (`not` stores in v4; `call_1n` no store in v5).
    #[test]
    fn decodes_1op_0f_version_dependent() {
        // 0x8F = short form (bits7-6=10), bits5-4=00 → large const, opcode=0x0F
        let mut m = Memory::new(sample_story(4)).unwrap();
        m.write_byte(0x10, 0x8F);
        m.write_word(0x11, 0x1234); // large operand
        m.write_byte(0x13, 0x07);   // store byte (only consumed in v4)

        // v4: `not` stores
        let ins_v4 = decode(&m, 0x10, 4);
        assert_eq!(ins_v4.opcode, 0x0F);
        assert_eq!(ins_v4.store, Some(0x07));
        assert_eq!(ins_v4.next_pc, 0x14);

        // v5: `call_1n` does not store
        let ins_v5 = decode(&m, 0x10, 5);
        assert_eq!(ins_v5.opcode, 0x0F);
        assert_eq!(ins_v5.store, None);
        assert_eq!(ins_v5.next_pc, 0x13); // no store byte consumed
    }

    // Supplementary: extended form (v5+) with type byte and store.
    #[test]
    fn decodes_extended_form() {
        // 0xBE = ext prefix; next byte = 0x02 (log_shift, stores)
        // Type byte 0b01_01_11_11 → small, small, omit, omit → 2 operands
        let mut m = Memory::new(sample_story(5)).unwrap();
        m.write_byte(0x10, 0xBE);           // ext prefix
        m.write_byte(0x11, 0x02);           // log_shift
        m.write_byte(0x12, 0b01_01_11_11); // type byte: small, small, omit, omit
        m.write_byte(0x13, 0x0A);           // small operand 1 = 10
        m.write_byte(0x14, 0x02);           // small operand 2 = 2
        m.write_byte(0x15, 0x03);           // store into var 3
        let ins = decode(&m, 0x10, 5);
        assert_eq!(ins.opcode, 0x02);
        assert_eq!(ins.form, Form::Extended);
        assert_eq!(ins.operand_count, OperandCount::Ext);
        assert_eq!(ins.operands.len(), 2);
        assert_eq!(ins.operands[0], Operand::Small(0x0A));
        assert_eq!(ins.operands[1], Operand::Small(0x02));
        assert_eq!(ins.store, Some(0x03));
        assert_eq!(ins.next_pc, 0x16);
    }

    // Supplementary: variable-form 2OP (opcode byte 0xC0–0xDF, bit5=0).
    #[test]
    fn decodes_variable_form_2op() {
        // 0xC1 = 0b1100_0001: variable form (bits7-6=11), bit5=0 → 2OP, opcode=1 (je, branches)
        // Type byte: 0b01_01_11_11 → small, small, omit, omit → 2 operands
        let mut m = Memory::new(sample_story(3)).unwrap();
        m.write_byte(0x10, 0xC1);           // je in variable form
        m.write_byte(0x11, 0b01_01_11_11); // type byte
        m.write_byte(0x12, 0x05);           // small operand 1
        m.write_byte(0x13, 0x05);           // small operand 2
        m.write_byte(0x14, 0x80 | 0x40 | 2); // branch: on_true, single-byte, offset=2
        let ins = decode(&m, 0x10, 3);
        assert_eq!(ins.opcode, 1);
        assert_eq!(ins.form, Form::Variable);
        assert_eq!(ins.operand_count, OperandCount::Two);
        assert_eq!(ins.operands.len(), 2);
        let br = ins.branch.as_ref().unwrap();
        assert!(br.on_true);
        assert_eq!(br.offset, 2);
        assert_eq!(ins.next_pc, 0x15);
    }

    // Regression: scan_table (VAR:0x17) must store AND branch (ZMSD §14).
    // Bug was: marked branch-only → store byte never read → next_pc one byte short.
    #[test]
    fn decodes_scan_table_stores_and_branches() {
        // VAR form opcode byte for VAR:0x17:
        //   bits7-6 = 11 (variable form), bit5 = 1 (VAR class), bits4-0 = 0x17
        //   = 0b1110_0000 | 0x17 = 0xE0 | 0x17 = 0xF7
        // Type byte 0b01_01_01_11 → small, small, small, omitted → 3 operands (3 bytes)
        // Then: store byte (1 byte), branch byte single-byte form (1 byte).
        // Layout at 0x10:
        //   0x10: 0xF7          opcode
        //   0x11: 0b01_01_01_11 type byte (3 small operands)
        //   0x12: 0x01          operand 1
        //   0x13: 0x02          operand 2
        //   0x14: 0x03          operand 3
        //   0x15: 0x04          store variable
        //   0x16: 0xC5          branch byte: on_true (bit7=1), short form (bit6=1), offset=5
        //   next_pc = 0x17
        let mut m = Memory::new(sample_story(5)).unwrap();
        m.write_byte(0x10, 0xF7);           // scan_table opcode
        m.write_byte(0x11, 0b01_01_01_11); // type byte: small,small,small,omitted
        m.write_byte(0x12, 0x01);           // operand 1
        m.write_byte(0x13, 0x02);           // operand 2
        m.write_byte(0x14, 0x03);           // operand 3
        m.write_byte(0x15, 0x04);           // store variable
        m.write_byte(0x16, 0xC0 | 5);      // branch: on_true, short form, offset=5
        let ins = decode(&m, 0x10, 5);
        assert_eq!(ins.opcode, 0x17);
        assert_eq!(ins.form, Form::Variable);
        assert_eq!(ins.operand_count, OperandCount::Var);
        assert_eq!(ins.operands.len(), 3);
        assert!(ins.store.is_some(), "scan_table must store");
        assert_eq!(ins.store, Some(0x04));
        assert!(ins.branch.is_some(), "scan_table must branch");
        let br = ins.branch.as_ref().unwrap();
        assert!(br.on_true);
        assert_eq!(br.offset, 5);
        // 1 opcode + 1 type byte + 3 operand bytes + 1 store byte + 1 branch byte = 7 bytes
        assert_eq!(ins.next_pc, 0x17);
    }

    // Supplementary: long-form with variable operand types (bits 6 and 5 set).
    #[test]
    fn decodes_long_form_with_variable_operands() {
        // 0x74 = 0b0111_0100: long form, bits6=1 (var), bit5=1 (var), opcode=0x14 (add)
        let mut m = Memory::new(sample_story(5)).unwrap();
        m.write_byte(0x10, 0x74); // add, var, var
        m.write_byte(0x11, 0x01); // var operand 1: local 1
        m.write_byte(0x12, 0x02); // var operand 2: local 2
        m.write_byte(0x13, 0x03); // store into var 3
        let ins = decode(&m, 0x10, 5);
        assert_eq!(ins.opcode, 0x14);
        assert_eq!(ins.operands[0], Operand::Var(0x01));
        assert_eq!(ins.operands[1], Operand::Var(0x02));
        assert_eq!(ins.store, Some(0x03));
        assert_eq!(ins.next_pc, 0x14);
    }

    #[test]
    fn decode_branch_at_reports_form_and_length() {
        let mut m = Memory::new(sample_story(3)).unwrap();
        // Single-byte form: on_true, short-form bit, offset 5.
        m.write_byte(0x40, 0x80 | 0x40 | 5);
        let b1 = decode_branch_at(&m, 0x40);
        assert!(b1.on_true);
        assert_eq!(b1.offset, 5);
        assert_eq!(b1.len, 1, "short-form branch is 1 byte");

        // Two-byte form: on_false (bit7=0), long-form (bit6=0), 14-bit offset = -1.
        m.write_byte(0x50, 0x3F); // high6 = 0x3F, sign bit (0x2000) set
        m.write_byte(0x51, 0xFF); // low8 = 0xFF -> raw 0x3FFF -> -1
        let b2 = decode_branch_at(&m, 0x50);
        assert!(!b2.on_true);
        assert_eq!(b2.offset, -1);
        assert_eq!(b2.len, 2, "long-form branch is 2 bytes");
    }

    #[test]
    fn v6_ext_signatures_consume_correct_bytes() {
        use crate::header::tests_support::sample_story;
        // (opcode, stores, branches) — from the ZMSD-verified table (Task 4 Step 1)
        let cases: [(u8, bool, bool); 18] = [
            (0x05, false, false), (0x06, false, true),  (0x07, false, false),
            (0x08, false, false), (0x10, false, false), (0x11, false, false),
            (0x12, false, false), (0x13, true,  false), (0x14, false, false),
            (0x15, false, false), (0x16, false, false), (0x17, false, false),
            (0x18, false, true),  (0x19, false, false), (0x1A, false, false),
            (0x1B, false, true),  (0x1C, false, false), (0x1D, true,  false),
        ];
        for (op, stores, branches) in cases {
            let mut buf = sample_story(6);
            let at = 0x0040usize;
            buf[at] = 0xBE;      // EXT prefix
            buf[at + 1] = op;    // ext opcode
            buf[at + 2] = 0xFF;  // VAR types byte: all omitted (0 operands)
            let mut n = at + 3;
            if stores   { buf[n] = 0x05; n += 1; }         // store into var 0x05
            if branches { buf[n] = 0xC1; n += 1; }         // 1-byte branch, on-true, offset 1
            let m = Memory::new(buf).unwrap();
            let instr = decode(&m, at as u32, 6);
            assert_eq!(instr.store.is_some(), stores,   "op {op:#04x} store");
            assert_eq!(instr.branch.is_some(), branches, "op {op:#04x} branch");
            assert_eq!(instr.next_pc, n as u32,          "op {op:#04x} next_pc");
        }
    }

    // v6 `pull stack -> (result)` ALWAYS carries a store byte, whatever the
    // operand encoding — frotz z_pull calls store() unconditionally in its V6
    // branch and picks user vs game stack by argc, not operand type. Zork Zero's
    // verb parse path encodes the stack address as a Var operand; keying the
    // store byte off Operand::Large alone corrupted decode there (SQ-0452).
    #[test]
    fn v6_pull_var_operand_has_store_byte() {
        let mut m = Memory::new(sample_story(6)).unwrap();
        m.write_byte(0x40, 0xE9);        // VAR:0x09 pull
        m.write_byte(0x41, 0b10_11_11_11); // type byte: one Var operand
        m.write_byte(0x42, 0x10);        // operand: global G0 (holds the stack addr)
        m.write_byte(0x43, 0x07);        // store byte -> var 7
        let ins = decode(&m, 0x40, 6);
        assert_eq!(ins.opcode, 0x09);
        assert_eq!(ins.operands, vec![Operand::Var(0x10)]);
        assert_eq!(ins.store, Some(0x07), "v6 pull always stores");
        assert_eq!(ins.next_pc, 0x44);
    }

    #[test]
    fn v6_pull_no_operand_has_store_byte() {
        // Omitted operand = game-stack pop; store byte still present in v6.
        let mut m = Memory::new(sample_story(6)).unwrap();
        m.write_byte(0x40, 0xE9);
        m.write_byte(0x41, 0xFF); // all operands omitted
        m.write_byte(0x42, 0x07); // store byte
        let ins = decode(&m, 0x40, 6);
        assert!(ins.operands.is_empty());
        assert_eq!(ins.store, Some(0x07));
        assert_eq!(ins.next_pc, 0x43);
    }

    #[test]
    fn v5_pull_var_operand_has_no_store_byte() {
        // v1-5 form is `pull (variable)` — the operand IS the destination.
        let mut m = Memory::new(sample_story(5)).unwrap();
        m.write_byte(0x40, 0xE9);
        m.write_byte(0x41, 0b10_11_11_11);
        m.write_byte(0x42, 0x10);
        let ins = decode(&m, 0x40, 5);
        assert_eq!(ins.store, None, "v1-5 pull must not eat a store byte");
        assert_eq!(ins.next_pc, 0x43);
    }
}
