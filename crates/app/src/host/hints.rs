//! Open a story's InvisiClues-style hint session (SQ-1586).
//!
//! Resolving where a story's hint file lives and booting its companion
//! Z-machine VM used to be bin-only, inline in the TUI's `main.rs`
//! (`open_hints`/`hint_opening`), so an embedding host that is not a terminal
//! had no way to open hints at all. [`open`] is that extraction; [`available`]
//! answers "would `open` find something automatically?" cheaply, without
//! booting a VM.
//!
//! [`available`] and [`open`] share ONE resolution rule —
//! [`hints::resolve_hint_source`] — so the two questions "can I open hints?"
//! and "does opening hints work?" can never disagree. That is deliberately
//! NOT the rule the library scan uses to decide whether a sidecar's own row is
//! hidden ([`crate::picker`]'s `associate_hint_sidecars`, name/title/identity
//! matching only): `resolve_hint_source` also looks inside zips and the
//! story's own container, and is the one place a host must ask.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::data_roots::DataRoots;
use crate::hint_download::{HintDest, HintDownloader};
use crate::picker::{HintStatus, StoryEntry};
use crate::hints::{self, HintIndex, HintResolution, HintStory};
use crate::session::{GameSession, InputKind};
use crate::state::{HintSession, HintSource};

/// Whether [`open`] would find a hint source to boot, without booting one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintAvailability {
    /// A hint file resolves; `open` will attempt to boot it.
    Available,
    /// Several hint files tie and the player must pick one (SQ-1690); `open`
    /// returns `Ok(None)` until a pick is [`remember`]ed. The candidates are
    /// absolute paths, in a stable order.
    Choose(Vec<PathBuf>),
    /// No hint source resolves automatically; `open` returns `Ok(None)`.
    None,
}

/// Whether a hint source resolves for `story_path`/`story` — the SAME question
/// [`open`] itself asks via [`hints::resolve_hint_source`], so this can never
/// say yes to a story `open` then fails to find anything for (or vice versa).
/// `story` carries the IFID, the title and the game's documents folder, which is
/// searched first (SQ-1690).
///
/// Cheap: resolution reads the index and the filesystem (directory listing,
/// maybe a zip's entry table) but never boots a VM.
pub fn available(story_path: &Path, story: HintStory<'_>, index: &HintIndex) -> HintAvailability {
    available_with_tuid(story_path, story, None, index)
}

/// [`available`] for a story that may carry an IFDB tuid.
pub fn available_with_tuid(
    story_path: &Path,
    story: HintStory<'_>,
    tuid: Option<&str>,
    index: &HintIndex,
) -> HintAvailability {
    match hints::resolve_hint_source_with_tuid(story_path, story, tuid, index) {
        HintResolution::File(_) | HintResolution::ZipEntry { .. } => HintAvailability::Available,
        HintResolution::Choose(c) => HintAvailability::Choose(c),
        HintResolution::AskUser | HintResolution::None => HintAvailability::None,
    }
}

/// Remember that `chosen` is the hint file for `ifid` (the answer to a
/// [`HintAvailability::Choose`]): the same per-IFID association table
/// ([`hints::save_hint_assoc`], `<user_dir>/hints/index.toml`) every other
/// resolution reads, so the next [`available`]/[`open`] finds it.
pub fn remember(user_dir: &Path, ifid: &str, chosen: &Path) -> std::io::Result<()> {
    hints::save_hint_assoc(user_dir, ifid, chosen)
}

/// Why [`open`] could not hand back a booted hint session, once a hint source
/// DID resolve — the TUI's own diagnostic text for each failure, carried here
/// unchanged so a host sees exactly what the TUI always has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintOpenError {
    /// The resolved hint file's bytes could not be loaded as a Z-code story.
    ReadFailed(String),
    /// The resolved zip could not be opened, or reading its entry failed.
    ZipReadFailed(String),
    /// Resolution named a zip entry that could not be found when reading it.
    ZipEntryNotFound,
    /// The hint file's bytes did not boot as a Z-machine story.
    BootFailed(String),
    /// A chosen hint file could not be recorded as the story's (SQ-1694).
    RememberFailed(String),
}

impl std::fmt::Display for HintOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HintOpenError::ReadFailed(e) => write!(f, "hints: cannot read hint file: {e}"),
            HintOpenError::ZipReadFailed(e) => write!(f, "hints: cannot read zip entry: {e}"),
            HintOpenError::ZipEntryNotFound => write!(f, "hints: hint entry not found in zip"),
            HintOpenError::BootFailed(e) => write!(f, "hints: failed to load hint VM: {e}"),
            HintOpenError::RememberFailed(e) => write!(f, "hints: cannot remember the choice: {e}"),
        }
    }
}

