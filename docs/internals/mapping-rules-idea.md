# lanthorn Automapper — Current Rules Reference

> **Purpose.** A complete, faithful snapshot of how the automapper places rooms and
> draws the map *today*, so we have a clear baseline to critique and redesign. This
> is descriptive, not aspirational — it documents what the code does now, including
> its exceptions, tensions, and stale comments. Edit freely to turn it into the
> improved rule set.
>
> Source of truth as read on 2026-07-02 (HEAD `4adf7f9`). File:line refs are into
> `crates/mapper/src/` and `crates/app/src/` as noted.

---

## 0. Vocabulary & coordinate conventions

- **Cell** — integer grid coordinate `(x, y)` a room occupies. `+x = east`, `+y = south`,
  so **north is `−y`**. This convention is pervasive (`direction.rs:57-65`).
- **Compass direction** — one of the 8 planar directions (N/S/E/W + NE/NW/SE/SW).
  `grid_offset(dir)` returns `Some((dx,dy))` for these (`direction.rs:56-70`).
- **Portal / non-planar direction** — Up, Down, In, Out, Unknown. `grid_offset`
  returns **`None`** for all five. This `None` is the de-facto "is this a portal?"
  test used everywhere in the engine.
- **Direction parsing** (`direction.rs:18-49`): also maps nautical terms — `fore/forward/bow → N`,
  `aft/stern → S`, `port → W`, `starboard → E` (for Seastalker-style games).
- **Layer** — a manual grouping of rooms drawn as one floor/region. `MAIN_LAYER = 0`
  always exists. Layers are never auto-derived; only explicit peel/merge create them.
- **Two layout regimes**: *incremental* (per-turn, local, stable) and *relayout*
  (on-demand, global, re-derives everything). See §2.
- **Distorted edge** — a compass edge whose geometry can't be honored (was dropped as
  a constraint, or endpoints aren't correctly aligned). Rendered magenta.

---

## 1. Where layout runs (invocation map)

