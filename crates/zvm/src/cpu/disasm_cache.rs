//! Disassembly cache: an ordered model of the code region as display units.
//!
//! Task 1 scope only: the `Unit`/`DisasmCache` data model, code-region bounds,
//! and an empty-cache skeleton constructor. Routine discovery (populating
//! `units`/`routines`) lands in later tasks.

use crate::cpu::decode::{decode, Operand, OperandCount};
use crate::cpu::disasm::{format_instr, format_instr_basic, format_instr_raw, mnemonic, Unpack};
use crate::memory::Memory;
use std::collections::{BTreeSet, HashSet};

/// Static confidence provenance of a code unit (SQ-0428): where the
/// disassembler's classification of these bytes came from. `Data` bytes are
/// never code; a code unit is `Rd` when its routine entry is hard (recursive
/// descent from a constant call target, the initial PC, or execution-confirmed)
/// and `Soft` when it came only from the linear scan (an unverified guess).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Provenance {
    /// Hard: RD-discovered / initial-PC / execution-confirmed code.
    Rd,
    /// Soft: a linear-scan guess, not yet verified by execution.
    Soft,
    /// Not code: an opaque `.byte` run (`Unit::Data`).
    Data,
}

/// A single displayable unit within the disassembled code region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Unit {
    /// A decoded instruction spanning `[addr, next)`.
    Instr { addr: u32, next: u32, prov: Provenance },
    /// A routine header spanning `[addr, first_instr)` (ZMSD §5.2: one byte of
    /// local count, then `nlocals` initial-value words in v1-4).
    RoutineHeader { addr: u32, nlocals: u8, first_instr: u32, prov: Provenance },
    /// An opaque data run spanning `[addr, addr+len)` (not decoded as code).
    Data { addr: u32, len: u32 },
}

impl Unit {
    /// Start address of this unit.
    pub fn addr(&self) -> u32 {
        match *self {
            Unit::Instr { addr, .. } => addr,
            Unit::RoutineHeader { addr, .. } => addr,
            Unit::Data { addr, .. } => addr,
        }
    }

    /// One-past-the-end address (exclusive). `RoutineHeader` occupies
    /// `[addr, first_instr)`.
    pub fn end(&self) -> u32 {
        match *self {
            Unit::Instr { next, .. } => next,
            Unit::RoutineHeader { first_instr, .. } => first_instr,
            Unit::Data { addr, len } => addr + len,
        }
    }

    /// This unit's static confidence [`Provenance`]. `Data` units are always
    /// `Provenance::Data`; code units carry the tag set at build/confirm time.
    pub fn provenance(&self) -> Provenance {
        match *self {
            Unit::Instr { prov, .. } => prov,
            Unit::RoutineHeader { prov, .. } => prov,
            Unit::Data { .. } => Provenance::Data,
        }
    }
}

/// Rendering detail level for a cached unit (mirrors `disasm`'s
/// full/basic/raw views).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum CacheFmt {
    Full,
    Basic,
    Raw,
}

/// Ordered cache of the code region, tiled by `Unit`s with routine entry
/// points tracked separately for fast lookup.
pub struct DisasmCache {
    /// Sorted by `addr()`, tiling `[region_start, region_end)` with no gaps
    /// once discovery (later tasks) has populated it.
    units: Vec<Unit>,
    /// Routine ENTRY (header) addresses, populated by discovery.
    routines: std::collections::BTreeSet<u32>,
    region_start: u32,
    region_end: u32,
    version: u8,
    unpack: Unpack,
}

impl DisasmCache {
    /// Build an empty cache with region bounds computed but no units
    /// discovered yet. Later tasks populate `units`/`routines`.
    pub fn empty(mem: &Memory) -> DisasmCache {
        let (region_start, region_end) = code_region(mem);
        DisasmCache {
            units: Vec::new(),
            routines: Default::default(),
            region_start,
            region_end,
            version: mem.version(),
            unpack: Unpack::from_mem(mem),
        }
    }

    /// Build the RD-discovered cache: discover routines, then tile the code
    /// region `[region_start, region_end)` into sorted, gapless `Unit`s.
    ///
    /// Code starts are the discovered routine entries plus the headerless
    /// initial-PC "main" context. Untiled bytes before the next code start
    /// become a single `Data` run. At a routine entry a `RoutineHeader` is
    /// emitted (the main context has none); then instructions are decoded
    /// linearly up to the next code start (or `region_end`), with the final
    /// instruction truncated so it never crosses that boundary.
    pub fn build(mem: &Memory) -> DisasmCache {
        let (rstart, rend) = code_region(mem);
        let version = mem.version();
        let unpack = Unpack::from_mem(mem);
        let rd = discover_rd(mem, version, &unpack, (rstart, rend));
        let extra = discover_linear(mem, version, (rstart, rend), &rd);
        let routines: BTreeSet<u32> = rd.union(&extra).copied().collect();

        // Code starts to tile from: routine entries (which carry a header) plus
        // the headerless initial-PC main context. Kept as one sorted set so
        // each run's extent is bounded by the next code start of either kind.
        //
        // v6's `main` is a real ROUTINE (packed header-0x06 address), already
        // discovered by `discover_rd` and present in `rd`/`routines` — so it
        // needs no extra headerless start. Only v1-5/7/8, where header 0x06 is a
        // direct PC into a headerless main, inject `initial_pc` here.
        let mut starts: BTreeSet<u32> = routines.clone();
        // Truncation boundaries are HARD: only real RD routine entries (reached
        // by a constant call) and the initial-PC main context force an
        // instruction to end. Linear-scan entries (`extra`) are SOFT — honoured
        // only where the instruction stream lands on them exactly. A soft
        // boundary an instruction straddles is a false positive (e.g. a call's
        // store byte, `0x01`, misread as a 1-local routine header) and is
        // dropped so it can never split a real instruction.
        let mut hard: BTreeSet<u32> = rd.clone();
        if version != 6 {
            let initial_pc = mem.read_word(0x06) as u32;
            if initial_pc >= rstart && initial_pc < rend {
                starts.insert(initial_pc);
                hard.insert(initial_pc);
            }
        }

        let mut units: Vec<Unit> = Vec::new();
        let mut cur = rstart;
        while cur < rend {
            // Smallest code start >= cur, else region end.
            let next_start = starts.range(cur..).next().copied().unwrap_or(rend);

            if cur < next_start {
                // Data gap before the next code start.
                units.push(Unit::Data { addr: cur, len: next_start - cur });
                cur = next_start;
                continue;
            }

            // cur == next_start == a code start. Routine entries carry a
            // header; the main context (initial_pc) does not.
            let start = cur;
            // This run's provenance: HARD (RD entry / initial-PC main context)
            // is `Rd`, a linear-scan-only (`extra`) entry is `Soft`. The header
            // and every instruction of the run share the run's provenance.
            let run_prov = if hard.contains(&start) { Provenance::Rd } else { Provenance::Soft };
            if routines.contains(&start) {
                let nlocals = mem.read_byte(start);
                let first_instr = routine_first_instr(mem, start, version).min(rend);
                units.push(Unit::RoutineHeader { addr: start, nlocals, first_instr, prov: run_prov });
                cur = first_instr;
            }

            // This run decodes forward, truncating only at the next HARD
            // boundary (or region end); it flows across soft (linear-only)
            // boundaries. Landing exactly on a soft boundary ends the run so the
            // outer loop emits that routine's header next; a straddled soft
            // boundary is skipped (dropped), so it can't split an instruction.
            let hard_limit = hard
                .range((start + 1)..)
                .next()
                .copied()
                .unwrap_or(rend)
                .min(rend);

            while cur < hard_limit {
                let instr = decode(mem, cur, version);
                let mut next = if instr.next_pc > cur { instr.next_pc } else { cur + 1 };
                // Boundary wins only at a HARD boundary / region end.
                if next > hard_limit {
                    next = hard_limit;
                }
                units.push(Unit::Instr { addr: cur, next, prov: run_prov });
                cur = next;
                // Stop at an aligned soft boundary so its header is emitted next —
                // but ONLY when the instruction that reached it is a block
                // terminator. A real routine header follows a control-flow break
                // (ret/jump/…), never fall-through; a soft boundary reached by
                // fall-through is mid-code (a linear-scan false positive — a byte
                // ≤15 that looked like a locals count), so decode straight through
                // it instead of emitting a phantom routine header.
                if extra.contains(&cur) && is_terminator(instr.operand_count.clone(), instr.opcode) {
                    break;
                }
            }
        }

        DisasmCache {
            units,
            routines,
            region_start: rstart,
            region_end: rend,
            version,
            unpack,
        }
    }

