//! Z-machine dictionary and input tokeniser — ZMSD §13.
//!
//! Dictionary layout (at `mem.dictionary()` base):
//!
//! ```text
//! 1 byte  n            — number of word-separator ZSCII codes
//! n bytes              — the separator codes
//! 1 byte  entry_length — bytes per entry (≥ 4 for v3, ≥ 6 for v4+)
//! 2 bytes count        — number of entries (i16; negative ⇒ unsorted)
//! ```
//!
//! Then `count` (abs) entries of `entry_length` bytes each.
//! The first 4 (v3) or 6 (v4+) bytes of each entry are the encoded word key.

use crate::memory::Memory;
use crate::text::encode::encode_word_mem;

pub struct Dictionary {
    pub entry_length: u8,
    pub count: u16,       // absolute count (always ≥ 1)
    pub base: u32,        // byte address of first entry
    pub separators: Vec<u8>,
    sorted: bool,
    key_len: u8,          // 4 for v3, 6 for v4+
}

/// Parse the standard story dictionary (header 0x08) and return a `Dictionary`.
pub fn load(mem: &Memory) -> Dictionary {
    load_at(mem, mem.dictionary() as u32)
}

/// Parse a dictionary whose header begins at `dict_base` and return a
/// `Dictionary` ready for lookup. Used by `tokenise` (VAR:0x1B) to honour a
/// caller-supplied custom dictionary address (ZMSD §15); custom dictionaries
/// share the standard header layout.
pub fn load_at(mem: &Memory, dict_base: u32) -> Dictionary {
    let mut cursor = dict_base;

    // Separator list.
    let n = mem.read_byte(cursor) as usize;
    cursor += 1;
    let mut separators = Vec::with_capacity(n);
    for _ in 0..n {
        separators.push(mem.read_byte(cursor));
        cursor += 1;
    }

    // Entry structure.
    let entry_length = mem.read_byte(cursor);
    cursor += 1;
    let raw_count = mem.read_word(cursor) as i16;
    cursor += 2;

    let (count, sorted) = if raw_count < 0 {
        ((-raw_count) as u16, false)
    } else {
        (raw_count as u16, true)
    };

    // Clamp the entry count so every entry both lies inside loaded memory and
    // STARTS within the 16-bit address space: entry addresses are returned as
    // u16 words (the parse buffer stores them in 2 bytes, ZMSD §13.6.1), so an
    // entry past 0xFFFF is unaddressable — a malformed custom dictionary would
    // otherwise make `lookup` silently truncate `base + i*entry_length` to a
    // wrong low address. A zero entry_length is equally malformed (every entry
    // would alias the first); treat it as an empty dictionary.
    let count = if entry_length == 0 || cursor > 0xFFFF || cursor as usize >= mem.len() {
        0
    } else {
        let elen = entry_length as u32;
        let fit_addr = (0xFFFF - cursor) / elen + 1; // entries starting ≤ 0xFFFF
        let fit_mem = (mem.len() as u32 - cursor) / elen; // entries fully in memory
        (count as u32).min(fit_addr).min(fit_mem) as u16
    };

    let key_len: u8 = if mem.version() <= 3 { 4 } else { 6 };

    Dictionary {
        entry_length,
        count,
        base: cursor,
        separators,
        sorted,
        key_len,
    }
}

impl Dictionary {
    /// Return all words stored in the dictionary as decoded strings.
    ///
    /// Each entry's key bytes are decoded with `zvm::text::decode_string`.
    /// The decoded strings are returned in entry order (sorted order for
    /// typical dictionaries). Trailing pad characters (Z-char 5 pad = ASCII
    /// space in some contexts) are stripped; empty strings are omitted.
    pub fn words(&self, mem: &Memory) -> Vec<String> {
        use crate::text::decode::decode_string;
        let elen = self.entry_length as u32;
        let mut result = Vec::with_capacity(self.count as usize);
        for i in 0..self.count as u32 {
            let entry_addr = self.base + i * elen;
            // The key occupies the first key_len bytes of each entry. We
            // decode from the entry address; decode_string reads until the
            // high-bit terminator word, which for a 4-byte (v3) or 6-byte
            // (v4+) key is the single 2-byte or 3-byte Z-string.
            let (word, _) = decode_string(mem, entry_addr);
            let word = word.trim().to_lowercase();
            if !word.is_empty() {
                result.push(word);
            }
        }
        result
    }

