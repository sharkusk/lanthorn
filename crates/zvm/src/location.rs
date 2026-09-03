// Mapper-facing API — current player location and read-only object tree.
//
// This module provides two signals that the future automapper consumes:
//   1. `current_location` — the object representing where the player is now.
//   2. `object_tree_view` — a read-only enumeration of all objects.
//
// # Location heuristic
//
// The Z-machine specification (ZMSD) has no standard mechanism for identifying
// the player's current location.  We use version-dependent heuristics:
//
// ## v3 (status-line games, ZMSD §8.2.2.1)
// The interpreter status line reads the current room from **global variable 0**
// (variable number 0x10, the first global).  This is the object number of the
// current room.  We read that global; if it is nonzero and within the valid
// object-number range we return its snapshot.
//
// ## v4+ (no status line / Inform games)
// There is no guaranteed status-line global.  Many Inform games still store a
// location-ish object in global 0, so we try the same strategy.  This is a
// best-effort heuristic; the automapper's "unknown direction" mechanism handles
// the occasional wrong or missing value gracefully.
//
// # Object-tree enumeration bounds
//
// The Z-machine does not store the object count explicitly.  We infer it from
// the layout: objects are stored in a compact array immediately after the
// property-defaults table; each object entry contains a pointer to its own
// property table.  The smallest property-table address found across all entries
// marks where the object entries array ends, because property tables are always
// placed after the object entries in well-formed story files.
//
// Concretely: iterate candidate objects starting from 1.  For each candidate,
// read the property-table pointer stored in its entry.  If that pointer is less
// than or equal to the start of the current candidate's own entry (meaning the
// pointer points back into the entry region itself), we have run past the end of
// the real object table.  We also stop if the pointer is zero.  A reasonable
// absolute cap of 2000 objects is applied to guard against malformed data.
//
// **Documented limitations:**
//   - The v4+ location is a best-effort guess; wrong answers are expected
//     occasionally and the automapper is designed to tolerate them.
//   - Object-count inference can be wrong for unusual story layouts (hand-crafted
//     or very old files where property tables are interleaved with entries).
//   - v8 and v7 stories use the same heuristic as v4+ for location.

use crate::cpu::exec::Machine;
use crate::objects::{
    entries_base, entry_size, get_parent, object_snapshot, prop_table_ptr_offset, short_name,
    ObjectSnapshot, ParseNames,
};
use crate::screen::{UpperWindow, V6Cell};

