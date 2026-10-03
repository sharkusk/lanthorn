//! On-demand InvisiClues hint-file downloader (SQ-0445).
//!
//! The story picker's Shift-H key resolves a [`crate::hints::HintDownload`] for
//! the highlighted game and hands its URL + destination here. Each download runs
//! on its own short-lived thread; results drain (non-blocking) into the picker
//! loop, mirroring how `cover::CoverDecoder` and `fetch_worker::Fetcher` feed the
//! UI without an async runtime.
//!
//! Only a handful of these ever run (one per keypress), so a persistent worker
//! thread would be overkill — a thread per request sharing one result channel is
//! simpler and just as non-blocking.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Cap on a downloaded hint file. Real InvisiClues images are tiny (~30–60 KB);
/// this bounds a misbehaving mirror (or an Internet Archive error page) well
/// clear of any legitimate file.
const MAX_HINT: u64 = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

/// What became of one download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintDlOutcome {
    /// The file was fetched, validated, and written to `dest`.
    Done,
    /// The download failed (transport error, non–Z-machine payload, or write
    /// error); `dest` was not created.
    Failed(String),
}

/// Where a download is saved (SQ-1690).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintDest {
    /// Beside the story, at this exact path: the unlinked game's home, as always.
    Beside(PathBuf),
    /// Into the IFDB-linked game's documents folder `dir` (created on demand),
    /// through `documents::import`, so a copy already there is not duplicated and
    /// a name clash with different bytes never overwrites.
    Documents { dir: PathBuf, filename: String },
}

impl HintDest {
    /// The destination for `filename` for the story at `story`: its documents
    /// folder when the game has one (`documents` is its path, existing or not),
    /// else beside the story.
    pub fn for_story(story: &Path, filename: &str, documents: Option<PathBuf>) -> HintDest {
        match documents {
            Some(dir) => HintDest::Documents { dir, filename: filename.to_owned() },
            None => HintDest::Beside(story.with_file_name(filename)),
        }
    }
}

/// One completed download, drained by the picker loop.
#[derive(Debug, Clone)]
pub struct HintDlResult {
    /// The story the hint belongs to — how the picker finds the row to badge.
    pub story: PathBuf,
    /// Which story on `story`, when it is a disk image holding several
    /// (SQ-0859): one compilation is several rows, and the path alone would
    /// badge whichever of them sorted first.
    pub disk_entry: Option<String>,
    /// Where the file was (or would have been) saved: beside the story, or the
    /// file in the documents folder that now holds these bytes.
    pub dest: PathBuf,
    /// The story's display title, for the status line.
    pub title: String,
    pub outcome: HintDlOutcome,
}

/// Spawns hint downloads and drains their results without blocking the UI.
pub struct HintDownloader {
    tx: mpsc::Sender<HintDlResult>,
    rx: mpsc::Receiver<HintDlResult>,
    inflight: usize,
}

impl Default for HintDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl HintDownloader {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, inflight: 0 }
    }

    /// Begin downloading `url` to `dest` (the hint file for `story`, named after
    /// the story `title`). Returns immediately; the result arrives via
    /// [`drain`](Self::drain).
    pub fn start(
        &mut self,
        url: String,
        dest: HintDest,
        story: PathBuf,
        disk_entry: Option<String>,
        title: String,
    ) {
        self.inflight += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let (outcome, dest) = match fetch_bytes(&url) {
                Ok(bytes) => finalize_into(&bytes, dest, &url, &title),
                Err(e) => {
                    let shown = match dest {
                        HintDest::Beside(p) => p,
                        HintDest::Documents { dir, filename } => dir.join(filename),
                    };
                    (HintDlOutcome::Failed(e), shown)
                }
            };
            let _ = tx.send(HintDlResult { story, disk_entry, dest, title, outcome });
        });
    }

    /// Non-blocking drain of every download finished since the last call.
    pub fn drain(&mut self) -> Vec<HintDlResult> {
        let done: Vec<_> = self.rx.try_iter().collect();
        self.inflight = self.inflight.saturating_sub(done.len());
        done
    }

    /// True while at least one download is still in flight.
    pub fn busy(&self) -> bool {
        self.inflight > 0
    }
}

/// GET `url` and return its body bytes. ureq follows the Internet Archive's
/// 302 redirect to the nearest capture automatically.
fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(user_agent())
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = agent.get(url).call().map_err(|e| e.to_string())?;
    resp.body_mut()
        .with_config()
        .limit(MAX_HINT)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

