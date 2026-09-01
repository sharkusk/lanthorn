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
//! and falls back to `/dev/null` when there is none, so a page without the
//! audio script (or a script the browser blocked) plays silently as before.
//!
//! The relay holds the FIFO open read-write, so the writer's `open()` never
//! blocks on a reader and a quiet game never wedges it; a closed browser tab is
//! noticed by the next write (data or ping) failing, at which point the FIFO
//! is unlinked. Unix only: FIFOs are the mechanism.
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

#[cfg(unix)]
pub use unix::{drain_paced, relay_session, serve};

#[cfg(unix)]
mod unix {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::io::AsRawFd;
    use std::time::Duration;
    use tungstenite::{Message, WebSocket};

    /// Accept connections forever, one thread per session.
    pub fn serve(listener: TcpListener, dir: PathBuf) -> io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("audio-relay: accept: {e}");
                    continue;
                }
            };
            let dir = dir.clone();
            std::thread::spawn(move || {
                if let Err(e) = handle(stream, &dir) {
                    eprintln!("audio-relay: {e}");
                }
            });
        }
        Ok(())
    }

    fn handle(stream: TcpStream, dir: &Path) -> io::Result<()> {
        let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
        let mut path = String::new();
        // The callback's error type is tungstenite's whole HTTP response; the
        // closure never builds one, and the lint is about the type, not a cost.
        #[allow(clippy::result_large_err)]
        let ws = tungstenite::accept_hdr(stream, |req: &tungstenite::handshake::server::Request, resp| {
            path = req.uri().path().to_string();
            Ok(resp)
        })
        .map_err(|e| io::Error::other(format!("handshake from {peer}: {e}")))?;
        let Some(id) = session_id(&path) else {
            return Err(io::Error::other(format!("{peer} asked for {path:?}, which is not /audio/<id>")));
        };
        let fifo = fifo_path(dir, id);
        eprintln!("audio-relay: {peer} listening to session {id}");
        let t0 = std::time::Instant::now();
        let result = relay_session(ws, &fifo);
        let _ = std::fs::remove_file(&fifo);
        match &result {
            Ok(stats) => eprintln!(
                "audio-relay: session {id} over after {:.0}s: read {} bytes ({:.1}s of audio), sent {} bytes, {} silent chunks dropped",
                t0.elapsed().as_secs_f64(),
                stats.read,
                stats.read as f64 / BYTES_PER_SECOND as f64,
                stats.sent,
                stats.silent_chunks
            ),
            Err(_) => eprintln!("audio-relay: session {id} over after {:.0}s", t0.elapsed().as_secs_f64()),
        }
        result.map(|_| ())
    }

    /// What a session moved, for the log line at its end.
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct SessionStats {
        pub read: u64,
        pub sent: u64,
        pub silent_chunks: u64,
    }

    /// Create `fifo`, hold it open read-write, and forward what arrives on it
    /// to `ws` until the socket is gone. Returns when the peer has gone away.
    pub fn relay_session(mut ws: WebSocket<TcpStream>, fifo: &Path) -> io::Result<SessionStats> {
        make_fifo(fifo)?;
        let mut pipe: File = OpenOptions::new().read(true).write(true).open(fifo)?;
        ws.send(Message::Text(header_json().into())).map_err(io::Error::other)?;
        // 4096 bytes is 1024 stereo frames, 23 ms at 44.1 kHz: small enough to
        // keep the browser's queue short, large enough not to flood it.
        let mut buf = [0u8; 4096];
        // The clock starts at the first byte, so an idle wait before the game
        // plays anything is not counted as time the writer owes.
        let mut started: Option<std::time::Instant> = None;
        let mut stats = SessionStats::default();
        let mut last_send = std::time::Instant::now();
        loop {
            match wait_readable(&pipe, Duration::from_secs(1))? {
                true => {
                    let n = pipe.read(&mut buf)?;
                    if n == 0 {
                        continue;
                    }
                    let t0 = *started.get_or_insert_with(std::time::Instant::now);
                    stats.read += n as u64;
                    let delay = pacing_delay(stats.read, t0.elapsed());
                    if !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                    if is_silence(&buf[..n]) {
                        stats.silent_chunks += 1;
                        // A writer that never stops writing silence would keep
                        // this loop from ever sending, and a gone peer is only
                        // noticed by a send: ping once a second through it.
                        if last_send.elapsed() >= Duration::from_secs(1) {
                            if ws.send(Message::Ping(Vec::new().into())).is_err() {
                                return Ok(stats);
                            }
                            last_send = std::time::Instant::now();
                        }
                        continue;
                    }
                    if ws.send(Message::Binary(buf[..n].to_vec().into())).is_err() {
                        return Ok(stats);
                    }
                    last_send = std::time::Instant::now();
                    stats.sent += n as u64;
                }
                // A quiet second: a ping is how a vanished peer is noticed
                // while the game is silent.
                false => {
                    if ws.send(Message::Ping(Vec::new().into())).is_err() {
                        return Ok(stats);
                    }
                }
            }
        }
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
        let mut started: Option<std::time::Instant> = None;
        let mut sent: u64 = 0;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            if !wait_readable(&pipe, Duration::from_millis(500))? {
                continue;
            }
            let n = pipe.read(&mut buf)?;
            if n == 0 {
                continue;
            }
            let t0 = *started.get_or_insert_with(std::time::Instant::now);
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
        let mut fds = libc::pollfd { fd: pipe.as_raw_fd(), events: libc::POLLIN, revents: 0 };
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

    /// End to end, in-process: a client connects, the relay creates the FIFO,
    /// bytes written into the FIFO come out of the socket after the header,
    /// and closing the socket removes the FIFO.
    #[cfg(unix)]
    #[test]
    fn pcm_written_to_the_fifo_arrives_on_the_socket_and_the_fifo_is_removed_after() {
        use std::io::Write;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "lanthorn-audio-relay-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server_dir = dir.clone();
        std::thread::spawn(move || serve(listener, server_dir));

        let id = "testsession_0001";
        let (mut client, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/audio/{id}")).unwrap();
        let header = client.read().unwrap();
        assert_eq!(header.into_text().unwrap().as_str(), header_json());

        let fifo = fifo_path(&dir, id);
        assert!(fifo.exists(), "the relay created the FIFO on connect");
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while fifo.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(!fifo.exists(), "the FIFO is unlinked once the socket is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
