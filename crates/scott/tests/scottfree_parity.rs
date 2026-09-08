//! ScottFree-parity tests for SQ-0628/SQ-0629: noun resolution (MatchUpItem
//! location matching, unknown-word handling, first-word direction promotion),
//! the opcode-69 lamp refill, and loader hardening against hostile headers.
//!
//! Reference: ScottFree 1.14 `scott.c` (cspiegel/scottfree-glk mirror) —
//! `MatchUpItem`, `GetInput`, `PerformActions`, `PerformLine` case 69.

use scott::{Action, Condition, Database, Item, LoadError, Room, Vm};
use scott::database::{CARRIED, LAMP_EMPTY_FLAG, LIGHT_SOURCE};

fn rooms3() -> Vec<Room> {
    (0..3)
        .map(|i| Room {
            exits: [0; 6],
            desc: format!("room{i}"),
            literal: true,
        })
        .collect()
}

fn base_db(items: Vec<Item>) -> Database {
    let mut verbs = vec![String::new(); 19];
    verbs[1] = "GO".into();
    verbs[10] = "GET".into();
    verbs[18] = "DROP".into();
    let mut nouns = vec![String::new(); 8];
    nouns[1] = "NORTH".into();
    nouns[2] = "SOUTH".into();
    nouns[3] = "EAST".into();
    nouns[4] = "WEST".into();
    nouns[5] = "UP".into();
    nouns[6] = "DOWN".into();
    nouns[7] = "BOTTLE".into();
    Database {
        max_carry: 6,
        start_room: 1,
        num_treasures: 0,
        word_length: 3,
        light_time: -1,
        treasure_room: 0,
        actions: vec![],
        verbs,
        nouns,
        rooms: rooms3(),
        messages: vec![String::new()],
        items,
        adventure_number: 0,
    }
}

/// Two items sharing an auto-noun, the OUT-OF-PLAY one first in the table —
/// Adventureland's two `/BOT/` bottles, or a lit/unlit lamp pair.
fn twin_bottles(first_loc: i32, second_loc: i32) -> Vec<Item> {
    vec![
        Item {
            text: "an empty bottle".into(),
            treasure: false,
            auto_noun: Some("BOT".into()),
            start_loc: first_loc,
        },
        Item {
            text: "a bottle of water".into(),
            treasure: false,
            auto_noun: Some("BOT".into()),
            start_loc: second_loc,
        },
    ]
}

// ── SQ-0628: MatchUpItem requires the location to match ──────────────────────

/// ScottFree's `MatchUpItem(NounText, MyLoc)`: GET must resolve the duplicate
/// auto-noun to the twin IN THE ROOM, not the first table entry (which is out
/// of play here).
#[test]
fn get_matches_the_in_room_twin_not_the_first_table_entry() {
    // Item 0 (first match under the old rule) is nowhere; item 1 is in room 1.
    let mut vm = Vm::new(base_db(twin_bottles(0, 1)));
    vm.take_output();
    vm.supply_line("get bottle");
    vm.step();
    let out = vm.take_output();
    assert!(out.contains("OK."), "GET succeeds on the in-room twin: {out:?}");
    assert_eq!(vm.item_loc(1), CARRIED, "the in-room bottle was taken");
    assert_eq!(vm.item_loc(0), 0, "the out-of-play twin is untouched");
}

/// ScottFree's DROP path uses `MatchUpItem(NounText, CARRIED)`: the carried
/// twin must be found even when the not-carried twin comes first in the table.
#[test]
fn drop_matches_the_carried_twin_not_the_first_table_entry() {
    // Item 0 (first match under the old rule) is nowhere; item 1 is carried.
    let mut vm = Vm::new(base_db(twin_bottles(0, CARRIED)));
    vm.take_output();
    vm.supply_line("drop bottle");
    vm.step();
    let out = vm.take_output();
    assert!(out.contains("OK."), "DROP succeeds on the carried twin: {out:?}");
    assert_eq!(vm.item_loc(1), 1, "the carried bottle lands in the room");
    assert_eq!(vm.item_loc(0), 0, "the out-of-play twin is untouched");
}

/// GET on a known noun whose item is elsewhere is "beyond my power", not a
/// grab of an out-of-play twin (ScottFree's MatchUpItem miss reply).
#[test]
fn get_when_no_twin_is_in_the_room_is_beyond_my_power() {
    // Both bottles out of reach: one nowhere, one in another room.
    let mut vm = Vm::new(base_db(twin_bottles(0, 2)));
    vm.take_output();
    vm.supply_line("get bottle");
    vm.step();
    let out = vm.take_output();
    assert!(
        out.contains("It's beyond my power to do that."),
        "GET of an absent item is refused: {out:?}"
    );
    assert_eq!(vm.item_loc(0), 0);
    assert_eq!(vm.item_loc(1), 2);
}

