//! SQ-0778: Amiga Shogun's `>` prompt is not reverse video, and neither is the
//! prose behind it.
//!
//! The report: *"after the first input, the `>` is improperly reversed in future
//! inputs."* Reproduced on `stories/James Clavell's Shogun.adf` — **release 295 /
//! serial 890321**, the Amiga release floppy and a different BUILD from the bare
//! `shogun-r322-s890706.z6` beside it (CLAUDE.md, "a disk image is a different
//! release"). The medium selects the Amiga interpreter profile, and the profile
//! is the whole of it: measured knob by knob, the ONLY input that turns the
//! defect on is the interpreter number in header `$1E` (4, Amiga — ZMSD §11.1.3).
//! Neither the Amiga palette, nor its default colour pair, nor its 320×200
//! standard window, nor `honor_game_colours` moves it, and forcing interpreter 4
//! onto r322 reproduces it there too.
//!
//! What Shogun does with interpreter 4, from its own screen trace, once per turn:
//!
//! ```text
//! @set_window(upper)
//! @set_text_style(reverse)      ← the status line, in window 1
//! …paints the status line…
//! @set_window(lower)            ← back to the prose window; no set_text_style(0)
//! ```
//!
//! It needs no `set_text_style 0` because in Version 6 the style is not global:
//! ZMSD §8.8.3.2 makes it window property **10** and §8.8.3.2.3 says it "is set
//! just as in Version 4, using `set_text_style` (which sets that for the current
//! window)". zvm's `set_window` mirrored the selected window's COLOUR pair (§8.3)
//! and not its style, so window 1's reverse video followed the game back into
//! window 0 and stayed there for the rest of the game. The fix is in
//! `crates/zvm/src/cpu/exec.rs`, VAR:0x0B; `crates/zvm/tests/v6_window_text_style.rs`
//! pins the mechanism on a synthetic story, and this file pins it on the game.
//!
//! It was never only the prompt. Everything window 0 printed from the second turn
//! on carried the bit: the 335-character opening paragraph, every room heading
//! (`Bridge` came out reverse+bold instead of bold), the death notice. Swept
//! across the whole v6 corpus — both Shogun builds, both Journey builds, both Zork
//! Zero builds, both Arthur builds, advent, fmvpoker, mysterious01, scopa,
//! sunburst, Beyond Zork, Zork I and ZTUU, floppies included — the fix moves this
//! one release and leaves every other title byte-identical.
//!
//! Reverse video itself must survive: Shogun asks for it in window 1 every turn
//! and it has to land there. [`the_status_window_really_is_reverse_video`] is that
//! control, so a regression cannot be "fixed" by never reversing anything.
//!
//! Both `honor_game_colours` modes are pinned (project convention: `true` is the
//! shipped default and primary baseline). The style bit is colour-independent, so
//! agreeing across the two IS the assertion.
//!
//! `stories/` is gitignored (CLAUDE.md), so every case skips vacuously when the
//! media are absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

/// The build the report is about: the Amiga release floppy.
const AMIGA_RELEASE: &str = "James Clavell's Shogun.adf";
/// The IBM PC build as an ordinary story file — the control that was always right.
const PC_RELEASE: &str = "shogun-r322-s890706.z6";

/// (release, serial) each medium must carry, guarded before anything is measured
/// so no finding here can be attributed to the wrong build. Mirrors the table in
/// `real_media_releases.rs`.
fn pinned(file: &str) -> (u16, &'static str) {
    match file {
        AMIGA_RELEASE => (295, "890321"),
        PC_RELEASE => (322, "890706"),
        other => panic!("no pinned release for {other}"),
    }
}

/// ZMSD §8.7.2's style bits: 1 reverse video, 2 bold, 4 italic, 8 fixed-pitch.
const REVERSE: u8 = 0x01;
const BOLD: u8 = 0x02;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// How a failure names its subject.
fn ctx(file: &str) -> String {
    let (release, serial) = pinned(file);
    format!("{file} [release {release}, serial {serial}]")
}

