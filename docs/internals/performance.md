# Engine performance

Three timing harnesses, one per VM crate, and the numbers they produced against
the reference interpreter for each format. Their whole purpose is that an
embedder choosing an engine — or anyone doubting a claim on this page — can
rerun them and get their own numbers on their own machine, rather than taking a
ratio somebody measured once on hardware they do not own.

Every harness is a plain `--example` in the crate it measures, because `zvm`,
`gvm` and `scott` take **zero external dependencies** and a benchmark crate
would be one. They use `std::time::Instant` and nothing else.

```sh
cargo run --release -p lanthorn-zvm   --example bench -- <story> <script> [--turns N] [--repeat N]
cargo run --release -p lanthorn-gvm   --example bench -- <story> <script> [--turns N] [--repeat N] [--no-accel]
cargo run --release -p lanthorn-scott --example bench -- <story> <script> [--turns N] [--repeat N]
```

Each loads the story, boots it, feeds the script's lines through the engine's
step loop with output discarded, and prints wall clock, turns, µs/turn and
turns/s — plus, for `gvm`, opcodes and opcodes/s, because
`gvm::Machine::insn_count` already exists. `zvm` and `scott` have no equivalent
counter and none was added for this: a counter in the hot loop is a change to
the thing being measured.

## Read this before quoting a number

- **`--release`, always.** A debug build measures the un-inlined, bounds-checked
  shape of the interpreter, not the one anybody ships. Debug numbers here run
  three to twenty times slower and mean nothing; the harnesses say so in their
  own module docs.
- **Best of three.** `--repeat` defaults to 3 and the FASTEST run is reported.
  The best of several is the run least polluted by whatever else the machine was
  doing; a mean folds that noise into the answer.
- **The script loops.** Every committed script is a closed cycle that leaves its
  game exactly where it found it, so `--turns` can be any size. Each crate's
  example carries a `#[test]` asserting that property — see "What holds the
  scripts honest" below.
- **A turn is not a fixed unit of work.** 20,000 minizork turns and 2,700
  glulxercise turns are wildly different amounts of computation. Compare an
  engine against ITS reference on the SAME script; never compare µs/turn across
  the three tables.

## Machine and build

| | |
|---|---|
| CPU | Apple M2 Max (12 logical cores: 8 performance + 4 efficiency) |
| OS | macOS 26.6.2 (build 25G83) |
| Rust | rustc 1.98.0 (`88d9e12ae`, 2026-08-18), `--release` |
| Date | 2026-09-08 |
| Quest | SQ-1428 (harness), SQ-1438 and SQ-1431 (the zvm work below) |

The machine was otherwise idle. All three references were built from source
with `cc -O2` — see each one's recipe below — because an unoptimised reference
flatters us for a reason that has nothing to do with either interpreter.

## Baselines

Reference wall clocks are best-of-3 of the whole process, stdin from a file,
stdout to `/dev/null`. Ours are best-of-3 of load + boot + play inside the
process. Process spawn is a couple of milliseconds against runs of hundreds,
so the two spans are comparable; the reference additionally writes its output
to `/dev/null` where ours only counts the bytes, which is worth well under a
millisecond per megabyte and is the residual asymmetry in every row.

| engine | story | turns | lanthorn | reference | reference is | ratio |
|---|---|---|---|---|---|---|
| `zvm` | `minizork.z3` | 20,000 | **0.587 s** (29.4 µs/turn, 34,051 turns/s) | 0.92 s | dfrotz (Frotz 2.55) | **1.57× faster** |
| `gvm` | `glulxercise.ulx` | 2,700 | **0.628 s** (233 µs/turn, 26.9 M opcodes/s) | 1.13 s | glulxe 0.6.1 + cheapglk 1.0.7 | **1.80× faster** |
| `gvm` `--no-accel` | `glulxercise.ulx` | 2,700 | **0.972 s** (360 µs/turn, 28.5 M opcodes/s) | 1.13 s | glulxe 0.6.1 + cheapglk 1.0.7 | **1.16× faster** |
| `scott` | `tiny_cave.dat` | 100,000 | **0.152 s** (1.52 µs/turn, 658,409 turns/s) | 0.15 s | ScottFree 1.14 (cspiegel/scottfree-glk) + cheapglk | **parity** |

