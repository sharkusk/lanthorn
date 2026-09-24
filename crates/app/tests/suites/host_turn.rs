//! SQ-1538: a turn is applied through the library, by the rules the TUI uses,
//! with no terminal and no audio device.
//!
//! `app::host::finish_command_turn` / `apply_game_driven_result` are the TUI's
//! own per-turn apply, moved into the library; `map_view: None` is a host with
//! no map pane, and sound goes to whatever `SoundSink` the host installed.
//!
//! | fixture | what | where |
//! |---|---|---|
//! | `zork1-r88-s840726.z3` | a nine-command walk into the grue | `stories/` (skips when absent) |
//! | a hand-assembled v5 story | `@sound_effect` with a finish routine | built below |

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use app::config::Config;
use app::engine::{Engine, KeyInput};
use app::host::sound::{SampleLevel, SoundId, SoundSink};
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts, TurnOutcome};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;
use mapper::direction::Direction;

use crate::fixture_paths::fixture_path;

fn headless_config(home: &Path) -> Config {
    Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        enable_sound: true,
        ..Config::default()
    }
}

fn boot(story: PathBuf, home: &Path) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg: headless_config(home),
        data_base: home.join("saves"),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

/// Submit `cmd` and apply the turn exactly as the TUI's submit path does.
fn command(b: &mut BootedStory, cmd: &str, tidy: &mut u32) -> TurnOutcome {
    let result = b.session.submit(cmd);
    app::host::finish_command_turn(
        cmd,
        true,
        result,
        &mut b.state,
        &mut b.mapper,
        &mut *b.session,
        &b.game_dir,
        &b.ifid,
        &b.arc_file,
        None,
        tidy,
    )
}

fn room_named(b: &BootedStory, name: &str) -> Option<mapper::graph::RoomId> {
    b.mapper.graph.rooms().find(|r| r.name == name).map(|r| r.id)
}

// ── Zork I: the transcript, the map and the death watch ────────────────────────

/// Zork I r88 / s840726, booted headlessly and walked nine commands: round the
/// house, in at the kitchen window, down the trap door into the dark Cellar, and
/// north into the grue. Every turn goes through the library's
/// `finish_command_turn` with no map pane and no terminal.
#[test]
fn a_scripted_zork_walk_builds_the_transcript_the_map_and_the_death_watch() {
    let story = fixture_path("zork1-r88-s840726.z3");
    if !story.is_file() {
        eprintln!("SKIP: {} absent", story.display());
        return;
    }
    let home = app::scratch_dir("host-turn-zork");
    let mut b = boot(story, &home);
    let mut tidy = 0u32;
    let walk = [
        "north", "east", "open window", "enter window", "west", "move rug", "open trap door",
        "down",
    ];
    for cmd in walk {
        let before = b.state.transcript.len();
        let out = command(&mut b, cmd, &mut tidy);
        assert!(!out.quit, "{cmd}: the game goes on");
        assert!(b.state.transcript.len() > before, "{cmd}: the transcript grows");
        assert_eq!(
            b.state.transcript_runs.len(),
            b.state.transcript.len(),
            "{cmd}: every line carries its style runs"
        );
        // A line read is pending and nothing vetoes [more], so a paginating host
        // pauses at this turn's first line, which is the game's reply.
        assert!(out.paging.would_arm && !out.paging.more_suppressed, "{cmd}: {:?}", out.paging);
        // At or after what was there — or ON the prompt line itself, when the game's
        // reply opened with the command's own words and was folded onto it.
        assert!(
            out.paging.first_line + 1 >= before && out.paging.first_line < b.state.transcript.len(),
            "{cmd}: the reply starts where this turn's output does: before={before} {:?}",
            out.paging,
        );
        assert_eq!(b.session.pending_input(), InputKind::Line);
    }
    for name in ["West of House", "North of House", "Behind House", "Kitchen", "Living Room", "Cellar"] {
        assert!(room_named(&b, name).is_some(), "the map gained {name}");
    }
    let cellar = room_named(&b, "Cellar").expect("the Cellar is mapped");
    assert_eq!(b.mapper.graph.current(), Some(cellar), "and the player stands in it");
    let living = room_named(&b, "Living Room").unwrap();
    assert!(
        b.mapper.graph.connections().iter().any(|c| c.origin == living && c.dir == Direction::Down && c.dest == cellar),
        "the trap door is a passage down from the Living Room"
    );

    // North into the dark: a grue, and the resurrection in the Forest.
    let out = command(&mut b, "north", &mut tidy);
    assert!(!out.quit, "Zork I resurrects the player rather than ending");
    let text = b.state.transcript.join("\n");
    assert!(text.contains("You have died"), "premise: the grue killed the player: {text}");
    assert!(
        !b.mapper.graph.is_tried(cellar, Direction::N),
        "the direction that killed the player is rolled back off the Cellar's record (SQ-0671)"
    );
    assert!(
        !b.mapper.graph.connections().iter().any(|c| c.origin == cellar && c.dir == Direction::N),
        "and no passage north was minted out of a death"
    );
    let _ = std::fs::remove_dir_all(&home);
}

