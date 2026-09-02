//! Optional filesystem watcher for live style reload. Watches the directory
//! containing style.toml and signals the run loop, which debounces and reloads.

use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// A live style watcher: keeps the `notify` watcher alive and exposes its events.
pub struct StyleWatcher {
    _watcher: RecommendedWatcher,
    pub rx: Receiver<notify::Result<notify::Event>>,
}

/// Start watching the directory that contains `file` (non-recursive), so the file
/// being created/edited/replaced all surface. Returns `None` if the path has no
/// parent or the watcher cannot be created.
pub fn start(file: &Path) -> Option<StyleWatcher> {
    let dir = file.parent()?;
    let (tx, rx) = channel();
    let mut watcher = notify::recommended_watcher(move |res| { let _ = tx.send(res); }).ok()?;
    watcher.watch(dir, RecursiveMode::NonRecursive).ok()?;
    Some(StyleWatcher { _watcher: watcher, rx })
}

impl StyleWatcher {
    /// Also watch `dir` (non-recursively) on this watcher; ignored on error.
    pub fn also_watch(&mut self, dir: &std::path::Path) {
        let _ = self._watcher.watch(dir, RecursiveMode::NonRecursive);
    }
}

/// True when a pending change has settled: dirty and at least `window` elapsed.
pub fn due(dirty_since: Option<Instant>, now: Instant, window: Duration) -> bool {
    match dirty_since {
        Some(t) => now.duration_since(t) >= window,
        None => false,
    }
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn due_only_after_window() {
        let now = Instant::now();
        let win = Duration::from_millis(200);
        assert!(!due(None, now, win), "never due when not dirty");
        assert!(!due(Some(now), now, win), "not due immediately");
        assert!(!due(Some(now), now + Duration::from_millis(100), win), "not due within window");
        assert!(due(Some(now), now + Duration::from_millis(250), win), "due after window");
    }
}
