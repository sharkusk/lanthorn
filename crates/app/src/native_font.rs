//! The typeface a release shipped on its own medium (SQ-1011).
//!
//! A companion to [`crate::graphics::PictSource`], and resolved the same way: the
//! MEDIUM is asked what it carries, and the answer is used only when it fits the
//! machine the medium named. Pictures were the first thing a release disk had that
//! beat lanthorn's own; the typeface is the second.
//!
//! # Why the Macintosh needed this and the others do not
//!
//! SQ-0917 gave the Macintosh its own Version 6 cell — **7x15**, which is what
//! `mac/xzip.lst` declares (`colWidth := 7; lineHeight := 15 {16}`) and what four
//! independent measurements off 1:1 captures confirm. That fixed the story's
//! layout and left its TEXT worse, because `crate::render::vga16` is drawn for an
//! **8-pixel advance**: 76 of its 94 printable glyphs ink out to column 6, so
//! column 7 is their entire inter-character gap and a 7-wide cell drops it. The
//! letters end up touching.
//!
//! The disk has the answer on it. Infocom shipped `FONT` 524 — 7x15, fixed-pitch
//! across printable ASCII, with left side bearings — which is exactly the cell,
//! so it blits **1:1 with no resampling on either axis**. That is the property
//! `vga16` was introduced for (SQ-0932) and precisely what it cannot deliver here.
//!
//! Every other machine still runs an 8x16 cell where `vga16` already blits 1:1,
//! so none of them reach this module.
//!
//! # The metric and the face are different facts
//!
//! Worth stating because they are easy to conflate: the Macintosh **declared** a
//! fixed 7 while **painting** proportional Geneva 12 — measured on
//! `machine-screenshots/mac-zorkzero-hint.png`, where the same `WING` glyph run
//! starts at x=137 in `EAST WING` and x=139 in `WEST WING`. `FONT` 524 is a
//! fixed-pitch resource the same release shipped. lanthorn renders on a fixed
//! cell, so the fixed face is the one that fits; see [`zvm::screen::V6Cell`] for
//! the declared-versus-drawn boundary this sits on.

use crate::interpreter::{InterpreterProfile, ProfileSource};
use crate::render::bitfont::STYLE_BOLD;
use blorb::bitmap_font::BitmapFont;
use std::path::Path;

/// Everything the face cascade is asked about ONE launch (SQ-1037).
///
/// # Why a value and not six parameters
///
/// CLAUDE.md's refactoring policy, and the same tell as [`TextFace`]: every field
/// here comes from the same scope — the launch, or the restart that repeats it —
/// and a caller who supplies a subset gets a plausible answer rather than an
/// error. `art_scale` is the one that would go missing, exactly as
/// `native_std_window` and the Version 6 cell did four times over (SQ-0901,
/// SQ-1020, SQ-1021, SQ-1022), and its absence is invisible: on the Macintosh the
/// text scale is `(1, 1)` whatever the artwork does, so an omission there is
/// silent on the only press that reads a system face today.
pub struct FaceRequest<'a> {
    /// The story file or disk image this session opened.
    pub story_path: &'a Path,
    /// Which story on a multi-game image — [`crate::config::Config::disk_entry`].
    pub entry: Option<&'a str>,
    /// The machine, as the MEDIUM named it.
    pub profile: InterpreterProfile,
    /// How it was named — see [`resolve`] for why the release rung gates on this
    /// and the system rung does not.
    pub source: ProfileSource,
    /// The archive's art density (SQ-0790), converted to a TEXT scale inside —
    /// `None` for an undoubled rendition.
    pub art_scale: Option<(u32, u32)>,
    /// The player's own boot disks, or `None` to consult none at all: a test that
    /// must not depend on what the person running it keeps in `~/.lanthorn/`, and
    /// every harness that predates this.
    pub disks: Option<&'a crate::system_fonts::UserDisks>,
}

impl FaceRequest<'_> {
    /// The archive's density, with `None` meaning an undoubled rendition — the
    /// number every face space is resolved against (SQ-0790).
    fn art(&self) -> (u32, u32) {
        self.art_scale.unwrap_or((1, 1))
    }

    /// Native pixels per FACE pixel for a face off the player's own BOOT MEDIA,
    /// which is not the number a face off the RELEASE's medium takes (SQ-1053).
    ///
    /// The Amiga is the machine where the two differ: its releases author a face
    /// in the 320-wide picture space and its system topaz is drawn in the 640x200
    /// hires mode, so this is (1, 2) on a doubled press where
    /// [`InterpreterProfile::release_face_space`] gives (2, 2). Getting it from
    /// the row instead is what would size topaz 8 as a 16x16 face and refuse it.
    fn system_text_scale(&self) -> (u32, u32) {
        self.profile.system_face_space().text_scale(self.art())
    }
}

/// The space an admitted face's bitmaps are authored in, from WHERE IT CAME FROM
/// and the machine drawing it.
///
/// # The provenance decides, not the machine alone
///
/// SQ-1053. One machine can want two different scales at once: the Amiga draws
/// *Arthur*'s `char.data` in the picture space (its ten face rows are the
/// twenty-row line `machine-screenshots/amiga-arthur-text.png` measures) and
/// topaz in the 640x200 hires space (its eight rows are the sixteen-row line
/// `machine-screenshots/amiga-shogun-game.png` measures). A single number per
/// machine can express one of those and silently mis-scales the other.
///
/// It is derived from the ORIGIN the cascade already records rather than stored
/// beside it, so a face re-tested against a changed profile — which is exactly
/// what `reload.rs` does on every style reload — is re-scaled against the new
/// machine instead of carrying the old one's answer forward.
fn face_space(
    profile: InterpreterProfile,
    origin: Option<&FaceOrigin>,
) -> zvm::interpreter::V6FaceSpace {
    match origin {
        Some(FaceOrigin::SystemDisk { .. }) => profile.system_face_space(),
        Some(FaceOrigin::Release) | None => profile.release_face_space(),
    }
}

