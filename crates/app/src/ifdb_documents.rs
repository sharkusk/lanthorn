//! A game's support documents — manuals, feelies, maps — from the links on its
//! IFDB record (SQ-1680), saved into its SQ-1679 documents folder.
//!
//! **What is offered.** The non-game links of the `viewgame?ifiction` record the
//! story downloader already reads ([`parse_document_options`]); the extension of
//! the link's URL decides, never IFDB's `<format>` alone:
//!
//! | URL | kind |
//! |---|---|
//! | `.pdf` | [`LinkKind::Pdf`] |
//! | `.png .jpg .jpeg .gif .webp .tif .tiff` | [`LinkKind::Image`] |
//! | `.txt .doc .rtf .md` | [`LinkKind::Text`] |
//! | `.zip` | [`LinkKind::Archive`] |
//! | no extension, or one nobody knows (`.step1`, `.many`), with `<format>` `text` or `document` | [`LinkKind::Text`] |
//!
//! Left out: anything `<isGame/>`, executables and setup programs (by format, so
//! the `setup` zip on Zork I's record goes), `.hqx`/StuffIt and other archives
//! that are not zip, audio, web pages, and `.inv` (the Shift-H hint downloader
//! owns those). A link under `/games/` whose extension is not a document one is
//! source code, not a document — `hugozork.hug` is the case that decided that
//! rule — so the unknown-extension fallback does not apply there.
//!
//! **Spoilers.** A URL path containing `/solutions/` or `/hints/` is flagged
//! [`DocumentOption::spoiler`]; the chooser tags it so nobody opens a walkthrough
//! by accident.
//!
//! **Zips are opened without being downloaded.** A zip row can expand to its
//! contents, and one entry can be saved on its own, by reading the zip's
//! end-of-central-directory and central directory over HTTP `Range` requests
//! ([`RemoteZip`]). A server that does not answer
//! `Range: bytes=0-0` with 206 and a `Content-Range` ([`RangeProbe`]) gets
//! "contents unknown" and whole-file download only: the archive is NEVER fetched
//! just to be listed.
//!
//! **Limits.** [`MAX_DOWNLOAD`] per file or per inflated entry, judged by the
//! declared size where there is one and again while reading (a zip bomb's
//! declared size is no guarantee); URLs come from the IFDB record only; an entry
//! name with `..`, an absolute path or a drive letter is refused, and only its
//! basename is ever used; and every write goes through [`crate::documents::import`]
//! (the folder made on demand, a clash suffixed, the file renamed into place), so
//! a failed or oversized download leaves nothing behind.
//!
//! **IFDB's terms** (see `ifdb_search`): everything here is one request in flight
//! for one user action, on the existing worker shape, with the existing
//! User-Agent. Nothing is prefetched: a size probe runs for the row the cursor
//! rests on, and a zip's directory is fetched only when the user expands it.

use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use crate::data_roots::DataRoots;
use crate::documents::{DocMeta, Imported};
use crate::ifdb_search::{
    basename_from_url, child_text, one_line, sanitize_basename, subtitle_of, too_large_message,
    IfdbGate, IfdbWorker, RangeProbe, SearchError, SearchSource, MAX_DOWNLOAD,
};

/// Bytes fetched for a text preview.
const PREVIEW_BYTES: u64 = 16 * 1024;
/// Lines shown in a text preview.
pub const PREVIEW_LINES: usize = 20;

// ── Link selection ───────────────────────────────────────────────────────────

/// What a document link is, for the chooser's Kind column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Pdf,
    Image,
    Text,
    Archive,
}

impl LinkKind {
    pub fn label(self) -> &'static str {
        match self {
            LinkKind::Pdf => "pdf",
            LinkKind::Image => "image",
            LinkKind::Text => "text",
            LinkKind::Archive => "zip",
        }
    }
}

/// One non-game link of a game's IFDB record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentOption {
    /// The URL basename, made safe to use as a file name.
    pub filename: String,
    pub url: String,
    pub kind: LinkKind,
    /// IFDB's declared format (`document`, `text`, `guemap`, …), for display.
    pub format: Option<String>,
    pub title: Option<String>,
    pub desc: Option<String>,
    /// The URL is under `/solutions/` or `/hints/`, or the file name, title or
    /// description says walkthrough/solution/hint ([`looks_like_spoiler`]).
    pub spoiler: bool,
}

/// Whether `text` (a file name, path, title or description) names a spoiler:
/// walkthrough (also hyphenated, spaced, `walkthru`), solution(s), hint(s),
/// cheat(s), invisiclues, answer(s), spoiler(s), or "how to win". Case-insensitive
/// and whole-word: everything that is not a letter (`_ . - /`, digits, spaces)
/// separates words, so `zork1_solution.txt` matches and `chintz` does not.
/// Errs towards flagging.
pub fn looks_like_spoiler(text: &str) -> bool {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).collect();
    const WORDS: &[&str] = &[
        "walkthrough", "walkthroughs", "walkthru", "walkthrus", "solution", "solutions", "hint", "hints", "cheat",
        "cheats", "invisiclues", "invisiclue", "answer", "answers", "spoiler", "spoilers",
    ];
    words.iter().any(|w| WORDS.contains(w))
        || words.windows(2).any(|p| p[0] == "walk" && matches!(p[1], "through" | "thru"))
        || words.windows(3).any(|p| p == ["how", "to", "win"])
}

impl DocumentOption {
    /// The one-line description under the filename — the story chooser's rule.
    pub fn subtitle(&self) -> Option<String> {
        subtitle_of(&self.filename, self.title.as_deref(), self.desc.as_deref())
    }
}

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "tif", "tiff"];
const TEXT_EXTS: &[&str] = &["txt", "doc", "rtf", "md"];
/// Extensions that are never a document: audio, web pages, executables, the
/// archive families a zip reader cannot open, and story/source files.
const EXCLUDED_EXTS: &[&str] = &[
    "html", "htm", "inv", "hqx", "sit", "sea", "exe", "com", "msi", "dmg", "bin", "cpt", "tar", "gz", "tgz", "bz2",
    "xz", "7z", "rar", "lzh", "lha", "arj", "z", "mp3", "ogg", "wav", "aif", "aiff", "mid", "midi", "flac", "m4a",
    "mod", "hex", "hug", "inf", "t3", "gam", "taf", "acd", "sna", "tzx", "d64", "adf", "blb", "ulx", "dat",
];

/// The lower-cased path of a URL: no scheme, host, query or fragment.
fn url_path(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.find('/').map_or("", |i| &rest[i..]);
    path.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase()
}

