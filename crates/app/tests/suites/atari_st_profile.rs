//! SQ-0835, the profile half: an Atari ST floppy is an Atari ST, and *Beyond
//! Zork* can tell.
//!
//! The container half of that quest shipped `DiskImage::Fat12AtariSt` answering
//! `None` for its interpreter number, reasoning that no ST press of a graphical
//! v6 title exists, so ZMSD §11.1.3's 5 would be "a byte with no verified
//! machine behind it". The premise is true and the conclusion does not follow.
//! `app::interpreter`'s module doc warns against a number that CONTRADICTS the
//! rest of the machine — the real incident behind it was `interpreter_number =
//! 4` set by hand while the artwork stayed IBM PC — and that failure cannot
//! arise on a corpus with no artwork in it. All thirty-nine stories across the
//! nine ST compilations are v3, v4 or v5.
//!
//! ## What is actually observable, measured across the whole ST corpus
//!
//! Every story on all nine compilations was booted under interpreter 1 and 5 and
//! the traces diffed. Thirty-two are byte-identical. Six print the number in
//! their VERSION block and are otherwise unchanged (Sherlock, Border Zone, Nord
//! and Bert, Trinity — on two disks — and ZTUU). **One behaves differently, and
//! it is the interesting one:**
//!
//! *Beyond Zork* r49 s870917, off `Infocom Compilation 6`, told **1**:
//!
//! ```text
//!   Is this a VT220?
//!   [Please type YES or NO.] >
//! ```
//!
//! answer NO and the game draws a plain-ASCII fallback — no box, `\` and `@-`
//! for the compass rose, "Use the UP and DOWN arrow keys". Answer YES and it
//! draws the full box-drawn UI. Told **5**, it never asks at all and goes
//! straight to the box-drawn UI, because an Atari ST is not a terminal that
//! might or might not have line-drawing characters. Its VERSION block changes
//! from `DEC-20 Color Version A` to `Atari ST Color Version A` — the game naming
//! the machine, exactly as Zork Zero r296 names the Macintosh (SQ-0838).
//!
//! Note what does NOT change: interpreter 5's screen is identical to
//! interpreter 1's *after answering YES*. No new glyph path is involved. The ST
//! simply is not asked a question that only makes sense on a DECSystem-20.
//!
//! **CP437 is not involved either, and that is asserted rather than assumed.**
//! `zvm`'s `exec.rs` gates its CP437 remap on `read_byte(0x1E) == 6`, so the
//! ST's 5 does not take that path; the box drawing here is Font 3 resolved to
//! Unicode, and `assert_no_cp437_mojibake` pins that it stayed that way.
//!
//! The story files are gitignored (CLAUDE.md), so every case skips vacuously
//! when the compilations are absent.

use std::path::{Path, PathBuf};

use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

/// The compilation *Beyond Zork* is on, and which release it must be. Opening
/// this disk gives you Beyond Zork because `BEYZORK.T` is 262144 bytes against
/// ~85K for each of the three Zorks beside it.
const BEYOND_ZORK_DISK: &str = "Infocom Compilation 6 (19xx)(-).st";
const BEYOND_ZORK_RELEASE: u16 = 49;
const BEYOND_ZORK_SERIAL: &str = "870917";

