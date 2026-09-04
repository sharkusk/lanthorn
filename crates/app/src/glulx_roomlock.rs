//! Learn which RAM word holds a Glulx game's *current room*, and use it as the
//! room's identity (SQ-0526).
//!
//! Glulx exposes no object tree, so lanthorn recovers the current room from the
//! room HEADING the game prints and hashes that name into a room id. Any two
//! rooms sharing a name are therefore the same room as far as the map is
//! concerned — which is exactly how Adventure's maze collapses into a single
//! node no matter how long you wander it.
//!
//! The identity does exist, though: an Inform game keeps the current room in its
//! `location` global, and the value is the room's object address. Nothing tells
//! us where that global lives — `advent.blb` supplies no `@accelparam` metadata —
//! but it can be *found*, because it is the word whose changes coincide with the
//! room changing. Diffing RAM each turn and scoring every word on that
//! correlation converges within a handful of moves: in Adventure the winner is
//! `ramstart+0x28` with a perfect record, and it cleanly separates three maze
//! rooms that all print the heading "Maze".
//!
//! This mirrors how the Z-machine side locks onto the player object: guess from
//! observed behaviour, commit once the evidence is one-sided, and give up the
//! guess the moment it is contradicted.
//!
//! ## Scoring only the turns we are sure about
//!
//! The correlation is only meaningful on turns whose room-change status is not in
//! doubt, and one case must be excluded or the whole scheme defeats itself:
//!
//! * **Changed** — a heading was printed and differs from the last one.
//! * **Unchanged** — no heading was printed at all (`take`, `wait`, a refused move).
//! * **Ambiguous** — a heading was printed and *equals* the last one. That is
//!   either a `look`, or a move into a DIFFERENT room bearing the SAME NAME.
//!   Scoring these would punish the correct candidate on every maze step — the
//!   one place the whole feature has to work.
//!
//! ## What a surviving candidate's VALUE has to look like
//!
//! Correlation alone leaves a handful of words standing, so the winner also has
//! to hold something that could BE a room — and the only exact answer to that is
//! the story's own object table, which `gvm::objects::ParseNames` walks
//! (SQ-1286). [`RoomLock::set_objects`] hands it over; a candidate whose value is
//! not one of those addresses is not a room, whatever it correlates with.
//!
//! It used to be approximated by "a nonzero address inside the scanned RAM
//! window", and that approximation is why the lock had never resolved on most of
//! the corpus. The window is 64 KB from `ramstart` — plenty for the GLOBAL, which
//! Inform lays out at the very start of RAM — but an Inform story's object table
//! sits *after* its globals and arrays, and on all but the smallest games that is
//! far beyond the window: measured across the 42 Glulx stories in `stories/`,
//! only five keep their objects within 64 KB of `ramstart`. Counterfeit Monkey's
//! `location` global is `ramstart+0x98` and holds `0x5440b3` — an object **1.9 MB
//! above** the window — so the true candidate was thrown away every turn and the
//! game keyed rooms by name hash for the whole session. The fallback below is
//! still the old range test, for a story whose object table cannot be found at
//! all.

/// How many confidently-observed room changes must agree before a candidate is
/// trusted. Three is enough to shake out counters and turn tallies (which change
/// every turn, so they fail the "unchanged" turns) while still locking within the
/// opening moves of a game.
const REQUIRED_CHANGES: u32 = 3;

/// How many confidently-observed *heading-less* turns must also agree. Without
/// this a game whose first moves are all room changes could lock onto a plain
/// move counter, which is indistinguishable from the room until something fails
/// to move the player.
const REQUIRED_STILLS: u32 = 1;

/// The most learning snapshots retained. Each is one `u32` per RAM word (~53 KB
/// for Adventure), kept only until the lock resolves — they are what lets the
/// pre-lock rooms be re-keyed to their real ids once the address is known.
const MAX_HISTORY: usize = 12;

