//! Pre-game story picker: when a directory is passed at launch instead of a
//! story file, scan it for Z-machine stories and let the user choose one.
//!
//! Metadata (title, author, …) is resolved cheaply (no game is run) by
//! precedence, per field: a blorb's own `IFmd` chunk, then a fetched IFDB
//! sidecar, then (title only) the known-title table keyed by the IFID, then
//! the filename stem. See `resolve`.

use std::path::{Path, PathBuf};

use crate::hints;

/// The VM engine a story runs on (version-agnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    ZCode,
    Glulx,
    Scott,
}

/// One blorb resource-index entry, string-rendered for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkInfo {
    pub usage: String,      // "Exec" | "Pict" | "Snd " | "Data" …
    pub number: u32,
    pub chunk_type: String, // "ZCOD" | "GLUL" | "PNG " | "OGGV" …
    pub len: usize,
    /// Parsed format detail (e.g. "15.4 kHz · 8-bit · mono · 2.2s" for a sound,
    /// "800×600 · 32bpp" for an image). `None` when the resource isn't a
    /// sound/image, or its header couldn't be parsed.
    pub detail: Option<String>,
}

/// Best-effort static feature signals. Glulx-unknowable features are `None`/false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Features {
    pub sound: bool,
    pub graphics: bool,
    pub colour: Option<bool>, // Z: Some(bit6); Glulx: None (runtime Glk → omit)
    pub hints: bool,          // folded in from StoryAux when the aux resolves
}

/// Eager per-story metadata, derived from bytes `scan_stories` already reads.
#[derive(Debug, Clone, PartialEq)]
pub struct StoryMeta {
    /// The size of the file on disk — the container, when the story lives in one.
    pub size_bytes: u64,
    /// The size of the story image lanthorn actually runs, after mounting the
    /// container (SQ-0771). Equal to `size_bytes` for a plain `.z*`/`.ulx`/`.dat`;
    /// smaller for every container, and *unrelated* to it for an Amiga floppy —
    /// a `.adf` is 880 KB whatever it holds, so the container's length says
    /// nothing about the game. Reported for every container kind (`.adf`, blorb,
    /// zip), not just the disk image.
    pub story_bytes: u64,
    pub modified: Option<String>, // "YYYY-MM-DD"
    pub engine: Engine,
    pub format: String,           // "Z-code" | "Glulx" | "Blorb (Z-code)" | "Blorb (Glulx)"
    pub version: Option<String>,  // Z: "3"; Glulx: "3.1.2"
    pub serial: Option<String>,   // Z only
    pub release: Option<u16>,     // Z only
    pub ifid: String,
    pub features: Features,
    pub self_blorb: Option<Vec<ChunkInfo>>, // Some when the story file itself is a blorb
    /// The story was mounted out of a release floppy rather than read as a plain
    /// file, and which kind, so the TYPE column names that container: `Z6 (ADF)`
    /// for an Amiga disk, `Z6 (HFS)` for a Macintosh one (SQ-0737, SQ-0837).
    /// Decided by the mount, from the disk's own filesystem — never from the
    /// filename.
    pub disk_image: Option<crate::hints::DiskImage>,
    /// **Which** story on that image this row is: the name the volume stores it
    /// under (`LEATHRGODDESSES`, `HITCHHIK.DAT`, `HITCHHIK/STORY.DAT` on the one
    /// format with directories), as [`crate::hints::mounted_stories`] listed it
    /// (SQ-0859).
    ///
    /// `None` for every loose story file, and for a disk image holding exactly
    /// one story — there is nothing to choose there, so that row opens by path
    /// exactly as it always did. `Some` is what the launch carries back to
    /// [`crate::hints::load_mounted_story_from`], and it is the only thing that
    /// tells two rows off one compilation apart at open time.
    pub disk_entry: Option<String>,
    /// Resolved per `resolve`'s precedence: IFmd > fetched sidecar. No TSV/stem
    /// source for these (title-only), so absent means genuinely unknown.
    pub author: Option<String>,
    pub year: Option<String>, // from iFiction's `first_published`
    pub genre: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    /// The story's IFDB page URL, present only once fetched (no IFmd equivalent).
    pub ifdb_link: Option<String>,
    /// IFDB's community average rating, 1–5 (SQ-0529). Fetched-only, like the
    /// link. `None` covers both "never fetched" and "IFDB has no ratings for
    /// it", and the RATE column renders both as blank — never as `0.0`.
    pub ifdb_rating: Option<f32>,
    /// The number of ratings behind `ifdb_rating`; the rating sort's tiebreak.
    pub ifdb_rating_count: Option<u32>,
    /// A fetch ran but IFDB had no record for this IFID — so the panel offers a
    /// manual IFDB search link instead of a dead end (SQ-0371).
    pub fetch_not_found: bool,
}

impl StoryMeta {
    /// The metadata a row that is not a story carries: every field at its
    /// "unknown" value. Only [`StoryEntry::folder`] builds one.
    fn placeholder() -> StoryMeta {
        StoryMeta {
            size_bytes: 0,
            story_bytes: 0,
            modified: None,
            engine: Engine::ZCode,
            format: String::new(),
            version: None,
            serial: None,
            release: None,
            ifid: String::new(),
            features: Features::default(),
            self_blorb: None,
            disk_image: None,
            disk_entry: None,
            author: None,
            year: None,
            genre: None,
            language: None,
            description: None,
            ifdb_link: None,
            ifdb_rating: None,
            ifdb_rating_count: None,
            fetch_not_found: false,
        }
    }

    /// The build that names this story's save directory when it was mounted out
    /// of a disk image, and `None` for a loose story file — the scan already
    /// read the header, so a row can be keyed without touching the disk again
    /// (SQ-0850). Feed to [`crate::storage::story_key_for`].
    pub fn disk_build(&self) -> Option<crate::storage::DiskBuild> {
        Some(crate::storage::DiskBuild {
            // The MEDIUM and the VERSION are half of what names a Version 6
            // game's directory (SQ-1068) — one build pressed onto two disks is
            // one game for v1-v5 and two machines for v6.
            medium: self.disk_image?,
            version: self.version.as_deref().and_then(|v| v.parse().ok())?,
            release: self.release?,
            serial: self.serial.as_ref()?.clone(),
        })
    }
}

/// What a picker row stands for: a story, or a place stories live.
///
/// A library is a tree the moment someone sorts two thousand files into
/// folders, and the picker used to see one level of it. A folder is a row in
/// the same list rather than a second list, so every mechanism the list has
/// (selection, scrolling, mouse hit-testing, the sort that keeps caches
/// index-aligned) applies to it unchanged; what changes is what `Enter` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowKind {
    #[default]
    Story,
    /// A sub-folder of the library, or the `..` that leads back out of one.
    /// `path` is the directory itself; `Enter` descends into it.
    Folder,
}

/// The label a folder row wears when it leads to the parent directory.
pub const PARENT_LABEL: &str = "..";

/// One selectable story in the picker.
#[derive(Debug, Clone, PartialEq)]
pub struct StoryEntry {
    pub path: PathBuf,
    /// Display title: `known_title(ifid)` or the filename stem.
    pub title: String,
    /// The bare filename (e.g. `zork1.z5`), shown beside the title.
    pub filename: String,
    pub meta: StoryMeta,
    /// An InvisiClues/hint sidecar detected beside this game and associated with
    /// it during the scan (SQ-0443). The sidecar entry is hidden from the list;
    /// its presence lights the hint badge and names the file in the info panel.
    pub hint_sidecar: Option<std::path::PathBuf>,
    /// Story or folder. Every reader that opens, fetches, badges or launches a
    /// row asks this first; a folder has none of those.
    pub kind: RowKind,
}

impl StoryEntry {
    /// True for a folder row (a sub-directory or `..`), which is navigated
    /// rather than played.
    pub fn is_folder(&self) -> bool {
        self.kind == RowKind::Folder
    }

    /// A folder row: `path` is the directory, `label` what the list prints
    /// (`name/`, or [`PARENT_LABEL`]).
    pub fn folder(path: PathBuf, label: &str) -> StoryEntry {
        StoryEntry {
            path,
            title: label.to_string(),
            filename: label.to_string(),
            meta: StoryMeta::placeholder(),
            hint_sidecar: None,
            kind: RowKind::Folder,
        }
    }

    /// **What identifies a row.** The container's path, plus which story on it
    /// when the container is a disk image holding several (SQ-0859).
    ///
    /// The path alone stopped being an identity the moment one image could
    /// contribute six rows: `stories.iter().find(|e| e.path == p)` would find
    /// whichever of the six sorted first, and hand a fetch result, a hint
    /// sidecar or a cursor position to the wrong game.
    pub fn is(&self, path: &Path, disk_entry: Option<&str>) -> bool {
        self.path == path && self.meta.disk_entry.as_deref() == disk_entry
    }

    /// Whether this row and `other` are the same story.
    pub fn same_story(&self, other: &StoryEntry) -> bool {
        self.is(&other.path, other.meta.disk_entry.as_deref())
    }

    /// This story's per-game directory token — its build when it came off a disk
    /// image, its ENTRY's basename when it came out of a zip (SQ-1098), and its
    /// own basename when it is a loose file (SQ-0850). Free: the scan already
    /// read the header.
    ///
    /// **The one place a row's key is worked out.** Three call sites used to
    /// reassemble it from `path` and `disk_build()` alone, which is how the zip
    /// entry went missing from two of them and not the third.
    pub fn story_key(&self) -> String {
        crate::storage::story_key_for(crate::storage::StoryOrigin {
            path: &self.path,
            entry: self.meta.disk_entry.as_deref(),
            build: self.meta.disk_build().as_ref(),
        })
    }

    /// Where this story's saves, sidecars and fetched metadata live.
    pub fn game_dir(&self, data_base: &Path) -> PathBuf {
        crate::storage::game_dir(data_base, &self.story_key())
    }

    /// The key this row's cover art is cached under — and the file the decoder
    /// reads it from.
    ///
    /// A loose story is its own path, because a blorb's `Fspc` frontispiece
    /// lives inside the file and outranks anything fetched. A story off a disk
    /// image is its **game directory** instead: five rows off `INFOCOM6` share
    /// one path, so keying by path would paint the first row's jacket onto all
    /// five, and a disk image is never a blorb — there is no frontispiece in
    /// there to lose. The fetched `cover.png` sits in that same directory, so
    /// the key doubles as the source.
    pub fn cover_key(&self, data_base: &Path) -> PathBuf {
        match self.meta.disk_entry {
            Some(_) => self.game_dir(data_base),
            None => self.path.clone(),
        }
    }
}

/// Candidate **bare** story-file extensions (matched case-insensitively).
/// `.zblorb` / `.blorb` / zips are handled by `load_story_bytes`; `.dat` covers
/// some Infocom releases.
///
/// Release *disk images* are deliberately absent: their spellings belong to
/// `blorb::medium`'s format table, and [`has_story_ext`] takes the union.
const STORY_EXTS: &[&str] = &[
    "z3", "z4", "z5", "z6", "z7", "z8", "zblorb", "blorb", "zlb", "dat", "ulx", "gblorb", "blb",
    // A ZIP is opened by `hints::read_story_file` exactly as a disk image is —
    // the container is unwrapped and the story inside comes out — so the scan
    // that lists disk images had no principled reason to skip archives, and
    // SQ-1086 gave it a pressing one: a story fetched from a `.zip` URL and KEPT
    // landed in the library invisible to the only view that would show it, under
    // a dialog that had just promised "it will be there next time".
    "zip",
];

/// Is this file worth opening during a directory scan?
///
/// The bare story extensions above, unioned with every spelling
/// `blorb::medium::image_extensions` names — an Amiga `.adf`, a Macintosh
/// `.image`, a DOS `.ima`/`.img`, an Atari ST `.st` — whose story `load_story`
/// mounts out of the disk image (SQ-0719, SQ-0837, SQ-0833, SQ-0835).
///
/// **Only a pre-filter on what is worth opening**: every candidate is then
/// mounted and rejected unless a story actually comes out of it, which is what
/// makes admitting a generic extension like `.image`, `.img` or `.zip` safe.
/// Nothing here concludes a *format* from a name; that stays `DiskImage::detect`'s
/// answer over the bytes.
///
/// That rejection was CHECKED rather than assumed before `.zip` was admitted
/// (SQ-1086): `resolve_entry` is `hints::load_mounted_story_from(..).ok()?`, so
/// an archive holding no story is an `Err` and yields no row at all, and
/// `entry_from_loaded` then requires whatever did come out to construct a VM
/// before it becomes one. An archive of holiday photos in a story directory
/// costs one open and appears nowhere.
///
/// The disk half used to be a second list written out here, and it went stale
/// exactly as a duplicated census does: it knew `.adf` and `.image` and never
/// heard about the DOS and ST formats two later quests had already taught
/// lanthorn to mount, so those floppies were absent from the story list while
/// opening one by name worked fine (SQ-0849).
pub(crate) fn has_story_ext(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    STORY_EXTS.contains(&ext.as_str()) || blorb::medium::image_extensions().any(|e| e == ext)
}

/// True for blorb-container extensions (case-insensitive).
fn is_blorb_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "zblorb" | "blorb" | "gblorb" | "blb"))
        .unwrap_or(false)
}

