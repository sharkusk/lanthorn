//! SQ-0744 — Shogun's Amiga picture archive uses the format's *other* record
//! size, and must read alongside the one Zork Zero uses.
//!
//! Reported symptom, booting Shogun off its Amiga floppy: "not showing any
//! graphics at all, so no title screen". Every other Amiga release we mount —
//! Zork Zero, Journey, Arthur — drew its art fine.
//!
//! MEASURED CAUSE. `Pic.data`'s 16-byte header carries `gh.flags` at byte 1 and
//! `gh.dirEntryLen` at byte 8. Zork Zero, Journey and Arthur all declare flags
//! `6` and records of `14`; Shogun declares flags `2` and records of `16`. The
//! reader treated 14 as the marker of the Amiga flavour and rejected everything
//! else, so Shogun's whole archive was dropped before a single picture was
//! decoded — `all_pict_dims()` came back empty and the game painted no canvas at
//! all.
//!
//! Byte 1 is not a version number. It is the `HF_*` flag set, and
//! `ReadGFXEntry` in Infocom's own `amiga/gfx.c` reads it to decide the record
//! layout: `HF_EHUFF` without `HF_GHUFF` means every picture names its own
//! Huffman tree in a further word, making the record 16 bytes instead of 14.
//! Shogun's 48 records each carry a tree; the other three games share one global
//! tree named in the header. `blorb::infocom_pics` now derives the record size
//! from the flags and reads both.
//!
//! Both fixtures are gitignored, so every case **skips vacuously** when absent.

use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::GameSession;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

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

const ADF: &str = "James Clavell's Shogun.adf";

fn native() -> Option<blorb::infocom_pics::InfocomPics> {
    let adf = blorb::adf::Adf::mount(read(ADF)?).expect("the Shogun floppy mounts");
    Some(adf.pictures().expect("the floppy carries Pic.data").1)
}

/// Is the Blorb's image a pixel-for-pixel copy of the top-left `bw × bh` window
/// of the native decode? Fully transparent pixels count as equal to each other
/// whatever RGB sits under them.
fn matches_top_left(native: &[u8], nw: usize, blorb: &[u8], bw: usize, bh: usize) -> bool {
    (0..bh).all(|y| {
        (0..bw).all(|x| {
            let (n, b) = (((y * nw) + x) * 4, ((y * bw) + x) * 4);
            let (p, q) = (&native[n..n + 4], &blorb[b..b + 4]);
            (p[3] == 0 && q[3] == 0) || p == q
        })
    })
}

/// The oracle. `Shogun.blb` was produced by Infocom's own converter from this
/// very archive, so for every picture both hold, a native decode must reproduce
/// the converter's PNG byte for byte — the standard SQ-0713 held the 14-byte
/// reader to (383/388 against `Zork0.blb`).
///
/// Measured result: of the 39 pictures both sources carry, **34 are byte-exact**
/// — 15 at identical dimensions, and 19 where the Blorb kept a smaller window of
/// the same pixels (18 of them dropped the last scanline, 185 rows to 184; for
/// picture 3 it kept the leftmost 23 of 320 columns). The five that differ do so
/// for reasons that are about the Blorb, not the decode:
///
/// * **16, 25** — the pixels are identical; the Blorb's `PLTE` quantises the
///   Amiga's 4-bit channels as `n << 4` where the native palette (and every
///   other picture in this Blorb) uses `n * 0x11`. Max channel delta 14, below
///   the 17 that one 4-bit step costs, so no colour is actually different.
/// * **37** — 1310 of 34965 pixels (3.7%) genuinely disagree: the Blorb's asset
///   has detail in a region the native archive fills with a flat colour, and its
///   palette carries an extra entry to hold it. The other 96.3% match.
/// * **42** — 3 of 49 pixels of a 7×7 icon: white in the Blorb, blue natively.
/// * **50** — the Blorb keeps a 320×29 band of a 320×200 picture and paints 18
///   pixels the native archive marks transparent. Native wins, exactly as it
///   does for Zork Zero's ids 5–8.
#[test]
fn the_native_archive_reproduces_shoguns_blorb() {
    let (Some(pics), Some(blb)) = (native(), read("Shogun.blb")) else { return };
    let blorb = blorb::Blorb::parse(blb).expect("Shogun.blb parses");

    let (mut full, mut cropped) = (0usize, 0usize);
    let (mut differs, mut native_only, mut placeholders) = (Vec::new(), Vec::new(), Vec::new());
    for e in pics.entries() {
        if !e.has_pixels() {
            // A size-only record: the Blorb spells the same thing as a `Rect`.
            placeholders.push(e.id);
            assert!(
                matches!(blorb.resource(b"Pict", u32::from(e.id)), Some((ty, _)) if ty == b"Rect"),
                "native placeholder {} must be a Rect in the Blorb",
                e.id
            );
            continue;
        }
        let Some((ty, png)) = blorb.resource(b"Pict", u32::from(e.id)) else {
            native_only.push(e.id);
            continue;
        };
        assert_eq!(ty, b"PNG ", "picture {} carries pixels natively", e.id);

        let p = pics.decode(e.id).expect("a picture with pixels decodes");
        let img = app::cover::decode(png).expect("the Blorb PNG decodes").to_rgba8();
        let (bw, bh) = (img.width() as usize, img.height() as usize);
        let (nw, nh) = (usize::from(p.width), usize::from(p.height));
        assert!(bw <= nw && bh <= nh, "picture {} is bigger in the Blorb: {bw}x{bh} vs {nw}x{nh}", e.id);

        if matches_top_left(&p.rgba(), nw, &img.into_raw(), bw, bh) {
            if (bw, bh) == (nw, nh) {
                full += 1;
            } else {
                cropped += 1;
            }
        } else {
            differs.push(e.id);
        }
    }

    eprintln!("Shogun: {full} byte-exact, {cropped} byte-exact as a crop, {differs:?} differ");
    assert_eq!(placeholders, vec![2, 45, 46, 47, 48, 49], "the size-only records");
    assert_eq!(native_only, vec![4, 21, 35], "pictures only the floppy has");
    assert_eq!((full, cropped), (15, 19), "34 of the 39 shared pictures must be byte-exact");
    assert_eq!(
        differs,
        vec![16, 25, 37, 42, 50],
        "only the five catalogued disagreements may differ — see this test's doc comment"
    );
}

