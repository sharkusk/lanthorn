//! Per-game documents folders: manuals, feelies, maps (SQ-1679).
//!
//! One folder per IFDB game, shared by every release of it and by every player:
//! `<documents root>/<Title> [<TUID>]/` (the root is [`DataRoots::documents`]).
//!
//! **The folder is found by the bracketed TUID alone.** The title is only there
//! to make the folder readable; an IFDB title change renames nothing and cannot
//! produce a second folder, and a folder the user made or renamed by hand is used
//! as it is as long as its name ends in ` [<TUID>]`.
//!
//! Unlinked games (no TUID) have no folder. A relink to another TUID ensures the
//! new game's folder and never touches the old one: those are the user's files.
//!
//! The host API (`list` / `import` / `remove`) works on a folder path obtained
//! from [`documents_dir`] / [`ensure_documents_dir`]. Only the folder's own
//! top-level files are listed: no subdirectories, no dotfiles.

use std::io;
use std::path::{Path, PathBuf};

use crate::data_roots::DataRoots;

/// The longest title part of a folder name, in characters.
const MAX_TITLE_CHARS: usize = 80;

/// Names Windows refuses for a file or folder whatever the extension.
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
    "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Make `title` safe as a folder name on Windows, macOS and Linux. Spaces stay.
/// `/ \ : * ? " < > |` and control characters become `_`; leading dots become `_`
/// (no hidden folders); trailing dots and spaces go (Windows); the result is
/// capped at 80 characters; a Windows device name gets a `_` prefix. An empty
/// result falls back to `fallback` (the story key), sanitised the same way, and
/// finally to `"game"`.
pub fn sanitise_title(title: &str, fallback: &str) -> String {
    let clean = |s: &str| -> String {
        let replaced: String = s
            .chars()
            .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
            .collect();
        let trimmed = replaced.trim_matches(' ').trim_end_matches(['.', ' ']);
        let fixed = match trimmed.strip_prefix('.') {
            Some(rest) => format!("_{rest}"),
            None => trimmed.to_string(),
        };
        let capped: String = fixed.chars().take(MAX_TITLE_CHARS).collect();
        capped.trim_end_matches(['.', ' ']).to_string()
    };
    let mut out = clean(title);
    if out.is_empty() {
        out = clean(fallback);
    }
    if out.is_empty() {
        out = "game".to_string();
    }
    let stem = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        out.insert(0, '_');
    }
    out
}

/// A TUID is IFDB's id: letters, digits, `-` and `_`. Anything else cannot be a
/// real one and must never reach a path.
fn valid_tuid(tuid: &str) -> bool {
    !tuid.is_empty() && tuid.len() <= 64 && tuid.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.to_string())
}

/// The existing folder for `tuid` under `root`: a directory whose name ends in
/// ` [<tuid>]`. If several do (hand-made copies), the alphabetically first wins,
/// so every caller agrees.
pub fn find_dir(root: &Path, tuid: &str) -> Option<PathBuf> {
    if !valid_tuid(tuid) {
        return None;
    }
    let suffix = format!(" [{tuid}]");
    let mut hits: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(&suffix)) && e.path().is_dir())
        .map(|e| e.path())
        .collect();
    hits.sort();
    hits.into_iter().next()
}

/// Where this game's documents folder is, or WOULD be: the existing one when
/// there is one, else `<root>/<Title> [<tuid>]` (nothing is created). `None` for
/// an unusable TUID.
pub fn documents_dir(roots: &DataRoots, tuid: &str, title: &str) -> Option<PathBuf> {
    if !valid_tuid(tuid) {
        return None;
    }
    Some(find_dir(roots.documents(), tuid).unwrap_or_else(|| planned_dir(roots.documents(), tuid, title)))
}

fn planned_dir(root: &Path, tuid: &str, title: &str) -> PathBuf {
    root.join(format!("{} [{tuid}]", sanitise_title(title, "")))
}

/// Where a game's documents folder stands, for the story info panel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Location {
    /// Not linked to IFDB (no TUID): no folder, and nowhere to put one.
    #[default]
    Unlinked,
    /// The folder exists (found by its TUID).
    Exists(PathBuf),
    /// Linked, but no folder yet: the exact path that would be created.
    Missing(PathBuf),
}

/// Look up (never create) the folder state for a game.
pub fn locate(roots: &DataRoots, tuid: Option<&str>, title: &str) -> Location {
    let Some(tuid) = tuid.filter(|t| valid_tuid(t)) else { return Location::Unlinked };
    match find_dir(roots.documents(), tuid) {
        Some(p) => Location::Exists(p),
        None => Location::Missing(planned_dir(roots.documents(), tuid, title)),
    }
}