/// Normalize for matching/hashing: trim, collapse whitespace, lowercase.
pub(crate) fn normalize_name(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Strip the posture suffix after the first comma, then trim.
fn clean_room_text(s: &str) -> String {
    s.split(',').next().unwrap_or(s).trim().to_string()
}

/// Map Unicode box-drawing (U+2500–U+257F) and block-element (U+2580–U+259F)
/// glyphs to a space for status-line PARSING only (never for display).
///
/// BeyondZork's VT220 mode frames the centered room title with half-block bars
/// (`▐`…`▌`, U+2590/U+258C); without this the leading bar reads as a bogus
/// left-justified "room name" and defeats the centered fallback. Ordinary room
/// names never contain these glyphs, so this is a no-op for every other game.
fn deframe(ch: char) -> char {
    if ('\u{2500}'..='\u{259F}').contains(&ch) {
        ' '
    } else {
        ch
    }
}

/// Extract a candidate room name from the v4+ status-line grid, or None.
///
/// Scans at most the first 2 active rows. Prefers a `Location:` label segment;
/// otherwise takes row 1's first segment (text before the first run of 2+
/// spaces, which separates the left-justified room name from the right-aligned
/// score/moves/time block). Strips a trailing posture suffix after a comma.
pub fn status_line_room_name(upper: &UpperWindow, active_rows: u16) -> Option<String> {
    let scan = active_rows.min(2).min(upper.rows);
    let row_text = |r: u16| -> String {
        let mut s = String::new();
        for c in 1..=upper.cols {
            s.push(deframe(upper.cell(r, c).ch));
        }
        s
    };

    // 1. Label form: any scanned row containing a "Location:" segment.
    for r in 1..=scan {
        let line = row_text(r);
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find("location:") {
            let after = line[idx + "location:".len()..].trim_start();
            let value = after.split("  ").next().unwrap_or("").trim();
            let candidate = clean_room_text(value);
            if !candidate.is_empty() {
                return Some(candidate);
            }
        }
    }

    // 2. Common form: row 1's first segment (before the first 2+ space run).
    if scan >= 1 {
        let line = row_text(1);
        let first = line.split("  ").next().unwrap_or("").trim();
        let candidate = clean_room_text(first);
        if !candidate.is_empty() {
            return Some(candidate);
        }
    }

    None
}

/// The values of the `Label: value` fields painted on the status line, in the
/// order they are painted.
///
/// A status line is not always a room name with a score block beside it. *The
/// Impossible Stairs* paints `" Year: 2001  Place: Front Lawn"`: the room is the
/// value of a LABELLED FIELD, and the left-justified first segment the common
/// form takes — `"Year: 2001"`, or here the whole row, since the fields are
/// separated by single spaces — is not a room at all. Believing it mints a brand
/// new room every time the year changes, for a place the player never left.
///
/// Nothing here knows which label means "room", and it must not — a rule keyed to
/// the word `Place` would be a per-title special case in disguise. Every value is
/// handed to `detect_location`, which subjects it to the SAME object-tree
/// validation the ordinary candidate gets and believes only what the avatar's own
/// ancestor chain confirms. A score, a date or a time simply fails to validate,
/// so the parse does not have to be perfect — only generous.
///
/// The parse: split a row on `':'`. Each colon ends a label and opens a value,
/// and that value runs up to the start of the NEXT label, which is the last word
/// before the next colon. So `" Year: 2001  Place: Front Lawn"` yields `"2001"`
/// and `"Front Lawn"`, and `" Location:  Foo Bar   Date:  3/16/2031"` yields
/// `"Foo Bar"` and `"3/16/2031"`.
fn status_line_field_values(upper: &UpperWindow, active_rows: u16) -> Vec<String> {
    let scan = active_rows.min(2).min(upper.rows);
    let mut out = Vec::new();
    for r in 1..=scan {
        let line: String = (1..=upper.cols).map(|c| deframe(upper.cell(r, c).ch)).collect();
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 2 {
            continue;
        }
        for (i, part) in parts.iter().enumerate().skip(1) {
            // Every chunk but the last ends with the next field's label; drop it.
            let value = if i + 1 < parts.len() {
                match part.trim_end().rsplit_once(char::is_whitespace) {
                    Some((head, _label)) => head,
                    // The whole chunk is the next label, so this field has no value.
                    None => continue,
                }
            } else {
                part
            };
            let candidate = clean_room_text(value);
            if !candidate.is_empty() {
                out.push(candidate);
            }
        }
    }
    out
}

/// Centered-title fallback for row 1 when the common form yields nothing.
///
/// Some v4+ games (e.g. BeyondZork) CENTER the room name in row 1 with leading
/// padding and put stats on row 2, so the left-justified first segment (text
/// before the first 2+-space run) is empty and `status_line_room_name` returns
/// `None`. This retries on the TRIMMED line, taking its first 2+-space segment.
///
/// It fires ONLY where the left-justified first segment is empty, so it never
/// changes left-justified behavior. A centered line can be a banner/title
/// rather than a room, so `detect_location` accepts this result only under the
/// strongest validation — the avatar's ancestor chain reaches a room of this
/// name (PlayerParent). A bare name match (StatusName) or unvalidated `NameOnly`
/// is NOT trusted here.
fn centered_status_line_room_name(upper: &UpperWindow, active_rows: u16) -> Option<String> {
    let scan = active_rows.min(2).min(upper.rows);
    if scan < 1 {
        return None;
    }
    let line: String = (1..=upper.cols).map(|c| deframe(upper.cell(1, c).ch)).collect();
    // Only a centered title: the left-justified first segment must be empty
    // (the line begins with 2+ spaces). Otherwise the common form handled it.
    if !line.split("  ").next().unwrap_or("").trim().is_empty() {
        return None;
    }
    let first = line.trim().split("  ").next().unwrap_or("").trim();
    let candidate = clean_room_text(first);
    if candidate.is_empty() {
        None
    } else {
        Some(candidate)
    }
}

// ── v6 (graphical) status-band room extraction ─────────────────────────────────
//
// v6 games never populate the v4+ upper-window GRID; their status text is PAINT
// in the v6 window model (`machine.screen.v6` — each window's `texts` are runs
// with screen-absolute 1-based pixel x/y, ZMSD §8.8). The extractor below reads
// those paint runs and yields ordered room-name candidates for the SAME
// validation ladder the grid path uses.
//
// Grounded in the four real v6 titles at the 640×400 geometry (SQ-0479 re-trace,
// re-probed headlessly for SQ-0530; `story_top` is window 0's `y_coord`):
//   - Zork Zero: story_top=79; window 1 status band at y_coord=1. Room name is
//     the LEFT run on the top text row (y=11, x=71 → dx=70): "Banquet Hall" →
//     "Scullery". Score/Moves on the next row (y=27). The kingdom "Flatheadia" is
//     a RIGHT-anchored field (x=489, dx=488).
//   - Shogun: story_top=33; window 1 (x_coord=47), two rows. Row 1 (y=1):
//     "Erasmus:" (the ship, a LABELLED field — a ":" glyph abuts it, x=49 dx=2) +
//     "SHOGUN" (a CENTERED banner, x=296 dx=249) + Score (x=504 dx=457). Row 2
//     (y=17): "Bridge" (the room, left, x=49 dx=2) + Moves. The avatar is NOT
//     parented to the room object here, so only StatusName (resolve_room_object)
//     recovers "Bridge"; "Erasmus" and "SHOGUN" must be rejected.
//   - Arthur: story_top=209 — the status band is NOT at the top of the screen.
//     Arthur reserves native rows 0–11 for a graphics panel and paints its status
//     into a one-row window 1 (y_coord=193, y_size=16) sandwiched directly above
//     the story window. The room is the LEFT field ("Churchyard", x=37 dx=8) and
//     the date ("St Anne's Day, Compline", x=420 dx=391) the right one — emitted
//     as 72 SINGLE-GLYPH runs, one per cell, not as whole words.
//   - Journey: story_top=1 — the story window owns the top of the screen, so the
//     band is empty and there are no candidates → None (correct; Journey is
//     menu-driven and paints its command menu at y≥305, BELOW the story).

/// A field is "left-anchored" (a classic room-name slot, eligible for the weaker
/// StatusName path) when it starts within this many pixels of its window's left
/// edge. Re-grounded on fresh 640×400 traces (SQ-0479/SQ-0530): room names sit
/// dx≤~72 (Zork0 "Banquet Hall"/"Scullery" x=73 in a x_coord=1 window → dx=72;
/// Shogun "Bridge" x=49 in a x_coord=47 window → dx=2; Arthur "Churchyard" x=41
/// in a x_coord=29 window → dx=12); centered/right fields sit far out (Shogun
/// "SHOGUN" banner dx=250, Score dx=457; Zork0 "Flatheadia" dx=488; Arthur's
/// date dx=392). 96px (12 cells) divides them with margin — above every room
/// name, below every banner/score.
const V6_LEFT_ANCHOR_MAX_DX: u16 = 96;

/// Tallest window that can be a status STRIP overlaying the story window: two text
/// rows. A real v6 status bar is one or two rows pinned to the top of the screen
/// (advent.z6's is 20px, a single row); anything taller that starts up there is a
/// panel or a full-screen overlay, not a bar — Journey's menu window is the whole
/// 640×400 screen, and treating IT as a status band would mine menu labels for room
/// names. Zork Zero's 78px band and Arthur's 12-rows-down bar are unaffected either
/// way: they sit genuinely ABOVE their story window and are found by that rule.
fn v6_status_strip_max_h(cell: V6Cell) -> u16 {
    2 * cell.h()
}

/// One v6 status-band candidate: the cleaned room text and whether it was
/// left-anchored. Left-anchored runs are tried for StatusName; centered/other
/// runs are PlayerParent-only (a centered run is usually a banner, e.g. "SHOGUN").
struct V6Candidate {
    name: String,
    left_anchored: bool,
}

/// True for a status FIELD that is never a room name: a score/moves/turns/time
/// label, or a run with no letters at all (bare numbers, ":", padding).
fn is_v6_stat_field(s: &str) -> bool {
    let n = normalize_name(s);
    if n.is_empty() {
        return true;
    }
    if ["score", "moves", "turns", "time"].iter().any(|k| n.contains(k)) {
        return true;
    }
    !n.chars().any(|c| c.is_alphabetic())
}

/// One field of a rasterized status row: its text, the screen pixel `x` of its
/// first glyph, and the left edge of the window that painted it.
struct V6Segment {
    text: String,
    x: u16,
    win_x: u16,
}

/// Rasterize one paint row into a virtual character line, then cut it into the
/// same 2+-space-separated FIELDS the v4+ grid path uses.
///
/// v6 text is paint, not a grid, and games are free to emit a status row however
/// they like: Zork Zero paints whole words ("Banquet Hall"), while Arthur emits
/// the SAME row as 72 single-glyph runs, one per cell. Reading runs individually
/// therefore turns Arthur's "Churchyard" into ten one-letter candidates. Laying
/// the glyphs back down on their 8-px columns recovers the row the player
/// actually sees, and then the ordinary "two or more spaces separate fields"
/// rule splits the location from the score/date block for every title at once.
/// Overlapping repaint is handled naturally — the later glyph wins the cell.
fn v6_row_segments(runs: &[(&crate::screen::V6Text, u16)], cell: V6Cell) -> Vec<V6Segment> {
    let mut line: Vec<char> = Vec::new();
    let mut owner: Vec<u16> = Vec::new();
    // The screen pixel each column's glyph was painted at, kept only so a field can
    // report where it starts; the LINE itself is addressed in columns.
    let mut pen: Vec<u16> = Vec::new();
    for (t, win_x) in runs {
        // The column the engine's own grid cursor held (SQ-1048), not `t.x / cell.w`.
        // Dividing the pixel is only the column while the pen advances one declared
        // cell per glyph: on Arthur's Amiga press it climbs ~1.3 columns per glyph,
        // so the derivation leaves gaps mid-word, the 2+-space field rule cuts there,
        // and "Churchyard" arrives as "C hur chy ard" — a name no object answers to.
        let start = usize::from(t.gcol);
        for (i, ch) in t.text.chars().enumerate() {
            let c = start + i;
            if c >= line.len() {
                line.resize(c + 1, ' ');
                owner.resize(c + 1, *win_x);
                pen.resize(c + 1, 0);
            }
            line[c] = deframe(ch);
            owner[c] = *win_x;
            // Exact for i=0, and for the rest wherever the pen is fixed. A
            // proportional machine emits one run per glyph (that is how the drift
            // above is visible at all), so i>0 does not arise there.
            pen[c] = t.x.saturating_add((i as u16).saturating_mul(cell.w()));
        }
    }

    let mut out = Vec::new();
    let n = line.len();
    let mut i = 0usize;
    while i < n {
        if line[i] == ' ' {
            i += 1;
            continue;
        }
        let start = i;
        let mut last = i;
        let mut j = i;
        while j < n {
            if line[j] != ' ' {
                last = j;
                j += 1;
            } else if j + 1 < n && line[j + 1] == ' ' {
                break; // two or more spaces end the field
            } else {
                j += 1; // a single space is part of the field ("Banquet Hall")
            }
        }
        out.push(V6Segment {
            text: line[start..=last].iter().collect(),
            x: pen[start],
            win_x: owner[start],
        });
        i = last + 1;
    }
    out
}

/// True when window `i` is a status STRIP laid over the top of the story window: a
/// short window that is not the prose window itself, pinned to the top of the SCREEN,
/// whose bottom edge reaches past the story window's top. "Pinned to the top" is what keeps Shogun's bottom
/// menu window (y=337, the same y as its story window, 48px tall) from qualifying —
/// a bar overlaying the story starts at the screen top or not at all.
fn is_v6_status_strip(
    i: usize,
    prose_idx: usize,
    w: &crate::screen::ZWindow,
    story_top: u16,
    cell: V6Cell,
) -> bool {
    i != prose_idx
        && w.y_size > 0
        && w.y_size <= v6_status_strip_max_h(cell)
        && w.y_coord <= cell.h()
        && w.y_coord + w.y_size > story_top
}

/// Ordered v6 status-band room candidates: left-anchored fields first (top rows
/// first), then centered/other fields. A pure read of the v6 paint model; empty
/// when the story is not v6 or paints no text above the story window.
///
/// The band is **everything painted above the story window's top edge** — window
/// 0 is v6's main text window (ZMSD §8.4), so its `y_coord` is where the prose
/// begins and the status furniture necessarily sits above it. That is derived per
/// game rather than assumed: Zork Zero and Shogun open the story at y=79/33 and
/// really do put the bar at the top of the screen, but Arthur reserves the top
/// twelve rows for a graphics panel and hangs its bar at y=193, immediately above
/// a story window that starts at y=209. A fixed "first N rows" cut-off finds the
/// first two and never finds Arthur (SQ-0530). Journey falls out for free: its
/// story window owns y=1, so nothing is above it and a menu screen yields no room.
fn v6_status_candidates(machine: &Machine) -> Vec<V6Candidate> {
    // SQ-0917: the session's cell, which every pixel-to-column step below divides by.
    let cell = machine.v6_cell();
    let Some(v6) = machine.screen.v6.as_ref() else {
        return Vec::new();
    };
    // The window the game streams prose through — window 0 for Infocom, window 7 for
    // Inform 6's v6 library, decided by the same wrap+scroll test the printer uses
    // (SQ-0459/SQ-0583). advent.z6 never touches window 0, so window 0 keeps its
    // boot-time full-screen rect: reading the band above THAT finds nothing once the
    // game splits the screen and moves its status bar down beside the real prose.
    let prose_idx = {
        let cur = v6.current as usize;
        if v6.windows[cur].attributes & 0b11 == 0b11 { cur } else { 0 }
    };
    let story_top = v6.windows[prose_idx].y_coord.max(1);

    use std::collections::BTreeMap;
    // Group band runs by row (absolute y), carrying each run's window left edge
    // so left-anchoring is measured relative to the window, not the screen.
    let mut rows: BTreeMap<u16, Vec<(&crate::screen::V6Text, u16)>> = BTreeMap::new();
    for (i, w) in v6.windows.iter().enumerate() {
        // A status STRIP overlays the story window instead of sitting above it
        // (SQ-0581): advent.z6 leaves window 0 covering the whole screen and hangs
        // window 1 — one row tall, pinned at the top — over its first row, painting
        // "At End Of Road   Score: 36   Moves: 1" there. Nothing is above the story
        // window, so the rule below finds no band at all. Such a strip IS the band,
        // and only its own rows are: window 0's prose is never scooped in, because
        // window 0 can't be a strip.
        let strip_bottom =
            is_v6_status_strip(i, prose_idx, w, story_top, cell).then(|| w.y_coord + w.y_size);
        for t in w.texts.iter() {
            if t.text.is_empty() {
                continue;
            }
            // Wholly above the story text: a run straddling the boundary is prose.
            let above_story = t.y + cell.h() <= story_top;
            let in_strip = strip_bottom.is_some_and(|b| t.y + cell.h() <= b);
            if above_story || in_strip {
                rows.entry(t.y).or_default().push((t, w.x_coord));
            }
        }
    }
    let mut left = Vec::new();
    let mut other = Vec::new();
    for (_y, mut runs) in rows {
        runs.sort_by_key(|(t, _)| t.x);
        for seg in v6_row_segments(&runs, cell) {
            if is_v6_stat_field(&seg.text) {
                continue;
            }
            // Label field (e.g. Shogun's "Erasmus:"): a field ending in a colon
            // names the value beside it, so it is never itself a room — skip,
            // mirroring the grid path's "Location:" handling.
            if seg.text.ends_with(':') {
                continue;
            }
            let cand = clean_room_text(&seg.text);
            if cand.is_empty() {
                continue;
            }
            let left_anchored = seg.x.saturating_sub(seg.win_x) <= V6_LEFT_ANCHOR_MAX_DX;
            if left_anchored {
                left.push(V6Candidate { name: cand, left_anchored: true });
            } else {
                other.push(V6Candidate { name: cand, left_anchored: false });
            }
        }
    }
    left.into_iter().chain(other).collect()
}

/// Ordered v6 status-band room-name candidates (left-anchored first). Pure read
/// of the v6 paint model; empty for non-v6 stories or top-band-less screens.
/// Public for introspection/tests; `detect_location` uses the richer internal
/// form that also tracks left-anchoring for the StatusName gate.
pub fn v6_status_room_candidates(machine: &Machine) -> Vec<String> {
    v6_status_candidates(machine).into_iter().map(|c| c.name).collect()
}

/// v6 location via the same validation ladder as the grid path, but sourced from
/// the v6 paint runs (`v6_status_candidates`).
///
/// 1. PlayerParent (strongest) across ALL candidates, left then centered: the
///    first candidate whose name is reached by some avatar's ancestor chain wins.
///    This is what makes Zork Zero work (its avatar sits directly in the room
///    object) and what would reject a banner even if it were centered.
/// 2. StatusName for LEFT-ANCHORED candidates only (mirrors the grid): resolve
///    the name to a real object, preferring the player_room_beside form. This is
///    the ONLY thing that recovers Shogun's "Bridge" (whose avatar is not
///    parented to the room). Centered runs are never StatusName'd — a centered
///    run is a banner far more often than a room.
/// 3. `global_room_by_shown_text` — the room's NAME is a property and the
///    pointer to the current room is a GLOBAL. See that function; it is the only
///    thing that maps the Mysterious Adventures ports (SQ-0724).
/// 4. NameOnly is DROPPED for v6: with no backing object a v6 candidate is far
///    more likely a title/character-sheet/banner than a room, and there is no
///    grid-shaped left-justified discipline to lean on. Returning None on a
///    title/menu screen is the correct answer, so an object-less candidate yields
///    None rather than inventing a room.
fn detect_location_v6(machine: &Machine) -> Option<Location> {
    let cands = v6_status_candidates(machine);
    // The avatar set does not depend on the candidate, and `player_candidates`
    // walks the whole object table decoding every short name (and, since
    // SQ-1259, every parse-name property) — hoist both it and its `ParseNames`
    // reader out of the loop (SQ-1183).
    let parse_names = ParseNames::detect(&machine.mem);
    // 1. PlayerParent across all candidates.
    let players = player_candidates(machine, parse_names.as_ref());
    for cand in &cands {
        for &player in &players {
            if let Some(room) = nearest_matching_ancestor(machine, player, &cand.name) {
                return Some(Location::PlayerParent(room));
            }
        }
    }
    // 2. StatusName for left-anchored candidates only.
    for cand in cands.iter().filter(|c| c.left_anchored) {
        if let Some(shown) = resolve_room_object(machine, &cand.name) {
            if let Some(room) = player_room_beside(machine, &shown) {
                return Some(Location::PlayerParent(room));
            }
            return Some(Location::StatusName(shown));
        }
    }
    // 3. The room object exists but is not named by its short name, and no avatar
    //    is parented into it. Corroborate a global against the shown text.
    if let Some(room) = global_room_by_shown_text(machine, &cands) {
        return Some(Location::StatusName(room));
    }
    // 4. No object-backed candidate: do NOT invent a NameOnly room for v6.
    None
}

/// Shortest candidate this rung will corroborate a global against, normalized.
/// Every real room description clears it by a wide margin ("I'm in a Cave"), and
/// it keeps a stray one- or two-glyph run — the paint model can emit those — from
/// prefix-matching an arbitrary object's text and blessing an arbitrary global.
const V6_GLOBAL_ROOM_MIN_LEN: usize = 6;

/// The room object a GLOBAL points at, confirmed by the text the game is
/// painting right now — the last v6 rung, and the one the Mysterious Adventures
/// ports need (SQ-0724).
///
/// Brian Howarth's *Mysterious Adventures* ship as Inform 6 re-implementations of
/// the Scott Adams engine, and they defeat every rung above for two independent
/// reasons at once. Their avatar (`player`, object #5) has parent 0 — it is never
/// put into the tree at all, so PlayerParent has nothing to walk. And every room
/// object carries the *same* compiled short name, `ScottRoom`, with the text the
/// player reads ("I'm in a dense SPOOKY Forest") held in a PROPERTY instead — so
/// `resolve_room_object`, which matches short names, can never name one either.
///
/// What the games do have is a global holding the current room's object number,
/// and that is the signal worth taking: it is exact. The alternative — matching
/// the shown text against every object's properties — collapses mazes, because
/// these games reuse a description across many rooms (mysterious07 has ten rooms
/// that all read "I'm in a Tunnel", mysterious05 eight "ventilation ducts"). The
/// global distinguishes them; a name never can.
///
/// A global is not trusted for being a global. It is trusted only when the object
/// it names carries, in one of its own properties, the very text the status band
/// is painting this turn — so the object tree and the screen corroborate each
/// other, every turn, and no global INDEX is hard-coded. The returned snapshot
/// takes its name from that property, because the object's short name is the
/// useless `ScottRoom`.
fn global_room_by_shown_text(machine: &Machine, cands: &[V6Candidate]) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    let max_obj = max_object_number(mem);
    if max_obj == 0 {
        return None;
    }
    // The 240 globals (ZMSD §6.2), deduplicated and filtered to plausible object
    // numbers: these games mirror the room pointer into several globals, and the
    // duplicates all name the same object. Slots past the end of memory are
    // skipped rather than read — a short or hand-built story must not be turned
    // into a memory fault by our own scan.
    let base = mem.global_vars() as u32;
    let mut objs: Vec<u16> = (0..240u32)
        .map(|i| base + i * 2)
        .take_while(|&at| at as usize + 1 < mem.len())
        .map(|at| mem.read_word(at))
        .filter(|&v| v != 0 && v <= max_obj)
        .collect();
    objs.sort_unstable();
    objs.dedup();

    for cand in cands {
        if normalize_name(&cand.name).len() < V6_GLOBAL_ROOM_MIN_LEN {
            continue;
        }
        for &obj in &objs {
            if let Some(text) = object_text_property(machine, obj, &cand.name) {
                return Some(ObjectSnapshot { name: text, ..object_snapshot(mem, obj) });
            }
        }
    }
    None
}

/// The text of `obj`'s first property that reads as a string the status band is
/// showing as `name`, or None.
///
/// A word-sized property holding a PACKED string address is how Inform stores a
/// printable name it does not put in the short name. Every such property is
/// unpacked and decoded, and the decoded text is accepted only if `name` matches
/// it by the ordinary [`status_name_matches`] rule — with the *property* as the
/// full text and the shown `name` as its leading part, because the band's text is
/// what gets clipped: `clean_room_text` cuts it at the first comma, and a long
/// description can be cut again by the window's width.
///
/// Most of this is guessing, and the guesses are kept harmless two ways. A word
/// that is not really a string address is rejected up front unless it reaches a
/// terminated Z-string of plausible length ([`is_zstring`]), and the decode
/// itself runs under [`Memory::without_fault_latch`] — an abbreviation inside a
/// mis-guessed string can still point anywhere, and a speculative read landing
/// out of bounds must answer "not a string", never fault the story.
fn object_text_property(machine: &Machine, obj: u16, name: &str) -> Option<String> {
    use crate::objects::{get_prop_addr, get_prop_len, property_numbers};
    let mem = &machine.mem;
    // `property_numbers` and not a bare `get_next_prop` walk: a table listing
    // the same number twice makes that loop cycle forever, and one does
    // (SQ-1143).
    for prop in property_numbers(mem, obj) {
        let addr = get_prop_addr(mem, obj, prop);
        if get_prop_len(mem, addr) == 2 {
            let packed = mem.read_word(addr as u32);
            let str_addr = mem.unpack_string(packed);
            if packed != 0 && is_zstring(mem, str_addr) {
                let text =
                    mem.without_fault_latch(|| crate::text::decode::decode_string(mem, str_addr).0);
                if status_name_matches(&text, name) {
                    return Some(text);
                }
            }
        }
    }
    None
}

