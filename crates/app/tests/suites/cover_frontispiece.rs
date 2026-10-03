//! **The shelf does carry frontispieces — forty-four of them** (SQ-0985).
//!
//! The quest arrived believing the opposite. A scan run while assembling the
//! SQ-0979 cover-art corpus reported zero Blorb frontispieces, and concluded
//! that every cover the picker shows must be a fetched `cover.png` sidecar,
//! leaving `cover::load_cover`'s embedded-frontispiece branch never once
//! exercised against real content. That scan globbed `stories/*.blb`. The
//! twenty-one `.blb` files here are Infocom and Scott Adams art containers, and
//! it is quite true that not one declares an `Fspc` — but no modern Blorb is
//! spelled `.blb`. Re-run over every file in `stories/`, `treasures/` and
//! `masterpieces/`, the count is **44 blorbs carrying an `Fspc`** (re-measured SQ-1708 once the
//! walker stopped looping on a `stories/stories` self-link; the earlier 26 predates
//! corpus growth): all in
//! `stories/`, all `.gblorb`/`.zblorb`/`.blorb`, all naming Pict resource 1,
//! (at 26: nineteen JPEG and seven PNG). `treasures/` and `masterpieces/` carry none —
//! their `.adf`/`.dc42`/`.iso`/`.bin` media hold no Blorb at all, so the
//! picker's assumption that a disk-image row has no frontispiece to lose
//! (`StoryEntry::cover_key`) holds on this corpus.
//!
//! So the untested path was real and the fixtures for it were on disk the whole
//! time. That is what this suite does: sweep the corpus, and require every
//! blorb declaring an `Fspc` to decode to the picture that `Fspc` names.
//!
//! The Blorb spec's frontispiece chunk is four bytes of chunk id, a length of
//! 4, and "number of a Pict resource"; there may not be more than one, and the
//! image "may be of any legal Blorb type (except a placeholder rectangle)".
//! It says nothing about a chunk naming a resource that is not there.
//! `cover.rs` resolves that to no cover at all, silently — the only behaviour a
//! picker can want — and `cover.rs`'s own unit tests pin it on synthesised
//! containers, which is where a case for a container nobody ships belongs.
//!
//! All three directories are gitignored, so every case here skips vacuously.
//! [`shelf_declares_frontispieces`] is the guard that stops a *local* run
//! passing vacuously because the parse quietly stopped finding any.

use std::path::{Path, PathBuf};

fn corpus_dirs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    ["stories", "masterpieces", "treasures"].iter().map(|d| root.join(d)).collect()
}

// `files_under` is the shared loop-safe walker (SQ-1708): `treasures/` nests its
// discs a level down, and a symlink back to an ancestor must not recurse forever.
use crate::fixture_paths::files_under;

/// Every regular file in the three corpora, sorted.
fn corpus_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in corpus_dirs() {
        files_under(&dir, &mut out);
    }
    out.sort();
    out
}

/// The first 12 bytes of `path` — all [`blorb::Blorb::is_blorb`] reads, and the
/// difference between sniffing this corpus and reading 544 MB of it (a single
/// `.bin` disc image in `masterpieces/` is 338 MB).
fn head12(path: &Path) -> Option<[u8; 12]> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut b = [0u8; 12];
    f.read_exact(&mut b).ok()?;
    Some(b)
}

/// Every blorb in the corpus, with its bytes. Sniffed by header, so only the
/// containers that really are Blorbs are read in full.
fn shelf_blorbs() -> Vec<(PathBuf, Vec<u8>)> {
    corpus_files()
        .into_iter()
        .filter(|p| head12(p).is_some_and(|h| blorb::Blorb::is_blorb(&h)))
        .filter_map(|p| std::fs::read(&p).ok().map(|b| (p, b)))
        .collect()
}

/// Every blorb on the shelf that declares an `Fspc`, with the Pict number it
/// names — read through `blorb`'s own parser, which is the thing under test.
fn declared_frontispieces() -> Vec<(PathBuf, u32)> {
    shelf_blorbs()
        .into_iter()
        .filter_map(|(p, bytes)| {
            let b = blorb::Blorb::parse(bytes).ok()?;
            b.frontispiece().map(|n| (p, n))
        })
        .collect()
}

/// Non-vacuity guard for the rest of this suite: if any blorb on the shelf even
/// *contains* the four bytes `Fspc`, the parser must have found at least one
/// frontispiece. Without it, a regression that made `Blorb::frontispiece`
/// always answer `None` would empty the corpus every case below iterates and
/// turn the whole suite green — which is the exact failure mode this quest is
/// about.
///
/// Deliberately one-directional rather than an equality check: four such bytes
/// could fall inside a JPEG by chance on a shelf that keeps growing, and a case
/// that fails on the user's next download is worse than one that merely answers
/// "is the corpus here?" without asking the parser. Measured when this landed:
/// 44 blorbs contain the bytes, 44 declare an `Fspc`, the two sets identical.
#[test]
fn shelf_declares_frontispieces() {
    let raw: Vec<PathBuf> = shelf_blorbs()
        .into_iter()
        .filter(|(_, b)| b.windows(4).any(|w| w == b"Fspc"))
        .map(|(p, _)| p)
        .collect();
    let parsed = declared_frontispieces();
    assert!(
        raw.is_empty() || !parsed.is_empty(),
        "{} shelf blorb(s) contain the bytes `Fspc` (e.g. {}) but `Blorb::frontispiece` \
         found none — the Fspc parse has regressed",
        raw.len(),
        raw.first().map_or_else(|| "-".into(), |p| p.display().to_string()),
    );
}

