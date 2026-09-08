//! SQ-0743 — a picture with no palette of its own is adaptive, whichever archive
//! it came out of.
//!
//! Booted off its Amiga floppy, Zork Zero drew its compass arrows and room icons
//! in bright blue, purple and yellow-green, and the colours drifted from room to
//! room; the same game booted from `Zork0.blb` drew them correctly.
//!
//! MEASURED CAUSE. Blorb declares adaptivity with a top-level `APal` chunk
//! (§11.3): the listed pictures carry a placeholder palette and must be plotted
//! with the *Current Palette*, the palette of the most recently drawn
//! non-adaptive picture. `graphics.rs` implements that, and SQ-0720's 38930-row
//! `BPal` oracle proves it matches Infocom's own converter. The native `Pic.data`
//! archive has no `APal` chunk — but it says the same thing per picture, in the
//! 3-byte **palette offset** of each 14-byte directory record. A zero there
//! means the picture has no colours of its own. For Zork Zero the 172 records
//! with pixels and a zero palette offset are, id for id, exactly the 172 numbers
//! `Zork0.blb` lists in `APal`: the Blorb's chunk was derived from this field.
//!
//! `PictSource` treated a native archive as having no adaptive pictures at all,
//! so those 172 expanded through the archive's stock 16-colour fallback — the
//! EGA table, whose entries 9, 13 and 14 are precisely bright blue, purple and
//! yellow — and the native Current Palette was never established (`None`) no
//! matter how many room illustrations the game drew. Feeding the native source
//! into the same Current-Palette machinery is the whole fix; the Current Palette
//! is the same RGB triples either way, so one representation still serves both
//! archives and the host Save State that carries it.
//!
//! Both fixtures are gitignored, so every case **skips vacuously** when absent.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The largest per-channel difference tolerated between the two archives.
///
/// Amiga hardware palettes are 4 bits per channel, so every colour byte in a
/// native palette — and in every Blorb PLTE converted from one — is a multiple
/// of `0x11`. The smallest *genuine* disagreement the format can express is
/// therefore one 4-bit step, 17, and a tolerance of 16 is the tightest bound
/// that cannot hide one. The measured residual is well under it: 10, and only
/// against Pict 8, whose Blorb palette (128/160/96/64) is not a 12-bit Amiga
/// palette at all — it is the substituted asset SQ-0713 catalogued, where the
/// native archive holds the real frame and the Blorb a solid rectangle.
const TOLERANCE: u16 = 16;

fn read(name: &str) -> Option<Vec<u8>> {
    let p = stories_dir().join(name);
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored fixture missing at {}", p.display());
            None
        }
    }
}

fn native() -> Option<blorb::infocom_pics::InfocomPics> {
    Some(blorb::infocom_pics::InfocomPics::parse(read("zork0.pic")?).expect("a native archive parses"))
}

/// `(width, height, RGBA8 bytes)` of a decoded picture.
fn rgba(img: &image::DynamicImage) -> (u32, u32, Vec<u8>) {
    let b = img.to_rgba8();
    (b.width(), b.height(), b.into_raw())
}

/// The largest per-channel difference between two decodes, ignoring pixels both
/// call fully transparent. `None` when they disagree on size — a different
/// picture, not a different palette.
fn max_delta(a: &(u32, u32, Vec<u8>), b: &(u32, u32, Vec<u8>)) -> Option<u16> {
    if (a.0, a.1) != (b.0, b.1) {
        return None;
    }
    let mut m = 0u16;
    for (p, q) in a.2.as_chunks::<4>().0.iter().zip(b.2.as_chunks::<4>().0.iter()) {
        if p[3] == 0 && q[3] == 0 {
            continue;
        }
        for c in 0..4 {
            m = m.max((i16::from(p[c]) - i16::from(q[c])).unsigned_abs());
        }
    }
    Some(m)
}

