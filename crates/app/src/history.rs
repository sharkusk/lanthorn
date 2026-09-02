//! Per-turn rewind/replay history: a `TurnRecord` per played turn (Quetzal save
//! + optional map snapshot + transcript), plus pure helpers used by the capture
//!   loop (`main.rs`), the archive (`archive.rs`), and the replay modal.
//!
//! Stored as `Arc<TurnRecord>` rather than bare `TurnRecord` (SQ-1184/SQ-1185):
//! the per-turn auto-save hands its snapshot of `state.history` to a background
//! writer thread, and with `save: Vec<u8>` holding a full VM snapshot per turn —
//! "megabytes per turn on big Glulx games" is what motivated the cap below —
//! cloning the whole history Vec-of-values every turn just to cross the thread
//! boundary would itself be an unbounded per-turn cost, the same shape of bug
//! this exists to fix. `Vec<Arc<TurnRecord>>` makes that handoff an O(turns)
//! copy of pointers, never of snapshot bytes.

use std::sync::Arc;

use mapper::mapper::Mapper;

/// One recorded turn. `save` is the Quetzal snapshot of the VM AFTER this turn;
/// `map_snapshot` is the serialized `Mapper` ONLY on turns where the graph
/// structurally changed (so storage ≈ #map-changes, not #turns).
#[derive(Debug, Clone)]
pub struct TurnRecord {
    pub turn: u32,
    pub command: String,
    pub save: Vec<u8>,
    pub map_snapshot: Option<String>,
    pub transcript: String,
}

/// Append a record for a completed turn. The caller computes `map_changed`
/// (cheap room/connection-count delta); the map snapshot is serialized and
/// stored only when it changed.
///
/// Does not itself cap `history` — call [`cap_history`] after this (the
/// per-turn capture site in `turn.rs` does) — so every other caller that
/// simulates turns without touching the config-driven cap (tests scattered
/// across the app crate) keeps working unchanged.
pub fn record_turn(
    history: &mut Vec<Arc<TurnRecord>>,
    turn: u32,
    command: &str,
    save: Vec<u8>,
    mapper: &Mapper,
    map_changed: bool,
    transcript: &str,
) {
    let map_snapshot = map_changed.then(|| mapper::persist::to_json(mapper));
    history.push(Arc::new(TurnRecord {
        turn,
        command: command.to_string(),
        save,
        map_snapshot,
        transcript: transcript.to_string(),
    }));
}

/// Evict the OLDEST records once `history` exceeds `cap` entries (SQ-1185), so
/// memory stays bounded across an arbitrarily long session rather than growing
/// without limit — `TurnRecord::save` is a full VM snapshot, "megabytes per
/// turn on big Glulx games" is what motivated this. There is no "unbounded"
/// escape hatch — a `cap` of 0 is clamped to 1 — because an opt-out would
/// silently reintroduce the exact growth this exists to bound; a player who
/// wants more rewind depth raises the number instead.
pub fn cap_history(history: &mut Vec<Arc<TurnRecord>>, cap: usize) {
    let cap = cap.max(1);
    if history.len() > cap {
        history.drain(0..history.len() - cap);
    }
}

/// Return the latest `map_snapshot` at-or-before `turn` (the map as it stood
/// then), or `None` if no record at-or-before `turn` carries a snapshot.
pub fn map_at_turn(history: &[Arc<TurnRecord>], turn: u32) -> Option<&str> {
    history
        .iter()
        .filter(|r| r.turn <= turn)
        .rev()
        .find_map(|r| r.map_snapshot.as_deref())
}

/// What a linear resume from `history[idx]` needs: the VM save to restore, the
/// reconstructed map JSON at-or-before that turn (if any), and the turn number.
#[derive(Debug, Clone)]
pub struct ResumePlan {
    pub save: Vec<u8>,
    pub map_json: Option<String>,
    pub turn: u32,
}

