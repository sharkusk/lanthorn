//! In-crate hostile-input fuzzing harness (SQ-1502) for the picture and
//! table readers added 2026-09-10: [`crate::zx_mysterious`],
//! [`crate::saga_us`], [`crate::saga_atari`], [`crate::saga_dos`],
//! [`crate::apple_pictures`], and the dialect-dispatching loader
//! ([`crate::Database::parse`], [`crate::detect_dialect`]).
//!
//! Same shape as `zvm`'s `fuzz_harness.rs` (SQ-1407): a hand-rolled
//! xorshift64 PRNG (zero dependencies, per the crate's hard rule) drives a
//! few thousand fixed-seed random and mutated inputs through every public
//! decode entry point these modules expose, asserting only three things —
//! no panic, no hang, and (implicitly, since every allocation size here is
//! bounded by the corresponding count field) no runaway allocation. It runs
//! under `cargo nextest`/`cargo test` and therefore on every CI push.
//!
//! **Real specimen bytes, where this repo has them.** `stories/scott-dialects/`
//! (gitignored, commercial disk/tape images — see that directory's own
//! READMEs) carries pre-extracted database arrays for the three US S.A.G.A.
//! platforms (`atari/db/*.bin`, `apple/db/*.bin`, `c64/db/hulk.bin` — exactly
//! the bytes [`crate::parse_saga_us`] wants, unshifted), whole Atari
//! companion-picture ATR sides (`atari/*.atr`), ZX Spectrum `.z80` snapshots
//! (`spectrum/*.z80`), and reference-format `.dat` text databases
//! (`mysterious-dat/*.dat`). Every case here mutates those where they exist
//! and skips vacuously — loudly, on stderr — when they don't, exactly like
//! the specimen suites under `tests/`. There is no committed family-D
//! (`M2`/scrambled-record) or family-E (`.PAK`) specimen in this set — those
//! two get structured random synthesis instead (a real header, garbage
//! body), the same shape `zvm`'s `random_image` uses for a Z-machine header.
//!
//! Seeds are fixed (`BASE_SEED..`) so a failure reproduces; set
//! `LANTHORN_FUZZ_SEEDS=N` to run more of them locally than the default.

