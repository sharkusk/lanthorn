//! SQ-0957 — a picture's transparent index is a HOLE, not a colour, so the ground
//! the hole shows is whatever the ground IS. Driven here through the one thing a
//! player actually touches: **Zork Zero's own `color` command.**
//!
//! # The question, and why the model-level pin did not answer it
//!
//! `Picture::rgba_with` drops a plate's transparent index to alpha 0
//! (`blorb::infocom_pics`), and `blit_clipped` skips any source pixel under alpha
//! 128 — so a transparent pixel never reaches the composite at all, and the page
//! flood behind it (`v6_layout::flatten_onto_page` for raster,
//! `fill_story_page_clear`/`fill_window_pages` for hybrid) is what the player sees
//! there. The quest asks whether a MID-GAME colour change moves those holes with
//! the ground.
//!
//! SQ-0956 pinned that the ground moves, in
//! `v6_cga_stencil_page::a_story_colour_change_moves_the_ground_the_plate_stands_on`
//! — but it applied the swap **at the model**, reaching into the window tree and
//! setting `fg`/`bg` where `@set_colour` would have. That proves the renderer, not
//! the game. This file drives the story's own command instead, so what is measured
//! is the sequence a player types.
//!
//! # What `color` actually is
//!
//! Not a graphical menu — the premise two earlier rounds of this quest ran on, and
//! the reason both concluded "no harness can drive it". It is **one rule with two
//! presentations**, and which one you get is decided by whether the interpreter
//! told the story it has colours (§8.3.2):
//!
//! | interpreter says | what `color` offers | keys |
//! |------------------|---------------------|------|
//! | colours (`honor_game_colours = true`)  | a confirm, a six-entry ink menu, a six-entry page menu, a confirm | `color` `y` `<1-6>` `<1-6>` `y` |
//! | no colours (`honor_game_colours = false`) | a confirm, then the only other pair there is | `color` `y` `y` |
//!
//! Both are "pick any two of the colours available, as long as they differ"; with
//! no colours available that collapses to the swap. The six entries are the game's
//! own numbering, and they are §8.3.1 numbers with `WHITE` moved to the front:
//! `1 WHITE`(9) `2 BLACK`(2) `3 RED`(3) `4 GREEN`(4) `5 YELLOW`(5) `6 BLUE`(6).
//! This file types **5 then 6** — yellow ink on a blue page — because it is the
//! pair furthest from every default on every press here.
//!
//! **And the prose arrives in `TurnResult::transcript`, not in
//! `Engine::take_transcript_elems`.** That is the whole of the earlier "no
//! transcript prose, the command must be graphical" dead end: on this press the
//! elems stream is empty for every turn after boot — `version` and a nonsense verb
//! come back just as blank — while the turn result carries the text. A harness
//! that reads only the elems sees nothing and concludes the wrong thing.
//!
//! # Specimens
//!
//! One release, four volumes, and the archive is the only thing that moves — the
//! same control `v6_cga_stencil_page` uses, because which plate a launch gets is
//! decided by the disk the player put in. No `--pictures` anywhere.
//!
//! | volume              | archive | palette    | boot page | after `color` 5/6 |
//! |---------------------|---------|------------|-----------|-------------------|
//! | 360K **Disk 1**     | `.CG1`  | `IbmCga`   | `#000000` | `#ADADAD`         |
//! | 720K **Disk 2**     | `.CG1`  | `IbmCga`   | `#000000` | `#ADADAD`         |
//! | 360K **Disk 3**     | `.EG1`  | `IbmYzip`  | `#FFFFFF` | `#0000AD`         |
//! | 720K **Disk 1**     | `.MG1`  | `IbmYzip`  | `#FFFFFF` | `#0000AD`         |
//!
//! The two-colour presses land on the other side of the card's one bit rather than
//! on the blue they asked for — `zvm::screen::two_colour_card_request`, SQ-0956 —
//! which is why the CGA rows read `#ADADAD` and not `#0000AD`. The colour presses
//! take the named pair as named. Both are the ground moving; the quest is only
//! whether the holes move with it, and on all four they do.
//!
//! **Turn count: one line and four keypresses**, all of them the `color` exchange
//! — the boot banner is the "before" frame and the command is every turn after it.
//! Nothing is walked into the game, because a colour change is a colour change
//! wherever the player stands, and every extra turn is another way for the frame
//! to differ.
//!
//! **The swapped state looks washed out, and that is correct.** The user's emulator
//! run says a real CGA machine washes out the same way: the `.cg1` plates are light
//! line work authored for a black ground. Nothing here is trying to improve it.
//!
//! # Which layer, and why not the pty
//!
//! The cheapest layer answers it, on both v6 render modes, because **the ground is
//! resolved into a surface before any backend sees it**. Raster flattens its whole
//! canvas opaque (`flatten_onto_page`); hybrid ships bands and floods the story
//! page into them first (`fill_story_page_clear`, whose own comment records that a
//! pixel left transparent there "is the TERMINAL's to resolve" — the defect that
//! fix existed to close). So the RGBA handed to kitty, sixel and half-blocks alike
//! already carries the answer, and `pty_capture` would be resolving bytes whose
//! colours were decided in-process. The escalation exists to tell an image
//! PLACEMENT from a background PAINTED into cells; this quest asks what colour a
//! hole came out, which is the same number in either. Both surfaces are read below
//! anyway — the raster composite pixel-for-pixel, and the hybrid frame through a
//! half-blocks picker, which paints the art into cells.
//!
//! The DOS press is gitignored commercial media, so every case skips vacuously
//! without it and [`the_press_was_actually_read`] is what stops the file quietly
//! passing on a machine that has none of it.
//!
//! **Palette**: every case here asserts a resolved colour, and the table it reads
//! through is the SESSION's own (SQ-0958, SQ-1393) — not `Standard`, but whatever
//! the volume's own archive names, pinned per press in `PRESSES` and carried into
//! the `MachineBoot` and the `ColorScheme` by the boot below.