/// Where an admitted face came from, for the info panel and for a report.
///
/// Worth carrying because a System 7 Geneva must not quietly stand in for a
/// System 6 one: two disks can hand back a face that reads identically and came
/// off different releases of the operating system, and the only honest answer to
/// "which" is the disk's own name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaceOrigin {
    /// The story's own medium — the release shipped it.
    Release,
    /// A boot MEDIUM the player put under `~/.lanthorn/` — a Mac OS System disk,
    /// a Workbench floppy, or an Amiga Kickstart ROM (SQ-1053).
    ///
    /// A ROM is not a disk and the field names say "disk" anyway, because what
    /// this records is the FILE that answered and the rung it answered on. There
    /// is one system rung, not one per kind of medium, and a variant per medium
    /// would be a second spelling of the same fact for every reader downstream to
    /// match on.
    SystemDisk {
        /// The file's name, directly under `~/.lanthorn/`.
        disk: String,
        /// How that medium names the face — `FONT 396`, `fonts/topaz/11`,
        /// `topaz/8`.
        name: String,
    },
}

/// The faces a machine draws with: its BODY face and its FIXED-PITCH alternate.
///
/// # Two faces because the machines had two
///
/// `mac/xzip.lst` names them in as many words — `ZSTD: TextFont (stdFont)` with
/// `stdFont := geneva`, and `ZMONO: TextFont (monaco)` — and
/// `machine-screenshots/mac-zorkzero-game.png` shows both on one screen: `Banquet
/// Hall` in the status bar steps a uniform 7 px per character while the prose two
/// lines below advances 7, 7, 5. *Zork Zero* brackets that bar in `@set_font 4` /
/// `@set_font 1`, which `zvm` folds into §8.7.1's fixed-pitch bit so that one
/// question — [`TextFace::face_for`] — answers for both halves.
///
/// # And the two rungs supply different halves
///
/// The Macintosh's release medium carries `FONT` 524, Monaco 12, which IS its 7x15
/// cell — so the game disk answers for the alternate and cannot answer for the
/// body, because Geneva is in the System file and on no Infocom platter at all
/// (SQ-1036). A player's own System disk answers for the body. Arthur's Amiga
/// floppy is the mirror image: `char.data` is a body face and there is no
/// alternate.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FaceSet {
    body: Option<BitmapFont>,
    fixed: Option<BitmapFont>,
    body_origin: Option<FaceOrigin>,
    fixed_origin: Option<FaceOrigin>,
}

impl FaceSet {
    /// No face at all — a bare story, or a machine whose media carry none.
    pub fn none() -> FaceSet {
        FaceSet::default()
    }

    /// One face off a release's own medium, sorted into whichever role [`fit`]
    /// gives it on `profile` — the first rung of [`resolve`]'s cascade, on its own.
    ///
    /// For a caller that already holds a face and only needs it paired: a harness
    /// with a synthetic one, and `reload.rs`'s re-test against a changed profile.
    /// The RANKING is still `resolve`'s and this cannot express one, which is the
    /// point — there is no way to spell "system face" here.
    pub fn release(
        face: BitmapFont,
        profile: InterpreterProfile,
        art_scale: Option<(u32, u32)>,
    ) -> FaceSet {
        let mut set = FaceSet::none();
        set.admit(face, profile, art_scale.unwrap_or((1, 1)), FaceOrigin::Release);
        set
    }

    /// The face body text is drawn with.
    pub fn body(&self) -> Option<&BitmapFont> {
        self.body.as_ref()
    }

    /// The face a §8.7.1 fixed-pitch run is drawn with, where the machine has one.
    pub fn fixed(&self) -> Option<&BitmapFont> {
        self.fixed.as_ref()
    }

    /// Where the body face came from.
    pub fn body_origin(&self) -> Option<&FaceOrigin> {
        self.body_origin.as_ref()
    }

    /// Where the fixed-pitch alternate came from — the other half of the same
    /// question, and not the same answer: the Macintosh's release medium supplies
    /// the alternate and a player's System disk the body (SQ-1036), so a report
    /// that showed only one of them would name the wrong disk half the time
    /// (SQ-1047).
    pub fn fixed_origin(&self) -> Option<&FaceOrigin> {
        self.fixed_origin.as_ref()
    }

    /// Whether `face` is one of the two this set actually draws with — what the
    /// info panel's `used` column asks, and asked of the CASCADE's answer rather
    /// than re-derived (see [`detected`]).
    pub fn draws(&self, face: &BitmapFont) -> bool {
        self.body.as_ref() == Some(face) || self.fixed.as_ref() == Some(face)
    }

