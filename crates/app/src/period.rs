//! The period look: painting a story the way its machine's own interpreter did
//! (SQ-0873, extended to v1–v6 by SQ-0935).
//!
//! [`zvm::interpreter::PeriodLook`] holds the measurements — a body pair, how the
//! status line was set apart, and the input cursor's shape and colour, for the
//! five machines `machine-screenshots/` has captures of. This module is the half
//! that decides whether they apply to *this* launch and turns them into terminal
//! cells.
//!
//! # The gate, and why every clause of it is there
//!
//! **The machine's screen is the machine's screen, v1 to v6.** This gate used to
//! stop at v4, on the reasoning that a period look would be "a lie for a v5+
//! story, where the pair on screen is a fact the story can read". That objection
//! was real but it was aimed at the wrong thing: painting one colour and REPORTING
//! another is the lie, not painting the machine's.
//!
//! Both come from the same row now. [`zvm::interpreter::MachineProfile`] holds a
//! machine's `default_colours` — the §8.3.1 NUMBERS a v5+ story reads out of
//! `$2C`/`$2D` — and its [`PeriodLook`], the RGB those numbers were on that
//! machine. They are two spellings of one fact, so a launch that paints the second
//! and answers the first is telling the story exactly what is on its screen. The
//! IBM PC states 6 under 9 and its screen IS those two numbers through EGA — so
//! that row stores no RGB at all, and resolving 6 is the whole of the answer
//! (SQ-0983).
//!
//! Before this, only v1–v4 read the RGB and v5+ resolved the number through the
//! app's own theme, so the same DOS machine showed EGA's blue to a v3 story and the
//! theme's `#006BB5` to a v6 one — and Shogun, which never names a colour until it
//! leaves InvisiClues, sat on the player's theme for the whole game and turned blue
//! on the way out of a menu (SQ-0935).
//!
//! A story that DOES name a colour still wins, on every version: the machine's
//! screen is the ground, laid under the theme by [`apply_to_theme`], and the game's
//! own pair is applied over it by the render paths. Base coat, not override.
//!
//! **v7 and v8 decline**, which is the same evidence rule the rest of this module
//! keeps: no Infocom machine shipped an interpreter for them, so there is no
//! period screen to have measured.
//!
//! **The STATUS BAND is the exception and stops at v4** — see
//! [`machine_states_the_status_colour`]. The boundary is where the row's COLOUR
//! changes hands, which is v5, and not where its drawing does, which is v4: a v4
//! game already splits its own upper window and paints the row, but has no
//! `set_colour` to shade it, so the shade is still the interpreter's. From v5 the
//! game can name that pair and does.
//!
//! **`honor_game_colours` is the master switch and this one is narrower.** A
//! player who turns game colours off has said "keep my terminal's colours", and
//! a blue Amiga page painted over that would be fighting them — so an off
//! `honor_game_colours` (from `--game-colours`, a `garglk.ini` stylehint, the
//! per-game sidecar, or SQ-0860's monochrome-artwork force-off) takes the period
//! look with it. The reverse does **not** hold, which is the whole reason
//! [`crate::config::Config::period_look`] exists separately: declining the look
//! must not also cost a v5+ story the colours it asked for.
//!
//! SQ-0860's force-off cannot actually reach here, and it is worth saying why
//! rather than leaving it to be rediscovered: it fires only for a **monochrome
//! artwork archive**, and an archive means Version 6. A story cannot be both
//! v6 and v1–v4, so the two features are disjoint by construction and the
//! ordering above is belt-and-braces.
//!
//! `NO_COLOR` has no reader in the TUI at all — it is a convention about adding
//! ANSI colour to command output, and lanthorn is a full-screen program whose
//! whole surface is decoration. `zvm-cli` is where it applies, and there it
//! already folds into the same `honor` flag `--game-colours` sets, so the
//! period look inherits the composition rather than restating it.
//!
//! # What the terminal can and cannot say
//!
//! The measurements are in pixels: a 1x16 Macintosh caret between glyphs, an 8x1
//! Commodore underscore on the cell's bottom scanline, an Amiga status line
//! reverse-videoed behind each run of text with the page showing between. A cell
//! grid can express the last one exactly and the first two only as the glyph that
//! occupies the same part of the cell — `▏` and `▁`. That is the honest analogue,
//! and it is named here rather than in a comment at the draw site so the loss is
//! recorded once.
//!
//! # The one thing a look may decline to say
//!
//! **A caret need not have a colour.** The Amiga's and the IBM PC's Version 6
//! interpreters draw it as the pair ON SCREEN reversed, which follows every colour
//! the story sets — `amiga-zorkzero.png` draws it black on Zork Zero's grey page
//! and `amiga-shogun.png` white on Shogun's dark one, one machine and two answers.
//! No RGB can stand for that, so [`caret_cell`] and [`caret_over_text`] answer
//! `None` and the host's structural caret draws instead: it reverses the live
//! style, which is exactly the machine's behaviour rather than an approximation of
//! it. Everything else in a look is still stated outright (SQ-0947).