impl std::error::Error for HintOpenError {}

/// The TUI's text for [`open`] returning `Ok(None)` — no hint source resolved
/// automatically (SQ-1690). `documents` is the game's documents folder when it is
/// linked to IFDB (see [`HintStory::documents`]); `None` means unlinked, and the
/// message says to link first. Carried here so a host needs no copy of the wording.
pub fn no_hint_message(documents: Option<&Path>) -> String {
    match documents {
        Some(dir) => format!(
            "no hint file found — put a hint file (a Z-code .z5, such as zork1inv.z5) in this game's documents folder, {}, or use Download hints",
            dir.display()
        ),
        None => "no hint file found — link this game to IFDB so it has a documents folder to put a hint file in, or use Download hints".to_string(),
    }
}

/// Resolve and boot `story_path`'s hint session.
///
/// `story` is the story's own IFID/title (the mounted story's, for a disk image)
/// plus its documents folder, searched first.
/// `index` is the loaded per-IFID association table ([`hints::load_hint_index`]);
/// `dict_words` is the STORY's OWN dictionary (not the hint file's) — it drives
/// [`hints::story_supports_hint`], which decides `HintSession::builtin_hint`
/// (the "this game has its own hints — type HINT" suggestion).
///
/// `screen` is the `(rows, cols)` the companion should boot at (its host's real panel size);
/// `None` boots the Z-machine's default 80x24. A hint program reads its width at boot --
/// centring its title, wrapping its hint screens -- so a size given later is not the same as
/// one given here (SQ-1753).
///
/// - `Ok(None)`: resolution found nothing to open automatically
///   ([`HintResolution::AskUser`]/[`HintResolution::None`]) or a choice is
///   pending ([`HintAvailability::Choose`]) — [`no_hint_message`] is the TUI's
///   text for the former.
/// - `Err(_)`: a hint source resolved but could not become a running session.
/// - `Ok(Some(_))`: a ready [`HintSession`], its InvisiClues narrow-screen
///   opening banner already skipped when `cfg.hint_skip_screen_warning` is on
///   and the boot output is that banner (see [`hint_opening`]).
pub fn open(
    story_path: &Path,
    story: HintStory<'_>,
    index: &HintIndex,
    dict_words: &[String],
    cfg: &Config,
    screen: Option<(u16, u16)>,
) -> Result<Option<HintSession>, HintOpenError> {
    open_with_tuid(story_path, story, None, index, dict_words, cfg, screen)
}

/// [`open`] for a story that may carry an IFDB tuid.
pub fn open_with_tuid(
    story_path: &Path,
    story: HintStory<'_>,
    tuid: Option<&str>,
    index: &HintIndex,
    dict_words: &[String],
    cfg: &Config,
    screen: Option<(u16, u16)>,
) -> Result<Option<HintSession>, HintOpenError> {
    let builtin_hint = hints::story_supports_hint(dict_words.iter().cloned());
    let resolution = hints::resolve_hint_source_with_tuid(story_path, story, tuid, index);

    let (bytes, label) = match resolution {
        HintResolution::File(p) => {
            let bytes = hints::load_story_bytes(&p).map_err(|e| HintOpenError::ReadFailed(e.to_string()))?;
            let label = p.file_name().and_then(|n| n.to_str()).unwrap_or("Hints").to_owned();
            (bytes, label)
        }
        HintResolution::ZipEntry { zip_path, entry } => {
            let pred = |name: &str| name == entry;
            let bytes = hints::read_zip_entry(&zip_path, pred)
                .map_err(|e| HintOpenError::ZipReadFailed(e.to_string()))?
                .ok_or(HintOpenError::ZipEntryNotFound)?;
            let label = entry.rsplit('/').next().unwrap_or(&entry).to_owned();
            (bytes, label)
        }
        HintResolution::Choose(_) | HintResolution::AskUser | HintResolution::None => return Ok(None),
    };

    let mut vm = GameSession::new_with_trace(
        bytes,
        cfg.honor_game_colours,
        false,
        cfg.interpreter_number,
        false,
        Vec::new(),
        None,
        None,
        screen,
    )
    .map_err(|e| HintOpenError::BootFailed(format!("{e:?}")))?;
    vm.machine.undo_cap = cfg.undo_levels;
    let opening = hint_opening(&mut vm, cfg.hint_skip_screen_warning);
    let transcript: Vec<String> = opening.split('\n').map(|l| l.to_owned()).collect();

    Ok(Some(HintSession {
        source: HintSource::Zcode(vm),
        transcript,
        scroll: 0,
        clear_anchor: None,
        scroll_anim: None,
        input: String::new(),
        label,
        builtin_hint,
    }))
}

