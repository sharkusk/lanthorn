//! SQ-0857, the profile half: a ProDOS volume is an Apple IIgs, and *Beyond
//! Zork* can tell.
//!
//! ## The reversal this suite pins
//!
//! SQ-0836 shipped `DiskImage::ProDos` answering `None`, reasoning that ProDOS
//! is the Apple II *family's* filesystem and ZMSD §11.1.3 numbers three machines
//! in it — 2 Apple IIe, 9 Apple IIc, 10 Apple IIgs — so no volume can say which
//! press it is. **That premise is not merely still true, it is now proved from
//! Infocom's own code**, which is the opposite of how these reversals usually
//! go. `apple/yzip/rel.15/bsubs.asm` in `github.com/erkyrath/infocom-zcode-terps`
//! — the Apple II YZIP, Infocom's Version 6 interpreter for the machine — picks
//! between all three at boot:
//!
//! ```text
//!   ; Make sure we are on a good machine, like a ][c or ][e+/][gs
//!   MACHINE:
//!       lda MACHID1 / cmp #6 / bne BADMACH
//!       lda MACHID2 / bne MACH1
//!       lda #IIcID                          ; Apple ][c thank you
//!   MACH1:
//!       sec / jsr MACHCHK / bcs OLDMACH
//!       lda #IIgsID                         ; this is a ][gs
//!   OLDMACH:
//!       lda #IIeID                          ; this is IIe
//!   MACH2:
//!       sta ARG2+LO                         ; save machine id
//! ```
//!
//! and `zboot.asm` puts the result straight into header `$1E` (`ZINTWD EQU 30`):
//! `lda ARG2+LO { get machine id! } / sta ZBEGIN+ZINTWD`. Infocom pressed ONE
//! disk for the whole family and detected the machine at boot — so the medium
//! genuinely cannot name the press, and `IIeID EQU 2 / IIcID EQU 9 / IIgsID EQU
//! 10` in `apple.equ` are all three equally real.
//!
//! That routine is not just in the archive. It assembles to a 32-byte sequence
//! which occurs **byte-identically at offset 1711 of `INFOCOM.SYSTEM` on both
//! `Journey.2mg` and `Arthur Quest 4 Excalibur.2mg`** — the ProDOS 8 launchers
//! of the corpus's two Version 6 releases. `the_shipped_apple_interpreter_still_
//! detects_the_machine_at_boot` pins exactly that, so the argument this quest
//! rests on is checked against the disks rather than recited.
//!
//! ## Why 10 anyway
//!
//! Because **declining is not neutral here**, which is the one thing SQ-0836
//! did not weigh. `DiskImage::Fat12Dos` can answer `None` because zvm's own rule
//! — Frotz's, 6 for Version 6 and 1 otherwise — *is* the IBM PC's rule, so a DOS
//! floppy describes itself correctly by default. On a ProDOS volume that same
//! deferral lands on 1, the DECSystem-20, or on Version 6 on 6, the IBM PC — a
//! machine on another continent, and the one value `zvm`'s `exec.rs` gates its
//! CP437 remap on. `None` does not leave the Apple II unnamed; it names it
//! something else.
//!
//! §11.1.3 asks the question the row actually has to answer: *"An interpreter
//! should choose the interpreter number most suitable for the machine it will
//! run on."* The number is a property of the machine in front of the player —
//! which is precisely why Infocom detected it — so the question is which Apple II
//! lanthorn is. Of the three the YZIP will start on at all (`cmp #6 / bne
//! BADMACH` refuses anything below an enhanced IIe), the IIgs is the top and the
//! one a modern terminal resembles. The other two stay reachable by name.
//!
//! ## What is observable, measured across the whole ProDOS corpus
//!
//! Every story on all ten `.2mg` images was booted under the default rule and
//! under 10 and the traces diffed. Thirty-one stories: **twenty-four are
//! byte-identical** (every Version 3 one, including the high-ASCII-serial
//! *Leather Goddesses* SQ-0856 made visible, plus *A Mind Forever Voyaging* and
//! *Bureaucracy*). **Five print the number** in their VERSION block and are
//! otherwise unchanged — Hitchhiker's, Trinity, Sherlock, Border Zone, Nord and
//! Bert. **One behaves differently, twice, and it is the interesting one:**
//!
//! *Beyond Zork* r57 s871221 — on both the GS/OS `BZ.DAT` press and the *Lost
//! Treasures* `BEYOND.ZORK` one — told the default **1**:
//!
//! ```text
//!   Is this a VT220?
//!   [Please type YES or NO.] >
//! ```
//!
//! Told **10** it never asks and goes straight to BEGIN/RESTORE/QUIT, because an
//! Apple IIgs is not a terminal that might or might not have line-drawing
//! characters. That is the identical finding SQ-0835 recorded for the Atari ST,
//! on the identical game — see `atari_st_profile.rs`, whose shape this follows.
//!
//! **And the game names the machine, in Infocom's own spelling.** Its VERSION
//! block changes from `DEC-20 Color Version A` to **`Apple //gs Color Version
//! A`** — which is the story's reading of our header rather than our header, and
//! the same corroboration SQ-0835 got from "Atari ST Color Version A" and
//! SQ-0838 from Zork Zero r296's Macintosh. Nothing in this codebase wrote the
//! string "Apple //gs"; *Beyond Zork* did, in 1987, on being told 10.
//!
//! The story files are gitignored (CLAUDE.md), so every case skips vacuously
//! when the media are absent.

