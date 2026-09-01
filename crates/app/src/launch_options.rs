//! Launch options: the boot-time choices a story can only be *started* with,
//! and the three doors that reach them (SQ-0789 / SQ-0791).
//!
//! # What belongs here, and the rule that decides it
//!
//! **An option is admitted only if it cannot be changed after boot.** That is
//! the entire argument for a launch dialog: a boot-time choice has nowhere else
//! to live, while anything the running app can already change belongs in the
//! settings screen, and duplicating it here would create two editors for one
//! value.
//!
//! Admitted:
//!
//! - **The picture archive.** Its pictures are resolved as the story starts, and
//!   swapping it under a running game would re-resolve resources beneath a VM
//!   that never learns it happened.
//! - **The interpreter number.** Header byte `0x1E` is read by the *story*
//!   (`crates/zvm/src/cpu/exec.rs` branches on `read_byte(0x1E) == 6`), so a game
//!   that has booted has already made decisions from it.
//!
//! Rejected, and why — because "any other options" is how a dialog becomes a
//! second settings screen:
//!
//! - **v6 render mode.** Checked rather than assumed: `/set-v6-render` switches
//!   hybrid/raster **live** (`SlashOutcome::SetV6Render`, applied to
//!   `state.config.v6_render` mid-session) and the settings screen persists it.
//!   It fails the admission rule outright.
//! - Colours, styles, map behaviour — all live-editable, all already have a home.
//!
//! # The one thing this module enumerates, and the line it must not cross
//!
//! [`discover_art_candidates`] lists the native archives a story can use: those
//! beside it that carry *this story's name*, and those on the release it came out
//! of. SQ-0734 rejected exactly that enumeration as an
//! input to *resolution*, and that rejection stands: the format carries no
//! release number and no serial, every Infocom Amiga release names its archive
//! `Pic.data`, and a wrong pairing is invisible — Arthur's plates drawn into Zork
//! Zero look like art, not like an error.
//!
//! **Discovery for DISPLAY is safe; discovery for PAIRING is not.** The list
//! below is safe for precisely the reason auto-pairing is not: it ends at a
//! human, who knows which game they own and can supply the assertion the file
//! format cannot make. Nothing in this module may ever be wired into
//! [`crate::graphics::PictureOverride::resolve`] or into any other automatic
//! choice. If you are here to "close the gap" by picking the best candidate
//! programmatically, that is the failure the tier policy exists to prevent.
//!
//! The name filter does not weaken that line, because it is not a *pairing* — it
//! is which rows a person is shown. Every archive it declines to list stays
//! reachable by naming it outright, through `--pictures` or the game's own
//! `pictures` key, which is where an oddly-named file (`FMVPOKER.EG1` is Zork
//! Zero's EGA art under a fan game's name) has always belonged. The dialog says
//! so on screen; see `docs/internals/v6-graphics.md`.

use std::path::{Path, PathBuf};

use blorb::infocom_pics::{Flavour, InfocomPics};

// ── LaunchOverrides ───────────────────────────────────────────────────────────

/// The boot-time overrides one launch carries, ahead of anything on disk.
///
/// Empty is the ordinary case: every field `None` means "inherit", exactly as an
/// absent key in the per-game sidecar does. A field is `Some` only when a door
/// into this mechanism was actually used — `--pictures` on the command line, or
/// a value the launch-options dialog reports as *changed* from what this story
/// already inherits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOverrides {
    /// A native picture archive named for this launch. Outranks the per-game
    /// sidecar's `pictures` key — the more specific and more recent instruction
    /// wins, and the help text says so.
    pub pictures: Option<String>,
    /// An interpreter number for this launch (ZMSD §11.1.3), outranking both the
    /// per-game sidecar and the global config.
    pub interpreter_number: Option<u8>,
}

impl LaunchOverrides {
    /// Nothing overridden — the launch behaves exactly as it did before any of
    /// this existed.
    pub fn is_empty(&self) -> bool {
        self.pictures.is_none() && self.interpreter_number.is_none()
    }
}

// ── ArtCandidate ──────────────────────────────────────────────────────────────

/// One native picture archive a story can use, described well enough to choose
/// by: what wrote it, how many pictures it holds, and which part of a multi-part
/// set it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtCandidate {
    /// The file to open: the archive itself when it is loose, and the disk image
    /// when the archive is **on** a volume — the archive inside has no path of
    /// its own on this machine. See [`on_medium`](ArtCandidate::on_medium).
    pub path: PathBuf,
    /// Bare filename — what goes into `pictures = "…"`. The key resolves a bare
    /// name against the story's own directory first and, failing that, against
    /// the volume the story was mounted out of, so one name reaches both
    /// (`crate::graphics::read_off_the_medium`).
    pub filename: String,
    /// Which platform's codec read it.
    pub flavour: Flavour,
    /// The rendition label a human recognises: `Amiga`, `MCGA`, `EGA`, `CGA`.
    pub rendition: &'static str,
    /// Directory entries — pictures plus size-only placeholders — across the
    /// **whole set**, continuation parts included (SQ-0798).
    pub pictures: usize,
    /// Part number of the file named; multi-part sets number their files 1, 2, …
    pub part: u8,
    /// How many files this candidate is: 2 for `arthur.eg1`, which carries
    /// `arthur.eg2` with it, and 1 for everything else (SQ-0798).
    pub parts: u8,
    /// The width of the picture space its coordinates use: 320, 480 or 640.
    pub space_width: u16,
    /// Is this archive INSIDE the disk image at [`path`](ArtCandidate::path)
    /// rather than a loose file beside the story (SQ-0843)?
    ///
    /// Shown, because a person looking at `CPic.data` in a folder that plainly
    /// does not contain one deserves to be told where it is. Not otherwise
    /// consumed: picking it writes the same bare `filename`, and the two doors
    /// meet in `PictureOverride::resolve_with_session`.
    pub on_medium: bool,
    /// **Which** volume of the release carries it, when the release is a set
    /// (SQ-0865). `None` for a loose file and for a single-image release.
    ///
    /// SQ-0862 is what made this necessary rather than merely nice: an archive
    /// can now come off a *sibling* volume, so booting the 360K press's disk 2
    /// offers `ZORK0.EG1` — which is physically on disk 3. "on disk" was vague
    /// before that and ambiguous after it. Straight from
    /// [`crate::assets::AssetFile::disk_number`], so the number is the release's
    /// own and never parsed out of a filename here.
    pub disk_number: Option<u64>,
}

impl ArtCandidate {
    /// The machine this rendition asks lanthorn to present itself as.
    pub fn profile(&self) -> crate::interpreter::InterpreterProfile {
        crate::interpreter::InterpreterProfile::for_art_flavour(self.flavour)
    }

    /// An honest one-line caveat, or `None` when the rendition draws correctly.
    ///
    /// **Every rendition draws correctly today, so this is `None` for all of
    /// them** — and that is the second time this function has had to be emptied
    /// rather than edited, which is the note worth leaving for whoever adds the
    /// third caveat. A caveat is a sentence a person reads in a dialog while
    /// deciding; it outlives its defect silently, because nothing fails when a
    /// warning is merely no longer true. Both of the ones that stood here were
    /// removed by the quest that fixed what they described, only after the user
    /// had seen the dialog still saying it.
    ///
    /// What they were:
    ///
    /// * **Geometry** — a 640-wide archive was drawn at the 320-wide scale, so an
    ///   EGA banner, pillar and compass all sat in the wrong place. SQ-0790's
    ///   per-axis art scale draws it `(1, 2)` against MCGA's `(2, 2)` and they now
    ///   land exactly where the MCGA ones do.
    /// * **Dithered colour** — EGA's sixteen colours were fixed in the card, so
    ///   its artists dithered for the ones they lacked, and lanthorn kept all 640
    ///   columns distinct where the card fused them in the eye. SQ-0797 fuses them
    ///   at the archive boundary: Zork Zero's boot frame went from horizontal
    ///   speckle 49.1 to 8.4 against the MCGA rendition's own 4.3, and the arch
    ///   reads as bronze rather than as salmon-and-olive.
    ///
    /// The one thing SQ-0815 measured that a caveat could still describe is
    /// deliberately NOT here. Zork Zero's EGA pillars are error-diffusion
    /// dithered rather than column-dithered, and the `[1, 2, 1] / 4` tent is a
    /// notch at one frequency, not a low-pass — so their shaft keeps some speckle
    /// (flank horizontal speckle 62.9 raw, 12.7 fused, against the MCGA flank's
    /// 12.3 in its own 320-wide space). That is a property of two pictures on one
    /// plate, not of the rendition a person is picking, and the honest place for
    /// it is `docs/internals/v6-graphics.md` — where it is, with the measurements.
    /// Widening the kernel until the pillars fuse mushes the compass rose's
    /// lettering on the same frame, which is why it was not widened.
    pub fn caveat(&self) -> Option<&'static str> {
        None
    }
}

