//! `/export-json`: the played map, in the SAME versioned `lanthorn-map` schema
//! `lanthorn-mapgen` writes for a static map (SQ-1336).
//!
//! One serialiser for both — [`crate::mapgen::render_json_view`] — fed a
//! borrowed [`crate::mapgen::JsonMapView`] built here instead of read out of a
//! [`crate::mapgen::GeneratedMap`]. `source` is `"walked"`, so a reader can
//! always tell the two apart; everything a mapgen reader learned from the
//! story file (a door's name, a CEXIT's global) this file has no way to know,
//! since nothing here was ever played through that code path — see
//! [`walked_edge_facts`] for exactly which fields that leaves `null`.

use std::collections::BTreeMap;
use std::path::Path;

use mapper::graph::{MapGraph, PassageWeight, RoomId};

use crate::engine::Engine;
use crate::mapgen::{self, EdgeFact, EdgeKind, EngineRef, StoryIdent};

/// Everything [`render_walked_json`] needs about the running story that a
/// [`MapGraph`] does not carry itself: the story's own header identity, and —
/// for Glulx, whose [`RoomId`] is a hash of the object address
/// ([`crate::roomid::glulx_room_id`]) — every address the session has resolved
/// so far.
pub struct WalkedStory {
    pub ident: StoryIdent,
    /// `RoomId -> Glulx object address`, for every room this session has
    /// resolved one for ([`crate::glulx_session::GlulxSession::known_room_addresses`]).
    /// Empty for a non-Glulx engine, where it is never consulted.
    pub glulx_addrs: BTreeMap<RoomId, u32>,
}

/// Build a [`WalkedStory`] for the running `session`, identified the way its
/// own format identifies itself — the same header fields
/// [`mapgen::z_story_ident`]/[`mapgen::glulx_story_ident`] read for a static
/// map, over `story_bytes` (the mounted executable image `session` was built
/// from, exactly as `mapgen`'s own readers want it).
pub fn build_walked_story(session: &dyn Engine, story_bytes: &[u8], story_path: &Path) -> WalkedStory {
    let file = story_path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(gs) = session.as_any().downcast_ref::<crate::glulx_session::GlulxSession>() {
        WalkedStory {
            ident: mapgen::glulx_story_ident(story_bytes, file),
            glulx_addrs: gs.known_room_addresses().into_iter().collect(),
        }
    } else if session.as_any().is::<crate::scott_session::ScottSession>() {
        // A Scott Adams database has no release, serial or checksum field at
        // all (SQ-1306's own reader agrees) — all three are `null`.
        WalkedStory {
            ident: StoryIdent { file, engine: "scott", release: None, serial: None, checksum: None },
            glulx_addrs: BTreeMap::new(),
        }
    } else {
        WalkedStory { ident: mapgen::z_story_ident(story_bytes, file), glulx_addrs: BTreeMap::new() }
    }
}

/// Every room's engine-native identity (SQ-1336), the live-session counterpart
/// of [`mapgen::assemble`]'s `engine_refs`.
///
/// A Z-machine or Scott Adams [`RoomId`] IS the engine's own object
/// number/room index (`session.rs`'s and `scott_session.rs`'s own docs), so no
/// lookup is needed. A Glulx [`RoomId`] is a hash and only resolves for a room
/// [`WalkedStory::glulx_addrs`] has an entry for — every room the player has
/// stood in, but never a room only ever seen as a destination. A name-only
/// room (`crate::roomid::is_synthetic_room` true on a Z-machine or Scott
/// session, which never happens in practice but is not excluded by the type)
/// has no engine identity at all and is left out, which
/// [`mapgen::render_json_view`] reports as `{kind: "none"}`, same as a
/// destination `mapgen` itself never visited.
pub fn walked_engine_refs(graph: &MapGraph, story: &WalkedStory) -> BTreeMap<RoomId, EngineRef> {
    let mut out = BTreeMap::new();
    for room in graph.rooms() {
        let id = room.id;
        if crate::roomid::is_synthetic_room(id) {
            if let Some(&addr) = story.glulx_addrs.get(&id) {
                out.insert(id, EngineRef::GlulxAddr(addr));
            }
        } else {
            match story.ident.engine {
                "z-machine" => {
                    out.insert(id, EngineRef::ZObject(id as u16));
                }
                "scott" => {
                    out.insert(id, EngineRef::ScottIndex(id as usize));
                }
                _ => {}
            }
        }
    }
    out
}

