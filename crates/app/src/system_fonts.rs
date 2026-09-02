//! Typefaces on the user's OWN boot media under `~/.lanthorn/` — reported to
//! the browser's info panel, and supplied to the face cascade (SQ-1038,
//! SQ-1037, SQ-1053).
//!
//! # Media, not only disks
//!
//! Two of the three media a machine keeps its system face on are volumes and one
//! is not. A Macintosh keeps Geneva in the System file on an HFS disk; an Amiga
//! keeps `topaz/11` in a Workbench `FONTS:` drawer — and keeps **topaz 8**, the
//! face its Version 6 interpreter actually painted prose with, in **Kickstart
//! ROM**, which has no filesystem at all. A `.rom` under `~/.lanthorn/` is read
//! here for that one reason (see [`ROM_EXTENSION`] and
//! [`blorb::amiga_font::faces_in_rom`]); without it every Amiga rung declines and
//! only *Arthur*, which ships its own face on its own floppy, ever gets one.
//!
//! This exists to make SQ-1038's fix visible: before it, `blorb::mac_font` and
//! `blorb::amiga_font` refused every proportional system face (`Glyph::rows` was
//! one byte per row, so nothing wider than 8px could be represented), and a
//! player with a mounted System 6 or Workbench disk beside their stories could
//! not tell. This module answers "what can lanthorn read off my own media" —
//! the same question [`crate::native_font::detected`] answers for a STORY's own
//! medium, asked instead of the disks a person keeps around for their own
//! reasons (a Workbench boot floppy, a System Startup disk) rather than any
//! one game's release.
//!
//! # Reporting and supplying are two doors on one lookup
//!
//! [`scan`] REPORTS what is readable, for the browser's info panel;
//! [`named_faces_in`] SUPPLIES the faces a machine's own body face names, for
//! [`crate::native_font::resolve`] to rank. Both walk the same [`faces_on`], so
//! a panel cannot list a face the cascade would not see, or miss one it would.
//!
//! **Neither decides.** Whether a face that comes back may actually be DRAWN is
//! one question asked in one place — `native_font::fit` — and this module never
//! asks it (SQ-1011 shipped inert twice over a fitness rule that lived in two
//! places). What this module knows is a NAME: the family a Macintosh id encodes,
//! the drawer an AmigaDOS path names, and which machine a volume speaks for.
//!
//! # One lookup, reused rather than rewritten
//!
//! Faces are found through [`blorb::mac_font::faces_in_fork`],
//! [`blorb::amiga_font::faces_in_volume`] and
//! [`blorb::amiga_font::faces_in_rom`] — the same parsers
//! `native_font::detected` calls for a story's own medium, and the LAST of those
//! is the same `TextFont` reader as the middle one with its pointers relocated,
//! not a second one — rather than a third copy of the fitness question. SQ-1011 shipped inert twice because a fitness
//! rule existed in two places and correcting one left the other; there is no
//! fitness rule here at all, only "does it parse", which is one function.
//!
//! # Every entry with a resource fork, not only `APPL`
//!
//! A Macintosh system disk's fonts live in `System Folder/System`, whose file
//! type is `ZSYS`, not `APPL` — [`blorb::mac_font::from_volume`] already scans
//! every fork for exactly this reason (its own doc: "Searches the `APPL`
//! entries" is about where an INFOCOM RELEASE puts its font, not where a
//! system disk does). This module does the same: every catalog entry with a
//! resource fork is checked, matching `crates/app/examples/font_scout.rs`'s
//! fix for the same gap.

use std::path::{Path, PathBuf};

/// The extension a Kickstart ROM image carries. It is NOT in
/// [`blorb::medium::image_extensions`] and must not be: that table is what the
/// story browser offers to MOUNT, and a ROM is not a volume with a game on it.
/// This module reads it for one thing — the Amiga's own system typeface, which
/// lives in ROM and on no floppy (SQ-1053).
const ROM_EXTENSION: &str = "rom";

