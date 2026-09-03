//! What a Glulx object is, and what it can be **called** — the dictionary words
//! that refer to it, not the text the game prints for it.
//!
//! ── Where the format is specified ────────────────────────────────────────────
//!
//! Objects are Inform's, not Glulx's: the Glulx specification describes a
//! virtual machine and knows nothing about them. Two authoritative sources,
//! both consulted directly:
//!
//!   * **"The Glulx Inform Technical Reference"**, Andrew Plotkin — §2 "Object
//!     Structure" and §3 "Property Tables".
//!     <https://eblong.com/zarf/glulx/Glulx-Inform-Tech.html>
//!   * **The Inform 6 compiler** — `tables.c::construct_storyfile_g` for the
//!     order the RAM tables are emitted in, and `objects.c` for the property
//!     numbering. <https://github.com/DavidKinder/Inform6>
//!
//! §2 gives the object structure, which is what every offset below comes from:
//!
//! ```text
//!   byte      $70 — the type identifier for an object
//!   byte[N]   attributes          (N = NUM_ATTR_BYTES, always of the form 4i+3)
//!   long      next object in the overall linked list
//!   long      hardware name string
//!   long      property table address
//!   long      parent / sibling / child
//! ```
//!
//! and §3 the property table: a long count, then ten-byte entries of
//! `{short id, short length in WORDS, long data address, short flags}`, sorted
//! by id, followed by the data itself.
//!
//! The property is number **1** on every Inform story, and this is hard-coded
//! in the compiler rather than conventional. `Inform6/src/objects.c`: "A
//! special rule applies to values in double-quotes of the built-in property
//! `name`, which always has number 1: such property values are dictionary
//! entries and not static strings", with `objects_begin_pass()` seeding
//! `commonprops[1]` before any user property is numbered. Inform 7 keeps it —
//! `inform7/runtime-module/Chapter 7/Name Properties.w`: "the names of objects
//! are parsed as nouns using the values of two properties: `name`, a simple
//! array of dictionary words, and `parse_name`, a GPR function".
//!
//! ── The hard part is finding the tree, exactly as with the grammar ───────────
//!
//! **A Glulx image records the object tree's address nowhere**, for the same
//! reason [`crate::grammar`] documents at length for the grammar table: the
//! header names RAMSTART, EXTSTART, ENDMEM, the start function and the decoding
//! table, and Inform's own 24-byte block after it holds a layout tag, two
//! version strings, a release and a serial — no table addresses at all
//! (`Inform6/src/files.c`, `GLULX_STATIC_ROM_SIZE`).
//!
//! So the tree is *derived*, from the signature §2 hands us. An object is a
//! `$70` byte whose next-link points at the byte immediately after it, which is
//! itself a `$70` byte, all the way to a `0` that ends the list. `NUM_ATTR_BYTES`
//! is not in the header either, but it is constrained to `4i+3` and it fixes
//! the stride, so the candidate strides are few and the walk either closes on
//! one of them or on none.
//!
//! A clean walk is not on its own proof — the walk starting at object *k* is
//! also clean, since the list from there is a list. The head is therefore the
//! **lowest** address whose walk closes, and the walk is then *verified*: a
//! real object tree's property 1 is an array of dictionary records, and a run
//! of bytes that merely looks like a linked list is not. That check is the one
//! that does the work, and it is strict — every entry of every array must land
//! on a record boundary of the dictionary [`crate::grammar::locate`] found, and
//! carry the `$60` tag Inform writes there.
//!
//! ── …and the tree comes free with it ────────────────────────────────────────
//!
//! Three of §2's six longs — `parent`, `sibling`, `child` — are Inform's
//! ordinary containment tree, the one `objectloop (x in y)` walks, and they are
//! readable the moment the stride is known. So a verified list is also a
//! verified TREE: what the player is carrying is the children of the avatar,
//! and what is in the room is the children of `location` (SQ-1241).
//!
//! Two things the image does not hand over, and how each is answered:
//!
//!   * **Which object is the avatar.** Not a global this reader can find, so
//!     [`ParseNames::find_player`] applies the rule
//!     `zvm::location::find_player_object` already applies on the other
//!     back-end — avatar-ish names, then *validated against the room*, because
//!     a name alone has picked the wrong object before.
//!   * **Which object is the room.** Nothing in the image says, and this reader
//!     does not guess: `find_player` takes the room as an argument. The app
//!     supplies it — from the `location` global it learns by watching which RAM
//!     word changes when the room does (`app::glulx_roomlock`, SQ-0526), or,
//!     before that resolves, from the top-level object whose
//!     [`short_name`](ParseNames::short_name) is the heading the story printed.
//!
//! Attributes are deliberately NOT read. Their numbering is the library's, not
//! the format's — it changes between Inform 6 library releases and again under
//! Inform 7 — so `container`/`open`/`transparent` cannot be identified from the
//! image, and a nested "what can you see inside that" walk would be guessing.
//! One level is the honest answer and is the one this module gives.
//!
//! ── The limitation, stated once ──────────────────────────────────────────────
//!
//! The `name` array is the complete set of **single** words that refer to an
//! object. Multi-word `Understand` phrases, conditional understandings and
//! visible-property adjectives are compiled by Inform 7 into a `parse_name`
//! routine — machine code, not a static array — and are not enumerable from the
//! image in any Inform version. `Chapter 7/Command Grammar Lines.w` says so
//! from the other side: single-fixed-word grammar lines are *moved out* of
//! `parse_name` and into the `name` array precisely because that array is where
//! single words belong.
//!
//! Inform 7 objects also routinely have an **empty** hardware name, because I7
//! prints objects through a rule rather than through the short name. There the
//! word list is the only text in the image that identifies the object at all.

