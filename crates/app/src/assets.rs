//! Where a story's assets come from — **one enumeration over every place a
//! story's files can live** (SQ-0843).
//!
//! # The defect this exists to end
//!
//! SQ-0840 unified how a disk image is *mounted and queried*: one table in
//! `blorb::medium`, one vocabulary, and no front-end that names a format. What
//! it could not unify is code that never mounted anything — and the art
//! enumeration in [`crate::launch_options`] was exactly that, a `read_dir` of
//! the story's own directory, written before lanthorn could open a disk at all.
//!
//! So a story opened out of `stories/Zork Zero Disk.image` had **no** pickable
//! artwork: the two archives it wants are on the volume, and a directory scan
//! cannot see inside one. The Macintosh's two-colour `Pic.data` was reachable
//! only by typing its name into `--pictures` or the per-game `pictures` key.
//!
//! The user's rule, 2026-08-13: *"let's make sure we unify how this works
//! across all disk formats so we are not repeating ourselves"*.
//!
//! # The shape, and the property it buys
//!
//! [`files`] answers *"which files can this story's assets be read from?"*, and
//! it unions every SOURCE a story has: the host directory beside it, and the
//! volumes of the release it came off when it came off a disk image. Callers
//! filter that one list for the asset kind they want.
//!
//! Three kinds of growth, and each costs exactly one edit:
//!
//! | what is added | where it goes |
//! | --- | --- |
//! | a disk **format** | a row in `blorb::medium::FORMATS` (SQ-0840) |
//! | an asset **source** | an arm in [`files`], below |
//! | an asset **kind** | a filter over [`files`], written once |
//!
//! # The release, not the platter (SQ-0862)
//!
//! [`volumes`] is the one place that knows how far a story's assets reach, and
//! every consumer of the medium goes through it: [`files`] for the enumeration,
//! `graphics::PictSource::resolve` for the automatic pick, and
//! `graphics::read_off_the_medium` for a name the user typed. See that
//! function's docs for the rule and what it refuses.
//!
//! **No caller learns that disk images exist.** That is the whole point:
//! `launch_options` asks this module what files there are and applies its own
//! "is this a picture archive?" test to each, so the next asset kind that needs
//! the same answer inherits disk support rather than re-deriving it.
//!
//! # What this module does NOT do
//!
//! It does not migrate the hint-sidecar directory scan (`hints.rs`), and that is
//! deliberate: no disk in the corpus carries a hint sidecar, the Solid Gold
//! releases carry their hints *inside the story file* where `hints.rs` already
//! mounts to reach them, and building for a case with no example is the
//! speculative generality this project's rules forbid. The seam is shaped so
//! that job would be a filter and not a rewrite; it is not done here.
//!
//! It also does not identify anything. A file is a name, a place and some bytes;
//! whether those bytes are artwork is the caller's test, applied identically to
//! every source — which is what stops a volume's files being classified by one
//! rule and a directory's by another.
//!
//! # Cost
//!
//! The loose arm is **name-first**: it lists a directory and reads nothing, so a
//! caller can decline a file on its name before paying for its megabytes. That
//! property is load-bearing — the story browser's info panel asks per highlight,
//! and a flat library holds a dozen archives.
//!
//! The medium arm cannot be name-first, because a volume must be mounted before
//! it can be listed. It costs one read of the story file plus, when that file
//! really is a disk image, the mount. Measured on `stories/Zork Zero Disk.image`
//! (838 KB, DiskCopy 4.2 around HFS, five files): **0.2 ms** to read and sniff,
//! **0.5 ms** to mount and list — against 6.4 ms cold / 1.5 ms warm for the
//! whole of [`crate::launch_options::discover_art_candidates`] on that story,
//! most of which is parsing two Huffman picture directories. A story that is not
//! a disk image pays only the read and the sniff — ~0.3 ms for a 300 KB `.z6` —
//! because [`blorb::medium::DiskImage::detect`] refuses it before anything is
//! mounted.
//!
//! A story on a multi-disk release pays that mount once per volume, and [`volumes`]
//! stops as soon as the release turns out to be a shelf rather than one game — so
//! the twenty-game DOS `floppy1.ima`…`floppy5.ima` set costs two mounts, not five.
//!
//! The story browser's info panel asks once per highlight, into the `StoryAux`
//! cache that already holds everything per-story that touches the disk, so none
//! of this is per frame.

