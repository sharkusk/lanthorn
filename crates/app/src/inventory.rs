//! Inventory sourcing: heuristic player-object detection and parse fallback.
//!
//! This module is pure (no I/O, no rendering) so all three functions are unit-testable.

use std::collections::BTreeSet;
use zvm::memory::Memory;
use zvm::objects::{get_child, get_sibling, short_name};

// ── detect_player_obj ─────────────────────────────────────────────────────────

/// Attempt to identify the player object by watching which object moved from the
/// previous room into the current room.
///
/// On a location change A→B (both nonzero, A≠B): the player is an object that
/// appears in both `prev_objects_here` (objects at A) and `objects_here` (objects
/// at B). If exactly one such object exists, return `Some(obj)`. Otherwise `None`.
///
/// When prev_location == current_location (no move), or either is 0, returns `None`
/// (not enough information to lock — we re-try next move).
pub fn detect_player_obj(
    prev_location: Option<u16>,
    prev_objects_here: &BTreeSet<u16>,
    current_location: u16,
    objects_here: &BTreeSet<u16>,
) -> Option<u16> {
    let prev = prev_location?;
    // Both must be nonzero and different (an actual move occurred).
    if prev == 0 || current_location == 0 || prev == current_location {
        return None;
    }
    // Objects that were at the previous room AND are now at the current room.
    let candidates: Vec<u16> = objects_here
        .iter()
        .filter(|o| prev_objects_here.contains(o))
        .copied()
        .collect();
    if candidates.len() == 1 {
        Some(candidates[0])
    } else {
        None
    }
}

// ── list_inventory ────────────────────────────────────────────────────────────

/// What object `obj` is, and what this story's parser will accept for it.
///
/// The two facts travel as one value (SQ-1042) because a caller needs both and
/// neither implies the other: Zork I prints "brass lantern" and answers to
/// `lamp`, `lanter` and `light`, while an Inform 7 object prints nothing at all
/// and its words are the only text naming it.
///
/// `names` is `None` for a story that keeps no parse names anywhere readable —
/// Journey has no parser, `advent.z8` brings its own — and then the answer
/// carries the printed name with an empty word list, which is the whole truth
/// about such a story rather than a failure to look.
pub fn object_words(
    mem: &Memory,
    names: Option<&zvm::objects::ParseNames>,
    obj: u16,
) -> grammar_model::ObjectWords {
    names.and_then(|p| p.of(mem, obj)).unwrap_or_else(|| {
        grammar_model::ObjectWords::new(u32::from(obj), short_name(mem, obj), Vec::new(), None, None)
    })
}

/// Return all direct children of `player_obj` in the Z-machine object tree
/// (top-level carried items only), each with the words the parser accepts for
/// it. An object the story holds no text for at all is left out — there is
/// nothing a panel could show and nothing a player could type.
pub fn list_inventory(
    mem: &Memory,
    names: Option<&zvm::objects::ParseNames>,
    player_obj: u16,
) -> Vec<grammar_model::ObjectWords> {
    let mut result = Vec::new();
    let mut child = get_child(mem, player_obj);
    while child != 0 {
        let o = object_words(mem, names, child);
        if o.display_name().is_some() {
            result.push(o);
        }
        child = get_sibling(mem, child);
    }
    result
}

/// Everything the player can SEE among `holder`'s contents: its direct
/// children, plus the contents of any child whose contents are visible, to the
/// object model's own depth cap (SQ-1133).
///
/// The nesting half of [`list_inventory`], and the carried half of scope. The
/// two differ on purpose: the inventory DOCK lists what you are carrying, which
/// is the top level and nothing else, while scope is what you can name, and an
/// opened sack's lunch is nameable. `model.visible_contents` is the same walk
/// [`crate::render::room_info::list_room_objects_excluding`] uses for a room,
/// so a shut container's contents cannot reach either list.
pub fn list_visible_contents(
    model: &zvm::world::WorldModel,
    names: Option<&zvm::objects::ParseNames>,
    mem: &Memory,
    holder: u16,
) -> Vec<grammar_model::ObjectWords> {
    model
        .visible_contents(mem, holder, 0)
        .into_iter()
        .map(|o| object_words(mem, names, o))
        .filter(|o| o.display_name().is_some())
        .collect()
}

// ── parse_inventory_output ────────────────────────────────────────────────────

/// Parse the text output of an inventory command into a list of item names.
///
/// Recognises:
/// - A header line matching "carrying", "holding", "have", or "You are empty"
///   (case-insensitive).
/// - Subsequent non-empty lines as items; leading articles (a/an/the), bullets
///   (-, *) and whitespace are stripped.
/// - An "empty-handed" phrase in the output → empty list.
///
/// Returns `[]` for non-inventory text.
pub fn parse_inventory_output(text: &str) -> Vec<String> {
    // If "empty-handed" appears anywhere, the player is carrying nothing.
    if text.to_lowercase().contains("empty-handed") {
        return Vec::new();
    }

    let mut found_header = false;
    let mut items = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if !found_header {
            let lower = trimmed.to_lowercase();
            if lower.contains("carrying")
                || lower.contains("holding")
                || lower.contains("have")
                || lower.contains("you are empty")
            {
                found_header = true;
            }
        } else {
            if trimmed.is_empty() {
                continue;
            }
            // Strip leading bullets and whitespace.
            let stripped = trimmed
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim();
            if stripped.is_empty() {
                continue;
            }
            // Strip leading article (a/an/the followed by a space).
            let item = strip_article(stripped);
            if !item.is_empty() {
                items.push(item.to_owned());
            }
        }
    }

    items
}

