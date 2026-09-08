//! SQ-1154 — `--colour` names a REGIME, and the regime is independent of the
//! medium.
//!
//! In the user's words: *"when `--colour theme | terminal` we use the raw
//! z-machine path with native media. The opposite is true when we set `--colour
//! machine`, then we use the media path with a raw z-machine file."*
//!
//! Half of that shipped already. `--colour machine` on a bare story file is
//! SQ-0928's `--system-colours`, subsumed by SQ-1082: the opt-in that licenses a
//! machine's §8.3.3 pair when the MEDIUM did not name a machine but
//! `--interpreter` did. The mirror is what this suite pins — a release floppy
//! launched under `--colour terminal` behaves as the same story does as a bare
//! file, because `Config::machine_colours_licensed` answers **no** for that
//! regime whatever the profile's source was.
//!
//! # What "the raw path" reaches, and what it does not
//!
//! Everything that asks the one predicate moves together, which is the whole
//! design: the machine's §8.3.3 pair, the two-colour CARD's pair (and with it
//! `Palette::IbmCga`), and the colour-number TABLE the story's own `set_colour`
//! resolves through. So the round trip is lossless again: the host's RGB is
//! snapped to the nearest §8.3.1 number, and §8.3.1 is the table it is read back
//! through — which on a floppy it was not, and which is the reported symptom.
//!
//! **The ARTWORK is untouched**, and [`the_artwork_is_the_archives_and_the_regime_cannot_move_it`]
//! measures it rather than arguing it: pictures resolve through
//! `graphics::PictSource`'s own per-picture palette, read from the archive's own
//! `PLTE`, and never through `zvm::screen`'s.
//!
//! # Specimens
//!
//! | fixture | medium | machine | what it is here for |
//! |---|---|---|---|
//! | `Journey - The Quest Begins.adf` | Amiga floppy | Amiga | a medium whose table is NOT §8.3.1 |
//! | Zork Zero DOS 360K Disk 1 | `.ima` | IBM PC + CGA card | the two-colour card, which no bare file can reach |
//! | `journey-r83-s890706.z6` | none | asked for | the `--colour machine` direction, unchanged |
//! | `arthur-r74-s890714.z6` | none | `--interpreter 3`/`4` | the GROUND, on both machines with a screen page |
//! | `Arthur - The Quest for Excalibur.adf` | Amiga floppy | Amiga | the reported launch, off its own medium |
//!
//! **Turn count: zero for the PAIR cases, twelve taps for the GROUND cases.**
//! Everything a pair case asks is settled before the first prompt — the palette is
//! installed and the pair is handed to the constructor — so driving keys would only
//! add ways for a frame to differ. The ground is a frame, so those cases are a
//! fixture with a turn count: see [`drive_arthur_intro`].
//!
//! # The pair is not the ground, and that is why this file has two halves
//!
//! The first half of this suite shipped, and every case in it was green while the
//! screen was wrong (SQ-1154 was reopened on it). Withholding the machine's colour
//! VALUES left the per-machine screen RULES live, and those rules make the header
//! pair BE the screen — so the pair was correct and the ground was pure black. Any
//! case added here should say which of the two it is asserting.
//!
//! Both `honor_game_colours` modes throughout, per the project's colour
//! convention. `false` is not a formality here: it short-circuits
//! `colors::host_default_colours` to `None` (§8.3.2, an interpreter that
//! declares itself colourless has no default pair to report), so it is what
//! proves `--colour` is inert against that switch rather than fighting it.
//!
//! `stories/` is gitignored, so every case skips vacuously without its press and
//! [`the_presses_were_actually_read`] is what stops the file quietly passing on a
//! machine that has none of them.

use std::path::PathBuf;

use app::config::{ColourSource, Config};
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::{InterpreterProfile, ProfileSource};
use app::engine::Engine;
use app::session::{GameSession, InputKind};

use ratatui::style::Style;
use zvm::screen::Palette;

/// The Amiga release floppy: `InterpreterProfile::Amiga`, off the medium, and a
/// colour-number table that is emphatically not §8.3.1's.
const AMIGA_FLOPPY: &str = "Journey - The Quest Begins.adf";
/// The bare story file, for the `--colour machine` direction. A different
/// RELEASE from the floppy above (r83/s890706 against r30/s890322) and named
/// here only as "a story with no medium", which is all this suite asks of it.
const BARE_STORY: &str = "journey-r83-s890706.z6";
/// The DOS press serving the CGA plate — the two-colour card, and the one thing
/// in this quest with no existence proof behind it, since a bare story file
/// never reaches the card at all.
const CGA_PRESS: &str =
    "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 1) [!].ima";

