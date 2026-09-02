//! Scott Adams debug-inspector adapter (SQ-0464): implements the engine-neutral
//! [`Debugger`] trait for the `scott` VM, on top of the `scott::decompile` API.
//!
//! **Addressing.** Scott has no program counter — its "code" is the fixed
//! *action table*. So a debug "address" here is an **action index**: the
//! Disassembly section lists one summary line per action (`{index:06x}  VERB
//! NOUN …`), `next_instr`/`prev_instr` step by ±1 action, and the runtime
//! coverage sets ([`executed_pcs`](Debugger::executed_pcs) /
//! [`ever_executed_pcs`](Debugger::ever_executed_pcs)) are just the fired-action
//! index sets cast to `u32`. Because the renderer keys its tier colouring off
//! each line's leading 6-hex field, an action-index prefix makes the existing
//! executed/ever colouring and the PC divider work unchanged.
//!
//! **Sections.** Call Stack / Eval Stack / Memory are meaningless for Scott, so
//! [`sections`](Debugger::sections) hands the panel a Scott-specific 3-window
//! layout that reuses the plain-list slots: Disassembly→Actions,
//! Globals→State, Objects→Items, Dictionary→Vocab, Locals→World (rooms +
//! messages). [`section_label`](Debugger::section_label) relabels them.
//!
//! **Cost.** Every method here is cold: nothing runs until the panel calls in
//! (i.e. only while the inspector is open). Construction and normal play touch
//! none of it.

use std::collections::HashSet;

use scott::{decompile_action, list_items, list_rooms, list_vocab, Vm, CARRIED, DARK_FLAG, LAMP_EMPTY_FLAG};

use crate::debug_panel::Section;
use crate::engine::Debugger;

/// The Scott inspector's 3-window tab layout (see the module docs): only the
/// plain-list / disasm-capable sections, no Call Stack / Eval Stack / Memory.
pub const SCOTT_TABS: [&[Section]; 3] = [
    &[Section::Disasm],
    &[Section::Globals, Section::Objects, Section::Dict],
    &[Section::Locals],
];

/// Human name of a flag index, naming the two the engine treats specially.
fn flag_name(idx: usize) -> String {
    match idx {
        DARK_FLAG => "dark".to_string(),
        LAMP_EMPTY_FLAG => "lamp_empty".to_string(),
        other => other.to_string(),
    }
}

/// The `✗cond{slot}` blocked suffix for action `idx` this turn, if the player's
/// verb/noun matched it but a condition failed. Empty otherwise.
fn blocked_suffix(vm: &Vm, idx: usize) -> String {
    vm.last_blocked()
        .iter()
        .find(|&&(a, _)| a == idx)
        .map(|&(_, slot)| format!("  ✗cond{slot}"))
        .unwrap_or_default()
}

/// One-line **Full** summary of action `idx`: resolved verb/noun, resolved
/// conditions, resolved commands, plus the fired-this-turn blocked suffix.
fn summary_full(vm: &Vm, idx: usize) -> String {
    let db = vm.database();
    let Some(d) = decompile_action(db, idx) else { return format!("{idx:06x}  (no action)") };
    let mut s = format!("{idx:06x}  {} {}", d.verb_word, d.noun_word);
    if !d.conditions.is_empty() {
        let conds: Vec<String> = d
            .conditions
            .iter()
            .map(|c| match &c.operand {
                Some(op) => format!("{}({op})", c.mnemonic),
                None => c.mnemonic.to_string(),
            })
            .collect();
        s.push_str(&format!("  if {}", conds.join(", ")));
    }
    if !d.commands.is_empty() {
        let cmds: Vec<String> = d
            .commands
            .iter()
            .map(|c| match &c.operand {
                Some(op) => format!("{}({op})", c.mnemonic),
                None => c.mnemonic.to_string(),
            })
            .collect();
        s.push_str(&format!("  -> {}", cmds.join("; ")));
    }
    s.push_str(&blocked_suffix(vm, idx));
    s
}

/// One-line **Basic** summary: mnemonics kept, but operands shown as raw numbers
/// (no item/room/message names) and the trigger as `V{verb} N{noun}`.
fn summary_basic(vm: &Vm, idx: usize) -> String {
    let db = vm.database();
    let Some(a) = db.actions.get(idx) else { return format!("{idx:06x}  (no action)") };
    let d = decompile_action(db, idx);
    let mut s = format!("{idx:06x}  V{} N{}", a.verb, a.noun);
    if let Some(d) = &d {
        if !d.conditions.is_empty() {
            let conds: Vec<String> =
                d.conditions.iter().map(|c| format!("{}({})", c.mnemonic, c.value)).collect();
            s.push_str(&format!("  if {}", conds.join(", ")));
        }
        if !d.commands.is_empty() {
            let cmds: Vec<String> = d.commands.iter().map(|c| c.mnemonic.to_string()).collect();
            s.push_str(&format!("  -> {}", cmds.join("; ")));
        }
    }
    s.push_str(&blocked_suffix(vm, idx));
    s
}

