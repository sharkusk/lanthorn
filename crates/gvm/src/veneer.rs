//! Fingerprinted acceleration of the Inform 6 veneer (SQ-1209).
//!
//! Games built by Inform 7 6E59 (2010) and later announce their veneer routines
//! with `@accelfunc` and their layout constants with `@accelparam`, and
//! [`crate::accel`] then runs the seven well-known routines natively. Older
//! games — every plain Inform 6 build, and every Inform 7 build before 6E59 —
//! never make those calls, so the same routines are interpreted opcode by
//! opcode. SQ-1205 measured `King of Shreds and Patches`' `inventory` turn: 91%
//! of it inside the four routines `accel` already implements.
//!
//! This module finds those routines without being told where they are, and it
//! is deliberately conservative about it. A routine is accelerated only when
//! **every** one of the following holds:
//!
//! 1. Its bytecode matches a committed template **byte for byte** outside the
//!    template's masked operand slots. Nothing fuzzy, nothing partial. The mask
//!    covers exactly the operands whose bytes are image-specific: memory
//!    references and RAM-relative operands (addressing modes `5/6/7` and
//!    `D/E/F`), call targets, run-time-error message addresses, and the
//!    operands that carry the `@accelparam` constants. Every other byte —
//!    opcode numbers, addressing-mode nibbles, branch offsets, local offsets,
//!    and every genuine constant — must be identical.
//! 2. It matched at exactly one address in ROM. Two matches are an ambiguity we
//!    refuse rather than guess at.
//! 3. The whole set of seven matched, and the call targets embedded in them
//!    point at each other: `RA__Pr` must call the matched `OC__Cl` and the
//!    matched `CP__Tab`, `RV__Pr` must call the matched `RA__Pr`, and so on.
//!    This is what makes the match a statement about the game's *veneer* rather
//!    than about a body that happens to look like one — a lone copy of a routine
//!    somewhere else in ROM cannot satisfy a closed call graph.
//! 4. The nine parameters read out of the matched operands agree wherever the
//!    same parameter appears more than once, and pass a set of cross-checks
//!    against the story's object table — facts the bytecode does not supply.
//!    See [`CrossCheck`].
//!
//! If anything fails, nothing at all is installed and the reason is recorded in
//! [`VeneerReport::rejected`]; the routines are then interpreted exactly as
//! before. `--accel off` disables the interception outright, fingerprinted or
//! declared.
//!
//! ## Template provenance
//!
//! The committed template is the veneer of `BlueLacuna.gblorb` (Inform 6.31,
//! serial 100717), which registers its own `@accelfunc`s and so states the
//! ground truth for both the addresses and the nine parameters. It was extracted
//! by `cargo run -p gvm --example veneer_gen -- stories/BlueLacuna.gblorb`, which
//! is kept so the bytes below can be remade rather than only trusted. Across the 35
//! stories in the development corpus that register — Inform 6.31 through 6.41 —
//! the seven routines are *identical instruction for instruction*, and the only
//! fields that vary at all are the masked ones. `veneer_matches_registering_stories`
//! re-derives every one of them and checks the answer against what the game
//! itself declared.
//!
//! Inform 6.21 (City of Secrets, `advent.blb`, …) is **not** covered: its
//! codegen differs (no `jgeu`/`callfi`; `Z__Region` calls an
//! `Unsigned__Compare` helper), and more importantly its `CP__Tab` omits the
//! `Z__Region` guard that Glulxe's `accel.c` — and therefore [`crate::accel`] —
//! performs, so the native routine is not a drop-in for the interpreted one.
//! Matching refuses those games, which is the correct outcome.

use std::collections::HashMap;

use crate::decode::operand_data_len;
use crate::disasm::decode_instr;
use crate::memory::Memory;

/// How many `@accelparam` slots the spec defines (0..=8).
pub(crate) const NUM_PARAMS: usize = 9;

/// What a template operand means. Everything not named here must match the
/// template's bytes exactly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    /// A call target that must resolve to the matched address of accelerated
    /// function `n` — the call-graph closure check.
    Call(u32),
    /// The `RT__Err` veneer's address. Not matched against a template, but every
    /// occurrence across the seven routines must name the same function.
    RtErr,
    /// A run-time-error message address. Image-specific and unchecked.
    Message,
    /// Carries `@accelparam` `p` directly.
    Param(u32),
    /// Carries `@accelparam` `p` plus a fixed offset (`indiv_prop_start + k`).
    ParamPlus(u32, u32),
    /// A RAM-relative operand carrying `@accelparam` `p`: the parameter is
    /// `RAMSTART + value`.
    ParamRam(u32),
    /// A word index into the object record: `aload obj, W` reads `obj + 4*W`,
    /// which the veneer uses for the class-chain field at `obj + 13 +
    /// num_attr_bytes`. So `num_attr_bytes = 4*W - 13`.
    NabWordIndex,
}

/// One veneer routine's fingerprint: the reference bytes plus the operands that
/// are allowed to differ, addressed as (instruction index, operand index).
struct Template {
    /// Accelerated-function number (spec §2.17; 1..=7, the V1 forms).
    num: u32,
    name: &'static str,
    bytes: &'static [u8],
    slots: &'static [(u16, u8, Slot)],
}

/// The template family, extracted from `BlueLacuna.gblorb` (Inform 6.31).
const TEMPLATE_SOURCE: &str = "Inform 6.31-6.41 (BlueLacuna.gblorb, serial 100717)";

// ─── the seven routines ───────────────────────────────────────────────────────

/// `Z__Region` — no addresses, no parameters, no calls: it matches on every byte.
const T_Z_REGION: &[u8] = &[
    0xc1, 0x04, 0x03, 0x00, 0x00, 0x26, 0x19, 0x00, 0x00, 0x24, 0x81, 0x02, 0x09, 0x08, 0x2b, 0x99,
    0x01, 0x00, 0x08, 0x33, 0x4a, 0x09, 0x09, 0x00, 0x04, 0x26, 0x29, 0x01, 0x04, 0x00, 0xe0, 0x05,
    0x31, 0x01, 0x03, 0x26, 0x29, 0x01, 0x04, 0x00, 0xc0, 0x05, 0x31, 0x01, 0x02, 0x26, 0x19, 0x01,
    0x04, 0x70, 0x14, 0x28, 0x19, 0x01, 0x04, 0x7f, 0x0e, 0x48, 0x10, 0x08, 0x02, 0x26, 0x89, 0x01,
    0x00, 0x05, 0x31, 0x01, 0x01, 0x31, 0x00,
];

