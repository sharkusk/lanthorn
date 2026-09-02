// Inform grammar (syntax) tables in a Glulx image — which verbs the story
// knows, and what sentence shapes each of them accepts.
//
// ── Where the format is specified ────────────────────────────────────────────
//
// The Glulx specification describes the virtual machine and says nothing about
// grammar: these tables are Inform's, not Glulx's. Two authoritative sources,
// both consulted directly rather than recalled:
//
//   * **"The Glulx Inform Technical Reference"**, Andrew Plotkin — §4 "The
//     Dictionary", §6 "Grammar Table", §7 "Actions Table". This is the Glulx
//     counterpart of the Inform Technical Manual's §8.6, written by the person
//     who designed the layout.
//     <https://eblong.com/zarf/glulx/Glulx-Inform-Tech.html>
//
//   * **The Inform 6 compiler itself** — `tables.c::construct_storyfile_g`,
//     which emits the tables in the order this module relies on, `verbs.c` for
//     the `/`-alternation bits ($20 on the token before a slash, $10 on the
//     token after one), and `text.c` for the dictionary record's shape and
//     `header.h` for the `*_DFLAG` flag bits.
//     <https://github.com/DavidKinder/Inform6>
//
// Cross-checked against `glulxdump` (Andrew Plotkin, shipped in the Glulxe
// source tree), which dumps the same tables when handed their address.
//
// ── The layout ───────────────────────────────────────────────────────────────
//
//   grammar table   long   number of verbs
//                   long   address of this verb's lines     × that many
//
//   per verb        byte   number of lines
//                   per line:
//                     short  action number
//                     byte   flags ($01 = swap noun and second)
//                     per token:
//                       byte  token type
//                       long  token data
//                     byte   ENDIT (15)
//
//   actions table   long   number of actions
//                   long   address of the action's routine  × that many
//
//   dictionary      long   number of words
//                   per word, one of two record shapes — see below
//
// ── The dictionary has two record shapes, and `DICT_CHAR_SIZE` picks ─────────
//
// A Glulx dictionary stores its characters as bytes or as four-byte Unicode
// values, chosen at compile time by `$DICT_CHAR_SIZE` (1 or 4; Inform 7's
// `Use dictionary with Unicode` sets 4). Both shapes are in the Glulx Inform
// Technical Reference §4, verbatim:
//
//     ...each word: {                  ...each word: {
//         byte: 60                         byte: 60
//                                          bytes[3]: unused (zero)
//         bytes[]: lower-case text,        words[]: Unicode text,
//             zero-padded (nine               zero-padded (nine words
//             bytes by default)               by default)
//         short: flags                     short: flags
//         short: verb number               short: verb number
//         short: unused (zero)             shorts[2]: unused (zero)
//     }                                }
//
// and the arithmetic is `Inform6/inform.c`, which is where the compiler turns
// `DICT_CHAR_SIZE` into the two numbers this module needs:
//
//     DICT_WORD_BYTES = DICT_WORD_SIZE*DICT_CHAR_SIZE;
//     if (DICT_CHAR_SIZE == 1) {
//         DICT_ENTRY_BYTE_LENGTH = (7+DICT_WORD_BYTES);
//         DICT_ENTRY_FLAG_POS = (1+DICT_WORD_BYTES);
//     }
//     else {
//         DICT_ENTRY_BYTE_LENGTH = (12+DICT_WORD_BYTES);
//         DICT_ENTRY_FLAG_POS = (4+DICT_WORD_BYTES);
//     }
//
// So with W = `DICT_WORD_SIZE`, a record is:
//
//   | field            | `DICT_CHAR_SIZE=1` | `DICT_CHAR_SIZE=4`         |
//   |------------------|--------------------|----------------------------|
//   | `$60` type tag   | byte at +0         | byte at +0, then 3 zeroes  |
//   | text             | W bytes at +1      | W big-endian longs at +4   |
//   | flags            | short at 1+W       | short at 4+4W              |
//   | verb number      | short at 3+W       | short at 6+4W              |
//   | adjective number | short at 5+W       | short at 8+4W              |
//   | record length    | 7+W                | 12+4W                      |
//
// The reference adds that "in this form, the dictionary entry size is a
// multiple of four. The compiler also takes care that a Unicode dictionary will
// start at a word-aligned address" — which is what [`dict_char_size`] leans on,
// alongside the three zero bytes after the tag.
//
// ── The verb number is inverted, and from *which* base is a version fact ─────
//
// A dictionary record does not hold a verb's grammar-table index; it holds that
// index subtracted from a base, so that verbs count DOWN from the top of the
// field (`text.c`: "The verb number is inverted (we count down from $FF/$FFFF)
// and stored in #dict_par2"). On the Z-machine the base has always been $FF,
// because the field is one byte. Glulx widened the field to two bytes — but for
// its first decade the compiler kept writing the Z-machine's $FF into it, so
// only the low half was ever used and no Glulx story could hold more than 255
// verbs. `Inform6/verbs.c` through **v6.31**:
//
//     dictionary_add(English_verbs_given[i], …, 0xff-Inform_verb, 0);
//     dictionary_set_verb_number(token_text, 0xff-no_Inform_verbs);
//
// **Inform 6.32 widened it**, and the same line reads:
//
//     (glulx_mode)?(0xffff-Inform_verb):(0xff-Inform_verb), 0);
//
// which is where it still is on master, moved into
// `text.c::dictionary_set_verb_number`:
//
//     int flag2 = ((glulx_mode)?(0xffff-infverb):(0xff-infverb));
//
// So a Glulx dictionary uses one of exactly two bases, and which one is a
// property of the compiler that built the file rather than of the format. See
// [`verb_number_base`] for how this module decides between them — by checking
// both against the grammar table's own verb count, which is decidable rather
// than guessed, instead of trusting the "6.21"/"6.33" string Inform stamps into
// its header block.
//
// Plotkin: "This is nearly identical to the grammar version 2 format in
// Z-machine Inform. The only differences are that the token data is 4 bytes
// long, and the switch flag is no longer stuck in the action number." Token
// type bytes carry the same three fields as GV2 — top two bits the data kind,
// next two the `/`-alternation state, bottom four the type.
//
// ── The hard part is not the format; it is finding the tables ────────────────
//
// **A Glulx image records the grammar table's address nowhere.** On the
// Z-machine, header word $0E points at it (`zvm::grammar` relies on that). The
// Glulx header names RAMSTART, EXTSTART, ENDMEM, the start function and the
// string-decoding table, and nothing else; Inform's own 24-byte block after it
// holds a layout tag, two version strings, a release number and a serial, and
// no table addresses at all (`Inform6/src/files.c`, `GLULX_STATIC_ROM_SIZE`).
//
// This is not an oversight we can route around: `glulxdump` — written by the
// designer of both Glulx and this layout — requires the address on the command
// line (`-g <addr>`), and its header comment says so outright: "This whole
// situation could be improved by adding a 'layout convention' field, at the
// start of ROM, which could contain compiler-specific information about how to
// decompile the file. Maybe someday."
//
// So the tables are *derived*, by a chain that is verified end to end rather
// than guessed. Inform emits grammar, actions and dictionary contiguously and
// in that order, and each is self-describing:
//
//   1. **The dictionary** is found first, because it has the strongest
//      signature in the image: a run of records at a constant stride, each
//      beginning with the byte $60, whose length equals the count word
//      immediately before the run. Nothing else in memory looks like that.
//   2. **The actions table** ends exactly where the dictionary begins (Inform
//      inserts up to three bytes of alignment padding, and only for Unicode
//      dictionaries). Its own count word must agree with its length, and every
//      entry must be a plausible code address — below RAMSTART, since Glulx
//      Inform keeps all code and strings in ROM.
//   3. **The grammar table** ends exactly where the actions table begins, and
//      its first verb pointer must equal `base + 4 + 4 * verb_count` exactly.
//      Walking every verb, every line and every token of it must land on the
//      actions table's first byte and not one byte elsewhere.
//
// A candidate that satisfies all three is not a guess. The last step is the one
// that does the work, and it is worth being precise about how much: across the
// 22 Glulx stories in the local corpus, **889 byte offsets satisfy the
// pointer-array precondition alone** — 279 in one game — and **exactly 22
// survive the walk**, one per story. The walk is what discriminates; scanning
// backwards from the actions table merely means the right answer is usually the
// first one tried. Where nothing survives, this module refuses — see
// [`GrammarError`].