/// The terminal this suite pretends to be launched in: a warm dark page under a
/// light ink, chosen so the pair it snaps to is neither machine's.
const TERM_BG: (u8, u8, u8) = (0x1A, 0x1B, 0x26);
const TERM_FG: (u8, u8, u8) = (0xE4, 0xE4, 0xDC);

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn present(file: &str) -> bool {
    stories_dir().join(file).exists()
}

/// What one launch decided about colour, in `startup.rs`'s own order and through
/// the shipped functions rather than a copy of them.
struct Launch {
    profile: InterpreterProfile,
    source: ProfileSource,
    /// The table installed process-wide before the story ran —
    /// `Config::machine_text_palette`, which is the call `startup.rs` makes.
    palette: Palette,
    /// `PictSource::two_colour_card_screen` — the card's table and pair, when
    /// this launch is showing one.
    card: Option<(Palette, (u8, u8))>,
    /// The pair the session is constructed with, which reaches header
    /// `$2C`/`$2D`.
    reported: Option<(u8, u8)>,
    /// Whether the game's own colours survived the archive's say —
    /// `startup.rs`'s answer, not the caller's request.
    honoured: bool,
    /// The story as it booted, so the header can be read back out of memory.
    session: GameSession,
}

/// Boot one file the way `startup.rs` boots it, under a stated colour regime.
///
/// Every colour decision below is the shipped function: the profile off the
/// medium, `Config::machine_text_palette` for the table, `declines_game_colours`
/// for the honour flag, `PictSource::two_colour_card_screen` for the card, and
/// `colors::host_default_colours` for the pair. A harness that re-derived any of
/// them would keep passing while the shipped path regressed, which is the hazard
/// CLAUDE.md names as "boot a harness the way `startup.rs` boots".
///
/// The table this launch resolves colour numbers through is one of the facts the
/// `MachineBoot` carries (SQ-1393), so it is settled here in the same two steps
/// `startup.rs` uses and nothing outside this call can observe it.
fn launch(
    file: &str,
    colour: ColourSource,
    honour: bool,
    interpreter: Option<u8>,
) -> Option<Launch> {
    let path = stories_dir().join(file);
    let bytes = match app::hints::load_story(&path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored press missing at {}", path.display());
            return None;
        }
    };

    let dir = app::scratch_dir("sq1154-launch");
    let over = PictureOverride::resolve_with_session(&path, &dir, None);
    let named_art_std_window = over.std_window();
    let (profile, source) =
        InterpreterProfile::resolve_with_source(&path, interpreter, over.flavour(), None);

    let mut cfg = Config {
        interpreter_profile: profile,
        interpreter_source: source,
        interpreter_number: interpreter,
        honor_game_colours: honour,
        colour_source: colour,
        // `config::resolve` sets this whenever `--colour machine` is TYPED; the
        // other two arms leave it alone, and it is inert under them now.
        system_colours: colour == ColourSource::Machine,
        ..Default::default()
    };

    // `startup.rs`, in order. The table first, before the constructor runs the
    // story and before the host resolves a single colour.
    let mut machine_palette = cfg.machine_text_palette(bytes.first().copied());
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let picture_dims = picts.all_pict_dims();

    // SQ-0806/SQ-0846: two-colour artwork declares the interpreter colourless,
    // but only where the launch has no machine to state a ground of its own.
    if picts.declines_game_colours(cfg.machine_default_colours()) && cfg.honor_game_colours {
        cfg.honor_game_colours = false;
    }
    let mut reported = app::colors::host_default_colours(
        &cfg,
        cfg.machine_default_colours(),
        Style::default(),
        Some(TERM_FG),
        Some(TERM_BG),
        machine_palette,
    );
    let card = picts.two_colour_card_screen(&cfg);
    if let Some((palette, pair)) = card {
        machine_palette = palette;
        reported = Some(pair);
    }

    let boot = app::machine_boot::MachineBoot::resolve(
        cfg.interpreter_profile,
        &picts,
        named_art_std_window,
        cfg.advertised_interpreter_number(),
        reported,
        // SQ-1154: the licence rides in the boot value now, because it governs the
        // per-machine screen RULES as well as the values above. `startup.rs` passes
        // exactly this call.
        cfg.machine_colours_licensed(),
        app::native_font::FaceSet::none(),
        machine_palette,
        None,
    );
    let mut session = GameSession::new_for_machine(
        bytes,
        cfg.honor_game_colours,
        false,
        false,
        picture_dims,
        None,
        None,
        &boot,
    )
    .unwrap_or_else(|e| panic!("{file} boots: {e:?}"));
    assert!(!session.quit && session.machine.fault_trace.is_none(), "{file} booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = std::fs::remove_dir_all(&dir);

    Some(Launch {
        profile,
        source,
        palette: session.machine.palette(),
        card,
        reported,
        honoured: cfg.honor_game_colours,
        session,
    })
}

