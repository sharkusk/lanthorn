//! Infocom's **raw self-booting Apple II 5.25-inch disks** — a story on a floppy
//! with no filesystem under it at all (SQ-0868).
//!
//! # What it is, and how it differs from the `.dsk` beside it
//!
//! `stories/Planetfall r29 (clean copy from retail disk).dsk` and
//! `stories/shogun_s1.dsk` are both 143,360 bytes, both are 35 tracks of 16
//! 256-byte sectors in the order the drive numbers them, and both are Apple II
//! 5.25-inch dumps of an Infocom release. There the resemblance stops. The
//! Shogun disk is an ordinary **ProDOS volume** whose sectors are shuffled —
//! de-interleave it and block 2 is a volume directory naming `SHOGUN.D1`, and
//! [`crate::prodos`] reads it like any other volume (SQ-0864).
//!
//! This one has no volume directory anywhere, in any order, and no DOS 3.3 VTOC
//! either. Its boot sector opens
//!
//! ```text
//!   01 A5 27 C9 09 D0 16 A5 2B 85 60 85 61 4A 4A 4A
//! ```
//!
//! which is the Apple II boot ROM's own RWTS entry sequence, not ProDOS 8's
//! `01 38 B0 03 4C`. Track 17 sector 0 — where DOS 3.3 keeps its VTOC — reads
//! release 0, 26 tracks, 99 sectors per track: noise, not a VTOC. There is
//! nothing on this disk to enumerate. Infocom's loader boots, and then reads the
//! story off known tracks with its own RWTS, which is what "self-booting" means.
//!
//! # Where the story is, and how far it runs
//!
//! **Measured, not recalled**, because there is no filesystem to ask. Putting
//! the sectors into DOS 3.3 **logical** order ([`crate::dos_order::logical_order`])
//! and testing every 256-byte boundary in the image, exactly one offset holds a
//! Version 3 header: **12,288 — track 3, sector 0** — release 29, serial
//! 840118, declaring 54,526 length units.
//!
//! The story is **contiguous from there in logical order**, and its extent is
//! the header's own `$1A` length field times the version's packed-address scale
//! (ZMSD §11.1.6): 54,526 × 2 = **109,052 bytes**, ending inside track 29. The
//! disk's remaining 22,020 bytes are the loader, the two unused tracks before
//! the story, and slack. Nothing about the layout is hard-coded here — see
//! "the sniff", below.
//!
//! # The checksum is the oracle, and it is not the whole oracle
//!
//! ZMSD §11.1.6's header checksum at `$1C` reads `$842E`, and the sum over
//! `$40..109052` matches it under the logical order and under no other:
//!
//! ```text
//!   physical order        $529D
//!   ProDOS block order    $97D5   (crate::dos_order::prodos_order)
//!   DOS 3.3 logical       $842E   ← the header's own value
//! ```
//!
//! That is decisive about *which sectors* the story is made of, and it is worth
//! being explicit that it is **blind to their order**: the checksum is a byte
//! sum, so it cannot tell a correctly ordered story from an anagram of one. The
//! orders above differ in their sums only because the story ends part-way
//! through a track and each order puts different sectors in that tail.
//!
//! So the order was settled by a second, structural reading — decoding the
//! dictionary the header points at. Under the logical order it is a textbook
//! Version 3 dictionary: three word separators, `, . "`, seven-byte entries,
//! 665 of them. Under the other two the same pointer lands on 100 or 204
//! separators and entries 148 and 210 bytes long. And the third reading is the
//! one the user makes: the game boots and plays, which is what the quest asked
//! for and what no anagram would do.
//!
//! # The sniff, and why it names no offset
//!
//! [`InfocomBoot::looks_like_boot_disk`] does not look for `01 A5 27`, and does
//! not look at track 3. It de-interleaves and asks whether any sector boundary
//! starts a story whose **own header checksum verifies over its own declared
//! length** ([`crate::infocom_packed::verified`], shared rather than copied).
//!
//! That is deliberate on both counts. A boot-code signature is one loader out of
//! however many Infocom pressed across ten years of Apple II releases, and
//! fitting the sniff to the single disk in the corpus would be fitting it to a
//! sample of one. A fixed track would be worse still, and neither is *evidence*:
//! a verified story is far stronger than any signature, because it cannot be
//! passed by accident. Testing every boundary on all fifteen 143,360-byte images
//! in `stories/` yields exactly one hit — this disk, at 12,288 — and zero on all
//! fourteen ProDOS volumes.
//!
//! # Why it is a format and the ProDOS de-interleave was not
//!
//! [`crate::dos_order`]'s header says it "is not a new disk format" because what
//! comes out the other side is ProDOS and answers to the ProDOS row. Nothing
//! comes out the other side here. There is no volume, no directory, no file, no
//! name — so there is no existing row that could describe it, and
//! [`crate::medium`] gains one.

