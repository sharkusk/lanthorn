//! Background archive writer (SQ-1184).
//!
//! The per-turn auto-save used to build and write the whole `.lanthorn`
//! archive synchronously, on the main thread, every turn: re-serialize the
//! mapper to JSON, clone every transcript line + style run, JSON-pretty-print
//! them, re-Deflate every retained history turn's save, PNG-encode every
//! inline image from scratch, then write the zip. All of that is now done by
//! one dedicated writer thread instead. The main thread only gathers cheap
//! inputs (an `Arc` clone of this turn's engine snapshot, `Arc`-cheap image
//! clones, owned copies of small metadata) and hands them off.
//!
//! Mirrors `fetch_worker::Fetcher`: one long-lived `std::thread`, no async
//! runtime. Unlike the fetcher, this queue COALESCES rather than processing
//! every request — a fast burst of turns must never pile up a backlog of
//! stale archive writes, and two writers must never touch `path` at once. A
//! newer enqueued job supersedes an older one that hasn't been picked up yet;
//! only the latest ever gets written. Because each job is a *snapshot of the
//! whole current state* rather than a delta, skipping a superseded job loses
//! nothing a later job doesn't already contain.
//!
//! [`ArchiveWorker::flush`] blocks until every job enqueued before the call
//! has landed on disk (or been superseded by one that itself lands) — called
//! before any *synchronous* save to the same path (an explicit `/save`, the
//! exit-save paths) so it can never race the background writer onto one file.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;

use crate::archive::{build_archive_bytes, DisplayListDto, HistoryReuseStats, Meta, OwnedSessionRecord, PngBlobCache};
use crate::engine::EngineSave;

/// Everything one archive write needs, owned rather than borrowed — built on
/// the main thread from cheap `Arc` clones and small owned copies, then
/// handed to the writer thread, which does the expensive part.
///
/// `mapper_graph` carries only `MapGraph` (already `Clone`), not the whole
/// `Mapper`: `mapper::persist::to_json` only ever reads `.graph`, and the
/// graph is bounded by the explored world rather than by turn count — unlike
/// the transcript and history fields below, cloning it costs nothing that
/// grows with session length.
///
/// `session`'s `transcript`/`kinds`/`runs`/`para` clone as plain `Vec`s and
/// stay O(transcript length) on the calling (main) thread — that clone is a
/// memcpy-class cost (pointer/length copies plus `String` buffer copies), not
/// the CPU-bound JSON-serialize + Deflate + PNG-encode work this worker
/// exists to move off it, and `AppState` stores them as plain owned `Vec`s
/// rather than `Arc`-shared segments, so a cheaper handoff isn't available
/// without a broader storage change (out of scope here). `images` and
/// `history` clone as an `Arc` per element, which is why `AppState::history`
/// is `Vec<Arc<TurnRecord>>` (SQ-1184/SQ-1185) rather than `Vec<TurnRecord>`:
/// a `TurnRecord::save` is a full VM snapshot, and cloning every retained
/// turn's snapshot bytes on the main thread just to hand them to this worker
/// would reintroduce the exact per-turn cost this exists to remove.
pub struct ArchiveJob {
    pub path: PathBuf,
    pub mapper_graph: mapper::graph::MapGraph,
    pub save: Arc<EngineSave>,
    pub screen: Option<zvm::screen::ScreenState>,
    pub aux: BTreeMap<String, Vec<u8>>,
    pub meta: Meta,
    pub session: OwnedSessionRecord,
    pub pictures: Vec<(u8, Vec<u8>)>,
    pub display: Option<DisplayListDto>,
    pub ground: Option<Vec<u8>>,
}

struct Mailbox {
    /// The one job not yet started, tagged with its enqueue generation.
    /// Overwritten wholesale by a newer `enqueue` — that IS the coalescing.
    pending: Option<(u64, ArchiveJob)>,
    shutdown: bool,
    enqueued_gen: u64,
    completed_gen: u64,
}

