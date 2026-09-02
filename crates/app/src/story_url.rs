//! Fetching a story from a URL (SQ-1086).
//!
//! Everywhere lanthorn accepts a story path it now also accepts an `http://` or
//! `https://` address. The rule that keeps this small is that **nothing here
//! knows what a story is**: the URL is fetched to a local file and that file is
//! handed to the ordinary loader (`hints::load_mounted_story`), so `.z3`–`.z8`,
//! Blorbs, Glulx, Scott Adams databases, release disk images and ZIPs all work
//! for free and cannot drift from what opening the same file by name would do.
//!
//! ## Where a fetched file lands
//!
//! Into [`download_dir`] — a `lanthorn-fetch` directory under the system temp
//! dir — because at fetch time nobody has yet said they want to keep it. If they
//! do, `render::fetch_keep_dialog` copies it into the library directory the
//! picker reads. lanthorn never deletes what it left in the temp dir: the file
//! IS the story being played, and its basename is the save key
//! (`storage::story_key_for`), so removing it under a running game — or under a
//! Save State that names it — would be the one destructive thing a download
//! could do. Reaping temp files is the operating system's job.
//!
//! ## Untrusted input, and what is done about it
//!
//! A URL is typed (or pasted) by a person and served by a stranger, so:
//!
//! - **Redirects are capped** at [`MAX_REDIRECTS`] and a chain longer than that
//!   is an error rather than a silent stop, so a login-wall bounce cannot loop.
//! - **The body is capped** at [`MAX_DOWNLOAD`], rejected on an honest
//!   `Content-Length` before a byte is read and again while reading (the
//!   `read_capped` the IFDB downloader already uses).
//! - **The filename is derived, never adopted.** [`safe_basename`] keeps only
//!   the final path component under BOTH separators, drops control and
//!   path-significant characters and refuses `.`/`..`/leading-dot names, so a
//!   `Content-Disposition: filename="../../.bashrc"` cannot escape the
//!   destination directory. The write itself goes through
//!   `ifdb_search::unique_dest`, which never replaces an existing file.
//! - **The bytes must look like something openable** before anything is written
//!   ([`content_extension`]) and must actually open afterwards, or the file is
//!   removed and the error says what arrived instead.
//!
//! `file://` is deliberately NOT accepted: a local file is already a path, and
//! every caller here takes a path.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use crate::ifdb_search::{
    read_capped, unique_dest, BODY_TIMEOUT, CONNECT_TIMEOUT, HEADERS_TIMEOUT, MAX_DOWNLOAD,
};

/// Redirect chain cap. Generous for the archives that host IF (the IF Archive
/// bounces http→https→mirror), tight enough that a login-wall ping-pong ends.
pub const MAX_REDIRECTS: u32 = 5;

// ── Recognising a URL ────────────────────────────────────────────────────────

/// Is `s` a remote address lanthorn will fetch a story from?
///
/// Only `http` and `https`, case-insensitively — the two schemes every IF
/// archive serves. Anything else is a path (see [`declined_scheme`]).
pub fn is_story_url(s: &str) -> bool {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")) && has_host(s)
}

/// True when something follows the `://`, so a bare `https://` is not mistaken
/// for an address.
fn has_host(s: &str) -> bool {
    s.split_once("://").is_some_and(|(_, rest)| !rest.trim().is_empty())
}

/// A URL-shaped argument lanthorn will NOT fetch, and the reason — so a
/// `file://` or `ftp://` argument gets an answer instead of a "no such file"
/// about a path nobody typed. `None` for anything that is not scheme-shaped.
pub fn declined_scheme(s: &str) -> Option<String> {
    let s = s.trim();
    if is_story_url(s) {
        return None;
    }
    let (scheme, _) = s.split_once("://")?;
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-') {
        return None;
    }
    let lower = scheme.to_ascii_lowercase();
    if lower == "file" {
        return Some(
            "file:// is not a remote address — pass the path itself (lanthorn already opens local files)"
                .to_string(),
        );
    }
    Some(format!("{lower}:// addresses are not supported; lanthorn fetches http:// and https:// only"))
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The request never produced a body (DNS, TLS, a 404, too many redirects).
    Transport(String),
    /// The body is larger than [`MAX_DOWNLOAD`].
    TooLarge,
    /// Bytes arrived, but they are not a story of any kind lanthorn opens. The
    /// string says what they look like instead — an HTML page, a PDF, junk.
    Unopenable(String),
    /// Nothing safe and nameable could be derived for the local file.
    NoFilename,
    /// An archive entry named a path that would land outside the destination
    /// directory (SQ-1096). Refused outright rather than trimmed: a zip that
    /// tries to escape is not one lanthorn unpacks, and taking only its final
    /// component would silently reward the attempt.
    UnsafeEntry(String),
    /// A local filesystem error writing the download.
    Io(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Transport(e) => write!(f, "{e}"),
            FetchError::TooLarge => {
                write!(f, "the file is larger than {} MiB", MAX_DOWNLOAD / (1024 * 1024))
            }
            FetchError::Unopenable(what) => write!(f, "{what}"),
            FetchError::NoFilename => write!(f, "no usable filename could be derived from the URL"),
            FetchError::UnsafeEntry(name) => write!(
                f,
                "the archive entry '{name}' names a path outside your library, so nothing was unpacked"
            ),
            FetchError::Io(e) => write!(f, "{e}"),
        }
    }
}

// ── The network seam ─────────────────────────────────────────────────────────

/// One fetched response: the bytes, plus whatever the host said the file was
/// called.
pub struct Payload {
    pub disposition: Option<String>,
    pub bytes: Vec<u8>,
}