use std::path::{Path, PathBuf};

/// Where one candidate file lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetOrigin {
    /// A loose file in the story's own directory.
    BesideTheStory,
    /// A file on a volume of the release the story was mounted out of. Its
    /// identity is guaranteed by the medium — it shipped in the box with the
    /// story — which is exactly what a loose file's name has to be tested for.
    ///
    /// Deliberately **not** split into "the story's own platter" and "a sibling
    /// volume": [`volumes`] only offers a sibling when the release carries one
    /// game, and at that point the two assertions are the same assertion. A
    /// caller that wants to know which image a file came off reads
    /// [`AssetFile::path`], which names it.
    OnTheMedium,
}

/// One file a story's assets could be read from, wherever it lives.
///
/// The bytes are deliberately *not* a public field. A loose file has not been
/// read yet and must not be until a caller decides it wants it; a file off a
/// volume was read by the mount and is already in hand. [`AssetFile::into_bytes`]
/// is the one door, and it hides which of those two happened.
#[derive(Debug, Clone)]
pub struct AssetFile {
    /// The filename, exactly as the directory or the volume spells it — which on
    /// FAT12, the one format with directories, includes the folder
    /// (`HITCHHIK/STORY.DAT`). This is what goes into a `pictures = "…"` key:
    /// both doors resolve the name against the story, the host filesystem first
    /// and then the medium (see `crate::graphics::read_off_the_medium`), so
    /// whatever is shown here can be asked for.
    pub name: String,
    /// The host path to open: the file itself when it is loose, and the disk
    /// image when it is on a volume — which is the file a person opened either
    /// way, and the only one that exists on this machine.
    pub path: PathBuf,
    /// Which source it came from.
    pub origin: AssetOrigin,
    /// **Which volume of the release** carries it, when the release is a
    /// multi-disk set (SQ-0865): the disk number `crate::disk_set` reads off the
    /// set's varying index, which is the release's own spelling and not a
    /// position in a list.
    ///
    /// `None` for a loose file, and `None` for a release that is a single image
    /// — a story whose art is on the one disk it booted from has a "game disk"
    /// and no disk *number*, and the two cases read differently to a person.
    pub disk_number: Option<u64>,
    /// Already in hand for a volume's file; `None` for a loose one, which is
    /// what keeps the directory arm name-first.
    bytes: Option<Vec<u8>>,
}

impl AssetFile {
    /// Is this file inside the medium rather than beside the story?
    pub fn is_on_medium(&self) -> bool {
        self.origin == AssetOrigin::OnTheMedium
    }

    /// The bytes a MEDIUM file already has in hand, without consuming the entry
    /// and without touching the host filesystem — `None` for a loose file,
    /// whose bytes are deliberately not read until someone commits to it.
    ///
    /// For a caller that needs to look at one file while deciding about
    /// another: the continuation parts of a multi-part archive are on the same
    /// volume, and resolving them must not mean mounting it twice.
    pub fn peek_bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    /// The file's bytes, read now if they were not read already. `None` when a
    /// loose file will not read — a caller that cannot use it simply skips it.
    pub fn into_bytes(self) -> Option<Vec<u8>> {
        match self.bytes {
            Some(bytes) => Some(bytes),
            None => std::fs::read(&self.path).ok(),
        }
    }
}