const T_CP_TAB: &[u8] = &[
    0xc1, 0x04, 0x05, 0x00, 0x00, 0x81, 0x61, 0x93, 0x08, 0x00, 0x1c, 0xbf, 0x73, 0x00, 0x24, 0x18,
    0x01, 0x01, 0x0e, 0x81, 0x62, 0x13, 0x09, 0x00, 0x1c, 0xba, 0x44, 0x17, 0x00, 0x31, 0x00, 0x48,
    0x19, 0x09, 0x00, 0x04, 0x08, 0x23, 0x19, 0x08, 0x04, 0x31, 0x00, 0x48, 0x09, 0x09, 0x08, 0x0c,
    0x10, 0x19, 0x09, 0x08, 0x04, 0x08, 0x81, 0x51, 0x19, 0x19, 0x09, 0x90, 0x04, 0x02, 0x08, 0x0a,
    0x0c, 0x10, 0x31, 0x09, 0x10,
];

const T_RA_PR: &[u8] = &[
    0xc1, 0x04, 0x05, 0x00, 0x00, 0x18, 0x39, 0x08, 0x04, 0xff, 0xff, 0x00, 0x00, 0x22, 0x18, 0x2b,
    0x18, 0x39, 0x08, 0x04, 0x00, 0x00, 0xff, 0xff, 0x48, 0x83, 0x09, 0x00, 0x52, 0x41, 0x89, 0x08,
    0x81, 0x62, 0x93, 0x89, 0x00, 0x1c, 0xb8, 0x89, 0x00, 0x08, 0x23, 0x18, 0x04, 0x31, 0x00, 0x1e,
    0x19, 0x09, 0x04, 0x10, 0x04, 0x40, 0x99, 0x08, 0x00, 0x81, 0x62, 0x93, 0x99, 0x00, 0x1c, 0xc0,
    0x6d, 0x00, 0x04, 0x0c, 0x23, 0x19, 0x0c, 0x04, 0x31, 0x00, 0x48, 0x19, 0x08, 0x00, 0x05, 0x25,
    0x38, 0x01, 0x00, 0x3b, 0x92, 0x01, 0x16, 0x23, 0x19, 0x08, 0x12, 0x26, 0x29, 0x01, 0x04, 0x01,
    0x00, 0x09, 0x26, 0x29, 0x01, 0x04, 0x01, 0x08, 0x04, 0x31, 0x00, 0x24, 0x9d, 0x01, 0x10, 0x00,
    0x0e, 0x4b, 0x19, 0x09, 0x0c, 0x48, 0x10, 0x22, 0x19, 0x10, 0x04, 0x31, 0x00, 0x48, 0x19, 0x08,
    0x0c, 0x01, 0x31, 0x08,
];

const T_RL_PR: &[u8] = &[
    0xc1, 0x04, 0x05, 0x00, 0x00, 0x18, 0x39, 0x08, 0x04, 0xff, 0xff, 0x00, 0x00, 0x22, 0x18, 0x2b,
    0x18, 0x39, 0x08, 0x04, 0x00, 0x00, 0xff, 0xff, 0x48, 0x83, 0x09, 0x00, 0x52, 0x41, 0x89, 0x08,
    0x81, 0x62, 0x93, 0x89, 0x00, 0x1c, 0xb8, 0x89, 0x00, 0x08, 0x23, 0x18, 0x04, 0x31, 0x00, 0x1e,
    0x19, 0x09, 0x04, 0x10, 0x04, 0x40, 0x99, 0x08, 0x00, 0x81, 0x62, 0x93, 0x99, 0x00, 0x1c, 0xc0,
    0x6d, 0x00, 0x04, 0x0c, 0x23, 0x19, 0x0c, 0x04, 0x31, 0x00, 0x48, 0x19, 0x08, 0x00, 0x05, 0x25,
    0x38, 0x01, 0x00, 0x3b, 0x92, 0x01, 0x16, 0x23, 0x19, 0x08, 0x12, 0x26, 0x29, 0x01, 0x04, 0x01,
    0x00, 0x09, 0x26, 0x29, 0x01, 0x04, 0x01, 0x08, 0x04, 0x31, 0x00, 0x24, 0x9d, 0x01, 0x10, 0x00,
    0x0e, 0x4b, 0x19, 0x09, 0x0c, 0x48, 0x10, 0x22, 0x19, 0x10, 0x04, 0x31, 0x00, 0x49, 0x19, 0x09,
    0x0c, 0x01, 0x10, 0x12, 0x91, 0x08, 0x04, 0x10, 0x31, 0x08,
];

