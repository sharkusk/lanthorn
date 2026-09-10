# lanthorn-zvm

A from-scratch, zero-dependency Z-machine virtual machine — Infocom's
interactive-fiction bytecode format, every published version 1 through 8
including the graphical Version 6 titles (*Zork Zero*, *Arthur*, *Shogun*,
*Journey*). Handles execution, standard Quetzal save/restore, and the
per-machine rendering facts (screen model, palettes, fonts) that let a front
end draw a release the way its original interpreter did.

It is the engine behind [lanthorn](https://github.com/sharkusk/lanthorn), a
terminal interactive-fiction player with live automapping, and is also usable
standalone by anything that wants to run Z-machine story files.

## Quick start

```rust,no_run
use zvm::cpu::exec::{BootConfig, Machine, StepResult};
use zvm::io::BufferOutput;
use zvm::memory::Memory;

let bytes = std::fs::read("story.z5")?;
let mem = Memory::new(bytes)?;
let mut m = Machine::boot(mem, Box::new(BufferOutput::new()), BootConfig::new());

loop {
    match m.step() {
        StepResult::Continue => {}
        StepResult::NeedLine { .. } => {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            m.supply_line(line.trim_end_matches(['\n', '\r']), 13);
        }
        StepResult::Quit => break,
        StepResult::Fault => {
            eprintln!("story faulted: {:?}", m.take_fault_trace());
            break;
        }
        // `StepResult` is `#[non_exhaustive]`; match the rest with a wildcard.
        _ => {}
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

See `examples/run_story.rs` for a complete, runnable stdin/stdout host
(versions 1-5, 7-8 — Version 6 needs a real renderer) and
`examples/v6_host.rs` for the graphical seams a Version 6 host implements
(`Resources`, paint-event draining, a `V6Metric` text face), rendered
headless to a PPM.

## What the host implements

`Machine::boot(mem, output, BootConfig)` takes every fact a host has an
opinion about — honouring the story's own colour requests, a PRNG seed, the
default page/ink pair, the interpreter number/version to advertise, the
palette, the Version 6 character cell and picture-space scale, the screen
size — and applies them in the one order that is correct; a bare
`BootConfig::new()` states no opinion and boots exactly as a default
Z-machine would. From there the host drives an explicit, pull-based
protocol — nothing runs on a callback or a background thread:

- **`Machine::step()`** runs one instruction and returns a `StepResult` that
  is `Continue` until the machine needs something: `NeedLine`/`NeedChar` (a
  line or a single keypress — answer with `supply_line`/`supply_char`),
  `SaveRequest`/`RestoreRequest` (answer with `complete_save` and
  `complete_restore_success`/`complete_restore_failure`), `Restart` (answer
  with `Machine::restart()`, never by rebuilding the machine — ZMSD §6.1.3
  requires the two game-writable `Flags 2` bits to survive a restart, along
  with every mid-session host setting), `Quit`, or `Fault` (a runtime fault;
  `take_fault_trace()` gives the stack trace). `step()` is idempotent while a
  suspension is pending — calling it again before answering one returns the
  identical result rather than completing it with blank/default content.
- **Draining.** Several independent queues accumulate host-facing events
  between drains: `take_paint_events()` (every Version 6
  `draw_picture`/`erase_picture`/`erase_window` fill, merged onto one
  timeline in issue order — replaying pictures and fills as two separate
  lists reorders a turn and erases artwork), `take_pending_sounds()`,
  `take_diagnostics()`, `take_screen_trace()` (once tracing is turned on),
  and `take_fault_trace()`.
- **Rendering.** `Machine`'s `screen` field is a public `screen::ScreenState`
  — window layout, cursor, text style, and (for Version 6) the
  pixel-addressed window model — a host walks it directly to render a frame.
  Text itself flows through the pluggable `io::Output` sink supplied at
  boot; `io::BufferOutput` is a minimal accumulating sink for tests and
  headless use.
- **Saving.** `Machine::save_quetzal()`/`restore_quetzal()` produce and
  consume the spec-conformant Quetzal blob any Z-machine interpreter can
  read — what `@save`/`@restore` in the story itself use. Quetzal carries no
  screen state by design (the standard assumes the story repaints after a
  restore), so a host snapshotting a *live* display outside the story's own
  save points additionally wants `Machine::screen_snapshot()` /
  `restore_screen_snapshot(&[u8])` — a versioned, dependency-free binary
  blob covering the classic screen state and, for Version 6, all eight
  windows' geometry, grid, and printed runs.
- **Version 6 picture resources.** A Version 6 story asks how many pictures
  its archive holds and how big each one is. Implement the `resources::Resources`
  trait for a host whose archive is streamed or lazily decoded, or use
  `resources::PictureTable` (what `Machine::set_picture_dims` and
  `BootConfig::with_picture_dims` build) for a host that already has the
  whole table; install either with `Machine::set_resources` or
  `BootConfig::with_resources`. Widths and heights are answered in the
  resource's own pixels — `BootConfig` scales every answer into the story's
  unit screen.
- **Version 6 text metrics.** `screen::V6Metric` pairs the declared character
  cell with a per-ZSCII-byte advance table; install one with
  `Machine::set_v6_text` for a host painting proportional text, or rely on
  the fixed-pen default.

## Robustness

A malformed or hostile story file never panics the load path: `Memory::new`
validates the header and memory map and returns a `error::ZError` naming
what's wrong, and there is no raw indexing of a story address anywhere in
`memory.rs` — reads go through bounds-checked accessors, latch a fault
instead of panicking, and writes below `static_mem_base` are refused rather
than corrupting memory. A runtime fault during play surfaces as
`StepResult::Fault` rather than an error return, since that is the right
shape for a VM: the machine keeps its state and a host can inspect
`take_fault_trace()` rather than losing the session.

`crates/zvm/src/fuzz_harness.rs` runs a hand-rolled, zero-dependency random-
and mutated-story generator through `step()`, the restore paths, and the
disassemblers on every `cargo test`/`cargo nextest` invocation, asserting no
panic and no hang; a separate `cargo-fuzz` sweep (`crates/fuzz/`, run by
hand, not part of CI) hunts for anything a hand-rolled generator wouldn't
think to try. See `docs/internals/fuzzing.md` for what each half covers and
what it has already found and fixed (an integer-overflow panic reachable
through `put_wind_prop`, an unbounded `print_table`/`copy_table` denial of
service, a `DisasmCache` panic on a header pointing past EOF). Some of that
work is still open — check that document for the current state before
depending on this crate to survive a truly hostile input unattended.

## Stability

This crate is pre-1.0: a semver-minor bump may still break API. Read-only
result and event types (`StepResult`, `ZError`, `PaintEvent`, and others) are
marked `#[non_exhaustive]`, so match them with a wildcard arm — a new variant
is then an additive, not a breaking, change even before 1.0.

## Where to read more

Crate-level documentation (`cargo doc --open -p lanthorn-zvm`, or
[docs.rs/lanthorn-zvm](https://docs.rs/lanthorn-zvm) once published) covers
the full boot recipe, the drain protocol, and the save/restore story in
detail. `docs/internals/zvm-embedding-review.md` in the lanthorn repository
is a standing review of this crate specifically as a dependency for someone
else's project — read it for the seams that are still rough and what has
already been resolved. `docs/internals/fuzzing.md` covers the hostile-input
harness referenced above.

License: BSD-3-Clause.