use ratatui::style::{Color, Modifier, Style};
use zvm::interpreter::{CursorShape, PeriodLook, StatusBand};

use crate::interpreter::InterpreterProfile;
use crate::theme::resolve::Theme;

/// Does this launch get a period look, and which?
///
/// `licensed` is `Config::machine_colours_licensed` — **the same media rule the
/// machine's `$2C`/`$2D` pair obeys** (SQ-0928), and it applies here for the same
/// reason. A period look is what a machine's screen LOOKED LIKE, so being on that
/// machine is what makes it true of the launch. `InterpreterProfile::IbmPc` is
/// what every story with no medium falls through to, and it has a period look now
/// (`dos-hitchhiker.png`) — so without this, opening any bare v3 file would paint
/// it DOS blue.
///
/// `zversion` is the story's header byte 0; pass `None` for an engine that has no
/// such byte (Glulx, Scott Adams), which declines for the same reason a v5 story
/// does — §11.1.3 interpreter numbers are the Z-machine's vocabulary and nothing
/// else's.
///
/// See the module docs for why each clause is there.
pub fn resolve(
    profile: InterpreterProfile,
    enabled: bool,
    honor_game_colours: bool,
    licensed: bool,
    zversion: Option<u8>,
) -> Option<PeriodLook> {
    if !enabled || !honor_game_colours || !licensed {
        return None;
    }
    if !matches!(zversion, Some(1..=6)) {
        return None;
    }
    // SQ-0939: asked with the VERSION, because the IBM PC's white moved between
    // Infocom's two interpreters and its body pair is its own palette's resolution
    // of the pair it already states — one table lookup, not a second measurement to
    // drift from the first. `zvm::interpreter::period_look_for` carries the rule and
    // why the Amiga is excluded from it.
    zvm::interpreter::period_look_for(profile.row_number(), zversion)
}

