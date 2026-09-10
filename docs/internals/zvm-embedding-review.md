# Reviewing `zvm` as a crate someone else depends on

[← back to architecture](architecture.md)

`zvm` is to be the standard Rust Z-machine: a pure, zero-dependency VM core that
someone who has never heard of lanthorn can put behind their own front-end. That
goal changes what `pub` means. Every one of the crate's ~360 public items is
currently a convenience extended to `app`, and the day a stranger builds against
one it becomes a promise. This document reads the surface as that stranger would
meet it, ranks what should change, and says for each finding whether changing it
is free today and expensive later.

The framing matters more than any individual item on the list. `zvm` is `0.2.0`
and nothing outside this workspace depends on it, so a breaking change right now
costs a rename in three of our own crates and nothing else. The same change once
a third party has shipped against it costs them a migration and costs us the
argument about whether it is worth it. **Several findings below are breaking
changes that are free this week and permanent debt next year**, and separating
those from the additive ones is the most useful thing this review produces.

A note on what has already moved: SQ-1013 has landed since this review was
scoped. `app::InterpreterProfile::v6_font_cell`, `std_window` and
`default_colours` are now one-line forwarders — `self.machine().map_or(…)` — over
`zvm::interpreter::MachineProfile`'s `v6_cell`, `v6_std_window` and
`default_colours` fields, and `AMIGA_STD_WINDOW` / `MACINTOSH_STD_WINDOW` are
`pub use zvm::interpreter::{…}` re-exports at `crates/app/src/interpreter.rs:941`.
The machine table is where it should be. What is left in `app` and shouldn't be
is a different shape, and §8 names it.

## 1. Write the host first

The sharpest test of an API is writing against it, so here is what a third-party
embedder must write to load a story, run to the first prompt, render a screen,
feed a line, and save. It is not a lot of code. It is a lot of *knowledge*, and
almost none of the knowledge is in `zvm`'s docs.

```rust
// 1. Load.
let mem = zvm::memory::Memory::new(std::fs::read("story.z5")?)?;
let mut m = zvm::cpu::exec::Machine::with_output(mem, Box::new(MySink::default()));

// 2. Configure — in an order nothing in zvm states.
m.set_honor_game_colours(true);   // writes Flags1 bit 0 immediately
m.set_sound_available(false);     // writes the sound bits immediately
m.set_rng_seed(entropy());        // MUST precede the boot run: initialisation draws
m.set_default_colours(2, 9);      // MUST precede init_caps: Beyond Zork reads $2C/$2D while booting
m.set_picture_dims(dims);         // MUST precede the boot run: v6 calls picture_data during boot
m.set_interpreter_number(Some(4));// latched — takes effect AT init_caps, not now
m.init_caps();                    // and only now is the header a real machine's
m.set_screen_dims(rows, cols);    // AFTER init_caps, which seeded 80x24 over the top

// 3. Drive to the first prompt.
loop {
    match m.step() {
        StepResult::Continue => {}
        StepResult::NeedLine { .. } | StepResult::NeedChar => break,
        StepResult::Restart => m.restart(),
        StepResult::SaveRequest => m.complete_save(false),
        StepResult::RestoreRequest => m.complete_restore_failure(),
        StepResult::Quit | StepResult::Fault => return,
    }
}

// 4. Render. `m.screen` is a pub struct; walk it yourself.
// 5. Drain. Six independent Vecs, by std::mem::take, in the right order.
// 6. Feed. m.supply_line(&text, terminator);
// 7. Save. m.save_quetzal() — plus your own serialisation of m.screen,
//    because Quetzal carries no screen state and the host owns the pixels.
```

Every comment in step 2 is a real constraint, every one of them was learned by a
lanthorn defect, and **not one of them is stated where an embedder would look**.
`Machine::init_caps`'s own doc (`crates/zvm/src/cpu/exec.rs:595`) says only "call
this from the host after loading a real story file, before the first `step()`".
It names nothing that must precede *it*. The three classes of setter — those that
write the header immediately (`set_honor_game_colours`, `set_sound_available`),
those latched until `init_caps` (`set_interpreter_number`), and those that affect
nothing in the header but must precede the boot *run* (`set_picture_dims`,
`set_rng_seed`) — are distinguishable only by reading each doc comment and
noticing which of them mentions `init_caps`. The ordering rationale lives instead
in `crates/app/src/session.rs:818` and `:823`, in a private function's doc
comment in a different crate.

The evidence that this is a real cost and not a stylistic complaint is that the
workspace has two embedders and they do not agree. `crates/app/src/session.rs:3762`
answers `StepResult::Restart` with `machine.restart()`, which is ZMSD §6.1.3's
reboot-in-place, preserving the two game-writable Flags 2 bits.
`crates/zvm-cli/src/main.rs:1739` answers the same `StepResult` by throwing the
machine away and calling `build_machine` again from the original bytes, which
does not preserve them. One of those is wrong about a spec clause, they were
written by the same people about the same crate, and `zvm` never expressed an
opinion. A third-party host is being asked to answer the same question a third
time.

## 2. The process-global palette

`crates/zvm/src/screen.rs:1787` and `:1793` held two process-wide atomics —
`ACTIVE_PALETTE` and `INTERPRETER_VERSION` — written through `set_palette` and
`set_interpreter_version`. This was the crate's worst embeddability defect.

The justification stated in the doc at `screen.rs:1778` was: "the palette
is a property of *the machine lanthorn is pretending to be*, and there is exactly
one of those per run." That premise was true of lanthorn and was not a fact about
the Z-machine. It was false for a GUI with two windows open, for a server running
a session per player, for a test harness comparing an Amiga press against an IBM
one, and for any host that puts a `Machine` on a thread. Two `Machine`s in one
process could not have different palettes, and there was no API by which they
could.

Three things made this rank first rather than third.

**The project already paid for it, in a currency it could measure.** `CLAUDE.md`
devoted its longest section to the consequences: four consecutive red builds on
main (SQ-0904, SQ-0958, SQ-0959, SQ-0987) and a three-layer apparatus built to
contain them — a mutex kept *private to `app`* so a test suite physically could not
take it raw, an `app::v6_set_palette` that panicked unless the calling thread held
a guard, and a source-scanning test, `palette_lock_discipline`, that failed any
file under `tests/suites/` naming `zvm::screen::set_palette` directly. The
apparatus was not small: `crates/app` held **234 `v6_palette` / `v6_palette_at_boot`
guard acquisitions and 44 `v6_set_palette` calls**, none of which would have existed if
the palette were a field on the session being rendered. **An
embedder would inherit the hazard and none of the apparatus**, and would not know to
build it, because the reason it existed was documented in our repo instructions
rather than in the crate.

**`zvm`'s own docs argued against it.** `screen.rs:2222`, on `V6Cell`, read: "it
is emphatically not process-global — see `zvm::screen::set_palette` for what that
costs." The cell was moved onto `Machine` for exactly this reason. The palette was
the same kind of fact, moved by the same argument, and had not moved.

**The blast radius was four call sites.** Grepping non-test readers of both
statics inside the crate found exactly this:

| site | what reads the global |
|---|---|
| `screen.rs:1579` | `init_header_caps` writing the `$1F` interpreter-version byte |
| `screen.rs:1734` | `two_colour_card_request`, reached from `exec.rs:1453` — inside `Machine`, so `&self` is in scope |
| `screen.rs:1905` | `standard_true_colour`, the only reader of `palette()` |
| `screen.rs:70,72` | `ZColour::true_value`, which called the above, reached from `exec.rs:3096-3097` (window properties 16/17) |

Outside the crate there was one non-test reader, `crates/app/src/colors.rs:65`,
which was a host renderer and should have been asking the session it was rendering rather
than the process. The plumbing for the fix already existed: `true_colour_in(p, n)`
at `screen.rs:1918` took the palette by value and was written precisely so
`crates/zvm/src/machines.rs` could print every machine's table side by side
without writing global state — the doc at `:1913` explained that a
borrow-and-hand-back "is atomic to nobody". That argument was correct and it
applied to sessions as much as to tables.

**Resolved (SQ-1393, 2026-09-07).** `Machine::palette` and `Machine::interpreter_version` are now fields with setters, carrying those facts on every machine instance. `init_header_caps` and `two_colour_card_request` read them from `&self`. `ZColour::true_value` takes the palette by value (alongside the `interpreter_default` it already took), resolving a colour number with both facts in scope. The change deleted `app::v6_palette`, `app::v6_set_palette`, `app::v6_palette_at_boot`, the private mutex, both cases of `palette_lock_discipline`, and every one of the 234 guard acquisitions in `app`. `MachineBoot::resolve` carries both facts and `GameSession` sets them on the machine before `init_caps`; `ColorScheme` gained `machine_palette` so renderers read the session's table. The palette problem is now `zvm`'s alone and is no longer one.

## 3. There is no boot recipe, so the host is the spec

Everything §1 spells out should be one value and one call. `Machine::boot(mem,
BootConfig { .. })` owning the ordering — construct, apply, `init_caps`, size the
screen, drive to the first stop — would move ~100 lines of Z-machine knowledge out
of `crates/app/src/session.rs` and into the crate that knows it, and would give the
`Restart` divergence in §1 a single answer. The individual setters stay for
mid-run changes; what they stop being is the *only* door.

This is the repo's own refactoring policy applied to itself. `CLAUDE.md`: "facts
that must be considered together should travel together as a value, not
positionally", and "a hand-maintained invariant across files is the symptom; the
cure is a type". `app::machine_boot::MachineBoot` exists because five per-machine
boot facts as five positional arguments were omitted one at a time by four
separate callers, including `reset.rs` in production (SQ-0901, SQ-1020, SQ-1021,
SQ-1022). `zvm` has the same problem one layer down and has not applied the same
cure. Note the two are different layers and both are wanted: `MachineBoot` answers
"what does this *medium* say the machine is", `BootConfig` answers "in what order
must a `Machine` be told things".

Additive, non-breaking, and the thing an embedder feels on day one.

**Resolved (SQ-1396, 2026-09-08).** `zvm::cpu::exec::BootConfig` (in `crates/zvm/src/cpu/boot.rs`; `#[non_exhaustive]`, `new()` + `with_*` builders) and `Machine::boot(mem, out, config)` apply the ordering spelled in §1 in one stated pass (palette and interpreter version first, then the immediate header writers, RNG seed, default colours, v6 text metric, picture dims, the latched interpreter number, `init_caps`, and the screen size after it), returning a machine ready for its first `step()` without stepping. App: `MachineBoot::boot_config(...)` converts its machine facts and config into `BootConfig`; `GameSession::from_boot_config` replaces the fourteen-argument constructor; both `reset.rs` and `startup.rs` reach it through `new_for_machine`. The bare `BootConfig::new()` boots byte-for-byte as the old path did, pinned by a test. The `Restart` answer is settled: ZMSD §6.1.3 preserving "Flags 2" is now `Machine::restart()`, and `zvm-cli` calls it rather than reconstructing.

## 4. The drain protocol is prose, and one rule of it is correctness-critical

`crates/zvm/src/cpu/exec.rs:212` still says "Fields are `pub` so Tasks 11+ can
attach I/O channels" — a planning scheme retired long ago, which left 27 of
`Machine`'s 41 fields public behind it. Several are queues the host must empty, and the
only way to empty them is `std::mem::take` on a bare `Vec` with no method to
call: `pending_sounds` (`:273`), `pending_pictures` (`:282`),
`pending_erase_fills` (`:312`), `diagnostics` (`:367`), `screen_trace` (`:402`),
`exec_pcs` (`:407`), `v6_prose_retired` (`:328`). Nothing enumerates them. Miss
one and it grows without bound for the life of the session, silently.

The serious half is the interleave rule. `exec.rs:148` explains, in a comment,
that draining `pending_pictures` and `pending_erase_fills` as two separate lists
replays a v6 turn in the wrong order and erases the artwork — because *scopa*
draws every playing card with `erase_window` fills and every fill is ordered
against every picture. That is a correctness constraint on the host, discoverable
only by reading a comment on a private constant, and every embedder must
reimplement the merge from prose. It should be one method — `take_paint_events()
-> Vec<PaintEvent>`, already merged, ordered, and impossible to get wrong.

Additive as `take_*` methods; breaking only if the fields are also made private,
which they should eventually be.

**Resolved (SQ-1396, 2026-09-08).** `PaintEvent { Picture, Erase }` and `Machine::take_paint_events()` merge pictures and erase fills in issue order (the order was already recoverable from `EraseFill::pics_before`, no new field); `take_pending_pictures` / `take_pending_erase_fills` are removed, the read-only views stay, and the app's own merge loop is deleted. A bug found alongside: `Machine::restart` cleared pending pictures but not pending erase fills.

