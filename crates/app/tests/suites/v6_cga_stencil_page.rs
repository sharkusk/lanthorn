//! SQ-0956 — the CARD an IBM PC is showing is part of the machine, and a CGA
//! card's screen is not the machine's: Zork Zero's white page painted out its own
//! CGA artwork on a DOS press.
//!
//! # As reported
//!
//! *DOS Zork Zero with the CGA rendition shows a white page bleeding into its
//! artwork* — and, in the same breath, the constraint that shapes the fix: **the
//! PC background SHOULD be white for Zork Zero normally, just not for CGA.** Then,
//! after a first fix that turned the story's colours OFF for a two-colour
//! rendition: *that killed the in-game `color` command*, which is a regression and
//! is why this file's mechanism is not that one.
//!
//! # The three machine screenshots that settle it
//!
//! `machine-screenshots/dos-zorkzero.png` — the Banquet Hall on the COLOUR
//! rendition, **25.7% `#FFFFFF`**. White, and the user confirms white is right
//! there: Zork Zero issues `set_colour(fg=2, bg=9)` on a window the size of the
//! screen and a full-colour plate has no quarrel with it.
//!
//! `machine-screenshots/dos-zorkzero-cga.png` — the same room, same release, a DOS
//! emulator in CGA mode running `zork0.cg1`. Censused whole at 507x317:
//!
//! | share | value     | what                                   |
//! |-------|-----------|----------------------------------------|
//! | 48.3% | `#000000` | the page                               |
//! |  8.8% | `#A0A0A0` | the ink AND the artwork, one shade     |
//! |   —   | 161 hues  | a grey ramp from video scaling, no second colour |
//!
//! Row parity was checked before the census, because an interlaced capture
//! censuses backwards (SQ-0933): even rows 39,252 black / 7,135 grey, odd rows
//! 38,391 / 6,968 — they agree, so the whole-frame number is the honest one.
//!
//! `machine-screenshots/mac-zorkzero-game.png` — the OTHER two-colour archive, and
//! the control that keeps this from being "two colours means dark": 77.2%
//! `#FFFFFF` against 22.8% `#000000` inside the 480x300 game window — a 1-bit
//! capture, so those are its only two values — with the pillars dithered black
//! on that white. Same
//! `EF_MONO` flag, opposite polarity, a real white. That is why
//! `blorb::infocom_pics` now carries two tables instead of one.
//!
//! # The mechanism, and why it is not the honour flag
//!
//! The CGA frame is the story's own pair INVERTED — it asks for black ink on a
//! white page and the card shows light ink on a black one. Turning colours OFF
//! reproduces that frame, because the game checks the colour flag and issues no
//! `set_colour` at all when it is clear (measured on this press) — and it costs the
//! `color` command for the same reason. So the flag stays SET and the CARD
//! answers instead: a display with two states takes one bit from a §8.3.1 pair,
//! which channel wants the lit state, and shows its own two colours in that
//! polarity. `zvm::screen::two_colour_card_request` is the rule and carries the
//! argument; [`zvm::screen::Palette::IbmCga`] is what installs it, off
//! `PictSource::two_colour_card` — the archive's CONTAINER, which is the only
//! thing that names the card.
//!
//! | launch                          | palette   | reported pair | story's colours |
//! |---------------------------------|-----------|---------------|-----------------|
//! | DOS press, `.CG1`               | `IbmCga`  | **(2, 9)**    | honoured, through the card |
//! | DOS press, `.EG1`/`.MG1`        | `IbmYzip` | (6, 9)        | honoured, as named |
//! | bare `.z6` + `--pictures *.cg1` | Standard  | none          | declined — SQ-0806 unmoved |
//!
//! # Specimens
//!
//! One release, one press, one machine — and the archive is the only thing that
//! moves, which is as clean a control as this corpus offers.
//!
//! | fixture                          | release      | archive served | two-colour |
//! |----------------------------------|--------------|----------------|------------|
//! | DOS 360K **Disk 1**              | r393/s890714 | `.CG1` (CGA)   | yes        |
//! | DOS 360K **Disk 3**              | r393/s890714 | `.EG1` (EGA)   | no         |
//! | DOS 720K **Disk 2**              | r393/s890714 | `.CG1` (CGA)   | yes        |
//! | DOS 720K **Disk 1**              | r393/s890714 | `.MG1` (MCGA)  | no         |
//!
//! Each volume is opened directly and `crate::assets::volumes` mounts the rest of
//! the set around it, so which plate a launch gets is decided by the disk the
//! player put in — no `--pictures` anywhere here, because the reported launch had
//! none. **Turn count: zero.** Every frame below is the boot banner, flushed
//! through the real elems pipeline; Zork Zero paints its ornate border and its
//! window-0 page before it asks for anything, so the bleed is on screen at the
//! first prompt and driving keys would only add ways for the frame to differ. The
//! one case that must move the ground drives one turn and says so.
//!
//! Both `honor_game_colours` modes are pinned throughout, per the project's colour
//! convention — and here the `true` half is the load-bearing one, since it is the
//! mode the defect was reported in and the mode lanthorn ships.
//!
//! The DOS press is gitignored commercial media, so every case skips vacuously
//! without it and [`the_press_was_actually_read`] is what stops the file quietly
//! passing on a machine that has none of it.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::{InterpreterProfile, ProfileSource};
use app::session::GameSession;
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const RELEASE: u16 = 393;
const SERIAL: &[u8] = b"890714";