use std::path::PathBuf;

use app::engine::{Engine, KeyInput, WinNode};
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};
use app::state::AppState;

use image::RgbaImage;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const RELEASE: u16 = 393;
const SERIAL: &[u8] = b"890714";

const CGA_360: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 1) [!].ima";
const EGA_360: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 3) [!].ima";
const CGA_720: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (720K) (Disk 2) [!].ima";
const MCGA_720: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (720K) (Disk 1) [!].ima";

/// One volume, and the two grounds the `color` exchange moves it between.
struct Press {
    file: &'static str,
    /// The archive the medium serves — named so a failure says which plate.
    archive: &'static str,
    /// The palette that archive installs, which every colour below resolves through.
    palette: zvm::screen::Palette,
    /// The page a plate's holes stand on at boot.
    boot: [u8; 4],
    /// …and after `color` asks for yellow ink on a blue page.
    after: [u8; 4],
}

const PRESSES: &[Press] = &[
    Press {
        file: CGA_360,
        archive: ".CG1",
        palette: zvm::screen::Palette::IbmCga,
        boot: [0x00, 0x00, 0x00, 255],
        after: [0xAD, 0xAD, 0xAD, 255],
    },
    Press {
        file: CGA_720,
        archive: ".CG1",
        palette: zvm::screen::Palette::IbmCga,
        boot: [0x00, 0x00, 0x00, 255],
        after: [0xAD, 0xAD, 0xAD, 255],
    },
    Press {
        file: EGA_360,
        archive: ".EG1",
        palette: zvm::screen::Palette::IbmYzip,
        boot: [0xFF, 0xFF, 0xFF, 255],
        after: [0x00, 0x00, 0xAD, 255],
    },
    Press {
        file: MCGA_720,
        archive: ".MG1",
        palette: zvm::screen::Palette::IbmYzip,
        boot: [0xFF, 0xFF, 0xFF, 255],
        after: [0x00, 0x00, 0xAD, 255],
    },
];