/// A `file://` URL for an absolute path, percent-encoding everything outside the
/// unreserved set (spaces and the `[ ]` of a documents folder name included).
pub fn file_url(path: &Path) -> String {
    let mut out = String::from("file://");
    let text = path.to_string_lossy().replace('\\', "/");
    if !text.starts_with('/') {
        out.push('/'); // a Windows drive path: file:///C:/...
    }
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// [`documents_dir`], creating the folder when it does not exist. Race-safe: two
/// players or threads ensuring at once end up with one folder.
pub fn ensure_documents_dir(roots: &DataRoots, tuid: &str, title: &str) -> io::Result<PathBuf> {
    if !valid_tuid(tuid) {
        return Err(invalid("not an IFDB id"));
    }
    let root = roots.documents();
    if let Some(found) = find_dir(root, tuid) {
        return Ok(found);
    }
    std::fs::create_dir_all(root)?;
    let mine = planned_dir(root, tuid, title);
    if let Err(e) = std::fs::create_dir(&mine) {
        if !mine.is_dir() {
            return Err(e);
        }
    }
    // Racers name the same folder (same TUID, same title from the same sidecar),
    // so `create_dir` succeeding for one and finding it present for the rest ends
    // with one folder. Answer with what the lookup finds, so every caller agrees.
    find_dir(root, tuid).ok_or_else(|| io::Error::other("documents folder vanished while being created"))
}

/// Ensure the folders for many `(tuid, title)` pairs with one directory read, for
/// a library scan. A no-op when the shared config turned automatic creation off.
/// Failures are skipped: a read-only root must not break a scan.
pub fn ensure_linked<'a>(roots: &DataRoots, games: impl IntoIterator<Item = (&'a str, &'a str)>) {
    if !roots.creates_documents() {
        return;
    }
    let mut have: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Ok(rd) = std::fs::read_dir(roots.documents()) {
        for e in rd.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Some(open) = name.rfind(" [") {
                    if let Some(id) = name[open + 2..].strip_suffix(']') {
                        have.insert(id.to_string());
                    }
                }
            }
        }
    }
    for (tuid, title) in games {
        if valid_tuid(tuid) && have.insert(tuid.to_string()) {
            let _ = ensure_documents_dir(roots, tuid, title);
        }
    }
}

/// What kind of document a file is, by extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Pdf,
    Image,
    Text,
    Other,
}

impl DocKind {
    pub fn of(name: &str) -> DocKind {
        let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "pdf" => DocKind::Pdf,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" => DocKind::Image,
            "txt" | "md" | "rtf" | "html" | "htm" => DocKind::Text,
            _ => DocKind::Other,
        }
    }
}

/// One file in a documents folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocEntry {
    /// The name to show: the file name without its extension.
    pub display_name: String,
    pub kind: DocKind,
    pub size: u64,
    pub path: PathBuf,
    /// Stable within the folder: the file name. What [`remove`] takes.
    pub id: String,
}

fn entry_for(path: PathBuf, size: u64) -> Option<DocEntry> {
    let id = path.file_name()?.to_str()?.to_string();
    let display_name = Path::new(&id).file_stem().and_then(|s| s.to_str()).unwrap_or(&id).to_string();
    Some(DocEntry { display_name, kind: DocKind::of(&id), size, path, id })
}

/// The documents in `dir`, sorted by name (case-insensitive). Top-level regular
/// files only: dotfiles and subdirectories are ignored. A folder that does not
/// exist yet lists as empty, not an error.
pub fn list(dir: &Path) -> io::Result<Vec<DocEntry>> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out: Vec<DocEntry> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            entry_for(e.path(), meta.len())
        })
        .collect();
    out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    Ok(out)
}

/// Copy `src` into `dir` (creating `dir` on demand: an import is an explicit
/// action). Written to a temp file and renamed into place; a name clash is
/// resolved by suffixing (`manual (2).pdf`), never by overwriting.
pub fn import(dir: &Path, src: &Path) -> io::Result<DocEntry> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let name = src.file_name().and_then(|n| n.to_str()).ok_or_else(|| invalid("no file name"))?;
    if name.starts_with('.') {
        return Err(invalid("a dotfile is not a document"));
    }
    if !std::fs::metadata(src)?.is_file() {
        return Err(invalid("not a file"));
    }
    std::fs::create_dir_all(dir)?;
    let stem = Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = Path::new(name).extension().and_then(|s| s.to_str());
    // Claim a free name atomically (`create_new`), so two imports of the same
    // name get two names; then rename the finished copy over the claim.
    let mut n = 1;
    let dest = loop {
        let candidate = match (n, ext) {
            (1, _) => name.to_string(),
            (_, Some(e)) => format!("{stem} ({n}).{e}"),
            (_, None) => format!("{stem} ({n})"),
        };
        let p = dir.join(&candidate);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(_) => break p,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    };
    let tmp = dir.join(format!(".import.part-{}-{}", std::process::id(), NTH.fetch_add(1, Ordering::Relaxed)));
    let result = std::fs::copy(src, &tmp).and_then(|_| std::fs::rename(&tmp, &dest));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&dest);
        return Err(e);
    }
    let size = std::fs::metadata(&dest)?.len();
    entry_for(dest, size).ok_or_else(|| invalid("unusable file name"))
}

