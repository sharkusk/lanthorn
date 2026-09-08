//! [`Vm`], the mutable play state that turns a static [`crate::Database`]
//! into a running game: item locations, the player's room, flags, counters,
//! lamp fuel, and the PRNG occurrence rolls draw from.
//!
//! A host drives a session through [`Vm::step`] / [`Vm::supply_line`] and
//! [`StepResult`] (see the crate-level docs for the full protocol), and
//! saves it with [`Vm::snapshot`] / [`Vm::restore`] — the latter reporting a
//! shape mismatch as a [`RestoreError`] rather than corrupting state.
//!
//! Verb and command dispatch follow ScottFree's own C source (`main.c`'s
//! opcode 0-51 conditions and 52+ commands) wherever the database format
//! leaves room for disagreement between implementations; see individual
//! method docs for the sites where that mattered.

use crate::*;
use crate::database::{CARRIED, DARK_FLAG, LAMP_EMPTY_FLAG, LIGHT_SOURCE};
use std::collections::HashSet;

/// Condition codes 0..=19 implemented by `Vm::eval_condition` below (the
/// loader unpacks the raw `20*value + code` encoding into this range, see
/// `Database::parse`). Test-only: exists purely so `decompile.rs`'s mnemonic
/// coverage test can assert its table has exactly these keys, so the two
/// can't silently drift apart. Kept in sync by hand next to the match arms.
#[cfg(test)]
pub(crate) const CONDITION_CODES: [u8; 20] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
];

/// Command opcodes 52..=89, individually implemented by `Vm::run_commands`
/// below (0 is a no-op slot; 1..=51 and 102.. print a message; 90..=101 are
/// unimplemented/no-op — see the `_ => {}` arm there). Test-only, for the
/// same reason as `CONDITION_CODES` above. Kept in sync by hand.
#[cfg(test)]
pub(crate) const FIXED_COMMAND_OPCODES: [u16; 38] = [
    52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74,
    75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StepResult {
    Continue,
    NeedLine,
    Quit,
}

/// Why [`Vm::restore`] rejected a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RestoreError {
    /// The byte buffer ended before every field of the snapshot was read.
    Truncated,
    /// A room or counter index encoded in the snapshot does not fit this
    /// game's room table.
    OutOfRange,
    /// The item count encoded in the snapshot does not match this game's own
    /// item table — the usual sign of a version/format mismatch (a snapshot
    /// taken against a different compiled `.dat`).
    ItemCountMismatch,
    /// Bytes remained in the buffer after every field was read.
    TrailingData,
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreError::Truncated => write!(f, "save data ended before the snapshot was fully read"),
            RestoreError::OutOfRange => write!(f, "save data names a room or counter index outside this game"),
            RestoreError::ItemCountMismatch => write!(f, "save data's item count does not match this game"),
            RestoreError::TrailingData => write!(f, "save data has extra bytes after the snapshot"),
        }
    }
}

impl std::error::Error for RestoreError {}

pub struct Vm {
    pub(crate) db: Database,
    pub(crate) item_loc: Vec<i32>, // per item: current location (room index; -1/255 = carried; 0 = nowhere)
    pub(crate) player: usize,      // current room
    pub(crate) flags: [bool; 32],
    pub(crate) current_counter: i32, // the live counter all counter ops/conditions act on
    pub(crate) counters: [i32; 16],  // backup registers, swapped via op 81
    pub(crate) saved_room: usize,    // op-80 register (swapped with `player`)
    pub(crate) saved_rooms: [usize; 16], // op-87 registers
    pub(crate) lamp: i32, // remaining light; -1 = infinite
    pub(crate) out: String, // pending transcript
    pub(crate) quit: bool,
    pub(crate) needs_line: bool,
    pub(crate) last_noun: String, // for print-noun opcodes
    pub(crate) rng_state: u32, // xorshift PRNG state
    pub(crate) pending_line: Option<String>, // buffered command
    // Transient per-turn signal: the action interpreter ran opcode 71 (SAVE
    // GAME) this turn. The host drains it after `step()` to trigger a save. Not
    // part of game state, so it is excluded from snapshot/restore.
    pub(crate) save_requested: bool,
    // The picture opcode 89 (draw picture) requested this turn, if any; overrides
    // the default room picture until the next command. Cosmetic, host-read via
    // `current_picture`; excluded from snapshot/restore.
    pub(crate) pending_picture: Option<u16>,
    // --- SQ-0464 debug-inspector support: fired-action tracing. Off by
    // default; every field below is a no-op / stays empty unless a host opts
    // in via `set_trace_fired`, so there is zero cost in normal play beyond a
    // single `bool` check per action. Not game state — excluded from
    // snapshot/restore, like `save_requested` and `pending_picture` above.
    pub(crate) trace_fired: bool,
    /// Action indices whose commands executed THIS turn (cleared at the start
    /// of every `run_turn`; the opening occurrence pass in `new` populates it
    /// for the pre-first-prompt "turn").
    pub(crate) fired_actions: Vec<usize>,
    /// Action indices whose commands have EVER executed this session. Never
    /// cleared; seed via `seed_ever_fired` to restore persisted coverage.
    pub(crate) ever_fired: HashSet<usize>,
    /// This turn's (action index, first failing condition slot) pairs, for
    /// player-command actions whose verb/noun matched but a condition blocked
    /// them. Cleared every turn; only populated while `trace_fired` is on.
    pub(crate) last_blocked: Vec<(usize, usize)>,
}

impl Vm {
    /// The PRNG seed a `Vm` nobody seeds starts from: fixed, so every unit test
    /// draws the same sequence. Nonzero — xorshift32 is absorbing at zero.
    pub const DEFAULT_RNG_SEED: u32 = 0x1234_5678;

    /// Initialize: item_loc from each item's start_loc, player=start_room, lamp=light_time,
    /// flags/counters cleared, current_counter=0, saved_room/saved_rooms=0, needs_line=true.
    /// Describes the starting room into `out` so the host can read the intro via
    /// `take_output()` before driving input.
    pub fn new(db: Database) -> Vm {
        Vm::new_with_trace(db, false)
    }

    /// Like [`Vm::new`], but with fired-action tracing enabled from the very
    /// first instruction — so the opening occurrence pass (run inside this
    /// constructor, before any host code can call [`Vm::set_trace_fired`]) is
    /// captured too. A host's debug-boot path (lanthorn's `--debug` flag, for
    /// instance) uses this so a Scott story's start-of-game auto-events show up
    /// as fired from the first frame.
    pub fn new_with_trace(db: Database, trace_fired: bool) -> Vm {
        Vm::new_seeded(db, trace_fired, Vm::DEFAULT_RNG_SEED)
    }

    /// [`Vm::new_with_trace`] with the PRNG seed supplied (SQ-0811).
    ///
    /// A seed belongs in the CONSTRUCTOR, not in a `seed_rng` call after it: the
    /// opening occurrence pass runs below, inside this function, and occurrences
    /// roll percentage chances — so a game seeded afterwards has already had its
    /// first random events decided by the default seed. The host supplies the
    /// seed here — lanthorn passes its `random_seed` config key, or an entropy
    /// draw when that key is unset.
    pub fn new_seeded(db: Database, trace_fired: bool, seed: u32) -> Vm {
        let item_loc = db.items.iter().map(|i| i.start_loc).collect();
        let player = db.start_room;
        let lamp = db.light_time;
        let mut vm = Vm {
            db,
            item_loc,
            player,
            flags: [false; 32],
            current_counter: 0,
            counters: [0; 16],
            saved_room: 0,
            saved_rooms: [0; 16],
            lamp,
            out: String::new(),
            quit: false,
            needs_line: true,
            last_noun: String::new(),
            rng_state: seed.max(1),
            pending_line: None,
            save_requested: false,
            pending_picture: None,
            trace_fired,
            fired_actions: Vec::new(),
            ever_fired: HashSet::new(),
            last_blocked: Vec::new(),
        };
        // Run the opening occurrence pass before the first prompt, so auto-events
        // that fire at game start (e.g. "A Voice BOOOMS out:") are shown on load —
        // matching ScottFree/Gargoyle, where occurrences run at the top of the turn
        // loop rather than only when a command is entered.
        vm.run_occurrences();
        vm.needs_line = true;
        vm
    }

