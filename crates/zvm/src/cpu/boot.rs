//! Everything a host must tell a [`Machine`] before the story runs, as one
//! value (SQ-1396).
//!
//! # Why this is a value and not a documented recipe
//!
//! Booting a Z-machine faithfully is not one call; it is a dozen setters in an
//! order the crate never stated. Three classes of setter have to be told apart,
//! and the only way to tell them apart used to be reading each doc comment and
//! noticing which of them mentioned [`Machine::init_caps`]:
//!
//! * those that **write the header immediately**, so a not-yet-run story sees
//!   the capability ([`Machine::set_honor_game_colours`],
//!   [`Machine::set_sound_available`], [`Machine::set_default_colours`]);
//! * those **latched until `init_caps`**, which is where the header capability
//!   bytes are actually stamped ([`Machine::set_interpreter_number`] for `$1E`,
//!   [`Machine::set_interpreter_version`] for `$1F`);
//! * those that touch no header bit but must nevertheless precede the **boot
//!   run**, because the story's own initialisation reads them
//!   ([`Machine::set_rng_seed`] — an initialisation routine may already draw
//!   from the generator; [`Machine::set_picture_dims`] — a Version 6 story calls
//!   `picture_data` while booting).
//!
//! …and one that must come *after* `init_caps`, because `init_caps` seeds a
//! generic 80x24 over the top of it: the screen size.
//!
//! The whole ordering lived in a private function's doc comment in lanthorn's
//! `app` crate, and the second embedder in this very workspace (`zvm-cli`)
//! reproduced part of it and diverged on the rest. That is the shape
//! `CLAUDE.md`'s refactoring policy names outright — "facts that must be
//! considered together should travel together as a value, not positionally", and
//! "a hand-maintained invariant across files is the symptom; the cure is a type".
//!
//! So the recipe is a value and [`Machine::boot`] performs it. The individual
//! setters remain public for a MID-RUN change — a host that resizes its window,
//! or lets the player toggle colours — which is what they are actually good at.
//!
//! ```no_run
//! # fn entropy() -> u32 { 0 }
//! use zvm::cpu::exec::{BootConfig, Machine};
//! use zvm::memory::Memory;
//! let mem = Memory::new(std::fs::read("story.z5").unwrap()).unwrap();
//! let mut m = Machine::boot(
//!     mem,
//!     Box::new(zvm::io::BufferOutput::new()),
//!     BootConfig::new()
//!         .with_honor_game_colours(true)
//!         .with_rng_seed(entropy())
//!         .with_screen_grid(24, 80),
//! );
//! // `m` is ready for its first `step()`.
//! ```

use crate::cpu::exec::Machine;
use crate::screen::{Palette, V6Metric};

/// The facts a [`Machine`] is told once, before the story runs.
///
/// Construct with [`BootConfig::new`] and refine with the `with_*` builders; a
/// bare `BootConfig::new()` boots a story exactly as [`Machine::with_output`]
/// followed by [`Machine::init_caps`] always has, so a host that has no opinion
/// about a fact simply does not state one.
///
/// `#[non_exhaustive]` because the list has grown four times in this project's
/// life (the interpreter version, the palette, the Version 6 cell, the art
/// scale) and will grow again: a host that builds one through `new()` keeps
/// compiling when it does.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BootConfig {
    honor_game_colours: bool,
    sound_available: bool,
    rng_seed: Option<u32>,
    default_colours: Option<(u8, u8)>,
    picture_dims: Vec<(u16, u16, u16)>,
    interpreter_number: Option<u8>,
    interpreter_version: Option<u8>,
    palette: Palette,
    v6_text: Option<V6Metric>,
    v6_screen_px: Option<(u16, u16)>,
    v6_art_scale: Option<(u32, u32)>,
    screen_grid: Option<(u8, u8)>,
}

impl Default for BootConfig {
    fn default() -> BootConfig {
        BootConfig::new()
    }
}

impl BootConfig {
    /// How far the artwork is blown up on its way onto the Version 6 unit
    /// screen when the host does not say (SQ-0479/SQ-0790): the ×2 of Frotz's
    /// Amiga/DOS profile, which is every rendition in the corpus but the
    /// Macintosh's 1:1 monochrome plate and the half-width pixels of EGA/CGA.
    pub const DEFAULT_V6_ART_SCALE: (u32, u32) = (2, 2);

