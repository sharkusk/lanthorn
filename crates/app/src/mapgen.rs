//! Static map generation: a story's COMPLETE map, read out of the story file
//! itself, with nothing ever played (SQ-1306).
//!
//! The live automapper builds its graph from what the player has actually seen,
//! which is the only honest thing for a map the player is reading. This is the
//! other question — "what does this story file DECLARE?" — and it is a
//! developer's question, not a player's: it produces the reference maps our
//! layout tests measure against, and 100-room graphs to stress the router with.
//!
//! Four sources, one per way a story can spell its map, and each one is read by
//! the module that already knows how (this file adds no format knowledge of its
//! own — see [`crate::mapgen`]'s per-source functions for exactly which call
//! answers each question):
//!
//! | source | engine | reader | rooms come from |
//! |---|---|---|---|
//! | `i7-world` | Glulx | [`gvm::i7map::I7World`] | `I7World::rooms()` — the story's own `Map_Storage` row order |
//! | `i6-library` | Glulx | [`gvm::world::WorldModel`] | objects that declare an exit, plus every object one leads to |
//! | `i6-library` | Z-machine | [`zvm::world::WorldModel`] | the same derivation, over object numbers |
//! | `zil` | Z-machine | [`zvm::world::WorldModel`] | the same derivation, over object numbers |
//! | `scott` | Scott Adams | [`scott::Database`] | `db.rooms`, complete by construction |
//!
//! # What a static map is not
//!
//! **It is the map as COMPILED, and a story may edit its own map as it runs.**
//! Inform 7's `AssertMapConnection`, Inform 6's `door_dir` pointing at a
//! routine, and ZIL's FEXIT all decide at run time; none of them leaves a fact
//! in the story file for anything here to read. So a static map can be missing
//! passages a player would find, and — where a story dismantles a connection —
//! can show one a player never can. Phase 2 (a headless walker) is the answer
//! to that and is a separate quest; nothing here is built for it.
//!
//! **A conditional exit is shown, and marked.** ZIL's CEXIT gates a real,
//! static destination on a global variable. Dropping it loses genuine passages
//! (Zork I's grating and trap door are CEXITs), so the edge is drawn and marked
//! [`EdgeKind::Conditional`] — see [`zvm::world::ExitDetail`], which exists so
//! this can be told from a routine nothing can resolve.
//!
//! **A door is a passage, not a room** — except where the story will not say
//! what is on the far side. An I7 two-sided door resolves statically and
//! becomes an ordinary edge marked [`EdgeKind::Door`] with the door named in
//! `via`; a one-sided door, or one whose far side is computed, cannot, and
//! becomes an edge to a node standing for the DOOR itself rather than a guess
//! at the room behind it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};

use crate::hints::LoadedStory;

/// Every direction a map edge can carry, in the order the JSON's `directions`
/// vocabulary lists them. The twelve are exactly the twelve both world models
/// index their exit tables by (`Compass` in `zvm::world` and `gvm::world`),
/// which is why this can be a fixed array rather than a per-story discovery.
const DIRS: [Direction; 12] = [
    Direction::N,
    Direction::NE,
    Direction::E,
    Direction::SE,
    Direction::S,
    Direction::SW,
    Direction::W,
    Direction::NW,
    Direction::Up,
    Direction::Down,
    Direction::In,
    Direction::Out,
];

/// Which of the four static readers answered for this story.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Inform 7's own `Map_Storage` table (Glulx).
    I7World,
    /// The Inform 6 library's `door_dir`/`*_to` convention (Glulx or Z-machine).
    I6Library,
    /// Infocom's ZIL exit properties — UEXIT/NEXIT/FEXIT/CEXIT/DEXIT.
    Zil,
    /// A Scott Adams database's own room table.
    Scott,
}

impl SourceKind {
    /// The stable tag written into the JSON and printed in the summary. These
    /// strings are part of the file format; changing one is a format change.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::I7World => "i7-world",
            SourceKind::I6Library => "i6-library",
            SourceKind::Zil => "zil",
            SourceKind::Scott => "scott",
        }
    }
}

/// What kind of passage an edge is, most specific wins.
///
/// The precedence, most to least specific, is `Random`, `Conditional`,
/// `Routine`, `Door`, `OneWay`, `Declared` — so an edge is never labelled
/// twice and a consumer can switch on one value. `reciprocal` is reported
/// alongside and independently, because a door or a conditional can be
/// one-way too and the kind has room for only one fact.
///
/// **A consumer should tolerate a kind it does not know.** Phase 1 never emits
/// `Random` — no static source can know that a passage is randomised — but the
/// vocabulary is fixed here so that a later phase adding one is not a format
/// break for a reader that already skips the unfamiliar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// An ordinary declared passage whose reverse is declared too.
    Declared,
    /// A declared passage with no declared reverse.
    OneWay,
    /// The passage goes through a door object (named in [`EdgeFact::via`]).
    Door,
    /// The passage exists only while the story allows it — ZIL's CEXIT.
    Conditional,
    /// The destination is computed by the story's own code (a ZIL FEXIT, or an
    /// Inform `door_dir`/`*_to` routine) rather than declared directly, but
    /// SOME other room's plain, door or conditional exit declares the way back
    /// — see the `declared_to` lookup in `zmachine_map` (SQ-1334).
    Routine,
    /// The destination is drawn from a pool. Never emitted by phase 1.
    Random,
}

impl EdgeKind {
    /// The stable tag written into the JSON. Part of the file format.
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Declared => "declared",
            EdgeKind::OneWay => "one-way",
            EdgeKind::Door => "door",
            EdgeKind::Conditional => "conditional",
            EdgeKind::Routine => "routine",
            EdgeKind::Random => "random",
        }
    }
}

/// The engine-native identity of a room, so a consumer can correlate a node
/// here with the same room in a debugger, a disassembly or another tool.
///
/// [`RoomId`] is lanthorn's, and for Glulx it is a HASH of the address rather
/// than the address itself ([`crate::roomid::glulx_room_id`]) — irreversible,
/// so the raw fact has to travel beside it or it is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineRef {
    /// A Z-machine object number.
    ZObject(u16),
    /// A Glulx object's address in the story image.
    GlulxAddr(u32),
    /// An index into a Scott Adams database's room table.
    ScottIndex(usize),
}

/// One edge, with everything the graph itself has nowhere to put.
///
/// [`mapper::graph::Connection`] carries origin, direction, destination and
/// whether the layout had to distort it — which is all a DRAWN map needs. The
/// door and the condition are facts about the story, not about the drawing, so
/// they live here and travel beside the graph rather than inside it.
#[derive(Debug, Clone)]
pub struct EdgeFact {
    pub origin: RoomId,
    pub dir: Direction,
    pub dest: RoomId,
    pub kind: EdgeKind,
    /// The door object's name, when [`EdgeKind::Door`]. `None` otherwise.
    pub via: Option<String>,
    /// Free text describing the condition or the unresolved far side.
    pub note: Option<String>,
}

/// The story this map was read out of, identified the way its own format
/// identifies itself.
#[derive(Debug, Clone)]
pub struct StoryIdent {
    /// The file's base name, never its path — a reference map is checked in and
    /// read on other machines, and an absolute path is noise there.
    pub file: String,
    pub engine: &'static str,
    /// Z-machine (ZMSD §11.1) or an Inform-compiled Glulx image (Glulx-Inform-Tech
    /// §1 "Static Data"): release number and serial code. `None` for a Scott
    /// Adams database, whose format carries neither.
    pub release: Option<u16>,
    pub serial: Option<String>,
    /// The story's own header checksum, formatted as a lowercase `0x`-prefixed
    /// hex string — a Z-machine word (ZMSD §11.1, `$1C`) or a Glulx image's
    /// whole-memory sum (Glulx spec §1.4, offset `0x20`). `None` for Scott
    /// Adams, which has no such field.
    pub checksum: Option<String>,
}

/// The room the story puts the player in at boot, once it has been matched to
/// a node of the static graph.
///
/// The name travels with the id because the id alone is engine-native and
/// unreadable — a Z-machine object number, a hashed Glulx address, a Scott
/// room index — and every surface that reports the start room reports it by
/// name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRoom {
    pub id: RoomId,
    pub name: String,
}

/// What the headless boot ([`probe_start_room`]) learned about where the story
/// starts — the three states the dump header and the JSON both report.
///
/// The distinction between [`Unknown`](Self::Unknown) and
/// [`Skipped`](Self::Skipped) is worth keeping: the first says the story was
/// booted and would not say (a menu-driven v6 title that never reaches a line
/// prompt, a Glulx prologue whose room lock never resolves), the second says
/// nobody asked. Both fall back to the largest-component rule, and a reader of
/// a dump should be able to tell which happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartProbe {
    /// The boot named a room and it matched a room of this graph.
    Found(StartRoom),
    /// The boot ran and could not name a room this map has.
    Unknown,
    /// No boot was run — [`MapgenOptions::boot_for_start`] was false.
    Skipped,
}

impl StartProbe {
    /// The room, when there is one.
    pub fn room(&self) -> Option<&StartRoom> {
        match self {
            StartProbe::Found(r) => Some(r),
            _ => None,
        }
    }

    /// The one line the text dump's header carries and the binary prints — the
    /// same sentence in both, so a dump and a terminal never disagree about
    /// what happened.
    pub fn header_line(&self) -> String {
        match self {
            StartProbe::Found(r) => format!("start room: {} (#{})", r.name, r.id),
            StartProbe::Unknown => {
                "start room: unknown (largest component kept as Main)".to_string()
            }
            StartProbe::Skipped => {
                "start room: not probed (--no-boot; largest component kept as Main)".to_string()
            }
        }
    }
}

/// A complete static map: the graph, laid out unless asked not to, plus
/// everything about it that the graph cannot hold.
#[derive(Debug)]
pub struct GeneratedMap {
    pub graph: MapGraph,
    pub source: SourceKind,
    pub story: StoryIdent,
    pub facts: Vec<EdgeFact>,
    pub engine_refs: BTreeMap<RoomId, EngineRef>,
    /// How long [`mapper::layout::relayout_auto`] took, or `None` when the
    /// caller asked for no layout (in which case no room has a position).
    pub layout_time: Option<Duration>,
    /// Where the story starts the player, and how confidently (SQ-1359). When
    /// this is [`StartProbe::Found`], `graph.current()` is that room and the
    /// layer holding it is `Main`.
    pub start: StartProbe,
}

impl GeneratedMap {
    /// Rooms whose name the source could produce. A room the story names only
    /// through a routine has an empty name here, which is a real answer and not
    /// a failure — see [`gvm::i7map::I7World::printed_name`].
    pub fn named_rooms(&self) -> usize {
        self.graph.rooms().filter(|r| !r.label().trim().is_empty()).count()
    }

    pub fn doors(&self) -> usize {
        self.facts.iter().filter(|f| f.kind == EdgeKind::Door).count()
    }

    pub fn conditionals(&self) -> usize {
        self.facts.iter().filter(|f| f.kind == EdgeKind::Conditional).count()
    }
}

/// Why a story produced no map.
#[derive(Debug)]
pub enum GenError {
    /// The file could not be read or was not a story at all.
    Load(std::io::Error),
    /// The engine's own loader refused the image.
    Engine(String),
    /// Every static reader for this engine declined. The `String` says which
    /// were tried and what each wanted — this is what the binary exits 2 with,
    /// so it has to be a sentence a reader can act on.
    NoStaticSource(String),
}

impl std::fmt::Display for GenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenError::Load(e) => write!(f, "{e}"),
            GenError::Engine(m) | GenError::NoStaticSource(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for GenError {}

/// Knobs for the auto-split pass ([`split_layers`]) that runs between building
/// the graph and laying it out.
///
/// `layer_min` is deliberately the SAME constant the live app's suggestion
/// engine floors a structural region at
/// ([`mapper::suggest::STRUCTURAL_FLOOR`]) unless a caller overrides it — the
/// two are meant to agree, and drifting apart would mean a static map and a
/// played one disagree about how big a region has to be before it earns a
/// layer of its own.
#[derive(Debug, Clone, Copy)]
pub struct MapgenOptions {
    /// Split maze and portal-only regions onto their own layers ([`split_layers`]).
    /// `false` reproduces the pre-SQ-1308 behaviour: everything on `MAIN_LAYER`.
    pub auto_layers: bool,
    /// The smallest portal-only region worth its own layer — [`STRUCTURAL_FLOOR`]
    /// by default. A maze region has no floor: any size gets its own layer once
    /// its name says so.
    ///
    /// [`STRUCTURAL_FLOOR`]: mapper::suggest::STRUCTURAL_FLOOR
    pub layer_min: usize,
    /// Boot the story headlessly, once, to learn which room it starts the
    /// player in ([`probe_start_room`]) — the room whose layer is then kept as
    /// `Main` and which the graph carries as its `current` room.
    ///
    /// `false` skips the boot entirely (`--no-boot`), which is what a caller
    /// wants when the story is a menu-driven title that reaches no prompt, or
    /// when a purely static read is the point. Nothing else about the map
    /// changes: the split falls back to the largest component, exactly as it
    /// did before SQ-1359.
    pub boot_for_start: bool,
}

impl Default for MapgenOptions {
    fn default() -> Self {
        MapgenOptions {
            auto_layers: true,
            layer_min: mapper::suggest::STRUCTURAL_FLOOR,
            boot_for_start: true,
        }
    }
}

/// One layer [`split_layers`] carved out, for the summary the binary prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerSplit {
    pub id: mapper::layer::LayerId,
    pub name: String,
    pub rooms: usize,
    pub maze: bool,
}