/// The volume serving the CGA plate on the 360K press — the reported launch.
const CGA_DISK: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 1) [!].ima";
/// The same press's EGA volume: the control that must keep its colours.
const EGA_DISK: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 3) [!].ima";
/// The 720K press, where the two plates sit the other way round.
const CGA_DISK_720: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (720K) (Disk 2) [!].ima";
const MCGA_DISK_720: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (720K) (Disk 1) [!].ima";

/// Every specimen, with what the press serves off it.
const PRESS: &[(&str, bool)] =
    &[(CGA_DISK, true), (EGA_DISK, false), (CGA_DISK_720, true), (MCGA_DISK_720, false)];

/// A pane roomy enough for Zork Zero's chrome ring plus a real story viewport.
const PANE: Rect = Rect { x: 0, y: 0, width: 120, height: 45 };

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn present(file: &str) -> bool {
    stories_dir().join(file).exists()
}

/// What `startup.rs` decided about this launch, before any rendering.
struct Decision {
    profile: InterpreterProfile,
    source: ProfileSource,
    monochrome: bool,
    /// `PictSource::two_colour_card` — is the archive a VIDEO CARD's two colours?
    card: bool,
    /// `Config::machine_default_colours` — the machine's own screen.
    machine_pair: Option<(u8, u8)>,
    /// `Config::machine_two_colour_colours` — the card's, when one is showing.
    two_colour_pair: Option<(u8, u8)>,
    declines: bool,
    /// The pair `startup.rs` hands the session, which reaches header `$2C`/`$2D`.
    reported: Option<(u8, u8)>,
}

/// `startup.rs`'s colour sequence for one volume, run through the real
/// [`app::config::Config`] so the licence gate is the shipped one and not a
/// re-implementation of it.
fn decide(file: &str) -> Option<Decision> {
    let path = stories_dir().join(file);
    if !path.exists() {
        eprintln!("SKIP: gitignored DOS press missing at {}", path.display());
        return None;
    }
    let dir = app::scratch_dir("sq956-decide");
    // No `--pictures`: the reported launch named no archive, and the medium is
    // what serves the plate.
    let over = PictureOverride::resolve_with_session(&path, &dir, None);
    let (profile, source) = InterpreterProfile::resolve_with_source(&path, None, over.flavour(), None);
    let picts = PictSource::resolve_with_override(&path, over, None);
    let cfg = app::config::Config {
        interpreter_profile: profile,
        interpreter_source: source,
        ..Default::default()
    };
    let machine_pair = cfg.machine_default_colours();
    let two_colour_pair = cfg.machine_two_colour_colours();
    let card_screen = picts.two_colour_card_screen(&cfg);
    let card = card_screen.is_some();
    let d = Decision {
        profile,
        source,
        monochrome: picts.is_monochrome(),
        card,
        machine_pair,
        two_colour_pair,
        declines: picts.declines_game_colours(machine_pair),
        // `startup.rs`'s own order: the card's pair when one is showing, else the
        // machine's. The theme fallback below it cannot be reached here, because a
        // DOS press always licenses a machine.
        reported: card_screen.map(|(_, pair)| pair).or(machine_pair),
    };
    let _ = std::fs::remove_dir_all(&dir);
    Some(d)
}

/// Whether the launch is allowed to know which CARD it is showing.
///
/// `Card::Blind` is the pre-SQ-0956 screen, reproduced on purpose: the same press,
/// the same archive, the same honoured colours, with the machine's own pair and
/// palette (`IbmYzip`, white `#FFFFFF`) instead of the card's. That is the frame
/// the defect was reported on, and every measurement below is a comparison against
/// it rather than against a literal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Card {
    /// What lanthorn ships: the archive names the card, the card states the screen.
    Read,
    /// The card unread — the machine's pair for a display that is not the machine's.
    Blind,
}

/// One booted-and-rendered frame, plus the honour answer that produced it.
struct Frame {
    buf: Buffer,
    viewport: Rect,
    honoured: bool,
    /// The story window's own background — Zork Zero's `set_colour` page, as the
    /// §8.3.1 number it declared and as the colour the render resolves it to
    /// (through the IBM PC's palette and the player's theme, never a literal).
    story_bg: zvm::screen::ZColour,
    story_page: Option<ratatui::style::Color>,
    /// The model and the state the frame was drawn from, so the other surfaces
    /// that fill a transparent hole can be asked the same question.
    model: app::engine::ScreenModel,
    state: AppState,
    /// The palette this frame was rendered under — carried so a case that builds
    /// a SECOND surface from `model`/`state` after the fact can reinstall it.
    /// Without that the composite resolves colour numbers through whichever
    /// palette the last `frame_after` call left behind, which is the reader half
    /// of SQ-0958 and cost this file two frames measured under the wrong table.
    palette: zvm::screen::Palette,
}

