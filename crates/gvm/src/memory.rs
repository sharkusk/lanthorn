// Glulx memory model — GLULX_NOTES.md §2.
//
// The full ENDMEM span is allocated as a `Vec<u8>`. `[0, EXTSTART)` is copied
// from the image; `[EXTSTART, ENDMEM)` is zero-initialized. All multi-byte
// accesses are big-endian. Every access is bounds-checked: out-of-range reads
// return `None` and out-of-range writes return `Err`, so malformed programs
// never panic.

use crate::error::GError;
use crate::header::{parse_header, Header};

/// Why a write was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum WriteFault {
    /// The target address is below RAMSTART (ROM). The write is a no-op.
    Rom,
    /// The target address is outside the current memory map.
    OutOfRange,
}

/// Glulx main memory: a flat big-endian byte array sized to the current ENDMEM.
#[derive(Debug)]
pub struct Memory {
    bytes: Vec<u8>,
    header: Header,
    /// The original ENDMEM, the floor below which `set_mem_size` cannot shrink.
    endmem_floor: u32,
    /// The original loaded image `[0, EXTSTART)`. Used as the diff base for
    /// `CMem` save-state compression and as the reset image on restore.
    orig: Vec<u8>,
}

impl Memory {
    /// Load a Glulx image: validate the header, allocate the full ENDMEM span
    /// (image bytes for `[0, EXTSTART)`, zeros for `[EXTSTART, ENDMEM)`).
    pub fn new(image: Vec<u8>) -> Result<Memory, GError> {
        let header = parse_header(&image)?;
        let endmem = header.endmem as usize;
        let extstart = header.extstart as usize;
        let mut bytes = vec![0u8; endmem];
        // Copy the stored initial memory [0, EXTSTART). The header check
        // guaranteed image.len() >= EXTSTART.
        bytes[..extstart].copy_from_slice(&image[..extstart]);
        let orig = image[..extstart].to_vec();
        Ok(Memory { bytes, header, endmem_floor: header.endmem, orig })
    }

    // ── header field accessors ────────────────────────────────────────────────

    /// First writable address (end of ROM).
    pub fn ramstart(&self) -> u32 {
        self.header.ramstart
    }
    /// End of the stored initial memory in the image.
    pub fn extstart(&self) -> u32 {
        self.header.extstart
    }
    /// The original ENDMEM (the `set_mem_size` floor).
    pub fn endmem(&self) -> u32 {
        self.endmem_floor
    }
    /// Stack size requested by the program.
    pub fn stack_size(&self) -> u32 {
        self.header.stack_size
    }
    /// Address of the start function.
    pub fn start_func(&self) -> u32 {
        self.header.start_func
    }
    /// String-decoding-table address (0 = none).
    pub fn decode_table(&self) -> u32 {
        self.header.decode_table
    }
    /// The whole-image checksum word stored in the header (bytes 0x20-0x23 —
    /// GLULX_NOTES.md §"Header field layout" / Glulx spec §1.4: "the sum of the
    /// whole initial memory as 32-bit ints"), exactly as the compiler wrote it.
    /// Not itself a validity check — see [`Self::checksum_ok`] for that — just
    /// the raw field, which a host can use as part of an identity for the image
    /// that produced a piece of state it persists across runs (e.g. lanthorn's
    /// Glulx room-lock sidecar, SQ-1305: a story rebuilt under the same
    /// filename gets a different checksum, so stale host state keyed to the old
    /// build is never mistaken for the new one's).
    pub fn checksum(&self) -> u32 {
        self.header.checksum
    }

    // ── size ──────────────────────────────────────────────────────────────────

    /// The current ENDMEM (memory-map size in bytes).
    pub fn mem_size(&self) -> u32 {
        self.bytes.len() as u32
    }

    /// Resize the memory map. `new` must be a multiple of 256 and not less than
    /// the original ENDMEM floor. Growth is zero-filled; shrink truncates.
    /// Returns `true` on success, `false` if the request is illegal.
    pub fn set_mem_size(&mut self, new: u32) -> bool {
        if !new.is_multiple_of(256) || new < self.endmem_floor {
            return false;
        }
        self.bytes.resize(new as usize, 0);
        true
    }

    /// Resize the byte map directly, bypassing the `set_mem_size`
    /// alignment/floor checks. The allocation heap (which manages the memory
    /// map itself) uses this; growth is zero-filled, shrink truncates.
    pub(crate) fn set_raw_size(&mut self, new: u32) {
        self.bytes.resize(new as usize, 0);
    }

    /// The original loaded byte at `addr` — the image as it was at load time,
    /// extended with zeros at and above EXTSTART. This is the diff base for
    /// `CMem` save-state compression and the reset image for restore.
    pub(crate) fn orig_byte(&self, addr: u32) -> u8 {
        self.orig.get(addr as usize).copied().unwrap_or(0)
    }

