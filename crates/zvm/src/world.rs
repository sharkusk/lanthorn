//! What the player can SEE in a room — inferred conventions, never spec (SQ-0678).
//!
//! # Why this module has to guess
//!
//! The Z-Machine Standards Document assigns **no meaning whatsoever** to object
//! attributes and properties (ZMSD §12.3, §12.4: "the interpreter is not
//! interested in what they mean"). Every one of them is defined by the story's
//! own compiler and source. So "is this container open?" and "which objects are
//! visible in this room but not children of it?" are questions the spec cannot
//! answer — they can only be *inferred* from the shape of the story's own data.
//!
//! Everything here is therefore a heuristic with an explicit failure mode, and
//! every failure mode points the same way: **show less**. When an inference is
//! not confident the model reports `None` and the caller falls back to the
//! room's direct children, which is what lanthorn listed before this module
//! existed.
//!
//! # The two things it infers
//!
//! 1. **The openness attribute** — the bit a story sets on a container whose
//!    contents are currently visible. Needed because a room's here-list must
//!    include the sack and bottle sitting on the kitchen table (children of the
//!    *table*, not of the room) while never revealing the lunch and garlic
//!    inside the *closed* sack. Listing those would be cheating: the game has
//!    not shown them to the player.
//!
//! 2. **The local-globals property** — in Infocom's ZIL, scenery shared by many
//!    rooms (a window, a chimney, the forest) lives in one bucket object and
//!    each room names the ones it can see in a property (`GLOBAL` in the ZIL
//!    sources). Such an object is never a child of the room, so a
//!    children-only walk misses the window at Behind House entirely.
//!
//! # How stable is any of this?
//!
//! Measured, not assumed. Attribute numbers are **not** portable between
//! Infocom games — Zork I r52 marks openness with attribute 28 while Mini-Zork
//! r34 uses attribute 10, and the container bit is 34 vs 9 respectively. The
//! same holds for the local-globals property: 37 in Zork I, 12 in Mini-Zork and
//! Zork II, 7 in Zork III, 17 in Enchanter, 41 in Planetfall. Nothing may be
//! hard-coded; the numbers have to be recovered per story, at run time, from
//! the story's own table.
//!
//! Inform-compiled stories are a different family: they have a fixed library
//! attribute set (`container`, `open`, `openable`, `supporter`, `transparent`)
//! and no local-globals concept at all — visibility of shared scenery is done
//! with `found_in`, not a property listing object numbers. The local-globals
//! walk is therefore switched off outright for stories that identify themselves
//! as Inform-compiled, and the openness inference simply comes up empty on them
//! more often than not (Inform's `openable` is as container-shaped as `open`,
//! and there is no honest way to tell them apart from the table alone). That is
//! the intended outcome: no nesting, direct children only.

use crate::memory::Memory;
use crate::objects::{
    get_attr, get_child, get_next_prop, get_parent, get_prop_addr, get_prop_len, get_sibling,
};

/// Deepest the here-list walks below the room itself: room → child →
/// grandchild → great-grandchild. A visible box on a visible table is real, but
/// past three levels the payoff is nil and the blast radius of a mis-inferred
/// openness bit grows with every level.
const MAX_NEST_DEPTH: u8 = 3;

/// Hard ceiling on one room's here-list. A corrupt or hostile tree can form a
/// cycle; the depth cap alone would not stop a wide one.
const MAX_ITEMS: usize = 64;

/// Longest sibling chain the walker will follow before giving up on it.
const MAX_SIBLINGS: usize = 256;

/// Conventions recovered from one story's object table.
///
/// Cheap to hold, moderately expensive to build (a couple of passes over every
/// object × every attribute), and **constant for the life of the story** — the
/// numbers describe the compiler's layout, not the game state. Callers build it
/// once and keep it; the live game state is read through it on every query, so
/// a container that the game opens mid-turn becomes visible on the next refresh
/// without rebuilding anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorldModel {
    /// Highest valid object number (`location::max_object_number`).
    pub max_object: u16,
    /// The object every room hangs off (0 when rooms are top-level, as in
    /// Inform). Used only to tell rooms from things.
    pub room_holder: u16,
    /// The attribute that marks "this object can hold things", when one was
    /// identified confidently.
    pub container_attr: Option<u8>,
    /// The attribute that marks "this holder's contents are visible right now".
    /// `None` means *do not nest* — see the module docs.
    pub open_attr: Option<u8>,
    /// The property on a room that lists shared-scenery object numbers.
    pub globals_prop: Option<u8>,
    /// The bucket object those shared-scenery objects live in. Always `Some`
    /// exactly when `globals_prop` is.
    pub globals_holder: Option<u16>,
    /// The twelve `*_to` exit-property numbers (SQ-1257), indexed by
    /// [`Compass`] — `exit_props[Compass::N as usize]` is the property number
    /// `n_to` was compiled to, and so on. `None` at every index when the
    /// `door_dir` convention (see [`Self::declared_exit`]) could not be
    /// identified, which is every non-Inform-library story.
    pub exit_props: [Option<u8>; 12],
    /// The `door_to` property number, when a "two-way door" object could be
    /// found and cross-checked (see [`Self::declared_exit`]). `None` is the
    /// ordinary case for a story with no doors, or one where the doors found
    /// disagreed too much to trust a single property number.
    pub door_to_prop: Option<u8>,
    /// The `door_dir` property number itself (SQ-1257) — what tells
    /// [`Self::declared_exit`] whether an exit's target object is a room or a
    /// connector to follow through [`Self::door_to_prop`]. Named distinctly
    /// from `exit_props` (which holds the *contents* `door_dir` gives back)
    /// because this is the property number that stores that mapping, not one
    /// derived from it.
    door_dir_prop_hint: Option<u8>,
    /// The ZIL exit-property numbers (SQ-1260), indexed by [`Compass`] exactly
    /// like `exit_props` — but derived from Infocom's OWN compiler convention
    /// (the `DIR`-flagged dictionary words, see the "Declared exits: ZIL"
    /// section below) rather than Inform's `door_dir`. `None` at every index
    /// for an Inform-compiled story (where `exit_props` is the one that's
    /// populated instead — [`Self::discover`] tries Inform first and only
    /// looks for this convention when that comes up empty) or for a story
    /// whose dictionary carries no `DIR`-flagged words at all.
    pub zil_exit_props: [Option<u8>; 12],
    /// How many bytes a ZIL UEXIT/DEXIT destination-room reference occupies
    /// in THIS story's compiled exit tables (SQ-1268) — 1 or 2. Only
    /// meaningful when `zil_exit_props` has anything `Some`; `0` otherwise
    /// (never consulted then). Always 1 for a V3 story (SQ-1260's original,
    /// unchanged derivation — V3 object numbers are one byte, ZMSD §12.3).
    /// For V4+, this is NOT implied by the Z-machine version: it was measured
    /// to vary per STORY (Trinity/AMFV/Bureaucracy/Beyond Zork all compile
    /// 2-byte room references; Sherlock and every V6 title checked compile
    /// 1-byte ones, matching V3's own convention despite being V5/V6) — see
    /// [`infer_zil_room_width`] and the "Declared exits: ZIL" module docs.
    zil_room_width: u8,
}

impl WorldModel {
    /// Recover the model from the story's **pristine** object table.
    ///
    /// Always prefer this over [`Self::discover`] on a running machine. The
    /// inference reads holder sets and attribute populations, and those drift as
    /// the game is played — open a door, take a lamp, and an attribute that was
    /// unique at boot is no longer unique. Deriving from the boot image makes
    /// the answer a property of the *story* rather than of when the caller
    /// happened to ask, which also means a save restored into a fresh session
    /// gets exactly the same model.
    pub fn discover_at_boot(machine: &crate::cpu::exec::Machine) -> Self {
        let mut buf = machine.mem.raw_bytes().to_vec();
        let n = machine.original_dynamic.len().min(buf.len());
        buf[..n].copy_from_slice(&machine.original_dynamic[..n]);
        match Memory::new(buf) {
            Ok(pristine) => Self::discover(&pristine),
            // A story image we cannot re-wrap is one we should not reason about
            // from a half-known state either, but the live table is strictly
            // better than nothing and every downstream guard still applies.
            Err(_) => Self::discover(&machine.mem),
        }
    }

    /// Recover the model from an object table as it stands right now. Public for
    /// tests and for callers holding only a `Memory`; see
    /// [`Self::discover_at_boot`] for why a running machine wants that instead.
    pub fn discover(mem: &Memory) -> Self {
        let max_object = crate::location::max_object_number(mem);
        if max_object == 0 {
            return Self::default();
        }
        let nattr = attr_count(mem);

        // Parser dummy objects ("it", "that", "random object") carry nearly
        // every attribute at once so that any `test_attr` on them answers yes.
        // They are noise in every set below — one such object would make two
        // unrelated attributes look like they overlap.
        let real: Vec<u16> = (1..=max_object).filter(|&o| !is_wildcard(mem, o, nattr)).collect();

        // Holder sets, one per attribute, over real objects only.
        let sets: Vec<Vec<u16>> = (0..nattr)
            .map(|a| real.iter().copied().filter(|&o| get_attr(mem, o, a)).collect())
            .collect();

        let room_holder = modal_parent(mem, &real);
        let is_room = |o: u16| get_parent(mem, o) == room_holder;

        // Things (not rooms) that actually hold something at this instant.
        let holders: Vec<u16> =
            real.iter().copied().filter(|&o| get_child(mem, o) != 0 && !is_room(o)).collect();

        let container_attr = infer_container_attr(&sets, &holders);
        let open_attr = container_attr.and_then(|c| infer_open_attr(&sets, c, real.len()));
        let (globals_prop, globals_holder) =
            infer_local_globals(mem, max_object, room_holder, &real);
        let (exit_props, door_dir_prop_hint, door_to_prop) = infer_exits(mem, max_object);
        // ZIL only where Inform found nothing — the two conventions have never
        // been observed to both match one story, but there is no reason to let
        // a ZIL-shaped table override a real Inform derivation if that ever
        // changes (SQ-1260).
        let zil_exit_props = if exit_props.iter().all(Option::is_none) {
            infer_zil_exits(mem, max_object).unwrap_or([None; 12])
        } else {
            [None; 12]
        };
        // SQ-1268: the room-reference width is a V3 constant (1 byte, ZMSD
        // §12.3) but a per-STORY fact for V4+ — see `infer_zil_room_width`.
        // Skipped (left 0, never consulted) when no ZIL convention was found
        // at all, and not re-derived for V3 since that path is unchanged.
        let zil_room_width = if !zil_exit_props.iter().any(Option::is_some) {
            0
        } else if mem.version() <= 3 {
            1
        } else {
            infer_zil_room_width(mem, max_object, &zil_exit_props)
        };

        Self {
            max_object,
            room_holder,
            container_attr,
            open_attr,
            globals_prop,
            globals_holder,
            exit_props,
            door_to_prop,
            door_dir_prop_hint,
            zil_exit_props,
            zil_room_width,
        }
    }

    /// What room `origin`'s own map data declares for `dir`, resolving through
    /// a two-way "door" object when the property points at one (SQ-1257).
    ///
    /// This is a STATIC read of the story's compiled table — the same data
    /// [`crate::objects::get_prop`] would return whether or not the direction
    /// has ever been walked — so it can be asked about any room the caller
    /// already knows the number of, not only the one the player is standing
    /// in. See the module docs on [`Compass`] and [`DeclaredExit`] for what the
    /// answer does and does not promise.
    pub fn declared_exit(&self, mem: &Memory, origin: u16, dir: Compass) -> DeclaredExit {
        self.declared_exit_detail(mem, origin, dir).flatten()
    }