Output volume, as a cross-check that both sides did the same work: zvm printed
1,028,281 bytes, gvm 2,045,646 bytes, scott 13,590,154 bytes. Those three counts
have not moved across any change on this page, which is the cheapest evidence a
performance edit left semantics alone.

**All eight numbers above — ours and all four references — were re-measured in
one sitting** (2026-09-08, SQ-1431). That matters more than their absolute
values: a ratio built from our number today and a reference number from a
quieter afternoon last week is not a ratio. This machine drifts 2-4% between
runs of the same binary, which is why every row is a best-of-five and why the
per-change deltas in the profiles below are quoted against a baseline measured
the same way, minutes apart.

### What the rows say

**`zvm` was the one that was behind**, by a factor of about 2.4 on a real v3
game, until SQ-1438. A sampling profile of the 400,000-turn run (macOS
`sample`, 6,113 samples) found where it went: roughly **47% of samples were in
the allocator** (`malloc`/`free`/`realloc` and `RawVec` growth) and another
**~15% in `core::fmt`**, leaving the actual opcode dispatch
(`cpu::decode::decode`, `Machine::step`, `state::read_var`) a minority of the
time. Two call sites accounted for most of it, both on the per-instruction
path:

- `exec.rs`'s `opcode_name()` was called for **every instruction** and
  returned a `String`. For the eight opcodes it knows it is a `to_string()`;
  for every other opcode it is `format!("op:{:?}/0x{:02x}", …)` — a `Debug`
  format and a `LowerHex` format, allocated and thrown away — and it is
  consumed only when the instruction faults, which is approximately never.
- `decode()` built a `Vec<Operand>` per instruction and `execute()` collected
  another from it (`spec_from_iter_nested` on `Vec<Operand>`, 316 samples).

