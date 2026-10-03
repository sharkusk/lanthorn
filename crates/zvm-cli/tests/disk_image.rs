//! End-to-end: point the compiled `zvm-cli` at a disk image and let it mount
//! the story off it (SQ-0834).
//!
//! Two kinds of disk here. The **real** ones come out of the gitignored
//! `stories/` tree — the only proof that the mount reads genuine media, and
//! vacuously skipped when a fixture is absent. The **synthetic** ones are built
//! here, because every Amiga floppy in the corpus carries exactly one story and
//! the menu only appears when a disk carries several.
//!
//! Since SQ-0833/SQ-0835 the menu has real disks to stand on: a DOS compilation
//! carrying six games and an Atari ST one carrying four, where the ST's four are
//! all called `STORY.DAT` and only the directory tells them apart.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

// ── running the binary ────────────────────────────────────────────────────────

/// Run `zvm-cli <image> [args]` with `stdin_script` piped in — so stdin is
/// never a terminal, which is the non-interactive path under test.
fn run(image: &std::path::Path, extra_args: &[&str], stdin_script: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zvm-cli"))
        .arg(image)
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("zvm-cli spawns");
    child.stdin.take().unwrap().write_all(stdin_script.as_bytes()).unwrap();
    child.wait_with_output().expect("zvm-cli runs")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ── synthetic media ───────────────────────────────────────────────────────────

/// AmigaDOS block size.
const BSIZE: usize = 512;
/// Blocks on a real 880 KB DD floppy.
const DD_BLOCKS: usize = 1760;

/// A minimal AmigaDOS (FFS) disk image builder. Only what a file needs: a
/// bootblock, and a header block per file with its data blocks listed in the
/// reverse-order table at `BSIZE-204`. Files here are small enough never to
/// need an extension block.
struct Floppy {
    image: Vec<u8>,
    next: usize,
}

impl Floppy {
    fn new() -> Floppy {
        let mut image = vec![0u8; DD_BLOCKS * BSIZE];
        image[0..3].copy_from_slice(b"DOS");
        image[3] = 1; // FFS: data blocks are raw 512-byte payload
        Floppy { image, next: 881 } // files start after the root block, as on a real disk
    }

    fn put32(&mut self, block: usize, off: usize, v: u32) {
        let at = block * BSIZE + off;
        self.image[at..at + 4].copy_from_slice(&v.to_be_bytes());
    }

    fn add_file(&mut self, name: &str, data: &[u8]) -> &mut Floppy {
        let header = self.next;
        self.next += 1;
        self.put32(header, 0, 2); // T_HEADER
        self.put32(header, 4, header as u32); // a header names its own block
        self.put32(header, BSIZE - 4, 0xFFFF_FFFD); // ST_FILE
        self.put32(header, BSIZE - 188, data.len() as u32);
        let at = header * BSIZE + BSIZE - 80;
        self.image[at] = name.len() as u8;
        self.image[at + 1..at + 1 + name.len()].copy_from_slice(name.as_bytes());

        for (i, chunk) in data.chunks(BSIZE).enumerate() {
            let db = self.next;
            self.next += 1;
            let at = db * BSIZE;
            self.image[at..at + chunk.len()].copy_from_slice(chunk);
            self.put32(header, BSIZE - 204 - 4 * i, db as u32);
        }
        self.put32(header, 8, data.len().div_ceil(BSIZE) as u32); // high_seq
        self
    }

    /// Write the image beside the test binary and return its path.
    fn write(&self, name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NTH: AtomicUsize = AtomicUsize::new(0);
        let nth = NTH.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("zvm-cli-adf-{}-{name}-{nth}", std::process::id()));
        std::fs::write(&path, &self.image).expect("temp image written");
        path
    }
}

/// A story that quits the moment it starts: enough header for `looks_like_story`
/// to recognise it and for `zvm` to boot it, and `@quit` at the initial PC so
/// the run ends without waiting for input.
fn quitting_story(version: u8, release: u16, serial: &str) -> Vec<u8> {
    let mut b = vec![0u8; 0x400];
    b[0x00] = version;
    b[0x02..0x04].copy_from_slice(&release.to_be_bytes());
    let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
    word(0x04, 0x0400); // high memory
    word(0x06, 0x0040); // initial PC
    word(0x08, 0x0300); // dictionary
    word(0x0a, 0x0100); // objects
    word(0x0c, 0x0200); // globals
    word(0x0e, 0x0280); // static memory base
    word(0x18, 0x0060); // abbreviations
    word(0x1a, (0x400 / if version == 3 { 2 } else { 4 }) as u16); // declared length
    b[0x12..0x18].copy_from_slice(serial.as_bytes());
    b[0x40] = 0xBA; // 0OP:0x0A quit
    b
}

/// A story that prints header byte `$1E` — the interpreter number the host told
/// it it is running on — between brackets, then quits (SQ-0839).
///
/// Hand-assembled at the initial PC, which for v1–v5 is a plain byte address
/// rather than a routine: `print_char '['` (VAR:0x05), `loadb 0 $1E -> sp`
/// (2OP:0x10, long form, both operands small constants), `print_num sp`
/// (VAR:0x06, operand on the stack), `print_char ']'`, `quit`. The brackets are
/// what make the assertion safe — a bare digit would match the score line.
fn interpreter_reporting_story() -> Vec<u8> {
    let mut b = quitting_story(3, 88, "840726");
    b[0x40..0x4E].copy_from_slice(&[
        0xE5, 0x7F, 0x5B, // print_char '['
        0x10, 0x00, 0x1E, 0x00, // loadb 0 $1E -> sp
        0xE6, 0xBF, 0x00, // print_num sp
        0xE5, 0x7F, 0x5D, // print_char ']'
        0xBA, // quit
    ]);
    b
}

/// Two stories on one disk, named and released differently so the menu has
/// something to tell apart.
fn two_story_floppy(name: &str) -> PathBuf {
    let mut f = Floppy::new();
    f.add_file("Zork1.Data", &quitting_story(3, 88, "840726"));
    f.add_file("Zork2.Data", &quitting_story(5, 48, "840904"));
    f.write(name)
}

// ── real media ────────────────────────────────────────────────────────────────

/// The gitignored `stories/` tree, two levels up from this crate.
fn story_path(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join("stories").join(name);
    if p.exists() {
        return Some(p);
    }
    eprintln!("SKIP: gitignored disk image missing at {}", p.display());
    None
}

/// The original Amiga *Zork I* floppy carries one story (`Zork1.Data`, v3 r88
/// s840726) — so it opens straight into the game with nothing asked, and the
/// game really is running: "West of House" is Zork's opening room.
#[test]
fn a_single_story_floppy_opens_without_asking() {
    let Some(image) = story_path("Zork I - The Great Underground Empire.adf") else { return };
    let out = run(&image, &[], "quit\ny\n");
    let text = stdout_of(&out);
    assert!(text.contains("West of House"), "the story mounted off the floppy ran:\n{text}");
    assert!(!text.contains("Which one?"), "one story is not a choice:\n{text}");
    assert!(!stderr_of(&out).contains("--story"), "nothing to pick:\n{}", stderr_of(&out));
}

