//! Remembering "no story here" for disk images (SQ-1691).
//!
//! A disk image that mounts but holds no playable story used to be re-examined
//! on every scan, and for a crunched Commodore 64 disk "examined" means a 6502
//! emulation that costs seconds. This remembers that one outcome, install-wide,
//! in `<user_dir>/cache/no-story-disks.json` ([`crate::data_roots::DataRoots::cache`]).
//! It is derived data: deleting the file only costs the next scan its time.
//!
//! **Only a miss is ever stored.** A positive result is cheap to recompute and
//! stale data there would hide or misname a real game; an error (an unreadable
//! file, a mount failure) may be transient and is not "no story" at all, so the
//! caller records nothing for it ([`crate::hints::DiskScan::NoStory`] is the one
//! outcome that reaches [`record_miss`]).
//!
//! # Invalidation
//!
//! An entry is the file's canonical path, its size and its modification time,
//! all of which must still match, under [`DETECTOR_VERSION`]. **Bump
//! `DETECTOR_VERSION` whenever detection logic changes** — a new disk format, a
//! new dialect, a new depacker, anything that could turn an old miss into a hit —
//! so an improvement re-checks every file it previously gave up on. A cache file
//! written under another version is ignored whole. A corrupt or unreadable file
//! is treated as empty.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::data_roots::DataRoots;

/// The version of the "does this disk hold a story" logic. See the module docs:
/// bump it whenever that logic changes.
pub const DETECTOR_VERSION: u32 = 1;

const FILE_NAME: &str = "no-story-disks.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    size: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Misses {
    detector: u32,
    misses: BTreeMap<String, Stamp>,
}

/// One load of each cache file per process; scans ask per file and a miss is
/// recorded rarely, so the file is read once and rewritten only on a new miss.
static MEMO: Mutex<Option<HashMap<PathBuf, Misses>>> = Mutex::new(None);

fn key_and_stamp(path: &Path) -> Option<(String, Stamp)> {
    let canon = std::fs::canonicalize(path).ok()?;
    let meta = std::fs::metadata(&canon).ok()?;
    let since = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    let stamp = Stamp { size: meta.len(), mtime_secs: since.as_secs(), mtime_nanos: since.subsec_nanos() };
    Some((canon.to_string_lossy().into_owned(), stamp))
}

