//! Adaptive-palette (Blorb spec §11.3, SQ-0485) headless oracle on the real
//! Zork Zero blorb. Zork0's compass overlays (Pict 9–24, 205–329, …) are listed
//! in the `APal` chunk with PLACEHOLDER palettes (the stock 16-colour EGA
//! table). At boot Zork0 draws non-adaptive base art (Pict 5, then 497/498)
//! which establishes the "Current Palette" — the Zork0 game palette — then draws
//! the adaptive compass overlays, which must be plotted with that palette rather
//! than their own EGA placeholder. Before SQ-0485 lanthorn decoded every PNG
//! with its embedded palette, so the compass rendered in wrong (EGA) colours.
//!
//! Skips cleanly when the gitignored story is absent (CI), mirroring the other
//! `zork0_v6_*` tests.

use std::collections::HashSet;
use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::GameSession;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Distinct non-transparent RGB colours in a decoded picture.
fn opaque_colors(img: &image::DynamicImage) -> HashSet<[u8; 3]> {
    img.to_rgba8()
        .pixels()
        .filter(|p| p.0[3] != 0)
        .map(|p| [p.0[0], p.0[1], p.0[2]])
        .collect()
}

#[test]
fn zork0_compass_overlay_decodes_with_the_base_game_palette() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    // Boot Zork0 the same way `zork0_v6_windows_smoke` does: resolve the Pict
    // sidecar (`Zork0.blb`), inject the dimension table before boot, construct,
    // thread the Pict source on, and flush the boot-time picture draws. That
    // flush replays Zork0's real opening draw sequence — base Pict 5/497/498
    // (which set the Current Palette to the game palette) followed by the
    // adaptive compass overlays (17, 10, 11, 20, 13, 22, 15, 24) — so the
    // adaptive pictures are decoded through the palette-substitution path.
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    // Sanity: the sidecar actually declares adaptive pictures.
    let blorb2 = blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b).expect("sidecar");
    assert!(blorb2.is_adaptive_picture(10), "Pict 10 must be listed adaptive in APal");
    assert!(!blorb2.is_adaptive_picture(5), "base Pict 5 must NOT be adaptive");

    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit && session.machine.fault_trace.is_none(), "Zork0 booted cleanly");

    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    // The Zork0 game palette (verified from the base pictures 5/497/498/2 in the
    // real blorb — all share this 16-colour table). Every visible colour of a
    // correctly-substituted compass overlay must come from here.
    let base_palette: HashSet<[u8; 3]> = [
        [0, 0, 0], [0, 0, 0], [238, 170, 136], [204, 136, 102], [170, 102, 68], [136, 68, 34],
        [102, 34, 0], [68, 0, 0], [204, 0, 0], [170, 0, 0], [102, 0, 0], [34, 34, 34],
        [238, 136, 0], [102, 102, 102], [68, 68, 68], [238, 187, 119],
    ]
    .into_iter()
    .collect();

    // Substituted decode: the overlay as the boot actually composited it (the
    // Current Palette was the game palette by the time the compass drew).
    let sub = session
        .pict_source_mut()
        .expect("pict source")
        .image(10)
        .expect("adaptive compass overlay Pict 10 decodes");
    let sub_colors = opaque_colors(&sub);

    // Placeholder decode: a fresh Pict source with no base picture ever drawn
    // through it falls back to the overlay's own EGA placeholder palette — what
    // lanthorn wrongly rendered before this fix.
    let mut raw_src = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let raw = raw_src.image(10).expect("placeholder decode of Pict 10");
    let raw_colors = opaque_colors(&raw);

    eprintln!("Pict 10 substituted colours: {sub_colors:?}");
    eprintln!("Pict 10 placeholder colours: {raw_colors:?}");

    assert!(!sub_colors.is_empty(), "the compass overlay has visible pixels");
    assert!(
        sub_colors.iter().all(|c| base_palette.contains(c)),
        "every visible overlay colour must come from the base game palette, not the placeholder; \
         offending: {:?}",
        sub_colors.iter().filter(|c| !base_palette.contains(*c)).collect::<Vec<_>>()
    );
    assert!(
        raw_colors.iter().any(|c| !base_palette.contains(c)),
        "the placeholder (unsubstituted) decode must contain an off-palette EGA colour — \
         otherwise this test proves nothing about the fix; placeholder: {raw_colors:?}"
    );
    assert_ne!(sub_colors, raw_colors, "palette substitution changed the overlay's colours");
}