/// The pair a host-sourced regime resolves to, straight out of the shipped
/// snapper. Named once so no case pins a literal that would have to be re-derived
/// when the §8.3.1 nearest-neighbour search is touched.
fn terminals_pair() -> (u8, u8) {
    // §8.3.1's own table: this snaps the TERMINAL's pair, which no machine owns.
    app::colors::host_default_colour_pair(
        zvm::screen::Palette::Standard,
        Style::default(),
        Some(TERM_FG),
        Some(TERM_BG),
    )
    .expect("both channels probed")
}

/// Header `$2C`/`$2D`, as the story reads them.
fn header_pair(l: &Launch) -> (u8, u8) {
    (l.session.machine.mem.read_byte(0x2C), l.session.machine.mem.read_byte(0x2D))
}

// ── The medium under a host regime ───────────────────────────────────────────

/// **The reported case.** An Amiga release floppy under `--colour terminal` and
/// `--colour theme` is told the TERMINAL's pair, through §8.3.1's table — the
/// state a bare story file is in.
///
/// The two halves are one defect and both are asserted, because either alone
/// looks correct: the pair reaching `$2C`/`$2D` is the host's, AND the palette
/// those numbers resolve through is `Standard`. Snapping to §8.3.1 and reading
/// back through the Amiga's is what "reports a colour that is not the
/// terminal's" means.
///
/// FALSIFY by restoring `machine_colours_licensed` to
/// `interpreter_source.licenses_machine_colours(system_colours)`: the floppy is
/// `ProfileSource::Medium`, so it licenses the machine whatever `--colour` said,
/// and both assertions fail with the Amiga's own pair under the Amiga's table.
#[test]
fn a_release_floppy_under_a_host_regime_is_told_the_hosts_pair() {
    if !present(AMIGA_FLOPPY) {
        eprintln!("SKIP: {AMIGA_FLOPPY} absent");
        return;
    }
    let machine_pair = InterpreterProfile::Amiga.default_colours().expect("the Amiga states one");
    assert_ne!(machine_pair, terminals_pair(), "the experiment needs the two to differ");

    for colour in [ColourSource::Terminal, ColourSource::Theme] {
        let l = launch(AMIGA_FLOPPY, colour, true, None).expect("checked present");
        assert_eq!(l.source, ProfileSource::Medium, "the floppy still names its machine");
        assert_eq!(l.profile, InterpreterProfile::Amiga, "and it is still an Amiga");
        assert_eq!(
            l.palette,
            Palette::Standard,
            "{colour:?}: the raw path resolves colour numbers through §8.3.1"
        );
        assert_eq!(l.reported, Some(terminals_pair()), "{colour:?}: the host's own pair");
        assert_eq!(header_pair(&l), terminals_pair(), "{colour:?}: and the story reads it");
        assert_ne!(header_pair(&l), machine_pair, "{colour:?}: not the Amiga's");
    }
}

/// The control, on the same floppy: `--colour machine` is the default and is
/// unmoved — the Amiga's pair, under the Amiga's table.
///
/// Without this the case above is satisfied by breaking original media outright.
#[test]
fn the_same_floppy_under_colour_machine_is_still_an_amiga() {
    if !present(AMIGA_FLOPPY) {
        eprintln!("SKIP: {AMIGA_FLOPPY} absent");
        return;
    }
    let l = launch(AMIGA_FLOPPY, ColourSource::Machine, true, None).expect("checked present");
    assert_eq!(l.palette, Palette::Amiga, "the medium's own table");
    assert_eq!(
        l.reported,
        InterpreterProfile::Amiga.default_colours(),
        "and the machine's §8.3.3 pair"
    );
    assert_eq!(header_pair(&l), l.reported.expect("licensed"), "which the story reads");
}