use std::collections::BTreeMap;

use crate::memory::Memory;

// The shape of the ANSWER is shared with `zvm::grammar` and lives in the
// `grammar-model` crate (SQ-1103), as does the CONTAINER it arrives in
// (`Vocabulary`, SQ-1108); the READERS share nothing, for the reasons set out
// at the bottom of this file. Re-exported here so `gvm::grammar::Token` still
// names the type. What stayed behind is what is about this FORMAT rather than
// about the answer: `Tables` (these addresses are derived, so where they were
// found is part of the answer here and there is no Z-machine counterpart) and
// `GrammarError` (whose refusals belong to a locator that has to close a
// chain).
pub use grammar_model::{NounKind, RoutineRef, Slot, SyntaxLine, Token, Verb, WordRoles};

use grammar_model::Vocabulary;

/// Inform's `*_DFLAG` dictionary flag bits (`Inform6/src/header.h`).
const VERB_DFLAG: u16 = 1;
const META_DFLAG: u16 = 2;
const PLURAL_DFLAG: u16 = 4;
const PREP_DFLAG: u16 = 8;
const SING_DFLAG: u16 = 16;
const TRUNC_DFLAG: u16 = 64;
const NOUN_DFLAG: u16 = 128;

/// The type tag every dictionary record begins with (Glulx Inform Tech. Ref. §4).
const DICT_TAG: u8 = 0x60;
/// End of a grammar line (Glulx Inform Tech. Ref. §6).
const ENDIT: u8 = 15;

/// Smallest dictionary this module will accept as a positive identification.
/// Real games run to hundreds of words; a shorter run is noise.
const MIN_DICT_WORDS: u32 = 16;
/// `DICT_ENTRY_BYTE_LENGTH`: `7 + DICT_WORD_SIZE` for a byte-valued dictionary,
/// `12 + 4*DICT_WORD_SIZE` for a Unicode one. Inform's default word size is 9
/// (stride 16, or 48 Unicode); the corpus also contains 10 and 12 (17 and 19).
const DICT_STRIDE_RANGE: std::ops::RangeInclusive<u32> = 8..=80;
/// Sanity ceilings. The largest table seen in the corpus is Cragne Manor's 368
/// verbs and 375 actions; these are far above that and far below noise.
const MAX_VERBS: u32 = 20_000;
const MAX_ACTIONS: u32 = 8_000;

/// The base a Glulx dictionary's verb numbers count down from, since Inform
/// 6.32 (`Inform6/text.c::dictionary_set_verb_number`).
const VERB_BASE_WIDE: u32 = 0xFFFF;
/// The base Inform used in Glulx mode through v6.31 — the Z-machine's, written
/// into the low half of the two-byte field (`Inform6/verbs.c`).
const VERB_BASE_NARROW: u32 = 0xFF;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Why a Glulx story's grammar could not be read.
///
/// Refusing is the contract. Because the tables are located rather than looked
/// up, a reader that answered anyway would hand its consumer a confident
/// reading of the wrong bytes, and nothing downstream could tell.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GrammarError {
    /// No Inform tables in this image. A Glulx file need not be an Inform
    /// game at all, and even an Inform one need not have a parser —
    /// `glulxercise.ulx` is a VM conformance suite with a dictionary and no
    /// grammar.
    Absent,
    /// The dictionary was found but no actions table ends where it begins, or
    /// no grammar table ends where that begins. The chain could not be closed,
    /// so no address here is trustworthy.
    TablesNotFound,
    /// A table address or entry ran past the end of memory.
    Truncated,
    /// A grammar line held a value the format forbids — an unknown token type,
    /// an elementary token above 9, or a line that never reached its ENDIT.
    BadSyntaxLine,
    // There was a `UnicodeDictionary` refusal here, for `$DICT_CHAR_SIZE=4`.
    // Both record shapes are read now (SQ-1231), so nothing can produce it and
    // it is gone rather than left unreachable.
}

// ── The one value type that is this reader's own ─────────────────────────────

/// Where the three Inform tables were found, and how big each is.
///
/// Worth having on its own: no Glulx tool can be told "dump this game's
/// grammar" without these numbers, so they are the input `glulxdump -g` wants
/// and the thing to quote in any finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Tables {
    /// Address of the grammar table's verb count.
    pub grammar: u32,
    /// Number of verbs.
    pub verb_count: u32,
    /// Address of the actions table's count.
    pub actions: u32,
    /// Number of actions.
    pub action_count: u32,
    /// Address of the dictionary's word count.
    pub dictionary: u32,
    /// Number of dictionary words.
    pub word_count: u32,
    /// Bytes per dictionary record (`DICT_ENTRY_BYTE_LENGTH`).
    pub dict_stride: u32,
    /// `DICT_WORD_SIZE` — CHARACTERS of text per record, which is not the
    /// record's text in bytes unless [`Tables::dict_char_size`] is 1.
    pub dict_word_size: u32,
    /// `DICT_CHAR_SIZE` — 1 for a byte-valued dictionary, 4 for a Unicode one.
    /// See the module header for the two record shapes it selects between, and
    /// [`dict_char_size`] for how this module decides which is in front of it.
    pub dict_char_size: u32,
}