/// Every connection on `graph`, as the [`EdgeFact`] list [`mapgen::render_json_view`]
/// wants (SQ-1336) — the live-session counterpart of a static reader's own
/// declared-exit table.
///
/// `kind` is read straight off what the graph already knows: [`EdgeKind::Random`]
/// for a connection [`MapGraph::is_random_exit`] agrees is one, [`EdgeKind::Door`]/
/// [`EdgeKind::Conditional`] from the connection's own [`PassageWeight`] (SQ-1312 —
/// set when the player opens a door or the story gates a passage), else
/// [`EdgeKind::OneWay`] when nothing declares the reverse or [`EdgeKind::Declared`]
/// when something does. [`EdgeKind::Routine`] never appears here: it names a
/// mapgen-only reconciliation (SQ-1334, recovering a ZIL FEXIT's destination
/// from another room's declared reverse) that a played graph has no analogue
/// for — every edge here was walked, so its destination is never in question.
///
/// **What a walked map cannot fill, unlike a static one:** `via` (a door's own
/// name — the graph records that a passage is a door, never which object it
/// is) and `note` (a CEXIT's condition, or SQ-1334's routine explanation) are
/// always `null`.
pub fn walked_edge_facts(graph: &MapGraph) -> Vec<EdgeFact> {
    let conns = graph.connections();
    let declared: std::collections::BTreeSet<(RoomId, RoomId)> =
        conns.iter().map(|c| (c.origin, c.dest)).collect();
    conns
        .iter()
        .map(|c| {
            let kind = if graph.is_random_exit(c.origin, c.dir) {
                EdgeKind::Random
            } else {
                match c.weight {
                    PassageWeight::Door => EdgeKind::Door,
                    PassageWeight::Conditional => EdgeKind::Conditional,
                    PassageWeight::Hard if declared.contains(&(c.dest, c.origin)) => EdgeKind::Declared,
                    PassageWeight::Hard => EdgeKind::OneWay,
                }
            };
            EdgeFact { origin: c.origin, dir: c.dir, dest: c.dest, kind, via: None, note: None }
        })
        .collect()
}

/// Render `graph` as the versioned `lanthorn-map` JSON, `source: "walked"`
/// (SQ-1336) — the schema documented in `docs/internals/mapping.md`, same as
/// [`mapgen::render_json`].
pub fn render_walked_json(graph: &MapGraph, story: &WalkedStory) -> String {
    let facts = walked_edge_facts(graph);
    let engine_refs = walked_engine_refs(graph, story);
    mapgen::render_json_view(&mapgen::JsonMapView {
        generator: "lanthorn",
        graph,
        story: &story.ident,
        source: "walked",
        facts: &facts,
        engine_refs: &engine_refs,
    })
}

/// Write [`render_walked_json`] to `path`.
pub fn export_json(path: &Path, graph: &MapGraph, story: &WalkedStory) -> std::io::Result<()> {
    crate::storage::atomic_write(path, render_walked_json(graph, story).as_bytes())
}

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;
    use mapper::direction::Direction;

    /// A two-room graph walked N then S back, round-tripped through
    /// `serde_json::Value` — the app-side counterpart of mapgen's own
    /// `json_map_pins_its_format_and_required_keys` (SQ-1336).
    #[test]
    fn walked_json_round_trips_and_names_itself_walked() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Clearing".into());
        g.upsert_room(2, "Cave Mouth".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);

        let story = WalkedStory {
            ident: StoryIdent {
                file: "test.z5".into(),
                engine: "z-machine",
                release: Some(1),
                serial: Some("990101".into()),
                checksum: Some("0xabcd".into()),
            },
            glulx_addrs: BTreeMap::new(),
        };

        let v: serde_json::Value =
            serde_json::from_str(&render_walked_json(&g, &story)).expect("must round-trip");
        assert_eq!(v["format"], "lanthorn-map");
        assert_eq!(v["version"], 1);
        assert_eq!(v["story"]["source"], "walked");
        assert_eq!(v["story"]["engine"], "z-machine");
        assert_eq!(v["generator"]["name"], "lanthorn");

        // The SAME required-key set `mapgen`'s own
        // `json_map_pins_its_format_and_required_keys` pins for a static map —
        // one schema, two producers, and this is the check that keeps them
        // from silently drifting apart (SQ-1336).
        for key in ["format", "version", "generator", "story", "directions", "rooms", "edges", "layers"] {
            assert!(!v[key].is_null(), "top-level key {key:?} is required");
        }
        for key in ["file", "engine", "source", "generated_at"] {
            assert!(!v["story"][key].is_null(), "story.{key} is required");
        }

        let rooms = v["rooms"].as_array().unwrap();
        assert_eq!(rooms.len(), 2);
        for r in rooms {
            for key in ["id", "raw_id", "name", "ordinal", "layer", "flags", "engine_ref"] {
                assert!(!r[key].is_null(), "room key {key:?} is required");
            }
            assert_eq!(r["engine_ref"]["kind"], "z-object");
        }

        let edges = v["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 2);
        for e in edges {
            for key in ["from", "to", "dir", "kind", "reciprocal"] {
                assert!(!e[key].is_null(), "edge key {key:?} is required");
            }
            assert_eq!(e["kind"], "declared");
            assert_eq!(e["reciprocal"], true);
            assert!(e["via"].is_null(), "a walked map never learns a door's own name");
            assert!(e["note"].is_null(), "a walked map never learns a CEXIT's condition");
        }
    }

    /// A one-way passage with no reverse reads `one-way`, and a door-weighted
    /// one reads `door` even with no reverse either — a door keeps its own,
    /// more specific label per `mapgen::assemble`'s own rule.
    #[test]
    fn walked_edge_kinds_follow_weight_then_reciprocity() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.add_edge(1, Direction::N, 2); // one-way: nothing declares the reverse
        g.add_edge_weighted(1, Direction::E, 3, PassageWeight::Door); // one-way door

        let facts = walked_edge_facts(&g);
        let one_way = facts.iter().find(|f| f.dir == Direction::N).unwrap();
        assert_eq!(one_way.kind, EdgeKind::OneWay);
        let door = facts.iter().find(|f| f.dir == Direction::E).unwrap();
        assert_eq!(door.kind, EdgeKind::Door);
    }
}