use crate::grammar::{self, GrammarError, Tables};
use crate::memory::Memory;
/// The shared answer type, re-exported so `gvm::objects::ObjectWords` names it
/// — as `gvm::grammar` re-exports the rest of `grammar-model`.
pub use grammar_model::ObjectWords;

/// Inform's `name` property, on every Inform story and both back-ends.
pub const NAME_PROPERTY: u32 = 1;

/// A property table entry is `{short id, short length, long data, short flags}`.
const PROP_ENTRY_BYTES: u32 = 10;

/// Bytes of an object after `NUM_ATTR_BYTES`: six longs (next, name, property
/// table, parent, sibling, child). Plus the one `$70` tag byte before them.
const OBJECT_TAIL_BYTES: u32 = 6 * 4;

/// The six long fields that follow the attribute bytes, in the order §2 lists
/// them — `next`, `hardware name`, `property table`, `parent`, `sibling`,
/// `child`.
///
/// Named rather than spelled as offsets at each reader, because the ORDER is
/// the whole fact: three of the six are object addresses of identical shape, so
/// a transposed pair reads back a plausible tree rather than an error. Two
/// independent places in this crate already depend on it — [`crate::veneer`]'s
/// cross-check reads the class-chain at `13 + num_attr_bytes`, which is
/// [`Field::Parent`], and refuses a fingerprint when it does not name `Class`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Next = 0,
    Name = 1,
    Props = 2,
    Parent = 3,
    Sibling = 4,
    Child = 5,
}

/// Where field `f` of the object at `addr` lives, given the image's
/// `NUM_ATTR_BYTES`: past the `$70` tag, past the attributes, then four bytes
/// per preceding long.
fn field_at(addr: u32, attr_bytes: u32, f: Field) -> u32 {
    addr + 1 + attr_bytes + 4 * f as u32
}

/// `name`-array words that denote the player avatar.
///
/// **Narrower than the Z-machine's list on purpose.** `zvm::location` matches
/// avatar names against an object's PRINTED short name, where a generous set
/// costs little; matching against parse words is far noisier, because a word
/// array is every word the parser accepts and the loose ones are everywhere.
/// Measured on `CounterfeitMonkey-11.gblorb`: `me`, `you` and `player` pull in
/// conversation quips ("what he thinks of you") and a crowd of theatrical
/// `players` in `King_of_Shreds_and_Patches.gblorb`, none of them avatars.
///
/// Both standard libraries put a word from this list on their avatar, so
/// nothing is lost by the trim: Inform 6's `selfobj` carries `'me' 'myself'
/// 'self'` and Inform 7's Standard Rules `Understand "yourself" or "myself" or
/// "self" as yourself`.
const PLAYER_WORDS: [&str; 4] = ["yourself", "myself", "cretin", "adventurer"];

/// Printed short names that denote the avatar, lower-cased — `zvm::location`'s
/// list, which can afford to be generous for the reason above.
///
/// `(self object)` is the Inform 6 library's own `selfobj`, the avatar of every
/// Inform 6 game that never calls `ChangePlayer`. Inform 7 gives its `yourself`
/// object no hardware short name at all, so on those stories the word array is
/// the only evidence there is.
const PLAYER_NAMES: [&str; 8] =
    ["(self object)", "yourself", "you", "me", "myself", "self", "cretin", "adventurer"];

/// How far a parent chain is walked before it is called a cycle. Matches
/// `zvm::location::has_ancestor`.
const MAX_DEPTH: u32 = 32;

/// `NUM_ATTR_BYTES` is "always of the form 4i+3" (§2), and Inform's default is
/// 7. Trying them in likelihood order costs nothing and keeps the stride out of
/// the guesswork.
const ATTR_BYTE_CANDIDATES: [u32; 8] = [7, 3, 11, 15, 19, 23, 27, 31];

/// A tree must hold at least this many objects, and this many of them must have
/// a readable name array, before it is believed. Inform emits four metaclass
/// objects (`Class`, `Object`, `Routine`, `String`) before the story's own, so
/// a real tree clears both comfortably.
const MIN_OBJECTS: usize = 6;

/// Why an image cannot be asked what its objects are called.
///
/// Distinct from [`GrammarError`] for the reason that enum's own documentation
/// gives: sharing a refusal type means each reader carrying variants it can
/// never return. These three are this reader's, and the first wraps the
/// grammar reader's because the dictionary is genuinely a prerequisite —
/// without it the arrays are addresses that cannot be turned back into words.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObjectError {
    /// The dictionary could not be located, so an address in a `name` array
    /// cannot be resolved to a word. Carries the grammar reader's own refusal.
    NoDictionary(GrammarError),
    /// No run of RAM walks as an Inform object list. Not an Inform image, or
    /// not one this reader recognises.
    NoObjectTree,
    /// A tree was found, but too few of its objects hold a `name` array of
    /// dictionary records for it to be one. Refusing beats naming the words a
    /// run of arbitrary bytes happens to spell.
    NoNameArrays,
}

/// The reader for one story's parse names: where the object list is, how wide
/// an object is, and the dictionary needed to turn the addresses in a `name`
/// array back into words.
///
/// Built once per story ([`detect`](ParseNames::detect)) and then asked about
/// objects. It answers with [`ObjectWords`], which carries the object's
/// address, its printed name and its words together — the words are never
/// offered on their own, because a caller holding them without the name cannot
/// say which thing they belong to.
#[derive(Debug, Clone)]
pub struct ParseNames {
    head: u32,
    attr_bytes: u32,
    stride: u32,
    count: usize,
    tables: Tables,
}

