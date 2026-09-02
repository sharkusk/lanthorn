//! The interpreter profile: which historical machine lanthorn presents itself
//! as (SQ-0719).
//!
//! A Version 6 story asks the header what it is running on and then behaves
//! differently — Zork Zero picks a whole colour scheme from it, Beyond Zork
//! swaps Font 3 arrows for CP437 character graphics. Byte `$1E` alone is not
//! enough to answer that question honestly, because the answer is a *bundle*:
//! the machine's screen, its interpreter number, the colours it says are its
//! defaults, and the palette its colour numbers name. Setting one of those in
//! isolation produces an incoherent machine — a byte that changes what games do
//! without changing the machine it implies — which is exactly what happened
//! when `interpreter_number = 4` was set by hand and the artwork kept its IBM PC
//! scale while the text turned white-on-grey.
//!
//! So the bundle is one named thing, with two members:
//!
//! - [`InterpreterProfile::IbmPc`] is **today's behaviour, named**. Nothing here
//!   is new: interpreter number by Frotz's rule (6 for v6, 1 otherwise), the
//!   Blorb `Reso` chunk as the standard window, default colours taken from the
//!   user's terminal, ZMSD §8.3.1 colour resolution, the 8×16 v6 cell. Every
//!   knob below returns "no opinion" for it, which is what makes it byte-for-byte
//!   what shipped.
//! - [`InterpreterProfile::Amiga`] is the sibling, for stories that came off
//!   Amiga media.
//! - [`InterpreterProfile::Macintosh`] is the third, for stories that came off
//!   an HFS volume (SQ-0838).
//! - [`InterpreterProfile::AtariSt`] is the fourth, for stories that came off a
//!   GEMDOS floppy (SQ-0835) — and the one that shows a profile may honestly
//!   **decline** a member of the bundle. It states a number, a default page and
//!   a palette, all read out of Infocom's own ST interpreters, and states no
//!   standard window, because Infocom never wrote a Version 6 interpreter for
//!   the ST and a standard window is a Version 6 art geometry.
//!
//!   That distinction is worth keeping straight, because it is the whole reason
//!   this profile was blocked for one commit. The bundle argument above is a
//!   warning against a number that **contradicts** the rest of the machine, and
//!   the ST corpus is where that cannot happen: all thirty-nine stories on the
//!   nine compilations are v3, v4 or v5, so there is no artwork for the number
//!   to disagree with. "I cannot verify every member" is an argument for
//!   declining the members you cannot verify, not for declining the ones you
//!   can — [`InterpreterProfile::IbmPc`] has answered `None` from
//!   [`default_colours`](InterpreterProfile::default_colours) all along.
//! - [`InterpreterProfile::AppleIIgs`] is the fifth, for stories that came off a
//!   ProDOS volume (SQ-0857), and it is the machine that makes "decline" a
//!   *judgement* rather than a shortage. Its number, its black page and its
//!   palette all come out of the Apple II YZIP — Infocom's own Version 6
//!   interpreter for the machine, which is not merely in the source archive but
//!   sitting on two of the disks in `stories/`. It states no standard window
//!   even so, because the Apple's Version 6 screen is 140x192 on a 3x9 cell and
//!   that is a different screen MODEL, not a resolution this knob can hold. See
//!   [`InterpreterProfile::std_window`].
//!
//!   It is also the profile whose NUMBER had to be argued rather than read. The
//!   Amiga, the Macintosh and the ST each write one byte and mean it; the Apple
//!   II YZIP *detects the machine at boot* and writes 2, 9 or 10 accordingly, so
//!   the medium genuinely cannot name the press. What settles it is that
//!   declining is not neutral: zvm's own rule would tell an Apple II story it is
//!   a DECSystem-20, or on Version 6 an IBM PC. §11.1.3 asks an interpreter to
//!   "choose the interpreter number most suitable for the machine it will run
//!   on", and of the three machines that YZIP will start on at all, the one a
//!   modern terminal resembles is the IIgs. [`blorb::medium`] carries the whole
//!   argument and the measurement behind it.
//!
//! **Selection**, most specific first (SQ-0734):
//!
//! 1. An explicit `interpreter_number` (config or `--interpreter`) — the
//!    number you name is the machine you are asking for, and it brings its whole
//!    profile with it.
//! 2. The ART: a picture archive named outright in the per-game sidecar
//!    (`pictures = "…"`, tier 3 of the resource policy — see
//!    [`crate::graphics::PictureOverride`]). Asking for a game's EGA rendition is
//!    asking for the IBM PC that drew it; asking for its `Pic.data` is asking for
//!    the Amiga. The flavour comes from the archive's CONTENT — the two codecs
//!    are structurally distinguishable — not from its extension, which a rename
//!    can make a lie.
//!
//!    **A codec that names two machines is refined by step 3, not settled by
//!    step 2** (SQ-0843). [`Flavour::AmigaMac`] is one container written by both
//!    the Amiga and the Macintosh, so an archive of that flavour states a codec;
//!    when the story also came off a disk, the medium states the machine and
//!    wins. That is not a reordering — [`Flavour::Pc`] is unambiguous and still
//!    beats the medium outright — it is one ambiguous answer resolved by a
//!    definite one. Without it, picking `CPic.data` off `Zork Zero Disk.image`
//!    (the archive that disk loads on its own) turned a Macintosh into an Amiga.
//! 3. The medium: a story mounted out of an Amiga `.adf` release floppy is an
//!    Amiga, one mounted out of an HFS volume is a Macintosh, one mounted out of
//!    an Atari ST GEMDOS floppy is an ST, and one mounted out of an Apple ProDOS
//!    volume is an Apple IIgs. (A DOS FAT12 floppy is the same
//!    filesystem as that last one and still resolves to the IBM PC, whose number
//!    is version-dependent and already in force — see [`blorb::medium`], where
//!    the two rows are argued side by side.) The
//!    medium→machine mapping itself is [`blorb::medium`]'s, not this module's,
//!    because `zvm-cli` has to reach the same conclusion off the same bytes and
//!    does not depend on `app` (SQ-0839).
//!
//!    **The medium is the only honest discriminator for the Macintosh**, and
//!    that is a measurement rather than a preference: the Amiga and the
//!    Macintosh wrote the *same* colour archive, and `Zork Zero Disk.image`
//!    proves it — its `CPic.data` is structurally indistinguishable from an
//!    Amiga `Pic.data`, which is why [`Self::for_art_flavour`] still answers
//!    `Amiga` for the whole of [`Flavour::AmigaMac`]. A volume, by contrast,
//!    cannot be mistaken: HFS is Apple's filesystem and nothing else wrote one.
//! 4. [`InterpreterProfile::IbmPc`], for everything else.
//!
//! Step 2 cannot move the existing corpus, and that is worth stating because
//! header byte `$1E` is not inert — `zvm`'s `exec.rs` branches on
//! `read_byte(0x1E) == 6`, so a v6 story that stopped being an IBM PC would
//! start *doing* something different. Two things pin it. The key that triggers
//! the inference is new, so no story in `stories/` has one; and the only
//! non-Amiga flavour, [`Flavour::Pc`], maps to `IbmPc`, whose
//! [`interpreter_number`](InterpreterProfile::interpreter_number) is `None` and
//! therefore leaves zvm's own rule (6 for v6) exactly where it was. Nothing
//! moves unless a user writes a `pictures` key naming an Amiga archive, which is
//! precisely the request being honoured.
//!
//! Authenticity can cost readability — the Amiga's own default page is a dark
//! grey (see [`AMIGA_DEFAULT_BACKGROUND`]), and a game that picks white text
//! against it was legible on a 1989 monitor and is merely adequate in a modern
//! terminal. There is deliberately no new setting for that: `honor_game_colours`
//! already governs whether the game's colour choices are honoured at all, so
//! turning it off returns the user's theme, profile or no profile.
//!
//! It can also cost a lanthorn CONVENIENCE, and that has to be paid too. §8.3's
//! Amiga has exactly two pens for the whole screen, so the transcript's built-in
//! "a whole line in brackets came from the interpreter, mute it" rule is wrong
//! twice over there — the line is the game's prose, and the mute was picked to
//! recede against the theme's page rather than the machine's. It stands down on
//! this profile only; see [`crate::colors::ColorScheme::resolve_story_style`].

use std::path::Path;

use blorb::infocom_pics::Flavour;

/// How a launch arrived at its [`InterpreterProfile`] — and so whether it may
/// present that machine's own colours (SQ-0928).
///
/// The user's rule: **system colours apply only when the game is run from its
/// original media.** A machine named by the disk it came off is a fact about the
/// launch; a machine reached by falling through is not a machine at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileSource {
    /// The MEDIUM named it — a release floppy, a DOS image, a hybrid disc's own
    /// answer for this story. Original media, and the only source that licenses
    /// the machine's colours on its own.
    Medium,
    /// The player named it: `--interpreter N`, `interpreter_number`, or an
    /// archive whose flavour the medium could not refine. Advertises the byte in
    /// `$1E`, and licenses the colours only with the opt-in
    /// (`Config::system_colours`, from `--colour machine`), because a number typed
    /// at a bare story file is a request about the STORY, not a statement about
    /// where it came from.
    Asked,
    /// Nothing named a machine, so [`InterpreterProfile::IbmPc`] answered as the
    /// historical default. **Never** licenses machine colours: this is every
    /// modern Inform story ever opened, and the IBM PC's own doc has always said
    /// that here "default" should mean what the player actually sees.
    ///
    /// The DEFAULT variant, deliberately: a `Config` that has not resolved a story
    /// yet has no machine, and the safe answer to "may I paint this?" is no.
    #[default]
    Fallback,
}

impl ProfileSource {
    /// May this launch present its machine's §8.3.3 pair?
    ///
    /// `opt_in` is `Config::system_colours` — the escape hatch for a player who
    /// wants the Amiga's grey on a bare `.z6`. It cannot rescue [`Self::Fallback`],
    /// because there is no machine there to be faithful to.
    pub fn licenses_machine_colours(self, opt_in: bool) -> bool {
        match self {
            ProfileSource::Medium => true,
            ProfileSource::Asked => opt_in,
            ProfileSource::Fallback => false,
        }
    }
}

