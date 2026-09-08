//! SQ-1071 — a v6 hint clue **wraps inside its own window** on the two machines
//! Infocom shipped a Version 6 interpreter for, however the window's wrapping
//! attribute reads.
//!
//! ## What was wrong
//!
//! Shogun's InvisiClues clears window 0's wrapping attribute
//! (`@window_style(win=0, flags=0b0001, op=2)`), sizes the window to 500x330 at
//! native (71,71), and prints a clue longer than that. lanthorn read the
//! attribute on every machine and then clipped at the SCREEN edge, so the clue
//! ran out to native x=639 — across the frame art, out of its own window — and
//! was cut mid-word at `bef`.
//!
//! ZMSD §8.8.3.1.2.2's commentary tabulates what Infocom's own interpreters did,
//! and the Macintosh and Amiga columns read `---` on every attribute row: *"here
//! `---` means that the interpreter **ignores** the given state"*. Both follow the
//! `buffer_mode` opcode, which defaults on, and therefore **word wrap**. See
//! `zvm::interpreter::V6WrapRegime`.
//!
//! ## The falsifiers
//!
//! `machine-screenshots/amiga-shogun-hintshown.png` and
//! `machine-screenshots/mac-shogun-hintshown.png` — the same clue, the same turn,
//! on each machine:
//!
//! ```text
//! Amiga      6> You have to do everything you can to keep your ship from
//!            sinking before you get to Japan.
//!            5>
//!
//! Macintosh  6> You have to do everything you can to keep your ship from sinking before you
//!            get to Japan.
//!            5>
//! ```
//!
//! Both break at a WORD and both continue at the window's own left margin, and
//! each case here pins its own machine's break: the Amiga steps a fixed 8-px pen,
//! the Macintosh draws proportional Geneva 12 and fits eleven more characters on
//! the line. Getting the Macintosh's right needed SQ-1072 as well — see the note
//! on `mac_clue_wraps_where_the_machine_wrapped`.
//!
//! ## Specimen table
//!
//! | fixture | machine | release / serial | route | turns |
//! |---|---|---|---|---|
//! | `James Clavell's Shogun.adf` | Amiga | **295 / 890321** | 14 taps, `hint`, `y`, Return x4 | 19 |
//! | `Shogun.toast` | Macintosh | **292 / 890314** | 14 taps, `hint`, `y`, Return x4 | 19 |
//!
//! `stories/` is gitignored, so both cases skip vacuously without the medium.
//! The Macintosh press draws with Geneva off a System file the player supplies
//! (SQ-1036), so its PEN is machine-dependent — that case therefore asserts
//! nothing that depends on a particular advance table.

