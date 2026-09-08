//! SQ-0837: run a game straight off a Macintosh release floppy — a DiskCopy 4.2
//! `.image` wrapped around an HFS volume.
//!
//! Same shape as `adf_disk_image.rs`, one machine over. The first half builds a
//! disk image in-process — no fixture, no media — and drives the real loading
//! path over it: `hints::load_story` must mount the filesystem and hand back the
//! story, and `PictSource::resolve` must find the picture archive that shipped
//! on the same disk (SQ-0734's tier 2, where the medium itself guarantees the
//! pairing).
//!
//! The second half boots the user's own *Zork Zero Disk.image* end to end, and
//! skips vacuously: it lives outside the repo and will not exist for anyone
//! else.
//!
//! **The release is the point.** That disk carries Zork Zero **release 296,
//! serial 881019** — October 1988, where every other Zork Zero in the corpus is
//! r393/890714 or the Amiga's r366/890323. Per this project's standing rule a
//! disk image is a different build rather than the same story on other media, so
//! nothing measured here transfers to the others and nothing measured there
//! transfers here.
//!
//! (Its release pin lives in `real_media_releases.rs` beside every other
//! medium's; this suite is about the container.)

use app::graphics::PictSource;

// ── A disk image, built by hand ───────────────────────────────────────────────

const BLOCK: usize = 512;
/// 1600 blocks, which is what an 800 KB Macintosh floppy holds.
const VOLUME_BLOCKS: usize = 1600;
/// First allocation block, in 512-byte blocks from the volume's start.
const ALLOC_START: usize = 4;
/// The DiskCopy 4.2 header this builder writes.
const DISKCOPY_HEADER: usize = 84;

fn be32(v: usize) -> [u8; 4] {
    (v as u32).to_be_bytes()
}

fn be16(v: usize) -> [u8; 2] {
    (v as u16).to_be_bytes()
}

