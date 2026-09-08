//! End-to-end oracle test for the `scott` crate: loads the "Tiny Cave" fixture,
//! drives a fixed command script through the turn loop, and freezes the exact
//! transcript. The script is authored to exercise the corrected occurrence,
//! continuation, and counter semantics — every line below is explainable by the
//! ScottFree behavioral rules, not merely frozen from whatever the code prints.
//!
//! Distinguishing cases baked into the fixture (would FAIL under the old bugs):
//!   * The room-2 ambient occurrence uses noun=100 ("always"), NOT noun=0. Under
//!     the corrected "roll against 0 never passes" rule a noun-0 occurrence would
//!     never fire, so an always-occurrence must be encoded as 100.
//!   * A verb-0/noun-0 line (action 1) with a passing guard sits where nothing
//!     continues into it; it must NEVER fire standalone (the "SECRET" message and
//!     flag 10 stay untouched). Under the old "noun 0 always fires" bug it would.
//!   * PUSH BUTTON runs opcode 73 then chains two consecutive 0/0 continuation
//!     lines, each re-checking its own conditions. When the lever is unpulled the
//!     first (flag-5-gated) line is SKIPPED; after PULL LEVER it RUNS. The second
//!     line (unconditional) prints even though the first carries no 73 — proving
//!     the walk continues across consecutive 0/0 lines.
//!   * STASH uses opcode 81 to SWAP the live current counter with a backup, so a
//!     set/stash/change/stash-back round trip restores the original value (7),
//!     which the old select-an-index counter model would report as 3.
//!
//! A second test exercises `Vm::snapshot`/`Vm::restore`.

use scott::{Database, Vm};

const TINY_CAVE: &str = include_str!("tiny_cave.dat");

fn run_script(vm: &mut Vm, cmds: &[&str]) -> String {
    let mut transcript = String::new();
    transcript.push_str(&vm.take_output()); // intro (room 1 description)
    for cmd in cmds {
        vm.supply_line(cmd);
        vm.step();
        transcript.push_str(&format!("> {cmd}\n"));
        transcript.push_str(&vm.take_output());
    }
    transcript
}

#[test]
fn golden_transcript() {
    let db = Database::parse(TINY_CAVE).expect("tiny_cave.dat parses");
    let mut vm = Vm::new(db);
    vm.seed_rng(1); // deterministic; the only occurrence is noun=100 (always fires)

    // The room is shown by the panel (room_block), never the transcript. Room 1 is
    // a `*`-literal sentence (printed verbatim), has only a Down exit, and holds
    // the lamp — exits/items are period-separated (the C64 presentation style).
    let start_block = vm.room_block();
    assert_eq!(
        start_block,
        "You are in a sunlit forest clearing. A narrow path leads down into darkness.\n\
         \n\
         Obvious exits: Down.\n\
         \n\
         I can also see:\n\
         \x20 a brass lamp"
    );

    let script = [
        "push button", // continue-chain: click, first 0/0 line SKIPPED (lever unpulled), dust settles
        "pull lever",  // sets flag 5, arming the gated continuation line
        "push button", // continue-chain: click, hidden door (flag 5 set) THEN dust settles
        "count",       // op79: current counter <- 7
        "tally",       // op78: prints 7
        "stash",       // op81: swap current<->backup2  (current becomes 0)
        "tally",       // op78: prints 0
        "mark",        // op79: current counter <- 3
        "stash",       // op81: swap back  (current restored to 7, not 3)
        "tally",       // op78: prints 7   <-- distinguishes op81 SWAP from index-select
        "take lamp",   // verb synonym TAKE -> GET(10); auto-noun picks up the lamp
        "down",        // scripted GO-down (room1 -> room2); water occurrence fires AFTER the move
        "rub lamp",    // RUB reveals the idol; then the room-2 water occurrence fires
        "get idol",    // auto-noun GET picks up the idol; then the water occurrence
        "up",          // GO up back to room1; the water occurrence does NOT fire (we left room2)
        "down",        // scripted GO-down (room1 -> room2); water fires after
        "down",        // built-in GO (room2 -> room3); no water in room3
        "score",       // idol still carried -> fallback score action
        "drop idol",   // auto-noun DROP deposits the idol in the treasure room
        "score",       // idol now in treasure room -> win action fires, quits
    ];

    let transcript = run_script(&mut vm, &script);
    let expected = "\
> push button\n\
You press the button. A click echoes.\n\
Dust settles in the chamber.\n\
> pull lever\n\
The lever clicks into place.\n\
> push button\n\
You press the button. A click echoes.\n\
A hidden door grinds open.\n\
Dust settles in the chamber.\n\
> count\n\
> tally\n\
7\n\
> stash\n\
> tally\n\
0\n\
> mark\n\
> stash\n\
> tally\n\
7\n\
> take lamp\n\
OK.\n\
> down\n\
You hear water dripping somewhere in the darkness.\n\
> rub lamp\n\
The lamp's glow reveals a niche in the rock - and within it, a gleaming gold idol!\n\
You hear water dripping somewhere in the darkness.\n\
> get idol\n\
OK.\n\
You hear water dripping somewhere in the darkness.\n\
> up\n\
> down\n\
You hear water dripping somewhere in the darkness.\n\
> down\n\
> score\n\
You have 0 out of 1 treasures.\n\
> drop idol\n\
OK.\n\
> score\n\
You set the idol down. *** You have won! ***\n";
    assert_eq!(transcript, expected);
    assert!(vm.has_quit(), "win action should have executed opcode 63 (quit)");

    // The standalone verb-0/noun-0 line (action 1) never fired: its guard
    // (player in room 1) passed on every room-1 turn, but a noun-0 occurrence
    // must never fire on its own.
    assert!(
        !transcript.contains("SECRET"),
        "noun-0 line must not fire standalone"
    );
    assert!(!vm.flag(10), "noun-0 line's flag must never be set");
}

