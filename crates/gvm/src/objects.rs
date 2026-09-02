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
        if addr < self.head
            || !(addr - self.head).is_multiple_of(self.stride)
            || (addr - self.head) / self.stride >= self.count as u32
            || mem.read8(addr) != Some(0x70)
        {
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

    /// The object's hardware short name, decoded. Empty when it has none, which
    /// is the normal case for Inform 7.
    fn printed_name(&self, mem: &Memory, addr: u32) -> String {
        let string = match mem.read32(addr + 1 + self.attr_bytes + 4) {
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
        let table = mem.read32(addr + 1 + self.attr_bytes + 8)?;
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
        let next = mem.read32(cur + 1 + attr_bytes)?;
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