/// What the observer could tell about a turn's room change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Movement {
    /// A heading was printed and differs from the previous one.
    Changed,
    /// No heading was printed: the game did not move the player.
    Unchanged,
    /// A heading was printed but repeats the previous one — a `look`, or a move
    /// into a different room with the same name. Carries no information.
    Ambiguous,
}

/// One learning turn: the RAM snapshot taken after it, and the heading in force.
struct Observation {
    ram: Vec<u32>,
    heading: Option<String>,
}

/// The learn/lock/unlock state machine.
pub struct RoomLock {
    /// Base address of the scanned region; `ram[i]` is the word at `base + i*4`.
    base: u32,
    /// Per-word agreement / disagreement tallies while learning.
    agree: Vec<u32>,
    disagree: Vec<u32>,
    /// Confidently-observed turns of each kind so far.
    changes: u32,
    stills: u32,
    /// Retained learning turns, oldest first (see [`MAX_HISTORY`]).
    history: Vec<Observation>,
    /// The previous turn's snapshot and heading.
    prev: Option<Observation>,
    /// The story's object addresses, sorted — what a candidate's VALUE is
    /// checked against (SQ-1286). `None` until [`RoomLock::set_objects`] supplies
    /// them, and after a story whose object table could not be walked at all.
    objects: Option<Vec<u32>>,
    /// The locked address, once the evidence is one-sided.
    locked: Option<u32>,
    /// Set for one call after a lock resolves, so the caller can re-key the rooms
    /// that were mapped under name-derived ids before the address was known.
    pending_remap: Vec<(String, u32)>,
}

impl RoomLock {
    /// A learner for the RAM region starting at `base`, `words` words long.
    pub fn new(base: u32, words: usize) -> Self {
        RoomLock {
            base,
            agree: vec![0; words],
            disagree: vec![0; words],
            changes: 0,
            stills: 0,
            history: Vec::new(),
            prev: None,
            objects: None,
            locked: None,
            pending_remap: Vec::new(),
        }
    }

    /// A learner that starts already locked to `addr` — the per-game sidecar
    /// remembers the address across runs, and object addresses are fixed for a
    /// given story file, so every run after the first identifies rooms from its
    /// very first turn and never maps one under a name-derived id at all.
    pub fn locked_at(base: u32, words: usize, addr: u32) -> Self {
        let mut l = RoomLock::new(base, words);
        l.locked = Some(addr);
        l
    }

    /// The locked address, or `None` while still learning.
    pub fn locked(&self) -> Option<u32> {
        self.locked
    }

    /// True while no object table has been supplied — the caller's cue to walk
    /// one and hand it over. Asked rather than pushed on every turn because
    /// deriving the table is a whole-image scan, and a learner only ever needs
    /// it once.
    pub fn needs_objects(&self) -> bool {
        self.objects.is_none()
    }

    /// Supply the story's object addresses, in any order — what
    /// [`RoomLock::try_lock`] checks a candidate's VALUE against. `None` means
    /// the story's object table could not be walked, and leaves the learner on
    /// the range approximation the module docs describe.
    pub fn set_objects(&mut self, addrs: Option<Vec<u32>>) {
        self.objects = addrs.map(|mut v| {
            v.sort_unstable();
            v.dedup();
            v
        });
    }

    /// What the LOCKED word says about this turn — the story's own answer to "did
    /// the player change rooms?", which outranks the heading once it is available
    /// (SQ-1294). `None` while still learning, and for an address outside the
    /// scanned window; the caller then falls back to comparing headings, which is
    /// all it ever had.
    ///
    /// Never [`Movement::Ambiguous`]: that state exists only because two rooms can
    /// print one name, and an address cannot be ambiguous about itself. A maze step
    /// is a plain `Changed` here, which is the whole reason the lock is worth
    /// having.
    ///
    /// The first turn of a session that booted already locked (the per-game
    /// `room-global` sidecar) has no predecessor to compare against, and is an
    /// arrival by definition — you are somewhere, and nothing has told the map
    /// where yet — so it answers `Changed` rather than refusing.
    pub fn movement(&self, ram: &[u32]) -> Option<Movement> {
        let idx = self.word_index(self.locked?)?;
        let Some(prev) = &self.prev else { return Some(Movement::Changed) };
        let (Some(&a), Some(&b)) = (prev.ram.get(idx), ram.get(idx)) else {
            return Some(Movement::Changed);
        };
        Some(if a != b { Movement::Changed } else { Movement::Unchanged })
    }

