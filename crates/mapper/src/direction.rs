#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Direction {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
    Up,
    Down,
    In,
    Out,
    Unknown,
}

pub fn parse_direction(cmd: &str) -> Option<Direction> {
    let lower = cmd.trim().to_lowercase();
    let mut tokens = lower.split_whitespace();
    let first = tokens.next()?;

    let word = if first == "go" {
        tokens.next()?
    } else {
        first
    };

    match word {
        "n" | "north" => Some(Direction::N),
        "s" | "south" => Some(Direction::S),
        "e" | "east" => Some(Direction::E),
        "w" | "west" => Some(Direction::W),
        "ne" | "northeast" => Some(Direction::NE),
        "nw" | "northwest" => Some(Direction::NW),
        "se" | "southeast" => Some(Direction::SE),
        "sw" | "southwest" => Some(Direction::SW),
        "u" | "up" => Some(Direction::Up),
        "d" | "down" => Some(Direction::Down),
        "in" | "inside" | "enter" => Some(Direction::In),
        "out" | "outside" | "exit" => Some(Direction::Out),
        // Ship directions (Seastalker et al.): the bow/front points north.
        "fore" | "forward" | "bow" | "f" => Some(Direction::N),
        "aft" | "stern" | "a" | "af" => Some(Direction::S),
        "port" | "p" => Some(Direction::W),
        "starboard" | "sb" => Some(Direction::E),
        // Quarter directions (Counterfeit Monkey's Atlantida Herself, SQ-1296): the ship's four
        // diagonals, halfway between a cardinal ship direction and its neighbour.
        "aft-port" | "pa" | "ap" => Some(Direction::SW),
        "aft-starboard" | "sa" | "as" | "asb" => Some(Direction::SE),
        "fore-port" | "pf" | "fp" => Some(Direction::NW),
        "fore-starboard" | "sf" | "fs" | "fsb" => Some(Direction::NE),
        _ => None,
    }
}

/// True for the four intercardinal directions (NE/NW/SE/SW).
pub fn is_diagonal(d: Direction) -> bool {
    matches!(d, Direction::NE | Direction::NW | Direction::SE | Direction::SW)
}

/// The directions a room can be asked "have you tried this way?" about (SQ-0391): all eight
/// compass points plus Up and Down.
///
/// In/Out are excluded — they are not part of the rose a player scans, and a game that has them
/// usually has one obvious way in.
pub const UNTRIED_DIRS: [Direction; 10] = [
    Direction::N,
    Direction::NE,
    Direction::E,
    Direction::SE,
    Direction::S,
    Direction::SW,
    Direction::W,
    Direction::NW,
    Direction::Up,
    Direction::Down,
];

/// The directions a RETURN PROBE may try, in the order it falls back to when it has no
/// direction to seed from (SQ-0785): cardinals, then diagonals, then up/down, then in/out.
///
/// All twelve real passages — wider than [`UNTRIED_DIRS`], which deliberately omits In/Out
/// because they are not part of the compass rose a player scans. A probe is not scanning a rose;
/// it is looking for the way back, and `out` is very often exactly that.
///
/// **Not what [`MapGraph::probe_candidates`]'s own fallback step draws from any more** — see
/// [`PROBE_FALLBACK_DIRS`] (SQ-1290). This full list is still what a caller wanting every real
/// direction word wants (`vocab`'s vetting-plan alternates, the return-probe cost harness), so it
/// keeps its four portals; only the return search's OWN blind-fallback step was narrowed.
///
/// [`MapGraph::probe_candidates`]: crate::graph::MapGraph::probe_candidates
pub const PROBE_DIRS: [Direction; 12] = [
    Direction::N,
    Direction::E,
    Direction::S,
    Direction::W,
    Direction::NE,
    Direction::SE,
    Direction::SW,
    Direction::NW,
    Direction::Up,
    Direction::Down,
    Direction::In,
    Direction::Out,
];

