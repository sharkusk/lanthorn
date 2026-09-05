//! The Inform **7** world model, read straight off a Glulx image (SQ-1303):
//! which objects are rooms, what each room is CALLED, and which room each
//! direction leads to — none of it played for.
//!
//! ── Why [`crate::world`] cannot answer this ──────────────────────────────────
//!
//! [`crate::world`] reads the Inform **6** library's exit convention:
//! `door_dir` on each compass object naming a `*_to` property, and that
//! property held on the room. Inform 7 does not compile that. It stores the
//! whole map in ONE array and gives each room and each direction an index into
//! it, which is a completely different shape and needs a completely different
//! derivation.
//!
//! ── Where the format is specified ────────────────────────────────────────────
//!
//! The authority is Inform 7's own runtime template, consulted directly rather
//! than from memory. Two files, both in <https://github.com/ganelson/inform>:
//!
//! * **`retrospective/6M62/Internal/I6T/WorldModel.i6t`**, section "Map
//!   Connections" (also `retrospective/6L38/…`, and, spelled identically,
//!   `inform7/Internal/…/WorldModelKit/Sections/WorldModel.i6t` for 10.x):
//!
//!   ```text
//!   [ MapConnection from_room dir  in_direction through_door;
//!       if ((from_room ofclass K1_room) && (dir ofclass K3_direction)) {
//!           in_direction = Map_Storage-->
//!               ((from_room.IK1_Count)*No_Directions + dir.IK3_Count);
//!   ```
//!
//!   So the map is a single word array, `Map_Storage`, indexed
//!   `room_index * No_Directions + direction_index`, where the two indices are
//!   *properties* the compiler puts on every room and every direction.
//!
//! * **`inform7/runtime-module/Chapter 6/The Map.w`** (`RTMap::compile_model_tables`)
//!   says what is emitted and in what order — one row per room, in instance
//!   order, `No_Directions` entries per row, each entry either an instance
//!   name or a literal `0`:
//!
//!   > The `Map_Storage` array consists only of the `exits` arrays written out
//!   > one after another. It looks wasteful of memory, since it is almost always
//!   > going to be filled mostly with `0` entries (meaning: no exit that way).
//!   > But the memory needs to be there because map connections can be added
//!   > dynamically at run-time, so we can't know now how many we will need.
//!
//!   That last sentence is the reason the array is in **RAM**: `AssertMapConnection`
//!   in the same template writes to it. What this module reads is therefore the
//!   map as the story was COMPILED, which is the map as the story STARTS. A story
//!   that rewrites its own map ("change the north exit of the Hall to the Cellar")
//!   diverges from it, and nothing in the image says so.
//!
//! ── Nothing names any of it, so all four facts are derived ───────────────────
//!
//! A Glulx image records no table addresses and no symbols — see
//! [`crate::objects`]'s header for the long version. `Map_Storage`,
//! `No_Directions`, `IK1_Count` and `IK3_Count` are all *compiler-assigned* and
//! all invisible, so each is recovered from a signature:
//!
//! * **`IK3_Count`, the direction index** — an instance-count property is a
//!   property whose values across the objects that carry it are exactly
//!   `0..n-1`, each once (a bijection); a story has several, one per kind that
//!   needs one. The DIRECTION one is the bijection property carried by the
//!   objects the dictionary calls `north`, `south`, `east`, `west`… A story
//!   with author-defined directions simply has a larger `n` (Counterfeit
//!   Monkey has 20 where the Standard Rules define 12), which is why the count
//!   is read rather than assumed.
//! * **`IK1_Count`, the room index** — every other bijection property is a
//!   candidate, and the one that is right is the one that makes an array work.
//! * **`Map_Storage`** — a window of `rooms * directions` words in RAM in which
//!   every entry is `0` or one of this story's objects, scored by RECIPROCITY:
//!   an I7 map connection is two-way by default, so in the true window the
//!   room named at `(r, d)` almost always names room `r` back somewhere in its
//!   own row. Measured on Counterfeit Monkey, the true base scores 180 of 181
//!   room entries reciprocal and the next offset along scores 133 — the peak is
//!   not close.
//! * **`printed name`** — the property whose text most often contains one of
//!   the object's own dictionary words ("New Church" against `church`). See
//!   [`I7World::printed_name`] for how a text value is shaped.
//!
//! **Arrays here are not word-aligned.** `Map_Storage` sits at `0x378f28` in
//! `CounterfeitMonkey-11.gblorb` and at an address ≡ 1 (mod 4) in
//! `The_Wizard_Sniffer.gblorb`; Glulx imposes no alignment and Inform packs its
//! arrays, so every scan below runs at all four byte phases. A scan that steps
//! four bytes from RAMSTART finds the first story's map and is *blind* to the
//! second's.

