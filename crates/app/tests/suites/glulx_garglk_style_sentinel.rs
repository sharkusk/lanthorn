//! Kerkerkruip's `0xF400A1` sentinel, end to end (SQ-0803 — the second half of
//! SQ-0319).
//!
//! The game asks the host one question before it decides how to present itself:
//! `glk_style_measure(gg_mainwin, style_User2, stylehint_TextColor)`. Its shipped
//! Gargoyle config paints `style_User2` "Fashion Fuchsia" (`tcolor 10 F400A1
//! ffffff`) precisely because nobody else ever would, so the answer tells the
//! game whether its own config file was applied. We import that ini and we RENDER
//! the fuchsia — but until SQ-0803 we reported the pane's base colour when asked,
//! which is the one thing the sentinel is not.
//!
//! **The ini beside the story is the opt-in.** Answering truthfully sends the
//! game down its Gargoyle branch: no screen-reader prompt, menu hyperlinks on,
//! graphics border strips. A player who would rather be asked simply does not
//! keep `Kerkerkruip.ini` next to the story — hence the second test here, which
//! pins that escape hatch by booting the same story from a directory with no ini.
//!
//! Constants verified against glk.h (eblong.com), not recall: `style_User2` = 10,
//! `stylehint_TextColor` = 7, `style_NUMSTYLES` = 11.
//!
//! Fixture: `stories/Kerkerkruip.gblorb` — Kerkerkruip 9.0.1, 2014-04-19, IFID
//! AC0DAF65-F40F-4A41-A4E4-50414F836E14. Note the ini's section header is
//! `[ Kerkerkruip.gblorb ]`, so it is that release the ini selects; the sibling
//! `Kerkerkruip.b10.gblorb` matches no section and keeps the un-imported look.

use std::path::{Path, PathBuf};

use app::engine::Engine;
use app::glk_backend::GlkStylePairs;
use app::glulx_session::GlulxSession;

use crate::fixture_paths::fixture_path;

const STORY: &str = "Kerkerkruip.gblorb";
/// glk.h: `style_User2`.
const STYLE_USER2: usize = 10;
/// The ini's `tcolor 10 F400A1 ffffff` — the sentinel the game looks for.
const FASHION_FUCHSIA: u32 = 0x00F4_00A1;

fn story_path() -> Option<PathBuf> {
    let p = fixture_path(STORY);
    if !p.is_file() {
        eprintln!("SKIP: gitignored fixture missing at {}", p.display());
        return None;
    }
    Some(p)
}

/// The theme colours the app would push into the Glk backend for a story at
/// `path` — the real chain: resolve a scheme, overlay whatever garglk.ini sits
/// beside the story (`startup.rs` does exactly this, before the engine boots),
/// then derive the per-Glk-style pairs.
fn theme_pairs_for(path: &Path) -> GlkStylePairs {
    let mut cs = app::colors::ColorScheme::default();
    if let Some(ov) = app::garglk_ini::discover(path) {
        ov.apply(&mut cs);
    }
    app::glk_backend::theme_style_colours(&cs)
}

/// Boot the story to its first input request with `theme` in place.
fn boot(image: Vec<u8>, blorb: blorb::Blorb, theme: GlkStylePairs) -> GlulxSession {
    GlulxSession::new_in(
        PathBuf::new(), // no persistent store: a fresh, never-answered install
        image,
        80,
        24,
        true,  // acceleration
        true,  // graphics (the Gargoyle branch opens border windows)
        false, // sound
        false, // borderless
        (8.0, 16.0),
        Some(blorb),
        &[], // empty VFS: the game has no remembered answer to reuse
        theme,
        false,
        None,
    )
    .expect("Kerkerkruip boots")
}

fn image_and_blorb(path: &Path) -> (Vec<u8>, blorb::Blorb) {
    let bytes = std::fs::read(path).expect("read the story");
    let blorb = blorb::Blorb::parse(bytes).expect("Kerkerkruip is a Blorb");
    let image = blorb.executable().expect("Glulx exec chunk").1.to_vec();
    (image, blorb)
}

#[test]
fn the_imported_ini_makes_style_user2_measure_fuchsia_and_takes_the_gargoyle_branch() {
    let Some(path) = story_path() else { return };
    assert!(
        path.with_file_name("Kerkerkruip.ini").is_file(),
        "this test needs the shipped Kerkerkruip.ini beside the story"
    );

    // 1. What we report for style_User2 IS what the ini says we paint.
    let theme = theme_pairs_for(&path);
    assert_eq!(
        theme[0][STYLE_USER2].0,
        Some(FASHION_FUCHSIA),
        "style_User2 must measure the ini's tcolor 10 slot, not the pane base"
    );
    assert_eq!(theme[0][STYLE_USER2].1, Some(0x00FF_FFFF), "…including its white background");
    assert_ne!(
        theme[0][0].0,
        Some(FASHION_FUCHSIA),
        "sanity: the pane base is not itself fuchsia, so the assertion above means something"
    );

    // 2. …and the game believes us: the Gargoyle branch never asks the question.
    //    It boots straight into its graphical main menu instead — a full-pane
    //    graphics window over a zero-height text buffer, waiting on an event
    //    (its menu hyperlinks), with nothing printed as prose at all.
    let (image, blorb) = image_and_blorb(&path);
    let mut sess = boot(image, blorb, theme);
    let text = sess.take_transcript();
    assert!(
        !text.to_lowercase().contains("screen reader"),
        "with its config applied Kerkerkruip must skip the screen-reader prompt; got:\n{text}"
    );
    let dump = sess.window_dump().join("\n");
    assert!(
        dump.contains("Graphics"),
        "sanity: the Gargoyle branch draws its main menu into a graphics window; layout was:\n{dump}"
    );
    assert_eq!(
        sess.pending_input(),
        app::session::InputKind::Event,
        "the graphical menu waits on an event, not on the prompt's keypress"
    );
}

#[test]
fn without_the_ini_the_screen_reader_prompt_is_unchanged() {
    let Some(real) = story_path() else { return };

    // Same story, no ini beside it — the player's escape hatch from the Gargoyle
    // presentation. Symlinked rather than copied: the fixture is 22 MB.
    let dir = std::env::temp_dir().join(format!("lanthorn-sq0803-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(STORY);
    let _ = std::fs::remove_file(&path);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &path).expect("symlink the fixture");
    #[cfg(not(unix))]
    std::fs::copy(&real, &path).expect("copy the fixture");

    assert!(app::garglk_ini::discover(&path).is_none(), "no ini beside this copy");
    let theme = theme_pairs_for(&path);
    assert_eq!(
        theme[0][STYLE_USER2], theme[0][0],
        "with no ini, style_User2 measures exactly what every other style does"
    );

    let (image, blorb) = image_and_blorb(&path);
    let mut sess = boot(image, blorb, theme);
    let text = sess.take_transcript();
    assert!(
        text.to_lowercase().contains("screen reader"),
        "without the ini Kerkerkruip must still offer its screen-reader mode; got:\n{text}"
    );
    // One plain text buffer, and the game is waiting for the Yes/No keypress —
    // no graphical menu, exactly as before SQ-0803.
    let dump = sess.window_dump().join("\n");
    assert!(!dump.contains("Graphics"), "no graphics main menu on this branch; layout was:\n{dump}");
    assert_eq!(sess.pending_input(), app::session::InputKind::Char);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}
