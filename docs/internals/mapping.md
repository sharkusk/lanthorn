# Live automapping

> For players, the short version is in [the guide](../guide/the-map.md).

[← back to README](../../README.md)

Play the game; the map draws itself. Every room you enter and every exit you take
is boxed, connected, and de-overlapped on the fly, then continuously nudged into a
clean layout — no graph paper, no pausing to annotate, no manual placement. Walk
north and a new room slides into place north of where you stood; double back and
the connection closes into a loop. This is lanthorn's flagship feature, and it is
the reason the map pane earns half your terminal.

The mapper is deliberately **engine-agnostic**. It never sees a Z-machine opcode
or a Glk call — it consumes a plain stream of *locations* and *movements* and
turns it into a spatial graph. That means the **same automapper draws every
game**, whether you're charting the Great Underground Empire in *Zork*, threading
*Counterfeit Monkey* in Glulx, or exploring *Adventureland* in a classic Scott
Adams adventure. One map builder, three engines, zero special cases.

![lanthorn playing Zork I with a live automap of the Great Underground Empire](../automapping.png)

## Knowing where you are — across three engines

Before the mapper can place a room it has to be told which room you're in, and
each engine surfaces that differently. lanthorn handles all of it, and records
*how* it worked out each room the first time it finds it — click a room, then
click the room panel's **Diagnostics** tab, to see "Found by:" there. It is kept
with the room, so the answer is still there long after the turn that
discovered it.

- **Classic Z-machine (v3)** reports the room in the status-line variable —
  `via status variable`.