/// A Glulx story's grammar: which words are verbs, and what each verb accepts.
///
/// Self-contained once loaded — no `&Memory` is needed to query it, so it can
/// be cached beside a session or handed to another thread.
#[derive(Debug, Clone)]
pub struct Grammar {
    /// This reader's own facts: where the tables were found, and the base the
    /// dictionary counts verb numbers down from.
    tables: Tables,
    verb_base: u32,
    /// Everything a Z-machine grammar also has — the verbs, the spelling
    /// index, the prepositions, the dictionary roles and the action routines.
    /// Shared with `zvm::grammar::Grammar`, which composes the same value
    /// (SQ-1108). The accessors below delegate to it one for one, so this
    /// type's public API is unchanged and reads on its own.
    vocab: Vocabulary,
}

impl Grammar {
    /// Locate and read the story's Inform tables.
    pub fn load(mem: &Memory) -> Result<Grammar, GrammarError> {
        let tables = locate(mem)?;
        let words = read_dictionary(mem, &tables)?;

        let verb_base = verb_number_base(&words, tables.verb_count);

        let mut roles = BTreeMap::new();
        let mut spellings: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for w in &words {
            roles.insert(w.text.clone(), w.roles);
            if w.roles.verb {
                let Some(n) = verb_base.checked_sub(w.verb_field) else {
                    continue;
                };
                spellings.entry(n).or_default().push(w.text.clone());
            }
        }

        let mut verbs = Vec::with_capacity(tables.verb_count as usize);
        for i in 0..tables.verb_count {
            let address = read32(mem, tables.grammar + 4 + i * 4)?;
            let lines = read_verb_lines(mem, address, &words)?.1;
            verbs.push(Verb::new(i, address, spellings.remove(&i).unwrap_or_default(), lines));
        }

        let mut action_routines = Vec::with_capacity(tables.action_count as usize);
        for i in 0..tables.action_count {
            action_routines.push(read32(mem, tables.actions + 4 + i * 4)?);
        }

        Ok(Grammar { tables, verb_base, vocab: Vocabulary::new(verbs, roles, action_routines) })
    }

    /// The base this story's dictionary counts verb numbers down from — $FFFF
    /// for Inform 6.32 and later, $FF for anything earlier. Not a table
    /// address, so it is not part of [`Tables`]; it is a reading of the
    /// dictionary's contents, and worth quoting in a finding because a story
    /// read with the wrong one has verbs and no verb WORDS.
    pub fn verb_number_base(&self) -> u32 {
        self.verb_base
    }

    /// Where the tables were found. Also reachable without reading them, via
    /// [`locate`].
    pub fn tables(&self) -> Tables {
        self.tables
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

    /// Every literal word the grammar names, deduplicated and sorted.
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
    /// and nouns and buzzwords alike. `zvm::grammar::Grammar::words` answers
    /// the same question about a Z-machine story (SQ-1103); the words of one
    /// syntax LINE are [`SyntaxLine::literals`].
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.vocab.words()
    }

    /// Addresses of the action routines, indexed by action number.
    pub fn action_routines(&self) -> &[u32] {
        self.vocab.action_routines()
    }

    /// Every verb with a line matching `nouns` noun phrases and exactly `words`
    /// as its literal words.
    pub fn verbs_accepting(&self, nouns: usize, words: &[&str]) -> Vec<&Verb> {
        self.vocab.verbs_accepting(nouns, words)
    }
}

// ── Locating the tables ──────────────────────────────────────────────────────

/// Find the grammar, actions and dictionary tables without reading them.
///
/// The chain is described at the top of this file. Every step is checked
/// against the next, so a returned `Tables` is a reading no other arrangement
/// of the image's bytes can produce.
pub fn locate(mem: &Memory) -> Result<Tables, GrammarError> {
    let ram = mem.ramstart();
    let lim = mem.extstart();
    let (dictionary, word_count, dict_stride) = find_dictionary(mem, ram, lim)?;

    let char_size = dict_char_size(mem, dictionary, word_count, dict_stride);
    // `DICT_ENTRY_BYTE_LENGTH` inverted (`Inform6/inform.c`, module header).
    let dict_word_size = if char_size == 4 { (dict_stride - 12) / 4 } else { dict_stride - 7 };

    for (actions, action_count) in action_candidates(mem, dictionary, ram) {
        if let Some((grammar, verb_count)) = find_grammar(mem, ram, lim, actions) {
            return Ok(Tables {
                grammar,
                verb_count,
                actions,
                action_count,
                dictionary,
                word_count,
                dict_stride,
                dict_word_size,
                dict_char_size: char_size,
            });
        }
    }
    Err(GrammarError::TablesNotFound)
}

/// Whether this dictionary stores its characters as bytes or as four-byte
/// Unicode values — `DICT_CHAR_SIZE`, which the image records nowhere.
///
/// Decidable rather than guessed, the same way [`verb_number_base`] is. A
/// Unicode record opens `60 00 00 00` — the tag padded out to a long, "bytes[3]:
/// unused (zero)" — and then holds every character as a big-endian long, so in
/// a Unicode dictionary **the byte after the tag is zero in every record**. In a
/// byte-valued dictionary that byte is the word's first character, and it is
/// zero only for the EMPTY word, of which a dictionary sorted by Latin-1 holds
/// at most one. So the test is over the whole table rather than a sample of it,
/// and no byte dictionary with two distinct words can satisfy it.
///
/// **That distinction is the whole of SQ-1231.** This test used to look at the
/// first eight records and refuse the story if ANY of them had a zero there —
/// and `stories/CoS.blb` (City of Secrets, Inform 6.21, serial 030624) opens
/// with the empty word, flagged `VERB|META|TRUNC`, which its menu system
/// defines. An entirely ordinary 3,551-word byte dictionary, stride 16, was
/// read as Unicode and the story refused outright: no verb column, no word
/// reveal, no guidance offer, for the whole game. It is the only story in the
/// 41-Glulx corpus with an empty dictionary word, and the only one that failed.
///
/// The alignment guards come from the reference's own promise about the shape:
/// "the dictionary entry size is a multiple of four. The compiler also takes
/// care that a Unicode dictionary will start at a word-aligned address."
fn dict_char_size(mem: &Memory, dictionary: u32, word_count: u32, stride: u32) -> u32 {
    // `12 + 4*DICT_WORD_SIZE`, so a Unicode record is a multiple of four bytes
    // and holds at least one character.
    if stride < 16 || !stride.is_multiple_of(4) || !dictionary.is_multiple_of(4) {
        return 1;
    }
    let padded_tag = (0..word_count).all(|i| {
        let e = dictionary + 4 + i * stride;
        byte(mem, e + 1) == 0 && byte(mem, e + 2) == 0 && byte(mem, e + 3) == 0
    });
    if padded_tag { 4 } else { 1 }
}

