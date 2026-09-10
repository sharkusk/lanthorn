//! ScottFree 1.14's four runtime option flags — `-y`/`-s`/`-t`/`-p`, named
//! `YOUARE`/`SCOTTLIGHT`/`TRS80_STYLE`/`PREHISTORIC_LAMP` in ScottFree's own
//! usage message — held as a value on [`crate::Vm`] rather than a
//! process-global — this crate takes no I/O policy, so there is nowhere for
//! a global to live, and a host embedding several sessions (a library
//! browser, a test harness looping over fixtures) needs each session able to
//! choose independently anyway.
//!
//! [`Options::default`] matches ScottFree's own default — every flag off,
//! i.e. exactly what running ScottFree with none of `-y -s -t -p` gives you —
//! so a host that never touches this type gets the plain build's behaviour.
//!
//! [`Wording`] gathers every reply string these flags select between into one
//! table (`Wording::for_options`), rather than leaving the choice scattered
//! across [`crate::vm`]'s message sites — see its own docs for why, and for
//! the full inventory of what changes.

/// How the current room's exits/items block is laid out — the part of
/// ScottFree's own observed `Look()` behaviour that
/// `-t`/[`Options::trs80_style`] switches, plus this crate's own
/// pre-existing layout.
///
/// This is deliberately narrower than the full `TRS80_STYLE` flag: ScottFree's
/// `-t` ALSO changes the inventory command's item separator (case 66) and
/// the terminal geometry — neither of which this crate models, since
/// this crate has no terminal of its own and the inventory separator is a
/// cosmetic detail no `.dat` file depends on. `Presentation` covers exactly
/// the one thing [`crate::Vm::room_block`] renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Presentation {
    /// ScottFree's own plain layout: exits joined `", "`, a trailing period,
    /// items joined `" - "`, no rule.
    ScottFree,
    /// ScottFree's `-t`/`TRS80_STYLE` layout: items each followed by `". "`
    /// (no `" - "` separator) and the whole block framed by the TRS-80's
    /// `<------>` rule.
    Trs80,
    /// lanthorn's own pre-existing layout (exits joined `". "`, items each on
    /// their own indented line) — a THIRD presentation, matching neither of
    /// ScottFree's. This is the crate's own default, kept as the default so
    /// existing embedders (lanthorn's own `scott_session.rs`, this crate's
    /// pinned golden test) see no change from adding [`Options`].
    C64,
    /// The **TI-99/4A** releases' own layout and message set
    /// (`docs/internals/scott-dialects-spec.md` §9.1): `I am in a ` as the
    /// room prefix, `Obvious exits : ` and `Visible items are : ` as the two
    /// headers, `", "` as the delimiter for both lists, and the room
    /// description terminated with a period when any items are visible.
    ///
    /// Not a host choice: the specification's Appendix A states that the
    /// runtime differences of its §9 "are properties of the *database*, not
    /// of the host, and have to travel with it", so [`crate::Vm`] forces
    /// this presentation — and the two lamp flags of §9.2 — for any
    /// [`crate::Database`] loaded from that dialect, whatever the host asked
    /// for.
    Ti994a,
}

impl Default for Presentation {
    /// This crate's own pre-existing room-block layout — see [`Presentation::C64`].
    /// Not ScottFree's default; ScottFree has no `Presentation` concept at all
    /// (its layout is fixed to [`Presentation::ScottFree`] unless `-t` is given).
    /// A host that wants ScottFree's own choice sets [`Presentation::ScottFree`]
    /// explicitly via [`Options::with_presentation`].
    fn default() -> Self {
        Presentation::C64
    }
}

