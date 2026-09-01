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

`crates/zvm/src/screen.rs:1787` and `:1793` hold two process-wide atomics —
`ACTIVE_PALETTE` and `INTERPRETER_VERSION` — written through `set_palette` and
`set_interpreter_version`. This is the crate's worst embeddability defect and the
one to fix first.

The justification is stated plainly in the doc at `screen.rs:1778`: "the palette
is a property of *the machine lanthorn is pretending to be*, and there is exactly
one of those per run." That premise is true of lanthorn and is not a fact about
the Z-machine. It is false for a GUI with two windows open, for a server running
a session per player, for a test harness comparing an Amiga press against an IBM
one, and for any host that puts a `Machine` on a thread. Two `Machine`s in one
process cannot have different palettes today, and there is no API by which they
could.

Three things make this rank first rather than third.

**The project already pays for it, in a currency it can measure.** `CLAUDE.md`
devotes its longest section to the consequences: four consecutive red builds on
main (SQ-0904, SQ-0958, SQ-0959, SQ-0987) and a three-layer apparatus built to
contain them — a mutex kept *private to `app`* so a test suite physically cannot
take it raw, an `app::v6_set_palette` that panics unless the calling thread holds
a guard, and a source-scanning test, `palette_lock_discipline`, that fails any
file under `tests/suites/` naming `zvm::screen::set_palette` directly. The
apparatus is not small: `crates/app` holds **234 `v6_palette` / `v6_palette_at_boot`
guard acquisitions and 44 `v6_set_palette` calls**, none of which would exist if
the palette were a field on the session being rendered. **An
embedder inherits the hazard and none of the apparatus**, and will not know to
build it, because the reason it exists is documented in our repo instructions
rather than in the crate.

**`zvm`'s own docs argue against it.** `screen.rs:2222`, on `V6Cell`, reads: "it
is emphatically not process-global — see `zvm::screen::set_palette` for what that
costs." The cell was moved onto `Machine` for exactly this reason. The palette is
the same kind of fact, moved by the same argument, and has not moved.

**The blast radius is four call sites.** Grepping non-test readers of both
statics inside the crate finds exactly this:

| site | what reads the global |
|---|---|
| `screen.rs:1579` | `init_header_caps` writing the `$1F` interpreter-version byte |
| `screen.rs:1734` | `two_colour_card_request`, reached from `exec.rs:1453` — inside `Machine`, so `&self` is in scope |
| `screen.rs:1905` | `standard_true_colour`, the only reader of `palette()` |
| `screen.rs:70,72` | `ZColour::true_value`, which calls the above, reached from `exec.rs:3096-3097` (window properties 16/17) |

Outside the crate there is one non-test reader, `crates/app/src/colors.rs:65`,
which is a host renderer and should be asking the session it is rendering rather
than the process. The plumbing for the fix already exists: `true_colour_in(p, n)`
at `screen.rs:1918` takes the palette by value and was written precisely so
`crates/zvm/src/machines.rs` could print every machine's table side by side
without writing global state — the doc at `:1913` explains that a
borrow-and-hand-back "is atomic to nobody". That argument is correct and it
applies to sessions as much as to tables.

**Assessment.** Add `Machine::palette` and `Machine::interpreter_version` fields
with setters. `init_header_caps` and `two_colour_card_request` both already have a
`Machine` in scope at their call sites. The one genuinely awkward step is
`ZColour::true_value`, a method on a `Copy` value type that would need the palette
as a second parameter beside the `interpreter_default` it already takes — which is
mechanical, and is the correct shape anyway, since resolving a colour number
without saying which machine's table you mean is the bug. The whole change deletes
`app::v6_palette`, `app::v6_set_palette`, `app::v6_palette_at_boot`, the private
mutex, and both cases of `palette_lock_discipline`. It is breaking, it is cheap,
and it is the single item on this list whose cost grows fastest with delay.

`set_interpreter_version`'s own doc concedes the point in advance: it says the
byte "cannot be a session parameter because `GameSession`'s constructor runs the
story to its first input, so the header has to be right before construction
returns". `GameSession` is `app`'s type. That is a constraint of lanthorn's
constructor, stated inside `zvm` as though it were a constraint of the Z-machine,
and a `BootConfig` (§3) dissolves it.

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

**`Output: Any` forces `'static`.** `crates/zvm/src/io.rs:36` requires `Any` on
the sink so callers can downcast, which means a GUI sink that borrows a frame
buffer cannot be used at all, and every host round-trips its own state through
`as_any_mut` downcasts. `zvm-cli` does it at `main.rs:1710`. The trait's own
header says the requirement exists "so callers can downcast to concrete types
(e.g., to read `BufferOutput::buf` in tests)" — a test convenience paid for by
every embedder. Worth revisiting; the fix is not obvious and this is not urgent.

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
already-pinned road as Journey — the command band keeps its generic column and
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

Two must **not** be marked: `TextAttrs` (`io.rs:14`) and `V6Text`
(`screen.rs:369`) are constructed by hosts, and `#[non_exhaustive]` would make
that impossible. If they need to grow, give them constructors first.

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

## 12. The ledger

Ordered by what it costs an embedder, with the fix cost and whether it breaks.
**"Free later" is the column that matters**: those changes are cheap this week
and unaffordable after a release.

| # | change | fix cost | breaking | free now, expensive later |
|---|---|---|---|---|
| 1 | palette + interpreter version onto `Machine`; delete both statics | medium — 4 in-crate sites, ~278 `app` sites, deletes `app`'s whole lock apparatus | **yes** | **yes** |
| 2 | clamp `ZWindow::put_prop`; cap `print_table`/`copy_table`; fix `unit_index_at` on an empty cache; ~~privatise `V6Cell`'s fields~~ (done, SQ-1031) | small — six overflow sites collapse to one clamp | partly | partly |
| 3 | `#[non_exhaustive]` sweep on read-only enums and structs | small | **yes** | **yes** |
| 4 | privatise opcode internals (`cpu::state`, `State`/`Frame` fields, `do_branch`, `print_text`) and the queue fields | small — no external callers | **yes** | **yes** |
| 5 | gate `pub mod fixtures` behind `cfg(test)` or a feature | trivial | **yes** | **yes** |
| 6 | `BootConfig` owning the `init_caps` ordering and the `Restart` answer | medium | no | — |
| 7 | `take_*` drains, and a merged `take_paint_events()` | small | no | — |
| 8 | screen-snapshot format in `zvm` | medium | no | — |
| 9 | module headers `//` → `//!`; crate-level `//!` docs; a compiled example | trivial | no | — |
| 10 | doc pass: de-lanthorn, drop the `location` root re-export, decide `True24` | small | partly | partly |
| 11 | `FontMetrics`, `trait Resources`, revisit `Output: Any` | large | partly | partly |

On #5: `crates/zvm/src/fixtures.rs:11` is `PathBuf::from(env!("CARGO_MANIFEST_DIR"))`,
unconditionally public, which bakes **the build machine's absolute source path**
into every downstream binary that links `zvm`. It has no users outside `zvm`'s
own tests. `header::tests_support` (`header.rs:84`) is the in-crate precedent for
how to gate it.

**Item 2 is the one to do regardless of any release schedule**, because until it
lands no host can point `zvm` at a story it did not write. Items 1, 3, 4 and 5
get materially more expensive once a release exists and should be done before one
is cut. Items 6–9 are what an embedder feels on day one, and 9 is an afternoon.
