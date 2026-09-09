//! Coverage-guided fuzzing of the Z-machine's host-facing restore paths
//! (SQ-1407): `restore_quetzal` (in-game `@restore`) and `restore_file` (host
//! Save State restore), both fed the fuzzer's raw bytes as the save blob.

#![no_main]

use libfuzzer_sys::fuzz_target;
use zvm::cpu::exec::Machine;
use zvm::memory::Memory;

/// Mirrors `zvm::header::tests_support::sample_story(3)` — duplicated rather
/// than imported because that helper is `#[cfg(test)]`-only inside the zvm
/// crate and not reachable from this separate package (see
/// docs/internals/fuzzing.md).
fn tiny_valid_story() -> Vec<u8> {
    let mut buf = vec![0u8; 0x400];
    buf[0x00] = 3; // version
    buf[0x04] = 0x04;
    buf[0x05] = 0x00; // high_mem_base = 0x0400
    buf[0x06] = 0x00;
    buf[0x07] = 0x40; // initial_pc = 0x0040
    buf[0x08] = 0x02;
    buf[0x09] = 0x00; // dictionary = 0x0200
    buf[0x0A] = 0x01;
    buf[0x0B] = 0x00; // object_table = 0x0100
    buf[0x0C] = 0x03;
    buf[0x0D] = 0x00; // global_vars = 0x0300
    buf[0x0E] = 0x04;
    buf[0x0F] = 0x00; // static_mem_base = 0x0400
    buf[0x18] = 0x00;
    buf[0x19] = 0x40; // abbrev_table = 0x0040
    buf
}

fuzz_target!(|data: &[u8]| {
    let mem = Memory::new(tiny_valid_story()).expect("tiny_valid_story must parse");
    let mut m = Machine::new(mem);
    let _ = m.restore_quetzal(data);
    let _ = m.step(); // must still be steppable whatever happened above

    let mem2 = Memory::new(tiny_valid_story()).expect("tiny_valid_story must parse");
    let mut m2 = Machine::new(mem2);
    let _ = m2.restore_file(data);
    let _ = m2.step();
});
