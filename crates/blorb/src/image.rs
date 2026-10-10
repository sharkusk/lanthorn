//! The one door through which a disk image FILE is read (SQ-1762).
//!
//! A launch used to read the same image four or five times over: the story
//! step read it to mount and find the game, the artwork step read it to mount
//! and list its files, the native-font step read it twice more, and the
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
//! # The cache
//!
//! A small, process-wide, most-recently-used set of opened images, keyed by
//! `(path, length, modification time)`. The key is the point: an image edited
//! between the picker's scan and the launch has a new mtime or length and is
//! read afresh, so the cache can only ever answer for the file as it is now.
//! A file that is NOT a disk image is remembered too — as a negative, with no
//! bytes — so the machine-detection fallback does not re-read a 40 MB Glulx
//! story to learn what the story step already knew.
//!
//! The cache is bounded by entries and by the bytes it holds, and the oldest
//! image goes first.
//!
//! # Counting
//!
//! [`whole_reads_on_this_thread`] counts the whole-file reads this module has
//! made on the calling thread. It exists so a test can pin "a launch reads the
//! image once" as a number; the test is only as good as the claim that nothing
//! else reads an image, which `app`'s `image_read_discipline` suite scans for.

use crate::medium::{DiskImage, MountedDisk};
use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// How many opened images (and remembered negatives) the cache keeps.
const MAX_ENTRIES: usize = 24;
/// The most image bytes the cache keeps alive at once.
const MAX_BYTES: usize = 400 * 1024 * 1024;

thread_local! {
    static WHOLE_READS: Cell<usize> = const { Cell::new(0) };
}

/// How many times [`open`] and friends have read a whole image file from disk
/// on this thread. A cache hit does not count; that is the point.
pub fn whole_reads_on_this_thread() -> usize {
    WHOLE_READS.with(Cell::get)
}

/// Forget everything the cache holds, so a test starts from a cold one.
pub fn clear_cache() {
    if let Ok(mut c) = CACHE.lock() {
        c.clear();
    }
}

/// An opened release disk image: what file it is, what format its content is,
/// and the bytes. Cheap to clone — the bytes are shared.
#[derive(Debug, Clone)]
pub struct DiskFile {
    path: PathBuf,
    format: DiskImage,
    bytes: Arc<Vec<u8>>,
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

    /// The image's bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
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

fn lookup(key: &Key) -> Option<Option<DiskFile>> {
    let mut cache = CACHE.lock().ok()?;
    let at = cache.iter().position(|e| &e.key == key)?;
    let hit = cache.remove(at);
    let answer = hit.file.clone();
    cache.push(hit);
    Some(answer)
}

fn remember(key: Key, file: Option<DiskFile>) {
    let Ok(mut cache) = CACHE.lock() else { return };
    cache.retain(|e| e.key.path != key.path);
    cache.push(Entry { key, file });
    let held = |c: &Vec<Entry>| -> usize {
        c.iter().filter_map(|e| e.file.as_ref()).map(|f| f.bytes.len()).sum()
    };
    while cache.len() > MAX_ENTRIES || (cache.len() > 1 && held(&cache) > MAX_BYTES) {
        cache.remove(0);
    }
}

fn read_whole(path: &Path) -> io::Result<Vec<u8>> {
    WHOLE_READS.with(|c| c.set(c.get() + 1));
    std::fs::read(path)
}

/// Open `path`: a disk image comes back mountable, anything else comes back as
/// the plain bytes read to find that out. The file is read from disk at most
/// once per `(path, length, mtime)` for a disk image.
pub fn open(path: &Path) -> io::Result<Opened> {
    let key = Key::of(path)?;
    if let Some(Some(file)) = lookup(&key) {
        return Ok(Opened::Disk(file));
    }
    // A remembered negative has no bytes, so a caller wanting them reads again;
    // that is what an ordinary story file has always cost.
    let raw = read_whole(path)?;
    match DiskImage::detect(&raw) {
        Some(format) => {
            let file = DiskFile { path: path.to_path_buf(), format, bytes: Arc::new(raw) };
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
pub fn read_bytes(path: &Path) -> io::Result<Vec<u8>> {
    match open(path)? {
        Opened::Disk(file) => Ok(file.bytes().to_vec()),
        Opened::Plain(raw) => Ok(raw),
    }
}

impl MountedDisk {
    /// [`MountedDisk::mount_set`] for an opened image: the same mount, without
    /// the caller having read the file itself.
    pub fn mount_file(
        file: &DiskFile,
        companions: impl FnOnce() -> Vec<Vec<u8>>,
    ) -> Result<MountedDisk, crate::medium::MountError> {
        MountedDisk::mount_set(file.bytes().to_vec(), companions)
    }
}

impl crate::hfs::Hfs {
    /// [`crate::hfs::Hfs::mount`] for an opened image, for the callers that want
    /// the Macintosh volume itself (its resource forks) rather than the
    /// format-neutral [`MountedDisk`].
    pub fn mount_file(file: &DiskFile) -> Result<crate::hfs::Hfs, crate::hfs::HfsError> {
        crate::hfs::Hfs::mount(file.bytes().to_vec())
    }
}