use std::collections::{HashMap, HashSet};

use crate::memory::Memory;
use crate::objects::ParseNames;
use crate::world::Compass;

/// A room count above this is not believed — it would make the window scan
/// quadratic on a story that has no map at all. The largest room count in the
/// corpus measured here is 100 (`CounterfeitMonkey-11.gblorb`).
const MAX_ROOMS: usize = 4096;

/// How many directions a story may have. The Standard Rules define twelve;
/// Counterfeit Monkey has twenty. Bounded so the direction property cannot be
/// confused with a large instance-count property.
const DIR_RANGE: std::ops::RangeInclusive<usize> = 4..=64;

/// A run of RAM shorter than this cannot hold a map worth reporting, and
/// skipping the short ones is most of what makes the scan cheap.
const MIN_RUN: usize = 8;

/// A window with fewer room entries than this is not believed to be a map.
/// Refusing beats naming a coincidence: `Kerkerkruip.gblorb` builds its dungeon
/// at run time, so its compiled `Map_Storage` is entirely zeros, and without
/// this floor the scan reports whichever three-room accident scores highest.
const MIN_MAP_ROOM_ENTRIES: usize = 4;

/// One Inform 7 story's compiled world model.
///
/// Built once per story with [`detect`](I7World::detect) — which is a scan over
/// RAM and is not free — and then asked about rooms, names and exits.
#[derive(Debug, Clone)]
pub struct I7World {
    room_prop: u16,
    dir_prop: u16,
    name_prop: Option<u16>,
    map_storage: u32,
    rooms: Vec<u32>,
    directions: Vec<u32>,
    room_index: HashMap<u32, usize>,
}

/// What one entry of a room's row in `Map_Storage` turns out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I7Exit {
    /// The entry named a room directly.
    Room(u32),
    /// The entry named a door, and the door's two sides are known: this is the
    /// side that is not the room we came from. `WorldModel.i6t`'s
    /// `FrontSideOfDoor`/`BackSideOfDoor` read a two-sided door's `found_in`
    /// array as `[front, back]`, which is static data.
    ThroughDoor { door: u32, to: u32 },
    /// The entry named a door whose destination this reader cannot resolve — a
    /// one-sided door, whose far side `WorldModel.i6t` computes by calling
    /// `door_to()`, or a two-sided door whose `found_in` array could not be
    /// identified.
    Door(u32),
}

impl I7Exit {
    /// The room this exit reaches, where that is known statically.
    pub fn destination(self) -> Option<u32> {
        match self {
            I7Exit::Room(r) | I7Exit::ThroughDoor { to: r, .. } => Some(r),
            I7Exit::Door(_) => None,
        }
    }
}

