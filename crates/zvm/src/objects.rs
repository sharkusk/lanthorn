// Z-machine object table — ZMSD §12.
//
// Attributes, parent/child/sibling tree, properties, short names.
// v3: 1-byte object numbers, 32 attrs, 31-word default table, 9-byte entries.
// v4+: 2-byte object numbers, 48 attrs, 63-word default table, 14-byte entries.

#[cfg(feature = "grammar")]
use crate::dictionary;
#[cfg(feature = "grammar")]
use crate::grammar;
use crate::memory::Memory;
use crate::text::decode_string;
#[cfg(feature = "grammar")]
use std::collections::{BTreeMap, BTreeSet};

/// The shared answer type, re-exported so `zvm::objects::ObjectWords` names it
/// — as `zvm::grammar` re-exports the rest of `grammar-model`. Behind the
/// `grammar` feature along with [`ParseNames`], the only reader that produces
/// one.
#[cfg(feature = "grammar")]
pub use grammar_model::{Adjectives, ObjectWords};

// ── Layout helpers ────────────────────────────────────────────────────────────

/// Number of words in the property-defaults table.
fn prop_defaults_count(version: u8) -> u32 {
    if version <= 3 { 31 } else { 63 }
}

/// Number of attribute bytes per object entry.
fn attr_bytes(version: u8) -> u32 {
    if version <= 3 { 4 } else { 6 }
}

/// Size of a single object entry in bytes.
pub(crate) fn entry_size(version: u8) -> u32 {
    if version <= 3 { 9 } else { 14 }
}

/// Base address of the object-entries region (after the property-defaults table).
pub(crate) fn entries_base(mem: &Memory) -> u32 {
    mem.object_table() as u32 + prop_defaults_count(mem.version()) * 2
}

/// Byte address of object `obj`'s entry (1-based). Object 0 is the null object.
/// Public single source of truth for the object-table layout so callers don't
/// duplicate the 31/63-defaults + 9/14-entry-size constants.
pub fn object_entry_addr(mem: &Memory, obj: u16) -> u32 {
    entry_addr(mem, obj)
}

/// Byte address of object `obj`'s entry (object numbers are 1-based).
/// Object 0 is the null object — callers must guard against it.
fn entry_addr(mem: &Memory, obj: u16) -> u32 {
    debug_assert!(obj != 0, "entry_addr called with null object 0");
    entries_base(mem) + (obj as u32 - 1) * entry_size(mem.version())
}

/// True when object `obj` is a real, in-memory object (non-null and its whole
/// entry lies within loaded memory). Out-of-range object numbers are treated as
/// the null object by every accessor below rather than faulting the VM.
///
/// This matters for Zork Zero (v6): its display code deterministically derives a
/// window-record *address* (e.g. 0xc118) and hands it to `test_attr`, guarded
/// only by `je gb1, #021c` — i.e. the story assumes the interpreter answers
/// `test_attr`/`get_prop`/tree queries on a non-object with the null-object
/// result (false / defaults / 0) instead of reading past the story image. A
/// bounds fault here made the game unplayable on the turn after any room change.
fn addressable(mem: &Memory, obj: u16) -> bool {
    obj != 0 && (entry_addr(mem, obj) + entry_size(mem.version())) as usize <= mem.len()
}

// ── Attributes ────────────────────────────────────────────────────────────────

/// Read attribute `attr` of object `obj`.  Attribute N is bit `7-(N%8)` of
/// attribute byte `N/8` (MSB-first, ZMSD §12.3).
/// Object 0 is the null object; returns false.
pub fn get_attr(mem: &Memory, obj: u16, attr: u8) -> bool {
    if !addressable(mem, obj) {
        return false;
    }
    let byte_addr = entry_addr(mem, obj) + (attr / 8) as u32;
    let bit = 7 - (attr % 8);
    (mem.read_byte(byte_addr) >> bit) & 1 == 1
}

/// Set attribute `attr` of object `obj`.
pub fn set_attr(mem: &mut Memory, obj: u16, attr: u8) {
    if !addressable(mem, obj) { return; }
    let byte_addr = entry_addr(mem, obj) + (attr / 8) as u32;
    let bit = 7 - (attr % 8);
    let v = mem.read_byte(byte_addr) | (1 << bit);
    mem.write_byte(byte_addr, v);
}

/// Clear attribute `attr` of object `obj`.
pub fn clear_attr(mem: &mut Memory, obj: u16, attr: u8) {
    if !addressable(mem, obj) { return; }
    let byte_addr = entry_addr(mem, obj) + (attr / 8) as u32;
    let bit = 7 - (attr % 8);
    let v = mem.read_byte(byte_addr) & !(1 << bit);
    mem.write_byte(byte_addr, v);
}

// ── Tree pointers ─────────────────────────────────────────────────────────────

/// Byte offset within an entry of the parent/sibling/child fields.
/// They come after the attribute bytes.
fn tree_offset(version: u8, which: u8) -> u32 {
    // which: 0=parent, 1=sibling, 2=child
    if version <= 3 {
        attr_bytes(version) + which as u32
    } else {
        attr_bytes(version) + (which as u32) * 2
    }
}

fn read_tree_ptr(mem: &Memory, obj: u16, which: u8) -> u16 {
    if !addressable(mem, obj) {
        return 0;
    }
    let base = entry_addr(mem, obj) + tree_offset(mem.version(), which);
    if mem.version() <= 3 {
        mem.read_byte(base) as u16
    } else {
        mem.read_word(base)
    }
}

fn write_tree_ptr(mem: &mut Memory, obj: u16, which: u8, val: u16) {
    // Same null-object leniency as the read side: an out-of-range object's
    // tree writes are graceful no-ops (insert_obj/remove_obj on a non-object
    // must not corrupt memory past the entry table or fault).
    if !addressable(mem, obj) {
        return;
    }
    let base = entry_addr(mem, obj) + tree_offset(mem.version(), which);
    if mem.version() <= 3 {
        mem.write_byte(base, val as u8);
    } else {
        mem.write_word(base, val);
    }
}

pub fn get_parent(mem: &Memory, obj: u16) -> u16 { read_tree_ptr(mem, obj, 0) }
pub fn get_sibling(mem: &Memory, obj: u16) -> u16 { read_tree_ptr(mem, obj, 1) }
pub fn get_child(mem: &Memory, obj: u16) -> u16 { read_tree_ptr(mem, obj, 2) }

pub fn set_parent(mem: &mut Memory, obj: u16, val: u16) { write_tree_ptr(mem, obj, 0, val); }
pub fn set_sibling(mem: &mut Memory, obj: u16, val: u16) { write_tree_ptr(mem, obj, 1, val); }
pub fn set_child(mem: &mut Memory, obj: u16, val: u16) { write_tree_ptr(mem, obj, 2, val); }

// ── Tree manipulation ─────────────────────────────────────────────────────────

/// Remove `obj` from its current parent's child list, leaving `obj`'s own
/// children untouched.  Sets `obj`'s parent and sibling to 0.
///
/// Returns `false` if the parent's sibling chain exceeded the maximum possible
/// number of distinct objects — i.e. the object table is corrupted with a
/// sibling cycle — so the caller can latch a fault instead of hanging the VM.
#[must_use]
pub fn remove_obj(mem: &mut Memory, obj: u16) -> bool {
    let parent = get_parent(mem, obj);
    if parent == 0 {
        // Already detached.
        return true;
    }
    let first_child = get_child(mem, parent);
    if first_child == obj {
        // obj is the first child — promote its sibling.
        let sib = get_sibling(mem, obj);
        set_child(mem, parent, sib);
    } else {
        // Walk sibling chain to find the predecessor of obj. A valid chain can
        // hold at most one entry per object number (object numbers are u16 and
        // 1-based), so more iterations than that proves a cycle in a corrupted
        // table — bail out rather than spin forever (cf. the depth bound in
        // location.rs's nearest_matching_ancestor for the parent-chain analogue).
        let mut cur = first_child;
        let mut steps = 0u32;
        loop {
            let next = get_sibling(mem, cur);
            if next == obj {
                let obj_sib = get_sibling(mem, obj);
                set_sibling(mem, cur, obj_sib);
                break;
            }
            if next == 0 {
                // obj not found in chain — shouldn't happen in a valid story.
                break;
            }
            steps += 1;
            if steps > u16::MAX as u32 {
                return false; // sibling cycle — corrupted object table
            }
            cur = next;
        }
    }
    set_parent(mem, obj, 0);
    set_sibling(mem, obj, 0);
    true
}

/// Make `obj` the first child of `dest`, first unlinking it from its current
/// parent.  (ZMSD §12.4 insert_obj semantics.)
///
/// Returns `false` on a corrupted sibling chain (see [`remove_obj`]).
#[must_use]
pub fn insert_obj(mem: &mut Memory, obj: u16, dest: u16) -> bool {
    let ok = remove_obj(mem, obj);
    let old_first = get_child(mem, dest);
    set_sibling(mem, obj, old_first);
    set_child(mem, dest, obj);
    set_parent(mem, obj, dest);
    ok
}

// ── Property table address ────────────────────────────────────────────────────

/// Address of the property-table pointer field within the object entry.
pub(crate) fn prop_table_ptr_offset(version: u8) -> u32 {
    // After attributes (4 or 6 bytes) + 3 tree fields (1-byte each v3, 2-byte each v4+).
    if version <= 3 {
        4 + 3
    } else {
        6 + 6
    }
}

/// Read the property-table address stored in an object's entry.
fn prop_table_addr(mem: &Memory, obj: u16) -> u32 {
    let base = entry_addr(mem, obj) + prop_table_ptr_offset(mem.version());
    mem.read_word(base) as u32
}

/// Byte address of object `obj`'s property table — the ZMSD §12.4 header, i.e.
/// the one-byte count of the short name's Z-text *words*, immediately followed
/// by the name itself and then the properties. `None` when `obj` is not an
/// addressable object.
///
/// Public counterpart to [`object_entry_addr`]: the entry (§12.3) holds the
/// attribute flags, the tree links and this pointer, but none of the object's
/// text — so a caller wanting to *see* an object's name wants this address, not
/// the entry's.
pub fn object_prop_table_addr(mem: &Memory, obj: u16) -> Option<u32> {
    addressable(mem, obj).then(|| prop_table_addr(mem, obj))
}

// ── Short name ────────────────────────────────────────────────────────────────

/// Decode the short name of object `obj` from its property table.
pub fn short_name(mem: &Memory, obj: u16) -> String {
    if !addressable(mem, obj) {
        return String::new();
    }
    let ptbl = prop_table_addr(mem, obj);
    // Byte 0: number of words of Z-text for the short name.
    let name_words = mem.read_byte(ptbl) as u32;
    if name_words == 0 {
        return String::new();
    }
    let (s, _) = decode_string(mem, ptbl + 1);
    s
}

// ── The name the STORY prints ─────────────────────────────────────────────────