**SQ-1438 fixed both, without touching semantics or the public `Instr`/
`Operand` shape** (`zvm::cpu::decode` is reachable by an embedder, so the
fix had to keep those types as they were): `opcode_name()` is now called
only on the fault path that actually consumes its `String`, using the two
`Copy`/cheap-`Clone` fields (`opcode`, `operand_count`) saved off the
instruction before it is moved into `execute()`. And `decode()` grew a
`decode_into()` sibling that fills a caller-supplied `Vec<Operand>` instead
of allocating one — `Machine` now owns that buffer (and a second one for
`execute()`'s resolved `Vec<u16>`) and lends each out before use and takes it
back after, so the same heap allocation is reused for the rest of the run
instead of being `malloc`'d and `free`'d every instruction. `decode()` itself
is unchanged (`decode_into(mem, pc, version, Vec::new())`), so every existing
caller — `disasm.rs`, `disasm_cache.rs`, the crate's own tests — is unaffected.
The result: 20,000 Mini-Zork turns in **0.599 s**, down from 2.258 s (~3.7×),
which flipped the ratio against dfrotz from 2.43× slower to faster.

**SQ-1431 then went looking for what was left**, profiled both engines, and
found the honest answer to be *not much*: three small changes worth 1.8%
together, and every larger candidate either already done, measured too small to
pay for itself, or a redesign. The row above is where that leaves zvm —
**0.587 s, 1.57× faster than dfrotz** — and the [Profiles](#profiles) section
below is the record of what was measured, kept and rejected, so the next person
to wonder about the dispatch loop can start from numbers instead of hunches.

**`gvm` is ahead of glulxe, and the margin needs an asterisk.** With
acceleration on we are 1.80× faster, but that is not a like-for-like dispatch
comparison: gvm recognises Inform veneer functions **by fingerprint**
(`gvm::veneer`) and intercepts them whether or not the story registered them
with `@accelfunc`, where glulxe accelerates only what the story asks it to. The
opcode counts show the difference plainly — 16.9 M dispatched with acceleration
on against 27.7 M with it off, for identical output. `--no-accel` is the honest
dispatch-loop row: **1.16× faster**, which is the number to quote when the
question is "how fast is the interpreter loop". The quest that prompted this
page recorded gvm as 1.2–1.5× *behind* glulxe on the dispatch loop; that was an
ad-hoc measurement and this one does not reproduce it, which is exactly why a
rerunnable harness exists.

**`scott` is at parity**, and at 1.5 µs a turn there is nothing to chase. A
Scott Adams turn is a linear scan of a few hundred action entries over a few
kilobytes of state; both implementations are memory-bandwidth-bound on data that
fits in L1, and the difference between the two is smaller than the 10 ms
resolution `/usr/bin/time -p` reports the reference in.

## Profiles

Where the time actually goes, by `sample`(1) — the tool is on every macOS box,
needs no build flags and no dependency in the crate being measured, which is the
same constraint that made the harnesses plain `--example`s. Each profile is one
1 ms-interval sample of a long run of the harness above, and the percentages are
of that run's total thread samples.

Read these as *self* time: a frame's number is the samples caught with that
function at the top of the stack. Release builds inline aggressively, so a big
number often covers several source functions — `Machine::step` below has
`execute`, four of the five `exec_*` arms, `resolve`, `do_branch` and `do_store`
folded into it, and reads as the whole interpretation half of the loop.

Rerunning one is the harness command with a bigger `--turns`, backgrounded, and
`sample` attached to it — twelve seconds is plenty for a stable ranking:

```sh
cargo build --release -p lanthorn-zvm --example bench
./target.noindex/release/examples/bench-<hash> \
    crates/zvm/tests/fixtures/minizork.z3 \
    crates/zvm/tests/fixtures/bench/minizork.script --turns 400000 --repeat 1 &
sample $! 12 1 -f /tmp/zvm.sample
```

Run the binary directly rather than through `cargo run`, or `sample` attaches to
cargo and reports a process that spends its life waiting; two examples in this
workspace are both named `bench`, so take the hash-suffixed path cargo wrote and
check which engine it is by feeding it a story. The ranking lives under "Sort by
top of stack" at the end of the report; the call graph above it is what says
*whose* allocator samples those are.

### zvm

400,000 Mini-Zork turns (12.3 s, 9,179 samples), 2026-09-08, on the tree as
SQ-1438 left it:

| frame | samples | share |
|---|---|---|
| `cpu::exec::Machine::step` (with `execute`, `exec_2op`/`exec_1op`/`exec_0op`, `resolve`, `do_branch`, `do_store` inlined in) | 3,382 | 36.8% |
| `cpu::decode::decode_into` | 2,414 | 26.3% |
| `cpu::state::read_var` | 785 | 8.6% |
| `cpu::decode::read_operand` | 493 | 5.4% |
| `cpu::state::write_var` | 470 | 5.1% |
| `cpu::exec::Machine::exec_var` | 326 | 3.6% |
| `cpu::decode::read_operands_from_type_byte` | 287 | 3.1% |
| the allocator, all callers (`malloc`/`free`/`realloc`) | 274 | 3.0% |
| `bench::main` — the harness's own output accounting | 224 | 2.4% |
| `text::decode::decode_into::<String>` — `print`/`print_ret` inline text | 85 | 0.9% |

**The allocator is finished as a target.** It was 47% before SQ-1438 and is 3%
here, spread across `call_routine`'s per-frame locals `Vec`, the inline-text
`String`, and the output sink — no single call site worth more than one point.
So "allocation-free event pushes", one of the candidates this quest was opened to
test, has a ceiling of three points and nothing left to aim at.

What is left is the dispatch loop itself: **63% of the run is `step` plus
`decode_into`**, and both are doing real work rather than overhead. Decode reads
one to six bytes, walks the form and signature tables and fills an `Instr`;
`step` resolves the operands and runs the opcode. The whole thing costs roughly
45 cycles per Z-machine instruction, against dfrotz's ~68.

Three changes came out of this profile (SQ-1431):

| change | why the profile pointed at it |
|---|---|
| `execute()` resolves operands into a stack `[u16; MAX_OPERANDS]` instead of a reused `Vec<u16>` field | the resolved list is bounded by the encoding at 8, so it never needed to live behind a pointer; drops a `Machine` field with it |
| `#[inline]` on `read_operand` and `read_operands_from_type_byte` | `read_operand` had its own 5.4% self-time frame, which is what a function being *called* two to eight times per instruction looks like. After, it is gone, folded into its caller |
| `print_num` formats into a stack buffer instead of `format!("{}", val)` | the last per-opcode `String` SQ-1438 left behind. Mini-Zork's script barely prints numbers, so the harness cannot see this one — it is here because it is an allocation on a path every game hits, not because it moved the row |

Together, 20,000 turns: **0.598 s → 0.587 s, 1.8% faster**, output byte-identical.

**1.8% is the honest total, and it is below the bar this quest set itself
(>5%).** The three were kept because each removes an allocation or an
indirection and none adds a line the profile does not name — not because they
cleared it. The lane's real finding is the section below.

#### Measured and rejected

- **Operand-decode caching, i.e. an inline-operand `Instr`.** `Instr.operands` is
  a `Vec<Operand>`, and the obvious next move is a fixed `[Operand; 8]` inside
  the struct, so decode touches no heap at all. This was **built and measured**
  rather than argued about: a throwaway `OperandList` with a `Deref<Target =
  [Operand]>` — which every in-crate caller, and any embedder that only reads the
  list, survives unchanged — took 20,000 turns from 0.587 s to **0.571 s, 2.6%**.
  That is the measured ceiling for the whole idea, and it costs a change to the
  public `Instr` shape `zvm::cpu::decode` promises embedders. **Not worth it**,
  and written down here so nobody prototypes it a second time.
- **`#[inline]` on `decode_into` itself**, to let LLVM scalarize the ~80-byte
  `Instr` across the return: 0.593 s → 0.586 s, inside this machine's run-to-run
  drift, and it bloats every `disasm` caller to buy it. Dropped.
- **A batched run-until-stop loop** replacing the per-instruction `StepResult`
  return. The profile gives it no support: `step`'s prologue (the pending
  read/save/restore guards) and its epilogue (two fault latches, the v6
  empty-frame check) are a handful of predictable branches that do not surface as
  frames at all, and `StepResult` comes back in registers. There is nothing
  measurable to win, and it would put a second execution path beside the one the
  conformance corpus covers.
- **Validated-then-unchecked memory reads.** `Memory::read_byte`/`read_word`
  already compile to a bounds check and a load — `get()` with a cold fault-latch
  arm — and do not appear as frames because they inline into their callers. There
  is no loop to hoist a check out of, either: one decode reads one to six bytes
  at unrelated addresses.

Two things left could plausibly matter, and neither is filed as work because
neither has a measured case yet:

- **Locals in a flat stack** rather than a `Vec<u16>` per `Frame`. It would turn
  `read_var`'s `frames.last()` plus bounds check (8.6%, and `write_var`'s 5.1%
  beside it) into a direct index, and delete a `malloc`/`free` pair per routine
  call. It changes `Frame`, which Quetzal serializes.
