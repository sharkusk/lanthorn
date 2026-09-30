//! Learn which RAM word holds a Glulx game's *player* global, for the stories
//! whose avatar [`gvm::objects::ParseNames::find_player`]'s static scan cannot
//! find at all (SQ-1655).
//!
//! ## The story that motivates this
//!
//! Counterfeit Monkey compiles its player-facing `Understand` grammar into a
//! `parse_name` ROUTINE — ordinary Inform 7 behaviour whenever the grammar is
//! conditional or multi-word — so none of its 2,494 objects answers to
//! `yourself`/`myself`/`self`, and the avatar carries no hardware short name
//! either. `find_player`'s static scan is correct to refuse rather than guess
//! (see that function's own doc), but the refusal means
//! [`crate::glulx_session::GlulxSession::player_addr`] returns `None` for the
//! whole session, and item tracking never reports a single carried object even
//! as the game's own prose confirms the player is holding one.
//!
//! ## The precedent this ports: `glulx_roomlock`
//!
//! [`crate::glulx_roomlock::RoomLock`] solves the identical class of problem
//! for a DIFFERENT global (`location`): nothing in a Glulx image says which RAM
//! word is which, so it is found by *observation* instead — watching what
//! changes in step with something the host can independently confirm, and
//! trusting a candidate once the evidence is one-sided.
//!
//! `PlayerLock` is that same idea aimed at a different fact. Inform's standard
//! library keeps the avatar's object number in a conventionally-named global
//! (`player`, and usually a second alias such as `actor`) exactly the way it
//! keeps the current room in `location` — nothing in the compiled image says
//! WHERE that global lives, but it can be found the same way: by matching a RAM
//! word's value against a truth the host already knows.
//!
//! ## Why the room-lock's own scoring mechanism does not transfer
//!
//! `RoomLock` needs several turns because `location` changes on nearly every
//! room transition, and a handful of *other* words — turn counters, RNG state —
//! also change every turn and would tie a bare "did it change" test. Only a
//! turn-over-turn CORRELATION separates the real global from the noise.
//!
//! The player global is the opposite shape: it is expected to stay constant for
//! nearly the whole game (a story can legitimately swap the avatar's body —
//! Counterfeit Monkey's own plot is literally about identity synthesis — but
//! rarely, and never on a predictable schedule). A word that never changes
//! correlates with nothing, so `RoomLock`'s scoring has no foothold here at
//! all.
//!
//! What the host has instead is a much STRONGER single fact than "a room
//! changed": a complete, unambiguous item-tracking event. When a command the
//! player typed makes an object visibly leave the room and become held by
//! something else, that something else — once checked for being a story
//! object that is itself SITUATED somewhere, the same plausibility test
//! `find_player` already applies to its own candidates — can only sensibly be
//! the avatar. `GlulxSession::learn_player_from_pickups` is what recognises
//! that event (comparing a room's live children turn over turn) and hands this
//! module the one fact it asks for: the avatar's own object address, already
//! validated. This module never touches [`gvm::memory::Memory`] or
//! [`gvm::objects::ParseNames`] itself — exactly like `RoomLock`, it is a pure
//! state machine over RAM snapshots and addresses the caller supplies.
//!
//! An exact address match is strong enough evidence to commit on the FIRST
//! one: `RoomLock::name_witness` already establishes the precedent that a
//! single well-validated match locks immediately rather than waiting for a
//! correlation window (see that function's own doc), and this evidence is
//! stronger still — a coincidental RAM word holding one exact 21-plus-bit
//! address by chance is vanishingly unlikely, where `name_witness` only ever
//! had a room NAME (a much smaller space) to go on. Multiple RAM words
//! matching at once are not noise but the expected shape: Inform keeps
//! `player` and `actor` (or similar) side by side, both holding the avatar, the
//! same "aliases of one global" pattern `RoomLock` already documents for
//! `location`/`real_location` — so the lowest address wins, the same
//! deterministic, reproducible tie-break `RoomLock` uses for the same reason.
//!
//! ## Falsifying a lock
//!
//! Every later confirmed pickup re-checks the locked word against the SAME
//! truth that would have located it fresh: if the word no longer holds the
//! avatar address the evidence just proved, that is the story contradicting
//! the guess outright (not a heuristic, exactly the standard
//! `RoomLock::check_room_lock_against_story` already holds itself to) — the
//! address is rejected, never reoffered for the life of this learner, and the
//! very same turn's evidence is free to lock a different word immediately.
//! This is deliberately weaker than `RoomLock`'s multi-layered falsification
//! apparatus (no frozen-lock counter, no passive per-turn object-validity
//! check): that apparatus was built up over a long field record of Glulx games
//! actively lying to a locked `location` global (Anchorhead's *room gone to*,
//! SQ-1305's stale-sidecar shape), and no analogous record exists yet for a
//! player global. A contradiction is still caught the next time the evidence
//! that could prove one actually arrives; what is not attempted is guessing at
//! staleness in between.
//!
//! ## What this is not
//!
//! A fallback net, never an override:
//! [`crate::glulx_session::GlulxSession::player_addr`] only ever consults
//! [`PlayerLock::locked`] after `find_player`'s static scan has already
//! answered `None`. A story whose avatar the static scan already identifies
//! never has this learner's answer read at all, however it happens to be
//! learning in the background.