use crate::adf::looks_like_story;

/// Bytes in a sector; the granularity the loader reads and therefore the only
/// alignment a story can start on.
const SECTOR: usize = 256;

/// Sectors per track, for naming the place a story was found.
const SECTORS: usize = 16;

/// Errors that can arise while mounting a raw self-booting disk.
#[derive(Debug, PartialEq, Eq)]
pub enum InfocomBootError {
    /// Not a 5.25-inch dump carrying a story that verifies against its own
    /// header checksum.
    NotABootDisk,
}

/// A mounted raw self-booting Infocom disk.
///
/// It holds one story and nothing else, because one story and nothing else is
/// all such a disk can be shown to hold. There is no directory to enumerate, so
/// the artwork question does not arise either — see [`InfocomBoot::pictures`].
#[derive(Debug)]
pub struct InfocomBoot {
    /// Where the story starts, as a (track, sector) of the de-interleaved image.
    /// Kept because it is the only name this medium can give a file; see
    /// [`InfocomBoot::story`].
    at: (usize, usize),
    /// The story, truncated to its declared length and checksum-verified.
    story: Vec<u8>,
}

impl InfocomBoot {
    /// Cheap sniff: is this a raw self-booting Infocom disk?
    ///
    /// "Cheap" is relative — it de-interleaves 140 KB and walks its sector
    /// boundaries — but the walk copies nothing until a boundary passes the
    /// structural [`looks_like_story`] test, which no boundary on an ordinary
    /// image does. See the module header for why the sniff is a verified story
    /// rather than a boot signature.
    ///
    /// **Disjoint from ProDOS by construction, not by luck.** A `.dsk` that is a
    /// ProDOS volume is declined here outright, so no order of
    /// [`crate::medium::FORMATS`] can change which row claims an image and
    /// `DiskImage::detect`'s promise that table order is "a formality rather
    /// than a precedence" survives a second format wearing the same spelling,
    /// the same size and the same sector order.
    pub fn looks_like_boot_disk(raw: &[u8]) -> bool {
        if crate::prodos::ProDos::looks_like_prodos(raw) || story_as_it_lies(raw) {
            return false;
        }
        story_on(raw).is_some()
    }

    /// Open the disk, or `Err` when it carries no verified story.
    ///
    /// The sniff's own work, kept — recognising and opening are the same
    /// question here, so [`Self::looks_like_boot_disk`] cannot claim bytes this
    /// would refuse.
    pub fn mount(raw: Vec<u8>) -> Result<InfocomBoot, InfocomBootError> {
        if crate::prodos::ProDos::looks_like_prodos(&raw) || story_as_it_lies(&raw) {
            return Err(InfocomBootError::NotABootDisk);
        }
        let (at, story) = story_on(&raw).ok_or(InfocomBootError::NotABootDisk)?;
        Ok(InfocomBoot { at, story })
    }

    /// How the story is named in a listing: `T3/S0`, the track and sector it
    /// starts at once the sectors are in DOS 3.3 logical order.
    ///
    /// A disk with no filesystem has no filename to report, and this seam has to
    /// report something — [`crate::medium::DiskStory`] carries a `String`, and
    /// `--pictures`-style lookup needs a name a caller was shown to be a name it
    /// can ask back for. Where the story is, is the only thing this medium
    /// actually knows about it, so that is what it says. It is a label and never
    /// an identifier, which is what that type's own docs already require of every
    /// format's names.
    pub fn entry_name(&self) -> String {
        let (track, sector) = self.at;
        format!("T{track}/S{sector}")
    }

