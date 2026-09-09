# gvm benchmark fixtures (SQ-1428)

What `cargo run --release -p lanthorn-gvm --example bench` drives. The recorded
baselines, the machine they were taken on and the matching `glulxe` commands are
in [`docs/internals/performance.md`](../../../../../docs/internals/performance.md).

## `glulxercise.ulx`

Andrew Plotkin's Glulx interpreter unit test — Release 13 / serial 241202 /
Inform v6.43, 231,680 bytes, sha256
`b732127fee4cb266a5330981c1111fdfaba237134525754e063e6dc5f449b348`.

**Source:** <https://eblong.com/zarf/glulx/> (the glk-dev `unittests/`
manifest), the same origin as the eleven conformance stories
[`../README.md`](../README.md) already documents — freely redistributable on
Plotkin's own terms.

**Why it is committed here rather than fetched.** A benchmark is worth having
only if the number can be reproduced, and a fixture that has to be downloaded
first is one an embedder will not bother with. The identical file also lives in
the repo-root `unit_tests/` for the conformance suites, where `.ulx` is
gitignored — that directory is gitignored to keep *commercial* v6 stories out
(see `.gitignore`'s own note), not because these stories cannot be
redistributed. `crates/gvm/tests/fixtures/` already commits
`startsavetest.gblorb` on exactly this reasoning.

**Why glulxercise and not a real game.** Adventure/`advent.ulx` would be more
game-shaped, but its Glulx builds carry no licence anybody can point to, and
what the Glulx side of this comparison is actually asking about is the
**dispatch loop** — the quest that prompted this page recorded gvm as 1.2–1.5×
behind glulxe there. glulxercise's groups are dense straight-line opcode work
with almost no Glk in them, which is the cleanest available answer to that
question, and every group self-checks so a benchmark that silently stopped
computing would fail rather than get faster.

## `glulxercise.script`

Twenty-seven test-group names, one per line. Blank lines and `#` comments are
skipped by the harness; strip them with `grep -v '^#' | grep -v '^$'` before
feeding the same script to `glulxe -q -u`, and the two runs are turn-for-turn
identical.

Every group reports `Passed.` and leaves the machine as it found it, so the
list loops indefinitely. `script_runs_only_passing_tests` in
`crates/gvm/examples/bench.rs` asserts exactly one `Passed.` per line and no
failure text — which is what stops a mistyped group name from quietly measuring
glulxercise's help text instead of an opcode group.

The script file itself carries the list of groups deliberately EXCLUDED and the
reason for each (PRNG output that differs between implementations; undo/restore
groups that measure `memcpy` and the allocator; `memsize`/`protect`/`heap`,
which mutate state so a second lap is a different benchmark; the Glk groups,
which would compare our null sink against a real terminal; and the float/double
groups, which measure the host FPU).