/// The longest run of `$60`-tagged records at a constant stride whose length
/// matches the count word immediately before it.
fn find_dictionary(mem: &Memory, ram: u32, lim: u32) -> Result<(u32, u32, u32), GrammarError> {
    let mut best: Option<(u32, u32, u32)> = None;
    let mut p = ram;
    while p < lim {
        if byte(mem, p) != DICT_TAG {
            p += 1;
            continue;
        }
        for stride in DICT_STRIDE_RANGE {
            // Only start a chain at its head, so each run is measured once.
            if p >= ram + stride && byte(mem, p - stride) == DICT_TAG {
                continue;
            }
            let mut n = 1u32;
            let mut q = p + stride;
            while q < lim && byte(mem, q) == DICT_TAG {
                n += 1;
                q += stride;
            }
            if n < MIN_DICT_WORDS || p < ram + 4 {
                continue;
            }
            if read32(mem, p - 4).ok() == Some(n) && best.is_none_or(|(_, bn, _)| n > bn) {
                best = Some((p - 4, n, stride));
            }
        }
        p += 1;
    }
    best.ok_or(GrammarError::Absent)
}

/// Actions tables that could end where the dictionary begins.
///
/// Inform pads to a four-byte boundary before a Unicode dictionary and not
/// otherwise, so up to three bytes may sit between the two. Every entry must
/// look like a code address — Glulx Inform keeps all code and strings in ROM,
/// below RAMSTART.
fn action_candidates(mem: &Memory, dictionary: u32, ram: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for pad in 0..4u32 {
        let Some(end) = dictionary.checked_sub(pad) else { continue };
        for k in 1..MAX_ACTIONS {
            let Some(a) = end.checked_sub(4 + 4 * k) else { break };
            if a < ram {
                break;
            }
            if read32(mem, a).ok() != Some(k) {
                continue;
            }
            let plausible = (0..k).all(|i| {
                let v = read32(mem, a + 4 + i * 4).unwrap_or(0);
                (60..ram).contains(&v)
            });
            if plausible {
                out.push((a, k));
            }
        }
    }
    out
}

/// The grammar table that ends exactly at `actions`, walking every verb, every
/// line and every token to prove it.
fn find_grammar(mem: &Memory, ram: u32, lim: u32, actions: u32) -> Option<(u32, u32)> {
    let mut base = actions;
    while base > ram {
        base -= 1;
        let Ok(n) = read32(mem, base) else { continue };
        if !(1..=MAX_VERBS).contains(&n) {
            continue;
        }
        let first = base.checked_add(4 + 4 * n)?;
        if first >= actions || read32(mem, base + 4).ok() != Some(first) {
            continue;
        }
        if walk_grammar(mem, base, n, lim, actions) {
            return Some((base, n));
        }
    }
    None
}