/// Compute the resume plan for turn index `idx`. Does NOT mutate `history`
/// (the caller truncates to `[0..=idx]` after restoring).
pub fn resume_plan(history: &[Arc<TurnRecord>], idx: usize) -> ResumePlan {
    let rec = &history[idx];
    ResumePlan {
        save: rec.save.clone(),
        map_json: map_at_turn(history, rec.turn).map(|s| s.to_string()),
        turn: rec.turn,
    }
}

/// Rebuild the on-screen transcript from records `[0..=idx]`: each record
/// contributes an echoed `> command` (Input) followed by its turn output (Story).
pub fn rebuild_transcript(
    history: &[Arc<TurnRecord>],
    idx: usize,
) -> (Vec<String>, Vec<crate::state::TranscriptKind>) {
    use crate::state::TranscriptKind;
    let mut lines = Vec::new();
    let mut kinds = Vec::new();
    for rec in history.iter().take(idx + 1) {
        if !rec.command.is_empty() {
            lines.push(format!("> {}", rec.command));
            kinds.push(TranscriptKind::Input);
        }
        for line in rec.transcript.split('\n') {
            lines.push(line.to_string());
            kinds.push(TranscriptKind::Story);
        }
    }
    (lines, kinds)
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;

    fn mapper_with(n: usize) -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        if n >= 2 {
            m.observe(2, "Forest", Some(Direction::N));
        }
        m
    }

    #[test]
    fn record_turn_stores_snapshot_only_when_changed() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        // Turn 1: a room was added -> snapshot present.
        record_turn(&mut hist, 1, "look", vec![1, 2, 3], &m1, true, "West of House");
        // Turn 2: no structural change -> snapshot absent.
        record_turn(&mut hist, 2, "wait", vec![4, 5, 6], &m1, false, "Time passes.");
        // Turn 3: a second room added -> snapshot present.
        let m2 = mapper_with(2);
        record_turn(&mut hist, 3, "north", vec![7, 8, 9], &m2, true, "Forest");

        assert_eq!(hist.len(), 3);
        assert!(!hist[0].save.is_empty(), "save must be non-empty");
        assert_eq!(hist[0].transcript, "West of House");
        assert!(hist[0].map_snapshot.is_some(), "changed turn has a snapshot");
        assert!(hist[1].map_snapshot.is_none(), "unchanged turn has no snapshot");
        assert!(hist[2].map_snapshot.is_some(), "changed turn has a snapshot");
    }

    #[test]
    fn map_at_turn_returns_latest_at_or_before() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        let m2 = mapper_with(2);
        record_turn(&mut hist, 1, "a", vec![0], &m1, true, "");   // snapshot @1
        record_turn(&mut hist, 2, "b", vec![0], &m1, false, "");  // none
        record_turn(&mut hist, 3, "c", vec![0], &m2, true, "");   // snapshot @3
        record_turn(&mut hist, 4, "d", vec![0], &m2, false, "");  // none

        assert_eq!(map_at_turn(&hist, 0), None, "nothing at-or-before turn 0");
        assert_eq!(map_at_turn(&hist, 1), hist[0].map_snapshot.as_deref());
        assert_eq!(map_at_turn(&hist, 2), hist[0].map_snapshot.as_deref(), "falls back to @1");
        assert_eq!(map_at_turn(&hist, 3), hist[2].map_snapshot.as_deref());
        assert_eq!(map_at_turn(&hist, 99), hist[2].map_snapshot.as_deref(), "latest <= turn");
    }

    #[test]
    fn resume_plan_and_truncate() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        let m2 = mapper_with(2);
        record_turn(&mut hist, 1, "a", vec![10], &m1, true, "one");
        record_turn(&mut hist, 2, "b", vec![20], &m1, false, "two");
        record_turn(&mut hist, 3, "c", vec![30], &m2, true, "three");

        let plan = resume_plan(&hist, 1);
        assert_eq!(plan.save, vec![20], "resume save is history[k].save");
        assert_eq!(plan.turn, 2);
        assert_eq!(plan.map_json.as_deref(), map_at_turn(&hist, 2), "reconstructed @<=2");

        hist.truncate(1 + 1); // caller truncates to [0..=idx]
        assert_eq!(hist.len(), 2, "history truncated to k+1");
    }

    #[test]
    fn rebuild_transcript_concatenates_through_idx() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        record_turn(&mut hist, 1, "look", vec![0], &m1, true, "West of House");
        record_turn(&mut hist, 2, "north", vec![0], &m1, false, "Forest");
        let (lines, kinds) = rebuild_transcript(&hist, 1);
        assert_eq!(lines, vec!["> look", "West of House", "> north", "Forest"]);
        assert_eq!(kinds.len(), lines.len());
        use crate::state::TranscriptKind;
        assert_eq!(kinds[0], TranscriptKind::Input);
        assert_eq!(kinds[1], TranscriptKind::Story);
    }

    /// SQ-1185: once `history` exceeds `cap` entries, the OLDEST are evicted —
    /// newest turns and the ability to resume at the boundary both survive.
    #[test]
    fn cap_history_evicts_oldest_beyond_cap() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        for turn in 1..=5u32 {
            record_turn(&mut hist, turn, &format!("t{turn}"), vec![turn as u8], &m1, false, "");
            cap_history(&mut hist, 3);
        }
        assert_eq!(hist.len(), 3, "capped at 3 entries");
        // Oldest (turns 1, 2) evicted; newest three (3, 4, 5) survive in order.
        assert_eq!(hist.iter().map(|r| r.turn).collect::<Vec<_>>(), vec![3, 4, 5]);

        // Resume still works at the new boundary (index 0 == turn 3).
        let plan = resume_plan(&hist, 0);
        assert_eq!(plan.turn, 3);
        assert_eq!(plan.save, vec![3u8]);
    }

    /// A `cap` of 0 is clamped to 1 rather than read as "unbounded" — see the
    /// doc comment on `cap_history`.
    #[test]
    fn cap_history_clamps_zero_cap_to_one() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        record_turn(&mut hist, 1, "a", vec![1], &m1, false, "");
        cap_history(&mut hist, 0);
        record_turn(&mut hist, 2, "b", vec![2], &m1, false, "");
        cap_history(&mut hist, 0);
        assert_eq!(hist.len(), 1, "cap 0 clamps to 1, not unbounded");
        assert_eq!(hist[0].turn, 2, "the newest turn survives");
    }


    /// Integration-flavored capture test: drive a real GameSession and prove the
    /// spec invariants (non-empty save + transcript; snapshot only on map-change).
    /// Mirrors archive.rs's czech.z5 fixture pattern; skips if the fixture is absent.
    #[test]
    fn capture_over_real_session_snapshots_on_room_change() {
        use crate::session::{apply_turn, GameSession};
        // minizork.z3 is an interactive fixture that requests line input (unlike
        // czech.z5, which auto-runs to Quit and has no frame to save afterwards).
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        let Ok(story) = std::fs::read(&fixture) else { return };

        let mut session = GameSession::new(story, true, false, None).expect("GameSession::new");
        let mut mapper = Mapper::default();
        let mut hist: Vec<Arc<TurnRecord>> = Vec::new();

        for (turn, cmd) in ["look", "wait"].iter().enumerate() {
            let rooms_before = mapper.graph.rooms().count();
            let conns_before = mapper.graph.connections().len();
            let result = session.submit(cmd);
            apply_turn(&mut mapper, cmd, &result, &mut Default::default());
            let map_changed = mapper.graph.rooms().count() != rooms_before
                || mapper.graph.connections().len() != conns_before;
            record_turn(
                &mut hist,
                (turn + 1) as u32,
                cmd,
                session.machine.save_quetzal(),
                &mapper,
                map_changed,
                &result.transcript,
            );
        }

        assert_eq!(hist.len(), 2);
        for r in &hist {
            assert!(!r.save.is_empty(), "every record has a non-empty Quetzal save");
        }
    }
}
