//! SQ-1303: an Inform 7 Glulx story's own compiled world model as the room source.
//!
//! # What changed
//!
//! Glulx has no object tree, so lanthorn recovered a room's IDENTITY by learning which RAM word
//! holds the `location` global (SQ-0526/SQ-1286) and its NAME from whatever the story printed —
//! a `Subheader` heading, a silent `look` (SQ-1293), the status grid (SQ-1302). Both are
//! observations, and both cost turns: measured on `CounterfeitMonkey-11.gblorb`, the lock did not
//! resolve until the **tenth** command, and every room reached before that was keyed by the hash
//! of a heading and had to be re-keyed afterwards.
//!
//! An Inform 7 story has already written the answers down. `gvm::i7map` reads its compiled world
//! model straight off the image — which objects are rooms, what each is called, and which room
//! each direction leads to — with no turn played. `GlulxSession` builds it once at boot and uses
//! it for four things, all of them pinned below:
//!
//! * the room the player starts in is keyed by its own object ADDRESS from the first prompt,
//!   before the lock has learned anything at all;
//! * a room's NAME comes from its `printed name` property rather than from the turn's heading, so
//!   one room cannot become two nodes because the story spelled it two ways;
//! * the lock resolves on the FIRST move, by matching the room a candidate word just changed to
//!   against the heading the story just printed;
//! * and `Engine::declared_exit` — which only the Inform 6 `door_dir` convention could answer for
//!   before, i.e. never for an I7 story — answers from `Map_Storage`.
//!
//! # The fixtures, and what each one is here to prove
//!
//! All three are gitignored commercial media, so every case here skips vacuously without them.
//!
//! * **`CounterfeitMonkey-11.gblorb`** — release 11 / serial 230220 / Inform 7 build 6M62. The
//!   reported game, 100 rooms, and the one the reader was developed against.
//! * **`The_Wizard_Sniffer.gblorb`** — release 1 / serial 171007 / Inform 7 build 6L38. The story
//!   that prints no heading ANYWHERE (SQ-1302) and is named off its status grid: proof that the
//!   static world serves the grid path as well as the heading path.
//! * **`Kerkerkruip.gblorb`** — deals its dungeon at run time, so its compiled `Map_Storage` is
//!   all zeros and `I7World::detect` refuses it. It is the FALLBACK proof: with no world model,
//!   every id, name and lock must be exactly the ones the pre-SQ-1303 code produced, and
//!   [`kerkerkruip_maps_exactly_as_it_did_before_the_world_model_existed`] pins a dump taken from
//!   the tree before any of this was written.
//!
//! `AnchorheadDemo.gblorb` is a second refusal (Inform 7 build 4K41, older than `Map_Storage`)
//! and its non-regression is `sq1286_glulx_room_lock` and `sq1304_anchorhead_twisting_lane`,
//! which are unchanged by this quest; one case here checks it still declares no exits.
//!
//! Every route below passes a fixed `random_seed`, because Kerkerkruip's route depends on the
//! dungeon it deals and a route that walks a different dungeon each run can pin nothing.

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::roomid::synthetic_room_id;
use app::session::{apply_turn, DeathWatch, InputKind};
use mapper::direction::Direction;
use mapper::mapper::Mapper;

use crate::fixture_paths::fixture_path;

const CM: &str = "CounterfeitMonkey-11.gblorb";
const WIZARD: &str = "The_Wizard_Sniffer.gblorb";
const KERKERKRUIP: &str = "Kerkerkruip.gblorb";
const ANCHORHEAD: &str = "AnchorheadDemo.gblorb";

/// The seed every route here runs under — see the module docs.
const SEED: u32 = 1234;

/// One step of a route: a typed command, or a single keypress.
enum Step {
    Cmd(&'static str),
    Key(char),
}

use Step::{Cmd, Key};

/// Boot a Glulx blorb with a private store and a fixed RNG seed. `None` when the gitignored
/// fixture is absent, which is how every case here skips vacuously.
fn boot(name: &str, tag: &str) -> Option<GlulxSession> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let blorb = blorb::Blorb::parse(bytes).expect("a readable blorb");
    let (kind, exec) = blorb.executable().expect("an executable chunk");
    assert_eq!(kind, blorb::ExecKind::Glulx, "{name} is a Glulx blorb");
    let store = app::scratch_dir(tag);
    let s = GlulxSession::new_in(
        store,
        exec.to_vec(),
        80,
        30,
        true,
        false,
        false,
        false,
        (8, 16),
        None,
        &[],
        [[(None, None); 11]; 2],
        false,
        Some(SEED),
    )
    .unwrap_or_else(|e| panic!("{name} boots: {e:?}"));
    Some(s)
}