/// The learn/lock/unlock state machine for the `player` global.
///
/// Scans the SAME RAM window [`crate::glulx_roomlock::RoomLock`] does — both
/// learners are fed the identical snapshot by
/// [`crate::glulx_session::GlulxSession`], and Inform lays every one of its
/// globals out at the very start of RAM regardless of which one a given global
/// is, so there is no reason for the two windows to differ.
pub struct PlayerLock {
    /// Base address of the scanned region; `ram[i]` is the word at `base + i*4`.
    base: u32,
    /// The locked address — of the GLOBAL itself (a RAM cell), not the avatar's
    /// own object address. Dereferencing it (`mem.read32`) gives the avatar,
    /// exactly as [`crate::glulx_session::GlulxSession::location_addr`]
    /// dereferences [`crate::glulx_roomlock::RoomLock::locked`] for `location`.
    locked: Option<u32>,
    /// Addresses a lock was taken on and then caught out by a later
    /// contradicting confirmation, sorted — never offered as a candidate again
    /// for the life of this learner, the same discipline
    /// [`crate::glulx_roomlock::RoomLock::reject`] documents and for the same
    /// reason: without it a rejection is a loop rather than a correction, since
    /// the very next confirmed pickup would simply re-elect the same losing
    /// word by the same tie-break that chose it the first time.
    rejected: Vec<u32>,
}

impl PlayerLock {
    /// A learner for the RAM region starting at `base`.
    pub fn new(base: u32) -> Self {
        PlayerLock { base, locked: None, rejected: Vec::new() }
    }

    /// A learner that starts already locked to `addr` — the per-game sidecar
    /// remembers the address across runs, mirroring
    /// [`crate::glulx_roomlock::RoomLock::locked_at`].
    pub fn locked_at(base: u32, addr: u32) -> Self {
        let mut l = PlayerLock::new(base);
        l.locked = Some(addr);
        l
    }

    /// The locked GLOBAL address, or `None` while still learning. Dereference
    /// it against the current RAM to get the avatar's own object address.
    pub fn locked(&self) -> Option<u32> {
        self.locked
    }

    /// The index into a scanned-RAM snapshot for `addr`, bounded by `len` (the
    /// snapshot's own word count) — `None` for an address below `base` (which
    /// would underflow the subtraction) or beyond the snapshot actually held,
    /// mirroring [`crate::glulx_roomlock::RoomLock`]'s own `word_index`.
    fn word_index(&self, addr: u32, len: usize) -> Option<usize> {
        let idx = (addr.checked_sub(self.base)? / 4) as usize;
        (idx < len).then_some(idx)
    }

    /// Has `addr` already been caught out once?
    fn is_rejected(&self, addr: u32) -> bool {
        self.rejected.binary_search(&addr).is_ok()
    }

    fn reject(&mut self, addr: u32) {
        if let Err(i) = self.rejected.binary_search(&addr) {
            self.rejected.insert(i, addr);
        }
        if self.locked == Some(addr) {
            self.locked = None;
        }
    }

    /// Fold in one confirmed pickup: `ram` is this turn's snapshot of the
    /// scanned window (the same one [`crate::glulx_roomlock::RoomLock`] is fed),
    /// and `avatar` is the object address the caller has already validated as
    /// plausibly the player — see the module doc for what that validation is
    /// and why it belongs to the caller, not here.
    ///
    /// While unlocked: every RAM word whose CURRENT value equals `avatar`
    /// becomes a candidate, and the lowest address among them locks
    /// immediately (see the module doc for why one confirmation is enough and
    /// why the tie-break is address order). A turn whose window holds no such
    /// word teaches nothing and is silently ignored — not every turn's
    /// evidence has to land inside a 64 KB window for the story to have one
    /// that does.
    ///
    /// Once locked: re-checks the locked word against this SAME truth. Still
    /// equal is silent confirmation. Different is the story contradicting the
    /// guess outright — the address is rejected (never re-offered) and the
    /// SAME turn's evidence is immediately eligible to lock a different word,
    /// so one contradicting confirmation costs at most one turn of learning,
    /// not a whole new multi-turn wait.
    pub fn observe_pickup(&mut self, ram: &[u32], avatar: u32) {
        if let Some(locked) = self.locked {
            match self.word_index(locked, ram.len()) {
                Some(idx) if ram[idx] == avatar => return, // still agrees; nothing to do
                _ => self.reject(locked),
            }
        }
        let mut best: Option<u32> = None;
        for (i, &v) in ram.iter().enumerate() {
            if v != avatar {
                continue;
            }
            let addr = self.base + (i as u32) * 4;
            if self.is_rejected(addr) {
                continue;
            }
            if best.is_none_or(|b| addr < b) {
                best = Some(addr);
            }
        }
        if let Some(addr) = best {
            self.locked = Some(addr);
        }
    }
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;

