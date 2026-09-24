//! Shadow-probe answer routing (SQ-1548): collect one answer from the shared
//! shadow [`crate::probe::ShadowProbe`] and hand it to whichever consumer
//! asked for it — the vetted vocabulary "try instead" offer
//! ([`crate::vocab`]), a return-probe map edge ([`crate::return_probe`]), or a
//! random-exit map edge ([`crate::random_exit_probe`]).
//!
//! Moved out of the binary's `loop_tick::poll_shadow_answers` (SQ-0785,
//! SQ-1124, SQ-1257), which is now a thin call into [`poll`], so a host that is
//! not a terminal collects these answers too. Without this, a headless host
//! that only calls [`super::turn::finish_command_turn`] arms the probes
//! (`host::turn`'s own `offer_vocabulary` / `arm_return_search` /
//! `arm_for_finished_turn` calls) but never learns what they answered: a
//! vetted offer sits in `state.vocab_pending` forever, and a return-probe or
//! random-exit discovery never reaches the [`Mapper`].
//!
//! **One collector, because there is one channel.** `ShadowProbe` serves three
//! consumers and hands back whatever has arrived with no idea who wanted it —
//! see [`poll`]'s own docs for why they cannot each poll for themselves.
//!
//! **Not time-driven.** [`crate::probe::ShadowProbe::poll`] is a nonblocking
//! channel read, not a deadline: there is no wall-clock moment an answer
//! becomes due, so there is nothing here to fold into
//! [`super::clock::next_deadline`]. The TUI collects an answer on its ordinary
//! loop cadence (a poll timeout unrelated to any game clock, see
//! `main.rs`'s event loop); a headless host does the same by calling [`poll`]
//! on whatever cadence it already ticks on.

use mapper::mapper::Mapper;

use crate::state::AppState;

/// Collect one answer from the shared shadow and give it to whoever asked for
/// it, then let the return search hand out its next question (SQ-0785).
///
/// **One collector, because there is one channel.** [`crate::probe::ShadowProbe`]
/// serves three consumers — the vocabulary offer, the return search and the
/// random-exit search — and [`crate::probe::ShadowProbe::poll`] takes whatever
/// has arrived without knowing who wanted it. Three consumers each polling for
/// themselves would mean the first one to look takes another's answer off the
/// channel and drops it, silently and only sometimes. So the answer is
/// collected here, once, and routed by the token it carries.
///
/// The pump runs after the route, so a search whose attempt has just come back
/// asks its next question on the same pass rather than idling a tick per
/// direction.
///
/// Returns `true` when anything changed — an assist line shown, or a map edge
/// minted or removed — which is the caller's redraw/notify contribution.
pub fn poll(state: &mut AppState, mapper: &mut Mapper, bg_tidy_counter: &mut u32) -> bool {
    let mut changed = false;
    if let Some(answer) = state.probe.poll() {
        if crate::vocab::owns(state, answer.token) {
            changed |= crate::vocab::deliver_answer(state, answer);
        } else if crate::return_probe::owns(state, answer.token)
            && crate::return_probe::deliver(state, mapper, &answer).is_some()
        {
            // A new passage is a geometry change, so it gets everything a walked
            // one gets: the layout rescheduled and a redraw. Minting the edge already
            // bumped `Mapper::struct_gen` (SQ-1544), which invalidates the render memo
            // on its own — an edge nobody lays out or draws is a discovery the player
            // never sees.
            super::turn::schedule_map_maintenance(state, mapper, false, true, bg_tidy_counter);
            changed = true;
        } else if crate::random_exit_probe::owns(state, answer.token)
            && crate::random_exit_probe::deliver(state, mapper, &answer)
        {
            // SQ-1257 Phase 2: an edge was just DELETED (a random exit confirmed), which is a
            // geometry change exactly like a new one — `struct_gen` already bumped for it
            // (SQ-1544), so the render memo and any in-flight tidy will not go on describing
            // the edge that is now gone.
            super::turn::schedule_map_maintenance(state, mapper, false, true, bg_tidy_counter);
            changed = true;
        }
        // An answer nobody owns is one whose asker has moved on — an aborted
        // search, or a vocabulary offer the player typed past. Dropping it is the
        // silence discipline all three consumers already have.
    }
    crate::return_probe::pump_return_search(state);
    changed
}