    /// Code-region bounds this cache was built for: `(region_start, region_end)`.
    pub fn region(&self) -> (u32, u32) {
        (self.region_start, self.region_end)
    }

    /// The cached units, in address order. Empty until discovery (later tasks)
    /// populates it.
    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// Routine entry (header) addresses discovered so far.
    pub fn routines(&self) -> &std::collections::BTreeSet<u32> {
        &self.routines
    }

    /// Z-machine version this cache was built for.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// Packed-address unpacking context this cache was built for.
    pub fn unpack(&self) -> &Unpack {
        &self.unpack
    }

    /// Index of the unit containing `addr`. Clamps to `0` if `addr` is
    /// before the first unit, and to the last index if `addr` is at or past
    /// the last unit's start.
    ///
    /// `None` when there are no units, which is not hypothetical: a story whose
    /// header `$04`/`$06` point past EOF gives `region_start == region_end`, so
    /// `build` tiles nothing — and [`DisasmCache::empty`] constructs exactly that
    /// object on purpose. The old signature returned `usize` and computed
    /// `self.units.len() - 1`: a debug build stopped at the `debug_assert!` that
    /// stood here, and a RELEASE build wrapped the subtraction to `usize::MAX`,
    /// clamped it back to `0`, and indexed an empty vector (SQ-1030). An `Option`
    /// is the honest answer — there is no unit at any address, because there are
    /// no units.
    fn unit_index_at(&self, addr: u32) -> Option<usize> {
        let last = self.units.len().checked_sub(1)?;
        let pp = self.units.partition_point(|u| u.addr() <= addr);
        Some(pp.saturating_sub(1).min(last))
    }

    /// Start address of the unit strictly after the one containing `addr`.
    /// Clamps to the last unit's start (no movement past the end).
    pub fn next_addr(&self, addr: u32) -> u32 {
        let Some(i) = self.unit_index_at(addr) else { return addr }; // no units: nowhere to move
        if i + 1 < self.units.len() {
            self.units[i + 1].addr()
        } else {
            self.units[i].addr()
        }
    }

    /// Start address of the unit strictly before the one containing `addr`.
    /// Clamps to `region_start` (the first unit's start).
    pub fn prev_addr(&self, addr: u32) -> u32 {
        let Some(i) = self.unit_index_at(addr) else { return addr }; // no units: nowhere to move
        if i > 0 {
            self.units[i - 1].addr()
        } else {
            self.units[0].addr()
        }
    }

    /// Format up to `lines` display rows starting at the unit at/after `addr`.
    ///
    /// Rendering always begins at the containing unit's own start (the whole
    /// unit is drawn even if `addr` falls mid-unit), and walks forward emitting
    /// rows until `lines` rows are produced or the units are exhausted. Instr
    /// units decode on demand and match the three legacy `disasm` formatters
    /// byte-for-byte; a `Data` run emits one `.byte` row per 16 bytes, each row
    /// counting as one line (so a large run can fill the window and stop
    /// mid-run).
    pub fn disassemble(&self, mem: &Memory, addr: u32, lines: usize, fmt: CacheFmt) -> Vec<String> {
        self.disassemble_tiered(mem, addr, lines, fmt)
            .into_iter()
            .map(|(row, _prov)| row)
            .collect()
    }

    /// Like [`disassemble`](Self::disassemble), but tags each display row with
    /// the [`Provenance`] of the unit it came from (SQ-0428). The row strings are
    /// byte-identical to [`disassemble`]; `Unit::Data`'s multiple `.byte` rows
    /// each carry `Provenance::Data`.
    pub fn disassemble_tiered(
        &self,
        mem: &Memory,
        addr: u32,
        lines: usize,
        fmt: CacheFmt,
    ) -> Vec<(String, Provenance)> {
        let mut out = Vec::with_capacity(lines);
        if self.units.is_empty() || lines == 0 {
            return out;
        }
        let Some(mut i) = self.unit_index_at(addr) else { return out };
        while i < self.units.len() && out.len() < lines {
            let prov = self.units[i].provenance();
            match self.units[i] {
                Unit::Instr { addr, .. } => {
                    let instr = decode(mem, addr, self.version);
                    let row = match fmt {
                        CacheFmt::Full => {
                            format!("{:06x}  {}", addr, format_instr(&instr, &self.unpack))
                        }
                        CacheFmt::Basic => {
                            format!("{:06x}  {}", addr, format_instr_basic(&instr, self.version))
                        }
                        CacheFmt::Raw => {
                            let end = instr.next_pc.min(mem.len() as u32);
                            let n = end.saturating_sub(addr).min(12);
                            let truncated = end.saturating_sub(addr) > 12;
                            let bytes: Vec<u8> = (0..n).map(|k| mem.read_byte(addr + k)).collect();
                            format!("{:06x}: {}", addr, format_instr_raw(&instr, &bytes, truncated))
                        }
                    };
                    out.push((row, prov));
                }
                Unit::RoutineHeader { addr, nlocals, first_instr, .. } => match fmt {
                    // Raw mode shows the untranslated header bytes (no `;` comment
                    // or decoded local count), matching the raw-instruction style.
                    CacheFmt::Raw => {
                        let end = first_instr.min(addr + 12);
                        let mut hex = (addr..end)
                            .map(|a| format!("{:02x}", mem.read_byte(a)))
                            .collect::<Vec<_>>()
                            .join(" ");
                        if first_instr.saturating_sub(addr) > 12 {
                            hex.push_str(" …");
                        }
                        out.push((format!("{:06x}: {}   routine", addr, hex), prov));
                    }
                    CacheFmt::Full | CacheFmt::Basic => {
                        let plural = if nlocals == 1 { "local" } else { "locals" };
                        out.push((format!("{:06x}  ; routine, {} {}", addr, nlocals, plural), prov));
                    }
                },
                Unit::Data { addr, len } => {
                    let end = addr + len;
                    let mut row_addr = addr;
                    while row_addr < end && out.len() < lines {
                        let row_end = (row_addr + 16).min(end);
                        let hexbytes = (row_addr..row_end)
                            .map(|a| format!("{:02x}", mem.read_byte(a)))
                            .collect::<Vec<_>>()
                            .join(" ");
                        out.push((format!("{:06x}  .byte {}", row_addr, hexbytes), prov));
                        row_addr += 16;
                    }
                }
            }
            i += 1;
        }
        out
    }

    /// A confirmed instruction start (a PC the VM executed). If the cache
    /// disagrees — `pc` lands inside a `Data` unit, or mid-instruction (not at
    /// an existing unit boundary) — re-anchor at `pc` and re-decode forward,
    /// replacing the overlapped unit locally so `pc` becomes an `Instr` unit
    /// boundary. Returns true iff the cache changed. A `pc` already at an
    /// `Instr` unit boundary is a no-op (returns false). `pc` outside
    /// `[region_start, region_end)` is ignored (returns false).
    ///
    /// Observed execution is ground truth: it overrides any static
    /// classification of those bytes as data or a differently-aligned
    /// instruction.
    pub fn confirm_pc(&mut self, mem: &Memory, pc: u32) -> bool {
        if pc < self.region_start || pc >= self.region_end {
            return false;
        }
        let Some(i) = self.unit_index_at(pc) else { return false };
        if matches!(self.units[i], Unit::Instr { .. }) && self.units[i].addr() == pc {
            return false; // already an Instr boundary
        }
        // Re-decode forward from `pc`, ABSORBING any following units the decoded
        // instruction stream overlaps, until it re-syncs with an existing unit
        // boundary at or beyond the containing unit's end. A confirmed instruction
        // can be LONGER than the stale unit `pc` landed in — e.g. a `read` (4 bytes)
        // misclassified as a shorter op whose length ate the read's opcode byte, so
        // `pc` lands inside a 2-byte unit. Truncating the re-decode to that unit's
        // end would strand a phantom fragment (the old bug); absorbing the overlapped
        // units instead lets the real instruction span its full extent.
        let lo = self.units[i].addr();
        let start_hi = self.units[i].end();
        let mut new_instrs: Vec<Unit> = Vec::new();
        let mut cur = pc;
        loop {
            let instr = decode(mem, cur, self.version);
            let next = if instr.next_pc > cur { instr.next_pc } else { cur + 1 };
            let next = next.min(self.region_end);
            // Execution-confirmed bytes are ground truth → hard `Rd` provenance.
            new_instrs.push(Unit::Instr { addr: cur, next, prov: Provenance::Rd });
            cur = next;
            // Re-synced: past the original unit end and landed on an existing
            // boundary, so the tiling is already consistent from here on.
            if cur >= start_hi && self.is_unit_boundary(cur) {
                break;
            }
            if cur >= self.region_end {
                break;
            }
        }
        let end_excl = if cur >= self.region_end {
            self.units.len()
        } else {
            self.unit_index_at(cur).unwrap_or(self.units.len())
        };
        let mut new_units: Vec<Unit> = Vec::new();
        if pc > lo {
            new_units.push(Unit::Data { addr: lo, len: pc - lo });
        }
        new_units.extend(new_instrs);
        self.units.splice(i..end_excl, new_units);
        #[cfg(debug_assertions)]
        self.debug_check_tiling();
        true
    }

