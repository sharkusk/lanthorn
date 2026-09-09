//! The static game data a Scott Adams `.dat` file describes: rooms, items,
//! the verb/noun action table, vocabulary, and messages. [`Database::parse`]
//! builds one from source text; every field here is `pub` so a host can also
//! build one by hand — see the crate-level example.
//!
//! This module holds only the data and the vocabulary lookups
//! ([`Database::match_verb`], [`Database::match_noun`]) that both loading
//! and play need; turn-by-turn state lives on [`crate::Vm`] instead.

pub use grammar_model::ObjectWords;

/// The complete static game data a `.dat` file describes — rooms, items, the
/// action table, vocabulary, and messages — plus the handful of header
/// numbers that don't belong to any of those tables. Immutable for the life
/// of a game: turn-by-turn state (item locations, flags, the player's room)
/// lives on [`crate::Vm`] instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Database {
    /// How many items the player may carry at once. Opcode 52 (auto-get)
    /// refuses to pick up a further item once the carried count reaches this
    /// EXACTLY — `Vm::run_commands` checks `==`, matching ScottFree's own
    /// observed behaviour, not `>=`.
    pub max_carry: i32,
    /// The room index the player starts in, and the room a fresh game (or an
    /// in-game restart) places them in.
    pub start_room: usize,
    /// How many of the game's items are treasures — the count `Vm` checks
    /// against how many currently sit in `treasure_room` to decide whether
    /// the game has been won.
    pub num_treasures: i32,
    /// How many leading characters of a typed word the parser compares
    /// against the verb/noun vocabulary before deciding two words match; 0
    /// means compare the whole word with no truncation
    /// ([`Database::match_verb`]/[`Database::match_noun`]).
    pub word_length: usize,
    /// How many turns of light-source fuel a new game starts with; `Vm`
    /// counts this down every turn the lamp (see [`LIGHT_SOURCE`]) is in
    /// play at all, and sets [`LAMP_EMPTY_FLAG`] once it runs out.
    pub light_time: i32, // -1 = infinite
    /// The room index treasures must be carried into for `Vm` to count them
    /// toward the win condition.
    pub treasure_room: usize,
    /// The action table: one entry per verb/noun rule the game recognizes
    /// (plus verb-0 occurrence and continuation lines), each pairing a set
    /// of conditions with the commands that run once every one of them
    /// passes — effectively the game's script.
    pub actions: Vec<Action>,
    /// The verb vocabulary, indexed by the verb numbers `Action::verb` and
    /// [`Database::match_verb`] operate on; a `*`-prefixed entry is a
    /// synonym of the nearest preceding non-`*` entry rather than a
    /// canonical verb of its own.
    pub verbs: Vec<String>, // index 0 = placeholder
    /// The noun vocabulary, indexed and synonym-linked the same way as
    /// `verbs`; shared by the two-word parser and by item auto-get/drop word
    /// matching ([`Database::item_words`]).
    pub nouns: Vec<String>,
    /// The room table, indexed by the room numbers used throughout this
    /// struct and `Vm` (`Room::exits`, `start_room`, `treasure_room`, item
    /// locations, and the room-check conditions).
    pub rooms: Vec<Room>,
    /// The message pool: canned text printed by index — an `Action::commands`
    /// opcode in 1..=51 prints message `n`, and one at 102 or above prints
    /// message `n - 50`.
    pub messages: Vec<String>,
    /// The item table: every object and piece of scenery in the game,
    /// indexed by the item numbers `Action::conditions`/`Action::commands`
    /// and `Vm`'s per-item location state refer to.
    pub items: Vec<Item>,
    /// Adventure number from the trailer, best-effort. 0 = unknown/absent.
    pub adventure_number: i32,
    /// The tokenised action script, for a database loaded from a
    /// **TI-99/4A** release ([`crate::parse_ti994a`]) — `None` for every
    /// other dialect, which is every database whose actions fit `actions`
    /// above.
    ///
    /// This is the one table that does not decode to the reference format's
    /// shape, and it is not a choice: a tokenised record is a
    /// variable-length opcode stream in which conditions and commands
    /// interleave, and one real record reaches 25 conditions and 37 commands
    /// where an [`Action`] has room for five and four. When this is `Some`,
    /// `actions` is empty and [`crate::Vm`] runs the script instead — see
    /// [`crate::ti994a`] for the encoding and the runtime differences it
    /// brings with it.
    pub ti99: Option<crate::ti994a::Ti99Script>,
}

/// One room's exits, description text, and how that description should be
/// presented.
#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    /// The room reached by moving North, South, East, West, Up, and Down, in
    /// that order; 0 means there is no exit that way (room 0 itself is
    /// conventionally an unused placeholder in a real Scott Adams database,
    /// which is why it doubles as the "no exit" sentinel).
    pub exits: [usize; 6], // order: N,S,E,W,Up,Down
    /// The room's description text, exactly as the database stores it — the
    /// leading `*` that would mark it `literal` has already been stripped.
    pub desc: String,
    /// Whether `desc` should be printed exactly as stored rather than after
    /// an "I'm in a "-style prefix — Scott's convention for text that
    /// doesn't read naturally following that phrase (e.g. "Outside a large
    /// gothic looking building."). Set from a leading `*` on the room's
    /// description in the source file, stripped from `desc` itself.
    pub literal: bool,
}

