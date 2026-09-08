//! Quetzal 1.4 save/restore for the Z-machine — no filesystem I/O.
//!
//! Quetzal is an IFF FORM of type "IFZS" containing three required chunks:
//!
//! - `IFhd` — story identity (release, serial, checksum) + save-time PC
//! - `CMem` — dynamic memory XOR'd against the original story, RLE-compressed
//! - `Stks` — call stack frames
//!
//! The host owns file I/O; this module returns/accepts `Vec<u8>`/`&[u8]`.
//!
//! IFF structure: `FORM<4> + total-length<4> + "IFZS"<4> + chunks`.
//! Each chunk: `type<4> + length<4> + data` + optional pad byte (to make total even).
//!
//! Save PC semantics: the `IFhd` PC comes from `Machine::save_pc()`. For an in-game
//! `@save`/`@restore` opcode (`pending_save` set) it is the result-descriptor address —
//! the store byte (v4+) or first branch byte (v3) — per Quetzal §5.8; restore
//! reads that descriptor forward (see `complete_restore_success`). For a host
//! "Save State" snapshot (no pending save) it is `state.pc`, and restore simply
//! resumes there (see `restore_file`).

use crate::cpu::exec::Machine;
use crate::cpu::state::{Frame, State};
use crate::error::ZError;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a Quetzal IFF save file from the current machine state.
///
/// `original_dynamic` must be the original (pre-mutation) dynamic memory bytes
/// — exactly the first `static_mem_base` bytes of the story image as loaded.
/// Machine stores this as `Machine::original_dynamic`.
pub fn save_quetzal(machine: &Machine) -> Vec<u8> {
    let orig = &machine.original_dynamic;
    let ifhd = encode_ifhd(machine);
    let cmem = encode_cmem(&machine.mem, orig);
    let stks = encode_stks(&machine.state, machine.mem.version() == 6);

    let mut body = Vec::new();
    body.extend_from_slice(b"IFZS");
    write_chunk(&mut body, b"IFhd", &ifhd);
    write_chunk(&mut body, b"CMem", &cmem);
    write_chunk(&mut body, b"Stks", &stks);

    let mut out = Vec::new();
    out.extend_from_slice(b"FORM");
    write_u32_be(&mut out, body.len() as u32);
    out.extend_from_slice(&body);
    out
}

/// Validate and restore machine state from a Quetzal save buffer.
///
/// Checks that IFhd release/serial/checksum match the loaded story.
/// On success, dynamic memory, call frames, eval stack, and PC are replaced.
/// On error the machine state is untouched.
pub fn restore_quetzal(machine: &mut Machine, data: &[u8]) -> Result<(), ZError> {
    let chunks = parse_iff(data)?;

    // IFhd is mandatory
    let ifhd_data = find_chunk(&chunks, b"IFhd").ok_or(ZError::Truncated)?;
    validate_ifhd(machine, ifhd_data)?;
    let saved_pc = decode_ifhd_pc(ifhd_data);

    // Dynamic memory: prefer the compressed CMem chunk (what we emit); also accept
    // the uncompressed UMem chunk (Quetzal §3) so saves from other interpreters
    // restore correctly. At least one must be present.
    let restored_dyn = if let Some(cmem_data) = find_chunk(&chunks, b"CMem") {
        decode_cmem(cmem_data, &machine.original_dynamic)?
    } else if let Some(umem_data) = find_chunk(&chunks, b"UMem") {
        decode_umem(umem_data, &machine.original_dynamic)?
    } else {
        return Err(ZError::Truncated);
    };

    // Stks is mandatory
    let stks_data = find_chunk(&chunks, b"Stks").ok_or(ZError::Truncated)?;
    let (frames, eval_stack) = decode_stks(stks_data, machine.mem.version() == 6)?;

    // All parsing succeeded — apply to machine
    let dyn_len = machine.original_dynamic.len();
    for (i, &b) in restored_dyn.iter().enumerate().take(dyn_len) {
        machine.mem.write_byte(i as u32, b);
    }
    machine.state.pc = saved_pc;
    machine.state.frames = frames;
    machine.state.eval_stack = eval_stack;

    Ok(())
}

// ---------------------------------------------------------------------------
// IFhd chunk — 13 bytes of identity + PC
// ---------------------------------------------------------------------------
// Layout:
//   0..2  release number  (header 0x02–0x03)
//   2..8  serial number   (header 0x12–0x17, 6 ASCII bytes)
//   8..10 checksum        (header 0x1C–0x1D)
//  10..13 PC              (3 bytes, big-endian)