/// Strip a leading "a ", "an ", or "the " (case-insensitive) from `s`.
fn strip_article(s: &str) -> &str {
    let lower = s.to_lowercase();
    for article in &["the ", "an ", "a "] {
        if lower.starts_with(article) {
            return s[article.len()..].trim_start();
        }
    }
    s
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;

    // ── detect_player_obj ─────────────────────────────────────────────────────

    #[test]
    fn detect_returns_sole_mover() {
        // Player (obj 5) + troll (obj 7) were at room A=10; player moved to room B=20.
        let prev_objects = BTreeSet::from([5u16, 7u16]);
        let cur_objects = BTreeSet::from([5u16, 9u16]); // obj 5 followed; 9 is new
        assert_eq!(
            detect_player_obj(Some(10), &prev_objects, 20, &cur_objects),
            Some(5)
        );
    }

    #[test]
    fn detect_returns_none_when_ambiguous() {
        // Both obj 5 and 7 moved together (NPCs travelling with player).
        let prev = BTreeSet::from([5u16, 7u16]);
        let cur = BTreeSet::from([5u16, 7u16, 9u16]);
        assert_eq!(detect_player_obj(Some(10), &prev, 20, &cur), None);
    }

    #[test]
    fn detect_returns_none_when_no_movers() {
        let prev = BTreeSet::from([3u16]);
        let cur = BTreeSet::from([9u16]); // no overlap
        assert_eq!(detect_player_obj(Some(10), &prev, 20, &cur), None);
    }

    #[test]
    fn detect_returns_none_when_no_location_change() {
        // Same location A→A: no move occurred.
        let prev = BTreeSet::from([5u16]);
        let cur = BTreeSet::from([5u16]);
        assert_eq!(detect_player_obj(Some(10), &prev, 10, &cur), None);
    }

    #[test]
    fn detect_returns_none_when_prev_location_none() {
        let prev = BTreeSet::from([5u16]);
        let cur = BTreeSet::from([5u16]);
        assert_eq!(detect_player_obj(None, &prev, 10, &cur), None);
    }

    #[test]
    fn detect_returns_none_when_current_location_zero() {
        let prev = BTreeSet::from([5u16]);
        let cur = BTreeSet::from([5u16]);
        assert_eq!(detect_player_obj(Some(10), &prev, 0, &cur), None);
    }

    // ── parse_inventory_output ────────────────────────────────────────────────

    #[test]
    fn parse_carrying_two_items() {
        let text = "You are carrying:\n  a brass lamp\n  a sword";
        let items = parse_inventory_output(text);
        assert_eq!(items, vec!["brass lamp", "sword"]);
    }

    #[test]
    fn parse_empty_handed() {
        let text = "You are empty-handed.";
        assert_eq!(parse_inventory_output(text), Vec::<String>::new());
    }

    #[test]
    fn parse_non_inventory_text_returns_empty() {
        let text = "It is pitch black. You are likely to be eaten by a grue.";
        assert_eq!(parse_inventory_output(text), Vec::<String>::new());
    }

    #[test]
    fn parse_strips_article_an() {
        let text = "You are carrying:\n  an ancient scroll";
        assert_eq!(parse_inventory_output(text), vec!["ancient scroll"]);
    }

    #[test]
    fn parse_strips_article_the() {
        let text = "You are carrying:\n  the rusty key";
        assert_eq!(parse_inventory_output(text), vec!["rusty key"]);
    }

    #[test]
    fn parse_strips_bullet_dash() {
        let text = "You have:\n  - leaflet";
        assert_eq!(parse_inventory_output(text), vec!["leaflet"]);
    }

    #[test]
    fn parse_holding_header() {
        let text = "You are holding:\n  a lantern";
        assert_eq!(parse_inventory_output(text), vec!["lantern"]);
    }

    // ── list_inventory (synthetic memory) ────────────────────────────────────
    //
    // We build a minimal v3 story where:
    //   player_obj = 1 (parent = 0)
    //   child1     = 2 (parent = 1, sibling = 3, name = "lamp")
    //   child2     = 3 (parent = 1, sibling = 0, name = "sword")
    // Verifying that list_inventory walks the sibling chain correctly.

    fn build_player_story() -> Vec<u8> {
        // Layout mirrors the hand-built stories in zvm's objects.rs tests.
        // object_table = 0x0100 (v3)
        // entries_base = 0x0100 + 31*2 = 0x013E
        // obj1 at 0x013E, obj2 at 0x0147, obj3 at 0x0150
        // prop tables at 0x0200, 0x0220, 0x0240

        let mut buf = zvm_test_support::sample_story_v3();

        const ENTRIES_BASE: usize = 0x013E;
        const OBJ1: usize = ENTRIES_BASE;
        const OBJ2: usize = ENTRIES_BASE + 9;
        const OBJ3: usize = ENTRIES_BASE + 18;

        const PROP1: u16 = 0x0200;
        const PROP2: u16 = 0x0220;
        const PROP3: u16 = 0x0240;

        // obj1: parent=0, sibling=0, child=2
        buf[OBJ1 + 4] = 0; // parent
        buf[OBJ1 + 5] = 0; // sibling
        buf[OBJ1 + 6] = 2; // child
        buf[OBJ1 + 7] = (PROP1 >> 8) as u8;
        buf[OBJ1 + 8] = (PROP1 & 0xFF) as u8;

        // obj2: parent=1, sibling=3, child=0, name="lamp"
        buf[OBJ2 + 4] = 1; // parent
        buf[OBJ2 + 5] = 3; // sibling
        buf[OBJ2 + 6] = 0; // child
        buf[OBJ2 + 7] = (PROP2 >> 8) as u8;
        buf[OBJ2 + 8] = (PROP2 & 0xFF) as u8;

        // obj3: parent=1, sibling=0, child=0, name="sword"
        buf[OBJ3 + 4] = 1; // parent
        buf[OBJ3 + 5] = 0; // sibling
        buf[OBJ3 + 6] = 0; // child
        buf[OBJ3 + 7] = (PROP3 >> 8) as u8;
        buf[OBJ3 + 8] = (PROP3 & 0xFF) as u8;

        // Prop table for obj1: name="plyr" (4 bytes encoded), no properties.
        write_name(&mut buf, PROP1 as usize, "plyr");

        // Prop table for obj2: name="lamp"
        write_name(&mut buf, PROP2 as usize, "lamp");

        // Prop table for obj3: name="swrd" (4 chars for v3 encode_word)
        write_name(&mut buf, PROP3 as usize, "swrd");

        buf
    }

    /// Write a v3 short-name property table header at `offset`.
    /// Uses zvm's encode_word to produce a 4-byte Z-string (2 words).
    fn write_name(buf: &mut [u8], offset: usize, text: &str) {
        let encoded = zvm::text::encode::encode_word(text, 3); // 4 bytes
        assert_eq!(encoded.len(), 4);
        buf[offset] = 2; // 2 Z-words
        buf[offset + 1..offset + 5].copy_from_slice(&encoded);
        buf[offset + 5] = 0x00; // no properties
    }

    #[test]
    fn list_inventory_walks_sibling_chain() {
        let buf = build_player_story();
        let mem = Memory::new(buf).expect("Memory::new failed");
        let items = list_inventory(&mem, None, 1);
        // Both children should be listed; order is child-first (lamp, sword).
        assert_eq!(items.len(), 2, "expected 2 items, got {:?}", items);
        // Names should start with "lamp" and "swrd" (encode may add padding).
        assert!(items[0].printed_name.starts_with("lamp"), "first item: {:?}", items[0]);
        assert!(items[1].printed_name.starts_with("swrd"), "second item: {:?}", items[1]);
        // A story with no parse-name property answers with printed names and no
        // words at all, which is the whole truth about it (SQ-1042).
        assert!(items.iter().all(|o| o.words.is_empty()), "{items:?}");
    }

    #[test]
    fn list_inventory_empty_when_no_children() {
        let buf = build_player_story();
        let mem = Memory::new(buf).unwrap();
        // obj2 has no children (child=0); its inventory should be empty.
        let items = list_inventory(&mem, None, 2);
        assert!(items.is_empty(), "expected empty inventory for obj2, got {:?}", items);
    }
}