/// One object or piece of scenery in the game.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// The item's display description, with the leading `*` (see
    /// `treasure`) and any trailing `/NOUN/` auto-get marker (see
    /// `auto_noun`) already stripped by the loader.
    pub text: String,
    /// Whether this item counts toward the game's win condition — set from a
    /// leading `*` on the item's text in the source file, stripped from
    /// `text` itself.
    pub treasure: bool,
    /// The one noun the player can use to auto-get or auto-drop this item by
    /// name, lifted from a trailing `/NOUN/` marker on the item's raw text
    /// per the `.dat` format's own auto-get convention (`loader::extract_auto_noun`,
    /// re-derived from the Swansea Definition §2.5). `None` for an item with
    /// no marker — scenery and other objects the parser cannot refer to
    /// directly.
    pub auto_noun: Option<String>,
    /// The room this item occupies at the start of a new game, or
    /// [`CARRIED`] if the player starts holding it, or 0 if it starts out of
    /// play entirely. `Vm` seeds its live per-item location from this value,
    /// and conditions 17/18 ("item still/not in its start location") compare
    /// the live location back against it.
    pub start_loc: i32,
}

/// One line of the action table: a verb/noun trigger, the conditions that
/// must all hold, and the commands that run when they do.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    /// The verb number this line responds to ([`Database::verbs`] index).
    /// Verb 0 marks the line as an occurrence or continuation line instead
    /// of a player-triggered command — see `Action::noun` and
    /// `crate::Vm`'s occurrence pass.
    pub verb: u16,
    /// For a player-triggered line (`verb != 0`): the noun number it
    /// requires, or 0 to match any noun (including no noun at all). For a
    /// verb-0 occurrence line: the percent chance (0-100, rolled each turn)
    /// that it fires; a verb-0 line with noun 0 as well is a pure
    /// continuation target, reachable only by another line's opcode 73
    /// ("continue"), never fired on its own.
    pub noun: u16,
    /// Five conditions, all of which must hold for this line's commands to
    /// run; a line with all five as the always-true code 0 has no guard.
    pub conditions: [Condition; 5],
    /// Up to four command opcodes to run, in order, once every condition
    /// passes: 0 is an unused slot, 1..=51 and 102 upward print a message
    /// (see `Database::messages`), and the rest (52..=89, with 90..=101
    /// currently no-ops) are built-in verbs like take, drop, move the
    /// player, or continue into the next action line.
    pub commands: [u16; 4],
}

/// One guard on an [`Action`] line: a condition code and the operand it
/// tests against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Condition {
    /// Which test to run (0..=19; see `Vm::eval_condition`) — for example
    /// "item is carried", "player is in room", "flag is set", or a counter
    /// comparison. Code 0 is always true (an unconditional line).
    pub code: u8,
    /// The operand `code` tests against: depending on the code, an item
    /// index, a room index, a flag index, or a counter threshold.
    pub value: u16,
}

/// Item number of the light source (the lamp): carrying it, or having it in
/// the current room, is what keeps [`DARK_FLAG`] from blocking the player,
/// and it's the item whose fuel `light_time`/[`LAMP_EMPTY_FLAG`] track.
pub const LIGHT_SOURCE: usize = 9;
/// Flag index that marks the CURRENT room as dark — `Vm` consults it (along
/// with whether the lamp is carried or present in the room) to decide
/// whether the player can see.
pub const DARK_FLAG: usize = 15;
/// Flag index `Vm` sets once the lamp's fuel (`light_time`) has run out, so
/// the game can react to (and print) the lamp going dead.
pub const LAMP_EMPTY_FLAG: usize = 16;
/// Sentinel item location meaning "carried by the player", used throughout
/// `Vm`'s per-item location state and by [`Item::start_loc`]. The on-disk
/// `.dat` format instead uses the byte value 255 for this; the loader
/// normalizes it to `CARRIED` at parse time (see `Database::parse`).
pub const CARRIED: i32 = -1;

impl Database {
    /// Match a typed word against the verb vocabulary, returning the canonical
    /// verb NUMBER (index) or None. Comparison ignores case, truncates both sides
    /// to `word_length`, and treats a stored `*`-prefixed entry as a synonym of the
    /// nearest preceding non-`*` verb (whose index is the number returned).
    pub fn match_verb(&self, word: &str) -> Option<u16> {
        match_word(&self.verbs, word, self.word_length)
    }
    /// As `match_verb`, for the noun vocabulary.
    pub fn match_noun(&self, word: &str) -> Option<u16> {
        match_word(&self.nouns, word, self.word_length)
    }