/// Extensions worth opening as a native Infocom archive. Case-insensitive.
///
/// `Pic.data` has no extension at all, so it is matched by *name* below — that
/// being the name every Infocom Amiga release uses, which is exactly why a stem
/// rule could never pair one automatically.
const ART_EXTS: &[&str] = &["pic", "mg1", "mg2", "eg1", "eg2", "cg1", "cg2", "data"];

/// The native picture archives `story_path` can use, in a stable order (by
/// filename), each one actually parsed so the list can state its flavour,
/// picture count and part number rather than guess.
///
/// Two sources, unioned by [`crate::assets::files`] and filtered here: the loose
/// files beside the story that carry **this story's name**, and — when the story
/// came out of a disk image — the archives on every volume of **that release**
/// (SQ-0843, widened by SQ-0862). This function is the one place that answers
/// "what artwork can this story use?", and it is the seam that knows disks exist
/// so that no caller has to.
///
/// The release rather than the platter is what makes the DOS presses of Zork Zero
/// pickable. Its 360K press puts the story alone on disk 2 with CGA on disk 1 and
/// EGA on disk 3, so a person booting the story disk was shown no artwork at all;
/// `crate::assets::volumes` states the rule and, importantly, the case it refuses
/// — a twenty-game compilation does not offer one game's plates to another.
///
/// The medium arm is why the Macintosh disk is pickable at all. `stories/Zork
/// Zero Disk.image` carries a colour `CPic.data` and a monochrome `Pic.data`,
/// neither of which exists on the host filesystem; a `read_dir` could not see
/// either, so the dialog offered no way to choose the two-colour art and
/// `--pictures Pic.data` was the only door. `blorb::medium::MountedDisk::pictures`
/// is not enough on its own here — it answers with THE archive by the format's
/// own tiebreak, which is deliberately the colour one — so this walks
/// `contents()` and identifies every file by parsing it.
///
/// **This is display-only.** See the module header: enumerating candidates is
/// safe because a person picks; nothing may consume this list automatically.
/// The name filter narrows *what a person is shown*, which is why it is allowed
/// to be a guess — the alternative is a story library's whole folder in one
/// dialog, since `stories/` holds Arthur, Journey, Shogun and Zork Zero side by
/// side and most of what sits "beside" any story belongs to another game.
///
/// A file that does not parse is simply absent from the list — it is not a
/// candidate, and a name-shaped file that is not an archive (`saved.data`, say)
/// must not be offered as one. That silence is right *here* and wrong in
/// [`crate::graphics::PictureOverride::resolve`], where a named-but-unusable
/// archive is loud on purpose: this function answers "what could you pick?",
/// that one answers "you asked for this and it did not work". An archive whose
/// name resembles nothing is in the same position: not offered, still reachable
/// by name through `--pictures` or the `pictures` key.
///
/// # A multi-part set is ONE row (SQ-0798)
///
/// Arthur's EGA art is `arthur.eg1` **and** `arthur.eg2`, and naming the first
/// now loads both. Offering the second as a separate choice would be offering
/// half an archive: on its own `arthur.eg2` is 101 of the set's 171 ids, so
/// picking it means silently losing the rest. So a file whose earlier part sits
/// beside it is not listed at all, and the row that IS listed reports the whole
/// set's picture count. The bare continuation stays reachable by name, like every
/// other file this list declines to show, for anyone who genuinely wants only
/// disk two.
/// # Which story, on a disc that holds several (SQ-0876)
///
/// `disk_entry` is the story's own name on the volume. On a medium whose games
/// live in FOLDERS, only the archives in this story's folder are listed — the
/// medium's guarantee is "it shipped in the box", and on a compilation the box
/// holds six games. Offering all 22 of the Masterpieces CD's archives for one
/// game is the same wrong-pairing hazard this module's header warns about, just
/// sourced from one disc instead of one directory.
///
/// The comparison is on the folder the MEDIUM spells, so a story at the volume
/// root sees the root's archives — which is every single-game floppy, and also
/// the multi-disk case where a sibling volume carries the art at ITS root
/// (SQ-0862). `None` lists everything on the medium, exactly as before.
pub fn discover_art_candidates(story_path: &Path, disk_entry: Option<&str>) -> Vec<ArtCandidate> {
    let story_stem = story_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let want_folder = disk_entry.map(folder_of);
    let mut out: Vec<ArtCandidate> = Vec::new();
    let all = crate::assets::files(story_path);
    // The names on the MEDIUM, and the bytes of the ones that could be a
    // continuation — everything needed to resolve a multi-part set without
    // mounting the volume a second time. Only a file whose extension ends in
    // 2..9 can be a continuation, so this holds two files on the largest disc
    // in the corpus rather than the whole platter (SQ-0881).
    let medium_names: Vec<&str> =
        all.iter().filter(|f| f.is_on_medium()).map(|f| f.name.as_str()).collect();
    let medium_parts: Vec<(&str, &[u8])> = all
        .iter()
        .filter(|f| f.is_on_medium() && part_of_name(&f.name).is_some_and(|n| n > 1))
        .filter_map(|f| f.peek_bytes().map(|b| (f.name.as_str(), b)))
        .collect();
    for file in &all {
        // **The name filter is the LOOSE source's, and only its.** A file beside
        // the story proves nothing by sitting there — `stories/` holds Arthur,
        // Journey, Shogun and Zork Zero side by side — so it must carry this
        // story's name to be shown. A file on the medium needs no such test: it
        // shipped in the box the story was mounted out of, which is the one
        // pairing the medium itself asserts (`blorb::medium::DiskArt`). That
        // covers a sibling volume of a single-game release exactly as well —
        // `crate::assets::volumes` offers no other kind.
        //
        // Name first, bytes second on that arm: an archive is megabytes, and a
        // flat library holds a dozen of them. Deciding on the name before
        // reading keeps this cheap enough for the browser's info panel to ask
        // per story.
        if file.is_on_medium() {
            // Same folder as the story, when we know which story it is.
            if want_folder.is_some_and(|want| folder_of(&file.name) != want) {
                continue;
            }
        } else {
            if !looks_like_art_name(&file.name) {
                continue;
            }
            let stem = Path::new(&file.name).file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if !belongs_to_story(story_stem, stem, &file.name) {
                continue;
            }
        }
        let filename = file.name.clone();
        let path = file.path.clone();
        let on_medium = file.is_on_medium();
        let disk_number = file.disk_number;
        let Some(raw) = file.clone().into_bytes() else { continue };
        // **Identified by parsing, for both sources alike** — the same
        // content-first rule `adf.rs` and `hfs.rs` apply file by file inside a
        // volume, and it has to be the same one here or a directory's files
        // would be classified by a different test from a disk's. It matters most
        // on the Macintosh, whose two archives are called `CPic.data` and
        // `Pic.data` and whose names say nothing about which is which.
        let Ok(mut pics) = InfocomPics::parse(raw) else { continue };
        // A container with no pixels anywhere is not artwork, whatever it parsed
        // as. On a volume that is the guard against a `Story.data` or a desktop
        // database that happens to satisfy the header; beside the story it has
        // never excluded anything, and the real-media test says so.
        if !pics.entries().iter().any(|e| e.has_pixels()) {
            continue;
        }
        if on_medium {
            // **The same rule the loose arm has always had, on the medium**
            // (SQ-0881). `ARTHUR.EG1` and `ARTHUR.EG2` are one two-part EGA set
            // that the run merges, so offering them as two rows offers half a
            // set as a choice — picking `.EG2` means silently losing the front
            // half. Off the Masterpieces CD the dialog listed both.
            //
            // A continuation whose earlier part is on the same volume is not a
            // row; the row that IS listed absorbs it and reports the whole
            // set's picture count.
            if crate::graphics::part_name(&filename, pics.part().saturating_sub(1))
                .is_some_and(|prev| medium_names.iter().any(|n| n.eq_ignore_ascii_case(&prev)))
            {
                continue;
            }
            absorb_medium_continuations(&mut pics, &filename, &medium_parts);
        } else {
            // A continuation whose earlier part is here is not a choice — it is
            // the back half of the row above it, and that row already carries it.
            if crate::graphics::part_path(&path, pics.part().saturating_sub(1))
                .is_some_and(|prev| prev.is_file())
            {
                continue;
            }
            // Whatever this file continues into is part of what picking it gets
            // you, so the count has to say so. A refused continuation is not
            // reported here (this list is display-only and silent by design —
            // see above); the loud version is `PictureOverride::warning`, on the
            // archive actually chosen.
            crate::graphics::absorb_continuations(&mut pics, &path);
        }
        // Deliberately NOT on the medium arm: `part_path` names a sibling of
        // `path`, which for a volume's file is the disk image, so it would look
        // for `<image dir>/FOO.EG2` — a file on the host, next to the wrong
        // thing. No disk lanthorn mounts ships a multi-part native archive (both
        // that do are DOS releases, whose FAT12 mount is queued as SQ-0833), so
        // the honest move is to leave the walk off rather than aim it wrongly;
        // the part number below still reports what the archive says it is.
        let space_width = pics.picture_space_width();
        let mono = pics.is_monochrome();
        out.push(ArtCandidate {
            rendition: rendition_label(pics.flavour(), space_width, &filename, mono),
            flavour: pics.flavour(),
            pictures: pics.entries().len(),
            part: pics.part(),
            parts: pics.parts(),
            space_width,
            on_medium,
            disk_number,
            filename,
            path,
        });
    }
    out.sort_by_key(|c| c.filename.to_lowercase());
    out
}