    /// Like [`words`](Self::words) but pairs each non-empty decoded word with
    /// its entry byte address — for the debug inspector's clickable dictionary
    /// links (jump the Memory pane to the entry). Entry order, same filtering.
    pub fn entries(&self, mem: &Memory) -> Vec<(u32, String)> {
        use crate::text::decode::decode_string;
        let elen = self.entry_length as u32;
        let mut result = Vec::with_capacity(self.count as usize);
        for i in 0..self.count as u32 {
            let entry_addr = self.base + i * elen;
            let (word, _) = decode_string(mem, entry_addr);
            let word = word.trim().to_lowercase();
            if !word.is_empty() {
                result.push((entry_addr, word));
            }
        }
        result
    }

    /// Bytes of each entry taken up by its encoded key: 4 in v1–3 (ZMSD §13.3),
    /// 6 in v4+ (§13.4). The rest of the entry — `entry_length` is a byte the
    /// *game* chose, and §13.2 gives only a minimum — is game-defined data, not
    /// Z-text, so a caller mapping addresses to strings must stop here.
    pub fn key_len(&self) -> u8 {
        self.key_len
    }

    /// Look up `word` in the dictionary. Returns the entry's byte address, or
    /// 0 if not found. Encodes `word` with `encode_word` (which truncates to
    /// the key length) and compares against stored keys.
    pub fn lookup(&self, mem: &Memory, word: &str) -> u16 {
        let key = encode_word_mem(word, mem);
        let klen = self.key_len as usize;
        let elen = self.entry_length as u32;

        if self.sorted {
            // Binary search: keys are big-endian bytes, byte comparison works.
            let mut lo = 0usize;
            let mut hi = self.count as usize;
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                let entry_addr = self.base + mid as u32 * elen;
                let entry_key = self.read_key(mem, entry_addr, klen);
                match entry_key.as_slice().cmp(key.as_slice()) {
                    std::cmp::Ordering::Equal => return entry_addr as u16,
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                }
            }
            0
        } else {
            // Linear search for unsorted dictionaries.
            for i in 0..self.count as u32 {
                let entry_addr = self.base + i * elen;
                let entry_key = self.read_key(mem, entry_addr, klen);
                if entry_key == key {
                    return entry_addr as u16;
                }
            }
            0
        }
    }

    fn read_key(&self, mem: &Memory, addr: u32, klen: usize) -> Vec<u8> {
        (0..klen as u32).map(|i| mem.read_byte(addr + i)).collect()
    }

    /// Tokenise `text` — the game text buffer's contents as raw **ZSCII bytes**
    /// (one byte per character, exactly as stored in memory by `read`) — by
    /// splitting on spaces and dictionary separators.
    /// Each separator character is itself a token. Returns tokens in order with
    /// their dictionary address (0 if absent), length, and byte position in `text`.
    ///
    /// Operating on ZSCII bytes (not `&str`) is load-bearing: separator codes
    /// and buffer bytes can be ≥ 0x80 (accented ZSCII), and indexing a UTF-8
    /// `&str` at those byte positions panicked on char boundaries (SQ-0621).
    pub fn tokenise(&self, mem: &Memory, text: &[u8]) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut i = 0usize;

        while i < text.len() {
            let b = text[i];

            // Skip spaces.
            if b == b' ' {
                i += 1;
                continue;
            }

            // Separator character — emit as its own 1-character token.
            if self.separators.contains(&b) {
                let addr = self.lookup_zscii(mem, &text[i..i + 1]);
                tokens.push(Token {
                    dict_addr: addr,
                    len: 1,
                    text_pos: i as u8,
                });
                i += 1;
                continue;
            }

            // Regular word — consume until space or separator.
            let start = i;
            while i < text.len() && text[i] != b' ' && !self.separators.contains(&text[i]) {
                i += 1;
            }
            let addr = self.lookup_zscii(mem, &text[start..i]);
            tokens.push(Token {
                dict_addr: addr,
                len: (i - start) as u8,
                text_pos: start as u8,
            });
        }

        tokens
    }

    /// [`lookup`](Self::lookup) for a word given as raw ZSCII bytes (the text
    /// buffer's native representation): decode each ZSCII code to its Unicode
    /// character (custom Unicode table first, ZMSD §3.8.5.4), then encode for
    /// the key comparison — the exact inverse of how `supply_line` produced
    /// the bytes, so accented input round-trips to the right dictionary key.
    fn lookup_zscii(&self, mem: &Memory, word: &[u8]) -> u16 {
        let decoded: String = word
            .iter()
            .map(|&b| {
                let z = b as u16;
                mem.unicode_char(z).unwrap_or_else(|| crate::text::decode::zscii_to_char(z))
            })
            .collect();
        self.lookup(mem, &decoded)
    }
}

