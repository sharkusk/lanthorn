// /// Configurable map symbols for the lanthorn renderer.
// ///
// /// All glyphs the map renderer uses are centralized here. The defaults reproduce
// /// today's hardcoded literals exactly, so an absent `[symbols]` config changes nothing.

// ── Sub-structs ───────────────────────────────────────────────────────────────

/// Six glyphs that form one room-outline style (box-drawing corners + lines).
/// Tuple field order: (tl, tr, bl, br, h, v).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxStyle {
    pub tl: char,
    pub tr: char,
    pub bl: char,
    pub br: char,
    pub h: char,
    pub v: char,
}

/// Cardinal + diagonal arrow glyphs for connector arrowheads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrows {
    pub north: char,
    pub south: char,
    pub east: char,
    pub west: char,
    pub ne: char,
    pub nw: char,
    pub se: char,
    pub sw: char,
}

/// Box-drawing glyphs for the path line-art table (what glyph_for returns per mask).
/// Field names match the direction-bit combinations: ew=east-west straight, ns=north-south,
/// se=southeast corner (coming from south, going east), etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathGlyphs {
    pub ew: char,
    pub ns: char,
    pub se: char,
    pub sw: char,
    pub ne: char,
    pub nw: char,
    pub nse: char,
    pub nsw: char,
    pub ews: char,
    pub ewn: char,
    pub nesw: char,
    // ── Diagonal corner-exit stubs (SQ-0314) ─────────────────────────────────
    // Half-diagonals from the Legacy Computing block, named for their two
    // endpoints (matching their Unicode names). Unlike ╱/╲ (U+2571/2572), which
    // run corner-to-corner, EVERY endpoint here is an edge MIDPOINT — the same
    // points ─ attaches at (middle left/right) and │ attaches at (upper/lower
    // centre). That is what lets a diagonal stub hand off to an orthogonal path
    // cleanly. Used only when `diagonal_corners` is on.
    /// Upper-centre ↔ middle-left (U+1FBA0 🮠).
    pub diag_ul: char,
    /// Upper-centre ↔ middle-right (U+1FBA1 🮡).
    pub diag_ur: char,
    /// Middle-left ↔ lower-centre (U+1FBA2 🮢).
    pub diag_ll: char,
    /// Middle-right ↔ lower-centre (U+1FBA3 🮣).
    pub diag_lr: char,
}

/// The two cells a tooltip's pointer is drawn from, per direction (SQ-1139).
///
/// **Always two cells, in both presets.** A one-cell pointer sounds tidier and
/// is worse on both counts available: `▲` (U+25B2) is an *inset* outline — 62..538
/// of a 0..600 cell in SauceCodePro NFM — so it floats visibly clear of the box
/// it is meant to be part of, and no single glyph in a guaranteed range makes an
/// apex that meets a flat edge. Two cells buys a real wedge and keeps the
/// geometry one shape for both presets.
///
/// The pointer is drawn in the tooltip's BACKGROUND colour, so it reads as the
/// box growing a spur rather than as a character sitting next to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TipGlyphs {
    /// Pointing UP — the box is BELOW the anchor, and these meet its top edge.
    pub up_left: char,
    pub up_right: char,
    /// Pointing DOWN — the box is ABOVE the anchor, meeting its bottom edge.
    pub down_left: char,
    pub down_right: char,
}

impl TipGlyphs {
    /// The two glyphs for a wedge on `side`, left cell first.
    pub fn wedge(&self, up: bool) -> (char, char) {
        if up {
            (self.up_left, self.up_right)
        } else {
            (self.down_left, self.down_right)
        }
    }

    /// Return a named preset, or `None` for an unknown name. Shares
    /// [`ControlGlyphs`]'s names, because it shares its `control_icons` key.
    ///
    /// - "plain"    — half blocks (U+2584/2580). Not an arrow: a flat tab. But it
    ///   is flush with the box by construction and present in every font measured
    ///   (43 of 43 installed faces on the machine this was chosen on), which is
    ///   what the unpatched answer has to be.
    /// - "nerdfont" — the Powerline Extra corner triangles, which make a real
    ///   wedge with an apex.
    ///
    /// **Every codepoint below was read from the font's own `cmap` and `post`
    /// tables** (SQ-1045's rule, and SQ-1141's method). The obvious choice —
    /// quadrant triangles U+25E2/25E3 — is NOT used and cannot be: they are BASE
    /// typeface coverage, absent from SauceCodePro NFM, JetBrains, 0xProto,
    /// ProggyClean **and from Symbols Nerd Font Mono**, so patching guarantees
    /// nothing. Only 3 of 9 installed Nerd Fonts had them. The Powerline Extra
    /// set is part of the patch and was in all nine.
    ///
    /// And these overlap the cell on purpose: `ple-lower_right_triangle` spans
    /// xMin 1 to xMax 630 against a 600 advance, its neighbour -30 to 599. That
    /// 40-unit bleed each way is why the two halves meet with no hairline down
    /// the middle, which a pointer colour-matched to its box cannot hide.
    pub fn preset(name: &str) -> Option<TipGlyphs> {
        Some(match name {
            "plain" => SymbolSet::default().tip,
            "nerdfont" => TipGlyphs {
                // ple-lower_right_triangle + ple-lower_left_triangle: each has a
                // full bottom edge meeting the box, rising to the shared apex.
                up_left: '\u{E0BA}',
                up_right: '\u{E0B8}',
                // ple-upper_right_triangle + ple-upper_left_triangle, mirrored:
                // full TOP edge, falling to the apex below.
                down_left: '\u{E0BE}',
                down_right: '\u{E0BC}',
            },
            _ => return None,
        })
    }
}

/// Portal icon glyphs: directional markers + connector path char.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalGlyphs {
    /// Marker drawn in the notes/icon column for a room with notes (●).
    pub marker: char,
    /// Dotted vertical connector for Up/Down portal links (┊).
    pub path: char,
    /// Dotted horizontal connector for Up/Down portal links (┄).
    pub path_h: char,
    /// Up portal icon (↑).
    pub up: char,
    /// Down portal icon (↓).
    pub down: char,
    /// In portal icon (◉).
    pub in_: char,
    /// Out portal icon (◎).
    pub out: char,
    /// Unknown portal icon (?).
    pub unknown: char,
}

/// The glyphs on the pane border's clickable toggle controls (SQ-1123).
///
/// Every slot is a STATE, not a control: a toggle draws one of two glyphs
/// depending on which way it would move things, so the icon says what is on
/// before the colour does. The panel toggles are arrows pointing the way the
/// panel would go — the map lives to the right of the story pane and the verb
/// panel below it, so `map_hide` points right (click and the map leaves that
/// way) and `band_show` points up (click and the band rises into view).
///
/// Defaults come from Geometric Shapes (U+25xx) for the same reason
/// [`PortalGlyphs`]' do: it is the block an ordinary monospace face already has
/// to carry for the map's ● ▲ ▼ ◀ ▶, so the controls draw on a stock terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlGlyphs {
    /// Map hidden — click and it slides in from the right (◀).
    pub map_show: char,
    /// Map shown — click and it leaves to the right (▶).
    pub map_hide: char,
    /// Command band closed — click and it rises from the bottom (▲).
    pub band_show: char,
    /// Command band open — click and it drops back down (▼).
    pub band_hide: char,
    /// Lanthorn's Guiding Light is on (●; the lamp itself in a patched font).
    pub guidance_on: char,
    /// The Guiding Light is off (○).
    pub guidance_off: char,
    /// v6 render mode `hybrid` — half text, half art (◧).
    pub render_hybrid: char,
    /// v6 render mode `raster` — the whole frame is a picture (■).
    pub render_raster: char,
    /// v6 render mode `extended` (▦).
    pub render_extended: char,
    /// v6 pixel lock engaged — art pinned to whole device pixels (▣).
    pub lock_on: char,
    /// v6 pixel lock off (□).
    pub lock_off: char,
    /// The return probe (SQ-0785) — a footprint, in one state rather than two.
    ///
    /// **One of two controls here with a single glyph, and deliberately.** Most
    /// of them name two modes and draw the mode they are in: shown/hidden,
    /// open/closed, locked/unlocked. This one has no opposite mode — it is either
    /// looking for the way back or it is not — and it is also the only SWITCH
    /// whose off state is the DEFAULT, which is the state a player has to notice
    /// in order to ever turn it on. So the mark is always the same and the colour
    /// carries the state: muted when off, lit through `panel.control:lit` when
    /// on. A shape that changed here would be saying "the other mode is engaged"
    /// about a thing with no other mode.
    pub return_probe: char,
    /// The momentary word reveal (SQ-1107) — a light, in one state.
    ///
    /// **The only TRIGGER in the set.** Every other control reports a state and
    /// flips it; this one has no state at all — it makes something happen
    /// elsewhere on the screen and is over a few seconds later. So one glyph,
    /// like the probe above it, lit for exactly as long as the reveal is up. That
    /// lighting is not a state report: it is so that a click visibly DID
    /// something, because a click that happens to light no words would otherwise
    /// look broken. The tooltip carries the rest, and carries more weight here
    /// than on its neighbours, since the glyph alone cannot say what a press does.
    pub reveal: char,
}

/// The story picker's row badges (SQ-0559), as one preset rather than six loose
/// keys (SQ-1159).
///
/// **Why a preset at all.** These were free-text `[elements]` keys with letter
/// defaults and nothing behind them, so the font check — which sets
/// `arrow_set`, `portal_icons` and `control_icons` from one answer — could not
/// reach them. A player who said "yes, my font is patched" got patched glyphs
/// everywhere except here. Loose keys are what made that possible; one name
/// that resolves to the whole set is what stops it happening again, and it
/// leaves the per-badge keys working as overrides on top.
///
/// **Three badges, and every one of them is an ARTIFACT** — a save, a hint you
/// have, a hint you could fetch — so each is a picture of that thing. The set
/// was six until SQ-1160: the story TYPE and the Blorb were badges once too,
/// and SQ-0369 moved both into the picker's TYPE column as text
/// (`Z5 (blorb)`), leaving three keys themeable and drawn nowhere. They are
/// gone rather than redrawn — the column is the shipped answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoryBadges {
    /// The story has a save.
    pub save: char,
    /// Hints for the story are installed.
    pub hint: char,
    /// Hints for the story exist but have not been downloaded — the weaker
    /// claim, so the weaker mark (`h` against `H` in the plain set).
    pub hint_available: char,
}

// ── Top-level set ─────────────────────────────────────────────────────────────