/// The connected cluster of maze-named rooms containing `start`, all on the
/// layer `start` is on — every step of the walk, not just its first, must
/// [`mapper::suggest::mentions_maze`].
///
/// **Vertical passages count here, unlike everywhere else a region is walked**
/// (SQ-1372). A portal is a boundary between FLOORS, and that is exactly what
/// `planar_region` is right to stop at — but a maze is one warren whose name
/// says so, and the twisty little passage that happens to go UP is no more a
/// change of floor than the one that goes north. Adventure's "all alike" maze
/// is fourteen rooms hanging together through six `u_to`/`d_to` links, and
/// stopping at those split it into layers of eleven, three and ONE — three
/// floor plans of a warren that has no floors. The name filter is what keeps
/// this walk honest (see below); the direction never was.
///
/// This is deliberately **not** [`mapper::layer::planar_region`], which has no
/// way to stop at a name and sweeps up anything compass-reachable regardless —
/// and Zork I's own maze is exactly the story that makes that the wrong walk.
/// Verified against `crates/zvm/tests/fixtures/minizork.z3` (Mini-Zork I,
/// r34/s871124): the Cyclops Room has an UNCONDITIONAL compass link out to the
/// Living Room ("Strange Passage" — `Cyclops Room -NW-> Maze` is how the maze
/// is entered from that side), so a bare `planar_region` from any maze room
/// pulls in 55 of the story's 70 rooms — Kitchen, Living Room, Troll Room,
/// West of House and the rest of the surface included, i.e. everything BUT the
/// dozen rooms directly behind Troll Room, which is not a maze layer.
/// [`mapper::layer::region_at_arrival`] — the live app's own `name_trigger`
/// walk — fares no better: probed at all three of the maze's real compass
/// entrances (Troll Room, Cyclops Room, Grating Room), it is `NotASeam` at two
/// of them and, at the third, succeeds only by excluding that ONE entrance
/// room while including the same 54-room sweep, because the other two
/// entrances are still standing open for the walk to leave through. Neither of
/// `mapper::layer`'s public region walks can be asked to stop at a room-name
/// boundary, so this one does it directly with the graph.
fn maze_region(graph: &MapGraph, start: RoomId) -> mapper::layer::Region {
    let layer = graph.layer_of(start);
    let mut rooms: BTreeSet<RoomId> = BTreeSet::new();
    rooms.insert(start);
    let mut q: std::collections::VecDeque<RoomId> = std::collections::VecDeque::new();
    q.push_back(start);
    while let Some(cur) = q.pop_front() {
        for c in graph.connections() {
            let other = if c.origin == cur {
                c.dest
            } else if c.dest == cur {
                c.origin
            } else {
                continue;
            };
            if graph.layer_of(other) == layer
                && graph.room(other).is_some_and(|r| mapper::suggest::mentions_maze(r.label()))
                && rooms.insert(other)
            {
                q.push_back(other);
            }
        }
    }
    mapper::layer::Region { anchor: start, rooms }
}

/// Rename maze layers that share a name, after the room each is ENTERED FROM
/// (SQ-1372) — "Maze (off At West End of Hall of Mists)".
///
/// Every room of a maze is called the same thing; that is what a maze is, and
/// it is why the layer a maze region becomes takes that one name. A story with
/// TWO mazes therefore ends up with two layers called "Maze", which names
/// neither of them: Adventure ships an "all alike" maze and an "all different"
/// one, and its alike maze arrives in two pieces besides (`At Brink of Pit` is
/// a named room standing in the middle of it, and [`maze_region`]'s walk stops
/// at a name).
///
/// The entrance is the disambiguator a player already uses — the maze *off the
/// Hall of Mists*, the one *off the Long Hall* — and it is a fact of the map
/// rather than an ordinal, so it does not renumber when a story is re-read.
/// Where a region has several outside neighbours, the one with the most edges
/// into it wins, ties going to the lowest room id; a `#2` suffix is the last
/// resort for two layers that really are entered from the same room, so that
/// layer names stay unique whatever the map does.
///
/// Only layers whose name is NOT unique are touched: one maze in a story stays
/// plainly "Maze".
fn name_maze_layers_by_entrance(graph: &mut MapGraph, splits: &mut [LayerSplit]) {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for m in graph.layers().values() {
        *count.entry(m.name.clone()).or_default() += 1;
    }
    let mut used: BTreeSet<String> = graph.layers().values().map(|m| m.name.clone()).collect();
    let ambiguous: Vec<usize> = splits
        .iter()
        .enumerate()
        .filter(|(_, s)| s.maze && count.get(&s.name).copied().unwrap_or(0) > 1)
        .map(|(i, _)| i)
        .collect();
    for i in ambiguous {
        let Some(entrance) = maze_entrance(graph, splits[i].id) else { continue };
        let base = format!("{} (off {})", splits[i].name, entrance);
        let mut name = base.clone();
        let mut n = 2;
        while !used.insert(name.clone()) {
            name = format!("{base} #{n}");
            n += 1;
        }
        graph.set_layer_name(splits[i].id, name.clone());
        splits[i].name = name;
    }
}

/// The room `layer` is most entered from: the room OUTSIDE it with the most
/// edges into it, ties to the lowest room id. `None` for a layer nothing leads
/// into. See [`name_maze_layers_by_entrance`].
fn maze_entrance(graph: &MapGraph, layer: mapper::layer::LayerId) -> Option<String> {
    let mut edges: BTreeMap<RoomId, usize> = BTreeMap::new();
    for c in graph.connections() {
        if c.is_self_loop() {
            continue;
        }
        let outside = match (graph.layer_of(c.origin) == layer, graph.layer_of(c.dest) == layer) {
            (true, false) => c.dest,
            (false, true) => c.origin,
            _ => continue,
        };
        *edges.entry(outside).or_default() += 1;
    }
    // `BTreeMap` iterates by ascending room id, and `max_by_key` keeps the LAST
    // maximum — so reverse the id to make a tie fall to the lowest.
    edges
        .into_iter()
        .max_by_key(|&(id, n)| (n, std::cmp::Reverse(id)))
        .and_then(|(id, _)| graph.room(id).map(|r| r.label().to_string()))
}

/// SQ-1311, widened by SQ-1391: absorb onto a maze layer any room still on
/// Main that has no COMPASS passage anywhere but into maze layers, and at
/// least one passage — compass or portal — into one. [`maze_region`]'s walk
/// stops at a room name on purpose (the Cyclops Room case in its doc
/// comment), so a genuine maze exit merely named "Dead End" is excluded from
/// the region and left stranded on Main; this repairs that without touching
/// the walk, since it runs only after every maze region already exists.
///
/// **Only a COMPASS passage disqualifies; a portal never does, and can now
/// admit on its own** (SQ-1391). Mini-Zork's Grating Room is the case that
/// pins the first half: it has one compass edge into the maze and one
/// portal — the grating itself — up to Forest Path, a genuine surface room,
/// and it must still join the maze on the strength of the compass edge
/// alone, exactly as it always has (`minizork_grating_room_joins_the_maze_by_its_compass_edge_alone`).
/// A portal is a boundary between FLOORS everywhere else this graph is
/// walked ([`maze_region`]'s own doc comment), and the grating is exactly
/// that: the maze's door to the surface, not evidence the room is somewhere
/// else. Adventure's dead ends pin the second half: `#78`, `#43` and `#49`
/// hang off the "all alike" maze by Down alone (`#45 D #78`, `#78 U #45`,
/// nothing compass at all), so SQ-1311's compass-only sweep — which required
/// at least one compass edge to fire at all — left every one of them
/// stranded; a portal-only pocket must be able to join too; a dead end is
/// part of the warren it dead-ends in, whichever way you have to go to get
/// there.
///
/// A pocket can sit between two DIFFERENT maze layers (Adventure's #80: one
/// compass passage into the "all alike" maze, two into the "off At Brink of
/// Pit" one) — those go to whichever maze the room has the most passages
/// (compass or portal) into, ties going to whichever maze layer was created
/// earlier in pass 1 (the lower [`mapper::layer::LayerId`], since layer ids
/// are handed out in the order `split_layers` walks maze rooms — ascending
/// room id — so "walked first" and "numbered lower" are the same fact).
///
/// Iterates to a fixed point, because a dead end can hang off another dead
/// end that only just got absorbed this round (a corridor of them, each one
/// passage step from the last).
fn absorb_maze_adjacent_rooms(graph: &mut MapGraph) {
    loop {
        let mut absorbed_any = false;
        let candidates: Vec<RoomId> =
            graph.rooms().filter(|r| r.layer == mapper::layer::MAIN_LAYER).map(|r| r.id).collect();
        for room in candidates {
            let mut edge_counts: BTreeMap<mapper::layer::LayerId, usize> = BTreeMap::new();
            let mut ok = true;
            for c in graph.connections() {
                if c.is_self_loop() {
                    continue;
                }
                let other = if c.origin == room {
                    c.dest
                } else if c.dest == room {
                    c.origin
                } else {
                    continue;
                };
                let layer = graph.layer_of(other);
                let is_maze = layer != mapper::layer::MAIN_LAYER && graph.layer_is_maze(layer);
                if is_maze {
                    *edge_counts.entry(layer).or_default() += 1;
                } else if mapper::direction::grid_offset(c.dir).is_some() {
                    // A COMPASS edge to Main (or a room not yet on any maze layer) is a
                    // genuine exit — the Cyclops Room case — and disqualifies the room
                    // outright. A PORTAL edge to the same place is just a floor boundary
                    // (the Grating Room case) and is silently ignored either way.
                    ok = false;
                    break;
                }
            }
            if ok && !edge_counts.is_empty() {
                // Most passages into wins; ties to the lower layer id (`Reverse` because
                // `BTreeMap` iterates ascending and `max_by_key` keeps the LAST maximum).
                if let Some((&layer, _)) =
                    edge_counts.iter().max_by_key(|&(&id, &n)| (n, std::cmp::Reverse(id)))
                {
                    graph.set_room_layer(room, layer);
                    absorbed_any = true;
                }
            }
        }
        if !absorbed_any {
            break;
        }
    }
}

