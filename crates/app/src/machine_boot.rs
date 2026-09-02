//! The per-machine facts a faithful Version 6 boot needs, resolved once
//! (SQ-1022).
//!
//! # Why this is a value and not a documented recipe
//!
//! CLAUDE.md states the chain in prose — "boot a harness the way `startup.rs`
//! boots, or you measure a screen the app never draws" — and every caller has
//! been expected to follow it by hand. Four have not, and the failures are
//! identical each time: the numbers stay entirely self-consistent and describe a
//! screen the player never sees.
//!
//! * SQ-0901 found `ring_scout` and `v6_side_border_tiling` both omitting
//!   `native_std_window`, so Journey r77 and Arthur r63 — **560x384** presses —
//!   were measured at 640x400. That produced a fabricated Arthur frame which a
//!   whole quest was then fixed and tested against.
//! * SQ-1020 found `ring_scout` omitting the Version 6 CELL, so every Macintosh
//!   frame it ever reported was laid out on 8x16 where the app uses 7x15 — in the
//!   instrument built to catch SQ-0901.
//! * SQ-1021 found the same omission across twelve Macintosh render harnesses.
//!
//! The clearest evidence that a recipe cannot hold this is in the two places that
//! got it RIGHT. `reset.rs` carries the comment "The same four links `startup.rs`
//! resolves, in the same order" — an invariant maintained across files by hand,
//! by someone who saw the hazard and had no way to express it. And
//! `v6_mac_pillar_feet`'s harness resolves the profile from its medium, gets all
//! four `std_window` links right, takes the palette lock correctly — and passes
//! `None` for the cell, because the cell was added to the chain after that
//! harness was written.
//!
//! **A recipe grows and its copies do not.** So the chain is a value: a caller
//! cannot omit a step because there are no steps to perform.
//!
//! # What it does and does not own
//!
//! It derives the three facts that keep getting dropped — the standard window,
//! the art scale, and the cell. It TAKES the three that a caller legitimately
//! owns, because deciding them involves policy this module has no business in:
//!
//! * the **profile**, which must come from the medium the MOUNT returned rather
//!   than be re-derived from the path (SQ-0876 — a hybrid disc carries DOS builds
//!   in a Macintosh volume, and answering "HFS" for all of them told every PC
//!   story it was a Macintosh);
//! * the **interpreter number** and **default colours**, which pass through
//!   `Config::advertised_interpreter_number` and the two-colour-card rule
//!   (SQ-0930, SQ-0956) — at launch AND at a restart, through the same call in
//!   both places. This line used to say a restart carried "simply what the launch
//!   settled", which was never true: the launch's answer is not stored anywhere,
//!   and `reset.rs` re-derived it by hand with rung 2 of that cascade missing
//!   (SQ-1058). A restart re-asks, and re-asks the one implementation;
//! * the **faces** the cascade resolves (`crate::native_font::resolve`) — the
//!   release's own off the story's medium, then the machine's system face off a
//!   boot disk the player supplied — because reaching them needs the story path,
//!   the disc entry, how the profile was decided and where the player keeps their
//!   media: facts a caller owns and this module never sees.
//!
//! The face is TAKEN and the CELL is derived from it (SQ-1009). A proportional
//! typeface off a release disk states its own line height, so on Arthur's Amiga
//! floppy the declared cell is 8x20 rather than the machine table's 8x16 — and that
//! is precisely a fact three files were settling by hand, which is what put
//! `reset.rs` a grid apart from `startup.rs` in SQ-1022.

use crate::graphics::PictSource;
use crate::interpreter::InterpreterProfile;

