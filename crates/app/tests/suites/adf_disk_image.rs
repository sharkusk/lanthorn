//! SQ-0719: run a game straight off an Amiga `.adf` release floppy.
//!
//! Two levels. The first builds a disk image in-process — no fixture, no media
//! — and drives the real loading path over it: `hints::load_story` must mount
//! the filesystem and hand back the story, and `PictSource::resolve` must find
//! the `Pic.data` that shipped on the same disk (SQ-0734's tier 2, where the
//! medium itself guarantees the pairing).
//!
//! The second boots the original *Zork Zero* disk end to end, and skips
//! vacuously: those images are the user's own media, live outside the repo, and
//! will not exist for anyone else.

use app::graphics::PictSource;

// ── A disk image, built by hand ───────────────────────────────────────────────

const BSIZE: usize = 512;
/// Offset of the first data-block pointer; the table runs downwards from here.
const DATA_TABLE_TOP: usize = BSIZE - 204;
/// OFS data blocks reserve this many bytes for their own header.
const OFS_DATA_HEADER: usize = 24;
/// Blocks on an 880 KB DD floppy.
const DD_BLOCKS: usize = 1760;

/// Write an OFS 880 KB image holding `files`. Deliberately minimal — a single
/// data-block table per file, which is all the small files here need; the
/// extension-block chain and the FFS layout are exercised in `blorb`'s own
/// unit tests.
fn build_adf(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut image = vec![0u8; DD_BLOCKS * BSIZE];
    image[0..3].copy_from_slice(b"DOS");
    image[3] = 0; // OFS, as both Zork Zero disks are
    let mut next = 881; // first free block after the root
    let put32 = |img: &mut Vec<u8>, block: usize, off: usize, v: u32| {
        let at = block * BSIZE + off;
        img[at..at + 4].copy_from_slice(&v.to_be_bytes());
    };
    for (name, data) in files {
        let header = next;
        next += 1;
        put32(&mut image, header, 0, 2); // T_HEADER
        put32(&mut image, header, 4, header as u32); // names its own block
        put32(&mut image, header, BSIZE - 4, 0xFFFF_FFFD); // ST_FILE
        put32(&mut image, header, BSIZE - 188, data.len() as u32);
        let at = header * BSIZE + BSIZE - 80;
        image[at] = name.len() as u8;
        image[at + 1..at + 1 + name.len()].copy_from_slice(name.as_bytes());

        let chunks: Vec<&[u8]> = data.chunks(BSIZE - OFS_DATA_HEADER).collect();
        assert!(chunks.len() <= 72, "{name} needs an extension block");
        for (i, chunk) in chunks.iter().enumerate() {
            let db = next;
            next += 1;
            put32(&mut image, db, 0, 8); // T_DATA
            put32(&mut image, db, 4, header as u32);
            put32(&mut image, db, 8, i as u32 + 1);
            put32(&mut image, db, 12, chunk.len() as u32);
            let at = db * BSIZE + OFS_DATA_HEADER;
            image[at..at + chunk.len()].copy_from_slice(chunk);
            put32(&mut image, header, DATA_TABLE_TOP - 4 * i, db as u32);
        }
        put32(&mut image, header, 8, chunks.len() as u32);
    }
    image
}

/// A structurally valid v5 story image — enough header for the loader to
/// recognise it, which is all this test asks of it.
fn fake_story() -> Vec<u8> {
    let mut b = vec![0u8; 2048];
    b[0] = 5;
    let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
    word(0x04, 0x0400); // high memory
    word(0x08, 0x0300); // dictionary
    word(0x0a, 0x0100); // object table
    word(0x0c, 0x0200); // globals
    word(0x0e, 0x0280); // static memory base
    word(0x1a, 512); // file length, v5 unit of 4
    b[0x12..0x18].copy_from_slice(b"891231");
    b[1000] = 0xAB; // something for the round-trip to catch
    b
}