## 5. Saving a screen is left entirely to the host

Quetzal saves no screen state by design — the standard assumes the story
repaints. A host snapshot does not get that assumption: it swaps memory under a
game that never learns it happened, so everything the screen needs is the host's
to carry. `zvm` provides `save_quetzal` / `restore_quetzal` and stops there.

The result is `crates/app/src/archive.rs`, 2,603 lines, of which a large fraction
is a hand-rolled mirror of `zvm`'s own screen types: `ScreenDto` (`:308`),
`ZWindowDto` (`:490`), `V6WindowsDto` (`:602`), `V6TextDto` (`:484`),
`GridCellDto` (`:481`), `ZColourDto`, `V6WindowOpsDto` (`:133`). Every one of
them must move in lockstep with the `zvm` type it mirrors, across a crate
boundary, with nothing checking that they still agree. A second embedder writes
the whole thing again.

`zvm` already hand-rolls a dependency-free binary format in `quetzal.rs`, so
nothing about the zero-dependency rule blocks a versioned
`Machine::screen_snapshot()` / `restore_screen_snapshot()` alongside it. It must
stay backend- and terminal-neutral — v6 geometry in native pixels, no cell
coordinates, no font metrics, no picker state — which is already the rule
`CLAUDE.md` states for the archive, and which is easier to hold inside `zvm`
than outside it, because inside it there is no terminal to be tempted by.

Additive.

**Resolved (SQ-1401, 2026-09-08).** `crates/zvm/src/screen_snapshot.rs` holds a versioned, dependency-free binary blob (magic `ZSCR`, `u16` version 1, big-endian like Quetzal) with `encode` / `decode` and `Machine::screen_snapshot()` / `restore_screen_snapshot(&[u8])`. Carries the classic screen state, the upper-window grid with each cell's fg/bg (prior mirrors lost per-cell colours across restores), and all eight v6 windows: sixteen properties each, grid, texts, prose, streamed and retired runs. Six transient fields deliberately dropped with a reason each; for v6 `current_fg` / `current_bg` encode as Default because the window table is the authority (ZMSD §8.3). File-repair clamps moved into `decode`. The blob travels as a separate archive entry (`screen.bin`), not inside `EngineSave.bytes`, because `SaveTrigger::Ingame` promises the Quetzal bytes are written verbatim as `game.qzl` and interchange-grade. App: all six DTO mirrors (`ScreenDto`, `ZWindowDto`, `V6WindowsDto`, `V6TextDto`, `GridCellDto`, `ZColourDto`) and `grid_from_dto` deleted, archive.rs 3400 → 3053 lines, `CURRENT_FORMAT_VERSION` 8 → 9 with no shim, and `session::restore_screen` stays the single install point, calling the zvm method.

## 6. Errors, panics and what a hostile story file can do

`crates/zvm/src/error.rs` is fourteen lines and four variants, all of them
load-time: `NotAStoryFile`, `UnsupportedVersion`, `Truncated`, `SaveMismatch`.
Runtime faults do not travel through it — they surface as `StepResult::Fault`
plus a `StackTrace` the host drains, which is the right design for a VM and is
well documented.

**Two things are true at once, and the good one is real.** The running-story
error channel is not `Result` and should not be: faults surface as two in-band
latches — `Memory::mem_fault` (`memory.rs:20`, set at `:155-163`) and
`State::fault` (`cpu/state.rs:33`) — both drained by `Machine::step`
(`exec.rs:1109-1120`) and turned into `StepResult::Fault`. `memory.rs` is the
strongest file in the crate: there is **no raw indexing of a story address
anywhere in it**. `read_byte` / `read_word` (`:95`, `:124`) go through `.get()`,
return 0 and latch; `write_byte` / `write_word` (`:115`, `:141`) refuse writes at
or above `static_mem_base`, latch, and leave memory untouched. Opcode dispatch is
total — every class has a benign catch-all (`exec.rs:1509`, `:1637`, `:1767`,
`:2853`, `:3584`) and `decode_form` covers all 256 opcode bytes. Fuzzing the
inspection APIs (`dictionary`, `objects`, `text::decode`, `ifid`, `world`,
`location`, `save_quetzal`) over 10,000 random stories produced zero panics, and
2,000 hostile Quetzal buffers through `restore_quetzal` likewise: `quetzal.rs`
bounds-checks every read, and abbreviation expansion (`text/decode.rs:134-155`)
is depth-capped, so the classic nested-abbreviation stack overflow is absent.

Of the six non-test `unwrap` / `expect` / `unreachable!` sites, all six are
genuinely unreachable or debug-only. The `unreachable!()` at `disasm.rs:313`
looks reachable and is not — `Operand::Var` returns early eleven lines above, at
`:295-301`.

**The bad one is that a malformed story can still abort the host, and the
mechanism is arithmetic rather than indexing.** `ZWindow::put_prop`
(`screen.rs:667`) is the v6 `put_wind_prop` opcode's setter and it writes the
story's raw `u16` into every window field with no clamp at all — `1 => x_coord`,
`5 => x_cursor`, `6 => left_margin`. Those fields are then added together in the
print path with plain `+`:

| site | expression |
|---|---|
| `exec.rs:4256` | `w.x_coord.max(1) + w.x_cursor.max(1) - 1`, then `abs_x + fw - 1` |
| `exec.rs:4203-4204` | `w.y_cursor += fh;` and `w.x_cursor = w.left_margin + 1` |
| `exec.rs:2776` | `self.screen.cursor_row = start_row + row` in `print_table` |

A v6 `main` of four instructions — `put_wind_prop` window 1 property 1 to
`0xFFFF`, property 5 to `0x7FFF`, `set_window 1`, `print_char` — panics at
`exec.rs:4256` with "attempt to add with overflow". Property 6 instead reaches
`:4204`. The same shape repeats at `exec.rs:3273`, `:3437`, `:4238` and `:4283`.
These are **debug-only** — a release host wraps silently — which is precisely the
split `CLAUDE.md` warns about under "debug vs release overflow", and a library
cannot choose its embedder's profile.

**And the discipline already exists, one file away.** `screen.rs:529` writes the
identical expression as `self.x_cursor = self.left_margin.saturating_add(1)`, and
`exec.rs:3995` writes `w.x_cursor = w.x_cursor.saturating_add(fw)`. `set_cursor`
clamps its arguments (`exec.rs:2485-2486`). `put_wind_prop` does not, so the same
quantity is safe on one path and not on another. This is exactly `CLAUDE.md`'s
"a guard beats a convention": the cure is to clamp inside `put_prop`, or make the
four fields private behind saturating accessors, which fixes all six sites at
once.

`V6Cell` was the same defeat in miniature, and is **fixed** (SQ-1031). `V6Cell::new`
clamps each axis to at least 1, and its doc said why: "Guard against a zero axis
reaching the divisions below. A profile that stated `0` would otherwise panic
somewhere far from the mistake." But `w` and `h` were `pub`, so
`V6Cell { w: 0, h: 0 }` walked straight past the constructor into the divisions at
`exec.rs:4255` and `:3275` — and division by zero panics in **both** profiles.
The guard was documented, correct, and bypassable by a struct literal.

The fields are now private behind `w()`/`h()`, and the type lives in its own
`mod v6_cell` inside `screen.rs` so the private fields are invisible to the rest
of that file too — Rust scopes a private field to the defining module *and its
children*, and `screen.rs` is four thousand lines of exactly the code most likely
to write a new cell. The workspace holds one `V6Cell` literal, inside `new`, and
the guard is now unreachable rather than merely documented. Note that privatising
also closed a second route the original finding did not name: `pub w` is a
*mutable* field on a `Copy` type, so `let mut c = m.v6_cell(); c.w = 0;` bypassed
the constructor exactly as a literal did, and no non-exhaustive marker or private
sentinel field would have stopped it.

**One panic fires in release too.** `code_region` (`disasm_cache.rs:551`) returns
`(min(high_mem_base, boot_root), mem.len())` with no check that start precedes
end. A story whose header `$04`/`$06` point past EOF gives `region_start >
region_end`, `build`'s loop never runs, and `units` is empty — after which
`unit_index_at` (`:251`) evaluates `self.units.len() - 1`. In debug the
`debug_assert!` on the line above fires; in release the subtraction wraps and
`next_addr` indexes `units[0]` on an empty vector. `DisasmCache::empty()`
(`:96`) constructs the same object deliberately. `crates/app/src/session.rs:4102`
builds and navigates this cache.

**Finally, one legal instruction is an unbounded denial of service.**
`print_table` (`exec.rs:2759-2778`) loops `height x width`, both story-controlled
`u16`s, with no cap. Measured in a **release** build: one nine-byte instruction
ran for 29.2 s and peaked at 4.3 GB resident before faulting. The host cannot
interrupt it, because `step()` *is* the interruption point. `copy_table` has the
same shape, and `State::frames` / `eval_stack` (`state.rs:29-31`) have no depth
cap, so runaway recursion is an OOM abort rather than a fault. The awareness
exists elsewhere — the v6 grid is capped by `GRID_CELL_CAP` (`screen.rs:2323`,
applied at `exec.rs:2065` and `:3275`) — it simply has not been applied here.

**Bottom line for an embedder: not yet.** A debug-built host is abortable by a
handful of story bytes; a release-built one survives the arithmetic but still has
the `DisasmCache` panic and the unbounded `step()`. None of the fixes is large,
and the crate has no fuzzing or malformed-input corpus to have caught them —
there is no `fuzz/` directory in the workspace, and the one adversarial test,
`crates/zvm/tests/object_scan_eof.rs`, exists because `czech.z5` once panicked in
`read_byte`. That bug class has bitten before and was patched at the symptom. An
in-repo `#[cfg(test)]` random-story harness costs no dependency and would have
caught all of the above.

**Resolved in part (SQ-1395, 2026-09-07).** `print_table` and `copy_table` check the whole table span against memory up front and fault at once; `State::MAX_CALL_DEPTH = 32_768` frames and `MAX_EVAL_STACK = 614_400` words are both derived from ZMSD §6.3.3's nfrotz figure with 10x headroom.

## 7. Missing seams, and the one `gvm` already built

**`FontMetrics` — closed by `V6Metric` (SQ-1009).** `zvm` named this gap itself,
on `V6Cell`: a proportional renderer "needs per-glyph advances, which is a
`FontMetrics`-shaped thing supplied by the host". That thing now exists as
`screen::V6Metric` — the declared cell and a per-ZSCII-byte advance table in one
value, installed with `Machine::set_v6_text` and defaulting to a fixed pen, so a
host that has no face to offer behaves exactly as before. It is the seam the
standard-implementation goal anticipated: the engine advances its cursor, wraps
its lines and answers header `$30` through the same table the host draws with, and
a GUI painting proportional text supplies one table rather than reimplementing the
layout. The zero-dependency rule is intact — the host builds the table from
whatever font it has, and `zvm` only reads it.

**Resources are a pre-filled `Vec`, not an interface.** `picture_dims:
Vec<(u16, u16, u16)>` (`exec.rs:277`) forces a host to enumerate every picture in
the archive before boot. A `trait Resources` answering on demand would serve a
lazy or streaming host. Blorb living in a separate crate is correct and should
stay; the issue is that the *seam* is a vector rather than a question.

**Resolved (SQ-1402, 2026-09-08).** `crates/zvm/src/resources.rs` holds `pub trait Resources { fn picture_count(&self) -> u16; fn picture_release(&self) -> u16; fn picture_dims(&self, number: u16) -> Option<(u16, u16)>; }` and a `PictureTable` vector-backed implementation. `BootConfig::with_resources(Box<dyn Resources>)` is the new door, serving lazy, streaming or reimplemented backends. `Machine::picture_count()`, `picture_release()`, and `picture_dims(n)` accessors expose the interface; `picture_dims` is no longer a pub field. The unit-space scaling is applied by the private `ScaledResources` adapter that `BootConfig` wraps around resources at query time (see above).

**The scaling rule that governs `picture_data` is not in `zvm` at all.**
`set_picture_dims`'s doc (`exec.rs:1018`) says only "the host builds this from
the self-blorb's `Pict` resources". It does not say that for a v6 story the table
must be reported in *unit space* — art-native dimensions multiplied by the
archive's art scale — which is what Infocom's own Amiga/DOS interpreter does and
what a v6 game's layout arithmetic assumes. That rule lives in
`crates/app/src/session.rs` around `V6_ART_SCALE` (`session.rs:194`), and `zvm`
mentions art scale exactly once, in a doc comment at `interpreter.rs:785`. An
embedder who reads `zvm`'s docs and does the obvious thing hands the game
half-size pictures and gets a self-consistent screen the player never sees —
the exact failure shape `CLAUDE.md`'s refactoring policy catalogues.