/// ScottFree 1.14's four runtime option flags, plus [`Presentation`] (this
/// crate's own addition — ScottFree has no separate concept, see there).
/// `#[non_exhaustive]` and built through the `with_*` methods, so a fifth
/// flag can be added later without breaking a caller's struct literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Options {
    /// `-y`/`YOUARE`, pinned by
    /// `you_are_option_swaps_death_and_inventory_wording` in
    /// scottfree_parity.rs: second-person replies ("You are dead.", "You
    /// are carrying:") in place of ScottFree's default
    /// first-person ones ("I am dead.", "I'm carrying:"). See [`Wording`] for
    /// every string this swaps.
    ///
    /// **Robin of Sherwood needs this to read correctly** — its prose was
    /// authored assuming the `-y` build, per the reference audit (SQ-1014
    /// note, 2026-09-08) and ScottFree's own convention of shipping some
    /// Brian Howarth titles with `-y` in their launch script.
    pub you_are: bool,
    /// `-s`/`SCOTTLIGHT`, pinned by
    /// `scott_light_option_shows_a_running_countdown_instead_of_growing_dim`
    /// in scottfree_parity.rs: the original Adams lamp countdown wording —
    /// "Light runs out in N turns." every turn under 25, "Light has run
    /// out! " at zero — replacing ScottFree's own embellished "Your light
    /// is growing dim." (only every 5th turn) / "Your light has run out."
    pub scott_light: bool,
    /// `-t`/`TRS80_STYLE`: the TRS-80 room-block layout — see
    /// [`Presentation::Trs80`]. Setting this to `true` alone does NOT change
    /// [`Options::presentation`]; a host wanting the TRS-80 layout sets
    /// `presentation` explicitly (see [`Options::with_trs80_style`]'s doc
    /// for why the two are kept separate).
    pub trs80_style: bool,
    /// `-p`/`PREHISTORIC_LAMP`, pinned by
    /// `prehistoric_lamp_option_destroys_the_light_source_on_run_out` in
    /// scottfree_parity.rs: the light source is destroyed the instant its
    /// fuel reaches zero, rather than merely going dark and staying an
    /// inert carried item — ScottFree's original Adams-era behaviour,
    /// restored for the games that were tuned against it.
    pub prehistoric_lamp: bool,
    /// The room-block layout — see [`Presentation`]. Not one of ScottFree's
    /// four `-y`/`-s`/`-t`/`-p` flags (this crate's own addition, see there).
    pub presentation: Presentation,
}

impl Options {
    /// ScottFree's own defaults: every flag off, [`Presentation::C64`] — see
    /// the type docs for why the presentation default differs from ScottFree's.
    pub fn new() -> Self {
        Self::default()
    }
    /// Sets [`Options::you_are`] — see its doc for the replies this swaps to
    /// second person, and which game needs it.
    pub fn with_you_are(mut self, v: bool) -> Self {
        self.you_are = v;
        self
    }
    /// Sets [`Options::scott_light`] — see its doc for the lamp-countdown
    /// wording this switches to.
    pub fn with_scott_light(mut self, v: bool) -> Self {
        self.scott_light = v;
        self
    }
    /// Sets [`Options::trs80_style`] only — see its doc for why
    /// [`Options::presentation`] is a separate knob.
    pub fn with_trs80_style(mut self, v: bool) -> Self {
        self.trs80_style = v;
        self
    }
    /// Sets [`Options::prehistoric_lamp`] — see its doc for how this changes
    /// what happens the instant the lamp's fuel reaches zero.
    pub fn with_prehistoric_lamp(mut self, v: bool) -> Self {
        self.prehistoric_lamp = v;
        self
    }
    /// Sets [`Options::presentation`] — see [`Presentation`] for the
    /// room-block layouts a host can choose between.
    pub fn with_presentation(mut self, p: Presentation) -> Self {
        self.presentation = p;
        self
    }
}

