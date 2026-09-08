//! SQ-1047 — `/dump-windows` names the FACE its metrics came from.
//!
//! # Why the dump needed this
//!
//! The report already carried every number a v6 layout is made of — the rect, the
//! grid, the cell, the margins, the usable width — and not one word about which
//! typeface produced them. So a DISK-FONT defect (the wrong face admitted, or none
//! at all) and a METRIC defect (the right face measured wrong) present identically:
//! a column count that does not match the machine. Twice in one day that cost a
//! round of reading `native_font.rs` to answer a question the dump was standing
//! right next to.
//!
//! # Why these two presses
//!
//! Because a default cannot falsify anything. `TextFace::cell_only` reports an 8x16
//! cell at scale (1, 1) with a fixed pen, which is also what a *broken* cascade
//! reports, so a case built on a synthetic face passes whether or not the face
//! reaches the dump at all. Both fixtures below carry a real face off a real
//! medium, and the two disagree on every field this quest added:
//!
//! | press | fixture | face | fit | declared cell | text scale |
//! |---|---|---|---|---|---|
//! | Amiga | `Arthur - The Quest for Excalibur.adf` (r54/890606) | `char.data` 10x10 | Metric | 8x20 | 2x2 |
//! | Macintosh | `InfocomMasterpieces.img`, ARTHUR FOLDER | `FONT` 7x15 | Cell | 7x15 | 1x1 |
//!
//! The Macintosh row is the one that keeps the scale honest: its artwork is doubled
//! exactly as the Amiga's is, and its text is *not* (SQ-1039). A report that scaled
//! the face by the ART scale would print 2x2 on both lines and look entirely
//! plausible.
//!
//! `stories/` is gitignored, so both cases skip vacuously when their medium is
//! absent.

use std::path::PathBuf;

use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::native_font::TextFace;
use app::session::GameSession;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Arthur's Amiga floppy, booted the way `startup.rs` boots (CLAUDE.md): the medium
/// picks the profile and the source, `PictSource` has its say about the screen and
/// the art density, and every per-machine fact travels in one `MachineBoot`.
///
/// `disks: None` — a case here must not depend on what the person running it keeps
/// in `~/.lanthorn/` (SQ-1037). The floppy answers on the first rung anyway.
///
/// **Turn count: 0.** The face is a LAUNCH fact and the window table is seeded at
/// `restart_screen`, so the boot frame carries everything this suite reads; driving
/// the intro would only make the fixture harder to describe.
fn amiga_arthur() -> Option<(GameSession, TextFace)> {
    const FIXTURE: &str = "Arthur - The Quest for Excalibur.adf";
    let path = stories_dir().join(FIXTURE);
    let Ok((loaded, _)) = app::hints::load_mounted_story(&path) else {
        eprintln!("SKIP: gitignored floppy missing at {}", path.display());
        return None;
    };
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 54, "{FIXTURE}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), "890606", "{FIXTURE}: serial");
    let (profile, source) = InterpreterProfile::resolve_with_source(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let faces = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source,
        art_scale: picts.art_scale(),
        disks: None,
    });
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        faces,
        profile.palette(),
        None,
    );
    let face = boot.text_face();
    let mut session =
        GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &boot)
            .unwrap_or_else(|e| panic!("{FIXTURE}: should boot without a ZError: {e:?}"));
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some((session, face))
}

/// Arthur off the Macintosh compilation volume — same game, different machine, and
/// the press whose text scale stays (1, 1) under doubled artwork.
///
/// `disks: None`, for the same reason as the Amiga case and with more force here:
/// `UserDisks::new("")` reads the REAL `~/.lanthorn/`, and on a machine that has a
/// Mac OS System disk in it the body face resolves to `FONT 396` off that disk
/// instead of to the release's own. Consulting no disks at all is the only way this
/// case reports the same face on every machine that runs it.
fn mac_arthur() -> Option<(GameSession, TextFace)> {
    const ENTRY: &str = "InfocomMasterpieces/ARTHUR FOLDER/STORY.DATA";
    let path = stories_dir().join("InfocomMasterpieces.img");
    if !path.is_file() {
        eprintln!("SKIP: gitignored compilation volume missing at {}", path.display());
        return None;
    }
    let (profile, source) = InterpreterProfile::resolve_with_source(&path, None, None, None);
    let bytes = match app::hints::load_mounted_story_from(&path, Some(ENTRY)).ok()?.0 {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("Arthur is Z-code on this volume, got {other:?}"),
    };
    let mut picts =
        PictSource::resolve_with_override(&path, app::graphics::PictureOverride::Unset, Some(ENTRY));
    let picture_dims = picts.all_pict_dims();
    let faces = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: Some(ENTRY),
        profile,
        source,
        art_scale: picts.art_scale(),
        disks: None,
    });
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        faces,
        profile.palette(),
        None,
    );
    let face = boot.text_face();
    let mut session =
        GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &boot)
            .expect("Arthur boots off the Macintosh volume");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some((session, face))
}

/// The line of `dump` that starts with `needle`, trimmed — panics with the whole
/// dump when there is none, because a missing line and a wrong one are different
/// failures and only one of them is worth reading a diff for.
fn line<'a>(dump: &'a [String], needle: &str) -> &'a str {
    dump.iter()
        .map(|l| l.trim())
        .find(|l| l.starts_with(needle))
        .unwrap_or_else(|| panic!("no `{needle}` line in:\n{}", dump.join("\n")))
}

