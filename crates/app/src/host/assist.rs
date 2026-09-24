//! Input-help session rules a non-terminal host needs too (SQ-1549): the
//! Guiding Light's per-game switch, and the command band's data.
//!
//! Completion (ranking, apply, the ghost-text tail) and the reveal's
//! text-in/words-out variant live beside the code they are pure twins of —
//! [`crate::complete`] and [`crate::reveal::arm_from_text`] — rather than
//! here, since neither needs an `AppState` write of its own. What is here is
//! genuinely host-shaped: a persisted setting, and a live-object refresh, the
//! same two things [`super::persist`] and [`super::turn`] already are for
//! saves and turns.

use crate::state::AppState;

/// Set the Guiding Light's per-game override exactly as `/set-guidance`
/// does: persist `arg` to the game's sidecar (`GuidanceArg::Auto` clears the
/// override), update `state.config.guidance` to the effective value —
/// `arg`'s bool when it names one, else `state.guidance_base` (the global
/// default captured at boot) — and pin/release the one-run override so a
/// per-game choice never escapes into the user's global `config.toml`.
/// Returns the effective value.
///
/// `game_dir` is the story's save directory (`AppState::game_dir`); an empty
/// path is a no-op sidecar write and the effective value still comes back
/// correctly (matches every other per-game setting's "no game_dir → no
/// sidecar" rule, used by unit tests to stay off the filesystem).
pub fn set_guidance(
    state: &mut AppState,
    game_dir: &std::path::Path,
    arg: crate::slash::GuidanceArg,
) -> std::io::Result<bool> {
    use crate::slash::GuidanceArg;
    let want = match arg {
        GuidanceArg::On => Some(true),
        GuidanceArg::Off => Some(false),
        GuidanceArg::Auto => None,
        GuidanceArg::Toggle => Some(!state.config.guidance),
    };
    crate::styles::write_per_game_guidance(game_dir, want)?;
    state.config.guidance = want.unwrap_or(state.guidance_base);
    match want {
        Some(v) => state.config.one_run.pin(crate::config::keys::GUIDANCE, v),
        None => state.config.one_run.release(crate::config::keys::GUIDANCE),
    }
    Ok(state.config.guidance)
}

/// The command band's data, refreshed from the engine, as plain values
/// (SQ-1549) — no cell rects, no animation state, no picker/scroll/focus, the
/// three things [`crate::render::command_band::CommandBandState`] carries
/// beyond what a host actually needs to draw its own command help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandData {
    /// The VERB column: every verb this story's grammar (or the built-in/
    /// configured fallback) offers, with the sentence shapes it accepts.
    ///
    /// Ranked for a player (SQ-1554): [`VerbEntry::tier`] says which rows a
    /// host shows up front (`Core`, then `Story`) and which go behind its own
    /// "More…" (`More`); the list is already in that order. One row per verb,
    /// its other spellings in [`VerbEntry::synonyms`].
    ///
    /// [`VerbEntry::tier`]: crate::render::command_band::VerbEntry::tier
    /// [`VerbEntry::synonyms`]: crate::render::command_band::VerbEntry::synonyms
    pub verbs: Vec<crate::render::command_band::VerbEntry>,
    /// Where [`Self::verbs`] came from — the story's own grammar, the
    /// built-in fallback, or the player's `[command_panel] verbs`.
    pub verb_source: crate::render::command_band::VerbSource,
    /// The one-click quick-action row (compass directions, look, inventory,
    /// …), as the plain word list — a host lays out its own compass/flow from
    /// these; this is not the TUI's resolved rose-block geometry.
    pub quick: Vec<String>,
    /// Objects the object tree says are in the current room.
    pub here: Vec<String>,
    /// Objects the player carries.
    pub carried: Vec<String>,
    /// Words the story has PRINTED that name a thing, newest first — the
    /// weaker second tier under `here`/`carried` (SQ-1135).
    pub here_seen: Vec<String>,
    /// What [`Self::here`]'s rows actually are, and so what a host's own
    /// header may honestly claim.
    pub here_source: crate::state::HereSource,
    /// The second-object ("…WITH…") slot's candidates: carried first, then
    /// `here`, then `here_seen`, deduplicated across all three — the same
    /// tiers `here` itself offers.
    pub preposition_objects: Vec<String>,
}

/// Refresh the command band's verbs and live objects from `session` and hand
/// back the result as plain values (SQ-1549): the read side of
/// [`crate::render::command_band::refresh_verbs`] +
/// [`crate::render::command_band::refresh_objects`], which this calls to do
/// the actual refreshing — the same two functions `loop_tick::
/// refresh_command_band` calls for the TUI's own band, so a host never sees a
/// different answer than the TUI would for the same turn.
///
/// Both refreshers work only on an OPEN `state.overlays.command_band`, which
/// a host driving `AppState` directly has no picker/overlay concept to open
/// through — so this opens one exactly as `input::open_command_band` does
/// (the same initial verb table and quick row) when none is already open,
/// reads the refreshed data back, and — if it opened one — closes it again
/// before returning, so a host that never otherwise uses the TUI's overlay
/// leaves no trace of having asked. A TUI that already has the band open
/// keeps it open and gets the benefit of `refresh_verbs`'/`refresh_objects`'s
/// own once-per-open / once-per-turn caching, exactly as today.
pub fn refresh_band_data(state: &mut AppState, session: &dyn crate::engine::Engine) -> BandData {
    let had_band = state.overlays.command_band.is_some();
    if !had_band {
        let (table, _warnings) = state.config.resolve_band_verbs();
        let quick = state.config.command_band.resolve_quick_for(&state.game_dir);
        let mut band = crate::state::CommandBandState::new(table, quick);
        band.sync_from_input(&state.input.value);
        state.overlays.command_band = Some(band);
    }
    crate::render::command_band::refresh_verbs(state, session);
    crate::render::command_band::refresh_objects(state, session);
    let band = state.overlays.command_band.as_ref().expect("just ensured above");
    let data = BandData {
        verbs: band.verbs.clone(),
        verb_source: band.verb_source,
        quick: band.quick.clone(),
        here: band.here.clone(),
        carried: band.carried.clone(),
        here_seen: band.here_seen.clone(),
        here_source: band.here_source,
        preposition_objects: band.items(crate::render::command_band::COL_SECOND),
    };
    if !had_band {
        state.overlays.command_band = None;
    }
    data
}