/// Every file `story_path`'s assets could be read from: the loose files beside
/// it, then the files on every volume of the release it was mounted out of.
///
/// Loose files come first and in directory order; the caller sorts. Nothing is
/// identified and nothing is filtered — see the module header.
pub fn files(story_path: &Path) -> Vec<AssetFile> {
    let mut out = beside_the_story(story_path);
    out.extend(on_the_medium(story_path));
    out
}

/// Source 1: the story's own directory, listed and not read.
fn beside_the_story(story_path: &Path) -> Vec<AssetFile> {
    let dir = match story_path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?.to_string();
            Some(AssetFile {
                name,
                path,
                origin: AssetOrigin::BesideTheStory,
                disk_number: None,
                bytes: None,
            })
        })
        .collect()
}

/// Source 2: the release the story came out of, when it came off a disk image.
///
/// Content, not extension, and one mount path for every format — the sniff and
/// the open are both `blorb::medium`'s, so a format lanthorn can detect is a
/// format this arm enumerates, and adding one touches nothing here (SQ-0840).
///
/// A story that is not an image costs the read and the sniff and stops.
fn on_the_medium(story_path: &Path) -> Vec<AssetFile> {
    volumes(story_path)
        .into_iter()
        .flat_map(|v| {
            v.disk.contents().into_iter().map(move |(name, bytes)| AssetFile {
                name,
                path: v.path.clone(),
                origin: AssetOrigin::OnTheMedium,
                disk_number: v.disk_number,
                bytes: Some(bytes),
            })
        })
        .collect()
}

/// One mounted volume of a release, with the host path of the image that carries
/// it — which is the file a person actually has, and the only one that exists on
/// this machine.
pub struct MountedVolume {
    /// The disk image on the host filesystem.
    pub path: PathBuf,
    /// The open volume.
    pub disk: blorb::medium::MountedDisk,
    /// Which volume of its release this is, when the release is a multi-disk
    /// set — `crate::disk_set`'s own index, not a position in this list.
    /// `None` when the release is a single image (SQ-0865).
    pub disk_number: Option<u64>,
}