/// The shortest normalised stem this rule will match on. Below it a substring
/// test stops meaning anything — three letters land inside unrelated words, and
/// a false positive here shows a person another game's artwork as if it were
/// theirs, which is the one outcome the whole tier policy is arranged around.
const MIN_STEM: usize = 4;

/// Does the archive `art_filename` (stem `art_stem`) carry the same game's name
/// as the story `story_stem`?
///
/// Both names are reduced by [`crate::hints::normalize_ident`] — lowercased,
/// ASCII alphanumerics only — which is the primitive the hint index already
/// matches game names with, and it is what makes a spaced disk-image name and a
/// DOS 8.3 archive comparable at all: `Beyond Zork - The Coconut of Quendor`
/// becomes `beyondzorkthecoconutofquendor`, and `beyondzo.mg1` becomes
/// `beyondzo`.
///
/// The test then runs in **both directions**, because either name can be the
/// longer one. Measured against the whole of `stories/`:
///
/// | story | archive | direction |
/// | --- | --- | --- |
/// | `zork0-r393-s890714.z6` | `zork0.{cg1,eg1,mg1,pic}` | archive inside story |
/// | `beyondzork-r57-s871221.z5` | `beyondzo.mg1` | archive inside story |
/// | `James Clavell's Shogun.adf` | `shogun.*` | archive inside story |
/// | `fmvpoker.z6` | `FMVPOKER.EG1` | equal, case aside |
///
/// A pure prefix rule would miss the disk images, whose titles begin with an
/// author or an article; a one-directional rule would miss whichever of the two
/// happened to be shorter. Neither costs anything to allow, because the result
/// is a list a person reads.
fn belongs_to_story(story_stem: &str, art_stem: &str, art_filename: &str) -> bool {
    // `Pic.data` and `CPIC.DATA` are the names EVERY Infocom Amiga release uses,
    // which is the fact SQ-0734 rejected auto-pairing over. A name that carries
    // no game identity cannot be filtered BY identity, and there can only be one
    // of each in a folder, so the honest answer is to offer it and let the person
    // who extracted it say whether it is theirs.
    if is_generic_archive_name(art_filename) {
        return true;
    }
    let story = crate::hints::normalize_ident(story_stem);
    let art = crate::hints::normalize_ident(art_stem);
    if story.len() < MIN_STEM || art.len() < MIN_STEM {
        return false;
    }
    story.contains(&art) || art.contains(&story)
}

/// The two extension-less names every Infocom Amiga release ships its archive
/// under, which is exactly why they say nothing about *which* release.
fn is_generic_archive_name(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    lower == "pic.data" || lower == "cpic.data"
}

/// Is this filename worth opening as a picture archive?
fn looks_like_art_name(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    if is_generic_archive_name(&lower) {
        return true;
    }
    match lower.rsplit_once('.') {
        // `.data` alone is far too broad to admit on its own; only the two
        // Infocom archive names above use it.
        Some((_, "data")) => false,
        Some((_, ext)) => ART_EXTS.contains(&ext),
        None => false,
    }
}

/// The trailing "…and it is more than one file" phrase both surfaces append to a
/// candidate row, or `""` for the ordinary single-file archive (SQ-0798).
///
/// Two different facts, and only one of them is ever true at a time. `2 disks`
/// is the multi-part set collapsed into one row — the count beside it is already
/// the whole set's, and this says why it is larger than the file looks. `part 2`
/// is the opposite case: a lone continuation whose part 1 is not on disk, listed
/// because it is all there is, and flagged because it is not a whole archive.
pub fn parts_note(c: &ArtCandidate) -> String {
    if c.parts > 1 {
        format!("  {} disks", c.parts)
    } else if c.part != 1 {
        format!("  part {}", c.part)
    } else {
        String::new()
    }
}

/// **Which disk** an archive lives on, or `""` for an ordinary file beside the
/// story (SQ-0865).
///
/// The phrase both surfaces append, so the dialog and the browser's info panel
/// cannot drift into two answers about one archive. Each adds its own separator;
/// this is the words alone.
///
/// Three states, and the middle one is the whole reason this replaced `"on
/// disk"`. A multi-disk release says **which** disk, because since SQ-0862 an
/// archive can come off a volume the player never booted — the 360K press offers
/// `ZORK0.EG1` from disk 3 while the story runs off disk 2, and "on disk" read
/// as if it meant the disk in the drive. A single-image release has no disk
/// number to give and says `from game disk`, which is the user's own wording and
/// says the thing the marker was always for: this file is inside the image, not
/// beside it. A loose file says nothing, because a file in the folder the story
/// is in needs no explanation.
pub fn medium_note(c: &ArtCandidate) -> String {
    where_note(c.on_medium, c.disk_number)
}

/// [`medium_note`] over the two facts it reads, so the resolved-default row can
/// share the wording without inventing an [`ArtCandidate`] to carry it.
fn where_note(on_medium: bool, disk_number: Option<u64>) -> String {
    match (on_medium, disk_number) {
        (false, _) => String::new(),
        (true, Some(n)) => format!("from disk {n}"),
        (true, None) => "from game disk".to_string(),
    }
}

// ── What "use this story's own art" actually resolves to ─────────────────────