// ── Test support shim ─────────────────────────────────────────────────────────

/// A private shim that provides the sample_story_v3 helper used only in tests.
/// This mirrors what zvm's own tests do internally.
#[cfg(all(test, feature = "t-session"))]
mod zvm_test_support {
    pub fn sample_story_v3() -> Vec<u8> {
        // Build a minimal valid v3 story buffer matching the header layout in
        // zvm/src/header.rs tests_support::sample_story(3).
        //
        // Header offsets (ZMSD §11):
        //   0x00       version
        //   0x04-0x05  high_mem_base
        //   0x06-0x07  initial_pc
        //   0x08-0x09  dictionary
        //   0x0A-0x0B  object_table
        //   0x0C-0x0D  global_vars
        //   0x0E-0x0F  static_mem_base
        //   0x18-0x19  abbrev_table
        let mut buf = vec![0u8; 0x1000];
        buf[0x00] = 3;                       // version = 3
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        buf[0x06] = 0x00; buf[0x07] = 0x40; // initial_pc = 0x0040
        buf[0x08] = 0x02; buf[0x09] = 0x00; // dictionary = 0x0200
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060

        // Dictionary at 0x0200: 0 word-separators, entry-size=4, 0 entries.
        buf[0x0200] = 0;
        buf[0x0201] = 4;
        buf[0x0202] = 0; buf[0x0203] = 0;

        // QUIT opcode at initial PC 0x0040.
        buf[0x0040] = 0xba;

        buf
    }
}
