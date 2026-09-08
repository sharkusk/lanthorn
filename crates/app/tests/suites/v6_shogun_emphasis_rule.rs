//! SQ-1028 — §8.7.1's Italic bit reaches the HYBRID pane as exactly one rendering.
//!
//! # What this frame is, and why it is the one
//!
//! `machine-screenshots/amiga-shogun-game.png` is a capture of Shogun running on a
//! real Amiga. It draws `Erasmus` in "This is the bridge of the Erasmus, a Dutch
//! merchant and privateer" with a solid rule under that word and nothing under the
//! words beside it — row by row at the capture's scale the glyph ink runs 336..349
//! and the rule is 350..351 against a sixteen-row line pitch. It is the only capture
//! in the tree that shows an emphasised run at all, so it is the frame every question
//! about emphasis has to be asked on.
//!
//! # What is settled, and what is not
//!
//! ZMSD §8.7.1 leaves the rendering open in as many words — "An interpreter need not
//! provide Bold or Italic (even for font 1) and is free to interpret them broadly.
//! (For example, rendering bold-face by changing the colour, or rendering italic with
//! underlining.)" — so neither answer is a compliance question. lanthorn's rule is to
//! use a real italic FACE where one is available, underline where none is, and never
//! synthesise a slope of its own. On the cell paths this pane is drawn by, the face is
//! the player's TERMINAL font and `Modifier::ITALIC` (SGR 3) is precisely the request
//! for it, so hybrid asks for italics; where lanthorn holds the face itself —
//! `render::bitfont`, blitting a release's own bitmap typeface — the second half of
//! the rule applies instead.
//!
//! So the cases below pin what the rule settles either way, on the real frame:
//!
//! 1. the bit **arrives** — it was dropped entirely on the v6 cell paths once, which
//!    rendered emphasised text roman;
//! 2. it arrives as **exactly one** rendering — sloping AND ruling the same run is
//!    neither of the two things a machine did;
//! 3. and it lands on the emphasised **word only**, not the line, which is what the
//!    capture shows and what a line-wide underline would quietly break.
//!
//! # The specimen
//!
//! | fixture | release / serial | machine | inputs |
//! |---|---|---|---|
//! | `James Clavell's Shogun.adf` | 295 / 890321 | Amiga | 2 |
//! | `shogun-r322-s890706.z6` | 322 / 890706 | none — a bare story file | 2 |
//!
//! Two inputs: one keypress takes START on the boot menu, and the intro prose and the
//! Bridge description arrive on the next. The emphasised word is PROSE, which in
//! hybrid is the terminal transcript rather than a painted run — measured, not
//! assumed: on this frame no `PxText` carries the Italic bit and the transcript run
//! does. (`Erasmus` also appears as the location in the status bar two rows up,
//! unemphasised and reverse-video, which is why the cases anchor on the sentence.)
//!
//! Both `honor_game_colours` modes are pinned (CLAUDE.md): emphasis is not a colour
//! the game asked for, and a suite that pinned only the shipped default could not
//! show that.
//!
//! `stories/` is gitignored, so every case skips vacuously.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// The Amiga floppy — the press `amiga-shogun-game.png` is a capture of.
const AMIGA: &str = "James Clavell's Shogun.adf";
const AMIGA_RELEASE: u16 = 295;
const AMIGA_SERIAL: &str = "890321";

/// The bare story file: no medium, so no machine behind it.
const BARE: &str = "shogun-r322-s890706.z6";
const BARE_RELEASE: u16 = 322;
const BARE_SERIAL: &str = "890706";

/// The word the capture rules under.
const EMPHASISED: &str = "Erasmus";

/// The sentence it sits in — the anchor that tells the PROSE occurrence apart from
/// the status bar's, which carries the same word unemphasised.
const SENTENCE: &str = "This is the bridge of the ";