/// One typeface found on one of the user's own disks under `~/.lanthorn/`, as a
/// person reads it.
///
/// Mirrors [`crate::native_font::DiskFace`]'s fields (name/width/height/
/// proportional); [`UserFace`] is the same row with the face itself attached,
/// which is what the cascade takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFace {
    /// The disk it came off — the filename directly under `~/.lanthorn/`, so a
    /// face that reads identically off two disks (Workbench 1.2 and 1.3 ship
    /// IDENTICAL font drawers) still names which one answered, rather than
    /// collapsing into a single ambiguous row.
    pub disk: String,
    /// How the medium names it — `FONT 396` on a Macintosh, the filename on an
    /// Amiga volume.
    pub name: String,
    /// The cell it is drawn for.
    pub width: u8,
    pub height: u8,
    /// Whether its advance actually varies — see
    /// [`blorb::bitmap_font::BitmapFont::proportional`].
    pub proportional: bool,
    /// The machine this disk speaks for, from the volume's own filesystem — an
    /// HFS volume is a Macintosh, anything else `blorb` mounts is an Amiga.
    ///
    /// A story is only ever drawn with ITS OWN machine's faces, so a Macintosh
    /// System disk has nothing to say about an Amiga release and vice versa.
    /// Reporting both against either would be the same "present but never used"
    /// confusion SQ-1018 was, one layer out.
    pub machine: crate::interpreter::InterpreterProfile,
}

/// One typeface off a user disk, WITH the face — what the cascade ranks.
///
/// [`SystemFace`] is this projected onto what a reader needs; the glyph bitmaps
/// stay here rather than on the panel's row, because a full System 6 disk parses
/// to eighteen faces and the picker holds one of these per story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFace {
    /// The disk image it came off, named as the filename under the media dir —
    /// so a face that reads identically off two disks still says which answered,
    /// which is the whole reason a System 7 Geneva cannot quietly stand in for a
    /// System 6 one.
    pub disk: String,
    /// How the medium names it — `FONT 396` on a Macintosh, the path on an Amiga
    /// volume.
    pub name: String,
    /// The `FONT`/`NFNT` resource id, on a Macintosh volume only. The FAMILY is
    /// read out of it ([`blorb::mac_font::family_of`]), which is how a request
    /// for Geneva tells 396 from 524.
    pub mac_id: Option<i16>,
    /// The machine this disk speaks for — see [`SystemFace::machine`].
    pub machine: crate::interpreter::InterpreterProfile,
    /// The face itself.
    pub font: blorb::bitmap_font::BitmapFont,
}

impl UserFace {
    /// This row as the info panel reads it.
    pub fn described(&self) -> SystemFace {
        SystemFace {
            disk: self.disk.clone(),
            name: self.name.clone(),
            width: self.font.width,
            height: self.font.height,
            proportional: self.font.proportional,
            machine: self.machine,
        }
    }

    /// Whether this face is the one `want` NAMES — family on a Macintosh, drawer
    /// on an Amiga.
    ///
    /// A name and nothing else. Whether it FITS is `native_font::fit`'s, and
    /// which of a family's sizes the machine drew with is the cascade's, since
    /// only it holds the declared cell.
    fn answers(&self, want: zvm::interpreter::V6SystemFace) -> bool {
        match want {
            zvm::interpreter::V6SystemFace::MacFamily(family) => {
                self.mac_id.is_some_and(|id| blorb::mac_font::family_of(id) == family)
            }
            zvm::interpreter::V6SystemFace::AmigaDrawer(drawer) => {
                blorb::amiga_font::drawer_of(&self.name)
                    .is_some_and(|d| d.eq_ignore_ascii_case(drawer))
            }
        }
    }
}

/// Where the player's own boot disks live, and which one answers first.
///
/// # Several disks COMPOSE; the key only breaks a tie
///
/// The obvious pick-one rules are all bad (SQ-1037): first-found is filesystem
/// order, so the answer changes for no reason a player can see; newest-version
/// needs a version parsed off a filename they may have renamed; most-fonts is
/// arbitrary and wrong the moment one disk has more faces but not the one being
/// asked for. So every disk of the right kind is read and the faces pool, and the
/// REQUEST — family, drawer, size — picks out of the pool.
///
/// `prefer` exists because a pool still has to be ordered when two disks carry
/// the same face, and "whichever the filesystem listed first" is not an answer a
/// person can predict or change. It is `config`'s `system_font_disk`: a substring
/// of the disk's filename, matched case-insensitively, promoted to the front.
/// Nothing is EXCLUDED by it — a preferred disk that lacks the face falls through
/// to the rest, because a naming preference must not be able to lose you a face.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDisks {
    /// The directory to read — [`user_media_dir`] in production, a temp
    /// directory of our own in a test, which is the only way a case here can
    /// pass without depending on what the person running it keeps in
    /// `~/.lanthorn/`.
    pub dir: std::path::PathBuf,
    /// `config`'s `system_font_disk`, or `None` for no preference.
    pub prefer: Option<String>,
}

