//! Native implementations of the 13 well-known Glulx accelerated functions
//! (`@accelfunc`). See docs/superpowers/plans/2026-07-02-glulx-acceleration-algorithms.md
//! (authoritative) and the Glulx spec §2.17 / Glulxe accel.c.

use crate::exec::{Machine, R};

/// True iff `num` names an accelerated function this VM implements (1..=13).
pub(crate) fn accel_impl_supported(num: u32) -> bool {
    (1..=13).contains(&num)
}

/// The well-known name of accelerated function `num` (spec §2.17 / Glulxe
/// accel.c), for the disassembler's function-header badge. Numbers 2/8, 3/9,
/// … are the V1/V2 variants of the same routine.
pub fn accel_name(num: u32) -> &'static str {
    match num {
        1 => "Z__Region",
        2 | 8 => "CP__Tab",
        3 | 9 => "RA__Pr",
        4 | 10 => "RL__Pr",
        5 | 11 => "OC__Cl",
        6 | 12 => "RV__Pr",
        7 | 13 => "OP__Pr",
        _ => "accel?",
    }
}

/// Which property-table offset `CP__Tab` (and everything built on it) uses inside
/// the object record. See algorithms.md §"cp_tab" — the only V1/V2 divergence.
#[derive(Clone, Copy, PartialEq)]
enum Variant {
    V1,
    V2,
}

impl Machine {
    /// Run accelerated function `num` (assumed 1..=13) with `args`, returning its
    /// value. Never builds a frame; a memory fault propagates as an interpreter error.
    pub(crate) fn accel_dispatch(&mut self, num: u32, args: &[u32]) -> R<u32> {
        match num {
            1 => self.accel_z_region(args),
            2 => self.accel_cp_tab(args, Variant::V1),
            3 => self.accel_ra_pr(args, Variant::V1),
            4 => self.accel_rl_pr(args, Variant::V1),
            5 => self.accel_oc_cl(args, Variant::V1),
            6 => self.accel_rv_pr(args, Variant::V1),
            7 => self.accel_op_pr(args, Variant::V1),
            8 => self.accel_cp_tab(args, Variant::V2),
            9 => self.accel_ra_pr(args, Variant::V2),
            10 => self.accel_rl_pr(args, Variant::V2),
            11 => self.accel_oc_cl(args, Variant::V2),
            12 => self.accel_rv_pr(args, Variant::V2),
            13 => self.accel_op_pr(args, Variant::V2),
            _ => Ok(0),
        }
    }

    #[inline]
    fn accel_arg(args: &[u32], i: usize) -> u32 {
        args.get(i).copied().unwrap_or(0)
    }

    #[inline]
    fn accel_param_or0(&self, i: u32) -> u32 {
        self.accel_param(i).unwrap_or(0)
    }

    /// accel.c's `accel_error(msg)` (SQ-1416 item 7): writes `"\n{msg}\n"` to the
    /// current Glk stream, but only when the VM's I/O system is `iosys_Glk` (mode
    /// 2) — accel.c's own comment on the `iosys_Filter` case: "there's no way to
    /// validly call the filter function, and the Glk printing path might throw a
    /// fatal error. Just give up." Same three calls it makes: `glk_put_char('\n')`,
    /// `glk_put_string(msg)`, `glk_put_char('\n')`, folded into one
    /// [`Machine::glk_put_current`] write of the same bytes. Correct games never
    /// reach these programming-error paths (they only fire on already-broken
    /// stories); accelfunctest's slow-vs-fast transcripts pin the wording.
    fn accel_error(&mut self, msg: &str) {
        if self.iosys_mode == 2 {
            self.glk_put_current(&format!("\n{msg}\n"));
        }
    }

    /// Function 1 — Z__Region.
    fn accel_z_region(&self, args: &[u32]) -> R<u32> {
        let addr = Self::accel_arg(args, 0);
        if addr < 36 || addr >= self.mem.endmem() {
            return Ok(0);
        }
        let tb = self.m8(addr)?;
        Ok(if tb >= 0xE0 {
            3
        } else if tb >= 0xC0 {
            2
        } else if (0x70..=0x7F).contains(&tb) && addr >= self.mem.ramstart() {
            1
        } else {
            0
        })
    }

    /// `obj_in_class(obj)` shared helper — accel.c 223–228.
    fn obj_in_class(&self, obj: u32) -> R<bool> {
        let nab = self.accel_param_or0(7);
        Ok(self.m32(obj.wrapping_add(13).wrapping_add(nab))? == self.accel_param_or0(2))
    }