/// The directions [`MapGraph::probe_candidates`]'s step 4 — "everything else" — may add once the
/// seeded reciprocal and its bearing-widened neighbours are exhausted (SQ-1290).
///
/// **The eight compass points only — never Up/Down/In/Out.** A portal is asked ONLY as the direct
/// reciprocal of a portal move the player just made (`moved.map(opposite)`, e.g. Down → Up), never
/// as a blind fallback once the compass words run out. A search that has not just crossed a portal
/// has no business revealing one the player has not walked: on an ordinary compass map the only
/// way back from some room may genuinely be `up`, and finding that and drawing it before the
/// player has ever gone up is exactly what this list must not do. [`PROBE_DIRS`] stays the full
/// twelve for callers that want every direction word; only this fallback step was narrowed.
///
/// [`MapGraph::probe_candidates`]: crate::graph::MapGraph::probe_candidates
pub const PROBE_FALLBACK_DIRS: [Direction; 8] = [
    Direction::N,
    Direction::E,
    Direction::S,
    Direction::W,
    Direction::NE,
    Direction::SE,
    Direction::SW,
    Direction::NW,
];

/// The reciprocal WORD for a direction command from a non-compass vocabulary family (SQ-1290):
/// the nautical fore/aft/port/starboard set and its bow/stern synonyms.
///
/// A return probe should ask the way back in the words the PLAYER used. After "fore" the way
/// back is overwhelmingly "aft", not "south" — both parse to the same [`Direction::S`], but a
/// story that models FORE/AFT/PORT/STARBOARD as exits distinct from the compass (Shogun's
/// `defs.zil`: `<DIRECTIONS NORTH ... FORE AFT PORT STARBOARD>`) refuses the compass word and
/// answers only the nautical one.
///
/// `None` for a plain compass word — [`parse_direction`] still resolves it; there is simply no
/// other vocabulary family to prefer — and for anything [`parse_direction`] does not recognise at
/// all. Returns the word alongside the compass [`Direction`] it fills, so a caller can send the
/// word and record the slot it stands for in one step.
pub fn reciprocal_word(cmd: &str) -> Option<(&'static str, Direction)> {
    let lower = cmd.trim().to_lowercase();
    let mut tokens = lower.split_whitespace();
    let first = tokens.next()?;
    let word = if first == "go" { tokens.next()? } else { first };
    match word {
        "fore" | "forward" | "f" => Some(("aft", Direction::S)),
        "bow" => Some(("stern", Direction::S)),
        "aft" | "a" | "af" => Some(("fore", Direction::N)),
        "stern" => Some(("bow", Direction::N)),
        "port" | "p" => Some(("starboard", Direction::E)),
        "starboard" | "sb" => Some(("port", Direction::W)),
        // Quarter directions (Counterfeit Monkey's Atlantida Herself, SQ-1296): each answers with
        // its opposite quarter, the full word the game's own parser accepts.
        "aft-port" | "pa" | "ap" => Some(("fore-starboard", Direction::NE)),
        "aft-starboard" | "sa" | "as" | "asb" => Some(("fore-port", Direction::NW)),
        "fore-port" | "pf" | "fp" => Some(("aft-starboard", Direction::SE)),
        "fore-starboard" | "sf" | "fs" | "fsb" => Some(("aft-port", Direction::SW)),
        _ => None,
    }
}

/// A compass direction's bearing in degrees, north being 0 and east 90.
///
/// `None` for everything that is not on the rose — Up, Down, In, Out and Unknown. That is the
/// point of it: it lets "the two perpendiculars" and "the two adjacent diagonals" be arithmetic
/// (±90°, ±45°) rather than a hand-written table with eight rows and a comment promising it
/// matches [`opposite`].
pub fn bearing(d: Direction) -> Option<u16> {
    Some(match d {
        Direction::N => 0,
        Direction::NE => 45,
        Direction::E => 90,
        Direction::SE => 135,
        Direction::S => 180,
        Direction::SW => 225,
        Direction::W => 270,
        Direction::NW => 315,
        _ => return None,
    })
}

