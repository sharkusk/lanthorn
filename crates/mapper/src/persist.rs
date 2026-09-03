use std::collections::BTreeMap;
use crate::direction::Direction;
use crate::graph::{Connection, MapGraph, Room, RoomId};
use crate::layer::{LayerId, LayerMeta};
use crate::mapper::Mapper;
use crate::suggest::{SeamDecision, SeamKey};

/// One answer the player gave a layer-suggestion prompt (SQ-0439).
///
/// A flat list rather than a map because the key is a room-plus-direction pair and JSON object keys
/// are strings; flattening it also keeps the file readable, which is the whole point of storing the
/// map as JSON.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SeamRecord {
    pub from: RoomId,
    pub dir: Direction,
    pub decision: SeamDecision,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PersistState {
    pub version: u32,
    pub rooms: Vec<Room>,
    pub connections: Vec<Connection>,
    pub current: Option<RoomId>,
    #[serde(default)]
    pub layers: BTreeMap<LayerId, LayerMeta>,
    #[serde(default)]
    pub next_layer_id: LayerId,
    /// Per-layer "last room visited" memory (SQ-0672): absent from any file written before this,
    /// which loads as though nothing had ever been visited on any layer.
    #[serde(default)]
    pub last_visited: BTreeMap<LayerId, RoomId>,
    /// The next room-discovery ordinal to mint (SQ-0685). Absent from a file written before
    /// discovery order was tracked, which loads as 0 — harmless, since [`MapGraph::from_parts`]
    /// resumes it past whatever it backfills onto the rooms in that case anyway.
    #[serde(default)]
    pub next_seq: u64,
    /// The player's answers to the layer-suggestion prompt (SQ-0439). Player DECISIONS, so nothing
    /// can recompute them and the map has to carry them: a restored game that re-asked about a
    /// passage already declined would be the exact nagging the prompt was designed to avoid.
    #[serde(default)]
    pub seams: Vec<SeamRecord>,
}

pub fn to_json(mapper: &Mapper) -> String {
    let state = PersistState {
        version: 1,
        rooms: mapper.graph.rooms().cloned().collect(),
        connections: mapper.graph.connections().to_vec(),
        current: mapper.graph.current(),
        layers: mapper.graph.layers().clone(),
        next_layer_id: mapper.graph.next_layer_id(),
        last_visited: mapper.graph.last_visited_map().clone(),
        next_seq: mapper.graph.next_seq(),
        seams: mapper
            .graph
            .seam_decisions()
            .iter()
            .map(|(k, v)| SeamRecord { from: k.from, dir: k.dir, decision: *v })
            .collect(),
    };
    serde_json::to_string_pretty(&state).expect("PersistState is always serializable")
}