/// Split `graph`'s `MAIN_LAYER` the way the APP's own layer suggestions would if
/// a player accepted every one of them (SQ-1308) — reusing
/// [`mapper::layer::planar_region`] and [`mapper::layer::move_region`] rather
/// than a second implementation of what a region is.
///
/// Two passes, in order:
///
/// 1. **Maze layers.** Every room still on `MAIN_LAYER` whose name
///    [`mapper::suggest::mentions_maze`] anchors a [`maze_region`] walk; if
///    that region is still (wholly) on Main — an earlier maze room's region may
///    already have carried it away, which is what "one layer per component"
///    means — it moves to a fresh layer flagged as a maze, exactly as accepting
///    the app's [`mapper::suggest::Trigger::Name`] prompt does
///    ([`crate::input::apply_region_prompt`]).
/// 2. **Portal-only regions.** What is left of Main is partitioned into
///    compass-connected components. **The component holding the graph's
///    CURRENT room is kept as Main** (SQ-1359) — mapgen sets that to the room
///    the story boots the player into ([`probe_start_room`]), so a static map
///    anchors its primary layer on exactly what the live map anchors on:
///    wherever play begins. Every other component at or above
///    `opts.layer_min` becomes its own layer — INCLUDING the largest, when the
///    largest is not the start's — named after the room its entering portal
///    leads into, the same anchor [`move_region`]'s `New` target names a peel
///    after ([`mapper::layer::move_region`]'s doc comment on
///    `MoveTarget::New`). A component under the floor stays on Main untouched.
///
///    With no current room — `--no-boot`, a story that reaches no prompt, a
///    caller driving this function over a graph of its own — the LARGEST
///    component is kept instead, which is what mapgen did before SQ-1359 and
///    is still the only answer available when nothing says where play starts.
///    Zork I r52 is why the rule changed: its underground is bigger than its
///    surface, so the largest-component rule put West of House and the whole
///    above-ground world on a layer called "Rocky Ledge" and called the
///    Cellar and its neighbours "Main".
///
/// `opts.auto_layers = false` skips both passes and returns an empty list —
/// the pre-SQ-1308 flat map.
pub fn split_layers(graph: &mut MapGraph, opts: &MapgenOptions) -> Vec<LayerSplit> {
    use mapper::layer::{move_region, planar_region, MoveTarget, Region, MAIN_LAYER};

    let mut splits = Vec::new();
    if !opts.auto_layers {
        return splits;
    }

    // ── 1. Maze layers ──────────────────────────────────────────────────────
    let maze_rooms: Vec<RoomId> = graph
        .rooms()
        .filter(|r| r.layer == MAIN_LAYER && mapper::suggest::mentions_maze(r.label()))
        .map(|r| r.id)
        .collect();
    let mut done: BTreeSet<RoomId> = BTreeSet::new();
    for room in maze_rooms {
        // Already carried off by an earlier maze room's region: "one layer per
        // component", not one per room that happens to be named "maze".
        if graph.layer_of(room) != MAIN_LAYER || done.contains(&room) {
            continue;
        }
        let region = maze_region(graph, room);
        done.extend(region.rooms.iter().copied());
        if let Ok(layer) = move_region(graph, &region, MoveTarget::New) {
            graph.set_layer_maze(layer, true);
            splits.push(LayerSplit {
                id: layer,
                name: graph.layer_name(layer).to_string(),
                rooms: region.rooms.len(),
                maze: true,
            });
        }
    }

    // ── 1b. Dead ends off a maze ────────────────────────────────────────────
    // `maze_region`'s walk deliberately refuses to cross into a room whose
    // name does not mention "maze" (its own doc comment, the Cyclops Room
    // case), so a maze exit that happens to be named "Dead End" is excluded
    // from the region above and left stranded on Main. Absorb it now that the
    // regions exist to absorb it onto.
    absorb_maze_adjacent_rooms(graph);
    for split in splits.iter_mut().filter(|s| s.maze) {
        split.rooms = graph.rooms_in_layer(split.id).len();
    }

    // ── 1c. Tell one maze from another ─────────────────────────────────────
    name_maze_layers_by_entrance(graph, &mut splits);

    // ── 2. Portal-only regions among what is left on Main ──────────────────
    let mut seen: BTreeSet<RoomId> = BTreeSet::new();
    let mut components: Vec<Region> = Vec::new();
    let remaining: Vec<RoomId> = graph.rooms().filter(|r| r.layer == MAIN_LAYER).map(|r| r.id).collect();
    for room in remaining {
        if seen.contains(&room) {
            continue;
        }
        let region = planar_region(graph, room);
        seen.extend(region.rooms.iter().copied());
        components.push(region);
    }

    // SQ-1359: the component the player STARTS in is Main. `graph.current()`
    // is where `generate_with_options` recorded that (and is what the live map
    // means by the same field), so this needs no extra argument and no second
    // notion of "primary" — a caller who never set one gets the old rule.
    //
    // The start room can legitimately be absent from `components`: pass 1 may
    // already have carried it onto a maze layer, in which case no component on
    // Main holds it. Falling through to the largest is right there too.
    let start_idx = graph
        .current()
        .and_then(|id| components.iter().position(|r| r.rooms.contains(&id)));

    // Failing that, the largest component is Main; ties keep whichever was
    // found first (ascending room id), so the choice is deterministic rather
    // than an artifact of `BTreeSet` iteration.
    let main_idx = start_idx.or_else(|| {
        components
            .iter()
            .enumerate()
            .max_by_key(|&(i, r)| (r.rooms.len(), std::cmp::Reverse(i)))
            .map(|(i, _)| i)
    });

    let mut below_floor: Vec<Region> = Vec::new();
    for (i, region) in components.into_iter().enumerate() {
        if Some(i) == main_idx {
            continue;
        }
        if region.rooms.len() < opts.layer_min {
            below_floor.push(region);
            continue;
        }
        let named = name_region_by_entry(graph, region);
        if let Ok(layer) = move_region(graph, &named, MoveTarget::New) {
            splits.push(LayerSplit {
                id: layer,
                name: graph.layer_name(layer).to_string(),
                rooms: named.rooms.len(),
                maze: false,
            });
        }
    }

    // ── 3. Adopt below-floor leftovers onto a portal-connected layer ───────
    adopt_stranded_regions(graph, below_floor);

    splits
}

/// SQ-1310: a below-floor component (too small for its own layer) still gets
/// discovered on whichever layer the PLAYER was standing on — a one-room attic
/// reached by `Up` from the Kitchen stays with the house, never lands on Main
/// by itself. `split_layers`'s pass 2 has no such context (it only ever sees
/// Main), so every stranded component defaults there; this pass corrects that
/// by following each one's own portal edges (`Up`/`Down`/`In`/`Out` —
/// [`mapper::direction::grid_offset`] is `None` for exactly these) to whichever
/// neighbouring layer is already settled.
///
/// A neighbour "already settled" means outside every region still in `pending`
/// — Main itself counts (its rooms sit on [`mapper::layer::MAIN_LAYER`] from the
/// start), and so does every maze/portal layer pass 1/2 just created. Ties among
/// a region's several portal neighbours go to whichever layer has the MOST
/// portal links into the region, and ties on THAT go to the lowest layer id, so
/// the outcome never depends on `BTreeSet`/`Vec` iteration order. Adoption
/// repeats to a fixed point (`loop`) because one stranded component can hang
/// off ANOTHER stranded component that only just got a home this pass — the
/// order they happen to appear in `pending` must not matter.
///
/// A maze layer only ever adopts a component whose own room names
/// [`mapper::suggest::mentions_maze`] — a stray one-room dead end that merely
/// happens to open off a maze stays with whatever NON-maze neighbour it has
/// instead (or Main, if it has none), exactly as pass 1 never pulls an
/// unrelated room onto a maze layer by accident.
fn adopt_stranded_regions(graph: &mut MapGraph, mut pending: Vec<mapper::layer::Region>) {
    loop {
        let pending_rooms: BTreeSet<RoomId> =
            pending.iter().flat_map(|r| r.rooms.iter().copied()).collect();
        let mut next_pending = Vec::new();
        let mut adopted_any = false;

        for region in pending {
            let is_maze_named = region.rooms.iter().any(|&id| {
                graph.room(id).map(|r| mapper::suggest::mentions_maze(r.label())).unwrap_or(false)
            });

            let mut tally: BTreeMap<mapper::layer::LayerId, usize> = BTreeMap::new();
            for c in graph.connections() {
                if mapper::direction::grid_offset(c.dir).is_some() || c.is_self_loop() {
                    continue; // a compass edge is not a portal, and a self-loop links nothing
                }
                let outside = if region.rooms.contains(&c.origin) && !region.rooms.contains(&c.dest) {
                    c.dest
                } else if region.rooms.contains(&c.dest) && !region.rooms.contains(&c.origin) {
                    c.origin
                } else {
                    continue;
                };
                if pending_rooms.contains(&outside) {
                    continue; // that neighbour has no home yet either — wait for it
                }
                let layer = graph.layer_of(outside);
                if graph.layer_is_maze(layer) && !is_maze_named {
                    continue; // SQ-1310: only a maze-named region may adopt onto a maze layer
                }
                *tally.entry(layer).or_insert(0) += 1;
            }

            match tally.into_iter().max_by_key(|&(layer, count)| (count, std::cmp::Reverse(layer))) {
                Some((layer, _)) => {
                    for &id in &region.rooms {
                        graph.set_room_layer(id, layer);
                    }
                    adopted_any = true;
                }
                None => next_pending.push(region),
            }
        }

        pending = next_pending;
        if !adopted_any || pending.is_empty() {
            break; // fixed point: nothing left adopted anything this round, or nothing is left
        }
    }
    // Whatever remains in `pending` has no portal-connected home at all — it
    // stays on Main untouched, exactly as pass 2 already left it.
}

/// Re-anchor `region` on the room its entering portal leads into, so
/// [`mapper::layer::move_region`]'s `MoveTarget::New` names the fresh layer
/// after that room rather than whichever room `planar_region`'s own BFS
/// happened to start from.
///
/// "The entering portal" is any portal connection whose destination is IN the
/// region and whose origin is not — mirroring [`mapper::suggest::entry_seam`]'s
/// restriction to portals (an `Unknown` edge is a cut for [`planar_region`]'s
/// purposes but not a passage anyone can point at). Unlike `entry_seam`, this
/// has no discovery order to break ties with (mapgen plays nothing), so among
/// several candidate entrances the lowest room id wins — deterministic, and
/// the room a static reader would call "first" for want of any other order.
/// A region with no inbound portal at all (an island with no edge in from
/// anywhere) falls back to its lowest room id, same tie-break, one level up.
///
/// "Lowest room id" is the story's own author-definition order (its rooms'
/// compiled/declared order, not a property of the map's shape) — which is
/// why it names Zork I's underground `Cellar` (not `Studio` or `Canyon
/// Bottom`), its mine pocket `Coal Mine`, and Anchorhead's upstairs
/// `Upstairs Hall`. A nearest-entrance walk from the start room was built
/// and measured here 2026-09-07 (SQ-1361) and rejected: plain hop-counting
/// named the underground `Studio` (the Kitchen's CEXIT staircase, one hop
/// nearer than the Living Room's trap door); costing every gated exit at 3
/// against 1 for an ordinary one named it `Canyon Bottom` instead (an
/// ungated canyon route at weighted distance 6, beating both gated "front
/// doors" also at 6); and costing the trap door's own door-like ROUTINE
/// exit back down to 1, same as a Door, restored `Cellar` — but only by ONE
/// STEP (5 vs 6 vs 6). A rule that close to a coin flip on the layer that
/// matters most, while also renaming two already-good peels elsewhere
/// (Anchorhead's `Upstairs Hall`/`Storm Tunnel`), was not worth keeping over
/// the simpler, deterministic rule already here.
fn name_region_by_entry(graph: &MapGraph, region: mapper::layer::Region) -> mapper::layer::Region {
    let entry = graph
        .connections()
        .iter()
        .filter(|c| {
            region.rooms.contains(&c.dest)
                && !region.rooms.contains(&c.origin)
                && mapper::direction::is_portal(c.dir)
        })
        .map(|c| c.dest)
        .min()
        .unwrap_or_else(|| *region.rooms.iter().min().expect("a region always has a room"));
    mapper::layer::Region { anchor: entry, rooms: region.rooms }
}

// ---------------------------------------------------------------------------
// Where the story starts (SQ-1359)
// ---------------------------------------------------------------------------

/// The most keypresses [`probe_start_room`] will spend clearing an opening
/// gate. A story that wants more than this is one nobody is playing past
/// either — and the cap is what keeps a menu-driven title from hanging mapgen,
/// which reads a file and must always terminate.
const START_PROBE_KEYS: usize = 24;

/// The most LINE commands [`probe_start_room`] will spend. Two, because the
/// only line it ever types is `look`, and a story that will not name its room
/// after being asked twice is not going to.
const START_PROBE_LINES: usize = 2;

