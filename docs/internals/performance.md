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
| Quest | SQ-1428 |

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
| `zvm` | `minizork.z3` | 20,000 | **2.258 s** (113 µs/turn, 8,856 turns/s) | 0.930 s | dfrotz (Frotz 2.55) | **2.43× slower** |
| `gvm` | `glulxercise.ulx` | 2,700 | **0.633 s** (234 µs/turn, 26.7 M opcodes/s) | 1.141 s | glulxe 0.6.1 + cheapglk 1.0.7 | **1.80× faster** |
| `gvm` `--no-accel` | `glulxercise.ulx` | 2,700 | **0.974 s** (361 µs/turn, 28.4 M opcodes/s) | 1.141 s | glulxe 0.6.1 + cheapglk 1.0.7 | **1.17× faster** |
| `scott` | `tiny_cave.dat` | 100,000 | **0.153 s** (1.53 µs/turn, 651,838 turns/s) | 0.157 s | ScottFree 1.14 (cspiegel/scottfree-glk) + cheapglk | **1.02× faster** (parity) |

Output volume, as a cross-check that both sides did the same work: zvm printed
1,028,281 bytes, gvm 2,045,646 bytes, scott 13,590,154 bytes.

### What the rows say

**`zvm` is the one that is behind**, by a factor of about 2.4 on a real v3 game.
That is a genuine gap, not a measurement artifact — the script is eight ordinary
commands and dfrotz is doing strictly more I/O work than we are. A sampling
profile of the 400,000-turn run (macOS `sample`, 6,113 samples) says where it
goes: roughly **47% of samples are in the allocator** (`malloc`/`free`/`realloc`
and `RawVec` growth) and another **~15% in `core::fmt`**, leaving the actual
opcode dispatch (`cpu::decode::decode`, `Machine::step`, `state::read_var`) a
minority of the time. Two call sites account for most of it, and both are on the
per-instruction path:

- `exec.rs`'s `opcode_name()` is called for **every instruction** and returns a
  `String`. For the eight opcodes it knows it is a `to_string()`; for every
  other opcode it is `format!("op:{:?}/0x{:02x}", …)` — a `Debug` format and a
  `LowerHex` format, allocated and thrown away — and it is consumed only when
  the instruction faults, which is approximately never.
- `decode()` builds a `Vec<Operand>` per instruction and `execute()` collects
  another from it (`spec_from_iter_nested` on `Vec<Operand>`, 316 samples).

Both are fixable without touching semantics and neither is fixed here — this
lane is measurement only, and the numbers above are the "before" they would be
judged against. **SQ-1438** carries the work.

**`gvm` is ahead of glulxe, and the margin needs an asterisk.** With
acceleration on we are 1.80× faster, but that is not a like-for-like dispatch
comparison: gvm recognises Inform veneer functions **by fingerprint**
(`gvm::veneer`) and intercepts them whether or not the story registered them
with `@accelfunc`, where glulxe accelerates only what the story asks it to. The
opcode counts show the difference plainly — 16.9 M dispatched with acceleration
on against 27.7 M with it off, for identical output. `--no-accel` is the honest
dispatch-loop row: **1.17× faster**, which is the number to quote when the
question is "how fast is the interpreter loop". The quest that prompted this
page recorded gvm as 1.2–1.5× *behind* glulxe on the dispatch loop; that was an
ad-hoc measurement and this one does not reproduce it, which is exactly why a
rerunnable harness exists.

**`scott` is at parity**, and at 1.5 µs a turn there is nothing to chase. A
Scott Adams turn is a linear scan of a few hundred action entries over a few
kilobytes of state; both implementations are memory-bandwidth-bound on data that
fits in L1, and the 3% gap is inside the noise of a 0.15 s run.

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
