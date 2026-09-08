//! A restored canvas must survive the next palette change — SQ-0587.
//!
//! Host Save State persists each v6 window's canvas as a PNG: pixels, not the draw
//! ops that produced them. SQ-0567 recolours the screen when the Current Palette
//! changes (Blorb §11.3) by CLEARING each window's canvas and replaying its display
//! list under the new palette — which is right for a window whose ops built it, and
//! destroys one whose pixels were restored, because its list is empty. Worse, it is
//! not merely empty: `erase_screen_rect` records an Erase op against every window a
//! LATER erase punches, so a restored window's list can hold nothing but erases, and
//! the replay faithfully reproduces "blank".
//!
//! Arthur is the case that shows it. One move after a restore recolours the palette;
//! its full-screen border window replays a list of pure erases and the surrounding
//! art vanishes, while the room picture — redrawn by the game that same turn — stays.
//! That asymmetry (centre survives, surround does not) is the signature.
//!
//! Skip-if-missing per the other gitignored-story smokes.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn picts() -> PictSource {
    let p = stories_dir().join("arthur-r74-s890714.z6");
    PictSource::new(blorb::resolve_resource_blorb(&p).map(|(b, _)| b))
}

fn boot() -> Option<GameSession> {
    let p = stories_dir().join("arthur-r74-s890714.z6");
    let bytes = std::fs::read(&p).ok()?;
    let mut pic = picts();
    let dims = pic.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, pic.std_window(), None, None)
        .expect("Arthur (v6) boots");
    s.set_pict_source(Some(pic));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..12 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
    }
    Some(s)
}

/// The lowest opaque row across every window canvas — how far down the screen still
/// carries art. Arthur's border reaches the bottom of its 400px screen; when the
/// replay wipes it, this collapses to the room picture's own extent.
fn art_bottom(session: &GameSession) -> u32 {
    let mut bottom = 0;
    for (win, canvas) in session.pictures_canvas.iter() {
        let top = session
            .machine
            .screen
            .v6
            .as_ref()
            .map(|v6| v6.windows[*win as usize].y_coord.saturating_sub(1) as u32)
            .unwrap_or(0);
        let img = &canvas.img;
        for y in (0..img.height()).rev() {
            if (0..img.width()).any(|x| img.get_pixel(x, y)[3] >= 128) {
                bottom = bottom.max(top + y);
                break;
            }
        }
    }
    bottom
}

#[test]
fn a_restored_canvas_survives_the_next_palette_change() {
    let Some(mut session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let _ = session.submit("look");
    let _ = session.take_transcript();
    let before = art_bottom(&session);
    assert!(before > 300, "Arthur's art reaches down the screen before saving (got {before})");

    // Through the real archive, as Save State and auto-resume do.
    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(&session);
    let path = std::env::temp_dir().join(format!("arthur-replay-{}.lanthorn", std::process::id()));
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        Some(&session.machine.screen),
        &session.machine.aux_data,
        app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 0, saved_at: String::new(),
            location: None, score: None, trigger: app::archive::SaveTrigger::HostState,
        },
        &app::archive::SessionRecord::empty(),
        // No display list: this test pins the LEGACY (pixels-only) restore path.
        &session.pictures_png(),
        None,
        None,
    )
    .expect("save archive");
    let ac = app::archive::load_archive(&path).expect("load archive");
    let _ = std::fs::remove_file(&path);

    let mut fresh = boot().expect("fresh boot");
    Engine::restore_state(&mut fresh, &ac.engine_save()).expect("restore");
    app::session::restore_screen(&mut fresh, ac.screen.clone().expect("screen"));
    fresh.load_pictures_png(&ac.pictures);
    fresh.set_pict_source(Some(picts()));
    let restored = art_bottom(&fresh);
    assert!(restored > 300, "the restore brings the art back (got {restored})");

    // Moving recolours the palette — the frame that used to wipe the restored art.
    for cmd in ["e", "w", "look"] {
        let r = fresh.submit(cmd);
        assert!(!r.quit && r.fault.is_none(), "\"{cmd}\" faulted/quit");
        let _ = fresh.take_transcript();
        let now = art_bottom(&fresh);
        assert!(
            now > 300,
            "the restored art survives the palette change on \"{cmd}\": art bottom fell to {now} \
             (was {restored} right after the restore) — the replay wiped a canvas it could not rebuild"
        );
    }
}