/// The archive a launch will open when nothing is overridden — the *default*
/// choice, described well enough to name in the dialog before the story boots
/// (SQ-0865).
///
/// # Why this exists at all
///
/// The default row used to read *"Use this story's own art (Blorb / disk
/// image)"*: prose among columns, and the one row that said nothing about what
/// accepting it would do — which is precisely the thing you want to know before
/// accepting it. Naming the archive is only safe if the name is the one boot
/// will really open, so this is **derived from the resolution boot performs**
/// ([`crate::graphics::release_art`], then the resource Blorb) and never from a
/// second reading of the same evidence. A row that claimed one archive while the
/// boot took another would be worse than the prose it replaced.
///
/// Note the direction of the dependency, which is the tier policy's line held
/// exactly where SQ-0734 put it: this *reports* an automatic choice that was
/// already made, and nothing here feeds back into making one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultArt {
    /// The archive's name, as its volume or its directory spells it.
    pub filename: String,
    /// The rendition label, from the same function the candidate rows use — or
    /// `"Blorb"` when the story resolves its own resource file.
    pub rendition: &'static str,
    /// How many directory entries it holds.
    pub pictures: usize,
    /// Is it inside a disk image? See [`ArtCandidate::on_medium`].
    pub on_medium: bool,
    /// Which volume of the release, when the release is a set.
    pub disk_number: Option<u64>,
}

impl DefaultArt {
    /// Where this archive lives, in the dialog's words.
    pub fn medium_note(&self) -> String {
        where_note(self.on_medium, self.disk_number)
    }
}

/// What `story_path` will draw with if the launch overrides nothing, or `None`
/// when it will draw with nothing at all (SQ-0865).
///
/// The two tiers [`crate::graphics::PictSource::resolve`] walks, asked in its
/// order and through its own functions: the release's native archive first, then
/// the story's resource Blorb.
///
/// # Cost, and why the dialog resolves this once
///
/// It mounts the release's volumes — the same ~1.5 ms warm that
/// [`discover_art_candidates`] pays, and for the same reason. That is far too
/// much to repeat per frame, so [`LaunchOptionsState::new`] calls this **once**
/// when the dialog opens and the renderer only formats the answer. The dialog is
/// modal and nothing behind it can change the artwork on disk while it is up, so
/// a value settled at open time cannot go stale before it is closed.
pub fn resolved_default_art(story_path: &Path, disk_entry: Option<&str>) -> Option<DefaultArt> {
    if let Some(art) = crate::graphics::release_art(story_path, disk_entry) {
        let space_width = art.pictures.picture_space_width();
        let mono = art.pictures.is_monochrome();
        return Some(DefaultArt {
            rendition: rendition_label(art.pictures.flavour(), space_width, &art.name, mono),
            pictures: art.pictures.entries().len(),
            on_medium: true,
            disk_number: art.disk_number,
            filename: art.name,
        });
    }
    // Tier 1. A Blorb is not a rendition of anything — it is the modern
    // container, with no video card behind it — so the column says what it is
    // rather than inventing a machine for it.
    //
    // Through `graphics::resource_blorb`, not `blorb::resolve_resource_blorb`:
    // that function is where a Blorb naming a DIFFERENT build is refused
    // (SQ-0866), and a row that offered `Arthur.blb` for the Apple IIgs disk
    // while the boot drew nothing would be exactly the untrustworthy row this
    // function exists to prevent. The default row reads "no artwork found"
    // instead, which is what the boot will do.
    let (blorb, path) = crate::graphics::resource_blorb(story_path).found?;
    let pictures = blorb.resources().iter().filter(|r| &r.usage == b"Pict").count();
    if pictures == 0 {
        return None; // a sound-only sidecar draws nothing
    }
    Some(DefaultArt {
        filename: path.file_name()?.to_str()?.to_string(),
        rendition: "Blorb",
        pictures,
        on_medium: false,
        disk_number: None,
    })
}

/// The rendition a human recognises.
///
/// The codec, the picture-space width and the two-colour flag are read from the
/// file, so those three are facts. Splitting the 640-wide PC case into EGA and
/// CGA is *not* — the two write the same container, and only Infocom's DOS 8.3
/// naming tells them apart. That is a display label, never an input to anything,
/// so leaning on the extension here costs nothing; a 640-wide PC archive under
/// some other name says "EGA/CGA" and stays honest.
///
/// # Why the two-colour Amiga/Mac archive says "Mac B&W"
///
/// The Macintosh release disk carries **two** archives and both are
/// [`Flavour::AmigaMac`], so without a second axis the dialog would offer
/// `CPic.data` and `Pic.data` as two identical-looking rows — the state SQ-0843
/// was reported from, one step on. `monochrome` is the honest discriminator,
/// and it is not a guess about the file: it is
/// [`blorb::infocom_pics::InfocomPics::is_monochrome`], off the archive's own
/// `EF_MONO` flags, the same test that decides the two-colour hardware palette.
///
/// Naming the MACHINE on it is the part that needs an argument, since the codec
/// cannot tell an Amiga from a Mac in general (SQ-0838). Two independent things
/// say Macintosh here and nothing says Amiga: Spatterlight's bocfel reclassifies
/// a monochrome `Pic.data` as the B&W Mac on exactly this flag, and the archive
/// is drawn in the 480×300 picture space that is Infocom's own `GFXMAC_X` /
/// `GFXMAC_Y` — "1.5 x Amiga sizes", a screen the Amiga never had. A *colour*
/// AmigaMac archive still says plain "Amiga", because there the ambiguity is
/// real and unresolved.
fn rendition_label(
    flavour: Flavour,
    space_width: u16,
    filename: &str,
    monochrome: bool,
) -> &'static str {
    match flavour {
        Flavour::AmigaMac if monochrome => "Mac B&W",
        Flavour::AmigaMac => "Amiga",
        // Double hi-res, 140×192, sixteen hardware colours — one rendition, one
        // machine, and no filename or flag needed to tell them apart (SQ-0863).
        Flavour::Apple => "Apple II",
        Flavour::Pc if space_width == 320 => "MCGA",
        Flavour::Pc => {
            let lower = filename.to_lowercase();
            if lower.ends_with(".cg1") || lower.ends_with(".cg2") {
                "CGA"
            } else if lower.ends_with(".eg1") || lower.ends_with(".eg2") {
                "EGA"
            } else {
                "EGA/CGA"
            }
        }
    }
}

// ── The derived interpreter number, and where it came from ────────────────────

/// Why the launch is advertising the interpreter number it is.
///
/// The dialog shows this beside the number because picking prettier art can
/// *move* it: SQ-0734 rules that a named archive's flavour selects the machine
/// unless a number is set explicitly. Silently changing the emulated machine
/// because someone chose an `.eg1` is the surprise this dialog exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpreterSource {
    /// A number was set outright — in this dialog, on the command line, or in a
    /// config file. Beats everything.
    Explicit,
    /// Inferred from the flavour of the chosen picture archive.
    Artwork,
    /// Inferred from the medium: a story mounted out of an Amiga floppy.
    DiskImage,
    /// Nobody said anything: Frotz's rule, 6 for Version 6 and 1 otherwise.
    Default,
}

impl InterpreterSource {
    /// The provenance phrase shown after the number.
    pub fn label(self) -> &'static str {
        match self {
            InterpreterSource::Explicit => "set here",
            InterpreterSource::Artwork => "from the artwork",
            InterpreterSource::DiskImage => "from the disk image",
            InterpreterSource::Default => "default for this story",
        }
    }
}

/// The ZMSD §11.1.3 machine name for an interpreter number, or `"?"`.
///
/// Verified against the table quoted on `Cli::interpreter_number` (which is
/// itself the ZMSD's): 1 DECSystem-20, 2 Apple IIe, 3 Macintosh, 4 Amiga,
/// 5 Atari ST, 6 IBM PC, 7 Commodore 128, 8 Commodore 64, 9 Apple IIc,
/// 10 Apple IIgs, 11 Tandy Color.
pub fn interpreter_name(n: u8) -> &'static str {
    match n {
        1 => "DECSystem-20",
        2 => "Apple IIe",
        3 => "Macintosh",
        4 => "Amiga",
        5 => "Atari ST",
        6 => "IBM PC",
        7 => "Commodore 128",
        8 => "Commodore 64",
        9 => "Apple IIc",
        10 => "Apple IIgs",
        11 => "Tandy Color",
        _ => "?",
    }
}