/// Boot one volume the way `startup.rs` boots — the medium picks the profile and
/// the palette, the archive names the card and states the pair, the four
/// screen-size links resolve in order — and render one hybrid frame after `turns`
/// blank lines.
///
/// `card` is the one knob, and it is the whole experiment: see [`Card`].
fn frame_after(file: &str, card: Card, turns: usize) -> Option<Frame> {
    let path = stories_dir().join(file);
    let bytes = match app::hints::load_story(&path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored DOS press missing at {}", path.display());
            return None;
        }
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), RELEASE, "this press carries r{RELEASE}");
    assert_eq!(&bytes[0x12..0x18], SERIAL);

    let dir = app::scratch_dir("sq956-frame");
    let over = PictureOverride::resolve_with_session(&path, &dir, None);
    let named_art_std_window = over.std_window();
    let (profile, source) = InterpreterProfile::resolve_with_source(&path, None, over.flavour(), None);
    app::v6_set_palette(zvm::interpreter::palette_for(
        profile.row_number(),
        bytes.first().copied(),
    ));
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let picture_dims = picts.all_pict_dims();

    let cfg = app::config::Config {
        interpreter_profile: profile,
        interpreter_source: source,
        ..Default::default()
    };
    let honoured = !picts.declines_game_colours(cfg.machine_default_colours());
    // `startup.rs`'s two lines, through the SHIPPED function rather than a copy of
    // it — `PictSource::two_colour_card_screen` is what boot and restart both call,
    // so a regression there fails here (SQ-0956). `Card::Blind` is the only thing
    // this harness does differently, and it does it by declining to ask.
    let card_screen = match card {
        Card::Read => picts.two_colour_card_screen(&cfg),
        Card::Blind => None,
    };
    if let Some((palette, _)) = card_screen {
        app::v6_set_palette(palette);
    }
    let reported =
        card_screen.map(|(_, pair)| pair).or_else(|| cfg.machine_default_colours());
    // SQ-1021/SQ-1022: every per-machine fact in one value.
    let boot = app::machine_boot::MachineBoot::resolve(
        cfg.interpreter_profile,
        &picts,
        named_art_std_window,
        cfg.advertised_interpreter_number(),
        honoured.then_some(reported).flatten(),
        true,
        app::native_font::FaceSet::none(),
    );
    let mut session =
        GameSession::new_for_machine(bytes, honoured, false, false, picture_dims, None, None, &boot)
    .expect("Zork Zero boots off the DOS press");
    assert!(!session.quit && session.machine.fault_trace.is_none(), "{file} booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    for _ in 0..turns {
        let _ = Engine::submit(&mut session, "");
    }
    let _ = std::fs::remove_dir_all(&dir);

    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honoured;
    let elems = Engine::take_transcript_elems(&mut session);
    app::state::apply_transcript_elems(&mut state, &elems);

    let model = session.screen();
    let (story_bg, story_page) = {
        let app::engine::WinNode::Layered(items) = &model.root else {
            panic!("v6 builds a Layered root")
        };
        let story = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT).story;
        let (_, b) = app::render::v6_layout::story_pair_packed(story);
        // `story_bg_rgba` is the very function `render::screen` floods the pane
        // with, so the colour compared below is the one the frame was painted in.
        let page = app::render::v6_layout::story_bg_rgba(story, &state.colors)
            .map(|p| ratatui::style::Color::Rgb(p[0], p[1], p[2]));
        (app::state::unpack_zcolour(b), page)
    };
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, PANE, &mut buf);
    let viewport = state
        .transcript_geom
        .get()
        .expect("hybrid renders window 0 as a terminal transcript")
        .area;
    Some(Frame {
        buf,
        viewport,
        honoured,
        story_bg,
        story_page,
        model,
        state,
        palette: zvm::screen::palette(),
    })
}

/// The boot frame — turn count **zero**, which is every case here but one.
fn frame(file: &str, card: Card) -> Option<Frame> {
    frame_after(file, card, 0)
}

/// How many cells of the story viewport are read on `page`.
fn cells_on(f: &Frame, page: ratatui::style::Color) -> usize {
    let vp = f.viewport;
    (vp.y..vp.bottom())
        .flat_map(|y| (vp.x..vp.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| f.buf.cell((x, y)).is_some_and(|c| c.bg == page))
        .count()
}

/// The plate's own region: every cell of the pane the story viewport does not
/// cover — Zork Zero's ornate border, its flanks and the banner above them.
fn art_region(f: &Frame) -> impl Iterator<Item = (u16, u16)> + '_ {
    let vp = f.viewport;
    (PANE.y..PANE.bottom())
        .flat_map(|y| (PANE.x..PANE.right()).map(move |x| (x, y)))
        .filter(move |&(x, y)| {
            !(x >= vp.x && x < vp.right() && y >= vp.y && y < vp.bottom())
        })
}

/// **How much of the line work survives**, as two numbers and the ground they
/// were counted against.
///
/// A halfblocks picker writes each vertical pixel pair as a cell's foreground over
/// its background, so a cell of the plate carries the two colours that landed
/// there — which makes the drawn buffer an honest oracle for detail (the same
/// reason `v6_float_machine_page` reaches for it).
///
/// **The ground is read off the frame itself**, as the commonest background in the
/// plate's own region, rather than being passed in: the point is how much of the
/// art can be told apart from whatever it ended up sitting on, and comparing two
/// frames against one fixed colour would score the frame whose ground is that
/// colour unfairly low for that reason alone. Returns `(distinct colours,
/// distinguishable cells, ground)`.
struct ArtDetail {
    hues: usize,
    lit: usize,
    ground: ratatui::style::Color,
    /// The commonest colour in the plate's region that is NOT the ground — the
    /// plate's own lit state, which on this card must be its light grey.
    paint: Option<ratatui::style::Color>,
}

