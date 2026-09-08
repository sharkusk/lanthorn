//! Story grammar (syntax) tables — the parts of speech and sentence shapes a
//! story knows, as opposed to the flat word list `dictionary.rs` returns.
//!
//! # Where the formats are specified
//!
//! The Z-Machine Standards Document specifies the DICTIONARY (§13) and nothing
//! else here: "The grammar tables, used by the parser in an adventure game, are
//! not specified by the Z-machine at all (contrary to popular opinion)"
//! — Inform Technical Manual §8.6. Everything below therefore comes from two
//! non-ZMSD sources, both consulted directly rather than from memory:
//!
//! * **Inform Technical Manual** (Graham Nelson), §8.5 "Dictionary" for the
//!   `dict_par1..3` flag byte and verb/preposition numbering, and §8.6
//!   "Grammar version numbers GV1 and GV2" for both Inform line formats, the
//!   token value tables, the ENDIT marker, the $400 REVERSE bit and the
//!   "adjectives" (preposition) table's 4-byte entries.
//!   <https://www.inform-fiction.org/source/tm/TechMan.txt>
//!
//! * **ztools / infodump** (Mark Howell; V6 grammar work by Matthew T. Russotto),
//!   `showverb.c` — the reference implementation, and the only written
//!   description of Infocom's own table shapes: the fixed 8-byte and variable
//!   2/4/7-byte ZIL syntax entries, the preposition-table two forms, and the
//!   wholly different Version 6 layout used by Zork Zero, Shogun and Arthur.
//!   Format constants (`VERB` $40, `PREP` $08, `DESC` $20, `NOUN` $80,
//!   `DATA_FIRST` $03, `ENDIT` $0F) are from its `tx.h`.
//!   <https://github.com/ecliptik/ztools>
//!
//! The Version 6 dictionary FLAG BYTE is the one thing here neither source
//! describes: `tx.h` names only `VERB_V6` and infodump prints no parts of speech
//! for a V6 story, so the three bits that carry across Zork Zero, Shogun and
//! Arthur were measured through those games' own parsers and are documented at
//! `F_INFOCOM_V6_VERB` (SQ-1153). Bits above the third are per-game, and this
//! module declines to name them.
//!
//! Every table shape below was checked against `infodump -g` output for real
//! stories; see `crates/zvm/tests/grammar_tables.rs`.
//!
//! # The five shapes
//!
//! All but the Version 6 Infocom games put a table of 2-byte pointers at the
//! base of static memory (header $0E), one pointer per verb. A verb's number in
//! the dictionary counts DOWN from 255, so verb number 255 is table slot 0.
//! Each pointer leads to a count byte and then that many syntax lines:
//!
//! ```text
//! InfocomFixed     8 bytes/line: [objs][prep1][prep2][.. 4 ..][action]
//! InfocomVariable  2, 4 or 7 bytes/line, sized by the top two bits of byte 0
//! Inform5 / Gv1    8 bytes/line: [params][6 token bytes][action]
//! InformGv2        variable: [action word][3-byte tokens..][ENDIT]
//!
//! InfocomV6        no pointer table at all — the dictionary entry itself
//!                  holds the address of an 8-byte verb record, which points
//!                  at separate one-object and two-object entry blocks.
//! ```
//!
//! # The compilers that emit no such table
//!
//! Not every Z-machine story has a grammar table to find. **Dialog** (Linus
//! Åkesson; community fork at <https://github.com/Dialog-IF/dialog>) compiles a
//! predicate language to Z-code and keeps its parser in the story's own library
//! code — `(understand $ as $)` querying a `(grammar entry $ $ $)` predicate, in
//! Dialog's own data representation, indistinguishable from any other predicate
//! once compiled. The compiler proves it: the string "grammar" does not occur
//! anywhere in `dialogc`'s sources, and `src/backend_z.c` lays static memory out
//! as the optimised alphabet table (when used), then wordmaps, then data tables,
//! then the dictionary — no verb-pointer array, at the base of static memory or
//! anywhere else.
//!
//! Dialog signs every Z-machine story it emits, which is what lets us say so
//! rather than guess (`backend_z.c`, the header block): byte $38 is `*` for a
//! `-dev` build and zero otherwise, $39..$3B are `D`, `i`, `a`, and $3C..$3F are
//! the four characters of the version with its slash removed — `0m03` for
//! release 0m/03, `1a01` for 1a/01. That is the same $3C..$3F slot Inform stamps
//! `6.NN` into, so a Dialog story reads as Inform 5 to a version check alone.
//! [`is_dialog`] tests the three-byte signature, and [`Grammar::load`] answers
//! [`GrammarError::Absent`] for such a story — the honest answer, and one that
//! forecloses the real hazard, which is a Dialog wordmap whose leading bytes
//! happen to pass the verb-table shape checks and yield a fabricated grammar.
//!
//! # What this module is not
//!
//! A read-only description of what the story's parser will accept. It is not a
//! parser, it does not rewrite input, and it emits no player-facing text. The
//! `describe` methods exist to diff this module against `infodump -g` and for
//! debug inspectors; a consumer showing something to a player writes its own
//! wording.

use std::collections::BTreeMap;

use crate::dictionary::{self, Dictionary};
use crate::memory::Memory;
use crate::text::decode_string;

// The shape of the ANSWER is shared with `gvm::grammar` and lives in the
// `grammar-model` crate (SQ-1103), as does the CONTAINER it arrives in
// (`Vocabulary`, SQ-1108); the READERS share nothing, and that split was taken
// on evidence. Re-exported here so `zvm::grammar::Token` still names the type.
// What stayed behind is what is about the FORMAT rather than the answer:
// `GrammarFormat` (five table shapes, of which Glulx has none) and
// `GrammarError` (whose refusals are this reader's, down to the variant).
pub use grammar_model::{NounKind, RoutineRef, Slot, SyntaxLine, Token, Verb, WordRoles};

use grammar_model::Vocabulary;

// ── Format constants, from ztools `tx.h` and Inform Technical Manual §8.5 ────

/// Infocom V1–5 dictionary flag: the word can be a verb.
const F_INFOCOM_VERB: u8 = 0x40;
/// Infocom V1–5 dictionary flag: the word can be a noun.
const F_INFOCOM_NOUN: u8 = 0x80;
/// Infocom V1–5 dictionary flag: the word can be an adjective ("descriptor").
const F_INFOCOM_DESC: u8 = 0x20;
/// Infocom V1–5 dictionary flag: the word can be a preposition.
const F_INFOCOM_PREP: u8 = 0x08;
/// Infocom V1–5 dictionary flag: the word is "special" (buzzword/direction).
const F_INFOCOM_SPECIAL: u8 = 0x04;
/// Infocom V1–5: which data byte comes first (`DATA_FIRST` in `tx.h`).
const F_INFOCOM_DATA_FIRST: u8 = 0x03;
/// `DATA_FIRST` value meaning the verb number is the first data byte.
const F_INFOCOM_VERB_FIRST: u8 = 0x01;

/// Inform dictionary flag bit 0: the word can be a verb.
const F_INFORM_VERB: u8 = 0x01;
/// Inform dictionary flag bit 1: the verb is "meta".
const F_INFORM_META: u8 = 0x02;
/// Inform dictionary flag bit 2: the noun is plural (`//p`).
const F_INFORM_PLURAL: u8 = 0x04;
/// Inform dictionary flag bit 3: the word appears literally in grammar lines
/// (Inform calls it "adj"; in practice these are prepositions).
const F_INFORM_ADJ: u8 = 0x08;
/// Inform dictionary flag bit 7: the word can be a noun.
const F_INFORM_NOUN: u8 = 0x80;

