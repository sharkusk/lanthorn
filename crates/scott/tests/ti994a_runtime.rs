//! The runtime behaviours that make a TI-99/4A release play differently
//! from a reference-format one (SQ-1414).
//!
//! `docs/internals/scott-dialects-spec.md` §9 states each of these "as
//! behaviour observable from outside the program, with a test that would
//! catch getting it wrong", and this file is those tests. Every case builds
//! its own tiny [`Database`] by hand — no game file, nothing fetched,
//! nothing to skip over — so all of it runs on CI, where the specimens never
//! exist.
//!
//! The section each case implements is named in its doc comment. Where §9
//! contrasts this dialect with the reference format, the case runs BOTH and
//! asserts they differ, because "the tokenised one does X" is only half the
//! claim.

use scott::{
    Action, Condition, Database, Item, Options, Presentation, Room, StepResult, Ti99Record,
    Ti99Script, Vm,
};

// ── opcode names, so the byte streams below read as programs ──────────────
// Conditions (spec §3.7): each takes one operand byte except the two noted.
const C_ITEM_CARRIED: u8 = 183;
const C_ITEM_HERE: u8 = 184;
const C_IN_ROOM: u8 = 191;
const C_FLAG_SET: u8 = 193;
const C_FLAG_CLEAR: u8 = 194;
const C_CARRYING_SOMETHING: u8 = 195; // no operand
// Commands (spec §3.7).
const X_INVENTORY_ON: u8 = 214;
const X_INVENTORY_OFF: u8 = 215;
const X_ON_FAIL: u8 = 218; // push a failure handler
const X_TAKE_LIMITED: u8 = 219;
const X_DROP: u8 = 220;
const X_GOTO: u8 = 221;
const X_SET_FLAG: u8 = 225;
const X_CLEAR_FLAG: u8 = 226;
const X_KILL: u8 = 229;
const X_PUT_ITEM_IN_ROOM: u8 = 230; // room FIRST, then item
const X_END_GAME: u8 = 231;
const X_TAKE_UNLIMITED: u8 = 237;
const X_COUNTER_UP: u8 = 242;
const X_UNASSIGNED: u8 = 205; // spec §11: 202-211 and 213 have no meaning
const END: u8 = 255; // end of record, and the record succeeds

/// Message indices the fixture below uses, so an assertion reads as text.
const M_A: u8 = 1;
const M_B: u8 = 2;
const M_C: u8 = 3;
const M_D: u8 = 4;

fn rec(key: u8, ops: &[u8]) -> Ti99Record {
    Ti99Record {
        key,
        ops: ops.to_vec(),
    }
}

/// A two-room, three-item world with a five-word vocabulary, which every
/// case below then gives its own script.
///
/// Verb 1 is `GO` and verb 10 is `GET` and verb 18 is `DROP`, matching the
/// numbering both this dialect and the reference format use for the built-in
/// handling spec §9.1 says still applies. Verb 20 is a spare the cases key
/// their own chains to.
fn world(script: Ti99Script) -> Database {
    let mut verbs = vec![String::new(); 21];
    verbs[0] = "AUTO".into();
    verbs[1] = "GO".into();
    verbs[10] = "GET".into();
    verbs[18] = "DROP".into();
    verbs[20] = "RUB".into();
    let mut nouns = vec![String::new(); 21];
    nouns[0] = "ANY".into();
    nouns[1] = "NORTH".into();
    nouns[2] = "SOUTH".into();
    nouns[5] = "LAMP".into();
    nouns[6] = "COIN".into();
    Database {
        max_carry: 4,
        start_room: 1,
        num_treasures: 0,
        // 4, as six of the twelve real releases declare. NOT 0: spec §2.5
        // says a loader should read 0 as "compare the whole word", and
        // `Database::match_verb` does, but `Vm`'s own auto-get/drop matcher
        // truncates to `word_length` unconditionally and so matches nothing
        // at 0. No TI-99/4A release has a word length of 0, so that
        // pre-existing inconsistency is not this dialect's to fix — it is
        // just not something to build a fixture on.
        word_length: 4,
        light_time: -1,
        treasure_room: 0,
        actions: Vec::new(),
        verbs,
        nouns,
        rooms: vec![
            Room { exits: [0; 6], desc: "limbo".into(), literal: true },
            // Room 1 goes north to room 2.
            Room { exits: [2, 0, 0, 0, 0, 0], desc: "study".into(), literal: true },
            Room { exits: [0, 1, 0, 0, 0, 0], desc: "attic".into(), literal: true },
        ],
        messages: vec![
            ".".into(),
            "MESSAGE-A".into(),
            "MESSAGE-B".into(),
            "MESSAGE-C".into(),
            "MESSAGE-D".into(),
        ],
        items: vec![
            Item { text: "nothing".into(), treasure: false, auto_noun: None, start_loc: 0 },
            // Item 1 sits in room 1 and answers to LAMP.
            Item {
                text: "a lamp".into(),
                treasure: false,
                auto_noun: Some("LAMP".into()),
                start_loc: 1,
            },
            // Item 2 sits in room 1 and answers to COIN.
            Item {
                text: "a coin".into(),
                treasure: false,
                auto_noun: Some("COIN".into()),
                start_loc: 1,
            },
        ],
        adventure_number: 0,
        mysterious: false,
        ti99: Some(script),
    }
}

