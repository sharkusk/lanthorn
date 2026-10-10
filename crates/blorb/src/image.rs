//! The one door through which a disk image FILE is read (SQ-1762, SQ-1761).
//!
//! A launch used to read the same image four or five times over: the story step
//! read it to mount and find the game, the artwork step read it to mount and
//! list its files, the native-font step read it twice more, and the
//! machine-detection step read it only to sniff the format. Every one of them
//! did `std::fs::read(path)` and handed the bytes to a reader, so no step could
//! know another had already paid. On a network share the 354 MB *Masterpieces*
//! CD made each of those a minute.
//!
//! The fix is that **nothing outside this module reads an image file.** Every
//! consumer asks [`open`] (or its narrower siblings [`open_disk`],
//! [`detect_at`], [`read_bytes`]) and gets a [`DiskFile`]: the path, the format
//! its content turned out to be, and the bytes — one value carrying the facts
//! together, so a step cannot hold the bytes and forget the format, or the
//! reverse.
//!
//! # A CD is not read at all
//!
//! A floppy is read whole, once, into memory. A **CD image** — an ISO 9660 `.iso`,
//! a raw `.bin` dump, an Apple-partitioned hybrid like the *Masterpieces* disc —
//! is not: [`open`] reads its first 48 KB, decides *by content* that it is one,
//! and keeps the file open. The [`DiskFile`] it returns is a handle, and the
//! volume readers behind it read the sectors they are asked for (see
//! [`crate::source`]). Opening one game off the 354 MB disc reads a few
//! megabytes of it.
//!
//! # The cache
//!
//! A small, process-wide, most-recently-used set of opened images, keyed by
//! `(path, length, modification time)`. The key is the point: an image edited
//! between the picker's scan and the launch has a new mtime or length and is
//! opened afresh, so the cache can only ever answer for the file as it is now.
//! A file that is NOT a disk image is remembered too — as a negative, with no
//! bytes — so the machine-detection fallback does not re-read a 40 MB Glulx
//! story to learn what the story step already knew.
//!
//! The cache is bounded by entries and by the bytes it holds, and the oldest
//! image goes first. A CD's handle holds no bytes and costs nothing against the
//! bound.
//!
//! # Counting
//!
//! [`whole_reads_on_this_thread`] and [`bytes_read_on_this_thread`]
//! count what this module has read on the calling thread. They exist so a test
//! can pin "a launch reads the image once" and "a CD launch reads megabytes, not
//! the disc" as numbers; those tests are only as good as the claim that nothing
//! else reads an image, which `app`'s `image_read_once` suite scans for.

use crate::medium::{DiskImage, MountedDisk};
pub use crate::source::bytes_read_on_this_thread;
use crate::source::{count_bytes, Disc, Source};
use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// How many opened images (and remembered negatives) the cache keeps.
const MAX_ENTRIES: usize = 24;
/// The most image bytes the cache keeps alive at once.
const MAX_BYTES: usize = 400 * 1024 * 1024;
/// A file shorter than this is not a CD image: no volume descriptor fits.
const MIN_DISC: u64 = 64 * 1024;

thread_local! {
    static WHOLE_READS: Cell<usize> = const { Cell::new(0) };
}

/// How many times this module has read a whole image file from disk on this
/// thread. A cache hit does not count, and neither does opening a CD, which
/// reads sectors instead; that is the point.
pub fn whole_reads_on_this_thread() -> usize {
    WHOLE_READS.with(Cell::get)
}

/// Give the calling thread a private, empty cache until the guard drops.
///
/// For a test that pins a read COUNT. The process-wide cache is shared by every
/// thread, and `cargo test` runs a binary's tests on threads of one process, so
/// another test's opens could evict an image mid-count and add a read that is
/// nobody's fault. Nothing in the app calls this.
#[must_use = "the private cache lasts only as long as the guard"]
pub fn isolate() -> Isolated {
    Isolated(ISOLATED.with(|c| c.replace(Some(Vec::new()))))
}