// ── Infocom's Version 6 flag byte, measured rather than looked up ────────────
//
// Infocom's three parser-driven V6 games do NOT keep the V1–5 bits above, and
// they do not keep Inform's either — reading them as Inform's is what SQ-1153
// existed to fix. The flag byte is the LAST byte of the dictionary entry (the
// data word before it is the verb record's address, which is why the entry is
// 9 bytes in Zork Zero and 10 in Arthur and Shogun); the three constants below
// are the only bits that mean the same thing in all three games.
//
// **Only the low three.** Above bit 2 the byte — and, in Arthur and Shogun, the
// second flag byte in front of it — is each game's own allocation of whatever
// word classes its ZIL source declared, one bit each, from bit 0 upwards and
// rounded up to whole bytes. Line the three games' fields up as one big-endian
// bitfield and the first three bits agree and nothing else does:
//
//   bit   Zork Zero (1 byte)          Arthur (2 bytes)        Shogun (2 bytes)
//   0     verb                        verb                    verb
//   1     noun                        noun                    noun
//   2     adjective                   adjective               adjective
//   3     directions + once/twice     don't once thrice twice articles
//   4     prepositions + articles     articles                again be not oops
//   5     ' again but except the      ' again be not of the   DIRECTIONS
//   6     . ! ? ask order tell then   DIRECTIONS              am are is was were
//   7     buzzword aggregate          am are is was were      how what when who
//   8–15  (no second byte)            wh-words, modals,       wh-words, modals,
//                                     PREPOSITIONS, …         PREPOSITIONS, …
//
// Shogun is Arthur's allocation minus one class with everything above it
// shifted down a bit, which is exactly what a per-game declaration order
// produces and is not a layout anybody can predict from one title. So
// [`decode_roles`] reads three bits and leaves `preposition` and `special`
// false — "this family has no such bit", per [`WordRoles::from_raw`] — rather
// than picking whichever bit fits the game in front of it.
//
// Verified against the games' own parsers, not against this reader (SQ-1153).
// Booted headless to the first prompt and asked `x <word>` for every
// three-letter-or-longer all-alphabetic non-verb dictionary word, the three
// stories answer:
//
//   * of 1805 words with $02 — 501 Arthur, 751 Zork Zero, 553 Shogun — the
//     parser refuses **two** ("gonzale" and "the", both Shogun) and takes the
//     rest as the name of a thing ("You can't see any spyglass right here.");
//   * of the words with $04 and not $02, 483 are answered with the parser's own
//     word for a descriptor standing alone — "You can't see any **elongated
//     object** right here" — and three are refused;
//   * words with none of the three are refused, or answered as buzzwords, and
//     are never taken as the name of a thing.
//
// Under the Inform reading this replaced, $80 selected `are is was were will`
// on Arthur, six function words on Zork Zero and nothing at all on Shogun, and
// `x were` is refused by Arthur's parser in as many words.

/// Infocom V6 dictionary flag: the word can be a verb (`VERB_V6` in ztools'
/// `tx.h`, and the bit [`load_v6`] reads the verb record's address from).
const F_INFOCOM_V6_VERB: u8 = 0x01;
/// Infocom V6 dictionary flag: the word can be a noun.
const F_INFOCOM_V6_NOUN: u8 = 0x02;
/// Infocom V6 dictionary flag: the word can be an adjective ("descriptor").
const F_INFOCOM_V6_DESC: u8 = 0x04;

/// GV2's end-of-line marker (Inform Technical Manual §8.6).
const ENDIT: u8 = 0x0F;

/// A sanity ceiling on the verb-pointer table. Both Inform and Infocom number
/// verbs downwards from 255, so 256 is the real limit; the slack allows for a
/// future 2-byte non-inverted index without turning a valid table into an error.
const MAX_VERBS: u32 = 512;

/// A sanity ceiling on syntax lines per verb. Inform 6.10 allows 32; Infocom's
/// own games stay well under. Anything larger means we are not reading a table.
const MAX_LINES_PER_VERB: u8 = 64;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Why a story's grammar could not be read.
///
/// Every variant except [`Absent`](GrammarError::Absent) means the bytes did not
/// describe a table this module recognises. Refusing is the point: a grammar
/// parser that silently yields a wrong-but-well-formed table hands its consumer
/// confident nonsense with no way to detect it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GrammarError {
    /// The story has no grammar table. Journey is the canonical example: a
    /// Version 6 game driven entirely by menus, whose dictionary marks no verbs.
    /// Also reached when the verb-pointer table's first entry is zero, and for
    /// any story the Dialog compiler produced — Dialog's parser lives in the
    /// story's own library predicates, so there is no table of any shape to
    /// find (see the module header, and [`is_dialog`]).
    Absent,
    /// A table address or entry ran past the end of the story file.
    Truncated,
    /// The verb-pointer table's shape is impossible (zero-length, absurdly
    /// large, or pointing backwards into itself).
    BadVerbTable,
    /// A syntax line held a value the format forbids — an object count above 2,
    /// a GV1 token in the illegal 9–15 or 112–127 ranges, an unknown GV2 token
    /// type, or a GV2 line that never reached its ENDIT.
    BadSyntaxLine,
    /// The grammar table's total size is not a whole number of entries for
    /// either Inform grammar version (Inform Technical Manual §8.6).
    BadTableSize,
}

// ── The one value type that is this engine's own ─────────────────────────────

/// Which of the five table shapes a story uses. There is no Glulx counterpart:
/// that back-end has exactly one shape, so it has nothing to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GrammarFormat {
    /// Infocom ZIL, fixed 8-byte syntax lines (the earlier games).
    InfocomFixed,
    /// Infocom ZIL, variable 2/4/7-byte syntax lines (the later V3–V5 games).
    InfocomVariable,
    /// Infocom's Version 6 games — Zork Zero, Shogun, Arthur.
    InfocomV6,
    /// Inform 1–5, whose lines are GV1-shaped.
    Inform5,
    /// Inform 6 grammar version 1.
    InformGv1,
    /// Inform 6 grammar version 2 (library 6/3 and later).
    InformGv2,
}

impl GrammarFormat {
    /// True for the three Infocom-era shapes.
    pub fn is_infocom(self) -> bool {
        matches!(
            self,
            GrammarFormat::InfocomFixed | GrammarFormat::InfocomVariable | GrammarFormat::InfocomV6
        )
    }

    /// True for the three Inform-compiler shapes.
    pub fn is_inform(self) -> bool {
        !self.is_infocom()
    }
}

/// A story's grammar: which words are verbs, and what each verb accepts.
///
/// Built once from the story image and self-contained afterwards — no `&Memory`
/// is needed to query it, so it can be cached beside a session or handed to
/// another thread. Cost is proportional to the dictionary (a few hundred
/// kilobytes for the largest stories).
#[derive(Debug, Clone)]
pub struct Grammar {
    /// This engine's own fact: which of the five table shapes was read.
    format: GrammarFormat,
    /// Everything a Glulx grammar also has — the verbs, the spelling index,
    /// the prepositions, the dictionary roles and the action routines. Shared
    /// with `gvm::grammar::Grammar`, which composes the same value (SQ-1108).
    /// The accessors below delegate to it one for one, so this type's public
    /// API is unchanged and reads on its own.
    vocab: Vocabulary,
}

impl Grammar {
    /// Read the story's grammar tables.
    ///
    /// Returns [`GrammarError::Absent`] for a story that has no grammar (a menu
    /// -driven Version 6 game such as Journey), and one of the other variants
    /// when the bytes do not describe a table this module recognises.
    pub fn load(mem: &Memory) -> Result<Grammar, GrammarError> {
        // Dialog first: its parser is library code, so there is no table of any
        // shape here, and its static memory begins with wordmaps that a shape
        // check could in principle mistake for a verb-pointer array.
        if is_dialog(mem) {
            return Err(GrammarError::Absent);
        }
        let format = detect_format(mem);
        let dict = dictionary::load(mem);
        let words = scan_dictionary(mem, &dict, format);

        let mut roles = BTreeMap::new();
        for w in &words {
            roles.insert(w.text.clone(), w.roles);
        }

        // `detect_format` narrows the family from the header; the two
        // within-family ambiguities (Infocom fixed vs variable, Inform GV1 vs
        // GV2) can only be settled by the table itself, so `load_classic`
        // returns the format it actually read.
        let (format, verbs, action_routines) = if format == GrammarFormat::InfocomV6 {
            let (v, a) = load_v6(mem, &words)?;
            (format, v, a)
        } else {
            load_classic(mem, format, &words)?
        };

        Ok(Grammar { format, vocab: Vocabulary::new(verbs, roles, action_routines) })
    }

    /// Which table shape this story uses.
    pub fn format(&self) -> GrammarFormat {
        self.format
    }

    /// Every verb, in grammar-table order.
    pub fn verbs(&self) -> &[Verb] {
        self.vocab.verbs()
    }

    /// The verb a spelling belongs to, if it is one.
    pub fn verb_for_word(&self, word: &str) -> Option<&Verb> {
        self.vocab.verb_for_word(word)
    }

    /// True if the story can begin a command with this word.
    pub fn is_verb(&self, word: &str) -> bool {
        self.vocab.is_verb(word)
    }

    /// Every spelling that can begin a command, sorted.
    pub fn verb_words(&self) -> impl Iterator<Item = &str> {
        self.vocab.verb_words()
    }

    /// Every literal word the grammar names, deduplicated and sorted — the
    /// story's prepositions.
    pub fn prepositions(&self) -> &[String] {
        self.vocab.prepositions()
    }

    /// True if the grammar uses this word literally in some line.
    pub fn is_preposition(&self, word: &str) -> bool {
        self.vocab.is_preposition(word)
    }

    /// The parts of speech the dictionary marks `word` with, if it knows it.
    pub fn roles(&self, word: &str) -> Option<WordRoles> {
        self.vocab.roles(word)
    }