- **Fused decode-and-dispatch** — no `Instr` value at all, operands read straight
  into stack slots by the opcode's own arm, a known dispatch-loop technique
  (dfrotz and glulxe both use some form of it, though neither's source was
  read to arrive at it here). That is the only change with real headroom left;
  the 63% above is its target, per SQ-1441's own inline-operand measurement below.
  It is also a second interpreter to keep correct beside the one the corpus
  covers, and the inline-operand measurement says the *struct* is only 2.6
  points of that 63 — so most of it is work no change of shape avoids.

### gvm

30,000 glulxercise turns, 2026-09-08. Both modes, because the accelerated run is
not a dispatch-loop measurement — see the row note above.

**Accelerated** (8.4 s, 6,270 samples):

| frame | samples | share |
|---|---|---|
| `exec::Machine::read_operands` | 1,306 | 20.8% |
| `exec::Machine::resolve_load` | 769 | 12.3% |
| `exec::Machine::local_load` | 453 | 7.2% |
| `exec::Machine::step` | 444 | 7.1% |
| `exec::Machine::step_once` | 323 | 5.2% |
| `exec::Machine::execute` | 318 | 5.1% |
| `exec::Machine::decode_compressed` | 285 | 4.5% |
| the allocator, all callers | 283 | 4.5% |
| `exec::Machine::local_store` | 265 | 4.2% |
| `exec::Machine::build_frame_and_enter` | 232 | 3.7% |
| `exec::Machine::resolve_store` | 182 | 2.9% |
| `exec::Machine::reload_frame_meta` | 169 | 2.7% |
| SipHash, under `accel::obj_in_class`'s map lookups | 119 | 1.9% |

**`--no-accel`** (10.9 s, 8,309 samples) — the same shape, weighted harder toward
the loop because the veneer work acceleration would have intercepted is now
dispatched opcode by opcode:

| frame | samples | share |
|---|---|---|
| `exec::Machine::read_operands` | 1,951 | 23.5% |
| `exec::Machine::resolve_load` | 1,200 | 14.4% |
| `exec::Machine::step` | 667 | 8.0% |
| `exec::Machine::local_load` | 563 | 6.8% |
| `exec::Machine::execute` | 511 | 6.1% |
| `exec::Machine::step_once` | 420 | 5.1% |
| `exec::Machine::build_frame_and_enter` | 306 | 3.7% |
| `exec::Machine::local_store` | 290 | 3.5% |
| `exec::Machine::resolve_store` | 273 | 3.3% |
| `exec::Machine::decode_compressed` | 261 | 3.1% |
| `exec::Machine::reload_frame_meta` | 230 | 2.8% |

(`decode_compressed` in both tables is now spelled `stream_string` — SQ-1418
rebuilt the printing engine to run on the Glulx stack. The frame names above are
left as they were sampled, since a profile is a record of a run.)

**Nothing was changed in `gvm`, and the profile is why.** Operand decoding is a
third of the run in both modes, and **SQ-1208 already took the allocator out of
it**: `read_operands` fills two fixed-capacity `Operands<T>` values on the stack,
`MAX_OPERANDS` is derived from the ISA's widest opcode rather than guessed, and
the 4.5% of allocator samples above belong to Glk output and the save stack, not
to dispatch.

What is left is the *shape* of `read_operands`: it returns
`(Operands<u32>, Operands<Dest>)` — about 112 bytes — by value through a
`Result<_, String>`, on every instruction, and each of the ~150 call sites names
its load and store counts as literals the compiler cannot exploit because the
function is far too large to inline into all of them. Fixing that means
out-parameters or per-shape monomorphization across every one of those sites:
not local, not surgical, and this lane found no way to estimate the win short of
doing it. Recorded as an observation rather than filed as work, because
`--no-accel` — the honest dispatch row — is already 1.16× ahead of glulxe.

One cheap thing was noticed and left alone: `accel::obj_in_class` hashes through
`std`'s SipHash `RandomState` on every accelerated property lookup, 1.9% of the
accelerated run. A cheaper hasher would recover most of it, but it is well below
the bar and it is on the accelerated path only.

`Memory::checksum_ok` shows at ~0.9% in both modes and is **not** a defect:
glulxercise's own test script calls `@verify`, which is specified to checksum the
whole image.

### scott

Not profiled. At 1.5 µs a turn against a reference inside measurement
resolution, there is no gap to explain — see the row note above.


## Rerunning it

### zvm vs dfrotz

```sh
# lanthorn
cargo run --release -p lanthorn-zvm --example bench -- \
    crates/zvm/tests/fixtures/minizork.z3 \
    crates/zvm/tests/fixtures/bench/minizork.script --turns 20000

# dfrotz — brew install frotz (measured: FROTZ V2.55, commit acf20558, 2025-02-01)
grep -v '^#' crates/zvm/tests/fixtures/bench/minizork.script | grep -v '^$' > /tmp/mz.txt
python3 - <<'PY'
open('/tmp/mz20k.txt','w').write(open('/tmp/mz.txt').read() * 2500)
PY
for i in 1 2 3; do
  /usr/bin/time -p dfrotz -m -w 200 crates/zvm/tests/fixtures/minizork.z3 \
      < /tmp/mz20k.txt > /dev/null
done   # take the smallest "real"
```

`-m` disables dfrotz's `***MORE***` pager, which would otherwise eat lines out
of the piped script and desync it; `-w 200` fixes the wrap width so the amount
of text formatting does not depend on the terminal.

### gvm vs glulxe

Homebrew's `glulxe` links `libncurses` and cannot be scripted from stdin
(`otool -L` confirms it) — it must be built against **cheapglk**, exactly as
`crates/gvm/tests/fixtures/README.md` already describes for the conformance
oracle:

```sh
git clone https://github.com/erkyrath/cheapglk && (cd cheapglk && make OPTIONS="-O2 -Wall -Wno-unused")
git clone https://github.com/erkyrath/glulxe  && (cd glulxe && \
    make OPTIONS="-O2 -Wall -Wno-unused -DOS_MAC" \
         GLKINCLUDEDIR=../cheapglk GLKLIBDIR=../cheapglk GLKMAKEFILE=Make.cheapglk)

# lanthorn
cargo run --release -p lanthorn-gvm --example bench -- \
    crates/gvm-cli/tests/fixtures/glulxercise.ulx \
    crates/gvm/tests/fixtures/bench/glulxercise.script --turns 2700
# …and again with --no-accel for the dispatch-loop row

# glulxe
grep -v '^#' crates/gvm/tests/fixtures/bench/glulxercise.script | grep -v '^$' > /tmp/gx.txt
python3 - <<'PY'
open('/tmp/gx100.txt','w').write(open('/tmp/gx.txt').read() * 100)
PY
for i in 1 2 3; do
  /usr/bin/time -p ./glulxe/glulxe -q -u \
      crates/gvm-cli/tests/fixtures/glulxercise.ulx < /tmp/gx100.txt > /dev/null
done
```

`-q -u` suppresses cheapglk's banner and asks for UTF-8 I/O, matching what the
conformance corpus already uses.

### scott vs ScottFree

**ScottFree is not packaged by Homebrew** (`brew search scott` finds nothing).
The IF Archive's `ScottFree.tar.gz` is the curses build, which is no easier to
script than ncurses glulxe; the measurement above used **cspiegel/scottfree-glk**,
a maintained C port of Alan Cox's ScottFree 1.14 that links any Glk library —
so it links the same cheapglk built above and scripts from stdin:

```sh
git clone https://github.com/cspiegel/scottfree-glk
ln -s "$PWD/cheapglk" scottfree-glk/cheapglk
(cd scottfree-glk && make GLK=cheapglk CC=cc OPT=-O2)   # produces ./scott

# lanthorn
cargo run --release -p lanthorn-scott --example bench -- \
    crates/scott/tests/tiny_cave.dat \
    crates/scott/tests/fixtures/bench/tiny_cave.script --turns 100000

# ScottFree
grep -v '^#' crates/scott/tests/fixtures/bench/tiny_cave.script | grep -v '^$' > /tmp/sc.txt
python3 - <<'PY'
open('/tmp/sc100k.txt','w').write(open('/tmp/sc.txt').read() * 10000)
PY
for i in 1 2 3; do
  /usr/bin/time -p ./scottfree-glk/scott crates/scott/tests/tiny_cave.dat \
      < /tmp/sc100k.txt > /dev/null
done
```

## What holds the scripts honest

A benchmark script that quietly stops doing what it claims is worse than no
benchmark: the numbers stay plausible and the workload changes underneath them.
Each example therefore carries `#[cfg(test)]` cases that CI compiles and runs
(the `[[example]]` entries set `test = true`), and each asserts the property its
script depends on — never a duration, which would be a flake generator:

| crate | case | what it holds |
|---|---|---|
| `zvm` | `bench_harness_drives_a_short_script` | the harness plays the turns asked for and produces output |
| `zvm` | `script_is_a_closed_cycle` | two laps print nearly twice one lap's bytes — the loop returns to where it started |
| `gvm` | `bench_harness_drives_a_short_script` | turns played, opcodes dispatched, output produced |
| `gvm` | `script_runs_only_passing_tests` | one `Passed.` per script line and no failures — a typo'd test name would otherwise measure glulxercise's help text |
| `scott` | `bench_harness_drives_a_short_script` | turns played and output produced |
| `scott` | `script_never_ends_the_game` | ten laps' turns are all played — `tiny_cave` is won by dropping the idol, and a benchmark that wins on lap one measures a quit loop |

All six skip vacuously (with an `eprintln!`) if their fixture is missing.

## Fixtures

All three stories and all three scripts are committed and freely
redistributable; provenance, checksums and the reasoning behind each script's
contents live beside them:

- `crates/zvm/tests/fixtures/bench/README.md` — `minizork.z3` (already committed
  for the save-interop suite) and the eight-command house circuit
- `crates/gvm/tests/fixtures/bench/README.md` — the twenty-seven-group dispatch
  script, including which groups are deliberately excluded and why. The story is
  the workspace's single copy of `glulxercise.ulx`, under
  `crates/gvm-cli/tests/fixtures/`, which `gvm`'s conformance suites already read
  by that same relative path
- `crates/scott/tests/fixtures/bench/README.md` — `tiny_cave.dat` (this repo's
  own fixture) and the ten-command circuit
