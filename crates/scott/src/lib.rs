//! A zero-dependency virtual machine for Scott Adams (ScottFree `.dat`)
//! format interactive fiction — the database format Scott Adams' own
//! Adventure International games and their ScottFree-compatible successors
//! use: a text file of whitespace-separated integers and `"`-quoted strings
//! describing rooms, items, an action table of verb/noun-triggered
//! conditions and commands, and a message pool. No two Scott Adams
//! interpreters agree on every corner of the format by writing it down in
//! one place first; this crate's behaviour is checked against ScottFree's own
//! OUTPUT — run as a black box and compared byte-for-byte in the
//! `scottfree_parity` suite — and against `docs/internals/scott-dialects-spec.md`,
//! never against ScottFree's C source, and each site where the two could
//! disagree says so in its doc comment. Where the Swansea "Definition"
//! document is silent, or disagrees with what ScottFree 1.14 actually prints,
//! this crate follows ScottFree's observed behaviour — the Definition
//! documents the format's shape, but ScottFree's own output is what every
//! commercial `.dat` was authored and tested against, so it is the more
//! authoritative oracle for anything the format itself leaves unstated
//! (message wording, movement in the dark, the lamp countdown, the
//! auto-get/drop noun split, and so on).
//!
//! Like [`lanthorn-gvm`](https://docs.rs/lanthorn-gvm) and
//! `lanthorn-zvm`, this crate takes **zero external dependencies** — all
//! parsing and save/restore encoding is hand-rolled — so it can sit behind
//! any host's own I/O policy. lanthorn (a terminal interactive-fiction
//! player) is one such host; this crate does not know it exists.
//!
//! # Where this crate KNOWINGLY disagrees with ScottFree
//!
//! Three places where ScottFree 1.14 itself, Spatterlight's `terps/scott`
//! fork, and the Swansea Definition document do not all agree, and this
//! crate had to pick one (SQ-1413's reference audit, 2026-09-08):
//!
//! * **Condition 16** (`vm.rs`, `Vm::eval_condition`) is `>` in both
//!   ScottFree and Spatterlight, but `>=` in the Definition. This crate
//!   follows ScottFree/Spatterlight.
//! * **Opcode 77**'s decrement floors at -1 in ScottFree, but at 0 in
//!   Spatterlight's fork (`vm.rs`, `Vm::run_commands` case 77). This crate
//!   follows ScottFree.
//! * **Opcode 89** is the SAGA "draw picture" command in both ScottFree and
//!   the Definition, but Spatterlight renumbered it to 90 in its own fork
//!   (`vm.rs`, `Vm::run_commands` case 89). This crate follows ScottFree/the
//!   Definition — opcode 90 is unused — for every database except the US
//!   S.A.G.A. binary ones (`Database::saga_us.is_some()`), where 89 and 90
//!   are both real commands with operand counts the reverse of the
//!   reference format's own numbering (`docs/internals/scott-dialects-spec.md`
//!   §12.8/§12.11, SQ-1472).
//!
//! All three follow this module doc's stated priority: ScottFree's own
//! behaviour outranks a document or a fork wherever they disagree, because
//! every commercial `.dat` was authored and tested against ScottFree.
//!
//! # Loading a story
//!
//! This crate reads **five** encodings of the same game data, and
//! [`Database::parse`] answers for all of them from one entry point — hand it
//! the file's raw bytes and it returns the static game data (rooms, items, the
//! action table, vocabulary, and messages) or a [`LoadError`] naming what
//! did not fit:
//!
//! * the **ScottFree `.dat` text format**, the plain-ASCII interchange
//!   encoding described at the top of this page;
//! * the **TI-99/4A tokenised releases** — the twelve original Adventure
//!   International games as sold for that machine, which are a raw memory
//!   image with the script compiled to bytecode rather than text at all
//!   ([`parse_ti994a`], and [`crate::ti994a`] for the format); and
//! * the **Commodore 64 *Mysterious Adventures*** — Brian Howarth's eleven
//!   titles from *The Golden Baton* to *Waxworks*, as Commodore program files
//!   holding an uncompressed 6502 memory image ([`parse_c64_mysterious_prg`],
//!   and [`crate::c64`] for the format). Those releases also carry line-drawn
//!   artwork, which [`decode_family_b_pictures`] turns into indexed bitmaps.
//! * the **ZX Spectrum *Mysterious Adventures*** — the same eleven titles on
//!   the other side of the Irish Sea, as 48K `.z80` snapshots
//!   ([`parse_zx_mysterious_z80`], and [`crate::zx_mysterious`] for the
//!   format), carrying the same line-drawn artwork. Unlike every other binary
//!   dialect here this one needs **no per-release catalogue at all**: the
//!   driver plants nine table addresses in the nine words that follow the
//!   header, so the header's field order and the dictionary's verb/noun split
//!   are read out of the bytes rather than looked up.
//! * the **US S.A.G.A. binary database** — the American "Scott Adams Graphic
//!   Adventure" disk editions of Adventures 1-6 and 13 for the Atari 8-bit and
//!   the Apple II, and the Questprobe *Hulk* for the Commodore 64: a flat
//!   binary array with a fifteen-word header, a dictionary of nouns then
//!   verbs, length-prefixed strings, a **column-major** action table and
//!   **direction-major** room connections ([`parse_saga_us`], and
//!   [`crate::saga_us`] for the format).
//!
//! **A container is the host's business, not this crate's.** These eleven ship
//! on two `.d64` compilation disks holding six and five games each, and
//! nothing in the container says which one a player wants; a host extracts the
//! named program file and hands the bytes over. What this crate does take is
//! the program file itself, load-address bytes and all, because stripping
//! those two bytes is part of reading the format rather than part of reading
//! the disk. The same split holds for the S.A.G.A. releases: the host mounts
//! the ATR, `.dsk` or `.d64` and hands over what it found, and
//! [`SagaPlatform`] carries the one offset that turns those bytes into the
//! database array.
//!
//! [`looks_like_scott_bytes`] is the sniff to reach for when a host is
//! guessing among several engines from a file's bytes alone: it answers for
//! both. ([`looks_like_scott`] is the text-only half, and takes a `&str`, so
//! it can never answer for a binary dialect — a host spelling its check
//! `from_utf8(bytes).is_ok_and(looks_like_scott)` rejects every TI-99/4A
//! file before this crate ever sees it.)
//!
//! Scott Adams games also shipped in binary dialects this crate does NOT
//! read: the Atari 8-bit and Apple II memory snapshots that carry the tables
//! as machine data, the ZX Spectrum releases outside the *Mysterious
//! Adventures* series (the character-cell picture family, and the two that
//! compress their action table and their text), and the Commodore 64 releases
//! outside that series. It does [`detect_dialect`] them, so a
//! failed parse over one comes back as [`LoadError::UnsupportedDialect`]
//! and a host can say "this is a Commodore 64 memory snapshot" instead of
//! reporting whichever token the text lexer tripped over first. A file that
//! IS one of the three readable encodings but is damaged comes back as
//! [`LoadError::BadDialectData`] instead, naming what did not check out.
//! See [`Dialect`] for what each signature is and how it was established.
//!
//! Some of those snapshots are further wrapped in a HOST-machine container
//! before the game's own tables are reachable at all — a ZX Spectrum
//! `.z80` file, for instance, RLE-compresses the whole 48K memory image the
//! tables live in, so no in-memory offset means anything until that layer
//! is peeled off first. [`decompress_z80`] does that one container step
//! (see its module docs for the format and its source), and
//! [`parse_zx_mysterious_z80`] is that step plus the Spectrum loader spelled
//! once — [`Database::parse`] recognises a `.z80` and runs both, so a host
//! that already hands this crate a file's bytes needs no new wiring. Nothing
//! reads a snapshot's raw bytes: the RLE passes literal text through, so the
//! dictionary signature is right there in the compressed file at an offset
//! that means nothing.
//!
//! # Driving a session
//!
//! [`Vm::new`] (or [`Vm::new_seeded`], to fix the PRNG a game's occurrence
//! rolls draw from) wraps a [`Database`] in mutable play state. From there
//! the host drives a small, explicit protocol:
//!
//! * [`Vm::step`] runs one turn if a command is buffered, then returns a
//!   [`StepResult`] saying what the VM needs next — [`StepResult::NeedLine`]
//!   (read output with [`Vm::take_output`], then supply the player's next
//!   line) or [`StepResult::Quit`] (the game ended).
//! * [`Vm::supply_line`] hands the VM the player's typed command.
//!
//! There is no separate "engine fault" outcome: a Scott Adams database has
//! no opcodes that can misbehave the way a general-purpose VM's can, so
//! every [`StepResult`] is one of those two.
//!
//! # Saving
//!
//! [`Vm::snapshot`] serializes the mutable half of play state (item
//! locations, the player's room, flags, counters, the op-80/op-87 saved-room
//! registers, and lamp fuel) to a compact byte buffer, behind a 4-byte magic
//! ([`Vm::SNAPSHOT_MAGIC`]) and a `u16` format version
//! ([`Vm::SNAPSHOT_VERSION`], SQ-1402); [`Vm::restore`] checks both before
//! reading anything else, and rejects a snapshot whose shape doesn't match
//! this `Vm`'s own database (wrong item count, an out-of-range room or
//! counter index) — every failure mode is a [`RestoreError`] rather than
//! silently corrupted state. Nothing about the encoding is
//! Scott-Adams-standard — there is no such standard for host save state — so
//! a snapshot is only ever read back by a `scott` build whose
//! `SNAPSHOT_VERSION` is at least the one the file declares; there is no
//! back-compat requirement pre-release, and no build reads the headerless
//! form this crate produced before SQ-1402 (`BadMagic`, since it never wrote
//! this magic). A host wanting a durable save format of its own builds one
//! from the accessors ([`Vm::item_loc`], [`Vm::flag`], [`Vm::counter`],
//! [`Vm::current_room`], [`Vm::lamp`], …) rather than persisting these bytes
//! directly.
//!
//! # Example
//!
//! A minimal two-room world built by hand (no `.dat` file needed), driven
//! through the `step`/`supply_line` protocol, then saved and restored:
//!
//! ```rust
//! use scott::{Database, Room, StepResult, Vm};
//!
//! let db = Database {
//!     max_carry: 6,
//!     start_room: 1,
//!     num_treasures: 0,
//!     word_length: 3,
//!     light_time: -1, // -1 = no lamp to manage
//!     treasure_room: 0,
//!     actions: vec![],
//!     verbs: vec![String::new()], // index 0 is always a placeholder
//!     nouns: vec![String::new(), "NORTH".into()],
//!     rooms: vec![
//!         Room { exits: [0; 6], desc: "limbo".into(), literal: true }, // room 0: unused
//!         Room { exits: [2, 0, 0, 0, 0, 0], desc: "a dusty study".into(), literal: true },
//!         Room { exits: [0; 6], desc: "a cramped attic".into(), literal: true },
//!     ],
//!     messages: vec![],
//!     items: vec![],
//!     adventure_number: 0,
//!     ti99: None, // a TI-99/4A tokenised script; None for every other source
//!     mysterious: false, // Brian Howarth's Mysterious Adventures series; false for every other source
//!     saga_us: None, // a US S.A.G.A. release identity; None for every other source
//! };
//!
//! let mut vm = Vm::new(db);
//! assert_eq!(vm.step(), StepResult::NeedLine);
//! assert_eq!(vm.current_room(), 1);
//!
//! let save = vm.snapshot();
//!
//! vm.supply_line("north");
//! assert_eq!(vm.step(), StepResult::NeedLine);
//! assert_eq!(vm.current_room(), 2);
//! assert_eq!(vm.room_name(vm.current_room()), "a cramped attic");
//!
//! vm.restore(&save).unwrap();
//! assert_eq!(vm.current_room(), 1);
//! ```
//!
//! See `examples/run_story.rs` for a complete stdin/stdout host that loads a
//! real `.dat` file and plays it.