/// Where bytes come from. The one seam a test substitutes, mirroring
/// `ifdb::MetadataSource` and `ifdb_search::SearchSource` — every test in this
/// module drives a canned source, so nothing here ever reaches the network.
pub trait UrlSource: Send + Sync {
    fn get(&self, url: &str) -> Result<Payload, FetchError>;
}

/// The live `ureq` fetcher: capped redirects, capped body, lanthorn's own
/// User-Agent (the same string `ifdb.rs` and `hint_download.rs` send).
pub struct HttpSource {
    agent: ureq::Agent,
}

impl HttpSource {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            // Per PHASE, never end-to-end. `timeout_global` is documented as
            // "from DNS lookup to finishing reading the response body", so any
            // value small enough to be a useful stall detector is also small
            // enough to refuse a 22 MB Glulx game on an ordinary link — the same
            // false premise as the old 16 MiB cap, wearing a clock. The budget
            // and the size cap live together in `ifdb_search`; see the note
            // above them for why they are one fact.
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(HEADERS_TIMEOUT))
            .timeout_recv_body(Some(BODY_TIMEOUT))
            .user_agent(user_agent())
            .max_redirects(MAX_REDIRECTS)
            // A chain longer than the cap must FAIL, not quietly hand back the
            // last redirect's body — a login wall would otherwise be written to
            // disk as the story.
            .max_redirects_will_error(true)
            .build();
        Self { agent: ureq::Agent::new_with_config(config) }
    }
}

impl Default for HttpSource {
    fn default() -> Self {
        Self::new()
    }
}

impl UrlSource for HttpSource {
    fn get(&self, url: &str) -> Result<Payload, FetchError> {
        let mut resp =
            self.agent.get(url).call().map_err(|e| FetchError::Transport(e.to_string()))?;

        // Fast reject on an honest Content-Length, before reading a byte.
        if let Some(len) = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
        {
            if len > MAX_DOWNLOAD {
                return Err(FetchError::TooLarge);
            }
        }
        let disposition =
            resp.headers().get("content-disposition").and_then(|v| v.to_str().ok()).map(str::to_string);
        let bytes = read_capped(resp.body_mut().as_reader(), MAX_DOWNLOAD)
            .map_err(|e| match e {
                crate::ifdb_search::SearchError::TooLarge => FetchError::TooLarge,
                other => FetchError::Transport(other.to_string()),
            })?;
        Ok(Payload { disposition, bytes })
    }
}

fn user_agent() -> String {
    format!("lanthorn/{} (+https://github.com/sharkusk/lanthorn)", env!("CARGO_PKG_VERSION"))
}

// ── What arrived ─────────────────────────────────────────────────────────────

const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// The file extension `bytes` themselves imply, or `None` when they are not
/// anything lanthorn opens.
///
/// Content decides, never the URL — which is the same rule `hints::read_story_file`
/// and `hints::extract_story` follow when opening a local file, so a fetch cannot
/// admit something the loader would then refuse (or vice versa). The extension it
/// returns matters for exactly one thing: a file KEPT in the library has to carry
/// a name the picker's directory scan (`picker::has_story_ext`) will look inside.
pub fn content_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(ZIP_MAGIC) {
        return Some("zip");
    }
    if let Some(kind) = blorb::medium::DiskImage::detect(bytes) {
        return kind.extensions().first().copied();
    }
    if blorb::Blorb::is_blorb(bytes) {
        return Some("blb");
    }
    if bytes.starts_with(b"Glul") {
        return Some("ulx");
    }
    // A Z-code image proves itself by its HEADER, and by the same predicate the
    // loader will apply a moment later — `blorb::adf::looks_like_zcode`, ZMSD
    // §1.1's memory map and §11.1.6's file-length word (SQ-0889). Borrowed, not
    // restated: a bare version-byte test passes roughly 2.3% of arbitrary
    // containers, so a second copy here would admit files the loader then
    // refuses, and the fetch would report "not a story" only after writing one.
    if blorb::adf::looks_like_zcode(bytes) {
        return match bytes[0] {
            3 => Some("z3"),
            4 => Some("z4"),
            5 => Some("z5"),
            6 => Some("z6"),
            7 => Some("z7"),
            _ => Some("z8"),
        };
    }
    if std::str::from_utf8(bytes).is_ok_and(scott::looks_like_scott) {
        return Some("dat");
    }
    None
}

/// A one-line description of a payload that is NOT a story, for the failure
/// message. "lanthorn should say what it fetched and why it could not open it":
/// a 404 page, a login redirect and a PDF are three different mistakes and the
/// player can only tell which they made if we say.
pub fn describe_payload(bytes: &[u8]) -> String {
    let head: Vec<u8> = bytes.iter().take(512).copied().collect();
    let lower = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let kind = if lower.trim_start().starts_with("<!doctype html")
        || lower.trim_start().starts_with("<html")
        || lower.contains("<title>")
    {
        "a web page"
    } else if bytes.starts_with(b"%PDF") {
        "a PDF"
    } else if bytes.starts_with(b"\x1f\x8b") {
        "a gzip archive"
    } else if bytes.starts_with(b"\x89PNG") || bytes.starts_with(b"\xff\xd8\xff") {
        "an image"
    } else if bytes.is_empty() {
        "an empty response"
    } else {
        "not a story file of any kind lanthorn opens"
    };
    format!("the server sent {} ({} bytes)", kind, bytes.len())
}

// ── Naming the local file ────────────────────────────────────────────────────