/// Format a `SystemTime` mtime as "YYYY-MM-DD" (UTC, civil-date arithmetic; no
/// chrono dependency). Returns None if the time is before the Unix epoch.
fn format_mtime_ymd(t: std::time::SystemTime) -> Option<String> {
    let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Z-machine version byte at header offset 0x00.
fn z_version(exec: &[u8]) -> Option<u8> {
    exec.first().copied()
}

/// Z-machine release: big-endian word at header offset 0x02.
fn z_release(exec: &[u8]) -> Option<u16> {
    match (exec.get(0x02), exec.get(0x03)) {
        (Some(&h), Some(&l)) => Some(u16::from_be_bytes([h, l])),
        _ => None,
    }
}

/// Z-machine serial: 6 ASCII bytes at header offset 0x12..0x18.
///
/// **Bit 7 comes off each byte**, exactly as `cli_host::storage::DiskBuild::of`,
/// `blorb::adf::looks_like_story` and `zvm-cli`'s disk menu read the same six
/// bytes (SQ-0856): the Apple II wrote text with the high bit set, and
/// `LEATHRGODDESSES` on *Lost Treasures* `INFOCOM6` spells its serial
/// `C2 EC EF F7 EE A1` — `Blown!`. Unmasked, `from_utf8_lossy` turned that into
/// six replacement characters, so the SERIAL column showed nothing readable and
/// — the part that actually broke — the row's save key derived from it
/// (`StoryMeta::disk_build`) disagreed with the key the launch computes, sending
/// the game's saves to two different directories depending on which side of the
/// list you were on (SQ-0859). Every other serial in the corpus has bit 7 clear,
/// so nothing else moves.
fn z_serial(exec: &[u8]) -> Option<String> {
    let s = exec.get(0x12..0x18)?;
    Some(s.iter().map(|c| char::from(c & 0x7f)).collect())
}

/// Z-machine Flags2: big-endian word at header offset 0x10.
/// bit 3 (0x0008)=graphics, bit 6 (0x0040)=colours, bit 7 (0x0080)=sound.
fn z_flags2(exec: &[u8]) -> u16 {
    match (exec.get(0x10), exec.get(0x11)) {
        (Some(&h), Some(&l)) => u16::from_be_bytes([h, l]),
        _ => 0,
    }
}

/// Glulx version: 16-bit major at 0x04, minor at 0x06, subminor at 0x07 →
/// "major.minor.subminor".
fn glulx_version(exec: &[u8]) -> Option<String> {
    let major = u16::from_be_bytes([*exec.get(0x04)?, *exec.get(0x05)?]);
    let minor = *exec.get(0x06)?;
    let subminor = *exec.get(0x07)?;
    Some(format!("{major}.{minor}.{subminor}"))
}

/// Lazily-resolved, per-highlight data that touches other files/dirs.
pub struct StoryAux {
    /// Sibling/dir-scan blorb resources when the story is NOT itself a blorb.
    /// Carries the source path so the panel can name the file.
    pub assoc_blorb: Option<(PathBuf, Vec<ChunkInfo>)>,
    pub saves: Vec<crate::persist_files::SaveInfo>,
    pub hints_available: bool,
    /// The story's per-game dir (SQ-0284), for the info panel's Saves header.
    pub game_dir: PathBuf,
    /// `.qzl` in-game saves in `game_dir` (SQ-0285).
    pub qzl_saves: Vec<crate::persist_files::SaveInfo>,
    /// Game-managed automatic `.qzl` saves in `game_dir` (SQ-0296): the
    /// `_`-prefixed fixed-name files the player saves list hides.
    pub auto_saves: Vec<crate::persist_files::SaveInfo>,
    /// Sidecar filenames present in `game_dir` (`default.aux`/`default.glkvfs`).
    pub sidecars: Vec<&'static str>,
    /// Native picture archives detected for this story (SQ-0789), from the very
    /// same [`crate::launch_options::discover_art_candidates`] the launch-options
    /// dialog lists. **One source, so the two surfaces cannot disagree** — a
    /// panel that says "four renditions" over a dialog that offers three is worse
    /// than either alone, and the shared function is what makes that impossible
    /// rather than merely unlikely.
    ///
    /// Still display-only. The panel is read-only, so this is the *safe* half of
    /// the discovery/pairing split: it ends at a human's eyes, and nothing
    /// downstream consumes it.
    pub art_candidates: Vec<crate::launch_options::ArtCandidate>,
    /// The archive the game's own `config.toml` names, if any — so the panel can
    /// say which of the detected renditions is actually in force.
    pub art_in_use: Option<String>,
    /// Sound effects the story's own MEDIUM carries (SQ-0907), sorted by effect
    /// number — the same [`crate::native_sound::from_medium`] the launch path plays
    /// from, so the panel cannot claim a sound the game will not get.
    ///
    /// Display-only here, exactly like `art_candidates`: it ends at a human's eyes.
    pub disk_sounds: Vec<crate::native_sound::DiskSound>,
    /// Typefaces the story's own MEDIUM carries (SQ-1018), through the same
    /// pairing the renderer uses — so the panel cannot claim a face the game will
    /// not get, and shows one that is present but unused as exactly that.
    ///
    /// Display-only here, exactly like `art_candidates` and `disk_sounds`.
    pub disk_fonts: Vec<crate::native_font::DiskFace>,
    /// Typefaces the user's OWN disk images under `~/.lanthorn/` carry (SQ-1038)
    /// — a Workbench or System disk kept beside the stories rather than any one
    /// game's release. Same value for every story (it does not depend on
    /// `entry`), resolved here anyway so the panel reads it the same way it
    /// reads `disk_fonts`. Display-only, and unlike `disk_fonts` there is no
    /// "in use": see [`crate::system_fonts`].
    pub system_fonts: Vec<crate::system_fonts::SystemFace>,
}

/// Resolve the lazy aux for one story. `data_base` is the storage base
/// (`user_dir/saves` or `--data-dir`); the story's saves live in its per-game
/// dir `<data_base>/<story-key>/` (SQ-0284). `hint_index` is the shared index
/// loaded once at picker start (still keyed by IFID).
pub fn resolve_aux(
    entry: &StoryEntry,
    data_base: &Path,
    hint_index: &hints::HintIndex,
) -> StoryAux {
    // Only record an ASSOCIATED blorb (a different file); the self-blorb case is
    // already carried in StoryMeta.self_blorb.
    //
    // Through `graphics::resource_blorb` (SQ-1085), because the panel's job is
    // to name the archive the LAUNCH will draw from, and the bare `blorb` call
    // is no longer that: it cannot see inside a zip. It also stops listing a
    // Blorb that names a different build, which the launch has refused since
    // SQ-0866 — showing the player a sidecar nothing will read was telling them
    // something untrue.
    let assoc_blorb = match crate::graphics::resource_blorb(&entry.path).found {
        Some((b, src)) if src != entry.path => Some((src, chunks_of(&b))),
        _ => None,
    };
    let game_dir = entry.game_dir(data_base);
    let saves = crate::persist_files::list_saves(&game_dir);
    let hints_available = hint_index.get(&entry.meta.ifid).is_some();
    let qzl_saves = crate::persist_files::list_qzl(&game_dir);
    let auto_saves = crate::persist_files::list_qzl_auto(&game_dir);
    let mut sidecars = Vec::new();
    if game_dir.join("default.aux").exists() { sidecars.push("default.aux"); }
    if game_dir.join("default.glkvfs").exists() { sidecars.push("default.glkvfs"); }
    // Resolved here rather than in the panel because the panel redraws every
    // frame and this reads and parses whole archives; the aux cache is already
    // the per-story "things that touch the disk" tier.
    // The story's own entry, so the info panel lists one game's archives rather
    // than the whole platter's on a compilation (SQ-0876).
    let art_candidates = crate::launch_options::discover_art_candidates(
        &entry.path,
        entry.meta.disk_entry.as_deref(),
    );
    let art_in_use = crate::styles::read_per_game_pictures(&game_dir);
    // Same tier as the artwork scan above, and for the same reason: it mounts the
    // medium, which the panel must not do on every frame.
    let mut disk_sounds: Vec<crate::native_sound::DiskSound> =
        crate::native_sound::from_medium(&entry.path).into_values().collect();
    disk_sounds.sort_by_key(|s| s.effect);
    // Same tier again, and paired with this story's own entry for the same reason
    // the artwork scan is (SQ-0876/SQ-1018): a compilation carries one
    // application per game, and only one of them is this row's.
    // The MEDIUM names the machine, the same door the launch takes — not the path,
    // and not a guess (SQ-0876).
    let (profile, profile_source) =
        crate::interpreter::InterpreterProfile::resolve_with_source(&entry.path, None, None, None);
    // `disks: None` — this column is about the STORY's own medium, and asking the
    // system rung as well would cost a second mount of every boot disk per row for
    // an answer it cannot change: a release face the cascade admits is drawn either
    // as the body or as the machine's fixed-pitch alternate, and `FaceSet::draws`
    // counts both (SQ-1036). `art_scale` likewise only ever reaches the system rung.
    let disk_fonts = crate::native_font::detected(&crate::native_font::FaceRequest {
        story_path: &entry.path,
        entry: entry.meta.disk_entry.as_deref(),
        profile,
        source: profile_source,
        art_scale: None,
        disks: None,
    });
    // Same tier again. Not paired with `entry` at all — the user's own disks
    // under `~/.lanthorn/` are not any one game's release — but resolved here
    // rather than on every frame, for the same reason the three scans above are.
    //
    // **Only for a Version 6 story, and only for its OWN machine.** Below v6 no
    // text is drawn from a disk face at all — it goes through the terminal — so a
    // system disk is irrelevant whatever machine it names. And a Macintosh System
    // disk has nothing to say about an Amiga release: showing Geneva under a
    // Journey floppy would be a row that can never apply, which is exactly the
    // "present but never used" confusion this panel exists to prevent (SQ-1018).
    let system_fonts = if entry.meta.engine == Engine::ZCode
        && entry.meta.version.as_deref() == Some("6")
    {
        crate::system_fonts::detected_for(profile)
    } else {
        Vec::new()
    };
    StoryAux {
        assoc_blorb,
        saves,
        hints_available,
        game_dir,
        qzl_saves,
        auto_saves,
        sidecars,
        art_candidates,
        art_in_use,
        disk_sounds,
        disk_fonts,
        system_fonts,
    }
}

/// Convert a parsed blorb's resource index into displayable `ChunkInfo`.
pub fn chunks_of(b: &blorb::Blorb) -> Vec<ChunkInfo> {
    b.resources()
        .iter()
        .map(|r| ChunkInfo {
            usage: String::from_utf8_lossy(&r.usage).into_owned(),
            number: r.number,
            chunk_type: String::from_utf8_lossy(&r.chunk_type).into_owned(),
            len: r.len,
            detail: resource_detail(&r.usage, &r.chunk_type, b.resource_data(r)),
        })
        .collect()
}

// ── Resource format-detail parsing ──────────────────────────────────────
//
// Best-effort header-only parsing of Blorb `Snd `/`Pict` resources, for the
// info panel's Resources listing. Never decodes actual audio/pixel data, and
// is panic-proof on malformed/truncated input: every read is bounds-checked
// and a parse failure simply yields `None`.

/// Dispatch a resource's format detail by usage, or `None` when the usage
/// isn't a sound/image or the payload doesn't parse.
fn resource_detail(usage: &[u8; 4], chunk_type: &[u8; 4], data: &[u8]) -> Option<String> {
    match usage {
        b"Snd " => sound_detail(chunk_type, data),
        b"Pict" => image_detail(chunk_type, data),
        _ => None,
    }
}

/// Big-endian u16 at `off`, bounds-checked.
fn be_u16(data: &[u8], off: usize) -> Option<u16> {
    let s = data.get(off..off + 2)?;
    Some(u16::from_be_bytes([s[0], s[1]]))
}

/// Big-endian u32 at `off`, bounds-checked.
fn be_u32(data: &[u8], off: usize) -> Option<u32> {
    let s = data.get(off..off + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Decode an IEEE 80-bit extended-precision float (as used for AIFF sample
/// rates) to its nearest `u32`. Returns 0 on malformed/negative-exponent
/// input rather than panicking.
fn extended80_to_u32(e: &[u8]) -> u32 {
    if e.len() < 10 {
        return 0;
    }
    let exp = ((((e[0] as u16) << 8) | e[1] as u16) & 0x7fff) as i32 - 16383;
    let mant = u64::from_be_bytes([e[2], e[3], e[4], e[5], e[6], e[7], e[8], e[9]]);
    if exp < 0 {
        return 0;
    }
    let shift = 63 - exp;
    if !(0..=63).contains(&shift) {
        return 0;
    }
    (mant >> shift) as u32
}

/// Sound resource format detail, dispatched by chunk type (matches
/// [`blorb::SoundKind`] detection: `FORM` → AIFF/AIFC, `OGGV` → Ogg, `MOD ` →
/// module).
fn sound_detail(chunk_type: &[u8; 4], data: &[u8]) -> Option<String> {
    match chunk_type {
        b"FORM" => aiff_detail(data),
        b"OGGV" => ogg_detail(data),
        b"MOD " => mod_detail(data),
        _ => None,
    }
}

/// AIFF/AIFC sample-rate + bit depth + channels + duration, parsed from the
/// `COMM` subchunk. Blorb strips the outer `FORM` header, so `data` starts
/// with the form type (`AIFF`/`AIFC`) followed by subchunks.
fn aiff_detail(data: &[u8]) -> Option<String> {
    let sig = data.get(0..4)?;
    if sig != b"AIFF" && sig != b"AIFC" {
        return None;
    }
    let mut pos = 4;
    while pos + 8 <= data.len() {
        let id = data.get(pos..pos + 4)?;
        let clen = be_u32(data, pos + 4)? as usize;
        let cs = pos + 8;
        if cs.checked_add(clen)? > data.len() {
            return None;
        }
        if id == b"COMM" {
            if clen < 18 {
                return None;
            }
            let channels = be_u16(data, cs)?;
            let num_frames = be_u32(data, cs + 2)?;
            let sample_size = be_u16(data, cs + 6)?;
            let rate_bytes = data.get(cs + 8..cs + 18)?;
            let rate = extended80_to_u32(rate_bytes);
            let mut parts = vec![format!("{:.1} kHz", rate as f64 / 1000.0)];
            parts.push(format!("{sample_size}-bit"));
            parts.push(match channels {
                1 => "mono".to_string(),
                2 => "stereo".to_string(),
                n => format!("{n}ch"),
            });
            if rate != 0 {
                parts.push(format!("{:.1}s", num_frames as f64 / rate as f64));
            }
            return Some(parts.join(" · "));
        }
        pos = cs + clen + (clen & 1);
    }
    None
}

/// Ogg Vorbis sample rate + channels, found by scanning for the Vorbis
/// identification-header packet (`\x01vorbis`) within the first ~512 bytes.
fn ogg_detail(data: &[u8]) -> Option<String> {
    if data.get(0..4)? != b"OggS" {
        return None;
    }
    let window = data.get(0..data.len().min(512))?;
    let needle = b"\x01vorbis";
    let p = window.windows(needle.len()).position(|w| w == needle)?;
    let channels = *data.get(p + 11)?;
    let rate_bytes = data.get(p + 12..p + 16)?;
    let rate = u32::from_le_bytes([rate_bytes[0], rate_bytes[1], rate_bytes[2], rate_bytes[3]]);
    let ch_word = match channels {
        1 => "mono".to_string(),
        2 => "stereo".to_string(),
        n => format!("{n}ch"),
    };
    Some(format!("{:.1} kHz · {ch_word}", rate as f64 / 1000.0))
}

/// Amiga ProTracker module channel count, read from the format tag at
/// offset 1080..1084 (present only when the module has 31 instruments).
fn mod_detail(data: &[u8]) -> Option<String> {
    if data.len() < 1084 {
        return None;
    }
    let tag = data.get(1080..1084)?;
    let n: u32 = match tag {
        b"M.K." | b"M!K!" | b"FLT4" | b"4CHN" => 4,
        b"6CHN" => 6,
        b"8CHN" | b"FLT8" => 8,
        _ => {
            let s = std::str::from_utf8(tag).unwrap_or("");
            let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().unwrap_or(4)
        }
    };
    Some(format!("{n}ch"))
}

/// Image resource format detail, dispatched by chunk type.
fn image_detail(chunk_type: &[u8; 4], data: &[u8]) -> Option<String> {
    match chunk_type {
        b"PNG " => png_detail(data),
        b"JPEG" => jpeg_detail(data),
        _ => None,
    }
}

/// PNG width × height + bits-per-pixel, parsed from the IHDR chunk (fixed
/// offsets right after the 8-byte PNG signature).
fn png_detail(data: &[u8]) -> Option<String> {
    const SIG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if data.len() < 26 {
        return None;
    }
    if data.get(0..8)? != SIG {
        return None;
    }
    let width = be_u32(data, 16)?;
    let height = be_u32(data, 20)?;
    let bit_depth = *data.get(24)?;
    let color_type = *data.get(25)?;
    let channels: u32 = match color_type {
        0 => 1, // grayscale
        2 => 3, // RGB
        3 => 1, // palette
        4 => 2, // grayscale + alpha
        6 => 4, // RGBA
        _ => 1,
    };
    let bpp = bit_depth as u32 * channels;
    Some(format!("{width}×{height} · {bpp}bpp"))
}

/// JPEG width × height + precision + component count, parsed by scanning
/// markers for the first SOF (start-of-frame) segment.
fn jpeg_detail(data: &[u8]) -> Option<String> {
    if data.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2;
    while pos < data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }
        // Skip fill bytes (runs of 0xFF before the real marker byte).
        let mut m_pos = pos;
        while data.get(m_pos) == Some(&0xFF) {
            m_pos += 1;
        }
        let marker = *data.get(m_pos)?;
        let seg_start = m_pos + 1;
        // Markers with no length field: SOI/EOI/RSTn.
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            pos = seg_start;
            continue;
        }
        let len = be_u16(data, seg_start)? as usize;
        if len < 2 {
            return None;
        }
        let body_off = seg_start + 2;
        let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            let precision = *data.get(body_off)?;
            let height = be_u16(data, body_off + 1)?;
            let width = be_u16(data, body_off + 3)?;
            let components = *data.get(body_off + 5)?;
            return Some(format!("{width}×{height} · {precision}-bit · {components}ch"));
        }
        pos = body_off.checked_add(len - 2)?;
    }
    None
}

/// Eager `Features` for a Z-code exec image, folding in self-blorb resources.
fn z_features(exec: &[u8], self_blorb: Option<&[ChunkInfo]>) -> Features {
    let f2 = z_flags2(exec);
    let mut sound = f2 & 0x0080 != 0;
    let mut graphics = f2 & 0x0008 != 0;
    if let Some(chunks) = self_blorb {
        if chunks.iter().any(|c| c.usage == "Snd ") {
            sound = true;
        }
        if chunks.iter().any(|c| c.usage == "Pict") {
            graphics = true;
        }
    }
    Features { sound, graphics, colour: Some(f2 & 0x0040 != 0), hints: false }
}

/// Eager `Features` for a Glulx story — colour is runtime Glk (None); sound and
/// graphics come from a self-blorb only.
fn glulx_features(self_blorb: Option<&[ChunkInfo]>) -> Features {
    let mut f = Features { sound: false, graphics: false, colour: None, hints: false };
    if let Some(chunks) = self_blorb {
        f.sound = chunks.iter().any(|c| c.usage == "Snd ");
        f.graphics = chunks.iter().any(|c| c.usage == "Pict");
    }
    f
}

/// Per-field metadata resolution, produced once by [`resolve`] and read
/// verbatim by everything downstream (list, sort, info panel).
#[derive(Debug, Clone, PartialEq)]
struct Resolved {
    title: String,
    author: Option<String>,
    year: Option<String>,
    genre: Option<String>,
    language: Option<String>,
    description: Option<String>,
    ifdb_link: Option<String>,
    ifdb_rating: Option<f32>,
    ifdb_rating_count: Option<u32>,
    fetch_not_found: bool,
}

/// The publication year from a Treaty of Babel `<firstpublished>`, which is
/// `YYYY` or `YYYY-MM-DD` (iFiction allows the full ISO date). Keep just the
/// leading four-digit year, so the value both sorts numerically and fits the
/// narrow YEAR column; anything without a 4-digit lead is dropped as unusable.
fn leading_year(s: &str) -> Option<String> {
    let y: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    (y.len() == 4).then_some(y)
}

/// A bundled Scott-format entry: canonical title plus, where known, its IFDB id
/// and — for the homebrew games with no IFDB record — author and one-line
/// description, so the browser can show those without a fetch.
struct ScottEntry {
    title: &'static str,
    tuid: Option<&'static str>,
    author: Option<&'static str>,
    description: Option<&'static str>,
}

/// Canonical metadata for Scott-format ("ScottFree") `.dat`/`.blb` adventures,
/// keyed by the lowercase filename stem, bundled in `scott_titles.tsv`
/// (`include_str!`d at build time). Keyed by filename rather than the `.dat`
/// trailer's adventure number because that number is not unique across the
/// ScottFree corpus (Brian Howarth's Mysterious Adventures reuse 1-11;
/// Questprobe titles have none).
fn scott_titles() -> &'static std::collections::HashMap<&'static str, ScottEntry> {
    use std::sync::OnceLock;
    static TABLE: OnceLock<std::collections::HashMap<&'static str, ScottEntry>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("scott_titles.tsv")
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                // <filename-stem>\t<title>[\t<ifdb-tuid>[\t<author>[\t<description>]]]
                let mut cols = line.splitn(5, '\t');
                let stem = cols.next()?.trim();
                let title = cols.next()?.trim();
                let tuid = cols.next().map(str::trim).filter(|c| !c.is_empty());
                let author = cols.next().map(str::trim).filter(|c| !c.is_empty());
                let description = cols.next().map(str::trim).filter(|c| !c.is_empty());
                (!stem.is_empty() && !title.is_empty())
                    .then_some((stem, ScottEntry { title, tuid, author, description }))
            })
            .collect()
    })
}

/// Look up a bundled Scott entry by filename stem (matched case-insensitively).
fn scott_entry(stem: &str) -> Option<&'static ScottEntry> {
    scott_titles().get(stem.to_ascii_lowercase().as_str())
}

/// The canonical title for a known Scott-format game, keyed by filename stem
/// (matched case-insensitively).
pub fn scott_title(stem: &str) -> Option<&'static str> {
    scott_entry(stem).map(|e| e.title)
}

/// The IFDB game id (TUID) for a known Scott-format game, keyed by filename stem
/// (matched case-insensitively), if we have one. A Scott `.dat`'s computed IFID
/// never resolves on IFDB, so the metadata fetch looks the game up by this id.
pub fn scott_tuid(stem: &str) -> Option<&'static str> {
    scott_entry(stem).and_then(|e| e.tuid)
}

/// The bundled author for a Scott-format game (filename stem, case-insensitive),
/// present only for the homebrew games that have no IFDB record to fetch it from.
pub fn scott_author(stem: &str) -> Option<&'static str> {
    scott_entry(stem).and_then(|e| e.author)
}

/// The bundled one-line description for a Scott-format game (filename stem,
/// case-insensitive), present only for the homebrew games with no IFDB record.
pub fn scott_description(stem: &str) -> Option<&'static str> {
    scott_entry(stem).and_then(|e| e.description)
}

/// Resolve a display title for a Scott-format `.dat` story from its filename
/// stem. `None` when the filename isn't a known Scott game (caller falls back to
/// the filename stem).
pub fn scott_story_title(path: &Path) -> Option<String> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    scott_title(stem).map(str::to_string)
}

/// The bundled-table title for a story: the Scott filename table
/// (`scott_titles.tsv`, keyed by the stem) when the story is a Scott database,
/// else the IFID-keyed known-title table. Neither table needs the file — this is
/// the offline tier, below any real metadata.
pub fn bundled_title(stem: &str, ifid: &str, is_scott: bool) -> Option<&'static str> {
    is_scott
        .then(|| scott_title(stem))
        .flatten()
        .or_else(|| crate::session::known_title(ifid))
}

/// A blorb container's own `IFmd` (Treaty of Babel iFiction) chunk, parsed.
/// `None` for a non-blorb, an unreadable file, or a blorb carrying no such
/// chunk — which is the common case (`photo201.blb` has only an `RIdx`).
///
/// [`resolve_entry`] reads the same chunk out of the parse it already does for
/// the resource index; this is the standalone reader for callers that want only
/// the title.
fn container_ifmd(path: &Path) -> Option<crate::ifiction::IFiction> {
    if !is_blorb_ext(path) {
        return None;
    }
    let raw = std::fs::read(path).ok()?;
    if !blorb::Blorb::is_blorb(&raw) {
        return None;
    }
    let b = blorb::Blorb::parse(raw).ok()?;
    crate::ifiction::parse(b.metadata()?).ok()
}

/// The title the story browser resolves for `path` from **real metadata**, with
/// no filename fallback: the container's own `IFmd` chunk, then the fetched IFDB
/// sidecar under `data_base`, then the bundled tables. `None` when no source
/// knows this story.
///
/// This is the browser's own answer, exported so the in-game story pane can ask
/// for it instead of guessing from the boot banner (SQ-0766). The precedence is
/// literally [`resolved_title`], shared with [`resolve`], so the list and the
/// pane cannot name the same game differently.
pub fn metadata_title(path: &Path, data_base: &Path, ifid: &str, is_scott: bool) -> Option<String> {
    let game_dir = crate::storage::game_dir(data_base, &crate::storage::story_key_at(path));
    metadata_title_in(path, &game_dir, ifid, is_scott)
}