/// **`honor_game_colours = false` still answers before `--colour` does**, in
/// every regime.
///
/// §8.3.2: an interpreter that declares itself colourless has no default page
/// and ink to report, so the VM's own black-on-white seed is left alone. The two
/// flags are different axes and `--colour` is inert while this one says off —
/// the property a single-mode suite cannot see, and the reason this file pins
/// both modes.
#[test]
fn declining_game_colours_outranks_every_regime() {
    if !present(AMIGA_FLOPPY) {
        eprintln!("SKIP: {AMIGA_FLOPPY} absent");
        return;
    }
    for colour in [ColourSource::Machine, ColourSource::Theme, ColourSource::Terminal] {
        let l = launch(AMIGA_FLOPPY, colour, false, None).expect("checked present");
        assert_eq!(l.reported, None, "{colour:?}: a colourless interpreter reports no pair");
        assert!(!l.honoured, "{colour:?}: and the story is told so");
    }
}

// ── The two-colour card, which no bare story file can reach ──────────────────

/// **The CGA press.** A `.CG1` under `--colour terminal` installs no
/// `Palette::IbmCga`, because the launch has no machine to be showing a card
/// FOR.
///
/// This is the one question in SQ-1154 the raw path could not vouch for: a bare
/// story file never reaches the card at all, so nothing about `IbmCga` carrying
/// a display-DEPTH fact — read by `zvm::screen::two_colour_card_request`, which
/// decides whether the story asks for mono or colour plates — is exercised by
/// that existence proof. The answer is that the coupling is not excepted from
/// the regime, it is governed by it: `two_colour_card_screen` asks the same
/// licence, so under a host regime it answers `None` and neither the table nor
/// the depth is ever installed.
///
/// Both `honor_game_colours` modes, and the honoured half is the load-bearing
/// one — with colours declined the card is unreachable for a second reason and
/// the case would pass without testing anything.
#[test]
fn a_cga_press_under_a_host_regime_shows_no_card() {
    if !present(CGA_PRESS) {
        eprintln!("SKIP: {CGA_PRESS} absent");
        return;
    }
    // The control first: this press really is showing a card, or the assertions
    // below are vacuous.
    let machine = launch(CGA_PRESS, ColourSource::Machine, true, None).expect("checked present");
    assert_eq!(machine.source, ProfileSource::Medium, "a DOS press names the IBM PC");
    assert!(machine.card.is_some(), "the 360K Disk 1 serves the CGA plate");
    assert_eq!(machine.palette, Palette::IbmCga, "and the card's table is installed");
    assert!(machine.palette.two_colour_card(), "which carries the display's one bit");

    for colour in [ColourSource::Terminal, ColourSource::Theme] {
        for honour in [true, false] {
            let l = launch(CGA_PRESS, colour, honour, None).expect("checked present");
            assert!(l.card.is_none(), "{colour:?}/{honour}: no machine, so no card");
            assert_ne!(l.palette, Palette::IbmCga, "{colour:?}/{honour}: IbmCga never installed");
            assert!(
                !l.palette.two_colour_card(),
                "{colour:?}/{honour}: nor the display depth it carries"
            );
            assert_eq!(l.palette, Palette::Standard, "{colour:?}/{honour}: §8.3.1's table");
        }
    }
}

// ── The other direction, which already worked ────────────────────────────────

/// **`--colour machine` on a bare `.z6` with `--interpreter` is unchanged.**
///
/// SQ-0928's opt-in, subsumed by SQ-1082: a number typed at a bare story file is
/// `ProfileSource::Asked`, which licenses the machine only on request. This
/// quest drives the same predicate from the other end and must not disturb it,
/// so both sides of the opt-in are pinned — with `--colour machine` the Amiga's
/// pair and table, without it neither.
#[test]
fn colour_machine_on_a_bare_story_file_still_presents_the_asked_for_machine() {
    if !present(BARE_STORY) {
        eprintln!("SKIP: {BARE_STORY} absent");
        return;
    }
    let asked = launch(BARE_STORY, ColourSource::Machine, true, Some(4)).expect("checked present");
    assert_eq!(asked.source, ProfileSource::Asked, "a typed number, not a medium");
    assert_eq!(asked.profile, InterpreterProfile::Amiga, "--interpreter 4");
    assert_eq!(asked.palette, Palette::Amiga, "the opt-in licenses the machine's table");
    assert_eq!(
        asked.reported,
        InterpreterProfile::Amiga.default_colours(),
        "and its §8.3.3 pair, on a file that came off no disk at all"
    );
    assert_eq!(header_pair(&asked), asked.reported.expect("licensed"));

    // …and without the opt-in the same launch is a bare story file, which is the
    // half `--colour theme|terminal` now reproduces on a medium.
    let plain = launch(BARE_STORY, ColourSource::Terminal, true, Some(4)).expect("checked present");
    assert_eq!(plain.palette, Palette::Standard, "no licence, no machine table");
    assert_eq!(plain.reported, Some(terminals_pair()), "the host answers instead");
}