    /// Every word the dictionary holds, sorted — the whole vocabulary, verbs
    /// and nouns and buzzwords alike. `gvm::grammar::Grammar::words` answers
    /// the same question about a Glulx story (SQ-1103); the words of one
    /// syntax LINE are [`SyntaxLine::literals`].
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.vocab.words()
    }

    /// Unpacked byte addresses of the action routines, indexed by action number.
    /// Empty for [`GrammarFormat::InfocomV6`], whose action table this module
    /// locates but does not walk.
    pub fn action_routines(&self) -> &[u32] {
        self.vocab.action_routines()
    }

    /// Every verb with a line matching `nouns` noun phrases and exactly `words`
    /// as its literal words — the shape query a caller uses to keep a suggestion
    /// plausible instead of merely near.
    pub fn verbs_accepting(&self, nouns: usize, words: &[&str]) -> Vec<&Verb> {
        self.vocab.verbs_accepting(nouns, words)
    }
}

// ── Dictionary scan ──────────────────────────────────────────────────────────

/// One dictionary entry, decoded far enough to answer part-of-speech questions.
struct DictWord {
    /// Byte address of the entry, which is also the value a `name`/`SYNONYM`
    /// property stores to refer to this word.
    address: u32,
    text: String,
    roles: WordRoles,
    /// The verb number (classic formats) or verb-record address (V6), when the
    /// flags mark this word a verb.
    verb_key: Option<u16>,
}

/// Decode the whole dictionary once, reading each entry's flag byte and, for
/// verbs, the number that links it to the grammar table.
///
/// Flag byte position: the entry's data follows the 4-byte (v1–3) or 6-byte
/// (v4+) key, so the flags are the first data byte — except in Infocom's V6
/// games, where `showverb.c` records that the flags moved to the LAST byte of
/// the entry and the first data word became the verb record's address.
fn scan_dictionary(mem: &Memory, dict: &Dictionary, format: GrammarFormat) -> Vec<DictWord> {
    let elen = dict.entry_length as u32;
    let klen = dict.key_len() as u32;
    let mut out = Vec::with_capacity(dict.count as usize);

    for i in 0..dict.count as u32 {
        let entry = dict.base + i * elen;
        if (entry + elen) as usize > mem.len() {
            break;
        }
        let (text, _) = decode_string(mem, entry);
        let text = text.trim().to_lowercase();
        if text.is_empty() {
            continue;
        }

        let (flags, data0, data1) = if format == GrammarFormat::InfocomV6 {
            (mem.read_byte(entry + elen - 1), 0, 0)
        } else {
            (
                mem.read_byte(entry + klen),
                mem.read_byte(entry + klen + 1),
                mem.read_byte(entry + klen + 2),
            )
        };

        let roles = decode_roles(flags, format);

        let verb_key = if format == GrammarFormat::InfocomV6 {
            // The verb record's address lives in the first data word, and the
            // verb bit is $01 (`VERB_V6` in ztools' `tx.h`). A word may be both
            // noun and verb — Shogun's "sail", "ship" and "bow" all are — so
            // the noun bit does not disqualify it here. `load_v6` applies the
            // stricter test separately, where it belongs: to bounding the verb
            // record area.
            let addr = mem.read_word(entry + klen);
            (flags & F_INFOCOM_V6_VERB != 0 && addr != 0).then_some(addr)
        } else if format.is_inform() {
            // Inform Technical Manual §8.5: dict_par2 is the verb number.
            (flags & F_INFORM_VERB != 0).then_some(data0 as u16)
        } else if flags & F_INFOCOM_VERB != 0 {
            // ztools `lookup_word`: the verb number is the first data byte when
            // DATA_FIRST says VERB_FIRST, otherwise the second.
            let n = if flags & F_INFOCOM_DATA_FIRST == F_INFOCOM_VERB_FIRST { data0 } else { data1 };
            Some(n as u16)
        } else {
            None
        };

        out.push(DictWord { address: entry, text, roles, verb_key });
    }

    out
}

fn decode_roles(flags: u8, format: GrammarFormat) -> WordRoles {
    // `WordRoles` is shared with `gvm` and `#[non_exhaustive]`, so it is built
    // from the flag field and then filled in. The bits this family does not
    // define are left false, which is exactly what they mean here: `singular`
    // and `truncated` are Glulx's, and the Z-machine dictionary has no room
    // for either.
    let mut roles = WordRoles::from_raw(u16::from(flags));
    if format == GrammarFormat::InfocomV6 {
        // Three bits, and only three: see the constants' own header for the
        // per-game allocation above bit 2 and for the parser measurement that
        // settled these. `preposition` and `special` stay false because this
        // family keeps no bit that means either in every game — not because
        // these stories have no prepositions.
        roles.verb = flags & F_INFOCOM_V6_VERB != 0;
        roles.noun = flags & F_INFOCOM_V6_NOUN != 0;
        roles.adjective = flags & F_INFOCOM_V6_DESC != 0;
    } else if format.is_inform() {
        roles.verb = flags & F_INFORM_VERB != 0;
        roles.noun = flags & F_INFORM_NOUN != 0;
        roles.preposition = flags & F_INFORM_ADJ != 0;
        roles.meta = flags & F_INFORM_META != 0;
        roles.plural = flags & F_INFORM_PLURAL != 0;
    } else {
        roles.verb = flags & F_INFOCOM_VERB != 0;
        roles.noun = flags & F_INFOCOM_NOUN != 0;
        roles.adjective = flags & F_INFOCOM_DESC != 0;
        roles.preposition = flags & F_INFOCOM_PREP != 0;
        roles.special = flags & F_INFOCOM_SPECIAL != 0;
    }
    roles
}

// ── Dictionary words on their own ────────────────────────────────────────────

/// One dictionary entry: where it is, what it says, and what parts of speech
/// the story marks it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryWord {
    /// Byte address of the entry. This is the value an object's parse-name
    /// property stores, so it is the key a reverse lookup needs.
    pub address: u32,
    /// The decoded word, lower-cased and truncated as the story stores it.
    pub text: String,
    /// The flags decoded for this story's compiler family.
    pub roles: WordRoles,
}

/// Every dictionary word with its address and roles, **without reading the
/// grammar table**.
///
/// [`Grammar::load`] answers the same question but has to find and walk the
/// syntax tables first, and refuses outright when they are malformed — which
/// is the right answer for "what sentences does this story accept?" and the
/// wrong one for "is this word a noun?". Journey and Scopa have no parser and
/// no usable verb table, and still have a dictionary whose flags are perfectly
/// readable; so does any story whose grammar version we do not recognise.
///
/// The format is settled from the header alone ([`detect_format`]), which is
/// what decides where the flag byte sits and how its bits are read.
pub fn dictionary_words(mem: &Memory) -> Vec<DictionaryWord> {
    let dict = dictionary::load(mem);
    let format = detect_format(mem);
    scan_dictionary(mem, &dict, format)
        .into_iter()
        .map(|w| DictionaryWord { address: w.address, text: w.text, roles: w.roles })
        .collect()
}

// ── Format detection ─────────────────────────────────────────────────────────

/// Decide which family compiled this story, from the header alone.
///
/// `showverb.c`'s test, unchanged: a serial number whose six bytes are a
/// plausible YYMMDD *and* whose first digit is not '8' means the Inform
/// compiler, since Infocom's own serials are all 8x. Inform 6 additionally
/// writes its version string into the four bytes at $3C, so a '6' or later
/// there separates Inform 6 from Inform 1–5.
///
/// GV1 versus GV2 needs the table itself and is settled in `load_classic`.
/// True when the Dialog compiler produced this story.
///
/// Dialog writes `D`, `i`, `a` into header bytes $39..$3B of every Z-machine
/// story it emits, unconditionally, and the four characters of its own version
/// into $3C..$3F — `dialogc`'s `src/backend_z.c`:
///
/// ```text
/// if(VERSION[5] == '-' && VERSION[6] == 'd') { zcore[0x38] = '*'; }
/// zcore[0x39] = 'D'; zcore[0x3a] = 'i'; zcore[0x3b] = 'a';
/// zcore[0x3c] = VERSION[0]; ... zcore[0x3f] = VERSION[4];
/// ```
///
/// Those bytes are "unspecified" in the Z-Machine Standards Document, so no
/// other producer has a claim on them; `ImpossibleStairs.z8` reads `Dia0m03`
/// and `frankenfingers_260330.z5` reads `*Dia1a01`, and both print the same
/// version from their own banner (`Dialog compiler version 0m/03` and
/// `1a/01-dev`) — the independent check on this signature.
///
/// The version characters are deliberately NOT tested: the signature is the
/// three letters, and a Dialog release numbered anything at all is still Dialog.
pub fn is_dialog(mem: &Memory) -> bool {
    mem.read_byte(0x39) == b'D' && mem.read_byte(0x3A) == b'i' && mem.read_byte(0x3B) == b'a'
}