// ── every format, not just the Amiga's (SQ-0840) ──────────────────────────────

/// A bare, structurally valid HFS volume with an empty catalog — unmistakably a
/// Macintosh disk, and nothing like an AmigaDOS one. Signature `BD` a kilobyte
/// in, then a Master Directory Block whose geometry describes the volume it sits
/// in. No fixture needed, so this never skips.
fn macintosh_volume() -> Vec<u8> {
    let mut v = vec![0u8; 1600 * BSIZE];
    let mdb = 2 * BSIZE;
    v[mdb..mdb + 2].copy_from_slice(&0x4244u16.to_be_bytes()); // drSigWord
    v[mdb + 18..mdb + 20].copy_from_slice(&1596u16.to_be_bytes()); // drNmAlBlks
    v[mdb + 20..mdb + 24].copy_from_slice(&512u32.to_be_bytes()); // drAlBlkSiz
    v[mdb + 28..mdb + 30].copy_from_slice(&4u16.to_be_bytes()); // drAlBlSt
    v
}

/// **The bug this quest was filed for.** `zvm-cli` refused a Macintosh disk
/// outright — `blorb` had read that filesystem since SQ-0837, but this
/// front-end's detector was pinned to `Adf`, so the raw image went to the VM as
/// if it were a story file.
///
/// A Mac volume must now be *mounted* like any other disk, and the message must
/// be about what is on the disk. This one has nothing on it, so that message is
/// the boot-disk one.
///
/// FALSIFICATION: restore `looks_like_image` to `== Some(DiskImage::Adf)` and
/// this fails with the symptom as reported —
/// `Error: Z-machine version 0 is not supported.` — byte 0 of an unmounted
/// volume, read as a story header.
#[test]
fn a_macintosh_volume_is_mounted_like_any_other_disk() {
    let image = write_temp("mac-empty.image", &macintosh_volume());
    let out = run(&image, &[], "");
    assert!(!out.status.success(), "an empty volume holds no game");
    let err = stderr_of(&out);
    assert!(err.contains("no story file on this disk image"), "mounted, and said so:\n{err}");
    assert!(!err.contains("is not supported"), "never handed to the VM as a story:\n{err}");
    let _ = std::fs::remove_file(&image);
}

/// The real Macintosh release disk: *Zork Zero* release 296 / serial 881019, an
/// HFS volume inside a DiskCopy 4.2 wrapper. It mounts, and is then declined at
/// load because `zvm-cli` renders no v6 graphics — which is this front-end's own
/// limitation and the same answer the five graphical Amiga floppies get. **The
/// user must see the v6 refusal, not a disk error**; that is what says the mount
/// worked.
///
/// FALSIFICATION: with `looks_like_image` narrowed back to `Adf`, this reads
/// `Error: Z-machine version 14 is not supported.` — 14 being the Pascal length
/// of the DiskCopy header's disk name, `Zork Zero Disk`.
#[test]
fn the_real_macintosh_release_disk_mounts_and_is_declined_as_v6() {
    let Some(image) = story_path("Zork Zero Disk.image") else { return };
    let out = run(&image, &[], "");
    let err = stderr_of(&out);
    assert!(err.contains("v6 graphical games are not supported"), "the v6 refusal:\n{err}");
    assert!(!err.contains("version"), "not a version-number complaint about the image:\n{err}");
    assert!(!err.contains("disk image"), "not a disk error — the disk mounted fine:\n{err}");
}

/// The same disk on the other medium, for contrast: the Amiga *Zork Zero* is
/// also v6 and gets the identical refusal. Two formats, one behaviour — which is
/// the whole of the user's rule.
#[test]
fn an_amiga_v6_floppy_is_declined_exactly_as_the_macintosh_one_is() {
    let Some(amiga) = story_path("Zork Zero - The Revenge of Megaboz.adf") else { return };
    let Some(mac) = story_path("Zork Zero Disk.image") else { return };
    assert_eq!(
        stderr_of(&run(&amiga, &[], "")),
        stderr_of(&run(&mac, &[], "")),
        "the medium changes nothing about how the CLI declines a v6 game"
    );
}

// ── the menu ──────────────────────────────────────────────────────────────────

/// Nobody at the keyboard and more than one story: list what is there, name the
/// flag, and stop. A prompt here would hang every script and every test.
#[test]
fn several_stories_and_no_terminal_lists_them_and_refuses_to_block() {
    let image = two_story_floppy("list.adf");
    let out = run(&image, &[], "");
    assert!(!out.status.success(), "no story was chosen, so this cannot succeed");
    let err = stderr_of(&out);
    assert!(err.contains("holds 2 stories"), "says what it found:\n{err}");
    assert!(err.contains("--story <n|name>"), "names the flag to pass:\n{err}");
    let one = "1) Zork I: The Great Underground Empire  (v3 r88 s840726)  Zork1.Data";
    let two = "2) Zork II: The Wizard of Frobozz  (v5 r48 s840904)  Zork2.Data";
    assert!(err.contains(one), "labels the first:\n{err}");
    assert!(err.contains(two), "labels the second:\n{err}");
    let _ = std::fs::remove_file(&image);
}

/// `--story <n>` picks by menu number, and the chosen story is the one that
/// boots — the run reaches `@quit` and exits cleanly.
#[test]
fn story_number_picks_one_off_the_disk() {
    let image = two_story_floppy("number.adf");
    let out = run(&image, &["--story", "2"], "");
    let text = stdout_of(&out);
    assert!(out.status.success(), "the chosen story ran: {}", stderr_of(&out));
    let two = "Opening 2) Zork II: The Wizard of Frobozz  (v5 r48 s840904)  Zork2.Data";
    assert!(text.contains(two), "says which:\n{text}");
    let _ = std::fs::remove_file(&image);
}

/// `--story <name>` picks by name, case-insensitively and on a partial. On this
/// medium the names differ; on an Atari ST compilation every story is called
/// `STORY.DAT`, which is why the number always works too.
#[test]
fn story_name_picks_one_off_the_disk() {
    let image = two_story_floppy("name.adf");
    let out = run(&image, &["--story", "zork1"], "");
    let text = stdout_of(&out);
    assert!(out.status.success(), "the chosen story ran: {}", stderr_of(&out));
    let one = "Opening 1) Zork I: The Great Underground Empire  (v3 r88 s840726)  Zork1.Data";
    assert!(text.contains(one), "says which:\n{text}");

    let out = run(&image, &["--story", "zork9"], "");
    assert!(!out.status.success(), "a name that matches nothing cannot open a story");
    assert!(stderr_of(&out).contains("is named 'zork9'"), "{}", stderr_of(&out));
    let _ = std::fs::remove_file(&image);
}

// ── real compilations, on both FAT12 machines (SQ-0833, SQ-0835) ──────────────