/// Longest Z-string the room-text probe will consider, in words (3 Z-chars each).
/// A room description is a line of a status band; 48 words is 144 Z-chars, well
/// past any of them and short enough that a wrong guess costs nothing.
const ZSTRING_PROBE_MAX_WORDS: u32 = 48;

/// Whether the bytes at `addr` are a Z-string: terminated within
/// [`ZSTRING_PROBE_MAX_WORDS`] without running off the end of the story file.
/// Reads only words that lie wholly inside memory, so it can never latch a fault
/// of its own.
fn is_zstring(mem: &crate::memory::Memory, addr: u32) -> bool {
    (0..ZSTRING_PROBE_MAX_WORDS)
        .map(|i| addr + i * 2)
        .take_while(|&at| at as usize + 1 < mem.len())
        .any(|at| mem.read_word(at) & 0x8000 != 0)
}

/// True if `short` names the room shown as `candidate`: equality, or `short` is
/// a leading prefix of `candidate` ending on a word boundary (next char
/// non-alphanumeric, or end of string). Both normalized; `short` non-empty.
pub fn status_name_matches(candidate: &str, short: &str) -> bool {
    let c = normalize_name(candidate);
    let s = normalize_name(short);
    if s.is_empty() {
        return false;
    }
    if c == s {
        return true;
    }
    match c.strip_prefix(&s) {
        Some(rest) => rest.chars().next().is_none_or(|ch| !ch.is_alphanumeric()),
        None => false,
    }
}

/// The current player object: among all objects whose normalized short name is
/// a plausible avatar name (see `PLAYER_NAMES`), the one that is actually WHERE
/// THE PLAYER IS — i.e. whose ancestor chain reaches the room `detect_location`
/// reports. None if no candidate exists.
///
/// A name alone does not identify the avatar. Zork 1 (r52) ships two named
/// candidates: #21 "you" — the parser's stand-in for the player as a *noun*,
/// parked in the "it" pseudo-container with the other globals — and #46
/// "cretin", the real avatar sitting in the room. Both are "situated" (non-zero
/// parent), so preferring the lowest-numbered situated candidate (SQ-0212)
/// picks the noun and reports an inventory that is empty forever (SQ-0668).
/// Only "cretin" is ever inside a room, which is exactly the discrimination
/// `detect_location` already performs to name the room in the first place — so
/// reuse it rather than inventing a second, weaker rule.
///
/// Mirrors `gvm::objects::ParseNames::find_player`'s rule exactly, because it
/// is the same problem with the same trap (see that function's doc comment):
/// among several candidates, prefer the SITUATED ones (non-zero parent —
/// Inform parks its off-stage doubles at the top level, and a player stands
/// somewhere); one survivor needs no further discrimination; otherwise the
/// avatar is the candidate whose containment chain reaches the room
/// `detect_location` itself confirms.
///
/// **And where that cannot settle it, the answer is `None` — there is no
/// "first plausible candidate" fallback**, because a wrong avatar is worse
/// than no avatar: its children become an inventory the player is told they
/// are carrying. This never returns an unsituated candidate when a situated
/// one exists, and never returns a candidate that fails the room test when
/// the room is known.
///
/// Not recursive: `detect_location` discriminates avatars with
/// `player_candidates` + `nearest_matching_ancestor` directly and never calls
/// back into this function. Keep it that way.
pub fn find_player_object(machine: &Machine) -> Option<u16> {
    let parse_names = ParseNames::detect(&machine.mem);
    let cands = player_candidates(machine, parse_names.as_ref());
    // One candidate needs no discrimination — and skipping `detect_location`
    // keeps the common case (most Inform games, minizork) as cheap as it was.
    if cands.len() < 2 {
        return cands.first().copied();
    }
    let situated: Vec<u16> =
        cands.iter().copied().filter(|&obj| get_parent(&machine.mem, obj) != 0).collect();
    let pool: &[u16] = if situated.is_empty() { &cands } else { &situated };
    if pool.len() < 2 {
        return pool.first().copied();
    }
    let room = detect_location(machine).and_then(|l| l.object().map(|o| o.number));
    pool.iter().copied().find(|&obj| room.is_some_and(|r| has_ancestor(machine, obj, r)))
}

/// True when `ancestor` is a strict ancestor of `start` in the object tree.
/// Depth-bounded (32) to tolerate cycles, like `nearest_matching_ancestor`.
/// Walks the chain rather than testing the parent alone so an avatar riding
/// inside a vehicle (Zork's boat) still counts as being in the room.
fn has_ancestor(machine: &Machine, start: u16, ancestor: u16) -> bool {
    let mut cur = get_parent(&machine.mem, start);
    for _ in 0..32 {
        if cur == 0 {
            return false;
        }
        if cur == ancestor {
            return true;
        }
        cur = get_parent(&machine.mem, cur);
    }
    false
}

/// Object short-names that plausibly denote the player avatar, including ZIL's
/// "cretin"/"adventurer" and the bare "player" Planetfall and Deadline give
/// theirs. The set may be generous: both consumers validate a candidate against
/// the room before trusting it (`detect_location`, `find_player_object`), so a
/// name that is not the avatar simply fails to validate and is skipped.
///
/// `(self object)` is the Inform 6 library's own `selfobj` — the avatar of every
/// Inform 6 game that never calls `ChangePlayer` — and the entry that was
/// missing (SQ-0701). The name is the literal short name compiled into the story
/// (games that print "yourself" do it from a `short_name` routine, which is code,
/// not the string we read), and it turns up as object #20 in every Inform 6 title
/// checked: anchor.z8, photopia.z5, LostPig.z8, nameless.z8.
///
/// Anchorhead is why it matters. Its only *matching* candidate was a conversation
/// TOPIC named "yourself" parked in `(con_topics)` alongside "Michael" and
/// "lighthouse", while the real avatar — `(self object)`, correctly parented to
/// the room — was invisible here. The topic never validates, so detection fell
/// through to `player_room_beside`, which walked one step up from the topic and
/// reported the topic BAG as the room: the same object `(con_topics)` for every
/// room in the game, so the automap saw one room and never a single connection.
const PLAYER_NAMES: [&str; 9] =
    ["yourself", "you", "me", "myself", "self", "cretin", "adventurer", "player", "(self object)"];

/// Parse-name WORDS (as opposed to PRINTED short names) that plausibly denote
/// the player avatar, matched via [`ParseNames::of`]/`ObjectWords::refers_to`
/// against every object's `name` array — mirroring
/// `gvm::objects::ParseNames::find_player`'s `PLAYER_WORDS`, which solves the
/// identical problem on Glulx.
///
/// This is what finds an avatar with no avatar-ish PRINTED name at all. Lost
/// Pig's Grunk (#87) prints "Grunk", nothing in [`PLAYER_NAMES`] — the game
/// never uses Inform's `selfobj`, so `(self object)` (#20, present but
/// parentless) is a decoy — and the only signal that #87 is the avatar is
/// that its parse words are `["grunk", "green", "orc", "me"]` (SQ-1259). Both
/// standard libraries put one of these words on their avatar: Inform 6's
/// `selfobj` carries `'me' 'myself' 'self'`, and Inform 7's Standard Rules
/// `Understand "yourself" or "myself" or "self" as yourself`.
///
/// gvm's own list drops `me`/`you`/`player` as too noisy for parse-word
/// matching alone (measured on `CounterfeitMonkey-11.gblorb`: they pull in
/// conversation quips like "what he thinks of you"). This list keeps `me`
/// anyway, because Lost Pig needs exactly that word and both consumers here
/// — `detect_location` and `find_player_object` — validate every candidate
/// against the room before trusting it (see [`PLAYER_NAMES`]'s doc comment):
/// a quip's word array never validates, because a quip is never IN a room.
const PLAYER_WORDS: [&str; 4] = ["me", "myself", "self", "yourself"];

/// All objects whose short name plausibly denotes the player avatar (see
/// [`PLAYER_NAMES`]), OR whose parse words include one of [`PLAYER_WORDS`], in
/// ascending object order. `detect_location` validates each against the
/// status-line room; `find_player_object` validates each against the room the
/// object tree itself confirms.
///
/// `parse_names` is the caller's own [`ParseNames::detect`] reader — building
/// one walks every object's property table, so every top-level caller builds
/// it once and threads it through rather than paying for it again per
/// candidate list.
fn player_candidates(machine: &Machine, parse_names: Option<&ParseNames>) -> Vec<u16> {
    let n = max_object_number(&machine.mem);
    (1..=n)
        .filter(|&obj| {
            let nm = normalize_name(&short_name(&machine.mem, obj));
            if PLAYER_NAMES.contains(&nm.as_str()) {
                return true;
            }
            parse_names
                .and_then(|pn| pn.of(&machine.mem, obj))
                .is_some_and(|words| PLAYER_WORDS.iter().any(|w| words.refers_to(w)))
        })
        .collect()
}

/// The narrower, short-name-only half of [`player_candidates`] — [`PLAYER_NAMES`]
/// alone, none of [`PLAYER_WORDS`]. The one caller that needs it is
/// [`player_room_beside`], whose own heuristic carries no name check at all
/// (it trusts "shares the shown room's PARENT", nothing about the candidate's
/// own name or words) — so a candidate admitted only by a parse word is a much
/// weaker bet there than everywhere else `player_candidates` is used, where a
/// widened candidate still has to pass a real name match
/// ([`nearest_matching_ancestor`]) or an exact ancestor-reaches-the-room test
/// ([`find_player_object`]).
///
/// Nord and Bert — a game built entirely out of wordplay jokes — is why this
/// matters concretely (SQ-1259). Widening turned up object #89: printed name
/// EMPTY, admitted only because its parse words include "myself" (some `x
/// myself` joke, unrelated to the avatar), parented under #92 "Jean Stock".
/// Fed into `player_room_beside`'s loose test, #89's parent #92 shares its
/// OWN parent with the real hub room "Beginning" (both sit in the `it`
/// globals pseudo-container) — so the heuristic reported "Jean Stock" as the
/// room the status line names "Beginning". Restricting this one caller back
/// to the short-name-only pool is what keeps a joke object out of it, without
/// giving up the widening everywhere the widening is actually safe.
fn player_candidates_by_name(machine: &Machine) -> Vec<u16> {
    let n = max_object_number(&machine.mem);
    (1..=n)
        .filter(|&obj| PLAYER_NAMES.contains(&normalize_name(&short_name(&machine.mem, obj)).as_str()))
        .collect()
}

/// Nearest ancestor of `start` (exclusive) whose short name matches `name` via
/// `status_name_matches`. Depth-bounded (32) to tolerate cycles.
fn nearest_matching_ancestor(machine: &Machine, start: u16, name: &str) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    let mut cur = get_parent(mem, start);
    for _ in 0..32 {
        if cur == 0 {
            break;
        }
        if status_name_matches(name, &short_name(mem, cur)) {
            return Some(object_snapshot(mem, cur));
        }
        cur = get_parent(mem, cur);
    }
    None
}

/// The player's real room when the status line has gone STALE — i.e. no avatar's ancestor chain
/// reaches the room the status line names, so the TEXT and the OBJECT TREE disagree (SQ-0358).
///
/// The tree is the game's own state; the status line is a rendering the game may simply not have
/// refreshed. Zork's Loud Room is the case in point: its echo routine intercepts input, so the
/// status line keeps naming the room you *came from* for as long as you stand there, while
/// `cretin`'s parent points squarely at the Loud Room the whole time.
///
/// Nothing about a room's identity is hard-coded. The status line names a room we CAN resolve, so
/// its object teaches us what a room looks like structurally — its CONTAINER — and the player's
/// nearest ancestor sharing that container is the room they are actually in. Zork keeps every room
/// under one container object, and Inform keeps rooms at the top level (container 0); both fall out
/// of the same rule.
///
/// Candidates are tried in order and the first that yields a room wins, which is what rejects
/// decorative avatars: Zork's `you` hangs off `it` at the top level and never reaches the room
/// container, while `cretin` does.
fn player_room_beside(machine: &Machine, shown: &ObjectSnapshot) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    for player in player_candidates_by_name(machine) {
        let mut cur = get_parent(mem, player);
        for _ in 0..32 {
            // Depth-bounded like `nearest_matching_ancestor`, to tolerate cycles.
            if cur == 0 {
                break;
            }
            if get_parent(mem, cur) == shown.parent {
                return Some(object_snapshot(mem, cur));
            }
            cur = get_parent(mem, cur);
        }
    }
    None
}

/// The object the status line's `name` most likely refers to, or `None` if
/// nothing matches.
///
/// A short name is not unique, and picking the wrong owner of it reports the
/// WRONG room even though the right one is right there in the tree. Lost
/// Pig's "Outside" is two objects at once: the room (#93, parent 0) and a
/// compass direction (#18, a child of #6 "compass", whose own short name is
/// also "outside") — the old rule (longest match, ties -> lowest number)
/// picked #18, the direction, every time (SQ-1259).
///
/// Resolved in order, applied only where more than one object matches:
/// 1. The game's own `location` global, read the same way
///    [`current_location`] reads it — see that function's doc comment for why
///    it is trusted only NAME-VALIDATED like this, never on its own. If it
///    names one of the matches, that object is the room: it is the game's own
///    answer to "where is the player", not a guess from a name.
/// 2. A room is a top-level object (parent 0); a compass direction sits
///    inside the DIRECTIONS/compass object and a conversation topic inside a
///    topics bag. Prefer parent-0 matches over those.
/// 3. Longest match wins; ties -> lowest object number (the original rule,
///    unchanged, and the only rule most games ever need).
fn resolve_room_object(machine: &Machine, name: &str) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    let n = max_object_number(mem);
    let mut matches: Vec<(usize, u16)> = Vec::new(); // (normalized short-name length, object)
    for obj in 1..=n {
        let sn = short_name(mem, obj);
        if status_name_matches(name, &sn) {
            matches.push((normalize_name(&sn).len(), obj));
        }
    }
    if matches.len() > 1 {
        // (1) The game's own `location` global.
        if let Some(global_room) = current_location(machine) {
            if matches.iter().any(|&(_, obj)| obj == global_room.number) {
                return Some(global_room);
            }
        }
        // (2) Prefer a top-level object — a room — over a direction or topic.
        let top_level: Vec<(usize, u16)> =
            matches.iter().copied().filter(|&(_, obj)| get_parent(mem, obj) == 0).collect();
        if !top_level.is_empty() {
            matches = top_level;
        }
    }
    // (3) Longest match; ties -> lowest object number.
    matches
        .into_iter()
        .max_by_key(|&(len, obj)| (len, std::cmp::Reverse(obj)))
        .map(|(_, obj)| object_snapshot(mem, obj))
}

