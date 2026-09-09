# lanthorn-scott

A zero-dependency virtual machine for Scott Adams (ScottFree `.dat`) format
text adventures — the classic early-1980s interactive-fiction engine.

It is the Scott Adams engine behind
[lanthorn](https://github.com/sharkusk/lanthorn), a terminal
interactive-fiction player with live automapping, and is also usable
standalone by anything that wants to run Scott Adams story files.

## What it reads

The **ScottFree `.dat`** text format: the whitespace-separated integers and
quoted strings that Adventure International's own games, Brian Howarth's
Mysterious Adventures, and their many successors were converted into. Anything
byte-like goes in — a Latin-1 file loads as readily as a UTF-8 one — and
`Database::parse` answers either the game data or a `LoadError` naming what in
the file didn't fit.

## What it names but refuses

Scott Adams games also shipped in several **binary** dialects that this crate
does not read: TI-99/4A game images, and the C64 / ZX Spectrum / Atari 8-bit /
Apple II memory snapshots that carry the game's tables as machine data rather
than as text. `detect_dialect` recognises those by the opening bytes of the
game's own verb dictionary, and `Database::parse` reports one as
`LoadError::UnsupportedDialect(dialect)` — so a host can tell the player *"this
is a TI-99/4A game image"* rather than showing a parse error about a stray
token, which is what a file full of machine code produces when a text lexer
reaches it.

Detection is checked two ways: fixtures built from the format rules, committed
and run on every build, and the real game images, which are commercial files
and so are not redistributable — `tests/dialect_specimens.rs` measures those
when they are present and skips, loudly, when they are not.

Reading these dialects is tracked as SQ-1414. It is gated on a description of
each format written independently of the established interpreters, which are
GPL where this crate is BSD-3-Clause; every fact in the detector above was
measured off real game files for the same reason.