// ── SQ-0628: GetInput parity — unknown words, direction promotion ────────────

/// An unknown FIRST word is ScottFree's "You use word(s) I don't know!" and no
/// turn passes — even when the second word is a direction. Under the old rule
/// the vb==0 && no∈1..=6 promotion moved the player.
#[test]
fn unknown_first_word_with_direction_second_word_does_not_move() {
    let mut db = base_db(twin_bottles(0, 0));
    db.rooms[1].exits[0] = 2; // north -> room 2
    // An always-occurrence: fires every turn that actually passes.
    db.messages = vec![String::new(), "The wind howls.".into()];
    db.actions.push(Action {
        verb: 0,
        noun: 100,
        conditions: [Condition { code: 0, value: 0 }; 5],
        commands: [1, 0, 0, 0],
    });
    let mut vm = Vm::new(db);
    vm.take_output();

    vm.supply_line("xyzzy north");
    vm.step();
    let out = vm.take_output();
    assert!(
        out.contains("You use word(s) I don't know!"),
        "ScottFree's unknown-words reply: {out:?}"
    );
    assert_eq!(vm.current_room(), 1, "an unknown verb must not move the player");
    assert!(
        !out.contains("The wind howls."),
        "no turn passes on an unknown verb (ScottFree re-prompts inside GetInput): {out:?}"
    );

    // A real command still passes a turn: the occurrence fires.
    vm.supply_line("north");
    vm.step();
    let out = vm.take_output();
    assert_eq!(vm.current_room(), 2, "a direction first word moves");
    assert!(out.contains("The wind howls."), "a real turn runs occurrences: {out:?}");
}

/// ScottFree's GetInput checks the FIRST word against the noun list: a
/// direction there becomes GO <dir> and the second word is ignored.
#[test]
fn direction_first_word_moves_even_with_a_junk_second_word() {
    let mut db = base_db(twin_bottles(0, 0));
    db.rooms[1].exits[0] = 2; // north -> room 2
    let mut vm = Vm::new(db);
    vm.take_output();
    vm.supply_line("north xyzzy");
    vm.step();
    assert_eq!(
        vm.current_room(),
        2,
        "the Scott 'avoid typing GO' hack promotes the first word; the second is ignored"
    );
}

/// GET with an unknown noun is ScottFree's "What ?", never a grab and never a
/// generic not-understood.
#[test]
fn get_with_unknown_noun_asks_what() {
    let mut vm = Vm::new(base_db(twin_bottles(1, 1)));
    vm.take_output();
    vm.supply_line("get xyzzy");
    vm.step();
    let out = vm.take_output();
    assert!(out.contains("What?"), "unknown GET noun asks What?: {out:?}");
    assert_eq!(vm.item_loc(0), 1, "nothing was taken");
    assert_eq!(vm.item_loc(1), 1, "nothing was taken");
}

/// A lone GO (or GO + unknown noun) asks for a direction BEFORE the action
/// table, so a catch-all "GO ANY" action cannot swallow it (ScottFree's
/// `vb==1 && no==-1` early reply).
#[test]
fn bare_go_asks_for_a_direction_before_the_action_table() {
    let mut db = base_db(twin_bottles(0, 0));
    db.messages = vec![String::new(), "You wander aimlessly.".into()];
    db.actions.push(Action {
        verb: 1, // catch-all GO <anything>
        noun: 0,
        conditions: [Condition { code: 0, value: 0 }; 5],
        commands: [1, 0, 0, 0],
    });
    let mut vm = Vm::new(db);
    vm.take_output();
    vm.supply_line("go");
    vm.step();
    let out = vm.take_output();
    assert!(out.contains("I need a direction."), "bare GO asks for a direction: {out:?}");
    assert!(
        !out.contains("You wander aimlessly."),
        "the catch-all GO action must not fire on a bare GO: {out:?}"
    );
}

// ── SQ-0628: opcode 69 (refill lamp) parity ──────────────────────────────────

