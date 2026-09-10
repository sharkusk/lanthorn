# zvm benchmark fixtures (SQ-1428)

What `cargo run --release -p lanthorn-zvm --example bench` drives. The recorded
baselines, the machine they were taken on and the matching `dfrotz` commands are
in [`docs/internals/performance.md`](../../../../../docs/internals/performance.md).

## Story

**`../minizork.z3`** — Mini-Zork I, Release 34 / serial 871124, 52,216 bytes,
sha256 `c74f01a2…2c69e31ea4a6`. Not duplicated here: it is the same fetched
fixture (SQ-1453) the save-interop and story-location suites use one
directory up, reached the same way — `zvm::fixtures::load("minizork.z3")` —
and its provenance is tabulated in [`../README.md`](../README.md).

A real v3 game rather than an opcode exerciser, on purpose. What an embedder
wants to know is how long a *turn of a game* takes — dictionary lookup, parse,
object-tree walk, text decode, status line — not how long a tight loop of
`@add`s takes.

## `minizork.script`

Eight commands, one per line. Blank lines and `#` comments are skipped by the
harness; strip them with `grep -v '^#' | grep -v '^$'` before feeding the same
script to `dfrotz`, and the two runs are turn-for-turn identical.

The four movement commands are the above-ground house circuit — West of House →
North of House → Behind House → South of House → West of House — so the cycle
is closed and can be looped for any number of turns. The other four are
parser-only turns that are idempotent on repetition (`open mailbox` answers "It
is already open." from lap two, `take leaflet` answers "You already have that.").

Verified to 20,000 turns through `dfrotz` before it was committed: the game ends
in West of House with score 0 and Moves 20000, having never reached the Thief, a
grue, the lamp battery or any random event. `script_is_a_closed_cycle` in
`crates/zvm/examples/bench.rs` re-checks the cycle property on every CI run.

## Why no Curses or TerpEtude script

`curses.z5` is committed here and would make a fine second data point, but the
game has no short closed circuit near its start and a long walkthrough is a
fixture that rots the moment anybody edits it. `etude.z5` and `czech.z5` are
menu-driven exercisers whose "turns" are self-tests, which is the shape
glulxercise already covers on the Glulx side.