    /// The same derivation as [`Self::declared_exit`], keeping the SHAPE the
    /// story compiled instead of flattening it away (SQ-1306).
    ///
    /// [`DeclaredExit`] is deliberately lossy, because it answers the one
    /// question `session::apply_turn` asks — "does this direction lead to a
    /// fixed room, and which one?" So a ZIL CEXIT (a real destination behind a
    /// global flag) and a ZIL FEXIT (a routine nothing can resolve statically)
    /// both collapse to [`DeclaredExit::Code`], and both DEXIT and Inform's
    /// `door_to` hop collapse to a bare [`DeclaredExit::Room`] with the door
    /// thrown away.
    ///
    /// That is the right answer for a live turn and the wrong one for anything
    /// DRAWING the map. A static map generator wants the conditional exit's
    /// destination — Zork I's grating and trap door are CEXITs, and dropping
    /// them loses real passages — and wants to say which edges are doors. This
    /// returns all of it, and [`Self::declared_exit`] is now literally
    /// `declared_exit_detail(..).flatten()`, so there is exactly ONE
    /// description of each compiled shape and the two can never disagree.
    pub fn declared_exit_detail(&self, mem: &Memory, origin: u16, dir: Compass) -> ExitDetail {
        if let Some(prop) = self.exit_props[dir as usize] {
            if origin == 0 || origin > self.max_object {
                return ExitDetail::Unknown;
            }
            let raw = crate::objects::get_prop(mem, origin, prop);
            // Distinguished from `Unknown` (SQ-1257 Phase 2): the compass WAS
            // identified — `exit_props[dir]` answered — so a zero here is this
            // ROOM declaring nothing for this direction, not a story with no
            // `door_dir` convention at all. Lost Pig's gnome-tunnel rooms are
            // exactly this: `door_dir`/`*_to` are real and derived (see
            // `infer_exits`), and every one of their `*_to` properties is simply
            // absent — a "before going" rule intercepts the move before the
            // library's own exit-table code ever reads it.
            if raw == 0 {
                return ExitDetail::Absent;
            }
            return self.resolve(mem, origin, raw);
        }
        // SQ-1260: the ZIL convention — see "Declared exits: ZIL" below. Same
        // origin-range guard, same `Absent`-for-a-property-this-room-simply-
        // doesn't-declare contract as the Inform branch above; the shape of
        // the property's DATA is what differs, so the byte-length dispatch
        // lives in `resolve_zil` rather than `resolve`.
        let Some(prop) = self.zil_exit_props[dir as usize] else { return ExitDetail::Unknown };
        if origin == 0 || origin > self.max_object {
            return ExitDetail::Unknown;
        }
        self.resolve_zil(mem, origin, prop)
    }

    /// One ZIL room's raw exit-property bytes for `prop`, already known
    /// present on `origin` (SQ-1260, widened to V4+ by SQ-1268) — the
    /// UEXIT/NEXIT/FEXIT/CEXIT/DEXIT shapes [`infer_zil_exits`] found this
    /// story's `<DIRECTIONS>` compiled to. See "Declared exits: ZIL" below
    /// for the citations and the exact byte layouts this switches on.
    ///
    /// Every shape's length is `self.zil_room_width` (`w`) plus a fixed
    /// offset — `w` itself for UEXIT, `w+1` for NEXIT, and so on through
    /// DEXIT at `w+4` — reproducing V3's original fixed table (`w` is always
    /// 1 there) unchanged, and matching Trinity's ground-truth-verified V4
    /// byte layout exactly at `w=2`. See the module docs for the citations
    /// and for why this does NOT collide the way the original SQ-1260
    /// comment worried a fixed 2-byte NEXIT would: NEXIT scales with `w`
    /// too, so it never lands on UEXIT's length.
    fn resolve_zil(&self, mem: &Memory, origin: u16, prop: u8) -> ExitDetail {
        let addr = crate::objects::get_prop_addr(mem, origin, prop);
        if addr == 0 {
            // The compass word IS a real ZIL direction in this story — that's
            // how `prop` was found at all — and this room simply declares
            // nothing for it: the ZIL-side equivalent of the Inform branch's
            // Lost Pig case above, and of SQ-1257 Phase 2's `Absent`.
            return ExitDetail::Absent;
        }
        let len = crate::objects::get_prop_len(mem, addr);
        let w = self.zil_room_width.max(1) as u16;
        // A destination room number that isn't a plausible object is not a
        // room this derivation can vouch for — refuse rather than mint a
        // `Room` that resolves to nothing, the same discipline `resolve`'s
        // Inform branch applies to `raw > self.max_object`.
        let plausible = |dest: u16| dest != 0 && dest <= self.max_object;
        let room = |dest: u16| {
            if plausible(dest) {
                ExitDetail::Room(dest)
            } else {
                ExitDetail::Code
            }
        };
        // UEXIT/DEXIT's destination room is the property's first `w` bytes —
        // a single byte (V3, and every V4+ story measured with few enough
        // objects to fit one) or a big-endian word (Trinity/AMFV/Bureaucracy/
        // Beyond Zork's wider compile) — read the same way regardless of
        // which of the two shapes this is, since both mean the same thing to
        // the caller (see the DEXIT case below).
        let room_ref = || -> u16 {
            if w == 1 { mem.read_byte(addr as u32) as u16 } else { mem.read_word(addr as u32) }
        };
        let len = len as u16;
        if len == w {
            // UEXIT: the property's data IS the destination room number —
            // nothing else is stored.
            room(room_ref())
        } else if len == w + 1 {
            // NEXIT: a packed STRING address (the refusal message) and
            // nothing else — there is no passage here in any state the game
            // can be in, so this is `Message`, not `Code` (SQ-1260: distinct
            // from a computed exit specifically so Phase 2 never wastes a
            // probe on a direction that can never lead anywhere).
            ExitDetail::Message
        } else if len == w + 2 {
            // FEXIT: a packed ROUTINE address decides, at run time, whether
            // and where the player moves.
            ExitDetail::Code
        } else if len == w + 3 {
            // CEXIT: [room][global variable number][packed string address] —
            // gated on a global the story can flip on any later turn, exactly
            // as dynamic as Inform's `door_dir` pointing at a routine. This
            // length is EXTRAPOLATED, not independently confirmed — see the
            // module docs' "no CEXIT example found" note.
            //
            // The destination is a real, static room number and is kept here
            // (SQ-1306); `flatten` throws it away again so `declared_exit`
            // answers `Code` exactly as it always has. A live turn must not
            // trust it — the global may be clear — but a MAP drawn from the
            // story file should show the passage, because it is one.
            let dest = room_ref();
            if plausible(dest) {
                ExitDetail::Conditional { dest, gate: mem.read_byte(addr as u32 + w as u32) }
            } else {
                ExitDetail::Code
            }
        } else if len == w + 4 {
            // DEXIT: [room][door object][packed string address]. The
            // destination is a STATIC room in every DEXIT this derivation has
            // been checked against — the compiler never stores a routine
            // there, only whether the move actually lands this turn depends
            // on the door, and `declared_exit`'s only caller
            // (`session::apply_turn`'s Phase 1) acts on this exclusively when
            // the player's live move actually changed rooms (the
            // `moved_room` guard) — a shut door means no move at all, so it
            // never mints a false edge from a `Room` this turn's door
            // happened to refuse.
            //
            // Which door it is has been kept since SQ-1306 — `flatten` drops
            // it back to a bare `Room`, so every existing caller sees exactly
            // what it saw before, and a map generator can label the edge.
            // Door offsets straight off the two tables in "Declared exits:
            // ZIL" below, which is the only place they are written down:
            // `[room:1][door:1][string:2][pad:1]` at `w == 1`, and
            // `[room:2][door:2][string:2]` at `w == 2`. So the door slot is
            // one byte at `addr + 1` narrow and a word at `addr + 2` wide —
            // it is `w`-sized, exactly like the room reference beside it.
            let dest = room_ref();
            let door = if w == 1 {
                mem.read_byte(addr as u32 + 1) as u16
            } else {
                mem.read_word(addr as u32 + 2)
            };
            match (plausible(dest), plausible(door)) {
                (true, true) => ExitDetail::Door { dest, door },
                (true, false) => ExitDetail::Room(dest),
                (false, _) => ExitDetail::Code,
            }
        } else {
            // A length none of the five known shapes produce — refuse rather
            // than guess at a sixth shape from one story's byte layout.
            ExitDetail::Code
        }
    }

    /// One step of exit resolution: classify a raw, NONZERO `*_to` (or
    /// `door_to`) value already read off some object's property table. The
    /// zero case is handled by the caller — see [`Self::declared_exit`] for why
    /// it means something different at the top level (`Absent`) than partway
    /// through a door hop (`Code`, below: a door with no static far side is
    /// exactly as unresolvable as one whose `door_to` is a routine).
    fn resolve(&self, mem: &Memory, holder: u16, raw: u16) -> ExitDetail {
        if raw > self.max_object {
            // A packed routine or string address — GoSub's `metaclass() ==
            // Routine`/`String` branches. zvm has no general way to tell those
            // two apart without executing the story (see module docs), so both
            // collapse to `Code`; `Message` is reserved for a future refinement.
            return ExitDetail::Code;
        }
        // `raw` is a plausible object number. If it does not itself carry the
        // `door_dir` property, it is not a connector in this story's
        // convention (see `infer_exits`) and IS the destination room.
        let Some(dd) = self.door_dir_prop_hint else { return ExitDetail::Room(raw) };
        if crate::objects::get_prop_addr(mem, raw, dd) == 0 || raw == holder {
            return ExitDetail::Room(raw);
        }
        // `raw` is a door. Follow `door_to`, one hop only — GoSub itself never
        // chases a second door from the far side of the first.
        let Some(door_to) = self.door_to_prop else { return ExitDetail::Code };
        let k = crate::objects::get_prop(mem, raw, door_to);
        if k == 0 {
            return ExitDetail::Code;
        }
        if k > self.max_object {
            return ExitDetail::Code;
        }
        // The hop succeeded, and `raw` was the door it went through — kept
        // since SQ-1306 so a map can label the edge. `flatten` drops it back
        // to `Room(k)`, which is what every caller before that saw.
        ExitDetail::Door { dest: k, door: raw }
    }

    /// True when `obj`'s contents are visible to the player right now.
    ///
    /// Read live from the object table on every call, so opening a box shows
    /// its contents on the very next refresh and closing it hides them again.
    /// Answers `false` for every object when the openness bit was not
    /// identified — the fail-toward-less default.
    pub fn shows_contents(&self, mem: &Memory, obj: u16) -> bool {
        match self.open_attr {
            Some(a) => get_attr(mem, obj, a),
            None => false,
        }
    }

    /// The shared-scenery objects this room can see, in the order the story
    /// lists them. Empty when no local-globals convention was identified.
    pub fn local_globals(&self, mem: &Memory, room: u16) -> Vec<u16> {
        let (Some(prop), Some(holder)) = (self.globals_prop, self.globals_holder) else {
            return Vec::new();
        };
        object_list_prop(mem, room, prop, self.max_object)
            .into_iter()
            .filter(|&o| get_parent(mem, o) == holder)
            .collect()
    }

    /// Everything the player can see in `room`, as object numbers, in reading
    /// order: each direct child, followed immediately by the contents of any
    /// child whose contents are visible (recursively, to [`MAX_NEST_DEPTH`]),
    /// then the room's shared scenery.
    ///
    /// `exclude` (0 for none) is dropped along with its whole subtree — the
    /// caller passes the player object, and the player's pockets are the
    /// *carried* list, never the *here* list.
    ///
    /// # Leak safety
    ///
    /// The walk descends into a holder only when [`Self::shows_contents`] says
    /// its contents are visible, and that is `false` for every object unless a
    /// specific openness bit was identified for this story. So the two failure
    /// modes are: the bit is unknown (nothing nests — the pre-SQ-0678
    /// behaviour), or the bit is known and a closed container is skipped. There
    /// is no path on which an unopened container's contents reach this list.
    pub fn visible_room_objects(&self, mem: &Memory, room: u16, exclude: u16) -> Vec<u16> {
        if room == 0 || room > self.max_object {
            return Vec::new();
        }
        let mut out = self.visible_contents(mem, room, exclude);
        for g in self.local_globals(mem, room) {
            if out.len() >= MAX_ITEMS {
                break;
            }
            if g != exclude && g != room && !out.contains(&g) {
                out.push(g);
            }
        }
        out
    }

