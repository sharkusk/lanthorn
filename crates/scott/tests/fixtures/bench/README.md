# scott benchmark fixtures (SQ-1428)

What `cargo run --release -p lanthorn-scott --example bench` drives. The
recorded baselines, the machine they were taken on and the ScottFree build
recipe are in
[`docs/internals/performance.md`](../../../../../docs/internals/performance.md).

## Story

**`../tiny_cave.dat`** — this repository's own three-room ScottFree-format
fixture, 1,755 bytes, sha256
`f3cae21353ab4faf00c3db6ec24cdd8fcce9657a88b87535ba37b5e5b2f4f0f4`. Written
here, committed here, redistributable by definition; already driven by
`crates/scott/tests/golden.rs`.

Not duplicated into this directory — one copy, in the place the other suites
already read it from.

A real Scott Adams game (Adventureland, Pirate Adventure, …) would be a larger
action table, but the shape of the work is identical: every turn scans the
action table twice, once for the player's verb/noun and once for the occurrence
pass. `tiny_cave` measures that loop at a size both interpreters parse in
microseconds, which keeps the comparison about the loop rather than about
`.dat` parsing.

## `tiny_cave.script`

Ten commands, one per line. Blank lines and `#` comments are skipped by the
harness; strip them with `grep -v '^#' | grep -v '^$'` before feeding the same
script to ScottFree, and the two runs are turn-for-turn identical.

The map is three rooms in a line — clearing (1) down to cave (2) down to grotto
(3) — so `down`/`down`/`up`/`up` is a closed circuit. The other six commands
exercise the verb/noun matcher, the occurrence pass and the action table while
changing nothing that persists: `get lamp` answers "You already have that." from
lap two, `rub lamp` re-reveals an already-revealed idol, and `push button` /
`pull lever` / `score` / `count` only print.

`drop idol` is deliberately absent — dropping the idol in the clearing is how
`tiny_cave` is WON, and a benchmark that wins on lap one measures a quit loop
from then on. `script_never_ends_the_game` in `crates/scott/examples/bench.rs`
asserts that ten laps' worth of turns are all played, which is what holds that.
Verified to 100,000 turns through ScottFree 1.14 before it was committed: 100,001
prompts, no win, no death, ending back in the clearing.