fn art_detail(f: &Frame) -> ArtDetail {
    let mut grounds: std::collections::BTreeMap<String, (usize, ratatui::style::Color)> =
        Default::default();
    for (x, y) in art_region(f) {
        if let Some(c) = f.buf.cell((x, y)) {
            let e = grounds.entry(format!("{:?}", c.bg)).or_insert((0, c.bg));
            e.0 += 1;
        }
    }
    let ground = grounds.values().max_by_key(|(n, _)| *n).map(|(_, c)| *c).expect("a drawn pane");

    let mut hues = std::collections::BTreeSet::new();
    let mut paints: std::collections::BTreeMap<String, (usize, ratatui::style::Color)> =
        Default::default();
    let mut lit = 0usize;
    for (x, y) in art_region(f) {
        let Some(c) = f.buf.cell((x, y)) else { continue };
        hues.insert(format!("{:?}", c.fg));
        hues.insert(format!("{:?}", c.bg));
        // A cell shows something only if some part of it is not the ground: an
        // all-ground cell is a hole, whatever glyph is nominally in it.
        let shows = |col: ratatui::style::Color| col != ground;
        if shows(c.bg) || (shows(c.fg) && c.symbol() != " ") {
            lit += 1;
        }
        for col in [c.bg, c.fg] {
            if shows(col) {
                paints.entry(format!("{col:?}")).or_insert((0, col)).0 += 1;
            }
        }
    }
    let paint = paints.values().max_by_key(|(n, _)| *n).map(|(_, c)| *c);
    ArtDetail { hues: hues.len(), lit, ground, paint }
}


// ── The premise ──────────────────────────────────────────────────────────────

/// **Non-vacuity.** Every case here skips without the gitignored press, so
/// something has to fail when the whole suite skips for a reason other than the
/// fixtures being absent — and something has to check that the press really does
/// serve the plate each specimen claims.
#[test]
fn the_press_was_actually_read() {
    let _g = app::v6_palette_at_boot();
    let any = PRESS.iter().any(|(f, _)| present(f));
    let mut seen = 0usize;
    for (file, two_colour) in PRESS {
        let Some(d) = decide(file) else { continue };
        seen += 1;
        assert_eq!(d.profile, InterpreterProfile::IbmPc, "{file}: a DOS press is an IBM PC");
        assert_eq!(d.source, ProfileSource::Medium, "{file}: the medium named the machine");
        assert!(d.machine_pair.is_some(), "{file}: and a medium licenses that machine's colours");
        assert_eq!(
            d.monochrome, *two_colour,
            "{file}: the plate this volume serves — the archive is the only thing that moves \
             across this table",
        );
        assert_eq!(
            d.card, *two_colour,
            "{file}: and a DOS two-colour plate is a CGA CARD, which is the thing the \
             Macintosh's mono archive is not",
        );
    }
    assert!(!any || seen > 0, "the press is on disk but not one volume was read");
}

// ── The rule ─────────────────────────────────────────────────────────────────

/// **THE DEFECT, at the decision that causes it.** The CGA volume of a real DOS
/// press reports the CARD's pair — black 2 under white 9 — where the EGA volume
/// beside it reports the machine's blue 6. Neither declines the story's colours,
/// which is what the `color` command needs and what the first fix took away.
///
/// FALSIFIED by unreading the card (`Card::Blind`, or dropping
/// `PictSource::two_colour_card` from `startup.rs`): every row then reports
/// `(6, 9)`, the CGA rows included, and blue is a colour a two-state display does
/// not have.
///
/// The last row is SQ-0806's rule, which is all that is left of it: a stencil
/// opened beside a bare `.z6` has no machine to state a screen, so the interpreter
/// declares itself colourless and the host theme is the ground. That launch is not
/// on this press — it is pinned in `honor_colours_artwork_pin` — but the licence
/// gate it turns on is asserted here, because this is the file that would break it.
#[test]
fn a_cga_volume_reports_the_cards_pair_and_an_ega_volume_the_machines() {
    let _g = app::v6_palette_at_boot();
    let any = PRESS.iter().any(|(f, _)| present(f));
    let mut seen = 0usize;
    for (file, two_colour) in PRESS {
        let Some(d) = decide(file) else { continue };
        seen += 1;
        assert_eq!(d.machine_pair, Some((6, 9)), "{file}: the IBM PC's own screen");
        assert_eq!(d.two_colour_pair, Some((2, 9)), "{file}: and its two-colour display");
        assert!(
            !d.declines,
            "{file}: a licensed machine keeps the story's colours — `color` needs the flag set",
        );
        assert_eq!(
            d.reported,
            Some(if *two_colour { (2, 9) } else { (6, 9) }),
            "{file}: header $2C/$2D carries the CARD's page, not the machine's",
        );
    }
    assert!(!any || seen > 0, "the press is on disk but not one volume was read");
}

/// **One channel is what moves, and the ink is not it** (SQ-0956).
///
/// The card's pair against the machine's, asserted on the tables rather than on a
/// launch: white 9 both times, and only the page moves, blue 6 to black 2. The ink
/// LOOKS different all the same, because `Palette::IbmCga` resolves that one number
/// to the card's `#AAAAAA` where `IbmYzip` gives `#FFFFFF` — which is
/// `zvm::screen`'s to pin and is pinned there.
#[test]
fn the_cards_pair_is_the_machines_with_one_channel_moved() {
    let pc = InterpreterProfile::IbmPc;
    let (page, ink) = pc.default_colours().expect("SQ-0928: the IBM PC states its pair");
    let (card_page, card_ink) = pc.two_colour_colours().expect("and states its card's page");
    assert_eq!((page, ink), (6, 9), "the machine's screen: blue under white");
    assert_eq!((card_page, card_ink), (2, 9), "its CGA card: BLACK under the same white");
    assert_eq!(ink, card_ink, "one channel — the ink is the card's, unmoved");
    assert_ne!(page, card_page, "…and the page is not");
    assert_eq!(
        (card_ink, card_page),
        zvm::screen::CGA_CARD_PAIR,
        "and it is the same pair `two_colour_card_request` shows, stated once",
    );
}

// ── The screen ───────────────────────────────────────────────────────────────