/// The machine lanthorn presents itself to the story as/// The machine lanthorn presents itself to the story as. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpreterProfile {
    /// Today's behaviour, named: an IBM PC (interpreter 6 on v6, 1 elsewhere),
    /// the container's own declared art resolution, the host terminal's colours
    /// and ZMSD §8.3.1's palette.
    #[default]
    IbmPc,
    /// An Amiga: interpreter 4, a 320×200 standard window doubled onto the
    /// 640×400 screen, the Amiga's own default colours, and the palette
    /// Infocom's Amiga interpreter loaded.
    Amiga,
    /// A Macintosh: interpreter 3, black text on a white page, and a screen
    /// that is **whichever one the artwork in hand was drawn for** — see
    /// [`Self::std_window`], which is where the interesting part is.
    Macintosh,
    /// An Atari ST: interpreter 5, black text on a white page, ZMSD §8.3.1's
    /// palette, and **no standard window at all** — the one machine here that
    /// declines a member of the bundle, because it never had a Version 6
    /// interpreter for one to describe (SQ-0835). See [`Self::std_window`].
    AtariSt,
    /// An Apple IIgs: interpreter 10, **white text on a black page** — the only
    /// dark page here that is genuinely black — ZMSD §8.3.1's palette, and no
    /// standard window (SQ-0857).
    ///
    /// The second profile to decline a member, and for a quite different reason
    /// to the Atari ST's. Infocom very much *did* write a Version 6 interpreter
    /// for this machine — the Apple II YZIP, whose own `MACHINE:` routine sits
    /// on `Journey.2mg` and `Arthur Quest 4 Excalibur.2mg` — so there is a real
    /// Version 6 screen to describe. It is 140x192 on a 3x9 cell, which is not
    /// the quantity [`Self::std_window`] holds. See that knob.
    AppleIIgs,
    /// A Commodore 128: **interpreter 7, and nothing else** (SQ-0869).
    ///
    /// The thinnest profile here, deliberately. It exists because the number
    /// would otherwise be dropped: `blorb`'s `.d64` row answers 7, `zvm-cli`
    /// takes that answer straight off the medium, and this enum is how the TUI
    /// takes it — so a Commodore medium with no variant here would have the two
    /// front-ends disagreeing, which is the exact half-wiring `blorb::medium`
    /// exists to make impossible.
    ///
    /// Every OTHER member is declined, and each for a stated reason rather than
    /// for want of effort. There is no standard window because Infocom never
    /// wrote a Version 6 interpreter for the machine — the same ground the
    /// [`Self::AtariSt`] declines on. There is no palette of the Commodore's own
    /// here because none has been read out of Infocom's Commodore interpreter;
    /// the C64's sixteen hardware colours are famous and are not evidence, which
    /// is the call [`Self::AtariSt`] makes about the ST's 512 and
    /// [`Self::AppleIIgs`] about the Apple's double hi-res. And the default
    /// colour pair is declined for the same reason, so a Commodore story is told
    /// what the player's terminal actually looks like.
    ///
    /// **Filling those in is a separate piece of work**, wanted only if it can be
    /// sourced the way the other three were: out of Infocom's own Commodore
    /// interpreter, not out of the hardware's reputation.
    Commodore128,
    /// A Commodore 64: **interpreter 8, and a period look** (SQ-0873).
    ///
    /// As thin as [`Self::Commodore128`] on everything a story can read, and for
    /// the same reasons — no Infocom Commodore interpreter has been read, so the
    /// palette and the `$2C`/`$2D` pair are declined rather than invented. What
    /// it adds is the one thing that IS measured: the machine's own screen, off
    /// `machine-screenshots/c64-zork1-solidgold.png`, whose banner reads
    /// "Interpreter 8 Version J".
    ///
    /// **No medium selects it.** A `.d64` is a 1541 image both Commodore machines
    /// read, so `blorb::medium` answers 7 and this variant is reached only by
    /// naming 8 outright — exactly the route [`Self::AppleIIe`] exists for on a
    /// ProDOS volume that cannot name which of the family pressed it.
    Commodore64,
    /// An Apple IIe: **interpreter 2, and otherwise the [`Self::AppleIIgs`]
    /// bundle exactly** (SQ-0872).
    ///
    /// SQ-0857 established that the Apple II YZIP is one program for three
    /// machines — `bsubs.asm`'s `MACHINE:` picks between `IIeID 2`, `IIcID 9` and
    /// `IIgsID 10` at boot, *after* `zboot.asm` has already seeded the same black
    /// page and white ink — so the only thing that distinguishes the family is the
    /// byte. That quest scoped itself to the IIgs because the medium cannot name
    /// the press; this variant exists for the other route, a player naming 2
    /// outright, which until now got an IBM PC wearing an Apple's number.
    ///
    /// No standard window, for the reason [`Self::AppleIIgs`] states at length:
    /// the Apple's Version 6 screen is 140x192 on a 3x9 cell, which is a different
    /// screen MODEL rather than a resolution this knob can hold.
    AppleIIe,
    /// An Apple IIc: **interpreter 9**, and the same bundle again — see
    /// [`Self::AppleIIe`], which shares every value and every argument.
    AppleIIc,
}

impl InterpreterProfile {
    /// Resolve the profile for a launch: an explicit interpreter number wins,
    /// else the flavour of a picture archive the user named outright, else the
    /// medium the story came out of, else [`Self::IbmPc`]. See the module docs
    /// for why step two cannot disturb the existing corpus.
    ///
    /// `story_path` is the path the user opened, which for a disk image is the
    /// image itself rather than the story inside it — that is the whole point,
    /// since the medium is what identifies the machine.
    ///
    /// `named_art` is [`crate::graphics::PictureOverride::flavour`]: `None`
    /// whenever no usable archive was named, which is every launch that does not
    /// use tier 3.
    ///
    /// `mounted_as` is the medium a caller ALREADY resolved for the particular
    /// story it is launching — [`crate::hints::load_mounted_story_from`]'s second
    /// answer. Pass it whenever you have it (SQ-0876). Two reasons, and the first
    /// is correctness: on a hybrid disc the medium is a property of the story and
    /// not of the image, so re-deriving it from `story_path` reads the
    /// FILESYSTEM's machine and tells all 50 of the Masterpieces CD's DOS builds
    /// to advertise the Macintosh. The second is cost — that caller has already
    /// mounted the image, and this is a 354 MB read not to repeat.
    ///
    /// `None` means "work it out from the path", which is what every caller
    /// without a mount does and is exactly the old behaviour.
    pub fn resolve(
        story_path: &Path,
        configured_interpreter_number: Option<u8>,
        named_art: Option<Flavour>,
        mounted_as: Option<blorb::medium::DiskImage>,
    ) -> Self {
        Self::resolve_with_source(story_path, configured_interpreter_number, named_art, mounted_as).0
    }

    /// [`Self::resolve`], and **where the answer came from** (SQ-0928).
    ///
    /// The profile alone cannot say whether this launch may present the machine's
    /// own colours, because [`Self::IbmPc`] is two different answers wearing one
    /// name: the machine a DOS floppy names, and the thing every story with no
    /// medium at all falls through to. Paint the first blue and you are being
    /// faithful; paint the second blue and every modern Inform story comes up blue.
    ///
    /// So the source travels with the profile, and [`ProfileSource::licenses_machine_colours`]
    /// is the question `Config::machine_default_colours` asks of it.
    pub fn resolve_with_source(
        story_path: &Path,
        configured_interpreter_number: Option<u8>,
        named_art: Option<Flavour>,
        mounted_as: Option<blorb::medium::DiskImage>,
    ) -> (Self, ProfileSource) {
        if let Some(n) = configured_interpreter_number {
            // SQ-0930: a number this table does not model is a FALLBACK, not a
            // machine the player asked for. `for_interpreter_number` lands every
            // unmodelled number on `IbmPc` — which was inert while that variant
            // stated nothing, and is not now that it states blue under white:
            // `--interpreter 1 --colour machine` would have painted a
            // DECSystem-20 in the IBM PC's colours. The number still reaches
            // `$1E`, because the story asked and §11.1.3 has an answer.
            let src = match Self::try_for_interpreter_number(n) {
                Some(_) => ProfileSource::Asked,
                None => ProfileSource::Fallback,
            };
            return (Self::for_interpreter_number(n), src);
        }
        let medium = mounted_as.or_else(|| Self::medium(story_path));
        // A named archive is an INSTRUCTION about which artwork to load, and it
        // refines to the medium underneath where there is one — so the source is
        // the medium's when the medium is what settled it, and otherwise the
        // player's. Naming an `.mg1` beside a bare story file is not original
        // media and does not license that machine's colours.
        if let Some(flavour) = named_art {
            let from_medium = medium
                .is_some_and(|m| m.interpreter_number().is_some() || m.implies_ibm_pc());
            let src = if from_medium { ProfileSource::Medium } else { ProfileSource::Asked };
            return (Self::for_art_flavour_on(flavour, medium), src);
        }
        if let Some(n) = medium.and_then(|m| m.interpreter_number()) {
            return (Self::for_interpreter_number(n), ProfileSource::Medium);
        }
        // SQ-0930: a DOS medium NAMES the IBM PC — its `interpreter_number` is
        // `None` because the machine's number is a version rule, not because the
        // disk says nothing. Reading those two alike made a DOS floppy resolve as
        // no medium at all: the story was told DECSystem-20 and the machine's own
        // page never applied, on the one medium that unambiguously states it.
        if medium.is_some_and(|m| m.implies_ibm_pc()) {
            return (Self::IbmPc, ProfileSource::Medium);
        }
        (Self::IbmPc, ProfileSource::Fallback)
    }