/// A single parsed token from the input line.
#[non_exhaustive]
pub struct Token {
    /// Byte address of the dictionary entry, or 0 if not found.
    pub dict_addr: u16,
    /// Length in bytes of the token text.
    pub len: u8,
    /// Byte position of the token in the input string.
    pub text_pos: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::text::encode::encode_word;

    // Build a sample Memory with a hand-crafted v3 dictionary at 0x0200.
    //
    // Dictionary layout:
    //   0x0200: 2         — 2 separators
    //   0x0201: 46 (.)
    //   0x0202: 44 (,)
    //   0x0203: 7         — entry_length = 7 (4-byte key + 3 bytes game data)
    //   0x0204-0x0205: 3  — 3 entries
    //   entries start at 0x0206
    //     entry 0: key("mailbox") + [0,0,0]
    //     entry 1: key("north")   + [0,0,0]
    //     entry 2: key("open")    + [0,0,0]
    //
    // Entries are in sorted key order (a requirement for binary search).
    fn make_v3_mem_with_dict() -> (Memory, [u8; 4], [u8; 4], [u8; 4]) {
        let mut buf = sample_story(3);

        let key_mailbox = encode_word("mailbox", 3);
        let key_north   = encode_word("north",   3);
        let key_open    = encode_word("open",    3);

        // Sort entries by key to satisfy binary search.
        let mut entries: Vec<&[u8]> = vec![&key_mailbox, &key_north, &key_open];
        entries.sort();
        // Reify sorted order.
        let sorted: Vec<Vec<u8>> = entries.iter().map(|k| k.to_vec()).collect();

        // Write dictionary header at 0x0200.
        buf[0x0200] = 2;    // 2 separators
        buf[0x0201] = 46;   // '.'
        buf[0x0202] = 44;   // ','
        buf[0x0203] = 7;    // entry_length
        buf[0x0204] = 0;    // count high byte
        buf[0x0205] = 3;    // count low byte = 3

        // Write 3 entries at 0x0206.
        for (i, key) in sorted.iter().enumerate() {
            let base = 0x0206 + i * 7;
            buf[base..base + 4].copy_from_slice(&key[..4]);
            // 3 bytes game data — leave as zeros.
        }

        let km: [u8; 4] = key_mailbox.try_into().unwrap();
        let kn: [u8; 4] = key_north.try_into().unwrap();
        let ko: [u8; 4] = key_open.try_into().unwrap();

        (Memory::new(buf).unwrap(), km, kn, ko)
    }

    fn entry_addr_for_key(key: &[u8; 4], sorted_keys: &[[u8; 4]]) -> u16 {
        // Find the entry address of a given key in our hand-built dict.
        for (i, k) in sorted_keys.iter().enumerate() {
            if k == key {
                return (0x0206 + i * 7) as u16;
            }
        }
        panic!("key not in sorted list");
    }

    #[test]
    fn load_reads_separators_entry_length_and_count() {
        let (m, _, _, _) = make_v3_mem_with_dict();
        let d = load(&m);
        assert_eq!(d.separators, vec![46, 44]);
        assert_eq!(d.entry_length, 7);
        assert_eq!(d.count, 3);
        assert_eq!(d.base, 0x0206);
    }

    #[test]
    fn lookup_finds_present_word() {
        let (m, km, kn, ko) = make_v3_mem_with_dict();
        let d = load(&m);

        // Sort the keys so we can compute expected addresses.
        let mut sorted_keys = [km, kn, ko];
        sorted_keys.sort();

        let addr_open = entry_addr_for_key(&ko, &sorted_keys);
        assert_ne!(d.lookup(&m, "open"), 0, "'open' must be found");
        assert_eq!(d.lookup(&m, "open"), addr_open);
    }

    #[test]
    fn lookup_returns_zero_for_absent_word() {
        let (m, _, _, _) = make_v3_mem_with_dict();
        let d = load(&m);
        assert_eq!(d.lookup(&m, "xyzzy"), 0);
    }