/// The inverse of [`bearing`]: the compass direction at `deg`, or `None` for anything off the
/// eight points.
pub fn from_bearing(deg: u16) -> Option<Direction> {
    Some(match deg % 360 {
        0 => Direction::N,
        45 => Direction::NE,
        90 => Direction::E,
        135 => Direction::SE,
        180 => Direction::S,
        225 => Direction::SW,
        270 => Direction::W,
        315 => Direction::NW,
        _ => return None,
    })
}

/// A direction's short tag, lower case: `n`, `ne`, `u`, `d`, `i`, `o` (SQ-0666). Uppercased it is
/// the matrix view's column header; as-is it is the `⇠w` return suffix inside a cell.
pub fn short_label(d: Direction) -> &'static str {
    match d {
        Direction::N => "n",
        Direction::S => "s",
        Direction::E => "e",
        Direction::W => "w",
        Direction::NE => "ne",
        Direction::NW => "nw",
        Direction::SE => "se",
        Direction::SW => "sw",
        Direction::Up => "u",
        Direction::Down => "d",
        Direction::In => "i",
        Direction::Out => "o",
        Direction::Unknown => "?",
    }
}

/// A direction's full word, lower case: `north`, `northeast`, `up`, `down`, `in`, `out` (SQ-0858).
///
/// **Prose, never a key.** [`short_label`] is load-bearing identity — `SeamKey`'s hand-written
/// `Ord` is built on it, and those keys are persisted in the map file — so changing it silently
/// reorders a `BTreeMap` that a save has already been written against. Anything the player READS
/// spells the direction out here instead, and the two can never be confused for one another.
///
/// [`Direction::Unknown`] is a passage whose direction the map never learned. It has no word, so it
/// gets a placeholder; no prompt reaches it today, because every sentence that names a direction is
/// about a portal ([`is_portal`]) and `Unknown` is not one.
pub fn long_label(d: Direction) -> &'static str {
    match d {
        Direction::N => "north",
        Direction::S => "south",
        Direction::E => "east",
        Direction::W => "west",
        Direction::NE => "northeast",
        Direction::NW => "northwest",
        Direction::SE => "southeast",
        Direction::SW => "southwest",
        Direction::Up => "up",
        Direction::Down => "down",
        Direction::In => "in",
        Direction::Out => "out",
        Direction::Unknown => "some way",
    }
}

/// True for the four passages a layer boundary is made of: Up, Down, In, Out.
///
/// Narrower than "has no [`grid_offset`]", which also catches [`Direction::Unknown`] — a passage
/// whose direction was never learned, not a portal. A region walk may treat both as a cut, but only
/// a real portal is EVIDENCE that somewhere is somewhere else (SQ-0439).
pub fn is_portal(d: Direction) -> bool {
    matches!(d, Direction::Up | Direction::Down | Direction::In | Direction::Out)
}

pub fn grid_offset(d: Direction) -> Option<(i32, i32)> {
    match d {
        Direction::N => Some((0, -1)),
        Direction::S => Some((0, 1)),
        Direction::E => Some((1, 0)),
        Direction::W => Some((-1, 0)),
        Direction::NE => Some((1, -1)),
        Direction::NW => Some((-1, -1)),
        Direction::SE => Some((1, 1)),
        Direction::SW => Some((-1, 1)),
        Direction::Up | Direction::Down | Direction::In | Direction::Out | Direction::Unknown => {
            None
        }
    }
}

/// Layout-only directional offset: like [`grid_offset`], but Up/Down also carry a
/// vertical N/S offset (Up → north, Down → south). Used ONLY by the layout,
/// placement, and directional-scoring code so up/down lay out like N/S. Rendering,
/// routing, layer-cutting, and `mark_distorted` keep using `grid_offset` (which
/// returns None for Up/Down), so up/down still draw as dotted portal stubs and are
/// never marked distorted.
pub fn layout_offset(d: Direction) -> Option<(i32, i32)> {
    match d {
        Direction::Up => Some((0, -1)),
        Direction::Down => Some((0, 1)),
        _ => grid_offset(d),
    }
}

