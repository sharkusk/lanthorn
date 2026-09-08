use crate::*;

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LoadError {
    Truncated,
    BadInt(String),
    Unterminated,
    /// A header count is negative or implausibly large — a hostile file can
    /// otherwise request gigabytes of `Vec` capacity before a single body
    /// token is read (SQ-0629).
    BadCount(&'static str, i32),
    /// A room exit points outside the room table (negative or > NumRooms).
    BadExit(i32),
}

/// Tokenizer over a `.dat` source: whitespace-separated ints and `"`-delimited strings.
struct Lexer {
    chars: Vec<char>,
    pos: usize,
}

impl Lexer {
    fn new(src: &str) -> Self {
        Lexer {
            chars: src.chars().collect(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn next_int(&mut self) -> Result<i32, LoadError> {
        self.skip_ws();
        if self.pos >= self.chars.len() {
            return Err(LoadError::Truncated);
        }
        let start = self.pos;
        while self.pos < self.chars.len() && !self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
        let word: String = self.chars[start..self.pos].iter().collect();
        word.parse::<i32>().map_err(|_| LoadError::BadInt(word))
    }

    fn next_str(&mut self) -> Result<String, LoadError> {
        self.skip_ws();
        if self.pos >= self.chars.len() {
            return Err(LoadError::Truncated);
        }
        if self.chars[self.pos] != '"' {
            return Err(LoadError::Unterminated);
        }
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.chars.len() && self.chars[self.pos] != '"' {
            self.pos += 1;
        }
        if self.pos >= self.chars.len() {
            return Err(LoadError::Unterminated);
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        self.pos += 1;
        Ok(s)
    }
}

/// Extracts a trailing `/NOUN/` marker from an item description, uppercasing it and
/// stripping the marker from `text`. Returns `None` if there is no trailing marker.
fn extract_auto_noun(text: &mut String) -> Option<String> {
    if !text.ends_with('/') {
        return None;
    }
    let last_idx = text.rfind('/')?;
    let before = &text[..last_idx];
    let start_idx = before.rfind('/')?;
    let noun = text[start_idx + 1..last_idx].to_uppercase();
    text.truncate(start_idx);
    Some(noun)
}

impl Database {
    pub fn parse(src: &str) -> Result<Database, LoadError> {
        let mut lex = Lexer::new(src);

        let _unknown = lex.next_int()?;
        let num_items = lex.next_int()?;
        let num_actions = lex.next_int()?;
        let num_words = lex.next_int()?;
        let num_rooms = lex.next_int()?;
        let max_carry = lex.next_int()?;
        let player_room = lex.next_int()?;
        let num_treasures = lex.next_int()?;
        let word_length = lex.next_int()?;
        let light_time = lex.next_int()?;
        let num_messages = lex.next_int()?;
        let treasure_room = lex.next_int()?;

        // Bound every count that sizes a pre-reserved Vec. Real Scott
        // databases have counts in the low hundreds; `looks_like_scott` uses
        // the same <10000 ceiling for its sniff. Without this, an unvalidated
        // header int (num_actions = 2_000_000_000) asks `with_capacity` for
        // tens of gigabytes before any body token is read (SQ-0629).
        const MAX_COUNT: i32 = 10_000;
        for (name, v) in [
            ("NumItems", num_items),
            ("NumActions", num_actions),
            ("NumWords", num_words),
            ("NumRooms", num_rooms),
            ("NumMessages", num_messages),
        ] {
            if !(0..=MAX_COUNT).contains(&v) {
                return Err(LoadError::BadCount(name, v));
            }
        }

        let mut actions = Vec::with_capacity(num_actions as usize + 1);
        for _ in 0..=num_actions {
            let w0 = lex.next_int()?;
            let verb = (w0 / 150) as u16;
            let noun = (w0 % 150) as u16;
            let mut conditions = [Condition { code: 0, value: 0 }; 5];
            for c in conditions.iter_mut() {
                let w = lex.next_int()?;
                c.code = (w % 20) as u8;
                c.value = (w / 20) as u16;
            }
            let w6 = lex.next_int()?;
            let w7 = lex.next_int()?;
            let commands = [
                (w6 / 150) as u16,
                (w6 % 150) as u16,
                (w7 / 150) as u16,
                (w7 % 150) as u16,
            ];
            actions.push(Action {
                verb,
                noun,
                conditions,
                commands,
            });
        }

        let mut verbs: Vec<String> = Vec::with_capacity(num_words as usize + 1);
        let mut nouns: Vec<String> = Vec::with_capacity(num_words as usize + 1);
        for _ in 0..=num_words {
            let verb = lex.next_str()?;
            let noun = lex.next_str()?;
            verbs.push(verb);
            nouns.push(noun);
        }

        let mut rooms = Vec::with_capacity(num_rooms as usize + 1);
        for _ in 0..=num_rooms {
            let mut exits = [0usize; 6];
            for e in exits.iter_mut() {
                let v = lex.next_int()?;
                // Exits index the room table at move time; a negative value
                // would wrap to a huge usize and an over-large one soft-locks
                // the player in a nonexistent room (SQ-0629). 0 = no exit.
                if !(0..=num_rooms).contains(&v) {
                    return Err(LoadError::BadExit(v));
                }
                *e = v as usize;
            }
            let mut desc = lex.next_str()?;
            let literal = if let Some(stripped) = desc.strip_prefix('*') {
                desc = stripped.to_string();
                true
            } else {
                false
            };
            rooms.push(Room {
                exits,
                desc,
                literal,
            });
        }

        let mut messages = Vec::with_capacity(num_messages as usize + 1);
        for _ in 0..=num_messages {
            messages.push(lex.next_str()?);
        }

        let mut items = Vec::with_capacity(num_items as usize + 1);
        for _ in 0..=num_items {
            let mut text = lex.next_str()?;
            let start_loc = lex.next_int()?;
            let treasure = text.starts_with('*');
            let auto_noun = extract_auto_noun(&mut text);
            items.push(Item {
                text,
                treasure,
                auto_noun,
                start_loc,
            });
        }

        // Trailer (optional metadata, best-effort): NumActions+1 comment strings,
        // then a version int, then the adventure number. Any failure here just
        // leaves adventure_number at 0 — the trailer is not required to parse.
        let mut adventure_number = 0;
        (|| -> Result<(), LoadError> {
            for _ in 0..=num_actions {
                lex.next_str()?;
            }
            let _version = lex.next_int()?;
            adventure_number = lex.next_int()?;
            Ok(())
        })()
        .ok();

        // Guard against a malformed start-room field that would index past the
        // rooms table at runtime.
        if player_room < 0 || player_room as usize >= rooms.len() {
            return Err(LoadError::Truncated);
        }

        Ok(Database {
            max_carry,
            start_room: player_room as usize,
            num_treasures,
            word_length: word_length as usize,
            light_time,
            treasure_room: treasure_room as usize,
            actions,
            verbs,
            nouns,
            rooms,
            messages,
            items,
            adventure_number,
        })
    }
}

/// Cheap content sniff for engine detection: parse the 12 header ints and sanity-check.
pub fn looks_like_scott(src: &str) -> bool {
    let mut lex = Lexer::new(src);
    let mut ints = [0i32; 12];
    for slot in ints.iter_mut() {
        match lex.next_int() {
            Ok(v) => *slot = v,
            Err(_) => return false,
        }
    }
    let in_range = |v: i32, lo: i32, hi: i32| v >= lo && v < hi;
    in_range(ints[1], 0, 10000)
        && in_range(ints[2], 0, 10000)
        && in_range(ints[3], 0, 10000)
        && in_range(ints[4], 0, 10000)
        && in_range(ints[10], 0, 10000)
        && in_range(ints[5], 0, 1000)
        && in_range(ints[8], 1, 10)
        // Start room (field 6) must be a valid room index: 0..=num_rooms (field 4).
        && in_range(ints[6], 0, ints[4] + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    // NumItems=1, NumActions=0, NumWords=1, NumRooms=2, NumMessages=0
    const MINI: &str = r#"
32767 1 0 1 2 6 1 0 3 125 0 1
150 1 0 0 0 0 0 0
"" ""
"GO" "NORTH"
0 0 0 0 0 0 "*limbo"
2 0 0 0 0 0 "*forest clearing"
0 0 0 0 0 0 "*swamp"
""
"" 0
"*a brass lamp/LAMP/" 1
"action comment"
0
1
1
"#;
    #[test]
    fn parses_header_rooms_items() {
        let db = Database::parse(MINI).expect("parse");
        assert_eq!(db.start_room, 1);
        assert_eq!(db.light_time, 125);
        assert_eq!(db.rooms.len(), 3); // NumRooms=2 -> 3 slots
        assert_eq!(db.rooms[1].desc, "forest clearing");
        assert!(db.rooms[1].literal);
        assert_eq!(db.rooms[1].exits[0], 2); // north -> room 2
        assert_eq!(db.items.len(), 2); // NumItems=1 -> 2 slots
        assert_eq!(db.items[1].auto_noun.as_deref(), Some("LAMP"));
        assert!(!db.items[1].text.contains('/'));
        assert!(db.items[1].text.starts_with('*')); // treasure keeps its asterisk
    }
    #[test]
    fn action_decodes_verb_noun() {
        let db = Database::parse(MINI).unwrap();
        assert_eq!(db.actions.len(), 1); // NumActions=0 -> 1 slot
        assert_eq!(db.actions[0].verb, 1);
        assert_eq!(db.actions[0].noun, 0);
    }
    #[test]
    fn parses_trailer_adventure_number() {
        // MINI's trailer: 1 comment string ("action comment"), version=0,
        // adventure_number=1, checksum=1.
        let db = Database::parse(MINI).unwrap();
        assert_eq!(db.adventure_number, 1);
    }
    #[test]
    fn missing_trailer_leaves_adventure_number_zero() {
        // Same header/body as MINI, but with the trailer entirely removed.
        const NO_TRAILER: &str = r#"
32767 1 0 1 2 6 1 0 3 125 0 1
150 1 0 0 0 0 0 0
"" ""
"GO" "NORTH"
0 0 0 0 0 0 "*limbo"
2 0 0 0 0 0 "*forest clearing"
0 0 0 0 0 0 "*swamp"
""
"" 0
"*a brass lamp/LAMP/" 1
"#;
        let db = Database::parse(NO_TRAILER).expect("parse without trailer");
        assert_eq!(db.adventure_number, 0);
    }
    #[test]
    fn tiny_cave_fixture_reports_adventure_number() {
        let db = Database::parse(include_str!("../tests/tiny_cave.dat")).expect("parse fixture");
        assert_eq!(db.adventure_number, 99);
    }
    #[test]
    fn sniff_accepts_scott_rejects_garbage() {
        assert!(looks_like_scott(MINI));
        assert!(!looks_like_scott("This is a plain english sentence."));
        assert!(!looks_like_scott("\x01\x02\x03 not text"));
    }
    #[test]
    fn negative_count_errors_not_panics() {
        // NumActions = -1 (field index 2) must yield a LoadError, not a panic
        let bad = "32767 1 -1 1 2 6 1 0 3 125 0 1\n";
        assert!(Database::parse(bad).is_err());
    }

    #[test]
    fn out_of_range_start_room_errors_not_panics() {
        // MINI has NumRooms=2 (3 slots: 0..2). A start room of 9 (field index 6)
        // is out of range and must yield a LoadError rather than panic at runtime.
        const BAD_START: &str = r#"
32767 1 0 1 2 6 9 0 3 125 0 1
150 1 0 0 0 0 0 0
"" ""
"GO" "NORTH"
0 0 0 0 0 0 "*limbo"
2 0 0 0 0 0 "*forest clearing"
0 0 0 0 0 0 "*swamp"
""
"" 0
"*a brass lamp/LAMP/" 1
"#;
        assert!(Database::parse(BAD_START).is_err());
        // The sniff should also reject it (start room outside 0..=num_rooms).
        assert!(!looks_like_scott(BAD_START));
    }
}
