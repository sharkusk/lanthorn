//! Import of ScottFree 1.14's own save-file format (`SaveGame`/`LoadGame`,
//! `ScottCurses.c:653-706`) — a plain whitespace-separated text format,
//! **not** this crate's own [`crate::Vm::snapshot`] binary blob. A player who
//! has an old ScottFree `.sav` (or one from a fork that kept the format —
//! Gargoyle's `scott.c` and cspiegel's `scottfree-glk` both read/write it
//! unchanged) can bring it into lanthorn via [`crate::Vm::restore_scottfree`].
//!
//! The format, read in this exact order (`LoadGame`, `ScottCurses.c:679-706`):
//!
//! ```c
//! for(ct=0;ct<16;ct++)
//!     fscanf(f,"%d %d\n",&Counters[ct],&RoomSaved[ct]);
//! fscanf(f,"%ld %d %hd %d %d %hd\n",
//!     &BitFlags,&DarkFlag,&MyLoc,&CurrentCounter,&SavedRoom,
//!     &GameHeader.LightTime);
//! for(ct=0;ct<=GameHeader.NumItems;ct++)
//!     fscanf(f,"%hd\n",&lo); /* Items[ct].Location=(unsigned char)lo; */
//! ```
//!
//! i.e. 16 `Counters[ct] RoomSaved[ct]` pairs (this crate's `counters`/
//! `saved_rooms` op-81/op-87 backup registers), one state line (`BitFlags`,
//! a redundant/backward-compat `DarkFlag` bit ORed into bit 15, the player's
//! room, the live current counter, the op-80 saved-room register, and the
//! lamp fuel), then `NumItems+1` item locations. `fscanf`'s `%d`/`%hd`/`%ld`
//! all skip leading whitespace (including newlines) exactly like the `.dat`
//! lexer's own tokens, so this reader is the same whitespace/integer lexer
//! shape as `loader.rs`'s, not a line-oriented parser.
//!
//! Fields this crate's `Vm` has no equivalent for (there are none — every
//! `LoadGame` field maps onto an existing `Vm` field) or that `SaveGame`
//! never wrote (there are none either) are not a concern here; the two
//! formats happen to carry exactly the same mutable state, just serialized
//! differently.

use crate::database::CARRIED;
use crate::vm::RestoreError;
use crate::Vm;

/// Tokenizer over whitespace-separated decimal integers — the same shape as
/// `loader::Lexer::next_int`, kept separate because a ScottFree save has no
/// quoted strings to lex and needn't take the `.dat` loader's byte-vs-`&str`
/// generality.
struct Tokens<'a> {
    rest: std::str::SplitAsciiWhitespace<'a>,
}

impl<'a> Tokens<'a> {
    fn new(src: &'a str) -> Self {
        Tokens { rest: src.split_ascii_whitespace() }
    }
    fn next_i64(&mut self) -> Result<i64, RestoreError> {
        self.rest
            .next()
            .ok_or(RestoreError::ScottFreeMalformed("save data ended early"))?
            .parse::<i64>()
            .map_err(|_| RestoreError::ScottFreeMalformed("expected an integer"))
    }
    fn next_usize(&mut self, max_exclusive: usize) -> Result<usize, RestoreError> {
        let v = self.next_i64()?;
        if v < 0 || v as usize >= max_exclusive {
            return Err(RestoreError::OutOfRange);
        }
        Ok(v as usize)
    }
}

/// Cheap shape sniff for a ScottFree 1.14 save file, so a host can tell one
/// apart from this crate's own binary [`crate::Vm::snapshot`] (whose
/// [`crate::Vm::SNAPSHOT_MAGIC`] a caller checks first — it is unambiguous —
/// before ever reaching this function) and route to
/// [`Vm::restore_scottfree`] instead (SQ-1413).
///
/// The FIRST 38 whitespace-separated tokens (16 `Counters[ct] RoomSaved[ct]`
/// pairs, then the 6-int state line — see the module doc for the exact
/// field order `LoadGame` reads) must all parse as plain decimal integers.
/// Unlike [`crate::looks_like_scott`]'s 12-int `.dat` header sniff, a save
/// has no file-specific counts to cross-check against — every field's WIDTH
/// is fixed by the format itself — so this is an exact shape check on a
/// fixed prefix, not a heuristic guess at a variable one.
///
/// Detection lives in exactly this one place so a host does not duplicate
/// the rule: `ScottSession::restore_game_save` (lanthorn's own app) and
/// `scott-cli`'s `/restore` both call this, after ruling out the binary
/// magic, before `Vm::restore_scottfree`.
///
/// Not perfectly exclusive of a `.dat` GAME file — a database's own header
/// and action table are also runs of decimal integers, and nothing here
/// reads far enough to rule one out by content. In practice this only
/// matters if a host offers this check somewhere a full `.dat` could
/// plausibly be picked as a SAVE target, which none of this crate's own
/// callers do (a save restore and a fresh game load are different
/// operations in both the app and `scott-cli`).
pub fn looks_like_scottfree_save(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut tokens = text.split_ascii_whitespace();
    for _ in 0..38 {
        match tokens.next() {
            Some(tok) if tok.parse::<i64>().is_ok() => {}
            _ => return false,
        }
    }
    true
}