impl I7World {
    /// Derive this story's map, or `None` if it is not an Inform 7 image this
    /// reader recognises (an Inform 6 story, or an I7 build old enough to
    /// predate `Map_Storage` — `AnchorheadDemo.gblorb`, Inform 7 build 4K41,
    /// carries no instance-count properties at all).
    pub fn detect(mem: &Memory, names: &ParseNames) -> Option<I7World> {
        let objects: HashSet<u32> = names.objects().collect();
        let props = PropIndex::build(mem, names);
        let counts = props.instance_counts();

        let (dir_prop, directions) = props.direction_property(mem, names, &counts)?;
        let (room_prop, rooms, map_storage) =
            props.locate_map(mem, &objects, &counts, dir_prop, directions.len())?;

        let room_index = rooms.iter().enumerate().map(|(i, &a)| (a, i)).collect();
        let name_prop = props.printed_name_property(mem);
        Some(I7World {
            room_prop,
            dir_prop,
            name_prop,
            map_storage,
            rooms,
            directions,
            room_index,
        })
    }

    /// Every room, in `IK1_Count` order — which is also `Map_Storage` row order.
    pub fn rooms(&self) -> &[u32] {
        &self.rooms
    }

    /// Every direction object, in `IK3_Count` order — `Map_Storage` column order.
    pub fn directions(&self) -> &[u32] {
        &self.directions
    }

    /// Address of the `Map_Storage` array. In RAM: see the module header on why
    /// a story can move its own map out from under this.
    pub fn map_storage(&self) -> u32 {
        self.map_storage
    }

    /// The property number this story uses for `IK1_Count` (rooms), `IK3_Count`
    /// (directions) and `printed name` — all compiler-assigned, all different
    /// from story to story. Exposed for diagnostics, not for arithmetic.
    pub fn properties(&self) -> (u16, u16, Option<u16>) {
        (self.room_prop, self.dir_prop, self.name_prop)
    }

    /// Is `addr` one of this story's rooms?
    pub fn is_room(&self, addr: u32) -> bool {
        self.room_index.contains_key(&addr)
    }

    /// The raw `Map_Storage` entry for `room` in direction column `dir` — an
    /// object address, or `None` for a `0` entry (no exit that way) and for a
    /// `room`/`dir` that is not one of ours.
    pub fn raw_exit(&self, mem: &Memory, room: u32, dir: usize) -> Option<u32> {
        let r = *self.room_index.get(&room)?;
        if dir >= self.directions.len() {
            return None;
        }
        let cell = self.map_storage + ((r * self.directions.len() + dir) as u32) * 4;
        mem.read32(cell).filter(|&v| v != 0)
    }

    /// Every exit `room` declares, as the compiled map has it, with each
    /// direction resolved to a [`Compass`] where its dictionary words say so.
    ///
    /// A direction this reader cannot name — an author-defined one, which
    /// Counterfeit Monkey has eight of — is reported with `None` for the
    /// compass and the direction object's address, so a caller can still ask
    /// [`Self::printed_name`] what it is called.
    pub fn exits(
        &self,
        mem: &Memory,
        names: &ParseNames,
        room: u32,
    ) -> Vec<(Option<Compass>, u32, I7Exit)> {
        let mut out = vec![];
        for (d, &dir_obj) in self.directions.iter().enumerate() {
            let Some(entry) = self.raw_exit(mem, room, d) else {
                continue;
            };
            out.push((
                self.compass_of(mem, names, dir_obj),
                dir_obj,
                self.resolve(mem, names, room, entry),
            ));
        }
        out
    }

    /// What the story PRINTS for `obj` — its I7 `printed name`.
    ///
    /// I7 gives objects no hardware short name (measured: empty on every
    /// Counterfeit Monkey room), so [`ParseNames::short_name`] cannot answer
    /// this. The text lives in the `printed name` property, and a text property
    /// value takes one of two shapes:
    ///
    /// * a ROM address of a string object (`E0`/`E1`/`E2`), read directly —
    ///   which is what Inform 7 build 4K41 stores; or
    /// * a RAM address of an eight-byte record `{ kind tag, value }`, which is
    ///   what 6L38 through 10.1.2 store. When `value` is a string object the
    ///   text is a constant and is decoded here; when it is a routine the text
    ///   has substitutions in it ("the [colour] door") and only running the
    ///   story can say what it says, so this returns `None`.
    ///
    /// Measured on `CounterfeitMonkey-11.gblorb`, 2459 of 2480 objects carrying
    /// the property have a constant text; the other 21 are routines.
    pub fn printed_name(&self, mem: &Memory, names: &ParseNames, obj: u32) -> Option<String> {
        let prop = self.name_prop?;
        let value = names
            .property(mem, obj, prop)
            .and_then(|(d, _)| mem.read32(d))?;
        text_value(mem, value)
    }