    /// Offer a face to the set, in cascade order: it lands in whichever slot
    /// [`fit`] names, and the FIRST offer for a slot keeps it.
    ///
    /// **This is the only place a face is sorted into a role**, and it asks `fit`
    /// rather than repeating its rule — SQ-1011 shipped INERT TWICE because the
    /// fitness test existed in two places and only one of them was corrected.
    ///
    /// A [`FaceFit::Cell`] face also becomes the BODY when nothing proportional
    /// has been admitted, which is every configuration that shipped before this:
    /// the Macintosh with no System disk keeps drawing all of its text in Monaco,
    /// byte for byte as it did.
    fn admit(
        &mut self,
        face: BitmapFont,
        profile: InterpreterProfile,
        art_scale: (u32, u32),
        origin: FaceOrigin,
    ) {
        // The face's OWN scale, from its provenance — see [`face_space`]. Judging a
        // system face at the release space is what refused topaz 8 outright: at (2,
        // 2) its eight rows measure sixteen NATIVE and its eight columns sixteen
        // too, against an eight-wide cell (SQ-1053).
        let scale = face_space(profile, Some(&origin)).text_scale(art_scale);
        match fit(&face, profile, scale) {
            Some(FaceFit::Metric) => {
                if self.body.is_none() || self.body_is_borrowed_alternate() {
                    self.body_origin = Some(origin);
                    self.body = Some(face);
                }
            }
            Some(FaceFit::Cell) if self.fixed.is_none() => {
                if self.body.is_none() {
                    self.body = Some(face.clone());
                    self.body_origin = Some(origin.clone());
                }
                self.fixed_origin = Some(origin);
                self.fixed = Some(face);
            }
            Some(FaceFit::Cell) => {}
            None => {}
        }
    }

    /// Whether the body slot is holding the fixed alternate for want of anything
    /// better — the state a later proportional face is allowed to displace.
    fn body_is_borrowed_alternate(&self) -> bool {
        self.body.is_some() && self.body == self.fixed
    }
}

/// The faces `request` resolves to — the ONE statement of the order.
///
/// # The order
///
/// 1. **the release's own face, on the story's own medium** — Arthur's Amiga
///    `char.data`, the Macintosh's `FONT` 524. It is the release's, so it is the
///    most specific thing anyone has;
/// 2. **the machine's system face, off a boot medium the player supplied** —
///    Geneva out of a Mac OS System file, topaz out of a Workbench `FONTS:`
///    drawer or, on the machine that keeps its real face there, out of a
///    **Kickstart ROM** (SQ-1053). The machine NAMES it
///    ([`InterpreterProfile::v6_system_face`]) and the player supplies it, so it
///    is the machine's own face without lanthorn shipping a byte of it;
/// 3. **the built-in**, which is no face at all here: [`FaceSet::none`] leaves the
///    renderer on `crate::render::vga16` exactly as before.
///
/// This is the existing "a release disk's own resources outrank a `.blb` beside
/// the story" rule extended one notch, and it keeps SQ-1016's embedded substitute
/// useful rather than redundant — it is what CI and a player with no boot disk get.
///
/// # Why only the FIRST rung gates on `ProfileSource`
///
/// The release's face lives in the story's own medium, so it exists only when the
/// MEDIUM named the machine: `--interpreter 3` beside a bare `.z6` reaches
/// `Macintosh` with no volume to read. A SYSTEM disk is a different medium
/// entirely — the player's — so a machine asked for by hand can still be drawn
/// with its own face, and a machine nobody named resolves to `IbmPc`, which names
/// no system face and therefore reads nothing.
///
/// # Which SIZE, on a rung that offers seven
///
/// A release ships one face and states its own line height, and the declared cell
/// FOLLOWS it ([`declared_cell`]). A System disk ships a family — Geneva at 9, 10,
/// 12, 14, 18, 20 and 24 point on `MacOS_6.0.8_System_Startup.img` — and the
/// machine drew with exactly one of them. The machine says which: `mac/xzip.lst`
/// declares `lineHeight := 15`, so the size whose face is fifteen rows tall is the
/// size it painted, and that is Geneva 12 (`FONT` 396, 15x15). Measured, not
/// chosen: `machine-screenshots/mac-zorkzero-game.png` puts consecutive prose
/// baselines 15 rows apart (y = 136, 151, 166, 181, 196, 211).
///
/// The comparison is in NATIVE pixels, so it goes through the TEXT scale rather
/// than the art scale (SQ-1039) — and the SYSTEM face's text scale, which on the
/// Amiga is not the one its releases take (SQ-1053). `(1, 1)` on every Macintosh
/// press; `(1, 2)` for topaz on a doubled Amiga press, which is what makes topaz
/// 8's eight rows the sixteen-row line and topaz 11's eleven a refusal. The
/// Macintosh colour press cannot falsify that on its own and the monochrome one
/// cannot falsify it at all.
pub fn resolve(request: &FaceRequest<'_>) -> FaceSet {
    let mut set = FaceSet::none();
    // Rung 1 — the release's own medium.
    if request.source == ProfileSource::Medium {
        if let Some(face) = release_face(request.story_path, request.entry) {
            set.admit(face, request.profile, request.art(), FaceOrigin::Release);
        }
    }
    // Rung 2 — the machine's own system face, off a boot medium the player supplied.
    if let Some(disks) = request.disks {
        let scale = request.system_text_scale();
        let cell = request.profile.v6_font_cell();
        for found in crate::system_fonts::named_faces_in(disks, request.profile) {
            if u32::from(found.font.height) * scale.1 != u32::from(cell.h()) {
                continue; // a size of the family the machine did not draw with
            }
            set.admit(
                found.font,
                request.profile,
                request.art(),
                FaceOrigin::SystemDisk { disk: found.disk, name: found.name },
            );
        }
    }
    set
}