impl ParseNames {
    /// Find this story's object list, or say why it cannot be found.
    pub fn detect(mem: &Memory) -> Result<ParseNames, ObjectError> {
        let tables = grammar::locate(mem).map_err(ObjectError::NoDictionary)?;
        let mut saw_tree = false;
        // RAM is what the image holds between RAMSTART and EXTSTART; everything
        // above EXTSTART is zero-filled at load and cannot hold a tree.
        for addr in mem.ramstart()..mem.extstart() {
            if mem.read8(addr) != Some(0x70) {
                continue;
            }
            for attr_bytes in ATTR_BYTE_CANDIDATES {
                let stride = 1 + attr_bytes + OBJECT_TAIL_BYTES;
                let Some(count) = walk_object_list(mem, addr, attr_bytes, stride) else {
                    continue;
                };
                if count < MIN_OBJECTS {
                    continue;
                }
                saw_tree = true;
                let candidate =
                    ParseNames { head: addr, attr_bytes, stride, count, tables };
                if candidate.readable_name_arrays(mem) >= MIN_OBJECTS {
                    return Ok(candidate);
                }
            }
        }
        Err(if saw_tree { ObjectError::NoNameArrays } else { ObjectError::NoObjectTree })
    }

    /// Address of the first object — Inform's `Class` metaclass, which §2 says
    /// every `objectloop` starts from.
    pub fn head(&self) -> u32 {
        self.head
    }

    /// How many objects the list holds.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Only ever false — a list this short is refused at detection — but clippy
    /// asks for it beside [`len`](ParseNames::len) and a caller may prefer it.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// `NUM_ATTR_BYTES` for this image, which is what fixes the object stride.
    pub fn attr_bytes(&self) -> u32 {
        self.attr_bytes
    }

