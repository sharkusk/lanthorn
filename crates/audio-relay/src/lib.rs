//! The audio half of lanthorn's browser delivery.
//!
//! ttyd serves the terminal: bytes over a pty, one lanthorn process per
//! browser connection. Sound does not travel over a pty, so it takes a second
//! channel, and this crate is that channel's server side.
//!
//! The capture side needs no code in lanthorn at all. In the container, ALSA's
//! `default` PCM is a `plug` over the `file` plugin (see `docker/asound.conf`),
//! and the `file` plugin writes whatever the process plays, as S16_LE 44.1 kHz
//! stereo, to the path in `LANTHORN_AUDIO_OUT`. The per-connection wrapper
//! (`docker/serve-session.sh`) points that variable at a FIFO named after the
//! browser's session id, and this relay is the FIFO's reader: one WebSocket
//! per session at `/audio/<id>`, a JSON text frame naming the sample format,
//! then binary frames of raw PCM as they come.
//!
//! Ordering matters and is arranged by the page: the browser opens the audio
//! socket FIRST, which is when the FIFO is created, and only then the terminal
//! socket that spawns lanthorn. The wrapper still waits briefly for the FIFO,
//! and falls back to the paced sink when there is none, so a page without the
//! audio script (or a script the browser blocked) plays silently as before.
//!
//! **The FIFO belongs to the SESSION, not to the socket** (SQ-1328). A served
//! game outlives its browser connection — `docker/serve-session.sh` runs it
//! under `dtach`, keyed by the same id — so a FIFO that died with the socket
//! would leave a live game's ALSA output bound to a pipe nobody reads, which is
//! why a detachable session used to be silent. Instead the relay keeps a
//! per-id session: the FIFO, a reader thread that is ALWAYS reading it, and
//! whichever websocket is attached at the moment. A dropped tab merely detaches
//! — the thread goes on draining at real time and discarding, so the game is
//! never held up — and the next connection with the same id attaches to the
//! same FIFO, gets a fresh header frame, and hears the game it left.
//!
//! The relay holds the FIFO open read-write, so the writer's `open()` never
//! blocks on a reader and the read end never sees the EOF that a quiet moment
//! between two sounds would otherwise look like. That is also why a session
//! cannot notice its own game exiting: with our own write end open there is no
//! EOF to notice. So ending one is somebody else's word, and the word is the
//! FIFO's ABSENCE — `docker/entrypoint.sh`'s reaper unlinks `<id>.pcm` when it
//! ends a session, and the loop below, which is polling anyway, sees the path
//! gone and closes the session out. That is a poll of a CONDITION rather than
//! the receipt of an event: a control message that never arrived would leak a
//! thread and an open pipe for the life of the container, where a missed sweep
//! merely waits for the next one.
//!
//! `LANTHORN_WEB_DETACH=off` takes the other arrangement — one game per
//! websocket — and there the session dies with its socket, FIFO and all,
//! exactly as it did before. Unix only: FIFOs are the mechanism.
//!
//! **The relay is also the clock.** ALSA's `null` slave has no timing: it
//! accepts samples as fast as they are mixed, and rodio mixes silence without
//! end, so an unpaced reader sees the writer run at hundreds of megabytes a
//! second (measured: 10.5 GB in 45 s). The relay reads at the format's real
//! rate instead; the FIFO fills, the writer's `write()` blocks, and the audio
//! thread is paced by this reader as it would be by a sound card. Chunks that
//! are digital silence are consumed but not sent.

use std::io;
use std::path::{Path, PathBuf};

/// The sample format ALSA is configured to write (`docker/asound.conf`), sent
/// as the first, text frame so the page can build its player to match.
pub const SAMPLE_RATE: u32 = 44_100;
pub const CHANNELS: u16 = 2;

/// Bytes per second of the configured format: what the pacing clock counts.
pub const BYTES_PER_SECOND: u64 = SAMPLE_RATE as u64 * CHANNELS as u64 * 2;

/// A chunk that is all zero: digital silence, which the page plays on its own
/// when nothing arrives, so there is no point sending it. It still counts
/// against the pacing clock, since the writer produced it in real time.
pub fn is_silence(chunk: &[u8]) -> bool {
    chunk.iter().all(|&b| b == 0)
}