/// Write an HFS volume holding `files`, wrapped the way DiskCopy 4.2 does —
/// header, volume, then 12 bytes of sector tag per block, which are NOT part of
/// the filesystem and which a reader that folds them in will trip over.
///
/// Deliberately minimal: 512-byte allocation blocks, one contiguous extent per
/// file, and a catalog of one header node plus one leaf. The extents overflow
/// file and the B*-tree edge cases are exercised in `blorb`'s own unit tests.
fn build_mac_image(files: &[(&str, &[u8; 4], &[u8])]) -> Vec<u8> {
    let mut volume = vec![0u8; VOLUME_BLOCKS * BLOCK];
    // Allocation blocks 0..3 are the (empty) extents overflow file, 3..6 the
    // catalog; file data starts after them.
    let mut next = 6usize;
    let mut records: Vec<Vec<u8>> = Vec::new();

    for (i, (name, ftype, data)) in files.iter().enumerate() {
        let blocks = data.len().div_ceil(BLOCK);
        let start = next;
        next += blocks.max(1);
        for b in 0..blocks {
            let src = b * BLOCK;
            let end = data.len().min(src + BLOCK);
            let at = (ALLOC_START + start + b) * BLOCK;
            volume[at..at + (end - src)].copy_from_slice(&data[src..end]);
        }

        // Catalog key: length, reserved, parent id, Pascal name — then the file
        // record at the next even offset.
        let mut rec = vec![0u8; 7 + name.len()];
        rec[0] = (rec.len() - 1) as u8;
        rec[2..6].copy_from_slice(&be32(2)); // the root directory
        rec[6] = name.len() as u8;
        rec[7..].copy_from_slice(name.as_bytes());
        if !rec.len().is_multiple_of(2) {
            rec.push(0);
        }
        let mut d = vec![0u8; 102];
        d[0] = 2; // cdrFilRec
        d[4..8].copy_from_slice(*ftype);
        d[8..12].copy_from_slice(b"IN0Z");
        d[20..24].copy_from_slice(&be32(16 + i)); // filFlNum
        d[26..30].copy_from_slice(&be32(data.len())); // filLgLen
        d[74..76].copy_from_slice(&be16(start)); // filExtRec[0]
        d[76..78].copy_from_slice(&be16(blocks));
        rec.extend_from_slice(&d);
        records.push(rec);
    }

    // The catalog: node 0 is the header (its first record names the first leaf),
    // node 1 is that leaf.
    let cat = (ALLOC_START + 3) * BLOCK;
    volume[cat + 8] = 1; // ndType: header
    volume[cat + 10..cat + 12].copy_from_slice(&be16(1));
    volume[cat + BLOCK - 2..cat + BLOCK].copy_from_slice(&be16(14));
    volume[cat + BLOCK - 4..cat + BLOCK - 2].copy_from_slice(&be16(120));
    volume[cat + 14 + 10..cat + 14 + 14].copy_from_slice(&be32(1)); // bthFNode
    let leaf = cat + BLOCK;
    volume[leaf + 8] = 0xFF; // ndType: leaf
    volume[leaf + 10..leaf + 12].copy_from_slice(&be16(records.len()));
    let mut at = 14usize;
    for (i, r) in records.iter().enumerate() {
        volume[leaf + BLOCK - 2 * (i + 1)..leaf + BLOCK - 2 * i].copy_from_slice(&be16(at));
        volume[leaf + at..leaf + at + r.len()].copy_from_slice(r);
        at += r.len();
    }
    let n = records.len();
    volume[leaf + BLOCK - 2 * (n + 1)..leaf + BLOCK - 2 * n].copy_from_slice(&be16(at));

    // The Master Directory Block, two blocks in.
    let mdb = 2 * BLOCK;
    volume[mdb..mdb + 2].copy_from_slice(&be16(0x4244)); // 'BD'
    volume[mdb + 18..mdb + 20].copy_from_slice(&be16(VOLUME_BLOCKS - ALLOC_START));
    volume[mdb + 20..mdb + 24].copy_from_slice(&be32(BLOCK));
    volume[mdb + 28..mdb + 30].copy_from_slice(&be16(ALLOC_START));
    let vname = b"Test Disk";
    volume[mdb + 36] = vname.len() as u8;
    volume[mdb + 37..mdb + 37 + vname.len()].copy_from_slice(vname);
    volume[mdb + 130..mdb + 134].copy_from_slice(&be32(3 * BLOCK)); // drXTFlSize
    volume[mdb + 134..mdb + 136].copy_from_slice(&be16(0));
    volume[mdb + 136..mdb + 138].copy_from_slice(&be16(3));
    volume[mdb + 146..mdb + 150].copy_from_slice(&be32(3 * BLOCK)); // drCTFlSize
    volume[mdb + 150..mdb + 152].copy_from_slice(&be16(3));
    volume[mdb + 152..mdb + 154].copy_from_slice(&be16(3));

    // The DiskCopy 4.2 wrapper.
    let tags = 12 * VOLUME_BLOCKS;
    let mut out = vec![0u8; DISKCOPY_HEADER];
    out[0] = vname.len() as u8;
    out[1..1 + vname.len()].copy_from_slice(vname);
    out[0x40..0x44].copy_from_slice(&be32(volume.len()));
    out[0x44..0x48].copy_from_slice(&be32(tags));
    out[0x50] = 0x01;
    out[0x51] = 0x22;
    out[0x52..0x54].copy_from_slice(&be16(0x0100));
    out.extend_from_slice(&volume);
    out.extend(std::iter::repeat_n(0xA5u8, tags));
    out
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
    b[0x12..0x18].copy_from_slice(b"881019");
    b[1000] = 0xAB; // something for the round-trip to catch
    b
}

/// A one-picture big-endian archive: id 7, 4×2. The same bytes
/// `adf_disk_image.rs` uses — the Macintosh and the Amiga wrote the *same*
/// container, which is exactly why one decoder serves both.
fn fake_pic_data() -> Vec<u8> {
    const ENTRY_SIZE: usize = 14;
    const HUFF_LEN: usize = 256;
    let mut f = vec![0u8; 16];
    f[0] = 1; // part number
    f[2] = 0x00; // Huffman-tree offset, in words
    f[3] = 0x0F;
    f[5] = 1; // one picture
    f[8] = ENTRY_SIZE as u8;
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
    let path = app::scratch_dir(&format!("hfs-{name}")).join(format!("lanthorn-hfs-{name}"));
    std::fs::write(&path, image).expect("write the disk image");
    path
}

// ── Synthetic disk: the whole loading path, no fixture ────────────────────────

