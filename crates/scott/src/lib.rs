//! A zero-dependency virtual machine for Scott Adams (ScottFree `.dat`)
//! format interactive fiction — the database format Scott Adams' own
//! Adventure International games and their ScottFree-compatible successors
//! use: a text file of whitespace-separated integers and `"`-quoted strings
//! describing rooms, items, an action table of verb/noun-triggered
//! conditions and commands, and a message pool. No two Scott Adams
//! interpreters agree on every corner of the format by writing it down in
//! one place first; this crate's behaviour is checked against ScottFree's
//! own C source wherever the two could disagree, and each such site says so
//! in its doc comment. Where the Swansea "Definition" document is silent, or
//! disagrees with what ScottFree 1.14 (`ScottCurses.c`) actually does, this
//! crate follows ScottFree — the Definition documents the format's shape,
//! but ScottFree's own behaviour is what every commercial `.dat` was
//! authored and tested against, so it is the more authoritative oracle for
//! anything the format itself leaves unstated (message wording, movement in
//! the dark, the lamp countdown, the auto-get/drop noun split, and so on).
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
//!   Definition — opcode 90 is unused.
//!
//! All three follow this module doc's stated priority: ScottFree's own
//! behaviour outranks a document or a fork wherever they disagree, because
//! every commercial `.dat` was authored and tested against ScottFree.
//!
//! # Loading a story
//!
//! [`looks_like_scott`] sniffs whether a text buffer is plausibly this
//! format (useful when a host is guessing among several engines from a
//! file's bytes alone); [`Database::parse`] does the real parse and returns
//! the static game data — rooms, items, the action table, vocabulary, and
//! messages — or a [`LoadError`] naming what in the text didn't fit.
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

mod loader;
mod options;
mod scottfree_save;
mod vm;
pub mod database;
pub mod decompile;
pub use database::{Action, Condition, Database, Item, Room};
pub use decompile::{decompile_action, list_items, list_rooms, list_vocab};
pub use loader::{detect_dialect, looks_like_scott, Dialect, LoadError};
pub use options::{Options, Presentation, Wording};
pub use vm::{RestoreError, StepResult, Vm};