const T_OC_CL: &[u8] = &[
    0xc1, 0x04, 0x06, 0x00, 0x00, 0x81, 0x61, 0x93, 0x09, 0x00, 0x1c, 0xbf, 0x73, 0x00, 0x08, 0x25,
    0x19, 0x01, 0x08, 0x03, 0x0d, 0x24, 0x39, 0x01, 0x04, 0x00, 0x3b, 0x92, 0x61, 0x01, 0x31, 0x00,
    0x25, 0x19, 0x01, 0x08, 0x02, 0x0d, 0x24, 0x39, 0x01, 0x04, 0x00, 0x3b, 0x92, 0x41, 0x01, 0x31,
    0x00, 0x25, 0x19, 0x00, 0x08, 0x01, 0x25, 0x39, 0x01, 0x04, 0x00, 0x3b, 0x92, 0x01, 0x35, 0x48,
    0x19, 0x08, 0x00, 0x05, 0x24, 0x38, 0x01, 0x00, 0x3b, 0x92, 0x01, 0x01, 0x24, 0x39, 0x01, 0x00,
    0x00, 0x3b, 0x92, 0x01, 0x01, 0x24, 0x39, 0x01, 0x00, 0x00, 0x3b, 0x92, 0x61, 0x01, 0x24, 0x39,
    0x01, 0x00, 0x00, 0x3b, 0x92, 0x41, 0x01, 0x24, 0x39, 0x01, 0x00, 0x00, 0x3b, 0x92, 0x21, 0x01,
    0x31, 0x00, 0x25, 0x39, 0x01, 0x04, 0x00, 0x3b, 0x92, 0x21, 0x31, 0x48, 0x19, 0x08, 0x00, 0x05,
    0x24, 0x38, 0x00, 0x00, 0x3b, 0x92, 0x01, 0x24, 0x39, 0x00, 0x00, 0x00, 0x3b, 0x92, 0x01, 0x24,
    0x39, 0x00, 0x00, 0x00, 0x3b, 0x92, 0x61, 0x24, 0x39, 0x00, 0x00, 0x00, 0x3b, 0x92, 0x41, 0x24,
    0x39, 0x00, 0x00, 0x00, 0x3b, 0x92, 0x21, 0x31, 0x01, 0x01, 0x24, 0x39, 0x00, 0x04, 0x00, 0x3b,
    0x92, 0x61, 0x24, 0x39, 0x00, 0x04, 0x00, 0x3b, 0x92, 0x41, 0x48, 0x19, 0x08, 0x04, 0x05, 0x24,
    0x38, 0x01, 0x00, 0x3b, 0x92, 0x01, 0x13, 0x81, 0x63, 0x33, 0x19, 0x00, 0x00, 0x1c, 0xba, 0x44,
    0x00, 0x29, 0x81, 0x2c, 0x04, 0xff, 0x31, 0x00, 0x81, 0x62, 0x93, 0x91, 0x00, 0x1c, 0xb7, 0x18,
    0x00, 0x02, 0x10, 0x22, 0x09, 0x10, 0x81, 0x62, 0x93, 0x81, 0x00, 0x1c, 0xb7, 0x9c, 0x00, 0x02,
    0x13, 0x18, 0x09, 0x04, 0x14, 0x40, 0x90, 0x0c, 0x27, 0x99, 0x01, 0x0c, 0x14, 0x15, 0x48, 0x99,
    0x08, 0x10, 0x0c, 0x24, 0x98, 0x01, 0x04, 0x01, 0x10, 0x19, 0x09, 0x0c, 0x01, 0x0c, 0x20, 0x01,
    0xe9, 0x31, 0x00,
];

const T_RV_PR: &[u8] = &[
    0xc1, 0x04, 0x03, 0x00, 0x00, 0x81, 0x62, 0x93, 0x99, 0x00, 0x1c, 0xb7, 0x18, 0x00, 0x04, 0x08,
    0x23, 0x19, 0x08, 0x29, 0x29, 0x09, 0x01, 0x04, 0x13, 0x27, 0x29, 0x01, 0x04, 0x01, 0x00, 0x0c,
    0x48, 0x93, 0x08, 0x00, 0x52, 0x40, 0xe9, 0x04, 0x31, 0x08, 0x81, 0x63, 0x33, 0x99, 0x00, 0x00,
    0x1c, 0xba, 0x44, 0x00, 0x29, 0x81, 0x1f, 0x00, 0x04, 0x31, 0x00, 0x48, 0x09, 0x08, 0x08, 0x31,
    0x08,
];

const T_OP_PR: &[u8] = &[
    0xc1, 0x04, 0x03, 0x00, 0x00, 0x81, 0x61, 0x93, 0x09, 0x00, 0x1c, 0xbf, 0x73, 0x00, 0x08, 0x25,
    0x19, 0x01, 0x08, 0x03, 0x12, 0x24, 0x29, 0x01, 0x04, 0x01, 0x06, 0x01, 0x24, 0x29, 0x01, 0x04,
    0x01, 0x07, 0x01, 0x31, 0x00, 0x25, 0x19, 0x01, 0x08, 0x02, 0x0b, 0x24, 0x29, 0x01, 0x04, 0x01,
    0x05, 0x01, 0x31, 0x00, 0x25, 0x19, 0x00, 0x08, 0x01, 0x26, 0x29, 0x01, 0x04, 0x01, 0x00, 0x16,
    0x27, 0x29, 0x01, 0x04, 0x01, 0x08, 0x0f, 0x48, 0x19, 0x08, 0x00, 0x05, 0x24, 0x38, 0x01, 0x00,
    0x3b, 0x92, 0x01, 0x01, 0x81, 0x62, 0x93, 0x89, 0x00, 0x1c, 0xb7, 0x18, 0x00, 0x04, 0x23, 0x18,
    0x01, 0x31, 0x00,
];

/// Accelerated-function numbers, named so the slot tables read as call graphs.
const Z_REGION: u32 = 1;
const CP_TAB: u32 = 2;
const RA_PR: u32 = 3;
const RL_PR: u32 = 4;
const OC_CL: u32 = 5;

/// `RA__Pr` and `RL__Pr` are the same routine up to their last two instructions,
/// so they share a slot table.
const RA_RL_SLOTS: &[(u16, u8, Slot)] = &[
    (3, 0, Slot::Param(0)),      // aload #classes_table, sp, L8
    (4, 0, Slot::Call(OC_CL)),   // callfii OC__Cl(obj, cla)
    (9, 0, Slot::Call(CP_TAB)),  // callfii CP__Tab(obj, id)
    (12, 1, Slot::NabWordIndex), // aload obj, #(13+nab)/4
    (13, 1, Slot::Param(2)),     // jne sp, #class_metaclass
    (15, 1, Slot::Param(1)),     // jlt id, #indiv_prop_start
    (16, 1, Slot::ParamPlus(1, 8)),
    (18, 0, Slot::ParamRam(6)), // jeq @self, obj
];