/// How far ahead of real time `sent` bytes are after `elapsed`, as the sleep
/// that would put them back on the clock; zero when behind or on it.
pub fn pacing_delay(sent: u64, elapsed: std::time::Duration) -> std::time::Duration {
    let due = std::time::Duration::from_secs_f64(sent as f64 / BYTES_PER_SECOND as f64);
    due.saturating_sub(elapsed)
}

/// The JSON header frame.
pub fn header_json() -> String {
    format!("{{\"format\":\"s16le\",\"rate\":{SAMPLE_RATE},\"channels\":{CHANNELS}}}")
}

/// The session id in a request path of the form `/audio/<id>`, or `None` for
/// anything else. An id is 8 to 64 characters of `[A-Za-z0-9_-]`: it names a
/// file, so nothing that could be a path component is accepted, and the page
/// mints 16 random characters, so a shorter one is not one of ours.
pub fn session_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/audio/")?;
    let ok_len = (8..=64).contains(&id.len());
    let ok_chars = id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    (ok_len && ok_chars).then_some(id)
}

/// Where a session's FIFO lives under `dir`.
pub fn fifo_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.pcm"))
}

/// Where FIFOs live: `LANTHORN_AUDIO_DIR`, or `/tmp/lanthorn-audio`. The
/// wrapper script reads the same variable with the same default.
pub fn fifo_dir() -> PathBuf {
    std::env::var_os("LANTHORN_AUDIO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/lanthorn-audio"))
}

/// The address to listen on: `LANTHORN_WEB_AUDIO_BIND`, or `0.0.0.0:7682`.
pub fn bind_addr() -> String {
    std::env::var("LANTHORN_WEB_AUDIO_BIND").unwrap_or_else(|_| "0.0.0.0:7682".to_string())
}

/// Whether a session outlives the websocket that opened it:
/// `LANTHORN_WEB_DETACH`, which is `on` unless it says `off` — the same rule
/// `docker/serve-session.sh` applies to the game, read here so the FIFO and the
/// game agree about how long they live.
pub fn detach_enabled() -> bool {
    std::env::var("LANTHORN_WEB_DETACH").map(|v| v != "off").unwrap_or(true)
}

#[cfg(unix)]
pub use unix::{drain_paced, serve, serve_sessions, SessionStats};

#[cfg(unix)]
mod unix {
    use super::*;
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::io::{AsRawFd, RawFd};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tungstenite::{Message, WebSocket};

    /// The drain's poll period, and so also how soon a reaped FIFO is noticed.
    const TICK: Duration = Duration::from_millis(500);
    /// How long a chunk waits for a socket that is not taking bytes before it is
    /// dropped. Audio is real time: a late chunk is worth less than a game held
    /// up by a browser that has stopped reading.
    const WRITABLE_WAIT: Duration = Duration::from_millis(200);
    /// How long a socket may stay unwritable before it counts as gone. A
    /// suspended tab keeps its connection open and stops reading; without this
    /// it would hold the attachment — and so the session's sound — for as long
    /// as the laptop is shut.
    const STALL_LIMIT: Duration = Duration::from_secs(10);
    /// How often a session with nothing to send pings, which is how a peer that
    /// went away without a `FIN` is noticed.
    const PING_EVERY: Duration = Duration::from_secs(1);

    /// One browser session's audio: the FIFO the game writes to, and whichever
    /// socket is attached at the moment — `None` while the tab is away. Its
    /// reader thread is the only thing that touches the pipe.
    struct Session {
        id: String,
        fifo: PathBuf,
        attached: Mutex<Option<WebSocket<TcpStream>>>,
        /// Whether this session outlives its socket (`LANTHORN_WEB_DETACH`).
        keep_when_detached: bool,
    }

    /// Every live session by id: what makes a reconnect find the FIFO it left
    /// rather than mint a second one.
    #[derive(Default)]
    struct Registry {
        map: Mutex<HashMap<String, Arc<Session>>>,
    }

    /// Accept connections forever, attaching each to its session.
    pub fn serve(listener: TcpListener, dir: PathBuf) -> io::Result<()> {
        serve_sessions(listener, dir, detach_enabled())
    }