**Resolved (SQ-1396, then SQ-1402, 2026-09-08).** SQ-1396 moved the scaling rule into `zvm` by having `BootConfig` multiply picture dimensions by the resolved art scale. SQ-1402 turned that into a private `ScaledResources` adapter that `BootConfig` wraps around any `Resources` implementation at query time, so the host need not scale; the adapter applies the rule transparently.

**`Output: Any` forces `'static`.** `crates/zvm/src/io.rs:36` requires `Any` on
the sink so callers can downcast, which means a GUI sink that borrows a frame
buffer cannot be used at all, and every host round-trips its own state through
`as_any_mut` downcasts. `zvm-cli` does it at `main.rs:1710`. The trait's own
header says the requirement exists "so callers can downcast to concrete types
(e.g., to read `BufferOutput::buf` in tests)" — a test convenience paid for by
every embedder. Worth revisiting; the fix is not obvious and this is not urgent.

**Resolved in part (SQ-1402, 2026-09-08).** `Output: Any` was kept by user decision. Module docs on `io.rs` now explain the bound, that it forces `'static`, and document the `Rc<RefCell<_>>` shared-buffer pattern in a compiled doctest so a host can work around the constraint without guessing.

**The grammar seam is now closed too (SQ-1040).** `zvm::grammar` reads the
story's syntax tables the way `dictionary.rs` reads its words: `Grammar::load`
returns a self-contained snapshot — no `&Memory` needed afterwards, so it caches
beside a session or crosses a thread — answering whether a word is a verb, what
sentence shapes that verb accepts, which prepositions it expects, and what parts
of speech the dictionary marks any word with. Five table formats are covered
(Infocom's fixed and variable ZIL forms, Infocom's Version 6 form, and Inform's
GV1 and GV2), from the Inform Technical Manual §§8.5–8.6 and ztools'
`showverb.c`. This is API an embedder wants and could not previously build: the
dictionary is a flat list with no parts of speech, so before this a host could
tell a player a word was unknown and nothing more.

Two things about it are deliberate and worth keeping if it grows. It **refuses
rather than guesses** — `GrammarError` distinguishes "this story has no grammar"
(Journey) from five ways the bytes failed to describe a table — because a
wrong-but-well-formed grammar is indistinguishable from a right one to every
consumer downstream. And every public type in it is already
`#[non_exhaustive]`, which is item 3 below applied to a module while that is
still free.

Two stories in the local corpus are refused: `frankenfingers_260330.z5` and
`ImpossibleStairs.z8` begin static memory with something other than the
verb-pointer table. That was recorded here as "a limitation of the format
assumption both tools share", and SQ-1102 falsified the framing while reading
Inform's source for the Glulx side. Inform 6 writes `p[14]/p[15] =
grammar_table_at` (`tables.c`), so on the Z-machine the grammar table **is**
where this reader looks — the assumption is Inform's own layout, not a guess —
and Inform stamps its version as `6.NN` at header bytes `$3C..$3F`, where these
two files hold `1a01` and `0m03` instead. They are not Inform 6 output at all,
which is why `infodump` declines them too.

**SQ-1101 closed that: both were compiled by Dialog, and Dialog emits no grammar
table of any shape.** The files say so themselves — `Dia` sits in header bytes
`$39..$3B`, `1a01`/`0m03` in `$3C..$3F` is the compiler's own version with its
slash removed, and byte `$38` is `*` for a `-dev` build, which is why
frankenfingers' banner reads `Dialog compiler version 1a/01-dev` and
ImpossibleStairs' reads `0m/03`. `dialogc`'s `src/backend_z.c` writes that
signature unconditionally, and settles the substantive question alongside it: the
string "grammar" does not occur anywhere in the compiler's sources. Dialog's
parser is library code — `(understand $ as $)` querying a `(grammar entry $ $ $)`
predicate defined in `stdlib.dg` — compiled to the same predicate representation
as any other rule, with no Z-machine table to point at. Static memory begins with
the optimised alphabet table (when the story uses one), then wordmaps, then data
tables, then the dictionary, which is what the "address/length pairs" at
`$38ee`/`$4710` actually are.

So `zvm::grammar` now answers **`Absent`** for a Dialog story rather than
`BadVerbTable`, tested by `is_dialog` on the signature and *before* any shape
check. That is not only a truer refusal, it forecloses the one failure this
module exists to prevent: these two files happen to fail the shape checks, and
the next Dialog story's wordmaps need not. A Dialog story now takes the same
already-pinned road as Journey — the command panel keeps its generic column and
labels it, the vocabulary offer stays silent — pinned in
`crates/zvm/tests/dialog_grammar.rs` and
`crates/app/tests/suites/dialog_story_degradation.rs`. The corpus-wide census in
the first of those is the durable part: every Z-machine story on disk is
Infocom's (no stamp), Inform's (`6.NN`), or Dialog's (`Dia`), and the case fails
if a fourth producer ever turns up.

**The Glulx half now exists (SQ-1102).** `gvm::grammar` answers the same
questions about the modern corpus, and the two readers deliberately share no
code: the Z-machine's table address is header-named while a Glulx image records
it nowhere, verb numbers count down from `$FF` against `$FFFF`, line headers are
2 bytes against 3, tokens 1+2 against 1+4, and this reader carries five table
formats against Glulx's one. A trait over "read a byte at an address" would
abstract a handful of lines out of several hundred while making two
zero-dependency crates share a vocabulary. What they *do* share is the shape of
the **answer** — `Token`, `NounKind`, `Slot`, `SyntaxLine`, `Verb`, `WordRoles`
— and **SQ-1103 lifted those into `grammar-model`**, one small dependency-free
workspace crate both readers produce and re-export, before SQ-1041 could harden
against either spelling. What stayed behind is what is about a FORMAT rather
than an answer: `GrammarFormat` (five table shapes here, one there),
`gvm::grammar::Tables`/`locate` (addresses this reader gets from a header and
that one has to derive), and each crate's own `GrammarError`. The join also
settled the one asymmetry a consumer would have hit: `Grammar::words` now
enumerates the whole dictionary on both engines, and the per-line accessor that
used to squat on that name here is `SyntaxLine::literals`.

## 8. What machine knowledge is still in the wrong crate

SQ-1013 is done, and the answer to "what else is shaped like it" is: not much of
the *table* kind. `crates/app/src/interpreter.rs` is now a forwarding layer, and
its remaining public constants are all `pub use zvm::interpreter::{…}`.

What is still in `app` and is a fact about the Z-machine rather than about our
renderer is procedural, not tabular, and §§1–5 have already named it: the boot
ordering, the `Restart` semantics, the picture/fill interleave, the unit-space
picture-dimension rule, and the screen-snapshot format. Those are the SQ-1013
argument applied to behaviour instead of to constants, and they are worth more
than another column would be.

## 9. The docs are excellent and largely invisible

`zvm`'s comments are better than most published crates'. The problem is where
they are pointed and how they are spelled.

**Fifteen module headers are `//` rather than `//!`**, so rustdoc does not render
them at all: `screen.rs`, `cpu/exec.rs`, `memory.rs`, `io.rs`, `quetzal.rs`,
`location.rs`, `objects.rs`, `dictionary.rs`, `header.rs`, `error.rs`,
`cpu/decode.rs`, `cpu/state.rs`, and all three of `text/`. That is **137 lines of
module-level orientation prose that a stranger opening docs.rs cannot see**,
including every overview of the crate's two largest and least self-explanatory
modules. Ten other modules already use `//!`, so this is inconsistency rather
than policy. One character per line, no API risk, and it is the highest
value-per-unit-effort item in this document.

**`lib.rs` has no crate-level `//!` docs at all.** docs.rs would show a bare list
of sixteen modules with no statement of what `zvm` is, which Z-machine versions
it covers, that it takes no dependencies, or how to run a story. §1's sketch is
roughly what belongs there.

