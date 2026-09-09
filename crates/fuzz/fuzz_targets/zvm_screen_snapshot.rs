//! Coverage-guided fuzzing of the Version 6 screen-snapshot decoder (SQ-1407)
//! — a pure decode with no machine required, so the fuzzer's bytes go
//! straight in.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = zvm::screen_snapshot::decode(data);
});
