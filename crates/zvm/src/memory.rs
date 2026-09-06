// Z-machine memory model — ZMSD §1.1, §1.2.
//
// Dynamic memory (0x0000 up to but not including static_mem_base) is readable
// and writable. Static memory and high memory are read-only. All multi-byte
// story values are big-endian.

use crate::error::ZError;
use crate::header::{parse_header, Header};

#[derive(Debug)]
pub struct Memory {
    bytes: Vec<u8>,
    header: Header,
    /// Custom Unicode translation table parsed from the header-extension table
    /// (ZMSD §3.8.5.4). Maps ZSCII codes 155, 156, … to Unicode chars.
    /// `None` if the story has no custom table; the default table is used instead.
    unicode_table: Option<Vec<char>>,
    /// Latched out-of-bounds access from the current instruction: (is_write,
    /// size_bytes, addr). Interior-mutable so read paths (&self) can record it.
    /// Drained by `take_mem_fault`; the CPU checks it after each instruction.
    mem_fault: std::cell::Cell<Option<(bool, u8, u32)>>,
    /// Memo for `objects::short_name_property` — the property number this
    /// story's own symbol table gives Inform's `short_name`, or `None` where it
    /// carries no such table (SQ-1372). Finding it reads the whole image once;
    /// `objects::printed_name` asks for it per object, per turn.
    ///
    /// A story's property numbering is fixed at compile time, so one answer
    /// serves the whole session — a `@restore` rewrites dynamic memory with the
    /// same story's bytes, and the identifiers table is part of them.
    short_name_prop: std::cell::OnceCell<Option<u8>>,
}

/// Parse the custom Unicode translation table from raw story bytes if present
/// (ZMSD §3.8.5.4). Returns `None` if no custom table is defined.
///
/// Header byte 0x36 (word) → header-extension table address.
/// Header-ext word 0 = number of words in that table.
/// Header-ext word 3 (1-indexed) = address of Unicode translation table.
/// Unicode table: 1 byte N, then N big-endian 16-bit Unicode code points
/// mapping ZSCII 155, 156, … 154+N.
fn parse_unicode_table(bytes: &[u8]) -> Option<Vec<char>> {
    // Byte 0x36 is only present in v5+ headers (which are ≥ 64 bytes).
    if bytes.len() < 0x38 {
        return None;
    }
    let ext_addr = (((bytes[0x36] as u16) << 8) | bytes[0x37] as u16) as usize;
    if ext_addr == 0 || ext_addr + 2 > bytes.len() {
        return None;
    }
    // Word 0 of header-ext table: number of words in the table.
    let ext_len = (((bytes[ext_addr] as u16) << 8) | bytes[ext_addr + 1] as u16) as usize;
    if ext_len < 3 {
        return None;
    }
    // Word 3 (byte offset 6 from table start) = Unicode table address.
    let uni_word_addr = ext_addr + 6;
    if uni_word_addr + 2 > bytes.len() {
        return None;
    }
    let uni_addr = (((bytes[uni_word_addr] as u16) << 8) | bytes[uni_word_addr + 1] as u16) as usize;
    if uni_addr == 0 || uni_addr >= bytes.len() {
        return None;
    }
    let n = bytes[uni_addr] as usize;
    if n == 0 {
        return None;
    }
    let mut table = Vec::with_capacity(n);
    for i in 0..n {
        let off = uni_addr + 1 + i * 2;
        if off + 2 > bytes.len() {
            break;
        }
        let cp = ((bytes[off] as u32) << 8) | bytes[off + 1] as u32;
        table.push(char::from_u32(cp).unwrap_or('?'));
    }
    Some(table)
}

impl Memory {
    /// Construct a `Memory` from raw story bytes.
    ///
    /// Parses the header and validates that `static_mem_base` does not exceed
    /// the buffer length (which would make the dynamic region nonsensical).
    pub fn new(bytes: Vec<u8>) -> Result<Memory, ZError> {
        let header = parse_header(&bytes)?;
        if header.static_mem_base as usize > bytes.len() {
            return Err(ZError::Truncated);
        }
        let unicode_table = parse_unicode_table(&bytes);
        Ok(Memory {
            bytes,
            header,
            unicode_table,
            mem_fault: std::cell::Cell::new(None),
            short_name_prop: std::cell::OnceCell::new(),
        })
    }