#[test]
fn snapshot_restore_round_trip() {
    let db = Database::parse(TINY_CAVE).expect("tiny_cave.dat parses");
    let mut vm = Vm::new(db);
    vm.take_output();

    // Reach room2 carrying the lamp, with flag3 set (RUB LAMP guard) and the
    // live current counter at 7 (via the fixture's COUNT verb), then snapshot.
    // The lamp lives in room1, so fetch it before descending again.
    for cmd in ["down", "up", "take lamp", "down", "rub lamp", "count"] {
        vm.supply_line(cmd);
        vm.step();
        vm.take_output();
    }
    assert_eq!(vm.current_room(), 2);
    assert_eq!(vm.item_loc(9), scott::database::CARRIED); // lamp carried
    assert!(vm.flag(3)); // RUB LAMP guard flag set
    assert_eq!(vm.counter(), 7); // current_counter

    let snap = vm.snapshot();

    // Mutate: move away, drop the lamp.
    vm.supply_line("up");
    vm.step();
    vm.take_output();
    vm.supply_line("drop lamp");
    vm.step();
    vm.take_output();

    assert_ne!(vm.current_room(), 2);
    assert_ne!(vm.item_loc(9), scott::database::CARRIED);

    vm.restore(&snap).expect("restore succeeds");

    assert_eq!(vm.current_room(), 2);
    assert_eq!(vm.item_loc(9), scott::database::CARRIED);
    assert!(vm.flag(3));
    assert_eq!(vm.counter(), 7);
}

#[test]
fn restore_rejects_malformed_input() {
    let db = Database::parse(TINY_CAVE).expect("tiny_cave.dat parses");
    let mut vm = Vm::new(db);
    assert!(vm.restore(&[]).is_err());
    assert!(vm.restore(&[1, 2, 3]).is_err());

    let mut snap = vm.snapshot();
    snap.truncate(snap.len() - 1);
    assert!(vm.restore(&snap).is_err()); // truncated

    let mut bad_count = vm.snapshot();
    bad_count[0] = 0xFF; // corrupt the item_loc length prefix
    assert!(vm.restore(&bad_count).is_err());
}