/// The face the STORY's own medium carries, whichever kind of medium it is.
///
/// An HFS volume keeps its faces in a resource fork; every other medium keeps them
/// as FILES, and Arthur's Amiga floppy is the second kind. Ask the volume that can
/// answer, and let [`FaceSet::admit`] decide what came back is good for.
///
/// # And it is paired with ONE story, not with the disc
///
/// `entry` is which story on the image the session opened, as
/// [`crate::config::Config::disk_entry`] spells it. It matters because a
/// compilation carries many applications and only one of them is the game being
/// played — see [`blorb::mac_font::from_volume_beside`], and SQ-1018 for the
/// Masterpieces CD, where the first application on the platter ships no `FONT`
/// and every graphical game on it therefore drew its 7x15 cell with the 8-wide
/// fallback.
fn release_face(story_path: &Path, entry: Option<&str>) -> Option<BitmapFont> {
    let file = blorb::image::open_disk(story_path)?;
    match blorb::hfs::Hfs::mount_file(&file) {
        Ok(hfs) => {
            // `entry` is `None` for every loose file and single-story floppy — and
            // also for a direct launch of a multi-game image, where `Hfs::story` is
            // the thing that CHOSE the story, so asking it again names the same one
            // rather than guessing. That is what makes
            // `lanthorn InfocomMasterpieces.img` pair correctly without a picker
            // row behind it.
            let opened = entry.map(str::to_string).or_else(|| hfs.story().map(|(p, _)| p));
            match opened {
                Some(p) => blorb::mac_font::from_volume_beside(&hfs, &p),
                None => blorb::mac_font::from_volume(&hfs),
            }
        }
        Err(_) => amiga_face(story_path),
    }
}

/// Is `story_path` a CD image read by sector? An AmigaDOS disk font is not on
/// one — and finding that out by content means reading every file on it, which
/// is the disc (SQ-1761).
fn on_a_cd(story_path: &Path) -> bool {
    blorb::image::open_disk(story_path).is_some_and(|f| f.is_file_backed())
}

/// The disk font an AmigaDOS volume carries, if it carries one.
///
/// Split out because it is the same lookup [`detected`] performs and the two must
/// not drift: SQ-1011 shipped inert twice over a fitness rule that existed in two
/// places, and a second copy of the LOOKUP would be the same defect one layer down.
fn amiga_face(story_path: &Path) -> Option<BitmapFont> {
    if on_a_cd(story_path) {
        return None;
    }
    let files: Vec<(String, Vec<u8>)> = crate::assets::files(story_path)
        .into_iter()
        .filter(|f| f.is_on_medium())
        .filter_map(|f| {
            let name = f.name.clone();
            f.into_bytes().map(|b| (name, b))
        })
        .collect();
    blorb::amiga_font::from_volume(files.iter().map(|(p, b)| (p.as_str(), b.as_slice())))
}

/// How a release's own face may be drawn, where it may be drawn at all.
///
/// The two are not degrees of the same thing. [`FaceFit::Cell`] is a face that IS
/// the character cell and blits into it untouched; [`FaceFit::Metric`] is a face
/// whose own metrics REPLACE the cell's, and adopting it changes what the story is
/// told (SQ-1009). A caller has to know which it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceFit {
    /// Same width, same height, one advance across printable ASCII — blit 1:1.
    /// The Macintosh's `FONT` 524.
    Cell,
    /// A proportional typeface: the pen advances per glyph and the line is the
    /// face's own height. Arthur's Amiga `char.data`.
    Metric,
}

/// Which of the two ways — if either — this face may be drawn on `profile`.
///
/// # `Cell`: the face IS the cell
///
/// This is the guard that keeps SQ-1011's fix from becoming the defect it replaces.
/// A fixed face drawn for a different advance has to be resampled into the cell,
/// and resampling is exactly what made `vga16` crowd at 7 wide. Better to keep the
/// known face than to introduce a second, differently-wrong one — so a mismatch
/// declines rather than scales. It also keeps the two facts honest about each
/// other: if a future profile moves its cell and the shipped face no longer
/// matches, this notices instead of silently drawing at the wrong pitch.
///
/// # `Metric`: the face brings its own
///
/// A PROPORTIONAL face has no single advance to match a cell against, so the test
/// above can never admit one and the resampling argument does not apply — there is
/// nothing to resample it to. What it has instead is a real advance per glyph and a
/// real line height, and a machine that shipped one drew text with them. So it is
/// admitted on being a typeface at all, and the cell follows it
/// ([`declared_cell`]) rather than the other way round.
///
/// **Proportional is also what separates a typeface from the font-3 SET**, which is
/// the trap this ordering avoids. Journey and Beyond Zork ship an 8x8 `Char.data` /
/// `Graphic.Data` that parses identically and is a box-drawing graphics set, not
/// letters — its code 65 is a solid block (SQ-1017). Both are fixed-pitch and carry
/// null `tf_CharSpace`/`tf_CharKern`, so both fall to the `Cell` test and are
/// declined there for not being the cell, exactly as they were before this existed.
///
/// # Why the printable range and not `BitmapFont::proportional`
///
/// That flag is measured over every non-blank glyph in the resource, and `FONT`
/// 524's Mac-roman accented range genuinely does vary — so it answers `true` for a
/// face that advances by exactly 7 across `!` to `~`, which is the only part a story
/// prints. SQ-0916 recorded this ("called proportional — true only if you count the
/// accented high range, which no game prints") and
/// `the_macintosh_font_is_fixed_pitch_but_narrower_than_our_cell` measures the
/// printable set as exactly `{7}`.
///
/// Gating on the flag is what made SQ-1011 ship INERT: the face resolved, failed
/// here, and the renderer silently kept `vga16` while four before/after frames came
/// back byte-identical. Ask the question the renderer actually depends on — does
/// every character a game prints advance by the same amount. That same question,
/// answered the other way, is what now admits the Amiga's.
///
/// # The `scale` is the FACE's, and it is what lets topaz 8 in
///
/// SQ-1053. `scale` is native pixels per FACE pixel, resolved from the face's own
/// provenance ([`face_space`]) — never the art scale, and never the machine's row
/// on its own. A face IS the cell when it is the cell **once drawn**, and the
/// Amiga's system topaz is 8x8 drawn at (1, 2), which is exactly the 8x16 that
/// machine declares. Measured on `machine-screenshots/amiga-shogun-game.png`: ten
/// distinct scanlines across a twenty-row band, ~8 native px per character.
///
/// The three tests below are separate on purpose, because collapsing them is how
/// this admits something it must not:
///
/// * **fixed pitch** is "every printable glyph advances the SAME", asked of the
///   face alone. It is deliberately not "advances by one cell": *Journey*'s and
///   *Beyond Zork*'s 8x8 `Char.data` is a font-3 BOX-DRAWING set, not letters
///   (SQ-1017), and folding the cell into this test would make it read as
///   proportional and admit it as a typeface;
/// * **the pitch fills the cell** — `advance * scale.0 == cell.w`;
/// * **the bitmap fills the cell** — `width * scale.0 == cell.w` and
///   `height * scale.1 == cell.h`.
///
/// A fixed-pitch face failing either of the last two declines, exactly as
/// `fonts/topaz/11` does (8x11 is 22 native rows against a 16-row cell) and
/// exactly as those font-3 sets do. Only a genuinely PROPORTIONAL face is admitted
/// without measuring against the cell, because it has no single advance to measure
/// — and the NAME filter in [`crate::system_fonts::named_faces_in`] is what keeps
/// that door from swinging open for a Workbench display face, which is why it is
/// load-bearing rather than decorative.
pub fn fit(
    face: &BitmapFont,
    profile: InterpreterProfile,
    scale: (u32, u32),
) -> Option<FaceFit> {
    let cell = profile.v6_font_cell();
    let mut advances = (b'!'..=b'~').filter_map(|c| face.glyph(c)).map(|g| u32::from(g.width));
    // A face covering none of printable ASCII has no advance to disagree with
    // itself, so it falls to the nominal cell below and is measured there.
    let pitch = advances.next();
    let fixed = pitch.is_none_or(|first| advances.all(|a| a == first));
    if !fixed {
        return Some(FaceFit::Metric);
    }
    let (sx, sy) = (scale.0.max(1), scale.1.max(1));
    let fills = pitch.unwrap_or(u32::from(face.width)) * sx == u32::from(cell.w())
        && u32::from(face.width) * sx == u32::from(cell.w())
        && u32::from(face.height) * sy == u32::from(cell.h());
    fills.then_some(FaceFit::Cell)
}