/// The headline: pointing lanthorn at a Macintosh disk image loads the game and
/// its art with no extraction step and nothing to configure.
///
/// FALSIFY by dropping the `Hfs` arm from `hints::read_story_file`: the image is
/// then handed to the VM whole and the story never appears.
#[test]
fn a_macintosh_disk_image_yields_both_the_story_and_its_artwork() {
    let story = fake_story();
    let image = build_mac_image(&[
        ("Story.data", b"INdf", &story),
        ("CPic.data", b"INdf", &fake_pic_data()),
        // The clutter a real release disk carries, none of which is a story.
        ("Zork Zero", b"APPL", &[0x4a, 0x6f, 0x79, 0x21]),
        ("Desktop", b"FNDR", &[0x00, 0x00, 0x01, 0x00]),
    ]);
    let path = write_image("full", &image);

    let loaded = app::hints::load_story(&path).expect("the story mounts out of the image");
    assert_eq!(loaded, app::hints::LoadedStory::ZCode(story), "byte-exact off the disk");

    let mut picts = PictSource::resolve(&path, None);
    assert_eq!(picts.all_pict_dims(), vec![(7, 4, 2)], "the disk's own archive is the art");
    let img = picts.image(7).expect("picture 7 decodes");
    assert_eq!((img.width(), img.height()), (4, 2));

    let _ = std::fs::remove_file(&path);
}

/// The mount says WHICH machine's media this is, so the story list can name the
/// container and the launch dialog can decline to infer an Amiga from it.
#[test]
fn the_mount_names_the_container_it_came_out_of() {
    let path = write_image(
        "container",
        &build_mac_image(&[("Story.data", b"INdf", &fake_story())]),
    );
    let (_, image) = app::hints::load_mounted_story(&path).expect("mounts");
    assert_eq!(image, Some(app::hints::DiskImage::Hfs));
    assert_eq!(image.map(|i| i.label()), Some("HFS"));

    // …and the launch dialog reports the MACHINE, which since SQ-0838 is a
    // Macintosh: interpreter 3 (ZMSD §11.1.3), sourced from the disk rather than
    // from the default rule. It is emphatically not 4 — the Amiga and the
    // Macintosh wrote the same colour archive, and only the volume tells them
    // apart.
    assert_eq!(
        app::launch_options::derived_interpreter(None, None, image, Some(6)),
        Some((3, app::launch_options::InterpreterSource::DiskImage)),
        "an HFS volume is a Macintosh, and never an Amiga"
    );
    assert_eq!(
        app::interpreter::InterpreterProfile::resolve(&path, None, None, None),
        app::interpreter::InterpreterProfile::Macintosh,
        "and the boot path agrees with the dialog"
    );

    let _ = std::fs::remove_file(&path);
}

/// The 12 tag bytes per block sit AFTER the volume and are not part of it. A
/// reader that counts them in shifts every allocation block; one that misplaces
/// the 84-byte header shifts everything by 84. Either mistake loses the story.
#[test]
fn the_diskcopy_wrapper_is_unwrapped_by_its_own_declared_length() {
    let image = build_mac_image(&[("Story.data", b"INdf", &fake_story())]);
    assert_eq!(image.len(), 84 + VOLUME_BLOCKS * BLOCK + 12 * VOLUME_BLOCKS);
    let path = write_image("tags", &image);
    assert!(app::hints::load_story(&path).is_ok(), "the wrapper is stripped, the tags ignored");

    // The bare volume, with no wrapper at all, reads exactly the same.
    let bare = write_image("bare", &image[84..84 + VOLUME_BLOCKS * BLOCK]);
    assert_eq!(app::hints::load_story(&bare).ok(), app::hints::load_story(&path).ok());

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&bare);
}

/// A disk with no game on it must fail by saying so, not by feeding the Finder's
/// desktop database to the VM.
#[test]
fn a_disk_with_no_game_fails_with_a_useful_message() {
    let image = build_mac_image(&[
        ("Desktop", b"FNDR", &[0x00, 0x00, 0x01, 0x00]),
        ("System", b"ZSYS", &[0x00, 0x01, 0x02, 0x03]),
    ]);
    let path = write_image("empty", &image);

    let err = app::hints::load_story(&path).expect_err("this disk holds no story");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(err.to_string().contains("no story file on the disk image"), "{err}");
    assert!(err.to_string().contains("Test Disk"), "the volume names itself: {err}");

    let _ = std::fs::remove_file(&path);
}