**The audience is us.** "lanthorn" appears in fifteen rendered doc comments
across five files; several docs name types the reader cannot see
(`GameSession::drain_turn` at `exec.rs:327` and `:355`, "the `app` crate's
`CaptureSink`" at `io.rs:46`, `:61`, `:64`); and three name a retired planning
scheme ("Tasks 11+" at `exec.rs:213`, "(Task 9)" at `:279`, "(Plan 1b)" at
`:285`). None of this is *wrong* — the rationale is load-bearing and should stay
— but each should name the role with lanthorn as the worked example rather than
as the definition. `machines.rs:1` opens by describing a CLI flag no embedder
has. A `SQ-0917` reference means nothing to a stranger; the ZMSD section number
beside it means everything, so lead with the spec citation and keep the quest as
provenance.

**`location` is promoted to the crate root and its caveat is not.** `lib.rs:18`
re-exports six items from `location` — the only such promotion in the crate — so
`zvm::current_location` reads like a VM primitive. It is a documented best-effort
heuristic that reads global variable 0 and hopes (`location.rs:8-22`), and that
45-line explanation is one of the `//`-comment blocks rustdoc discards. Either
promotion or the invisible caveat would be survivable; together they are a trap.
Drop the root re-export, keep the module, and make its header `//!`.

**One doc line says the opposite of the truth.** `Machine::new`
(`exec.rs:521-523`) reads "`state.pc` is set to the header's `initial_pc` field
(direct instruction address for v3/4/5/7/8; **v6 is not supported**)". What it
means is that v6 does not start from `initial_pc` — it enters the packed `main`
routine per ZMSD §5.4, which the very next function does correctly. What it says,
on the crate's most-read constructor, is that `zvm` does not run Version 6, which
is both false and the crate's headline capability. `zvm-cli`'s v6 refusal is
careful to disclaim exactly this (`main.rs:519`: "the refusal is the FRONT-END's,
not the library's"); the library's own doc is not.

**Smaller things.** `screen.rs:1013` re-exports `AMIGA_INTERPRETER_NUMBER`
mid-file, giving one constant two public paths for no stated reason.
`amiga_global_colour_pair` (`:986`), `amiga_screen_pair` (`:1038`) and
`machine_screen_pair` (`:1082`) are three near-identical names covering two
different concepts. `cargo doc -p zvm` emits 21 warnings, of which six are the
same unresolved `MORE` link and six are public docs linking to private items —
all cosmetic, all noise a newcomer reads as neglect. And `doctest = false` in
`crates/zvm/Cargo.toml` is currently free because there are no examples; for a
crate meant to be embedded, **a compiled example is the cheapest possible proof
the API is usable**, and the setting should come off the moment one exists.

**Resolved (SQ-1398 / SQ-1399, 2026-09-08).** All three crates gained crate-level `//!` docs and compiled doctests (zvm's builds a v3 story by hand in memory; gvm and scott have minimal text-only backends for their examples). Module headers converted from `//` to `//!` (zvm: 17 modules, gvm: 5, scott: 3), routing all previously-invisible prose into rustdoc. Docs rephrased for a stranger, routing out lanthorn-specific references and planning-scheme names; all references to the Z-machine instead of the implementation now lead with the ZMSD section number. `cargo doc --no-deps` warnings dropped to zero for all three. `doctest = false` removed where examples now exist. Examples in `examples/run_story.rs` for gvm and scott verified independently; zvm example verified on Zork I. **Follow-up (SQ-1406, 2026-09-08):** `run_story` refuses Version 6, so the graphical half is `crates/zvm/examples/v6_host.rs` — it boots a v6 story through `BootConfig`, answers `picture_data` through a std-only `Resources` implementation, drains `take_paint_events()` into a host display list, draws the window table's runs at the `V6Metric` cell, writes a P6 PPM, and proves the `screen_snapshot`/`restore_screen_snapshot` + `restore_paint_log` trio round-trips by restoring and replaying a move. It carries a `#[test]` (`test = true` on its `[[example]]` entry) that skips vacuously without `stories/`. **Note:** The `location` root re-export drop was SQ-1397; `ZColour::True24` was kept by decision (retained as a general exact-24-bit host colour), both already recorded in §10's ledger entry.

**Follow-up (SQ-1408, 2026-09-08): `#![warn(missing_docs)]` turned on in all three crates, every flagged item documented.** zvm: 205 items across 18 files (`screen.rs` 59, `text/input.rs` 20, `cpu/exec.rs` 19, `world.rs` 16, `paint_log.rs` 15, `cpu/disasm_cache.rs` 12, `cpu/disasm.rs` 10, `objects.rs` 9, `cpu/decode.rs` 9, `location.rs` 8, `header.rs` 7, `io.rs` 7, `error.rs` 5, `dictionary.rs` 5, `lib.rs`/`cpu/state.rs`/`interpreter.rs`/`memory.rs` 1 each). gvm: 64 items across 6 files (`glk.rs` 34, `disasm.rs` 12, `world.rs` 12, `header.rs`/`i7map.rs`/`veneer.rs` 2 each). scott: 61 items across 5 files (`database.rs` 34, `vm.rs` 10, `decompile.rs` 9, `loader.rs`/`options.rs` 4 each) — 330 items total. A handful of one-line enum/struct-variants (`gvm::glk::StreamKind`, `gvm::i7map::I7Exit::ThroughDoor`, `scott::vm::RestoreError::NewerVersion`, `zvm::cpu::decode::{Form,OperandCount}`, `zvm::cpu::disasm::{OpRole,Unpack}`, `zvm::cpu::disasm_cache::Unit`, `zvm::cpu::exec::{StepResult::NeedLine,ScreenSnapshotVersion,PaintLogVersion}`, `zvm::paint_log::PaintOp::Clear`, `zvm::screen::StatusRight::{ScoreTurns,Time}`, `zvm::world::ExitDetail::{Conditional,Door}`) were split one-field-per-line — whitespace only, verified field-for-field against the pre-split source — because a `///` comment cannot attach to a field packed onto the same line as its siblings. Also fixed nine pre-existing `cargo doc` warnings the missing-docs sweep incidentally exposed a fresh full rebuild needed to see (`private_intra_doc_links`/`broken_intra_doc_links` in `cpu/boot.rs`, `cpu/decode.rs`, `paint_log.rs` x5, `resources.rs`, `screen_snapshot.rs` x2, `text/input.rs`, `scott/options.rs`) by de-linking references to private items and fixing two unresolved paths — none were introduced by this quest's own doc additions. `cargo doc --no-deps`, `cargo clippy --all-targets -- -D warnings`, `cargo test --doc`, and `cargo nextest run` are all clean for all three crates; `cargo check --workspace --all-targets` is clean. Candidates flagged as possibly-leaked internals (documented, not made private): `zvm::objects::{set_parent,set_sibling,set_child}` (tree-desync risk if called outside `insert_obj`/`remove_obj`); the `zvm::cpu::disasm`/`disasm_cache` debugging/tracing surface generally. One accuracy note: `scott::vm::StepResult::Continue` is documented as reserved-but-currently-unreturned — `Vm::step` never constructs it; every real return is `NeedLine` or `Quit`, matching the crate doc's own "every `StepResult` is one of those two."

## 10. Stability: nothing is `#[non_exhaustive]`

Outside `grammar` (SQ-1040) there is not one `#[non_exhaustive]` in `zvm`, `gvm`
or `scott`. Every other public enum is exhaustively matchable, so adding a
variant to any of them is a breaking change for every embedder, forever.

Some of these enums have *demonstrably* grown. `Palette` (`screen.rs:1626`) went
from two variants to five across SQ-0719 and SQ-0956, and will grow again the
next time a machine's interpreter is read. `ZColour` (`:49`) gained `True24`.
`StepResult` (`exec.rs:153`), `ZError` (`error.rs:5`), `LocationMethod`,
`MachineLook`, `CursorShape`, `StatusBand` and the disassembler's `Form` /
`OperandCount` / `Operand` are all in the same position. Host-read-only structs
should follow: `SoundEvent`, `PictureEvent`, `EraseFill`, `MachineProfile`,
`PeriodLook`, `StackTrace` / `TraceFrame`, `Header`, `Token`, `ObjectSnapshot`.

`TextAttrs` (`io.rs:14`) alone must remain unmarked — it is constructed by hosts and `#[non_exhaustive]` would make that impossible. All other host-constructed structs acquire constructors in waves 2–3 and are marked: `SoundEvent`, `PictureEvent`, `EraseFill`, `PeriodLook` in SQ-1397; `Cell`, `ZWindow`, `V6Windows`, `ScreenState`, `UpperWindow`, `V6Text` (`screen.rs:369`) in SQ-1401 (`Cell::new`, `ZWindow::new`, `V6Windows::new`, `ScreenState` with no public constructor, `UpperWindow::from_cells`, `V6Text::at_cell`).

This is the purest breaking-now-free-later item on the list. Applied today it
costs a `..` in a handful of our own match arms. Applied after a release it
cannot be applied at all without a major version.

**Opcode internals are public with no external users.** Grepping `app`,
`zvm-cli` and `mapper` finds no caller for `cpu::state::{read_var, write_var,
peek_stack, poke_stack, call_routine, return_value}` (`state.rs:52-208`), and
`State` and `Frame` (`state.rs:10-33`) expose every field including `pc`,
`frames` and `eval_stack` — which pins the call-stack representation as API
forever. `Machine::do_branch` (`exec.rs:3642`) and `print_text` (`:4100`) are
the same. `do_store` (`:4056`) has external callers but only from two tests.
Making these `pub(crate)` is free now and impossible later.

**`Machine::out` is a `pub` field** (`exec.rs:218`), so a host can swap the
output sink mid-run with no invariant governing when that is safe.

**Resolved in part (SQ-1394, 2026-09-07).** All 25 of `zvm`'s public enums plus `MachineProfile`, `StackTrace`, `TraceFrame`, `Header`, `Token`, and `ObjectSnapshot` are now marked `#[non_exhaustive]`. The following were deliberately NOT marked because a host must construct them by literal (constructors are Wave 2 work): `SoundEvent`, `PictureEvent`, `EraseFill`, `PeriodLook`, `Cell`, `ZWindow`, `V6Windows`, `ScreenState`, `UpperWindow`, `TextAttrs`, and `V6Text`. `cpu::state` free functions (`do_branch`, `do_store`, `print_text`) are `pub(crate)`; `State` and `Frame` fields are now private behind read accessors (`pc()`, `frames()`, `eval_stack()`, `Frame::func_addr()` etc.); `Machine::out` is private behind `output()` and `output_mut()`; the event queues are private behind `pending_*` and `take_pending_*` drains. `pub mod fixtures` is behind a `fixtures` Cargo feature, off by default and verified absent from the built rlib.

**Extended (SQ-1401, 2026-09-08).** `SoundEvent::new`, `PictureEvent::new`, `EraseFill::new`, `PeriodLook::new` acquired constructors in SQ-1397; `Cell`, `ZWindow`, `V6Windows`, `ScreenState`, `UpperWindow`, and `V6Text` acquired constructors in SQ-1401 (`Cell::new`, `ZWindow::new`, `V6Windows::new`, `ScreenState` with no public constructor, `UpperWindow::from_cells`, `V6Text::at_cell`), and all nine are now marked `#[non_exhaustive]`. `TextAttrs` remains unmarked, construction deferred to host literals.

`TextAttrs` is marked too now: `TextAttrs::new` and `#[non_exhaustive]` landed in SQ-1404, so every host-constructed zvm screen struct is covered.

## 11. `gvm` and `scott`, briefly

Both hold the zero-dependency line, and — the useful finding — **neither has any
process-global mutable state**. A grep for `static`, `thread_local!`, `OnceLock`
and the atomics over both crates turns up only `&'static str` annotations and one
instance-level `RefCell`. The palette problem is `zvm`'s alone, which removes the
last argument that it is somehow inherent to a VM core.

**`gvm` has already built the seam `zvm` lacks.** `pub trait GlkBackend`
(`crates/gvm/src/glk.rs:540`) is 34 methods of which exactly two are required
(`as_any` / `as_any_mut`, `:679` and `:681`); everything else defaults to a no-op
or to "the host has no such facility", so a minimal embedder implements two
downcast shims and gets windows, styled text, graphics, ten sound calls, screen
size, glyph metrics and image info for free. `Machine::with_glk(mem, backend)`
(`exec.rs:762`) is the only public constructor, input arrives through an explicit
`StepResult` / `supply_*` / `deliver_*` protocol, and timers are pull-based
(`glk_timer_interval()`, `exec.rs:5461`) so the VM holds no clock. `app`'s
`AppGlk` (`crates/app/src/glk_backend.rs:19`) implements the trait rather than
defining it, so Glk genuinely does not leak upward. That is what a host seam
should look like, and it is one crate away from `zvm`'s pub-fields-and-prose.

`gvm`'s own costs are the mirror image: it ships a 99 KB disassembler and a
stack-trace formatter as public modules in the release library; the `as_any`
boilerplate every third-party backend must write exists so *our* suites can
downcast to `TestBackend` (`glk.rs:539` says so); runtime faults escape only as a
string pushed onto a `pub diagnostics: Vec<String>` field plus
`StepResult::Quit`, so an embedder who never reads that field cannot distinguish
a clean quit from a crashed story; and 18 non-test `unwrap`/`expect` calls in
`glk.rs` (`:1873`–`:2779`) abort the host process rather than following the
crate's own stated fault-and-continue policy. `set_borderless` (`exec.rs:2880`)
is terminal-chrome policy inside a VM, and `seed_ever_executed` /
`clear_executed_pcs` exist for lanthorn's debug panel.

**`scott` is the tidiest of the three and the least documented.** Every one of
`Vm`'s ~30 fields is `pub(crate)` with accessors instead, test-only mutators are
properly `#[cfg(test)]`, and there are **zero** non-test `unwrap`/`expect`/
`panic!`/`unreachable!` in the whole crate. Against that: `lib.rs` is eight lines
with no `//!` docs, and two of its four re-exports are globs (`pub use
database::*`, `pub use decompile::*`), which means the public surface is whatever
happens to be `pub` in those files — currently including four unnamespaced
constants at the crate root (`database.rs:48-51`). `pub use vm::Input` is dead:
nothing in the workspace constructs it, since input actually arrives via
`supply_line`. `restore` returns `Result<(), ()>` (`vm.rs:259`), so a host cannot
tell a truncated snapshot from a version mismatch. And `Vm::room_block()`
(`vm.rs:952`) returns a pre-formatted display block — "I'm in a …\n\nObvious
exits: …" — whose doc comment describes lanthorn's top panel; an embedder with a
different layout must string-parse it or rebuild it from `Database`.

Neither crate has `zvm`'s fixture problem, and neither depends on the other. The
one cross-engine leak is at the adapter layer rather than in a VM:
`crates/app/src/glulx_session.rs:27` imports `zvm::location::LocationMethod` and
constructs `zvm::ObjectSnapshot` (`:721`, `:1032`) and `zvm::screen::ZColour`
(`:1044`), so `zvm` is doubling as the workspace's shared vocabulary crate. Which
is the other half of a `zvm` finding: `ZColour::True24` (`screen.rs:53`) is
documented as "used by the Glulx host", **no `zvm` code constructs it**, and its
only producers are `crates/app/src/glk_backend.rs:75` and `:788`. A Z-machine
embedder writes a match arm for a structurally unreachable variant belonging to a
VM this crate does not implement.

**Resolved in part (SQ-1394, SQ-1395, 2026-09-07).** `GError`, `StepResult`, `WinType`, `GlkStyle`, `WriteFault`, `SaveLoadRequest`, `StackTrace`, and `TraceFrame` are now marked `#[non_exhaustive]`. `StepResult::Fault` is a new variant, distinct from `Quit`, backed by a `faulted` flag that persists after `take_fault_trace()` drains; the fault event surface follows the crate's own stated fault-and-continue policy. The 18 non-test `unwrap`/`expect` sites in `glk.rs` (`:1873`–`:2779`) that sat on the documented window-tree invariant have been addressed: 17 became panic-free branches, one `expect` in `build_win_tree` remains and is documented as guarded by an invariant. **Wave 2 extensions (SQ-1396, SQ-1397, SQ-1398, SQ-1399):** `gvm`'s diagnostics, screen_trace, trace_screen, fault_trace, trace_exec, executed_pcs, ever_executed are private behind accessors (SQ-1396). `scott` gained `Vm::room_is_literal()`, `room_exits()`, `items_in_room()`, and rebuilt `room_block()` as a convenience layout (SQ-1397). `Grammar` became a default-on Cargo feature in both zvm and gvm (SQ-1397). All three crates gained crate-level `//!` docs with compiled doctests and examples (SQ-1398, SQ-1399): `gvm` and `scott` examples feature minimal text-only backends. Module headers converted to `//!` (SQ-1398, SQ-1399): gvm: error, exec, grammar, header, memory; scott: database, loader, vm; zvm: 17 modules total.

**Wave 3 extensions (SQ-1402, 2026-09-08):** `gvm` moved the borderless policy from `Machine::set_borderless` onto `GlkBackend` as `fn borderless(&self) -> bool { false }` (the `winmethod_Border`/`NoBorder` bit that `glk_window_open` honours at the library's discretion; the host flag overrides the story's request when true), moving terminal-chrome policy out of the VM layer into the backend seam. `scott` snapshot format gained magic `ScSv` and `u16` version 1, with `RestoreError::BadMagic` and `NewerVersion` variants for malformed buffers.

## 12. The ledger

Ordered by what it costs an embedder, with the fix cost and whether it breaks.
**"Free later" is the column that matters**: those changes are cheap this week
and unaffordable after a release.

| # | change | fix cost | breaking | free now, expensive later | status |
|---|---|---|---|---|---|
| 1 | palette + interpreter version onto `Machine`; delete both statics | medium — 4 in-crate sites, ~278 `app` sites, deletes `app`'s whole lock apparatus | **yes** | **yes** | done, SQ-1393 |
| 2 | clamp `ZWindow::put_prop`; cap `print_table`/`copy_table`; fix `unit_index_at` on an empty cache; ~~privatise `V6Cell`'s fields~~ (done, SQ-1031) | small — six overflow sites collapse to one clamp | partly | partly | done (caps SQ-1395; clamp and unit_index_at landed earlier) |
| 3 | `#[non_exhaustive]` sweep on read-only enums and structs | small | **yes** | **yes** | done, SQ-1394 |
| 4 | privatise opcode internals (`cpu::state`, `State`/`Frame` fields, `do_branch`, `print_text`) and the queue fields | small — no external callers | **yes** | **yes** | done, SQ-1394 |
| 5 | gate `pub mod fixtures` behind `cfg(test)` or a feature | trivial | **yes** | **yes** | done, SQ-1394 |
| 6 | `BootConfig` owning the `init_caps` ordering and the `Restart` answer | medium | no | — | done, SQ-1396 |
| 7 | `take_*` drains, and a merged `take_paint_events()` | small | no | — | done, SQ-1396 |
| 8 | screen-snapshot format in `zvm` | medium | no | — | done, SQ-1401 |
| 9 | module headers `//` → `//!`; crate-level `//!` docs; a compiled example | trivial | no | — | done, SQ-1398, SQ-1399 |
| 10 | doc pass: de-lanthorn, drop the `location` root re-export, decide `True24` | small | partly | partly | partly: `location` re-export dropped and docs de-lanthorned (SQ-1397, SQ-1399); `True24` stays |
| 11 | `FontMetrics`, `trait Resources`, revisit `Output: Any` | large | partly | partly | done, SQ-1402: `Resources` trait; `FontMetrics` closed earlier by `V6Metric` (SQ-1009); `Output: Any` kept by decision, documented |
| 12 | a `ZsciiInput` newtype for `supply_char`, replacing the raw `u8` SQ-1419 had only runtime-checked | small — one new type, ~25 call sites across `zvm`, `app` and `zvm-cli` | **yes** | **yes** | done, SQ-1426 |
| 13 | hostile-input fuzzing: an in-crate xorshift harness (`fuzz_harness.rs`, CI-gated) plus `crates/fuzz`'s coverage-guided cargo-fuzz targets, run by hand | medium — two harnesses, one new detached package | no | — | done, SQ-1407 |
| 14 | Versions 1 and 2: open the header gate and answer every version-dependent path for them, so the crate covers 1–8 as Frotz and Bocfel do | small — one gate, the text codec's shift/abbreviation rules, three header facts | no | — | done, SQ-1422 |

On #14: this was the last thing standing between the crate doc's claim ("every
published Z-machine version, 1 through 8") and the truth — the gate stopped at 3
while `lib.rs` already said 1. What actually differs is the TEXT FORMAT, not the
machine: ZMSD §3.2.2 gives Versions 1 and 2 a shift-lock alphabet the later
versions have no counterpart for, §3.3 leaves Version 2 one abbreviation table and
Version 1 none at all, §3.5.4 gives Version 1 its own A2 row, and §3.7.1 makes the
lock mandatory when ENCODING a dictionary word — the rule that lets Zork I's PDP-10
answer to "pdp10". The object model, the opcode signatures and the packed-address
scale are all Version 3's already (§12, §14, §11.1.6), and the code's `<= 3` /
`>= 4` ranges answered for 1 and 2 without a change. The header loses two fields
(§11.1's "3+" file length and checksum) and gains no interpreter-writable
capability bit (§11.1's Flags 1 table is Version 3 throughout). No redistributable
Version 1 or 2 story exists, so `crates/zvm/tests/v1_v2.rs` is hand-built images
with every expected string computed from the standard's own tables.

On #5: `crates/zvm/src/fixtures.rs:11` is `PathBuf::from(env!("CARGO_MANIFEST_DIR"))`,
unconditionally public, which bakes **the build machine's absolute source path**
into every downstream binary that links `zvm`. It has no users outside `zvm`'s
own tests. `header::tests_support` (`header.rs:84`) is the in-crate precedent for
how to gate it.

**Item 2 is the one to do regardless of any release schedule**, because until it
lands no host can point `zvm` at a story it did not write. Items 1, 3, 4 and 5
get materially more expensive once a release exists and should be done before one
is cut. Items 6–9 are what an embedder feels on day one, and 9 is an afternoon.

## Status and plan, 2026-09-08

**Wave 1 — landed 2026-09-07:**

- **SQ-1393:** Palette and interpreter version are `Machine` fields. `init_header_caps` and `two_colour_card_request` read them from `&self`. `ZColour::true_value` takes the palette by value. Every instance of `app::v6_palette*`, the private mutex, and both cases of `palette_lock_discipline` are deleted.
- **SQ-1394:** All public enums marked `#[non_exhaustive]`; opcode internals and queue fields privatised; `pub mod fixtures` gated behind a feature. `scott` gains named re-exports replacing globs; dead `Input` type deleted; `Vm::restore` returns `Result<(), RestoreError>`.
- **SQ-1395:** Caps and bounds checks landed. `print_table` and `copy_table` check spans upfront and fault at once. `State::MAX_CALL_DEPTH` and `MAX_EVAL_STACK` capped with headroom. `gvm`: 17 of 18 `unwrap`/`expect` sites in `glk.rs` converted to panic-free branches; one `expect` in `build_win_tree` remains, documented as guarded by invariant. `StepResult::Fault` variant added to both `gvm` and `zvm`.

**Landed earlier:** the put_wind_prop clamp (WINDOW_PX_CAP), `DisasmCache::unit_index_at`'s checked subtraction, `V6Cell`'s private fields behind a guarded constructor (SQ-1031), and SQ-1013's MachineProfile facts.

**Wave 2 — landed 2026-09-08 as SQ-1396, SQ-1397, SQ-1398, SQ-1399:**

- **SQ-1396:** `BootConfig` (in `crates/zvm/src/cpu/boot.rs`; `#[non_exhaustive]`, `new()` + `with_*` builders) and `Machine::boot(mem, out, config)` apply the boot sequence in stated order. `GameSession::from_boot_config` replaces the fourteen-argument constructor. `Machine::restart()` answers ZMSD §6.1.3. `PaintEvent { Picture, Erase }` and `Machine::take_paint_events()` merge drains in issue order; `take_pending_pictures` / `take_pending_erase_fills` removed.
- **SQ-1397:** `SoundEvent::new`, `PictureEvent::new`, `EraseFill::new`, `PeriodLook::new` added and marked `#[non_exhaustive]`. `TextAttrs`, `V6Text`, `Cell`, `ZWindow`, `V6Windows`, `ScreenState`, and `UpperWindow` deliberately left unmarked because a host must construct them by literal; constructors are Wave 3 work. `location` root re-export dropped; callers use `zvm::location::…`. `grammar` is a default-on Cargo feature in both zvm and gvm. `scott` gained `Vm::room_is_literal()`, `room_exits()`, `items_in_room()`, rebuilt `room_block()`.
- **SQ-1398:** `gvm` and `scott` gained crate-level `//!` docs with compiled doctests. Examples in `examples/run_story.rs` feature a minimal `GlkBackend` for gvm and a minimal text-only host for scott. Module headers converted to `//!` (gvm: error, exec, grammar, header, memory; scott: database, loader, vm). `doctest = false` removed. `cargo doc --no-deps` warnings gvm 22→0, scott 2→0.
- **SQ-1399:** `zvm` crate-level `//!` docs with a compiled doctest building a minimal v3 story by hand. Example `examples/run_story.rs` verified on Zork I. 17 module headers converted to `//!`. Docs rephrased for a stranger; `cargo doc --no-deps` warnings 40→0. `Machine::new` false "v6 is not supported" line fixed; `location` re-export caveat restored to module-level docs.

**Wave 3 — landed 2026-09-08 as SQ-1401, SQ-1402:**

- **SQ-1401:** Screen snapshot versioned format in `crates/zvm/src/screen_snapshot.rs` with `encode` / `decode` and `Machine::screen_snapshot()` / `restore_screen_snapshot(&[u8])`; carries classic screen, upper-window grid, all eight v6 windows. App: all DTO mirrors deleted, archive.rs lines reduced 3400 → 3053, `CURRENT_FORMAT_VERSION` 8 → 9. Constructors added to host-built structs: `Cell::new`, `ZWindow::new`, `V6Windows::new`, `ScreenState`, `UpperWindow::from_cells`, `V6Text::at_cell`. All nine now marked `#[non_exhaustive]`. `TextAttrs` remains unmarked.
- **SQ-1402:** `crates/zvm/src/resources.rs` holds `pub trait Resources` with demand-driven picture metadata queries; `BootConfig::with_resources(Box<dyn Resources>)` is the door for lazy or streaming hosts. Unit-space picture scaling moved into `zvm` as private `ScaledResources` adapter. `Output: Any` kept by decision; module docs now document the sharing pattern. `gvm`: borderless moved from `Machine::set_borderless` onto `GlkBackend::borderless()`. `scott`: snapshot format gained magic `ScSv` and `u16` version 1.

- **SQ-1406:** `crates/zvm/examples/v6_host.rs`, a headless graphical host — the Version 6 example the text-only `run_story` cannot be. Exercises `BootConfig`'s v6 facts, `Resources`, `take_paint_events`, `V6Metric`/`v6_cell`, `picture_dims` (scaled by `ScaledResources` on the way out), `screen_snapshot`/`restore_screen_snapshot`, `paint_log`/`restore_paint_log` and `save_quetzal`/`restore_quetzal`, rendering to a PPM. Run under the gate via `test = true`; skips vacuously without the gitignored story.
- **SQ-1435/SQ-1436/SQ-1437:** four accessors `v6_host.rs` was re-deriving header offsets for — `Machine::v6_metric()` (cell + pen, `v6_cell()` is now `v6_metric().cell()`), `Machine::v6_screen_px()` (header `$22`/`$24`, `None` below v6), `Machine::default_colours()` (`(bg, fg)`, header `$2C`/`$2D`), and `Machine::art_scale()` (`BootConfig::resolved_art_scale` carried onto the booted `Machine`, a boot fact untouched by `restart()` and absent from `screen_snapshot()`). `v6_host.rs` and three of the machine's own `read_word(0x22)`/`0x24` sites (`restart`, `set_v6_text`, `erase_window -1`) now read through them; the `0x22` clip-bound read in the v6 print path keeps its own special-cased zero handling and was left alone.

**All three waves closed:** the embedding review is complete. All three crates (`zvm`, `gvm`, `scott`) received the full treatment per the product decision: process-global state eliminated, VM cores embeddable and stable, public surfaces hardened, and three working examples. The v6 display list stays host-owned by design (the paint events since the last clear, with the archive's layer semantics); the screen snapshot is now `zvm`'s (SQ-1401). Two follow-ups remain for future work: whether the v6 display list architecture is engine-shaped (revisit now that app reads the snapshot), and whether `TextAttrs` needs a constructor if it grows beyond its current field set.

**SQ-1429 (documentation, 2026-09-08):** `gvm`'s fileref/VFS layer — §11's "seam `zvm` lacks" — was documented as its own contract for a stranger embedding the crate: `docs/internals/gvm-fileref-seam.md`. Covers the in-memory VFS and its lifetime (verified: reset by the Glulx `restart` opcode itself, `0x0122`/spec §2.9, with no `StepResult` signal — distinct from a host's own external rebuild-and-carry-forward restart); the `StepResult`/`Machine`-call table for `NeedFilename`/`SaveRequest`/`RestoreRequest`, including the memory-/resource-stream cases that resolve in-process and never reach the host (SQ-1427, SQ-0595); what a host must persist (the `GVFS` sidecar codec, when to flush it, what silently breaks if it's skipped); and the fileref-sanitizer-to-disk-filename boundary, using the SQ-1416 regression (`851ed241`) as the worked cautionary example — two production bugs from assuming gvm's sanitized name and lanthorn's on-disk stem were the same string. `crates/gvm/examples/run_story.rs` was extended (previously declined all saves) to answer the seam for real — a single-slot VFS-sidecar-plus-`@save` host — so the doc's code sketch is excerpted from compiled, `clippy -D warnings`-clean example code rather than untested prose.

**SQ-1407 (2026-09-08):** Hostile-input fuzzing. SQ-1014's robustness pass found `print_table` and `DisasmCache` bugs with a throwaway fuzzer in minutes; the caps landed (item 2 above, SQ-1395) but no harness did, leaving `object_scan_eof` (`czech.z5`) as the repo's one adversarial regression test. Two halves now exist — see `docs/internals/fuzzing.md` for the full writeup: an in-crate `#[cfg(test)] mod fuzz_harness` in each of `zvm` and `gvm` (a hand-rolled xorshift64 PRNG, zero dependencies, fixed seeds, CI-gated via `cargo nextest`/`cargo test`) driving random and mutated-fixture story images through `step()`, the restore paths, the Glk selector surface (`gvm` only — `Machine::glk_dispatch` made `pub(crate)` for this, still not embedder-facing), and the disassemblers; and a detached `crates/fuzz` package (excluded from the root workspace, own `[workspace]` table — same shape as `crates/gvm/tables-gen`) carrying seven `cargo-fuzz` targets for coverage-guided runs by hand on nightly. No production panics were found during this quest's own run — both harnesses' failures were self-inflicted budget miscalibrations (a random/garbage image can legitimately loop on a `read`/`glk_select` prompt for the whole step cap without ever panicking, since the harness's own random answers never type "quit"), fixed by sizing `MAX_STEPS`/`PER_STORY_BUDGET` to the measured worst-case legitimate run rather than by patching engine code.

**SQ-1424 (Glk 0.7.6, 2026-09-08):** `gvm` reports **Glk 0.7.6** again. SQ-1416 item 1 had dropped `GLK_VERSION` to `0x0000_0705` because the one call 0.7.6 added, `0x00EC glk_image_draw_scaled_ext`, was unimplemented and `gestalt_DrawImageScale` (24) therefore answered 0 — an honest 0.7.5 in place of a claimed 0.7.6, with the reason recorded in the constant's own doc comment: a `wintype_Graphics` draw is a one-shot resolve that the existing `graphics_draw_image` seam could carry unchanged, but `imagerule_WidthRatio` in a `wintype_TextBuffer` window is **standing** — the spec's §"Graphics in Text Buffer Windows" requires the width to stay "relative to the *current* window width", re-resolved whenever the window is resized — which is per-image state wired through relayout, not one delegated call.

That is what this quest built, and the shape is the refactoring policy's: `gvm::glk::ImageRule` carries the rule word with the three arguments it interprets (`width`, `height`, `maxwidth`), so no caller can supply a plausible subset, and it owns the arithmetic — width first, then height (`imagerule_AspectRatio` reads the width just resolved), then `maxwidth` as a proportional reduction of both, all in `u64` with round-half-up division to match the reference library's `std::round`. Two entry points rather than one with a flag: `resolve_in_graphics` (maxwidth ignored, per §"Graphics in Graphics Windows") and `resolve_in_buffer`. The constants are taken verbatim from the 0.7.6 `glk.h` and cross-checked against `gi_dispa.c`; note there is no `imagerule_HeightRatio` — the third height rule is `AspectRatio`, relative to the image width rather than the window — and that `WidthRatio` *is* `WidthMask` and `AspectRatio` *is* `HeightMask`, each rule being a two-bit field whose top value is the ratio.

The standing half is a new backend seam, `GlkBackend::buffer_draw_image_ext`, which hands the host the **rule** rather than a size — deliberately a different door from `graphics_draw_image`, which takes a resolved size and is exactly what a standing rule must not be collapsed into. Its default implementation resolves once and delegates, which is right for a host with no relayout of its own. lanthorn overrides it: `InlineImage` gained a `rule` field that supersedes `scaled`, and `InlineImage::fitted_cells` — the function the transcript wrapper already calls with the live band width on every layout — re-resolves it there. So there is no separate "on resize, re-scale the images" path for a future change to forget to call; a resize is nothing more than the next layout arriving with a different width. The archive persists the rule for the same reason it persists every other recipe: a restore lands in a different pane than the save came from, and a stored pixel size could not recover the proportion.

Verified against Plotkin's own `imagetest.gblorb`, whose `scales` command sweeps the options: with graphics enabled it drives eight `0x00EC` calls carrying rule words `0x0A`, `0x0B`, `0x0E` and `0x0F` — every combination of the two masks — and the pictures that carry a ratio resize with the pane. Falsified by making the buffer path resolve one-shot, which reddens four of the six synthetic cases with the picture frozen at its drawing width.

**SQ-1414 (partial — detection re-established, loaders blocked on licence, 2026-09-09; the TI-99/4A half was unblocked the same day, see the entry below):** the Scott Adams binary dialects (TI-99/4A game images; C64/ZX Spectrum/Atari 8-bit/Apple II memory snapshots, packed and unpacked) are still refused rather than read, and the refusal is still `LoadError::UnsupportedDialect` naming which one. What changed is **where the detector's facts come from**. The signatures had been taken from Gargoyle's `terps/scott/`, whose doc comments cited it file-and-line; that tree is GPL-2+ in its entirety (Gargoyle's own `License.txt`: "ScottFree is copyright (c) Alan Cox … GNU General Public License", and every file under `terps/scott/` declares itself part of ScottFree), where lanthorn is BSD-3-Clause. Every citation was therefore replaced by a **measurement off real game files**, and the resulting description is strictly better than the one it replaced: each signature is simply the first two entries — `AUTO`, `GO` — of the game's own verb dictionary, laid out in fixed-width slots, so the three "uncompressed" signatures are one construction at three field widths (4, 5 and 6 bytes for word lengths 3, 4 and 5), and the "compressed" one is the same table with the NUL pad dropped and the synonym marker moved into the first letter's case (`gO` canonical, `CLIM` a synonym of it) — which is why it is a byte shorter and mixed-case. Measured over twelve TI-99/4A releases and twenty ZX Spectrum snapshots; the Spectrum corpus splits 13 unpacked / 2 packed / 5 not-Scott-games-at-all, and that last five is the part worth having, since a detector that fired on them would blame the wrong format for every failure. Committed fixtures built from the format rules cover CI; `crates/scott/tests/dialect_specimens.rs` re-measures the real corpus when it is present and skips loudly when it is not. **The loaders themselves did not land**: reading these formats needs each dialect's table layout and, for TI-99/4A, its action bytecode, none of which is derivable from the specimens alone — that is now gated on an independently written functional specification (`docs/internals/scott-dialects-spec.md`), and the implementer must be someone who has not read the GPL sources.

**SQ-1414 (partial — TI-99/4A now LOADS; the memory-image dialects still refused, 2026-09-09):** the first of the binary dialects is read rather than refused. `crates/scott/src/ti994a.rs` implements `docs/internals/scott-dialects-spec.md` §3 in full — the signature scan and the baseline rule that absorbs any container prefix, the 34-byte header and its eleven big-endian table pointers, the flat byte arrays and the pointer tables whose entry *i* + 1 is entry *i*'s END, the length-prefixed string chunks joined by an implied single space, the two separate dictionaries of bare unterminated characters, and the tokenised action encoding — and `Database::parse` routes a signature match to it, so ONE entry point answers for both encodings exactly as that document's Appendix A asks. `LoadError::UnsupportedDialect` now means only "a format I cannot read"; the new `LoadError::BadDialectData` means "a format I can read, in a file that is damaged".

Written under the clean-room protocol (`docs/internals/clean-room.md`): the specification and the specimens were the only inputs, no GPL interpreter was read, and every rule in the module cites the section it implements.

**One thing did not fit the model, and it is not a cosmetic difference.** Appendix A states that every dialect decodes to the reference format's shape, "an action table of (verb, noun, five conditions, four commands)". The tokenised script cannot: conditions and commands share one linear opcode stream and interleave freely, operands are inline bytes, and opcode 218 pushes a failure handler that gives the format an if/else. Measured across the twelve specimens, one record of *Savage Island part I* holds **25 conditions and 37 commands**, and 89 records push a handler. So `Database` gained `ti99: Option<Ti99Script>` — per-verb chains of variable-length records — with `Database::actions` left empty for that dialect and `Vm` running the script instead. The §9 runtime differences travel with the database exactly as Appendix A requires: a TI-99/4A `Database` forces `scott_light`, `prehistoric_lamp` and the new `Presentation::Ti994a` on at `Vm::new_full`, whatever the host asked for.

**The oracle, and how strong it is.** §10.1's check — decode the native file, compare against the plain-text conversion of the same game — run over all twelve pairs, and reported per table in `crates/scott/tests/ti994a_specimens.rs`. These are different RELEASES, so whole-`Database` equality was never available and §10.2 says so; the suite therefore pins order-independent overlaps, which is what that section recommends for the titles whose counts already differ. The decisive number: **every verb of eleven of the twelve tokenised releases is also a verb of its `.dat` twin**, and 103 of *Ghost Town*'s 104. A pointer table read with the wrong sentinel rule, the wrong endianness or an off-by-one on "length is the difference between consecutive entries" cannot produce another release's vocabulary by accident. Room text overlaps 62-100%, item text 72-98%, message text 41-66% (the tokenised releases were re-typeset, and the chunk encoding has no way to store the conversion's hard line breaks). `max_carry`, `word_length`, `light_time` and `treasure_room` agree for all twelve; `start_room` differs only for *Savage Island part II*, whose conversion has one extra room, and the treasure count only for *Pyramid of Doom*, whose tokenised release has a fourteenth `*` item — which is precisely why §3.3 calls the header's own treasure byte "present but not authoritative".

**Two places the specification and the specimens disagree, both resolved toward the specimens and both needing a correction upstream.** §3.4 states that this dialect has no in-band value meaning "carried"; byte 255 occurs in five of the twelve, on exactly the items whose `.dat` twins give −1 or 255 — *Mystery Fun House*'s Shoes and Watch among them — so it is normalised to `CARRIED` as the text loader does, and reading it as a room number would start those games with the player's own possessions in a room that does not exist. And §3.1's "all eleven pointers resolve within the file" holds, but the dispatch TABLE's full extent does not: `adv07.fiad` ends two bytes before its last verb's entry, so an entry that does not lie within the file is read as "no records" rather than refusing the game.

**What §3 does not say, and the specimens make load-bearing.** §9.1 names the built-in handling that survives a chain yielding no success as "take, drop and go". In *Adventureland*'s tokenised release the chains for `INVENTORY`, `QUIT`, `STORE` and `SCORE` each consist of a single zero-length record keyed to noun 0 — and §3.7 is explicit that such a record is "still a real record, eligible to match and to run", which having an empty stream means it always fails. Followed literally, as this implementation does, typing `INVENTORY` answers "I can't do that yet." rather than listing anything. Either those four verbs are also built in on this dialect, or the terminator record is not eligible after all; the specification supports neither reading, and this is named rather than guessed, per §11's own instruction.

**SQ-1414 (partial — the Commodore 64 *Mysterious Adventures* now LOAD, 2026-09-09):** the second of the binary dialects is read rather than refused. `crates/scott/src/c64.rs` implements `docs/internals/scott-dialects-spec.md` §4 (locating and the plain encodings), §5.3 (the two per-release repairs), §6 (the series), §8.2 (the Family B vector pictures) and §11's refusals for Brian Howarth's eleven Commodore 64 releases, and `Database::parse` routes a program file carrying one of them to it — so ONE entry point now answers for three encodings.

**This dialect is the easiest member of the memory-image family, not the hardest, and the reason is worth recording.** §7.2 offers three honest options for the Commodore 64 — implement a 6502 emulator and twenty-one cruncher recognisers, support only the releases whose pass count is zero, or demand pre-unpacked images. All eleven of these are zero-pass: nothing is crunched, every table sits in the clear, and the `JMP` plus long zero run at `$4000` that looked like a packer stub is an entry vector and empty workspace. Better still, §4.6's warning that "a memory image contains no directory, no pointer word and no self-describing structure" is a statement about the FORMAT and not about this PROGRAM: the eleven are one interpreter binary with eleven data payloads, and its initialisation code at `$48E6` plants six table addresses into zero page as literal operands, with a seventh — the dictionary — a few bytes later. So the loader reads its addresses out of the file instead of carrying seventeen numbers per release, and needs exactly **two** tabulated facts per title, which is what §4.6 now says to look for.

| title | header shape (§4.5) | verb cells |
|---|---|---|
| The Golden Baton | Mysterious | 79 |
| The Time Machine | Mysterious | 86 |
| Arrow of Death part 1 | Mysterious | 91 |
| Arrow of Death part 2 | Arrow 2 | 80 |
| Escape from Pulsar 7 | early | 146 |
| Circus | Mysterious | 98 |
| Feasibility Experiment | early | 56 |
| The Wizard of Akyrz | Mysterious | 67 |
| Perseus and Andromeda | early | 131 |
| Ten Little Indians | Ten Little Indians (byte-packed) | 64 |
| Waxworks | early | 91 |

The noun-cell count is the remainder of whatever the dictionary span holds, and in all eleven the larger of the two blocks is exactly (word count + 1). Both columns were derived here from the specimens — the shape by trying each of §4.5's four candidates and keeping the one where "action table begins after the header" and "dictionary minus (actions + 1) × 16" agree, the split by searching for the division at which the leading cells match the published conversion's verb column — and neither came from any interpreter's catalogue.

**The oracle is far stronger here than for the TI-99/4A.** For SIX of the eleven — *The Golden Baton*, both *Arrow of Death* parts, *Feasibility Experiment*, *Perseus and Andromeda* and *Waxworks* — the actions, room descriptions and exits, messages, item descriptions and locations and both vocabulary columns come out **identical** to the same tables built from the published `.dat` conversion, and `crates/scott/tests/c64_specimens.rs` asserts that equality per table rather than an overlap. The other five are different releases of the same games, and their measured overlaps are pinned rather than floored, because a floor would either pass everything or fail an honest release: *Escape from Pulsar 7* shares 7 of 45 room texts (the Commodore 64 edition names rooms in two words where the conversion writes sentences) and all 146 of its verb cells — the second number is the one that could not come out right by accident.

**Two §5.3 repairs, and one §5.3 repair that turned out to be unnecessary.** *Escape from Pulsar 7*'s stored action count of 195 is replaced by 190, which the span from the header's end to the dictionary confirms to the byte (3,056 bytes, 191 sixteen-byte records); *The Time Machine*'s item block holds 62 descriptions where its header implies 63, so item 62 gets an empty one rather than a 63rd string read out of the location table. The loader verifies the first by requiring the action table to end exactly on the dictionary's first byte, so a wrong number cannot pass silently. What it does NOT do is §5.3's Mysterious Commodore 64 dictionary repair — "noun cell 0 set to `ANY`, and noun cells 1-6 copied from the first six system messages… these releases do not store direction nouns in the dictionary". Measured on all eleven, they do: `ANY`, `NORT`, `SOUT`, `EAST`, `WEST`, `UP`, `DOWN` are already noun cells 0-6, applying the repair would truncate *The Time Machine*'s stored five-letter `NORTH` and `SOUTH`, and a case in the specimen suite records the measurement.

**§4.2's dictionary reader is wrong for this family, and the sixfold equality is what proves it.** That section describes a stream with three escapes; two of them are wrong here. A `*` synonym marker occupies one of the cell's own bytes rather than an extra one, and *Waxworks* carries dictionary cells that are nothing but spaces, which §4.2's "a space followed by a non-space is dropped and the count rewinds" mis-aligns the whole table on. Read instead as plain (word length + 1)-byte NUL-padded cells — keeping only §4.2's first escape, a NUL where a cell should begin being alignment padding, which fires exactly once in the whole corpus and is what makes *The Golden Baton*'s final cell read `CAST` — every verb and noun cell of the six byte-identical titles comes out exactly right, and under §4.2's reading they do not.

**The pictures decode, and nothing draws them yet.** §6.1's claim that the Commodore 64 releases carry "a separate Commodore 64 bitmap set" is wrong: all eleven use §8.2's Family B vector format, the same as the ZX releases. `decode_family_b_pictures` walks the block and yields one 255x94 indexed bitmap per room, with §8.2's Bresenham rasteriser, its FIFO flood fill (bounded at 1024 points, the "real behavioural fork" that section warns about — measured here to make no difference at this canvas size, because a dropped neighbour is reached again from another direction) and the Commodore 64 palette composed with remap table A. For every one of the eleven the walk yields exactly the room count of images and consumes the file to its final byte, which accounts for every byte of every game file. Rendering them in the app is a separate quest.

**And the series is FIRST-person on this platform**, which §6.4 now says and an existing implementation gets wrong. The 790-byte system-message block at `$4406` is byte-identical across all eleven and reads `I'm in a `, `I am carrying:`, `I'm DEAD!!`; not one of `You are in a`, `You can also see`, `You haven't got it` or `You are carrying` occurs anywhere in any of the eleven files. So `Database::mysterious` forces only §9.2's two lamp options — the countdown and the lamp that is destroyed at zero — and leaves the wording to the host, whose default is already right.

**SQ-1414 (partial — the two host front-ends now OPEN what the loader and the disk walk already read, 2026-09-09):** `crates/scott/src/c64.rs`'s loader and `blorb::medium`'s D64 directory walk (`MountedDisk::contents`, which lists every program file as `(name, bytes)` with its two load-address bytes kept) landed separately and each stopped at its own crate boundary — nothing yet asked `blorb` what was on `MYSTADV1.D64` and handed the answer to `scott`. That is now the one disk seam every front-end already shares for Z-code: `MountedDisk::stories` stays Z-code/Glulx/Blorb-only by design (its own module doc), so a host wanting the ELEVENTH kind of thing a disk can hold scans `contents()` itself and keeps what `scott::looks_like_scott_bytes` accepts — `app::hints::mounted_stories`/`read_story_file` and a new `scott-cli --story <n|name>` (mirroring `zvm-cli`'s flag, matched through the same `cli_host::story_pick`) both do exactly that, so a number or a name resolves identically at the app's picker and at the prompt. `picker::resolve_entries`, the existing multi-story door SQ-0859 built for Infocom compilations, needed no new UI at all: six `LoadedStory::Scott` rows off one D64 are just six more rows. `QUESTPR1.D64`'s `SHULK.DB` — the US-format *Hulk*, a Commodore 64 family this crate does not read — is the negative case: `looks_like_scott_bytes` correctly declines it, so it contributes no row and no crash, on either front-end.

**One latent defect surfaced immediately, because this was the first time it could.** `app::ifid::compute_ifid` fell through to `zvm::ifid::compute_ifid` for ANY non-Glulx bytes with no check that they were shaped like a Z-machine header at all — reading whatever sat at header-shaped offsets `0x02`, `0x12..0x18`, `0x1C..0x1E` of an arbitrary byte string. A Commodore 64 *Mysterious Adventures* program file's driver code is byte-identical across all eleven releases for roughly its first kilobyte, which covers every one of those offsets, so all eleven fabricated the SAME `ZCODE-…` id — and `picker::dedupe_within_a_volume`'s ifid-keyed fold, built for two copies of one Z-code build on a hybrid disc, collapsed six distinct games on `MYSTADV1.D64` into one row before this was even about media enumeration. Fixed the way SQ-0339 fixed the identical shape for Glulx: a Scott Adams database now gets its own stable content hash (`SCOTT-<hash>`), checked with `scott::looks_like_scott_bytes` before ever reaching the Z-machine reader, never the borrowed one.

Saves are keyed per program without a new field: `story_key_for`'s existing zip-entry rule (no `DiskBuild` — Scott bytes carry no Z-machine header — so the key falls to the chosen entry's own basename) already gives `BATON` and `TIME MACHINE` off one D64 two directories, and the eleven CBM names across both disks are all distinct so nothing here can collide by construction. A disk-mounted Scott row's TITLE comes from `scott::c64::RELEASES` (`file_name` → `title`) by the disk's own spelling, not from `app`'s `scott_titles.tsv`, which is keyed on the IF-Archive `.dat` filenames these disks never carry and so never resolved `BATON` to anything.

Tests: `crates/app/tests/suites/c64_mysterious_disks.rs` (six real-fixture cases: disk-order enumeration with no `BOOT` row, the picker's own rows, a full boot to *The Golden Baton*'s opening room, distinct save keys across both disks, and the Hulk-disk refusal) and `crates/scott-cli/tests/c64_mysterious_disks.rs` (four, spawning the real binary: `--story BATON` to the first prompt, both disks' bare menus, and the Hulk disk's named refusal) — all `stories/`-gated and skip vacuously without the fixture. `crates/app/src/ifid.rs` gained a unit case reproducing the collision directly (two different databases sharing an identical prefix at the borrowed offsets must not collide).

**Not reached through this seam**: the Atari 8-bit and Apple II DOS 3.3 media SQ-1458 already mounts and lists still have no Scott loader behind them (this crate reads only the ScottFree text format, the TI-99/4A tokenised releases and the Commodore 64 Mysterious Adventures, per this file's own module doc) — those disks still list what is on them and say none of it is a game yet, exactly as before.

**SQ-1463 (2026-09-09): rendering them in the app was the separate quest above, and it is done.** `ScottSession::new_with_options` (`crates/app/src/scott_session.rs`) recognises a Mysterious C64 program file the same way `Database::parse` already does — `scott::c64::looks_like_c64_mysterious` over `prg_image`'s stripped load address — and decodes its Family B pictures once at construction, alongside the database, rather than re-deriving anything per turn. `crate::graphics::PictSource` gained a third source (`PictSource::from_scott_c64`, beside the Blorb and native-Infocom-archive ones it already carried): `PictSource::get` maps a picture NUMBER (`scott::Vm::current_picture()`, "by convention, picture number == room number") to the decoder's own picture INDEX with one `checked_sub(1)`, since `decode_family_b_pictures`'s §8.6 identity is `pictures[n - 1]` for room *n*. `scott_c64_image` turns one decoded `Picture` into the same `DynamicImage` shape the Blorb and native-Infocom paths already hand `WinNode::Graphics` — no protocol branch anywhere in this path, so kitty, sixel and half-blocks all draw it exactly as they draw a `.blb` game's room picture, through the SAME `PICTURE_ROWS` band `scott_session.rs` has reserved above the room panel since SQ-0402. `crates/app/tests/suites/scott_c64_native_pictures.rs` is the side-by-side oracle promised above: *The Golden Baton*'s `BATON.prg` (native, no Blorb) against `golden_baton.blb` (the same title, Blorb-carried at a different native resolution — 255×94 against the Blorb's 256×96) — same rows reserved, same `GraphicsWindow` shape, and an identical placed rect under both a half-blocks and a kitty `Picker`, plus a pixel-for-pixel oracle check against `decode_family_b_pictures` run independently that a reverted `resnum - 1` offset (verified by hand) fails on exactly.

**SQ-1414 / SQ-1464 (the US S.A.G.A. binary database now LOADS, 2026-09-09):** the third binary dialect is read rather than refused, and it is the one §11 used to refuse by name. `crates/scott/src/saga_us.rs` implements `docs/internals/scott-dialects-spec.md` §12 in full — §12.2's version/adventure scan over the front matter, §12.3's three container offsets, §12.4's fifteen-word header in §4.5's US field order and its twenty-NINE-byte consume rule, §12.5's character-counted dictionary of nouns-then-verbs found by scanning for `ANY`, §12.6's length-prefixed strings and the auto-noun back-reference byte, §12.7's three pointer tables and the base they recover, §12.8's column-major action table, §12.9's direction-major connections, §12.11's runtime facts and §12.14's refusals — and `Database::parse` routes a recognised array to it, so ONE entry point now answers for four encodings. That is Adventures 1-6 and 13 on the Atari 8-bit and the Apple II, and Questprobe 1 (*The Hulk*) on the Commodore 64: fifteen databases, every one checked against the published text conversion of the same game.

**Almost nothing is shared with §4, which is why it is its own module.** No dictionary signature, no per-release address catalogue, a different string encoding, a transposed action table, transposed connections and its own pointer tables. It is also the first dialect here with **no per-release table at all**: §12.12's fifteen facts are every one of them recovered from the bytes, so the specimen suite's table is an identification checksum rather than an input. The only per-platform knowledge is the three array offsets, and each of those is chosen so the header lands at 0x38.

**The oracle is the strongest of the three.** All fifteen reproduce their twin's eleven header numbers exactly; item start locations and room connections match exactly on all fourteen undamaged specimens; *Voodoo Castle* (190 records) and *The Count* (220) are **identical rule for rule on both platforms**, which is the case §12.13 says to bring up first because "a column-major reader that gets either of them wrong is wrong about the format, not about the release". Everything else differs by a handful of records — 4 of 170 on *Adventureland*, 10 of 262 on the *Hulk* — always in the same shape, the graphic edition carrying one extra command the text conversion does not.

**Two things the specification has slightly wrong, both recorded for its author.** §12.5's leading-NUL escape has to fire **at most once per cell**; letting it repeat swallows a run of pad bytes that is really an empty dictionary entry, and on *Pirate Adventure* the whole verb column then runs one slot early from index 58 and reads room text as vocabulary. And §12.5's "the Apple II *Claymorgue* differs from its twin in six verbs and two nouns" is measured here as **nineteen** verbs and two nouns — six mid-table synonyms the v122 release spells `.`, plus eleven trailing entries (`LIGHT` and its synonyms) it does not carry at all. Both are release differences, not decoding ones: the Atari v125 release matches its twin exactly.

**And one thing §12 does not mention: these releases spell draw-picture as command 90.** Counted over the action tables, opcode **90** occurs in six of the eight titles measured (11 times in the *Hulk*, 3 in *Pirate*, 3 in *Strange Odyssey*, 2 in *Adventureland*, 2 in *Mission Impossible*) and never once in any of their text conversions — it is exactly the "one extra command" §12.8 says the graphic release carries. Opcode 89, which this crate implements as ScottFree's draw-picture, also occurs. Nothing was changed on the strength of that: lanthorn draws no Scott Adams pictures yet, so 90 stays the `_ => {}` no-op it has always been, and the question of what it means belongs to the specification rather than to a guess.

**The damaged specimen is refused, not decoded.** The Atari *Mission Impossible* side A carries about fifty corrupt bytes in its room-description block (§12.13), so §12.7's pointer tables do not resolve onto the strings a sequential read produced, and §12.14 says to stop: it comes back as `LoadError::BadDialectData` — "a format I can read, in a file that is damaged" — while the clean Apple II `A3.DAT` of the same title loads. Questprobe 3, *Fantastic Four*, stays unidentified on both its releases.

**The container half is deliberately not here.** `scott` reads no disk images, so a host mounts the ATR, `.dsk` or `.d64` and hands over what it found; `SagaPlatform` carries the one offset that turns those bytes into the array. Wiring that into lanthorn and `scott-cli` is a separate quest. `Database::saga_us` is the release identity the loader could establish — §12.2's pair plus the platform — and unlike `Database::mysterious` it forces **no** options, because §12.11 says these Adventure International releases take the host's lamp settings.

**SQ-1418 (2026-09-08):** the one architectural item the `gvm` reference audit left open — filter-iosys output and string-embedded routines ran as a NESTED interpreter loop (`emit` → `run_call_to_return` → `step_once` until the callee's frame returned) rather than on the Glulx stack, and everything that costs follows from the same cause. A native loop costs native stack, so it needed a cap (`FILTER_MAX_DEPTH = 32`) and past it characters were **silently dropped** — a filter a hundred deep printed thirty-three and reported success. A native loop waits on a frame pointer, so a legal `@throw` past that frame never satisfied it and turned into "function called within a string halted the machine". And a native loop is not on the Glulx stack, so a `@save` taken mid-print serialised no resume state: the blob restored into a filter call that returned to the instruction after the stream opcode, silently dropping the rest of the string — in `gvm` and in any other interpreter that read it.

The engine now does what the spec describes and glulxe implements. `stream_string` / `stream_num` / `stream_char` push a call stub — DestType `0x10` (resume a compressed string at a bit position), `0x11` (resume function code after the string), `0x12` (resume a decimal at a digit position), `0x13`/`0x14` (resume an E0/E2 string), all spec §1.3.2 — set the PC to the resume position, enter the callee and **return to the interpreter loop**; `pop_save_stub_and_store` recognises 10-14 and re-enters the print, and `pop_callstub_string` unwinds the `0x11` terminator when it ends. An embedded object reference (node types `0x08`-`0x0B`) goes through the stack in *every* I/O system, not only the filter one, which is what makes a throw past a print an ordinary unwind. The machine therefore keeps **no printing state of its own**: it is all stack, so it saves, restores, undoes and unwinds like any other call, and the `Stks` chunk carries the stubs for free.

Deleted: `emit`, `emit_latin1`, `emit_uni`, `print_object`, `decode_compressed`, the `filter_depth` field and its cap, and the `pending_fault` field with its `step()` drain (printing has an error channel again, so a faulting filter propagates directly). One native nest remains and is named: `emit_capture`, the compressed-string decoder behind `glk_put_string`, which owes a Glk dispatch call a finished `String` and so cannot suspend into the loop. It is bounded by `CAPTURE_MAX_DEPTH`, never involves the filter (capture bypasses the iosys entirely), and `run_call_to_return` now refuses an unwind past its own frame instead of spinning on it.

Cross-checked against glulxe 0.6.1 (MIT, Andrew Plotkin) at `56ab8743bab565de307bd892c555d8d8897ed517` built against cheapglk 1.0.7: two synthetic `.ulx` images — a 201-deep filter recursion, and a `@saveundo`/`@restoreundo` taken inside a filter mid-`streamnum` — produce byte-identical output under both interpreters. Note the second had to be `@saveundo` rather than `@save`: glulxe's `serial.c` refuses `@save`/`@restore` outright when the I/O system is not Glk ("Streams are only available in Glk I/O system", an implementation limit of its stream writer rather than a spec rule), where `gvm` has no such restriction, so an in-filter `@save` is one thing the reference cannot be asked about. glulxercise stays 68/68 and the bench moved ~1.8% (26.9 vs 27.4 Mopcodes/s best-of-three), which is the honest cost of routing embedded-node calls through the stack in Glk mode too.

**SQ-1467 (2026-09-09): the Family B pictures are a display list, and `scott` now says so in its API.** §8.2's artwork is lines and flood fills over a palette, not a bitmap, and nothing in the data fixes a canvas size — so `crates/scott/src/c64.rs` separates the two halves an embedder was previously handed fused. `decode_family_b_lists` / `decode_family_b_picture_lists` return `PictureList` (a background, the derived line colour, and `PictureOp::Line`/`Fill` in stream order, with the opcode stream's current point and image-wide line colour already resolved away); `PictureList::rasterise` draws §8.2's own 255 x 94 canvas, byte for byte what the crate always produced, and `decode_family_b_block` / `decode_family_b_pictures` are now that pair spelled once, unchanged in shape and in output. `PictureList::rasterise_at(scale)` is the new door: the same drawing on a canvas an integer multiple larger, `Picture::scale` carrying which.

**The rule it settled on is worth recording, because the obvious one is wrong.** Re-running §8.2's flood fills on the large canvas leaks: a fill sealed at 1x by two lines a pixel apart, or by two whose 1x rounding put them in the same row, finds a half-pixel seam once the lines are drawn where the geometry actually puts them and repaints the region beyond it. Measured over the eleven Commodore 64 releases, **25 images of 516 leaked at one scale or another, the worst flooding 6,118 of 23,970 native pixels** — a quarter of the canvas the wrong colour. Nor is it a pen-width problem: the 1x pixel a shallow line lands in is up to a whole pixel from where the line truly runs, so nothing under a two-pixel stroke covers it, and a two-pixel stroke is not this artwork's line. Sub-pixel staircases and 1x region topology are not both available; the topology is the one the player saw. So the native raster owns the REGIONS and the supersample redraws only the LINES — every device pixel is the line colour where the scaled line covers it, its native pixel's 1x colour where that pixel was not line, and, where the finer line has vacated part of a 1x line pixel, the colour it reaches first breadth-first without crossing the scaled line. A leak is then unrepresentable rather than merely unlikely, and §8.2's flood — FIFO order, background-only boundary test, 1024-point queue bound — runs exactly once per picture on the canvas it was measured on. `every_picture_keeps_its_regions_at_every_supersample` (`crates/scott/tests/c64_specimens.rs`) is the corpus proof: every image of all eleven titles at scales 2, 3 and 4, majority-voted back down to the native grid, with zero disagreements the ink does not explain.

The app side is `PictSource::from_scott_c64(lists, band_px_high)`, which keeps the display lists (a few tens of kilobytes for a whole game) and draws a room only when it is first shown, at `band rows x cell height / 94` rounded up and capped at 4 — the band's own magnification, so the renderer's resample is close to 1:1 and in the minifying direction. Backend-neutral by construction: kitty, sixel and half-blocks receive the same `DynamicImage` and fit it the same way, and half-blocks gains only a better-averaged downsample, which is a reason to keep one path rather than to fork one.

**SQ-1470 (2026-09-09): the container half.** `scott::saga_us` reads no disk image (see its own entry above); this is that host wiring, and it landed in `app::hints` and `scott-cli`, not in `scott` — the loader gained one addition, `SagaUs::display_title`, a `&'static str` per-release table matching §12.12's own, because two different hosts both needed the same "which title, on which platform" answer and a hand-maintained duplicate is exactly the invariant CLAUDE.md's refactoring policy warns about.

The Atari 8-bit needed a real extension: its database is not a directory entry (`blorb::atr::IMAGE_ENTRY` reads the whole image, a door that landed with the ATR reader itself but that nothing used until now), so `app::hints::scott_disk_stories` and `scott-cli::story_candidates` both probe it in addition to the ordinary directory scan when the mounted format is `AtariDos2`. Apple II and the Commodore 64 needed none — `A1.DAT`…`A6.DAT`/`DATABASE` and `SHULK.DB` are ordinary catalogue files the existing scan already lists.

**Two defects surfaced only because this family has candidates neither host's existing code had ever produced.** First, the cheap `looks_like_scott_bytes` sniff a disk-sourced candidate was filtered through never actually called `Database::parse` — every prior disk-Scott source (`Mysterious Adventures`) happened to always parse cleanly, so nothing had ever exercised the gap, and the Atari *Mission Impossible* side A (damaged, `BadDialectData`) walked straight through it and would have shown up as a fabricated row. `scott_disk_stories` now filters on `Database::parse(bytes).is_ok()` directly. Second, `app::picker::resolve_entries` and `hints::read_story_file`'s un-named ("bare tiebreak") path both went through `MountedDisk::story()`, which is Z-code/Glulx/Blorb-only **by design** (a Scott database off a disk is this crate's own business, never that door's) — every existing Scott-disk source held at least two candidates and so never took that path; every US S.A.G.A. side holds exactly one, and `MountedDisk::story()` answering `None` for it meant a bare `lanthorn "some.atr"` (no `--story`, no picker) could not open it at all. `read_story_file` now falls back to `scott_disk_stories`'s own single-candidate answer when the format tiebreak finds nothing, which is also what makes `resolve_entries`' existing single-story branch (`resolve_entry` → the same fallback) correct without needing its own change — ambiguous (zero, or more than one Scott candidate with no Z-code alternative) still refuses rather than guessing.

**A save key needed the same content identity the title does.** `blorb::atr::IMAGE_ENTRY` is the literal name `IMAGE` on every one of the seven Atari sides, and the Apple II boot disks spell two DIFFERENT titles `DATABASE` (*The Count*, *Claymorgue Castle*) — both in one directory — so a disk-sourced Scott entry's save key, which `cli_host::story_key_for` builds from the container's own entry name (Scott bytes carry no Z-machine header for `DiskBuild`), would collide under either literal name whenever a host actually names the entry (`scott-cli` always does, choosing one candidate explicitly even when there is only one). `saga_us_keyed_name` (duplicated, deliberately, in `app::hints` and `scott-cli::main` — the same small shape `scott_c64_title` already is in both) appends the (version, adventure, platform) triple to the real container name before it reaches the key, a no-op for every other Scott source whose names were already unique. `app`'s own picker path sidesteps the question differently for its single-candidate rows: `disk_entry` stays `None` there (the `read_story_file` fallback above never names the entry either), so the key falls to the container's own FILENAME, which is already distinct for every specimen in the corpus — the `saga_us_keyed_name` rename matters once a host actually enumerates and names more than one SAGA candidate off one disk, which no specimen here does, but the corpus not exercising a case is not the same as the case being unreachable.