/// ScottFree case 69: `GameHeader.LightTime=LightRefill;
/// Items[LIGHT_SOURCE].Location=CARRIED; BitFlags&=~(1<<LIGHTOUTBIT);` — the
/// light source returns to the pack, not just the fuel and flag.
#[test]
fn refill_lamp_op69_moves_the_light_source_into_the_pack() {
    let mut items: Vec<Item> = (0..10)
        .map(|i| Item {
            text: format!("filler{i}"),
            treasure: false,
            auto_noun: None,
            start_loc: 0,
        })
        .collect();
    items[LIGHT_SOURCE].text = "an old lamp".into();
    let mut db = base_db(items);
    db.light_time = 100;
    // Verb 5 = REFILL, wired straight to opcode 69.
    db.verbs[5] = "REFILL".into();
    db.actions.push(Action {
        verb: 5,
        noun: 0,
        conditions: [Condition { code: 0, value: 0 }; 5],
        commands: [69, 0, 0, 0],
    });
    let mut vm = Vm::new(db);
    vm.take_output();
    assert_eq!(vm.item_loc(LIGHT_SOURCE), 0, "the lamp starts out of play");

    vm.supply_line("refill");
    vm.step();
    assert_eq!(
        vm.item_loc(LIGHT_SOURCE),
        CARRIED,
        "opcode 69 puts the light source into the pack (ScottFree case 69)"
    );
    // Refill sets the fuel to LightTime (100); the same turn's end-of-turn
    // lamp tick (the lamp is now carried and lit) consumes one, as in
    // ScottFree's main loop, which counts down after PerformActions.
    assert_eq!(vm.lamp(), 99, "fuel was reset to LightTime and ticked once");
    assert!(!vm.flag(LAMP_EMPTY_FLAG), "the lamp-empty flag is cleared");
}

// ── SQ-0629: movement never lands in a nonexistent room ──────────────────────

/// A corrupt exit value (only reachable via a hand-built Database — the loader
/// rejects them) is treated as no exit rather than soft-locking the player in
/// a nonexistent room.
#[test]
fn out_of_range_exit_is_treated_as_no_exit() {
    let mut db = base_db(twin_bottles(0, 0));
    db.rooms[1].exits[0] = 99; // north -> nonexistent room
    let mut vm = Vm::new(db);
    vm.take_output();
    vm.supply_line("north");
    vm.step();
    let out = vm.take_output();
    assert_eq!(vm.current_room(), 1, "the player must not enter a nonexistent room");
    assert!(out.contains("can't go"), "the blocked move is reported: {out:?}");
}

// ── SQ-0629: loader hardening against hostile headers ────────────────────────

/// A hostile NumActions (2e9 would request ~64GB of Vec capacity before any
/// body token is read) must be rejected up front.
#[test]
fn hostile_action_count_is_rejected_without_allocating() {
    let bad = "32767 1 2000000000 1 2 6 1 0 3 125 0 1\n";
    assert_eq!(
        Database::parse(bad),
        Err(LoadError::BadCount("NumActions", 2_000_000_000))
    );
}

/// Every pre-reserved count is bounded, not just NumActions.
#[test]
fn hostile_counts_in_every_header_slot_are_rejected() {
    // Header slots: _, items, actions, words, rooms, carry, room, treasures,
    // wordlen, time, messages, treasure_room.
    for (idx, name) in [
        (1, "NumItems"),
        (2, "NumActions"),
        (3, "NumWords"),
        (4, "NumRooms"),
        (10, "NumMessages"),
    ] {
        let mut fields = ["0"; 12];
        fields[8] = "3"; // plausible word length
        let big = "1000000000";
        fields[idx] = big;
        let src = fields.join(" ");
        match Database::parse(&src) {
            Err(LoadError::BadCount(n, v)) => {
                assert_eq!(n, name);
                assert_eq!(v, 1_000_000_000);
            }
            other => panic!("{name}: expected BadCount, got {other:?}"),
        }
    }
}

/// Room exits must index the room table: a negative exit would wrap to a huge
/// usize, an over-large one points past the table.
#[test]
fn out_of_range_room_exit_is_rejected_at_load() {
    // NumRooms=2 (3 slots), but room 1's north exit says 9.
    const BAD_EXIT: &str = r#"
32767 1 0 1 2 6 1 0 3 125 0 1
150 1 0 0 0 0 0 0
"" ""
"GO" "NORTH"
0 0 0 0 0 0 "*limbo"
9 0 0 0 0 0 "*forest clearing"
0 0 0 0 0 0 "*swamp"
""
"" 0
"*a brass lamp/LAMP/" 1
"#;
    assert_eq!(Database::parse(BAD_EXIT), Err(LoadError::BadExit(9)));

    let negative = BAD_EXIT.replacen("9 0 0 0 0 0", "-5 0 0 0 0 0", 1);
    assert_eq!(Database::parse(&negative), Err(LoadError::BadExit(-5)));
}