/// A pane roomy enough for Zork Zero's chrome ring plus a real story viewport —
/// the same one `v6_cga_stencil_page` measures in.
const PANE: Rect = Rect { x: 0, y: 0, width: 120, height: 45 };

/// The game's own menu numbers for yellow ink and a blue page.
const YELLOW: char = '5';
const BLUE: char = '6';

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn present(file: &str) -> bool {
    stories_dir().join(file).exists()
}

/// One booted press, at the boot banner, with the state the frame is drawn from.
struct Booted {
    session: GameSession,
    state: AppState,
}

/// Boot one volume the way `startup.rs` boots: the profile from the medium the
/// MOUNT returned, the palette from the archive's card, and the four screen-size
/// links in order (`picts.std_window()` → the named archive → `native_std_window`
/// → `profile.std_window()`) with `art_scale` alongside. Skipping any of them lays
/// the game's windows out differently and every rect measured afterwards is of a
/// screen nobody sees (SQ-0901/0883/0899), so the decision is printed.
///
/// `user_honours` is the player's `honor_game_colours`, which is the only knob:
/// `false` is `--game-colours off`, and it is what makes `color` show its other face.
fn boot(file: &str, user_honours: bool) -> Option<Booted> {
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

    let dir = app::scratch_dir("sq957");
    let over = PictureOverride::resolve_with_session(&path, &dir, None);
    let named_art_std_window = over.std_window();
    let (profile, source) =
        InterpreterProfile::resolve_with_source(&path, None, over.flavour(), None);
    // The table this press resolves colour numbers through — the machine's own,
    // then the CARD's where the archive names one, exactly as `startup.rs`
    // settles it (SQ-1393).
    let base_palette =
        zvm::interpreter::palette_for(profile.row_number(), bytes.first().copied());
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let picture_dims = picts.all_pict_dims();

    let cfg = app::config::Config {
        interpreter_profile: profile,
        interpreter_source: source,
        ..Default::default()
    };
    // `startup.rs`'s order: the archive may decline colours outright, and then the
    // CARD it is showing states the screen (SQ-0956) — through the shipped
    // functions, so a regression in either fails here.
    let honoured = user_honours && !picts.declines_game_colours(cfg.machine_default_colours());
    let card_screen = picts.two_colour_card_screen(&cfg);
    let machine_palette = card_screen.map(|(p, _)| p).unwrap_or(base_palette);
    let reported = card_screen.map(|(_, pair)| pair).or_else(|| cfg.machine_default_colours());
    let boot = app::machine_boot::MachineBoot::resolve(
        cfg.interpreter_profile,
        &picts,
        named_art_std_window,
        cfg.advertised_interpreter_number(),
        honoured.then_some(reported).flatten(),
        true,
        app::native_font::FaceSet::none(),
        machine_palette,
        None,
    );
    eprintln!(
        "{file}: r{RELEASE} profile={profile:?}/{source:?} screen={:?} \
         art_scale={:?} palette={:?} honoured={honoured} reported={reported:?}",
        boot.screen_px,
        boot.art_scale,
        boot.palette,
    );

    // SQ-1021/SQ-1022: every per-machine fact in one value.
    let mut session =
        GameSession::new_for_machine(bytes, honoured, false, false, picture_dims, None, None, &boot)
    .expect("Zork Zero boots off the DOS press");
    assert!(!session.quit && session.machine.fault_trace.is_none(), "{file} booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = std::fs::remove_dir_all(&dir);

    let mut state = AppState::default();
    // The scheme resolves standard colour numbers through the same table the
    // machine does — the card's, on a press that has one (SQ-1393).
    state.colors = app::colors::ColorScheme::terminal_default_in(machine_palette);
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honoured;
    let elems = Engine::take_transcript_elems(&mut session);
    app::state::apply_transcript_elems(&mut state, &elems);
    Some(Booted { session, state })
}

/// Type one thing at whatever the game is waiting for, and hand back what it said.
///
/// `color` reads its answers with `read_char` (§10.7) and its invocation with
/// `read`, so the harness has to follow the game between the two rather than
/// assume one — and the reply comes back on `TurnResult::transcript`, which is the
/// window this quest's earlier rounds were not reading.
fn say(b: &mut Booted, k: KeyInput) -> String {
    match Engine::pending_input(&b.session) {
        InputKind::Char => Engine::submit_key(&mut b.session, k)
            .expect("a keypress the game is waiting for is always accepted")
            .transcript,
        InputKind::Line => {
            let line = match k {
                KeyInput::Char(c) => c.to_string(),
                _ => String::new(),
            };
            Engine::submit(&mut b.session, &line).transcript
        }
        other => panic!("the game is waiting on {other:?}, which the Z-machine never produces"),
    }
}

/// Say a whole word at a line prompt.
fn command(b: &mut Booted, line: &str) -> String {
    assert!(
        matches!(Engine::pending_input(&b.session), InputKind::Line),
        "a command needs a line prompt",
    );
    Engine::submit(&mut b.session, line).transcript
}

/// The `expect` idea from SQ-0942: nothing is measured until the screen has SAID
/// it is the thing being measured. Two blank keypresses once picked a different
/// game off a neighbouring disk under a caption about this one.
#[track_caller]
fn expect(what: &str, said: &str, needle: &str) {
    assert!(
        said.contains(needle),
        "{what}: expected {needle:?} on screen, got {said:?}",
    );
}

/// Drive `color` all the way to the pair being applied, asserting every prompt on
/// the way. Returns the four replies so a case can pin the wording it relied on.
///
/// The colour form: confirm, ink, page, confirm.
fn choose_a_pair(b: &mut Booted, file: &str) -> Vec<String> {
    let mut said = Vec::new();

    let r = command(b, "color");
    expect(file, &r, "Aesthetically, we recommend not changing the standard setting");
    expect(file, &r, "black text on a white background");
    expect(file, &r, "(Y or N)");
    said.push(r);

    let r = say(b, KeyInput::Char('y'));
    expect(file, &r, "The current text color is black.");
    expect(file, &r, "1 --> WHITE");
    expect(file, &r, "5 --> YELLOW");
    expect(file, &r, "select the text color");
    said.push(r);

    let r = say(b, KeyInput::Char(YELLOW));
    expect(file, &r, "The current background color is white.");
    expect(file, &r, "6 --> BLUE");
    expect(file, &r, "select the background color");
    said.push(r);

    let r = say(b, KeyInput::Char(BLUE));
    expect(file, &r, "You should now get yellow text on a blue background.");
    expect(file, &r, "(Y or N)");
    said.push(r);

    // The confirm is the turn that applies it; the game returns to its prompt.
    let r = say(b, KeyInput::Char('y'));
    assert!(
        matches!(Engine::pending_input(&b.session), InputKind::Line),
        "{file}: confirming the pair hands the player back their command prompt",
    );
    said.push(r);
    said
}

/// Every composite coordinate a graphics window's OWN canvas leaves transparent.
///
/// This is the plate's stencil, read off the window rather than guessed from the
/// frame: `blit_clipped` copies a source pixel only at alpha 128 or above, so
/// these are exactly the pixels no picture put anything at, and exactly the ones
/// the page flood is left to answer for. Keyed by window id, because Zork Zero
/// draws two — the full-screen plate (7) and the banner strip (1) — and only the
/// first has the story's page beneath it.
fn stencil(b: &mut Booted) -> Vec<(u32, Vec<(u32, u32)>)> {
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let mut out = Vec::new();
    for it in items {
        let WinNode::Graphics(g) = &it.node else { continue };
        let w = g.canvas.width().min(it.w_px.max(1) as u32);
        let h = g.canvas.height().min(it.h_px.max(1) as u32);
        let mut holes = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if g.canvas.get_pixel(x, y)[3] < 128 {
                    holes.push((it.x_px as u32 + x, it.y_px as u32 + y));
                }
            }
        }
        out.push((g.win, holes));
    }
    out
}