/// A one-picture Amiga archive: id 7, 4×2, rows of colour 2 then colour 3.
/// The bit-level derivation lives in `blorb::infocom_pics`' own tests; this is
/// the same bytes, so the app-side plumbing has something real to decode.
fn fake_pic_data() -> Vec<u8> {
    const ENTRY_SIZE: usize = 14;
    const HUFF_LEN: usize = 256;
    let mut f = vec![0u8; 16];
    f[0] = 1; // part number
    f[2] = 0x00; // Huffman-tree offset, in words
    f[3] = 0x0F;
    f[5] = 1; // one picture
    f[8] = ENTRY_SIZE as u8; // the Amiga/Mac record size
    let data_off = 16 + ENTRY_SIZE + HUFF_LEN;
    f.extend_from_slice(&[
        0, 7, // id
        0, 4, // width
        0, 2, // height
        0, 3, // EF_TRANS | EF_PHUFF
        (data_off >> 16) as u8,
        (data_off >> 8) as u8,
        data_off as u8,
        0, 0, 0, // no palette
    ]);
    let mut tree = vec![0u8; HUFF_LEN];
    tree[0] = 128 + 2; // leaf, colour 2
    tree[1] = 1; // internal node 1
    tree[2] = 128 + 1; // leaf, colour 1
    tree[3] = 128 + 18; // leaf, "repeat 3 more"
    f.extend_from_slice(&tree);
    f.extend_from_slice(&[0, 0, 1]); // minSize
    f.extend_from_slice(&[0, 0, 4]); // midSize: four symbols
    f.push(0b0111_0110);
    f
}

fn write_image(name: &str, image: &[u8]) -> std::path::PathBuf {
    let path = app::scratch_dir(&format!("adf-{name}")).join(format!("lanthorn-adf-{name}"));
    std::fs::write(&path, image).expect("write the disk image");
    path
}

// ── Synthetic disk: the whole loading path, no fixture ────────────────────────

/// The headline: pointing lanthorn at a disk image loads the game and its art
/// with no extraction step and nothing to configure.
#[test]
fn a_disk_image_yields_both_the_story_and_its_artwork() {
    let story = fake_story();
    let image = build_adf(&[
        ("Story.data", &story),
        ("Pic.data", &fake_pic_data()),
        // The clutter a real release disk carries, none of which is a story.
        ("Zork Zero", &[0x00, 0x00, 0x03, 0xf3, 0x00, 0x00, 0x00, 0x00]),
        ("Story.data.info", &[0xe3, 0x10, 0x00, 0x01]),
    ]);
    let path = write_image("full", &image);

    let loaded = app::hints::load_story(&path).expect("the story mounts out of the image");
    assert_eq!(loaded, app::hints::LoadedStory::ZCode(story), "byte-exact off the disk");

    let mut picts = PictSource::resolve(&path, None);
    assert_eq!(picts.all_pict_dims(), vec![(7, 4, 2)], "the disk's own Pic.data is the art");
    let img = picts.image(7).expect("picture 7 decodes");
    assert_eq!((img.width(), img.height()), (4, 2));

    // SQ-0838: an archive INSIDE the medium can also be named by hand. The
    // Macintosh is where that matters — its disk carries two archives and only
    // one of them is automatic — but the door is one piece of code and an Amiga
    // floppy gets it too, so it is checked on both media.
    let dir = std::env::temp_dir();
    let over = app::graphics::PictureOverride::resolve_with_session(&path, &dir, Some("Pic.data"));
    assert!(
        matches!(over, app::graphics::PictureOverride::Loaded { .. }),
        "a bare name absent from the filesystem is looked up on the volume, got {over:?}"
    );
    let mut named = PictSource::resolve_with_override(&path, over, None);
    assert_eq!(named.all_pict_dims(), vec![(7, 4, 2)]);
    let absent =
        app::graphics::PictureOverride::resolve_with_session(&path, &dir, Some("Nope.data"));
    assert!(matches!(absent, app::graphics::PictureOverride::Missing { .. }), "and only that name");

    let _ = std::fs::remove_file(&path);
}