    /// The `short_name` property-number memo — see the field, and
    /// `objects::printed_name` which is its only caller.
    pub(crate) fn short_name_prop_memo(&self) -> &std::cell::OnceCell<Option<u8>> {
        &self.short_name_prop
    }

    /// Length of the story file in bytes.
    #[allow(clippy::len_without_is_empty)]
    // A loaded Memory always holds at least a 64-byte header; never empty.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Read a single byte at `addr`.
    pub fn read_byte(&self, addr: u32) -> u8 {
        match self.bytes.get(addr as usize) {
            Some(&b) => b,
            None => {
                self.latch_fault(false, 1, addr);
                0
            }
        }
    }

    /// Write a single byte at `addr`. Only dynamic memory (addr < static_mem_base)
    /// is writable (ZMSD §1.1): a story write into static/high memory latches a
    /// fault and leaves memory untouched. This must be a real runtime check, not
    /// a debug_assert — Quetzal CMem only saves the dynamic region, so a silent
    /// release-build write above static_mem_base would mutate state that a
    /// save/restore round-trip can never reproduce.
    pub fn write_byte(&mut self, addr: u32, v: u8) {
        if addr >= self.header.static_mem_base as u32 {
            self.latch_fault(true, 1, addr);
            return;
        }
        match self.bytes.get_mut(addr as usize) {
            Some(slot) => *slot = v,
            None => self.latch_fault(true, 1, addr),
        }
    }

    /// Read a big-endian 16-bit word at `addr`.
    pub fn read_word(&self, addr: u32) -> u16 {
        let i = addr as usize;
        match (self.bytes.get(i), self.bytes.get(i + 1)) {
            (Some(&hi), Some(&lo)) => ((hi as u16) << 8) | lo as u16,
            _ => {
                self.latch_fault(false, 2, addr);
                0
            }
        }
    }

    /// Write a big-endian 16-bit word at `addr`. Only dynamic memory is writable
    /// — same runtime read-only enforcement as [`write_byte`]. The whole word
    /// must lie below static_mem_base (a word straddling the boundary would
    /// half-corrupt static memory).
    pub fn write_word(&mut self, addr: u32, v: u16) {
        if addr + 1 >= self.header.static_mem_base as u32 {
            self.latch_fault(true, 2, addr);
            return;
        }
        let i = addr as usize;
        if i + 1 < self.bytes.len() {
            self.bytes[i] = (v >> 8) as u8;
            self.bytes[i + 1] = (v & 0xFF) as u8;
        } else {
            self.latch_fault(true, 2, addr);
        }
    }

    fn latch_fault(&self, is_write: bool, size: u8, addr: u32) {
        if self.mem_fault.get().is_none() {
            self.mem_fault.set(Some((is_write, size, addr)));
        }
    }

    /// Take and clear a latched OOB access recorded since the last drain.
    pub fn take_mem_fault(&self) -> Option<(bool, u8, u32)> {
        self.mem_fault.take()
    }

    /// Run `f` without letting anything it reads latch a memory fault, and
    /// without disturbing a fault already latched.
    ///
    /// The fault latch exists to report *the story* stepping outside its own
    /// memory, and the app turns it into a VM fault that ends the session. Host
    /// code that SPECULATES — the automapper following a property word that may
    /// or may not be a packed string address (`object_text_property`) — is not
    /// the story, and a guess that lands out of bounds is an answer ("not a
    /// string"), not a crash. Without this the automapper's own probing killed
    /// mysterious01 one turn later, with a fault trace pointing at whatever the
    /// story happened to be executing (SQ-0724).
    ///
    /// Only for reads. A speculative WRITE has no legitimate caller.
    pub fn without_fault_latch<T>(&self, f: impl FnOnce() -> T) -> T {
        let saved = self.mem_fault.get();
        let out = f();
        self.mem_fault.set(saved);
        out
    }

    /// Unpack a packed routine address (ZMSD §1.2.3).
    pub fn unpack_routine(&self, packed: u16) -> u32 {
        let p = packed as u32;
        match self.header.version {
            3 => 2 * p,
            4 | 5 => 4 * p,
            6 | 7 => 4 * p + 8 * self.header.routines_offset as u32,
            8 => 8 * p,
            _ => unreachable!("version validated by parse_header"),
        }
    }