/// Everything `GameSession` needs to be told about the machine it is running on.
///
/// Construct with [`MachineBoot::resolve`]; never assemble the fields by hand,
/// which is the practice this type exists to end.
#[derive(Debug, Clone, PartialEq)]
pub struct MachineBoot {
    /// The machine, as the medium named it.
    pub profile: InterpreterProfile,
    /// Header `$1E`, or `None` to leave zvm's own default rule in force.
    pub interpreter_number: Option<u8>,
    /// The screen the story is TOLD it has, in native pixels — [`Self::resolve`]'s
    /// four-link cascade.
    pub screen_px: Option<(u16, u16)>,
    /// How dense the artwork is (SQ-0790): a 320-wide rendition doubles onto the
    /// unit screen at (2, 2), an EGA/CGA one is 640 wide with half-width pixels
    /// and arrives at (1, 2). `None` for every Blorb-sourced story.
    pub art_scale: Option<(u32, u32)>,
    /// §8.3.3's pair, where the machine or the card states one.
    pub default_colours: Option<(u8, u8)>,
    /// May this launch present its machine's per-machine SCREEN RULES?
    /// [`crate::config::Config::machine_colours_licensed`] (SQ-1154).
    ///
    /// A field of its own rather than `default_colours.is_some()`. That proxy
    /// happens to agree today and would break the moment a machine claims a rule
    /// without stating a pair — the Atari ST and the IBM PC already claim no
    /// screen page at all, so the two facts are plainly independent.
    ///
    /// It rides HERE, in the value [`Self::resolve`] hands to
    /// [`crate::session::GameSession::new_for_machine`], for the reason this whole
    /// module exists: a re-seeding site that forgets it is a screen the player
    /// never sees. `resolve` takes it as a required parameter so the compiler
    /// enumerates those sites — `startup.rs` and `reset.rs` — instead of somebody
    /// remembering (SQ-1022).
    pub machine_colours_licensed: bool,
    /// The Version 6 character cell this machine declares (SQ-0917) — 7x15 on a
    /// Macintosh, 8x16 everywhere else. The machine table's, since SQ-1013, and
    /// the admitted FACE's where the release shipped a proportional one
    /// ([`crate::native_font::declared_cell`], SQ-1009).
    pub cell: zvm::screen::V6Cell,
    /// How this machine decides what a Version 6 window does with text that
    /// reaches its right margin (SQ-1071) — see
    /// [`zvm::interpreter::V6WrapRegime`]. Beside the cell for the same reason:
    /// a harness that resolves the machine must not be able to omit it.
    pub wrap_regime: zvm::interpreter::V6WrapRegime,
    /// The typefaces this machine draws with — its body face and its fixed-pitch
    /// alternate, resolved through [`crate::native_font::resolve`]'s cascade: the
    /// release's own medium first, then the machine's own system face off a boot
    /// disk the player supplied (SQ-1037). Empty for every launch that reaches
    /// neither, which is every machine but the Macintosh and Arthur's Amiga floppy.
    pub faces: crate::native_font::FaceSet,
}

impl MachineBoot {
    /// Resolve the machine's facts from an already-mounted archive.
    ///
    /// `named_art_std_window` is [`crate::graphics::PictureOverride::std_window`],
    /// read BEFORE the override is consumed by `PictSource::resolve_with_override`
    /// — it is the second link below and the one `ring_scout` dropped.
    ///
    /// # The four links, and why they are in this order
    ///
    /// SQ-0837/SQ-0838. The ARCHIVE comes before the MACHINE because Infocom's own
    /// Macintosh interpreter chose its window and its picture file in one decision
    /// — "for a small window use mono gfx, for a big window use color gfx" — so a
    /// mono `Pic.data` mounted off a Mac volume states the 480x300 std-Mac screen
    /// it was drawn for. It disturbs no other medium: for an `.adf` the archive and
    /// the Amiga profile give the same 320x200, and a story with no native archive
    /// falls through to the machine exactly as it always did.
    ///
    /// 1. the container's own `Reso` chunk, if it declares one;
    /// 2. the archive the PLAYER named, if they named one;
    /// 3. the native archive's own picture space — the link SQ-0901 was about;
    /// 4. the machine's standard window.
    pub fn resolve(
        profile: InterpreterProfile,
        picts: &PictSource,
        named_art_std_window: Option<(u16, u16)>,
        interpreter_number: Option<u8>,
        default_colours: Option<(u8, u8)>,
        machine_colours_licensed: bool,
        faces: crate::native_font::FaceSet,
    ) -> MachineBoot {
        let art_scale = picts.art_scale();
        MachineBoot {
            profile,
            interpreter_number,
            machine_colours_licensed,
            wrap_regime: profile.v6_wrap_regime(),
            screen_px: picts
                .std_window()
                .or(named_art_std_window)
                .or_else(|| picts.native_std_window())
                .or_else(|| profile.std_window()),
            art_scale,
            default_colours,
            // SQ-1009: the cell follows the face, so this is one call rather than
            // a rule each of `startup.rs`, `reload.rs` and `reset.rs` remembers.
            cell: crate::native_font::declared_cell(
                profile,
                &faces,
                art_scale.unwrap_or((1, 1)),
            ),
            faces,
        }
    }

