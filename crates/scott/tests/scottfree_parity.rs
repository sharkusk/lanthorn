//! ScottFree-parity tests for SQ-0628/SQ-0629: noun resolution (MatchUpItem
//! location matching, unknown-word handling, first-word direction promotion),
//! the opcode-69 lamp refill, and loader hardening against hostile headers.
//!
//! Reference: ScottFree 1.14 `scott.c` (cspiegel/scottfree-glk mirror) —
//! `MatchUpItem`, `GetInput`, `PerformActions`, `PerformLine` case 69.

use std::path::PathBuf;

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

// ── SQ-1412: ScottFree format-required behaviours ────────────────────────────
//
// Reference: ScottFree 1.14 (ifarchive ScottFree.tar.gz, `ScottCurses.c`),
// built locally against a small stdout curses stub to diff transcripts.
// Every fixture below is purpose-built and minimal — just enough database to
// exercise one rule in isolation, not a real game.

// Item 1: GetInput's single-letter direction/INVENTORY expansion
// (ScottCurses.c:613-625) must happen BEFORE vocabulary lookup and
// word-length truncation.
#[test]
fn single_letter_directions_and_inventory_expand_before_vocab_lookup() {
    const DAT: &str = r#"
0 0 1 2 2 6 1 0 3 -1 0 0
0 0 0 0 0 0 0 0
300 0 0 0 0 0 9900 0
"" ""
"" "NORTH"
"INVENTORY" ""
0 0 0 0 0 0 "*limbo"
2 0 0 0 0 0 "*start room"
0 0 0 0 0 0 "*north room"
""
"" 0
"#;
    let db = Database::parse(DAT).expect("parses");
    let mut vm = Vm::new(db);
    vm.take_output();

    vm.supply_line("n");
    vm.step();
    assert_eq!(vm.current_room(), 2, "'n' expands to NORTH and moves via the exit table");

    vm.supply_line("i");
    vm.step();
    let out = vm.take_output();
    assert!(
        out.contains("carrying nothing"),
        "'i' expands to INVENTORY, matches the truncated verb, and fires opcode 66: {out:?}"
    );

    // ScottFree's rule fires only when `*noun==0`: a single-letter word WITH
    // a second word must not expand.
    vm.supply_line("n xyzzy");
    vm.step();
    assert!(
        vm.take_output().contains("You use word(s) I don't know!"),
        "a single-letter word with a second word is not expanded"
    );
}

// Item 2: opcode 65 (SCORE), once every treasure is stored, prints "Well
// done." and falls through to opcode 63's ending (ScottCurses.c:899-923,
// `goto doneit`).
#[test]
fn op65_win_prints_well_done_and_ends_the_game() {
    const DAT: &str = r#"
0 1 1 2 1 6 1 1 5 -1 0 1
0 0 0 0 0 0 0 0
300 0 0 0 0 0 9750 0
"" ""
"" ""
"SCORE" ""
0 0 0 0 0 0 "*limbo"
0 0 0 0 0 0 "*vault"
""
"" 0
"*gold coin" 1
"#;
    let db = Database::parse(DAT).expect("parses");
    let mut vm = Vm::new(db);
    vm.take_output();

    vm.supply_line("score");
    vm.step();
    let out = vm.take_output();
    assert!(out.contains("Well done."), "win prints Well done.: {out:?}");
    assert!(
        out.contains("The game is now over."),
        "win falls through to opcode 63's ending line: {out:?}"
    );
    assert!(vm.has_quit(), "SCORE with every treasure deposited ends the game");
}

// Item 3: opcode 61's plain (non `-y`) wording is "I am dead." and does not
// itself end the game; opcode 63 prints "The game is now over." and does
// (ScottCurses.c:873-891).
#[test]
fn op61_prints_i_am_dead_and_op63_prints_game_over_and_quits() {
    const DAT: &str = r#"
0 0 2 3 1 6 1 0 4 -1 0 0
0 0 0 0 0 0 0 0
300 0 0 0 0 0 9150 0
450 0 0 0 0 0 9450 0
"" ""
"" ""
"DIE" ""
"STOP" ""
0 0 0 0 0 0 "*limbo"
0 0 0 0 0 0 "*start"
""
"" 0
"#;
    let db = Database::parse(DAT).expect("parses");
    let mut vm = Vm::new(db);
    vm.take_output();

    vm.supply_line("die");
    vm.step();
    let out = vm.take_output();
    assert_eq!(out, "I am dead.\n", "op61's plain wording: {out:?}");
    assert!(!vm.has_quit(), "op61 alone does not end the game");

    vm.supply_line("stop");
    vm.step();
    let out = vm.take_output();
    assert_eq!(out, "The game is now over.\n", "op63's wording: {out:?}");
    assert!(vm.has_quit(), "op63 ends the game");
}

