//! SQ-1354, the live path: *Bureaucracy*'s bracketed notes are BRIGHT on the
//! IBM PC, and this is the frame the player actually looks at.
//!
//! # What the machine did
//!
//! On the IBM PC a text cell is one attribute byte and `HLIGHT ,H-BOLD` is bit 3
//! of its foreground nibble — the same colour, *lit*. `crates/app`'s
//! `render::ibm_bold_fg` applies that to a run's RESOLVED ink under
//! [`Palette::IbmXzip`](zvm::screen::Palette::IbmXzip), and its unit test pins the
//! arithmetic on a synthetic base style. What the unit test cannot reach is the
//! CHAIN: the launch resolves a machine, the machine licenses a palette, the
//! palette resolves a period look, the look becomes the transcript selector's
//! style, the render reads that back as `base_style.fg`, and only then does the
//! bold rule get a colour to light. Every link is somebody else's code.
//!
//! # The specimen
//!
//! | fixture | release / serial | how the frame is reached |
//! |---|---|---|
//! | `stories/bureaucracy-r116-s870602.z4` | 116 / 870602 | 1 line, 1 key, the 14-field licence form, then one EMPTY command |
//!
//! That empty command is the turn this suite is about. *Bureaucracy* answers it
//! with two bracketed lines and prints them differently:
//!
//! ```text
//! <TELL "[What?]">                                   ; plain — grey
//! <HLIGHT ,H-BOLD> <TELL "[Your blood pressure …]">  ; bold  — white
//! ```
//!
//! (`SAY-SCORE-UPDATE`, `verbs.zil`.) `machine-screenshots/dos-bureaucracy.png`
//! is the oracle and measures exactly that split — `#A0A0A0` for `[What?]` and
//! `#FFFFFF` for the blood-pressure line — so one turn carries both halves of
//! the rule and a fix that brightened everything would fail here too.
//!
//! # The launch
//!
//! `lanthorn --interpreter 6 --colour machine <story>`: the flag names the IBM
//! PC ([`ProfileSource::Asked`](app::interpreter::ProfileSource::Asked)) and
//! `--colour machine` is the opt-in that licenses it, which is what
//! `Config::machine_colours_licensed` asks. Both are needed — see
//! [`the_licence_is_what_lights_the_line`], which pins the other side of that
//! gate, because `--interpreter 6` ALONE leaves `system_colours` false and the
//! whole machine — palette, period look and bold rule together — unlicensed.
//!
//! `stories/` is gitignored (CLAUDE.md), so every case skips vacuously without
//! it.

use std::path::PathBuf;