/// Decide whether a link is a document and which kind. `None` for everything
/// the module docs list as left out.
fn classify(url: &str, format: Option<&str>) -> Option<LinkKind> {
    let name = basename_from_url(url)?;
    let format = format.map(str::to_ascii_lowercase);
    if matches!(format.as_deref(), Some("setup" | "executable")) {
        return None;
    }
    let ext = Path::new(&name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let ext = ext.as_deref().unwrap_or("");
    if ext == "pdf" {
        return Some(LinkKind::Pdf);
    }
    if IMAGE_EXTS.contains(&ext) {
        return Some(LinkKind::Image);
    }
    if TEXT_EXTS.contains(&ext) {
        return Some(LinkKind::Text);
    }
    if ext == "zip" {
        return Some(LinkKind::Archive);
    }
    if EXCLUDED_EXTS.contains(&ext) || crate::picker::has_story_ext(Path::new(&name)) {
        return None;
    }
    // No extension, or one that names nothing we know (`zorkI.step1`,
    // `hints.many`): IFDB's own word for it decides, and a `/games/` path is
    // source code.
    let texty = matches!(format.as_deref(), Some("text" | "document"));
    (texty && !url_path(url).contains("/games/")).then_some(LinkKind::Text)
}

/// A file name that is safe on Windows, macOS and Linux: [`sanitize_basename`]
/// plus the characters Windows refuses, trailing dots and spaces dropped, and a
/// device name (`CON`) defanged.
fn document_filename(raw: &str) -> Option<String> {
    let base = sanitize_basename(raw)?;
    let replaced: String =
        base.chars().map(|c| if matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let trimmed = replaced.trim_end_matches(['.', ' ']).to_string();
    if trimmed.is_empty() || trimmed.starts_with('.') {
        return None;
    }
    let stem = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    Some(if crate::documents::WINDOWS_RESERVED.contains(&stem.as_str()) { format!("_{trimmed}") } else { trimmed })
}

/// The documents among a viewgame iFiction record's download links, in record
/// order. A record with none, or one that does not parse, yields an empty vec.
pub fn parse_document_options(xml: &[u8]) -> Vec<DocumentOption> {
    let Ok(text) = std::str::from_utf8(xml) else { return Vec::new() };
    let Ok(doc) = roxmltree::Document::parse(text) else { return Vec::new() };
    doc.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "link")
        .filter_map(|link| {
            let url = child_text(link, "url")?;
            if link.children().any(|n| n.is_element() && n.tag_name().name() == "isGame") {
                return None;
            }
            let format = child_text(link, "format");
            let kind = classify(&url, format.as_deref())?;
            let filename = document_filename(&basename_from_url(&url)?)?;
            let path = url_path(&url);
            let title = child_text(link, "title").and_then(|t| one_line(&t));
            let desc = child_text(link, "desc").and_then(|d| one_line(&d));
            Some(DocumentOption {
                url,
                kind,
                format,
                spoiler: path.contains("/solutions/")
                    || path.contains("/hints/")
                    || looks_like_spoiler(&filename)
                    || title.as_deref().is_some_and(looks_like_spoiler)
                    || desc.as_deref().is_some_and(looks_like_spoiler)
                    || child_text(link, "label").is_some_and(|l| looks_like_spoiler(&l)),
                title,
                desc,
                filename,
            })
        })
        .collect()
}

// ── Names ────────────────────────────────────────────────────────────────────

/// The basename to save a zip entry as, or `None` when its name is not one to
/// trust: a `..` component, an absolute path, a drive letter or a NUL is
/// refused outright (never "fixed"), and what remains is cut to its last
/// component and made safe by [`document_filename`].
pub fn safe_entry_basename(path: &str) -> Option<String> {
    if path.contains('\0') {
        return None;
    }
    let norm = path.replace('\\', "/");
    let bytes = norm.as_bytes();
    if norm.starts_with('/') || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':') {
        return None;
    }
    if norm.split('/').any(|c| c == "..") {
        return None;
    }
    document_filename(norm.trim_end_matches('/').rsplit('/').next()?)
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Does a file name read as this game's own? Case, spaces and punctuation are
/// ignored (`ZorkI` is `Zork I`), and the part of a title before a colon counts
/// on its own (`Zork I: The Great Underground Empire`).
pub fn matches_title(filename: &str, game_title: &str) -> bool {
    let stem = Path::new(filename).file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
    let stem = squash(stem);
    if stem.is_empty() {
        return false;
    }
    let head = game_title.split([':', '—']).next().unwrap_or(game_title);
    [game_title, head].iter().any(|t| squash(t) == stem)
}

// ── Reading a remote zip ─────────────────────────────────────────────────────
//
// The zip is read by hand rather than through `zip::ZipArchive`, which was the
// plan and which does not do what the plan needed: zip 2.4.2's `ZipArchive::new`
// validates EVERY central-directory record by seeking to that entry's local
// header (`find_data_start`, called from `central_header_to_zip_file`), so
// opening a 57-entry archive over a range adapter costs a request per entry, and
// a 1,000-entry one a thousand. Parsing the two structures directly is small and
// costs what it should: one request for the tail (the end-of-central-directory
// record and, nearly always, the whole central directory with it), then two per
// entry actually read (its local header, then its bytes). Names AND sizes come
// from the directory, which `ZipArchive` could only give by touching every
// entry.

/// What the first request takes: the largest end-of-central-directory record
/// (a 22-byte record plus a comment of up to 65,535 bytes).
const TAIL: u64 = 65_535 + 22;
/// The biggest central directory we will fetch when it is not already in the tail.
const MAX_DIRECTORY: u64 = 16 * 1024 * 1024;
const EOCD_SIG: [u8; 4] = *b"PK\x05\x06";
const ZIP64_LOCATOR_SIG: [u8; 4] = *b"PK\x06\x07";
const CENTRAL_SIG: [u8; 4] = *b"PK\x01\x02";
const LOCAL_SIG: [u8; 4] = *b"PK\x03\x04";

fn u16_at(b: &[u8], o: usize) -> usize {
    u16::from_le_bytes([b[o], b[o + 1]]) as usize
}

fn u32_at(b: &[u8], o: usize) -> u64 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as u64
}

fn archive_err(what: &str) -> SearchError {
    SearchError::Archive(what.to_string())
}

/// One saveable file inside a remote zip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    /// Index into the archive, what [`RemoteZip::read_entry`] takes.
    pub index: usize,
    /// The name inside the archive, directories and all.
    pub path: String,
    /// The safe basename it would be saved as.
    pub name: String,
    /// Uncompressed size, from the central directory.
    pub size: u64,
    /// The entry's own name says walkthrough/solution/hint ([`looks_like_spoiler`]).
    pub spoiler: bool,
}

/// What a zip row holds, as far as the server lets us know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipListing {
    Entries(Vec<ZipEntry>),
    /// No way to look inside (the server serves no ranges, or the archive is
    /// zip64): whole-file download only.
    Unknown,
}

/// A central-directory record.
struct Raw {
    path: String,
    flags: usize,
    method: usize,
    crc: u32,
    compressed: u64,
    size: u64,
    offset: u64,
}

/// A zip read over HTTP `Range` requests (see the section note).
pub struct RemoteZip<'a> {
    source: &'a dyn SearchSource,
    url: &'a str,
    total: u64,
    raw: Vec<Raw>,
}

impl<'a> RemoteZip<'a> {
    /// Open `url` as a zip, `Ok(None)` when it cannot be looked inside: it serves
    /// no ranges (see [`RangeProbe`]) or it is zip64. It is never downloaded
    /// whole to be opened.
    pub fn open(source: &'a dyn SearchSource, url: &'a str) -> Result<Option<Self>, SearchError> {
        let total = match source.probe_range(url)? {
            RangeProbe::Unsupported { .. } => return Ok(None),
            RangeProbe::Supported { total } => total,
        };
        let not_zip = || archive_err("not a zip file");
        if total < 22 {
            return Err(not_zip());
        }
        let tail_start = total.saturating_sub(TAIL);
        let tail = fetch_exact(source, url, tail_start, total - tail_start)?;
        let at = (0..=tail.len() - 22)
            .rev()
            .find(|&i| tail[i..i + 4] == EOCD_SIG && i + 22 + u16_at(&tail, i + 20) <= tail.len())
            .ok_or_else(not_zip)?;
        if at >= 20 && tail[at - 20..at - 16] == ZIP64_LOCATOR_SIG {
            return Ok(None);
        }
        let (count, dir_size, dir_at) = (u16_at(&tail, at + 10), u32_at(&tail, at + 12), u32_at(&tail, at + 16));
        if count == 0xFFFF || dir_size == 0xFFFF_FFFF || dir_at == 0xFFFF_FFFF {
            return Ok(None);
        }
        if dir_at + dir_size > total || dir_size > MAX_DIRECTORY {
            return Err(archive_err("damaged zip directory"));
        }
        let dir = if dir_at >= tail_start {
            let from = (dir_at - tail_start) as usize;
            tail[from..from + dir_size as usize].to_vec()
        } else {
            fetch_exact(source, url, dir_at, dir_size)?
        };
        let raw = parse_directory(&dir, count)?;
        Ok(Some(Self { source, url, total, raw }))
    }

    /// The files in the archive that could be saved: directories, entries that
    /// cannot be read (password-protected, a compression we have no decoder for)
    /// and entries whose names are not safe ([`safe_entry_basename`]) are left out.
    pub fn entries(&self) -> Vec<ZipEntry> {
        self.raw
            .iter()
            .enumerate()
            .filter_map(|(index, r)| {
                if r.path.ends_with('/') || r.path.ends_with('\\') || r.flags & 1 != 0 || !matches!(r.method, 0 | 8) {
                    return None;
                }
                Some(ZipEntry {
                    index,
                    path: r.path.clone(),
                    name: safe_entry_basename(&r.path)?,
                    size: r.size,
                    spoiler: looks_like_spoiler(&r.path),
                })
            })
            .collect()
    }

    /// The compressed bytes of entry `r`, up to `want` of them.
    fn data(&self, r: &Raw, want: u64) -> Result<Vec<u8>, SearchError> {
        let header = fetch_exact(self.source, self.url, r.offset, 30)?;
        if header[..4] != LOCAL_SIG {
            return Err(archive_err("damaged zip entry"));
        }
        let start = r.offset + 30 + u16_at(&header, 26) as u64 + u16_at(&header, 28) as u64;
        if start + r.compressed > self.total {
            return Err(archive_err("damaged zip entry"));
        }
        fetch_exact(self.source, self.url, start, want.min(r.compressed))
    }

