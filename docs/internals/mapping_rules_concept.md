# IF Mapping rules

Generally room constraints are all relative.

## Layout Process

* Rooms are first arranged on a fixed grid, with grid line spacing = 0 (i.e. there is no space between room grids)
* As new room placement occurs, rooms will likely need to be expanded to allow new placement. Care should be taken to retain constraints during this expansion (algorithm research here?)
* Once rooms layout is complete, grid in spacing is added to allow paths to travel between rooms
* Paths are drawn from left to right, top to bottom order. Paths should always take direct
* If additional grid spacing is required to complete a path, paths are cleared, universal grid spacing increased, and paths are redrawn
* Once all paths are completed, map is scanned in horizontal and vertical directions to remove unnecessary grid spacing

## Path Rules

* Paths should always take the straightest path possible to their destination, with the fewest number of turns.
* Paths may push and pull rooms as long as the room constraints are not violated
* Paths should not overlap or cross

## Strong Constraint (linear movement only)

* This occurs when two rooms are directly connected to each other in both directions (i.e. room a has a direct path to room b, and room b has a direct path to room a).
* Linearly aligned (horizontally/vertical/diagonal): rooms have a shared reciprocal direct path. E.G. If we travel from A to B going west, and from B to A going east the two rooms are considered to be horizontally aligned.
  * Placement rules: rooms must be aligned along their constraint axis, but may be any distance apart. Another room cannot lie in between.
* Non-linear: rooms are connected to each other buy not reciprocally. E.G. we travel from A to B going west, and from B to A going N
  * Placement rules: rooms must be placed in positions which preserve their relative position as expressed by the path taken between the two rooms. E.G. A -w-> B, and B -n-> A represents that B is southwest of A. Therefore B must be one or more cells south and one or more cells west of A.
* Path rules: Another path may not intersect or overlap the line between these rooms.
* Room connector location: center for H and V, or corner for D
* Arrows: Outgoing arrows on both sides of the path.
* Exceptions: It is possible for rooms to have multiple paths to each other.
  * If one room has a single path to it’s neighbor, but the neighbor has multiple paths back, the single path’s direction should be used as the constraint axis. All connectors should coalesce along that path before entering the room with the single path.
  * If both rooms have multiple paths to each other, the constraint can be relaxed in relation to these paths and multiple axises can be evaluated during layout.

### Diagrams

```
     +---+     +---+
-<=>-| A >-<=>-< B |-<=>-
     +---+     +---+
```

Travel from room A in E direction, return path W from B to A.

```
  ^
  |
  v
+---+
| A |
+-v-+
  ^  
  |
  v  
+-^-+
| B |
+---+
  ^  
  |  
  v  
```

Travel from room A in S direction, return path N from B to A.

```
+---+
| A >-<=>-+
+---+     |
          ^
          |
          v
          |
        +-^-+
        | B |
        +---+
```

Travel from room A in E direction, return path N from B to A.

## Weak Constraint (sub-planar movement)

* This occurs when a room is connected to another, but there is not a return path.
* Rooms should honor the directional constraint in the single dimension specified by the path, but are free to move otherwise. For example, if we travel from room B west, and reach room B, B may be placed anywhere in the plane starting one cell west of A. Directional direction
* Path rules: Another path may not intersect or overlap the line between these rooms.
* Room connector location: starting room: centered (H/V), corner (D); destination: flexible, preferred closest edge to starting room, centered (H/V), corner (D) at destination if no conflict, otherwise pushed to nearest location without overlap
* Arrows: Outgoing arrow at starting room, no arrow at destination
* Exceptions: It is possible for a room to have multiple paths to another. Directions should be averaged to define plane for placement. E.G. paths going N/E/S to a destination: destination plane would be east of starting room.
  * Paths may overlap and converge before reaching destination

### Diagrams

#### Example Weakly Connected Room Allowed Movement

```
       |
       |    ^
       |    |
       |    v
+---+  |  +---+
| A >--|--| B |-<=>-
+---+  |  +---+
       |    ^
       |    |
       |    v
       |
```

Travel from room A in E direction, no return path from B.

```
+---+
| A |
+---+
     \ 
      \ _____________
       | +---+
       | | B |-<=>-
       | +---+
       |   ^
       |   |
       |   v
       |
       |
```

Travel from room A in SE direction, no return path from B.

# Up / Down / In / Out

* The above rules generally apply to any portions of the map that exist on the same plane (i.e. the rooms are not stacked)
* When we traverse up or down we enter another