/// What the story prints for object `obj` — its `short_name` PROPERTY when that
/// property holds a string, and the object header's short name otherwise
/// (SQ-1372).
///
/// [`short_name`] reads §12.4's header name and nothing else, which is what
/// `@print_obj` prints. The Inform 6 library does not print objects that way:
/// `parserm.h`'s `PrintShortName` runs
///
/// ```text
/// if (obj provides short_name) i = PrintOrRun(obj, short_name, true);
/// if (i == 0) print (object) obj;
/// ```
///
/// so a `short_name` property WINS over the header name, and a story that gives
/// one never has the header name printed at all. Graham Nelson's *Adventure* is
/// the specimen: `Class MazeRoom with short_name "Maze"` and thirty-nine rooms
/// declared `MazeRoom Alike_Maze_1` with no quoted name — the header name Inform
/// compiles for those is the parenthesised IDENTIFIER, `(Alike_Maze_1)`, which
/// no player ever sees. Reading only the header name renamed every maze room
/// after its source identifier, and the automapper's `mentions_maze` test — and
/// with it the whole maze-layer split — never fired.
///
/// **A `short_name` ROUTINE keeps the header name.** `PrintOrRun` calls it, and
/// machine code cannot be read statically; SQ-1307's walker is where a computed
/// name would have to come from. The routine case is *detected* rather than
/// guessed: ZMSD §5.1 fixes a routine's first byte as its local count, 0..=15,
/// and no Z-string's first byte can be that low unless its first Z-character is
/// a space or an abbreviation — so a first byte above 15 at the address the
/// value unpacks to as a ROUTINE proves the value is not one. Anything that
/// fails that test falls back to the header name rather than printing the
/// gibberish a routine decodes to.
pub fn printed_name(mem: &Memory, obj: u16) -> String {
    inform_short_name(mem, obj).unwrap_or_else(|| short_name(mem, obj))
}

/// The decoded `short_name` property of `obj`, or `None` where the story has no
/// such property, this object does not provide it, or its value is not readable
/// as a string. See [`printed_name`].
fn inform_short_name(mem: &Memory, obj: u16) -> Option<String> {
    // Discovered once per story and remembered: the scan below reads the whole
    // image, and this is called for every object the location ladder considers,
    // every turn.
    let prop = (*mem.short_name_prop_memo().get_or_init(|| short_name_property(mem)))?;
    let addr = get_prop_addr(mem, obj, prop);
    if addr == 0 || get_prop_len(mem, addr) != 2 {
        return None;
    }
    let packed = mem.read_word(addr as u32);
    if packed == 0 {
        return None;
    }
    // ZMSD §5.1: "A routine begins with one byte indicating the number of local
    // variables it has (between 0 and 15 inclusive)." A first byte above 15
    // therefore cannot start a routine. In v6/v7 the routine and string offsets
    // differ (§1.2.3), so the test is applied where a ROUTINE would begin.
    let rtn = mem.unpack_routine(packed);
    if rtn as usize >= mem.len() || mem.without_fault_latch(|| mem.read_byte(rtn)) <= 15 {
        return None;
    }
    let text = decoded_string_at(mem, mem.unpack_string(packed))?;
    (!text.trim().is_empty()).then_some(text)
}

/// Longest Z-string this module's speculative decodes will consider, in words.
/// A printed short name is a room heading or an object's name; 48 words is 144
/// Z-characters, well past any of them.
const NAME_PROBE_MAX_WORDS: u32 = 48;

/// Decode the Z-string at `addr`, or `None` when the bytes there are not one:
/// out of bounds, or not terminated within [`NAME_PROBE_MAX_WORDS`].
///
/// Speculative by nature — the caller has guessed that a property word is a
/// packed string address — so the decode runs under
/// [`Memory::without_fault_latch`]: an abbreviation inside a mis-guessed string
/// can point anywhere, and a read that lands out of bounds must answer "not a
/// string" rather than fault the story (the rule `location.rs`'s room-text probe
/// has followed since SQ-0724).
fn decoded_string_at(mem: &Memory, addr: u32) -> Option<String> {
    let terminated = (0..NAME_PROBE_MAX_WORDS)
        .map(|i| addr + i * 2)
        .take_while(|&at| at as usize + 1 < mem.len())
        .any(|at| mem.read_word(at) & 0x8000 != 0);
    terminated.then(|| mem.without_fault_latch(|| decode_string(mem, addr).0))
}

/// The property number this story gives Inform's `short_name`, read out of the
/// story's OWN symbol table (SQ-1372). `None` for a story that carries no such
/// table — a ZIL game, or an Inform 6 compile with `$OMIT_SYMBOL_TABLE=1`.
///
/// The number cannot be assumed. Inform 6 hardwires exactly one property
/// number, `name` = 1 (`Inform6/src/objects.c`); everything else is numbered in
/// declaration order, so `short_name` is whatever the LIBRARY's declaration
/// order made it — 46 in `advent.z6` (Inform 6.21), 45 in the Glulx build of the
/// same game. A constant here would be right for one library release and
/// silently wrong for the next.
///
/// So it is read rather than assumed. The story carries its own symbol table:
/// Inform's *Table of Identifier Names*, which `#identifiers_table` names and
/// `$OMIT_SYMBOL_TABLE=1` is the switch for leaving OUT — so it is there unless
/// a story went out of its way. `Inform6/tables.c`, `construct_storyfile`,
/// writes it as
///
/// ```text
/// identifier_names_offset:
///     word  no_individual_properties                  (the count)
///     word  individual_name_strings[i]  for i = 1 …   (packed string address)
/// ```
///
/// — one packed string address per property number, indexed BY that number.
/// The scan looks for a table whose entry **1** decodes to exactly `"name"` (the
/// one number the compiler fixes, which is what makes the table identifiable at
/// all) and returns the index whose entry decodes to `"short_name"`.
///
/// Only dynamic memory is searched, and only above the object table: the same
/// function writes the class-numbers table, this one and the individuals table
/// straight after the object tables, and sets `static_memory_offset =
/// grammar_table_at`, which comes later still — so this table is always inside
/// `[object_table, static_mem_base)`.
///
/// The search stops at the highest COMMON property number (ZMSD §12.2: 31 in
/// v3, 63 in v4+). `short_name` is declared `Property short_name` in the
/// library, so it is a common property; the table continues past that point
/// with individual property names, and an index taken from there would name a
/// property `get_prop_addr` cannot read.
///
/// **`"name"` alone is not enough of an anchor**, and a game has already proved
/// it: `Facility.z8` has a run of bytes whose word 1 unpacks to a string
/// reading `name` and whose other entries decode to prose and to fragments of
/// prose starting mid-word. Taking that table gave property 21 and renamed 169
/// objects to things like `"regexp too complex"`. So every entry the search
/// reads must be an IDENTIFIER (see [`is_inform_identifier`]) or zero — which
/// is what an identifiers table holds by definition — and enough of them must
/// be named to make a coincidence implausible.
pub fn short_name_property(mem: &Memory) -> Option<u8> {
    let len = mem.len() as u32;
    let max_prop = prop_defaults_count(mem.version());
    let entry = |a: u32| -> Option<String> {
        let packed = mem.read_word(a);
        if packed == 0 {
            return None;
        }
        decoded_string_at(mem, mem.unpack_string(packed))
    };
    let window = mem.object_table() as u32..(mem.static_mem_base() as u32).min(len);
    'table: for base in window {
        // A table too short to hold a property name, or one that would run off
        // the end of the story, is not the identifiers table.
        let count = mem.read_word(base) as u32;
        if count < 2 || base + 2 + count * 2 > len {
            continue;
        }
        if entry(base + 2).as_deref() != Some("name") {
            continue;
        }
        let mut found = None;
        let mut named = 0;
        let mut library = false;
        for prop in 2..=count.min(max_prop) {
            let Some(text) = entry(base + prop * 2) else { continue };
            if !is_inform_identifier(&text) {
                continue 'table; // prose where a symbol should be: not the table
            }
            named += 1;
            match text.as_str() {
                "short_name" => found = u8::try_from(prop).ok(),
                EXIT_PROPERTY => library = true,
                _ => {}
            }
        }
        if library && named >= MIN_IDENTIFIERS {
            if let Some(prop) = found {
                return Some(prop);
            }
        }
    }
    None
}

/// The Inform 6 library property whose presence in the identifiers table says
/// this story is built on that library — declared in `linklpa.h` beside
/// `short_name` itself, and absent from Inform 7's template, which puts its map
/// in `Map_Storage` arrays instead.
///
/// The gate exists because of `Facility.z8`, an Inform 7 story whose symbol
/// table is genuine and DOES name a `short_name` at property 21 — I7 borrows
/// much of the library's property vocabulary — but whose values are neither
/// strings nor routines. Decoding one gave `"regexp too complex"`, and 169
/// objects were renamed to fragments of the runtime's error messages. On the
/// Z-machine a property word is just a word: ZMSD §5.1 proves the value is not
/// a ROUTINE (see [`printed_name`]) and nothing proves it is a STRING, so the
/// only safe reading of a bare word is one made under a convention we can name.
/// That convention is `parserm.h`'s `PrintShortName`, and this is how the story
/// says it is the library that defines it. Glulx needs no such gate: §1.6.1
/// tags every object in memory, so `gvm` can simply ask whether the value IS a
/// string.
const EXIT_PROPERTY: &str = "n_to";

/// How many named entries a candidate identifiers table must carry before it is
/// believed. The Inform 6 library declares dozens of common properties before a
/// story's own; a handful of accidental identifier-shaped decodes in a row is
/// not a symbol table.
const MIN_IDENTIFIERS: usize = 8;

/// Whether `s` is shaped like a compiler symbol — what every entry of Inform's
/// identifier-names table is, and what a mis-guessed address decoded as text
/// virtually never is.
fn is_inform_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The byte span `[start, end)` of object `obj`'s short-name Z-text.
///
/// ZMSD §12.4: a property table opens with a one-byte count of the Z-text
/// *words* its short name occupies, and the text itself follows immediately —
/// so the span is known without decoding a single character. `None` when `obj`
/// is unaddressable or its name is empty (count 0).
///
/// Lets a caller that already holds a memory address ask "is this inside a
/// short name, and where did that name start?" — a Z-string can only be decoded
/// from its start (§3.2: the alphabet shift and abbreviation state carry across
/// words, so there is no resync point mid-string).
pub fn short_name_span(mem: &Memory, obj: u16) -> Option<(u32, u32)> {
    if !addressable(mem, obj) {
        return None;
    }
    let ptbl = prop_table_addr(mem, obj);
    let name_words = mem.read_byte(ptbl) as u32;
    (name_words > 0).then(|| (ptbl + 1, ptbl + 1 + name_words * 2))
}

// ── Property walking ──────────────────────────────────────────────────────────

/// Returns the address of the first property entry (just after the name).
fn first_prop_addr(mem: &Memory, obj: u16) -> u32 {
    let ptbl = prop_table_addr(mem, obj);
    let name_words = mem.read_byte(ptbl) as u32;
    ptbl + 1 + name_words * 2
}