/// An image with a story but no picture archive still boots; picture resolution
/// simply falls back to the Blorb tier, which finds nothing here.
#[test]
fn a_disk_without_artwork_still_loads_the_story() {
    let path = write_image("nopics", &build_mac_image(&[("Story.data", b"INdf", &fake_story())]));

    assert!(app::hints::load_story(&path).is_ok());
    assert!(PictSource::resolve(&path, None).all_pict_dims().is_empty());

    let _ = std::fs::remove_file(&path);
}

// ── Real media: the original Macintosh Zork Zero disk ─────────────────────────

const FIXTURE: &str = "Zork Zero Disk.image";
/// The build this disk carries, and it is nobody else's.
const RELEASE: u16 = 296;
const SERIAL: &[u8] = b"881019";

/// The user's own disk image, outside the repo. `None` (with a SKIP note) when
/// it is not there, which is the normal case.
fn mac_disk() -> Option<std::path::PathBuf> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(FIXTURE);
    if path.exists() {
        Some(path)
    } else {
        eprintln!("SKIP: gitignored Macintosh medium missing at {}", path.display());
        None
    }
}

/// End to end on real media: boot Zork Zero out of the Macintosh disk the way
/// the app does. This is the acceptance criterion for the whole quest.
#[test]
fn zork_zero_boots_from_its_macintosh_release_floppy() {
    let Some(path) = mac_disk() else { return };

    let (loaded, image) = app::hints::load_mounted_story(&path).expect("Story.data mounts");
    assert_eq!(image, Some(app::hints::DiskImage::Hfs));
    let bytes = match loaded {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code, got {other:?}"),
    };
    assert_eq!(bytes[0], 6, "Zork Zero is a v6 story");
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        RELEASE,
        "this medium carries a DIFFERENT build than release {RELEASE}"
    );
    assert_eq!(&bytes[0x12..0x18], SERIAL);
    assert_eq!(bytes.len(), 295_936);

    let profile = app::interpreter::InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    // The same chain `startup.rs` runs: the Blorb's `Reso` (there is none), the
    // machine's own answer (the IBM PC bundle has none), then the archive the
    // medium supplied.
    let std_window = picts
        .std_window()
        .or_else(|| profile.std_window())
        .or_else(|| picts.native_std_window());
    assert_eq!(std_window, Some((320, 200)), "the disk's own archive says how big its art is");
    let mut session = app::session::GameSession::new_with_trace(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        dims,
        std_window,
        profile.default_colours(),
        None,
    )
    .expect("Zork Zero boots from the disk image");
    assert!(!session.quit, "quit during boot");
    assert!(session.machine.fault_trace.is_none(), "faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    // …and it takes input. A game that boots and then cannot be played is not
    // "opens and plays".
    for _ in 0..12 {
        match session.pending_input() {
            app::session::InputKind::Line => {
                let _ = session.submit("");
            }
            app::session::InputKind::Char => {
                let _ = session.submit_char(13);
            }
            app::session::InputKind::Event => {
                let _ = session.submit("");
            }
        }
        assert!(!session.quit, "quit while being driven");
        assert!(session.machine.fault_trace.is_none(), "faulted while being driven");
    }
}

