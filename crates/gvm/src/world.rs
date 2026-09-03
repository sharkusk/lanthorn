//! Declared exits (SQ-1264): what a room's own compiled exit table says for a
//! direction, read from a Glulx object table independently of anything ever
//! walked.
//!
//! Mirrors `zvm::world`'s `door_dir`/`*_to`/`door_to` derivation (SQ-1257) for
//! the SAME Inform 6 library convention — a Glulx game compiled from the
//! Inform 6 library (`inform6lib`, <https://github.com/DavidGriffith/inform6lib>)
//! uses exactly the scheme that module's header documents at length:
//!
//! * **`door_dir`** (`english.h`): each compass-direction object carries a
//!   `door_dir` property whose VALUE is the property number of the matching
//!   `*_to` (`n_obj` declares `door_dir n_to`, and so on). `verblib.h`'s
//!   `GoSub` reads it the same way on both back-ends: `thedir = noun.door_dir;
//!   next_loc = i.thedir;` — a property NUMBER held in a variable, dereferenced
//!   with `.thedir`, Inform 6's "property by number" form, compiled the same
//!   for the Z-machine and for Glulx.
//! * **`door_to`** (`linklpa.h`): when the exit property's value is an object
//!   that also carries `door_dir`, `GoSub` takes one more hop through that
//!   object's `door_to` property.
//!
//! `gvm` takes no dependency on `zvm` — each VM core is independent (see
//! CLAUDE.md's hard rules) — so this module has its own small [`Compass`] and
//! [`DeclaredExit`], shaped identically to `zvm::world`'s so a caller
//! converting between them (`GlulxSession::declared_exit`, which hands back
//! the shared `zvm::world::DeclaredExit` every `Engine::declared_exit` caller
//! already expects) is a plain `match`.
//!
//! # What differs from the Z-machine derivation
//!
//! Everything except HOW an object/property is read. The Z-machine's object
//! numbers are `u16`s bounded by the story's own object count (ZMSD §12.3);
//! Glulx objects are `u32` RAM addresses, read through
//! [`crate::objects::ParseNames`] rather than `zvm::objects`'s byte-oriented
//! accessors. And Glulx property ids are `u16` with **no** 1..=63 ceiling the
//! way the Z-machine's are — Inform 6 assigns them sequentially at compile
//! time with no per-format cap, so there is no spec bound to scan up to.
//! [`MAX_PROP_SCAN`] is therefore a measured, generous headroom rather than an
//! authoritative limit: the highest property id anywhere in `advent.blb`'s
//! whole object table is 276 (measured directly off the compiled image), and
//! `door_dir` itself is a LIBRARY-assigned property (`english.h`, included
//! near the start of a compile), so it lands low in every corpus story tried.

use crate::memory::Memory;
use crate::objects::ParseNames;

/// Highest property id this derivation will scan for `door_dir`/`*_to`/
/// `door_to` — see the module docs for why there is no authoritative bound to
/// use instead.
const MAX_PROP_SCAN: u16 = 1000;

/// The twelve directions a room's exit table may name (SQ-1264) — ordered and
/// named exactly as `zvm::world::Compass`, so [`WorldModel::exit_props`] can be
/// indexed by `dir as usize` directly and a caller translating from
/// `mapper::direction::Direction` (which `gvm` also takes no dependency on)
/// writes the same `match` shape on both engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compass {
    N = 0,
    S = 1,
    E = 2,
    W = 3,
    Ne = 4,
    Nw = 5,
    Se = 6,
    Sw = 7,
    Up = 8,
    Down = 9,
    In = 10,
    Out = 11,
}

impl Compass {
    /// All twelve, in [`WorldModel::exit_props`] index order.
    pub const ALL: [Compass; 12] = [
        Compass::N,
        Compass::S,
        Compass::E,
        Compass::W,
        Compass::Ne,
        Compass::Nw,
        Compass::Se,
        Compass::Sw,
        Compass::Up,
        Compass::Down,
        Compass::In,
        Compass::Out,
    ];

    /// The word the Inform 6 library's compass objects carry for this
    /// direction (`english.h`'s `CompassDirection ->` entries) — what
    /// [`ParseNames::find`] is asked for.
    fn word(self) -> &'static str {
        match self {
            Compass::N => "north",
            Compass::S => "south",
            Compass::E => "east",
            Compass::W => "west",
            Compass::Ne => "northeast",
            Compass::Nw => "northwest",
            Compass::Se => "southeast",
            Compass::Sw => "southwest",
            Compass::Up => "up",
            Compass::Down => "down",
            Compass::In => "in",
            Compass::Out => "out",
        }
    }
}