/// Parse the size byte(s) at `addr` in a property entry.
/// Returns `(property_number, data_length_in_bytes, header_length_in_bytes)`.
///
/// v3 (ZMSD §12.4.1): one size byte; bits 7–5 = (size-1), bits 4–0 = prop num.
/// v4+ (ZMSD §12.4.2): if bit 7 clear, one byte: bits 6–0 encode prop num,
///   bit 6 set → 2-byte data, else 1-byte data. If bit 7 set, two size bytes:
///   first byte bits 5–0 = prop num; second byte bits 5–0 = data length
///   (0 means 64).
fn parse_prop_header(mem: &Memory, addr: u32) -> (u16, u32, u32) {
    let b0 = mem.read_byte(addr);
    if mem.version() <= 3 {
        let prop_num = (b0 & 0x1F) as u16;
        let data_len = ((b0 >> 5) as u32) + 1;
        (prop_num, data_len, 1)
    } else {
        if b0 & 0x80 != 0 {
            // Two-byte form.
            let prop_num = (b0 & 0x3F) as u16;
            let b1 = mem.read_byte(addr + 1);
            let raw_len = (b1 & 0x3F) as u32;
            let data_len = if raw_len == 0 { 64 } else { raw_len };
            (prop_num, data_len, 2)
        } else {
            // One-byte form.
            let prop_num = (b0 & 0x3F) as u16;
            let data_len = if b0 & 0x40 != 0 { 2 } else { 1 };
            (prop_num, data_len, 1)
        }
    }
}

/// Find the address of property `prop`'s data in `obj`'s property table.
/// Returns 0 if the property is absent or `obj` is 0.
pub fn get_prop_addr(mem: &Memory, obj: u16, prop: u8) -> u16 {
    if !addressable(mem, obj) {
        return 0;
    }
    let mut addr = first_prop_addr(mem, obj);
    loop {
        let b0 = mem.read_byte(addr);
        if b0 == 0 {
            return 0; // end-of-table sentinel
        }
        let (pnum, data_len, hdr_len) = parse_prop_header(mem, addr);
        if pnum == prop as u16 {
            return (addr + hdr_len) as u16;
        }
        if pnum < prop as u16 {
            // Properties are stored in descending order.
            return 0;
        }
        addr += hdr_len + data_len;
    }
}

/// Read the data length (in bytes) of the property whose data starts at
/// `prop_addr`.  The size byte immediately precedes the data. (ZMSD §12.4.4)
pub fn get_prop_len(mem: &Memory, prop_addr: u16) -> u8 {
    if prop_addr == 0 {
        return 0;
    }
    let size_byte_addr = prop_addr as u32 - 1;
    let b = mem.read_byte(size_byte_addr);
    if mem.version() <= 3 {
        (b >> 5) + 1
    } else {
        if b & 0x80 != 0 {
            // Two-byte form: the byte immediately before prop_addr is the
            // second size byte (bits 5–0 = data length).
            let raw = b & 0x3F;
            if raw == 0 { 64 } else { raw }
        } else {
            // One-byte form.
            if b & 0x40 != 0 { 2 } else { 1 }
        }
    }
}

/// Read property `prop` of object `obj`.  Falls back to the property-defaults
/// table if the property is absent from the object.  (ZMSD §12.1)
/// Object 0 falls back to property defaults immediately.
pub fn get_prop(mem: &Memory, obj: u16, prop: u8) -> u16 {
    // Property 0 does not exist (property numbers are 1..=31 / 1..=63, ZMSD
    // §12.4.1/§12.4.2) — get_prop 0 is illegal game code. Without this guard
    // the defaults-table fallback below computes `(0 - 1) * 2`, which
    // underflows (debug panic / wild read in release). Frotz performs the
    // equivalent wild read (object table base minus 2) and returns garbage;
    // we answer with a benign 0 instead.
    if prop == 0 {
        return 0;
    }
    let addr = get_prop_addr(mem, obj, prop);
    if addr == 0 {
        // Property defaults: 0-indexed, prop 1 is at offset 0.
        let default_addr = mem.object_table() as u32 + (prop as u32 - 1) * 2;
        return mem.read_word(default_addr);
    }
    let len = get_prop_len(mem, addr);
    match len {
        1 => mem.read_byte(addr as u32) as u16,
        _ => mem.read_word(addr as u32),
    }
}

/// Write property `prop` of object `obj` to `val`.  Only 1- and 2-byte
/// properties are writable via put_prop. (ZMSD §12.4)
///
/// Returns `false` when the object has no such property — ZMSD §15 put_prop:
/// "If the property does not exist for that object, the interpreter should
/// halt with a suitable error message" — so the caller latches a VM fault
/// (never a process panic; illegal story code must not crash the host).
#[must_use]
pub fn put_prop(mem: &mut Memory, obj: u16, prop: u8, val: u16) -> bool {
    // An out-of-range object has no property table (get_prop_addr → 0); treat
    // the write as a graceful no-op rather than faulting, matching the read-side
    // null-object handling above.
    if !addressable(mem, obj) {
        return true;
    }
    let addr = get_prop_addr(mem, obj, prop);
    if addr == 0 {
        return false;
    }
    let len = get_prop_len(mem, addr);
    match len {
        1 => mem.write_byte(addr as u32, val as u8),
        _ => mem.write_word(addr as u32, val),
    }
    true
}

/// Return the next property number after `prop` in object `obj`'s property
/// table.  If `prop` is 0, returns the first property number.  Returns 0 if
/// there are no more properties. (ZMSD §12.4.3)
pub fn get_next_prop(mem: &Memory, obj: u16, prop: u8) -> u8 {
    if !addressable(mem, obj) {
        return 0;
    }
    let mut addr = first_prop_addr(mem, obj);
    if prop == 0 {
        // Return first property.
        let b0 = mem.read_byte(addr);
        if b0 == 0 {
            return 0;
        }
        let (pnum, _, _) = parse_prop_header(mem, addr);
        return pnum as u8;
    }
    // Scan for prop, then return the next one.
    loop {
        let b0 = mem.read_byte(addr);
        if b0 == 0 {
            return 0; // prop not found or end of table
        }
        let (pnum, data_len, hdr_len) = parse_prop_header(mem, addr);
        if pnum == prop as u16 {
            addr += hdr_len + data_len;
            let next_b0 = mem.read_byte(addr);
            if next_b0 == 0 {
                return 0;
            }
            let (next_pnum, _, _) = parse_prop_header(mem, addr);
            return next_pnum as u8;
        }
        addr += hdr_len + data_len;
    }
}

/// Every property number object `obj` provides, in the descending order ZMSD
/// §12.4.1 mandates — and it **stops** where the bytes stop descending instead
/// of walking forever.
///
/// This is the only safe way for a READER to enumerate an object's properties.
/// [`get_next_prop`] is the opcode (§12.4.3): it scans for the number it is
/// given and answers with the one after it, which on a table holding the same
/// number twice is that number again. `while prop != 0 { prop =
/// get_next_prop(…) }` then never terminates, and a corrupt object is not
/// hypothetical — **Sherlock r26/880127 object 308** lists property 43 twice
/// (`51, 50, 47, 46, 45, 44, 43, 43, 43, …`), which hung
/// [`ParseNames::detect`] on a retail game for as long as it was asked
/// (SQ-1143).
///
/// `get_next_prop` itself is deliberately left alone: the story's own parser
/// executes it and depends on its exact semantics, corrupt table or not. The
/// guard belongs on this side, where a reader is only ever asking a question.
pub fn property_numbers(mem: &Memory, obj: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut prop = get_next_prop(mem, obj, 0);
    // At most 63 property numbers exist in any version (§12.2), so a walk that
    // reaches 64 entries has already stopped describing a property table.
    while prop != 0 && out.len() < 64 {
        if out.last().is_some_and(|&last| prop >= last) {
            break;
        }
        out.push(prop);
        prop = get_next_prop(mem, obj, prop);
    }
    out
}

// ── Mapper snapshot ───────────────────────────────────────────────────────────

/// Stable per-object identity for the automapper.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ObjectSnapshot {
    pub number: u16,
    pub parent: u16,
    pub name: String,
}

/// The snapshot carries [`printed_name`], not [`short_name`]: what the mapper
/// puts on the map is what the STORY calls the room, which for an Inform 6 game
/// with a `short_name` property is that property and not the header name
/// (SQ-1372).
pub fn object_snapshot(mem: &Memory, obj: u16) -> ObjectSnapshot {
    ObjectSnapshot {
        number: obj,
        parent: get_parent(mem, obj),
        name: printed_name(mem, obj),
    }
}

// ── Parse names: what an object can be CALLED ────────────────────────────────
//
// `short_name` above answers what a story *prints* for an object. It does not
// answer what a player may TYPE for it, and the two are different sets: Zork
// I's "brass lantern" is reached by `lamp`, `lanter` and `light`, none of which
// is the printed name, and an Inform 7 object has no printed name at all.
//
// ── Where the words are, and why the property number is not a constant ───────
//
// Both compiler families store the words the same way — a property whose data
// is an array of 2-byte **dictionary entry addresses** — and disagree about
// which property that is.
//
//   * **Inform**, on both back-ends, always uses property 1. This is hard-coded
//     in the compiler rather than conventional: `Inform6/src/objects.c` reads
//     "A special rule applies to values in double-quotes of the built-in
//     property `name`, which always has number 1: such property values are
//     dictionary entries and not static strings", and
//     `objects_begin_pass()` seeds `commonprops[1]` as the first of three
//     predefined common properties before any user property is numbered.
//     Inform 7 keeps it: `inform7/runtime-module/Chapter 7/Name Properties.w`
//     — "the names of objects are parsed as nouns using the values of two
//     properties: `name`, a simple array of dictionary words, and `parse_name`,
//     a GPR function".
//
//   * **Infocom's own games** number ZIL's `SYNONYM` property per game, and the
//     numbers really do move. Measured here over the `stories/` corpus, one
//     story at a time: 14 in Seastalker, 17 in Zork II, Zork III, Suspect,
//     Hollywood Hijinx and the Zork I sampler, 18 in Zork I, Moonmist and
//     Wishbringer, 19 in Deadline, Enchanter, Planetfall, Infidel, Ballyhoo,
//     Cutthroats, The Witness and The Lurking Horror, 29 in Plundered Hearts,
//     31 in Spellbreaker, The Hitchhiker's Guide and Leather Goddesses, and
//     45–63 in the v4–v6 games. **Nothing in the image names it**: the game's
//     own parser reaches it through a constant compiled into its code.
//
// So the number is DETECTED for Infocom and KNOWN for Inform, and either way it
// travels inside [`ParseNames`] rather than being asked of the caller — a
// caller who guessed 1 on Zork I would get plausible garbage out of whatever
// property 1 happens to hold.
//
// ── Infocom adjectives: read from V4, and refused before it ──────────────────
//
// Every Infocom object keeps its ADJECTIVES in a SECOND property, and they are
// real parse words a player types: Zork Zero's "Dirigible Hangar" answers
// `hangar` under property 51 and `dirigible`/`large` under property 46; A Mind
// Forever Voyaging's park answers `park garden gardens common commons` under 52
// and `kennedy riverside halley church street small downtown old popular
// public` under 51. Inform has no such split — its adjectives sit in the same
// `name` array as its nouns — so this is an Infocom question only.
//
// **From V4 the second property holds dictionary addresses**, exactly like the
// nouns, and `infocom_properties` reads it: it is the runner-up in the same
// ranking that picks the nouns, and the CONTAINMENT test that separates them is
// what identifies it (an object cannot have adjectives without having nouns).
// Measured over `stories/`, all fifteen V4–V6 titles agree — Zork Zero p51/p46,
// Shogun p45/p32, Arthur p51/p45, Trinity p51/p49, A Mind Forever Voyaging
// p52/p51, Beyond Zork p49/p48, Sherlock p44/p43, Bureaucracy p55/p47, Nord and
// Bert p63/p50, Border Zone p51/p38, Wishbringer (r23) p50/p40, and the four
// v5 InvisiClues re-releases — with the runner-up covering 103 to 344 objects.
//
// **In V1–3 the second property holds one-byte adjective NUMBERS**, not
// addresses, resolved against the dictionary, where an adjective's entry
// carries the `DESC` flag ($20) with `DATA_FIRST` = `ADJ_FIRST` ($02) and holds
// its number in the first data byte — confirmed on Zork I, whose `brass` reads
// `flags=$22 d0=$dd` and whose brass lantern's property 16 is the single byte
// `$dd`. That property is NOT findable here and is not guessed at:
//
//   * A byte array is not a word array, so `candidate_properties` never sees
//     it at all — the runner-up on a V1–3 story is something else entirely, and
//     measurably noise. Zork I's is one object, Hitchhiker's one, Infidel's one,
//     Starcross's one, Suspended's one, Spellbreaker's two, Moonmist's four,
//     Seastalker's four; the leaders they sit beneath cover 136 to 246.
//     Reading those as adjectives would have answered `win` for Zork I's
//     kitchen window and `plate` for Seastalker's search light.
//   * Detecting the byte property directly has no margin worth trusting either:
//     any short property of small byte values matches "every byte is a known
//     adjective number", and the object-count and adjective-coverage rankings
//     pick DIFFERENT properties (Hitchhiker p29 has 131 objects covering 125 of
//     220 adjectives, p30 has 117 covering 193; Moonmist's p17, p18 and p19 are
//     within a whisker of one another).
//
// So the version gate in `infocom_properties` is load-bearing, and what a V1–3
// story reports is `Adjectives::Unavailable` — "this story cannot say" — and
// never an empty list, which would read as "this object has none". That
// distinction is the whole reason the V4+ half could ship without making a word
// list mean two things (SQ-1120).