/// Which family's grammar tables a story uses, judged from the header alone.
///
/// **Answers an Inform variant for a Dialog story**, whose $3C..$3F holds a
/// Dialog version where Inform stamps `6.NN`; [`is_dialog`] is the test that
/// tells them apart, and [`Grammar::load`] applies it first. Nothing here is
/// changed for Dialog on purpose — [`crate::objects::ParseNames`] reads this to
/// choose between Inform's fixed `name` property and a tallied Infocom one, and
/// the Inform branch is the one that then *refuses* on a Dialog story rather
/// than adopting whatever property the tally liked best.
pub fn detect_format(mem: &Memory) -> GrammarFormat {
    let s: Vec<u8> = (0x12..0x18).map(|a| mem.read_byte(a)).collect();
    let digit = |b: u8, lo: u8, hi: u8| b >= lo && b <= hi;
    let inform = digit(s[0], b'0', b'9')
        && digit(s[1], b'0', b'9')
        && digit(s[2], b'0', b'1')
        && digit(s[3], b'0', b'9')
        && digit(s[4], b'0', b'3')
        && digit(s[5], b'0', b'9')
        && s[0] != b'8';

    if inform {
        // Byte $3C is the first character of Inform 6's version string.
        if mem.read_byte(0x3C) >= b'6' {
            GrammarFormat::InformGv1 // refined to GV2 once the table is read
        } else {
            GrammarFormat::Inform5
        }
    } else if mem.version() == 6 {
        GrammarFormat::InfocomV6
    } else {
        GrammarFormat::InfocomFixed // refined to Variable once the table is read
    }
}

// ── The classic (pointer-table) formats ──────────────────────────────────────

/// Everything the two passes over the verb table need to agree about.
struct ClassicLayout {
    format: GrammarFormat,
    verb_table_base: u32,
    verb_count: u32,
    action_table_base: u32,
    action_count: u32,
    /// Base of the preposition ("adjectives") table; zero for GV2, which has none.
    prep_table_base: u32,
    /// 0 = 4-byte entries with a word index, 1 = 3-byte entries with a byte index.
    prep_entry_form: u8,
}

/// Read every verb of a pointer-table format, returning the format actually
/// found alongside the verbs and the action-routine table.
fn load_classic(
    mem: &Memory,
    detected: GrammarFormat,
    words: &[DictWord],
) -> Result<(GrammarFormat, Vec<Verb>, Vec<u32>), GrammarError> {
    let layout = configure_classic(mem, detected)?;
    let preps = read_preposition_table(mem, &layout)?;

    // Verb number → its dictionary spellings, in dictionary order.
    let mut spellings: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for w in words {
        if let Some(n) = w.verb_key {
            spellings.entry(n).or_default().push(w.text.clone());
        }
    }

    let mut verbs = Vec::with_capacity(layout.verb_count as usize);
    for i in 0..layout.verb_count {
        // The dictionary's verb numbers count downwards from 255, so table slot
        // i belongs to verb number 255 - i (Inform Technical Manual §8.5; the
        // same convention in Infocom's own games).
        let number = 255u16.wrapping_sub(i as u16);
        let entry_ptr = layout.verb_table_base + i * 2;
        let address = read_word(mem, entry_ptr)? as u32;
        let mut cur = address;
        let count = read_byte(mem, cur)?;
        cur += 1;
        if count > MAX_LINES_PER_VERB {
            return Err(GrammarError::BadVerbTable);
        }

        let mut lines = Vec::with_capacity(count as usize);
        for _ in 0..count {
            lines.push(read_classic_line(mem, &layout, &preps, &mut cur)?);
        }

        verbs.push(Verb::new(
            u32::from(number),
            address,
            spellings.remove(&number).unwrap_or_default(),
            lines,
        ));
    }

    let mut action_routines = Vec::with_capacity(layout.action_count as usize);
    for i in 0..layout.action_count {
        let packed = read_word(mem, layout.action_table_base + i * 2)?;
        action_routines.push(mem.unpack_routine(packed));
    }

    Ok((layout.format, verbs, action_routines))
}

/// Locate the tables and settle the two format ambiguities (Infocom fixed vs
/// variable, Inform GV1 vs GV2), following `showverb.c::configure_parse_tables`.
fn configure_classic(mem: &Memory, detected: GrammarFormat) -> Result<ClassicLayout, GrammarError> {
    let verb_table_base = mem.static_mem_base() as u32;
    let first = read_word(mem, verb_table_base)? as u32;
    if first == 0 {
        return Err(GrammarError::Absent);
    }
    if first <= verb_table_base || !(first - verb_table_base).is_multiple_of(2) {
        return Err(GrammarError::BadVerbTable);
    }
    let verb_count = (first - verb_table_base) / 2;
    if verb_count == 0 || verb_count > MAX_VERBS {
        return Err(GrammarError::BadVerbTable);
    }
    if first as usize >= mem.len() {
        return Err(GrammarError::Truncated);
    }

    // The second pointer bounds the first verb's data, which is what tells the
    // two Infocom shapes and the two Inform grammar versions apart.
    let second = if verb_count >= 2 { read_word(mem, verb_table_base + 2)? as u32 } else { first };
    let entry_count = read_byte(mem, first)?;
    let span = second.saturating_sub(first);

    let format = match detected {
        GrammarFormat::InfocomFixed => {
            if entry_count > 0 && span > 0 && span / entry_count as u32 <= 7 {
                GrammarFormat::InfocomVariable
            } else {
                GrammarFormat::InfocomFixed
            }
        }
        GrammarFormat::InformGv1 => classify_inform6(mem, first, span, entry_count)?,
        other => other,
    };

    // Pass one: the highest action number and parsing-routine number in use,
    // and where the grammar data ends.
    let mut action_count = 0u32;
    let mut parse_count = 0u32;
    let mut data_end = first;
    for i in 0..verb_count {
        let mut cur = read_word(mem, verb_table_base + i * 2)? as u32;
        let n = read_byte(mem, cur)?;
        cur += 1;
        if n > MAX_LINES_PER_VERB {
            return Err(GrammarError::BadVerbTable);
        }
        for _ in 0..n {
            let (action, parse_max) = measure_classic_line(mem, format, &mut cur)?;
            action_count = action_count.max(action as u32 + 1);
            parse_count = parse_count.max(parse_max);
        }
        data_end = data_end.max(cur);
    }

    // The action table follows the grammar data, after any zero padding.
    let mut action_table_base = data_end;
    let mut skipped = 0;
    while read_byte(mem, action_table_base)? == 0 {
        action_table_base += 1;
        skipped += 1;
        if skipped > 32 {
            return Err(GrammarError::BadVerbTable);
        }
    }

    let preact_table_base = action_table_base + action_count * 2;
    let (prep_table_base, prep_entry_form) = if format == GrammarFormat::InformGv2 {
        // GV2 has neither a preactions nor an adjectives table.
        (0, 0)
    } else {
        let base = if format == GrammarFormat::InformGv1 {
            preact_table_base + parse_count * 2
        } else {
            preact_table_base + action_count * 2
        };
        // Form 0 stores the index in a word, so the byte after the first
        // entry's dictionary address is that word's zero high byte.
        let form = u8::from(read_byte(mem, base + 4)? != 0);
        (base, form)
    };

    Ok(ClassicLayout {
        format,
        verb_table_base,
        verb_count,
        action_table_base,
        action_count,
        prep_table_base,
        prep_entry_form,
    })
}

/// GV1 or GV2, by the size of the first verb's grammar data.
///
/// A GV1 line is 8 bytes and a table is `1 + 8n`; a GV2 line is `2 + 3t + 1` and
/// a table is `1 + sum`, so `1 mod 3`. `1 mod 24` fits both and is settled by
/// looking for values GV1 forbids (Inform Technical Manual §8.6 lists 9–15 and
/// 112–127 as illegal token bytes).
///
/// `showverb.c` writes that disambiguation as
/// `if ((val >= 9) || (val <= 15) || (val >= 112) || (val <= 127))`, which is
/// true for almost every byte and so declares GV2 unconditionally. The `&&`
/// pairing plainly intended is used here instead; the difference can only show
/// on a table whose first verb's data is `1 mod 24` bytes long.
fn classify_inform6(
    mem: &Memory,
    first: u32,
    span: u32,
    entry_count: u8,
) -> Result<GrammarFormat, GrammarError> {
    if span == 0 {
        return Ok(GrammarFormat::InformGv1);
    }
    if span % 3 == 1 {
        if entry_count as u32 * 8 + 1 != span {
            return Ok(GrammarFormat::InformGv2);
        }
        // Ambiguous: walk the GV1 reading and see whether it is legal.
        let mut cur = first + 1;
        for _ in 0..entry_count {
            if read_byte(mem, cur)? > 6 {
                return Ok(GrammarFormat::InformGv2);
            }
            cur += 1;
            for _ in 0..6 {
                let v = read_byte(mem, cur)?;
                cur += 1;
                if (9..=15).contains(&v) || (112..=127).contains(&v) {
                    return Ok(GrammarFormat::InformGv2);
                }
            }
            cur += 1; // action byte, unconstrained
        }
        Ok(GrammarFormat::InformGv1)
    } else if span % 8 == 1 {
        Ok(GrammarFormat::InformGv1)
    } else {
        Err(GrammarError::BadTableSize)
    }
}

