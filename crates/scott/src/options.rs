//! ScottFree 1.14's four runtime option flags (`ScottCurses.c:1299-1342`,
//! `main`'s `-y`/`-s`/`-t`/`-p` switches), ported as a value on [`crate::Vm`]
//! rather than a process-global — this crate takes no I/O policy, so there is
//! nowhere for a global to live, and a host embedding several sessions (a
//! library browser, a test harness looping over fixtures) needs each session
//! able to choose independently anyway.
//!
//! ```c
//! case 'y': Options|=YOUARE; break;
//! case 's': Options|=SCOTTLIGHT; break;
//! case 't': Options|=TRS80_STYLE; break;
//! case 'p': Options|=PREHISTORIC_LAMP; break;
//! ```
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
/// ScottFree's `Look()` (`ScottCurses.c:436-528`) that `-t`/[`Options::trs80_style`]
/// switches, plus this crate's own pre-existing layout.
///
/// This is deliberately narrower than the full `TRS80_STYLE` flag: ScottFree's
/// `-t` ALSO changes the inventory command's item separator (case 66,
/// `ScottCurses.c:925-953`) and the terminal geometry (`main`,
/// `ScottCurses.c:1359-1370`) — neither of which this crate models, since
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
    /// `<------>` rule (`TRS80_LINE`, `ScottCurses.c:97`).
    Trs80,
    /// lanthorn's own pre-existing layout (exits joined `". "`, items each on
    /// their own indented line) — a THIRD presentation, matching neither of
    /// ScottFree's. This is the crate's own default, kept as the default so
    /// existing embedders (lanthorn's own `scott_session.rs`, this crate's
    /// pinned golden test) see no change from adding [`Options`].
    C64,
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
    /// `-y`/`YOUARE` (`ScottCurses.c:1310-1312`): second-person replies
    /// ("You are dead.", "You are carrying:") in place of ScottFree's default
    /// first-person ones ("I am dead.", "I'm carrying:"). See [`Wording`] for
    /// every string this swaps.
    ///
    /// **Robin of Sherwood needs this to read correctly** — its prose was
    /// authored assuming the `-y` build, per the reference audit (SQ-1014
    /// note, 2026-09-08) and ScottFree's own convention of shipping some
    /// Brian Howarth titles with `-y` in their launch script.
    pub you_are: bool,
    /// `-s`/`SCOTTLIGHT` (`ScottCurses.c:1319-1321`): the original Adams lamp
    /// countdown wording — "Light runs out in N turns." every turn under 25,
    /// "Light has run out! " at zero — replacing ScottFree's own embellished
    /// "Your light is growing dim." (only every 5th turn) / "Your light has
    /// run out." (`main`, `ScottCurses.c:1416-1452`).
    pub scott_light: bool,
    /// `-t`/`TRS80_STYLE` (`ScottCurses.c:1322-1324`): the TRS-80 room-block
    /// layout — see [`Presentation::Trs80`]. Setting this to `true` alone does
    /// NOT change [`Options::presentation`]; a host wanting the TRS-80 layout
    /// sets `presentation` explicitly (see [`Options::with_trs80_style`]'s doc
    /// for why the two are kept separate).
    pub trs80_style: bool,
    /// `-p`/`PREHISTORIC_LAMP` (`ScottCurses.c:1325-1327`): the light source
    /// is destroyed (`Location=DESTROYED`) the instant its fuel reaches zero,
    /// rather than merely going dark and staying an inert carried item —
    /// ScottFree's original Adams-era behaviour, restored for the games that
    /// were tuned against it (`main`, `ScottCurses.c:1430-1431`).
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
    pub fn with_you_are(mut self, v: bool) -> Self {
        self.you_are = v;
        self
    }
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
    pub fn with_prehistoric_lamp(mut self, v: bool) -> Self {
        self.prehistoric_lamp = v;
        self
    }
    pub fn with_presentation(mut self, p: Presentation) -> Self {
        self.presentation = p;
        self
    }
}