const TEMPLATES: &[Template] = &[
    Template { num: Z_REGION, name: "Z__Region", bytes: T_Z_REGION, slots: &[] },
    Template {
        num: CP_TAB,
        name: "CP__Tab",
        bytes: T_CP_TAB,
        slots: &[(0, 0, Slot::Call(Z_REGION)), (2, 0, Slot::RtErr)],
    },
    Template { num: RA_PR, name: "RA__Pr", bytes: T_RA_PR, slots: RA_RL_SLOTS },
    Template { num: RL_PR, name: "RL__Pr", bytes: T_RL_PR, slots: RA_RL_SLOTS },
    Template {
        num: OC_CL,
        name: "OC__Cl",
        bytes: T_OC_CL,
        slots: &[
            (0, 0, Slot::Call(Z_REGION)),
            (2, 1, Slot::Param(5)), // string_metaclass
            (5, 1, Slot::Param(4)), // routine_metaclass
            (8, 1, Slot::Param(2)), // class_metaclass
            (9, 1, Slot::NabWordIndex),
            (10, 1, Slot::Param(2)),
            (11, 1, Slot::Param(2)),
            (12, 1, Slot::Param(5)),
            (13, 1, Slot::Param(4)),
            (14, 1, Slot::Param(3)), // object_metaclass
            (16, 1, Slot::Param(3)),
            (17, 1, Slot::NabWordIndex),
            (18, 1, Slot::Param(2)),
            (19, 1, Slot::Param(2)),
            (20, 1, Slot::Param(5)),
            (21, 1, Slot::Param(4)),
            (22, 1, Slot::Param(3)),
            (24, 1, Slot::Param(5)),
            (25, 1, Slot::Param(4)),
            (26, 1, Slot::NabWordIndex),
            (27, 1, Slot::Param(2)),
            (28, 0, Slot::RtErr),
            (28, 1, Slot::Message),
            (30, 0, Slot::Call(RA_PR)),
            (32, 0, Slot::Call(RL_PR)),
        ],
    },
    Template {
        num: 6,
        name: "RV__Pr",
        bytes: T_RV_PR,
        slots: &[
            (0, 0, Slot::Call(RA_PR)),
            (3, 1, Slot::Param(1)), // jge id, #indiv_prop_start
            (4, 0, Slot::Param(8)), // aload #cpv__start, id, sp
            (6, 0, Slot::RtErr),
            (6, 1, Slot::Message),
        ],
    },
    Template {
        num: 7,
        name: "OP__Pr",
        bytes: T_OP_PR,
        slots: &[
            (0, 0, Slot::Call(Z_REGION)),
            (2, 1, Slot::ParamPlus(1, 6)), // print
            (3, 1, Slot::ParamPlus(1, 7)), // print_to_array
            (6, 1, Slot::ParamPlus(1, 5)), // call
            (9, 1, Slot::Param(1)),
            (10, 1, Slot::ParamPlus(1, 8)),
            (11, 1, Slot::NabWordIndex),
            (12, 1, Slot::Param(2)),
            (13, 0, Slot::Call(RA_PR)),
        ],
    },
];

// ─── decoding a template ──────────────────────────────────────────────────────

/// One template operand, located by byte offset within the template.
struct TOperand {
    off: usize,
    len: usize,
    slot: Slot,
}

/// A template with its mask and capture points resolved.
struct Decoded {
    num: u32,
    name: &'static str,
    bytes: &'static [u8],
    /// True where the candidate's byte must equal the template's byte.
    compare: Vec<bool>,
    /// Operands whose value we read out of the candidate.
    captures: Vec<TOperand>,
}

/// Wrap `bytes` in a minimal valid Glulx image so the shared instruction decoder
/// can walk them. The routine lands at address 36 (just past the header), which
/// is where a real image's code region starts too.
fn template_memory(bytes: &[u8]) -> Memory {
    let end = (36 + bytes.len() as u32).next_multiple_of(256);
    let mut img = vec![0u8; end as usize];
    img[0..4].copy_from_slice(b"Glul");
    img[4..8].copy_from_slice(&0x0003_0102u32.to_be_bytes());
    img[8..12].copy_from_slice(&end.to_be_bytes()); // RAMSTART
    img[12..16].copy_from_slice(&end.to_be_bytes()); // EXTSTART
    img[16..20].copy_from_slice(&end.to_be_bytes()); // ENDMEM
    img[20..24].copy_from_slice(&4096u32.to_be_bytes()); // stack size
    img[24..28].copy_from_slice(&36u32.to_be_bytes()); // start function
    img[36..36 + bytes.len()].copy_from_slice(bytes);
    Memory::new(img).expect("synthetic template image is well-formed")
}

/// Byte offset of the first instruction: past the type byte and the
/// local-format pairs, which end at a `00 00` pair.
fn body_offset(bytes: &[u8]) -> usize {
    let mut i = 1;
    while i + 1 < bytes.len() {
        if bytes[i] == 0 && bytes[i + 1] == 0 {
            return i + 2;
        }
        i += 2;
    }
    bytes.len()
}

impl Template {
    /// Decode this template into its byte mask and capture points. Panics on a
    /// malformed template — the tables are compiled-in constants, so a failure
    /// here is a source defect, and `template_tables_decode_cleanly` catches it.
    fn decode(&self) -> Decoded {
        let mem = template_memory(self.bytes);
        let mut compare = vec![true; self.bytes.len()];
        let mut captures = Vec::new();
        let mut pc = 36 + body_offset(self.bytes) as u32;
        let limit = 36 + self.bytes.len() as u32;
        let mut idx: u16 = 0;
        while pc < limit {
            let ins = decode_instr(&mem, pc)
                .unwrap_or_else(|e| panic!("veneer template {} does not decode: {e}", self.name));
            assert!(ins.next <= limit, "veneer template {} overruns its extent", self.name);
            for (oi, op) in ins.operands.iter().enumerate() {
                let off = (op.data_addr - 36) as usize;
                let len = operand_data_len(op.mode) as usize;
                let slot = self
                    .slots
                    .iter()
                    .find(|(i, o, _)| *i == idx && *o as usize == oi)
                    .map(|(_, _, s)| *s);
                let masked = matches!(op.mode & 0x0F, 0x5..=0x7 | 0xD..=0xF) || slot.is_some();
                if masked {
                    for b in compare.iter_mut().skip(off).take(len) {
                        *b = false;
                    }
                }
                if let Some(slot) = slot {
                    captures.push(TOperand { off, len, slot });
                }
            }
            pc = ins.next;
            idx += 1;
        }
        assert_eq!(pc, limit, "veneer template {} does not end on an instruction", self.name);
        // Every declared slot must have been reached; a stale index would
        // silently un-mask a field.
        assert_eq!(
            captures.len(),
            self.slots.len(),
            "veneer template {} declares a slot at an index the decode never reached",
            self.name
        );
        Decoded { num: self.num, name: self.name, bytes: self.bytes, compare, captures }
    }
}

