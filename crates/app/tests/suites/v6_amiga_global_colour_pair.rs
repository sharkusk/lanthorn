//! ZMSD §8.3's Amiga rule, on the real release floppies — SQ-0740.
//!
//! §8.3 gives every Version 6 window its own foreground/background pair, and then
//! names one machine where that is wrong:
//!
//! > "Note that a Version 6 interpreter going under the Amiga interpreter number
//! > must use the same pair of colours for all windows when running Infocom's
//! > games. If either is changed, then the interpreter must change the colour of
//! > all text on the screen to match. This simulates the Amiga hardware, which
//! > used two logical colours for text and switched palette to change their
//! > physical colour."
//!
//! **And one gate the standard does not mention.** Infocom's own Amiga interpreter
//! (`amiga/yzip3.c`) changes text colours "only in window 0, and ignore[s] requests
//! in other windows (except for the special case of bg = -1)". lanthorn implements
//! that gate, because §8.3's stated purpose is to *simulate the Amiga hardware* and
//! a reading of it that diverges from that hardware defeats its own reason for
//! existing. The evidence is Journey: release 30 makes a single `set_colour(9, 2)`
//! — white ink, black page — on **window 3**, and real Amiga captures of the game
//! show grey with white text rather than black, i.e. Infocom's own
//! `DEF_BACK` / `DEF_FORE 9` defaults. The machine ignored the call.
//!
//! **The shade of that grey moved in SQ-0822, and the gate did not.** `DEF_BACK` is
//! 12 (dark grey `$444`), not the 11 (`$777`) `amiga/yzip.h` gives — the leaked
//! header is a development snapshot, and every interpreter on every Amiga release
//! floppy says 12 (`v6_amiga_shipped_interpreter.rs` reads it out of their 68000
//! code). Real Amiga captures agree: lemonamiga.com's Journey release-30 gallery
//! tallies 173,994 pixels of `#444444` under 25,878 of `#FFFFFF`. The evidence for
//! the window-0 gate was that Journey is *not black*, and a dark-grey page is not a
//! black one, so every case below still reads exactly as it did — with `12` where
//! it said `11`.
//!
//! The mechanism, and the retroactive repaint that is the hard half of it, are
//! unit-tested in `zvm::screen` where they live. What this suite adds is the part
//! only a real game can answer: that the OPCODE reaches the rule, that the gate
//! drops Journey's call, that a title which never calls `set_colour` is untouched,
//! that the IBM PC profile — the whole existing v6 corpus — does not move, and that
//! with `honor_game_colours` off the host theme still owns the screen.
//!
//! **…and that any of it reaches the SCREEN.** The first cut of this suite asserted
//! only on the screen MODEL, passed, shipped — and the user saw no change at all:
//! *"journey in amiga mode - i dont see any change from terminal colors."* The pen
//! was never lost. It arrived, and it was invisible, because Journey asks for
//! standard 9 (white) and the host theme's story ink is white too, while the pen's
//! page could not reach a window that never declared one. What was missing was the
//! machine's OWN pair: lanthorn advertised the Amiga's `$2C`/`$2D` defaults to the
//! story (§8.3.3) and then painted the host terminal's colours. So the cases below
//! assert on rendered CELLS, not on the model alone.
//!
//! **Media.** Every case drives an Amiga release floppy, named with its release
//! and serial, because a disk image is a different BUILD and not the same story on
//! other media: `Journey - The Quest Begins.adf` is release 30 / serial 890322,
//! not the release 83 of `journey-r83-s890706.z6`. `stories/` is gitignored, so a
//! missing fixture skips vacuously — loudly, on stderr.

use std::path::PathBuf;

use app::interpreter::InterpreterProfile;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use zvm::screen::ZColour;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// One Amiga release floppy, pinned to the build it carries. The table in
/// `real_media_releases.rs` is the authority; these rows repeat it so a failure
/// here names the release it measured, which is the single most expensive kind of
/// failure this project has had.
struct Floppy {
    file: &'static str,
    release: u16,
    serial: &'static str,
    turns: usize,
}

const JOURNEY: Floppy = Floppy {
    file: "Journey - The Quest Begins.adf",
    release: 30,
    serial: "890322",
    turns: 12,
};
const ZORK_ZERO: Floppy = Floppy {
    file: "Zork Zero - The Revenge of Megaboz.adf",
    release: 366,
    serial: "890323",
    turns: 12,
};
const ARTHUR: Floppy =
    Floppy { file: "Arthur - The Quest for Excalibur.adf", release: 54, serial: "890606", turns: 12 };

fn ctx(f: &Floppy, profile: InterpreterProfile, honor: bool) -> String {
    format!(
        "{} [release {}, serial {} — {profile:?} profile, honor_game_colours={honor}]",
        f.file, f.release, f.serial
    )
}