    /// The machine implied by an archive of `flavour` **found on `medium`** —
    /// [`Self::for_art_flavour`] with the one ambiguity its codec cannot settle
    /// resolved by the disk under it (SQ-0843).
    ///
    /// [`Flavour::AmigaMac`] is one container written by two machines, so naming
    /// such an archive states a codec and not a machine. When the story came out
    /// of a disk image, the medium states the machine — HFS is Apple's
    /// filesystem and nothing else wrote one — and a fact beats an ambiguity.
    /// [`Flavour::Pc`] is unambiguous and still beats the medium outright, which
    /// is what keeps "naming an archive is an instruction" true: an `.mg1` named
    /// on a Macintosh disk asks for the IBM PC and gets it.
    ///
    /// **This is what [`Self::for_art_flavour`] already documented and did not
    /// do.** Its own text says "a `Pic.data` on the Mac disk gets the Macintosh
    /// anyway, from the disk under it", and until SQ-0843 the named-archive step
    /// simply returned before the medium was ever consulted. The gap was
    /// invisible while `--pictures Pic.data` was the only door to a Macintosh
    /// archive; the launch-options dialog now lists both of that disk's archives,
    /// so picking `CPic.data` — the very archive the story loads on its own —
    /// would have demoted a Macintosh to an Amiga, and said so on screen two
    /// lines below the row that did it.
    ///
    /// A medium that implies neither Amiga nor Macintosh cannot refine an
    /// Amiga/Mac archive and does not try; the archive's own answer stands.
    pub fn for_art_flavour_on(flavour: Flavour, medium: Option<blorb::medium::DiskImage>) -> Self {
        if flavour == Flavour::AmigaMac {
            let refined = medium
                .and_then(|d| d.interpreter_number())
                .map(Self::for_interpreter_number)
                .filter(|p| matches!(p, Self::Amiga | Self::Macintosh));
            if let Some(profile) = refined {
                return profile;
            }
        }
        Self::for_art_flavour(flavour)
    }

    /// The machine implied by the flavour of a picture archive.
    ///
    /// [`Flavour::Pc`] covers `.MG1` (MCGA), `.EG1`/`.EG2` (EGA) and `.CG1`
    /// (CGA) — three video cards, one machine, and that machine is the IBM PC.
    /// The card is a display choice, not a Z-machine one: Frotz's DOS port picks
    /// the extension from its display mode and never consults byte `$1E` to do
    /// it, so there is nothing finer than "IBM PC" for the header to say.
    ///
    /// [`Flavour::AmigaMac`] is named for a real limit rather than a shortcut:
    /// the Amiga and the Macintosh wrote the *same* big-endian Huffman container,
    /// and nothing in it distinguishes them in general. (ZMSD §11.1.3 numbers
    /// them separately — 3 Macintosh, 4 Amiga — so the distinction would matter
    /// if it could be made.) The one lead is Spatterlight's bocfel, which
    /// reclassifies a `Pic.data` as monochrome Macintosh when the file's flags
    /// byte reads `0x0e`, with the honest comment that the flags "always *seem*
    /// to equal 0xe if the graphics are monochrome" — a heuristic, and one that
    /// separates only the B&W Mac.
    ///
    /// **There is a [`Self::Macintosh`] to select now (SQ-0838), and this still
    /// answers [`Self::Amiga`]** — because the archive is not what knows. The
    /// Macintosh release disk settled it by counterexample: `CPic.data` off
    /// `Zork Zero Disk.image` is a Mac colour archive, and nothing in it
    /// distinguishes it from an Amiga one. Only the MEDIUM does, which is where
    /// the Macintosh hangs (see the module docs, precedence 3). Naming an
    /// archive by hand therefore still asks for the machine the *codec* implies,
    /// and a `Pic.data` on the Mac disk gets the Macintosh anyway, from the disk
    /// under it.
    ///
    /// That last sentence describes [`Self::for_art_flavour_on`], which is where
    /// the disk is actually consulted — and until SQ-0843 nothing did it, so the
    /// claim was true of the design and false of the code. Call that one unless
    /// you genuinely have no medium to offer it.
    ///
    /// The Apple is the one flavour with no ambiguity to resolve: its codec, its
    /// 8-byte record and its 140×192 picture space are peculiar to the Apple II
    /// and no other machine shipped them, so the archive really does name the
    /// machine here (SQ-0863).
    pub fn for_art_flavour(flavour: Flavour) -> Self {
        match flavour {
            Flavour::Pc => Self::IbmPc,
            Flavour::AmigaMac => Self::Amiga,
            Flavour::Apple => Self::AppleIIgs,
        }
    }

    /// The profile a story header byte `$1E` value implies, falling back to the
    /// IBM PC bundle — the historical default — for a machine lanthorn does not
    /// model.
    ///
    /// **The fallback is the honest answer and it is also a silent one**, which is
    /// why [`Self::try_for_interpreter_number`] exists beside it: asking for a
    /// machine with no profile gets that number in `$1E` and an IBM PC everywhere
    /// else, and a caller that can say so should.
    pub fn for_interpreter_number(n: u8) -> Self {
        Self::try_for_interpreter_number(n).unwrap_or(Self::IbmPc)
    }

    /// The profile `n` names, or `None` when lanthorn models no such machine.
    ///
    /// The set is [`zvm::interpreter::MACHINES`]'s, which is where the gaps and
    /// the reason for each are argued: 1 (DECSystem-20) is a decision rather than
    /// a datum, 8 (Commodore 64) has no interpreter read for it, 11 (Tandy Color)
    /// has no fixture and no sourced constant. Answering `None` rather than
    /// [`Self::IbmPc`] is what lets a front-end report "I do not model that
    /// machine" instead of quietly substituting another (SQ-0872).
    pub fn try_for_interpreter_number(n: u8) -> Option<Self> {
        match n {
            APPLE_IIE_INTERPRETER_NUMBER => Some(Self::AppleIIe),
            MACINTOSH_INTERPRETER_NUMBER => Some(Self::Macintosh),
            AMIGA_INTERPRETER_NUMBER => Some(Self::Amiga),
            ATARI_ST_INTERPRETER_NUMBER => Some(Self::AtariSt),
            IBM_PC_INTERPRETER_NUMBER => Some(Self::IbmPc),
            COMMODORE_128_INTERPRETER_NUMBER => Some(Self::Commodore128),
            COMMODORE_64_INTERPRETER_NUMBER => Some(Self::Commodore64),
            APPLE_IIC_INTERPRETER_NUMBER => Some(Self::AppleIIc),
            APPLE_IIGS_INTERPRETER_NUMBER => Some(Self::AppleIIgs),
            _ => None,
        }
    }