/// A roomy pane, so the description reaches the emphasised word on one row and the
/// wrap is not part of what is under test.
const PANE: Rect = Rect { x: 0, y: 0, width: 120, height: 44 };

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` boots (CLAUDE.md): the profile comes from the medium
/// the MOUNT returned, the screen size from `MachineBoot`'s four-link cascade, and the
/// release's own face rides along, which is what settles the declared cell.
///
/// Prints what it booted, so a fabricated frame shows up in the log rather than
/// looking plausible in an assertion (SQ-0901).
fn boot(file: &str, release: u16, serial: &str) -> Option<(GameSession, app::machine_boot::MachineBoot)> {
    let path = stories_dir().join(file);
    let Ok((loaded, _)) = app::hints::load_mounted_story(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release, "{file}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{file}: serial");
    let (profile, source) = InterpreterProfile::resolve_with_source(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    // `disks: None` on purpose: a case here must not depend on what the person
    // running it keeps in `~/.lanthorn/` (SQ-1037). Shogun carries no typeface on
    // either press, so every rung declines and the built-in answers — which is what
    // this suite wants, since it is measuring where the STYLE BIT goes and not which
    // face draws it.
    let face = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source,
        art_scale: picts.art_scale(),
        disks: None,
    });
    let machine = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        face,
        profile.palette(),
        None,
    );
    eprintln!(
        "{file}: release {release}, profile {:?}, screen {:?}, art_scale {:?}, cell {:?}",
        machine.profile, machine.screen_px, machine.art_scale, machine.cell,
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &machine)
        .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some((s, machine))
}

/// A hybrid `AppState` carrying the machine's own face, cell and art density, and the
/// story's Version — the four things `startup.rs` puts there.
fn hybrid_state(machine: &app::machine_boot::MachineBoot, honor: bool) -> app::state::AppState {
    #[allow(deprecated)] // `from_fontsize`: a headless test has no terminal to query.
    let picker = ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 16));
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state.v6_text = machine.text_face();
    if let Some(scale) = machine.art_scale {
        state.v6_art_scale = scale;
    }
    state.story_zversion = Some(6);
    state
}

/// Drive to the Bridge, keeping each turn's output the way `turn.rs` keeps it — with
/// its style chunks, which is where §8.7.1's bits live.
fn on_the_bridge(file: &str, release: u16, serial: &str, honor: bool) -> Option<(GameSession, app::state::AppState, usize)> {
    let (mut s, machine) = boot(file, release, serial)?;
    let mut state = hybrid_state(&machine, honor);
    let t0 = s.take_transcript();
    state.push_transcript_kind(&t0, app::state::TranscriptKind::Story);
    let mut inputs = 0;
    for _ in 0..6 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        inputs += 1;
        assert!(r.fault.is_none(), "{file}: faulted on input {inputs}: {:?}", r.fault);
        state.push_transcript_runs(&r.transcript, app::state::TranscriptKind::Story, &r.transcript_runs);
        if r.transcript.contains(EMPHASISED) {
            return Some((s, state, inputs));
        }
    }
    panic!("{file}: never reached the Bridge description in 6 inputs");
}

/// Every span of transcript the GAME emphasised, as text — §8.7.1's Italic bit as the
/// model carries it, before any renderer has had an opinion.
fn emphasised_runs(state: &app::state::AppState) -> Vec<String> {
    let mut out = Vec::new();
    for (i, runs) in state.transcript_runs.iter().enumerate() {
        for run in runs {
            if run.bits & 4 != 0 {
                let line = state.transcript.get(i).map(|l| l.as_str()).unwrap_or("");
                out.push(line.chars().skip(run.start).take(run.end - run.start).collect());
            }
        }
    }
    out
}

/// Render the frame, find the emphasised word in the SENTENCE (never the status bar's
/// copy of it), and hand back the modifiers of its cells and of the four cells after
/// the comma that follows it.
fn prose_word_modifiers(session: &GameSession, state: &app::state::AppState) -> Option<(Vec<Modifier>, Vec<Modifier>)> {
    let model = Engine::screen(session);
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, false, None, state, PANE, &mut buf);
    for y in PANE.y..PANE.bottom() {
        let row: String = (PANE.x..PANE.right())
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default())
            .collect();
        let Some(at) = row.find(SENTENCE) else { continue };
        let col = (row[..at].chars().count() + SENTENCE.chars().count()) as u16;
        let mods = |from: u16, n: u16| -> Vec<Modifier> {
            (from..from + n).filter_map(|x| buf.cell((PANE.x + x, y)).map(|c| c.modifier)).collect()
        };
        let n = EMPHASISED.chars().count() as u16;
        let word: String = row.chars().skip(col as usize).take(n as usize).collect();
        assert_eq!(word, EMPHASISED, "the sentence must be followed by the emphasised word");
        // ", a D" — the cells after the word, which the capture leaves unruled.
        return Some((mods(col, n), mods(col + n, 5)));
    }
    None
}

/// One bit, one rendering, one word — on the press the capture is of.
///
/// Falsified by dropping bit 4 in `render::apply_text_style`: "the emphasised run must
/// reach the pane as a rendering — cell 0 of `Erasmus` carries NONE".
#[test]
fn amiga_shogun_emphasised_prose_carries_exactly_one_rendering() {
    emphasis_case(AMIGA, AMIGA_RELEASE, AMIGA_SERIAL);
}

/// The same frame off the bare story file, which has no machine behind it: the rule
/// is a property of the RENDERER's face, so the answer here is the same one.
#[test]
fn bare_shogun_emphasised_prose_carries_exactly_one_rendering() {
    emphasis_case(BARE, BARE_RELEASE, BARE_SERIAL);
}

fn emphasis_case(file: &str, release: u16, serial: &str) {
    for honor in [true, false] {
        let Some((session, state, inputs)) = on_the_bridge(file, release, serial, honor) else {
            return;
        };
        // Non-vacuity: this is the frame the capture is of, reached the way the table
        // says, and the game really did emphasise exactly the one word.
        assert_eq!(inputs, 2, "{file}: the Bridge description is 2 inputs in");
        assert_eq!(
            emphasised_runs(&state),
            vec![EMPHASISED.to_string()],
            "{file} honor={honor}: the frame must carry exactly the emphasised run the capture rules under"
        );
        let Some((word, after)) = prose_word_modifiers(&session, &state) else {
            panic!("{file} honor={honor}: the Bridge description must appear in the rendered pane");
        };
        assert_eq!(word.len(), EMPHASISED.chars().count(), "{file} honor={honor}: every cell of the word");
        for (i, m) in word.iter().enumerate() {
            let slope = m.contains(Modifier::ITALIC);
            let rule = m.contains(Modifier::UNDERLINED);
            assert!(
                slope || rule,
                "{file} honor={honor}: the emphasised run must reach the pane as a rendering — \
                 cell {i} of `{EMPHASISED}` carries {m:?}"
            );
            assert!(
                !(slope && rule),
                "{file} honor={honor}: sloping and ruling are two renderings of ONE bit, and doing \
                 both is neither — cell {i} of `{EMPHASISED}` carries {m:?}"
            );
        }
        // …and it is the WORD that is emphasised, not the line: the capture rules
        // under `Erasmus` and under nothing beside it.
        for (i, m) in after.iter().enumerate() {
            assert!(
                !m.contains(Modifier::ITALIC) && !m.contains(Modifier::UNDERLINED),
                "{file} honor={honor}: the words beside the emphasised one carry no rendering of the \
                 bit — cell {i} after `{EMPHASISED}` carries {m:?}"
            );
        }
    }
}