/// [`metadata_title`] for a caller that already knows which per-game directory
/// this story's sidecar lives in.
///
/// The path cannot always work it out: one disk image holds several games and
/// [`crate::storage::story_key_at`] answers with the format's tiebreak, so a
/// story chosen off a compilation would be handed its disk-mate's fetched
/// metadata (SQ-0859). Whoever mounted the story has the right directory in
/// hand; this takes it.
pub fn metadata_title_in(
    path: &Path,
    game_dir: &Path,
    ifid: &str,
    is_scott: bool,
) -> Option<String> {
    let ifmd = container_ifmd(path);
    let fetched = crate::story_info::load(game_dir, ifid).and_then(|i| i.fetched);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    resolved_title(ifmd.as_ref(), fetched.as_ref(), bundled_title(stem, ifid, is_scott))
}

/// The title tiers of SPEC "Precedence", stopping short of the filename stem:
/// the file's own `IFmd` chunk, then a fetched IFDB sidecar, then the bundled
/// table. `None` means no source knows this story, and the caller decides what
/// the last resort is — the stem for the browser list ([`resolve`]), the boot
/// banner and then the stem for the story pane ([`metadata_title`], SQ-0766).
fn resolved_title(
    ifmd: Option<&crate::ifiction::IFiction>,
    fetched: Option<&crate::story_info::FetchedMeta>,
    tsv_title: Option<&str>,
) -> Option<String> {
    ifmd.and_then(|i| i.title.clone())
        .or_else(|| fetched.and_then(|f| f.title.clone()))
        .or_else(|| tsv_title.map(str::to_string))
}

/// SPEC "Precedence": per field, independently, first non-empty wins —
/// `ifmd` (the file's own `IFmd` chunk) > `fetched` (an IFDB sidecar) > the
/// bundled `scott_titles.tsv` (`tsv_title`/`tsv_author`/`tsv_description`, only
/// populated for Scott-format games) > `stem` (the filename, title only). The
/// TSV author/description feed the homebrew Scott games that have no IFDB record
/// to fetch from; a real fetch still outranks them.
///
/// Pure so the whole table is testable without touching a filesystem.
fn resolve(
    ifmd: Option<&crate::ifiction::IFiction>,
    fetched: Option<&crate::story_info::FetchedMeta>,
    tsv_title: Option<&str>,
    tsv_author: Option<&str>,
    tsv_description: Option<&str>,
    stem: &str,
) -> Resolved {
    let title = resolved_title(ifmd, fetched, tsv_title).unwrap_or_else(|| stem.to_string());
    let author = ifmd
        .and_then(|i| i.author.clone())
        .or_else(|| fetched.and_then(|f| f.author.clone()))
        .or_else(|| tsv_author.map(str::to_string));
    let year = ifmd
        .and_then(|i| i.first_published.clone())
        .or_else(|| fetched.and_then(|f| f.first_published.clone()))
        .and_then(|s| leading_year(&s));
    let genre = ifmd
        .and_then(|i| i.genre.clone())
        .or_else(|| fetched.and_then(|f| f.genre.clone()));
    let language = ifmd
        .and_then(|i| i.language.clone())
        .or_else(|| fetched.and_then(|f| f.language.clone()));
    let description = ifmd
        .and_then(|i| i.description.clone())
        .or_else(|| fetched.and_then(|f| f.description.clone()))
        .or_else(|| tsv_description.map(str::to_string));
    // IFDB-only: the page link and the community rating exist solely in a
    // fetched block — an IFmd chunk has no equivalent for either.
    let ifdb_link = fetched.and_then(|f| f.ifdb_link.clone());
    let ifdb_rating = fetched.and_then(|f| f.ifdb_rating);
    let ifdb_rating_count = fetched.and_then(|f| f.ifdb_rating_count);
    let fetch_not_found = fetched.map(|f| f.not_found).unwrap_or(false);
    Resolved {
        title, author, year, genre, language, description, ifdb_link, ifdb_rating,
        ifdb_rating_count, fetch_not_found,
    }
}

/// Scan `dir` (top level, non-recursive) for **launchable** Z-machine stories,
/// resolving a display title for each. Files that don't load or don't parse as
/// a supported story are silently skipped (v6 is supported since SQ-0186).
/// Sorted by title (case-insensitive), then filename.
///
/// `data_base` is the storage base (as passed to `ensure_aux`/`compute_row_badges`),
/// used to locate each story's per-game `info.json` sidecar (SQ-0348's fetched
/// metadata) for precedence resolution.
pub fn scan_stories(dir: &Path, data_base: &Path) -> Vec<StoryEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    // Read-dir order is the filesystem's business and differs between machines;
    // the scan's own order must not. Sorting here is what makes "the first disk
    // that offers a build is the one that keeps it" a rule rather than a
    // coincidence (see `dedupe_within_sets`).
    files.sort();

    // Which of these files are volumes of one multi-disk release (SQ-0844).
    // Decided before anything is opened, because the rule is entirely about
    // names.
    let sets = crate::disk_set::group(&files);

    let mut out: Vec<StoryEntry> = Vec::new();
    for path in &files {
        if !has_story_ext(path) {
            continue;
        }
        out.extend(resolve_entries(path, data_base));
    }
    dedupe_within_a_volume(&mut out);
    dedupe_within_sets(&mut out, &sets);
    associate_hint_sidecars(&mut out);
    sort_stories(&mut out, Sort { key: SortKey::Title, desc: false });
    out
}

/// The sub-folders of `dir` as rows: dot-directories skipped, sorted by name
/// case-insensitively. Symlinks are followed (a library on a NAS is often one),
/// which is why [`library_dirs`] keeps a visited set.
pub fn scan_folders(dir: &Path) -> Vec<StoryEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_dir() {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            if name.starts_with('.') {
                return None;
            }
            Some((name, path))
        })
        .collect();
    dirs.sort_by(|(a, _), (b, _)| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
    dirs.into_iter().map(|(name, path)| StoryEntry::folder(path, &format!("{name}/"))).collect()
}

/// Everything the picker lists for `dir` inside a library rooted at `root`:
/// `..` when `dir` is below the root, then its sub-folders, then its stories,
/// in the default sort (which keeps the folders on top; see [`sort_stories`]).
///
/// One directory at a time, on purpose. The scan opens every candidate file it
/// lists, and a whole library is gigabytes; walking it is the indexer's job
/// ([`spawn_library_index`]), off the thread that draws.
pub fn library_rows(dir: &Path, root: &Path, data_base: &Path) -> Vec<StoryEntry> {
    let mut out: Vec<StoryEntry> = Vec::new();
    if dir != root {
        if let Some(parent) = dir.parent() {
            out.push(StoryEntry::folder(parent.to_path_buf(), PARENT_LABEL));
        }
    }
    out.extend(scan_folders(dir));
    out.extend(scan_stories(dir, data_base));
    sort_stories(&mut out, Sort::default());
    out
}

/// Every directory under `root`, root first, breadth-first, dot-directories
/// skipped and each real directory visited once however many symlinks lead to
/// it.
pub fn library_dirs(root: &Path) -> Vec<PathBuf> {
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut queue: std::collections::VecDeque<PathBuf> = std::collections::VecDeque::new();
    queue.push_back(root.to_path_buf());
    while let Some(dir) = queue.pop_front() {
        let real = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !seen.insert(real) {
            continue;
        }
        for sub in scan_folders(&dir) {
            queue.push_back(sub.path);
        }
        out.push(dir);
    }
    out
}

/// One folder's worth of the library index, as the indexing thread delivers it.
pub struct IndexBatch {
    pub dir: PathBuf,
    pub entries: Vec<StoryEntry>,
}

/// Scan every folder under `root` and hand each one's stories to `deliver` as
/// it finishes. Per-folder rather than one flat scan, so the rules that only
/// make sense within a directory (multi-disk grouping, hint-sidecar
/// association) keep applying within one.
pub fn index_library(root: &Path, data_base: &Path, mut deliver: impl FnMut(IndexBatch)) {
    for dir in library_dirs(root) {
        let entries = scan_stories(&dir, data_base);
        deliver(IndexBatch { dir, entries });
    }
}

/// [`index_library`] on its own thread. The receiver yields one batch per
/// folder and disconnects when the walk is done, so a reader can tell "still
/// indexing" from "indexed" without a flag.
pub fn spawn_library_index(root: PathBuf, data_base: PathBuf) -> std::sync::mpsc::Receiver<IndexBatch> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        index_library(&root, &data_base, |batch| {
            // A dropped receiver means the picker has gone; nothing to do but
            // stop walking, which the next iteration's send failure also does.
            let _ = tx.send(batch);
        });
    });
    rx
}

/// The fetch targets a headless `--fetch` works through: one per story in
/// `source`, and for a library that means the stories in all of its folders,
/// in the order [`index_library`] visits them.
pub fn fetch_targets(source: &StorySource, data_base: &Path) -> Vec<crate::fetch_worker::FetchTarget> {
    let mut out: Vec<crate::fetch_worker::FetchTarget> = Vec::new();
    match source {
        StorySource::Library(root) => index_library(root, data_base, |batch| {
            out.extend(batch.entries.iter().map(crate::fetch_worker::FetchTarget::row));
        }),
        other @ StorySource::DiskSet { .. } => {
            out.extend(other.scan(data_base).iter().map(crate::fetch_worker::FetchTarget::row));
        }
    }
    out
}

/// Where `entry` lives relative to `dir`: `None` when it sits directly in
/// `dir` (or outside it altogether), `Some("sub/deeper")` otherwise, always
/// with forward slashes since it is a label, not a path.
pub fn folder_label(entry: &StoryEntry, dir: &Path) -> Option<String> {
    let rel = entry.path.parent()?.strip_prefix(dir).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    Some(parts.join("/"))
}

/// The stories in `index` matching `query`: every whitespace-separated term
/// must occur, case-insensitively, in the title, the author, the filename or
/// the folder (relative to `root`). An empty query matches everything. Folder
/// rows never match; the result is stories only, in the default sort.
pub fn search_library(index: &[StoryEntry], root: &Path, query: &str) -> Vec<StoryEntry> {
    search_library_under(index, root, root, query)
}

/// [`search_library`] restricted to the stories under `scope` (a folder at or
/// below `root`): what the cover gallery shows for a folder, since a grid of
/// covers is worth more the more of the library it covers, and a folder that
/// holds only folders would otherwise show an empty one.
pub fn search_library_under(index: &[StoryEntry], root: &Path, scope: &Path, query: &str) -> Vec<StoryEntry> {
    let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
    let mut out: Vec<StoryEntry> = index
        .iter()
        .filter(|e| !e.is_folder())
        .filter(|e| e.path.starts_with(scope))
        .filter(|e| {
            if terms.is_empty() {
                return true;
            }
            let hay = format!(
                "{}\n{}\n{}\n{}",
                e.title,
                e.meta.author.as_deref().unwrap_or(""),
                e.filename,
                folder_label(e, root).unwrap_or_default()
            )
            .to_lowercase();
            terms.iter().all(|t| hay.contains(t.as_str()))
        })
        .cloned()
        .collect();
    sort_stories(&mut out, Sort::default());
    out
}

/// **One disc, not two layouts** (SQ-0878): drop a row whose build is already
/// offered by an earlier story on the *same volume*, for the *same machine*.
///
/// A compilation can carry one build twice, and the Masterpieces CD does it
/// seventeen times over. The reason is archaeological rather than accidental:
/// *The Lost Treasures of Infocom* (1991) laid its DOS side out flat — one
/// interpreter per game at `PC/`, one shared `PC/DATA/` holding every story —
/// and the 1996 Masterpieces reissue reorganised into self-contained per-game
/// folders (`PC/ZORK1/` with its own `.BAT`, `.COM`, `DATA/` and `SAVE/`) while
/// leaving the old shared directory in place. The two `PC/DATA/` sets are
/// identical across the two discs but for `HITCHHIK.DAT`, and Masterpieces'
/// `PC/` root still holds the old `BZORK.EXE` and `NNANSI.COM`. So the
/// duplicates are a 1991 layout fossilised inside a 1996 one.
///
/// **Keyed on the machine as well as the build, and that is the whole
/// subtlety.** Byte-identity alone would be wrong in both directions here:
///
/// - Mac and DOS ship the SAME build of many games — Zork I is r88/840726 on
///   both sides of the disc — so the IFID alone folds two machines' pressings
///   into one row, which is the opposite of telling them apart.
/// - `MAC/CUTTHROATS` and `PC/CUTTHROA/CUTTHROA.DAT` are byte-identical, all
///   112,640 of them, so even comparing the bytes folds a cross-machine pair.
///
/// The machine is `blorb::medium::MountedDisk::image_for`'s answer, already on
/// the row (SQ-0876). A medium that answers one machine for everything on it —
/// which is every medium but a hybrid disc — folds purely on the build, as it
/// should.
///
/// **The deepest path survives**, then disk order. The copies are byte-identical
/// so nothing about the game changes either way; what changes is the row a
/// person reads, and `PC/ZORK1/DATA/ZORK1.DAT` names its game where
/// `PC/DATA/ZORK1.DAT` names a bucket. Disk order alone would not do it —
/// `PC/DATA/` precedes `PC/ZORK1/` for the games whose folder sorts after it,
/// so which copy came first is an accident of the alphabet.
///
/// Beyond Zork keeps its row regardless: it is the one game the reorganisation
/// missed, and `PC/DATA/BEYONDZO.DAT` is the only copy of it on the disc.
fn dedupe_within_a_volume(out: &mut Vec<StoryEntry>) {
    type Key = (PathBuf, crate::hints::DiskImage, String);
    let key = |e: &StoryEntry| -> Option<Key> {
        // Only a row that came off a disk image can be a duplicate of another
        // row off the same one; a loose story file has no volume to share.
        //
        // **Both halves are asked** (SQ-1098). `disk_entry.is_some()` used to
        // IMPLY `disk_image.is_some()`, so asking one was asking both — until a
        // zip's entries became rows, which carry an entry name and no medium.
        // The fold's whole justification is archaeological and belongs to
        // pressed discs: two copies on a compilation are byte-identical, where
        // a `game.z5` and a `game.zblorb` packed in one download are not, and
        // already have two save directories under the basename rule. Folding
        // them would drop the one that ships the artwork.
        e.meta.disk_entry.as_ref()?;
        Some((e.path.clone(), e.meta.disk_image?, e.meta.ifid.clone()))
    };
    let depth = |e: &StoryEntry| {
        e.meta.disk_entry.as_deref().map_or(0, |n| n.matches('/').count())
    };
    let mut best: std::collections::HashMap<Key, usize> = std::collections::HashMap::new();
    for (i, e) in out.iter().enumerate() {
        let Some(k) = key(e) else { continue };
        match best.get(&k) {
            Some(&j) if depth(&out[j]) >= depth(e) => {}
            _ => {
                best.insert(k, i);
            }
        }
    }
    let mut i = 0;
    out.retain(|e| {
        let keep = key(e).is_none_or(|k| best.get(&k) == Some(&i));
        i += 1;
        keep
    });
}

/// **One collection, not several disks** (SQ-0844): drop a row whose build is
/// already offered by an earlier volume of the *same* set.
///
/// The compilations really do repeat themselves. `Infocom Compilation 5` stores
/// its games as flat files and `Infocom Compilation 8` in per-game directories,
/// and both carry Trinity r11/860509 checksum `FAAE` — one IFID,
/// `ZCODE-11-860509-FAAE`, two rows. Lurking Horror, Moonmist, Stationfall,
/// Cutthroats and Hitchhiker's are duplicated the same way across that nine-disk
/// shelf: 39 rows for 33 games.
///
/// Three properties this deliberately has:
///
/// - **Scoped to one set.** Two rows are folded together only when they are
///   volumes of one release. The same Zork Zero build (`ZCODE-393-890714-791C`)
///   sits on `floppy5.ima` and on both of the DOS 360K/720K presses; those are
///   three separate sets and stay three rows, because they are three pieces of
///   media the player deliberately keeps. Nothing outside a set is ever folded.
/// - **Keyed on the IFID**, which for Z-code is release, serial and checksum —
///   the identity of a *build*. Zork Zero's r296, r366 and r393 are three
///   different builds and therefore three rows however they are reached, which
///   is the same rule SQ-0850 keys their saves on.
/// - **The lowest disk number wins.** `disk_set::group` returns its members in
///   disk order and the scan walks a sorted file list, so which copy survives is
///   fixed and reproducible rather than whatever `read_dir` happened to yield.
fn dedupe_within_sets(out: &mut Vec<StoryEntry>, sets: &[Vec<PathBuf>]) {
    // Rank every set member by its disk number; a file in no set gets no rank
    // and is never a candidate for folding.
    let mut rank: std::collections::HashMap<&Path, (usize, usize)> =
        std::collections::HashMap::new();
    for (set_idx, members) in sets.iter().enumerate() {
        for (disk_idx, m) in members.iter().enumerate() {
            rank.insert(m.as_path(), (set_idx, disk_idx));
        }
    }
    if rank.is_empty() {
        return;
    }
    // Visit the set rows in (set, disk, position-on-disk) order and keep the
    // first row per (set, IFID). `out` is already in sorted-file order, so the
    // enumerate index is the within-disk order the mount reported.
    let mut order: Vec<(usize, usize, usize, usize)> = out
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let &(set_idx, disk_idx) = rank.get(e.path.as_path())?;
            Some((set_idx, disk_idx, i, i))
        })
        .collect();
    order.sort();
    let mut seen: std::collections::HashSet<(usize, String)> = std::collections::HashSet::new();
    let mut drop: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (set_idx, _disk, _pos, i) in order {
        if !seen.insert((set_idx, out[i].meta.ifid.clone())) {
            drop.insert(i);
        }
    }
    if drop.is_empty() {
        return;
    }
    let mut i = 0;
    out.retain(|_| {
        let keep = !drop.contains(&i);
        i += 1;
        keep
    });
}

/// Where the browser's list comes from (SQ-0844).
///
/// Both arms produce the same thing — a list of *stories*, one row per game,
/// exactly as SQ-0859 established — and differ only in which files they read.
/// The picker takes one of these rather than a bare directory so that naming a
/// single volume of a multi-disk release can open the whole shelf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorySource {
    /// Every story in one directory: the ordinary library launch.
    Library(PathBuf),
    /// Every story on one multi-disk release, reached by naming any one of its
    /// volumes — or on **one volume that holds several games** (SQ-0962), which
    /// is the same list read off a `members` of one. `dir` is the directory they
    /// share, which is still where a downloaded story would land and still what
    /// the browser's header names.
    DiskSet { dir: PathBuf, members: Vec<PathBuf> },
}

impl StorySource {
    /// What `path` means as a launch argument: a directory is a library, a
    /// volume of a multi-disk release is that release, a disk holding several
    /// games is those games, and anything else is none of the above.
    ///
    /// A source that offers **fewer than two games** is not one worth a menu, so
    /// it reports `None` and the caller opens the file the way it always did: a
    /// player naming Zork Zero's `(360K) (Disk 2)` wants Zork Zero, not a
    /// one-row browser, and so does one naming the DiskCopy *Lost Treasures*
    /// `Disk 5 - Zork Zero`, whose whole content is one game. The set is still
    /// *recognised* — this is only about whether it is worth presenting.
    ///
    /// # One disk is not one game (SQ-0962)
    ///
    /// This asked `disk_set::members` and stopped when the answer was `None`, so
    /// a volume belonging to no multi-disk release was launched as **whatever
    /// story the format's tiebreak preferred** however many it held. Pointed at
    /// the DiskCopy *Lost Treasures* disk 1 — *Beyond Zork* and three copies of
    /// *The Lurking Horror* — lanthorn started Beyond Zork and there was no way
    /// to reach the other game; `InfocomMasterpieces.img` opened one of
    /// thirty-three. It was never a missing feature: `meta.disk_entry` threads a
    /// chosen story through the picker, the launch dialog and the save key, and
    /// [`dedupe_within_a_volume`] exists specifically for a volume holding
    /// several. The chooser was simply not reached, because "is this a set?" was
    /// standing in for "is there a choice to make?" and they are different
    /// questions — a compilation on a single disc is a shelf too.
    ///
    /// The mount that answers it is the cost, so it is asked only of files that
    /// really are disk images, and only after the cheap name-only rule has
    /// declined. Every loose story file still leaves here on the first line.
    pub fn of(path: &Path, data_base: &Path) -> Option<StorySource> {
        if path.is_dir() {
            return Some(StorySource::Library(path.to_path_buf()));
        }
        let members = crate::disk_set::members(path)
            .or_else(|| holds_several_games(path).then(|| vec![path.to_path_buf()]))?;
        let dir = path.parent()?.to_path_buf();
        let source = StorySource::DiskSet { dir, members };
        (source.scan(data_base).len() >= 2).then_some(source)
    }