/// The disk's own artwork, and the answer to "does the Macintosh release carry
/// art at all" — it carries **two** archives, one per screen Apple sold.
///
/// Both read now (SQ-0838), and the automatic choice is the COLOUR one. That is
/// a preference and not a parse failure, which is the whole difference this
/// quest made: the monochrome archive declares 12-byte directory records and
/// header flags `0x0e`, and used to be refused as a container variant
/// `InfocomPics` did not know. A Macintosh interpreter PROFILE is still separate
/// work — this suite pins what is on the disk, not what a Macintosh screen
/// should look like.
#[test]
fn the_macintosh_disk_carries_two_picture_archives_and_the_colour_one_is_the_art() {
    let Some(path) = mac_disk() else { return };
    let hfs = blorb::hfs::Hfs::mount(std::fs::read(&path).expect("read")).expect("mounts");

    let listing: Vec<(&str, usize, usize)> =
        hfs.files().iter().map(|e| (e.name.as_str(), e.size, e.resource_size)).collect();
    assert_eq!(
        listing,
        vec![
            ("CPic.data", 218_624, 0),
            ("Desktop", 0, 1_665),
            ("Pic.data", 239_104, 0),
            ("Story.data", 295_936, 0),
            ("Zork Zero", 0, 38_833),
        ],
        "the whole volume: a story, two picture archives, Infocom's own interpreter, the desktop"
    );

    let mut picts = PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    assert_eq!(dims.len(), 483, "every directory record reaches the dimension table");
    let img = picts.image(1).expect("picture 1 decodes straight off the disk");
    assert_eq!((img.width(), img.height()), (320, 200));

    // …and it lands on the screen at the size the game lays out for, rather than
    // at its own 320×200 on a 640×400 screen — the SQ-0736 1× symptom, which a
    // medium with no interpreter profile of its own would otherwise walk into.
    assert_eq!(picts.native_std_window(), Some((320, 200)));
    assert_eq!(picts.art_scale(), Some((2, 2)), "a 320-wide picture space doubles");
    assert!(!picts.is_monochrome(), "and it is the colour archive, not the mono one");
}

/// SQ-0838: naming the monochrome archive by hand draws the monochrome artwork.
///
/// The colour archive is the disk's default and stays it; this is the door, not
/// a change of policy. `--pictures Pic.data` is the same door `--pictures`
/// already was for a loose `.MG1` beside a story — the only new thing is that
/// the name is looked up ON THE VOLUME, because a story mounted out of a disk
/// image has no directory for a loose file to sit in and the archive the user
/// wants is already there.
///
/// What comes back is a different SCREEN, not a recoloured copy of the same one:
/// 480×300 where the colour archive is 320×200, which is the picture space
/// `mac/gfx.p` names in `GF_MONO`'s own definition ("scaled for a 480x300 screen
/// (std Mac)"). So it does not double onto the 640×400 unit screen the way every
/// other rendition does — it lands 1:1, which is also how Infocom's own
/// interpreter displayed it (`IF ge.mono OR myTiny THEN { scale 1x for display }`).
#[test]
fn naming_the_monochrome_archive_by_hand_draws_the_monochrome_artwork() {
    let Some(path) = mac_disk() else { return };
    let dir = std::env::temp_dir().join("lanthorn-mac-mono-override");
    let _ = std::fs::create_dir_all(&dir);

    let over = app::graphics::PictureOverride::resolve_with_session(&path, &dir, Some("Pic.data"));
    assert!(
        matches!(over, app::graphics::PictureOverride::Loaded { .. }),
        "the name is looked up on the volume, got {over:?}"
    );
    assert_eq!(over.warning(), None, "a name that loads is not a complaint");
    assert_eq!(over.flavour(), Some(blorb::infocom_pics::Flavour::AmigaMac));

    let mut picts = app::graphics::PictSource::resolve_with_override(&path, over, None);
    assert!(picts.is_monochrome(), "the named archive is the two-colour one");
    assert_eq!(picts.all_pict_dims().len(), 483, "the same catalogue as the colour archive");

    let img = picts.image(1).expect("picture 1 decodes off the volume");
    assert_eq!((img.width(), img.height()), (480, 300), "the standard Macintosh screen");
    assert_eq!(picts.art_scale(), Some((1, 1)), "a 480×300 space does NOT double");

    // Two colours and nothing between them, drawn through the archive's own
    // hardware table rather than through `DEFAULT_PALETTE` (which would make
    // colour 2 green and colour 3 cyan).
    let mut shades: Vec<[u8; 3]> = img
        .to_rgba8()
        .pixels()
        .filter(|p| p.0[3] != 0)
        .map(|p| [p.0[0], p.0[1], p.0[2]])
        .collect();
    shades.sort_unstable();
    shades.dedup();
    assert_eq!(shades, vec![[0, 0, 0], [255, 255, 255]], "black and white, and nothing else");

    // A name that is on neither the volume nor the filesystem is still loud.
    let missing =
        app::graphics::PictureOverride::resolve_with_session(&path, &dir, Some("NotHere.data"));
    assert!(matches!(missing, app::graphics::PictureOverride::Missing { .. }));
    assert!(missing.warning().is_some());

    let _ = std::fs::remove_dir_all(&dir);
}