// Item 4: PerformActions (ScottCurses.c:1091-1133) — the dark-move warning
// prints whether or not the move succeeds; only a move with NO exit while
// dark ends the game.
#[test]
fn death_in_the_dark_matches_scottfree_wording_and_ends_the_game() {
    const DAT: &str = r#"
0 0 0 1 2 6 1 0 3 -1 0 0
100 0 0 0 0 0 8400 0
"" ""
"" "NORTH"
0 0 0 0 0 0 "*limbo"
2 0 0 0 0 0 "*start room"
0 0 0 0 0 0 "*dead end"
""
"" 0
"#;
    let db = Database::parse(DAT).expect("parses");
    let mut vm = Vm::new(db);
    vm.take_output(); // the opening occurrence already set the dark flag
    assert!(vm.is_dark(), "the opening occurrence set the dark flag with no light source");

    // Room 1 has a north exit: the warning prints, but the move still happens.
    vm.supply_line("north");
    vm.step();
    let out = vm.take_output();
    assert_eq!(
        out, "Dangerous to move in the dark!\n",
        "the warning prints even on a successful move: {out:?}"
    );
    assert_eq!(vm.current_room(), 2, "the move still happened");
    assert!(!vm.has_quit());

    // Room 2 is a dead end: no exit while dark is death.
    vm.supply_line("north");
    vm.step();
    let out = vm.take_output();
    assert_eq!(
        out, "Dangerous to move in the dark!\nI fell down and broke my neck.\n",
        "no exit while dark ends the game: {out:?}"
    );
    assert!(vm.has_quit(), "death in the dark ends the game");
}

// Item 5: the lamp's main-loop countdown (ScottCurses.c:1416-1450) — the
// "growing dim" warning fires on the `<25 && %5==0` turns, and the run-out
// warning fires on BOTH turns the live fuel crosses below 1 (0, then -1)
// before the `!= -1` guard stops the tick for good.
#[test]
fn lamp_countdown_dims_once_and_runs_out_exactly_twice() {
    const DAT: &str = r#"
0 9 0 2 1 6 1 0 4 6 0 0
300 0 0 0 0 0 0 0
"" ""
"" ""
"WAIT" ""
0 0 0 0 0 0 "*limbo"
0 0 0 0 0 0 "*start room"
""
"" 0
"" 0
"" 0
"" 0
"" 0
"" 0
"" 0
"" 0
"" 0
"a brass lamp" -1
"#;
    let db = Database::parse(DAT).expect("parses");
    let mut vm = Vm::new(db);
    vm.take_output();

    let mut dim_count = 0;
    let mut out_count = 0;
    for _ in 0..10 {
        vm.supply_line("wait");
        vm.step();
        let out = vm.take_output();
        dim_count += out.matches("Your light is growing dim.").count();
        out_count += out.matches("Your light has run out.").count();
    }
    assert_eq!(dim_count, 1, "the dim warning fires exactly once (fuel 5)");
    assert_eq!(out_count, 2, "the run-out warning fires exactly twice (fuel 0, then -1)");
    assert!(vm.flag(LAMP_EMPTY_FLAG), "the empty flag is set once the lamp runs out");
}

// Item 6a: ReadString's escaped-quote rule (`""` -> a literal `"`), CR
// stripping, and non-ASCII -> `?` — needed now that `Database::parse` reads
// raw bytes rather than a text file already decoded by the host's own
// locale (SQ-1412).
#[test]
fn escaped_quote_cr_and_non_ascii_bytes_in_a_quoted_string() {
    let mut src: Vec<u8> = Vec::new();
    src.extend_from_slice(b"0 0 0 0 0 6 0 0 3 -1 1 0\n");
    src.extend_from_slice(b"0 0 0 0 0 0 0 0\n");
    src.extend_from_slice(b"\"\" \"\"\n");
    src.extend_from_slice(b"0 0 0 0 0 0 \"*limbo\"\n");
    src.extend_from_slice(b"\"\"\n"); // message 0 (empty)
    src.push(b'"');
    src.extend_from_slice(b"Say \"\"hi\"\" here\r\nthen ");
    src.push(0xE9); // a non-ASCII byte
    src.extend_from_slice(b" end");
    src.push(b'"');
    src.push(b'\n');
    src.extend_from_slice(b"\"\" 0\n"); // item 0

    let db = Database::parse(&src).expect("parses raw bytes with escapes/CR/non-ASCII");
    assert_eq!(
        db.messages[1],
        "Say \"hi\" here\nthen ? end",
        "doubled-quote escape, CR strip, non-ASCII -> '?'"
    );
}