### 1a. Per turn — incremental (always, in Auto mode)
`Mapper::observe` (`mapper.rs:14-39`) runs on every location observation:
1. `upsert_room(location, name)`.
2. First room ever (`current == None`): anchor at `(0,0)` if unplaced.
3. Otherwise, if the new location differs from the previous room:
   - `add_edge(prev, dir, location)` (dir defaults to `Unknown` if the move command
     didn't parse to a direction).
   - **Auto mode only:** `place_incremental(graph, prev, location, dir)` (§3).
4. `set_current(location)`.
5. **Auto mode only:** `mark_distorted(graph, ∅)` over the whole graph (cheap; no relayout).

Then, in the app bridge `apply_turn` (`app/session.rs:379-386`), **Auto mode only**:
- `cleanup_overlaps(graph, radius=2, max_passes=20)` — a light per-turn overlap sweep
  so the live map never shows an illegal connector overlap.

### 1b. On demand — full relayout + tidy pipeline (background job)
Triggered from the main loop (`app/main.rs:2999`) via `should_bg_tidy(...)`, which spawns
a worker thread running `tidy_layer_silent` on the **active layer's subgraph**
(coalesced: only if no tidy job is already in flight; a stale result is discarded and
re-run — generation guard at `app/input.rs:1699`).

`should_bg_tidy` (`app/input.rs:1736-1773`):
- Gate 0: if the turn **did not change the graph** (`!changed`) → never tidy. (`changed = new_room || new_conn`.)
- Then by `background_tidy` config mode (`app/config.rs:239-254`, default **`EveryRoom`**):
  - `Off` → never.
  - `EveryRoom` → `new_room || overlap`.
  - `OnOverlap` → `overlap` only.
  - `Debounced` → overlap fires immediately (resets counter); else count new rooms,
    fire at `BG_TIDY_DEBOUNCE = 5`.
  - `overlap = has_overlap || has_distorted` for the active layer.

`tidy_layer_silent` pipeline (`app/input.rs:1640-1673`), **exact order**:
1. `sub = graph.layer_subgraph(layer)`
2. `relayout_auto(sub)` — mapper-core global solve (§4)
3. `cleanup_overlaps(sub, 3, 40)` (§5a)
4. `repair_directional_hints(sub, 3, 40)` (§5b)
5. `stack_updown_rooms(sub)` (§5c)
6. `cleanup_overlaps(sub, 3, 40)` (again)
7. `compact_empty_lines(sub)` (§5d)
8. write positions + distortion flags back to the live graph for that layer.

> **Note.** Steps 3–7 live in `crates/app/src/render/map.rs`, *not* in the mapper
> crate — they are app-side post-processing that consumes the mapper's scoring
> functions. The mapper core is only step 2. The animated variant `run_tidy_pipeline`
> (`app/input.rs:1455`) runs the identical stages, capturing a frame per move (cap 2000).

`LayoutMode { Auto, Manual }` (default `Auto`, `layout/mod.rs:50-55`) gates all of the
above; in Manual mode nothing auto-places, and the user drives placement with `nudge`
(which only moves a room to a *free* cell, `mapper.rs:58-75`).

---

## 2. The two regimes, contrasted

| | **Incremental** (`place_incremental`) | **Relayout** (`relayout_auto`) |
|---|---|---|
| When | Every turn a new room is seen | On-demand background tidy |
| Scope | One new room, local | Whole active layer, from scratch |
| Stability | Existing rooms almost never move | Re-derives *all* positions |
| Strategy | Trizbort-style: place at compass offset, shift rooms "beyond" on collision | Constrained stress-majorization (sort → constraints → SMACOF/VPSC → align → contiguify → pack) |
| Up/Down | Soft hint (prefers N/S, *yields* on collision) | No constraint at all; fixed up afterward by app-side `stack_updown_rooms` |
| Guarantees | Integer cells, no overlap in-layer | Integer cells, no overlap; determinism |

Both regimes keep every room on an integer cell and never let two rooms share a cell.
The **router** (path drawing) is a separate concern layered on top of whatever
positions these produce (§6).

---

## 3. Incremental placement rules (`incremental.rs`)

`place_incremental(graph, prev, dest, dir)`:

1. **Revisit guard** — if `dest` already has a position → **no-op** (loop closures never
   move a placed room). `incremental.rs:16`
2. **Prev-unplaced guard** — if `prev` has no position → defensive no-op. `:19`
3. **Layer inheritance** — `dest` joins `prev`'s layer. `:25`
4. **Offset selection** (`:32-37`):
   - Compass dir → `grid_offset(dir)`.
   - `Up` → hint `(0, −1)` (directly north); `Down` → hint `(0, +1)` (directly south).
   - `In`/`Out`/`Unknown` → `None`.
5. **Placement** (`:38-65`):
   - **`ideal = prev_pos + delta`.** If `ideal` is free (within the layer) → place there.
   - If `ideal` is occupied:
     - **True cardinal move and not up/down** → `shift_beyond` (push the whole run of
       rooms at/after `ideal` one step further along the axis to open the cell), then
       place at `ideal`. This is the only case that displaces existing rooms.
     - **Up/Down hint, or a diagonal** → *yield*: place at `nearest_free_cell` from `ideal`.
       (Up/Down never push neighbors aside — the hint is soft.)
   - **`In`/`Out`/`Unknown` (`delta == None`)** → `nearest_free_cell` starting from `prev_pos`.

`shift_beyond` (`:71-87`): translates every placed room **in the same layer** at or beyond
`ideal` along the step axis by one unit. "Beyond" is a half-plane test per axis. Other
layers are never touched.

`nearest_free_cell` (`mod.rs:71-107`): returns `from` if free; else spirals outward in
square rings (top row, bottom row, left col, right col), deterministic tie-break by that
traversal order.

**Consequence to note for redesign:** Up/Down rooms are placed by a *hint that yields*,
so on any collision they scatter to a nearby free cell with no vertical preference
preserved — the reason the app-side `stack_updown_rooms` pass exists to pull them back.

---

## 4. Relayout (global solve) — `relayout_auto` (`mod.rs:588-825`)

Re-derives all positions for the layer. Pipeline stages (observer labels in order:
`seed → stress → align → contiguify → pack`):

**Guards first:** empty graph → return; `> MAX_NODES (400)` rooms → use `sort_layout`
only and return (`:617-636`).

### 4.1 Seed — topological sort (`sort.rs`)
`sort_layout` assigns integer cells by **per-axis longest-path layering**:
- For each compass edge with `grid_offset = Some((dx,dy))`, add an ordering edge on each
  non-zero axis (e.g. E puts origin left of dest; N puts dest above origin since north = −y).
  Non-compass edges contribute nothing (`sort.rs:177-180`).
- `layer_axis` (`:17-53`): drop any edge that would **close a cycle** (`reaches` check),
  then Kahn topological sort + longest-path relaxation (`coord[w] = max(coord[w], coord[v]+1)`).
- `align_free_axes` (§4.3) tidies unconstrained axes.
- Pack components left-to-right at `nearest_free_cell`, 1-cell gap between components,
  then **anchor the lowest room-id at `(0,0)`**.

### 4.2 Stress majorization — `stress.rs` + `vpsc.rs`
Per connected component (Unknown edges excluded from adjacency; Up/Down/In/Out still connect):
- `all_pairs_dist` = BFS hop-count matrix (ideal edge length `GAP = 1.0`).
- `build_axis_constraints` (§4.4) produces separation constraints.
- `stress_layout` runs **`ITERS = 60`** SMACOF iterations. Each iteration: Guttman
  transform for x → project through `vpsc::solve_axis` (Dwyer block-merge VPSC, enforces
  `x[right] − x[left] ≥ gap`) → write x; then the same for y. `n ≤ 1` returns the seed unchanged.
- Result rounded to integer cells (`snapped`).

### 4.3 Align — `align_free_axes` (`sort.rs:87-152`)
`ALIGN_PASSES = 4`. A node is "Y-free" if it has no compass edge with `dy≠0` (symmetric for X).
Each free node snaps to the **lower-median** of its E/W (or N/S) neighbors' coordinate,
preferring *constrained* neighbors as anchors. Only free nodes move, so no ordering is violated.
Returns per-node `(x_constrained, y_constrained)` flags used by collision resolution.

### 4.4 Constraints — `build_axis_constraints` (`constraints.rs:44-117`)
- **Chain equalities first, unconditionally** (`:54-80`): for every E/W chain, adjacent
  members get a gap-0 equality on **Y** (share a row); for every N/S chain, a gap-0
  equality on **X** (share a column). Added to the adjacency *before* directional
  constraints so a later contradicting directional constraint is the one dropped.
- **Directional constraints**, taken **strongest evidence first**, insertion order breaking
  ties inside each tier. `layout_offset(dir)` gates, so In/Out/Unknown create no constraints
  at all, while Up/Down do — as the weakest tier. The three tiers:
  1. **Reciprocated compass pairs** (SQ-1287) — the passage walked from both ends, two
     observations agreeing.
  2. **One-way compass edges** — one observation, but still the game's own word.
  3. **Up/Down**, reciprocated or not (SQ-1291).

  For each non-zero axis, add a `≥ gap` separation in the correct order (north = smaller y).
  If adding it would **close a cycle** (`creates_cycle`), the edge is *dropped* and recorded
  in `dropped` (→ later marked distorted). The ordering is what decides WHICH of two
  contradicting edges survives, so it must not be the player's route: a passage walked from
  both ends is better evidence than a lone one-way crossing. Taken in mint order instead,
  Adventure's opening put `In A Valley` north of `At End Of Road`, because the first move of
  the game was one step north into a random forest room the valley happens to share a row with.
  - **Why Up/Down come last** (SQ-1291). North-for-up is a *drawing convention* this crate
    invents in `layout_offset`; `grid_offset` — what `mark_distorted`, the chain detector and
    the router all read — says Up/Down have no bearing at all. A compass word is the game's
    own statement of where a room IS, so it must outrank a staircase even when the staircase
    was walked both ways and the compass word only once. Zork I's `East-West Passage` is the
    case: it reaches `Chasm` by a reciprocated `Down`/`Up` pair *and* the chasm names its own
    `SW` return. Ranked as a reciprocated pair, the stairwell claimed the Y axis first, pinned
    the chasm a row south, and the `SW` bearing — arriving at a Y axis that already
    contradicted it — was dropped and drawn distorted. The map then disagreed with both the
    game's prose ("a stairway leading down at the north end of the room") and the player's own
    return walk.
  - A dropped Up/Down constraint costs nothing on screen: `mark_distorted` gates on
    `grid_offset`, so a stairwell is never drawn distorted whatever the solver does with it.
- **Rooms whose own evidence contradicts itself are left out entirely** (SQ-1289,
  `positionally_unreliable`). A room qualifies on two counts together: (1) two of its
  **outgoing** compass edges name the same neighbour on opposing sides of an axis —
  "the valley is west of me" *and* "the valley is east of me" — and (2) it has **no
  reciprocated compass pair** anywhere to anchor it. Every edge touching such a room,
  inbound as well as outbound, is skipped: no separation constraint, and (unlike a
  cycle-closing drop) nothing added to `dropped`, so the edge is flagged distorted or
  not purely on where the room ends up. Self-loops and non-compass directions are not
  evidence and neither test looks at them.
  - Outgoing only: `A E B` together with `B E A` is two rooms each making one coherent
    claim, and the acyclic pass above already settles that by dropping one leg. Marking
    both rooms unreliable would throw away the half of the evidence that is sound.
  - Condition (2) is what keeps this narrow. A room with one muddled bearing and a
    passage walked from both ends keeps all its constraints — the reciprocated pair
    (§4.4 above, SQ-1287) is the best evidence the map has. It also means an unreliable
    room can never be a chain member, so the equalities above are untouched.
  - The room still lands on the map: the stress solve places it by graph distance among
    its neighbours, and §4.6 gives it a free cell. It simply stops pushing rooms whose
    geometry *is* known apart on the strength of a position it does not have.
  - Adventure's `In Forest` #55642 is the case this was found on. The story scatters
    arrivals between two rooms of that name (SQ-1264), so exits recorded against one id
    were walked from whichever forest the player was really in, and the bundle is a
    mixture. `HILL S FOREST` and `VALLEY Up FOREST` contradict nothing, so `creates_cycle`
    never got a say — the forest quietly claimed the row between `At End Of Road` and
    `In A Valley` and pushed the valley to road + 2. Note that *ordering* those edges last
    changes nothing (measured): the sort only decides which of two **contradicting** edges
    gives way.

### 4.5 Contiguify (`mod.rs:492-524`)
Pulls chain members back onto their shared row/column after stress if they've drifted,
but **only** if the chain still holds its equality (skips a chain whose equality was
dropped, or with < 2 members in the component). Members are never moved off-line.

### 4.6 Pack & resolve collisions (`mod.rs:746-771`)
Normalize each component to its own origin, place components left-to-right (`pack_x += 2`,
one-cell gap). Resolve any residual collisions with the rooms
that have real geometry first and the `positionally_unreliable` ones (§4.4) last, each
group in ascending room-id order, via `place_preserving_alignment` — which searches *along* a constrained row/column (to keep
alignment) or spirals if free on both axes. The ordering is the guarantee that a room with
no geometry never bumps one that has geometry off its cell (SQ-1289) — room id alone would
decide it by an accident of the story's object numbering. Finally anchor lowest room-id at
`(0,0)` and `mark_distorted`.

### Scoring functions (mostly consumed by app-side passes)
- `edge_is_satisfied` (`mod.rs:158`): non-compass → **always true**; compass → strict —
  both axes' signs must match `grid_offset` exactly (zero axis must be *exactly* aligned).
- `room_side_score` (`:306`): count of a room's compass edges kept on the correct **side**
  (cross-axis free). Reciprocal (bidirectional) edges weighted ×2 (`RECIPROCAL_WEIGHT`).
- `room_alignment_score` (`:341`): same, but **strict** (exact row/column) — protects
  clean chains from being disturbed.
- `directional_hint_score` (`:375`): whole-map weighted count of directed edges on the
  correct side. A satisfied **compass** hint is worth more than every satisfied **Up/Down**
  hint on the map put together (SQ-1291), so the sum reads as a lexicographic comparison —
  compass hints first, stairwells only as the tie-break, the same rule §4.4 sorts its tiers
  by. Without it §5b undid §4.4: the solver put Zork I's `Chasm` north-east of the
  `East-West Passage`, honouring the chasm's own `SW` return, and the repair pass then
  dragged it due south because straightening the stairwell's two legs scored +2 against the
  bearing's +1. Scores are only ever compared **within one graph**, so deriving the compass
  weight from that graph's connection count is well defined.
- `room_compass_degree` (`:396`): number of a room's compass edges (fewer = safer to nudge).
- `mark_distorted` (`:566`): non-compass edges are **never** distorted; compass edges are
  distorted if dropped or not `edge_is_satisfied`.

---

## 5. App-side tidy passes (`crates/app/src/render/map.rs`)

All operate on the layer subgraph, use `render_overlap_stats` (renders the plan and counts
illegal connector overlaps), and are greedy + bounded + deterministic. All restore the
prior position on any rejected trial.

### 5a. `cleanup_overlaps(graph, radius, max_passes)` (`map.rs:1671`)
Each pass: if zero overlaps, stop. Otherwise try moving each placed room to every cell
within Chebyshev `radius` (rings ordered nearest-first) and commit the single best move
that strictly reduces `(overlaps, crossings)`. **Guards on every trial:**
- Never onto an occupied cell.
- `move_keeps_updown_sides` (`:1633`): a move must not flip a currently-correct Up/Down
  side, nor drag a room out of its partner's column if it's stacked there.
- Never reduce `room_side_score` (don't break a satisfied side hint).
- Tie-break key: `(overlaps, alignment_broken, side_broken, crossings, compass_degree, id, move_idx)`.

Called with `(2, 20)` per turn (light) and `(3, 40)` in the background pipeline (twice).

### 5b. `repair_directional_hints(graph, 3, 40)` (`map.rs:1770`)
Sibling of cleanup, but **maximizes `directional_hint_score`** (fixes one-way edges that
ended up on the wrong side). Each pass commits the single room move with the greatest
strict gain in satisfied hints, subject to: adds no illegal overlap, and does not reduce
that room's `room_alignment_score` (never undoes chain alignment). Converges on strict
improvement only.

### 5c. `stack_updown_rooms(graph)` (`map.rs:1923`)
The corrective pass that gives Up/Down rooms their vertical placement (since the solver
gives them none). For each Up edge (`dest` should be directly **north**) and Down edge
(`dest` directly **south**):
- **Anchor skip** (`:1948`): if `dest` currently sits axis-aligned on a *straight* cardinal
  edge to some other room, leave it — a portal stack must not pull it off a real compass
  neighbor it lines up with (e.g. Canyon View lined up due east of Clearing).
- **Preferred targets, in order** (`:1963`): directly N/S of the partner, then the two
  diagonal-adjacent cells (NW/NE for Up, SW/SE for Down). `try_stack_dest_at` seats it at
  the first cell that can be opened.
- **Opening a cell** (`try_stack_dest_at`, `:2051`): (1) shift the ideal column outward as
  a *closed set* so chains travel whole (E/W row-chain mates + whatever sits in a shifting
  room's path), then drop `dest` in; or (2) **cluster-drag** — move `dest` together with its
  small (≤ `CLUSTER_LIMIT = 4`) unanchored compass-edge cluster by the same delta. Both are
  guarded: no collision, no new overlap, no side-hint loss, no exact-alignment loss.
- **Yield** (`:1978`): if no adjacent cell can be opened, keep the room on the correct
  **side** at the nearest free, overlap- and hint-safe cell within `YIELD_RADIUS = 10`.
  If none is clean, **band-shift**: translate everything beyond the ideal cell one step
  out (the following `cleanup_overlaps` clears the transient overlaps).

Helper `exact_alignment_count` (`:2188`) = number of compass edges that are exactly
axis-aligned; the pass must never reduce it.

### 5d. `compact_empty_lines(graph)` (`map.rs:1856`)
Collapses fully-empty interior rows/columns left behind by earlier passes. For each empty
line, shift every room beyond it one cell toward it (translates a half-plane uniformly, so
all relative order and alignment survive and no two rooms collide). **Reverted entirely if
it raises the overlap count** (cosmetic tightening never worth a new overlap).

---

## 6. Path (connector) routing & drawing — Boxes zoom only

Compass connectors are line-art; Compact/Overview draw none (Compact shows stub labels only,
Overview nothing).

### 6.1 The router (`crates/mapper/src/route/mod.rs`, `router.rs`)
Produces a `RoutePlan` of `RoutedConnector`s in **doubled coordinates** (room cell `(c,r)` →
`(2c, 2r)`; channels live on odd coordinates between rooms). Highlights:
- Prefers **direct routes**, then L/Z paths with ≤ 2 bends; greedy crossing minimization.
- **Lanes**: multiple connectors sharing a channel are assigned distinct lanes so they
  run parallel without overlapping.
- **Reciprocal dedupe**: a true opposite pair (A→N→B and B→S→A) collapses to one connector.
- **Merge stubs / T-junctions**: an extra edge between an already-connected pair routes to
  the existing trunk and ends there (renders departure arrow only).
- **Routing failure fallback**: if both L-corners are blocked, a direct 2-point segment is
  drawn and the edge marked `distorted = true` (rendered magenta).
- **Diagonal side rule**: N/S axis dominates — NE/NW attach on Top, SE/SW on Bottom
  (`router.rs:70-80`). One-way diagonal *entry* sides: NW→Top, NE→Right, SE→Bottom, SW→Left.

### 6.2 Where a connector touches a box border — `box_edge_anchor` (`map.rs:1004`)
With a box at cols 0–10, rows 0–4 (center col 5, row 2), the **slot-0** (single connector)
anchors are:

| Direction | Border cell | (col, row) |
|---|---|---|
| E (Right) | right edge, mid | (10, 2) |
| W (Left)  | left edge, mid  | (0, 2)  |
| N (Top)   | top edge, mid   | (5, 0)  |
| S (Bottom)| bottom edge, mid| (5, 4)  |

Multiple connectors on one side **fan out** via `slot_offset` (0 = center, then +1, −1,
+2, −2 …), clamped to stay off the corners (`v_max = 1` vertical, `h_max = 4` horizontal).
Diagonal exits use the box **corners** instead (`corner_anchor`, `:258`): NE→(10,0),
NW→(0,0), SE→(10,4), SW→(0,4).

The connector leaves the box **perpendicular** (90° straight stub on the anchor's own
row/col) out to the channel, then steps along the edge into the lane (`attach_bridge`,
`:1033`) — so a slot-displaced connector crosses a centered one as a clean `┼`.

### 6.3 Channel geometry — `lane_pixel` (`map.rs:704`)
- Box column center = `box_left + BOX_W/2` = box_left + 5.
- Vertical channel lane `k` = `box_left + BOX_W + LANE_BASE + k·LANE_SPACING` = box_left + 11 + 1 + 2k
  — **lane 0 sits one cell into the gutter, never on the box edge**.
- Row center = `box_top + BOX_H/2` = box_top + 2; horizontal channel lane `k` = box_top + 5 + 1 + 2k.

### 6.4 Glyphs — `glyph_for(mask)` (`map.rs:669`)
Direction bits `N=1, E=2, S=4, W=8` OR-accumulated per cell in a shared map, so crossing
connectors merge into junctions. Defaults (`symbols.rs:112`): `─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼`;
unknown mask → `·`. Style: `connector` (cyan) normally, `connector_distorted` (magenta)
when `conn.distorted`.

---

## 7. Portal connectors (Up/Down) — `draw_portal_connectors` (`map.rs:1108`)

Boxes zoom only, style `portal_connector` (cyan). Drawn **before** rooms (so stub cells
under a neighboring box are harmlessly covered).

Rules:
- **Only Up/Down get a dotted line.** In/Out/Unknown → no line at all (`:1135`).
- A reciprocal Up/Down pair is handled **once, from the Up side** (`:1139`).
- Skipped entirely if a **compass connector already joins** the pair (`:1147`).
- **Cleanly stacked** — dest is exactly one cell up/down of origin (`dc == (oc.0, oc.1±1)`):
  draw one L-shaped dotted connector on the box's **right column (col 9, = `BOX_W-2`)**,
  aligned with the in-room portal icons. Vertical run uses `┊` (`portal.path`); horizontal
  run uses `┄` (`portal.path_h`) at the target's mid-row. Both clipped out of room interiors.
- **Yielded** — not adjacent: no long map-spanning line. Instead `portal_stub` on each room —
  origin points out in the portal direction, dest points back.

`portal_stub` (`map.rs:1193`), on column `box_left + BOX_W-2` (col 9):
- Up: dot `┊` one cell above the box, glyph `↑` two cells above.
- Down: dot `┊` at row 5 (just below the box), glyph `↓` at row 6.

---

## 8. Portal icons (in-room) — `draw_portal_icons` (`map.rs:1246`)

Boxes zoom only, drawn **after** rooms (overlay on the box). Default glyphs (`symbols.rs:125`):
`↑` up, `↓` down, `⊙` in, `⊗` out, `?` unknown, `●` notes marker.

Each room's stub (portal) edges map to a slot (`portal_slot`, `:1217`):

| Slot | Directions | Interior row (numbers-shown) |
|---|---|---|
| 0 | Up | row 1 |
| 1 (mid) | In / Out / Unknown | row 2 |
| 2 | Down | row 3 |

Mid-slot contention (a room with several of In/Out/Unknown) resolves by `mid_precedence`
(`:1227`): In ▸ Out ▸ Unknown (lower wins).

**Unknown special case (`:1278`):** edges with `dir == Unknown` are skipped entirely — no
in-room icon *and* no dotted connector. So the `?` glyph is effectively never drawn in-room;
the mid slot only ever shows In/Out in practice.

Three placement modes:
1. **Portal view** (`show_portal_labels`): icons move **onto the border**, destination names
   float outside. Up → top-border center (5,0), name above; Down → bottom-border center (5,4),
   name below; mid → right border (10,2), name to the right (Unknown: glyph only, no name).
2. **Numbers shown**: icons in the interior right column (col 9), Up=row 1, mid=row 2,
   Down=row 3. If an Up icon claims the top-right corner and the room has notes, the `●`
   marker shifts one cell left (col 8) so both stay visible.
3. **Numbers hidden**: all present glyphs on interior row 3, space-separated, centered in the
   9-wide interior.

**None of this reaches a SAME-LAYER stairwell** (SQ-1291, worth knowing before hunting a
badge on the wrong side of a box). A same-layer Up/Down passage is lane-routed like any
compass passage, so it is never a stub and never enters `draw_portal_icons` at all: its `↑`/`↓`
rides the connector's **departure anchor** on the box border (§7, `render_lane_connectors`),
which is derived from the two rooms' cells and therefore already faces the partner. What does
reach `badge_bearing`'s Up/Down arm is the **cross-layer** portal, whose destination is on
another plane — so no partner cell exists to aim at, and top/bottom (the direction of travel
off the plane) is both the only answer available and the right one. When a stairway glyph
appears on the wrong side of a room, the layout put the partner there; the badge is a
faithful reporter.

---

## 9. Arrows

### Compass connector arrowheads — `draw_connector_arrows` (`map.rs:939`)
- Drawn **last** (after rooms and portal icons), so they embed in the room border,
  **replacing the box-edge glyph** and pointing outward.
- Suppressed entirely in portal view (`show_portal_labels`) — that view shows only portal
  border icons.
- Departure glyphs (default `Arrows`, `symbols.rs:102`): E `▶`, W `◀`, N `▲`, S `▼`;
  diagonals `↗ ↖ ↘ ↙`.
- **Far-end (arrival) arrow only for reciprocal connectors.** One-way edges get a departure
  arrow only.
- The arrow cell's background is painted to match the room box's *visible* background
  (accounting for the REVERSED fg/bg swap on the current room); arrow fg = connector color.

### Portal arrows
Up/Down glyphs (`↑`/`↓`) serve as the arrows, drawn by `portal_stub` (yielded case) and as
in-room icons. In/Out use `⊙`/`⊗` (not directional). **No arrowheads are drawn on the dotted
portal connector line itself.**

---

## 10. Room box drawing (Boxes zoom, 11×5) — `draw_box_room` (`map.rs:1495`)

Box occupies cols 0–10 × rows 0–4. Border glyph set chosen by `outline_for` (`:1365`) with
precedence **current ▸ portal ▸ selected ▸ normal**:
- `room_normal` `╭╮╰╯─│` (rounded), `room_current` `┏┓┗┛━┃` (heavy),
  `room_portal` `╔╗╚╝═║` (double, when the room owns an outgoing cross-layer portal),
  `room_selected` = normal (selection is **color-only**).

Interior (width 9):
- Name: `wrap_two(label, 9)` — word-wrap into ≤ 2 lines, over-long word truncated — centered
  on **rows 1 and 2**.
- Row 3: `#<id>` centered (+ ` <align_code>` when `show_alignment`), **only when
  `show_room_numbers`**; otherwise row 3 is freed for portal icons.
- Notes marker `●` at col 9, row 1 (top-right interior).

Color via `room_style` (`:290`): current+selected → selected + REVERSED; current →
room_current (reversed white); selected → yellow; else white.

**Compact (8×3)**: single-line label truncated to 6 chars (no wrap, no id, no icons).
**Overview (1×1)**: a single `■` glyph, no borders/connectors/icons/arrows.

---

## 11. Render order — `render_map` (`map.rs:435`)

Bottom → top (each layer overwrites the previous):
1. *(Overview early-return: `■` per room, nothing else.)*
2. Compute axes + scroll offset; place rooms in virtual space.
3. **Compass connectors** (line-art) — returns arrowhead list.
4. **Portal connectors** (dotted Up/Down).
5. **Rooms** — boxes overwrite all line-art beneath them.
6. **Portal icons** overlay (on the box).
7. **Connector arrowheads** — embed in borders (unless portal view).

`render_map_layered` (`:619`) may add a layer-tab strip on top and a detection-method
indicator in the bottom-right.

---

## 12. Coordinate systems (⚠ two of them, and they disagree)

- **System A — uniform `cell_to_screen`** (`map.rs:308`): step **19×11** for Boxes, 12×5
  Compact, 2×2 Overview. **Effectively test-only for Boxes today** — no production render/
  hit-test path uses it. `recenter_on` deliberately uses 13×7 for Boxes instead.
- **System B — non-uniform packed axis (`PosTable` / `boxes_axes`)** (`map.rs:175-242`): the
  real render + hit-test + recenter path at Boxes zoom. Box stride = `BOX_W/BOX_H` +
  `channel_width`; minimum **13×7** (`BOX_W+MIN_GUTTER` × `BOX_H+MIN_GUTTER`). Busy channels
  widen only their own column/row (non-uniform).

The whole map is laid out in scroll-independent "virtual" space and blitted with one
translate + clip, so connector routes never shift as you pan.

Constants: `BOX_W=11`, `BOX_H=5`, `MIN_GUTTER=2`, `LANE_BASE=1`, `LANE_SPACING=2`.

**Stale docs to ignore:** the header comment at `map.rs:8-12` claims Boxes step 29×17 —
wrong; the live value is 19×11 (`state.rs:879`). Trust the code, not that comment.

---

## 13. All special cases / exceptions (consolidated)

**Placement / layout:**
- Revisit never moves a placed room; first room anchored at `(0,0)`.
- Up/Down are soft yielding hints incrementally; In/Out/Unknown get nearest-free (no hint).
- `Unknown` edges don't group rooms into a connected component and exert no graph-distance
  pull; Up/Down/In/Out still connect components but create no constraints.
- Non-compass edges create no sort ordering, no constraint, no chain, no alignment, and are
  never distorted.
- Cycle-closing ordering edges / constraints are dropped (and the dropped compass edge is
  marked distorted).
- `> MAX_NODES (400)` rooms → sort-only, no stress solve.
- Layers: relayout and placement are per-layer; peel is a no-op if the region is the whole
  layer; merging MAIN is a no-op.

**Rendering:**
- Unknown portals: no in-room icon, no dotted connector.
- Dotted portal line only for Up/Down, and only when no compass edge already joins the pair.
- Reciprocal compass/portal pairs dedupe to one connector; only reciprocals get a far-end arrow.
- Merge stub (extra edge between already-connected rooms) ends at the trunk (departure arrow only).
- Routing failure → direct distorted (magenta) segment.
- Notes `●` shifts left when an Up icon takes the corner.
- Row-3 id vs portal-icon reuse depends on `show_room_numbers`.
- Everything clipped per-cell to the pane; off-area rooms culled/clamped for hit-testing.
- Compact = no line-art (stub labels only); Overview = glyphs only.

---

## 14. Sample layouts

All samples are **Boxes zoom, numbers-shown**, one box = cols 0–10 × rows 0–4.

### 14.1 A single room (anatomy)
```
 col: 0123456789A       (A = 10)
row0  ╭─────────╮
row1  │ West of ●│   ● = notes marker (col 9)
row2  │  House   │   name centered on rows 1–2 (wrap_two)
row3  │   #1     │   "#<id>" centered (numbers-shown)
row4  ╰─────────╯
```
The current room uses the heavy outline `┏━┓ ┃ ┗━┛`.

### 14.2 Two rooms, reciprocal N/S (A —N→ B, B —S→ A)
North = smaller y, so B sits **above** A. One connector (reciprocal dedupe); arrowheads on
**both** borders. Connector attaches at each box's vertical center-top / center-bottom
(col 5), runs through the gutter channel between them.
```
      ╭─────────╮
      │  Forest  │   room B  (north)
      │   #2     │
      ╰────▼────╯     ▼ = B's departure arrow, south toward A (B's bottom border, col 5)
           │           single cyan │ in the 2-cell gutter (lane 0)
      ╭────▲────╮
      │ West of  │   room A  (south)
      │  House   │     ▲ = A's departure arrow, north toward B (A's top border, col 5)
      ╰─────────╯
```
- Each room shows its own outward-pointing departure arrow on the shared border. A one-way
  A→N→B would show only A's `▲`; the reciprocal back-edge adds B's `▼`.

### 14.3 Two rooms E/W, plus a one-way branch
`A —E→ B` (reciprocal) and `A —N→ C` (one-way):
```
  ╭─────────╮          ╭─────────╮
  │  Kitchen │◀────────▶│ Pantry  │     E/W: connector on the shared row-center (row 2),
  │   #1     │          │   #3    │      arrowheads at (col 10,row2) of #1 and (col 0,row2) of #3
  ╰────▲────╯          ╰─────────╯
       │  one-way N (departure arrow ▲ only, on #1 top border col 5)
  ╭────┴────╮
  │  Attic   │  #5
  ╰─────────╯
```

### 14.4 Cleanly stacked Up portal (dest exactly one cell north)
`A —Up→ B` with B placed directly north (by `stack_updown_rooms`). No compass edge joins
them, so a **dotted** connector is drawn on the **right column (col 9)** with `↑`/`↓` icons
in each room's right interior column:
```
      ╭─────────╮
      │  Loft   ↑│   B: Up-icon at (col 9,row1); this is the "up" partner (north)
      │   #2    ┊│   dotted ┊ on col 9
      ╰─────────╯
                ┊    (vertical ┊ in the gutter, clipped out of interiors)
      ╭─────────╮
      │  Hall   ↑│   A: Up-icon (its Up edge → B); dotted line leaves top on col 9
      │   #1    ┊│
      ╰─────────╯
```
(For a reciprocal Up/Down the pair is drawn once from the Up side; a Down partner shows `↓`
on its col-9 slot row 3.)

### 14.5 Yielded Up portal (partner far away)
When B could not be stacked adjacent, no long line is drawn. Instead each room gets a stub +
glyph on col 9: the origin points out (`↑` above it), the far room points back (`↓`).
```
   (room B, somewhere else)          ↓          ← portal_stub on B: ↓ two rows below? 
                                     ┊             (dest points back opposite the portal dir)

              ↑    ← portal_stub on A: glyph ↑ at row −2, dot ┊ at row −1 (col 9)
              ┊
        ╭─────────╮
        │  Hall   │  A  (has the Up edge)
        ╰─────────╯
```

### 14.6 In/Out/Unknown icons (no lines)
In/Out/Unknown never draw a connector. They only appear as an in-room mid-slot icon
(row 2, col 9), by precedence In ▸ Out ▸ Unknown — **except Unknown, which is drawn as
nothing at all**:
```
  ╭─────────╮
  │  Cave    │
  │  #7    ⊙ │   ⊙ = In portal icon (mid slot, col 9 row 2). ⊗ if Out.
  ╰─────────╯      (an Unknown-only exit here would show NO icon)
```

### 14.7 Portal view (labels on, `show_portal_labels`)
Icons move onto the border; destination names float outside:
```
        Loft                ← Up destination name, above
      ╭────↑────╮           ↑ on top-border center (col 5, row 0)
      │  Hall    │⊙ Cellar   ⊙ on right border (row 2) + name to the right (mid slot)
      │          │
      ╰────↓────╯           ↓ on bottom-border center (col 5, row 4)
        Basement            ← Down destination name, below
```

---

## 15. Known tensions & smells (for the redesign discussion)

1. **Up/Down have no place in the solver.** The mapper core treats Up/Down (and In/Out/
   Unknown) as `grid_offset == None` and gives them zero geometric influence. All vertical
   placement is bolted on afterward by the app-side `stack_updown_rooms`, which is a large,
   heavily special-cased greedy pass. A prior experiment to teach the constraint engine a
   full up/down diagonal broke compass chains — the fundamental tension between "portals as
   spatial hints" and "compass chains as hard constraints" is unresolved.
2. **Layout logic split across two crates.** The global solve is in `mapper`, but four of
   the six pipeline stages (overlap cleanup, hint repair, up/down stacking, compaction) live
   in `app/render/map.rs`. The tidy pipeline order is defined in `app/input.rs`. There is no
   single place that owns "the layout algorithm."
3. **Two coordinate systems that disagree** (§12), with `cell_to_screen` effectively dead for
   Boxes zoom, plus a stale header comment claiming 29×17.
4. **Router runs after positioning with no feedback.** Positions are fixed first, then paths
   are routed; a routing failure just marks an edge distorted rather than nudging rooms apart.
   Room "size" (number of exits) does not influence placement.
5. **Greedy, overlap-driven correction.** cleanup/repair/stack are all local greedy hill-
   climbs guarded by scores; they converge but give no global optimality and their
   interaction order matters (cleanup runs twice, around stacking).
6. **Portal drawing couples to placement.** The dotted-line vs stub decision depends purely
   on whether `stack_updown_rooms` happened to seat the partner adjacent — geometry and
   semantics are entangled.
</content>
</invoke>