/// Where this turn's room id comes from: `Named` while it is nothing but the hash of the room's
/// printed name, `Object` once it carries the room's own address.
#[derive(Debug, PartialEq, Eq)]
enum Keying {
    Named,
    Object,
}

/// `(name, id, keying)` for the room the session currently believes it is in.
fn here(s: &GlulxSession) -> Option<(String, mapper::graph::RoomId, Keying)> {
    let l = s.current_location()?;
    let k = if l.number == synthetic_room_id(&l.name) { Keying::Named } else { Keying::Object };
    Some((l.name, l.number, k))
}

/// Play one step, draining any keypress page or non-input event the story parks on first.
///
/// Kerkerkruip's title card animates on a Glk timer and its menu reads single keys, so a route
/// that only knows how to type would stall on the first frame.
fn step(s: &mut GlulxSession, st: &Step) -> app::session::TurnResult {
    match st {
        Key(c) => {
            assert_eq!(s.pending_input(), InputKind::Char, "this step wants a keypress");
            let k = if *c == '\n' { KeyInput::Enter } else { KeyInput::Char(*c) };
            s.submit_key(k).expect("Glulx takes keys")
        }
        Cmd(cmd) => {
            for _ in 0..400 {
                match s.pending_input() {
                    InputKind::Char => {
                        let _ = s.submit_key(KeyInput::Enter);
                    }
                    InputKind::Event => {
                        let _ = s.deliver_timer();
                    }
                    InputKind::Line => break,
                }
            }
            assert_eq!(s.pending_input(), InputKind::Line, "step {cmd:?} wants a line prompt");
            s.submit(cmd)
        }
    }
}

// ── Counterfeit Monkey ──────────────────────────────────────────────────────

/// The prologue and the first move: `y` (the consent that begins play), `andra` (the name
/// question), the keypress before the banner, `look`, then north out of the Back Alley.
const CM_OPENING: &[Step] = &[Cmd("y"), Cmd("andra"), Key('\n'), Cmd("look"), Cmd("n")];

#[test]
fn counterfeit_monkey_is_address_keyed_from_turn_zero_and_locks_on_the_first_move() {
    let Some(mut s) = boot(CM, "sq1303-cm-opening") else { return };

    // ── Non-vacuity: the reader really did read THIS story ───────────────────
    let world = s.i7_world().expect("Counterfeit Monkey's compiled world model is readable");
    assert_eq!(world.rooms().len(), 100, "release 11's room count; the fixture has changed");

    // ── The prologue, which prints no room heading at all (SQ-1293) ──────────
    for st in &CM_OPENING[..3] {
        step(&mut s, st);
    }
    let (name, opening, k) = here(&s).expect("the player is standing in a room");
    assert_eq!(name, "Back Alley", "the room Counterfeit Monkey opens in");
    assert_eq!(
        k,
        Keying::Object,
        "the opening room is keyed by its own address before a single command is typed, \
         not by the hash of its name (#{opening} vs #{})",
        synthetic_room_id(&name)
    );
    assert!(
        s.locked_room_global().is_none(),
        "…and the lock has not resolved yet, so that identity came from the story's own world \
         model and nowhere else"
    );

    // ── One move, and the lock lands ─────────────────────────────────────────
    step(&mut s, &CM_OPENING[3]); // look — a repeated heading says nothing
    assert!(s.locked_room_global().is_none(), "a `look` is not a move and cannot lock anything");
    let north = step(&mut s, &CM_OPENING[4]);
    assert_eq!(
        north.location.as_ref().map(|l| l.name.as_str()),
        Some("Sigil Street"),
        "north from the Back Alley is Sigil Street"
    );
    assert!(
        s.locked_room_global().is_some(),
        "SQ-1303: one move north is enough — the word that changed to a room the story calls \
         `Sigil Street` on the turn it printed that heading IS the `location` global"
    );

    // ── And the two identities agree, which is the whole point ───────────────
    let south = s.submit("s");
    assert_eq!(
        south.location.as_ref().map(|l| l.number),
        Some(opening),
        "walking back in lands on the node the boot recorded — the address the world model \
         gave before the lock resolved is the address the lock now reads"
    );
}