/// A disk with no game on it — Zork Zero's Disk0 is exactly an AmigaDOS boot
/// floppy — must fail by saying so, not by feeding a system library to the VM.
#[test]
fn a_boot_disk_fails_with_a_useful_message() {
    let image = build_adf(&[
        ("startup-sequence", b"LoadWB\n"),
        ("icon.library", &[0x00, 0x00, 0x03, 0xf3, 0x11, 0x22]),
    ]);
    let path = write_image("boot", &image);

    let err = app::hints::load_story(&path).expect_err("a boot disk holds no story");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(err.to_string().contains("no story file on the disk image"), "{err}");

    let _ = std::fs::remove_file(&path);
}

/// An image with a story but no picture archive still boots; picture resolution
/// simply falls back to the Blorb tier, which finds nothing here.
#[test]
fn a_disk_without_artwork_still_loads_the_story() {
    let image = build_adf(&[("Story.data", &fake_story())]);
    let path = write_image("nopics", &image);

    assert!(app::hints::load_story(&path).is_ok());
    assert!(PictSource::resolve(&path, None).all_pict_dims().is_empty());

    let _ = std::fs::remove_file(&path);
}

// ── Real media: the original Zork Zero floppies ───────────────────────────────

/// The user's own disk images, outside the repo. `None` (with a SKIP note) when
/// they are not there, which is the normal case.
fn zork_zero_disk1() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let path = std::path::Path::new(&home)
        .join("Downloads/Zork Zero - The Revenge of Megaboz")
        .join("Zork Zero - The Revenge of Megaboz_Disk1.adf");
    if path.exists() {
        Some(path)
    } else {
        eprintln!("SKIP: original Amiga media absent at {}", path.display());
        None
    }
}

/// End to end on real media: boot Zork Zero out of the disk image the way the
/// app does, and confirm the artwork it draws during boot came off that same
/// disk. This is the acceptance criterion for the whole quest.
#[test]
fn zork_zero_boots_and_draws_from_its_release_floppy() {
    let Some(path) = zork_zero_disk1() else { return };

    let loaded = app::hints::load_story(&path).expect("Story.data mounts");
    let bytes = match loaded {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code, got {other:?}"),
    };
    assert_eq!(bytes[0], 6, "Zork Zero is a v6 story");
    assert_eq!(&bytes[0x12..0x18], b"890323");

    let mut picts = PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    assert_eq!(dims.len(), 495, "every directory record reaches the dimension table");
    let std_window = picts.std_window();

    let mut session = app::session::GameSession::new_with_trace(
        bytes, true, false, None, false, dims, std_window, None, None,
    )
    .expect("Zork Zero boots from the disk image");
    assert!(!session.quit, "quit during boot");
    assert!(session.machine.fault_trace.is_none(), "faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    // The boot sequence paints art; with the native archive wired in, that art
    // must be real pixels rather than an empty canvas.
    let painted: usize = session
        .pictures_canvas
        .values()
        .map(|c| c.img.as_raw().iter().filter(|b| **b != 0).count())
        .sum();
    assert!(painted > 0, "no artwork reached the canvas from the disk image");
}

/// The five pictures where the Amiga original and the circulating Blorb
/// disagree (SQ-0713): the disk holds full-size frames the Blorb keeps only a
/// cropped band of. Pinning them here is what proves the native archive — not a
/// Blorb sitting somewhere beside it — is the source being drawn from.
#[test]
fn the_disk_carries_the_full_size_frames_the_blorb_crops() {
    let Some(path) = zork_zero_disk1() else { return };
    let mut picts = PictSource::resolve(&path, None);
    for id in [5u32, 6, 7] {
        let img = picts.image(id).unwrap_or_else(|| panic!("picture {id} decodes"));
        assert_eq!(
            (img.width(), img.height()),
            (320, 200),
            "picture {id} is the full frame on the original media"
        );
    }
}