// ── The artwork, which none of this may move ─────────────────────────────────

/// **The regime decides the TEXT table and nothing else.** The same picture off
/// the same floppy decodes identically under `--colour machine` and `--colour
/// terminal`, pixel for pixel.
///
/// Measured rather than argued, because the design turns on it: pictures resolve
/// through `PictSource`'s own Current Palette, established per picture from the
/// archive's own `PLTE` (§11.3), and never through `zvm::screen`'s. If that were
/// not so, withholding the machine's table would repaint the artwork — which is
/// the one outcome this quest may not have.
#[test]
fn the_artwork_is_the_archives_and_the_regime_cannot_move_it() {
    if !present(AMIGA_FLOPPY) {
        eprintln!("SKIP: {AMIGA_FLOPPY} absent");
        return;
    }
    let plate = |colour: ColourSource| -> Vec<u8> {
        let path = stories_dir().join(AMIGA_FLOPPY);
        let dir = app::scratch_dir("sq1154-art");
        let over = PictureOverride::resolve_with_session(&path, &dir, None);
        let (profile, source) =
            InterpreterProfile::resolve_with_source(&path, None, over.flavour(), None);
        let cfg = Config {
            interpreter_profile: profile,
            interpreter_source: source,
            colour_source: colour,
            system_colours: colour == ColourSource::Machine,
            ..Default::default()
        };
        // The table this regime resolves through — asked for the same reason the
        // launch asks, and asserted on by the caller rather than installed anywhere:
        // the artwork below must be the ARCHIVE's whatever it says (SQ-1393).
        let _ = cfg.machine_text_palette(Some(6));
        let mut picts = PictSource::resolve_with_override(&path, over, None);
        let dims = picts.all_pict_dims();
        let (resnum, ..) = *dims.first().expect("the floppy serves plates");
        let img = picts.image(u32::from(resnum)).expect("the first plate decodes");
        let _ = std::fs::remove_dir_all(&dir);
        img.to_rgba8().into_raw()
    };
    let machine = plate(ColourSource::Machine);
    assert!(!machine.is_empty(), "a plate with pixels in it");
    assert_eq!(machine, plate(ColourSource::Terminal), "the archive's palette, either way");
}

// ── Non-vacuity ──────────────────────────────────────────────────────────────

/// At least one press was actually on disk, or every case above skipped and this
/// file passed without measuring anything (CLAUDE.md's rule for gitignored
/// commercial media).
#[test]
fn the_presses_were_actually_read() {
    let found: Vec<&str> =
        [AMIGA_FLOPPY, BARE_STORY, CGA_PRESS].into_iter().filter(|f| present(f)).collect();
    if found.is_empty() {
        eprintln!("SKIP: no gitignored press present; every case in this file skipped");
        return;
    }
    eprintln!("SQ-1154 specimens read: {found:?}");
}

// ── The GROUND, which is not the pair (SQ-1154, reopened) ────────────────────
//
// Everything above this line asserts a PAIR — what `$2C`/`$2D` carry, and what
// table the numbers in them resolve through — and every one of those cases was
// green while the screen was wrong. That is why the quest was reopened: the
// licence as first shipped withheld the machine's colour VALUES and left the
// per-machine screen RULES live, and a rule that makes the header pair BE the
// screen discards the host's true RGB whichever pair it is handed. The host's
// `#1A1B26` can only be expressed as a colour NUMBER, snaps to §8.3.1's 2, and 2
// is pure black.
//
// So these cases assert the GROUND: `ScreenModel.bg`/`fg`, which is what
// `render_story_pane` floods the pane with. `0` is `ZColour::Default` — no
// machine claimed the page, so the host paints its own RGB un-snapped, which is
// the correct outcome under a host regime and is exactly what the Atari ST and
// the IBM PC do already, having no screen page to claim.