    /// The room id for the current turn: the locked word's value, or `None` while
    /// learning — the caller then falls back to hashing the room name, exactly as
    /// before this existed.
    pub fn room_id(&self, ram: &[u32]) -> Option<u32> {
        let idx = self.word_index(self.locked?)?;
        ram.get(idx).copied().filter(|&v| v != 0)
    }

    /// The index into a scanned-RAM snapshot for `addr`, or `None` when the
    /// address is not inside the scanned window.
    ///
    /// The locked address does not have to come from the learner: it is also read
    /// back from the per-game `room-global` sidecar, a plain text file a user can
    /// edit and a stale one can outlive a story rebuild. A value BELOW `base`
    /// underflowed `addr - self.base` — a panic every turn in a debug build, and a
    /// wild index in release. Out of range simply means "this lock tells us
    /// nothing": the caller falls back to the name-derived id, and `verify` drops
    /// the lock so the learner starts over. (SQ-0658)
    fn word_index(&self, addr: u32) -> Option<usize> {
        addr.checked_sub(self.base).map(|off| (off / 4) as usize)
    }

    /// Take the re-key table produced when a lock resolves: `(room name, real id)`
    /// for each room seen while learning. Empty except on the turn a lock lands,
    /// and empty entirely for a learner that started locked.
    pub fn take_remap(&mut self) -> Vec<(String, u32)> {
        std::mem::take(&mut self.pending_remap)
    }

    /// Fold this turn into the model. `ram` is the snapshot taken after the turn
    /// ran; `heading` is the room heading in force (sticky across heading-less
    /// turns); `movement` is what the caller could tell about the room changing.
    pub fn observe(&mut self, ram: Vec<u32>, heading: Option<String>, movement: Movement) {
        if let Some(addr) = self.locked {
            self.verify(&ram, addr);
            self.prev = Some(Observation { ram, heading });
            return;
        }
        if let (Some(prev), Movement::Changed | Movement::Unchanged) = (&self.prev, movement) {
            let expect_change = movement == Movement::Changed;
            for (i, (a, b)) in prev.ram.iter().zip(ram.iter()).enumerate() {
                // A word out of range of the shorter snapshot is simply not scored.
                if (a != b) == expect_change {
                    self.agree[i] += 1;
                } else {
                    self.disagree[i] += 1;
                }
            }
            if expect_change {
                self.changes += 1;
            } else {
                self.stills += 1;
            }
        }
        if self.history.len() == MAX_HISTORY {
            self.history.remove(0);
        }
        self.history.push(Observation { ram: ram.clone(), heading: heading.clone() });
        self.prev = Some(Observation { ram, heading });
        self.try_lock();
    }

