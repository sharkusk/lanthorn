# lanthorn-gvm

A zero-dependency Glulx virtual machine (with a Glk I/O layer) for modern
Inform 7 games, including accelerated-function support for the Inform veneer
and full floating-point opcodes.

It is the Glulx engine behind [lanthorn](https://github.com/sharkusk/lanthorn),
a terminal interactive-fiction player with live automapping, and is also
usable standalone by anything that wants to run Glulx story files.

Floating-point opcodes are ported from [glulxe](https://github.com/erkyrath/glulxe) under the MIT license — see the repository's `THIRD-PARTY-NOTICES.md` for full attribution and license text.

## Quick start

```rust,no_run
use gvm::{GlkBackend, Machine, Memory, StepResult, TestBackend};

let image = std::fs::read("story.ulx")?;
let mem = Memory::new(image)?;
let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));

loop {
    match m.step() {
        StepResult::Continue => {}
        StepResult::NeedLine { .. } => {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            m.supply_line(line.trim());
        }
        StepResult::NeedChar { .. } => m.supply_char(' ' as u32),
        StepResult::Quit => break,
        StepResult::Fault => {
            eprintln!("story faulted: {:?}", m.take_fault_trace());
            break;
        }
        // A real host also answers `SaveRequest`/`RestoreRequest` and the
        // other `StepResult` variants; see `examples/run_story.rs`.
        _ => {}
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

`examples/run_story.rs` is a complete, runnable stdin/stdout host: it unwraps
a `.gblorb` archive (bare `.ulx` images pass through unchanged), answers the
Glk fileref/file-VFS seam, and persists both the game's own `@save` and its
file-backed data between runs.

## What the host implements

`Memory::new` validates a Glulx image (the `Glul` magic, a supported major
version, a consistent memory map) and returns a `GError` naming what's wrong
when it doesn't fit. `Machine::with_glk` then takes that memory plus a
`Box<dyn GlkBackend>` and builds a machine sitting at the start function,
ready to run. From there the host drives an explicit, pull-based protocol —
nothing runs on a callback or a background thread:

- **`Machine::step()`** runs until the machine needs something from the host,
  returning a `StepResult`: `NeedLine`/`NeedChar` (answer with
  `supply_line`/`supply_char`, then keep stepping), `Quit` (the story ended
  cleanly), or `Fault` (a runtime fault halted it, distinct from `Quit` so a
  host that never reads `diagnostics` can still tell a crash from a clean
  exit — `take_fault_trace()` gives the detail).
- **`GlkBackend`** is the display seam: 35 methods, all but two
  (`as_any`/`as_any_mut`) defaulted to a no-op or "the host has no such
  facility", so a minimal text-only backend implements almost nothing and
  gets windows, styled text, graphics, sound, screen size and glyph metrics
  for free the moment it wants them. `TestBackend` is a complete in-memory
  implementation (built for this crate's own tests, and usable as a starting
  point); a real host implements the trait itself.
- **Events are pulled, not pushed.** Sound, timer, mouse, hyperlink and
  window-arrangement events all work the same way: the VM holds no clock or
  event queue of its own. `Machine::glk_timer_interval()` reports the
  interval the game last requested (or `None`); the host keeps wall time and
  calls `Machine::deliver_timer()` when it elapses, and similarly for
  `deliver_arrange`, `deliver_mouse`, `deliver_hyperlink`,
  `deliver_sound_notify` and `deliver_volume_notify`.
- **Accelerated functions need nothing from the host.** The 13 well-known
  Glulx `@accelfunc` routines (spec §2.17) that the Inform veneer relies on
  for hot object-model work are implemented natively rather than
  interpreted. A recognised Inform 6 veneer is fingerprinted before the
  first opcode runs, so a story is accelerated from its very first turn even
  if it never calls `@accelfunc` itself; a story that does call it overwrites
  those assignments with its own, naming the same routines.
- **Saving.** `Machine::save_quetzal()` returns the spec-conformant Quetzal
  `IFhd`/`CMem`/`Stks`/`MAll` blob any Glulx interpreter can read back with
  `Machine::complete_restore_quetzal()` — this is what `@save`/`@restore` in
  the story itself use, via `StepResult::SaveRequest`/`RestoreRequest`.
  `Machine::save_state()` adds this crate's own Glk state (window layout,
  streams — Glk state the story didn't ask to save, needed for a host that
  wants to snapshot a session anywhere rather than only at the game's own
  save points); `Machine::complete_restore_success()` reads either shape
  back. Quetzal carries no screen contents by design — the story is expected
  to repaint — so a host snapshotting a live display keeps that state
  itself.
- **Files.** A story's Glk filerefs and file streams are serviced against an
  in-memory VFS this crate owns and never writes to disk itself; see
  `docs/internals/gvm-fileref-seam.md` in the lanthorn repository for the
  full host contract — which `StepResult`s to answer, what to persist
  between runs, and the fileref-name-to-disk-filename boundary.

## Robustness

A malformed Glulx image never panics the load path: `Memory::new` validates
the header and memory map and returns a `GError` naming what's wrong. Runtime
faults surface as `StepResult::Fault` rather than an error return or a
process abort, so a host can inspect `take_fault_trace()` and keep going
rather than losing the session.

`crates/gvm/src/fuzz_harness.rs` runs a hand-rolled, zero-dependency random-
and mutated-image generator through `step()`, the restore paths, and the
disassembler on every `cargo test`/`cargo nextest` invocation, asserting no
panic and no hang; a separate `cargo-fuzz` sweep (`crates/fuzz/`, run by
hand, not part of CI) hunts for anything a hand-rolled generator wouldn't
think to try. See `docs/internals/fuzzing.md` for what each half covers.

## Performance

Measured with the workspace's own harness (`cargo run --release -p lanthorn-gvm --example bench`),
interpreter core only, one machine, one story. Details, profiles and the caveats are in
[`docs/internals/performance.md`](../../docs/internals/performance.md).

| story | turns | lanthorn-gvm | reference | ratio |
|---|---|---|---|---|
| glulxercise.ulx | 2,700 | 0.628 s (26.9 M opcodes/s) | glulxe 0.6.1 + cheapglk 1.0.7: 1.13 s | 1.80× |
| glulxercise.ulx `--no-accel` | 2,700 | 0.972 s (28.5 M opcodes/s) | glulxe 0.6.1 + cheapglk 1.0.7: 1.13 s | 1.16× |

The default row above is with acceleration on, which is what a game gets; `--no-accel` is the like-for-like dispatch-loop measurement against glulxe.

Apple M2 Max, macOS 26.6.2 (build 25G83), rustc 1.98.0, `--release`, 2026-09-08. A different story or machine
gives a different ratio; rerun the harness rather than quoting this line.

## Stability

This crate is pre-1.0: a semver-minor bump may still break API. Read-only
result and event types (`StepResult`, `GError`, and others) are marked
`#[non_exhaustive]`, so match them with a wildcard arm — a new variant is
then an additive, not a breaking, change even before 1.0.

## Where to read more

Crate-level documentation (`cargo doc --open -p lanthorn-gvm`, or
[docs.rs/lanthorn-gvm](https://docs.rs/lanthorn-gvm) once published) covers
the full `GlkBackend` seam, the accelerated-function set, and the save/restore
story in detail. `docs/internals/gvm-fileref-seam.md` and
`docs/internals/fuzzing.md` in the lanthorn repository cover the file VFS
contract and the hostile-input harness referenced above.

License: BSD-3-Clause.