/// **THE DELIVERABLE, as reported:** *a white page bleeding into the artwork*, on
/// DOS Zork Zero with the CGA rendition.
///
/// Zork Zero issues `set_colour(fg=2, bg=9)` on a window the size of the screen
/// for every video card alike — it cannot see which plate was loaded — so with the
/// card unread that page floods the story viewport and the two-colour border art
/// is read on it. On the colour renditions that is right, and
/// `machine-screenshots/dos-zorkzero.png` shows it. On the CGA plate the machine
/// shows the opposite polarity entirely, and reading the card is what produces it:
/// the pair arrives at `two_colour_card_request`, the card takes the one bit it
/// carries, and window 0 comes out white 9 over black 2 instead of black over
/// white.
///
/// **The RELATION is asserted, never an RGB.** The page goes through the card's
/// palette and the player's theme, so a literal would be pinning the resolver
/// rather than the rule. What is pinned is that the ground the CGA frame is read
/// on is not the ground the story asked for — while the same frame with the card
/// unread (the pre-fix screen, reproduced here on purpose) is covered in it.
///
/// **And then what the user actually reported, which a colour equality cannot
/// see.** On the real machine the black background punches through the plate in
/// its transparent areas; in lanthorn those areas stayed white, the light line
/// work sat on white, *and a lot of detail simply disappeared*. So the second half
/// asks what the plate is PAINTED IN and what it is standing ON, each frame
/// against its own ground. Re-measured on the boot frame, both presses:
///
/// | the card | frame's ground | plate's paint | hues | cells lit |
/// |----------|----------------|---------------|------|-----------|
/// | unread   | `#FFFFFF`      | `#AAAAAA`     | 253  | 1,639     |
/// | read     | `#000000`      | `#AAAAAA`     | 172  | 1,635     |
///
/// **The `lit` count no longer separates the two frames, and that is a real
/// finding rather than a loosened assertion.** It was 1,115 against 1,672 while
/// `CGA_PALETTE`'s lit state was pure `#FFFFFF`: the plate's own paint WAS the
/// story's page, so half the artwork was a hole. The capture says that paint is
/// the card's `#AAAAAA` and the table now says so too, which lifts the unread
/// frame's count on its own — a light-grey plate reads on a white page, badly, and
/// the user has seen exactly that on the real machine after choosing the light
/// ground from Zork Zero's own `color` menu. What separates the frames is the
/// polarity, so that is what is asserted: the plate is the card's grey **on the
/// card's black**, and the hue count falls to a two-colour screen's.
///
/// FALSIFIED by unreading the card: the ground comes back `#FFFFFF`, the story
/// viewport is flooded with it again and `story_bg` is `Standard(9)`. Falsified
/// the other way by restoring `CGA_PALETTE`'s pure white: `paint` comes back
/// `Rgb(255, 255, 255)`, which no pixel of the capture has. Both halves are needed
/// and both are pinned — and reverting BOTH puts the reported frame back exactly,
/// **1,115 cells lit of 251 hues, paint `#000000` on a `#FFFFFF` ground**, which
/// are the figures the first attempt at this quest recorded for the defect. The
/// plate's paint being its own black is the report in one field: everything the
/// artwork drew in white had vanished into the page. The `unread` column is also
/// the non-vacuity guard — if the story ever stops setting that page, this test
/// proves nothing and says so.
#[test]
fn the_storys_white_page_does_not_reach_a_cga_frame() {
    let _g = app::v6_palette_at_boot();
    let any = present(CGA_DISK) || present(CGA_DISK_720);
    let mut seen = 0usize;
    for file in [CGA_DISK, CGA_DISK_720] {
        let Some(blind) = frame(file, Card::Blind) else { continue };
        let Some(shipped) = frame(file, Card::Read) else { continue };
        seen += 1;
        assert!(blind.honoured, "{file}: both frames honour the story's colours");
        assert!(shipped.honoured, "{file}: …including the shipped one — `color` needs the flag");
        assert_eq!(
            blind.story_bg,
            zvm::screen::ZColour::Standard(9),
            "{file}: premise — Zork Zero really does ask for §8.3.1's white page",
        );
        let white = blind.story_page.expect("an honoured page resolves to a colour");
        let flooded = cells_on(&blind, white);
        assert!(
            flooded > 0,
            "{file}: the symptom must reproduce with the card unread, else this proves nothing",
        );

        // The detail next, because it is what was reported and what a colour
        // equality cannot see. Both frames are printed, because the interesting
        // fact is not one number: it is that the plate's own paint and the ground
        // it stands on are the CARD's two colours once the card is read, and two
        // colours the card never had before.
        let before = art_detail(&blind);
        let after = art_detail(&shipped);
        eprintln!(
            "{file}: card unread, {flooded} cells on the story's page {white:?}; art detail — {} \
             cells lit of {} hues, paint {:?} on ground {:?} UNREAD; {} of {}, paint {:?} on \
             ground {:?} READ",
            before.lit, before.hues, before.paint, before.ground,
            after.lit, after.hues, after.paint, after.ground,
        );
        assert_eq!(
            after.ground,
            ratatui::style::Color::Rgb(0, 0, 0),
            "{file}: read, the plate stands on the card's own black page",
        );
        assert_eq!(
            after.paint,
            Some(ratatui::style::Color::Rgb(0xAA, 0xAA, 0xAA)),
            "{file}: …and its paint is the card's light grey, `CGA_PALETTE`'s lit state — \
             which is the `#A0A0A0` the capture measures for every lit pixel it has",
        );
        assert!(
            after.lit > 0 && after.paint != Some(after.ground),
            "{file}: the plate's line work must be TELLABLE APART from the ground — {} cells \
             lit, paint {:?}, ground {:?}",
            after.lit,
            after.paint,
            after.ground,
        );
        // The card unread is the reported screen, and this is where it differs:
        // the same paint on the story's white page, which is the washed-out
        // polarity the user also saw on the real machine when they chose it from
        // the game's own `color` menu. Half a shade of contrast where the card
        // gives a full one.
        assert_eq!(before.ground, white, "{file}: unread, the plate sits on the story's page");
        assert_ne!(after.ground, white, "{file}: read, it does not");

        // …and the ground itself, which is what makes the detail above possible.
        assert_eq!(
            shipped.story_bg,
            zvm::screen::ZColour::Standard(2),
            "{file}: the card took the bit — the story's white page is its BLACK one",
        );
        assert_eq!(
            cells_on(&shipped, white),
            0,
            "{file}: not one cell of the story's page on a two-colour frame — {flooded} of them \
             before the card was read",
        );
    }
    assert!(!any || seen > 0, "a CGA volume is on disk but no frame was rendered");
}