struct WorkerHandle {
    mailbox: Arc<Mutex<Mailbox>>,
    cv: Arc<Condvar>,
    failures: Arc<Mutex<Vec<String>>>,
    /// Raw-copied/encoded turn counts from the MOST RECENT completed write
    /// (SQ-1202) — overwritten each cycle, not accumulated, since it answers
    /// "how did the last write go" rather than a running total. `failures` is
    /// drained because each entry is a one-shot player notice; this is a
    /// snapshot a caller can re-read any time.
    history_stats: Arc<Mutex<HistoryReuseStats>>,
    _thread: JoinHandle<()>,
}

impl WorkerHandle {
    fn spawn() -> WorkerHandle {
        let mailbox = Arc::new(Mutex::new(Mailbox {
            pending: None,
            shutdown: false,
            enqueued_gen: 0,
            completed_gen: 0,
        }));
        let cv = Arc::new(Condvar::new());
        let failures = Arc::new(Mutex::new(Vec::new()));
        let history_stats = Arc::new(Mutex::new(HistoryReuseStats::default()));
        let (mb, cvh, fh, hs) =
            (Arc::clone(&mailbox), Arc::clone(&cv), Arc::clone(&failures), Arc::clone(&history_stats));
        let thread = std::thread::Builder::new()
            .name("archive-writer".into())
            .spawn(move || worker_loop(mb, cvh, fh, hs))
            .expect("spawn archive writer thread");
        WorkerHandle { mailbox, cv, failures, history_stats, _thread: thread }
    }
}

/// One dedicated writer thread's body: take the latest pending job (draining
/// the mailbox and blocking when there isn't one, exiting only once shutdown
/// is requested AND nothing is left pending — so a final in-flight turn is
/// never dropped), build it, write it, publish completion, repeat.
fn worker_loop(
    mailbox: Arc<Mutex<Mailbox>>,
    cv: Arc<Condvar>,
    failures: Arc<Mutex<Vec<String>>>,
    history_stats: Arc<Mutex<HistoryReuseStats>>,
) {
    // Lives across turns for the life of the thread — an image's PNG bytes
    // never change while its `Arc<RgbaImage>` lives, so this is what makes a
    // stable inline image reuse its prior encode (SQ-1184).
    let mut png_cache = PngBlobCache::default();
    loop {
        let (gen, job) = {
            let mut mb = mailbox.lock().expect("archive mailbox lock poisoned");
            loop {
                if let Some(next) = mb.pending.take() {
                    break next;
                }
                if mb.shutdown {
                    return;
                }
                mb = cv.wait(mb).expect("archive mailbox lock poisoned");
            }
        };
        let path = job.path.clone();
        let result = build_and_write(job, &mut png_cache);
        // Record the failure BEFORE signalling completion: `flush()` returns
        // the moment `completed_gen` reaches its target, and the exit path
        // drains failures right after flushing — a failure pushed after the
        // wake could slip past that drain and be lost at quit. CI's Linux
        // runner hit exactly that window (SQ-1184).
        match result {
            Ok(stats) => {
                // Only the last write's shape matters to a reader (see the
                // field doc on `WorkerHandle::history_stats`), and only worth
                // a line when there was actually history to report on.
                if stats.raw_copied > 0 || stats.encoded > 0 {
                    eprintln!(
                        "lanthorn: archive history at {}: {} turn(s) raw-copied, {} encoded",
                        path.display(), stats.raw_copied, stats.encoded
                    );
                }
                *history_stats.lock().expect("archive history-stats lock poisoned") = stats;
            }
            Err(e) => {
                failures.lock().expect("archive failures lock poisoned")
                    .push(format!("could not save to {}: {}", path.display(), e));
            }
        }
        {
            let mut mb = mailbox.lock().expect("archive mailbox lock poisoned");
            mb.completed_gen = gen;
        }
        // Wake both `flush()` waiters and (harmlessly) anyone re-checking shutdown.
        cv.notify_all();
    }
}