/// The same, for the pixels a plate DID paint — the control that tells "the holes
/// followed the ground" apart from "everything was recoloured".
fn ink(b: &mut Booted, win: u32) -> Vec<(u32, u32)> {
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let mut out = Vec::new();
    for it in items {
        let WinNode::Graphics(g) = &it.node else { continue };
        if g.win != win {
            continue;
        }
        let w = g.canvas.width().min(it.w_px.max(1) as u32);
        let h = g.canvas.height().min(it.h_px.max(1) as u32);
        for y in 0..h {
            for x in 0..w {
                if g.canvas.get_pixel(x, y)[3] >= 128 {
                    out.push((it.x_px as u32 + x, it.y_px as u32 + y));
                }
            }
        }
    }
    out
}

/// The raster composite, built the way `render::screen` builds it.
fn composite(b: &mut Booted) -> RgbaImage {
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    app::render::screen::build_v6_raster_canvas(&layout, native, &b.state).0
}

/// What colour a set of coordinates came out, commonest first.
fn census(canvas: &RgbaImage, at: &[(u32, u32)]) -> Vec<([u8; 4], usize)> {
    let mut tally: std::collections::BTreeMap<[u8; 4], usize> = Default::default();
    for &(x, y) in at {
        if x < canvas.width() && y < canvas.height() {
            *tally.entry(canvas.get_pixel(x, y).0).or_default() += 1;
        }
    }
    let mut v: Vec<_> = tally.into_iter().collect();
    v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    v
}