/// Every player-facing reply string ScottFree's `-y`/[`Options::you_are`]
/// switches between, gathered in one place (`Wording::for_options`) rather
/// than scattered across the `vm` module's message sites — so a host (or a
/// reviewer) can see the complete set of what changes, and a future flag
/// extends one function instead of hunting through `run_turn`/`run_commands`.
///
/// Every field names the parity/golden test that pins it, or notes where
/// none exists. Two fields
/// ([`Wording::light_dim`]/[`Wording::light_out`] vs
/// [`Wording::light_runs_out_prefix`]/[`suffix`](Wording::light_runs_out_suffix))
/// are additionally gated on [`Options::scott_light`], not `you_are` — the
/// lamp countdown is the one place ScottFree lets the two flags combine, so
/// [`Wording::for_options`] reads both.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Wording {
    /// GET/DROP success, pinned by `golden_transcript` in golden.rs:
    /// `"O.K. "` — always this, regardless of `you_are` (ScottFree's
    /// single-item GET/DROP path never
    /// branches on `YOUARE`). The TI-99/4A set spells it `"OK. "`, and uses
    /// the SAME string for taking, dropping and generic acknowledgement —
    /// that dialect does not distinguish "Taken." from "Dropped."
    /// (spec §9.1).
    pub ok: &'static str,
    /// Printed before the new room is described after a successful compass
    /// move. Empty for every ScottFree-derived set, which acknowledges a
    /// move only by describing where you now are; `"OK. "` for the TI-99/4A
    /// set, where "movement is acknowledged" is an observable difference
    /// spec §9.1 gives a test for.
    pub move_ok: &'static str,
    /// GET ALL/DROP ALL per-item success suffix, pinned by
    /// `get_all_runs_each_items_own_get_action_then_takes_it_and_skips_star_marked_items`
    /// in scottfree_parity.rs: `": O.K.\n"` after the item's own text —
    /// also unconditional.
    pub ok_all_suffix: &'static str,
    /// Inventory header (case 66), pinned by
    /// `you_are_option_swaps_death_and_inventory_wording` in
    /// scottfree_parity.rs: `"I'm carrying:\n"` / `"You are carrying:\n"`.
    pub carrying_header: &'static str,
    /// Inventory empty-pack line (case 66), pinned by
    /// `you_are_option_swaps_death_and_inventory_wording` in
    /// scottfree_parity.rs: `"Nothing"` —
    /// always this text; only the trailing `".\n"` (unconditional, appended
    /// by the caller) differs from a populated list, which gets the same
    /// suffix after its last item.
    pub nothing_carried: &'static str,
    /// Inventory item separator (case 66, non-TRS80 branch) — ScottFree's
    /// own observed wording (no covering parity/golden case found): `" - "`
    /// — always this (TRS80's `". "` alternative is not modelled, see
    /// [`Presentation`]'s doc).
    pub carrying_sep: &'static str,
    /// SCORE's stored-treasure count prefix (case 65), pinned by
    /// `golden_transcript` in golden.rs: `"I've stored "` / `"You have
    /// stored "`.
    pub stored_prefix: &'static str,
    /// GO with no/unknown noun, pinned by
    /// `bare_go_asks_for_a_direction_before_the_action_table` in
    /// scottfree_parity.rs: `"Give me a direction too."` — unconditional
    /// (no `you_are` variant).
    pub direction_needed: &'static str,
    /// GET/DROP ALL with an unmatched noun some other command produced,
    /// pinned by `get_with_unknown_noun_asks_what` in scottfree_parity.rs:
    /// `"What ? "` — unconditional.
    pub what: &'static str,
    /// `Look()`'s darkness line — ScottFree's own observed wording (no
    /// covering parity/golden case found): `"I can't see. It is too
    /// dark!\n"` / `"You can't see. It is too dark!\n"`.
    /// Read only under [`Presentation::ScottFree`]/[`Presentation::Trs80`] —
    /// [`Presentation::C64`] keeps its own unconditional wording (see
    /// [`crate::Vm::room_block`]).
    pub too_dark_to_see: &'static str,
    /// `Look()`'s "also see" header, pinned by
    /// `presentation_option_selects_room_block_layout` in
    /// scottfree_parity.rs: `"\nI can also see: "` /
    /// `"\nYou can also see: "`. Same
    /// [`Presentation`] scoping as [`Wording::too_dark_to_see`].
    pub see_also_header: &'static str,
    /// `Look()`'s room-description prefix: `"I'm in a "` / `"You are in a "`.
    /// Same [`Presentation`] scoping.
    ///
    /// **The second-person form was `"You are "` until SQ-1478**, which is
    /// the one string in this table no parity or golden case covered — and
    /// the one that is wrong, because every Scott Adams room text in every
    /// dialect is written as the continuation of "in a": *The Golden Baton*'s
    /// room 1 is `dense forest, very SPOOKY`, so the old form printed `You
    /// are dense forest, very SPOOKY`. `docs/internals/scott-dialects-spec.md`
    /// §6.4 tabulates the two message sets and gives `I'm in a ` against `You
    /// are in a `; the first-person form here already matched that column
    /// exactly and the second-person one now does too.
    pub room_prefix: &'static str,
    /// `main`'s unmatched-return fallback, pinned by
    /// `matched_but_blocked_action_replies_cant_do_that_yet_not_dont_understand`
    /// in scottfree_parity.rs: `"I don't understand your command. "` —
    /// unconditional.
    pub dont_understand: &'static str,
    /// `main`'s "matched but blocked" fallback, pinned by
    /// `matched_but_blocked_action_replies_cant_do_that_yet_not_dont_understand`
    /// in scottfree_parity.rs: `"I can't do that yet. "` — unconditional,
    /// and the whole point of
    /// SQ-1413 item 3 (`PerformActions`'s `-2` return, previously collapsed
    /// into [`Wording::dont_understand`]).
    pub cant_do_that_yet: &'static str,
    /// GET ALL, nothing with an auto-get word present, pinned by
    /// `get_all_and_drop_all_report_nothing_with_scottfrees_exact_punctuation`
    /// in scottfree_parity.rs: `"Nothing taken."` — no trailing newline,
    /// unconditional.
    pub nothing_taken: &'static str,
    /// DROP ALL, nothing carried with an auto-get word, pinned by
    /// `get_all_and_drop_all_report_nothing_with_scottfrees_exact_punctuation`
    /// in scottfree_parity.rs: `"Nothing dropped.\n"` — WITH a trailing
    /// newline (unlike
    /// [`Wording::nothing_taken`]), unconditional.
    pub nothing_dropped: &'static str,
    /// GET ALL's darkness short-circuit, pinned by
    /// `get_all_short_circuits_in_a_dark_room` in scottfree_parity.rs:
    /// `"It is dark.\n"` — unconditional; DROP ALL has no such check
    /// (dropping in the dark is always allowed).
    pub it_is_dark: &'static str,
    /// GET (single item)/GET ALL's "pack is full" refusal, PERIOD-terminated
    /// — ScottFree's own observed wording (no covering parity/golden case
    /// found): `"I've too much to carry. "` / `"You are carrying too
    /// much. "`. Distinct from
    /// [`Wording::too_much_bang`] — ScottFree's own two capacity-refusal call
    /// sites disagree on punctuation and this crate matches both faithfully.
    pub too_much_period: &'static str,
    /// Opcode 52's "pack is full" refusal, BANG-terminated for the plain
    /// wording only — ScottFree's own observed wording, pinned (as far as
    /// the `too much` substring) by
    /// `cmd_get_op52_refuses_only_at_exact_capacity_not_over` in vm.rs's
    /// tests: `"I've too much to carry! "` / `"You are
    /// carrying too much. "` — note the `you_are` variant is
    /// IDENTICAL text to [`Wording::too_much_period`]'s `you_are` variant;
    /// only the plain (`!you_are`) wording differs by punctuation.
    pub too_much_bang: &'static str,
    /// Opcode 61's death message, pinned by
    /// `you_are_option_swaps_death_and_inventory_wording` in
    /// scottfree_parity.rs: `"I am dead.\n"` / `"You are dead.\n"`.
    pub dead: &'static str,
    /// GET's "no such item here" refusal, pinned by
    /// `get_when_no_twin_is_in_the_room_is_beyond_my_power` in
    /// scottfree_parity.rs: `"It's beyond my
    /// power to do that. "` / `"It is beyond your power to do
    /// that. "` — note ScottFree drops the apostrophe only in the `you_are`
    /// GET variant (`"It is"`, not `"It's"`); [`Wording::beyond_power_drop`]
    /// keeps the apostrophe in both variants. Trailing space, no newline.
    pub beyond_power_get: &'static str,
    /// DROP's "not carrying that" refusal — ScottFree's own observed
    /// wording (no covering parity/golden case found): `"It's beyond my
    /// power to do that.\n"` / `"It's beyond your power to do
    /// that.\n"` — BOTH variants keep the apostrophe (unlike
    /// [`Wording::beyond_power_get`]'s `you_are` form), and both end with a
    /// newline rather than a trailing space.
    pub beyond_power_drop: &'static str,
    /// Movement with an exit, pinned (as far as the `can't go` substring)
    /// by `out_of_range_exit_is_treated_as_no_exit` in scottfree_parity.rs:
    /// `"I can't go in that direction. "` / `"You can't go in that
    /// direction. "`.
    pub cant_go_that_direction: &'static str,
    /// Movement into darkness with no exit, pinned by
    /// `death_in_the_dark_matches_scottfree_wording_and_ends_the_game` in
    /// scottfree_parity.rs: `"I fell down and broke my neck. "` / `"You
    /// fell down and broke your neck. "`.
    pub fell_and_broke_neck: &'static str,
    /// Movement while dark, printed whether or not the move then succeeds,
    /// pinned by `death_in_the_dark_matches_scottfree_wording_and_ends_the_game`
    /// in scottfree_parity.rs: `"Dangerous to move in the dark! "` —
    /// unconditional (no `you_are` variant).
    pub dangerous_in_dark: &'static str,
    /// Opcode 63's `doneit` label, pinned by `golden_transcript` in
    /// golden.rs ("The game is now over."), also reached via opcode 65's
    /// win check: `"The game is now over.\n"` — unconditional.
    pub game_now_over: &'static str,
    /// Opcode 65's win line, printed just before falling into
    /// [`Wording::game_now_over`], pinned by
    /// `op65_win_prints_well_done_and_ends_the_game` in scottfree_parity.rs:
    /// `"Well done.\n"` — unconditional.
    pub well_done: &'static str,
    /// `GetInput`'s unknown-verb reply, pinned by
    /// `unknown_first_word_with_direction_second_word_does_not_move` in
    /// scottfree_parity.rs: `"You use word(s) I don't know! "` —
    /// unconditional.
    pub unknown_words: &'static str,
    /// Lamp run-out, pinned by
    /// `lamp_countdown_dims_once_and_runs_out_exactly_twice` in
    /// scottfree_parity.rs, gated on [`Options::scott_light`] rather than
    /// `you_are`: `"Light has run out! "` (scott_light) / `"Your light has
    /// run out. "` (default).
    pub light_out: &'static str,
    /// Lamp low-fuel warning under the DEFAULT (non-`scott_light`) wording,
    /// pinned by `lamp_countdown_dims_once_and_runs_out_exactly_twice` in
    /// scottfree_parity.rs, shown only every 5th turn: `"Your light is
    /// growing dim. "`.
    /// Empty under `scott_light` — that
    /// variant uses [`Wording::light_runs_out_prefix`]/[`suffix`](Wording::light_runs_out_suffix)
    /// instead, shown every turn.
    pub light_dim: &'static str,
    /// Lamp low-fuel warning under `scott_light`, shown every turn under
    /// 25, pinned by
    /// `scott_light_option_shows_a_running_countdown_instead_of_growing_dim`
    /// in scottfree_parity.rs: `"Light runs out in "`, then the live fuel
    /// number, then [`Wording::light_runs_out_suffix`].
    pub light_runs_out_prefix: &'static str,
    /// See [`Wording::light_runs_out_prefix`]: `" turns. "`.
    pub light_runs_out_suffix: &'static str,
}