// ─── matching ─────────────────────────────────────────────────────────────────

impl Decoded {
    /// Does the candidate at `base` in `rom` match outside the mask?
    fn matches_at(&self, rom: &[u8], base: usize) -> bool {
        if base + self.bytes.len() > rom.len() {
            return false;
        }
        let window = &rom[base..base + self.bytes.len()];
        for (i, (&want, &cmp)) in self.bytes.iter().zip(self.compare.iter()).enumerate() {
            if cmp && window[i] != want {
                return false;
            }
        }
        true
    }

    /// Read a captured operand out of the candidate at `base`.
    fn read(&self, rom: &[u8], base: usize, op: &TOperand) -> u32 {
        let s = base + op.off;
        let mut v = 0u32;
        for &b in &rom[s..s + op.len] {
            v = (v << 8) | b as u32;
        }
        v
    }
}

/// One derived cross-check and whether the story passed it. Everything here is
/// read out of the story's own object table, not out of the bytecode the
/// parameters came from — the point is to fail a derivation that is
/// self-consistent and wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossCheck {
    pub name: &'static str,
    pub passed: bool,
}

/// What fingerprinting concluded about a story. Attached to the machine and
/// surfaced by the hosts' diagnostics.
#[derive(Clone, Debug, Default)]
pub struct VeneerReport {
    /// The template family that matched (empty when nothing did).
    pub template: &'static str,
    /// `(accelerated-function number, routine name, ROM address)`, ascending.
    pub matched: Vec<(u32, &'static str, u32)>,
    /// The nine derived `@accelparam` values, in index order.
    pub params: [u32; NUM_PARAMS],
    /// Every cross-check that ran, in the order it ran.
    pub checks: Vec<CrossCheck>,
    /// Why nothing was installed, when nothing was.
    pub rejected: Option<String>,
}

impl VeneerReport {
    /// True iff the routines in [`Self::matched`] were installed.
    pub fn installed(&self) -> bool {
        self.rejected.is_none() && !self.matched.is_empty()
    }