    /// Turn a raw `Map_Storage` entry into an exit, resolving a two-sided door
    /// to the side that is not `from`.
    fn resolve(&self, mem: &Memory, names: &ParseNames, from: u32, entry: u32) -> I7Exit {
        if self.is_room(entry) {
            return I7Exit::Room(entry);
        }
        match self.door_sides(mem, names, entry) {
            Some((a, b)) if a == from => I7Exit::ThroughDoor { door: entry, to: b },
            Some((a, b)) if b == from => I7Exit::ThroughDoor { door: entry, to: a },
            _ => I7Exit::Door(entry),
        }
    }

    /// A two-sided door's `[front, back]` rooms, from the `found_in` array
    /// `WorldModel.i6t`'s `FrontSideOfDoor`/`BackSideOfDoor` read. The property
    /// NUMBER is not knowable, so the array is identified by its shape: exactly
    /// two words, both of them rooms.
    fn door_sides(&self, mem: &Memory, names: &ParseNames, door: u32) -> Option<(u32, u32)> {
        for prop in 0..=MAX_DOOR_PROP {
            let Some((data, 2)) = names.property(mem, door, prop) else {
                continue;
            };
            let (a, b) = (mem.read32(data)?, mem.read32(data.saturating_add(4))?);
            if a != b && self.is_room(a) && self.is_room(b) {
                return Some((a, b));
            }
        }
        None
    }

    /// Which compass point `dir_obj` is, by the dictionary words the parser
    /// accepts for it. `None` for an author-defined direction.
    fn compass_of(&self, mem: &Memory, names: &ParseNames, dir_obj: u32) -> Option<Compass> {
        let words = names.of(mem, dir_obj)?.words;
        Compass::ALL
            .into_iter()
            .find(|c| words.iter().any(|w| w.eq_ignore_ascii_case(c.word())))
    }
}

/// Highest property id [`I7World::door_sides`] scans for a door's `found_in`
/// array. Library properties land low; `crate::world`'s `MAX_PROP_SCAN`
/// explains at length why there is no authoritative bound to use instead.
const MAX_DOOR_PROP: u16 = 512;

/// Read a text-valued property's value. See [`I7World::printed_name`] for the
/// two shapes.
fn text_value(mem: &Memory, value: u32) -> Option<String> {
    let direct = |a: u32| {
        matches!(mem.read8(a), Some(0xe0) | Some(0xe1) | Some(0xe2))
            .then(|| crate::disasm::string_text(mem, mem.decode_table(), a, None))
            .flatten()
            .filter(|s| !s.is_empty())
    };
    // `value` is whatever a property held — an address, a small integer, or
    // `0xffffffff`. Every step past it is checked: an overflow here is a
    // panic in a debug build and a silent wrap in a release one, which is the
    // worst possible split for a reader that runs over arbitrary story data.
    if value >= mem.ramstart() {
        return direct(mem.read32(value.checked_add(4)?)?);
    }
    direct(value)
}

/// Every object's property table, read once. Building this is the only walk of
/// the object table any of the derivations below needs.
struct PropIndex {
    /// `prop -> [(object, first word of its value)]`, one-word properties only.
    singles: HashMap<u16, Vec<(u32, u32)>>,
    /// `object -> its dictionary words, lower-cased`.
    words: HashMap<u32, Vec<String>>,
}