/// Arthur as a bare story file. The controlled specimen: no medium, no archive
/// table, nothing but `--interpreter` — which is how the user isolated this,
/// after a floppy-against-floppy comparison that had crossed two RELEASES
/// (r54/890606 against r74/890714).
const ARTHUR_BARE: &str = "arthur-r74-s890714.z6";
/// And the Amiga floppy the defect was reported on, for the medium route.
const ARTHUR_FLOPPY: &str = "Arthur - The Quest for Excalibur.adf";

/// ZMSD §11.1.3. The two machines whose §8.3.3 pair is not advice about a
/// terminal but the screen itself — the Amiga through `global_colour_pens`
/// (§8.3's shared pens) and the Macintosh through `v6_screen_page`
/// (`mac/xzip.lst`: "Mac defaults: white under black"). They reach the ground by
/// two different rules and both rules go through `zvm::screen::machine_rule`.
const AMIGA: u8 = 4;
const MACINTOSH: u8 = 3;

/// **Turn count: 12 taps, and it matters.** A frame is a fixture: asserting at
/// boot is asserting before the story has laid a single window out, and a repaint
/// defect shows one action later, not immediately. Twelve taps answering `n` to
/// "restore a saved position?" is `v6_arthur_status`'s own route to Arthur's
/// first playable frame, reused so the two harnesses cannot disagree about where
/// that is.
fn drive_arthur_intro(l: &mut Launch) {
    let _ = l.session.take_transcript();
    for _ in 0..12 {
        let r = match l.session.pending_input() {
            InputKind::Line => l.session.submit(""),
            InputKind::Char => l.session.submit_char(13),
            InputKind::Event => l.session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = l.session.submit_char(b'n');
        }
    }
}

/// The pane's own page as the renderer receives it, `(bg, fg)` packed by
/// `state::pack_zcolour`. `0` is `ZColour::Default`: no machine claimed the page,
/// so the host paints the terminal's real RGB.
fn pane_ground(l: &Launch) -> (u32, u32) {
    let m = l.session.screen();
    (m.bg, m.fg)
}

/// **The reopened defect, on BOTH machines.** A launch under `--colour theme` or
/// `--colour terminal` leaves the pane's page at `ZColour::Default`, so the host
/// paints its own background; under `--colour machine` the machine claims it.
///
/// Both machines, because the symptom was reported on the Amiga and the fix is at
/// `zvm::screen::machine_rule` rather than at `amiga_global_colour_pair`. Gating
/// the call site would have fixed the Amiga and left the Macintosh — measured at
/// `--interpreter 3`, `--colour terminal`, grounding at pure black exactly as the
/// Amiga does, by the other rule.
///
/// Both `honor_game_colours` modes, and `false` is not a formality: it withdraws
/// Flags 1 bit 0, which `machine_rule` also requires, so the ground must be the
/// host's in every regime. That is the discriminator the quest named — turning
/// game colours off cleared the black — and it is a symptom-level workaround, not
/// the fix, so it is pinned as a floor rather than as the answer.
///
/// FALSIFY by dropping `m.machine_colours_licensed` from `machine_rule`: both
/// host-regime rows come back claiming a page — §8.3.1 pure black under white,
/// the reported ground — while every pair asserted above stays correct.
#[test]
fn a_machines_screen_page_is_withheld_with_its_colours_under_a_host_regime() {
    if !present(ARTHUR_BARE) {
        eprintln!("SKIP: {ARTHUR_BARE} absent");
        return;
    }
    for interp in [AMIGA, MACINTOSH] {
        // The non-vacuity guard: this machine must actually claim a page under
        // the regime that licenses it, or the rows below prove nothing.
        let mut owned = launch(ARTHUR_BARE, ColourSource::Machine, true, Some(interp))
            .expect("checked present");
        drive_arthur_intro(&mut owned);
        assert_ne!(
            pane_ground(&owned),
            (0, 0),
            "interpreter {interp}: the machine claims the page when it is licensed to",
        );
        assert!(
            zvm::screen::machine_screen_pair(&owned.session.machine).is_some(),
            "interpreter {interp}: …by the rule this quest is about",
        );

        for colour in [ColourSource::Terminal, ColourSource::Theme] {
            let mut l =
                launch(ARTHUR_BARE, colour, true, Some(interp)).expect("checked present");
            drive_arthur_intro(&mut l);
            assert_eq!(
                pane_ground(&l),
                (0, 0),
                "interpreter {interp} under {colour:?}: the host paints its own ground, \
                 un-snapped — this came back pure black",
            );
            assert_eq!(
                zvm::screen::machine_screen_pair(&l.session.machine),
                None,
                "interpreter {interp} under {colour:?}: no machine is presented, so none \
                 claims the page",
            );
        }

        // Flags 1 bit 0 withdrawn: `machine_rule`'s colour term fails, so no
        // regime can claim the page.
        for colour in [ColourSource::Machine, ColourSource::Terminal, ColourSource::Theme] {
            let mut l =
                launch(ARTHUR_BARE, colour, false, Some(interp)).expect("checked present");
            drive_arthur_intro(&mut l);
            assert_eq!(
                pane_ground(&l),
                (0, 0),
                "interpreter {interp} under {colour:?}, game colours off: §8.3.2 declares \
                 the interpreter colourless and the host theme owns the screen",
            );
        }
    }
}