use app::colors::ColorScheme;
use app::interpreter::{InterpreterProfile, ProfileSource};
use app::machine_boot::MachineBoot;
use app::native_font::FaceSet;
use app::render::screen::render_story_pane;
use app::session::{screen_model_from_machine, GameSession, InputKind, TurnResult};
use app::state::{AppState, TranscriptKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

const STORY: &str = "bureaucracy-r116-s870602.z4";
const SCREEN: (u16, u16) = (25, 80);

/// The IBM PC's body ink, as its own palette resolves colour 9 — the grey every
/// plain run of prose is drawn in, and the colour the bold rule has to LIGHT
/// rather than replace.
const MACHINE_INK: Color = Color::Rgb(0xAD, 0xAD, 0xAD);
/// …and what lighting it produces: EGA 7 with the intensity bit on, which is 15.
const LIT_INK: Color = Color::Rgb(0xFF, 0xFF, 0xFF);

const PLAIN_LINE: &str = "[What?]";
const BOLD_LINE: &str = "[Your blood pressure";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The config a `--interpreter 6 --colour machine` launch resolves to, as far as
/// the colour chain reads it: the machine the flag named, the source that says a
/// FLAG named it, and the opt-in `--colour machine` pins.
///
/// `licensed` false models `--interpreter 6` on its own — same machine, same
/// profile source, no opt-in — which is the launch `Asked` declines.
fn ibm_config(licensed: bool) -> app::config::Config {
    let mut cfg = app::config::Config::default();
    cfg.interpreter_number = Some(6);
    cfg.interpreter_profile = InterpreterProfile::IbmPc;
    cfg.interpreter_source = ProfileSource::Asked;
    cfg.colour_source = app::config::ColourSource::Machine;
    cfg.system_colours = licensed;
    cfg.honor_game_colours = true;
    cfg.period_look = true;
    cfg
}

/// Boot the story the way `startup.rs` boots it under those flags, in the same
/// order: the profile first, the palette from the profile AND the story's
/// Version (`Config::machine_text_palette`, the one function both the launcher
/// and the harnesses ask), then the machine's five boot facts in one value, then
/// the session — with the host pane seeded before boot so `GO`'s `<LOWCORE SCRV>`
/// reads the real screen (SQ-0680).
///
/// Returns the session and the config it was booted under, because the render
/// half has to read the SAME config: a harness that booted licensed and rendered
/// unlicensed would measure a screen no launch produces.
fn boot(licensed: bool) -> Option<(GameSession, app::config::Config)> {
    let path = stories_dir().join(STORY);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let cfg = ibm_config(licensed);
    let zversion = bytes.first().copied();
    assert_eq!(zversion, Some(4), "Bureaucracy is a Version 4 story");

    // The palette this launch resolves colour numbers through — installed BEFORE
    // the constructor runs the story, exactly as `startup.rs` does.
    let palette = cfg.machine_text_palette(zversion);
    app::v6_set_palette(palette);
    if licensed {
        assert_eq!(
            palette,
            zvm::screen::Palette::IbmXzip,
            "a licensed IBM PC launch of a v4 story resolves through XZIP's table"
        );
        assert!(
            palette.bold_lights_the_intensity_bit(),
            "…which is the one display whose bold lights the intensity bit"
        );
    }

    let mut picts = app::graphics::PictSource::resolve_with_override(
        &path,
        app::graphics::PictureOverride::Unset,
        None,
    );
    // SQ-1021/SQ-1022: every per-machine boot fact in one value, so this harness
    // cannot omit one.
    let boot = MachineBoot::resolve(
        cfg.interpreter_profile,
        &picts,
        None,
        cfg.advertised_interpreter_number(),
        cfg.machine_default_colours(),
        cfg.machine_colours_licensed(),
        FaceSet::none(),
    );
    let s = GameSession::new_for_machine(
        bytes,
        cfg.honor_game_colours,
        false,
        false,
        picts.all_pict_dims(),
        Some(SCREEN),
        None,
        &boot,
    )
    .expect("Bureaucracy r116 should load and boot without a ZError");
    assert_eq!(
        u16::from_be_bytes([s.machine.mem.read_byte(2), s.machine.mem.read_byte(3)]),
        116,
        "this suite is pinned to release 116"
    );
    Some((s, cfg))
}

/// The app state that launch produces: the terminal-default theme with the
/// machine's screen laid under it, which is what `reload::reload_style` does on
/// every launch whose flags license one.
fn state_for(cfg: &app::config::Config) -> AppState {
    let mut state = AppState::default();
    state.colors = ColorScheme::terminal_default();
    state.config = cfg.clone();
    state.story_zversion = Some(4);
    state.period_look = app::period::resolve(
        cfg.interpreter_profile,
        cfg.period_look,
        cfg.honor_game_colours,
        cfg.machine_colours_licensed(),
        state.story_zversion,
    );
    if let Some(look) = state.period_look {
        app::period::apply_to_theme(&mut state.colors.theme, &look, state.story_zversion);
    }
    state
}

/// The upper window as text, one line per row — the form's own error line lives
/// here, and the driver reads it.
fn upper(s: &GameSession) -> String {
    let up = &s.machine.screen.upper;
    (0..up.rows as usize)
        .map(|r| {
            let row: String =
                (0..up.cols as usize).map(|c| up.cells[r * up.cols as usize + c].ch).collect();
            format!("{}\n", row.trim_end())
        })
        .collect()
}

/// Fold one turn's output into the state the way the app's turn path does.
fn apply_turn(state: &mut AppState, r: &TurnResult) {
    // …including the location signal, which is what arms the built-in LOCATION
    // rule below. `turn::apply_result` keeps the previous name when a turn has
    // none, so this mirrors it rather than clearing.
    if let Some(loc) = &r.location {
        state.current_room_name = Some(loc.name.clone());
    }
    if r.erase_lower {
        if let Some(anchor) = state.clear_anchor {
            state.truncate_transcript(anchor);
        }
        state.mark_screen_clear();
    }
    if r.transcript_elems.is_empty() {
        state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
    } else {
        app::state::apply_transcript_elems(state, &r.transcript_elems);
    }
}

/// Boot past the licence form to the game's first real prompt — the driver
/// SQ-1355 wrote, which offers a character per field and reads the form's own
/// `ERROR:` line back out of the upper window rather than scripting values.
fn fill_the_form(s: &mut GameSession) -> TurnResult {
    assert_eq!(s.pending_input(), InputKind::Line, "GO's DO-FORM? reads a line");
    s.submit("start");
    assert_eq!(s.pending_input(), InputKind::Char, "…then [Press any key to begin.]");
    s.submit_char(13);
    assert_eq!(s.pending_input(), InputKind::Char, "FILL-FIELD reads the form char by char");
    let form = upper(s);
    assert!(
        form.contains("SOFTWARE LICENCE APPLICATION"),
        "the licence form must be on screen before it can be filled in; upper was:\n{form}"
    );
    let mut last = None;
    for _ in 0..30 {
        if s.pending_input() != InputKind::Char {
            break;
        }
        for &cand in b"5MA" {
            s.submit_char(cand);
            if !upper(s).contains("ERROR:") {
                break;
            }
        }
        last = Some(s.submit_char(13));
    }
    assert_eq!(
        s.pending_input(),
        InputKind::Line,
        "the form should end at the game's own read prompt; upper was:\n{}",
        upper(s)
    );
    last.expect("the form asked for at least one field")
}

/// The pane the player is looking at, as `(glyphs, foregrounds)` per row: the
/// upper grid over the transcript, drawn by the shipped composite.
fn pane(s: &GameSession, state: &AppState) -> Vec<(String, Vec<Color>)> {
    let area = Rect::new(0, 0, SCREEN.1, SCREEN.0);
    let mut buf = Buffer::empty(area);
    let model = screen_model_from_machine(&s.machine);
    let _ = render_story_pane(&model, false, None, state, area, &mut buf);
    (0..area.height)
        .map(|y| {
            let mut text = String::new();
            let mut fgs = Vec::new();
            for x in 0..area.width {
                let cell = buf.cell((x, y)).unwrap();
                text.push_str(cell.symbol());
                fgs.push(cell.fg);
            }
            (text, fgs)
        })
        .collect()
}

/// The foregrounds of the cells holding `needle`, on the first pane row that
/// carries it. `None` when no row does — which every assertion below treats as a
/// failure, because a colour claim about a line that is not on screen is not a
/// claim about anything.
fn inks_of(pane: &[(String, Vec<Color>)], needle: &str) -> Option<Vec<Color>> {
    pane.iter().find_map(|(text, fgs)| {
        let at = text.find(needle)?;
        // `find` is a BYTE offset and the pane is one char per cell; both lines
        // this suite reads are ASCII, so the two coincide — asserted rather than
        // assumed, because a non-ASCII glyph left of the match would slide the
        // window silently.
        assert!(text.is_char_boundary(at) && text[..at].chars().count() == at);
        Some(fgs[at..at + needle.chars().count()].to_vec())
    })
}

/// Drive to the frame and hand back the pane, or `None` when the story is absent.
fn frame(licensed: bool) -> Option<Vec<(String, Vec<Color>)>> {
    let (mut s, cfg) = boot(licensed)?;
    let mut state = state_for(&cfg);
    // The turn that ends the form carries the banner, *Front Room* and its
    // description (SQ-1355), so the frame this suite measures has the room-name
    // heading on it as well as the two bracketed lines — one pane, three rules.
    let opening = fill_the_form(&mut s);
    apply_turn(&mut state, &opening);

    // The empty command that produces both lines. Everything before it is setup;
    // this is the turn the suite is about.
    let r = s.submit("");
    assert!(
        r.transcript.contains(PLAIN_LINE) && r.transcript.contains("blood pressure"),
        "an empty command must answer with both bracketed lines; it said {:?}",
        r.transcript
    );
    apply_turn(&mut state, &r);
    Some(pane(&s, &state))
}

/// The quest, on the frame the player sees.
///
/// Falsified by removing `render::ibm_bold_fg`'s call from
/// `render/transcript.rs` (`s = s.fg(c)`), which renders the blood-pressure line
/// in the machine ink and reports:
///
/// ```text
/// the bold line is the machine's ink LIT: [Rgb(173, 173, 173), …]
/// ```
#[test]
fn the_bold_bracketed_line_is_white_and_the_plain_one_is_not() {
    let _g = app::v6_palette(zvm::screen::Palette::IbmXzip);
    let Some(pane) = frame(true) else { return };

    let bold = inks_of(&pane, BOLD_LINE).unwrap_or_else(|| {
        panic!("{BOLD_LINE:?} is not on screen; pane was:\n{}", flat(&pane))
    });
    assert!(
        bold.iter().all(|&c| c == LIT_INK),
        "the bold line is the machine's ink LIT: {bold:?}"
    );

    // …and the plain line beside it is NOT, which is what makes the first claim
    // about BOLD rather than about the whole turn. `dos-bureaucracy.png`
    // measures `[What?]` at the body grey and the line below it at white.
    let plain = inks_of(&pane, PLAIN_LINE).unwrap_or_else(|| {
        panic!("{PLAIN_LINE:?} is not on screen; pane was:\n{}", flat(&pane))
    });
    assert!(
        plain.iter().all(|&c| c == MACHINE_INK),
        "the plain line stays the machine's body ink: {plain:?}"
    );
}

/// The engine half, stated so a failure names the cause as well as the symptom:
/// the game really does print that line with `HLIGHT ,H-BOLD`, and the bit
/// survives the capture into `state.transcript_runs`.
///
/// Without this, a render that lit nothing and a game that asked for nothing
/// would look the same from the pane.
#[test]
fn the_game_asks_for_bold_on_that_line_and_the_bit_survives_the_capture() {
    let _g = app::v6_palette(zvm::screen::Palette::IbmXzip);
    let Some((mut s, cfg)) = boot(true) else { return };
    let mut state = state_for(&cfg);
    let opening = fill_the_form(&mut s);
    apply_turn(&mut state, &opening);
    let r = s.submit("");
    apply_turn(&mut state, &r);

    let (line, runs) = state
        .transcript
        .iter()
        .zip(state.transcript_runs.iter())
        .find(|(l, _)| l.contains(BOLD_LINE))
        .unwrap_or_else(|| {
            panic!("{BOLD_LINE:?} is not in the transcript: {:?}", state.transcript)
        });
    let at = line.find(BOLD_LINE).expect("just matched");
    let covering: Vec<_> = runs.iter().filter(|r| r.start <= at && at < r.end).collect();
    assert!(
        !covering.is_empty() && covering.iter().all(|r| r.bits & 0x02 != 0),
        "SAY-SCORE-UPDATE prints this line inside <HLIGHT ,H-BOLD>, so the run \
         covering it must carry bit 0x02; runs on {line:?} were {runs:?}"
    );

    // …and the plain line does not, on the same turn.
    let (pline, pruns) = state
        .transcript
        .iter()
        .zip(state.transcript_runs.iter())
        .find(|(l, _)| l.contains(PLAIN_LINE))
        .unwrap_or_else(|| panic!("{PLAIN_LINE:?} is not in the transcript"));
    let pat = pline.find(PLAIN_LINE).expect("just matched");
    assert!(
        pruns.iter().filter(|r| r.start <= pat && pat < r.end).all(|r| r.bits & 0x02 == 0),
        "[What?] is a plain TELL; runs on {pline:?} were {pruns:?}"
    );
}

/// The gate, from the other side: **`--interpreter 6` alone does not light it**,
/// and this is where a player's report of "still dim" is most likely to come
/// from.
///
/// `ProfileSource::Asked` licenses a machine's own colours only on the opt-in
/// (`Config::machine_colours_licensed`), so a launch that names the machine and
/// nothing else resolves through §8.3.1's table, gets no period look, and its
/// bold runs are a terminal BOLD over the theme's ink — which most terminals do
/// not brighten. Nothing here is broken; it is the licence that is missing.
#[test]
fn the_licence_is_what_lights_the_line() {
    let _g = app::v6_palette(zvm::screen::Palette::Standard);
    assert!(
        !ibm_config(false).machine_colours_licensed(),
        "--interpreter 6 names a machine; --colour machine is what licenses it"
    );
    assert_eq!(
        ibm_config(false).machine_text_palette(Some(4)),
        zvm::screen::Palette::Standard,
        "…so an unlicensed launch resolves through §8.3.1's table"
    );

    let Some(pane) = frame(false) else { return };
    let bold = inks_of(&pane, BOLD_LINE)
        .unwrap_or_else(|| panic!("{BOLD_LINE:?} is not on screen; pane was:\n{}", flat(&pane)));
    assert!(
        bold.iter().all(|&c| c != LIT_INK),
        "an unlicensed launch has no machine to light: {bold:?}"
    );
}

fn flat(pane: &[(String, Vec<Color>)]) -> String {
    pane.iter().map(|(t, _)| format!("{}\n", t.trim_end())).collect()
}


/// The two other things on that same pane, pinned so the bold rule's REACH is a
/// record rather than an accident — one line it lights, one it does not.
///
/// - **`BUREAUCRACY`** is `V-VERSION`'s banner, printed inside `HLIGHT ,H-BOLD`,
///   and no built-in rule matches it. So it inherits the machine's body ink and
///   the bold rule lights it: `#FFFFFF`, which is what
///   `machine-screenshots/dos-bureaucracy.png` shows.
/// - **`Front Room`**, the room-name heading, is the built-in LOCATION rule's, and
///   that rule is **not** withdrawn on a machine frame — SQ-0822 kept it
///   deliberately, on the reasoning that it is a reading aid painting an accent
///   rather than a mute, and an accent is legible on any page. The accent is a
///   theme colour, so `ibm_bold_fg`'s round-trip guard leaves it alone and the
///   heading is `transcript_location`'s cyan whatever the game asked for.
///
/// **The capture disagrees with the second one**: DOS draws that heading bold
/// white, like the banner above it, because the game prints it exactly the same
/// way. Whether the location rule should stand down on a machine frame for the
/// same reason the system rule does is a decision SQ-0822 took the other way and
/// this quest did not reopen — so it is pinned here, with the disagreement named,
/// rather than left to be discovered as a surprise.
#[test]
fn the_banner_lights_and_the_room_name_heading_keeps_its_accent() {
    let _g = app::v6_palette(zvm::screen::Palette::IbmXzip);
    let Some(pane) = frame(true) else { return };

    let banner = inks_of(&pane, "BUREAUCRACY")
        .unwrap_or_else(|| panic!("the banner is not on screen; pane was:\n{}", flat(&pane)));
    assert!(
        banner.iter().all(|&c| c == LIT_INK),
        "V-VERSION prints the banner in HLIGHT and no rule claims it, so it is the \
         machine's ink lit: {banner:?}"
    );

    let heading = pane
        .iter()
        .skip_while(|(t, _)| !t.contains("Release 116"))
        .find_map(|(t, fgs)| {
            let at = t.find("Front Room")?;
            Some(fgs[at..at + "Front Room".len()].to_vec())
        })
        .unwrap_or_else(|| panic!("the room heading is not on screen; pane was:\n{}", flat(&pane)));
    assert!(
        heading.iter().all(|&c| c == Color::Cyan),
        "the built-in LOCATION rule paints the heading `transcript_location`'s \
         accent, which SQ-0822 left standing on machine frames: {heading:?}"
    );
    assert!(
        heading.iter().all(|&c| c != Color::DarkGray),
        "…and NOT the muted system colour, which is the rule this quest withdrew"
    );
}