    #[test]
    fn tokenise_two_words_correct_positions() {
        let (m, km, kn, ko) = make_v3_mem_with_dict();
        let d = load(&m);
        let toks = d.tokenise(&m, b"open mailbox");

        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].text_pos, 0);
        assert_eq!(toks[0].len, 4); // "open"
        assert_eq!(toks[1].text_pos, 5);
        assert_eq!(toks[1].len, 7); // "mailbox"

        // Both should be found in the dict.
        let mut sorted_keys = [km, kn, ko];
        sorted_keys.sort();
        let expected_open    = entry_addr_for_key(&ko, &sorted_keys);
        let expected_mailbox = entry_addr_for_key(&km, &sorted_keys);
        assert_eq!(toks[0].dict_addr, expected_open);
        assert_eq!(toks[1].dict_addr, expected_mailbox);
    }

    #[test]
    fn tokenise_splits_on_separator() {
        let (m, _, _, _) = make_v3_mem_with_dict();
        let d = load(&m);
        // "a,b" → tokens: "a" (pos 0, len 1), "," (pos 1, len 1), "b" (pos 2, len 1)
        let toks = d.tokenise(&m, b"a,b");

        assert_eq!(toks.len(), 3);
        assert_eq!(toks[0].text_pos, 0); assert_eq!(toks[0].len, 1);
        assert_eq!(toks[1].text_pos, 1); assert_eq!(toks[1].len, 1);
        assert_eq!(toks[2].text_pos, 2); assert_eq!(toks[2].len, 1);
    }

    #[test]
    fn tokenise_separator_is_own_token_with_dict_lookup() {
        let (m, _, _, _) = make_v3_mem_with_dict();
        let d = load(&m);
        // "." is a separator — it is a token even if not in dict.
        let toks = d.tokenise(&m, b".");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].len, 1);
        assert_eq!(toks[0].text_pos, 0);
    }

    #[test]
    fn tokenise_skips_leading_and_trailing_spaces() {
        let (m, _, _, _) = make_v3_mem_with_dict();
        let d = load(&m);
        let toks = d.tokenise(&m, b"  open  ");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].text_pos, 2);
        assert_eq!(toks[0].len, 4);
    }

    #[test]
    fn lookup_handles_unsorted_dict() {
        // Build an unsorted dict: count written as negative i16.
        let mut buf = sample_story(3);

        let key_open    = encode_word("open",    3);
        let key_mailbox = encode_word("mailbox", 3);

        // Write dict header — unsorted: count = -2 (0xFFFE as u16).
        // Layout with 0 separators:
        //   0x0200: n=0
        //   0x0201: entry_length=7
        //   0x0202-0x0203: count = 0xFFFE = -2 as i16 → abs=2, unsorted
        //   0x0204: first entry
        buf[0x0200] = 0;    // 0 separators
        buf[0x0201] = 7;    // entry_length
        buf[0x0202] = 0xFF; // count high = 0xFF (negative i16)
        buf[0x0203] = 0xFE; // count low → i16 = -2, abs = 2

        // Entries in UNsorted order starting at 0x0204.
        buf[0x0204..0x0208].copy_from_slice(&key_open);
        // second entry at 0x0204 + 7 = 0x020B
        buf[0x020B..0x020F].copy_from_slice(&key_mailbox);

        let m = Memory::new(buf).unwrap();
        let d = load(&m);
        assert!(!d.sorted, "should be unsorted");
        assert_eq!(d.count, 2);

        assert_ne!(d.lookup(&m, "open"), 0);
        assert_ne!(d.lookup(&m, "mailbox"), 0);
        assert_eq!(d.lookup(&m, "north"), 0);
    }

    #[test]
    fn malformed_dictionary_entries_past_u16_are_clamped() {
        // A custom dictionary near the top of the 16-bit address space whose
        // claimed count would place entries past 0xFFFF. Entry addresses are
        // returned (and stored in the parse buffer) as u16 words, so those
        // entries are unaddressable: the count is clamped at load. Pre-fix,
        // lookup silently truncated `base + i*entry_length` and returned a
        // bogus low address. (SQ-0620)
        let mut buf = vec![0u8; 0x11000];
        buf[0x00] = 3;    // v3
        buf[0x04] = 0x04; // high_mem_base
        buf[0x07] = 0x40; // initial_pc
        buf[0x08] = 0x02; // dictionary (unused; we call load_at)
        buf[0x0A] = 0x01; // object_table
        buf[0x0C] = 0x03; // global_vars
        buf[0x0E] = 0x04; // static_mem_base = 0x0400
        buf[0x19] = 0x40; // abbrev_table

        let dict_base = 0xFFF0usize;
        buf[dict_base] = 0;     // 0 separators
        buf[dict_base + 1] = 7; // entry_length
        // count = -100 (unsorted ⇒ linear scan visits every claimed entry)
        buf[dict_base + 2] = 0xFF;
        buf[dict_base + 3] = 0x9C;
        // Key for "open" placed in entry index 10 — which starts past 0xFFFF.
        let key = encode_word("open", 3);
        let entries_base = dict_base + 4;
        let idx10 = entries_base + 10 * 7;
        buf[idx10..idx10 + 4].copy_from_slice(&key);

        let m = Memory::new(buf).unwrap();
        let d = load_at(&m, dict_base as u32);
        assert!(d.count <= 2, "count clamped to entries starting ≤ 0xFFFF, got {}", d.count);
        assert_eq!(
            d.lookup(&m, "open"),
            0,
            "an entry past 0xFFFF must miss, not return a truncated address"
        );
    }

    #[test]
    fn zero_entry_length_dictionary_is_empty() {
        // entry_length 0 would make every entry alias the first (and divide by
        // zero in the clamp) — treated as an empty dictionary. (SQ-0620)
        let mut buf = sample_story(3);
        buf[0x0200] = 0; // 0 separators
        buf[0x0201] = 0; // entry_length = 0 (malformed)
        buf[0x0202] = 0;
        buf[0x0203] = 50; // claims 50 entries
        let m = Memory::new(buf).unwrap();
        let d = load(&m);
        assert_eq!(d.count, 0);
        assert_eq!(d.lookup(&m, "open"), 0);
    }

    #[test]
    fn tokenise_handles_zscii_bytes_above_0x7f() {
        // The text buffer holds ZSCII bytes, which can be ≥ 0x80 (accented
        // characters) — both as word characters and as separators. The old
        // &str-based tokeniser indexed UTF-8 byte positions and panicked on
        // char boundaries. (SQ-0621)
        let (m, _, _, _) = make_v3_mem_with_dict();
        let mut d = load(&m);
        d.separators.push(155); // 'ä' as a separator code
        // 'é'(170) 'x' | 'ä'(155) separator | 'é'(170)
        let toks = d.tokenise(&m, &[170, b'x', 155, 170]);
        assert_eq!(toks.len(), 3);
        assert_eq!((toks[0].text_pos, toks[0].len), (0, 2), "word 'éx'");
        assert_eq!((toks[1].text_pos, toks[1].len), (2, 1), "separator 'ä'");
        assert_eq!((toks[2].text_pos, toks[2].len), (3, 1), "word 'é'");
    }

    // Fixture-backed test — skips if minizork.z3 is not present.
    #[test]
    fn tokenises_and_resolves_known_words() {
        let Some(story) = crate::fixtures::load("minizork.z3") else { return };
        let m = crate::memory::Memory::new(story).unwrap();
        let d = load(&m);
        let toks = d.tokenise(&m, b"open mailbox");
        assert_eq!(toks.len(), 2);
        assert!(toks[0].dict_addr != 0, "'open' should be in MiniZork's dictionary");
    }

    #[test]
    fn entries_pairs_each_word_with_its_entry_address() {
        let (m, _, _, _) = make_v3_mem_with_dict();
        let d = load(&m);
        let entries = d.entries(&m);
        assert_eq!(entries.len(), 3);
        // Entries are laid out contiguously from `base`, `entry_length` apart.
        assert_eq!(entries[0].0, d.base);
        assert_eq!(entries[1].0, d.base + d.entry_length as u32);
        assert_eq!(entries[2].0, d.base + 2 * d.entry_length as u32);
        // The words match `words()` (same decode + filtering), now addressed.
        let words: Vec<String> = entries.iter().map(|(_, w)| w.clone()).collect();
        assert_eq!(words, d.words(&m));
    }

    // Fixture-backed test for words() — skips if minizork.z3 is not present.
    #[test]
    fn words_returns_nonempty_list_containing_known_verb() {
        let Some(story) = crate::fixtures::load("minizork.z3") else { return };
        let m = crate::memory::Memory::new(story).unwrap();
        let d = load(&m);
        let words = d.words(&m);
        assert!(!words.is_empty(), "words() must return a non-empty list for minizork.z3");
        assert!(
            words.iter().any(|w| w == "open"),
            "minizork.z3 dictionary must contain 'open'; got: {:?}",
            &words[..words.len().min(20)]
        );
    }
}