/// The Version 6 cell a machine declares once its own face has been admitted.
///
/// # The cell follows the FACE, and only when the face brings metrics
///
/// [`FaceFit::Cell`] and no face at all both answer `profile.v6_font_cell()`
/// unchanged, so every configuration that shipped before SQ-1009 lands on the same
/// number it always did. A [`FaceFit::Metric`] face is the one case where the
/// machine's table is not the whole story: it is a real typeface off the release's
/// own disk, drawn on its own line, and Arthur's Amiga floppy is the release that
/// has one.
///
/// # Why the HEIGHT moves and the WIDTH does not
///
/// The height is MEASURED. Arthur's `char.data` is 10 rows, `machine-screenshots/
/// amiga-arthur.png` reads a text pitch of exactly 10 in the machine's own 320x200
/// frame, and the four 2x captures read 20 — the same fact twice, and the reason a
/// scale multiplies it: a v6 coordinate is a NATIVE pixel, and on an Amiga press
/// whose art doubles onto the unit screen one FACE row is two native rows. For
/// Arthur that is a 20-row line against the 16 we declared, which is the 20 text
/// rows the machine shows against our 25.
///
/// **The scale is the TEXT scale and not the art scale** (SQ-1039). They coincide
/// on the Amiga, which draws its face in the picture space, and they do not on the
/// Macintosh, whose colour press doubles `CPic.data` onto the unit screen while
/// painting text at one native pixel per face pixel — so Geneva 12's fifteen rows
/// would be declared as thirty. `InterpreterProfile::text_scale` resolves it from
/// `zvm::interpreter::V6FaceSpace`; the monochrome press hides the difference,
/// because `Pic.data` is 480x300 at (1, 1) and 15 x 1 is still 15.
///
/// The width is NOT measured, and there is nothing here to measure. A proportional
/// face has no single advance — that is what makes it proportional — so any number
/// picked for `$27` would be a guess, and this repo does not guess declared
/// metrics (the Macintosh states `colWidth := 7` in `mac/xzip.lst` while painting
/// proportional Geneva; no equivalent Amiga listing survives, see SQ-1009). The
/// profile's width therefore stands, and what the story is TOLD about its columns
/// is unchanged. Only the DRAWING is proportional, which is the split
/// [`zvm::screen::V6Cell`] exists to name.
///
/// `art_scale` is [`crate::machine_boot::MachineBoot::art_scale`]; `None` there
/// means an undoubled rendition and should arrive here as `(1, 1)`. It is converted
/// to a text scale inside, so callers keep passing the ARCHIVE's number and only one
/// place knows the machine's rule.
pub fn declared_cell(
    profile: InterpreterProfile,
    faces: &FaceSet,
    art_scale: (u32, u32),
) -> zvm::screen::V6Cell {
    let cell = profile.v6_font_cell();
    // The TEXT scale, not the art scale (SQ-1039), and the BODY FACE'S text scale
    // rather than the machine's (SQ-1053) — which is why this takes the whole
    // `FaceSet` and not a bare face. The set records where each face came from, and
    // the space follows the provenance: a face off the Amiga's own release medium
    // scales with the artwork, one off a Kickstart ROM does not.
    let text = face_space(profile, faces.body_origin()).text_scale(art_scale);
    match faces.body().and_then(|f| fit(f, profile, text).map(|k| (f, k))) {
        Some((f, FaceFit::Metric)) => {
            zvm::screen::V6Cell::new(cell.w(), u16::from(f.height).saturating_mul(text.1 as u16))
        }
        _ => cell,
    }
}