impl UserDisks {
    /// The player's own media directory, with `config`'s `system_font_disk` as
    /// the preference. An EMPTY key is no preference, which is the default and
    /// what the template documents.
    pub fn new(prefer: &str) -> UserDisks {
        UserDisks {
            dir: user_media_dir(),
            prefer: (!prefer.is_empty()).then(|| prefer.to_string()),
        }
    }
}

/// The faces `machine`'s own system body face NAMES, pooled from every disk in
/// `disks.dir` and ordered so the answer never depends on filesystem order.
///
/// Empty for a machine that names no system face (every row but the Macintosh and
/// the Amiga), for an absent or fontless directory, and for a disk speaking for
/// another machine. The caller decides which of the sizes that come back the
/// machine actually drew with — see [`crate::native_font::resolve`], which holds
/// the declared cell that settles it.
pub fn named_faces_in(
    disks: &UserDisks,
    machine: crate::interpreter::InterpreterProfile,
) -> Vec<UserFace> {
    let Some(want) = machine.v6_system_face() else { return Vec::new() };
    let prefer = disks.prefer.as_deref().map(str::to_ascii_lowercase);
    let mut out: Vec<UserFace> = scan_fonts(&disks.dir)
        .into_iter()
        .filter(|f| f.machine == machine && f.answers(want))
        .collect();
    // Preferred disk first, then by disk name, then in the order the volume's own
    // catalog gave them — a total order over facts a person can see, so two runs
    // on one machine and the same disks cannot disagree.
    out.sort_by_key(|f| {
        let lower = f.disk.to_ascii_lowercase();
        let promoted = !prefer.as_ref().is_some_and(|p| !p.is_empty() && lower.contains(p));
        (promoted, lower)
    });
    out
}

/// Every typeface on every mountable disk image directly inside `dir`.
///
/// Quiet on anything short of a parsed face: an absent `dir`, an empty one, one
/// with files that are not disk images, or a disk image with no font all answer
/// an empty `Vec` rather than an error — a player with no system disks under
/// `~/.lanthorn/` must see no change at all (SQ-1038).
///
/// `dir` is a parameter rather than always [`user_media_dir`] so a test can
/// point this at a temp directory carrying a synthetic fixture instead of the
/// user's own machine, which this module must never depend on for a passing
/// test (`unit_tests/macfont.hfs` is the one committed here).
pub fn scan(dir: &Path) -> Vec<SystemFace> {
    scan_fonts(dir).iter().map(UserFace::described).collect()
}

/// [`scan`] with the faces still attached — the one walk both doors take.
fn scan_fonts(dir: &Path) -> Vec<UserFace> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_image = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case(ROM_EXTENSION)
                    || blorb::medium::image_extensions().any(|known| known.eq_ignore_ascii_case(ext))
            });
        if !is_image {
            continue;
        }
        let Some(disk) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else { continue };
        out.extend(faces_on(&path, &disk, bytes));
    }
    out
}

