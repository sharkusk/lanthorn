//! Parses the ScottFree `.dat` text format into a [`Database`]: a lexer over
//! whitespace-separated integers and `"`-quoted strings, a header of table
//! sizes, then rooms, messages, actions, verbs/nouns, and items in that
//! fixed order. [`looks_like_scott`] does the cheap header-shape sniff a
//! multi-engine host uses to decide whether a file is worth handing to
//! [`Database::parse`] at all.
//!
//! Every count and index in the header is bounds-checked before it sizes an
//! allocation or indexes a table (SQ-0629): a hostile or truncated file
//! returns a [`LoadError`] rather than exhausting memory or panicking.
//!
//! [`Database::parse`] takes any `AsRef<[u8]>` (a `&str` still works
//! unchanged) and lexes over raw bytes rather than `char`s, so a Latin-1 or
//! otherwise non-UTF-8 `.dat` loads instead of failing a UTF-8 check before
//! it ever reaches the parser (SQ-1412). Bytes outside ASCII inside a quoted
//! string become `?`.

use crate::database::CARRIED;
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

/// Tokenizer over a `.dat` source: whitespace-separated ints and `"`-delimited
/// strings, over raw bytes (not `char`s) so non-UTF-8 input never panics or
/// fails up front — see the module doc.
struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a [u8]) -> Self {
        Lexer { bytes: src, pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn next_int(&mut self) -> Result<i32, LoadError> {
        self.skip_ws();
        if self.pos >= self.bytes.len() {
            return Err(LoadError::Truncated);
        }
        let start = self.pos;
        while self.pos < self.bytes.len() && !self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        // Header/action/exit tokens are always plain ASCII digits (optionally
        // signed), so a lossy-to-ASCII read is exact here; a non-numeric token
        // (including one containing non-ASCII bytes) fails to parse below.
        let word = String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned();
        word.parse::<i32>().map_err(|_| LoadError::BadInt(word))
    }

    /// Reads one `"`-delimited string, porting ScottFree 1.14's `ReadString`
    /// (`ScottCurses.c:189-224`) byte rule-for-rule: a backtick (`` ` ``, 0x60)
    /// becomes `"`, and a doubled `""` inside the string is an escaped literal
    /// `"` rather than the closing quote (checked BEFORE the backtick
    /// substitution, matching the source's order — `ReadString` tests `c=='"'`
    /// first and only then rewrites a backtick). Two additions beyond
    /// `ReadString`, needed because this lexer now reads raw bytes rather than
    /// a text file already decoded by the host's own locale: a `\r` byte is
    /// dropped (CRLF-authored `.dat`s read the same as LF ones) and a byte
    /// outside ASCII becomes `?` (so an arbitrary, e.g. Latin-1, `.dat` is
    /// always valid UTF-8 once lexed, never a panic or a `LoadError`).
    fn next_str(&mut self) -> Result<String, LoadError> {
        self.skip_ws();
        if self.pos >= self.bytes.len() {
            return Err(LoadError::Truncated);
        }
        if self.bytes[self.pos] != b'"' {
            return Err(LoadError::Unterminated);
        }
        self.pos += 1;
        let mut s = String::new();
        loop {
            if self.pos >= self.bytes.len() {
                return Err(LoadError::Unterminated);
            }
            let c = self.bytes[self.pos];
            self.pos += 1;
            if c == b'\r' {
                continue;
            }
            if c == b'"' {
                if self.bytes.get(self.pos) == Some(&b'"') {
                    self.pos += 1; // doubled quote: escaped literal '"'
                } else {
                    break; // the closing quote
                }
                s.push('"');
                continue;
            }
            if c == 0x60 {
                s.push('"'); // backtick -> '"' (ReadString, ScottCurses.c ~215)
            } else if c.is_ascii() {
                s.push(c as char);
            } else {
                s.push('?');
            }
        }
        Ok(s)
    }
}

/// Extracts an item's auto-get/drop noun, porting ScottFree 1.14's item-load
/// loop verbatim (`ScottCurses.c:319-327`):
/// ```c
/// ip->AutoGet=strchr(ip->Text,'/');
/// /* Some games use // to mean no auto get/drop word! */
/// if(ip->AutoGet && strcmp(ip->AutoGet,"//") && strcmp(ip->AutoGet,"/*"))
/// {
///     char *t;
///     *ip->AutoGet++=0;
///     t=strchr(ip->AutoGet,'/');
///     if(t!=NULL)
///         *t=0;
/// }
/// ```
/// `AutoGet` starts at the FIRST `/`, not the last (the previous port here
/// used `rfind` and mis-split `"Luger/LUGER/GUN/"` as display "Luger/LUGER" /
/// bind "GUN" instead of ScottFree's display "Luger" / bind "LUGER" —
/// `secret.dat` item 33, SQ-1412). `strcmp(AutoGet, "//")` / `"/*"` compares
/// the WHOLE remainder of the string from that first slash to end-of-string,
/// not a prefix — so `"//"`/`"/*"` only means "no autoget word" when the text
/// ends there exactly; when they match, ScottFree skips the split entirely
/// and `Text` (and the display) keeps its literal trailing `//`/`/*`. Once a
/// real marker is found, `AutoGet` runs from just after the first `/` to the
/// next `/` if there is one — tolerating a missing close (`t==NULL`), which
/// leaves the noun running to the end of the original text.
fn extract_auto_noun(text: &mut String) -> Option<String> {
    let first = text.find('/')?;
    let tail = &text[first..];
    if tail == "//" || tail == "/*" {
        return None; // no autoget word; Text (and the display) is untouched
    }
    let after_first = &text[first + 1..];
    let noun = match after_first.find('/') {
        Some(second) => after_first[..second].to_uppercase(),
        None => after_first.to_uppercase(), // missing close: runs to the end
    };
    text.truncate(first);
    Some(noun)
}

impl Database {
    /// Parse a ScottFree `.dat` file. Accepts anything byte-like — `&str`,
    /// `&[u8]`, `&Vec<u8>` — so a caller holding raw file bytes (a Latin-1 or
    /// otherwise non-UTF-8 `.dat`, which a `&str` conversion would reject
    /// outright) can hand them over directly; see the module doc.
    pub fn parse<S: AsRef<[u8]> + ?Sized>(src: &S) -> Result<Database, LoadError> {
        let mut lex = Lexer::new(src.as_ref());

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
            let mut start_loc = lex.next_int()?;
            // ScottFree stores an item's location (and start location) as an
            // `unsigned char` with `CARRIED` defined as 255 (`Scott.h`); this
            // crate represents "carried" as -1 (`database::CARRIED`)
            // throughout `Vm`. Normalise at load so conditions 17/18 ("item
            // still/not in its initial room") still hold after a
            // programmatic take moves an item to -1 — comparing a live -1
            // against a start_loc left at 255 would never match (SQ-1412).
            if start_loc == 255 {
                start_loc = CARRIED;
            }
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
    let mut lex = Lexer::new(src.as_bytes());
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