/// **The menu the synthetic disks above were standing in for.** Until now every
/// real floppy in the corpus carried exactly one story, so the choose-a-story
/// path was only ever exercised against a disk this file built. *The Lost
/// Treasures of Infocom* I, floppy2 carries six, and they are six different
/// games with six different headers.
///
/// `HITCHHIK.DAT` here is the Solid Gold **v5 r31 s871119** — a different
/// release AND a different Z-machine version from the standalone DOS disk's v3
/// r58 s851002 and from the Atari ST's v3 r56 s841221. Which is why the menu
/// shows the header beside the name: on this medium the names differ, and on
/// the next one they do not.
#[test]
fn a_real_dos_compilation_lists_six_games_and_story_opens_one() {
    let Some(image) = story_path("floppy2.ima") else { return };
    let err = stderr_of(&run(&image, &[], ""));
    // Twenty, not six: `floppy2` is one volume of `floppy1`…`floppy5`, and since
    // SQ-0961 the menu is the RELEASE's rather than the platter's — the same
    // twenty rows the browser has listed for this set since SQ-0844. The named
    // volume's own six still come first, which is why the indices below hold.
    assert!(err.contains("holds 20 stories"), "the whole release is offered:\n{err}");
    let gold = "1) The Hitchhiker's Guide to the Galaxy  (v5 r31 s871119)  HITCHHIK.DAT";
    assert!(err.contains(gold), "the Solid Gold release:\n{err}");
    let rest = "4) Ballyhoo  (v3 r97 s851218)  BALLYHOO.DAT";
    assert!(err.contains(rest), "…and the rest of them:\n{err}");

    let out = run(&image, &["--story", "hitchhik"], "quit\ny\n");
    let text = stdout_of(&out);
    let opened = "Opening 1) The Hitchhiker's Guide to the Galaxy  (v5 r31 s871119)  HITCHHIK.DAT";
    assert!(text.contains(opened), "says which it opened:\n{text}");
    assert!(
        text.contains("Release 31 / Serial number 871119"),
        "and the GAME says which release it is:\n{text}"
    );
}

/// The Atari ST, where the filename gives nothing away: four games, four
/// folders, and four files called `STORY.DAT`. The **directory** is the only
/// thing that tells them apart, so it is what the menu shows — which is the
/// whole of SQ-0835's naming finding, visible at the prompt.
///
/// FALSIFICATION: have `blorb::fat12` report `Fat12Entry::name` instead of
/// `Fat12Entry::path` and this menu becomes four identical `STORY.DAT` lines,
/// distinguishable only by their headers and unpickable by `--story <name>`.
#[test]
fn a_real_atari_st_compilation_names_its_games_by_directory() {
    let Some(image) = story_path("Infocom Compilation 9 (19xx)(-).st") else { return };
    let err = stderr_of(&run(&image, &[], ""));
    // Thirty-three: the nine-volume ST shelf, whose duplicate builds fold to 33
    // games exactly as `disk_set_rows` pins them for the browser (SQ-0961). This
    // volume's own four are still the first four rows, and the claim under test
    // is what they are CALLED.
    assert!(err.contains("holds 33 stories"), "the whole shelf is offered:\n{err}");
    for line in [
        "1) The Hitchhiker's Guide to the Galaxy  (v3 r56 s841221)  HITCHHIK/STORY.DAT",
        "2) Bureaucracy  (v4 r86 s870212)  BUREAUCR.ACY/STORY.DAT",
        "3) Cutthroats  (v3 r23 s840809)  CUTHROAT/STORY.DAT",
        "4) Leather Goddesses of Phobos  (v3 r59 s860730)  LEATHER.GOD/STORY.DAT",
    ] {
        assert!(err.contains(line), "expected {line:?} in:\n{err}");
    }
    assert!(!err.contains(".SAV"), "the disk's 1996 saved games are not games:\n{err}");

    // …and the directory name is enough to pick one, which a bare `STORY.DAT`
    // never could be.
    let out = run(&image, &["--story", "cuthroat"], "quit\ny\n");
    let text = stdout_of(&out);
    let opened = "Opening 3) Cutthroats  (v3 r23 s840809)  CUTHROAT/STORY.DAT";
    assert!(text.contains(opened), "says which it opened:\n{text}");
    assert!(text.contains("Hardscrabble Island"), "…and Cutthroats really ran:\n{text}");
}

// ── the Apple II (SQ-0836) ───────────────────────────────────────────────────

/// **A fifth filesystem, and this front-end needed no line of code for it.**
///
/// `blorb::medium` gained a ProDOS row; `zvm-cli` reads the table and nothing
/// else, so a 2IMG-wrapped Apple IIgs volume mounts, its games are listed with
/// their headers, and `--story` opens one — all of it through the same
/// `looks_like_image` / `mount_and_pick` path an Amiga floppy takes. That is the
/// property SQ-0840 built the table for, exercised on the first format added
/// since.
///
/// *The Lost Treasures of Infocom* (1993, Big Red Computer Club) volume 7 is the
/// smallest useful case: three games, three names, no directories.
///
/// FALSIFICATION: delete the `DiskImage::ProDos` row from `blorb::medium`'s
/// `FORMATS` and this fails at the first assertion with
/// `Error: Z-machine version 50 is not supported.` — `0x32`, the `2` of `2IMG`,
/// read as a story header.
#[test]
fn a_real_apple_iigs_compilation_mounts_through_the_shared_table() {
    let Some(image) =
        story_path("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 7 of 7).2mg")
    else {
        return;
    };
    let err = stderr_of(&run(&image, &[], ""));
    // Thirty across the seven-volume press (SQ-0961); this volume's own three
    // lead, and they are what this case is about.
    assert!(err.contains("holds 30 stories"), "the whole release is offered:\n{err}");
    for line in [
        "1) Hollywood Hijinx  (v3 r37 s861215)  HOLLYWOOD",
        "2) Cutthroats  (v3 r23 s840809)  CUTTTHROAT",
        "3) Wishbringer  (v3 r69 s850920)  WISHBRINGER",
    ] {
        assert!(err.contains(line), "expected {line:?} in:\n{err}");
    }

    let out = run(&image, &["--story", "wishbringer"], "quit\ny\n");
    let text = stdout_of(&out);
    let opened = "Opening 3) Wishbringer  (v3 r69 s850920)  WISHBRINGER";
    assert!(text.contains(opened), "says which it opened:\n{text}");
    assert!(
        text.contains("Release 69 / Serial Number 850920"),
        "and the GAME says which release it is:\n{text}"
    );
}

