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
    match hints::resolve_hint_source(story_path, story, index) {
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
}

impl std::fmt::Display for HintOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HintOpenError::ReadFailed(e) => write!(f, "hints: cannot read hint file: {e}"),
            HintOpenError::ZipReadFailed(e) => write!(f, "hints: cannot read zip entry: {e}"),
            HintOpenError::ZipEntryNotFound => write!(f, "hints: hint entry not found in zip"),
            HintOpenError::BootFailed(e) => write!(f, "hints: failed to load hint VM: {e}"),
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
) -> Result<Option<HintSession>, HintOpenError> {
    let builtin_hint = hints::story_supports_hint(dict_words.iter().cloned());
    let resolution = hints::resolve_hint_source(story_path, story, index);

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

    let mut vm = GameSession::new(bytes, cfg.honor_game_colours, false, cfg.interpreter_number)
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