// Item 6b: the live case (Adventureland's own intro). Real commercial
// fixture — `stories/` is gitignored, so this skips vacuously when absent
// (worktrees lack it unless symlinked; see CLAUDE.md).
#[test]
fn adventureland_intro_prints_real_quotes_from_backtick_pairs() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/adv01.dat");
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let db = Database::parse(&bytes).expect("adv01.dat parses from raw bytes");
    let found = db.rooms.iter().any(|r| r.desc.contains("\"ADVENTURELAND\""))
        || db.messages.iter().any(|m| m.contains("\"ADVENTURELAND\""));
    assert!(found, "the backtick pair around ADVENTURELAND became real double quotes");
}

// Item 7: ScottFree stores CARRIED as 255 (`unsigned char`, `Scott.h`); this
// crate's live item_loc uses -1. Without normalising the item's ORIGINAL
// start_loc at load, condition 17 ("item still at its initial location")
// compares a live -1 against a start_loc still literally 255 the moment the
// item is carried again after being dropped, and wrongly reports "not at
// its initial location".
#[test]
fn item_start_location_255_normalizes_to_carried_and_survives_a_drop_take_cycle() {
    const DAT: &str = r#"
0 0 1 18 0 6 0 0 3 -1 2 0
0 0 0 0 0 0 0 0
1650 17 0 0 0 0 300 0
"" ""
"" ""
"" ""
"" ""
"" ""
"" ""
"" ""
"" "WIDGET"
"" ""
"" ""
"GET" ""
"CHECK" ""
"" ""
"" ""
"" ""
"" ""
"" ""
"" ""
"DROP" ""
0 0 0 0 0 0 "*start room"
""
""
"AT START"
"a widget/WID/" 255
"#;
    let db = Database::parse(DAT).expect("parses");
    assert_eq!(db.items[0].start_loc, -1, "255 normalizes to CARRIED (-1) at load");

    let mut vm = Vm::new(db);
    vm.take_output();

    vm.supply_line("check");
    vm.step();
    assert!(
        vm.take_output().contains("AT START"),
        "starts carried, at its initial location"
    );

    vm.supply_line("drop widget");
    vm.step();
    vm.take_output();
    vm.supply_line("check");
    vm.step();
    assert!(
        !vm.take_output().contains("AT START"),
        "dropped: no longer at its initial (carried) location"
    );

    vm.supply_line("get widget");
    vm.step();
    vm.take_output();
    vm.supply_line("check");
    vm.step();
    assert!(
        vm.take_output().contains("AT START"),
        "carried again: back at its initial location — fails without the 255->-1 normalisation"
    );
}

// Item 8: auto-noun extraction (ScottCurses.c:319-327) starts at the FIRST
// `/`, not the last, tolerates a missing close, and honours `//`/`/*` as
// "no autoget word" markers that leave the display text untouched.
#[test]
fn auto_noun_extraction_matches_scottfree_first_slash_and_marker_conventions() {
    const DAT: &str = r#"
0 3 0 0 0 6 0 0 3 -1 0 0
0 0 0 0 0 0 0 0
"" ""
0 0 0 0 0 0 "*start room"
""
"Luger/LUGER/GUN/" 0
"candle//" 0
"candle/*" 0
"torch/TORCH" 0
"#;
    let db = Database::parse(DAT).expect("parses");

    // FIRST slash pair wins: display "Luger", bind "LUGER" — not the old
    // rfind-based display "Luger/LUGER" / bind "GUN" (secret.dat item 33).
    assert_eq!(db.items[0].text, "Luger");
    assert_eq!(db.items[0].auto_noun.as_deref(), Some("LUGER"));

    // "//" means "no autoget word": ScottFree skips the split entirely, so
    // the display text keeps its literal trailing "//".
    assert_eq!(db.items[1].text, "candle//");
    assert_eq!(db.items[1].auto_noun, None);

    // Same for "/*".
    assert_eq!(db.items[2].text, "candle/*");
    assert_eq!(db.items[2].auto_noun, None);

    // A missing closing slash is tolerated: the noun runs to the end.
    assert_eq!(db.items[3].text, "torch");
    assert_eq!(db.items[3].auto_noun.as_deref(), Some("TORCH"));
}
