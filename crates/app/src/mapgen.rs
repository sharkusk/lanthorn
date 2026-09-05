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

use std::collections::{BTreeMap, BTreeSet};
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
/// The precedence is `Random` > `Conditional` > `Door` > `OneWay` > `Declared`,
/// so an edge is never labelled twice and a consumer can switch on one value.
/// `reciprocal` is reported alongside and independently, because a door or a
/// conditional can be one-way too and the kind has room for only one fact.
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

/// Generate the static map for the story at `path`.
///
/// `layout` runs the mapper's own tidy pass ([`mapper::layout::relayout_auto`])
/// over the finished graph, which is what gives every room a position; without
/// it the graph is pure topology and every `pos` is `None`.
///
/// The story is mounted through [`crate::hints::load_mounted_story`] — the same
/// call `startup.rs` boots from — so a Blorb, a zip and a disk image all reach
/// here as bare executable bytes, classified by engine.
pub fn generate(path: &Path, layout: bool) -> Result<GeneratedMap, GenError> {
    let (loaded, _medium) = crate::hints::load_mounted_story(path).map_err(GenError::Load)?;
    let file = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();

    let mut map = match &loaded {
        LoadedStory::ZCode(bytes) => zmachine_map(bytes, file)?,
        LoadedStory::Glulx(bytes) => glulx_map(bytes, file)?,
        LoadedStory::Scott(bytes) => scott_map(bytes, file)?,
    };

    if layout {
        let t = Instant::now();
        mapper::layout::relayout_auto(&mut map.graph);
        map.layout_time = Some(t.elapsed());
    }
    annotate_rooms(&mut map);
    Ok(map)
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
        graph.add_edge(e.origin, e.dir, e.dest);
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

    GeneratedMap { graph, source, story, facts, engine_refs, layout_time: None }
}