use crate::apple_pictures::{
    apple_look_table, decode_family_d, decode_family_d_plain, decode_family_d_scrambled,
    scan_scrambled_pictures,
};
use crate::saga_atari::{decode_record, record_at, scan_picture_side, splice_vtoc};
use crate::saga_dos::{decode_family_e, looks_like_family_e, MAGIC as FAMILY_E_MAGIC};
use crate::saga_pictures::FamilyCScheme;
use crate::saga_us::{detect_saga_us, looks_like_saga_us, parse_saga_us, SagaPlatform};
use crate::z80::decompress_z80;
use crate::zx_mysterious::{
    decode_picture_lists, decode_picture_lists_z80, identify, identify_z80, locate,
    looks_like_zx_mysterious, looks_like_zx_mysterious_z80, parse_zx_mysterious,
    parse_zx_mysterious_z80,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Minimal xorshift64 PRNG. No external dependency — see the crate's hard
/// rule (`CLAUDE.md`: "zvm, gvm, and scott take ZERO external dependencies").
/// Same algorithm `zvm::fuzz_harness` uses (public-domain xorshift, not
/// shared code — this crate takes no dependency on `zvm` either).
struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        // xorshift64 never advances from state 0.
        XorShift64(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A uniform value in `0..n` (0 if `n == 0`).
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    fn fill(&mut self, buf: &mut [u8]) {
        let mut i = 0;
        while i < buf.len() {
            let r = self.next_u64().to_le_bytes();
            let n = (buf.len() - i).min(8);
            buf[i..i + n].copy_from_slice(&r[..n]);
            i += n;
        }
    }
}

/// Per-input wall-clock budget — a hang guard, not a performance bound. Every
/// decoder here is a single bounded pass over a byte slice with no read loop
/// that can outrun its own input (unlike a stepped VM), so this is deliberately
/// generous and exists only to turn a genuine infinite loop into a failing
/// test instead of a wedged CI runner.
const PER_INPUT_BUDGET: Duration = Duration::from_secs(5);

/// Base of the fixed seed range every case below walks a slice of.
const BASE_SEED: u64 = 0x5CA7_0000_0000_0001;

/// `LANTHORN_FUZZ_SEEDS=N` overrides the default seed count for a longer
/// local run; unset or unparsable falls back to `default`.
fn seed_count(default: usize) -> usize {
    std::env::var("LANTHORN_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// The same three candidates every `tests/*_specimens.rs` suite tries, in the
/// same order — the environment override, the workspace-root path (when the
/// test binary's own cwd already is the workspace root), and the path
/// relative to this crate's directory (the normal case: cargo sets a test
/// binary's cwd to its package root, `crates/scott`).
fn fixtures_dir() -> Option<PathBuf> {
    [
        std::env::var_os("SCOTT_DIALECT_FIXTURES").map(PathBuf::from),
        Some(PathBuf::from("stories/scott-dialects")),
        Some(PathBuf::from("../../stories/scott-dialects")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_dir())
}

/// Every regular file directly under `stories/scott-dialects/<subdir>`,
/// skipping dotfiles and the directory's own `README.txt` — empty (not an
/// error) when the fixture tree, or this one subdirectory, is absent.
fn specimen_files(subdir: &str) -> Vec<Vec<u8>> {
    let Some(base) = fixtures_dir() else { return Vec::new() };
    let dir = base.join(subdir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if name.starts_with('.') || name.eq_ignore_ascii_case("README.txt") {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            out.push(bytes);
        }
    }
    out.sort_by_key(|b| b.len()); // deterministic order across platforms
    out
}

/// A committed-or-fetched specimen with random damage: a run of byte flips, a
/// random truncation, or a random block splice. Same three mutation kinds
/// `zvm::fuzz_harness::mutated_fixture` uses.
fn mutate(seed: u64, original: &[u8]) -> Vec<u8> {
    let mut rng = XorShift64::new(seed);
    let mut buf = original.to_vec();
    if buf.is_empty() {
        return buf;
    }
    match rng.below(3) {
        0 => {
            let flips = 1 + rng.below(64) as usize;
            for _ in 0..flips {
                let idx = rng.below(buf.len() as u64) as usize;
                buf[idx] ^= (1 + rng.below(255)) as u8;
            }
        }
        1 => {
            let cut = 1 + rng.below(buf.len() as u64) as usize;
            buf.truncate(cut);
        }
        _ => {
            let start = rng.below(buf.len() as u64) as usize;
            let splice_len = 1 + rng.below((buf.len() - start).max(1) as u64) as usize;
            let mut junk = vec![0u8; splice_len];
            rng.fill(&mut junk);
            buf[start..start + splice_len].copy_from_slice(&junk);
        }
    }
    buf
}

/// Pure random byte soup: no real specimen behind it at all, the other half
/// of "mutation of the real bytes plus random byte soup".
fn random_soup(seed: u64, max_len: usize) -> Vec<u8> {
    let mut rng = XorShift64::new(seed);
    let len = rng.below(max_len as u64) as usize;
    let mut buf = vec![0u8; len];
    rng.fill(&mut buf);
    buf
}

/// Run `body` under `catch_unwind` AND a wall-clock budget for every `(seed,
/// bytes)` pair `inputs` yields, so one bad seed doesn't stop the sweep —
/// every seed runs, and every failure (panic or hang) is collected before the
/// case fails once, listing every seed that found one. Mirrors
/// `zvm::fuzz_harness::run_seeds`.
fn run_seeds(label: &str, inputs: impl Iterator<Item = (u64, Vec<u8>)>, body: impl Fn(&[u8]) + std::panic::RefUnwindSafe) {
    let mut failures = Vec::new();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // one bad seed shouldn't spam stderr thousands of times
    for (seed, bytes) in inputs {
        let start = Instant::now();
        let result = std::panic::catch_unwind(|| body(&bytes));
        match result {
            Ok(()) => {
                if start.elapsed() > PER_INPUT_BUDGET {
                    failures.push(format!("seed {seed:#x}: exceeded {PER_INPUT_BUDGET:?} budget"));
                }
            }
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "<non-string panic payload>".to_string());
                failures.push(format!("seed {seed:#x}: PANIC: {msg}"));
            }
        }
    }
    std::panic::set_hook(prev_hook);
    assert!(failures.is_empty(), "{label}: {} failing seed(s):\n{}", failures.len(), failures.join("\n"));
}

/// Cycle `n` seeds across `specimens`, mutating a different specimen each
/// time (round-robin) so a run with few specimens still gets `n` distinct
/// mutations rather than reusing one every time.
fn mutated_seeds(base: u64, n: usize, specimens: &[Vec<u8>]) -> Vec<(u64, Vec<u8>)> {
    if specimens.is_empty() {
        return Vec::new();
    }
    (0..n as u64)
        .map(|i| {
            let seed = base + i;
            let original = &specimens[(i as usize) % specimens.len()];
            (seed, mutate(seed, original))
        })
        .collect()
}

fn soup_seeds(base: u64, n: usize, max_len: usize) -> Vec<(u64, Vec<u8>)> {
    (0..n as u64).map(|i| (base + i, random_soup(base + i, max_len))).collect()
}

// ── zx_mysterious ────────────────────────────────────────────────────────────

/// The `.z80` decode path — `looks_like_zx_mysterious_z80`, `identify_z80`,
/// `parse_zx_mysterious_z80`, `decode_picture_lists_z80` — against mutated
/// real Spectrum snapshots and random soup.
#[test]
fn zx_mysterious_z80_path_survives_hostile_input() {
    let specimens = specimen_files("spectrum");
    if specimens.is_empty() {
        eprintln!("SKIP: no stories/scott-dialects/spectrum — see this file's module docs");
        return;
    }
    let n = seed_count(800);
    let inputs = mutated_seeds(BASE_SEED, n, &specimens)
        .into_iter()
        .chain(soup_seeds(BASE_SEED + 0x1000_0000, n / 4, 0x1_2000));
    run_seeds("zx_mysterious_z80_path_survives_hostile_input", inputs, |bytes| {
        let _ = looks_like_zx_mysterious_z80(bytes);
        let _ = identify_z80(bytes);
        let _ = parse_zx_mysterious_z80(bytes);
        let _ = decode_picture_lists_z80(bytes);
    });
}

/// The decompressed-image decode path — `looks_like_zx_mysterious`,
/// `identify`, `locate`, `parse_zx_mysterious`, `decode_picture_lists` — fed a
/// mutated REAL 48K image (a genuine specimen decompressed once, then
/// damaged), which reaches table-address logic no `.z80`-shaped mutation
/// reliably does, since RLE-compressed bytes rarely decompress to anything
/// structured at all.
#[test]
fn zx_mysterious_image_path_survives_hostile_input() {
    let specimens = specimen_files("spectrum");
    let images: Vec<Vec<u8>> =
        specimens.iter().filter_map(|z80| decompress_z80(z80).ok()).collect();
    if images.is_empty() {
        eprintln!("SKIP: no stories/scott-dialects/spectrum — see this file's module docs");
        return;
    }
    let n = seed_count(800);
    let inputs = mutated_seeds(BASE_SEED + 0x2000_0000, n, &images)
        .into_iter()
        .chain(soup_seeds(BASE_SEED + 0x3000_0000, n / 4, crate::z80::IMAGE_LEN + 4096));
    run_seeds("zx_mysterious_image_path_survives_hostile_input", inputs, |bytes| {
        let _ = looks_like_zx_mysterious(bytes);
        let _ = identify(bytes);
        let _ = locate(bytes);
        let _ = parse_zx_mysterious(bytes);
        let _ = decode_picture_lists(bytes);
    });
}

// ── saga_us ──────────────────────────────────────────────────────────────────

/// `looks_like_saga_us`, `detect_saga_us`, `parse_saga_us` against every
/// platform's real, unshifted database array (`atari/db`, `apple/db`,
/// `c64/db/hulk.bin`) mutated, plus random soup at each platform's own array
/// offset.
#[test]
fn saga_us_decoders_survive_hostile_input() {
    let mut any = false;
    for (subdir, platform, salt) in [
        ("atari/db", SagaPlatform::Atari8Bit, 0x4000_0000u64),
        ("apple/db", SagaPlatform::AppleII, 0x5000_0000u64),
        ("c64/db", SagaPlatform::Commodore64, 0x6000_0000u64),
    ] {
        let specimens = specimen_files(subdir);
        if specimens.is_empty() {
            continue;
        }
        any = true;
        let n = seed_count(600);
        let inputs = mutated_seeds(BASE_SEED + salt, n, &specimens).into_iter().chain(soup_seeds(
            BASE_SEED + salt + 0x0100_0000,
            n / 4,
            platform.array_offset() + 0x4000,
        ));
        run_seeds(&format!("saga_us_decoders_survive_hostile_input[{subdir}]"), inputs, |bytes| {
            let _ = looks_like_saga_us(bytes, platform);
            let _ = detect_saga_us(bytes);
            let _ = parse_saga_us(bytes, platform);
        });
    }
    if !any {
        eprintln!("SKIP: no stories/scott-dialects/{{atari,apple,c64}}/db — see this file's module docs");
    }
}

// ── saga_atari ───────────────────────────────────────────────────────────────

/// `splice_vtoc`, `scan_picture_side`, `record_at`, `decode_record` against
/// mutated real Atari companion-picture ATR sides (every one of the fourteen
/// side files, family-C bitmap or line-art alike — the decoder must not
/// assume it was handed a bitmap side) and random soup.
#[test]
fn saga_atari_decoders_survive_hostile_input() {
    let specimens = specimen_files("atari")
        .into_iter()
        .filter(|b| b.len() > 16) // skip the `db/*.bin` array files if ever nested here
        .collect::<Vec<_>>();
    if specimens.is_empty() {
        eprintln!("SKIP: no stories/scott-dialects/atari — see this file's module docs");
        return;
    }
    let n = seed_count(500);
    let inputs = mutated_seeds(BASE_SEED + 0x7000_0000, n, &specimens)
        .into_iter()
        .chain(soup_seeds(BASE_SEED + 0x7100_0000, n / 4, 100_000));
    run_seeds("saga_atari_decoders_survive_hostile_input", inputs, |bytes| {
        let spliced = splice_vtoc(bytes);
        for scheme in [FamilyCScheme::Standard, FamilyCScheme::NoLiteral] {
            let found = scan_picture_side(bytes, scheme);
            // Every record `scan_picture_side` reports must itself decode
            // without panicking — the same records the scanner already
            // proved self-consistent, but decoding is a second pass over the
            // same bytes and worth exercising directly too.
            for record in &found {
                let _ = decode_record(&spliced, record, scheme);
            }
            // And a handful of arbitrary offsets into the spliced bytes,
            // which `scan_picture_side` itself never tries once it has found
            // a valid record at an earlier offset.
            let mut rng = XorShift64::new(bytes.len() as u64 ^ 0xA5A5);
            for _ in 0..8 {
                if spliced.is_empty() {
                    break;
                }
                let offset = rng.below(spliced.len() as u64) as usize;
                let _ = record_at(&spliced, offset, scheme);
            }
        }
    });
}

// ── saga_dos ─────────────────────────────────────────────────────────────────

/// A structurally-plausible family-E header (real [`FAMILY_E_MAGIC`] bytes,
/// everything else random) — the `zvm::fuzz_harness::random_image` shape,
/// used here because no `.PAK` specimen is committed (the MS-DOS releases
/// under `stories/scott-dialects/msdos/` are zipped, and this crate takes no
/// dependency that could unzip one).
fn synthetic_family_e_header(seed: u64) -> Vec<u8> {
    let mut rng = XorShift64::new(seed);
    let len = 0x17 + rng.below(4096) as usize;
    let mut buf = vec![0u8; len];
    rng.fill(&mut buf);
    for (i, b) in FAMILY_E_MAGIC.iter().enumerate() {
        if let Some(b) = b {
            buf[i] = *b;
        }
    }
    buf
}

/// `looks_like_family_e` and `decode_family_e` against structured random
/// headers (real magic, garbage body — see [`synthetic_family_e_header`]) and
/// pure random soup.
#[test]
fn saga_dos_decode_family_e_survives_hostile_input() {
    let n = seed_count(1500);
    let inputs = (0..n as u64)
        .map(|i| (BASE_SEED + 0x8000_0000 + i, synthetic_family_e_header(BASE_SEED + 0x8000_0000 + i)))
        .chain(soup_seeds(BASE_SEED + 0x8100_0000, n, 8192));
    run_seeds("saga_dos_decode_family_e_survives_hostile_input", inputs, |bytes| {
        let _ = looks_like_family_e(bytes);
        let _ = decode_family_e(bytes);
    });
}

// ── apple_pictures ───────────────────────────────────────────────────────────

/// `scan_scrambled_pictures`, `apple_look_table`, and (over the ranges the
/// scan finds, and over pure random slices) `decode_family_d` and its two
/// sub-variants, against mutated real Apple II boot-side `.dsk` images —
/// whole disks, not extracted records, which is exactly the shape
/// `scan_scrambled_pictures` and `apple_look_table` themselves take (a
/// full-image scan for a marker), and a source of realistic byte STATISTICS
/// (real 6502 code and real picture data) for the record decoders even
/// though no single slice is guaranteed to be a valid record.
#[test]
fn apple_pictures_decoders_survive_hostile_input() {
    let disks: Vec<Vec<u8>> = specimen_files("apple")
        .into_iter()
        .filter(|b| b.len() > 100_000) // the `.dsk` images, not the `db/*.bin` arrays or `.txt` transcripts
        .collect();
    if disks.is_empty() {
        eprintln!("SKIP: no stories/scott-dialects/apple .dsk images — see this file's module docs");
        return;
    }
    let n = seed_count(400);
    let inputs = mutated_seeds(BASE_SEED + 0x9000_0000, n, &disks)
        .into_iter()
        .chain(soup_seeds(BASE_SEED + 0x9100_0000, n / 4, 200_000));
    run_seeds("apple_pictures_decoders_survive_hostile_input", inputs, |bytes| {
        let ranges = scan_scrambled_pictures(bytes);
        for range in ranges.iter().take(32) {
            let record = &bytes[range.clone()];
            let _ = decode_family_d(record, SagaPlatform::AppleII);
            let _ = decode_family_d_scrambled(record, SagaPlatform::AppleII);
            let _ = decode_family_d_plain(record, SagaPlatform::AppleII);
        }
        let _ = apple_look_table(bytes);
        // A handful of arbitrary slices too, the same complement
        // `saga_atari_decoders_survive_hostile_input` gives `record_at`: real
        // byte statistics at offsets the marker scan never lands on.
        let mut rng = XorShift64::new(bytes.len() as u64 ^ 0x5A5A);
        for _ in 0..8 {
            if bytes.is_empty() {
                break;
            }
            let start = rng.below(bytes.len() as u64) as usize;
            let len = rng.below((bytes.len() - start) as u64 + 1) as usize;
            let slice = &bytes[start..start + len];
            let _ = decode_family_d(slice, SagaPlatform::AppleII);
            let _ = decode_family_d_scrambled(slice, SagaPlatform::AppleII);
            let _ = decode_family_d_plain(slice, SagaPlatform::AppleII);
        }
    });
}

// ── The loader ───────────────────────────────────────────────────────────────

/// [`crate::Database::parse`] and [`crate::detect_dialect`] — the one entry
/// point a host actually calls when it doesn't already know which of the
/// five encodings a file is — against mutated real specimens from EVERY
/// dialect this crate reads or names (reference-format `.dat` text, the ZX
/// Spectrum and Apple II/Atari/Commodore 64 binaries), so a mutation that
/// looks like one dialect's signature but decodes as garbage under another
/// dialect's reader is exercised too. Plus random soup.
#[test]
fn loader_parse_survives_hostile_input_across_every_dialect() {
    let mut specimens = Vec::new();
    specimens.extend(specimen_files("mysterious-dat"));
    specimens.extend(specimen_files("spectrum"));
    specimens.extend(specimen_files("atari/db"));
    specimens.extend(specimen_files("apple/db"));
    specimens.extend(specimen_files("c64/db"));
    if specimens.is_empty() {
        eprintln!("SKIP: no stories/scott-dialects — see this file's module docs");
        return;
    }
    let n = seed_count(1200);
    let inputs = mutated_seeds(BASE_SEED + 0xA000_0000, n, &specimens)
        .into_iter()
        .chain(soup_seeds(BASE_SEED + 0xA100_0000, n / 3, 20_000));
    run_seeds("loader_parse_survives_hostile_input_across_every_dialect", inputs, |bytes| {
        let _ = crate::detect_dialect(bytes);
        let _ = crate::Database::parse(bytes);
    });
}