/// **Five games on `INFOCOM6`, and the fifth is the one that was invisible**
/// (SQ-0856).
///
/// *Lost Treasures* volume 6 holds five stories, but `LEATHRGODDESSES` writes
/// its header serial as `C2 EC EF F7 EE A1` — "Blown!" in the high ASCII the
/// Apple II wrote text in — and the story recogniser demanded bit 7 clear, so
/// the disk listed four. Nothing in this front-end knows any of that: it reads
/// `blorb::medium`'s table, so the game arrived in the menu with the rule change
/// and the CLI was not touched. That is what this case is here to prove, and
/// then it plays the game to prove it is a game.
///
/// FALSIFICATION: drop the `& 0x7f` from the serial check in
/// `blorb::adf::looks_like_zcode` and this fails at the first assertion with
/// `holds 4 stories` — the reported symptom, off the reported disk.
#[test]
fn a_high_ascii_serial_does_not_hide_a_game_from_the_menu() {
    let Some(image) =
        story_path("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 6 of 7).2mg")
    else {
        return;
    };
    let err = stderr_of(&run(&image, &[], ""));
    // Thirty across the press (SQ-0961); volume 6's own five lead it, and the
    // fifth of those is the one this case exists for.
    assert!(err.contains("holds 30 stories"), "the whole release is offered:\n{err}");
    for line in [
        "1) Sherlock: The Riddle of the Crown Jewels  (v5 r21 s871214)  SHERLOCK",
        "2) Border Zone  (v5 r9 s871008)  BORDERZONE",
        "3) Nord and Bert Couldn't Make Head or Tail of It  (v4 r19 s870722)  NORDANDBERT",
        // The one build on any set the table cannot name: this Apple II press
        // writes its header in high ASCII, so its release and serial read as 0
        // and `Blown!`. There is no key to add, so the stored name stands.
        "4) LEATHRGODDESSES  (v3 r0 sBlown!)",
        "5) Plundered Hearts  (v3 r26 s870730)  PLUNDERED",
    ] {
        assert!(err.contains(line), "expected {line:?} in:\n{err}");
    }

    // …and it really is Leather Goddesses of Phobos. The game opens on its own
    // content warning and waits for RETURN, so the script leads with a blank
    // line before asking it to identify itself.
    let out = run(&image, &["--story", "leathr"], "\nversion\nquit\ny\n");
    let text = stdout_of(&out);
    assert!(text.contains("Opening 4) LEATHRGODDESSES"), "says which it opened:\n{text}");
    assert!(
        text.contains("LEATHER GODDESSES OF PHOBOS"),
        "and the GAME says what it is:\n{text}"
    );
    assert!(text.contains("Joe's Bar"), "…and it reached its opening room:\n{text}");
}

/// The other half, and the finding behind it: `Journey.2mg` is the segmented
/// Apple II press, so no whole story file comes off it. The disk must still
/// MOUNT — the user is told what is on it and asked whether this is the boot
/// disk, rather than shown a corrupt-story error.
#[test]
fn a_real_prodos_disk_with_no_whole_story_reports_what_it_mounted() {
    let Some(image) = story_path("Journey.2mg") else { return };
    let err = stderr_of(&run(&image, &[], ""));
    assert!(
        err.contains("no story file on this disk image") && err.contains("mounted on JOURNEY"),
        "the mount succeeded and named the volume:\n{err}"
    );
}

/// **The first Apple II disk this front-end can actually PLAY** (SQ-0868).
///
/// Every Apple format lanthorn reads could be mounted here and never played,
/// because `zvm-cli` declines Version 6 by design and *Arthur*, *Journey*,
/// *Shogun* and *Zork Zero* — the Apple releases in the corpus — are all v6. The
/// two `.2mg` compilations above are the exception that proves it: they play
/// because they are collections of v3/v5 games, not because Apple media works
/// end to end.
///
/// `Planetfall r29 (clean copy from retail disk).dsk` is the first Apple disk in
/// `stories/` carrying a single non-v6 game, and it is a **raw self-booting
/// disk** with no filesystem at all — the story is 426 sectors located only by
/// putting them into DOS 3.3 logical order and verifying them against the
/// story's own header checksum (`blorb::infocom_boot`).
///
/// **Nothing in this front-end was touched to make this pass.** `zvm-cli` reads
/// `blorb::medium`'s table, so the format arrived with the reader — which is the
/// property SQ-0840 built the table for and this is its fourth demonstration.
///
/// FALSIFICATION: delete the `DiskImage::InfocomBootDisk` row from
/// `blorb::medium`'s `FORMATS` and this fails at the first assertion with
/// `Error: Z-machine version 1 is not supported.` — `0x01`, the first byte of
/// the Apple II boot sector, read as a story header.
#[test]
fn a_raw_self_booting_apple_disk_plays_through_the_shared_table() {
    let Some(image) = story_path("Planetfall r29 (clean copy from retail disk).dsk") else {
        return;
    };
    let out = run(&image, &[], "look\nquit\ny\n");
    let text = stdout_of(&out);
    assert!(
        text.contains("PLANETFALL:  INTERLOGIC Science Fiction"),
        "the game boots off the raw disk:\n{text}"
    );
    assert!(
        text.contains("Release 29 / Serial number 840118"),
        "and the GAME says which release came off those sectors:\n{text}"
    );
    assert!(text.contains("Deck Nine"), "…and it reached its opening room:\n{text}");
}

// ── the medium picks the machine (SQ-0839) ────────────────────────────────────

/// Write bytes to a temp file and return the path.
fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let nth = NTH.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("zvm-cli-tmp-{}-{name}-{nth}", std::process::id()));
    std::fs::write(&path, bytes).expect("temp file written");
    path
}

/// The rule, and its override, on one disk: a story mounted off an Amiga floppy
/// is told it is running on an Amiga — interpreter 4, ZMSD §11.1.3 — and `-I`
/// still outranks the medium, because the number you name is the machine you
/// asked for.
///
/// FALSIFICATION: drop the `.or_else(|| medium.and_then(…))` in `main.rs` and
/// the first assertion fails with the reported symptom — `[1]`, the IBM PC
/// default, on a disk that is not one.
#[test]
fn a_story_off_an_amiga_floppy_is_told_it_is_an_amiga() {
    let mut f = Floppy::new();
    f.add_file("Story.data", &interpreter_reporting_story());
    let image = f.write("machine.adf");

    let text = stdout_of(&run(&image, &[], ""));
    assert!(text.contains("[4]"), "the medium defaults header $1E to the Amiga's 4:\n{text}");

    // The contract: an explicit number beats the medium, in both directions.
    let text = stdout_of(&run(&image, &["-I", "6"], ""));
    assert!(text.contains("[6]"), "-I 6 outranks the Amiga floppy it was opened from:\n{text}");
    let text = stdout_of(&run(&image, &["--interpreter", "3"], ""));
    assert!(text.contains("[3]"), "--interpreter is the same flag, same rank:\n{text}");

    let _ = std::fs::remove_file(&image);
}

/// …and the blast radius is exactly zero for everything that is not a medium.
/// An ordinary story file keeps zvm's own rule (Frotz's: 1 below Version 6), so
/// this feature cannot move a single story anybody already plays.
#[test]
fn an_ordinary_story_file_keeps_the_default_interpreter() {
    let story = write_temp("plain.z3", &interpreter_reporting_story());
    let text = stdout_of(&run(&story, &[], ""));
    assert!(text.contains("[1]"), "no medium, no opinion — zvm's own rule stands:\n{text}");
    let text = stdout_of(&run(&story, &["-I", "4"], ""));
    assert!(text.contains("[4]"), "and asking for a machine still works without one:\n{text}");
    let _ = std::fs::remove_file(&story);
}