fn encode_ifhd(machine: &Machine) -> Vec<u8> {
    let mem = &machine.mem;
    let mut out = Vec::with_capacity(13);
    // Release (2 bytes at header 0x02)
    out.push(mem.read_byte(0x02));
    out.push(mem.read_byte(0x03));
    // Serial (6 bytes at header 0x12)
    for i in 0x12u32..0x18 {
        out.push(mem.read_byte(i));
    }
    // Checksum (2 bytes at header 0x1C)
    out.push(mem.read_byte(0x1C));
    out.push(mem.read_byte(0x1D));
    // PC at save time (3 bytes, big-endian). For an in-game @save this is the
    // result-descriptor address (Quetzal §5.8); for a host Save State it is
    // state.pc. See Machine::save_pc.
    let pc = machine.save_pc();
    out.push(((pc >> 16) & 0xFF) as u8);
    out.push(((pc >>  8) & 0xFF) as u8);
    out.push(( pc        & 0xFF) as u8);
    out
}

fn validate_ifhd(machine: &Machine, ifhd: &[u8]) -> Result<(), ZError> {
    if ifhd.len() < 13 {
        return Err(ZError::Truncated);
    }
    let mem = &machine.mem;
    // Release
    if ifhd[0] != mem.read_byte(0x02) || ifhd[1] != mem.read_byte(0x03) {
        return Err(ZError::SaveMismatch);
    }
    // Serial
    for (i, j) in (0x12u32..0x18).enumerate() {
        if ifhd[2 + i] != mem.read_byte(j) {
            return Err(ZError::SaveMismatch);
        }
    }
    // Checksum
    if ifhd[8] != mem.read_byte(0x1C) || ifhd[9] != mem.read_byte(0x1D) {
        return Err(ZError::SaveMismatch);
    }
    Ok(())
}

fn decode_ifhd_pc(ifhd: &[u8]) -> u32 {
    ((ifhd[10] as u32) << 16) | ((ifhd[11] as u32) << 8) | (ifhd[12] as u32)
}

/// Test helper: extract the IFhd program counter from a Quetzal blob.
#[cfg(test)]
pub(crate) fn saved_pc_of(data: &[u8]) -> u32 {
    let chunks = parse_iff(data).expect("valid IFF");
    let ifhd = find_chunk(&chunks, b"IFhd").expect("IFhd present");
    decode_ifhd_pc(ifhd)
}

// ---------------------------------------------------------------------------
// CMem chunk — compressed dynamic memory (XOR + zero-RLE)
// ---------------------------------------------------------------------------
// Quetzal §3: XOR current dynamic memory with original, then RLE-encode:
//   non-zero bytes → emitted literally
//   runs of zero bytes → 0x00 <n-1>  (one 0x00 encodes 1 zero byte, etc.)

fn encode_cmem(mem: &crate::memory::Memory, orig: &[u8]) -> Vec<u8> {
    let dyn_len = orig.len();
    let mut xored: Vec<u8> = (0..dyn_len as u32)
        .map(|i| mem.read_byte(i) ^ orig[i as usize])
        .collect();

    // Strip trailing zeros (Quetzal spec says truncate after last non-zero)
    while xored.last() == Some(&0) {
        xored.pop();
    }

    // RLE-encode zero runs
    let mut out = Vec::new();
    let mut i = 0;
    while i < xored.len() {
        if xored[i] != 0 {
            out.push(xored[i]);
            i += 1;
        } else {
            // Count consecutive zeros
            let start = i;
            while i < xored.len() && xored[i] == 0 && (i - start) < 256 {
                i += 1;
            }
            let run = i - start; // 1..=256
            out.push(0x00);
            out.push((run - 1) as u8);
        }
    }
    out
}

fn decode_cmem(cmem: &[u8], orig: &[u8]) -> Result<Vec<u8>, ZError> {
    // Decode RLE back to XOR'd bytes
    let mut xored = Vec::with_capacity(orig.len());
    let mut i = 0;
    while i < cmem.len() {
        if cmem[i] != 0x00 {
            xored.push(cmem[i]);
            i += 1;
        } else {
            i += 1;
            if i >= cmem.len() {
                return Err(ZError::Truncated);
            }
            let count = cmem[i] as usize + 1;
            i += 1;
            xored.resize(xored.len() + count, 0x00);
        }
    }

    // XOR against original to recover current dynamic memory
    // Start with a full copy of original (handles case where trailing zeros were stripped)
    let mut dyn_mem = orig.to_vec();
    for (j, &x) in xored.iter().enumerate() {
        if j >= dyn_mem.len() {
            return Err(ZError::Truncated);
        }
        dyn_mem[j] ^= x;
    }
    Ok(dyn_mem)
}