/// The ART REGION's commonest cell background in the HYBRID frame — every cell of
/// the pane the story viewport does not cover, which is Zork Zero's ring, its
/// flanks and its banner.
///
/// Drawn through a half-blocks picker, which paints the art into cells and makes
/// the drawn buffer an honest oracle for what the ground under it resolved to
/// (the same reason `v6_cga_stencil_page` reaches for one).
fn hybrid_art_ground(b: &mut Booted) -> (ratatui::style::Color, usize) {
    let model = b.session.screen();
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, false, None, &b.state, PANE, &mut buf);
    let vp = b
        .state
        .transcript_geom
        .get()
        .expect("hybrid renders window 0 as a terminal transcript")
        .area;
    let mut tally: std::collections::BTreeMap<String, (usize, ratatui::style::Color)> =
        Default::default();
    for y in PANE.y..PANE.bottom() {
        for x in PANE.x..PANE.right() {
            if x >= vp.x && x < vp.right() && y >= vp.y && y < vp.bottom() {
                continue;
            }
            if let Some(c) = buf.cell((x, y)) {
                tally.entry(format!("{:?}", c.bg)).or_insert((0, c.bg)).0 += 1;
            }
        }
    }
    let (n, col) = *tally.values().max_by_key(|(n, _)| *n).expect("a drawn pane");
    (col, n)
}