/// Reduce an untrusted name to a safe basename — the final component under both
/// separators, control and path characters dropped, `.`/`..`/leading-dot names
/// refused. Unlike `ifdb_search::sanitize_filename` this does NOT require a
/// story extension, because a fetched file's extension is settled by its
/// CONTENT ([`local_filename`]) and plenty of legitimate download URLs end in
/// `/download` or a query string.
pub fn safe_basename(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    if base.is_empty() || base.starts_with('.') {
        return None;
    }
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | '\0' | ':'))
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned == ".." || cleaned.chars().all(|c| c == '.') {
        return None;
    }
    Some(cleaned)
}

/// The last path segment of a URL, percent-decoded, query and fragment stripped.
fn basename_from_url(url: &str) -> Option<String> {
    let no_frag = url.split('#').next().unwrap_or(url);
    let no_query = no_frag.split('?').next().unwrap_or(no_frag);
    let last = no_query.rsplit('/').next().unwrap_or(no_query);
    let decoded = percent_decode(last);
    (!decoded.is_empty()).then_some(decoded)
}

/// Pull a `filename` out of a `Content-Disposition` value (quoted or bare).
fn filename_from_disposition(value: &str) -> Option<String> {
    let idx = value.to_ascii_lowercase().find("filename=")?;
    let after = value[idx + "filename=".len()..].trim_start();
    let name = if let Some(rest) = after.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest)
    } else {
        after.split(';').next().unwrap_or(after).trim()
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// The name a fetched story is written under: the host's `Content-Disposition`
/// name if it survives sanitising, else the URL basename, else `story`; with an
/// extension the picker's scan understands, taken from the BYTES.
///
/// The bytes overrule the name whenever the name carries no openable extension,
/// so `…/download?id=7` serving a `.z5` lands as `download.z5` rather than as an
/// extensionless file the library would never list. A name that already ends in
/// an accepted extension is left alone even when the content disagrees, because
/// the loader sniffs content anyway and the player's own file name is the more
/// useful of the two.
///
/// `None` only when the bytes are not openable at all.
pub fn local_filename(disposition: Option<&str>, url: &str, bytes: &[u8]) -> Option<String> {
    let ext = content_extension(bytes)?;
    let stem = disposition
        .and_then(filename_from_disposition)
        .and_then(|n| safe_basename(&n))
        .or_else(|| basename_from_url(url).and_then(|n| safe_basename(&n)))
        .unwrap_or_else(|| "story".to_string());
    // `picker::has_story_ext` includes `.zip` (SQ-1086), so an archive keeps its
    // own name rather than being relabelled after the story inside it — the
    // loader unwraps one, and a `curses.zip` renamed `curses.z5` would be a file
    // whose name lies about what it holds.
    if crate::picker::has_story_ext(Path::new(&stem)) {
        return Some(stem);
    }
    Some(format!("{stem}.{ext}"))
}

/// Decode `%xx` escapes (and `+` → space) in a URL path segment; invalid
/// escapes are left verbatim.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

// ── The fetch ────────────────────────────────────────────────────────────────

/// A story that came off the network, and where it landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedStory {
    /// The address it was fetched from — shown by the keep prompt, because a
    /// player who typed three URLs needs to know which one they are answering.
    pub url: String,
    /// The local file the loader was handed.
    pub path: PathBuf,
}

impl FetchedStory {
    /// The basename the library copy would carry.
    pub fn filename(&self) -> String {
        self.path.file_name().and_then(|n| n.to_str()).unwrap_or("story").to_string()
    }
}

/// What a fetch turned out to be (SQ-1096).
///
/// Two answers rather than one, because a download that lanthorn can *play* and
/// a download that lanthorn can only *unpack* take different routes through the
/// boot: the first is handed to the loader and booted, the second has to be
/// offered to the player before anything else can happen at all. Making that an
/// enum rather than a flag on [`FetchedStory`] is deliberate — a caller that
/// ignored a flag would hand a zip of floppies to the engine as if it were a
/// story, and the failure would be a long way from the cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// A file the ordinary loader opens. Everything before SQ-1096 is this.
    Story(FetchedStory),
    /// A ZIP holding no story the loader can run, but one or more supported
    /// release disk images.
    DiskImages(FetchedArchive),
}

// ── A downloaded zip of disk images (SQ-1096) ────────────────────────────────
//
// A zip stays what SQ-1085 made it: a volume holding raw stories and their
// resources, classified by CONTENT. It does not learn to mount media, and
// `hints::extract_story` still knows exactly four kinds of story.
//
// What changes is the FETCH, which is a different question. Archive sites ship
// C64, Amiga and Apple II floppies inside zips as a matter of course, so a
// download that holds five `.dsk` files is the normal packaging of a multi-disk
// release rather than an oddity — and the fetch is deciding what to UNPACK, not
// what a file IS. Extension is the right signal for that question and carries
// none of SQ-1085's tension (a `.d64` is raw sectors with no magic at all, so
// there is no content answer to be had).
//
// **Only disk images come out.** Not a readme, not a cover scan, not a loose
// `.z5` sitting beside them, not an installer. Three reasons, and they are why
// the whitelist is the feature rather than a detail of it: the library directory
// is scanned on every launch, so a stray file there is a row nobody asked for;
// an arbitrary archive from an arbitrary URL is untrusted input, and "extract
// everything" is how a dotfile or an executable lands in a directory lanthorn
// reads; and the offer on screen says "keep this game in your library", not
// "unpack this archive".