    /// The directory these stories live in — where a download lands and what the
    /// browser's header shows.
    pub fn dir(&self) -> &Path {
        match self {
            StorySource::Library(dir) => dir,
            StorySource::DiskSet { dir, .. } => dir,
        }
    }

    /// This source's stories, sorted by title, deduped within the set.
    pub fn scan(&self, data_base: &Path) -> Vec<StoryEntry> {
        match self {
            StorySource::Library(dir) => scan_stories(dir, data_base),
            StorySource::DiskSet { members, .. } => {
                let mut out: Vec<StoryEntry> = Vec::new();
                for m in members {
                    out.extend(resolve_entries(m, data_base));
                }
                // **Only across volumes**, because that is what the fold means:
                // "a build an EARLIER volume already offered". A lone volume has
                // no earlier one, and applying it anyway would fold the pair a
                // hybrid disc keeps on purpose — Zork I is r88/840726 on both
                // sides of *Masterpieces*, and telling the machines apart is
                // `dedupe_within_a_volume`'s whole subtlety (SQ-0878/SQ-0962).
                if members.len() > 1 {
                    dedupe_within_sets(&mut out, std::slice::from_ref(members));
                }
                associate_hint_sidecars(&mut out);
                sort_stories(&mut out, Sort { key: SortKey::Title, desc: false });
                out
            }
        }
    }
}

/// Does `path` hold **two or more** launchable-looking stories of its own?
///
/// The mount, asked once, of a file that already looks like a disk image; a
/// story file reads its bytes, fails `DiskImage::detect` and costs nothing more.
/// It answers the question [`StorySource::of`] used to answer with "is it a
/// volume of a set?", which was a different question (SQ-0962).
///
/// A **zip** is asked the same question (SQ-1098) — it is a container of
/// stories exactly as a compilation disc is, and a download of two games was
/// opening one of them silently. The four-byte magic check declines every
/// ordinary story file before anything is inflated.
fn holds_several_games(path: &Path) -> bool {
    crate::hints::mounted_stories(path).is_some_and(|(_, stories)| stories.len() >= 2)
        || crate::hints::zipped_stories(path).is_some_and(|stories| stories.len() >= 2)
}

/// Second pass over a freshly-scanned list: attach each detected InvisiClues/
/// hint sidecar to the game it belongs to and hide the sidecar's own row.
///
/// A sidecar ([`hints::is_hint_sidecar`]) is matched to a game when its
/// curated/derived game key is contained in the game's filename stem OR its
/// title. Every game keeps at most one sidecar (first after a stable filename
/// sort). Sidecars matched to some present game are removed from `out`; a lone
/// sidecar with no matching game stays listed. O(games × sidecars) — the list
/// is small and built once.
/// The name a row is JUDGED by when asking whether it is a hint file.
///
/// Its filename, except for a CONTAINER — a `.zip`, admitted to the scan by
/// SQ-1086. `hints::hint_name_matches` requires a `.z3`/`.z5`/`.z8` extension,
/// and its reason for requiring one is to reject a `hints.txt` DOCUMENT; an
/// archive that unwrapped into a playable Z-machine image is not a document.
/// Without this, `deadline-hints.zip` lists as a game of its own beside the very
/// story it belongs to, while the identical loose `deadlineinv.z5` folds into
/// that story's row — the same file, two answers, decided by its wrapper.
fn hint_classification_name(filename: &str) -> String {
    let p = Path::new(filename);
    match p.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("zip") => {
            format!("{}.z5", p.file_stem().and_then(|s| s.to_str()).unwrap_or(filename))
        }
        _ => filename.to_string(),
    }
}

fn associate_hint_sidecars(out: &mut Vec<StoryEntry>) {
    // What each row is judged by — see `hint_classification_name`. Computed once
    // so the split below and the matching further down cannot ask two different
    // questions about the same row.
    let names: Vec<String> = out.iter().map(|e| hint_classification_name(&e.filename)).collect();
    // Split into sidecar and game indices.
    let mut sidecar_idxs: Vec<usize> = Vec::new();
    let mut game_idxs: Vec<usize> = Vec::new();
    for (i, _e) in out.iter().enumerate() {
        if hints::is_hint_sidecar(&names[i]) {
            sidecar_idxs.push(i);
        } else {
            game_idxs.push(i);
        }
    }
    // Stable candidate order so association is deterministic — and a LOOSE hint
    // file ahead of an archived one, so admitting `.zip` to the scan (SQ-1086)
    // cannot change which sidecar an existing library already resolves. An
    // archive is chosen only when nothing loose answers, which is exactly the
    // case that had no answer at all before.
    sidecar_idxs.sort_by(|&a, &b| {
        let arch = |i: usize| {
            Path::new(&out[i].filename)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
        };
        arch(a).cmp(&arch(b)).then_with(|| out[a].filename.cmp(&out[b].filename))
    });

    let mut matched: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for &g in &game_idxs {
        let stem = out[g]
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let title = out[g].title.clone();
        let ifid = out[g].meta.ifid.clone();
        let chosen = sidecar_idxs.iter().copied().find(|&s| {
            // Identity first (SQ-0767): a story mounted out of a disk image is
            // named for the box, so neither its stem nor its title can say
            // which clues file is its own.
            hints::hint_matches_identity(&names[s], &ifid)
                || hints::hint_matches_story(&names[s], &stem)
                || hints::hint_matches_story(&names[s], &title)
        });
        if let Some(s) = chosen {
            out[g].hint_sidecar = Some(out[s].path.clone());
            matched.insert(out[s].path.clone());
        }
    }
    // Hide the sidecars that were associated with some present game.
    out.retain(|e| !matched.contains(&e.path));
}

/// Resolve one story file into a [`StoryEntry`], re-reading its bytes and its
/// (possibly just-updated) IFDB sidecar. `None` if the file doesn't load or
/// isn't launchable. Shared by `scan_stories` (the initial directory scan) and
/// the picker's fetch-progress handler (SQ-0348), which re-resolves a single
/// story right after its sidecar is (re)written so a completed fetch's title/
/// author/year land in the list without a full re-scan.
pub fn resolve_entry(path: &Path, data_base: &Path) -> Option<StoryEntry> {
    resolve_entry_from(path, None, data_base)
}

/// [`resolve_entry`] for one **named** story off a disk image that holds several
/// (SQ-0859). `None` is the format's own tiebreak, i.e. exactly
/// [`resolve_entry`].
pub fn resolve_entry_from(
    path: &Path,
    disk_entry: Option<&str>,
    data_base: &Path,
) -> Option<StoryEntry> {
    let (loaded, disk_image) = crate::hints::load_mounted_story_from(path, disk_entry).ok()?;
    entry_from_loaded(path, disk_entry, loaded, disk_image, data_base)
}

/// **Every** launchable story `path` offers, as its own row.
///
/// One for an ordinary story file, and one *per game* for a container that
/// holds several — the fix for the browser listing `INFOCOM6` once and opening
/// whichever story the format's tiebreak preferred, leaving the other four
/// unreachable however long you looked at the list (SQ-0859), and for the same
/// thing happening to a zip of two games (SQ-1098).
///
/// The image is mounted **once** and every row is built from that one mount, so
/// a six-game disk costs the read it always cost. A container holding one story
/// takes the plain path with no selector at all: nothing about a single-game
/// floppy or a single-game download changes, which is most of the corpus.
pub fn resolve_entries(path: &Path, data_base: &Path) -> Vec<StoryEntry> {
    if let Some((_, stories)) = crate::hints::mounted_stories(path) {
        if stories.len() >= 2 {
            let mut rows: Vec<StoryEntry> = stories
                .into_iter()
                .filter_map(|(story, image)| {
                    let loaded = crate::hints::extract_story(story.bytes).ok()?;
                    // `image` is THIS story's, not the volume's: on a hybrid
                    // disc the two differ, and the row's badge and interpreter
                    // both follow from it (SQ-0876).
                    entry_from_loaded(path, Some(&story.name), loaded, Some(image), data_base)
                })
                .collect();
            // One build per machine, once — the same fold the directory scan
            // applies, applied here because this door builds a volume's rows on
            // its own (SQ-0878).
            dedupe_within_a_volume(&mut rows);
            return rows;
        }
        return resolve_entry(path, data_base).into_iter().collect();
    }
    // A zip is a container too (SQ-1098). Its entries carry no `DiskImage`, so
    // every row's save key is its ENTRY's basename — which is what had to be
    // settled before this line could exist at all, because two rows keyed on
    // the archive's own name would have shared one save directory.
    //
    // Deliberately NOT deduped the way a volume is: two entries of one archive
    // are somebody's own filing, and `game.z5` and `game.zblorb` are already
    // two save directories by the basename rule, so folding them would hide a
    // row whose saves are its own.
    if let Some(stories) = crate::hints::zipped_stories(path) {
        if stories.len() >= 2 {
            return stories
                .into_iter()
                .filter_map(|(name, bytes)| {
                    let loaded = crate::hints::extract_story(bytes).ok()?;
                    entry_from_loaded(path, Some(&name), loaded, None, data_base)
                })
                .collect();
        }
    }
    resolve_entry(path, data_base).into_iter().collect()
}

/// The body both doors share: build one row out of a story that is already
/// loaded, whichever door loaded it.
fn entry_from_loaded(
    path: &Path,
    disk_entry: Option<&str>,
    loaded: crate::hints::LoadedStory,
    disk_image: Option<crate::hints::DiskImage>,
    data_base: &Path,
) -> Option<StoryEntry> {
    // Only list stories lanthorn can actually launch: Z-code via the
    // Z-machine loader (accepts v3/4/5/7/8, rejects v6/v1/v2), Glulx via the
    // Glulx loader, Scott Adams via the Scott database parser.
    let bytes = loaded.bytes().to_vec();
    let launchable = match &loaded {
        crate::hints::LoadedStory::ZCode(b) => zvm::memory::Memory::new(b.clone()).is_ok(),
        crate::hints::LoadedStory::Glulx(b) => gvm::Memory::new(b.clone()).is_ok(),
        crate::hints::LoadedStory::Scott(b) => {
            std::str::from_utf8(b).ok().map(|s| scott::Database::parse(s).is_ok()).unwrap_or(false)
        }
    };
    if !launchable {
        return None;
    }
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let ifid = crate::ifid::compute_ifid(&bytes);

    // fs metadata: size + mtime → "YYYY-MM-DD". `size_bytes` measures the file
    // on disk; `story_bytes` measures what was mounted out of it (SQ-0771).
    let fs_meta = std::fs::metadata(path).ok();
    let size_bytes = fs_meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let story_bytes = bytes.len() as u64;
    let modified = fs_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(format_mtime_ymd);

    // Self-blorb chunks: only blorb-container files carry a resource index,
    // and extraction (`load_story`) discards it — re-read the raw file for
    // those extensions only, so plain .z* files stay single-read. The same
    // parse yields the `IFmd` chunk (if any) for precedence resolution below.
    let mut ifmd: Option<crate::ifiction::IFiction> = None;
    let self_blorb = if is_blorb_ext(path) {
        std::fs::read(path).ok().and_then(|raw| {
            if blorb::Blorb::is_blorb(&raw) {
                blorb::Blorb::parse(raw).ok().map(|b| {
                    if let Some(xml) = b.metadata() {
                        ifmd = crate::ifiction::parse(xml).ok();
                    }
                    chunks_of(&b)
                })
            } else {
                None
            }
        })
    } else {
        None
    };

    // Fetched IFDB sidecar: absent (never fetched, unreadable, malformed,
    // wrong IFID) is simply no metadata, never a scan error. The mount is
    // already done, so the disk-image save key (SQ-0850) costs nothing here.
    let disk_build = disk_image.and_then(|kind| crate::storage::DiskBuild::of(&bytes, kind));
    let game_dir = crate::storage::game_dir(
        data_base,
        &crate::storage::story_key_for(crate::storage::StoryOrigin {
            path,
            // The row does not exist yet, so this is the one site that cannot
            // ask `StoryEntry::story_key`. It states all three facts anyway,
            // and the struct is what makes leaving one out a compile error.
            entry: disk_entry,
            build: disk_build.as_ref(),
        }),
    );
    let fetched = crate::story_info::load(&game_dir, &ifid).and_then(|info| info.fetched);
    // Scott stories have no IFID-keyed table; resolve their title (and, for the
    // homebrew games with no IFDB record, author/description) from the filename
    // stem via the bundled filename->metadata table instead.
    //
    // A story chosen off a compilation answers to the name the DISK gives it
    // instead (SQ-0859): the image's stem names the box, so five rows off
    // `INFOCOM6` would every one of them fall back to *Lost Treasures of Infocom
    // (Disk 6 of 7)*. `LEATHRGODDESSES` is the row that needs it — its header
    // reads release 0 serial `Blown!`, which no title table answers to, so the
    // last resort is all it has. (A disk story is never Scott — every mountable
    // format here is Infocom Z-code — so the Scott lookups below are unaffected
    // by the substitution.)
    let stem = disk_entry.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&filename)
    });
    let is_scott = matches!(loaded, crate::hints::LoadedStory::Scott(_));
    let tsv_title = bundled_title(stem, &ifid, is_scott);
    let tsv_author = is_scott.then(|| scott_author(stem)).flatten();
    let tsv_description = is_scott.then(|| scott_description(stem)).flatten();
    let resolved = resolve(
        ifmd.as_ref(),
        fetched.as_ref(),
        tsv_title,
        tsv_author,
        tsv_description,
        stem,
    );
    let title = resolved.title;

    let engine = match &loaded {
        crate::hints::LoadedStory::ZCode(_) => Engine::ZCode,
        crate::hints::LoadedStory::Glulx(_) => Engine::Glulx,
        crate::hints::LoadedStory::Scott(_) => Engine::Scott,
    };
    let is_container = self_blorb.is_some();
    let (version, serial, release, features, format) = match engine {
        Engine::ZCode => {
            let version = z_version(&bytes).map(|v| v.to_string());
            let serial = z_serial(&bytes);
            let release = z_release(&bytes);
            let features = z_features(&bytes, self_blorb.as_deref());
            let format = if is_container { "Blorb (Z-code)" } else { "Z-code" };
            (version, serial, release, features, format.to_string())
        }
        Engine::Glulx => {
            let version = glulx_version(&bytes);
            let features = glulx_features(self_blorb.as_deref());
            let format = if is_container { "Blorb (Glulx)" } else { "Glulx" };
            (version, None, None, features, format.to_string())
        }
        // Scott Adams databases carry no version/serial/release. The graphic
        // (SAGA/Mysterious Adventures) versions ship in a blorb (`.blb`); a plain
        // `.dat` does not.
        Engine::Scott => {
            let format = if is_container { "Blorb (Scott Adams)" } else { "Scott Adams" };
            (None, None, None, Features::default(), format.to_string())
        }
    };

    let meta = StoryMeta {
        size_bytes,
        story_bytes,
        modified,
        engine,
        format,
        version,
        serial,
        release,
        ifid,
        features,
        self_blorb,
        disk_image,
        disk_entry: disk_entry.map(str::to_string),
        author: resolved.author,
        year: resolved.year,
        genre: resolved.genre,
        language: resolved.language,
        description: resolved.description,
        ifdb_link: resolved.ifdb_link,
        ifdb_rating: resolved.ifdb_rating,
        ifdb_rating_count: resolved.ifdb_rating_count,
        fetch_not_found: resolved.fetch_not_found,
    };
    Some(StoryEntry { path: path.to_path_buf(), title, filename, meta, hint_sidecar: None, kind: RowKind::Story })
}

/// Column a story list can be sorted by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Title,
    Author,
    Year,
    Rating,
    Type,
}

/// A sort column plus direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sort {
    pub key: SortKey,
    pub desc: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort { key: SortKey::Title, desc: false }
    }
}

/// A lowercased title sort key with a leading English article dropped, so
/// "The Lurking Horror" files under L and "A Mind Forever Voyaging" under M —
/// standard bibliographic ordering (SQ-0373). Only strips an article that is
/// followed by more text (a story literally titled "The" keeps it).
fn bibliographic_key(title: &str) -> String {
    let lower = title.trim().to_lowercase();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(article) {
            let rest = rest.trim_start();
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    lower
}

/// The container name the TYPE column prints in parentheses for a row, or
/// `None` when the row shows no parenthetical at all.
///
/// **One source of truth for two readers**: `picker_ui::interp_label` renders
/// this into the column ("Z6 (ADF)", "Z5 (blorb)"), and `sort_stories`' Type
/// key orders by it. They were two independent rules until SQ-1057, and the
/// sort's rule was simply to ignore the container — so `Z6`, `Z6 (ADF)` and
/// `Z6 (HFS)` interleaved under the filename tiebreak and a reader sorting by
/// TYPE to group their original media by machine got nothing of the sort.
/// A second copy of the `match` below would drift the same way, in the other
/// direction: a key that named a container the label never prints would order
/// rows by something the reader cannot see.
///
/// The rule is per engine, because the label's is:
/// - **Z-code** shows the disk image it was mounted out of when there is one
///   (that is the mount's own answer, not the filename's), else "blorb".
/// - **Glulx** shows no container ever, blorbed or not — Glulx games are
///   effectively always blorbed, so the suffix would say nothing (SQ-0369).
/// - **Scott** shows "blorb" for the graphic `.blb` versions only.
///
/// `blorb` is `RowBadges::blorb` — see [`row_is_blorb`], which costs a
/// directory read.
pub fn type_container(meta: &StoryMeta, blorb: bool) -> Option<&'static str> {
    match meta.engine {
        Engine::ZCode => match meta.disk_image {
            Some(image) => Some(image.label()),
            None => blorb.then_some("blorb"),
        },
        Engine::Glulx => None,
        Engine::Scott => blorb.then_some("blorb"),
    }
}

/// Is this row blorb-wrapped — itself a blorb, or sitting beside a resource
/// blorb? The fact behind `RowBadges::blorb` and behind the TYPE column's
/// "(blorb)". **It touches the filesystem** (`sibling_blorb_exists`), so a
/// caller that needs it for many rows must measure each row once; see
/// `sort_stories`.
fn row_is_blorb(entry: &StoryEntry) -> bool {
    #[cfg(test)]
    TYPE_BLORB_PROBES.with(|c| c.set(c.get() + 1));
    entry.meta.self_blorb.is_some() || sibling_blorb_exists(&entry.path)
}