    /// The Version 6 picture space assumed when no archive declares one — the
    /// standard window of Infocom's own v6 renditions.
    pub const DEFAULT_V6_PICTURE_SPACE: (u16, u16) = (320, 200);

    /// A boot that states nothing: what [`Machine::with_output`] plus
    /// [`Machine::init_caps`] have always produced.
    pub fn new() -> BootConfig {
        BootConfig {
            honor_game_colours: false,
            sound_available: false,
            rng_seed: None,
            default_colours: None,
            picture_dims: Vec::new(),
            interpreter_number: None,
            interpreter_version: None,
            palette: Palette::Standard,
            v6_text: None,
            v6_screen_px: None,
            v6_art_scale: None,
            screen_grid: None,
        }
    }

    /// Honour the story's own `set_colour` calls, advertising the Flags1 colour
    /// bit (ZMSD §11.1). See [`Machine::set_honor_game_colours`].
    pub fn with_honor_game_colours(mut self, on: bool) -> BootConfig {
        self.honor_game_colours = on;
        self
    }

    /// Advertise sound-effect capability in the header bits (ZMSD §11.1). See
    /// [`Machine::set_sound_available`].
    pub fn with_sound_available(mut self, on: bool) -> BootConfig {
        self.sound_available = on;
        self
    }

    /// Seed the `random` PRNG (ZMSD §2.4). Unstated leaves
    /// [`Machine::DEFAULT_RNG_SEED`], so a host that wants a reproducible
    /// sequence gets one by saying nothing.
    pub fn with_rng_seed(mut self, seed: u32) -> BootConfig {
        self.rng_seed = Some(seed);
        self
    }

    /// The host's own `(background, foreground)` §8.3.1 standard colour pair for
    /// header `$2C`/`$2D` (ZMSD §8.3.3). Unstated leaves zvm's §8.3.2
    /// black-on-white seed. See [`Machine::set_default_colours`].
    pub fn with_default_colours(mut self, bg: u8, fg: u8) -> BootConfig {
        self.default_colours = Some((bg, fg));
        self
    }

    /// The Version 6 `Pict` dimension table `picture_data` answers from, in the
    /// ART's own pixels — [`Self::with_v6_art_scale`] is applied to it on the way
    /// in, because a story lays out in the unit screen's coordinates and not the
    /// archive's (SQ-0479).
    pub fn with_picture_dims(mut self, dims: Vec<(u16, u16, u16)>) -> BootConfig {
        self.picture_dims = dims;
        self
    }

    /// Header `$1E`, the interpreter number (ZMSD §11.1.3). `None` — the default
    /// — leaves zvm's own rule. Latched until `init_caps`.
    pub fn with_interpreter_number(mut self, n: Option<u8>) -> BootConfig {
        self.interpreter_number = n;
        self
    }

    /// Header `$1F`, the interpreter version (ZMSD §11.1.3.1). `None` — the
    /// default — leaves zvm's own `b'A'`. Latched until `init_caps`.
    pub fn with_interpreter_version(mut self, v: Option<u8>) -> BootConfig {
        self.interpreter_version = v;
        self
    }

    /// The table a standard colour NUMBER resolves through on this machine
    /// (§8.3.1's own, unless the host is presenting particular hardware).
    pub fn with_palette(mut self, p: Palette) -> BootConfig {
        self.palette = p;
        self
    }

    /// The Version 6 character cell and pen together (ZMSD §8.8.3.2, SQ-0917 /
    /// SQ-1009). Applied before the screen is sized and before the boot run: the
    /// story reads `$26`/`$27` and lays its windows out from them, so a cell that
    /// arrives later is one the game has already disagreed with. Unstated keeps
    /// zvm's 8x16, which is every machine but the Macintosh.
    pub fn with_v6_text(mut self, metric: V6Metric) -> BootConfig {
        self.v6_text = Some(metric);
        self
    }

    /// The Version 6 PICTURE SPACE this story's artwork is drawn in — a Blorb
    /// `Reso` standard window, or a native archive's own. The screen the story is
    /// told it has is this times [`Self::with_v6_art_scale`] (SQ-0838). Unstated
    /// means no archive declared one, and per Blorb §11 nothing is scalable: the
    /// picture table is left at art-native size.
    pub fn with_v6_screen_px(mut self, space: (u16, u16)) -> BootConfig {
        self.v6_screen_px = Some(space);
        self
    }