// ── Sound: a recording sink, and a finish routine reported back ────────────────

#[derive(Debug, Clone, PartialEq)]
enum Heard {
    Tone { freq_hz: u32, z_volume: u8 },
    Play { resource: u32, bytes: usize, level: SampleLevel, repeats: u8, id: SoundId },
    Stop(SoundId),
    Pause(SoundId),
    Unpause(SoundId),
    Gain(SoundId, u32),
    StopAll,
    Volume(u8),
}

/// A sink that plays nothing and writes down everything, the way a host that
/// forwards sound elsewhere would.
#[derive(Debug, Default)]
struct Recorder {
    log: Rc<RefCell<Vec<Heard>>>,
    next: SoundId,
}

impl SoundSink for Recorder {
    fn tone(&mut self, freq_hz: f32, _ms: u32, z_volume: u8) {
        self.log.borrow_mut().push(Heard::Tone { freq_hz: freq_hz as u32, z_volume });
    }
    fn play(&mut self, s: app::host::sound::SampleStart<'_>) -> Option<SoundId> {
        self.next += 1;
        self.log.borrow_mut().push(Heard::Play {
            resource: s.resource,
            bytes: s.bytes.len(),
            level: s.level,
            repeats: s.repeats,
            id: self.next,
        });
        Some(self.next)
    }
    fn stop(&mut self, id: SoundId) {
        self.log.borrow_mut().push(Heard::Stop(id));
    }
    fn pause(&mut self, id: SoundId) {
        self.log.borrow_mut().push(Heard::Pause(id));
    }
    fn unpause(&mut self, id: SoundId) {
        self.log.borrow_mut().push(Heard::Unpause(id));
    }
    fn set_gain(&mut self, id: SoundId, gain: f32) {
        self.log.borrow_mut().push(Heard::Gain(id, (gain * 1000.0) as u32));
    }
    fn stop_all(&mut self) {
        self.log.borrow_mut().push(Heard::StopAll);
    }
    fn set_volume(&mut self, volume: u8) {
        self.log.borrow_mut().push(Heard::Volume(volume));
    }
    fn finished(&mut self) -> Vec<SoundId> {
        Vec::new()
    }
}

fn install_recorder(b: &mut BootedStory) -> Rc<RefCell<Vec<Heard>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    b.state.audio = Some(Box::new(Recorder { log: log.clone(), next: 0 }));
    log
}

/// A v5 story that waits for a key, then plays sample 3 with a finish routine and
/// high bleep 1, then waits again. The routine stores 42 into global 1 (`G1`),
/// which is how a test sees that it ran.
///
/// ```text
/// 0x40  read_char 1 -> G0
/// 0x44  sound_effect 3 2 8 routine=0x0200 (packed 0x80)
/// 0x4B  sound_effect 1
/// 0x4E  read_char 1 -> G0
/// 0x52  quit
/// 0x200 routine: store G1 42 ; rtrue
/// ```
fn sound_story() -> Vec<u8> {
    let mut buf = vec![0u8; 0x0800];
    buf[0x00] = 5;
    buf[0x04] = 0x04; // high memory base 0x0400
    buf[0x06] = 0x00;
    buf[0x07] = 0x40; // initial PC
    buf[0x08] = 0x04;
    buf[0x09] = 0x00; // dictionary 0x0400 (empty), in static memory as a story's is
    buf[0x401] = 4;
    buf[0x0A] = 0x01; // object table 0x0100
    buf[0x0C] = 0x03; // globals 0x0300
    buf[0x0E] = 0x04; // static memory 0x0400
    buf[0x12..0x18].copy_from_slice(b"260923"); // serial: printable, as the story sniffer requires
    buf[0x18] = 0x00;
    buf[0x19] = 0x60; // abbreviations
    let code: &[u8] = &[
        0xF6, 0x7F, 0x01, 0x10, // read_char 1 -> G0
        0xF5, 0x54, 0x03, 0x02, 0x08, 0x00, 0x80, // sound_effect 3 2 8 routine
        0xF5, 0x7F, 0x01, // sound_effect 1
        0xF6, 0x7F, 0x01, 0x10, // read_char 1 -> G0
        0xBA, // quit
    ];
    buf[0x40..0x40 + code.len()].copy_from_slice(code);
    buf[0x200..0x205].copy_from_slice(&[0x00, 0x0D, 0x11, 42, 0xB0]); // 0 locals; store G1 42; rtrue
    buf
}