    /// Commit to a candidate once the evidence is one-sided.
    fn try_lock(&mut self) {
        if self.changes < REQUIRED_CHANGES || self.stills < REQUIRED_STILLS {
            return;
        }
        let scored = self.changes + self.stills;
        // Survivors: perfect agreement across every turn we were sure about, and a
        // value that really is an object of this story (or, with no object table
        // to consult, one that could be — see the module docs). The counters and
        // flags that merely correlate get filtered by the heading-less turns; this
        // filters the rest.
        let end = self.base + (self.agree.len() as u32) * 4;
        let cur = match self.history.last() {
            Some(o) => &o.ram,
            None => return,
        };
        let mut best: Option<u32> = None;
        for i in 0..self.agree.len() {
            if self.agree[i] != scored || self.disagree[i] != 0 {
                continue;
            }
            let v = cur.get(i).copied().unwrap_or(0);
            if !self.is_room_value(v, end) {
                continue;
            }
            // Aliases of the same global are common (`real_location`, the player's
            // parent). Any of them identifies the room, so take the lowest address
            // for a deterministic, reproducible choice.
            let addr = self.base + (i as u32) * 4;
            if best.is_none_or(|b| addr < b) {
                best = Some(addr);
            }
        }
        let Some(addr) = best else { return };
        self.locked = Some(addr);
        // Rooms already mapped under a name-derived id: hand back the real id for
        // each heading seen while learning, so the caller can re-key them instead
        // of leaving a duplicate behind.
        let idx = ((addr - self.base) / 4) as usize;
        let mut seen: Vec<(String, u32)> = Vec::new();
        for o in &self.history {
            let (Some(name), Some(&v)) = (o.heading.as_ref(), o.ram.get(idx)) else { continue };
            if v != 0 && !seen.iter().any(|(n, id)| n == name && *id == v) {
                seen.push((name.clone(), v));
            }
        }
        self.pending_remap = seen;
        // The learning history has done its job; release it.
        self.history = Vec::new();
        self.agree = Vec::new();
        self.disagree = Vec::new();
    }

    /// Could `v` be a room? Exactly — it is one of the story's own objects — when
    /// an object table has been supplied, and approximately — a nonzero address
    /// inside the scanned window — when none could be walked. See the module docs
    /// for why the approximation alone is not enough (SQ-1286).
    fn is_room_value(&self, v: u32, end: u32) -> bool {
        match &self.objects {
            Some(objs) => objs.binary_search(&v).is_ok(),
            None => v != 0 && v >= self.base && v < end,
        }
    }

    /// Once locked, keep checking — but check the RIGHT thing (SQ-1294).
    ///
    /// This used to drop the lock whenever the locked word disagreed with what the
    /// printed heading implied, and that is backwards: the heading is the weaker
    /// witness of the two, and the turns where they disagree are exactly the turns
    /// the lock exists for. Counterfeit Monkey drives its car from Deep Street to
    /// the Traffic Circle and prints no heading at all; Counterfeit Monkey's
    /// `remember` prints a flashback heading for a yacht galley without moving the
    /// player anywhere. Under the old rule each of those threw away a lock that was
    /// telling the truth, and every room until it re-resolved was keyed by the hash
    /// of a room NAME again — which is how one Deep Street becomes two.
    ///
    /// So the only thing that can falsify a lock now is its own VALUE: a word that
    /// no longer holds one of this story's objects is not the `location` global,
    /// whatever the screen says. A zero is not evidence either way (a game may park
    /// `location` at nothing for a turn mid-scene) and [`RoomLock::room_id`] already
    /// refuses to identify a room from one.
    fn verify(&mut self, ram: &[u32], addr: u32) {
        // An address outside the scanned window can never be verified against a
        // snapshot, so it can never be dropped by the check below either — it would
        // be a permanently unfalsifiable lock. Drop it here instead and re-learn.
        // (SQ-0658; see `word_index` for where such an address comes from.)
        let Some(idx) = self.word_index(addr) else {
            self.relearn(ram.len());
            return;
        };
        let Some(&v) = ram.get(idx) else { return };
        if v == 0 {
            return;
        }
        // `agree` is released the moment a lock resolves, so the fallback range test
        // takes its bound from the snapshot in hand rather than from that vector.
        let end = self.base + (ram.len() as u32) * 4;
        if !self.is_room_value(v, end) {
            self.relearn(ram.len());
        }
    }

    /// Throw the model away and learn again from scratch, keeping the object
    /// table — it is a property of the STORY, not of the guess that just failed,
    /// and re-deriving it means another whole-image scan.
    fn relearn(&mut self, words: usize) {
        let objects = self.objects.take();
        *self = RoomLock::new(self.base, words);
        self.objects = objects;
    }
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;

