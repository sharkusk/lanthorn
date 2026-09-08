//! The static game data a Scott Adams `.dat` file describes: rooms, items,
//! the verb/noun action table, vocabulary, and messages. [`Database::parse`]
//! builds one from source text; every field here is `pub` so a host can also
//! build one by hand — see the crate-level example.
//!
//! This module holds only the data and the vocabulary lookups
//! ([`Database::match_verb`], [`Database::match_noun`]) that both loading
//! and play need; turn-by-turn state lives on [`crate::Vm`] instead.

pub use grammar_model::ObjectWords;

#[derive(Debug, Clone, PartialEq)]
pub struct Database {
    pub max_carry: i32,
    pub start_room: usize,
    pub num_treasures: i32,
    pub word_length: usize,
    pub light_time: i32, // -1 = infinite
    pub treasure_room: usize,
    pub actions: Vec<Action>,
    pub verbs: Vec<String>, // index 0 = placeholder
    pub nouns: Vec<String>,
    pub rooms: Vec<Room>,
    pub messages: Vec<String>,
    pub items: Vec<Item>,
    /// Adventure number from the trailer, best-effort. 0 = unknown/absent.
    pub adventure_number: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    pub exits: [usize; 6], // order: N,S,E,W,Up,Down
    pub desc: String,
    pub literal: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub text: String,
    pub treasure: bool,
    pub auto_noun: Option<String>,
    pub start_loc: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub verb: u16,
    pub noun: u16,
    pub conditions: [Condition; 5],
    pub commands: [u16; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Condition {
    pub code: u8,
    pub value: u16,
}

pub const LIGHT_SOURCE: usize = 9;
pub const DARK_FLAG: usize = 15;
pub const LAMP_EMPTY_FLAG: usize = 16;
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