    fn entry(&self, index: usize) -> Result<&Raw, SearchError> {
        let r = self.raw.get(index).ok_or_else(|| archive_err("no such entry"))?;
        if r.flags & 1 != 0 {
            return Err(archive_err("password-protected"));
        }
        if !matches!(r.method, 0 | 8) {
            return Err(archive_err("compressed in a way lanthorn cannot read"));
        }
        Ok(r)
    }

    /// Inflate one entry, refusing one over `cap` — by its declared size first,
    /// then while reading, because the declared size of a zip bomb is a lie — and
    /// one whose checksum does not match.
    pub fn read_entry(&self, index: usize, cap: u64) -> Result<(String, Vec<u8>), SearchError> {
        let r = self.entry(index)?;
        let name = safe_entry_basename(&r.path).ok_or(SearchError::NoFilename)?;
        if r.size > cap || r.compressed > cap {
            return Err(SearchError::TooLarge);
        }
        let out = inflate(r.method, &self.data(r, r.compressed)?, cap)?;
        let mut crc = flate2::Crc::new();
        crc.update(&out);
        if crc.sum() != r.crc {
            return Err(archive_err("damaged zip entry (checksum mismatch)"));
        }
        Ok((name, out))
    }

    /// The first `n` bytes of one entry, for a preview: only the start of its
    /// compressed bytes is fetched, and a stream cut short is not an error.
    pub fn read_entry_prefix(&self, index: usize, n: u64) -> Result<Vec<u8>, SearchError> {
        let r = self.entry(index)?;
        let bytes = self.data(r, if r.method == 0 { n } else { 64 * 1024 })?;
        let mut out = Vec::new();
        let mut reader: Box<dyn Read> = match r.method {
            0 => Box::new(bytes.as_slice()),
            _ => Box::new(flate2::read::DeflateDecoder::new(bytes.as_slice())),
        };
        // A truncated deflate stream ends in an error after the bytes it did
        // produce, which is exactly what a preview wants.
        let _ = (&mut reader).take(n).read_to_end(&mut out);
        Ok(out)
    }
}

/// `len` bytes at `start`, or an error if the server sent fewer.
fn fetch_exact(source: &dyn SearchSource, url: &str, start: u64, len: u64) -> Result<Vec<u8>, SearchError> {
    let data = source.fetch_range(url, start, len)?;
    if data.len() as u64 != len {
        return Err(archive_err("the server sent a short reply"));
    }
    Ok(data)
}

fn parse_directory(dir: &[u8], count: usize) -> Result<Vec<Raw>, SearchError> {
    let bad = || archive_err("damaged zip directory");
    let mut raw = Vec::with_capacity(count);
    let mut at = 0usize;
    for _ in 0..count {
        let fixed = dir.get(at..at + 46).ok_or_else(bad)?;
        if fixed[..4] != CENTRAL_SIG {
            return Err(bad());
        }
        let (name_len, extra_len, comment_len) = (u16_at(fixed, 28), u16_at(fixed, 30), u16_at(fixed, 32));
        let name = dir.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
        let (compressed, size, offset) = (u32_at(fixed, 20), u32_at(fixed, 24), u32_at(fixed, 42));
        if compressed == 0xFFFF_FFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
            return Err(archive_err("zip64 entries are not supported"));
        }
        raw.push(Raw {
            path: String::from_utf8_lossy(name).into_owned(),
            flags: u16_at(fixed, 8),
            method: u16_at(fixed, 10),
            crc: u32_at(fixed, 16) as u32,
            compressed,
            size,
            offset,
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(raw)
}

/// Decode `data` (stored, or raw deflate) refusing more than `cap` bytes out.
fn inflate(method: usize, data: &[u8], cap: u64) -> Result<Vec<u8>, SearchError> {
    let mut out = Vec::new();
    let read = match method {
        0 => data.take(cap + 1).read_to_end(&mut out),
        _ => flate2::read::DeflateDecoder::new(data).take(cap + 1).read_to_end(&mut out),
    };
    read.map_err(|_| archive_err("damaged zip entry"))?;
    if out.len() as u64 > cap {
        return Err(SearchError::TooLarge);
    }
    Ok(out)
}

/// List the zip at `url` without downloading it.
pub fn list_zip(source: &dyn SearchSource, url: &str) -> Result<ZipListing, SearchError> {
    Ok(match RemoteZip::open(source, url)? {
        Some(zip) => ZipListing::Entries(zip.entries()),
        None => ZipListing::Unknown,
    })
}

/// A size for display: `812 B`, `12 KB`, `1.4 MB`.
pub fn format_size(n: u64) -> String {
    const KB: f64 = 1024.0;
    let f = n as f64;
    if n < 1024 {
        format!("{n} B")
    } else if f < KB * KB {
        format!("{:.0} KB", f / KB)
    } else {
        format!("{:.1} MB", f / (KB * KB))
    }
}

// ── Previews ─────────────────────────────────────────────────────────────────

/// Can a name be previewed as text? Plain text and Markdown, or no extension.
pub fn is_previewable(filename: &str) -> bool {
    match Path::new(filename).extension().and_then(|e| e.to_str()) {
        None => true,
        Some(e) => ["txt", "md", "nfo", "text"].iter().any(|t| e.eq_ignore_ascii_case(t)),
    }
}

/// The first [`PREVIEW_LINES`] lines of `bytes` as display text: lossy UTF-8,
/// controls other than the line break dropped, long lines cut.
pub fn preview_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .take(PREVIEW_LINES)
        .map(|l| {
            let clean: String = l.chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect();
            clean.chars().take(200).collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Fetch the preview of a top-level link (`entry` `None`) or of one entry of the
/// zip at `url`.
pub fn fetch_preview(source: &dyn SearchSource, url: &str, entry: Option<usize>) -> Result<String, SearchError> {
    let bytes = match entry {
        None => source.fetch_range(url, 0, PREVIEW_BYTES)?,
        Some(i) => match RemoteZip::open(source, url)? {
            Some(zip) => zip.read_entry_prefix(i, PREVIEW_BYTES)?,
            None => return Err(SearchError::Archive("this server cannot show inside a zip".into())),
        },
    };
    Ok(preview_text(&bytes))
}

// ── Saving ───────────────────────────────────────────────────────────────────

/// Write `bytes` as `name` into `dir` through [`crate::documents::import`]: the
/// folder is made if missing, a clash is suffixed, and bytes the folder already
/// holds are not written again. `meta` goes into the folder's index (SQ-1687). The bytes go to a scratch
/// file first, so nothing partial is ever in the folder.
pub fn save_document(dir: &Path, name: &str, bytes: &[u8], meta: Option<DocMeta>) -> io::Result<Imported> {
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let scratch = std::env::temp_dir().join(format!(
        "lanthorn-doc-{}-{}",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&scratch)?;
    let src = scratch.join(name);
    let result = std::fs::write(&src, bytes).and_then(|()| crate::documents::import_with(dir, &src, meta));
    // File then (empty) directory, never a recursive delete.
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_dir(&scratch);
    result
}

// ── What is already saved ────────────────────────────────────────────────────

/// Whether a row of the documents chooser is already in the game's folder, for a
/// host to say so before downloading (SQ-1699).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedState {
    /// Nothing in the folder came from this link or entry (or it was deleted).
    NotSaved,
    /// The folder holds the file, at the size it was saved with (or the index
    /// recorded no size, an entry from before sizes were kept).
    Saved,
    /// The folder holds the file but its size is no longer the recorded one: it
    /// was edited or replaced, so downloading again is a real choice.
    Changed,
}

/// The state of the link at `url` (`zip_entry` `None`) or of the entry
/// `zip_entry` (its [`ZipEntry::path`]) of the zip at `url`, against `docs`, the
/// folder's [`crate::documents::list`]. No network and no disk: the index's
/// `source_url` / `zip_entry` say where a file came from, `docs` says it is still
/// there and how big it is. Any file that is [`Saved`](SavedState::Saved) makes
/// the answer `Saved`; failing that, any `Changed` one makes it `Changed`.
pub fn saved_state(docs: &[crate::documents::DocEntry], url: &str, zip_entry: Option<&str>) -> SavedState {
    let mut state = SavedState::NotSaved;
    for d in docs {
        if d.source_url.as_deref() != Some(url) || d.zip_entry.as_deref() != zip_entry {
            continue;
        }
        match d.recorded_size {
            Some(r) if r != d.size => state = SavedState::Changed,
            _ => return SavedState::Saved,
        }
    }
    state
}

// ── The worker ───────────────────────────────────────────────────────────────

/// A row of the chooser: link `i`, or entry `j` of link `i`'s zip.
pub type RowKey = (usize, Option<usize>);

/// One thing to save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadItem {
    /// A whole link (a file, or a whole zip).
    File { url: String, filename: String, info: LinkInfo },
    /// One entry of the zip at `zip_url`.
    Entry { zip_url: String, index: usize, info: LinkInfo },
}

/// What the index keeps of a link besides its URL (SQ-1687).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkInfo {
    pub title: Option<String>,
    pub desc: Option<String>,
    pub spoiler: bool,
    /// For a zip entry saved on its own: its path inside the zip (what
    /// [`saved_state`] matches on). `None` for a whole link.
    pub entry: Option<String>,
}

impl LinkInfo {
    /// A top-level link's own title, description and spoiler flag.
    pub fn of_link(o: &DocumentOption) -> LinkInfo {
        LinkInfo { title: o.title.clone(), desc: o.desc.clone(), spoiler: o.spoiler, entry: None }
    }

    /// A zip entry's: its own name, the zip link's description with
    /// ` (from <zip filename>)` appended, and the entry's own spoiler flag.
    pub fn of_entry(zip: &DocumentOption, entry: &ZipEntry) -> LinkInfo {
        let from = format!("(from {})", zip.filename);
        let desc = match zip.desc.as_deref() {
            Some(d) => format!("{d} {from}"),
            None => from,
        };
        LinkInfo { title: Some(entry.name.clone()), desc: Some(desc), spoiler: entry.spoiler, entry: Some(entry.path.clone()) }
    }
}

/// A unit of work for [`DocumentWorker`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocJob {
    /// The game's document links, by IFDB id.
    Resolve { tuid: String },
    /// Size and range support of link `link`.
    Probe { link: usize, url: String },
    /// The contents of the zip that is link `link`.
    ListZip { link: usize, url: String },
    /// A text preview of row `key`: the file at `url`, or entry `entry` (an index
    /// from [`ZipEntry::index`]) of the zip at `url`.
    Preview { key: RowKey, url: String, entry: Option<usize> },
    /// List the game's documents folder (nothing is created), for the chooser's
    /// "In your documents" marks.
    Scan { tuid: String, title: String },
    /// Save `items` into the game's documents folder, making it if need be.
    Download { tuid: String, title: String, items: Vec<DownloadItem> },
}

/// What the worker reports. Every message is already worded for the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocEvent {
    Resolved(Result<Vec<DocumentOption>, String>),
    Probed { link: usize, size: Option<u64>, ranges: bool },
    Listed { link: usize, result: Result<ZipListing, String> },
    Previewed { key: RowKey, result: Result<String, String> },
    Progress { done: usize, total: usize, name: String },
    /// What the documents folder holds now, answering [`DocJob::Scan`].
    Scanned(Vec<crate::documents::DocEntry>),
    Finished {
        dir: Option<std::path::PathBuf>,
        saved: Vec<String>,
        /// Files skipped because the folder already held these exact bytes: the
        /// existing file's name.
        already: Vec<String>,
        failed: Vec<(String, String)>,
    },
}

