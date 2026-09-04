// ── Tidy pipeline ─────────────────────────────────────────────────────────────

/// True when `layer`'s geometry is FROZEN: nothing may move its rooms after they are first
/// placed (SQ-0671).
///
/// A maze layer is the case. Tidy optimizes a compass layout towards an objective a maze cannot
/// satisfy — in the reference save 29 of 47 passages are unsatisfiable at once — so the pass never
/// converges, every turn produces a different arrangement, and the pane repaints (and pulses) for
/// a layout nobody is looking at: the matrix reads the graph, not the grid. Freezing costs the
/// player nothing they can see and takes the churn out of the loop.
///
/// This freezes only the OPTIMIZATION. A newly discovered room is still dead-reckoned into place
/// by `place_incremental` on the turn it is found, edges and `tried` records still accrue, and
/// switching the layer back to the drawn view still shows every room somewhere sensible — the
/// positions simply stop being re-derived behind the player's back.
pub fn layer_is_frozen(graph: &mapper::graph::MapGraph, layer: mapper::layer::LayerId) -> bool {
    graph.layer_is_maze(layer)
}

/// Whether a turn schedules background map maintenance for `layer` at all (SQ-0671).
///
/// `changed` is the existing signal — a turn that added no room and no connection has nothing to
/// re-derive. The freeze is the second half: on a maze layer NO job is spawned, rather than one
/// spawned and its result thrown away, so the worker, the border pulse and the generation bump
/// its completion causes all stay out of the loop. `should_bg_tidy` still chooses FULL vs.
/// cleanup-only for the layers that do get one.
pub fn should_schedule_tidy(
    graph: &mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
    changed: bool,
) -> bool {
    changed && !layer_is_frozen(graph, layer)
}