/// The cell, the face it is drawn with, and the pen that advances across it.
///
/// # Why these are one value
///
/// CLAUDE.md's refactoring policy: facts that must be considered TOGETHER travel
/// together, and the tell is a parameter list where two arguments always come from
/// the same place. `cell: V6Cell, face: Option<&BitmapFont>` were adjacent
/// parameters on five render functions and were supplied from the same two
/// [`crate::state::AppState`] fields at every one of the six call sites. Adding the
/// scale — which a [`FaceFit::Metric`] face needs and a `Cell` one does not — would
/// have edited all six again, which is when the omissions get made (SQ-0901,
/// SQ-1020, SQ-1021, SQ-1022, all the same shape).
///
/// # The pen
///
/// [`Self::advance`] is the whole difference between a machine that drew Arthur's
/// prose and lanthorn before SQ-1009. With no face, or a `Cell` face, it answers
/// the cell width for every character and every existing path behaves byte for
/// byte as it did. With a `Metric` face it answers that glyph's own advance times
/// the art scale, which is what `machine-screenshots/amiga-arthur-text.png`
/// measures to the pixel on three separate runs.
#[derive(Debug, Clone, PartialEq)]
pub struct TextFace {
    /// The body face and the machine's FIXED-PITCH alternate, as the cascade
    /// resolved them (SQ-1036/SQ-1037). Kept whole rather than split into two
    /// fields so a style reload can rebuild this value from it without losing
    /// which disk answered — see [`Self::faces`].
    faces: FaceSet,
    fit: Option<FaceFit>,
    scale: (u32, u32),
    /// The declared cell and the pen as [`zvm`] holds them — see
    /// [`zvm::screen::V6Metric`].
    ///
    /// **The engine is handed THIS value, not a copy of the rule that built it**
    /// (SQ-1009). Both halves of the split measure the same text: the renderer
    /// asks [`Self::advance_styled`] where the next glyph goes, and zvm asks the
    /// same table where the cursor lands, where a line wraps and how wide a run
    /// came out for header `$30`. Two implementations of one advance would drift
    /// — SQ-1026 and SQ-1035 are a matched pair of exactly that — so there is
    /// one, and everything below delegates to it.
    metric: zvm::screen::V6Metric,
    /// Whether this machine rules under an emphasised run instead of sloping it
    /// (SQ-1028). A machine fact, carried here because `TextFace` is what already
    /// reaches every glyph the raster path draws.
    underline_emphasis: bool,
    /// A digest of everything above that moves a WRAP BOUNDARY: the declared cell
    /// and the advance of every byte the pen can measure (SQ-1034).
    ///
    /// Computed once here rather than compared per frame because the honest
    /// comparison — `PartialEq` on the whole face — walks a `BitmapFont`'s glyph
    /// bitmaps, and the wrap cache has to ask "has the face moved?" on every
    /// frame of both render paths. It is derived from the same `metric` the
    /// renderer and `zvm` both measure with, so a face that wraps differently
    /// cannot fingerprint the same.
    wrap_fp: u64,
}

/// Digest the wrap-relevant half of a resolved [`zvm::screen::V6Metric`]: the
/// cell and the advance of every byte. See [`TextFace::wrap_fingerprint`].
fn wrap_fingerprint_of(metric: &zvm::screen::V6Metric) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let cell = metric.cell();
    cell.w().hash(&mut h);
    cell.h().hash(&mut h);
    // Every §8.7.1 face the pen can be asked for, not just roman: the raster wrap
    // measures the run it is about to break, and an emphasised run is measured
    // with the emphasised advances. **Fixed pitch (bit 3) is in the list** because
    // on a machine with an alternate it is a different pen entirely, and a wrap
    // cache that could not see it would keep a Geneva line for a Monaco run
    // (SQ-1036).
    for style in [0u8, 2, 4, 6, 8, 10, 12, 14] {
        for b in 0u16..=255 {
            metric.advance(b as u8 as char, style).hash(&mut h);
        }
    }
    h.finish()
}

impl TextFace {
    /// Pair a resolved face with the machine that will draw it.
    ///
    /// The cell is [`declared_cell`]'s and never the caller's, so the three places
    /// that settle it — launch, style reload, `@restart` — cannot disagree.
    pub fn new(
        profile: InterpreterProfile,
        faces: FaceSet,
        art_scale: Option<(u32, u32)>,
    ) -> TextFace {
        // The TEXT scale (SQ-1039), and the BODY FACE'S rather than the machine's
        // (SQ-1053). Stored rather than the art scale because all three consumers
        // of `scale` are text: the declared cell, the advance table below, and
        // `render::bitfont`'s per-glyph blit. The artwork's own density travels
        // separately, in `AppState::v6_art_scale`.
        let art = art_scale.unwrap_or((1, 1));
        let scale = face_space(profile, faces.body_origin()).text_scale(art);
        let fit = faces.body().and_then(|f| fit(f, profile, scale));
        let cell = declared_cell(profile, &faces, art);
        let metric = match (faces.body(), fit) {
            (Some(f), Some(FaceFit::Metric)) => {
                // Every byte carries a usable number, so `V6Metric` never has to
                // guess: a glyph the face does not cover falls back to the cell,
                // which is what the renderer draws it at.
                let mut advances = Box::new([cell.w(); 256]);
                for (b, a) in advances.iter_mut().enumerate() {
                    if let Some(g) = f.glyph(b as u8) {
                        *a = (u32::from(g.width) * scale.0).min(u32::from(u16::MAX)) as u16;
                    }
                }
                let bold = (u32::from(f.bold_smear) * scale.0).min(u32::from(u16::MAX)) as u16;
                let m = zvm::screen::V6Metric::proportional(cell, advances, bold);
                // The alternate IS the declared cell — that is what `FaceFit::Cell`
                // means and why `with_fixed_alternate` takes no width — so a
                // fixed-pitch run advances by it and the machine's status bar lines
                // its columns up (SQ-1036). Without an alternate the bit stays the
                // no-op it has always been: there would be no face to draw the run
                // in, and a pen that moved anyway would only be wrong twice.
                if faces.fixed().is_some() { m.with_fixed_alternate() } else { m }
            }
            _ => zvm::screen::V6Metric::fixed(cell),
        };
        let wrap_fp = wrap_fingerprint_of(&metric);
        TextFace {
            faces,
            fit,
            scale,
            metric,
            underline_emphasis: profile.underlines_emphasis(),
            wrap_fp,
        }
    }