/// **The quest.** A colour change the player made, through the game's own command,
/// moves the plate's transparent holes onto the new ground.
///
/// The plate is never redrawn and never asked about — its own canvas is
/// byte-identical either side of the exchange, because nothing about the picture
/// changed. What changed is the page underneath it, and the holes are the page.
///
/// **Falsified** by memoizing `v6_layout::story_bg_rgba` in a `OnceLock` — the
/// ground of the first moment, kept, which is the defect exactly as the quest
/// titles it. Both this case and [`the_hybrid_frame_resolves_the_new_ground_too`]
/// then fail on the reported symptom ("the holes are still showing the ground the
/// plate was composited against before the player changed it"), while the two
/// cases below stay green — the declined-colours ground was never going to move,
/// and the media guard reads no colour at all.
#[test]
fn the_games_own_color_command_moves_the_plates_transparent_holes() {
    let any = PRESSES.iter().any(|p| present(p.file));
    let mut seen = 0usize;
    for p in PRESSES {
        let Some(mut b) = boot(p.file, true) else { continue };
        seen += 1;
        assert_eq!(
            b.session.machine.palette(),
            p.palette,
            "{}: the {} archive installs its own table, and every colour below is read \
             through it (SQ-0958)",
            p.file,
            p.archive,
        );

        // The stencil, and the boot ground it stands on.
        let holes = stencil(&mut b);
        let (_, plate) = holes
            .iter()
            .max_by_key(|(_, h)| h.len())
            .expect("Zork Zero paints at least one graphics window");
        assert!(
            plate.len() > 100_000,
            "{}: non-vacuity — the boot frame's plate must be mostly stencil, got {} holes",
            p.file,
            plate.len(),
        );
        let painted = ink(&mut b, holes.iter().max_by_key(|(_, h)| h.len()).unwrap().0);
        assert!(!painted.is_empty(), "{}: …and must have drawn something too", p.file);

        let before = composite(&mut b);
        let was = census(&before, plate);
        let ink_was: Vec<[u8; 4]> =
            painted.iter().map(|&(x, y)| before.get_pixel(x, y).0).collect();
        assert_eq!(
            was[0].0, p.boot,
            "{}: the plate's holes boot on the page this press shows — census {:?}",
            p.file,
            &was[..was.len().min(3)],
        );

        let said = choose_a_pair(&mut b, p.file);
        assert_eq!(said.len(), 5, "{}: `color` asks four questions and applies", p.file);

        let after = composite(&mut b);
        let now = census(&after, plate);
        eprintln!(
            "{} ({}): holes {:?} -> {:?}",
            p.file,
            p.archive,
            &was[..was.len().min(2)],
            &now[..now.len().min(2)],
        );
        assert_ne!(
            now[0].0, was[0].0,
            "{}: the holes are still showing the ground the plate was composited against \
             before the player changed it — SQ-0957",
            p.file,
        );
        assert_eq!(
            now[0].0, p.after,
            "{}: …and the ground they moved to is the one this press resolves the new pair \
             to — census {:?}",
            p.file,
            &now[..now.len().min(3)],
        );
        assert!(
            now[0].1 * 4 > plate.len() * 3,
            "{}: and it is most of the stencil that moved, not a corner of it ({} of {})",
            p.file,
            now[0].1,
            plate.len(),
        );

        // The control: it is the HOLES that follow the ground.
        let moved_ink =
            painted.iter().zip(&ink_was).filter(|(&(x, y), c)| after.get_pixel(x, y).0 != **c).count();
        assert_eq!(
            moved_ink, 0,
            "{}: {moved_ink} of {} painted plate pixels changed colour — a colour change must \
             move the page a picture stands on, never the picture",
            p.file,
            painted.len(),
        );
    }
    assert!(!any || seen > 0, "a Zork Zero volume is on disk but no press was booted");
}

/// **And the frame the app actually ships moves with it.** Hybrid is the default
/// v6 mode and the one the defect was reported in; it reaches the ground by a
/// different route (bands flooded with `fill_story_page_clear`, not one flattened
/// canvas), so it gets its own pin rather than being assumed to follow raster.
#[test]
fn the_hybrid_frame_resolves_the_new_ground_too() {
    let any = PRESSES.iter().any(|p| present(p.file));
    let mut seen = 0usize;
    for p in PRESSES {
        let Some(mut b) = boot(p.file, true) else { continue };
        seen += 1;
        let (was, was_n) = hybrid_art_ground(&mut b);
        assert!(
            was_n > 200,
            "{}: non-vacuity — the art region must be most of the pane outside the \
             viewport, got {was_n} cells on the commonest ground",
            p.file,
        );
        assert_eq!(
            was,
            ratatui::style::Color::Rgb(p.boot[0], p.boot[1], p.boot[2]),
            "{}: the hybrid art region boots on the same page raster does",
            p.file,
        );

        let _ = choose_a_pair(&mut b, p.file);

        let (now, now_n) = hybrid_art_ground(&mut b);
        eprintln!("{}: hybrid art ground {was:?} ({was_n}) -> {now:?} ({now_n})", p.file);
        assert_eq!(
            now,
            ratatui::style::Color::Rgb(p.after[0], p.after[1], p.after[2]),
            "{}: …and follows the player's new pair, exactly as the raster composite does",
            p.file,
        );
        assert!(now_n > 200, "{}: on a comparable share of the region ({now_n} cells)", p.file);
    }
    assert!(!any || seen > 0, "a Zork Zero volume is on disk but no press was booted");
}

