//! SQ-0838 (second half): the Macintosh interpreter profile, and the screen
//! model that comes with it.
//!
//! The first half taught `InfocomPics` to read the monochrome archive on
//! `stories/Zork Zero Disk.image` — **Zork Zero v6 release 296, serial 881019**.
//! This half answers the question that archive raised: *what machine is this,
//! and how big is its screen?*
//!
//! # Everything here is quoted from Infocom's own Macintosh interpreter
//!
//! `mac/xzip.lst` and `mac/gfx.p` in `github.com/erkyrath/infocom-zcode-terps`,
//! the sources already cited throughout `blorb::infocom_pics`. The Mac sizes its
//! window and picks its picture file in ONE decision:
//!
//! ```text
//!   GFXAM_Y  = 200;  GFXAM_X  = 320;   { "raw" size of full-screen Amiga pics }
//!   GFXMAC_Y = 300;  GFXMAC_X = 480;   { 1.5 x Amiga sizes }
//!   …
//!   IF ((ydisplay < 2*GFXAM_Y) OR (xdisplay < 2*GFXAM_X))
//!     OR ((mColor = FALSE) OR (ttyToggle)) THEN
//!     BEGIN  myBig := FALSE;  wy := GFXMAC_Y;  wx := GFXMAC_X;  END
//!   ELSE
//!     BEGIN  myBig := TRUE;   wy := 2*GFXAM_Y; wx := 2*GFXAM_X; END
//!   …
//!   IF myBig THEN gfxName := 'CPic.Data' ELSE gfxName := 'Pic.Data';
//! ```
//!
//! and the screen the STORY is told about is that window, in pixels:
//!
//! ```text
//!   { calculate our logical screen size, based on window size }
//!     WITH myWindow^.portRect DO
//!       BEGIN
//!       totRows := (bottom - top) {DIV lineheight};
//!       totCols := ((right - left) - (2 * wMarg)) {DIV colWidth};
//!       END;
//! ```
//!
//! So there are **two Macintosh screens**, and the artwork picks:
//!
//! | archive          | picture space | display scale | screen  |
//! |------------------|---------------|---------------|---------|
//! | `CPic.data`      | 320×200       | 2× (`myBig`)  | 640×400 |
//! | `Pic.data` (b&w) | 480×300       | 1× (`ge.mono`)| 480×300 |
//!
//! **512×342 is the hardware and never the standard window.** The compact Mac's
//! screen appears in the interpreter only as `screenRect := screenBits.bounds`,
//! the thing the window is centred inside — `MoveWindow (myWindow, …,
//! (ydisplay-wy) DIV 2 + 17 {mbar fudge}, TRUE)` puts a 300-pixel content rect
//! at y = 38 on a 342-pixel screen, which is exactly a 20-pixel menu bar plus a
//! 19-pixel title bar. A photograph of a real Mac Zork Zero is therefore 512×342
//! with the game filling nearly all of it, while the game itself is being told
//! 480×300.
//!
//! **Neither path has non-square pixels.** Every scaling arm in `gfx.p` moves
//! both axes by the same factor — `ge.dy := ge.ry * 2; ge.dx := ge.rx * 2`,
//! `(ge.ry * 3) DIV 2` with `(ge.rx * 3) DIV 2`, or 1× for both — unlike the
//! IBM PC's EGA rendition, whose pixels really are half as wide (SQ-0790). Both
//! Mac archives are 1.6:1 and stay 1.6:1.

use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};
use std::path::PathBuf;

const RELEASE: u16 = 296;
const SERIAL: &[u8] = b"881019";

fn mac_disk() -> Option<PathBuf> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/Zork Zero Disk.image");
    if !path.exists() {
        eprintln!("SKIP: gitignored Macintosh medium missing at {}", path.display());
        return None;
    }
    Some(path)
}

/// What a launch resolved, so a failure can say which link decided.
struct Launch {
    session: GameSession,
    std_window: Option<(u16, u16)>,
    art_scale: Option<(u32, u32)>,
    monochrome: bool,
    /// Whether the game's colours were honoured once the archive had had its
    /// say — `startup.rs`'s answer, not the caller's request (SQ-0846).
    honoured: bool,
}