    /// What item `index` is, and what it can be **called**.
    ///
    /// A Scott Adams database has no properties and no per-object word arrays —
    /// there was never anywhere to put them. What it has is a flat noun table
    /// and, on each item that the player can pick up, a trailing `/NOUN/`
    /// marker naming the one noun that refers to it (`extract_auto_noun` in the
    /// loader lifts it out of the description at load time). The words that
    /// refer to an item are therefore that noun plus the `*`-prefixed synonyms
    /// that follow it in the table, which is what the two-word parser resolves
    /// through [`match_noun`](Database::match_noun) anyway.
    ///
    /// `None` for an item with no marker — scenery, messages, the pieces of the
    /// map that exist only to be looked at. That is not a limitation of this
    /// reader: those items have no word, and the parser cannot name them either.
    ///
    /// Answers with the same [`ObjectWords`] the Z-machine and Glulx readers
    /// return, so a caller asking "what is this and what can it be called" gets
    /// one shape from all three engines.
    pub fn item_words(&self, index: usize) -> Option<ObjectWords> {
        let item = self.items.get(index)?;
        let noun = item.auto_noun.as_deref()?;
        let number = self.match_noun(noun)? as usize;
        // The canonical entry, then every `*`-prefixed synonym that follows it,
        // which is exactly the run `match_word` walks back through.
        let mut words = Vec::new();
        for entry in &self.nouns[number..] {
            if words.is_empty() || entry.starts_with('*') {
                words.push(entry.trim_start_matches('*').to_lowercase());
            } else {
                break;
            }
        }
        words.retain(|w| !w.is_empty());
        if words.is_empty() {
            return None;
        }
        Some(ObjectWords::new(
            index as u32,
            item.text.clone(),
            words,
            // No properties exist here to have read them from.
            None,
            (self.word_length > 0).then_some(self.word_length),
        ))
    }
}

/// Shared matcher. `word_length` 0 means "no truncation" (compare full words).
fn match_word(list: &[String], input: &str, word_length: usize) -> Option<u16> {
    let key = trunc_upper(input, word_length);
    if key.is_empty() {
        return None;
    }
    for (i, entry) in list.iter().enumerate() {
        let e = entry.strip_prefix('*').unwrap_or(entry);
        if e.is_empty() {
            continue;
        }
        if trunc_upper(e, word_length) == key {
            // Resolve a synonym to the nearest preceding non-`*` canonical index.
            let mut j = i;
            while j > 0 && list[j].starts_with('*') {
                j -= 1;
            }
            return Some(j as u16);
        }
    }
    None
}

fn trunc_upper(s: &str, word_length: usize) -> String {
    let up = s.trim().to_uppercase();
    if word_length == 0 {
        return up;
    }
    up.chars().take(word_length).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn database_constructs_and_indexes_from_zero() {
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 1,
            word_length: 3,
            light_time: 125,
            treasure_room: 1,
            actions: vec![],
            verbs: vec!["".into(), "GO".into()],
            nouns: vec!["".into(), "NORTH".into()],
            rooms: vec![
                Room {
                    exits: [0; 6],
                    desc: "limbo".into(),
                    literal: true,
                },
                Room {
                    exits: [0, 0, 0, 0, 0, 0],
                    desc: "dark forest".into(),
                    literal: false,
                },
            ],
            messages: vec!["".into()],
            items: vec![Item {
                text: "*gold*".into(),
                treasure: true,
                auto_noun: None,
                start_loc: 1,
            }],
            adventure_number: 0,
            ti99: None,
        };
        assert_eq!(db.rooms.len(), 2);
        assert_eq!(db.start_room, 1);
        assert!(db.items[0].treasure);
        assert_eq!(DARK_FLAG, 15);
    }

    #[test]
    fn matches_verbs_nouns_and_synonyms_to_canonical_number() {
        // Build a tiny Database by hand with a synonym in each column.
        // verbs: index 1 = "GO", 10 = "GET", 11 = "*TAKE" (synonym of GET)
        // nouns: index 1 = "NORTH", 2 = "*N" (synonym of NORTH)  [word_length 3]
        let mut verbs = vec![String::new(); 12];
        verbs[1] = "GO".into();
        verbs[10] = "GET".into();
        verbs[11] = "*TAKE".into();
        let mut nouns = vec![String::new(); 12];
        nouns[1] = "NORTH".into();
        nouns[2] = "*N".into();
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs,
            nouns,
            rooms: vec![],
            messages: vec![],
            items: vec![],
            adventure_number: 0,
            ti99: None,
        };
        assert_eq!(db.match_verb("go"), Some(1));
        assert_eq!(db.match_verb("GET"), Some(10));
        assert_eq!(db.match_verb("take"), Some(10)); // synonym -> canonical GET number
        assert_eq!(db.match_verb("xyzzy"), None);
        assert_eq!(db.match_noun("north"), Some(1));
        assert_eq!(db.match_noun("n"), Some(1)); // "*N" synonym -> NORTH number
        // word_length=3 truncation: "northXYZ" still matches NORTH via first 3 chars
        assert_eq!(db.match_noun("nor"), Some(1));
    }
}