    /// Binary search the property table: `num` 10-byte records from `start`, 2-byte
    /// big-endian key at record offset 0. Returns the matching record address or 0.
    fn binsearch_prop(&self, key: u32, start: u32, num: u32) -> R<u32> {
        let key = key & 0xFFFF;
        let (mut lo, mut hi) = (0u32, num);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let rec = start.wrapping_add(mid.wrapping_mul(10));
            let have = self.m16(rec)?;
            match have.cmp(&key) {
                std::cmp::Ordering::Equal => return Ok(rec),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        Ok(0)
    }

    /// Functions 2/8 — CP__Tab.
    fn accel_cp_tab(&mut self, args: &[u32], variant: Variant) -> R<u32> {
        let obj = Self::accel_arg(args, 0);
        let id = Self::accel_arg(args, 1);
        if self.accel_z_region(&[obj])? != 1 {
            self.accel_error("[** Programming error: tried to find the \".\" of (something) **]");
            return Ok(0);
        }
        let otab = match variant {
            Variant::V1 => self.m32(obj.wrapping_add(16))?,
            Variant::V2 => {
                let nab = self.accel_param_or0(7);
                self.m32(obj.wrapping_add(4u32.wrapping_mul(3 + nab / 4)))?
            }
        };
        if otab == 0 {
            return Ok(0);
        }
        let max = self.m32(otab)?;
        self.binsearch_prop(id, otab.wrapping_add(4), max)
    }

    /// `get_prop(obj, id, variant)` shared helper — accel.c 230–264 / 266–303.
    /// Mutually recursive with `accel_oc_cl` via the class-property path.
    fn get_prop(&mut self, mut obj: u32, mut id: u32, variant: Variant) -> R<u32> {
        let mut cla = 0u32;
        if id & 0xFFFF_0000 != 0 {
            cla = self.m32(self.accel_param_or0(0).wrapping_add((id & 0xFFFF).wrapping_mul(4)))?;
            if self.accel_oc_cl(&[obj, cla], variant)? == 0 {
                return Ok(0);
            }
            id >>= 16;
            obj = cla;
        }
        let prop = self.accel_cp_tab(&[obj, id], variant)?;
        if prop == 0 {
            return Ok(0);
        }
        if self.obj_in_class(obj)? && cla == 0 {
            let ips = self.accel_param_or0(1);
            if id < ips || id >= ips.wrapping_add(8) {
                return Ok(0);
            }
        }
        if self.m32(self.accel_param_or0(6))? != obj && self.m8(prop + 9)? & 1 != 0 {
            return Ok(0);
        }
        Ok(prop)
    }

    /// Functions 3/9 — RA__Pr.
    fn accel_ra_pr(&mut self, args: &[u32], variant: Variant) -> R<u32> {
        let prop = self.get_prop(Self::accel_arg(args, 0), Self::accel_arg(args, 1), variant)?;
        if prop == 0 {
            Ok(0)
        } else {
            self.m32(prop + 4)
        }
    }

    /// Functions 4/10 — RL__Pr.
    fn accel_rl_pr(&mut self, args: &[u32], variant: Variant) -> R<u32> {
        let prop = self.get_prop(Self::accel_arg(args, 0), Self::accel_arg(args, 1), variant)?;
        if prop == 0 {
            Ok(0)
        } else {
            Ok(4 * self.m16(prop + 2)?)
        }
    }

    /// Functions 5/11 — OC__Cl. accel.c 393–458 / 573–638.
    fn accel_oc_cl(&mut self, args: &[u32], variant: Variant) -> R<u32> {
        let obj = Self::accel_arg(args, 0);
        let cla = Self::accel_arg(args, 1);
        let zr = self.accel_z_region(&[obj])?;
        if zr == 3 {
            return Ok(if cla == self.accel_param_or0(5) { 1 } else { 0 });
        }
        if zr == 2 {
            return Ok(if cla == self.accel_param_or0(4) { 1 } else { 0 });
        }
        if zr != 1 {
            return Ok(0);
        }
        if cla == self.accel_param_or0(2) {
            if self.obj_in_class(obj)? {
                return Ok(1);
            }
            if obj == self.accel_param_or0(2) {
                return Ok(1);
            }
            if obj == self.accel_param_or0(5) {
                return Ok(1);
            }
            if obj == self.accel_param_or0(4) {
                return Ok(1);
            }
            if obj == self.accel_param_or0(3) {
                return Ok(1);
            }
            return Ok(0);
        }
        if cla == self.accel_param_or0(3) {
            if self.obj_in_class(obj)? {
                return Ok(0);
            }
            if obj == self.accel_param_or0(2) {
                return Ok(0);
            }
            if obj == self.accel_param_or0(5) {
                return Ok(0);
            }
            if obj == self.accel_param_or0(4) {
                return Ok(0);
            }
            if obj == self.accel_param_or0(3) {
                return Ok(0);
            }
            return Ok(1);
        }
        if cla == self.accel_param_or0(5) || cla == self.accel_param_or0(4) {
            return Ok(0);
        }
        if !self.obj_in_class(cla)? {
            self.accel_error("[** Programming error: tried to apply 'ofclass' with non-class **]");
            return Ok(0);
        }
        let prop = self.get_prop(obj, 2, variant)?;
        if prop == 0 {
            return Ok(0);
        }
        let inlist = self.m32(prop + 4)?; // inlined RA__Pr(obj,2)
        if inlist == 0 {
            return Ok(0);
        }
        let inlistlen = self.m16(prop + 2)?; // inlined RL__Pr(obj,2)/WORDSIZE
        for jx in 0..inlistlen {
            if self.m32(inlist.wrapping_add(4u32.wrapping_mul(jx)))? == cla {
                return Ok(1);
            }
        }
        Ok(0)
    }

    /// Functions 6/12 — RV__Pr.
    fn accel_rv_pr(&mut self, args: &[u32], variant: Variant) -> R<u32> {
        let id = Self::accel_arg(args, 1);
        let addr = self.accel_ra_pr(args, variant)?;
        if addr == 0 {
            if id > 0 && id < self.accel_param_or0(1) {
                return self.m32(self.accel_param_or0(8).wrapping_add(4u32.wrapping_mul(id)));
            }
            self.accel_error("[** Programming error: tried to read (something) **]");
            return Ok(0);
        }
        self.m32(addr)
    }

    /// Functions 7/13 — OP__Pr.
    fn accel_op_pr(&mut self, args: &[u32], variant: Variant) -> R<u32> {
        let obj = Self::accel_arg(args, 0);
        let id = Self::accel_arg(args, 1);
        let zr = self.accel_z_region(&[obj])?;
        if zr == 3 {
            let ips = self.accel_param_or0(1);
            if id == ips.wrapping_add(6) {
                return Ok(1); // print
            }
            if id == ips.wrapping_add(7) {
                return Ok(1); // print_to_array
            }
            return Ok(0);
        }
        if zr == 2 {
            return Ok(if id == self.accel_param_or0(1).wrapping_add(5) { 1 } else { 0 }); // call
        }
        if zr != 1 {
            return Ok(0);
        }
        let ips = self.accel_param_or0(1);
        if id >= ips && id < ips.wrapping_add(8) && self.obj_in_class(obj)? {
            return Ok(1);
        }
        Ok(if self.accel_ra_pr(args, variant)? != 0 { 1 } else { 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm;
    use crate::glk::TestBackend;
    use crate::memory::Memory;

    /// A machine whose start function is empty, with `ram_bytes` of RAM
    /// (RAMSTART == 0x100, per `asm::assemble`'s tiny-code layout).
    fn accel_test_machine() -> (Machine, u32, u32, u32) {
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x100);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        assert_eq!(m.mem.ramstart(), 0x100, "test assumes RAMSTART == 0x100");

        let obj_addr = 0x100;
        let routine_addr = 0x104;
        let string_addr = 0x108;
        m.mem.write8(obj_addr, 0x70).unwrap();
        m.mem.write8(routine_addr, 0xC0).unwrap();
        m.mem.write8(string_addr, 0xE0).unwrap();

        (m, obj_addr, routine_addr, string_addr)
    }

    #[test]
    fn z_region_classifies_addresses() {
        let (mut m, obj_addr, routine_addr, string_addr) = accel_test_machine();
        assert_eq!(m.accel_dispatch(1, &[10]).unwrap(), 0); // addr < 36
        assert_eq!(m.accel_dispatch(1, &[obj_addr]).unwrap(), 1); // object
        assert_eq!(m.accel_dispatch(1, &[routine_addr]).unwrap(), 2); // routine
        assert_eq!(m.accel_dispatch(1, &[string_addr]).unwrap(), 3); // string
        assert_eq!(m.accel_dispatch(1, &[]).unwrap(), 0); // no arg -> addr 0
    }

    // ── Task 3: synthetic property/class world ─────────────────────────────────
    //
    // Layout (RAMSTART == 0x100, num_attr_bytes == 7 so V1's fixed `obj+16` and
    // V2's `obj+4*(3+nab/4)` land on the same address):
    //
    //   OBJ         = 0x100  type=0x70; +16 -> OTAB_OBJ; +20 (=13+7) -> class-chain
    //                 field (0, so OBJ is not itself a class)
    //   OTAB_OBJ     = 0x120  count=2, records at +4: id=2 (inlist prop), id=P(8)
    //   INLIST_ADDR  = 0x180  1 entry: THE_CLASS
    //   PROP_ADDR    = 0x190  PROP_VALUE
    //   THE_CLASS    = 0x1A0  type=0x70; +20 -> CLASS_METACLASS (is-a-class)
    //   OTHER_CLASS  = 0x1C0  type=0x70; +20 -> CLASS_METACLASS (is-a-class, but
    //                 absent from OBJ's inlist)
    //   ROUTINE_ADDR = 0x1E0  type=0xC0
    //   SELF_GLOBAL  = 0x1F0  holds OBJ (irrelevant: no property is private)
    //   CPV_START    = 0x200  common-property defaults array
    //
    // Property ids: P = 8 (present, common), Q_ABSENT = 9 (absent),
    // Q_ABSENT_COMMON = 5 (absent, common). indiv_prop_start (param 1) = 20.

    const OBJ: u32 = 0x100;
    const OTAB_OBJ: u32 = 0x120;
    const INLIST_ADDR: u32 = 0x180;
    const PROP_ADDR: u32 = 0x190;
    const THE_CLASS: u32 = 0x1A0;
    const OTHER_CLASS: u32 = 0x1C0;
    const ROUTINE_ADDR: u32 = 0x1E0;
    const SELF_GLOBAL: u32 = 0x1F0;
    const CPV_START: u32 = 0x200;

    const P: u32 = 8;
    const Q_ABSENT: u32 = 9;
    const Q_ABSENT_COMMON: u32 = 5;
    const INDIV: u32 = 20; // indiv_prop_start
    const PROP_VALUE: u32 = 0xABCD_1234;
    const PROP_LEN_BYTES: u32 = 4; // 1 word
    const CPV_DEFAULT: u32 = 0xCAFE_BABE;
    const CLASS_METACLASS: u32 = 0x9000;
    const OBJECT_METACLASS: u32 = 0x9001;
    const ROUTINE_METACLASS: u32 = 0x9002;
    const STRING_METACLASS: u32 = 0x9003;

    /// Builds the synthetic world described above (`num_attr_bytes == 7`).
    fn accel_world() -> Machine {
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x300);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        assert_eq!(m.mem.ramstart(), 0x100, "test assumes RAMSTART == 0x100");

        // OBJ
        m.mem.write8(OBJ, 0x70).unwrap();
        m.mem.write32(OBJ + 16, OTAB_OBJ).unwrap();
        m.mem.write32(OBJ + 20, 0).unwrap(); // not itself a class

        // OTAB_OBJ: 2 records, sorted by id (2, then P=8)
        m.mem.write32(OTAB_OBJ, 2).unwrap(); // count
        let rec0 = OTAB_OBJ + 4;
        m.mem.write16(rec0, 2).unwrap(); // id = 2 (inlist prop)
        m.mem.write16(rec0 + 2, 1).unwrap(); // len = 1 word
        m.mem.write32(rec0 + 4, INLIST_ADDR).unwrap();
        m.mem.write8(rec0 + 9, 0).unwrap(); // flags: not private
        let rec1 = OTAB_OBJ + 14;
        m.mem.write16(rec1, P).unwrap();
        m.mem.write16(rec1 + 2, 1).unwrap(); // len = 1 word (4 bytes)
        m.mem.write32(rec1 + 4, PROP_ADDR).unwrap();
        m.mem.write8(rec1 + 9, 0).unwrap(); // flags: not private

        m.mem.write32(INLIST_ADDR, THE_CLASS).unwrap();
        m.mem.write32(PROP_ADDR, PROP_VALUE).unwrap();

        m.mem.write8(THE_CLASS, 0x70).unwrap();
        m.mem.write32(THE_CLASS + 20, CLASS_METACLASS).unwrap();

        m.mem.write8(OTHER_CLASS, 0x70).unwrap();
        m.mem.write32(OTHER_CLASS + 20, CLASS_METACLASS).unwrap();

        m.mem.write8(ROUTINE_ADDR, 0xC0).unwrap();

        m.mem.write32(SELF_GLOBAL, OBJ).unwrap();

        m.mem.write32(CPV_START + 4 * Q_ABSENT_COMMON, CPV_DEFAULT).unwrap();

        m.set_accel_param(0, 0x1000); // classes_table (unused by these tests)
        m.set_accel_param(1, INDIV);
        m.set_accel_param(2, CLASS_METACLASS);
        m.set_accel_param(3, OBJECT_METACLASS);
        m.set_accel_param(4, ROUTINE_METACLASS);
        m.set_accel_param(5, STRING_METACLASS);
        m.set_accel_param(6, SELF_GLOBAL);
        m.set_accel_param(7, 7); // num_attr_bytes
        m.set_accel_param(8, CPV_START);

        m
    }

    /// A minimal second world with `num_attr_bytes == nab` (!= 7), whose object's
    /// real CP__Tab pointer sits only at the V2 offset `obj+4*(3+nab/4)`; the V1
    /// offset `obj+16` is left zeroed.
    fn accel_world_nab(nab: u32) -> Machine {
        let start = asm::func(0xC1, &[], &[]);
        let built = asm::assemble(&[start], 0, 0x300);
        let mem = Memory::new(built.image).expect("valid image");
        let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
        assert_eq!(m.mem.ramstart(), 0x100, "test assumes RAMSTART == 0x100");

        let otab2 = 0x120u32;
        let v2_off = 4 * (3 + nab / 4);
        assert_ne!(v2_off, 16, "test requires nab to diverge from the V1 offset");

        m.mem.write8(OBJ, 0x70).unwrap();
        m.mem.write32(OBJ + v2_off, otab2).unwrap();

        m.mem.write32(otab2, 1).unwrap(); // count
        let rec0 = otab2 + 4;
        m.mem.write16(rec0, P).unwrap();
        m.mem.write16(rec0 + 2, 1).unwrap();
        m.mem.write32(rec0 + 4, 0x140).unwrap();
        m.mem.write8(rec0 + 9, 0).unwrap();

        m.set_accel_param(7, nab);

        m
    }

    #[test]
    fn ra_rl_rv_on_synthetic_object() {
        let mut m = accel_world();
        // present property P on OBJ: address non-zero, length as laid out, value dereferenced
        assert_eq!(m.accel_dispatch(3, &[OBJ, P]).unwrap(), PROP_ADDR); // RA__Pr v1
        assert_eq!(m.accel_dispatch(9, &[OBJ, P]).unwrap(), PROP_ADDR); // RA__Pr v2 (same, nab=7)
        assert_eq!(m.accel_dispatch(4, &[OBJ, P]).unwrap(), PROP_LEN_BYTES); // RL__Pr
        assert_eq!(m.accel_dispatch(6, &[OBJ, P]).unwrap(), PROP_VALUE); // RV__Pr
        // absent property Q -> RA/RL 0; RV falls back to cpv__start default for common props
        assert_eq!(m.accel_dispatch(3, &[OBJ, Q_ABSENT]).unwrap(), 0);
        assert_eq!(m.accel_dispatch(6, &[OBJ, Q_ABSENT_COMMON]).unwrap(), CPV_DEFAULT);
    }

    #[test]
    fn oc_cl_and_op_pr_classify() {
        let mut m = accel_world();
        assert_eq!(m.accel_dispatch(5, &[OBJ, THE_CLASS]).unwrap(), 1); // OC__Cl: obj is of class
        assert_eq!(m.accel_dispatch(5, &[OBJ, OTHER_CLASS]).unwrap(), 0);
        assert_eq!(m.accel_dispatch(7, &[OBJ, P]).unwrap(), 1); // OP__Pr: provides P
        assert_eq!(m.accel_dispatch(7, &[OBJ, Q_ABSENT]).unwrap(), 0);
        // Z__Region routing inside OP__Pr: routine provides only `call` (indiv+5)
        assert_eq!(m.accel_dispatch(7, &[ROUTINE_ADDR, INDIV + 5]).unwrap(), 1);
    }

    #[test]
    fn cp_tab_v1_v2_agree_at_nab7_and_diverge_otherwise() {
        // With num_attr_bytes = 7 the V1 (obj+16) and V2 (obj+4*(3+7/4)=obj+16) offsets match.
        let mut m = accel_world();
        let v1 = m.accel_dispatch(2, &[OBJ, P]).unwrap();
        assert_ne!(v1, 0, "agreement check is vacuous if the shared value is 0");
        assert_eq!(v1, m.accel_dispatch(8, &[OBJ, P]).unwrap());
        // With num_attr_bytes != 7, only V2 lands on the real table. The second world's
        // object places its prop-table pointer at obj+4*(3+nab/4); assert V2 finds
        // the property and V1 (obj+16) does not.
        let mut m2 = accel_world_nab(9);
        assert!(m2.accel_dispatch(8, &[OBJ, P]).unwrap() != 0);
        assert_eq!(m2.accel_dispatch(2, &[OBJ, P]).unwrap(), 0);
    }
}