/// Glyphs for the MAP pane's own control cluster (SQ-1148) — five slots on its
/// border, the way [`ControlGlyphs`] is the story pane's.
///
/// Resolved from the same `control_icons` preset as [`ControlGlyphs`] and
/// [`TipGlyphs`], not a key of its own, for the reason given on
/// [`SymbolSet::tip`]: a patched font draws all of them or none of them, and one
/// question has already settled which.
///
/// **Two of the five controls are two-mode, and each keeps BOTH of its slots —
/// but only the patched preset can fill them with different marks.** The house
/// rule is that a two-mode control changes SHAPE between its states, the way
/// every one in [`ControlGlyphs`] does (`●`/`○`, `▣`/`□`, `◧`/`■`/`▦`); the two
/// single-glyph controls there (`return_probe`, `reveal`) are single only
/// because they have no opposite mode to draw. Room numbers and the view switch
/// both DO have one, so both keep a pair of fields, and the patched preset obeys
/// the rule outright: `md-numeric`/`md-numeric_off` and `md-grid`/`md-grid_off`.
///
/// **The plain preset spells the same mark in both slots — `#`/`#` and `M`/`M` —
/// and lets COLOUR carry the state instead.** That is a degradation forced by
/// the plain set's vocabulary, not a second house pattern: ASCII has no
/// off-shape for a `#` or an `M`, and Geometric Shapes cannot supply one that is
/// better covered than the on-shape. Measured over sixteen text faces, every
/// plain mark that says "number" by shape reaches at most 14/16, the `╬`/`┼`
/// pair first chosen for the view switch was the weakest mark in the whole
/// cluster at 13/16 and the only one that actually fell back in a surveyed face
/// (Monaco), and what is left untaken inside Geometric Shapes is the
/// worst-covered run in the survey at 5/16 to 9/16. `#` and `M` are ASCII, so
/// they are drawable in every face BY CONSTRUCTION, and each is the letter of
/// the thing it names.
///
/// So each preset does the best its own font allows, which is why the pair of
/// fields survives even where one preset fills it twice over. A player whose
/// terminal draws neither preset well can still set either half by hand —
/// `[symbols.overrides] map_control.view_drawn = "…"` — the same way
/// `badge_icons` chooses a set and a hand-set `badge_*` key still wins for its
/// own slot. **Do not "tidy away" the duplicate**; it is the override surface.
///
/// What makes the degraded plain path survivable is that it is not colour
/// ALONE: `panel.control:lit` is the `alert` role PLUS BOLD, so a colour-blind
/// player or a low-contrast theme still reads a WEIGHT change, and the default
/// pair (`muted` DarkGray against `alert` Yellow) separates by brightness rather
/// than by hue. `border_controls`'s
/// `every_on_state_is_lit_from_the_alert_role_and_every_off_state_is_muted`
/// asserts that BOLD — **dropping that assertion as redundant beside the colour
/// check is exactly what would make the plain cluster unreadable**, since colour
/// is the only other channel these two marks have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapControlGlyphs {
    /// Room numbers shown. The plain preset draws the same `#` in both states
    /// and lets colour say which; the patched preset changes shape. See above.
    pub room_numbers_on: char,
    /// Room numbers hidden.
    pub room_numbers_off: char,
    /// Recentre the map on the current room.
    pub centre: char,
    pub zoom_out: char,
    pub zoom_in: char,
    /// `MapView::Matrix`. Plain draws `M`; patched draws a lattice.
    pub view_matrix: char,
    /// The drawn map. Plain draws the same `M`, muted; patched draws the
    /// lattice struck through.
    pub view_drawn: char,
}

/// All map glyphs used by the renderer, resolved from config at startup.
///
/// `Default` returns the exact set of glyphs that were hardcoded before this
/// abstraction was introduced — back-compat is guaranteed by that contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolSet {
    pub room_normal: BoxStyle,
    pub room_current: BoxStyle,
    pub room_portal: BoxStyle,
    /// Selected room outline. Defaults to normal (selection is color-only today).
    pub room_selected: BoxStyle,
    pub arrows: Arrows,
    pub path: PathGlyphs,
    pub portal: PortalGlyphs,
    /// Glyphs for the pane border's clickable toggle controls (SQ-1123).
    pub controls: ControlGlyphs,
    /// The tooltip's pointer — the spur that aims a hint at the icon it explains
    /// (SQ-1139). Chrome, like [`Self::controls`], and resolved from the same
    /// `control_icons` preset rather than a key of its own: a patched font draws
    /// both or neither, and one question already settled which.
    pub tip: TipGlyphs,
    /// Glyphs for the MAP pane's own control cluster (SQ-1148). Shares
    /// `control_icons` with [`Self::controls`] and [`Self::tip`].
    pub map_controls: MapControlGlyphs,
    /// Gutter marker glyph for META transcript lines.
    pub meta_gutter: char,
    /// Gutter marker glyph for WARNING transcript lines.
    pub warning_gutter: char,
    /// The mark of Lanthorn's Guiding Light, drawn in the gutter of every ASSIST
    /// transcript line (SQ-1045). It is not a bar beside an icon — it **is** the
    /// icon, and the only thing that identifies an assist on screen, since the
    /// lines themselves carry no marker in their text.
    ///
    /// `●` (U+25CF) by default, chosen by scanning the cmaps of eight text faces
    /// on a working machine: every glyph that actually *depicts* a light misses
    /// too many of them (`☼` 4/8, `★` 2/8, the dingbat stars 2/8), while the
    /// filled circle reaches 6/8. It is a mark, not a picture, and a mark that
    /// draws everywhere beats a lamp that draws in three fonts.
    ///
    /// A patched font has the lamp itself: set `[symbols.overrides]
    /// "gutter.assist" = "\u{F1A60}"` — Nerd Fonts' `md-post_lamp`, verified
    /// against the font's own `post` table rather than a cheat sheet — which
    /// reaches the same 6/8, missing only the unpatched system faces. SQ-1104
    /// will pick it automatically when a first-run font check can see it.
    ///
    /// **Not `*`**: Infocom games spend asterisks on footnotes, and a footnote
    /// marker in the margin of an interpreter's own line is exactly the
    /// impersonation this register exists to avoid.
    pub assist_gutter: char,
    /// Header marker for the room dock while it FOLLOWS the player.
    ///
    /// Hollow against [`Self::dock_pinned`]'s filled, the same reading the portal
    /// icons use: hollow is the moving state, filled is the fixed one. Both were
    /// hard-coded in `render::room_dock` until SQ-0989's follow-up, as `U+2316`
    /// POSITION INDICATOR and `U+2299` CIRCLED DOT — the second being the very
    /// glyph that quest removed from the map for being undrawable, and neither is
    /// in Fira Code (`fc-list ":charset=2316"` and `":charset=2299"` match no
    /// FiraCode face; `25C6`/`25C7` match 13 each).
    pub dock_following: char,
    /// Header marker for the room dock while it is PINNED to a selected room.
    ///
    /// A BMP glyph, not the design sketch's emoji: an emoji is double-width in a
    /// cell grid and the header is drawn cell-by-cell like every other line there.
    pub dock_pinned: char,
    /// Draw ne/nw/se/sw connectors as a chain of half-diagonals out of the room corner, using the
    /// `path.diag_*` glyphs (SQ-0314). On by default.
    ///
    /// Turn it off for a terminal/font without Unicode 13 Legacy Computing coverage: the connector
    /// still leaves and arrives on the same CORNERS — that part is the router's doing, not this
    /// setting's — but walks between them orthogonally instead.
    pub diagonal_corners: bool,
}

impl Default for SymbolSet {
    fn default() -> Self {
        let room_normal = BoxStyle { tl: '╭', tr: '╮', bl: '╰', br: '╯', h: '─', v: '│' };
        Self {
            room_normal,
            room_current: BoxStyle { tl: '┏', tr: '┓', bl: '┗', br: '┛', h: '━', v: '┃' },
            room_portal: BoxStyle { tl: '╔', tr: '╗', bl: '╚', br: '╝', h: '═', v: '║' },
            room_selected: room_normal, // color-only selection today
            arrows: Arrows {
                north: '▲',
                south: '▼',
                east: '▶',
                west: '◀',
                ne: '↗',
                nw: '↖',
                se: '↘',
                sw: '↙',
            },
            path: PathGlyphs {
                ew: '─',
                ns: '│',
                se: '┌',
                sw: '┐',
                ne: '└',
                nw: '┘',
                nse: '├',
                nsw: '┤',
                ews: '┬',
                ewn: '┴',
                nesw: '┼',
                diag_ul: '🮠',
                diag_ur: '🮡',
                diag_ll: '🮢',
                diag_lr: '🮣',
            },
            portal: PortalGlyphs {
                marker: '●',
                path: '┊',
                path_h: '┄',
                up: '↑',
                down: '↓',
                // ◉/◎, not ⊙/⊗ — see `PortalGlyphs::preset`'s "ascii" arm.
                in_: '◉',
                out: '◎',
                unknown: '?',
            },
            controls: ControlGlyphs {
                map_show: '◀',
                map_hide: '▶',
                band_show: '▲',
                band_hide: '▼',
                guidance_on: '●',
                guidance_off: '○',
                render_hybrid: '◧',
                render_raster: '■',
                render_extended: '▦',
                lock_on: '▣',
                lock_off: '□',
                // ◌ (U+25CC), the only mark in Geometric Shapes that reads as a
                // TRACE rather than as a state — the print left by something that
                // walked through and is not there any more, which is exactly what
                // the shadow leaves behind. Everything else in the block is a
                // filled/hollow pair saying which of two modes is in force.
                return_probe: '◌',
                // ◈ (U+25C8), chosen the way `return_probe` chose ◌: the one mark
                // in Geometric Shapes that reads as a light SOURCE rather than as
                // a state — a bright point inside its own halo, which is what a
                // reveal casts over the prose. Deliberately not another circle:
                // the Guiding Light sits one column away as ●/○, and a third disc
                // beside that pair would read as a third state of the same lamp.
                reveal: '◈',
            },
            // The unpatched pointer: a flat two-cell tab in half blocks. Flush
            // with the box by construction, and in every font (SQ-1139).
            tip: TipGlyphs {
                up_left: '▄',
                up_right: '▄',
                down_left: '▀',
                down_right: '▀',
            },
            map_controls: MapControlGlyphs {
                // `#` in BOTH slots; colour delineates on from off, because
                // ASCII has no off-shape for it. See the type's note.
                room_numbers_on: '#',
                room_numbers_off: '#',
                // ¤ is a reticle — a ring with four spokes — which is what
                // "centre the map here" looks like, and 15/16 faces draw it.
                centre: '¤',
                // The true MINUS SIGN, not a hyphen: it sits at the same optical
                // height as the `+` beside it, which a hyphen does not.
                zoom_out: '−',
                zoom_in: '+',
                // `M` in BOTH slots, the letter of the mode it announces:
                // lit for MATRIX, muted for the drawn map. It replaced a ╬/┼
                // pair that was a real shape change and the weakest-covered
                // mark in the cluster — 13/16 faces, and the only one that fell
                // back in a surveyed face. ASCII offers no off-shape for it, so
                // colour carries the state here exactly as it does for `#`.
                view_matrix: 'M',
                view_drawn: 'M',
            },
            dock_following: '◇',
            dock_pinned: '◆',
            meta_gutter: '▏',
            assist_gutter: '●',
            warning_gutter: '!',
            diagonal_corners: true,
        }
    }
}

// ── Presets ───────────────────────────────────────────────────────────────────

