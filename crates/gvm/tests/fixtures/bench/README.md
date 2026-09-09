# gvm benchmark fixtures (SQ-1428)

What `cargo run --release -p lanthorn-gvm --example bench` drives. The recorded
baselines, the machine they were taken on and the matching `glulxe` commands are
in [`docs/internals/performance.md`](../../../../../docs/internals/performance.md).

## Story

**`../../../../gvm-cli/tests/fixtures/glulxercise.ulx`** — Andrew Plotkin's Glulx
interpreter unit test, Release 13 / serial 241202 / Inform v6.43, 231,680 bytes,
sha256 `b732127fee4cb266a5330981c1111fdfaba237134525754e063e6dc5f449b348`.

**Source:** <https://eblong.com/zarf/glulx/> (the glk-dev `unittests/`
manifest), freely redistributable on Plotkin's own terms — the same origin as
the eleven conformance stories [`../README.md`](../README.md) documents.

**Not copied here.** The workspace keeps exactly one committed `glulxercise.ulx`,
in `gvm-cli`'s fixtures, and every `gvm` suite that wants it reaches across by
relative path — `tests/object_words.rs`, `tests/grammar_tables.rs` and
`src/disasm.rs` all spell
`PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../gvm-cli/tests/fixtures/glulxercise.ulx")`,
and `examples/bench.rs` now does the same. A second copy would be 231 KB of
binary that can silently drift from the one the conformance suites read, which
is exactly the sort of divergence a benchmark must not be able to hide.

**Why glulxercise and not a real game.** Adventure/`advent.ulx` would be more
game-shaped, but its Glulx builds carry no licence anybody can point to, and
what the Glulx side of this comparison is actually asking about is the
**dispatch loop** — the quest that prompted this page recorded gvm as 1.2–1.5x
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