/// Boot `f` off its floppy under `profile` and drive it to a settled frame, or
/// `None` when the gitignored medium is absent.
fn boot(f: &Floppy, profile: InterpreterProfile, honor: bool) -> Option<GameSession> {
    let path = stories_dir().join(f.file);
    let bytes = match app::hints::load_mounted_story(&path) {
        Ok((loaded, _)) => loaded.bytes().to_vec(),
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    // The build in hand is the build the row names. Nothing measured after this
    // can be attributed to the wrong release.
    assert_eq!(bytes[0], 6, "{}: Z-machine version", ctx(f, profile, honor));
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        f.release,
        "{}: this medium carries a DIFFERENT build than the row says",
        ctx(f, profile, honor)
    );
    assert_eq!(
        String::from_utf8_lossy(&bytes[0x12..0x18]),
        f.serial,
        "{}: serial",
        ctx(f, profile, honor)
    );

    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    // `startup.rs`'s own chain, `native_std_window` included — this suite boots disk
    // media exclusively, and a press whose art is not 640x400 is otherwise told it has
    // a 640x400 screen and lays its own windows out to fit (CLAUDE.md, SQ-0901).
    let v6_screen_px =
        picts.std_window().or_else(|| picts.native_std_window()).or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        bytes,
        honor,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", ctx(f, profile, honor)));
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    s.machine.set_palette(profile.palette());
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    for _ in 0..f.turns {
        match s.pending_input() {
            InputKind::Line => {
                let _ = s.submit("");
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
        assert!(!s.quit, "{}: quit while driving", ctx(f, profile, honor));
        assert!(s.machine.fault_trace.is_none(), "{}: faulted while driving", ctx(f, profile, honor));
    }
    Some(s)
}

/// Every window's foreground, and every window's background, as the v6 screen
/// model holds them.
fn window_pairs(s: &GameSession) -> Vec<(ZColour, ZColour)> {
    s.machine.screen.v6.as_ref().expect("a v6 story").windows.iter().map(|w| (w.fg, w.bg)).collect()
}

/// Every distinct FOREGROUND carried by a glyph that is currently painted on the
/// screen — the grids, the pixel-positioned runs, the streamed prose and the
/// prose frozen behind a window that moved.
fn painted_foregrounds(s: &GameSession) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for w in &s.machine.screen.v6.as_ref().expect("a v6 story").windows {
        for c in w.grid.cells.iter().filter(|c| c.ch != ' ') {
            out.insert(format!("{:?}", c.fg));
        }
        for t in w.texts.iter().chain(w.streamed.iter()).chain(w.retired.iter()) {
            out.insert(format!("{:?}", t.fg));
        }
    }
    out
}

// ── (a) The rule itself, on the titles that exercise it ──────────────────────

/// Journey release 30 makes its one and only `set_colour(9, 2)` against window
/// **3**, and Infocom's Amiga interpreter ignores a colour set anywhere but window
/// 0. So the call lands nowhere: no window takes it, no glyph is repainted, and
/// the game is played on the machine's own default pair.
///
/// This assertion is the INVERSE of the one this suite shipped with, which read
/// the standard's "same pair for all windows" without Infocom's gate in front of
/// it and had window 0 adopting standard 9. That is not what the hardware did —
/// see the module docs for the walkthrough evidence — and it is why Journey came
/// out on a black page instead of the Amiga's light grey.
///
/// FALSIFY by deleting the `win != 0` gate in `ScreenState::set_amiga_colour_pair`:
/// every window adopts window 3's ink and this fails immediately.
#[test]
fn a_colour_set_outside_window_0_never_lands_on_an_amiga() {
    let Some(s) = boot(&JOURNEY, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::Amiga, true);

    for (i, (fg, bg)) in window_pairs(&s).iter().enumerate() {
        assert_eq!(*fg, ZColour::Default, "{who}: window {i} ink — the window-3 call is ignored");
        assert_eq!(*bg, ZColour::Default, "{who}: window {i} page — the window-3 call is ignored");
    }
    let inks = painted_foregrounds(&s);
    assert!(
        inks.iter().all(|c| c == "Default"),
        "{who}: no glyph may be repainted by a call the machine dropped, got {inks:?}",
    );

    // …and the case is only meaningful with a full frame on screen: window 1
    // carries Journey's whole line-drawing border as painted runs.
    let painted: usize = s
        .machine
        .screen
        .v6
        .as_ref()
        .unwrap()
        .windows
        .iter()
        .map(|w| w.texts.len() + w.streamed.len() + w.retired.len())
        .sum();
    assert!(painted > 100, "{who}: this case is only meaningful with a full frame drawn: {painted} runs");

    // What the screen IS, then: the machine's own pair, straight off header
    // $2D/$2C, which under this profile is Infocom's `DEF_FORE 9` over
    // `DEF_BACK 12` — white on dark grey (SQ-0822).
    assert_eq!(
        zvm::screen::amiga_screen_pair(&s.machine),
        Some((ZColour::Standard(9), ZColour::Standard(12))),
        "{who}: the pair the whole screen is painted with (the floppies' DEF_FORE/DEF_BACK)",
    );
}

/// A colour set from window **0** is the one the machine does take, and §8.3's
/// sharing rule then applies in full: Zork Zero release 366 boots
/// `set_colour(2, 10)` on its story window, and every other window's ink follows
/// the pen with it.
///
/// The page does not spread, and must not: the pens carry ink and page, but a page
/// nobody ever laid down is not a pixel a pen can reach. Filling one in anyway
/// paints the game's own artwork out of the frame.
///
/// FALSIFY by dropping the `repaint_amiga_pens` call: window 1 keeps `Default` ink
/// and the sharing rule is gone, while window 0 alone still shows the colour.
#[test]
fn a_colour_set_from_window_0_moves_the_pen_for_every_window() {
    let Some(s) = boot(&ZORK_ZERO, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&ZORK_ZERO, InterpreterProfile::Amiga, true);
    let pairs = window_pairs(&s);
    assert_eq!(
        pairs[0],
        (ZColour::Standard(2), ZColour::Standard(10)),
        "{who}: the story window keeps the black-on-light-grey scheme the game chose",
    );
    for (i, (fg, _)) in pairs.iter().enumerate() {
        assert_eq!(*fg, ZColour::Standard(2), "{who}: window {i} must share the foreground pen");
    }
    for i in [1usize, 2, 3] {
        assert_eq!(
            pairs[i].1,
            ZColour::Default,
            "{who}: window {i} was never given a background and must not be handed an opaque one",
        );
    }
}

/// Zork Zero release 366 prints its banner labels over the ribbon artwork with a
/// background of **-1** — "the colour of the pixel under the cursor" (§8.3.1).
/// That names no colour, so it loads no pen, and the labels stay transparent no
/// matter what the pens do. (Infocom's own Amiga interpreter carves the same case
/// out, in `amiga/yzip3.c`.)
#[test]
fn a_label_drawn_over_artwork_stays_over_the_artwork() {
    let Some(s) = boot(&ZORK_ZERO, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&ZORK_ZERO, InterpreterProfile::Amiga, true);
    let v6 = s.machine.screen.v6.as_ref().unwrap();
    let banner = &v6.windows[1];
    assert!(!banner.texts.is_empty(), "{who}: the banner window must have painted its labels");
    for t in &banner.texts {
        assert_eq!(
            t.bg,
            ZColour::Default,
            "{who}: banner label {:?} must keep the artwork showing through it",
            t.text,
        );
        assert_eq!(t.fg, ZColour::Standard(2), "{who}: …in the ink the game selected");
    }
    // The prose page, meanwhile, is a real background and keeps it.
    assert_eq!(
        (v6.windows[0].fg, v6.windows[0].bg),
        (ZColour::Standard(2), ZColour::Standard(10)),
        "{who}: the story window keeps the black-on-light-grey scheme the game chose",
    );
}

/// A title that never calls `set_colour` is not touched by a rule about calling
/// it. Arthur release 54 makes none, on either profile.
#[test]
fn a_title_that_never_sets_a_colour_is_untouched() {
    let Some(s) = boot(&ARTHUR, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&ARTHUR, InterpreterProfile::Amiga, true);
    for (i, (fg, bg)) in window_pairs(&s).iter().enumerate() {
        assert_eq!(*fg, ZColour::Default, "{who}: window {i} foreground");
        assert_eq!(*bg, ZColour::Default, "{who}: window {i} background");
    }
}

// ── (b) What must NOT move ───────────────────────────────────────────────────

/// The IBM PC profile is the whole existing v6 corpus, and §8.3's carve-out names
/// the Amiga alone. Under interpreter 6 every window keeps its own pair: Journey
/// colours window 3 and window 3 only.
///
/// FALSIFY by dropping the interpreter-number term from
/// `zvm::screen::amiga_global_colour_pair`: this fails immediately, with window 0
/// having quietly adopted window 3's ink.
#[test]
fn the_ibm_pc_profile_keeps_one_pair_per_window() {
    let Some(s) = boot(&JOURNEY, InterpreterProfile::IbmPc, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::IbmPc, true);
    let pairs = window_pairs(&s);
    assert_eq!(
        pairs[3],
        (ZColour::Standard(9), ZColour::Standard(2)),
        "{who}: the window the game coloured still takes the colour",
    );
    for i in [0usize, 1, 2, 4, 5, 6, 7] {
        assert_eq!(
            pairs[i],
            (ZColour::Default, ZColour::Default),
            "{who}: window {i} must be untouched by a colour set on window 3",
        );
    }
    // And the text on it is drawn in the interpreter's default ink, exactly as
    // before — no pen ever reached it.
    assert!(
        painted_foregrounds(&s).iter().all(|c| c == "Default"),
        "{who}: no glyph may adopt another window's colour",
    );
}

/// `honor_game_colours = false` declares the interpreter colourless to the story
/// (§8.3.2 — Flags 1 bit 0 is cleared), which is the user saying the host theme
/// owns the screen. The Amiga rule must not reach past that switch, on either
/// profile: a shared pen is still a game colour.
#[test]
fn with_game_colours_off_the_theme_owns_the_screen_on_both_profiles() {
    for profile in [InterpreterProfile::IbmPc, InterpreterProfile::Amiga] {
        let Some(s) = boot(&JOURNEY, profile, false) else { return };
        let who = ctx(&JOURNEY, profile, false);
        let pairs = window_pairs(&s);
        for i in [0usize, 1, 2, 4, 5, 6, 7] {
            assert_eq!(
                pairs[i].0,
                ZColour::Default,
                "{who}: window {i} must keep the theme's ink when colours are off",
            );
        }
        assert!(
            painted_foregrounds(&s).iter().all(|c| c == "Default"),
            "{who}: no glyph may be repainted by a rule the player switched off",
        );
    }
}

// ── (c) …and on the SCREEN ───────────────────────────────────────────────────

/// A hybrid render at real kitty-ish cell metrics (8×18) — the shipped default
/// mode, and the one the report was filed against. `Picker::halfblocks()` reports
/// a 1×2 cell, a layout regime that reproduces nothing, so the harness has to run
/// at a plausible font cell (the SQ-0548 lesson).
#[allow(deprecated)]
fn render_hybrid(s: &GameSession, honor: bool, cols: u16, rows: u16) -> (Rect, Buffer) {
    use app::engine::Engine;
    let model = s.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default_in(s.machine.palette());
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    (area, buf)
}

/// The characters Journey draws its frame, rules and menu dividers with under the
/// Amiga profile. (Under IBM PC the same rules are reverse-video spaces — see
/// `v6_journey_amiga_frame.rs` — which is why this set is Amiga-specific.)
const FRAME_GLYPHS: [char; 6] = ['┌', '─', '┐', '│', '└', '┘'];

/// Every distinct `(fg, bg)` a frame glyph is drawn in, and the tally of every
/// cell background in the pane.
type CellTally = (
    std::collections::BTreeMap<(String, String), usize>,
    std::collections::BTreeMap<String, usize>,
);
fn tally(area: Rect, buf: &Buffer) -> CellTally {
    let mut frame: std::collections::BTreeMap<(String, String), usize> = Default::default();
    let mut pages: std::collections::BTreeMap<String, usize> = Default::default();
    for y in 0..area.height {
        for x in 0..area.width {
            let c = buf.cell((x, y)).expect("in-bounds cell");
            *pages.entry(format!("{:?}", c.bg)).or_default() += 1;
            if FRAME_GLYPHS.contains(&c.symbol().chars().next().unwrap_or(' ')) {
                *frame.entry((format!("{:?}", c.fg), format!("{:?}", c.bg))).or_default() += 1;
            }
        }
    }
    (frame, pages)
}

/// The Amiga's own default pair as the renderer must resolve it: `DEF_FORE 9`
/// (white) over `DEF_BACK 12` (dark grey `$444`), through the ACTIVE palette and
/// the profile's own constants rather than a hardcoded RGB, so a palette change
/// moves the expectation with it.
///
/// The page comes out `Rgb(66, 66, 66)` and not the hardware's exact `#444444`
/// because a 4-bit Amiga channel is widened to the Z-machine's 5 bits and then to
/// 8 — `4/15` becomes `8/31` becomes `66/255`. Two units, and the 15-bit word is
/// the Z-machine's own currency, so this is the faithful number rather than a
/// rounding to paper over.
fn amiga_pair_rgb() -> (String, String) {
    let amiga = zvm::screen::Palette::Amiga;
    let (r, g, b) = app::colors::standard_colour_rgb(amiga, app::interpreter::AMIGA_DEFAULT_FOREGROUND)
        .expect("standard 9 is white");
    let (gr, gg, gb) = zvm::screen::grey_rgb(amiga, app::interpreter::AMIGA_DEFAULT_BACKGROUND);
    (format!("Rgb({r}, {g}, {b})"), format!("Rgb({gr}, {gg}, {gb})"))
}

/// **The deliverable.** Journey on its Amiga floppy renders white on the machine's
/// own dark grey —
/// not "in the model", on the cells the terminal is handed.
///
/// This is the case the first cut of SQ-0740 did not have. It shipped on a model
/// assertion, and the report back was *"journey in amiga mode - i dont see any
/// change from terminal colors"*: the pen reached the cells and was invisible
/// there, because the theme already drew white, while the Amiga's own PAGE — the
/// half a player would actually notice — was advertised to the story in header $2C
/// and never painted by anything.
///
/// FALSIFY by making `session::v6_screen_model` publish `Default` for the v6 page
/// again (drop the `amiga_screen_pair` call): every frame glyph comes back as the
/// theme's `White` on `Black`, the pane page goes back to `Reset`, and the Amiga
/// profile is once more indistinguishable from the IBM PC one on screen — which is
/// the user's symptom, verbatim.
#[test]
fn journey_renders_white_on_the_machines_dark_grey_on_the_amiga_floppy() {
    let Some(s) = boot(&JOURNEY, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::Amiga, true);
    let (white, grey) = amiga_pair_rgb();

    // Swept across pane sizes: a colour that only lands where the numbers happen to
    // coincide is the bug in another costume (the SQ-0548/SQ-0742 lesson).
    for (cols, rows) in [(115u16, 61u16), (96, 51), (150, 71)] {
        let (area, buf) = render_hybrid(&s, true, cols, rows);
        let (frame, pages) = tally(area, &buf);
        let at = format!("{who} @ {cols}x{rows}");

        // The frame, the rules and the menu dividers: one pair, and it is the
        // machine's. Journey names no colours at all now that its window-3 call is
        // gated away, so every one of these glyphs is an INHERITED channel — which
        // is exactly the channel that used to resolve to the host terminal.
        assert!(!frame.is_empty(), "{at}: no frame glyphs drawn — this case measures nothing");
        assert_eq!(
            frame.keys().cloned().collect::<Vec<_>>(),
            vec![(white.clone(), grey.clone())],
            "{at}: every frame glyph must be the Amiga's white ink on its dark-grey page, got {frame:?}",
        );

        // …and the page runs to the pane, not merely under the glyphs.
        let cells = area.width as usize * area.height as usize;
        let on_page = pages.get(&grey).copied().unwrap_or(0);
        assert!(
            on_page * 2 > cells,
            "{at}: the machine's page must cover the pane — {on_page} of {cells} cells carry it: {pages:?}",
        );
    }
}

/// The IBM PC profile is the whole existing v6 corpus, and none of it moves: the
/// same frame, rendered under interpreter 6, is drawn in the host THEME's colours
/// exactly as it was before this quest existed — and never in the Amiga's grey.
#[test]
fn the_ibm_pc_profile_renders_in_the_host_theme_exactly_as_before() {
    let Some(s) = boot(&JOURNEY, InterpreterProfile::IbmPc, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::IbmPc, true);
    let (_, grey) = amiga_pair_rgb();
    let (area, buf) = render_hybrid(&s, true, 115, 61);
    let (frame, pages) = tally(area, &buf);
    assert!(!frame.is_empty(), "{who}: no frame glyphs drawn — this case measures nothing");
    assert_eq!(
        frame.keys().cloned().collect::<Vec<_>>(),
        vec![("White".to_string(), "Black".to_string())],
        "{who}: the theme's own named colours, resolved to no RGB at all, got {frame:?}",
    );
    assert_eq!(
        pages.get(&grey).copied().unwrap_or(0),
        0,
        "{who}: no cell may take an Amiga page on a machine that is not an Amiga",
    );
}

/// `honor_game_colours = false` declares the interpreter colourless to the story
/// (§8.3.2 — Flags 1 bit 0 cleared), which is the player saying the host theme owns
/// the screen. A pair the INTERPRETER paints with is still a game colour, so the
/// Amiga's page must not reach past that switch either — and with the flag off
/// `amiga_global_colour_pair` is false at the source, so the machine publishes no
/// pair at all.
#[test]
fn with_game_colours_off_the_amiga_page_never_reaches_the_cells() {
    let Some(s) = boot(&JOURNEY, InterpreterProfile::Amiga, false) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::Amiga, false);
    let (_, grey) = amiga_pair_rgb();
    assert_eq!(
        zvm::screen::amiga_screen_pair(&s.machine),
        None,
        "{who}: a colourless interpreter has no pair to paint with",
    );
    // Both render-side settings, because the model keeps what it recorded while the
    // flag was as it was: a mid-game `/set-game-colours` must be honoured by the
    // RENDER, not merely by the boot-time header.
    for honor in [false, true] {
        let (area, buf) = render_hybrid(&s, honor, 115, 61);
        let (_, pages) = tally(area, &buf);
        assert_eq!(
            pages.get(&grey).copied().unwrap_or(0),
            0,
            "{who} (rendered with honor={honor}): the theme owns the screen — no Amiga page anywhere",
        );
    }
}