/// The whole point, where the player sees it: pointed at the floppy, Shogun
/// boots and paints its title screen.
///
/// Falsified by restoring the 14-byte-only check in `InfocomPics::parse`:
/// `all_pict_dims()` comes back empty and `pictures_canvas` is `[]` after boot —
/// the reported symptom exactly, no graphics at all.
#[test]
fn shogun_boots_off_its_floppy_and_draws_its_title_screen() {
    for honor_game_colours in [true, false] {
        let path = stories_dir().join(ADF);
        if read(ADF).is_none() {
            return;
        }
        let bytes = match app::hints::load_story(&path).expect("Story.data mounts") {
            app::hints::LoadedStory::ZCode(b) => b,
            other => panic!("expected Z-code off the floppy, got {other:?}"),
        };
        assert_eq!(bytes[0], 6, "Shogun is a v6 story");

        let mut picts = PictSource::resolve(&path, None);
        let dims = picts.all_pict_dims();
        assert_eq!(
            dims.len(),
            48,
            "honor_game_colours={honor_game_colours}: every directory record must reach the dimension table"
        );
        // The title screen the report is about, and the palette it draws through.
        let title = picts.image(1).expect("picture 1 decodes");
        assert_eq!((title.width(), title.height()), (320, 200));

        // `std_window` stands in for the Blorb `Reso` chunk a native archive has
        // not got — what the Amiga interpreter profile supplies at startup.
        let mut session = GameSession::new_with_trace(
            bytes,
            honor_game_colours,
            false,
            None,
            false,
            dims,
            Some((320, 200)),
            None,
            None,
        )
        .expect("Shogun (v6) boots without a ZError");
        session.set_pict_source(Some(picts));
        session.flush_boot_pictures();

        let canvas = session.pictures_canvas.get(&7).unwrap_or_else(|| {
            panic!("honor_game_colours={honor_game_colours}: Shogun must paint its boot canvas")
        });
        assert_eq!(
            (canvas.img.width(), canvas.img.height()),
            (640, 400),
            "honor_game_colours={honor_game_colours}: the title screen fills the v6 unit screen"
        );
        // Real art, not a blank fill — and every colour on it is one of the
        // title screen's own, so the pixels came out of the floppy's archive.
        let want: std::collections::HashSet<[u8; 3]> =
            title.to_rgba8().as_chunks::<4>().0.iter().map(|p| [p[0], p[1], p[2]]).collect();
        let got: std::collections::HashSet<[u8; 3]> =
            canvas.img.as_raw().as_chunks::<4>().0.iter().map(|p| [p[0], p[1], p[2]]).collect();
        assert!(
            got.len() >= 8,
            "honor_game_colours={honor_game_colours}: the boot canvas is a flat fill, not the title screen ({} colours)",
            got.len()
        );
        assert!(
            got.is_subset(&want),
            "honor_game_colours={honor_game_colours}: the canvas holds colours the title screen does not: {:?}",
            got.difference(&want).collect::<Vec<_>>()
        );
    }
}