// Counts `row_is_blorb` calls on the CURRENT THREAD, so a test can pin that
// the Type sort measures each row once rather than once per comparison.
// Thread-local rather than a global counter on purpose: every test runs on its
// own thread, and under `cargo test` (one process, many threads) a shared
// counter would be a race between any two tests that scan or sort.
#[cfg(test)]
thread_local! {
    static TYPE_BLORB_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Order `stories` in place by `sort`. Blanks (no author / no year, a
/// non-numeric year, or no IFDB rating) always sort last, in both ascending and descending
/// order — only the non-blank comparison reverses with `desc`. Filename is
/// the tie-break in every case.
pub fn sort_stories(stories: &mut [StoryEntry], sort: Sort) {
    use std::cmp::Ordering;

    /// Compares two `(is_blank, value)` keys: blank entries always sort last,
    /// non-blank entries compare by `value` (reversed when `desc`).
    fn cmp_blank_last<T: Ord>(
        a_blank: bool,
        a_val: &T,
        b_blank: bool,
        b_val: &T,
        desc: bool,
    ) -> Ordering {
        match (a_blank, b_blank) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                let ord = a_val.cmp(b_val);
                if desc { ord.reverse() } else { ord }
            }
        }
    }

    fn title_key(e: &StoryEntry) -> (bool, String) {
        let t = bibliographic_key(&e.title);
        (t.is_empty(), t)
    }

    fn author_key(e: &StoryEntry) -> (bool, String) {
        // Case-insensitive, like the title sort: a plain byte sort would file
        // every capitalised author ahead of every lowercase one ("Zarf" before
        // "adam cadre"), which reads as broken in a name list.
        let a = e.meta.author.clone().unwrap_or_default();
        (a.is_empty(), a.to_lowercase())
    }

    fn year_key(e: &StoryEntry) -> (bool, i64) {
        match e.meta.year.as_deref().and_then(|s| s.trim().parse::<i64>().ok()) {
            Some(n) => (false, n),
            None => (true, 0),
        }
    }

    /// IFDB's average rating as tenths, so it sorts through the same `Ord`
    /// path as every other key (`f32` is not `Ord`). Unrated — including a
    /// story that has simply never been fetched — is blank, so it lands last
    /// in both directions. The rating count is the tiebreak: between two 4.6s,
    /// the one 200 people rated outranks the one 3 people did.
    fn rating_key(e: &StoryEntry) -> (bool, (u32, u32)) {
        match e.meta.ifdb_rating {
            Some(r) if r.is_finite() && r > 0.0 => (
                false,
                ((r * 10.0).round().max(0.0) as u32, e.meta.ifdb_rating_count.unwrap_or(0)),
            ),
            _ => (true, (0, 0)),
        }
    }

    /// Groups rows by engine (Z-code, then Glulx, then Scott), then within an
    /// engine by version, then by the CONTAINER the TYPE column prints for the
    /// row. Each dotted version component is zero-padded to a fixed width so a
    /// plain string compare orders numerically (Z3 < Z5 < Z8, Glulx 3.1.2 <
    /// 3.1.11). Every story has an engine, so the key is never blank.
    ///
    /// Engine and version dominate the container, because that is the order the
    /// label reads in: a `Z3 (HFS)` is a Z3 first and a Macintosh disk second.
    /// Within one engine and version, a row with NO parenthetical sorts first
    /// (nothing before something — its label is a prefix of every other label
    /// in the group), then containers alphabetically and
    /// **case-insensitively**: "blorb" is a format name and "ADF"/"HFS"/"DOS"
    /// are acronyms, so a raw byte compare would file every acronym ahead of
    /// "blorb" by that accident of casing rather than by anything a reader
    /// means. See [`type_container`] for what a row shows, which is the same
    /// call the column itself makes (SQ-1057).
    ///
    /// **Costs a filesystem read** via [`row_is_blorb`], which is why
    /// `sort_stories` measures it once per row instead of inside the
    /// comparator.
    fn type_key(e: &StoryEntry) -> String {
        let rank = match e.meta.engine {
            Engine::ZCode => 0,
            Engine::Glulx => 1,
            Engine::Scott => 2,
        };
        let version: String = e
            .meta
            .version
            .as_deref()
            .unwrap_or("")
            .split('.')
            .map(|part| format!("{part:0>4}"))
            .collect::<Vec<_>>()
            .join(".");
        let container = type_container(&e.meta, row_is_blorb(e))
            .map(str::to_lowercase)
            .unwrap_or_default();
        format!("{rank} {version} {container}")
    }

    // The Type key is the one key here whose value costs a FILESYSTEM read —
    // the container a row shows can be a sibling blorb, and only a directory
    // lookup answers that. Every row's key is measured ONCE, before any
    // comparison; inside the comparator it would be O(n log n) directory reads.
    // The other four keys are pure and are still taken from the entry.
    let type_keys: Vec<String> = if sort.key == SortKey::Type {
        stories.iter().map(type_key).collect()
    } else {
        Vec::new()
    };

    // Sorting a permutation rather than the rows, so the comparator can reach
    // each row's pre-measured key by its ORIGINAL index.
    /// Which shelf a row sits on: `..` above the folders, folders above the
    /// stories, under every sort key and in both directions. A folder has no
    /// author, year or rating, and letting it sink to the bottom as a blank
    /// would hide the way up under two thousand stories.
    fn shelf(e: &StoryEntry) -> u8 {
        match e.kind {
            RowKind::Folder if e.title == PARENT_LABEL => 0,
            RowKind::Folder => 1,
            RowKind::Story => 2,
        }
    }

    let mut order: Vec<usize> = (0..stories.len()).collect();
    order.sort_by(|&i, &j| {
        let (a, b) = (&stories[i], &stories[j]);
        let (sa, sb) = (shelf(a), shelf(b));
        if sa != sb {
            return sa.cmp(&sb);
        }
        if sa < 2 {
            // Folders sort by name alone, whatever column the stories are on.
            return a.title.to_lowercase().cmp(&b.title.to_lowercase()).then_with(|| a.title.cmp(&b.title));
        }
        let ord = match sort.key {
            SortKey::Title => {
                let (a_blank, a_val) = title_key(a);
                let (b_blank, b_val) = title_key(b);
                cmp_blank_last(a_blank, &a_val, b_blank, &b_val, sort.desc)
            }
            SortKey::Author => {
                let (a_blank, a_val) = author_key(a);
                let (b_blank, b_val) = author_key(b);
                cmp_blank_last(a_blank, &a_val, b_blank, &b_val, sort.desc)
            }
            SortKey::Year => {
                let (a_blank, a_val) = year_key(a);
                let (b_blank, b_val) = year_key(b);
                cmp_blank_last(a_blank, &a_val, b_blank, &b_val, sort.desc)
            }
            SortKey::Rating => {
                let (a_blank, a_val) = rating_key(a);
                let (b_blank, b_val) = rating_key(b);
                cmp_blank_last(a_blank, &a_val, b_blank, &b_val, sort.desc)
            }
            SortKey::Type => {
                cmp_blank_last(false, &type_keys[i], false, &type_keys[j], sort.desc)
            }
        };
        ord.then_with(|| a.filename.cmp(&b.filename))
    });
    let reordered: Vec<StoryEntry> = order.iter().map(|&i| stories[i].clone()).collect();
    for (slot, entry) in stories.iter_mut().zip(reordered) {
        *slot = entry;
    }
}

/// Reorder `stories` by `sort`, keeping the selection on the same story — by
/// path, never by index. Three things reorder the picker's list (changing the
/// sort key, toggling direction, and an `r` sweep landing new titles under a
/// cursor the user isn't touching), and every one of them must not silently
/// move the cursor to a different game. Returns the new index of the
/// previously-selected story (or `0` if it's gone, e.g. an empty list).
pub fn resort_preserving_selection(stories: &mut [StoryEntry], selected: usize, sort: Sort) -> usize {
    let keep = stories.get(selected).map(|e| e.path.clone());
    sort_stories(stories, sort);
    keep.and_then(|p| stories.iter().position(|e| e.path == p)).unwrap_or(0)
}

/// A row's hint state, driving which (if any) hint glyph the row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HintBadge {
    /// No hint locally and none available to download.
    #[default]
    None,
    /// No local hint, but a matching InvisiClues can be downloaded (`H`) —
    /// shown as the lowercase available-hint glyph.
    Available,
    /// A hint file is present locally (a sidecar or a remembered association) —
    /// shown as the uppercase hint glyph.
    Present,
}

/// Cheap existence flags shown on every list row (panel-independent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowBadges {
    pub blorb: bool,
    pub save: bool,
    pub hint: HintBadge,
}

/// True if `path` has an associated resource blorb — an exact same-stem
/// `.blb`/`.blorb`/`.zblorb` sibling, or (like the info panel's resource
/// resolution) an unambiguous stem-prefix match in the same directory, e.g.
/// `Lurking.blb` for `lurkinghorror-r219-s870912.z3`. Filename-only, so the
/// per-row `(blorb)` tag stays cheap (no blorb parsing).
fn sibling_blorb_exists(path: &Path) -> bool {
    blorb::sibling_blorb_by_name(path).is_some()
}

/// Compute a row's artifact badges. `data_base` is the storage base; the save
/// badge lights when the story's per-game dir `<data_base>/<story-key>/` exists
/// and holds a `.lanthorn` or `.qzl` (SQ-0284). `hint_index` (IFID-keyed) is
/// loaded once at picker start. No archive reads.
pub fn compute_row_badges(
    entry: &StoryEntry,
    data_base: &Path,
    hint_index: &hints::HintIndex,
) -> RowBadges {
    if entry.is_folder() {
        return RowBadges::default();
    }
    let ifid = &entry.meta.ifid;
    let game_dir = entry.game_dir(data_base);
    let hint = if hint_index.get(ifid).is_some() || entry.hint_sidecar.is_some() {
        HintBadge::Present
    } else {
        // No local hint — light the lowercase glyph if one is downloadable.
        let stem = entry.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if hints::hint_download_for(&entry.meta.ifid, stem, &entry.title).is_some() {
            HintBadge::Available
        } else {
            HintBadge::None
        }
    };
    RowBadges {
        blorb: row_is_blorb(entry),
        save: game_dir_has_save(&game_dir),
        hint,
    }
}

/// True if `game_dir` exists and contains at least one `.lanthorn` or `.qzl`.
fn game_dir_has_save(game_dir: &Path) -> bool {
    std::fs::read_dir(game_dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.ends_with(".lanthorn") || n.ends_with(".qzl"))
        })
}

/// Borrowed badge glyphs from the `[symbols]` config, for row rendering.
///
/// The story TYPE and the Blorb had glyphs here until SQ-1160. SQ-0369 had
/// already moved both into the row's TYPE column as text (`Z5 (blorb)`), so
/// they were borrowed by no drawing code for a year; what remains is what the
/// row actually paints.
pub struct BadgeGlyphs<'a> {
    pub save: &'a str,
    pub hint: &'a str,
    pub hint_available: &'a str,
}