/// The format fact the whole fix rests on, and the one lookup that establishes
/// it: the compass arrows carry NO palette of their own, so they are adaptive by
/// the native format's own convention — and the set of such pictures is exactly
/// the Blorb's `APal` list, per picture, not archive-wide.
#[test]
fn a_zero_palette_offset_is_the_native_apal_chunk() {
    let Some(pics) = native() else { return };

    // Zork Zero's 16 compass overlays, the pictures the report is about.
    for id in 9..=24u16 {
        let e = pics.entry(id).expect("compass overlay is in the directory");
        assert!(e.has_pixels(), "compass overlay {id} carries pixels");
        assert!(
            !e.has_own_palette(),
            "compass overlay {id} must have NO palette of its own — that is what makes it adaptive"
        );
    }
    // …while the room illustrations beside them do carry one, so this is a
    // per-picture distinction and never an archive-wide mode.
    for id in [1u16, 5, 25, 26, 34] {
        assert!(
            pics.entry(id).expect("illustration is in the directory").has_own_palette(),
            "illustration {id} carries its own palette"
        );
    }

    let Some(blb) = read("Zork0.blb") else { return };
    let blorb = blorb::Blorb::parse(blb).expect("Zork0.blb parses");
    let native_set: HashSet<u32> = pics.adaptive_pictures().iter().map(|&i| u32::from(i)).collect();
    let apal: HashSet<u32> = blorb.adaptive_pictures().iter().copied().collect();
    assert_eq!(native_set.len(), 172, "Zork Zero has 172 adaptive pictures");
    assert_eq!(
        native_set, apal,
        "the native zero-palette records and the Blorb's APal list must name the same pictures"
    );
}

/// The oracle. Infocom's own converter pre-computed §11.3 for every
/// (background, adaptive) pair and stored the answers as extra `Pict` resources
/// indexed by the `BPal` chunk (SQ-0720). Replaying that table through the
/// NATIVE archive asks a party that never saw this code whether the colours are
/// right — for every one of the 172 adaptive pictures, under 216 different
/// current palettes.
///
/// Rows the native archive cannot serve are skipped, not fudged: backgrounds
/// 497–504 exist only in the Blorb (SQ-0713's unexplained high-id block).
#[test]
fn the_native_archive_reproduces_the_converters_bake() {
    let (Some(blb), Some(pic_bytes)) = (read("Zork0.blb"), read("zork0.pic")) else { return };
    let table = blorb::bpal::palette_bakes(&blb);
    assert!(table.len() > 38000, "Zork0.blb must carry its full BPal table, got {}", table.len());
    let blorb = blorb::Blorb::parse(blb).expect("Zork0.blb parses");

    let mut baked: HashMap<u32, (u32, u32, Vec<u8>)> = HashMap::new();
    let (mut rows, mut exact, mut skipped, mut worst) = (0usize, 0usize, 0usize, 0u16);
    let mut offenders: HashSet<u32> = HashSet::new();
    let mut failures: Vec<String> = Vec::new();

    // Walk the table in background runs, one `PictSource` per run: the adaptive
    // decode cache is keyed by palette generation, so a single source across all
    // 224 backgrounds would hold every one of the 38528 results at once.
    let mut i = 0;
    while i < table.len() {
        let bg = table[i].background;
        let mut j = i;
        while j < table.len() && table[j].background == bg {
            j += 1;
        }
        let mut src = PictSource::from_native(
            blorb::infocom_pics::InfocomPics::parse(pic_bytes.clone()).expect("parses"),
        );
        // Drawing the non-adaptive background is what establishes the Current
        // Palette — the precondition every row of the table describes.
        if src.image(bg).is_none() {
            skipped += j - i;
            i = j;
            continue;
        }
        for r in &table[i..j] {
            let Some(got) = src.image(r.adaptive) else {
                skipped += 1;
                continue;
            };
            let want = baked.entry(r.baked).or_insert_with(|| {
                let (_ty, png) = blorb.resource(b"Pict", r.baked).expect("baked Pict exists");
                rgba(&app::cover::decode(png).expect("baked Pict decodes"))
            });
            rows += 1;
            let d = max_delta(&rgba(&got), want);
            match d {
                Some(0) => exact += 1,
                Some(d) => {
                    worst = worst.max(d);
                    offenders.insert(r.background);
                    if failures.len() < 10 {
                        failures.push(format!(
                            "background={} adaptive={} baked={}: max channel delta {d}",
                            r.background, r.adaptive, r.baked
                        ));
                    }
                }
                None => {
                    offenders.insert(r.background);
                    if failures.len() < 10 {
                        failures.push(format!(
                            "background={} adaptive={} baked={}: sizes differ",
                            r.background, r.adaptive, r.baked
                        ));
                    }
                    worst = u16::MAX;
                }
            }
        }
        i = j;
    }

    eprintln!("native BPal replay: {exact}/{rows} exact, {skipped} skipped, worst delta {worst}");
    assert!(rows > 37_000, "only {rows} BPal rows replayed through the native archive — wrong fixture?");
    assert!(
        worst <= TOLERANCE,
        "the native archive disagrees with the converter by {worst} (tolerance {TOLERANCE}):\n  {}",
        failures.join("\n  ")
    );
    // Everything but one background is byte-exact, and that background is the
    // substituted asset — narrow the claim so a regression cannot hide inside
    // the tolerance by spreading itself over other backgrounds.
    assert_eq!(
        offenders,
        HashSet::from([8]),
        "only Pict 8 — the picture the two archives genuinely disagree about — may differ at all"
    );
    assert!(exact >= 36_900, "only {exact} rows are byte-exact");
}