use std::path::{Path, PathBuf};

use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

/// The standalone Apple IIgs press of *Beyond Zork* — a GS/OS disk whose story
/// is the whole file `BZ.DAT`.
const BEYOND_ZORK_DISK: &str = "Beyond Zork (1988)(Infocom).2mg";
/// The same release again, on the 1993 *Lost Treasures* compilation. Two presses
/// of one build, which is why the behaviour case drives both.
const LOST_TREASURES_2: &str =
    "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 2 of 7).2mg";
const BEYOND_ZORK_RELEASE: u16 = 57;
const BEYOND_ZORK_SERIAL: &str = "871221";

/// A disk that is entirely Version 3, for the negative.
const V3_DISK: &str = "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 7 of 7).2mg";

/// The two Version 6 releases. They carry Infocom's own Apple II YZIP, and no
/// whole story file — the pictures and the story live in the segmented
/// `ARTHUR.D1`-`.D5` / `JOURNEY.D1`-`.D4` container, read by
/// `blorb::infocom_packed` (SQ-0852) and `blorb::infocom_pics`'s Apple flavour
/// (SQ-0863). They are read here for the INTERPRETER, not for a game; what the
/// artwork does to the story's screen is `apple_release_artwork.rs`'s.
const V6_DISKS: [&str; 2] = ["Journey.2mg", "Arthur Quest 4 Excalibur.2mg"];

/// Every ProDOS image this suite knows about.
const ALL_DISKS: [&str; 4] = [BEYOND_ZORK_DISK, LOST_TREASURES_2, V3_DISK, "Journey.2mg"];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Whether any ProDOS image this suite knows about is on disk at all.
///
/// `stories/` is gitignored, so CI and a worktree without the symlink have NO
/// media and every case here must skip vacuously (CLAUDE.md's rule). A bare
/// `assert!(ran > 0)` broke that and failed CI on all three platforms. The guard
/// still earns its keep where it can: with the media present, "nothing ran"
/// means a filename drifted, and that must fail loudly rather than pass empty.
fn any_prodos_disk_present() -> bool {
    ALL_DISKS.iter().chain(V6_DISKS.iter()).any(|n| stories_dir().join(n).is_file())
}

fn disk(name: &str) -> Option<PathBuf> {
    let p = stories_dir().join(name);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: gitignored medium missing at {}", p.display());
        None
    }
}

