//! SQ-1572: a host restore must give a v6 story back its screen in the exact
//! PIXELS it was declared in, not reconstituted from the character grid.
//!
//! `post_restore_fixups` (`zvm::cpu::exec::Machine`) used to re-apply the
//! screen via `set_screen_dims(rows, cols)` for every version, v6 included.
//! For v6 that multiplies the (truncated) character grid back up —
//! `cols * cell.w()` / `rows * cell.h()` — which is exact only while the cell
//! divides the screen. The Macintosh's own 7x15 cell does not: `640 / 7 = 91`
//! (truncated), and `91 * 7` comes back **637**, not 640; `400 / 15 = 26`, and
//! `26 * 15` comes back **390**, not 400. So a restore shrank the screen from
//! 640x400 to 637x390.
//!
//! `@restart` got the equivalent fix at SQ-1156: keep the screen the host
//! declared, in pixels, and re-apply it with `set_v6_screen_px` rather than
//! reconstituting it. SQ-1572 gives `post_restore_fixups` the same fix,
//! sharing that one helper so both paths are driven by a single rule.
//!
//! Per CLAUDE.md's restore-testing convention, this restores through the real
//! archive (as host Save State / Restore State do), PERTURBS with a move, and
//! only THEN asserts — the frame right after a restore still looks correct;
//! the shrink is a property of a REPAINT, so it wants a repaint to see.
//!
//! Skip-if-missing per the other gitignored-story smokes.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The window this specimen booted, so a failure names it rather than a bare
/// number — CLAUDE.md's "print the profile, release and screen size" rule.
struct Booted {
    session: GameSession,
    screen_px: Option<(u16, u16)>,
    label: &'static str,
}