impl BoxStyle {
    /// All known preset names for BoxStyle, in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["rounded", "thick", "double", "solid", "super-thick", "ascii", "borderless"]
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "rounded"    — rounded corners (default, matches `SymbolSet::default().room_normal`)
    /// - "thick"      — heavy box-drawing (matches `room_current`)
    /// - "double"     — double-line box-drawing (matches `room_portal`)
    /// - "solid"      — full-block walls: every edge/corner is `█` (single-width)
    /// - "super-thick" — full-block edges `█` with quadrant-block corners `▛▜▙▟`
    ///   (heavy block frame with beveled inner corners)
    /// - "ascii"      — ASCII-only: corners `+`, horizontal `-`, vertical `|`
    /// - "borderless" — all spaces (invisible walls)
    pub fn preset(name: &str) -> Option<BoxStyle> {
        Some(match name {
            "rounded" => BoxStyle { tl: '╭', tr: '╮', bl: '╰', br: '╯', h: '─', v: '│' },
            "thick" => BoxStyle { tl: '┏', tr: '┓', bl: '┗', br: '┛', h: '━', v: '┃' },
            "double" => BoxStyle { tl: '╔', tr: '╗', bl: '╚', br: '╝', h: '═', v: '║' },
            "solid" => BoxStyle { tl: '█', tr: '█', bl: '█', br: '█', h: '█', v: '█' },
            "super-thick" => BoxStyle { tl: '▛', tr: '▜', bl: '▙', br: '▟', h: '█', v: '█' },
            "ascii" => BoxStyle { tl: '+', tr: '+', bl: '+', br: '+', h: '-', v: '|' },
            "borderless" => BoxStyle { tl: ' ', tr: ' ', bl: ' ', br: ' ', h: ' ', v: ' ' },
            _ => return None,
        })
    }
}