/// The real-media proof, and the one that matters: a genuine Infocom release
/// floppy, driven as a real game, answering VERSION in its own words.
///
/// *Zork: The Undiscovered Underground* (release 16 / serial 970828) is
/// Inform-compiled, and the Inform library's banner prints header `$1E` outright
/// — so the game itself says which machine it was told about. The app-side
/// suite (`real_media_releases.rs`) pins the same line off the same floppy; this
/// is the CLI reaching the identical conclusion, which is the whole point of
/// SQ-0839.
///
/// FALSIFICATION: with the `main.rs` default reverted this reads
/// `Interpreter 1 Version A`, which is the bug as reported — an Amiga floppy
/// mounted and then run as an IBM PC.
#[test]
fn a_real_release_floppy_tells_the_game_which_machine_it_is() {
    let Some(image) = story_path("Zork - The Undiscovered Underground.adf") else { return };

    let text = stdout_of(&run(&image, &["--pager", "off"], "version\nquit\ny\n"));
    assert!(
        text.contains("Release 16 / Serial number 970828"),
        "this is the build the floppy carries:\n{text}"
    );
    assert!(
        text.contains("Interpreter 4 "),
        "the game names the Amiga it was told it is running on:\n{text}"
    );

    // And the override, on the same real media.
    let text = stdout_of(&run(&image, &["--pager", "off", "-I", "6"], "version\nquit\ny\n"));
    assert!(text.contains("Interpreter 6 "), "-I outranks the floppy:\n{text}");
}

// ── a release that is on no single disk (SQ-0874) ────────────────────────────

/// Every multi-volume release in the corpus, named by a volume that carries no
/// whole story of its own. `v6` marks the ones this front-end will mount and then
/// decline, which is a pass: the refusal is about the GAME, so it proves the
/// mount reached one.
const ACROSS_THE_SET: &[(&str, bool)] = &[
    ("TRINITY1.D64", false),
    ("TRINITY2.D64", false),
    ("shogun_s1.dsk", true),
    ("journey_s1.dsk", true),
    ("zork_zero_1.dsk", true),
];

fn any_set_volume_present() -> bool {
    ACROSS_THE_SET.iter().any(|(n, _)| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join("stories").join(n).exists()
    })
}

/// **The defect this quest was filed for.** `zvm-cli` mounted with
/// `MountedDisk::mount` — `mount_set` with no companions — so a release whose
/// story is paged across its volumes was unreachable here while the TUI opened it
/// perfectly well.
///
/// The rule for which files are one release lives in `cli_host::disk_set` now, so
/// both front-ends ask one implementation, and the mount goes through
/// `cli_host::disk_set::mount_at`.
///
/// FALSIFICATION: put `MountedDisk::mount(raw)` back in `media::story_candidates`
/// and every row here fails with the symptom as reported. Verbatim, off
/// *Trinity*, whose game is on both sides and on neither:
///
/// ```text
/// TRINITY1.D64: the set was never consulted:
/// Error: no story file on this disk image (0 files mounted on TRINITY; is this the boot disk?)
/// ```
#[test]
fn a_release_paged_across_its_volumes_opens_from_any_one_of_them() {
    let mut ran = 0;
    for (name, v6) in ACROSS_THE_SET {
        let Some(image) = story_path(name) else { continue };
        ran += 1;
        let out = run(&image, &["--pager", "off"], "\nversion\nquit\ny\n");
        let err = stderr_of(&out);
        assert!(
            !err.contains("no story file on this disk image"),
            "{name}: the set was never consulted:\n{err}"
        );
        if *v6 {
            // Mounted, then declined for what it is — this front-end renders no
            // v6 graphics. A disk error here would mean the mount failed.
            assert!(err.contains("v6 graphical games are not supported"), "{name}: {err}");
            assert!(!err.contains("disk image"), "{name}: not a disk error:\n{err}");
        } else {
            let text = stdout_of(&out);
            assert!(
                text.contains("Release 12 / Serial Number 860926"),
                "{name}: the reassembled Trinity did not run:\n{text}"
            );
            assert!(text.contains("Kensington Gardens"), "{name}: no opening room:\n{text}");
        }
    }
    assert!(ran > 0 || !any_set_volume_present(), "set media are present but nothing ran");
}

/// The Apple II presses name the story the set carries, and its release — which
/// is what `--story` picks by, and the proof the reassembly is the real build
/// (*Shogun* release 311 / serial 890510) rather than plausible-looking Z-code.
///
/// A set volume offers exactly ONE candidate: the story the release keeps across
/// it. So `--story 1` is the whole range, `--story <name>` matches that one
/// story's name, and nothing is ever ambiguous — the menu never appears, because
/// there is no choice to make.
///
/// FALSIFICATION: the same revert as above, and this fails with
/// `Error: no story file on this disk image (4 files mounted on SHOGUN.1; is
/// this the boot disk?)` — four segments of a story and not a story.
#[test]
fn a_set_offers_exactly_the_one_story_the_release_carries() {
    let Some(image) = story_path("shogun_s1.dsk") else { return };
    // Out of range says what the range is, which is how the name gets shown
    // without the v6 refusal cutting the run short.
    let err = stderr_of(&run(&image, &["--story", "2"], ""));
    assert!(err.contains("pick 1 to 1"), "one story, so one choice:\n{err}");
    let row = "1) Shogun  (v6 r311 s890510)  SHOGUN.D1";
    assert!(err.contains(row), "the build it reassembled:\n{err}");

    // …and the name really does select it: this reaches the v6 refusal, which
    // only a story that was found can be refused for.
    let err = stderr_of(&run(&image, &["--story", "shogun"], ""));
    assert!(err.contains("v6 graphical games are not supported"), "--story picked it:\n{err}");
}

/// **The other half: a lone volume is still a lone volume.** *Hitchhiker's* is a
/// single Commodore floppy sitting in the same directory as *Trinity*'s two
/// sides, on the same `.d64` spelling — it must open exactly as it always did,
/// with no sibling consulted and nothing folded into it.
#[test]
fn a_single_volume_release_is_unaffected_by_the_sets_beside_it() {
    let Some(image) = story_path("Hitchhikers_Guide_to_the_Galaxy_The_1984_Infocom.d64") else {
        return;
    };
    let text = stdout_of(&run(&image, &["--pager", "off"], "quit\ny\n"));
    assert!(
        text.contains("Release 47 / Serial number 840914"),
        "the lone floppy's own build, not a set's:\n{text}"
    );
    assert!(!text.contains("Which one?"), "one story is not a choice:\n{text}");
}

/// A disk with no story on it — an AmigaDOS boot disk, say — says so, rather
/// than failing later as if the image were a corrupt story file.
#[test]
fn a_disk_with_no_story_says_what_it_mounted() {
    let mut f = Floppy::new();
    f.add_file("Startup-Sequence", b"echo hello\n");
    let image = f.write("boot.adf");
    let out = run(&image, &[], "");
    assert!(!out.status.success());
    let err = stderr_of(&out);
    assert!(err.contains("no story file on this disk image"), "{err}");
    assert!(err.contains("1 file mounted"), "says what it did find:\n{err}");
    let _ = std::fs::remove_file(&image);
}

