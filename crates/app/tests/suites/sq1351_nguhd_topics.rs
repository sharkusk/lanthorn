//! SQ-1351: *Never Gives Up Her Dead*, and the conversation topics that became rooms.
//!
//! # The report
//!
//! `stories/Never Gives Up Her Dead.gblorb` — Brian Rushton (Mathbrush), Inform 7, Glulx.
//! *"Playing it in lanthorn, conversation topics end up as rooms on the map."*
//!
//! # The mechanism
//!
//! The game's topic system answers `TOPICS` (or `T`, or `TALK TO <someone>`) with a list under a
//! `Subheader` banner — `"Things to say to Gareth"` — printed on its own line and set flush
//! against the topics beneath it. That is exactly the shape an Inform 7 room heading has, and
//! every test `crate::glk_backend::StoryScan` applies says room: a bold run that begins at line
//! start, owns its line, and is JOINED to the text below rather than detached from it by a blank
//! line. Measured on the opening two turns in, `TOPICS` minted *Things to say to Gareth* as a
//! node; and because the session's cached room is sticky across heading-less turns, the map then
//! stayed inside the topic for the whole conversation, so the first real move afterwards —
//! `west`, into the dark end of the storage room — was minted as a passage leading out of a
//! CONVERSATION.
//!
//! Nothing in the buffer can tell the two apart. What can is that the story is saying where the
//! player is somewhere else at the same time: this story paints the room name into its status
//! grid, and it read `" Storage Room"` on the very turn the banner said otherwise. So
//! [`app::glulx_session::GlulxSession::refuse_banner_the_status_line_contradicts`] drops a bold
//! banner the grid contradicts — but only while the grid is corroborating the room the map is
//! already in, which is what stops a status line full of chrome from ever getting a vote. A
//! genuine arrival repaints both together (`west` prints the `"Darkness"` heading with
//! `" Darkness"` in the grid) and never reaches the refusal at all; that is what
//! [`the_map_still_follows_a_real_move`] pins, and it is the falsifier for a refusal that reaches
//! too far.
//!
//! Every case skips vacuously without `stories/` (gitignored).

use std::path::PathBuf;

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::roomid::synthetic_room_id;
use app::session::{apply_turn, DeathWatch, InputKind};
use app::state::AppState;
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

/// The room the game opens in — the only place the player can be for the whole conversation.
const OPENING: &str = "Storage Room";
/// The banner the topic system prints, which is not a room and never was.
const TOPIC_BANNER: &str = "Things to say to Gareth";