/// The SQ-1285 route: five rooms, a puzzle, and the `get all` that used to mint a phantom room
/// named after a taken object. Nothing here may share a node with anything else.
const CM_WALK: &[Step] = &[
    Cmd("y"),
    Cmd("andra"),
    Key('\n'),
    Cmd("tutorial off"),
    Cmd("pauses off"),
    Cmd("n"),                       // Back Alley → Sigil Street
    Cmd("e"),                       // Sigil Street → Ampersand Bend
    Cmd("wave x-remover at codex"), // the museum's codex → a code reading "305"
    Cmd("unlock barrier"),
    Cmd("n"), // Ampersand Bend → Fair
    Cmd("w"), // Fair → Midway
    Cmd("s"), // Midway → Ampersand Bend
    Cmd("w"), // Ampersand Bend → Sigil Street
    Cmd("s"), // Sigil Street → Back Alley
];

#[test]
fn no_two_nodes_share_a_name_across_the_whole_walk() {
    let Some(mut s) = boot(CM, "sq1303-cm-walk") else { return };
    let mut mapper = Mapper::default();
    let mut death = DeathWatch::default();
    let mut remaps = 0;

    for st in CM_WALK {
        let label = match st {
            Cmd(c) => *c,
            Key(_) => "<key>",
        };
        let r = step(&mut s, st);
        // Drive the same re-key `app::turn` performs every turn, and count the ones that
        // actually landed on a node — the lock always hands back the rooms it saw while
        // learning, whether or not any of them was ever MAPPED under a name.
        for (name, addr) in s.take_room_remap() {
            if mapper.rekey_room(synthetic_room_id(&name), app::roomid::glulx_room_id(addr)) {
                remaps += 1;
            }
        }
        apply_turn(&mut mapper, label, &r, &mut death);
    }

    // ── Non-vacuity: the route really did move through the town ──────────────
    let labels: Vec<String> = mapper.graph.rooms().map(|r| r.label().to_string()).collect();
    assert!(labels.len() >= 5, "the walk visits at least five rooms, mapped {labels:?}");
    for room in ["Back Alley", "Sigil Street", "Ampersand Bend", "Fair", "Midway"] {
        assert!(labels.iter().any(|l| l == room), "{room} is missing: {labels:?}");
    }

    // ── One node per room, and every one of them keyed by its address ────────
    let mut sorted = labels.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        labels.len(),
        "two nodes share a name, which is the duplicate this quest exists to stop: {labels:?}"
    );
    for r in mapper.graph.rooms() {
        assert_ne!(
            r.id,
            synthetic_room_id(r.label()),
            "{:?} is keyed by the hash of its name rather than by its address",
            r.label()
        );
    }
    assert_eq!(
        remaps, 0,
        "nothing on this walk was ever MAPPED under the hash of a name, so the lock's remap \
         finds no node to re-key when it lands"
    );
}