impl Arrows {
    /// All known preset names for Arrows, in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["filled", "line", "nerdfont", "nf-bold", "nf-box", "nf-chevron", "nf-circle", "nf-outline"]
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "filled"     — filled triangle glyphs ▲▼▶◀ + diagonal arrows ↗↖↘↙ (default)
    /// - "line"       — thin Unicode arrows ↑↓→← + diagonal ↗↖↘↙
    /// - "nerdfont"   — MDI bold-box arrows, the same set as "nf-box" (requires a
    ///   patched font). This is what the font check installs, so it is the set
    ///   most players see. It is boxed rather than bare because a connector
    ///   arrowhead sits ON a line of path glyphs: a box gives the head an edge of
    ///   its own, where a chevron reads as one more bend in the path.
    ///   Diagonal: native MDI bold-box diagonals — one family for all eight,
    ///   which is the same rule `ControlGlyphs` states for its own pairs.
    /// - "nf-chevron" — the MDI chevrons "nerdfont" used to be
    ///   (U+F0143/F0140/F0142/F0141), kept reachable by name so a player who
    ///   preferred them keeps them with one line of config.
    ///   Diagonal: same as "line" (↗↖↘↙)
    /// - "nf-bold"    — MDI arrow-{up,down,left,right}-bold (F0737/F072E/F0731/F0734)
    ///   Diagonal: Unicode fallback ↖↗↙↘ (no native MDI bold diagonals)
    /// - "nf-box"     — MDI arrow-{up,down,left,right}-bold-box (F0738/F072F/F0732/F0735)
    ///   Diagonal: native MDI bold-box diagonals (F1968/F196A/F1964/F1966)
    /// - "nf-circle"  — MDI arrow-{up,down,left,right}-bold-circle (F005F/F0047/F004F/F0056)
    ///   Diagonal: Unicode fallback ↖↗↙↘ (no native MDI circle diagonals)
    /// - "nf-outline" — MDI arrow-{up,down,left,right}-bold-outline (F09C7/F09BF/F09C0/F09C2)
    ///   Diagonal: native MDI bold-outline diagonals (F09C3/F09C5/F09B7/F09B9)
    pub fn preset(name: &str) -> Option<Arrows> {
        Some(match name {
            "filled" => Arrows {
                north: '▲', south: '▼', east: '▶', west: '◀',
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            "line" => Arrows {
                north: '↑', south: '↓', east: '→', west: '←',
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            "nf-chevron" => Arrows {
                // MDI chevron glyphs (single-width in patched fonts):
                // chevron-up F0143, chevron-down F0140, chevron-right F0142, chevron-left F0141.
                north: '\u{F0143}', south: '\u{F0140}',
                east: '\u{F0142}', west: '\u{F0141}',
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            "nf-bold" => Arrows {
                // MDI arrow-up-bold F0737, arrow-down-bold F072E,
                // arrow-left-bold F0731, arrow-right-bold F0734
                north: '\u{F0737}', south: '\u{F072E}',
                east: '\u{F0734}', west: '\u{F0731}',
                // No native MDI plain-bold diagonal arrows; use Unicode fallback
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            // One arm, two names: "nerdfont" is what the font check writes and
            // "nf-box" is what it IS. Spelling the glyphs once means the set the
            // check installs and the set that name promises cannot drift apart.
            "nerdfont" | "nf-box" => Arrows {
                // MDI arrow-up-bold-box F0738, arrow-down-bold-box F072F,
                // arrow-left-bold-box F0732, arrow-right-bold-box F0735
                north: '\u{F0738}', south: '\u{F072F}',
                east: '\u{F0735}', west: '\u{F0732}',
                // Native MDI bold-box diagonal arrows (verified)
                // arrow-top-left-bold-box F1968, arrow-top-right-bold-box F196A,
                // arrow-bottom-left-bold-box F1964, arrow-bottom-right-bold-box F1966
                nw: '\u{F1968}', ne: '\u{F196A}',
                sw: '\u{F1964}', se: '\u{F1966}',
            },
            "nf-circle" => Arrows {
                // MDI arrow-up-bold-circle F005F, arrow-down-bold-circle F0047,
                // arrow-left-bold-circle F004F, arrow-right-bold-circle F0056
                north: '\u{F005F}', south: '\u{F0047}',
                east: '\u{F0056}', west: '\u{F004F}',
                // No native MDI circle diagonal arrows; use Unicode fallback
                ne: '↗', nw: '↖', se: '↘', sw: '↙',
            },
            "nf-outline" => Arrows {
                // MDI arrow-up-bold-outline F09C7, arrow-down-bold-outline F09BF,
                // arrow-left-bold-outline F09C0, arrow-right-bold-outline F09C2
                north: '\u{F09C7}', south: '\u{F09BF}',
                east: '\u{F09C2}', west: '\u{F09C0}',
                // Native MDI bold-outline diagonal arrows (verified from MDI CSS)
                // arrow-top-left-bold-outline F09C3, arrow-top-right-bold-outline F09C5,
                // arrow-bottom-left-bold-outline F09B7, arrow-bottom-right-bold-outline F09B9
                nw: '\u{F09C3}', ne: '\u{F09C5}',
                sw: '\u{F09B7}', se: '\u{F09B9}',
            },
            _ => return None,
        })
    }
}

impl PathGlyphs {
    /// All known preset names for PathGlyphs, in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["light", "heavy", "dotted"]
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "light"  — light box-drawing lines ─│┌┐└┘├┤┬┴┼ (default)
    /// - "heavy"  — heavy box-drawing lines ━┃┏┓┗┛┣┫┳┻╋
    /// - "dotted" — dotted/dashed box-drawing lines ╌╎┄┆ with fallbacks
    pub fn preset(name: &str) -> Option<PathGlyphs> {
        Some(match name {
            // The four diag_* slots are identical across every preset: the Legacy
            // Computing block has only LIGHT half-diagonals — no heavy or dotted
            // variants exist — so they fall back the same way "dotted" already
            // falls back to light corners for its turns. (SQ-0314)
            "light" => PathGlyphs {
                ew: '─', ns: '│', se: '┌', sw: '┐', ne: '└', nw: '┘',
                nse: '├', nsw: '┤', ews: '┬', ewn: '┴', nesw: '┼',
                diag_ul: '🮠', diag_ur: '🮡',
                diag_ll: '🮢', diag_lr: '🮣',
            },
            "heavy" => PathGlyphs {
                ew: '━', ns: '┃', se: '┏', sw: '┓', ne: '┗', nw: '┛',
                nse: '┣', nsw: '┫', ews: '┳', ewn: '┻', nesw: '╋',
                diag_ul: '🮠', diag_ur: '🮡',
                diag_ll: '🮢', diag_lr: '🮣',
            },
            "dotted" => PathGlyphs {
                // Quadruple-dash light for straights; turns fall back to light corners
                // since Unicode has no dotted corner glyphs.
                ew: '┄', ns: '┆', se: '┌', sw: '┐', ne: '└', nw: '┘',
                nse: '├', nsw: '┤', ews: '┬', ewn: '┴', nesw: '┼',
                diag_ul: '🮠', diag_ur: '🮡',
                diag_ll: '🮢', diag_lr: '🮣',
            },
            _ => return None,
        })
    }
}

impl PortalGlyphs {
    /// All known preset names for PortalGlyphs, in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["ascii", "nerdfont", "nerdfont-stairs"]
    }

    /// The `(vertical, horizontal)` connector pair for a named portal-path
    /// preset, or `None` for an unknown name. Chosen by `portal_path_style`
    /// independently of the icon set, so the up/down/in/out links can be styled
    /// apart from the cardinal paths (`path_style`).
    ///
    /// - "light"  — │ / ─
    /// - "heavy"  — ┃ / ━
    /// - "dotted" — ┊ / ┄ (the default: the connectors the map has always drawn)
    pub fn path_preset(name: &str) -> Option<(char, char)> {
        Some(match name {
            "light" => ('│', '─'),
            "heavy" => ('┃', '━'),
            "dotted" => ('┊', '┄'),
            _ => return None,
        })
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "ascii"            — ASCII-compatible glyphs (default): ●/↑/↓/◉/◎/? with ┊┄ connectors
    /// - "nerdfont"         — Nerd Font single-width icon codepoints (requires patched font)
    ///   nf-fa-circle (U+F111) for marker, nf-md-arrow_up_circle (U+F0CE1) for up,
    ///   nf-md-arrow_down_circle (U+F0CDB) for down, nf-fa-sign_in (U+F090) for in,
    ///   nf-fa-sign_out (U+F08B) for out, nf-fa-question_circle (U+F059) for unknown
    /// - "nerdfont-stairs"  — Nerd Font 4 distinct direction icons (requires patched font)
    ///   up=mdi-stairs-up (U+F12BD), down=mdi-stairs-down (U+F12BE),
    ///   in=mdi-location-enter (U+F0FC4), out=mdi-exit-run (U+F0A48)
    pub fn preset(name: &str) -> Option<PortalGlyphs> {
        Some(match name {
            // In/Out are ◉ FISHEYE (U+25C9) and ◎ BULLSEYE (U+25CE), not the ⊙ (U+2299) and
            // ⊗ (U+2297) they were until SQ-0989. The old pair sits in Miscellaneous
            // Mathematical Operators, which monospace faces routinely skip: Fira Code — the
            // face `pty_stream::gallery::FONT_CANDIDATES` leads with, and a common terminal
            // font — carries neither (checked with `fc-list ":charset=2299"`, and pinned
            // against its cmap by SQ-0963), so the default map drew tofu or borrowed a
            // fallback face with the wrong metrics. Geometric Shapes is the block the map
            // already depends on for ● ▲ ▼ ◀ ▶, and every monospace face measured that has
            // ⊙/⊗ also has ◉/◎ — the swap costs no coverage and buys Fira Code's.
            // Same reading as before (a circle with something in it, non-directional):
            // ◉ a filled way in, ◎ a hollow way out. A user who prefers the old pair keeps
            // it with `portal.in`/`portal.out` overrides in `style.toml`.
            "ascii" => PortalGlyphs {
                marker: '●', path: '┊', path_h: '┄',
                up: '↑', down: '↓', in_: '◉', out: '◎', unknown: '?',
            },
            "nerdfont" => PortalGlyphs {
                // nf-fa-circle U+F111, connectors keep the same box-drawing chars
                marker: '\u{F111}', path: '┊', path_h: '┄',
                // md-arrow_up_circle U+F0CE1, md-arrow_down_circle U+F0CDB — resolved by NAME
                // from the Nerd Fonts `glyphnames.json` (v3.5.1). They used to read F0B71 and
                // F0B72, which that file calls md-card_bulleted_off{,_outline}: patched faces
                // do carry those codepoints, so the preset drew a crisp, confident icon of the
                // wrong thing rather than a missing glyph anyone would notice (SQ-0989).
                up: '\u{F0CE1}', down: '\u{F0CDB}',
                // nf-fa-sign_in U+F090, nf-fa-sign_out U+F08B
                in_: '\u{F090}', out: '\u{F08B}',
                // nf-fa-question_circle U+F059
                unknown: '\u{F059}',
            },
            "nerdfont-stairs" => PortalGlyphs {
                // Reuse nf-fa-circle U+F111 for marker, nf-fa-question_circle U+F059 for unknown
                marker: '\u{F111}', path: '┊', path_h: '┄',
                // Four DISTINCT direction icons (resolved from MDI webfont CSS by name):
                // mdi-stairs-up U+F12BD
                up: '\u{F12BD}',
                // mdi-stairs-down U+F12BE
                down: '\u{F12BE}',
                // mdi-location-enter U+F0FC4
                in_: '\u{F0FC4}',
                // mdi-exit-run U+F0A48
                out: '\u{F0A48}',
                unknown: '\u{F059}',
            },
            _ => return None,
        })
    }
}

impl ControlGlyphs {
    /// All known preset names for [`ControlGlyphs`], in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["plain", "nerdfont"]
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// Presets:
    /// - "plain"    — Geometric Shapes only (default): ◀▶▲▼ ●○ ◧■▦ ▣□
    /// - "nerdfont" — a named icon for every one of the eleven states.
    ///
    /// **Every nerdfont codepoint below was read from the font's own `post`
    /// table**, not inferred from a name. SQ-0989 is what a guessed codepoint
    /// costs: a patched face draws the wrong icon crisply and confidently and
    /// nobody notices, because there is nothing on our side that can see it.
    /// Two of the names originally proposed for this set do not exist in the
    /// font at all (`cod-layout_panel_dock`, `md-post_map`), which is exactly
    /// the failure the reading catches. `nerdfont_control_glyphs_are_the_names_
    /// that_were_read_from_the_font` pins them.
    ///
    /// **Each control's two states come from ONE icon family** — `fa-` for the
    /// map, `cod-` for the command band, `md-` for the Guiding Light, the render
    /// mode and the pixel lock. Codicons, Font Awesome and Material Design carry
    /// different stroke weights and cap heights, so a control whose states came
    /// from different families appeared to JUMP on toggle, independently of the
    /// shape change that was meant to be the signal.
    pub fn preset(name: &str) -> Option<ControlGlyphs> {
        let plain = SymbolSet::default().controls;
        Some(match name {
            "plain" => plain,
            "nerdfont" => ControlGlyphs {
                // fa-map_location / fa-map_location_dot — the dot reads as
                // "you are here", which is what an automap is for.
                map_show: '\u{0EE68}',
                map_hide: '\u{0EE69}',
                // cod-layout_panel_off / cod-layout_panel — a purpose-built
                // off/on pair rather than two icons pressed into service.
                band_show: '\u{0EC01}',
                band_hide: '\u{0EBF2}',
                // md-post_lamp — the Guiding Light's own mark, the same glyph
                // `font_check_dialog::ASSIST_LAMP` draws in the gutter — and
                // md-help for the light that is out.
                guidance_on: '\u{F1A60}',
                guidance_off: '\u{F02D6}',
                // md-monitor / md-monitor_shimmer / md-monitor_star: one screen
                // per way of drawing the screen.
                render_hybrid: '\u{F0379}',
                render_raster: '\u{F1104}',
                render_extended: '\u{F0DDC}',
                // md-lock / md-lock_open.
                lock_on: '\u{F033E}',
                lock_off: '\u{F033F}',
                // md-shoe_print — a footprint, for the search that walks the way
                // back and leaves nothing behind but the knowledge that it does.
                return_probe: '\u{F0DFA}',
                // md-flashlight — a lamp you point at one thing for a moment,
                // which is the whole feature. Read from the font's own `post`
                // table, like every codepoint here; do not substitute a name.
                reveal: '\u{F0244}',
            },
            _ => return None,
        })
    }
}

impl MapControlGlyphs {
    /// All known preset names, in display order — [`ControlGlyphs`]'s, because
    /// it shares its `control_icons` key.
    pub fn preset_names() -> &'static [&'static str] {
        ControlGlyphs::preset_names()
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// **Every nerdfont codepoint below was read from the font's own `post`
    /// table**, the rule SQ-0989 bought and SQ-1141 refined: name resolved to a
    /// codepoint, codepoint resolved back to a unique name, and the outline
    /// confirmed non-empty so a blank glyph cannot pass for a drawn one.
    /// `nerdfont_map_control_glyphs_are_the_names_that_were_read_from_the_font`
    /// pins them.
    ///
    /// All seven are Material Design, the family the border controls already
    /// draw from, so the two clusters cannot disagree about stroke weight or cap
    /// height the way mixed families did before SQ-0989.
    ///
    /// **This preset fills both halves of both two-mode pairs, where the plain
    /// preset repeats one mark.** That is the house rule working as intended:
    /// a two-mode control changes SHAPE, and a patched font can draw the
    /// off-shape where ASCII simply has none. See [`MapControlGlyphs`].
    pub fn preset(name: &str) -> Option<MapControlGlyphs> {
        let plain = SymbolSet::default().map_controls;
        Some(match name {
            "plain" => plain,
            "nerdfont" => MapControlGlyphs {
                // md-numeric / md-numeric_off — a real shape change, which is
                // what the plain `#`/`#` gives up for coverage.
                room_numbers_on: '\u{F03A0}',
                room_numbers_off: '\u{F19D3}',
                // md-crosshairs — the reticle `¤` gestures at, drawn properly.
                centre: '\u{F01A3}',
                // md-magnify_minus / md-magnify_plus: one family, one shape, the
                // sign being the only thing that differs.
                zoom_out: '\u{F034A}',
                zoom_in: '\u{F034B}',
                // md-grid / md-grid_off — a lattice and a lattice struck
                // through. The plain preset cannot follow: `M` has no
                // off-shape, so it repeats and lets colour say which.
                view_matrix: '\u{F02C1}',
                view_drawn: '\u{F02C2}',
            },
            _ => return None,
        })
    }
}

impl StoryBadges {
    /// The plain answer, and the letters the picker has drawn since it landed:
    /// `S H h`. Also the source of the `config::default_badge_*` values, so
    /// there is one place a default badge is spelled rather than two that agree
    /// by hand.
    pub const PLAIN: StoryBadges = StoryBadges {
        save: 'S',
        hint: 'H',
        hint_available: 'h',
    };

    /// All known preset names for [`StoryBadges`], in display order.
    pub fn preset_names() -> &'static [&'static str] {
        &["plain", "nerdfont"]
    }

    /// Return a named preset, or `None` for an unknown name.
    ///
    /// - "plain"    — [`Self::PLAIN`], the three letters (default).
    /// - "nerdfont" — three Material Design icons, which is also why the font
    ///   check's sample row needs no new slot for them: the row already samples
    ///   MDI (`md-post_lamp` and the boxed arrows), and a face that draws that
    ///   draws these. This is the same argument [`ControlGlyphs`]'s nerdfont arm
    ///   makes, for the same reason.
    ///
    /// **Every codepoint below was read from the patched font's own `cmap`, and
    /// its name from the font's own glyph order** (SQ-1045's rule, SQ-1141's
    /// method) — never from a Nerd Fonts cheat sheet. Checked across the nine
    /// patched families installed on the machine this was chosen on
    /// (0xProto, Fira Code ×3, IosevkaTerm, JetBrains Mono, ProggyClean,
    /// SauceCodePro and Symbols Nerd Font Mono): each resolves to the SAME
    /// codepoint under the SAME name in all nine, and each rasterises with ink
    /// at 12–16px, so none of them is tofu on a face that claims the range.
    /// **They are not to be re-derived.** SQ-1160 retired three of the original
    /// six and SQ-1168 relieved the surviving three of a weight they never had;
    /// each replacement was read the same way, in all nine, before it shipped.
    ///
    /// The choices, and why each survives being a few pixels tall in a list row:
    ///
    /// - `md-content_save_outline` — the floppy, hollow. The most legible small
    ///   silhouette there is, and the one glyph here nobody has to be taught.
    /// - `md-lightbulb_on` / `md-lightbulb_on_outline` — one FAMILY and one SHAPE
    ///   for the hint slot's two states, differing only in fill, because that is
    ///   the distinction being drawn: filled is a hint you have, hollow is one you
    ///   could fetch. It is the same reading `H`/`h` carried, and the same
    ///   filled-is-settled grammar as the portal icons' ◉/◎ and the room dock's
    ///   two header marks.
    ///
    /// **All three are the OUTLINE-weight, `16x16`-boxed members of their group,
    /// and that is the whole of SQ-1168.** The set shipped by SQ-1159 was
    /// `md-content_save` (filled) and the `md-lightbulb`/`md-lightbulb_outline`
    /// pair, and it read as bold when it is not: `story_badge` inherits `text`
    /// with an EMPTY delta, so nothing adds a modifier — the weight is the
    /// drawing. Measured in the gallery's own face and size (Fira Code Nerd Font
    /// Mono at 26 px/em in a 16x32 cell, which is what `Face::outline` picks for
    /// the shots the report was made against), against a cap height of 18.4px:
    ///
    /// | mark | ink | vs cap |
    /// |---|---|---|
    /// | `S` `H` `h`, the letters these replaced | 13x19, 12x18, 11x20 | 1.00 |
    /// | `md-lightbulb` / `_outline` (was) | **16x23** | **1.25** |
    /// | `md-content_save` (was) | 16x16, solid | 0.87 |
    /// | `md-content_save_outline` (now) | 16x16, hollow | 0.87 |
    /// | `md-lightbulb_on` / `_on_outline` (now) | 16x16 | 0.87 |
    ///
    /// So the bulb was a quarter TALLER than the letters beside it and the floppy
    /// was solid ink edge to edge; every badge now sits inside the same 16x16 box
    /// and none of them overshoots the letters. Nerd Fonts scales each icon by its
    /// source artwork's own bounds, so this is per-GLYPH and not per-family: the
    /// `_on` bulbs are smaller than the plain ones because their rays are counted
    /// into the box that gets normalised. **A lighter colour or a style modifier
    /// would not have fixed it** — there is no modifier to remove.
    ///
    /// **The hint pair must come from one family, and Material Design is the only
    /// family that HAS one.** Font Awesome carries exactly one bulb
    /// (`fa-lightbulb_o`, U+0F0EB — established on SQ-1159), Octicons exactly one
    /// (`oct-light_bulb`, U+0F400), and **Codicons exactly one too**:
    /// `cod-lightbulb` (U+0EA61) and `cod-lightbulb_autofix` (U+0EB13) are the
    /// whole of that family, and the second is the same solid bulb with a gear
    /// beside it, not the first one hollowed. `cod-lightbulb` is by some way the
    /// lightest bulb in the patched font (12x17, the closest of any to letter
    /// size) and it cannot be used here, because the `Available` state would then
    /// differ from `Present` by COLOUR alone — the degradation SQ-1148 accepts
    /// only where the font offers no off-shape, and one this font does offer.
    pub fn preset(name: &str) -> Option<StoryBadges> {
        Some(match name {
            "plain" => StoryBadges::PLAIN,
            "nerdfont" => StoryBadges {
                save: '\u{F0818}',           // md-content_save_outline
                hint: '\u{F06E8}',           // md-lightbulb_on
                hint_available: '\u{F06E9}', // md-lightbulb_on_outline
            },
            _ => return None,
        })
    }
}

// ── resolve ───────────────────────────────────────────────────────────────────

impl SymbolSet {
    /// Build a `SymbolSet` from a `SymbolConfig`:
    /// 1. Start from each category's named preset (unknown name → category default).
    /// 2. Apply per-slot overrides from `cfg.overrides`.
    ///
    /// Override validation: the value must be exactly one `char` (checked via
    /// `chars().count() == 1`). We do not add a `unicode-width` dependency; we
    /// instead reject any char with a code point in the known CJK/wide ranges
    /// (U+1100..=U+FFEF broad block that covers fullwidth and wide CJK) plus any
    /// char above U+FFFF that terminals commonly render as double-wide (emoji etc.).
    /// Single-byte ASCII and the entire BMP box-drawing block are always accepted.
    /// Invalid values (empty, multi-char, wide estimate) → keep the preset glyph.
    pub fn resolve(cfg: &crate::config::SymbolConfig) -> SymbolSet {
        let mut s = SymbolSet {
            room_normal: BoxStyle::preset(&cfg.box_style).unwrap_or_else(|| SymbolSet::default().room_normal),
            room_current: SymbolSet::default().room_current,
            room_portal: SymbolSet::default().room_portal,
            room_selected: BoxStyle::preset(&cfg.box_style).unwrap_or_else(|| SymbolSet::default().room_selected),
            arrows: Arrows::preset(&cfg.arrow_set).unwrap_or_else(|| SymbolSet::default().arrows),
            path: PathGlyphs::preset(&cfg.path_style).unwrap_or_else(|| SymbolSet::default().path),
            portal: PortalGlyphs::preset(&cfg.portal_icons).unwrap_or_else(|| SymbolSet::default().portal),
            controls: ControlGlyphs::preset(&cfg.control_icons).unwrap_or_else(|| SymbolSet::default().controls),
            tip: TipGlyphs::preset(&cfg.control_icons).unwrap_or_else(|| SymbolSet::default().tip),
            map_controls: MapControlGlyphs::preset(&cfg.control_icons)
                .unwrap_or_else(|| SymbolSet::default().map_controls),
            meta_gutter: SymbolSet::default().meta_gutter,
            warning_gutter: SymbolSet::default().warning_gutter,
            assist_gutter: SymbolSet::default().assist_gutter,
            dock_following: SymbolSet::default().dock_following,
            dock_pinned: SymbolSet::default().dock_pinned,
            diagonal_corners: cfg.diagonal_corners,
        };

        // The portal connectors are a preset of their own, layered on the icon
        // set: every icon preset ships the same ┊/┄ pair, so `portal_path_style`
        // is what actually chooses them (unknown name → keep the icon set's).
        if let Some((v, h)) = PortalGlyphs::path_preset(&cfg.portal_path_style) {
            s.portal.path = v;
            s.portal.path_h = h;
        }

        for (key, val) in &cfg.overrides {
            // Validate: exactly one char, estimated single display width.
            let mut chars = val.chars();
            let Some(ch) = chars.next() else { continue }; // empty
            if chars.next().is_some() { continue; } // multi-char
            if is_wide_estimate(ch) { continue; } // likely wide

            apply_override(&mut s, key, ch);
        }

        s
    }

    /// Build a `SymbolSet` from four named presets (box, arrow, portal, path).
    /// Unknown preset names fall back to the category default (same as `resolve`).
    pub fn from_preset_names(box_: &str, arrow: &str, portal: &str, path: &str) -> SymbolSet {
        let cfg = crate::config::SymbolConfig {
            box_style: box_.to_owned(),
            arrow_set: arrow.to_owned(),
            portal_icons: portal.to_owned(),
            path_style: path.to_owned(),
            portal_path_style: crate::config::default_portal_path_style(),
            control_icons: crate::config::default_control_icons(),
            badge_save: crate::config::default_badge_save(),
            badge_hint: crate::config::default_badge_hint(),
            badge_hint_available: crate::config::default_badge_hint_available(),
            diagonal_corners: crate::config::default_diagonal_corners(),
            overrides: std::collections::BTreeMap::new(),
        };
        SymbolSet::resolve(&cfg)
    }
}

/// Conservative "likely wide" estimate without unicode-width dependency.
/// Rejects chars in the CJK/fullwidth/emoji-heavy ranges. The box-drawing
/// block (U+2500..=U+257F), arrows (U+2190..=U+21FF), and BMP geometric
/// shapes are always accepted.
pub(crate) fn is_wide_estimate(c: char) -> bool {
    let cp = c as u32;
    // U+1FBA0..=U+1FBAF are the Legacy Computing box-drawing half-diagonals: narrow
    // line-art, not emoji, despite sitting inside the blanket 0x1F000..=0x1FFFF
    // reject below. Carve them out or `path.diag_*` overrides are silently dropped
    // and the slots stop being themeable. (SQ-0314)
    if (0x1FBA0..=0x1FBAF).contains(&cp) {
        return false;
    }
    matches!(cp,
        0x1100..=0x115F  // Hangul Jamo
        | 0x2E80..=0x2EFF  // CJK Radicals
        | 0x2F00..=0x2FDF  // Kangxi Radicals
        | 0x2FF0..=0x303F  // CJK Symbols
        | 0x3040..=0x309F  // Hiragana
        | 0x30A0..=0x30FF  // Katakana
        | 0x3100..=0x312F  // Bopomofo
        | 0x3130..=0x318F  // Hangul Compatibility
        | 0x3190..=0x319F  // Kanbun
        | 0x31A0..=0x31BF  // Bopomofo Extended
        | 0x31F0..=0x31FF  // Katakana Phonetic
        | 0x3200..=0x32FF  // Enclosed CJK
        | 0x3300..=0x33FF  // CJK Compatibility
        | 0x3400..=0x4DBF  // CJK Extension A
        | 0x4E00..=0x9FFF  // CJK Unified Ideographs
        | 0xA000..=0xA48F  // Yi Syllables
        | 0xA490..=0xA4CF  // Yi Radicals
        | 0xAC00..=0xD7AF  // Hangul Syllables
        | 0xF900..=0xFAFF  // CJK Compatibility Ideographs
        | 0xFE10..=0xFE1F  // Vertical Forms
        | 0xFE30..=0xFE4F  // CJK Compatibility Forms
        | 0xFE50..=0xFE6F  // Small Form Variants
        | 0xFF00..=0xFFEF  // Halfwidth/Fullwidth Forms
        | 0x1F000..=0x1FFFF // Emoji, Mahjong, etc.
        | 0x20000..=0x2A6DF // CJK Extension B
        | 0x2A700..=0x2CEAF // CJK Extension C/D/E
    )
}

/// Apply one validated override char to the matching slot in `s`.
/// Unknown slot keys are ignored.
fn apply_override(s: &mut SymbolSet, key: &str, ch: char) {
    match key {
        "room.normal.tl"   => s.room_normal.tl = ch,
        "room.normal.tr"   => s.room_normal.tr = ch,
        "room.normal.bl"   => s.room_normal.bl = ch,
        "room.normal.br"   => s.room_normal.br = ch,
        "room.normal.h"    => s.room_normal.h = ch,
        "room.normal.v"    => s.room_normal.v = ch,
        "room.current.tl"  => s.room_current.tl = ch,
        "room.current.tr"  => s.room_current.tr = ch,
        "room.current.bl"  => s.room_current.bl = ch,
        "room.current.br"  => s.room_current.br = ch,
        "room.current.h"   => s.room_current.h = ch,
        "room.current.v"   => s.room_current.v = ch,
        "room.portal.tl"   => s.room_portal.tl = ch,
        "room.portal.tr"   => s.room_portal.tr = ch,
        "room.portal.bl"   => s.room_portal.bl = ch,
        "room.portal.br"   => s.room_portal.br = ch,
        "room.portal.h"    => s.room_portal.h = ch,
        "room.portal.v"    => s.room_portal.v = ch,
        "room.selected.tl" => s.room_selected.tl = ch,
        "room.selected.tr" => s.room_selected.tr = ch,
        "room.selected.bl" => s.room_selected.bl = ch,
        "room.selected.br" => s.room_selected.br = ch,
        "room.selected.h"  => s.room_selected.h = ch,
        "room.selected.v"  => s.room_selected.v = ch,
        "arrow.north"      => s.arrows.north = ch,
        "arrow.south"      => s.arrows.south = ch,
        "arrow.east"       => s.arrows.east = ch,
        "arrow.west"       => s.arrows.west = ch,
        "arrow.ne"         => s.arrows.ne = ch,
        "arrow.nw"         => s.arrows.nw = ch,
        "arrow.se"         => s.arrows.se = ch,
        "arrow.sw"         => s.arrows.sw = ch,
        "path.ew"          => s.path.ew = ch,
        "path.ns"          => s.path.ns = ch,
        "path.se"          => s.path.se = ch,
        "path.sw"          => s.path.sw = ch,
        "path.ne"          => s.path.ne = ch,
        "path.nw"          => s.path.nw = ch,
        "path.nse"         => s.path.nse = ch,
        "path.nsw"         => s.path.nsw = ch,
        "path.ews"         => s.path.ews = ch,
        "path.ewn"         => s.path.ewn = ch,
        "path.cross"       => s.path.nesw = ch,
        "path.diag_ul"     => s.path.diag_ul = ch,
        "path.diag_ur"     => s.path.diag_ur = ch,
        "path.diag_ll"     => s.path.diag_ll = ch,
        "path.diag_lr"     => s.path.diag_lr = ch,
        "portal.up"        => s.portal.up = ch,
        "portal.down"      => s.portal.down = ch,
        "portal.in"        => s.portal.in_ = ch,
        "portal.out"       => s.portal.out = ch,
        "portal.unknown"   => s.portal.unknown = ch,
        "portal.path"      => s.portal.path = ch,
        "portal.marker"    => s.portal.marker = ch,
        "control.map_show"       => s.controls.map_show = ch,
        "control.map_hide"       => s.controls.map_hide = ch,
        "control.band_show"      => s.controls.band_show = ch,
        "control.band_hide"      => s.controls.band_hide = ch,
        "control.guidance_on"    => s.controls.guidance_on = ch,
        "control.guidance_off"   => s.controls.guidance_off = ch,
        "control.render_hybrid"  => s.controls.render_hybrid = ch,
        "control.render_raster"  => s.controls.render_raster = ch,
        "control.render_extended" => s.controls.render_extended = ch,
        "control.lock_on"        => s.controls.lock_on = ch,
        "control.lock_off"       => s.controls.lock_off = ch,
        "control.return_probe"   => s.controls.return_probe = ch,
        "control.reveal"         => s.controls.reveal = ch,
        // The map pane's own cluster (SQ-1148). Both halves of each two-mode
        // pair are settable, which is the point of keeping the pair: the plain
        // preset spells one mark twice, and a player who wants a shape change
        // sets the off half themselves. `md-numeric_off` (U+F19D3) and
        // `md-grid_off` (U+F02C2) are what such a player would reach for.
        "map_control.room_numbers_on"  => s.map_controls.room_numbers_on = ch,
        "map_control.room_numbers_off" => s.map_controls.room_numbers_off = ch,
        "map_control.centre"           => s.map_controls.centre = ch,
        "map_control.zoom_out"         => s.map_controls.zoom_out = ch,
        "map_control.zoom_in"          => s.map_controls.zoom_in = ch,
        "map_control.view_matrix"      => s.map_controls.view_matrix = ch,
        "map_control.view_drawn"       => s.map_controls.view_drawn = ch,
        "tip.up_left"      => s.tip.up_left = ch,
        "tip.up_right"     => s.tip.up_right = ch,
        "tip.down_left"    => s.tip.down_left = ch,
        "tip.down_right"   => s.tip.down_right = ch,
        "gutter.meta"      => s.meta_gutter = ch,
        "gutter.warning"   => s.warning_gutter = ch,
        "gutter.assist"    => s.assist_gutter = ch,
        "dock.following"   => s.dock_following = ch,
        "dock.pinned"      => s.dock_pinned = ch,
        _ => {} // unknown key — ignored
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_glyph_defaults_and_overrides() {
        let s = SymbolSet::default();
        assert_eq!(s.meta_gutter, '▏');
        assert_eq!(s.warning_gutter, '!');
        // resolve(default) keeps defaults.
        assert_eq!(SymbolSet::resolve(&crate::config::SymbolConfig::default()), SymbolSet::default());
        // overrides apply.
        let mut cfg = crate::config::SymbolConfig::default();
        cfg.overrides.insert("gutter.meta".into(), "|".into());
        cfg.overrides.insert("gutter.warning".into(), "*".into());
        let r = SymbolSet::resolve(&cfg);
        assert_eq!(r.meta_gutter, '|');
        assert_eq!(r.warning_gutter, '*');
    }

    #[test]
    fn default_matches_todays_glyphs() {
        let s = SymbolSet::default();
        assert_eq!((s.room_normal.tl, s.room_normal.br, s.room_normal.h), ('╭', '╯', '─'));
        assert_eq!((s.room_current.tl, s.room_current.v), ('┏', '┃'));
        assert_eq!((s.room_portal.tl, s.room_portal.v), ('╔', '║'));
        // selected defaults to the normal set (color-only selection today)
        assert_eq!((s.room_selected.tl, s.room_selected.v), (s.room_normal.tl, s.room_normal.v));
        assert_eq!((s.arrows.north, s.arrows.east, s.arrows.ne), ('▲', '▶', '↗'));
        assert_eq!((s.path.ew, s.path.nesw, s.path.se), ('─', '┼', '┌'));
        assert_eq!(s.portal.marker, '●');
    }

    #[test]
    fn presets_resolve_and_default_names_match_default_set() {
        assert_eq!(BoxStyle::preset("rounded"), Some(SymbolSet::default().room_normal));
        let ascii = BoxStyle::preset("ascii").unwrap();
        assert_eq!((ascii.tl, ascii.h, ascii.v), ('+', '-', '|'));
        let borderless = BoxStyle::preset("borderless").unwrap();
        assert_eq!(borderless.h, ' ');
        assert_eq!(Arrows::preset("filled"), Some(SymbolSet::default().arrows));
        assert_eq!(PathGlyphs::preset("light"), Some(SymbolSet::default().path));
        assert!(BoxStyle::preset("nonsense").is_none());
    }

    #[test]
    fn resolve_default_config_equals_default_set() {
        let cfg = crate::config::SymbolConfig::default();
        assert_eq!(SymbolSet::resolve(&cfg), SymbolSet::default());
    }

    #[test]
    fn resolve_applies_preset_then_override() {
        let mut cfg = crate::config::SymbolConfig::default();
        cfg.box_style = "ascii".into();
        cfg.overrides.insert("room.normal.tl".into(), "#".into());
        let s = SymbolSet::resolve(&cfg);
        assert_eq!(s.room_normal.tl, '#');   // override beats preset
        assert_eq!(s.room_normal.h, '-');    // rest from ascii preset
    }

    #[test]
    fn resolve_rejects_bad_width_override() {
        let mut cfg = crate::config::SymbolConfig::default();
        cfg.overrides.insert("arrow.north".into(), "ab".into());  // multi-char
        cfg.overrides.insert("arrow.south".into(), "".into());    // empty
        let s = SymbolSet::resolve(&cfg);
        assert_eq!(s.arrows.north, SymbolSet::default().arrows.north); // unchanged
        assert_eq!(s.arrows.south, SymbolSet::default().arrows.south);
    }

    #[test]
    fn legacy_computing_diagonals_are_not_rejected_as_wide() {
        // SQ-0314: U+1FBA0..=U+1FBAF sit inside the blanket 0x1F000..=0x1FFFF
        // "emoji" reject, but they are NARROW box-drawing half-diagonals. Without
        // the carve-out every `path.diag_*` override is silently dropped and the
        // slots stop being themeable.
        for cp in 0x1FBA0..=0x1FBAFu32 {
            let ch = char::from_u32(cp).unwrap();
            assert!(!is_wide_estimate(ch), "U+{cp:05X} {ch:?} must be accepted as narrow");
        }
        // The guard is surgical: a neighbouring real emoji is still rejected.
        assert!(is_wide_estimate('\u{1F600}'), "emoji outside the carve-out stay rejected");
        assert!(is_wide_estimate('\u{1FB00}'), "0x1FB00 is below the carve-out and stays rejected");
    }

    #[test]
    fn diagonal_slots_default_to_legacy_computing_and_accept_overrides() {
        // Defaults are the four half-diagonals, and every preset carries them
        // (Legacy Computing has no heavy/dotted variants, so all presets share them).
        let d = SymbolSet::default().path;
        assert_eq!((d.diag_ul, d.diag_ur, d.diag_ll, d.diag_lr),
                   ('🮠', '🮡', '🮢', '🮣'));
        for name in PathGlyphs::preset_names() {
            let p = PathGlyphs::preset(name).unwrap();
            assert_eq!((p.diag_ul, p.diag_ur, p.diag_ll, p.diag_lr),
                       ('🮠', '🮡', '🮢', '🮣'), "preset {name}");
        }
        // And they are themeable: an override reaches the slot (proving the
        // is_wide_estimate carve-out and the apply_override key are both wired).
        let mut cfg = crate::config::SymbolConfig::default();
        cfg.overrides.insert("path.diag_ul".into(), "🮣".into());
        cfg.overrides.insert("path.diag_lr".into(), "/".into());
        let s = SymbolSet::resolve(&cfg);
        assert_eq!(s.path.diag_ul, '🮣', "a Legacy Computing override is accepted");
        assert_eq!(s.path.diag_lr, '/', "an ASCII override is accepted");
    }

    #[test]
    fn diagonal_corners_defaults_on_and_follows_config() {
        assert!(SymbolSet::default().diagonal_corners, "diagonals are on out of the box");
        assert!(SymbolSet::resolve(&crate::config::SymbolConfig::default()).diagonal_corners);
        // Turning it off is what a font without Unicode 13 coverage does.
        let cfg = crate::config::SymbolConfig { diagonal_corners: false, ..Default::default() };
        assert!(!SymbolSet::resolve(&cfg).diagonal_corners);
    }

    #[test]
    fn preset_names_cover_all_known_presets() {
        assert!(BoxStyle::preset_names().contains(&"ascii"));
        assert!(BoxStyle::preset_names().contains(&"rounded"));
        assert!(Arrows::preset_names().contains(&"filled"));
        assert!(PathGlyphs::preset_names().contains(&"light"));
        assert!(PortalGlyphs::preset_names().contains(&"ascii"));
    }

    #[test]
    fn from_preset_names_matches_resolve() {
        let cfg = crate::config::SymbolConfig {
            box_style: "ascii".into(),
            arrow_set: "filled".into(),
            portal_icons: "ascii".into(),
            path_style: "light".into(),
            portal_path_style: crate::config::default_portal_path_style(),
            control_icons: crate::config::default_control_icons(),
            badge_save: crate::config::default_badge_save(),
            badge_hint: crate::config::default_badge_hint(),
            badge_hint_available: crate::config::default_badge_hint_available(),
            diagonal_corners: crate::config::default_diagonal_corners(),
            overrides: std::collections::BTreeMap::new(),
        };
        let expected = SymbolSet::resolve(&cfg);
        let got = SymbolSet::from_preset_names("ascii", "filled", "ascii", "light");
        assert_eq!(got, expected);
    }

    /// The DEFAULT icons must be drawable by an ordinary monospace face, which the
    /// ⊙/⊗ they used to be were not (SQ-0989): Fira Code — the face the gallery
    /// rasterises with — carries no Miscellaneous Mathematical Operators at all.
    /// Geometric Shapes is the block the map already requires for ● ▲ ▼ ◀ ▶, so
    /// pin the default in/out there and a future swap back to a maths operator
    /// fails here instead of on somebody's screen.
    #[test]
    fn default_portal_in_out_come_from_geometric_shapes() {
        let p = PortalGlyphs::preset("ascii").expect("default preset");
        let shapes = 0x25A0..=0x25FF;
        for (slot, ch) in [("in", p.in_), ("out", p.out)] {
            assert!(shapes.contains(&(ch as u32)), "portal.{slot} = {ch:?} is outside Geometric Shapes");
            assert!(!is_wide_estimate(ch), "portal.{slot} = {ch:?} estimates as double-width");
        }
        assert_ne!(p.in_, p.out, "in and out must be told apart");
        assert_ne!(p.in_, p.marker, "in must not read as the notes marker");
        assert_ne!(p.out, p.marker, "out must not read as the notes marker");
        // And the pair is the same one `SymbolSet::default()` hands the renderer.
        assert_eq!((SymbolSet::default().portal.in_, SymbolSet::default().portal.out), (p.in_, p.out));
    }

    /// Codepoints resolved by NAME from the Nerd Fonts `glyphnames.json` (v3.5.1),
    /// not from memory: up/down here were F0B71/F0B72 — `md-card_bulleted_off` and
    /// its outline — for as long as the preset existed, and a patched face draws
    /// those happily, so nothing looked broken (SQ-0989).
    #[test]
    fn nerdfont_portal_icons_are_the_named_codepoints() {
        let p = PortalGlyphs::preset("nerdfont").expect("preset");
        assert_eq!(p.up, '\u{F0CE1}', "md-arrow_up_circle");
        assert_eq!(p.down, '\u{F0CDB}', "md-arrow_down_circle");
        assert_eq!(p.in_, '\u{F090}', "fa-sign_in");
        assert_eq!(p.out, '\u{F08B}', "fa-sign_out");
        assert_eq!(p.marker, '\u{F111}', "fa-circle");
        assert_eq!(p.unknown, '\u{F059}', "fa-question_circle");
    }

    #[test]
    fn nerdfont_stairs_portal_has_four_distinct_single_width_icons() {
        assert!(PortalGlyphs::preset_names().contains(&"nerdfont-stairs"));
        let p = PortalGlyphs::preset("nerdfont-stairs").unwrap();
        // four DISTINCT direction icons
        let four = [p.up, p.down, p.in_, p.out];
        for ch in four { assert!(!is_wide_estimate(ch)); }
        assert_eq!(four.iter().collect::<std::collections::HashSet<_>>().len(), 4, "up/down/in/out must differ");
    }

    /// The map cluster's nerdfont set is SEVEN named icons, each codepoint read
    /// from the font's own `post` table (SQ-1148) by the SQ-1141 method: name to
    /// codepoint, codepoint back to a unique name, and a non-empty outline so a
    /// blank glyph cannot pass for a drawn one.
    ///
    /// **Seven, because the patched preset obeys the shape rule that the plain
    /// preset cannot.** Both two-mode controls get a real off-shape here —
    /// `md-numeric_off` and `md-grid_off` — where the plain set repeats `#` and
    /// `M` and lets colour carry the state. Each preset does the best its own
    /// font allows; see [`MapControlGlyphs`].
    #[test]
    fn nerdfont_map_control_glyphs_are_the_names_that_were_read_from_the_font() {
        let m = MapControlGlyphs::preset("nerdfont").expect("preset");
        for (name, got, want) in [
            ("md-numeric", m.room_numbers_on, '\u{F03A0}'),
            ("md-numeric_off", m.room_numbers_off, '\u{F19D3}'),
            ("md-crosshairs", m.centre, '\u{F01A3}'),
            ("md-magnify_minus", m.zoom_out, '\u{F034A}'),
            ("md-magnify_plus", m.zoom_in, '\u{F034B}'),
            ("md-grid", m.view_matrix, '\u{F02C1}'),
            ("md-grid_off", m.view_drawn, '\u{F02C2}'),
        ] {
            assert_eq!(got, want, "{name} moved: U+{:05X} is not U+{:05X}", got as u32, want as u32);
        }

        let all = [
            m.room_numbers_on, m.room_numbers_off, m.centre,
            m.zoom_out, m.zoom_in, m.view_matrix, m.view_drawn,
        ];

        // All seven from ONE family, which is what keeps the cluster from
        // appearing to jump between weights — the lesson SQ-0989 paid for on the
        // border controls, restated here because a new cluster is where it recurs.
        assert!(
            all.iter().all(|c| ('\u{F0001}'..='\u{F1AF0}').contains(c)),
            "every map control glyph must be Material Design"
        );

        // BOTH two-mode pairs are a real shape change in this preset, which is
        // the whole reason it is seven marks and not five.
        assert_ne!(m.room_numbers_on, m.room_numbers_off, "patched must change SHAPE on the numbers");
        assert_ne!(m.view_matrix, m.view_drawn, "patched must change SHAPE on the view");
        // …and every slot is a distinct mark, so no two controls read alike.
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two map controls draw the same glyph U+{:05X}", *a as u32);
            }
        }
    }

    /// The plain map cluster is drawable everywhere, which is the whole reason
    /// these particular marks were chosen over prettier ones.
    ///
    /// `#` and `M` especially: they are the two slots whose states are told
    /// apart by COLOUR rather than by shape, so if either failed to draw there
    /// would be no fallback reading left at all — where a patched glyph that
    /// tofus still leaves its differently-shaped opposite recognisable beside
    /// it. ASCII is what makes that safe, and this asserts it rather than
    /// trusting the choice to survive an edit.
    ///
    /// It also pins that the plain preset REPEATS each of those marks, since
    /// "one mark, two slots" is the whole shape of the degradation and a
    /// half-applied edit would leave one state drawn as something else.
    #[test]
    fn the_plain_map_cluster_needs_no_patched_font() {
        let m = SymbolSet::default().map_controls;
        for (what, on, off, want) in [
            ("room numbers", m.room_numbers_on, m.room_numbers_off, '#'),
            ("the view switch", m.view_matrix, m.view_drawn, 'M'),
        ] {
            assert!(on.is_ascii(), "{what} must be ASCII — colour is its only other channel");
            assert_eq!(on, want, "{what}");
            assert_eq!(on, off, "{what}: the plain preset spells one mark in both slots");
        }
        for c in [
            m.room_numbers_on, m.room_numbers_off, m.centre,
            m.zoom_out, m.zoom_in, m.view_matrix, m.view_drawn,
        ] {
            assert!(
                !('\u{E000}'..='\u{F8FF}').contains(&c) && !('\u{F0000}'..='\u{FFFFD}').contains(&c),
                "U+{:04X} is private-use — the plain preset must not need a patch",
                c as u32
            );
        }
    }

    /// `control_icons` drives the map cluster, the border controls and the
    /// tooltip pointer from ONE answer (SQ-1148). Not three keys that happen to
    /// agree: a patched font draws all of them or none of them, and asking twice
    /// would let a config end up half-patched with no way for the player to tell
    /// which half.
    #[test]
    fn control_icons_resolves_the_map_cluster_as_well_as_the_border_controls() {
        for name in ["plain", "nerdfont"] {
            let cfg = crate::config::SymbolConfig { control_icons: name.into(), ..Default::default() };
            let set = SymbolSet::resolve(&cfg);
            assert_eq!(
                set.map_controls,
                MapControlGlyphs::preset(name).unwrap(),
                "control_icons = {name:?} must reach the map cluster"
            );
            assert_eq!(set.controls, ControlGlyphs::preset(name).unwrap());
            assert_eq!(set.tip, TipGlyphs::preset(name).unwrap());
        }
        // An unknown name falls back to plain for all three, the way every other
        // category treats one.
        let cfg = crate::config::SymbolConfig {
            control_icons: "no-such-preset".into(),
            ..Default::default()
        };
        assert_eq!(SymbolSet::resolve(&cfg).map_controls, SymbolSet::default().map_controls);
    }

    /// The border controls' nerdfont set is TWELVE named icons, each codepoint
    /// read from the font's own `post` table rather than inferred from a name.
    ///
    /// This pins the numbers, because nothing else can: SQ-0989 is what a
    /// guessed codepoint costs — a patched face draws the wrong icon crisply and
    /// confidently, and there is no assertion on our side of the terminal that
    /// could notice. Two of the names first proposed for this set turned out not
    /// to exist in the font (`cod-layout_panel_dock`, `md-post_map`), which is
    /// the same failure caught one step earlier.
    #[test]
    fn nerdfont_control_glyphs_are_the_names_that_were_read_from_the_font() {
        let c = ControlGlyphs::preset("nerdfont").expect("preset");
        for (name, got, want) in [
            ("fa-map_location", c.map_show, '\u{0EE68}'),
            ("fa-map_location_dot", c.map_hide, '\u{0EE69}'),
            ("cod-layout_panel_off", c.band_show, '\u{0EC01}'),
            ("cod-layout_panel", c.band_hide, '\u{0EBF2}'),
            ("md-help", c.guidance_off, '\u{F02D6}'),
            ("md-post_lamp", c.guidance_on, '\u{F1A60}'),
            ("md-monitor", c.render_hybrid, '\u{F0379}'),
            ("md-monitor_shimmer", c.render_raster, '\u{F1104}'),
            ("md-monitor_star", c.render_extended, '\u{F0DDC}'),
            ("md-lock_open", c.lock_off, '\u{F033F}'),
            ("md-lock", c.lock_on, '\u{F033E}'),
            ("md-shoe_print", c.return_probe, '\u{F0DFA}'),
            ("md-flashlight", c.reveal, '\u{F0244}'),
        ] {
            assert_eq!(got, want, "{name} moved: U+{:05X} is not U+{:05X}", got as u32, want as u32);
        }
        // The Guiding Light's lit mark is the SAME glyph the gutter draws, not a
        // second lamp that could drift away from it.
        assert_eq!(c.guidance_on, crate::render::font_check_dialog::ASSIST_LAMP);
        // Each toggle's two states must still differ, and each pair must stay
        // inside ONE icon family — mixed families have different stroke weights
        // and cap heights, so the control appears to jump on toggle.
        for (slot, off, on) in [
            ("map", c.map_show, c.map_hide),
            ("band", c.band_show, c.band_hide),
            ("guidance", c.guidance_off, c.guidance_on),
            ("lock", c.lock_off, c.lock_on),
        ] {
            assert_ne!(off, on, "control.{slot}'s two states are the same glyph");
        }
        for (slot, a, b) in [
            ("map", c.map_show as u32, c.map_hide as u32),
            ("band", c.band_show as u32, c.band_hide as u32),
            ("guidance", c.guidance_off as u32, c.guidance_on as u32),
            ("lock", c.lock_off as u32, c.lock_on as u32),
        ] {
            // `fa-`/`cod-` live in the 0xE000 private-use block, `md-` above
            // 0xF0000; a pair straddling that line is two families.
            assert_eq!(
                a >= 0xF_0000, b >= 0xF_0000,
                "control.{slot}'s two states come from different icon families",
            );
        }
        // …and the render mode's three are one family too.
        for ch in [c.render_hybrid, c.render_raster, c.render_extended] {
            assert!(ch as u32 >= 0xF_0000, "the render icons are all Material Design");
        }
        for (slot, ch) in [
            ("map_show", c.map_show), ("map_hide", c.map_hide),
            ("band_show", c.band_show), ("band_hide", c.band_hide),
            ("guidance_on", c.guidance_on), ("guidance_off", c.guidance_off),
            ("render_hybrid", c.render_hybrid), ("render_raster", c.render_raster),
            ("render_extended", c.render_extended),
            ("lock_on", c.lock_on), ("lock_off", c.lock_off),
            ("return_probe", c.return_probe),
        ] {
            assert!(!is_wide_estimate(ch), "control.{slot} = {ch:?} estimates as double-width");
        }
        // The return probe is Material Design like the rest of the `md-` set, and
        // it is exempt from the two-states rule above because it HAS one state:
        // its off-reading is the muted colour, not a second glyph (SQ-0785).
        assert!(c.return_probe as u32 >= 0xF_0000, "md-shoe_print is Material Design");
    }

    /// The PLAIN defaults must be drawable by an ordinary monospace face, so
    /// every one of them comes out of Geometric Shapes — the block the map
    /// already requires (see `default_portal_in_out_come_from_geometric_shapes`).
    /// And each toggle's two states must actually differ, or the icon says
    /// nothing and only the colour is left carrying it.
    #[test]
    fn plain_control_glyphs_are_geometric_shapes_and_tell_their_states_apart() {
        let c = ControlGlyphs::preset("plain").expect("the default preset");
        assert_eq!(c, SymbolSet::default().controls);
        let shapes = 0x25A0..=0x25FF;
        for (slot, ch) in [
            ("map_show", c.map_show), ("map_hide", c.map_hide),
            ("band_show", c.band_show), ("band_hide", c.band_hide),
            ("guidance_on", c.guidance_on), ("guidance_off", c.guidance_off),
            ("render_hybrid", c.render_hybrid), ("render_raster", c.render_raster),
            ("render_extended", c.render_extended),
            ("lock_on", c.lock_on), ("lock_off", c.lock_off),
            ("return_probe", c.return_probe),
        ] {
            assert!(shapes.contains(&(ch as u32)), "control.{slot} = {ch:?} is outside Geometric Shapes");
            assert!(!is_wide_estimate(ch), "control.{slot} = {ch:?} estimates as double-width");
        }
        assert_ne!(c.map_show, c.map_hide);
        assert_ne!(c.band_show, c.band_hide);
        assert_ne!(c.guidance_on, c.guidance_off);
        assert_ne!(c.lock_on, c.lock_off);
        // Three render modes, three distinct glyphs.
        let modes = [c.render_hybrid, c.render_raster, c.render_extended];
        assert_eq!(modes.iter().collect::<std::collections::HashSet<_>>().len(), 3);
        // …and the return probe's single mark is not any of the others, so it is
        // still legible as its own control on a border that draws several
        // (SQ-0785). It has no second state by design — the colour carries that.
        let all = [
            c.map_show, c.map_hide, c.band_show, c.band_hide, c.guidance_on, c.guidance_off,
            c.render_hybrid, c.render_raster, c.render_extended, c.lock_on, c.lock_off,
        ];
        assert!(!all.contains(&c.return_probe), "the footprint is its own mark");
    }

    /// The story badges' nerdfont set is three named icons, each codepoint read
    /// from the patched font's own `cmap` under the font's own glyph name and
    /// confirmed to rasterise with ink — never taken from a Nerd Fonts cheat
    /// sheet (SQ-1045's rule, SQ-1141's method, SQ-1159's set, SQ-1160's cut).
    ///
    /// This pins the numbers because nothing else can. There is no assertion on
    /// our side of the terminal that could notice a wrong icon drawn crisply and
    /// confidently, and a badge that comes out as tofu is worse than the letter
    /// it replaced — the row would say nothing at all where a mark belongs.
    #[test]
    fn nerdfont_badge_glyphs_are_the_names_that_were_read_from_the_font() {
        let b = StoryBadges::preset("nerdfont").expect("preset");
        for (name, got, want) in [
            ("md-content_save_outline", b.save, '\u{F0818}'),
            ("md-lightbulb_on", b.hint, '\u{F06E8}'),
            ("md-lightbulb_on_outline", b.hint_available, '\u{F06E9}'),
        ] {
            assert_eq!(got, want, "{name} moved: U+{:05X} is not U+{:05X}", got as u32, want as u32);
        }
        // One family for the whole set — Material Design, above U+F0000 — so the
        // three share a stroke weight and cap height and a row of them does not
        // look assembled out of parts. It is also what lets the font check's
        // sample row stand in for them: it already samples MDI.
        for (slot, ch) in [
            ("save", b.save), ("hint", b.hint), ("hint_available", b.hint_available),
        ] {
            assert!(ch as u32 >= 0xF_0000, "badge.{slot} = U+{:05X} is not Material Design", ch as u32);
            assert!(!is_wide_estimate(ch), "badge.{slot} = {ch:?} estimates as double-width");
        }
        // The hint pair says two different things, and its two states stay one
        // shape apart in FILL alone (adjacent codepoints in MDI's
        // filled/outline convention) — that is the whole distinction being
        // drawn between a hint you have and one you could get.
        assert_ne!(b.hint, b.hint_available, "the hint's two states are the same glyph");
        assert_eq!(
            b.hint_available as u32,
            b.hint as u32 + 1,
            "md-lightbulb_on_outline is no longer the filled bulb's own outline",
        );
        // Three badges, three distinct marks: two rows cannot say the same thing.
        let all = [b.save, b.hint, b.hint_available];
        assert_eq!(all.iter().collect::<std::collections::HashSet<_>>().len(), 3);
    }

    /// The PLAIN badges are the letters the picker has always drawn, and they
    /// are the source the `config::default_badge_*` values are taken FROM —
    /// so a default cannot drift from the preset that is supposed to hold it.
    #[test]
    fn plain_badges_are_the_letters_and_the_config_defaults_come_from_them() {
        let b = StoryBadges::preset("plain").expect("the default preset");
        assert_eq!(b, StoryBadges::PLAIN);
        assert_eq!((b.save, b.hint, b.hint_available), ('S', 'H', 'h'));
        for (slot, got, want) in [
            ("save", crate::config::default_badge_save(), b.save),
            ("hint", crate::config::default_badge_hint(), b.hint),
            ("hint_available", crate::config::default_badge_hint_available(), b.hint_available),
        ] {
            assert_eq!(got, want.to_string(), "default_badge_{slot} is not the plain preset's glyph");
        }
        // Every plain badge must be drawable by an ordinary monospace face —
        // which is what "plain" promises — so all three are bare ASCII.
        for (slot, ch) in [
            ("save", b.save), ("hint", b.hint), ("hint_available", b.hint_available),
        ] {
            assert!(ch.is_ascii_alphanumeric(), "badge.{slot} = {ch:?} is not plain ASCII");
        }
    }

    /// Both badge presets resolve, and `preset_names` names exactly them — the
    /// same contract `preset_names_cover_all_known_presets` holds the map's
    /// families to.
    #[test]
    fn badge_preset_names_cover_all_known_presets() {
        for name in StoryBadges::preset_names() {
            assert!(StoryBadges::preset(name).is_some(), "preset_names lists {name:?}, which does not resolve");
        }
        assert_eq!(StoryBadges::preset_names(), &["plain", "nerdfont"]);
        assert!(StoryBadges::preset("no-such-set").is_none());
    }

    /// Every control slot is themeable one glyph at a time, the way every other
    /// family is: a key `apply_override` silently ignores is a knob that does
    /// nothing (SQ-0558).
    #[test]
    fn every_control_slot_accepts_an_override() {
        let baseline = SymbolSet::resolve(&crate::config::SymbolConfig::default());
        for key in [
            "control.map_show", "control.map_hide", "control.band_show", "control.band_hide",
            "control.guidance_on", "control.guidance_off", "control.render_hybrid",
            "control.render_raster", "control.render_extended", "control.lock_on",
            "control.lock_off",
        ] {
            let mut cfg = crate::config::SymbolConfig::default();
            cfg.overrides.insert(key.into(), "#".into());
            assert_ne!(SymbolSet::resolve(&cfg), baseline, "override {key} changed nothing");
        }
    }

    /// Every MAP control slot is themeable one glyph at a time too, and BOTH
    /// halves of each two-mode pair are (SQ-1148).
    ///
    /// That is not decoration. The plain preset spells one mark in both slots of
    /// a pair — `#`/`#`, `M`/`M` — because ASCII has no off-shape for either, so
    /// the second key is how a player who wants a shape change gets one
    /// (`md-numeric_off` U+F19D3 and `md-grid_off` U+F02C2 being the obvious
    /// reaches). A pair whose off half no key can address would be a duplicate
    /// with no purpose, and the next reader would rightly delete it.
    #[test]
    fn every_map_control_slot_accepts_an_override_including_both_halves_of_a_pair() {
        let baseline = SymbolSet::resolve(&crate::config::SymbolConfig::default());
        for key in [
            "map_control.room_numbers_on", "map_control.room_numbers_off",
            "map_control.centre", "map_control.zoom_out", "map_control.zoom_in",
            "map_control.view_matrix", "map_control.view_drawn",
        ] {
            let mut cfg = crate::config::SymbolConfig::default();
            cfg.overrides.insert(key.into(), "@".into());
            let got = SymbolSet::resolve(&cfg);
            assert_ne!(got, baseline, "override {key} changed nothing");
            // …and it moved ONLY its own slot, so the two halves are independent
            // rather than one key wearing two names.
            let mut want = baseline.clone();
            match key {
                "map_control.room_numbers_on" => want.map_controls.room_numbers_on = '@',
                "map_control.room_numbers_off" => want.map_controls.room_numbers_off = '@',
                "map_control.centre" => want.map_controls.centre = '@',
                "map_control.zoom_out" => want.map_controls.zoom_out = '@',
                "map_control.zoom_in" => want.map_controls.zoom_in = '@',
                "map_control.view_matrix" => want.map_controls.view_matrix = '@',
                "map_control.view_drawn" => want.map_controls.view_drawn = '@',
                _ => unreachable!(),
            }
            assert_eq!(got, want, "override {key} reached a slot that is not its own");
        }
    }

    #[test]
    fn nf_arrow_presets_exist_and_are_single_width() {
        for name in ["nf-bold","nf-box","nf-circle","nf-outline"] {
            assert!(Arrows::preset_names().contains(&name), "{name} missing");
            let a = Arrows::preset(name).expect("preset");
            for ch in [a.north,a.south,a.east,a.west,a.ne,a.nw,a.se,a.sw] {
                assert!(!is_wide_estimate(ch), "{name}: wide char {:?}", ch);
            }
        }
        // verified cardinal codepoints for nf-bold:
        let b = Arrows::preset("nf-bold").unwrap();
        assert_eq!(b.north, '\u{F0737}');
        assert_eq!(b.south, '\u{F072E}');
        assert_eq!(b.east,  '\u{F0734}');
        assert_eq!(b.west,  '\u{F0731}');
        // nf-box native diagonals:
        let bx = Arrows::preset("nf-box").unwrap();
        assert_eq!(bx.ne, '\u{F196A}');
        assert_eq!(bx.nw, '\u{F1968}');
        assert_eq!(bx.se, '\u{F1966}');
        assert_eq!(bx.sw, '\u{F1964}');
    }
}
