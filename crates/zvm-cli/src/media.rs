//! Opening an original release disk image.
//!
//! `blorb` mounts release floppies for the TUI, and `zvm-cli` already links
//! `blorb` — so pointing this front-end at one costs no new dependency and no
//! new parsing, only the wiring in this file (SQ-0834).
//!
//! **Every format `blorb` reads, this front-end opens** (SQ-0840). Nothing here
//! names a filesystem: `blorb::medium` is asked whether the bytes are a disk and
//! what is on it, so an Amiga floppy and a Macintosh volume arrive by the same
//! road and the next format arrives without this file changing. That is the
//! user's rule — *"please keep the functionality consistent for all disk image
//! formats"* — made structural rather than remembered.
//!
//! The one thing a disk needs that a bare story file does not is a **choice**.
//! An original single-game floppy carries one story and opens straight away,
//! but a compilation disk carries several, so a mounted image yields a list and
//! somebody has to pick from it: the player at a prompt, or `--story` when
//! nobody is watching.
//!
//! **And a release is not always one disk** (SQ-0874). This front-end mounted
//! with [`blorb::medium::MountedDisk::mount`] — no companions — so every
//! multi-volume release failed here while the TUI opened it: *Trinity* is a
//! Version 4 story of 262,064 bytes paged across two 174,848-byte Commodore
//! sides, and the Apple II 5.25-inch presses page theirs across four and five
//! floppies, none of which carries a whole story on its own. The mount now goes
//! through [`cli_host::disk_set::mount_at`], which is the seam the TUI uses and
//! the only place the "which files are one release" rule is written down.

use blorb::medium::DiskImage;
use std::path::Path;

/// One story found on a mounted disk image.
pub struct Candidate {
    /// The best name the medium gives this story: the stored filename, prefixed
    /// by its directory on a format that has them (`HITCHHIK/STORY.DAT`).
    /// AmigaDOS release floppies name every story `Story.data` and Atari ST
    /// compilations name every one `STORY.DAT`, so on a flat disk this rarely
    /// distinguishes anything — which is why the menu also shows
    /// [`Candidate::header`], and why [`Candidate::title`] prefers a real game
    /// name when the build has one.
    pub name: String,
    /// The story bytes, read off the image.
    pub bytes: Vec<u8>,
    /// The medium THIS story came off, which on a hybrid disc is not the image's
    /// own format (SQ-0930).
    ///
    /// `Classic Text Adventure Masterpieces` mounts as HFS and carries both
    /// machines' builds; `LostTreasures1.iso` mounts as ISO 9660 and does the
    /// same. Reading the IMAGE's format told every PC story on the first that it
    /// was a Macintosh, and every PC story on the second that it was nothing.
    /// The TUI has resolved this per story since SQ-0876; this is the CLI
    /// catching up.
    pub image: Option<blorb::medium::DiskImage>,
}

impl Candidate {
    /// The Z-machine version, release and serial, e.g. `v3 r88 s840726`.
    ///
    /// `None` when the bytes are not Z-code (a Blorb or Glulx image on a disk
    /// would be), because then there is no header to read. This is what
    /// actually tells two candidates apart: the corpus holds three different
    /// releases of *Hitchhiker's* alone (v3 r56 s841221, v3 r58 s851002,
    /// v5 r31 s871119).
    ///
    /// **Bit 7 comes off each serial byte**, exactly as
    /// `blorb::adf::looks_like_story` and `cli_host::storage::DiskBuild` mask it
    /// (SQ-0856), so a story `blorb` offers is a story this menu can label:
    /// `LEATHRGODDESSES` on *Lost Treasures* `INFOCOM6` writes its serial in the
    /// Apple II's high ASCII and reads `Blown!` with the bit off.
    pub fn header(&self) -> Option<String> {
        let (v, release, serial) = self.build()?;
        Some(format!("v{v} r{release} s{serial}"))
    }