    /// The `zvm` machine table row behind this profile — the number, the `$2C`/
    /// `$2D` pair, the palette and the §8.3 screen rules — or `None` for
    /// [`Self::IbmPc`], whose number is a rule rather than a constant and which
    /// therefore states no row to look up (SQ-0872).
    ///
    /// This is the single place the app's bundle and the CLI's meet: everything
    /// below that a story can READ comes through here, so the two front-ends
    /// cannot drift into presenting different machines.
    fn machine(self) -> Option<&'static zvm::interpreter::MachineProfile> {
        zvm::interpreter::machine(self.row_number())
    }

    /// The §11.1.3 number whose ROW describes this machine.
    ///
    /// Not [`Self::interpreter_number`], and the difference is [`Self::IbmPc`]'s
    /// alone. That knob answers *"what should go in header `$1E`?"* and the IBM PC
    /// answers `None` on purpose, so zvm's own version rule (Frotz's 6-for-v6,
    /// 1-otherwise) stays in force and naming the profile cannot change what the
    /// corpus advertises. But the machine is still the IBM PC, and its row still
    /// describes it — so a question about the MACHINE has to look 6 up regardless.
    ///
    /// Leaving the two conflated made the IBM PC's row unreachable through the
    /// profile: `machine()` asked for a number the profile declines to state, got
    /// `None`, and reported that the IBM PC has no palette and no colours — which
    /// was invisible while the row declined a pair anyway, and became SQ-0928's
    /// whole feature failing silently the moment it stated one.
    pub fn row_number(self) -> u8 {
        self.interpreter_number().unwrap_or(IBM_PC_INTERPRETER_NUMBER)
    }

    /// The release medium at `path`, or `None` when it is not one. The single
    /// read this module does, and only the fallback — [`Self::resolve`]'s
    /// `mounted_as` is preferred, being both per-story and already paid for.
    ///
    /// Content, not extension: [`blorb::medium::DiskImage::detect`] reads the
    /// filesystem, exactly as `PictSource::resolve` and `hints::read_story_file`
    /// do, so a disk image with any name (or none) is recognised and a mis-named
    /// ordinary story file is not. The MAPPING is `blorb`'s so that `zvm-cli`
    /// reaches the same conclusion off the same bytes (SQ-0839) — this only
    /// supplies the file.
    fn medium(path: &Path) -> Option<blorb::medium::DiskImage> {
        let raw = std::fs::read(path).ok()?;
        blorb::medium::DiskImage::detect(&raw)
    }

    /// The interpreter number to advertise in header `$1E`, or `None` to leave
    /// the VM's own default rule in force.
    ///
    /// [`Self::IbmPc`] returns `None` on purpose rather than computing 6-or-1
    /// here: zvm's existing default (Frotz's rule — 6 for Version 6, 1
    /// otherwise) *is* the IBM PC rule, and deferring to it means naming the
    /// profile cannot possibly change what the corpus advertises.
    pub fn interpreter_number(self) -> Option<u8> {
        match self {
            Self::IbmPc => None,
            Self::Amiga => Some(AMIGA_INTERPRETER_NUMBER),
            Self::Macintosh => Some(MACINTOSH_INTERPRETER_NUMBER),
            Self::AtariSt => Some(ATARI_ST_INTERPRETER_NUMBER),
            Self::AppleIIgs => Some(APPLE_IIGS_INTERPRETER_NUMBER),
            Self::Commodore128 => Some(COMMODORE_128_INTERPRETER_NUMBER),
            Self::Commodore64 => Some(COMMODORE_64_INTERPRETER_NUMBER),
            Self::AppleIIe => Some(APPLE_IIE_INTERPRETER_NUMBER),
            Self::AppleIIc => Some(APPLE_IIC_INTERPRETER_NUMBER),
        }
    }

    /// The standard window — the machine's native ART resolution — when the
    /// resource container declares none, or `None` to keep the container's
    /// answer as the only one.
    ///
    /// Blorb §11 lets a resource file declare its art's intended resolution in a
    /// `Reso` chunk, and lanthorn scales v6 artwork by 2 onto the 640×400 unit
    /// screen only when such a declaration exists — a file with no `Reso`
    /// declares its images non-scalable, so scopa and mysterious01 correctly
    /// draw at 1:1 (SQ-0715). A native Amiga `Pic.data` archive has no `Reso`
    /// chunk because **the format has no such concept**, not because anyone
    /// declared anything, and reading that absence as a declaration is what left
    /// Zork Zero's 320×200 art at half size on a 640×400 screen (SQ-0736). The
    /// machine, not the container, is what knows the answer there — so the
    /// profile supplies it, and the existing rule fires unchanged.
    ///
    /// [`Self::IbmPc`] returns `None`: a Blorb-sourced story keeps deciding for
    /// itself, exactly as before.
    ///
    /// # The Macintosh has TWO screens, and the artwork picks
    ///
    /// This is the one machine whose answer is not a single pair, and the
    /// reason is that Infocom's own Macintosh interpreter sized its window and
    /// chose its picture file in **one decision**. `mac/xzip.lst`:
    ///
    /// ```text
    ///   IF ((ydisplay < 2*GFXAM_Y) OR (xdisplay < 2*GFXAM_X))
    ///     OR ((mColor = FALSE) OR (ttyToggle)) THEN
    ///     BEGIN  myBig := FALSE;  wy := GFXMAC_Y;  wx := GFXMAC_X;  END
    ///   ELSE
    ///     BEGIN  myBig := TRUE;   wy := 2*GFXAM_Y; wx := 2*GFXAM_X; END
    /// ```
    ///
    /// with `GFXAM_X = 320; GFXAM_Y = 200` ("raw" size of full-screen Amiga
    /// pics) and `GFXMAC_X = 480; GFXMAC_Y = 300` ("1.5 x Amiga sizes") — and
    /// the very same flag then names the file: *"for a small window use mono
    /// gfx, for a big window use color gfx"*, `IF myBig THEN gfxName :=
    /// 'CPic.Data' ELSE gfxName := 'Pic.Data'`.
    ///
    /// So a big colour Macintosh runs a **640×400** window off `CPic.data`
    /// (320×200 art at 2×), and a standard compact Mac runs a **480×300** window
    /// off `Pic.data` (480×300 monochrome art at 1× — `IF ge.mono OR myTiny THEN
    /// { scale 1x for display }`). The screen the game is told about is that
    /// window and nothing else:
    ///
    /// ```text
    ///   { calculate our logical screen size, based on window size }
    ///     WITH myWindow^.portRect DO
    ///       BEGIN
    ///       totRows := (bottom - top) {DIV lineheight};
    ///       totCols := ((right - left) - (2 * wMarg)) {DIV colWidth};
    ///       END;
    /// ```
    ///
    /// (the `DIV`s commented out, so both are in PIXELS, and the `2 * wMarg`
    /// takes the 4-pixel text inset back off — `totCols` is exactly `wx`.)
    ///
    /// **512×342 is the hardware, not the standard window.** The compact Mac's
    /// screen only ever appears here as `screenRect := screenBits.bounds`, the
    /// thing the window is centred *in*: `SizeWindow (myWindow, wx + (2*wMarg),
    /// wy, FALSE)` then `MoveWindow (myWindow, ((xdisplay-wx) DIV 2) - wMarg,
    /// (ydisplay-wy) DIV 2 + 17 {mbar fudge}, TRUE)`. On a 512×342 screen that
    /// is a 488×300 content rect at y = (342−300)/2 + 17 = 38 — which leaves
    /// exactly the 20-pixel menu bar and a 19-pixel title bar above it, and 4
    /// pixels below. A screenshot of a real Mac Zork Zero is therefore 512×342
    /// with the game filling nearly all of it, and the story is still being told
    /// its screen is 480×300.
    ///
    /// This profile answers with the big-colour pair, because that is the
    /// machine the disk's DEFAULT archive belongs to. The 480×300 screen is not
    /// this knob's to state: it arrives with the monochrome archive, from
    /// [`crate::graphics::PictSource::native_std_window`], exactly as it arrived
    /// with `Pic.Data` on the Mac. One decision there too.
    ///
    /// # The Atari ST has no answer, and that is a FACT about the machine
    ///
    /// [`Self::AtariSt`] returns `None`, and this is the one place in the bundle
    /// where a profile declines. It is not a gap awaiting a fixture. **Infocom
    /// never wrote a Version 6 interpreter for the Atari ST**: `st/` in
    /// `infocom-zcode-terps` holds a ZIP (`stzip.s`) and an XZIP (`stx*.s`,
    /// `xzip.c`) and no YZIP, where the repository lists one for the Apple and
    /// the Macintosh. A standard window is a Version 6 ART geometry, so there is
    /// no ST artwork for it to be the resolution *of* — which is the same fact
    /// the corpus reports from the other end, all thirty-nine stories across the
    /// nine ST compilations being v3, v4 or v5.
    ///
    /// The ST's own screen word is not this quantity in any case. `st/stx1.s`
    /// fills `PSCRWD` from the live text display rather than from a constant:
    ///
    /// ```text
    ///   st/stx1.s:615   MOVE.W  _columns,D1   * SIZE OF ATARI SCREEN DISPLAY (40 OR 80)
    ///   st/stx1.s:735   MOVE.W  _rows,D0
    ///   st/stx1.s:742   MOVE.W  D0,PSCRWD(A2) * SET SCREEN-PARAMETERS WORD
    /// ```
    ///
    /// — rows and columns of whatever display is attached, which is already what
    /// lanthorn tells a story about its pane.
    ///
    /// # The Apple IIgs HAS an answer, and it is not this quantity
    ///
    /// [`Self::AppleIIgs`] also returns `None`, and the reason is the opposite of
    /// the Atari ST's (SQ-0857). Infocom wrote a Version 6 interpreter for this
    /// machine and it is in hand twice over — `apple/yzip/rel.15/` in
    /// `infocom-zcode-terps`, and its `MACHINE:` routine byte-for-byte on the two
    /// Version 6 ProDOS disks in `stories/`. Its screen is stated outright, in
    /// `apple/yzip/rel.15/apple.equ`:
    ///
    /// ```text
    ///   MAXWIDTH   EQU 140   ; 560 / 4 = max "pixels"
    ///   MAXHEIGHT  EQU 192   ; 192 screen lines
    ///   FONT_H     EQU 9     ; font height
    ///   MFONT_W    EQU 3     ; mono spaced font width, to game
    /// ```
    ///
    /// and `zboot.asm` hands exactly those to the story, `ZHWRD`/`ZVWRD` being
    /// header `$22`/`$24` and `ZSCRWD` the character grid at `$20`/`$21`:
    ///
    /// ```text
    ///   lda #MAXWIDTH      / sta ZBEGIN+ZHWRD+1     ; 140 pixels across
    ///   lda #MAXWIDTH/3    / sta ZBEGIN+ZSCRWD+1    ; 46 columns
    ///   lda #MAXHEIGHT     / sta ZBEGIN+ZVWRD+1     ; 192 pixels down
    ///   lda #MAXHEIGHT/FONT_H / sta ZBEGIN+ZSCRWD   ; 21 lines
    ///   lda #FONT_H        / sta ZBEGIN+ZFWRD       ; 9 tall
    ///   lda #3             / sta ZBEGIN+ZFWRD+1     ; 3 wide
    /// ```
    ///
    /// So the machine's Version 6 screen is **140x192 on a 3x9 cell**, 46x21
    /// characters — the 560-dot double hi-res display counted in four-dot colour
    /// pixels. That is a different screen MODEL, not a different resolution, and
    /// this knob cannot express it. What it holds is the art picture space that
    /// [`crate::session::V6_ART_SCALE`] doubles onto the 640x400 unit screen,
    /// which is then divided by [`Self::v6_font_cell`]'s fixed 8x16. Answering
    /// `Some((140, 192))` would tell the story its screen is 280x384 and 35x24
    /// characters — a machine that never existed, and further from the Apple's
    /// own 46x21 than the 80x25 it gets by declining. Honouring 140x192 honestly
    /// means making `V6_FONT_WIDTH`/`V6_FONT_HEIGHT` runtime state, which is the
    /// same refactor this bundle already declined for the Macintosh's real 7x15
    /// cell and for EGA's 8x8 — see [`Self::v6_font_cell`].
    ///
    /// **And there is now something for the ARCHIVE to size, which is why this
    /// knob still declines** (SQ-0863). Arthur's, Journey's, Shogun's and Zork
    /// Zero's Apple pictures live inside the segmented `ARTHUR.D1`-`.D5`
    /// container rather than in a file, and `blorb::infocom_pics`'s Apple
    /// flavour reads them: 168 pictures off `Arthur Quest 4 Excalibur.2mg`, 135
    /// off the five-volume `journey_s*.dsk` press, 55 off `shogun_s*.dsk`, 496
    /// off `zork_zero_*.dsk`. Every one of those archives states a 140x192
    /// picture space, and `crate::graphics::PictSource::native_std_window`
    /// carries it to the story ahead of this knob — so *Arthur* r63 lays out on
    /// a 560x384 screen where it used to get the artless 640x400.
    ///
    /// That is the answer to "what sizes it", and it is not this knob's: an
    /// archive outranks a profile here for the same reason the standard
    /// Macintosh's monochrome `Pic.data` outranks [`MACINTOSH_STD_WINDOW`] and
    /// lays Zork Zero out on 480x300 (SQ-0838). The run-time-cell question is
    /// untouched and still open — the story is told 70x24 characters on an 8x16
    /// cell where the Apple's own YZIP said 46x21 on a 3x9 — but it is no longer
    /// in the way of the pictures, because the space the art needs and the
    /// character grid the machine used are two different quantities and only the
    /// first of them is a standard window.
    pub fn std_window(self) -> Option<(u16, u16)> {
        self.machine().and_then(|m| m.v6_std_window)
    }

    /// The default background/foreground colour numbers this machine reports in
    /// header bytes `$2C`/`$2D` (ZMSD §8.3.3), or `None` to report the host
    /// terminal's own colours.
    ///
    /// [`Self::IbmPc`] returns `None`, which is right for a terminal-native
    /// experience: lanthorn tells the story what the player's terminal actually
    /// looks like, so "default" means what the player sees. A profile whose
    /// entire purpose is to present as an Amiga should not be describing the
    /// user's terminal, so [`Self::Amiga`] answers with the Amiga's own pair.
    ///
    /// **The pairs themselves are [`zvm::interpreter`]'s** (SQ-0872), which is
    /// where each is sourced out of Infocom's own interpreter for that machine —
    /// and which is why `zvm-cli` can now paint the same page lanthorn does. The
    /// Commodore 128's `None` is a *decline* rather than a default and is argued
    /// on the row: nothing in hand states the pair Infocom's Commodore
    /// interpreter reported, and the machine's famous light-blue-on-blue boot
    /// screen is the hardware's reputation, not evidence (SQ-0869).
    pub fn default_colours(self) -> Option<(u8, u8)> {
        self.machine()?.default_colours
    }

    /// The `(background, foreground)` this machine's screen states when its
    /// display is showing **two colours** — the ground a two-colour picture
    /// archive's transparency reveals (SQ-0956).
    ///
    /// A rendition belongs to a machine, and what a stencil reveals belongs to
    /// that machine's display MODE — never to the story, which cannot see which
    /// archive was loaded (Zork Zero issues `set_colour(fg=2, bg=9)` for every
    /// video card alike). So this is the same kind of claim as
    /// [`Self::default_colours`], asked of a narrower screen.
    ///
    /// **Only the IBM PC answers differently, and only in one channel.** Its
    /// CGA plate is one of four renditions the same machine could show, and the
    /// card it is showing is not the machine: `machine-screenshots/dos-zorkzero-cga.png`
    /// gives a **black** page under white ink where
    /// [`zvm::interpreter::IBM_PC_DEFAULT_BACKGROUND`] is blue. That constant's
    /// doc carries the census.
    ///
    /// **Every other machine falls through, and the Macintosh is why that is not
    /// an omission.** A Mac's screen IS a two-colour display — `mac/xzip.lst`'s
    /// `SetColor := (zWHITE*256) + zBLACK` and the mono `Pic.data` that same
    /// interpreter chose *for* it are one decision (SQ-0838) — so its two-colour
    /// pair is its pair, stated once and not twice. Nothing here exempts it; it
    /// simply has nothing extra to say.
    ///
    /// **This is what a two-colour launch REPORTS**, in header `$2C`/`$2D` — see
    /// [`crate::graphics::PictSource::two_colour_card_screen`], which is the one
    /// place `startup.rs`, `reset.rs` and the pins all ask. It is reported and not
    /// merely compared: with the pair in the header and
    /// [`zvm::screen::Palette::IbmCga`] installed beside it, a story's own
    /// `set_colour` reaches `zvm::screen::two_colour_card_request` and the card
    /// shows these two colours in whichever polarity the story asked for.
    pub fn two_colour_colours(self) -> Option<(u8, u8)> {
        match self {
            Self::IbmPc => Some((IBM_PC_TWO_COLOUR_BACKGROUND, IBM_PC_DEFAULT_FOREGROUND)),
            _ => self.default_colours(),
        }
    }

    /// What this machine's screen LOOKED LIKE for a story that has no opinion
    /// about it — the page and ink, how the status line was set apart, and the
    /// shape and colour of the input cursor (SQ-0873).
    ///
    /// **A different kind of claim to [`Self::default_colours`], which is why it
    /// is a different knob.** That pair is a fact the story can read, sourced
    /// from Infocom's own code; this is presentation, observed off the emulator
    /// captures in `machine-screenshots/`. [`zvm::interpreter::PeriodLook`] states
    /// the provenance and what it costs.
    ///
    /// Answering here is not the same as applying it. Colour arrives with Version
    /// 5, so a period look belongs only to a v1–v4 story — the gate is
    /// [`crate::period::resolve`], and this knob only says what the machine has.
    ///
    /// **The v1–v5 answer**, because a machine on its own has no Version to ask
    /// about. That matters on one row: the IBM PC's body pair is its own palette
    /// resolving the pair it reports, and Infocom's two IBM interpreters disagree
    /// about white — so the screen a story actually gets comes from
    /// [`crate::period::resolve`], which knows the Version (SQ-0939/SQ-0983).
    pub fn period_look(self) -> Option<zvm::interpreter::PeriodLook> {
        zvm::interpreter::period_look_for(self.row_number(), None)
    }

    /// The palette the story's colour NUMBERS resolve through.
    ///
    /// The Macintosh resolves through [`zvm::screen::Palette::Standard`], and
    /// that is a reading rather than a default. `mac/xzip.lst`'s `MapColor`
    /// **is** ZMSD §8.3.1's table, one arm per colour and no others:
    ///
    /// ```text
    ///   CONST zBLACK = 2; zRED = 3; zGREEN = 4; zYELLOW = 5;
    ///         zBLUE = 6; zMAGENTA = 7; zCYAN = 8; zWHITE = 9;
    ///   CASE zcid OF
    ///     zBLACK: mcid := blackColor;   zRED:   mcid := redColor;   …
    ///     zWHITE: mcid := whiteColor;
    ///   OTHERWISE mcid := 0;  { "map Z id to Mac id, 0 if err/unchanged" }
    /// ```
    ///
    /// Those eight are QuickDraw's original saturated planar constants, so the
    /// Macintosh named the standard colours and meant them — where the Amiga
    /// loaded a palette of its own, which is why only it needs one here.
    ///
    /// **The Atari ST answers `Standard` for the same kind of reason, read the
    /// same way** (SQ-0835). `st/xzip.c`'s `color_table` is the ST's whole
    /// palette, one row per Z-machine colour id and no others, and the ids run
    /// 2..9 in ZMSD §8.3.1's order:
    ///
    /// ```text
    ///   WORD color_table[8*RGBLEN] =        /* XZIP ST color settings */
    ///       { 0x0000, 0x0000, 0x0000,       /* id 2 = black  (0.5 / 8) */
    ///         0x03A9, 0x003E, 0x003E,       /* id 3 = red */
    ///         0x003E, 0x032C, 0x003E,       /* id 4 = green */
    ///         0x03A9, 0x03A9, 0x003E,       /* id 5 = yellow */
    ///         0x003E, 0x003E, 0x03A9,       /* id 6 = blue */
    ///         0x03A9, 0x003E, 0x03A9,       /* id 7 = magenta */
    ///         0x003E, 0x03A9, 0x03A9,       /* id 8 = cyan */
    ///         0x03A9, 0x03A9, 0x03A9 };     /* id 9 = white  (7.5 / 8) */
    /// ```
    ///
    /// Those are GEM VDI intensities in the VDI's own 0–1000 range, and they are
    /// the saturated primaries: `0x03A9` is 937 and `0x003E` is 62, which the
    /// file's own comments gloss as 7.5/8 and 0.5/8. So the ST asked for §8.3.1's
    /// eight colours and meant them, and the file adds that the hardware rounded
    /// even that off — *"Realized settings are (currently) less detailed (8/8 and
    /// 0/8)"*. Nothing here is a palette of the ST's own in the sense the Amiga's
    /// is; the 512-colour hardware is not evidence, and is not consulted.
    ///
    /// What the ST could not do was show them all at once, and that is a display
    /// limit rather than a palette. `st/color.note` (5/26/87): *"The color
    /// function in the XZIP spec lists eight colors. The Atari ST, in 80-column
    /// mode, can display at most four of them at any one time"*, with only one
    /// background, always index 0. `st/xzip.c` recycles indices under an LRU to
    /// fake the rest. A terminal has no such ceiling, so there is nothing to
    /// express — this is noted so the absence reads as measured rather than
    /// missed.
    ///
    /// **The Apple IIgs answers `Standard` on the same reading, and this time the
    /// source proves itself** (SQ-0857). `apple/yzip/rel.15/tables.asm` carries
    /// the map and its inverse on consecutive lines:
    ///
    /// ```text
    ///   tables.asm:219  ZIPCOLOR: db 0,1,6,7,$C,$B,$E,$F
    ///   tables.asm:220  APLCOLOR: db 2,3,$FF,$FF,$FF,$FF,4,5,$FF,$FF,$FF,7,$FF,$FF,8,9
    /// ```
    ///
    /// `ZIPCOLOR` is indexed by Z-machine colour id zero-based from 2 —
    /// `machine.asm`'s `ZCOLOR` does `dex ; lda ZIPCOLOR,X` — so it is one entry
    /// per §8.3.1 colour, in §8.3.1's order, 2..9 and no others. `APLCOLOR` is
    /// the exact inverse, indexed by the Apple's own 16-colour double hi-res
    /// hardware value: 0 to 2, 1 to 3, 6 to 4, 7 to 5, $B to 7, $C to 6, $E to 8,
    /// $F to 9, and `$FF` — "no Z-machine colour" — for the other eight. A round
    /// trip that closes on exactly §8.3.1's eight is the machine saying it asked
    /// for the standard colours and took the nearest of the sixteen it had.
    ///
    /// So there is no palette of the Apple's own in the sense the Amiga's is, and
    /// the double hi-res hardware is not consulted: its RGB has no canonical
    /// values (it is an artefact of NTSC colour fringing and differs by monitor
    /// and by emulator), and nothing in Infocom's sources states any. Declining
    /// to invent one is the same call [`Self::AtariSt`] makes about the ST's
    /// 512-colour hardware, one paragraph up.
    ///
    /// **The table is [`zvm::interpreter`]'s** (SQ-0872): only the Amiga loaded a
    /// palette of its own, and every other row answers
    /// [`zvm::screen::Palette::Standard`] on the readings quoted above.
    pub fn palette(self) -> zvm::screen::Palette {
        self.machine().map_or(zvm::screen::Palette::Standard, |m| m.palette)
    }

    /// The Version 6 character cell this machine declares — header `$26`/`$27`.
    ///
    /// **The table is [`zvm::interpreter`]'s** (SQ-1013), for the same reason the
    /// palette above is: an embedder that gets this machine's interpreter number
    /// from the engine should not have to rediscover its cell from the host. See
    /// [`zvm::interpreter::MachineProfile::v6_cell`] and
    /// [`zvm::interpreter::MACINTOSH_V6_CELL`], which carry the evidence.
    ///
    /// The Macintosh is the only row that is not 8x16: `mac/xzip.lst` sets
    /// `colWidth := 7; lineHeight := 15 {16}`, so 640x400 gives 91x26 characters
    /// and 480x300 gives 68x20. It is a DECLARED metric — the machine painted
    /// proportional Geneva 12 and still told the story 7 — and it is the MACHINE's
    /// rather than a press's: one code path serves both, and only the window it
    /// divides changes (SQ-0917).
    ///
    /// **The Apple IIgs is the row that still does not state its own** (SQ-0857,
    /// SQ-0863). Its cell is 3x9 — `MFONT_W EQU 3` and `FONT_H EQU 9` in
    /// `apple/yzip/rel.15/apple.equ`, handed to the story as `ZFWRD` — giving
    /// 46x21 characters where 8x16 gives 70x24 on the 560x384 its archive asks
    /// for. Moving it needs its cell and its screen stated in ONE coordinate
    /// system, and today they are not: 140x192 is a PICTURE space, which needs no
    /// cell to be true and is why the artwork did not have to wait for this.
    pub fn v6_font_cell(self) -> zvm::screen::V6Cell {
        self.machine().map_or(zvm::screen::V6Cell::DEFAULT, |m| m.v6_cell)
    }

    /// How this machine decides what a Version 6 window does with text that
    /// reaches its right margin (SQ-1071).
    ///
    /// The two machines Infocom shipped a Version 6 interpreter for do **not**
    /// read the window's wrapping attribute at all — see
    /// [`zvm::interpreter::V6WrapRegime`], which carries §8.8.3.1.2.2's table of
    /// what their interpreters actually did and the captures that confirm it. A
    /// row with no machine keeps the standard's own rule, which is what a bare
    /// story file with no medium to name a machine should get.
    pub fn v6_wrap_regime(self) -> zvm::interpreter::V6WrapRegime {
        self.machine()
            .map_or(zvm::interpreter::V6WrapRegime::Attributes, |m| m.v6_wrap_regime)
    }

    /// The space a face off this machine's OWN RELEASE MEDIA is authored in —
    /// Arthur's `char.data`, the Macintosh's `FONT` 524 (SQ-1039).
    ///
    /// `art_scale` is the ARCHIVE's (SQ-0790) and says how dense the PICTURES are.
    /// A typeface is a separate question, and the two machines that ship one answer
    /// it differently: the Amiga draws its RELEASE face in the picture space, so a
    /// doubled press doubles the face with it, and the Macintosh draws text at one
    /// native pixel per face pixel while its colour press doubles `CPic.data`
    /// around it. Scaling a face by the art scale there would declare Geneva 12's
    /// fifteen rows as thirty.
    ///
    /// The table is [`zvm::interpreter`]'s, beside the cell, for the same reason
    /// the cell is: see [`zvm::interpreter::V6FaceSpace`]. Turn it into native
    /// pixels with [`zvm::interpreter::V6FaceSpace::text_scale`], which is the one
    /// place that arithmetic lives.
    pub fn release_face_space(self) -> zvm::interpreter::V6FaceSpace {
        self.machine()
            .map_or(zvm::interpreter::V6FaceSpace::Native, |m| m.v6_release_face_space)
    }

    /// The space this machine's own SYSTEM face is authored in — Geneva out of a
    /// Mac OS System file, topaz out of a Workbench drawer or Kickstart ROM.
    ///
    /// **Not always the same as [`Self::release_face_space`]**, and the Amiga is
    /// why (SQ-1053): its releases author a face in the 320-wide picture space
    /// while topaz is drawn in the 640x200 hires mode the interpreter ran in, so
    /// one machine wants two different scales at once. The rule belongs to the
    /// NAME rather than to the row — [`zvm::interpreter::V6SystemFace::face_space`]
    /// — and a machine that names no system face falls back to its release space,
    /// which is unreachable: nothing looks for a system face it cannot name.
    pub fn system_face_space(self) -> zvm::interpreter::V6FaceSpace {
        self.v6_system_face().map_or_else(
            || self.release_face_space(),
            zvm::interpreter::V6SystemFace::face_space,
        )
    }

    /// What this machine's own SYSTEM body face is called on its boot media, or
    /// `None` where it has none to name (SQ-1037).
    ///
    /// The two machines Infocom wrote a Version 6 interpreter for both painted
    /// prose with a face that lives on the operating system rather than on the
    /// game disk — Geneva in the Macintosh System file, topaz in the Amiga's ROM
    /// and `FONTS:` drawer — so a release's own medium can answer for the
    /// fixed-pitch ALTERNATE and not for the body. See
    /// [`zvm::interpreter::V6SystemFace`], and `crate::system_fonts` for the
    /// reading of the player's own disks.
    pub fn v6_system_face(self) -> Option<zvm::interpreter::V6SystemFace> {
        self.machine().and_then(|m| m.v6_system_face)
    }

    /// Whether this machine draws §8.7.1's Italic bit as an UNDERLINE rather than a
    /// slope (SQ-1028).
    ///
    /// Both machines Infocom shipped a Version 6 interpreter for do, measured on one
    /// frame of one game: `machine-screenshots/amiga-shogun-game.png` and
    /// `mac-shogun.jpg` both rule under `Erasmus` in "the Erasmus, a Dutch merchant"
    /// and under nothing beside it. The standard licenses either — §8.7.1 offers
    /// "rendering italic with underlining" as its own example — so this is a
    /// fidelity fact, not a compliance one. See [`zvm::interpreter::V6Emphasis`].
    pub fn underlines_emphasis(self) -> bool {
        self.machine()
            .is_some_and(|m| m.v6_emphasis == zvm::interpreter::V6Emphasis::Underline)
    }
}