/// The interpreter numbers the dialog offers, in cycling order: `None` (auto)
/// followed by the whole ZMSD table.
pub const INTERPRETER_CHOICES: [Option<u8>; 12] =
    [None, Some(1), Some(2), Some(3), Some(4), Some(5), Some(6), Some(7), Some(8), Some(9), Some(10), Some(11)];

/// Resolve the interpreter number a launch would advertise, and say where it
/// came from — the same precedence [`crate::interpreter::InterpreterProfile::resolve`]
/// applies at boot, reported rather than applied.
///
/// `z_version` is the story's Z-machine version when known, because the default
/// rule depends on it (6 for Version 6, else 1). `None` — a Glulx or Scott story,
/// where header `0x1E` does not exist at all — yields no number to report.
pub fn derived_interpreter(
    explicit: Option<u8>,
    art: Option<&ArtCandidate>,
    disk_image: Option<crate::hints::DiskImage>,
    z_version: Option<u8>,
) -> Option<(u8, InterpreterSource)> {
    if let Some(n) = explicit {
        return Some((n, InterpreterSource::Explicit));
    }
    let version = z_version?;
    if let Some(c) = art {
        // The archive's own answer, refined by the medium where the codec cannot
        // tell an Amiga from a Macintosh — through the very function
        // `InterpreterProfile::resolve` uses at boot, so the number the dialog
        // advertises and the number the story is handed cannot disagree.
        let profile =
            crate::interpreter::InterpreterProfile::for_art_flavour_on(c.flavour, disk_image);
        // A profile that answers `None` is the IBM PC, which defers to zvm's own
        // rule rather than pinning a number — the same deferral, reported. Asked
        // of zvm rather than restated: this dialog open-coded Frotz's rule twice
        // (SQ-0872), which is exactly the second copy that drifts.
        let n = profile
            .interpreter_number()
            .unwrap_or_else(|| zvm::screen::default_interpreter_number(version));
        // …and when the disk was what settled it, say the disk. An art row that
        // claimed "from the artwork" over a number the artwork did not choose is
        // the provenance line telling a small lie.
        let source = if profile == c.profile() {
            InterpreterSource::Artwork
        } else {
            InterpreterSource::DiskImage
        };
        return Some((n, source));
    }
    // The medium's own answer, asked of the one place that knows it
    // (`blorb::medium`, SQ-0839) rather than restated here — a second copy of
    // "an `.adf` means 4" is exactly what drifts. A Macintosh disk answers
    // `None` and falls through to the default rule, which is what
    // `InterpreterProfile::resolve` does at boot: the dialog must report the
    // machine that will actually run, not the one the medium came from (SQ-0837).
    if let Some(n) = disk_image.and_then(|d| d.interpreter_number()) {
        return Some((n, InterpreterSource::DiskImage));
    }
    Some((zvm::screen::default_interpreter_number(version), InterpreterSource::Default))
}

// ── LaunchOptionsState ────────────────────────────────────────────────────────

/// Which row of the dialog the cursor is on. Art rows come first (index 0 is
/// "no override"), then the interpreter row, then the persist checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// `0` = inherit (Blorb / disk image); `1..` indexes `candidates`.
    Art(usize),
    Interpreter,
    Persist,
}

/// Everything the launch-options dialog holds while it is open.
///
/// The two `baseline_*` fields are what this story *already* inherits with no
/// dialog involved. Every decision the dialog makes — what to apply to this
/// launch, and what the checkbox writes — is a comparison against them, which is
/// what keeps the per-game sidecar's "absent key = inherit" semantics intact: a
/// key written with the value it already inherits is **not** the same as a key
/// left absent, and this dialog never writes one.
#[derive(Debug, Clone)]
pub struct LaunchOptionsState {
    pub title: String,
    pub story_path: PathBuf,
    pub candidates: Vec<ArtCandidate>,
    /// What row 0 — "inherit" — actually resolves to, settled once when the
    /// dialog opens (SQ-0865). See [`resolved_default_art`] for why it is
    /// resolved here and not per frame.
    pub default_art: Option<DefaultArt>,
    /// `0` = inherit; `1..` selects `candidates[i - 1]`.
    pub art: usize,
    pub interpreter: Option<u8>,
    pub persist: bool,
    pub cursor: Row,
    /// Button focus ring: `0` = Play, `1` = Cancel.
    pub focus: usize,
    pub baseline_art: usize,
    pub baseline_interpreter: Option<u8>,
    /// The story's Z-machine version, for the default-rule half of the derived
    /// interpreter number. `None` for a non-Z story.
    pub z_version: Option<u8>,
    /// The release disk image the story was mounted out of, if any.
    pub disk_image: Option<crate::hints::DiskImage>,
    /// **Which** story on that image, when it holds several (SQ-0859): the name
    /// the volume stores it under. Carried because the path alone cannot say —
    /// a dialog opened on *Leather Goddesses* and one opened on *Sherlock* hold
    /// the same `INFOCOM6` path, and Play must start the one the player was
    /// looking at, not the largest file on the disk.
    ///
    /// `None` for every loose file and every single-story image, which is the
    /// unchanged path. Set by [`LaunchOptionsState::on_disk_entry`] so the
    /// constructor stays the six arguments it was.
    pub disk_entry: Option<String>,
    /// The sidecar's inherited `pictures` name, kept so
    /// [`LaunchOptionsState::on_disk_entry`] can re-derive the selection after
    /// it narrows the candidate list to one game's folder (SQ-0876).
    pub(crate) inherited_pictures: Option<String>,
}

/// Drop the candidate the "Automatic" row already names, so one archive is not
/// offered twice.
///
/// Row 0 resolves to a specific archive and SAYS which (SQ-0865), so listing
/// that same archive again below it is one choice wearing two rows. It was easy
/// to miss while a compilation offered sixteen; scoping the list to one game's
/// folder left Journey with exactly `CPIC.DATA` and `PIC.DATA` and the duplicate
/// became half the list.
///
/// Dropping it costs nothing that can be reached another way: picking
/// "Automatic" draws the very archive the dropped row would have, and because
/// `baseline_art` is derived from the SAME filtered list, a sidecar that happens
/// to name the default resolves to row 0 with no change recorded — so no
/// `pictures` key is written and none is cleared.
fn without_the_default(
    candidates: Vec<ArtCandidate>,
    default_art: Option<&DefaultArt>,
) -> Vec<ArtCandidate> {
    let Some(default) = default_art else { return candidates };
    candidates.into_iter().filter(|c| !c.filename.eq_ignore_ascii_case(&default.filename)).collect()
}

/// Which part of a multi-part set this name is, or `None` when it is not one.
///
/// The digit an Infocom PC archive ends its extension with — `.EG1` is part 1,
/// `.EG2` part 2 — and nothing else here reads it, because a Macintosh or Amiga
/// archive is never split.
fn part_of_name(name: &str) -> Option<u8> {
    let (_, ext) = name.rsplit_once('.')?;
    let d = ext.as_bytes().last()?;
    d.is_ascii_digit().then(|| d - b'0')
}

/// Merge the later parts of a multi-part archive that live on the same volume.
///
/// The medium's answer to [`crate::graphics::absorb_continuations`], which reads
/// the host filesystem. A part that will not parse or will not append ends the
/// merge where it is: this list is display-only and silent by design, and the
/// loud version is `PictureOverride::warning` on the archive actually chosen.
fn absorb_medium_continuations(
    pics: &mut InfocomPics,
    name: &str,
    parts: &[(&str, &[u8])],
) {
    while let Some(next) = crate::graphics::part_name(name, pics.next_part()) {
        let Some((_, raw)) = parts.iter().find(|(n, _)| n.eq_ignore_ascii_case(&next)) else {
            return; // no such part: the set ends here, as most do.
        };
        let Ok(part) = InfocomPics::parse(raw.to_vec()) else { return };
        if pics.append_part(part).is_err() {
            return;
        }
    }
}