pub fn from_json(s: &str) -> Result<Mapper, serde_json::Error> {
    let state: PersistState = serde_json::from_str(s)?;
    let mut graph = MapGraph::from_parts(
        state.rooms, state.connections, state.current, state.layers, state.next_layer_id,
        state.last_visited, state.next_seq,
    );
    // Collapse `?` stubs that a real directional edge already covers, so existing saved maps
    // clean up on load. (SQ-0220)
    graph.collapse_unknown_edges();
    graph.restore_seam_decisions(
        state
            .seams
            .into_iter()
            .map(|r| (SeamKey { from: r.from, dir: r.dir }, r.decision)),
    );
    // A loaded map has no walked arrival: the player has not moved yet this
    // session, so a bare peel falls back to the portal-seam search until they do.
    Ok(Mapper::restored(graph))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::Mapper;
    use crate::direction::Direction;

    #[test]
    fn round_trips_layers() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let l = m.graph.new_layer(Some(0), "Basement".into());
        m.graph.set_room_layer(2, l);
        let json = to_json(&m);
        let m2 = from_json(&json).unwrap();
        assert_eq!(m2.graph.layer_of(2), l);
        assert_eq!(m2.graph.layer_name(l), "Basement");
        assert_eq!(m2.graph.next_layer_id(), m.graph.next_layer_id());
    }

    /// SQ-0666: the maze flag, the per-layer view mode and self-loop edges all have to survive
    /// the map file, or `/mark-maze-layer` and `/view-map` would be undone by every reload.
    #[test]
    fn round_trips_maze_flag_view_mode_and_self_loops() {
        use crate::layer::MapView;
        let mut m = Mapper::default();
        m.observe(1, "Maze", None);
        m.observe(2, "Maze", Some(Direction::N));
        let l = m.graph.new_layer(Some(0), "Maze".into());
        m.graph.set_room_layer(2, l);
        m.graph.set_layer_maze(l, true);
        m.graph.set_layer_view(l, Some(MapView::Drawn)); // an explicit choice AGAINST the default
        assert!(m.graph.add_self_loop(1, Direction::W));

        let m2 = from_json(&to_json(&m)).unwrap();
        assert!(m2.graph.layer_is_maze(l), "the maze flag survives");
        assert_eq!(
            m2.graph.layer_view(l),
            MapView::Drawn,
            "an explicit view choice survives and still beats the maze default"
        );
        assert_eq!(m2.graph.layer_view_choice(0), None, "an unchosen layer stays unchosen");
        assert_eq!(m2.graph.self_loops(1), vec![Direction::W], "the self-loop edge survives");
    }

    /// SQ-1257: a random-exit mark must survive the map file, or every restore would forget which
    /// directions the story randomises and start minting false edges for them again.
    #[test]
    fn round_trips_random_exit_marks() {
        let mut m = Mapper::default();
        m.observe(1, "Windy Cave", None);
        assert!(m.record_random_exit(1, Direction::N));

        let m2 = from_json(&to_json(&m)).unwrap();
        assert!(m2.graph.is_random_exit(1, Direction::N), "the random mark survives");
        assert!(m2.graph.is_tried(1, Direction::N), "and it still counts as tried");
        assert_eq!(
            crate::matrix::classify(&m2.graph, 1, Direction::N),
            crate::matrix::MatrixCell::Random { destinations: 0 },
            "so a reload reads the cell exactly as the live session did"
        );
    }

    /// A map file saved before SQ-1257 has no `random_exits` field at all; it must load as an
    /// empty list rather than fail to parse.
    #[test]
    fn a_pre_sq1257_map_file_has_no_random_exits_field_and_loads_fine() {
        let old = r#"{"version":1,"rooms":[{"id":1,"name":"Hall","label_override":null,"notes":"","pos":[0,0]}],"connections":[],"current":1}"#;
        let m = from_json(old).unwrap();
        assert!(m.graph.room(1).unwrap().random_exits.is_empty());
    }

    /// SQ-1261: the destinations a random exit has been seen to land in must survive the map
    /// file too, or every restore would forget where Lost Pig's gnome tunnels have sent the
    /// player and the room card would go back to saying only "destination varies".
    #[test]
    fn round_trips_random_exit_destinations() {
        let mut m = Mapper::default();
        m.observe(1, "Windy Cave", None);
        assert!(m.record_random_exit(1, Direction::N));
        m.graph.note_random_destination(1, Direction::N, 2);
        m.graph.note_random_destination(1, Direction::N, 3);

        let m2 = from_json(&to_json(&m)).unwrap();
        assert_eq!(m2.graph.random_destinations(1, Direction::N), &[2, 3], "order and membership survive");
        assert_eq!(
            crate::matrix::classify(&m2.graph, 1, Direction::N),
            crate::matrix::MatrixCell::Random { destinations: 2 },
            "and the matrix cell's count agrees after reload"
        );
    }

    /// A map file saved before SQ-1261 has no `random_destinations` field at all; it must load
    /// as an empty list rather than fail to parse — the same back-compat shape `random_exits`
    /// itself needed when it was new.
    #[test]
    fn a_pre_sq1261_map_file_has_no_random_destinations_field_and_loads_fine() {
        let old = r#"{"version":1,"rooms":[{"id":1,"name":"Windy Cave","label_override":null,"notes":"","pos":[0,0],"random_exits":["N"]}],"connections":[],"current":1}"#;
        let m = from_json(old).unwrap();
        assert!(m.graph.room(1).unwrap().random_destinations.is_empty());
        assert!(m.graph.random_destinations(1, Direction::N).is_empty());
        assert!(m.graph.is_random_exit(1, Direction::N), "the older field still loads fine");
    }

    /// SQ-1257 Phase 3: a room's aliases must survive the map file, or every restore would
    /// forget every OTHER name the story ever printed for a room like Lost Pig's gnome tunnels
    /// and start the alias list over from whatever it happens to be called at reload time.
    #[test]
    fn round_trips_room_aliases() {
        let mut m = Mapper::default();
        m.observe_moved(183, "Twisty Cave", None);
        m.observe_moved(183, "Confusing Passage", Some(Direction::N));
        m.observe_moved(183, "Strange Place", Some(Direction::E));

        let m2 = from_json(&to_json(&m)).unwrap();
        assert_eq!(m2.graph.room(183).unwrap().name, "Strange Place", "the current label survives");
        assert_eq!(
            m2.graph.room(183).unwrap().aliases,
            vec!["Twisty Cave", "Confusing Passage"],
            "every other name survives, in first-seen order"
        );
        assert!(m2.graph.is_random_exit(183, Direction::N));
        assert!(m2.graph.is_random_exit(183, Direction::E));
    }

    /// A map file saved before SQ-1257 Phase 3 has no `aliases` field at all; it must load as an
    /// empty list rather than fail to parse — the same back-compat shape as `random_exits` above.
    #[test]
    fn a_pre_phase3_map_file_has_no_aliases_field_and_loads_fine() {
        let old = r#"{"version":1,"rooms":[{"id":1,"name":"Hall","label_override":null,"notes":"","pos":[0,0]}],"connections":[],"current":1}"#;
        let m = from_json(old).unwrap();
        assert!(m.graph.room(1).unwrap().aliases.is_empty());
    }

    /// SQ-0672: the per-layer "last room visited" memory must survive a save/load round trip, or
    /// a layer switch after a restore would always fall back to the bounding-box centre instead
    /// of the room the player actually last stood on there.
    #[test]
    fn round_trips_last_visited_memory() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None); // Main; last_visited(Main) = 1
        m.observe(2, "Forest", Some(Direction::N)); // Main; last_visited(Main) = 2
        let l = m.graph.new_layer(Some(0), "Basement".into());
        m.observe(3, "Cellar", Some(Direction::Down)); // still on Main when observed
        m.graph.set_room_layer(3, l); // peeled onto the new layer AFTER the visit was recorded
        m.observe(2, "Forest", Some(Direction::S)); // back on Main; last_visited(Main) = 2 again

        let m2 = from_json(&to_json(&m)).unwrap();
        assert_eq!(m2.graph.last_visited(0), Some(2), "Main's last-visited survives the round trip");
        assert_eq!(
            m2.graph.last_visited(l), None,
            "layer l was never re-observed after the peel, so it has no memory of its own yet"
        );
    }

    /// A last-visited entry naming a room id that no longer exists (hand-edited or truncated
    /// file) must be dropped on load, exactly like a dangling `current` or connection endpoint —
    /// otherwise a layer switch would recenter on a phantom room instead of falling back to the
    /// bounding-box centre (SQ-0672).
    #[test]
    fn a_dangling_last_visited_room_id_is_dropped_on_load() {
        let json = r#"{"version":1,
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]}],
            "connections":[],"current":1,
            "last_visited":{"0":1,"7":99}}"#;
        let m = from_json(json).unwrap();
        assert_eq!(m.graph.last_visited(0), Some(1), "the entry naming a real room survives");
        assert_eq!(m.graph.last_visited(7), None, "the entry naming room 99 (does not exist) is dropped");
    }

    /// A map saved before SQ-0672 carries no `last_visited` field at all; it must still load,
    /// with every layer's memory simply absent (mirrors the pre-SQ-0666 maze-flag test above).
    #[test]
    fn a_map_saved_before_last_visited_existed_still_loads() {
        let json = r#"{"version":1,
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]}],
            "connections":[],"current":1}"#;
        let m = from_json(json).expect("a pre-SQ-0672 map still loads");
        assert_eq!(m.graph.last_visited(0), None);
    }

    /// A layer written before SQ-0666 has neither field; it must load as an ordinary drawn,
    /// non-maze layer rather than failing the whole map.
    #[test]
    fn a_layer_saved_before_the_maze_flag_loads_as_a_plain_drawn_layer() {
        use crate::layer::MapView;
        let json = r#"{"version":1,
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0],"layer":1}],
            "connections":[],"current":1,
            "layers":{"0":{"name":"Main","parent":null},"1":{"name":"Cellar","parent":0}},
            "next_layer_id":2}"#;
        let m = from_json(json).expect("a pre-SQ-0666 map still loads");
        assert!(!m.graph.layer_is_maze(1));
        assert_eq!(m.graph.layer_view(1), MapView::Drawn);
    }

    /// Maps written before SQ-0600 carry a `"mode"` field that no longer exists
    /// on `PersistState`. Serde ignores unknown fields, so they still load —
    /// pinned here because a stray `deny_unknown_fields` would silently make
    /// every previously-saved map unreadable.
    #[test]
    fn a_map_saved_with_the_old_layout_mode_field_still_loads() {
        let json = r#"{"version":1,"mode":"Manual",
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]}],
            "connections":[],"current":1}"#;
        let m = from_json(json).expect("a pre-SQ-0600 map still loads");
        assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)));
        assert_eq!(m.graph.current(), Some(1));
    }

    #[test]
    fn from_json_collapses_redundant_unknown_edges() {
        // An existing save with a redundant `?` 1→2 (a real N 1→2 already covers it) plus a lone
        // `?` 2→3 (no known counterpart). Loading collapses the redundant one, keeps the lone one.
        // (SQ-0220)
        let json = r#"{"version":1,"mode":"Auto",
            "rooms":[
                {"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]},
                {"id":2,"name":"B","label_override":null,"notes":"","pos":[0,-1]},
                {"id":3,"name":"C","label_override":null,"notes":"","pos":[1,0]}],
            "connections":[
                {"origin":1,"dir":"Unknown","dest":2,"distorted":false},
                {"origin":1,"dir":"N","dest":2,"distorted":false},
                {"origin":2,"dir":"Unknown","dest":3,"distorted":false}],
            "current":1}"#;
        let m = from_json(json).unwrap();
        assert!(
            !m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown),
            "the redundant Unknown 1→2 collapsed on load: {:?}", m.graph.connections()
        );
        assert!(
            m.graph.connections().iter().any(|c| c.origin == 2 && c.dir == Direction::Unknown && c.dest == 3),
            "the lone Unknown 2→3 (no known counterpart) survives load"
        );
        assert_eq!(m.graph.connections().len(), 2);
    }

    /// SQ-0632: a map file referencing room ids that do not exist (hand-edited, truncated, or
    /// corrupt) must be cleaned on load. Phantom endpoint ids otherwise enter layout components
    /// via `connected_components`' adjacency insertion, permanently wasting a grid cell and
    /// flagging a stray distorted edge; a dangling `current` misdraws the highlight forever.
    #[test]
    fn dangling_connections_and_current_are_dropped_on_load() {
        let json = r#"{"version":1,
            "rooms":[
                {"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]},
                {"id":2,"name":"B","label_override":null,"notes":"","pos":[1,0]}],
            "connections":[
                {"origin":1,"dir":"E","dest":2,"distorted":false},
                {"origin":1,"dir":"N","dest":99,"distorted":false},
                {"origin":98,"dir":"S","dest":1,"distorted":false}],
            "current":42}"#;
        let m = from_json(json).unwrap();
        assert_eq!(
            m.graph.connections().len(),
            1,
            "both connections with a phantom endpoint are dropped: {:?}",
            m.graph.connections()
        );
        assert_eq!(m.graph.connections()[0].dest, 2, "the valid edge survives");
        assert_eq!(m.graph.current(), None, "a current naming no room is reset");
    }

    #[test]
    fn legacy_save_without_layers_loads_as_main() {
        // A v1 save predating layers: no `layer` on rooms, no `layers`/`next_layer_id`.
        let json = r#"{"version":1,"mode":"Auto",
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]}],
            "connections":[],"current":1}"#;
        let m = from_json(json).unwrap();
        assert_eq!(m.graph.layer_of(1), 0);
        assert_eq!(m.graph.layer_name(0), "Main");
    }

    /// SQ-0685: a room's discovery sequence must survive the round trip AND go on protecting its
    /// number afterward — a lower-id duplicate discovered only AFTER a reload must still land
    /// behind everything found before it, on either side of the save file.
    #[test]
    fn seq_round_trips_and_still_protects_numbering_after_reload() {
        let mut m = Mapper::default();
        m.observe(5, "Maze", None);
        m.observe(7, "Maze", Some(Direction::N));
        assert_eq!(m.graph.room(5).unwrap().seq, 0);
        assert_eq!(m.graph.room(7).unwrap().seq, 1);
        assert_eq!(m.graph.next_seq(), 2);

        let mut m2 = from_json(&to_json(&m)).unwrap();
        assert_eq!(m2.graph.room(5).unwrap().seq, 0, "seq survives the round trip");
        assert_eq!(m2.graph.room(7).unwrap().seq, 1);
        assert_eq!(m2.graph.next_seq(), 2, "next_seq resumes exactly where it left off");

        m2.observe(2, "Maze", Some(Direction::S)); // a lower id, discovered only after the reload
        let lbl = crate::matrix::labels(&m2.graph, crate::layer::MAIN_LAYER);
        assert_eq!(lbl.row_of(5), "Maze 1", "the room found before the save keeps its number");
        assert_eq!(lbl.row_of(7), "Maze 2", "so does the second one found before the save");
        assert_eq!(lbl.row_of(2), "Maze 3", "the newcomer is numbered after both, not before");
    }

    #[test]
    fn round_trips_full_state() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        m.set_notes(2, "trees\nwith a newline".into()); // freeform notes incl newline
        m.rename_room(2, Some("Deep Forest".into()));
        let json = to_json(&m);
        let m2 = from_json(&json).unwrap();
        assert_eq!(m2.graph.room(2).unwrap().label(), "Deep Forest");
        assert_eq!(m2.graph.room(2).unwrap().notes, "trees\nwith a newline");
        assert_eq!(m2.graph.current(), Some(2));
        assert_eq!(m2.graph.connections(), m.graph.connections());
        assert_eq!(m2.graph.room(2).unwrap().pos, m.graph.room(2).unwrap().pos);
    }
}