/// Every player-facing reply string ScottFree's `-y`/[`Options::you_are`]
/// switches between, gathered in one place (`Wording::for_options`) rather
/// than scattered across [`crate::vm`]'s message sites — so a host (or a
/// reviewer) can see the complete set of what changes, and a future flag
/// extends one function instead of hunting through `run_turn`/`run_commands`.
///
/// Every field cites the `ScottCurses.c` site it ports. Two fields
/// ([`Wording::light_dim`]/[`Wording::light_out`] vs
/// [`Wording::light_runs_out_prefix`]/[`suffix`](Wording::light_runs_out_suffix))
/// are additionally gated on [`Options::scott_light`], not `you_are` — the
/// lamp countdown is the one place ScottFree lets the two flags combine, so
/// [`Wording::for_options`] reads both.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Wording {
    /// GET/DROP success (`ScottCurses.c:1245,1290`): `"O.K. "` — always this,
    /// regardless of `you_are` (ScottFree's single-item GET/DROP path never
    /// branches on `YOUARE`).
    pub ok: &'static str,
    /// GET ALL/DROP ALL per-item success suffix (`ScottCurses.c:1213,1265`):
    /// `": O.K.\n"` after the item's own text — also unconditional.
    pub ok_all_suffix: &'static str,
    /// Inventory header (case 66, `ScottCurses.c:929-932`): `"I'm carrying:\n"`
    /// / `"You are carrying:\n"`.
    pub carrying_header: &'static str,
    /// Inventory empty-pack line (case 66, `ScottCurses.c:949-951`): `"Nothing"`
    /// — always this text; only the trailing `".\n"` (unconditional, appended
    /// by the caller) differs from a populated list, which gets the same
    /// suffix after its last item.
    pub nothing_carried: &'static str,
    /// Inventory item separator (case 66, non-TRS80 branch,
    /// `ScottCurses.c:941-942`): `" - "` — always this (TRS80's `". "`
    /// alternative is not modelled, see [`Presentation`]'s doc).
    pub carrying_sep: &'static str,
    /// SCORE's stored-treasure count prefix (case 65, `ScottCurses.c:910-913`):
    /// `"I've stored "` / `"You have stored "`.
    pub stored_prefix: &'static str,
    /// GO with no/unknown noun (`PerformActions`, `ScottCurses.c:1099-1102`):
    /// `"Give me a direction too."` — unconditional (no `you_are` branch in
    /// the source).
    pub direction_needed: &'static str,
    /// GET/DROP ALL with an unmatched noun some other command produced
    /// (`ScottCurses.c:1224,1277`): `"What ? "` — unconditional.
    pub what: &'static str,
    /// `Look()`'s darkness line (`ScottCurses.c:451-454`):
    /// `"I can't see. It is too dark!\n"` / `"You can't see. It is too dark!\n"`.
    /// Read only under [`Presentation::ScottFree`]/[`Presentation::Trs80`] —
    /// [`Presentation::C64`] keeps its own unconditional wording (see
    /// [`crate::Vm::room_block`]).
    pub too_dark_to_see: &'static str,
    /// `Look()`'s "also see" header (`ScottCurses.c:497-500`):
    /// `"\nI can also see: "` / `"\nYou can also see: "`. Same
    /// [`Presentation`] scoping as [`Wording::too_dark_to_see`].
    pub see_also_header: &'static str,
    /// `Look()`'s room-description prefix (`ScottCurses.c:465-468`):
    /// `"I'm in a "` / `"You are "`. Same [`Presentation`] scoping.
    pub room_prefix: &'static str,
    /// `main`'s unmatched-return fallback (`ScottCurses.c:1410`):
    /// `"I don't understand your command. "` — unconditional.
    pub dont_understand: &'static str,
    /// `main`'s "matched but blocked" fallback (`ScottCurses.c:1412`):
    /// `"I can't do that yet. "` — unconditional, and the whole point of
    /// SQ-1413 item 3 (`PerformActions`'s `-2` return, previously collapsed
    /// into [`Wording::dont_understand`]).
    pub cant_do_that_yet: &'static str,
    /// GET ALL, nothing with an auto-get word present (`ScottCurses.c:1218-1219`):
    /// `"Nothing taken."` — no trailing newline, unconditional.
    pub nothing_taken: &'static str,
    /// DROP ALL, nothing carried with an auto-get word (`ScottCurses.c:1271-1272`):
    /// `"Nothing dropped.\n"` — WITH a trailing newline (unlike
    /// [`Wording::nothing_taken`]), unconditional.
    pub nothing_dropped: &'static str,
    /// GET ALL's darkness short-circuit (`ScottCurses.c:1189-1193`):
    /// `"It is dark.\n"` — unconditional; DROP ALL has no such check (dropping
    /// in the dark is always allowed).
    pub it_is_dark: &'static str,
    /// GET (single item)/GET ALL's "pack is full" refusal, PERIOD-terminated
    /// (`ScottCurses.c:1204-1208,1229-1233`): `"I've too much to carry. "` /
    /// `"You are carrying too much. "`. Distinct from
    /// [`Wording::too_much_bang`] — ScottFree's own two capacity-refusal call
    /// sites disagree on punctuation and this crate ports both faithfully.
    pub too_much_period: &'static str,
    /// Opcode 52's "pack is full" refusal, BANG-terminated for the plain
    /// wording only (`ScottCurses.c:833-836`): `"I've too much to carry! "` /
    /// `"You are carrying too much. "` — note the `you_are` variant is
    /// IDENTICAL text to [`Wording::too_much_period`]'s `you_are` variant;
    /// only the plain (`!you_are`) wording differs by punctuation.
    pub too_much_bang: &'static str,
    /// Opcode 61's death message (`ScottCurses.c:874-877`): `"I am dead.\n"`
    /// / `"You are dead.\n"`.
    pub dead: &'static str,
    /// GET's "no such item here" refusal (`ScottCurses.c:1238-1241`):
    /// `"It's beyond my power to do that. "` / `"It is beyond your power to do
    /// that. "` — note ScottFree drops the apostrophe only in the `you_are`
    /// GET variant (`"It is"`, not `"It's"`); [`Wording::beyond_power_drop`]
    /// keeps the apostrophe in both variants. Trailing space, no newline.
    pub beyond_power_get: &'static str,
    /// DROP's "not carrying that" refusal (`ScottCurses.c:1283-1286`):
    /// `"It's beyond my power to do that.\n"` / `"It's beyond your power to do
    /// that.\n"` — BOTH variants keep the apostrophe (unlike
    /// [`Wording::beyond_power_get`]'s `you_are` form), and both end with a
    /// newline rather than a trailing space.
    pub beyond_power_drop: &'static str,
    /// Movement with an exit (`PerformActions`, `ScottCurses.c:1130-1133`):
    /// `"I can't go in that direction. "` / `"You can't go in that direction. "`.
    pub cant_go_that_direction: &'static str,
    /// Movement into darkness with no exit (`PerformActions`,
    /// `ScottCurses.c:1122-1124`): `"I fell down and broke my neck. "` /
    /// `"You fell down and broke your neck. "`.
    pub fell_and_broke_neck: &'static str,
    /// Movement while dark, printed whether or not the move then succeeds
    /// (`PerformActions`, `ScottCurses.c:1111`): `"Dangerous to move in the
    /// dark! "` — unconditional (no `you_are` branch in the source).
    pub dangerous_in_dark: &'static str,
    /// Opcode 63's `doneit` label (`ScottCurses.c:891`), also reached via
    /// opcode 65's win check: `"The game is now over.\n"` — unconditional.
    pub game_now_over: &'static str,
    /// Opcode 65's win line, printed just before falling into
    /// [`Wording::game_now_over`] (`ScottCurses.c:920`): `"Well done.\n"` —
    /// unconditional.
    pub well_done: &'static str,
    /// `GetInput`'s unknown-verb reply (`ScottCurses.c:646`): `"You use
    /// word(s) I don't know! "` — unconditional.
    pub unknown_words: &'static str,
    /// Lamp run-out (`main`, `ScottCurses.c:1425-1428`), gated on
    /// [`Options::scott_light`] rather than `you_are`: `"Light has run out! "`
    /// (scott_light) / `"Your light has run out. "` (default).
    pub light_out: &'static str,
    /// Lamp low-fuel warning under the DEFAULT (non-`scott_light`) wording
    /// (`main`, `ScottCurses.c:1447-1449`), shown only every 5th turn:
    /// `"Your light is growing dim. "`. Empty under `scott_light` — that
    /// variant uses [`Wording::light_runs_out_prefix`]/[`suffix`](Wording::light_runs_out_suffix)
    /// instead, shown every turn.
    pub light_dim: &'static str,
    /// Lamp low-fuel warning under `scott_light`, shown every turn under 25
    /// (`main`, `ScottCurses.c:1441-1444`): `"Light runs out in "`, then the
    /// live fuel number, then [`Wording::light_runs_out_suffix`].
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
        Wording {
            ok: "O.K. ",
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
            room_prefix: if y { "You are " } else { "I'm in a " },
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