#[test]
fn counterfeit_monkeys_declared_exits_come_from_its_own_compiled_map() {
    let Some(mut s) = boot(CM, "sq1303-cm-exits") else { return };
    for st in &CM_OPENING[..3] {
        step(&mut s, st);
    }
    let (alley_name, alley, _) = here(&s).expect("the prologue ends in a room");
    assert_eq!(alley_name, "Back Alley");
    step(&mut s, &Cmd("n"));
    let (name, sigil, _) = here(&s).expect("north reaches Sigil Street");
    assert_eq!(name, "Sigil Street");
    step(&mut s, &Cmd("e"));
    let (bend_name, bend, _) = here(&s).expect("east reaches Ampersand Bend");
    assert_eq!(bend_name, "Ampersand Bend");
    step(&mut s, &Cmd("w"));
    assert_eq!(here(&s).map(|(_, id, _)| id), Some(sigil), "west comes back to Sigil Street");

    // The claim: what the compiled map declares for this room is what the player actually
    // walked. The expected ids are the WALKED ones, so this compares two independent
    // derivations of the same passage rather than the model against itself.
    use app::engine::DeclaredExit as E;
    assert_eq!(
        Engine::declared_exit(&s, sigil, Direction::E),
        E::Room(bend),
        "Sigil Street runs east to Ampersand Bend, and `Map_Storage` says so — the same room \
         the player reached by typing `e`"
    );
    assert_eq!(
        Engine::declared_exit(&s, sigil, Direction::S),
        E::Room(alley),
        "…and south back into the alley the prologue started in"
    );
    assert_eq!(
        Engine::declared_exit(&s, sigil, Direction::Up),
        E::Absent,
        "`up` is a direction this story HAS and this room declares nothing for — which is a \
         different answer from not knowing"
    );

    // The answer really is coming from the I7 map: this story's Inform 6 `door_dir` convention
    // is absent (that is not what its compiler emits), so the pre-SQ-1303 path had nothing.
    assert_eq!(
        Engine::declared_exit(&s, 0xdead_beef, Direction::E),
        E::Unknown,
        "a room this session has never stood in cannot be asked about"
    );
}

// ── The Wizard Sniffer: the status-grid path, served by the same model ──────

/// SQ-1302's route: two keypresses clear the prologue card and the banner, `sniff rope` drops the
/// drawbridge, and two moves north walk into the baileys.
const WIZARD_ROUTE: &[Step] =
    &[Key('\n'), Key('\n'), Cmd("sniff rope"), Cmd("n"), Cmd("n")];