    /// The whole current memory map as one slice — the fast path for whole-RAM
    /// operations (`CMem` compression), where a bounds-checked `read8` per byte
    /// is pure call overhead (SQ-1177).
    pub(crate) fn raw_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The original image `[0, EXTSTART)` as one slice — the diff base
    /// [`Self::orig_byte`] reads one byte at a time. Bytes at and above EXTSTART
    /// are zero by definition and are NOT in this slice.
    pub(crate) fn orig_bytes(&self) -> &[u8] {
        &self.orig
    }

    /// Write one byte directly, bypassing the ROM/range checks. The caller
    /// guarantees `addr` is in range; used only by save-state restore to lay
    /// down the reconstructed RAM image.
    pub(crate) fn write_byte_raw(&mut self, addr: u32, val: u8) {
        self.bytes[addr as usize] = val;
    }

    /// Reset RAM to the initial image: restore `[RAMSTART, EXTSTART)` from
    /// `orig`, zero `[EXTSTART, ENDMEM)`, and shrink back to the original
    /// ENDMEM floor. Used by the `@restart` opcode.
    pub(crate) fn reset_ram(&mut self) {
        let ramstart = self.header.ramstart as usize;
        let extstart = self.header.extstart as usize;
        let endmem = self.endmem_floor as usize;
        self.bytes.resize(endmem, 0);
        self.bytes[ramstart..extstart].copy_from_slice(&self.orig[ramstart..extstart]);
        self.bytes[extstart..endmem].fill(0);
    }

    /// True if the stored header checksum (field 0x20) matches the sum of the
    /// initial memory as big-endian 32-bit words, with the checksum field itself
    /// treated as zero (spec §1.4). Backs the `verify` opcode.
    pub(crate) fn checksum_ok(&self) -> bool {
        let mut sum = 0u32;
        let mut a = 0;
        while a + 4 <= self.orig.len() {
            let mut w = u32::from_be_bytes([
                self.orig[a],
                self.orig[a + 1],
                self.orig[a + 2],
                self.orig[a + 3],
            ]);
            if a == 0x20 {
                w = 0; // the checksum field is excluded from its own sum
            }
            sum = sum.wrapping_add(w);
            a += 4;
        }
        sum == self.header.checksum
    }

    // ── reads (big-endian, bounds-checked) ────────────────────────────────────

    /// Read one byte; `None` if `addr` is out of range.
    pub fn read8(&self, addr: u32) -> Option<u32> {
        self.bytes.get(addr as usize).map(|&b| b as u32)
    }

    /// Read a big-endian 16-bit value; `None` if out of range.
    pub fn read16(&self, addr: u32) -> Option<u32> {
        let a = addr as usize;
        let s = self.bytes.get(a..a + 2)?;
        Some(((s[0] as u32) << 8) | s[1] as u32)
    }

