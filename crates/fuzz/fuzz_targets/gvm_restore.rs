//! Coverage-guided fuzzing of the Glulx restore paths (SQ-1407):
//! `restore_quetzal` (in-game `@restore`/`@save`) and `load_vfs` (the virtual
//! filesystem a Glulx story's save/transcript files live in), both fed the
//! fuzzer's raw bytes.

#![no_main]

use gvm::exec::Machine;
use gvm::glk::TestBackend;
use gvm::memory::Memory;
use libfuzzer_sys::fuzz_target;

/// Mirrors `gvm::asm::assemble`'s header layout for a single trivial function
/// — duplicated rather than imported because `asm` is `#[cfg(test)]`-only
/// inside the gvm crate and not reachable from this separate package (see
/// docs/internals/fuzzing.md).
fn tiny_valid_image() -> Vec<u8> {
    let ramstart = 256u32;
    let extstart = 256u32;
    let endmem = 512u32;
    let stack_size = 256u32;
    let start_func = 0u32;
    let decode_table = 0u32;
    let mut img = vec![0u8; extstart as usize];
    img[0..4].copy_from_slice(b"Glul");
    img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes()); // version 3.1.2
    img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
    img[0x0C..0x10].copy_from_slice(&extstart.to_be_bytes());
    img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
    img[0x14..0x18].copy_from_slice(&stack_size.to_be_bytes());
    img[0x18..0x1C].copy_from_slice(&start_func.to_be_bytes());
    img[0x1C..0x20].copy_from_slice(&decode_table.to_be_bytes());
    img
}

fuzz_target!(|data: &[u8]| {
    let mem = Memory::new(tiny_valid_image()).expect("tiny_valid_image must parse");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    let _ = m.restore_quetzal(data);
    m.load_vfs(data);
    let _ = m.step(); // must still be steppable whatever happened above
});