    /// The story on the disk, with the name [`Self::entry_name`] gives it.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        Some((self.entry_name(), self.story.clone()))
    }

    /// Everything the disk can be shown to hold: the story, and nothing else.
    ///
    /// The loader and its RWTS are on here too, and are not files — they are 6502
    /// the boot ROM jumps into. Reporting them as contents would be inventing a
    /// directory this medium does not have.
    pub fn contents(&self) -> Vec<(String, Vec<u8>)> {
        vec![(self.entry_name(), self.story.clone())]
    }

    /// One entry by the name a caller was shown, case-insensitively.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        name.eq_ignore_ascii_case(&self.entry_name()).then(|| self.story.clone())
    }

    /// How many entries the mount found: always one.
    pub fn file_count(&self) -> usize {
        1
    }

    /// **No artwork, and that is a limit rather than a finding.**
    ///
    /// The one disk of this kind in the corpus is *Planetfall*, a Version 3 game
    /// with no pictures to carry, so there is nothing here to test a picture
    /// search against and no evidence about where a raw disk would keep an
    /// archive. Scanning for one would be a guess with no medium behind it,
    /// which is the rule the rest of this crate applies to every spelling and
    /// geometry it declines. It arrives with a disk that has art on it, or not at
    /// all.
    pub fn pictures(&self) -> Option<(String, crate::infocom_pics::InfocomPics)> {
        None
    }
}

/// The verified story on `raw`, with the (track, sector) it starts at, or `None`.
///
/// The whole reader, in one function: de-interleave into DOS 3.3 logical order,
/// then take the first sector boundary that starts a story whose own header
/// checksum agrees with its own declared length.
/// **A story file is not a disk, however long it is.** A self-booting disk's
/// first sector is the boot ROM's loader (the module header quotes it), never a
/// story header; bytes that are already a whole verified story in physical
/// order from offset 0 are a story file that happens to be one floppy long.
/// Without this, such a file de-interleaves into a sector-anagram of itself,
/// its header still sits at track 0 sector 0, and the byte-sum checksum, blind
/// to order, passes it: `SpAdventure.z5` off the IF Archive is exactly 143,360
/// bytes and was being listed, and would have been opened, as a boot disk.
fn story_as_it_lies(raw: &[u8]) -> bool {
    looks_like_story(raw) && crate::infocom_packed::verified(raw.to_vec()).is_some()
}