/// Advance `cur` past one syntax line without decoding it, reporting the action
/// number and the highest GV1 parsing-routine number it names.
fn measure_classic_line(
    mem: &Memory,
    format: GrammarFormat,
    cur: &mut u32,
) -> Result<(u16, u32), GrammarError> {
    match format {
        GrammarFormat::InfocomFixed => {
            let action = read_byte(mem, *cur + 7)?;
            *cur += 8;
            Ok((action as u16, 0))
        }
        GrammarFormat::InfocomVariable => {
            let b0 = read_byte(mem, *cur)?;
            let action = read_byte(mem, *cur + 1)?;
            *cur += variable_line_size(b0)?;
            Ok((action as u16, 0))
        }
        GrammarFormat::Inform5 | GrammarFormat::InformGv1 => {
            let mut parse_max = 0;
            for j in 1..7 {
                let v = read_byte(mem, *cur + j)?;
                if (16..112).contains(&v) {
                    parse_max = parse_max.max((v as u32 - 16) % 32 + 1);
                }
            }
            let action = read_byte(mem, *cur + 7)?;
            *cur += 8;
            Ok((action as u16, parse_max))
        }
        GrammarFormat::InformGv2 => {
            let action = read_word(mem, *cur)? & 0x03FF;
            *cur += 2;
            let mut guard = 0;
            loop {
                let t = read_byte(mem, *cur)?;
                *cur += 1;
                if t == ENDIT {
                    break;
                }
                *cur += 2;
                guard += 1;
                if guard > 64 {
                    return Err(GrammarError::BadSyntaxLine);
                }
            }
            Ok((action, 0))
        }
        GrammarFormat::InfocomV6 => unreachable!("V6 has no pointer table"),
    }
}

/// Byte length of an Infocom variable-form line, from the object count in the
/// top two bits of its first byte (`verb_sizes` in `showverb.c`).
fn variable_line_size(b0: u8) -> Result<u32, GrammarError> {
    match (b0 >> 6) & 0x03 {
        0 => Ok(2),
        1 => Ok(4),
        2 => Ok(7),
        _ => Err(GrammarError::BadSyntaxLine),
    }
}

fn read_classic_line(
    mem: &Memory,
    layout: &ClassicLayout,
    preps: &BTreeMap<u16, String>,
    cur: &mut u32,
) -> Result<SyntaxLine, GrammarError> {
    match layout.format {
        GrammarFormat::InfocomFixed => {
            let objs = read_byte(mem, *cur)?;
            let p0 = read_byte(mem, *cur + 1)?;
            let p1 = read_byte(mem, *cur + 2)?;
            let action = read_byte(mem, *cur + 7)?;
            *cur += 8;
            infocom_line(objs, prep_index(p0), prep_index(p1), action, preps)
        }
        GrammarFormat::InfocomVariable => {
            let b0 = read_byte(mem, *cur)?;
            let action = read_byte(mem, *cur + 1)?;
            let size = variable_line_size(b0)?;
            let objs = (b0 >> 6) & 0x03;
            // The first preposition's low six bits are packed into byte 0; the
            // top two bits of a preposition index are always set.
            let p0 = if b0 & 0x3F != 0 { Some(b0 as u16 | 0xC0) } else { None };
            let p1 = if objs > 1 {
                let b4 = read_byte(mem, *cur + 4)?;
                if b4 & 0x3F != 0 {
                    Some(b4 as u16 | 0xC0)
                } else {
                    None
                }
            } else {
                None
            };
            *cur += size;
            infocom_line(objs, p0, p1, action, preps)
        }
        GrammarFormat::Inform5 | GrammarFormat::InformGv1 => {
            let mut params = read_byte(mem, *cur)?;
            let mut slots = Vec::new();
            for j in 1..7u32 {
                let v = read_byte(mem, *cur + j)?;
                if v >= 0xB0 {
                    let word = preps.get(&(v as u16)).cloned().unwrap_or_default();
                    slots.push(Slot::one(Token::Word(word)));
                    continue;
                }
                if v == 0 && params == 0 {
                    break; // trailing null padding
                }
                slots.push(Slot::one(gv1_token(v)?));
                params = params.saturating_sub(1);
            }
            let action = read_byte(mem, *cur + 7)?;
            *cur += 8;
            Ok(SyntaxLine::new(action as u16, false, slots))
        }
        GrammarFormat::InformGv2 => {
            let head = read_word(mem, *cur)?;
            *cur += 2;
            let action = head & 0x03FF;
            let reverse = head & 0x0400 != 0;
            let mut slots: Vec<Slot> = Vec::new();
            loop {
                let t = read_byte(mem, *cur)?;
                *cur += 1;
                if t == ENDIT {
                    break;
                }
                let data = read_word(mem, *cur)?;
                *cur += 2;
                let token = gv2_token(mem, t & 0x0F, data)?;
                // Bits 4–5: $$10 opens a list of alternatives, $$01 continues one.
                let continues = (t >> 4) & 0x01 != 0;
                match (continues, slots.last_mut()) {
                    (true, Some(last)) => last.alternatives.push(token),
                    _ => slots.push(Slot::one(token)),
                }
                if slots.len() > 64 {
                    return Err(GrammarError::BadSyntaxLine);
                }
            }
            Ok(SyntaxLine::new(action, reverse, slots))
        }
        GrammarFormat::InfocomV6 => unreachable!("V6 has no pointer table"),
    }
}

/// A preposition byte counts as one only with its top bit set (`showverb.c`
/// checks `>= 0x80` in the fixed form).
fn prep_index(b: u8) -> Option<u16> {
    (b >= 0x80).then_some(b as u16)
}

/// Assemble an Infocom line: `verb [prep] [noun] [prep] [noun]`, exactly the
/// order `showverb.c` prints.
fn infocom_line(
    objs: u8,
    p0: Option<u16>,
    p1: Option<u16>,
    action: u8,
    preps: &BTreeMap<u16, String>,
) -> Result<SyntaxLine, GrammarError> {
    if objs > 2 {
        return Err(GrammarError::BadSyntaxLine);
    }
    let mut slots = Vec::new();
    for (i, p) in [p0, p1].into_iter().enumerate() {
        if let Some(idx) = p {
            slots.push(Slot::one(Token::Word(preps.get(&idx).cloned().unwrap_or_default())));
        }
        if objs as usize > i {
            slots.push(Slot::one(Token::Noun(NounKind::Noun)));
        }
    }
    Ok(SyntaxLine::new(action as u16, false, slots))
}

/// GV1 token byte → token (Inform Technical Manual §8.6).
fn gv1_token(v: u8) -> Result<Token, GrammarError> {
    Ok(match v {
        0..=8 => {
            Token::Noun(NounKind::from_elementary(u32::from(v)).expect("0..=8 is elementary"))
        }
        16..=47 => Token::FilteredNoun(RoutineRef::Index(v - 16)),
        48..=79 => Token::Routine(RoutineRef::Index(v - 48)),
        80..=111 => Token::Scope(RoutineRef::Index(v - 80)),
        128..=175 => Token::Attribute(u32::from(v - 128)),
        // 9–15 and 112–127 are listed as illegal; 176+ was handled as a
        // preposition before reaching here.
        _ => return Err(GrammarError::BadSyntaxLine),
    })
}

/// GV2 token type and data → token (Inform Technical Manual §8.6).
fn gv2_token(mem: &Memory, ty: u8, data: u16) -> Result<Token, GrammarError> {
    Ok(match ty {
        1 => Token::Noun(
            NounKind::from_elementary(u32::from(data)).ok_or(GrammarError::BadSyntaxLine)?,
        ),
        2 => Token::Word(dict_text(mem, data as u32)?),
        3 => Token::FilteredNoun(RoutineRef::Packed(data)),
        4 => Token::Attribute(u32::from(data)),
        5 => Token::Scope(RoutineRef::Packed(data)),
        6 => Token::Routine(RoutineRef::Packed(data)),
        _ => return Err(GrammarError::BadSyntaxLine),
    })
}

/// The preposition ("adjectives") table: a count word, then entries pairing a
/// dictionary address with an index. Inform stores them lowest index first;
/// Infocom's order varies. Either way we only ever look entries up by index.
fn read_preposition_table(
    mem: &Memory,
    layout: &ClassicLayout,
) -> Result<BTreeMap<u16, String>, GrammarError> {
    let mut map = BTreeMap::new();
    if layout.prep_table_base == 0 {
        return Ok(map); // GV2
    }
    let mut cur = layout.prep_table_base;
    let count = read_word(mem, cur)?;
    cur += 2;
    if count as u32 > 512 {
        return Err(GrammarError::BadVerbTable);
    }
    for _ in 0..count {
        let addr = read_word(mem, cur)? as u32;
        cur += 2;
        let index = if layout.prep_entry_form == 0 {
            let w = read_word(mem, cur)?;
            cur += 2;
            w
        } else {
            let b = read_byte(mem, cur)?;
            cur += 1;
            b as u16
        };
        if addr != 0 {
            map.entry(index).or_insert(dict_text(mem, addr)?);
        }
    }
    Ok(map)
}