// ── the machine, not just its number (SQ-0872) ────────────────────────────────

/// A Version 5 story that prints header `$1E`, `$2C` and `$2D` as `[n/bg/fg/]`.
///
/// Version 5 because `$2C`/`$2D` are colour bytes from V5 on — below that
/// `write_default_colours` correctly declines — and because `zvm-cli` refuses
/// Version 6 outright, so v5 is the highest machine this front-end can be asked
/// about.
fn machine_reporting_story() -> Vec<u8> {
    let mut b = quitting_story(5, 88, "840726");
    let mut code: Vec<u8> = vec![0xE5, 0x7F, 0x5B]; // print_char '['
    for addr in [0x1E_u8, 0x2C, 0x2D] {
        code.extend_from_slice(&[
            0x10, 0x00, addr, 0x00, // loadb 0 addr -> sp
            0xE6, 0xBF, 0x00, // print_num sp
            0xE5, 0x7F, 0x2F, // print_char '/'
        ]);
    }
    code.extend_from_slice(&[0xE5, 0x7F, 0x5D, 0xBA]); // print_char ']' ; quit
    b[0x40..0x40 + code.len()].copy_from_slice(&code);
    b
}

/// **The defect this quest was filed for.** `zvm-cli` set header `$1E` and
/// nothing else, so a story off release media was told WHICH machine it was on
/// and left to work out what that machine LOOKED like from zvm's own §8.3.2 seed
/// — the interpreter's number and the interpreter's page disagreeing, which is
/// the half-wired state SQ-0836 refused to ship.
///
/// The pairs are `zvm::interpreter`'s, each sourced out of Infocom's own
/// interpreter for that machine, and the app reads the very same table — so the
/// two front-ends cannot present different machines off the same bytes.
///
/// FALSIFICATION: delete the `zvm::interpreter::machine` block from `main.rs`'s
/// `build_machine` and every non-Apple row here fails with the machine's pair
/// absent, verbatim:
///
/// ```text
/// --interpreter 3 (Macintosh) must report its own $2C/$2D
///   left: "[3/2/9/]"
///  right: "[3/9/2/]"
/// ```
///
/// — `2/9`, zvm's own §8.3.2 seed, under a byte that says Macintosh.
#[test]
fn an_explicit_machine_brings_its_default_colours_with_it() {
    let story = write_temp("machine.z5", &machine_reporting_story());

    // SQ-0928 CHANGED WHAT THIS TEST IS ABOUT, and the change is deliberate.
    //
    // A machine's `$2C`/`$2D` describes a MACHINE. Running a story off its release
    // disk makes that description true of the launch; typing a number does not.
    // The distinction had no teeth while the IBM PC declined a pair, because the
    // FALLBACK machine painted nothing — but it states blue under white now, and
    // `InterpreterProfile` answers `IbmPc` for every story with no medium, so
    // without the gate `-I 6` (and the plain default) would paint everything blue.
    //
    // So a hand-named machine reaches `$1E` and stops there, and `--colour machine`
    // is how a player says they meant the whole machine.
    for (n, name, want) in [
        (3u8, "Macintosh", "[3/9/2/]"),
        (5, "Atari ST", "[5/9/2/]"),
        (10, "Apple IIgs", "[10/2/9/]"),
    ] {
        let named = stdout_of(&run(&story, &["-I", &n.to_string()], ""));
        assert!(
            named.contains(&format!("[{n}/2/9/]")),
            "--interpreter {n} ({name}) names the machine and defers the page\n   got: {named}"
        );
        let opted = stdout_of(&run(&story, &["-I", &n.to_string(), "--colour", "machine"], ""));
        assert!(
            opted.contains(want),
            "--interpreter {n} --colour machine reports its own $2C/$2D\n  want: {want}\n   got: {opted}"
        );
    }

    // **A decline is a decline, not a default.** The Commodore 128 states no pair
    // (SQ-0869 read no Commodore interpreter), so the story is told what zvm's own
    // §8.3.2 seed says — the same answer an IBM PC gets, which is the point.
    let cbm = stdout_of(&run(&story, &["-I", "7"], ""));
    let pc = stdout_of(&run(&story, &["-I", "6"], ""));
    assert!(cbm.contains("[7/2/9/]"), "the Commodore declines its pair:\n{cbm}");
    // SQ-0928: the IBM PC no longer declines — it states blue 6 under white 9 —
    // but a number NAMED BY HAND is not original media, so the pair is withheld
    // and the story still sees zvm's own §8.3.2 seed. That is the whole of the
    // gate: without it, `-I 6` on any story anyone opened would paint it blue.
    assert!(pc.contains("[6/2/9/]"), "…and a hand-named IBM PC still defers:\n{pc}");
    // …until the player says they meant it.
    let pc_opt = stdout_of(&run(&story, &["-I", "6", "--colour", "machine"], ""));
    assert!(
        pc_opt.contains("[6/6/9/]"),
        "--colour machine advertises the machine's own blue under white:\n{pc_opt}"
    );

    // The override contract (SQ-0839/SQ-0855): `--game-colours off` declares the
    // interpreter COLOURLESS (§8.3.2), so no machine's page is advertised even
    // when one is asked for by name. The number still lands — that is not a
    // colour — and the pair falls back to the seed.
    let mono = stdout_of(&run(&story, &["-I", "3", "--game-colours", "off"], ""));
    assert!(
        mono.contains("[3/2/9/]"),
        "a colourless interpreter advertises no machine page:\n{mono}"
    );

    // And the Amiga is the one machine whose page a v5 story cannot be told:
    // colour 12 is a Version 6 grey (§8.3.1), so `clamp_default_colour` folds it
    // back to black. Stated so the clamp reads as measured rather than missed —
    // `zvm-cli` refuses Version 6, so the Amiga's dark grey never reaches it.
    let amiga = stdout_of(&run(&story, &["-I", "4"], ""));
    assert!(amiga.contains("[4/2/9/]"), "12 is a v6-only grey and clamps here:\n{amiga}");

    let _ = std::fs::remove_file(&story);
}

/// …and the MEDIUM route, which is the one a player actually takes: no flags at
/// all, just a disk. The number and the page arrive together off the same bytes.
#[test]
fn a_story_off_a_floppy_is_told_the_whole_machine() {
    let mut f = Floppy::new();
    f.add_file("Story.data", &machine_reporting_story());
    let image = f.write("machine-colours.adf");

    // An Amiga floppy: interpreter 4, and its pair clamped as above.
    let text = stdout_of(&run(&image, &[], ""));
    assert!(text.contains("[4/"), "the medium still picks the machine:\n{text}");
    // …and `-I` still outranks it for the NUMBER. It no longer brings the named
    // machine's page on its own (SQ-0928): the disk is what makes a machine true
    // of a launch, and naming a different one over the top is a request about the
    // story rather than a statement about where it came from.
    let text = stdout_of(&run(&image, &["-I", "3"], ""));
    assert!(text.contains("[3/2/9/]"), "-I names the Macintosh and defers its page:\n{text}");
    // The whole machine is still one flag away.
    let text = stdout_of(&run(&image, &["-I", "3", "--colour", "machine"], ""));
    assert!(text.contains("[3/9/2/]"), "-I --colour machine brings its page too:\n{text}");

    let _ = std::fs::remove_file(&image);
}