    /// Drive the learner with a synthetic "RAM": word 0 is the room (changes only
    /// on a move), word 1 a turn counter (changes every turn), word 2 a constant.
    /// Only word 0 can survive both kinds of evidence.
    fn drive(l: &mut RoomLock, steps: &[(Movement, u32)]) {
        let mut counter = 0u32;
        for (i, &(mv, room)) in steps.iter().enumerate() {
            counter += 1;
            let ram = vec![room, counter, 7];
            let heading = Some(format!("room-{room}"));
            let _ = i;
            l.observe(ram, heading, mv);
        }
    }

    /// The base is 0x1000 and words are 4 bytes apart, so word 0 is 0x1000. Its
    /// values must look like addresses inside the region for the lock to accept
    /// them, mirroring the real object-address check.
    fn base_region() -> (u32, usize) {
        (0x1000, 3)
    }

    #[test]
    fn a_lock_below_the_scan_base_is_rejected_rather_than_underflowing() {
        // SQ-0658: the locked address is not always something the learner chose —
        // it is also read back from the per-game `room-global` sidecar, a plain
        // text file that a user can edit and that can outlive a story rebuild. An
        // address BELOW `base` underflowed `addr - self.base`: a debug panic on
        // every single turn, and a wild index in release.
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, base - 0x400);
        let ram = vec![0x1000, 1, 7];

        assert_eq!(l.room_id(&ram), None, "an out-of-window lock identifies no room");