    /// True if `addr` is an existing unit start (or exactly `region_end`).
    fn is_unit_boundary(&self, addr: u32) -> bool {
        addr == self.region_end
            || self.unit_index_at(addr).is_some_and(|i| self.units[i].addr() == addr)
    }

    /// A confirmed routine ENTRY (a call-stack `func_addr` — the strongest
    /// signal). Ensures a `RoutineHeader` unit at `entry` and an `Instr` unit
    /// at its first instruction, re-aligning that routine forward. Returns true
    /// iff the units changed. Adds `entry` to `self.routines`.
    pub fn confirm_routine(&mut self, mem: &Memory, entry: u32) -> bool {
        if entry < self.region_start || entry >= self.region_end {
            return false;
        }
        self.routines.insert(entry);

        let first = routine_first_instr(mem, entry, self.version).min(self.region_end);
        let Some(i) = self.unit_index_at(entry) else { return false };
        let lo = self.units[i].addr();
        // Repair span end: the unit containing `first` (so the header AND its
        // first instruction both fit). When `first == region_end` there is no
        // instruction to place; the span ends at the last unit.
        let end_i = if first < self.region_end {
            // `units` is non-empty here — the `unit_index_at(entry)` above returned.
            self.unit_index_at(first).unwrap_or(0)
        } else {
            self.units.len() - 1
        };
        let hi = self.units[end_i].end();

        let mut new_units: Vec<Unit> = Vec::new();
        if entry > lo {
            new_units.push(Unit::Data { addr: lo, len: entry - lo });
        }
        let nlocals = mem.read_byte(entry);
        // A confirmed routine entry is a call-stack `func_addr` — hard `Rd`.
        new_units.push(Unit::RoutineHeader { addr: entry, nlocals, first_instr: first, prov: Provenance::Rd });
        new_units.extend(self.decode_instrs(mem, first, hi, Provenance::Rd));

        // Idempotent: nothing to do if the span already tiles exactly this way.
        if self.units[i..=end_i] == new_units[..] {
            return false;
        }
        self.units.splice(i..=end_i, new_units);
        #[cfg(debug_assertions)]
        self.debug_check_tiling();
        true
    }

    /// Linear decode of `Instr` units over `[from, hi)`, boundary-wins at `hi`
    /// exactly like `build`: each instruction's extent is truncated so it never
    /// crosses `hi`, and a non-advancing decode is forced forward one byte.
    fn decode_instrs(&self, mem: &Memory, from: u32, hi: u32, prov: Provenance) -> Vec<Unit> {
        let mut out = Vec::new();
        let mut cur = from;
        while cur < hi {
            let instr = decode(mem, cur, self.version);
            let mut next = if instr.next_pc > cur { instr.next_pc } else { cur + 1 };
            if next > hi {
                next = hi;
            }
            out.push(Unit::Instr { addr: cur, next, prov });
            cur = next;
        }
        out
    }

    /// Debug-only invariant guard: `units` is non-empty, sorted, gapless, and
    /// exactly tiles `[region_start, region_end)`. Catches splice bugs at the
    /// point of the repair.
    #[cfg(debug_assertions)]
    fn debug_check_tiling(&self) {
        assert!(!self.units.is_empty(), "tiling: empty units after repair");
        assert_eq!(
            self.units[0].addr(),
            self.region_start,
            "tiling: first unit does not start at region_start"
        );
        assert_eq!(
            self.units.last().unwrap().end(),
            self.region_end,
            "tiling: last unit does not end at region_end"
        );
        for w in self.units.windows(2) {
            assert!(
                w[0].addr() < w[1].addr(),
                "tiling: not strictly sorted: {:#x?} !< {:#x?}",
                w[0],
                w[1]
            );
            assert_eq!(
                w[0].end(),
                w[1].addr(),
                "tiling: gap/overlap between {:#x?} and {:#x?}",
                w[0],
                w[1]
            );
        }
    }
}

/// Byte address where the interpreter begins executing (ZMSD §5.5).
///
/// v1-5/7/8: header 0x06 is the *direct* first-instruction PC of a headerless
/// "main" context — returned as-is. v6: header 0x06 is the *packed address of
/// the `main` routine* (a real routine with a locals header); returned unpacked
/// to its byte entry. A v6 disassembler that used the raw word here would root
/// its reachability at a nonsense address (the packed value is ~1/4 the real
/// one) and misclassify the whole region.
pub fn boot_root(mem: &Memory) -> u32 {
    let w = mem.read_word(0x06);
    if mem.version() == 6 {
        Unpack::from_mem(mem).routine(w)
    } else {
        w as u32
    }
}

/// Code-region bounds: `(region_start, region_end)`.
///
/// `region_start` = `min(high_mem_base, boot_root)`; `region_end` =
/// `mem.len()`. Permissive by design — discovery/validation in later tasks
/// reject non-code content within these bounds.
pub fn code_region(mem: &Memory) -> (u32, u32) {
    let high_mem_base = mem.read_word(0x04) as u32;
    let region_end = mem.len() as u32;
    // Clamped to `region_end`, so the region is empty rather than INVERTED when a
    // malformed header points `$04`/`$06` past the end of the file. Every consumer
    // reads this as a half-open `[start, end)`; a start past the end made `build`
    // tile nothing while `region_start > region_end` still described a range, and
    // the emptiness then surfaced far away in `unit_index_at` (SQ-1030).
    let region_start = high_mem_base.min(boot_root(mem)).min(region_end);
    (region_start, region_end)
}

/// First-instruction address of a routine whose header is at `entry`:
/// `entry + 1` (locals-count byte) `+ nlocals*2` initial-value words in v1-4
/// (ZMSD §5.2); v5+ routines carry no initial-value words.
///
/// Consumed by [`discover_rd`] and by tiling in Task 3.
pub(crate) fn routine_first_instr(mem: &Memory, entry: u32, version: u8) -> u32 {
    let nlocals = mem.read_byte(entry) as u32;
    entry + 1 + if version <= 4 { nlocals * 2 } else { 0 }
}

/// Is `(operand_count, opcode)` a `call*` opcode (whose routine operand is
/// operand index 0)? The v1-4 1OP:0x0F encoding is `not`, not `call_1n`, so
/// that case is gated on `version >= 5`.
fn is_call(count: OperandCount, opcode: u8, version: u8) -> bool {
    use OperandCount::*;
    match (count, opcode) {
        (One, 0x08) => true,          // call_1s
        (One, 0x0F) => version >= 5,  // call_1n (v5+); `not` in v1-4
        (Two, 0x19) => true,          // call_2s
        (Two, 0x1A) => true,          // call_2n
        (Var, 0x00) => true,          // call_vs
        (Var, 0x0C) => true,          // call_vs2
        (Var, 0x19) => true,          // call_vn
        (Var, 0x1A) => true,          // call_vn2
        _ => false,
    }
}

/// Does `(operand_count, opcode)` unconditionally end a routine's linear run?
fn is_terminator(count: OperandCount, opcode: u8) -> bool {
    use OperandCount::*;
    match (count, opcode) {
        (One, 0x0B) => true,  // ret
        (One, 0x0C) => true,  // jump (unconditional)
        (Zero, 0x00) => true, // rtrue
        (Zero, 0x01) => true, // rfalse
        (Zero, 0x03) => true, // print_ret
        (Zero, 0x08) => true, // ret_popped
        (Zero, 0x0A) => true, // quit
        _ => false,
    }
}