    #[test]
    fn locks_on_the_first_confirmed_pickup() {
        let mut l = PlayerLock::new(0x1000);
        // Word 0 (addr 0x1000) holds the avatar's address, 0xABCD; word 1 holds
        // something unrelated.
        l.observe_pickup(&[0xABCD, 7], 0xABCD);
        assert_eq!(l.locked(), Some(0x1000), "a single exact-value match is enough to lock");
    }

    #[test]
    fn a_turn_with_no_matching_word_teaches_nothing() {
        let mut l = PlayerLock::new(0x1000);
        l.observe_pickup(&[0x1111, 0x2222], 0xABCD);
        assert_eq!(l.locked(), None, "no word in the window holds this truth, so there is nothing to lock onto");
    }

    #[test]
    fn two_aliases_take_the_lower_address() {
        // Inform keeps `player` and `actor` (or similar) side by side, both
        // holding the avatar — the same alias shape RoomLock documents for
        // `location`/`real_location`. Either identifies the avatar, so the
        // tie-break is deterministic rather than clever.
        let mut l = PlayerLock::new(0x1000);
        l.observe_pickup(&[0xABCD, 0xABCD, 7], 0xABCD);
        assert_eq!(l.locked(), Some(0x1000), "the lower of the two agreeing words");
    }

    #[test]
    fn a_lock_survives_a_confirming_pickup() {
        let mut l = PlayerLock::locked_at(0x1000, 0x1004);
        l.observe_pickup(&[0, 0xABCD, 7], 0xABCD);
        assert_eq!(l.locked(), Some(0x1004), "the locked word still holds the truth — nothing to do");
    }

    #[test]
    fn a_contradicting_pickup_rejects_the_lock_and_relocks_the_same_turn() {
        // The locked word (0x1004) no longer holds this turn's truth, but a
        // DIFFERENT word (0x1008) does — the same turn's evidence both catches
        // the stale lock out and re-establishes a fresh one, without waiting
        // for a second turn.
        let mut l = PlayerLock::locked_at(0x1000, 0x1004);
        l.observe_pickup(&[0, 0x9999, 0xABCD], 0xABCD);
        assert_eq!(l.locked(), Some(0x1008), "the story contradicted the old lock; a fresh one took its place");
    }

    #[test]
    fn a_rejected_word_is_never_locked_on_again() {
        let mut l = PlayerLock::locked_at(0x1000, 0x1004);
        // Contradicted: 0x1004 no longer holds the truth, and no replacement is
        // offered this turn either.
        l.observe_pickup(&[0, 0x9999, 0], 0xABCD);
        assert_eq!(l.locked(), None, "contradicted with nothing to replace it");

        // A LATER turn's evidence would naively re-elect 0x1004 by the same
        // tie-break that chose it originally — the rejection list must refuse it.
        l.observe_pickup(&[0, 0xABCD, 0xABCD], 0xABCD);
        assert_eq!(l.locked(), Some(0x1008), "the rejected address is skipped; the next alias wins");
    }

    #[test]
    fn a_lock_below_the_scan_base_is_rejected_rather_than_underflowing() {
        // SQ-0658's guard on `RoomLock` applies here too: the locked address is
        // not always something this learner chose — it is also read back from
        // a sidecar a user can edit and that can outlive a story rebuild.
        let mut l = PlayerLock::locked_at(0x1000, 0x1000 - 0x400);
        // The out-of-window lock can never be verified against a snapshot, so
        // `word_index` must refuse it rather than underflow — and the learner
        // then treats it as contradicted (nothing to confirm it with) and looks
        // for a fresh candidate in the same turn's evidence.
        l.observe_pickup(&[0xABCD, 7], 0xABCD);
        assert_eq!(l.locked(), Some(0x1000), "the unverifiable lock was dropped and a fresh one taken");
    }

    #[test]
    fn identity_change_is_learnable_like_any_other_contradiction() {
        // The avatar legitimately changing body (Counterfeit Monkey's own plot)
        // is not a special case: it is exactly the shape of any other
        // contradiction, and the learner relocks onto whatever the story says
        // now holds true.
        let mut l = PlayerLock::locked_at(0x1000, 0x1004);
        l.observe_pickup(&[0, 0x1111, 7], 0x1111); // still the OLD avatar, confirmed
        assert_eq!(l.locked(), Some(0x1004));
        l.observe_pickup(&[0, 0x1111, 0x2222], 0x2222); // a pickup now proves a NEW avatar
        assert_eq!(l.locked(), Some(0x1008), "relocked onto the word that now holds the new avatar");
    }
}