/// Is the status row's colour the MACHINE's to state on this version?
///
/// **Not "does the interpreter draw the row" — that boundary is v3/v4 and it is
/// the wrong question.** ZMSD §8.2 has the interpreter print the status line in
/// v1–v3 only; from v4 the game splits its own upper window and paints the row
/// itself (`session::status_model_from_machine` draws exactly that line, `Classic`
/// at `version() <= 3` and `HostManaged` above it). So a v4 game already owns the
/// row, and this still answers true for it.
///
/// The question that decides the band is **who can COLOUR the row**, and colour
/// arrives at v5. A v4 game paints its status as reverse-video spaces and has no
/// `set_colour` to say what reverse means — the shade is the interpreter's, which
/// makes it the machine's, which is exactly what a [`PeriodLook`] records. The
/// Commodore 128's band was measured on *Trinity*, a v4 story, for that reason: the
/// game drew the row and the machine coloured it.
///
/// From v5 the game can name that row's pair outright and games do — Shogun colours
/// a 548x32 status strip of its own. Laying a machine's band under that would be
/// overriding the game rather than presenting the machine.
///
/// This is the one part of a look that does not generalise past v4. The body pair
/// is the machine's screen whoever prints on it, and the cursor is the
/// interpreter's own however the story is written.
pub fn machine_states_the_status_colour(zversion: Option<u8>) -> bool {
    matches!(zversion, Some(1..=4))
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

/// The machine's body pair as a style: its ink over its page.
pub fn body_style(look: &PeriodLook) -> Style {
    Style::new().fg(rgb(look.ink)).bg(rgb(look.page))
}

/// The status band's own base style — what the whole row is filled with before a
/// single segment is drawn.
///
/// [`StatusBand::PerRun`] fills with the **body** pair, because on the Amiga the
/// page is what shows between the two runs; the reversal is per segment and is
/// [`status_run_style`]'s. [`StatusBand::Ruled`] likewise fills with the body
/// pair — the Macintosh does not distinguish the band by ground at all — and
/// carries the rule as an underline, which is the one row-wide horizontal line a
/// cell grid has.
pub fn status_style(look: &PeriodLook) -> Style {
    let body = body_style(look);
    match look.status {
        StatusBand::FullReverse => reversed(body),
        StatusBand::PerRun => body,
        StatusBand::Own { ground, ink } => Style::new().fg(rgb(ink)).bg(rgb(ground)),
        StatusBand::Ruled => body.add_modifier(Modifier::UNDERLINED),
    }
}

/// The style for one run of status text when the band is [`StatusBand::PerRun`],
/// or `None` when the band's base already carries the whole answer.
///
/// The Amiga is the only machine measured that reverses behind the text and
/// leaves the page showing between; every other band is uniform, so its segments
/// inherit the base and this answers `None`.
pub fn status_run_style(look: &PeriodLook) -> Option<Style> {
    matches!(look.status, StatusBand::PerRun).then(|| reversed(body_style(look)))
}

fn reversed(s: Style) -> Style {
    Style::new().fg(s.bg.unwrap_or(Color::Reset)).bg(s.fg.unwrap_or(Color::Reset))
}

/// The caret at the END of the input line, where the cell is empty and the shape
/// is the whole of what is drawn: the glyph and the style to draw it in.
///
/// - [`CursorShape::Bar`] → `▏`, the left eighth of the cell. The Macintosh caret
///   is one pixel wide and sits in the gap *after* the last glyph, which is where
///   this cell is.
/// - [`CursorShape::Block`] → a space in the cursor's colour, filling the cell as
///   the Apple II's and the Amiga's do.
/// - [`CursorShape::Underscore`] → `▁`, the bottom eighth, which is where both
///   Commodores put their single scanline.
///
/// **`None` for [`CursorShape::ReverseSpace`]**, the caret the Amiga's and the IBM
/// PC's Version 6 interpreters draw (SQ-0947). It is the pair ON SCREEN reversed,
/// which changes with every colour the story sets, so there is no cell to state
/// here — and the host's own structural caret is already exactly that reversal.
/// Answering `None` hands the draw back to it rather than freezing one pair's
/// answer into a machine constant.
pub fn caret_cell(look: &PeriodLook) -> Option<(&'static str, Style)> {
    let colour = rgb(look.cursor_colour);
    match look.cursor_shape {
        CursorShape::Bar => Some(("▏", Style::new().fg(colour).bg(rgb(look.page)))),
        CursorShape::Block => Some((" ", Style::new().fg(rgb(look.page)).bg(colour))),
        CursorShape::Underscore => Some(("▁", Style::new().fg(colour).bg(rgb(look.page)))),
        CursorShape::ReverseSpace => None,
    }
}

/// The caret sitting ON a character — mid-line, or over the completion hint's
/// first glyph.
///
/// The glyph is kept and only the style applies, because the text has to stay
/// readable while it is edited. That means the SHAPE cannot be drawn: a `▏` or a
/// `▁` in this cell would replace the character rather than mark it. What the
/// machine's colours can still say is which cell the caret is in, so an
/// underscore machine underlines it in the cursor's colour and the other two
/// swap the pair. The shape is expressible only where the cell is empty; see
/// [`caret_cell`].
///
/// `None` for [`CursorShape::ReverseSpace`], for the reason [`caret_cell`] gives.
pub fn caret_over_text(look: &PeriodLook) -> Option<Style> {
    let colour = rgb(look.cursor_colour);
    match look.cursor_shape {
        CursorShape::Underscore => {
            Some(Style::new().fg(colour).bg(rgb(look.page)).add_modifier(Modifier::UNDERLINED))
        }
        CursorShape::Bar | CursorShape::Block => Some(Style::new().fg(rgb(look.page)).bg(colour)),
        CursorShape::ReverseSpace => None,
    }
}

/// The selectors the period look paints, each with the registry parent that has
/// to be unclaimed too, and what it paints them with.
///
/// **Only the story pane's own surfaces.** The map, the dialogs and the rest of
/// the chrome are lanthorn's, not the machine's, and a Commodore 64's grey page
/// across the whole application would be dressing up rather than presenting.
///
/// Two kinds of paint, and the difference matters. The prose and the line being
/// typed take the machine's PAIR, because on that machine they were its ink on
/// its page and nothing else. lanthorn's own annotations — the echoed command,
/// the meta gutter, a warning — take only the PAGE: their ink says something
/// lanthorn means (this line is yours, this one is not the story's) and no
/// machine has an opinion about it, but leaving their ground alone would punch
/// the theme's page through the machine's in the middle of the transcript.
fn painted(look: &PeriodLook) -> [(&'static str, &'static str, Style); 11] {
    let body = body_style(look);
    let page = Style::new().bg(rgb(look.page));
    [
        ("transcript", "text", body),
        // SQ-0873: **and the upper window**, which is where the bleed showed. A
        // v4 game's status bar is a grid it writes into, and *A Mind Forever
        // Voyaging* does not write across the whole width — so every cell it left
        // alone kept the host theme and the machine's page showed in stripes
        // beside the bar. The pane flood underneath cannot help: `draw_upper_window`
        // fills its own rect from this selector and paints over it.
        ("upper_window", "chrome", body),
        ("input_line", "line", body),
        ("input_text", "text", body),
        ("input_prompt", "text", body),
        ("transcript_input", "accent", page),
        ("transcript_meta", "muted", page),
        ("transcript_warning", "alert", page),
        // SQ-1045: the assist voice sits in the same stream as the meta lines and
        // has to take the machine's page with them, or an assist under a period
        // look is the one line still wearing the host theme's ground. Both tones
        // ride `alert` — the yellow slot — and separate by weight, so the role
        // named here is the same one twice.
        ("transcript_assist", "alert", page),
        ("transcript_assist_caution", "alert", page),
        // SQ-1107: the reveal paints over the STORY's own rows, so it has to take
        // the machine's page with them — otherwise a lit word is the one spot on a
        // machine's screen still wearing the host theme's ground.
        ("transcript_reveal", "accent", page),
    ]
}

/// Lay the machine's colours under the resolved theme.
///
/// **A user's choice outranks a machine default**, per selector, which is the
/// same rule SQ-0847 applied when the Macintosh's white page first reached the
/// input line: a selector any layer wrote (global `style.toml`, a discovered
/// `garglk.ini`, the per-game sidecar) keeps what that layer said, and only one
/// still at [`Provenance::Default`](crate::theme::resolve::Provenance) — along
/// with the role it inherits from — is the machine's to fill. So a player who
/// themed their transcript gets their theme on an Amiga floppy, and one who never
/// touched it gets the Amiga.
///
/// Called from `reload::reload_style`, which is the single place the theme is
/// built — startup, `/reload-style`, the style watcher and the per-game overlay
/// all funnel through it, so patching there reaches every path.
pub fn apply_to_theme(theme: &mut Theme, look: &PeriodLook, zversion: Option<u8>) {
    for (sel, role, style) in painted(look) {
        theme.fill_unclaimed(sel, role, style);
    }
    match look.status {
        // **Reverse is just reverse.** The `status_bar` row's registry default
        // already carries REVERSED — that is lanthorn's own way of setting the bar
        // apart — so patching the machine's UNswapped pair under it draws the band
        // with nothing stated: ink over page, reversed at draw, which is page over
        // ink. Identical to what stating `reversed(body)` absolutely produced, and
        // it needs no version clause, because it is not an override of anything. It
        // is what reversing the machine's screen means, and a game that colours its
        // own status row still wins over the theme at draw time.
        //
        // This is also where `Theme::set_unclaimed` came from: a swapped pair
        // patched under REVERSED swaps back, "a full reverse rendered as no reverse
        // at all". Patching the pair unswapped removes the cause rather than
        // working around it, so that hazard cannot recur here.
        StatusBand::FullReverse | StatusBand::PerRun => {
            theme.fill_unclaimed("status_bar", "chrome", body_style(look));
        }
        // The two bands that are NOT a reverse of anything: the Macintosh rules its
        // row instead of grounding it, and `Own` names a pair unrelated to the
        // body's. Those genuinely state the row rather than describing it, so they
        // are stated absolutely — and only while the row's colour is the machine's
        // to state (SQ-0935). See [`machine_states_the_status_colour`]: the boundary
        // is v5, where the game gains `set_colour` and can name that row itself, NOT
        // v4, where it merely starts drawing it.
        StatusBand::Ruled | StatusBand::Own { .. } => {
            if machine_states_the_status_colour(zversion) {
                theme.set_unclaimed("status_bar", "chrome", status_style(look));
            }
        }
    }
}

#[cfg(all(test, feature = "t-theme"))]
mod tests {
    use super::*;

    /// Asked of `period_look_for` rather than read off the row, because one row
    /// stores no pair at all — the IBM PC's screen is its own palette resolving the
    /// pair it reports (SQ-0983). `None` is the v1–v5 answer, which is every
    /// machine this helper is used for.
    fn look_of(n: u8) -> PeriodLook {
        zvm::interpreter::period_look_for(n, None).expect("measured")
    }

    /// The machine's screen is the machine's screen, v1 to v6 — and v7/v8 decline
    /// because no Infocom machine shipped an interpreter for them.
    ///
    /// This case used to assert the opposite for v5+ and was named for it. The
    /// objection it encoded — that a v5 story can READ its pair, so painting
    /// something else would be a lie — is answered rather than overruled: the
    /// painted RGB and the reported numbers are two spellings of one row now, so
    /// there is nothing to be inconsistent with. See the module docs.
    #[test]
    fn the_machines_screen_applies_through_version_six_and_no_further() {
        let amiga = InterpreterProfile::Amiga;
        for v in 1..=6 {
            assert!(resolve(amiga, true, true, true, Some(v)).is_some(), "v{v} ran on this machine");
        }
        for v in 7..=8 {
            assert!(resolve(amiga, true, true, true, Some(v)).is_none(), "v{v} never shipped on one");
        }
        // Glulx and Scott Adams have no §11.1.3 number to be a machine of.
        assert!(resolve(amiga, true, true, true, None).is_none());
    }

    /// …and the BAND is the one part that stops at v4 — because that is where the
    /// row's COLOUR changes hands, not where its drawing does (SQ-0935). A v4 game
    /// already paints its own status row, but has no `set_colour` to shade it; from
    /// v5 it does, and Shogun colours a 548x32 strip of its own.
    #[test]
    fn the_status_band_stops_where_the_game_starts_drawing_the_row() {
        for v in 1..=4 {
            assert!(machine_states_the_status_colour(Some(v)), "v{v}: the machine states the row's colour");
        }
        for v in 5..=8 {
            assert!(!machine_states_the_status_colour(Some(v)), "v{v}: the game can name it with set_colour");
        }
        assert!(!machine_states_the_status_colour(None), "an engine with no version byte is not a Z-machine");
        // The distinction this is named for: v4 DRAWS its own row and still gets the
        // machine's colour, because `set_colour` does not exist until v5.
        assert!(machine_states_the_status_colour(Some(4)), "v4 draws the row but cannot colour it");

        // …and it reaches the theme that way — but only for a band that is not a
        // plain reverse. The MACINTOSH is the specimen because its band is `Ruled`:
        // it rules the row instead of grounding it, which states the row rather than
        // describing it, and that is what the version clause guards. A full-reverse
        // machine has nothing to gate (see
        // `a_full_reverse_band_is_the_same_on_every_version_it_applies_to`).
        let look = look_of(zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER);
        assert!(matches!(look.status, StatusBand::Ruled), "the Mac rules its row");
        use crate::theme::resolve::{resolve as resolve_theme, Decls, Roles};
        let mut v3 = resolve_theme(&Roles::terminal_default(), &Decls::new(), &Decls::new(), &Decls::new());
        apply_to_theme(&mut v3, &look, Some(3));
        let mut v6 = resolve_theme(&Roles::terminal_default(), &Decls::new(), &Decls::new(), &Decls::new());
        apply_to_theme(&mut v6, &look, Some(6));
        assert_ne!(
            v3.get("status_bar").style,
            v6.get("status_bar").style,
            "the band is the version-specific part; the body pair is not"
        );
        // The body pair, by contrast, is identical on both.
        assert_eq!(v3.get("transcript").style, v6.get("transcript").style, "the machine's screen does not change with the version");
    }

    /// One-way composition (SQ-0855/SQ-0860): the master switch takes the look
    /// with it, the narrow one does not reach game colours.
    #[test]
    fn honor_game_colours_is_the_master_and_the_key_is_narrower() {
        let amiga = InterpreterProfile::Amiga;
        assert!(resolve(amiga, true, false, true, Some(3)).is_none(), "colours off takes the look");
        assert!(resolve(amiga, false, true, true, Some(3)).is_none(), "and the key declines alone");
        assert!(resolve(amiga, true, true, true, Some(3)).is_some());
    }

    /// **The LICENCE is what declines, not the absence of a measurement** — and
    /// that distinction is now the only one left, because every machine in the
    /// table has been measured (the Atari ST was the last, SQ-0933).
    ///
    /// It was always the stronger guarantee, and it is why the change of
    /// arrangement costs this case nothing: a rule that holds for every machine,
    /// including ones measured later, beats one that happened to hold because a
    /// screenshot was missing.
    #[test]
    fn an_unlicensed_launch_declines_even_a_measured_machine() {
        // Both of these machines HAVE a capture now.
        assert!(InterpreterProfile::AtariSt.period_look().is_some(), "st-zork1.png");
        assert!(InterpreterProfile::IbmPc.period_look().is_some(), "dos-hitchhiker.png");
        assert!(
            resolve(InterpreterProfile::AtariSt, true, true, false, Some(3)).is_none(),
            "an unlicensed launch declines the ST's look",
        );
        assert!(
            resolve(InterpreterProfile::IbmPc, true, true, false, Some(3)).is_none(),
            "the PC has one, and an unlicensed launch still declines it",
        );
        assert!(
            resolve(InterpreterProfile::IbmPc, true, true, true, Some(3)).is_some(),
            "…and a licensed one gets it",
        );
    }

    /// **No machine reverses per run any more**, by the user's ruling (SQ-0873):
    /// the Amiga and the IBM PC both measure that way and both draw a full-width
    /// reverse, because a band broken into pieces reads as damage in a terminal.
    /// So `status_run_style` answers `None` for every row, and this is the case
    /// that fails if one quietly comes back.
    #[test]
    fn no_machine_reverses_per_run_and_the_bands_are_uniform() {
        for n in [
            zvm::interpreter::AMIGA_INTERPRETER_NUMBER,
            zvm::interpreter::IBM_PC_INTERPRETER_NUMBER,
            zvm::interpreter::APPLE_IIE_INTERPRETER_NUMBER,
            zvm::interpreter::COMMODORE_128_INTERPRETER_NUMBER,
            zvm::interpreter::COMMODORE_64_INTERPRETER_NUMBER,
            zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER,
        ] {
            assert!(status_run_style(&look_of(n)).is_none(), "interpreter {n}: uniform band");
        }
        // A full reverse is the swapped body pair, stated outright — see
        // `Theme::set_unclaimed` for why it is not left to a REVERSED modifier.
        let amiga = look_of(zvm::interpreter::AMIGA_INTERPRETER_NUMBER);
        let band = status_style(&amiga);
        assert_eq!(band.fg, Some(rgb(amiga.page)));
        assert_eq!(band.bg, Some(rgb(amiga.ink)));
    }

    /// The Macintosh does not distinguish its band by ground at all — it rules
    /// it. A cell grid's one horizontal rule is the underline.
    #[test]
    fn the_macintosh_band_is_the_body_pair_with_a_rule_under_it() {
        let mac = look_of(zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER);
        let band = status_style(&mac);
        assert_eq!((band.fg, band.bg), (body_style(&mac).fg, body_style(&mac).bg));
        assert!(band.add_modifier.contains(Modifier::UNDERLINED), "rules, not a ground");
    }

    /// Three shapes, three glyphs, and the cursor's colour is neither the page
    /// nor the ink on two of the five machines — so the caret cannot be built out
    /// of the body pair.
    #[test]
    fn the_caret_draws_its_machines_shape_and_its_own_colour() {
        let mac = look_of(zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER);
        let amiga = look_of(zvm::interpreter::AMIGA_INTERPRETER_NUMBER);
        let c128 = look_of(zvm::interpreter::COMMODORE_128_INTERPRETER_NUMBER);
        assert_eq!(caret_cell(&mac).expect("the Mac states one").0, "▏");
        assert_eq!(caret_cell(&amiga).expect("the Amiga states one").0, " ");
        assert_eq!(caret_cell(&c128).expect("the C128 states one").0, "▁");

        // The Amiga's orange is in neither channel of its body pair, which is the
        // case that would break if the caret were built by reversing the body.
        let cell = caret_cell(&amiga).expect("the Amiga states one").1;
        assert_eq!(cell.bg, Some(rgb(amiga.cursor_colour)));
        assert_ne!(cell.bg, Some(rgb(amiga.page)));
        assert_ne!(cell.bg, Some(rgb(amiga.ink)));
    }

    /// …and a look may state NO caret, which is not the same as stating a dull
    /// one (SQ-0947).
    ///
    /// The Amiga's and the IBM PC's Version 6 interpreters draw the pair on screen
    /// reversed, so the caret has no colour of its own to hold — two Amiga v6
    /// captures show it black on one game's grey page and white on another's dark
    /// one. `None` is how that is said, and it hands the draw to the host's
    /// structural caret, which is already that reversal.
    #[test]
    fn a_version_six_look_states_no_caret_because_its_caret_is_the_live_pair() {
        for profile in [InterpreterProfile::Amiga, InterpreterProfile::IbmPc] {
            let v6 = resolve(profile, true, true, true, Some(6)).expect("a measured machine");
            assert!(caret_cell(&v6).is_none(), "{profile:?} v6 states no cell");
            assert!(caret_over_text(&v6).is_none(), "{profile:?} v6 states no style");

            // …while every version the stored capture speaks for still does, so
            // this cannot quietly undo SQ-0873.
            for v in 1..=5 {
                let look = resolve(profile, true, true, true, Some(v)).expect("a measured machine");
                assert!(caret_cell(&look).is_some(), "{profile:?} v{v} keeps its measured caret");
                assert!(caret_over_text(&look).is_some(), "{profile:?} v{v}");
            }
        }
        // The Macintosh is the control: its v6 frames draw the same bar its v3 one
        // does, so it states a caret on every version.
        let mac = resolve(InterpreterProfile::Macintosh, true, true, true, Some(6)).expect("measured");
        assert_eq!(caret_cell(&mac).expect("the Mac states one").0, "▏");
    }

    /// SQ-0847's rule, reused: a machine default fills what nobody claimed and
    /// never overwrites a choice. Both directions, and the ROLE counts as a claim
    /// — [`Provenance`] does not travel down the parent chain, so a player who
    /// recoloured `text` and left `transcript` alone has still chosen the
    /// transcript's ink.
    #[test]
    fn a_users_choice_outranks_the_machine_and_an_untouched_selector_does_not() {
        use crate::theme::resolve::{resolve, Decls, Roles};
        use crate::theme::registry::Delta;

        let look = look_of(zvm::interpreter::AMIGA_INTERPRETER_NUMBER);
        let one = |sel: &str, d: Delta| {
            let mut m = Decls::new();
            m.insert(sel.to_string(), d);
            m
        };

        // Nobody claimed anything: the Amiga's page and ink land.
        let mut bare = resolve(&Roles::terminal_default(), &Decls::new(), &Decls::new(), &Decls::new());
        apply_to_theme(&mut bare, &look, Some(3));
        assert_eq!(bare.get("transcript").style.bg, Some(rgb(look.page)));
        assert_eq!(bare.get("transcript").style.fg, Some(rgb(look.ink)));

        // The selector itself claimed: the player's ink survives the floppy.
        let mine = one("transcript", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let mut themed = resolve(&Roles::terminal_default(), &mine, &Decls::new(), &Decls::new());
        apply_to_theme(&mut themed, &look, Some(3));
        assert_eq!(themed.get("transcript").style.fg, Some(Color::Green));
        assert_ne!(themed.get("transcript").style.bg, Some(rgb(look.page)));

        // Only the ROLE claimed, and that is a claim too.
        let role = one("text", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let mut inherited = resolve(&Roles::terminal_default(), &role, &Decls::new(), &Decls::new());
        apply_to_theme(&mut inherited, &look, Some(3));
        assert_eq!(inherited.get("transcript").style.fg, Some(Color::Green));
    }

    /// **Reverse is just reverse**, so a full-reverse band states nothing and comes
    /// out identical.
    ///
    /// This case used to assert the opposite, and was named for it. The status bar
    /// ships REVERSED as its registry default — lanthorn's own way of setting the
    /// bar apart — and stating `reversed(body)` under that swapped back, "a full
    /// reverse rendered as no reverse at all", which is why `Theme::set_unclaimed`
    /// exists. Patching the pair UNSWAPPED removes the cause instead: ink over page,
    /// reversed at draw, which is page over ink.
    ///
    /// The assertion is the identity, because that is the whole claim — what the
    /// row DRAWS is unchanged, and only the way it got there is simpler.
    #[test]
    fn a_full_reverse_band_is_left_to_reverse_itself_and_draws_the_same() {
        use crate::theme::resolve::{resolve, Decls, Roles};
        let look = look_of(zvm::interpreter::COMMODORE_64_INTERPRETER_NUMBER);
        let mut theme = resolve(&Roles::terminal_default(), &Decls::new(), &Decls::new(), &Decls::new());
        assert!(
            theme.get("status_bar").style.add_modifier.contains(Modifier::REVERSED),
            "the registry default the band now leans on rather than guards against"
        );
        apply_to_theme(&mut theme, &look, Some(3));
        let band = theme.get("status_bar").style;
        // The pair is the body's, UNSWAPPED, and REVERSED survives to do the swap.
        assert_eq!(band.fg, Some(rgb(look.ink)), "the machine's ink, unswapped");
        assert_eq!(band.bg, Some(rgb(look.page)));
        assert!(band.add_modifier.contains(Modifier::REVERSED), "and the row reverses it at draw");
        // Which is the same thing on screen as the old stated band: page over ink.
        assert_eq!(
            (band.bg, band.fg),
            (status_style(&look).fg, status_style(&look).bg),
            "reversed at draw == what `status_style` stated absolutely",
        );
    }

    /// …and because it is not an override, a full-reverse band needs no version
    /// clause: it applies wherever the machine's screen does. The C64 is the same
    /// row on a v3 story and a v6 one.
    #[test]
    fn a_full_reverse_band_is_the_same_on_every_version_it_applies_to() {
        use crate::theme::resolve::{resolve, Decls, Roles};
        let look = look_of(zvm::interpreter::COMMODORE_64_INTERPRETER_NUMBER);
        let band = |v: u8| {
            let mut t = resolve(&Roles::terminal_default(), &Decls::new(), &Decls::new(), &Decls::new());
            apply_to_theme(&mut t, &look, Some(v));
            t.get("status_bar").style
        };
        for v in 1..=6 {
            assert_eq!(band(v), band(3), "v{v}: reversing the machine's pair is not an override");
        }
    }

    /// Over a character the glyph must survive, so the shape stands down and only
    /// the colour speaks — except for the underscore, which a terminal can draw
    /// under a character without hiding it.
    #[test]
    fn a_caret_over_text_keeps_the_text_readable() {
        let c64 = look_of(zvm::interpreter::COMMODORE_64_INTERPRETER_NUMBER);
        let over = caret_over_text(&c64).expect("the C64 states one");
        assert!(over.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(over.fg, Some(rgb(c64.cursor_colour)));

        let mac = look_of(zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER);
        let over = caret_over_text(&mac).expect("the Mac states one");
        assert!(!over.add_modifier.contains(Modifier::UNDERLINED), "a bar cannot go under a glyph");
        assert_eq!(over.bg, Some(rgb(mac.cursor_colour)));
    }
}