/// A number naming a machine `zvm::interpreter` does not model still reaches
/// `$1E` — the story asked and §11.1.3 has an answer — but everything else about
/// the presentation is then the IBM PC's, and SQ-0872 exists because a silent
/// substitution is indistinguishable from a supported machine.
///
/// 11 is the Tandy Color and 1 the DECSystem-20: each absent for a stated reason
/// (see `zvm::interpreter::MACHINES`), and each announced. **8 used to be in this
/// list and is not** — the Commodore 64 gained a row in SQ-0873, so asking for it
/// is now asking for a machine zvm models rather than reaching for a gap.
#[test]
fn an_unmodelled_machine_says_so_instead_of_quietly_becoming_an_ibm_pc() {
    let story = write_temp("unmodelled.z5", &machine_reporting_story());

    for n in [1u8, 11] {
        let out = run(&story, &["-I", &n.to_string()], "");
        let err = stderr_of(&out);
        assert!(
            err.contains(&format!("interpreter {n} names a machine zvm does not model")),
            "-I {n} must announce the gap:\n{err}"
        );
        // …and the byte still lands, because declining it would name a DIFFERENT
        // machine rather than none — zvm's own rule would answer 1 here.
        assert!(stdout_of(&out).contains(&format!("[{n}/")), "…and $1E still says {n}");
    }

    // A machine we DO model is not announced — the warning has to mean something.
    for n in [3u8, 4, 5, 6, 7, 10] {
        let err = stderr_of(&run(&story, &["-I", &n.to_string()], ""));
        assert!(!err.contains("does not model"), "-I {n} is modelled and must stay quiet:\n{err}");
    }

    // Nor is a plain disk launch ever announced: every number a medium hands out
    // is modelled, and `interpreter_profile`'s agreement test pins that.
    let mut f = Floppy::new();
    f.add_file("Story.data", &machine_reporting_story());
    let image = f.write("quiet.adf");
    assert!(!stderr_of(&run(&image, &[], "")).contains("does not model"), "a disk stays quiet");

    let _ = std::fs::remove_file(&image);
    let _ = std::fs::remove_file(&story);
}

/// The real-media proof, on the disk the quest was reported against: *Beyond
/// Zork* off its Apple IIgs ProDOS press (`Beyond Zork (1988)(Infocom).2mg`,
/// release 57 / serial 871221). The game names the machine in its own banner.
///
/// **And it is the medium on which the old defect was INVISIBLE**, which is worth
/// pinning rather than glossing: the Apple's page is standard colour 2 over ink 9
/// (`zboot.asm`), and zvm's own §8.3.2 seed is 2 over 9 as well — so the byte and
/// the page happened to agree here all along, while a Macintosh or an Atari ST
/// press was being handed a machine that said one thing and looked like another.
/// A regression that put the seed back would pass this test's colour assertion
/// and fail `an_explicit_machine_brings_its_default_colours_with_it`, which is
/// why both exist.
#[test]
fn the_prodos_press_names_its_machine_and_reports_that_machines_page() {
    let Some(image) = story_path("Beyond Zork (1988)(Infocom).2mg") else { return };

    let text = stdout_of(&run(&image, &["--pager", "off"], "begin\n\n\n\n\nversion\nquit\ny\n"));
    assert!(
        text.contains("Release 57 / Serial Number 871221"),
        "this is the build the ProDOS press carries:\n{text}"
    );
    assert!(
        text.contains("Apple //gs"),
        "and the GAME names the machine it was told it is running on:\n{text}"
    );
}

// ── the compilation discs, named from the bundled table (SQ-0884) ─────────────

/// A gitignored compilation disc, by directory and filename. These live beside
/// `stories/` rather than in it — `treasures/` and `masterpieces/` — and like it
/// they are absent from any checkout that has not been handed the media.
fn disc_path(dir: &str, name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(dir).join(name);
    if p.exists() {
        return Some(p);
    }
    eprintln!("SKIP: gitignored disc missing at {}", p.display());
    None
}

/// The three compilation sets this quest measured, with the number of stories
/// each carries. The count is here so a test that enumerates nothing cannot
/// quietly pass as a test that found everything named.
const COMPILATION_DISCS: &[(&str, &str, usize)] = &[
    ("treasures/ISOs", "LostTreasures1.iso", 40),
    ("treasures/ISOs", "LostTreasures2.iso", 28),
    ("masterpieces", "Classic Text Adventure Masterpieces of Infocom (USA).bin", 83),
];

/// The Z-machine release and serial in a story's header, with bit 7 off each
/// serial byte exactly as `Candidate::header` masks it.
fn build_of(bytes: &[u8]) -> Option<(u16, String)> {
    if bytes.len() < 0x18 || !(3..=8).contains(&bytes[0]) {
        return None;
    }
    let serial: String = bytes[0x12..0x18].iter().map(|c| char::from(c & 0x7f)).collect();
    Some((u16::from_be_bytes([bytes[0x02], bytes[0x03]]), serial))
}

/// **Half two of this quest, as a test.** Every Z-code build these three sets
/// carry must be a build `known_titles.tsv` can name — that is what makes the
/// menu above read as a shelf of games rather than a directory listing, and it
/// is the thing that goes stale when a new set arrives.
///
/// Driving the media is the point: the table is checked against what is actually
/// pressed on the discs, not against a remembered list. A row that fails here
/// names the exact release and serial to add, measured off the medium that
/// carries it — which is the only provenance this table accepts, because a disc
/// is a different release and not the same story on other media.
///
/// The sweep that first ran this found **nothing missing**: all 151 stories
/// across the three sets resolve, so no row was added. The value is forward —
/// the next set that turns up an uncovered build fails here and says which.
#[test]
fn every_build_on_the_compilation_discs_has_a_canonical_title() {
    let mut ran = 0;
    for (dir, name, want) in COMPILATION_DISCS {
        let Some(path) = disc_path(dir, name) else { continue };
        ran += 1;
        let raw = std::fs::read(&path).expect("the disc reads");
        let disk = cli_host::disk_set::mount_at(&path, raw).expect("the disc mounts");
        let stories = disk.stories();
        assert_eq!(stories.len(), *want, "{name}: the disc's story count changed");
        let unnamed: Vec<String> = stories
            .iter()
            .filter_map(|s| {
                let (release, serial) = build_of(&s.bytes)?;
                cli_host::titles::title_for_build(release, &serial)
                    .is_none()
                    .then(|| format!("ZCODE-{release}-{serial}\t({})", s.name))
            })
            .collect();
        assert!(
            unnamed.is_empty(),
            "{name}: known_titles.tsv does not carry these builds — add them:\n{}",
            unnamed.join("\n")
        );
    }
    assert!(
        ran > 0 || COMPILATION_DISCS.iter().all(|(d, n, _)| disc_path(d, n).is_none()),
        "discs are present but nothing ran"
    );
}