    // --- accessors used by later tasks + the host adapter ---
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.out)
    }
    pub fn current_room(&self) -> usize {
        self.player
    }
    pub fn room_name(&self, r: usize) -> &str {
        self.db.rooms.get(r).map(|room| room.desc.as_str()).unwrap_or("")
    }
    pub fn has_quit(&self) -> bool {
        self.quit
    }
    /// Take (read and clear) the "the game ran SAVE GAME (opcode 71) this turn"
    /// signal. The host calls this after `step()`; when true it should perform a
    /// save. Transient — never persisted in a snapshot.
    pub fn take_save_request(&mut self) -> bool {
        std::mem::take(&mut self.save_requested)
    }
    /// The picture the host should display: an opcode-89 override if one was set
    /// this turn, else the current room's own picture (by convention, picture
    /// number == room number). The host maps this to a blorb `Pict` resource and
    /// shows nothing when none exists.
    pub fn current_picture(&self) -> Option<u16> {
        self.pending_picture.or(Some(self.player as u16))
    }
    /// Current location of item `idx` (-1/255 = carried, 0 = nowhere, else room index).
    pub fn item_loc(&self, idx: usize) -> i32 {
        self.item_loc.get(idx).copied().unwrap_or(0)
    }
    /// Whether flag `idx` is set.
    pub fn flag(&self, idx: usize) -> bool {
        self.flags.get(idx).copied().unwrap_or(false)
    }
    /// Value of the live current counter.
    pub fn counter(&self) -> i32 {
        self.current_counter
    }
    /// The parsed game database (rooms, items, actions, vocab, messages) — the
    /// static tables the debug inspector decompiles and lists on demand.
    pub fn database(&self) -> &Database {
        &self.db
    }
    /// Remaining lamp fuel (light turns); `-1` means an infinite light source.
    pub fn lamp(&self) -> i32 {
        self.lamp
    }
    /// The op-80 saved-room register (the room swapped out by SWAP_ROOM), or 0
    /// if nothing has been stashed there yet.
    pub fn saved_room(&self) -> usize {
        self.saved_room
    }

    /// Enable/disable fired-action tracing (off by default). While off, the
    /// action interpreter pays only a single `bool` check per action — no
    /// vec/set writes, no allocation. The app's debug inspector turns this on.
    pub fn set_trace_fired(&mut self, on: bool) {
        self.trace_fired = on;
    }
    /// Whether fired-action tracing is currently on.
    pub fn trace_fired(&self) -> bool {
        self.trace_fired
    }
    /// Action indices whose commands executed this turn. Empty unless
    /// `trace_fired` is on.
    pub fn fired_actions(&self) -> &[usize] {
        &self.fired_actions
    }
    /// Action indices whose commands have ever executed this session. Empty
    /// unless `trace_fired` is on (or has been at some point this session).
    pub fn ever_fired(&self) -> &HashSet<usize> {
        &self.ever_fired
    }
    /// Merge a previously-persisted coverage set (stored as `u32`s, the
    /// SQ-0449 pattern used elsewhere) into the cumulative ever-fired set.
    pub fn seed_ever_fired(&mut self, seed: &HashSet<u32>) {
        self.ever_fired.extend(seed.iter().map(|&v| v as usize));
    }
    /// This turn's (action index, first failing condition slot) pairs, for
    /// player-command actions whose verb/noun matched but were blocked by a
    /// condition — answers "why didn't my command work". Empty unless
    /// `trace_fired` is on.
    pub fn last_blocked(&self) -> &[(usize, usize)] {
        &self.last_blocked
    }

    /// Serialize mutable game state: item locations, player room, flags,
    /// the live current counter, backup counters, the op-80 saved room, the
    /// op-87 saved-room registers, and lamp fuel. Manual little-endian byte
    /// encoding, zero-dep.
    pub fn snapshot(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(self.item_loc.len() as u32).to_le_bytes());
        for v in &self.item_loc {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.extend_from_slice(&(self.player as u32).to_le_bytes());
        for &f in &self.flags {
            buf.push(f as u8);
        }
        buf.extend_from_slice(&self.current_counter.to_le_bytes());
        for c in &self.counters {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        buf.extend_from_slice(&(self.saved_room as u32).to_le_bytes());
        for &r in &self.saved_rooms {
            buf.extend_from_slice(&(r as u32).to_le_bytes());
        }
        buf.extend_from_slice(&self.lamp.to_le_bytes());
        buf
    }

    /// Restore state from `snapshot` bytes. Rejects short/malformed input, a
    /// mismatched item count, or an out-of-range room/counter index.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), RestoreError> {
        fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, RestoreError> {
            let end = pos.checked_add(4).ok_or(RestoreError::Truncated)?;
            let slice = bytes.get(*pos..end).ok_or(RestoreError::Truncated)?;
            *pos = end;
            Ok(u32::from_le_bytes(slice.try_into().unwrap()))
        }
        fn read_i32(bytes: &[u8], pos: &mut usize) -> Result<i32, RestoreError> {
            read_u32(bytes, pos).map(|v| v as i32)
        }

        let mut pos = 0usize;
        let item_count = read_u32(bytes, &mut pos)? as usize;
        if item_count != self.item_loc.len() {
            return Err(RestoreError::ItemCountMismatch);
        }
        let mut item_loc = Vec::with_capacity(item_count);
        for _ in 0..item_count {
            item_loc.push(read_i32(bytes, &mut pos)?);
        }
        let player = read_u32(bytes, &mut pos)? as usize;
        if player >= self.db.rooms.len() {
            return Err(RestoreError::OutOfRange);
        }
        let flags_slice = bytes.get(pos..pos + 32).ok_or(RestoreError::Truncated)?;
        let mut flags = [false; 32];
        for (i, &b) in flags_slice.iter().enumerate() {
            flags[i] = b != 0;
        }
        pos += 32;
        let current_counter = read_i32(bytes, &mut pos)?;
        let mut counters = [0i32; 16];
        for c in counters.iter_mut() {
            *c = read_i32(bytes, &mut pos)?;
        }
        let saved_room = read_u32(bytes, &mut pos)? as usize;
        if saved_room >= self.db.rooms.len() {
            return Err(RestoreError::OutOfRange);
        }
        let mut saved_rooms = [0usize; 16];
        for r in saved_rooms.iter_mut() {
            let v = read_u32(bytes, &mut pos)? as usize;
            if v >= self.db.rooms.len() {
                return Err(RestoreError::OutOfRange);
            }
            *r = v;
        }
        let lamp = read_i32(bytes, &mut pos)?;

        if pos != bytes.len() {
            return Err(RestoreError::TrailingData);
        }

        self.item_loc = item_loc;
        self.player = player;
        self.flags = flags;
        self.current_counter = current_counter;
        self.counters = counters;
        self.saved_room = saved_room;
        self.saved_rooms = saved_rooms;
        self.lamp = lamp;
        Ok(())
    }

    /// Run one turn if a line is buffered, then request the next.
    pub fn step(&mut self) -> StepResult {
        if self.quit {
            return StepResult::Quit;
        }
        if let Some(cmd) = self.pending_line.take() {
            // A new command starts a fresh turn: drop any opcode-89 override from
            // the previous turn so the picture reverts to the room's own.
            self.pending_picture = None;
            self.run_turn(&cmd);
            if self.quit {
                return StepResult::Quit;
            }
        }
        StepResult::NeedLine
    }

    /// Buffer the command; clear needs_line.
    pub fn supply_line(&mut self, line: &str) {
        self.pending_line = Some(line.to_string());
        self.needs_line = false;
    }

    /// Seed the deterministic PRNG (used by occurrence percentage rolls).
    /// To seed a NEW game use [`Vm::new_seeded`] — the opening occurrence pass
    /// runs inside the constructor, before this could be called.
    pub fn seed_rng(&mut self, s: u32) {
        self.rng_state = s.max(1);
    }

    /// The current PRNG state — the seed the next roll advances from.
    /// Diagnostic only (the startup seed report); the game never sees it.
    pub fn rng_seed(&self) -> u32 {
        self.rng_state
    }

    /// xorshift32 PRNG, deterministic given `rng_state`.
    fn next_rand(&mut self) -> u32 {
        let mut x = self.rng_state.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        x
    }

    fn roll_100(&mut self) -> u32 {
        self.next_rand() % 100
    }

    pub(crate) fn eval_condition(&self, c: &Condition) -> bool {
        let value = c.value as usize;
        match c.code {
            0 => true,
            1 => self.item_carried(value),
            2 => self.item_in_room(value),
            3 => self.item_present(value),
            4 => self.player == value,
            5 => !self.item_in_room(value),
            6 => !self.item_carried(value),
            7 => self.player != value,
            8 => self.flag_get(value),
            9 => !self.flag_get(value),
            10 => (0..self.item_loc.len()).any(|i| self.item_carried(i)),
            11 => !(0..self.item_loc.len()).any(|i| self.item_carried(i)),
            12 => !self.item_present(value),
            13 => self.item_in_play(value),
            14 => !self.item_in_play(value),
            15 => self.current_counter <= c.value as i32,
            16 => self.current_counter > c.value as i32,
            19 => self.current_counter == c.value as i32,
            17 => self.item_at_start(value),
            18 => !self.item_at_start(value),
            _ => false,
        }
    }

    fn item_loc_of(&self, idx: usize) -> Option<i32> {
        self.item_loc.get(idx).copied()
    }

    fn item_carried(&self, idx: usize) -> bool {
        matches!(self.item_loc_of(idx), Some(-1) | Some(255))
    }

    fn item_in_room(&self, idx: usize) -> bool {
        self.item_loc_of(idx) == Some(self.player as i32)
    }

    fn item_present(&self, idx: usize) -> bool {
        self.item_carried(idx) || self.item_in_room(idx)
    }

    fn item_in_play(&self, idx: usize) -> bool {
        matches!(self.item_loc_of(idx), Some(loc) if loc != 0)
    }

    fn item_at_start(&self, idx: usize) -> bool {
        match (self.item_loc_of(idx), self.db.items.get(idx)) {
            (Some(loc), Some(item)) => loc == item.start_loc,
            _ => false,
        }
    }

    fn flag_get(&self, idx: usize) -> bool {
        self.flags.get(idx).copied().unwrap_or(false)
    }

    fn flag_set(&mut self, idx: usize, v: bool) {
        if let Some(f) = self.flags.get_mut(idx) {
            *f = v;
        }
    }

    fn set_item_loc(&mut self, idx: usize, loc: i32) {
        if let Some(l) = self.item_loc.get_mut(idx) {
            *l = loc;
        }
    }

    fn carried_count(&self) -> i32 {
        (0..self.item_loc.len())
            .filter(|&i| self.item_carried(i))
            .count() as i32
    }

    /// Execute the 4 command opcodes of one action, pulling operands in order from
    /// `params`. Returns true if a `continue` (73) was executed.
    fn run_commands(&mut self, cmds: &[u16; 4], params: &[u16]) -> bool {
        let mut p = params.iter().copied();
        let mut did_continue = false;
        for &n in cmds {
            match n {
                0 => {}
                1..=51 => self.print_message(n as usize),
                // Action codes >= 102 print message (code - 50), with no upper
                // bound — a game with 100+ messages emits them via codes >= 152.
                // `print_message` bounds-checks, so an out-of-range index is a
                // safe no-op.
                102..=u16::MAX => self.print_message(n as usize - 50),
                52 => {
                    let item = p.next().unwrap_or(0) as usize;
                    if self.carried_count() < self.db.max_carry {
                        self.set_item_loc(item, CARRIED);
                    } else {
                        self.out.push_str("You are carrying too much.\n");
                    }
                }
                53 => {
                    let item = p.next().unwrap_or(0) as usize;
                    self.set_item_loc(item, self.player as i32);
                }
                54 => {
                    let room = p.next().unwrap_or(0) as usize;
                    if room < self.db.rooms.len() {
                        self.player = room;
                    }
                }
                55 => {
                    let item = p.next().unwrap_or(0) as usize;
                    self.set_item_loc(item, 0);
                }
                56 => self.flag_set(DARK_FLAG, true),
                57 => self.flag_set(DARK_FLAG, false),
                58 => {
                    let flag = p.next().unwrap_or(0) as usize;
                    self.flag_set(flag, true);
                }
                59 => {
                    let item = p.next().unwrap_or(0) as usize;
                    self.set_item_loc(item, 0);
                }
                60 => {
                    let flag = p.next().unwrap_or(0) as usize;
                    self.flag_set(flag, false);
                }
                61 => {
                    self.out.push_str("You have died.\n");
                    self.flag_set(DARK_FLAG, false);
                    if let Some(last) = self.db.rooms.len().checked_sub(1) {
                        self.player = last;
                    }
                }
                62 => {
                    let item = p.next().unwrap_or(0) as usize;
                    let room = p.next().unwrap_or(0) as usize;
                    if room < self.db.rooms.len() {
                        self.set_item_loc(item, room as i32);
                    }
                }
                63 => self.quit = true,
                64 => { /* LOOK: the host redraws the room panel every frame */ }
                65 => self.print_score(),
                66 => self.print_inventory(),
                67 => self.flag_set(0, true),
                68 => self.flag_set(0, false),
                69 => {
                    // ScottFree case 69 (refill lamp): reset the fuel, move the
                    // light source (item 9) into the pack, clear the empty flag.
                    self.lamp = self.db.light_time;
                    self.set_item_loc(LIGHT_SOURCE, CARRIED);
                    self.flag_set(LAMP_EMPTY_FLAG, false);
                }
                70 => self.out.clear(),
                71 => self.save_requested = true, // SAVE GAME: host performs the save after the turn
                72 => {
                    let a = p.next().unwrap_or(0) as usize;
                    let b = p.next().unwrap_or(0) as usize;
                    if let (Some(&la), Some(&lb)) = (self.item_loc.get(a), self.item_loc.get(b)) {
                        self.set_item_loc(a, lb);
                        self.set_item_loc(b, la);
                    }
                }
                73 => did_continue = true,
                74 => {
                    let item = p.next().unwrap_or(0) as usize;
                    self.set_item_loc(item, CARRIED);
                }
                75 => {
                    let a = p.next().unwrap_or(0) as usize;
                    let b = p.next().unwrap_or(0) as usize;
                    if let Some(&lb) = self.item_loc.get(b) {
                        self.set_item_loc(a, lb);
                    }
                }
                76 => { /* LOOK (redraw): the host room panel is always current */ }
                77 => {
                    self.current_counter = (self.current_counter - 1).max(-1);
                }
                78 => {
                    let v = self.current_counter;
                    self.out.push_str(&v.to_string());
                    self.out.push('\n');
                }
                79 => {
                    self.current_counter = p.next().unwrap_or(0) as i32;
                }
                80 => {
                    std::mem::swap(&mut self.player, &mut self.saved_room);
                }
                81 => {
                    // Swap the live current counter with backup register `param`.
                    let idx = (p.next().unwrap_or(0) as usize).min(self.counters.len() - 1);
                    std::mem::swap(&mut self.current_counter, &mut self.counters[idx]);
                }
                82 => {
                    let v = p.next().unwrap_or(0) as i32;
                    self.current_counter += v;
                }
                83 => {
                    let v = p.next().unwrap_or(0) as i32;
                    self.current_counter = (self.current_counter - v).max(-1);
                }
                84 => {
                    let noun = self.last_noun.clone();
                    self.out.push_str(&noun);
                }
                85 => {
                    let noun = self.last_noun.clone();
                    self.out.push_str(&noun);
                    self.out.push('\n');
                }
                86 => self.out.push('\n'),
                87 => {
                    let idx = (p.next().unwrap_or(0) as usize).min(self.saved_rooms.len() - 1);
                    std::mem::swap(&mut self.player, &mut self.saved_rooms[idx]);
                }
                88 => { /* pause: host handles timing */ }
                89 => self.pending_picture = Some(p.next().unwrap_or(0)), // draw picture N
                _ => {} // 90..=101 unused
            }
        }
        did_continue
    }

    /// Run one full turn: parse, command match, built-in fallback, lamp, then the
    /// occurrence pass for the *next* prompt. ScottFree runs occurrences at the top
    /// of its loop, so they fire after the command that precedes them (seeing its
    /// effects); the opening pass runs in `Vm::new`.
    fn run_turn(&mut self, cmd: &str) {
        // Debug-inspector tracing (SQ-0464): reset this turn's records. Gated
        // on `trace_fired` so the off path is a single `bool` check and
        // nothing else — the vecs only ever grow while tracing is on, so
        // there is nothing to clear when it never was.
        if self.trace_fired {
            self.fired_actions.clear();
            self.last_blocked.clear();
        }
        let mut w = cmd.split_whitespace();
        let w1 = w.next();
        let w2 = w.next();
        // ScottFree's GetInput tries the FIRST word against the noun list
        // before anything else: a direction noun (1..=6) is "the Scott Adams
        // system['s] hack to avoid typing 'go'" — it becomes GO <dir> and the
        // second word is never consulted. Only otherwise is the first word
        // looked up as a verb. An unknown verb answers "You use word(s) I
        // don't know!" and NO turn passes — ScottFree re-prompts inside
        // GetInput, so no actions run, the lamp doesn't tick, and the
        // occurrence pass doesn't fire (SQ-0628).
        let first_as_noun = w1.and_then(|s| self.db.match_noun(s));
        let (vb, no): (u16, i32) = if let Some(d) =
            first_as_noun.filter(|d| (1..=6).contains(d))
        {
            (1, d as i32)
        } else if let Some(vb) = w1.and_then(|s| self.db.match_verb(s)) {
            // An absent or unrecognized second word is noun -1 (ScottFree's
            // WhichWord miss), NOT 0: wildcard (noun-0) actions still match
            // it, but GET/DROP answer "What?" instead of grabbing something,
            // and GO without a direction asks for one before the action table.
            let no = w2
                .and_then(|s| self.db.match_noun(s))
                .map(|n| n as i32)
                .unwrap_or(-1);
            (vb, no)
        } else {
            self.out.push_str("You use word(s) I don't know!\n");
            self.needs_line = true;
            return;
        };
        // ScottFree keeps NounText = the second word only (empty for a
        // one-word command), for the print-noun opcodes and the GET/DROP hack.
        self.last_noun = w2.unwrap_or("").to_uppercase();

        // ScottFree resolves GO + a compass direction from the room's exit table
        // BEFORE consulting the action table, so a catch-all "GO <anything>"
        // action (verb 1, noun 0) can't intercept ordinary movement. A GO with a
        // non-direction noun (e.g. GO TENT) still falls through to the actions.
        let mut handled = false;
        if vb == 1 && (1..=6).contains(&no) {
            let dir = no as usize - 1;
            if self.is_dark() {
                self.out.push_str("It is dangerous to move in the dark!\n");
            }
            let dest = self
                .db
                .rooms
                .get(self.player)
                .map(|room| room.exits[dir])
                .unwrap_or(0);
            // A destination outside the room table (a corrupt exit value in a
            // hand-built Database; the loader rejects them in a .dat) is
            // treated as no exit rather than soft-locking the player in a
            // nonexistent room (SQ-0629).
            if dest != 0 && dest < self.db.rooms.len() {
                self.player = dest;
            } else {
                self.out.push_str("I can't go in that direction.\n");
            }
            handled = true;
        } else if vb == 1 && no == -1 {
            // ScottFree: GO with no (or an unknown) noun is answered "Give me
            // a direction too." BEFORE the action table — a catch-all
            // "GO ANY" action must not swallow a bare GO.
            self.out.push_str("I need a direction.\n");
            handled = true;
        } else if vb != 0 {
            // Only a recognized verb consults the command actions. Verb 0 is
            // reserved for occurrences (the top-of-turn auto-event pass), so an
            // unrecognized command must NOT match them here — otherwise an
            // occurrence (e.g. a conditional death) fires as if it were the reply.
            // Manual loop (rather than `.iter().find(..)`) so a verb/noun match
            // whose conditions block it can be recorded for the debug
            // inspector's `last_blocked` (SQ-0464). `conditions` is copied out
            // of the borrowed `Action` (it's `Copy`, five small structs) so
            // the borrow ends before `eval_condition(&self)` is called —
            // otherwise short-circuits/perf are unchanged from `.find`.
            let mut matched = None;
            let mut blocked: Vec<(usize, usize)> = Vec::new();
            for i in 0..self.db.actions.len() {
                let a = &self.db.actions[i];
                let (av, an, conditions) = (a.verb, a.noun, a.conditions);
                // ScottFree's vocab match: `nv==no || nv==0` — a noun-0
                // wildcard also matches the unknown-noun sentinel (-1).
                if av != vb || (an as i32 != no && an != 0) {
                    continue;
                }
                let mut first_fail = None;
                for (slot, c) in conditions.iter().enumerate() {
                    if !self.eval_condition(c) {
                        first_fail = Some(slot);
                        break;
                    }
                }
                match first_fail {
                    None => {
                        matched = Some(i);
                        break;
                    }
                    Some(slot) if self.trace_fired => blocked.push((i, slot)),
                    Some(_) => {}
                }
            }
            if self.trace_fired {
                self.last_blocked = blocked;
            }
            if let Some(idx) = matched {
                self.run_action_chain(idx);
                handled = true;
            }
        }

        if !handled {
            if vb == 1 {
                self.out.push_str("I need a direction.\n");
            } else if vb == 10 {
                if self.last_noun == "ALL" {
                    self.get_all();
                } else if no == -1 {
                    // ScottFree: GET with an unknown noun is "What ?", never a grab.
                    self.out.push_str("What?\n");
                } else if self.carried_count() >= self.db.max_carry {
                    // ScottFree checks the carry limit before matching the item.
                    self.out.push_str("You are carrying too much.\n");
                } else {
                    match self.match_up_item(false) {
                        Some(idx) => {
                            self.set_item_loc(idx, CARRIED);
                            self.out.push_str("OK.\n");
                        }
                        None => self.out.push_str("It's beyond my power to do that.\n"),
                    }
                }
            } else if vb == 18 {
                if self.last_noun == "ALL" {
                    self.drop_all();
                } else if no == -1 {
                    self.out.push_str("What?\n");
                } else {
                    match self.match_up_item(true) {
                        Some(idx) => {
                            self.set_item_loc(idx, self.player as i32);
                            self.out.push_str("OK.\n");
                        }
                        None => self.out.push_str("It's beyond my power to do that.\n"),
                    }
                }
            } else {
                self.out.push_str("I don't understand your command.\n");
            }
        }

        if self.db.light_time != -1 && self.item_in_play(LIGHT_SOURCE) && self.lamp > 0 {
            self.lamp -= 1;
            if self.lamp == 0 {
                self.flag_set(LAMP_EMPTY_FLAG, true);
                self.out.push_str("Your light has run out.\n");
            }
        }

        // Occurrences for the next prompt (ScottFree's top-of-loop pass), unless the
        // command ended the game.
        if !self.quit {
            self.run_occurrences();
        }

        self.needs_line = true;
    }

    /// GET ALL: take every item in the current room that has an auto-get noun,
    /// respecting MaxCarry. Reports each taken item; a full pack stops the sweep.
    fn get_all(&mut self) {
        let mut took_any = false;
        for i in 0..self.item_loc.len() {
            if self.item_in_room(i) && self.db.items[i].auto_noun.is_some() {
                if self.carried_count() < self.db.max_carry {
                    self.set_item_loc(i, CARRIED);
                    let text = self.db.items[i].text.clone();
                    self.out.push_str(&text);
                    self.out.push_str(": OK.\n");
                    took_any = true;
                } else {
                    self.out.push_str("You are carrying too much.\n");
                    return;
                }
            }
        }
        if !took_any {
            self.out.push_str("I don't understand your command.\n");
        }
    }

    /// DROP ALL: drop every carried item that has an auto-get noun.
    fn drop_all(&mut self) {
        let mut dropped_any = false;
        for i in 0..self.item_loc.len() {
            if self.item_carried(i) && self.db.items[i].auto_noun.is_some() {
                self.set_item_loc(i, self.player as i32);
                let text = self.db.items[i].text.clone();
                self.out.push_str(&text);
                self.out.push_str(": OK.\n");
                dropped_any = true;
            }
        }
        if !dropped_any {
            self.out.push_str("I don't understand your command.\n");
        }
    }

    /// ScottFree's `MatchUpItem(NounText, loc)`: the first item whose
    /// `auto_noun` matches the last typed noun AND which is at the required
    /// location — the player's room for GET (`want_carried == false`), carried
    /// for DROP (`want_carried == true`). Without the location requirement a
    /// duplicate auto-noun (Adventureland's two `/BOT/` bottles, lit/unlit
    /// lamp pairs) resolves to the out-of-play twin first and GET/DROP fail
    /// wrongly (SQ-0628).
    fn match_up_item(&self, want_carried: bool) -> Option<usize> {
        // Scott matching is significant only to `word_length` characters, so the
        // typed noun and the item's auto-noun must be compared truncated — a full
        // typed word (e.g. "LAMP") still matches a shorter auto-noun ("LAM") when
        // word_length is 3.
        let wl = self.db.word_length;
        let trunc = |s: &str| s.trim().to_uppercase().chars().take(wl).collect::<String>();
        // ScottFree first maps the typed noun through the vocabulary
        // (MapSynonym): a `*`-synonym resolves to its group's canonical word
        // before being compared against the items' auto-get nouns. A word not
        // in the vocabulary at all is compared as typed.
        let key = match self.db.match_noun(&self.last_noun) {
            Some(n) => trunc(self.db.nouns[n as usize].trim_start_matches('*')),
            None => trunc(&self.last_noun),
        };
        if key.is_empty() {
            return None;
        }
        (0..self.db.items.len()).find(|&i| {
            let at_loc = if want_carried {
                self.item_carried(i)
            } else {
                self.item_in_room(i)
            };
            at_loc
                && self.db.items[i]
                    .auto_noun
                    .as_deref()
                    .map(|n| trunc(n) == key)
                    .unwrap_or(false)
        })
    }

    /// Occurrence pass: walk actions in order; for each verb==0 action, evaluate
    /// its conditions AT THAT POINT (so an earlier fired occurrence's state change
    /// is visible to a later guard), then fire it iff a d100 roll passes the
    /// noun-as-percent chance. A roll against 0 never passes, so noun==0 never
    /// fires standalone (those are continuation targets reached only via opcode 73).
    fn run_occurrences(&mut self) {
        for idx in 0..self.db.actions.len() {
            let (is_occ, noun, pass) = {
                let a = &self.db.actions[idx];
                let is_occ = a.verb == 0;
                let pass = is_occ && a.conditions.iter().all(|c| self.eval_condition(c));
                (is_occ, a.noun, pass)
            };
            if is_occ && pass && self.roll_100() < noun as u32 {
                self.run_action_chain(idx);
            }
        }
    }

    /// Run action `start`'s commands. If they executed `continue` (opcode 73),
    /// walk forward over CONSECUTIVE continuation lines (verb==0 && noun==0),
    /// running each one's commands only if its own conditions re-evaluate true.
    /// The walk stops at the first action whose vocab is not 0/0 (regardless of
    /// whether the intermediate lines themselves carry a 73).
    fn run_action_chain(&mut self, start: usize) {
        if !self.run_action_line(start) {
            return;
        }
        let mut idx = start + 1;
        while idx < self.db.actions.len() {
            let (is_cont, pass) = {
                let a = &self.db.actions[idx];
                let is_cont = a.verb == 0 && a.noun == 0;
                let pass = is_cont && a.conditions.iter().all(|c| self.eval_condition(c));
                (is_cont, pass)
            };
            if !is_cont {
                break;
            }
            if pass {
                self.run_action_line(idx);
            }
            idx += 1;
        }
    }

    /// Run one action line's commands, rebuilding its param list (the values of
    /// its code-0 conditions) fresh. Returns true if it executed `continue`.
    fn run_action_line(&mut self, idx: usize) -> bool {
        let (cmds, params) = {
            let a = &self.db.actions[idx];
            let params: Vec<u16> = a
                .conditions
                .iter()
                .filter(|c| c.code == 0)
                .map(|c| c.value)
                .collect();
            (a.commands, params)
        };
        let did_continue = self.run_commands(&cmds, &params);
        // SQ-0464: record this firing (both player-command and occurrence /
        // continuation actions run through here). Off path is one `bool`
        // check — no vec push, no set insert.
        if self.trace_fired {
            self.fired_actions.push(idx);
            self.ever_fired.insert(idx);
        }
        did_continue
    }

    fn print_message(&mut self, n: usize) {
        if let Some(msg) = self.db.messages.get(n) {
            self.out.push_str(msg);
            self.out.push('\n');
        }
    }

    /// True when the current room is unlit: the darkness flag is set and no light
    /// source (item 9) is either carried or present in the room.
    pub fn is_dark(&self) -> bool {
        self.flags[DARK_FLAG]
            && !(self.item_carried(LIGHT_SOURCE) || self.item_in_room(LIGHT_SOURCE))
    }

    /// The current room's display block, in the classic Scott Adams "top window"
    /// form: the room line, then "Obvious exits:", then "I can also see:". The host
    /// shows this as a persistent panel above the scrolling transcript (the CLI
    /// prints it inline), so it is a pure query and never touches `out`.
    ///
    /// A `*`-literal room prints verbatim; a non-literal room gets the "I'm in a "
    /// prefix. When the room is dark, only the darkness line is returned.
    pub fn room_block(&self) -> String {
        if self.is_dark() {
            return "It is too dark to see.".to_string();
        }
        let mut s = String::new();
        if let Some(room) = self.db.rooms.get(self.player) {
            if room.literal {
                s.push_str(&room.desc);
            } else {
                s.push_str("I'm in a ");
                s.push_str(&room.desc);
            }
            let names = ["North", "South", "East", "West", "Up", "Down"];
            let exits: Vec<&str> = room
                .exits
                .iter()
                .enumerate()
                .filter(|(_, &dest)| dest != 0)
                .map(|(i, _)| names[i])
                .collect();
            s.push_str("\n\nObvious exits: ");
            s.push_str(&if exits.is_empty() {
                "none".to_string()
            } else {
                format!("{}.", exits.join(". "))
            });
        }
        let visible: Vec<&str> = self
            .db
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| self.item_in_room(*i))
            .map(|(_, it)| it.text.as_str())
            .collect();
        if !visible.is_empty() {
            s.push_str("\n\nI can also see:");
            for item in &visible {
                s.push_str("\n  ");
                s.push_str(item);
            }
        }
        s
    }

    fn print_inventory(&mut self) {
        let carried: Vec<&str> = self
            .db
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| self.item_carried(*i))
            .map(|(_, it)| it.text.as_str())
            .collect();
        if carried.is_empty() {
            self.out.push_str("You are carrying nothing.\n");
        } else {
            self.out.push_str("You are carrying: ");
            self.out.push_str(&carried.join(", "));
            self.out.push('\n');
        }
    }

    /// The player's score as Scott counts it: treasures deposited in the
    /// treasure room, and the total there are to find.
    ///
    /// Scott stores no score anywhere — the `SCORE` verb recomputes this each
    /// time it is asked — so a host that wants to notice the score changing has
    /// to recount too. Cheap: it is a scan of the item table.
    pub fn treasures_stored(&self) -> (i32, i32) {
        let stored = self
            .db
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.treasure)
            .filter(|(i, _)| self.item_loc_of(*i) == Some(self.db.treasure_room as i32))
            .count() as i32;
        (stored, self.db.num_treasures)
    }

    fn print_score(&mut self) {
        let (in_treasure_room, total) = self.treasures_stored();
        self.out.push_str(&format!(
            "You have {in_treasure_room} out of {total} treasures.\n"
        ));
    }

    // --- test-only helpers (or make fields pub(crate) and set directly) ---
    #[cfg(test)]
    pub(crate) fn set_player(&mut self, r: usize) {
        self.player = r;
    }
    #[cfg(test)]
    pub(crate) fn set_flag(&mut self, i: usize, v: bool) {
        self.flags[i] = v;
    }
    #[cfg(test)]
    pub(crate) fn set_counter(&mut self, v: i32) {
        self.current_counter = v;
    }
    #[cfg(test)]
    pub(crate) fn item_loc_at(&self, idx: usize) -> i32 {
        self.item_loc[idx]
    }
    #[cfg(test)]
    pub(crate) fn flag_at(&self, i: usize) -> bool {
        self.flags[i]
    }
    #[cfg(test)]
    pub(crate) fn counter_at(&self) -> i32 {
        self.current_counter
    }
    #[cfg(test)]
    pub(crate) fn saved_room_at(&self) -> usize {
        self.saved_room
    }
    #[cfg(test)]
    pub(crate) fn backup_counter_at(&self, i: usize) -> i32 {
        self.counters[i]
    }
    #[cfg(test)]
    pub(crate) fn saved_rooms_at(&self, i: usize) -> usize {
        self.saved_rooms[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Condition, Database, Item, Room};
    fn vm_with(items: Vec<Item>, rooms: Vec<Room>, player: usize) -> Vm {
        let db = Database {
            max_carry: 6,
            start_room: player,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs: vec!["".into()],
            nouns: vec!["".into()],
            rooms,
            messages: vec!["".into()],
            items,
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);
        vm.set_player(player);
        vm
    }
    #[test]
    fn cond_carried_and_present() {
        let items = vec![
            Item {
                text: "limbo".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 0,
            },
            Item {
                text: "sword".into(),
                treasure: false,
                auto_noun: None,
                start_loc: -1,
            }, // carried
            Item {
                text: "rock".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 2,
            }, // in room 2
        ];
        let rooms = vec![
            Room {
                exits: [0; 6],
                desc: "limbo".into(),
                literal: true,
            },
            Room {
                exits: [0; 6],
                desc: "r1".into(),
                literal: true,
            },
            Room {
                exits: [0; 6],
                desc: "r2".into(),
                literal: true,
            },
        ];
        let vm = vm_with(items, rooms, 2);
        assert!(vm.eval_condition(&Condition { code: 1, value: 1 })); // sword carried
        assert!(vm.eval_condition(&Condition { code: 2, value: 2 })); // rock in room
        assert!(vm.eval_condition(&Condition { code: 3, value: 1 })); // sword present
        assert!(vm.eval_condition(&Condition { code: 6, value: 2 })); // rock not carried
        assert!(vm.eval_condition(&Condition { code: 4, value: 2 })); // player in room 2
        assert!(vm.eval_condition(&Condition { code: 0, value: 99 })); // param always true
        assert!(vm.eval_condition(&Condition { code: 5, value: 1 })); // sword NOT in room 2 (it's carried)
        assert!(vm.eval_condition(&Condition { code: 13, value: 2 })); // rock in play
        assert!(vm.eval_condition(&Condition { code: 17, value: 2 })); // rock still in initial room 2
    }
    #[test]
    fn cond_flags_and_counters() {
        let mut vm = vm_with(
            vec![Item {
                text: "x".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 0,
            }],
            vec![Room {
                exits: [0; 6],
                desc: "r".into(),
                literal: true,
            }],
            0,
        );
        vm.set_flag(8, true);
        assert!(vm.eval_condition(&Condition { code: 8, value: 8 }));
        assert!(vm.eval_condition(&Condition { code: 9, value: 7 })); // flag 7 clear
        vm.set_counter(5);
        assert!(vm.eval_condition(&Condition { code: 15, value: 5 })); // counter <= 5
        assert!(vm.eval_condition(&Condition { code: 19, value: 5 })); // counter == 5
        assert!(vm.eval_condition(&Condition { code: 16, value: 4 })); // counter > 4
    }

    fn rooms4() -> Vec<Room> {
        (0..4)
            .map(|i| Room {
                exits: [0; 6],
                desc: format!("room{i}"),
                literal: true,
            })
            .collect()
    }

    #[test]
    fn cmd_get_drop_goto_flags_counters() {
        let items = vec![
            Item {
                text: "limbo".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 0,
            },
            Item {
                text: "item1".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 1,
            },
            Item {
                text: "item2".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 2,
            },
        ];
        let mut vm = vm_with(items, rooms4(), 1);

        // GET item1
        assert!(!vm.run_commands(&[52, 0, 0, 0], &[1]));
        assert_eq!(vm.item_loc_at(1), CARRIED);

        // DROP item1
        vm.run_commands(&[53, 0, 0, 0], &[1]);
        assert_eq!(vm.item_loc_at(1), vm.current_room() as i32);

        // GOTO room 3
        vm.run_commands(&[54, 0, 0, 0], &[3]);
        assert_eq!(vm.current_room(), 3);

        // set/clear flag 4
        vm.run_commands(&[58, 0, 0, 0], &[4]);
        assert!(vm.flag_at(4));
        vm.run_commands(&[60, 0, 0, 0], &[4]);
        assert!(!vm.flag_at(4));

        // counter set/add/sub (all act on the single live current counter)
        vm.run_commands(&[79, 0, 0, 0], &[7]);
        assert_eq!(vm.counter_at(), 7);
        vm.run_commands(&[82, 0, 0, 0], &[2]);
        assert_eq!(vm.counter_at(), 9);
        vm.run_commands(&[83, 0, 0, 0], &[3]);
        assert_eq!(vm.counter_at(), 6);
    }

    fn one_item() -> Vec<Item> {
        vec![Item {
            text: "x".into(),
            treasure: false,
            auto_noun: None,
            start_loc: 0,
        }]
    }

    // CRIT-2: op 81 SWAPs the live current counter with a backup register; a
    // stash-change-restore round trip must recover the original value. Under the
    // old "select an index" model the final value would be 3, not 7.
    #[test]
    fn cmd_counter_swap_op81() {
        let mut vm = vm_with(one_item(), rooms4(), 0);
        vm.run_commands(&[79, 0, 0, 0], &[7]); // current = 7
        assert_eq!(vm.counter_at(), 7);
        vm.run_commands(&[81, 0, 0, 0], &[2]); // swap into backup 2: current<-0, backup2<-7
        assert_eq!(vm.counter_at(), 0);
        assert_eq!(vm.backup_counter_at(2), 7);
        vm.run_commands(&[79, 0, 0, 0], &[3]); // change current
        assert_eq!(vm.counter_at(), 3);
        vm.run_commands(&[81, 0, 0, 0], &[2]); // swap back: current<-7 (restored)
        assert_eq!(vm.counter_at(), 7);
        assert_eq!(vm.backup_counter_at(2), 3);
    }

    // CRIT-2 / MIN-7: ops 77 and 83 floor the current counter at -1.
    #[test]
    fn cmd_counter_floors_at_minus_one() {
        let mut vm = vm_with(one_item(), rooms4(), 0);
        vm.run_commands(&[79, 0, 0, 0], &[0]); // current = 0
        vm.run_commands(&[77, 0, 0, 0], &[]); // dec -> -1
        assert_eq!(vm.counter_at(), -1);
        vm.run_commands(&[77, 0, 0, 0], &[]); // dec again -> stays -1
        assert_eq!(vm.counter_at(), -1);
        vm.run_commands(&[79, 0, 0, 0], &[2]); // current = 2
        vm.run_commands(&[83, 0, 0, 0], &[10]); // sub 10 -> floor at -1
        assert_eq!(vm.counter_at(), -1);
    }

    // MIN-6: op 80 (saved_room scalar) and op 87 (saved_rooms array) are distinct
    // registers. Previously op 80 aliased saved_rooms[0]; here changing the array
    // must not disturb the op-80 register and vice-versa.
    #[test]
    fn cmd_room_registers_distinct_op80_op87() {
        let mut vm = vm_with(one_item(), rooms4(), 1);
        // Seed saved_rooms[0] = 2 via op 87, then reset player to 1.
        vm.set_player(2);
        vm.run_commands(&[87, 0, 0, 0], &[0]); // swap player(2) <-> saved_rooms[0](0)
        assert_eq!(vm.current_room(), 0);
        assert_eq!(vm.saved_rooms_at(0), 2);
        vm.set_player(1);
        // op 80 swaps player <-> the SEPARATE saved_room scalar (still 0), NOT array[0].
        vm.run_commands(&[80, 0, 0, 0], &[]);
        assert_eq!(vm.current_room(), 0); // would be 2 if op80 wrongly used saved_rooms[0]
        assert_eq!(vm.saved_room_at(), 1);
        assert_eq!(vm.saved_rooms_at(0), 2); // op80 left the op87 array untouched
    }

    // A catch-all "GO <anything>" action (verb 1, noun 0) must not intercept
    // compass movement: ScottFree consults the exit table first (Mysterious
    // Adventures regression — circus "go south" printed "Sorry" and never moved).
    #[test]
    fn go_direction_beats_catch_all_go_action() {
        let mut verbs = vec![String::new(); 2];
        verbs[1] = "GO".into();
        let nouns: Vec<String> = ["ANY", "NORTH", "SOUTH", "EAST", "WEST", "UP", "DOWN"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut rooms = rooms4();
        rooms[1].exits[1] = 2; // south -> room 2
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 4,
            light_time: -1,
            treasure_room: 0,
            actions: vec![Action {
                verb: 1,
                noun: 0,
                conditions: [Condition { code: 0, value: 0 }; 5], // always matches
                commands: [1, 0, 0, 0],                           // print message 1 ("Sorry")
            }],
            verbs,
            nouns,
            rooms,
            messages: vec![String::new(), "Sorry".into()],
            items: one_item(),
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);
        vm.take_output();
        vm.supply_line("go south");
        vm.step();
        let out = vm.take_output();
        assert_eq!(vm.current_room(), 2, "GO SOUTH moves via the exit table");
        assert!(!out.contains("Sorry"), "catch-all GO action must not fire: {out:?}");
    }

    // An unrecognized command (verb 0) must not match verb-0 occurrence actions
    // as if they were the reply — otherwise a conditional occurrence fires on a
    // typo (Mysterious Adventures regression: "knock" in circus killed the player).
    #[test]
    fn unknown_command_does_not_fire_a_verb0_occurrence() {
        let verbs = vec!["AUTO".to_string(), "GO".to_string()];
        let nouns = vec!["ANY".to_string(), "NORTH".to_string()];
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 4,
            light_time: -1,
            treasure_room: 0,
            actions: vec![Action {
                verb: 0,
                noun: 0, // 0% occurrence: never fires on its own, but the old
                // command loop wrongly matched it on an unknown command.
                conditions: [Condition { code: 0, value: 0 }; 5],
                commands: [63, 0, 0, 0], // quit
            }],
            verbs,
            nouns,
            rooms: rooms4(),
            messages: vec![String::new()],
            items: one_item(),
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);
        vm.take_output();
        vm.supply_line("xyzzy"); // unrecognized verb
        vm.step();
        assert!(!vm.has_quit(), "an unknown command must not fire a verb-0 occurrence");
        assert!(
            vm.take_output().contains("You use word(s) I don't know!"),
            "an unknown command gets ScottFree's unknown-words reply"
        );
    }

    // current_picture defaults to the room number and is overridden by opcode 89
    // (draw picture) until the next command resets it.
    #[test]
    fn current_picture_room_default_and_op89_override() {
        let mut vm = vm_with(one_item(), rooms4(), 1);
        assert_eq!(vm.current_picture(), Some(1), "defaults to the current room");
        vm.run_commands(&[89, 0, 0, 0], &[7]); // draw picture 7
        assert_eq!(vm.current_picture(), Some(7), "opcode 89 overrides");
        vm.supply_line("look");
        vm.step(); // a fresh turn drops the override
        assert_eq!(vm.current_picture(), Some(1), "reverts to the room picture");
    }

    // Action codes >= 102 print message (code - 50) with no upper cap: a game
    // with 100+ messages emits them via codes >= 152, which must not be dropped.
    #[test]
    fn cmd_message_code_above_149_prints_high_message() {
        let mut messages = vec![String::new(); 103];
        messages[102] = "you found the hidden vault".into();
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs: vec![String::new(); 2],
            nouns: vec![String::new(); 2],
            rooms: rooms4(),
            messages,
            items: one_item(),
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);
        vm.take_output();
        vm.run_commands(&[152, 0, 0, 0], &[]); // 152 - 50 -> message 102
        assert!(
            vm.take_output().contains("you found the hidden vault"),
            "message code 152 prints message 102"
        );
    }

    // Opcode 71 (SAVE GAME) raises a transient, host-drained save request; it is
    // not the quit flag and does not persist across a take.
    #[test]
    fn cmd_save_request_flag_op71() {
        let mut vm = vm_with(one_item(), rooms4(), 1);
        assert!(!vm.take_save_request(), "no save requested before op 71");
        vm.run_commands(&[71, 0, 0, 0], &[]);
        assert!(!vm.has_quit(), "SAVE GAME does not quit");
        assert!(vm.take_save_request(), "op 71 raises the save request");
        assert!(!vm.take_save_request(), "the request clears once taken");
    }

    // MIN-8: GET ALL / DROP ALL sweep every auto-noun item in the room / pack.
    #[test]
    fn get_all_and_drop_all() {
        let items = vec![
            Item {
                text: "limbo".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 0,
            },
            Item {
                text: "a coin".into(),
                treasure: false,
                auto_noun: Some("COIN".into()),
                start_loc: 1,
            },
            Item {
                text: "a key".into(),
                treasure: false,
                auto_noun: Some("KEY".into()),
                start_loc: 1,
            },
        ];
        let mut verbs = vec![String::new(); 19];
        verbs[1] = "GO".into();
        verbs[10] = "GET".into();
        verbs[18] = "DROP".into();
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs,
            nouns: vec![String::new(); 7], // no "ALL" in vocab -> matched via last_noun
            rooms: rooms4(),
            messages: vec![String::new()],
            items,
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);
        vm.take_output();

        vm.supply_line("get all");
        vm.step();
        let out = vm.take_output();
        assert!(out.contains("a coin"), "get all names each item: {out:?}");
        assert!(out.contains("a key"), "get all names each item: {out:?}");
        assert_eq!(vm.item_loc_at(1), CARRIED);
        assert_eq!(vm.item_loc_at(2), CARRIED);

        vm.supply_line("drop all");
        vm.step();
        vm.take_output();
        assert_eq!(vm.item_loc_at(1), 1); // dropped back into room 1
        assert_eq!(vm.item_loc_at(2), 1);
    }

    // Snapshot/restore must round-trip the new current_counter scalar, the backup
    // registers, and the op-80 saved_room register.
    #[test]
    fn snapshot_restore_counter_and_room() {
        let mut vm = vm_with(one_item(), rooms4(), 1);
        vm.run_commands(&[79, 0, 0, 0], &[9]); // current = 9
        vm.run_commands(&[81, 0, 0, 0], &[2]); // swap -> current=0, backup2=9
        vm.run_commands(&[79, 0, 0, 0], &[5]); // current = 5
        vm.run_commands(&[80, 0, 0, 0], &[]); // player(1) <-> saved_room(0): player=0, saved_room=1
        assert_eq!(vm.current_room(), 0);
        assert_eq!(vm.saved_room_at(), 1);

        let snap = vm.snapshot();

        vm.run_commands(&[79, 0, 0, 0], &[99]); // current = 99
        vm.run_commands(&[80, 0, 0, 0], &[]); // player=1, saved_room=0
        assert_ne!(vm.counter_at(), 5);

        vm.restore(&snap).expect("restore");
        assert_eq!(vm.counter_at(), 5);
        assert_eq!(vm.backup_counter_at(2), 9);
        assert_eq!(vm.saved_room_at(), 1);
        assert_eq!(vm.current_room(), 0);
    }

    #[test]
    fn cmd_message_ranges() {
        let mut messages = vec!["".to_string(); 53];
        messages[1] = "hello".into();
        messages[52] = "deep".into();
        let items = vec![Item {
            text: "x".into(),
            treasure: false,
            auto_noun: None,
            start_loc: 0,
        }];
        let db = Database {
            max_carry: 6,
            start_room: 0,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs: vec!["".into()],
            nouns: vec!["".into()],
            rooms: vec![Room {
                exits: [0; 6],
                desc: "r".into(),
                literal: true,
            }],
            messages,
            items,
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);

        vm.run_commands(&[1, 0, 0, 0], &[]);
        assert!(vm.take_output().contains("hello"));
        vm.run_commands(&[102, 0, 0, 0], &[]); // 102 -> message 52
        assert!(vm.take_output().contains("deep"));
    }

    #[test]
    fn cmd_continue_flag_and_quit() {
        let items = vec![Item {
            text: "x".into(),
            treasure: false,
            auto_noun: None,
            start_loc: 0,
        }];
        let mut vm = vm_with(items.clone(), rooms4(), 0);
        assert!(vm.run_commands(&[73, 0, 0, 0], &[]));

        let mut vm2 = vm_with(items, rooms4(), 0);
        vm2.run_commands(&[63, 0, 0, 0], &[]);
        assert!(vm2.has_quit());
    }

    fn tiny_world() -> Database {
        let mut items = Vec::new();
        for i in 0..10 {
            if i == 9 {
                items.push(Item {
                    text: "lamp".into(),
                    treasure: false,
                    auto_noun: Some("LAMP".into()),
                    start_loc: 1,
                });
            } else {
                items.push(Item {
                    text: format!("filler{i}"),
                    treasure: false,
                    auto_noun: None,
                    start_loc: 0,
                });
            }
        }
        let mut verbs = vec![String::new(); 19];
        verbs[1] = "GO".into();
        verbs[10] = "GET".into();
        verbs[18] = "DROP".into();
        let mut nouns = vec![String::new(); 8];
        nouns[1] = "NORTH".into();
        nouns[2] = "SOUTH".into();
        nouns[3] = "EAST".into();
        nouns[4] = "WEST".into();
        nouns[5] = "UP".into();
        nouns[6] = "DOWN".into();
        // The lamp must be in the noun vocabulary: ScottFree answers "What ?"
        // to a GET whose noun WhichWord doesn't know, so a gettable item's
        // noun is always listed in a real game.
        nouns[7] = "LAMP".into();
        let rooms = vec![
            Room {
                exits: [0; 6],
                desc: "limbo".into(),
                literal: true,
            },
            Room {
                exits: [2, 0, 0, 0, 0, 0],
                desc: "forest clearing".into(),
                literal: true,
            },
            Room {
                exits: [0, 1, 0, 0, 0, 0],
                desc: "damp cave".into(),
                literal: true,
            },
        ];
        Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs,
            nouns,
            rooms,
            messages: vec![String::new()],
            items,
            adventure_number: 0,
        }
    }

    #[test]
    fn walk_north_moves_and_describes() {
        let mut vm = Vm::new(tiny_world());
        vm.seed_rng(1);
        vm.supply_line("go north");
        assert_eq!(vm.step(), StepResult::NeedLine);
        assert_eq!(vm.current_room(), 2);
        // The destination is described by the room panel (room_block), not the
        // scrolling transcript.
        let block = vm.room_block();
        assert!(block.to_lowercase().contains("cave"), "moved into the cave: {block:?}");
        assert!(block.contains("Obvious exits: South."), "exits listed: {block:?}");
    }

    #[test]
    fn room_block_prefix_exits_items_and_darkness() {
        let mut db = tiny_world();
        // A non-literal noun room with North + Down exits; the lamp (item 9) is here.
        db.rooms[1].literal = false;
        db.rooms[1].desc = "forest".into();
        db.rooms[1].exits = [2, 0, 0, 0, 0, 3];
        let mut vm = Vm::new(db);

        // Non-literal rooms get the "I'm in a " prefix; exits and items are
        // period-separated (C64 style).
        assert_eq!(
            vm.room_block(),
            "I'm in a forest\n\nObvious exits: North. Down.\n\nI can also see:\n  lamp"
        );

        // With the darkness flag set and no light source in play, only the dark
        // line shows — no room text, exits, or items.
        vm.set_item_loc(LIGHT_SOURCE, 0); // banish the lamp to nowhere
        vm.flag_set(DARK_FLAG, true);
        assert!(vm.is_dark());
        assert_eq!(vm.room_block(), "It is too dark to see.");
    }

    #[test]
    fn get_specific_item_matches_at_word_length() {
        let mut db = tiny_world(); // word_length 3
        // A real game stores auto-nouns at word-length significance: the lamp
        // (item 9, starting in room 1) carries the 3-char auto-noun "LAM".
        db.items[9].auto_noun = Some("LAM".into());
        let mut vm = Vm::new(db);
        // Typing the full word "lamp" must still match "LAM" (both truncate to 3).
        vm.supply_line("get lamp");
        vm.step();
        let out = vm.take_output();
        assert!(out.contains("OK."), "get lamp succeeds: {out:?}");
        assert!(vm.item_carried(9), "the lamp is now carried");

        // And dropping it by the full word works too.
        vm.supply_line("drop lamp");
        vm.step();
        assert!(!vm.item_carried(9), "the lamp was dropped");
    }

    #[test]
    fn opening_occurrence_fires_before_first_prompt() {
        let mut db = tiny_world();
        db.messages = vec![String::new(), "A Voice BOOOMS out: Beware!".into()];
        // An always-occurrence (verb 0, noun 100) printing message 1, no guard.
        db.actions.push(Action {
            verb: 0,
            noun: 100,
            conditions: [Condition { code: 0, value: 0 }; 5],
            commands: [1, 0, 0, 0],
        });
        // The opening message is available immediately, before any command.
        let mut vm = Vm::new(db);
        let intro = vm.take_output();
        assert!(
            intro.contains("A Voice BOOOMS out"),
            "opening occurrence shows on load: {intro:?}"
        );
    }

    #[test]
    fn bare_direction_moves() {
        let mut vm = Vm::new(tiny_world());
        let _ = vm.take_output();
        vm.supply_line("north");
        vm.step();
        assert_eq!(vm.current_room(), 2);
        vm.supply_line("south");
        vm.step();
        assert_eq!(vm.current_room(), 1);
    }

    #[test]
    fn get_lamp_then_inventory() {
        let mut vm = Vm::new(tiny_world());
        let _ = vm.take_output();
        vm.supply_line("get lamp");
        vm.step();
        vm.supply_line("inventory");
        vm.step(); // no INVENTORY verb in tiny_world -> "don't understand"; instead assert the item moved
        // Better: assert the lamp is now carried.
        assert_eq!(vm.item_loc_at(LIGHT_SOURCE), CARRIED);
    }

    #[test]
    fn blocked_direction_reports() {
        let mut vm = Vm::new(tiny_world());
        let _ = vm.take_output();
        vm.supply_line("east"); // room1 has no east exit
        vm.step();
        assert!(vm.take_output().to_lowercase().contains("can't go"));
        assert_eq!(vm.current_room(), 1);
    }

    // --- SQ-0464: fired-action tracing --------------------------------

    #[test]
    fn trace_fired_off_by_default_records_nothing() {
        // Verb index 1 is GO by Scott convention (a bare GO-verb asks for a
        // direction before the action table), so PUSH lives at index 2.
        let mut verbs = vec![String::new(); 3];
        verbs[2] = "PUSH".into();
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 4,
            light_time: -1,
            treasure_room: 0,
            actions: vec![Action {
                verb: 2,
                noun: 0,
                conditions: [Condition { code: 0, value: 0 }; 5],
                commands: [1, 0, 0, 0], // print message 1
            }],
            verbs,
            nouns: vec![String::new(); 1],
            rooms: rooms4(),
            messages: vec![String::new(), "Click.".into()],
            items: one_item(),
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);
        assert!(!vm.trace_fired(), "tracing is off by default");
        vm.supply_line("push");
        vm.step();
        assert!(vm.fired_actions().is_empty(), "tracing off: no per-turn firings recorded");
        assert!(vm.ever_fired().is_empty(), "tracing off: no cumulative firings recorded");
    }

    #[test]
    fn trace_fired_records_per_turn_and_cumulative_firings() {
        // Verb index 1 is GO by Scott convention, so the test verbs start at 2.
        let mut verbs = vec![String::new(); 4];
        verbs[2] = "PUSH".into();
        verbs[3] = "PULL".into();
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 4,
            light_time: -1,
            treasure_room: 0,
            actions: vec![
                Action {
                    verb: 2,
                    noun: 0,
                    conditions: [Condition { code: 0, value: 0 }; 5],
                    commands: [1, 0, 0, 0], // idx 0: print message 1
                },
                Action {
                    verb: 3,
                    noun: 0,
                    conditions: [Condition { code: 0, value: 0 }; 5],
                    commands: [2, 0, 0, 0], // idx 1: print message 2
                },
            ],
            verbs,
            nouns: vec![String::new(); 1],
            rooms: rooms4(),
            messages: vec![String::new(), "Click.".into(), "Clunk.".into()],
            items: one_item(),
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);
        vm.set_trace_fired(true);

        vm.supply_line("push");
        vm.step();
        assert_eq!(vm.fired_actions(), &[0]);
        assert!(vm.ever_fired().contains(&0));

        vm.supply_line("pull");
        vm.step();
        // Per-turn list is turn-scoped: only this turn's firing (idx 1).
        assert_eq!(vm.fired_actions(), &[1]);
        // The cumulative set keeps both turns' firings.
        assert!(vm.ever_fired().contains(&0));
        assert!(vm.ever_fired().contains(&1));
    }

    #[test]
    fn trace_fired_records_blocked_action_when_verb_noun_matches_but_condition_fails() {
        // Verb index 1 is GO by Scott convention, so OPEN lives at index 2.
        let mut verbs = vec![String::new(); 3];
        verbs[2] = "OPEN".into();
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 4,
            light_time: -1,
            treasure_room: 0,
            actions: vec![Action {
                verb: 2,
                noun: 0,
                conditions: [
                    Condition { code: 8, value: 5 }, // requires flag 5 set
                    Condition { code: 0, value: 0 },
                    Condition { code: 0, value: 0 },
                    Condition { code: 0, value: 0 },
                    Condition { code: 0, value: 0 },
                ],
                commands: [1, 0, 0, 0],
            }],
            verbs,
            nouns: vec![String::new(); 1],
            rooms: rooms4(),
            messages: vec![String::new(), "It opens.".into()],
            items: one_item(),
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);
        vm.set_trace_fired(true);

        vm.supply_line("open"); // flag 5 is not set -> verb/noun match, condition blocks it
        vm.step();
        assert!(vm.fired_actions().is_empty(), "the guarded action must not have fired");
        assert_eq!(vm.last_blocked(), &[(0, 0)], "action 0's condition slot 0 blocked it");

        vm.set_flag(5, true);
        vm.supply_line("open"); // now the guard passes
        vm.step();
        assert_eq!(vm.fired_actions(), &[0]);
        assert!(vm.last_blocked().is_empty(), "no longer blocked once it fires");
    }

    #[test]
    fn seed_ever_fired_merges_persisted_coverage() {
        let mut vm = vm_with(one_item(), rooms4(), 0);
        let mut seed: HashSet<u32> = HashSet::new();
        seed.insert(3);
        seed.insert(7);
        vm.seed_ever_fired(&seed);
        assert!(vm.ever_fired().contains(&3));
        assert!(vm.ever_fired().contains(&7));
    }
}