impl Vm {
    /// Restore state from ScottFree 1.14's own save-file text format (see the
    /// module docs for the exact field order). Distinct from
    /// [`Vm::restore`]/[`Vm::snapshot`], this crate's own binary format —
    /// a host wanting to accept EITHER format detects by shape first (a
    /// ScottFree save is plain ASCII digits and whitespace; this crate's own
    /// snapshot starts with the 4-byte [`Vm::SNAPSHOT_MAGIC`], which a valid
    /// ScottFree save can never begin with, since `b'S'` fscanf's as
    /// [`RestoreError::ScottFreeMalformed`] rather than an integer).
    ///
    /// Every failure mode is a [`RestoreError`] and nothing is written until
    /// the whole file parses successfully AND every index it names fits this
    /// game's own tables — the same all-or-nothing discipline as `restore`,
    /// so a save from a different (or corrupt) `.dat` is refused rather than
    /// half-applied.
    ///
    /// Fields a ScottFree save carries that this crate's `Vm` has no analog
    /// for: none — `Counters`/`RoomSaved` are this crate's `counters`/
    /// `saved_rooms` (op-81/op-87 backup registers), `BitFlags`/`DarkFlag`
    /// are `flags`, and the rest (room, current counter, saved room, lamp
    /// fuel, item locations) map onto the same-named `Vm` fields
    /// [`Vm::snapshot`] itself carries.
    pub fn restore_scottfree(&mut self, bytes: &[u8]) -> Result<(), RestoreError> {
        // ScottFree's own file is plain ASCII (fscanf'd integers separated by
        // spaces/newlines); non-UTF-8 input can't be this format at all.
        let text = std::str::from_utf8(bytes)
            .map_err(|_| RestoreError::ScottFreeMalformed("not ASCII/UTF-8 text"))?;
        let mut t = Tokens::new(text);

        let mut counters = [0i32; 16];
        let mut saved_rooms = [0usize; 16];
        for i in 0..16 {
            counters[i] = t.next_i64()? as i32;
            // ScottFree's RoomSaved values are room indices (`fscanf("%d")`,
            // signed) but this crate stores them `usize`; a negative or
            // out-of-range room here is refused like any other bad index.
            saved_rooms[i] = t.next_usize(self.db.rooms.len())?;
        }

        // `BitFlags` (`%ld`): read as i64 so a 32-bit `long`'s sign-extended
        // decimal (a bit >=31 set reads negative under `%ld` on a 32-bit
        // build) still recovers the original low-32 bit pattern once
        // reinterpreted as unsigned — see the module doc.
        let bit_flags = t.next_i64()? as u64;
        let mut flags = [false; 32];
        for (i, f) in flags.iter_mut().enumerate() {
            *f = (bit_flags >> i) & 1 == 1;
        }
        // `DarkFlag` (`%d`): redundant backward-compat bit ScottFree ORs into
        // bit 15 (`crate::database::DARK_FLAG`) after loading `BitFlags` —
        // `if(DarkFlag) BitFlags|=(1<<15);` (LoadGame, ScottCurses.c:698-699).
        // OR, never clear: a zero here must not un-set a bit `BitFlags`
        // already carried.
        let dark_flag = t.next_i64()?;
        if dark_flag != 0 {
            flags[crate::database::DARK_FLAG] = true;
        }

        let player = t.next_usize(self.db.rooms.len())?;
        let current_counter = t.next_i64()? as i32;
        let saved_room = t.next_usize(self.db.rooms.len())?;
        let lamp = t.next_i64()? as i32;

        // Item locations: `NumItems+1` of them (`Items[ct].Location`, an
        // `unsigned char` — 255 means CARRIED, normalised to this crate's -1
        // sentinel exactly as the `.dat` loader normalises `start_loc`
        // (loader.rs, SQ-1412).
        let mut item_loc = Vec::with_capacity(self.item_loc.len());
        for _ in 0..self.item_loc.len() {
            let v = t.next_i64()?;
            item_loc.push(if v == 255 { CARRIED } else { v as i32 });
        }
        if t.rest.next().is_some() {
            return Err(RestoreError::TrailingData);
        }

        self.counters = counters;
        self.saved_rooms = saved_rooms;
        self.flags = flags;
        self.player = player;
        self.current_counter = current_counter;
        self.saved_room = saved_room;
        self.lamp = lamp;
        self.item_loc = item_loc;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Database, Item, Room};