fn story_on(raw: &[u8]) -> Option<((usize, usize), Vec<u8>)> {
    let image = crate::dos_order::logical_order(raw)?;
    for at in (0..image.len()).step_by(SECTOR) {
        // Structural test on a borrow. Only a boundary that passes it is worth
        // copying a story's worth of bytes for, and on a disk that is not one of
        // these, none does.
        if !looks_like_story(&image[at..]) {
            continue;
        }
        if let Some(story) = crate::infocom_packed::verified(image[at..].to_vec()) {
            let sector = at / SECTOR;
            return Some(((sector / SECTORS, sector % SECTORS), story));
        }
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A synthetic raw self-booting disk carrying `story` at track 3 sector 0 —
    /// the place the one real disk of this kind keeps its own.
    ///
    /// [`crate::medium`]'s census uses it, which is why it is `pub(crate)`: every
    /// format there must be able to produce a sample it can then detect, mount
    /// and read back. **No filename argument**, unlike its siblings in
    /// [`crate::prodos`] and [`crate::fat12`] — this medium has no directory to
    /// put a name in, which is most of what makes it a separate format.
    pub(crate) fn sample_disk(story: &[u8]) -> Vec<u8> {
        const START: usize = 3 * SECTORS * SECTOR;
        let mut image = vec![0u8; crate::dos_order::DOS_ORDER_LEN];
        image[START..START + story.len()].copy_from_slice(story);
        // …and then scatter it the way the drive numbers its sectors, which is
        // the state a dump arrives in and the state the reader has to undo.
        crate::dos_order::dos_order_dump(&image).expect("the 5.25-inch geometry")
    }

    /// The one fixture, and the release it is. Named rather than globbed so a
    /// vanished file is a skip and never a silent pass.
    const PLANETFALL: &str = "Planetfall r29 (clean copy from retail disk).dsk";

    /// The sample is a boot disk, and finding it costs the reader the same
    /// de-interleave the real one does — so a table that broke would fail here
    /// with no fixture on disk at all, which is what CI runs.
    #[test]
    fn a_synthetic_boot_disk_round_trips_without_a_fixture() {
        let mut story = vec![0u8; 4096];
        story[0] = 3; // Version 3
        let mut word = |o: usize, v: u16| story[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x1a, 4096 / 2); // file length, the Version 3 unit
        story[0x12..0x18].copy_from_slice(b"840118");
        // Header checksum LAST, over `$40..` of everything above, so this fixture
        // is held to the same rule as the retail disk rather than exempted by a
        // zero.
        let sum = story[64..].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
        story[0x1c..0x1e].copy_from_slice(&sum.to_be_bytes());

        let raw = sample_disk(&story);
        assert_eq!(raw.len(), crate::dos_order::DOS_ORDER_LEN);
        assert!(InfocomBoot::looks_like_boot_disk(&raw));
        let disk = InfocomBoot::mount(raw).expect("it mounts");
        assert_eq!(disk.entry_name(), "T3/S0");
        assert_eq!(disk.story().expect("a story").1, story, "byte-exact off the disk");

        // FALSIFICATION, and one CI can run: declare a checksum one greater than
        // the truth — a byte below `$40` and therefore outside the sum it
        // describes — and the disk stops being one. Everything else about it,
        // including the structural `looks_like_story` test, still passes.
        let mut wrong = story.clone();
        wrong[0x1d] = wrong[0x1d].wrapping_add(1);
        assert!(looks_like_story(&wrong), "the premise: still structurally a story");
        assert!(!InfocomBoot::looks_like_boot_disk(&sample_disk(&wrong)), "and not a boot disk");
    }

    /// **A story file exactly one floppy long is a story, not a disk.** The IF
    /// Archive's `SpAdventure.z5` is 143,360 bytes, which is also the length of
    /// every Apple II 5.25-inch dump, and the sniff used to de-interleave it,
    /// find its header at track 0 sector 0, and pass the checksum, because a
    /// byte sum cannot tell a story from a sector-anagram of one (the module
    /// header says as much). The picker then listed a plain `.z5` as mounted
    /// off a boot disk, and opening it would have handed the machine the
    /// anagram.
    ///
    /// A self-booting disk's first sector is the boot ROM's loader, never a
    /// story header, so bytes that are a whole verified story in physical
    /// order from offset 0 cannot be one.
    ///
    /// FALSIFICATION: drop the `story_as_it_lies` guard and the second
    /// assertion fails with the reported symptom, a boot disk at `T0/S0`.
    #[test]
    fn a_story_file_exactly_one_floppy_long_is_a_story_not_a_disk() {
        let len = crate::dos_order::DOS_ORDER_LEN;
        let mut story = vec![0u8; len];
        story[0] = 5; // Version 5: 143,360 / 4 fits the length word, as SpAdventure's does
        let mut word = |o: usize, v: u16| story[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400);
        word(0x08, 0x0300);
        word(0x0a, 0x0100);
        word(0x0c, 0x0200);
        word(0x0e, 0x0280);
        word(0x1a, (len / 4) as u16);
        story[0x12..0x18].copy_from_slice(b"971030");
        let sum = story[64..].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
        story[0x1c..0x1e].copy_from_slice(&sum.to_be_bytes());

        assert!(
            crate::infocom_packed::verified(story.clone()).is_some(),
            "the premise: a whole verified story as it lies"
        );
        assert!(!InfocomBoot::looks_like_boot_disk(&story), "one floppy long, but a story file");
        assert!(InfocomBoot::mount(story.clone()).is_err(), "and it does not mount as one");
        assert_eq!(crate::medium::DiskImage::detect(&story), None, "no format claims it");
    }

    fn stories_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    fn fixture(name: &str) -> Option<Vec<u8>> {
        std::fs::read(stories_dir().join(name)).ok()
    }

    /// A 5.25-inch dump of the right size holding nothing at all is not a boot
    /// disk — the sniff needs a story, not a geometry.
    #[test]
    fn an_empty_five_and_a_quarter_inch_dump_is_not_a_boot_disk() {
        let blank = vec![0u8; crate::dos_order::DOS_ORDER_LEN];
        assert!(!InfocomBoot::looks_like_boot_disk(&blank));
        assert_eq!(InfocomBoot::mount(blank).err(), Some(InfocomBootError::NotABootDisk));
    }

    /// Nor is anything of another size, however story-like. DOS sector order is a
    /// 5.25-inch phenomenon and this reader claims exactly that geometry, the
    /// same rule [`crate::dos_order`] applies.
    #[test]
    fn only_the_five_and_a_quarter_inch_geometry_is_claimed() {
        assert!(!InfocomBoot::looks_like_boot_disk(&[]));
        assert!(!InfocomBoot::looks_like_boot_disk(&vec![0u8; 819_200]));
        assert!(!InfocomBoot::looks_like_boot_disk(&vec![0u8; crate::dos_order::DOS_ORDER_LEN - 1]));
    }

    /// **The fixture, end to end**: the disk is claimed, it mounts, and what
    /// comes off it is the release the corpus says it is.
    #[test]
    fn the_planetfall_boot_disk_yields_release_twenty_nine() {
        let Some(raw) = fixture(PLANETFALL) else {
            eprintln!("SKIP: no {PLANETFALL}");
            return;
        };
        assert_eq!(raw.len(), crate::dos_order::DOS_ORDER_LEN, "35 × 16 × 256");
        assert!(InfocomBoot::looks_like_boot_disk(&raw));
        let disk = InfocomBoot::mount(raw).expect("it mounts");
        assert_eq!(disk.file_count(), 1);
        assert_eq!(disk.entry_name(), "T3/S0", "track 3, sector 0");
        let (name, story) = disk.story().expect("a story");
        assert_eq!(name, "T3/S0");
        assert_eq!(story.len(), 109_052, "54,526 length units × 2 (ZMSD §11.1.6)");
        assert_eq!(story[0], 3, "Version 3");
        assert_eq!(u16::from_be_bytes([story[2], story[3]]), 29, "release 29");
        assert_eq!(&story[0x12..0x18], b"840118", "serial 840118");
        assert_eq!(u16::from_be_bytes([story[0x1c], story[0x1d]]), 0x842E, "checksum $842E");
    }

    /// **FALSIFICATION** (SQ-0868). The checksum is the oracle, so state what it
    /// rejects: the *other* two orders of the very same sectors produce a story
    /// that is right about its version, release and serial and wrong about its
    /// checksum, which is the trap this whole reader exists to avoid walking
    /// into.
    #[test]
    fn the_other_sector_orders_produce_a_plausible_story_with_a_wrong_checksum() {
        let Some(raw) = fixture(PLANETFALL) else {
            eprintln!("SKIP: no {PLANETFALL}");
            return;
        };
        let declared = 0x842Eu16;
        for (what, image) in [
            ("physical", raw.clone()),
            ("ProDOS block", crate::dos_order::prodos_order(&raw).expect("the geometry")),
        ] {
            // The header is at track 3 sector 0 in every order — sector 0 is
            // sector 0 in all of them — so the version, release and serial are
            // identical and *only* the checksum disagrees.
            let story = &image[12_288..];
            assert_eq!(story[0], 3, "{what}: still says Version 3");
            assert_eq!(u16::from_be_bytes([story[2], story[3]]), 29, "{what}: still release 29");
            assert_eq!(&story[0x12..0x18], b"840118", "{what}: still serial 840118");
            let sum = story[64..109_052].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
            assert_ne!(sum, declared, "{what}: a wrong order must NOT verify");
        }
        // …and the reader, which uses the RIGHT order, claims the disk.
        assert!(InfocomBoot::looks_like_boot_disk(&raw));
    }

    /// The two exact sums the module header quotes, pinned. A future change to
    /// the de-interleave that made "physical" mean something else would leave the
    /// documentation quietly wrong; this makes it fail instead.
    #[test]
    fn the_three_orders_sum_to_the_three_values_documented() {
        let Some(raw) = fixture(PLANETFALL) else {
            eprintln!("SKIP: no {PLANETFALL}");
            return;
        };
        let sum = |image: &[u8]| {
            image[12_288 + 64..12_288 + 109_052].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)))
        };
        assert_eq!(sum(&raw), 0x529D, "physical order");
        assert_eq!(sum(&crate::dos_order::prodos_order(&raw).expect("geometry")), 0x97D5, "ProDOS");
        assert_eq!(sum(&crate::dos_order::logical_order(&raw).expect("geometry")), 0x842E, "logical");
    }

    /// **The structural oracle the checksum cannot be** (see the module header):
    /// a byte sum is blind to order, so the dictionary is decoded too. Only the
    /// logical order puts a real Version 3 dictionary where the header points.
    #[test]
    fn only_the_logical_order_puts_a_real_dictionary_where_the_header_points() {
        let Some(raw) = fixture(PLANETFALL) else {
            eprintln!("SKIP: no {PLANETFALL}");
            return;
        };
        // (separator count, separators, entry length, entry count) at the
        // dictionary address in header `$08`.
        let dictionary = |image: &[u8]| {
            let s = &image[12_288..];
            let at = usize::from(u16::from_be_bytes([s[8], s[9]]));
            let n = usize::from(s[at]);
            let seps = s[at + 1..at + 1 + n].to_vec();
            let entry = s[at + 1 + n];
            let count = u16::from_be_bytes([s[at + 2 + n], s[at + 3 + n]]);
            (n, seps, entry, count)
        };
        let (n, seps, entry, count) =
            dictionary(&crate::dos_order::logical_order(&raw).expect("geometry"));
        assert_eq!(n, 3, "three word separators");
        assert_eq!(seps, b",.\"", "Infocom's Version 3 separators");
        assert_eq!(entry, 7, "seven-byte entries, the Version 3 size");
        assert_eq!(count, 665, "665 words");

        for (what, image) in [
            ("physical", raw.clone()),
            ("ProDOS block", crate::dos_order::prodos_order(&raw).expect("geometry")),
        ] {
            let (n, _, entry, _) = dictionary(&image);
            assert_ne!((n, entry), (3, 7), "{what}: must not decode as a dictionary");
        }
    }

    /// **The order-sensitive oracle** (SQ-0868), and the one this suite was
    /// missing until a falsification run found it.
    ///
    /// Breaking the interleave by swapping two entries of `dos_order`'s table
    /// leaves every OTHER test here passing, because each of them is blind to
    /// the swap for a different reason: the byte sum is order-blind by nature and
    /// the two sectors involved fall past the story's last partial track anyway,
    /// the header is in sector 0 which no swap moves, and the dictionary happens
    /// to sit in a sector the swap did not touch. All three are real checks and
    /// none of them covers 426 sectors.
    ///
    /// This does. FNV-1a over the whole extracted story is order-sensitive by
    /// construction, so any permutation of any two sectors changes it.
    ///
    /// **It is a pin and not evidence**, and the difference matters: a
    /// fingerprint of one's own output is exactly the test that passes when the
    /// implementation is wrong. What makes this value trustworthy is that the
    /// bytes behind it were established three other ways first — the header
    /// checksum over their multiset, the Version 3 dictionary decoding out of
    /// them, and *Planetfall* release 29 booting and playing off this disk
    /// through `zvm-cli`. This records the result so a later change cannot move
    /// it quietly.
    #[test]
    fn the_extracted_story_is_the_same_426_sectors_in_the_same_order() {
        let Some(raw) = fixture(PLANETFALL) else {
            eprintln!("SKIP: no {PLANETFALL}");
            return;
        };
        let disk = InfocomBoot::mount(raw).expect("it mounts");
        let (_, story) = disk.story().expect("a story");
        assert_eq!(story.len(), 109_052);
        assert_eq!(fnv1a(&story), 0xC411_7D47_EF39_465A, "the story's bytes, in order");
    }

    /// FNV-1a 64, hand-rolled — `blorb` takes no dependencies, and the property
    /// wanted here is only that reordering the input changes the output.
    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    /// The property the test above rests on, said out loud: the fingerprint sees
    /// a swap that the header checksum cannot. This is the falsification of
    /// SQ-0868's own falsification, and it needs no fixture.
    #[test]
    fn the_fingerprint_sees_a_reordering_the_checksum_is_blind_to() {
        let a: Vec<u8> = (0..1024u32).map(|i| i as u8).collect();
        let mut b = a.clone();
        b.swap(3, 900);
        let sum = |v: &[u8]| v.iter().fold(0u16, |a, &x| a.wrapping_add(u16::from(x)));
        assert_eq!(sum(&a), sum(&b), "a byte sum cannot see a reordering");
        assert_ne!(fnv1a(&a), fnv1a(&b), "and the fingerprint must");
    }

    /// A raw disk holds no artwork this reader will claim, and says so rather
    /// than guessing. See [`InfocomBoot::pictures`].
    #[test]
    fn a_boot_disk_offers_no_artwork() {
        let Some(raw) = fixture(PLANETFALL) else {
            eprintln!("SKIP: no {PLANETFALL}");
            return;
        };
        let disk = InfocomBoot::mount(raw).expect("it mounts");
        assert!(disk.pictures().is_none());
    }

    /// The name a caller is shown is a name it can ask back for — the property
    /// every format in this crate has to have, because it is the `--pictures`
    /// door.
    #[test]
    fn what_the_disk_lists_it_will_also_hand_over() {
        let Some(raw) = fixture(PLANETFALL) else {
            eprintln!("SKIP: no {PLANETFALL}");
            return;
        };
        let disk = InfocomBoot::mount(raw).expect("it mounts");
        let contents = disk.contents();
        assert_eq!(contents.len(), 1);
        let (name, bytes) = &contents[0];
        assert_eq!(disk.read_named(name).as_ref(), Some(bytes));
        assert_eq!(disk.read_named("t3/s0").as_ref(), Some(bytes), "case-insensitive");
        assert_eq!(disk.read_named("T9/S9"), None);
    }

    /// **The disjointness guard** (SQ-0868), from this side: every ProDOS
    /// 5.25-inch volume in the corpus is the same size, the same sector order and
    /// the same spelling as the boot disk, and not one of them is claimed here.
    ///
    /// Both halves are asserted, because the interesting failure is not "a
    /// ProDOS volume was claimed" — the explicit refusal in
    /// [`InfocomBoot::looks_like_boot_disk`] makes that unreachable — but "it
    /// would have been claimed even without that refusal", which is what would
    /// make table order start to matter.
    #[test]
    fn no_prodos_five_and_a_quarter_inch_volume_is_a_boot_disk() {
        let volumes = [
            "shogun_s1.dsk", "shogun_s2.dsk", "shogun_s3.dsk", "shogun_s4.dsk", "shogun_s5.dsk",
            "zork_zero_1.dsk", "zork_zero_2.dsk", "zork_zero_3.dsk", "zork_zero_4.dsk",
            "journey_s1.dsk", "journey_s2.dsk", "journey_s3.dsk", "journey_s4.dsk",
            "journey_s5.dsk",
        ];
        let mut ran = 0;
        for name in volumes {
            let Some(raw) = fixture(name) else { continue };
            ran += 1;
            assert_eq!(raw.len(), crate::dos_order::DOS_ORDER_LEN, "{name}");
            assert!(crate::prodos::ProDos::looks_like_prodos(&raw), "{name}: is a ProDOS volume");
            assert!(!InfocomBoot::looks_like_boot_disk(&raw), "{name}: refused outright");
            // …and it carries no whole verified story of its own either, so the
            // two sniffs would be disjoint even with the refusal removed.
            assert!(story_on(&raw).is_none(), "{name}: no whole story on one segment");
        }
        assert!(ran == 14 || ran == 0, "expected fourteen ProDOS 5.25-inch volumes, got {ran}");
        if ran == 0 {
            eprintln!("SKIP: no ProDOS 5.25-inch media present");
        }
    }

    /// **The whole shelf**, from this side: exactly one file in `stories/` is a
    /// raw self-booting disk, and it is the one this module names. A sniff that
    /// grew loose enough to claim a second file fails here.
    #[test]
    fn exactly_one_image_in_the_corpus_is_a_boot_disk() {
        let Ok(dir) = std::fs::read_dir(stories_dir()) else {
            eprintln!("SKIP: no stories directory");
            return;
        };
        let mut claimed: Vec<String> = Vec::new();
        let mut seen = 0;
        for entry in dir.flatten() {
            let path = entry.path();
            let Ok(raw) = std::fs::read(&path) else { continue };
            seen += 1;
            if InfocomBoot::looks_like_boot_disk(&raw) {
                claimed.push(path.file_name().unwrap_or_default().to_string_lossy().into_owned());
            }
        }
        if seen == 0 {
            eprintln!("SKIP: empty stories directory");
            return;
        }
        assert_eq!(claimed, [PLANETFALL], "exactly one raw self-booting disk");
    }
}
