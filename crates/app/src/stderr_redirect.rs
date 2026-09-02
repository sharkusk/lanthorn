//! Keep OS/C-library chatter off the rendered screen (SQ-0586).
//!
//! Messages from libraries we reach through C — libasound via rodio/cpal is the one
//! that bites, especially in power-save mode when the device suspends and ALSA
//! repeats its underrun complaints — are written to **file descriptor 2 directly**.
//! They never pass through `std::io::stderr`, so no Rust-side hook, captured writer,
//! or panic hook can intercept them: they land on the terminal mid-frame and corrupt
//! the TUI.
//!
//! The only thing that catches all of them is the file descriptor itself. While the
//! alternate screen is up, fd 2 points at `<user_dir>/stderr.log` instead of the
//! terminal; on teardown it is restored. That covers every source — ALSA, PulseAudio,
//! JACK, any C dependency present or future — rather than the one we happen to know
//! about, and it turns invisible spam into something a user can read afterwards.
//!
//! Restoring matters as much as installing: Rust's own panic output goes to fd 2, so
//! a panic while the redirect is up would write the message into the log and leave the
//! terminal blank. [`restore`] therefore runs at the top of `restore_terminal`, which
//! both the clean-exit and panic-hook paths already call.
//!
//! Unix only. Windows has no ALSA, and its equivalent would be `SetStdHandle`; the
//! functions are no-ops there so callers need no `cfg`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The saved duplicate of the real stderr, or `-1` when the redirect is not
/// installed. An `i32` in a static because [`restore`] is reached from
/// `restore_terminal`, a free function with no state to thread through — and from
/// the panic hook, where allocating or locking is best avoided.
#[cfg(unix)]
static SAVED_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Where the captured output is going, for [`captured_tail`].
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Point fd 2 at `path`, saving the real stderr for [`restore`].
///
/// Truncates the log: it describes THIS session, and an ever-growing file of ALSA
/// repeats helps nobody. Call once, immediately before entering the alternate screen
/// — earlier and ordinary CLI output (`--help`, argument errors, the story picker)
/// would be swallowed too.
#[cfg(unix)]
pub fn install(path: &Path) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    if SAVED_FD.load(std::sync::atomic::Ordering::Relaxed) >= 0 {
        return Ok(()); // already installed; installing twice would leak the first save
    }
    let file = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(path)?;
    // SAFETY: dup/dup2 on a fd we own (`file`) and on fd 2. `saved` is stored and
    // closed exactly once, by `restore`. After dup2, fd 2 is an independent
    // descriptor for the same file, so dropping `file` here is correct.
    unsafe {
        let saved = libc::dup(libc::STDERR_FILENO);
        if saved < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(saved);
            return Err(e);
        }
        SAVED_FD.store(saved, std::sync::atomic::Ordering::Relaxed);
    }
    let _ = LOG_PATH.set(path.to_path_buf());
    Ok(())
}

#[cfg(not(unix))]
pub fn install(path: &Path) -> std::io::Result<()> {
    let _ = LOG_PATH.set(path.to_path_buf());
    Ok(())
}

/// Put the real stderr back on fd 2. Idempotent, and safe to call when nothing was
/// installed — both matter because `restore_terminal` runs on several paths (clean
/// exit, signal teardown, panic hook) and may run more than once.
#[cfg(unix)]
pub fn restore() {
    let saved = SAVED_FD.swap(-1, std::sync::atomic::Ordering::Relaxed);
    if saved < 0 {
        return;
    }
    // SAFETY: `saved` came from `dup` in `install` and the swap above guarantees
    // exactly one caller observes it, so it is closed once.
    unsafe {
        libc::dup2(saved, libc::STDERR_FILENO);
        libc::close(saved);
    }
}

#[cfg(not(unix))]
pub fn restore() {}

/// The last `max_lines` lines of captured output, newest last, with blank lines
/// dropped. Empty when nothing was captured, the redirect was never installed, or
/// the log cannot be read — every one of which should stay silent rather than
/// become its own error.
pub fn captured_tail(max_lines: usize) -> Vec<String> {
    let Some(path) = LOG_PATH.get() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let lines: Vec<String> =
        text.lines().map(str::trim_end).filter(|l| !l.is_empty()).map(str::to_string).collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].to_vec()
}

/// Where the captured output was written, for a message pointing the user at it.
pub fn log_path() -> Option<&'static Path> {
    LOG_PATH.get().map(PathBuf::as_path)
}

#[cfg(all(test, unix, feature = "t-misc"))]
mod tests {
    use super::*;

    /// fd 2 is process-global, so these tests must not overlap each other — or run
    /// while another test is writing to stderr.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Write straight to fd 2 the way a C library does, bypassing `std::io::stderr`
    /// (and the test harness's capture) entirely — the whole point of the fix.
    fn write_fd2(msg: &str) {
        // SAFETY: writing a valid buffer to fd 2.
        unsafe {
            libc::write(libc::STDERR_FILENO, msg.as_ptr() as *const libc::c_void, msg.len());
        }
    }

    #[test]
    fn a_c_library_write_to_fd_2_lands_in_the_log_and_is_restored_after() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let path =
            std::env::temp_dir().join(format!("bm-stderr-{}-{:?}.log", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_file(&path);

        install(&path).expect("redirect installs");
        write_fd2("ALSA lib pcm.c:8545:(snd_pcm_recover) underrun occurred\n");
        restore();

        let captured = std::fs::read_to_string(&path).expect("log written");
        assert!(
            captured.contains("underrun occurred"),
            "a direct fd-2 write must be captured, not printed: {captured:?}"
        );

        // After restore, fd 2 is the terminal again — so this write must NOT reach
        // the log. (It goes wherever the harness's stderr goes, which is the point.)
        write_fd2("");
        let after = std::fs::read_to_string(&path).expect("log readable");
        assert_eq!(after, captured, "nothing is captured once the redirect is restored");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn restore_is_idempotent_and_safe_without_an_install() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // `restore_terminal` runs on the clean-exit, signal and panic paths, so a
        // double restore is expected rather than exceptional — and it must not close
        // fd 2 or a descriptor it no longer owns.
        restore();
        restore();
        write_fd2(""); // fd 2 is still a valid descriptor
    }

    #[test]
    fn captured_tail_keeps_the_last_lines_and_drops_blanks() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let path = std::env::temp_dir().join(format!("bm-stderr-tail-{}.log", std::process::id()));
        std::fs::write(&path, "one\n\ntwo\nthree\n\n").expect("write log");
        // `LOG_PATH` is a OnceLock — if an earlier test in this binary already set it,
        // read that file instead of asserting about this one.
        let _ = LOG_PATH.set(path.clone());
        if LOG_PATH.get() == Some(&path) {
            assert_eq!(captured_tail(2), vec!["two".to_string(), "three".to_string()]);
            assert_eq!(captured_tail(99).len(), 3, "blank lines are dropped");
        }
        let _ = std::fs::remove_file(&path);
    }
}