#[test]
fn the_wizard_sniffer_takes_its_ids_and_names_from_its_own_world_model() {
    let Some(mut s) = boot(WIZARD, "sq1303-wizard") else { return };
    let world = s.i7_world().expect("The Wizard Sniffer's compiled world model is readable");
    assert_eq!(world.rooms().len(), 40, "release 1's room count; the fixture has changed");

    let mut seen: Vec<(String, Keying, bool)> = Vec::new();
    let mut transcripts = Vec::new();
    for st in WIZARD_ROUTE {
        let r = step(&mut s, st);
        transcripts.push(r.transcript.clone());
        if let Some((n, _, k)) = here(&s) {
            seen.push((n, k, s.locked_room_global().is_some()));
        }
    }

    // ── Non-vacuity: this really is the story that prints no heading, so the name below can
    //    only have come from the status grid (SQ-1302) ─────────────────────────
    for (i, t) in transcripts.iter().enumerate() {
        for room in ["Atop a Mountain", "Southern Bailey", "Centre Bailey"] {
            assert!(
                !t.contains(room),
                "turn {i} printed {room:?} in the BUFFER; this story is supposed to name its \
                 rooms only on the status line: {t:?}"
            );
        }
    }

    let names: Vec<&str> = seen.iter().map(|(n, _, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["Atop a Mountain", "Atop a Mountain", "Southern Bailey", "Centre Bailey"],
        "the route walks the mountain into the baileys"
    );
    assert!(
        seen.iter().all(|(_, k, _)| *k == Keying::Object),
        "every room on the route is keyed by its own address, the opening one included, \
         before anything is locked: {seen:?}"
    );
    assert_eq!(
        seen.iter().map(|(_, _, locked)| *locked).collect::<Vec<_>>(),
        [false, false, true, true],
        "…and the lock resolves on the first MOVE, not on the seventh turn it used to take"
    );
}

// ── The fallback: a story the reader refuses must be untouched ──────────────

/// Kerkerkruip's opening: `n` declines the screen-reader mode, SPACE starts a new game, and then
/// ten ordinary commands in the dealt dungeon.
const KERKERKRUIP_ROUTE: &[Step] = &[
    Key('n'),
    Key(' '),
    Cmd("look"),
    Cmd("north"),
    Cmd("south"),
    Cmd("wait"),
    Cmd("east"),
    Cmd("west"),
    Cmd("wait"),
    Cmd("north"),
    Cmd("down"),
    Cmd("up"),
];

/// What the pre-SQ-1303 tree produced on [`KERKERKRUIP_ROUTE`] at seed [`SEED`], captured before
/// a line of this quest's production code was written: `(room name, keyed by its address?,
/// locked yet?)` after each of the ten commands.
///
/// Kerkerkruip generates its dungeon at run time, so its compiled `Map_Storage` is all zeros and
/// `I7World::detect` refuses it (`gvm`'s own
/// `a_story_that_builds_its_map_at_run_time_is_refused_rather_than_guessed_at`). Every line here
/// is therefore the heading-first path — name hashes until the correlation resolves the lock on
/// the sixth command, addresses after — and it must stay exactly this.
const KERKERKRUIP_BEFORE: &[(&str, bool, bool)] = &[
    ("Entrance Hall", false, false),
    ("Entrance Hall", false, false),
    ("Entrance Hall", false, false),
    ("Entrance Hall", false, false),
    ("Phantasmagoria", false, false),
    ("Entrance Hall", true, true),
    ("Entrance Hall", true, true),
    ("Entrance Hall", true, true),
    ("Entrance Hall", true, true),
    ("Entrance Hall", true, true),
];

/// The `location` global this route locks onto, and the two room ids it produces — the literal
/// numbers from that same pre-change dump.
const KERKERKRUIP_GLOBAL: u32 = 1_443_004;
const KERKERKRUIP_HALL_NAMED: mapper::graph::RoomId = 4_200_397_318;
const KERKERKRUIP_HALL_ADDRESS: mapper::graph::RoomId = 3_819_564_611;

#[test]
fn kerkerkruip_maps_exactly_as_it_did_before_the_world_model_existed() {
    let Some(mut s) = boot(KERKERKRUIP, "sq1303-kerkerkruip") else { return };

    // ── Non-vacuity: this is the refusal, and it is a refusal at BOOT ────────
    assert!(
        s.i7_world().is_none(),
        "Kerkerkruip deals its dungeon at run time; there is no compiled map to read, and \
         reporting one would be reporting a coincidence"
    );

    let mut got: Vec<(String, bool, bool)> = Vec::new();
    let mut ids: Vec<mapper::graph::RoomId> = Vec::new();
    for st in KERKERKRUIP_ROUTE {
        step(&mut s, st);
        if matches!(st, Key(_)) {
            continue;
        }
        let (name, id, k) = here(&s).expect("the dungeon puts the player somewhere");
        got.push((name, k == Keying::Object, s.locked_room_global().is_some()));
        ids.push(id);
    }

    let expected: Vec<(String, bool, bool)> =
        KERKERKRUIP_BEFORE.iter().map(|&(n, o, l)| (n.to_string(), o, l)).collect();
    assert_eq!(
        got, expected,
        "SQ-1303 must be invisible to a story whose world model it refuses; this is the dump \
         taken from the tree before the change"
    );
    assert_eq!(
        s.locked_room_global(),
        Some(KERKERKRUIP_GLOBAL),
        "…onto the same `location` global, at the same turn"
    );
    assert_eq!(ids[0], KERKERKRUIP_HALL_NAMED, "the pre-lock Entrance Hall id is unchanged");
    assert_eq!(ids[9], KERKERKRUIP_HALL_ADDRESS, "and so is the post-lock one");
}

#[test]
fn a_story_with_no_compiled_map_declares_no_exits() {
    // The Anchorhead demo is Inform 7 build 4K41, older than `Map_Storage`, and carries no
    // Inform 6 `door_dir` convention either — so both halves of `declared_exit` refuse and the
    // answer is the `Unknown` it has always been. `sq1286`/`sq1304` cover the rest of what this
    // fixture must keep doing.
    let Some(mut s) = boot(ANCHORHEAD, "sq1303-anchorhead") else { return };
    assert!(s.i7_world().is_none(), "build 4K41 carries no instance-count properties at all");
    let _ = step(&mut s, &Cmd("look"));
    let (_, id, _) = here(&s).expect("the demo opens in a room");
    for dir in [Direction::N, Direction::S, Direction::E, Direction::W] {
        assert_eq!(
            Engine::declared_exit(&s, id, dir),
            app::engine::DeclaredExit::Unknown,
            "{dir:?}: a story with neither convention declares nothing, which is not the same \
             as declaring there is no exit"
        );
    }
}