impl PropIndex {
    fn build(mem: &Memory, names: &ParseNames) -> PropIndex {
        let mut singles: HashMap<u16, Vec<(u32, u32)>> = HashMap::new();
        let mut words = HashMap::new();
        for obj in names.objects() {
            if let Some(w) = names.of(mem, obj) {
                if !w.words.is_empty() {
                    words.insert(obj, w.words.iter().map(|s| s.to_lowercase()).collect());
                }
            }
            let Some(table) = mem.read32(obj.saturating_add(9 + names.attr_bytes())) else {
                continue;
            };
            let Some(entries) = mem.read32(table).filter(|&n| n <= 0x1000) else {
                continue;
            };
            for i in 0..entries {
                let e = table.saturating_add(4).saturating_add(i * 10);
                let (Some(id), Some(len), Some(data)) = (
                    mem.read16(e),
                    mem.read16(e.saturating_add(2)),
                    mem.read32(e.saturating_add(4)),
                ) else {
                    continue;
                };
                if len == 1 {
                    if let Some(v) = mem.read32(data) {
                        singles.entry(id as u16).or_default().push((obj, v));
                    }
                }
            }
        }
        PropIndex { singles, words }
    }

    /// Properties whose values are a bijection onto `0..n-1` — Inform 7's
    /// `IK<n>_Count` shape, one per kind that needs an instance index.
    fn instance_counts(&self) -> HashMap<u16, Vec<u32>> {
        let mut out = HashMap::new();
        for (&id, vs) in &self.singles {
            let n = vs.len();
            if !(2..=MAX_ROOMS).contains(&n) {
                continue;
            }
            let mut slots = vec![0u32; n];
            let mut seen = vec![false; n];
            let ok = vs.iter().all(|&(obj, v)| {
                (v as usize) < n && !std::mem::replace(&mut seen[v as usize], true) && {
                    slots[v as usize] = obj;
                    true
                }
            });
            if ok {
                out.insert(id, slots);
            }
        }
        out
    }

    /// The instance-count property carried by the compass objects, and the
    /// direction objects in `IK3_Count` order.
    ///
    /// Identified by the dictionary rather than by number: at least four of the
    /// members must answer to one of [`Compass`]'s twelve words. Four rather
    /// than twelve because a story may rename or drop directions, and more than
    /// one because a single scenery object called "north wall" is not a compass.
    fn direction_property(
        &self,
        mem: &Memory,
        names: &ParseNames,
        counts: &HashMap<u16, Vec<u32>>,
    ) -> Option<(u16, Vec<u32>)> {
        let mut best: Option<(usize, u16, Vec<u32>)> = None;
        for (&id, members) in counts {
            if !DIR_RANGE.contains(&members.len()) {
                continue;
            }
            let hits = Compass::ALL
                .into_iter()
                .filter(|c| {
                    members.iter().any(|&m| {
                        names.of(mem, m).is_some_and(|w| {
                            w.words.iter().any(|x| x.eq_ignore_ascii_case(c.word()))
                        })
                    })
                })
                .count();
            if hits >= 4 && best.as_ref().is_none_or(|(h, _, _)| hits > *h) {
                best = Some((hits, id, members.clone()));
            }
        }
        best.map(|(_, id, m)| (id, m))
    }