/// One-line **Raw** summary: the numeric verb/noun/condition/command tuples,
/// no mnemonics or lookups at all.
fn summary_raw(vm: &Vm, idx: usize) -> String {
    let db = vm.database();
    let Some(a) = db.actions.get(idx) else { return format!("{idx:06x}  (no action)") };
    let conds: Vec<String> = a
        .conditions
        .iter()
        .filter(|c| !(c.code == 0 && c.value == 0))
        .map(|c| format!("{}/{}", c.code, c.value))
        .collect();
    let cmds: Vec<String> = a.commands.iter().filter(|&&n| n != 0).map(|n| n.to_string()).collect();
    format!(
        "{idx:06x}  v{} n{} c[{}] k[{}]{}",
        a.verb,
        a.noun,
        conds.join(","),
        cmds.join(","),
        blocked_suffix(vm, idx),
    )
}

/// The "World" section: rooms (with exits) then the message table.
fn world_lines(vm: &Vm) -> Vec<String> {
    let db = vm.database();
    let mut out = vec!["Rooms:".to_string()];
    out.extend(list_rooms(db));
    out.push(String::new());
    out.push("Messages:".to_string());
    out.extend(
        db.messages
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.is_empty())
            .map(|(i, m)| format!("#{i} {m}")),
    );
    out
}

/// The "State" section: current/saved room, lamp, darkness, counter, set flags,
/// and a carried-items summary.
fn state_lines(vm: &Vm) -> Vec<String> {
    let db = vm.database();
    let mut out = Vec::new();
    let room = vm.current_room();
    out.push(format!("Room: #{room} {}", vm.room_name(room)));
    let saved = vm.saved_room();
    if saved != 0 {
        out.push(format!("Saved room (op80): #{saved} {}", vm.room_name(saved)));
    }
    out.push(match vm.lamp() {
        -1 => "Lamp fuel: infinite".to_string(),
        n => format!("Lamp fuel: {n}"),
    });
    out.push(format!("Dark: {}", vm.is_dark()));
    out.push(format!("Counter: {}", vm.counter()));
    let flags: Vec<String> = (0..32).filter(|&i| vm.flag(i)).map(flag_name).collect();
    out.push(if flags.is_empty() {
        "Flags set: (none)".to_string()
    } else {
        format!("Flags set: {}", flags.join(", "))
    });
    let carried: Vec<String> = db
        .items
        .iter()
        .enumerate()
        .filter(|(i, _)| vm.item_loc(*i) == CARRIED)
        .map(|(_, it)| it.text.clone())
        .collect();
    out.push(if carried.is_empty() {
        "Carried: (nothing)".to_string()
    } else {
        format!("Carried: {}", carried.join(", "))
    });
    out
}

impl Debugger for Vm {
    fn pc(&self) -> u32 {
        // Scott has no live PC; anchor the Actions view at the top of the table.
        0
    }

    fn disassemble(&self, addr: u32, lines: usize) -> Vec<String> {
        let n = self.database().actions.len();
        (addr as usize..)
            .take(lines)
            .take_while(|&i| i < n)
            .map(|i| summary_full(self, i))
            .collect()
    }

    fn disassemble_basic(&self, addr: u32, lines: usize) -> Vec<String> {
        let n = self.database().actions.len();
        (addr as usize..).take(lines).take_while(|&i| i < n).map(|i| summary_basic(self, i)).collect()
    }

    fn disassemble_raw(&self, addr: u32, lines: usize) -> Vec<String> {
        let n = self.database().actions.len();
        (addr as usize..).take(lines).take_while(|&i| i < n).map(|i| summary_raw(self, i)).collect()
    }

    fn next_instr(&self, addr: u32) -> u32 {
        let last = (self.database().actions.len() as u32).saturating_sub(1);
        (addr + 1).min(last)
    }

    fn prev_instr(&self, addr: u32) -> u32 {
        addr.saturating_sub(1)
    }

    fn describe_line(&self, addr: u32) -> Option<Vec<String>> {
        let idx = addr as usize;
        let mut lines = decompile_action(self.database(), idx)?.lines();
        if let Some(&(_, slot)) = self.last_blocked().iter().find(|&&(a, _)| a == idx) {
            lines.push(String::new());
            lines.push(format!("BLOCKED this turn: condition slot {slot} was false"));
        }
        Some(lines)
    }