/// One disk image inside a downloaded archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveImage {
    /// The name the archive stores it under — possibly with directories, e.g.
    /// `Journey/disks/journey_s1.dsk`.
    pub entry: String,
    /// What it lands in the library as: the final component only
    /// ([`safe_basename`]).
    ///
    /// **Flattened, never recreated.** A multi-disk release whose images landed
    /// in a subdirectory would not be found as siblings by
    /// `cli_host::disk_set::mount_at`, which scans the directory the named image
    /// sits in — so flattening is not tidiness, it is what makes a five-floppy
    /// set mount as one release. It also means no directory is ever created,
    /// which is one fewer way for an entry name to reach outside the library.
    pub name: String,
}

/// A ZIP that came off the network holding disk images and no runnable story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedArchive {
    /// The address it was fetched from — the prompt shows it, for the same
    /// reason [`FetchedStory`] carries one.
    pub url: String,
    /// The archive itself, in the download directory.
    pub path: PathBuf,
    /// The supported disk images inside it, in archive order. Never empty: an
    /// archive with none of these is not a [`Fetched::DiskImages`] at all.
    pub images: Vec<ArchiveImage>,
}

impl FetchedArchive {
    /// The archive's own basename, for the prompt.
    pub fn filename(&self) -> String {
        self.path.file_name().and_then(|n| n.to_str()).unwrap_or("download.zip").to_string()
    }

    /// The names the images would carry in the library, in archive order.
    pub fn names(&self) -> Vec<String> {
        self.images.iter().map(|i| i.name.clone()).collect()
    }
}

/// Does `name` end in an extension `blorb::medium` claims as a release disk
/// image?
///
/// Asked of the stored ENTRY NAME, which is the whole point: `blorb::medium`
/// enumerates every spelling lanthorn mounts, so this cannot drift from what
/// opening the same file loose would do. Case-insensitive, because archives are
/// written on every platform there is (`TRINITY1.D64` is how it ships).
pub fn is_disk_image_name(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => {
            blorb::medium::image_extensions().any(|e| e.eq_ignore_ascii_case(ext))
        }
        _ => false,
    }
}

/// Would writing `entry` reach outside the directory it is unpacked into?
///
/// An absolute name, a drive-qualified one, or any `..` component under either
/// separator. Only ever asked of an entry lanthorn is about to WRITE — the rest
/// of the archive is never touched, so its names cannot matter.
fn escapes_destination(entry: &str) -> bool {
    entry.starts_with('/')
        || entry.starts_with('\\')
        || entry.as_bytes().get(1) == Some(&b':')
        || entry.split(['/', '\\']).any(|c| c == "..")
}

/// The supported disk images inside the ZIP at `path`, in archive order.
///
/// Empty when it holds none. [`FetchError::UnsafeEntry`] when one of them names
/// a path that would escape the destination — the whole archive is refused, not
/// the one entry, because an archive that tries it has told us what it is.
pub fn zip_disk_images(path: &Path) -> Result<Vec<ArchiveImage>, FetchError> {
    let names = crate::hints::zip_entry_names(path).map_err(|e| FetchError::Io(e.to_string()))?;
    let mut out = Vec::new();
    for entry in names {
        if !is_disk_image_name(&entry) {
            continue;
        }
        if escapes_destination(&entry) {
            return Err(FetchError::UnsafeEntry(entry));
        }
        match safe_basename(&entry) {
            Some(name) if is_disk_image_name(&name) => out.push(ArchiveImage { entry, name }),
            _ => return Err(FetchError::UnsafeEntry(entry)),
        }
    }
    Ok(out)
}

/// Unpack every disk image in `archive` into `library_dir`, and answer where
/// each landed, sorted by name.
///
/// `mode` answers a collision with a file the library ALREADY held, exactly as
/// it does for a single kept story. A collision between two entries of the SAME
/// archive (`side_a/disk1.d64` and `side_b/disk1.d64` both flatten to
/// `disk1.d64`) is never a replace: the second lands beside the first under
/// `unique_dest`'s `-2` name, because whatever the player answered about their
/// own library, they cannot have meant "overwrite the file I am unpacking with
/// the next one out of the same zip".
pub fn unpack_disk_images(
    archive: &FetchedArchive,
    library_dir: &Path,
    mode: KeepMode,
) -> Result<Vec<PathBuf>, FetchError> {
    std::fs::create_dir_all(library_dir).map_err(|e| FetchError::Io(e.to_string()))?;
    let mut written: Vec<PathBuf> = Vec::new();
    for image in &archive.images {
        // Re-asked at the write, not trusted from the scan: the value that
        // decides where bytes land must be checked where the bytes land.
        if escapes_destination(&image.entry) || safe_basename(&image.name).as_deref() != Some(image.name.as_str()) {
            return Err(FetchError::UnsafeEntry(image.entry.clone()));
        }
        let plain = library_dir.join(&image.name);
        let dest = match mode {
            KeepMode::Replace if !written.contains(&plain) => plain,
            _ => unique_dest(library_dir, &image.name),
        };
        // Belt and braces over `safe_basename`, the same guard `keep_in_library`
        // applies: the destination must be a direct child of the library.
        if dest.parent() != Some(library_dir) {
            return Err(FetchError::UnsafeEntry(image.entry.clone()));
        }
        let want = image.entry.clone();
        let bytes = crate::hints::read_zip_entry(&archive.path, |n| n == want)
            .map_err(|e| FetchError::Io(e.to_string()))?
            .ok_or_else(|| FetchError::UnsafeEntry(image.entry.clone()))?;
        std::fs::write(&dest, &bytes).map_err(|e| FetchError::Io(e.to_string()))?;
        written.push(dest);
    }
    written.sort();
    Ok(written)
}