/// Every `Fspc` on the shelf resolves to a decodable picture, and it is the
/// picture the chunk names — not merely *some* image out of the container.
///
/// `load_cover` resolves `Fspc` → `Pict` through `Blorb::resource`, which
/// matches on usage *and* number, and the second half of this case re-resolves
/// the named resource independently and compares. What that catches, measured:
/// a lookup that took the first resource of any usage picks a gblorb's `GLUL`
/// executable and fails here at once.
///
/// What it does **not** catch, and this shelf cannot: a lookup that kept the
/// `Pict` usage but ignored the number. All of these name resource 1 and 1
/// is the first picture in every one of their indexes, so a number-blind
/// lookup returns the right picture on every specimen. That gap is why the
/// dangling-`Fspc` cases in `cover.rs` are synthesised — a container whose
/// `Fspc` names a resource nobody indexed is not something anyone ships, and a
/// real-media sweep will never produce one.
#[test]
fn every_declared_frontispiece_decodes_to_the_pict_it_names() {
    for (path, number) in declared_frontispieces() {
        let img = app::cover::load_cover(&path, None).unwrap_or_else(|| {
            panic!(
                "{}: declares Fspc → Pict {number}, but load_cover produced no cover",
                path.display()
            )
        });
        assert!(
            img.width() > 0 && img.height() > 0,
            "{}: Fspc → Pict {number} decoded to an empty image",
            path.display()
        );

        let bytes = std::fs::read(&path).expect("re-read");
        let b = blorb::Blorb::parse(bytes).expect("re-parse");
        let (_ty, data) = b.resource(b"Pict", number).unwrap_or_else(|| {
            panic!("{}: Fspc names Pict {number}, absent from the index", path.display())
        });
        let named = app::cover::decode(data)
            .unwrap_or_else(|| panic!("{}: Pict {number} does not decode", path.display()));
        assert_eq!(
            (img.width(), img.height()),
            (named.width(), named.height()),
            "{}: load_cover returned a different picture than Fspc's Pict {number}",
            path.display()
        );
    }
}

/// A story's own frontispiece outranks a fetched `cover.png` sidecar — on real
/// content, not a synthesised container.
///
/// `cover.rs` pins this precedence on a hand-built blorb; what it cannot show
/// is that a real `.gblorb` off the shelf reaches the same branch. The sidecar
/// written here is 2x2 and no cover on the shelf is that small, so the returned
/// dimensions alone say which source won.
#[test]
fn a_real_frontispiece_outranks_a_fetched_cover_png() {
    let declared = declared_frontispieces();
    let Some((path, number)) = declared.first() else { return }; // shelf absent

    let own = app::cover::load_cover(path, None).expect("shelf frontispiece decodes");
    assert!(
        own.width() > 2 && own.height() > 2,
        "{}: this case distinguishes by size, and Pict {number} is 2x2 or smaller",
        path.display()
    );

    let game_dir =
        std::env::temp_dir().join(format!("lanthorn-fspc-precedence-{}", std::process::id()));
    std::fs::create_dir_all(&game_dir).unwrap();
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3])))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(game_dir.join("cover.png"), &png).unwrap();

    let got = app::cover::load_cover(path, Some(&game_dir)).expect("a cover either way");
    let _ = std::fs::remove_dir_all(&game_dir);

    assert_eq!(
        (got.width(), got.height()),
        (own.width(), own.height()),
        "{}: the fetched cover.png beat the story's own Fspc → Pict {number}",
        path.display()
    );
}

/// The `treasures/` and `masterpieces/` media carry no Blorb at all, which is
/// what lets `StoryEntry::cover_key` key a disk-image row by its game directory
/// — "a disk image is never a blorb — there is no frontispiece in there to
/// lose". Pinned because that comment is a claim about a corpus, and a corpus
/// is exactly the sort of thing that changes without anyone editing a comment.
#[test]
fn disk_media_carry_no_frontispiece() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in ["masterpieces", "treasures"] {
        let mut paths = Vec::new();
        files_under(&root.join(dir), &mut paths);
        for path in paths {
            assert!(
                !head12(&path).is_some_and(|h| blorb::Blorb::is_blorb(&h)),
                "{}: a Blorb turned up in {dir}/ — re-check cover_key's assumption \
                 that a disk-image row has no embedded cover",
                path.display()
            );
        }
    }
}