use app::engine::Engine;
use app::session::{GameSession, InputKind};
use std::path::PathBuf;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot the way `startup.rs` boots — the profile from the medium the MOUNT
/// returned, the screen through the four-link cascade, the machine's own cell and
/// face (CLAUDE.md). `MachineBoot::resolve` is one call so this harness cannot
/// omit a link, which is what SQ-1021/SQ-1022 were about.
fn boot(fixture: &str, release: u16, serial: &str) -> Option<GameSession> {
    let path = stories_dir().join(fixture);
    let Ok((loaded, medium)) = app::hints::load_mounted_story(&path) else {
        eprintln!("SKIP: gitignored medium missing at {}", path.display());
        return None;
    };
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release, "{fixture}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{fixture}: serial");
    let (profile, source) =
        app::interpreter::InterpreterProfile::resolve_with_source(&path, None, None, medium);
    let mut picts = app::graphics::PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        app::native_font::resolve(&app::native_font::FaceRequest {
            story_path: &path,
            entry: None,
            profile,
            source,
            art_scale: picts.art_scale(),
            disks: Some(&app::system_fonts::UserDisks::new("")),
        }),
        zvm::screen::Palette::Standard,
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &boot)
        .unwrap_or_else(|e| panic!("{fixture}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// 19 turns: 14 intro taps, `hint`, `y`, then Return four times — select the
/// already-highlighted first topic (*Erasmus*), page to the clue prompt, reveal
/// hint 6. The captures are of exactly this frame.
fn clue_frame(fixture: &str, release: u16, serial: &str) -> Option<GameSession> {
    let mut s = boot(fixture, release, serial)?;
    for _ in 0..14 {
        match s.pending_input() {
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            _ => {
                s.submit("");
            }
        }
        assert!(!s.quit, "{fixture}: quit during the intro");
    }
    let r = s.submit("hint");
    assert!(r.fault.is_none(), "{fixture}: `hint` faulted: {:?}", r.fault);
    let r = s.submit_char(b'y');
    assert!(r.fault.is_none(), "{fixture}: entering the menu faulted: {:?}", r.fault);
    for _ in 0..4 {
        let r = s.submit_char(13);
        assert!(r.fault.is_none(), "{fixture}: paging the clue faulted: {:?}", r.fault);
    }
    Some(s)
}

/// Window 0's painted runs as `(y, x, text)`, in paint order.
fn win0_runs(s: &GameSession) -> Vec<(u16, u16, String)> {
    s.machine.screen.v6.as_ref().expect("v6")
        .windows[0]
        .texts
        .iter()
        .map(|t| (t.y, t.x, t.text.clone()))
        .collect()
}

/// The frame's own signature, asserted before anything is measured on it: a
/// release that stops clearing the wrapping bit, or a route that stops reaching
/// the clue, must FAIL here rather than pass vacuously somewhere below.
fn guard_shape(s: &GameSession, tag: &str) {
    let w0 = &s.machine.screen.v6.as_ref().expect("v6").windows[0];
    assert_eq!(
        w0.attributes & 0b0001,
        0,
        "{tag}: this frame is only interesting while the game has CLEARED window 0's \
         wrapping attribute — attrs {:04b}",
        w0.attributes & 0b1111,
    );
    assert_eq!(
        (w0.x_size, w0.y_size, w0.x_coord, w0.y_coord),
        (500, 330, 71, 71),
        "{tag}: the game's own clue window, as both captures show it",
    );
}

/// The clue that used to be cut mid-word, whole and on two lines — asserted
/// against `machine-screenshots/amiga-shogun-hintshown.png` glyph for glyph.
#[test]
fn amiga_clue_wraps_where_the_machine_wrapped() {
    let Some(s) = clue_frame("James Clavell's Shogun.adf", 295, "890321") else { return };
    guard_shape(&s, "amiga");
    let runs = win0_runs(&s);

    // The capture, read off the frame: the label, the marker, the clue's first
    // line, its continuation at the window's own left margin, then hint 5's label
    // one line further down. The window is at native (71,71) with a 16-px cell.
    let want: &[(u16, u16, &str)] = &[
        (71, 71, "6"),
        (71, 79, "> "),
        (71, 95, "You have to do everything you can to keep your ship from"),
        (87, 71, "sinking before you get to Japan."),
        (103, 71, "5"),
        (103, 79, "> "),
    ];
    let got: Vec<(u16, u16, &str)> =
        runs.iter().take(want.len()).map(|(y, x, t)| (*y, *x, t.as_str())).collect();
    assert_eq!(got, want, "amiga: the clue as `amiga-shogun-hintshown.png` shows it\n{runs:#?}");

    // …and NOTHING reaches past the window, which is the defect's own signature:
    // the clue used to run to native x=639, the screen's edge.
    for (y, x, t) in &runs {
        let end = u32::from(*x) + s.machine.v6_metric.run_px(t, 0);
        assert!(end <= 571, "amiga: run {t:?} at ({x},{y}) ends at {end}, past the window's 570");
    }
}

/// The Macintosh press wraps at its OWN break point, which is not the Amiga's:
/// Geneva 12 is proportional, so more of the clue fits on the first line.
///
/// Asserted against `machine-screenshots/mac-shogun-hintshown.png` — the machine
/// breaks after `you`, and its first line measures native 72..557 where lanthorn
/// draws 71..559.
///
/// **This case needed SQ-1072 as well as SQ-1071.** Making the window wrap at all
/// was not enough: `wrap_text` breaks at whichever of the pixel and column limits
/// fills first (SQ-1009, deliberately), and the column limit was `x_size / cell.w`
/// = 71, which a 6.25-px pen reaches nine characters before it reaches 500 px. The
/// line came out a word short — and Arthur's Macintosh status ribbon lost the `e`
/// of `Compline` to the same edge. A proportional pen now has no column limit and
/// the grid grows to hold the line instead.
///
/// The face is Geneva off a System file the player supplies (SQ-1036), so this
/// press's PEN is machine-dependent; the case is skipped along with the medium and
/// its break point is the one that medium's face produces.
#[test]
fn mac_clue_wraps_where_the_machine_wrapped() {
    let Some(s) = clue_frame("Shogun.toast", 292, "890314") else { return };
    guard_shape(&s, "macintosh");
    let runs = win0_runs(&s);

    let want: &[(u16, u16, &str)] = &[
        (71, 71, "6"),
        (71, 79, "> "),
        (71, 89, "You have to do everything you can to keep your ship from sinking before you"),
        (86, 71, "get to Japan."),
        (101, 71, "5"),
        (101, 79, "> "),
    ];
    let got: Vec<(u16, u16, &str)> =
        runs.iter().take(want.len()).map(|(y, x, t)| (*y, *x, t.as_str())).collect();
    assert_eq!(got, want, "macintosh: the clue as `mac-shogun-hintshown.png` shows it\n{runs:#?}");

    // …and nothing reaches past the window, which is the defect's own signature:
    // the clue used to run to native x=639, the screen's edge.
    for (y, x, t) in &runs {
        let end = u32::from(*x) + s.machine.v6_metric.run_px(t, 0);
        assert!(end <= 571, "macintosh: run {t:?} at ({x},{y}) ends at {end}, past the window's 570");
    }
}
