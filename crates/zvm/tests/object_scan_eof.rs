//! Regression: the object scan must not read past the end of the story file.
//!
//! czech.z5 (the Z-machine torture test) has an object table whose layout makes
//! the count-inference heuristic walk one entry past the real table into data
//! that looks like a valid property-table pointer aimed beyond the file. Before
//! the fix, `object_tree_view` then read the bogus object's name at an
//! out-of-bounds address and panicked (memory.rs read_byte index-out-of-bounds).
use zvm::cpu::exec::Machine;
use zvm::memory::Memory;

#[test]
fn object_tree_view_does_not_read_past_eof_on_czech() {
    let story =
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/czech.z5")).unwrap();
    let len = story.len();
    let mem = Memory::new(story).unwrap();
    let machine = Machine::new(mem);

    // Must not panic, and every reported object number is positive and within
    // the inferred count (the snapshot reads stayed inside the file).
    let view = zvm::location::object_tree_view(&machine);
    assert!(!view.is_empty(), "czech.z5 has objects");
    assert!(view.len() < len, "object count must be far below the file length");
    for (i, snap) in view.iter().enumerate() {
        assert_eq!(snap.number as usize, i + 1, "objects are numbered 1..=n in order");
    }
}