    /// The Z-machine version, release and serial as the header spells them —
    /// what [`Candidate::header`] prints and what [`Candidate::title`] looks up.
    ///
    /// One reader for both, because they must agree about which bytes are a
    /// header at all: a menu row that says `v3 r88 s840726` and then declines to
    /// name the build, or names one the header does not describe, is worse than
    /// either alone.
    fn build(&self) -> Option<(u8, u16, String)> {
        let b = &self.bytes;
        // Every published Z-machine version, 1 to 8 (ZMSD §11.1's byte 0).
        if b.len() < 0x18 || !(1..=8).contains(&b[0]) {
            return None;
        }
        let serial: String = b[0x12..0x18].iter().map(|c| char::from(c & 0x7f)).collect();
        if !serial.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
            return None;
        }
        Some((b[0], u16::from_be_bytes([b[0x02], b[0x03]]), serial))
    }

    /// The canonical title `cli_host`'s bundled `known_titles.tsv` gives this
    /// build, when it carries one (SQ-0884).
    ///
    /// **Keyed by the build, never by the filename**, which is the whole reason
    /// this is worth doing here: a release disk's names are `Story.data`,
    /// `STORY.DAT` and `PC/DATA/BEYONDZO.DAT`, and none of them is a title. The
    /// release and serial in the header are, and they are the same key
    /// `app::picker` names the story browser's rows with and
    /// `cli_host::storage` builds the per-game save directory from — so a game
    /// opened off a disc reads the same in all three places without a second
    /// table.
    ///
    /// `None` for a build the table does not carry, which is honest rather than
    /// unfortunate: the fallback is the name the medium stored, and a missing
    /// row costs a filename while a wrong one mislabels a game.
    pub fn title(&self) -> Option<&'static str> {
        let (_, release, serial) = self.build()?;
        cli_host::titles::title_for_build(release, &serial)
    }

    /// The name this candidate goes by: its canonical title when the table knows
    /// the build, and otherwise the name the medium stored.
    pub fn display_name(&self) -> &str {
        self.title().unwrap_or(&self.name)
    }

    /// How this candidate reads in the menu: the name it goes by, its header
    /// when there is one, and — when the title replaced it — the stored name it
    /// came off the disc under.
    ///
    /// The stored name is kept rather than dropped because the title alone does
    /// not tell two rows apart: *Masterpieces* carries *Ballyhoo* three times as
    /// `MAC/BALLYHOO`, `PC/BALLYHOO/DATA/BALLYHOO.DAT` and `PC/DATA/BALLYHOO.DAT`
    /// — one build, three files — and a menu of three identical lines is not a
    /// choice anybody can make.
    pub fn label(&self) -> String {
        let mut s = self.display_name().to_string();
        if let Some(h) = self.header() {
            s.push_str(&format!("  ({h})"));
        }
        if self.title().is_some() {
            s.push_str(&format!("  {}", self.name));
        }
        s
    }
}

/// Are these bytes a disk image this front-end can mount?
///
/// Content, not extension — exactly as the TUI asks the question, of exactly the
/// same recogniser, so an image with any name is recognised and a mis-named
/// story file is not.
///
/// It used to be **narrower** than [`blorb::medium::DiskImage::detect`], pinned
/// to `Adf` alone with a comment explaining that this front-end must not claim
/// an image it cannot open — an honest guard around a real hole, since
/// `story_candidates` had no HFS arm and a Macintosh disk would have detected
/// and then failed. The hole is gone: detect and mount now walk one table
/// (SQ-0840), so whatever `blorb` recognises, `blorb` opens, and the guard has
/// nothing left to guard.
pub fn looks_like_image(raw: &[u8]) -> bool {
    DiskImage::detect(raw).is_some()
}