// ── Infocom's Version 6 shape ────────────────────────────────────────────────

/// Zork Zero, Shogun and Arthur. There is no pointer table: each verb's
/// dictionary entry carries the address of an 8-byte record, and the one- and
/// two-object sentence shapes live in separate blocks that record points at.
/// Layout from `showverb.c`'s commentary (Matthew T. Russotto).
fn load_v6(mem: &Memory, words: &[DictWord]) -> Result<(Vec<Verb>, Vec<u32>), GrammarError> {
    let objects = mem.object_table() as u32;
    if objects < 4 {
        return Err(GrammarError::BadVerbTable);
    }
    // The action and pre-action table addresses sit in the last two globals,
    // immediately below the object table.
    let action_table_base = read_word(mem, objects - 4)? as u32;

    // Bound the verb-record area from the dictionary words that can only be
    // verbs. A word carrying the noun bit as well is a weaker witness — its
    // data word might be a property, not a record address — so it is admitted
    // as a spelling below but never allowed to move the bounds.
    let mut lo = u32::MAX;
    let mut hi = 0u32;
    for w in words {
        if let Some(addr) = w.verb_key {
            if w.roles.noun || (addr as u32) >= action_table_base {
                continue;
            }
            lo = lo.min(addr as u32);
            hi = hi.max(addr as u32 + 8);
        }
    }
    if hi == 0 {
        return Err(GrammarError::Absent);
    }
    if !(hi - lo).is_multiple_of(8) || (hi - lo) / 8 > MAX_VERBS {
        return Err(GrammarError::BadVerbTable);
    }

    // Now attach every spelling that lands on a record inside those bounds.
    let mut spellings: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for w in words {
        if let Some(addr) = w.verb_key {
            let a = addr as u32;
            if a >= lo && a < hi && (a - lo).is_multiple_of(8) {
                spellings.entry(addr).or_default().push(w.text.clone());
            }
        }
    }

    let mut verbs = Vec::new();
    let mut addr = lo;
    while addr < hi {
        let bare_action = read_word(mem, addr)?;
        let one_obj = read_word(mem, addr + 4)? as u32;
        let two_obj = read_word(mem, addr + 6)? as u32;

        let mut lines = Vec::new();
        if bare_action != 0xFFFF {
            lines.push(SyntaxLine::new(bare_action, false, Vec::new()));
        }
        if one_obj != 0 {
            lines.extend(read_v6_block(mem, one_obj, 1)?);
        }
        if two_obj != 0 {
            lines.extend(read_v6_block(mem, two_obj, 2)?);
        }

        // The record address is both this format's verb number and the place
        // its sentence shapes are read from, since there is no pointer array.
        verbs.push(Verb::new(
            addr,
            addr,
            spellings.remove(&(addr as u16)).unwrap_or_default(),
            lines,
        ));
        addr += 8;
    }

    // The V6 action table's extent is not derivable the way the classic one's
    // is; report none rather than a guess.
    Ok((verbs, Vec::new()))
}

/// One block of Version 6 sentence entries: a count word, then that many
/// entries of `2 + 4 * objects` bytes — an action word, then per object a
/// preposition's dictionary address and an attribute/selector pair.
fn read_v6_block(mem: &Memory, base: u32, objects: u32) -> Result<Vec<SyntaxLine>, GrammarError> {
    let count = read_word(mem, base)?;
    if count as u32 > 256 {
        return Err(GrammarError::BadSyntaxLine);
    }
    let stride = 2 + 4 * objects;
    let mut lines = Vec::with_capacity(count as usize);
    for i in 0..count as u32 {
        let mut cur = base + 2 + i * stride;
        let action = read_word(mem, cur)?;
        cur += 2;
        let mut slots = Vec::new();
        for _ in 0..objects {
            let prep = read_word(mem, cur)? as u32;
            cur += 2;
            let attribute = read_byte(mem, cur)?;
            let selector = read_byte(mem, cur + 1)?;
            cur += 2;
            if prep != 0 {
                slots.push(Slot::one(Token::Word(dict_text(mem, prep)?)));
            }
            slots.push(Slot::one(Token::InfocomObject { attribute, selector }));
        }
        lines.push(SyntaxLine::new(action, false, slots));
    }
    Ok(lines)
}

// ── Bounds-checked reads ─────────────────────────────────────────────────────
//
// `Memory::read_byte` latches a memory fault and returns 0 past the end of the
// image, which would turn a corrupt table into a plausible one and disturb the
// interpreter's own fault reporting. A read-only parser checks its own bounds.

fn read_byte(mem: &Memory, addr: u32) -> Result<u8, GrammarError> {
    if addr as usize >= mem.len() {
        return Err(GrammarError::Truncated);
    }
    Ok(mem.read_byte(addr))
}

fn read_word(mem: &Memory, addr: u32) -> Result<u16, GrammarError> {
    if addr as usize + 1 >= mem.len() {
        return Err(GrammarError::Truncated);
    }
    Ok(mem.read_word(addr))
}