/// The documents chooser's worker: the same [`IfdbWorker`] the search modal
/// uses, so there is one request in flight whatever the UI does.
pub type DocumentWorker = IfdbWorker<DocJob, DocEvent>;

impl DocumentWorker {
    pub fn new(gate: IfdbGate, source: Box<dyn SearchSource>, roots: DataRoots) -> Self {
        Self::spawn(gate, move |job, out| run_job(source.as_ref(), &roots, job, &out.tx))
    }
}

/// Run one job, sending its events; `false` when the receiver is gone.
fn run_job(source: &dyn SearchSource, roots: &DataRoots, job: DocJob, tx: &mpsc::Sender<DocEvent>) -> bool {
    run_job_capped(source, roots, job, tx, MAX_DOWNLOAD)
}

fn run_job_capped(
    source: &dyn SearchSource,
    roots: &DataRoots,
    job: DocJob,
    tx: &mpsc::Sender<DocEvent>,
    cap: u64,
) -> bool {
    let send = |e: DocEvent| tx.send(e).is_ok();
    match job {
        DocJob::Resolve { tuid } => send(DocEvent::Resolved(
            source.download_options(&tuid).map(|r| r.documents).map_err(|e| e.to_string()),
        )),
        DocJob::Probe { link, url } => {
            let (size, ranges) = match source.probe_range(&url) {
                Ok(RangeProbe::Supported { total }) => (Some(total), true),
                Ok(RangeProbe::Unsupported { length }) => (length, false),
                Err(_) => (None, false),
            };
            send(DocEvent::Probed { link, size, ranges })
        }
        DocJob::ListZip { link, url } => {
            send(DocEvent::Listed { link, result: list_zip(source, &url).map_err(|e| e.to_string()) })
        }
        DocJob::Preview { key, url, entry } => {
            let result = fetch_preview(source, &url, entry).map_err(|e| e.to_string());
            send(DocEvent::Previewed { key, result })
        }
        DocJob::Scan { tuid, title } => {
            let docs = crate::documents::documents_dir(roots, &tuid, &title)
                .and_then(|d| crate::documents::list(&d).ok())
                .unwrap_or_default();
            send(DocEvent::Scanned(docs))
        }
        DocJob::Download { tuid, title, items } => {
            let dir = match crate::documents::ensure_documents_dir(roots, &tuid, &title) {
                Ok(d) => d,
                Err(e) => {
                    return send(DocEvent::Finished {
                        dir: None,
                        saved: Vec::new(),
                        already: Vec::new(),
                        failed: vec![("documents folder".into(), format!("could not be made: {e}"))],
                    })
                }
            };
            let total = items.len();
            let (mut saved, mut already, mut failed) = (Vec::new(), Vec::new(), Vec::new());
            // The zip last opened, so several entries of one archive share one
            // directory read.
            let mut open: Option<(String, Option<RemoteZip>)> = None;
            for (n, item) in items.iter().enumerate() {
                let (label, got, source_url, info) = match item {
                    DownloadItem::File { url, filename, info } => (
                        filename.clone(),
                        source.fetch_capped(url, cap).map(|b| (filename.clone(), b)),
                        url,
                        info,
                    ),
                    DownloadItem::Entry { zip_url, index, info } => {
                        if open.as_ref().is_none_or(|(u, _)| u != zip_url) {
                            open = Some((zip_url.clone(), RemoteZip::open(source, zip_url).ok().flatten()));
                        }
                        let label = format!("{zip_url} #{index}");
                        match open.as_ref().and_then(|(_, z)| z.as_ref()) {
                            Some(z) => (label, z.read_entry(*index, cap), zip_url, info),
                            None => (label, Err(SearchError::Archive("could not open the zip".into())), zip_url, info),
                        }
                    }
                };
                match got.and_then(|(name, bytes)| {
                    let mut meta = DocMeta::now(
                        info.title.clone(),
                        info.desc.clone(),
                        Some(source_url.clone()),
                        info.spoiler,
                    );
                    meta.zip_entry = info.entry.clone();
                    save_document(&dir, &name, &bytes, Some(meta)).map_err(|e| SearchError::Io(e.to_string()))
                }) {
                    Ok(Imported::Added(d)) => saved.push(d.id),
                    Ok(Imported::AlreadyPresent(d)) => already.push(d.id),
                    Err(SearchError::TooLarge) => failed.push((label, too_large_message(cap))),
                    Err(e) => failed.push((label, e.to_string())),
                }
                if !send(DocEvent::Progress { done: n + 1, total, name: saved.last().or(already.last()).cloned().unwrap_or_default() }) {
                    return false;
                }
            }
            send(DocEvent::Finished { dir: Some(dir), saved, already, failed })
        }
    }
}


#[cfg(all(test, feature = "t-picker"))]
pub(crate) mod tests {
    use super::*;
    use crate::data_roots::DocumentsSettings;
    use crate::ifdb_search::{ResolvedGame, SearchHit};
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex;

    pub(crate) const ZORK: &[u8] = include_bytes!("../tests/fixtures/ifdb-zork1.xml");

    // ── link selection ───────────────────────────────────────────────────────