/// Recursive-descent routine discovery. Seeded from the initial-PC
/// first-instruction address, decode each routine forward and follow every
/// `call*` whose routine operand is a non-zero constant, to a fixpoint.
/// Returns the set of routine ENTRY (header) addresses discovered.
///
/// The initial "main" context has no header, so the seed is enqueued on the
/// first-instruction worklist directly and never added to the routine set.
///
/// Consumed by Task 3 (tiling/nav).
pub fn discover_rd(mem: &Memory, version: u8, unpack: &Unpack, region: (u32, u32)) -> BTreeSet<u32> {
    /// Safety cap on instructions decoded per routine run (guards a malformed
    /// decode loop that never reaches a terminator or the region end).
    const RUN_INSTR_CAP: u32 = 4096;

    let (rstart, rend) = region;
    let mut routines: BTreeSet<u32> = BTreeSet::new();
    let mut visited: HashSet<u32> = HashSet::new();

    // Seed the reachability root. v1-5/7/8: header 0x06 is the direct
    // first-instruction PC of the headerless main context — enqueue it. v6:
    // header 0x06 is the PACKED address of the `main` ROUTINE, which the
    // interpreter calls (ZMSD §5.5); unpack it to the routine entry, record it
    // as a discovered routine, and enqueue its first instruction.
    let mut worklist: Vec<u32> = Vec::new();
    if version == 6 {
        let entry = unpack.routine(mem.read_word(0x06));
        if entry >= rstart && entry < rend {
            let fi = routine_first_instr(mem, entry, version);
            if fi < rend {
                routines.insert(entry);
                worklist.push(fi);
            }
        }
    } else {
        worklist.push(mem.read_word(0x06) as u32);
    }

    while let Some(first) = worklist.pop() {
        if !visited.insert(first) {
            continue;
        }
        let mut pc = first;
        let mut steps = 0u32;
        loop {
            if pc >= rend || steps >= RUN_INSTR_CAP {
                break;
            }
            steps += 1;
            let instr = decode(mem, pc, version);

            if is_call(instr.operand_count.clone(), instr.opcode, version) {
                let target = match instr.operands.first() {
                    Some(Operand::Large(n)) => Some(*n),
                    Some(Operand::Small(n)) => Some(*n as u16),
                    _ => None,
                };
                if let Some(n) = target {
                    if n != 0 {
                        let entry = unpack.routine(n);
                        if entry >= rstart && entry < rend {
                            let fi = routine_first_instr(mem, entry, version);
                            if fi < rend && routines.insert(entry) {
                                worklist.push(fi);
                            }
                        }
                    }
                }
            }

            if is_terminator(instr.operand_count.clone(), instr.opcode) {
                break;
            }
            pc = if instr.next_pc > pc { instr.next_pc } else { pc + 1 };
        }
    }

    routines
}

/// Linear-scan augmentation: scan `region` for routine headers not already in
/// `known`, accepting a candidate only if it validates as real code. Returns
/// the NEW entries found (not already in `known`).
///
/// Finds routines RD misses because they are only ever called indirectly (via
/// object properties, grammar/action tables, or a `call` with a variable
/// operand). Validation — a clean forward decode to a terminator or a known
/// boundary — is what keeps data that happens to look like a header out.
fn discover_linear(
    mem: &Memory,
    version: u8,
    region: (u32, u32),
    known: &std::collections::BTreeSet<u32>,
) -> std::collections::BTreeSet<u32> {
    let (rstart, rend) = region;
    let mut extra: BTreeSet<u32> = BTreeSet::new();

    let mut p = rstart;
    while p < rend {
        // Skip addresses RD already owns (RD wins on conflict).
        if known.contains(&p) {
            p += 1;
            continue;
        }
        // (a) plausible locals-count byte.
        if mem.read_byte(p) > 15 {
            p += 1;
            continue;
        }
        // (b) forward decode from the first instruction must validate cleanly.
        match validate_routine(mem, version, p, region, known) {
            // ACCEPTED: record it and skip past its validated body. Routines
            // do not overlap, so the body cannot hide another real header.
            Some(extent) => {
                extra.insert(p);
                p = extent.max(p + 1);
            }
            // Ambiguous bytes are classified as data, not a routine.
            None => p += 1,
        }
    }

    extra
}