/// What [`start`] did (SQ-1694).
pub enum HintStart {
    /// A hint session booted and is ready.
    Started(Box<HintSession>),
    /// The picked program is the one already running; nothing to do.
    AlreadyRunning,
    /// Several hint files tie; the player must pick one (answer with `picked`).
    Choose(Vec<PathBuf>),
    /// Nothing resolved; the text is [`no_hint_message`].
    NoHint(String),
    /// A source resolved (or a pick could not be remembered) but no session booted.
    Failed(HintOpenError),
}

/// True when `picked` is the hint program the running session (whose
/// [`HintSession::label`] is `running_label`) already shows.
pub fn already_running(running_label: Option<&str>, picked: &Path) -> bool {
    running_label.is_some() && running_label == picked.file_name().and_then(|n| n.to_str())
}

/// Start a story's hint session, the whole decision in one host-level call: a
/// `picked` file (the answer to an earlier [`HintStart::Choose`]) is remembered
/// first, unless it is the program already running (`running_label`, a no-op);
/// then a tie becomes the chooser, nothing found becomes [`no_hint_message`], and
/// anything else is [`open`]ed. `story` carries the documents folder; `screen` is
/// [`open`]'s boot size.
pub fn start(
    story_path: &Path,
    story: HintStory<'_>,
    picked: Option<&Path>,
    running_label: Option<&str>,
    dict_words: &[String],
    cfg: &Config,
    screen: Option<(u16, u16)>,
) -> HintStart {
    start_with_tuid(story_path, story, None, picked, running_label, dict_words, cfg, screen)
}

/// [`start`] for a story that may carry an IFDB tuid.
pub fn start_with_tuid(
    story_path: &Path,
    story: HintStory<'_>,
    tuid: Option<&str>,
    picked: Option<&Path>,
    running_label: Option<&str>,
    dict_words: &[String],
    cfg: &Config,
    screen: Option<(u16, u16)>,
) -> HintStart {
    if let Some(p) = picked {
        if already_running(running_label, p) {
            return HintStart::AlreadyRunning;
        }
        if let Err(e) = remember(&cfg.user_dir, story.ifid, p) {
            return HintStart::Failed(HintOpenError::RememberFailed(e.to_string()));
        }
    }
    let index = hints::load_hint_index(&cfg.user_dir);
    if let HintAvailability::Choose(candidates) = available_with_tuid(story_path, story, tuid, &index) {
        return HintStart::Choose(candidates);
    }
    match open_with_tuid(story_path, story, tuid, &index, dict_words, cfg, screen) {
        Ok(Some(session)) => HintStart::Started(Box::new(session)),
        Ok(None) => HintStart::NoHint(no_hint_message(story.documents)),
        Err(e) => HintStart::Failed(e),
    }
}

/// The status line when a download is already running (browser and Hints tab).
pub const ALREADY_DOWNLOADING: &str = "Already downloading hints…";

/// Begin the download of the hint file for `ifid`, if one exists: the file goes to
/// `documents` when the game is IFDB-linked, else beside the story. `false` when
/// there is nothing to fetch. The ONE place a download is launched, shared by the
/// browser ([`start_story_download`]) and the running game ([`start_game_download`]).
fn launch_download(
    downloader: &mut HintDownloader,
    ifid: &str,
    tuid: Option<&str>,
    story_path: &Path,
    disk_entry: Option<&str>,
    title: &str,
    documents: Option<&Path>,
) -> bool {
    let Some(dl) = hints::hint_download_for_with_tuid(ifid, tuid) else {
        return false;
    };
    let dest = HintDest::for_story(story_path, &dl.filename, documents.map(Path::to_path_buf));
    downloader.start(dl.url, dest, story_path.to_path_buf(), disk_entry.map(str::to_owned), title.to_owned());
    true
}