/// Inform's `name` property, on every Inform story and both back-ends.
#[cfg(feature = "grammar")]
pub const INFORM_NAME_PROPERTY: u8 = 1;

/// A property must be an array of dictionary addresses on at least this many
/// objects before it is believed to be a parse-name property. Below it there is
/// no story here, only coincidence.
#[cfg(feature = "grammar")]
const MIN_AGREEING_OBJECTS: usize = 4;

/// …or, where the runner-up is not the adjectives, the leader must beat it by
/// this factor to be a story-wide convention rather than a coincidence.
#[cfg(feature = "grammar")]
const REQUIRED_MARGIN: usize = 2;

/// How many objects the object-entry table holds.
///
/// ZMSD §12.3 gives no count — the entries simply stop where the first property
/// table begins. Every entry names its own property table, and the lowest such
/// address is where the entries must end, so walking forwards while the next
/// entry still fits below the lowest address seen so far settles it in one pass
/// without trusting any single object.
pub fn object_count(mem: &Memory) -> u16 {
    let base = entries_base(mem);
    let esz = entry_size(mem.version());
    let poff = if mem.version() <= 3 { 7 } else { 12 };
    let mut end = mem.len() as u32;
    let mut n: u16 = 0;
    let mut i: u32 = 0;
    while base + (i + 1) * esz <= end && i < u16::MAX as u32 {
        let ptbl = mem.read_word(base + i * esz + poff) as u32;
        if ptbl > base && ptbl < end {
            end = ptbl;
        }
        i += 1;
        n = i as u16;
    }
    n
}

/// The reader for one story's parse names: which property holds them, and the
/// dictionary needed to turn the addresses in it back into words.
///
/// Built once per story ([`detect`](ParseNames::detect)) and then asked about
/// objects. It answers with [`ObjectWords`], which carries the object's number,
/// its printed name and its words together. Asking for the words alone is not
/// offered: a caller holding words without the name cannot say which thing they
/// belong to, and one holding the name without the words is offering a player
/// something the parser never agreed to accept.
#[derive(Debug, Clone)]
#[cfg(feature = "grammar")]
pub struct ParseNames {
    property: u8,
    adjective_property: Option<u8>,
    key_chars: usize,
    words: BTreeMap<u32, String>,
}

#[cfg(feature = "grammar")]
impl ParseNames {
    /// Work out where this story keeps its parse names, and refuse if it does
    /// not keep them anywhere readable.
    ///
    /// Inform's number is known from the compiler and only *verified* here.
    /// Infocom's is found by tallying, over the whole object table, which
    /// properties hold an array of dictionary addresses, and then taking the
    /// one whose objects **contain** every other candidate's — see
    /// [`candidate_properties`] for why that test and not a bigger count.
    ///
    /// `None` for a story with no parse names to read, which is a real answer
    /// and not only a failure. Journey and Scopa have no parser and no word
    /// arrays; `stories/advent.z8` — the 1993 port of the original Adventure,
    /// which boots and plays fine — implements its own tokeniser over its own
    /// word table and leaves the Z-machine dictionary declaring **zero
    /// entries**; `stories/ImpossibleStairs.z8` was built by **Dialog** (SQ-1101
    /// settled it from the `Dia` signature at header $39..$3B), which keeps its
    /// per-object data in arrays of its own rather than Z-machine properties —
    /// so there is no `name` property and no dictionary flag bytes to find. The
    /// Inform branch below is the one it takes, and refusing is what that branch
    /// then does: `MIN_AGREEING_OBJECTS` is why it does not adopt whatever
    /// property 1 happens to hold.
    pub fn detect(mem: &Memory) -> Option<ParseNames> {
        let index = Self::dictionary_index(mem);
        if index.by_address.is_empty() {
            return None;
        }
        let candidates = candidate_properties(mem, &index.by_address);
        let (property, adjective_property) = if grammar::detect_format(mem).is_inform() {
            // Known, not chosen — but still checked, so a story that is not
            // laid out the way its header claims refuses rather than reading
            // whatever property 1 happens to hold. Inform keeps adjectives in
            // that same array, so there is no second property to find.
            let agreeing = candidates.get(&INFORM_NAME_PROPERTY).map_or(0, BTreeSet::len);
            ((agreeing >= MIN_AGREEING_OBJECTS).then_some(INFORM_NAME_PROPERTY)?, None)
        } else {
            infocom_properties(&candidates, mem.version())?
        };
        Some(ParseNames {
            property,
            adjective_property,
            key_chars: index.key_chars,
            words: index.by_address,
        })
    }

    /// The same reader with the property number supplied instead of worked out,
    /// and nothing said about adjectives.
    ///
    /// For a story whose convention is known from outside — a disassembly, a
    /// reference dump — or to falsify [`detect`](ParseNames::detect) by asking
    /// for the wrong property and watching every object refuse. The documented
    /// default is [`INFORM_NAME_PROPERTY`]; nothing here assumes it.
    pub fn with_property(mem: &Memory, property: u8) -> Option<ParseNames> {
        Self::with_properties(mem, property, None)
    }

    /// The same, naming the adjective property too — the falsification handle
    /// for the adjective half, and the way to read a story whose second
    /// property is known from a disassembly.
    pub fn with_properties(
        mem: &Memory,
        property: u8,
        adjective_property: Option<u8>,
    ) -> Option<ParseNames> {
        let index = Self::dictionary_index(mem);
        if index.by_address.is_empty() {
            return None;
        }
        Some(ParseNames {
            property,
            adjective_property,
            key_chars: index.key_chars,
            words: index.by_address,
        })
    }

    /// Which property the words are read from.
    pub fn property(&self) -> u8 {
        self.property
    }

    /// Which property the ADJECTIVES are read from, and `None` where this story
    /// keeps none that can be read — see [`Adjectives`] for what that means and
    /// [`infocom_properties`] for how it is decided.
    pub fn adjective_property(&self) -> Option<u8> {
        self.adjective_property
    }

    /// What object `obj` is, and what it can be called — nouns always, and
    /// adjectives where [`adjective_property`](ParseNames::adjective_property)
    /// found somewhere to read them.
    ///
    /// `None` when the NOUNS cannot be read: see
    /// [`word_array`](ParseNames::word_array) for exactly when that is. An
    /// object whose adjectives cannot be read still answers — with an empty
    /// adjective list, which is a different claim from the
    /// [`Adjectives::Unavailable`] a story that keeps none reports.
    pub fn of(&self, mem: &Memory, obj: u16) -> Option<ObjectWords> {
        let words = self.word_array(mem, obj, self.property)?;
        let object = ObjectWords::new(
            u32::from(obj),
            short_name(mem, obj),
            words,
            Some(u32::from(self.property)),
            Some(self.key_chars),
        );
        match self.adjective_property {
            // No second property to read: the answer is that this story cannot
            // be asked, which `ObjectWords` reports as `Adjectives::Unavailable`
            // and never as an empty list.
            None => Some(object),
            // An object with no adjective property, or one whose adjective
            // property holds something that is not a word array, has NO
            // adjectives to offer — an empty list, not a refusal. The nouns
            // were read from a property this story keeps for that and are not
            // in doubt because a second one disagreed.
            Some(p) => Some(
                object.with_adjectives(
                    self.word_array(mem, obj, p).unwrap_or_default(),
                    u32::from(p),
                ),
            ),
        }
    }

    /// One property decoded as an array of dictionary addresses.
    ///
    /// `None` when the object has no such property, when its data is not a
    /// whole number of words, or when **any** entry in it is not the address of
    /// a dictionary word. That last one is the point: a property that is not a
    /// word array yields nothing, rather than a list of plausible-looking words
    /// decoded from arbitrary addresses.
    fn word_array(&self, mem: &Memory, obj: u16, property: u8) -> Option<Vec<String>> {
        let data = get_prop_addr(mem, obj, property);
        if data == 0 {
            return None;
        }
        let len = get_prop_len(mem, data) as u32;
        if len < 2 || !len.is_multiple_of(2) {
            return None;
        }
        let mut words = Vec::with_capacity(len as usize / 2);
        for i in 0..len / 2 {
            let addr = mem.read_word(data as u32 + i * 2) as u32;
            words.push(self.words.get(&addr)?.clone());
        }
        Some(words)
    }

    /// Every object that answers, in object-number order.
    pub fn all(&self, mem: &Memory) -> Vec<ObjectWords> {
        (1..=object_count(mem)).filter_map(|obj| self.of(mem, obj)).collect()
    }

    /// The first object, in object-number order, that `word` refers to.
    pub fn find(&self, mem: &Memory, word: &str) -> Option<ObjectWords> {
        (1..=object_count(mem)).filter_map(|obj| self.of(mem, obj)).find(|o| o.refers_to(word))
    }

    fn dictionary_index(mem: &Memory) -> DictionaryIndex {
        let dict = dictionary::load(mem);
        let by_address =
            grammar::dictionary_words(mem).into_iter().map(|w| (w.address, w.text)).collect();
        // §13.3/§13.4: a v1–3 key is 4 bytes holding 6 Z-characters, a v4+ key
        // 6 bytes holding 9. `Dictionary::key_len` is the byte figure; the
        // character figure is what truncates a player's word.
        let key_chars = dict.key_len() as usize / 2 * 3;
        DictionaryIndex { by_address, key_chars }
    }
}

/// The dictionary, indexed the way a parse-name property refers to it.
#[cfg(feature = "grammar")]
struct DictionaryIndex {
    by_address: BTreeMap<u32, String>,
    key_chars: usize,
}