    /// The room instance-count property, the rooms in row order, and the
    /// address of `Map_Storage`.
    ///
    /// Scored by reciprocity — see the module header. The winner is the window
    /// with the most reciprocal room entries; ties go to the larger room count,
    /// because a spurious candidate is always a SUBSET shape (half the rows of
    /// the real array read as a smaller array of the same width).
    fn locate_map(
        &self,
        mem: &Memory,
        objects: &HashSet<u32>,
        counts: &HashMap<u16, Vec<u32>>,
        dir_prop: u16,
        n_dirs: usize,
    ) -> Option<(u16, Vec<u32>, u32)> {
        let runs = object_runs(mem, objects);
        let mut best: Option<(usize, usize, u16, u32)> = None; // (recip, n_rooms, prop, addr)
        for (&prop, members) in counts {
            if prop == dir_prop || members.len() < 2 {
                continue;
            }
            let need = members.len() * n_dirs;
            let index: HashMap<u32, usize> =
                members.iter().enumerate().map(|(i, &a)| (a, i)).collect();
            for run in &runs {
                if run.words.len() < need {
                    continue;
                }
                for off in 0..=(run.words.len() - need) {
                    let win = &run.words[off..off + need];
                    let (mut rooms, mut filled, mut recip) = (0usize, 0usize, 0usize);
                    for (i, &w) in win.iter().enumerate() {
                        if w == 0 {
                            continue;
                        }
                        filled += 1;
                        let Some(&to) = index.get(&w) else { continue };
                        rooms += 1;
                        let back = members[i / n_dirs];
                        if win[to * n_dirs..(to + 1) * n_dirs].contains(&back) {
                            recip += 1;
                        }
                    }
                    // A real map is SPARSE — most rooms leave most directions
                    // empty — and most of what it does hold is rooms rather than
                    // doors, and reciprocal. `oppositely-opal.gblorb` has a
                    // fully-dense run of object addresses next to its map that
                    // passes every other test here.
                    if filled * 2 > need || rooms < MIN_MAP_ROOM_ENTRIES || recip * 2 < rooms {
                        continue;
                    }
                    let key = (recip, members.len());
                    if best.as_ref().is_none_or(|(r, n, _, _)| key > (*r, *n)) {
                        best = Some((recip, members.len(), prop, run.addr + (off as u32) * 4));
                    }
                }
            }
        }
        let (_, _, prop, addr) = best?;
        Some((prop, counts[&prop].clone(), addr))
    }

    /// The `printed name` property: the one whose decoded text most often
    /// contains one of the object's own dictionary words.
    ///
    /// A story's objects carry several text properties — description, initial
    /// appearance, printed plural name — and only the printed name is
    /// systematically made of the words the parser accepts for the thing.
    /// Measured on `CounterfeitMonkey-11.gblorb`: the winner matches on 2188 of
    /// 2459 objects, the runner-up on 1431.
    fn printed_name_property(&self, mem: &Memory) -> Option<u16> {
        let mut best: Option<(usize, u16)> = None;
        for (&id, vs) in &self.singles {
            if vs.len() < 8 {
                continue;
            }
            let mut hits = 0;
            for &(obj, v) in vs {
                let Some(words) = self.words.get(&obj) else {
                    continue;
                };
                let Some(text) = text_value(mem, v) else {
                    continue;
                };
                if text.len() > 64 {
                    continue;
                }
                let low = text.to_lowercase();
                if words.iter().any(|w| low.contains(w.as_str())) {
                    hits += 1;
                }
            }
            if hits >= 4 && best.is_none_or(|(h, _)| hits > h) {
                best = Some((hits, id));
            }
        }
        best.map(|(_, id)| id)
    }
}

/// A maximal run of RAM words, at one byte phase, in which every word is `0` or
/// one of this story's object addresses.
struct Run {
    addr: u32,
    words: Vec<u32>,
}

/// Every such run, at all four byte phases.
///
/// The phases are the point: Inform packs its arrays and Glulx imposes no
/// alignment, so `Map_Storage` is at `0x378f28` in one story of this corpus and
/// at an address ≡ 1 (mod 4) in another. A scan that steps four bytes from
/// RAMSTART sees the first and is structurally blind to the second.
fn object_runs(mem: &Memory, objects: &HashSet<u32>) -> Vec<Run> {
    let mut out = vec![];
    for phase in 0..4u32 {
        let mut a = mem.ramstart() + phase;
        while a + 4 <= mem.endmem() {
            let w = mem.read32(a).unwrap_or(1);
            if w != 0 && !objects.contains(&w) {
                a += 4;
                continue;
            }
            let start = a;
            let mut words = vec![];
            while a + 4 <= mem.endmem() {
                let w = mem.read32(a).unwrap_or(1);
                if w != 0 && !objects.contains(&w) {
                    break;
                }
                words.push(w);
                a += 4;
            }
            if words.len() >= MIN_RUN && words.iter().any(|&w| w != 0) {
                out.push(Run { addr: start, words });
            }
        }
    }
    out
}