/// **The defect this quest was filed for**, off the real disc. *Lost Treasures
/// of Infocom I* listed forty rows of `MAC/BALLYHOO` and `PC/DATA/BEYONDZO.DAT`
/// — the names the medium stored, none of which is a game's name.
///
/// FALSIFICATION: make `media::Candidate::label` build from `self.name` again
/// and every row here fails with the symptom as reported, verbatim:
///
/// ```text
/// 2) MAC/BEYOND ZORK  (v5 r57 s871221)
/// 22) PC/DATA/BEYONDZO.DAT  (v5 r57 s871221)
/// ```
///
/// — one game, two files, and nothing on either line that says *Beyond Zork*.
#[test]
fn a_real_compilation_disc_names_its_games_from_the_table() {
    let Some(image) = disc_path("treasures/ISOs", "LostTreasures1.iso") else { return };
    let err = stderr_of(&run(&image, &[], ""));
    // `LostTreasures1.iso` and `LostTreasures2.iso` are two volumes of one
    // release, and since SQ-0961 naming either offers both, so the count is the
    // two volumes' `COMPILATION_DISCS` counts summed, not a literal. Disc 1's own
    // forty lead, so every index below is disc 1's own.
    let n: usize = COMPILATION_DISCS
        .iter()
        .filter(|(d, name, _)| *d == "treasures/ISOs" && name.starts_with("LostTreasures"))
        .map(|(_, _, c)| c)
        .sum();
    assert!(err.contains(&format!("holds {n} stories")), "both discs are offered:\n{err}");
    for line in [
        "1) Ballyhoo  (v3 r97 s851218)  MAC/BALLYHOO",
        "2) Beyond Zork: The Coconut of Quendor  (v5 r57 s871221)  MAC/BEYOND ZORK",
        "22) Beyond Zork: The Coconut of Quendor  (v5 r57 s871221)  PC/DATA/BEYONDZO.DAT",
        "25) The Hitchhiker's Guide to the Galaxy  (v5 r31 s871119)  PC/DATA/HITCHHIK.DAT",
        "40) Zork Zero: The Revenge of Megaboz  (v6 r393 s890714)  PC/ZORK0/ZORK0.ZIP",
    ] {
        assert!(err.contains(line), "expected {line:?} in:\n{err}");
    }

    // …and the title the menu shows is a title `--story` answers to. `Ballyhoo`
    // is on this disc twice, so the ambiguity is real and is reported rather
    // than guessed at; `beyond zork` picks out one game and two files.
    let err = stderr_of(&run(&image, &["--story", "ballyhoo"], ""));
    assert!(err.contains("matches more than one story"), "two copies, two answers:\n{err}");
    let err = stderr_of(&run(&image, &["--story", "coconut of quendor"], ""));
    assert!(err.contains("matches more than one story"), "the subtitle matches too:\n{err}");
}

/// *Masterpieces* carries *Ballyhoo* three times — `MAC/BALLYHOO`,
/// `PC/BALLYHOO/DATA/BALLYHOO.DAT` and `PC/DATA/BALLYHOO.DAT`, one build in
/// three files. Naming rows from the table must not turn those into three
/// identical lines, so the stored name stays on every row.
#[test]
fn one_build_in_three_files_is_still_three_distinguishable_rows() {
    let Some(image) =
        disc_path("masterpieces", "Classic Text Adventure Masterpieces of Infocom (USA).bin")
    else {
        return;
    };
    let err = stderr_of(&run(&image, &[], ""));
    let rows: Vec<&str> =
        err.lines().filter(|l| l.contains("Ballyhoo  (v3 r97 s851218)")).collect();
    assert_eq!(rows.len(), 3, "three files carry this build:\n{err}");
    for name in ["MAC/BALLYHOO", "PC/BALLYHOO/DATA/BALLYHOO.DAT", "PC/DATA/BALLYHOO.DAT"] {
        assert!(rows.iter().any(|r| r.ends_with(name)), "no row for {name}:\n{err}");
    }
}


// ── SQ-0930: a hybrid disc is two machines, per story ─────────────────────────

/// **The CLI read the IMAGE's format where it should have read the STORY's.**
///
/// Two compilation discs carry both machines' builds side by side, and the
/// difference is per file, not per volume:
///
///   masterpieces .bin   mounts HFS   -> every PC build reported the MACINTOSH
///   LostTreasures1.iso  mounts ISO   -> every PC build reported NOTHING
///
/// The TUI has resolved this per story since SQ-0876 (`read_story_file` calls
/// `image_for`); the CLI never did, and it only became visible when a machine
/// started carrying colours. Both discs are gitignored, so this skips vacuously —
/// with a guard, because a skip that reads as a pass proves nothing.
#[test]
fn a_hybrid_disc_tells_a_pc_build_from_a_mac_one() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut seen = 0;
    for (disc, pc_entry, mac_entry) in [
        (
            "masterpieces/Classic Text Adventure Masterpieces of Infocom (USA).bin",
            "PC/DATA/ZORK1.DAT",
            Some("MAC/ZORK I"),
        ),
        ("treasures/ISOs/LostTreasures1.iso", "PC/DATA/ZORK1.DAT", Some("MAC/ZORK I")),
    ] {
        let path = root.join(disc);
        if !path.is_file() {
            eprintln!("SKIP: gitignored disc missing at {}", path.display());
            continue;
        }
        let raw = std::fs::read(&path).expect("read");
        let Ok(disk) = blorb::medium::MountedDisk::mount(raw) else {
            panic!("{disc}: mounts")
        };
        seen += 1;

        // The PC side names the IBM PC — by the row's `implies_ibm_pc`, because
        // that machine's §11.1.3 number is a version rule rather than a constant.
        let pc = disk.image_for(pc_entry);
        assert!(
            pc.implies_ibm_pc() || pc.interpreter_number() == Some(6),
            "{disc}: {pc_entry} must name the IBM PC, got {pc:?}",
        );

        // …and the Mac side still names the Macintosh, which is the half that was
        // already right and must not be traded away for the other.
        if let Some(mac_entry) = mac_entry {
            let mac = disk.image_for(mac_entry);
            assert_eq!(
                mac.interpreter_number(),
                Some(3),
                "{disc}: {mac_entry} must still name the Macintosh, got {mac:?}",
            );
            assert_ne!(pc, mac, "{disc}: the two halves must not resolve alike");
        }
    }
    let any_present = [
        "masterpieces/Classic Text Adventure Masterpieces of Infocom (USA).bin",
        "treasures/ISOs/LostTreasures1.iso",
    ]
    .iter()
    .any(|d| root.join(d).is_file());
    assert!(!any_present || seen > 0, "a hybrid disc is present but none was read");
}