/// The §11.1.3 interpreter numbers, from [`zvm::interpreter`] — the machine
/// table, which is where each is sourced and where the colour pair, palette and
/// screen rules that go with it live (SQ-0872).
///
/// Re-exported rather than restated so the app names exactly the constant `zvm`
/// writes into `$1E` and `zvm-cli` reads back. `blorb::medium` states the same
/// values for a different question — which machine a DISK implies — because it
/// takes zero external dependencies and so cannot see this table; the two are
/// pinned against each other by `interpreter_profile`'s agreement test, and a
/// future divergence fails there rather than in a game.
pub use zvm::interpreter::{
    AMIGA_INTERPRETER_NUMBER, APPLE_IIC_INTERPRETER_NUMBER, APPLE_IIE_INTERPRETER_NUMBER,
    APPLE_IIGS_INTERPRETER_NUMBER, ATARI_ST_INTERPRETER_NUMBER, COMMODORE_64_INTERPRETER_NUMBER,
    COMMODORE_128_INTERPRETER_NUMBER, IBM_PC_INTERPRETER_NUMBER, MACINTOSH_INTERPRETER_NUMBER,
};

/// The §8.3.3 default colour pairs, from [`zvm::interpreter`] — each quoted at
/// its constant out of Infocom's own interpreter for that machine (SQ-0872).
///
/// These moved out of this module so `zvm-cli` could reach them: the app was
/// seeding header `$2C`/`$2D` from a profile the CLI could not depend on, so a
/// story off a release disk was told which machine it was on by both front-ends
/// and what that machine looked like by only one.
pub use zvm::interpreter::{
    AMIGA_DEFAULT_BACKGROUND, AMIGA_DEFAULT_FOREGROUND, APPLE_DEFAULT_BACKGROUND,
    APPLE_DEFAULT_FOREGROUND, IBM_PC_DEFAULT_FOREGROUND, IBM_PC_TWO_COLOUR_BACKGROUND,
    MAC_DEFAULT_BACKGROUND, MAC_DEFAULT_FOREGROUND, ST_DEFAULT_BACKGROUND, ST_DEFAULT_FOREGROUND,
};