/// What a room's own exit table declares for one direction (SQ-1264) — shaped
/// identically to `zvm::world::DeclaredExit`; see that type's docs for what
/// each variant does and does not promise. `Message` is carried for the same
/// reason it is there: unreachable today (neither VM can tell a packed/plain
/// STRING address from a ROUTINE address without executing the story), kept
/// distinct so a caller wanting to special-case it later needs no shape change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredExit {
    /// The exit is a fixed room: the property named it directly, or named a
    /// connector object whose own `door_to` names it.
    Room(u32),
    /// The destination is computed at run time (a routine), or is a printed
    /// string — see the `Message` note above for why the two collapse here.
    Code,
    /// Reserved; see the variant-level note above. Currently unreachable.
    Message,
    /// The compass WAS identified for this story, and this room's `*_to`
    /// property for this direction is simply absent — the room declares
    /// NOTHING here, as opposed to declaring code this derivation merely
    /// cannot resolve ([`Self::Code`]).
    Absent,
    /// No exit is declared this way at all: `origin` is not one of this
    /// story's objects, or this story's `door_dir` convention could not be
    /// identified (a non-Inform-library Glulx story, or one whose parser-name
    /// property this reader cannot locate at all).
    Unknown,
}

/// Conventions recovered from one story's Glulx object table (SQ-1264) — the
/// `door_dir`/`*_to`/`door_to` property numbers, exactly as
/// `zvm::world::WorldModel` recovers them for the Z-machine. Cheap to hold;
/// built once per story and read on every [`Self::declared_exit`] ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorldModel {
    /// The twelve `*_to` exit-property numbers, indexed by [`Compass`].
    /// `None` at every index when the `door_dir` convention could not be
    /// identified, which is every non-Inform-library Glulx story.
    exit_props: [Option<u16>; 12],
    /// The `door_dir` property number itself — what tells
    /// [`Self::declared_exit`] whether an exit's target object is a room or a
    /// connector to follow through [`Self::door_to_prop`].
    door_dir_prop: Option<u16>,
    /// The `door_to` property number, when a connector object could be found
    /// and cross-checked. `None` is the ordinary case for a story with no
    /// connectors, or one where the connectors found disagreed too much to
    /// trust a single property number.
    door_to_prop: Option<u16>,
}

impl WorldModel {
    /// Recover the model from the story's object table, through `names`
    /// (already detected once per story — see `GlulxSession::parse_names`).
    pub fn discover(mem: &Memory, names: &ParseNames) -> WorldModel {
        let (exit_props, door_dir_prop, door_to_prop) = infer_exits(mem, names);
        WorldModel { exit_props, door_dir_prop, door_to_prop }
    }

    /// What room `origin`'s own map data declares for `dir` (SQ-1264),
    /// resolving through a connector object when the property points at one —
    /// see `zvm::world::WorldModel::declared_exit`, which this mirrors.
    pub fn declared_exit(
        &self,
        mem: &Memory,
        names: &ParseNames,
        origin: u32,
        dir: Compass,
    ) -> DeclaredExit {
        let Some(prop) = self.exit_props[dir as usize] else { return DeclaredExit::Unknown };
        if !names.is_object(mem, origin) {
            return DeclaredExit::Unknown;
        }
        let raw = names.property_word(mem, origin, prop).unwrap_or(0);
        if raw == 0 {
            return DeclaredExit::Absent;
        }
        self.resolve(mem, names, origin, raw)
    }

    /// One step of exit resolution: classify a raw, NONZERO `*_to` (or
    /// `door_to`) value already read off some object's property table. See
    /// `zvm::world::WorldModel::resolve`, which this mirrors.
    fn resolve(&self, mem: &Memory, names: &ParseNames, holder: u32, raw: u32) -> DeclaredExit {
        if !names.is_object(mem, raw) {
            // A routine or string address — neither VM back-end can tell
            // those apart without executing the story (see the module docs).
            return DeclaredExit::Code;
        }
        let Some(dd) = self.door_dir_prop else { return DeclaredExit::Room(raw) };
        if names.property(mem, raw, dd).is_none() || raw == holder {
            return DeclaredExit::Room(raw);
        }
        // `raw` is a connector. Follow `door_to`, one hop only.
        let Some(door_to) = self.door_to_prop else { return DeclaredExit::Code };
        let k = names.property_word(mem, raw, door_to).unwrap_or(0);
        if k == 0 || !names.is_object(mem, k) {
            return DeclaredExit::Code;
        }
        DeclaredExit::Room(k)
    }
}