/// True when some object's short name is `name` with the spaces taken out.
///
/// A compiler's object identifier is not the name the player reads. Dialog calls
/// The Impossible Stairs' front lawn `FrontLawn`, and frankenfingers' parts room
/// is `partsRoom`: both are the printed name with its spaces elided. That is
/// plenty to ANSWER A QUESTION about the tree — which of the status line's
/// labelled fields names a room — but not to answer with, which is why the one
/// caller keeps the screen's own text as the room's label and uses this purely to
/// arbitrate. Deliberately not folded into `status_name_matches`: relaxing that
/// would start putting `partsRoom` on the map in place of "Parts Room".
fn names_an_object_ignoring_spaces(machine: &Machine, name: &str) -> bool {
    let mem = &machine.mem;
    let wanted = normalize_name(name).replace(' ', "");
    if wanted.is_empty() {
        return false;
    }
    (1..=max_object_number(mem))
        .any(|obj| normalize_name(&short_name(mem, obj)).replace(' ', "") == wanted)
}

/// How the current room was determined (drives the map indicator label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationMethod {
    GlobalVar0,
    PlayerParent,
    StatusName,
    NameOnly,
    /// Glulx: the room was read from the Inform 7 `Subheader` room heading in
    /// the story buffer (name-based; no backing object). Trusted directly — not
    /// subject to the `NameOnly`-empty-graph gate.
    RoomHeading,
}

/// The mapper-facing location signal for one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    GlobalVar0(ObjectSnapshot),
    PlayerParent(ObjectSnapshot),
    StatusName(ObjectSnapshot),
    NameOnly(String),
}

impl Location {
    /// The backing object snapshot, or None for a name-only room.
    pub fn object(&self) -> Option<&ObjectSnapshot> {
        match self {
            Location::GlobalVar0(s) | Location::PlayerParent(s) | Location::StatusName(s) => Some(s),
            Location::NameOnly(_) => None,
        }
    }
    /// The detection method tag.
    pub fn method(&self) -> LocationMethod {
        match self {
            Location::GlobalVar0(_) => LocationMethod::GlobalVar0,
            Location::PlayerParent(_) => LocationMethod::PlayerParent,
            Location::StatusName(_) => LocationMethod::StatusName,
            Location::NameOnly(_) => LocationMethod::NameOnly,
        }
    }
}

/// Best-effort current room, version-gated:
/// - v3 and below: global variable 0 -> GlobalVar0, or None.
/// - v4+: validated player-parent -> player-parent beside a stale status line -> status-name ->
///   name-only -> None.
///
/// Stateless: a pure function of the machine, re-run each turn.
pub fn detect_location(machine: &Machine) -> Option<Location> {
    if machine.mem.version() <= 3 {
        return current_location(machine).map(Location::GlobalVar0);
    }
    // v6 (graphical) games paint their status text into the v6 window model, not
    // the v4+ grid, so the grid parser below always sees an empty upper window.
    // Source the candidates from the paint runs instead, feeding the SAME ladder.
    if machine.screen.v6.is_some() {
        return detect_location_v6(machine);
    }
    // Built once and threaded through every candidate lookup below — see
    // `player_candidates`'s doc comment for why (SQ-1183/SQ-1259).
    let parse_names = ParseNames::detect(&machine.mem);
    if let Some(name) = status_line_room_name(&machine.screen.upper, machine.screen.upper_window_rows) {
        // Prefer the avatar whose ancestor chain validates against the status-line
        // room name. Trying every plausible player object (and using the first whose
        // parent chain reaches the shown room) distinguishes same-named rooms that a
        // name-only match would collapse — e.g. Zork's several "Forest" rooms — and
        // rejects decorative "you"/"self" objects whose parent never tracks the player.
        for player in player_candidates(machine, parse_names.as_ref()) {
            if let Some(room) = nearest_matching_ancestor(machine, player, &name) {
                return Some(Location::PlayerParent(room));
            }
        }
        // Nothing validated: the status line does not describe where the player is. That is the
        // signal the text has gone stale, not a reason to trust it — prefer the object tree, which
        // is the game's own state (SQ-0358).
        if let Some(shown) = resolve_room_object(machine, &name) {
            if let Some(room) = player_room_beside(machine, &shown) {
                return Some(Location::PlayerParent(room));
            }
            // No avatar reaches a room at all: the tree has nothing better to offer, so the status
            // line stands — this is what keeps games with no identifiable player object working.
            return Some(Location::StatusName(shown));
        }
        // The whole left half names nothing in the tree. The status line may still be naming the
        // room, just as one LABELLED FIELD among several — The Impossible Stairs paints
        // " Year: 2001  Place: Front Lawn", and taking the left half whole mints a brand new room
        // on every year change for a place the player never left.
        //
        // Which label means "room" is not ours to decide, and a rule keyed to the word `Place`
        // would be a per-title special case in disguise. The object tree answers instead: the
        // field that names a room is the one the tree has an object for. The tree only ARBITRATES
        // here — the name that reaches the map is the screen's own text, because the object's
        // short name is a compiler identifier the player never sees.
        for field in
            status_line_field_values(&machine.screen.upper, machine.screen.upper_window_rows)
        {
            if names_an_object_ignoring_spaces(machine, &field) {
                return Some(Location::NameOnly(field));
            }
        }
        return Some(Location::NameOnly(name));
    }
    // Centered-title fallback (BeyondZork et al.): a centered row-1 name that the
    // left-justified common form can't parse. A centered line is often a banner
    // or story title, not a room (e.g. Photopia's "Photopia by Adam Cadre", whose
    // top-level title object would satisfy a bare name match). So accept it ONLY
    // under the STRONGEST validation — the avatar's own ancestor chain reaches a
    // room of that name (PlayerParent). A mere name match (StatusName) or an
    // unvalidated NameOnly is NOT trusted here; anything else returns None.
    if let Some(name) =
        centered_status_line_room_name(&machine.screen.upper, machine.screen.upper_window_rows)
    {
        for player in player_candidates(machine, parse_names.as_ref()) {
            if let Some(room) = nearest_matching_ancestor(machine, player, &name) {
                return Some(Location::PlayerParent(room));
            }
        }
    }
    None
}

/// Returns the object representing the player's current location, or `None` if
/// the heuristic cannot determine a plausible location.
///
/// See module-level docs for the version-specific strategy. Also used by
/// [`resolve_room_object`] on v4+ stories — global 0 keeps naming *something*
/// on those (Inform's `location` variable most often, ZIL's less reliably),
/// and that function reads it purely as a disambiguator: it decides between
/// several SAME-NAME objects only when this snapshot's object number is
/// already one of them, so a global that names something unrelated on a ZIL
/// v4/v5 game (Trinity, AMFV, Bureaucracy) simply never matches and is
/// ignored. Nothing here extends v4+'s PRIMARY location signal to global 0 —
/// that stays PlayerParent/StatusName/NameOnly, exactly as before.
pub fn current_location(machine: &Machine) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    // Global variable 0 is at address `global_vars + 0` (var 0x10 maps to
    // global index 0, stored at global_vars base with no offset).
    let global0_addr = mem.global_vars() as u32;
    let obj_num = mem.read_word(global0_addr);

    if obj_num == 0 {
        return None;
    }

    // Validate that obj_num is within the object table.
    // We use the same bound logic as object_tree_view: check the entry would
    // lie before the first property table pointer within the table.
    let max_obj = max_object_number(mem);
    if obj_num > max_obj {
        return None;
    }

    Some(object_snapshot(mem, obj_num))
}