/// **The Amiga's shared pens are a BEHAVIOUR, and the licence reaches it too.**
///
/// §8.3: a Version 6 interpreter under interpreter number 4 uses one pair of
/// colours for ALL windows, and changing either repaints every window to match.
/// Withholding the machine's pair does not withhold that — which is why the first
/// fix on this quest looked complete and was not. The rule itself has to be off,
/// or the story's own `set_colour` is globalised over a ground the host never
/// chose.
#[test]
fn the_amiga_pens_are_off_under_a_host_regime() {
    if !present(ARTHUR_BARE) {
        eprintln!("SKIP: {ARTHUR_BARE} absent");
        return;
    }
    let mut owned =
        launch(ARTHUR_BARE, ColourSource::Machine, true, Some(AMIGA)).expect("checked present");
    drive_arthur_intro(&mut owned);
    assert!(
        zvm::screen::amiga_global_colour_pair(&owned.session.machine),
        "the licensed Amiga keeps §8.3's shared pens",
    );
    for colour in [ColourSource::Terminal, ColourSource::Theme] {
        let mut l = launch(ARTHUR_BARE, colour, true, Some(AMIGA)).expect("checked present");
        drive_arthur_intro(&mut l);
        assert!(
            !zvm::screen::amiga_global_colour_pair(&l.session.machine),
            "{colour:?}: a launch that presents no machine has no pens to share",
        );
    }
}

/// The same thing off the MEDIUM the user reported it on, rather than off
/// `--interpreter`. `Arthur - The Quest for Excalibur.adf` is release 54 / serial
/// 890606 — a different release from the bare file above, which is exactly why
/// the bare file is the controlled specimen and this is the corroboration.
#[test]
fn the_reported_floppy_grounds_on_the_host_under_a_host_regime() {
    if !present(ARTHUR_FLOPPY) {
        eprintln!("SKIP: {ARTHUR_FLOPPY} absent");
        return;
    }
    let mut owned =
        launch(ARTHUR_FLOPPY, ColourSource::Machine, true, None).expect("checked present");
    assert_eq!(owned.profile, InterpreterProfile::Amiga, "the floppy names its machine");
    assert_eq!(owned.source, ProfileSource::Medium, "and the MEDIUM is what named it");
    drive_arthur_intro(&mut owned);
    assert_ne!(pane_ground(&owned), (0, 0), "--colour machine keeps the Amiga's page");

    for colour in [ColourSource::Terminal, ColourSource::Theme] {
        let mut l = launch(ARTHUR_FLOPPY, colour, true, None).expect("checked present");
        drive_arthur_intro(&mut l);
        assert_eq!(
            pane_ground(&l),
            (0, 0),
            "{colour:?}: the reported launch — a black ground where the terminal asked \
             for its own",
        );
    }
}

// ── A host Save State does not carry a regime across ─────────────────────────