/// The folder part of a name the medium spells — everything before the last
/// `/`, and `""` at the volume root.
///
/// Both [`crate::hints::mounted_stories`] and [`crate::assets::files`] report a
/// medium's names the same way (`MAC/ZORK ZERO/STORY.DATA`), so comparing this
/// of a story against this of an archive asks "did these ship in the same
/// folder" without either side knowing what a folder is.
fn folder_of(name: &str) -> &str {
    match name.rfind('/') {
        Some(at) => &name[..at],
        None => "",
    }
}

/// What a key did to the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOptionsAction {
    /// Nothing the caller must act on.
    None,
    /// Start the story with these options.
    Play,
    /// Back out; launch nothing.
    Cancel,
}

impl LaunchOptionsState {
    /// Open the dialog for `story_path`, seeded with what it already inherits.
    ///
    /// `inherited_pictures` is the per-game sidecar's `pictures` key (`None` when
    /// absent), and `inherited_interpreter` the effective number in force before
    /// the dialog — sidecar first, then whatever the config/CLI settled.
    pub fn new(
        title: &str,
        story_path: &Path,
        inherited_pictures: Option<&str>,
        inherited_interpreter: Option<u8>,
        z_version: Option<u8>,
        disk_image: Option<crate::hints::DiskImage>,
    ) -> LaunchOptionsState {
        let default_art = resolved_default_art(story_path, None);
        let candidates =
            without_the_default(discover_art_candidates(story_path, None), default_art.as_ref());
        // A sidecar naming an archive that is not in the list (an absolute path,
        // or a file that no longer parses) still deserves to be the baseline —
        // "inherit" is what it is, and the dialog must not silently re-point the
        // story at the Blorb just because it could not match the name.
        let art = inherited_pictures
            .and_then(|name| candidates.iter().position(|c| c.filename.eq_ignore_ascii_case(name)))
            .map_or(0, |i| i + 1);
        LaunchOptionsState {
            title: title.to_string(),
            story_path: story_path.to_path_buf(),
            candidates,
            inherited_pictures: inherited_pictures.map(str::to_string),
            // Resolved with no entry here and re-resolved by `on_disk_entry`:
            // which story on the medium is bound after construction, and on a
            // compilation that is what decides which archive is the default
            // (SQ-0876).
            default_art,
            art,
            interpreter: inherited_interpreter,
            persist: false,
            cursor: Row::Art(art),
            focus: 0,
            baseline_art: art,
            baseline_interpreter: inherited_interpreter,
            z_version,
            disk_image,
            disk_entry: None,
        }
    }