pub fn opposite(d: Direction) -> Direction {
    match d {
        Direction::N => Direction::S,
        Direction::S => Direction::N,
        Direction::E => Direction::W,
        Direction::W => Direction::E,
        Direction::NE => Direction::SW,
        Direction::SW => Direction::NE,
        Direction::NW => Direction::SE,
        Direction::SE => Direction::NW,
        Direction::Up => Direction::Down,
        Direction::Down => Direction::Up,
        Direction::In => Direction::Out,
        Direction::Out => Direction::In,
        Direction::Unknown => Direction::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compass_and_long_forms() {
        assert_eq!(parse_direction("n"), Some(Direction::N));
        assert_eq!(parse_direction("North"), Some(Direction::N));
        assert_eq!(parse_direction("go se"), Some(Direction::SE));
        assert_eq!(parse_direction("enter"), Some(Direction::In));
        assert_eq!(parse_direction("up"), Some(Direction::Up));
        assert_eq!(parse_direction("xyzzy"), None);
        assert_eq!(parse_direction("take lamp"), None);
    }

    #[test]
    fn parses_ship_directions() {
        // Nautical terms map onto compass directions (front of ship = north).
        assert_eq!(parse_direction("fore"), Some(Direction::N));
        assert_eq!(parse_direction("forward"), Some(Direction::N));
        assert_eq!(parse_direction("bow"), Some(Direction::N));
        assert_eq!(parse_direction("aft"), Some(Direction::S));
        assert_eq!(parse_direction("stern"), Some(Direction::S));
        assert_eq!(parse_direction("port"), Some(Direction::W));
        assert_eq!(parse_direction("starboard"), Some(Direction::E));
        // Still works after "go" and case-insensitively.
        assert_eq!(parse_direction("go Starboard"), Some(Direction::E));
    }

    /// Counterfeit Monkey's Atlantida Herself (SQ-1296): the four quarter directions and every
    /// abbreviation "Act V Atlantida Herself.i7x" declares for them, plus the short forms of the
    /// plain four this ship set also uses (`f`, `a`/`af`, `p`, `sb`).
    #[test]
    fn parses_quarter_directions_and_short_abbreviations() {
        assert_eq!(parse_direction("f"), Some(Direction::N));
        assert_eq!(parse_direction("a"), Some(Direction::S));
        assert_eq!(parse_direction("af"), Some(Direction::S));
        assert_eq!(parse_direction("p"), Some(Direction::W));
        assert_eq!(parse_direction("sb"), Some(Direction::E));

        assert_eq!(parse_direction("aft-port"), Some(Direction::SW));
        assert_eq!(parse_direction("pa"), Some(Direction::SW));
        assert_eq!(parse_direction("ap"), Some(Direction::SW));

        assert_eq!(parse_direction("aft-starboard"), Some(Direction::SE));
        assert_eq!(parse_direction("sa"), Some(Direction::SE));
        assert_eq!(parse_direction("as"), Some(Direction::SE));
        assert_eq!(parse_direction("asb"), Some(Direction::SE));

        assert_eq!(parse_direction("fore-port"), Some(Direction::NW));
        assert_eq!(parse_direction("pf"), Some(Direction::NW));
        assert_eq!(parse_direction("fp"), Some(Direction::NW));

        assert_eq!(parse_direction("fore-starboard"), Some(Direction::NE));
        assert_eq!(parse_direction("sf"), Some(Direction::NE));
        assert_eq!(parse_direction("fs"), Some(Direction::NE));
        assert_eq!(parse_direction("fsb"), Some(Direction::NE));

        // Case-insensitive and works after "go", like every other spelling.
        assert_eq!(parse_direction("go FS"), Some(Direction::NE));
        assert_eq!(parse_direction("Aft-Port"), Some(Direction::SW));

        // Still nothing for words this set does not use.
        assert_eq!(parse_direction("xyzzy"), None);
    }

    #[test]
    fn reciprocal_word_answers_only_the_nautical_family() {
        assert_eq!(reciprocal_word("fore"), Some(("aft", Direction::S)));
        assert_eq!(reciprocal_word("forward"), Some(("aft", Direction::S)));
        assert_eq!(reciprocal_word("aft"), Some(("fore", Direction::N)));
        assert_eq!(reciprocal_word("port"), Some(("starboard", Direction::E)));
        assert_eq!(reciprocal_word("starboard"), Some(("port", Direction::W)));
        assert_eq!(reciprocal_word("bow"), Some(("stern", Direction::S)));
        assert_eq!(reciprocal_word("stern"), Some(("bow", Direction::N)));
        // Case-insensitive and works after "go", like `parse_direction`.
        assert_eq!(reciprocal_word("Fore"), Some(("aft", Direction::S)));
        assert_eq!(reciprocal_word("go aft"), Some(("fore", Direction::N)));
        // A plain compass word parses to a Direction just fine, but has no OTHER
        // vocabulary family to prefer, so this yields nothing to prepend.
        assert_eq!(reciprocal_word("north"), None);
        assert_eq!(reciprocal_word("n"), None);
        assert_eq!(reciprocal_word("up"), None);
        assert_eq!(reciprocal_word("enter"), None);
        assert_eq!(reciprocal_word("xyzzy"), None);
    }

    /// Every quarter-direction spelling answers with the FULL opposite word (SQ-1296) — the word
    /// Atlantida Herself's own parser accepts, not the compass equivalent.
    #[test]
    fn reciprocal_word_answers_the_quarter_directions_and_short_forms() {
        assert_eq!(reciprocal_word("f"), Some(("aft", Direction::S)));
        assert_eq!(reciprocal_word("a"), Some(("fore", Direction::N)));
        assert_eq!(reciprocal_word("af"), Some(("fore", Direction::N)));
        assert_eq!(reciprocal_word("p"), Some(("starboard", Direction::E)));
        assert_eq!(reciprocal_word("sb"), Some(("port", Direction::W)));

        assert_eq!(reciprocal_word("aft-port"), Some(("fore-starboard", Direction::NE)));
        assert_eq!(reciprocal_word("pa"), Some(("fore-starboard", Direction::NE)));
        assert_eq!(reciprocal_word("ap"), Some(("fore-starboard", Direction::NE)));

        assert_eq!(reciprocal_word("aft-starboard"), Some(("fore-port", Direction::NW)));
        assert_eq!(reciprocal_word("sa"), Some(("fore-port", Direction::NW)));
        assert_eq!(reciprocal_word("as"), Some(("fore-port", Direction::NW)));
        assert_eq!(reciprocal_word("asb"), Some(("fore-port", Direction::NW)));

        assert_eq!(reciprocal_word("fore-port"), Some(("aft-starboard", Direction::SE)));
        assert_eq!(reciprocal_word("pf"), Some(("aft-starboard", Direction::SE)));
        assert_eq!(reciprocal_word("fp"), Some(("aft-starboard", Direction::SE)));

        assert_eq!(reciprocal_word("fore-starboard"), Some(("aft-port", Direction::SW)));
        assert_eq!(reciprocal_word("sf"), Some(("aft-port", Direction::SW)));
        assert_eq!(reciprocal_word("fs"), Some(("aft-port", Direction::SW)));
        assert_eq!(reciprocal_word("fsb"), Some(("aft-port", Direction::SW)));

        // Every reciprocal word is itself something `parse_direction` accepts, and resolves to
        // the SAME direction reported alongside it — the probe has to be able to type what it
        // is handed back.
        for cmd in [
            "f", "a", "af", "p", "sb", "aft-port", "pa", "ap", "aft-starboard", "sa", "as", "asb",
            "fore-port", "pf", "fp", "fore-starboard", "sf", "fs", "fsb",
        ] {
            let (word, dir) = reciprocal_word(cmd).unwrap();
            assert_eq!(parse_direction(word), Some(dir), "{cmd} -> {word} must parse back to {dir:?}");
        }
    }

    #[test]
    fn offsets_and_opposites() {
        assert_eq!(grid_offset(Direction::N), Some((0, -1)));
        assert_eq!(grid_offset(Direction::SE), Some((1, 1)));
        assert_eq!(grid_offset(Direction::Up), None);
        assert_eq!(opposite(Direction::N), Direction::S);
        assert_eq!(opposite(Direction::NE), Direction::SW);
        assert_eq!(opposite(Direction::In), Direction::Out);
    }

    /// The full set of directions, so a table test cannot quietly miss the one that was added.
    const ALL: [Direction; 13] = [
        Direction::N,
        Direction::S,
        Direction::E,
        Direction::W,
        Direction::NE,
        Direction::NW,
        Direction::SE,
        Direction::SW,
        Direction::Up,
        Direction::Down,
        Direction::In,
        Direction::Out,
        Direction::Unknown,
    ];

    /// `short_label` is a KEY, not a label: `SeamKey`'s `Ord` is built on it and those keys are
    /// persisted, so this pins the exact table. Changing any entry reorders a `BTreeMap` that
    /// existing map files were written against — which is why SQ-0858 added a second function
    /// rather than spelling this one out.
    #[test]
    fn short_label_is_the_pinned_ordering_key() {
        let table: Vec<&str> = ALL.iter().map(|&d| short_label(d)).collect();
        assert_eq!(table, ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d", "i", "o", "?"]);
        let mut sorted = table.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), table.len(), "distinct for all thirteen, or SeamKey's Ord collides");
    }

    /// …and `long_label` is the one the player reads: a real word for every direction, never a tag.
    #[test]
    fn long_label_spells_every_direction_out() {
        assert_eq!(long_label(Direction::Down), "down");
        assert_eq!(long_label(Direction::Up), "up");
        assert_eq!(long_label(Direction::NE), "northeast");
        assert_eq!(long_label(Direction::In), "in");
        assert_eq!(long_label(Direction::Out), "out");
        let mut words: Vec<&str> = Vec::new();
        for d in ALL {
            let long = long_label(d);
            assert_ne!(long, short_label(d), "{d:?} must be spelled out, not handed back its key");
            assert!(
                long.chars().all(|c| c.is_ascii_lowercase() || c == ' '),
                "{d:?} is lower-case prose so a caller can case it as it likes: {long:?}"
            );
            if d != Direction::Unknown {
                // `Unknown` is the one with no word of its own, so it is the one exempt.
                assert_eq!(
                    long.chars().next(),
                    short_label(d).chars().next(),
                    "{d:?}: the word and the tag disagree about which direction this is ({long:?})"
                );
            }
            words.push(long);
        }
        let mut sorted = words.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), words.len(), "no two directions read the same: {words:?}");
    }

    #[test]
    fn is_diagonal_only_for_intercardinals() {
        assert!(is_diagonal(Direction::NE));
        assert!(is_diagonal(Direction::NW));
        assert!(is_diagonal(Direction::SE));
        assert!(is_diagonal(Direction::SW));
        assert!(!is_diagonal(Direction::N));
        assert!(!is_diagonal(Direction::E));
        assert!(!is_diagonal(Direction::Up));
    }

    #[test]
    fn layout_offset_maps_updown_to_ns_but_grid_offset_stays_none() {
        use super::{grid_offset, layout_offset, Direction};
        // Up/Down get an N/S layout offset...
        assert_eq!(layout_offset(Direction::Up), Some((0, -1)));
        assert_eq!(layout_offset(Direction::Down), Some((0, 1)));
        // ...but grid_offset is untouched (still None) — rendering/layers rely on this.
        assert_eq!(grid_offset(Direction::Up), None);
        assert_eq!(grid_offset(Direction::Down), None);
        // Compass delegates to grid_offset.
        assert_eq!(layout_offset(Direction::N), grid_offset(Direction::N));
        assert_eq!(layout_offset(Direction::E), grid_offset(Direction::E));
        // In/Out/Unknown remain None.
        assert_eq!(layout_offset(Direction::In), None);
        assert_eq!(layout_offset(Direction::Unknown), None);
    }
}