/// See [`isolate`].
pub struct Isolated(Option<Vec<Entry>>);

impl Drop for Isolated {
    fn drop(&mut self) {
        let previous = self.0.take();
        ISOLATED.with(|c| *c.borrow_mut() = previous);
    }
}

thread_local! {
    static ISOLATED: std::cell::RefCell<Option<Vec<Entry>>> = const { std::cell::RefCell::new(None) };
}

#[derive(Debug, Clone)]
enum Backing {
    /// The whole image, held.
    Mem(Arc<Vec<u8>>),
    /// A CD image left on disk: a window onto the volume it carries.
    Disc(Source),
}

/// An opened release disk image: what file it is, what format its content is,
/// and a way to its bytes. Cheap to clone — the bytes, or the open file, are
/// shared.
#[derive(Debug, Clone)]
pub struct DiskFile {
    path: PathBuf,
    format: DiskImage,
    backing: Backing,
}

impl DiskFile {
    /// The file this was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The format the image's CONTENT claimed.
    pub fn format(&self) -> DiskImage {
        self.format
    }

    /// The image's bytes — or `None` for a CD, which stays on disk. A caller that
    /// needs bytes from one mounts it and asks the volume for a file.
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.backing {
            Backing::Mem(bytes) => Some(bytes),
            Backing::Disc(_) => None,
        }
    }

    /// Whether this is a CD read by sector rather than held in memory.
    pub fn is_file_backed(&self) -> bool {
        matches!(self.backing, Backing::Disc(_))
    }

    fn held_bytes(&self) -> usize {
        match &self.backing {
            Backing::Mem(bytes) => bytes.len(),
            Backing::Disc(_) => 0,
        }
    }
}

/// What a path turned out to hold.
#[derive(Debug)]
pub enum Opened {
    /// A release disk image, ready to mount.
    Disk(DiskFile),
    /// Anything else — an ordinary story file, an archive — with the bytes that
    /// were read to find that out, so the caller need not read them again.
    Plain(Vec<u8>),
}