    /// Bind this dialog to one story on a multi-story disk image (SQ-0859).
    ///
    /// Re-resolves the default artwork, because on a disc that keeps its games
    /// in folders the answer is the STORY's and not the platter's — every
    /// graphical game on the Masterpieces CD otherwise reports Arthur's
    /// `CPIC.DATA` as what it will draw with (SQ-0876). One extra mount, once,
    /// when the dialog opens on a disk entry; `resolved_default_art` documents
    /// why that cost is paid at open time and not per frame.
    pub fn on_disk_entry(mut self, disk_entry: Option<&str>) -> LaunchOptionsState {
        self.disk_entry = disk_entry.map(str::to_string);
        if self.disk_entry.is_some() {
            let entry = self.disk_entry.as_deref();
            self.default_art = resolved_default_art(&self.story_path, entry);
            // …and the LIST too, not only the default row: a compilation offers
            // one game's archives, not the whole platter's (SQ-0876).
            self.candidates = without_the_default(
                discover_art_candidates(&self.story_path, entry),
                self.default_art.as_ref(),
            );
            self.art = self
                .inherited_pictures
                .as_deref()
                .and_then(|name| {
                    self.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case(name))
                })
                .map_or(0, |i| i + 1);
            self.cursor = Row::Art(self.art);
        }
        self
    }

    /// The chosen archive, or `None` for "inherit".
    pub fn chosen_art(&self) -> Option<&ArtCandidate> {
        self.art.checked_sub(1).and_then(|i| self.candidates.get(i))
    }

    /// The number this launch would advertise, and where it came from.
    pub fn derived(&self) -> Option<(u8, InterpreterSource)> {
        derived_interpreter(self.interpreter, self.chosen_art(), self.disk_image, self.z_version)
    }

    /// Total selectable rows: one per art choice (plus "inherit"), the
    /// interpreter row, and the persist checkbox.
    pub fn row_count(&self) -> usize {
        self.candidates.len() + 3
    }

    /// The cursor as a flat row index.
    pub fn cursor_index(&self) -> usize {
        match self.cursor {
            Row::Art(i) => i,
            Row::Interpreter => self.candidates.len() + 1,
            Row::Persist => self.candidates.len() + 2,
        }
    }

    /// Move the cursor to a flat row index (clamped).
    pub fn set_cursor_index(&mut self, idx: usize) {
        let n = self.candidates.len();
        self.cursor = if idx <= n {
            Row::Art(idx)
        } else if idx == n + 1 {
            Row::Interpreter
        } else {
            Row::Persist
        };
    }

    /// The overrides this launch should carry: only what actually differs from
    /// what the story already inherits. An untouched dialog produces
    /// [`LaunchOverrides::default`], so opening it and pressing Play is
    /// indistinguishable from launching normally.
    pub fn overrides(&self) -> LaunchOverrides {
        LaunchOverrides {
            pictures: (self.art != self.baseline_art)
                .then(|| self.chosen_art().map(|c| c.filename.clone()))
                .flatten(),
            interpreter_number: (self.interpreter != self.baseline_interpreter)
                .then_some(self.interpreter)
                .flatten(),
        }
    }

    /// Did the user clear a choice they had inherited? Selecting "inherit" over a
    /// sidecar that names an archive cannot be expressed as an override (there is
    /// no "override with nothing"), so this launch honours it only via the
    /// checkbox — and the dialog says so rather than pretending otherwise.
    pub fn clears_inherited_art(&self) -> bool {
        self.art == 0 && self.baseline_art != 0
    }

    /// Same, for the interpreter number.
    pub fn clears_inherited_interpreter(&self) -> bool {
        self.interpreter.is_none() && self.baseline_interpreter.is_some()
    }

    /// Handle one keystroke.
    ///
    /// The model is the settings screen's, deliberately, because that is the
    /// idiom this project already edits values with: Up/Down move a row cursor,
    /// **Space** acts on the row under it, Left/Right cycle a row that has
    /// several values, Tab/Shift-Tab move button focus, Enter activates the
    /// focused button, Esc cancels. `config_screen_key_to_action` says it in one
    /// line — *"Enter activates the focused button; Space still toggles the
    /// selected row"* — which is what "Space is widget-reserved" means: the
    /// focused widget owns it, and dialog chrome must not.
    pub fn on_key(&mut self, key: crossterm::event::KeyEvent) -> LaunchOptionsAction {
        use crossterm::event::{KeyCode, KeyModifiers};
        let n = self.row_count();
        match key.code {
            KeyCode::Up => {
                self.set_cursor_index(self.cursor_index().saturating_sub(1));
                LaunchOptionsAction::None
            }
            KeyCode::Down => {
                self.set_cursor_index((self.cursor_index() + 1).min(n - 1));
                LaunchOptionsAction::None
            }
            KeyCode::Home => {
                self.set_cursor_index(0);
                LaunchOptionsAction::None
            }
            KeyCode::End => {
                self.set_cursor_index(n - 1);
                LaunchOptionsAction::None
            }
            KeyCode::Tab => {
                self.focus = crate::input::cycle_focus(self.focus, 2, 1);
                LaunchOptionsAction::None
            }
            KeyCode::BackTab => {
                self.focus = crate::input::cycle_focus(self.focus, 2, -1);
                LaunchOptionsAction::None
            }
            KeyCode::Left => {
                self.cycle(-1);
                LaunchOptionsAction::None
            }
            KeyCode::Right => {
                self.cycle(1);
                LaunchOptionsAction::None
            }
            KeyCode::Char(' ') if key.modifiers == KeyModifiers::NONE => {
                self.activate_row();
                LaunchOptionsAction::None
            }
            KeyCode::Enter => {
                if self.focus == 1 { LaunchOptionsAction::Cancel } else { LaunchOptionsAction::Play }
            }
            KeyCode::Esc => LaunchOptionsAction::Cancel,
            _ => LaunchOptionsAction::None,
        }
    }

    /// Space on the cursor row: select this archive, advance the interpreter by
    /// one, or flip the checkbox.
    fn activate_row(&mut self) {
        match self.cursor {
            Row::Art(i) => self.art = i,
            Row::Interpreter => self.cycle(1),
            Row::Persist => self.persist = !self.persist,
        }
    }

    /// Left/Right on the cursor row. Only rows with an ordered set of values
    /// respond; an art row is a radio button, not a cycler.
    fn cycle(&mut self, dir: isize) {
        match self.cursor {
            Row::Interpreter => {
                let cur = INTERPRETER_CHOICES.iter().position(|c| *c == self.interpreter).unwrap_or(0);
                let len = INTERPRETER_CHOICES.len();
                let next = (cur as isize + dir).rem_euclid(len as isize) as usize;
                self.interpreter = INTERPRETER_CHOICES[next];
            }
            Row::Persist => self.persist = !self.persist,
            Row::Art(_) => {}
        }
    }

    /// Write the checkbox's promise to `<game_dir>/config.toml`: the keys the
    /// user actually changed, and **only** those.
    ///
    /// A key the dialog merely *displayed* is never written. That is not tidiness
    /// — the sidecar's contract is "at most a few keys; absent key = inherit"
    /// (CLAUDE.md), so writing a key at the value it already inherits converts an
    /// inheritance into a pin, and the story stops tracking a later change to the
    /// global config. Only a difference from the baseline is a decision.
    pub fn persist_to(&self, game_dir: &Path) -> std::io::Result<()> {
        if self.art != self.baseline_art {
            crate::styles::write_per_game_pictures(
                game_dir,
                self.chosen_art().map(|c| c.filename.clone()),
            )?;
        }
        if self.interpreter != self.baseline_interpreter {
            crate::styles::write_per_game_interpreter_number(game_dir, self.interpreter)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stories_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    fn tmp(tag: &str) -> PathBuf {
        crate::scratch_dir(&format!("launchopt-{tag}"))
    }

    #[test]
    fn art_names_are_recognised_but_bare_dot_data_is_not() {
        assert!(looks_like_art_name("zork0.mg1"));
        assert!(looks_like_art_name("ZORK0.EG1"));
        assert!(looks_like_art_name("arthur.cg2"));
        assert!(looks_like_art_name("zork0.pic"));
        assert!(looks_like_art_name("Pic.data"));
        assert!(looks_like_art_name("CPIC.DATA"));
        // `.data` is far too common a suffix to admit on its own.
        assert!(!looks_like_art_name("saved.data"));
        assert!(!looks_like_art_name("zork0.z6"));
        assert!(!looks_like_art_name("Zork0.blb"));
        assert!(!looks_like_art_name("notes"));
    }

    #[test]
    fn an_archive_belongs_to_a_story_when_either_name_contains_the_other() {
        // The four shapes the real library actually has.
        assert!(belongs_to_story("zork0-r393-s890714", "zork0", "zork0.mg1"));
        assert!(belongs_to_story("beyondzork-r57-s871221", "beyondzo", "beyondzo.mg1"));
        assert!(belongs_to_story("James Clavell's Shogun", "shogun", "shogun.mg1"));
        assert!(belongs_to_story("fmvpoker", "FMVPOKER", "FMVPOKER.EG1"));
        assert!(belongs_to_story("Beyond Zork - The Coconut of Quendor", "beyondzo", "beyondzo.mg1"));

        // …and the ones it must decline, which is the whole point of filtering.
        assert!(!belongs_to_story("zork0-r393-s890714", "arthur", "arthur.mg1"));
        assert!(!belongs_to_story("zork0-r393-s890714", "journey", "journey.mg1"));
        assert!(!belongs_to_story("zork0-r393-s890714", "FMVPOKER", "FMVPOKER.EG1"));

        // A stem too short to mean anything matches nothing…
        assert!(!belongs_to_story("epic-adventure", "pic", "pic.mg1"));
        // …but the two names an Amiga floppy actually uses carry no identity at
        // all, so they are offered rather than hidden.
        assert!(belongs_to_story("epic-adventure", "Pic", "Pic.data"));
        assert!(belongs_to_story("anything", "CPIC", "CPIC.DATA"));
    }

    /// The Macintosh disk carries two `Flavour::AmigaMac` archives, so the label
    /// is the only thing that can tell them apart in a list (SQ-0843). The
    /// two-colour one names the machine bocfel's `0x0e` heuristic names, and the
    /// colour one stays honestly ambiguous because there the codec really cannot
    /// say. A two-colour PC archive is a `.cg1` and is unaffected.
    #[test]
    fn the_two_colour_amiga_mac_archive_is_labelled_as_the_macintoshs() {
        assert_eq!(rendition_label(Flavour::AmigaMac, 480, "Pic.data", true), "Mac B&W");
        assert_eq!(rendition_label(Flavour::AmigaMac, 320, "CPic.data", false), "Amiga");
        assert_eq!(rendition_label(Flavour::AmigaMac, 320, "zork0.pic", false), "Amiga");
        // The PC arm reads its extension exactly as before; CGA is two-colour
        // too and must not be relabelled by the new axis.
        assert_eq!(rendition_label(Flavour::Pc, 640, "zork0.cg1", true), "CGA");
        assert_eq!(rendition_label(Flavour::Pc, 640, "zork0.eg1", false), "EGA");
        assert_eq!(rendition_label(Flavour::Pc, 320, "zork0.mg1", false), "MCGA");
        assert_eq!(rendition_label(Flavour::Pc, 640, "mystery.dat", false), "EGA/CGA");
    }

    #[test]
    fn a_name_shaped_file_that_is_not_an_archive_is_not_a_candidate() {
        // Display-only discovery must not offer something that cannot be used:
        // the list answers "what could you pick?", and an unparseable file is not
        // an answer. (A file NAMED in the sidecar is the opposite case, and stays
        // loud — see PictureOverride::resolve.)
        let dir = tmp("bogus");
        std::fs::write(dir.join("story.z6"), b"not a story").unwrap();
        std::fs::write(dir.join("story.mg1"), b"nowhere near an archive").unwrap();
        assert!(discover_art_candidates(&dir.join("story.z6"), None).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zork_zeros_renditions_are_all_listed_with_their_flavours() {
        let z0 = stories_dir().join("zork0-r393-s890714.z6");
        if !z0.is_file() {
            return; // gitignored fixtures; skip vacuously (CI-safe)
        }
        let found = discover_art_candidates(&z0, None);
        let by_name = |n: &str| found.iter().find(|c| c.filename.eq_ignore_ascii_case(n)).cloned();
        // Every rendition the SQ-0734 note tabulates, each labelled from its own
        // codec and picture-space width rather than from its name.
        if let Some(c) = by_name("zork0.mg1") {
            assert_eq!(c.rendition, "MCGA");
            assert_eq!(c.flavour, Flavour::Pc);
            assert_eq!(c.space_width, 320);
            assert!(c.pictures > 100, "a real archive holds many pictures, got {}", c.pictures);
            assert!(c.caveat().is_none(), "MCGA draws correctly today");
        }
        if let Some(c) = by_name("zork0.eg1") {
            assert_eq!(c.rendition, "EGA");
            assert_eq!(c.space_width, 640);
            // SQ-0815: EGA's dithering caveat is GONE, because SQ-0797 fused the
            // dither it warned about. It had already outlived SQ-0790's geometry
            // fix once; the user saw the dialog still warning about both.
            assert!(c.caveat().is_none(), "EGA draws correctly since SQ-0797 fused its dither");
            // Zork Zero's 360K release gave EGA a whole disk, so this one really
            // is complete on its own — the multi-part path must not invent a
            // second file for it (SQ-0798).
            assert_eq!(c.parts, 1, "zork0.eg1 is a single-part archive");
            assert_eq!(c.pictures, 503, "the whole zork0.eg1 directory");
        }
        if let Some(c) = by_name("zork0.cg1") {
            // 640-wide like EGA, and no caveat: CGA is two-colour line art, so
            // there is no dithered colour to fuse (SQ-0794 / SQ-0797).
            assert_eq!(c.rendition, "CGA");
            assert_eq!(c.space_width, 640);
            assert!(c.caveat().is_none(), "CGA line art draws correctly at 1:1");
        }
        if let Some(c) = by_name("zork0.pic") {
            assert_eq!(c.rendition, "Amiga");
            assert_eq!(c.flavour, Flavour::AmigaMac);
            assert!(c.caveat().is_none());
        }
        // The Blorb beside it is not a native archive and is never a candidate.
        assert!(by_name("Zork0.blb").is_none(), "a Blorb is tier 1, not a pickable archive");
        // …and neither is another game's art, which is the filter's whole job.
        assert!(by_name("arthur.mg1").is_none(), "Arthur's plates are not Zork Zero's choice");
        assert!(by_name("journey.mg1").is_none());
    }

    #[test]
    fn an_untouched_dialog_overrides_nothing() {
        let dir = tmp("untouched");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let st = LaunchOptionsState::new("Story", &story, None, None, Some(6), None);
        assert_eq!(st.overrides(), LaunchOverrides::default());
        assert!(st.overrides().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checkbox_writes_only_what_changed() {
        // The inherit contract: a key present at its inherited value is NOT the
        // same as a key absent, so a dialog that wrote every field it displayed
        // would quietly pin three settings the user never touched.
        let dir = tmp("persist");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let game_dir = dir.join("game");
        let mut st = LaunchOptionsState::new("Story", &story, None, None, Some(6), None);

        // Nothing changed → nothing written; the sidecar is not even created.
        st.persist_to(&game_dir).unwrap();
        assert!(!crate::styles::per_game_config_path(&game_dir).exists());

        // Change only the interpreter → only that key lands.
        st.interpreter = Some(4);
        st.persist_to(&game_dir).unwrap();
        let body = std::fs::read_to_string(crate::styles::per_game_config_path(&game_dir)).unwrap();
        assert!(body.contains("interpreter_number = 4"), "got {body:?}");
        assert!(!body.contains("pictures"), "an untouched art choice must stay absent: {body:?}");
        assert!(!body.contains("honor_game_colours"), "unrelated keys stay absent: {body:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_derived_interpreter_reports_its_provenance() {
        // Explicit beats everything.
        assert_eq!(
            derived_interpreter(Some(3), None, Some(crate::hints::DiskImage::Adf), Some(6)),
            Some((3, InterpreterSource::Explicit))
        );
        // A story with no Z version (Glulx/Scott) has no header 0x1E to report.
        assert_eq!(derived_interpreter(None, None, None, None), None);
        // The medium.
        assert_eq!(
            derived_interpreter(None, None, Some(crate::hints::DiskImage::Adf), Some(6)),
            Some((4, InterpreterSource::DiskImage))
        );
        // Frotz's rule, both halves.
        assert_eq!(derived_interpreter(None, None, None, Some(6)), Some((6, InterpreterSource::Default)));
        assert_eq!(derived_interpreter(None, None, None, Some(5)), Some((1, InterpreterSource::Default)));
    }

    #[test]
    fn picking_amiga_art_moves_the_machine_and_the_dialog_can_say_so() {
        // The surprise this dialog exists to prevent: the art choice moves the
        // emulated machine unless a number is set explicitly.
        let amiga = ArtCandidate {
            path: PathBuf::from("/x/Pic.data"),
            filename: "Pic.data".into(),
            flavour: Flavour::AmigaMac,
            rendition: "Amiga",
            pictures: 172,
            part: 1,
            parts: 1,
            space_width: 320,
            on_medium: false,
            disk_number: None,
        };
        let pc = ArtCandidate { flavour: Flavour::Pc, rendition: "MCGA", ..amiga.clone() };
        assert_eq!(
            derived_interpreter(None, Some(&amiga), None, Some(6)),
            Some((4, InterpreterSource::Artwork))
        );
        assert_eq!(
            derived_interpreter(None, Some(&pc), None, Some(6)),
            Some((6, InterpreterSource::Artwork))
        );
        // …and an explicit number still wins over the art.
        assert_eq!(
            derived_interpreter(Some(6), Some(&amiga), None, Some(6)),
            Some((6, InterpreterSource::Explicit))
        );
    }

    #[test]
    fn keys_follow_the_settings_screen_idiom() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let dir = tmp("keys");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let mut st = LaunchOptionsState::new("Story", &story, None, None, Some(6), None);
        let k = |c| KeyEvent::new(c, KeyModifiers::NONE);

        // No candidates here, so rows are: Art(0), Interpreter, Persist.
        assert_eq!(st.row_count(), 3);
        assert_eq!(st.cursor, Row::Art(0));
        st.on_key(k(KeyCode::Down));
        assert_eq!(st.cursor, Row::Interpreter);
        // Right cycles the interpreter forward off auto; Left comes back.
        st.on_key(k(KeyCode::Right));
        assert_eq!(st.interpreter, Some(1));
        st.on_key(k(KeyCode::Left));
        assert_eq!(st.interpreter, None);
        // Space acts on the row under the cursor (the settings-screen rule).
        st.on_key(k(KeyCode::Char(' ')));
        assert_eq!(st.interpreter, Some(1), "Space advances the interpreter row");

        st.on_key(k(KeyCode::Down));
        assert_eq!(st.cursor, Row::Persist);
        assert!(!st.persist);
        st.on_key(k(KeyCode::Char(' ')));
        assert!(st.persist, "Space toggles the checkbox");
        st.on_key(k(KeyCode::Char(' ')));
        assert!(!st.persist);

        // Down at the end does not run off; Up/Home/End behave.
        st.on_key(k(KeyCode::Down));
        assert_eq!(st.cursor, Row::Persist);
        st.on_key(k(KeyCode::Home));
        assert_eq!(st.cursor, Row::Art(0));
        st.on_key(k(KeyCode::End));
        assert_eq!(st.cursor, Row::Persist);

        // Tab moves button focus and Shift-Tab reverses it (standing policy).
        assert_eq!(st.focus, 0);
        st.on_key(k(KeyCode::Tab));
        assert_eq!(st.focus, 1);
        assert_eq!(st.on_key(k(KeyCode::Enter)), LaunchOptionsAction::Cancel);
        st.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(st.focus, 0);
        assert_eq!(st.on_key(k(KeyCode::Enter)), LaunchOptionsAction::Play);
        assert_eq!(st.on_key(k(KeyCode::Esc)), LaunchOptionsAction::Cancel);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selecting_an_archive_produces_exactly_one_override() {
        let z0 = stories_dir().join("zork0-r393-s890714.z6");
        if !z0.is_file() {
            return;
        }
        let mut st = LaunchOptionsState::new("Zork Zero", &z0, None, None, Some(6), None);
        let Some(idx) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.mg1")) else {
            return;
        };
        st.art = idx + 1;
        let ov = st.overrides();
        assert_eq!(ov.pictures.as_deref(), Some("zork0.mg1"));
        assert_eq!(ov.interpreter_number, None, "an untouched interpreter row overrides nothing");
        // Seeding from that same sidecar value makes it the baseline, so the
        // dialog reopened on it overrides nothing at all.
        let st2 = LaunchOptionsState::new("Zork Zero", &z0, Some("zork0.mg1"), None, Some(6), None);
        assert_eq!(st2.art, idx + 1);
        assert!(st2.overrides().is_empty());
    }
}