    fn db_with_items(n: usize) -> Database {
        Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 3,
            light_time: 100,
            treasure_room: 0,
            actions: vec![],
            verbs: vec![String::new()],
            nouns: vec![String::new()],
            rooms: (0..3).map(|i| Room { exits: [0; 6], desc: format!("room{i}"), literal: true }).collect(),
            messages: vec![],
            items: (0..n)
                .map(|_| Item { text: "thing".into(), treasure: false, auto_noun: None, start_loc: 0 })
                .collect(),
            adventure_number: 0,
        }
    }

    /// Hand-authored ScottFree save (per LoadGame's field order): 16
    /// `counter room` pairs, a state line, then one location per item. Built
    /// by hand from the format documented above (no `cc`/ScottFree binary
    /// available in this environment) — every field lands in a distinct,
    /// checkable slot so a transposition would fail the assertions below.
    fn hand_authored_save(bit_flags: i64, dark_flag: i64) -> String {
        let mut s = String::new();
        for ct in 0..16 {
            // Counters[ct] = ct+1 (distinct per slot), RoomSaved[ct] = 1 (a valid room).
            s.push_str(&format!("{} 1\n", ct + 1));
        }
        // BitFlags DarkFlag MyLoc CurrentCounter SavedRoom LightTime
        s.push_str(&format!("{bit_flags} {dark_flag} 2 42 1 7\n"));
        // 3 items: carried (255), nowhere (0), in room 2.
        s.push_str("255\n0\n2\n");
        s
    }

    #[test]
    fn round_trip_fields_land_in_their_own_slots() {
        let mut vm = Vm::new(db_with_items(3));
        let save = hand_authored_save(0b101, 0); // bits 0 and 2 set, no DarkFlag
        vm.restore_scottfree(save.as_bytes()).expect("well-formed save parses");

        assert_eq!(vm.current_room(), 2, "MyLoc");
        assert_eq!(vm.counter(), 42, "CurrentCounter");
        assert_eq!(vm.saved_room(), 1, "SavedRoom");
        assert_eq!(vm.lamp(), 7, "LightTime -> lamp");
        assert!(vm.flag(0) && vm.flag(2) && !vm.flag(1), "BitFlags decoded per-bit");
        assert_eq!(vm.item_loc(0), CARRIED, "255 normalises to CARRIED");
        assert_eq!(vm.item_loc(1), 0);
        assert_eq!(vm.item_loc(2), 2);
        assert_eq!(vm.backup_counter_at(0), 1, "Counters[0]");
        assert_eq!(vm.backup_counter_at(15), 16, "Counters[15]");
        assert_eq!(vm.saved_rooms_at(0), 1, "RoomSaved[0]");
    }

    #[test]
    fn dark_flag_ors_bit_15_without_clearing_it() {
        let mut vm = Vm::new(db_with_items(3));
        // BitFlags has NOTHING set; DarkFlag=1 must still set bit 15 (DARK_FLAG).
        let save = hand_authored_save(0, 1);
        vm.restore_scottfree(save.as_bytes()).unwrap();
        assert!(vm.flag(crate::database::DARK_FLAG), "DarkFlag=1 sets bit 15 via OR");
    }

    #[test]
    fn malformed_input_is_refused_not_panicking() {
        let mut vm = Vm::new(db_with_items(3));
        assert!(vm.restore_scottfree(b"").is_err());
        assert!(vm.restore_scottfree(b"not a number at all").is_err());
        // Truncated mid-header.
        assert!(vm.restore_scottfree(b"1 1\n2 1\n").is_err());
        // A binary snapshot's magic is not ASCII/whitespace-int shaped either.
        let snap = vm.snapshot();
        assert!(vm.restore_scottfree(&snap).is_err());
    }

    #[test]
    fn wrong_item_count_is_out_of_range_not_a_panic() {
        // Save authored for 3 items, restored against a 5-item database.
        let mut vm = Vm::new(db_with_items(5));
        let save = hand_authored_save(0, 0);
        assert!(vm.restore_scottfree(save.as_bytes()).is_err());
    }

    #[test]
    fn trailing_data_after_a_complete_save_is_refused() {
        let mut vm = Vm::new(db_with_items(3));
        let mut save = hand_authored_save(0, 0);
        save.push_str("999\n");
        assert!(matches!(
            vm.restore_scottfree(save.as_bytes()),
            Err(RestoreError::TrailingData)
        ));
    }

    #[test]
    fn a_failed_restore_leaves_state_untouched() {
        let mut vm = Vm::new(db_with_items(3));
        vm.set_player(2);
        let before = vm.current_room();
        assert!(vm.restore_scottfree(b"garbage").is_err());
        assert_eq!(vm.current_room(), before, "no partial write on failure");
    }

    // ── looks_like_scottfree_save (SQ-1413) ───────────────────────────────

    #[test]
    fn a_well_formed_save_is_recognised() {
        let save = hand_authored_save(0, 0);
        assert!(looks_like_scottfree_save(save.as_bytes()));
    }

    #[test]
    fn this_crates_own_binary_snapshot_is_not_mistaken_for_one() {
        let vm = Vm::new(db_with_items(3));
        let snap = vm.snapshot();
        assert!(!looks_like_scottfree_save(&snap), "ScSv-magic bytes are not decimal-integer text");
    }

    #[test]
    fn garbage_is_refused() {
        assert!(!looks_like_scottfree_save(b""));
        assert!(!looks_like_scottfree_save(b"not a number at all"));
        assert!(!looks_like_scottfree_save(b"1 1\n2 1\n")); // truncated: far short of 38 tokens
        assert!(!looks_like_scottfree_save(&[0xffu8; 40])); // non-UTF-8 bytes
    }
}
