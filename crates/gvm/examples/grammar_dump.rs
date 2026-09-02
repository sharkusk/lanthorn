//! Locate and dump a Glulx story's Inform grammar tables.
//!
//! A development tool, not a feature. Its first job is to print the addresses
//! `glulxdump` needs, since that tool — the only reference implementation for
//! these tables — cannot find them itself and must be told:
//!
//!     cargo run -p gvm --example grammar_dump -- --tables story.gblorb
//!     glulxdump -g <grammar-addr> story.ulx
//!
//! `--sentences` prints one sentence per line, for diffing.
//!
//!     cargo run -p gvm --example grammar_dump -- stories/Eat_Me.gblorb

use gvm::grammar::{Grammar, RoutineRef};
use gvm::memory::Memory;

/// Pull the `GLUL` chunk out of a Blorb, or pass a bare Glulx image through.
/// Hand-rolled rather than reaching for the `blorb` crate: `gvm` takes no
/// dependencies, and this is fifteen lines.
fn glulx_image(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.starts_with(b"Glul") {
        return Some(bytes);
    }
    if !(bytes.starts_with(b"FORM") && bytes.get(8..12) == Some(b"IFRS")) {
        return None;
    }
    let be32 = |a: usize| -> usize {
        u32::from_be_bytes([bytes[a], bytes[a + 1], bytes[a + 2], bytes[a + 3]]) as usize
    };
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let len = be32(i + 4);
        if &bytes[i..i + 4] == b"GLUL" {
            return bytes.get(i + 8..i + 8 + len).map(<[u8]>::to_vec);
        }
        i += 8 + len + (len & 1);
    }
    None
}

/// A routine token's address in hex, which is what `glulxdump` prints. Glulx
/// tokens hold plain addresses; `RoutineRef`'s other two spellings are the
/// Z-machine's preactions index and packed address, and no Glulx story can
/// produce either.
fn hex(r: &RoutineRef) -> String {
    match r {
        RoutineRef::Address(a) => format!("{a:x}"),
        other => other.describe(),
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let sentences = args.iter().any(|a| a == "--sentences");
    let tables_only = args.iter().any(|a| a == "--tables");
    let raw = args.iter().any(|a| a == "--raw");
    args.retain(|a| !a.starts_with("--"));
    let Some(path) = args.first() else {
        eprintln!("usage: grammar_dump [--tables|--sentences] <story>");
        std::process::exit(2);
    };

    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("{path}: cannot read");
        std::process::exit(2);
    };
    let Some(image) = glulx_image(bytes) else {
        eprintln!("{path}: no Glulx image");
        std::process::exit(2);
    };
    let mem = match Memory::new(image) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{path}: {e:?}");
            std::process::exit(2);
        }
    };

    if tables_only {
        match gvm::grammar::locate(&mem) {
            Ok(t) => println!(
                "grammar={:#x} verbs={} actions={:#x} n={} dict={:#x} words={} stride={} \
                 word_size={} char_size={}",
                t.grammar,
                t.verb_count,
                t.actions,
                t.action_count,
                t.dictionary,
                t.word_count,
                t.dict_stride,
                t.dict_word_size,
                t.dict_char_size
            ),
            Err(e) => {
                eprintln!("{path}: {e:?}");
                std::process::exit(1);
            }
        }
        return;
    }

    let grammar = match Grammar::load(&mem) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{path}: no grammar ({e:?})");
            std::process::exit(1);
        }
    };
    let t = grammar.tables();
    if !sentences {
        eprintln!(
            "grammar={:#x} verbs={} actions={} dict={:#x} words={}",
            t.grammar, t.verb_count, t.action_count, t.dictionary, t.word_count
        );
    }
    for verb in grammar.verbs() {
        let name = verb.word().unwrap_or("no-verb");
        if !sentences && !raw {
            println!(
                "{:3}: {:08x}: {} lines: {}",
                verb.number,
                verb.address,
                verb.lines.len(),
                if verb.words.len() > 1 { verb.words[1..].join(", ") } else { String::new() }
            );
        }
        if raw {
            for line in &verb.lines {
                let types: Vec<String> = line
                    .slots
                    .iter()
                    .flat_map(|s| s.alternatives.iter())
                    .map(|t| match t {
                        gvm::grammar::Token::Noun(k) => format!("1:{}", k.name()),
                        gvm::grammar::Token::Word(_) => "2".to_string(),
                        gvm::grammar::Token::FilteredNoun(r) => format!("3:{}", hex(r)),
                        gvm::grammar::Token::Attribute(a) => format!("4:{a:x}"),
                        gvm::grammar::Token::Scope(r) => format!("5:{}", hex(r)),
                        gvm::grammar::Token::Routine(r) => format!("6:{}", hex(r)),
                        _ => "?".to_string(),
                    })
                    .collect();
                println!(
                    "{} ac {:04x} fl {:02x} : {}",
                    verb.number,
                    line.action,
                    u8::from(line.reverse),
                    types.join(" ")
                );
            }
            continue;
        }
        for line in &verb.lines {
            if sentences {
                println!("{}", line.describe(name));
            } else {
                println!("    {}", line.describe(name));
            }
        }
    }
}