    /// Everything the player can see inside `holder`, as object numbers, in the
    /// same reading order and under the same leak guard as
    /// [`Self::visible_room_objects`] — minus the shared-scenery pass, which is
    /// a room's property and nothing a holder has.
    ///
    /// This is the CARRIED half of scope (SQ-1133). It used to be the holder's
    /// direct children and nothing more, so a room and a rucksack were read by
    /// two different rules: the sack on Zork I's kitchen table listed its lunch
    /// once opened, and the same sack in the player's hands did not. One walk
    /// answers both, so the two cannot drift apart again.
    ///
    /// The depth cap is [`MAX_NEST_DEPTH`], shared with the room walk for the
    /// same reason. Measured need on the carried side is **one** level — Zork I
    /// r88 and Mini-Zork r34 both put the lunch and the garlic one below an
    /// opened sack — and every level past that costs nothing while a holder
    /// stays shut.
    pub fn visible_contents(&self, mem: &Memory, holder: u16, exclude: u16) -> Vec<u16> {
        let mut out = Vec::new();
        if holder == 0 || holder > self.max_object {
            return out;
        }
        self.walk(mem, holder, exclude, 0, &mut out);
        out
    }

    fn walk(&self, mem: &Memory, parent: u16, exclude: u16, depth: u8, out: &mut Vec<u16>) {
        let mut child = get_child(mem, parent);
        let mut guard = 0usize;
        while child != 0 && out.len() < MAX_ITEMS && guard < MAX_SIBLINGS {
            guard += 1;
            if child != exclude && !out.contains(&child) {
                out.push(child);
                if depth + 1 < MAX_NEST_DEPTH && self.shows_contents(mem, child) {
                    self.walk(mem, child, exclude, depth + 1, out);
                }
            }
            child = get_sibling(mem, child);
        }
    }
}

// ── Declared exits (SQ-1257) ─────────────────────────────────────────────────
//
// A room object's exit in a given compass direction is DATA the story compiled
// in, not something that can only be learned by walking it — the same `n_to`
// (etc.) property `verblib.h`'s `GoSub` reads to decide where "go north"
// leads. Reading it independently of any move lets the mapper tell a REAL
// passage from one a routine improvised on the spot (Lost Pig's gnome tunnels,
// which relocate the player somewhere the room's own exit table never named).
//
// The two library conventions this recovers, both from `inform6lib`
// (https://github.com/DavidGriffith/inform6lib):
//
// * **`door_dir`** (`english.h`): each compass-direction object (the parser's
//   "north", "south", …) carries a `door_dir` property whose VALUE is the
//   property number of the matching `*_to` — `n_obj` declares `door_dir
//   n_to`, `s_obj` declares `door_dir s_to`, and so on
//   (english.h:47-70). `verblib.h`'s `GoSub` reads it exactly this way:
//   `thedir = noun.door_dir; next_loc = i.thedir;` (verblib.h:2071,2090) — a
//   property NUMBER held in a variable, dereferenced with `.thedir`, which is
//   Inform 6's "property by number" form.
// * **`door_to`** (`linklpa.h`): when the exit property's value is an object
//   with `has door` set — a "tunnel to east"-style connector — `GoSub` takes
//   one more hop through that object's `door_to` property:
//   `k = RunRoutines(next_loc, door_to); ... next_loc = k;` (verblib.h:2093-2096).
//
// Property NUMBERS are never portable between compiles — Lost Pig's `door_dir`
// is property 34 and its `*_to` set is 20–31, nothing like `linklpa.h`'s own
// declaration order, because Inform 7 injects a great many properties of its
// own ahead of the library's. Every number below is recovered from the STORY,
// never assumed from the library source.

/// The twelve directions a room's exit table may name (SQ-1257).
///
/// A `zvm`-local type rather than `mapper::direction::Direction` — `zvm` takes
/// no dependency on the app's mapper crate — but ordered exactly as
/// `inform6lib/english.h` declares its compass objects, so
/// [`WorldModel::exit_props`] can be indexed by `dir as usize` directly. A
/// caller holding a `mapper::Direction` maps it to this with a `match`; there
/// is deliberately no `Unknown` member here, because "no direction" is not a
/// question this type can be asked — the caller simply does not call
/// [`WorldModel::declared_exit`] for one.
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
    /// [`crate::objects::ParseNames::find`] is asked for.
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

/// What a room's own exit table declares for one direction, in the shape the
/// story compiled it (SQ-1306) — see [`WorldModel::declared_exit_detail`].
///
/// This is the derivation's full answer; [`DeclaredExit`] is the projection of
/// it that a live turn wants, and [`ExitDetail::flatten`] is the only place the
/// two are related. Every variant here means what its [`DeclaredExit`]
/// counterpart means; the two extra ones carry a fact `DeclaredExit` has
/// nowhere to put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitDetail {
    /// An unconditional passage to a fixed room: Inform's `*_to` naming a room
    /// directly, or ZIL's UEXIT.
    Room(u16),
    /// ZIL's CEXIT: a passage to `dest` the story allows only while a condition
    /// holds. The destination is static and real — what the condition gates is
    /// whether the move HAPPENS, not where it goes — so a map drawn from the
    /// story file should show it, marked conditional. A live turn must not
    /// trust it, which is why [`Self::flatten`] answers [`DeclaredExit::Code`].
    ///
    /// `gate` is the CEXIT's second byte, RAW and deliberately unattributed.
    /// The V3 table below calls that slot `[global:1]`, and it is not the
    /// global's Z-machine variable number: on Zork I r52 the seven CEXITs that
    /// the game gates on RAINBOW-FLAG (Aragain Falls and End of Rainbow → On
    /// the Rainbow) and on WON-FLAG (West of House → Stone Barrow) ALL read 0,
    /// and two distinct flags cannot be one variable. The other eighteen read
    /// 75–101, which look like variable numbers and may be. Until something
    /// authoritative says which it is, callers get the byte and no claim about
    /// it — `lanthorn-mapgen` prints "conditional" and does not name a global.
    Conditional { dest: u16, gate: u8 },
    /// A passage to `dest` through door object `door`: ZIL's DEXIT, or Inform's
    /// `*_to` naming a door whose `door_to` names the far side. Whether the
    /// move lands this turn depends on the door being open; where it goes does
    /// not, so [`Self::flatten`] answers [`DeclaredExit::Room`].
    Door { dest: u16, door: u16 },
    /// The destination is computed at run time — ZIL's FEXIT, or an Inform
    /// `*_to`/`door_to` holding a routine.
    Code,
    /// A fixed refusal message and no passage at all: ZIL's NEXIT.
    Message,
    /// The compass was identified for this story and this room declares
    /// nothing for it. See [`DeclaredExit::Absent`].
    Absent,
    /// No exit is declared this way at all. See [`DeclaredExit::Unknown`].
    Unknown,
}

impl ExitDetail {
    /// Project down to the [`DeclaredExit`] a live turn asks for: keep the
    /// destination where one is static enough to walk, and throw away the door
    /// and the conditional's destination.
    ///
    /// `Conditional` becomes [`DeclaredExit::Code`] rather than
    /// [`DeclaredExit::Room`] deliberately — the global may be clear, and
    /// `session::apply_turn` mints an edge from a `Room` answer.
    pub fn flatten(self) -> DeclaredExit {
        match self {
            ExitDetail::Room(r) | ExitDetail::Door { dest: r, .. } => DeclaredExit::Room(r),
            ExitDetail::Conditional { .. } | ExitDetail::Code => DeclaredExit::Code,
            ExitDetail::Message => DeclaredExit::Message,
            ExitDetail::Absent => DeclaredExit::Absent,
            ExitDetail::Unknown => DeclaredExit::Unknown,
        }
    }

    /// The room this passage leads to, when the derivation could name one —
    /// including a conditional's destination, which [`Self::flatten`] drops.
    pub fn destination(self) -> Option<u16> {
        match self {
            ExitDetail::Room(r)
            | ExitDetail::Conditional { dest: r, .. }
            | ExitDetail::Door { dest: r, .. } => Some(r),
            _ => None,
        }
    }
}

/// What a room's own exit table declares for one direction (SQ-1257) — read
/// from the story's compiled data, independent of anything ever having been
/// walked. See [`WorldModel::declared_exit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredExit {
    /// The exit is a fixed room: the property named it directly, or named a
    /// two-way "door" object whose own `door_to` names it.
    Room(u16),
    /// The destination is computed at run time — the property (or a door's
    /// `door_to`) holds a routine, so nothing here can say where it leads.
    Code,
    /// The property holds a printed string (a fixed refusal message: "the
    /// window is stuck shut" and the like) rather than a destination at all.
    ///
    /// Currently unreachable from this derivation — `zvm` has no equivalent of
    /// the Z-Machine's `metaclass` opcode to tell a packed STRING address from
    /// a packed ROUTINE address without executing the story, so both read as
    /// [`DeclaredExit::Code`] today. Kept as a distinct variant for callers
    /// that want to special-case it once that distinction is implemented,
    /// rather than changing the shape of this type twice.
    Message,
    /// The compass WAS identified for this story, and this room's `*_to`
    /// property for this direction is simply absent (SQ-1257 Phase 2) — the
    /// room declares NOTHING here, as opposed to declaring code the derivation
    /// merely cannot resolve ([`Self::Code`]). Lost Pig's gnome-tunnel rooms
    /// are exactly this: their exit properties are unset because a "before
    /// going" rule intercepts the move before the library's own exit-table
    /// code ever reads it. Distinct from [`Self::Unknown`] so a caller can
    /// treat "this story has no data here" (worth a Phase 2 probe) differently
    /// from "this story has no `door_dir` convention at all" (Zork I, Glulx,
    /// Scott — never worth probing, since there is no reason to think ANY
    /// property here means an exit).
    Absent,
    /// No exit is declared this way at all: the room number is out of range,
    /// or this story's `door_dir` convention could not be identified (every
    /// non-Inform-library story, e.g. Zork I).
    Unknown,
}