fn build_and_write(job: ArchiveJob, png_cache: &mut PngBlobCache) -> std::io::Result<HistoryReuseStats> {
    // `to_json` only reads `mapper.graph`; a fresh `Mapper` wrapping the
    // cloned graph carries everything it needs (see the `ArchiveJob` doc).
    let mut mapper = mapper::mapper::Mapper::default();
    mapper.graph = job.mapper_graph;
    let session = job.session.as_borrowed();
    let mut stats = HistoryReuseStats::default();
    let bytes = build_archive_bytes(
        &mapper,
        &job.save,
        job.screen.as_ref(),
        &job.aux,
        &job.meta,
        &session,
        &job.pictures,
        job.display.as_ref(),
        job.ground.as_deref(),
        Some(png_cache),
        // The auto-save's own target IS the "previous archive" to reuse from
        // (SQ-1202): this write is about to overwrite exactly that file.
        Some(&job.path),
        Some(&mut stats),
    )?;
    // Evict PNG-cache entries for images no longer present in this (the
    // latest) session snapshot, so a picture that scrolls out of the
    // transcript for good doesn't pin its encoded bytes forever.
    let live: std::collections::HashSet<usize> = session
        .images
        .iter()
        .filter_map(|o| o.as_ref())
        .map(|img| Arc::as_ptr(&img.pixels) as usize)
        .collect();
    png_cache.retain_live(&live);
    crate::storage::atomic_write(&job.path, &bytes)?;
    Ok(stats)
}

/// One archive-writing background thread, coalescing: a newer enqueued job
/// supersedes an older un-started one, so a fast burst of turns never queues
/// more than the latest snapshot, and two writes can never interleave onto
/// one file (SQ-1184).
///
/// Lazily spawns its thread on the first [`enqueue`](Self::enqueue) — most
/// `AppState::default()` construction (hundreds of unit tests) never touches
/// persistence and must not pay a thread spawn for it.
pub struct ArchiveWorker {
    handle: OnceLock<WorkerHandle>,
}

impl Default for ArchiveWorker {
    fn default() -> Self {
        ArchiveWorker { handle: OnceLock::new() }
    }
}

impl std::fmt::Debug for ArchiveWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveWorker").field("spawned", &self.handle.get().is_some()).finish()
    }
}

impl ArchiveWorker {
    pub fn new() -> Self {
        Self::default()
    }

    fn handle(&self) -> &WorkerHandle {
        self.handle.get_or_init(WorkerHandle::spawn)
    }

    /// Queue `job`, superseding any not-yet-started pending job (latest wins;
    /// see the module doc).
    pub fn enqueue(&self, job: ArchiveJob) {
        let h = self.handle();
        let mut mb = h.mailbox.lock().expect("archive mailbox lock poisoned");
        mb.enqueued_gen += 1;
        let gen = mb.enqueued_gen;
        mb.pending = Some((gen, job));
        drop(mb);
        h.cv.notify_all();
    }

    /// Block until every job enqueued before this call has been written (or
    /// superseded by a later one that itself gets written — see the module
    /// doc for why that never loses data). Call this before any synchronous
    /// archive write to the SAME path, and on every exit path, so a quit can
    /// never race the background writer or lose the last turn (SQ-1184).
    ///
    /// A no-op when the worker has never been spawned (nothing was ever
    /// enqueued, so there is nothing to wait for).
    pub fn flush(&self) {
        let Some(h) = self.handle.get() else { return };
        let mut mb = h.mailbox.lock().expect("archive mailbox lock poisoned");
        let target = mb.enqueued_gen;
        while mb.completed_gen < target {
            mb = h.cv.wait(mb).expect("archive mailbox lock poisoned");
        }
    }

    /// Non-blocking drain of write failures accumulated since the last call,
    /// for the caller to surface to the player on a later tick — a
    /// background write cannot push a notice onto `AppState` itself, since it
    /// has no access to it.
    pub fn drain_failures(&self) -> Vec<String> {
        match self.handle.get() {
            Some(h) => std::mem::take(&mut *h.failures.lock().expect("archive failures lock poisoned")),
            None => Vec::new(),
        }
    }

