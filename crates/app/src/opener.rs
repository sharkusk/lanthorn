//! Hand a URL or a file path to the platform's opener (`open` on macOS,
//! `xdg-open` on Linux, `cmd /C start` on Windows). One implementation for the
//! story picker (SQ-0367: an IFDB link) and the Journal's Documents tab
//! (SQ-1681: a PDF), so there is one place that knows how each OS does it.

/// Open `target` (a URL or an absolute path) with the system's default
/// application. Fire-and-forget: the target is passed as a single argument (no
/// shell), so it needs no escaping, and a missing opener is silently nothing.
pub fn open(target: &str) {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");

    let _ = cmd
        .arg(target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// True in the Docker image's browser mode, where lanthorn runs on a server
/// behind ttyd and there is no local viewer to launch: the image's
/// `LANTHORN_WEB_PORT` is set (see the `Dockerfile`). Anything that would spawn
/// a viewer shows the path instead.
pub fn is_web_mode() -> bool {
    web_mode_from(std::env::var("LANTHORN_WEB_PORT").ok().as_deref())
}

fn web_mode_from(port: Option<&str>) -> bool {
    port.is_some_and(|p| !p.trim().is_empty())
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    #[test]
    fn web_mode_is_the_images_port_variable() {
        assert!(web_mode_from(Some("7681")));
        assert!(!web_mode_from(Some("  ")));
        assert!(!web_mode_from(None));
    }
}