/// Rebuild the layer from scratch by replaying discovery order (the subgraph's
/// connection insertion order), emitting one "Build" frame (with the connection
/// manifest) followed by one "Placement" frame per room. Returns the fully-placed
/// rebuilt graph for the tidy stages to consume. Respects `max_frames`.
fn replay_build_and_placement(
    sub: &mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
    frames: &mut Vec<crate::state::TidyFrame>,
    max_frames: usize,
    progress: Option<&std::sync::Arc<std::sync::atomic::AtomicUsize>>,
) -> mapper::graph::MapGraph {
    use crate::state::TidyFrame;
    use mapper::graph::{MapGraph, RoomId};
    use mapper::layout::{place_incremental, TidyStats};

    let name_of = |g: &MapGraph, id: RoomId| -> String {
        g.room(id).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{id}"))
    };

    let conns = sub.connections();
    let mut rebuild = MapGraph::new();

    // Placement order: anchor first (origin of the first connection, else the first
    // room), then each room as it first appears in the connection list, then any
    // isolated rooms with no connections at all.
    let anchor: Option<RoomId> =
        conns.first().map(|c| c.origin).or_else(|| sub.rooms().next().map(|r| r.id));
    let mut order: Vec<RoomId> = Vec::new();
    let mut seen: std::collections::BTreeSet<RoomId> = std::collections::BTreeSet::new();
    if let Some(a) = anchor {
        order.push(a);
        seen.insert(a);
    }
    for c in conns {
        for id in [c.origin, c.dest] {
            if seen.insert(id) { order.push(id); }
        }
    }
    for r in sub.rooms() {
        if seen.insert(r.id) { order.push(r.id); }
    }

    // ── Build: construct rooms + edges (no positions) on the same layer. ──
    for &id in &order {
        rebuild.upsert_room(id, name_of(sub, id));
        rebuild.set_room_layer(id, layer);
    }
    for c in conns {
        rebuild.add_edge(c.origin, c.dir, c.dest);
    }
    let manifest: Vec<String> = conns.iter()
        .map(|c| format!("{} \u{2192}{:?}\u{2192} {}", name_of(sub, c.origin), c.dir, name_of(sub, c.dest)))
        .collect();
    if frames.len() < max_frames {
        frames.push(TidyFrame {
            label: "Build".into(),
            graph: rebuild.clone(),
            description: format!("Graph built: {} rooms, {} connections", order.len(), conns.len()),
            stats: TidyStats::default(),
            stage_start: true,
            manifest: Some(manifest),
        });
        if let Some(p) = progress { p.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
    }

    // ── Placement: anchor at origin, then place each room in discovery order. ──
    let mut first = true;
    let emit = |rebuild: &MapGraph, desc: String, first: &mut bool, frames: &mut Vec<TidyFrame>| {
        if frames.len() < max_frames {
            frames.push(TidyFrame {
                label: "Placement".into(),
                graph: rebuild.clone(),
                description: desc,
                stats: TidyStats::default(),
                stage_start: *first,
                manifest: None,
            });
            if let Some(p) = progress { p.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
        }
        *first = false;
    };

    if let Some(a) = anchor {
        rebuild.set_pos(a, (0, 0));
        emit(&rebuild, format!("placed room {} ({}) at origin", a, name_of(sub, a)), &mut first, frames);
    }
    // Iterate to a fixed point: `conns` is raw insertion order, which can be
    // disrupted (e.g. a deleted-then-re-added connection moves to the end), so a
    // single forward pass can encounter an edge whose origin isn't placed yet even
    // though an earlier edge in the list would have placed it. Repeat the scan,
    // placing every edge whose origin is placed and dest is not, until a full pass
    // places nothing new. Bounded by `conns.len()` passes as a safety cap: each
    // productive pass places at least one room, so there can be at most that many.
    for _ in 0..conns.len() {
        let mut placed_any = false;
        for c in conns {
            if rebuild.room(c.dest).and_then(|r| r.pos).is_some() { continue; } // revisit
            if rebuild.room(c.origin).and_then(|r| r.pos).is_none() { continue; } // not yet reachable
            place_incremental(&mut rebuild, c.origin, c.dest, c.dir);
            let pos = rebuild.room(c.dest).and_then(|r| r.pos).unwrap_or((0, 0));
            emit(&rebuild, format!("placed room {} ({}) {:?} of room {} at ({},{})",
                c.dest, name_of(sub, c.dest), c.dir, c.origin, pos.0, pos.1), &mut first, frames);
            placed_any = true;
        }
        if !placed_any { break; }
    }
    // Isolated rooms (no in-layer connection): place relative to the anchor.
    if let Some(a) = anchor {
        let unplaced: Vec<RoomId> =
            order.iter().copied().filter(|&id| rebuild.room(id).and_then(|r| r.pos).is_none()).collect();
        for id in unplaced {
            place_incremental(&mut rebuild, a, id, mapper::direction::Direction::Unknown);
            let pos = rebuild.room(id).and_then(|r| r.pos).unwrap_or((0, 0));
            emit(&rebuild, format!("placed room {} ({}) at ({},{})",
                id, name_of(sub, id), pos.0, pos.1), &mut first, frames);
        }
    }

    rebuild
}

/// Run the auto-tidy pipeline on the given `layer`, returning a labelled snapshot of the
/// sub-graph after each stage (frame 0 is the pre-tidy state). The tidied positions are written
/// back into `graph` for every room in `layer`; all other rooms are untouched. Caller must be in
/// Auto mode.
///
/// If `progress` is supplied, the counter is bumped once per emitted frame (so it ends equal to
/// `frames.len()`); the caller uses it to drive a progress bar while the build runs off-thread.
pub(crate) fn run_tidy_pipeline(
    graph: &mut mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
    progress: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
) -> Vec<crate::state::TidyFrame> {
    use crate::render::map::{cleanup_overlaps_observed, compact_empty_lines_observed, repair_directional_hints_observed};
    use crate::state::TidyFrame;
    use mapper::layout::TidyStats;
    use std::sync::atomic::Ordering;

    const MAX_TIDY_FRAMES: usize = 2000;

    let sub = graph.layer_subgraph(layer);
    let mut frames: Vec<TidyFrame> = Vec::new();

    let mut pipe_overlaps: u32 = 0;
    let mut pipe_hints: u32 = 0;
    let mut pipe_rooms_moved: u32 = 0;
    let mut pipe_constraints: u32 = 0;

    // Build + placement replay produces the front frames and the rebuilt graph
    // that the tidy stages run on.
    let mut sub = replay_build_and_placement(&sub, layer, &mut frames, MAX_TIDY_FRAMES, progress.as_ref());

    // Layout stages via relayout_auto_observed
    mapper::layout::relayout_auto_observed(&mut sub, Some(&mut |g: &mapper::graph::MapGraph, label: &str, desc: &str, s: &TidyStats| {
        pipe_rooms_moved = s.rooms_moved;
        pipe_constraints = s.constraints_dropped;
        if frames.len() < MAX_TIDY_FRAMES {
            frames.push(TidyFrame {
                label: label.into(),
                graph: g.clone(),
                description: desc.into(),
                stats: TidyStats {
                    rooms_moved: s.rooms_moved,
                    constraints_dropped: s.constraints_dropped,
                    overlaps_resolved: pipe_overlaps,
                    hints_repaired: pipe_hints,
                },
                stage_start: true,
                manifest: None,
            });
            if let Some(p) = &progress { p.fetch_add(1, Ordering::Relaxed); }
        }
    }));

    // First cleanup_overlaps pass
    {
        let mut first = true;
        cleanup_overlaps_observed(&mut sub, 3, 40, Some(&mut |g, _label, desc, _s| {
            pipe_overlaps += 1;
            if frames.len() < MAX_TIDY_FRAMES {
                frames.push(TidyFrame {
                    label: "cleanup_overlaps".into(),
                    graph: g.clone(),
                    description: desc.into(),
                    stats: TidyStats {
                        rooms_moved: pipe_rooms_moved,
                        constraints_dropped: pipe_constraints,
                        overlaps_resolved: pipe_overlaps,
                        hints_repaired: pipe_hints,
                    },
                    stage_start: first,
                    manifest: None,
                });
                if let Some(p) = &progress { p.fetch_add(1, Ordering::Relaxed); }
                first = false;
            }
        }));
    }

    // repair_directional_hints
    {
        let mut first = true;
        repair_directional_hints_observed(&mut sub, 3, 40, Some(&mut |g, _label, desc, _s| {
            pipe_hints += 1;
            if frames.len() < MAX_TIDY_FRAMES {
                frames.push(TidyFrame {
                    label: "repair_hints".into(),
                    graph: g.clone(),
                    description: desc.into(),
                    stats: TidyStats {
                        rooms_moved: pipe_rooms_moved,
                        constraints_dropped: pipe_constraints,
                        overlaps_resolved: pipe_overlaps,
                        hints_repaired: pipe_hints,
                    },
                    stage_start: first,
                    manifest: None,
                });
                if let Some(p) = &progress { p.fetch_add(1, Ordering::Relaxed); }
                first = false;
            }
        }));
    }

    // Second cleanup_overlaps pass
    {
        let mut first = true;
        cleanup_overlaps_observed(&mut sub, 3, 40, Some(&mut |g, _label, desc, _s| {
            pipe_overlaps += 1;
            if frames.len() < MAX_TIDY_FRAMES {
                frames.push(TidyFrame {
                    label: "cleanup_overlaps".into(),
                    graph: g.clone(),
                    description: desc.into(),
                    stats: TidyStats {
                        rooms_moved: pipe_rooms_moved,
                        constraints_dropped: pipe_constraints,
                        overlaps_resolved: pipe_overlaps,
                        hints_repaired: pipe_hints,
                    },
                    stage_start: first,
                    manifest: None,
                });
                if let Some(p) = &progress { p.fetch_add(1, Ordering::Relaxed); }
                first = false;
            }
        }));
    }

    // compact_empty_lines
    {
        let mut first = true;
        compact_empty_lines_observed(&mut sub, Some(&mut |g, _label, desc, _s| {
            if frames.len() < MAX_TIDY_FRAMES {
                frames.push(TidyFrame {
                    label: "compact".into(),
                    graph: g.clone(),
                    description: desc.into(),
                    stats: TidyStats {
                        rooms_moved: pipe_rooms_moved,
                        constraints_dropped: pipe_constraints,
                        overlaps_resolved: pipe_overlaps,
                        hints_repaired: pipe_hints,
                    },
                    stage_start: first,
                    manifest: None,
                });
                if let Some(p) = &progress { p.fetch_add(1, Ordering::Relaxed); }
                first = false;
            }
        }));
    }

    // Frame cap: if the layout is extremely large, frames are silently truncated at MAX_TIDY_FRAMES.

    // Write the tidied positions back into the live graph for this layer's rooms.
    for id in graph.rooms_in_layer(layer) {
        if let Some(p) = sub.room(id).and_then(|r| r.pos) {
            graph.set_pos(id, p);
        }
    }

    // Write distortion flags back.
    let n = graph.connections().len();
    for idx in 0..n {
        let c = graph.connections()[idx].clone();
        if graph.layer_of(c.origin) == layer && graph.layer_of(c.dest) == layer {
            if let Some(sc) = sub.connections().iter()
                .find(|s| s.origin == c.origin && s.dir == c.dir && s.dest == c.dest)
            {
                graph.set_conn_distorted(idx, sc.distorted);
            }
        }
    }

    frames
}

/// Run the same tidy pipeline stages as `run_tidy_pipeline` but discard the
/// animation frames. The final positions and distortion flags are written back
/// into `graph` exactly as `run_tidy_pipeline` does, but no frame snapshots are
/// allocated. Use this for silent background re-tidy where playback is not wanted.
pub fn tidy_layer_silent(
    graph: &mut mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
) {
    use crate::render::map::{cleanup_overlaps, compact_empty_lines, repair_directional_hints};
    if layer_is_frozen(graph, layer) {
        return; // maze layer: the positions stand as dead-reckoned (SQ-0671)
    }
    run_layer_ops_silent(graph, layer, |sub| {
        mapper::layout::relayout_auto(sub);
        cleanup_overlaps(sub, 3, 40);
        repair_directional_hints(sub, 3, 40);
        cleanup_overlaps(sub, 3, 40);
        compact_empty_lines(sub);
    });
}

/// Background overlap cleanup for one layer: nudges rooms only enough to remove
/// rendered overlaps, WITHOUT the full relayout/repair/compact that
/// [`tidy_layer_silent`] does — so it preserves the existing (possibly hand-tuned)
/// layout and only un-overlaps it. Runs on the background worker on every geometry
/// change so no overlap work touches the interpreter thread (SQ-0379).
pub fn cleanup_overlaps_layer_silent(
    graph: &mut mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
) {
    use crate::render::map::cleanup_overlaps;
    if layer_is_frozen(graph, layer) {
        return; // maze layer: overlaps are a drawn-view problem and the matrix is the view
    }
    run_layer_ops_silent(graph, layer, |sub| {
        cleanup_overlaps(sub, 2, 20);
    });
}

/// Run `ops` on a clone of `layer`'s subgraph, then write the resulting room
/// positions and connection-distortion flags back into `graph`. Shared write-back
/// for the full tidy and the overlap-only cleanup so both merge results identically.
fn run_layer_ops_silent(
    graph: &mut mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
    ops: impl FnOnce(&mut mapper::graph::MapGraph),
) {
    let mut sub = graph.layer_subgraph(layer);
    ops(&mut sub);

    // Write final positions back into the live graph.
    for id in graph.rooms_in_layer(layer) {
        if let Some(p) = sub.room(id).and_then(|r| r.pos) {
            graph.set_pos(id, p);
        }
    }

    // Write distortion flags back.
    let n = graph.connections().len();
    for idx in 0..n {
        let c = graph.connections()[idx].clone();
        if graph.layer_of(c.origin) == layer && graph.layer_of(c.dest) == layer {
            if let Some(sc) = sub.connections().iter()
                .find(|s| s.origin == c.origin && s.dir == c.dir && s.dest == c.dest)
            {
                graph.set_conn_distorted(idx, sc.distorted);
            }
        }
    }
}

/// Outcome of attempting to apply a finished async tidy job to the real graph.
pub enum ApplyTidyOutcome {
    /// Positions were applied; caller should recenter if needed.
    Applied,
    /// Job result was stale (graph changed mid-tidy); caller should re-trigger.
    Stale,
}

/// Pure helper: apply the positions from a finished tidy worker to the real graph,
/// guarded by a generation check.
///
/// If `job_gen == current_gen` the worker's final room positions (and distortion flags)
/// are written into `real_graph` for every room that still exists, and `Applied` is
/// returned.  If the generations differ the result is discarded and `Stale` is returned;
/// the caller must re-trigger a fresh tidy.
///
/// Extracted for unit-testability; does not spawn threads.
pub fn apply_tidy_result(
    real_graph: &mut mapper::graph::MapGraph,
    tidied: mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
    job_gen: u64,
    current_gen: u64,
) -> ApplyTidyOutcome {
    if job_gen != current_gen {
        return ApplyTidyOutcome::Stale;
    }
    if layer_is_frozen(real_graph, layer) {
        // The layer was flagged a maze while this job was in flight (`/mark-maze-layer` during a
        // tidy is exactly the moment a player does it). Its result is a layout for a layer whose
        // geometry is now frozen: drop it, and report Applied so nothing re-triggers. (SQ-0671)
        return ApplyTidyOutcome::Applied;
    }

    // Copy final positions from the tidied clone back into the real graph.
    for id in real_graph.rooms_in_layer(layer) {
        if let Some(p) = tidied.room(id).and_then(|r| r.pos) {
            real_graph.set_pos(id, p);
        }
    }

    // Copy distortion flags.
    let n = real_graph.connections().len();
    for idx in 0..n {
        let c = real_graph.connections()[idx].clone();
        if real_graph.layer_of(c.origin) == layer && real_graph.layer_of(c.dest) == layer {
            if let Some(sc) = tidied.connections().iter()
                .find(|s| s.origin == c.origin && s.dir == c.dir && s.dest == c.dest)
            {
                real_graph.set_conn_distorted(idx, sc.distorted);
            }
        }
    }

    ApplyTidyOutcome::Applied
}

/// Pure decision function for background-tidy mode. Extracted for unit-testability.
///
/// - `mode`: the configured `BackgroundTidy` value.
/// - `new_room`: whether this turn discovered at least one new room.
/// - `overlap`: whether the active layer has a room overlap or distorted edge after
///   incremental placement (fed to all modes now, not only `OnOverlap`).
/// - `counter`: mutable debounce counter; incremented on each new room, reset when
///   a tidy fires under `Debounced`.
///
/// Returns true when a background re-tidy should be triggered.
pub fn should_bg_tidy(
    mode: crate::config::BackgroundTidy,
    new_room: bool,
    overlap: bool,
    changed: bool,
    counter: &mut u32,
) -> bool {
    use crate::config::BackgroundTidy;
    // A turn that did not change the graph (look, examine, inventory, a failed
    // move, …) must never auto-tidy. `overlap` is a state predicate — true
    // whenever the layout currently has any overlap/distortion — so without this
    // gate a persistent overlap re-triggered a tidy on EVERY turn, making the map
    // border pulse on a bare "look". Re-tidying an unchanged graph is also
    // deterministically pointless (same input → same layout).
    if !changed {
        return false;
    }
    match mode {
        BackgroundTidy::Off => false,
        BackgroundTidy::EveryRoom => new_room || overlap,
        BackgroundTidy::OnOverlap => overlap,
        BackgroundTidy::Debounced => {
            // An overlap fires immediately without waiting for the debounce counter.
            if overlap {
                *counter = 0;
                return true;
            }
            if new_room {
                *counter += 1;
                if *counter >= crate::config::BG_TIDY_DEBOUNCE {
                    *counter = 0;
                    return true;
                }
            }
            false
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;

    /// Regression: re-tidy must not let `cleanup_overlaps` move a room to a cell that breaks
    /// its own satisfied compass hints. In the A129 map, #180 must sit NW of #80 and SW of #81
    /// (from `180 S 80`+`80 W 180` and `180 N 81`+`81 W 180`). `relayout_auto` places it there;
    /// before the cleanup guard, the overlap pass shoved #180 into #80's column and below it.
    #[test]
    fn retidy_keeps_180_north_west_of_80_and_south_west_of_81() {
        use mapper::direction::Direction::*;
        let mut g = mapper::graph::MapGraph::new();
        for id in [25u32, 26, 27, 74, 75, 76, 77, 78, 79, 80, 81, 88, 136, 143, 180, 193, 201, 203, 239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180, N, 81), (81, W, 180), (180, W, 78), (78, N, 143), (143, E, 77), (77, S, 74),
            (74, S, 76), (76, W, 78), (143, W, 78), (78, S, 76), (76, N, 74), (74, E, 25),
            (25, W, 76), (74, W, 79), (79, E, 74), (25, E, 26), (26, Up, 25), (78, E, 75),
            (77, E, 239), (239, N, 77), (77, Unknown, 180), (180, S, 80), (80, W, 180),
            (80, E, 79), (79, S, 80), (79, N, 81), (81, E, 79), (80, S, 76), (76, Unknown, 180),
            (79, Unknown, 180), (75, S, 81), (75, W, 78), (75, E, 77), (239, S, 77), (77, W, 75),
            (75, N, 143), (143, S, 75), (26, Down, 27), (27, N, 136), (136, SW, 27), (27, Up, 26),
            (26, Unknown, 180), (79, W, 203), (203, W, 193), (193, E, 203), (203, E, 79),
            (203, Up, 201), (201, Down, 203), (25, Unknown, 180), (239, W, 77), (81, N, 75),
            (25, Down, 26), (75, Up, 88), (88, Down, 75), (143, Unknown, 180),
        ] {
            g.add_edge(o, d, dst);
        }
        let region = mapper::layer::planar_region(&g, 27); // the user's scenario: 27/136 in their own layer
        let _ = mapper::layer::move_region(&mut g, &region, mapper::layer::MoveTarget::New);
        run_tidy_pipeline(&mut g, 0, None);
        let p = |id: mapper::graph::RoomId| g.room(id).unwrap().pos.unwrap();
        let (a, b, c) = (p(180), p(80), p(81));
        assert!(a.0 < b.0 && a.1 < b.1, "180 {a:?} must be NW of 80 {b:?}");
        assert!(a.0 < c.0 && a.1 > c.1, "180 {a:?} must be SW of 81 {c:?}");
    }

    #[test]
    fn reciprocal_ns_keeps_column_when_updown_contends() {
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;

        // A--N-->B and B--S-->A  (reciprocal N/S pair, should share a column).
        // A also has an Up exit to C, which contends for the cell north of A.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.set_pos(3, (0, -1)); // deliberately conflicting to force the tidy to resolve
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        g.add_edge(1, Direction::Up, 3);

        let layer = g.layer_of(1);
        let _ = run_tidy_pipeline(&mut g, layer, None);

        // The reciprocal pair keeps its shared column; the up/down room yields off it.
        let a = g.room(1).unwrap().pos.unwrap();
        let b = g.room(2).unwrap().pos.unwrap();
        let c = g.room(3).unwrap().pos.unwrap();
        assert_eq!(a.0, b.0, "reciprocal N/S pair shares a column");
        assert_ne!(c, b, "the up/down room does not sit on top of the reciprocal neighbor");
    }

    #[test]
    fn pipeline_prepends_build_and_placement_frames() {
        use mapper::mapper::Mapper; // constructed via Mapper::default() (Auto mode)
        use mapper::direction::Direction;

        // A →N→ B →E→ C, placed incrementally (no tidy yet).
        let mut m = Mapper::default();
        m.observe(1, "Foyer", None);
        m.observe(2, "Hall", Some(Direction::N));
        m.observe(3, "Study", Some(Direction::E));

        let layer = m.graph.layer_of(1);
        let frames = run_tidy_pipeline(&mut m.graph, layer, None);

        // First frame is the single Build stop, carrying a manifest and no positioned rooms.
        assert_eq!(frames[0].label, "Build");
        let manifest = frames[0].manifest.as_ref().expect("build frame has a manifest");
        assert_eq!(manifest.len(), 2, "one manifest line per connection");
        assert!(frames[0].graph.rooms().all(|r| r.pos.is_none()),
            "no room is positioned during the build stop");
        assert!(frames[0].description.contains("3 rooms"));
        assert!(frames[0].description.contains("2 connections"));

        // Next three frames are Placement, one per room, all with manifest = None.
        assert_eq!(frames[1].label, "Placement");
        assert_eq!(frames[2].label, "Placement");
        assert_eq!(frames[3].label, "Placement");
        assert!(frames[1..4].iter().all(|f| f.manifest.is_none()));

        // The last placement frame has all three rooms positioned.
        assert_eq!(frames[3].graph.rooms().filter(|r| r.pos.is_some()).count(), 3);

        // Existing tidy stages still follow the placement frames (each stage marks a
        // stage_start frame; the "before" frame is gone now).
        assert!(frames.len() > 4);
        assert!(frames[4..].iter().any(|f| f.stage_start));
    }

    #[test]
    fn pipeline_final_positions_match_silent_for_raw_incremental() {
        use mapper::mapper::Mapper; // constructed via Mapper::default() (Auto mode)
        use mapper::direction::Direction;

        let build = || {
            let mut m = Mapper::default();
            m.observe(1, "Foyer", None);
            m.observe(2, "Hall", Some(Direction::N));
            m.observe(3, "Study", Some(Direction::E));
            m.observe(4, "Attic", Some(Direction::N));
            m
        };
        let mut animated = build();
        let mut silent = build();
        let layer = animated.graph.layer_of(1);

        let _ = run_tidy_pipeline(&mut animated.graph, layer, None);
        tidy_layer_silent(&mut silent.graph, layer);

        for id in [1u32, 2, 3, 4] {
            assert_eq!(
                animated.graph.room(id).unwrap().pos,
                silent.graph.room(id).unwrap().pos,
                "room {id} final position must match the silent (today's) pipeline"
            );
        }
    }

    #[test]
    fn pipeline_progress_counter_counts_every_frame() {
        use mapper::mapper::Mapper;
        use mapper::direction::Direction;
        let mut m = Mapper::default();
        m.observe(1, "Foyer", None);
        m.observe(2, "Hall", Some(Direction::N));
        m.observe(3, "Study", Some(Direction::E));
        m.observe(4, "Attic", Some(Direction::N));
        let layer = m.graph.layer_of(1);
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let frames = run_tidy_pipeline(&mut m.graph, layer, Some(std::sync::Arc::clone(&counter)));
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::Relaxed),
            frames.len(),
            "progress ends equal to the number of emitted frames",
        );
    }

    #[test]
    fn pipeline_single_room_layer() {
        use mapper::mapper::Mapper;
        let mut m = Mapper::default();
        m.observe(1, "Foyer", None);
        let layer = m.graph.layer_of(1);
        let frames = run_tidy_pipeline(&mut m.graph, layer, None);
        assert_eq!(frames[0].label, "Build");
        assert_eq!(frames[0].manifest.as_ref().unwrap().len(), 0, "no connections");
        assert_eq!(frames[1].label, "Placement");
        assert_eq!(frames[1].graph.room(1).unwrap().pos, Some((0, 0)));
    }

    /// Regression: `sub.connections()` is raw insertion order, which can be disrupted
    /// (e.g. a deleted-then-re-added connection moves to the end of the vec). Build a
    /// graph where the edge that would place room 4 (3 --E--> 4) is inserted *before*
    /// the edge that places its origin, room 3 (2 --N--> 3):
    ///   conns = [1--N-->2, 3--E-->4, 2--N-->3]
    /// A single forward pass places 2 (via the first edge), then skips 3--E-->4 because
    /// 3 isn't placed yet, then places 3 (via the third edge) — leaving 4 stranded for
    /// the isolated-room fallback even though it has a perfectly good directional edge.
    /// The fixed-point loop must re-scan and place 4 via its real edge instead.
    #[test]
    fn replay_build_and_placement_reaches_fixed_point_on_disrupted_connection_order() {
        use mapper::direction::Direction::*;
        use mapper::graph::MapGraph;

        let mut sub = MapGraph::new();
        for id in [1u32, 2, 3, 4] {
            sub.upsert_room(id, format!("R{id}"));
        }
        sub.add_edge(1, N, 2); // places room 2 from the anchor
        sub.add_edge(3, E, 4); // placeable only once room 3 exists — inserted early
        sub.add_edge(2, N, 3); // places room 3 — inserted last, disrupting order

        let mut frames: Vec<crate::state::TidyFrame> = Vec::new();
        let rebuild = replay_build_and_placement(&sub, 0, &mut frames, 2000, None);

        // Room 4 must land at its true directional hop: 3 is at (0,-2), so 3--E-->4 is (1,-2).
        assert_eq!(rebuild.room(4).unwrap().pos, Some((1, -2)),
            "room 4 must be placed via its real edge (E of room 3), not the isolated fallback");

        // The frame that places room 4 must be attributed to its real edge, not the
        // isolated-fallback wording (which omits "of room ...").
        let frame4 = frames.iter()
            .find(|f| f.label == "Placement" && f.description.contains("room 4"))
            .expect("a placement frame for room 4");
        assert!(frame4.description.contains("of room 3"),
            "room 4 should be placed relative to room 3 via its real edge: {}", frame4.description);
    }

    #[test]
    fn retidy_refreshes_distorted_flags_after_layer_scoped_tidy() {
        // Regression: run_tidy_pipeline was writing back ONLY positions from the sub-graph,
        // discarding the freshly-computed distorted flags. This test fails RED before the fix
        // (the forced-true flag on a satisfied edge stays true) and GREEN after.
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        use mapper::layout::edge_is_satisfied;

        // Build a small acyclic single-layer compass graph: 1 -E-> 2 -E-> 3 with
        // reciprocal W edges so all compass edges are satisfiable.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);

        // Force a WRONG distorted flag on index 0 (edge 1→E→2).
        // After tidy this edge will be satisfied, so the correct flag is false.
        // Before the fix the stale true remains; after the fix it is corrected to false.
        g.set_conn_distorted(0, true);

        run_tidy_pipeline(&mut g, mapper::layer::MAIN_LAYER, None);

        // After tidy every compass connection's distorted flag must match the geometry.
        for conn in g.connections() {
            if mapper::direction::grid_offset(conn.dir).is_some() {
                let expected = !edge_is_satisfied(&g, conn);
                assert_eq!(
                    conn.distorted, expected,
                    "distorted flag stale on edge {:?}: got {} want {}",
                    conn, conn.distorted, expected,
                );
            }
        }
    }

    #[test]
    fn retidy_only_moves_the_active_layer() {
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        // Layer 0: a 3-room tangle that relayout will move.
        g.upsert_room(1, "A".into()); g.set_pos(1, (0, 0));
        g.upsert_room(2, "B".into()); g.set_pos(2, (5, 5));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        // Layer 1: a room with a fixed position that must NOT move.
        let l = g.new_layer(Some(0), "Other".into());
        g.upsert_room(9, "X".into()); g.set_room_layer(9, l); g.set_pos(9, (3, 3));
        let _frames = run_tidy_pipeline(&mut g, l, None); // tidy the OTHER layer
        assert_eq!(g.room(1).unwrap().pos, Some((0, 0)), "layer-0 room 1 untouched");
        assert_eq!(g.room(2).unwrap().pos, Some((5, 5)), "layer-0 room 2 untouched");
        // Room 9 is the only room in layer l → relayout anchors it at the origin.
        assert_eq!(g.room(9).unwrap().pos, Some((0, 0)), "lone room in tidied layer is anchored");
    }

    // ── should_bg_tidy ────────────────────────────────────────────────────────

    #[test]
    fn should_bg_tidy_off_always_false() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        assert!(!should_bg_tidy(BackgroundTidy::Off, true, true, true, &mut c));
        assert!(!should_bg_tidy(BackgroundTidy::Off, false, false, false, &mut c));
    }

    #[test]
    fn should_bg_tidy_no_change_never_fires() {
        use crate::config::BackgroundTidy;
        // Regression (bug: "look pulses tidy"): a turn that did not change the
        // graph must NOT auto-tidy, even with a persistent layout overlap.
        let mut c = 0u32;
        for mode in [BackgroundTidy::EveryRoom, BackgroundTidy::OnOverlap, BackgroundTidy::Debounced] {
            assert!(!should_bg_tidy(mode, false, true, false, &mut c),
                "{:?}: overlap without a graph change must not fire", mode);
            assert!(!should_bg_tidy(mode, true, true, false, &mut c),
                "{:?}: changed=false must override new_room/overlap", mode);
        }
    }

    #[test]
    fn should_bg_tidy_every_room_follows_new_room_or_overlap() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        // Fires on new room.
        assert!(should_bg_tidy(BackgroundTidy::EveryRoom, true, false, true, &mut c));
        // Fires on overlap even without a new room (the change added a connection).
        assert!(should_bg_tidy(BackgroundTidy::EveryRoom, false, true, true, &mut c));
        // No new room and no overlap: no fire.
        assert!(!should_bg_tidy(BackgroundTidy::EveryRoom, false, false, false, &mut c));
    }

    #[test]
    fn should_bg_tidy_on_overlap_follows_overlap() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        assert!(should_bg_tidy(BackgroundTidy::OnOverlap, false, true, true, &mut c));
        assert!(!should_bg_tidy(BackgroundTidy::OnOverlap, true, false, true, &mut c));
    }

    #[test]
    fn should_bg_tidy_debounced_fires_every_k_new_rooms() {
        use crate::config::{BackgroundTidy, BG_TIDY_DEBOUNCE};
        let mut c = 0u32;
        // First K-1 new rooms should not fire.
        for _ in 0..BG_TIDY_DEBOUNCE - 1 {
            assert!(!should_bg_tidy(BackgroundTidy::Debounced, true, false, true, &mut c));
        }
        // K-th new room fires and resets counter.
        assert!(should_bg_tidy(BackgroundTidy::Debounced, true, false, true, &mut c));
        assert_eq!(c, 0, "counter resets after Debounced fires");
        // No new room: never fires.
        assert!(!should_bg_tidy(BackgroundTidy::Debounced, false, false, false, &mut c));
    }

    #[test]
    fn should_bg_tidy_debounced_fires_immediately_on_overlap() {
        use crate::config::BackgroundTidy;
        // Overlap fires immediately regardless of debounce counter value.
        let mut c = 0u32;
        assert!(should_bg_tidy(BackgroundTidy::Debounced, false, true, true, &mut c),
            "overlap should fire immediately even without a new room");
        assert_eq!(c, 0, "counter is reset when overlap fires");

        // Even with a partially-accumulated counter, overlap fires immediately.
        let mut c = 2u32;
        assert!(should_bg_tidy(BackgroundTidy::Debounced, false, true, true, &mut c),
            "overlap fires even with a non-zero counter");
        assert_eq!(c, 0, "counter is reset when overlap fires");
    }

    // ── tidy_layer_silent ─────────────────────────────────────────────────────

    #[test]
    fn tidy_layer_silent_single_room_noop() {
        // A single-room layer should not panic and leave the room with a position.
        let mut g = mapper::graph::MapGraph::new();
        g.upsert_room(1, "Room".into());
        tidy_layer_silent(&mut g, 0);
        // Room should still exist.
        assert!(g.room(1).is_some());
    }

    #[test]
    fn tidy_layer_silent_leaves_graph_in_same_final_state_as_run_tidy_pipeline() {
        // Build a small two-room graph; run both paths and compare final positions.
        use mapper::direction::Direction;
        let make_graph = || {
            let mut g = mapper::graph::MapGraph::new();
            g.upsert_room(1, "A".into());
            g.upsert_room(2, "B".into());
            g.add_edge(1, Direction::E, 2);
            g.add_edge(2, Direction::W, 1);
            g
        };

        let mut g_pipeline = make_graph();
        run_tidy_pipeline(&mut g_pipeline, 0, None);

        let mut g_silent = make_graph();
        tidy_layer_silent(&mut g_silent, 0);

        let pos = |g: &mapper::graph::MapGraph, id| g.room(id).and_then(|r| r.pos);
        assert_eq!(pos(&g_pipeline, 1), pos(&g_silent, 1), "room 1 position must match");
        assert_eq!(pos(&g_pipeline, 2), pos(&g_silent, 2), "room 2 position must match");
    }

    // ── apply_tidy_result ─────────────────────────────────────────────────────

    #[test]
    fn apply_tidy_result_matching_gen_writes_positions() {
        use mapper::direction::Direction;
        // Build a two-room graph, run tidy on a clone (simulating worker output),
        // then apply to the original.
        let mut real = mapper::graph::MapGraph::new();
        real.upsert_room(1, "A".into());
        real.upsert_room(2, "B".into());
        real.add_edge(1, Direction::E, 2);
        real.add_edge(2, Direction::W, 1);

        let mut tidied = real.clone();
        tidy_layer_silent(&mut tidied, mapper::layer::MAIN_LAYER);
        let tidied_pos1 = tidied.room(1).and_then(|r| r.pos);
        let tidied_pos2 = tidied.room(2).and_then(|r| r.pos);

        let gen = 42u64;
        let outcome = apply_tidy_result(&mut real, tidied, mapper::layer::MAIN_LAYER, gen, gen);
        assert!(matches!(outcome, ApplyTidyOutcome::Applied), "matching gen should return Applied");
        assert_eq!(real.room(1).and_then(|r| r.pos), tidied_pos1, "position 1 must be applied");
        assert_eq!(real.room(2).and_then(|r| r.pos), tidied_pos2, "position 2 must be applied");
    }

    #[test]
    fn apply_tidy_result_stale_gen_discards_result() {
        use mapper::direction::Direction;
        let mut real = mapper::graph::MapGraph::new();
        real.upsert_room(1, "A".into());
        real.upsert_room(2, "B".into());
        real.add_edge(1, Direction::E, 2);
        real.add_edge(2, Direction::W, 1);

        // Force known positions on the real graph so we can confirm they are NOT overwritten.
        real.set_pos(1, (100, 100));
        real.set_pos(2, (200, 200));

        let mut tidied = real.clone();
        tidy_layer_silent(&mut tidied, mapper::layer::MAIN_LAYER);

        let job_gen = 5u64;
        let current_gen = 6u64; // graph changed mid-tidy
        let outcome = apply_tidy_result(&mut real, tidied, mapper::layer::MAIN_LAYER, job_gen, current_gen);
        assert!(matches!(outcome, ApplyTidyOutcome::Stale), "differing gen should return Stale");
        // Positions must be untouched.
        assert_eq!(real.room(1).and_then(|r| r.pos), Some((100, 100)), "stale result must not overwrite position 1");
        assert_eq!(real.room(2).and_then(|r| r.pos), Some((200, 200)), "stale result must not overwrite position 2");
    }
}