// ---------------------------------------------------------------------------
// UMem chunk — uncompressed dynamic memory (Quetzal §3)
// ---------------------------------------------------------------------------
// UMem holds the contents of dynamic memory verbatim (no XOR, no RLE). We never
// emit it (CMem is more compact), but accept it on restore for cross-interpreter
// compatibility. The chunk must cover the whole dynamic region.

fn decode_umem(umem: &[u8], orig: &[u8]) -> Result<Vec<u8>, ZError> {
    if umem.len() < orig.len() {
        return Err(ZError::Truncated);
    }
    Ok(umem[..orig.len()].to_vec())
}

// ---------------------------------------------------------------------------
// Stks chunk — call stack frames
// ---------------------------------------------------------------------------
// Quetzal §4.3, per-frame layout:
//   3 bytes  return PC (big-endian)
//   1 byte   flags: bits 3..0 = local count; bit 4 = discard result (store_var is None)
//   1 byte   result variable number (0 if discarding)
//   1 byte   argument bitmap: (1 << arg_count) - 1
//   2 bytes  number of eval-stack words pushed in this frame (big-endian)
//   then     locals (one word each, big-endian)
//   then     frame's eval-stack words (big-endian)
//
// Frames are written oldest-first (bottom of call stack first).
// The outermost "dummy" frame for the main routine is the first frame in the
// list, but in our State there is no explicit dummy frame — the main program
// runs with frames.len() == 0 and its eval words in eval_stack[0..].
// We emit one dummy frame (return_pc=0, no locals, no args, no stack words)
// representing the main routine's initial state before frames[0].
//
// v6 is the exception: Quetzal §4.11 requires the dummy frame "in all versions
// other than V6" — a v6 game boots via a genuine call to its `main` routine
// (ZMSD §5.4), so its outermost frame is a real one and no dummy is written
// (nor expected on decode). Interoperable interpreters (Frotz) follow this.

fn encode_stks(state: &State, v6: bool) -> Vec<u8> {
    let mut out = Vec::new();

    // Dummy "main routine" frame — Quetzal §4.11.1: "The dummy frame has all
    // fields set to zero except n, the amount of evaluation stack used." So the
    // flags byte is 0x00 (NOT 0x10 — the discard bit must be clear here even
    // though main stores no result; dfrotz rejects the save otherwise).
    // Its eval stack words = eval_stack[0..eval_base_of_first_frame]
    // Not written for v6 (Quetzal §4.11: dummy frame is for "all versions
    // other than V6" — v6's base frame is the real call to `main`).
    if !v6 {
        let first_frame_base = state.frames.first().map(|f| f.eval_base).unwrap_or(state.eval_stack.len());
        let main_eval: &[u16] = &state.eval_stack[..first_frame_base];

        write_frame_raw(
            &mut out,
            0,         // return_pc
            0x00,      // flags: dummy frame is all-zero except n (Quetzal §4.11.1)
            0,         // result var
            0,         // arg bitmap
            main_eval,
            &[],       // no locals
        );
    }

    // Real call frames, oldest first
    for (idx, frame) in state.frames.iter().enumerate() {
        let eval_end = if idx + 1 < state.frames.len() {
            state.frames[idx + 1].eval_base
        } else {
            state.eval_stack.len()
        };
        let frame_eval = &state.eval_stack[frame.eval_base..eval_end];

        let discard = frame.store_var.is_none();
        let local_count = frame.locals.len().min(15) as u8;
        let flags: u8 = local_count | if discard { 0x10 } else { 0x00 };
        let result_var = frame.store_var.unwrap_or(0);
        let arg_bitmap: u8 = if frame.arg_count >= 8 {
            0xFF
        } else {
            (1u8 << frame.arg_count).wrapping_sub(1)
        };

        write_frame_raw(
            &mut out,
            frame.return_pc,
            flags,
            result_var,
            arg_bitmap,
            frame_eval,
            &frame.locals,
        );
    }

    out
}