impl<'a> BadgeGlyphs<'a> {
    pub fn from_symbols(s: &'a crate::config::SymbolConfig) -> Self {
        Self {
            save: &s.badge_save,
            hint: &s.badge_hint,
            hint_available: &s.badge_hint_available,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid v3 story bytes (same minimal header as the render tests).
    fn minimal_v3_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 3;
        buf[0x04] = 0x00; buf[0x05] = 0x40;
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        buf[0x0A] = 0x00; buf[0x0B] = 0x80;
        buf[0x0C] = 0x01; buf[0x0D] = 0x00;
        buf[0x0E] = 0x03; buf[0x0F] = 0x00;
        buf[0x08] = 0x04; buf[0x09] = 0x00;
        buf[0x18] = 0x00; buf[0x19] = 0x60;
        // A printable serial (ZMSD §11.1, bytes $12–$17). It was six zero bytes
        // until SQ-0889, which cost nothing while a version byte was the whole
        // of a story's claim and costs the fixture its identity now that one has
        // to look like a story: `$12..$18` being binary is precisely how a saved
        // game is told from a game.
        buf[0x12..0x18].copy_from_slice(b"000000");
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf
    }

    fn temp_dir(tag: &str) -> PathBuf {
        crate::scratch_dir(&format!("picker-{tag}"))
    }

    #[test]
    fn scan_lists_valid_stories_and_skips_junk() {
        let dir = temp_dir("scan");
        std::fs::write(dir.join("game.z5"), minimal_v3_story()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a story").unwrap();   // wrong ext
        std::fs::write(dir.join("broken.z5"), b"garbage").unwrap();       // bad header

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1, "only the valid .z5 is listed");
        assert_eq!(stories[0].filename, "game.z5");
        // No known title for this synthetic IFID → falls back to the stem.
        assert_eq!(stories[0].title, "game");
    }

    /// An Amiga release floppy is a listable story file (SQ-0719) — the picker
    /// offers it and `load_story` mounts the game out of it.
    #[test]
    fn disk_images_are_listed_as_stories() {
        assert!(has_story_ext(Path::new("Zork Zero.adf")));
        assert!(has_story_ext(Path::new("DISK1.ADF")), "matched case-insensitively");
    }

    /// **Every format `blorb` mounts, the scan is willing to open** — the whole
    /// of SQ-0849. The enumeration is the format table's, not a copy of it, so a
    /// format added there arrives here in the same commit and cannot go missing
    /// from the story list the way the DOS and ST presses did.
    ///
    /// FALSIFICATION: put the disk spellings back into `STORY_EXTS` by hand as
    /// `"adf", "image"` and this fails on `ima` — the originally reported
    /// symptom, that `.ima` and `.st` media are never offered.
    #[test]
    fn the_scan_admits_every_extension_the_format_table_names() {
        for ext in blorb::medium::image_extensions() {
            assert!(
                has_story_ext(Path::new(&format!("release.{ext}"))),
                "a .{ext} disk image is a format blorb mounts but the scan will not open"
            );
            assert!(
                has_story_ext(Path::new(&format!("RELEASE.{}", ext.to_ascii_uppercase()))),
                ".{ext} is matched case-insensitively, like every other candidate"
            );
        }
        // The pre-existing bare-story behaviour is untouched by the union.
        for ext in ["z3", "z5", "z8", "zblorb", "dat", "ulx", "blb"] {
            assert!(has_story_ext(Path::new(&format!("game.{ext}"))), "{ext}");
        }
        // …and the union admits nothing beyond the two lists. `2mg` used to sit
        // in this list and now sits in the loop above, having moved from one to
        // the other the moment `blorb` learned to mount ProDOS (SQ-0836) — with
        // nothing in this file edited to make it happen, which is the point.
        // `dsk` made the same crossing in SQ-0864, when the ProDOS row learned
        // that a 5.25-inch dump is the same filesystem in the drive's sector
        // order; this file was again not touched to let it through.
        for ext in ["txt", "png", "qzl"] {
            assert!(!has_story_ext(Path::new(&format!("thing.{ext}"))), "{ext}");
        }
    }

    /// The union is safe **because content decides**: a `.img` that is not a
    /// disk image is opened, refused and never listed (SQ-0849's guard).
    ///
    /// This is the property that lets the pre-filter be generous. Admitting
    /// `.img` would be reckless if the extension were evidence of anything; it
    /// is not, so the file is read, `DiskImage::detect` declines it, and the
    /// scan drops it exactly as it drops a corrupt `.z5`.
    #[test]
    fn a_file_that_only_looks_like_a_disk_image_is_not_listed() {
        let dir = temp_dir("notadisk");
        std::fs::write(dir.join("holiday.img"), vec![0u8; 64 * 1024]).unwrap();
        std::fs::write(dir.join("notes.ima"), b"this is not a floppy").unwrap();
        std::fs::write(dir.join("archive.st"), vec![0x5au8; 8 * 1024]).unwrap();
        // A real story beside them, so an empty list cannot pass by accident.
        std::fs::write(dir.join("game.z5"), minimal_v3_story()).unwrap();

        let stories = scan_stories(&dir, &dir);
        let names: Vec<String> = stories.iter().map(|s| s.filename.clone()).collect();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(names, vec!["game.z5".to_string()], "only the real story is listed: {names:?}");
    }

    #[test]
    fn scan_lists_v6_but_skips_unsupported_versions() {
        let dir = temp_dir("v6");
        // v6 is supported since SQ-0186 (it boots) — a v6 story with the real
        // `.z6` extension IS now listed (the extension is in STORY_EXTS and the
        // header parses).
        let mut v6 = minimal_v3_story();
        v6[0x00] = 6;
        std::fs::write(dir.join("graphic.z6"), &v6).unwrap();
        // v1/v2 remain unsupported (parse_header rejects them) → skipped.
        let mut v1 = minimal_v3_story();
        v1[0x00] = 1;
        std::fs::write(dir.join("old.z5"), &v1).unwrap();

        let stories = scan_stories(&dir, &dir);
        let names: Vec<String> = stories.iter().map(|s| s.filename.clone()).collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(names.iter().any(|n| n == "graphic.z6"), "v6 .z6 story is listed (supported): {names:?}");
        assert!(!names.iter().any(|n| n == "old.z5"), "v1 remains unsupported → skipped: {names:?}");
    }

    #[test]
    fn scan_sorts_by_title() {
        let dir = temp_dir("sort");
        std::fs::write(dir.join("zebra.z5"), minimal_v3_story()).unwrap();
        std::fs::write(dir.join("apple.z5"), minimal_v3_story()).unwrap();
        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);
        let titles: Vec<&str> = stories.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["apple", "zebra"]);
    }

    /// Builds a bare-bones `StoryEntry` for `sort_stories` tests: only
    /// title/filename/author/year vary, everything else is a placeholder.
    fn story(title: &str, filename: &str, author: Option<&str>, year: Option<&str>) -> StoryEntry {
        StoryEntry {
            path: PathBuf::from(filename),
            title: title.to_string(),
            filename: filename.to_string(),
            meta: StoryMeta {
                size_bytes: 0, story_bytes: 0,
                modified: None,
                engine: Engine::ZCode,
                format: "Z-code".to_string(),
                version: None,
                serial: None,
                release: None,
                ifid: String::new(),
                features: Features::default(),
                self_blorb: None,
                disk_image: None,
                disk_entry: None,
                author: author.map(|s| s.to_string()),
                year: year.map(|s| s.to_string()),
                genre: None,
                language: None,
                description: None, ifdb_link: None, ifdb_rating: None,
                ifdb_rating_count: None, fetch_not_found: false,
            },
            hint_sidecar: None,
            kind: RowKind::Story,
        }
    }

    fn titles_of(stories: &[StoryEntry]) -> Vec<&str> {
        stories.iter().map(|s| s.title.as_str()).collect()
    }

    #[test]
    fn sort_stories_title_ascending_case_insensitive() {
        let mut stories = vec![
            story("Zebra", "z.z5", None, None),
            story("apple", "a.z5", None, None),
            story("Mango", "m.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        assert_eq!(titles_of(&stories), vec!["apple", "Mango", "Zebra"]);
    }

    #[test]
    fn sort_stories_by_type_groups_by_engine_then_version() {
        // Type sort groups Z-code (ordered by version) < Glulx < Scott,
        // independent of title order.
        let typed = |title: &str, engine: Engine, version: Option<&str>| {
            let mut e = story(title, &format!("{title}.dat"), None, None);
            e.meta.engine = engine;
            e.meta.version = version.map(str::to_string);
            e
        };
        let mut stories = vec![
            typed("scott", Engine::Scott, None),
            typed("z8", Engine::ZCode, Some("8")),
            typed("glulx", Engine::Glulx, Some("3.1.2")),
            typed("z3", Engine::ZCode, Some("3")),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: false });
        assert_eq!(titles_of(&stories), vec!["z3", "z8", "glulx", "scott"]);

        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: true });
        assert_eq!(titles_of(&stories), vec!["scott", "glulx", "z8", "z3"]);
    }

    /// A row whose only container is a self-blorb, without touching the disk.
    fn blorbed(mut e: StoryEntry) -> StoryEntry {
        e.meta.self_blorb = Some(Vec::new());
        e
    }

    /// SQ-1057: the TYPE column names the container a story came out of, and
    /// the TYPE sort must order by the value the column actually shows — a
    /// `Z6`, a `Z6 (ADF)` and a `Z6 (HFS)` may not interleave under the
    /// filename tiebreak.
    #[test]
    fn sort_stories_by_type_groups_by_container_within_a_version() {
        // Filenames run counter to the intended order, so a key that ignored
        // the container would fall back to them and interleave the three.
        let z6 = |title: &str, filename: &str, image: Option<crate::hints::DiskImage>| {
            let mut e = story(title, filename, None, None);
            e.meta.version = Some("6".to_string());
            e.meta.disk_image = image;
            e
        };
        let mut stories = vec![
            z6("hfs", "a.image", Some(crate::hints::DiskImage::Hfs)),
            z6("bare", "z.z6", None),
            blorbed(z6("blorb", "m.zblorb", None)),
            z6("adf", "n.adf", Some(crate::hints::DiskImage::Adf)),
        ];

        // Bare first (no parenthetical at all), then the containers
        // alphabetically and case-insensitively: ADF, blorb, HFS. A raw byte
        // compare would file "blorb" after every acronym.
        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: false });
        assert_eq!(titles_of(&stories), vec!["bare", "adf", "blorb", "hfs"]);

        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: true });
        assert_eq!(titles_of(&stories), vec!["hfs", "blorb", "adf", "bare"]);
    }

    /// Engine and version still dominate the container: a `Z3 (HFS)` is a Z3
    /// first and a Macintosh disk second, and no container promotes a row out
    /// of its version group.
    #[test]
    fn sort_stories_by_type_ranks_engine_and_version_above_the_container() {
        let typed = |title: &str, engine: Engine, version: &str,
                     image: Option<crate::hints::DiskImage>| {
            let mut e = story(title, &format!("{title}.dat"), None, None);
            e.meta.engine = engine;
            e.meta.version = Some(version.to_string());
            e.meta.disk_image = image;
            e
        };
        let mut stories = vec![
            typed("z6-bare", Engine::ZCode, "6", None),
            typed("glulx", Engine::Glulx, "3.1.2", None),
            typed("z3-hfs", Engine::ZCode, "3", Some(crate::hints::DiskImage::Hfs)),
            typed("z6-adf", Engine::ZCode, "6", Some(crate::hints::DiskImage::Adf)),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: false });
        assert_eq!(titles_of(&stories), vec!["z3-hfs", "z6-bare", "z6-adf", "glulx"]);
    }

    /// Glulx shows no container in the TYPE column even when blorbed (SQ-0369),
    /// so the sort may not order Glulx rows by one either — they fall through to
    /// the filename tiebreak, exactly as two identical labels should.
    #[test]
    fn sort_stories_by_type_does_not_order_glulx_by_an_invisible_container() {
        let glulx = |title: &str, filename: &str| {
            let mut e = story(title, filename, None, None);
            e.meta.engine = Engine::Glulx;
            e.meta.version = Some("3.1.2".to_string());
            e
        };
        let mut stories = vec![
            blorbed(glulx("blorbed-z", "z.gblorb")),
            glulx("bare-a", "a.ulx"),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: false });
        assert_eq!(titles_of(&stories), vec!["bare-a", "blorbed-z"]);

        // Ascending vs descending swaps only the filename tiebreak, because the
        // keys themselves are equal.
        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: true });
        assert_eq!(titles_of(&stories), vec!["bare-a", "blorbed-z"]);
    }

    /// The Type key costs a directory read (`sibling_blorb_exists`), so it is
    /// measured ONCE PER ROW before ordering — not inside the comparator, where
    /// it would be O(n log n) directory reads on a list of a few thousand.
    #[test]
    fn sort_stories_by_type_probes_the_filesystem_once_per_row() {
        let mut stories: Vec<StoryEntry> = (0..16)
            .map(|i| {
                let mut e = story(&format!("s{i}"), &format!("s{i}.z5"), None, None);
                e.meta.version = Some("5".to_string());
                e
            })
            .collect();
        TYPE_BLORB_PROBES.with(|c| c.set(0));
        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: false });
        assert_eq!(TYPE_BLORB_PROBES.with(|c| c.get()), stories.len());

        // And a sort on another column pays nothing at all.
        TYPE_BLORB_PROBES.with(|c| c.set(0));
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        assert_eq!(TYPE_BLORB_PROBES.with(|c| c.get()), 0);
    }

    /// The column and the sort read the same rule: whatever
    /// `type_container` answers is exactly the parenthetical the TYPE column
    /// prints, for every engine and every medium. This is the guard against the
    /// two ends drifting apart again — `interp_label` lives in `picker_ui.rs`
    /// and the key lives here.
    #[test]
    fn type_container_is_the_parenthetical_the_type_column_prints() {
        let mut meta = story("s", "s.z5", None, None).meta;

        meta.engine = Engine::ZCode;
        assert_eq!(type_container(&meta, false), None);
        assert_eq!(type_container(&meta, true), Some("blorb"));
        for image in [
            crate::hints::DiskImage::Adf,
            crate::hints::DiskImage::Hfs,
            crate::hints::DiskImage::Fat12Dos,
            crate::hints::DiskImage::ProDos,
        ] {
            meta.disk_image = Some(image);
            // The disk image wins over a blorb sibling, and is the mount's own
            // label rather than a name invented here.
            assert_eq!(type_container(&meta, false), Some(image.label()));
            assert_eq!(type_container(&meta, true), Some(image.label()));
        }
        meta.disk_image = None;

        // Glulx never shows one; Scott shows "blorb" only.
        meta.engine = Engine::Glulx;
        assert_eq!(type_container(&meta, false), None);
        assert_eq!(type_container(&meta, true), None);
        meta.engine = Engine::Scott;
        assert_eq!(type_container(&meta, false), None);
        assert_eq!(type_container(&meta, true), Some("blorb"));
    }

    #[test]
    fn sort_stories_title_ignores_leading_articles() {
        // SQ-0373: bibliographic ordering. "The Lurking Horror" files under L,
        // "A Mind Forever Voyaging" under M — but the full title still displays.
        let mut stories = vec![
            story("The Lurking Horror", "lh.z3", None, None),
            story("A Mind Forever Voyaging", "amfv.z4", None, None),
            story("Bureaucracy", "bur.z3", None, None),
            story("An Act of Murder", "aom.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        assert_eq!(
            titles_of(&stories),
            vec!["An Act of Murder", "Bureaucracy", "The Lurking Horror", "A Mind Forever Voyaging"],
        );
    }

    #[test]
    fn bibliographic_key_strips_only_a_real_leading_article() {
        assert_eq!(super::bibliographic_key("The Lurking Horror"), "lurking horror");
        assert_eq!(super::bibliographic_key("A Mind Forever Voyaging"), "mind forever voyaging");
        assert_eq!(super::bibliographic_key("An Act of Murder"), "act of murder");
        // "Theatre" starts with "the" but isn't the article "the ".
        assert_eq!(super::bibliographic_key("Theatre"), "theatre");
        // A story literally titled "The" keeps it (nothing follows the article).
        assert_eq!(super::bibliographic_key("The"), "the");
    }

    #[test]
    fn sort_stories_title_descending() {
        let mut stories = vec![
            story("Zebra", "z.z5", None, None),
            story("apple", "a.z5", None, None),
            story("Mango", "m.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: true });
        assert_eq!(titles_of(&stories), vec!["Zebra", "Mango", "apple"]);
    }

    #[test]
    fn sort_stories_title_filename_tiebreak() {
        let mut stories = vec![
            story("Same", "b.z5", None, None),
            story("Same", "a.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        let filenames: Vec<&str> = stories.iter().map(|s| s.filename.as_str()).collect();
        assert_eq!(filenames, vec!["a.z5", "b.z5"]);
    }

    #[test]
    fn sort_stories_author_blanks_last_ascending() {
        // A naive sort_by_key on the raw (possibly-empty) string would put the
        // blank author first ("" < "Adams"). It must sort LAST instead.
        let mut stories = vec![
            story("Unfetched", "u.z5", None, None),
            story("Hitchhiker", "h.z5", Some("Adams"), None),
            story("Zork", "z.z5", Some("Blank, Marc"), None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Author, desc: false });
        assert_eq!(titles_of(&stories), vec!["Hitchhiker", "Zork", "Unfetched"]);
    }

    #[test]
    fn sort_stories_author_blanks_last_descending() {
        // Blanks sort last in BOTH directions — descending must not flip the
        // blank entry to the front just because the whole tuple got reversed.
        let mut stories = vec![
            story("Unfetched", "u.z5", None, None),
            story("Hitchhiker", "h.z5", Some("Adams"), None),
            story("Zork", "z.z5", Some("Blank, Marc"), None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Author, desc: true });
        assert_eq!(titles_of(&stories), vec!["Zork", "Hitchhiker", "Unfetched"]);
    }

    #[test]
    fn sort_stories_author_case_insensitive() {
        // Byte order puts capitals before lowercase (all uppercase < any
        // lowercase), so a case-sensitive sort would file "Zarf" ahead of
        // "adam cadre". The list sorts by name, not by ASCII code.
        let mut stories = vec![
            story("Spider", "s.z5", Some("Zarf"), None),
            story("Photopia", "p.z5", Some("adam cadre"), None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Author, desc: false });
        assert_eq!(titles_of(&stories), vec!["Photopia", "Spider"]);
    }

    #[test]
    fn sort_stories_year_numeric_not_lexical() {
        // Lexical comparison would put "1980" after "1998" is fine, but would
        // put "700" before "80" — assert numeric ordering explicitly.
        let mut stories = vec![
            story("B", "b.z5", None, Some("1998")),
            story("A", "a.z5", None, Some("1980")),
            story("C", "c.z5", None, Some("700")),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Year, desc: false });
        assert_eq!(titles_of(&stories), vec!["C", "A", "B"]);
    }

    #[test]
    fn sort_stories_year_blank_and_non_numeric_last_both_directions() {
        let mut stories = vec![
            story("NoYear", "n.z5", None, None),
            story("BadYear", "x.z5", None, Some("circa 1990")),
            story("Old", "o.z5", None, Some("1980")),
            story("New", "y.z5", None, Some("1998")),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Year, desc: false });
        // Blanks/non-numeric sort last; among themselves order is stable per
        // the filename tie-break ("n.z5" < "x.z5").
        assert_eq!(titles_of(&stories), vec!["Old", "New", "NoYear", "BadYear"]);

        sort_stories(&mut stories, Sort { key: SortKey::Year, desc: true });
        assert_eq!(titles_of(&stories), vec!["New", "Old", "NoYear", "BadYear"]);
    }

    /// `story()` plus an IFDB rating — SQ-0529's sort key.
    fn rated(title: &str, filename: &str, rating: Option<f32>, count: Option<u32>) -> StoryEntry {
        let mut e = story(title, filename, None, None);
        e.meta.ifdb_rating = rating;
        e.meta.ifdb_rating_count = count;
        e
    }

    /// Ratings are `f32`, so a naive sort would not even compile against `Ord`;
    /// the key goes through tenths. Check the ordering is numeric, not lexical
    /// ("10" vs "9" is the classic trap even though IFDB caps at 5).
    #[test]
    fn sort_stories_rating_orders_numerically() {
        let mut stories = vec![
            rated("Mid", "m.z5", Some(3.8), Some(226)),
            rated("Best", "b.z5", Some(4.6), Some(50)),
            rated("Worst", "w.z5", Some(1.2), Some(9)),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: false });
        assert_eq!(titles_of(&stories), vec!["Worst", "Mid", "Best"]);

        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: true });
        assert_eq!(titles_of(&stories), vec!["Best", "Mid", "Worst"]);
    }

    /// SPEC (SQ-0529): unrated stories sort LAST — in both directions. A story
    /// that has simply never been fetched is unrated too, and the two are
    /// indistinguishable here by design; neither may masquerade as a 0.0 and
    /// lead the descending list.
    #[test]
    fn sort_stories_rating_unrated_last_both_directions() {
        let mut stories = vec![
            rated("Unfetched", "u.z5", None, None),
            rated("Loved", "l.z5", Some(4.6), Some(50)),
            rated("Panned", "p.z5", Some(1.2), Some(9)),
            rated("Unrated", "z.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: false });
        assert_eq!(titles_of(&stories), vec!["Panned", "Loved", "Unfetched", "Unrated"]);

        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: true });
        assert_eq!(
            titles_of(&stories), vec!["Loved", "Panned", "Unfetched", "Unrated"],
            "descending flips the rated rows only — the unrated tail stays put"
        );
    }

    /// Two identical averages are broken by how many people rated them, so a
    /// 4.6 from 200 voters outranks a 4.6 from three. Without the tiebreak the
    /// pair would fall through to the filename, which is meaningless here.
    #[test]
    fn sort_stories_rating_ties_break_on_the_rating_count() {
        let mut stories = vec![
            rated("Fluke", "a.z5", Some(4.6), Some(3)),
            rated("Classic", "z.z5", Some(4.6), Some(200)),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: true });
        assert_eq!(
            titles_of(&stories), vec!["Classic", "Fluke"],
            "the well-rated 4.6 leads, despite losing the filename tie-break"
        );
    }

    #[test]
    fn sort_stories_default_is_title_ascending() {
        let default = Sort::default();
        assert_eq!(default.key, SortKey::Title);
        assert!(!default.desc);
    }

    // ── resort_preserving_selection: THE highest-value property in the quest ───
    //
    // Selection is an index. Reordering the list under it (a sort-key change,
    // a direction toggle, or a background fetch sweep rewriting titles) must
    // never silently move the cursor to a different story.

    #[test]
    fn resort_preserving_selection_survives_a_sort_key_change() {
        // Chosen so the selected story lands at a DIFFERENT index under the new
        // sort (title-order index 2, author-order index 1) — a naive
        // index-clamping "helper" would silently land on the wrong story here.
        let mut stories = vec![
            story("Anchorhead", "a.z5", Some("Zed"), None),
            story("Curses", "c.z5", Some("Amy"), None),
            story("Zebra", "z.z5", Some("Cara"), None),
        ];
        // Title-ascending: Anchorhead(0), Curses(1), Zebra(2) — select "Zebra".
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        let selected = stories.iter().position(|e| e.title == "Zebra").unwrap();
        assert_eq!(selected, 2);

        // Switch to Author-ascending: Amy(Curses,0), Cara(Zebra,1), Zed(Anchorhead,2).
        let new_idx = resort_preserving_selection(
            &mut stories,
            selected,
            Sort { key: SortKey::Author, desc: false },
        );
        assert_eq!(new_idx, 1, "Zebra must land at its new author-sorted index");
        assert_eq!(stories[new_idx].title, "Zebra", "selection must still point at Zebra");
        assert_eq!(stories[new_idx].path, PathBuf::from("z.z5"));
    }

    #[test]
    fn resort_preserving_selection_survives_a_direction_toggle() {
        // Four items (even count) so reversing genuinely moves every index,
        // including the selected one — with three items the middle entry's
        // index is unchanged by a reversal, which would hide an index-based bug.
        let mut stories = vec![
            story("Anchorhead", "a.z5", None, None),
            story("Bogus", "b.z5", None, None),
            story("Curses", "c.z5", None, None),
            story("Zebra", "z.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        let selected = 0; // "Anchorhead"
        assert_eq!(stories[selected].title, "Anchorhead");

        let new_idx = resort_preserving_selection(
            &mut stories,
            selected,
            Sort { key: SortKey::Title, desc: true },
        );
        assert_eq!(new_idx, 3, "descending reverses the list, moving index 0 to the end");
        assert_eq!(stories[new_idx].title, "Anchorhead");
        assert_eq!(stories[new_idx].path, PathBuf::from("a.z5"));
    }

    #[test]
    fn resort_preserving_selection_survives_a_sweep_rewriting_titles() {
        // Simulates an `r` sweep landing new (fetched) titles mid-flight: the
        // selected story's title changes to something that now sorts
        // elsewhere, while the cursor stays untouched by the user.
        let mut stories = vec![
            story("zork2-r63-s860811", "b.z5", None, None), // stem title, not yet fetched
            story("Anchorhead", "a.z5", None, None),
            story("Curses", "c.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        // Alphabetically: Anchorhead(0), Curses(1), zork2-r63-s860811(2) (case-fold
        // puts the lowercase stem after the capitalized titles).
        let selected = stories.iter().position(|e| e.path == *"b.z5").unwrap();
        assert_eq!(selected, 2);

        // The sweep just fetched this story's real title — one that now sorts
        // FIRST, so a naive index-clamp would land on the wrong (unrelated) story.
        stories[selected].title = "AAA Zork II".to_string();

        let new_idx = resort_preserving_selection(&mut stories, selected, Sort::default());
        assert_eq!(new_idx, 0, "the rewritten title now sorts first");
        assert_eq!(stories[new_idx].path, PathBuf::from("b.z5"), "selection follows the story by path");
        assert_eq!(stories[new_idx].title, "AAA Zork II");
    }

    #[test]
    fn resort_preserving_selection_defaults_to_zero_when_the_story_is_gone() {
        let mut stories = vec![story("Anchorhead", "a.z5", None, None)];
        let new_idx = resort_preserving_selection(&mut stories, 5, Sort::default());
        assert_eq!(new_idx, 0);
    }

    #[test]
    fn z_header_helpers_parse_version_release_serial_flags() {
        let mut b = minimal_v3_story();
        b[0x00] = 3;                       // version
        b[0x02] = 0x00; b[0x03] = 0x58;    // release 88
        b[0x12..0x18].copy_from_slice(b"840726");
        b[0x10] = 0x00; b[0x11] = 0x08 | 0x40 | 0x80; // flags2: graphics|colour|sound

        assert_eq!(z_version(&b), Some(3));
        assert_eq!(z_release(&b), Some(88));
        assert_eq!(z_serial(&b).as_deref(), Some("840726"));
        let f2 = z_flags2(&b);
        assert!(f2 & 0x0008 != 0, "graphics bit");
        assert!(f2 & 0x0040 != 0, "colour bit");
        assert!(f2 & 0x0080 != 0, "sound bit");
    }

    #[test]
    fn glulx_version_formats_major_minor_subminor() {
        let mut b = vec![0u8; 0x40];
        b[0x00..0x04].copy_from_slice(b"Glul");
        b[0x04] = 0x00; b[0x05] = 0x03;    // major = 3
        b[0x06] = 0x01;                    // minor = 1
        b[0x07] = 0x02;                    // subminor = 2
        assert_eq!(glulx_version(&b).as_deref(), Some("3.1.2"));
    }

    #[test]
    fn scan_populates_story_meta_for_v3() {
        let dir = temp_dir("meta");
        let mut b = minimal_v3_story();
        b[0x02] = 0x00; b[0x03] = 0x58;                 // release 88
        b[0x12..0x18].copy_from_slice(b"840726");
        b[0x10] = 0x00; b[0x11] = 0x40;                 // colour bit set
        std::fs::write(dir.join("game.z3"), &b).unwrap();

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1);
        let m = &stories[0].meta;
        assert_eq!(m.engine, Engine::ZCode);
        assert_eq!(m.format, "Z-code");
        assert_eq!(m.version.as_deref(), Some("3"));
        assert_eq!(m.release, Some(88));
        assert_eq!(m.serial.as_deref(), Some("840726"));
        assert_eq!(m.features.colour, Some(true));
        assert!(m.size_bytes > 0);
        assert_eq!(m.story_bytes, m.size_bytes, "a bare story file IS its story");
        assert!(m.self_blorb.is_none());
    }

    /// A self-contained blorb around a story: `Blorb` FORM wrapper + resource
    /// index + one `ZCOD` Exec chunk. Deliberately larger than the story it
    /// holds, so `story_bytes` and `size_bytes` cannot coincide.
    fn blorb_with_exec(story: &[u8]) -> Vec<u8> {
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let ridx_data_len = 4 + 12;
        let exec_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes());
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes());
        ridx.extend_from_slice(&(exec_off as u32).to_be_bytes());
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&chunk(b"ZCOD", story));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    /// SQ-0771: a container's byte length is the container's, never the game's.
    /// `size_bytes` measures the file on disk and `story_bytes` measures what
    /// was mounted out of it — for a blorb (and for a zip, and for the `.adf`
    /// the bug was reported on) those are different numbers.
    #[test]
    fn a_container_reports_the_mounted_storys_size_beside_its_own() {
        let dir = temp_dir("story-bytes");
        let story = minimal_v3_story();
        std::fs::write(dir.join("bare.z3"), &story).unwrap();
        std::fs::write(dir.join("wrapped.zblorb"), blorb_with_exec(&story)).unwrap();

        let bare = resolve_entry(&dir.join("bare.z3"), &dir).expect("bare story resolves");
        let blorb = resolve_entry(&dir.join("wrapped.zblorb"), &dir).expect("blorb resolves");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(bare.meta.story_bytes, story.len() as u64);
        assert_eq!(bare.meta.size_bytes, story.len() as u64);
        // The container is bigger than the game, and it is the game the field
        // has to report.
        assert_eq!(blorb.meta.story_bytes, story.len() as u64, "the mounted story's size");
        assert!(
            blorb.meta.size_bytes > blorb.meta.story_bytes,
            "the blorb file is larger than its Exec chunk: {} vs {}",
            blorb.meta.size_bytes,
            blorb.meta.story_bytes
        );
    }

    /// SQ-0767: a `zork1inv.z5` sitting beside a story whose *file* is named for
    /// the box is that story's InvisiClues, and only the mounted story's release
    /// and serial can say so. Fixture-free — the header carries Zork I release
    /// 88 / serial 840726, which is the whole of the identity.
    #[test]
    fn a_sidecar_is_associated_with_a_box_named_story_by_identity() {
        let dir = temp_dir("sidecar-identity");
        let mut zork1 = minimal_v3_story();
        zork1[0x02] = 0x00;
        zork1[0x03] = 88; // release 88
        zork1[0x12..0x18].copy_from_slice(b"840726");
        std::fs::write(dir.join("Zork I - The Great Underground Empire.z3"), &zork1).unwrap();
        std::fs::write(dir.join("zork1inv.z5"), minimal_v3_story()).unwrap();

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1, "the sidecar is hidden once associated: {stories:?}");
        let game = &stories[0];
        assert!(
            !crate::hints::normalize_ident(&game.filename).contains("zork1"),
            "the premise: the name says nothing"
        );
        let sidecar = game.hint_sidecar.as_ref().expect("the clues file is associated by identity");
        assert!(sidecar.ends_with("zork1inv.z5"));
    }

    /// End to end on real media (skips vacuously — `stories/` is gitignored):
    /// the Zork floppies' hint badge lights, which is the surface the defect was
    /// reported on (SQ-0767). Their containers are named for the box, so the
    /// badge can only come from the mounted story's identity.
    #[test]
    fn a_real_disk_image_lights_the_downloadable_hint_badge() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
        let base = std::env::temp_dir().join(format!("lanthorn-adf-hint-{}", std::process::id()));
        let index = hints::load_hint_index(&base);
        for name in [
            "Zork I - The Great Underground Empire.adf",
            "Zork II - The Wizard of Frobozz.adf",
            "Zork III - The Dungeon Master.adf",
            "Zork - The Undiscovered Underground.adf",
            "Zork Zero - The Revenge of Megaboz.adf",
        ] {
            let path = dir.join(name);
            if !path.is_file() {
                continue; // no story media here — skip
            }
            let entry = resolve_entry(&path, &base).expect("the floppy mounts and is launchable");
            assert_eq!(
                compute_row_badges(&entry, &base, &index).hint,
                HintBadge::Available,
                "{name} (IFID {}): pre-fix the container's name matched no catalog key",
                entry.meta.ifid
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// End to end on real media (skips vacuously — `stories/` is gitignored):
    /// every Amiga floppy in the story directory is the same 880 KB whatever it
    /// holds, so its container length says nothing about the game; `story_bytes`
    /// must be the mounted image's own length (SQ-0771).
    #[test]
    fn a_real_disk_image_reports_the_mounted_storys_size() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return; // no story media here — skip
        };
        let data_base =
            std::env::temp_dir().join(format!("lanthorn-adf-size-{}", std::process::id()));
        let mut saw_adf = false;
        for e in entries.flatten() {
            let path = e.path();
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            if ext != "adf" {
                continue;
            }
            let Some(entry) = resolve_entry(&path, &data_base) else {
                continue; // not launchable — the picker wouldn't list it either
            };
            saw_adf = true;
            let mounted = crate::hints::load_story(&path).expect("the floppy mounts").into_bytes();
            assert_eq!(
                entry.meta.story_bytes,
                mounted.len() as u64,
                "{}: story_bytes is the mounted image's length",
                path.display()
            );
            assert!(
                entry.meta.story_bytes < entry.meta.size_bytes,
                "{}: pre-fix this reported the {}-byte floppy as the story's size",
                path.display(),
                entry.meta.size_bytes
            );
        }
        let _ = std::fs::remove_dir_all(&data_base);
        let _ = saw_adf; // no `.adf` present is a vacuous skip, not a failure
    }

    // ── `resolve` precedence (pure, no filesystem) ─────────────────────────

    use crate::ifiction::IFiction;
    use crate::story_info::FetchedMeta;

    /// A fetch that ran to completion but found nothing worth reporting: every
    /// field absent, `not_found: false` (callers override per-test).
    fn fetched_stub() -> FetchedMeta {
        FetchedMeta {
            scanned_at: "2026-07-16T00:00:00Z".into(),
            fetch_version: crate::story_info::FETCH_VERSION,
            source: "ifdb".into(),
            title: None,
            author: None,
            language: None,
            first_published: None,
            genre: None,
            description: None,
            ifdb_tuid: None,
            ifdb_link: None,
            ifdb_rating: None,
            ifdb_rating_count: None,
            cover: None,
            not_found: false,
        }
    }

    /// SPEC "Precedence". Resolution happens ONCE, here — everything downstream
    /// reads plain fields and never asks where a value came from.
    #[test]
    fn ifmd_outranks_a_fetched_sidecar_field_by_field() {
        let ifmd = IFiction { title: Some("From IFmd".into()), author: None, ..Default::default() };
        let fetched = FetchedMeta { title: Some("From IFDB".into()), author: Some("From IFDB".into()), ..fetched_stub() };
        let r = resolve(Some(&ifmd), Some(&fetched), None, None, None, "stem");
        assert_eq!(r.title, "From IFmd", "the file's own metadata wins");
        assert_eq!(r.author.as_deref(), Some("From IFDB"), "but IFDB fills the gap IFmd left");
    }

    #[test]
    fn tsv_then_stem_when_nothing_else_has_a_title() {
        assert_eq!(resolve(None, None, Some("From TSV"), None, None, "stem").title, "From TSV");
        assert_eq!(resolve(None, None, None, None, None, "stem").title, "stem");
    }

    #[test]
    fn a_not_found_block_contributes_nothing_but_is_not_an_error() {
        let nf = FetchedMeta { not_found: true, title: None, ..fetched_stub() };
        assert_eq!(
            resolve(None, Some(&nf), Some("From TSV"), None, None, "stem").title,
            "From TSV"
        );
    }

    #[test]
    fn tsv_author_and_description_fill_gaps_but_a_fetch_still_wins() {
        // Homebrew Scott games have only the bundled TSV author/description.
        let r = resolve(None, None, Some("Marooned"), Some("Kim Watt"), Some("A desc."), "stem");
        assert_eq!(r.author.as_deref(), Some("Kim Watt"));
        assert_eq!(r.description.as_deref(), Some("A desc."));
        // A real IFDB fetch outranks the TSV fallback, field by field.
        let fetched = FetchedMeta {
            author: Some("From IFDB".into()),
            description: Some("From IFDB".into()),
            ..fetched_stub()
        };
        let r = resolve(None, Some(&fetched), None, Some("Kim Watt"), Some("A desc."), "stem");
        assert_eq!(r.author.as_deref(), Some("From IFDB"));
        assert_eq!(r.description.as_deref(), Some("From IFDB"));
    }

    /// The rating is IFDB-only: a blorb's IFmd chunk has no equivalent, so it
    /// comes from a fetched block or not at all (SQ-0529). A story with rich
    /// local metadata and no sidecar still has no rating — and the resolver
    /// must leave it None rather than default it.
    #[test]
    fn the_ifdb_rating_comes_only_from_a_fetched_block() {
        let ifmd = IFiction { title: Some("Local".into()), ..Default::default() };
        let r = resolve(Some(&ifmd), None, None, None, None, "stem");
        assert_eq!(r.ifdb_rating, None, "an IFmd chunk carries no community rating");
        assert_eq!(r.ifdb_rating_count, None);

        let fetched = FetchedMeta {
            ifdb_rating: Some(3.818_584),
            ifdb_rating_count: Some(226),
            ..fetched_stub()
        };
        let r = resolve(Some(&ifmd), Some(&fetched), None, None, None, "stem");
        assert_eq!(r.ifdb_rating, Some(3.818_584), "IFmd wins the title but has no rating to win");
        assert_eq!(r.ifdb_rating_count, Some(226));
    }

    #[test]
    fn leading_year_takes_the_year_from_a_bare_or_iso_firstpublished() {
        assert_eq!(leading_year("1984"), Some("1984".to_string()));
        // iFiction allows a full ISO date; the YEAR column and numeric sort
        // want just the year, not "1984-06-01".
        assert_eq!(leading_year("1984-06-01"), Some("1984".to_string()));
        assert_eq!(leading_year("  1980 "), Some("1980".to_string()));
        // Nothing usable → dropped, so it sorts/displays as "unknown", not "0".
        assert_eq!(leading_year("forthcoming"), None);
        assert_eq!(leading_year("198"), None, "a 3-digit lead is not a year");
    }

    // ── `scan_stories` integration: sidecar resolution end-to-end ──────────

    #[test]
    fn scan_resolves_title_from_a_fetched_sidecar() {
        let dir = temp_dir("sidecar-fetched");
        let bytes = minimal_v3_story();
        std::fs::write(dir.join("game.z5"), &bytes).unwrap();
        let ifid = crate::ifid::compute_ifid(&bytes);

        let data_base = dir.join("data");
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&dir.join("game.z5")));
        let info = crate::story_info::StoryInfo {
            format_version: crate::story_info::FORMAT_VERSION,
            ifid: ifid.clone(),
            fetched: Some(FetchedMeta { title: Some("Fetched Title".into()), ..fetched_stub() }),
            probe: None,
        };
        crate::story_info::save(&game_dir, &info).unwrap();

        let stories = scan_stories(&dir, &data_base);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].title, "Fetched Title");
    }

    #[test]
    fn scan_falls_back_past_a_wrong_ifid_sidecar() {
        let dir = temp_dir("sidecar-wrong-ifid");
        let bytes = minimal_v3_story();
        std::fs::write(dir.join("game.z5"), &bytes).unwrap();

        let data_base = dir.join("data");
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&dir.join("game.z5")));
        let info = crate::story_info::StoryInfo {
            format_version: crate::story_info::FORMAT_VERSION,
            ifid: "WRONG-IFID".into(), // doesn't match the story's real IFID
            fetched: Some(FetchedMeta { title: Some("Should Not Appear".into()), ..fetched_stub() }),
            probe: None,
        };
        crate::story_info::save(&game_dir, &info).unwrap();

        let stories = scan_stories(&dir, &data_base);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].title, "game", "wrong-IFID sidecar ignored entirely; falls to the stem");
    }

    // Build a StoryEntry with a controllable ifid + self_blorb, on a synthetic path.
    fn entry_with(ifid: &str, path: PathBuf, self_blorb: Option<Vec<ChunkInfo>>) -> StoryEntry {
        StoryEntry {
            path,
            title: "T".into(),
            filename: "t.z5".into(),
            meta: StoryMeta {
                size_bytes: 1, story_bytes: 1, modified: None, engine: Engine::ZCode,
                format: "Z-code".into(), version: Some("5".into()),
                serial: None, release: None, ifid: ifid.into(),
                features: Features::default(), self_blorb, disk_image: None, disk_entry: None,
                author: None, year: None, genre: None, language: None, description: None,
                ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None,
                fetch_not_found: false,
            },
            hint_sidecar: None,
            kind: RowKind::Story,
        }
    }

    #[test]
    fn scan_associates_and_hides_hint_sidecar() {
        let dir = temp_dir("hint-sidecar");
        std::fs::write(dir.join("zork1.z3"), minimal_v3_story()).unwrap();
        std::fs::write(dir.join("zork1_hints.z5"), minimal_v3_story()).unwrap();

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        // (a) the game is listed; (b) the sidecar is NOT listed.
        assert_eq!(stories.len(), 1, "only the game is listed, sidecar hidden");
        assert_eq!(stories[0].filename, "zork1.z3");
        // (c) the game entry points at the hidden sidecar file.
        assert_eq!(
            stories[0].hint_sidecar.as_deref(),
            Some(dir.join("zork1_hints.z5").as_path())
        );
    }

    #[test]
    fn scan_keeps_a_lone_hint_sidecar_listed() {
        // A hint sidecar with no matching game is not orphaned — it stays listed.
        let dir = temp_dir("lone-sidecar");
        std::fs::write(dir.join("deadlineinv.z5"), minimal_v3_story()).unwrap();

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1, "lone sidecar stays listed");
        assert_eq!(stories[0].filename, "deadlineinv.z5");
        assert!(stories[0].hint_sidecar.is_none());
    }

    #[test]
    fn scan_does_not_hide_a_solid_gold_game() {
        // A Solid Gold `*-invclues-rNN-sNNN.z5` carries a release/serial, so it is
        // NOT a hint sidecar and must stay listed as a normal game.
        let dir = temp_dir("solid-gold");
        std::fs::write(dir.join("zork1-invclues-r52-s871125.z5"), minimal_v3_story()).unwrap();

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1, "Solid Gold game is not dropped");
        assert_eq!(stories[0].filename, "zork1-invclues-r52-s871125.z5");
        assert!(stories[0].hint_sidecar.is_none());
    }

    #[test]
    fn compute_row_badges_lights_hint_from_sidecar() {
        // With an empty index, a detected sidecar alone lights the hint badge.
        let dir = temp_dir("badge-sidecar");
        let mut e = entry_with("IFID-H", dir.join("zork1.z3"), None);
        e.hint_sidecar = Some(dir.join("zork1_hints.z5"));
        let base = dir.join("data");
        let hi = hints::load_hint_index(&dir); // empty index

        let b = compute_row_badges(&e, &base, &hi);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(b.hint, HintBadge::Present, "sidecar presence lights the present-hint badge with an empty index");
    }

    #[test]
    fn compute_row_badges_covers_each_signal() {
        let dir = temp_dir("badges");
        // A self-blorb story lights `blorb` with no sibling.
        let e_self = entry_with("IFID-A", dir.join("a.z5"),
            Some(vec![ChunkInfo { usage: "Exec".into(), number: 0, chunk_type: "ZCOD".into(), len: 4, detail: None }]));
        // A story with a same-stem sibling .blorb lights `blorb`.
        std::fs::write(dir.join("b.z5"), b"x").unwrap();
        std::fs::write(dir.join("b.blorb"), b"x").unwrap();
        let e_sibling = entry_with("IFID-B", dir.join("b.z5"), None);
        // A plain story with nothing.
        let e_bare = entry_with("IFID-C", dir.join("c.z5"), None);

        // Storage base with per-game dirs keyed by story filename (SQ-0284):
        // A has a default Save State, B a named `.qzl` game save, C nothing.
        let base = dir.join("data");
        let a_dir = crate::storage::game_dir(&base, &crate::storage::story_key_at(&dir.join("a.z5")));
        let b_dir = crate::storage::game_dir(&base, &crate::storage::story_key_at(&dir.join("b.z5")));
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        std::fs::write(a_dir.join("default.lanthorn"), b"x").unwrap();
        std::fs::write(b_dir.join("before.qzl"), b"x").unwrap();

        let hi = hints::load_hint_index(&dir); // empty index (no hints/index.toml)

        let a = compute_row_badges(&e_self, &base, &hi);
        let b = compute_row_badges(&e_sibling, &base, &hi);
        let c = compute_row_badges(&e_bare, &base, &hi);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!((a.blorb, a.save, a.hint), (true, true, HintBadge::None));
        assert_eq!((b.blorb, b.save, b.hint), (true, true, HintBadge::None));
        assert_eq!((c.blorb, c.save, c.hint), (false, false, HintBadge::None));
    }

    /// A game with no local hint but a matching downloadable InvisiClues lights
    /// the lowercase available-hint badge; a game with neither stays None.
    #[test]
    fn compute_row_badges_marks_downloadable_hint_available() {
        let dir = temp_dir("badge-available");
        let base = dir.join("data");
        let hi = hints::load_hint_index(&dir); // empty index

        // "deadline" matches the SLAG catalog → Available (no local file).
        let e_dl = entry_with("IFID-DL", dir.join("deadline.z3"), None);
        assert_eq!(compute_row_badges(&e_dl, &base, &hi).hint, HintBadge::Available);

        // A game no catalog covers stays None.
        let e_none = entry_with("IFID-N", dir.join("colossal.z5"), None);
        assert_eq!(compute_row_badges(&e_none, &base, &hi).hint, HintBadge::None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Minimal blorb with one Snd resource so resolve_resource_blorb accepts a sibling.
    fn blorb_with_sound() -> Vec<u8> {
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 { v.push(0); }
            v
        }
        let ridx_data_len = 4 + 12;
        let snd_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes());
        ridx.extend_from_slice(b"Snd ");
        ridx.extend_from_slice(&0u32.to_be_bytes());
        ridx.extend_from_slice(&(snd_off as u32).to_be_bytes());
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&chunk(b"OGGV", b"snd"));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    #[test]
    fn resolve_aux_finds_sibling_blorb_and_saves() {
        let dir = temp_dir("aux");
        std::fs::write(dir.join("g.z5"), minimal_v3_story()).unwrap();
        std::fs::write(dir.join("g.blb"), blorb_with_sound()).unwrap();
        let entry = entry_with("IFID-G", dir.join("g.z5"), None);

        let hi = hints::load_hint_index(&dir);
        let aux = resolve_aux(&entry, &dir, &hi); // data_base=dir (no per-game saves)
        let _ = std::fs::remove_dir_all(&dir);

        let (src, chunks) = aux.assoc_blorb.expect("sibling blorb resolved");
        assert!(src.ends_with("g.blb"));
        assert!(chunks.iter().any(|c| c.usage == "Snd "));
        assert!(aux.saves.is_empty());
        assert!(!aux.hints_available);
    }

    #[test]
    fn resolve_aux_reports_game_dir_qzl_saves_and_sidecars() {
        let dir = temp_dir("aux-qzl");
        std::fs::write(dir.join("g.z5"), minimal_v3_story()).unwrap();
        let entry = entry_with("IFID-G", dir.join("g.z5"), None);

        // A separate data base so the per-game dir doesn't collide with the
        // story file itself (SQ-0284 keys by filename).
        let base = dir.join("data");
        let game_dir = crate::storage::game_dir(&base, &crate::storage::story_key_at(&entry.path));
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("default.lanthorn"), b"x").unwrap();
        std::fs::write(game_dir.join("quick.qzl"), b"x").unwrap();
        std::fs::write(game_dir.join("_startup.qzl"), b"x").unwrap();
        std::fs::write(game_dir.join("default.aux"), b"x").unwrap();

        let hi = hints::load_hint_index(&dir);
        let aux = resolve_aux(&entry, &base, &hi);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(aux.game_dir, game_dir);
        // default.lanthorn has no valid archive, so list_saves skips it here.
        assert_eq!(aux.saves.len(), 0, "notanarchive default.lanthorn is skipped by list_saves");
        assert_eq!(aux.qzl_saves.len(), 1);
        assert_eq!(aux.qzl_saves[0].name, "quick");
        assert!(!aux.qzl_saves.iter().any(|s| s.name == "_startup"), "auto save excluded from player list");
        assert_eq!(aux.auto_saves.len(), 1, "auto_saves carries the game-managed underscore save");
        assert_eq!(aux.auto_saves[0].name, "_startup");
        assert_eq!(aux.sidecars, vec!["default.aux"]);
    }

    // ── Resource format-detail parsing ──────────────────────────────────

    /// Encode `rate` as an IEEE 80-bit extended-precision float, the inverse
    /// of `extended80_to_u32`, for building AIFF `COMM` fixtures.
    fn encode_extended80(rate: u32) -> [u8; 10] {
        let bits = 32 - rate.leading_zeros(); // significant bits in `rate`
        let exp = 16383 + (bits as i32 - 1);
        let mantissa = (rate as u64) << (63 - (bits - 1));
        let mut out = [0u8; 10];
        out[0] = (exp >> 8) as u8;
        out[1] = exp as u8;
        out[2..10].copy_from_slice(&mantissa.to_be_bytes());
        out
    }

    /// Build a minimal AIFF `Snd ` payload (post-FORM-header, as blorb stores
    /// it): form type + one `COMM` subchunk.
    fn aiff_fixture(channels: u16, sample_size: u16, num_frames: u32, rate: u32) -> Vec<u8> {
        let mut comm = Vec::new();
        comm.extend_from_slice(&channels.to_be_bytes());
        comm.extend_from_slice(&num_frames.to_be_bytes());
        comm.extend_from_slice(&sample_size.to_be_bytes());
        comm.extend_from_slice(&encode_extended80(rate));
        let mut data = b"AIFF".to_vec();
        data.extend_from_slice(b"COMM");
        data.extend_from_slice(&(comm.len() as u32).to_be_bytes());
        data.extend_from_slice(&comm);
        data
    }

    #[test]
    fn aiff_sound_detail_parses_rate_bit_depth_and_channels() {
        let data = aiff_fixture(1, 8, 16000, 8000);
        let detail = sound_detail(b"FORM", &data).expect("valid AIFF COMM parses");
        assert!(detail.contains("8.0 kHz"), "{detail:?}");
        assert!(detail.contains("8-bit"), "{detail:?}");
        assert!(detail.contains("mono"), "{detail:?}");
        assert!(detail.contains("2.0s"), "{detail:?}");
    }

    #[test]
    fn aiff_sound_detail_rejects_garbage() {
        assert_eq!(sound_detail(b"FORM", b"not aiff at all"), None);
        assert_eq!(sound_detail(b"FORM", b"AIFF"), None); // no COMM subchunk
        assert_eq!(sound_detail(b"FORM", &[]), None);
    }

    #[test]
    fn ogg_sound_detail_parses_rate_and_channels() {
        let mut data = b"OggS".to_vec();
        data.extend_from_slice(&[0u8; 20]); // leading page-header padding
        data.extend_from_slice(b"\x01vorbis");
        data.extend_from_slice(&[0u8; 4]); // vorbis_version (unused)
        data.push(2); // channels: stereo
        data.extend_from_slice(&44_100u32.to_le_bytes());
        let detail = sound_detail(b"OGGV", &data).expect("valid Ogg Vorbis header parses");
        assert!(detail.contains("44.1 kHz"), "{detail:?}");
        assert!(detail.contains("stereo"), "{detail:?}");
    }

    #[test]
    fn ogg_sound_detail_rejects_garbage() {
        assert_eq!(sound_detail(b"OGGV", b"not ogg"), None);
        assert_eq!(sound_detail(b"OGGV", b"OggS"), None); // no vorbis ident packet
    }

    #[test]
    fn mod_sound_detail_reads_channel_tag() {
        let mut data = vec![0u8; 1084];
        data[1080..1084].copy_from_slice(b"M.K.");
        assert_eq!(sound_detail(b"MOD ", &data).as_deref(), Some("4ch"));

        let mut data6 = vec![0u8; 1084];
        data6[1080..1084].copy_from_slice(b"6CHN");
        assert_eq!(sound_detail(b"MOD ", &data6).as_deref(), Some("6ch"));
    }

    #[test]
    fn mod_sound_detail_rejects_too_short() {
        assert_eq!(sound_detail(b"MOD ", &[0u8; 100]), None);
    }

    #[test]
    fn png_image_detail_parses_dimensions_and_bpp() {
        let mut data = vec![0u8; 26];
        data[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        data[8..12].copy_from_slice(&13u32.to_be_bytes()); // IHDR length
        data[12..16].copy_from_slice(b"IHDR");
        data[16..20].copy_from_slice(&800u32.to_be_bytes()); // width
        data[20..24].copy_from_slice(&600u32.to_be_bytes()); // height
        data[24] = 8; // bit depth
        data[25] = 6; // color type: RGBA → 4 channels
        let detail = image_detail(b"PNG ", &data).expect("valid PNG IHDR parses");
        assert!(detail.contains("800×600"), "{detail:?}");
        assert!(detail.contains("32bpp"), "{detail:?}");
    }

    #[test]
    fn png_image_detail_rejects_truncated() {
        assert_eq!(image_detail(b"PNG ", b"\x89PNG\r\n\x1a\n"), None); // signature only
        assert_eq!(image_detail(b"PNG ", b"not a png"), None);
    }

    #[test]
    fn jpeg_image_detail_parses_dimensions_and_components() {
        let mut data = vec![0xFFu8, 0xD8, 0xFF, 0xC0]; // SOI, SOF0 marker
        data.extend_from_slice(&17u16.to_be_bytes()); // segment length
        data.push(8); // precision
        data.extend_from_slice(&100u16.to_be_bytes()); // height
        data.extend_from_slice(&200u16.to_be_bytes()); // width
        data.push(3); // components
        data.extend_from_slice(&[0u8; 9]); // 3 components × 3 bytes each
        let detail = image_detail(b"JPEG", &data).expect("valid JPEG SOF0 parses");
        assert!(detail.contains("200×100"), "{detail:?}");
        assert!(detail.contains("8-bit"), "{detail:?}");
        assert!(detail.contains("3ch"), "{detail:?}");
    }

    #[test]
    fn jpeg_image_detail_rejects_garbage() {
        assert_eq!(image_detail(b"JPEG", b"not a jpeg"), None);
        assert_eq!(image_detail(b"JPEG", &[0xFF, 0xD8]), None); // SOI only, no SOF
    }

    #[test]
    fn resource_detail_dispatches_by_usage_none_for_unknown() {
        let png = {
            let mut data = vec![0u8; 26];
            data[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
            data[16..20].copy_from_slice(&1u32.to_be_bytes());
            data[20..24].copy_from_slice(&1u32.to_be_bytes());
            data[24] = 8;
            data[25] = 2;
            data
        };
        assert!(resource_detail(b"Pict", b"PNG ", &png).is_some());
        assert_eq!(resource_detail(b"Data", b"PNG ", &png), None);
        assert_eq!(resource_detail(b"Exec", b"ZCOD", b"whatever"), None);
    }

    #[test]
    fn scott_title_lookup_by_filename() {
        assert_eq!(scott_title("adv01"), Some("Adventureland"));
        assert_eq!(scott_title("adv13"), Some("The Sorcerer of Claymorgue Castle"));
        // Distinct games that share the "14" number resolve by filename.
        assert_eq!(scott_title("adv14a"), Some("Return to Pirate's Isle"));
        assert_eq!(scott_title("adv14b"), Some("Buckaroo Banzai"));
        // Howarth's Mysterious Adventures reuse numbers 1-11 but key by name.
        assert_eq!(scott_title("1_baton"), Some("The Golden Baton"));
        assert_eq!(scott_title("b_waxworks"), Some("Waxworks"));
        // Lookup is case-insensitive (the readme uses uppercase stems).
        assert_eq!(scott_title("ADV01"), Some("Adventureland"));
        assert_eq!(scott_title("nope"), None);

        // scott_story_title keys off the path's filename stem.
        assert_eq!(scott_story_title(Path::new("adv01.dat")).as_deref(), Some("Adventureland"));
        assert_eq!(scott_story_title(Path::new("quest1.dat")).as_deref(), Some("The Hulk"));
        // Unknown filename -> None (caller falls back to the filename stem).
        assert_eq!(scott_story_title(Path::new("mygame.dat")), None);

        // Homebrew games carry a bundled author + description; IFDB games and
        // unknown stems have neither.
        assert_eq!(scott_author("marooned"), Some("Kim Watt"));
        assert_eq!(scott_description("miner"), Some("Collect four lost treasures in a mine."));
        assert_eq!(scott_author("bond"), None); // author genuinely unknown
        assert!(scott_description("bond").is_some());
        assert_eq!(scott_author("adv01"), None);
        assert_eq!(scott_description("adv01"), None);
        assert_eq!(scott_author("nope"), None);
    }

    #[test]
    fn scott_tuid_lookup_where_known() {
        assert_eq!(scott_tuid("adv01"), Some("dy4ok8sdlut6ddj7")); // Adventureland
        assert_eq!(scott_tuid("adv13"), Some("11tnb08k1jov4hyl")); // Sorcerer of Claymorgue
        assert_eq!(scott_tuid("quest1"), Some("4blbm63qfki4kf2p")); // The Hulk (Questprobe)
        // The `.dat` and graphics `.blb` repackaging of a Mysterious Adventure
        // are the same game, so they share one IFDB id.
        assert_eq!(scott_tuid("1_baton"), Some("v148gq1vx7leo8al"));
        assert_eq!(scott_tuid("golden_baton"), Some("v148gq1vx7leo8al"));
        // The sampler and the homebrew games have a title but no IFDB entry.
        assert_eq!(scott_title("sampler1"), Some("Adventureland (Sampler)"));
        assert_eq!(scott_tuid("sampler1"), None);
        assert_eq!(scott_title("bond"), Some("James Bond Adventure"));
        assert_eq!(scott_tuid("bond"), None);
        assert_eq!(scott_tuid("nope"), None);
        // Rows not known to lack an IFDB entry carry both a title and a TUID.
        const NO_TUID: &[&str] = &[
            "sampler1", "miner", "bond", "burglar", "romulan", "secret", "gamma", "marooned",
            "conquest",
        ];
        for (stem, entry) in scott_titles() {
            assert!(!entry.title.is_empty(), "title for {stem}");
            if !NO_TUID.contains(stem) {
                assert!(entry.tuid.is_some(), "IFDB id for {stem}");
            }
        }
    }

    #[test]
    fn scott_titles_file_parses_without_dupes() {
        let table = scott_titles();
        let lines = include_str!("scott_titles.tsv")
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .count();
        assert_eq!(lines, table.len(), "no duplicate filename stems in scott_titles.tsv");
    }

    // ── Folders and the in-memory find ────────────────────────────────────────

    /// Builds `root/{a,b}/…` with one story in each level plus a dot-directory
    /// and a non-story file, so every scan below has something to skip.
    fn nested_library(tag: &str) -> PathBuf {
        let root = temp_dir(tag);
        std::fs::create_dir_all(root.join("zcode/german")).unwrap();
        std::fs::create_dir_all(root.join("Glulx")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("top.z5"), minimal_v3_story()).unwrap();
        std::fs::write(root.join("zcode/curses.z5"), minimal_v3_story()).unwrap();
        std::fs::write(root.join("zcode/german/burg.z5"), minimal_v3_story()).unwrap();
        std::fs::write(root.join("Glulx/notes.txt"), b"not a story").unwrap();
        std::fs::write(root.join(".hidden/secret.z5"), minimal_v3_story()).unwrap();
        root
    }

    #[test]
    fn folders_list_before_stories_and_dot_directories_are_skipped() {
        let root = nested_library("folders");
        let rows = library_rows(&root, &root, &root);
        let _ = std::fs::remove_dir_all(&root);

        let labels: Vec<&str> = rows.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(labels, vec!["Glulx/", "zcode/", "top"], "folders first, by name, case-insensitively; no `..` at the root; no dot-dir");
        assert!(rows[0].is_folder() && rows[1].is_folder() && !rows[2].is_folder());
        assert_eq!(rows[1].path, root.join("zcode"), "a folder row's path is the directory itself");
    }

    #[test]
    fn library_rows_offer_the_parent_only_below_the_root() {
        let root = nested_library("parent");
        let rows = library_rows(&root.join("zcode"), &root, &root);
        let _ = std::fs::remove_dir_all(&root);

        let labels: Vec<&str> = rows.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(labels, vec![PARENT_LABEL, "german/", "curses"]);
        assert_eq!(rows[0].path, root, "`..` leads to the directory above");
    }

    /// Whatever column the stories are sorted on, and in either direction, the
    /// way out stays at the top: `..`, then folders by name, then the stories.
    #[test]
    fn sort_stories_keeps_folders_on_top_under_every_key_and_direction() {
        let mut rows = vec![
            story("Zork", "zork.z5", Some("Infocom"), Some("1980")),
            StoryEntry::folder(PathBuf::from("/lib/b"), "b/"),
            story("Advent", "advent.z5", None, None),
            StoryEntry::folder(PathBuf::from("/lib"), PARENT_LABEL),
            StoryEntry::folder(PathBuf::from("/lib/A"), "A/"),
        ];
        for key in [SortKey::Title, SortKey::Author, SortKey::Year, SortKey::Rating, SortKey::Type] {
            for desc in [false, true] {
                sort_stories(&mut rows, Sort { key, desc });
                let labels: Vec<&str> = rows.iter().take(3).map(|e| e.title.as_str()).collect();
                assert_eq!(labels, vec![PARENT_LABEL, "A/", "b/"], "{key:?} desc={desc}");
                assert!(rows[3..].iter().all(|e| !e.is_folder()));
            }
        }
    }

    #[test]
    fn index_library_reaches_every_nested_folder_once_and_skips_dot_directories() {
        let root = nested_library("index");
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut all: Vec<StoryEntry> = Vec::new();
        index_library(&root, &root, |b| {
            dirs.push(b.dir.clone());
            all.extend(b.entries);
        });
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(dirs[0], root, "the root comes first");
        assert_eq!(dirs.len(), 4, "root, Glulx, zcode, zcode/german: {dirs:?}");
        let mut names: Vec<&str> = all.iter().map(|e| e.filename.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["burg.z5", "curses.z5", "top.z5"], "every story once, none from `.hidden`");
        assert!(all.iter().all(|e| !e.is_folder()), "the index carries stories, not folder rows");
    }

    #[test]
    fn search_library_matches_title_author_filename_and_folder_case_insensitively() {
        let root = PathBuf::from("/lib");
        let mut curses = story("Curses", "curses.z5", Some("Graham Nelson"), Some("1993"));
        curses.path = root.join("zcode/curses.z5");
        let mut burg = story("Die Burg", "burg.z5", None, None);
        burg.path = root.join("zcode/german/burg.z5");
        let mut top = story("Top", "top.z5", Some("Nobody"), None);
        top.path = root.join("top.z5");
        let index = vec![StoryEntry::folder(root.join("zcode"), "zcode/"), curses, burg, top];

        let titles = |q: &str| -> Vec<String> { search_library(&index, &root, q).iter().map(|e| e.title.clone()).collect() };
        assert_eq!(titles(""), vec!["Curses", "Die Burg", "Top"], "empty query lists every story, sorted by title, never a folder");
        assert_eq!(titles("CURSES"), vec!["Curses"], "title, case-insensitively");
        assert_eq!(titles("nelson"), vec!["Curses"], "author");
        assert_eq!(titles("burg.z5"), vec!["Die Burg"], "filename");
        assert_eq!(titles("german"), vec!["Die Burg"], "folder, relative to the root");
        assert_eq!(titles("zcode"), vec!["Curses", "Die Burg"], "a parent folder matches everything under it");
        assert_eq!(titles("zcode nel"), vec!["Curses"], "several terms all have to hit");
        assert!(titles("nothing-here").is_empty());
    }

    #[test]
    fn fetch_targets_reach_the_stories_in_all_folders_and_no_folder_rows() {
        let root = nested_library("fetch-targets");
        let targets = fetch_targets(&StorySource::Library(root.clone()), &root);
        let _ = std::fs::remove_dir_all(&root);
        let mut names: Vec<String> = targets
            .iter()
            .map(|t| t.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["burg.z5", "curses.z5", "top.z5"]);
        assert!(targets.iter().all(|t| !t.ifid.is_empty()), "a target carries the IFID the fetch is keyed on");
    }

    #[test]
    fn search_library_under_keeps_to_the_scope_and_its_folders() {
        let root = PathBuf::from("/lib");
        let mut a = story("Alpha", "a.z5", None, None);
        a.path = root.join("zcode/a.z5");
        let mut b = story("Beta", "b.z5", None, None);
        b.path = root.join("zcode/german/b.z5");
        let mut c = story("Gamma", "c.z5", None, None);
        c.path = root.join("glulx/c.z5");
        let index = vec![a, b, c];
        let names = |scope: &Path| -> Vec<String> {
            search_library_under(&index, &root, scope, "").iter().map(|e| e.title.clone()).collect()
        };
        assert_eq!(names(&root), vec!["Alpha", "Beta", "Gamma"], "the root is the whole library");
        assert_eq!(names(&root.join("zcode")), vec!["Alpha", "Beta"], "a folder and the folders under it");
        assert_eq!(names(&root.join("zcode/german")), vec!["Beta"]);
        assert!(names(&root.join("nothing")).is_empty());
        assert_eq!(
            search_library_under(&index, &root, &root.join("zcode"), "beta").len(),
            1,
            "the query still applies within the scope"
        );
    }

    #[test]
    fn folder_label_names_the_folder_below_the_root_and_nothing_at_it() {
        let root = PathBuf::from("/lib");
        let mut e = story("x", "x.z5", None, None);
        e.path = root.join("x.z5");
        assert_eq!(folder_label(&e, &root), None);
        e.path = root.join("zcode/german/x.z5");
        assert_eq!(folder_label(&e, &root).as_deref(), Some("zcode/german"));
        assert_eq!(folder_label(&e, &root.join("zcode")).as_deref(), Some("german"));
        e.path = PathBuf::from("/elsewhere/x.z5");
        assert_eq!(folder_label(&e, &root), None, "outside the tree is not a folder of it");
    }

    #[test]
    fn a_folder_row_carries_no_badges() {
        let dir = temp_dir("folder-badges");
        let row = StoryEntry::folder(dir.clone(), "dir/");
        let badges = compute_row_badges(&row, &dir, &hints::load_hint_index(&dir));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(badges, RowBadges::default());
    }
}