/// Returns a read-only enumeration of all objects in the story as snapshots.
///
/// Object count is inferred from the layout — see module docs for the approach.
pub fn object_tree_view(machine: &Machine) -> Vec<ObjectSnapshot> {
    let mem = &machine.mem;
    let n = max_object_number(mem);
    (1..=n).map(|i| object_snapshot(mem, i)).collect()
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Infer the maximum valid object number from the object table layout.
///
/// Object entries are stored contiguously and are immediately followed by the
/// property tables, so the LOWEST property-table pointer marks the end of the
/// entries region. We track that minimum as we scan and stop once an entry
/// would extend into (or past) it — the bytes there are property data that
/// merely resemble another entry. (The previous "pointer looks plausible"
/// check had no such bound, so it walked straight into the property tables and
/// over-counted, surfacing garbage objects with corrupt names.) Capped at 2000
/// to guard against pathological data.
pub fn max_object_number(mem: &crate::memory::Memory) -> u16 {
    let version = mem.version();
    let base = entries_base(mem);
    let esize = entry_size(version);
    let prop_ptr_offset = prop_table_ptr_offset(version);

    let mut min_ptbl = u32::MAX;
    let mut n: u16 = 0;
    for candidate in 1u16..=2000 {
        // Address of this candidate's entry.
        let entry_addr = base + (candidate as u32 - 1) * esize;
        // Stop if this entry would run past the end of the story file, or into
        // the first property table (the end of the real entries region).
        if (entry_addr + esize) as usize > mem.len() || entry_addr + esize > min_ptbl {
            break;
        }
        // Property-table pointer is the last word of the entry.
        let ptbl_addr = mem.read_word(entry_addr + prop_ptr_offset) as u32;

        // A valid property-table pointer is nonzero, points after this entry,
        // and lies within the story file (its name-length byte must be
        // readable). Anything else means we've walked past the real object
        // table into unrelated data that merely looks like an entry.
        if ptbl_addr == 0 || ptbl_addr <= entry_addr || ptbl_addr as usize >= mem.len() {
            break;
        }
        min_ptbl = min_ptbl.min(ptbl_addr);
        n = candidate;
    }
    n
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::exec::{Machine, StepResult};
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::screen::UpperWindow;

    fn upper_with(rows: &[&str]) -> UpperWindow {
        let cols = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u16;
        let mut u = UpperWindow::default();
        u.resize(rows.len() as u16, cols.max(1));
        for (r, line) in rows.iter().enumerate() {
            for (c, ch) in line.chars().enumerate() {
                u.put((r + 1) as u16, (c + 1) as u16, ch, 0, crate::screen::ZColour::Default, crate::screen::ZColour::Default);
            }
        }
        u
    }

    #[test]
    fn status_room_name_common_form_strips_score_and_posture() {
        let u = upper_with(&[" Bedroom, in the bed                              Score: 0     Moves: 1"]);
        assert_eq!(status_line_room_name(&u, 1).as_deref(), Some("Bedroom"));
    }

    #[test]
    fn status_room_name_plain() {
        let u = upper_with(&[" Darkness                                         Score: 0     Moves: 0"]);
        assert_eq!(status_line_room_name(&u, 1).as_deref(), Some("Darkness"));
    }

    #[test]
    fn status_room_name_location_label_form() {
        let u = upper_with(&[
            " Mode:  Communications Mode                                Time:  7:07pm",
            " Location:  Foo Bar                                        Date:  3/16/2031",
        ]);
        assert_eq!(status_line_room_name(&u, 2).as_deref(), Some("Foo Bar"));
    }

    #[test]
    fn status_room_name_empty_grid_is_none() {
        let u = upper_with(&["                                "]);
        assert_eq!(status_line_room_name(&u, 1), None);
    }

    #[test]
    fn status_field_values_split_labelled_fields() {
        // The Impossible Stairs: single spaces between the fields, so the common
        // form swallows the whole row and the fields are the only way through.
        let stairs = upper_with(&[" Year: 2001 Place: Front Lawn"]);
        assert_eq!(
            status_line_room_name(&stairs, 1).as_deref(),
            Some("Year: 2001 Place: Front Lawn"),
            "the common form takes the row whole — which is the bug the fields fix"
        );
        assert_eq!(status_line_field_values(&stairs, 1), vec!["2001", "Front Lawn"]);
    }

    #[test]
    fn status_field_values_handle_wide_gaps_and_a_score_block() {
        // Wide 2+-space gaps between label and value, as the `Location:` form uses.
        let labelled = upper_with(&[" Location:  Foo Bar     Date:  3/16/2031"]);
        assert_eq!(status_line_field_values(&labelled, 1), vec!["Foo Bar", "3/16/2031"]);
        // An ordinary room-plus-score line has no room in its fields, only numbers.
        // Harmless: `detect_location` reaches the fields only once the room name
        // itself has failed, and a score never names an object.
        let plain = upper_with(&[" Bedroom                    Score: 0     Moves: 1"]);
        assert_eq!(status_line_field_values(&plain, 1), vec!["0", "1"]);
    }

    #[test]
    fn status_field_values_none_without_a_label() {
        let plain = upper_with(&[" Kitchen                                         "]);
        assert!(status_line_field_values(&plain, 1).is_empty());
    }

    #[test]
    fn status_room_name_centered_defeats_common_form() {
        // A centered title has empty left-justified first segment → None from
        // the common form; the centered fallback recovers the name.
        let u = upper_with(&["                           Hilltop                                     "]);
        assert_eq!(status_line_room_name(&u, 1), None);
        assert_eq!(centered_status_line_room_name(&u, 1).as_deref(), Some("Hilltop"));
    }

    #[test]
    fn centered_fallback_ignores_left_justified() {
        // Left-justified lines must NOT trigger the centered fallback — the
        // common form already handles them, so it returns None here.
        let u = upper_with(&[" Bedroom                                         Score: 0     Moves: 1"]);
        assert_eq!(centered_status_line_room_name(&u, 1), None);
    }

    #[test]
    fn centered_fallback_multiword_and_empty() {
        let multi = upper_with(&["                     Palace Gate                                        "]);
        assert_eq!(centered_status_line_room_name(&multi, 1).as_deref(), Some("Palace Gate"));
        let empty = upper_with(&["                                                                        "]);
        assert_eq!(centered_status_line_room_name(&empty, 1), None);
    }

    #[test]
    fn bordered_centered_title_vt220() {
        // BeyondZork VT220 mode frames the centered room title with half-block
        // bars: `▐  <spaces>  Hilltop  <spaces>  ▌  <trailing spaces>`. The
        // leading bar must NOT be read as a left-justified room name, and the
        // centered fallback must recover the true name.
        let u = upper_with(&["▐                          Hilltop                           ▌                  "]);
        assert_eq!(status_line_room_name(&u, 1), None, "leading bar must not become a bogus room name");
        assert_eq!(centered_status_line_room_name(&u, 1).as_deref(), Some("Hilltop"));
    }

    #[test]
    fn bordered_centered_title_vt220_multiword() {
        let u = upper_with(&["▐                     Palace Gate                            ▌                  "]);
        assert_eq!(status_line_room_name(&u, 1), None);
        assert_eq!(centered_status_line_room_name(&u, 1).as_deref(), Some("Palace Gate"));
    }

    #[test]
    fn status_name_matches_rules() {
        assert!(status_name_matches("Bedroom", "Bedroom"));            // equal
        assert!(status_name_matches("Bedroom (messy)", "Bedroom"));    // trailing decoration
        assert!(status_name_matches("Bedroom, north end", "Bedroom")); // (post-strip safety net)
        assert!(!status_name_matches("Hallway", "Hall"));              // word-boundary guard
        assert!(status_name_matches("hall  ", "Hall"));                // case + whitespace
        assert!(!status_name_matches("Kitchen", "Bedroom"));          // unrelated
        assert!(!status_name_matches("Bedroom", ""));                 // empty short
    }

    #[test]
    fn find_player_object_by_name() {
        // Rename obj3 to "you" so it is the player.
        // "yourself" would be truncated to "yoursel" by v3 Z-char encoding (6 Z-char max),
        // so we use "you" which is in NAMES and encodes without truncation.
        let mut buf = build_v3_story();
        let name = z_name("you");
        buf[PROP3_TBL as usize] = (name.len() / 2) as u8;
        buf[PROP3_TBL as usize + 1..PROP3_TBL as usize + 1 + name.len()].copy_from_slice(&name);
        let machine = make_machine(buf);
        assert_eq!(find_player_object(&machine), Some(3));
    }

    #[test]
    fn find_player_object_finds_player_in_minizork() {
        // Real-game check: minizork's player object is #30, short name "you".
        // This is what makes the inventory panel's name-based player lookup reliable.
        //
        // Boots to the first read prompt (like every other real-game test in
        // this module) rather than asking on the raw, un-executed image.
        // Minizork's compiled object table starts BOTH "you" (#30) and a
        // second parse-word candidate, "brave adventurer" (#13, a rank-message
        // object whose words include "me"/"myself"/"self" — SQ-1259 widened
        // matching to parse words, which is what makes #13 a candidate at
        // all), unsituated at parent 0; only the init routine places #30 into
        // West of House. Asking before that ran would make #13 look like the
        // sole situated candidate and win by default — a fixture artifact
        // that would never happen once a game is actually playable.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minizork.z3");
        if !fixture.exists() {
            eprintln!("SKIP: minizork.z3 fixture not found");
            return;
        }
        let data = std::fs::read(&fixture).expect("read minizork.z3");
        let mut machine = make_machine(data);
        machine.init_caps();
        for _ in 0..100_000u64 {
            match machine.step() {
                StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart | StepResult::Fault => break,
                StepResult::NeedChar => { machine.supply_char(b'\n'); }
                StepResult::SaveRequest => { machine.complete_save(false); }
                StepResult::RestoreRequest => { machine.complete_restore_failure(); }
                StepResult::Continue => {}
            }
        }
        assert_eq!(find_player_object(&machine), Some(30), "minizork player object is #30 (\"you\")");
    }

    #[test]
    fn resolve_room_object_matches_short_name() {
        let machine = make_machine(build_v3_story()); // obj1 "west", obj2 "east", obj3 "hall"
        let r = resolve_room_object(&machine, "hall").expect("hall resolves");
        assert_eq!(r.number, 3);
        assert!(resolve_room_object(&machine, "nowhere").is_none());
    }

    /// Writes a v5 object's name-only property table at `ptbl`: length byte,
    /// the fixed-width encoded name, then the end-of-properties terminator.
    fn write_v5_name_only(buf: &mut [u8], ptbl: usize, text: &str) {
        let nm = crate::text::encode::encode_word(text, 5);
        buf[ptbl] = (nm.len() / 2) as u8;
        buf[ptbl + 1..ptbl + 1 + nm.len()].copy_from_slice(&nm);
        buf[ptbl + 1 + nm.len()] = 0x00;
    }

    // ── SQ-1259: resolve_room_object must not let a same-named compass
    // direction (or any other non-room object) outrank the room ─────────────

    #[test]
    fn resolve_room_object_prefers_the_room_the_global_names() {
        // Lost Pig shape: two objects share a short name ("outside") — here
        // BOTH are nested under some other object (parent 5, which need not
        // exist), so the parent-0 rule below could not disambiguate on its
        // own — and only the game's own `location` global (rule 1) picks the
        // right one. The global names the HIGHER-numbered object, so a pass
        // here proves rule 1 actually fired rather than the old "lowest
        // number wins" default coincidentally agreeing.
        let mut buf = sample_story(5);
        const TBL1: usize = 0x220;
        const TBL2: usize = 0x240;
        write_v5_name_only(&mut buf, TBL1, "outside");
        write_v5_name_only(&mut buf, TBL2, "outside");
        put_word(&mut buf, v5_entry(1) + 6, 5);
        put_word(&mut buf, v5_entry(1) + 12, TBL1 as u16);
        put_word(&mut buf, v5_entry(2) + 6, 5);
        put_word(&mut buf, v5_entry(2) + 12, TBL2 as u16);
        put_word(&mut buf, GLOBAL_VARS as usize, 2); // global 0 = object #2
        let machine = make_machine(buf);
        let r = resolve_room_object(&machine, "outside").expect("matches both objects");
        assert_eq!(r.number, 2, "the global-named object wins over the lower-numbered one");
    }

    #[test]
    fn resolve_room_object_prefers_a_top_level_room_when_no_global_names_it() {
        // Same two "outside" objects, but now #1 is nested (standing in for
        // Lost Pig's #18, a child of the compass) and #2 is top-level
        // (standing in for #93, the room), and NO global corroborates
        // either. Rule 2 (prefer parent 0) must pick #2 even though #1 is
        // numbered lower and the old rule (longest match, ties -> lowest
        // number) would have picked #1.
        let mut buf = sample_story(5);
        const TBL1: usize = 0x220;
        const TBL2: usize = 0x240;
        write_v5_name_only(&mut buf, TBL1, "outside");
        write_v5_name_only(&mut buf, TBL2, "outside");
        put_word(&mut buf, v5_entry(1) + 6, 5); // #1: nested (a direction)
        put_word(&mut buf, v5_entry(1) + 12, TBL1 as u16);
        put_word(&mut buf, v5_entry(2) + 6, 0); // #2: top-level (the room)
        put_word(&mut buf, v5_entry(2) + 12, TBL2 as u16);
        // global 0 left at 0 (untouched) — no corroboration.
        let machine = make_machine(buf);
        let r = resolve_room_object(&machine, "outside").expect("matches both objects");
        assert_eq!(
            r.number, 2,
            "the top-level object wins over the nested one, even though it is numbered higher"
        );
    }

    #[test]
    fn player_candidates_widen_to_parse_words_and_situated_wins() {
        // A Lost-Pig-shaped synthetic story: the avatar's printed name is
        // NOT in PLAYER_NAMES at all ("grunk", standing in for the real
        // Grunk) — only its PARSE WORDS include "me" — while an unrelated,
        // UNSITUATED "you" is a decoy that DOES match PLAYER_NAMES by short
        // name (SQ-1259). Exercises the exact path `find_player_object` runs
        // in production — including its own internal `ParseNames::detect` —
        // rather than handing it a pre-built reader, so the story must also
        // satisfy `detect`'s own heuristics: an Inform-shaped serial number,
        // and at least `MIN_AGREEING_OBJECTS` (4) objects whose property 1 is
        // a readable word array. Three filler objects (#4-6, matching neither
        // PLAYER_NAMES nor PLAYER_WORDS) supply the other three.
        let mut buf = sample_story(5);
        // An Inform-shaped serial number (`grammar::detect_format`): six
        // digits, YYMMDD-plausible, first digit not '8'. Lost Pig's own is
        // "080406"; reused here for the same reason.
        buf[0x12..0x18].copy_from_slice(b"080406");

        // ── Dictionary: "me" and a second word for the fillers, at the
        //    header's default 0x0200. ─────────────────────────────────────
        const DICT: usize = 0x200;
        buf[DICT] = 0; // no separators
        buf[DICT + 1] = 7; // entry_length: 6-byte key + 1 flag byte
        put_word(&mut buf, DICT + 2, 2); // 2 entries
        let me = crate::text::encode::encode_word("me", 5);
        let filler_word = crate::text::encode::encode_word("filler", 5);
        assert_eq!(me.len(), 6);
        assert_eq!(filler_word.len(), 6);
        buf[DICT + 4..DICT + 10].copy_from_slice(&me);
        buf[DICT + 11..DICT + 17].copy_from_slice(&filler_word);
        let me_addr = (DICT + 4) as u16;
        let filler_addr = (DICT + 11) as u16;

        // ── Object property tables ──────────────────────────────────────────
        const GRUNK_TBL: usize = 0x220;
        const YOU_TBL: usize = 0x240;
        const ROOM_TBL: usize = 0x260;
        const FILLER_TBL: [usize; 3] = [0x280, 0x2A0, 0x2C0];
        write_v5_name_only(&mut buf, GRUNK_TBL, "grunk");
        // Property 1 (Inform's `name` array): one word, the dictionary
        // address of "me". Short form header: bit6 set (2 bytes), number 1.
        let after_name = GRUNK_TBL + 1 + 6; // name occupies exactly 3 words (fixed width)
        buf[after_name] = 0x41;
        put_word(&mut buf, after_name + 1, me_addr);
        buf[after_name + 3] = 0x00; // terminator
        write_v5_name_only(&mut buf, YOU_TBL, "you");
        write_v5_name_only(&mut buf, ROOM_TBL, "room");
        for (i, &tbl) in FILLER_TBL.iter().enumerate() {
            write_v5_name_only(&mut buf, tbl, &format!("filler{i}"));
            let after = tbl + 1 + 6;
            buf[after] = 0x41; // property 1, one word: "filler"
            put_word(&mut buf, after + 1, filler_addr);
            buf[after + 3] = 0x00;
        }

        // ── Object entries ───────────────────────────────────────────────────
        put_word(&mut buf, v5_entry(1) + 6, 3); // #1 grunk: situated, in the room
        put_word(&mut buf, v5_entry(1) + 12, GRUNK_TBL as u16);
        put_word(&mut buf, v5_entry(2) + 6, 0); // #2 "you": parentless decoy
        put_word(&mut buf, v5_entry(2) + 12, YOU_TBL as u16);
        put_word(&mut buf, v5_entry(3) + 6, 0); // #3 room: top-level
        put_word(&mut buf, v5_entry(3) + 10, 1); // child #1
        put_word(&mut buf, v5_entry(3) + 12, ROOM_TBL as u16);
        for (i, &tbl) in FILLER_TBL.iter().enumerate() {
            let obj = 4 + i as u16;
            put_word(&mut buf, v5_entry(obj) + 12, tbl as u16);
        }

        let machine = make_machine(buf);
        assert_eq!(normalize_name(&short_name(&machine.mem, 1)), "grunk");
        assert_eq!(normalize_name(&short_name(&machine.mem, 2)), "you");
        assert_eq!(max_object_number(&machine.mem), 6, "grunk, you, room, and 3 fillers");

        let parse_names =
            ParseNames::detect(&machine.mem).expect("Inform-shaped, 4 objects agree on property 1");
        assert_eq!(parse_names.property(), 1);
        let cands = player_candidates(&machine, Some(&parse_names));
        assert_eq!(
            cands,
            vec![1, 2],
            "grunk matches via the parse word \"me\"; \"you\" matches via its short name; \
             the fillers (property 1 = \"filler\") match neither"
        );

        assert_eq!(
            find_player_object(&machine),
            Some(1),
            "the situated avatar — found only through its parse word — beats the parentless decoy"
        );
    }

    #[test]
    fn player_room_beside_ignores_a_parse_word_only_candidate_with_no_name() {
        // Nord and Bert Couldn't Make Head or Tail of It (a game built entirely
        // out of wordplay) is what this reproduces (SQ-1259). Its status line
        // names a real hub room, "Beginning" (#1 here), but widening
        // `player_candidates` to parse words turned up an unrelated object
        // with an EMPTY short name (#4 here — some `x myself` joke, matched
        // only by the parse word "myself"), parented under a SIBLING object
        // (#3, "Jean Stock" there) that happens to share the room's own
        // parent (#5, "it", the globals pseudo-container).
        //
        // `player_room_beside`'s heuristic has no name check at all — it
        // trusts "shares the shown room's parent" — so fed the WIDENED pool
        // it walks up from the joke object to #3 and reports THAT as the
        // room, even though #3's own name ("stock") has nothing to do with
        // "Beginning". Restricting `player_room_beside` to the short-name-only
        // candidate pool (which excludes the joke object, since its short
        // name is empty) is what keeps this from firing, and the room falls
        // through correctly to StatusName(#1) "Beginning" instead.
        let mut buf = sample_story(5);
        buf[0x12..0x18].copy_from_slice(b"080406"); // Inform-shaped serial

        const DICT: usize = 0x200;
        buf[DICT] = 0;
        buf[DICT + 1] = 7;
        put_word(&mut buf, DICT + 2, 2);
        let myself = crate::text::encode::encode_word("myself", 5);
        let filler_word = crate::text::encode::encode_word("filler", 5);
        buf[DICT + 4..DICT + 10].copy_from_slice(&myself);
        buf[DICT + 11..DICT + 17].copy_from_slice(&filler_word);
        let myself_addr = (DICT + 4) as u16;
        let filler_addr = (DICT + 11) as u16;

        const ROOM_TBL: usize = 0x220; // #1 "Beginning"
        const YOURSELF_TBL: usize = 0x240; // #2 "yourself"
        const SIBLING_TBL: usize = 0x260; // #3 "stock" (the wrong sibling)
        const JOKE_TBL: usize = 0x280; // #4 "" + property 1 = "myself"
        const HUB_TBL: usize = 0x2A0; // #5 "it"
        const FILLER_TBL: [usize; 3] = [0x2C0, 0x2E0, 0x300];

        write_v5_name_only(&mut buf, ROOM_TBL, "beginning");
        write_v5_name_only(&mut buf, YOURSELF_TBL, "yourself");
        write_v5_name_only(&mut buf, SIBLING_TBL, "stock");
        write_v5_name_only(&mut buf, JOKE_TBL, "");
        let after_joke_name = JOKE_TBL + 1 + 6;
        buf[after_joke_name] = 0x41;
        put_word(&mut buf, after_joke_name + 1, myself_addr);
        buf[after_joke_name + 3] = 0x00;
        write_v5_name_only(&mut buf, HUB_TBL, "it");
        for (i, &tbl) in FILLER_TBL.iter().enumerate() {
            write_v5_name_only(&mut buf, tbl, &format!("filler{i}"));
            let after = tbl + 1 + 6;
            buf[after] = 0x41;
            put_word(&mut buf, after + 1, filler_addr);
            buf[after + 3] = 0x00;
        }

        put_word(&mut buf, v5_entry(1) + 6, 5); // #1 room: child of the hub
        put_word(&mut buf, v5_entry(1) + 12, ROOM_TBL as u16);
        put_word(&mut buf, v5_entry(2) + 6, 0); // #2 "yourself": parentless
        put_word(&mut buf, v5_entry(2) + 12, YOURSELF_TBL as u16);
        put_word(&mut buf, v5_entry(3) + 6, 5); // #3 "stock": ALSO a child of the hub
        put_word(&mut buf, v5_entry(3) + 12, SIBLING_TBL as u16);
        put_word(&mut buf, v5_entry(4) + 6, 3); // #4 joke: child of #3
        put_word(&mut buf, v5_entry(4) + 12, JOKE_TBL as u16);
        put_word(&mut buf, v5_entry(5) + 6, 0); // #5 hub: top-level
        put_word(&mut buf, v5_entry(5) + 12, HUB_TBL as u16);
        for (i, &tbl) in FILLER_TBL.iter().enumerate() {
            let obj = 6 + i as u16;
            put_word(&mut buf, v5_entry(obj) + 12, tbl as u16);
        }

        let mut m = make_machine(buf);
        m.screen.upper = upper_with(&[" Beginning                                        Score: 0    Moves: 0"]);
        m.screen.upper_window_rows = 1;

        assert_eq!(normalize_name(&short_name(&m.mem, 4)), "");
        let parse_names = ParseNames::detect(&m.mem).expect("Inform-shaped, 4 objects agree on property 1");
        let cands = player_candidates(&m, Some(&parse_names));
        assert_eq!(cands, vec![2, 4], "\"yourself\" by name; the joke object by its parse word alone");
        assert_eq!(
            player_candidates_by_name(&m),
            vec![2],
            "the joke object (no printed name) never enters the narrow pool"
        );

        let loc = detect_location(&m).expect("the status line names a real room");
        assert_eq!(loc.method(), LocationMethod::StatusName);
        assert_eq!(
            loc.object().unwrap().number,
            1,
            "must resolve to the room \"Beginning\" (#1), not the unrelated sibling \"stock\" (#3) \
             that player_room_beside's un-narrowed heuristic would have picked"
        );
    }

    #[test]
    fn nearest_matching_ancestor_walks_up() {
        // obj tree: obj3 (parent obj1), obj2 (parent obj1), obj1 (parent 0).
        // Searching from obj3 for "west" should walk up to obj1.
        let machine = make_machine(build_v3_story());
        let r = nearest_matching_ancestor(&machine, 3, "west").expect("walks up to west");
        assert_eq!(r.number, 1);
        assert!(nearest_matching_ancestor(&machine, 3, "nowhere").is_none());
    }

    #[test]
    fn detect_location_v3_uses_global0() {
        let mut buf = build_v3_story();
        put_word(&mut buf, GLOBAL_VARS as usize, 1); // global 0 = obj 1
        let machine = make_machine(buf);
        match detect_location(&machine) {
            Some(Location::GlobalVar0(s)) => assert_eq!(s.number, 1),
            other => panic!("expected GlobalVar0, got {other:?}"),
        }
        assert_eq!(detect_location(&machine).unwrap().method(), LocationMethod::GlobalVar0);
    }

    #[test]
    fn location_object_and_method_accessors() {
        let s = ObjectSnapshot { number: 5, parent: 0, name: "Hall".into() };
        assert_eq!(Location::StatusName(s.clone()).object().map(|o| o.number), Some(5));
        assert_eq!(Location::NameOnly("X".into()).object(), None);
        assert_eq!(Location::NameOnly("X".into()).method(), LocationMethod::NameOnly);
        assert_eq!(Location::PlayerParent(s).method(), LocationMethod::PlayerParent);
    }

    // We reuse the same object-table layout as objects.rs tests:
    //   object_table = 0x0100, entries_base = 0x013E (v3)
    //   obj1 at 0x013E, obj2 at 0x0147, obj3 at 0x0150
    //   property tables at 0x0200, 0x0220, 0x0230
    //   global_vars = 0x0300

    const OBJ_TABLE: u32 = 0x0100;
    const ENTRIES_V3: u32 = OBJ_TABLE + 31 * 2; // 0x013E
    const OBJ1_ENTRY: u32 = ENTRIES_V3;
    const OBJ2_ENTRY: u32 = ENTRIES_V3 + 9;
    const OBJ3_ENTRY: u32 = ENTRIES_V3 + 18;

    const PROP1_TBL: u16 = 0x0200;
    const PROP2_TBL: u16 = 0x0220;
    const PROP3_TBL: u16 = 0x0230;

    const GLOBAL_VARS: u32 = 0x0300;

    fn put_word(buf: &mut [u8], offset: usize, val: u16) {
        buf[offset]     = (val >> 8) as u8;
        buf[offset + 1] = (val & 0xFF) as u8;
    }

    /// Write `val` as a big-endian word into the buffer.
    fn z_name(text: &str) -> Vec<u8> {
        crate::text::encode::encode_word(text, 3) // 4 bytes = 2 Z-words for v3
    }

    /// Build a minimal v3 story with 3 objects and properly encoded short names.
    /// Object structure: obj1 (root), obj2 (child of obj1), obj3 (sibling of obj2).
    fn build_v3_story() -> Vec<u8> {
        let mut buf = sample_story(3);

        // ── obj1 entry: parent=0 sibling=0 child=2, prop=PROP1_TBL ──────────
        buf[OBJ1_ENTRY as usize + 4] = 0; // parent
        buf[OBJ1_ENTRY as usize + 5] = 0; // sibling
        buf[OBJ1_ENTRY as usize + 6] = 2; // child
        put_word(&mut buf, OBJ1_ENTRY as usize + 7, PROP1_TBL);

        // ── obj2 entry: parent=1 sibling=3 child=0 ───────────────────────────
        buf[OBJ2_ENTRY as usize + 4] = 1;
        buf[OBJ2_ENTRY as usize + 5] = 3;
        buf[OBJ2_ENTRY as usize + 6] = 0;
        put_word(&mut buf, OBJ2_ENTRY as usize + 7, PROP2_TBL);

        // ── obj3 entry: parent=1 sibling=0 child=0 ───────────────────────────
        buf[OBJ3_ENTRY as usize + 4] = 1;
        buf[OBJ3_ENTRY as usize + 5] = 0;
        buf[OBJ3_ENTRY as usize + 6] = 0;
        put_word(&mut buf, OBJ3_ENTRY as usize + 7, PROP3_TBL);

        // ── Property table for obj1: name "west", no properties ──────────────
        // name_words = 2 (encode_word produces 4 bytes = 2 Z-words for v3)
        let name1 = z_name("west");
        assert_eq!(name1.len(), 4);
        buf[PROP1_TBL as usize] = 2; // 2 Z-words in name
        buf[PROP1_TBL as usize + 1..PROP1_TBL as usize + 5].copy_from_slice(&name1);
        buf[PROP1_TBL as usize + 5] = 0x00; // end-of-properties sentinel

        // ── Property table for obj2: name "east" ─────────────────────────────
        let name2 = z_name("east");
        buf[PROP2_TBL as usize] = 2;
        buf[PROP2_TBL as usize + 1..PROP2_TBL as usize + 5].copy_from_slice(&name2);
        buf[PROP2_TBL as usize + 5] = 0x00;

        // ── Property table for obj3: name "hall" ─────────────────────────────
        let name3 = z_name("hall");
        buf[PROP3_TBL as usize] = 2;
        buf[PROP3_TBL as usize + 1..PROP3_TBL as usize + 5].copy_from_slice(&name3);
        buf[PROP3_TBL as usize + 5] = 0x00;

        buf
    }

    /// Build a Machine from story bytes.
    fn make_machine(buf: Vec<u8>) -> Machine {
        Machine::new(Memory::new(buf).unwrap())
    }

    // ── v5 avatar-based detection ────────────────────────────────────────────
    // Regression for the Zork1-r52 forest collapse: same-named rooms must be
    // distinguished by the true avatar's parent, not by a name-only match that
    // resolves every "forest" to one scenery object.

    const V5_ENTRIES: u32 = OBJ_TABLE + 63 * 2; // prop-defaults = 63 words (v4+)
    fn v5_entry(obj: u16) -> usize {
        (V5_ENTRIES + (obj as u32 - 1) * 14) as usize
    }

    /// Build a v5 story with 6 objects reproducing the r52 topology:
    ///   #1 "forest" scenery (top-level)   — what StatusName wrongly resolves to
    ///   #2 "you"    the parser's player NOUN, parent 0 by default
    ///   #3 "forest" a real forest ROOM
    ///   #4 "cretin" the true avatar, child of #3
    ///   #5 "forest" a second real forest ROOM
    ///   #6 "it"     the globals pseudo-container (top-level, holds no rooms)
    /// Lowercase names avoid A1/A2 shift-encoding; the status line matches case-
    /// insensitively.
    ///
    /// Six is the ceiling: entry 7 would start where the first property table
    /// does, and `max_object_number` (rightly) stops there.
    fn build_v5_forests() -> Vec<u8> {
        let mut buf = sample_story(5);
        let props: [(u16, &str); 6] = [
            (0x1D2, "forest"),
            (0x1DA, "you"),
            (0x1E2, "forest"),
            (0x1EA, "cretin"),
            (0x1F2, "forest"),
            (0x280, "it"),
        ];
        let parents: [u16; 6] = [0, 0, 0, 3, 0, 0];
        for (i, (ptbl, name)) in props.iter().enumerate() {
            let obj = (i + 1) as u16;
            let e = v5_entry(obj);
            put_word(&mut buf, e + 6, parents[i]); // parent (word, v4+)
            put_word(&mut buf, e + 8, 0); // sibling
            put_word(&mut buf, e + 10, 0); // child
            put_word(&mut buf, e + 12, *ptbl); // property-table pointer
            let nm = crate::text::encode::encode_word(name, 5);
            let p = *ptbl as usize;
            buf[p] = (nm.len() / 2) as u8; // name length in words
            buf[p + 1..p + 1 + nm.len()].copy_from_slice(&nm);
            buf[p + 1 + nm.len()] = 0x00; // end-of-properties sentinel
        }
        buf
    }

    /// v5 machine with the avatar (#4 cretin) placed in `cretin_parent` and a
    /// status line showing "forest".
    fn machine_in_forest(cretin_parent: u16) -> Machine {
        let mut buf = build_v5_forests();
        put_word(&mut buf, v5_entry(4) + 6, cretin_parent);
        let mut m = make_machine(buf);
        m.screen.upper = upper_with(&[" forest                              Score: 0    Moves: 0"]);
        m.screen.upper_window_rows = 1;
        m
    }

    /// The real Zork1-r52 topology (SQ-0668): "you" is NOT parentless — it is
    /// parked in the "it" globals pseudo-container (#6), so it is just as
    /// "situated" as the avatar. Only "cretin" (#4) is inside the room.
    fn machine_zork1_r52() -> Machine {
        let mut buf = build_v5_forests();
        put_word(&mut buf, v5_entry(2) + 6, 6); // "you" lives in the globals object
        put_word(&mut buf, v5_entry(4) + 6, 3); // "cretin" lives in forest ROOM #3
        let mut m = make_machine(buf);
        m.screen.upper = upper_with(&[" forest                              Score: 0    Moves: 0"]);
        m.screen.upper_window_rows = 1;
        m
    }

    #[test]
    fn v5_builder_encodes_names() {
        // Guard: distinguish a builder bug from a detection bug.
        let m = machine_in_forest(3);
        assert_eq!(normalize_name(&short_name(&m.mem, 3)), "forest");
        assert_eq!(normalize_name(&short_name(&m.mem, 4)), "cretin");
        assert_eq!(status_line_room_name(&m.screen.upper, m.screen.upper_window_rows).as_deref(), Some("forest"));
    }

    #[test]
    fn v5_same_named_rooms_resolve_via_avatar_not_name() {
        // Avatar #4 is in forest ROOM #3; scenery "forest" is #1. Name-only
        // resolution would return #1; the avatar's parent gives the true #3.
        let m = machine_in_forest(3);
        let loc = detect_location(&m).expect("should detect a location");
        assert_eq!(loc.method(), LocationMethod::PlayerParent, "must validate the avatar, not match by name");
        assert_eq!(loc.object().unwrap().number, 3, "true room is #3, not scenery #1");
    }

    #[test]
    fn find_player_object_prefers_situated_avatar_over_decorative_you() {
        // Zork1-r52 topology: #2 "you" is a decorative object with no parent,
        // #4 "cretin" is the real avatar situated inside forest ROOM #3.
        // Old name-only lookup grabbed #2 (empty child chain -> inventory always
        // empty); the situated-candidate heuristic must select #4 instead.
        let m = machine_in_forest(3);
        assert_eq!(get_parent(&m.mem, 2), 0, "decorative \"you\" #2 is parentless");
        assert_eq!(get_parent(&m.mem, 4), 3, "avatar \"cretin\" #4 sits in room #3");
        assert_eq!(
            find_player_object(&m),
            Some(4),
            "must pick the situated avatar #4 (cretin), not the decorative #2 (you)"
        );
    }

    /// SQ-0668. The regression SQ-0212 *thought* it was fixing: in the real
    /// Zork1 r52 story file the decorative "you" is not parentless at all — it
    /// sits in the "it" globals container, which makes "prefer the lowest
    /// situated candidate" pick it and report an empty inventory forever. The
    /// avatar is the candidate that is actually inside the detected ROOM.
    #[test]
    fn find_player_object_rejects_a_you_parked_in_the_globals_container() {
        let m = machine_zork1_r52();
        assert_eq!(get_parent(&m.mem, 2), 6, "\"you\" #2 is situated — inside globals #6");
        assert_eq!(get_parent(&m.mem, 6), 0, "…and #6 \"it\" is a top-level pseudo-container");
        assert_eq!(get_parent(&m.mem, 4), 3, "avatar \"cretin\" #4 sits in room #3");
        assert_eq!(
            detect_location(&m).and_then(|l| l.object().map(|o| o.number)),
            Some(3),
            "the room is #3 — the same discrimination find_player_object must reuse"
        );
        assert_eq!(
            find_player_object(&m),
            Some(4),
            "must pick the avatar in the room (#4 cretin), not the situated-but-global #2 (you)"
        );
    }

    /// An avatar riding a vehicle is still in the room: validation walks the
    /// whole ancestor chain, not just the parent.
    #[test]
    fn find_player_object_accepts_an_avatar_inside_a_vehicle_in_the_room() {
        let mut buf = build_v5_forests();
        put_word(&mut buf, v5_entry(2) + 6, 6); // "you" parked in globals
        put_word(&mut buf, v5_entry(4) + 6, 5); // "cretin" inside object #5…
        put_word(&mut buf, v5_entry(5) + 6, 3); // …which is itself inside room #3
        let mut m = make_machine(buf);
        m.screen.upper = upper_with(&[" forest                              Score: 0    Moves: 0"]);
        m.screen.upper_window_rows = 1;
        assert_eq!(find_player_object(&m), Some(4), "the chain reaches the room, so #4 still wins");
    }

    #[test]
    fn find_player_object_refuses_to_guess_when_none_situated_and_room_unresolved() {
        // No candidate is situated (avatar #4 also parentless) and neither
        // reaches the room the status line resolves to (scenery "forest" #1,
        // which nothing here is parented under either) — so, mirroring
        // `gvm::objects::ParseNames::find_player`'s rule exactly (SQ-1259),
        // there is no "first plausible candidate" fallback left to fall back
        // to. A guessed avatar is worse than none: its children become an
        // inventory the player is told they carry. This used to return the
        // lowest-numbered candidate (#2 "you"); it must now return None.
        let m = machine_in_forest(0);
        assert_eq!(get_parent(&m.mem, 2), 0);
        assert_eq!(get_parent(&m.mem, 4), 0);
        assert_eq!(
            find_player_object(&m),
            None,
            "no situated candidate and no ancestor chain reaches the room: refuse rather than guess"
        );
    }

    #[test]
    fn v5_distinct_forests_get_distinct_ids() {
        let a = detect_location(&machine_in_forest(3)).unwrap();
        let b = detect_location(&machine_in_forest(5)).unwrap();
        assert_eq!(a.object().unwrap().number, 3);
        assert_eq!(b.object().unwrap().number, 5);
        assert_ne!(
            a.object().unwrap().number,
            b.object().unwrap().number,
            "two different forest rooms must not collapse to one id"
        );
    }

    // ── v6 (graphical) status-band detection (SQ-0468) ───────────────────────
    // The v6 paint model (`screen.v6`) replaces the v4+ grid as the status-text
    // source. Bands below reproduce the real Zork0/Shogun layouts verified
    // headlessly (see the app-level v6_location_mapper gameplay tests).

    use crate::screen::{V6Text, V6Windows, ZColour};

    /// A v6 status band in window 1 (y_coord=1) painted with the given
    /// `(y, x, text)` runs at their absolute screen pixel positions, with the
    /// story window (0) opening at Zork Zero's real y=79 so the band above it is
    /// the top five native rows.
    fn v6_band(runs: &[(u16, u16, &str)]) -> V6Windows {
        v6_band_below(79, runs)
    }

    /// As `v6_band`, but with the story window (0) opening at `story_top` —
    /// Arthur hangs its bar just above a story window at y=209.
    fn v6_band_below(story_top: u16, runs: &[(u16, u16, &str)]) -> V6Windows {
        let mut v = V6Windows::default();
        v.windows[0].y_coord = story_top;
        v.windows[1].y_coord = 1;
        v.windows[1].x_coord = 1;
        for &(y, x, text) in runs {
            v.windows[1].texts.push(V6Text::derived(
                y,
                x,
                text.into(),
                0,
                ZColour::Default,
                ZColour::Default,
                crate::screen::V6Cell::DEFAULT,
            ));
        }
        v
    }

    /// As [`v6_band`], but every run also carries the COLUMN the engine's grid pen
    /// was standing on — which on a proportional machine is NOT `x / cell.w`.
    fn v6_band_penned(runs: &[(u16, u16, u16, &str)]) -> V6Windows {
        let mut v = V6Windows::default();
        v.windows[0].y_coord = 79;
        v.windows[1].y_coord = 1;
        v.windows[1].x_coord = 1;
        for &(y, x, gcol, text) in runs {
            v.windows[1].texts.push(V6Text {
                y,
                x,
                text: text.into(),
                style: 0,
                fg: ZColour::Default,
                bg: ZColour::Default,
                grow: 0,
                gcol,
            });
        }
        v
    }

    /// SQ-1048 — the band is laid out on the pen's columns, not on `x / cell.w`.
    ///
    /// Transcribed from `stories/Arthur - The Quest for Excalibur.adf` (release 54 /
    /// serial 890606, the Amiga press), fifteen blank turns and a `look`: the
    /// churchyard bar, emitted one run per glyph as Arthur always emits it. The pen
    /// advances by the disk face's own widths — 12, 10, 10, 10, 10, 10, 12, 10, 10 —
    /// so the pixel quotient climbs about 1.3 columns per glyph and lands the name on
    /// 4, 6, 7, 8, 10, 11, 12, 14, 15, 16. The gaps that opens are what the ordinary
    /// "two or more spaces end a field" rule then cuts on, and `Churchyard` reaches
    /// the object tree as `C hur chy ard` — a name nothing in the story answers to,
    /// which cost Arthur room detection outright while the F3 and F5 overlays went on
    /// offering `red piece of glass` and `You are standing…` in its place.
    ///
    /// The date is here so the fix cannot be "merge everything": those columns really
    /// are far enough out to be a second field, and must stay one.
    #[test]
    fn v6_candidates_lay_the_band_on_the_engines_pen_not_the_pixel_quotient() {
        let v = v6_band_penned(&[
            (1, 35, 4, "C"),
            (1, 47, 5, "h"),
            (1, 57, 6, "u"),
            (1, 67, 7, "r"),
            (1, 77, 8, "c"),
            (1, 87, 9, "h"),
            (1, 97, 10, "y"),
            (1, 109, 11, "a"),
            (1, 119, 12, "r"),
            (1, 129, 13, "d"),
            (1, 390, 48, "S"),
            (1, 402, 49, "t"),
            (1, 416, 51, "A"),
            (1, 428, 52, "n"),
            (1, 438, 53, "n"),
            (1, 448, 54, "e"),
            (1, 458, 55, "'"),
            (1, 464, 56, "s"),
            (1, 480, 58, "D"),
            (1, 492, 59, "a"),
            (1, 502, 60, "y"),
        ]);
        let mut m = make_machine(build_v5_forests());
        m.screen.v6 = Some(v);
        assert_eq!(
            v6_status_room_candidates(&m),
            vec!["Churchyard".to_string(), "St Anne's Day".to_string()],
        );
    }

    #[test]
    fn v6_candidates_shogun_layout_filters_label_banner_and_stats() {
        // Row 1: "Erasmus:" (label — ":" abuts) + "SHOGUN" (centered banner) +
        // Score. Row 2: "Bridge" (room, left) + Moves. Expect only the real
        // room-ish runs, left-anchored first: Bridge, then the centered SHOGUN.
        // 640×400 coords (SQ-0479 re-trace, window x_coord=1): Erasmus label
        // dx=2, ":" abuts, SHOGUN banner dx=249, Score dx=457; row 2 (y=17):
        // Bridge dx=2, Moves dx=457.
        let v = v6_band(&[
            (1, 3, "Erasmus"),
            (1, 59, ":"),
            (1, 250, "SHOGUN"),
            (1, 458, "Score:"),
            (1, 538, "0"),
            (17, 3, "Bridge"),
            (17, 458, "Moves:"),
            (17, 538, "1"),
        ]);
        let mut m = make_machine(build_v5_forests());
        m.screen.v6 = Some(v);
        assert_eq!(v6_status_room_candidates(&m), vec!["Bridge".to_string(), "SHOGUN".to_string()]);
    }

    #[test]
    fn v6_candidates_zork0_layout_room_left_kingdom_other() {
        // Zork0: room "Banquet Hall" left on the top row; kingdom "Flatheadia"
        // right-anchored; Score/Moves filtered. Left-anchored room comes first.
        // 640×400 coords (SQ-0479 re-trace): room y=11 x=71 (dx=70, left);
        // Flatheadia x=489 (dx=488, right); Score/Moves on the y=27 row.
        let v = v6_band(&[
            (11, 71, "Banquet Hall"),
            (11, 489, "Flatheadia"),
            (27, 71, "Moves:"),
            (27, 489, "Score:"),
        ]);
        let mut m = make_machine(build_v5_forests());
        m.screen.v6 = Some(v);
        assert_eq!(
            v6_status_room_candidates(&m),
            vec!["Banquet Hall".to_string(), "Flatheadia".to_string()]
        );
    }

    #[test]
    fn v6_candidates_below_band_and_blanks_ignored() {
        // Runs at or below the story window's top edge (Journey's menu at y=305)
        // and blank padding never become candidates → menu screens yield nothing.
        // With the story opening at y=79, a run at y=65 straddles the boundary
        // (65+16 = 81 > 79), so it is prose, not status furniture.
        let v = v6_band(&[(9, 121, " "), (305, 75, "START the game"), (65, 26, "TooLow")]);
        let mut m = make_machine(build_v5_forests());
        m.screen.v6 = Some(v);
        assert!(v6_status_room_candidates(&m).is_empty());
    }

    // ── SQ-0530: the band is derived from the story window, not the screen top ──

    #[test]
    fn v6_candidates_arthur_deep_band_single_glyph_runs() {
        // Arthur's real layout: a one-row status window hung at y=193, directly
        // above a story window that opens at y=209 — twelve native rows BELOW the
        // top of the screen, where a fixed "first N rows" band never looked. The
        // row arrives as SINGLE-GLYPH runs (one per 8px cell), so the location
        // must be reassembled from its letters rather than read run by run.
        let mut runs: Vec<(u16, u16, String)> = Vec::new();
        for (i, ch) in " Churchyard                                     ".chars().enumerate() {
            runs.push((193, 29 + 8 * i as u16, ch.to_string()));
        }
        for (i, ch) in "St Anne's Day, Compline ".chars().enumerate() {
            runs.push((193, 420 + 8 * i as u16, ch.to_string()));
        }
        let borrowed: Vec<(u16, u16, &str)> = runs.iter().map(|(y, x, s)| (*y, *x, s.as_str())).collect();
        let mut v = v6_band_below(209, &borrowed);
        v.windows[1].y_coord = 193;
        v.windows[1].x_coord = 29;
        let mut m = make_machine(build_v5_forests());
        m.screen.v6 = Some(v);
        // The location is one word (not ten letters), the date is a separate
        // field, and the location comes first because it is left-anchored.
        assert_eq!(
            v6_status_room_candidates(&m),
            vec!["Churchyard".to_string(), "St Anne's Day".to_string()]
        );
    }

    #[test]
    fn v6_band_floor_follows_the_story_window() {
        // The same painted row is status furniture when the story opens BELOW it
        // and prose when the story opens ABOVE it — nothing about the row itself
        // changes, only where the story window starts.
        let runs = [(193u16, 37u16, "Churchyard")];
        let mut m = make_machine(build_v5_forests());

        m.screen.v6 = Some(v6_band_below(209, &runs));
        assert_eq!(v6_status_room_candidates(&m), vec!["Churchyard".to_string()]);

        m.screen.v6 = Some(v6_band_below(1, &runs));
        assert!(v6_status_room_candidates(&m).is_empty(), "text inside the story window is prose, not status");
    }

    #[test]
    fn v6_row_segments_split_fields_but_keep_multiword_names() {
        // Two or more spaces separate FIELDS; a single space stays inside a name.
        let v = v6_band(&[(11, 1, "Palace Gate"), (11, 401, "Score: 0")]);
        let mut m = make_machine(build_v5_forests());
        m.screen.v6 = Some(v);
        assert_eq!(v6_status_room_candidates(&m), vec!["Palace Gate".to_string()]);
    }

    #[test]
    fn v6_detect_playerparent_validates_left_room() {
        // build_v5_forests: avatar #4 "cretin" sits in forest ROOM #3. A
        // left-anchored "forest" run must validate via the avatar's ancestor
        // chain → PlayerParent(#3), NOT the scenery "forest" #1.
        let mut m = machine_in_forest(3);
        m.screen.v6 = Some(v6_band(&[(11, 71, "forest"), (27, 71, "Score: 0")]));
        let loc = detect_location(&m).expect("v6 left room should detect");
        assert_eq!(loc.method(), LocationMethod::PlayerParent);
        assert_eq!(loc.object().unwrap().number, 3, "true room #3, not scenery #1");
    }

    #[test]
    fn v6_detect_statusname_when_avatar_not_parented_to_room() {
        // Shogun case: the avatar is NOT in the named room's subtree, so
        // PlayerParent can't fire. A LEFT-anchored name that resolves to a real
        // object falls back to StatusName. Here cretin #4 is parentless (0) so no
        // ancestor validates; "forest" resolves to the scenery object #1.
        let mut m = machine_in_forest(0);
        m.screen.v6 = Some(v6_band(&[(17, 71, "forest"), (17, 489, "Moves: 1")]));
        let loc = detect_location(&m).expect("left-anchored name resolves to an object");
        assert_eq!(loc.method(), LocationMethod::StatusName);
        assert_eq!(loc.object().unwrap().number, 1, "\"forest\" resolves to scenery #1");
    }

    #[test]
    fn v6_detect_rejects_centered_banner_without_playerparent() {
        // A CENTERED run that matches no avatar ancestor is never StatusName'd —
        // banners like "SHOGUN" must not become rooms. Even though "forest" is a
        // REAL object (#1), a centered placement with a parentless avatar yields
        // None (StatusName is left-anchored-only).
        let mut m = machine_in_forest(0);
        m.screen.v6 = Some(v6_band(&[(11, 250, "forest"), (11, 489, "Score: 0")]));
        assert_eq!(detect_location(&m), None, "centered object-name must not yield StatusName");
    }

    #[test]
    fn v6_detect_drops_nameonly_for_unknown_left_name() {
        // A left-anchored name with NO backing object and not an ancestor yields
        // None for v6 (NameOnly is dropped) — a title/character-sheet name must
        // never invent a room.
        let mut m = machine_in_forest(0);
        m.screen.v6 = Some(v6_band(&[(11, 71, "Nowhere City"), (11, 489, "Score: 0")]));
        assert_eq!(detect_location(&m), None, "unknown v6 name must not become a NameOnly room");
    }

    #[test]
    fn v6_detect_none_on_blank_menu_band() {
        // Menu/title phase: only blank/padding runs in the band → None.
        let mut m = machine_in_forest(3);
        m.screen.v6 = Some(v6_band(&[(1, 121, " "), (9, 121, " ")]));
        assert_eq!(detect_location(&m), None, "no room text → no location (title/menu is None)");
    }

    // ── v6 rung 3: room text in a PROPERTY, room pointer in a GLOBAL (SQ-0724) ──
    // The Mysterious Adventures shape: no avatar in the tree, every room object
    // sharing one useless short name, and the text the player reads held in a
    // property. Built synthetically here; driven against all eleven real titles
    // by `crates/app/tests/mysterious_room_detection.rs`.

    /// Encode a lowercase/space string as a Z-string (alphabet A0 only), padded
    /// to a whole number of words with the standard z-char 5 and terminated.
    fn zstring_bytes(text: &str) -> Vec<u8> {
        let mut z: Vec<u8> =
            text.chars().map(|c| if c == ' ' { 0 } else { c as u8 - b'a' + 6 }).collect();
        while !z.len().is_multiple_of(3) {
            z.push(5);
        }
        let words = z.len() / 3;
        let mut out = Vec::new();
        for i in 0..words {
            let mut w = ((z[i * 3] as u16) << 10) | ((z[i * 3 + 1] as u16) << 5) | z[i * 3 + 2] as u16;
            if i == words - 1 {
                w |= 0x8000;
            }
            out.extend_from_slice(&w.to_be_bytes());
        }
        out
    }

    /// A machine shaped like a Mysterious Adventures port: the avatar is
    /// parentless, object #6's short name ("it") is nothing like the room, its
    /// real text sits in property 10, and global `g` points at it.
    ///
    /// `room_obj` is what the global is set to — pointing it somewhere else is how
    /// the negative test shows the text has to corroborate the global.
    fn machine_scott_shaped(g: u32, room_obj: u16, text: &str) -> Machine {
        const ROOM_TEXT_ADDR: usize = 0x600; // packed (v5, ×4) = 0x180
        const DECOY_ADDR: usize = 0x610; //     packed (v5, ×4) = 0x184
        let mut buf = build_v5_forests();
        put_word(&mut buf, v5_entry(4) + 6, 0); // avatar parentless: no PlayerParent
        buf.resize(0x800, 0);
        let enc = zstring_bytes(text);
        buf[ROOM_TEXT_ADDR..ROOM_TEXT_ADDR + enc.len()].copy_from_slice(&enc);

        // A DECOY: a property word that IS a well-formed, terminated Z-string, but
        // whose first z-char is an abbreviation (1) whose table entry points far
        // past the end of the story. Decoding it walks out of bounds — the exact
        // shape `without_fault_latch` exists for, and unreachable by any bounds
        // check on the property word itself.
        put_word(&mut buf, DECOY_ADDR, 0x8000 | (1 << 10) | (5 << 5) | 5);
        put_word(&mut buf, 0x40 + 2 * 5, 0x7000); // abbrev #5 → byte address 0xE000

        // Append property 10 (one-byte header, bit 6 = 2 data bytes) over an
        // object's end-of-properties sentinel, holding a PACKED string address.
        let add_text_prop = |buf: &mut Vec<u8>, ptbl: usize, packed: u16| {
            let after_name = ptbl + 1 + buf[ptbl] as usize * 2;
            buf[after_name] = 0x40 | 10;
            put_word(buf, after_name + 1, packed);
            buf[after_name + 3] = 0x00;
        };
        add_text_prop(&mut buf, 0x280, (ROOM_TEXT_ADDR / 4) as u16); // #6 "it": the room
        add_text_prop(&mut buf, 0x1F2, (DECOY_ADDR / 4) as u16); //    #5 "forest": the decoy

        // global_vars = 0x300. The decoy object is named by a global too, and is
        // probed FIRST (objects are tried in ascending order).
        put_word(&mut buf, 0x300 + g as usize * 2, room_obj);
        put_word(&mut buf, 0x300 + (g as usize + 1) * 2, 5);
        let mut m = make_machine(buf);
        m.screen.v6 = Some(v6_band(&[(11, 71, text), (11, 489, "Score: 0")]));
        m
    }

    #[test]
    fn v6_detect_room_from_a_global_confirmed_by_a_property_string() {
        let m = machine_scott_shaped(3, 6, "deep caverns");
        // Guard: the room is NOT findable by any rung above — its short name is
        // "it" and no avatar reaches it.
        assert_eq!(normalize_name(&short_name(&m.mem, 6)), "it");
        assert_eq!(resolve_room_object(&m, "deep caverns"), None);

        let loc = detect_location(&m).expect("the global-plus-property room must be detected");
        assert_eq!(loc.method(), LocationMethod::StatusName);
        let obj = loc.object().expect("object-backed");
        assert_eq!(obj.number, 6, "the exact object the global names");
        assert_eq!(obj.name, "deep caverns", "named from the property, not the short name \"it\"");
    }

    #[test]
    fn v6_global_room_needs_the_shown_text_to_corroborate_it() {
        // Same story, but the global points at object #1 ("forest"), which carries
        // no such text. A global is never trusted for being a global.
        let m = machine_scott_shaped(3, 1, "deep caverns");
        assert_eq!(detect_location(&m), None, "an uncorroborated global must not become a room");
    }

    #[test]
    fn v6_global_room_ignores_a_too_short_candidate() {
        // A stray one- or two-glyph run must never prefix-match its way into
        // blessing a global (V6_GLOBAL_ROOM_MIN_LEN).
        let mut m = machine_scott_shaped(3, 6, "deep caverns");
        m.screen.v6 = Some(v6_band(&[(11, 71, "de"), (11, 489, "Score: 0")]));
        assert_eq!(detect_location(&m), None, "a two-glyph run must not resolve a room");
    }

    #[test]
    fn v6_global_room_probe_never_latches_a_memory_fault() {
        // The probe decodes property words that are not string addresses at all.
        // Whatever it reads, the story must be left un-faulted (SQ-0724).
        let m = machine_scott_shaped(3, 6, "deep caverns");
        let _ = m.mem.take_mem_fault();
        assert!(detect_location(&m).is_some());
        assert_eq!(m.mem.take_mem_fault(), None, "speculative probing must not fault the story");
    }

    // ── TDD Step 1: write the failing tests ───────────────────────────────────
    // (These were written BEFORE the implementation; the RED→GREEN cycle is
    //  documented in the task report.)

    // ── current_location: v3 hit ──────────────────────────────────────────────

    #[test]
    fn v3_current_location_from_global0() {
        let mut buf = build_v3_story();
        // Set global 0 (at GLOBAL_VARS) to object 1.
        put_word(&mut buf, GLOBAL_VARS as usize, 1);
        let machine = make_machine(buf);
        let loc = current_location(&machine).expect("should return a snapshot");
        assert_eq!(loc.number, 1);
        // Name should be "west" (our encoded name for obj1).
        assert!(loc.name.starts_with('w'), "expected name starting with 'w', got {:?}", loc.name);
    }

    // ── current_location: v3 None when global0 == 0 ──────────────────────────

    #[test]
    fn v3_current_location_none_when_global0_zero() {
        let mut buf = build_v3_story();
        // global 0 = 0 → no location
        put_word(&mut buf, GLOBAL_VARS as usize, 0);
        let machine = make_machine(buf);
        assert!(current_location(&machine).is_none());
    }

    // ── current_location: None when global0 exceeds max object ───────────────

    #[test]
    fn v3_current_location_none_when_global0_out_of_range() {
        let mut buf = build_v3_story();
        // 0xFFFF is never a valid object in our tiny tree
        put_word(&mut buf, GLOBAL_VARS as usize, 0xFFFF);
        let machine = make_machine(buf);
        assert!(current_location(&machine).is_none());
    }

    // ── object_tree_view: returns all 3 objects with correct fields ───────────

    #[test]
    fn v3_object_tree_view_count_and_fields() {
        let mut buf = build_v3_story();
        // global 0 = 0 (irrelevant for tree view)
        put_word(&mut buf, GLOBAL_VARS as usize, 0);
        let machine = make_machine(buf);
        let tree = object_tree_view(&machine);

        assert_eq!(tree.len(), 3, "expected exactly 3 objects, got {}", tree.len());

        // obj1: number=1 parent=0 name starts 'w'
        assert_eq!(tree[0].number, 1);
        assert_eq!(tree[0].parent, 0);
        assert!(tree[0].name.starts_with('w'), "obj1 name: {:?}", tree[0].name);

        // obj2: number=2 parent=1 name starts 'e'
        assert_eq!(tree[1].number, 2);
        assert_eq!(tree[1].parent, 1);
        assert!(tree[1].name.starts_with('e'), "obj2 name: {:?}", tree[1].name);

        // obj3: number=3 parent=1 name starts 'h'
        assert_eq!(tree[2].number, 3);
        assert_eq!(tree[2].parent, 1);
        assert!(tree[2].name.starts_with('h'), "obj3 name: {:?}", tree[2].name);
    }

    // ── Fixture-backed test (skips when minizork.z3 absent) ──────────────────

    #[test]
    fn minizork_current_location_returns_something() {
        let Some(story) = crate::fixtures::load("minizork.z3") else {
            // Fixture absent — skip.
            return;
        };
        let mem = Memory::new(story).unwrap();
        let mut machine = Machine::new(mem);
        machine.init_caps();
        // In minizork the opening location is set in global 0 before the first
        // read instruction.  Run until NeedLine (the first READ opcode) so that
        // the init code has had a chance to store the starting room into global 0.
        for _ in 0..100_000u64 {
            match machine.step() {
                StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart | StepResult::Fault => break,
                StepResult::NeedChar => { machine.supply_char(b'\n'); }
                StepResult::SaveRequest => { machine.complete_save(false); }
                StepResult::RestoreRequest => { machine.complete_restore_failure(); }
                StepResult::Continue => {}
            }
        }
        let loc = current_location(&machine);
        assert!(loc.is_some(), "minizork: expected a location from global 0, got None");
    }
    #[test]
    fn minizork_object_table_stops_at_the_real_end_no_garbage() {
        // Regression: the object-entry scan used to walk past the real table into
        // the property-table data and over-count, surfacing garbage objects with
        // corrupt names. Bounded by the lowest property-table pointer, minizork
        // has exactly 179 objects; the last is "pseudo" (the pseudo-object), not
        // a garbage-named phantom.
        let Some(story) = crate::fixtures::load("minizork.z3") else {
            return; // fixture absent — skip
        };
        let mem = Memory::new(story).unwrap();
        let machine = Machine::new(mem);
        let tree = object_tree_view(&machine);
        assert_eq!(tree.len(), 179, "minizork has 179 real objects");
        assert_eq!(tree.first().map(|s| s.name.as_str()), Some("forest"));
        assert_eq!(tree.last().map(|s| s.name.as_str()), Some("pseudo"));
    }
}