    /// How dense the artwork is, per axis (SQ-0790) — a 320-wide rendition
    /// doubles onto the unit screen at `(2, 2)`, an EGA/CGA one is 640 wide with
    /// half-width pixels and arrives at `(1, 2)`, a Macintosh monochrome plate is
    /// displayed 1:1. Unstated is [`Self::DEFAULT_V6_ART_SCALE`].
    pub fn with_v6_art_scale(mut self, scale: (u32, u32)) -> BootConfig {
        self.v6_art_scale = Some(scale);
        self
    }

    /// The screen the story is told it has, as a CHARACTER GRID — header `$20` /
    /// `$21` (ZMSD §8.4). This is the unit for every version but 6, whose screen
    /// is [`Self::with_v6_screen_px`]'s pixels; it is ignored there.
    ///
    /// Unstated leaves the generic 80x24 `init_caps` seeds. State the real pane
    /// if you have one: a v4/v5 status routine lays itself out ONCE, at boot, and
    /// never re-reads `$21` (SQ-0679/SQ-0680).
    pub fn with_screen_grid(mut self, rows: u8, cols: u8) -> BootConfig {
        self.screen_grid = Some((rows, cols));
        self
    }

    /// The character grid this config states, if any — [`Self::with_screen_grid`]
    /// read back, for a host that records what it declared at boot.
    pub fn screen_grid(&self) -> Option<(u8, u8)> {
        self.screen_grid
    }

    /// The scale actually applied to this story's ART, at this story's version.
    ///
    /// Not simply [`Self::with_v6_art_scale`]'s value: below Version 6 there is
    /// no art to scale, and per Blorb §11 an archive that declares no standard
    /// window has no scalable images either — "non-scalable images are shown at
    /// their actual size" — so the absence of [`Self::with_v6_screen_px`] is the
    /// spec's own signal for 1:1. A host that composites the pictures itself
    /// needs the same number [`Machine::boot`] scaled the table by, and this is
    /// it.
    pub fn resolved_art_scale(&self, version: u8) -> (u32, u32) {
        if version == 6 && self.v6_screen_px.is_some() {
            self.v6_art_scale.unwrap_or(Self::DEFAULT_V6_ART_SCALE)
        } else {
            (1, 1)
        }
    }

    /// The Version 6 screen in native pixels this config declares: the picture
    /// space at the scale this machine drew it (SQ-0838).
    ///
    /// Absent a declared picture space there is nothing to scale, so the uniform
    /// rule stands over the default space and the answer is the 640x400 every
    /// Blorb-less v6 story (scopa, mysterious01) has always booted at. Note that
    /// this is deliberately NOT [`Self::resolved_art_scale`], which answers 1:1
    /// in that same case: an undeclared picture space means nothing to SCALE, not
    /// a smaller SCREEN.
    fn v6_screen_dims(&self) -> (u16, u16) {
        let (art_w, art_h) = self.v6_screen_px.unwrap_or(Self::DEFAULT_V6_PICTURE_SPACE);
        let scale = match (self.v6_screen_px, self.v6_art_scale) {
            (Some(_), Some(s)) => s,
            _ => Self::DEFAULT_V6_ART_SCALE,
        };
        (
            art_w.saturating_mul(scale.0.max(1) as u16),
            art_h.saturating_mul(scale.1.max(1) as u16),
        )
    }

