//! How the templates in `gvm::veneer` were made, kept so they can be remade.
//!
//! Point this at a Glulx story that registers its own accelerated functions —
//! any Inform 7 6E59-or-later build — and it prints the seven veneer routines as
//! Rust byte arrays, in accelerated-function-number order, plus the nine
//! parameters the story declares. The committed template came from
//! `stories/BlueLacuna.gblorb` (Inform 6.31, serial 100717); everything else the
//! module needs (the mask, the parameter slots) is derived from those bytes at
//! run time, so this is the whole recipe.
//!
//! ```sh
//! cargo run -p gvm --release --example veneer_gen -- stories/BlueLacuna.gblorb
//! ```
//!
//! A routine's extent is its function header up to the next discovered function,
//! which is how the veneer is laid out in every build in the corpus; `veneer.rs`
//! asserts at decode time that the bytes end exactly on an instruction boundary,
//! so a wrong extent fails loudly rather than silently masking a byte.

use std::collections::HashMap;

use gvm::disasm::DisasmCache;
use gvm::{Machine, Memory, StepResult, TestBackend};

/// Unwrap a Blorb, or pass a bare `.ulx` through.
fn extract_glulx(bytes: Vec<u8>) -> Vec<u8> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return bytes;
    }
    let b = blorb::Blorb::parse(bytes).expect("valid Blorb");
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => data.to_vec(),
        _ => panic!("expected a Glulx Blorb"),
    }
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: veneer_gen <story.gblorb|story.ulx>");
        std::process::exit(2);
    };
    let image = extract_glulx(std::fs::read(&path).expect("readable story"));
    let mem = Memory::new(image).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    let mut steps = 0u64;
    while !m.declares_own_accel() || m.accel_funcs().len() < 7 {
        steps += 1;
        assert!(steps < 20_000_000, "{path} never registered seven accelerated functions");
        match m.step() {
            StepResult::Continue => {}
            StepResult::NeedEvent { timer_ms: Some(_), .. } => m.deliver_timer(),
            StepResult::SaveRequest | StepResult::RestoreRequest => {
                let req = m.pending_saveload_request().unwrap_or_default();
                if req.restore {
                    m.complete_restore_failure();
                } else {
                    m.complete_save(false);
                }
            }
            other => panic!("{path} stopped at {other:?} before registering"),
        }
    }
    eprintln!("registered after {steps} opcodes");
    eprintln!("params: {:?}", (0..9).map(|i| m.accel_param(i)).collect::<Vec<_>>());

    let mut funcs: Vec<(u32, u32)> = m.accel_funcs().iter().map(|(a, n)| (*n, *a)).collect();
    funcs.sort();
    let empty: HashMap<u32, u32> = HashMap::new();
    let starts: Vec<u32> =
        DisasmCache::build(m.mem()).functions(&empty).iter().map(|f| f.addr).collect();
    for (num, addr) in &funcs {
        let end = starts.iter().copied().find(|s| *s > *addr).unwrap_or(m.mem().ramstart());
        let len = (end - addr) as usize;
        println!("// accel {num} ({}) @ {addr:#x}, {len} bytes", gvm::accel::accel_name(*num));
        print!("&[");
        for off in 0..len {
            if off % 16 == 0 {
                print!("\n    ");
            }
            print!("0x{:02x}, ", m.mem().read8(addr + off as u32).unwrap_or(0));
        }
        println!("\n];");
    }
}