/// Mount the image at `path`, whose bytes are `raw`, and return every story the
/// **release** offers — including the one it keeps on no single volume
/// (SQ-0874) and the ones its other volumes carry whole (SQ-0961).
///
/// Identified by content: a release disk's filenames prove nothing — AmigaDOS
/// has no extensions and every Atari ST story is called `STORY.DAT` — so
/// `blorb` decides by the bytes, strictly enough to reject the saved games the
/// original *Zork Zero* floppy carries.
///
/// `path` is here for the set and nothing else. It is a **name**, so it decides
/// which files are siblings and never what is on them.
///
/// **How far to look is not this file's decision** (SQ-0961). It used to be, by
/// omission: the mount answered for one platter and the menu showed what the
/// mount had, so `zvm-cli` on *Lost Treasures* disk 1 offered six games where
/// lanthorn offered thirty. Two front-ends with two ideas of what a release is
/// disagree eventually — the argument `cli_host::disk_set`'s module doc makes
/// about the mount, now made about the enumeration too, and by the same seam.
pub fn story_candidates(path: &Path, raw: Vec<u8>) -> Result<Vec<Candidate>, String> {
    let disk = cli_host::disk_set::mount_at(path, raw)
        .map_err(|e| format!("Error: cannot mount the disk image: {e}"))?;
    let mounted = disk.file_count();
    let found: Vec<Candidate> = cli_host::disk_set::stories_across_the_release(path, &disk)
        .into_iter()
        // SQ-0930: `image` is the story's OWN half of the platter, not the
        // image's format — the same call `app::hints::read_story_file` has made
        // since SQ-0876, and its absence here is why a PC build on the
        // Masterpieces disc reported the Macintosh.
        .map(|r| Candidate { name: r.name, bytes: r.bytes, image: Some(r.image) })
        .collect();
    if found.is_empty() {
        let files = if mounted == 1 { "file" } else { "files" };
        // Formats that keep a volume name say it; the ones that do not read the
        // same as they always did.
        let named = disk.volume_name().map(|n| format!(" on {n}")).unwrap_or_default();
        return Err(format!(
            "Error: no story file on this disk image \
             ({mounted} {files} mounted{named}; is this the boot disk?)"
        ));
    }
    Ok(found)
}

/// These candidates as the shared chooser sees them (SQ-1078).
///
/// The rule `--story` matches by lives in `cli_host::story_pick`, because
/// lanthorn offers the same flag and the two must not drift; this is the only
/// thing that is `zvm-cli`'s — which names a row shows.
fn rows(cands: &[Candidate]) -> Vec<cli_host::story_pick::Row> {
    cands
        .iter()
        .map(|c| cli_host::story_pick::Row {
            name: c.name.clone(),
            title: c.title().map(str::to_string),
            label: c.label(),
        })
        .collect()
}

/// The numbered list a player picks from.
pub fn menu(cands: &[Candidate]) -> String {
    cli_host::story_pick::menu(&rows(cands))
}

/// Resolve what `--story` asked for: a 1-based number, or a name to match
/// (case-insensitive, and a substring is enough as long as it picks out one
/// story).
pub fn find(cands: &[Candidate], want: &str) -> Result<usize, String> {
    cli_host::story_pick::find(&rows(cands), want, "this disk")
}