    /// Raw-copied/encoded turn counts from the most recently COMPLETED write
    /// (SQ-1202), for a caller (currently: tests) measuring the effect of
    /// reusing unchanged history turns across archive writes. `Default` (all
    /// zero) when the worker has never completed a write.
    pub fn last_history_reuse_stats(&self) -> HistoryReuseStats {
        match self.handle.get() {
            Some(h) => *h.history_stats.lock().expect("archive history-stats lock poisoned"),
            None => HistoryReuseStats::default(),
        }
    }
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;
    use crate::archive::{Meta, SaveTrigger, SessionRecord, CURRENT_FORMAT_VERSION};

    fn empty_meta() -> Meta {
        Meta {
            format_version: CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: 0,
            saved_at: String::new(),
            location: None,
            score: None,
            trigger: SaveTrigger::HostState,
        }
    }

    fn job_at(path: PathBuf, turns: u32) -> ArchiveJob {
        ArchiveJob {
            path,
            mapper_graph: mapper::mapper::Mapper::default().graph,
            save: Arc::new(EngineSave::new("test", 1, vec![1, 2, 3])),
            screen: None,
            aux: BTreeMap::new(),
            meta: Meta { turns, ..empty_meta() },
            session: SessionRecord::empty().snapshot(),
            pictures: Vec::new(),
            display: None,
            ground: None,
        }
    }

    /// `enqueue` + `flush` round-trips: the archive exists and is readable
    /// once `flush` returns, with no sleep/poll on the caller's part.
    #[test]
    fn enqueue_then_flush_lands_the_write() {
        let dir = crate::scratch_dir("archive-worker-flush");
        let path = dir.join("save.lanthorn");
        let worker = ArchiveWorker::new();

        worker.enqueue(job_at(path.clone(), 1));
        worker.flush();

        assert!(path.exists(), "flush must guarantee the write landed");
        let ac = crate::archive::load_archive(&path).expect("archive readable after flush");
        assert_eq!(ac.meta.turns, 1);
    }

    /// Falsifies the coalescing claim: enqueue several jobs back-to-back
    /// before the worker can start any of them, then flush. Only the LAST
    /// one's content must be on disk — an older superseded job is dropped,
    /// not written and then overwritten (which `flush` alone can't tell
    /// apart from coalescing, but the single archive-writer thread means at
    /// most one job is ever "picked up" between two `enqueue` calls this
    /// close together, so this reliably exercises the supersede path).
    #[test]
    fn a_newer_enqueue_supersedes_an_older_unstarted_job() {
        let dir = crate::scratch_dir("archive-worker-coalesce");
        let path = dir.join("save.lanthorn");
        let worker = ArchiveWorker::new();

        for turns in 1..=5u32 {
            worker.enqueue(job_at(path.clone(), turns));
        }
        worker.flush();

        let ac = crate::archive::load_archive(&path).expect("archive readable after flush");
        assert_eq!(ac.meta.turns, 5, "the latest enqueued job is the one that gets written");
    }

    /// A write failure (an unwritable path) must surface via `drain_failures`
    /// after `flush`, not vanish silently.
    #[test]
    fn a_write_failure_is_drained_not_lost() {
        let dir = crate::scratch_dir("archive-worker-fail");
        // A path whose parent is a FILE, not a directory: the write cannot succeed.
        let blocker = dir.join("blocker-file");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let bad_path = blocker.join("save.lanthorn");

        let worker = ArchiveWorker::new();
        worker.enqueue(job_at(bad_path, 1));
        worker.flush();

        let failures = worker.drain_failures();
        assert_eq!(failures.len(), 1, "the failed write must be reported: {failures:?}");
        assert!(worker.drain_failures().is_empty(), "drain empties the queue");
    }

    /// A worker that was never enqueued into must not block on `flush` (the
    /// lazily-spawned thread never even starts).
    #[test]
    fn flush_on_an_unused_worker_returns_immediately() {
        let worker = ArchiveWorker::new();
        worker.flush(); // must return, not hang
        assert!(worker.drain_failures().is_empty());
    }
}