/// Derive the `*_to` property numbers, the `door_dir` property number itself,
/// and (best-effort) the `door_to` property number, all from the compiled
/// object table. `None` in every slot for a story with no `door_dir`
/// convention to find. See `zvm::world::infer_exits`, which this mirrors.
fn infer_exits(mem: &Memory, names: &ParseNames) -> ([Option<u16>; 12], Option<u16>, Option<u16>) {
    let none = ([None; 12], None, None);

    // Only the eight cardinal/intercardinal words are trusted to IDENTIFY the
    // compass objects — see `zvm::world::infer_exits`'s doc comment for why
    // Up/Down/In/Out cannot be trusted the same way (common words that a
    // dictionary lookup can and does resolve to something else entirely).
    const PRIMARY: [Compass; 8] =
        [Compass::N, Compass::S, Compass::E, Compass::W, Compass::Ne, Compass::Nw, Compass::Se, Compass::Sw];

    let mut primary_ids: [Option<u32>; 12] = [None; 12];
    for dir in PRIMARY {
        if let Some(o) = names.find(mem, dir.word()) {
            primary_ids[dir as usize] = Some(o.id);
        }
    }
    let found: Vec<(Compass, u32)> = PRIMARY
        .into_iter()
        .filter_map(|d| primary_ids[d as usize].map(|id| (d, id)))
        .collect();
    // Need at least six of the eight to trust a shared property as `door_dir`
    // — same confidence bar as the Z-machine derivation.
    if found.len() < 6 {
        return none;
    }

    // `door_dir`: the property every found compass object carries whose
    // values, across them, are distinct small numbers (each direction's own
    // `*_to`).
    let mut door_dir_prop = None;
    'search: for prop in 1u16..=MAX_PROP_SCAN {
        let mut vals: Vec<u32> = Vec::with_capacity(found.len());
        for &(_, id) in &found {
            let Some(v) = names.property_word(mem, id, prop) else { continue 'search };
            vals.push(v);
        }
        let mut sorted = vals.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() == vals.len() && vals.iter().all(|&v| v > 0 && v <= MAX_PROP_SCAN as u32) {
            door_dir_prop = Some(prop);
            break;
        }
    }
    let Some(door_dir_prop) = door_dir_prop else { return none };

    // `exit_props[dir]` is exactly the door_dir VALUE the matching compass
    // object carries.
    let mut exit_props = [None; 12];
    let mut used_props: Vec<u16> = Vec::with_capacity(12);
    for &(dir, id) in &found {
        if let Some(p) = names.property_word(mem, id, door_dir_prop) {
            if p > 0 && p <= MAX_PROP_SCAN as u32 {
                exit_props[dir as usize] = Some(p as u16);
                used_props.push(p as u16);
            }
        }
    }
    // Up/Down/In/Out: accepted only when the object `ParseNames::find` turns
    // up for the word ALSO carries `door_dir` with a value that is a
    // plausible, still-unused `*_to` property number.
    for dir in [Compass::Up, Compass::Down, Compass::In, Compass::Out] {
        let Some(o) = names.find(mem, dir.word()) else { continue };
        let id = o.id;
        let Some(p) = names.property_word(mem, id, door_dir_prop) else { continue };
        if p > 0 && p <= MAX_PROP_SCAN as u32 && !used_props.contains(&(p as u16)) {
            primary_ids[dir as usize] = Some(id);
            exit_props[dir as usize] = Some(p as u16);
            used_props.push(p as u16);
        }
    }
    let compass_ids = primary_ids;

    // `door_to`: found by cross-checking real CONNECTOR objects — anything
    // (other than a compass object) that also carries `door_dir`.
    const MAX_CONNECTORS: usize = 24;
    let compass_id_set: Vec<u32> = compass_ids.iter().filter_map(|&o| o).collect();
    let mut connectors: Vec<u32> = Vec::new();
    for addr in names.objects() {
        if compass_id_set.contains(&addr) {
            continue;
        }
        if names.property(mem, addr, door_dir_prop).is_some() {
            connectors.push(addr);
            if connectors.len() >= MAX_CONNECTORS {
                break;
            }
        }
    }
    let door_to_prop = infer_door_to(mem, names, &connectors, &compass_id_set, door_dir_prop);

    (exit_props, Some(door_dir_prop), door_to_prop)
}