    /// `serve`, with the detach policy stated rather than read from the
    /// environment — which is what the tests drive, since the environment is
    /// process-global and they share a process under `cargo test`.
    pub fn serve_sessions(listener: TcpListener, dir: PathBuf, keep_when_detached: bool) -> io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let registry = Arc::new(Registry::default());
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("audio-relay: accept: {e}");
                    continue;
                }
            };
            let dir = dir.clone();
            let registry = Arc::clone(&registry);
            std::thread::spawn(move || {
                if let Err(e) = handle(stream, &dir, &registry, keep_when_detached) {
                    eprintln!("audio-relay: {e}");
                }
            });
        }
        Ok(())
    }

    /// One connection: shake hands, name the session, hand the socket over.
    /// Returns as soon as it is attached — from there the SESSION owns the
    /// socket, because the session is what outlives it.
    fn handle(stream: TcpStream, dir: &Path, registry: &Arc<Registry>, keep_when_detached: bool) -> io::Result<()> {
        let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
        let mut path = String::new();
        // The callback's error type is tungstenite's whole HTTP response; the
        // closure never builds one, and the lint is about the type, not a cost.
        #[allow(clippy::result_large_err)]
        let mut ws = tungstenite::accept_hdr(stream, |req: &tungstenite::handshake::server::Request, resp| {
            path = req.uri().path().to_string();
            Ok(resp)
        })
        .map_err(|e| io::Error::other(format!("handshake from {peer}: {e}")))?;
        let Some(id) = session_id(&path) else {
            return Err(io::Error::other(format!("{peer} asked for {path:?}, which is not /audio/<id>")));
        };
        // Every attach sends the header, not just the first: a page that
        // reconnected has a new socket and a new decoder, and has been told
        // nothing about the format yet.
        ws.send(Message::Text(header_json().into())).map_err(io::Error::other)?;
        let fresh = attach(registry, dir, id, ws, keep_when_detached)?;
        eprintln!(
            "audio-relay: {peer} listening to session {id} ({})",
            if fresh { "new" } else { "reattached" }
        );
        Ok(())
    }

    /// Attach `ws` to session `id`, starting the session if it is not running.
    /// `Ok(true)` when it was started here, `Ok(false)` for a reattach.
    fn attach(
        registry: &Arc<Registry>,
        dir: &Path,
        id: &str,
        ws: WebSocket<TcpStream>,
        keep_when_detached: bool,
    ) -> io::Result<bool> {
        let mut map = lock(&registry.map);
        if let Some(session) = map.get(id) {
            // A reconnect that beat the old socket's failing send: the new one
            // wins, and dropping the old closes it.
            *lock(&session.attached) = Some(ws);
            return Ok(false);
        }
        let session = Arc::new(Session {
            id: id.to_string(),
            fifo: fifo_path(dir, id),
            attached: Mutex::new(Some(ws)),
            keep_when_detached,
        });
        make_fifo(&session.fifo)?;
        let pipe: File = OpenOptions::new().read(true).write(true).open(&session.fifo)?;
        map.insert(session.id.clone(), Arc::clone(&session));
        drop(map);

        let registry = Arc::clone(registry);
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let (stats, reaped) = run_session(&session, pipe);
            // Not when the FIFO was reaped out from under us: by now that path
            // may already name the NEXT session's pipe.
            if !reaped {
                let _ = std::fs::remove_file(&session.fifo);
            }
            let mut map = lock(&registry.map);
            if map.get(&session.id).is_some_and(|s| Arc::ptr_eq(s, &session)) {
                map.remove(&session.id);
            }
            drop(map);
            eprintln!(
                "audio-relay: session {} over after {:.0}s{}: read {} bytes ({:.1}s of audio), sent {} bytes, {} silent chunks dropped, {} chunks the browser was too slow for",
                session.id,
                t0.elapsed().as_secs_f64(),
                if reaped { " (reaped)" } else { "" },
                stats.read,
                stats.read as f64 / BYTES_PER_SECOND as f64,
                stats.sent,
                stats.silent_chunks,
                stats.dropped_chunks
            );
        });
        Ok(true)
    }

    /// What a session moved, for the log line at its end.
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct SessionStats {
        pub read: u64,
        pub sent: u64,
        pub silent_chunks: u64,
        pub dropped_chunks: u64,
    }

    /// The session's whole life: read the FIFO at the format's real rate and
    /// forward it to whoever is attached, discarding what nobody is there to
    /// hear. Returns when the session is over, and whether it ended because its
    /// FIFO was reaped.
    fn run_session(session: &Session, mut pipe: File) -> (SessionStats, bool) {
        // 4096 bytes is 1024 stereo frames, 23 ms at 44.1 kHz: small enough to
        // keep the browser's queue short, large enough not to flood it.
        let mut buf = [0u8; 4096];
        // The clock starts at the first byte, so an idle wait before the game
        // plays anything is not counted as time the writer owes.
        let mut started: Option<Instant> = None;
        let mut stats = SessionStats::default();
        let mut last_send = Instant::now();
        let mut unwritable_since: Option<Instant> = None;
        loop {
            // The only word we get that the game is gone; see the module docs.
            if std::fs::symlink_metadata(&session.fifo).is_err() {
                return (stats, true);
            }
            let readable = match wait_readable(&pipe, TICK) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("audio-relay: session {}: poll: {e}", session.id);
                    return (stats, false);
                }
            };
            if readable {
                let n = match pipe.read(&mut buf) {
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("audio-relay: session {}: read: {e}", session.id);
                        return (stats, false);
                    }
                };
                if n > 0 {
                    let t0 = *started.get_or_insert_with(Instant::now);
                    stats.read += n as u64;
                    // Pacing is what makes this a clock rather than a drain: it
                    // holds the writer to real time whether anybody is
                    // listening or not.
                    let delay = pacing_delay(stats.read, t0.elapsed());
                    if !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                    if is_silence(&buf[..n]) {
                        stats.silent_chunks += 1;
                    } else {
                        match send_attached(session, Message::Binary(buf[..n].to_vec().into()), &mut unwritable_since) {
                            Delivery::Sent => {
                                stats.sent += n as u64;
                                last_send = Instant::now();
                            }
                            Delivery::Slow => stats.dropped_chunks += 1,
                            Delivery::Nobody | Delivery::Gone => {}
                        }
                    }
                }
            }
            // A ping through the same gate: a peer that went away is noticed by
            // a send failing, and a game that is silent (or writing silence at
            // full rate) sends nothing else. The stamp moves either way, so a
            // session with nobody attached does not ping in a tight loop.
            if last_send.elapsed() >= PING_EVERY {
                send_attached(session, Message::Ping(Vec::new().into()), &mut unwritable_since);
                last_send = Instant::now();
            }
            // One game per websocket: the session is the connection, so it ends
            // with it — the pre-SQ-1328 behaviour, kept exactly.
            if !session.keep_when_detached && lock(&session.attached).is_none() {
                return (stats, false);
            }
        }
    }

    /// What became of one frame.
    enum Delivery {
        /// Nobody is attached: the tab is away, and this is simply discarded.
        Nobody,
        Sent,
        /// The socket is not taking bytes; dropped rather than waited on.
        Slow,
        /// The peer is gone, and has been detached.
        Gone,
    }

    /// Send on the attached socket, if there is one and it is taking bytes.
    ///
    /// The writability gate is what keeps a stalled browser from becoming a
    /// stalled GAME: a blocking `send` into a full socket buffer would stop this
    /// thread reading the FIFO, the pipe would fill, and the audio thread on the
    /// other side would block on `write` — the wedge this whole design exists to
    /// avoid, arriving by the front door.
    fn send_attached(session: &Session, msg: Message, unwritable_since: &mut Option<Instant>) -> Delivery {
        let mut guard = lock(&session.attached);
        let Some(fd) = guard.as_ref().map(|ws| ws.get_ref().as_raw_fd()) else {
            *unwritable_since = None;
            return Delivery::Nobody;
        };
        match poll_fd(fd, libc::POLLOUT, WRITABLE_WAIT) {
            Ok(true) => {}
            Ok(false) => {
                let since = *unwritable_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= STALL_LIMIT {
                    *guard = None;
                    *unwritable_since = None;
                    return Delivery::Gone;
                }
                return Delivery::Slow;
            }
            Err(_) => {
                *guard = None;
                *unwritable_since = None;
                return Delivery::Gone;
            }
        }
        *unwritable_since = None;
        let Some(ws) = guard.as_mut() else {
            return Delivery::Nobody;
        };
        if ws.send(msg).is_err() {
            // Dropping the socket closes it, which is the whole of detaching.
            *guard = None;
            return Delivery::Gone;
        }
        Delivery::Sent
    }

    /// A lock that cannot poison the relay: a panicking session thread must not
    /// take the registry — and every other session — down with it.
    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The clock with nobody listening: create `fifo`, hold it open, and
    /// consume what arrives at the format's real rate until `stop` is set.
    /// A session with no browser audio (a plain terminal, audio switched off
    /// in the page) writes here instead of to `/dev/null`, which has no clock
    /// either and let the audio thread spin a whole core (measured at 100%).
    pub fn drain_paced(fifo: &Path, stop: &std::sync::atomic::AtomicBool) -> io::Result<()> {
        make_fifo(fifo)?;
        let mut pipe: File = OpenOptions::new().read(true).write(true).open(fifo)?;
        let mut buf = [0u8; 4096];
        let mut started: Option<Instant> = None;
        let mut sent: u64 = 0;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            if !wait_readable(&pipe, TICK)? {
                continue;
            }
            let n = pipe.read(&mut buf)?;
            if n == 0 {
                continue;
            }
            let t0 = *started.get_or_insert_with(Instant::now);
            sent += n as u64;
            let delay = pacing_delay(sent, t0.elapsed());
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        }
        Ok(())
    }

    fn make_fifo(path: &Path) -> io::Result<()> {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::other("FIFO path holds a NUL"))?;
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
        let rc = unsafe { libc::mkfifo(c.as_ptr(), 0o600) };
        if rc != 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::AlreadyExists {
                return Err(e);
            }
        }
        Ok(())
    }

    /// `poll(2)` the FIFO for readability. `Ok(false)` is a timeout.
    fn wait_readable(pipe: &File, timeout: Duration) -> io::Result<bool> {
        poll_fd(pipe.as_raw_fd(), libc::POLLIN, timeout)
    }

    /// `poll(2)` one descriptor for `events`. `Ok(false)` is a timeout.
    fn poll_fd(fd: RawFd, events: libc::c_short, timeout: Duration) -> io::Result<bool> {
        let mut fds = libc::pollfd { fd, events, revents: 0 };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: `fds` is one valid pollfd and lives across the call.
        let rc = unsafe { libc::poll(&mut fds, 1, ms) };
        match rc {
            0 => Ok(false),
            r if r > 0 => Ok(true),
            _ => {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted { Ok(false) } else { Err(e) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn session_ids_are_file_safe_and_of_our_length() {
        assert_eq!(session_id("/audio/abcdefgh12345678"), Some("abcdefgh12345678"));
        assert_eq!(session_id("/audio/a-b_c-d_e"), Some("a-b_c-d_e"));
        assert_eq!(session_id("/audio/short"), None, "too short to be ours");
        assert_eq!(session_id("/audio/../../etc/passwd"), None, "no path characters");
        assert_eq!(session_id("/audio/has space here"), None);
        assert_eq!(session_id("/other/abcdefgh12345678"), None, "only /audio/");
        assert_eq!(session_id(&format!("/audio/{}", "x".repeat(65))), None, "too long");
    }

    #[test]
    fn the_header_names_the_format_alsa_is_configured_to_write() {
        assert_eq!(header_json(), r#"{"format":"s16le","rate":44100,"channels":2}"#);
    }

    #[test]
    fn silence_is_all_zero_bytes_and_nothing_else() {
        assert!(is_silence(&[0; 4096]));
        assert!(is_silence(&[]));
        let mut one = [0u8; 4096];
        one[4095] = 1;
        assert!(!is_silence(&one));
    }

    /// The pace is the format's byte rate: a second's worth of bytes sent in
    /// half a second owes half a second; sent in a second and a half owes
    /// nothing.
    #[test]
    fn pacing_owes_the_time_the_bytes_ran_ahead_of() {
        use std::time::Duration;
        let d = pacing_delay(BYTES_PER_SECOND, Duration::from_millis(500));
        assert!((d.as_millis() as i64 - 500).abs() <= 1, "{d:?}");
        assert_eq!(pacing_delay(BYTES_PER_SECOND, Duration::from_millis(1500)), Duration::ZERO);
        assert_eq!(pacing_delay(0, Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn fifo_paths_live_under_the_dir_by_id() {
        assert_eq!(fifo_path(Path::new("/tmp/x"), "abcdefgh12345678"), PathBuf::from("/tmp/x/abcdefgh12345678.pcm"));
    }

    /// The sink consumes at the format's rate: a writer pushing a second of
    /// audio through the 64 KB pipe is held back to about a second.
    #[cfg(unix)]
    #[test]
    fn the_sink_holds_a_writer_to_real_time() {
        use std::io::Write;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "lanthorn-audio-sink-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("null.pcm");
        let stop = Arc::new(AtomicBool::new(false));
        let (f, s) = (fifo.clone(), Arc::clone(&stop));
        let sink = std::thread::spawn(move || drain_paced(&f, &s));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !fifo.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut w = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
        let second = vec![1u8; BYTES_PER_SECOND as usize];
        let t0 = std::time::Instant::now();
        w.write_all(&second).unwrap();
        let took = t0.elapsed();
        // The pipe holds ~64 KB of slack ahead of the clock; the rest waits.
        assert!(took >= Duration::from_millis(500), "held back: {took:?}");
        stop.store(true, Ordering::Relaxed);
        drop(w);
        sink.join().unwrap().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A scratch directory unique per CALL — an `AtomicUsize` beside the pid,
    /// not the pid alone. This crate cannot reach `app::scratch_dir`, and under
    /// `cargo test` (which is what CI runs) these cases share one process, so a
    /// pid-keyed name would hand every caller the same directory and each would
    /// delete it under the others (SQ-1131).
    #[cfg(unix)]
    fn scratch(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "lanthorn-audio-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A relay on its own port, serving `dir`. `keep_when_detached` is stated
    /// rather than left to `LANTHORN_WEB_DETACH`, which is process-global.
    #[cfg(unix)]
    fn start_relay(dir: &Path, keep_when_detached: bool) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = dir.to_path_buf();
        std::thread::spawn(move || serve_sessions(listener, dir, keep_when_detached));
        port
    }

    /// Connect as the page does, and check the header frame that every attach
    /// opens with — a reconnecting page has a new decoder and has been told
    /// nothing about the format yet.
    #[cfg(unix)]
    #[allow(clippy::type_complexity)]
    fn attach_to(
        port: u16,
        id: &str,
    ) -> tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>> {
        let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/audio/{id}")).unwrap();
        let header = ws.read().unwrap();
        assert_eq!(header.into_text().unwrap().as_str(), header_json(), "every attach opens with the header frame");
        ws
    }

    #[cfg(unix)]
    fn wait_for(path: &Path, present: bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while path.exists() != present && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        path.exists() == present
    }

    /// End to end, in-process: a client connects, the relay creates the FIFO,
    /// bytes written into the FIFO come out of the socket after the header,
    /// and — with `LANTHORN_WEB_DETACH=off`, one game per websocket — closing
    /// the socket removes the FIFO.
    #[cfg(unix)]
    #[test]
    fn pcm_arrives_on_the_socket_and_with_detach_off_the_fifo_dies_with_it() {
        use std::io::Write;
        let dir = scratch("relay");
        let port = start_relay(&dir, false);

        let id = "testsession_0001";
        let mut client = attach_to(port, id);

        let fifo = fifo_path(&dir, id);
        assert!(wait_for(&fifo, true), "the relay created the FIFO on connect");
        let payload: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        {
            let mut w = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
            w.write_all(&payload).unwrap();
        }
        let mut got = Vec::new();
        while got.len() < payload.len() {
            match client.read().unwrap() {
                tungstenite::Message::Binary(b) => got.extend_from_slice(&b),
                tungstenite::Message::Ping(_) => {}
                other => panic!("unexpected frame {other:?}"),
            }
        }
        assert_eq!(got, payload, "byte-exact, in order");
        // No timing assertion here: on macOS CI this payload came through in
        // 19 ms where 113 ms were due, while the sink test passed on the same
        // runner. The pace is covered by that test and by the arithmetic test;
        // this one covers bytes, order and cleanup.

        client.close(None).unwrap();
        drop(client);
        // The relay notices on its next send: a data frame, or the one-second ping.
        assert!(wait_for(&fifo, false), "the FIFO is unlinked once the socket is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole of SQ-1328, in one flow: the tab goes away, the FIFO stays,
    /// the game keeps playing into a reader that is still consuming at real
    /// time — and the next tab with the same id hears it again.
    #[cfg(unix)]
    #[test]
    fn a_dropped_tab_keeps_its_fifo_draining_and_the_next_one_hears_the_game_again() {
        use std::io::Write;
        let dir = scratch("detach");
        let port = start_relay(&dir, true);
        let id = "detachsession01";

        let client = attach_to(port, id);
        let fifo = fifo_path(&dir, id);
        assert!(wait_for(&fifo, true), "the relay created the FIFO on connect");
        // The game's ALSA output: opened once, and it outlives every socket.
        let mut game = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();

        drop(client);
        std::thread::sleep(Duration::from_millis(1200)); // the ping notices

        // With nobody listening the FIFO must still be READ, or the pipe fills
        // at 64 KB and the audio thread on the other end blocks for good. A
        // second of audio is 176,400 bytes: it can only pass if the drain is
        // still running, and it can only take about a second if it is paced.
        let mut writer = game.try_clone().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let second = vec![9u8; BYTES_PER_SECOND as usize];
            let t0 = std::time::Instant::now();
            writer.write_all(&second).unwrap();
            let _ = tx.send(t0.elapsed());
        });
        let took = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("a detached session must keep draining, or the game is wedged on a full pipe");
        assert!(took >= Duration::from_millis(300), "the drain is still the clock: {took:?}");
        assert!(fifo.exists(), "a detached session keeps its FIFO — it belongs to the game, not the socket");

        // The tab comes back with the same id: a fresh header (checked inside
        // attach_to) and the game it left.
        let mut client = attach_to(port, id);
        let payload: Vec<u8> = (0..8_000u32).map(|i| ((i % 180) + 20) as u8).collect();
        game.write_all(&payload).unwrap();
        let mut got = Vec::new();
        loop {
            // Up to a pipeful of the audio written while nobody was attached is
            // still in flight, and the reattached tab hears the tail of it.
            // Every byte of that is a 9 and none of the payload is.
            while got.first() == Some(&9) {
                got.remove(0);
            }
            if got.len() >= payload.len() {
                break;
            }
            match client.read().unwrap() {
                tungstenite::Message::Binary(b) => got.extend_from_slice(&b),
                tungstenite::Message::Ping(_) => {}
                other => panic!("unexpected frame {other:?}"),
            }
        }
        assert_eq!(got, payload, "streaming resumes byte-exact after a reattach");

        drop(client);
        drop(game);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reap control path: the entrypoint unlinks a killed session's FIFO,
    /// and that absence is the only word this relay ever gets that the game is
    /// gone (it holds the write end itself, so there is no EOF to see). The
    /// session must close out — socket and all — rather than read a pipe
    /// nobody will write to again for the life of the container.
    #[cfg(unix)]
    #[test]
    fn unlinking_the_fifo_ends_the_session_and_frees_the_id() {
        let dir = scratch("reap");
        let port = start_relay(&dir, true);
        let id = "reapedsession01";

        let mut client = attach_to(port, id);
        let fifo = fifo_path(&dir, id);
        assert!(wait_for(&fifo, true), "the relay created the FIFO on connect");

        // Long enough that the session is inside its poll wait rather than on
        // its very first iteration: the absence has to be noticed by the loop,
        // which is the only place it ever can be.
        std::thread::sleep(Duration::from_millis(200));
        std::fs::remove_file(&fifo).unwrap();

        // The session ends, which drops the socket it held: the client sees the
        // connection close rather than hanging on a relay thread that is still
        // there.
        if let tungstenite::stream::MaybeTlsStream::Plain(s) = client.get_ref() {
            s.set_read_timeout(Some(Duration::from_millis(250))).unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut closed = false;
        while std::time::Instant::now() < deadline {
            match client.read() {
                Ok(tungstenite::Message::Close(_)) => {
                    closed = true;
                    break;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => {
                    closed = true;
                    break;
                }
            }
        }
        assert!(closed, "a reaped session closes its socket instead of holding the thread");

        // …and the id is free: the next visitor gets a session of their own,
        // with a new FIFO at the same path.
        let next = attach_to(port, id);
        assert!(wait_for(&fifo, true), "a reaped id can be used again");
        drop(next);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