/// Boot `loaded` headlessly through the SAME session the app plays it with and
/// ask where the player is standing (SQ-1359).
///
/// This is the one thing in this module that RUNS the story, and it is
/// deliberately the smallest run that can answer the question: nothing is
/// rendered, nothing is saved, and the session is dropped the moment it has
/// answered. The map itself is still read statically — the boot decides only
/// which layer is called `Main` and which room the drawing highlights.
///
/// Three shapes of story, three answers:
///
/// - **Z-machine.** [`crate::session::GameSession`] resolves the player's
///   containing object during boot, so Zork I answers `West of House` with no
///   turn played at all.
/// - **Glulx.** There is no object tree to walk: [`crate::glulx_session`]
///   recovers the room from the heading the story PRINTS, so a story whose
///   prologue prints none (Counterfeit Monkey — SQ-1293) may answer nothing
///   until it hands over the command prompt. A `look` is spent to ask, and if
///   the answer is still nothing, so be it.
/// - **Scott Adams.** The VM's own current room, which exists from the first
///   instruction.
///
/// An opening KEYPRESS gate is cleared with SPACE, the same idiom (and for the
/// same reason) as `declared_exit.rs`'s `boot` and `vocabulary_vetting.rs`'s
/// `Play::gated_z5`: a story waiting on `read_char` never reaches its first
/// room until a key is actually pressed, and it must be a KEY — a line routed
/// to a char prompt is delivered as its first character, which Curses reads as
/// `l` and ignores. Both spends are capped ([`START_PROBE_KEYS`],
/// [`START_PROBE_LINES`]).
pub fn probe_start_room(loaded: &LoadedStory) -> Option<crate::engine::LocationInfo> {
    use crate::engine::Engine;

    let mut engine: Box<dyn Engine> = match loaded {
        LoadedStory::ZCode(bytes) => Box::new(
            crate::session::GameSession::new_with_trace(
                bytes.clone(),
                true,
                false,
                None,
                false,
                Vec::new(),
                None,
                None,
                Some((25, 80)),
            )
            .ok()?,
        ),
        LoadedStory::Glulx(image) => Box::new(
            crate::glulx_session::GlulxSession::new(
                image.clone(),
                80,
                30,
                true,
                false,
                false,
                (8, 16),
                None,
                &[],
            )
            .ok()?,
        ),
        LoadedStory::Scott(bytes) => {
            Box::new(crate::scott_session::ScottSession::new(bytes.clone(), None).ok()?)
        }
    };

    let mut keys = 0usize;
    let mut lines = 0usize;
    loop {
        // A room with a name is an answer; a room with an EMPTY name is what a
        // Glulx session reports before its lock resolves, and is not one.
        if let Some(loc) = engine.current_location() {
            if !loc.name.trim().is_empty() {
                return Some(loc);
            }
        }
        match engine.pending_input() {
            crate::session::InputKind::Char if keys < START_PROBE_KEYS => {
                keys += 1;
                let _ = engine.submit_key(crate::engine::KeyInput::Char(' '));
            }
            crate::session::InputKind::Line if lines < START_PROBE_LINES => {
                lines += 1;
                let _ = engine.submit("look");
            }
            // Out of budget, or waiting for something no probe should answer
            // (a filename, a quit): whatever the session knows now is the last
            // word, and it may well be nothing.
            _ => return engine.current_location(),
        }
    }
}

/// Match what [`probe_start_room`] found against the STATIC graph's own rooms.
///
/// Every engine's [`crate::engine::LocationInfo::number`] is already in the
/// same id space the corresponding reader keys its rooms by — a Z-machine
/// object number, `crate::roomid::glulx_room_id` of the room's address, a
/// Scott room index — so the id is tried first and is what answers for all
/// three in the ordinary case.
///
/// The NAME is the fallback, and only when it is unambiguous: a Glulx session
/// whose room lock never resolved reports a room hashed from the printed
/// HEADING instead of from an address (`glulx_session::heading_to_room`), which
/// is a real room under an id the static reader has never heard of. One room
/// of the map bearing that exact name is the same room; two are not evidence
/// of anything, and no match at all is the honest answer.
fn resolve_start(graph: &MapGraph, loc: &crate::engine::LocationInfo) -> Option<StartRoom> {
    if let Some(r) = graph.room(loc.number) {
        return Some(StartRoom { id: r.id, name: r.label().to_string() });
    }
    let name = loc.name.trim();
    if name.is_empty() {
        return None;
    }
    let mut hits = graph.rooms().filter(|r| r.label().trim() == name);
    let first = hits.next()?;
    if hits.next().is_some() {
        return None; // ambiguous: two rooms of this map answer to that name
    }
    Some(StartRoom { id: first.id, name: first.label().to_string() })
}

/// Generate the static map for the story at `path`, with mapgen's own defaults
/// (SQ-1308's layer auto-split, floored at [`mapper::suggest::STRUCTURAL_FLOOR`]).
///
/// `layout` runs the app's whole tidy pipeline over the finished graph, which is
/// what gives every room a position; without it the graph is pure topology and
/// every `pos` is `None`. It runs **once per layer** ([`layout_all_layers`])
/// rather than once over the whole graph — see that function's doc comment for
/// why (SQ-1309), and for why the pipeline is the same five stages the live map
/// gets rather than the solve alone (SQ-1376).
///
/// The story is mounted through [`crate::hints::load_mounted_story`] — the same
/// call `startup.rs` boots from — so a Blorb, a zip and a disk image all reach
/// here as bare executable bytes, classified by engine.
pub fn generate(path: &Path, layout: bool) -> Result<GeneratedMap, GenError> {
    generate_with_options(path, layout, &MapgenOptions::default())
}

/// [`generate`], with the auto-split pass's knobs exposed ([`MapgenOptions`]).
pub fn generate_with_options(
    path: &Path,
    layout: bool,
    opts: &MapgenOptions,
) -> Result<GeneratedMap, GenError> {
    let (loaded, _medium) = crate::hints::load_mounted_story(path).map_err(GenError::Load)?;
    let file = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();

    let mut map = match &loaded {
        LoadedStory::ZCode(bytes) => zmachine_map(bytes, file)?,
        LoadedStory::Glulx(bytes) => glulx_map(bytes, file)?,
        LoadedStory::Scott(bytes) => scott_map(bytes, file)?,
    };

    // SQ-1359: learn where play begins BEFORE the split, because the split
    // reads it (`split_layers`' pass 2) — and record it as the graph's current
    // room, which is what puts the drawing's "you are here" highlight on the
    // starting room rather than on nothing.
    map.start = if opts.boot_for_start {
        match probe_start_room(&loaded).as_ref().and_then(|loc| resolve_start(&map.graph, loc)) {
            Some(start) => {
                map.graph.set_current(start.id);
                StartProbe::Found(start)
            }
            None => StartProbe::Unknown,
        }
    } else {
        StartProbe::Skipped
    };

    split_layers(&mut map.graph, opts);

    if layout {
        let t = Instant::now();
        layout_all_layers(&mut map.graph);
        map.layout_time = Some(t.elapsed());
    }
    annotate_rooms(&mut map);
    Ok(map)
}