/// Boot the story `path` opens to, exactly as `startup.rs` does — the profile
/// from the medium — but with `interpreter_override` standing in for an explicit
/// `--interpreter`, which is how the falsification drives this.
fn boot(path: &Path, honor: bool, interpreter_override: Option<u8>) -> Option<GameSession> {
    let (loaded, mounted) = app::hints::load_mounted_story(path).ok()?;
    assert_eq!(
        mounted,
        Some(blorb::medium::DiskImage::ProDos),
        "{}: this suite's fixtures are Apple ProDOS volumes",
        path.display()
    );
    let profile = InterpreterProfile::resolve(path, interpreter_override, None, None);
    // SQ-1021/SQ-1022: every per-machine fact in one value. This suite mounts no
    // archive, so the screen and the density come back `None` exactly as they were
    // written by hand — and the CELL now rides along, which is the point.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &app::graphics::PictSource::new(None),
        None,
        interpreter_override.or_else(|| profile.interpreter_number()),
        profile.default_colours(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let s = GameSession::new_for_machine(
        loaded.bytes().to_vec(),
        honor,
        false,
        false,
        Vec::new(),
        None,
        None,
        &boot,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", path.display()));
    assert!(!s.quit, "{}: quit during boot", path.display());
    Some(s)
}

fn header_release(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[2], b[3]])
}