/// A one-sound resource Blorb: `Snd ` 3, an AIFF-typed chunk the recorder never decodes.
fn one_sound_blorb() -> blorb::Blorb {
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = ty.to_vec();
        c.extend_from_slice(&(data.len() as u32).to_be_bytes());
        c.extend_from_slice(data);
        if data.len() % 2 == 1 {
            c.push(0);
        }
        c
    }
    let sample = chunk(b"FORM", b"AIFFsample");
    let ridx_len = 4 + 12;
    let first = 12 + 8 + ridx_len;
    let mut ridx = 1u32.to_be_bytes().to_vec();
    ridx.extend_from_slice(b"Snd ");
    ridx.extend_from_slice(&3u32.to_be_bytes());
    ridx.extend_from_slice(&(first as u32).to_be_bytes());
    let mut inner = b"IFRS".to_vec();
    inner.extend_from_slice(&chunk(b"RIdx", &ridx));
    inner.extend_from_slice(&sample);
    let mut file = b"FORM".to_vec();
    file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
    file.extend_from_slice(&inner);
    blorb::Blorb::parse(file).expect("a well-formed one-sound Blorb")
}

/// The keypress that starts the sounds is applied through the library's
/// game-driven path; the sink hears exactly what the TUI's device would be told
/// — the sample with its volume and repeats, then the bleep — and reporting the
/// sample finished runs the story's own finish routine.
#[test]
fn a_recording_sink_hears_the_turns_sounds_and_a_finish_runs_the_routine() {
    let home = app::scratch_dir("host-turn-sound");
    let story = home.join("sound.z5");
    std::fs::write(&story, sound_story()).unwrap();
    let mut b = boot(story, &home);
    b.state.sound_blorb = Some(one_sound_blorb());
    let log = install_recorder(&mut b);
    assert_eq!(b.session.pending_input(), InputKind::Char, "premise: the story waits for a key");

    let result = b.session.submit_key(KeyInput::Char('x')).expect("the key reaches the story");
    let events: Vec<(u16, u8, u8, u8, u16)> =
        result.sounds.iter().map(|e| (e.number, e.effect, e.volume, e.repeats, e.routine)).collect();
    assert_eq!(events.len(), 2, "the engine reported both sounds: {events:?}");
    let out = app::host::apply_game_driven_result(
        &mut b.state,
        &mut b.mapper,
        &result,
        &b.game_dir,
        None,
        &*b.session,
        app::pager::Driver::PlayerInput,
    );
    assert!(!out.quit);

    let heard = log.borrow().clone();
    let (sample, bleep) = (&result.sounds[0], &result.sounds[1]);
    assert_eq!(
        heard,
        vec![
            Heard::Play {
                resource: 3,
                bytes: 8 + b"AIFFsample".len(), // the whole FORM chunk, header included, as an AIFF file is
                level: SampleLevel::ZVolume(sample.volume),
                repeats: sample.repeats,
                id: 1,
            },
            Heard::Tone { freq_hz: 800, z_volume: bleep.volume },
        ],
        "the sink is told what to play, in the story's order",
    );
    assert_eq!(b.state.sound_routines.get(&1), Some(&sample.routine), "the routine waits on the sample");

    let g1 = |b: &BootedStory| app::engine_helpers::zvm_session_opt(&*b.session).unwrap().machine.global(1);
    assert_eq!(g1(&b), 0, "premise: the routine has not run");
    let quit = app::host::sound::sound_finished(&mut b.state, &mut b.mapper, &mut *b.session, 1, &b.game_dir, None);
    assert!(!quit);
    assert_eq!(g1(&b), 42, "reporting the sample finished ran the story's finish routine");
    assert!(b.state.sound_routines.is_empty() && b.state.sound_ids.is_empty(), "and it is forgotten");
    let _ = std::fs::remove_dir_all(&home);
}
