// Grammar extraction from a ZILF/ZAPF build — SQ-1718.
//
// `unit_tests/zork1-mit.z3` is a ZILF 0.11.1 build of Zork I from Microsoft's
// MIT-licensed source (see `unit_tests/README.md`), committed so CI always has
// it. ZAPF lays the tables out differently from Infocom's own assembler and
// stamps `"ZAPF"` at $3C, and a ZILF serial is the build date; three separate
// assumptions in `zvm::grammar` broke on that and `Grammar::load` answered
// `BadVerbTable`, which switched the automap off.

use std::path::PathBuf;

use zvm::grammar::{detect_format, Grammar, GrammarFormat};
use zvm::memory::Memory;

fn zilf_zork() -> Memory {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests/zork1-mit.z3");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Memory::new(bytes).expect("a valid story")
}

#[test]
fn a_zilf_serial_and_zapf_stamp_are_not_inform() {
    let mem = zilf_zork();
    // The specimen's serial is a date and $3C..$3F reads "ZAPF"; neither makes
    // it Inform.
    assert_eq!(mem.read_byte(0x3C), b'Z');
    assert!(!detect_format(&mem).is_inform());
    assert_eq!(detect_format(&mem), GrammarFormat::InfocomFixed);
}

#[test]
fn a_zapf_build_loads_as_an_infocom_grammar() {
    let g = Grammar::load(&zilf_zork()).expect("ZAPF layout is located structurally");
    assert!(g.format().is_infocom(), "got {:?}", g.format());
    // The verb pointer table runs $4416..$4526 in the specimen: 136 verbs.
    assert_eq!(g.verbs().len(), 136);
    // 149 actions, as the dictionary-independent walk of the syntax data finds.
    assert_eq!(g.action_routines().len(), 149);

    let open = g.verb_for_word("open").expect("knows open");
    assert!(open.lines.iter().any(|l| l.noun_count() == 1));
    assert!(open.accepts(2, &["with"]));
    let take = g.verb_for_word("take").expect("knows take");
    assert!(take.accepts(1, &[]));
    assert!(take.accepts(2, &["from"]));
    assert!(g.is_preposition("with") && g.is_preposition("from"));
    assert!(g.is_verb("north") || g.is_verb("look"));
}