    /// Every object's address, in list order.
    pub fn objects(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.count as u32).map(move |i| self.head + i * self.stride)
    }

    /// What the object at `addr` is, and what it can be called.
    ///
    /// `None` when `addr` is not an object of this list, when it has no `name`
    /// property, or when **any** entry of that property is not a dictionary
    /// record. That last one is the point: an array that is not a word array
    /// yields nothing, rather than words decoded from arbitrary addresses.
    pub fn of(&self, mem: &Memory, addr: u32) -> Option<ObjectWords> {
        if !self.is_object(mem, addr) {
            return None;
        }
        let (data, length) = self.name_array(mem, addr)?;
        let mut words = Vec::with_capacity(length as usize);
        for i in 0..length {
            words.push(self.dictionary_word(mem, mem.read32(data + i * 4)?)?);
        }
        Some(ObjectWords::new(
            addr,
            self.printed_name(mem, addr),
            words,
            Some(NAME_PROPERTY),
            Some(self.tables.dict_word_size as usize),
        ))
    }

    /// Every object that answers, in list order.
    pub fn all(&self, mem: &Memory) -> Vec<ObjectWords> {
        self.objects().filter_map(|addr| self.of(mem, addr)).collect()
    }

    /// The first object, in list order, that `word` refers to.
    pub fn find(&self, mem: &Memory, word: &str) -> Option<ObjectWords> {
        self.objects().filter_map(|addr| self.of(mem, addr)).find(|o| o.refers_to(word))
    }

    // ── the containment tree ────────────────────────────────────────────────
    //
    // §2's `parent`/`sibling`/`child` longs are Inform's ordinary containment
    // tree — the same one `objectloop (x in y)` walks — and are distinct from
    // the `next` link that threads every object in the image together. Every
    // reader below validates the address it reads back against this list, so a
    // field holding something that is not an object of ours answers `None`
    // rather than being followed.

    /// Address of the object at `index` in list order, or `None` past the end.
    pub fn addr_of(&self, index: usize) -> Option<u32> {
        (index < self.count).then(|| self.head + index as u32 * self.stride)
    }

    /// Position of `addr` in list order, or `None` when it is not an object of
    /// this list. Cheap: the list is a contiguous run of equal-stride records.
    pub fn index_of(&self, addr: u32) -> Option<usize> {
        if addr < self.head || !(addr - self.head).is_multiple_of(self.stride) {
            return None;
        }
        let i = ((addr - self.head) / self.stride) as usize;
        (i < self.count).then_some(i)
    }

    /// True when `addr` is one of this list's objects and still carries the
    /// `$70` tag. The tag is re-read rather than assumed because the tree lives
    /// in RAM and a running game writes to it.
    pub fn is_object(&self, mem: &Memory, addr: u32) -> bool {
        self.index_of(addr).is_some() && mem.read8(addr) == Some(0x70)
    }

    /// The object containing `addr`, or `None` when it is contained by nothing
    /// (Inform writes `0`) or `addr` is not an object of this list.
    pub fn parent(&self, mem: &Memory, addr: u32) -> Option<u32> {
        self.link(mem, addr, Field::Parent)
    }

    /// The next object beside `addr` in its parent's child list.
    pub fn sibling(&self, mem: &Memory, addr: u32) -> Option<u32> {
        self.link(mem, addr, Field::Sibling)
    }

    /// The first object contained by `addr`.
    pub fn child(&self, mem: &Memory, addr: u32) -> Option<u32> {
        self.link(mem, addr, Field::Child)
    }

    /// Everything directly inside `addr`, in the order the story keeps it.
    ///
    /// One level only — the question an inventory dock asks. Bounded by the
    /// list's own length, so a tree a running game has corrupted into a cycle
    /// terminates rather than spinning.
    pub fn children(&self, mem: &Memory, addr: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cur = self.child(mem, addr);
        while let Some(c) = cur {
            if out.len() >= self.count || out.contains(&c) {
                break;
            }
            out.push(c);
            cur = self.sibling(mem, c);
        }
        out
    }

    /// Everything directly inside `addr`, as answered objects.
    ///
    /// **Unlike [`all`](ParseNames::all) and [`of`](ParseNames::of), a child
    /// with no readable `name` array is still included here** (SQ-1241),
    /// falling back to its printed short name with an empty word list. `all`
    /// and `of` answer "what can the parser be told to fetch this by", which a
    /// `parse_name`-routine object genuinely has no static answer to; this
    /// answers "what is the player carrying", and an object does not stop
    /// being carried because Inform compiled its name into machine code
    /// instead of a static array. City of Secrets holds three such objects at
    /// the first prompt — Peter's letter, an express ticket and a wad of local
    /// money — each printable (its hardware short name) but not enumerable
    /// (its words come from a `parse_name` routine, not the `name` property),
    /// and dropping them from an inventory dock is not a naming refusal, it is
    /// half the player's own hands going unlisted.
    pub fn contents(&self, mem: &Memory, addr: u32) -> Vec<ObjectWords> {
        self.children(mem, addr)
            .into_iter()
            .map(|c| {
                self.of(mem, c).unwrap_or_else(|| {
                    ObjectWords::new(c, self.printed_name(mem, c), Vec::new(), None, None)
                })
            })
            .collect()
    }

    /// True when `ancestor` strictly contains `start`, at any depth.
    /// Depth-bounded ([`MAX_DEPTH`]) so a cycle is false rather than a hang.
    pub fn has_ancestor(&self, mem: &Memory, start: u32, ancestor: u32) -> bool {
        let mut cur = self.parent(mem, start);
        for _ in 0..MAX_DEPTH {
            match cur {
                None => return false,
                Some(a) if a == ancestor => return true,
                Some(a) => cur = self.parent(mem, a),
            }
        }
        false
    }

    /// The player's avatar, or `None` when no object in the story plausibly is
    /// one.
    ///
    /// The rule is `zvm::location::find_player_object`'s, because it is the same
    /// problem and the same trap: **a name alone does not identify the avatar**.
    /// Zork I ships two objects answering to avatar names and only one of them
    /// is the player; an Inform game routinely ships a conversation topic or a
    /// parser stand-in beside the real `selfobj`. So:
    ///
    /// 1. Candidates are objects whose `name` array holds one of
    ///    [`PLAYER_WORDS`] — asked through [`ObjectWords::refers_to`], so a
    ///    dictionary that truncates still matches — or whose printed short name
    ///    is one of [`PLAYER_NAMES`].
    /// 2. Candidates contained by nothing are dropped where any candidate is
    ///    contained at all: Inform parks its off-stage doubles at the top level,
    ///    and a player stands somewhere.
    /// 3. One survivor needs no discrimination.
    /// 4. Otherwise the avatar is the one actually WHERE THE PLAYER IS: the
    ///    candidate whose containment chain reaches `room` — the game's
    ///    `location`, which the app supplies by the two routes the module
    ///    header describes, and `None` when it cannot.
    ///
    /// Step 4 is not a formality. City of Secrets ships **both** shapes at
    /// once: Inform 6's own `selfobj`, printed `(self object)` and standing in
    /// the room, is the player, while a decoy printed `yourself` — with the
    /// richer word list `me/i/name/myself` — sits in a `(ConceptObjs)` bag.
    /// Anchorhead is the same pairing on the Z-machine with the same two names
    /// (`zvm::location`'s `PLAYER_NAMES` doc), and the two games put them the
    /// same way round, so no rule over the NAMES can separate them.
    ///
    /// **And where that cannot settle it, the answer is `None`.** There is no
    /// "first plausible candidate" fallback, because a wrong avatar is worse
    /// than no avatar: its children become an inventory the player is told they
    /// are carrying.
    ///
    /// Counterfeit Monkey is the story that makes the point, and it is refused
    /// outright: **not one of its 2,494 objects answers to `yourself`, `myself`
    /// or `self`**, and none carries an avatar-ish printed name — its Inform 7
    /// objects have no hardware short name at all. The only objects whose word
    /// arrays hold anything avatar-ish are conversation quips ("what he thinks
    /// of you", "what he kens about me"), parked together in a topics
    /// container. That is the limitation this module's header states from the
    /// other side: a conditional or multi-word `Understand` compiles to a
    /// `parse_name` ROUTINE rather than to the static array, and machine code
    /// is not enumerable in any Inform version.
    pub fn find_player(&self, mem: &Memory, room: Option<u32>) -> Option<u32> {
        let cands: Vec<u32> = self
            .objects()
            .filter(|&addr| match self.of(mem, addr) {
                Some(o) => {
                    PLAYER_WORDS.iter().any(|w| o.refers_to(w))
                        || PLAYER_NAMES.contains(&o.printed_name.to_lowercase().as_str())
                }
                // An object with no readable `name` array can still be the
                // avatar by its printed name alone — Inform 6's `selfobj` has
                // both, but a game that strips one keeps the other.
                None => PLAYER_NAMES.contains(&self.printed_name(mem, addr).to_lowercase().as_str()),
            })
            .collect();
        let situated: Vec<u32> =
            cands.iter().copied().filter(|&o| self.parent(mem, o).is_some()).collect();
        let pool = if situated.is_empty() { &cands } else { &situated };
        match pool.len() {
            0 => None,
            1 => pool.first().copied(),
            _ => pool.iter().copied().find(|&o| room.is_some_and(|r| self.has_ancestor(mem, o, r))),
        }
    }

    /// Where field `f` of the object at `addr` lives in this image.
    fn field(&self, addr: u32, f: Field) -> u32 {
        field_at(addr, self.attr_bytes, f)
    }

    /// Field `f` of `addr` read as a link to another object of this list:
    /// `None` for Inform's `0`, for an address that is not one of ours, and for
    /// an `addr` that is not one of ours either.
    fn link(&self, mem: &Memory, addr: u32, f: Field) -> Option<u32> {
        if !self.is_object(mem, addr) {
            return None;
        }
        let target = mem.read32(self.field(addr, f))?;
        self.is_object(mem, target).then_some(target)
    }

    /// The object's hardware short name — what the story PRINTS for it. Empty
    /// where it has none, which is the ordinary case on Inform 7; `None` when
    /// `addr` is not an object of this list.
    ///
    /// [`of`](ParseNames::of) already carries this, but only for an object with
    /// a readable `name` array. This answers for one without — which is how the
    /// app finds the ROOM the story has just printed a heading for, a room
    /// being an object nothing has to be able to refer to by word (SQ-1241).
    /// The Z-machine side has always found the room this way
    /// (`zvm::location::status_name_matches` against the status line).
    pub fn short_name(&self, mem: &Memory, addr: u32) -> Option<String> {
        self.is_object(mem, addr).then(|| self.printed_name(mem, addr))
    }

    /// The object's hardware short name, decoded. Empty when it has none, which
    /// is the normal case for Inform 7.
    fn printed_name(&self, mem: &Memory, addr: u32) -> String {
        let string = match mem.read32(self.field(addr, Field::Name)) {
            Some(a) if a != 0 => a,
            _ => return String::new(),
        };
        crate::disasm::string_text(mem, mem.decode_table(), string, None).unwrap_or_default()
    }

    /// `(data address, length in longs)` of the object's `name` property.
    ///
    /// §3: the table opens with a long count and its entries are sorted by id,
    /// so the scan may stop as soon as it passes 1.
    fn name_array(&self, mem: &Memory, addr: u32) -> Option<(u32, u32)> {
        let table = mem.read32(self.field(addr, Field::Props))?;
        let entries = mem.read32(table)?;
        // A count this large is not a property table; stop rather than walk
        // megabytes of whatever it is.
        if entries > 0x1000 {
            return None;
        }
        for i in 0..entries {
            let entry = table + 4 + i * PROP_ENTRY_BYTES;
            let id = mem.read16(entry)?;
            if id > NAME_PROPERTY {
                return None;
            }
            if id == NAME_PROPERTY {
                let length = mem.read16(entry + 2)?;
                let data = mem.read32(entry + 4)?;
                return (length > 0).then_some((data, length));
            }
        }
        None
    }

    /// `(data address, length in WORDS)` of object `addr`'s property `prop`
    /// (SQ-1264) — the general form of [`Self::name_array`] (property 1 only,
    /// with the early stop that assumes id 1 sorts first). `door_dir`/`*_to`/
    /// `door_to` (see `crate::world`) are ordinary user-numbered properties
    /// that can sit anywhere in the table, so this walks past property 1
    /// rather than stopping there — §3's sort-by-id guarantee still lets the
    /// scan give up the moment it passes `prop`.
    ///
    /// `None` when `addr` is not one of ours, it carries no property table, or
    /// simply does not have `prop` at all — same "absent" contract as
    /// [`Self::name_array`].
    pub fn property(&self, mem: &Memory, addr: u32, prop: u16) -> Option<(u32, u32)> {
        if !self.is_object(mem, addr) {
            return None;
        }
        let table = mem.read32(self.field(addr, Field::Props))?;
        let entries = mem.read32(table)?;
        if entries > 0x1000 {
            return None;
        }
        for i in 0..entries {
            let entry = table + 4 + i * PROP_ENTRY_BYTES;
            let id = mem.read16(entry)? as u16;
            if id == prop {
                let length = mem.read16(entry + 2)?;
                let data = mem.read32(entry + 4)?;
                return (length > 0).then_some((data, length));
            }
            if id > prop {
                return None;
            }
        }
        None
    }

    /// The first WORD of `addr`'s property `prop` (SQ-1264) — what
    /// `door_dir`/`*_to`/`door_to` actually store: a property NUMBER, an
    /// object address, or a routine/string address, always one word wide on
    /// Glulx (unlike the Z-machine, which packs several such values into one
    /// property entry on occasion). `None` exactly when [`Self::property`] is.
    pub fn property_word(&self, mem: &Memory, addr: u32, prop: u16) -> Option<u32> {
        let (data, _) = self.property(mem, addr, prop)?;
        mem.read32(data)
    }

    /// The text of the dictionary record at `addr`, or `None` if `addr` is not
    /// one.
    ///
    /// Strict on purpose. A record must start exactly on the dictionary's
    /// stride, lie inside its word count, and carry the `$60` tag Inform writes
    /// as the type identifier for a dictionary word — the same signature
    /// [`crate::grammar::locate`] uses to find the table in the first place.
    fn dictionary_word(&self, mem: &Memory, addr: u32) -> Option<String> {
        let base = self.tables.dictionary + 4;
        if addr < base || !(addr - base).is_multiple_of(self.tables.dict_stride) {
            return None;
        }
        if (addr - base) / self.tables.dict_stride >= self.tables.word_count {
            return None;
        }
        if mem.read8(addr) != Some(0x60) {
            return None;
        }
        // Both record shapes, exactly as `grammar::read_dictionary` reads them:
        // the text starts past the tag — one byte, or four once padded out to a
        // long — and a Unicode record's characters are big-endian longs.
        let text_at = if self.tables.dict_char_size == 4 { 4 } else { 1 };
        let mut text = String::new();
        for i in 0..self.tables.dict_word_size {
            let c = if self.tables.dict_char_size == 4 {
                mem.read32(addr + text_at + i * 4)
            } else {
                // Records are Latin-1 and Inform lower-cases them.
                mem.read8(addr + text_at + i)
            };
            match c {
                Some(0) | None => break,
                Some(c) => text.push(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER)),
            }
        }
        (!text.is_empty()).then_some(text)
    }

    /// How many of the list's objects hold a `name` array whose every entry is
    /// a dictionary record. The verification that separates a real object tree
    /// from a run of bytes that walks like one.
    fn readable_name_arrays(&self, mem: &Memory) -> usize {
        self.objects()
            .filter(|&addr| {
                let Some((data, length)) = self.name_array(mem, addr) else {
                    return false;
                };
                (0..length).all(|i| {
                    mem.read32(data + i * 4)
                        .and_then(|w| self.dictionary_word(mem, w))
                        .is_some()
                })
            })
            .count()
    }
}