/// The Version 6 standard windows, from [`zvm::interpreter`] — the machine table
/// (SQ-1013).
///
/// **They moved, and the note that kept them here did not survive its own
/// reasoning.** It said "a standard window stays in the app… it is a Version 6
/// picture space stated by an ARCHIVE, resolved against `PictSource`, and zvm has
/// no business reading resource files (SQ-0872)". The second half is still true
/// and is why [`InterpreterProfile::std_window`]'s CHAIN stays here — a container's
/// `Reso` chunk, then a named archive, then the archive's own picture space, then
/// the machine. But the constants are not that chain: they are its last link, the
/// answer a MACHINE gives when nothing else has one, and knowing what an Amiga
/// presented makes zvm read no files at all.
///
/// So the resolution is the host's and the value is the engine's, which is the
/// same split every other member of this bundle already had.
pub use zvm::interpreter::{AMIGA_STD_WINDOW, MACINTOSH_STD_WINDOW};

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;

    #[test]
    fn ibm_pc_is_the_default_and_has_no_opinion_anywhere() {
        // The acceptance criterion for SQ-0719 in one test: naming today's
        // behaviour must not BE a behaviour. Every knob defers.
        let p = InterpreterProfile::default();
        assert_eq!(p, InterpreterProfile::IbmPc);
        assert_eq!(p.interpreter_number(), None, "defer to zvm's Frotz rule");
        assert_eq!(p.std_window(), None, "defer to the container's Reso chunk");
        // SQ-0939 SPLIT A SECOND KNOB THE SAME WAY, for the same reason and with
        // the same shape as `default_colours` below. The MACHINE resolves colour
        // numbers through EGA — Infocom's own `Zip_to_ega`/`zip_to_ibm_color`
        // tables — and the LAUNCH still defers, because `startup` downgrades an
        // unlicensed one to §8.3.1's table before it ever reaches
        // `zvm::screen::set_palette`.
        assert_eq!(p.palette(), zvm::screen::Palette::IbmXzip, "the machine resolves through EGA");
        assert_eq!(
            zvm::interpreter::palette_for(p.row_number(), Some(6)),
            zvm::screen::Palette::IbmYzip,
            "…and a Version 6 story would have run under the other IBM interpreter",
        );
        // SQ-0928 SPLIT THIS ONE KNOB IN TWO, and the split is the quest.
        //
        // The MACHINE states blue under white — observed from DOS captures, and a
        // fact about the IBM PC whoever is asking. The LAUNCH still defers, because
        // this variant is also what every story with no medium falls through to,
        // and `ProfileSource::Fallback` licenses nothing. So "no opinion anywhere"
        // is now false of the machine and true of the default launch, which is
        // exactly the distinction that lets a DOS floppy be blue without painting
        // every Inform game blue.
        assert_eq!(p.default_colours(), Some((6, 9)), "the machine states its pair");
        assert!(
            !ProfileSource::Fallback.licenses_machine_colours(true),
            "…and the launch that merely fell through here never presents it",
        );
    }

    #[test]
    fn amiga_knobs_are_the_verified_constants() {
        let p = InterpreterProfile::Amiga;
        assert_eq!(p.interpreter_number(), Some(4), "ZMSD §11.1.3: 4 = Amiga");
        assert_eq!(p.std_window(), Some((320, 200)));
        assert_eq!(
            p.default_colours(),
            Some((12, 9)),
            "the release floppies' own DEF_BACK/DEF_FORE (SQ-0822)"
        );
        assert_eq!(p.palette(), zvm::screen::Palette::Amiga);
    }

    #[test]
    fn macintosh_knobs_are_the_verified_constants() {
        // Every one of these is quoted at its constant, from ZMSD §11.1.3 and
        // from Infocom's own Macintosh interpreter (`mac/xzip.lst`, `mac/gfx.p`).
        let p = InterpreterProfile::Macintosh;
        assert_eq!(p.interpreter_number(), Some(3), "ZMSD §11.1.3: 3 = Macintosh");
        assert_eq!(p.std_window(), Some((320, 200)), "wx := 2*GFXAM_X, wy := 2*GFXAM_Y");
        assert_eq!(
            p.default_colours(),
            Some((9, 2)),
            "SetColor := (zWHITE*256) + zBLACK — 'Mac defaults: white under black'",
        );
        // And the page really is the LIGHT one, which is the whole visual
        // difference from the Amiga's dark grey — asserted as a relation rather
        // than a repeat of the pair, so a swapped tuple cannot pass.
        let (mac_bg, _) = p.default_colours().expect("the Mac states its defaults");
        let (amiga_bg, _) =
            InterpreterProfile::Amiga.default_colours().expect("so does the Amiga");
        assert_ne!(mac_bg, amiga_bg, "white page against dark grey");
        assert_eq!(p.palette(), zvm::screen::Palette::Standard, "MapColor IS §8.3.1's table");
    }

    #[test]
    fn atari_st_knobs_are_the_verified_constants() {
        // Every one of these is quoted at its constant, from ZMSD §11.1.3 and
        // from Infocom's own ST interpreters (`st/stx1.s`, `st/stzip.s`,
        // `st/xzip.c`, `st/color.note`).
        let p = InterpreterProfile::AtariSt;
        assert_eq!(p.interpreter_number(), Some(5), "ZMSD §11.1.3: 5 = Atari ST");
        assert_eq!(
            p.default_colours(),
            Some((9, 2)),
            "st/xzip.c: DEF_BACK 9 = white, DEF_FORE 2 = black",
        );
        assert_eq!(p.palette(), zvm::screen::Palette::Standard, "color_table IS §8.3.1's eight");

        // **The declined member, asserted as declined.** The ST never had a
        // YZIP, so it has no Version 6 art geometry — this is the one knob in
        // the bundle a profile other than the IBM PC answers `None` to, and it
        // must stay `None` rather than drift to a plausible-looking pair.
        assert_eq!(
            p.std_window(),
            None,
            "Infocom wrote no Version 6 interpreter for the Atari ST — there is no ST art to size",
        );

        // The ST and the Macintosh agree on the page and disagree with the
        // Amiga. Asserted as relations so a copied-and-edited tuple cannot pass.
        let (st_bg, st_fg) = p.default_colours().expect("the ST states its defaults");
        let (mac_bg, mac_fg) =
            InterpreterProfile::Macintosh.default_colours().expect("so does the Mac");
        assert_eq!((st_bg, st_fg), (mac_bg, mac_fg), "both machines default to black on white");
        let (amiga_bg, _) = InterpreterProfile::Amiga.default_colours().expect("so does the Amiga");
        assert_ne!(st_bg, amiga_bg, "white page against the Amiga's dark grey");
    }

    #[test]
    fn apple_iigs_knobs_are_the_verified_constants() {
        // Every one of these is quoted at its constant, from ZMSD §11.1.3 and
        // from Infocom's own Apple II YZIP (`apple/yzip/rel.15/apple.equ`,
        // `zboot.asm`, `machine.asm`, `tables.asm`, `bsubs.asm`).
        let p = InterpreterProfile::AppleIIgs;
        assert_eq!(p.interpreter_number(), Some(10), "ZMSD §11.1.3: 10 = Apple IIgs");
        assert_eq!(
            p.default_colours(),
            Some((2, 9)),
            "zboot.asm: `lda #2 ; black is the background color`, `lda #9 ; the color white is \
             the foreground color` — and $2C is the BACKGROUND (ZMSD §8.3.3)",
        );
        assert_eq!(p.palette(), zvm::screen::Palette::Standard, "ZIPCOLOR IS §8.3.1's eight");

        // **The declined member, asserted as declined** — and declined for a
        // different reason to the ST's, which is the whole point of having both.
        // The Apple's Version 6 screen exists and is 140x192 on a 3x9 cell; this
        // knob holds a picture space that gets doubled onto 640x400 and cut into
        // 8x16 cells, so 140x192 would state a 280x384 / 35x24 machine that never
        // existed. See `std_window`'s docs.
        assert_eq!(
            p.std_window(),
            None,
            "the Apple's 140x192 on a 3x9 cell is a different screen model, not a std window",
        );
        assert_ne!(
            p.std_window(),
            Some((140, 192)),
            "and specifically NOT the Apple's own screen numbers — this knob cannot hold them",
        );

        // **The page is genuinely black, and it is the only one that is.**
        // Asserted as relations so a copied-and-edited tuple cannot pass: the
        // Apple is the dark one against the Mac's and the ST's white, and it is
        // darker than the Amiga's dark grey rather than equal to it (SQ-0740's
        // window-0 gate turns on the Amiga NOT being black).
        let (apple_bg, apple_fg) = p.default_colours().expect("the Apple states its defaults");
        let (mac_bg, _) = InterpreterProfile::Macintosh.default_colours().expect("so does the Mac");
        let (st_bg, _) = InterpreterProfile::AtariSt.default_colours().expect("so does the ST");
        let (amiga_bg, amiga_fg) =
            InterpreterProfile::Amiga.default_colours().expect("so does the Amiga");
        assert_ne!(apple_bg, mac_bg, "black page against the Macintosh's white");
        assert_ne!(apple_bg, st_bg, "black page against the Atari ST's white");
        assert_ne!(apple_bg, amiga_bg, "black page against the Amiga's dark grey — 2 is not 12");
        // …and white ink, which the Amiga agrees on and the other two do not.
        assert_eq!(apple_fg, amiga_fg, "both machines write white on a dark page");
    }

    /// The Apple's number is a **runtime detection**, where the ST's is a flat
    /// constant and the IBM PC's is a version rule — three different shapes, and
    /// the reason this row was argued rather than read (SQ-0857).
    ///
    /// What makes 10 the honest answer is not that the medium names it: it is
    /// that DECLINING names something else. zvm's own rule would hand an Apple II
    /// story 1 or, on Version 6, 6 — the DECSystem-20 or the IBM PC. This pins
    /// the fallback that would apply, so the argument cannot rot into "None is
    /// harmless" without a test noticing.
    #[test]
    fn declining_the_apples_number_would_name_a_different_machine_entirely() {
        assert_eq!(
            InterpreterProfile::AppleIIgs.interpreter_number(),
            Some(APPLE_IIGS_INTERPRETER_NUMBER),
            "apple.equ: `IIgsID EQU 10 ; ][gs Yzip`",
        );
        // The profile a `None` would fall through to, and the numbers zvm's own
        // Frotz rule then applies — 6 for Version 6, 1 otherwise. 6 is the IBM
        // PC, which is also the value `zvm`'s `exec.rs` gates its CP437 remap on.
        let fallback = InterpreterProfile::IbmPc;
        assert_eq!(fallback.interpreter_number(), None);
        assert_eq!(
            InterpreterProfile::for_interpreter_number(6),
            InterpreterProfile::IbmPc,
            "a declined ProDOS row would land a Version 6 Apple story on the IBM PC",
        );
        assert_eq!(
            InterpreterProfile::for_interpreter_number(1),
            InterpreterProfile::IbmPc,
            "…and every other Apple story on the DECSystem-20's number",
        );
    }

    /// The ST's number is a **flat constant**, where the IBM PC's is a rule —
    /// which is the whole reason one row in `blorb::medium::FORMATS` answers
    /// `Some` and its filesystem-identical neighbour answers `None`.
    #[test]
    fn the_atari_st_states_one_number_where_the_ibm_pc_states_a_rule() {
        assert_eq!(
            InterpreterProfile::AtariSt.interpreter_number(),
            Some(ATARI_ST_INTERPRETER_NUMBER),
            "st/stx1.s: INTWRD DC.B 5 — no version arm, no condition",
        );
        assert_eq!(
            InterpreterProfile::IbmPc.interpreter_number(),
            None,
            "the IBM PC's honest number is version-dependent, so zvm's own rule stays in force",
        );
    }

    #[test]
    fn the_v6_cell_matches_what_zvm_advertises() {
        // Knob 6: stated for completeness, pinned so it cannot silently drift.
        // **The Macintosh is the one that differs, and since SQ-0917 it says so.**
        // `mac/xzip.lst` sets `colWidth := 7` and `lineHeight := 15 {16}`, and the
        // 1:1 captures in `machine-screenshots/` agree four ways — the inverse
        // PROLOGUE bar is 15 rows, the topic list indexes as `118 + 15*i`, the
        // colour press's prose tops are 15 apart, and the insertion caret is 1x15.
        // **The Macintosh is the one that differs, and it says so.** `mac/xzip.lst`
        // sets `colWidth := 7` and `lineHeight := 15 {16}`, and the 1:1 captures in
        // `machine-screenshots/` agree four ways — the inverse PROLOGUE bar is 15
        // rows, the topic list indexes as `118 + 15*i`, the colour press's prose
        // tops are 15 apart, and the insertion caret is 1x15.
        assert_eq!(
            InterpreterProfile::Macintosh.v6_font_cell(),
            zvm::interpreter::MACINTOSH_V6_CELL,
            "the Macintosh declares Geneva 12's metric, not zvm's default",
        );

        // Every other machine answers the default. The Apple IIgs's real 3x9
        // and EGA's 8x8 remain unexpressed for the reasons `v6_font_cell` gives —
        // the IIgs in particular needs its cell and its screen in ONE coordinate
        // system, and today they are not (SQ-0863).
        for p in [
            InterpreterProfile::IbmPc,
            InterpreterProfile::Amiga,
            InterpreterProfile::AtariSt,
            InterpreterProfile::AppleIIgs,
        ] {
            assert_eq!(
                p.v6_font_cell(),
                zvm::screen::V6Cell::DEFAULT,
                "{p:?} v6 cell",
            );
        }

        // And the default a bare `Machine` carries is still the pair this crate
        // has always advertised, so a story with no profile is told what it always
        // was. `V6Cell::DEFAULT` is the single source of that truth now.
        assert_eq!(
            (zvm::screen::V6Cell::DEFAULT.w(), zvm::screen::V6Cell::DEFAULT.h()),
            (zvm::screen::V6_FONT_WIDTH, zvm::screen::V6_FONT_HEIGHT),
            "V6Cell::DEFAULT and the constants must not drift apart",
        );
    }

    #[test]
    fn an_explicit_interpreter_number_selects_the_whole_profile() {
        // SQ-0734 precedence 1, and the fix for the incoherent machine the user
        // hit: asking for interpreter 4 asks for the Amiga, not just the byte.
        assert_eq!(InterpreterProfile::for_interpreter_number(4), InterpreterProfile::Amiga);
        assert_eq!(InterpreterProfile::for_interpreter_number(3), InterpreterProfile::Macintosh);
        assert_eq!(InterpreterProfile::for_interpreter_number(5), InterpreterProfile::AtariSt);
        assert_eq!(InterpreterProfile::for_interpreter_number(10), InterpreterProfile::AppleIIgs);
        assert_eq!(InterpreterProfile::for_interpreter_number(7), InterpreterProfile::Commodore128);
        // **2 and 9 joined the list** (SQ-0872). They are the Apple IIe and IIc,
        // the other two machines the Apple II YZIP runs on, and SQ-0857 scoped
        // itself to the IIgs — so asking for one of them used to get an IBM PC
        // wearing an Apple's number. They were nearly free: `bsubs.asm`'s
        // `MACHINE:` picks between the family's three numbers at boot, *after*
        // `zboot.asm` has seeded the one page all three share.
        assert_eq!(InterpreterProfile::for_interpreter_number(2), InterpreterProfile::AppleIIe);
        assert_eq!(InterpreterProfile::for_interpreter_number(9), InterpreterProfile::AppleIIc);
        // **And 8 joined it** (SQ-0873). The Commodore 64 states nothing a story
        // can read — no palette, no `$2C`/`$2D` pair, for want of an Infocom
        // Commodore interpreter to read them out of — but it has a measured period
        // look, and a variant is what a profile needs to carry one. No medium
        // selects it: a `.d64` is a 1541 image both Commodore machines read, so 8
        // is reached only by asking for it.
        assert_eq!(InterpreterProfile::for_interpreter_number(8), InterpreterProfile::Commodore64);
        // Every other number is served by the IBM PC bundle, the historical
        // default: 1 the DECSystem-20, 11 the Tandy Color. Each is absent for a
        // stated reason — see `zvm::interpreter::MACHINES` — and each is stated
        // here so the gap is visible rather than assumed closed.
        for n in [1u8, 11] {
            assert_eq!(
                InterpreterProfile::for_interpreter_number(n),
                InterpreterProfile::IbmPc,
                "interpreter {n}",
            );
            // …and the fallback is no longer SILENT: the profile can say it does
            // not model the machine, which is what `zvm-cli` warns on.
            assert_eq!(
                InterpreterProfile::try_for_interpreter_number(n),
                None,
                "interpreter {n} must report the gap rather than answer IbmPc",
            );
        }
        // 6 is the IBM PC itself, so it is MODELLED and answers rather than
        // falling through — the distinction the two functions exist to make.
        assert_eq!(
            InterpreterProfile::try_for_interpreter_number(6),
            Some(InterpreterProfile::IbmPc),
        );
    }

    /// The Apple family is one bundle with three numbers (SQ-0872): everything a
    /// story can read is identical but `$1E`, because `zboot.asm` seeds the page
    /// before `bsubs.asm`'s `MACHINE:` picks the number.
    #[test]
    fn the_two_new_apples_are_the_iigs_bundle_with_a_different_number() {
        let gs = InterpreterProfile::AppleIIgs;
        for (p, n) in [(InterpreterProfile::AppleIIe, 2u8), (InterpreterProfile::AppleIIc, 9)] {
            assert_eq!(p.interpreter_number(), Some(n), "bsubs.asm MACHINE: IIeID 2 / IIcID 9");
            assert_eq!(p.default_colours(), gs.default_colours(), "zboot.asm seeds one page");
            assert_eq!(p.default_colours(), Some((2, 9)), "black page, white ink");
            assert_eq!(p.palette(), gs.palette(), "ZIPCOLOR IS §8.3.1's eight");
            assert_eq!(p.std_window(), None, "140x192 on a 3x9 cell is a different screen model");
            assert_eq!(p.v6_font_cell(), gs.v6_font_cell());
            assert_ne!(p.interpreter_number(), gs.interpreter_number(), "…but not the number");
        }
    }

    /// Every knob a story can READ comes off `zvm`'s table, which is the whole
    /// point of SQ-0872 — so the two must agree for every profile that states a
    /// number, or the CLI and the TUI would present different machines.
    #[test]
    fn every_profile_agrees_with_the_zvm_machine_table() {
        for p in [
            InterpreterProfile::IbmPc,
            InterpreterProfile::Amiga,
            InterpreterProfile::Macintosh,
            InterpreterProfile::AtariSt,
            InterpreterProfile::AppleIIgs,
            InterpreterProfile::AppleIIe,
            InterpreterProfile::AppleIIc,
            InterpreterProfile::Commodore128,
            InterpreterProfile::Commodore64,
        ] {
            let Some(n) = p.interpreter_number() else {
                // Only the IBM PC declines a number, and it does so because its
                // number is a version RULE rather than a constant.
                assert_eq!(p, InterpreterProfile::IbmPc);
                continue;
            };
            let row = zvm::interpreter::machine(n)
                .unwrap_or_else(|| panic!("{p:?} states {n} but zvm models no such machine"));
            assert_eq!(p.default_colours(), row.default_colours, "{p:?} $2C/$2D");
            assert_eq!(p.palette(), row.palette, "{p:?} palette");
            assert_eq!(InterpreterProfile::for_interpreter_number(n), p, "{p:?} round trip");
        }
    }

    #[test]
    fn a_missing_file_is_not_a_disk_image() {
        let missing = std::path::Path::new("/nonexistent/lanthorn/no-such-story.z6");
        assert_eq!(InterpreterProfile::resolve(missing, None, None, None), InterpreterProfile::IbmPc);
        // …and an explicit number still decides without ever touching the disk.
        assert_eq!(InterpreterProfile::resolve(missing, Some(4), None, None), InterpreterProfile::Amiga);
    }

    #[test]
    fn a_named_archives_flavour_names_the_machine() {
        // SQ-0734 precedence 2. MCGA/EGA/CGA are three cards on ONE machine.
        assert_eq!(InterpreterProfile::for_art_flavour(Flavour::Pc), InterpreterProfile::IbmPc);
        assert_eq!(
            InterpreterProfile::for_art_flavour(Flavour::AmigaMac),
            InterpreterProfile::Amiga,
        );
    }

    #[test]
    fn the_named_archive_outranks_the_medium_and_yields_to_an_explicit_number() {
        let plain = std::path::Path::new("/nonexistent/lanthorn/no-such-story.z6");
        // Naming an Amiga archive beside an ordinary file makes it an Amiga…
        assert_eq!(
            InterpreterProfile::resolve(plain, None, Some(Flavour::AmigaMac), None),
            InterpreterProfile::Amiga,
        );
        // …and an explicit number still outranks it (precedence 1 over 2).
        assert_eq!(
            InterpreterProfile::resolve(plain, Some(6), Some(Flavour::AmigaMac), None),
            InterpreterProfile::IbmPc,
        );
    }

    /// The one ambiguity a codec cannot settle, settled by the disk under it
    /// (SQ-0843). `Flavour::AmigaMac` is one container written by two machines,
    /// so on release media the medium answers; `Flavour::Pc` is unambiguous and
    /// still beats the medium outright, which is what keeps naming an archive an
    /// instruction rather than a hint.
    #[test]
    fn the_medium_settles_which_machine_an_amiga_mac_archive_belongs_to() {
        use blorb::medium::DiskImage;
        assert_eq!(
            InterpreterProfile::for_art_flavour_on(Flavour::AmigaMac, Some(DiskImage::Hfs)),
            InterpreterProfile::Macintosh,
            "an Amiga/Mac archive on an Apple filesystem is the Macintosh's",
        );
        assert_eq!(
            InterpreterProfile::for_art_flavour_on(Flavour::AmigaMac, Some(DiskImage::Adf)),
            InterpreterProfile::Amiga,
        );
        // No medium: the archive's own answer, exactly as before.
        assert_eq!(
            InterpreterProfile::for_art_flavour_on(Flavour::AmigaMac, None),
            InterpreterProfile::Amiga,
        );
        // An unambiguous flavour is never refined — an `.mg1` named on a
        // Macintosh disk asks for the IBM PC and gets it.
        for medium in [None, Some(DiskImage::Hfs), Some(DiskImage::Adf)] {
            assert_eq!(
                InterpreterProfile::for_art_flavour_on(Flavour::Pc, medium),
                InterpreterProfile::IbmPc,
                "{medium:?}",
            );
        }
    }

    #[test]
    fn naming_a_pc_archive_cannot_move_header_byte_1e() {
        // The blast-radius pin. `zvm`'s `exec.rs` branches on `$1E == 6`, and
        // every v6 story gets that today from zvm's own Frotz rule. Naming a
        // `.MG1`/`.EG1`/`.CG1` selects IBM PC, which has NO opinion on the
        // number — so the byte the story reads is the byte it read before, and
        // the whole v6 corpus is untouched by this feature by construction.
        let profile = InterpreterProfile::for_art_flavour(Flavour::Pc);
        assert_eq!(profile, InterpreterProfile::IbmPc);
        assert_eq!(profile.interpreter_number(), None, "zvm's rule stays in force");
    }
}
