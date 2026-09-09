//! Coverage-guided fuzzing of the Glulx disassembler (SQ-1407): the fuzzer's
//! bytes are the story image itself, decoded from an address it also derives
//! from the image.

#![no_main]

use gvm::disasm::decode_instr;
use gvm::memory::Memory;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 40 {
        return;
    }
    let Ok(mem) = Memory::new(data.to_vec()) else {
        return;
    };
    let addr = u32::from_be_bytes([data[36], data[37], data[38], data[39]]) % mem.endmem().max(1);
    let _ = decode_instr(&mem, addr);
});