/// Decode the dictionary word stored at `addr`.
fn dict_text(mem: &Memory, addr: u32) -> Result<String, GrammarError> {
    if addr as usize >= mem.len() {
        return Err(GrammarError::Truncated);
    }
    let (s, _) = decode_string(mem, addr);
    Ok(s.trim().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::text::encode::encode_word;

    const BASE: u32 = 0x0400; // static memory base in `sample_story`
    const DICT: u32 = 0x0200; // dictionary base in `sample_story`

    /// A hand-built story image: header, dictionary and grammar tables written
    /// byte by byte, so each format's layout is asserted against an oracle we
    /// control rather than against the implementation's own assumptions.
    struct Story {
        buf: Vec<u8>,
        version: u8,
        /// Byte address of each dictionary entry, in the order given.
        dict_addrs: Vec<u32>,
    }

    impl Story {
        /// `serial` picks the family: an Infocom serial starts with '8'.
        fn new(version: u8, serial: &[u8; 6], inform6: bool) -> Story {
            let mut buf = sample_story(version);
            buf.resize(0x1000, 0);
            buf[0x12..0x18].copy_from_slice(serial);
            if inform6 {
                buf[0x3C..0x40].copy_from_slice(b"6.15");
            }
            Story { buf, version, dict_addrs: Vec::new() }
        }

        fn b(&mut self, addr: u32, v: u8) {
            self.buf[addr as usize] = v;
        }

        fn w(&mut self, addr: u32, v: u16) {
            self.buf[addr as usize] = (v >> 8) as u8;
            self.buf[addr as usize + 1] = v as u8;
        }

        /// Write a dictionary of `(word, flags, [d0, d1, d2])` entries.
        fn dictionary(&mut self, entries: &[(&str, u8, [u8; 3])]) {
            let klen: u32 = if self.version <= 3 { 4 } else { 6 };
            let elen = klen + 3;
            let mut cur = DICT;
            self.b(cur, 0); // no word separators
            cur += 1;
            self.b(cur, elen as u8);
            cur += 1;
            self.w(cur, entries.len() as u16);
            cur += 2;
            for (word, flags, data) in entries {
                let key = encode_word(word, self.version);
                for (i, k) in key.iter().take(klen as usize).enumerate() {
                    self.b(cur + i as u32, *k);
                }
                self.b(cur + klen, *flags);
                self.b(cur + klen + 1, data[0]);
                self.b(cur + klen + 2, data[1]);
                self.b(cur + klen + 3, data[2]);
                self.dict_addrs.push(cur);
                cur += elen;
            }
        }

        /// Lay out the grammar area: pointer table, one blob per verb, then the
        /// action, pre-action and preposition tables at the addresses the
        /// format says they follow at.
        fn grammar(
            &mut self,
            blobs: &[Vec<u8>],
            action_count: u32,
            second_table_count: u32,
            preps: &[(u32, u16)],
        ) {
            let n = blobs.len() as u32;
            let mut cur = BASE + n * 2;
            for (i, blob) in blobs.iter().enumerate() {
                self.w(BASE + i as u32 * 2, cur as u16);
                for (j, byte) in blob.iter().enumerate() {
                    self.b(cur + j as u32, *byte);
                }
                cur += blob.len() as u32;
            }
            // Action table: any nonzero packed address will do.
            for i in 0..action_count {
                self.w(cur + i * 2, 0x0101);
            }
            cur += action_count * 2;
            // Pre-action / parsing-routine table.
            for i in 0..second_table_count {
                self.w(cur + i * 2, 0x0202);
            }
            cur += second_table_count * 2;
            // Preposition table, word-index form.
            self.w(cur, preps.len() as u16);
            cur += 2;
            for (addr, index) in preps {
                self.w(cur, *addr as u16);
                self.w(cur + 2, *index);
                cur += 4;
            }
        }

        fn mem(&self) -> Memory {
            Memory::new(self.buf.clone()).expect("sample story is valid")
        }

        fn grammar_of(&self) -> Result<Grammar, GrammarError> {
            Grammar::load(&self.mem())
        }

        /// The refusal, for the falsification cases. `Grammar` itself is not
        /// comparable, so the error is compared on its own.
        fn error(&self) -> Option<GrammarError> {
            self.grammar_of().err()
        }
    }

    // ── Infocom, fixed 8-byte lines ──────────────────────────────────────────

    /// "take OBJ", "take OBJ with OBJ", "open OBJ".
    fn infocom_fixed_story() -> Story {
        let mut s = Story::new(3, b"840726", false);
        // Infocom flags: VERB ($40) with DATA_FIRST = VERB_FIRST ($01), so the
        // verb number is the first data byte.
        s.dictionary(&[
            ("take", 0x41, [255, 0, 0]),
            ("grab", 0x41, [255, 0, 0]),
            ("open", 0x41, [254, 0, 0]),
            ("with", 0x08, [0, 0, 0]),
            ("lamp", 0x80, [0, 0, 0]),
        ]);
        let take = vec![
            2, // two lines
            1, 0x00, 0x00, 0, 0, 0, 0, 3, // "take OBJ", action 3
            2, 0x00, 0xF0, 0, 0, 0, 0, 4, // "take OBJ with OBJ", action 4
        ];
        let open = vec![1, 1, 0x00, 0x00, 0, 0, 0, 0, 5];
        let with_addr = s.dict_addrs[3];
        s.grammar(&[take, open], 6, 6, &[(with_addr, 0xF0)]);
        s
    }

    #[test]
    fn infocom_fixed_tables_decode() {
        let g = infocom_fixed_story().grammar_of().unwrap();
        assert_eq!(g.format(), GrammarFormat::InfocomFixed);
        assert_eq!(g.verbs().len(), 2);

        let take = g.verb_for_word("take").unwrap();
        assert_eq!(take.number, 255);
        assert_eq!(take.words, vec!["take".to_string(), "grab".to_string()]);
        assert_eq!(take.lines.len(), 2);
        assert_eq!(take.lines[0].describe("take"), "take noun");
        assert_eq!(take.lines[1].describe("take"), "take noun with noun");
        assert_eq!(take.lines[1].action, 4);

        // The shape query SQ-1041 needs: "with" is legal here, "from" is not.
        assert!(take.accepts(2, &["with"]));
        assert!(!take.accepts(2, &["from"]));
        assert!(take.accepts(1, &[]));
        assert!(!take.takes_bare());

        // A synonym resolves to the same verb.
        assert_eq!(g.verb_for_word("grab").unwrap().number, 255);
        assert!(g.is_verb("open"));
        assert!(!g.is_verb("lamp"));
        assert_eq!(g.prepositions(), ["with".to_string()]);
        assert!(g.is_preposition("with"));
    }

    // ── Infocom, variable 2/4/7-byte lines ───────────────────────────────────

    /// "hop", "pull up OBJ", "unlock OBJ with OBJ".
    fn infocom_variable_story() -> Story {
        let mut s = Story::new(3, b"871124", false);
        s.dictionary(&[
            ("hop", 0x41, [255, 0, 0]),
            ("pull", 0x41, [254, 0, 0]),
            ("unlock", 0x41, [253, 0, 0]),
            ("up", 0x08, [0, 0, 0]),
            ("with", 0x08, [0, 0, 0]),
        ]);
        // Byte 0 packs the object count into the top two bits and the first
        // preposition's low six bits into the rest.
        let hop = vec![1, 0x00, 7]; // 0 objects, action 7
        let pull = vec![1, 0x7C, 8, 0, 0]; // 1 object, prep $FC "up", action 8
        let unlock = vec![1, 0x80, 9, 0, 0, 0x3E, 0, 0]; // 2 objects, prep $FE
        let up = s.dict_addrs[3];
        let with = s.dict_addrs[4];
        s.grammar(&[hop, pull, unlock], 10, 10, &[(up, 0xFC), (with, 0xFE)]);
        s
    }

    #[test]
    fn infocom_variable_tables_decode() {
        let g = infocom_variable_story().grammar_of().unwrap();
        assert_eq!(g.format(), GrammarFormat::InfocomVariable);
        assert_eq!(g.verbs().len(), 3);

        assert_eq!(g.verb_for_word("hop").unwrap().lines[0].describe("hop"), "hop");
        assert!(g.verb_for_word("hop").unwrap().takes_bare());

        let pull = g.verb_for_word("pull").unwrap();
        assert_eq!(pull.lines[0].describe("pull"), "pull up noun");
        assert!(pull.accepts(1, &["up"]));

        let unlock = g.verb_for_word("unlock").unwrap();
        assert_eq!(unlock.lines[0].describe("unlock"), "unlock noun with noun");
        assert_eq!(unlock.max_nouns(), 2);
        assert_eq!(unlock.prepositions(), vec!["with"]);
    }

    // ── Inform 6, grammar version 1 ──────────────────────────────────────────

    /// One verb with three lines covering every GV1 token class.
    fn gv1_story() -> Story {
        let mut s = Story::new(5, b"961202", true);
        s.dictionary(&[
            ("take", 0x01, [255, 0, 0]),
            ("put", 0x01, [254, 0, 0]),
            ("in", 0x08, [0, 0, 0xFF]),
        ]);
        // 8 bytes each: [parameter count][6 tokens][action].
        // Four lines, so this verb's data is 33 bytes. The length matters:
        // `classify_inform6` only reaches its ambiguous branch when the data is
        // 1 mod 24, and the reference implementation misreads that branch (see
        // the note there), so a table meant as a shared oracle stays out of it.
        let take = vec![
            4, // four lines
            1, 0x00, 0, 0, 0, 0, 0, 20, // "take noun"
            2, 0x03, 0xFF, 0x01, 0, 0, 0, 21, // "take multiheld in held"
            2, 0x91, 0x35, 0, 0, 0, 0, 22, // ATTRIBUTE(17) then TEXT [parse 5]
            1, 0x14, 0, 0, 0, 0, 0, 24, // NOUN [parse 4]
        ];
        let put = vec![1, 1, 0x56, 0, 0, 0, 0, 0, 23]; // SCOPE [parse 6]
        let in_addr = s.dict_addrs[2];
        s.grammar(&[take, put], 25, 7, &[(in_addr, 0xFF)]);
        s
    }

    #[test]
    fn inform_gv1_tokens_decode() {
        let g = gv1_story().grammar_of().unwrap();
        assert_eq!(g.format(), GrammarFormat::InformGv1);

        let take = g.verb_for_word("take").unwrap();
        assert_eq!(take.lines.len(), 4);
        assert_eq!(take.lines[0].slots, vec![Slot::one(Token::Noun(NounKind::Noun))]);
        assert_eq!(
            take.lines[1].slots,
            vec![
                Slot::one(Token::Noun(NounKind::MultiHeld)),
                Slot::one(Token::Word("in".into())),
                Slot::one(Token::Noun(NounKind::Held)),
            ]
        );
        assert!(take.accepts(2, &["in"]));
        assert_eq!(
            take.lines[2].slots,
            vec![
                Slot::one(Token::Attribute(17)),
                Slot::one(Token::Routine(RoutineRef::Index(5))),
            ]
        );
        assert_eq!(
            take.lines[3].slots,
            vec![Slot::one(Token::FilteredNoun(RoutineRef::Index(4)))]
        );

        let put = g.verb_for_word("put").unwrap();
        assert_eq!(put.lines[0].slots, vec![Slot::one(Token::Scope(RoutineRef::Index(6)))]);
        assert_eq!(put.lines[0].action, 23);
    }

    // ── Inform 6, grammar version 2 ──────────────────────────────────────────

    /// Two lines: one with an alternative list and the reverse bit, one with
    /// a packed-address routine token.
    fn gv2_story() -> Story {
        let mut s = Story::new(5, b"981115", true);
        s.dictionary(&[
            ("get", 0x01, [255, 0, 0]),
            ("in", 0x08, [0, 0, 0]),
            ("into", 0x08, [0, 0, 0]),
        ]);
        let in_addr = s.dict_addrs[1] as u16;
        let into_addr = s.dict_addrs[2] as u16;
        let mut get: Vec<u8> = vec![2];
        // Line 1: action 30 with the $400 reverse bit; 'in' / 'into', then noun.
        get.extend_from_slice(&[0x04, 0x1E]);
        get.extend_from_slice(&[0x62, (in_addr >> 8) as u8, in_addr as u8]); // $$10 alt open
        get.extend_from_slice(&[0x52, (into_addr >> 8) as u8, into_addr as u8]); // $$01 continue
        get.extend_from_slice(&[0x01, 0x00, 0x00]); // elementary "noun"
        get.push(ENDIT);
        // Line 2: action 31, scope routine at packed $1234, attribute 9.
        get.extend_from_slice(&[0x00, 0x1F]);
        get.extend_from_slice(&[0x85, 0x12, 0x34]);
        get.extend_from_slice(&[0x04, 0x00, 0x09]);
        get.push(ENDIT);
        // A second verb: GV1 and GV2 are told apart by the length of the first
        // verb's data, which needs a second pointer to bound it.
        let drop = vec![1, 0x00, 0x28, 0x01, 0x00, 0x02, ENDIT];
        s.grammar(&[get, drop], 42, 0, &[]);
        s
    }

    #[test]
    fn inform_gv2_tokens_decode() {
        let g = gv2_story().grammar_of().unwrap();
        assert_eq!(g.format(), GrammarFormat::InformGv2);

        let get = g.verb_for_word("get").unwrap();
        assert_eq!(get.lines.len(), 2);

        let first = &get.lines[0];
        assert_eq!(first.action, 30);
        assert!(first.reverse);
        assert_eq!(first.slots.len(), 2);
        assert_eq!(
            first.slots[0].alternatives,
            vec![Token::Word("in".into()), Token::Word("into".into())]
        );
        assert!(first.slots[0].only().is_none());
        assert_eq!(first.slots[1].alternatives, vec![Token::Noun(NounKind::Noun)]);
        assert_eq!(first.noun_count(), 1);
        // Either spelling of the preposition satisfies the same slot.
        assert!(first.accepts(1, &["in"]));
        assert!(first.accepts(1, &["into"]));
        assert!(!first.accepts(1, &["on"]));

        let second = &get.lines[1];
        assert!(!second.reverse);
        assert_eq!(second.action, 31);
        assert_eq!(
            second.slots,
            vec![
                Slot::one(Token::Scope(RoutineRef::Packed(0x1234))),
                Slot::one(Token::Attribute(9)),
            ]
        );
    }

    // ── Parts of speech ──────────────────────────────────────────────────────

    #[test]
    fn roles_report_infocom_parts_of_speech() {
        let mut s = Story::new(3, b"840726", false);
        s.dictionary(&[
            ("take", 0x41, [255, 0, 0]),
            ("brass", 0x20, [0, 0, 0]),
            ("lamp", 0x80, [0, 0, 0]),
            ("with", 0x08, [0, 0, 0]),
            ("north", 0x04, [0, 0, 0]),
        ]);
        s.grammar(&[vec![1, 1, 0, 0, 0, 0, 0, 0, 1]], 2, 2, &[]);
        let g = s.grammar_of().unwrap();

        assert!(g.roles("take").unwrap().verb);
        assert!(g.roles("brass").unwrap().adjective);
        assert!(g.roles("lamp").unwrap().noun);
        assert!(!g.roles("lamp").unwrap().verb);
        assert!(g.roles("with").unwrap().preposition);
        assert!(g.roles("north").unwrap().special);
        assert_eq!(g.roles("nosuchword"), None);
    }

    #[test]
    fn roles_report_inform_parts_of_speech() {
        let mut s = gv2_story();
        // Add a meta verb and a plural noun to the dictionary.
        s.dictionary(&[
            ("get", 0x01, [255, 0, 0]),
            ("in", 0x08, [0, 0, 0]),
            ("into", 0x08, [0, 0, 0]),
            ("save", 0x03, [254, 0, 0]),
            ("coins", 0x84, [0, 0, 0]),
        ]);
        let g = s.grammar_of().unwrap();
        assert!(g.roles("save").unwrap().meta);
        assert!(g.roles("save").unwrap().verb);
        assert!(g.roles("coins").unwrap().plural);
        assert!(g.roles("coins").unwrap().noun);
        assert!(g.roles("in").unwrap().preposition);
        // Inform has no separate adjective bit.
        assert!(!g.roles("in").unwrap().adjective);
    }

    #[test]
    fn verbs_accepting_filters_by_shape() {
        let g = infocom_fixed_story().grammar_of().unwrap();
        let with: Vec<&str> =
            g.verbs_accepting(2, &["with"]).iter().filter_map(|v| v.word()).collect();
        assert_eq!(with, vec!["take"]);
        let bare: Vec<&str> =
            g.verbs_accepting(1, &[]).iter().filter_map(|v| v.word()).collect();
        assert_eq!(bare, vec!["take", "open"]);
        assert!(g.verbs_accepting(2, &["from"]).is_empty());
    }

    // ── Falsification ────────────────────────────────────────────────────────
    //
    // Each case corrupts exactly one byte of a table that parses cleanly above.
    // A grammar parser that answers plausibly here is worse than one that
    // fails: its consumer has no way to tell the difference.

    #[test]
    fn refuses_object_count_above_two() {
        let mut s = infocom_fixed_story();
        // First line's object count: 1 → 3.
        s.b(BASE + 4 + 1, 3);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_variable_line_with_three_objects() {
        let mut s = infocom_variable_story();
        // "pull"'s byte 0 is $7C; $FC sets the object count to 3, which
        // `verb_sizes` has no length for.
        s.b(BASE + 6 + 3 + 1, 0xFC);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_illegal_gv1_token() {
        let mut s = gv1_story();
        // First line's first token: 0 (noun) → 10, in the illegal 9–15 range.
        s.b(BASE + 4 + 2, 10);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_unknown_gv2_token_type() {
        let mut s = gv2_story();
        // The first token's type nibble: 2 (preposition) → 7, which no version
        // of the format defines.
        s.b(BASE + 4 + 3, 0x67);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_gv2_line_that_never_ends() {
        let mut s = gv2_story();
        // Overwrite the first line's ENDIT with another token type.
        s.b(BASE + 4 + 12, 0x01);
        assert!(matches!(
            s.error(),
            Some(GrammarError::BadSyntaxLine) | Some(GrammarError::Truncated)
        ));
    }

    #[test]
    fn refuses_backwards_verb_pointer() {
        let mut s = infocom_fixed_story();
        s.w(BASE, 0x0300); // points below the table it heads
        assert_eq!(s.error(), Some(GrammarError::BadVerbTable));
    }

    #[test]
    fn refuses_verb_entry_past_end_of_story() {
        let mut s = infocom_fixed_story();
        s.w(BASE + 2, 0x0FFF); // second verb's data starts at the last byte
        assert_eq!(s.error(), Some(GrammarError::Truncated));
    }

    #[test]
    fn reports_absent_rather_than_guessing_when_there_is_no_table() {
        let mut s = infocom_fixed_story();
        s.w(BASE, 0);
        assert_eq!(s.error(), Some(GrammarError::Absent));
    }

    #[test]
    fn refuses_absurd_line_count() {
        let mut s = infocom_fixed_story();
        // The first verb's line count: 2 → 200, far past what either format
        // allows and enough to walk the parser off the end of the tables.
        s.b(BASE + 4, 200);
        assert!(matches!(
            s.error(),
            Some(GrammarError::BadVerbTable) | Some(GrammarError::Truncated)
        ));
    }

    // ── Dialog: a compiler with no grammar table at all (SQ-1101) ────────────

    /// The signature, and that it is the SIGNATURE doing the work and not the
    /// bytes at static memory happening to look wrong.
    ///
    /// `infocom_fixed_story` is a table this module reads perfectly well. Stamp
    /// Dialog's three letters into $39..$3B and the same image answers `Absent`
    /// — because a Dialog story has no table, whatever its static memory holds.
    /// That is the whole point: `stories/ImpossibleStairs.z8` is caught today by
    /// a shape check, and the next Dialog story's wordmaps might not be.
    #[test]
    fn a_dialog_signature_means_absent_however_readable_the_bytes_are() {
        let mut s = infocom_fixed_story();
        assert!(s.grammar_of().is_ok(), "the unstamped image is a table we read");
        assert!(!is_dialog(&s.mem()));

        s.b(0x39, b'D');
        s.b(0x3A, b'i');
        s.b(0x3B, b'a');
        s.b(0x3C, b'0');
        s.b(0x3D, b'm');
        s.b(0x3E, b'0');
        s.b(0x3F, b'3');
        assert!(is_dialog(&s.mem()));
        assert_eq!(s.error(), Some(GrammarError::Absent));
    }

    /// Two letters of the signature are not the signature. A story whose $39
    /// and $3A happen to read `Di` is still read as whatever its header says.
    #[test]
    fn a_partial_signature_is_not_a_dialog_story() {
        let mut s = infocom_fixed_story();
        s.b(0x39, b'D');
        s.b(0x3A, b'i');
        assert!(!is_dialog(&s.mem()));
        assert!(s.grammar_of().is_ok());
    }

    /// Inform's stamp lives in $3C..$3F, one byte past Dialog's letters, and the
    /// two must not be confused: a GV2 story with `6.15` there still parses.
    #[test]
    fn informs_version_stamp_is_not_mistaken_for_dialogs() {
        let s = gv2_story();
        assert!(!is_dialog(&s.mem()));
        assert!(s.grammar_of().is_ok());
    }
}