/// Every volume of the release `story_path` came off, **the story's own image
/// first** and the rest in disk order. Empty when the story is not a disk image
/// at all (SQ-0862).
///
/// # The defect this exists to end
///
/// The DOS press of Zork Zero splits the story from its artwork across disks.
/// On the 360K press the story disk holds *no artwork whatever*: `ZORK0.ZIP` is
/// alone on disk 2 with `ZORKZERO.EXE`, CGA's `ZORK0.CG1` is on disk 1 and EGA's
/// `ZORK0.EG1` on disk 3. Mounting only the image the story came off therefore
/// found nothing to draw with, and the launch dialog offered nothing to pick —
/// which is exactly what the user reported. The 720K press shows the other half
/// of it: MCGA shares the story's disk and was found, CGA on disk 2 was not.
///
/// A release is not a platter. `crate::disk_set` already computes which files
/// are volumes of one release, from their names and without opening anything;
/// this is that answer applied to assets.
///
/// # Why a sibling volume may speak for the story, and when it may not
///
/// `crate::graphics::PictureOverride`'s tier policy turns on the strength of the
/// PAIRING evidence, and the medium's is the strongest thing short of a person
/// saying so: the story and the archive shipped in one box. That argument covers
/// disk 3 of a three-disk Zork Zero exactly as well as it covers the platter the
/// story sits on — the box is the same box.
///
/// It stops covering it the moment the box holds more than one game. And the
/// corpus has that case, sharply: DOS `floppy1.ima`…`floppy5.ima` is *The Lost
/// Treasures of Infocom*, twenty stories across five disks, with Zork Zero's
/// `ZORK0.CG1` on floppy 4 and its `ZORK0.EG1` on floppy 5. Widening across that
/// set unconditionally would offer Zork Zero's plates to Zork I, which is
/// precisely the invisible mis-pairing the tiers exist to prevent.
///
/// So: **a sibling contributes only when the release carries exactly one story.**
/// That threshold is the mirror of `crate::picker::StorySource::of`'s, which
/// declines to open a *menu* for a set offering fewer than two games, and between
/// them they partition the space with no overlap and no gap — a set with two or
/// more games gets a browser, a set with one gets its siblings' assets.
///
/// # What it never removes
///
/// The story's own volume is unconditional and always first. A single image, a
/// story with loose art beside it, and every multi-game compilation resolve
/// exactly as they did before this function existed; the widening can only ever
/// add.
///
/// # Cost
///
/// One read and one mount per volume, and the story count is tallied as it goes
/// so a shelf is abandoned as soon as its second game appears — `floppy5.ima`
/// costs two mounts of its five, because floppy 1 carries four stories on its
/// own. A story that is not a disk image pays the read and the sniff and stops,
/// and `disk_set::members` is never even asked, because it wants a disk-image
/// extension and answers `None` without touching the disk.
pub fn volumes(story_path: &Path) -> Vec<MountedVolume> {
    // The release's volumes and the disk number each one carries, from names
    // alone — no disk is opened to answer it, so asking before the mount below
    // costs one `read_dir` and nothing else (SQ-0865).
    let members = crate::disk_set::members_indexed(story_path);
    // The story's OWN volume is the one mounted across the set, because it is
    // the one every caller here asks first (SQ-0863). Its siblings are mounted
    // plainly: a set-spanning container answers the same for whichever volume
    // holds the question, so mounting all five of them across all five would buy
    // one answer at five times the reads.
    let Some(mut own) = mount_across(story_path, members.as_deref()) else {
        return Vec::new();
    };
    own.disk_number =
        members.as_ref().and_then(|m| m.iter().find(|(_, p)| p == story_path).map(|(n, _)| *n));
    let mut stories = own.disk.stories().len();
    let mut out = vec![own];
    if stories > 1 {
        return out; // one volume of a compilation: its siblings are other games'
    }
    let Some(members) = members else {
        return out;
    };
    let mut siblings = Vec::new();
    for (number, m) in members {
        if m == story_path {
            continue;
        }
        let Some(mut v) = mount(&m) else { continue };
        v.disk_number = Some(number);
        stories += v.disk.stories().len();
        if stories > 1 {
            return out; // a shelf, not a release: the siblings contribute nothing
        }
        siblings.push(v);
    }
    out.extend(siblings);
    out
}

/// Open one image, or `None` when the bytes are not a disk in any format.
///
/// The volume's own disk number is [`volumes`]'s to fill in: it is a property of
/// the release the image belongs to, which one image cannot see.
fn mount(path: &Path) -> Option<MountedVolume> {
    mount_across(path, None)
}

