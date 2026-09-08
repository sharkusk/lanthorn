# gvm-cli test fixtures

## glulxercise.ulx

The **glulxercise** Glulx interpreter conformance suite by Andrew Plotkin — the
Glulx analogue of the Z-machine `czech`/`praxix` torture tests. Driven headlessly
by `tests/glulxercise.rs`.

- **Source:** <https://eblong.com/zarf/glulx/glulxercise.ulx>
- **Version:** Release 13 / Serial 241202 (Inform v6.43), game-file format 3.1.3
- **SHA-256:** `b732127fee4cb266a5330981c1111fdfaba237134525754e063e6dc5f449b348`
- **Downloaded:** 2026-06-27 via `curl -L`

### All 68 groups pass (SQ-1417)

The story itself names 70 test-group commands (its `help`/boot banner lists
them; `tests/glulxercise.rs` parses that list at run time rather than hand-
maintaining a copy — see that file's module doc). `tests/glulxercise.rs`
asserts **every one of them except two** reports `Passed.`, no exclusions
beyond those two, and no hand-curated allow-list to go stale as glulxercise
adds groups in a future release:

- `random` — genuinely exercises the RNG and carries its own disclaimer
  ("Tests may, very occasionally, fail through sheer bad luck"); a
  conformance gate that can fail on bad luck isn't one. `nonrandom`
  (deterministic) exercises the same opcode and IS asserted.
- `safari5` — not a test of the interpreter at all; its own description says
  it tracks "a known Javascript bug in Safari 5 ... on Quixe" and always
  reports `Passed.` regardless of VM behavior.

This supersedes an earlier (2026-06-27 through SQ-1416) 26-, then 39-group
allow-list that undersold gvm's actual coverage in two different ways at
once: it asserted only a curated subset even though several groups outside
it (`operand`, `callstack`, `mcopy`, `nonrandom`, every `double*` group) were
passing all along and simply never got their own line in the list, while
separately claiming in prose below that some of those same names — and
`restore` — were "not yet implemented", which by SQ-1415 was no longer true
either. Both were corrected by widening to the full 68 rather than editing
either list by hand again; see `tests/glulxercise.rs`'s module doc for gvm's
xorshift32 vs. glulxe's xoshiro128** note on why `random` specifically can
never be a byte-level comparison even where it's exercised elsewhere.

The **filter I/O system** (iosys mode 1; SQ-0245), the **Glk dispatch
output-argument marshalling** group `gidispa` (SQ-0251: a type-tagged Inform
string object handed to `glk_put_string`/`glk_put_string_uni` decodes its
type byte like `@streamstr` instead of streaming the tag as a stray leading
char), **double-precision opcodes** (gestalt `Double` = 1; `fmod`/`dmodr`/
`dmodq` and `ftonumz`/`ftonumn`/`dtonumz`/`dtonumn` ported from glulxe's
`exec.c` formulas exactly, SQ-1415), **acceleration** (accelerated functions
are intercepted by default; `--accel off` disables), and the game's own
**`@save`/`@restore`** (SQ-1415's CMem-reader fix — a compressed-memory
stream a foreign writer ended early is not truncation — made `restore` and
its siblings `undo`/`multiundo`/`protect`/`memsize`/`undomemsize`/`heap`/
`undoheap`/`undorestart` all pass) are all included in the 68 and all pass.

(The gi_dispatch *introspection* API — `gidispatch_count_classes`, prototype
queries — is unreachable from Glulx bytecode and remains unimplemented; the
`gidispa` group does not exercise it, so this is not a gap the 68 groups can
show either way.)