fn write_frame_raw(
    out: &mut Vec<u8>,
    return_pc: u32,
    flags: u8,
    result_var: u8,
    arg_bitmap: u8,
    eval_words: &[u16],
    locals: &[u16],
) {
    out.push(((return_pc >> 16) & 0xFF) as u8);
    out.push(((return_pc >>  8) & 0xFF) as u8);
    out.push(( return_pc        & 0xFF) as u8);
    out.push(flags);
    out.push(result_var);
    out.push(arg_bitmap);
    let eval_count = eval_words.len() as u16;
    out.push((eval_count >> 8) as u8);
    out.push((eval_count & 0xFF) as u8);
    for &l in locals {
        out.push((l >> 8) as u8);
        out.push((l & 0xFF) as u8);
    }
    for &w in eval_words {
        out.push((w >> 8) as u8);
        out.push((w & 0xFF) as u8);
    }
}

fn decode_stks(stks: &[u8], v6: bool) -> Result<(Vec<Frame>, Vec<u16>), ZError> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut eval_stack: Vec<u16> = Vec::new();
    let mut pos = 0;
    // Non-v6: the first frame in the file is the §4.11 dummy frame (skipped —
    // its eval words go on the global stack but it is not a Frame). v6 saves
    // have NO dummy frame (§4.11), so the first frame is a real one.
    let mut first = !v6;

    while pos < stks.len() {
        if pos + 8 > stks.len() {
            return Err(ZError::Truncated);
        }
        let return_pc = ((stks[pos] as u32) << 16)
            | ((stks[pos + 1] as u32) << 8)
            | (stks[pos + 2] as u32);
        let flags = stks[pos + 3];
        let result_var = stks[pos + 4];
        let arg_bitmap = stks[pos + 5];
        let eval_count = ((stks[pos + 6] as usize) << 8) | stks[pos + 7] as usize;
        pos += 8;

        let local_count = (flags & 0x0F) as usize;
        let discard = (flags & 0x10) != 0;

        // Read locals
        if pos + local_count * 2 > stks.len() {
            return Err(ZError::Truncated);
        }
        let mut locals = Vec::with_capacity(local_count);
        for _ in 0..local_count {
            let w = ((stks[pos] as u16) << 8) | stks[pos + 1] as u16;
            locals.push(w);
            pos += 2;
        }

        // Read eval stack words for this frame
        if pos + eval_count * 2 > stks.len() {
            return Err(ZError::Truncated);
        }
        let eval_base = eval_stack.len();
        for _ in 0..eval_count {
            let w = ((stks[pos] as u16) << 8) | stks[pos + 1] as u16;
            eval_stack.push(w);
            pos += 2;
        }

        if first {
            // This is the dummy main-routine frame; its eval words are on the
            // global eval stack but it doesn't correspond to a Frame struct.
            first = false;
        } else {
            // Count args from bitmap
            let arg_count = arg_bitmap.count_ones() as u8;
            let store_var = if discard { None } else { Some(result_var) };
            frames.push(Frame {
                return_pc,
                locals,
                eval_base,
                store_var,
                arg_count,
                // Quetzal's Stks chunk does not record the routine's entry
                // address; restored frames report 0 (unknown).
                func_addr: 0,
            });
        }
    }

    Ok((frames, eval_stack))
}

// ---------------------------------------------------------------------------
// IFF helpers
// ---------------------------------------------------------------------------

fn write_u32_be(out: &mut Vec<u8>, v: u32) {
    out.push(((v >> 24) & 0xFF) as u8);
    out.push(((v >> 16) & 0xFF) as u8);
    out.push(((v >>  8) & 0xFF) as u8);
    out.push(( v        & 0xFF) as u8);
}

fn write_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(tag);
    write_u32_be(out, data.len() as u32);
    out.extend_from_slice(data);
    if !data.len().is_multiple_of(2) {
        out.push(0x00); // pad to even length
    }
}

type IffChunk<'a> = ([u8; 4], &'a [u8]);

