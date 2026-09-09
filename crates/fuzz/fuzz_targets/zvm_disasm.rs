//! Coverage-guided fuzzing of the Z-machine disassembler (SQ-1407): the
//! fuzzer's bytes are the story image itself, disassembled from an address
//! it also derives from the image.

#![no_main]

use libfuzzer_sys::fuzz_target;
use zvm::cpu::disasm::{disassemble, disassemble_raw};
use zvm::memory::Memory;

fuzz_target!(|data: &[u8]| {
    if data.len() < 66 {
        return;
    }
    let Ok(mem) = Memory::new(data.to_vec()) else {
        return;
    };
    let version = mem.version();
    let addr = (u32::from(data[64]) | (u32::from(data[65]) << 8)) % mem.len() as u32;
    let _ = disassemble(&mem, addr, version, 20);
    let _ = disassemble_raw(&mem, addr, version, 20);
});