/// The upper window as text, one line per row.
fn upper(s: &GameSession) -> String {
    let up = &s.machine.screen.upper;
    let mut out = String::new();
    for r in 0..up.rows as usize {
        let row: String =
            (0..up.cols as usize).map(|c| up.cells[r * up.cols as usize + c].ch).collect();
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

/// Drive *Beyond Zork* from boot into play, answering whatever it asks.
///
/// The reply is chosen from what the game just printed rather than from a fixed
/// script, because **which questions it asks is the thing under test**. `vt220`
/// is the answer given if it does ask. Lifted from `atari_st_profile.rs`, where
/// the same game is driven for the same reason.
fn into_play(s: &mut GameSession, vt220: &str) -> String {
    fn drain(s: &mut GameSession) -> String {
        let mut out = String::new();
        for _ in 0..24 {
            match s.pending_input() {
                InputKind::Line => break,
                InputKind::Char => out.push_str(&s.submit_char(13).transcript),
                InputKind::Event => out.push_str(&s.submit("").transcript),
            }
        }
        out
    }

    let mut all = drain(s);
    // `last` is deliberately the MOST RECENT turn only. Deciding from the whole
    // accumulated transcript keeps answering the VT220 question long after the
    // game has moved on to BEGIN/RESTORE/QUIT.
    let mut last = all.clone();
    for _ in 0..10 {
        let flat: String = last.split_whitespace().collect::<Vec<_>>().join(" ");
        let reply =
            if flat.contains("VT220") || flat.contains("YES or NO") { vt220 } else { "begin" };
        last = s.submit(reply).transcript;
        assert!(!s.quit, "Beyond Zork quit while being driven into play");
        let more = drain(s);
        last.push_str(&more);
        all.push_str(&last);
        if upper(s).contains('│') || upper(s).contains("EN:") {
            break;
        }
    }
    all
}

fn version_line(s: &mut GameSession) -> String {
    s.submit("version").transcript.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The CP437 remap is gated on `$1E == 6` and the Apple IIgs is 10, so none of
/// CP437's high-byte glyphs may appear. **This matters more here than on the
/// ST**, because 6 is exactly what a Version 6 ProDOS story gets when the row
/// declines — so these glyphs are the visible shape of the defect this quest
/// closes, not a hypothetical.
fn assert_no_cp437_mojibake(text: &str, who: &str) {
    for bad in ['♂', '♀', '♪', '☺', '☻', '♣'] {
        assert!(
            !text.contains(bad),
            "{who}: {bad:?} is a CP437 byte rendered as a glyph — the IBM PC remap fired on a \
             machine that is not the IBM PC. `exec.rs` gates it on `read_byte(0x1E) == 6`.",
        );
    }
}

// ── The evidence, checked against the disks ──────────────────────────────────

/// **The argument's foundation, pinned on the real media.** Infocom's own Apple
/// II YZIP decides the interpreter number by DETECTING the machine, and the
/// routine that does it is on both Version 6 ProDOS releases in the corpus.
///
/// This is what makes the family ambiguity a measured fact rather than a
/// cautious reading of ZMSD §11.1.3 — and, read the other way, what makes 10 a
/// choice this codebase is making rather than a number the disk stated. If this
/// ever stops matching, the whole argument at `blorb::medium`'s ProDOS row needs
/// re-reading, which is why it is a test and not a comment.
#[test]
fn the_shipped_apple_interpreter_still_detects_the_machine_at_boot() {
    // `apple/yzip/rel.13/boot.lst:1808-1826`, assembled 6502, with the one
    // `jmp MACH2` operand as the shipped build relocates it (`4C CC 26`).
    const MACHINE_ROUTINE: &[u8] = &[
        0xad, 0xb3, 0xfb, // lda MACHID1
        0xc9, 0x06, // cmp #6            — nothing below an enhanced ][e
        0xd0, 0x19, // bne BADMACH
        0xad, 0xc0, 0xfb, // lda MACHID2
        0xd0, 0x05, // bne MACH1
        0xa9, 0x09, // lda #IIcID        — 9, Apple IIc
        0x4c, 0xcc, 0x26, // jmp MACH2
        0x38, // sec
        0x20, 0x1f, 0xfe, // jsr MACHCHK
        0xb0, 0x04, // bcs OLDMACH
        0xa9, 0x0a, // lda #IIgsID       — 10, Apple IIgs
        0xd0, 0x02, // bne MACH2
        0xa9, 0x02, // lda #IIeID        — 2, Apple IIe
        0x85, 0x65, // sta ARG2+LO
        0x60, // rts
    ];

    let mut ran = 0;
    for name in V6_DISKS {
        let Some(path) = disk(name) else { continue };
        ran += 1;
        let raw = std::fs::read(&path).expect("readable");
        let pd = blorb::prodos::ProDos::mount(raw).expect("the ProDOS volume mounts");

        // The routine lives in `INFOCOM.SYSTEM`, the ProDOS 8 launcher — named
        // by path because Arthur nests its copy under `ARTHUR.1/`.
        let launcher = pd
            .files()
            .iter()
            .find(|e| e.path().to_uppercase().ends_with("INFOCOM.SYSTEM"))
            .unwrap_or_else(|| panic!("{name}: no INFOCOM.SYSTEM on this volume"));
        let bytes = pd.read(launcher).unwrap_or_else(|| panic!("{name}: unreadable launcher"));

        let at = bytes
            .windows(MACHINE_ROUTINE.len())
            .position(|w| w == MACHINE_ROUTINE)
            .unwrap_or_else(|| {
                panic!(
                    "{name}: Infocom's `MACHINE:` routine is not in {}. This suite's whole \
                     argument is that the Apple's interpreter number is DETECTED, not pressed \
                     — if the bytes moved, re-read `blorb::medium`'s ProDOS row.",
                    launcher.path(),
                )
            });
        assert_eq!(at, 1711, "{name}: the routine has moved within {}", launcher.path());

        // …and all three of the family's numbers really are in it, which is the
        // ambiguity stated as an assertion rather than as prose.
        for (id, machine) in [(2u8, "IIe"), (9, "IIc"), (10, "IIgs")] {
            assert!(
                MACHINE_ROUTINE.windows(2).any(|w| w == [0xa9, id]),
                "{name}: `lda #{id}` ({machine}) is not in the routine",
            );
        }
    }
    assert!(ran > 0 || !any_prodos_disk_present(), "ProDOS media are present but none were read");
}

// ── The number ───────────────────────────────────────────────────────────────

/// A ProDOS volume resolves to the Apple IIgs bundle, and the story is told 10.
/// The smallest statement of the change, on the real media.
#[test]
fn a_prodos_volume_tells_its_story_it_is_an_apple_iigs() {
    let mut ran = 0;
    for name in ALL_DISKS {
        let Some(path) = disk(name) else { continue };
        ran += 1;
        let profile = InterpreterProfile::resolve(&path, None, None, None);
        assert_eq!(profile, InterpreterProfile::AppleIIgs, "{name}: the medium picks the machine");
        assert_eq!(
            profile.interpreter_number(),
            Some(10),
            "{name}: ZMSD §11.1.3, and `IIgsID EQU 10 ; ][gs Yzip`",
        );
        // The declined member stays declined. The Apple's Version 6 screen is
        // 140×192 on a 3×9 cell — a different screen MODEL, not a picture space
        // this knob can hold.
        assert_eq!(profile.std_window(), None, "{name}: 140×192 on a 3×9 cell is not a std window");
        // A black page with white ink, out of the YZIP's own `zboot.asm`.
        assert_eq!(profile.default_colours(), Some((2, 9)), "{name}: black page, white ink");

        // `Journey.2mg` carries no whole story file, so it has no session to
        // boot — the profile is still the machine, which is what this pins.
        let Ok((loaded, _)) = app::hints::load_mounted_story(&path) else { continue };
        if loaded.bytes().is_empty() {
            continue;
        }
        for honor in [true, false] {
            let Some(s) = boot(&path, honor, None) else { continue };
            assert_eq!(
                s.machine.mem.read_byte(0x1E),
                10,
                "{name} honor_game_colours={honor}: header $1E as the story reads it",
            );
        }
    }
    assert!(ran > 0 || !any_prodos_disk_present(), "ProDOS media are present but none were read");
}

// ── What Beyond Zork does about it ───────────────────────────────────────────

/// **The deliverable.** Told 10, *Beyond Zork* stops asking whether the terminal
/// is a VT220 and draws its box-drawn UI unprompted; told 1 it asks, and a player
/// who answers NO gets a plain-ASCII fallback.
///
/// Driven on **both** ProDOS presses of the same build — the standalone GS/OS
/// disk and the *Lost Treasures* compilation — because the release is identical
/// and the volumes are not, so a result on one is not a result on the other.
///
/// Both `honor_game_colours` modes, per the project's colour convention: this
/// profile supplies default colours (black page, white ink), so this is a colour
/// area and a single-mode suite here would be the exact gap the convention
/// exists to close.
#[test]
fn beyond_zork_off_a_prodos_volume_is_never_asked_whether_it_is_a_vt220() {
    let mut ran = 0;
    for name in [BEYOND_ZORK_DISK, LOST_TREASURES_2] {
        let Some(path) = disk(name) else { continue };
        ran += 1;
        let (loaded, _) = app::hints::load_mounted_story(&path).expect("mounts");
        assert_eq!(
            (header_release(loaded.bytes()), String::from_utf8_lossy(&loaded.bytes()[0x12..0x18])),
            (BEYOND_ZORK_RELEASE, BEYOND_ZORK_SERIAL.into()),
            "{name} carries a different build of Beyond Zork than the suite was written against",
        );

        for honor in [true, false] {
            let who = format!("Beyond Zork r57 s871221 [{name}] honor_game_colours={honor}");

            // ── The medium's own answer: 10.
            let mut gs = boot(&path, honor, None).expect("boots");
            let gs_said = into_play(&mut gs, "no");
            assert!(
                !gs_said.contains("VT220"),
                "{who}: told it is an Apple IIgs, the game still asked {gs_said:?}",
            );
            let gs_screen = upper(&gs);
            assert!(
                gs_screen.contains('│') && gs_screen.contains('┌'),
                "{who}: the IIgs draws its box-drawn UI unprompted, and this frame is {gs_screen:?}",
            );
            assert_no_cp437_mojibake(&gs_said, &who);
            assert_no_cp437_mojibake(&gs_screen, &who);

            // …and the game names the machine **in Infocom's own spelling**,
            // which is the strongest form of this whole quest: not our header,
            // the STORY's reading of it. `Beyond Zork` answers VERSION with
            // "Apple //gs Color Version A" — the same shape as the "Atari ST
            // Color Version A" SQ-0835 pinned on the identical game, and as the
            // Macintosh Zork Zero r296 of SQ-0838.
            let gs_version = version_line(&mut gs);
            assert!(
                gs_version.contains("Apple //gs"),
                "{who}: VERSION answered {gs_version:?}",
            );
            assert!(
                !gs_version.contains("DEC-20"),
                "{who}: VERSION still names the DECSystem-20: {gs_version:?}",
            );
            // **The two colour modes genuinely differ here, and the game says
            // so.** Beyond Zork brands itself "…*Color* Version A" only while
            // the colour capability is advertised; with `honor_game_colours =
            // false` the same release off the same volume answers "Apple //gs
            // Version A". This is exactly the divergence the project's
            // both-modes convention exists to catch, so it is pinned rather than
            // tolerated by a loose match.
            assert_eq!(
                gs_version.contains("Apple //gs Color Version A"),
                honor,
                "{who}: the game brands itself Color only when colour is offered — {gs_version:?}",
            );

            // ── The falsification, driven rather than described: force 1 and
            // the question comes back, and answering NO gives the degraded UI.
            let mut dec = boot(&path, honor, Some(1)).expect("boots");
            let dec_said = into_play(&mut dec, "no");
            assert!(
                dec_said.contains("VT220"),
                "{who}: told it is a DECSystem-20, the game must ask about the terminal — it \
                 said {dec_said:?}. If this fails, the medium is no longer reaching the profile \
                 and the positive case above is passing for the wrong reason.",
            );
            assert!(
                !upper(&dec).contains('│'),
                "{who}: a DEC-20 that answered NO gets the plain-ASCII fallback",
            );
            assert!(
                version_line(&mut dec).contains("DEC-20"),
                "{who}: an explicit interpreter number must still outrank the medium (SQ-0839)",
            );
        }
    }
    assert!(ran > 0 || !any_prodos_disk_present(), "ProDOS media are present but none were read");
}

/// The same screen, reached two ways — so the change is legible as "the IIgs is
/// not asked" rather than "the IIgs draws something new".
///
/// Interpreter 1 with YES to the VT220 question produces the frame interpreter 10
/// produces with no question at all. Nothing in the render path moved.
#[test]
fn the_apple_frame_is_the_one_a_vt220_owner_already_had_to_ask_for() {
    let Some(path) = disk(BEYOND_ZORK_DISK) else { return };

    let mut gs = boot(&path, true, None).expect("boots");
    let _ = into_play(&mut gs, "no");

    let mut vt = boot(&path, true, Some(1)).expect("boots");
    let _ = into_play(&mut vt, "yes");

    assert_eq!(
        upper(&gs),
        upper(&vt),
        "the Apple IIgs's frame and a VT220-owning DEC-20's frame are the same frame; the profile \
         removes a question, it does not introduce a new rendering",
    );
}

// ── The rest of the corpus ───────────────────────────────────────────────────

/// A Version 3 story does not notice, and that is a property of Version 3 rather
/// than luck: header byte `$1E` has no meaning before Version 4.
///
/// Measured across the corpus rather than argued: twenty-four of the thirty-one
/// stories on the ten ProDOS images trace identically under the default rule and
/// under 10, and every Version 3 one is among them. This pins the disk that is
/// entirely v3.
#[test]
fn a_version_3_story_on_a_prodos_volume_is_unmoved_by_the_number() {
    let Some(path) = disk(V3_DISK) else { return };

    let (loaded, _) = app::hints::load_mounted_story(&path).expect("mounts");
    assert_eq!(loaded.bytes()[0], 3, "this disk's story is a Version 3 one");

    let mut told_10 = boot(&path, true, None).expect("boots");
    assert_eq!(told_10.machine.mem.read_byte(0x1E), 10, "the ProDOS volume still says 10");
    let mut told_1 = boot(&path, true, Some(1)).expect("boots");

    let a = told_10.submit("look").transcript;
    let b = told_1.submit("look").transcript;
    assert_eq!(
        a, b,
        "a Version 3 story must not observe header $1E — it carries no interpreter number",
    );
    assert_no_cp437_mojibake(&a, "a Version 3 ProDOS story");
}

// ── Every consumer agrees ────────────────────────────────────────────────────

/// **Guard 3, as a test rather than as a reading.** Whatever the row answers, the
/// TUI, `zvm-cli` and the launch-options dialog must advertise the SAME byte for
/// the same disk — which is exactly the half-wiring SQ-0836 declined `Some(10)`
/// to avoid, checked from the far side now that the Apple bundle exists.
///
/// All three reach the number by different routes: `startup.rs` asks the resolved
/// `InterpreterProfile`, `zvm-cli`'s `main.rs` asks `blorb::medium` directly off
/// the image bytes, and `launch_options::derived_interpreter` asks the row and
/// reports its provenance. This drives all three off the same file.
#[test]
fn the_tui_the_cli_and_the_launch_dialog_advertise_the_same_byte() {
    let mut ran = 0;
    for name in ALL_DISKS.iter().chain(V6_DISKS.iter()) {
        let Some(path) = disk(name) else { continue };
        ran += 1;
        let raw = std::fs::read(&path).expect("readable");

        // 1. `zvm-cli`'s route: the medium, straight off the bytes it read.
        let cli = blorb::medium::DiskImage::detect(&raw).and_then(|d| d.interpreter_number());
        assert_eq!(cli, Some(10), "{name}: zvm-cli reads the medium directly");

        // 2. The TUI's route: `InterpreterProfile::resolve` on the path opened.
        let tui = InterpreterProfile::resolve(&path, None, None, None).interpreter_number();
        assert_eq!(tui, cli, "{name}: the TUI and zvm-cli disagree about the same disk");

        // 3. The launch dialog's route, for every Z-version the corpus holds —
        // including 6, which is where a declining row and zvm's Frotz rule used
        // to diverge most sharply (the dialog would have said 6, the IBM PC).
        for version in [3u8, 4, 5, 6] {
            let derived = app::launch_options::derived_interpreter(
                None,
                None,
                Some(app::hints::DiskImage::ProDos),
                Some(version),
            );
            assert_eq!(
                derived.map(|(n, _)| n),
                Some(10),
                "{name}: the launch dialog advertises a different byte at v{version}",
            );
            assert_eq!(
                derived.map(|(_, src)| src),
                Some(app::launch_options::InterpreterSource::DiskImage),
                "{name}: …and must say the DISK is what settled it, at v{version}",
            );
        }

        // 4. …and an explicit number still outranks the medium on every route
        // (SQ-0839, and guard 4 of this quest's brief).
        // The number you name is the machine you get, and since SQ-0872 that is
        // literally true for the IIe: 2 now carries the family's own bundle
        // rather than falling through to the IBM PC. The volume still says IIgs
        // and is still outranked.
        assert_eq!(
            InterpreterProfile::resolve(&path, Some(2), None, None),
            InterpreterProfile::AppleIIe,
            "{name}: asking for the Apple IIe on a IIgs volume must get the IIe",
        );
        assert_eq!(
            InterpreterProfile::resolve(&path, Some(2), None, None).interpreter_number(),
            Some(2),
            "{name}: …and advertise 2, not the volume's 10",
        );
        assert_eq!(
            app::launch_options::derived_interpreter(
                Some(9),
                None,
                Some(app::hints::DiskImage::ProDos),
                Some(5),
            )
            .map(|(n, _)| n),
            Some(9),
            "{name}: an explicit IIc must outrank the volume",
        );
    }
    assert!(ran > 0 || !any_prodos_disk_present(), "ProDOS media are present but none were read");
}