/// Parse an IFF FORM/IFZS file into a list of (tag, data) pairs.
fn parse_iff(data: &[u8]) -> Result<Vec<IffChunk<'_>>, ZError> {
    if data.len() < 12 {
        return Err(ZError::Truncated);
    }
    if &data[0..4] != b"FORM" {
        return Err(ZError::Truncated);
    }
    let form_len = u32_be(&data[4..8]) as usize;
    if data.len() < 8 + form_len {
        return Err(ZError::Truncated);
    }
    if &data[8..12] != b"IFZS" {
        return Err(ZError::Truncated);
    }

    let mut chunks = Vec::new();
    let mut pos = 12;
    let end = 8 + form_len;

    while pos + 8 <= end {
        let tag: [u8; 4] = [data[pos], data[pos+1], data[pos+2], data[pos+3]];
        let chunk_len = u32_be(&data[pos+4..pos+8]) as usize;
        pos += 8;
        if pos + chunk_len > data.len() {
            return Err(ZError::Truncated);
        }
        chunks.push((tag, &data[pos..pos + chunk_len]));
        pos += chunk_len;
        if !chunk_len.is_multiple_of(2) {
            pos += 1; // skip pad byte
        }
    }

    Ok(chunks)
}

fn find_chunk<'a>(chunks: &[([u8; 4], &'a [u8])], tag: &[u8; 4]) -> Option<&'a [u8]> {
    chunks.iter().find(|(t, _)| t == tag).map(|(_, d)| *d)
}