        // And it must not be sticky: it can never be checked against a snapshot, so
        // it could never be falsified by the usual `verify` either. Drop it and
        // learn again from scratch.
        l.observe(ram, Some("Hall".to_string()), Movement::Changed);
        assert_eq!(l.locked(), None, "an unverifiable lock is dropped, not kept forever");
    }

    #[test]
    fn locks_onto_the_word_that_tracks_the_room() {
        let (base, words) = base_region();
        let mut l = RoomLock::new(base, words);
        drive(
            &mut l,
            &[
                (Movement::Unchanged, 0x1000),
                (Movement::Changed, 0x1004),
                (Movement::Unchanged, 0x1004),
                (Movement::Changed, 0x1008),
                (Movement::Changed, 0x1000),
            ],
        );
        assert_eq!(l.locked(), Some(0x1000), "the room word is the only one that survives both kinds of turn");
    }

    #[test]
    fn a_turn_counter_is_rejected() {
        // Every turn is a room change: a counter correlates perfectly and would
        // lock if heading-less turns were not required. REQUIRED_STILLS is what
        // makes the counter fail, so with none of them nothing may lock at all.
        let (base, words) = base_region();
        let mut l = RoomLock::new(base, words);
        drive(
            &mut l,
            &[
                (Movement::Changed, 0x1004),
                (Movement::Changed, 0x1008),
                (Movement::Changed, 0x1004),
                (Movement::Changed, 0x1008),
            ],
        );
        assert_eq!(l.locked(), None, "with no heading-less turn a counter is indistinguishable from the room");
    }

    #[test]
    fn ambiguous_turns_are_not_scored() {
        // A same-named room arrival (the maze) must not count as "unchanged" — the
        // room word DOES change there, and scoring it would penalise the truth.
        let (base, words) = base_region();
        let mut l = RoomLock::new(base, words);
        drive(
            &mut l,
            &[
                // The first turn has no predecessor, so it is never scored; the
                // second is the one that supplies the heading-less evidence.
                (Movement::Unchanged, 0x1000),
                (Movement::Unchanged, 0x1000),
                (Movement::Changed, 0x1004),
                (Movement::Ambiguous, 0x1008), // maze step: same name, different room
                (Movement::Ambiguous, 0x1000), // and another
                (Movement::Changed, 0x1004),
                (Movement::Changed, 0x1008),
            ],
        );
        assert_eq!(
            l.locked(),
            Some(0x1000),
            "maze steps carry no information and must leave the correct candidate unpenalised"
        );
    }

    #[test]
    fn an_object_outside_the_scan_window_is_still_a_room() {
        // SQ-1286: the value filter used to demand an address inside the scanned
        // region, and an Inform story's object table sits after its globals and
        // arrays — beyond that region on all but the smallest games. Counterfeit
        // Monkey's `location` is `ramstart+0x98` and holds an object 1.9 MB
        // higher, so the one true candidate was thrown away every turn.
        let (base, words) = base_region();
        let far = base + 0x4000; // an object well past `base + words*4`
        let mut l = RoomLock::new(base, words);
        l.set_objects(Some(vec![far + 0x20, far, far + 0x10]));
        drive(
            &mut l,
            &[
                (Movement::Unchanged, far),
                (Movement::Changed, far + 0x10),
                (Movement::Unchanged, far + 0x10),
                (Movement::Changed, far + 0x20),
                (Movement::Changed, far),
            ],
        );
        assert_eq!(
            l.locked(),
            Some(base),
            "a candidate holding a real object of the story locks however far that object is"
        );
    }

    #[test]
    fn a_word_that_correlates_but_holds_no_object_is_rejected() {
        // The other half: knowing the object table also RULES OUT a word that
        // tracks the room perfectly and holds something that is not a room. The
        // old range test could not tell those apart, and took the lowest address.
        let (base, words) = base_region();
        let mut l = RoomLock::new(base, words);
        l.set_objects(Some(vec![0x2000, 0x2004, 0x2008]));
        drive(
            &mut l,
            &[
                (Movement::Unchanged, 0x1000),
                (Movement::Changed, 0x1004),
                (Movement::Unchanged, 0x1004),
                (Movement::Changed, 0x1008),
                (Movement::Changed, 0x1000),
            ],
        );
        assert_eq!(
            l.locked(),
            None,
            "the word tracks the room but holds no object of this story, so it is not the room"
        );
    }

    #[test]
    fn with_no_object_table_the_range_approximation_still_applies() {
        // A story whose object table cannot be walked keeps exactly the behaviour
        // it had: the in-window range test, and the lock it always produced.
        let (base, words) = base_region();
        let mut l = RoomLock::new(base, words);
        l.set_objects(None);
        assert!(l.needs_objects(), "`None` is not an answer, so the caller may try again");
        drive(
            &mut l,
            &[
                (Movement::Unchanged, 0x1000),
                (Movement::Changed, 0x1004),
                (Movement::Unchanged, 0x1004),
                (Movement::Changed, 0x1008),
                (Movement::Changed, 0x1000),
            ],
        );
        assert_eq!(l.locked(), Some(0x1000), "unchanged from before the object table existed");
    }

    #[test]
    fn a_relearn_keeps_the_object_table() {
        // The table is the STORY's, not the guess's: re-deriving it is a whole
        // image scan, and a falsified lock says nothing about it.
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, 0x1000);
        l.set_objects(Some(vec![0x2000, 0x2004, 0x2008]));
        l.observe(vec![0x1000, 1, 7], Some("a".into()), Movement::Unchanged);
        assert_eq!(l.locked(), None, "the locked word holds nothing that is a room of this story");
        assert!(!l.needs_objects(), "…but the object table survived the re-learn");
    }

    #[test]
    fn room_id_reads_the_locked_word() {
        let (base, words) = base_region();
        let l = RoomLock::locked_at(base, words, 0x1004);
        assert_eq!(l.room_id(&[0x1000, 0x1234, 7]), Some(0x1234));
        assert_eq!(l.room_id(&[0x1000, 0, 7]), None, "a zero global is no identity");
    }

    #[test]
    fn a_lock_whose_value_stops_being_an_object_is_dropped() {
        // The one thing that can still falsify a lock: the word is no longer holding
        // a room. A stale `room-global` sidecar is where a wrong address comes from,
        // and a wrong lock must never be sticky — it would mis-key every room after it.
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, 0x1000);
        l.set_objects(Some(vec![0x2000, 0x2004]));
        l.observe(vec![0x1000, 1, 7], Some("a".into()), Movement::Unchanged);
        assert_eq!(l.locked(), None, "0x1000 is not one of this story's objects");
    }

    #[test]
    fn a_zero_is_not_evidence_against_a_lock() {
        // A game may park `location` at nothing for a turn mid-scene. `room_id`
        // already refuses to name a room from a zero; throwing the lock away for it
        // would cost every later room its identity for the sake of one turn.
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, 0x1000);
        l.set_objects(Some(vec![0x2000, 0x2004]));
        l.observe(vec![0, 1, 7], Some("a".into()), Movement::Unchanged);
        assert_eq!(l.locked(), Some(0x1000), "a zero says nothing");
        assert_eq!(l.room_id(&[0, 1, 7]), None, "…and still identifies no room");
    }

    #[test]
    fn a_disagreeing_heading_no_longer_drops_the_lock() {
        // SQ-1294, both shapes at once. First a heading that says the room changed
        // while the locked word stood still (Counterfeit Monkey's `remember` prints a
        // flashback heading for a yacht galley without moving the player); then the
        // reverse, the locked word moving with no heading printed at all (the car
        // driving out of Deep Street). Each used to `relearn`, and every room after
        // it was keyed by the hash of a NAME again.
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, 0x1000);
        l.set_objects(Some(vec![0x2000, 0x2004]));
        l.observe(vec![0x2000, 1, 7], Some("Dormitory Room".into()), Movement::Unchanged);
        l.observe(vec![0x2000, 2, 7], Some("Galley".into()), Movement::Changed);
        assert_eq!(l.locked(), Some(0x1000), "a flashback heading is not evidence about the lock");
        l.observe(vec![0x2004, 3, 7], None, Movement::Unchanged);
        assert_eq!(l.locked(), Some(0x1000), "nor is a move the story narrated without a heading");
    }

    #[test]
    fn movement_comes_from_the_locked_word() {
        // The lock's own verdict, which is what `GlulxSession` now folds in instead of
        // comparing room NAMES. Note there is no `Ambiguous` here: two rooms can share
        // a name, but an address cannot be ambiguous about itself, so a maze step is a
        // plain `Changed`.
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, 0x1000);
        l.set_objects(Some(vec![0x2000, 0x2004]));
        assert_eq!(
            l.movement(&[0x2000, 1, 7]),
            Some(Movement::Changed),
            "the first turn of a session that booted locked is an arrival"
        );
        l.observe(vec![0x2000, 1, 7], Some("Maze".into()), Movement::Changed);
        assert_eq!(l.movement(&[0x2000, 2, 7]), Some(Movement::Unchanged));
        assert_eq!(l.movement(&[0x2004, 2, 7]), Some(Movement::Changed), "a maze step is a move");
        assert_eq!(RoomLock::new(base, words).movement(&[0x2000, 1, 7]), None, "silent while learning");
    }

    #[test]
    fn a_resolved_lock_reports_the_rooms_seen_while_learning() {
        let (base, words) = base_region();
        let mut l = RoomLock::new(base, words);
        drive(
            &mut l,
            &[
                (Movement::Unchanged, 0x1000),
                (Movement::Changed, 0x1004),
                (Movement::Unchanged, 0x1004),
                (Movement::Changed, 0x1008),
                (Movement::Changed, 0x1000),
            ],
        );
        assert_eq!(l.locked(), Some(0x1000));
        let remap = l.take_remap();
        assert!(
            remap.contains(&("room-4100".to_string(), 0x1004)) || remap.iter().any(|(_, id)| *id == 0x1004),
            "the rooms mapped under name-derived ids come back with their real ids: {remap:?}"
        );
        assert!(l.take_remap().is_empty(), "the table is handed over once");
    }
}
