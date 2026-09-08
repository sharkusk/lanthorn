//! Disassembly-quality audit (SQ-0463): measure how the tiered disassembler
//! classifies the code region on v6 stories vs. real v3/v5 baselines.
//!
//! Background: a v6 story's header word 0x06 is the *packed address of the
//! `main` routine*, not a direct PC (the interpreter calls it — ZMSD §5.5).
//! A disassembler that used the raw word as its reachability root (the old
//! bug) rooted `code_region` and `discover_rd` at roughly a quarter of the
//! real entry address — pulling tens of KB of static data into the "code
//! region" (linear-scanned as phantom routines) and seeding recursive descent
//! at a non-instruction. This harness measures the per-tier byte share so that
//! regression can't return silently.
//!
//! Two things this harness pins down:
//!   * The `.byte` (Data) tier stays low on v6 — no wholesale desync of the
//!     instruction stream (the classic corruption signature).
//!   * A v6 region roots at high memory, not the tiny raw packed word.
//!
//! Note on the Rd (hard/recursive-descent) tier: it is naturally LOW on real
//! games of any version — most routines are reached indirectly (action/grammar
//! tables, object properties), which RD cannot follow, so the linear-scan Soft
//! tier carries them. Real v3 minizork sits near the v6 stories; only test
//! harnesses that call everything directly (czech, praxix) show a high Rd
//! share. So this harness does NOT assert on Rd — only on Data and on the v6
//! region root.
//!
//! Stories live in the repo's `stories/` dir (not the zvm fixtures dir) and are
//! skipped when absent, so this test passes cleanly on a checkout without them.

use std::path::PathBuf;
use zvm::cpu::disasm_cache::{code_region, DisasmCache, Provenance, Unit};
use zvm::memory::Memory;

/// Load a story from the workspace `stories/` dir (two levels up from this
/// crate's manifest). `None` when the file is absent (skip cleanly).
fn load_story(name: &str) -> Option<Vec<u8>> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates/
    path.pop(); // workspace root
    path.push("stories");
    path.push(name);
    std::fs::read(&path).ok()
}

/// Per-provenance byte totals over the tiled code region.
struct Density {
    region_bytes: u32,
    data_bytes: u32,
    soft_bytes: u32,
    rd_bytes: u32,
}

impl Density {
    fn of(cache: &DisasmCache) -> Density {
        let (rstart, rend) = cache.region();
        let mut d = Density { region_bytes: rend - rstart, data_bytes: 0, soft_bytes: 0, rd_bytes: 0 };
        for u in cache.units() {
            let len = u.end() - u.addr();
            match u.provenance() {
                Provenance::Data => d.data_bytes += len,
                Provenance::Soft => d.soft_bytes += len,
                Provenance::Rd => d.rd_bytes += len,
                _ => {}
            }
            // Data units should never claim to be code.
            if matches!(u, Unit::Data { .. }) {
                assert_eq!(u.provenance(), Provenance::Data);
            }
        }
        d
    }
    fn data_pct(&self) -> f64 { 100.0 * self.data_bytes as f64 / self.region_bytes as f64 }
    fn soft_pct(&self) -> f64 { 100.0 * self.soft_bytes as f64 / self.region_bytes as f64 }
    fn rd_pct(&self) -> f64 { 100.0 * self.rd_bytes as f64 / self.region_bytes as f64 }
}

fn report(label: &str, cache: &DisasmCache) -> Density {
    let d = Density::of(cache);
    let (rstart, rend) = cache.region();
    eprintln!(
        "{label:<28} region {:#08x}..{:#08x} ({:>6} B) | Data {:>5.1}%  Soft {:>5.1}%  Rd {:>5.1}%",
        rstart, rend, d.region_bytes, d.data_pct(), d.soft_pct(), d.rd_pct()
    );
    d
}

/// The four local v6 stories. Missing ones skip.
const V6_STORIES: &[&str] = &[
    "zork0-r393-s890714.z6",
    "shogun-r322-s890706.z6",
    "arthur-r74-s890714.z6",
    "journey-r83-s890706.z6",
];

#[test]
fn report_static_disasm_density_v6_vs_baselines() {
    eprintln!("\n=== STATIC DisasmCache::build density (no execution) ===");

    // Baselines: a real v3 game (indirect-call heavy, like the v6 titles) and
    // two direct-call test harnesses (high Rd) for contrast.
    let mut max_data_baseline = 0.0f64;
    for f in ["minizork.z3", "czech.z5", "praxix.z5"] {
        if let Some(bytes) = zvm::fixtures::load(f) {
            let mem = Memory::new(bytes).unwrap();
            let cache = DisasmCache::build(&mem);
            let d = report(f, &cache);
            max_data_baseline = max_data_baseline.max(d.data_pct());
        }
    }

    for name in V6_STORIES {
        let Some(bytes) = load_story(name) else {
            eprintln!("skipping (absent): {name}");
            continue;
        };
        let mem = Memory::new(bytes).unwrap();
        assert_eq!(mem.version(), 6, "{name} is not a v6 story");

        // The v6 region must root at high memory — NOT the tiny raw packed
        // header-0x06 word (the old bug made region_start ≈ raw_word/4, ~tens
        // of KB below high memory, swallowing static data as phantom code).
        let high_mem = mem.read_word(0x04) as u32;
        let raw06 = mem.read_word(0x06) as u32;
        let (rstart, _rend) = code_region(&mem);
        assert_eq!(
            rstart, high_mem,
            "{name}: v6 region_start {rstart:#x} should be high-mem {high_mem:#x}, \
             not derived from the raw packed word {raw06:#x}"
        );
        assert!(
            rstart > raw06,
            "{name}: region_start {rstart:#x} still tracks the raw packed word {raw06:#x} \
             — the v6 reachability root is unfixed"
        );

        let cache = DisasmCache::build(&mem);
        let d = report(name, &cache);

        // Data (`.byte`) tier stays low: no wholesale desync of the instruction
        // stream. Absolute bound (documented, not a brittle exact number) plus a
        // "within a small margin of the worst real-game baseline" ratchet.
        assert!(
            d.data_pct() < 15.0,
            "{name}: {:.1}% of the code region is .byte data — the v6 disassembler \
             looks desynced from the instruction stream",
            d.data_pct()
        );
        assert!(
            d.data_pct() <= max_data_baseline.max(1.0) * 2.0 + 5.0,
            "{name}: Data {:.1}% far exceeds the real-game baseline ceiling {:.1}%",
            d.data_pct(), max_data_baseline
        );
    }
}