/// A script whose only explicit chain belongs to verb 20.
fn verb20(records: Vec<Ti99Record>) -> Ti99Script {
    let mut verb_chains = vec![Vec::new(); 21];
    verb_chains[20] = records;
    Ti99Script { verb_chains, automatic: Vec::new() }
}

/// Drives one command and returns everything printed for it, with the
/// opening turn's output discarded first.
fn play(db: Database, commands: &[&str]) -> String {
    let mut vm = Vm::new_full(db, false, 12_345, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _opening = vm.take_output();
    let mut out = String::new();
    for c in commands {
        vm.supply_line(c);
        vm.step();
        out.push_str(&vm.take_output());
    }
    out
}

// ── §9.1: automatic actions are independent ───────────────────────────────

/// Spec §9.1, first test: "three automatic records at 100%, each printing a
/// distinct message, the second failing a condition after printing. All
/// three messages appear on the first turn, in order; under the reference
/// format's continuation semantics an equivalent construction would not
/// produce the third."
///
/// Success or failure has no effect on whether later records are visited:
/// there is no early exit, no chaining, and no continuation opcode.
#[test]
fn automatic_records_are_independent_and_a_failure_does_not_stop_the_pass() {
    let script = Ti99Script {
        verb_chains: vec![Vec::new(); 21],
        automatic: vec![
            rec(100, &[M_A, END]),
            // Prints, THEN fails a condition that cannot hold (flag 7 is
            // clear), so it never reaches END.
            rec(100, &[M_B, C_FLAG_SET, 7, END]),
            rec(100, &[M_C, END]),
        ],
    };
    let mut vm = Vm::new_full(world(script), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let opening = vm.take_output();
    assert!(opening.contains("MESSAGE-A"), "{opening:?}");
    assert!(opening.contains("MESSAGE-B"), "{opening:?}");
    assert!(
        opening.contains("MESSAGE-C"),
        "the third record must still run after the second failed: {opening:?}"
    );
    let (a, b, c) = (
        opening.find("MESSAGE-A").unwrap(),
        opening.find("MESSAGE-B").unwrap(),
        opening.find("MESSAGE-C").unwrap(),
    );
    assert!(a < b && b < c, "in order: {opening:?}");
}

/// A percentage key of 0 never fires and one of 100 always does — the roll
/// is against the record's own first byte (spec §3.7).
#[test]
fn an_automatic_records_percentage_key_gates_it() {
    let script = Ti99Script {
        verb_chains: vec![Vec::new(); 21],
        automatic: vec![rec(0, &[M_A, END]), rec(100, &[M_B, END])],
    };
    let mut vm = Vm::new_full(world(script), false, 99, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let out = vm.take_output();
    assert!(!out.contains("MESSAGE-A"), "a 0% record never fires: {out:?}");
    assert!(out.contains("MESSAGE-B"), "a 100% record always fires: {out:?}");
}

// ── §9.1: three dispatch outcomes ─────────────────────────────────────────

/// Spec §9.1, second test: "define a verb whose chain holds only a record
/// keyed to noun 5 with a condition that can never hold; typing it with noun
/// 5 must give the 'can't do that yet' wording and with noun 6 the 'don't
/// understand' wording."
#[test]
fn matched_but_blocked_and_never_matched_produce_different_wordings() {
    let script = verb20(vec![rec(5, &[C_FLAG_SET, 9, M_A, END])]);
    let blocked = play(world(script.clone()), &["rub lamp"]);
    assert!(
        blocked.contains("I can't do that yet."),
        "noun 5 matched a record whose condition failed: {blocked:?}"
    );
    assert!(!blocked.contains("MESSAGE-A"), "{blocked:?}");

    let unmatched = play(world(script), &["rub coin"]);
    assert!(
        unmatched.contains("I don't understand the command."),
        "noun 6 matched no record at all: {unmatched:?}"
    );
}

/// A verb with no chain at all is the third outcome, and reads the same as
/// having matched nothing (spec §9.1).
#[test]
fn a_verb_with_no_chain_reports_that_it_is_not_understood() {
    let out = play(world(verb20(Vec::new())), &["rub lamp"]);
    assert!(out.contains("I don't understand the command."), "{out:?}");
}

/// A failed record does not end the search: the walk continues, and a LATER
/// record in the same chain can still succeed (spec §3.7, "Record
/// selection").
#[test]
fn a_failed_record_does_not_end_the_chain_walk() {
    let script = verb20(vec![
        rec(0, &[C_FLAG_SET, 9, M_A, END]), // matches anything, always fails
        rec(0, &[M_B, END]),                // …so this one runs
    ]);
    let out = play(world(script), &["rub lamp"]);
    assert!(!out.contains("MESSAGE-A"), "{out:?}");
    assert!(out.contains("MESSAGE-B"), "{out:?}");
    assert!(!out.contains("can't do that yet"), "it succeeded: {out:?}");
}

/// A record with a genuinely empty opcode stream can never reach END, so it
/// always fails — which is observable, because it makes its chain answer "I
/// can't do that yet." rather than "I don't understand". This is a VM
/// boundary case, not what the loader produces for a real link-0 record: a
/// link (byte 1) of zero means only that no record follows, and that
/// record's own stream begins at byte 2 like any other's and is recovered by
/// walking arities to its own END — see `ti994a.rs`'s
/// `a_link_zero_records_real_opcode_stream_runs_through_the_vm`, which is the
/// case for that.
#[test]
fn a_record_with_a_truly_empty_opcode_stream_still_matches_and_still_fails() {
    let script = verb20(vec![rec(0, &[])]);
    let out = play(world(script), &["rub lamp"]);
    assert!(out.contains("I can't do that yet."), "{out:?}");
}

// ── §9.1: interleaving, and §9.4's contrast with the reference format ─────

/// Spec §9.1, third test: "a record that sets bit flag 4, then fails a
/// condition, then ends. The action reports failure **and** flag 4 is set,
/// observable through a second verb guarded on it."
///
/// And spec §9.4's contrast, run in the same case because half the claim is
/// that the reference format answers differently: there, "all five
/// conditions of a line are evaluated before any of its four commands runs,
/// so a line that fails changes nothing."
#[test]
fn conditions_interleave_and_side_effects_before_a_failure_persist() {
    let script = verb20(vec![rec(0, &[X_SET_FLAG, 4, C_FLAG_SET, 9, M_A, END])]);
    let mut vm = Vm::new_full(world(script), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("rub lamp");
    vm.step();
    let out = vm.take_output();
    assert!(out.contains("I can't do that yet."), "it failed: {out:?}");
    assert!(vm.flag(4), "and the flag it set before failing is still set");

    // The same logic in the reference format: condition 8 (flag set) on
    // flag 9, command 58 (set flag) with operand 4. Conditions first, so
    // nothing happens at all.
    let mut db = world(Ti99Script::default());
    db.ti99 = None;
    db.actions = vec![Action {
        verb: 20,
        noun: 0,
        conditions: [
            Condition { code: 8, value: 9 },
            Condition { code: 0, value: 4 },
            Condition { code: 0, value: 0 },
            Condition { code: 0, value: 0 },
            Condition { code: 0, value: 0 },
        ],
        commands: [58, 0, 0, 0],
    }];
    let mut vm = Vm::new_full(db, false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("rub lamp");
    vm.step();
    assert!(
        !vm.flag(4),
        "the reference format evaluates every condition first, so a failed \
         line changes nothing — this is the difference spec §9.4 names"
    );
}

// ── §9.1: failure handlers ────────────────────────────────────────────────

/// Spec §9.1, fourth test: "a record of handler marker targeting a 'print
/// message B' opcode, a failing condition, 'print message A', end, 'print
/// message B', end. Message B is printed and the record succeeds; flip the
/// condition to one that holds and message A is printed instead."
///
/// The handler's operand encodes its target as an offset from the operand
/// byte's own position within the opcode stream (spec §3.7).
#[test]
fn a_failure_handler_is_an_if_else() {
    // positions:  0            1  2            3  4    5    6    7
    //             X_ON_FAIL    d  C_FLAG_SET   9  M_A  END  M_B  END
    // The operand byte sits at position 1 and the else-branch begins at
    // position 6, so the operand value is 6 - 1 = 5.
    let else_arm = |cond: u8, operand: u8| {
        verb20(vec![rec(
            0,
            &[X_ON_FAIL, 5, cond, operand, M_A, END, M_B, END],
        )])
    };
    // Flag 9 is clear, so the condition fails and the handler resumes at
    // MESSAGE-B, which then reaches END and succeeds.
    let taken = play(world(else_arm(C_FLAG_SET, 9)), &["rub lamp"]);
    assert!(taken.contains("MESSAGE-B"), "{taken:?}");
    assert!(!taken.contains("MESSAGE-A"), "{taken:?}");
    assert!(!taken.contains("can't do that yet"), "it succeeded: {taken:?}");

    // Flip the condition to one that holds (flag 9 is clear): the then-arm
    // runs and its END clears the handler stack, so the else-arm is
    // unreachable — a handler can only ever be entered by failure, never by
    // falling into it.
    let not_taken = play(world(else_arm(C_FLAG_CLEAR, 9)), &["rub lamp"]);
    assert!(not_taken.contains("MESSAGE-A"), "{not_taken:?}");
    assert!(!not_taken.contains("MESSAGE-B"), "{not_taken:?}");
}

// ── §9.1: built-in handling survives ──────────────────────────────────────

/// Spec §9.1: "The built-in take, drop and go handling still applies when a
/// verb chain yields no success… *Test:* with no records for the take verb
/// at all, taking a linked noun still moves the item to the inventory and
/// acknowledges it."
#[test]
fn built_in_take_and_drop_still_apply_with_no_records_for_the_verb() {
    let mut vm = Vm::new_full(world(verb20(Vec::new())), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("get lamp");
    vm.step();
    let out = vm.take_output();
    assert_eq!(vm.item_loc(1), -1, "the lamp is carried: {out:?}");
    assert!(out.contains("OK. "), "and acknowledged in this dialect's wording: {out:?}");
    vm.supply_line("drop lamp");
    vm.step();
    let out = vm.take_output();
    assert_eq!(vm.item_loc(1), 1, "back in the room: {out:?}");
    assert!(out.contains("OK. "), "{out:?}");
}

/// Spec §9.1: "Movement is acknowledged. A successful compass move prints
/// `OK. ` before the new room is described." No ScottFree-derived wording
/// prints anything at all there, so the same case run under the reference
/// presentation must stay silent.
#[test]
fn a_successful_move_is_acknowledged_here_and_silent_in_the_reference_set() {
    let mut vm = Vm::new_full(world(verb20(Vec::new())), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("north");
    vm.step();
    let out = vm.take_output();
    assert_eq!(vm.current_room(), 2);
    assert!(out.contains("OK. "), "{out:?}");

    let mut db = world(Ti99Script::default());
    db.ti99 = None;
    let mut vm = Vm::new_full(
        db,
        false,
        1,
        Options::new().with_presentation(Presentation::ScottFree),
    );
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("north");
    vm.step();
    assert_eq!(vm.take_output(), "", "the reference set acknowledges nothing");
}

// ── §9.1: death is a side effect ──────────────────────────────────────────

/// Spec §9.1: "Death is a side effect, not a return… *Test:* a record of
/// kill, then 'print message C', then end. Message C is printed after the
/// death text." Only the end-the-game opcode stops a record immediately.
#[test]
fn the_kill_opcode_lets_the_rest_of_the_record_run_and_end_game_does_not() {
    let killed = play(
        world(verb20(vec![rec(0, &[X_KILL, M_C, END])])),
        &["rub lamp"],
    );
    assert!(killed.contains("I'm dead... "), "{killed:?}");
    assert!(
        killed.contains("MESSAGE-C"),
        "the rest of the record still runs: {killed:?}"
    );
    assert!(
        killed.find("I'm dead").unwrap() < killed.find("MESSAGE-C").unwrap(),
        "and runs AFTER the death text: {killed:?}"
    );

    let ended = play(
        world(verb20(vec![rec(0, &[X_END_GAME, M_C, END])])),
        &["rub lamp"],
    );
    assert!(
        !ended.contains("MESSAGE-C"),
        "the end-the-game opcode stops the record where it stands: {ended:?}"
    );
}

/// The kill opcode moves the player to the highest-numbered room — this
/// dialect's header spells the room count as the "red room" a dead player is
/// moved to, which is the same number (spec §3.3).
#[test]
fn the_kill_opcode_moves_the_player_to_the_red_room() {
    let mut vm = Vm::new_full(
        world(verb20(vec![rec(0, &[X_KILL, END])])),
        false,
        1,
        Options::new(),
    );
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("rub lamp");
    vm.step();
    assert_eq!(vm.current_room(), 2, "the highest-numbered room");
}

// ── §9.1: automatic inventory ─────────────────────────────────────────────

/// Spec §9.1: "Automatic inventory is on by default… toggled by two opcodes,
/// and its current state must survive save and restore."
#[test]
fn automatic_inventory_defaults_on_toggles_and_survives_a_save_and_restore() {
    let script = verb20(vec![
        rec(5, &[X_INVENTORY_OFF, END]),
        rec(6, &[X_INVENTORY_ON, END]),
    ]);
    let mut vm = Vm::new_full(world(script), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    assert!(vm.auto_inventory(), "on by default");

    vm.supply_line("rub lamp");
    vm.step();
    assert!(!vm.auto_inventory(), "opcode 215 turned it off");

    let save = vm.snapshot();
    vm.supply_line("rub coin");
    vm.step();
    assert!(vm.auto_inventory(), "opcode 214 turned it back on");

    vm.restore(&save).expect("restores");
    assert!(
        !vm.auto_inventory(),
        "the saved state had it off, so the restore must bring that back"
    );
}

/// A database from any other dialect has no opcode that can change it, so it
/// always answers `true`.
#[test]
fn a_reference_format_database_always_reports_automatic_inventory_on() {
    let mut db = world(Ti99Script::default());
    db.ti99 = None;
    let vm = Vm::new_full(db, false, 1, Options::new());
    assert!(vm.auto_inventory());
}

// ── §9.2: the lamp, and §9.1's message set ────────────────────────────────

/// Spec §9.2: "every Mysterious Adventures release and every TI-99/4A
/// release forces both on" — the countdown wording and the prehistoric
/// lamp — and spec Appendix A: the §9 differences "are properties of the
/// *database*, not of the host". So a host that explicitly asks for neither
/// still gets both.
#[test]
fn a_ti99_database_forces_both_lamp_options_and_its_own_message_set() {
    let asked_for_nothing = Options::new()
        .with_scott_light(false)
        .with_prehistoric_lamp(false)
        .with_presentation(Presentation::C64);
    let vm = Vm::new_full(world(verb20(Vec::new())), false, 1, asked_for_nothing);
    assert!(vm.options().scott_light, "spec §9.2 forces the countdown");
    assert!(vm.options().prehistoric_lamp, "spec §9.2 forces the destroy");
    assert_eq!(
        vm.options().presentation,
        Presentation::Ti994a,
        "spec §9.1: the message set is this dialect's own"
    );

    // A reference-format database in the same call keeps what the host asked
    // for — the override is the database's, not a global.
    let mut db = world(Ti99Script::default());
    db.ti99 = None;
    let vm = Vm::new_full(db, false, 1, asked_for_nothing);
    assert!(!vm.options().scott_light);
    assert!(!vm.options().prehistoric_lamp);
    assert_eq!(vm.options().presentation, Presentation::C64);
}

/// Spec §9.1's message set, spot-checked on the strings it pins literally.
/// "any interpreter emitting 'Taken.' or 'Exits: ' is not running this set."
#[test]
fn the_message_set_is_this_dialects_own() {
    let mut vm = Vm::new_full(world(verb20(Vec::new())), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();

    let block = vm.room_block();
    assert!(block.contains("Obvious exits : "), "{block:?}");
    assert!(block.contains("Visible items are : "), "{block:?}");
    assert!(!block.contains("Exits: "), "{block:?}");
    // Two items in the room, joined by this dialect's delimiter.
    assert!(block.contains("a lamp, a coin"), "{block:?}");

    // A non-literal room gets "I am in a ", not "I'm in a ".
    let mut db = world(verb20(Vec::new()));
    db.rooms[1].literal = false;
    let mut vm2 = Vm::new_full(db, false, 1, Options::new());
    assert_eq!(vm2.step(), StepResult::NeedLine);
    let _ = vm2.take_output();
    let block2 = vm2.room_block();
    assert!(block2.contains("I am in a study."), "{block2:?}");

    // The inventory heading and its trailing ". " rather than ".\n".
    vm.supply_line("get lamp");
    vm.step();
    let _ = vm.take_output();
    vm.supply_line("rub lamp"); // no chain: falls through, so ask directly
    vm.step();
    let _ = vm.take_output();
    let mut vm3 = Vm::new_full(
        world(verb20(vec![rec(0, &[233, END])])), // 233 = list the inventory
        false,
        1,
        Options::new(),
    );
    assert_eq!(vm3.step(), StepResult::NeedLine);
    let _ = vm3.take_output();
    vm3.supply_line("rub lamp");
    vm3.step();
    let inv = vm3.take_output();
    assert!(inv.contains("I am carrying : "), "{inv:?}");
    assert!(inv.contains("Nothing. "), "{inv:?}");
    assert!(!inv.contains("I'm carrying"), "{inv:?}");
}

// ── §3.7: the operand-order and opcode facts that are easy to get wrong ───

/// Spec §3.7: "Opcode 230 takes the room first and the item second, the
/// reverse of the reference format's equivalent." Getting it backwards puts
/// the wrong object somewhere plausible, so the case pins the direction.
#[test]
fn opcode_230_takes_the_room_first_and_the_item_second() {
    // Put item 2 (the coin) into room 2.
    let script = verb20(vec![rec(0, &[X_PUT_ITEM_IN_ROOM, 2, 2, END])]);
    let mut vm = Vm::new_full(world(script), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("rub lamp");
    vm.step();
    assert_eq!(vm.item_loc(2), 2, "the coin moved to room 2");

    // Now the operands the other way round: room 1, item 1. If the loader
    // read them item-first this would be indistinguishable, so use a pair
    // where only one order is even in range — room 9 does not exist.
    let script = verb20(vec![rec(0, &[X_PUT_ITEM_IN_ROOM, 9, 1, END])]);
    let mut vm = Vm::new_full(world(script), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("rub lamp");
    vm.step();
    assert_eq!(
        vm.item_loc(1),
        1,
        "room 9 is out of range, so nothing moved — read item-first, item 9 \
         would have been out of range instead and the lamp would have moved \
         to room 1 regardless, which is why the rooms differ here"
    );
}

/// Spec §3.7: the two zero-operand conditions "break any assumption that a
/// condition always carries an operand" — a reader that consumed one anyway
/// would run the following opcode as data.
#[test]
fn the_zero_operand_conditions_do_not_swallow_the_next_opcode() {
    // Carrying nothing at the start, so C_CARRYING_SOMETHING fails and the
    // record must NOT print. If the condition wrongly ate the M_A byte as an
    // operand, the stream would decode differently.
    let script = verb20(vec![rec(0, &[C_CARRYING_SOMETHING, M_A, END])]);
    let out = play(world(script), &["rub lamp"]);
    assert!(!out.contains("MESSAGE-A"), "{out:?}");

    // Take something first, and the same record prints.
    let script = verb20(vec![
        rec(5, &[X_TAKE_UNLIMITED, 1, END]),
        rec(6, &[C_CARRYING_SOMETHING, M_A, END]),
    ]);
    let out = play(world(script), &["rub lamp", "rub coin"]);
    assert!(out.contains("MESSAGE-A"), "{out:?}");
}

/// Spec §11: "Opcode bytes 202-211 and 213 are unassigned and their operand
/// counts are unknown, so encountering one makes the rest of the record
/// undecodable: abandon the record rather than skipping the byte as a
/// no-op." The commands before it have already run; the ones after it must
/// not.
#[test]
fn an_unassigned_opcode_abandons_the_rest_of_the_record() {
    let script = verb20(vec![rec(0, &[M_A, X_UNASSIGNED, M_B, END])]);
    let out = play(world(script), &["rub lamp"]);
    assert!(out.contains("MESSAGE-A"), "what ran before it still ran: {out:?}");
    assert!(!out.contains("MESSAGE-B"), "the rest is undecodable: {out:?}");
    assert!(
        out.contains("I can't do that yet."),
        "and the record failed: {out:?}"
    );
}

/// Spec §3.7: opcode 219 takes an item "respecting the carry limit; on
/// refusal print the 'carrying too much' message and fail the record" —
/// the one command that can end a record in failure. Opcode 237 ignores the
/// limit.
#[test]
fn the_limited_take_can_fail_a_record_and_the_unlimited_one_cannot() {
    let mut db = world(verb20(vec![
        rec(5, &[X_TAKE_LIMITED, 2, M_A, END]),
        rec(6, &[X_TAKE_UNLIMITED, 2, M_B, END]),
    ]));
    db.max_carry = 0; // already at capacity, holding nothing
    let out = play(db, &["rub lamp"]);
    assert!(out.contains("I am carrying too much."), "{out:?}");
    assert!(!out.contains("MESSAGE-A"), "the record failed there: {out:?}");

    let mut db = world(verb20(vec![rec(0, &[X_TAKE_UNLIMITED, 2, M_B, END])]));
    db.max_carry = 0;
    let out = play(db, &["rub lamp"]);
    assert!(out.contains("MESSAGE-B"), "the unlimited take ignores it: {out:?}");
}

/// Spec §3.7's counter opcodes: 242 INCREMENTS, which the reference format
/// has no equivalent for at all, and 243 decrements floored at 0 rather than
/// at the reference format's -1.
#[test]
fn the_counter_increments_and_its_decrement_floors_at_zero() {
    let script = verb20(vec![
        rec(5, &[X_COUNTER_UP, X_COUNTER_UP, END]),
        rec(6, &[243, 243, 243, 243, 243, END]),
    ]);
    let mut vm = Vm::new_full(world(script), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("rub lamp");
    vm.step();
    assert_eq!(vm.counter(), 2, "242 increments");
    vm.supply_line("rub coin");
    vm.step();
    assert_eq!(vm.counter(), 0, "243 floors at 0, not at -1");
}

/// A record that matches noun 0 matches any noun, including the sentinel a
/// command with no second word produces (spec §3.7).
#[test]
fn a_noun_zero_record_matches_a_command_with_no_noun_at_all() {
    let script = verb20(vec![rec(0, &[M_A, END])]);
    let out = play(world(script), &["rub"]);
    assert!(out.contains("MESSAGE-A"), "{out:?}");
}

/// The other commands that move things about, checked against the item
/// locations they produce rather than against any message.
#[test]
fn the_item_moving_commands_land_where_spec_3_7_says() {
    let script = verb20(vec![
        // Take the lamp, drop it in room 2 by going there first.
        rec(5, &[X_TAKE_UNLIMITED, 1, X_GOTO, 2, X_DROP, 1, END]),
        // Clear a flag we set, to check 225/226 are not the same opcode.
        rec(6, &[X_SET_FLAG, 3, X_CLEAR_FLAG, 3, C_IN_ROOM, 2, END]),
    ]);
    let mut vm = Vm::new_full(world(script), false, 1, Options::new());
    assert_eq!(vm.step(), StepResult::NeedLine);
    let _ = vm.take_output();
    vm.supply_line("rub lamp");
    vm.step();
    assert_eq!(vm.current_room(), 2);
    assert_eq!(vm.item_loc(1), 2, "dropped where the player now is");
    vm.supply_line("rub coin");
    vm.step();
    assert!(!vm.flag(3), "225 set it and 226 cleared it");
}

/// The conditions that read item location, spot-checked so a transposed row
/// of spec §3.7's condition table would fail.
#[test]
fn the_item_location_conditions_read_the_right_way_round() {
    // 183 is "carried", 184 is "in the current room" — the lamp starts in
    // room 1 and is not carried, so 184 holds and 183 does not.
    let here = play(world(verb20(vec![rec(0, &[C_ITEM_HERE, 1, M_A, END])])), &["rub lamp"]);
    assert!(here.contains("MESSAGE-A"), "{here:?}");
    let carried = play(
        world(verb20(vec![rec(0, &[C_ITEM_CARRIED, 1, M_D, END])])),
        &["rub lamp"],
    );
    assert!(!carried.contains("MESSAGE-D"), "{carried:?}");
}