/// Derive the `*_to` property numbers, the `door_dir` property number itself,
/// and (best-effort) the `door_to` property number, all from the compiled
/// object table. `None` in every slot for a story with no `door_dir`
/// convention to find (`ParseNames::detect` failing, or none of the twelve
/// compass words resolving to an object) — a Scott Adams or Glulx-shaped table
/// has nothing here to recover, and neither does a story whose parser-name
/// property isn't the one this searches through.
fn infer_exits(mem: &Memory, max_object: u16) -> ([Option<u8>; 12], Option<u8>, Option<u8>) {
    let none = ([None; 12], None, None);
    if max_object == 0 {
        return none;
    }
    let Some(pn) = crate::objects::ParseNames::detect(mem) else { return none };

    // Only the eight cardinal/intercardinal words are trusted to IDENTIFY the
    // compass objects (and so to derive `door_dir` from) — each is a
    // multi-letter word essentially no other object's vocabulary collides
    // with. "up", "down", "in" and "out" are common enough that
    // [`crate::objects::ParseNames::find`] can and does return the wrong
    // object for them: Curses' "in" resolves first to a "ship in a bottle"
    // (whatever holds the word "in" among its own adjectives/nouns), not a
    // direction. Those four are still read below, but only ACCEPTED once
    // `door_dir` is known and their own candidate can be checked against it.
    const PRIMARY: [Compass; 8] =
        [Compass::N, Compass::S, Compass::E, Compass::W, Compass::Ne, Compass::Nw, Compass::Se, Compass::Sw];

    let mut primary_ids: [Option<u16>; 12] = [None; 12];
    for dir in PRIMARY {
        if let Some(o) = pn.find(mem, dir.word()) {
            primary_ids[dir as usize] = Some(o.id as u16);
        }
    }
    let found: Vec<(Compass, u16)> = PRIMARY
        .into_iter()
        .filter_map(|d| primary_ids[d as usize].map(|id| (d, id)))
        .collect();
    // Need at least six of the eight to trust a shared property as `door_dir`
    // — matches `ParseNames::detect`'s own confidence bar in spirit (refuse
    // rather than guess from too small a sample).
    if found.len() < 6 {
        return none;
    }

    // `door_dir`: the property every found compass object carries whose
    // values, across them, are distinct small numbers (each direction's own
    // `*_to`). Scanning 1..=63 rather than assuming any particular number —
    // Lost Pig's is 34, nothing like `linklpa.h`'s own declaration order (see
    // module docs above).
    let mut door_dir_prop = None;
    'search: for prop in 1u8..=63 {
        let mut vals: Vec<u16> = Vec::with_capacity(found.len());
        for &(_, id) in &found {
            if crate::objects::get_prop_addr(mem, id, prop) == 0 {
                continue 'search;
            }
            vals.push(crate::objects::get_prop(mem, id, prop));
        }
        let mut sorted = vals.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() == vals.len() && vals.iter().all(|&v| v > 0 && v < 64) {
            door_dir_prop = Some(prop);
            break;
        }
    }
    let Some(door_dir_prop) = door_dir_prop else { return none };

    // `exit_props[dir]` is exactly the door_dir VALUE the matching compass
    // object carries — `n_obj.door_dir == n_to`'s property number.
    let mut exit_props = [None; 12];
    let mut used_props: Vec<u8> = Vec::with_capacity(12);
    for &(dir, id) in &found {
        let p = crate::objects::get_prop(mem, id, door_dir_prop);
        if p > 0 && p < 64 {
            exit_props[dir as usize] = Some(p as u8);
            used_props.push(p as u8);
        }
    }
    // Up/Down/In/Out: accepted only when the object `ParseNames::find` turns up
    // for the word ALSO carries `door_dir` with a value that is a plausible,
    // still-unused `*_to` property number — the check a false hit like
    // Curses' "ship in a bottle" fails, since nothing gave it one.
    for dir in [Compass::Up, Compass::Down, Compass::In, Compass::Out] {
        let Some(o) = pn.find(mem, dir.word()) else { continue };
        let id = o.id as u16;
        if crate::objects::get_prop_addr(mem, id, door_dir_prop) == 0 {
            continue;
        }
        let p = crate::objects::get_prop(mem, id, door_dir_prop);
        if p > 0 && p < 64 && !used_props.contains(&(p as u8)) {
            primary_ids[dir as usize] = Some(id);
            exit_props[dir as usize] = Some(p as u8);
            used_props.push(p as u8);
        }
    }
    let compass_ids = primary_ids;

    // `door_to`: found by cross-checking real CONNECTOR objects — anything
    // (other than a compass object) that also carries `door_dir`, which is
    // exactly the "tunnel to east"-style object `GoSub` takes a `door_to` hop
    // through. Capped, since a large Inform 7 table can hold tens of
    // thousands of objects and only a handful of agreeing doors are needed.
    const MAX_CONNECTORS: usize = 24;
    let compass_id_set: Vec<u16> = compass_ids.iter().filter_map(|&o| o).collect();
    let mut connectors: Vec<u16> = Vec::new();
    for obj in 1..=max_object {
        if compass_id_set.contains(&obj) {
            continue;
        }
        if crate::objects::get_prop_addr(mem, obj, door_dir_prop) != 0 {
            connectors.push(obj);
            if connectors.len() >= MAX_CONNECTORS {
                break;
            }
        }
    }
    let door_to_prop = infer_door_to(mem, &connectors, &compass_id_set, door_dir_prop, max_object);

    (exit_props, Some(door_dir_prop), door_to_prop)
}