/// Boot `file` the way `startup` does — the profile from the MEDIUM, the artwork
/// from that same container — with the game's `>` prompt left in the transcript.
///
/// `strip_prompt(false)` is not a test convenience: `command_bar` defaults off
/// (`config.rs`), and `startup.rs` passes that straight to `set_strip_prompt`, so
/// inline play is the shipped mode and the game's own `>` really is the last thing
/// on the transcript line the player types into. It is the character the report is
/// about.
fn boot(file: &str, honor_game_colours: bool) -> Option<GameSession> {
    let story_path = stories_dir().join(file);
    let (loaded, mounted) = match app::hints::load_mounted_story(&story_path) {
        Ok(pair) => pair,
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", story_path.display());
            return None;
        }
    };
    let bytes = loaded.bytes().to_vec();

    let (release, serial) = pinned(file);
    assert_eq!(bytes[0], 6, "{}: Shogun is a v6 story", ctx(file));
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        release,
        "{}: this medium carries a DIFFERENT build than the table says",
        ctx(file)
    );
    assert_eq!(&bytes[0x12..0x18], serial.as_bytes(), "{}: serial", ctx(file));
    assert_eq!(
        mounted == Some(app::hints::DiskImage::Adf),
        file.ends_with(".adf"),
        "{}: the mount reports the medium",
        ctx(file)
    );

    let profile = InterpreterProfile::resolve(&story_path, None, None, None);
    assert_eq!(
        profile,
        if file.ends_with(".adf") { InterpreterProfile::Amiga } else { InterpreterProfile::IbmPc },
        "{}: the medium picks the machine",
        ctx(file)
    );
    let mut picts = PictSource::resolve(&story_path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        bytes,
        honor_game_colours,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", ctx(file)));
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    session.machine.set_palette(profile.palette());
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    session.set_strip_prompt(false);
    let _ = session.take_transcript();
    Some(session)
}

/// One driven turn's transcript, with a style byte per character.
struct Turn {
    text: String,
    styles: Vec<u8>,
}

impl Turn {
    /// The style of the trailing `>` read prompt, when this turn ended with one.
    fn prompt_style(&self) -> Option<u8> {
        (self.text.ends_with('>')).then(|| *self.styles.last().unwrap_or(&0))
    }

    /// The style of the first character of `needle`.
    fn style_at(&self, needle: &str) -> Option<u8> {
        let byte = self.text.find(needle)?;
        let chars = self.text[..byte].chars().count();
        self.styles.get(chars).copied()
    }
}

/// Drive `turns` turns of `look`, answering any keypress with Enter, and expand
/// each turn's style runs to one byte per character.
///
/// Several turns, because the report is that the FIRST prompt is clean and every
/// later one is not: the game only paints its status line — the window whose style
/// leaked — once the story proper has started.
fn drive(session: &mut GameSession, turns: usize) -> Vec<Turn> {
    (0..turns)
        .map(|_| {
            let r = match session.pending_input() {
                InputKind::Line => session.submit("look"),
                InputKind::Char => session.submit_char(13),
                InputKind::Event => session.submit(""),
            };
            let mut styles = Vec::with_capacity(r.transcript.chars().count());
            for (count, bits, ..) in &r.transcript_runs {
                styles.extend(std::iter::repeat_n(*bits, *count));
            }
            Turn { text: r.transcript, styles }
        })
        .collect()
}

