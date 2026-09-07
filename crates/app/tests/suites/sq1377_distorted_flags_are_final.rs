//! A connection's `distorted` flag must agree with the FINAL room positions, not with a
//! snapshot taken partway through the layout pipeline (SQ-1377).
//!
//! `mapper::layout::mark_distorted` was called only once, at the end of
//! `relayout_auto` — but the pipeline both the live app (`app::tidy::run_layer_ops_silent`)
//! and mapgen (`app::mapgen::layout_all_layers`) actually run has four more stages after that
//! which MOVE rooms: `cleanup_overlaps`, `repair_directional_hints`, `cleanup_overlaps` again,
//! `compact_empty_lines`. None of those re-marks, so a flag written at the end of stage one can
//! go stale by the end of stage five: a bearing the solve had to drop may end up honoured once
//! `repair_directional_hints` puts the room back (a false red), or a room `compact_empty_lines`
//! nudges off its row may leave a bearing that used to be honoured now violated (a false plain).
//! SQ-1376 noted Zork I's `Forest #91 ↔ Forest Path #247` drawing plain on the live map by luck
//! of this, not by construction.
//!
//! [`app::render::map::distorted_flags_agree_with_geometry`] is the measurement: it re-derives
//! what each compass connection's flag SHOULD be from the graph's current positions (the same
//! rule [`mapper::layout::remark_distorted`] applies) and reports every disagreement. The fix is
//! `remark_distorted` called as the LAST step of both pipelines, after every stage that can move
//! a room — this suite is the specimen it was fixed against, on five real maps.
//!
//! All five fixtures live under the gitignored `stories/`, so every case here skips vacuously
//! off CI and says so. Run `cargo nextest run -p lanthorn sq1377` against a checkout with
//! `stories/` after any layout, tidy or mapgen change.

use std::path::{Path, PathBuf};

/// A story under the gitignored `stories/`, or `None` when this checkout has no copy.
fn story(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    p.is_file().then_some(p)
}

/// One case per fixture: mapgen's finished map must have EVERY `distorted` flag agree with its
/// own final geometry. A non-empty list here means some room moved after the last time its
/// connections' flags were derived — the defect this quest fixed.
///
/// `distorted_flags_agree_with_geometry` reads every connection on the whole graph — every
/// connection belongs to exactly one layer pair (`layer_subgraph` never crosses one), so this is
/// equivalent to checking each layer in turn and simpler.
fn assert_flags_are_final(story_name: &str, case: &str) {
    let Some(path) = story(story_name) else {
        eprintln!("SKIP {case}: stories/{story_name} absent");
        return;
    };
    let map = app::mapgen::generate(&path, true).expect("mapgen");
    // Non-vacuity: the map must actually hold rooms and connections, or an empty disagreement
    // list would prove nothing.
    assert!(map.graph.rooms().count() > 10, "[{case}] must generate a real map");
    assert!(map.graph.connections().len() > 10, "[{case}] must generate real connections");
    let bad = app::render::map::distorted_flags_agree_with_geometry(&map.graph);
    assert!(
        bad.is_empty(),
        "[{case}] {} distorted-flag disagreement(s):\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn zork1_distorted_flags_agree_with_final_geometry() {
    assert_flags_are_final("zork1-invclues-r52-s871125.z5", "zork1");
}

#[test]
fn anchorhead_distorted_flags_agree_with_final_geometry() {
    assert_flags_are_final("Anchorhead.gblorb", "anchorhead");
}

#[test]
fn counterfeit_monkey_distorted_flags_agree_with_final_geometry() {
    assert_flags_are_final("CounterfeitMonkey-11.gblorb", "counterfeit_monkey");
}

#[test]
fn lostpig_distorted_flags_agree_with_final_geometry() {
    assert_flags_are_final("LostPig.z8", "lostpig");
}

#[test]
fn adventure_distorted_flags_agree_with_final_geometry() {
    assert_flags_are_final("advent.z6", "adventure");
}