/// Boot Zork Zero off the Macintosh disk, on its DEFAULT (colour) archive —
/// `CPic.data`, 320x200 doubled to 640x400 (SQ-0838) — the specimen this suite
/// exists for: the Macintosh's 7x15 cell divides neither axis of that screen.
fn boot_mac_colour() -> Option<Booted> {
    let path = stories_dir().join("Zork Zero Disk.image");
    if !path.exists() {
        eprintln!("SKIP: gitignored Macintosh medium missing at {}", path.display());
        return None;
    }
    let bytes = match app::hints::load_story(&path).expect("Story.data mounts") {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code, got {other:?}"),
    };
    let dir = app::scratch_dir("sq1572-mac-restore");
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve_with_override(&path, PictureOverride::Unset, None);
    let picture_dims = picts.all_pict_dims();
    let honoured = !picts.declines_game_colours(profile.default_colours());
    let default_colours = honoured.then(|| profile.default_colours()).flatten();
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        default_colours,
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    eprintln!(
        "boot_mac_colour: profile {profile:?} · screen {:?} · art_scale {:?} · cell {:?}",
        boot.screen_px, boot.art_scale, boot.cell
    );
    let mut session =
        GameSession::new_for_machine(bytes, honoured, false, false, picture_dims, None, None, &boot)
            .expect("Zork Zero boots off the Macintosh disk");
    assert!(!session.quit, "quit during boot");
    assert!(session.machine.fault_trace.is_none(), "faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    let _ = std::fs::remove_dir_all(&dir);
    Some(Booted { session, screen_px: boot.screen_px, label: "Mac Zork Zero (7x15 cell, 640x400)" })
}

/// Boot Zork Zero release 393 off the bare story file — no disk medium, the
/// press whose declared cell (8x16, SQ-0917) divides 640x400 exactly, so it
/// was already correct before this fix: the regression guard proving this
/// change did not touch the case that worked.
fn boot_r393() -> Option<Booted> {
    let path = stories_dir().join("zork0-r393-s890714.z6");
    if !path.exists() {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let default_colours = profile.default_colours();
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        default_colours,
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    eprintln!(
        "boot_r393: profile {profile:?} · screen {:?} · art_scale {:?} · cell {:?}",
        boot.screen_px, boot.art_scale, boot.cell
    );
    let mut session =
        GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &boot)
            .expect("Zork Zero r393 boots");
    assert!(!session.quit, "quit during boot");
    assert!(session.machine.fault_trace.is_none(), "faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(Booted { session, screen_px: boot.screen_px, label: "Zork Zero r393 (8x16 cell)" })
}

/// Answer whatever the game is waiting on `taps` times, saying `n` to a
/// `y or n` prompt — the intro-card skip every v6 harness here uses.
fn tap_in(session: &mut GameSession, taps: usize) {
    for _ in 0..taps {
        let t = match session.pending_input() {
            InputKind::Line => session.submit("").transcript,
            InputKind::Char => session.submit_char(13).transcript,
            InputKind::Event => session.submit("").transcript,
        };
        if t.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
}

/// The screen facts this suite cares about, read straight off the machine so
/// a shrink anywhere in the chain shows up: the header's own declared pixel
/// screen ($22/$24), the character grid it derives ($20/$21), and window 0's
/// own native-pixel box — which ZMSD §8.8.3.3 says occupies the whole screen,
/// so it is the "story window" the acceptance criteria mean.
#[derive(Debug, PartialEq)]
struct ScreenFacts {
    header_px: (u16, u16),
    header_grid: (u8, u8),
    window0_px: (u16, u16),
}

fn screen_facts(b: &Booted) -> ScreenFacts {
    let m = &b.session.machine;
    let win0 = &m.screen.v6.as_ref().expect("v6 screen model").windows[0];
    ScreenFacts {
        header_px: (m.mem.read_word(0x22), m.mem.read_word(0x24)),
        header_grid: (m.mem.read_byte(0x20), m.mem.read_byte(0x21)),
        window0_px: (win0.x_size, win0.y_size),
    }
}

/// Save through the real archive (as host Save State does), restore into a
/// freshly booted session, PERTURB with a move, and return the restored,
/// perturbed `Booted` alongside the facts captured right before the save.
fn save_restore_perturb(b: Booted, rebook: impl Fn() -> Option<Booted>) -> (Booted, ScreenFacts) {
    let before = screen_facts(&b);

    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(&b.session);
    let path = app::scratch_dir("sq1572-restore").join("save.lanthorn");
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        Some(&b.session.machine.screen),
        &b.session.machine.aux_data,
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
        &b.session.pictures_png(),
        None,
        None,
    )
    .expect("save archive");
    let ac = app::archive::load_archive(&path).expect("load archive");
    let _ = std::fs::remove_file(&path);

    let mut fresh = rebook().expect("fresh boot for restore");
    Engine::restore_state(&mut fresh.session, &ac.engine_save()).expect("restore");
    app::session::restore_screen(&mut fresh.session, ac.screen.clone().expect("screen"));
    fresh.session.load_pictures_png(&ac.pictures);

    // PERTURB: everything still looks correct on the frame right after a
    // restore (CLAUDE.md) — the shrink surfaces on the next repaint.
    let r = fresh.session.submit("look");
    assert!(r.fault.is_none() && !r.quit, "{}: \"look\" after restore faulted/quit: {:?}", b.label, r.fault);
    let _ = fresh.session.take_transcript();

    (fresh, before)
}

/// The Macintosh case: the screen must come back 640x400, not the 637x390 a
/// cell-grid reconstitution produces, and the story window must not have lost
/// a row of it.
#[test]
fn mac_zork_zero_keeps_its_640x400_screen_across_a_host_restore() {
    let Some(mut b) = boot_mac_colour() else { return };
    tap_in(&mut b.session, 12);
    let _ = b.session.take_transcript();

    let (restored, before) = save_restore_perturb(b, || {
        let mut fresh = boot_mac_colour()?;
        tap_in(&mut fresh.session, 12);
        let _ = fresh.session.take_transcript();
        Some(fresh)
    });

    assert_eq!(before.header_px, (640, 400), "sanity — the Mac colour archive declares 640x400");

    let after = screen_facts(&restored);
    eprintln!(
        "{}: before {:?} / after {:?}",
        restored.label, before, after
    );
    assert_eq!(
        after.header_px, before.header_px,
        "{}: the screen shrank across a restore — header $22/$24 came back reconstituted \
         from the character grid instead of the pixels the host declared",
        restored.label
    );
    assert_eq!(
        after.header_grid, before.header_grid,
        "{}: the character grid changed across a restore", restored.label
    );
    assert_eq!(
        after.window0_px, before.window0_px,
        "{}: the story window (window 0, which occupies the whole screen per ZMSD §8.8.3.3) \
         lost pixels across a restore",
        restored.label
    );
    assert_eq!(after.header_px, (640, 400), "the composed screen is still native 640x400");
}

/// The regression guard: release 393's 8x16 cell divides 640x400 exactly, so
/// this case was already correct before the fix and must stay that way.
#[test]
fn zork_zero_r393_screen_is_unchanged_across_a_host_restore_regression_guard() {
    let Some(mut b) = boot_r393() else { return };
    tap_in(&mut b.session, 6);
    let _ = b.session.take_transcript();

    let (restored, before) = save_restore_perturb(b, || {
        let mut fresh = boot_r393()?;
        tap_in(&mut fresh.session, 6);
        let _ = fresh.session.take_transcript();
        Some(fresh)
    });

    let after = screen_facts(&restored);
    eprintln!(
        "{}: before {:?} / after {:?}",
        restored.label, before, after
    );
    assert_eq!(
        after, before,
        "{}: an exact-dividing cell must round-trip a restore unchanged \
         whichever path post_restore_fixups takes",
        restored.label
    );
}