/// Do any of `archive`'s images already have a namesake in `library_dir`?
pub fn archive_collision(archive: &FetchedArchive, library_dir: &Path) -> bool {
    archive.images.iter().any(|i| library_dir.join(&i.name).exists())
}

/// Fetch `url` into `dir` and return what arrived.
///
/// The whole of the "is this a story?" question is answered twice, on purpose:
/// once over the bytes before anything is written ([`content_extension`]), so a
/// 404 page never reaches the disk, and once over the written file with the REAL
/// loader, so a zip with no story entry or a disk image with no boot file fails
/// with the loader's own words rather than ours. A file that fails the second
/// check is removed.
///
/// **The loader is asked FIRST, and that is what settles a zip holding both a
/// story and disk images** (SQ-1096): the story wins, because it is the thing
/// lanthorn can actually run, and the decision is the loader's own rather than
/// whichever entry the archive happened to store first. Only when the loader has
/// nothing at all is the archive asked whether it is a set of floppies, and only
/// then is the download kept for the prompt to offer.
pub fn fetch_to_dir(src: &dyn UrlSource, url: &str, dir: &Path) -> Result<Fetched, FetchError> {
    let payload = src.get(url)?;
    let name = match local_filename(payload.disposition.as_deref(), url, &payload.bytes) {
        Some(n) => n,
        None => return Err(FetchError::Unopenable(describe_payload(&payload.bytes))),
    };
    std::fs::create_dir_all(dir).map_err(|e| FetchError::Io(e.to_string()))?;
    let dest = unique_dest(dir, &name);
    std::fs::write(&dest, &payload.bytes).map_err(|e| FetchError::Io(e.to_string()))?;
    if let Err(e) = crate::hints::load_mounted_story(&dest) {
        if crate::hints::is_zip(&dest) {
            match zip_disk_images(&dest) {
                Ok(images) if !images.is_empty() => {
                    return Ok(Fetched::DiskImages(FetchedArchive {
                        url: url.to_string(),
                        path: dest,
                        images,
                    }));
                }
                // A hostile entry name is the archive's answer about itself, and
                // it outranks the loader's "no story here": say THAT, and leave
                // nothing on disk.
                Err(unsafe_entry @ FetchError::UnsafeEntry(_)) => {
                    let _ = std::fs::remove_file(&dest);
                    return Err(unsafe_entry);
                }
                _ => {}
            }
        }
        let _ = std::fs::remove_file(&dest);
        return Err(FetchError::Unopenable(format!(
            "{} — {e}",
            describe_payload(&payload.bytes)
        )));
    }
    Ok(Fetched::Story(FetchedStory { url: url.to_string(), path: dest }))
}

/// Where a fetch lands when nobody has yet said to keep it. Stable rather than
/// per-run so repeated fetches of the same story do not scatter (and
/// `unique_dest` keeps them from colliding); never cleaned by lanthorn — see the
/// module header.
pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("lanthorn-fetch")
}

// ── Keeping it ───────────────────────────────────────────────────────────────

/// What copying a fetched story into the library should do about a name the
/// library already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepMode {
    /// Replace the existing file of that name.
    Replace,
    /// Keep both — the new one lands beside it as `stem-2.ext` (`unique_dest`).
    KeepBoth,
}

/// Copy a fetched story into `library_dir`, returning the path written.
///
/// The temp copy is left where it is: it is the file the running session was
/// booted from, and its basename is the save key, so removing it under a live
/// game is the one thing this must not do.
pub fn keep_in_library(
    fetched: &Path,
    library_dir: &Path,
    mode: KeepMode,
) -> Result<PathBuf, FetchError> {
    let name = fetched
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(safe_basename)
        .ok_or(FetchError::NoFilename)?;
    std::fs::create_dir_all(library_dir).map_err(|e| FetchError::Io(e.to_string()))?;
    let dest = match mode {
        KeepMode::Replace => library_dir.join(&name),
        KeepMode::KeepBoth => unique_dest(library_dir, &name),
    };
    // Belt and braces over `safe_basename`: the destination must be a direct
    // child of the library directory, whatever the name claimed.
    if dest.parent() != Some(library_dir) {
        return Err(FetchError::NoFilename);
    }
    std::fs::copy(fetched, &dest).map_err(|e| FetchError::Io(e.to_string()))?;
    Ok(dest)
}

/// Does `library_dir` already hold a file named after `fetched`?
pub fn library_collision(fetched: &Path, library_dir: &Path) -> bool {
    fetched
        .file_name()
        .map(|n| library_dir.join(n))
        .is_some_and(|p| p.exists())
}

// ── Background downloads (the picker) ────────────────────────────────────────

/// One finished URL download, drained by the picker loop.
#[derive(Debug, Clone)]
pub struct UrlDlResult {
    pub url: String,
    pub outcome: Result<PathBuf, String>,
}

/// Spawns URL downloads and drains their results without blocking the UI —
/// the same thread-per-request-over-one-channel shape as
/// [`crate::hint_download::HintDownloader`], for the same reason: a handful of
/// these ever run, one per keypress.
pub struct UrlDownloader {
    tx: mpsc::Sender<UrlDlResult>,
    rx: mpsc::Receiver<UrlDlResult>,
    inflight: usize,
}