    /// A cell with no release face behind it — a bare story, or a host default.
    pub fn cell_only(cell: zvm::screen::V6Cell) -> TextFace {
        let metric = zvm::screen::V6Metric::fixed(cell);
        let wrap_fp = wrap_fingerprint_of(&metric);
        TextFace {
            faces: FaceSet::none(),
            fit: None,
            scale: (1, 1),
            metric,
            underline_emphasis: false,
            wrap_fp,
        }
    }

    /// A digest of everything about this face that can move a wrap boundary — the
    /// declared cell and every advance the pen answers. The transcript wrap cache
    /// keys on this rather than on the face itself (SQ-1034); see the field.
    pub fn wrap_fingerprint(&self) -> u64 {
        self.wrap_fp
    }

    /// What the STORY was told: [`zvm::screen::V6Cell`], declared metrics.
    pub fn cell(&self) -> zvm::screen::V6Cell {
        self.metric.cell()
    }

    /// The cell and the pen as the ENGINE takes them — `Machine::set_v6_text`.
    pub fn metric(&self) -> &zvm::screen::V6Metric {
        &self.metric
    }

    /// The face the renderer may draw BODY text with, if a usable one was admitted.
    pub fn face(&self) -> Option<&BitmapFont> {
        self.faces.body()
    }

    /// The faces this was paired from, so a style reload can rebuild the pairing
    /// against a changed profile without going back to the medium — which it
    /// cannot do, having no medium in scope (see `reload.rs`).
    pub fn faces(&self) -> &FaceSet {
        &self.faces
    }

    /// The face a run carrying §8.7.1 `style` is drawn with (SQ-1036).
    ///
    /// The body face, except that a **fixed-pitch** run takes the machine's
    /// alternate where it has one. That bit reaches here two ways and means one
    /// thing — `@set_text_style 8`, or `@set_font 4`, which `zvm` folds into it —
    /// so this is the single question the renderer asks, and it is asked HERE
    /// rather than in `render::bitfont` because the pen
    /// ([`zvm::screen::V6Metric::advance`]) has to answer it the same way. Two
    /// implementations of one rule are SQ-1026 and SQ-1035, a matched pair.
    ///
    /// With no alternate this is the body face for everything, which is every
    /// configuration that shipped before this.
    pub fn face_for(&self, style: u8) -> Option<&BitmapFont> {
        if style & zvm::screen::STYLE_FIXED_PITCH != 0 {
            if let Some(alt) = self.faces.fixed() {
                return Some(alt);
            }
        }
        self.faces.body()
    }

    /// Whether a run in `style` is drawn with a PROPORTIONAL pen.
    ///
    /// [`Self::proportional`] is about the face; this is about one run, and they
    /// differ on exactly the case SQ-1036 introduced — a fixed-pitch run on a
    /// machine that has an alternate to draw it with is stamped into the declared
    /// cell, not stepped by Geneva's advances. The renderer asks this instead of
    /// testing the style bit itself, so the rule stays in one file.
    pub fn draws_proportionally(&self, style: u8) -> bool {
        self.proportional()
            && !(style & zvm::screen::STYLE_FIXED_PITCH != 0 && self.faces.fixed().is_some())
    }

    /// Whether the face a `style` run draws with has to go through the SCALED,
    /// per-glyph blit rather than being stamped 1:1 into the declared cell.
    ///
    /// Two faces need it and for different reasons (SQ-1053). A **proportional**
    /// face is drawn at its own size because it has no single advance to stretch
    /// to the cell — that is [`Self::draws_proportionally`]. A **fixed** face at a
    /// scale other than (1, 1) is the cell, but only after its pixels are
    /// replicated: the Amiga's topaz 8 is an 8x8 face on an 8x16 cell, every face
    /// row drawn twice, which is what `machine-screenshots/amiga-shogun-game.png`
    /// shows over `Erasmus`.
    ///
    /// Asked here rather than tested in `render::bitfont` so the rule has one home
    /// — the same reason [`Self::draws_proportionally`] is. With no face at all
    /// this is false whatever the scale says, because there is nothing to blit.
    pub fn draws_scaled(&self, style: u8) -> bool {
        self.face_for(style).is_some() && (self.draws_proportionally(style) || self.scale != (1, 1))
    }

    /// How that face may be drawn — see [`FaceFit`].
    pub fn fit(&self) -> Option<FaceFit> {
        self.fit
    }

    /// Native pixels per face pixel, on each axis.
    pub fn scale(&self) -> (u32, u32) {
        self.scale
    }

    /// Whether an emphasised run is RULED rather than sloped — see
    /// [`crate::interpreter::InterpreterProfile::underlines_emphasis`].
    pub fn underlines_emphasis(&self) -> bool {
        self.underline_emphasis
    }