// ── (d) Arthur's page, and the notices printed on it (SQ-0822) ───────────────

/// `boot` without its blank-turn tail, for a case that drives its own script.
fn boot_unscripted(f: &Floppy, profile: InterpreterProfile, honor: bool) -> Option<GameSession> {
    let no_turns = Floppy { turns: 0, ..*f };
    boot(&no_turns, profile, honor)
}

/// Drive Arthur release 54 off its floppy to the church and pray, which earns the
/// notice the report was filed against: `[You have earned ten chivalry points.]`.
/// Returns the session and the host transcript, or `None` when the medium is absent.
///
/// The scripted walk matters. `boot` above taps twelve blank turns, which never
/// gets past Arthur's *"Would you like to restore a saved position? Please press Y
/// or N>"* — and a session parked on that prompt is why the SQ-0740 lane recorded
/// that "Arthur … set[s] no colours". It does: a dozen `set_colour(0, 0, window=3)`
/// calls in the first turns of play, each preceded by
/// `get_wind_prop(win=3, prop=11)` returning 0. Both channels are the opcode's
/// "leave this alone" sentinel, so the call moves nothing on any machine — the
/// read-the-colours-back-and-restore-them idiom — and it is NOT where the page
/// comes from. The page is the machine's own, which is the whole of this.
fn arthur_at_the_church(honor: bool) -> Option<(GameSession, Vec<String>)> {
    let profile = InterpreterProfile::Amiga;
    let mut s = boot_unscripted(&ARTHUR, profile, honor)?;
    let who = ctx(&ARTHUR, profile, honor);
    let mut lines: Vec<String> = Vec::new();
    let script = ["east", "enter church", "pray"];
    let mut next = 0usize;
    for _ in 0..40 {
        let r = match s.pending_input() {
            InputKind::Line => {
                let cmd = script.get(next).copied().unwrap_or("");
                next += 1;
                s.submit(cmd)
            }
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        lines.extend(r.transcript.split('\n').map(str::to_owned));
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
        assert!(!s.quit, "{who}: quit while driving");
        assert!(s.machine.fault_trace.is_none(), "{who}: faulted while driving");
        if next > script.len() {
            break;
        }
    }
    assert!(
        lines.iter().any(|l| l.contains("one-room building")),
        "{who}: the walk must reach the church — this case measures nothing otherwise",
    );
    assert!(
        lines.iter().any(|l| l.trim() == "[You have earned ten chivalry points.]"),
        "{who}: praying must print the bracketed notice the report is about",
    );
    Some((s, lines))
}

/// Render the hybrid frame WITH the host transcript in it — the reading surface the
/// player's eye is on, which `render_hybrid` above (an empty transcript) never draws.
#[allow(deprecated)]
fn render_with_transcript(s: &GameSession, lines: &[String], honor: bool) -> (Rect, Buffer) {
    use app::engine::Engine;
    let model = s.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default_in(s.machine.palette());
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    for line in lines {
        state.push_transcript_kind(line, app::state::TranscriptKind::Story);
    }
    let area = Rect::new(0, 0, 115, 45);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    (area, buf)
}

/// Every distinct `(fg, bg)` carried by a non-blank cell of the row containing
/// `needle` — `None` when no row contains it.
fn row_styles(area: Rect, buf: &Buffer, needle: &str) -> Option<Vec<(String, String)>> {
    let text = |y: u16| -> String {
        (0..area.width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect()
    };
    let y = (0..area.height).find(|&y| text(y).contains(needle))?;
    let mut out: std::collections::BTreeSet<(String, String)> = Default::default();
    for x in 0..area.width {
        let c = buf.cell((x, y)).expect("in-bounds cell");
        if c.symbol() != " " {
            out.insert((format!("{:?}", c.fg), format!("{:?}", c.bg)));
        }
    }
    Some(out.into_iter().collect())
}

/// **The report, both halves.** Arthur release 54 in the church, on its Amiga
/// floppy: the page is the machine's dark grey, and the bracketed notice printed on
/// it is the same white as the prose around it.
///
/// The reference is MobyGames' capture of this exact scene on a real Amiga, which
/// measures `#444444` page under `#FFFFFF` ink across the whole text panel and
/// nothing else — with the status bar reversed to `#444444` on `#FFFFFF`, i.e. the
/// same two pens swapped, which is what proves the page is the text background
/// REGISTER and not artwork. The two things that were wrong:
///
/// 1. the page was standard 11 (`$777`, `Rgb(115, 115, 115)`) because
///    `AMIGA_DEFAULT_BACKGROUND` came from a development header rather than from
///    the machine — *"the page is lighter than the real Amiga's"*;
/// 2. the notice came out `DarkGray` on that page, because lanthorn's built-in
///    "a whole line in brackets is a message from the interpreter" rule mutes it —
///    *"nearly invisible"*, where the walkthrough screenshots show it white.
///
/// FALSIFY either half independently: restore `AMIGA_DEFAULT_BACKGROUND = 11` and
/// both rows fail on the page; drop the `machine_owns_ink` term from
/// `ColorScheme::resolve_story_style` and only the notice row fails, on its ink.
#[test]
fn arthurs_notices_are_the_machines_white_on_the_machines_dark_grey() {
    let Some((s, lines)) = arthur_at_the_church(true) else { return };
    let who = ctx(&ARTHUR, InterpreterProfile::Amiga, true);
    let (white, grey) = amiga_pair_rgb();
    let (area, buf) = render_with_transcript(&s, &lines, true);

    // The machine publishes its pair at all, straight off $2D/$2C.
    assert_eq!(
        zvm::screen::amiga_screen_pair(&s.machine),
        Some((ZColour::Standard(9), ZColour::Standard(12))),
        "{who}: DEF_FORE 9 over DEF_BACK 12",
    );
    for needle in ["one-room building", "[You have earned ten chivalry points.]"] {
        let styles = row_styles(area, &buf, needle)
            .unwrap_or_else(|| panic!("{who}: {needle:?} must be on screen"));
        assert_eq!(
            styles,
            vec![(white.clone(), grey.clone())],
            "{who}: {needle:?} must be the machine's ink on the machine's page",
        );
    }

    // …and an independent oracle for the half a player reported by eye, so this
    // case cannot pass merely by agreeing with the constant it was derived from:
    // the REFERENCE pixels. `#444444` page, `#FFFFFF` ink, straight off the Amiga
    // capture, within the two units the 4→5→8-bit widening costs (see
    // `amiga_pair_rgb`). A page of standard 11 lands on 115 and misses by 47.
    let (pr, pg, pb) =
        zvm::screen::grey_rgb(s.machine.palette(), app::interpreter::AMIGA_DEFAULT_BACKGROUND);
    for (got, want, ch) in [(pr, 0x44u8, 'r'), (pg, 0x44, 'g'), (pb, 0x44, 'b')] {
        assert!(
            got.abs_diff(want) <= 2,
            "{who}: the page's {ch} channel is {got}, and the real Amiga's is {want}",
        );
    }
    let (ir, ig, ib) =
        app::colors::standard_colour_rgb(s.machine.palette(), app::interpreter::AMIGA_DEFAULT_FOREGROUND)
            .expect("standard 9 is white");
    assert_eq!((ir, ig, ib), (0xFF, 0xFF, 0xFF), "{who}: the ink is the capture's white");
}

/// …and with `honor_game_colours` off the player has said the theme owns the
/// screen, so the machine publishes no pair, the notice goes back to the built-in
/// system style, and nothing on that row carries an Amiga page. A pair the
/// INTERPRETER paints with is still a game colour.
#[test]
fn with_game_colours_off_arthurs_notice_keeps_the_themes_own_system_style() {
    let Some((s, lines)) = arthur_at_the_church(false) else { return };
    let who = ctx(&ARTHUR, InterpreterProfile::Amiga, false);
    let (_, grey) = amiga_pair_rgb();
    let (area, buf) = render_with_transcript(&s, &lines, false);

    assert_eq!(
        zvm::screen::amiga_screen_pair(&s.machine),
        None,
        "{who}: a colourless interpreter has no pair to paint with",
    );
    let sys = format!(
        "{:?}",
        app::colors::ColorScheme::terminal_default_in(s.machine.palette())
            .theme
            .get("transcript_system")
            .style
            .fg
            .expect("the system style names an ink")
    );
    let styles = row_styles(area, &buf, "[You have earned ten chivalry points.]")
        .unwrap_or_else(|| panic!("{who}: the notice must be on screen"));
    assert!(
        styles.iter().all(|(fg, _)| *fg == sys),
        "{who}: off the Amiga the bracketed line keeps the theme's system style, got {styles:?}",
    );
    assert!(
        styles.iter().all(|(_, bg)| *bg != grey),
        "{who}: no Amiga page may reach the cells with colours declined, got {styles:?}",
    );
}

// ── (d) The input echo stands on the same ground (SQ-0847) ───────────────────

/// `render_with_transcript`, with a command either being TYPED at the live prompt
/// or already COMMITTED onto the game's own `>` line — which is what `turn.rs`
/// does with the echo in inline mode, the shipped default.
#[allow(deprecated)]
fn render_echo(s: &GameSession, lines: &[String], honor: bool, typed: &str, committed: bool) -> (Rect, Buffer) {
    use app::engine::Engine;
    let model = s.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default_in(s.machine.palette());
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    for line in lines {
        state.push_transcript_kind(line, app::state::TranscriptKind::Story);
    }
    state.push_transcript_kind(">", app::state::TranscriptKind::Story);
    if committed {
        state.append_to_last_transcript_line(typed);
    } else {
        state.input.set(typed, true);
    }
    let area = Rect::new(0, 0, 115, 45);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    (area, buf)
}

/// Every cell of `needle` where it was drawn, as `(fg, bg, modifier)` strings so a
/// failure prints what it measured.
fn span_look(area: Rect, buf: &Buffer, needle: &str) -> Vec<(String, String, String)> {
    for y in 0..area.height {
        let text: String = (0..area.width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect();
        if let Some(byte_at) = text.find(needle) {
            let x0 = text[..byte_at].chars().count() as u16;
            let x1 = (x0 + needle.chars().count() as u16).min(area.width - 1);
            return (x0..=x1)
                .map(|x| {
                    let c = buf.cell((x, y)).expect("in-bounds cell");
                    (format!("{:?}", c.fg), format!("{:?}", c.bg), format!("{:?}", c.modifier))
                })
                .collect();
        }
    }
    panic!("{needle:?} was never drawn into the pane");
}

/// **The Amiga half of SQ-0847's non-regression.** The Macintosh is where a player
/// saw it — white ink on a white page — but the defect was never about the
/// Macintosh: the typed line resolved `input_text` over the transcript style, and
/// that selector's ink is the `text` role's, so it overwrote whatever machine pair
/// lay beneath. On the Amiga the same overwrite happened and merely *looked*
/// right, because the role's ink is white and Infocom's `DEF_FORE` is white too —
/// the theme's `White` and the machine's `Rgb(255, 255, 255)`, two different
/// values agreeing by coincidence, which is exactly how a defect survives.
///
/// Arthur release 54 is the case: it never sets a colour anywhere, so its whole
/// screen is the machine's `DEF_FORE 9` over `DEF_BACK 12`, and the two echo paths
/// must land on that pair identically.
///
/// FALSIFY by reverting the machine-ground fallback in
/// `render::screen::game_input_style`: the typed span comes back `White` on the
/// machine's grey while the committed span stays `Rgb(255, 255, 255)`.
#[test]
fn the_amigas_typed_echo_stands_on_the_same_pair_as_its_committed_one() {
    let Some((s, lines)) = arthur_at_the_church(true) else { return };
    let who = ctx(&ARTHUR, InterpreterProfile::Amiga, true);
    let (white, grey) = amiga_pair_rgb();

    let (area, live) = render_echo(&s, &lines, true, "look", false);
    let (_, done) = render_echo(&s, &lines, true, "look", true);
    let live_span = span_look(area, &live, ">look");
    assert_eq!(
        live_span,
        span_look(area, &done, ">look"),
        "{who}: the echo and the committed text must render the same characters the same way",
    );
    for cell in &live_span[1..=4] {
        assert_eq!(
            (cell.0.clone(), cell.1.clone()),
            (white.clone(), grey.clone()),
            "{who}: a typed character must be the machine's ink on the machine's page, got {live_span:?}",
        );
    }

    // …and the switch that declines game colours declines this too: no Amiga page
    // reaches the typed line, and it keeps the theme's own input ink.
    let (area, off) = render_echo(&s, &lines, false, "look", false);
    let themed = span_look(area, &off, ">look");
    let theme_ink = format!(
        "{:?}",
        app::colors::ColorScheme::terminal_default_in(s.machine.palette())
            .theme
            .get("input_text")
            .style
            .fg
            .expect("the theme names an input ink")
    );
    for cell in &themed[1..=4] {
        assert_eq!(cell.0, theme_ink, "{who} (colours declined): the theme owns the input line, got {themed:?}");
        assert_ne!(cell.1, grey, "{who} (colours declined): no Amiga page may reach the typed line");
    }
}

/// …and a game that ASKED for a pair still gets the one it asked for. Zork Zero
/// release 366 sets `set_colour(2, 10)` from window 0, so the story window
/// declares a pair of its own and the machine's default ground is never consulted
/// — the SQ-0532 wave-6 path, unmoved by SQ-0847.
#[test]
fn a_game_that_named_its_own_pair_still_types_in_that_pair() {
    let Some(s) = boot(&ZORK_ZERO, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&ZORK_ZERO, InterpreterProfile::Amiga, true);
    let pairs = window_pairs(&s);
    assert_eq!(
        pairs[0],
        (ZColour::Standard(2), ZColour::Standard(10)),
        "{who}: premise — the story window named its own pair",
    );
    // Standard 2 and standard 10, resolved the way the prose resolves them —
    // through the SESSION's own table (SQ-1393), so the greys come through
    // `zvm::screen::grey_rgb` rather than off a default palette table.
    let (br, bg_, bb) =
        app::colors::standard_colour_rgb(s.machine.palette(), 2).expect("standard 2 is black");
    let (gr, gg, gb) = zvm::screen::grey_rgb(s.machine.palette(), 10);
    let (black, light_grey) = (format!("Rgb({br}, {bg_}, {bb})"), format!("Rgb({gr}, {gg}, {gb})"));

    let (area, live) = render_echo(&s, &["Nothing happens.".to_string()], true, "look", false);
    let span = span_look(area, &live, ">look");
    for cell in &span[1..=4] {
        assert_eq!(
            (cell.0.clone(), cell.1.clone()),
            (black.clone(), light_grey.clone()),
            "{who}: the typed line must keep the pair the GAME set, got {span:?}",
        );
    }
}

// ── The page an inherited channel resolves to (SQ-0906) ─────────────────────

/// Chrome that names no background sits on the page the GAME dressed the screen
/// with, not on the theme's.
///
/// Zork Zero's DEFINE menu is the frame. Its story window is black on
/// `Standard(10)`, light grey; all 526 of the menu's single-character runs name a
/// foreground of black and **no background at all**; and the menu window carries an
/// `ErasedFill`, whose `bg = 0` means "the page default". Every one of those
/// inherited the THEME's black, so the menu rendered black on black.
///
/// Three things this case has to get right, each of which cost an attempt:
///
/// * **A picker.** `AppState::default()` has none, so `render_story_pane` logs
///   `"cell — no image protocol"` and never enters the v6 arm at all. A colour case
///   without one measures the fallback and passes against any defect. Halfblocks, so
///   the bands land in the pane's own cells and their grounds are readable here.
/// * **The right path.** This frame does NOT reach the hybrid ring — it is routed by
///   `has_menu && hybrid && !menu_over_art` to the cell path, which has its own base
///   style. A fix aimed at the ring changes nothing here, and `v6_story_page` is
///   only published on the ring, so it is the wrong oracle. Asserted on the pane's
///   own CELLS instead.
/// * **The drive, by pending input.** `boot` leaves the game wherever twelve blank
///   turns land it, and `define` is only accepted at a line read.
///
/// Arthur and Journey ride along as the control: on this same profile they dress no
/// story background, so nothing about them may move.
#[test]
fn chrome_inherits_the_page_the_game_dressed() {
    let mut ran = 0;
    for (f, to_menu) in [(&ZORK_ZERO, true), (&ARTHUR, false), (&JOURNEY, false)] {
        let Some(mut s) = boot(f, InterpreterProfile::Amiga, true) else { continue };
        let label = ctx(f, InterpreterProfile::Amiga, true);
        if to_menu {
            for _ in 0..8 {
                if matches!(s.pending_input(), InputKind::Line) {
                    break;
                }
                let _ = s.submit_char(13);
            }
            let said = s.submit("define").transcript;
            assert!(
                said.to_lowercase().contains("key to define"),
                "{label}: premise — `define` did not open the key-definition menu: {said:?}",
            );
            let _ = s.submit_char(b' ');
        }
        let model = app::engine::Engine::screen(&s);
        let app::engine::WinNode::Layered(items) = &model.root else { panic!("{label}: Layered") };
        let dressed = app::render::v6_layout::story_bg_rgba(
            app::render::v6_layout::classify_windows(items.as_slice(), zvm::screen::V6Cell::DEFAULT).story,
            &app::colors::ColorScheme::terminal_default_in(s.machine.palette()),
        );

        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default_in(s.machine.palette());
        state.config.honor_game_colours = true;
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        let area = Rect::new(0, 0, 98, 37);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

        // Every ground an INKED cell sits on.
        let grounds: std::collections::BTreeSet<String> = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| !buf[(x, y)].symbol().trim().is_empty())
            .map(|(x, y)| format!("{:?}", buf[(x, y)].bg))
            .collect();
        let theme_ground = format!("{:?}", app::colors::ColorScheme::terminal_default_in(s.machine.palette()).theme.get("upper_window").style.bg);

        match dressed {
            Some(p) => {
                let page = format!("{:?}", ratatui::style::Color::Rgb(p[0], p[1], p[2]));
                assert!(
                    grounds.contains(&page),
                    "{label}: the game dressed this screen in {page} and NO inked cell sits on \
                     it. Grounds seen: {grounds:?}",
                );
                assert!(
                    !grounds.contains(&theme_ground),
                    "{label}: inked cells sit on the theme's ground {theme_ground} while the game \
                     dressed the screen in {page} — that is the DEFINE menu coming back black on \
                     black. Grounds seen: {grounds:?}",
                );
            }
            // The control, and it is a statement about the FIXTURE rather than about
            // the pane: these titles dress no page of their own on this profile, which
            // is exactly why `status_style` cannot move for them. Asserted rather than
            // assumed, because the day one of them starts dressing a page is the day
            // this case has to be re-read. (Their panes are not a useful oracle here —
            // Arthur's frame is all image bands and has no inked cell at all.)
            None => assert!(
                app::render::v6_layout::story_bg_rgba(
                    app::render::v6_layout::classify_windows(items.as_slice(), zvm::screen::V6Cell::DEFAULT).story,
                    &state.colors,
                )
                .is_none(),
                "{label}: this control now dresses a page of its own, so it is no longer a \
                 control — re-read what the inherited ground should be for it",
            ),
        }
        ran += 1;
    }
    if stories_dir().join(ZORK_ZERO.file).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran");
    }
}