/// The same thing where the player sees it: the compass arrows and room icons a
/// real boot of Zork Zero draws must come out of the native archive with the
/// colours the Blorb path gives them, under the palette the game itself
/// established by then.
#[test]
fn the_compass_off_native_media_matches_the_blorb_in_a_real_game() {
    for honor_game_colours in [true, false] {
        let Some(story) = read("zork0-r393-s890714.z6") else { return };
        let Some(pics) = native() else { return };
        let adaptive: Vec<u32> = pics.adaptive_pictures().iter().map(|&i| u32::from(i)).collect();

        // `std_win` stands in for the Blorb `Reso` chunk a native archive has
        // not got — what the Amiga interpreter profile supplies at startup.
        let mut n = opening(PictSource::from_native(pics), story.clone(), Some((320, 200)), honor_game_colours);
        let story_path = stories_dir().join("zork0-r393-s890714.z6");
        let mut b = opening(
            PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(x, _)| x)),
            story,
            None,
            honor_game_colours,
        );

        // The mechanism, not just the outcome: the native path establishes a
        // Current Palette at all. It was `None` before this fix, forever.
        let np = n.pict_source_mut().expect("native source").current_palette().map(<[u8]>::to_vec);
        let bp = b.pict_source_mut().expect("blorb source").current_palette().map(<[u8]>::to_vec);
        let np = np.unwrap_or_else(|| panic!(
            "honor_game_colours={honor_game_colours}: a native archive must establish a Current Palette"
        ));
        let bp = bp.expect("the Blorb path establishes one");
        // Entries 2..=15 are the picture's own colours and must agree exactly.
        // 0 and 1 are the Amiga's reserved registers, which the two archives
        // fill differently and no pixel of these pictures indexes (SQ-0740).
        assert_eq!(
            np[6..], bp[6..],
            "honor_game_colours={honor_game_colours}: both archives must load the same game palette"
        );

        let (mut compared, mut worst) = (0usize, 0u16);
        for id in adaptive {
            let (Some(ni), Some(bi)) = (
                n.pict_source_mut().unwrap().image(id),
                b.pict_source_mut().unwrap().image(id),
            ) else {
                continue;
            };
            let Some(d) = max_delta(&rgba(&ni), &rgba(&bi)) else { continue };
            compared += 1;
            worst = worst.max(d);
            assert!(
                d <= TOLERANCE,
                "honor_game_colours={honor_game_colours}: adaptive picture {id} differs from the \
                 Blorb by {d} (tolerance {TOLERANCE})"
            );
        }
        eprintln!("honor_game_colours={honor_game_colours}: {compared} adaptive pictures, worst delta {worst}");
        assert_eq!(
            compared, 172,
            "honor_game_colours={honor_game_colours}: every adaptive picture must resolve from both archives"
        );
    }
}

/// Boot Zork Zero against `picts` and play the opening, so the game itself has
/// drawn the illustrations that set the Current Palette.
fn opening(
    mut picts: PictSource,
    story: Vec<u8>,
    std_win: Option<(u16, u16)>,
    honor_game_colours: bool,
) -> GameSession {
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = std_win.or_else(|| picts.std_window());
    let mut session = GameSession::new_with_trace(
        story,
        honor_game_colours,
        false,
        None,
        false,
        picture_dims,
        v6_screen_px,
        None,
        None,
    )
    .expect("Zork Zero (v6) loads and boots without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    for _ in 0..6 {
        match session.pending_input() {
            InputKind::Line => session.submit("look"),
            InputKind::Char => session.submit_char(b' '),
            InputKind::Event => session.submit(""),
        };
    }
    session
}