/// **The other `honor_game_colours` mode**, per the project's colour convention —
/// and it is not merely the same test with a flag flipped, because the flag
/// changes what the COMMAND is.
///
/// Told the interpreter has no colours (§8.3.2), Zork Zero offers the only other
/// pair there is: one confirm, one "you should now get black text on a white
/// background", done. No menus, no numbers. And the ground does not move, which is
/// what declining a story's colours MEANS — the theme's page is the host's, not
/// the game's, so there is nothing for the holes to follow.
///
/// This is also the two-question form the quest's history describes, and it is
/// worth being precise about where it lives: it is not what a CGA press shows, it
/// is what a COLOURLESS interpreter shows. Both presses below take this shape.
#[test]
fn with_colours_declined_the_command_offers_only_a_swap_and_the_ground_stands_still() {
    let any = present(CGA_360) || present(EGA_360);
    let mut seen = 0usize;
    for p in PRESSES.iter().filter(|p| p.file == CGA_360 || p.file == EGA_360) {
        let Some(mut b) = boot(p.file, false) else { continue };
        seen += 1;
        assert!(!b.state.config.honor_game_colours, "{}: colours are off here", p.file);

        let holes = stencil(&mut b);
        let (_, plate) = holes.iter().max_by_key(|(_, h)| h.len()).expect("a graphics window");
        assert!(plate.len() > 100_000, "{}: non-vacuity — a mostly-stencil plate", p.file);
        let before = composite(&mut b);
        let was = census(&before, plate);

        let r = command(&mut b, "color");
        expect(p.file, &r, "Aesthetically, we recommend not changing the standard setting");
        expect(p.file, &r, "white text on a black background");
        expect(p.file, &r, "(Y or N)");
        assert!(
            !r.contains("--> WHITE"),
            "{}: a colourless interpreter is offered no menu of colours to pick from — {r:?}",
            p.file,
        );

        let r = say(&mut b, KeyInput::Char('y'));
        expect(p.file, &r, "You should now get black text on a white background.");
        expect(p.file, &r, "(Y or N)");
        let _ = say(&mut b, KeyInput::Char('y'));
        assert!(
            matches!(Engine::pending_input(&b.session), InputKind::Line),
            "{}: two questions and the command is over",
            p.file,
        );

        let after = composite(&mut b);
        let now = census(&after, plate);
        eprintln!("{} (colours declined): holes {:?} -> {:?}", p.file, &was[..1], &now[..1]);
        assert_eq!(
            now, was,
            "{}: with the story's colours declined the page is the theme's, so nothing the \
             game asks for can move the ground — or the holes standing on it",
            p.file,
        );
    }
    assert!(!any || seen > 0, "a Zork Zero volume is on disk but no press was booted");
}

/// The gitignored-media guard: on a machine that HAS the press, at least one case
/// above must have read it. Without this the whole file passes vacuously the day
/// a filename changes (SQ-0760 — an interpreter that relocates a story file turns
/// every real-game test into a silent skip).
#[test]
fn the_press_was_actually_read() {
    let mut seen = 0usize;
    for p in PRESSES {
        if !present(p.file) {
            continue;
        }
        let b = boot(p.file, true).expect("a volume that exists boots");
        assert_eq!(b.session.machine.palette(), p.palette, "{}: serves {}", p.file, p.archive);
        assert!(b.state.config.honor_game_colours, "{}: a DOS press has colours", p.file);
        seen += 1;
    }
    eprintln!("SQ-0957: {seen} of {} Zork Zero volumes present", PRESSES.len());
}