/// True if the whole table reads cleanly and ends on `actions`'s first byte.
fn walk_grammar(mem: &Memory, base: u32, n: u32, lim: u32, actions: u32) -> bool {
    let mut cur = base + 4 + 4 * n;
    for i in 0..n {
        if read32(mem, base + 4 + i * 4).ok() != Some(cur) {
            return false;
        }
        match skip_verb(mem, cur, lim, actions) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    cur == actions
}

/// Advance past one verb's line block without decoding it.
fn skip_verb(mem: &Memory, at: u32, lim: u32, ceiling: u32) -> Option<u32> {
    let mut cur = at;
    if cur >= lim {
        return None;
    }
    let lines = byte(mem, cur);
    cur += 1;
    for _ in 0..lines {
        cur = cur.checked_add(3)?;
        loop {
            if cur >= lim || cur > ceiling {
                return None;
            }
            let t = byte(mem, cur);
            cur += 1;
            if t == ENDIT {
                break;
            }
            cur = cur.checked_add(4)?;
        }
    }
    (cur <= ceiling).then_some(cur)
}

// ── Reading the tables ───────────────────────────────────────────────────────

/// One dictionary record, decoded far enough to answer the questions above.
struct DictWord {
    address: u32,
    text: String,
    roles: WordRoles,
    /// `#dict_par2` exactly as stored — the verb's grammar-table index
    /// subtracted from a base this record cannot name. See
    /// [`verb_number_base`].
    verb_field: u32,
}

/// Which base this file's verb numbers count down from — [`VERB_BASE_WIDE`] or
/// [`VERB_BASE_NARROW`]; see the module header for why there are two.
///
/// Decided from the file rather than from the compiler-version string Inform
/// stamps into its header block, because it is *decidable*: the grammar table
/// states how many verbs it holds, so a base is right only if every
/// verb-flagged dictionary record names one of them. The two acceptance windows
/// cannot overlap — [`MAX_VERBS`] is far below `VERB_BASE_WIDE -
/// VERB_BASE_NARROW` — so at most one base can pass, and the answer is a check
/// rather than a preference.
///
/// A file where neither passes falls back to the modern base, which is what
/// this module read before it knew about the other one: those records then name
/// no verb and attach no spelling, exactly as they did. Refusing outright would
/// turn a story whose dictionary merely holds one odd record into a story with
/// no grammar at all, and the tables here have already been verified end to end
/// by [`locate`] — an unreadable verb number falsifies nothing about them.
fn verb_number_base(words: &[DictWord], verb_count: u32) -> u32 {
    let fits = |base: u32| {
        words
            .iter()
            .filter(|w| w.roles.verb)
            .all(|w| base.checked_sub(w.verb_field).is_some_and(|n| n < verb_count))
    };
    if fits(VERB_BASE_WIDE) || !fits(VERB_BASE_NARROW) {
        VERB_BASE_WIDE
    } else {
        VERB_BASE_NARROW
    }
}

fn read_dictionary(mem: &Memory, t: &Tables) -> Result<Vec<DictWord>, GrammarError> {
    let w = t.dict_word_size;
    // `DICT_ENTRY_FLAG_POS` (`Inform6/inform.c`): the text runs from just past
    // the tag — one byte, or four once it is padded out to a long — and the
    // three shorts follow it.
    let text_at = if t.dict_char_size == 4 { 4 } else { 1 };
    let flag_pos = text_at + w * t.dict_char_size;
    let mut out = Vec::with_capacity(t.word_count as usize);
    for i in 0..t.word_count {
        let entry = t.dictionary + 4 + i * t.dict_stride;
        let mut text = String::new();
        for j in 0..w {
            // Records are lower-cased by Inform, and a byte-valued one is
            // Latin-1 — which `char::from_u32` agrees with over 0..=$FF, so the
            // two shapes decode through one line rather than two.
            let c = if t.dict_char_size == 4 {
                read32(mem, entry + text_at + j * 4)?
            } else {
                u32::from(byte(mem, entry + text_at + j))
            };
            if c == 0 {
                break;
            }
            // A four-byte record can hold a value that is not a Unicode scalar
            // (a surrogate, or above $10FFFF). Nothing a player can type, so it
            // becomes the replacement character rather than failing the story.
            text.push(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        let flags = read16(mem, entry + flag_pos)?;
        // `WordRoles` is shared with `zvm` and `#[non_exhaustive]`, so it is
        // built from the flag field and then filled in. The two bits left false
        // are the Infocom family's — `adjective` and `special` — which no
        // Inform back-end has.
        let mut roles = WordRoles::from_raw(flags);
        roles.verb = flags & VERB_DFLAG != 0;
        roles.meta = flags & META_DFLAG != 0;
        roles.plural = flags & PLURAL_DFLAG != 0;
        roles.preposition = flags & PREP_DFLAG != 0;
        roles.singular = flags & SING_DFLAG != 0;
        roles.truncated = flags & TRUNC_DFLAG != 0;
        roles.noun = flags & NOUN_DFLAG != 0;
        // Stored INVERTED, and from a base this record cannot state; the
        // subtraction happens in `load`, once the whole dictionary is in hand
        // and `verb_number_base` can decide which base this file was built
        // with.
        let verb_field = read16(mem, entry + flag_pos + 2)? as u32;
        out.push(DictWord {
            address: entry,
            text,
            roles,
            verb_field,
        });
    }
    Ok(out)
}

/// Read one verb's lines. Returns the address just past the block alongside
/// them, so a caller can check the block ended where it should.
fn read_verb_lines(
    mem: &Memory,
    at: u32,
    words: &[DictWord],
) -> Result<(u32, Vec<SyntaxLine>), GrammarError> {
    let mut cur = at;
    let count = byte(mem, cur);
    cur += 1;
    let mut lines = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let action = read16(mem, cur)?;
        let flags = byte(mem, cur + 2);
        cur += 3;
        let mut slots: Vec<Slot> = Vec::new();
        loop {
            let ty = byte(mem, cur);
            cur += 1;
            if ty == ENDIT {
                break;
            }
            let data = read32(mem, cur)?;
            cur += 4;
            let token = decode_token(ty & 0x0F, data, words)?;
            // Bits 4-5 are the `/`-alternation state: Inform sets $20 on a
            // token followed by a slash and $10 on one preceded by a slash
            // (`verbs.c`), so bit 4 means "continues the slot before me".
            match ((ty >> 4) & 0x01 != 0, slots.last_mut()) {
                (true, Some(last)) => last.alternatives.push(token),
                _ => slots.push(Slot::one(token)),
            }
            if slots.len() > 64 {
                return Err(GrammarError::BadSyntaxLine);
            }
        }
        lines.push(SyntaxLine::new(action, flags & 0x01 != 0, slots));
    }
    Ok((cur, lines))
}

fn decode_token(ty: u8, data: u32, words: &[DictWord]) -> Result<Token, GrammarError> {
    Ok(match ty {
        1 => Token::Noun(NounKind::from_elementary(data).ok_or(GrammarError::BadSyntaxLine)?),
        2 => Token::Word(
            words
                .iter()
                .find(|w| w.address == data)
                .map(|w| w.text.clone())
                .ok_or(GrammarError::BadSyntaxLine)?,
        ),
        3 => Token::FilteredNoun(RoutineRef::Address(data)),
        4 => Token::Attribute(data),
        5 => Token::Scope(RoutineRef::Address(data)),
        6 => Token::Routine(RoutineRef::Address(data)),
        _ => return Err(GrammarError::BadSyntaxLine),
    })
}

// ── Bounds-checked reads ─────────────────────────────────────────────────────

fn byte(mem: &Memory, addr: u32) -> u8 {
    mem.read8(addr).unwrap_or(0) as u8
}

fn read16(mem: &Memory, addr: u32) -> Result<u16, GrammarError> {
    mem.read16(addr).map(|v| v as u16).ok_or(GrammarError::Truncated)
}

fn read32(mem: &Memory, addr: u32) -> Result<u32, GrammarError> {
    mem.read32(addr).ok_or(GrammarError::Truncated)
}

// ── Why the reader is gvm's own and the answer is not ───────────────────────
//
// The two engines' grammar READERS have almost nothing in common. The formats
// agree on the token type numbering and on nothing else: the Z-machine's table
// is at a header-named address and this one is not recorded at all; its verb
// numbers count down from 255 and these count up from zero; its line header is
// two bytes with the reverse flag packed into the action and this one is three
// with a flags byte; its tokens are 1+2 bytes and these are 1+4; its dictionary
// is Z-encoded with a game-chosen record length and this one is plain bytes
// behind a type tag. `zvm::grammar` additionally carries four Infocom-era
// shapes that have no Glulx counterpart at all. A trait over "read a byte / read
// a word at an address" would abstract a handful of lines out of several hundred
// while forcing two zero-dependency crates to name a shared vocabulary, so that
// decision was taken on the evidence and stands.
//
// What the two DO share is the shape of the ANSWER — `Token`, `NounKind`,
// `Slot`, `SyntaxLine`, `Verb`, `WordRoles`, the elementary-token numbering and
// the six token types — and since SQ-1103 they share it as one thing: the
// zero-dependency `grammar-model` crate, re-exported at the top of this file.
// It was two near-identical copies for exactly as long as it took SQ-1040's
// public API to settle, because lifting it out rewrites that API and was worth
// doing once, on purpose, rather than as a side effect of adding this reader.
//
// The join is not lossy in either direction. Three facts stayed with their
// engines because they are about the FORMAT rather than the answer — this
// module's `Tables` and `locate` (the Z-machine reads its address out of the
// header and has nothing to locate), `zvm`'s `GrammarFormat` (five table shapes,
// of which Glulx has one), and each engine's `GrammarError` (whose variants are
// its own refusals down to the last one). Everything else is one type.

#[cfg(test)]
mod tests {
    use super::*;

    const RAM: u32 = 0x0100;
    // Room for the Unicode story below, whose twenty records are 48 bytes each
    // where the byte-valued ones are 16.
    const EXT: u32 = 0x0800;

    /// A hand-built Glulx image with the three Inform tables laid out the way
    /// `Inform6/src/tables.c::construct_storyfile_g` lays them out: grammar,
    /// then actions, then dictionary, contiguous and in that order.
    struct Story {
        buf: Vec<u8>,
    }

    impl Story {
        fn new() -> Story {
            let mut buf = vec![0u8; EXT as usize];
            buf[0..4].copy_from_slice(b"Glul");
            let w = |buf: &mut Vec<u8>, at: usize, v: u32| {
                buf[at..at + 4].copy_from_slice(&v.to_be_bytes());
            };
            w(&mut buf, 0x04, 0x0003_0102); // version 3.1.2
            w(&mut buf, 0x08, RAM);
            w(&mut buf, 0x0C, EXT);
            w(&mut buf, 0x10, EXT);
            w(&mut buf, 0x14, 0x1000);
            w(&mut buf, 0x18, 0x40);
            Story { buf }
        }

        fn b(&mut self, at: u32, v: u8) {
            self.buf[at as usize] = v;
        }

        fn w32(&mut self, at: u32, v: u32) {
            self.buf[at as usize..at as usize + 4].copy_from_slice(&v.to_be_bytes());
        }

        fn w16(&mut self, at: u32, v: u16) {
            self.buf[at as usize..at as usize + 2].copy_from_slice(&v.to_be_bytes());
        }

        /// Write a dictionary of `(word, flags, verb_index)` at `at`, with nine
        /// characters per record (Inform's default) in whichever record shape
        /// `char_size` names — the two the module header tabulates, laid out
        /// from `DICT_ENTRY_BYTE_LENGTH` and `DICT_ENTRY_FLAG_POS` rather than
        /// from this reader's arithmetic. Verb numbers are inverted against
        /// `verb_base`, the way the compiler inverts them.
        fn dictionary(
            &mut self,
            at: u32,
            entries: &[(&str, u16, u32)],
            verb_base: u32,
            char_size: u32,
        ) {
            let stride = dict_stride(char_size);
            let text_at = if char_size == 4 { 4 } else { 1 };
            let flag_pos = text_at + WORD_SIZE * char_size;
            self.w32(at, entries.len() as u32);
            for (i, (word, flags, verb)) in entries.iter().enumerate() {
                let e = at + 4 + i as u32 * stride;
                self.b(e, DICT_TAG);
                for (j, c) in word.chars().take(WORD_SIZE as usize).enumerate() {
                    let j = j as u32;
                    match char_size {
                        4 => self.w32(e + text_at + j * 4, c as u32),
                        _ => self.b(e + text_at + j, c as u32 as u8),
                    }
                }
                self.w16(e + flag_pos, *flags);
                self.w16(e + flag_pos + 2, (verb_base - *verb) as u16);
            }
        }

        fn mem(&self) -> Memory {
            Memory::new(self.buf.clone()).expect("synthetic image is valid")
        }

        fn grammar(&self) -> Result<Grammar, GrammarError> {
            Grammar::load(&self.mem())
        }

        fn error(&self) -> Option<GrammarError> {
            self.grammar().err()
        }
    }

    /// `DICT_WORD_SIZE` for every synthetic story here — Inform's own default.
    const WORD_SIZE: u32 = 9;

    /// `DICT_ENTRY_BYTE_LENGTH` (`Inform6/inform.c`): 16 bytes per record for a
    /// byte-valued dictionary, 48 for a Unicode one.
    fn dict_stride(char_size: u32) -> u32 {
        if char_size == 4 {
            12 + 4 * WORD_SIZE
        } else {
            7 + WORD_SIZE
        }
    }

    /// Two verbs. "take" has `take noun` and `take noun in / into noun`
    /// (reversed); "look" has a bare line and one with an attribute token.
    ///
    /// Layout is computed rather than hard-coded, because the locator's whole
    /// contract is that the three tables abut exactly.
    fn story() -> Story {
        story_shaped(VERB_BASE_WIDE, 1)
    }

    /// The same story, with its dictionary's verb numbers counted down from
    /// `verb_base` — $FFFF the way Inform 6.32 and later write a Glulx
    /// dictionary, $FF the way every release before it did. See the module
    /// header; the two are otherwise byte-identical files.
    fn story_numbered(verb_base: u32) -> Story {
        story_shaped(verb_base, 1)
    }

    /// The same story again, its dictionary written in the `$DICT_CHAR_SIZE=4`
    /// record shape: the tag padded out to a long, every character a big-endian
    /// long, the three shorts moved to `4 + 4*DICT_WORD_SIZE`, and the table
    /// word-aligned behind two bytes of padding the compiler inserts for
    /// exactly this reason. Three of the filler nouns carry what only this
    /// shape can hold — see [`reads_a_unicode_dictionary`].
    fn story_unicode() -> Story {
        story_shaped(VERB_BASE_WIDE, 4)
    }

    fn story_shaped(verb_base: u32, char_size: u32) -> Story {
        let mut s = Story::new();
        let g = RAM; // grammar table
        let verbs = 2u32;
        let v0 = g + 4 + 4 * verbs;
        // verb 0: 2 lines
        //   [ac 0007][fl 00] noun ENDIT                       = 3 + 5 + 1
        //   [ac 0008][fl 01] noun, 'in'/'into', noun ENDIT    = 3 + 20 + 1
        let v0_len = 1 + (3 + 5 + 1) + (3 + 20 + 1);
        let v1 = v0 + v0_len;
        // verb 1: 4 lines — a bare one, the decoy described below, and two
        // attribute lines after it so the decoy's forged pointer still lands
        // inside the grammar region.
        let v1_len = 1 + (3 + 1) + 3 * (3 + 5 + 1);
        let actions = v1 + v1_len;
        let action_count = 12u32;
        // "The compiler also takes care that a Unicode dictionary will start at
        // a word-aligned address" (Glulx Inform Tech. Ref. §4), so this table
        // sits behind two bytes of padding — which is the alignment slack
        // `action_candidates` searches, exercised here rather than assumed.
        let dict = match char_size {
            4 => (actions + 4 + 4 * action_count).next_multiple_of(4),
            _ => actions + 4 + 4 * action_count,
        };

        // At least `MIN_DICT_WORDS` records, because a shorter run is not a
        // positive identification and the locator declines it. Real dictionaries
        // run to hundreds; the filler nouns stand in for those.
        let mut words: Vec<(&str, u16, u32)> = vec![
            ("hold", VERB_DFLAG, 0),
            ("in", PREP_DFLAG, 0),
            ("into", PREP_DFLAG, 0),
            ("lamp", NOUN_DFLAG, 0),
            ("look", VERB_DFLAG, 1),
            ("take", VERB_DFLAG, 0),
        ];
        // The first three fillers are the ones a Unicode dictionary can hold
        // and a byte-valued one cannot: a Latin-1 letter, a word entirely above
        // $FF, and a word longer than `DICT_WORD_SIZE`.
        let fillers = match char_size {
            4 => ["café", "日本語", "abcdefghijkl"],
            _ => ["aa", "bb", "cc"],
        };
        for filler in fillers
            .into_iter()
            .chain(["dd", "ee", "ff", "gg", "hh", "ii", "jj", "kk", "ll", "mm", "nn"])
        {
            words.push((filler, NOUN_DFLAG, 0));
        }
        s.dictionary(dict, &words, verb_base, char_size);
        let stride = dict_stride(char_size);
        let in_addr = dict + 4 + stride;
        let into_addr = dict + 4 + 2 * stride;

        s.w32(g, verbs);
        s.w32(g + 4, v0);
        s.w32(g + 8, v1);

        let mut m = v0;
        s.b(m, 2);
        m += 1;
        s.w16(m, 7);
        s.b(m + 2, 0);
        m += 3;
        s.b(m, 0x01);
        s.w32(m + 1, 0);
        m += 5; // noun
        s.b(m, ENDIT);
        m += 1;
        s.w16(m, 8);
        s.b(m + 2, 0x01);
        m += 3; // reverse
        s.b(m, 0x01);
        s.w32(m + 1, 0);
        m += 5; // noun
        s.b(m, 0x62);
        s.w32(m + 1, in_addr);
        m += 5; // 'in', opens a list
        s.b(m, 0x52);
        s.w32(m + 1, into_addr);
        m += 5; // 'into', continues it
        s.b(m, 0x01);
        s.w32(m + 1, 0);
        m += 5; // noun
        s.b(m, ENDIT);
        m += 1;
        assert_eq!(m, v1, "verb 0 block length");

        s.b(m, 4);
        m += 1;
        s.w16(m, 9);
        s.b(m + 2, 0);
        m += 3;
        s.b(m, ENDIT);
        m += 1;

        // A DECOY, planted on purpose. Read as a grammar table, the four bytes
        // at this line's start are `00 00 00 04` — a verb count of 4 — and the
        // four after them are this attribute's value, set to exactly
        // `decoy + 4 + 4*4`. So the line satisfies `find_grammar`'s pointer-array
        // precondition perfectly, sits ABOVE the real table, and is therefore
        // the first thing a backward scan meets. Only walking it, and finding
        // that it does not end on the actions table, rejects it.
        //
        // This shape is not contrived. Across the 22 Glulx stories in the local
        // corpus, 889 byte offsets satisfy that precondition — 279 in one game —
        // and exactly 22 survive the walk.
        let decoy = m;
        s.w16(m, 0);
        s.b(m + 2, 0);
        m += 3;
        s.b(m, 0x04);
        s.w32(m + 1, decoy + 20);
        m += 5;
        s.b(m, ENDIT);
        m += 1;

        for (action, attr) in [(10u16, 17u32), (11, 18)] {
            s.w16(m, action);
            s.b(m + 2, 0);
            m += 3;
            s.b(m, 0x04);
            s.w32(m + 1, attr);
            m += 5;
            s.b(m, ENDIT);
            m += 1;
        }
        assert_eq!(m, actions, "verb 1 block length");
        assert!(decoy + 20 < actions, "the decoy must forge a pointer inside the region");

        s.w32(actions, action_count);
        for i in 0..action_count {
            s.w32(actions + 4 + i * 4, 0x60 + i); // plausible ROM addresses
        }
        s
    }

    #[test]
    fn locates_the_three_tables_by_the_chain() {
        let s = story();
        let t = locate(&s.mem()).expect("the chain closes");
        // Pinned exactly, not merely "found": the story plants a decoy that
        // satisfies every check except the walk landing on the actions table,
        // and it sits closer to `actions` than the real table does. An address
        // assertion is the only thing that can tell the two apart.
        assert_eq!(t.grammar, RAM);
        assert_eq!(t.verb_count, 2);
        assert_eq!(t.action_count, 12);
        assert_eq!(t.word_count, 20);
        assert_eq!(t.dict_stride, 16);
        assert_eq!(t.dict_word_size, 9);
        assert_eq!(t.dict_char_size, 1);
        // The tables abut: that is the property the locator proves, and the
        // only reason its answer can be trusted at all.
        assert!(t.grammar < t.actions && t.actions < t.dictionary);
        assert_eq!(t.actions + 4 + 4 * t.action_count, t.dictionary);
    }

    #[test]
    fn reads_verbs_lines_and_tokens() {
        let g = story().grammar().expect("synthetic story has a grammar");
        assert_eq!(g.verbs().len(), 2);

        // Inform numbers verbs downwards from $FFFF in the dictionary, so both
        // spellings of verb 0 must land on it.
        let take = g.verb_for_word("take").expect("knows 'take'");
        assert_eq!(take.number, 0);
        assert_eq!(take.words, vec!["hold".to_string(), "take".to_string()]);
        assert_eq!(take.lines.len(), 2);
        assert_eq!(take.lines[0].describe("take"), "take noun");
        assert_eq!(take.lines[0].action, 7);
        assert!(!take.lines[0].reverse);

        // Glulx keeps the swap flag in its own byte rather than in the action.
        assert!(take.lines[1].reverse);
        assert_eq!(take.lines[1].action, 8);
        assert_eq!(take.lines[1].describe("take"), "take noun in / into noun REVERSE");
        assert_eq!(take.lines[1].noun_count(), 2);
        assert_eq!(take.lines[1].slots[1].alternatives.len(), 2);
        assert!(take.accepts(2, &["in"]));
        assert!(take.accepts(2, &["into"]));
        assert!(!take.accepts(2, &["under"]));

        let look = g.verb_for_word("look").expect("knows 'look'");
        assert!(look.takes_bare());
        assert_eq!(look.lines.len(), 4);
        assert_eq!(look.lines[2].slots, vec![Slot::one(Token::Attribute(17))]);

        assert!(g.is_preposition("in") && g.is_preposition("into"));
        assert!(!g.is_verb("lamp"));
        assert!(g.roles("lamp").is_some_and(|r| r.noun && !r.verb));
        assert!(g.roles("take").is_some_and(|r| r.verb));
        assert_eq!(g.action_routines().len(), 12);
        assert_eq!(g.verbs_accepting(2, &["in"]).len(), 1);
    }

    // ── The two verb numberings ──────────────────────────────────────────────

    #[test]
    fn reads_the_pre_6_32_verb_numbering() {
        // Inform in Glulx mode wrote the Z-machine's one-byte inversion into
        // the two-byte field until v6.32 widened it, so a story built by
        // anything earlier holds $FF minus the verb index. Read against $FFFF
        // it names verb 65,427 — no verb at all — and the story comes back with
        // a full grammar table and not one verb WORD, which is the whole of
        // SQ-1114.
        let s = story_numbered(VERB_BASE_NARROW);
        let g = s.grammar().expect("synthetic story has a grammar");
        assert_eq!(g.verb_number_base(), VERB_BASE_NARROW);
        assert_eq!(g.verbs().len(), 2);
        let take = g.verb_for_word("take").expect("knows 'take'");
        assert_eq!(take.number, 0);
        assert_eq!(take.words, vec!["hold".to_string(), "take".to_string()]);
        assert_eq!(g.verb_for_word("look").map(|v| v.number), Some(1));
        assert_eq!(g.verb_words().count(), 3);
    }

    #[test]
    fn the_modern_verb_numbering_is_still_read_as_before() {
        let g = story().grammar().expect("synthetic story has a grammar");
        assert_eq!(g.verb_number_base(), VERB_BASE_WIDE);
        assert_eq!(g.verb_words().count(), 3);
    }

    #[test]
    fn a_verb_number_matching_neither_base_names_no_verb() {
        // The base is chosen by checking both against the grammar table's own
        // verb count, so a record that fits neither cannot drag the file onto
        // the wrong one: the modern base stands and that one word simply names
        // nothing, exactly as it did before this reader knew there were two.
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        s.w16(t.dictionary + 4 + 5 * 16 + 12, 0x8000); // "take"
        let g = s.grammar().expect("synthetic story has a grammar");
        assert_eq!(g.verb_number_base(), VERB_BASE_WIDE);
        assert!(!g.is_verb("take"));
        assert_eq!(g.verb_for_word("hold").map(|v| v.number), Some(0));
    }

    // ── Falsification ────────────────────────────────────────────────────────
    //
    // The locator derives three addresses that nothing in the file records. If
    // it can be made to answer when the chain does not actually close, every
    // address it returns is a guess and the consumer cannot tell.

    #[test]
    fn refuses_when_the_actions_table_does_not_abut_the_dictionary() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        // Change the action count so the table no longer ends at the dictionary.
        s.w32(t.actions, 10);
        assert_eq!(s.error(), Some(GrammarError::TablesNotFound));
    }

    #[test]
    fn refuses_when_the_grammar_walk_misses_the_actions_table() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        // Give the first verb one line too many: the walk now overruns.
        s.b(t.grammar + 4 + 4 * t.verb_count, 3);
        assert_eq!(s.error(), Some(GrammarError::TablesNotFound));
    }

    #[test]
    fn refuses_a_verb_pointer_that_does_not_follow_the_pointer_array() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        s.w32(t.grammar + 4, t.grammar + 4 + 4 * t.verb_count + 1);
        assert_eq!(s.error(), Some(GrammarError::TablesNotFound));
    }

    #[test]
    fn refuses_an_unknown_token_type() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        // Verb 0's first token: elementary (1) becomes 7, which no version of
        // the format defines. The walk still terminates, so this is caught by
        // the reader rather than the locator.
        s.b(t.grammar + 4 + 4 * t.verb_count + 4, 0x07);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_an_elementary_token_above_nine() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        s.w32(t.grammar + 4 + 4 * t.verb_count + 5, 40);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_a_preposition_that_names_no_dictionary_word() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        // Verb 0's second line, second token, is the 'in' preposition. Point it
        // between two records: a reader that shrugged would report a verb whose
        // preposition is a word the player can never type.
        let line2 = t.grammar + 4 + 4 * t.verb_count + 1 + 9;
        s.w32(line2 + 3 + 5 + 1, t.dictionary + 4 + 3);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn reports_absent_when_there_is_no_dictionary_at_all() {
        let s = Story::new();
        assert_eq!(s.error(), Some(GrammarError::Absent));
    }

    // ── The two dictionary record shapes ─────────────────────────────────────

    #[test]
    fn reads_a_unicode_dictionary() {
        // `$DICT_CHAR_SIZE=4`: the tag padded out to a long, characters as
        // big-endian longs, the flags and verb shorts at `4 + 4*DICT_WORD_SIZE`
        // instead of `1 + DICT_WORD_SIZE`, and a 48-byte record instead of a
        // 16-byte one. Before SQ-1231 this whole shape was a refusal
        // (`GrammarError::UnicodeDictionary`), which is what this case fails
        // with if the reader is reverted.
        let g = story_unicode().grammar().expect("a Unicode dictionary reads");
        let t = g.tables();
        assert_eq!(t.dict_char_size, 4);
        assert_eq!(t.dict_stride, 48);
        assert_eq!(t.dict_word_size, 9);
        assert_eq!(t.word_count, 20);
        // The table is word-aligned behind the compiler's padding, so it does
        // NOT abut the actions table the way a byte-valued one does.
        assert_eq!(t.dictionary, 388);
        assert_eq!(t.actions + 4 + 4 * t.action_count, 386);

        // A character that is Latin-1, and one that is not: read at one byte
        // per character these are `caf`+garbage and nothing at all, and read at
        // the wrong offset they are empty.
        assert!(g.roles("café").is_some_and(|r| r.noun));
        assert!(g.roles("日本語").is_some_and(|r| r.noun));
        // Truncated at `DICT_WORD_SIZE` CHARACTERS, not bytes.
        assert!(g.roles("abcdefghi").is_some_and(|r| r.noun));
        assert!(g.roles("abcdefghijkl").is_none());
        assert_eq!(g.words().count(), 20);

        // The flags and verb shorts landed where `DICT_ENTRY_FLAG_POS` puts
        // them: everything the byte-valued story asserts, off the wide records.
        assert_eq!(g.verb_number_base(), VERB_BASE_WIDE);
        let take = g.verb_for_word("take").expect("knows 'take'");
        assert_eq!(take.number, 0);
        assert_eq!(take.words, vec!["hold".to_string(), "take".to_string()]);
        assert_eq!(take.lines[1].describe("take"), "take noun in / into noun REVERSE");
        // And the grammar's own word tokens still resolve, which they can only
        // do if the records were walked at the wide stride.
        assert!(g.is_preposition("in") && g.is_preposition("into"));
        assert!(g.roles("lamp").is_some_and(|r| r.noun && !r.verb));
        assert_eq!(g.verb_words().count(), 3);
    }

    #[test]
    fn an_empty_word_does_not_make_a_byte_dictionary_look_unicode() {
        // SQ-1231, and the whole of it. `stories/CoS.blb` (City of Secrets,
        // Inform 6.21, serial 030624) opens its 3,551-word BYTE dictionary with
        // the empty word — a meta-verb its menu system defines, flagged
        // `VERB|META|TRUNC`. The Unicode test used to be "any of the first
        // eight records has a zero after the tag", so that one record read an
        // entirely ordinary stride-16 dictionary as Unicode and refused the
        // story: no verb column, no word reveal, no guidance offer, all game.
        //
        // Blank record 0's text — "hold", a verb, exactly as CoS's empty word
        // is one — and the file is still plainly byte-valued, because every
        // OTHER record has a character where a Unicode record has padding.
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        for j in 0..t.dict_word_size {
            s.b(t.dictionary + 4 + 1 + j, 0); // past the `$60` tag
        }
        let g = s.grammar().expect("a byte dictionary with an empty word still reads");
        assert_eq!(g.tables().dict_char_size, 1);
        assert_eq!(g.tables().dict_word_size, 9);
        // Nothing else moved: the empty spelling is simply what the record
        // holds, and every other reading is as it was.
        let take = g.verb_for_word("take").expect("knows 'take'");
        assert_eq!(take.words, vec![String::new(), "take".to_string()]);
        assert_eq!(take.lines[1].describe("take"), "take noun in / into noun REVERSE");
        assert!(g.is_preposition("in") && g.roles("lamp").is_some_and(|r| r.noun));
        assert_eq!(g.verb_for_word("look").map(|v| v.number), Some(1));
    }
}