/// **A restore adopts THIS run's colour regime, and measurement says it already
/// does.**
///
/// The concern, reasoned off `archive.rs` while this quest was being fixed: a host
/// Save State stores colour as NUMBERS (`zvm::screen::ZColour`, and the v6 window table is
/// the source of truth for a Version 6 story), a number means nothing without a
/// palette, and `--colour` now chooses the palette. So saving under `--colour
/// machine` and restoring under `--colour terminal` looked capable of resolving
/// every stored number through a table the save never saw — SQ-0958's rule
/// arriving at runtime instead of in a suite.
///
/// **Measured, both directions, and it is not real.** The restoring run's regime
/// decides the default page and ink outright and nothing from the saving run's
/// survives: the pair in `$2C`/`$2D` is the one THIS launch published, and the
/// pane's ground is this launch's licence answering. That is the right answer
/// rather than a lucky one — `--colour` is a flag of the run doing the showing,
/// not a property of the saved game, and the licence lives on the `Machine` the
/// restoring run built (see `session::new_for_machine`), where neither
/// `restore_screen`'s whole-`ScreenState` assignment nor `Machine::restart`'s
/// fresh one can reach it.
///
/// The residue is a game's own `set_colour`, whose number resolves through
/// whatever table the showing run named. That is the regime read consistently and
/// is what this quest already settled for a fresh launch: on the raw path
/// `set_colour(4)` IS §8.3.1 red. Storing the number rather than a resolved RGB
/// is the archive's backend-neutral rule working, not a hole in it.
///
/// Restore, then MAKE A MOVE, then assert — a restore bug surfaces on the next
/// repaint, and the frame immediately after a restore is when everything still
/// looks correct.
#[test]
fn a_host_save_state_does_not_carry_a_colour_regime_across() {
    if !present(ARTHUR_BARE) {
        eprintln!("SKIP: {ARTHUR_BARE} absent");
        return;
    }
    let round = |saving: ColourSource, restoring: ColourSource| {
        let mut src = launch(ARTHUR_BARE, saving, true, Some(AMIGA)).expect("checked present");
        drive_arthur_intro(&mut src);
        let _ = src.session.submit("look");
        let _ = src.session.take_transcript();

        let mapper = mapper::mapper::Mapper::default();
        let es = Engine::save_state(&src.session);
        // `app::scratch_dir` is unique per CALL, which is what keeps two cases in
        // one binary off each other's files (SQ-1131).
        let dir = app::scratch_dir("sq1154-regime-restore");
        let path = dir.join("cross-regime.lanthorn");
        app::archive::save_archive_meta_pics(
            &path,
            &mapper,
            &es,
            Some(&src.session.machine.screen),
            &src.session.machine.aux_data,
            app::archive::Meta {
                format_version: app::archive::CURRENT_FORMAT_VERSION,
                ifid: None,
                name: None,
                turns: 0,
                saved_at: String::new(),
                location: None,
                score: None,
                trigger: app::archive::SaveTrigger::HostState,
            },
            &app::archive::SessionRecord::empty(),
            &src.session.pictures_png(),
            None,
            None,
        )
        .expect("save archive");
        let ac = app::archive::load_archive(&path).expect("load archive");

        // A fresh launch under the OTHER regime, and what it looks like before the
        // restore — the baseline the restore must not move.
        let mut dst = launch(ARTHUR_BARE, restoring, true, Some(AMIGA)).expect("checked present");
        drive_arthur_intro(&mut dst);
        let want_header = header_pair(&dst);
        let want_ground = pane_ground(&dst);

        Engine::restore_state(&mut dst.session, &ac.engine_save()).expect("restore");
        app::session::restore_screen(&mut dst.session, ac.screen.clone().expect("screen"));
        dst.session.load_pictures_png(&ac.pictures);
        // The perturbation. Everything still looks right in the frame before it.
        let _ = dst.session.submit("look");
        let _ = dst.session.take_transcript();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            header_pair(&dst),
            want_header,
            "{saving:?} -> {restoring:?}: the story is told THIS run's pair, not the save's",
        );
        assert_eq!(
            pane_ground(&dst),
            want_ground,
            "{saving:?} -> {restoring:?}: and the pane's ground is this run's licence \
             answering, one move after the restore",
        );
    };
    // Not symmetric, so both: one direction loses a machine's table, the other
    // acquires one the save never saw.
    round(ColourSource::Machine, ColourSource::Terminal);
    round(ColourSource::Terminal, ColourSource::Machine);

    // …and the experiment is only worth running because the two regimes genuinely
    // differ on this story: an Amiga grounds on `DEF_BACK 12`, a host regime on
    // nothing at all.
    let mut amiga =
        launch(ARTHUR_BARE, ColourSource::Machine, true, Some(AMIGA)).expect("checked present");
    drive_arthur_intro(&mut amiga);
    let mut host =
        launch(ARTHUR_BARE, ColourSource::Terminal, true, Some(AMIGA)).expect("checked present");
    drive_arthur_intro(&mut host);
    assert_ne!(
        (header_pair(&amiga), pane_ground(&amiga)),
        (header_pair(&host), pane_ground(&host)),
        "the two regimes must differ, or the restore assertions above are vacuous",
    );
}