    fn executed_pcs(&self) -> HashSet<u32> {
        self.fired_actions().iter().map(|&i| i as u32).collect()
    }

    fn ever_executed_pcs(&self) -> HashSet<u32> {
        self.ever_fired().iter().map(|&i| i as u32).collect()
    }

    // ── Scott-specific section content (reused plain-list slots) ──
    fn globals_lines(&self) -> Vec<String> {
        state_lines(self)
    }

    fn locals_lines(&self) -> Vec<String> {
        world_lines(self)
    }

    fn object_tree_lines(&self) -> Vec<String> {
        list_items(self.database())
    }

    fn dictionary_lines(&self) -> Vec<String> {
        list_vocab(self.database())
    }

    // ── Sections meaningless for Scott: hidden by `sections()`, so these are
    //    only ever called (harmlessly) by `refresh` and return nothing. ──
    fn stack_lines(&self) -> Vec<String> {
        Vec::new()
    }
    fn eval_stack_lines(&self) -> Vec<String> {
        Vec::new()
    }
    fn memory_hex(&self, _addr: u32, _rows: usize) -> Vec<String> {
        Vec::new()
    }
    fn memory_len(&self) -> u32 {
        0
    }
    fn object_detail(&self, _obj: u16) -> Vec<String> {
        Vec::new()
    }
    fn frame_locals(&self, _idx: usize) -> Vec<String> {
        Vec::new()
    }
    fn var_value(&self, _var: u8) -> Option<u16> {
        None
    }

    // ── Panel layout + labels ──
    fn sections(&self) -> Option<[&'static [Section]; 3]> {
        Some(SCOTT_TABS)
    }