/// Start the hint download for a story-browser row and return the status line.
/// Busy, already-has ([`crate::picker::hint_status`], documents folder included)
/// and nothing-to-fetch each answer without starting; otherwise the file goes to
/// the game's documents folder when it is IFDB-linked, else beside the story.
pub fn start_story_download(
    downloader: &mut HintDownloader,
    entry: &StoryEntry,
    roots: &DataRoots,
    index: &HintIndex,
) -> String {
    if downloader.busy() {
        return ALREADY_DOWNLOADING.to_string();
    }
    if matches!(crate::picker::hint_status(entry, roots, index), HintStatus::File(_)) {
        return format!("{} already has a hint file", entry.title);
    }
    let documents = crate::picker::entry_documents_dir(entry, roots);
    if !launch_download(
        downloader,
        &entry.meta.ifid,
        entry.meta.ifdb_tuid.as_deref(),
        &entry.path,
        entry.meta.disk_entry.as_deref(),
        &entry.title,
        documents.as_deref(),
    ) {
        return format!("No InvisiClues found for {}", entry.title);
    }
    format!("Downloading hints for {}…", entry.title)
}

/// Start the hint download for the RUNNING game and return the status line
/// (SQ-1701). `session_running` is whether a hint session is already open (a
/// hint file is in hand); `disk_entry` names the story on a multi-story disk
/// image; `documents` is the game's documents folder when IFDB-linked. Busy,
/// already-has and nothing-to-fetch each answer without starting.
pub fn start_game_download(
    downloader: &mut HintDownloader,
    ifid: &str,
    title: &str,
    story_path: &Path,
    disk_entry: Option<&str>,
    documents: Option<&Path>,
    session_running: bool,
) -> String {
    start_game_download_with_tuid(downloader, ifid, None, title, story_path, disk_entry, documents, session_running)
}

/// [`start_game_download`] for a game that may carry an IFDB tuid.
#[allow(clippy::too_many_arguments)]
pub fn start_game_download_with_tuid(
    downloader: &mut HintDownloader,
    ifid: &str,
    tuid: Option<&str>,
    title: &str,
    story_path: &Path,
    disk_entry: Option<&str>,
    documents: Option<&Path>,
    session_running: bool,
) -> String {
    if downloader.busy() {
        ALREADY_DOWNLOADING.to_string()
    } else if session_running {
        "This story already has a hint file".to_string()
    } else if launch_download(downloader, ifid, tuid, story_path, disk_entry, title, documents) {
        "Downloading hints…".to_string()
    } else {
        "No InvisiClues found for this story".to_string()
    }
}

/// Where the running game's hints stand (SQ-1701): [`crate::picker::HintStatus`]
/// for a game being played, which has no library row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameHintStatus {
    /// A hint file resolves; [`open`] will attempt to boot it.
    Available,
    /// Several hint files tie; the player picks one.
    Choose(Vec<PathBuf>),
    /// No file, but a matching InvisiClues can be downloaded.
    Downloadable,
    None,
}

/// [`GameHintStatus`] for a running game — [`available`] first (one resolution
/// rule), then whether the catalogue has a download for the story's IFID.
pub fn game_hint_status(story_path: &Path, story: HintStory<'_>, index: &HintIndex) -> GameHintStatus {
    game_hint_status_with_tuid(story_path, story, None, index)
}

/// [`game_hint_status`] for a game that may carry an IFDB tuid.
pub fn game_hint_status_with_tuid(
    story_path: &Path,
    story: HintStory<'_>,
    tuid: Option<&str>,
    index: &HintIndex,
) -> GameHintStatus {
    match available_with_tuid(story_path, story, tuid, index) {
        HintAvailability::Available => GameHintStatus::Available,
        HintAvailability::Choose(c) => GameHintStatus::Choose(c),
        HintAvailability::None if hints::hint_download_for_with_tuid(story.ifid, tuid).is_some() => GameHintStatus::Downloadable,
        HintAvailability::None => GameHintStatus::None,
    }
}

/// The chooser's prompt line (without the TUI's key help), shown above the tied
/// candidates (SQ-1694).
pub const CHOOSE_PROMPT: &str = "Several hint files could be this game's \u{2014} pick one";

/// InvisiClues narrow-screen warning auto-skipped.
///
/// The izm hint files open on a "your screen is only N characters wide…"
/// banner and wait for a keypress before showing the topic menu (the menu
/// lives in the upper window). When the boot output is that banner, press one
/// key here so the player lands straight on the menu; the keypress erases the
/// banner. If the output isn't the banner (or the file isn't waiting for a
/// key), fall back to the raw opening — no harm, the banner just shows as
/// before.
///
/// Gated on `skip_warning` (the `hint_skip_screen_warning` config, default
/// on); when off, the banner is left in place for the player to dismiss.
pub fn hint_opening(vm: &mut GameSession, skip_warning: bool) -> String {
    let opening = vm.take_transcript();
    if skip_warning
        && hints::is_narrow_screen_warning(&opening)
        && matches!(vm.pending_input(), InputKind::Char)
    {
        return vm.submit_char(b' ').transcript;
    }
    opening
}