/// Boot the disk exactly as `startup.rs` does, optionally naming an archive by
/// hand (`--pictures`) and optionally overriding the interpreter number.
///
/// `honor_game_colours` is the value the CONFIG carries into the launch; the
/// value the session gets is that one filtered through
/// [`PictSource::declines_game_colours`], because `startup.rs` runs the same
/// filter and SQ-0846 is a defect about exactly which side of it the Macintosh
/// falls on. `Launch::honoured` is what actually reached the session.
fn launch(pictures: Option<&str>, honor_game_colours: bool, explicit: Option<u8>) -> Launch {
    let path = mac_disk().expect("caller checked the fixture is present");
    let bytes = match app::hints::load_story(&path).expect("Story.data mounts") {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code, got {other:?}"),
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), RELEASE, "this disk carries r{RELEASE}");
    assert_eq!(&bytes[0x12..0x18], SERIAL);

    let dir = app::scratch_dir("mac-profile");
    let over = match pictures {
        Some(name) => PictureOverride::resolve_with_session(&path, &dir, Some(name)),
        None => PictureOverride::Unset,
    };
    let named_art_std_window = over.std_window();
    let profile = InterpreterProfile::resolve(&path, explicit, over.flavour(), None);
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let picture_dims = picts.all_pict_dims();
    // The four links, in `startup.rs`'s order.
    let monochrome = picts.is_monochrome();
    // SQ-0806/SQ-0846: two-colour artwork declares the interpreter colourless —
    // but only where the machine has no colours of its own to declare.
    let honoured = honor_game_colours
        && !picts.declines_game_colours(profile.default_colours());
    let default_colours = honoured.then(|| profile.default_colours()).flatten();
    // SQ-1021/SQ-1022: every per-machine fact in one value — including the 7x15
    // cell this suite exists to exercise, which no longer has to be remembered
    // separately from the four screen links beside it.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        named_art_std_window,
        explicit.or_else(|| profile.interpreter_number()),
        default_colours,
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut session =
        GameSession::new_for_machine(bytes, honoured, false, false, picture_dims, None, None, &boot)
    .expect("Zork Zero boots off the Macintosh disk");
    assert!(!session.quit, "quit during boot");
    assert!(session.machine.fault_trace.is_none(), "faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = std::fs::remove_dir_all(&dir);
    Launch { session, std_window: boot.screen_px, art_scale: boot.art_scale, monochrome, honoured }
}

