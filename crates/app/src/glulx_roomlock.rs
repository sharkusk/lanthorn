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
//!
//! ## When the story has already written the answer down (SQ-1303)
//!
//! An Inform **7** story carries its whole compiled world model — which objects
//! are rooms, and what each room is called — and `gvm::i7map` reads it off the
//! image without playing a turn. Where that reader succeeds,
//! [`RoomLock::set_rooms`] hands the result over and two things get sharper:
//!
//! * the candidate pool narrows from "holds an object" to "holds a ROOM", which
//!   is what the `location` global actually holds;
//! * and the correlation stops being the only route in. A word that has just
//!   changed to a room **whose static name is the heading the story printed this
//!   turn** is the global on the evidence of ONE move
//!   ([`RoomLock::name_witness`]) — measured on Counterfeit Monkey, the first
//!   step north out of the Back Alley rather than the tenth command.
//!
//! Both are about CHOOSING a candidate. Neither is allowed to falsify a lock:
//! see [`RoomLock::is_known_room`] for the dark room that would otherwise cost a
//! correct lock its life. And a story the reader refuses — an Inform 6 game, a
//! pre-6L02 build, one that generates its map at run time — supplies no room set
//! and behaves exactly as it did before any of this existed.

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

/// Consecutive turns of a NEW heading printed while the locked word's value sat
/// still before a lock is declared frozen and dropped (SQ-1305).
///
/// The object-value check in [`RoomLock::verify`] catches a stale sidecar
/// address whose word has stopped holding an object of the story at all, but a
/// story rebuild can just as easily leave it parked on a word that is still
/// SOME object of the story forever — the globals region is full of them
/// (`player`, `actor`, `real_location`) — and such a word never fails that
/// test. What it can't do is keep agreeing with what the game is telling the
/// player: a run of turns where a brand-new room name appears on screen while
/// this word never moves is the story narrating the player somewhere this word
/// never noticed.
///
/// One such turn is not evidence — Counterfeit Monkey's `remember` flashback
/// (SQ-1294b) prints exactly one heading for a place the player never went
/// without moving the locked word, and a correct lock must survive it
/// (`sq1294b_glulx_flashback_heading` is the guard). Three is one more than
/// the flashback can produce on its own and small enough that a genuinely
/// frozen lock recovers within a couple of turns of the player noticing the
/// map has stopped, the same shake-out-coincidences-while-reacting-quickly
/// trade [`REQUIRED_CHANGES`] makes for the pre-lock learner.
const FROZEN_LOCK_HEADINGS: u32 = 3;

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
    /// This story's ROOMS and what each one is statically called, sorted by
    /// address (SQ-1303) — `None` for every story whose compiled world model
    /// `gvm::i7map` refuses, which is where this whole file behaves exactly as
    /// it did before the reader existed.
    ///
    /// Strictly narrower than [`objects`](Self::objects) and used for two
    /// things, both of them about CHOOSING a candidate:
    ///
    /// * [`try_lock`](Self::try_lock) will not commit to a word whose current
    ///   value is merely an object when the rooms are known — the `location`
    ///   global holds a ROOM, and a word holding the player, a container or a
    ///   scenery item is not it however well it correlates;
    /// * [`name_witness`](Self::name_witness) locks on the very first move by
    ///   matching a candidate's new value's static name against the heading the
    ///   story just printed.
    ///
    /// **Never used to falsify a lock** — see [`verify`](Self::verify) for the
    /// darkness case that rule exists for.
    rooms: Option<Vec<(u32, Option<String>)>>,
    /// The locked address, once the evidence is one-sided.
    locked: Option<u32>,
    /// Consecutive turns, once locked, on which a NEW heading was printed
    /// while the locked word's value did not change — see
    /// [`FROZEN_LOCK_HEADINGS`] (SQ-1305). Reset to 0 by any turn that doesn't
    /// fit that shape, and irrelevant (left at 0) while still learning.
    frozen_headings: u32,
    /// Set for one call after a lock resolves, so the caller can re-key the rooms
    /// that were mapped under name-derived ids before the address was known.
    pending_remap: Vec<(String, u32)>,
    /// Addresses a lock was taken on and then CAUGHT OUT, sorted — never
    /// offered as a candidate again for the life of this learner (SQ-1315).
    ///
    /// Without this a rejection is a loop rather than a correction: the
    /// learner keeps the story's room set, [`name_witness`](Self::name_witness)
    /// fires again on the very next move, and its address tie-break re-elects
    /// the same losing word — Anchorhead's `2242360` for as many turns as the
    /// player cares to play. Kept across [`relearn`](Self::relearn) with
    /// `objects` and `rooms`, for the same reason both of those are: it is
    /// knowledge about the STORY, not about the guess that just failed.
    rejected: Vec<u32>,
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
            rooms: None,
            locked: None,
            frozen_headings: 0,
            pending_remap: Vec::new(),
            rejected: Vec::new(),
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

    /// Supply this story's ROOMS and their static names, in any order — see the
    /// [`rooms`](Self::rooms) field for what they are and are not used for
    /// (SQ-1303). `None` is the answer for a story with no readable compiled
    /// world model, and leaves every decision below exactly where SQ-1286 left
    /// it.
    pub fn set_rooms(&mut self, rooms: Option<Vec<(u32, Option<String>)>>) {
        self.rooms = rooms.map(|mut v| {
            v.sort_by_key(|&(a, _)| a);
            v.dedup_by_key(|&mut (a, _)| a);
            v
        });
    }

    /// True while no room set has been supplied — the caller's cue to derive one
    /// and hand it over, asked rather than pushed for the same reason
    /// [`needs_objects`](Self::needs_objects) is.
    pub fn needs_rooms(&self) -> bool {
        self.rooms.is_none()
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

    /// The story itself contradicted the locked word: give the address up, and
    /// never take it again (SQ-1315).
    ///
    /// [`verify`](Self::verify) can only ask whether the locked word still looks
    /// like a room global — which a word that has never been one passes forever.
    /// Anchorhead's is Inform 7's own *room gone to*, a going-action variable
    /// that sits four bytes below `location` in RAM and therefore wins
    /// [`name_witness`](Self::name_witness)'s address tie-break on the first
    /// move. It tracks the room perfectly for as long as every move succeeds,
    /// and then says two different false things: on a move a check rule refuses
    /// it holds the room BEHIND the locked door the player never went through,
    /// and on a move an `instead` rule reroutes it holds nothing at all. Both
    /// are ordinary Inform, and neither is visible to a test that only asks
    /// whether the value is an object of this story.
    ///
    /// What CAN see it is the story's own answer: a room the game named this
    /// turn — a printed heading, or [`crate::glulx_session::GlulxSession`]'s
    /// silent `look` — which the compiled world model resolves to exactly one
    /// address that is not the one this word holds. The caller owns that
    /// comparison (only it can read the story); this is what it does with the
    /// answer.
    ///
    /// Relearns from scratch, keeping everything [`relearn`](Self::relearn)
    /// keeps plus the growing rejection list, so the next lock is taken on a
    /// DIFFERENT word. Inform keeps several aliases of the current room side by
    /// side (`location`, `real_location`, the player's parent), so there is
    /// normally another one a move or two away; a story with no survivor simply
    /// never locks again and keeps the identity `room_by_static_name` gives it.
    pub fn reject(&mut self, addr: u32, words: usize) {
        if let Err(i) = self.rejected.binary_search(&addr) {
            self.rejected.insert(i, addr);
        }
        self.relearn(words);
    }

    /// Has `addr` already been caught out once? [`reject`](Self::reject).
    fn is_rejected(&self, addr: u32) -> bool {
        self.rejected.binary_search(&addr).is_ok()
    }

    /// Take the re-key table produced when a lock resolves: `(room name, real id)`
    /// for each room seen while learning. Empty except on the turn a lock lands,
    /// and empty entirely for a learner that started locked.
    pub fn take_remap(&mut self) -> Vec<(String, u32)> {
        std::mem::take(&mut self.pending_remap)
    }

    /// Fold this turn into the model. `ram` is the snapshot taken after the turn
    /// ran; `heading` is the room heading in force (sticky across heading-less
    /// turns); `movement` is what the caller could tell about the room
    /// changing — the STORY's own verdict once locked, and otherwise identical
    /// to `heading_movement` (see below).
    ///
    /// `heading_movement` is the verdict `finish_turn` computes purely from the
    /// printed heading (`Changed`/`Unchanged`/`Ambiguous`, the same comparison
    /// the pre-lock learner has always used) — supplied on every call, but only
    /// read once locked, where it feeds [`Self::verify`]'s frozen-lock check
    /// (SQ-1305). It has to be threaded in from the caller rather than
    /// reconstructed here from `self.prev`'s stored heading: the SQ-1294b
    /// flashback turn writes a heading (`"Galley"`) into `self.prev` without
    /// the player having moved, so a NEXT turn's heading comparing against that
    /// stored value would read `Changed` even when the game's own notion of
    /// "last named room" never left the Dormitory Room — exactly the false
    /// positive the frozen-lock check must not produce from one flashback.
    pub fn observe(
        &mut self,
        ram: Vec<u32>,
        heading: Option<String>,
        movement: Movement,
        heading_movement: Movement,
    ) {
        if let Some(addr) = self.locked {
            self.verify(&ram, addr, heading_movement);
            self.prev = Some(Observation { ram, heading });
            return;
        }
        // SQ-1303: the one witness strong enough to lock on its own, checked
        // against the turn we are about to fold in rather than after it, because
        // it needs the PREVIOUS snapshot to say which words moved.
        let witness = match (movement, heading.as_deref()) {
            (Movement::Changed, Some(h)) => self.name_witness(&ram, h),
            _ => None,
        };
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
        match witness {
            Some(addr) => self.commit(addr),
            None => self.try_lock(),
        }
    }

    /// The word this turn's HEADING identifies outright, or `None` (SQ-1303).
    ///
    /// The correlation below needs several turns because a heading alone says
    /// only "the room changed", and plenty of words change with it. A story
    /// whose compiled world model this reader can hand over says something much
    /// stronger: it names every room, so a word that has just changed to a room
    /// **whose own static name is the name the story just printed** is the
    /// `location` global on the evidence of one move. Counterfeit Monkey took
    /// ten commands to lock by correlation and locks on the first move north out
    /// of the Back Alley by this; every room mapped in between was keyed by the
    /// hash of a NAME and had to be re-keyed afterwards.
    ///
    /// **Two words normally agree, and either is correct.** Inform keeps
    /// `location` and `real_location` side by side and both hold the room the
    /// player is in, so the winner is the lower ADDRESS — a deterministic,
    /// reproducible choice, and the same tie-break [`try_lock`] already makes for
    /// the same reason. (In DARKNESS the two do differ — `location` holds
    /// `thedark` and `real_location` the room — but nothing here can tell which
    /// is which without walking into a dark room, and a lock on `location` then
    /// reports the darkness, exactly as this map has always done.)
    ///
    /// Two words holding DIFFERENT rooms of that name is a refusal, not a
    /// tie-break: that is a maze, the case the lock exists for, and a name
    /// cannot settle it.
    fn name_witness(&self, ram: &[u32], heading: &str) -> Option<u32> {
        let rooms = self.rooms.as_ref()?;
        let prev = self.prev.as_ref()?;
        let (mut value, mut best): (Option<u32>, Option<u32>) = (None, None);
        for (i, &v) in ram.iter().enumerate() {
            // A word the story has already caught out is not a candidate, and is
            // not a witness against one either — it is out of the question
            // entirely, maze test included (SQ-1315).
            let addr = self.base + (i as u32) * 4;
            if self.is_rejected(addr) {
                continue;
            }
            // Only a word that MOVED this turn is a witness: the player arrived
            // somewhere, and the global that says where went with them.
            if prev.ram.get(i) == Some(&v) {
                continue;
            }
            let Ok(k) = rooms.binary_search_by_key(&v, |&(a, _)| a) else { continue };
            let Some(name) = rooms[k].1.as_deref() else { continue };
            if !zvm::location::status_name_matches(heading, name) {
                continue;
            }
            if value.is_some_and(|seen| seen != v) {
                return None; // two rooms of that name: a maze, and a name cannot say which
            }
            value = Some(v);
            if best.is_none_or(|b| addr < b) {
                best = Some(addr);
            }
        }
        best
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
            if !self.is_room_value(v, end) || !self.is_known_room(v) {
                continue;
            }
            // Aliases of the same global are common (`real_location`, the player's
            // parent). Any of them identifies the room, so take the lowest address
            // for a deterministic, reproducible choice.
            let addr = self.base + (i as u32) * 4;
            // …unless the story has already caught that one out (SQ-1315).
            if self.is_rejected(addr) {
                continue;
            }
            if best.is_none_or(|b| addr < b) {
                best = Some(addr);
            }
        }
        let Some(addr) = best else { return };
        self.commit(addr);
    }

    /// Take `addr` as the `location` global, whichever route found it — the
    /// correlation in [`try_lock`] or [`name_witness`]'s one-move evidence.
    fn commit(&mut self, addr: u32) {
        self.locked = Some(addr);
        // Rooms already mapped under a name-derived id: hand back the real id for
        // each heading seen while learning, so the caller can re-key them instead
        // of leaving a duplicate behind.
        let idx = ((addr.saturating_sub(self.base)) / 4) as usize;
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

    /// Is `v` one of this story's ROOMS, where the compiled world model could say
    /// (SQ-1303)? `true` for every value when it could not, so a story without one
    /// keeps exactly the candidate pool [`is_room_value`](Self::is_room_value)
    /// gave it.
    ///
    /// This narrows [`try_lock`]'s candidates and NOTHING else. It deliberately
    /// has no part in [`verify`](Self::verify): in darkness Inform parks
    /// `location` on `thedark`, which is an object of the story and not a room,
    /// so a rooms-only falsification test would throw away a lock that is telling
    /// the truth the moment the player walks into an unlit room.
    fn is_known_room(&self, v: u32) -> bool {
        match &self.rooms {
            Some(rooms) => rooms.binary_search_by_key(&v, |&(a, _)| a).is_ok(),
            None => true,
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
    /// So one thing that can falsify a lock is its own VALUE: a word that no
    /// longer holds one of this story's objects is not the `location` global,
    /// whatever the screen says. A zero is not evidence either way (a game may park
    /// `location` at nothing for a turn mid-scene) and [`RoomLock::room_id`] already
    /// refuses to identify a room from one.
    ///
    /// The other (SQ-1305) is a lock that never falsifies by that test because
    /// its word genuinely holds SOME object of the story forever, just not the
    /// right one — see [`FROZEN_LOCK_HEADINGS`] for why and how many turns of
    /// disagreement that takes.
    fn verify(&mut self, ram: &[u32], addr: u32, heading_movement: Movement) {
        // An address outside the scanned window can never be verified against a
        // snapshot, so it can never be dropped by the check below either — it would
        // be a permanently unfalsifiable lock. Drop it here instead and re-learn.
        // (SQ-0658; see `word_index` for where such an address comes from.)
        let Some(idx) = self.word_index(addr) else {
            self.relearn(ram.len());
            return;
        };
        let Some(&v) = ram.get(idx) else { return };
        if v != 0 {
            // `agree` is released the moment a lock resolves, so the fallback range
            // test takes its bound from the snapshot in hand rather than from that
            // vector.
            let end = self.base + (ram.len() as u32) * 4;
            if !self.is_room_value(v, end) {
                self.relearn(ram.len());
                return;
            }
        }
        // Did THIS turn move the locked word? `self.prev` is still last turn's
        // snapshot — the caller updates it to this turn's only after this call
        // returns. No predecessor (the very first observe of a session that
        // booted already locked) is an arrival, exactly as `movement` documents
        // it — not frozen evidence either way.
        let word_changed = match &self.prev {
            Some(p) => p.ram.get(idx) != Some(&v),
            None => true,
        };
        if !word_changed && heading_movement == Movement::Changed {
            self.frozen_headings += 1;
            if self.frozen_headings >= FROZEN_LOCK_HEADINGS {
                self.relearn(ram.len());
            }
        } else {
            self.frozen_headings = 0;
        }
    }

    /// Throw the model away and learn again from scratch, keeping the object
    /// table, the room set and the rejection list — all three are properties of
    /// the STORY, not of the guess that just failed, and re-deriving the first
    /// two means another whole-image scan (see [`reject`](Self::reject) for why
    /// the third must survive too).
    fn relearn(&mut self, words: usize) {
        let objects = self.objects.take();
        let rooms = self.rooms.take();
        let rejected = std::mem::take(&mut self.rejected);
        *self = RoomLock::new(self.base, words);
        self.objects = objects;
        self.rooms = rooms;
        self.rejected = rejected;
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
            // Pre-lock, `heading_movement` is always the same verdict as `mv` —
            // `GlulxSession` only ever computes them differently once locked
            // (see `observe`'s docs) — so every `drive`-driven test (all of
            // them pre-lock) can pass the one value twice.
            l.observe(ram, heading, mv, mv);
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
        l.observe(ram, Some("Hall".to_string()), Movement::Changed, Movement::Changed);
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
        l.observe(vec![0x1000, 1, 7], Some("a".into()), Movement::Unchanged, Movement::Unchanged);
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
        l.observe(vec![0x1000, 1, 7], Some("a".into()), Movement::Unchanged, Movement::Unchanged);
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
        l.observe(vec![0, 1, 7], Some("a".into()), Movement::Unchanged, Movement::Unchanged);
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
        //
        // This is also exactly ONE of the [`FROZEN_LOCK_HEADINGS`] (SQ-1305)
        // shape — word still, new heading — and one is under the threshold of
        // three: `frozen_lock_survives_two_fresh_headings_but_drops_on_three`
        // below is where the boundary itself is pinned.
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, 0x1000);
        l.set_objects(Some(vec![0x2000, 0x2004]));
        l.observe(vec![0x2000, 1, 7], Some("Dormitory Room".into()), Movement::Unchanged, Movement::Changed);
        l.observe(vec![0x2000, 2, 7], Some("Galley".into()), Movement::Changed, Movement::Changed);
        assert_eq!(l.locked(), Some(0x1000), "a flashback heading is not evidence about the lock");
        l.observe(vec![0x2004, 3, 7], None, Movement::Unchanged, Movement::Unchanged);
        assert_eq!(l.locked(), Some(0x1000), "nor is a move the story narrated without a heading");
    }

    #[test]
    fn frozen_lock_survives_two_fresh_headings_but_drops_on_three() {
        // SQ-1305: a stale sidecar can point at a word that holds SOME object
        // of the story forever — never zero, never outside the object table —
        // so `verify`'s value check alone never falsifies it. Three straight
        // turns of a brand-new heading with the locked word not moving an inch
        // is the story narrating the player somewhere this word never noticed.
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, 0x1000);
        l.set_objects(Some(vec![0x2000, 0x2004]));

        // Turn 1: the arrival. No predecessor, so this is never frozen evidence.
        l.observe(vec![0x2000, 1, 7], Some("Room A".into()), Movement::Unchanged, Movement::Changed);
        assert_eq!(l.locked(), Some(0x1000));

        // Turns 2 and 3: a fresh heading each time, the locked word never moving.
        // Two in a row is still under the threshold — the SQ-1294b flashback is
        // exactly one of these and must survive.
        l.observe(vec![0x2000, 2, 7], Some("Room B".into()), Movement::Unchanged, Movement::Changed);
        assert_eq!(l.locked(), Some(0x1000), "one fresh heading over a still word: the flashback shape");
        l.observe(vec![0x2000, 3, 7], Some("Room C".into()), Movement::Unchanged, Movement::Changed);
        assert_eq!(l.locked(), Some(0x1000), "two in a row still isn't three");

        // The third in a row: the lock is frozen and must be dropped.
        l.observe(vec![0x2000, 4, 7], Some("Room D".into()), Movement::Unchanged, Movement::Changed);
        assert_eq!(l.locked(), None, "three straight fresh headings over a motionless word is a frozen lock");
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
        l.observe(vec![0x2000, 1, 7], Some("Maze".into()), Movement::Changed, Movement::Changed);
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

    // ── SQ-1303: what the story's own compiled world model buys the learner ──

    /// A synthetic room set in the shape [`RoomLock::set_rooms`] takes: three
    /// rooms well outside the scan window (where a real Inform story keeps its
    /// object table), each with the name the story would print for it.
    fn three_rooms(base: u32) -> (u32, Vec<(u32, Option<String>)>) {
        let far = base + 0x4000;
        (
            far,
            vec![
                (far, Some("Back Alley".to_string())),
                (far + 0x10, Some("Sigil Street".to_string())),
                (far + 0x20, Some("Ampersand Bend".to_string())),
            ],
        )
    }

    #[test]
    fn a_heading_that_names_the_room_a_word_moved_to_locks_on_the_first_move() {
        // The whole point of reading the compiled world model: Counterfeit Monkey
        // took ten commands to lock by correlation alone, and every room mapped in
        // between was keyed by the hash of a NAME. One move is enough when the
        // story has already told us what its rooms are called.
        let (base, _) = base_region();
        let (far, rooms) = three_rooms(base);
        let mut l = RoomLock::new(base, 3);
        l.set_objects(Some(vec![far, far + 0x10, far + 0x20]));
        l.set_rooms(Some(rooms));

        l.observe(vec![far, 1, 7], Some("Back Alley".into()), Movement::Unchanged, Movement::Unchanged);
        assert_eq!(l.locked(), None, "nothing has moved yet");

        // One move north. Word 0 changes to the Sigil Street room; word 1 is a
        // turn counter that changes too and holds nothing that is a room.
        l.observe(vec![far + 0x10, 2, 7], Some("Sigil Street".into()), Movement::Changed, Movement::Changed);
        assert_eq!(
            l.locked(),
            Some(base),
            "the word that moved to a room whose static name is the printed heading IS the global"
        );
    }

    #[test]
    fn two_words_holding_the_same_room_take_the_lower_address() {
        // Inform keeps `location` and `real_location` side by side and both hold
        // the room. Either identifies it, so the tie-break is deterministic
        // rather than clever — see `name_witness`'s docs for the darkness caveat
        // that is the one place they differ.
        let (base, _) = base_region();
        let (far, rooms) = three_rooms(base);
        let mut l = RoomLock::new(base, 3);
        l.set_objects(Some(vec![far, far + 0x10, far + 0x20]));
        l.set_rooms(Some(rooms));
        l.observe(vec![far, far, 7], Some("Back Alley".into()), Movement::Unchanged, Movement::Unchanged);
        l.observe(vec![far + 0x10, far + 0x10, 7], Some("Sigil Street".into()), Movement::Changed, Movement::Changed);
        assert_eq!(l.locked(), Some(base), "the lower of the two agreeing words");
    }

    #[test]
    fn two_rooms_of_one_name_refuse_the_witness_and_leave_the_learner_learning() {
        // A maze is exactly the case the lock exists for, and exactly the case a
        // NAME cannot settle. Two words moved to two DIFFERENT rooms that are both
        // called "Maze": the fast path must decline rather than pick one.
        let (base, _) = base_region();
        let far = base + 0x4000;
        let mut l = RoomLock::new(base, 3);
        l.set_objects(Some(vec![far, far + 0x10, far + 0x20]));
        l.set_rooms(Some(vec![
            (far, Some("Hall".to_string())),
            (far + 0x10, Some("Maze".to_string())),
            (far + 0x20, Some("Maze".to_string())),
        ]));
        l.observe(vec![far, far, 7], Some("Hall".into()), Movement::Unchanged, Movement::Unchanged);
        l.observe(vec![far + 0x10, far + 0x20, 7], Some("Maze".into()), Movement::Changed, Movement::Changed);
        assert_eq!(
            l.locked(),
            None,
            "two rooms of that name: the heading cannot say which word is the global"
        );
    }

    #[test]
    fn the_room_set_rules_out_a_word_that_holds_an_object_which_is_not_a_room() {
        // The narrowing half. Word 0 tracks the room perfectly and holds an
        // OBJECT of the story — the player's own avatar, say — which the SQ-1286
        // filter accepts and this one does not. Word 1 is the real global.
        let (base, _) = base_region();
        let far = base + 0x4000;
        let thing = far + 0x100; // three non-room objects: a vehicle, a held item…
        let mut l = RoomLock::new(base, 3);
        l.set_objects(Some(vec![
            far,
            far + 0x10,
            far + 0x20,
            thing,
            thing + 0x10,
            thing + 0x20,
        ]));
        l.set_rooms(Some(vec![(far, None), (far + 0x10, None), (far + 0x20, None)]));
        // Word 0 changes EXACTLY when word 1 does, so the correlation cannot
        // separate them and the lower address would win. No heading names a room
        // here, so only the correlation and the room set are in play.
        for (mv, step) in [
            (Movement::Unchanged, 0x00),
            (Movement::Changed, 0x10),
            (Movement::Unchanged, 0x10),
            (Movement::Changed, 0x20),
            (Movement::Changed, 0x00),
        ] {
            l.observe(vec![thing + step, far + step, 7], None, mv, mv);
        }
        assert_eq!(
            l.locked(),
            Some(base + 4),
            "the room set skips the word holding a non-room object and takes the one holding a room"
        );
    }

    #[test]
    fn a_lock_survives_the_player_walking_into_the_dark() {
        // The rule the room set is NOT allowed to break. In darkness Inform parks
        // `location` on `thedark`, which is an object of the story and not one of
        // its rooms; a rooms-only falsification test would drop a lock that is
        // telling the truth, on the one turn the player most needs the map to hold
        // still. `verify` therefore stays on OBJECTS.
        let (base, _) = base_region();
        let far = base + 0x4000;
        let thedark = far + 0x200;
        let mut l = RoomLock::locked_at(base, 3, base);
        l.set_objects(Some(vec![far, far + 0x10, thedark]));
        l.set_rooms(Some(vec![(far, None), (far + 0x10, None)]));
        l.observe(vec![thedark, 1, 7], Some("Darkness".into()), Movement::Changed, Movement::Changed);
        assert_eq!(
            l.locked(),
            Some(base),
            "`thedark` is an object of this story, so it is no evidence against the lock"
        );
    }

    #[test]
    fn a_story_with_no_readable_world_model_learns_exactly_as_it_did_before() {
        // Kerkerkruip generates its dungeon at run time and `gvm::i7map` refuses
        // it; an Inform 6 story has no `Map_Storage` at all. Both hand over `None`,
        // and every decision here must be the SQ-1286 one, unchanged.
        let (base, words) = base_region();
        let mut l = RoomLock::new(base, words);
        l.set_objects(Some(vec![0x1000, 0x1004, 0x1008]));
        l.set_rooms(None);
        assert!(l.needs_rooms(), "`None` is not an answer, so the caller may try again");
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
        assert_eq!(l.locked(), Some(0x1000), "unchanged from before the world model existed");
    }

    #[test]
    fn a_relearn_keeps_the_room_set_too() {
        // Same argument as the object table: the rooms are the STORY's, not the
        // failed guess's, and re-deriving them is another whole-image scan.
        let (base, words) = base_region();
        let mut l = RoomLock::locked_at(base, words, 0x1000);
        l.set_objects(Some(vec![0x2000, 0x2004]));
        l.set_rooms(Some(vec![(0x2000, None)]));
        l.observe(vec![0x1000, 1, 7], Some("a".into()), Movement::Unchanged, Movement::Unchanged);
        assert_eq!(l.locked(), None, "the locked word holds nothing that is a room of this story");
        assert!(!l.needs_rooms(), "…but the room set survived the re-learn");
        assert!(!l.needs_objects(), "…and so did the object table");
    }

    // ── SQ-1315: a word the STORY caught out is never offered again ───────────

    #[test]
    fn a_rejected_word_is_never_locked_on_again_and_the_next_alias_wins() {
        // Anchorhead in miniature: two words hold the room and the LOWER one is a
        // going-action variable that tracks it only while every move succeeds.
        // Rejecting it must not simply relearn — `name_witness` would re-elect the
        // same address on the very next move, forever.
        let (base, _) = base_region();
        let (far, rooms) = three_rooms(base);
        let mut l = RoomLock::new(base, 3);
        l.set_objects(Some(vec![far, far + 0x10, far + 0x20]));
        l.set_rooms(Some(rooms));

        l.observe(vec![far, far, 7], Some("Back Alley".into()), Movement::Unchanged, Movement::Unchanged);
        l.observe(
            vec![far + 0x10, far + 0x10, 7],
            Some("Sigil Street".into()),
            Movement::Changed,
            Movement::Changed,
        );
        assert_eq!(l.locked(), Some(base), "the lower of the two agreeing words, as ever");

        // The caller reads the story and finds the two disagree.
        l.reject(base, 3);
        assert_eq!(l.locked(), None, "the address is given up on the spot");
        assert!(!l.needs_rooms(), "…and the room set survives, as for any other re-learn");
        assert!(!l.needs_objects(), "…and so does the object table");

        // Learn again from the same shape of evidence. The lower word is out of the
        // running now, so the alias beside it is what locks.
        l.observe(vec![far, far, 7], Some("Back Alley".into()), Movement::Unchanged, Movement::Unchanged);
        l.observe(
            vec![far + 0x10, far + 0x10, 7],
            Some("Sigil Street".into()),
            Movement::Changed,
            Movement::Changed,
        );
        assert_eq!(
            l.locked(),
            Some(base + 4),
            "the next word up — the rejected one may not be re-elected however well it witnesses"
        );
    }

    #[test]
    fn a_rejected_word_is_out_of_the_correlation_lock_too() {
        // The same exclusion on the slow route in: `try_lock`'s correlation must
        // not hand back an address `name_witness` has been told to refuse, or the
        // rejection would last exactly as long as the story kept printing headings.
        let (base, words) = base_region();
        let mut l = RoomLock::new(base, words);
        l.reject(base, words);
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
            "word 0 is the only one that correlates, and it is out of the running"
        );
    }

    #[test]
    fn a_rejected_word_survives_a_later_relearn() {
        // A rejection is a fact about the STORY, so it has to outlive the ordinary
        // re-learn that `verify` triggers — otherwise one dark room or one stale
        // sidecar hands the caught-out word straight back.
        let (base, words) = base_region();
        let mut l = RoomLock::new(base, words);
        l.reject(base, words);
        l.set_objects(Some(vec![0x2000]));
        // A `locked_at` whose word holds no object of the story: `verify` relearns.
        let mut l2 = RoomLock::locked_at(base, words, base + 8);
        l2.rejected = l.rejected.clone();
        l2.set_objects(Some(vec![0x2000]));
        l2.observe(vec![1, 1, 1], Some("a".into()), Movement::Unchanged, Movement::Unchanged);
        assert_eq!(l2.locked(), None, "the lock was dropped by the value check");
        assert!(l2.is_rejected(base), "…and the rejection list came through the re-learn intact");
    }
}