/// **Every surface that fills a transparent hole takes the same page.**
///
/// `Picture::rgba_with` drops a two-colour plate's clear index to alpha 0, and
/// three different places resolve that hole against "the page of the moment":
/// `v6_layout::flatten_onto_page` for the raster composite,
/// `fill_story_page_clear`/`fill_window_pages` for the hybrid ring, and
/// `inline_image::float_page` for a story float. Fixing the page fixes all three
/// only for as long as none of them keeps a copy — so this asks each of them
/// directly rather than trusting that they still share the gate.
///
/// The raster composite is the sharpest of the three, because it flattens its
/// WHOLE canvas opaque before shipping: every hole in the plate becomes a real
/// pixel of the page. It is measured in PIXELS, and with the same detail oracle
/// as the ring — **not** by counting the story's white, which cannot tell the
/// bleed apart from the plate's own paint. Counting the story's white is exactly
/// the trap the user's report describes from the other side — light line work on a
/// light ground — which is why "how much can be told apart from the ground" is the
/// honest question here too.
///
/// Re-measured on the boot frame, 360K disk 1 and 720K disk 2 alike:
///
/// | the card | composite's ground | colours | pixels lit |
/// |----------|--------------------|---------|------------|
/// | unread   | `#FFFFFF`          | 3       | 80,425     |
/// | read     | `#000000`          | 3       | 61,341     |
///
/// The lit count is HIGHER with the card unread and that is arithmetic, not a
/// regression: on a white ground both of the plate's states differ from it, on a
/// black ground only its light one does. What the count cannot say and the ground
/// can is which of them the machine shows, so the ground is what is asserted, and
/// beside it the three values themselves — `#000000`, the plate's `#AAAAAA` and the
/// ink's `#ADADAD`, every one a grey. The capture's own summary is "161 distinct
/// colours, a grey ramp from video scaling, with no second hue anywhere in the
/// frame"; a composite carries no video scaling, so the honest form of that claim
/// here is that nothing in it has a hue at all.
#[test]
fn the_raster_composite_and_the_floats_take_the_same_page_as_the_ring() {
    let _g = app::v6_palette_at_boot();
    let any = present(CGA_DISK) || present(CGA_DISK_720);
    let mut seen = 0usize;
    for file in [CGA_DISK, CGA_DISK_720] {
        let Some(blind) = frame(file, Card::Blind) else { continue };
        let Some(shipped) = frame(file, Card::Read) else { continue };
        seen += 1;
        let white = blind.story_page.expect("an honoured page resolves to a colour");
        let ratatui::style::Color::Rgb(r, g, b) = white else { panic!("{file}: an RGB page") };
        let white_px = image::Rgba([r, g, b, 255]);

        let mut raster = Vec::new();
        for (what, f, want_white) in [("unread", &blind, true), ("read", &shipped, false)] {
            // The raster composite, built exactly as `render::screen` builds it.
            let app::engine::WinNode::Layered(items) = &f.model.root else {
                panic!("v6 builds a Layered root")
            };
            app::v6_set_palette(f.palette);
            let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
            let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
            let (canvas, _) =
                app::render::screen::build_v6_raster_canvas(&layout, native, &f.state);
            let mut tally: std::collections::BTreeMap<[u8; 4], usize> = Default::default();
            for p in canvas.pixels() {
                *tally.entry(p.0).or_default() += 1;
            }
            let ground = *tally.iter().max_by_key(|(_, n)| **n).expect("a built canvas").0;
            let lit: usize = tally.iter().filter(|(c, _)| **c != ground).map(|(_, n)| *n).sum();
            if !want_white {
                // The shipped composite is a two-state screen: every value in it is
                // one of the card's greys, which is the capture's "no second hue"
                // asserted in the form a composite can carry (it has no video
                // scaling to spread a ramp).
                for c in tally.keys() {
                    assert!(
                        c[0] == c[1] && c[1] == c[2],
                        "{file}: a CGA composite has no hue in it — found {c:?}",
                    );
                }
                assert_eq!(
                    tally.get(&[0xAA, 0xAA, 0xAA, 255]).copied().unwrap_or(0),
                    45_688,
                    "{file}: the plate's own paint, in the card's light grey",
                );
            }
            let flooded = tally.get(&white_px.0).copied().unwrap_or(0);
            eprintln!(
                "{file} ({what}): raster ground {ground:?}, {lit} px lit of {} colours, {flooded} \
                 px of the story's white",
                tally.len(),
            );
            assert_eq!(
                ground == white_px.0,
                want_white,
                "{file} ({what}): the composite's own ground is the page of the moment",
            );
            raster.push(lit);

            // …and the ground a story float is flattened onto — layered by
            // `inline_image::float_page` over the very same story page.
            let float = app::render::inline_image::float_page(&f.state);
            assert_eq!(
                float == Some(white_px),
                want_white,
                "{file} ({what}): a float's ground must be the same page — got {float:?}",
            );
        }
        assert!(
            raster[0] > 0 && raster[1] > 0,
            "{file}: both frames must have artwork in them at all — {} px lit unread, {} px read",
            raster[0],
            raster[1],
        );
    }
    assert!(!any || seen > 0, "a CGA volume is on disk but no frame was rendered");
}

