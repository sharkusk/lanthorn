# gvm test fixtures

## The glulxe/cheapglk oracle

The `.glulxe.txt` and `.glksave` fixtures below were recorded against
**glulxe 0.6.1** (Andrew Plotkin's reference Glulx interpreter) linked
against **cheapglk 1.0.7** (its minimal "dumb terminal" Glk library), both
built from source with `cc`:

```sh
git clone https://github.com/erkyrath/cheapglk
git clone https://github.com/erkyrath/glulxe
cd cheapglk && make   # produces libcheapglk.a
cd ../glulxe && make  # GLKINCLUDEDIR/GLKLIBDIR point at ../cheapglk, GLKMAKEFILE=Make.cheapglk
```

Homebrew's `glulxe`/`cheapglk` formulas were NOT used — homebrew's `glulxe`
links against `libncurses` (confirmed via `otool -L`) and is not headless-
scriptable; only a from-source build against cheapglk answers stdin/stdout
like a script expects. `-q -u` on every invocation suppresses cheapglk's own
banner line and requests UTF-8 I/O (see `glk_conformance_corpus.rs`'s module
doc for why `-u` turned out to make a Latin-1 normalization unnecessary).

Every transcript here was captured by scripting `glulxe -q -u <story.ulx> < input.txt`
against the SAME `.ulx`/commands `crates/gvm/tests/glk_conformance_corpus.rs`
scripts through gvm, then trimming cheapglk's own `\n<end of input>\n` EOF
notice (printed once stdin closes — not story or gvm output) from the end.
See that file's per-test doc comments for the exact command each fixture
corresponds to, and its module doc for the three narrow, documented
normalizations applied before comparing (an interpreter's own version-banner
line; `datetimetest`'s live wall-clock text, which cannot be recorded at
all — only its UTC offset can be fixed to match; a Glk dispatch id's
opaque handle number).

## `.glulxe.txt` transcripts (glk_conformance_corpus.rs)

All eight are UTF-8 plain text, byte-for-byte matched (after normalization)
against gvm's own `TestBackend::all_text()` for the same scripted input. Each
corresponds to one `.ulx` under the repo-root `unit_tests/` (gitignored,
fetched per that directory's own README — see the project `CLAUDE.md` Test
Fixtures section) — freely redistributable per Andrew Plotkin's
glk-dev `unittests/` manifest, same terms as `glulxercise.ulx` above.

| Fixture | Story | What it exercises |
|---|---|---|
| `resizememstreamtest.glulxe.txt` | `resizememstreamtest.ulx` | a memory-pointer-resize bug class (glkop.c holding a stale pointer across `realloc`); self-checking, no input |
| `memcopytest.glulxe.txt` | `memcopytest.ulx` | `@mcopy`'s overlap handling — both the forward- and backward-overlapping branches (SQ-1415 hardened the descending one) |
| `memstreamtest.glulxe.txt` | `memstreamtest.ulx` | byte/Unicode memory streams, null streams, positioning, reading (including a non-Latin-1 character) |
| `memheaptest.glulxe.txt` | `memheaptest.ulx` | `@mallocheap`/`@malloc`/`@mfree` status and block addresses |
| `datetimetest.glulxe.txt` | `datetimetest.ulx` | the story's own timeval-to-date conversion via its fixed `x calendar` historical-dates listing |
| `unicasetest.glulxe.txt` | `unicasetest.ulx` | Unicode case-folding, decomposition, normalization (self-checking, `all`) |
| `extbinaryfile.glulxe.txt` | `extbinaryfile.ulx` | binary/text, char/word, Unicode fileref I/O round trips (self-checking, no input) |
| `selectvarianttest.glulxe.txt` | `selectvarianttest.ulx` | a memory stream kept open across `glk_select` char-event input, plus opcode-operand-source (stack vs. local) variations |
| `inputeventtest.glulxe.txt` | `inputeventtest.ulx` | one scripted char event then one scripted line event — the two input-request kinds a Glk story alternates between |

Three more corpus stories (`randomgen.ulx`, `statusbufferwin.ulx`,
`inputfeaturetest.ulx`) are driven by the same test file but are NOT diffed
against glulxe — for reasons that are properties of the oracle, not of gvm
(a PRNG algorithm the Glulx spec leaves implementation-defined and glulxe
happens to pick differently; two real cheapglk feature gaps, multi-window
layout and line-input terminators/echo). See `glk_conformance_corpus.rs`'s
module doc for the full explanation of each; no `.glulxe.txt` fixture exists
for them because there is nothing correct to record.

## `.glksave` saves (foreign_save_interop.rs, closing SQ-0229)

`statusbufferwin_apple.glulxe.glksave` and `statusbufferwin_apple.gvm.glksave`
are `FORM IFZS` (Glulx-Quetzal) saves of the IDENTICAL state — boot
`statusbufferwin.ulx`, `take apple`, `save` — one written by glulxe, one by
gvm. Both directions of cross-interpreter restore were verified; see
`GLULX_NOTES.md` §14's "Foreign-save interop" note for the full write-up,
including why the `MAll` chunk differs in byte length between the two
(both are correct — glulxe's own `heap_apply_summary` explicitly special-
cases gvm's spelling of "no heap") and the resolved-path gotcha that made
the first restore attempt read `Restore failed.` for a reason unrelated to
the save format. `foreign_save_interop.rs` automates the direction that can
run in the normal test suite (gvm restoring glulxe's save, then perturbing
with `inventory` before asserting per this project's restore-testing
convention); the reverse was checked by hand against the built oracle above,
since glulxe/cheapglk are not a workspace dependency.

## `startsavetest.gblorb`

Unrelated to the oracle above — SQ-1415's fixture, eblong.com's own
self-checking Glulx save/restore conformance story (autorestores on boot and
reports success/failure itself). Driven by `startsavetest_boots.rs`. See that
file and `GLULX_NOTES.md` §14 for detail; kept here rather than duplicated.

## `bench/` — the timing harness's story and script

`bench/glulxercise.ulx` and `bench/glulxercise.script` drive `cargo run
--release -p lanthorn-gvm --example bench`; see
[`bench/README.md`](bench/README.md) for the story's provenance and why it is
committed here rather than fetched into `unit_tests/`, and
`docs/internals/performance.md` for the recorded baselines against the
glulxe/cheapglk build described at the top of this file (SQ-1428).