fn load(file: &Path) -> Misses {
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn with_state<R>(file: &Path, f: impl FnOnce(&mut Misses) -> R) -> R {
    let mut guard = MEMO.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(HashMap::new).entry(file.to_path_buf()).or_insert_with(|| load(file));
    f(state)
}

fn is_known_miss_at(file: &Path, path: &Path, version: u32) -> bool {
    let Some((key, stamp)) = key_and_stamp(path) else { return false };
    with_state(file, |m| m.detector == version && m.misses.get(&key) == Some(&stamp))
}

fn record_miss_at(file: &Path, path: &Path, version: u32) {
    let Some((key, stamp)) = key_and_stamp(path) else { return };
    with_state(file, |m| {
        if m.detector != version {
            *m = Misses { detector: version, misses: BTreeMap::new() };
        }
        m.misses.insert(key, stamp);
        write_atomically(file, m);
    });
}

/// Temp file then rename, so a reader never sees half a file. Failure is
/// ignored: the cache is an optimisation.
fn write_atomically(file: &Path, m: &Misses) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let Ok(bytes) = serde_json::to_vec(m) else { return };
    let Some(dir) = file.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = dir.join(format!("{FILE_NAME}.{}-{}.tmp", std::process::id(), NTH.fetch_add(1, Ordering::Relaxed)));
    if std::fs::write(&tmp, bytes).is_err() || std::fs::rename(&tmp, file).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn cache_file(roots: &DataRoots) -> PathBuf {
    roots.cache().join(FILE_NAME)
}

/// Was `path` examined before, unchanged since (size and mtime) and under this
/// [`DETECTOR_VERSION`], and found to hold no story?
pub fn is_known_miss(roots: &DataRoots, path: &Path) -> bool {
    is_known_miss_at(&cache_file(roots), path, DETECTOR_VERSION)
}

/// Remember that `path`, as it is now, holds no story.
pub fn record_miss(roots: &DataRoots, path: &Path) {
    record_miss_at(&cache_file(roots), path, DETECTOR_VERSION);
}

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;

    fn file_in(dir: &Path) -> PathBuf {
        dir.join("cache").join(FILE_NAME)
    }

    fn sample(dir: &Path) -> PathBuf {
        let p = dir.join("a.d64");
        std::fs::write(&p, b"12345").unwrap();
        p
    }

    #[test]
    fn a_recorded_miss_is_known_and_survives_a_reload() {
        let dir = crate::scratch_dir("miss-known");
        let (f, p) = (file_in(&dir), sample(&dir));
        assert!(!is_known_miss_at(&f, &p, 1));
        record_miss_at(&f, &p, 1);
        assert!(is_known_miss_at(&f, &p, 1));
        // A fresh process would load from disk: drop the memo entry.
        MEMO.lock().unwrap().as_mut().unwrap().remove(&f);
        assert!(is_known_miss_at(&f, &p, 1), "the miss came back from the file");
    }

    #[test]
    fn a_changed_file_is_rechecked() {
        let dir = crate::scratch_dir("miss-changed");
        let (f, p) = (file_in(&dir), sample(&dir));
        record_miss_at(&f, &p, 1);
        std::fs::write(&p, b"123456").unwrap(); // size changes
        assert!(!is_known_miss_at(&f, &p, 1), "size change");
        record_miss_at(&f, &p, 1);
        assert!(is_known_miss_at(&f, &p, 1));
        let file = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60)).unwrap();
        assert!(!is_known_miss_at(&f, &p, 1), "mtime change");
    }

    #[test]
    fn a_detector_version_bump_rechecks() {
        let dir = crate::scratch_dir("miss-version");
        let (f, p) = (file_in(&dir), sample(&dir));
        record_miss_at(&f, &p, 1);
        assert!(!is_known_miss_at(&f, &p, 2));
        MEMO.lock().unwrap().as_mut().unwrap().remove(&f);
        assert!(!is_known_miss_at(&f, &p, 2), "and from the file too");
    }

    #[test]
    fn a_corrupt_cache_file_reads_as_empty_and_is_replaced() {
        let dir = crate::scratch_dir("miss-corrupt");
        let (f, p) = (file_in(&dir), sample(&dir));
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"{ not json \xff\x00").unwrap();
        assert!(!is_known_miss_at(&f, &p, 1));
        record_miss_at(&f, &p, 1);
        MEMO.lock().unwrap().as_mut().unwrap().remove(&f);
        assert!(is_known_miss_at(&f, &p, 1));
    }

    #[test]
    fn an_unreadable_path_is_never_a_miss_and_is_not_recorded() {
        let dir = crate::scratch_dir("miss-io");
        let f = file_in(&dir);
        let gone = dir.join("missing.d64");
        record_miss_at(&f, &gone, 1);
        assert!(!is_known_miss_at(&f, &gone, 1));
        assert!(!f.exists(), "nothing was written for a path that could not be stat'd");
    }

    // ---- through the real resolve path ------------------------------------

    /// A 35-track D64 holding one closed PRG that is neither a Scott database nor
    /// a story: a disk that mounts and yields "no story", the shape the 6502
    /// depack is attempted on. `payload` is the program's bytes (load address
    /// first).
    fn d64_with_one_prg(payload: &[u8]) -> Vec<u8> {
        fn spt(t: usize) -> usize {
            match t {
                1..=17 => 21,
                18..=24 => 19,
                25..=30 => 18,
                _ => 17,
            }
        }
        let at = |t: usize, s: usize| ((1..t).map(spt).sum::<usize>() + s) * 256;
        let mut img = vec![0u8; 174_848];
        let bam = at(18, 0);
        img[bam] = 18;
        img[bam + 1] = 1;
        img[bam + 2] = b'A';
        for t in 1..=35 {
            img[bam + 4 * t] = spt(t) as u8;
        }
        let dir = at(18, 1);
        img[dir] = 0;
        img[dir + 1] = 0xff;
        img[dir + 2] = 0x82; // closed PRG
        img[dir + 3] = 1; // first data block: 1/0
        img[dir + 4] = 0;
        img[dir + 5..dir + 21].fill(0xa0);
        img[dir + 5..dir + 9].copy_from_slice(b"JUNK");
        img[dir + 30] = 1; // one block
        let data = at(1, 0);
        assert!(payload.len() <= 254);
        img[data] = 0;
        img[data + 1] = (payload.len() + 1) as u8;
        img[data + 2..data + 2 + payload.len()].copy_from_slice(payload);
        img
    }

    fn crunched_looking_disk(dir: &Path) -> PathBuf {
        let p = dir.join("junk.d64");
        std::fs::write(&p, d64_with_one_prg(&[0x00, 0xc0, 0xea, 0xea, 0x60])).unwrap();
        p
    }

    fn depacks() -> usize {
        crate::hints::DEPACK_CALLS.with(|c| c.get())
    }

    #[test]
    fn a_disk_with_no_story_depacks_once_per_resolve_not_three_times() {
        let dir = crate::scratch_dir("miss-depack-once");
        let roots = DataRoots::single(dir.join("base"));
        let disk = crunched_looking_disk(&dir);
        let before = depacks();
        assert!(crate::picker::resolve_entries(&disk, &roots).is_empty());
        assert_eq!(depacks() - before, 1, "one depack for the one candidate (it was three before SQ-1691)");
    }

    #[test]
    fn a_launch_of_a_disk_with_no_story_depacks_once() {
        let dir = crate::scratch_dir("miss-launch-once");
        let disk = crunched_looking_disk(&dir);
        let before = depacks();
        assert!(crate::hints::load_mounted_story_from(&disk, None).is_err());
        assert_eq!(depacks() - before, 1, "the refusal reuses the scan's own depack");
    }

    #[test]
    fn a_cached_miss_is_skipped_and_a_modified_file_is_rechecked() {
        let dir = crate::scratch_dir("miss-skip");
        let roots = DataRoots::single(dir.join("base"));
        let disk = crunched_looking_disk(&dir);
        let before = depacks();
        assert!(crate::picker::resolve_entries(&disk, &roots).is_empty());
        assert_eq!(depacks() - before, 1);
        assert!(roots.cache().join(FILE_NAME).is_file(), "the miss reached the install-wide cache");
        assert!(crate::picker::resolve_entries(&disk, &roots).is_empty());
        assert_eq!(depacks() - before, 1, "the second resolve skipped the file entirely");
        // Same size, different bytes and a later mtime: still re-checked.
        let mut bytes = std::fs::read(&disk).unwrap();
        bytes[0x1_0000] ^= 1;
        std::fs::write(&disk, bytes).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&disk).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(120)).unwrap();
        assert!(crate::picker::resolve_entries(&disk, &roots).is_empty());
        assert_eq!(depacks() - before, 2, "a modified file is looked at again");
    }

    #[test]
    fn nothing_is_cached_for_a_positive_or_an_unreadable_path() {
        let dir = crate::scratch_dir("miss-positive");
        let roots = DataRoots::single(dir.join("base"));
        // A plain story file resolves (a positive) and is not a disk at all.
        let mut z = vec![0u8; 0x100];
        z[0] = 3;
        let story = dir.join("s.z3");
        std::fs::write(&story, &z).unwrap();
        let _ = crate::picker::resolve_entries(&story, &roots);
        // A path that cannot be read at all is an error, not "no story".
        assert!(crate::picker::resolve_entries(&dir.join("gone.d64"), &roots).is_empty());
        assert!(!roots.cache().join(FILE_NAME).exists(), "no miss was recorded");
        // A real disk with a story on it (skips without the gitignored fixture).
        let hulk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/scott-dialects/c64/QUESTPR1.D64");
        if hulk.is_file() {
            assert!(!crate::picker::resolve_entries(&hulk, &roots).is_empty());
            assert!(!is_known_miss(&roots, &hulk));
            assert!(!roots.cache().join(FILE_NAME).exists(), "a positive is never stored");
        } else {
            eprintln!("SKIP: {} absent (gitignored commercial fixture)", hulk.display());
        }
    }
}