/// Validate `bytes` and save them to `dest`; the path they ended up at comes back
/// with the outcome. A documents-folder save stages the bytes in a private
/// temporary folder and hands them to [`crate::documents::import_with`] (which
/// creates the folder, de-duplicates and writes atomically).
fn finalize_into(bytes: &[u8], dest: HintDest, url: &str, title: &str) -> (HintDlOutcome, PathBuf) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    match dest {
        HintDest::Beside(p) => (finalize_download(bytes, &p), p),
        HintDest::Documents { dir, filename } => {
            let planned = dir.join(&filename);
            if !looks_like_zmachine(bytes) {
                return (HintDlOutcome::Failed("downloaded file is not a Z-machine story".to_string()), planned);
            }
            let stage = std::env::temp_dir().join(format!(
                "lanthorn-hint-{}-{}",
                std::process::id(),
                NTH.fetch_add(1, Ordering::Relaxed)
            ));
            let staged = stage.join(&filename);
            let meta = crate::documents::DocMeta::now(
                Some(format!("InvisiClues hints for {title}")),
                None,
                Some(url.to_owned()),
                true,
            );
            let result = std::fs::create_dir_all(&stage)
                .and_then(|()| std::fs::write(&staged, bytes))
                .and_then(|()| crate::documents::import_with(&dir, &staged, Some(meta)));
            let _ = std::fs::remove_file(&staged);
            let _ = std::fs::remove_dir(&stage);
            match result {
                Ok(done) => (HintDlOutcome::Done, done.entry().path.clone()),
                Err(e) => (HintDlOutcome::Failed(format!("write failed: {e}")), planned),
            }
        }
    }
}

/// Validate `bytes` as a Z-machine story and, if valid, write them to `dest`.
///
/// SQ-0660 hardening: `dest` sits *beside the story*, so an existing file of
/// that name is refused rather than clobbered; and the write goes through a
/// same-directory temp file + rename, so a crash mid-write can never leave a
/// truncated file that the next scan would treat as the hint sidecar.
fn finalize_download(bytes: &[u8], dest: &Path) -> HintDlOutcome {
    if !looks_like_zmachine(bytes) {
        return HintDlOutcome::Failed("downloaded file is not a Z-machine story".to_string());
    }
    if dest.exists() {
        return HintDlOutcome::Failed(format!("{} already exists", dest.display()));
    }
    let tmp = dest.with_file_name(format!(
        "{}.part-{}",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("hint"),
        std::process::id()
    ));
    match std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, dest)) {
        Ok(()) => HintDlOutcome::Done,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            HintDlOutcome::Failed(format!("write failed: {e}"))
        }
    }
}

/// A raw Z-machine story image: version byte in `1..=8`, a full 64-byte
/// header, and header fields consistent with the body we actually received
/// (SQ-0660 — a bare version-byte check accepted any binary junk whose first
/// byte happened to be 1..=8):
///
/// - the declared file length (word at 0x1A, scaled by the version's packing
///   factor: ×2 for v1–3, ×4 for v4–5, ×8 for v6+ — Z-Machine Standards §11.1.6)
///   must not exceed the bytes received (it is 0 in some very early v3 files,
///   which is allowed);
/// - the high-memory (0x04) and static-memory (0x0E) bases must land inside
///   the file, past the 64-byte header.
///
/// Still rejects an HTML error page outright (starts with `<`), and now also
/// a truncated or random-binary body.
fn looks_like_zmachine(bytes: &[u8]) -> bool {
    if bytes.len() < 64 {
        return false;
    }
    let version = bytes[0];
    if !(1..=8).contains(&version) {
        return false;
    }
    let word = |off: usize| u16::from_be_bytes([bytes[off], bytes[off + 1]]) as usize;
    let scale = match version {
        1..=3 => 2,
        4..=5 => 4,
        _ => 8,
    };
    let declared = word(0x1A) * scale;
    if declared != 0 && (declared < 64 || declared > bytes.len()) {
        return false;
    }
    let in_file = |base: usize| (64..=bytes.len()).contains(&base);
    in_file(word(0x04)) && in_file(word(0x0E))
}

fn user_agent() -> String {
    format!("lanthorn/{} (+https://github.com/sharkusk/lanthorn)", env!("CARGO_PKG_VERSION"))
}

#[cfg(all(test, feature = "t-guidance"))]
mod tests {
    use super::*;