/// Validate a candidate routine header at `entry`: decode forward from its
/// first instruction until a terminator or a `known` routine boundary. Returns
/// the one-past-the-end address of the validated run, or `None` if the run is
/// not clean code (unassigned opcode, non-advancing decode, a read crossing
/// `region.1`, reaching region end without a terminator, or the instruction
/// cap). Errs toward rejection.
fn validate_routine(
    mem: &Memory,
    version: u8,
    entry: u32,
    region: (u32, u32),
    known: &std::collections::BTreeSet<u32>,
) -> Option<u32> {
    /// Safety cap on instructions decoded while validating a candidate; a
    /// cap-hit is treated as INVALID (err toward rejecting).
    const VALIDATE_INSTR_CAP: u32 = 4096;

    let (_rstart, rend) = region;
    let first = routine_first_instr(mem, entry, version);
    if first > rend {
        return None; // header alone crosses the region end
    }

    let mut pc = first;
    let mut steps = 0u32;
    loop {
        if steps >= VALIDATE_INSTR_CAP {
            return None;
        }
        // Clean boundary: the run flowed contiguously up to a known routine.
        if pc > first && known.contains(&pc) {
            return Some(pc);
        }
        // Reached region end without a terminator: reject (err toward data).
        if pc >= rend {
            return None;
        }
        steps += 1;
        let instr = decode(mem, pc, version);

        // Every opcode must be assigned in this version.
        if mnemonic(&instr.operand_count, instr.opcode, version).starts_with("op:") {
            return None;
        }
        // No read may cross the region end.
        if instr.next_pc > rend {
            return None;
        }
        // Decode must strictly advance.
        if instr.next_pc <= pc {
            return None;
        }
        if is_terminator(instr.operand_count.clone(), instr.opcode) {
            return Some(instr.next_pc);
        }
        pc = instr.next_pc;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A story whose header `$04` (high memory) and `$06` (initial PC / packed
    /// `main`) both point past the end of the file. There is nothing malformed
    /// about the LOAD — `Memory::new` accepts it — so the cache is asked to tile a
    /// region that starts past where it ends, and tiles nothing (SQ-1030).
    fn story_with_code_past_eof() -> Memory {
        let mut buf = crate::header::tests_support::sample_story(5);
        buf[0x04] = 0xFF; buf[0x05] = 0xFF; // high_mem_base = 0xFFFF, past EOF
        buf[0x06] = 0xFF; buf[0x07] = 0xFF; // initial PC likewise
        Memory::new(buf).unwrap()
    }

    #[test]
    fn code_region_past_eof_is_empty_not_inverted() {
        let mem = story_with_code_past_eof();
        let (start, end) = code_region(&mem);
        assert_eq!(end, mem.len() as u32);
        assert_eq!(start, end, "a start past EOF clamps to the end, giving an EMPTY region");
        assert!(start <= end, "the region is never inverted");
    }

    /// The release-mode half of SQ-1030: with no units, `unit_index_at` computed
    /// `self.units.len() - 1`. A debug build hit the `debug_assert!` above it; a
    /// release build wrapped to `usize::MAX`, clamped back to `0`, and indexed an
    /// empty vector. Both are a host abort, and both are reachable from a story
    /// file that loads cleanly.
    #[test]
    fn navigating_an_empty_cache_does_not_panic() {
        let mem = story_with_code_past_eof();
        for cache in [DisasmCache::build(&mem), DisasmCache::empty(&mem)] {
            assert!(cache.units().is_empty(), "nothing to tile");
            for addr in [0u32, 0x40, 0xFFFF, u32::MAX] {
                assert_eq!(cache.next_addr(addr), addr, "no unit to move to");
                assert_eq!(cache.prev_addr(addr), addr, "no unit to move to");
                assert!(cache.disassemble(&mem, addr, 8, CacheFmt::Full).is_empty());
            }
        }
    }

    /// The same emptiness reached through the two repair entry points, which an
    /// app-side debugger calls on every step it observes.
    #[test]
    fn confirming_into_an_empty_cache_is_a_clean_no_op() {
        let mem = story_with_code_past_eof();
        let mut cache = DisasmCache::build(&mem);
        for addr in [0u32, 0x40, 0xFFFF] {
            assert!(!cache.confirm_pc(&mem, addr), "nothing to confirm");
            assert!(!cache.confirm_routine(&mem, addr), "nothing to confirm");
        }
        assert!(cache.units().is_empty(), "still nothing to tile");
    }

    #[test]
    fn code_region_bounds_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let (start, end) = code_region(&mem);
        let initial_pc = mem.read_word(0x06) as u32;
        assert!(start <= initial_pc, "region_start {start:#x} > initial_pc {initial_pc:#x}");
        assert!(start < end, "region_start {start:#x} >= region_end {end:#x}");
        assert_eq!(end, mem.len() as u32);
    }

    #[test]
    fn v6_boot_root_unpacks_the_packed_main_address() {
        // v6: header 0x06 is the PACKED address of `main`, not a direct PC.
        // With routine offset 0, unpack.routine(p) = 4*p. The old bug used the
        // raw word directly, rooting reachability ~4x too low.
        let mut buf = crate::header::tests_support::sample_story(6);
        buf[0x04] = 0x01; buf[0x05] = 0x00; // high_mem_base = 0x0100
        buf[0x06] = 0x00; buf[0x07] = 0x80; // packed main = 0x0080 → entry 0x0200
        buf[0x28] = 0x00; buf[0x29] = 0x00; // routine offset = 0
        buf[0x2A] = 0x00; buf[0x2B] = 0x00; // string offset = 0
        buf[0x0200] = 0x00; // main routine: 0 locals
        buf[0x0201] = 0xB0; // rtrue — clean terminator for RD/validation
        let mem = Memory::new(buf).unwrap();

        // boot_root unpacks 0x0080 → 0x0200, NOT the raw 0x0080.
        assert_eq!(boot_root(&mem), 0x0200, "v6 boot root must unpack the packed word");

        // region_start is high memory, not min(high_mem, raw_word) = 0x0080.
        let (rstart, rend) = code_region(&mem);
        assert_eq!(rstart, 0x0100, "v6 region must root at high memory, not the raw packed word");
        assert_eq!(rend, mem.len() as u32);

        // RD roots at the true main entry and records it as a routine.
        let unpack = Unpack::from_mem(&mem);
        let rd = discover_rd(&mem, 6, &unpack, (rstart, rend));
        assert!(rd.contains(&0x0200), "RD did not root at the unpacked v6 main entry");
    }

    #[test]
    fn boot_root_is_the_raw_word_for_non_v6() {
        // v1-5/7/8: header 0x06 is a direct PC — boot_root returns it verbatim.
        let mem = Memory::new(crate::header::tests_support::sample_story(5)).unwrap();
        assert_eq!(boot_root(&mem), mem.read_word(0x06) as u32);
    }

    #[test]
    fn code_region_bounds_on_synthetic_story() {
        // sample_story() header: high_mem_base=0x0400, initial_pc=0x0040, len=0x0400.
        let bytes = crate::header::tests_support::sample_story(5);
        let mem = Memory::new(bytes).unwrap();
        let (start, end) = code_region(&mem);
        let initial_pc = mem.read_word(0x06) as u32;
        assert!(start <= initial_pc);
        assert!(start < end);
        assert_eq!(end, mem.len() as u32);
    }

    #[test]
    fn empty_cache_reports_the_region_it_was_built_for() {
        let bytes = crate::header::tests_support::sample_story(5);
        let mem = Memory::new(bytes).unwrap();
        let expected = code_region(&mem);
        let cache = DisasmCache::empty(&mem);
        assert_eq!(cache.region(), expected);
        assert!(cache.units().is_empty());
        assert!(cache.routines().is_empty());
        assert_eq!(cache.version(), mem.version());
        assert_eq!(cache.unpack().version, mem.version());
    }

    #[test]
    fn unit_addr_and_end_for_instr() {
        let u = Unit::Instr { addr: 10, next: 14, prov: Provenance::Rd };
        assert_eq!(u.addr(), 10);
        assert_eq!(u.end(), 14);
    }

    #[test]
    fn unit_addr_and_end_for_routine_header() {
        let u = Unit::RoutineHeader { addr: 20, nlocals: 3, first_instr: 27, prov: Provenance::Rd };
        assert_eq!(u.addr(), 20);
        assert_eq!(u.end(), 27);
    }

    #[test]
    fn unit_addr_and_end_for_data() {
        let u = Unit::Data { addr: 30, len: 8 };
        assert_eq!(u.addr(), 30);
        assert_eq!(u.end(), 38);
    }

    #[test]
    fn is_call_classifies_call_opcodes() {
        use OperandCount::*;
        assert!(is_call(Var, 0x00, 3), "call_vs");
        assert!(!is_call(One, 0x0F, 3), "1OP:0x0F is `not` in v3");
        assert!(is_call(One, 0x0F, 5), "1OP:0x0F is call_1n in v5");
        assert!(is_call(Two, 0x1A, 5), "call_2n");
        assert!(is_call(One, 0x08, 5), "call_1s");
    }

    #[test]
    fn is_terminator_classifies_terminators() {
        use OperandCount::*;
        assert!(is_terminator(One, 0x0B), "ret");
        assert!(is_terminator(Zero, 0x0A), "quit");
        assert!(!is_terminator(Two, 0x14), "add is not a terminator");
    }

    #[test]
    fn routine_first_instr_v3_skips_local_words() {
        // v3 (<= 4): first instr = entry + 1 + nlocals*2.
        let bytes = crate::header::tests_support::sample_story(3);
        let mut mem = Memory::new(bytes).unwrap();
        let entry = 0x0080u32;
        let nlocals = 3u8;
        mem.write_byte(entry, nlocals);
        assert_eq!(
            routine_first_instr(&mem, entry, 3),
            entry + 1 + (nlocals as u32) * 2
        );
    }

    #[test]
    fn routine_first_instr_v5_has_no_local_words() {
        // v5+ carries no initial-value words: first instr = entry + 1.
        let bytes = crate::header::tests_support::sample_story(5);
        let mut mem = Memory::new(bytes).unwrap();
        let entry = 0x0080u32;
        mem.write_byte(entry, 7);
        assert_eq!(routine_first_instr(&mem, entry, 5), entry + 1);
    }

    #[test]
    fn discover_rd_finds_routines_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let unpack = Unpack::from_mem(&mem);
        let region = code_region(&mem);

        let routines = discover_rd(&mem, version, &unpack, region);
        assert!(!routines.is_empty(), "RD found no routines");
        for &entry in &routines {
            assert!(
                entry >= region.0 && entry < region.1,
                "entry {entry:#x} outside region {region:#x?}"
            );
            let nlocals = mem.read_byte(entry);
            assert!(nlocals <= 15, "entry {entry:#x} locals byte {nlocals} > 15");
        }
    }

    #[test]
    fn discover_rd_follows_the_first_constant_call() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let unpack = Unpack::from_mem(&mem);
        let region = code_region(&mem);

        // Independently decode forward from initial_pc until the first `call*`
        // with a constant routine operand, and unpack its target ourselves.
        let mut pc = mem.read_word(0x06) as u32;
        let mut expected: Option<u32> = None;
        for _ in 0..4096 {
            if pc >= region.1 {
                break;
            }
            let instr = decode(&mem, pc, version);
            if is_call(instr.operand_count.clone(), instr.opcode, version) {
                let n = match instr.operands.first() {
                    Some(Operand::Large(n)) => Some(*n),
                    Some(Operand::Small(n)) => Some(*n as u16),
                    _ => None,
                };
                if let Some(n) = n {
                    if n != 0 {
                        expected = Some(unpack.routine(n));
                        break;
                    }
                }
            }
            if is_terminator(instr.operand_count.clone(), instr.opcode) {
                break;
            }
            pc = if instr.next_pc > pc { instr.next_pc } else { pc + 1 };
        }

        let expected = expected.expect("no constant call reachable from initial_pc");
        let routines = discover_rd(&mem, version, &unpack, region);
        assert!(
            routines.contains(&expected),
            "RD did not include first-call target {expected:#x}"
        );
    }

    #[test]
    fn linear_finds_indirect_routines_rd_misses() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let unpack = Unpack::from_mem(&mem);
        let region = code_region(&mem);

        let rd = discover_rd(&mem, version, &unpack, region);
        let extra = discover_linear(&mem, version, region, &rd);
        eprintln!(
            "counts: rd={} linear={} total={}",
            rd.len(),
            extra.len(),
            rd.len() + extra.len()
        );
        assert!(!extra.is_empty(), "linear scan found no extra routines");
        for &e in &extra {
            assert!(
                e >= region.0 && e < region.1,
                "extra {e:#x} outside region {region:#x?}"
            );
            assert!(mem.read_byte(e) <= 15, "extra {e:#x} locals byte > 15");
            assert!(!rd.contains(&e), "extra {e:#x} already in rd");
        }
    }

    #[test]
    fn linear_rejects_dictionary_data() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        // Dictionary base lives in static memory (header word 0x08) — a
        // data-heavy region. Scan a 256-byte window as its own region so
        // validation reads never leave it.
        let dict = mem.read_word(0x08) as u32;
        let window = 256u32;
        let empty = BTreeSet::new();
        let accepted = discover_linear(&mem, version, (dict, dict + window), &empty);
        eprintln!(
            "dictionary-window false positives: {} of {window}",
            accepted.len()
        );
        assert!(
            (accepted.len() as u32) < window / 8,
            "validator accepted too many data bytes: {} of {window}",
            accepted.len()
        );
    }

    #[test]
    fn build_tiles_gaplessly_with_linear_augmented_routines() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let unpack = Unpack::from_mem(&mem);
        let region = code_region(&mem);
        let (rstart, rend) = region;

        let rd = discover_rd(&mem, version, &unpack, region);
        let cache = DisasmCache::build(&mem);
        let u = cache.units();

        // Task 3 tiling invariant still holds with the larger routine set.
        assert!(!u.is_empty(), "build produced no units");
        for w in u.windows(2) {
            assert!(
                w[0].addr() < w[1].addr(),
                "not strictly sorted: {:#x} !< {:#x}",
                w[0].addr(),
                w[1].addr()
            );
            assert_eq!(
                w[0].end(),
                w[1].addr(),
                "gap/overlap between {:#x?} and {:#x?}",
                w[0],
                w[1]
            );
        }
        assert_eq!(u[0].addr(), rstart, "first unit does not start at region_start");
        assert_eq!(u.last().unwrap().end(), rend, "last unit does not end at region_end");

        // Linear scan added routines: more RoutineHeader units than RD alone.
        let header_count = u
            .iter()
            .filter(|x| matches!(x, Unit::RoutineHeader { .. }))
            .count();
        assert!(
            header_count > rd.len(),
            "linear scan added no routines: {header_count} headers <= {} rd",
            rd.len()
        );
    }

    #[test]
    fn build_with_linear_scan_completes() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        assert!(!cache.units().is_empty(), "build produced no units");
    }

    #[test]
    fn build_tiles_the_region_gaplessly_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let (rstart, rend) = code_region(&mem);
        let cache = DisasmCache::build(&mem);
        let u = cache.units();

        assert!(!u.is_empty(), "build produced no units");
        // Strictly sorted, gapless, no overlap.
        for w in u.windows(2) {
            assert!(
                w[0].addr() < w[1].addr(),
                "not strictly sorted: {:#x} !< {:#x}",
                w[0].addr(),
                w[1].addr()
            );
            assert_eq!(
                w[0].end(),
                w[1].addr(),
                "gap/overlap between {:#x?} and {:#x?}",
                w[0],
                w[1]
            );
        }
        assert_eq!(u[0].addr(), rstart, "first unit does not start at region_start");
        assert_eq!(
            u.last().unwrap().end(),
            rend,
            "last unit does not end at region_end"
        );
    }

    #[test]
    fn build_emits_routine_headers_and_instructions_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        let has_header = cache
            .units()
            .iter()
            .any(|u| matches!(u, Unit::RoutineHeader { .. }));
        let has_instr = cache
            .units()
            .iter()
            .any(|u| matches!(u, Unit::Instr { .. }));
        assert!(has_header, "no RoutineHeader units");
        assert!(has_instr, "no Instr units");
    }

    #[test]
    fn build_places_initial_pc_inside_an_instr_unit_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let initial_pc = mem.read_word(0x06) as u32;
        let cache = DisasmCache::build(&mem);
        let found = cache.units().iter().any(|u| {
            matches!(u, Unit::Instr { .. }) && u.addr() <= initial_pc && initial_pc < u.end()
        });
        assert!(
            found,
            "initial_pc {initial_pc:#x} not inside any Instr unit"
        );
    }

    #[test]
    fn build_never_truncates_an_instruction_at_a_soft_boundary_on_minizork() {
        // Regression: a linear-scan false positive (e.g. a call's store byte
        // misread as a routine header) must not truncate a real instruction.
        // Every Instr unit is either full length, or truncated ONLY where a
        // HARD boundary (an RD routine entry / initial_pc) or region end falls
        // inside it — never at a soft (linear-only) boundary.
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let cache = DisasmCache::build(&mem);
        let (rstart, rend) = code_region(&mem);
        let mut hard = discover_rd(&mem, version, &Unpack::from_mem(&mem), (rstart, rend));
        let initial_pc = mem.read_word(0x06) as u32;
        if initial_pc >= rstart && initial_pc < rend {
            hard.insert(initial_pc);
        }
        for u in cache.units() {
            if let Unit::Instr { addr, next, .. } = *u {
                let natural = decode(&mem, addr, version).next_pc;
                assert!(
                    next == natural
                        || (next < natural && (hard.contains(&next) || next == rend)),
                    "instr at {addr:#x} truncated to {next:#x} (natural {natural:#x}) \
                     at a non-hard boundary"
                );
            }
        }
    }

    #[test]
    fn nav_next_addr_from_unit_start_lands_on_next_unit_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        let u = cache.units();
        let k = u.len() / 3;
        let (a, b) = (u[k], u[k + 1]);
        assert_eq!(cache.next_addr(a.addr()), b.addr());
    }

    #[test]
    fn nav_next_addr_from_mid_unit_lands_on_next_unit_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        let u = cache.units();
        let k = u.len() / 3;
        let (a, b) = (u[k], u[k + 1]);
        let mid = if a.end() > a.addr() + 1 { (a.addr() + a.end()) / 2 } else { a.addr() };
        assert_eq!(cache.next_addr(mid), b.addr());
    }

    #[test]
    fn nav_prev_addr_from_unit_start_lands_on_previous_unit_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        let u = cache.units();
        let k = u.len() / 3;
        let (a, b, c) = (u[k], u[k + 1], u[k + 2]);
        assert_eq!(cache.prev_addr(c.addr()), b.addr());
        assert_eq!(cache.prev_addr(b.addr()), a.addr());
    }

    #[test]
    fn nav_clamps_at_region_bounds_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let (rstart, _rend) = code_region(&mem);
        let cache = DisasmCache::build(&mem);
        let u = cache.units();
        let last = u.last().unwrap();
        assert_eq!(cache.next_addr(last.addr()), last.addr());
        assert_eq!(cache.prev_addr(rstart), rstart);
        assert_eq!(cache.prev_addr(rstart), u[0].addr());
    }

    #[test]
    fn nav_never_invents_a_boundary_crossing_into_header_or_data_on_minizork() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        let u = cache.units();

        let boundary = u.windows(2).enumerate().find(|(_, w)| {
            matches!(w[0], Unit::Instr { .. })
                && matches!(w[1], Unit::RoutineHeader { .. } | Unit::Data { .. })
        });

        let Some((i, _)) = boundary else {
            eprintln!("skipping: no Instr -> RoutineHeader/Data boundary found in minizork.z3");
            return;
        };

        assert_eq!(cache.next_addr(u[i].addr()), u[i + 1].addr());
        assert!(
            !matches!(u[i + 1], Unit::Instr { .. }),
            "navigation landed on an invented mid-data instruction instead of the real boundary"
        );
    }

    #[test]
    fn build_returns_and_stays_consistent_on_synthetic_story() {
        // No fixture required: build must return and hold its invariant on any
        // valid story (guards against panic / infinite-loop on sparse code).
        let bytes = crate::header::tests_support::sample_story(5);
        let mem = Memory::new(bytes).unwrap();
        let (rstart, rend) = code_region(&mem);
        let cache = DisasmCache::build(&mem);
        let u = cache.units();
        assert!(!u.is_empty());
        for w in u.windows(2) {
            assert!(w[0].addr() < w[1].addr());
            assert_eq!(w[0].end(), w[1].addr());
        }
        assert_eq!(u[0].addr(), rstart);
        assert_eq!(u.last().unwrap().end(), rend);
    }

    #[test]
    fn disassemble_full_matches_legacy_for_a_real_instruction() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let initial_pc = mem.read_word(0x06) as u32;
        let cache = DisasmCache::build(&mem);
        assert_eq!(
            cache.disassemble(&mem, initial_pc, 1, CacheFmt::Full)[0],
            crate::cpu::disasm::disassemble(&mem, initial_pc, mem.version(), 1)[0]
        );
    }

    #[test]
    fn disassemble_basic_and_raw_match_legacy_for_a_real_instruction() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let initial_pc = mem.read_word(0x06) as u32;
        let cache = DisasmCache::build(&mem);
        assert_eq!(
            cache.disassemble(&mem, initial_pc, 1, CacheFmt::Basic)[0],
            crate::cpu::disasm::disassemble_basic(&mem, initial_pc, mem.version(), 1)[0]
        );
        assert_eq!(
            cache.disassemble(&mem, initial_pc, 1, CacheFmt::Raw)[0],
            crate::cpu::disasm::disassemble_raw(&mem, initial_pc, mem.version(), 1)[0]
        );
    }

    #[test]
    fn disassemble_renders_data_as_bytes_never_an_instruction() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        let Some(data) = cache.units().iter().find(|u| matches!(u, Unit::Data { .. })) else {
            eprintln!("skipping: no Data unit in minizork.z3");
            return;
        };
        let row = &cache.disassemble(&mem, data.addr(), 1, CacheFmt::Full)[0];
        let prefix = format!("{:06x}  ", data.addr());
        assert!(row.starts_with(&prefix), "addr prefix mismatch: {row:?}");
        assert!(
            row[prefix.len()..].starts_with(".byte "),
            "data row must render as .byte, not an instruction: {row:?}"
        );
    }

    #[test]
    fn disassemble_renders_routine_header_as_a_marker() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        let Some(&Unit::RoutineHeader { addr, nlocals, .. }) =
            cache.units().iter().find(|u| matches!(u, Unit::RoutineHeader { .. }))
        else {
            eprintln!("skipping: no RoutineHeader unit in minizork.z3");
            return;
        };
        let row = &cache.disassemble(&mem, addr, 1, CacheFmt::Full)[0];
        assert!(row.contains("; routine,"), "got {row:?}");
        let noun = if nlocals == 1 { "local" } else { "locals" };
        assert!(row.contains(&format!("{} {}", nlocals, noun)), "got {row:?}");
        // Basic keeps the marker too.
        let basic = &cache.disassemble(&mem, addr, 1, CacheFmt::Basic)[0];
        assert!(basic.contains("; routine,"), "got {basic:?}");
        // Raw shows the untranslated header bytes + "routine", no `;` comment.
        let raw = &cache.disassemble(&mem, addr, 1, CacheFmt::Raw)[0];
        assert!(raw.starts_with(&format!("{:06x}: ", addr)), "raw prefix: {raw:?}");
        assert!(raw.ends_with("   routine"), "raw suffix: {raw:?}");
        assert!(!raw.contains("; routine"), "raw must not use the `;` marker: {raw:?}");
    }

    #[test]
    fn disassemble_caps_lines_and_never_over_reads() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let (rstart, _rend) = code_region(&mem);
        let cache = DisasmCache::build(&mem);
        assert!(cache.disassemble(&mem, rstart, 5, CacheFmt::Full).len() <= 5);
        // Requesting far more lines than remain stops cleanly (no panic, bounded).
        let last = cache.units().last().unwrap().addr();
        let rows = cache.disassemble(&mem, last, 1000, CacheFmt::Full);
        assert!(rows.len() <= 1000);
    }

    #[test]
    fn disassemble_multi_row_data_steps_by_16_and_stops_mid_run() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        let Some(&Unit::Data { addr, len }) = cache
            .units()
            .iter()
            .find(|u| matches!(u, Unit::Data { len, .. } if *len > 16))
        else {
            eprintln!("skipping: no Data unit with len > 16 in minizork.z3");
            return;
        };
        let want = len.div_ceil(16) as usize;
        let rows = cache.disassemble(&mem, addr, want, CacheFmt::Full);
        assert!(rows.len() >= 2, "expected multiple .byte rows: {rows:?}");
        for (k, row) in rows.iter().enumerate() {
            assert!(row.contains(".byte "), "row {k} not a .byte row: {row:?}");
            let expect_addr = addr + 16 * k as u32;
            assert!(
                row.starts_with(&format!("{:06x}  ", expect_addr)),
                "row {k} addr should step by 16: {row:?}"
            );
        }
        // lines=1 stops after a single row, mid-run.
        assert_eq!(cache.disassemble(&mem, addr, 1, CacheFmt::Full).len(), 1);
    }

    /// Assert the gapless/sorted/bounds tiling invariant directly in a test
    /// (independent of the debug-only internal guard).
    fn assert_tiling(cache: &DisasmCache) {
        let (rstart, rend) = cache.region();
        let u = cache.units();
        assert!(!u.is_empty(), "no units");
        assert_eq!(u[0].addr(), rstart, "first unit not at region_start");
        assert_eq!(u.last().unwrap().end(), rend, "last unit not at region_end");
        for w in u.windows(2) {
            assert!(w[0].addr() < w[1].addr(), "not sorted: {:#x?} {:#x?}", w[0], w[1]);
            assert_eq!(w[0].end(), w[1].addr(), "gap/overlap: {:#x?} {:#x?}", w[0], w[1]);
        }
    }

    #[test]
    fn soft_routine_headers_follow_a_control_break_not_fall_through() {
        // A linear-scan (soft) routine header must be reached by a control break
        // (ret/jump/…), never by fall-through — a fall-through-reached candidate is
        // mid-code (a byte ≤15 that looked like a locals count). RD (hard) headers
        // are proven call targets and may follow fall-through in memory, so they're
        // exempt. (fall-through heuristic)
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let rd = discover_rd(&mem, version, &Unpack::from_mem(&mem), code_region(&mem));
        let cache = DisasmCache::build(&mem);
        let u = cache.units();
        for i in 1..u.len() {
            let Unit::RoutineHeader { addr, .. } = u[i] else { continue };
            if rd.contains(&addr) { continue; } // hard header: any predecessor is fine
            let Unit::Instr { addr: paddr, .. } = u[i - 1] else { continue }; // Data/header pred: fine
            let prev = decode(&mem, paddr, version);
            assert!(
                is_terminator(prev.operand_count.clone(), prev.opcode),
                "soft RoutineHeader at {addr:06x} is preceded by fall-through instr at {paddr:06x}"
            );
        }
    }

    #[test]
    fn confirm_pc_absorbs_an_instruction_longer_than_the_stale_unit() {
        // Regression: a `read` (4 bytes) whose opcode byte was eaten by a stale
        // 2-byte unit, so the confirmed pc lands INSIDE that unit and the real
        // instruction overlaps the following unit. confirm_pc must extend the read
        // to its full span and absorb the overlap — not truncate it to the stale
        // unit's end (which stranded a phantom fragment and hid the read).
        let mut story = crate::header::tests_support::sample_story(3);
        let b = 0x40usize; // [filler, e4 sread, af=two vars, var, var, ba quit]
        story[b] = 0x00;
        story[b + 1] = 0xe4;
        story[b + 2] = 0xaf;
        story[b + 3] = 0x10;
        story[b + 4] = 0x11;
        story[b + 5] = 0xba;
        let mem = Memory::new(story).unwrap();
        let base = b as u32;
        // Stale tiling: [base,base+2) ate the read's 0xe4; [base+2,base+5); quit.
        let mut cache = DisasmCache {
            units: vec![
                Unit::Instr { addr: base, next: base + 2, prov: Provenance::Soft },
                Unit::Instr { addr: base + 2, next: base + 5, prov: Provenance::Soft },
                Unit::Instr { addr: base + 5, next: base + 6, prov: Provenance::Soft },
            ],
            routines: Default::default(),
            region_start: base,
            region_end: base + 6,
            version: 3,
            unpack: Unpack::from_mem(&mem),
        };
        assert!(cache.confirm_pc(&mem, base + 1), "confirm changes the tiling");
        assert_tiling(&cache);
        assert_eq!(cache.next_addr(base + 1), base + 5, "read spans its full 4 bytes");
        assert_eq!(cache.prev_addr(base + 5), base + 1, "the read is quit's predecessor");
        let row = &cache.disassemble(&mem, base + 1, 1, CacheFmt::Full)[0];
        assert!(row.contains("sread"), "read renders as a read op: {row:?}");
    }

    #[test]
    fn confirm_pc_heals_inside_a_data_unit() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let mut cache = DisasmCache::build(&mem);
        let Some(&Unit::Data { addr, .. }) = cache
            .units()
            .iter()
            .find(|u| matches!(u, Unit::Data { len, .. } if *len >= 6))
        else {
            eprintln!("skipping: no Data unit with len >= 6 in minizork.z3");
            return;
        };
        let pc = addr + 2; // strictly inside the Data run
        assert!(cache.confirm_pc(&mem, pc), "confirm_pc should report a change");
        let i = cache.unit_index_at(pc).expect("a built minizork cache has units");
        assert!(
            matches!(cache.units()[i], Unit::Instr { .. }),
            "pc should now name an Instr unit"
        );
        assert_eq!(cache.units()[i].addr(), pc, "Instr unit should start at pc");
        assert_tiling(&cache);
    }

    #[test]
    fn confirm_pc_no_op_on_existing_instr_boundary() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let mut cache = DisasmCache::build(&mem);
        let instr_addr = cache
            .units()
            .iter()
            .find_map(|u| match u {
                Unit::Instr { addr, .. } => Some(*addr),
                _ => None,
            })
            .expect("no Instr unit");
        let before = cache.units().to_vec();
        assert!(!cache.confirm_pc(&mem, instr_addr), "boundary confirm should be a no-op");
        assert_eq!(cache.units(), &before[..], "units must be byte-identical");
    }

    #[test]
    fn confirm_pc_is_idempotent() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let mut cache = DisasmCache::build(&mem);
        let Some(&Unit::Data { addr, .. }) = cache
            .units()
            .iter()
            .find(|u| matches!(u, Unit::Data { len, .. } if *len >= 6))
        else {
            eprintln!("skipping: no Data unit with len >= 6 in minizork.z3");
            return;
        };
        let pc = addr + 2;
        assert!(cache.confirm_pc(&mem, pc), "first confirm heals");
        let after_first = cache.units().to_vec();
        assert!(!cache.confirm_pc(&mem, pc), "second confirm is a no-op");
        assert_eq!(cache.units(), &after_first[..], "units unchanged on 2nd call");
        assert_tiling(&cache);
    }

    #[test]
    fn confirm_routine_promotes_an_entry() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let mut cache = DisasmCache::build(&mem);
        let (_rstart, rend) = cache.region();

        // Pick an address inside a Data unit that is not already a routine
        // header, with a plausible locals byte and a first instruction inside
        // the region.
        let candidate = cache.units().iter().find_map(|u| {
            let Unit::Data { addr, len } = *u else { return None };
            (0..len).map(|k| addr + k).find(|&a| {
                mem.read_byte(a) <= 15
                    && routine_first_instr(&mem, a, version) < rend
                    && !cache.routines().contains(&a)
            })
        });
        let Some(e) = candidate else {
            eprintln!("skipping: no promotable entry candidate in minizork.z3");
            return;
        };

        assert!(!cache.routines().contains(&e));
        assert!(cache.confirm_routine(&mem, e), "confirm_routine should report a change");
        assert!(cache.routines().contains(&e), "entry must be tracked");
        assert!(
            cache.units().iter().any(|u| matches!(u, Unit::RoutineHeader { addr, .. } if *addr == e)),
            "a RoutineHeader unit at e must exist"
        );
        let first = routine_first_instr(&mem, e, version);
        assert!(
            cache.units().iter().any(|u| matches!(u, Unit::Instr { addr, .. } if *addr == first)),
            "an Instr unit at first_instr must exist"
        );
        assert_tiling(&cache);
        // Idempotent.
        assert!(!cache.confirm_routine(&mem, e), "second confirm_routine is a no-op");
        assert_tiling(&cache);
    }

    // ── SQ-0428: per-line provenance tiers ───────────────────────────────────

    #[test]
    fn data_units_are_provenance_data() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let cache = DisasmCache::build(&mem);
        let Some(data) = cache.units().iter().find(|u| matches!(u, Unit::Data { .. })) else {
            eprintln!("skipping: no Data unit in minizork.z3");
            return;
        };
        assert_eq!(data.provenance(), Provenance::Data);
        // The tiered accessor tags the `.byte` row(s) as Data.
        let rows = cache.disassemble_tiered(&mem, data.addr(), 1, CacheFmt::Full);
        assert_eq!(rows[0].1, Provenance::Data, "data row: {:?}", rows[0].0);
        assert!(rows[0].0.contains(".byte "), "got {:?}", rows[0].0);
    }

    #[test]
    fn rd_routine_lines_are_provenance_rd() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let unpack = Unpack::from_mem(&mem);
        let region = code_region(&mem);
        let rd = discover_rd(&mem, version, &unpack, region);
        let cache = DisasmCache::build(&mem);
        // A RoutineHeader whose entry is an RD (hard) routine is tagged Rd, and
        // its first-instruction row is Rd too.
        let Some(&entry) = rd.iter().find(|&&e| {
            cache.units().iter().any(|u| matches!(u, Unit::RoutineHeader { addr, .. } if *addr == e))
        }) else {
            eprintln!("skipping: no RD routine surfaced as a header in minizork.z3");
            return;
        };
        let rows = cache.disassemble_tiered(&mem, entry, 2, CacheFmt::Full);
        assert_eq!(rows[0].1, Provenance::Rd, "RD header should be Rd: {:?}", rows[0].0);
        assert!(rows.iter().all(|(_, p)| *p == Provenance::Rd), "RD run must be all Rd");
    }

    #[test]
    fn linear_only_routine_lines_are_provenance_soft() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let unpack = Unpack::from_mem(&mem);
        let region = code_region(&mem);
        let rd = discover_rd(&mem, version, &unpack, region);
        let extra = discover_linear(&mem, version, region, &rd);
        let cache = DisasmCache::build(&mem);
        // A RoutineHeader whose entry came ONLY from the linear scan is Soft.
        let Some(&entry) = extra.iter().find(|&&e| {
            cache.units().iter().any(|u| matches!(u, Unit::RoutineHeader { addr, .. } if *addr == e))
        }) else {
            eprintln!("skipping: no linear-only routine surfaced as a header in minizork.z3");
            return;
        };
        assert!(!rd.contains(&entry), "entry must be linear-only");
        let rows = cache.disassemble_tiered(&mem, entry, 1, CacheFmt::Full);
        assert_eq!(rows[0].1, Provenance::Soft, "linear-only header should be Soft: {:?}", rows[0].0);
    }

    #[test]
    fn confirm_pc_upgrades_soft_bytes_to_rd() {
        // A soft region proven real by execution (confirm_pc) re-decodes to Rd.
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let mut cache = DisasmCache::build(&mem);
        // Pick a byte strictly inside a Data unit (classified as data, i.e. not
        // even Soft code) and confirm the VM executed there.
        let Some(&Unit::Data { addr, .. }) = cache
            .units()
            .iter()
            .find(|u| matches!(u, Unit::Data { len, .. } if *len >= 6))
        else {
            eprintln!("skipping: no Data unit with len >= 6 in minizork.z3");
            return;
        };
        let pc = addr + 2;
        assert_eq!(
            cache.disassemble_tiered(&mem, pc, 1, CacheFmt::Full)[0].1,
            Provenance::Data,
            "pre-confirm the byte is classified as data"
        );
        assert!(cache.confirm_pc(&mem, pc), "confirm_pc heals the region");
        let rows = cache.disassemble_tiered(&mem, pc, 1, CacheFmt::Full);
        assert_eq!(rows[0].1, Provenance::Rd, "execution-confirmed bytes must be Rd: {:?}", rows[0].0);
    }

    #[test]
    fn confirm_routine_tags_the_reanchored_region_rd() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else {
            eprintln!("skipping: minizork.z3 fixture not present");
            return;
        };
        let mem = Memory::new(bytes).unwrap();
        let version = mem.version();
        let mut cache = DisasmCache::build(&mem);
        let (_rstart, rend) = cache.region();
        // A promotable entry inside a Data unit (mirrors confirm_routine_promotes_an_entry).
        let candidate = cache.units().iter().find_map(|u| {
            let Unit::Data { addr, len } = *u else { return None };
            (0..len).map(|k| addr + k).find(|&a| {
                mem.read_byte(a) <= 15
                    && routine_first_instr(&mem, a, version) < rend
                    && !cache.routines().contains(&a)
            })
        });
        let Some(e) = candidate else {
            eprintln!("skipping: no promotable entry candidate in minizork.z3");
            return;
        };
        assert!(cache.confirm_routine(&mem, e), "confirm_routine re-anchors the entry");
        // The header AND its first instruction are hard (Rd).
        let rows = cache.disassemble_tiered(&mem, e, 2, CacheFmt::Full);
        assert_eq!(rows[0].1, Provenance::Rd, "confirmed header should be Rd: {:?}", rows[0].0);
        assert_eq!(rows[1].1, Provenance::Rd, "confirmed first instr should be Rd: {:?}", rows[1].0);
    }
}