    /// Z-machine version from the header.
    pub fn version(&self) -> u8 {
        self.header.version
    }

    /// Abbreviation table base address from the header.
    pub fn abbrev_table(&self) -> u16 {
        self.header.abbrev_table
    }

    /// Object table base address from the header.
    pub fn object_table(&self) -> u16 {
        self.header.object_table
    }

    /// Dictionary base address from the header.
    pub fn dictionary(&self) -> u16 {
        self.header.dictionary
    }

    /// Global variables table base address from the header.
    pub fn global_vars(&self) -> u16 {
        self.header.global_vars
    }

    /// Static memory base address (= end of dynamic memory region).
    pub fn static_mem_base(&self) -> u16 {
        self.header.static_mem_base
    }

    /// Raw read access to the underlying byte slice (for Quetzal CMem XOR).
    pub fn raw_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Look up a ZSCII code ≥ 155 in the custom Unicode translation table
    /// (ZMSD §3.8.5.4). Returns `Some(char)` if the story defines a custom
    /// table and the code falls within it; `None` otherwise (caller falls back
    /// to the default table).
    pub fn unicode_char(&self, zscii: u16) -> Option<char> {
        let table = self.unicode_table.as_ref()?;
        let idx = zscii.checked_sub(155)? as usize;
        table.get(idx).copied()
    }

    /// Map a Rust `char` to a ZSCII byte, consulting the story's custom Unicode
    /// translation table first (ZMSD §3.8.5.4) and falling back to the default
    /// table (ZMSD §3.8) — the inverse of `unicode_char`/`zscii_to_char`.
    pub fn zscii_from_unicode(&self, ch: char) -> u8 {
        if let Some(table) = self.unicode_table.as_ref() {
            if let Some(i) = table.iter().position(|&c| c == ch) {
                return 155 + i as u8;
            }
        }
        crate::text::decode::char_to_zscii_default(ch)
    }