/// The `door_to` property number: the one present on most sampled CONNECTORS
/// whose value, where it names a room at all, is a plausible and DISTINCT
/// (not the same on every connector) TERMINAL — not another connector.
///
/// A one-way or code-computed door is real and common (Lost Pig's own "broken
/// stair" and "windy tunnel" objects both hold a ROUTINE in this property,
/// not a room — the north exit past the statue is exactly the code-decided
/// case this whole feature exists to notice), so a candidate is NOT thrown out
/// just because some connector's value is not a plain room number; only a
/// value that looks wrong in a way `door_to` never should is disqualifying:
///
/// * **pointing at itself or at a compass object** — never a real destination;
/// * **pointing at another connector** — `GoSub` takes `door_to` exactly one
///   hop (`verblib.h`:2093-2096) and never chases a second door from there, so
///   a `door_to` that resolves to something ITSELF carrying `door_dir` is not
///   the property this derivation is looking for.
///
/// What must still vary is the survivors: Lost Pig's "tunnel to east" and
/// "tunnel to west" both carry an unrelated property holding the same
/// constant 31 on both (evidently some other shared convention, not a
/// per-door destination) beside `door_to` itself holding 166 and 102
/// respectively — the two different rooms they actually lead to. Requiring at
/// least two DISTINCT room-like values among the survivors is what throws out
/// the constant and keeps the one that behaves like a destination.
///
/// `door_dir` itself is excluded up front: every connector also carries it,
/// and its own value there (the connector's own `*_to` property number, e.g.
/// 22 for an east-facing door) is small, in-range and genuinely different
/// between an east door and a west door — distinct for the wrong reason, and
/// would otherwise be picked first since it is scanned in the same 1..=63
/// sweep as every real candidate.
fn infer_door_to(
    mem: &Memory,
    connectors: &[u16],
    compass_ids: &[u16],
    door_dir_prop: u8,
    max_object: u16,
) -> Option<u8> {
    if connectors.len() < 2 {
        return None;
    }
    let min_present = (connectors.len() / 2).max(2);
    'search: for prop in 1u8..=63 {
        if prop == door_dir_prop {
            continue;
        }
        let mut present = 0usize;
        let mut room_like: Vec<u16> = Vec::new();
        for &c in connectors {
            if crate::objects::get_prop_addr(mem, c, prop) == 0 {
                continue; // absent on this one connector — does not disqualify the property
            }
            present += 1;
            let v = crate::objects::get_prop(mem, c, prop);
            if v == 0 || v > max_object {
                continue; // no exit, or a routine/string-valued door — a real possibility, not a disqualifier
            }
            if v == c || compass_ids.contains(&v) || crate::objects::get_prop_addr(mem, v, door_dir_prop) != 0
            {
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

// ── Declared exits: ZIL (SQ-1260) ────────────────────────────────────────────
//
// `infer_exits` above finds Inform's `door_dir` convention and nothing else —
// every ZIL (Infocom) story answers `Unknown` for every direction, which is
// why the Carousel Room in Zork II sends the player out at random with no
// warning at all (SQ-1257's Phase 1/Phase 2 protections never fire without a
// convention to read). This section is the second derivation, sharing the
// same [`DeclaredExit`] seam.
//
// ── The ZIL exit shapes, from the source ─────────────────────────────────────
//
// Infocom's own room-exit syntax — never called by these names in a `<ROOM
// ...>` form (that's just `(NORTH TO KITCHEN)`, `(EAST "message")`, …) but
// named this way by the compiler's own documentation for the five shapes it
// accepts — comes from **"Learning ZIL"** (Steve Meretzky, Infocom 1989;
// Microsoft Word conversion 1995), §2.2 "Exits":
// <https://eblong.com/infocom/other/Learning_ZIL_Meretzky_1995.pdf>
//
//   * **UEXIT** ("unconditional exit"): `(DIR TO ROOM)` — always leads there.
//   * **CEXIT** ("conditional exit"): `(DIR TO ROOM IF GLOBAL [ELSE "string"])`
//     — leads there when the named global is true, else prints the string (or
//     a compiler-supplied default when the string is omitted).
//   * **FEXIT** ("function exit"): `(DIR PER ROUTINE)` — the routine decides,
//     at run time, whether and where the player moves.
//   * **NEXIT** ("non-exit"): `(DIR SORRY "string")` or bare `(DIR "string")`
//     — never a passage, just a refusal nicer than the parser's own default.
//   * **DEXIT** ("door exit"): `(DIR TO ROOM IF DOOR IS OPEN [ELSE "string"])`
//     — a CEXIT whose condition is a door object's openness instead of a
//     global.
//
// ── The dictionary side: how a direction WORD is found at all ───────────────
//
// Z-Machine Standards §13 specifies the dictionary's key/flags/data LAYOUT and
// nothing about what any flag bit MEANS (exactly the gap `grammar.rs`'s module
// docs describe for the rest of the dictionary). The bit that marks a
// direction word, and the two-bit field that says which of a word's two data
// bytes holds which datum when more than one part-of-speech applies, are
// documented — and cited by `grammar.rs`'s own — in **ztools**' `tx.h` (Mark
// Howell; the reference disassembler `txd`/`infodump` read exactly this table
// to print a story's grammar): <https://github.com/ecliptik/ztools/blob/master/tx.h>
//
// ```text
// #define DIR          0x10   /* infocom V1-5 only */
// #define DATA_FIRST   0x03   /* infocom V1-5 only */
// #define DIR_FIRST    0x03   /* infocom V1-5 only */
// ```
//
// `grammar.rs` already reads this same dictionary entry shape (flags byte at
// `entry + key_len`, two data bytes following) for verb/noun/adjective/
// preposition detection, but keeps its OWN copy of the bit constants private
// to that module — `F_INFOCOM_SPECIAL` there is `SPECIAL` ($04, "buzzword"),
// a DIFFERENT bit from `DIR` ($10, "direction") despite `grammar.rs`'s module
// comment grouping them as "special (buzzword/direction)"; the two never
// coincide on one word in practice (measured on both `minizork.z3` and
// `zork1-r88-s840726.z3`: the buzzword "no" carries flags `$04` — `SPECIAL`
// with no other part-of-speech bit set; "north" carries `$13` — `DIR` plus
// its own `DATA_FIRST` field, never `SPECIAL`) and this module reads `DIR`
// directly, under its own name, for exactly that reason — see the constants
// just below `infer_zil_exits`.
//
// When `DIR` is set alongside another part-of-speech bit (up/down/in/out are
// also PREP-flagged — Zork's parser accepts "the trap door" but the bare word
// "in" doubles as a preposition), `DATA_FIRST` says which of the word's two
// data bytes is the direction's: `DIR_FIRST` ($03) means the FIRST data byte
// is the direction datum, any other value means the SECOND is. Measured
// against both fixtures: a lone-`DIR` word (north/south/east/…) always
// carries `DATA_FIRST == DIR_FIRST` and its datum in the first data byte;
// up/down/in/out carry a preposition datum first and their direction datum
// second.
//
// ── What the datum IS: the exit property number, directly ───────────────────
//
// Unlike Inform's `door_dir` indirection (a property NUMBER stored in a
// compass OBJECT's own property table, itself found by voting across the
// object table), ZIL's compiler stamps the exit property number straight
// into the DICTIONARY WORD'S data byte — one step, no object lookup. Verified
// by cross-referencing three independent sources for the same rooms:
//
//   * `stories/zork1-r88-s840726.z3`'s West of House (object #180) carries
//     properties 24/25/27/28/29/30/31 whose VALUES are the object numbers of
//     Stone Barrow (#178), South of House (#80) and North of House (#81) —
//     and property 31 (north's datum, read off the dictionary) is exactly the
//     property West of House uses for its north exit. The real ZIL source,
//     `1dungeon.zil` (<https://github.com/historicalsource/zork1>, the
//     retail game's own disassembled/recovered sources), declares
//     `(NORTH TO NORTH-OF-HOUSE) (SOUTH TO SOUTH-OF-HOUSE) (NE TO
//     NORTH-OF-HOUSE) (SE TO SOUTH-OF-HOUSE) (WEST TO FOREST-1) (EAST
//     "The door is boarded…") (SW TO STONE-BARROW IF WON-FLAG) (IN TO
//     STONE-BARROW IF WON-FLAG)` for this exact room — matching every one of
//     the seven properties' shapes below, direction for direction.
//   * The tracked fixture `minizork.z3`'s Kitchen (object #18) and the real
//     `zork1-r88-s840726.z3`'s Kitchen (object #203) both carry IDENTICAL
//     raw bytes on their EAST and OUT properties — matching `1dungeon.zil`'s
//     `(EAST TO EAST-OF-HOUSE IF KITCHEN-WINDOW IS OPEN) (OUT TO
//     EAST-OF-HOUSE IF KITCHEN-WINDOW IS OPEN)`, two identically-worded
//     DEXITs compiling to identical bytes.
//   * Zork I's Living Room (object #193) west exit is a CEXIT matching
//     `(WEST TO STRANGE-PASSAGE IF CYCLOPS-FLED ELSE "The wooden door is
//     nailed shut.")`, and its down exit — `(DOWN PER TRAP-DOOR-EXIT)` — is
//     the one FEXIT this derivation was checked against.
//
// ── The byte shapes themselves, and why LENGTH alone tells them apart ───────
//
// No explicit type tag is stored anywhere in the property — the FIVE shapes
// were found, empirically, to compile to five DISTINCT property lengths on
// every Version-3 room checked (`get_prop_len` on the exit property):
//
//   len 1  UEXIT   `[room:1]`                          — the room number, alone
//   len 2  NEXIT   `[string:2]`                         — packed refusal message
//   len 3  FEXIT   `[routine:2][pad:1]`                 — packed routine address
//   len 4  CEXIT   `[room:1][global:1][string:2]`       — `string` is 0 when no ELSE
//   len 5  DEXIT   `[room:1][door:1][string:2][pad:1]`  — `string` is 0 when no ELSE
//
// ── V4+ (SQ-1268): the room-reference width is a per-STORY fact, not a
//    per-VERSION one ─────────────────────────────────────────────────────────
//
// SQ-1260 refused every V4+ story outright, on the theory that object
// references being TWO bytes there (ZMSD §12.3 vs V3's one) would make
// UEXIT's length collide with NEXIT's (assumed fixed at a 2-byte packed
// address). That theory turned out to be wrong in the useful direction:
// checked against `stories/trinity-r12-s860926.z4`'s Palace Gate (object
// #236) and Bluff (#213) — cross-referenced byte-for-byte against the real
// ZIL source, `places.zil`
// (<https://github.com/historicalsource/trinity>) — EVERY V4+ shape is
// exactly ONE BYTE WIDER than its V3 counterpart, because the packed
// string/routine address fields scale too, not just the room reference:
//
//   len 2  UEXIT   `[room:2]`                           — Palace Gate NORTH → Broad Walk (#354),
//                                                          NE → The Wabe (#79), matching `(NORTH TO
//                                                          BROAD-WALK) (NE TO WABE)` exactly
//   len 3  NEXIT   `[string:2][pad:1]`                  — Bluff SOUTH, matching `(SOUTH SORRY "A
//                                                          sudden cliff blocks your path.")`
//   len 4  FEXIT   `[routine:2][pad:2]`                 — Bluff NORTH/NE/WEST/NW, all identical
//                                                          bytes, matching `(NORTH PER YOUD-FALL)`
//                                                          etc. (four directions, one shared routine)
//   len 5  CEXIT   `[room:2][global:1][string:2]`       — EXTRAPOLATED, not independently
//                                                          confirmed: no plain global-gated exit
//                                                          (as opposed to a door-gated one) was found
//                                                          in Trinity's, AMFV's or Bureaucracy's own
//                                                          `places.zil`/`apartment.zil`/`prism.zil` —
//                                                          every conditional exit in the three V4
//                                                          fixtures checked is a DEXIT. Follows the
//                                                          V3 table's own CEXIT/DEXIT spacing (no pad
//                                                          on CEXIT, one pad byte on DEXIT) by analogy.
//   len 6  DEXIT   `[room:2][door:2][string:2]`         — Bluff EAST/IN, matching `(EAST TO
//                                                          IN-COTTAGE IF COTTAGE-DOOR IS OPEN) (IN TO
//                                                          IN-COTTAGE IF COTTAGE-DOOR IS OPEN)`
//                                                          (`room`=0x0147, `door`=0x0013)
//
// So the collision SQ-1260 worried about does not happen: NEXIT is 3 bytes
// here, not a fixed 2, so it never lands on UEXIT's length. This shape — one
// byte wider than V3 at every step — is what `AMFV-r77-s850814.z4` and
// `bureaucracy-r116-s870602.z4` (both V4) and `beyondzork-r57-s871221.z5`
// (V5) all compile to as well.
//
// `stories/sherlock-r26-s880127.z5` does NOT: it is V5, but its compiler
// packed room references into a SINGLE byte, exactly like V3, throughout its
// exit tables — checked against 221-B Baker Street (object #38): NORTH (len
// 1, `[0x47]`=71="York Place") and SOUTH (len 1, `[0x3d]`=61="Orchard
// Street") are both one-byte UEXITs, and WEST/IN (len 3, matching V3's own
// FEXIT length) are FEXITs sharing one routine — plausibly `WHICH-WAY-IN`,
// the entry-hall puzzle the game's own dictionary word list suggests, though
// Sherlock's ZIL source is not in `historicalsource` to confirm by name.
// Sherlock's DICTIONARY entries are narrower too (`entry_length` 8, one data
// byte after the flags byte, vs 9/two bytes for the wide stories) — the two
// facts likely share one cause (a compiler mode that shrinks BOTH the
// dictionary and the exit tables when the story has few enough
// objects/rooms to fit a byte), but this derivation does not assume that:
// [`infer_zil_room_width`] measures the room width directly, off the exit
// tables themselves, never off the dictionary's own width.
//
// V6 (Zork Zero, Shogun, Arthur) is narrower again, but for a DIFFERENT
// reason than Sherlock: there is no `DIR` FLAG at all to test. ztools'
// `showdict.c` (`show_dictionary`) explicitly skips flag decoding for
// Version 6 (`else if (header.version != V6)`) — V6's dictionary entries use
// a different scheme (`tx.h`'s `parser_types` enum lists `infocom6_grammar`
// as its own case, distinct from `infocom_fixed`/`infocom_variable`).
// Empirically, the FIRST data byte of a V6 direction word's entry stores the
// exit-property number DIRECTLY, with no flag test or `DATA_FIRST`
// indirection: `stories/zork0-r393-s890714.z6`'s `north` entry is `3f 00 0e`
// and reads straight as property 63 — checked against Banquet Hall (object
// #7)'s own compiled properties, which are one-byte UEXITs matching
// `prologue.zil`'s `(WEST TO ENTRANCE-HALL) (SOUTH TO COURTYARD) (EAST TO
// KITCHEN)` (<https://github.com/historicalsource/zorkzero>) exactly, by
// object number AND by name (#56 "Entrance Hall", #8 "Courtyard", #59
// "Kitchen"). `stories/shogun-r322-s890706.z6`'s `ON-BRIDGE` room checks the
// same way against `osaka.zil`'s `(NORTH TO GATEWAY) (SOUTH TO
// AT-PORTCULLIS)` (<https://github.com/historicalsource/shogun>). A word
// whose first data byte is NOT a plausible property number (`> 63`, ZMSD
// §12.4.1's 6-bit V4+ property field) is not being used as a direction here
// — Zork Zero's, Shogun's and Arthur's own NE/NW/SE/SW dictionary entries all
// fail this test (their first byte is well over 127, some other word class
// entirely), matching that none of the three implements diagonal movement
// this way. `stories/journey-r83-s890706.z6`'s entire dictionary (27 entries)
// carries none of the twelve compass words at all — "no compass parser" is a
// fact about the dictionary itself, so this derivation naturally answers
// `None` for it (too few `DIR`-shaped words found) without needing to
// special-case the game by name.
//
// ── Classification into the shared seam ──────────────────────────────────────
//
// UEXIT and DEXIT both resolve to `DeclaredExit::Room` — DEXIT's destination
// is a plain, static room number in every case checked, never a routine
// (contrast Inform's `door_to`, where that same slot CAN hold one); whether a
// door lets the move actually happen this turn is a separate question
// `resolve_zil`'s doc comment addresses directly. CEXIT and FEXIT both
// resolve to `Code`, matching the task's classification for "the game decides
// at run time" exits. NEXIT resolves to `DeclaredExit::Message` — a real
// passage never exists there in any state the game can be in, which is a
// stronger claim than `Code` (unresolvable, but maybe real) and is exactly
// what `Message`'s own doc comment describes; using it here is what keeps
// SQ-1257 Phase 2 from wasting a probe on a direction that can never lead
// anywhere.

/// Infocom V1–5 dictionary flag: the word is a compass direction (ztools
/// `tx.h`'s `DIR`, distinct from `SPECIAL`/"buzzword" — see the module docs
/// above for why `grammar.rs`'s own, private, copy of these bits groups the
/// two under one doc comment despite them being different bits).
const F_ZIL_DIR: u8 = 0x10;
/// Infocom V1–5: which of a word's two data bytes holds which class's datum,
/// when more than one applies (ztools `tx.h`'s `DATA_FIRST`, a 2-bit field).
const F_ZIL_DATA_FIRST_MASK: u8 = 0x03;
/// `DATA_FIRST` value meaning the DIRECTION datum is the first data byte
/// (ztools `tx.h`'s `DIR_FIRST`); any other value means it is the second.
const F_ZIL_DIR_FIRST: u8 = 0x03;

/// How many of the twelve compass words must carry the `DIR` flag before this
/// derivation trusts the story as ZIL-shaped at all — the same confidence bar
/// in spirit as `infer_exits`' `found.len() < 6` (refuse rather than guess
/// from too small a sample), applied to all twelve directions rather than the
/// eight primary ones since `DIR` is a dedicated bit with no Inform-style
/// vocabulary-collision risk to work around.
const MIN_ZIL_DIRECTION_WORDS: usize = 6;

/// Derive the twelve ZIL exit-property numbers straight from the story's own
/// dictionary (SQ-1260, widened to V4+ by SQ-1268) — see the "Declared
/// exits: ZIL" module docs above for the citations and the byte layouts.
/// `None` for a story with too few direction-shaped compass words to trust
/// (Inform stories, Scott Adams, Glulx-shaped tables, and any V1/V2 ZIL story
/// alike — this has never been checked below V3).
fn infer_zil_exits(mem: &Memory, max_object: u16) -> Option<[Option<u8>; 12]> {
    match mem.version() {
        3..=5 => infer_zil_exits_flagged(mem, max_object),
        6 => infer_zil_exits_v6(mem, max_object),
        _ => None,
    }
}

/// V1–5's `DIR`-flag scheme (SQ-1260's original, widened to V4/V5 by
/// SQ-1268): a direction word's dictionary entry carries the `DIR` flag, and
/// `DATA_FIRST` says which of its data bytes holds the exit-property number.
/// V3's dictionary key is 4 bytes (6 Z-characters, ZMSD §13.2) with a 5-bit
/// property field (1..=31, §12.4.1); V4/V5's key is 6 bytes (9 Z-characters,
/// §13.3/§13.4) with a 6-bit property field (1..=63) — verified against
/// Trinity, AMFV, Bureaucracy (V4) and Beyond Zork, Sherlock (V5): every
/// `north`/`south`/… entry in all five carries `DIR` ($10) with the same
/// `DATA_FIRST` semantics ztools' `tx.h` documents, just wider. Sherlock's
/// dictionary entries have only ONE data byte after the flags byte (not two,
/// like the other four) — `d1` reads 0 rather than spilling into the next
/// entry when `entry_length` is too short to hold it.
fn infer_zil_exits_flagged(mem: &Memory, max_object: u16) -> Option<[Option<u8>; 12]> {
    if max_object == 0 {
        return None;
    }
    let dict = crate::dictionary::load(mem);
    if dict.count == 0 || dict.entry_length == 0 {
        return None;
    }
    let key_len = dict.key_len() as u32;
    let entry_len = dict.entry_length as u32;
    // Dictionary keys are truncated to 6 Z-characters in v1-3 and 9 in v4+
    // (ZMSD §13.2/§13.3) — a word longer than that ("northeast") is matched
    // by its own truncation, exactly what the compiler itself truncated to.
    let trunc = if mem.version() <= 3 { 6 } else { 9 };
    // Valid property numbers are 1..=31 in V3 (5-bit field) and 1..=63 in
    // V4+ (6-bit field, ZMSD §12.4.1) — anything else is not one to trust.
    let max_prop: u8 = if mem.version() <= 3 { 31 } else { 63 };

    let mut props: [Option<u8>; 12] = [None; 12];
    let mut found = 0usize;
    for dir in Compass::ALL {
        let key: String = dir.word().chars().take(trunc).collect();
        for i in 0..dict.count as u32 {
            let entry = dict.base + i * entry_len;
            if (entry + entry_len) as usize > mem.len() {
                break;
            }
            let (text, _) = crate::text::decode_string(mem, entry);
            if text.trim().to_lowercase() != key {
                continue;
            }
            if entry_len < key_len + 1 {
                break; // no data byte at all — nothing to read
            }
            let flags = mem.read_byte(entry + key_len);
            if flags & F_ZIL_DIR != 0 {
                let d0 = if entry_len >= key_len + 2 {
                    mem.read_byte(entry + key_len + 1)
                } else {
                    0
                };
                let d1 = if entry_len >= key_len + 3 {
                    mem.read_byte(entry + key_len + 2)
                } else {
                    0
                };
                let datum =
                    if flags & F_ZIL_DATA_FIRST_MASK == F_ZIL_DIR_FIRST { d0 } else { d1 };
                if datum > 0 && datum <= max_prop {
                    props[dir as usize] = Some(datum);
                    found += 1;
                }
            }
            break;
        }
    }
    if found < MIN_ZIL_DIRECTION_WORDS {
        return None;
    }
    Some(props)
}

/// V6's dictionary has no `DIR` flag to test at all (ztools' `showdict.c`
/// skips flag decoding for Version 6 outright — see the module docs above)
/// — a direction word's exit-property number is instead the dictionary
/// entry's FIRST data byte, directly, no flag or `DATA_FIRST` indirection.
/// Verified against Zork Zero's Banquet Hall and Shogun's `ON-BRIDGE`, both
/// cross-checked against their real ZIL source — see the module docs. A word
/// whose first data byte is not a plausible property number (`1..=63`) is
/// not being used as a direction in this story.
fn infer_zil_exits_v6(mem: &Memory, max_object: u16) -> Option<[Option<u8>; 12]> {
    if max_object == 0 {
        return None;
    }
    let dict = crate::dictionary::load(mem);
    if dict.count == 0 || dict.entry_length == 0 {
        return None;
    }
    let key_len = dict.key_len() as u32;
    let entry_len = dict.entry_length as u32;
    if entry_len < key_len + 1 {
        return None;
    }

    let mut props: [Option<u8>; 12] = [None; 12];
    let mut found = 0usize;
    for dir in Compass::ALL {
        // V6 dictionary keys are 6 bytes (9 Z-characters), same as V4/V5.
        let key: String = dir.word().chars().take(9).collect();
        for i in 0..dict.count as u32 {
            let entry = dict.base + i * entry_len;
            if (entry + entry_len) as usize > mem.len() {
                break;
            }
            let (text, _) = crate::text::decode_string(mem, entry);
            if text.trim().to_lowercase() != key {
                continue;
            }
            let datum = mem.read_byte(entry + key_len);
            if datum > 0 && datum <= 63 {
                props[dir as usize] = Some(datum);
                found += 1;
            }
            break;
        }
    }
    if found < MIN_ZIL_DIRECTION_WORDS {
        return None;
    }
    Some(props)
}

/// Per-story ZIL UEXIT/DEXIT room-reference width (SQ-1268): 1 byte or 2 —
/// see the module docs' "V4+" section for why this cannot be assumed from
/// the Z-machine version alone (Sherlock is V5 but narrow; Trinity is V4 but
/// wide). Derived empirically off the exit tables themselves: for every room
/// in the object table and every one of `zil_exit_props`' twelve properties,
/// a length-1 property whose single byte is a plausible object number casts
/// a "narrow" vote, and a length-2 property whose big-endian word is a
/// plausible object number casts a "wide" vote — UEXIT is the only shape
/// either width produces at that length (see the byte-length tables above),
/// so whichever width racks up more votes is the one this story's own
/// compiler chose. Defaults to 2 (the more common case measured, and the
/// only shape SQ-1260's original V3-only code never had to ask about) when
/// neither width finds any evidence at all. Only called when V4+ found a ZIL
/// convention at all (`zil_exit_props` has at least six `Some` entries).
fn infer_zil_room_width(mem: &Memory, max_object: u16, zil_exit_props: &[Option<u8>; 12]) -> u8 {
    let props: Vec<u8> = zil_exit_props.iter().filter_map(|p| *p).collect();
    if props.is_empty() {
        return 2;
    }
    let mut narrow_votes = 0u32;
    let mut wide_votes = 0u32;
    for obj in 1..=max_object {
        for &prop in &props {
            let addr = crate::objects::get_prop_addr(mem, obj, prop);
            if addr == 0 {
                continue;
            }
            match crate::objects::get_prop_len(mem, addr) {
                1 => {
                    let b = mem.read_byte(addr as u32) as u16;
                    if b != 0 && b <= max_object {
                        narrow_votes += 1;
                    }
                }
                2 => {
                    let w = mem.read_word(addr as u32);
                    if w != 0 && w <= max_object {
                        wide_votes += 1;
                    }
                }
                _ => {}
            }
        }
    }
    if narrow_votes > wide_votes { 1 } else { 2 }
}

// ── Attribute inference ──────────────────────────────────────────────────────

fn attr_count(mem: &Memory) -> u8 {
    if mem.version() <= 3 { 32 } else { 48 }
}

/// A parser dummy: 90% or more of every attribute set at once.
fn is_wildcard(mem: &Memory, obj: u16, nattr: u8) -> bool {
    let set = (0..nattr).filter(|&a| get_attr(mem, obj, a)).count();
    set * 10 >= nattr as usize * 9
}

/// The object number that is the parent of the most objects — the "rooms"
/// bucket in ZIL, and 0 (top level) in Inform.
fn modal_parent(mem: &Memory, real: &[u16]) -> u16 {
    let mut best = (0u16, 0usize);
    let mut counts: Vec<(u16, usize)> = Vec::new();
    for &o in real {
        let p = get_parent(mem, o);
        match counts.iter_mut().find(|(q, _)| *q == p) {
            Some((_, n)) => *n += 1,
            None => counts.push((p, 1)),
        }
    }
    for (p, n) in counts {
        if n > best.1 {
            best = (p, n);
        }
    }
    best.0
}

/// The container bit: the attribute set on the largest share of the objects
/// that are currently holding something. Requires a majority — a story whose
/// holders have no attribute in common has no container convention we can read,
/// and everything downstream then stays `None`.
fn infer_container_attr(sets: &[Vec<u16>], holders: &[u16]) -> Option<u8> {
    if holders.len() < 4 {
        return None;
    }
    let mut best: Option<(u8, usize)> = None;
    for (a, set) in sets.iter().enumerate() {
        let n = holders.iter().filter(|h| set.binary_search(h).is_ok()).count();
        if best.is_none_or(|(_, b)| n > b) {
            best = Some((a as u8, n));
        }
    }
    match best {
        Some((a, n)) if n * 2 >= holders.len() => Some(a),
        _ => None,
    }
}

/// The openness bit — the attribute a story sets on a holder whose contents are
/// visible right now.
///
/// Five filters, calibrated against measured ground truth (Zork I r52: container
/// 34, openness 28; Mini-Zork r34: container 9, openness 10 — established by
/// diffing the whole attribute space across an `open mailbox` turn):
///
/// 1. **Container-shaped.** At least [`MIN_OPEN_CONTAINER_PCT`] of the objects
///    carrying the bit also carry the container bit. Drops the bits meaning
///    takeable, burnable, readable, weapon, edible, worn — all set on far more
///    non-containers than containers. (Beyond Zork's "worn" bit reaches exactly
///    50% and is refused here; trusting it would have spilled the pack.)
/// 2. **Not a second name for "container".** A bit whose holder set nearly
///    coincides with the container set is another containment *class* marker,
///    not a state — Mini-Zork's attribute 18 covers the sack, the mailbox and
///    the trophy case whether open or shut, and trusting it would spill the
///    contents of every closed container in the game.
/// 3. **A surface bit sits strictly inside it.** In ZIL a surface (table, altar,
///    pedestal) is marked open as well, precisely so the describer lists what is
///    on it — so the surface bit's holders are a proper subset of the openness
///    bit's holders. The nested bit must itself look like a containment bit
///    ([`MIN_SURFACE_SUBSET`] holders, [`MIN_SURFACE_CONTAINER_PCT`] of them
///    containers), which is what separates openness (a state some containers are
///    in) from flat classification bits that happen to overlap.
/// 4. **Not near-universal.** A bit on more than a quarter of the story's
///    objects is describing something else entirely.
/// 5. **Unique.** If two attributes both survive 1–4, the story has told us
///    nothing that distinguishes them and we decline to guess. This is the
///    filter that does the most work: in the stories where the inference is
///    right exactly one candidate survives, and in the stories where it would
///    have been wrong several do (Anchorhead 4, Enchanter 2, Sorcerer 2,
///    Sherlock 2 — every one of those returns `None` and nests nothing).
///
/// # Known failure mode
///
/// Filter 3 is a ZIL habit, not a law, so this finds an answer mainly in the
/// Infocom family and comes up empty on Inform stories — whose `open` and
/// `openable` are equally container-shaped and cannot be told apart from the
/// table alone. Empty is the correct outcome there: no nesting, direct children
/// only. The residual risk is a story that satisfies all five filters with an
/// `openable`-style bit, which would list the contents of closed containers;
/// that is why the acceptance tests pin the *negative* case (Zork I's closed
/// sack and closed mailbox stay shut) and not merely the positive one.
fn infer_open_attr(sets: &[Vec<u16>], cont: u8, total_objects: usize) -> Option<u8> {
    let cont_set = &sets[cont as usize];
    let container_pct = |set: &Vec<u16>| {
        let inside = set.iter().filter(|o| cont_set.binary_search(o).is_ok()).count();
        (inside, inside * 100 / set.len().max(1))
    };

    let mut survivors: Vec<u8> = Vec::new();
    for (ai, set) in sets.iter().enumerate() {
        let a = ai as u8;
        if a == cont || set.len() < 2 || set.len() * 4 > total_objects {
            continue;
        }
        // 1. container-shaped
        let (inside, pct) = container_pct(set);
        if pct < MIN_OPEN_CONTAINER_PCT {
            continue;
        }
        // 2. not a restatement of the container set (Jaccard > 0.6)
        let union = set.len() + cont_set.len() - inside;
        if union > 0 && inside * 5 > union * 3 {
            continue;
        }
        // 3. a surface-shaped bit nests strictly inside it
        let nested = sets.iter().enumerate().any(|(bi, b)| {
            bi as u8 != a
                && bi as u8 != cont
                && b.len() >= MIN_SURFACE_SUBSET
                && b.len() < set.len()
                && container_pct(b).1 >= MIN_SURFACE_CONTAINER_PCT
                && b.iter().all(|o| set.binary_search(o).is_ok())
        });
        if !nested {
            continue;
        }
        survivors.push(a);
    }
    // 5. unique or nothing
    match survivors.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// Minimum share of an openness candidate's holders that must also be
/// containers.
const MIN_OPEN_CONTAINER_PCT: usize = 65;
/// Minimum holders for the nested surface-shaped bit that justifies a candidate.
const MIN_SURFACE_SUBSET: usize = 3;
/// Minimum share of that nested bit's holders that must be containers.
const MIN_SURFACE_CONTAINER_PCT: usize = 75;

// ── Local-globals inference ──────────────────────────────────────────────────

/// Inform 6 stamps its own version as ASCII (`"6.15"`) into header bytes
/// $3C–$3F, which every Infocom-era ZIL story leaves zeroed. This is a compiler
/// convention rather than anything the Standards Document requires, so it is
/// used in one direction only: a positive match switches the local-globals walk
/// **off** (Inform has no such convention — it uses `found_in`). A story that
/// does not match is not thereby claimed to be ZIL; it just goes on to the
/// evidence-based vote below, which has its own confidence floor.
fn looks_inform_compiled(mem: &Memory) -> bool {
    let b: Vec<u8> = (0x3C..0x40).map(|a| mem.read_byte(a)).collect();
    b[0].is_ascii_digit() && b[1] == b'.' && b[2].is_ascii_digit() && b[3].is_ascii_digit()
}

/// Decode property `prop` of `obj` as a list of object numbers. Object numbers
/// are one byte in v1–v3 and two in v4+ (ZMSD §12.3), so the property's data
/// length must be a whole number of them. Returns empty for anything that does
/// not decode cleanly, including a list containing object 0.
fn object_list_prop(mem: &Memory, obj: u16, prop: u8, max_object: u16) -> Vec<u16> {
    let width: u32 = if mem.version() <= 3 { 1 } else { 2 };
    let addr = get_prop_addr(mem, obj, prop);
    if addr == 0 {
        return Vec::new();
    }
    let len = get_prop_len(mem, addr) as u32;
    if len < width || !len.is_multiple_of(width) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..(len / width) {
        let v = if width == 1 {
            mem.read_byte(addr as u32 + i) as u16
        } else {
            mem.read_word(addr as u32 + i * 2)
        };
        if v == 0 || v > max_object {
            return Vec::new();
        }
        out.push(v);
    }
    out
}

/// Longest shared-scenery list we will believe. ZIL rooms name a handful.
const MAX_GLOBALS_PER_ROOM: usize = 16;
/// A bucket smaller than this is a coincidence (Hitchhiker's has a handbag with
/// one item in it that otherwise fits the shape); bigger than this is not a
/// scenery bucket but a second room list (A Mind Forever Voyaging parks 178
/// rooms in one object).
const MIN_GLOBALS_CHILDREN: usize = 4;
const MAX_GLOBALS_CHILDREN: usize = 96;
/// How many rooms must agree before the property is believed.
const MIN_GLOBALS_VOTES: u32 = 4;

/// Find the (property, bucket) pair that carries shared scenery.
///
/// The vote is joint on purpose: neither half is recognisable alone. A property
/// number means nothing by itself, and the bucket object has no marker either —
/// but "a property, present on many rooms, whose every entry is an object
/// parented to one and the same non-room object" is a shape that essentially
/// only the ZIL local-globals convention produces. Exit properties, which also
/// hold object numbers, are excluded by it: a ZIL door exit is
/// `[destination-room, door, flag]`, and both the room entry (parented to the
/// rooms bucket) and the trailing zero break the pattern.
fn infer_local_globals(
    mem: &Memory,
    max_object: u16,
    room_holder: u16,
    real: &[u16],
) -> (Option<u8>, Option<u16>) {
    if looks_inform_compiled(mem) {
        return (None, None);
    }
    let rooms: Vec<u16> = real.iter().copied().filter(|&o| get_parent(mem, o) == room_holder).collect();
    if rooms.len() < MIN_GLOBALS_VOTES as usize {
        return (None, None);
    }

    let mut votes: Vec<((u8, u16), u32)> = Vec::new();
    for &r in &rooms {
        let mut p = get_next_prop(mem, r, 0);
        let mut guard = 0;
        while p != 0 && guard < 64 {
            guard += 1;
            if let Some(g) = common_scenery_parent(mem, r, p, max_object, room_holder) {
                match votes.iter_mut().find(|((q, h), _)| *q == p && *h == g) {
                    Some((_, n)) => *n += 1,
                    None => votes.push(((p, g), 1)),
                }
            }
            p = get_next_prop(mem, r, p);
        }
    }

    let Some(&((prop, holder), n)) = votes.iter().max_by_key(|(_, n)| *n) else {
        return (None, None);
    };
    if n < MIN_GLOBALS_VOTES {
        return (None, None);
    }

    // The bucket must sit OUTSIDE the played world — never in a room and never
    // inside something in a room. (It is not necessarily top-level: every
    // Infocom story measured parks its local-globals bucket inside the parser's
    // dummy pseudo-object — #50 in #65 for Zork I, #36 in #45 for Mini-Zork.)
    // This is what stops a real container that happens to fit the shape from
    // being mistaken for the bucket.
    if !is_out_of_world(mem, holder, room_holder) {
        return (None, None);
    }
    // …and it must hold a decent number of things that are themselves empty:
    // shared scenery is scenery, and scenery holds nothing. A bucket full of
    // objects that hold things is a second room list, not scenery (A Mind
    // Forever Voyaging parks 178 rooms in one object).
    let mut kids = 0usize;
    let mut childless = 0usize;
    let mut c = get_child(mem, holder);
    let mut guard = 0;
    while c != 0 && guard < MAX_SIBLINGS {
        guard += 1;
        kids += 1;
        if get_child(mem, c) == 0 {
            childless += 1;
        }
        c = get_sibling(mem, c);
    }
    if !(MIN_GLOBALS_CHILDREN..=MAX_GLOBALS_CHILDREN).contains(&kids) || childless * 4 < kids * 3 {
        return (None, None);
    }

    // A property that non-rooms carry in the same shape is not room-scoped
    // scenery — it is something else that happens to hold object numbers.
    let non_room_uses = real
        .iter()
        .filter(|&&o| get_parent(mem, o) != room_holder)
        .filter(|&&o| common_scenery_parent(mem, o, prop, max_object, room_holder) == Some(holder))
        .count();
    if non_room_uses as u32 * 2 > n {
        return (None, None);
    }

    (Some(prop), Some(holder))
}

/// True when `obj`'s ancestor chain terminates at the null object without ever
/// passing through a room. `room_holder == 0` means the story keeps rooms at
/// top level (Inform), where "outside the world" is not expressible — every
/// top-level object then qualifies, and the other guards carry the weight.
fn is_out_of_world(mem: &Memory, obj: u16, room_holder: u16) -> bool {
    let mut a = obj;
    for _ in 0..8 {
        if a == 0 {
            return true;
        }
        if room_holder != 0 && get_parent(mem, a) == room_holder {
            return false; // `a` is a room, so `obj` is somewhere in the world
        }
        a = get_parent(mem, a);
    }
    false // a cycle, or a chain too deep to be a pseudo-object
}

/// `Some(bucket)` when `prop` of `obj` decodes to a short, non-empty list of
/// objects that all share one parent, and that parent is neither the null
/// object nor the rooms bucket.
fn common_scenery_parent(
    mem: &Memory,
    obj: u16,
    prop: u8,
    max_object: u16,
    room_holder: u16,
) -> Option<u16> {
    let list = object_list_prop(mem, obj, prop, max_object);
    if list.is_empty() || list.len() > MAX_GLOBALS_PER_ROOM {
        return None;
    }
    let mut parent = None;
    for o in list {
        let p = get_parent(mem, o);
        if p == 0 || p == room_holder {
            return None;
        }
        match parent {
            None => parent = Some(p),
            Some(q) if q == p => {}
            _ => return None,
        }
    }
    parent
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::encode::encode_word;

    // A synthetic v3 story shaped like a ZIL game, so every inference below is
    // exercised against a table whose ground truth we WROTE rather than guessed.
    //
    //   #1  rooms bucket   ── #3..#8 the rooms
    //   #2  scenery bucket ── #9 window, #10 chimny, #11 forest, #12 stairs
    //   #3  kitchn (room)  ── #13 table ── #15 sack ── #17 lunch, #18 garlic
    //                                  └─ #16 bottle
    //                      └─ #14 player
    //   #4  behind (room)  ── #23 mailbx ── #24 leafle
    //
    // Attributes: 10 = container, 12 = open, 14 = surface. The sack, mailbox
    // and chest are containers WITHOUT the open bit — they are the leak the
    // walker must never spring.
    const CONT: u8 = 10;
    const OPEN: u8 = 12;
    const SURFACE: u8 = 14;

    const ROOMS: u16 = 1;
    const SCENERY: u16 = 2;
    const KITCHEN: u16 = 3;
    const BEHIND: u16 = 4;
    const WINDOW: u16 = 9;
    const TABLE: u16 = 13;
    const PLAYER: u16 = 14;
    const SACK: u16 = 15;
    const BOTTLE: u16 = 16;
    const LUNCH: u16 = 17;
    const GARLIC: u16 = 18;
    const MAILBOX: u16 = 23;
    const LEAFLET: u16 = 24;

    /// The room property that lists shared scenery, mirroring ZIL's `GLOBAL`.
    const GLOBAL_PROP: u8 = 20;

    struct Obj {
        name: &'static str,
        parent: u16,
        sibling: u16,
        child: u16,
        attrs: &'static [u8],
        globals: &'static [u8],
    }

    const fn o(
        name: &'static str,
        parent: u16,
        sibling: u16,
        child: u16,
        attrs: &'static [u8],
        globals: &'static [u8],
    ) -> Obj {
        Obj { name, parent, sibling, child, attrs, globals }
    }

    fn objects() -> Vec<Obj> {
        vec![
            o("rooms", 0, 0, KITCHEN, &[], &[]),
            o("lgbkt", 0, 0, WINDOW, &[], &[]),
            // rooms
            o("kitchn", ROOMS, BEHIND, TABLE, &[], &[9, 10, 12]),
            o("behind", ROOMS, 5, MAILBOX, &[], &[9, 11]),
            o("cellar", ROOMS, 6, 19, &[], &[10]),
            o("attic", ROOMS, 7, 20, &[], &[12]),
            o("hall", ROOMS, 8, 21, &[], &[11]),
            o("study", ROOMS, 0, 25, &[], &[9]),
            // shared scenery
            o("window", SCENERY, 10, 0, &[], &[]),
            o("chimny", SCENERY, 11, 0, &[], &[]),
            o("forest", SCENERY, 12, 0, &[], &[]),
            o("stairs", SCENERY, 0, 0, &[], &[]),
            // kitchen contents
            o("table", KITCHEN, PLAYER, SACK, &[CONT, OPEN, SURFACE], &[]),
            o("player", KITCHEN, 0, 0, &[], &[]),
            o("sack", TABLE, BOTTLE, LUNCH, &[CONT], &[]),
            o("bottle", TABLE, 0, 0, &[CONT], &[]),
            o("lunch", SACK, GARLIC, 0, &[], &[]),
            o("garlic", SACK, 0, 0, &[], &[]),
            // elsewhere
            o("altar", 5, 0, 0, &[CONT, OPEN, SURFACE], &[]),
            o("pedest", 6, 0, 0, &[CONT, OPEN, SURFACE], &[]),
            o("basket", 7, 0, 22, &[CONT, OPEN], &[]),
            o("pebble", 21, 0, 0, &[], &[]),
            o("mailbx", BEHIND, 0, LEAFLET, &[CONT], &[]),
            o("leafle", MAILBOX, 0, 0, &[], &[]),
            o("chest", 8, 0, 0, &[CONT], &[]),
        ]
    }

    const OBJ_TABLE: usize = 0x0100;
    const ENTRIES: usize = OBJ_TABLE + 31 * 2; // v3: 31 default words
    const PROPS: usize = 0x0400;

    fn build() -> Memory {
        let objs = objects();
        let mut buf = vec![0u8; 0x1000];
        buf[0x00] = 3;
        buf[0x04] = 0x08; // high memory
        buf[0x06] = 0x00;
        buf[0x07] = 0x40; // initial PC
        buf[0x08] = 0x08;
        buf[0x0A] = (OBJ_TABLE >> 8) as u8;
        buf[0x0B] = OBJ_TABLE as u8;
        buf[0x0C] = 0x03; // globals
        buf[0x0E] = 0x08; // static memory
        buf[0x40] = 0xba; // quit

        let mut prop_at = PROPS;
        for (i, ob) in objs.iter().enumerate() {
            let e = ENTRIES + i * 9;
            for &a in ob.attrs {
                buf[e + (a / 8) as usize] |= 1 << (7 - (a % 8));
            }
            buf[e + 4] = ob.parent as u8;
            buf[e + 5] = ob.sibling as u8;
            buf[e + 6] = ob.child as u8;
            buf[e + 7] = (prop_at >> 8) as u8;
            buf[e + 8] = prop_at as u8;

            // Property table: short name, then the GLOBAL property, then the
            // 0 terminator.
            let encoded = encode_word(ob.name, 3);
            buf[prop_at] = (encoded.len() / 2) as u8;
            buf[prop_at + 1..prop_at + 1 + encoded.len()].copy_from_slice(&encoded);
            let mut p = prop_at + 1 + encoded.len();
            if !ob.globals.is_empty() {
                // v3 property header: 32 * (len - 1) + number (ZMSD §12.4.1)
                buf[p] = 32 * (ob.globals.len() as u8 - 1) + GLOBAL_PROP;
                p += 1;
                buf[p..p + ob.globals.len()].copy_from_slice(ob.globals);
                p += ob.globals.len();
            }
            buf[p] = 0;
            prop_at = p + 1;
        }
        Memory::new(buf).expect("synthetic story")
    }

    fn names(mem: &Memory, ids: &[u16]) -> Vec<String> {
        ids.iter().map(|&o| crate::objects::short_name(mem, o)).collect()
    }

    /// SQ-1257: Mini-Zork is ZIL, not Inform — there is no `door_dir` convention in its table for
    /// [`infer_exits`] to find. Post-SQ-1260 that no longer means `declared_exit` answers
    /// `Unknown` everywhere (the ZIL convention below is found instead); this test now pins the
    /// negative Inform-side half only. The positive ZIL half is
    /// [`a_zil_storys_own_exit_convention_matches_the_real_geography`] just below.
    /// `minizork.z3` is a tracked fixture (`crates/zvm/tests/fixtures/`), so this never skips.
    #[test]
    fn a_zil_story_has_no_inform_door_dir_convention_to_find() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else { return };
        let mem = Memory::new(bytes).unwrap();
        let m = WorldModel::discover(&mem);
        assert_eq!(m.door_to_prop, None, "no door_dir convention means no door_to to cross-check either");
        assert!(m.exit_props.iter().all(Option::is_none), "no Inform `*_to` property numbers to find either");
    }

    /// SQ-1260: Mini-Zork's `<DIRECTIONS>` words carry the ZIL exit-property numbers directly
    /// (see the "Declared exits: ZIL" module docs above `infer_zil_exits`), so
    /// [`WorldModel::declared_exit`] now reads real UEXIT/DEXIT/NEXIT data off the Kitchen (object
    /// #18) instead of answering `Unknown`. Every destination below was independently checked
    /// against the real geography by NAME (`short_name`), not just by number:
    /// `crates/zvm/examples/check_zil_exits.rs` (a scratch tool used to derive this test, since
    /// deleted) printed #53 "Living Room", #125 "Attic" and #28 "Behind House" for exactly these
    /// object numbers. `minizork.z3` is a tracked fixture, so this never skips.
    #[test]
    fn a_zil_storys_own_exit_convention_matches_the_real_geography() {
        let Some(bytes) = crate::fixtures::load("minizork.z3") else { return };
        let mem = Memory::new(bytes).unwrap();
        let m = WorldModel::discover(&mem);
        assert!(
            m.zil_exit_props.iter().filter(|p| p.is_some()).count() >= 6,
            "at least six of the twelve compass words must carry the ZIL DIR flag"
        );

        const KITCHEN: u16 = 18;
        assert_eq!(m.declared_exit(&mem, KITCHEN, Compass::W), DeclaredExit::Room(53), "west: a UEXIT to the Living Room");
        assert_eq!(m.declared_exit(&mem, KITCHEN, Compass::Up), DeclaredExit::Room(125), "up: a UEXIT to the Attic");
        assert_eq!(
            m.declared_exit(&mem, KITCHEN, Compass::E),
            DeclaredExit::Room(28),
            "east: a DEXIT (the window) to Behind House"
        );
        assert_eq!(
            m.declared_exit(&mem, KITCHEN, Compass::Out),
            DeclaredExit::Room(28),
            "out: the SAME DEXIT as east, same destination — identical ZIL source compiles to identical bytes"
        );
        assert_eq!(
            m.declared_exit(&mem, KITCHEN, Compass::Down),
            DeclaredExit::Message,
            "down: a NEXIT — the cut-down demo drops Zork I's chimney puzzle to a plain refusal"
        );
        assert_eq!(
            m.declared_exit(&mem, KITCHEN, Compass::N),
            DeclaredExit::Absent,
            "north: the compass word is real (that's how its property number was found at all), \
             but this room declares nothing for it"
        );
    }

    #[test]
    fn the_builder_lays_the_table_out_as_intended() {
        // Guard: tells a builder bug apart from an inference bug below.
        let mem = build();
        assert_eq!(crate::objects::short_name(&mem, TABLE), "table");
        assert_eq!(get_parent(&mem, SACK), TABLE);
        assert_eq!(get_parent(&mem, LUNCH), SACK);
        assert!(get_attr(&mem, TABLE, OPEN), "the table is open");
        assert!(!get_attr(&mem, SACK, OPEN), "the sack is shut");
        assert_eq!(crate::location::max_object_number(&mem), objects().len() as u16);
    }

    #[test]
    fn discovery_recovers_the_conventions_the_story_was_written_with() {
        let m = WorldModel::discover(&build());
        assert_eq!(m.room_holder, ROOMS, "rooms hang off #1");
        assert_eq!(m.container_attr, Some(CONT));
        assert_eq!(m.open_attr, Some(OPEN), "openness must not be confused with the surface bit");
        assert_eq!(m.globals_prop, Some(GLOBAL_PROP));
        assert_eq!(m.globals_holder, Some(SCENERY));
    }

    /// The local-globals walk: a room's shared scenery is named by a property
    /// and lives in a bucket, so it is never reachable by walking children.
    #[test]
    fn local_globals_come_from_the_room_property_not_the_child_chain() {
        let mem = build();
        let m = WorldModel::discover(&mem);
        assert_eq!(names(&mem, &m.local_globals(&mem, KITCHEN)), ["window", "chimny", "stairs"]);
        assert_eq!(names(&mem, &m.local_globals(&mem, BEHIND)), ["window", "forest"]);
        // …and none of them is a child of the room.
        assert!(m.local_globals(&mem, BEHIND).iter().all(|&g| get_parent(&mem, g) != BEHIND));
    }

    /// The whole point: the sack and bottle ON the table are visible, the lunch
    /// and garlic IN the shut sack are not.
    #[test]
    fn the_here_list_nests_through_open_holders_and_stops_at_shut_ones() {
        let mem = build();
        let m = WorldModel::discover(&mem);
        let here = names(&mem, &m.visible_room_objects(&mem, KITCHEN, PLAYER));
        assert_eq!(here, ["table", "sack", "bottle", "window", "chimny", "stairs"]);
        assert!(!here.contains(&"lunch".to_string()), "a shut container must not leak: {here:?}");
        assert!(!here.contains(&"garlic".to_string()), "a shut container must not leak: {here:?}");
        assert!(!here.contains(&"player".to_string()), "the player is the CARRIED column");
    }

    /// SQ-1133: a holder is read by the same rule wherever it is standing. Put
    /// the sack in the player's hands and `visible_contents` answers exactly as
    /// `visible_room_objects` did with it on the table — nothing while it is
    /// shut, the lunch and the garlic once it is open.
    ///
    /// Falsify by walking the player's direct children instead: the second
    /// assertion loses both, which is the reported symptom.
    #[test]
    fn a_holder_in_the_players_hands_reads_like_one_on_the_table() {
        let mut mem = build();
        crate::objects::set_parent(&mut mem, SACK, PLAYER);
        crate::objects::set_child(&mut mem, PLAYER, SACK);
        crate::objects::set_sibling(&mut mem, SACK, 0);
        let m = WorldModel::discover(&mem);

        assert_eq!(
            names(&mem, &m.visible_contents(&mem, PLAYER, 0)),
            ["sack"],
            "a shut sack in hand is one word, not three"
        );

        crate::objects::set_attr(&mut mem, SACK, OPEN);
        assert_eq!(
            names(&mem, &m.visible_contents(&mem, PLAYER, 0)),
            ["sack", "lunch", "garlic"],
            "opened, in hand"
        );

        crate::objects::clear_attr(&mut mem, SACK, OPEN);
        assert_eq!(names(&mem, &m.visible_contents(&mem, PLAYER, 0)), ["sack"], "shut again");
    }

    /// `visible_contents` never runs the shared-scenery pass: a holder has no
    /// `GLOBAL` property, and reading one off whatever byte sits there would be
    /// a room's answer given to a rucksack.
    #[test]
    fn a_holder_gets_no_shared_scenery() {
        let mem = build();
        let m = WorldModel::discover(&mem);
        let here = names(&mem, &m.visible_room_objects(&mem, KITCHEN, PLAYER));
        assert!(here.contains(&"window".to_string()), "the ROOM sees its shared scenery");
        assert_eq!(
            names(&mem, &m.visible_contents(&mem, KITCHEN, PLAYER)),
            ["table", "sack", "bottle"],
            "the same walk without the globals pass"
        );
    }

    #[test]
    fn opening_a_container_reveals_it_and_closing_it_hides_it_again() {
        let mut mem = build();
        let m = WorldModel::discover(&mem);
        let here = |mem: &Memory| names(mem, &m.visible_room_objects(mem, BEHIND, PLAYER));
        assert_eq!(here(&mem), ["mailbx", "window", "forest"]);

        crate::objects::set_attr(&mut mem, MAILBOX, OPEN);
        assert_eq!(here(&mem), ["mailbx", "leafle", "window", "forest"], "opened, live");

        crate::objects::clear_attr(&mut mem, MAILBOX, OPEN);
        assert_eq!(here(&mem), ["mailbx", "window", "forest"], "shut again, live");
    }

    /// The fail-toward-less contract: with no openness bit identified, the walk
    /// degrades to direct children and cannot leak anything at all.
    #[test]
    fn an_unidentified_openness_bit_disables_nesting_entirely() {
        let mem = build();
        let mut m = WorldModel::discover(&mem);
        m.open_attr = None;
        let here = names(&mem, &m.visible_room_objects(&mem, KITCHEN, PLAYER));
        assert_eq!(here, ["table", "window", "chimny", "stairs"], "children + scenery only");
    }

    #[test]
    fn a_cycle_in_the_tree_cannot_hang_or_flood_the_walk() {
        let mut mem = build();
        // Point the sack's contents back at the table and prop both open.
        crate::objects::set_attr(&mut mem, SACK, OPEN);
        crate::objects::set_child(&mut mem, SACK, TABLE);
        let m = WorldModel::discover(&mem);
        let here = m.visible_room_objects(&mem, KITCHEN, PLAYER);
        assert!(here.len() <= MAX_ITEMS);
        let mut sorted = here.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), here.len(), "no object may be listed twice");
    }

    /// Inform-compiled stories have no local-globals convention; the header
    /// marker switches the walk off rather than letting it find a lookalike.
    #[test]
    fn an_inform_marker_switches_the_local_globals_walk_off() {
        let mem = build();
        assert!(!looks_inform_compiled(&mem));
        let mut buf = mem.raw_bytes().to_vec();
        buf[0x3C..0x40].copy_from_slice(b"6.15");
        let inform = Memory::new(buf).unwrap();
        assert!(looks_inform_compiled(&inform));
        let m = WorldModel::discover(&inform);
        assert_eq!(m.globals_prop, None, "no ZIL scenery walk on an Inform story");
        assert!(m.local_globals(&inform, KITCHEN).is_empty());
    }
}