- **v4/v5 Z-machine games that hide it** (Hitchhiker, Bureaucracy, A Mind
  Forever Voyaging) don't expose a room in the classic variable, so the room name
  is read off the status line and resolved back to a game object — preferring the
  player object's room when the game re-parents the player, Inform-style
  (`via player object`), and falling back to a name-only room otherwise
  (`via name match`, or `via name (unlinked)` when it can't be tied to an object).
  A status line can also label several things at once — *The Impossible Stairs*
  reads `Year: 2001  Place: Front Lawn` — and lanthorn does not assume which label
  means "room": each labelled field is offered to the object tree, and the one the
  game recognises as a place is the one mapped, under the name on screen rather
  than the compiler's identifier for it.
- **Games with no room objects to find** (*The Impossible Bottle* and *The
  Impossible Stairs*, both compiled by Dialog; *Facility*; *frankenfingers*) name
  their rooms only on screen — there is nothing in the object tree to tie the name
  back to, ever. A name with nothing behind it is trusted to open a map only when
  the **story itself** printed it too, as a heading in the prose and not just on
  the status bar: real rooms are named twice, in two independent places, while a
  title screen or a character sheet is named once. That is what keeps *Beyond
  Zork*'s character sheet — which shows your name exactly where a room name goes —
  off the map, and it is the same evidence the Glulx side asks for.
  Games that **center** their room title in a custom status display (Beyond Zork,
  Trinity) are parsed too — the centered heading is accepted only once it
  validates against the player's room — so those now automap as well.
- **Graphical v6 Z-machine** (Zork Zero, Shogun, Arthur) has no status line at
  all: the bar is *painted* pixel by pixel wherever the game feels like putting
  it. lanthorn finds it by asking where the prose window starts and reading the
  band directly above it — which is how Arthur works, since it hides its bar
  twelve rows down the screen, tucked under a full-width panel of artwork. The
  glyphs are laid back onto their columns first, because Arthur paints its bar one
  letter at a time; the room then goes through exactly the same checks as every
  other Z-machine game (`via player object`, or `via name match` when the game
  doesn't re-parent you, as Shogun doesn't). Some games never reserve the band at
  all and simply *overlay* the bar on the top row of a full-screen prose window
  (advent.z6): a short, full-width strip pinned to the top of the screen counts as
  the band even though nothing is "above" the prose. A title banner or a right-hand
  date field is never promoted to a room on a name match alone, and Journey — whose
  story window owns the top of the screen and whose menus sit below it — correctly
  reports no room at all (its menu window is the whole screen, not a strip).
- **v6 ports that keep the room somewhere else again.** Brian Howarth's eleven
  *Mysterious Adventures* are Scott Adams games rebuilt as v6 Z-code, and they
  duck every check above at once: nobody is ever put in the object tree (the
  player object's parent stays empty for the whole game), and every room object
  answers to the same compiled name, `ScottRoom`, with the line you actually read
  — "I'm in a dense SPOOKY Forest" — tucked away in a property. What the games do
  keep is a variable holding the room you're standing in, and lanthorn takes it —
  but only after checking that the room it names is carrying, in its own
  properties, the very words on the screen this turn. Object tree and screen have
  to agree, every turn, and no variable is trusted just for being a variable. The
  payoff is an exact room, which matters here more than anywhere: these games
  reuse a description across whole mazes — ten rooms in *Feasibility Experiment*
  all read "I'm in a Tunnel" — and a room known only by its name would fold every
  one of them into a single dot.
- **Glulx (Inform 7)** games often keep the room out of the status bar entirely,
  so lanthorn reads the **Inform room heading** — the bold title line printed as
  you enter a room (`via room heading`). Games like FooFoo and Superluminal
  Vagrant Twin map cleanly this way; rooms are matched by name since the Glulx
  world model isn't introspectable, and pre-game menus or character-setup screens
  correctly produce no room. Front matter is bolded the same way a room is, so
  lanthorn reads the *shape of the page* rather than the words on it: Inform
  joins a room heading to the description underneath it and then hands you the
  **command prompt**, while a title, an act list or a content warning stands
  alone above a blank line on a page that never gets that far. That is why THE
  BAT's act list and its prologue's newspaper strapline don't become rooms, and
  why Adventure in `superbrief` — where a room is a bold line, a blank line and a
  list of what's lying about — still does. The prompt is the test, not merely
  "the game wants typing": Cragne Manor's CONTENT WARNING and CONCEPT WARNING
  pages ask you to *type* yes or no, and read the answer themselves without ever
  printing a `>`, so they stay off the map while Cragne's Railway Platform — the
  first page that does end at a prompt — goes on it.
  The shape is read in **every** buffer window, not just the one the game happens
  to be printing in when the turn starts: a GWindows title like *City of Secrets*
  opens a second buffer mid-prologue and prints its opening room into that,
  becoming the story window only once the turn ends. Scanning just the window that
  was the story window at the time would lose the room you actually start in —
  and with it the object-tree read that tells the inventory panel what you're
  carrying (SQ-1241).
  A heading owns its LINE, too, and that cuts both ways. Counterfeit Monkey's
  HIGHLIGHT option prints every object's name in bold — the same style the room
  heading uses — so a bolded noun opening a sentence used to become a room ("I
  typed GET ALL and lanthorn detected a new room called *ear*"), and a bolded
  NPC's name opening the line *directly below* a heading used to swallow the
  heading whole, which is how Brown's Lab spent a session being called the Samuel
  Johnson Basement. A word following the name on its own line says "sentence";
  a line the newcomer owns outright above prose says "the heading was real"
  (SQ-1285, SQ-1295).
  All of that is what lanthorn does when it has to *watch* a game to find out
  where you are. An **Inform 7** story has already written the answer down.
  The compiler emits the whole map as one array in memory — `Map_Storage`, one row
  per room, one column per direction — and puts every room's printed name in a
  property beside it, so which objects are rooms, what each is called and where
  each direction leads are all in the file before a single turn is played.
  Nothing in a Glulx image names a table or an address, so each of those four
  facts is recovered from its own signature rather than looked up (`gvm::i7map`'s
  header derives them at length, against Inform's own runtime template).
  Where that read succeeds it becomes the top of the order of authority, and it
  changes three things:
  * **The room you start in is a real room from the first prompt.** The name on
    screen is matched against the story's own room names and, when exactly one
    answers, the map keys the room by that room's object address — so Counterfeit
    Monkey's Back Alley is on the map before you type anything, and never as a
    name-derived node that has to be corrected later. Two rooms sharing a name is
    a maze, which a name cannot settle, so lanthorn declines instead of guessing.
  * **The `location` global is found on your first move, not your tenth command.**
    The learner below needs several unambiguous room changes to tell that global
    apart from every counter that moves with it; a word that has just changed to
    the room the story has *just named* is that global on one move's evidence.
    Measured on Counterfeit Monkey: the tenth command became the first step north
    out of the alley, and every room mapped in between stopped being provisional.
  * **A room is called what the story calls it**, not what it happened to print
    this turn. Heading, status line and silent `look` can each spell one room
    differently — Counterfeit Monkey's bar reads `Back Alley, noon` where its
    heading reads `Back Alley` — and two spellings of one room is exactly how one
    room becomes two dots.

  The exits come with it. An I7 room's own row says where each direction leads, so
  the check a Z-machine game gets against its compiled exit table — noticing that a
  passage's destination *varies* rather than minting a false edge for it — now runs
  for Inform 7 games too. That row lives in *writable* memory, on purpose: a story
  can move its own passages at run time ("change the north exit of the Hall to the
  Cellar"), and Counterfeit Monkey does. So lanthorn reads it fresh on every ask
  rather than trusting a copy taken at launch.

  None of this is available everywhere, and everything below is what happens when
  it is not. An Inform 6 game has no such array; nor does an Inform 7 build older
  than the array itself (*Anchorhead: Special Edition Demo* is one); and a game
  that *builds* its map as you play — Kerkerkruip deals a fresh dungeon every
  time — ships one full of zeros, which lanthorn refuses rather than reporting
  whichever three-room accident scores highest. Those games map exactly as they
  did before, by the chain that follows.

  Once lanthorn has learned where a Glulx game keeps its `location` global,
  though, the heading stops being the thing that decides you have MOVED — it only
  says what the room is called. The story's own word is better evidence, and it is
  right about the turns the heading is wrong about: a car that drives you across
  town narrating the trip without reprinting a room, and a flashback that prints a
  heading for somewhere you have never been. And when the story moves you
  somewhere it declines to name — including the room you wake up in, which
  Counterfeit Monkey never announces until you type LOOK — lanthorn asks it, by
  running a `look` in a copy of the game and throwing everything but the answer
  away (SQ-1293, SQ-1294).
  The moment the lock lands is its own small event: every room walked before it is
  still keyed by the hash of its heading, so the learner hands back the real id for
  each and the map re-keys them in place. The session's OWN cached room has to be
  re-keyed in the same breath — it is only rebuilt on a turn the lock calls a move,
  so without that a single `wait` afterwards would hand the map an id it had just
  retired and draw the room you were standing in a second time, tangled into its own
  twin (SQ-1304). Rooms that *shared* a heading before the lock are a harder case
  and still open: they were one node, and re-keying can rename a node but not split
  one.
  And when a game will not print a heading at all, the **status line** is the
  answer. *The Wizard Sniffer* keeps the room name entirely in its own two-row
  status bar — `Atop a Mountain` over `Exit: north` — and prints nothing but the
  description below, so every route above came up empty and the map stayed blank
  for the whole game. Reading the bar is exactly what lanthorn has always done for
  the Z-machine, and it now does it for Glk too, under three conditions that keep
  it off any game the heading already serves: the story must have printed no
  heading anywhere, the silent `look` must have been spent and come back nameless
  (so a game that *will* name its room when asked still answers in its own words),
  and the turn must end at the command prompt, so a status bar behind a title card
  is as much a banner as a bold line on one. What the bar holds must also look like
  a room: City of Secrets paints "For instructions and information type ABOUT…"
  there and FooFoo's whole bar is the bare word "Exits:", and a sentence or an
  empty label is not a place (SQ-1302).
- **Scott Adams** adventures feed their locations straight through the same
  engine-agnostic pipeline — nothing special to configure.

## Getting around the map

The map is a place you can move through, not just a picture.

- **Zoom** — `zoom-map in|out|reset` (or a signed step) scales between a detailed
  boxed view and a compact overview.
- **Pan** — `pan-map <dx> <dy>` slides the viewport; `center-map` snaps back to the
  selected room, or the room you're standing in.
- **Layer tabs** — multi-level areas are split into named **layers** shown as a tab
  strip across the top of the map (e.g. `Main  Cellar  Maze`, each with its room
  count); the active tab is highlighted, and a layer flagged as a maze carries a
  trailing `⌗` marker (`Maze ⌗`) in both tab strips. `cycle-layer next|prev` switches
  between them. Carving a layer off and folding one back turned out to be the same
  move — *take these rooms and put them on that layer* — so there is one verb for
  both: **`move-region [destination] [direction]`**. `move-region new` carves the
  region onto a fresh layer, `move-region main` folds it back into Main,
  `move-region parent` sends it home to whatever it was carved from, and any layer
  name works in place of those (`move-region Cellar`).
  Everything is anchored on the **selected room** — click one, or `select-room` —
  and *its* side of whatever gets cut is the side that travels. You never have to
  point at an edge, because lanthorn works out which rooms go, in three steps.
  First it walks the compass exits and stops where the portals are: that is a floor,
  a cellar, a tower, and it needs nothing from you but the room you picked. If the
  walk finds no portal to stop at and swallows the whole layer — Zork's underground
  being thirty-odd rooms of solid compass maze — it looks instead at the passages
  leading **into** your room, and cuts the one that is a genuine boundary. Exactly
  one usually is, and it says which: *cut the S passage from At West End of Long
  Hall*. That is the case that used to be unsayable, because the way in may be
  one-way and there is then no direction out of the room that names it. If several
  ways in are real boundaries — you are standing mid-corridor, and cutting east or
  cutting west take opposite halves of the map — it *asks*, offering them by name and
  by size (`e from A (2 rooms)`), because either answer would be a guess. That prompt is
  the only answer that always works: a maze happily has two rooms whose **south** exits
  both land where you are standing (Adventure's does), and no direction you could type
  would tell them apart. You can still name one from the command line when a direction
  does distinguish them (`move-region new e`); a direction that leads nowhere *in* is read
  as the passage leading *out*, which is how you name a one-way exit. The destination
  follows the same rule: leave it off and `move-region` takes the only possible answer
  when there is one, and offers you the choices when there is not. Nothing is ever severed — the passage you cut at simply becomes a
  connection *between* layers, which is why every move goes back the way it came.
  Because the destination is just an argument, a stranded room finally has a cure. A
  room discovered while exploring a maze layer is minted *onto* the maze layer even
  when it is really outside — a back door to the surface, say — so select it and
  `move-region main` cuts it off the maze and sends it home in one go. Rooms
  keep their positions where free; a room whose cell is taken in the destination
  lands on the nearest free one. Two things it will refuse, and it says which: a
  fresh layer for a region that is *already* the whole layer (that would only rename
  it), and anything that would leave `Main` with no rooms at all.
- **The map sometimes speaks first** — twice in a game, lanthorn notices that a set of
  rooms wants to be a layer of its own, and says so. It never acts: layers still come
  only from a `move-region` you asked for. It **suggests**, and you decide.
  The first case is structural: a set of rooms that hangs off one portal and nothing else.
  Two things have to be true of it whichever way you are walking, and each one is a prompt
  you would otherwise have got and not wanted. The cellar must be reachable **only**
  through portals: a balcony you can also walk round to is not behind a boundary at all,
  whatever the `up` says. And it must be **four rooms or more**, because a cupboard behind
  a door is not a floor plan and drawing it in place with a dotted stub is the right answer.
  There are then two moments it can be noticed at, because there are two shapes of cellar.
  Climb down a trapdoor, wander four rooms, climb back up, and *that* is the moment: the
  map has finished being drawn and you are the one who closed it. But Zork's trapdoor
  crashes shut and is barred behind you, and a cellar with no way out would on that rule
  never be mentioned at all — so when the rooms beyond a portal grow into a floor plan
  while you are still down there, lanthorn speaks on the room that makes it four, and then
  goes quiet. One offer per region, not one per room you add to it.
  What it offers is always the side you are **on**, never the side you came from — that is
  the whole safety of asking on the way in. A region only counts as one when every room in
  it was found *after* the room it hangs off, so your starting town, which predates
  everything, can never be what a prompt proposes to peel away.
  The second case is the name. Walk into a room called **Maze** from a room that isn't
  one, and lanthorn says so at the doorway — no four-room floor, no waiting for you to
  return, because the name *is* the evidence and it is there immediately. It fires on
  the way **in**, once, and then goes quiet: Zork's maze is fifteen rooms all called
  "Maze", and asking in each is the nagging this whole design exists to avoid.
  `\bmaze\b`, as a word — "Amazement Park" is not a maze and neither is "Amazed".
  When the rooms look like they belong on a layer that **already exists**, that layer is
  offered too, alongside a fresh one. The evidence is compass edges and nothing else,
  ranked by how many: a region tied to the Cellar layer by two east-west passages is
  probably part of the Cellar. Portals never nominate a home — they are what separates
  layers in the first place, so the `down` you reached the cellar by must not be read as
  proof the cellar belongs upstairs. Grid position is not evidence either; the router
  derives it, so a suggestion built on it could change without the map changing.
  And whatever you answer, lanthorn remembers it, in the map file, against the passage you
  will cross again — the way out when you were noticed leaving, the trapdoor itself when
  you were noticed inside. **Not now** re-arms
  the seam for your next crossing, **not this passage** silences that one passage for good,
  **never for this story** silences the whole prompt — every trigger, every passage — for the
  rest of this map, and folding a layer back into another silences every passage it just
  closed — you have already said those rooms belong together. A prompt that comes back on
  the very next step is worse than no prompt at all, because it teaches you to dismiss it
  blind.
  A layer you have flagged as a maze is exempt from the structural trigger outright:
  the point of flagging it was to keep the whole maze together.
  The offer itself is a small modal: what it noticed, which rooms would travel, and the
  destinations on offer as a list you arrow or `Tab` through — `Enter` accepts wherever the
  focus is resting, because landing on a choice already selects it. It says what it noticed
  the way round it noticed it, spelling the passage out: *"You came UP out of Cellar"* when
  you were caught leaving, *"You came DOWN from Living Room"* when you were caught inside —
  and then, of both, the one thing the region walk actually proves, which is that no compass
  passage reaches those rooms. The rooms themselves are a bulleted list, one to a row under a
  count, so the modal grows **taller** for a big region rather than eliding names into a line
  too narrow to hold them. Past eight names it stops naming and starts counting — *"…and 12
  more"* — and on a terminal too short for all of it the list is what gives up rows, never the
  choices or the buttons. Four buttons, and they
  are the four answers: **Separate** does it, **Not now** re-arms the seam for your next
  crossing, **Not this passage** silences that one passage for good, and **Never for this
  story** silences the prompt entirely — structural and maze-name alike — for the rest of
  this map. `Esc` means *not now* — declining to
  answer is not the same as saying no, which is why there is no Cancel. And it waits its
  turn: a suggestion never shoulders in front of a dialog you opened yourself, and a dropped
  one costs nothing, because nothing is written down until you answer.
- **Switching layers recenters the view** — cycling, clicking a tab, moving a region,
  or loading a map all land the viewport somewhere with a room in it, never on empty
  scroll space: on the room you're standing in if it's on the layer you switched to,
  else the last room you visited there, else that layer's own bounding-box centre. A
  matrix layer selects the same room as its row and scrolls the table to show it.
- **View mode** — `view-map` (leader `u`) switches the active layer between the **drawn**
  map and the **matrix** — the direction table described below. Bare, it cycles; `view-map
  drawn` / `view-map matrix` sets it outright. The choice is per-layer and saved with the map,
  so a maze can stay a table while everything around it stays a map.
- **Room card** — the [room panel](interface.md#the-room-panel)'s Room body (`toggle-room-panel`,
  leader `k`, or left-click a room) lists **every** travel direction, not just the ones that go
  somewhere: where each leads, how it comes back, which you tried and found walled up (`×`), and
  which you have never tried at all (`·`). That is the map's answer to "where haven't I been?",
  one room at a time — and the dock follows you as you walk, so the card is about wherever you
  are standing unless you pin it to a room by clicking one. The twelve directions lay out in up
  to three columns (cardinals, diagonals, portals) when the dock is wide enough, so the card
  costs four rows rather than twelve.
- **Room diagnostics** — `toggle-inspector` flips the same dock to its Diagnostics body: the
  room's id, name, layer, position, and the per-edge layout constraints, so you can see *why* a
  room landed where it did.
- **Hand edits** — select rooms with `select-room next|prev`, `rename-room` /
  `rename-layer`, jot `edit-notes`, or clean up the graph with
  `delete-connection` and `relabel-edge`. Room-number labels toggle with
  `toggle-room-numbers`. Room *positions* are the layout engine's — re-run
  `tidy-map` rather than placing boxes by hand.
- **Export** — take the map with you: `export-svg` writes a scalable vector image,
  `export-dot` emits Graphviz DOT (render it with `dot -Tsvg …`), and `export-map`
  writes the raw structure. Omit the filename for a default path in the game's data
  directory. A saved map can be reopened later with `load-map`.

## Connections that stay readable

A naïve "one arrow per exit" map dissolves into spaghetti fast. lanthorn routes
connections through a lane system with crossing-elimination and overlap removal,
and it understands the awkward cases:

- **Vertical connections** — up/down moves place the new room directly north (up)
  or south (down) of its neighbour, shoving ordinary rooms aside like a compass
  move but yielding to confirmed reciprocal N/S adjacencies. They render as dotted
  connectors with up/down (or stairs) glyphs — never as arrows, never as "distorted"
  red edges. A matching Up+Down pair between two rooms collapses to a single dotted
  path marked at both ends. Where a room pair is joined by *both* a compass direction
  and a staircase, only one line is drawn — see below for which wins.
- **Nautical directions** — ship games (Seastalker and kin) that steer by
  *fore / aft / port / starboard* (plus *bow* / *stern* / *forward*) instead of the
  compass are understood: those map onto north / south / west / east so the vessel's
  decks lay out correctly. Ships with quarter directions too (Counterfeit Monkey's
  Atlantida Herself) are understood the same way — *fore-port / fore-starboard /
  aft-port / aft-starboard* and every abbreviation the story accepts (`f`, `a`/`af`,
  `p`, `sb`, `pf`/`fp`, `sf`/`fs`/`fsb`, `pa`/`ap`, `sa`/`as`/`asb`) map onto the four
  intercardinals.
- **Combined multi-direction paths** — two rooms get **one** line between them, however
  many ways you can actually walk it. Zork's around-the-house ring links each pair by
  both a cardinal and a diagonal; Adventure's maze will happily connect the same two
  rooms four different ways, and a staircase often shadows a compass passage. Drawing
  them all means lines that exist only to cross each other, so lanthorn picks a single
  representative: a **reciprocal** pairing first — the two ends are exact opposites, so
  the line runs straight and each arrowhead points the way you really travel — and
  otherwise by direction priority, **N, S, E, W, NE, NW, SE, SW, up, down**. The line
  that wins keeps its own arrowhead (or `↑`/`↓` if a staircase won), and each passage
  that lost stamps its **own glyph beside the shared line's anchor** — a staircase that
  lost to a compass edge shows its `↑` on the border of the room it climbs from, so a
  known way back never disappears into the collapse. Lines carrying more than one
  passage are also tinted with the `shared_path` selector — and the room panel's Diagnostics
  body lists every exit with its direction and destination, so nothing is lost, only
  unstacked.

Where two unrelated connectors still have to cross, the map says so rather than drawing a
junction: the vertical run passes through unbroken and the horizontal one breaks for a single
cell, so a crossing never reads as a place the two passages meet.

A one-way connector never lands its arrowhead on a room's own compass anchor — the
mid-side cell a real exit or a `?` random-exit mark in that direction uses, or a
diagonal's shared corner — even when nothing else claims it: an arrival there would
draw exactly like "that direction leads back here", which for a one-way passage isn't
true. It settles beside that cell instead. Only a reciprocal pair is exempt, because a
reciprocal genuinely *is* the return path.

When two connectors both want the same straight room-line and neither will fit beside the
other, the longer one keeps it and the shorter weaves instead — and if they tie on length
too, the one that runs straight keeps the line over one that bends, so which route wins
never depends on the order the rooms were explored in.

Confirmed reciprocal N/S and E/W adjacencies are treated as inviolable: an up/down
move yields rather than shove a reciprocal partner off its shared column or row, and
overlap cleanup may only slide a reciprocal room *along* its own axis, never off it.

That **N, S, E, W, NE, NW, SE, SW, up, down** order above is which single line the
*render* draws when several passages share a room pair — it says nothing about how
the rooms got their positions. The *layout* engine breaks its own ties differently:
when a cycle on the grid forces it to give up one direction's evidence, it gives up a
diagonal before a cardinal. A diagonal only pins a room to the right quadrant (it's
satisfied by any offset with the right two signs, not an exact unit step), so
stretching one draws a slightly wider corner; a cardinal means exactly one shared row
or column, so losing one is a door that vanishes from the map entirely. Zork's
around-the-house ring is the case that forced this: the diagonal skirt (West of
House–North of House–Behind House) and the cardinal spine through the front door
(Behind House–Kitchen–Living Room–West of House) close one cycle together, and it's
the ring's own corners that give a little rather than any of the three doors.

## Keeping the layout tidy

The whole map re-optimizes itself as you discover rooms, so it stays readable as it
grows. How eagerly is up to you (`background_tidy`): after every new room (the
default), only when a new room overlaps an old one (`on_overlap`), debounced every
few rooms (`debounced`), or off entirely. Force a pass any time with `tidy-map`.

Two touches happen after the solve, because the solver cannot make them itself.
A **leaf** — a room whose compass exits all lead to one neighbour — is snapped onto
that neighbour's doorstep, since the separation a compass edge buys the solver is
only a minimum and a room hanging off the side of the map otherwise drifts two or
three cells out with nothing in between. And a **hub** — a room with two or more
passages walked from both ends — is no longer shoved aside to tidy a row it happens
to sit in, because the intersection of its own bearings is generally the one cell
that honours every door it has.

One thing outranks even the hub, and it is the rule the whole engine is built around:
two rooms joined by a north/south or east/west passage walked from both ends are *next
to each other*, and nothing may stand between them. A hub that wants a cell in the
middle of such a pair gives it up, whatever that costs its own corners.

**Maze layers are left alone.** A layer flagged as a maze (below) is *frozen*: it
schedules no tidy, `tidy-map` on it answers "maze layer: geometry is frozen — the
matrix is the view", and its rooms keep the positions they were first given. There
is no compass arrangement of a maze to converge on — the layout engine would keep
producing a different wrong one every turn, and the pane would keep repainting for a
grid nobody is reading. Only the *optimization* stops: rooms, passages and tried
directions go on being recorded exactly as before, and a newly discovered room is
still placed where the move you walked says it should go, so unflagging the layer
(or switching it back to `view-map drawn`) shows a real map again.

Curious how a layout got built? `animate-tidy` steps through the whole assembly
stage by stage — a **Build** stop that lists every connection, then
**room-by-room placement** as each box drops onto the grid, then the
relayout/overlap-cleanup passes with each move described ("moved 180 to clear
overlap with 193"). Step it with `anim-step forward|back`, play/pause with
`anim-play`, and leave with `anim-exit`. It's equal parts diagnostic and quietly
mesmerising.

## Mazes: the matrix view

![The matrix view over Colossal Cave's all-alike maze: rows of rooms, columns of directions, footnotes naming the door in and the way out](../maze-grid.png)

A compass map of a maze is a lie told carefully. In one real, half-explored
mapping of Colossal Cave's "all alike" maze — twelve rooms, forty-seven passages —
**two** passages come back the way you went. Eighteen come back by some other
direction, twenty-seven have no known return at all, and the layout engine has to
mark twenty-nine of the forty-seven "distorted" because no arrangement of boxes on
a grid can satisfy them. Eleven of the twelve rooms are called "Maze".

Compass geometry is not what a maze *is*. What you actually know in a maze is a
direction table per room: *west from here goes to that one, and the way back is
north*. So lanthorn will draw you the table.

```
               N     S     E     W    NE    NW    SE    SW     U     D     I     O
──────────────────────────────────────────────────────────────────────────────────
 Maze 1     →5⇠w    ⇢9    ⇢2    ⇢3     ·     ·     ·     ·     ·     ·     ·     ·
 Maze 2       ⇢3   ⇢10  →7⇠n    ⇢9     ·     ·     ·     ·     · →11⇠w     ·     ·
 Maze 3    →11⇠u    ⇢5  →9⇠e →10⇠s     ·     ·     ·     ·    ⇢4     ·     ·     ·
 Dead End¹    ⇄4     ·     _     ·     ·     ·     ·     ·     ·     ·     ·     ·
 Maze 4       ⇢1   ⇄DE  →5⇠s  →6⇠w     ·     ·     ·     ·    ⇢8    ⇢2     ·     ·
 …
▸Maze 11      ⇢8  →7⇠w    ⇢6  →2⇠d     ·     ·     ·     ·  →3⇠n  ⇱out     ·     ·
──────────────────────────────────────────────────────────────────────────────────
¹ Dead End, near Vending Machine
⇱out: D from 11 → At West End of Long Hall
⇲ in:  At West End of Long Hall —S→ Maze 11
```

One row per room, one column per direction — **all twelve, always**. An untried
cell in any direction may be exactly the thing full exploration needs, so none are
hidden however empty the column looks.

| Cell   | Meaning |
|--------|---------|
| `⇄4`   | reciprocal — the compass inverse brings you back |
| `→5⇠w` | goes to 5, and **w**est is the way back (the row is self-contained) |
| `⇢9`   | one-way — no return known |
| `↩`    | self-loop — this direction leads back into this very room |
| `⇱out` | leaves the layer; the destination is footnoted below the table |
| `×`    | tried, and there is no path that way |
| `·`    | untried — the exploration frontier |

A move that got you *killed* leaves no `×` behind. Dying says nothing about whether
the passage is open, so the attempt is taken back and the cell stays `·`, still on
the frontier — including when the game asks whether to reincarnate you before it
admits the death, in which case the move that caused it is the one rolled back.

Nor does *getting up again* leave an edge. A death stays outstanding until the game
says how it ends, however many turns of "Please answer yes or no." that takes, and
the next room change on that side of it is read as the resurrection: the map follows
you to wherever you woke up and mints no passage, because wherever a game drops a
resurrected player is not a way out of the room you died in. Adventure's `yes` →
*"--- POOF!! ---"* → the well house is the case that named it. Exactly one such
relocation is swallowed: play resuming — a room description reprinted where you
stand, or the arrival itself — settles the death, and the next passage you walk maps
like any other.

A travel command joins the same family: `GO TO`/`GOTO`/`GO BACK TO`/`RETURN
TO`/`REVISIT`/`WALK TO` a named room (Counterfeit Monkey's "Approaching" action, and
other Inform games built the same way) can walk you through any number of unseen
rooms in one turn, so it mints no passage either — just the relocation to wherever
you ended up, exactly like a death or a teleport. `WALK TO` is not part of
Counterfeit Monkey's own grammar, but is accepted anyway as a common synonym other
Inform and TADS games do declare.

**Reading it.** `▸` marks the room you are standing in. `⇲` marks a room a passage
from *outside* the layer leads into — a doorway into the maze, listed in a footnote
(`⇲ in:  <origin room> —<direction>→ <target>`) alongside where `⇱out` cells lead.
A room that is both here and a doorway shows `▸`: you are standing there, and the
entrance fact still reads in the footnote. Rooms sharing a display name are
numbered in the order you *found* them — "Maze 1" is whichever one you walked
into first, not whichever has the lowest id. That matters because the id is often
the story's own object number, which has nothing to do with when you found the
room: a number is minted the moment a room is first discovered and never changes
again, so finding a "new" duplicate that happens to have a lower id never
renumbers the ones you already know. Rows are in that same order, so a row's
position and its own number always agree. The numbering is otherwise
display-only — identity is still the room's own id — and it is stable across a
reload; a save from before this was tracked settles its numbers, once, to your
true visit order (each room's position in the save file), the first time it
reloads. Names too long for the label column are abbreviated and spelled out in a
footnote.

**Room ids on the map.** A Z-machine room's `#id` is the story's own object
number — small, and worth referencing directly. A Glulx or name-only room has no
such thing: its id is a hash, which used to be shown as `#8000ABCD`-style hex
everywhere the map names a room. It now shows a small per-map ORDINAL instead —
`#1` for the first room you ever discover this session, `#2` the second, and so
on — because the hex is opaque where the ordinal is something you can actually
hold in your head. The two diagnostic surfaces built for tracing a reported
problem (the room card's Diagnostics body, and `/export-map`'s `ROOM` line) show
both forms together (`#12 (8000ABCD)`), so a bug report can still be matched back
to the exact id every other tool — `export-dot`'s node ids included — uses. The
ordinal is a property of the room itself: a room the Glulx lock later re-keys
onto its real object address keeps the number it already had, and a `tidy-map` or
layer move never renumbers anything.

**Selection** moves with ↑/↓ (or Home/End, PageUp/PageDown) when the map pane has
focus, or by clicking a row. Clicking a *destination cell* jumps the selection to
that room's row. Selecting a room **bolds every cell elsewhere that arrives at it**
— its known entrances, which is the answer to "how do I get back here", and the one
question a row cannot answer about itself. That highlight is style, never a glyph:
the table's text does not change.

**Clicking a room also shows you the way there.** lanthorn searches the map for the
shortest route it already knows how to walk, from the room you are standing in to
the one you clicked, and marks **one cell per step: the row of the room you are in,
in the column you leave by**. Read the marks top to bottom and you have walking
instructions — and because it is the *leave-by* cell that lights up, each one keeps
its own glyph, so you can still see whether the step you are about to take comes
back or does not. It wears its own colour (`map.matrix.cell:path`), deliberately
unlike the entrance bolding beside it: the two answer opposite questions, and they
routinely light up in the same row.

The search walks passages only in the direction you walked them, so a one-way
corridor is never offered backwards — a route lanthorn shows you is a route you can
actually walk. It searches the *whole* map rather than just this layer, because a
layer is a way of reading rooms, not a wall between them; steps that land on other
layers simply have no row here to draw on, and where the route walks out of this
layer the `⇱out` cell it leaves by is the one marked (that cell already footnotes
where it goes). The view never jumps layers behind your back. If there is no known
route at all, the room still selects and lanthorn says so rather than falling
silent — a half-route to somewhere nearer would be answering a question you did not
ask.

`Esc` backs out one step at a time: the first press clears the route and leaves the
room selected with its entrances still bold, the next unpins the room, the next
closes the room panel.

**Narrow panes** degrade before they scroll. First the `⇠x` return suffixes drop
(cells shrink to `→5`, and the return is still readable on the destination's own row
and in its room card); only when even that will not fit does the table scroll
sideways, with the label column pinned. The thresholds are computed from the table's
own contents — there is nothing to configure.

The matrix is also the one map view a screen reader can read: a table linearises
where a drawing cannot.

### Marking a maze

`mark-maze-layer` (leader `z`) flags the active layer as a maze. The flag moves the
layer's *default* view to the matrix; it never overrides a `view-map` you chose by
hand, and unflagging puts an unchosen layer straight back to drawn. On a
maze-flagged layer the last few rooms you walked through are also highlighted as a
fading breadcrumb (`map.trail`) — the "how did I get here" a drawn map would have
answered by itself. The flag also puts a `⌗` marker on the layer's tab (`Maze ⌗`) in
both tab strips, and takes it away again when unflagged.

**The flag is always yours to set.** lanthorn never guesses from *statistics* — you
are in the maze long before any measure of tangledness could tell, which is why the
old asymmetry detector fired late, never, or on an ordinary ring of rooms. So the
moment you decide "this is a maze", press `z`; there is nothing to wait for.
What it *will* do is read: walk into a room the game itself calls a **Maze** and the
map offers to take it apart into a layer of its own — see [the map sometimes speaks
first](#getting-around-the-map). Accepting that offer sets this flag too, because you
just confirmed it by accepting a prompt that said so. Accepting a *structural*
suggestion sets nothing: a cellar is not a maze.

### Honest edges on the drawn map

The same asymmetry shows up outside mazes, so the drawn view stopped pretending
too, under one rule: **every arrow on a room border is that room's own exit** —
arrows are only ever outgoing. A two-way corridor wears each room's departure at
its own end; a **one-way** passage wears exactly one, at its origin, and the line
ending *bare* on the destination is the reading — you can get there, and nothing
known brings you back. One-way and
disagreeing-direction edges each have their own style selector (`map.edge:oneway`,
`map.edge:asym`), both defaulting to the ordinary connector so nothing changes
appearance until you choose to style it. A **self-loop** draws as a compact `↩w`
badge on the room box, never as a line looping out and back: a loop has no
geometry, and a drawn one would need its own lane to say less than three characters
do.

**A room whose own name keeps changing** — Lost Pig's gnome tunnels, where every
compass move rerolls what the story calls the room you are standing in — stays one
room on the map rather than a flicker of new boxes. The box shows whatever the
story is calling it right now, with a small superscript count of its other names
beside the label (`Twisty Passage⁵`; `⁹⁺` once there are more than nine), styled
through its own selector (`map.room_alias_marker`) so it can be coloured apart
from the room's own text. The room panel lists every one of them under "Also seen
as", and a compass move that both returns to the room it left AND changes its name
is not drawn as a self-loop at all — it reads `?` in the matrix and on the room
card, the same "destination varies" mark a declared-exit mismatch draws, because a
direction whose destination will not even hold still on a NAME is not honestly a
"leads back here" either.

**A `?` direction remembers where it has actually sent you.** The mark itself only
ever answered "does this direction vary?" — the room card said "destination
varies" and nothing more, however many times you had walked it. Now every distinct
room a `?` direction has actually landed you in gets named: the room card lists
them ("destination varies: Windy Cave, Twisty Passage"), the matrix cell grows a
small superscript count (`?²`), and the box shows it too — but as an ARROWHEAD, not
a `?` sitting on the border (SQ-1275). The border cell carries the direction's own
arrowhead, exactly the glyph a real exit that way would draw, styled through its
own selector (`map.room_random_stub`, defaulting the same colour as the matrix's
`?` cell) so it reads as a mark rather than an ordinary passage; the superscript
count — or a bare `?` when nothing has been recorded yet — sits one cell beyond it,
in the first cell a real connector leaving that side would step into. `diagonal_corners`
plays no part: the router leaves a diagonal exit's corner in the same place either
way (only the LINE ART between corners differs), so a diagonal `?` mark's two cells
never move when you toggle it. Nothing is drawn beyond the count cell: the whole
point of the mark is that there is nowhere stable to draw a line to. The router does
NOT reserve that cell — an unrelated connector elsewhere on the map is free to route
straight through it (SQ-1275 tried reserving it and disqualifying any crossing
candidate route, which cost a real Adventure map its shortest gutter-L route into a
marked room and had to be reverted, SQ-1281); the count is simply painted LAST, after
every connector in the frame, so its digit always wins the shared cell over the
crossing line. `/export-map`'s dump lists the recorded destinations on the `ROOM`
line (`random=[N→(#187 "Probably New Tunnel"), …]`) beside everything else it already
records about a room.

**Several exits to one destination collapse to a single arrowhead** (SQ-1276).
A room whose own name keeps rerolling isn't the only thing that can point at one
place two ways — a staircase alongside a compass passage, or two compass
directions that both happen to lead to the same neighbour, are both ordinary
shapes a real game builds. Drawing every one of them as its own line competed for
the same few border cells and said nothing a single line couldn't; now only the
PRIMARY direction — whichever one's compass bearing actually matches where the
neighbour sits, undistorted, with a fixed tie-break (north, south, east, west,
then the diagonals) on a genuine tie — is routed and drawn, in a REVERSED accent
(`map.room_stacked_exit`, defaulting to the room's own BORDER colour with the
video inverted, not the ordinary connector's — it reads as a bite taken out of
the room's own frame) so it reads as standing in for more than itself. The rest are
suppressed entirely: no line, no border badge, no portal icon. The GRAPH still
carries every direction — the matrix, the room card, `/export-map`'s dump and the
save archive show all of them exactly as before; only the drawing collapses. A
destination reached ONLY by a portal (Up/Down/In/Out), with no compass direction
alongside it, is left alone — there is nothing to prefer a portal over.

**All three superscripts answer a mouse hover, in the drawn view.** Hovering the
alias-count marker pops a tooltip listing the room's other names, in the order
the graph first saw them — the same list the room card's "Also seen as" line
gives. Hovering EITHER cell of a `?` mark — the arrowhead or the count — pops the
direction's recorded destinations, the room itself printed as "back here" exactly
as the room card's exit line does, or "destination varies — none recorded yet"
for a bare `?`. Hovering a stacked primary's arrowhead pops the arrow glyph for
every direction the collapse covers, space-separated on one line, primary first
— no title, no room name, just the glyphs themselves (Up/Down/In/Out draw their
portal icon rather than a compass arrow, the same resolver a room box's own
badges use). Neither hover claims a click; the marker rects
behind them (`render::map::MapHits::marker_rects`) exist only at Boxes zoom, since
Compact and Overview draw no marker to hover.

## Finding the way back, without guessing it

A map built from your moves learns a passage one direction at a time. Walk north
into a clearing and the map knows how you got there and nothing about how you
leave; half the rooms on a map you have walked through once hang off a single
arrow. The tempting fix is to assume passages run both ways, and it is wrong
often enough to be worse than the gap — these games are full of one-way drops,
doors that open from one side, and mazes whose entire design is that the way back
is not the way you came. A guessed arrow is the map asserting something false,
and nothing on screen tells you which arrows were walked and which were assumed.

So the **return probe** finds out instead. After a move that
leaves a gap lanthorn forks the story into a silent throwaway copy — the same
shadow the Guiding Light vets its word suggestions in — stands it exactly where
you are standing, and walks one direction. If it comes out in the room you just
left, that passage is real and joins the map. If it comes out anywhere else,
**nothing at all is recorded**: not the edge, not the room it wandered into, not
that the room exists. The map stays a record of what *you* have seen.

It leads with the way you came and widens from there — the opposite of your move,
then the two directions perpendicular to it, then the two diagonals beside it,
then everything else the compass offers, eight directions in all. Zork I's North
of House is the case worth knowing: south is boarded up and east is somewhere
else entirely, and it is **west** that takes you home. The search walks past the
refusal and past the wrong room to find it, and Behind House — which it really
did walk into — never appears on your map.

Up, down, in and out are asked only as the direct reciprocal of a portal move
you just made — climb down and it asks up, walk in and it asks out — never as a
blind fallback once the compass words are exhausted. A search that did not just
cross a portal has no business finding one you have never explored: on an
ordinary compass map the only way back from some room may genuinely be a
staircase you have not climbed, and drawing that before you have ever gone up
is exactly what the search must not do.

And on a ship — Shogun's fore, aft, port and starboard — the search asks the
way back in your own words, not the compass equivalent. After `fore` it tries
`aft` first, not `south`; both name the same passage, but a game that treats
fore/aft/port/starboard as exits distinct from the compass refuses the compass
word outright and would otherwise make the search wander past several refusals
before settling for whatever it found first, which more than once was a real
passage the wrong distance away — a staircase up, when the honest word for what
you asked was `aft`. Counterfeit Monkey's Atlantida Herself adds the four
quarter directions on top of that same set — after `fore-starboard` the search
tries `aft-port` first, in whatever spelling or abbreviation you typed.

Three things it is careful about:

- **It never marks a direction as tried.** That record is yours, and it is what
  the matrix view's `·` frontier is drawn from. A direction a shadow walked on
  your behalf is still a direction you have never explored, and the map goes on
  offering it.
- **It never claims the reverse.** Climbing through the kitchen window in Zork I
  is still `enter window` on the map, whatever direction brings you back out. The
  return passage is recorded as its own edge, and the geometry that follows from
  it — the kitchen sitting west of Behind House — is the layout's business, not a
  second passage.
- **It gives way to you.** Walk the way back yourself while a search is running
  and the search stops: what you actually did is the better evidence, and it is
  already on the map.

The work happens on a worker thread, so the story answers you at once and the map
catches up a beat later. Every direction it **answers** is remembered
*permanently*, so a room is searched once in the life of a map rather than once
per visit, and a search you interrupt by walking on resumes where it stopped.

An attempt is answered when the shadow comes out somewhere the map can read — a
room it holds, or no room at all, which is what a refusal looks like ("The
windows are all boarded" moves nobody), so a boarded window is remembered and
never asked again. An attempt that comes out in a room the map does **not** hold
is the one exception, and it leaves nothing behind at all: not the edge, not the
room, and not the attempt. That record is consulted forever after, so it has to
state a fact about the world rather than about how much of the map you had drawn
at one moment. "Wherever that goes, you have not
been there yet" is the second kind, and it stops being true the moment you walk
in — after which the direction would have been spent, and the room could never
learn its way back. The cost of asking again is one shadow move on a later
visit, and by then the map may well be able to read the answer.

It showed up as a difference between *directions*, which is the part worth
remembering. A search that does not answer at once spends the cardinals first —
the opposite of your move, the two perpendiculars, then the head of the compass
list — reaches the diagonals only if it gets that far, and never touches up,
down, in or out, which are only ever asked as a portal reciprocal. So a
staircase's way back turned up every time, a diagonal's usually, and a plain
compass exit's not until you walked it yourself. Worse, on Zork I's Behind House
the spent `south` pushed the search onto `southwest`, which also reaches South of
House — so the map drew a *diagonal* where a cardinal was the truth.

It is **on by default** — the probing shares the snapshot the turn already
takes for its own bookkeeping, so it costs the move almost nothing — and the
footprint on the **story** pane's bottom border, immediately inboard of the map
toggle, is its switch: muted when off, lit when on, and never hidden. It
lives there rather than on the map's own border for a reason worth stating: the
search keeps running while the map is hidden — hiding a view must not degrade the
data behind it — so its only switch cannot sit on a pane that disappears. `/set-return-probe` does the same from the keyboard,
and both remember the answer for *that story*, so you can afford it on a small
Z-machine game and decline it on a large Glulx one.

## Making it yours

Every glyph the map draws is a themeable preset in the `[map]` section of
`style.toml`, right alongside the map's colours — swap the whole look without
touching a line of code:

- `box_style` — room outlines: `rounded` (default), `thick`, `double`, `solid`,
  `super-thick`, `ascii`, or `borderless`.
- `arrow_set` — connector arrowheads: `filled` (default), `line`, or a family of
  Nerd Font sets (`nerdfont`, `nf-bold`, `nf-box`, `nf-chevron`, `nf-circle`,
  `nf-outline`, `nf-thick`, `nf-wind`, `nf-thin`) for patched fonts. `nf-thick` is
  the one Material family whose eight heads measure the same. `nf-wind` is the Weather
  Icons circled-arrow set — the one Nerd Font family whose eight directions are
  drawn at one weight from one source (its glyphs are named for the direction the
  wind comes FROM, so each slot takes the glyph named for its opposite); `nf-thin`
  is the same family's bare arrows, the most legible set at a one-cell size.
  `nerdfont` — what the font check installs when you tell it your terminal draws
  row 1 — is the outline set, the same glyphs as `nf-outline` (it was the boxed
  `nf-box` set until 2026-09-03: at a one-cell size a boxed head collapses to a
  square with a dot in it, where an outline head still reads as an arrow). All
  eight directions come from one icon family, diagonals included. `nf-chevron`
  keeps the older bare chevrons for anyone who preferred them.

  **In Ghostty specifically, a Nerd Font arrowhead can draw two cells wide**
  (SQ-1277). Ghostty's `constraintWidth()` (`src/renderer/cell.zig`) lets a
  "symbol-like" glyph — anything in a PUA, or in `isSymbol()`'s own Arrows /
  Dingbats / etc. blocks — spill into the FOLLOWING cell whenever that cell's
  codepoint is `0` or `isSpace()` (which lists only U+0020 SPACE and U+2002 EN
  SPACE) and the preceding cell isn't itself a non-graphics symbol. A room's own
  WEST arrowhead is followed by the box interior's padding space whenever the
  label is shorter than the interior width, so it spilled there; every other
  arrowhead site (`render::map::guard_symbol_spill`) writes U+00A0 NO-BREAK
  SPACE into a plain space immediately to an arrowhead's right, in that cell's
  own style — Ghostty's `isSpace()` does not list U+00A0, so the glyph is
  constrained back to one cell, and a NBSP reads identically to a space
  everywhere else that matters (`char::is_whitespace()`, the gallery capture
  harness, `map_dump`'s cell-copy).
- `path_style` — the line-art that draws the cardinal (N/S/E/W) connectors:
  `light` (default), `heavy`, or `dotted`.
- `portal_path_style` — the same three presets, applied on their own to the
  up/down/in/out portal links: `dotted` (default) keeps the familiar ┊/┄ threads.
- `portal_icons` — up/down/in/out markers: `ascii` (default: `↑ ↓ ◉ ◎`), `nerdfont`,
  or `nerdfont-stairs` for distinct stairway icons. The in/out pair is deliberately
  drawn from Geometric Shapes rather than the `⊙`/`⊗` of Miscellaneous Mathematical
  Operators, which plenty of monospace faces — Fira Code among them — simply do not
  carry; `portal.in` and `portal.out` override either one on its own.

The matrix view has its own selectors beside the map's colours:
`map.matrix.header`, `map.matrix.row:here`, `map.matrix.row:selected`,
`map.matrix.cell:entrance` (the bold cross-highlight), `map.matrix.cell:path` (the
route to the room you clicked), `map.matrix.cell:frontier` (the dimmed `·`/`×`
cells) and `map.matrix.footnote`; `map.trail` colours the maze breadcrumb.

Individual glyphs can be overridden one at a time in `[map.overrides]`, and
`diagonal_corners = false` drops the half-diagonal corner stubs (🮠🮡🮢🮣, Unicode 13
Legacy Computing) in favour of plain orthogonal corner exits — the escape hatch
for a font that has no glyphs for them. Reload changes live with `reload-style`. See
[customization & configuration](customization.md) for the full styling surface, and
[interface](interface.md) for mouse-driven map navigation.

## Generating a reference map

Everything above builds a map out of what a *player* has seen. `lanthorn-mapgen`
answers the other question — what the story file itself *declares* — and it
answers it without playing a turn (SQ-1306). Two things want that: reference
maps to measure the layout engine against, and hundred-room graphs to stress the
router with, neither of which anyone wants to produce by hand.

```sh
cargo run -p lanthorn --bin lanthorn-mapgen -- stories/advent.blb --out /tmp/maps
```

It writes four artefacts named after the story's own stem, and prints a summary:

| artefact | what it is |
|---|---|
| `<stem>.map.txt` | `app::map_dump::render_dump` — the annotated dump, with the ASCII drawing, each layer under its own heading |
| `<stem>.svg` | `app::export_svg::render_svg_layered` — every layer stacked top to bottom under a heading, one shared canvas |
| `<stem>.dot` | `app::export_dot::render_dot` — every layer as its own Graphviz cluster once there is more than one |
| `<stem>.map.json` | the documented, versioned JSON map described below |

Naming any of `--dump`, `--svg`, `--dot`, `--json` writes only the ones named;
naming none writes all four. `--no-layout` skips
`mapper::layout::relayout_auto`, leaving pure topology with no room positions —
much faster on a large map, and the right choice for a consumer doing its own
layout. Exit status is `0` for a map written, `1` for an I/O failure, and **`2`
for a story that declares no map anywhere in the file**, which is a distinct
code so that a script sweeping a shelf can skip rather than stop.

### Layers: mazes and portal-only regions split themselves out (SQ-1308)

Everything used to land on one flat `MAIN_LAYER`. `app::mapgen::split_layers`
now runs between building the graph and laying it out, splitting it the way
the APP's own layer suggestions (`mapper::suggest`) would if a player accepted
every one of them — the same `mapper::layer::move_region` a peel or a merge
in the TUI calls, never a second implementation of what a region is. Two
passes, in order:

1. **Maze layers.** Every room still on Main whose name
   `mapper::suggest::mentions_maze` anchors a walk that stays within
   maze-named rooms — a maze-to-maze compass walk, not
   `mapper::layer::planar_region`'s unrestricted one. That restriction is not
   pedantry: Mini-Zork I's Cyclops Room has an *unconditional* compass exit
   out to the Living Room ("Strange Passage"), so a bare `planar_region` from
   any maze room sweeps up 55 of its 70 rooms — Kitchen and West of House
   included — which is not a maze layer, it is everything but the dozen rooms
   directly behind Troll Room. Even `mapper::layer::region_at_arrival` (the
   live app's own `name_trigger` walk) cannot do better here: probed at all
   three of the maze's real entrances, it is `NotASeam` at two and, at the
   third, excludes only that one entrance room while pulling in the same
   sweep, because the other two entrances are still open. So the maze pass
   stops at the room-name boundary directly. Once a region moves, its layer
   is flagged a maze (`mapper::graph::MapGraph::set_layer_maze`), which
   freezes its layout exactly as the app's own maze flag does. One layer per
   *component* — a twenty-room maze that is all one connected cluster is one
   layer, not twenty.
1b. **Dead ends off a maze (SQ-1311).** The maze walk's own name-boundary
   restriction (above) is deliberate, but it has a cost: a genuine maze exit
   that happens to be named something other than "maze" — a "Dead End", a
   "Grating Room" — gets excluded from the region and left stranded on Main,
   because it never mentions "maze" itself. `absorb_maze_adjacent_rooms` runs
   immediately after every maze region is formed and recovers exactly these:
   a room still on Main joins a maze layer once EVERY compass edge touching
   it — as origin or as destination, `Up`/`Down`/`In`/`Out` are portals and
   never counted — leads to a room already on that ONE maze layer. The
   Cyclops Room protection still holds here: a room with even one compass
   edge to a non-maze room (or to a second maze layer) is left exactly where
   it was, so the restriction that keeps the maze WALK from sweeping in an
   unrelated hub is not weakened, only applied a second time to what the walk
   necessarily left behind. It iterates to a fixed point, because a dead end
   can hang off another dead end that only just got absorbed this round (a
   corridor of them, each one compass step from the last).
2. **Portal-only regions.** What's left of Main is partitioned into
   compass-connected components (`planar_region`, one per unvisited room).
   Mapgen has no start room to anchor a "primary" layer on the way the live
   map anchors on wherever the player began, so the **largest** component is
   kept as Main instead; every other component at or above `--layer-min`
   becomes its own layer, named after the room its entering portal leads
   into — the same anchor a peel names a fresh layer after
   (`MoveTarget::New`'s doc comment). A component under the floor is set
   aside for pass 3 rather than moved.
3. **Below-floor leftovers adopt a neighbour's layer (SQ-1310).** A component
   too small for its own layer does not simply default to Main — the live
   app never has this problem, because a room is discovered on whichever
   layer the player is STANDING on, so a one-room attic reached by climbing
   `Up` from the Kitchen is born on the Kitchen's own layer. Mapgen has no
   player to inherit that context from, so `adopt_stranded_regions` follows
   each below-floor component's own portal edges (`Up`/`Down`/`In`/`Out` —
   `mapper::direction::grid_offset` is `None` for exactly these, same gate
   `mark_distorted` uses) out to whichever neighbouring layer is ALREADY
   settled — Main itself counts, and so does anything pass 1 or 2 just
   created. Several portal neighbours on different layers is resolved by
   whichever layer has the most portal links into the component, ties going
   to the lowest layer id; a component with no portal neighbour at all still
   defaults to Main. This repeats to a fixed point, since one stranded
   one-room dead end can hang off ANOTHER stranded one-room dead end that
   only just got a home. A maze layer is adopted onto only by a component
   whose own room names mention "maze" — a stray dead end that merely opens
   off a maze keeps looking for a non-maze neighbour (or Main) instead,
   mirroring pass 1's own restriction against sweeping in unrelated rooms.

`--layer-min N` sets the portal-only floor; it defaults to
`mapper::suggest::STRUCTURAL_FLOOR` (4) — **the same constant the live
suggestion engine floors a structural region at**, so a static map and a
played one agree about how big a region has to be before it earns a layer of
its own. A maze has no floor: any size gets its own layer once its name says
so. `--no-auto-layers` skips all three passes, reproducing the flat,
single-layer map mapgen wrote before SQ-1308.

Zork I r52/s871125 splits into six layers at the default floor: `Main` (54
rooms — the underground core, once the maze that used to bridge it to the
surface is gone), `Maze` (20, flagged — the ten rooms actually named "Maze"
plus four "Dead End"s and the Grating Room, all absorbed by pass 1b because
every compass edge each one has leads back into the maze), `Rocky Ledge` (21
— the surface world, named for the room its portal from underground opens
onto, plus the Attic and Up a Tree adopted onto it by pass 3; the Grating
Room moved to the maze in pass 1b, before pass 3 ever saw it), `Coal Mine`
(6, plus Ladder Top adopted from pass 3), `Ladder Bottom` (5, including its
OWN "Dead End" at the bottom of the mine shaft — no compass edge to the maze,
so pass 1b leaves it alone) and `Torch Room` (4).

### What each source covers, and what it does not

`lanthorn-mapgen` adds no format knowledge of its own; each source is read by
the module that already owns it. The room-set derivation differs per source and
is worth knowing, because it is the part that can be wrong:

| source | reader | rooms are | doors | conditionals |
|---|---|---|---|---|
| `i7-world` | `gvm::i7map::I7World` | `I7World::rooms()` — the story's own `Map_Storage` row order, so the set is exact and nothing is derived | resolved: a two-sided door becomes an ordinary edge marked `door`, naming the door in `via` | n/a |
| `i6-library` (Glulx) | `gvm::world::WorldModel` | objects that declare an exit, plus every object an exit leads to | **invisible** — `gvm::world` resolves `door_to` and reports a plain room, so the passage is right and the door is lost | n/a |
| `zil` / `i6-library` (Z-machine) | `zvm::world::WorldModel` | the same derivation, over object numbers `1..=max_object` | DEXIT and Inform's `door_to` hop both keep the door (`ExitDetail::Door`) | CEXIT is drawn and marked `conditional` |
| `scott` | `scott::Database` | `db.rooms[1..]` — complete by construction; index 0 is the format's "no room" sentinel and is not a place | n/a | n/a |

The two Inform-6-style derivations deserve a word: neither `zvm::world` nor
`gvm::world` exposes a room list, so "a room is an object that declares an exit"
is the only signal available. The second half — "plus every object an exit leads
to" — is what keeps a room whose *own* exits are all computed from going missing
from its own map. Neither half needs the object tree, which is why the same code
serves ZIL (whose rooms are not parented to a rooms object the way Inform's are).

**One narrow exclusion from the Z-machine derivation (SQ-1311): an unnamed
object whose entire declared exit list leads only to ITSELF.** "Declares an
exit" is enough to pass the derivation above even when that exit is `IN ->
self` — Zork I's object #41 and Mini-Zork's object #27 are both exactly this,
a pseudo-room with no printed name and no way anywhere, compiled into the
object table for some reason the story keeps to itself. `zmachine_map` drops
an object from the room set when its printed name is empty AND every one of
its own declared exits resolves to `None` or to itself; the exclusion is
narrow on purpose — an unnamed room with a real exit ELSEWHERE stays (it may
simply never be printed), and so does one some OTHER object genuinely leads
to, since that is a real destination whatever this object calls itself and
excluding it would leave that edge dangling.

**Three things a static map cannot have.**

- **Runtime map edits.** Inform 7's `AssertMapConnection`, an Inform 6
  `door_dir` holding a routine, and ZIL's FEXIT all decide while the game runs
  and leave nothing in the file. A static map can therefore be missing a passage
  a player would find, and — where a story dismantles a connection — can show
  one a player never can. A headless walker is the answer and is a separate
  quest; nothing here is built toward it.
- **Randomised destinations.** No static source knows a passage is randomised,
  so `EdgeKind::Random` exists in the vocabulary and is never emitted.
- **Stories that declare no map at all.** The Inform 7 reader refuses an Inform 6
  build and any I7 build predating `Map_Storage`; where the Inform 6 reader also
  finds nothing (Kerkerkruip is the specimen — it builds its dungeon as you
  play), there is no source and the tool exits 2.

**Conditional exits are drawn, and this is why `zvm::world::ExitDetail` exists.**
`DeclaredExit` is the answer a live turn wants, and it flattens a ZIL CEXIT to
`Code`: correct for `session::apply_turn`, which must not mint an edge through a
gate that may be shut, and wrong for a map, which loses a real passage. Zork I
r52 has 25 of them, including West of House → Stone Barrow and both ends of the
rainbow. `declared_exit` is now `declared_exit_detail(..).flatten()`, so there is
one description of each compiled shape.

The CEXIT's second byte is reported raw and unattributed. The V3 table calls it
`[global:1]` and it is **not** the global's Z-machine variable number: on Zork I
r52 the seven CEXITs gated on `RAINBOW-FLAG` and on `WON-FLAG` all read `0`, and
two distinct flags cannot be one variable. So the map says "conditional" and
declines to name a global it cannot identify.

### The `.map.json` schema

Version 1. Designed for a consumer that has never seen lanthorn — an offline
map-building tool, a graph analysis, a diff between two releases of a game. It
carries **no lanthorn-internal state**: no seam decisions, no render slots, no
terminal cells.

Two rules for a reader. Refuse a file whose `format` is not `lanthorn-map`, and
**ignore any key or enum value you do not recognise** — the format grows by
addition, and only a change a version-1 reader could not survive bumps
`version`.

- **Top level** — `format` (always `"lanthorn-map"`), `version` (`1`),
  `generator` `{name, version}` (the binary and its full build string),
  `story`, `directions`, `rooms`, `edges`, `layers`.
- **`story`** — `file` (base name only; a reference map is read on other
  machines and an absolute path is noise), `engine` (`z-machine` / `glulx` /
  `scott`), `source` (`i7-world` / `i6-library` / `zil` / `scott`), `release`
  (integer), `serial` (string), `checksum` (lowercase `0x`-prefixed hex
  string), `generated_at` (RFC 3339, UTC).
  - **Z-machine** (ZMSD §11.1): release is the word at `$02`, serial the six
    ASCII digits at `$12..$18`, checksum the word at `$1C` — all three always
    present.
  - **Glulx**: `checksum` is the header's own whole-image checksum (Glulx spec
    §1.4, offset `0x20`) and is always present. `release`/`serial` come from
    the Inform compiler's `Info` block, which sits immediately after the
    36-byte header (Glulx-Inform-Tech.html §1 "Static Data": magic `'Info'` at
    `0x24`, then a memory-layout word, two 4-byte ASCII version strings, a
    release `short` at `0x34`, and a 6-byte serial at `0x36`) — read only when
    that magic matches, so both are `null` for a non-Inform Glulx image rather
    than a guess. Both Inform 6 and Inform 7 builds carry this block; it does
    not depend on the Inform 7 world model.
  - **Scott Adams**: the format has no release, serial or checksum field at
    all, so all three are `null`. (The trailer's adventure number, where
    present, is a title id, not a build identity, and is not reported here.)
- **`directions`** — the direction vocabulary, so a consumer need not hard-code
  it: `word` (what an edge's `dir` says), `short`, and `bearing` in degrees,
  north 0, clockwise. **`bearing` is `null` for up, down, in and out**, which is
  exactly what tells a consumer which directions it can lay out on a grid.
- **`rooms`** — `id` (the display id every lanthorn surface uses: `"#136"` for a
  Z-machine object number, `"#12"` for a synthetic room's ordinal), `raw_id`
  (the numeric `RoomId`), `name`, `ordinal`, `layer`, `pos` `{x, y}`, `flags`,
  and `engine_ref`.
  - **`pos` is the mapper's LOGICAL grid cell** — one unit is one room step, not
    a pixel and not a terminal cell. It is `null` throughout under `--no-layout`.
  - **`engine_ref`** is the engine-native identity, and it is load-bearing: for
    Glulx, `id` is a *hash* of the object address (`roomid::glulx_room_id`) and
    irreversible, so the address has to travel beside it. `{kind: "z-object",
    number}`, `{kind: "glulx-object", address}` (hex, `0x`-prefixed), or
    `{kind: "scott-room", number}`.
- **`edges`** — `from`, `to` (room `id`s), `dir` (canonical lowercase word,
  `"?"` for unknown), `kind`, `reciprocal`, `via`, `note`.
  - **`kind`** is the most specific of `random` > `conditional` > `door` >
    `one-way` > `declared`, so an edge is never labelled twice.
  - **`reciprocal`** is reported independently, because a door or a conditional
    can be one-way too and `kind` has room for only one fact.
  - **`via`** names the door object when `kind` is `door`, else `null`; `note` is
    free text.
- **`layers`** — `id`, `name`, `maze`, `rooms`.

A worked example, trimmed to one room and one edge:

```json
{
  "format": "lanthorn-map",
  "version": 1,
  "generator": { "name": "lanthorn-mapgen", "version": "0.4.3 (32148ae5)" },
  "story": {
    "file": "zork1-invclues-r52-s871125.z5",
    "engine": "z-machine",
    "source": "zil",
    "release": 52,
    "serial": "871125",
    "checksum": "0x4b37",
    "generated_at": "2026-09-05T02:52:34Z"
  },
  "directions": [
    { "word": "north", "short": "n", "bearing": 0 },
    { "word": "up",    "short": "u", "bearing": null }
  ],
  "rooms": [
    {
      "id": "#68",
      "raw_id": 68,
      "name": "West of House",
      "ordinal": 35,
      "layer": 0,
      "pos": { "x": -5, "y": -3 },
      "flags": [],
      "engine_ref": { "kind": "z-object", "number": 68, "address": null }
    }
  ],
  "edges": [
    {
      "from": "#68",
      "to": "#254",
      "dir": "southwest",
      "kind": "conditional",
      "reciprocal": true,
      "via": null,
      "note": "open only while the story allows it (ZIL CEXIT)"
    }
  ],
  "layers": [ { "id": 0, "name": "Main", "maze": false, "rooms": 111 } ]
}
```

The suite is `crates/app/tests/suites/sq1306_mapgen.rs`, in the `mapper_ui`
group binary, and it drives `app::mapgen::generate` rather than shelling out.
Three of the four sources run on CI — `minizork.z3` (ZIL) and
`tiny_cave.dat` (Scott) are tracked fixtures, and `czech.z5` is the negative
case. The two Inform sources have no tracked fixture and skip vacuously without
`stories/`, so **symlink it into a worktree before believing a green run there**.