fn story() -> Option<Vec<u8>> {
    let path = fixture_path("Never Gives Up Her Dead.gblorb");
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

/// Natural play through the REAL pipeline — `Engine::submit`, `apply_turn`, and
/// `random_exit_probe`'s own gate — the same harness shape `sq1315_anchorhead_2018` uses, and for
/// the same reason: the defect lives in what a player's own turns do to the map.
struct Play {
    state: AppState,
    mapper: Mapper,
    session: GlulxSession,
    death: DeathWatch,
    /// Every `(command, room label after the turn)`, for a failure message that explains itself.
    log: Vec<(String, String)>,
}

impl Play {
    fn boot(tag: &str) -> Option<Play> {
        let bytes = story()?;
        let blorb = blorb::Blorb::parse(bytes).ok()?;
        let (kind, exec) = blorb.executable().ok()?;
        assert_eq!(kind, blorb::ExecKind::Glulx, "Never Gives Up Her Dead is a Glulx blorb");
        let store: PathBuf = app::scratch_dir(tag);
        let mut s = GlulxSession::new_in(
            store,
            exec.to_vec(),
            80,
            24,
            true,
            false,
            false,
            false,
            (1, 1),
            None,
            &[],
            [[(None, None); 11]; 2],
            false,
            Some(1),
        )
        .unwrap_or_else(|e| panic!("Never Gives Up Her Dead boots: {e:?}"));
        for _ in 0..40 {
            if s.current_location().is_some() {
                break;
            }
            if s.pending_input() != InputKind::Char {
                break;
            }
            s.submit_key(KeyInput::Enter);
        }
        Some(Play {
            state: AppState::default(),
            mapper: Mapper::default(),
            session: s,
            death: DeathWatch::default(),
            log: Vec::new(),
        })
    }

    /// One turn, exactly as `turn::finish_command_turn` drives one. Returns the transcript.
    fn turn(&mut self, cmd: &str) -> String {
        for _ in 0..6 {
            if self.session.pending_input() != InputKind::Char {
                break;
            }
            self.session.submit_key(KeyInput::Enter);
        }
        if self.session.pending_input() != InputKind::Line {
            return String::new();
        }
        let room_before = self.mapper.graph.current();
        let mut result = Engine::submit(&mut self.session, cmd);
        result.declared_exit =
            app::random_exit_probe::declared_exit_for_command(cmd, room_before, |o, d| {
                Engine::declared_exit(&self.session, o, d)
            });
        for (name, addr) in self.session.take_room_remap() {
            self.mapper.rekey_room(synthetic_room_id(&name), app::roomid::glulx_room_id(addr));
        }
        apply_turn(&mut self.mapper, cmd, &result, &mut self.death);
        app::random_exit_probe::arm_for_finished_turn(
            &mut self.state,
            &self.session,
            &mut self.mapper,
            cmd,
            room_before,
            result.declared_exit,
        );
        app::random_exit_probe::settle_random_exit_search(&mut self.state, &mut self.mapper);
        self.log.push((cmd.to_string(), self.here()));
        result.transcript
    }

    /// What the map calls the room the player is standing in.
    fn here(&self) -> String {
        self.mapper
            .graph
            .current()
            .and_then(|id| self.mapper.graph.room(id))
            .map(|r| r.label().to_string())
            .unwrap_or_default()
    }

    /// Every room on the map, sorted — the whole of what this quest is about.
    fn rooms(&self) -> Vec<String> {
        let mut v: Vec<String> = self.mapper.graph.rooms().map(|r| r.label().to_string()).collect();
        v.sort();
        v
    }

    /// The map as a dump would show it, plus the route that produced it.
    fn picture(&self) -> String {
        let mut out = String::from("  route:\n");
        for (cmd, room) in &self.log {
            out.push_str(&format!("    {cmd:16} -> {room}\n"));
        }
        let mut rooms: Vec<_> = self.mapper.graph.rooms().collect();
        rooms.sort_by_key(|r| r.id);
        for r in rooms {
            let tag = if r.id == synthetic_room_id(r.label()) { "  <== NAME-KEYED" } else { "" };
            out.push_str(&format!("  ROOM #{} {:?}{tag}\n", r.id, r.label()));
        }
        for c in self.mapper.graph.connections() {
            out.push_str(&format!("  EDGE #{} {:?} #{}\n", c.origin, c.dir, c.dest));
        }
        out
    }
}

/// The opening conversation, turn for turn: the game calls over the recorder on the first turn,
/// `TOPICS` lists what there is to say, `RESPONSE` says it, and `T` lists again. Nothing here
/// moves the player one step — the story refuses to let them leave the storage room until they
/// have found the jacket — so the map must hold exactly one room at the end of it.
#[test]
fn a_conversation_mints_no_rooms() {
    let Some(mut p) = Play::boot("sq1351-topics") else { return };
    p.turn("look");
    let opening = vec![OPENING.to_string()];
    assert_eq!(p.rooms(), opening, "the opening room, and only it\n{}", p.picture());

    // The banner must actually be printed, or this case would pass on a fixture that no longer
    // has a topic system at all — a vacuous green is exactly what it exists to rule out.
    let listed = p.turn("topics");
    assert!(
        listed.contains(TOPIC_BANNER),
        "the topic list still prints its {TOPIC_BANNER:?} banner; the fixture has changed\n{listed}"
    );

    let before = p.rooms();
    for cmd in ["response", "t", "topics"] {
        p.turn(cmd);
    }
    assert_eq!(p.rooms(), before, "a topic exchange mints no rooms\n{}", p.picture());
    assert_eq!(
        p.rooms(),
        opening,
        "the map holds the storage room and nothing else\n{}",
        p.picture()
    );
    assert_eq!(p.here(), OPENING, "the player never left the storage room\n{}", p.picture());
}

/// The other half: a refusal that reached too far would take the real headings with it. The dark
/// west end of the storage room is one `west` away, and the game announces it in the buffer and
/// in the grid together — so the map follows the player there and back.
#[test]
fn the_map_still_follows_a_real_move() {
    let Some(mut p) = Play::boot("sq1351-move") else { return };
    for cmd in ["look", "topics", "response"] {
        p.turn(cmd);
    }
    let dark = p.turn("west");
    assert!(dark.contains("Darkness"), "west reaches the dark end of the room\n{dark}");
    assert_eq!(p.here(), "Darkness", "the map went west with the player\n{}", p.picture());
    p.turn("east");
    assert_eq!(p.here(), OPENING, "and came back\n{}", p.picture());
    assert_eq!(
        p.rooms(),
        vec!["Darkness".to_string(), OPENING.to_string()],
        "two rooms, both of them places\n{}",
        p.picture()
    );
}