#![warn(missing_docs)]

mod loader;
mod options;
mod scottfree_save;
mod vm;
mod z80;
pub mod c64;
pub mod database;
pub mod decompile;
pub mod saga_us;
pub mod ti994a;
pub mod zx_mysterious;
pub use c64::{
    decode_family_b_block, decode_family_b_pictures, looks_like_c64_mysterious,
    looks_like_c64_mysterious_prg, parse_c64_mysterious, parse_c64_mysterious_prg, prg_image,
    HeaderShape, Picture, Release, RELEASES,
};
pub use database::{Action, Condition, Database, Item, Room};
pub use decompile::{decompile_action, list_items, list_rooms, list_vocab};
pub use loader::{detect_dialect, looks_like_scott, looks_like_scott_bytes, Dialect, LoadError};
pub use options::{Options, Presentation, Wording};
pub use saga_us::{
    detect_saga_us, looks_like_saga_us, parse_saga_us, SagaPlatform, SagaUs, DARKNESS_PICTURE,
    INVENTORY_PICTURE,
};
pub use scottfree_save::looks_like_scottfree_save;
pub use ti994a::{looks_like_ti994a, parse_ti994a, Ti99Record, Ti99Script};
pub use vm::{RestoreError, StepResult, Vm};
pub use z80::{decompress_z80, looks_like_z80, Z80Error, IMAGE_LEN};
pub use zx_mysterious::{
    looks_like_zx_mysterious, looks_like_zx_mysterious_z80, parse_zx_mysterious,
    parse_zx_mysterious_z80, ZxLayout, ZxRelease,
};