    /// Read a big-endian 32-bit value; `None` if out of range.
    pub fn read32(&self, addr: u32) -> Option<u32> {
        let a = addr as usize;
        let s = self.bytes.get(a..a + 4)?;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    // ── writes (big-endian, bounds- and ROM-checked) ──────────────────────────

    /// Write one byte. ROM (`addr < RAMSTART`) writes are a no-op `Err(Rom)`;
    /// out-of-range writes are `Err(OutOfRange)`.
    pub fn write8(&mut self, addr: u32, v: u32) -> Result<(), WriteFault> {
        self.check_write(addr, 1)?;
        self.bytes[addr as usize] = v as u8;
        Ok(())
    }

    /// Write a big-endian 16-bit value.
    pub fn write16(&mut self, addr: u32, v: u32) -> Result<(), WriteFault> {
        self.check_write(addr, 2)?;
        let a = addr as usize;
        self.bytes[a] = (v >> 8) as u8;
        self.bytes[a + 1] = v as u8;
        Ok(())
    }

    /// Write a big-endian 32-bit value.
    pub fn write32(&mut self, addr: u32, v: u32) -> Result<(), WriteFault> {
        self.check_write(addr, 4)?;
        let a = addr as usize;
        self.bytes[a..a + 4].copy_from_slice(&v.to_be_bytes());
        Ok(())
    }

    /// Validate a write of `len` bytes at `addr`: in-range and at/above RAMSTART.
    fn check_write(&self, addr: u32, len: u32) -> Result<(), WriteFault> {
        let end = addr as u64 + len as u64;
        if end > self.bytes.len() as u64 {
            return Err(WriteFault::OutOfRange);
        }
        if addr < self.header.ramstart {
            return Err(WriteFault::Rom);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm;

    #[test]
    fn valid_header_loads_and_exposes_fields() {
        // RAMSTART=0x100, EXTSTART=0x100, ENDMEM=0x200, stack=0x400,
        // start=0x40, decode=0.
        let img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        let mem = Memory::new(img).expect("valid image");
        assert_eq!(mem.ramstart(), 0x100);
        assert_eq!(mem.extstart(), 0x100);
        assert_eq!(mem.endmem(), 0x200);
        assert_eq!(mem.stack_size(), 0x400);
        assert_eq!(mem.start_func(), 0x40);
        assert_eq!(mem.decode_table(), 0);
        assert_eq!(mem.mem_size(), 0x200);
    }

    #[test]
    fn bad_magic_rejected() {
        let mut img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        img[0] = b'X';
        assert_eq!(Memory::new(img).unwrap_err(), GError::BadMagic);
    }

    #[test]
    fn unknown_major_version_rejected() {
        let mut img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        // Set major version to 4.
        img[0x04..0x08].copy_from_slice(&0x0004_0000u32.to_be_bytes());
        assert_eq!(Memory::new(img).unwrap_err(), GError::UnsupportedVersion(0x0004_0000));
    }

    #[test]
    fn too_short_rejected() {
        assert_eq!(Memory::new(vec![b'G', b'l', b'u', b'l']).unwrap_err(), GError::TooShort);
    }

    #[test]
    fn misaligned_map_rejected() {
        // EXTSTART not a multiple of 256.
        let img = asm::image_with_map(0x100, 0x180, 0x200, 0x400, 0x40, 0);
        // 0x180 IS a multiple of 0x80 but not 0x100 → 0x180 % 256 = 128 ≠ 0.
        assert_eq!(Memory::new(img).unwrap_err(), GError::BadMemoryMap);
    }

    #[test]
    fn ext_zone_reads_back_zero() {
        // [EXTSTART, ENDMEM) is zero-initialized even though the file only
        // stores [0, EXTSTART).
        let img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        let mem = Memory::new(img).unwrap();
        assert_eq!(mem.read32(0x100), Some(0));
        assert_eq!(mem.read32(0x1FC), Some(0));
    }

    #[test]
    fn out_of_range_read_is_none_not_panic() {
        let img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        let mem = Memory::new(img).unwrap();
        assert_eq!(mem.read32(0x1FE), None); // 0x1FE+4 > 0x200
        assert_eq!(mem.read8(0x200), None);
    }

    #[test]
    fn rom_write_is_noop_fault() {
        let img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        let mut mem = Memory::new(img).unwrap();
        let before = mem.read8(0x00);
        assert_eq!(mem.write8(0x00, 0xAB), Err(WriteFault::Rom));
        assert_eq!(mem.read8(0x00), before); // unchanged
    }

    #[test]
    fn ram_write_then_read_roundtrips() {
        let img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        let mut mem = Memory::new(img).unwrap();
        mem.write32(0x100, 0xDEAD_BEEF).unwrap();
        assert_eq!(mem.read32(0x100), Some(0xDEAD_BEEF));
        mem.write16(0x110, 0x1234).unwrap();
        assert_eq!(mem.read16(0x110), Some(0x1234));
    }

    #[test]
    fn out_of_range_write_faults() {
        let img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        let mut mem = Memory::new(img).unwrap();
        assert_eq!(mem.write32(0x1FE, 1), Err(WriteFault::OutOfRange));
    }

    #[test]
    fn set_mem_size_grows_zeroed_and_shrinks_to_floor() {
        let img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        let mut mem = Memory::new(img).unwrap();
        // Grow by 256, new bytes zeroed.
        assert!(mem.set_mem_size(0x300));
        assert_eq!(mem.mem_size(), 0x300);
        assert_eq!(mem.read32(0x280), Some(0));
        // Shrink back to the floor.
        assert!(mem.set_mem_size(0x200));
        assert_eq!(mem.mem_size(), 0x200);
    }

    #[test]
    fn set_mem_size_rejects_below_floor_and_unaligned() {
        let img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        let mut mem = Memory::new(img).unwrap();
        assert!(!mem.set_mem_size(0x100)); // below 0x200 floor
        assert!(!mem.set_mem_size(0x250)); // not 256-aligned
        assert_eq!(mem.mem_size(), 0x200); // unchanged after failures
    }

    #[test]
    fn reset_ram_restores_initial_state() {
        // RAMSTART=0x100, EXTSTART=0x100 (so all RAM is in the ext zone, zeroed).
        let img = asm::image_with_map(0x100, 0x100, 0x200, 0x400, 0x40, 0);
        let mut mem = Memory::new(img).unwrap();
        // Grow the map, then write some values into RAM.
        assert!(mem.set_mem_size(0x300));
        mem.write32(0x100, 0xDEAD_BEEF).unwrap();
        mem.write32(0x280, 0x1234_5678).unwrap();
        assert_eq!(mem.mem_size(), 0x300);
        // reset_ram should shrink back to 0x200 and zero out RAM.
        mem.reset_ram();
        assert_eq!(mem.mem_size(), 0x200, "shrinks to endmem_floor");
        assert_eq!(mem.read32(0x100).unwrap(), 0, "RAM zeroed at RAMSTART");
        assert_eq!(mem.read32(0x1FC).unwrap(), 0, "last word in RAM zeroed");
        assert_eq!(mem.read8(0x280), None, "grown region removed");
    }
}