/// `(path, length, mtime)`: the file as it is right now.
#[derive(Debug, PartialEq, Eq, Clone)]
struct Key {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

impl Key {
    fn of(path: &Path) -> io::Result<Key> {
        let meta = std::fs::metadata(path)?;
        Ok(Key { path: path.to_path_buf(), len: meta.len(), modified: meta.modified().ok() })
    }
}

struct Entry {
    key: Key,
    /// `None` is a remembered negative: this file is not a disk image.
    file: Option<DiskFile>,
}

/// Most recently used LAST.
static CACHE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

/// Run `f` over this thread's private cache if it has one, else the shared one.
fn with_cache<R>(f: impl FnOnce(&mut Vec<Entry>) -> R) -> Option<R> {
    let mut f = Some(f);
    let private = ISOLATED.with(|c| c.borrow_mut().as_mut().and_then(|v| f.take().map(|f| f(v))));
    if private.is_some() {
        return private;
    }
    let mut shared = CACHE.lock().ok()?;
    f.take().map(|f| f(&mut shared))
}

fn lookup(key: &Key) -> Option<Option<DiskFile>> {
    with_cache(|cache| {
        let at = cache.iter().position(|e| &e.key == key)?;
        let hit = cache.remove(at);
        let answer = hit.file.clone();
        cache.push(hit);
        Some(answer)
    })?
}

fn remember(key: Key, file: Option<DiskFile>) {
    let _ = with_cache(|cache| remember_in(cache, key, file));
}

fn remember_in(cache: &mut Vec<Entry>, key: Key, file: Option<DiskFile>) {
    cache.retain(|e| e.key.path != key.path);
    cache.push(Entry { key, file });
    let held = |c: &Vec<Entry>| -> usize {
        c.iter().filter_map(|e| e.file.as_ref()).map(DiskFile::held_bytes).sum()
    };
    while cache.len() > MAX_ENTRIES || (cache.len() > 1 && held(cache) > MAX_BYTES) {
        cache.remove(0);
    }
}

fn read_whole(path: &Path) -> io::Result<Vec<u8>> {
    WHOLE_READS.with(|c| c.set(c.get() + 1));
    let raw = std::fs::read(path)?;
    count_bytes(raw.len());
    Ok(raw)
}

/// `path` as a CD image left on disk, when its first 48 KB say it is one.
///
/// **Content decides, never the extension.** A CD is either an Apple-partitioned
/// medium with a sane Macintosh volume in it, or a cooked ISO 9660 disc; the
/// same two tests the in-memory sniff makes, answered by reading sectors. A disc
/// an earlier row of the format table would claim by another arm (a bare
/// Macintosh volume, a DiskCopy header) declines here and goes the in-memory way,
/// so the two paths never choose different readers.
fn open_disc(path: &Path, key: &Key) -> Option<DiskFile> {
    if key.len < MIN_DISC {
        return None;
    }
    let disc = Arc::new(Disc::open(path).ok()?);
    let (format, window) = if let Some(w) = crate::hfs::disc_volume(&disc) {
        (DiskImage::Hfs, w)
    } else {
        (DiskImage::Iso9660, crate::iso9660::disc_volume(&disc)?)
    };
    Some(DiskFile { path: path.to_path_buf(), format, backing: Backing::Disc(window) })
}

/// Open `path`: a disk image comes back mountable, anything else comes back as
/// the plain bytes read to find that out. A floppy is read from disk at most
/// once per `(path, length, mtime)`; a CD is not read, only opened.
pub fn open(path: &Path) -> io::Result<Opened> {
    let key = Key::of(path)?;
    if let Some(Some(file)) = lookup(&key) {
        return Ok(Opened::Disk(file));
    }
    if let Some(file) = open_disc(path, &key) {
        remember(key, Some(file.clone()));
        return Ok(Opened::Disk(file));
    }
    // A remembered negative has no bytes, so a caller wanting them reads again;
    // that is what an ordinary story file has always cost.
    let raw = read_whole(path)?;
    match DiskImage::detect(&raw) {
        Some(format) => {
            let file =
                DiskFile { path: path.to_path_buf(), format, backing: Backing::Mem(Arc::new(raw)) };
            remember(key, Some(file.clone()));
            Ok(Opened::Disk(file))
        }
        None => {
            remember(key, None);
            Ok(Opened::Plain(raw))
        }
    }
}

/// [`open`], for a caller that wants a disk image or nothing: `None` for an
/// unreadable file and for one that is not a disk image. A remembered negative
/// answers without a read.
pub fn open_disk(path: &Path) -> Option<DiskFile> {
    let key = Key::of(path).ok()?;
    if let Some(known) = lookup(&key) {
        return known;
    }
    match open(path).ok()? {
        Opened::Disk(file) => Some(file),
        Opened::Plain(_) => None,
    }
}

/// The medium `path`'s content is, or `None` — [`DiskImage::detect`] asked of a
/// file.
pub fn detect_at(path: &Path) -> Option<DiskImage> {
    open_disk(path).map(|f| f.format)
}

/// A file's whole bytes, whatever it is. For the callers that want an image's
/// raw content rather than its mounted contents (a companion picture side).
/// Reads a CD whole, because that is what was asked for.
pub fn read_bytes(path: &Path) -> io::Result<Vec<u8>> {
    match open(path)? {
        Opened::Disk(file) => match file.bytes() {
            Some(bytes) => Ok(bytes.to_vec()),
            None => read_whole(path),
        },
        Opened::Plain(raw) => Ok(raw),
    }
}

impl MountedDisk {
    /// [`MountedDisk::mount_set`] for an opened image: the same mount, without
    /// the caller having read the file itself. A CD mounts over its file, by
    /// sector.
    pub fn mount_file(
        file: &DiskFile,
        companions: impl FnOnce() -> Vec<Vec<u8>>,
    ) -> Result<MountedDisk, crate::medium::MountError> {
        match &file.backing {
            Backing::Mem(bytes) => MountedDisk::mount_set(bytes.to_vec(), companions),
            Backing::Disc(window) => {
                MountedDisk::mount_window(file.format, window.clone(), companions)
            }
        }
    }
}

impl crate::hfs::Hfs {
    /// [`crate::hfs::Hfs::mount`] for an opened image, for the callers that want
    /// the Macintosh volume itself (its resource forks) rather than the
    /// format-neutral [`MountedDisk`].
    pub fn mount_file(file: &DiskFile) -> Result<crate::hfs::Hfs, crate::hfs::HfsError> {
        match &file.backing {
            Backing::Mem(bytes) => crate::hfs::Hfs::mount(bytes.to_vec()),
            Backing::Disc(window) if file.format == DiskImage::Hfs => {
                crate::hfs::Hfs::mount_source(window.clone())
            }
            Backing::Disc(_) => Err(crate::hfs::HfsError::NotHfs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::medium::MountedDisk;

    /// RAII guard that removes a temp directory (and its contents) on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("lanthorn-blorb-image-{}-{tag}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A structurally valid v6 story of `len` bytes with a non-zero body.
    fn story(len: usize, serial: &[u8; 6]) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[0] = 6;
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400);
        word(0x08, 0x0300);
        word(0x0a, 0x0100);
        word(0x0c, 0x0200);
        word(0x0e, 0x0280);
        word(0x1a, (len / 8) as u16);
        b[0x12..0x18].copy_from_slice(serial);
        for (i, byte) in b.iter_mut().enumerate().skip(64) {
            *byte = (i % 251) as u8;
        }
        b
    }

    /// `n` zero bytes on the end of a disc image: the other 300 MB of a CD.
    fn with_tail(mut image: Vec<u8>, n: usize) -> Vec<u8> {
        image.resize(image.len() + n, 0);
        image
    }

    /// Everything a front-end can ask of a mounted disk, as a comparable value.
    fn account_of(disk: &MountedDisk) -> String {
        let stories: Vec<(String, usize, Vec<u8>)> =
            disk.stories().into_iter().map(|s| (s.name.clone(), s.bytes.len(), s.bytes)).collect();
        format!(
            "{:?} {:?} files={} names={:?} story={:?} stories={:?} contents={:?}",
            disk.format(),
            disk.volume_name(),
            disk.file_count(),
            disk.story_names(),
            disk.story().map(|s| (s.name, s.bytes)),
            stories,
            disk.contents(),
        )
    }

    /// **The file-backed path mounts the same disk the in-memory path does** —
    /// for each CD framing the crate reads — and does it without reading the
    /// file.
    ///
    /// The padding is the other 6 MB of a CD; the volume and the stories are
    /// kilobytes of it. Falsify by making `open_disc` decline: the image is read
    /// whole, `is_file_backed` is false and the assertions below fail.
    #[test]
    fn a_cd_is_read_by_sector_and_agrees_with_the_in_memory_path() {
        let (a, b) = (story(8192, b"890101"), story(4096, b"890202"));
        let files: [(&str, &[u8]); 3] = [("Readme", b"a text file"), ("A.DAT", &a), ("B.DAT", &b)];
        let volume = crate::hfs::tests::sample_volume(&files);
        let cooked_hfs = crate::cd::tests::partitioned(&volume, 20 * volume.len() / 512);
        let raw_hfs = crate::cd::tests::raw_sectors(&cooked_hfs);
        let iso = crate::iso9660::tests::sample_disc(&files);

        let dir = TempDir::new("cd");
        for (what, image, format) in [
            ("a cooked partitioned .iso", cooked_hfs, DiskImage::Hfs),
            ("a raw .bin dump", raw_hfs, DiskImage::Hfs),
            ("an ISO 9660 disc", iso, DiskImage::Iso9660),
        ] {
            let image = with_tail(image, 6 * 1024 * 1024);
            let path = dir.file("disc.img", &image);
            let _isolated = isolate();
            let (reads0, bytes0) = (whole_reads_on_this_thread(), bytes_read_on_this_thread());

            let Opened::Disk(file) = open(&path).expect("opens") else { panic!("{what}: a disk") };
            assert!(file.is_file_backed(), "{what}: a CD stays on disk");
            assert_eq!(file.format(), format, "{what}");
            let disk = MountedDisk::mount_file(&file, Vec::new).expect("mounts");
            assert!(disk.is_file_backed(), "{what}");
            assert_eq!(disk.story_names().len(), 2, "{what}");
            let one = disk.story_named("b.dat").expect("found, ignoring case");
            assert_eq!(one.bytes, b, "{what}");
            assert_eq!(disk.story_count_up_to(1), 1, "{what}");
            assert!(disk.has_stories(), "{what}");

            assert_eq!(whole_reads_on_this_thread(), reads0, "{what}: no whole-file read");
            let read = bytes_read_on_this_thread() - bytes0;
            assert!(
                (read as usize) < image.len() / 4,
                "{what}: opened, mounted, listed and extracted with {read} of {} bytes read",
                image.len()
            );

            // …and it is the same disk. The in-memory mount is the oracle.
            let oracle = MountedDisk::mount(image.clone()).expect("the oracle mounts");
            assert!(!oracle.is_file_backed(), "{what}");
            assert_eq!(account_of(&disk), account_of(&oracle), "{what}");
            assert_eq!(DiskImage::detect(&image), Some(format), "{what}: the sniffs agree");
        }
    }

    /// A floppy is read whole, once, and stays in memory; so is a bare Macintosh
    /// volume and an ordinary story file. Content decides what a CD is.
    #[test]
    fn only_a_cd_stays_on_disk() {
        let adf = {
            let mut v = vec![0u8; 880 * 1024];
            v[0..3].copy_from_slice(b"DOS");
            v
        };
        let volume = crate::hfs::tests::sample_volume(&[("STORY.DAT", &story(4096, b"890303"))]);
        let dir = TempDir::new("floppy");
        for (what, image, is_disk) in [
            ("an Amiga floppy", adf, true),
            ("a bare Macintosh volume", volume, true),
            ("a story file", story(100 * 1024, b"890404"), false),
        ] {
            let path = dir.file("x.img", &image);
            let _isolated = isolate();
            let reads0 = whole_reads_on_this_thread();
            match open(&path).expect("opens") {
                Opened::Disk(file) => {
                    assert!(is_disk, "{what}");
                    assert!(!file.is_file_backed(), "{what}: held in memory");
                }
                Opened::Plain(raw) => {
                    assert!(!is_disk, "{what}");
                    assert_eq!(raw, image, "{what}");
                }
            }
            assert_eq!(whole_reads_on_this_thread(), reads0 + 1, "{what}: read whole, once");
        }
    }

    /// The cache answers for the file as it is now.
    #[test]
    fn an_edited_image_is_opened_afresh() {
        let dir = TempDir::new("edit");
        let mut adf = vec![0u8; 880 * 1024];
        adf[0..3].copy_from_slice(b"DOS");
        let path = dir.file("d.adf", &adf);
        let _isolated = isolate();
        let before = whole_reads_on_this_thread();
        assert!(open_disk(&path).is_some());
        assert!(open_disk(&path).is_some());
        assert_eq!(whole_reads_on_this_thread(), before + 1, "the second open is the cache's");
        adf.push(0);
        std::fs::write(&path, &adf).unwrap();
        let _ = open_disk(&path);
        assert_eq!(whole_reads_on_this_thread(), before + 2, "a changed length is a new file");
    }

    /// A real CD in the corpus, or `None` (a vacuous skip) when it is absent —
    /// CI has none of them.
    fn real_cd(relative: &str) -> Option<PathBuf> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative);
        if path.is_file() {
            Some(path)
        } else {
            eprintln!("SKIP: {} is absent", path.display());
            None
        }
    }