    /// A one-line summary for a host's diagnostics log.
    pub fn summary(&self) -> String {
        match &self.rejected {
            Some(why) => format!("veneer acceleration: not applied ({why})"),
            None if self.matched.is_empty() => {
                "veneer acceleration: no template matched".to_string()
            }
            None => {
                let names: Vec<String> = self
                    .matched
                    .iter()
                    .map(|(_, name, addr)| format!("{name}@{addr:#x}"))
                    .collect();
                format!(
                    "veneer acceleration: {} [{}] params classes_table={:#x} indiv_prop_start={} \
                     class/object/routine/string_metaclass={:#x}/{:#x}/{:#x}/{:#x} self={:#x} \
                     num_attr_bytes={} cpv__start={:#x}",
                    self.template,
                    names.join(" "),
                    self.params[0],
                    self.params[1],
                    self.params[2],
                    self.params[3],
                    self.params[4],
                    self.params[5],
                    self.params[6],
                    self.params[7],
                    self.params[8],
                )
            }
        }
    }
}

/// What a successful fingerprint hands the machine to install: the
/// `address → accelerated-function number` assignments and the nine parameters.
pub(crate) type Install = (HashMap<u32, u32>, [u32; NUM_PARAMS]);

/// Fingerprint `mem`'s ROM. Returns what was found and, when everything checks
/// out, what to install.
pub(crate) fn fingerprint(mem: &Memory) -> (VeneerReport, Option<Install>) {
    let mut report = VeneerReport { template: TEMPLATE_SOURCE, ..VeneerReport::default() };
    let rom_end = mem.ramstart() as usize;
    let rom = &mem.raw_bytes()[..rom_end.min(mem.raw_bytes().len())];

    let decoded: Vec<Decoded> = TEMPLATES.iter().map(|t| t.decode()).collect();
    // A veneer routine's first byte is its function type byte; every template in
    // this family starts `C1` (locals-format call), so one pass over ROM with
    // that as the filter is enough.
    let mut found: Vec<Option<usize>> = vec![None; decoded.len()];
    let shortest = decoded.iter().map(|d| d.bytes.len()).min().unwrap_or(0);
    for base in 36..rom.len().saturating_sub(shortest) {
        if rom[base] != 0xC1 {
            continue;
        }
        for (ti, d) in decoded.iter().enumerate() {
            if !d.matches_at(rom, base) {
                continue;
            }
            if found[ti].is_some() {
                report.rejected =
                    Some(format!("{} matches at more than one address in ROM", d.name));
                return (report, None);
            }
            found[ti] = Some(base);
        }
    }
    let Some(addrs) = found.iter().copied().collect::<Option<Vec<usize>>>() else {
        let missing: Vec<&str> = decoded
            .iter()
            .zip(found.iter())
            .filter(|(_, f)| f.is_none())
            .map(|(d, _)| d.name)
            .collect();
        report.rejected = Some(format!("no template match for {}", missing.join(", ")));
        return (report, None);
    };

    report.matched = decoded
        .iter()
        .zip(addrs.iter())
        .map(|(d, a)| (d.num, d.name, *a as u32))
        .collect();
    report.matched.sort_by_key(|(n, _, _)| *n);

    // Captures: parameters (with agreement), call-graph closure, RT__Err identity.
    let mut params: [Option<u32>; NUM_PARAMS] = [None; NUM_PARAMS];
    let mut plus: Vec<(u32, u32, u32)> = Vec::new(); // (param, offset, observed)
    let mut rt_err: Option<u32> = None;
    let mut disagree: Option<String> = None;
    fn set(
        params: &mut [Option<u32>; NUM_PARAMS],
        disagree: &mut Option<String>,
        p: usize,
        v: u32,
        who: &str,
    ) {
        match params[p] {
            Some(prev) if prev != v => {
                if disagree.is_none() {
                    *disagree =
                        Some(format!("accelparam {p} read as {prev:#x} and {v:#x} in {who}"));
                }
            }
            _ => params[p] = Some(v),
        }
    }
    for (d, &base) in decoded.iter().zip(addrs.iter()) {
        for cap in &d.captures {
            let v = d.read(rom, base, cap);
            match cap.slot {
                Slot::Call(n) => {
                    let want = addrs[decoded.iter().position(|x| x.num == n).unwrap()] as u32;
                    if v != want {
                        report.rejected = Some(format!(
                            "{} calls {v:#x} where the matched accel {n} is at {want:#x}",
                            d.name
                        ));
                        return (report, None);
                    }
                }
                Slot::RtErr => match rt_err {
                    Some(prev) if prev != v => {
                        report.rejected = Some(format!(
                            "RT__Err is {prev:#x} in one routine and {v:#x} in {}",
                            d.name
                        ));
                        return (report, None);
                    }
                    _ => rt_err = Some(v),
                },
                Slot::Message => {}
                Slot::Param(p) => set(&mut params, &mut disagree, p as usize, v, d.name),
                Slot::ParamPlus(p, k) => plus.push((p, k, v)),
                // RAM-relative operands are RAMSTART plus a zero-extended offset,
                // wrapping exactly as `resolve_load`'s 0xD..0xF arms do.
                Slot::ParamRam(p) => {
                    let a = mem.ramstart().wrapping_add(v);
                    set(&mut params, &mut disagree, p as usize, a, d.name)
                }
                Slot::NabWordIndex => {
                    if v < 4 {
                        report.rejected =
                            Some(format!("{} reads the class chain at word {v}", d.name));
                        return (report, None);
                    }
                    set(&mut params, &mut disagree, 7, 4 * v - 13, d.name);
                }
            }
        }
    }
    if let Some(why) = disagree {
        report.rejected = Some(why);
        return (report, None);
    }
    let Some(params) = collect_params(&params) else {
        report.rejected = Some("the template did not yield all nine accelparams".to_string());
        return (report, None);
    };
    for (p, k, observed) in plus {
        if observed != params[p as usize].wrapping_add(k) {
            report.rejected = Some(format!(
                "accelparam {p} + {k} is {observed:#x}, not {:#x}",
                params[p as usize].wrapping_add(k)
            ));
            return (report, None);
        }
    }
    report.params = params;

    report.checks = cross_check(mem, &params);
    if let Some(failed) = report.checks.iter().find(|c| !c.passed) {
        report.rejected = Some(format!("cross-check failed: {}", failed.name));
        return (report, None);
    }

    let assignments =
        report.matched.iter().map(|(n, _, a)| (*a, *n)).collect::<HashMap<u32, u32>>();
    (report, Some((assignments, params)))
}

fn collect_params(p: &[Option<u32>; NUM_PARAMS]) -> Option<[u32; NUM_PARAMS]> {
    let mut out = [0u32; NUM_PARAMS];
    for (i, v) in p.iter().enumerate() {
        out[i] = (*v)?;
    }
    Some(out)
}

/// Check the derived parameters against the story's object table — the
/// independent evidence the bytecode cannot supply.
///
/// * `classes_table` is Inform's array of class objects, and its first four
///   entries are the four metaclasses in the order Class, Object, Routine,
///   String. Reading them back ties parameter 0 to parameters 2..=5.
/// * Each metaclass address must be an object by `Z__Region`'s own test (a type
///   byte in `0x70..=0x7F`), and the four must be evenly spaced — they are
///   consecutive records in one object table.
/// * That stride must leave room for the class-chain field at `13 +
///   num_attr_bytes`, and `classes_table[4]` — the first genuine class — must
///   carry `class_metaclass` in exactly that field. This is what validates
///   `num_attr_bytes` against the object record layout rather than against the
///   `aload` index it was read from.
/// * `self` must be a RAM address, and `cpv__start`'s `indiv_prop_start`-entry
///   defaults array must lie inside the memory map.
fn cross_check(mem: &Memory, p: &[u32; NUM_PARAMS]) -> Vec<CrossCheck> {
    let (ct, ips, cm, om, rm, sm, self_g, nab, cpv) =
        (p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8]);
    let m32 = |a: u32| mem.read32(a);
    let is_object = |a: u32| matches!(mem.read8(a), Some(b) if (0x70..=0x7F).contains(&b));
    let chain = |a: u32| m32(a.wrapping_add(13).wrapping_add(nab));
    let stride = om.wrapping_sub(cm);
    let mut out = Vec::new();
    let mut push = |name, passed| out.push(CrossCheck { name, passed });
    push(
        "classes_table lists the four metaclasses",
        m32(ct) == Some(cm)
            && m32(ct.wrapping_add(4)) == Some(om)
            && m32(ct.wrapping_add(8)) == Some(rm)
            && m32(ct.wrapping_add(12)) == Some(sm),
    );
    push("metaclasses are objects", [cm, om, rm, sm].iter().all(|&a| is_object(a)));
    push(
        "metaclasses are evenly spaced",
        stride != 0 && rm.wrapping_sub(om) == stride && sm.wrapping_sub(rm) == stride,
    );
    push("num_attr_bytes fits the object stride", stride >= 13 + nab + 4);
    let first_class = m32(ct.wrapping_add(16));
    push(
        "the first class is of class Class at 13+num_attr_bytes",
        matches!(first_class, Some(c) if is_object(c) && chain(c) == Some(cm)),
    );
    push("self is a RAM address", self_g >= mem.ramstart() && self_g < mem.endmem());
    push(
        "the common-property defaults fit the memory map",
        ips > 0 && (cpv as u64) + 4 * (ips as u64) <= mem.endmem() as u64,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every committed template decodes to a whole number of instructions that
    /// exactly fills its byte array, and every declared slot lands on a real
    /// operand. `Template::decode` asserts both; this runs it for all seven.
    #[test]
    fn template_tables_decode_cleanly() {
        for t in TEMPLATES {
            let d = t.decode();
            assert_eq!(d.captures.len(), t.slots.len(), "{}", t.name);
            if t.num == Z_REGION {
                // Z__Region names no address and no parameter: every byte compares.
                assert!(d.compare.iter().all(|c| *c), "Z__Region should be fully exact");
            } else {
                assert!(d.compare.iter().any(|c| !*c), "{} masks nothing", t.name);
            }
        }
    }

    // ── a synthetic story built out of the templates ──────────────────────────
    //
    // A 8 KiB image: the seven routines planted back to back at 0x100 (each
    // separated by a padding byte, so the layout is nothing like the reference
    // story's), a tiny object world in RAM from 0x1040, and every masked slot
    // rewritten to point at them. Nothing here shares an address with the
    // reference story, which is the "addresses differ, still a match" half of
    // the contract.

    /// A tiny object world in RAM: four metaclasses spaced 32 apart from 0x1100,
    /// a classes_table at 0x1080 naming them plus one real class at 0x1180.
    const SYN_PARAMS: [u32; NUM_PARAMS] = [
        0x1080, // classes_table
        256,    // indiv_prop_start
        0x1100, // class_metaclass
        0x1120, // object_metaclass
        0x1140, // routine_metaclass
        0x1160, // string_metaclass
        0x1040, // self
        7,      // num_attr_bytes
        0x1200, // cpv__start
    ];
    const SYN_RAMSTART: u32 = 0x1000;

    /// Where each template gets planted.
    fn plant_addresses() -> Vec<usize> {
        let mut cursor = 0x100usize;
        TEMPLATES
            .iter()
            .map(|t| {
                let at = cursor;
                cursor += t.bytes.len() + 1;
                at
            })
            .collect()
    }

    fn put(img: &mut [u8], off: usize, len: usize, v: u32) {
        for i in 0..len {
            img[off + i] = (v >> (8 * (len - 1 - i))) as u8;
        }
    }

    /// Build the synthetic story. `nab_index` overrides the class-chain word
    /// index every routine reads (7 attribute bytes → index 5 is the truthful
    /// one); `patch` gets the finished image and the plant addresses.
    fn synthetic_story(nab_index: u32, patch: &dyn Fn(&mut [u8], &[usize])) -> Memory {
        let mut img = vec![0u8; 0x2000];
        img[0..4].copy_from_slice(b"Glul");
        img[4..8].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[8..12].copy_from_slice(&SYN_RAMSTART.to_be_bytes());
        img[12..16].copy_from_slice(&0x2000u32.to_be_bytes()); // EXTSTART
        img[16..20].copy_from_slice(&0x2000u32.to_be_bytes()); // ENDMEM
        img[20..24].copy_from_slice(&4096u32.to_be_bytes());
        img[24..28].copy_from_slice(&36u32.to_be_bytes()); // start function
        let at = plant_addresses();
        for (t, &base) in TEMPLATES.iter().zip(at.iter()) {
            img[base..base + t.bytes.len()].copy_from_slice(t.bytes);
        }
        for (ti, t) in TEMPLATES.iter().enumerate() {
            for cap in &t.decode().captures {
                let v = match cap.slot {
                    Slot::Call(n) => {
                        Some(at[TEMPLATES.iter().position(|x| x.num == n).unwrap()] as u32)
                    }
                    Slot::Param(p) => Some(SYN_PARAMS[p as usize]),
                    Slot::ParamPlus(p, k) => Some(SYN_PARAMS[p as usize] + k),
                    Slot::ParamRam(p) => Some(SYN_PARAMS[p as usize] - SYN_RAMSTART),
                    Slot::NabWordIndex => Some(nab_index),
                    Slot::RtErr | Slot::Message => None,
                };
                if let Some(v) = v {
                    put(&mut img, at[ti] + cap.off, cap.len, v);
                }
            }
        }
        // The object world the cross-checks read.
        for (i, a) in [0x1100usize, 0x1120, 0x1140, 0x1160].iter().enumerate() {
            img[*a] = 0x70;
            put(&mut img, 0x1080 + 4 * i, 4, *a as u32);
        }
        put(&mut img, 0x1090, 4, 0x1180); // classes_table[4]: a genuine class
        img[0x1180] = 0x70;
        put(&mut img, 0x1180 + 20, 4, 0x1100); // …of class Class at 13+nab
        patch(&mut img, &at);
        Memory::new(img).expect("synthetic story is well-formed")
    }

    /// The byte offset and width of one declared slot within its template.
    fn slot_at(num: u32, instr: u16, operand: u8) -> (usize, usize) {
        let t = TEMPLATES.iter().find(|t| t.num == num).unwrap();
        assert!(
            t.slots.iter().any(|(i, o, _)| *i == instr && *o == operand),
            "{}: instruction {instr} operand {operand} is not a declared slot",
            t.name
        );
        let mem = template_memory(t.bytes);
        let mut pc = 36 + body_offset(t.bytes) as u32;
        let limit = 36 + t.bytes.len() as u32;
        let mut idx = 0u16;
        while pc < limit {
            let ins = decode_instr(&mem, pc).unwrap();
            if idx == instr {
                let op = &ins.operands[operand as usize];
                return ((op.data_addr - 36) as usize, operand_data_len(op.mode) as usize);
            }
            pc = ins.next;
            idx += 1;
        }
        panic!("{}: instruction {instr} is past the end of the template", t.name)
    }

    #[test]
    fn planted_veneer_is_matched_and_its_params_derived() {
        let mem = synthetic_story(5, &|_, _| {});
        let at = plant_addresses();
        let (report, install) = fingerprint(&mem);
        assert!(report.installed(), "not installed: {:?}", report.rejected);
        assert_eq!(report.matched.len(), 7);
        assert_eq!(report.matched[0], (1, "Z__Region", at[0] as u32));
        assert_eq!(report.params, SYN_PARAMS);
        assert!(report.checks.iter().all(|c| c.passed), "{:?}", report.checks);
        let (assignments, params) = install.expect("install payload");
        assert_eq!(assignments.len(), 7);
        assert_eq!(assignments.get(&(at[2] as u32)), Some(&RA_PR));
        assert_eq!(params, SYN_PARAMS);
        assert!(report.summary().contains("RA__Pr@"), "{}", report.summary());
    }

    #[test]
    fn one_changed_non_address_byte_breaks_the_match() {
        // Flip a byte the mask does NOT cover — the first such byte inside
        // `RA__Pr`'s body. Addresses in the planted story already differ from the
        // reference story's in every masked slot and it still matched above, so
        // this isolates the unmasked half of the contract.
        let d = TEMPLATES.iter().find(|t| t.num == RA_PR).unwrap().decode();
        let body = body_offset(d.bytes);
        let flip = (body..d.bytes.len()).find(|i| d.compare[*i]).expect("some byte compares");
        let mem = synthetic_story(5, &|img, at| img[at[2] + flip] ^= 0x01);
        let report = fingerprint(&mem).0;
        assert!(!report.installed(), "a mutated body must not match");
        assert!(
            report.rejected.as_deref().unwrap_or_default().contains("RA__Pr"),
            "{:?}",
            report.rejected
        );
    }

    #[test]
    fn a_four_byte_constant_that_is_not_an_address_is_part_of_the_fingerprint() {
        // `RA__Pr` opens with `bitand id, #$ffff0000` — the property-id class
        // mask. It is a four-byte constant, so it *looks* like an address, but
        // changing it changes what the routine computes and must break the match.
        let d = TEMPLATES.iter().find(|t| t.num == RA_PR).unwrap().decode();
        let mask_at = d.bytes.windows(4).position(|w| w == [0xff, 0xff, 0x00, 0x00]).unwrap();
        assert!(d.compare[mask_at], "the $ffff0000 constant must be compared");
        let mem = synthetic_story(5, &|img, at| img[at[2] + mask_at] = 0xfe);
        assert!(!fingerprint(&mem).0.installed());
    }

    #[test]
    fn a_call_target_that_leaves_the_matched_set_is_refused() {
        // `RA__Pr` instruction 9 calls `CP__Tab`. Point it elsewhere: the body
        // still matches (call targets are masked) but the call graph no longer
        // closes on the matched routines.
        let (off, len) = slot_at(RA_PR, 9, 0);
        let mem = synthetic_story(5, &|img, at| put(img, at[2] + off, len, 0x999));
        let report = fingerprint(&mem).0;
        assert!(!report.installed());
        let why = report.rejected.unwrap_or_default();
        assert!(why.contains("RA__Pr calls"), "{why}");
    }

    #[test]
    fn disagreeing_parameter_reads_are_refused() {
        // `class_metaclass` is read in `RA__Pr`, `RL__Pr`, `OC__Cl` and
        // `OP__Pr`. Change only the `RA__Pr` copy.
        let (off, len) = slot_at(RA_PR, 13, 1);
        let mem = synthetic_story(5, &|img, at| put(img, at[2] + off, len, 0x1104));
        let report = fingerprint(&mem).0;
        assert!(!report.installed());
        let why = report.rejected.unwrap_or_default();
        assert!(why.contains("accelparam 2"), "{why}");
    }

    #[test]
    fn a_cross_check_failure_stops_acceleration() {
        // Move the class-chain word index from 5 to 6 in every routine that
        // reads it. Nothing in the bytecode is inconsistent — all four reads
        // agree, so `num_attr_bytes` derives cleanly as 11 — and only the story's
        // own object table can tell that it is wrong.
        assert!(fingerprint(&synthetic_story(5, &|_, _| {})).0.installed());
        let report = fingerprint(&synthetic_story(6, &|_, _| {})).0;
        assert_eq!(report.params[7], 11, "the wrong nab must still derive cleanly");
        assert!(!report.installed(), "a wrong num_attr_bytes must not accelerate");
        let why = report.rejected.unwrap_or_default();
        assert!(why.contains("cross-check failed"), "{why}");
        assert!(why.contains("first class is of class Class"), "{why}");
    }

    #[test]
    fn a_second_copy_of_a_routine_is_an_ambiguity_we_refuse() {
        // Plant `Z__Region` a second time in ROM. Two candidates for one
        // accelerated function is not a thing to pick between.
        let mem = synthetic_story(5, &|img, _| {
            let z = TEMPLATES[0].bytes;
            img[0xa00..0xa00 + z.len()].copy_from_slice(z);
        });
        let report = fingerprint(&mem).0;
        assert!(!report.installed());
        assert!(
            report.rejected.as_deref().unwrap_or_default().contains("more than one address"),
            "{:?}",
            report.rejected
        );
    }

    #[test]
    fn a_story_with_no_veneer_reports_no_match_and_installs_nothing() {
        let mut img = vec![0u8; 0x1000];
        img[0..4].copy_from_slice(b"Glul");
        img[4..8].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[8..12].copy_from_slice(&0x800u32.to_be_bytes());
        img[12..16].copy_from_slice(&0x1000u32.to_be_bytes());
        img[16..20].copy_from_slice(&0x1000u32.to_be_bytes());
        img[20..24].copy_from_slice(&4096u32.to_be_bytes());
        img[24..28].copy_from_slice(&36u32.to_be_bytes());
        let mem = Memory::new(img).unwrap();
        let (report, install) = fingerprint(&mem);
        assert!(install.is_none());
        assert!(!report.installed());
        assert!(report.summary().contains("not applied"), "{}", report.summary());
    }
}