    fn section_label(&self, s: Section) -> &'static str {
        match s {
            Section::Disasm => "Actions",
            Section::Globals => "State",
            Section::Objects => "Items",
            Section::Dict => "Vocab",
            Section::Locals => "World",
            other => other.label(),
        }
    }
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use scott::{Action, Condition, Database, Item, Room};

    /// A tiny hand-built database: one player-command action (GET LAMP, guarded
    /// by "lamp in room"), plus a lamp item and two rooms with an exit.
    fn tiny_db() -> Database {
        let mut verbs = vec![String::new(); 11];
        verbs[10] = "GET".into();
        let mut nouns = vec![String::new(); 2];
        nouns[1] = "LAMP".into();
        Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 1,
            word_length: 3,
            light_time: 50,
            treasure_room: 2,
            actions: vec![Action {
                verb: 10,
                noun: 1,
                conditions: [
                    Condition { code: 2, value: 1 }, // ITEM_IN_ROOM(lamp)
                    Condition { code: 0, value: 1 }, // PARAM for GET
                    Condition { code: 0, value: 0 },
                    Condition { code: 0, value: 0 },
                    Condition { code: 0, value: 0 },
                ],
                commands: [52, 1, 0, 0], // GET(param), PRINT msg 1
            }],
            verbs,
            nouns,
            rooms: vec![
                Room { exits: [0; 6], desc: "limbo".into(), literal: true },
                Room { exits: [0, 0, 0, 0, 0, 2], desc: "clearing".into(), literal: false },
                Room { exits: [0, 0, 0, 0, 1, 0], desc: "cave".into(), literal: false },
            ],
            messages: vec!["".into(), "OK.".into()],
            // The lamp is item #1 (the index the action's ITEM_IN_ROOM guard and
            // GET param both reference); item #0 is an inert filler so index 1 is
            // a real, resolvable item.
            items: vec![
                Item { text: "worthless rock".into(), treasure: false, auto_noun: None, start_loc: 0 },
                Item {
                    text: "a brass lamp".into(),
                    treasure: false,
                    auto_noun: Some("LAM".into()),
                    start_loc: 1,
                },
            ],
            adventure_number: 0,
        }
    }

    fn dbg(vm: &Vm) -> &dyn Debugger {
        vm
    }

    #[test]
    fn disassemble_full_prefixes_the_action_index_and_resolves_names() {
        let vm = Vm::new(tiny_db());
        let lines = dbg(&vm).disassemble(0, 16);
        assert_eq!(lines.len(), 1, "one summary line per action");
        assert!(lines[0].starts_with("000000  "), "6-hex action-index prefix: {:?}", lines[0]);
        assert!(lines[0].contains("GET LAMP"), "resolved trigger: {:?}", lines[0]);
        assert!(lines[0].contains("ITEM_IN_ROOM(a brass lamp)"), "resolved cond: {:?}", lines[0]);
        assert!(lines[0].contains("GET(a brass lamp)"), "resolved cmd: {:?}", lines[0]);
    }

    #[test]
    fn basic_and_raw_drop_resolved_names() {
        let vm = Vm::new(tiny_db());
        let basic = dbg(&vm).disassemble_basic(0, 16);
        assert!(basic[0].contains("V10 N1"), "raw trigger in basic: {:?}", basic[0]);
        assert!(basic[0].contains("ITEM_IN_ROOM(1)"), "mnemonic + raw operand: {:?}", basic[0]);
        assert!(!basic[0].contains("brass lamp"), "no resolved name in basic: {:?}", basic[0]);
        let raw = dbg(&vm).disassemble_raw(0, 16);
        assert!(raw[0].contains("v10 n1"), "numeric trigger in raw: {:?}", raw[0]);
        // Both real condition slots survive: the ITEM_IN_ROOM guard (2/1) and
        // the code-0 PARAM slot (0/1) that feeds GET its item.
        assert!(raw[0].contains("c[2/1,0/1]"), "numeric cond tuple: {:?}", raw[0]);
        assert!(raw[0].contains("k[52,1]"), "numeric command tuple: {:?}", raw[0]);
    }

    #[test]
    fn next_and_prev_instr_clamp_to_the_action_count() {
        let vm = Vm::new(tiny_db()); // 1 action → indices clamp to 0
        let d = dbg(&vm);
        assert_eq!(d.next_instr(0), 0, "clamped at the last action");
        assert_eq!(d.prev_instr(0), 0, "clamped at 0");
        assert_eq!(d.pc(), 0);
    }

    #[test]
    fn sections_hide_callstack_evalstack_memory_and_relabel() {
        let vm = Vm::new(tiny_db());
        let d = dbg(&vm);
        let layout = d.sections().expect("Scott provides a layout");
        let all: Vec<Section> = layout.iter().flat_map(|w| w.iter().copied()).collect();
        assert!(!all.contains(&Section::CallStack), "Call Stack hidden");
        assert!(!all.contains(&Section::EvalStack), "Eval Stack hidden");
        assert!(!all.contains(&Section::Memory), "Memory hidden");
        assert!(all.contains(&Section::Disasm) && all.contains(&Section::Globals));
        assert!(layout.iter().all(|w| !w.is_empty()), "no empty window");
        assert_eq!(d.section_label(Section::Disasm), "Actions");
        assert_eq!(d.section_label(Section::Globals), "State");
        assert_eq!(d.section_label(Section::Objects), "Items");
        assert_eq!(d.section_label(Section::Dict), "Vocab");
        assert_eq!(d.section_label(Section::Locals), "World");
    }

    #[test]
    fn state_items_vocab_and_world_sections_are_populated() {
        let vm = Vm::new(tiny_db());
        let d = dbg(&vm);
        assert!(d.globals_lines().iter().any(|l| l.contains("Lamp fuel: 50")), "state shows lamp");
        assert!(d.globals_lines().iter().any(|l| l.starts_with("Room: #1")), "state shows room");
        assert!(d.object_tree_lines().iter().any(|l| l.contains("a brass lamp")), "items list");
        assert!(d.dictionary_lines().iter().any(|l| l.contains("GET")), "vocab list");
        let world = d.locals_lines();
        assert!(world.iter().any(|l| l.contains("clearing")), "world lists rooms");
        assert!(world.iter().any(|l| l.contains("OK.")), "world lists messages");
    }

    #[test]
    fn fired_and_blocked_track_a_played_turn() {
        // Trace on; take the lamp (fires action 0). Then, after moving the lamp
        // out of the room, GET LAMP is blocked by its ITEM_IN_ROOM guard.
        let mut vm = Vm::new_with_trace(tiny_db(), true);
        vm.supply_line("get lamp");
        let _ = vm.step();
        assert!(dbg(&vm).executed_pcs().contains(&0), "action 0 fired this turn");
        assert!(dbg(&vm).ever_executed_pcs().contains(&0), "and cumulatively");

        // Now the lamp is carried (not in the room), so GET LAMP is blocked.
        vm.supply_line("get lamp");
        let _ = vm.step();
        let blocked_line = dbg(&vm).disassemble(0, 1).remove(0);
        assert!(blocked_line.contains("✗cond"), "blocked action is annotated: {blocked_line:?}");
        let desc = dbg(&vm).describe_line(0).expect("action 0 describes");
        assert!(desc.iter().any(|l| l.contains("BLOCKED")), "hover explains the block: {desc:?}");
    }

    #[test]
    fn ever_executed_seeds_from_a_prior_run() {
        let mut vm = Vm::new(tiny_db());
        vm.seed_ever_fired(&HashSet::from([0u32]));
        assert!(dbg(&vm).ever_executed_pcs().contains(&0), "seeded coverage surfaces");
    }
}