impl Wording {
    /// Resolves every string above from `options.you_are` and
    /// `options.scott_light` — the ONE place these two flags are read for
    /// message text, so a reviewer (or a future flag) has one function to
    /// check rather than a scatter of `if self.options.you_are` sites.
    pub fn for_options(options: &Options) -> Wording {
        let y = options.you_are;
        let s = options.scott_light;
        let base = Wording {
            ok: "O.K. ",
            move_ok: "",
            ok_all_suffix: ": O.K.\n",
            carrying_header: if y { "You are carrying:\n" } else { "I'm carrying:\n" },
            nothing_carried: "Nothing",
            carrying_sep: " - ",
            stored_prefix: if y { "You have stored " } else { "I've stored " },
            direction_needed: "Give me a direction too.",
            what: "What ? ",
            too_dark_to_see: if y {
                "You can't see. It is too dark!\n"
            } else {
                "I can't see. It is too dark!\n"
            },
            see_also_header: if y { "\nYou can also see: " } else { "\nI can also see: " },
            room_prefix: if y { "You are in a " } else { "I'm in a " },
            dont_understand: "I don't understand your command. ",
            cant_do_that_yet: "I can't do that yet. ",
            nothing_taken: "Nothing taken.",
            nothing_dropped: "Nothing dropped.\n",
            it_is_dark: "It is dark.\n",
            too_much_period: if y {
                "You are carrying too much. "
            } else {
                "I've too much to carry. "
            },
            too_much_bang: if y {
                "You are carrying too much. "
            } else {
                "I've too much to carry! "
            },
            dead: if y { "You are dead.\n" } else { "I am dead.\n" },
            beyond_power_get: if y {
                "It is beyond your power to do that. "
            } else {
                "It's beyond my power to do that. "
            },
            beyond_power_drop: if y {
                "It's beyond your power to do that.\n"
            } else {
                "It's beyond my power to do that.\n"
            },
            cant_go_that_direction: if y {
                "You can't go in that direction. "
            } else {
                "I can't go in that direction. "
            },
            fell_and_broke_neck: if y {
                "You fell down and broke your neck. "
            } else {
                "I fell down and broke my neck. "
            },
            dangerous_in_dark: "Dangerous to move in the dark! ",
            game_now_over: "The game is now over.\n",
            well_done: "Well done.\n",
            unknown_words: "You use word(s) I don't know! ",
            light_out: if s { "Light has run out! " } else { "Your light has run out. " },
            light_dim: if s { "" } else { "Your light is growing dim. " },
            light_runs_out_prefix: if s { "Light runs out in " } else { "" },
            light_runs_out_suffix: if s { " turns. " } else { "" },
        };
        if options.presentation != Presentation::Ti994a {
            return base;
        }
        // The TI-99/4A releases carry their OWN message set, distinct from
        // both the reference and the ZX Spectrum sets
        // (`docs/internals/scott-dialects-spec.md` §9.1). Every string below
        // is one that section pins literally; every field left at `base` is
        // one it does not mention, and keeping the first-person default
        // there is an honest "unknown" rather than an invented string. Two
        // of this dialect's strings belong to the host and not to this
        // crate, which has no prompt of its own: `What shall I do? ` and
        // `Resume a saved game? `.
        Wording {
            ok: "OK. ",
            move_ok: "OK. ",
            room_prefix: "I am in a ",
            see_also_header: "\nVisible items are : ",
            carrying_header: "I am carrying : ",
            carrying_sep: ", ",
            dont_understand: "I don't understand the command. ",
            cant_do_that_yet: "I can't do that yet. ",
            beyond_power_get: "It is beyond my power to do that. ",
            beyond_power_drop: "It is beyond my power to do that. ",
            fell_and_broke_neck: "\nI fell down and broke my neck.",
            dead: "I'm dead... ",
            too_much_period: "I am carrying too much.",
            too_much_bang: "I am carrying too much.",
            light_out: "Light went out! ",
            // Read only when `scott_light` is off, which for a real TI-99/4A
            // database it never is (spec §9.2 forces it on); carried anyway
            // because §9.1 lists it as part of the set.
            light_dim: "Light is growing dim ",
            game_now_over: "This adventure is over. Play again?",
            ..base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_are_scottfrees_own_defaults() {
        let o = Options::default();
        assert!(!o.you_are);
        assert!(!o.scott_light);
        assert!(!o.trs80_style);
        assert!(!o.prehistoric_lamp);
        assert_eq!(o.presentation, Presentation::C64);
    }

    #[test]
    fn builder_sets_one_flag_at_a_time() {
        let o = Options::new().with_you_are(true);
        assert!(o.you_are);
        assert!(!o.scott_light);
        let o = Options::new().with_presentation(Presentation::ScottFree);
        assert_eq!(o.presentation, Presentation::ScottFree);
    }

    #[test]
    fn wording_default_matches_scottfrees_plain_build() {
        let w = Wording::for_options(&Options::default());
        assert_eq!(w.dead, "I am dead.\n");
        assert_eq!(w.carrying_header, "I'm carrying:\n");
        assert_eq!(w.light_out, "Your light has run out. ");
        assert_eq!(w.light_dim, "Your light is growing dim. ");
    }

    #[test]
    fn wording_you_are_swaps_person() {
        let w = Wording::for_options(&Options::new().with_you_are(true));
        assert_eq!(w.dead, "You are dead.\n");
        assert_eq!(w.carrying_header, "You are carrying:\n");
        assert_eq!(w.beyond_power_get, "It is beyond your power to do that. ");
        assert_eq!(w.beyond_power_drop, "It's beyond your power to do that.\n");
    }

    #[test]
    fn wording_scott_light_swaps_lamp_wording_independent_of_you_are() {
        let w = Wording::for_options(&Options::new().with_scott_light(true));
        assert_eq!(w.light_out, "Light has run out! ");
        assert_eq!(w.light_runs_out_prefix, "Light runs out in ");
        assert_eq!(w.light_runs_out_suffix, " turns. ");
        assert_eq!(w.light_dim, "");
    }
}