#[test]
fn amiga_arthur_dump_names_its_release_face_and_its_pen() {
    let Some((session, face)) = amiga_arthur() else { return };
    let dump = session.v6_window_dump(&[], Some(&face));

    // Non-vacuity: this really is the press with a proportional release face. A
    // cascade that admitted nothing would still produce a `face:` block, and every
    // assertion below would then be pinning the built-in.
    assert!(face.proportional(), "Arthur's Amiga floppy ships a proportional face");

    assert_eq!(line(&dump, "face:"), "face: one launch fact — every window below is set in it");
    assert_eq!(line(&dump, "body:"), "body: 10x10px from the release's own medium · fit Metric");
    // The Amiga shipped no fixed-pitch alternate — `char.data` is the whole story.
    assert_eq!(line(&dump, "fixed:"), "fixed: none — a fixed-pitch run takes the body face");
    // The three numbers that differ, all on one line: a 10-row face, a 20-row
    // declared line, and the (2, 2) between them.
    assert_eq!(
        line(&dump, "declared cell"),
        "declared cell 8x20px · text scale 2x2 native px per face px"
    );
    // The pen, in NATIVE pixels — the face's own 2..9 advances at this press's (2,
    // 2). `native_disk_font.rs` pins the same table in FACE pixels off the same
    // floppy (`i` 3, `m` 8, the blank space 3), and the smear of 1 is what keeps
    // bold words apart (SQ-1009).
    assert_eq!(
        line(&dump, "pen:"),
        "pen: proportional 4–18px over printable ASCII · bold +2px (smear 1)"
    );
}

#[test]
fn mac_arthur_dump_scales_the_face_by_the_text_scale_and_not_the_art_scale() {
    let Some((session, face)) = mac_arthur() else { return };
    let dump = session.v6_window_dump(&[], Some(&face));

    // Non-vacuity: a real face off the volume, not the built-in. `cell_only`'s
    // fallback is an 8x16 cell at no fit at all, and every line below would read
    // plausibly wrong if the cascade had returned it.
    assert_eq!(face.fit(), Some(app::native_font::FaceFit::Cell), "the volume's own FONT is the 7x15 cell");

    // The Macintosh ships ONE face that is both roles: it IS the declared cell, so
    // it takes the fixed-pitch slot, and with nothing proportional admitted it
    // draws the body too (SQ-1036). Both lines name it, which is the point — put a
    // System disk under `~/.lanthorn/` and only the `body:` line moves.
    assert_eq!(line(&dump, "body:"), "body: 7x15px from the release's own medium · fit Cell");
    assert_eq!(line(&dump, "fixed:"), "fixed: 7x15px from the release's own medium");
    // **This is the line the quest exists for.** The artwork on this volume is
    // doubled exactly as the Amiga's is, and the text is not: a report that scaled
    // the face by the ART scale would say 2x2 here and be wrong in a way nothing on
    // screen would contradict (SQ-1039).
    assert_eq!(
        line(&dump, "declared cell"),
        "declared cell 7x15px · text scale 1x1 native px per face px"
    );
    assert_eq!(line(&dump, "pen:"), "pen: steps the cell — 7px for every character");
}

/// Every window reports the font props the GAME can read back — ZMSD §8.8.3.2
/// properties 12 and 13 — and they are seeded from the declared cell.
///
/// Per window because they ARE per window: a game may write prop 13, and Shogun
/// reads the width back out of it to size its input buffer. The dump says so
/// against the machine's cell, so a window the game re-sized stands out instead of
/// blending into seven that did not.
#[test]
fn every_window_reports_its_own_font_props_against_the_declared_cell() {
    let Some((session, face)) = amiga_arthur() else { return };
    let dump = session.v6_window_dump(&[], Some(&face));

    let fonts: Vec<&String> = dump.iter().filter(|l| l.trim_start().starts_with("font:")).collect();
    let wins = dump.iter().filter(|l| l.trim_start().starts_with("win")).count();
    assert!(wins > 0, "the boot frame publishes at least one window:\n{}", dump.join("\n"));
    assert_eq!(fonts.len(), wins, "one font line per window block:\n{}", dump.join("\n"));
    for f in &fonts {
        assert_eq!(
            f.trim(),
            "font: number 1 · size 8x20px (props 12/13)",
            "restart_screen seeds every window from the declared cell:\n{}",
            dump.join("\n")
        );
    }
}

/// The engine-only view says it has no face rather than inventing a default.
///
/// `Engine::window_dump` has no `AppState` in scope and so cannot know which face
/// is live. Printing `TextFace::cell_only`'s 8x16 there would be a fabricated
/// answer that reads exactly like a real one — the SQ-0901 shape — so it prints the
/// absence instead.
#[test]
fn the_engine_only_view_reports_no_face_rather_than_a_default_one() {
    let Some((session, _face)) = amiga_arthur() else { return };
    let dump = app::engine::Engine::window_dump(&session);
    assert_eq!(line(&dump, "face:"), "face: not supplied — engine-only view");
    assert!(
        !dump.iter().any(|l| l.contains("declared cell")),
        "and no metrics attributed to a face it does not have:\n{}",
        dump.join("\n")
    );
}