    /// Unpack a packed string address (ZMSD §1.2.3).
    pub fn unpack_string(&self, packed: u16) -> u32 {
        let p = packed as u32;
        match self.header.version {
            3 => 2 * p,
            4 | 5 => 4 * p,
            6 | 7 => 4 * p + 8 * self.header.strings_offset as u32,
            8 => 8 * p,
            _ => unreachable!("version validated by parse_header"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;

    #[test]
    fn reads_and_writes_words_big_endian() {
        let mut m = Memory::new(sample_story(3)).unwrap();
        m.write_word(0x40, 0xBEEF);
        assert_eq!(m.read_byte(0x40), 0xBE);
        assert_eq!(m.read_byte(0x41), 0xEF);
        assert_eq!(m.read_word(0x40), 0xBEEF);
    }

    /// SQ-0724. A speculative read inside `without_fault_latch` must answer with
    /// its zero and leave no trace: the automapper probes property words that may
    /// not be addresses at all, and a wrong guess is not the story faulting. A
    /// fault latched BEFORE the guarded block is the story's, and must survive.
    #[test]
    fn without_fault_latch_hides_speculative_reads_but_keeps_a_real_one() {
        let m = Memory::new(sample_story(3)).unwrap();
        let past_end = m.len() as u32 + 16;

        // Baseline: an out-of-bounds read normally latches.
        assert_eq!(m.read_word(past_end), 0);
        assert!(m.take_mem_fault().is_some(), "an unguarded OOB read must latch");

        // Guarded: same read, nothing latched.
        m.without_fault_latch(|| m.read_word(past_end));
        assert_eq!(m.take_mem_fault(), None, "a guarded OOB read must not latch");

        // A real fault outstanding when the guard runs is not swallowed.
        let _ = m.read_word(past_end);
        m.without_fault_latch(|| m.read_word(past_end + 32));
        assert_eq!(
            m.take_mem_fault(),
            Some((false, 2, past_end)),
            "the story's own fault must survive a guarded block, unchanged"
        );
    }

    #[test]
    fn unpacks_addresses_per_version() {
        assert_eq!(Memory::new(sample_story(3)).unwrap().unpack_routine(0x0100), 0x0200);
        assert_eq!(Memory::new(sample_story(5)).unwrap().unpack_routine(0x0100), 0x0400);
        assert_eq!(Memory::new(sample_story(8)).unwrap().unpack_routine(0x0100), 0x0800);
    }

    #[test]
    fn unpacks_string_addresses_per_version() {
        assert_eq!(Memory::new(sample_story(3)).unwrap().unpack_string(0x0100), 0x0200);
        assert_eq!(Memory::new(sample_story(5)).unwrap().unpack_string(0x0100), 0x0400);
        assert_eq!(Memory::new(sample_story(8)).unwrap().unpack_string(0x0100), 0x0800);
    }

    #[test]
    fn unpack_v6_uses_offsets() {
        let mut buf = sample_story(6);
        buf[0x28] = 0x00; buf[0x29] = 0x10; // routines_offset = 0x0010
        buf[0x2A] = 0x00; buf[0x2B] = 0x20; // strings_offset  = 0x0020
        let m = Memory::new(buf).unwrap();
        assert_eq!(m.unpack_routine(0x0100), 4 * 0x0100 + 8 * 0x0010);
        assert_eq!(m.unpack_string(0x0100), 4 * 0x0100 + 8 * 0x0020);
    }

    #[test]
    fn rejects_truncated_static_base() {
        let mut buf = crate::header::tests_support::sample_story(3);
        buf.truncate(64);
        let err = Memory::new(buf).unwrap_err();
        assert!(matches!(err, ZError::Truncated));
    }

    #[test]
    fn oob_read_latches_fault_instead_of_panicking() {
        let m = Memory::new(sample_story(3)).unwrap();
        let end = m.len() as u32;
        // Previously panicked (unchecked slice index); must now return 0 + latch.
        let v = m.read_word(end + 100);
        assert_eq!(v, 0, "OOB read returns benign 0");
        assert_eq!(m.take_mem_fault(), Some((false, 2, end + 100)));
        // Latch is drained by take.
        assert_eq!(m.take_mem_fault(), None);
    }

    #[test]
    fn oob_write_latches_fault() {
        let mut m = Memory::new(sample_story(3)).unwrap();
        let end = m.len() as u32;
        m.write_byte(end + 5, 0xAB);
        assert_eq!(m.take_mem_fault(), Some((true, 1, end + 5)));
    }

    #[test]
    fn write_to_static_memory_latches_fault_and_leaves_memory_unchanged() {
        // ZMSD §1.1: only dynamic memory (below static_mem_base) is writable.
        // This was a debug_assert — debug builds panicked on an illegal story
        // write and release builds silently mutated memory that Quetzal CMem
        // never saves. Now a real runtime check latches a fault and skips the
        // write in every build. (SQ-0622)
        let mut buf = sample_story(3); // static_mem_base = 0x0400
        buf.resize(0x500, 0xEE);       // give the file real static memory
        let mut m = Memory::new(buf).unwrap();

        m.write_byte(0x450, 0x12);
        assert_eq!(m.take_mem_fault(), Some((true, 1, 0x450)), "byte write faults");
        assert_eq!(m.read_byte(0x450), 0xEE, "static memory untouched");

        // A word straddling the boundary must not half-corrupt static memory.
        m.write_word(0x3FF, 0xABCD);
        assert_eq!(m.take_mem_fault(), Some((true, 2, 0x3FF)), "straddling word write faults");
        assert_eq!(m.read_byte(0x3FF), 0x00, "dynamic half untouched");
        assert_eq!(m.read_byte(0x400), 0xEE, "static half untouched");

        // Writes below the boundary still work.
        m.write_word(0x3F0, 0xBEEF);
        assert_eq!(m.take_mem_fault(), None);
        assert_eq!(m.read_word(0x3F0), 0xBEEF);
    }

    #[test]
    fn first_fault_wins() {
        let m = Memory::new(sample_story(3)).unwrap();
        let end = m.len() as u32;
        let _ = m.read_byte(end + 1);
        let _ = m.read_byte(end + 2);
        assert_eq!(m.take_mem_fault(), Some((false, 1, end + 1)), "keep first fault addr");
    }
}