    /// **Real media**: open, mount, list and extract one game off a real CD
    /// without reading the disc, and get exactly the answers the in-memory path
    /// gives (SQ-1761).
    ///
    /// The in-memory mount is the oracle, so this is as strong as the claim that
    /// the old path was right; what it adds is that the new one is the same.
    fn assert_cd_by_sector(relative: &str, format: DiskImage, stories: usize, want: &str) {
        let Some(path) = real_cd(relative) else { return };
        let size = std::fs::metadata(&path).unwrap().len();
        let _isolated = isolate();
        let (reads0, bytes0) = (whole_reads_on_this_thread(), bytes_read_on_this_thread());

        let Opened::Disk(file) = open(&path).expect("opens") else { panic!("a disk image") };
        assert!(file.is_file_backed());
        assert_eq!(file.format(), format);
        let disk = MountedDisk::mount_file(&file, Vec::new).expect("mounts");
        let names = disk.story_names();
        assert_eq!(names.len(), stories, "{relative}: every story the disc offers");
        let one = disk.story_named(want).unwrap_or_else(|| panic!("{want} is on {relative}"));
        let read = bytes_read_on_this_thread() - bytes0;
        assert_eq!(whole_reads_on_this_thread(), reads0, "the disc is never read whole");
        assert!(
            read < 16 * 1024 * 1024,
            "{relative}: mount + list + extract read {read} bytes of {size}"
        );
        eprintln!("{relative}: mount + list + extract one story read {read} of {size} bytes");

        // The oracle: the same disc, read whole into memory.
        let oracle = MountedDisk::mount(std::fs::read(&path).unwrap()).expect("oracle mounts");
        assert!(!oracle.is_file_backed());
        assert_eq!(oracle.format(), disk.format());
        assert_eq!(oracle.volume_name(), disk.volume_name());
        assert_eq!(oracle.file_count(), disk.file_count());
        let all = oracle.stories();
        assert_eq!(all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), names);
        assert_eq!(disk.stories(), all, "every story, byte for byte");
        assert_eq!(oracle.story(), disk.story(), "the tiebreak story");
        assert_eq!(Some(one), all.iter().find(|s| s.name == want).cloned());
        for s in &all {
            assert_eq!(oracle.image_for(&s.name), disk.image_for(&s.name), "{}", s.name);
        }
        // A sample of ALL files, not only stories.
        let listing = disk.files().expect("a listing");
        for (name, bytes) in oracle.contents().into_iter().step_by(7) {
            let i = listing.iter().position(|f| f.path == name).expect("listed");
            assert_eq!(disk.read_file(i).as_deref(), Some(&bytes[..]), "{name}");
        }
    }

    #[test]
    fn the_masterpieces_cd_opens_by_sector() {
        assert_cd_by_sector(
            "masterpieces/Classic Text Adventure Masterpieces of Infocom (USA).bin",
            DiskImage::Hfs,
            83,
            "MAC/ZORK I",
        );
    }

    #[test]
    fn a_lost_treasures_iso_opens_by_sector() {
        let Some(path) = real_cd("treasures/ISOs/LostTreasures1.iso") else { return };
        let probe = MountedDisk::mount(std::fs::read(path).unwrap()).expect("mounts");
        let stories = probe.stories();
        let want = stories.first().expect("a story").name.clone();
        assert_cd_by_sector(
            "treasures/ISOs/LostTreasures1.iso",
            DiskImage::Iso9660,
            stories.len(),
            &want,
        );
    }
}