    /// Whether the pen advances per glyph rather than by a fixed cell.
    pub fn proportional(&self) -> bool {
        self.fit == Some(FaceFit::Metric)
    }

    /// Native pixels the pen moves for one ROMAN character — [`Self::advance_styled`]
    /// with no style byte.
    pub fn advance(&self, ch: char) -> u32 {
        self.advance_styled(ch, 0)
    }

    /// Native pixels the pen moves for one character drawn in ZMSD §8.7.1 `style`.
    ///
    /// A character the face does not cover falls back to the cell width, which is
    /// what every non-`Metric` configuration answers for everything.
    ///
    /// # Bold is WIDER, and that is not decoration
    ///
    /// The Amiga emboldens by smearing a glyph `tf_BoldSmear` pixels to the right
    /// and advancing the pen by the same amount, so the extra column has somewhere
    /// to live. Our synthesised bold smears without widening — at the old fixed
    /// 8-wide cell there was slack to absorb that, and at a real 3-to-8 px
    /// proportional advance there is none, so every bold glyph ate its own
    /// inter-character gap and bold words ran together. Arthur's `char.data`
    /// states a smear of **1** (SQ-1009).
    pub fn advance_styled(&self, ch: char, style: u8) -> u32 {
        u32::from(self.metric.advance(ch, style))
    }

    /// How far a run in `style` smears, in FACE pixels — 0 unless it is bold.
    pub fn bold_smear(&self, style: u8) -> u8 {
        match self.face_for(style) {
            Some(f) if style & STYLE_BOLD != 0 => f.bold_smear,
            _ => 0,
        }
    }

    /// Native pixels a whole ROMAN run occupies — the pen's total, bearings and all.
    ///
    /// This is the width that WRAPS: `machine-screenshots/amiga-arthur-text.png`
    /// ends every full prose line within one word's width of the same pixel margin
    /// while carrying different character counts, which no column count reproduces.
    pub fn run_px(&self, s: &str) -> u32 {
        self.run_px_styled(s, 0)
    }

    /// [`Self::run_px`] for a run carrying a §8.7.1 style byte.
    pub fn run_px_styled(&self, s: &str, style: u8) -> u32 {
        self.metric.run_px(s, style)
    }

    /// Native pixels from one text baseline to the next — the cell's height.
    pub fn line_px(&self) -> u32 {
        u32::from(self.cell().h())
    }
}

/// One typeface a story's own medium carries, for the browser's info panel
/// (SQ-1018).
///
/// Display-only, exactly like [`crate::picker::StoryAux::art_candidates`] and
/// `disk_sounds`: it ends at a person's eyes and nothing downstream consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFace {
    /// How the medium names it — `FONT 524` on a Macintosh, the filename on an
    /// Amiga volume.
    pub name: String,
    /// The cell it is drawn for.
    pub width: u8,
    pub height: u8,
    /// Whether its advance actually varies — see [`BitmapFont::proportional`],
    /// and note it counts the accented range no game prints.
    pub proportional: bool,
    /// Whether [`resolve`] would hand THIS one to the renderer.
    pub used: bool,
}

/// Every face on the story's own medium, paired the way the renderer pairs them.
///
/// # Why this reports rather than re-deciding
///
/// `used` is settled by asking [`resolve`] and comparing the faces it returns —
/// BOTH of them, since a release's `FONT` 524 is genuinely in use as the machine's
/// fixed-pitch alternate even when a System disk supplies the body face (SQ-1036)
/// — not by re-deriving which one wins. That costs a second mount and is worth it:
/// SQ-1011 shipped INERT TWICE because a fitness rule existed in two places and
/// correcting one left the other, and both false branches fell back silently. A
/// panel that decided for itself would be a third copy of the same question, and
/// the one a person would trust when it disagreed with the screen.
///
/// Had this surface existed, SQ-1018 would have been visible on sight rather
/// than reported as crowded text: the Masterpieces CD would have shown the face
/// present and unused.
pub fn detected(request: &FaceRequest<'_>) -> Vec<DiskFace> {
    let (story_path, entry) = (request.story_path, request.entry);
    let chosen = resolve(request);
    let mark = |name: String, f: &BitmapFont| DiskFace {
        name,
        width: f.width,
        height: f.height,
        proportional: f.proportional,
        used: chosen.draws(f),
    };

    // A Macintosh names its faces, so report the ids: an id is family × 128 +
    // point size, which is what tells a reader that a release ships a body face
    // AND an alternate rather than two of the same thing.
    if let Some(hfs) = blorb::image::open_disk(story_path)
        .and_then(|f| blorb::hfs::Hfs::mount_file(&f).ok())
    {
        let opened = entry.map(str::to_string).or_else(|| hfs.story().map(|(p, _)| p));
        if let Some(p) = opened {
            let faces: Vec<DiskFace> = blorb::mac_font::faces_beside(&hfs, &p)
                .iter()
                .map(|(id, f)| mark(format!("FONT {id}"), f))
                .collect();
            if !faces.is_empty() {
                return faces;
            }
        }
    }

    // Every other medium: an AmigaDOS disk font is a file, so it is named by one.
    if on_a_cd(story_path) {
        return Vec::new();
    }
    let files: Vec<(String, Vec<u8>)> = crate::assets::files(story_path)
        .into_iter()
        .filter(|f| f.is_on_medium())
        .filter_map(|f| {
            let name = f.name.clone();
            f.into_bytes().map(|b| (name, b))
        })
        .collect();
    blorb::amiga_font::faces_in_volume(files.iter().map(|(p, b)| (p.as_str(), b.as_slice())))
        .iter()
        .map(|(name, f)| mark(name.clone(), f))
        .collect()
}