/// Walk the object list from `head`, returning how many objects it holds.
///
/// `None` unless every link is exact: each object carries the `$70` tag, its
/// next-link names the object one stride along, and the last one's is `0`.
fn walk_object_list(mem: &Memory, head: u32, attr_bytes: u32, stride: u32) -> Option<usize> {
    let mut cur = head;
    let mut count = 0usize;
    loop {
        if mem.read8(cur) != Some(0x70) {
            return None;
        }
        count += 1;
        let next = mem.read32(field_at(cur, attr_bytes, Field::Next))?;
        if next == 0 {
            return Some(count);
        }
        if next != cur + stride {
            return None;
        }
        cur = next;
        // A story with more objects than this is not one anybody has written;
        // an unterminated run of `$70` bytes would otherwise loop to the end of
        // memory on every candidate offset.
        if count > 100_000 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── A synthetic Inform story, laid out by the spec rather than by this
    // reader's arithmetic ────────────────────────────────────────────────────
    //
    // Every offset below comes from "The Glulx Inform Technical Reference" §2
    // and §3 as quoted in this module's header, written INDEPENDENTLY of the
    // code under test: the builder places `parent` at `13 + NUM_ATTR_BYTES`
    // because §2 lists it fourth of the six longs, not because [`Field`] says
    // so. That is what makes a transposed pair a failure here rather than an
    // assumption both sides share and neither can see.
    //
    // The tables the reader wants before it will believe a tree — grammar,
    // actions, dictionary, in the order `tables.c::construct_storyfile_g`
    // emits them — are built too, so these cases go through
    // [`ParseNames::detect`] end to end rather than around it.

    const RAM: u32 = 0x100;
    const EXT: u32 = 0x700;
    /// Inform's `NUM_ATTR_BYTES` default, and the corpus's.
    const NAB: u32 = 7;
    /// `1 + NUM_ATTR_BYTES + 6 longs` (§2).
    const STRIDE: u32 = 1 + NAB + 24;
    const OBJ: u32 = 0x300;
    // PROPS, NAMEDATA and STRINGS each sit right after the region before
    // them, sized for `N_OBJ` objects, so the ninth object ([`LETTER`]) has
    // room without hand-picking gaps.
    const PROPS: u32 = OBJ + N_OBJ as u32 * STRIDE;
    const NAMEDATA: u32 = PROPS + N_OBJ as u32 * 16;
    const STRINGS: u32 = NAMEDATA + N_OBJ as u32 * 16;
    const DICT: u32 = 0x118;
    /// `DICT_ENTRY_BYTE_LENGTH` for a byte-valued dictionary of nine
    /// characters: `7 + DICT_WORD_SIZE` (`Inform6/inform.c`).
    const DICT_STRIDE: u32 = 16;

    /// The story's dictionary. At least `MIN_DICT_WORDS` (16) records, because
    /// [`crate::grammar`] refuses a shorter run as noise rather than a table —
    /// the nine this tree actually uses, then filler to clear the floor.
    const WORDS: [&str; 20] = [
        "you", "me", "myself", "lamp", "brass", "sack", "apple", "table", "kitchen", "north",
        "south", "east", "west", "up", "down", "take", "drop", "look", "open", "close",
    ];

    /// Object indices, in list order. The decoy comes FIRST on purpose: Zork I
    /// ships its parser stand-in `you` at #21 and the real avatar `cretin` at
    /// #46, so a rule preferring the earliest candidate picks the wrong one.
    const BAG: usize = 0;
    const DECOY: usize = 1;
    const ROOM: usize = 2;
    const PLAYER: usize = 3;
    const LAMP: usize = 4;
    const SACK: usize = 5;
    const APPLE: usize = 6;
    const TABLE: usize = 7;
    /// A child with an empty `name` array — a `parse_name`-routine object,
    /// exactly the shape City of Secrets' letter, ticket and money are
    /// (SQ-1241). Parked on the table so the pinned `PLAYER` child chain
    /// below is untouched.
    const LETTER: usize = 8;
    const N_OBJ: usize = 9;

    struct Story {
        buf: Vec<u8>,
    }

    impl Story {
        fn new() -> Story {
            let mut s = Story { buf: vec![0u8; EXT as usize] };
            s.buf[0..4].copy_from_slice(b"Glul");
            s.w32(0x04, 0x0003_0102);
            s.w32(0x08, RAM);
            s.w32(0x0C, EXT);
            s.w32(0x10, EXT);
            s.w32(0x14, 0x1000);
            s.w32(0x18, 0x40); // start function; never executed here
            s.tables();
            s.objects();
            s
        }

        fn b(&mut self, at: u32, v: u8) {
            self.buf[at as usize] = v;
        }

        fn w16(&mut self, at: u32, v: u16) {
            self.buf[at as usize..at as usize + 2].copy_from_slice(&v.to_be_bytes());
        }

        fn w32(&mut self, at: u32, v: u32) {
            self.buf[at as usize..at as usize + 4].copy_from_slice(&v.to_be_bytes());
        }

        /// An unencoded Glulx string: `$E0`, then Latin-1, then a zero byte.
        fn string(&mut self, at: u32, text: &str) -> u32 {
            self.b(at, 0xE0);
            for (i, c) in text.chars().enumerate() {
                self.b(at + 1 + i as u32, c as u32 as u8);
            }
            at
        }

        /// Grammar, actions and dictionary, contiguous and in that order — the
        /// minimum [`crate::grammar::locate`] accepts, so the object tree here
        /// is found the way it is found in a real story.
        fn tables(&mut self) {
            self.w32(0x100, 1); // one verb
            self.w32(0x104, 0x108); // its line block
            self.b(0x108, 1); // one line
            self.b(0x10C, 15); // ENDIT, ending the line's tokens
            self.w32(0x10D, 1); // one action…
            self.w32(0x111, 100); // …whose routine is in ROM
            self.w32(DICT, WORDS.len() as u32);
            for (i, w) in WORDS.iter().enumerate() {
                let e = self.dict_addr(i);
                self.b(e, 0x60); // the dictionary-record tag
                for (j, c) in w.chars().enumerate() {
                    self.b(e + 1 + j as u32, c as u32 as u8);
                }
                // Flags then the verb field, at `1 + DICT_WORD_SIZE`.
                self.w16(e + 10, 0);
                self.w16(e + 12, 0);
            }
        }

        fn dict_addr(&self, i: usize) -> u32 {
            DICT + 4 + i as u32 * DICT_STRIDE
        }

        fn word_addr(&self, w: &str) -> u32 {
            self.dict_addr(WORDS.iter().position(|x| *x == w).expect("a word of this dictionary"))
        }

        fn obj(&self, i: usize) -> u32 {
            OBJ + i as u32 * STRIDE
        }

        /// §2's object records. The six longs are written by NAME, in the order
        /// the reference lists them, and nothing here consults [`Field`].
        #[allow(clippy::type_complexity)]
        fn objects(&mut self) {
            // (printed name, parse words, parent, sibling, child)
            let plan: [(&str, &[&str], Option<usize>, Option<usize>, Option<usize>); N_OBJ] = [
                ("(globals)", &["kitchen"], None, None, Some(DECOY)),
                ("you", &["you"], Some(BAG), None, None),
                ("Kitchen", &["kitchen"], None, None, Some(PLAYER)),
                ("(self object)", &["me", "myself"], Some(ROOM), Some(TABLE), Some(LAMP)),
                ("brass lamp", &["lamp", "brass"], Some(PLAYER), Some(SACK), None),
                ("sack", &["sack"], Some(PLAYER), None, Some(APPLE)),
                ("apple", &["apple"], Some(SACK), None, None),
                ("table", &["table"], Some(ROOM), None, Some(LETTER)),
                ("Peter's letter", &[], Some(TABLE), None, None),
            ];
            for (i, (printed, words, parent, sibling, child)) in plan.iter().enumerate() {
                let at = self.obj(i);
                let base = at + 1 + NAB; // past the tag and the attribute bytes
                self.b(at, 0x70);
                // long 0: next object in the overall linked list, 0 on the last.
                let next = if i + 1 < N_OBJ { self.obj(i + 1) } else { 0 };
                self.w32(base, next);
                // long 1: hardware name string.
                let sname = self.string(STRINGS + i as u32 * 24, printed);
                self.w32(base + 4, sname);
                // long 2: property table address.
                let table = PROPS + i as u32 * 16;
                self.w32(base + 8, table);
                // longs 3, 4, 5: parent, sibling, child.
                let addr_of = |o: &Option<usize>| o.map(|k| OBJ + k as u32 * STRIDE).unwrap_or(0);
                self.w32(base + 12, addr_of(parent));
                self.w32(base + 16, addr_of(sibling));
                self.w32(base + 20, addr_of(child));
                // §3's property table: a long count, then ten-byte entries of
                // {short id, short length in words, long data, short flags}.
                // A `words.is_empty()` object gets no property-1 entry at all
                // — the shape a `parse_name`-routine object compiles to,
                // which has no static `name` array to read (SQ-1241).
                let data = NAMEDATA + i as u32 * 16;
                self.w32(table, if words.is_empty() { 0 } else { 1 });
                if !words.is_empty() {
                    self.w16(table + 4, NAME_PROPERTY as u16);
                    self.w16(table + 6, words.len() as u16);
                    self.w32(table + 8, data);
                    self.w16(table + 12, 0);
                    for (j, w) in words.iter().enumerate() {
                        let a = self.word_addr(w);
                        self.w32(data + j as u32 * 4, a);
                    }
                }
            }
        }

        fn mem(&self) -> Memory {
            Memory::new(self.buf.clone()).expect("synthetic image is valid")
        }
    }

    fn detected() -> (Story, Memory, ParseNames) {
        let story = Story::new();
        let mem = story.mem();
        let pn = ParseNames::detect(&mem).expect("the synthetic story has an object list");
        (story, mem, pn)
    }

    #[test]
    fn detect_finds_the_list_its_stride_and_its_length() {
        let (_s, _mem, pn) = detected();
        assert_eq!(pn.head(), OBJ, "the head is the lowest address whose walk closes");
        assert_eq!(pn.attr_bytes(), NAB, "NUM_ATTR_BYTES is derived from the stride that closes");
        assert_eq!(pn.len(), N_OBJ, "every object of the `next` chain is counted");
        assert!(!pn.is_empty());
    }

    #[test]
    fn names_decode_from_the_hardware_string_and_the_name_array() {
        let (s, mem, pn) = detected();
        let lamp = pn.of(&mem, s.obj(LAMP)).expect("the lamp answers");
        assert_eq!(lamp.printed_name, "brass lamp");
        assert_eq!(lamp.words, ["lamp", "brass"]);
        assert_eq!(lamp.property, Some(NAME_PROPERTY));
        assert!(lamp.refers_to("brass"), "Inform keeps adjectives in the same array");
        assert_eq!(pn.find(&mem, "apple").map(|o| o.id), Some(s.obj(APPLE)));
        // Every object but LETTER — it alone has no readable `name` array, on
        // purpose (SQ-1241's `contents_includes_a_child_with_no_readable_name_array`
        // below covers that it is still reachable through `contents`).
        assert_eq!(pn.all(&mem).len(), N_OBJ - 1, "all() still needs a readable name array");
    }

    /// The three containment longs, read one at a time. A transposed pair would
    /// answer a *plausible* tree — this is the case that refuses it.
    #[test]
    fn parent_sibling_and_child_are_the_fourth_fifth_and_sixth_longs() {
        let (s, mem, pn) = detected();
        assert_eq!(pn.parent(&mem, s.obj(APPLE)), Some(s.obj(SACK)));
        assert_eq!(pn.parent(&mem, s.obj(PLAYER)), Some(s.obj(ROOM)));
        assert_eq!(pn.parent(&mem, s.obj(ROOM)), None, "a room is contained by nothing");
        assert_eq!(pn.sibling(&mem, s.obj(LAMP)), Some(s.obj(SACK)));
        assert_eq!(pn.sibling(&mem, s.obj(SACK)), None, "the last of a child list");
        assert_eq!(pn.child(&mem, s.obj(SACK)), Some(s.obj(APPLE)));
        assert_eq!(pn.child(&mem, s.obj(APPLE)), None, "an apple holds nothing");
    }

    #[test]
    fn children_walks_one_level_and_contents_names_them() {
        let (s, mem, pn) = detected();
        assert_eq!(pn.children(&mem, s.obj(PLAYER)), vec![s.obj(LAMP), s.obj(SACK)]);
        assert_eq!(pn.children(&mem, s.obj(ROOM)), vec![s.obj(PLAYER), s.obj(TABLE)]);
        let carried: Vec<String> =
            pn.contents(&mem, s.obj(PLAYER)).iter().filter_map(|o| o.display_name()).collect();
        assert_eq!(carried, ["brass lamp", "sack"]);
        // One level, always: the apple is inside the sack, not in your hands.
        assert!(!carried.iter().any(|n| n == "apple"));
    }

    /// The regression itself (SQ-1241): a child with no readable `name` array
    /// — a `parse_name`-routine object, exactly what City of Secrets' Peter's
    /// letter, express ticket and wad of money compile to — must still be
    /// LISTED, not silently dropped, because it is genuinely something the
    /// table contains. Falsifies before the fix: `contents` used to be
    /// `children(...).filter_map(|c| self.of(mem, c))`, which drops LETTER
    /// exactly as `all()` and `of()` correctly do.
    #[test]
    fn contents_includes_a_child_with_no_readable_name_array() {
        let (s, mem, pn) = detected();
        assert!(
            pn.of(&mem, s.obj(LETTER)).is_none(),
            "no property-1 array to answer `of` with — the parse_name-routine shape"
        );
        assert_eq!(pn.children(&mem, s.obj(TABLE)), vec![s.obj(LETTER)]);
        let held = pn.contents(&mem, s.obj(TABLE));
        assert_eq!(held.len(), 1, "a wordless child must still be listed, not dropped");
        assert_eq!(held[0].printed_name, "Peter's letter");
        assert!(held[0].words.is_empty(), "no parser words are known for it, and none are invented");
        assert_eq!(held[0].display_name().as_deref(), Some("Peter's letter"));
    }

    #[test]
    fn has_ancestor_walks_the_whole_chain() {
        let (s, mem, pn) = detected();
        assert!(pn.has_ancestor(&mem, s.obj(APPLE), s.obj(ROOM)), "apple → sack → player → room");
        assert!(!pn.has_ancestor(&mem, s.obj(DECOY), s.obj(ROOM)), "the decoy is off in a bag");
        assert!(!pn.has_ancestor(&mem, s.obj(ROOM), s.obj(ROOM)), "strictly an ancestor");
    }

    /// The whole point of taking a room: two objects answer to avatar words and
    /// only one of them is where the player is.
    #[test]
    fn find_player_prefers_the_candidate_that_is_in_the_room() {
        let (s, mem, pn) = detected();
        assert_eq!(
            pn.find_player(&mem, Some(s.obj(ROOM))),
            Some(s.obj(PLAYER)),
            "the avatar is the candidate whose chain reaches the room, not the earliest one"
        );
        // …and it is found by its PRINTED name — Inform 6's `selfobj` — while
        // the decoy is found by its parse word, so both routes are live.
        assert_eq!(pn.of(&mem, s.obj(PLAYER)).unwrap().printed_name, "(self object)");
    }

    /// Two situated candidates and nothing to tell them apart: refuse. There is
    /// no "first plausible one" here — that answer would put the decoy's
    /// children in the player's hands, and a wrong inventory is worse than an
    /// empty one. This is why the app supplies the learned `location` global.
    #[test]
    fn find_player_refuses_when_the_room_cannot_settle_two_candidates() {
        let (s, mem, pn) = detected();
        assert_eq!(pn.find_player(&mem, None), None, "no room, two situated candidates");
        assert_eq!(
            pn.find_player(&mem, Some(s.obj(BAG))),
            Some(s.obj(DECOY)),
            "…and the room is what decides, whichever way it points"
        );
    }

    #[test]
    fn indices_and_bounds_are_refused_rather_than_extrapolated() {
        let (s, mem, pn) = detected();
        assert_eq!(pn.addr_of(0), Some(OBJ));
        assert_eq!(pn.addr_of(N_OBJ - 1), Some(s.obj(N_OBJ - 1)));
        assert_eq!(pn.addr_of(N_OBJ), None, "one past the end is not an object");
        assert_eq!(pn.index_of(s.obj(SACK)), Some(SACK));
        assert_eq!(pn.index_of(OBJ + 1), None, "mid-record is not an object");
        assert_eq!(pn.index_of(OBJ - STRIDE), None, "before the head is not an object");
        assert_eq!(pn.index_of(OBJ + N_OBJ as u32 * STRIDE), None, "past the end is not an object");
        assert!(pn.is_object(&mem, s.obj(TABLE)));
        assert!(!pn.is_object(&mem, OBJ + 3));
        // A link read off something that is not an object is not followed.
        assert_eq!(pn.parent(&mem, OBJ + 3), None);
        assert!(pn.children(&mem, OBJ + 3).is_empty());
    }
}