fn u32_be(b: &[u8]) -> u32 {
    ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::exec::Machine;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;

    /// Build a Machine with the v5 sample story and inject a known serial/release/checksum.
    fn make_machine() -> Machine {
        let mut buf = sample_story(5);
        // Set release = 0x0102
        buf[0x02] = 0x01; buf[0x03] = 0x02;
        // Set serial = "ABCDEF"
        buf[0x12] = b'A'; buf[0x13] = b'B'; buf[0x14] = b'C';
        buf[0x15] = b'D'; buf[0x16] = b'E'; buf[0x17] = b'F';
        // Set checksum = 0x1234
        buf[0x1C] = 0x12; buf[0x1D] = 0x34;
        let mem = Memory::new(buf).unwrap();
        Machine::new(mem)
    }

    // -----------------------------------------------------------------------
    // Test (a): round-trip — save then restore recovers dynamic memory,
    // eval stack, frames, and PC exactly.
    // -----------------------------------------------------------------------
    #[test]
    fn round_trip_restores_full_state() {
        let mut m = make_machine();
        m.state.pc = 0x0050;

        // Mutate some dynamic memory (addr 0x100 is in dynamic region [0..0x400])
        m.mem.write_byte(0x100, 0xAB);
        m.mem.write_byte(0x101, 0xCD);

        // Push a call frame with locals + eval-stack words
        m.state.frames.push(crate::cpu::state::Frame {
            return_pc: 0x0030,
            locals: vec![0x1111, 0x2222, 0x3333],
            eval_base: 0,
            store_var: Some(0x10),
            arg_count: 2,
            func_addr: 0,
        });
        m.state.eval_stack.push(0xAAAA);
        m.state.eval_stack.push(0xBBBB);

        // Save
        let saved = save_quetzal(&m);

        // Further mutate state
        m.mem.write_byte(0x100, 0x00);
        m.mem.write_byte(0x101, 0x00);
        m.state.pc = 0x9999;
        m.state.frames.clear();
        m.state.eval_stack.clear();
        m.mem.write_byte(0x200, 0xFF);

        // Restore
        restore_quetzal(&mut m, &saved).expect("restore failed");

        // Verify state matches save point
        assert_eq!(m.state.pc, 0x0050, "PC not restored");
        assert_eq!(m.mem.read_byte(0x100), 0xAB, "dyn mem byte 0x100");
        assert_eq!(m.mem.read_byte(0x101), 0xCD, "dyn mem byte 0x101");
        assert_eq!(m.mem.read_byte(0x200), 0x00, "dyn mem byte 0x200 restored to original");
        assert_eq!(m.state.frames.len(), 1, "should have 1 frame");
        let f = &m.state.frames[0];
        assert_eq!(f.return_pc, 0x0030, "return_pc");
        assert_eq!(f.locals, vec![0x1111, 0x2222, 0x3333], "locals");
        assert_eq!(f.store_var, Some(0x10), "store_var");
        assert_eq!(f.arg_count, 2, "arg_count");
        assert_eq!(m.state.eval_stack, vec![0xAAAA, 0xBBBB], "eval stack");
    }

    // -----------------------------------------------------------------------
    // Test (b): CMem RLE — mostly-unchanged memory → compact CMem
    // -----------------------------------------------------------------------
    #[test]
    fn cmem_rle_compact_and_decompresses_correctly() {
        let mut m = make_machine();
        // Only change byte 0x100 — everything else is zero (XOR'd with itself)
        m.mem.write_byte(0x100, 0x42);

        let saved = save_quetzal(&m);

        let chunks = parse_iff(&saved).unwrap();
        let cmem_data = find_chunk(&chunks, b"CMem").unwrap();

        // The CMem should be very compact: mostly RLE-encoded zero runs
        // Dynamic region is 0x400 bytes; only 1 byte differs → CMem << 0x400 bytes
        assert!(cmem_data.len() < 50, "CMem should be compact, got {} bytes", cmem_data.len());

        // Decompress and verify it matches current state
        let orig = &m.original_dynamic;
        let restored = decode_cmem(cmem_data, orig).unwrap();
        assert_eq!(restored[0x100], 0x42, "decompressed byte 0x100");
        // All other bytes should match original (0)
        assert_eq!(restored[0x101], 0x00, "decompressed byte 0x101");
    }

    // -----------------------------------------------------------------------
    // Test (c): IFhd mismatch returns Err
    // -----------------------------------------------------------------------
    #[test]
    fn ifhd_mismatch_returns_err() {
        let m = make_machine();
        let saved = save_quetzal(&m);

        // Build a second machine with different serial
        let mut buf2 = sample_story(5);
        buf2[0x02] = 0x01; buf2[0x03] = 0x02;
        buf2[0x12] = b'X'; // different serial
        buf2[0x13] = b'B'; buf2[0x14] = b'C';
        buf2[0x15] = b'D'; buf2[0x16] = b'E'; buf2[0x17] = b'F';
        buf2[0x1C] = 0x12; buf2[0x1D] = 0x34;
        let mem2 = Memory::new(buf2).unwrap();
        let mut m2 = Machine::new(mem2);

        let result = restore_quetzal(&mut m2, &saved);
        assert_eq!(result, Err(ZError::SaveMismatch), "should reject mismatched serial with SaveMismatch");
    }

    // -----------------------------------------------------------------------
    // Test (d): Stks frame round-trip with locals + pushed eval words
    // -----------------------------------------------------------------------
    #[test]
    fn stks_frame_locals_and_eval_roundtrip() {
        let mut m = make_machine();
        m.state.pc = 0x0080;

        // Two frames nested: outer and inner
        m.state.frames.push(crate::cpu::state::Frame {
            return_pc: 0x0010,
            locals: vec![0xAAAA, 0xBBBB],
            eval_base: 0,
            store_var: None,       // discarding result
            arg_count: 1,
            func_addr: 0,
        });
        m.state.eval_stack.push(0x1234); // outer frame's eval word

        m.state.frames.push(crate::cpu::state::Frame {
            return_pc: 0x0020,
            locals: vec![0xCCCC],
            eval_base: 1,
            store_var: Some(0x12),
            arg_count: 3,
            func_addr: 0,
        });
        m.state.eval_stack.push(0x5678); // inner frame's eval word
        m.state.eval_stack.push(0x9ABC);

        let saved = save_quetzal(&m);
        // Wipe state
        m.state.frames.clear();
        m.state.eval_stack.clear();
        m.state.pc = 0;

        restore_quetzal(&mut m, &saved).expect("restore failed");

        assert_eq!(m.state.pc, 0x0080);
        assert_eq!(m.state.frames.len(), 2);

        let f0 = &m.state.frames[0];
        assert_eq!(f0.return_pc, 0x0010);
        assert_eq!(f0.locals, vec![0xAAAA, 0xBBBB]);
        assert_eq!(f0.store_var, None);
        assert_eq!(f0.arg_count, 1);

        let f1 = &m.state.frames[1];
        assert_eq!(f1.return_pc, 0x0020);
        assert_eq!(f1.locals, vec![0xCCCC]);
        assert_eq!(f1.store_var, Some(0x12));
        assert_eq!(f1.arg_count, 3);

        // Eval stack: outer word then inner words
        assert_eq!(m.state.eval_stack, vec![0x1234, 0x5678, 0x9ABC]);
    }

    // -----------------------------------------------------------------------
    // Test: a hand-built save using the uncompressed UMem chunk restores.
    // -----------------------------------------------------------------------
    #[test]
    fn restore_accepts_umem_chunk() {
        let mut m = make_machine();
        m.state.pc = 0x0060;

        // IFhd + Stks reuse the normal encoders (PC captured from state.pc).
        let ifhd = encode_ifhd(&m);
        let stks = encode_stks(&m.state, m.mem.version() == 6);

        // UMem = the full dynamic region verbatim, with one byte changed.
        let mut umem = m.original_dynamic.clone();
        umem[0x100] = 0x7E;

        let mut body = Vec::new();
        body.extend_from_slice(b"IFZS");
        write_chunk(&mut body, b"IFhd", &ifhd);
        write_chunk(&mut body, b"UMem", &umem);
        write_chunk(&mut body, b"Stks", &stks);
        let mut save = Vec::new();
        save.extend_from_slice(b"FORM");
        write_u32_be(&mut save, body.len() as u32);
        save.extend_from_slice(&body);

        // Move state away from the save point, then restore from the UMem save.
        m.state.pc = 0x9999;
        m.mem.write_byte(0x100, 0x00);

        restore_quetzal(&mut m, &save).expect("UMem restore should succeed");
        assert_eq!(m.state.pc, 0x0060, "PC restored from IFhd");
        assert_eq!(m.mem.read_byte(0x100), 0x7E, "dynamic memory restored from UMem");
    }

    // -----------------------------------------------------------------------
    // Test: v6 Stks has no dummy frame (Quetzal §4.11) and round-trips
    // -----------------------------------------------------------------------
    #[test]
    fn v6_stks_omits_dummy_frame_and_roundtrips() {
        // Quetzal §4.11: "in all versions other than V6 a dummy stack frame
        // must be stored as the first in the file" — a v6 game boots via a
        // genuine call to `main` (ZMSD §5.4), so its Stks begins with that
        // real frame and NO dummy is written or expected. (SQ-0622)
        let mut buf = sample_story(6);
        buf[0x02] = 0x01; buf[0x03] = 0x02; // release
        buf[0x12] = b'A'; buf[0x13] = b'B'; buf[0x14] = b'C';
        buf[0x15] = b'D'; buf[0x16] = b'E'; buf[0x17] = b'F';
        buf[0x1C] = 0x12; buf[0x1D] = 0x34; // checksum
        let mut m = Machine::new(Memory::new(buf).unwrap());

        // v6 boot pushed the real `main` frame; give it some state to carry.
        assert_eq!(m.state.frames.len(), 1, "v6 boots with the main call frame");
        m.state.frames[0].locals = vec![0x1111, 0x2222];
        m.state.eval_stack.push(0xBEEF);
        m.state.pc = 0x0123;

        let saved = save_quetzal(&m);

        // The raw Stks chunk holds exactly ONE frame: 8 header bytes +
        // 2 locals (2 bytes each) + 1 eval word (2 bytes). A leading §4.11
        // dummy would add 8 more.
        let chunks = parse_iff(&saved).unwrap();
        let stks = find_chunk(&chunks, b"Stks").unwrap();
        assert_eq!(stks.len(), 8 + 2 * 2 + 2, "no dummy frame in v6 Stks");

        // Round-trip: perturb, restore, verify the real frame survives.
        m.state.frames.clear();
        m.state.eval_stack.clear();
        m.state.pc = 0;
        restore_quetzal(&mut m, &saved).expect("v6 restore");
        assert_eq!(m.state.pc, 0x0123);
        assert_eq!(m.state.frames.len(), 1, "the real main frame is a Frame, not a discarded dummy");
        assert_eq!(m.state.frames[0].locals, vec![0x1111, 0x2222]);
        assert_eq!(m.state.eval_stack, vec![0xBEEF]);
    }

    // -----------------------------------------------------------------------
    // Test: no-frame (main routine only) round-trips
    // -----------------------------------------------------------------------
    #[test]
    fn round_trip_no_frames() {
        let mut m = make_machine();
        m.state.pc = 0x0042;
        m.mem.write_byte(0x50, 0xDE);
        m.state.eval_stack.push(0x1111);

        let saved = save_quetzal(&m);
        m.state.pc = 0;
        m.state.eval_stack.clear();
        m.mem.write_byte(0x50, 0);

        restore_quetzal(&mut m, &saved).unwrap();
        assert_eq!(m.state.pc, 0x0042);
        assert_eq!(m.mem.read_byte(0x50), 0xDE);
        assert_eq!(m.state.eval_stack, vec![0x1111]);
        assert_eq!(m.state.frames.len(), 0);
    }
}