/// The `door_to` property number: the one present on most sampled CONNECTORS
/// whose value, where it names a room at all, is a plausible and DISTINCT
/// TERMINAL — not another connector. See `zvm::world::infer_door_to`, which
/// this mirrors exactly (translated to `u32` addresses and `u16` property ids).
fn infer_door_to(
    mem: &Memory,
    names: &ParseNames,
    connectors: &[u32],
    compass_ids: &[u32],
    door_dir_prop: u16,
) -> Option<u16> {
    if connectors.len() < 2 {
        return None;
    }
    let min_present = (connectors.len() / 2).max(2);
    'search: for prop in 1u16..=MAX_PROP_SCAN {
        if prop == door_dir_prop {
            continue;
        }
        let mut present = 0usize;
        let mut room_like: Vec<u32> = Vec::new();
        for &c in connectors {
            let Some(v) = names.property_word(mem, c, prop) else { continue }; // absent on this one connector
            present += 1;
            if v == 0 || !names.is_object(mem, v) {
                continue; // no exit, or a routine/string-valued door — not disqualifying
            }
            if v == c || compass_ids.contains(&v) || names.property(mem, v, door_dir_prop).is_some() {
                continue 'search; // looks wrong in a way `door_to` never should
            }
            room_like.push(v);
        }
        if present < min_present {
            continue;
        }
        let mut distinct = room_like.clone();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() >= 2 {
            return Some(prop);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── A synthetic Glulx object table, laid out by the spec ────────────────
    //
    // Mirrors `crate::objects`'s own test builder (see its header for why
    // every offset comes from "The Glulx Inform Technical Reference" rather
    // than from this reader's own arithmetic), extended to let an object carry
    // ARBITRARY extra properties beyond the `name` array — `door_dir`/`*_to`/
    // `door_to` are exactly that, and are what this module's derivation reads.
    //
    // Every base address is chosen comfortably above [`MAX_PROP_SCAN`] so a
    // dictionary-word address stored in a compass object's `name` array (property
    // 1) can never be mistaken for a plausible `door_dir`/`*_to` VALUE — both are
    // validated against the same `<= MAX_PROP_SCAN` ceiling the real derivation
    // uses, so an address that happened to land under 1000 would open exactly
    // that false-positive door in this fixture, which no real story's address
    // space is small enough to hit.

    const RAM: u32 = 0x4000;
    const EXT: u32 = 0x9000;
    const NAB: u32 = 7;
    const STRIDE: u32 = 1 + NAB + 24;
    const OBJ: u32 = 0x5000;
    const PROPS: u32 = 0x6000;
    const NAMEDATA: u32 = 0x7000;
    const PROP_STRIDE: u32 = 64;
    const DICT: u32 = RAM + 0x18;
    const DICT_STRIDE: u32 = 16;

    const COMPASS_WORDS: [&str; 8] =
        ["north", "south", "east", "west", "northeast", "northwest", "southeast", "southwest"];
    /// Padding so the dictionary clears `MIN_DICT_WORDS` (16) — unused by any
    /// object, present only to satisfy `crate::grammar::locate`'s confidence
    /// floor, exactly as `crate::objects`'s own test builder pads its 9-word
    /// story vocabulary out to twenty.
    const FILLER_WORDS: [&str; 8] = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel"];

    /// One object's plan: the words its `name` property (1) answers to (empty
    /// for a room, which no parser command ever names directly in these
    /// tests), and whatever other properties it carries as `(id, value)`.
    struct ObjPlan {
        words: Vec<&'static str>,
        extra: Vec<(u16, u32)>,
    }

    /// The object address at list index `i` — deterministic, so a plan can
    /// reference another object's address before the image is built.
    fn addr_of(i: usize) -> u32 {
        OBJ + i as u32 * STRIDE
    }

    struct Builder {
        buf: Vec<u8>,
        words: Vec<&'static str>,
    }

    impl Builder {
        fn new() -> Builder {
            let mut words: Vec<&'static str> = COMPASS_WORDS.to_vec();
            words.extend_from_slice(&FILLER_WORDS);
            let mut b = Builder { buf: vec![0u8; EXT as usize], words };
            b.buf[0..4].copy_from_slice(b"Glul");
            b.w32(0x04, 0x0003_0102);
            b.w32(0x08, RAM);
            b.w32(0x0C, EXT);
            b.w32(0x10, EXT);
            b.w32(0x14, 0x1000);
            b.w32(0x18, RAM + 0x10); // start function; never executed here
            b.tables();
            b
        }

        fn w16(&mut self, at: u32, v: u16) {
            self.buf[at as usize..at as usize + 2].copy_from_slice(&v.to_be_bytes());
        }

        fn w32(&mut self, at: u32, v: u32) {
            self.buf[at as usize..at as usize + 4].copy_from_slice(&v.to_be_bytes());
        }

        /// Grammar, actions and dictionary, contiguous and in that order — the
        /// minimum [`crate::grammar::locate`] accepts, so the object tree here
        /// is found the way it is found in a real story (see `crate::objects`'s
        /// own test builder, which this mirrors byte for byte, shifted onto
        /// this fixture's own base addresses).
        fn tables(&mut self) {
            self.w32(RAM, 1); // one verb
            self.w32(RAM + 4, RAM + 8); // its line block
            self.buf[(RAM + 8) as usize] = 1; // one line
            self.buf[(RAM + 0xC) as usize] = 15; // ENDIT, ending the line's tokens
            self.w32(RAM + 0xD, 1); // one action…
            self.w32(RAM + 0x11, 100); // …whose routine is in ROM
            self.w32(DICT, self.words.len() as u32);
            let words = self.words.clone();
            for (i, w) in words.iter().enumerate() {
                let e = self.dict_addr(i);
                self.buf[e as usize] = 0x60; // the dictionary-record tag
                for (j, c) in w.chars().enumerate() {
                    self.buf[(e + 1 + j as u32) as usize] = c as u32 as u8;
                }
                self.w16(e + 10, 0); // flags
                self.w16(e + 12, 0); // verb field
            }
        }

        fn dict_addr(&self, i: usize) -> u32 {
            DICT + 4 + i as u32 * DICT_STRIDE
        }

        fn word_addr(&self, w: &str) -> u32 {
            DICT + 4 + self.words.iter().position(|x| *x == w).expect("a word of this dictionary") as u32 * DICT_STRIDE
        }

        /// Write the object list. §2's six longs are written by NAME, in the
        /// order the reference lists them, and nothing here consults
        /// `crate::objects::Field`. Each object's property table holds its
        /// `name` array (property 1, when `words` is non-empty) followed by
        /// `extra`'s entries, written in ASCENDING id order — §3's own
        /// ordering guarantee, and what [`ParseNames::property`]'s early-stop
        /// depends on.
        fn objects(&mut self, plan: &[ObjPlan]) {
            for (i, o) in plan.iter().enumerate() {
                let at = addr_of(i);
                let base = at + 1 + NAB; // past the tag and the attribute bytes
                self.buf[at as usize] = 0x70;
                let next = if i + 1 < plan.len() { addr_of(i + 1) } else { 0 };
                self.w32(base, next); // long 0: next object
                self.w32(base + 4, 0); // long 1: hardware name string (unused here)
                let table = PROPS + i as u32 * PROP_STRIDE;
                self.w32(base + 8, table); // long 2: property table address
                self.w32(base + 12, 0); // long 3: parent
                self.w32(base + 16, 0); // long 4: sibling
                self.w32(base + 20, 0); // long 5: child

                let mut entries: Vec<(u16, u32)> = Vec::new();
                if !o.words.is_empty() {
                    entries.push((1u16, u32::MAX)); // placeholder; name array is multi-word
                }
                entries.extend(o.extra.iter().copied());
                entries.sort_unstable_by_key(|(id, _)| *id);

                self.w32(table, entries.len() as u32);
                let data_base = NAMEDATA + i as u32 * PROP_STRIDE;
                let mut data_at = data_base;
                for (k, &(id, val)) in entries.iter().enumerate() {
                    let entry = table + 4 + k as u32 * 10;
                    self.w16(entry, id);
                    if id == 1u16 && !o.words.is_empty() {
                        self.w16(entry + 2, o.words.len() as u16);
                        self.w32(entry + 4, data_at);
                        self.w16(entry + 8, 0);
                        for (j, w) in o.words.iter().enumerate() {
                            self.w32(data_at + j as u32 * 4, self.word_addr(w));
                        }
                        data_at += o.words.len() as u32 * 4;
                    } else {
                        self.w16(entry + 2, 1);
                        self.w32(entry + 4, data_at);
                        self.w16(entry + 8, 0);
                        self.w32(data_at, val);
                        data_at += 4;
                    }
                }
            }
        }

        fn mem(self) -> Memory {
            Memory::new(self.buf).expect("synthetic image is valid")
        }
    }

    fn build(plan: Vec<ObjPlan>) -> Memory {
        let mut b = Builder::new();
        b.objects(&plan);
        b.mem()
    }

    fn compass_plan(word: &'static str, door_dir_val: u32) -> ObjPlan {
        ObjPlan { words: vec![word], extra: vec![(DOOR_DIR, door_dir_val)] }
    }

    fn room_plan(extra: Vec<(u16, u32)>) -> ObjPlan {
        ObjPlan { words: Vec::new(), extra }
    }

    /// The property numbers this fixture uses — arbitrary but fixed, exactly
    /// as a real compile's are arbitrary but fixed for the life of that story.
    const DOOR_DIR: u16 = 20;
    const N_TO: u16 = 6;
    const S_TO: u16 = 7;
    const E_TO: u16 = 8;
    const W_TO: u16 = 9;
    const DOOR_TO: u16 = 40;

    /// Object indices, in list order (0..=7 are the eight compass objects,
    /// referenced only positionally below — the rooms and connectors are
    /// referenced by name).
    const VALLEY: usize = 8;
    const FOREST1: usize = 9;
    const FOREST2: usize = 10;
    const HILL: usize = 11;
    const CONNECTOR1: usize = 12;

    struct RoomIds {
        valley: u32,
        forest1: u32,
        forest2: u32,
        hill: u32,
    }

    struct BareIds {
        room: u32,
    }

    /// Builds the fixture every `Room(_)`/`Code`/`door_to` test below shares:
    /// eight compass objects (`door_dir` = property 20, values 6/7/8/9/10/12/11/13
    /// — the exact shape measured off `advent.blb` itself, see `crates/gvm/src/world.rs`'s
    /// module docs), a valley whose E and W exits BOTH name forest 1 (mirroring
    /// `In A Valley`'s `e_to`/`w_to` in `advent.inf`), a forest 1 whose south exit
    /// is a non-object value (standing in for a routine address), a forest 2
    /// whose north exit is a CONNECTOR object resolved through `door_to`, and
    /// the hill that connector leads to.
    fn build_advent_shaped_story() -> (Memory, RoomIds) {
        let plan = vec![
            compass_plan("north", u32::from(N_TO)),
            compass_plan("south", u32::from(S_TO)),
            compass_plan("east", u32::from(E_TO)),
            compass_plan("west", u32::from(W_TO)),
            compass_plan("northeast", 10),
            compass_plan("northwest", 12),
            compass_plan("southeast", 11),
            compass_plan("southwest", 13),
            room_plan(vec![(E_TO, addr_of(FOREST1)), (W_TO, addr_of(FOREST1))]), // VALLEY
            room_plan(vec![(S_TO, 1)]),                                         // FOREST1: not an object
            room_plan(vec![(N_TO, addr_of(CONNECTOR1))]),                       // FOREST2
            room_plan(vec![]),                                                  // HILL
            room_plan(vec![(DOOR_DIR, 999), (DOOR_TO, addr_of(HILL))]),         // CONNECTOR1
            room_plan(vec![(DOOR_DIR, 999), (DOOR_TO, addr_of(VALLEY))]),       // CONNECTOR2
        ];
        assert_eq!(plan.len(), 14, "object indices below assume this exact layout");
        let ids = RoomIds {
            valley: addr_of(VALLEY),
            forest1: addr_of(FOREST1),
            forest2: addr_of(FOREST2),
            hill: addr_of(HILL),
        };
        (build(plan), ids)
    }

    /// A story with six ordinarily-named objects and a room, but NONE of the
    /// eight compass words — the shape a non-Inform-library Glulx story (or one
    /// this reader's `ParseNames` cannot see into) presents.
    fn build_bare_story() -> (Memory, BareIds) {
        let plan = vec![
            ObjPlan { words: vec!["alpha"], extra: vec![] },
            ObjPlan { words: vec!["bravo"], extra: vec![] },
            ObjPlan { words: vec!["charlie"], extra: vec![] },
            ObjPlan { words: vec!["delta"], extra: vec![] },
            ObjPlan { words: vec!["echo"], extra: vec![] },
            ObjPlan { words: vec!["foxtrot"], extra: vec![] },
            room_plan(vec![]), // ROOM
        ];
        let room = addr_of(plan.len() - 1);
        (build(plan), BareIds { room })
    }

    /// A synthetic Inform 6 story shaped like a small Advent-style area: eight
    /// compass objects with a `door_dir` property (20) whose values are the
    /// per-direction `*_to` property numbers, and three rooms — two of them
    /// (`FOREST_1`/`FOREST_2`) standing in for the two forests, one
    /// (`VALLEY`) whose east AND west exits both name `FOREST_1` — mirroring
    /// exactly the shape `advent.inf` compiles (see the SQ-1264 report: both
    /// `In A Valley`'s `e_to`/`w_to` name the SAME room, and the randomness is
    /// a redirect the destination performs on arrival, not a routine in the
    /// exit property itself — so this fixture's job is only to prove the
    /// DECLARED-EXIT reader recovers a plain `Room(_)` for both, not to model
    /// the redirect).
    #[test]
    fn discovers_the_door_dir_convention_and_reads_plain_room_exits() {
        let (mem, ids) = build_advent_shaped_story();
        let names = ParseNames::detect(&mem).expect("synthetic object tree");
        let model = WorldModel::discover(&mem, &names);

        assert_eq!(
            model.declared_exit(&mem, &names, ids.valley, Compass::E),
            DeclaredExit::Room(ids.forest1),
            "east out of the valley is a plain declared exit to forest 1"
        );
        assert_eq!(
            model.declared_exit(&mem, &names, ids.valley, Compass::W),
            DeclaredExit::Room(ids.forest1),
            "west out of the valley ALSO names forest 1 — both exits share one destination"
        );
        assert_eq!(
            model.declared_exit(&mem, &names, ids.valley, Compass::N),
            DeclaredExit::Absent,
            "the valley declares nothing north"
        );
    }

    /// A story with no compass objects at all (a non-Inform-library Glulx
    /// game, or one this reader's `ParseNames` cannot see into) must answer
    /// `Unknown` for every direction rather than mistake some coincidentally
    /// shaped property for `door_dir`.
    #[test]
    fn a_story_with_no_compass_objects_has_no_door_dir_convention_to_find() {
        let (mem, ids) = build_bare_story();
        let names = ParseNames::detect(&mem).expect("synthetic object tree");
        let model = WorldModel::discover(&mem, &names);
        for dir in Compass::ALL {
            assert_eq!(model.declared_exit(&mem, &names, ids.room, dir), DeclaredExit::Unknown);
        }
    }

    /// A `*_to` value that is not one of this story's objects at all (the
    /// synthetic stand-in for a routine or string address — Glulx addresses
    /// are absolute, so any address outside the object list is neither) must
    /// read `Code`, never a guessed room.
    #[test]
    fn a_non_object_exit_value_reads_as_code() {
        let (mem, ids) = build_advent_shaped_story();
        let names = ParseNames::detect(&mem).expect("synthetic object tree");
        let model = WorldModel::discover(&mem, &names);
        assert_eq!(
            model.declared_exit(&mem, &names, ids.forest1, Compass::S),
            DeclaredExit::Code,
            "forest 1's south exit is a routine address in the fixture, not an object"
        );
    }

    /// A connector object (carries `door_dir` itself) is followed one `door_to`
    /// hop to its real destination, exactly as `zvm::world` does.
    #[test]
    fn a_connector_object_is_followed_through_door_to() {
        let (mem, ids) = build_advent_shaped_story();
        let names = ParseNames::detect(&mem).expect("synthetic object tree");
        let model = WorldModel::discover(&mem, &names);
        assert_eq!(
            model.declared_exit(&mem, &names, ids.forest2, Compass::N),
            DeclaredExit::Room(ids.hill),
            "forest 2's north exit is a door object whose door_to names the hill"
        );
    }
}