/// [`mount`], with the other volumes of the release available to the mount
/// (SQ-0863).
///
/// `members` is [`crate::disk_set::members_indexed`]'s answer — names only, no
/// disk opened — and `path` itself is filtered out of it here. What the
/// companions can add is what a release keeps on no single volume: the *Shogun*
/// and *Zork Zero* 5.25-inch presses page both their story and their artwork
/// across the whole set, and `blorb::medium::MountedDisk::mount_set` is where
/// those two questions are answered.
///
/// **Nothing is read eagerly.** `mount_set` calls the closure only when the
/// named volume turns out to have no story of its own, so every ordinary floppy,
/// every compilation and every `.2mg` that answers for itself costs exactly the
/// one read it always did. The nine packed 5.25-inch volumes in the corpus are
/// the only ones that pay, and what they pay is the reads they need.
fn mount_across(path: &Path, members: Option<&[(u64, PathBuf)]>) -> Option<MountedVolume> {
    let raw = std::fs::read(path).ok()?;
    blorb::medium::DiskImage::detect(&raw)?;
    let disk = blorb::medium::MountedDisk::mount_set(raw, || {
        members
            .unwrap_or_default()
            .iter()
            .map(|(_, m)| m)
            .filter(|m| m.as_path() != path)
            .filter_map(|m| std::fs::read(m).ok())
            .collect()
    })
    .ok()?;
    Some(MountedVolume { path: path.to_path_buf(), disk, disk_number: None })
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        crate::scratch_dir(&format!("assets-{tag}"))
    }

    #[test]
    fn a_loose_file_is_listed_without_being_read() {
        let dir = tmp("loose");
        std::fs::write(dir.join("story.z6"), b"x").unwrap();
        std::fs::write(dir.join("art.mg1"), b"pretend archive").unwrap();
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        let found = files(&dir.join("story.z6"));
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"art.mg1"), "{names:?}");
        assert!(names.contains(&"story.z6"), "the story itself is a file like any other: {names:?}");
        assert!(!names.contains(&"subdir"), "a directory is not an asset file: {names:?}");
        for f in &found {
            assert_eq!(f.origin, AssetOrigin::BesideTheStory);
            assert!(!f.is_on_medium());
            assert!(f.bytes.is_none(), "{}: the directory arm must stay name-first", f.name);
        }
        let art = found.into_iter().find(|f| f.name == "art.mg1").unwrap();
        assert_eq!(art.into_bytes().as_deref(), Some(&b"pretend archive"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A story that is not release media mounts nothing, so nothing on the
    /// medium arm can cost it anything.
    #[test]
    fn an_ordinary_story_has_no_medium_arm_at_all() {
        let dir = tmp("nomedium");
        std::fs::write(dir.join("story.z6"), b"not a disk image").unwrap();
        assert!(files(&dir.join("story.z6")).iter().all(|f| !f.is_on_medium()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// …and it spans no volumes at all, so [`volumes`] never asks `disk_set` and
    /// never opens a sibling. The widening is invisible to every story that is
    /// not a disk image, which is most of them (SQ-0862).
    #[test]
    fn a_story_that_is_not_a_disk_image_spans_no_volumes() {
        let dir = tmp("novolumes");
        std::fs::write(dir.join("story.z6"), b"not a disk image").unwrap();
        // A neighbour whose name WOULD form a set, to prove the extension and
        // the sniff both have to agree before anything is mounted.
        std::fs::write(dir.join("disk1.img"), b"also not a disk image").unwrap();
        std::fs::write(dir.join("disk2.img"), b"nor this").unwrap();
        assert!(volumes(&dir.join("story.z6")).is_empty());
        assert!(volumes(&dir.join("disk1.img")).is_empty(), "the name is not the evidence");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The seam over a real volume: every file on the disk arrives with its
    /// bytes already in hand, named as the volume spells it, pointing at the
    /// image a person actually opened.
    #[test]
    fn a_disk_images_own_files_are_listed_beside_the_loose_ones() {
        let image = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Zork Zero Disk.image");
        if !image.is_file() {
            eprintln!("SKIP: gitignored Macintosh medium missing at {}", image.display());
            return;
        }
        let found = files(&image);
        let on_disk: Vec<&AssetFile> = found.iter().filter(|f| f.is_on_medium()).collect();
        let names: Vec<&str> = on_disk.iter().map(|f| f.name.as_str()).collect();
        // The volume listing SQ-0838 measured, in full.
        for want in ["CPic.data", "Pic.data", "Story.data", "Zork Zero", "Desktop"] {
            assert!(names.contains(&want), "{want} is on the volume: {names:?}");
        }
        for f in &on_disk {
            assert_eq!(f.path, image, "a volume's file points at the image that carries it");
            assert!(f.bytes.is_some(), "{}: the mount already read it", f.name);
        }
        // …and the loose arm is still there, unchanged, in the same list.
        assert!(
            found.iter().any(|f| !f.is_on_medium() && f.name.eq_ignore_ascii_case("zork0.mg1")),
            "the story's own directory is still enumerated"
        );
    }
}