    /// A minimal-but-coherent v5 header: high/static memory bases inside the
    /// file, declared length (0x1A, ×4 for v5) matching the 64 bytes.
    fn zmachine_v5() -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[0] = 5;
        b[0x04..0x06].copy_from_slice(&64u16.to_be_bytes()); // high memory base
        b[0x0E..0x10].copy_from_slice(&64u16.to_be_bytes()); // static memory base
        b[0x1A..0x1C].copy_from_slice(&(64u16 / 4).to_be_bytes()); // file length / 4
        b
    }

    #[test]
    fn looks_like_zmachine_accepts_a_story_and_rejects_junk() {
        assert!(looks_like_zmachine(&zmachine_v5()));
        // An HTML error page.
        assert!(!looks_like_zmachine(b"<!DOCTYPE html><html>404</html>"));
        // Too short to hold a header.
        assert!(!looks_like_zmachine(&[5u8; 10]));
        // Version byte out of range.
        let mut bad = zmachine_v5();
        bad[0] = 0;
        assert!(!looks_like_zmachine(&bad));
    }

    /// SQ-0660: a plausible first byte is not enough — the header fields must
    /// make sense for the body received.
    #[test]
    fn looks_like_zmachine_rejects_an_inconsistent_header() {
        // 64 bytes of a plausible version byte and nothing else: the memory-map
        // pointers are zero, i.e. inside the header — junk.
        let mut junk = vec![0u8; 64];
        junk[0] = 5;
        assert!(!looks_like_zmachine(&junk));

        // Declared length claims far more than the body actually holds
        // (a truncated download).
        let mut truncated = zmachine_v5();
        truncated[0x1A..0x1C].copy_from_slice(&0x4000u16.to_be_bytes()); // claims 64 KiB
        assert!(!looks_like_zmachine(&truncated));

        // Static-memory base pointing past the end of the file.
        let mut oob = zmachine_v5();
        oob[0x0E..0x10].copy_from_slice(&0x8000u16.to_be_bytes());
        assert!(!looks_like_zmachine(&oob));

        // A zero declared length (some very early v3 files) is still fine.
        let mut early = zmachine_v5();
        early[0] = 3;
        early[0x1A..0x1C].copy_from_slice(&0u16.to_be_bytes());
        assert!(looks_like_zmachine(&early));
    }

    #[test]
    fn finalize_writes_a_valid_story_and_refuses_an_html_page() {
        let dir = std::env::temp_dir().join(format!("bm-hintdl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let good = dir.join("deadlineinv.z5");
        assert_eq!(finalize_download(&zmachine_v5(), &good), HintDlOutcome::Done);
        assert_eq!(std::fs::read(&good).unwrap(), zmachine_v5());
        // The temp-then-rename write must not leave its .part file behind.
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "only the finished file remains — no .part leftovers"
        );

        let bad = dir.join("junk.z5");
        assert!(matches!(finalize_download(b"<html>oops</html>", &bad), HintDlOutcome::Failed(_)));
        assert!(!bad.exists(), "a rejected payload must not create the file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0660: `dest` sits beside the story, so an existing file of that name
    /// must never be overwritten — the failure names the file.
    #[test]
    fn finalize_refuses_to_overwrite_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("bm-hintdl-ow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let dest = dir.join("deadlineinv.z5");
        std::fs::write(&dest, b"precious existing bytes").unwrap();
        match finalize_download(&zmachine_v5(), &dest) {
            HintDlOutcome::Failed(msg) => {
                assert!(msg.contains("deadlineinv.z5"), "error names the file: {msg}")
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"precious existing bytes",
            "the existing file must survive untouched"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn downloader_starts_empty_and_not_busy() {
        let mut dl = HintDownloader::new();
        assert!(!dl.busy());
        assert!(dl.drain().is_empty());
    }

    /// SQ-1690: a download for an IFDB-linked game lands in its documents folder,
    /// created on demand; a second download of the same bytes writes nothing.
    #[test]
    fn finalize_into_a_documents_folder_creates_it_and_deduplicates() {
        let root = crate::scratch_dir("hintdl-docs");
        let dir = root.join("Deadline [tuid1]");
        let dest = || HintDest::Documents { dir: dir.clone(), filename: "deadlineinv.z5".into() };
        let (outcome, at) = finalize_into(&zmachine_v5(), dest(), "http://x/deadlineinv.z5", "Deadline");
        assert_eq!(outcome, HintDlOutcome::Done);
        assert_eq!(at, dir.join("deadlineinv.z5"));
        assert_eq!(std::fs::read(&at).unwrap(), zmachine_v5());
        let (again, at2) = finalize_into(&zmachine_v5(), dest(), "http://x/deadlineinv.z5", "Deadline");
        assert_eq!(again, HintDlOutcome::Done);
        assert_eq!(at2, at, "the same bytes are not copied again");
        let files: Vec<_> = crate::documents::list(&dir).unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(files, ["deadlineinv.z5"]);
        let (bad, _) = finalize_into(b"<html>", dest(), "u", "t");
        assert!(matches!(bad, HintDlOutcome::Failed(_)));
    }

    /// SQ-1690: an unlinked game keeps the old beside-the-story destination.
    #[test]
    fn dest_is_beside_the_story_unless_the_game_has_a_documents_folder() {
        let story = Path::new("/lib/deadline.z3");
        assert_eq!(HintDest::for_story(story, "deadlineinv.z5", None), HintDest::Beside("/lib/deadlineinv.z5".into()));
        let docs = PathBuf::from("/docs/Deadline [t]");
        assert_eq!(
            HintDest::for_story(story, "deadlineinv.z5", Some(docs.clone())),
            HintDest::Documents { dir: docs, filename: "deadlineinv.z5".into() }
        );
    }
}
