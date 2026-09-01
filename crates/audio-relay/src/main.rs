//! `lanthorn-audio-relay`: see the crate docs in `lib.rs`.
//!
//! With no arguments it serves. `client <ws-url> [seconds]` connects as a
//! browser would and reports what arrives, for checking a deployment from a
//! shell: `lanthorn-audio-relay client ws://nas:7682/audio/abcdefgh12345678 10`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("sink") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: lanthorn-audio-relay sink <fifo-path>");
                std::process::exit(2);
            };
            std::process::exit(sink(std::path::Path::new(path)));
        }
        Some("client") => {
            let Some(url) = args.get(1) else {
                eprintln!("usage: lanthorn-audio-relay client <ws-url> [seconds]");
                std::process::exit(2);
            };
            let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
            std::process::exit(client(url, secs));
        }
        Some("--help" | "-h") => {
            println!("lanthorn-audio-relay            serve (LANTHORN_WEB_AUDIO_BIND, LANTHORN_AUDIO_DIR)");
            println!("lanthorn-audio-relay sink <fifo>                 drain a FIFO at real time, for sessions nobody listens to");
            println!("lanthorn-audio-relay client <ws-url> [seconds]   connect and count what arrives");
        }
        Some(other) => {
            eprintln!("lanthorn-audio-relay: unknown argument {other:?}");
            std::process::exit(2);
        }
        None => serve_forever(),
    }
}

#[cfg(unix)]
fn serve_forever() {
    let addr = audio_relay::bind_addr();
    let dir = audio_relay::fifo_dir();
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("audio-relay: cannot listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("audio-relay: listening on {addr}, FIFOs under {}", dir.display());
    if let Err(e) = audio_relay::serve(listener, dir) {
        eprintln!("audio-relay: {e}");
        std::process::exit(1);
    }
}

#[cfg(unix)]
fn sink(path: &std::path::Path) -> i32 {
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("audio-relay: {}: {e}", dir.display());
            return 1;
        }
    }
    let never = std::sync::atomic::AtomicBool::new(false);
    match audio_relay::drain_paced(path, &never) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("audio-relay: sink {}: {e}", path.display());
            1
        }
    }
}

#[cfg(not(unix))]
fn sink(_path: &std::path::Path) -> i32 {
    eprintln!("lanthorn-audio-relay: the sink needs FIFOs, which is a Unix mechanism.");
    1
}

#[cfg(not(unix))]
fn serve_forever() {
    eprintln!("lanthorn-audio-relay: serving needs FIFOs, which is a Unix mechanism; use the container.");
    std::process::exit(1);
}

/// Connect, print the header, count PCM bytes for `secs`, and report.
fn client(url: &str, secs: u64) -> i32 {
    let (mut ws, _) = match tungstenite::connect(url) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("connect {url}: {e}");
            return 1;
        }
    };
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_ref() {
        let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let (mut bytes, mut frames) = (0usize, 0usize);
    while std::time::Instant::now() < deadline {
        match ws.read() {
            Ok(tungstenite::Message::Text(t)) => println!("header: {t}"),
            Ok(tungstenite::Message::Binary(b)) => {
                bytes += b.len();
                frames += 1;
            }
            Ok(tungstenite::Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => {
                eprintln!("read: {e}");
                return 1;
            }
        }
    }
    let seconds_of_audio = bytes as f64 / (f64::from(audio_relay::SAMPLE_RATE) * f64::from(audio_relay::CHANNELS) * 2.0);
    println!("{frames} frames, {bytes} bytes, {seconds_of_audio:.2} s of audio in {secs} s");
    0
}