    /// Apply every fact in the one correct order. [`Machine::boot`] is the only
    /// caller and the only door.
    pub(crate) fn apply(mut self, m: &mut Machine) {
        let version = m.mem.version();
        // Both derived from fields moved out below, so read them first.
        let art_scale = self.resolved_art_scale(version);
        let screen_px = self.v6_screen_dims();
        // FIRST, and before `init_caps`: the machine's own colour table and its
        // `$1F` byte. `init_caps` latches the version byte and writes the true
        // default colours THROUGH the table, and `set_default_colours` below
        // resolves through it too, so both have to be in force already (SQ-1393).
        m.set_palette(self.palette);
        m.set_interpreter_version(self.interpreter_version);
        // These two write the header on the spot, so their position is a matter
        // of being before the boot run rather than before `init_caps`.
        m.set_honor_game_colours(self.honor_game_colours);
        m.set_sound_available(self.sound_available);
        // Before the boot run: a story's initialisation routine may already draw
        // from the generator, so seeding after the first prompt is one turn too
        // late to change the game the player is handed (SQ-0811).
        if let Some(seed) = self.rng_seed {
            m.set_rng_seed(seed);
        }
        // Before `init_caps`, which re-applies whatever pair is current over its
        // own 2/9 seed — and before the boot run, because a game that reads the
        // header default pair while booting (Beyond Zork picks its colour scheme
        // there) must already see the host's real page and ink (SQ-0532).
        if let Some((bg, fg)) = self.default_colours {
            m.set_default_colours(bg, fg);
        }
        // Before the screen is sized (which is measured in cells) and before the
        // boot run (the story reads `$26`/`$27` and lays its windows out from
        // them) — SQ-0917, SQ-1009.
        if version == 6 {
            if let Some(metric) = self.v6_text.take() {
                m.set_v6_text(metric);
            }
        }
        // Before the boot run: `picture_data` is called DURING boot by every v6
        // story. The table crosses into unit space here, once — Frotz's
        // Amiga/DOS interpreter likewise returns `scaler * size` for every
        // picture (SQ-0479, SQ-0715, SQ-0790).
        let picture_dims = if version == 6 {
            std::mem::take(&mut self.picture_dims)
                .into_iter()
                .map(|(n, w, h)| (n, w * art_scale.0 as u16, h * art_scale.1 as u16))
                .collect()
        } else {
            std::mem::take(&mut self.picture_dims)
        };
        m.set_picture_dims(picture_dims);
        // Latched: this one takes effect AT `init_caps`, not now.
        m.set_interpreter_number(self.interpreter_number);
        m.init_caps();
        // AFTER `init_caps`, which seeded a generic 80x24 over the top of
        // whatever the host had said.
        if version == 6 {
            // SQ-0917: hand the machine the PIXELS and let it derive the grid.
            // A grid multiplied back into `$22`/`$24` loses whatever the cell
            // does not divide — at the Macintosh's 7-wide cell, 640 comes back
            // as 637.
            m.set_v6_screen_px(screen_px.0, screen_px.1);
        } else if let Some((rows, cols)) = self.screen_grid {
            m.set_screen_dims(rows, cols);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::BufferOutput;
    use crate::memory::Memory;

    fn v5_story() -> Vec<u8> {
        crate::header::tests_support::sample_story(5)
    }

    /// A bare config boots a v5 story exactly as `with_output` + `init_caps` did.
    #[test]
    fn a_bare_config_is_with_output_plus_init_caps() {
        let mut old = Machine::with_output(
            Memory::new(v5_story()).expect("story"),
            Box::new(BufferOutput::new()),
        );
        old.init_caps();
        let new = Machine::boot(
            Memory::new(v5_story()).expect("story"),
            Box::new(BufferOutput::new()),
            BootConfig::new(),
        );
        assert_eq!(
            old.mem.raw_bytes()[..0x40],
            new.mem.raw_bytes()[..0x40],
            "the header a bare BootConfig produces is byte-for-byte the old recipe's",
        );
    }

    /// The screen is sized AFTER `init_caps`, which would otherwise seed 80x24
    /// over the top of it.
    #[test]
    fn the_grid_survives_init_caps() {
        let m = Machine::boot(
            Memory::new(v5_story()).expect("story"),
            Box::new(BufferOutput::new()),
            BootConfig::new().with_screen_grid(30, 50),
        );
        assert_eq!(m.mem.read_byte(0x20), 30, "rows survive init_caps");
        assert_eq!(m.mem.read_byte(0x21), 50, "columns survive init_caps");
    }

    /// Blorb §11: no declared picture space means no scalable images, so the
    /// table stays art-native — but the SCREEN is still the 640x400 the corpus
    /// has always booted at.
    #[test]
    fn an_undeclared_picture_space_scales_the_table_but_not_the_screen() {
        let cfg = BootConfig::new();
        assert_eq!(cfg.resolved_art_scale(6), (1, 1), "nothing to scale");
        assert_eq!(cfg.v6_screen_dims(), (640, 400), "the uniform screen still stands");
        let declared = BootConfig::new().with_v6_screen_px((480, 300)).with_v6_art_scale((1, 1));
        assert_eq!(declared.resolved_art_scale(6), (1, 1));
        assert_eq!(declared.v6_screen_dims(), (480, 300), "a Macintosh plate is 1:1");
        let doubled = BootConfig::new().with_v6_screen_px((320, 200));
        assert_eq!(doubled.resolved_art_scale(6), (2, 2), "the default rule");
        assert_eq!(doubled.v6_screen_dims(), (640, 400));
        assert_eq!(doubled.resolved_art_scale(5), (1, 1), "below v6 there is no art to scale");
    }
}