    /// The cell, the face and the pen as ONE value, for the renderer (SQ-1009).
    ///
    /// [`crate::state::AppState`] holds this rather than the two halves, so the
    /// draw paths cannot be handed a cell from one boot and a face from another.
    pub fn text_face(&self) -> crate::native_font::TextFace {
        crate::native_font::TextFace::new(self.profile, self.faces.clone(), self.art_scale)
    }

    /// A story with no medium behind it — a bare `.z5`, or a Blorb.
    ///
    /// Every machine fact is absent and the cell is the universal 8x16, which is
    /// what [`crate::session::GameSession::new_with_trace`] already passes. Named
    /// so a test that genuinely has no machine says so, instead of looking like
    /// one that forgot.
    pub fn bare() -> MachineBoot {
        MachineBoot {
            profile: InterpreterProfile::IbmPc,
            interpreter_number: None,
            screen_px: None,
            art_scale: None,
            default_colours: None,
            // No medium named a machine, so this launch presents none — which is
            // also what every per-machine screen rule asks. Inert either way here,
            // since `interpreter_number: None` leaves `$1E` at zvm's own default
            // (6 for Version 6) and no rule claims that number; stated rather than
            // left to that coincidence.
            machine_colours_licensed: false,
            cell: InterpreterProfile::IbmPc.v6_font_cell(),
            wrap_regime: InterpreterProfile::IbmPc.v6_wrap_regime(),
            faces: crate::native_font::FaceSet::none(),
        }
    }
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;

    /// The cell rides along, which is the whole point (SQ-1021).
    ///
    /// Stated against the PROFILE rather than against a literal, so this holds
    /// when a machine's declared cell changes — SQ-1009 may yet move the Amiga's.
    #[test]
    fn the_cell_is_the_machines_and_never_has_to_be_remembered() {
        for profile in
            [InterpreterProfile::Macintosh, InterpreterProfile::Amiga, InterpreterProfile::IbmPc]
        {
            let boot = MachineBoot::resolve(
                profile,
                &PictSource::new(None),
                None,
                None,
                None,
                true,
                crate::native_font::FaceSet::none(),
            );
            assert_eq!(
                boot.cell,
                profile.v6_font_cell(),
                "{profile:?}: the cell comes from the machine, not from the caller",
            );
        }
        // And the one that motivated all of this is genuinely not 8x16.
        assert_eq!(
            MachineBoot::resolve(
                InterpreterProfile::Macintosh,
                &PictSource::new(None),
                None,
                None,
                None,
                true,
                crate::native_font::FaceSet::none(),
            )
            .cell,
            zvm::interpreter::MACINTOSH_V6_CELL,
            "a Macintosh boot carries 7x15 without the caller doing anything",
        );
    }

    /// A caller that names an archive gets that archive's screen, and one that
    /// does not falls through to the machine (SQ-0901's link).
    #[test]
    fn a_named_archive_outranks_the_machines_standard_window() {
        let bare = PictSource::new(None);
        let machine = MachineBoot::resolve(
            InterpreterProfile::Amiga,
            &bare,
            None,
            None,
            None,
            true,
            crate::native_font::FaceSet::none(),
        );
        assert_eq!(
            machine.screen_px,
            InterpreterProfile::Amiga.std_window(),
            "no archive says anything, so the machine answers",
        );
        let named = MachineBoot::resolve(
            InterpreterProfile::Amiga,
            &bare,
            Some((560, 384)),
            None,
            None,
            true,
            crate::native_font::FaceSet::none(),
        );
        assert_eq!(
            named.screen_px,
            Some((560, 384)),
            "the archive the player named outranks the machine — the link ring_scout dropped",
        );
    }

    /// `bare()` is the no-machine case, and says so.
    #[test]
    fn a_bare_story_carries_no_machine_facts_and_the_universal_cell() {
        let b = MachineBoot::bare();
        assert_eq!(
            (b.interpreter_number, b.screen_px, b.art_scale, b.default_colours),
            (None, None, None, None),
        );
        assert_eq!(b.cell, zvm::screen::V6Cell::DEFAULT, "the cell every machine but the Macintosh declares");
    }
}