/// Delete the file `id` (a name from [`list`]) inside `dir`. An id with a path
/// separator, `..` or a leading dot is refused, so nothing outside the folder (or
/// a subfolder) can be reached; a missing file is `NotFound`.
pub fn remove(dir: &Path, id: &str) -> io::Result<()> {
    if id.is_empty() || id.starts_with('.') || id.contains(['/', '\\', '\0']) || id.contains("..") {
        return Err(invalid("not a document id"));
    }
    let path = dir.join(id);
    if !std::fs::symlink_metadata(&path)?.is_file() {
        return Err(invalid("not a document"));
    }
    std::fs::remove_file(path)
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;
    use crate::data_roots::DocumentsSettings;

    fn roots(tag: &str) -> (PathBuf, DataRoots) {
        let home = crate::scratch_dir(tag);
        let r = DataRoots::resolve(&home, None, None, &DocumentsSettings::default());
        (home, r)
    }

    fn names(root: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(root).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        v.sort();
        v
    }

    #[test]
    fn nasty_titles_become_safe_folder_names() {
        assert_eq!(sanitise_title("A/B: C?", "k"), "A_B_ C_");
        assert_eq!(sanitise_title("Zork I.", "k"), "Zork I");
        assert_eq!(sanitise_title("Trailing   ", "k"), "Trailing");
        assert_eq!(sanitise_title("", "story.z5"), "story.z5");
        assert_eq!(sanitise_title("  ", ""), "game");
        assert_eq!(sanitise_title("...", "x"), "x");
        assert_eq!(sanitise_title(".hidden", "x"), "_hidden");
        assert_eq!(sanitise_title("a\u{0}b\tc", "x"), "a_b_c");
        assert_eq!(sanitise_title("CON", "x"), "_CON");
        assert_eq!(sanitise_title("Bahía de Süd – 魔法", "x"), "Bahía de Süd – 魔法");
        let long = sanitise_title(&"é".repeat(500), "x");
        assert_eq!(long.chars().count(), MAX_TITLE_CHARS);
        let dotty = format!("{}.", "a".repeat(MAX_TITLE_CHARS - 1) + ".");
        assert!(!sanitise_title(&dotty, "x").ends_with('.'));
        for t in ["A/B: C?", "x\\y|z", "<>\"*"] {
            let s = sanitise_title(t, "k");
            assert!(!s.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']), "{s}");
        }
    }

    #[test]
    fn ensure_creates_title_and_tuid_folder_once() {
        let (home, r) = roots("docs-ensure");
        let d = ensure_documents_dir(&r, "abc123", "Zork I").unwrap();
        assert_eq!(d, r.documents().join("Zork I [abc123]"));
        assert!(d.is_dir());
        assert_eq!(ensure_documents_dir(&r, "abc123", "Zork I").unwrap(), d);
        assert_eq!(names(r.documents()), ["Zork I [abc123]"]);
        assert!(documents_dir(&r, "../x", "t").is_none());
        assert!(ensure_documents_dir(&r, "a/b", "t").is_err());
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn lookup_is_by_tuid_so_a_title_change_reuses_the_folder() {
        let (home, r) = roots("docs-retitle");
        std::fs::create_dir_all(r.documents().join("Old Name [tuid1]")).unwrap();
        let d = ensure_documents_dir(&r, "tuid1", "Brand New Name").unwrap();
        assert_eq!(d, r.documents().join("Old Name [tuid1]"));
        assert_eq!(documents_dir(&r, "tuid1", "Brand New Name").unwrap(), d);
        assert_eq!(names(r.documents()), ["Old Name [tuid1]"], "no second folder appears");
        // A different id is a different game.
        let other = ensure_documents_dir(&r, "tuid2", "Brand New Name").unwrap();
        assert_ne!(other, d);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn concurrent_ensures_yield_one_folder() {
        let (home, r) = roots("docs-race");
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let r = r.clone();
                std::thread::spawn(move || ensure_documents_dir(&r, "racy", "Same Title").unwrap())
            })
            .collect();
        let got: Vec<PathBuf> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(got.windows(2).all(|w| w[0] == w[1]), "every caller sees the same folder: {got:?}");
        assert_eq!(names(r.documents()).len(), 1, "{:?}", names(r.documents()));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_named_player_shares_the_folder_with_the_default_player() {
        let home = crate::scratch_dir("docs-players");
        let s = DocumentsSettings::default();
        let default = DataRoots::resolve(&home, None, None, &s);
        let bob = DataRoots::resolve(&home, None, Some("bob"), &s);
        let made = ensure_documents_dir(&bob, "shared1", "Shared").unwrap();
        assert_eq!(find_dir(default.documents(), "shared1"), Some(made.clone()));
        assert_eq!(ensure_documents_dir(&default, "shared1", "Shared").unwrap(), made);
        assert_eq!(names(default.documents()).len(), 1);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn ensure_linked_makes_folders_only_when_opted_in() {
        let home = crate::scratch_dir("docs-linked");
        let on = DataRoots::resolve(&home, None, None, &DocumentsSettings { dir: None, auto_create: true });
        ensure_linked(&on, [("t1", "One"), ("t2", "Two"), ("t1", "One again")]);
        assert_eq!(names(on.documents()), ["One [t1]", "Two [t2]"]);
        let off_home = crate::scratch_dir("docs-linked-off");
        let off = DataRoots::resolve(&off_home, None, None, &DocumentsSettings::default());
        ensure_linked(&off, [("t1", "One")]);
        assert!(!off.documents().exists(), "off creates nothing");
        // A folder made by hand is still found when creation is off.
        std::fs::create_dir_all(off.documents().join("Mine [t9]")).unwrap();
        assert_eq!(documents_dir(&off, "t9", "Whatever"), Some(off.documents().join("Mine [t9]")));
        // And the would-be path is reported, not created.
        assert_eq!(documents_dir(&off, "t1", "One"), Some(off.documents().join("One [t1]")));
        assert!(!off.documents().join("One [t1]").exists());
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(off_home);
    }

    #[test]
    fn list_import_remove_behave() {
        let home = crate::scratch_dir("docs-api");
        let dir = home.join("game [t]");
        assert!(list(&dir).unwrap().is_empty(), "a missing folder lists as empty");
        let src_dir = crate::scratch_dir("docs-api-src");
        let manual = src_dir.join("Manual.pdf");
        std::fs::write(&manual, b"%PDF-1").unwrap();
        let a = import(&dir, &manual).unwrap();
        assert_eq!((a.id.as_str(), a.kind, a.size, a.display_name.as_str()), ("Manual.pdf", DocKind::Pdf, 6, "Manual"));
        assert_eq!(a.path, dir.join("Manual.pdf"));
        let b = import(&dir, &manual).unwrap();
        assert_eq!(b.id, "Manual (2).pdf", "a clash is suffixed");
        let c = import(&dir, &manual).unwrap();
        assert_eq!(c.id, "Manual (3).pdf");
        assert_eq!(std::fs::read(&manual).unwrap(), b"%PDF-1", "the source is untouched");
        std::fs::write(src_dir.join("map.PNG"), b"x").unwrap();
        std::fs::write(src_dir.join("notes"), b"x").unwrap();
        import(&dir, &src_dir.join("map.PNG")).unwrap();
        import(&dir, &src_dir.join("notes")).unwrap();
        // Dotfiles and subdirectories are not listed.
        std::fs::write(dir.join(".hidden"), b"x").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/inner.pdf"), b"x").unwrap();
        let listed = list(&dir).unwrap();
        let ids: Vec<&str> = listed.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["Manual (2).pdf", "Manual (3).pdf", "Manual.pdf", "map.PNG", "notes"]);
        assert_eq!(listed[3].kind, DocKind::Image);
        assert_eq!(listed[4].kind, DocKind::Other);
        assert_eq!(DocKind::of("a.TXT"), DocKind::Text);
        assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().contains(".part")), "no temp left");
        // remove: only inside the folder.
        let outside = home.join("outside.txt");
        std::fs::write(&outside, b"keep").unwrap();
        for bad in ["../outside.txt", "..", "sub/inner.pdf", "/etc/passwd", "a\\b", ".hidden", "", "sub"] {
            assert!(remove(&dir, bad).is_err(), "{bad:?}");
        }
        assert!(outside.exists() && dir.join("sub/inner.pdf").exists() && dir.join(".hidden").exists());
        remove(&dir, "Manual (2).pdf").unwrap();
        assert!(!dir.join("Manual (2).pdf").exists());
        assert_eq!(remove(&dir, "Manual (2).pdf").unwrap_err().kind(), io::ErrorKind::NotFound);
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(src_dir);
    }
}