impl Default for UrlDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl UrlDownloader {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, inflight: 0 }
    }

    /// Begin fetching `url` into `dir`. Returns immediately.
    pub fn start(&mut self, url: String, dir: PathBuf) {
        self.inflight += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let src = HttpSource::new();
            let outcome = match fetch_to_dir(&src, &url, &dir) {
                Ok(Fetched::Story(f)) => Ok(f.path),
                // SQ-1096: the picker's URL box downloads STRAIGHT INTO the
                // library and has no prompt in front of it, so it is not the
                // surface that unpacks an archive — it says what the archive
                // holds and leaves nothing behind. Launching lanthorn with the
                // URL is the route that offers to unpack it.
                Ok(Fetched::DiskImages(a)) => {
                    let n = a.images.len();
                    let _ = std::fs::remove_file(&a.path);
                    Err(format!(
                        "this download holds {n} disk image{} and no story; lanthorn does not run \
                         disk images from inside a zip — launch `lanthorn <url>` to unpack them \
                         into your library",
                        if n == 1 { "" } else { "s" },
                    ))
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(UrlDlResult { url, outcome });
        });
    }

    /// Non-blocking drain of every download finished since the last call.
    pub fn drain(&mut self) -> Vec<UrlDlResult> {
        let done: Vec<_> = self.rx.try_iter().collect();
        self.inflight = self.inflight.saturating_sub(done.len());
        done
    }

    pub fn busy(&self) -> bool {
        self.inflight > 0
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;

    /// A minimal-but-coherent v5 story image — every clause of
    /// `blorb::adf::looks_like_zcode` satisfied, because that is the predicate
    /// both `content_extension` and the real loader apply.
    pub(crate) fn zcode_v5() -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[0] = 5;
        b[0x04..0x06].copy_from_slice(&96u16.to_be_bytes()); // high memory
        b[0x08..0x0A].copy_from_slice(&100u16.to_be_bytes()); // dictionary (static)
        b[0x0A..0x0C].copy_from_slice(&64u16.to_be_bytes()); // objects (dynamic)
        b[0x0C..0x0E].copy_from_slice(&70u16.to_be_bytes()); // globals (dynamic)
        b[0x0E..0x10].copy_from_slice(&96u16.to_be_bytes()); // static memory base
        b[0x12..0x18].copy_from_slice(b"890714"); // serial
        b[0x1A..0x1C].copy_from_slice(&(128u16 / 4).to_be_bytes()); // file length / 4
        b
    }

    struct Canned {
        payload: Vec<u8>,
        disposition: Option<String>,
    }
    impl UrlSource for Canned {
        fn get(&self, _url: &str) -> Result<Payload, FetchError> {
            Ok(Payload { disposition: self.disposition.clone(), bytes: self.payload.clone() })
        }
    }
    struct Dead;
    impl UrlSource for Dead {
        fn get(&self, _url: &str) -> Result<Payload, FetchError> {
            Err(FetchError::Transport("404 Not Found".into()))
        }
    }

    /// Unwrap a fetch that must be a runnable story (SQ-1096).
    fn only_story(got: Fetched) -> FetchedStory {
        match got {
            Fetched::Story(s) => s,
            Fetched::DiskImages(a) => panic!("expected a story, got {} disk images", a.images.len()),
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        crate::scratch_dir(&format!("squrl-{tag}"))
    }

    #[test]
    fn recognises_http_and_https_and_nothing_else() {
        assert!(is_story_url("http://example.org/curses.z5"));
        assert!(is_story_url("https://example.org/curses.z5"));
        assert!(is_story_url("HTTPS://EXAMPLE.ORG/x.z5"), "scheme is case-insensitive");
        assert!(is_story_url("  https://example.org/x.z5  "), "surrounding space is trimmed");
        // Paths, not URLs.
        assert!(!is_story_url("stories/curses.z5"));
        assert!(!is_story_url("/abs/path/curses.z5"));
        assert!(!is_story_url("C:\\games\\curses.z5"));
        assert!(!is_story_url("file:///tmp/curses.z5"));
        assert!(!is_story_url("ftp://example.org/x.z5"));
        // Scheme with no host is not an address.
        assert!(!is_story_url("https://"));
        assert!(!is_story_url(""));
    }

    #[test]
    fn a_declined_scheme_says_why_instead_of_looking_like_a_missing_file() {
        assert!(declined_scheme("file:///tmp/x.z5").unwrap().contains("pass the path itself"));
        assert!(declined_scheme("ftp://example.org/x.z5").unwrap().starts_with("ftp://"));
        // Not scheme-shaped, and a real URL, both answer None.
        assert_eq!(declined_scheme("stories/x.z5"), None);
        assert_eq!(declined_scheme("https://example.org/x.z5"), None);
    }

    /// A remote name must never escape the directory it is written into.
    #[test]
    fn safe_basename_refuses_everything_that_escapes() {
        assert_eq!(safe_basename("curses.z5").as_deref(), Some("curses.z5"));
        assert_eq!(safe_basename("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(safe_basename("..\\..\\windows\\system32\\x").as_deref(), Some("x"));
        assert_eq!(safe_basename("/etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(safe_basename(".."), None);
        assert_eq!(safe_basename("."), None);
        assert_eq!(safe_basename(".bashrc"), None, "a leading dot is never a story file");
        assert_eq!(safe_basename(""), None);
        assert_eq!(safe_basename("   "), None);
        // Control characters and a Windows drive colon are dropped, not kept.
        assert_eq!(safe_basename("cur\u{7}ses.z5").as_deref(), Some("curses.z5"));
        assert_eq!(safe_basename("C:evil.z5").as_deref(), Some("Cevil.z5"));
    }

    #[test]
    fn content_extension_reads_the_bytes_not_the_name() {
        assert_eq!(content_extension(&zcode_v5()), Some("z5"));
        // Version 3 packs addresses by 2, so the declared length word changes
        // with the version — the header has to stay coherent, not just relabel.
        let mut v3 = zcode_v5();
        v3[0] = 3;
        v3[0x1A..0x1C].copy_from_slice(&(128u16 / 2).to_be_bytes());
        assert_eq!(content_extension(&v3), Some("z3"));
        // A version byte alone proves nothing: this is the 2.3%-of-junk case the
        // borrowed predicate exists to reject (SQ-0889).
        let mut liar = vec![0u8; 128];
        liar[0] = 5;
        assert_eq!(content_extension(&liar), None);
        assert_eq!(content_extension(b"Glul\0\0\0\0"), Some("ulx"));
        assert_eq!(content_extension(b"PK\x03\x04rest of a zip"), Some("zip"));
        // Not a story of any kind.
        assert_eq!(content_extension(b"<!DOCTYPE html><html>404</html>"), None);
        assert_eq!(content_extension(b""), None);
        // Too short to hold a Z-machine header, whatever its first byte says.
        assert_eq!(content_extension(&[5u8; 10]), None);
    }

    #[test]
    fn describe_payload_names_what_actually_arrived() {
        assert!(describe_payload(b"<!DOCTYPE html><html>404</html>").contains("a web page"));
        assert!(describe_payload(b"%PDF-1.4 ...").contains("a PDF"));
        assert!(describe_payload(b"").contains("an empty response"));
        assert!(describe_payload(b"\x89PNG\r\n").contains("an image"));
        // And always the byte count, which is what distinguishes a truncated
        // download from a login page.
        assert!(describe_payload(b"12345").contains("5 bytes"));
    }

    #[test]
    fn the_filename_comes_from_the_url_and_the_extension_from_the_bytes() {
        let z = zcode_v5();
        assert_eq!(local_filename(None, "https://x.org/curses.z5", &z).as_deref(), Some("curses.z5"));
        // No usable extension in the URL → the bytes supply one.
        assert_eq!(local_filename(None, "https://x.org/download?id=7", &z).as_deref(), Some("download.z5"));
        // Nothing nameable at all → still a name, still openable.
        assert_eq!(local_filename(None, "https://x.org/", &z).as_deref(), Some("story.z5"));
        // Content-Disposition wins when it survives sanitising…
        assert_eq!(
            local_filename(Some("attachment; filename=\"Curses r16.z5\""), "https://x.org/dl", &z).as_deref(),
            Some("Curses r16.z5")
        );
        // …and cannot escape the directory when it tries. `../../.bashrc` has no
        // safe basename at all (a leading dot is never a story file), so the
        // disposition is discarded and the URL's own basename is used instead.
        assert_eq!(
            local_filename(Some("attachment; filename=\"../../.bashrc\""), "https://x.org/dl", &z).as_deref(),
            Some("dl.z5"),
            "a traversal name is refused outright, not merely trimmed"
        );
        // A traversal that DOES leave a plausible basename keeps only that.
        assert_eq!(
            local_filename(Some("attachment; filename=\"../../evil.z5\""), "https://x.org/dl", &z).as_deref(),
            Some("evil.z5")
        );
        // A zip keeps its own name: the loader unwraps it, we do not rename it
        // after the story inside.
        assert_eq!(
            local_filename(None, "https://x.org/games.zip", b"PK\x03\x04junk").as_deref(),
            Some("games.zip")
        );
        // Unopenable bytes have no local name at all.
        assert_eq!(local_filename(None, "https://x.org/curses.z5", b"<html>404</html>"), None);
    }

    // ── SQ-1096: recognising an archive of floppies ──────────────────────────

    /// Every way an entry name can try to leave the directory it is unpacked
    /// into. Refused OUTRIGHT — the archive, not just the entry — because
    /// trimming to the final component silently rewards the attempt.
    #[test]
    fn every_shape_of_traversal_is_refused() {
        for hostile in [
            "../evil.d64",
            "../../../etc/evil.d64",
            "a/../../evil.d64",
            "..\\evil.d64",
            "/etc/evil.d64",
            "\\windows\\evil.d64",
            "C:\\windows\\evil.d64",
        ] {
            assert!(escapes_destination(hostile), "{hostile} must be refused");
        }
        for ok in ["disk1.d64", "Journey/disks/journey_s1.dsk", "a/b/c/side.adf", "..hidden.d64"] {
            assert!(!escapes_destination(ok), "{ok} is an ordinary name");
        }
    }

    /// A `..` inside a name is a component test, not a substring test — a file
    /// genuinely called `Zork.. Solid Gold.d64` is not an attack.
    #[test]
    fn a_dotdot_inside_a_component_is_not_traversal() {
        assert!(!escapes_destination("Zork.. Solid Gold.d64"));
        assert!(escapes_destination("Zork/../../gold.d64"));
    }

    /// The whitelist, spelled out: only what `blorb::medium` claims.
    #[test]
    fn only_medium_spellings_are_disk_images() {
        assert!(is_disk_image_name("TRINITY1.D64"));
        assert!(is_disk_image_name("Journey/disks/journey_s1.dsk"));
        assert!(!is_disk_image_name("readme.txt"));
        assert!(!is_disk_image_name("curses.z5"));
        assert!(!is_disk_image_name("bundle.zip"));
        assert!(!is_disk_image_name("d64"), "an extension is not a filename");
        assert!(!is_disk_image_name(".d64"), "…and a bare dotfile has no stem");
    }

    #[test]
    fn fetch_writes_a_story_and_hands_back_a_path_the_loader_opens() {
        let dir = tmp("ok");
        let src = Canned { payload: zcode_v5(), disposition: None };
        let got = only_story(fetch_to_dir(&src, "https://x.org/curses.z5", &dir).expect("fetch"));
        assert_eq!(got.path, dir.join("curses.z5"));
        assert_eq!(got.url, "https://x.org/curses.z5");
        assert_eq!(std::fs::read(&got.path).unwrap(), zcode_v5());
        // The point of the whole feature: the ordinary loader opens it.
        assert!(crate::hints::load_mounted_story(&got.path).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_fetch_of_the_same_name_never_clobbers_the_first() {
        let dir = tmp("dup");
        let src = Canned { payload: zcode_v5(), disposition: None };
        let a = only_story(fetch_to_dir(&src, "https://x.org/curses.z5", &dir).unwrap());
        let b = only_story(fetch_to_dir(&src, "https://x.org/curses.z5", &dir).unwrap());
        assert_eq!(a.path, dir.join("curses.z5"));
        assert_eq!(b.path, dir.join("curses-2.z5"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "A URL that is not a story at all" — an HTML page must not land on disk
    /// as `curses.z5`, and the failure must say what did arrive.
    #[test]
    fn an_html_page_is_refused_and_leaves_nothing_behind() {
        let dir = tmp("html");
        let src = Canned {
            payload: b"<!DOCTYPE html><html><title>Sign in</title></html>".to_vec(),
            disposition: None,
        };
        let err = fetch_to_dir(&src, "https://x.org/curses.z5", &dir).unwrap_err();
        match &err {
            FetchError::Unopenable(msg) => assert!(msg.contains("a web page"), "{msg}"),
            other => panic!("expected Unopenable, got {other:?}"),
        }
        assert!(!dir.join("curses.z5").exists(), "nothing written");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "no partial files left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A zip that holds no story passes the cheap sniff and fails the loader —
    /// which is why the loader gets the second word, and why the file it wrote
    /// is removed again.
    #[test]
    fn a_zip_with_no_story_inside_fails_and_is_removed() {
        let dir = tmp("emptyzip");
        // An empty ZIP (end-of-central-directory only) still starts PK, so the
        // sniff admits it; `read_story_file` then finds no `.z3/.z5/.z8` entry.
        let mut empty_zip = b"PK\x03\x04".to_vec();
        empty_zip.extend_from_slice(&[0u8; 26]);
        let src = Canned { payload: empty_zip, disposition: None };
        let err = fetch_to_dir(&src, "https://x.org/games.zip", &dir).unwrap_err();
        assert!(matches!(err, FetchError::Unopenable(_)), "{err:?}");
        assert!(!dir.join("games.zip").exists(), "the written file is removed again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_transport_failure_is_reported_verbatim() {
        let dir = tmp("dead");
        let err = fetch_to_dir(&Dead, "https://x.org/gone.z5", &dir).unwrap_err();
        assert_eq!(err.to_string(), "404 Not Found");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeping_copies_into_the_library_and_leaves_the_fetched_file_alone() {
        let src_dir = tmp("keep-src");
        let lib = tmp("keep-lib");
        let fetched = src_dir.join("curses.z5");
        std::fs::write(&fetched, zcode_v5()).unwrap();

        let kept = keep_in_library(&fetched, &lib, KeepMode::KeepBoth).unwrap();
        assert_eq!(kept, lib.join("curses.z5"));
        assert_eq!(std::fs::read(&kept).unwrap(), zcode_v5());
        assert!(fetched.exists(), "the running game's file must survive the copy");

        // A second keep of the same name never overwrites under KeepBoth…
        assert!(library_collision(&fetched, &lib));
        let both = keep_in_library(&fetched, &lib, KeepMode::KeepBoth).unwrap();
        assert_eq!(both, lib.join("curses-2.z5"));
        // …and does under Replace, which is the only way to lose the old file
        // and is never reached without the player choosing it.
        std::fs::write(lib.join("curses.z5"), b"older").unwrap();
        let replaced = keep_in_library(&fetched, &lib, KeepMode::Replace).unwrap();
        assert_eq!(replaced, lib.join("curses.z5"));
        assert_eq!(std::fs::read(&replaced).unwrap(), zcode_v5());

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&lib);
    }

    /// The library write must land inside the library directory whatever the
    /// fetched file is called.
    #[test]
    fn keeping_cannot_write_outside_the_library_directory() {
        let lib = tmp("escape-lib");
        let outside = tmp("escape-out");
        std::fs::write(outside.join("evil"), b"x").unwrap();
        // `keep_in_library` names the destination from the BASENAME only, so
        // even a fetched path several directories up lands in the library.
        let kept = keep_in_library(&outside.join("evil"), &lib, KeepMode::KeepBoth).unwrap();
        assert_eq!(kept.parent(), Some(lib.as_path()));
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn the_download_dir_is_under_the_system_temp_dir() {
        assert!(download_dir().starts_with(std::env::temp_dir()));
        assert_eq!(download_dir().file_name().unwrap(), "lanthorn-fetch");
    }

    #[test]
    fn downloader_starts_empty_and_not_busy() {
        let mut dl = UrlDownloader::new();
        assert!(!dl.busy());
        assert!(dl.drain().is_empty());
    }

    #[test]
    fn user_agent_identifies_lanthorn_and_its_repo() {
        let ua = user_agent();
        assert!(ua.starts_with("lanthorn/"));
        assert!(ua.contains("github.com/sharkusk/lanthorn"));
    }
}