/// Which objects hold an array of dictionary addresses under each property
/// number.
#[cfg(feature = "grammar")]
fn candidate_properties(
    mem: &Memory,
    by_address: &BTreeMap<u32, String>,
) -> BTreeMap<u8, BTreeSet<u16>> {
    let mut found: BTreeMap<u8, BTreeSet<u16>> = BTreeMap::new();
    for obj in 1..=object_count(mem) {
        for prop in property_numbers(mem, obj) {
            let data = get_prop_addr(mem, obj, prop);
            let len = get_prop_len(mem, data) as u32;
            if data == 0 || len < 2 || !len.is_multiple_of(2) {
                continue;
            }
            let all_words = (0..len / 2)
                .all(|i| by_address.contains_key(&(mem.read_word(data as u32 + i * 2) as u32)));
            if all_words {
                found.entry(prop).or_default().insert(obj);
            }
        }
    }
    found
}

/// Pick the Infocom parse-name property out of the candidates — and the
/// adjective property beside it where the story has a readable one — or refuse.
///
/// **The test is containment, not size.** Every Infocom game from V3 to V6
/// keeps its ADJECTIVES in a second property, and from V4 onwards those are
/// dictionary addresses too — so two properties are word arrays and the counts
/// alone are not decisive: Zork Zero leads 432 to 306, A Mind Forever Voyaging
/// 404 to 326, Trinity 450 to 344. But an object cannot have adjectives without
/// having nouns, and measured across the corpus the runner-up's objects are a
/// **strict subset** of the leader's in every single game — `|adj \ syn| = 0`
/// for Zork Zero, Arthur, Shogun, A Mind Forever Voyaging, Beyond Zork, Border
/// Zone, Bureaucracy, Nord and Bert, Trinity, Zork I, Moonmist, Seastalker and
/// The Hitchhiker's Guide, with the leader holding 57 to 242 objects the
/// runner-up does not.
///
/// Containment also settles the V6 games, which nothing else did: their
/// dictionary flags sit in the entry's last byte and mark almost nothing a
/// noun (Zork Zero: 24 of 1624 words, Shogun: 6 of 1389), so filtering the
/// candidate words by part of speech — which does separate the V4/V5 games
/// cleanly — leaves V6 with no candidates at all.
///
/// Containment is not the only way through, because the runner-up is not always
/// the adjectives. Planetfall's property 14 is a word array on twelve objects
/// that shares exactly one of them with property 19's 146 — some other list
/// entirely (object 254 answers `zzmgck` under 19 and `foo` under 14) — so it
/// is neither contained nor negligible by count alone. A leader that beats the
/// runner-up by [`REQUIRED_MARGIN`] is a story-wide convention next to
/// something that is not, and passes on that instead. The two tests are a
/// disjunction: Zork Zero leads 432 to 306 and needs containment, Planetfall
/// leads 146 to 12 and needs the margin, and neither test alone takes both.
///
/// Refuses when the leader does neither, or ties with the runner-up. Candidates
/// below the runner-up are not tested — that tail is noise, a handful of
/// objects whose two-byte property happens to hold a dictionary address, and
/// demanding anything of it refused six games these two tests settle.
///
/// **The same containment test names the adjectives, and only from V4.** Where
/// it is containment that let the leader through, the runner-up IS the adjective
/// property and is returned as one — but only on a V4+ story, because a V1–3
/// story keeps its adjectives as one-byte numbers that this scan can never see,
/// so its runner-up is something else. That distinction has teeth: eight V1–3
/// titles have a contained runner-up covering one to four objects, and reading
/// those as adjectives would answer `win` for Zork I's kitchen window. The
/// [`MIN_AGREEING_OBJECTS`] floor is a second guard on the same point rather
/// than a load-bearing one — every real V4+ adjective property covers 103
/// objects or more.
#[cfg(feature = "grammar")]
fn infocom_properties(
    candidates: &BTreeMap<u8, BTreeSet<u16>>,
    version: u8,
) -> Option<(u8, Option<u8>)> {
    let mut ranked: Vec<(&u8, &BTreeSet<u16>)> = candidates.iter().collect();
    // Largest first; ties broken by property number so the answer is stable.
    ranked.sort_by_key(|(p, s)| (std::cmp::Reverse(s.len()), **p));
    let (best, best_objects) = *ranked.first()?;
    if best_objects.len() < MIN_AGREEING_OBJECTS {
        return None;
    }
    let mut adjectives = None;
    if let Some((second, second_objects)) = ranked.get(1) {
        let contained = second_objects.is_subset(best_objects);
        let outnumbered =
            best_objects.len() >= second_objects.len().saturating_mul(REQUIRED_MARGIN);
        if second_objects.len() == best_objects.len() || !(contained || outnumbered) {
            return None;
        }
        // The runner-up is the ADJECTIVES exactly when it is contained (an
        // object cannot have adjectives without nouns) — and only from V4,
        // where they are dictionary addresses. See the module comment above for
        // why V1–3 cannot be answered here and what its runner-up really is.
        if version >= 4 && contained && second_objects.len() >= MIN_AGREEING_OBJECTS {
            adjectives = Some(**second);
        }
    }
    Some((*best, adjectives))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::text::encode::encode_word;

    // ── Shared layout for hand-built v3 object table ──────────────────────────
    //
    // sample_story places object_table at 0x0100.
    // v3 property-defaults: 31 words = 62 bytes → 0x0100..=0x013D
    // Object entries start at 0x013E.
    // Each v3 entry = 9 bytes:
    //   [0..3] attributes (4 bytes)
    //   [4]    parent
    //   [5]    sibling
    //   [6]    child
    //   [7..8] property-table address (word)
    //
    // We'll place property tables at 0x0200 for obj1, 0x0220 for obj2.
    //
    // Object 1 entry at 0x013E, Object 2 entry at 0x0147.

    const OBJ_TABLE: u32 = 0x0100;
    const ENTRIES_V3: u32 = OBJ_TABLE + 31 * 2; // = 0x013E
    const OBJ1_ENTRY: u32 = ENTRIES_V3;          // 0x013E
    const OBJ2_ENTRY: u32 = ENTRIES_V3 + 9;      // 0x0147
    const OBJ3_ENTRY: u32 = ENTRIES_V3 + 18;     // 0x0150

    const PROP1_TBL: u32 = 0x0200; // property table for object 1
    const PROP2_TBL: u32 = 0x0220; // property table for object 2
    const PROP3_TBL: u32 = 0x0230; // property table for object 3

    /// Write `val` as a big-endian word into a byte buffer at `offset`.
    fn put_word(buf: &mut [u8], offset: usize, val: u16) {
        buf[offset] = (val >> 8) as u8;
        buf[offset + 1] = (val & 0xFF) as u8;
    }

    /// Encode a Z-string for a property table name.
    /// Returns the bytes to write (2 bytes per encoded word).
    fn z_name(text: &str) -> Vec<u8> {
        // We'll just use encode_word as a proxy for a short Z-string.
        // For version 3 encoding (6 Z-chars = 4 bytes = 2 words).
        encode_word(text, 3)
    }

    /// Build a v3 story buffer with a small 2-object tree:
    ///   obj2 is child of obj1; obj3 is sibling of obj2.
    ///   obj1 has attr 0 set; obj2 has attr 7, 8 set (cross-byte boundary).
    fn build_v3_story() -> Vec<u8> {
        let mut buf = sample_story(3);

        // ── Object 1 entry ───────────────────────────────────────────────────
        // attr 0 set: byte 0 = 0x80
        buf[OBJ1_ENTRY as usize] = 0x80; // attr 0 set
        buf[OBJ1_ENTRY as usize + 1] = 0x00;
        buf[OBJ1_ENTRY as usize + 2] = 0x00;
        buf[OBJ1_ENTRY as usize + 3] = 0x00;
        // parent=0 sibling=0 child=2
        buf[OBJ1_ENTRY as usize + 4] = 0; // parent
        buf[OBJ1_ENTRY as usize + 5] = 0; // sibling
        buf[OBJ1_ENTRY as usize + 6] = 2; // child
        // property table at PROP1_TBL
        put_word(&mut buf, OBJ1_ENTRY as usize + 7, PROP1_TBL as u16);

        // ── Object 2 entry ───────────────────────────────────────────────────
        // attr 7 = last bit of byte 0: 0x01
        // attr 8 = first bit of byte 1: 0x80
        buf[OBJ2_ENTRY as usize] = 0x01;  // attr 7 set
        buf[OBJ2_ENTRY as usize + 1] = 0x80; // attr 8 set
        buf[OBJ2_ENTRY as usize + 2] = 0x00;
        buf[OBJ2_ENTRY as usize + 3] = 0x00;
        buf[OBJ2_ENTRY as usize + 4] = 1; // parent = obj1
        buf[OBJ2_ENTRY as usize + 5] = 3; // sibling = obj3
        buf[OBJ2_ENTRY as usize + 6] = 0; // child = none
        put_word(&mut buf, OBJ2_ENTRY as usize + 7, PROP2_TBL as u16);

        // ── Object 3 entry ───────────────────────────────────────────────────
        buf[OBJ3_ENTRY as usize] = 0x00;
        buf[OBJ3_ENTRY as usize + 1] = 0x00;
        buf[OBJ3_ENTRY as usize + 2] = 0x00;
        buf[OBJ3_ENTRY as usize + 3] = 0x00;
        buf[OBJ3_ENTRY as usize + 4] = 1; // parent = obj1
        buf[OBJ3_ENTRY as usize + 5] = 0; // sibling = none
        buf[OBJ3_ENTRY as usize + 6] = 0; // child = none
        put_word(&mut buf, OBJ3_ENTRY as usize + 7, PROP3_TBL as u16);

        // ── Property table for obj1 ──────────────────────────────────────────
        // name: 2 words of Z-text for "west" (4 bytes from encode_word v3).
        let name1 = z_name("west");
        assert_eq!(name1.len(), 4); // 2 Z-words
        buf[PROP1_TBL as usize] = 2; // 2 words of name
        buf[PROP1_TBL as usize + 1..PROP1_TBL as usize + 5].copy_from_slice(&name1);
        // Property 10: 2 bytes of data = 0xABCD
        // size byte: bits 7-5 = (2-1)=1, bits 4-0 = 10 → 0b001_01010 = 0x2A
        buf[PROP1_TBL as usize + 5] = 0x2A; // size byte: size 2, prop 10
        buf[PROP1_TBL as usize + 6] = 0xAB;
        buf[PROP1_TBL as usize + 7] = 0xCD;
        // Property 5: 1 byte of data = 0x42
        // size byte: bits 7-5 = (1-1)=0, bits 4-0 = 5 → 0b000_00101 = 0x05
        buf[PROP1_TBL as usize + 8] = 0x05; // size byte: size 1, prop 5
        buf[PROP1_TBL as usize + 9] = 0x42;
        // End sentinel
        buf[PROP1_TBL as usize + 10] = 0x00;

        // ── Property table for obj2 ──────────────────────────────────────────
        let name2 = z_name("east");
        buf[PROP2_TBL as usize] = 2;
        buf[PROP2_TBL as usize + 1..PROP2_TBL as usize + 5].copy_from_slice(&name2);
        // End sentinel (no properties)
        buf[PROP2_TBL as usize + 5] = 0x00;

        // ── Property table for obj3 ──────────────────────────────────────────
        let name3 = z_name("hall");
        buf[PROP3_TBL as usize] = 2;
        buf[PROP3_TBL as usize + 1..PROP3_TBL as usize + 5].copy_from_slice(&name3);
        buf[PROP3_TBL as usize + 5] = 0x00;

        // Property defaults: set prop 10's default to 0x1234 (0-indexed: prop 10 is at offset (10-1)*2 = 18).
        put_word(&mut buf, OBJ_TABLE as usize + (10 - 1) * 2, 0x1234);

        buf
    }

    // ── v3 attribute tests ────────────────────────────────────────────────────

    #[test]
    fn v3_get_attr_set_on_construction() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        assert!(get_attr(&m, 1, 0));   // obj1 attr 0
        assert!(!get_attr(&m, 1, 1));  // obj1 attr 1 clear
    }

    #[test]
    fn v3_set_and_clear_attr() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        assert!(!get_attr(&m, 1, 3));
        set_attr(&mut m, 1, 3);
        assert!(get_attr(&m, 1, 3));
        clear_attr(&mut m, 1, 3);
        assert!(!get_attr(&m, 1, 3));
    }

    #[test]
    fn v3_attr_cross_byte_boundary() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj2: attr 7 (byte 0, bit 0) and attr 8 (byte 1, bit 7)
        assert!(get_attr(&m, 2, 7));
        assert!(get_attr(&m, 2, 8));
        assert!(!get_attr(&m, 2, 6));
        assert!(!get_attr(&m, 2, 9));
    }

    #[test]
    fn v3_set_attr_at_boundary() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        assert!(!get_attr(&m, 2, 9));
        set_attr(&mut m, 2, 9);
        assert!(get_attr(&m, 2, 9));
        // attr 8 should still be set
        assert!(get_attr(&m, 2, 8));
    }

    // ── v3 tree pointer tests ─────────────────────────────────────────────────

    #[test]
    fn v3_object_entry_addr_matches_layout_formula() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // Object 1's entry is at entries_base; object 2 one entry_size later.
        assert_eq!(object_entry_addr(&m, 1), entries_base(&m));
        assert_eq!(
            object_entry_addr(&m, 2),
            entries_base(&m) + entry_size(m.version())
        );
        assert_eq!(object_entry_addr(&m, 1), OBJ1_ENTRY);
        assert_eq!(object_entry_addr(&m, 2), OBJ2_ENTRY);
    }

    #[test]
    fn v3_tree_pointers() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_parent(&m, 1), 0);
        assert_eq!(get_child(&m, 1), 2);
        assert_eq!(get_parent(&m, 2), 1);
        assert_eq!(get_sibling(&m, 2), 3);
        assert_eq!(get_parent(&m, 3), 1);
        assert_eq!(get_sibling(&m, 3), 0);
    }

    // ── v3 insert_obj / remove_obj ────────────────────────────────────────────

    #[test]
    fn v3_remove_obj_first_child() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        // Remove obj2 (first child of obj1); obj3 should become first child.
        assert!(remove_obj(&mut m, 2));
        assert_eq!(get_child(&m, 1), 3);
        assert_eq!(get_parent(&m, 2), 0);
        assert_eq!(get_sibling(&m, 2), 0);
        // obj3 parent still 1
        assert_eq!(get_parent(&m, 3), 1);
    }

    #[test]
    fn v3_remove_obj_later_sibling() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        // Remove obj3 (sibling of obj2): obj2's sibling should become 0.
        assert!(remove_obj(&mut m, 3));
        assert_eq!(get_sibling(&m, 2), 0);
        assert_eq!(get_parent(&m, 3), 0);
    }

    #[test]
    fn v3_insert_obj_makes_first_child() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        // Insert obj3 into obj2 (obj3 currently child of obj1, sibling of obj2).
        assert!(insert_obj(&mut m, 3, 2));
        // obj3 is now first child of obj2
        assert_eq!(get_child(&m, 2), 3);
        assert_eq!(get_parent(&m, 3), 2);
        // obj3's sibling should be 0 (obj2 had no children before)
        assert_eq!(get_sibling(&m, 3), 0);
        // obj1's child list: obj2 (obj3 was removed from the sibling chain)
        assert_eq!(get_child(&m, 1), 2);
        assert_eq!(get_sibling(&m, 2), 0); // obj3 is gone from obj1's sibling list
    }

    // ── v3 short_name ─────────────────────────────────────────────────────────

    #[test]
    fn v3_short_name_obj1() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let name = short_name(&m, 1);
        assert!(name.starts_with("west"), "got: {:?}", name);
    }

    #[test]
    fn v3_short_name_obj2() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let name = short_name(&m, 2);
        assert!(name.starts_with("east"), "got: {:?}", name);
    }

    #[test]
    fn short_name_span_is_the_bytes_short_name_actually_decodes() {
        // The span must be the name's own Z-text and nothing else: it starts one
        // byte past the property table (§12.4's length field) and covers exactly
        // the words that field counts. Round-trip it through the decoder — the
        // span's end must be where decoding from its start stops.
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        for obj in [1u16, 2] {
            let (start, end) = short_name_span(&m, obj).expect("both objects are named");
            assert!(end > start && (end - start) % 2 == 0, "whole 2-byte words only");
            let (text, decoded_end) = decode_string(&m, start);
            assert_eq!(decoded_end, end, "object {obj}'s declared span ends where its text does");
            assert_eq!(text, short_name(&m, obj), "…and holds exactly its short name");
        }
        assert_eq!(short_name_span(&m, 0), None, "object 0 is the null object");
    }

    /// §12.3 vs §12.4: the entry is flags, tree links and a POINTER; the text
    /// lives at the other end of that pointer. `object_prop_table_addr` returns
    /// the pointed-at header, whose first byte is the name's word count and
    /// whose second byte begins the name — so a reader landing there sees both.
    #[test]
    fn object_prop_table_addr_is_where_the_short_name_lives_not_the_entry() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        for obj in [1u16, 2, 3] {
            let ptbl = object_prop_table_addr(&m, obj).expect("a real object");
            assert_ne!(ptbl, object_entry_addr(&m, obj), "the table is not the entry");
            // §12.4: byte 0 is the name's length in 2-byte words, name at +1.
            let words = m.read_byte(ptbl) as u32;
            assert_eq!(
                short_name_span(&m, obj),
                (words > 0).then(|| (ptbl + 1, ptbl + 1 + words * 2)),
                "object {obj}'s name starts one byte past the table it points at",
            );
            // The entry is 9 bytes of §12.3 fields and holds none of that text.
            let entry = object_entry_addr(&m, obj);
            let (start, _) = short_name_span(&m, obj).expect("all three are named");
            assert!(
                start < entry || start >= entry + entry_size(m.version()),
                "object {obj}'s name is nowhere inside its entry",
            );
        }
        assert_eq!(object_prop_table_addr(&m, 0), None, "object 0 is the null object");
    }

    /// A name of zero words is legal (§12.4: "the text-length may be 0"). The
    /// table address is still the right place to land — the length byte is
    /// there to be read, even though `short_name_span` has nothing to offer.
    #[test]
    fn object_prop_table_addr_answers_for_an_object_with_no_short_name() {
        let mut buf = build_v3_story();
        buf[PROP3_TBL as usize] = 0; // obj3's name shrinks to nothing
        let m = Memory::new(buf).unwrap();
        assert_eq!(object_prop_table_addr(&m, 3), Some(PROP3_TBL));
        assert_eq!(short_name(&m, 3), "", "no name to decode");
        assert_eq!(short_name_span(&m, 3), None, "…and so no span");
    }

    // ── v3 property tests ─────────────────────────────────────────────────────

    #[test]
    fn v3_get_prop_present() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj1 prop 10 = 0xABCD (2 bytes)
        assert_eq!(get_prop(&m, 1, 10), 0xABCD);
    }

    #[test]
    fn v3_get_prop_one_byte() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj1 prop 5 = 0x42 (1 byte)
        assert_eq!(get_prop(&m, 1, 5), 0x42);
    }

    #[test]
    fn v3_get_prop_defaults_fallback() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj2 has no properties; prop 10 falls back to default 0x1234
        assert_eq!(get_prop(&m, 2, 10), 0x1234);
    }

    #[test]
    fn v3_put_prop_two_bytes() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        assert!(put_prop(&mut m, 1, 10, 0x5678));
        assert_eq!(get_prop(&m, 1, 10), 0x5678);
    }

    #[test]
    fn v3_put_prop_one_byte() {
        let buf = build_v3_story();
        let mut m = Memory::new(buf).unwrap();
        assert!(put_prop(&mut m, 1, 5, 0xFF));
        assert_eq!(get_prop(&m, 1, 5), 0xFF);
    }

    #[test]
    fn v3_get_prop_addr() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let addr = get_prop_addr(&m, 1, 10);
        assert_ne!(addr, 0);
        assert_eq!(m.read_word(addr as u32), 0xABCD);
    }

    #[test]
    fn v3_get_prop_addr_absent() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // obj2 has no properties
        assert_eq!(get_prop_addr(&m, 2, 10), 0);
    }

    #[test]
    fn v3_get_prop_len() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let addr = get_prop_addr(&m, 1, 10);
        assert_eq!(get_prop_len(&m, addr), 2);
        let addr5 = get_prop_addr(&m, 1, 5);
        assert_eq!(get_prop_len(&m, addr5), 1);
    }

    #[test]
    fn v3_get_next_prop() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        // First prop of obj1 (prop=0 → first)
        let first = get_next_prop(&m, 1, 0);
        assert_eq!(first, 10); // highest prop first
        let next = get_next_prop(&m, 1, 10);
        assert_eq!(next, 5);
        let end = get_next_prop(&m, 1, 5);
        assert_eq!(end, 0); // no more
    }

    /// SQ-1143. A property table listing the same number twice makes
    /// `get_next_prop` answer with that number again — it looks for the entry
    /// it was given and returns the one after it — so `while prop != 0 { prop =
    /// get_next_prop(…) }` never terminates. Sherlock r26/880127 object 308 is
    /// such a table on retail media, and it hung `ParseNames::detect` for as
    /// long as anything asked.
    ///
    /// `get_next_prop` is the OPCODE and keeps its behaviour; the guard is on
    /// the reader's side, where `property_numbers` stops at the first number
    /// that does not descend (§12.4.1 mandates descending order).
    #[test]
    fn property_numbers_stops_where_a_table_repeats_a_number_instead_of_cycling() {
        let mut buf = build_v3_story();
        // Object 1's table is prop 10 (2 bytes) then prop 5 (1 byte). Rewrite
        // the second entry's number to 10 as well, so the table reads 10, 10.
        buf[PROP1_TBL as usize + 8] = 0x0A; // size byte: size 1, prop 10
        let m = Memory::new(buf).unwrap();
        // The opcode itself still does exactly what §12.4.3 says, and that is
        // what makes the naive walk spin.
        assert_eq!(get_next_prop(&m, 1, 0), 10);
        assert_eq!(get_next_prop(&m, 1, 10), 10, "the second 10 is 'the one after' the first");
        // The reader stops.
        assert_eq!(property_numbers(&m, 1), [10]);
        // An honest descending table is walked in full and unchanged.
        let good = Memory::new(build_v3_story()).unwrap();
        assert_eq!(property_numbers(&good, 1), [10, 5]);
        assert_eq!(property_numbers(&good, 2), [], "an object with no properties");
    }

    #[test]
    fn get_prop_zero_returns_benign_zero() {
        // Property 0 does not exist (property numbers are 1-based, ZMSD
        // §12.4.1) — get_prop 0 is illegal game code. Pre-fix the defaults
        // fallback computed (0 - 1) * 2, underflowing u32 (debug panic /
        // wild read in release). (SQ-0619)
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_prop(&m, 1, 0), 0, "obj with props");
        assert_eq!(get_prop(&m, 2, 0), 0, "obj without props");
    }

    #[test]
    fn remove_obj_detects_sibling_cycle_instead_of_hanging() {
        // Corrupted table: obj2.sibling=3 (as built) and obj3.sibling=2 form a
        // cycle; obj1's parent is set to obj1 so the predecessor walk over
        // obj1's child chain (2→3→2→…) never finds obj1 or a 0. Pre-fix this
        // looped forever. (SQ-0619)
        let mut buf = build_v3_story();
        buf[OBJ3_ENTRY as usize + 5] = 2; // obj3.sibling = 2 → cycle
        buf[OBJ1_ENTRY as usize + 4] = 1; // obj1.parent = 1 (itself)
        let mut m = Memory::new(buf).unwrap();
        assert!(!remove_obj(&mut m, 1), "cycle must be reported, not spun on");
    }

    // ── v3 snapshot ──────────────────────────────────────────────────────────

    #[test]
    fn v3_object_snapshot() {
        let buf = build_v3_story();
        let m = Memory::new(buf).unwrap();
        let snap = object_snapshot(&m, 2);
        assert_eq!(snap.number, 2);
        assert_eq!(snap.parent, 1);
        assert!(snap.name.starts_with("east"), "got: {:?}", snap.name);
    }

    // ── v5 layout tests ───────────────────────────────────────────────────────
    //
    // sample_story(5) uses object_table = 0x0100.
    // v5 property-defaults: 63 words = 126 bytes → entries at 0x0100 + 0x7E = 0x017E.
    // Each v5 entry = 14 bytes:
    //   [0..5]   attributes (6 bytes)
    //   [6..7]   parent (word)
    //   [8..9]   sibling (word)
    //   [10..11] child (word)
    //   [12..13] property-table address (word)

    const ENTRIES_V5: u32 = OBJ_TABLE + 63 * 2; // = 0x017E
    const V5_OBJ1_ENTRY: u32 = ENTRIES_V5;        // 0x017E
    const V5_OBJ2_ENTRY: u32 = ENTRIES_V5 + 14;   // 0x018C
    const V5_OBJ3_ENTRY: u32 = ENTRIES_V5 + 28;   // 0x019A

    const V5_PROP1_TBL: u32 = 0x0280;
    const V5_PROP2_TBL: u32 = 0x02A0;
    const V5_PROP3_TBL: u32 = 0x02C0;

    fn build_v5_story() -> Vec<u8> {
        let mut buf = sample_story(5);

        // ── Object 1 entry ───────────────────────────────────────────────────
        // attr 0 set: byte 0 = 0x80
        buf[V5_OBJ1_ENTRY as usize] = 0x80;
        for i in 1..6 {
            buf[V5_OBJ1_ENTRY as usize + i] = 0;
        }
        // parent=0, sibling=0, child=2 (words)
        put_word(&mut buf, V5_OBJ1_ENTRY as usize + 6, 0);   // parent
        put_word(&mut buf, V5_OBJ1_ENTRY as usize + 8, 0);   // sibling
        put_word(&mut buf, V5_OBJ1_ENTRY as usize + 10, 2);  // child
        put_word(&mut buf, V5_OBJ1_ENTRY as usize + 12, V5_PROP1_TBL as u16);

        // ── Object 2 entry ───────────────────────────────────────────────────
        // attr 47 = last bit of byte 5 (bit 0) for v5 (48 attrs, 6 bytes)
        // attr 47: byte = 47/8 = 5, bit = 7 - (47%8) = 7 - 7 = 0 → 0x01
        buf[V5_OBJ2_ENTRY as usize] = 0x00;
        for i in 1..5 {
            buf[V5_OBJ2_ENTRY as usize + i] = 0;
        }
        buf[V5_OBJ2_ENTRY as usize + 5] = 0x01; // attr 47 set
        put_word(&mut buf, V5_OBJ2_ENTRY as usize + 6, 1);   // parent = obj1
        put_word(&mut buf, V5_OBJ2_ENTRY as usize + 8, 0);   // sibling = none
        put_word(&mut buf, V5_OBJ2_ENTRY as usize + 10, 0);  // child = none
        put_word(&mut buf, V5_OBJ2_ENTRY as usize + 12, V5_PROP2_TBL as u16);

        // ── Property table for obj1 (v5 two-byte property header) ────────────
        // name: "west" encoded (2 Z-words for v3 encoding; we use same 4-byte output)
        let name1 = encode_word("west", 5); // 6 bytes (3 Z-words) for v5
        let name_words = (name1.len() / 2) as u8; // 3 words
        buf[V5_PROP1_TBL as usize] = name_words;
        buf[V5_PROP1_TBL as usize + 1..V5_PROP1_TBL as usize + 1 + name1.len()]
            .copy_from_slice(&name1);
        let after_name = V5_PROP1_TBL as usize + 1 + name1.len();

        // Property 10, 2-byte data, using v5 one-byte form (bit 7 clear):
        // one-byte form: bit 6 set → 2 bytes of data; bits 5-0 = prop num 10 = 0b00_001010
        // so: bit7=0, bit6=1, bits5-0=10 → 0b0100_1010 = 0x4A
        buf[after_name] = 0x4A; // v5 one-byte size: 2 data bytes, prop 10
        buf[after_name + 1] = 0xAB;
        buf[after_name + 2] = 0xCD;
        // Property 5, 1-byte data, v5 one-byte form:
        // bit7=0, bit6=0, bits5-0=5 → 0b0000_0101 = 0x05
        buf[after_name + 3] = 0x05;
        buf[after_name + 4] = 0x42;
        buf[after_name + 5] = 0x00; // sentinel

        // ── Property table for obj2 ──────────────────────────────────────────
        let name2 = encode_word("east", 5);
        let name_words2 = (name2.len() / 2) as u8;
        buf[V5_PROP2_TBL as usize] = name_words2;
        buf[V5_PROP2_TBL as usize + 1..V5_PROP2_TBL as usize + 1 + name2.len()]
            .copy_from_slice(&name2);
        let after_name2 = V5_PROP2_TBL as usize + 1 + name2.len();
        buf[after_name2] = 0x00; // sentinel

        // Property default for prop 10: 0x9999 (63-word table, prop 10 at offset (10-1)*2=18)
        put_word(&mut buf, OBJ_TABLE as usize + (10 - 1) * 2, 0x9999);

        buf
    }

    #[test]
    fn v5_get_attr_set() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert!(get_attr(&m, 1, 0));
        assert!(!get_attr(&m, 1, 1));
    }

    #[test]
    fn v5_attr_47() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert!(get_attr(&m, 2, 47));
        assert!(!get_attr(&m, 2, 46));
    }

    #[test]
    fn v5_set_clear_attr() {
        let buf = build_v5_story();
        let mut m = Memory::new(buf).unwrap();
        assert!(!get_attr(&m, 1, 31));
        set_attr(&mut m, 1, 31);
        assert!(get_attr(&m, 1, 31));
        clear_attr(&mut m, 1, 31);
        assert!(!get_attr(&m, 1, 31));
    }

    #[test]
    fn v5_tree_pointers() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_parent(&m, 1), 0);
        assert_eq!(get_child(&m, 1), 2);
        assert_eq!(get_parent(&m, 2), 1);
        assert_eq!(get_sibling(&m, 2), 0);
    }

    #[test]
    fn v5_insert_obj() {
        let buf = build_v5_story();
        let mut m = Memory::new(buf).unwrap();
        // obj2 is child of obj1; insert obj2 into obj1 again (no-op tree-wise but exercises path)
        // More useful: insert obj2 into itself? No. Let's just test remove then re-insert.
        assert!(remove_obj(&mut m, 2));
        assert_eq!(get_child(&m, 1), 0);
        assert!(insert_obj(&mut m, 2, 1));
        assert_eq!(get_child(&m, 1), 2);
        assert_eq!(get_parent(&m, 2), 1);
    }

    #[test]
    fn v5_short_name() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        let name = short_name(&m, 1);
        assert!(name.starts_with("west"), "got: {:?}", name);
    }

    #[test]
    fn v5_get_prop_present() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_prop(&m, 1, 10), 0xABCD);
    }

    #[test]
    fn v5_get_prop_one_byte() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_prop(&m, 1, 5), 0x42);
    }

    #[test]
    fn v5_get_prop_defaults_fallback() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_prop(&m, 2, 10), 0x9999);
    }

    #[test]
    fn v5_put_prop() {
        let buf = build_v5_story();
        let mut m = Memory::new(buf).unwrap();
        assert!(put_prop(&mut m, 1, 10, 0xDEAD));
        assert_eq!(get_prop(&m, 1, 10), 0xDEAD);
    }

    #[test]
    fn v5_get_prop_addr_absent() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        assert_eq!(get_prop_addr(&m, 2, 10), 0);
    }

    #[test]
    fn v5_get_next_prop() {
        let buf = build_v5_story();
        let m = Memory::new(buf).unwrap();
        let first = get_next_prop(&m, 1, 0);
        assert_eq!(first, 10);
        let next = get_next_prop(&m, 1, 10);
        assert_eq!(next, 5);
        let end = get_next_prop(&m, 1, 5);
        assert_eq!(end, 0);
    }

    // ── v5 two-byte property header tests ────────────────────────────────────
    //
    // Builds on build_v5_story() by wiring up obj3 (at V5_OBJ3_ENTRY) with a
    // property table that uses the two-byte header form (ZMSD §12.4.2):
    //
    //   first byte:  0x80 | prop_num  (bit 7 set → two-byte form; bits 5-0 = prop num)
    //   second byte: 0x80 | size      (bit 7 set; bits 5-0 = data length; 0 → 64)
    //
    // Properties (descending order):
    //   prop 20: 4 bytes data (0xDEAD_BEEF)  — normal case, expect get_prop_len == 4
    //   prop 15: 64 bytes data (zeroed)       — escape case (size bits == 0), expect 64

    fn build_v5_two_byte_story() -> Vec<u8> {
        let mut buf = build_v5_story();

        // ── Object 3 entry (v5, 14 bytes) ───────────────────────────────────
        for i in 0..6 {
            buf[V5_OBJ3_ENTRY as usize + i] = 0; // no attributes
        }
        put_word(&mut buf, V5_OBJ3_ENTRY as usize + 6, 0);  // parent = none
        put_word(&mut buf, V5_OBJ3_ENTRY as usize + 8, 0);  // sibling = none
        put_word(&mut buf, V5_OBJ3_ENTRY as usize + 10, 0); // child = none
        put_word(&mut buf, V5_OBJ3_ENTRY as usize + 12, V5_PROP3_TBL as u16);

        // ── Property table for obj3 ──────────────────────────────────────────
        // Short name: "room" encoded for v5 (6 bytes, 3 Z-words)
        let name = encode_word("room", 5);
        let name_words = (name.len() / 2) as u8; // 3 words
        let base = V5_PROP3_TBL as usize;
        buf[base] = name_words;
        buf[base + 1..base + 1 + name.len()].copy_from_slice(&name);
        let mut p = base + 1 + name.len(); // first property byte offset

        // Property 20, 4 bytes data: two-byte header
        //   byte 0: 0x80 | 20 = 0x94
        //   byte 1: 0x80 | 4  = 0x84  (bits 5-0 = 4 → data length 4)
        buf[p]     = 0x94; // first size byte: two-byte form, prop 20
        buf[p + 1] = 0x84; // second size byte: length 4
        buf[p + 2] = 0xDE;
        buf[p + 3] = 0xAD;
        buf[p + 4] = 0xBE;
        buf[p + 5] = 0xEF;
        p += 6; // 2 header bytes + 4 data bytes

        // Property 15, 64 bytes data: two-byte header with length-escape
        //   byte 0: 0x80 | 15 = 0x8F
        //   byte 1: 0x80 | 0  = 0x80  (bits 5-0 = 0 → data length 64)
        buf[p]     = 0x8F; // first size byte: two-byte form, prop 15
        buf[p + 1] = 0x80; // second size byte: length escape → 64
        // 64 bytes of data (zeroed — the buffer is already 0 there)
        p += 2 + 64;

        // End sentinel
        buf[p] = 0x00;

        buf
    }

    #[test]
    fn v5_two_byte_property_header() {
        let buf = build_v5_two_byte_story();
        let m = Memory::new(buf).unwrap();

        // ── Normal case: prop 20, 4 bytes ────────────────────────────────────
        let addr20 = get_prop_addr(&m, 3, 20);
        assert_ne!(addr20, 0, "prop 20 should be present in obj3");
        // Data address must point just past the two size bytes.
        // The two size bytes are at addr20-2 (first) and addr20-1 (second).
        assert_eq!(m.read_byte(addr20 as u32 - 2), 0x94, "first size byte for prop 20");
        assert_eq!(m.read_byte(addr20 as u32 - 1), 0x84, "second size byte for prop 20");
        // get_prop_addr returns address of DATA, not header.
        assert_eq!(m.read_byte(addr20 as u32),     0xDE);
        assert_eq!(m.read_byte(addr20 as u32 + 1), 0xAD);
        assert_eq!(m.read_byte(addr20 as u32 + 2), 0xBE);
        assert_eq!(m.read_byte(addr20 as u32 + 3), 0xEF);
        // get_prop_len reads the byte immediately before prop_addr; for two-byte
        // form that is the SECOND size byte (0x84), so len = 0x84 & 0x3F = 4.
        assert_eq!(get_prop_len(&m, addr20), 4, "normal two-byte prop len should be 4");

        // ── Escape case: prop 15, 64 bytes ───────────────────────────────────
        let addr15 = get_prop_addr(&m, 3, 15);
        assert_ne!(addr15, 0, "prop 15 should be present in obj3");
        assert_eq!(m.read_byte(addr15 as u32 - 2), 0x8F, "first size byte for prop 15");
        assert_eq!(m.read_byte(addr15 as u32 - 1), 0x80, "second size byte for prop 15");
        // get_prop_len: second size byte = 0x80; 0x80 & 0x3F = 0 → returns 64.
        assert_eq!(get_prop_len(&m, addr15), 64, "escape two-byte prop len should be 64");
    }

    // ── The name the story PRINTS (SQ-1372) ──────────────────────────────────
    //
    // A v5 story with Inform's own two tables in it: an identifier-names table
    // (`Inform6/tables.c`: a word count, then one packed string address per
    // property number) and two objects using the `short_name` property it
    // names — one holding a STRING, one holding a ROUTINE.

    /// Where the hand-built v5 story's pieces go. Dynamic memory ends at
    /// `static_mem_base` = 0x0400 (`sample_story`), and the identifiers table
    /// must be inside `[object_table, static_mem_base)` exactly as Inform puts
    /// it, so it goes at 0x0350; the strings go above, where a packed address
    /// (v5: `4 * p`, ZMSD §1.2.3) can reach them.
    const IDENT_TABLE: u32 = 0x0350;
    /// How many identifier entries the table claims.
    const IDENT_COUNT: u32 = 40;
    /// The property number this story gives `short_name` — deliberately NOT the
    /// 46 `advent.z6` uses, since the whole point is that it is read.
    const SHORT_NAME_PROP: u32 = 30;
    const V5_OBJ_ENTRIES: u32 = 0x0100 + 63 * 2;
    const V5_PROPS_1: u32 = 0x0200;
    const V5_PROPS_2: u32 = 0x0220;
    const V5_STRINGS: u32 = 0x0400;

    /// Encode `text` as a Z-string (ZMSD §3.2): three 5-bit Z-characters per
    /// word, 0x8000 on the last. Written from the spec rather than through
    /// [`encode_word`], which pads to the DICTIONARY's fixed length.
    fn z_string(text: &str) -> Vec<u8> {
        let mut z: Vec<u8> = Vec::new();
        for ch in text.bytes() {
            if let Some(i) = crate::text::A0.iter().position(|&c| c == ch) {
                z.push(6 + i as u8);
            } else if let Some(i) = crate::text::A1.iter().position(|&c| c == ch) {
                z.push(4);
                z.push(6 + i as u8);
            } else if let Some(i) = crate::text::A2.iter().position(|&c| c == ch) {
                z.push(5);
                z.push(6 + i as u8);
            } else {
                panic!("z_string cannot encode {ch:?}");
            }
        }
        while !z.len().is_multiple_of(3) {
            z.push(5); // §3.6.1's padding: a shift with nothing after it
        }
        let mut out = Vec::new();
        let words = z.len() / 3;
        for (w, c) in z.chunks(3).enumerate() {
            let mut word = ((c[0] as u16) << 10) | ((c[1] as u16) << 5) | c[2] as u16;
            if w + 1 == words {
                word |= 0x8000;
            }
            out.extend_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// A v5 story whose objects and identifier names are laid out per the ZMSD
    /// and per `Inform6/tables.c`. Object 1 has an EMPTY header name and a
    /// string `short_name`; object 2 has a header name and a ROUTINE one.
    fn build_short_name_story() -> Vec<u8> {
        let mut buf = sample_story(5);
        buf.resize(0x0600, 0);

        // Strings, four-byte aligned so a packed address reaches them.
        let mut next = V5_STRINGS;
        let mut put_string = |buf: &mut Vec<u8>, text: &str| -> u16 {
            let at = next;
            let bytes = z_string(text);
            buf[at as usize..at as usize + bytes.len()].copy_from_slice(&bytes);
            next += (bytes.len() as u32).div_ceil(4) * 4;
            (at / 4) as u16
        };
        // The identifier names: property 1 is `name` (the compiler's own fixed
        // number), the library's `n_to` is what says this is that library, and
        // the rest are filler so the table is a table.
        let mut identifiers: Vec<(u32, u16)> = vec![(1, put_string(&mut buf, "name"))];
        for (i, id) in
            ["before", "after", "life", "n_to", "s_to", "e_to", "w_to", "description", "capacity"]
                .iter()
                .enumerate()
        {
            identifiers.push((4 + i as u32, put_string(&mut buf, id)));
        }
        identifiers.push((SHORT_NAME_PROP, put_string(&mut buf, "short_name")));
        let maze = put_string(&mut buf, "Maze");
        let cave = z_string("cave");

        // A "routine": ZMSD §5.1's first byte is the local count, 0..=15.
        let routine = next;
        buf[routine as usize] = 3;
        let routine_packed = (routine / 4) as u16;

        // ── The identifiers table ────────────────────────────────────────────
        put_word(&mut buf, IDENT_TABLE as usize, IDENT_COUNT as u16);
        for (prop, packed) in identifiers {
            put_word(&mut buf, (IDENT_TABLE + prop * 2) as usize, packed);
        }

        // ── Object 1: no header name, `short_name` is a string ───────────────
        put_word(&mut buf, (V5_OBJ_ENTRIES + 12) as usize, V5_PROPS_1 as u16);
        buf[V5_PROPS_1 as usize] = 0; // zero words of header name
        // ZMSD §12.4.2's one-byte form: bit 6 set = two bytes of data.
        buf[V5_PROPS_1 as usize + 1] = 0x40 | SHORT_NAME_PROP as u8;
        put_word(&mut buf, V5_PROPS_1 as usize + 2, maze);
        buf[V5_PROPS_1 as usize + 4] = 0; // end of the property table

        // ── Object 2: a header name, `short_name` is a routine ───────────────
        let entry2 = V5_OBJ_ENTRIES + entry_size(5);
        put_word(&mut buf, (entry2 + 12) as usize, V5_PROPS_2 as u16);
        buf[V5_PROPS_2 as usize] = (cave.len() / 2) as u8;
        buf[V5_PROPS_2 as usize + 1..V5_PROPS_2 as usize + 1 + cave.len()].copy_from_slice(&cave);
        let after_name = V5_PROPS_2 as usize + 1 + cave.len();
        buf[after_name] = 0x40 | SHORT_NAME_PROP as u8;
        put_word(&mut buf, after_name + 1, routine_packed);
        buf[after_name + 3] = 0;
        buf
    }

    #[test]
    fn short_name_property_number_is_read_from_the_storys_own_symbol_table() {
        let m = Memory::new(build_short_name_story()).unwrap();
        assert_eq!(
            short_name_property(&m),
            Some(SHORT_NAME_PROP as u8),
            "the number comes from the identifiers table, not from a constant"
        );
    }

    #[test]
    fn a_story_with_no_symbol_table_names_no_short_name_property() {
        let m = Memory::new(sample_story(5)).unwrap();
        assert_eq!(short_name_property(&m), None);
        assert_eq!(printed_name(&m, 1), short_name(&m, 1), "and printed_name is the header name");
    }

    #[test]
    fn printed_name_prefers_a_string_short_name_over_the_header_name() {
        let m = Memory::new(build_short_name_story()).unwrap();
        assert_eq!(short_name(&m, 1), "", "object 1's header name is empty, as Inform compiles it");
        assert_eq!(printed_name(&m, 1), "Maze", "the story prints its short_name property");
        assert_eq!(object_snapshot(&m, 1).name, "Maze", "and the mapper's snapshot carries it");
    }

    #[test]
    fn printed_name_keeps_the_header_name_when_short_name_is_a_routine() {
        let m = Memory::new(build_short_name_story()).unwrap();
        assert_eq!(short_name(&m, 2), "cave");
        assert_eq!(
            printed_name(&m, 2),
            "cave",
            "a routine cannot be read statically, so the header name stands"
        );
    }

    // ── Fixture test ──────────────────────────────────────────────────────────

    #[test]
    fn reads_object_tree_from_minizork() {
        let Some(story) = crate::fixtures::load("minizork.z3") else { return /* skip */ };
        let m = crate::memory::Memory::new(story).unwrap();
        let snap = object_snapshot(&m, 1);
        assert_eq!(snap.number, 1);
        assert!(!snap.name.is_empty());
    }
}