/// Lay out every layer of `graph` INDEPENDENTLY, the way the live app's
/// `run_tidy_pipeline`/`tidy_layer_silent` (`crates/app/src/tidy.rs`) tidy one
/// layer at a time via `graph.layer_subgraph` (SQ-1309).
///
/// Calling [`mapper::layout::relayout_auto`] once over the WHOLE graph runs every
/// layer's rooms through one shared stress-solve and one shared pack stage. Two
/// layers connected only by a portal (a maze reached by `Up`, a side region
/// reached by a one-way stub) are still one graph-connected component to that
/// solve, so it treats a maze's tangle of one-way passages as evidence about
/// where the PARENT layer's rooms belong, and packs rooms from unrelated layers
/// into the same cells the way it would pack two components of one real layer.
/// On Zork I this misplaced the Torch Room's four rooms entirely (a maze-free
/// layer, yet colliding with Main-layer cells it was never laid out against) and
/// pulled Main's own East-West Passage and Chasm off their rows and columns.
///
/// `graph.layer_subgraph(layer)` already drops every connection that crosses a
/// layer boundary (both endpoints must be in `layer`), so running the exact same
/// tidy pipeline on each layer's subgraph in isolation
/// gives each layer its own solve and its own pack, with no other layer's rooms
/// or portals in the room to compete with. A maze layer gets no special-case
/// here (unlike the live app, which freezes a maze layer's positions once it has
/// any — SQ-0671): mapgen has no earlier incremental placement to freeze, so the
/// maze still needs an initial layout, and running it in its own subgraph rather
/// than freezing it here confines whatever a maze's unsatisfiable geometry does
/// to the maze's own layer.
///
/// **And it is the WHOLE pipeline, not the solve** (SQ-1376). `relayout_auto` is stage one of
/// five; `cleanup_overlaps`, `repair_directional_hints`, `cleanup_overlaps` and
/// `compact_empty_lines` are the rest, and the live map has always had them
/// ([`crate::tidy::tidy_layer_silent`]). mapgen ran the first alone for its whole life, so a
/// generated map and a played map could disagree about the same rooms with the same solver —
/// which is exactly how SQ-1376 was reported ("it works live"). The gap is not cosmetic: the
/// contiguity stage inside `relayout_auto` deliberately breaks bearings to keep a row tight
/// (see `mapper::layout::contiguify`), and `repair_directional_hints` is the pass that puts
/// them back.
pub fn layout_all_layers(graph: &mut MapGraph) {
    let layer_ids: Vec<mapper::layer::LayerId> = graph.layers().keys().copied().collect();
    for layer in layer_ids {
        let mut sub = graph.layer_subgraph(layer);
        // The SAME five stages `tidy::tidy_layer_silent` runs on the live map, in the same
        // order (SQ-1376). `relayout_auto` alone is not the app's layout — it is the first
        // stage of it, and the four that follow are where a bearing the contiguity pass had to
        // break gets put back. Zork I's `Forest #91` is the specimen: the stress solve places
        // it due west of `West of House #68`, the hub-hole slide and the run tightening then
        // move #68 a column past it, and `repair_directional_hints` is what walks #91 back
        // west so the map still says what the game said. mapgen ran none of the four, so a
        // generated map and a played map disagreed about the same rooms with the same solver.
        // (The maze freeze `tidy_layer_silent` opens with is deliberately NOT copied — see this
        // function's own note above on why mapgen has no dead-reckoned positions to protect.)
        mapper::layout::relayout_auto(&mut sub);
        crate::render::map::cleanup_overlaps(&mut sub, 3, 40);
        crate::render::map::repair_directional_hints(&mut sub, 3, 40);
        crate::render::map::cleanup_overlaps(&mut sub, 3, 40);
        crate::render::map::compact_empty_lines(&mut sub);

        for id in graph.rooms_in_layer(layer) {
            if let Some(p) = sub.room(id).and_then(|r| r.pos) {
                graph.set_pos(id, p);
            }
        }

        let n = graph.connections().len();
        for idx in 0..n {
            let c = graph.connections()[idx].clone();
            if graph.layer_of(c.origin) == layer && graph.layer_of(c.dest) == layer {
                if let Some(sc) =
                    sub.connections().iter().find(|s| s.origin == c.origin && s.dir == c.dir && s.dest == c.dest)
                {
                    graph.set_conn_distorted(idx, sc.distorted);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared graph construction
// ---------------------------------------------------------------------------

/// A room the readers below have agreed on, before it becomes a graph node.
struct RawRoom {
    id: RoomId,
    name: String,
    engine_ref: EngineRef,
}

/// An edge the readers below have agreed on, before it becomes a connection.
struct RawEdge {
    origin: RoomId,
    dir: Direction,
    dest: RoomId,
    /// `Door` or `Conditional` where the source said so; `Declared` otherwise.
    /// `OneWay` is never set here — it is DERIVED once every edge is in, since
    /// no reader can know whether the reverse exists until the last one is read.
    kind: EdgeKind,
    via: Option<String>,
    note: Option<String>,
}

/// The note every [`EdgeKind::Routine`] edge carries (SQ-1334): the SAME text
/// regardless of which engine's reader found it, so a consumer never sees two
/// spellings of the same fact.
const ROUTINE_NOTE: &str = "decided by the story's code; the way back is declared";

/// From every declared exit that resolves to a destination — `(origin, dir,
/// dest)` triples — the `(destination, the direction that reaches it) ->
/// declaring room` lookup a `Code`/routine exit's own missing destination is
/// recovered through (SQ-1334): does anything declare a plain, door or
/// conditional exit BACK to a given room, travelling a given direction.
/// `or_insert` keeps the first (lowest room id, by iteration order) declarer
/// when more than one room declares the identical reverse.
///
/// Generic over the room-id type so the Z-machine's `u16` object numbers
/// (`zmachine_map`) and Glulx's `u32` addresses (`i6_glulx_map`) share the one
/// function — both readers already have their own declared-exits table in
/// exactly this `(origin, dir, dest)` shape by the time they call it.
fn declared_reverse_lookup<Id: Copy + Eq + std::hash::Hash>(
    declared: impl Iterator<Item = (Id, Direction, Id)>,
) -> HashMap<(Id, Direction), Id> {
    let mut declared_to: HashMap<(Id, Direction), Id> = HashMap::new();
    for (origin, dir, dest) in declared {
        declared_to.entry((dest, dir)).or_insert(origin);
    }
    declared_to
}

/// The room a `Code`/routine exit at `(obj, dir)` should be drawn to (SQ-1334):
/// [`declared_reverse_lookup`]'s answer to "does anything declare a plain,
/// door or conditional exit back to `obj`, travelling the direction opposite
/// `dir`". `None` when nothing does, in which case the exit stays undrawn —
/// Zork I's Kitchen has no such reverse for its own `Code` exit down to the
/// Studio (a CEXIT joke gated on a flag the game never sets, already drawn as
/// `Conditional` and untouched by this path) and a guessed destination would
/// be worse than a missing one.
fn routine_destination<Id: Copy + Eq + std::hash::Hash>(
    declared_to: &HashMap<(Id, Direction), Id>,
    obj: Id,
    dir: Direction,
) -> Option<Id> {
    declared_to.get(&(obj, mapper::direction::opposite(dir))).copied()
}

/// Turn agreed rooms and edges into a graph plus the facts beside it.
///
/// Every reader funnels through here, so room insertion order (and therefore
/// the discovery ordinal `#1`, `#2`, … that [`crate::roomid`] shows for a
/// synthetic room) is the source's own room order in every engine, and the
/// one-way derivation is done once rather than four times.
fn assemble(
    rooms: Vec<RawRoom>,
    edges: Vec<RawEdge>,
    source: SourceKind,
    story: StoryIdent,
) -> GeneratedMap {
    let mut graph = MapGraph::new();
    let mut engine_refs = BTreeMap::new();
    for r in rooms {
        graph.upsert_room(r.id, r.name);
        engine_refs.insert(r.id, r.engine_ref);
    }

    // Which (origin, dir, dest) triples exist at all, so the reverse lookup
    // below is a set membership test rather than a scan per edge.
    let declared: BTreeSet<(RoomId, RoomId)> =
        edges.iter().map(|e| (e.origin, e.dest)).collect();

    let mut facts = Vec::with_capacity(edges.len());
    for e in edges {
        // `add_edge` is the mapper's own upsert: it replaces an existing
        // connection in the same direction rather than doubling it, which is
        // what we want when a story declares the same passage twice (ZIL's
        // EAST and OUT commonly compile to identical DEXIT bytes).
        //
        // The edge carries its WEIGHT into the graph (`mapper::graph::PassageWeight`): a `Door`
        // or a `Conditional` exit is a passage the story GATES, so it is what the layout
        // surrenders first when a cycle closes and something must give. `Conditional` ranks
        // below `Door` because a door is a real walkable way through the geography that happens
        // to need opening, where a conditional exit is typically a secret the fiction wanted —
        // Zork I's magic-word `Strange Passage`, the rainbow (SQ-1312). `Routine` shares
        // `Conditional`'s weight and dotted styling (SQ-1334): its own direction is code, exactly
        // as unpredictable at layout time as a CEXIT's gate.
        let weight = match e.kind {
            EdgeKind::Door => mapper::graph::PassageWeight::Door,
            EdgeKind::Conditional | EdgeKind::Routine => mapper::graph::PassageWeight::Conditional,
            _ => mapper::graph::PassageWeight::Hard,
        };
        graph.add_edge_weighted(e.origin, e.dir, e.dest, weight);
        let reciprocal = declared.contains(&(e.dest, e.origin));
        let kind = match e.kind {
            // Only a plain declared edge is demoted to one-way: a door or a
            // conditional keeps its own, more specific, label and the
            // `reciprocal` flag beside it carries the rest.
            EdgeKind::Declared if !reciprocal => EdgeKind::OneWay,
            k => k,
        };
        facts.push(EdgeFact {
            origin: e.origin,
            dir: e.dir,
            dest: e.dest,
            kind,
            via: e.via,
            note: e.note,
        });
    }

    GeneratedMap {
        graph,
        source,
        story,
        facts,
        engine_refs,
        layout_time: None,
        // Filled in by `generate_with_options`, the only place that boots
        // anything; a map assembled straight from bytes has asked nobody.
        start: StartProbe::Skipped,
    }
}

/// Write each room's door, conditional and routine exits into its `notes`, so
/// the text dump says them.
///
/// [`crate::map_dump::render_dump`] prints `notes=` from the graph, and
/// [`mapper::graph::Connection`] has nowhere to put a per-edge annotation — so
/// rather than change the persisted graph format for a fact only this generator
/// ever produces, the facts are summarised onto the ORIGIN room in the same
/// `key=[…]` style as the dump's own `random=` and `dropped=` notes.
fn annotate_rooms(map: &mut GeneratedMap) {
    #[derive(Default)]
    struct Notes {
        doors: Vec<String>,
        conds: Vec<String>,
        routines: Vec<String>,
    }
    let mut by_room: BTreeMap<RoomId, Notes> = BTreeMap::new();
    for f in &map.facts {
        let entry = by_room.entry(f.origin).or_default();
        let dir = mapper::direction::short_label(f.dir).to_uppercase();
        match f.kind {
            EdgeKind::Door => {
                let via = f.via.as_deref().unwrap_or("door");
                entry.doors.push(format!("{dir}→{via:?}"));
            }
            EdgeKind::Conditional => {
                let note = f.note.as_deref().unwrap_or("condition unknown");
                entry.conds.push(format!("{dir}→({note})"));
            }
            // SQ-1334: the destination came from another room's declared
            // reverse, not from this room's own (unresolvable) code.
            EdgeKind::Routine => {
                let note = f.note.as_deref().unwrap_or("decided by the story's code");
                entry.routines.push(format!("{dir}→({note})"));
            }
            _ => {}
        }
    }
    for (id, notes) in by_room {
        let mut parts = Vec::new();
        if !notes.doors.is_empty() {
            parts.push(format!("door=[{}]", notes.doors.join(", ")));
        }
        if !notes.conds.is_empty() {
            parts.push(format!("conditional=[{}]", notes.conds.join(", ")));
        }
        if !notes.routines.is_empty() {
            parts.push(format!("routine=[{}]", notes.routines.join(", ")));
        }
        if !parts.is_empty() {
            map.graph.room_mut_notes(id, &parts.join(" "));
        }
    }
}

// ---------------------------------------------------------------------------
// Z-machine: the Inform 6 library and ZIL
// ---------------------------------------------------------------------------

/// [`Direction`] as the Z-machine world model indexes its exit table.
fn z_compass(d: Direction) -> Option<zvm::world::Compass> {
    use zvm::world::Compass as C;
    Some(match d {
        Direction::N => C::N,
        Direction::S => C::S,
        Direction::E => C::E,
        Direction::W => C::W,
        Direction::NE => C::Ne,
        Direction::NW => C::Nw,
        Direction::SE => C::Se,
        Direction::SW => C::Sw,
        Direction::Up => C::Up,
        Direction::Down => C::Down,
        Direction::In => C::In,
        Direction::Out => C::Out,
        Direction::Unknown => return None,
    })
}

fn zmachine_map(bytes: &[u8], file: String) -> Result<GeneratedMap, GenError> {
    let mem = zvm::memory::Memory::new(bytes.to_vec())
        .map_err(|e| GenError::Engine(format!("not a readable Z-machine story: {e:?}")))?;
    let wm = zvm::world::WorldModel::discover(&mem);

    // Which convention answered. `discover` fills `exit_props` from the Inform
    // library's `door_dir`/`*_to` and falls back to `zil_exit_props`; the two
    // are never both populated, so whichever is non-empty names the source.
    let source = if wm.exit_props.iter().any(Option::is_some) {
        SourceKind::I6Library
    } else if wm.zil_exit_props.iter().any(Option::is_some) {
        SourceKind::Zil
    } else {
        return Err(GenError::NoStaticSource(
            "no static map source: this Z-machine story declares neither the Inform 6 \
             library's door_dir/*_to exit convention nor ZIL's exit properties, so nothing \
             in the file says where its rooms lead"
                .into(),
        ));
    };

    // Pass 1: ask every object what it declares in every direction. An object
    // that declares ANYTHING at all — even a refusal message or a routine — is
    // using the story's exit convention, and nothing but a room does that.
    //
    // This is the room-set derivation for both Z-machine sources: the objects
    // that OWN an exit, plus the objects an exit LEADS TO. The second half
    // matters because a pure destination — a room whose own exits are all
    // computed — declares nothing and would otherwise be missing from its own
    // map. Neither half needs the object tree, which is why this works
    // identically for ZIL (whose rooms are not parented to a "rooms" object the
    // way Inform's are).
    let mut declares: BTreeMap<u16, Vec<(Direction, zvm::world::ExitDetail)>> = BTreeMap::new();
    let mut destinations: BTreeSet<u16> = BTreeSet::new();
    for obj in 1..=wm.max_object {
        let mut here = Vec::new();
        for dir in DIRS {
            let Some(c) = z_compass(dir) else { continue };
            let detail = wm.declared_exit_detail(&mem, obj, c);
            if matches!(detail, zvm::world::ExitDetail::Absent | zvm::world::ExitDetail::Unknown) {
                continue;
            }
            if let Some(d) = detail.destination() {
                destinations.insert(d);
            }
            here.push((dir, detail));
        }
        if !here.is_empty() {
            declares.insert(obj, here);
        }
    }

    // SQ-1311: an object with no printed name whose ENTIRE declared exit list
    // leads only to ITSELF (or nowhere the derivation can resolve — a routine
    // or a refusal message) is not a room a player could ever stand in. Zork
    // I's object #41 declares nothing but `IN -> 41` — a pseudo-room with no
    // name and no way out — which nonetheless "declares an exit" and would
    // otherwise pass the derivation above. Narrow on purpose: an unnamed room
    // with a real exit elsewhere stays (it may still be named at runtime, or
    // simply never printed), and so does one some OTHER object genuinely
    // leads to — that is a real destination, whatever this object calls
    // itself, and excluding it would leave that edge dangling.
    let mut pseudo_rooms: BTreeSet<u16> = BTreeSet::new();
    for (&obj, here) in &declares {
        if !zvm::objects::printed_name(&mem, obj).trim().is_empty() {
            continue;
        }
        let self_or_nowhere = here.iter().all(|&(_, detail)| match detail.destination() {
            None => true,
            Some(d) => d == obj,
        });
        if self_or_nowhere {
            pseudo_rooms.insert(obj);
        }
    }
    pseudo_rooms.retain(|&obj| {
        !declares.iter().any(|(&other, here)| {
            other != obj && here.iter().any(|&(_, detail)| detail.destination() == Some(obj))
        })
    });
    declares.retain(|obj, _| !pseudo_rooms.contains(obj));
    destinations.retain(|d| !pseudo_rooms.contains(d));

    let room_set: BTreeSet<u16> =
        declares.keys().copied().chain(destinations.iter().copied()).collect();
    if room_set.is_empty() {
        return Err(GenError::NoStaticSource(format!(
            "no static map source: the {} exit convention was identified but no object \
             declares an exit, so this story's map is not in its object table",
            source.as_str()
        )));
    }

    let rooms: Vec<RawRoom> = room_set
        .iter()
        .map(|&obj| RawRoom {
            id: obj as RoomId,
            name: zvm::objects::printed_name(&mem, obj),
            engine_ref: EngineRef::ZObject(obj),
        })
        .collect();

    // SQ-1334: a `Code` exit's own destination is unresolvable — the story
    // computes it in a routine — but the story's exit table can still say the
    // passage is real, when some OTHER room declares a plain/door/conditional
    // exit back to this one in the opposite direction. The Living Room's own
    // Down is a ZIL FEXIT (`TRAP-DOOR-EXIT`) and therefore `Code`, but the
    // Cellar declares a plain UP exit to the Living Room — the way back is
    // declared even though the way there is code, so `Living Room D → Cellar`
    // is drawn, one-way, as [`EdgeKind::Routine`]. See `declared_reverse_lookup`.
    let declared_to: HashMap<(u16, Direction), u16> = declared_reverse_lookup(declares.iter().flat_map(
        |(&obj, declared)| {
            declared.iter().filter_map(move |&(dir, detail)| detail.destination().map(|dest| (obj, dir, dest)))
        },
    ));

    let mut edges = Vec::new();
    for (&obj, declared) in &declares {
        for &(dir, detail) in declared {
            let (dest, kind, via, note) = match detail {
                zvm::world::ExitDetail::Room(d) => (d, EdgeKind::Declared, None, None),
                zvm::world::ExitDetail::Door { dest, door } => (
                    dest,
                    EdgeKind::Door,
                    Some(zvm::objects::printed_name(&mem, door)),
                    None,
                ),
                // Deliberately no claim about WHAT the condition is: the
                // CEXIT's gate byte is not the global's variable number (see
                // [`zvm::world::ExitDetail::Conditional`] for the Zork I
                // evidence), and naming a global we cannot identify would put
                // a confident falsehood in a reference artefact.
                zvm::world::ExitDetail::Conditional { dest, .. } => (
                    dest,
                    EdgeKind::Conditional,
                    None,
                    Some("open only while the story allows it (ZIL CEXIT)".to_string()),
                ),
                // SQ-1334: a `Code` exit whose destination some other room's
                // own declared exit gives away (see `declared_to` above).
                zvm::world::ExitDetail::Code => match routine_destination(&declared_to, obj, dir) {
                    Some(dest) => (
                        dest,
                        EdgeKind::Routine,
                        None,
                        Some(ROUTINE_NOTE.to_string()),
                    ),
                    None => continue,
                },
                // A refusal message, or a routine with no declared way back:
                // real map data, but not a passage anything static can draw.
                // Deliberately no edge — a guessed one would be worse than a
                // missing one.
                zvm::world::ExitDetail::Message
                | zvm::world::ExitDetail::Absent
                | zvm::world::ExitDetail::Unknown => continue,
            };
            edges.push(RawEdge { origin: obj as RoomId, dir, dest: dest as RoomId, kind, via, note });
        }
    }

    let story = z_story_ident(bytes, file);
    Ok(assemble(rooms, edges, source, story))
}

/// A Z-machine story's own header identity (ZMSD §11.1): release at $02 (word),
/// serial at $12..$18 (six ASCII digits), checksum at $1C (word). Read here
/// rather than through `header::parse_header`, which does not carry them.
///
/// Shared by [`zmachine_map`] and the live app's own `/export-json` (SQ-1336),
/// which has no [`GeneratedMap`] to read a [`StoryIdent`] off of — a running
/// session's raw story bytes are exactly what this wants.
pub fn z_story_ident(bytes: &[u8], file: String) -> StoryIdent {
    StoryIdent {
        file,
        engine: "z-machine",
        release: (bytes.len() > 0x03).then(|| u16::from_be_bytes([bytes[0x02], bytes[0x03]])),
        serial: (bytes.len() >= 0x18)
            .then(|| String::from_utf8_lossy(&bytes[0x12..0x18]).into_owned()),
        checksum: (bytes.len() > 0x1D)
            .then(|| format!("0x{:04x}", u16::from_be_bytes([bytes[0x1C], bytes[0x1D]]))),
    }
}

// ---------------------------------------------------------------------------
// Glulx: Inform 7's Map_Storage, then the Inform 6 library
// ---------------------------------------------------------------------------

/// [`Direction`] as the Glulx world models index their exit tables.
fn g_compass(d: Direction) -> Option<gvm::world::Compass> {
    use gvm::world::Compass as C;
    Some(match d {
        Direction::N => C::N,
        Direction::S => C::S,
        Direction::E => C::E,
        Direction::W => C::W,
        Direction::NE => C::Ne,
        Direction::NW => C::Nw,
        Direction::SE => C::Se,
        Direction::SW => C::Sw,
        Direction::Up => C::Up,
        Direction::Down => C::Down,
        Direction::In => C::In,
        Direction::Out => C::Out,
        Direction::Unknown => return None,
    })
}

/// The reverse: which of our directions a Glulx `Compass` is.
fn g_direction(c: gvm::world::Compass) -> Direction {
    use gvm::world::Compass as C;
    match c {
        C::N => Direction::N,
        C::S => Direction::S,
        C::E => Direction::E,
        C::W => Direction::W,
        C::Ne => Direction::NE,
        C::Nw => Direction::NW,
        C::Se => Direction::SE,
        C::Sw => Direction::SW,
        C::Up => Direction::Up,
        C::Down => Direction::Down,
        C::In => Direction::In,
        C::Out => Direction::Out,
    }
}

fn glulx_map(bytes: &[u8], file: String) -> Result<GeneratedMap, GenError> {
    let mem = gvm::memory::Memory::new(bytes.to_vec())
        .map_err(|e| GenError::Engine(format!("not a readable Glulx story: {e:?}")))?;
    let names = gvm::objects::ParseNames::detect(&mem)
        .map_err(|e| GenError::Engine(format!("Glulx object table not readable: {e:?}")))?;

    let story = glulx_story_ident(bytes, file);

    // Inform 7's own map table first — it is the higher authority for any story
    // that has one, since it is what the I7 runtime itself reads.
    if let Some(w) = gvm::i7map::I7World::detect(&mem, &names) {
        return Ok(i7_map(&mem, &names, &w, story));
    }
    i6_glulx_map(&mem, &names, story)
}

/// A Glulx image's own header identity: the whole-image checksum (Glulx spec
/// §1.4, offset 0x20 — every well-formed image has one) and, when the Inform
/// compiler's own `Info` block is present (magic at 0x24 confirms it —
/// Glulx-Inform-Tech.html §1 "Static Data"), the release and serial it carries.
/// `None` for a bare non-Inform Glulx image, which has neither.
///
/// Shared by [`glulx_map`] and the live app's own `/export-json` (SQ-1336),
/// same reason as [`z_story_ident`] — read straight off the raw image bytes
/// rather than through a parsed [`gvm::memory::Memory`], which the live path
/// has no reason to build again just for this.
pub fn glulx_story_ident(bytes: &[u8], file: String) -> StoryIdent {
    let checksum = (bytes.len() >= 0x24).then(|| {
        format!("0x{:08x}", u32::from_be_bytes([bytes[0x20], bytes[0x21], bytes[0x22], bytes[0x23]]))
    });
    let (release, serial) = match gvm::header::parse_inform_info(bytes) {
        Some(info) => (Some(info.release), Some(info.serial)),
        None => (None, None),
    };
    StoryIdent { file, engine: "glulx", release, serial, checksum }
}

fn i7_map(
    mem: &gvm::memory::Memory,
    names: &gvm::objects::ParseNames,
    w: &gvm::i7map::I7World,
    story: StoryIdent,
) -> GeneratedMap {
    // SQ-1334's `Code`/`Routine` reconciliation is N/A here: `Map_Storage` is
    // a plain per-room, per-direction DATA table, not compiled branches, so
    // [`gvm::i7map::I7Exit`] has nothing shaped like a routine to begin with —
    // one cell is a room, a door, or absent (filtered out below), never code.
    let name_of = |addr: u32| w.printed_name(mem, names, addr).unwrap_or_default();

    // The room set is the story's own: `Map_Storage` is indexed by room, so
    // `I7World::rooms()` IS the complete list and nothing has to be derived.
    let mut rooms: Vec<RawRoom> = w
        .rooms()
        .iter()
        .map(|&addr| RawRoom {
            id: crate::roomid::glulx_room_id(addr),
            name: name_of(addr),
            engine_ref: EngineRef::GlulxAddr(addr),
        })
        .collect();

    let mut edges = Vec::new();
    let mut door_nodes: BTreeMap<u32, RoomId> = BTreeMap::new();
    for &addr in w.rooms() {
        let origin = crate::roomid::glulx_room_id(addr);
        for (compass, _dir_obj, exit) in w.exits(mem, names, addr) {
            // A direction the story declares but no compass word names — an I7
            // author's own "port" or "starboard". Real, but there is no
            // `mapper::Direction` for it and inventing one would put a passage
            // on the map's compass that the player cannot type.
            let Some(c) = compass else { continue };
            let dir = g_direction(c);
            let (dest, kind, via, note) = match exit {
                gvm::i7map::I7Exit::Room(r) => {
                    (crate::roomid::glulx_room_id(r), EdgeKind::Declared, None, None)
                }
                gvm::i7map::I7Exit::ThroughDoor { door, to } => (
                    crate::roomid::glulx_room_id(to),
                    EdgeKind::Door,
                    Some(name_of(door)),
                    None,
                ),
                // The story names a door and will not say statically what is
                // behind it — a one-sided door, or one whose far side is
                // computed. The edge leads to the DOOR, which is a true fact,
                // rather than to a guessed room, which would not be.
                gvm::i7map::I7Exit::Door(door) => {
                    let id = crate::roomid::glulx_room_id(door);
                    door_nodes.insert(door, id);
                    (
                        id,
                        EdgeKind::Door,
                        Some(name_of(door)),
                        Some("far side not declared statically".to_string()),
                    )
                }
            };
            edges.push(RawEdge { origin, dir, dest, kind, via, note });
        }
    }

    // Door stand-in nodes are appended AFTER every real room, so a room's
    // discovery ordinal is never displaced by one.
    for (addr, id) in door_nodes {
        if !w.is_room(addr) {
            rooms.push(RawRoom {
                id,
                name: name_of(addr),
                engine_ref: EngineRef::GlulxAddr(addr),
            });
        }
    }

    assemble(rooms, edges, SourceKind::I7World, story)
}

fn i6_glulx_map(
    mem: &gvm::memory::Memory,
    names: &gvm::objects::ParseNames,
    story: StoryIdent,
) -> Result<GeneratedMap, GenError> {
    let wm = gvm::world::WorldModel::discover(mem, names);

    // Same derivation as the Z-machine's, and for the same reason: rooms are
    // the objects that declare an exit, plus the objects an exit leads to.
    // `gvm::world` exposes no room list at all, so there is nothing else to
    // use — and unlike `zvm::world` it has no `ExitDetail`, so an Inform door
    // on this path is resolved through `door_to` and reported as an ordinary
    // room: the passage is right, and the fact that a door stands in it is lost.
    let mut declares: BTreeMap<u32, Vec<(Direction, u32)>> = BTreeMap::new();
    let mut destinations: BTreeSet<u32> = BTreeSet::new();
    let mut any_declaration = false;
    // SQ-1334: `Code` carries no destination of its own — `gvm::world` has no
    // `ExitDetail`-shaped alternative — but it may still be a real passage
    // when some OTHER room's plain exit declares the way back, in the
    // opposite direction (see `declared_to` below). Kept separately from
    // `declares`, which stays exactly the plain-exit table it always was.
    let mut code_exits: Vec<(u32, Direction)> = Vec::new();
    for obj in names.objects() {
        let mut here = Vec::new();
        for dir in DIRS {
            let Some(c) = g_compass(dir) else { continue };
            match wm.declared_exit(mem, names, obj, c) {
                gvm::world::DeclaredExit::Room(d) => {
                    destinations.insert(d);
                    here.push((dir, d));
                }
                gvm::world::DeclaredExit::Code => {
                    any_declaration = true;
                    code_exits.push((obj, dir));
                }
                gvm::world::DeclaredExit::Message => {
                    any_declaration = true;
                }
                gvm::world::DeclaredExit::Absent | gvm::world::DeclaredExit::Unknown => {}
            }
        }
        if !here.is_empty() {
            any_declaration = true;
            declares.insert(obj, here);
        }
    }

    if !any_declaration {
        return Err(GenError::NoStaticSource(
            "no static map source: this Glulx story carries no Inform 7 Map_Storage table \
             (so it is not an I7 build this reader recognises) and declares no Inform 6 \
             library door_dir/*_to exits either"
                .into(),
        ));
    }

    let room_set: BTreeSet<u32> =
        declares.keys().copied().chain(destinations.iter().copied()).collect();
    let rooms: Vec<RawRoom> = room_set
        .iter()
        .map(|&addr| RawRoom {
            id: crate::roomid::glulx_room_id(addr),
            name: names.printed_name(mem, addr).unwrap_or_default(),
            engine_ref: EngineRef::GlulxAddr(addr),
        })
        .collect();

    let mut edges: Vec<RawEdge> = declares
        .iter()
        .flat_map(|(&obj, ds)| {
            ds.iter().map(move |&(dir, dest)| RawEdge {
                origin: crate::roomid::glulx_room_id(obj),
                dir,
                dest: crate::roomid::glulx_room_id(dest),
                kind: EdgeKind::Declared,
                via: None,
                note: None,
            })
        })
        .collect();

    // SQ-1334: the same reconciliation `zmachine_map` does for a ZIL FEXIT —
    // built over the same `declares` table the loop above already reads, so a
    // `Code` exit at `(obj, dir)` looks up whichever room declares a plain
    // exit back to `obj` in the opposite direction. See `declared_reverse_lookup`.
    let declared_to: HashMap<(u32, Direction), u32> =
        declared_reverse_lookup(declares.iter().flat_map(|(&obj, ds)| {
            ds.iter().map(move |&(dir, dest)| (obj, dir, dest))
        }));
    for (obj, dir) in code_exits {
        if let Some(dest) = routine_destination(&declared_to, obj, dir) {
            edges.push(RawEdge {
                origin: crate::roomid::glulx_room_id(obj),
                dir,
                dest: crate::roomid::glulx_room_id(dest),
                kind: EdgeKind::Routine,
                via: None,
                note: Some(ROUTINE_NOTE.to_string()),
            });
        }
    }

    Ok(assemble(rooms, edges, SourceKind::I6Library, story))
}

// ---------------------------------------------------------------------------
// Scott Adams
// ---------------------------------------------------------------------------

/// A Scott Adams room's six exit slots, in the order the database stores them
/// (`crates/scott/src/database.rs`: `exits: [usize; 6]`).
const SCOTT_DIRS: [Direction; 6] = [
    Direction::N,
    Direction::S,
    Direction::E,
    Direction::W,
    Direction::Up,
    Direction::Down,
];

fn scott_map(bytes: &[u8], file: String) -> Result<GeneratedMap, GenError> {
    let src = std::str::from_utf8(bytes)
        .map_err(|e| GenError::Engine(format!("Scott Adams database is not text: {e}")))?;
    let db = scott::Database::parse(src)
        .map_err(|e| GenError::Engine(format!("unreadable Scott Adams database: {e:?}")))?;

    // Room 0 is the format's "no room" sentinel — an exit slot holding 0 means
    // "no exit that way" (`scott::Vm`'s own move check is `dest != 0`), so
    // nothing can ever lead to room 0 and it is not a place. Every other index
    // is a room, and the table is complete: a Scott database lists its whole
    // map with no inference at all.
    let rooms: Vec<RawRoom> = (1..db.rooms.len())
        .map(|i| RawRoom {
            id: i as RoomId,
            // The database's own description, which is exactly what the live
            // Scott adapter names a room with (`scott_session.rs` calls
            // `Vm::room_name`, which is this string) — so a static map and a
            // played one name the same room the same way.
            name: db.rooms[i].desc.clone(),
            engine_ref: EngineRef::ScottIndex(i),
        })
        .collect();

    let mut edges = Vec::new();
    for (i, room) in db.rooms.iter().enumerate().skip(1) {
        for (slot, dir) in SCOTT_DIRS.iter().enumerate() {
            let dest = room.exits[slot];
            if dest == 0 || dest >= db.rooms.len() {
                continue;
            }
            edges.push(RawEdge {
                origin: i as RoomId,
                dir: *dir,
                dest: dest as RoomId,
                kind: EdgeKind::Declared,
                via: None,
                note: None,
            });
        }
    }

    let story = StoryIdent {
        file,
        engine: "scott",
        // A Scott Adams database has no release, serial or checksum field at
        // all (SQ-1306) — the trailer's adventure number is a title id, not a
        // build identity, so it does not belong in any of these three.
        release: None,
        serial: None,
        checksum: None,
    };
    Ok(assemble(rooms, edges, SourceKind::Scott, story))
}

// ---------------------------------------------------------------------------
// Artefacts
// ---------------------------------------------------------------------------

/// Which artefacts to write. All four by default.
#[derive(Debug, Clone, Copy)]
pub struct Artefacts {
    pub dump: bool,
    pub svg: bool,
    pub dot: bool,
    pub json: bool,
}

impl Default for Artefacts {
    fn default() -> Self {
        Self { dump: true, svg: true, dot: true, json: true }
    }
}

impl Artefacts {
    /// True when no artefact was named, meaning "write them all".
    pub fn none_selected(&self) -> bool {
        !self.dump && !self.svg && !self.dot && !self.json
    }
}

/// Write the selected artefacts for `map` into `out_dir`, named `<stem>.*`.
/// Returns the paths written, in the order they were written.
pub fn write_artefacts(
    map: &GeneratedMap,
    out_dir: &Path,
    stem: &str,
    what: Artefacts,
) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir)?;
    let mut written = Vec::new();

    if what.dump {
        let p = out_dir.join(format!("{stem}.map.txt"));
        std::fs::write(
            &p,
            crate::map_dump::render_dump_with_header(
                &map.graph,
                &crate::symbols::SymbolSet::default(),
                // SQ-1359: which room the story starts in — the one fact
                // behind this map's choice of `Main` that the graph itself has
                // nowhere to record.
                &[map.start.header_line()],
            ),
        )?;
        written.push(p);
    }
    if what.svg {
        let p = out_dir.join(format!("{stem}.svg"));
        // SQ-1392: `map.start` (line 1868, above) is a starting point for the highlight, not a
        // player — the generated form of the legend says so instead of the live "you are in" text.
        std::fs::write(&p, crate::export_svg::render_svg_layered_generated(&map.graph))?;
        written.push(p);
    }
    if what.dot {
        let p = out_dir.join(format!("{stem}.dot"));
        std::fs::write(&p, crate::export_dot::render_dot(&map.graph))?;
        written.push(p);
    }
    if what.json {
        let p = out_dir.join(format!("{stem}.map.json"));
        std::fs::write(&p, render_json(map))?;
        written.push(p);
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// The JSON map
// ---------------------------------------------------------------------------

/// The `format` string every `.map.json` carries. A consumer should refuse a
/// file whose `format` is not this.
pub const JSON_FORMAT: &str = "lanthorn-map";

/// The `version` every `.map.json` carries. Bump it only for a change that a
/// version-1 reader could not survive — adding a field is not one, since the
/// format asks consumers to ignore what they do not recognise. SQ-1359's
/// `start_room` is exactly such an addition and does NOT bump this.
pub const JSON_VERSION: u32 = 1;

#[derive(serde::Serialize)]
struct JsonMap<'a> {
    format: &'static str,
    version: u32,
    generator: JsonGenerator,
    story: JsonStory<'a>,
    /// The room the story starts the player in, spelled as `rooms[].id` spells
    /// it, or `null` when nothing could say (SQ-1359). It is the room the map's
    /// `Main` layer was chosen around, and the one a drawing highlights.
    start_room: Option<String>,
    directions: Vec<JsonDirection>,
    rooms: Vec<JsonRoom>,
    edges: Vec<JsonEdge>,
    layers: Vec<JsonLayer>,
}

#[derive(serde::Serialize)]
struct JsonGenerator {
    name: &'static str,
    version: &'static str,
}

#[derive(serde::Serialize)]
struct JsonStory<'a> {
    file: &'a str,
    engine: &'a str,
    source: &'static str,
    release: Option<u16>,
    serial: Option<&'a str>,
    checksum: Option<&'a str>,
    generated_at: String,
}

#[derive(serde::Serialize)]
struct JsonDirection {
    /// The canonical lowercase word an edge's `dir` uses.
    word: &'static str,
    /// Short tag, as the matrix view and the text dump spell it.
    short: &'static str,
    /// Compass bearing in degrees, north = 0, clockwise. Null for up, down,
    /// in and out, which are not compass directions and have no bearing.
    bearing: Option<u16>,
}

#[derive(serde::Serialize)]
struct JsonPos {
    x: i32,
    y: i32,
}

#[derive(serde::Serialize)]
struct JsonRoom {
    id: String,
    raw_id: RoomId,
    name: String,
    ordinal: u64,
    layer: u16,
    /// The mapper's LOGICAL grid cell — one unit is one room step, not a pixel
    /// and not a terminal cell. Null when the map was generated with no layout.
    pos: Option<JsonPos>,
    flags: Vec<&'static str>,
    engine_ref: JsonEngineRef,
}

#[derive(serde::Serialize)]
struct JsonEngineRef {
    kind: &'static str,
    /// A Z-machine object number or a Scott room index, decimal.
    number: Option<u64>,
    /// A Glulx object address, hex with an `0x` prefix.
    address: Option<String>,
}

#[derive(serde::Serialize)]
struct JsonEdge {
    from: String,
    to: String,
    dir: &'static str,
    kind: &'static str,
    reciprocal: bool,
    via: Option<String>,
    note: Option<String>,
}

#[derive(serde::Serialize)]
struct JsonLayer {
    id: u16,
    name: String,
    maze: bool,
    rooms: usize,
}

/// The room's id as every lanthorn surface spells it — `#12` for a synthetic
/// (Glulx/Scott) room's ordinal, `#136` for a Z-machine object number.
fn json_room_id(graph: &MapGraph, id: RoomId) -> String {
    crate::roomid::room_label_no(graph, id)
}

/// An RFC 3339 UTC timestamp, hand-formatted from the wall clock.
///
/// `app` has no date library and this is the only place anything here needs a
/// date, so the civil-calendar conversion is spelled out rather than adding a
/// dependency for one line. Days-from-epoch to a Gregorian date is Howard
/// Hinnant's `civil_from_days`, which is exact for every date in range.
fn rfc3339_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);

    // civil_from_days: shift the epoch to 0000-03-01 so leap day is last.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// Everything [`render_json_view`] needs, borrowed rather than owned, so both
/// [`render_json`] (a mapgen [`GeneratedMap`]) and the live app's own
/// `/export-json` (SQ-1336, over a played [`MapGraph`] with no `GeneratedMap`
/// to speak of) feed the one serialiser — one JSON writer, not two that can
/// drift apart on what a "declared" edge or a room's `engine_ref` means.
pub struct JsonMapView<'a> {
    /// The binary that produced the file (`"lanthorn-mapgen"` or `"lanthorn"`).
    pub generator: &'static str,
    pub graph: &'a MapGraph,
    pub story: &'a StoryIdent,
    /// The `story.source` tag — mapgen's own [`SourceKind::as_str`], or
    /// `"walked"` for a map read off actual play.
    pub source: &'static str,
    pub facts: &'a [EdgeFact],
    pub engine_refs: &'a BTreeMap<RoomId, EngineRef>,
    /// The story's starting room (SQ-1359), for the JSON's `start_room`.
    /// `None` for the live app's `/export-json`, which exports a map read off
    /// actual play: its `current` room is wherever the player is standing NOW,
    /// which is a different fact and must not be written into this field.
    pub start_room: Option<RoomId>,
}

/// Render `map` as the versioned, self-describing JSON map.
///
/// The schema is documented in `docs/internals/mapping.md`. Nothing
/// lanthorn-internal goes in: no seam decisions, no render slots, no terminal
/// cells — only what a tool that has never seen lanthorn could use.
pub fn render_json(map: &GeneratedMap) -> String {
    render_json_view(&JsonMapView {
        generator: "lanthorn-mapgen",
        graph: &map.graph,
        story: &map.story,
        source: map.source.as_str(),
        facts: &map.facts,
        engine_refs: &map.engine_refs,
        start_room: map.start.room().map(|r| r.id),
    })
}

/// [`render_json`], over a [`JsonMapView`] rather than a [`GeneratedMap`] — see
/// that type's docs for why there are two callers and one serialiser.
pub fn render_json_view(view: &JsonMapView) -> String {
    let graph = view.graph;

    let directions = DIRS
        .iter()
        .map(|&d| JsonDirection {
            word: mapper::direction::long_label(d),
            short: mapper::direction::short_label(d),
            bearing: mapper::direction::bearing(d),
        })
        .collect();

    let rooms: Vec<JsonRoom> = graph
        .rooms()
        .map(|r| {
            let engine_ref = match view.engine_refs.get(&r.id) {
                Some(EngineRef::ZObject(n)) => JsonEngineRef {
                    kind: "z-object",
                    number: Some(*n as u64),
                    address: None,
                },
                Some(EngineRef::ScottIndex(i)) => JsonEngineRef {
                    kind: "scott-room",
                    number: Some(*i as u64),
                    address: None,
                },
                Some(EngineRef::GlulxAddr(a)) => JsonEngineRef {
                    kind: "glulx-object",
                    number: None,
                    address: Some(format!("0x{a:08x}")),
                },
                None => JsonEngineRef { kind: "none", number: None, address: None },
            };
            let mut flags = Vec::new();
            if graph.layer_is_maze(r.layer) {
                flags.push("maze");
            }
            JsonRoom {
                id: json_room_id(graph, r.id),
                raw_id: r.id,
                name: r.label().to_string(),
                ordinal: r.ordinal(),
                layer: r.layer,
                pos: r.pos.map(|(x, y)| JsonPos { x, y }),
                flags,
                engine_ref,
            }
        })
        .collect();

    let edges: Vec<JsonEdge> = view
        .facts
        .iter()
        .map(|f| {
            let reciprocal = view
                .facts
                .iter()
                .any(|g| g.origin == f.dest && g.dest == f.origin);
            JsonEdge {
                from: json_room_id(graph, f.origin),
                to: json_room_id(graph, f.dest),
                dir: if f.dir == Direction::Unknown {
                    "?"
                } else {
                    mapper::direction::long_label(f.dir)
                },
                kind: f.kind.as_str(),
                reciprocal,
                via: f.via.clone(),
                note: f.note.clone(),
            }
        })
        .collect();

    let layers: Vec<JsonLayer> = graph
        .layers()
        .keys()
        .map(|&id| JsonLayer {
            id,
            name: graph.layer_name(id).to_string(),
            maze: graph.layer_is_maze(id),
            rooms: graph.rooms_in_layer(id).len(),
        })
        .collect();

    let doc = JsonMap {
        format: JSON_FORMAT,
        version: JSON_VERSION,
        generator: JsonGenerator { name: view.generator, version: buildinfo::LONG },
        story: JsonStory {
            file: &view.story.file,
            engine: view.story.engine,
            source: view.source,
            release: view.story.release,
            serial: view.story.serial.as_deref(),
            checksum: view.story.checksum.as_deref(),
            generated_at: rfc3339_now(),
        },
        start_room: view.start_room.map(|id| json_room_id(graph, id)),
        directions,
        rooms,
        edges,
        layers,
    };

    // `to_string_pretty` because these files are read, diffed and checked in.
    serde_json::to_string_pretty(&doc).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}")) + "\n"
}

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;

    /// SQ-1334's reconciliation, on plain `(origin, dir, dest)` triples rather than a full story
    /// fixture: Living Room's own Down is `Code` (unresolvable on its own), but the Cellar
    /// declares a plain Up exit to the Living Room — the way back — so the Down exit's
    /// destination is recovered as the Cellar.
    #[test]
    fn a_code_exits_destination_is_recovered_from_the_reverse_declared_exit() {
        // The Cellar (34) declares UP -> Living Room (79); nothing else is declared here at all,
        // in particular nothing declares Living Room's own Down.
        let declared_to =
            declared_reverse_lookup([(34u16, Direction::Up, 79u16)].into_iter());
        assert_eq!(
            routine_destination(&declared_to, 79, Direction::Down),
            Some(34),
            "Living Room's Code exit down recovers the Cellar from the Cellar's own declared Up"
        );
    }

    /// The falsifier for the rule above: a `Code` exit with no declared reverse anywhere stays
    /// undrawn — Zork I's Kitchen has a `Code`-shaped joke exit down to the Studio with nothing
    /// declaring the way back, and inventing a destination would be worse than missing one.
    #[test]
    fn a_code_exit_with_no_declared_reverse_recovers_nothing() {
        // Nothing at all declares an exit back to the Kitchen (28) from the north (the reverse
        // of a southbound Code exit) — only an unrelated declaration elsewhere.
        let declared_to =
            declared_reverse_lookup([(1u16, Direction::N, 2u16)].into_iter());
        assert_eq!(routine_destination(&declared_to, 28, Direction::S), None);
    }

    /// When two different rooms both declare the reverse, the lookup is deterministic: the FIRST
    /// one in iteration order wins, not whichever happens to be inserted last by hash order.
    #[test]
    fn two_declared_reverses_pick_the_first_in_iteration_order() {
        let declared_to = declared_reverse_lookup(
            [(10u16, Direction::Up, 5u16), (20u16, Direction::Up, 5u16)].into_iter(),
        );
        assert_eq!(routine_destination(&declared_to, 5, Direction::Down), Some(10));
    }

    /// SQ-1359, on a synthetic graph so the rule is stated without a story in the way: two
    /// compass-connected components joined by one portal, the big one five rooms and the small
    /// one exactly `layer_min`. With NOTHING saying where play begins, the big one is Main —
    /// the only answer available, and the one mapgen gave before this quest.
    #[test]
    fn with_no_start_room_the_largest_component_is_still_main() {
        let mut g = two_components();
        let splits = split_layers(&mut g, &MapgenOptions { layer_min: 4, ..Default::default() });
        assert_eq!(g.layer_of(1), mapper::layer::MAIN_LAYER, "the five-room component is Main");
        assert_ne!(g.layer_of(10), mapper::layer::MAIN_LAYER, "the four-room component peels off");
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].name, "Cellar", "a peel is named after the room its portal enters");
    }

    /// The same graph with the START ROOM in the SMALL component — which is what
    /// `generate_with_options` records as the graph's `current` before calling this. Now the
    /// small component is Main and the big one peels off, which is the whole of the fix: Zork I's
    /// underground is bigger than its surface, and the surface is where the player stands.
    #[test]
    fn the_component_holding_the_start_room_is_main_however_small_it_is() {
        let mut g = two_components();
        g.set_current(10); // the small component's own entry room
        let splits = split_layers(&mut g, &MapgenOptions { layer_min: 4, ..Default::default() });
        assert_eq!(g.layer_of(10), mapper::layer::MAIN_LAYER, "the START's component is Main");
        assert_ne!(
            g.layer_of(1),
            mapper::layer::MAIN_LAYER,
            "the LARGEST component peels off when it is not the start's"
        );
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].name, "West of House", "the peel is named after its own entry room");
    }

    /// Five rooms one way, four the other, one staircase between them. Nothing here mentions a
    /// maze, so pass 1 never fires and pass 2 sees both components whole.
    fn two_components() -> MapGraph {
        let mut g = MapGraph::new();
        for (id, name) in [
            (1, "West of House"),
            (2, "North of House"),
            (3, "Behind House"),
            (4, "South of House"),
            (5, "Forest"),
            (10, "Cellar"),
            (11, "Troll Room"),
            (12, "East-West Passage"),
            (13, "Round Room"),
        ] {
            g.upsert_room(id, name.to_string());
        }
        for (a, b) in [(1, 2), (2, 3), (3, 4), (4, 5)] {
            g.add_edge(a, Direction::E, b);
            g.add_edge(b, Direction::W, a);
        }
        for (a, b) in [(10, 11), (11, 12), (12, 13)] {
            g.add_edge(a, Direction::E, b);
            g.add_edge(b, Direction::W, a);
        }
        // The one link between them is a PORTAL, so `planar_region` cuts here and the two are
        // separate components — which is the situation pass 2 exists to resolve.
        g.add_edge(1, Direction::Down, 10);
        g.add_edge(10, Direction::Up, 1);
        g
    }

    // ── SQ-1361: `name_region_by_entry` names a peel by lowest room id ────────
    //
    // A nearest-entrance walk from the start room was built and measured here
    // (2026-09-07) and rejected — see this function's own doc comment for the
    // numbers. The lowest-id rule it replaced was never worse than a coin flip
    // and stays; these two cases are its own doc comment turned into code.

    /// Two candidate entrances into the same region: the lower room id wins,
    /// full stop — nothing about which one is "nearer" in any other sense
    /// enters into it.
    #[test]
    fn the_lowest_id_entrance_names_the_region() {
        let mut g = MapGraph::new();
        for (id, name) in [(1, "Start"), (5, "Low Id Room"), (9, "High Id Room")] {
            g.upsert_room(id, name.to_string());
        }
        g.add_edge(1, Direction::Down, 9); // reached first, but the higher id
        g.add_edge(1, Direction::Up, 5); // reached second, but the lower id
        let region = mapper::layer::Region { anchor: 5, rooms: [5, 9].into_iter().collect() };
        let named = name_region_by_entry(&g, region);
        assert_eq!(named.anchor, 5, "the lower room id wins between two entrances");
    }

    /// A region with no inbound portal at all falls back to its own lowest room id — the
    /// same tie-break one level up, for a region nothing points into from outside.
    #[test]
    fn a_region_with_no_inbound_portal_falls_back_to_its_own_lowest_id() {
        let mut g = MapGraph::new();
        for (id, name) in [(1, "Start"), (20, "Room A"), (15, "Room B")] {
            g.upsert_room(id, name.to_string());
        }
        // The region is compass-connected internally but has no portal edge in from anywhere.
        g.add_edge(20, Direction::E, 15);
        g.add_edge(15, Direction::W, 20);
        let region = mapper::layer::Region { anchor: 20, rooms: [20, 15].into_iter().collect() };
        let named = name_region_by_entry(&g, region);
        assert_eq!(named.anchor, 15, "no inbound portal at all falls back to the region's own lowest id");
    }

    /// A `Conditional` exit's destination counts as a declared reverse too (not only a plain
    /// one) — Zork I's Studio has a `Code` exit up, and the ONLY thing declaring the way back is
    /// the Kitchen's own CEXIT down to the Studio, a `Conditional` exit that still names a real
    /// destination. The reconciliation reads any exit `ExitDetail::destination()` answers, not
    /// only `Room`, so this must be recovered too.
    #[test]
    fn a_conditional_exits_destination_also_counts_as_a_declared_reverse() {
        let declared_to =
            declared_reverse_lookup([(28u16, Direction::Down, 229u16)].into_iter());
        assert_eq!(routine_destination(&declared_to, 229, Direction::Up), Some(28));
    }

    // ── SQ-1391: a maze's dead ends stay with the maze, whichever way you go ──

    /// A synthetic falsifier for SQ-1391: a three-room maze (all compass-connected, all named
    /// "Maze") with a non-maze-named leaf hanging off ONE room by Down alone — Adventure's own
    /// shape for `#78`, `#43` and `#49` (`#45 D #78`, `#78 U #45`, nothing compass at all).
    /// SQ-1311's compass-only sweep could never see this passage; the leaf must still join the
    /// maze layer, because its only way anywhere is into it.
    #[test]
    fn a_maze_leaf_reached_only_by_down_joins_the_maze_layer() {
        let mut g = MapGraph::new();
        for (id, name) in [(1, "Maze"), (2, "Maze"), (3, "Maze"), (4, "Dead End")] {
            g.upsert_room(id, name.to_string());
        }
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);
        g.add_edge(1, Direction::Down, 4);
        g.add_edge(4, Direction::Up, 1);

        split_layers(&mut g, &MapgenOptions::default());

        let maze_layer = g.layer_of(1);
        assert_ne!(maze_layer, mapper::layer::MAIN_LAYER, "the maze itself gets its own layer");
        assert_eq!(g.layer_of(4), maze_layer, "the Down-only leaf joins it too");
        assert!(g.layer_is_maze(maze_layer));
    }

    /// The falsifier's falsifier: the same leaf, but with a SECOND passage out to a genuine
    /// non-maze room. That second passage is the maze's real exit — the Cyclops Room case
    /// SQ-1311 protected against — and it must keep the leaf off the maze layer even though
    /// every OTHER passage it has still leads into the maze.
    #[test]
    fn a_leaf_with_any_passage_to_a_non_maze_room_stays_off_the_maze_layer() {
        let mut g = MapGraph::new();
        for (id, name) in [(1, "Maze"), (2, "Maze"), (3, "Maze"), (4, "Dead End"), (5, "Clearing")] {
            g.upsert_room(id, name.to_string());
        }
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);
        g.add_edge(1, Direction::Down, 4);
        g.add_edge(4, Direction::Up, 1);
        g.add_edge(4, Direction::N, 5); // the leaf's real exit, to a non-maze room
        g.add_edge(5, Direction::S, 4);

        split_layers(&mut g, &MapgenOptions::default());

        let maze_layer = g.layer_of(1);
        assert_ne!(maze_layer, mapper::layer::MAIN_LAYER);
        assert_eq!(
            g.layer_of(4),
            mapper::layer::MAIN_LAYER,
            "a passage to a non-maze room is the maze's exit, and stays outside it"
        );
    }

    /// A pocket standing between TWO different mazes — Adventure's `#80`, one compass passage
    /// into the "all alike" maze and a bidirectional one into "off At Brink of Pit" — goes to
    /// whichever it has the MOST passages into, not to whichever was walked first: room 2's
    /// single one-way passage loses to room 3's two-connection bidirectional one even though
    /// room 2 (walked first, ascending room id) gets the lower layer id.
    #[test]
    fn a_pocket_between_two_mazes_joins_the_one_with_more_passages() {
        let mut g = MapGraph::new();
        for (id, name) in [(1, "Dead End"), (2, "Maze One"), (3, "Maze Two")] {
            g.upsert_room(id, name.to_string());
        }
        g.add_edge(2, Direction::S, 1); // one-way: Maze One -> the pocket, nothing back
        g.add_edge(1, Direction::E, 3);
        g.add_edge(3, Direction::W, 1); // bidirectional: two connections into Maze Two

        split_layers(&mut g, &MapgenOptions::default());

        let maze_two_layer = g.layer_of(3);
        assert_ne!(maze_two_layer, g.layer_of(2), "the two mazes stay on separate layers");
        assert_eq!(g.layer_of(1), maze_two_layer, "more passages in wins over walked-first");
    }
}