/// The report itself.
///
/// Falsified by dropping the `screen.text_style` restore from `set_window`
/// (`crates/zvm/src/cpu/exec.rs`, VAR:0x0B):
///
/// ```text
/// James Clavell's Shogun.adf [release 295, serial 890321] (honor_game_colours=true):
/// the game's `>` read prompt is reverse video on turns [3, 4, 5, 6, 7, 8, 9, 10, 11,
/// 12, 13] — window 1's status-line style followed the game back into window 0
/// ```
#[test]
fn the_amiga_prompt_is_never_reverse_video() {
    for honor in [true, false] {
        let Some(mut session) = boot(AMIGA_RELEASE, honor) else { return };
        let turns = drive(&mut session, 13);

        let prompts: Vec<usize> =
            turns.iter().enumerate().filter(|(_, t)| t.prompt_style().is_some()).map(|(i, _)| i).collect();
        assert!(
            prompts.len() >= 8,
            "{} (honor_game_colours={honor}): only {} of 13 turns ended at the game's `>` — \
             the walkthrough never got into the story, so nothing here was measured",
            ctx(AMIGA_RELEASE),
            prompts.len()
        );

        let reversed: Vec<usize> = turns
            .iter()
            .enumerate()
            .filter(|(_, t)| t.prompt_style().is_some_and(|s| s & REVERSE != 0))
            .map(|(i, _)| i + 1)
            .collect();
        assert!(
            reversed.is_empty(),
            "{} (honor_game_colours={honor}): the game's `>` read prompt is reverse video on \
             turns {reversed:?} — window 1's status-line style followed the game back into \
             window 0",
            ctx(AMIGA_RELEASE)
        );
    }
}

/// The same leak, one layer wider: NOTHING window 0 printed asked for reverse
/// video, so nothing in the prose may carry it. A fix that only special-cased the
/// prompt character would leave the opening paragraph and the death notice
/// inverted, which is what the floppy actually looked like.
#[test]
fn no_amiga_prose_is_reverse_video() {
    for honor in [true, false] {
        let Some(mut session) = boot(AMIGA_RELEASE, honor) else { return };
        let turns = drive(&mut session, 13);

        for (i, turn) in turns.iter().enumerate() {
            let reversed: String = turn
                .text
                .chars()
                .zip(turn.styles.iter())
                .filter(|(_, s)| *s & REVERSE != 0)
                .map(|(c, _)| c)
                .collect();
            assert!(
                reversed.is_empty(),
                "{} (honor_game_colours={honor}): turn {} printed {} characters of window-0 \
                 prose in reverse video that the game never asked for: {:?}",
                ctx(AMIGA_RELEASE),
                i + 1,
                reversed.chars().count(),
                reversed.chars().take(60).collect::<String>()
            );
        }
    }
}

/// A room heading is BOLD. It was bold-and-reversed on the floppy and bold on the
/// story file, which is the sharpest statement of the defect available: two builds
/// of one game that agree about everything else disagreed about this only because
/// one of them declares a different machine.
#[test]
fn both_builds_style_a_room_heading_the_same_way() {
    let mut seen = 0;
    for file in [AMIGA_RELEASE, PC_RELEASE] {
        for honor in [true, false] {
            let Some(mut session) = boot(file, honor) else { continue };
            let turns = drive(&mut session, 13);
            let Some(style) = turns.iter().find_map(|t| t.style_at("Bridge")) else {
                panic!("{} (honor_game_colours={honor}): never reached the Bridge", ctx(file))
            };
            assert_eq!(
                style,
                BOLD,
                "{} (honor_game_colours={honor}): the `Bridge` heading must be bold and \
                 nothing else — got {style:#04x}",
                ctx(file)
            );
            seen += 1;
        }
    }
    assert!(seen == 0 || seen == 4, "both builds must be measured, or neither: {seen} of 4");
}

/// The control. Shogun asks for reverse video every turn — in WINDOW 1, for its
/// status line — and that request has to be honoured, or the "fix" is just a
/// deleted feature.
#[test]
fn the_status_window_really_is_reverse_video() {
    let Some(mut session) = boot(AMIGA_RELEASE, true) else { return };
    let _ = drive(&mut session, 13);

    let v6 = session.machine.screen.v6.as_ref().expect("a v6 story has the eight-window model");
    assert_eq!(
        v6.windows[1].text_style & REVERSE as u16,
        REVERSE as u16,
        "{}: window 1 holds the reverse video the game set for its status line \
         (ZMSD §8.8.3.2, property 10) — got {:#06x}",
        ctx(AMIGA_RELEASE),
        v6.windows[1].text_style
    );
    assert_eq!(
        v6.windows[0].text_style & REVERSE as u16,
        0,
        "{}: …and window 0 does not, which is the whole of the fix",
        ctx(AMIGA_RELEASE)
    );
}