/// Every face `bytes` (one disk image's contents, at `path` and named `disk`)
/// carries.
fn faces_on(path: &Path, disk: &str, bytes: Vec<u8>) -> Vec<UserFace> {
    // A Macintosh volume: every resource fork on it, not only an `APPL`'s — see
    // the module docs for why (`ZSYS`, the System file, is where a system disk's
    // fonts live).
    if blorb::hfs::Hfs::looks_like_hfs(&bytes) {
        let Ok(hfs) = blorb::hfs::Hfs::mount(bytes) else { return Vec::new() };
        return hfs
            .files()
            .iter()
            .filter(|e| e.resource_size > 0)
            .filter_map(|e| hfs.read_resource(e))
            .filter_map(|fork| blorb::resource_fork::ResourceFork::parse(&fork))
            .flat_map(|rf| blorb::mac_font::faces_in_fork(&rf))
            .map(|(id, f)| UserFace {
                disk: disk.to_string(),
                name: format!("FONT {id}"),
                mac_id: Some(id),
                machine: crate::interpreter::InterpreterProfile::Macintosh,
                font: f,
            })
            .collect();
    }
    // Every other medium blorb can mount: an AmigaDOS disk font is a file, named
    // by one, exactly as `native_font::detected` reads it.
    //
    // Through `cli_host::disk_set::mount_at`, not `blorb::medium::MountedDisk::
    // mount` directly — every front-end mounts a disk through that one seam (see
    // its own docs, and `release_enumeration::no_production_code_mounts_the_
    // platter_alone`), even though a font drawer never spans volumes the way a
    // paged Apple II release does: there is one seam, not one seam plus an
    // exception for callers that believe they don't need it.
    //
    // A KICKSTART ROM is the other Amiga boot medium, and it is NOT a volume — it
    // has no filesystem, so it never reaches the mounter at all (SQ-1053). The
    // machine's real topaz 8 is in there and on no floppy, which is why an Amiga
    // story drew in `vga16` however many Workbench disks a player owned.
    let amiga = |name: String, font: blorb::bitmap_font::BitmapFont| UserFace {
        disk: disk.to_string(),
        name,
        mac_id: None,
        machine: crate::interpreter::InterpreterProfile::Amiga,
        font,
    };
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ROM_EXTENSION)) {
        return blorb::amiga_font::faces_in_rom(&bytes)
            .into_iter()
            .map(|(name, f)| amiga(name, f))
            .collect();
    }
    let Ok(mounted) = crate::disk_set::mount_at(path, bytes) else { return Vec::new() };
    let files = mounted.contents();
    blorb::amiga_font::faces_in_volume(files.iter().map(|(n, b)| (n.as_str(), b.as_slice())))
        .into_iter()
        .map(|(name, f)| amiga(name, f))
        .collect()
}

/// `~/.lanthorn/` — the fixed spot a player drops their own system disks,
/// independent of `--user-dir` or `--data-dir`: those move where LANTHORN's OWN
/// state lives, not where a person's media sits. Same fallback as
/// `config::default_user_dir` (`$HOME`, or `.` when unset), kept as its own tiny
/// copy rather than threading `Config` through the picker's aux resolution just
/// for this.
pub fn user_media_dir() -> PathBuf {
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".lanthorn")
}

/// Every typeface on the user's own disks, off [`user_media_dir`].
pub fn detected() -> Vec<SystemFace> {
    scan(&user_media_dir())
}

/// The faces on the user's own disks that `machine` could actually draw with.
///
/// # Why this filters rather than reporting everything
///
/// A face is only ever a candidate for the machine that owns it: Geneva off a
/// Macintosh System disk says nothing about an Amiga release, and topaz off a
/// Workbench floppy says nothing about a Macintosh one. Listing both against
/// either would put rows in front of a reader that can never apply to the story
/// they are looking at — which is the "present but never used" confusion SQ-1018
/// cost a bug report for, and this panel exists partly to prevent.
///
/// The caller is also responsible for asking only about a **Version 6** story.
/// Nothing below v6 draws text from a disk face at all — v1-v5 text goes through
/// the terminal, so a system disk is irrelevant there whatever machine it names.
pub fn detected_for(machine: crate::interpreter::InterpreterProfile) -> Vec<SystemFace> {
    scan_for(&user_media_dir(), machine)
}