/// Ask the game for its VERSION line, which is where Zork Zero names the machine
/// it thinks it is on.
fn version_line(s: &mut GameSession) -> String {
    let mut said = String::new();
    for _ in 0..24 {
        match s.pending_input() {
            InputKind::Line => {
                said = s.submit("version").transcript;
                break;
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }
    said.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── The machine ──────────────────────────────────────────────────────────────

/// **The acceptance criterion, in the game's own words.** A profile is a claim
/// about the machine, and header `$1E` is not inert — so the test is not that
/// `resolve` returns a variant, it is that *Zork Zero release 296 reads the byte
/// and says what it read*.
///
/// FALSIFIED IN-LINE by the second half: ask for interpreter 6 on the same disk
/// and the same game says "IBM Interpreter" instead. Before SQ-0838 an HFS
/// volume resolved to the IBM PC default, so the first half of this test is
/// exactly the behaviour that changed.
#[test]
fn zork_zero_off_the_macintosh_disk_says_it_is_on_a_macintosh() {
    if mac_disk().is_none() {
        return;
    }
    let mut mac = launch(None, true, None);
    assert_eq!(mac.session.machine.mem.read_byte(0x1E), 3, "header $1E — ZMSD §11.1.3, 3 = Mac");
    let said = version_line(&mut mac.session);
    assert!(
        said.contains("Macintosh Interpreter version"),
        "the game answered VERSION with {said:?}, which does not name the Macintosh"
    );
    assert!(said.contains("Release 296 / Serial number 881019"), "{said:?}");

    // …and the medium is a DEFAULT, never an override (SQ-0839's other half).
    let mut pc = launch(None, true, Some(6));
    assert_eq!(pc.session.machine.mem.read_byte(0x1E), 6, "the explicit number wins");
    let said = version_line(&mut pc.session);
    assert!(
        !said.contains("Macintosh Interpreter"),
        "asked for interpreter 6 off a Macintosh disk and the game still said {said:?}"
    );
}

/// **Naming one of the disk's own archives must not change what machine this
/// is** (SQ-0843).
///
/// The launch-options dialog now lists both archives on this volume, so the
/// once-theoretical case is a row a person clicks. Until SQ-0843,
/// `InterpreterProfile::resolve` returned on the named archive's flavour before
/// the medium was ever consulted — and `Flavour::AmigaMac` is one container
/// written by two machines, so picking `CPic.data`, the very archive this disk
/// loads on its own, made Zork Zero announce itself on an **Amiga**. The medium
/// is what knows (`for_art_flavour`'s own doc comment said so all along), and
/// this is the acceptance criterion in the game's own words.
///
/// Pinned in both `honor_game_colours` modes: header `$1E` is not a colour and
/// must not move with one, and a two-colour archive has a say in that flag at
/// startup, which is exactly the kind of interaction a single-mode suite has
/// masked before. (On this machine it no longer gets one — SQ-0846, and
/// `the_macintoshs_own_archive_no_longer_declines_its_own_colours` below.)
#[test]
fn naming_either_of_the_disks_archives_still_boots_a_macintosh() {
    if mac_disk().is_none() {
        return;
    }
    for pictures in ["CPic.data", "Pic.data"] {
        for honour in [true, false] {
            let mut l = launch(Some(pictures), honour, None);
            assert_eq!(
                l.session.machine.mem.read_byte(0x1E),
                3,
                "--pictures {pictures} (honour={honour}) must leave header $1E a Macintosh",
            );
            let said = version_line(&mut l.session);
            assert!(
                said.contains("Macintosh Interpreter version"),
                "picked {pictures} off the Macintosh disk and the game said {said:?}",
            );
            assert!(said.contains("Release 296 / Serial number 881019"), "{said:?}");
        }
    }
    // …and the archive still decides the SCREEN, which is the other half: one
    // disk, one machine, two screens (see the test below).
    assert!(launch(Some("Pic.data"), true, None).monochrome);
    assert!(!launch(Some("CPic.data"), true, None).monochrome);
}

/// The rest of the bundle reaches the header: a white page and black ink, which
/// is the Macintosh's whole visual signature and the exact opposite of the
/// Amiga's dark grey.
///
/// Source: `mac/xzip.lst`'s `SetColor := (zWHITE*256) + zBLACK; { Mac defaults:
/// white under black }`, with `zWHITE = 9` and `zBLACK = 2`.
///
/// Pinned in BOTH `honor_game_colours` modes, per the project's colour
/// convention: `true` is the shipped default and the machine's own pair, `false`
/// hands the screen back to the user's theme and the profile must not smuggle
/// its colours in behind that switch.
#[test]
fn the_macintosh_page_is_white_and_only_when_game_colours_are_honoured() {
    if mac_disk().is_none() {
        return;
    }
    let honoured = launch(None, true, None);
    assert_eq!(honoured.session.machine.mem.read_byte(0x2C), 9, "header $2C background: white");
    assert_eq!(honoured.session.machine.mem.read_byte(0x2D), 2, "header $2D foreground: black");

    let themed = launch(None, false, None);
    let (bg, fg) = (
        themed.session.machine.mem.read_byte(0x2C),
        themed.session.machine.mem.read_byte(0x2D),
    );
    assert!(
        (bg, fg) != (9, 2),
        "honor_game_colours=false must hand the page back to the theme, got ({bg}, {fg})"
    );
}

// ── The screen, which is the interesting half ────────────────────────────────

/// **The two Macintosh screens, on the real disk.** The colour archive is the
/// big Mac's — 320×200 doubled, the Amiga's own unit space — and the monochrome
/// archive is the standard Mac's 480×300, which does not double at all.
///
/// FALSIFY by restoring `native_std_window`'s old fixed `INFOCOM_V6_STD_WINDOW`:
/// the monochrome launch is then told its screen is 640×400 while its artwork is
/// 480×300, which is the condition SQ-0841's missing right pillar was reported
/// under.
#[test]
fn the_archive_in_hand_picks_which_macintosh_screen_the_game_is_told_about() {
    if mac_disk().is_none() {
        return;
    }

    let colour = launch(None, true, None);
    assert!(!colour.monochrome, "the disk's default archive is the colour one");
    assert_eq!(colour.std_window, Some((320, 200)), "CPic.data's picture space");
    assert_eq!(colour.art_scale, Some((2, 2)), "wx := 2*GFXAM_X — doubled, as on the Amiga");
    assert_eq!(colour.session.machine.mem.read_word(0x22), 640, "screen width, header $22");
    assert_eq!(colour.session.machine.mem.read_word(0x24), 400, "screen height, header $24");
    // The grid is a QUOTIENT of that window, on the Macintosh's own 7x15 cell —
    // `totCols := ((right - left) - (2 * wMarg)) DIV colWidth` with `colWidth := 7`,
    // and `totRows := (bottom - top) DIV lineheight` with `lineheight := 15`.
    assert_eq!(colour.session.machine.mem.read_byte(0x21), 91, "columns, 640/7");
    assert_eq!(colour.session.machine.mem.read_byte(0x20), 26, "rows, 400/15");
    assert_eq!(colour.session.machine.mem.read_byte(0x27), 7, "$27 = font WIDTH in v6");
    assert_eq!(colour.session.machine.mem.read_byte(0x26), 15, "$26 = font HEIGHT in v6");

    let mono = launch(Some("Pic.data"), true, None);
    assert!(mono.monochrome, "--pictures Pic.data selects the two-colour archive");
    // Both doors give the same screen: naming the archive by hand (the link
    // that actually fires here) and mounting it off the volume it shipped on.
    // A mono-only Mac disk would reach the second, and must not land elsewhere.
    let mounted = PictSource::resolve_with_override(
        &mac_disk().expect("fixture"),
        PictureOverride::resolve_with_session(
            &mac_disk().expect("fixture"),
            &std::env::temp_dir(),
            Some("Pic.data"),
        ),
        None,
    );
    assert_eq!(mounted.native_std_window(), Some((480, 300)), "the archive's own picture space");
    assert_eq!(mounted.art_scale(), Some((1, 1)));
    assert_eq!(mono.std_window, Some((480, 300)), "GFXMAC_X/GFXMAC_Y — '1.5 x Amiga sizes'");
    assert_eq!(mono.art_scale, Some((1, 1)), "IF ge.mono OR myTiny THEN scale 1x for display");
    assert_eq!(mono.session.machine.mem.read_word(0x22), 480, "screen width, header $22");

    // **The pixel screen is now exact on BOTH axes**, which is the half of SQ-0917
    // that did land: `write_screen_dims_px` carries the window verbatim instead of
    // deriving it from the character grid, so 300 is 300. It used to read 304 —
    // the screen rounded up to a whole 16-pixel cell so Zork Zero's own 300-pixel
    // plate was never clipped — and that compensation is gone with the round trip
    // that caused it.
    assert_eq!(mono.session.machine.mem.read_word(0x24), 300, "screen height, header $24 — exact");
    // 300 divides EXACTLY by the Macintosh's 15-pixel cell — 20 rows, which is what
    // a real Mac fitted Geneva into. 480 does not divide by 7: it is 68 columns with
    // four pixels over, and the grid TRUNCATES because a partial cell at the edge is
    // not a character. Those four pixels stay in `$22`, which is the window and not
    // the grid — the round trip that used to lose them gave `68 * 7 = 476`.
    assert_eq!(mono.session.machine.mem.read_byte(0x20), 20, "rows, 300/15 exact");
    assert_eq!(mono.session.machine.mem.read_byte(0x21), 68, "columns, 480/7 truncated");
    assert_eq!(
        u16::from(mono.session.machine.mem.read_byte(0x21))
            * u16::from(mono.session.machine.mem.read_byte(0x27)),
        476,
        "the grid times the cell is 476 — exactly why $22 may not be derived from it",
    );
    assert_eq!(mono.session.machine.mem.read_byte(0x27), 7, "$27 = font WIDTH in v6");
    assert_eq!(mono.session.machine.mem.read_byte(0x26), 15, "$26 = font HEIGHT in v6");
    assert!(
        mono.session.machine.mem.read_word(0x24) >= 300,
        "the screen must contain the 480×300 plate, never clip it"
    );

    // The two screens really are different screens, which is the whole finding.
    assert_ne!(
        (colour.session.machine.mem.read_word(0x22), colour.session.machine.mem.read_word(0x24)),
        (mono.session.machine.mem.read_word(0x22), mono.session.machine.mem.read_word(0x24)),
    );
}

/// **Window property 13 carries the same cell the header does** (SQ-0917).
///
/// ZMSD §8.8.3.2 property 13 is a window's font size, packed `(height << 8) |
/// width`, and a story may ask for it with `@get_wind_prop` instead of reading
/// `$26`/`$27`. Two channels, one fact — so they have to agree.
///
/// This is the case that made `Machine::set_v6_cell` a setter rather than a public
/// field. `boot_state_and_screen` seeds every window's `font_size` while the
/// `Machine` is being built, when no profile has been declared yet, and
/// `set_screen_dims` never touches it afterwards. A cell assigned to a bare field
/// would have left a Macintosh story reading **8x16 from property 13 while the
/// header said 7x15** — a disagreement no existing case could see, because
/// nothing in the corpus asked property 13 of a machine whose cell was not the
/// default.
///
/// Falsified by making `set_v6_cell` assign `self.v6_cell` and nothing else,
/// which fails here naming both numbers.
#[test]
fn window_font_size_agrees_with_the_header_cell() {
    if mac_disk().is_none() {
        return;
    }
    for (label, l) in [("colour", launch(None, true, None)), ("mono", launch(Some("Pic.data"), true, None))] {
        let mem = &l.session.machine.mem;
        let (w, h) = (mem.read_byte(0x27), mem.read_byte(0x26));
        // Deliberately NOT `(7, 15)`: the invariant is that the two channels agree,
        // whatever the cell is, so this case keeps working while `v6_font_cell`'s
        // Macintosh arm is held at the default (SQ-0917) and needs no edit when it
        // moves. Pinning the literal here would make this a second test of the
        // profile rather than a test of the agreement.
        assert!(w > 0 && h > 0, "{label}: the header states a cell at all");
        let packed = (u16::from(h) << 8) | u16::from(w);
        let v6 = l.session.machine.screen.v6.as_ref().expect("a v6 story has a window table");
        for (i, win) in v6.windows.iter().enumerate() {
            assert_eq!(
                win.font_size, packed,
                "{label}: window {i} property 13 must be the header's cell, not zvm's default",
            );
        }
    }
}

/// The monochrome plate lands 1:1 on that screen, and the colour plate lands
/// doubled on its own — so in BOTH cases the game's picture table and its screen
/// are in the same space, which is what a v6 game's layout arithmetic assumes.
///
/// This is the invariant SQ-0736 was reported for, generalised: it used to be
/// "art doubles", which was true of every rendition Infocom shipped except one.
#[test]
fn every_macintosh_plate_and_its_screen_are_in_the_same_space() {
    if mac_disk().is_none() {
        return;
    }
    for (name, pictures, space) in
        [("colour", None, (320u16, 200u16)), ("mono", Some("Pic.data"), (480, 300))]
    {
        let l = launch(pictures, true, None);
        let screen = (
            l.session.machine.mem.read_word(0x22),
            l.session.machine.mem.read_word(0x24),
        );
        let (sx, sy) = l.art_scale.expect("a native archive states its density");
        // A full-screen plate, scaled, is the screen — to within the cell
        // rounding the 8×16 grid imposes on the 300-pixel one.
        let drawn = (u32::from(space.0) * sx, u32::from(space.1) * sy);
        assert_eq!(drawn.0, u32::from(screen.0), "{name}: width");
        assert!(
            u32::from(screen.1) >= drawn.1 && u32::from(screen.1) - drawn.1 < 16,
            "{name}: a {drawn:?} plate on a {screen:?} screen is off by more than one cell",
        );
        // …and the game's own picture table is in that same space: picture 1 is
        // the full-screen plate on both archives.
        let first = l
            .session
            .machine
            .picture_dims
            .iter()
            .find(|(n, _, _)| *n == 1)
            .copied()
            .expect("picture 1 is in the table");
        assert_eq!(
            (u32::from(first.1), u32::from(first.2)),
            drawn,
            "{name}: picture_data must report unit-space sizes",
        );
    }
}

// ── The colours, on screen ───────────────────────────────────────────────────

/// **The archive may not talk the Macintosh out of its own colours** (SQ-0846,
/// restated by SQ-0956).
///
/// SQ-0806 reads a two-colour archive as evidence that the interpreter has no
/// colours to offer, and bocfel's own note on the flag is *"the flags always
/// seem to equal 0xe if the graphics are monochrome"* — a heuristic, doing the
/// best that can be done with an archive alone.
///
/// A Macintosh is not that case. The medium named the machine (an HFS volume is
/// Apple's and nobody else's), and the machine's own interpreter named its
/// colours — `mac/xzip.lst`'s `SetColor := (zWHITE*256) + zBLACK`. That same
/// interpreter picked `Pic.Data` *for* that white page, in one decision. So the
/// guess stands down in front of the fact.
///
/// **And SQ-0956 widened it to every machine**, which is why the middle assertion
/// below now reads the other way. SQ-0846's rule spared the Mac because the Mac's
/// own screen IS a white page under black ink; SQ-0928 then gave the IBM PC a
/// screen too, and SQ-0956 gave the CGA card inside it one of its own — black
/// under light grey, `Palette::IbmCga`. A licensed machine of either kind keeps
/// its colours, so what is left of SQ-0806 is the launch that named NO machine,
/// asserted last.
///
/// **The Mac is still the case that must not move**, and it does not: the mono
/// archive keeps its colours under the Macintosh, exactly as SQ-0846 left it, and
/// it is not a two-colour CARD — `PictSource::two_colour_card` says no for it and
/// yes for a `.cg1`, which is the container telling the two machines apart.
#[test]
fn the_macintoshs_own_archive_no_longer_declines_its_own_colours() {
    if mac_disk().is_none() {
        return;
    }
    let path = mac_disk().expect("fixture");
    let dir = std::env::temp_dir().join(format!("lanthorn-mac-declines-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let mac = InterpreterProfile::Macintosh;
    let pc = InterpreterProfile::IbmPc;
    assert_eq!(
        (mac.default_colours(), mac.two_colour_colours()),
        (Some((9, 2)), Some((9, 2))),
        "the Macintosh states its two-colour page once, not twice",
    );
    assert_eq!(
        (pc.default_colours(), pc.two_colour_colours()),
        (Some((6, 9)), Some((2, 9))),
        "the IBM PC's card is not the IBM PC: one channel, blue 6 to black 2",
    );
    for (archive, mono) in [("Pic.data", true), ("CPic.data", false)] {
        let over = PictureOverride::resolve_with_session(&path, &dir, Some(archive));
        let picts = PictSource::resolve_with_override(&path, over, None);
        assert_eq!(picts.is_monochrome(), mono, "{archive}: the archive's own EF_MONO flags");
        assert!(
            !picts.declines_game_colours(mac.default_colours()),
            "{archive}: a machine whose own screen IS this display keeps its colours",
        );
        assert!(
            // SQ-0956: and on the IBM PC too, where the card states the screen
            // instead. Declining there was the reported defect.
            !picts.declines_game_colours(pc.default_colours()),
            "{archive}: a licensed machine of either kind keeps the story's colours",
        );
        assert!(
            // …and this Mac archive is not a CGA card whatever machine is asked,
            // because the CONTAINER is what names the card (SQ-0956).
            !picts.two_colour_card(),
            "{archive}: an Amiga/Mac container is never a video card",
        );
        assert_eq!(
            // SQ-0928: `IbmPc` used to BE "a machine with no defaults" and is not
            // any more — it states blue under white. What has no defaults is a
            // LAUNCH that never named a machine, and that is all SQ-0806's rule
            // has left to answer.
            picts.declines_game_colours(None),
            mono,
            "{archive}: the same bytes with no machine named — SQ-0806, unmoved",
        );
    }
    let _ = std::fs::remove_dir_all(&dir);

    // …and the launch chain agrees, which is the half `startup.rs` runs.
    assert!(launch(Some("Pic.data"), true, None).honoured, "the mono disk keeps its colours");
    assert!(
        !launch(Some("Pic.data"), false, None).honoured,
        "and the user's own switch still turns them off",
    );
}

/// **The Macintosh's page reaches the SCREEN MODEL, not just header `$2C`.**
///
/// This is SQ-0740's Amiga finding one machine later, and it was found the same
/// way — by the profile being invisible. Zork Zero release 296 **never calls
/// `set_colour` at all** on the Macintosh (measured: every window stays
/// [`ZColour::Default`], on both of the disk's archives and in both colour
/// modes), so with nothing painting the `$2C`/`$2D` pair the whole screen fell
/// through to the host theme and a Macintosh rendered exactly like an IBM PC.
///
/// Pinned in BOTH `honor_game_colours` modes, per the project's colour
/// convention. The `false` half is the load-bearing one here: a machine's own
/// page is still a game colour, and the switch that declines them must decline
/// this too — it does so at the source, since a colourless interpreter is never
/// handed the profile's pair to publish and the header then carries zvm's own
/// §8.3.2 seed, which is nobody's machine.
#[test]
fn the_macintosh_screen_model_carries_the_machines_white_page() {
    if mac_disk().is_none() {
        return;
    }
    use app::engine::Engine as _;
    use zvm::screen::ZColour;

    for archive in [None, Some("Pic.data"), Some("CPic.data")] {
        let honoured = launch(archive, true, None);
        assert!(honoured.honoured, "{archive:?}: colours are honoured on this machine");
        let v6 = honoured.session.machine.screen.v6.as_ref().expect("v6 window table");
        assert!(
            v6.windows.iter().all(|w| (w.fg, w.bg) == (ZColour::Default, ZColour::Default)),
            "{archive:?}: Zork Zero names no colour on the Mac — the machine's pair is all there is",
        );
        let model = honoured.session.screen();
        assert_eq!(
            (app::state::unpack_zcolour(model.fg), app::state::unpack_zcolour(model.bg)),
            (
                ZColour::Standard(app::interpreter::MAC_DEFAULT_FOREGROUND),
                ZColour::Standard(app::interpreter::MAC_DEFAULT_BACKGROUND)
            ),
            "{archive:?}: black ink on a white page, as `mac/xzip.lst` states them",
        );

        let themed = launch(archive, false, None);
        let model = themed.session.screen();
        assert_eq!(
            (app::state::unpack_zcolour(model.fg), app::state::unpack_zcolour(model.bg)),
            (ZColour::Default, ZColour::Default),
            "{archive:?}: colours declined — the host theme owns the page, machine or no machine",
        );
    }
}

/// A hybrid render at real kitty-ish cell metrics (8x18) — the shipped mode, and
/// the one SQ-0846 was reported against. Returns the state the frame left
/// behind, because the pair the pixel path resolves through
/// (`render::screen::v6_host_pair`) is published during the render.
#[allow(deprecated)]
fn render_hybrid(session: &GameSession, honour: bool) -> app::state::AppState {
    use app::engine::Engine as _;
    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honour;
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    state
}

/// Every colour inside the status banner window's own box, as the pixel path
/// composes it, plus the `(ink, page)` pair that composition resolved through.
type BannerTally = (image::Rgba<u8>, image::Rgba<u8>, std::collections::BTreeMap<[u8; 4], u32>);
fn banner_tally(session: &GameSession, honour: bool) -> BannerTally {
    use app::engine::{Engine as _, WinNode};
    let state = render_hybrid(session, honour);
    let (ink, page) = app::render::screen::v6_host_pair(&state);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let canvas = app::render::v6_layout::build_chrome_canvas(
        &layout.chrome,
        native,
        ink,
        page,
        &state.colors,
        app::render::v6_layout::TextLayer::All,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    // The banner is found by the text in it, never by a window id.
    let banner = layout
        .chrome
        .iter()
        .find(|it| match &it.node {
            WinNode::Grid(g) => g.px_texts.iter().any(|t| t.text.contains("Score:")),
            _ => false,
        })
        .expect("Zork Zero's status banner is a grid window carrying a `Score:` run");
    let (x0, y0) = (banner.x_px as u32, banner.y_px as u32);
    let x1 = (x0 + banner.w_px as u32).min(canvas.width());
    let y1 = (y0 + banner.h_px as u32).min(canvas.height());
    let mut tally: std::collections::BTreeMap<[u8; 4], u32> = Default::default();
    for y in y0..y1 {
        for x in x0..x1 {
            *tally.entry(canvas.get_pixel(x, y).0).or_default() += 1;
        }
    }
    (ink, page, tally)
}

/// **THE DELIVERABLE — SQ-0846, as reported:** *"the text is not rendering in
/// the location/score area"*, on `stories/Zork Zero Disk.image` with
/// `--pictures Pic.data`.
///
/// It was rendering. It was the host theme's **grey**, on the game's own
/// **white** two-colour plate, and a two-colour Macintosh has no grey anywhere
/// in it — so the banner read as empty. A real Mac drew it black on white, and
/// `mac/xzip.lst` says so in one line.
///
/// **Measured, on the banner window's own box (480x59 native pixels), release
/// 296 / serial 881019 at the first prompt.** The artwork accounts for 15,405
/// white pixels, 8,805 black and 2,608 transparent in both modes; the only thing
/// that moves is the 1,502 pixels of status-text ink:
///
/// | mode                     | black  | the theme's ink |
/// |--------------------------|--------|-----------------|
/// | colours honoured (fixed) | 10,307 | **0**           |
/// | colours declined (theme) |  8,805 | **1,502**       |
///
/// The RELATION is asserted rather than the constants, so the day this banner's
/// art changes under us the test still says the same true thing: every pixel the
/// theme draws in its own ink is drawn black instead, and none go missing.
///
/// FALSIFIED by reverting either half of the fix — `session::machine_screen_pair`
/// or [`PictSource::declines_game_colours`]. With either one back, the honoured
/// column becomes the themed one and 1,502 grey pixels sit on the white banner,
/// which is the report verbatim.
#[test]
fn the_macintosh_status_banner_is_black_ink_and_never_the_themes_grey() {
    if mac_disk().is_none() {
        return;
    }
    let honoured = launch(Some("Pic.data"), true, None);
    let themed = launch(Some("Pic.data"), false, None);
    assert!(honoured.monochrome && themed.monochrome, "both launches load the two-colour plate");

    let (ink, page, hon) = banner_tally(&honoured.session, honoured.honoured);
    let (theme_ink, _, thm) = banner_tally(&themed.session, themed.honoured);

    let black = image::Rgba([0, 0, 0, 255]);
    let white = image::Rgba([255, 255, 255, 255]);
    assert_eq!((ink, page), (black, white), "the Macintosh's own pair reaches the pixel path");
    assert_ne!(theme_ink, black, "the theme's ink is what the banner must NOT be drawn in");

    let count = |t: &std::collections::BTreeMap<[u8; 4], u32>, c: image::Rgba<u8>| {
        t.get(&c.0).copied().unwrap_or(0)
    };
    let grey_on_white = count(&thm, theme_ink);
    assert!(
        grey_on_white > 0,
        "the symptom must reproduce with colours declined, else this test proves nothing",
    );
    assert_eq!(count(&hon, theme_ink), 0, "not one pixel of the theme's ink on a two-colour Mac");
    assert_eq!(
        count(&hon, black),
        count(&thm, black) + grey_on_white,
        "every pixel the theme drew grey is drawn black instead — none lost, none added",
    );
    assert_eq!(count(&hon, white), count(&thm, white), "the plate's own white is untouched");
}
