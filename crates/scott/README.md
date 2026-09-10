# lanthorn-scott

A zero-dependency virtual machine for Scott Adams (ScottFree `.dat`) format
text adventures — the classic early-1980s interactive-fiction engine.

It is the Scott Adams engine behind
[lanthorn](https://github.com/sharkusk/lanthorn), a terminal
interactive-fiction player with live automapping, and is also usable
standalone by anything that wants to run Scott Adams story files.

## Experimental as an embeddable crate

The VM (`Vm`, `Database`, the `step`/`supply_line` protocol) and the
reference ScottFree `.dat` text format are stable and tested — that part of
this crate has been the standard-implementation goal from the start. The
platform loaders and picture decoders added in September 2026 (TI-99/4A, ZX
Spectrum and Commodore 64 *Mysterious Adventures*, and the US S.A.G.A.
binary database on the Atari 8-bit, Apple II, Commodore 64 and MS-DOS) are
newer: their public types' fields and variants may still change, and they
have not yet had a hostile-input fuzz pass the way the reference format has
(tracked together as SQ-1502). Treat those readers as a preview if you
embed this crate today.

## What it reads

`Database::parse` answers for five encodings of the same game data from one
entry point — hand it a file's raw bytes and it returns the static game data
(rooms, items, the action table, vocabulary, and messages) or a `LoadError`
naming what didn't fit:

- The **ScottFree `.dat`** text format: the whitespace-separated integers and
  quoted strings that Adventure International's own games, Brian Howarth's
  Mysterious Adventures, and their many successors were converted into.
  Anything byte-like goes in — a Latin-1 file loads as readily as a UTF-8
  one.
- The **TI-99/4A tokenised releases** — the twelve original Adventure
  International games as sold for that machine, a raw memory image with the
  script compiled to bytecode rather than text (`parse_ti994a`).
- The **Commodore 64** and **ZX Spectrum** editions of Brian Howarth's
  *Mysterious Adventures* — eleven titles each, as a Commodore program file
  (`parse_c64_mysterious_prg`) and a 48K `.z80` snapshot
  (`parse_zx_mysterious_z80`) respectively, both carrying line-drawn artwork
  this crate also decodes into indexed bitmaps.
- The **US S.A.G.A. binary database** — the American "Scott Adams Graphic
  Adventure" disk editions of Adventures 1-6 and 13 for the Atari 8-bit and
  Apple II, and the Questprobe *Hulk* for the Commodore 64
  (`parse_saga_us`).

A container (a `.d64` compilation disk, an `.atr`, a `.dsk`) is the host's
business, not this crate's: it takes the program file's own bytes, load
address included where the format carries one, and hands back a `Database`
or names what was wrong with it.

## What it names but refuses

Scott Adams games also shipped on the Commodore 64, ZX Spectrum, Atari
8-bit and Apple II as further binary dialects this crate does **not** read:
any memory snapshot outside the two *Mysterious Adventures* readers and the
S.A.G.A. releases above, and every ZX Spectrum title whose action table and
text are stored compressed. `detect_dialect` recognises those by the opening
bytes of the game's own verb dictionary, and `Database::parse` reports one
as `LoadError::UnsupportedDialect(dialect)` — so a host can tell the player
*"this is a Commodore 64 memory snapshot"* rather than showing a parse error
about a stray token, which is what a file full of machine code produces when
a text lexer reaches it.

Detection is checked two ways: fixtures built from the format rules,
committed and run on every build, and the real game images, which are
commercial files and so are not redistributable — the `*_specimens` test
suites measure those when they are present and skip, loudly, when they are
not.

## Driving a session

`Vm::new` (or `Vm::new_seeded`, to fix the PRNG a game's occurrence rolls
draw from) wraps a `Database` in mutable play state. `Vm::step` runs one turn
if a command is buffered and returns a `StepResult` — `NeedLine` (read output
with `Vm::take_output`, then supply the player's next line with
`Vm::supply_line`) or `Quit` (the game ended). There is no separate "engine
fault" outcome: a Scott Adams database has no opcodes that can misbehave the
way a general-purpose VM's can.

`Vm::snapshot`/`Vm::restore` save and load the mutable half of play state
behind a magic and format version; this encoding is this crate's own host
save format, not a Scott-Adams standard (there is no such standard). A host
wanting a save format of its own builds one from the accessors
(`Vm::item_loc`, `Vm::flag`, `Vm::counter`, `Vm::current_room`, `Vm::lamp`,
…) instead of persisting these bytes directly.

## Where to read more

Crate-level documentation (`cargo doc --open -p lanthorn-scott`, or
[docs.rs/lanthorn-scott](https://docs.rs/lanthorn-scott) once published)
covers every dialect's module and the full save/restore story.

Reading the remaining refused dialects is tracked as SQ-1414.

License: BSD-3-Clause.