/// [`scan`], keeping only the faces `machine` could draw with. Split out from
/// [`detected_for`] so the filter is testable against a directory of our own
/// rather than whatever the person running the tests keeps in `~/.lanthorn/`.
pub fn scan_for(dir: &Path, machine: crate::interpreter::InterpreterProfile) -> Vec<SystemFace> {
    let mut out = scan(dir);
    out.retain(|f| f.machine == machine);
    out
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    fn macfont_hfs_bytes() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests/macfont.hfs");
        std::fs::read(&path).expect("unit_tests/macfont.hfs is committed and readable")
    }

    /// A Macintosh disk answers a Macintosh story and says nothing to an Amiga
    /// one (SQ-1038).
    ///
    /// The filter matters because a face that can never be drawn is worse than no
    /// row at all: SQ-1018 was reported as crowded text and was really a face
    /// sitting present-and-unused, and a Geneva listed under a Journey floppy
    /// would be the same confusion one layer out.
    #[test]
    fn a_disk_only_answers_its_own_machine() {
        let dir = std::env::temp_dir().join(format!("sq1038-machine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("System.img"), macfont_hfs_bytes()).expect("write");

        let mac = scan_for(&dir, crate::interpreter::InterpreterProfile::Macintosh);
        assert!(!mac.is_empty(), "the Macintosh disk answers a Macintosh story");
        assert!(
            mac.iter().all(|f| f.machine == crate::interpreter::InterpreterProfile::Macintosh),
            "and every row it answers with is its own machine's: {mac:?}",
        );

        // Non-vacuity: the unfiltered scan really does find these, so the empty
        // answer below is the FILTER working and not an unreadable fixture.
        assert_eq!(scan(&dir).len(), mac.len(), "the filter kept everything the scan found");

        for other in [
            crate::interpreter::InterpreterProfile::Amiga,
            crate::interpreter::InterpreterProfile::IbmPc,
        ] {
            assert_eq!(scan_for(&dir, other), Vec::new(), "{other:?} sees nothing on a Mac disk");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An absent directory answers an empty list quietly — no error, no panic —
    /// which is what a player with no `~/.lanthorn/` at all must see.
    #[test]
    fn an_absent_directory_is_quiet() {
        let dir = std::env::temp_dir().join(format!("sq1038-absent-{}", std::process::id()));
        assert!(!dir.exists());
        assert_eq!(scan(&dir), Vec::new());
    }

    /// An existing but empty directory, and one with unrelated files, both
    /// answer empty too — nothing here treats "no fonts found" as an error.
    #[test]
    fn an_empty_or_irrelevant_directory_is_quiet() {
        let dir = std::env::temp_dir().join(format!("sq1038-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("notes.txt"), b"not a disk image").expect("write");
        std::fs::write(dir.join("config.toml"), b"honor_game_colours = true").expect("write");
        assert_eq!(scan(&dir), Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The synthetic fixture parses to its two known faces, named with the disk
    /// they came off — the case this module exists to pass without depending on
    /// the user's own `~/.lanthorn/` (SQ-1038).
    #[test]
    fn a_synthetic_macintosh_disk_reports_its_faces_named_with_the_disk() {
        let dir = std::env::temp_dir().join(format!("sq1038-macfont-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("MyStartup.img"), macfont_hfs_bytes()).expect("write fixture");
        // Not an image extension — must be skipped even though the bytes inside
        // are perfectly good HFS, since the pre-filter is on the name.
        std::fs::write(dir.join("MyStartup.txt"), macfont_hfs_bytes()).expect("write decoy");

        let faces = scan(&dir);
        assert_eq!(faces.len(), 2, "FONT 524 and FONT 1033, and nothing from the .txt decoy: {faces:?}");
        assert!(faces.iter().all(|f| f.disk == "MyStartup.img"), "named with the disk: {faces:?}");
        let body = faces.iter().find(|f| f.name == "FONT 524").expect("FONT 524 is listed");
        assert_eq!((body.width, body.height), (7, 15));
        let alt = faces.iter().find(|f| f.name == "FONT 1033").expect("FONT 1033 is listed");
        assert_eq!((alt.width, alt.height), (7, 12));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two disks with the same face still report as two rows: the exact case
    /// Workbench 1.2 and 1.3 exercise on the user's own machine (identical font
    /// drawers), reproduced here with the one fixture this module may depend on.
    #[test]
    fn duplicate_faces_across_two_disks_both_report() {
        let dir = std::env::temp_dir().join(format!("sq1038-dup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("Disk1.img"), macfont_hfs_bytes()).expect("write");
        std::fs::write(dir.join("Disk2.img"), macfont_hfs_bytes()).expect("write");

        let faces = scan(&dir);
        assert_eq!(faces.len(), 4, "2 faces x 2 disks: {faces:?}");
        let disks: std::collections::BTreeSet<&str> = faces.iter().map(|f| f.disk.as_str()).collect();
        assert_eq!(disks, std::collections::BTreeSet::from(["Disk1.img", "Disk2.img"]));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// [`user_media_dir`] resolves under `$HOME`, matching
    /// `config::default_user_dir`'s own fallback — pinned so the two cannot
    /// silently drift onto different homes.
    #[test]
    fn user_media_dir_is_under_home() {
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(user_media_dir(), PathBuf::from(home).join(".lanthorn"));
        }
    }
}