/// **The pair is settled BEFORE the story loads, and nothing re-derives it**
/// (SQ-0956, the user's own requirement: *"setup our colors before the story is
/// loaded, then we shouldn't have a bunch of different paths when the story is
/// actually running"*).
///
/// Two halves, and the first is an ORDERING that source alone can state. In
/// `startup.rs` the host pair is bound sixty lines before the archive that decides
/// it exists — `host_default_colours` at the top, `picts` further down — so a
/// change made where the binding *reads* would keep the EGA pair and the bare-`.z6`
/// pins would not notice, because there is no archive there and the answer is the
/// same either way. The card's pair is therefore assigned after `picts` and before
/// `GameSession::new_for_machine`, which is the call that runs the story to its
/// first prompt. This case reads the file and asserts that order.
///
/// SQ-1022 added a link in the middle: `MachineBoot::resolve` now TAKES the pair
/// and carries it into the constructor, so the chain is assign → resolve → boot
/// and this case pins all three. That is strictly stronger than before — the pair
/// can no longer be settled between the resolve and the boot and be silently
/// dropped, because the constructor no longer accepts it as a separate argument.
///
/// The second half is a COUNT: `PictSource::two_colour_card_screen` is the only
/// thing that decides the pair, and it has exactly two callers — `startup.rs` at
/// boot and `reset.rs` on an `@restart`, both of which rebuild the session. Nothing
/// in `render/`, nothing per-frame, nothing the running game can reach. The runtime
/// poller that DOES write `$2C`/`$2D` (`loop_tick::poll_zvm_default_colours`)
/// returns early on any launch with a licensed machine, and a launch that reaches
/// the card is licensed by construction — so it has already returned.
///
/// A source-level case for the same reason `palette_lock_discipline` is one: the
/// hazard is an ABSENCE (a caller that should not exist, an assignment that moved
/// back up), and no frame can be rendered that shows it.
#[test]
fn the_cards_pair_is_settled_before_the_story_loads() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let startup = std::fs::read_to_string(root.join("crates/app/src/startup.rs"))
        .expect("startup.rs is in the tree");
    let at = |needle: &str| {
        startup.find(needle).unwrap_or_else(|| panic!("startup.rs no longer contains {needle:?}"))
    };
    let picts = at("let mut picts = ");
    let decide = at("picts.two_colour_card_screen(&cfg)");
    let assign = at("host_default_colours = Some(pair)");
    let resolve = at("MachineBoot::resolve(");
    let boot = at("GameSession::new_for_machine(");
    assert!(
        picts < decide,
        "the archive must exist before it can name a card — the decision moved above `picts`",
    );
    assert!(
        assign < boot,
        "the pair must be in hand before the constructor runs the story to its first prompt",
    );
    assert!(decide < assign && assign < boot, "…and in that order");
    assert!(
        assign < resolve && resolve < boot,
        "the pair is carried into the constructor by `MachineBoot`, so it must be in \
         hand before the resolve, which must precede the boot (SQ-1022)",
    );

    // The whole workspace, so a third caller anywhere fails this rather than
    // quietly re-deriving the pair while a game runs.
    let mut callers: Vec<String> = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("target")) {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                for line in text.lines() {
                    // The definition, its doc comment and this very case are not
                    // calls; a call is the method on a value.
                    if line.contains(".two_colour_card_screen(") && !line.trim_start().starts_with("//") {
                        // Normalise the separator: on Windows `display()` yields
                        // `crates\app\tests\...`, so the `/tests/` filter below
                        // matched nothing and every test file counted as production.
                        let shown = path.display().to_string().replace('\\', "/");
                        callers.push(format!("{shown}: {}", line.trim()));
                    }
                }
            }
        }
    }
    let production: Vec<&String> =
        callers.iter().filter(|c| !c.contains("/tests/")).collect();
    assert_eq!(
        production.len(),
        2,
        "the pair is decided in ONE function with exactly two callers — boot and \
         restart, both of which rebuild the session. Found:\n{}",
        callers.join("\n"),
    );
    assert!(
        production.iter().any(|c| c.contains("startup.rs"))
            && production.iter().any(|c| c.contains("reset.rs")),
        "…and they are `startup.rs` and `reset.rs`. Found:\n{}",
        callers.join("\n"),
    );

    // And the runtime poller stands down for a licensed machine, which every
    // launch showing a card is.
    let loop_tick = std::fs::read_to_string(root.join("crates/app/src/loop_tick.rs"))
        .expect("loop_tick.rs is in the tree");
    let poller = loop_tick
        .split("pub(crate) fn poll_zvm_default_colours")
        .nth(1)
        .expect("the poller is still there");
    let body = &poller[..poller.find("\n}\n").expect("its end")];
    assert!(
        body.contains("state.config.machine_default_colours().is_some()")
            && body.contains("return"),
        "the only runtime writer of $2C/$2D must still stand down for a licensed \
         machine, or a CGA launch's pair would be overwritten mid-run",
    );
}