    /// The exact set Zork I's record yields, in record order — and with it every
    /// rule in the module docs: the .z5/setup zip/.hqx/mp3/html/.inv/hugozork.hug
    /// are all absent, `sample.from.zork` and the `.step1`/`.many` files are in.
    #[test]
    fn zork_i_yields_exactly_these_documents() {
        let got: Vec<(String, LinkKind, bool)> =
            parse_document_options(ZORK).into_iter().map(|d| (d.filename, d.kind, d.spoiler)).collect();
        let want: Vec<(&str, LinkKind, bool)> = vec![
            ("Zork_Trilogy.zip", LinkKind::Archive, false),
            ("zork1.zip", LinkKind::Archive, true),
            ("Sols3.zip", LinkKind::Archive, true),
            ("Sols2.zip", LinkKind::Archive, true),
            ("Sols1.zip", LinkKind::Archive, true),
            ("zorkI.step2", LinkKind::Text, true),
            ("jgunness.zip", LinkKind::Archive, true),
            ("zorkI.step1", LinkKind::Text, true),
            ("hints.many", LinkKind::Text, true),
            ("zork1.txt", LinkKind::Text, false),
            ("sample.from.zork", LinkKind::Text, false),
            ("zorkI.txt", LinkKind::Text, true),
        ];
        let want: Vec<(String, LinkKind, bool)> = want.into_iter().map(|(n, k, s)| (n.to_string(), k, s)).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn classification_follows_the_extension_not_the_format_alone() {
        let k = |url: &str, fmt: Option<&str>| classify(url, fmt);
        assert_eq!(k("https://x/a/Manual.PDF", Some("document")), Some(LinkKind::Pdf));
        for e in ["png", "jpg", "jpeg", "gif", "webp", "tif", "tiff"] {
            assert_eq!(k(&format!("https://x/map.{e}"), None), Some(LinkKind::Image), "{e}");
        }
        for e in ["txt", "doc", "rtf", "md"] {
            assert_eq!(k(&format!("https://x/n.{e}"), None), Some(LinkKind::Text), "{e}");
        }
        assert_eq!(k("https://x/if-archive/x/feelie", Some("document")), Some(LinkKind::Text), "extensionless");
        assert_eq!(k("https://x/if-archive/x/feelie", Some("zcode")), None, "extensionless, not texty");
        assert_eq!(k("https://x/a/b.mp3", Some("document")), None, "audio");
        assert_eq!(k("https://x/a/b.html", Some("html")), None);
        assert_eq!(k("https://x/a/b.hqx", Some("executable")), None);
        assert_eq!(k("https://x/a/b.zip", Some("setup")), None, "a setup zip");
        assert_eq!(k("https://x/a/B.inv", Some("document")), None, "Shift-H owns .inv");
        assert_eq!(k("https://x/games/source/hugo/g.hug", Some("document")), None, "source under /games/");
        assert_eq!(k("https://x/games/g.z5", Some("document")), None, "a story file");
        assert_eq!(k("https://x/", Some("document")), None, "no file name");
    }

    #[test]
    fn names_and_titles_flag_spoilers_outside_spoiler_paths() {
        let xml = |url: &str, title: &str| {
            format!("<ifiction><story><downloads><links><link><url>{url}</url><format>document</format><title>{title}</title></link></links></downloads></story></ifiction>")
        };
        let spoiler = |url: &str, t: &str| parse_document_options(xml(url, t).as_bytes())[0].spoiler;
        assert!(spoiler("https://x/if-archive/games/lostpig/walkthru.txt", "Walkthrough \u{2014} Competition version"));
        assert!(spoiler("https://x/a/b.txt", "Walk-through"));
        assert!(spoiler("https://x/a/zork1_solution.txt", "x"));
        assert!(spoiler("https://x/a/SOLUTION.TXT", "x"));
        assert!(spoiler("https://x/a/m.pdf", "Hints for new players"));
        assert!(!spoiler("https://x/a/manual.pdf", "Manual"));
    }

    #[test]
    fn spoiler_matcher_respects_word_boundaries() {
        for s in ["chintz", "thinter", "Hintergrund", "Manual", "cheater", "scheats"] {
            assert!(!looks_like_spoiler(s), "{s}");
        }
        for s in ["walk through", "How to win", "invisiclues.txt", "zork1_solution.txt", "answers"] {
            assert!(looks_like_spoiler(s), "{s}");
        }
    }

    #[test]
    fn solutions_and_hints_paths_are_spoilers() {
        let xml = |url: &str| {
            format!("<ifiction><story><downloads><links><link><url>{url}</url><format>document</format></link></links></downloads></story></ifiction>")
        };
        let spoiler = |url: &str| parse_document_options(xml(url).as_bytes())[0].spoiler;
        assert!(spoiler("https://x/if-archive/solutions/a.txt"));
        assert!(spoiler("https://x/if-archive/infocom/hints/b.txt"));
        assert!(!spoiler("https://x/if-archive/infocom/shipped-documentation/c.txt"));
        assert!(!spoiler("https://x/a/c.txt?next=/solutions/"), "only the path counts");
    }

    #[test]
    fn entry_names_that_could_escape_are_refused_and_the_rest_cut_to_a_basename() {
        for bad in ["../evil.txt", "a/../../evil.txt", "/etc/passwd", "\\windows\\x.txt", "C:\\x.txt", "c:evil.txt", "a\0b.txt", "..\\..\\x"] {
            assert_eq!(safe_entry_basename(bad), None, "{bad:?}");
        }
        assert_eq!(safe_entry_basename("Sols/ZorkI.txt").as_deref(), Some("ZorkI.txt"));
        assert_eq!(safe_entry_basename("a\\b\\map.png").as_deref(), Some("map.png"));
        assert_eq!(safe_entry_basename("dir/").as_deref(), Some("dir"));
        assert_eq!(safe_entry_basename("a/.hidden"), None, "no dotfiles");
        assert_eq!(safe_entry_basename("a/b:c?.txt").as_deref(), Some("b_c_.txt"));
        assert_eq!(safe_entry_basename("CON.txt").as_deref(), Some("_CON.txt"));
    }

    #[test]
    fn title_matching_ignores_case_spaces_and_punctuation() {
        assert!(matches_title("ZorkI.txt", "Zork I"));
        assert!(matches_title("zork_i", "Zork I"));
        assert!(matches_title("ZorkI", "Zork I: The Great Underground Empire"));
        assert!(!matches_title("ZorkII.txt", "Zork I"));
        assert!(!matches_title("Planetfall.txt", "Zork I"));
        assert!(!matches_title(".txt", "Zork I"));
    }

    #[test]
    fn preview_keeps_twenty_clean_lines() {
        let src: String = (0..50).map(|i| format!("line {i}\t\x07x\r\n")).collect();
        let p = preview_text(src.as_bytes());
        assert_eq!(p.lines().count(), PREVIEW_LINES);
        assert!(p.starts_with("line 0 x"), "{p:?}");
        assert!(!p.contains('\x07') && !p.contains('\r'));
        assert!(is_previewable("a.txt") && is_previewable("feelie") && !is_previewable("a.png"));
    }

    // ── a fake host ──────────────────────────────────────────────────────────

    /// A fake file host: a map of URL to bytes, with range support on or off, a
    /// log of every range asked for and a count of whole-file fetches.
    pub(crate) struct Host {
        pub(crate) files: HashMap<String, Vec<u8>>,
        pub(crate) ranges: bool,
        pub(crate) docs: Vec<DocumentOption>,
        pub(crate) log: Mutex<Vec<(u64, u64)>>,
        pub(crate) whole: Mutex<usize>,
    }

    impl Host {
        pub(crate) fn new(ranges: bool) -> Self {
            Host { files: HashMap::new(), ranges, docs: Vec::new(), log: Mutex::new(Vec::new()), whole: Mutex::new(0) }
        }
        pub(crate) fn with(mut self, url: &str, bytes: Vec<u8>) -> Self {
            self.files.insert(url.to_string(), bytes);
            self
        }
        fn get(&self, url: &str) -> Result<&Vec<u8>, SearchError> {
            self.files.get(url).ok_or_else(|| SearchError::Transport("404".into()))
        }
        pub(crate) fn requested(&self) -> Vec<(u64, u64)> {
            self.log.lock().unwrap().clone()
        }
    }

    impl SearchSource for Host {
        fn search(&self, _q: &str) -> Result<Vec<SearchHit>, SearchError> {
            Ok(Vec::new())
        }
        fn hot(&self) -> Result<Vec<SearchHit>, SearchError> {
            Ok(Vec::new())
        }
        fn download_options(&self, _tuid: &str) -> Result<ResolvedGame, SearchError> {
            Ok(ResolvedGame { documents: self.docs.clone(), ..Default::default() })
        }
        fn download(&self, _url: &str, _dest: &Path) -> Result<PathBuf, SearchError> {
            Err(SearchError::Transport("not used".into()))
        }
        fn probe_range(&self, url: &str) -> Result<RangeProbe, SearchError> {
            let total = self.get(url)?.len() as u64;
            Ok(if self.ranges { RangeProbe::Supported { total } } else { RangeProbe::Unsupported { length: Some(total) } })
        }
        fn fetch_range(&self, url: &str, start: u64, len: u64) -> Result<Vec<u8>, SearchError> {
            let bytes = self.get(url)?;
            if !self.ranges && start > 0 {
                return Err(SearchError::Transport("server ignored the Range request".into()));
            }
            self.log.lock().unwrap().push((start, len));
            let end = (start + len).min(bytes.len() as u64);
            Ok(bytes[start as usize..end as usize].to_vec())
        }
        fn fetch_capped(&self, url: &str, cap: u64) -> Result<Vec<u8>, SearchError> {
            let bytes = self.get(url)?;
            *self.whole.lock().unwrap() += 1;
            if bytes.len() as u64 > cap {
                return Err(SearchError::TooLarge);
            }
            Ok(bytes.clone())
        }
    }

    /// A stored (uncompressed) zip: `n` entries of `size` bytes each, named
    /// `Sols/game<i>.txt`, plus `extra` named entries.
    pub(crate) fn big_zip(n: usize, size: usize, extra: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for i in 0..n {
            w.start_file(format!("Sols/game{i}.txt"), opts).unwrap();
            let body: Vec<u8> = (0..size).map(|j| (j.wrapping_mul(31).wrapping_add(i) % 251) as u8).collect();
            w.write_all(&body).unwrap();
        }
        for (name, body) in extra {
            w.start_file(*name, opts).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    pub(crate) fn roots(tag: &str) -> (PathBuf, DataRoots) {
        let home = crate::scratch_dir(tag);
        let r = DataRoots::resolve(&home, None, None, &DocumentsSettings::default());
        (home, r)
    }

    pub(crate) fn files_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('.')).collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    // ── the range adapter ────────────────────────────────────────────────────

    const URL: &str = "https://ifarchive.example/solutions/Sols1.zip";

    #[test]
    fn listing_a_zip_reads_only_its_tail() {
        let zip = big_zip(20, 60_000, &[("Sols/ZorkI.txt", b"ZORK I walkthrough\n1. open mailbox\n")]);
        let total = zip.len() as u64;
        assert!(total > 16 * TAIL, "the specimen must dwarf the tail read: {total}");
        let host = Host::new(true).with(URL, zip);
        let listing = list_zip(&host, URL).unwrap();
        let ZipListing::Entries(entries) = listing else { panic!("ranges were on") };
        assert_eq!(entries.len(), 21);
        assert!(entries.iter().any(|e| e.name == "ZorkI.txt" && e.path == "Sols/ZorkI.txt" && e.size == 35));
        assert!(entries.iter().filter(|e| e.name.starts_with("game")).all(|e| e.size == 60_000), "sizes come from the directory");
        let reqs = host.requested();
        assert_eq!(reqs, [(total - TAIL, TAIL)], "ONE request, for the tail, and nothing else");
        let fetched: u64 = reqs.iter().map(|r| r.1).sum();
        assert!(fetched * 10 < total, "listing fetched {fetched} of {total} bytes");
        assert_eq!(*host.whole.lock().unwrap(), 0, "the archive was never downloaded whole");
    }

    #[test]
    fn zip_entries_are_flagged_by_their_own_name() {
        let url = "https://ifarchive.example/docs/pack.zip";
        let zip = big_zip(0, 0, &[("solution.txt", b"x"), ("manual.txt", b"y")]);
        let host = Host::new(true).with(url, zip);
        let ZipListing::Entries(entries) = list_zip(&host, url).unwrap() else { panic!("ranges were on") };
        let flag = |n: &str| entries.iter().find(|e| e.name == n).unwrap().spoiler;
        assert!(flag("solution.txt"));
        assert!(!flag("manual.txt"));
    }

    #[test]
    fn one_entry_is_extracted_without_the_rest() {
        let zip = big_zip(20, 60_000, &[("Sols/ZorkI.txt", b"ZORK I walkthrough\n")]);
        let total = zip.len() as u64;
        let host = Host::new(true).with(URL, zip);
        let z = RemoteZip::open(&host, URL).unwrap().unwrap();
        let idx = z.entries().into_iter().find(|e| e.name == "ZorkI.txt").unwrap().index;
        let (name, bytes) = z.read_entry(idx, MAX_DOWNLOAD).unwrap();
        assert_eq!((name.as_str(), bytes.as_slice()), ("ZorkI.txt", &b"ZORK I walkthrough\n"[..]));
        let fetched: u64 = host.requested().iter().map(|r| r.1).sum();
        assert!(fetched < total / 10, "{fetched} of {total}");
        assert_eq!(host.requested().len(), 3, "tail, local header, entry bytes");
    }

    #[test]
    fn a_server_without_ranges_gets_contents_unknown_and_no_download() {
        let host = Host::new(false).with(URL, big_zip(3, 1000, &[]));
        assert_eq!(list_zip(&host, URL).unwrap(), ZipListing::Unknown);
        assert!(host.requested().is_empty());
        assert_eq!(*host.whole.lock().unwrap(), 0, "never fetched whole just to list it");
        assert!(RemoteZip::open(&host, URL).unwrap().is_none());
    }

    #[test]
    fn a_text_entry_and_a_top_level_text_link_preview() {
        let body: String = (0..40).map(|i| format!("step {i}\n")).collect();
        let zip = big_zip(5, 40_000, &[("Sols/ZorkI.txt", body.as_bytes())]);
        let host = Host::new(true).with(URL, zip).with("https://x/a.txt", body.clone().into_bytes());
        let z = RemoteZip::open(&host, URL).unwrap().unwrap();
        let idx = z.entries().into_iter().find(|e| e.name == "ZorkI.txt").unwrap().index;
        drop(z);
        let inside = fetch_preview(&host, URL, Some(idx)).unwrap();
        assert_eq!(inside.lines().count(), PREVIEW_LINES);
        assert!(inside.starts_with("step 0\nstep 1"));
        let top = fetch_preview(&host, "https://x/a.txt", None).unwrap();
        assert_eq!(top, inside);
    }

    #[test]
    fn unsafe_entry_names_are_not_offered_and_cannot_be_read() {
        let zip = big_zip(0, 0, &[("../evil.txt", b"x"), ("ok/fine.txt", b"y"), ("/abs.txt", b"z"), ("C:\\drive.txt", b"w")]);
        let host = Host::new(true).with(URL, zip);
        let z = RemoteZip::open(&host, URL).unwrap().unwrap();
        let names: Vec<String> = z.entries().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["fine.txt"], "only the safe one is offered");
        // Asked for by index anyway (a stale or hostile request), it is refused.
        let evil = (0..4).find(|i| z.raw[*i].path == "../evil.txt").unwrap();
        assert!(matches!(z.read_entry(evil, MAX_DOWNLOAD), Err(SearchError::NoFilename)));
    }

    #[test]
    fn an_inflated_entry_over_the_cap_is_refused() {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        w.start_file("bomb.txt", opts).unwrap();
        w.write_all(&vec![0u8; 200_000]).unwrap(); // tiny when deflated, 200 kB inflated
        let zip = w.finish().unwrap().into_inner();
        assert!(zip.len() < 5_000, "the specimen must be small: {}", zip.len());
        let host = Host::new(true).with(URL, zip);
        let z = RemoteZip::open(&host, URL).unwrap().unwrap();
        assert!(matches!(z.read_entry(0, 100_000), Err(SearchError::TooLarge)));
        assert_eq!(z.read_entry(0, 200_000).unwrap().1.len(), 200_000, "exactly at the cap is fine");
    }

    // ── the worker and the folder ────────────────────────────────────────────

    fn run(host: &Host, roots: &DataRoots, job: DocJob, cap: u64) -> Vec<DocEvent> {
        let (tx, rx) = mpsc::channel();
        assert!(run_job_capped(host, roots, job, &tx, cap));
        drop(tx);
        rx.try_iter().collect()
    }

    fn finished(evs: Vec<DocEvent>) -> (Option<PathBuf>, Vec<String>, Vec<(String, String)>) {
        match evs.into_iter().last() {
            Some(DocEvent::Finished { dir, saved, failed, .. }) => (dir, saved, failed),
            other => panic!("no Finished: {other:?}"),
        }
    }

    #[test]
    fn downloads_land_in_the_documents_folder_skip_duplicates_and_suffix_clashes() {
        let (home, roots) = roots("docs-dl-land");
        let zip = big_zip(3, 2000, &[("Sols/ZorkI.txt", b"walkthrough")]);
        let host = Host::new(true).with("https://x/manual.pdf", b"%PDF-1.4 hello".to_vec()).with(URL, zip);
        let items = || {
            vec![
                DownloadItem::File { url: "https://x/manual.pdf".into(), filename: "manual.pdf".into(), info: LinkInfo::default() },
                DownloadItem::Entry { zip_url: URL.into(), index: 3, info: LinkInfo::default() },
            ]
        };
        let job = || DocJob::Download { tuid: "abc123".into(), title: "Zork I".into(), items: items() };

        let (dir, saved, failed) = finished(run(&host, &roots, job(), MAX_DOWNLOAD));
        let dir = dir.unwrap();
        assert_eq!(dir, roots.documents().join("Zork I [abc123]"), "made on demand");
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(saved, ["manual.pdf", "ZorkI.txt"]);
        assert_eq!(std::fs::read(dir.join("ZorkI.txt")).unwrap(), b"walkthrough");

        // A re-download of the same files adds nothing and says so.
        let evs = run(&host, &roots, job(), MAX_DOWNLOAD);
        let Some(DocEvent::Finished { saved: saved2, already, failed, .. }) = evs.into_iter().last() else { panic!() };
        assert!(saved2.is_empty() && failed.is_empty(), "{saved2:?} {failed:?}");
        assert_eq!(already, ["manual.pdf", "ZorkI.txt"]);
        assert_eq!(files_in(&dir), ["ZorkI.txt", "manual.pdf"]);

        // Same names, different bytes: suffixed, never overwritten.
        let host2 = Host::new(true)
            .with("https://x/manual.pdf", b"%PDF-1.4 hello v2".to_vec())
            .with(URL, big_zip(3, 2000, &[("Sols/ZorkI.txt", b"walkthrough, revised")]));
        let (_, saved3, _) = finished(run(&host2, &roots, job(), MAX_DOWNLOAD));
        assert_eq!(saved3, ["manual (2).pdf", "ZorkI (2).txt"]);
        assert_eq!(files_in(&dir), ["ZorkI (2).txt", "ZorkI.txt", "manual (2).pdf", "manual.pdf"]);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn downloads_write_the_index_with_link_info_and_a_zip_entrys_from_note() {
        let (home, roots) = roots("docs-dl-index");
        let zip = big_zip(3, 2000, &[("Sols/ZorkI.txt", b"walkthrough")]);
        let host = Host::new(true).with("https://x/manual.pdf", b"%PDF-1.4 hello".to_vec()).with(URL, zip);
        let items = vec![
            DownloadItem::File {
                url: "https://x/manual.pdf".into(),
                filename: "manual.pdf".into(),
                info: LinkInfo { title: Some("Manual".into()), desc: Some("The printed manual".into()), spoiler: false, entry: None },
            },
            DownloadItem::Entry {
                zip_url: URL.into(),
                index: 3,
                info: LinkInfo { title: Some("ZorkI.txt".into()), desc: Some("Sols (from Setup.zip)".into()), spoiler: true, entry: Some("Sols/ZorkI.txt".into()) },
            },
        ];
        let (dir, _, failed) =
            finished(run(&host, &roots, DocJob::Download { tuid: "abc123".into(), title: "Zork I".into(), items }, MAX_DOWNLOAD));
        assert!(failed.is_empty(), "{failed:?}");
        let listed = crate::documents::list(&dir.unwrap()).unwrap();
        let m = listed.iter().find(|e| e.id == "manual.pdf").unwrap();
        assert_eq!((m.desc.as_deref(), m.source_url.as_deref()), (Some("The printed manual"), Some("https://x/manual.pdf")));
        assert_eq!(m.subtitle().as_deref(), Some("Manual \u{2014} The printed manual"));
        let z = listed.iter().find(|e| e.id == "ZorkI.txt").unwrap();
        assert_eq!((z.desc.as_deref(), z.source_url.as_deref(), z.spoiler), (Some("Sols (from Setup.zip)"), Some(URL), true));
        // The size on disk is recorded for a link and for a zip entry, and the
        // entry remembers where in the zip it came from (SQ-1699).
        assert_eq!((m.recorded_size, m.zip_entry.as_deref()), (Some(14), None));
        assert_eq!((z.recorded_size, z.zip_entry.as_deref()), (Some(11), Some("Sols/ZorkI.txt")));
        let _ = std::fs::remove_dir_all(home);
    }

    /// Save a manual (a whole link) and one zip entry through the real worker,
    /// and return the home, the folder and its listing.
    fn saved_folder(tag: &str) -> (PathBuf, PathBuf, Vec<crate::documents::DocEntry>) {
        let (home, roots) = roots(tag);
        let zip = big_zip(3, 2000, &[("Sols/ZorkI.txt", b"walkthrough")]);
        let host = Host::new(true).with("https://x/manual.pdf", b"%PDF-1.4 hello".to_vec()).with(URL, zip);
        let items = vec![
            DownloadItem::File { url: "https://x/manual.pdf".into(), filename: "manual.pdf".into(), info: LinkInfo::default() },
            DownloadItem::Entry {
                zip_url: URL.into(),
                index: 3,
                info: LinkInfo { entry: Some("Sols/ZorkI.txt".into()), ..LinkInfo::default() },
            },
        ];
        let (dir, _, failed) =
            finished(run(&host, &roots, DocJob::Download { tuid: "abc123".into(), title: "Zork I".into(), items }, MAX_DOWNLOAD));
        assert!(failed.is_empty(), "{failed:?}");
        let dir = dir.unwrap();
        let listed = crate::documents::list(&dir).unwrap();
        (home, dir, listed)
    }

    #[test]
    fn saved_state_tells_saved_changed_and_not_saved_for_links_and_zip_entries() {
        let (home, dir, listed) = saved_folder("docs-saved-state");
        let pdf = "https://x/manual.pdf";
        assert_eq!(saved_state(&listed, pdf, None), SavedState::Saved);
        assert_eq!(saved_state(&listed, URL, Some("Sols/ZorkI.txt")), SavedState::Saved);
        // The entry came out of the zip: that is not the whole zip being saved,
        // and a different entry of it is not saved either.
        assert_eq!(saved_state(&listed, URL, None), SavedState::NotSaved);
        assert_eq!(saved_state(&listed, URL, Some("Sols/Other.txt")), SavedState::NotSaved);
        assert_eq!(saved_state(&listed, "https://x/other.pdf", None), SavedState::NotSaved);

        // Edited on disk: still there, a different size.
        std::fs::write(dir.join("manual.pdf"), b"%PDF-1.4 hello, edited").unwrap();
        std::fs::write(dir.join("ZorkI.txt"), b"x").unwrap();
        let listed = crate::documents::list(&dir).unwrap();
        assert_eq!(saved_state(&listed, pdf, None), SavedState::Changed);
        assert_eq!(saved_state(&listed, URL, Some("Sols/ZorkI.txt")), SavedState::Changed);

        // Deleted: not saved, however the index remembers it.
        std::fs::remove_file(dir.join("manual.pdf")).unwrap();
        let listed = crate::documents::list(&dir).unwrap();
        assert_eq!(saved_state(&listed, pdf, None), SavedState::NotSaved);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn an_entry_without_a_recorded_size_counts_as_saved_while_the_file_is_there() {
        let (home, dir, _) = saved_folder("docs-saved-legacy");
        // An index from before sizes were kept: drop every `size`.
        let idx = dir.join(".documents.json");
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&idx).unwrap()).unwrap();
        for e in v.as_object_mut().unwrap().values_mut() {
            e.as_object_mut().unwrap().remove("size");
        }
        std::fs::write(&idx, serde_json::to_vec(&v).unwrap()).unwrap();
        std::fs::write(dir.join("manual.pdf"), b"changed since, but nobody wrote the size down").unwrap();
        let listed = crate::documents::list(&dir).unwrap();
        assert!(listed.iter().all(|d| d.recorded_size.is_none()));
        assert_eq!(saved_state(&listed, "https://x/manual.pdf", None), SavedState::Saved);
        assert_eq!(saved_state(&listed, URL, Some("Sols/ZorkI.txt")), SavedState::Saved);
        std::fs::remove_file(dir.join("manual.pdf")).unwrap();
        let listed = crate::documents::list(&dir).unwrap();
        assert_eq!(saved_state(&listed, "https://x/manual.pdf", None), SavedState::NotSaved, "deleted");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn link_info_words_a_zip_entrys_description() {
        let zip = DocumentOption {
            filename: "Setup.zip".into(),
            url: URL.into(),
            kind: LinkKind::Archive,
            format: None,
            title: None,
            desc: Some("Solutions".into()),
            spoiler: false,
        };
        let entry = ZipEntry { index: 0, path: "a/b.txt".into(), name: "b.txt".into(), size: 1, spoiler: true };
        let info = LinkInfo::of_entry(&zip, &entry);
        assert_eq!(info, LinkInfo { title: Some("b.txt".into()), desc: Some("Solutions (from Setup.zip)".into()), spoiler: true, entry: Some("a/b.txt".into()) });
    }

    #[test]
    fn the_cap_aborts_cleanly_and_leaves_no_partial_file() {
        let (home, roots) = roots("docs-dl-cap");
        let zip = big_zip(1, 5000, &[]);
        let host = Host::new(true).with("https://x/huge.pdf", vec![7u8; 5000]).with(URL, zip);
        let items = vec![
            DownloadItem::File { url: "https://x/huge.pdf".into(), filename: "huge.pdf".into(), info: LinkInfo::default() },
            DownloadItem::Entry { zip_url: URL.into(), index: 0, info: LinkInfo::default() },
        ];
        let job = DocJob::Download { tuid: "abc123".into(), title: "Zork I".into(), items };
        let evs = run(&host, &roots, job, 4000);
        assert!(evs.iter().any(|e| matches!(e, DocEvent::Progress { done: 2, total: 2, .. })));
        let (dir, saved, failed) = finished(evs);
        assert!(saved.is_empty(), "{saved:?}");
        assert_eq!(failed.len(), 2, "{failed:?}");
        assert!(failed.iter().all(|(_, why)| why.starts_with("Too large to download")), "{failed:?}");
        assert_eq!(files_in(&dir.unwrap()), Vec::<String>::new(), "nothing, partial or whole, was left");
        let _ = std::fs::remove_dir_all(home);
    }

    /// The documents cap is the shared 100 MB (SQ-1682): a 60 MB file, which the
    /// old 50 MB limit refused, now saves; 101 MB is still refused, in words.
    #[test]
    fn documents_share_the_hundred_megabyte_cap() {
        assert_eq!(MAX_DOWNLOAD, 100 * 1024 * 1024);
        let (home, roots) = roots("docs-dl-100mb");
        let host = Host::new(true)
            .with("https://x/sixty.pdf", vec![1u8; 60 * 1024 * 1024])
            .with("https://x/huge.pdf", vec![1u8; 101 * 1024 * 1024]);
        let items = vec![
            DownloadItem::File { url: "https://x/sixty.pdf".into(), filename: "sixty.pdf".into(), info: LinkInfo::default() },
            DownloadItem::File { url: "https://x/huge.pdf".into(), filename: "huge.pdf".into(), info: LinkInfo::default() },
        ];
        let job = DocJob::Download { tuid: "abc123".into(), title: "Zork I".into(), items };
        let (dir, saved, failed) = finished(run(&host, &roots, job, MAX_DOWNLOAD));
        assert_eq!(saved, vec!["sixty.pdf".to_string()]);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert_eq!(failed[0].1, "Too large to download (over 100 MB)");
        assert_eq!(files_in(&dir.unwrap()), vec!["sixty.pdf".to_string()], "no partial huge.pdf");
        let _ = std::fs::remove_dir_all(home);
    }

    /// A zip entry that inflates past the shared cap is stopped at it even when
    /// the directory declares 10 bytes (101 MB of zeros deflates to ~100 KB).
    #[test]
    fn inflation_stops_at_the_shared_cap() {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        w.start_file("bomb.txt", opts).unwrap();
        w.write_all(&vec![0u8; 101 * 1024 * 1024]).unwrap();
        let mut zip = w.finish().unwrap().into_inner();
        let host = Host::new(true).with(URL, zip.clone());
        let z = RemoteZip::open(&host, URL).unwrap().unwrap();
        assert!(matches!(z.read_entry(0, MAX_DOWNLOAD), Err(SearchError::TooLarge)), "declared size");
        let at = zip.windows(4).position(|w| w == CENTRAL_SIG).unwrap();
        zip[at + 24..at + 28].copy_from_slice(&10u32.to_le_bytes());
        let host = Host::new(true).with(URL, zip);
        let z = RemoteZip::open(&host, URL).unwrap().unwrap();
        assert!(matches!(z.read_entry(0, MAX_DOWNLOAD), Err(SearchError::TooLarge)), "while inflating");
    }

    #[test]
    fn resolve_probe_list_and_preview_jobs_report_through_events() {
        let (home, roots) = roots("docs-jobs");
        let mut host = Host::new(true).with(URL, big_zip(2, 100, &[])).with("https://x/a.txt", b"hello\nworld\n".to_vec());
        host.docs = parse_document_options(ZORK);
        let (tx, rx) = mpsc::channel();
        run_job(&host, &roots, DocJob::Resolve { tuid: "t".into() }, &tx);
        run_job(&host, &roots, DocJob::Probe { link: 4, url: URL.into() }, &tx);
        run_job(&host, &roots, DocJob::ListZip { link: 4, url: URL.into() }, &tx);
        run_job(&host, &roots, DocJob::Preview { key: (7, None), url: "https://x/a.txt".into(), entry: None }, &tx);
        drop(tx);
        let evs: Vec<DocEvent> = rx.try_iter().collect();
        assert!(matches!(&evs[0], DocEvent::Resolved(Ok(d)) if d.len() == 12));
        assert!(matches!(&evs[1], DocEvent::Probed { link: 4, size: Some(_), ranges: true }));
        assert!(matches!(&evs[2], DocEvent::Listed { link: 4, result: Ok(ZipListing::Entries(e)) } if e.len() == 2));
        assert_eq!(evs[3], DocEvent::Previewed { key: (7, None), result: Ok("hello\nworld".into()) });
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_worker_thread_answers_a_job() {
        let (home, roots) = roots("docs-worker");
        let mut host = Host::new(true);
        host.docs = parse_document_options(ZORK);
        let w = DocumentWorker::new(IfdbGate::default(), Box::new(host), roots);
        w.request(DocJob::Resolve { tuid: "t".into() });
        let mut got = Vec::new();
        for _ in 0..2000 {
            got = w.drain();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(matches!(&got[0], DocEvent::Resolved(Ok(d)) if d.len() == 12));
        let _ = std::fs::remove_dir_all(home);
    }

    /// A directory that LIES about an entry's size (a zip bomb's whole trick) is
    /// stopped by the cap on what actually inflates, not by the declared figure.
    #[test]
    fn a_lying_declared_size_does_not_get_past_the_cap() {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        w.start_file("bomb.txt", opts).unwrap();
        w.write_all(&vec![0u8; 300_000]).unwrap();
        let mut zip = w.finish().unwrap().into_inner();
        let at = zip.windows(4).position(|w| w == CENTRAL_SIG).unwrap();
        zip[at + 24..at + 28].copy_from_slice(&10u32.to_le_bytes()); // "10 bytes", honest as a politician
        let host = Host::new(true).with(URL, zip);
        let z = RemoteZip::open(&host, URL).unwrap().unwrap();
        assert_eq!(z.entries()[0].size, 10);
        assert!(matches!(z.read_entry(0, 100_000), Err(SearchError::TooLarge)), "stopped while inflating");
    }

    #[test]
    fn deflated_entries_extract_preview_and_check_their_checksum() {
        let text: String = (0..500).map(|i| format!("line {i} of the walkthrough\n")).collect();
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        w.start_file("docs/walk.txt", opts).unwrap();
        w.write_all(text.as_bytes()).unwrap();
        let zip = w.finish().unwrap().into_inner();
        let host = Host::new(true).with(URL, zip.clone());
        let z = RemoteZip::open(&host, URL).unwrap().unwrap();
        assert_eq!(z.read_entry(0, MAX_DOWNLOAD).unwrap(), ("walk.txt".to_string(), text.clone().into_bytes()));
        let p = fetch_preview(&host, URL, Some(0)).unwrap();
        assert_eq!(p.lines().count(), PREVIEW_LINES);
        assert!(p.starts_with("line 0 of the walkthrough"));

        // Flip a byte of the compressed data: refused, never saved.
        let mut bad = zip;
        bad[60] ^= 0xFF;
        let host = Host::new(true).with(URL, bad);
        let z = RemoteZip::open(&host, URL).unwrap().unwrap();
        assert!(matches!(z.read_entry(0, MAX_DOWNLOAD), Err(SearchError::Archive(_))));
    }

    #[test]
    fn something_that_is_not_a_zip_is_an_error_not_a_panic() {
        let host = Host::new(true).with(URL, b"<html>404 not found</html> padding padding padding".to_vec());
        assert!(matches!(RemoteZip::open(&host, URL), Err(SearchError::Archive(_))));
        let host = Host::new(true).with(URL, vec![1u8; 10]);
        assert!(matches!(RemoteZip::open(&host, URL), Err(SearchError::Archive(_))));
        assert!(Host::new(true).with(URL, big_zip(1, 10, &[])).files.contains_key(URL));
    }

    #[test]
    fn format_size_reads_naturally() {
        assert_eq!(format_size(812), "812 B");
        assert_eq!(format_size(12 * 1024), "12 KB");
        assert_eq!(format_size(1_500_000), "1.4 MB");
    }
}