/// Write each room's door and conditional exits into its `notes`, so the text
/// dump says them.
///
/// [`crate::map_dump::render_dump`] prints `notes=` from the graph, and
/// [`mapper::graph::Connection`] has nowhere to put a per-edge annotation — so
/// rather than change the persisted graph format for a fact only this generator
/// ever produces, the facts are summarised onto the ORIGIN room in the same
/// `key=[…]` style as the dump's own `random=` and `dropped=` notes.
fn annotate_rooms(map: &mut GeneratedMap) {
    let mut by_room: BTreeMap<RoomId, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for f in &map.facts {
        let entry = by_room.entry(f.origin).or_default();
        let dir = mapper::direction::short_label(f.dir).to_uppercase();
        match f.kind {
            EdgeKind::Door => {
                let via = f.via.as_deref().unwrap_or("door");
                entry.0.push(format!("{dir}→{via:?}"));
            }
            EdgeKind::Conditional => {
                let note = f.note.as_deref().unwrap_or("condition unknown");
                entry.1.push(format!("{dir}→({note})"));
            }
            _ => {}
        }
    }
    for (id, (doors, conds)) in by_room {
        let mut parts = Vec::new();
        if !doors.is_empty() {
            parts.push(format!("door=[{}]", doors.join(", ")));
        }
        if !conds.is_empty() {
            parts.push(format!("conditional=[{}]", conds.join(", ")));
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
            name: zvm::objects::short_name(&mem, obj),
            engine_ref: EngineRef::ZObject(obj),
        })
        .collect();

    let mut edges = Vec::new();
    for (&obj, declared) in &declares {
        for &(dir, detail) in declared {
            let (dest, kind, via, note) = match detail {
                zvm::world::ExitDetail::Room(d) => (d, EdgeKind::Declared, None, None),
                zvm::world::ExitDetail::Door { dest, door } => (
                    dest,
                    EdgeKind::Door,
                    Some(zvm::objects::short_name(&mem, door)),
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
                // A routine or a refusal message: real map data, but not a
                // passage anything static can draw. Deliberately no edge — a
                // guessed one would be worse than a missing one.
                zvm::world::ExitDetail::Code
                | zvm::world::ExitDetail::Message
                | zvm::world::ExitDetail::Absent
                | zvm::world::ExitDetail::Unknown => continue,
            };
            edges.push(RawEdge { origin: obj as RoomId, dir, dest: dest as RoomId, kind, via, note });
        }
    }

    let story = StoryIdent {
        file,
        engine: "z-machine",
        // ZMSD §11.1: release at $02 (word), serial at $12..$18 (six ASCII
        // digits), checksum at $1C (word). Read here rather than through
        // `header::parse_header`, which does not carry them.
        release: (bytes.len() > 0x03).then(|| u16::from_be_bytes([bytes[0x02], bytes[0x03]])),
        serial: (bytes.len() >= 0x18)
            .then(|| String::from_utf8_lossy(&bytes[0x12..0x18]).into_owned()),
        checksum: (bytes.len() > 0x1D)
            .then(|| format!("0x{:04x}", u16::from_be_bytes([bytes[0x1C], bytes[0x1D]]))),
    };
    Ok(assemble(rooms, edges, source, story))
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

    // Glulx spec §1.4: the header's own whole-image checksum, offset 0x20 —
    // every Glulx image has one, unlike release/serial below.
    let checksum = Some(format!("0x{:08x}", mem.checksum()));

    // The release and serial are not part of the Glulx spec itself; they are
    // the Inform compiler's own `Info` block, read only when its magic at
    // 0x24 confirms the image actually carries one (Glulx-Inform-Tech.html
    // §1 "Static Data") — a bare non-Inform Glulx image has neither.
    let (release, serial) = match gvm::header::parse_inform_info(bytes) {
        Some(info) => (Some(info.release), Some(info.serial)),
        None => (None, None),
    };

    let story = StoryIdent { file, engine: "glulx", release, serial, checksum };

    // Inform 7's own map table first — it is the higher authority for any story
    // that has one, since it is what the I7 runtime itself reads.
    if let Some(w) = gvm::i7map::I7World::detect(&mem, &names) {
        return Ok(i7_map(&mem, &names, &w, story));
    }
    i6_glulx_map(&mem, &names, story)
}

fn i7_map(
    mem: &gvm::memory::Memory,
    names: &gvm::objects::ParseNames,
    w: &gvm::i7map::I7World,
    story: StoryIdent,
) -> GeneratedMap {
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
    for obj in names.objects() {
        let mut here = Vec::new();
        for dir in DIRS {
            let Some(c) = g_compass(dir) else { continue };
            match wm.declared_exit(mem, names, obj, c) {
                gvm::world::DeclaredExit::Room(d) => {
                    destinations.insert(d);
                    here.push((dir, d));
                }
                gvm::world::DeclaredExit::Code | gvm::world::DeclaredExit::Message => {
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
            name: names.short_name(mem, addr).unwrap_or_default(),
            engine_ref: EngineRef::GlulxAddr(addr),
        })
        .collect();

    let edges: Vec<RawEdge> = declares
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
        std::fs::write(&p, crate::map_dump::render_dump(&map.graph, &crate::symbols::SymbolSet::default()))?;
        written.push(p);
    }
    if what.svg {
        let p = out_dir.join(format!("{stem}.svg"));
        let rm = mapper::render::render(&map.graph);
        std::fs::write(&p, crate::export_svg::render_svg(&rm))?;
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
/// format asks consumers to ignore what they do not recognise.
pub const JSON_VERSION: u32 = 1;

#[derive(serde::Serialize)]
struct JsonMap<'a> {
    format: &'static str,
    version: u32,
    generator: JsonGenerator,
    story: JsonStory<'a>,
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

/// Render `map` as the versioned, self-describing JSON map.
///
/// The schema is documented in `docs/internals/mapping.md`. Nothing
/// lanthorn-internal goes in: no seam decisions, no render slots, no terminal
/// cells — only what a tool that has never seen lanthorn could use.
pub fn render_json(map: &GeneratedMap) -> String {
    let graph = &map.graph;

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
            let engine_ref = match map.engine_refs.get(&r.id) {
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

    let edges: Vec<JsonEdge> = map
        .facts
        .iter()
        .map(|f| {
            let reciprocal = map
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
        generator: JsonGenerator { name: "lanthorn-mapgen", version: buildinfo::LONG },
        story: JsonStory {
            file: &map.story.file,
            engine: map.story.engine,
            source: map.source.as_str(),
            release: map.story.release,
            serial: map.story.serial.as_deref(),
            checksum: map.story.checksum.as_deref(),
            generated_at: rfc3339_now(),
        },
        directions,
        rooms,
        edges,
        layers,
    };

    // `to_string_pretty` because these files are read, diffed and checked in.
    serde_json::to_string_pretty(&doc).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}")) + "\n"
}