/// **The `color` command, which is the regression this quest was reopened for**
/// (SQ-0956, and SQ-0957's mechanism).
///
/// The first fix declared the interpreter colourless. Zork Zero checks that flag
/// before it does any colour work at all — with it clear the boot press issues no
/// `set_colour` whatsoever, measured on this very volume — so the in-game `color`
/// command had nothing to act through. The flag is set again, which is asserted
/// two cases up; what this one pins is that a colour change still MOVES THE
/// GROUND, and that the plate's transparent holes come with it.
///
/// **Driven at the model rather than through the menu.** Zork Zero's `color`
/// picker is a graphical menu with no prose in the transcript, and driving it
/// headlessly would be pinning a key sequence rather than a colour rule. So the
/// swap is applied where the opcode applies it — every window's own pair, set to
/// the OTHER side of the card's one bit — and the composite is rebuilt. That is
/// exactly the state `two_colour_card_request(Some(2), Some(9))` produces, which
/// `zvm::screen`'s own tests pin from the opcode end.
///
/// **Perturb, then assert.** The frame immediately after a change is not the
/// question; the question is whether the surfaces that resolve a transparent pixel
/// went and looked again.
///
/// The swapped state is the washed-out one — a light plate on a light ground,
/// which the user confirmed looks the same way on the real machine — and that is
/// the point: it is the player's choice to make, and lanthorn now lets the game
/// offer it. Nothing here claims it looks good.
#[test]
fn a_story_colour_change_moves_the_ground_the_plate_stands_on() {
    let _g = app::v6_palette_at_boot();
    let any = present(CGA_DISK) || present(CGA_DISK_720);
    let mut seen = 0usize;
    for file in [CGA_DISK, CGA_DISK_720] {
        let Some(mut f) = frame(file, Card::Read) else { continue };
        seen += 1;
        app::v6_set_palette(f.palette);
        let ground = |f: &Frame| {
            let app::engine::WinNode::Layered(items) = &f.model.root else {
                panic!("v6 builds a Layered root")
            };
            let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
            let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
            let (canvas, _) =
                app::render::screen::build_v6_raster_canvas(&layout, native, &f.state);
            let mut tally: std::collections::BTreeMap<[u8; 4], usize> = Default::default();
            for p in canvas.pixels() {
                *tally.entry(p.0).or_default() += 1;
            }
            let g = *tally.iter().max_by_key(|(_, n)| **n).expect("a built canvas").0;
            (g, tally.get(&g).copied().unwrap_or(0))
        };
        let (before, before_n) = ground(&f);
        assert_eq!(before, [0, 0, 0, 255], "{file}: the card boots on its own black page");

        // The swap, applied where `@set_colour` applies it.
        let app::engine::WinNode::Layered(items) = &mut f.model.root else {
            panic!("v6 builds a Layered root")
        };
        let mut moved = 0usize;
        for it in items.iter_mut() {
            let pair = app::render::v6_layout::story_pair_packed(Some(it));
            if pair == (0, 0) {
                continue; // a window that named no colour is not the story's to swap
            }
            match &mut it.node {
                app::engine::WinNode::Grid(g) => {
                    g.fg = Some(app::state::pack_zcolour(zvm::screen::ZColour::Standard(2)));
                    g.bg = Some(app::state::pack_zcolour(zvm::screen::ZColour::Standard(9)));
                    moved += 1;
                }
                app::engine::WinNode::Buffer(b) => {
                    b.fg = Some(app::state::pack_zcolour(zvm::screen::ZColour::Standard(2)));
                    b.bg = Some(app::state::pack_zcolour(zvm::screen::ZColour::Standard(9)));
                    moved += 1;
                }
                _ => {}
            }
        }
        assert!(moved > 0, "{file}: premise — the story coloured some window to swap");

        let (after, after_n) = ground(&f);
        eprintln!(
            "{file}: composite ground {before:?} ({before_n} px) before the swap, {after:?} \
             ({after_n} px) after"
        );
        assert_ne!(after, before, "{file}: a colour change must move the ground — SQ-0957");
        assert_eq!(
            after,
            [0xAD, 0xAD, 0xAD, 255],
            "{file}: …to the other side of the card's one bit, colour 9 through `IbmCga`",
        );
    }
    assert!(!any || seen > 0, "a CGA volume is on disk but no frame was rendered");
}

/// **And the colour renditions of the same release are untouched** — which is the
/// user's own constraint: white is right for Zork Zero on a PC, just not for CGA.
///
/// Same press, same release, same machine; only the plate differs. The EGA and
/// MCGA volumes must still honour the story's page, and their frames must still be
/// read on it.
#[test]
fn the_colour_renditions_keep_the_white_page_they_asked_for() {
    let _g = app::v6_palette_at_boot();
    let any = present(EGA_DISK) || present(MCGA_DISK_720);
    let mut seen = 0usize;
    for file in [EGA_DISK, MCGA_DISK_720] {
        let Some(f) = frame(file, Card::Read) else { continue };
        seen += 1;
        assert!(f.honoured, "{file}: a sixteen-colour plate has colours to give");
        assert_eq!(
            f.story_bg,
            zvm::screen::ZColour::Standard(9),
            "{file}: the story asks for the same white it asks for on every card",
        );
        let white = f.story_page.expect("an honoured page resolves to a colour");
        assert!(
            cells_on(&f, white) > 0,
            "{file}: and it reaches the screen, exactly as `dos-zorkzero.png` shows it",
        );
    }
    assert!(!any || seen > 0, "a colour volume is on disk but no frame was rendered");
}
