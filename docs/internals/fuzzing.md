# Hostile-input fuzzing for `zvm` and `gvm`

[← back to README](../../README.md) · see also [Reviewing `zvm` as a crate someone else depends on](zvm-embedding-review.md) (§6 "Errors, panics and what a hostile story file can do", and the ledger's item 13)

A story file, a save file, or a `@restore`/Save-State blob is untrusted input
the moment it comes from anywhere but the interpreter's own hand — a
downloaded game, a save shared between players, a disk image pulled off
archive.org. `zvm` and `gvm` are meant to survive all of it without panicking:
a hostile or merely corrupt file should fault the machine or refuse to load,
never crash the host process. SQ-1014's robustness pass found real bugs this
way — `print_table` and `DisasmCache` — with a throwaway fuzzer run by hand
for a few minutes; the caps it led to landed as SQ-1395, but no repeatable
harness did, so the only adversarial regression test in the repo was
`crates/zvm/tests/object_scan_eof.rs` (`czech.z5`, found by accident). SQ-1407
is that harness, in two halves that answer different questions.

## The two halves

**Half 1 — the in-crate regression guard** (`crates/zvm/src/fuzz_harness.rs`,
`crates/gvm/src/fuzz_harness.rs`) runs on every `cargo test`/`cargo nextest`
invocation and therefore on every CI push. It is deliberately not
coverage-guided: a hand-rolled xorshift64 PRNG (zero external dependencies —
the crate's hard rule extends to its own tests) generates a few thousand
fixed-seed images and drives each through `step()`, the restore paths, and the
disassemblers, asserting no panic and no hang. Fixed seeds mean a failure
reproduces exactly by re-running the suite; there is no separate "known
failing seeds" list to keep in sync.

**Half 2 — the coverage-guided sweep** (`crates/fuzz/`) is `cargo-fuzz`
(libFuzzer) targets that mutate toward whatever byte patterns reach new code,
which finds inputs no hand-rolled generator would think to try. It needs a
nightly toolchain and runs for however long a person chooses to leave it
going, so it is not part of CI — it is a tool you reach for by hand, and any
crash it finds becomes a permanent, fast, CI-gated case in Half 1.

## Half 1: the in-crate harness

Both `fuzz_harness.rs` files share the same shape:

- `XorShift64`: `new(seed)`, `next_u64()`, `below(n)`, `fill(&mut [u8])`. No
  dependency, no `rand` crate — matches the crate's zero-dependency rule.
- `random_image(seed, ...)`: a header valid enough for `Memory::new` to accept
  it (`zvm`: version 3-8, `static_mem_base` inside the buffer; `gvm`: the
  `Glul` magic, a 2 or 3 major version, a 256-aligned RAMSTART ≤ EXTSTART ≤
  ENDMEM, a 256-aligned stack size, START FUNC inside the memory map — the
  facts each crate's own loader actually checks), followed by fully random
  bytes everywhere else, including the rest of the header. That is the
  hostile case: a structurally-loadable image whose every other field, and
  whose code, is garbage.
- `mutated_fixture(seed, original)`: a committed, freely redistributable
  fixture with random damage — a run of byte flips, a random truncation, or a
  random block splice. This finds a different class of bug than pure noise
  does: a plausible-but-corrupt header or table, rather than something
  `parse_header` already refuses outright.
- `drive(bytes, seed)`: boots the image and calls `step()` up to a bounded
  step count, answering `NeedLine`/`NeedChar` with random text/keys,
  `SaveRequest`/`RestoreRequest` with a random completion, and stopping on
  `Quit`/`Fault`/the step cap. A per-story wall-clock budget bounds how long
  any single image gets; exceeding it fails that seed as a hang.
- `run_seeds(...)`: runs every seed under `std::panic::catch_unwind` (with the
  panic hook silenced first) so ONE bad seed doesn't stop the sweep or spam
  stderr — every seed runs, every failure is collected, and the case fails
  once at the end listing every failing seed.

Plus, per crate: hostile `restore_quetzal`/`restore_file`/
`restore_screen_snapshot` buffers (`zvm`) or `restore_quetzal`/`load_vfs`
buffers (`gvm`) — both pure random noise and mutated-but-once-valid blobs —
asserting `Err`/`Ok` without a panic and that the machine is still steppable
afterward either way; a disassembler sweep at random addresses on random
images; and, `gvm` only, `glk_dispatch` (made `pub(crate)` for this — still
not an embedder-facing API) called with random selectors (mostly the real
0x0001-0x016F range, occasionally a fully random `u32` to cover invalid
selectors too) and a random number of random arguments.

### Running it

```sh
cargo nextest run -p lanthorn-zvm --lib fuzz_harness
cargo nextest run -p lanthorn-gvm --lib fuzz_harness
```

`LANTHORN_FUZZ_SEEDS=N` overrides the default seed count for a longer local
run (the harness reads it once per `#[test]`, so it applies to every case in
the file that walks a seed range):

```sh
LANTHORN_FUZZ_SEEDS=20000 cargo test -p lanthorn-zvm --lib fuzz_harness -- --nocapture
```

Default seed counts were chosen to keep the whole file comfortably under a
few seconds warm (measured: `zvm` ~0.75s, `gvm` ~0.75s, both well inside the
"~10s per crate under nextest" guidance in the top-level `CLAUDE.md`) while
still finding real budget-calibration issues — see "What this quest found"
below for the one class of failure it does surface.

### Step cap and budget: two different jobs, sized two different ways

`MAX_STEPS` and `PER_STORY_BUDGET` look like one knob but answer two
different questions, and conflating them is what went wrong on the first pass
(the "20,000 / 250ms" this heading used to be named after). **`MAX_STEPS` is
the real bound on the work a single image can do** — the harness's actual
depth-of-execution parameter. **`PER_STORY_BUDGET` is a hang guard, not a
performance bound** — it exists only to catch a `step()` call that never
returns at all (the pre-SQ-1395 shape: an unbounded loop inside one opcode
handler), and should otherwise never trip.

The obvious defaults (a step cap in the tens of thousands, a budget in the low
hundreds of milliseconds) turned out to conflate the two, and chasing that
down is worth recording so nobody re-derives it by hand next time. A garbage
image occasionally decodes a `read`/`read_char` (`zvm`) or `glk_select`
(`gvm`) inside what amounts to a loop that keeps re-arming it — the harness's
own random answers never type "quit", so a story that (deliberately or by
accident of its garbage bytecode) just keeps asking will keep being asked, for
the full step cap, on every run. That is **legitimate execution, not a
hang** — but it is also genuinely slower per step than a mostly-`Continue`
run, because each iteration walks the full input-suspend/resume machinery
instead of one decoded instruction. Measured during this quest, both in
unoptimized (`cargo test`, no `--release`) builds on the development machine
(an M2 Max):

| crate | scenario | steps | measured | per-step |
|---|---|---|---|---|
| `zvm` | mostly `Continue` (seed `0x5eed000000000074`, v4) | 20,000 | 1.445s | ~72µs |
| `zvm` | `NeedLine`/`NeedChar` loop, zero `Continue`s (seed `0x5eed000000000234`, v8) | 5,000 | 1.400s | ~280µs |

The read/select-loop case is roughly **4x** slower per step than the
mostly-`Continue` case, so a step cap and budget picked from the fast case's
throughput fails the slow-but-not-buggy case under load on the *same*
machine — which is exactly what happened first (`MAX_STEPS = 20_000`,
`PER_STORY_BUDGET = 250ms` failed 4-and-then-1 seeds out of 1500 on repeated
runs, each one a legitimate full-step-cap read loop, not a panic or a real
hang). Both crates now use `MAX_STEPS = 1_500`, which bounds a full read/select
loop to ~403ms measured locally — that number is what actually stops a
non-buggy image from taking unbounded wall time, by capping the WORK rather
than the clock.

**The budget is deliberately not sized off that ~403ms figure.** A number
tuned against one fast, uncontended development machine is exactly the kind
of thing that becomes a CI flake: GitHub's runners are 3-4 cores, `cargo test`
(not `cargo nextest`) runs a binary's tests as threads sharing those cores
rather than one process per test, and a case that measures 403ms locally can
easily take 2-3x longer there for reasons that have nothing to do with a bug
in the engine. Sizing the hang guard close to the legitimate worst case
practically guarantees an eventual red CI run on nothing more than scheduler
noise. `PER_STORY_BUDGET` is therefore `10s` in both crates — ample headroom
over any plausible CI slowdown of ~403ms, while still catching a real hang,
which would either never return at all or balloon by orders of magnitude, not
merely by a constant multiplier. If you raise `MAX_STEPS` later, re-measure
the read/select-loop case to know what the real per-story ceiling has become,
but there's no need to also move `PER_STORY_BUDGET` in lockstep — it isn't
tracking that number any more.

### What this quest found

**No production panics.** Both harnesses ran clean (`cargo test`/`cargo
nextest`, twice each for determinism, plus a `LANTHORN_FUZZ_SEEDS=8000`
stress pass — 5x the default — with zero failures) after the step-cap/budget
retuning above. The only failures the harness produced during development were
the budget-miscalibration false positives described above, which were a
harness-tuning problem, not an engine defect — no engine code changed as a
result. That is a genuinely useful negative result on its own: it is evidence
(not proof) that the SQ-1395 caps closed the class of bug SQ-1014's throwaway
fuzzer found, for the input shapes this harness generates. See "Turning a
crash into a pinned case" below for what to do the day that stops being true.

## Half 2: `crates/fuzz`

A detached package — its own `[workspace]` table, listed in the root
`Cargo.toml`'s `exclude` (same shape as `crates/gvm/tables-gen`) — so its
dependency on `libfuzzer-sys` never touches `zvm`/`gvm`'s zero-dependency
build. `cargo check`/`cargo clippy` on it alone **do** succeed on the
workspace's pinned stable toolchain (1.98) — `libfuzzer-sys` itself needs no
nightly feature to compile — but it stays detached anyway: `cargo fuzz run`
needs nightly regardless (coverage instrumentation is a nightly-only
`-Cinstrument-coverage`/sanitizer feature), the standard `cargo-fuzz` project
shape is a package like this one, and folding `libfuzzer-sys`'s C++ build into
every `cargo check --workspace`/`cargo test --workspace --all-features` would
tax exactly the routine-build cost the top-level `CLAUDE.md` spends its
opening section minimizing.

Seven targets under `crates/fuzz/fuzz_targets/`, each a thin `fuzz_target!`
that feeds the fuzzer's raw bytes straight in — no synthetic "valid-ish
header" construction needed for the `_step`/`_disasm` targets, since
libFuzzer's coverage guidance explores toward whatever byte patterns reach new
code on its own:

| target | exercises |
|---|---|
| `zvm_step` | `Memory::new` + the `step()` loop, answering every suspend point from the input bytes |
| `zvm_restore` | `restore_quetzal` and `restore_file` against a fixed tiny valid story |
| `zvm_screen_snapshot` | `screen_snapshot::decode` directly (pure decode, no machine) |
| `zvm_disasm` | `disassemble`/`disassemble_raw` at an address derived from the image |
| `gvm_step` | `Memory::new` + the `step()` loop (Glulx) |
| `gvm_restore` | `restore_quetzal` and `load_vfs` against a fixed tiny valid image |
| `gvm_disasm` | `decode_instr` at an address derived from the image |

The `_step`/`_restore` targets duplicate a little of Half 1's logic (a tiny
valid-header builder, the suspend-point answering loop) rather than reaching
into `fuzz_harness`'s private helpers: `crates/fuzz` is a separate crate, and
`fuzz_harness` is `#[cfg(test)]`-only inside `zvm`/`gvm`, so nothing in it is
reachable from here at all. Duplicating ~20-30 lines per target was preferred
over widening either crate's public API just for this — see the top-level
`CLAUDE.md`'s stance on hand-maintained invariants across files, which this
avoids by keeping each duplicate small, self-contained, and commented as a
duplicate rather than pretending to be the source of truth. The one exception
worth naming: `gvm`'s `Machine::glk_dispatch` was widened from private to
`pub(crate)` so Half 1's `fuzz_harness` (a sibling module, same crate) could
call it directly — but Half 2 has no Glk-dispatch target, so `crates/fuzz`
never needed that door at all.

### Running it

Requires the `cargo-fuzz` subcommand (`cargo install cargo-fuzz`) and a
nightly toolchain (`rustup toolchain install nightly`). From
`crates/fuzz/`:

```sh
cd crates/fuzz
cargo +nightly fuzz run zvm_step              # Ctrl-C to stop; runs until then
cargo +nightly fuzz run zvm_step -- -max_total_time=60   # or bound it
cargo +nightly fuzz run zvm_restore
cargo +nightly fuzz run zvm_screen_snapshot
cargo +nightly fuzz run zvm_disasm
cargo +nightly fuzz run gvm_step
cargo +nightly fuzz run gvm_restore
cargo +nightly fuzz run gvm_disasm
```

A crash writes a reproducer under `crates/fuzz/artifacts/<target>/`; both that
directory and `crates/fuzz/corpus/` are gitignored (`crates/fuzz/.gitignore`,
alongside `Cargo.lock` and `/target` — the same pattern `crates/gvm/tables-gen`
uses for a detached, hand-run package).

`cargo-fuzz` was **not installed** in this environment when SQ-1407 was done,
and per the quest's own instruction it was not installed for this pass —
`cargo check`/`cargo clippy` on the package were run instead (both clean) and
no actual fuzzing run was performed. The targets are believed correct against
the crates' current public API (they compile clean on stable) but have not
yet found — or failed to find — anything themselves.

### Turning a crash into a pinned case

When `cargo fuzz run` finds one:

1. It prints the failing input's path under `artifacts/<target>/`. Read it
   with `xxd` or copy it into a scratch file.
2. Reproduce the panic in isolation: `cargo +nightly fuzz run <target>
   artifacts/<target>/<hash>` re-runs just that one input.
3. Fix the bug at its cause in the engine crate — a bounds check, a cap, an
   early return, following the style of the SQ-1395 caps this whole effort
   traces back to. Keep the fix surgical; do not refactor the surrounding code.
4. Pin it as a named `#[test]` in the relevant `fuzz_harness.rs`, embedding the
   exact failing bytes (`include_bytes!` from a copy saved under the crate's
   own `tests/fixtures/`, or a literal `&[u8]` if it's small) so it runs under
   `cargo nextest`/CI forever after — the same role `object_scan_eof.rs` plays
   for `czech.z5`.
5. If the fix isn't surgical — it points at a real design problem rather than
   a one-line guard — pin the case as `#[ignore]` with a comment explaining
   why, and file a quest for the design work rather than leaving it unpinned.

No crashes have been found yet (the tool wasn't run), so there is no bugs list
here — this section is the recipe for when there is one.