/// A v3 compilation, for the negative: the ST's own Version 3 interpreter
/// leaves `$1E` zero and comments it "(UNUSED)", so a v3 story must not notice
/// the number at all.
const V3_DISK: &str = "Infocom Compilation 7 (19xx)(-).st";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Whether any ST floppy this suite knows about is on disk at all.
///
/// `stories/` is gitignored, so CI and a worktree without the symlink have NO
/// media and every case here must skip vacuously (CLAUDE.md's rule). A bare
/// `assert!(ran > 0)` broke that and failed CI on all three platforms. The guard
/// still earns its keep where it can: with the media present, "nothing ran"
/// means a filename drifted, and that must fail loudly rather than pass empty.
/// Same idiom as `real_media_releases::any_real_media_present`.
fn any_st_disk_present() -> bool {
    [BEYOND_ZORK_DISK, V3_DISK, "Infocom Compilation 8 (19xx)(-).st"]
        .iter()
        .any(|n| stories_dir().join(n).is_file())
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
        Some(blorb::medium::DiskImage::Fat12AtariSt),
        "{}: this suite's fixtures are Atari ST floppies",
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

/// The upper window as text, one line per row — where Beyond Zork draws its box,
/// its status line and its compass rose.
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

/// Drive Beyond Zork from boot into play, answering whatever it asks, and return
/// everything it said on the way.
///
/// The reply is chosen from what the game just printed rather than from a fixed
/// script, because **which questions it asks is the thing under test**: told 1
/// it opens with "Is this a VT220?", told 5 it opens with BEGIN/RESTORE/QUIT.
/// `vt220` is the answer given if it does ask.
fn into_play(s: &mut GameSession, vt220: &str) -> String {
    /// Answer every keypress/event prompt until the game wants a line again,
    /// returning what it said. Beyond Zork's prologue and its menus are pages of
    /// these, and they must not be mistaken for questions.
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
    // accumulated transcript instead keeps answering the VT220 question long
    // after the game has moved on to BEGIN/RESTORE/QUIT.
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

/// Whatever the game prints in answer to VERSION, whitespace-flattened.
fn version_line(s: &mut GameSession) -> String {
    s.submit("version").transcript.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The CP437 remap is gated on `$1E == 6` and the ST is 5, so none of CP437's
/// high-byte glyphs may appear. These are the ones that showed up in the wild
/// when the gate was wrong (Shogun's `♂` between sentences, SQ-0762 era).
fn assert_no_cp437_mojibake(text: &str, who: &str) {
    for bad in ['♂', '♀', '♪', '☺', '☻', '♣'] {
        assert!(
            !text.contains(bad),
            "{who}: {bad:?} is a CP437 byte rendered as a glyph — the IBM PC remap fired on a \
             machine that is not the IBM PC. `exec.rs` gates it on `read_byte(0x1E) == 6`.",
        );
    }
}

// ── The number ───────────────────────────────────────────────────────────────

/// An ST floppy resolves to the ST bundle, and the story is told 5. The
/// smallest statement of the change, on the real medium.
#[test]
fn an_atari_st_floppy_tells_its_story_it_is_an_atari_st() {
    let mut ran = 0;
    for name in [BEYOND_ZORK_DISK, V3_DISK, "Infocom Compilation 8 (19xx)(-).st"] {
        let Some(path) = disk(name) else { continue };
        ran += 1;
        let profile = InterpreterProfile::resolve(&path, None, None, None);
        assert_eq!(profile, InterpreterProfile::AtariSt, "{name}: the medium picks the machine");
        assert_eq!(
            profile.interpreter_number(),
            Some(5),
            "{name}: ZMSD §11.1.3, and `INTWRD DC.B 5 * MACHINE ID FOR ATARI ST`",
        );
        // The declined member stays declined: no ST YZIP ever existed.
        assert_eq!(profile.std_window(), None, "{name}: the ST has no Version 6 art geometry");

        for honor in [true, false] {
            let Some(s) = boot(&path, honor, None) else { continue };
            assert_eq!(
                s.machine.mem.read_byte(0x1E),
                5,
                "{name} honor_game_colours={honor}: header $1E as the story reads it",
            );
        }
    }
    assert!(ran > 0 || !any_st_disk_present(), "ST media are present but none were read");
}

// ── What Beyond Zork does about it ───────────────────────────────────────────

/// **The deliverable.** Told 5, *Beyond Zork* stops asking whether the terminal
/// is a VT220 and draws its box-drawn UI unprompted; told 1 it asks, and a
/// player who answers NO gets a plain-ASCII fallback.
///
/// Both `honor_game_colours` modes, per the project's colour convention — the
/// ST profile now supplies default colours (white page, black ink), so this area
/// is a colour area and a single-mode suite here would be the exact gap the
/// convention exists to close.
#[test]
fn beyond_zork_off_an_st_floppy_is_never_asked_whether_it_is_a_vt220() {
    let Some(path) = disk(BEYOND_ZORK_DISK) else { return };
    let (loaded, _) = app::hints::load_mounted_story(&path).expect("mounts");
    assert_eq!(
        (header_release(loaded.bytes()), String::from_utf8_lossy(&loaded.bytes()[0x12..0x18])),
        (BEYOND_ZORK_RELEASE, BEYOND_ZORK_SERIAL.into()),
        "this disk carries a different build of Beyond Zork than the suite was written against",
    );

    for honor in [true, false] {
        let who = format!("Beyond Zork r49 s870917 (Atari ST) honor_game_colours={honor}");

        // ── The medium's own answer: 5.
        let mut st = boot(&path, honor, None).expect("boots");
        let st_said = into_play(&mut st, "no");
        assert!(
            !st_said.contains("VT220"),
            "{who}: told it is an Atari ST, the game still asked {st_said:?}",
        );
        let st_screen = upper(&st);
        assert!(
            st_screen.contains('│') && st_screen.contains('┌'),
            "{who}: the ST draws its box-drawn UI unprompted, and this frame is {st_screen:?}",
        );
        assert_no_cp437_mojibake(&st_said, &who);
        assert_no_cp437_mojibake(&st_screen, &who);

        // …and the game names the machine, which is the strongest form of this
        // whole quest: not our header, the STORY's reading of it.
        let st_version = version_line(&mut st);
        assert!(
            st_version.contains("Atari ST") && st_version.contains("Version A"),
            "{who}: VERSION answered {st_version:?}",
        );
        assert!(
            !st_version.contains("DEC-20"),
            "{who}: VERSION still names the DECSystem-20: {st_version:?}",
        );
        // **The two colour modes genuinely differ here, and the game says so.**
        // Beyond Zork brands itself "Atari ST *Color* Version A" only while the
        // colour capability is advertised; with `honor_game_colours = false` the
        // same release off the same disk answers "Atari ST Version A". This is
        // exactly the divergence the project's both-modes convention exists to
        // catch, so it is pinned rather than tolerated by a loose match.
        assert_eq!(
            st_version.contains("Atari ST Color Version A"),
            honor,
            "{who}: the game brands itself Color only when colour is offered — {st_version:?}",
        );
        // Version A is itself corroboration: Infocom's ST XZIP — the Version 5
        // interpreter, the one that ran this game — is stamped "FROZEN Version
        // A" in `st/xzip.c`'s modification history.
        assert!(
            !st_version.contains("Version B"),
            "{who}: the ST XZIP was frozen at Version A: {st_version:?}",
        );

        // ── The falsification, driven rather than described: force 1 and the
        // question comes back, and answering NO gives the degraded UI.
        let mut dec = boot(&path, honor, Some(1)).expect("boots");
        let dec_said = into_play(&mut dec, "no");
        assert!(
            dec_said.contains("VT220"),
            "{who}: told it is a DECSystem-20, the game must ask about the terminal — it said \
             {dec_said:?}. If this fails, the medium is no longer reaching the profile and the \
             positive case above is passing for the wrong reason.",
        );
        let dec_screen = upper(&dec);
        assert!(
            !dec_screen.contains('│'),
            "{who}: a DEC-20 that answered NO gets the plain-ASCII fallback, not {dec_screen:?}",
        );
        assert!(
            version_line(&mut dec).contains("DEC-20"),
            "{who}: an explicit interpreter number must still outrank the medium (SQ-0839)",
        );
    }
}

/// The same screen, reached two ways — so the change is legible as "the ST is
/// not asked" rather than "the ST draws something new".
///
/// Interpreter 1 with YES to the VT220 question produces the frame interpreter 5
/// produces with no question at all. Nothing in the render path moved.
#[test]
fn the_st_frame_is_the_one_a_vt220_owner_already_had_to_ask_for() {
    let Some(path) = disk(BEYOND_ZORK_DISK) else { return };

    let mut st = boot(&path, true, None).expect("boots");
    let _ = into_play(&mut st, "no");

    let mut vt = boot(&path, true, Some(1)).expect("boots");
    let _ = into_play(&mut vt, "yes");

    assert_eq!(
        upper(&st),
        upper(&vt),
        "the Atari ST's frame and a VT220-owning DEC-20's frame are the same frame; the profile \
         removes a question, it does not introduce a new rendering",
    );
}

// ── The rest of the corpus ───────────────────────────────────────────────────

/// A Version 3 story does not notice, and that is a property of Version 3 rather
/// than luck: byte `$1E` has no meaning before Version 4, which is why the ST's
/// own v3 interpreter leaves it zero — `st/stzip.s`, `INTWRD DC.B 0 * (UNUSED)`
/// under `IFEQ CZIP`, against `DC.B 5` under `IFEQ EZIP`.
///
/// Measured across the corpus rather than argued: all thirty-two v3 stories on
/// the nine compilations trace identically under 1 and under 5. This pins the
/// disk that is entirely v3.
#[test]
fn a_version_3_story_on_an_st_floppy_is_unmoved_by_the_number() {
    let Some(path) = disk(V3_DISK) else { return };

    let (loaded, _) = app::hints::load_mounted_story(&path).expect("mounts");
    assert_eq!(loaded.bytes()[0], 3, "this disk's story is a Version 3 one");

    let mut told_5 = boot(&path, true, None).expect("boots");
    assert_eq!(told_5.machine.mem.read_byte(0x1E), 5, "the ST floppy still says 5");
    let mut told_1 = boot(&path, true, Some(1)).expect("boots");

    // Same opening, same first turn, either way.
    let a = told_5.submit("look").transcript;
    let b = told_1.submit("look").transcript;
    assert_eq!(
        a, b,
        "a Version 3 story must not observe header $1E — it carries no interpreter number",
    );
    assert_no_cp437_mojibake(&a, "a Version 3 ST story");
}
