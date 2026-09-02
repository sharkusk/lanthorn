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
            self.verify(&ram, addr, movement);
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
        // value that could be an object — a nonzero address inside the scanned
        // region. The counters and flags that merely correlate get filtered by the
        // heading-less turns; this filters the rest.
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
            if v == 0 || v < self.base || v >= end {
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

    /// Once locked, keep checking: a locked word that contradicts a turn we were
    /// sure about was the wrong guess, so drop it and learn again from scratch. A
    /// wrong lock must never be sticky — it would mis-key every room after it.
    fn verify(&mut self, ram: &[u32], addr: u32, movement: Movement) {
        // An address outside the scanned window can never be verified against a
        // snapshot, so it can never be dropped by the check below either — it would
        // be a permanently unfalsifiable lock. Drop it here instead and re-learn.
        // (SQ-0658; see `word_index` for where such an address comes from.)
        let Some(idx) = self.word_index(addr) else {
            *self = RoomLock::new(self.base, ram.len());
            return;
        };
        let (Some(prev), Movement::Changed | Movement::Unchanged) = (&self.prev, movement) else {
            return;
        };
        let (Some(&a), Some(&b)) = (prev.ram.get(idx), ram.get(idx)) else { return };
        if (a != b) != (movement == Movement::Changed) {
            let words = ram.len();
            *self = RoomLock::new(self.base, words);
        }
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
    fn room_id_reads_the_locked_word() {
        let (base, words) = base_region();
        let l = RoomLock::locked_at(base, words, 0x1004);
        assert_eq!(l.room_id(&[0x1000, 0x1234, 7]), Some(0x1234));
        assert_eq!(l.room_id(&[0x1000, 0, 7]), None, "a zero global is no identity");
    }

    #[test]
    fn a_contradicted_lock_is_dropped() {
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, 0x1000);
        // Establish a previous snapshot, then contradict it: the room supposedly
        // changed while the locked word stood still.
        l.observe(vec![0x1000, 1, 7], Some("a".into()), Movement::Unchanged);
        l.observe(vec![0x1000, 2, 7], Some("b".into()), Movement::Changed);
        assert_eq!(l.locked(), None, "a lock contradicted by a sure turn must be given up, not kept");
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