/// Pick a story off a mounted image.
///
/// One candidate opens without asking, whatever else is going on — the common
/// single-game floppy never sees a menu. `--story` decides it outright.
/// Otherwise the choice needs a person: with a terminal on stdin the menu is
/// printed and `read_line` answers it; without one this **refuses to block**
/// and says which flag to pass, because a prompt nobody can answer is a hang
/// (and would make this untestable and unscriptable).
///
/// `read_line` returns `None` at end of input. `announce` receives the menu and
/// the prompt, so the caller decides where they are printed and a test can keep
/// them.
pub fn choose(
    cands: &[Candidate],
    want: Option<&str>,
    stdin_is_tty: bool,
    mut announce: impl FnMut(&str),
    mut read_line: impl FnMut() -> Option<String>,
) -> Result<usize, String> {
    if let Some(w) = want {
        return find(cands, w);
    }
    if cands.len() == 1 {
        return Ok(0);
    }
    if !stdin_is_tty {
        return Err(format!(
            "Error: this disk image holds {} stories; pass --story <n|name> to pick one:\n{}",
            cands.len(),
            menu(cands)
        ));
    }
    announce(&format!("This disk holds {} stories:\n{}", cands.len(), menu(cands)));
    loop {
        announce(&format!("Which one? [1-{}] ", cands.len()));
        let Some(line) = read_line() else {
            return Err("Error: no story chosen.".to_string());
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match find(cands, line) {
            Ok(i) => return Ok(i),
            Err(e) => announce(&format!("{e}\n")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare, structurally valid HFS volume with an empty catalog: the
    /// cheapest thing that is unmistakably a Macintosh disk and not an
    /// AmigaDOS one. Signature `BD` a kilobyte in, then a Master Directory
    /// Block whose geometry describes the volume it sits in.
    fn macintosh_volume() -> Vec<u8> {
        let mut v = vec![0u8; 1600 * 512];
        let mdb = 2 * 512;
        v[mdb..mdb + 2].copy_from_slice(&0x4244u16.to_be_bytes()); // drSigWord
        v[mdb + 18..mdb + 20].copy_from_slice(&1596u16.to_be_bytes()); // drNmAlBlks
        v[mdb + 20..mdb + 24].copy_from_slice(&512u32.to_be_bytes()); // drAlBlkSiz
        v[mdb + 28..mdb + 30].copy_from_slice(&4u16.to_be_bytes()); // drAlBlSt
        v
    }

    /// An AmigaDOS floppy, boot block and all.
    fn amiga_floppy() -> Vec<u8> {
        let mut v = vec![0u8; 1760 * 512];
        v[0..3].copy_from_slice(b"DOS");
        v
    }

    /// **The rule, as a test**: whatever `blorb` recognises, this front-end
    /// claims — for every format, with no exceptions carved out here.
    ///
    /// FALSIFICATION: narrow `looks_like_image` back to `== Some(DiskImage::Adf)`
    /// and this fails on the Macintosh volume, which is exactly the reported
    /// bug — `zvm-cli` opened an Amiga floppy and refused a Mac disk that
    /// `blorb` had been able to read for a month.
    #[test]
    fn this_front_end_claims_every_disk_blorb_can_open() {
        for raw in [amiga_floppy(), macintosh_volume()] {
            let detected = DiskImage::detect(&raw).expect("a disk image");
            assert!(
                looks_like_image(&raw),
                "blorb detects {detected:?} but this front-end will not claim it"
            );
            // …and claiming it means being able to open it, not merely to name
            // it: a detector that claims a disk and then fails is worse than one
            // that declines it (SQ-0840).
            let mounted =
                blorb::medium::MountedDisk::mount(raw).expect("what we claim, we can mount");
            assert_eq!(mounted.format(), detected);
        }
    }

    /// **The menu is the release's, not the platter's** (SQ-0961), on both
    /// presses of *The Lost Treasures of Infocom* in `treasures/`.
    ///
    /// Six volumes on the Amiga press and five on the Macintosh DiskCopy one;
    /// twenty games either way, and the DiskCopy disk 1 offers twenty-two
    /// candidates because it stores *The Lurking Horror* three times over. The
    /// figure asserted is therefore the count of distinct BUILDS, which is the
    /// count of games. Measured 2026-08-21.
    ///
    /// `treasures/` is gitignored, so an absent fixture is a skip; the guard
    /// below is what keeps that from being a silent pass.
    ///
    /// FALSIFICATION: build `found` from `disk.stories()` again and the Amiga
    /// press offers six games and the Macintosh one four — the reported symptom
    /// exactly, against the browser's twenty.
    #[test]
    fn the_menu_lists_the_whole_release() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../treasures");
        let presses = [
            "Lost Treasures of Infocom, The_Disk3.adf",
            "The Lost Treasures of Infocom - Disk 1 - Beyond Zork, Lurking Horror.dc42",
        ];
        let mut ran = 0;
        for name in presses {
            let path = dir.join(name);
            let Ok(raw) = std::fs::read(&path) else { continue };
            ran += 1;
            let cands = story_candidates(&path, raw).expect("the press mounts");
            let mut builds: Vec<(u8, u16, String)> =
                cands.iter().filter_map(Candidate::build).collect();
            builds.sort();
            builds.dedup();
            assert_eq!(builds.len(), 20, "{name}: {} games", builds.len());
        }
        assert!(
            ran > 0 || !presses.iter().any(|n| dir.join(n).exists()),
            "a press is present but no menu was built",
        );
    }

    /// The other half: an ordinary story file is not a disk, and is not claimed.
    #[test]
    fn a_plain_story_file_is_not_claimed_as_a_disk() {
        assert!(!looks_like_image(&story(3, 88, "840726")));
        assert!(!looks_like_image(b"FORM\x00\x00\x00\x04IFRS"));
        assert!(!looks_like_image(&[]));
    }

    /// A story buffer whose header carries a given version, release and serial.
    /// Only the header is read here, so the rest can stay zero.
    fn story(v: u8, release: u16, serial: &str) -> Vec<u8> {
        let mut b = vec![0u8; 0x100];
        b[0x00] = v;
        b[0x02..0x04].copy_from_slice(&release.to_be_bytes());
        b[0x12..0x18].copy_from_slice(serial.as_bytes());
        b
    }

    fn cand(name: &str, v: u8, release: u16, serial: &str) -> Candidate {
        Candidate { name: name.to_string(), bytes: story(v, release, serial), image: None }
    }

    /// **The lookup** (SQ-0884). The whole point of showing the header was that
    /// an Amiga floppy calls every story `Story.data` and an ST compilation
    /// calls every one `STORY.DAT`, so the name alone makes a menu of identical
    /// lines — but the header is an identity, not a name. `known_titles.tsv`
    /// turns it into one, and the stored name stays on the row so two copies of
    /// one build are still told apart.
    ///
    /// FALSIFICATION: make `label` build from `self.name` again (or `title`
    /// return `None`) and this fails with exactly the reported symptom —
    /// `Story.data  (v3 r88 s840726)`, a menu row that will not say it is
    /// *Zork I*.
    #[test]
    fn a_candidate_is_labelled_by_its_canonical_title() {
        let c = cand("Story.data", 3, 88, "840726");
        assert_eq!(c.header().as_deref(), Some("v3 r88 s840726"));
        assert_eq!(c.title(), Some("Zork I: The Great Underground Empire"));
        assert_eq!(c.label(), "Zork I: The Great Underground Empire  (v3 r88 s840726)  Story.data");
    }

    /// The key is the **build**, never the filename — which is what makes this
    /// worth doing at all. `PC/DATA/BEYONDZO.DAT` off *Lost Treasures I* names
    /// nothing; its release and serial name the game exactly.
    #[test]
    fn the_title_comes_from_the_build_and_not_from_the_name() {
        let c = cand("PC/DATA/BEYONDZO.DAT", 5, 57, "871221");
        assert_eq!(c.title(), Some("Beyond Zork: The Coconut of Quendor"));
        // …and the same build under any other name resolves the same way.
        assert_eq!(cand("MAC/BEYOND ZORK", 5, 57, "871221").title(), c.title());
        // A different build of the same game is a different row, correctly named.
        assert_eq!(cand("STORY.DAT", 5, 51, "870923").title(), c.title());
    }

    /// A build the table does not carry falls back to the name the medium
    /// stored, and says so by carrying no title at all. A missing row costs a
    /// filename; a wrong one mislabels a game.
    #[test]
    fn an_unknown_build_keeps_the_name_the_medium_stored() {
        let c = cand("STORY.DAT", 3, 999, "010203");
        assert_eq!(c.title(), None);
        assert_eq!(c.display_name(), "STORY.DAT");
        assert_eq!(c.label(), "STORY.DAT  (v3 r999 s010203)");
    }

    /// **One build, three files** — *Masterpieces* carries *Ballyhoo* as
    /// `MAC/BALLYHOO`, `PC/BALLYHOO/DATA/BALLYHOO.DAT` and
    /// `PC/DATA/BALLYHOO.DAT`. Naming them all *Ballyhoo* and stopping there
    /// would print three identical rows, which is not a choice anybody can make,
    /// so the stored name stays.
    #[test]
    fn duplicate_builds_are_still_told_apart_by_their_stored_names() {
        let files = ["MAC/BALLYHOO", "PC/BALLYHOO/DATA/BALLYHOO.DAT", "PC/DATA/BALLYHOO.DAT"];
        let cands: Vec<Candidate> = files.iter().map(|n| cand(n, 3, 97, "851218")).collect();
        let mut labels: Vec<String> = cands.iter().map(|c| c.label()).collect();
        for (label, name) in labels.iter().zip(files) {
            assert!(label.starts_with("Ballyhoo  (v3 r97 s851218)  "), "{label}");
            assert!(label.ends_with(name), "{label}");
        }
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 3, "three files must not read as one row");
    }

    /// A serial in the Apple II's high ASCII labels as the text it is, so the
    /// game `blorb` newly offers off `INFOCOM6` is not the one row in the menu
    /// with no identity on it (SQ-0856).
    #[test]
    fn a_high_ascii_serial_labels_as_the_text_it_is() {
        let mut bytes = story(3, 0, "......");
        bytes[0x12..0x18].copy_from_slice(&[0xc2, 0xec, 0xef, 0xf7, 0xee, 0xa1]);
        let c = Candidate { name: "LEATHRGODDESSES".into(), bytes, image: None };
        assert_eq!(c.header().as_deref(), Some("v3 r0 sBlown!"));
        assert_eq!(c.label(), "LEATHRGODDESSES  (v3 r0 sBlown!)");
    }

    /// Nothing to read a header out of (a Blorb, say) still gets a usable line.
    #[test]
    fn a_candidate_without_a_readable_header_is_labelled_by_name_alone() {
        let c = Candidate { name: "game.blb".into(), bytes: b"FORM....IFRS".to_vec(), image: None };
        assert_eq!(c.header(), None);
        assert_eq!(c.label(), "game.blb");
    }

    #[test]
    fn one_story_opens_without_asking() {
        let cands = vec![cand("Story.data", 3, 88, "840726")];
        let mut asked = false;
        let i = choose(&cands, None, true, |_| asked = true, || panic!("must not read stdin"));
        assert_eq!(i, Ok(0));
        assert!(!asked, "a single-story disk asks nothing");
    }

    #[test]
    fn several_stories_are_listed_and_the_answer_selects_one() {
        let cands = vec![
            cand("Story.data", 3, 56, "841221"),
            cand("Story.data", 5, 31, "871119"),
            cand("Hints.data", 3, 22, "870918"),
        ];
        let mut said = String::new();
        let mut lines = ["2\n".to_string()].into_iter();
        let i = choose(&cands, None, true, |s| said.push_str(s), || lines.next());
        assert_eq!(i, Ok(1));
        // Two builds of one game, told apart by their headers and named by the
        // table; the third is a build the table does not carry.
        let hhgg = "The Hitchhiker's Guide to the Galaxy";
        let one = format!("1) {hhgg}  (v3 r56 s841221)  Story.data");
        let two = format!("2) {hhgg}  (v5 r31 s871119)  Story.data");
        assert!(said.contains(&one), "menu shown:\n{said}");
        assert!(said.contains(&two), "menu shown:\n{said}");
        assert!(said.contains("3) Hints.data  (v3 r22 s870918)"), "menu shown:\n{said}");
        assert!(said.contains("Which one? [1-3]"), "prompt shown:\n{said}");
    }

    /// A fat-fingered answer re-asks rather than giving up or opening the wrong
    /// game.
    #[test]
    fn a_bad_answer_is_asked_again() {
        let cands = vec![cand("Story.data", 3, 56, "841221"), cand("Other.data", 5, 31, "871119")];
        let mut said = String::new();
        let mut lines = ["9\n".to_string(), "other\n".to_string()].into_iter();
        let i = choose(&cands, None, true, |s| said.push_str(s), || lines.next());
        assert_eq!(i, Ok(1));
        assert!(said.contains("no story 9 on this disk"), "told what was wrong:\n{said}");
    }

    /// The requirement that makes this scriptable at all: no terminal means no
    /// prompt, ever.
    #[test]
    fn without_a_terminal_the_menu_refuses_to_block() {
        let cands = vec![cand("Story.data", 3, 56, "841221"), cand("Other.data", 5, 31, "871119")];
        let e = choose(&cands, None, false, |_| {}, || panic!("must not read stdin"))
            .expect_err("no terminal, so no prompt");
        assert!(e.contains("--story <n|name>"), "names the flag:\n{e}");
        assert!(e.contains("1) The Hitchhiker's Guide to the Galaxy"), "lists what it found:\n{e}");
    }

    #[test]
    fn story_picks_by_number_or_by_name() {
        let cands =
            vec![cand("Story.data", 3, 56, "841221"), cand("PLANETFA.DAT", 3, 37, "851003")];
        let never = || -> Option<String> { panic!("must not read stdin") };
        assert_eq!(choose(&cands, Some("1"), false, |_| {}, never), Ok(0));
        assert_eq!(choose(&cands, Some("planetfa"), false, |_| {}, never), Ok(1));
        assert!(find(&cands, "3").is_err(), "out of range");
        assert!(find(&cands, "zork").is_err(), "no such name");
        assert!(find(&cands, ".dat").is_err_and(|e| e.contains("more than one")), "ambiguous");
    }

    /// **The menu is the contract for `--story`** (SQ-0884): a row that reads
    /// *Zork I: The Great Underground Empire* must answer to that, or naming the
    /// table's titles made the front-end less usable rather than more.
    #[test]
    fn story_picks_by_the_title_the_menu_shows() {
        let cands =
            vec![cand("PC/DATA/ZORK1.DAT", 3, 88, "840726"), cand("Hints.data", 3, 22, "870918")];
        let never = || -> Option<String> { panic!("must not read stdin") };
        // The title, which appears nowhere in the stored name…
        assert_eq!(choose(&cands, Some("great underground"), false, |_| {}, never), Ok(0));
        assert_eq!(find(&cands, "Zork I"), Ok(0));
        // …and the stored name, which still works exactly as it did.
        assert_eq!(find(&cands, "zork1.dat"), Ok(0));
        assert_eq!(find(&cands, "hints"), Ok(1));
    }

    /// Two builds of one game share a title, so the title alone is ambiguous —
    /// which is the truth, and is reported as such rather than guessed at. The
    /// header on every row is what a person disambiguates with, plus the number.
    #[test]
    fn a_title_carried_by_two_builds_is_ambiguous_not_guessed() {
        let cands =
            vec![cand("Story.data", 3, 56, "841221"), cand("HITCHHIK.DAT", 5, 31, "871119")];
        assert!(
            find(&cands, "hitchhiker").is_err_and(|e| e.contains("more than one")),
            "two builds of Hitchhiker's are two answers"
        );
        assert_eq!(find(&cands, "2"), Ok(1), "the number always decides");
    }
}
