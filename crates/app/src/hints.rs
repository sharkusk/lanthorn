use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

// ── Name pattern matching ──────────────────────────────────────────────────────

/// Returns true if `file_name` looks like a hint file.
///
/// A hint file must:
/// - have a `.z3`, `.z5`, or `.z8` extension, AND
/// - contain one of the keywords `hint`, `clue`, or `invisiclues` in its stem
///   (case-insensitive).
///
/// The extension alone (e.g. `zork1.z5`) does NOT match.
pub fn hint_name_matches(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let has_ext = lower.ends_with(".z3") || lower.ends_with(".z5") || lower.ends_with(".z8");
    if !has_ext {
        return false;
    }
    // Strip the extension to check only the stem.
    let stem = &lower[..lower.rfind('.').unwrap_or(lower.len())];
    stem.contains("hint") || stem.contains("clue") || stem.contains("invisiclues")
}

/// Returns true if `name` (case-insensitive) contains an Infocom Solid Gold
/// release/serial marker of the form `-r<digits>-s<digits>` (e.g.
/// `-r52-s871125`). Hand-parsed (no regex): find `-r`, require ≥1 ascii digit,
/// then `-s`, then ≥1 ascii digit. Returns true on the first such occurrence.
pub fn has_release_serial(name: &str) -> bool {
    let b = name.to_ascii_lowercase().into_bytes();
    let n = b.len();
    let mut i = 0;
    while i + 1 < n {
        if b[i] == b'-' && b[i + 1] == b'r' {
            let mut j = i + 2;
            let rstart = j;
            while j < n && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > rstart && j + 1 < n && b[j] == b'-' && b[j + 1] == b's' {
                let mut k = j + 2;
                let sstart = k;
                while k < n && b[k].is_ascii_digit() {
                    k += 1;
                }
                if k > sstart {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Returns true if `file_name` looks like an InvisiClues / hint image by name.
///
/// It must have a `.z3/.z5/.z8` extension AND its stem (lowercased, extension
/// stripped like [`hint_stem`]) either contains one of `hint`, `clue`,
/// `invisiclues`, or ends with the SLAG suffix `inv`, or ends with
/// `_hints`/`-hints`.
///
/// Unlike [`hint_name_matches`], this also recognises the IF-Archive SLAG
/// naming (`deadlineinv.z5`, `stuga_hints.z5`) which carries no keyword.
pub fn is_invisiclues_name(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let has_ext = lower.ends_with(".z3") || lower.ends_with(".z5") || lower.ends_with(".z8");
    if !has_ext {
        return false;
    }
    let stem = hint_stem(file_name);
    stem.contains("hint")
        || stem.contains("clue")
        || stem.contains("invisiclues")
        || stem.ends_with("inv")
        || stem.ends_with("_hints")
        || stem.ends_with("-hints")
        // waitingforgo `<abbrev>izm.z5` carries no keyword and no `inv` suffix;
        // recognise it only by the curated table (not a bare `izm` suffix, which
        // could false-positive on an unrelated filename).
        || curated_hint_key(&stem).is_some()
}

/// Returns true if `file_name` is a hint *sidecar* — an InvisiClues/hint image
/// that is NOT itself a full Solid Gold game.
///
/// Infocom Solid Gold releases like `zork1-invclues-r52-s871125.z5` carry a
/// `-r<digits>-s<digits>` marker and are full games (with built-in clues), not
/// standalone hint files, so they are excluded here.
pub fn is_hint_sidecar(file_name: &str) -> bool {
    is_invisiclues_name(file_name) && !has_release_serial(file_name)
}

/// Curated `(hint-stem, game-key)` table for the IF-Archive SLAG InvisiClues
/// collection, whose abbreviated names don't stem-match their games.
///
/// The hint-stem is the lowercased file stem without extension; the game-key is
/// a lowercased keyword present in the game's title/filename.
const SLAG_HINTS: &[(&str, &str)] = &[
    ("deadlineinv", "deadline"),
    ("enchaninv", "enchanter"),
    ("hhgginv", "hitchhiker"),
    ("hollywoodinv", "hollywood"),
    ("lgopinv", "leather"),
    ("lurkinginv", "lurking"),
    ("planetinv", "planetfall"),
    ("sorcinv", "sorcerer"),
    ("spellbinv", "spellbreaker"),
    ("starcrossinv", "starcross"),
    ("stationinv", "stationfall"),
    ("stuga_hints", "stuga"),
    ("suspendedinv", "suspended"),
    ("trininv", "trinity"),
    ("wishbrinv", "wishbringer"),
    ("zork1inv", "zork1"),
    ("zork2inv", "zork2"),
    ("zork3inv", "zork3"),
    ("ztuuinv", "ztuu"),
];

/// Curated `(hint-stem, game-key)` table for the waitingforgo *InvisiClues* set
/// (the `<abbrev>izm.z5` naming). The site is defunct, so downloads come from
/// the Internet Archive; the table also lets a locally-present `*izm.z5` file be
/// detected/hidden/badged like a SLAG sidecar. Multi-game "Collection" images
/// (fant1izm, scifizm, …) are intentionally excluded — they map to no one game.
const IZM_HINTS: &[(&str, &str)] = &[
    ("zork1izm", "zork1"),
    ("zork2izm", "zork2"),
    ("zork3izm", "zork3"),
    // Compound-word keys use the full concatenated game name, so a stray word
    // in an unrelated title (e.g. "Brain Guzzlers from Beyond") can't match.
    ("bzorkizm", "beyondzork"),
    ("zork0izm", "zork0"),
    ("zuuizm", "ztuu"),
    ("wishbizm", "wishbringer"),
    ("enchizm", "enchanter"),
    ("sorcrizm", "sorcerer"),
    ("spellizm", "spellbreaker"),
    ("trntyizm", "trinity"),
    ("starcizm", "starcross"),
    ("spendizm", "suspended"),
    ("plntfizm", "planetfall"),
    ("hitchizm", "hitchhiker"),
    ("amfvizm", "amfv"),
    ("statnizm", "stationfall"),
    ("deadlizm", "deadline"),
    ("witnizm", "witness"),
    ("spectizm", "suspect"),
    ("ballyizm", "ballyhoo"),
    ("moonmizm", "moonmist"),
    ("infdlizm", "infidel"),
    ("seastizm", "seastalker"),
    ("cutthizm", "cutthroats"),
    ("hollyizm", "hollywood"),
    ("shognizm", "shogun"),
    ("leathizm", "leather"),
    ("bureaizm", "bureaucracy"),
    ("nordizm", "nordandbert"),
    ("lurkizm", "lurking"),
    ("plundizm", "plunderedhearts"),
    ("bordrizm", "borderzone"),
    ("sherlizm", "sherlock"),
    ("journizm", "journey"),
    ("arthrizm", "arthur"),
];

/// Canonical titles (from `known_titles.tsv`, keyed by IFID) whose text does not
/// spell the hint catalogs' key for them.
///
/// Every catalog files Zork I as `zork1`, and every title of it writes the
/// number as a roman numeral; `Zork: The Undiscovered Underground` is filed as
/// `ztuu`, an abbreviation that appears nowhere in its name. No substring rule
/// can bridge either gap, so the mapping is listed.
///
/// Matched on the WHOLE normalised canonical title, never a substring — the TSV
/// also names `Mini-Zork I` and `Zork I Demo`, which are different games and
/// must fall through rather than collide with Zork I's clues.
const TITLE_KEYS: &[(&str, &str)] = &[
    ("zorkithegreatundergroundempire", "zork1"),
    ("zorkiithewizardoffrobozz", "zork2"),
    ("zorkiiithedungeonmaster", "zork3"),
    ("zorkzerotherevengeofmegaboz", "zork0"),
    ("zorktheundiscoveredunderground", "ztuu"),
];

/// The catalog key for a story's **identity** — nothing here reads a filename.
///
/// The IFID names the exact build (release + serial), `known_titles.tsv` names
/// the game that build is, and [`TITLE_KEYS`] names the catalog key for that
/// game. `None` when the IFID is unknown or the title spells its own key (the
/// common case, which substring matching already handles).
fn identity_hint_key(ifid: &str) -> Option<&'static str> {
    let norm = normalize_ident(crate::session::known_title(ifid)?);
    TITLE_KEYS.iter().find(|(t, _)| *t == norm).map(|(_, k)| *k)
}

/// The canonical title for a story's identity, normalised for key matching.
///
/// Empty when the IFID names no game we know. Unlike the caller's displayed
/// title this cannot have come from a container's filename, so it is the first
/// thing key matching consults (SQ-0767).
fn identity_ident(ifid: &str) -> String {
    crate::session::known_title(ifid).map(normalize_ident).unwrap_or_default()
}

/// Lowercase a string keeping only ASCII alphanumerics — so `"Beyond Zork"`,
/// `"beyond_zork"`, and `"beyondzork"` all normalise to `"beyondzork"`. Matching
/// game keys against the normalised form lets a multi-word canonical name match
/// while a stray word inside an unrelated title/filename (e.g. the "Beyond" in
/// "Brain Guzzlers from Beyond") cannot.
pub(crate) fn normalize_ident(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The curated game-key for a hint-file `stem`, consulting both the SLAG and the
/// izm tables (SLAG first). `None` when neither table names the stem.
fn curated_hint_key(stem: &str) -> Option<&'static str> {
    SLAG_HINTS
        .iter()
        .chain(IZM_HINTS.iter())
        .find(|(s, _)| *s == stem)
        .map(|(_, key)| *key)
}

/// The lowercased game keyword a hint file belongs to, or `None`.
///
/// First consults the curated tables ([`SLAG_HINTS`] / [`IZM_HINTS`]) by exact
/// stem (so `enchaninv` → `enchanter`, which a naïve strip would get wrong); else derives
/// the base by stripping a trailing hint marker (longest first), returning it
/// when it is ≥3 chars (so `zork1_hints` → `zork1`).
pub fn hint_game_key(file_name: &str) -> Option<String> {
    let stem = hint_stem(file_name);
    if let Some(key) = curated_hint_key(&stem) {
        return Some(key.to_string());
    }
    // Longest markers first so `-invisiclues` isn't shortened to `inv` etc.
    const MARKERS: &[&str] = &[
        "-invisiclues",
        "-hints",
        "_hints",
        "-clues",
        "-hint",
        "_hint",
        "-clue",
        "-inv",
        "_inv",
        "inv",
    ];
    for m in MARKERS {
        if let Some(base) = stem.strip_suffix(m) {
            return (base.len() >= 3).then(|| base.to_string());
        }
    }
    None
}

/// Returns true if the hint file `hint_file_name` is associated with the story
/// identified by `story_stem_or_title` (a filename stem OR a title).
///
/// True when [`hint_game_key`] yields a key of length ≥3 that the lowercased
/// `story_stem_or_title` contains.
pub fn hint_matches_story(hint_file_name: &str, story_stem_or_title: &str) -> bool {
    match hint_game_key(hint_file_name) {
        Some(key) if key.len() >= 3 => {
            let k = normalize_ident(&key);
            !k.is_empty() && normalize_ident(story_stem_or_title).contains(&k)
        }
        _ => false,
    }
}

/// Returns true if the hint file `hint_file_name` belongs to the story with this
/// `ifid` — the same question as [`hint_matches_story`], asked of the story's
/// identity instead of a name (SQ-0767).
///
/// A local `zork1inv.z5` sitting beside `Zork I - The Great Underground
/// Empire.adf` is that floppy's InvisiClues, and no comparison of the two
/// filenames can say so.
pub fn hint_matches_identity(hint_file_name: &str, ifid: &str) -> bool {
    let Some(key) = hint_game_key(hint_file_name) else { return false };
    let k = normalize_ident(&key);
    if k.len() < 3 {
        return false;
    }
    match identity_hint_key(ifid) {
        Some(ik) => normalize_ident(ik) == k,
        None => identity_ident(ifid).contains(&k),
    }
}

/// A downloadable InvisiClues hint file for a story: where to fetch it and what
/// to name the file saved next to the story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintDownload {
    /// Filename to save beside the story, e.g. `deadlineinv.z5`.
    pub filename: String,
    /// Fully-qualified download URL.
    pub url: String,
}

fn slag_url(stem: &str) -> String {
    format!("https://ifarchive.org/if-archive/solutions/slag/{stem}.z5")
}

/// The waitingforgo site is defunct, so izm files come from a fixed Internet
/// Archive snapshot. The `id_` suffix serves the raw bytes (no archive chrome);
/// the request 302-redirects to the nearest capture, which ureq follows.
fn izm_url(stem: &str) -> String {
    format!("https://web.archive.org/web/20161027165356id_/http://www.waitingforgo.com/invisiclues/{stem}.z5")
}

/// Find a downloadable InvisiClues hint file for a story, matched by the game
/// key its **identity** resolves to — falling back to the filename stem and the
/// displayed title only when the identity names no game we know.
///
/// The medium is not the story (SQ-0767): a disk image is named for its box
/// (`Zork I - The Great Underground Empire.adf`), so its filename never
/// contains `zork1` and neither does the title derived from it. `ifid` carries
/// the mounted story's release and serial, which name the build regardless of
/// what the file on disk is called, so it is consulted first.
///
/// SLAG (live IF Archive) is preferred; the izm set (Internet Archive) is the
/// fallback for games SLAG doesn't cover. Returns `None` when no catalog entry
/// matches. A key must be ≥3 chars to match (guards against spurious hits).
pub fn hint_download_for(ifid: &str, game_stem: &str, game_title: &str) -> Option<HintDownload> {
    let identity_key = identity_hint_key(ifid);
    let canonical = identity_ident(ifid);
    let stem = normalize_ident(game_stem);
    let title = normalize_ident(game_title);
    let matches = |key: &str| {
        let k = normalize_ident(key);
        if k.len() < 3 {
            return false;
        }
        match identity_key {
            // The identity names its catalog key outright — authoritative, and
            // exclusive: no other key can be right for this build.
            Some(ik) => normalize_ident(ik) == k,
            // Else the identity-resolved canonical title, then — last resort —
            // the filename stem and the displayed title, which for a story
            // mounted out of a container are the CONTAINER's, not the game's.
            None => canonical.contains(&k) || stem.contains(&k) || title.contains(&k),
        }
    };
    if let Some((s, _)) = SLAG_HINTS.iter().find(|(_, k)| matches(k)) {
        return Some(HintDownload { filename: format!("{s}.z5"), url: slag_url(s) });
    }
    if let Some((s, _)) = IZM_HINTS.iter().find(|(_, k)| matches(k)) {
        return Some(HintDownload { filename: format!("{s}.z5"), url: izm_url(s) });
    }
    None
}

/// True if `text` is the InvisiClues narrow-screen boot banner. The izm hint
/// files print it when the advertised screen width (a single header byte, so
/// ≤255) is below their longest menu-item name, which can reach 512 chars — so
/// it fires for any real terminal. Matched on the stable phrase (not the width
/// number) so the hint boot can auto-skip it. See `hint_opening` in main.rs.
pub fn is_narrow_screen_warning(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("your screen is only") && t.contains("characters wide")
}

/// The stem of a hint file's name, i.e. the raw file name minus a trailing
/// `.z3/.z5/.z8` extension, lowercased.
///
/// Unlike `Path::file_stem`, this strips only the story extension, so a compound
/// name like `zork1.hints.z5` yields `zork1.hints` (not `zork1`).
fn hint_stem(file_name: &str) -> String {
    let lower = file_name.to_ascii_lowercase();
    for ext in [".z3", ".z5", ".z8"] {
        if let Some(s) = lower.strip_suffix(ext) {
            return s.to_string();
        }
    }
    lower
}

/// Returns true when the hint-file `candidate_name`'s stem starts with the
/// story's stem (both compared case-insensitively).
///
/// So story `zork1` matches `zork1_hints.z5`, `zork1.hints.z5`, and
/// `zork1-invisiclues.z5`, but not `zork2_hints.z5`.
///
/// A bare `starts_with` would let story `zork` match `zork2_hints.z5`, so the
/// prefix must be followed by end-of-stem or a separator (not another
/// alphanumeric): the hint name is the story name plus a hint suffix, never a
/// longer word that merely begins the same way.
fn stem_matches_story(story_stem: &str, candidate_name: &str) -> bool {
    if story_stem.is_empty() {
        return false;
    }
    let story = story_stem.to_ascii_lowercase();
    let cand = hint_stem(candidate_name);
    match cand.strip_prefix(&story) {
        Some(rest) => rest.chars().next().is_none_or(|c| !c.is_ascii_alphanumeric()),
        None => false,
    }
}

/// Rank hint candidate names by story-stem preference and return the chosen one.
///
/// Tiers:
/// 1. any candidate the story owns — its stem starts with `story_stem`
///    ([`stem_matches_story`]) or its curated/derived game key matches the story
///    ([`hint_matches_story`]) → the first such after a stable name sort
///    (deterministic regardless of readdir order);
/// 2. else if exactly one candidate exists → that lone (generic) candidate;
/// 3. else (multiple candidates, none story-specific) → `None` (ambiguous).
///
/// Returns `None` for an empty list too; callers distinguish empty from
/// ambiguous by checking whether the input was empty.
fn pick_hint_candidate(story_stem: &str, mut names: Vec<String>) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    names.sort();
    if let Some(m) = names
        .iter()
        .find(|n| stem_matches_story(story_stem, n) || hint_matches_story(n, story_stem))
    {
        return Some(m.clone());
    }
    if names.len() == 1 {
        return names.into_iter().next();
    }
    None
}

// ── Built-in HINT detection ───────────────────────────────────────────────────

/// Returns true if the story's dictionary contains `hint` or `hints`
/// (case-insensitive).  This is a heuristic: a dictionary entry strongly
/// suggests the story has a built-in hint command, surfaced as a suggestion
/// (never an auto-action).
pub fn story_supports_hint<I: IntoIterator<Item = String>>(dictionary: I) -> bool {
    for word in dictionary {
        let lower = word.to_ascii_lowercase();
        if lower == "hint" || lower == "hints" {
            return true;
        }
    }
    false
}

// ── Per-IFID hint index ───────────────────────────────────────────────────────

/// In-memory map of IFID → hint file path, loaded from `dir/hints/index.toml`.
pub struct HintIndex {
    map: HashMap<String, PathBuf>,
}

impl HintIndex {
    /// Look up the hint file associated with the given IFID.
    pub fn get(&self, ifid: &str) -> Option<PathBuf> {
        self.map.get(ifid).cloned()
    }
}

/// Load the hint index from `dir/hints/index.toml`.
///
/// Returns an empty index if the file does not exist or cannot be parsed.
pub fn load_hint_index(dir: &Path) -> HintIndex {
    let path = dir.join("hints").join("index.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return HintIndex { map: HashMap::new() },
    };
    let table: toml::Table = match toml::from_str(&text) {
        Ok(t) => t,
        Err(_) => return HintIndex { map: HashMap::new() },
    };
    let mut map = HashMap::new();
    for (key, val) in table {
        if let toml::Value::String(s) = val {
            map.insert(key, PathBuf::from(s));
        }
    }
    HintIndex { map }
}

/// Persist a hint-file association for `ifid` to `dir/hints/index.toml`.
///
/// Creates the `dir/hints/` directory if absent.  Merges into any existing
/// entries (does not overwrite unrelated IFIDs).
pub fn save_hint_assoc(dir: &Path, ifid: &str, path: &Path) -> io::Result<()> {
    let hints_dir = dir.join("hints");
    std::fs::create_dir_all(&hints_dir)?;
    let index_path = hints_dir.join("index.toml");

    // Load existing document (format-preserving) or start fresh.
    let existing = std::fs::read_to_string(&index_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = existing.parse().unwrap_or_default();

    doc[ifid] = toml_edit::value(path.to_string_lossy().as_ref());

    std::fs::write(&index_path, doc.to_string())
}

// ── Discovery ─────────────────────────────────────────────────────────────────

/// The outcome of hint-source resolution.
#[derive(Debug, PartialEq)]
pub enum HintResolution {
    /// A hint file was found at this path.
    File(PathBuf),
    /// A hint entry was found inside a ZIP at `zip_path`; `entry` is its name.
    ///
    /// The caller should use `read_zip_entry` to extract the bytes.
    ZipEntry {
        zip_path: PathBuf,
        entry: String,
    },
    /// No hint file was found automatically — ask the user to choose one.
    AskUser,
    /// (Reserved for future use — e.g. when a `None` branch is needed.)
    None,
}

/// Resolve a hint source for the given story.
///
/// Discovery order:
/// 1. Remembered: the per-IFID association from `index`.
/// 2. The story's OWN container: when `story_path` is itself a ZIP, a
///    hint-sidecar entry inside it (SQ-1085).
/// 3. Sibling files: any `.z3/.z5/.z8` whose name is a hint sidecar
///    ([`is_hint_sidecar`]) in the same directory as `story_path` — this finds
///    SLAG files like `deadlineinv.z5` while skipping full Solid Gold games
///    like `zork1-invclues-r52-s871125.z5`.
/// 4. Sibling ZIP: any `.zip` in the same directory that contains a hint-sidecar
///    entry; returns `ZipEntry` so the caller can extract the bytes with
///    `read_zip_entry`.
/// 5. Else: `AskUser` (caller should open the file browser).
pub fn resolve_hint_source(story_path: &Path, ifid: &str, index: &HintIndex) -> HintResolution {
    // Step 1: remembered association.
    if let Some(remembered) = index.get(ifid) {
        if remembered.exists() {
            return HintResolution::File(remembered);
        }
    }

    // Step 2: the container the story came out of (SQ-1085). A download that
    // packs the clues beside the game has already answered the question, so it
    // is asked before the directory — which is a place a person filed things,
    // and where an ambiguous pair of sidecars deliberately stops the search
    // (step 3 returns `AskUser` rather than falling through to the zips).
    if is_zip(story_path) {
        let stem = story_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Ok(Some(entry)) = find_hint_entry_in_zip(story_path, &stem) {
            return HintResolution::ZipEntry { zip_path: story_path.to_path_buf(), entry };
        }
    }

    // Steps 3 + 4: scan siblings, collecting zip files for step 4.
    if let Some(dir) = story_path.parent() {
        // The story's own stem drives story-aware matching (`zork1.z5` → `zork1`).
        let story_stem = story_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut hint_files: Vec<PathBuf> = Vec::new();
            let mut zips: Vec<PathBuf> = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path == story_path {
                    continue; // skip the story itself
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                // Step 3: collect hint sidecars (ranked below); skips full
                // Solid Gold games that carry a release/serial marker.
                if is_hint_sidecar(name) {
                    hint_files.push(path.clone());
                }
                // Collect ZIPs for step 4.
                if name.to_ascii_lowercase().ends_with(".zip") {
                    zips.push(path);
                }
            }

            // Step 3: rank the sibling hint files. Prefer a story-stem match,
            // then a lone generic; multiple generics with no story match are
            // ambiguous — ask the user rather than guess.
            if !hint_files.is_empty() {
                let names: Vec<String> = hint_files
                    .iter()
                    .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
                    .collect();
                if let Some(chosen) = pick_hint_candidate(&story_stem, names) {
                    for path in &hint_files {
                        if path.file_name().and_then(|n| n.to_str()) == Some(chosen.as_str()) {
                            return HintResolution::File(path.clone());
                        }
                    }
                }
                // Ambiguous (multiple, none story-specific): don't fall through
                // to the ZIP guess.
                return HintResolution::AskUser;
            }

            // Step 4: look inside sibling ZIPs for a hint entry, story-aware.
            zips.sort();
            for zip_path in zips {
                if let Ok(Some(entry_name)) = find_hint_entry_in_zip(&zip_path, &story_stem) {
                    return HintResolution::ZipEntry { zip_path, entry: entry_name };
                }
            }
        }
    }

    HintResolution::AskUser
}

/// Return the best-ranked hint-sidecar entry in `zip_path`, preferring an entry
/// the story owns (stem prefix or game-key match).
///
/// Applies the same tiers as [`pick_hint_candidate`]: a story-stem match wins;
/// a lone generic entry is used; multiple generics with no story match are
/// ambiguous and yield `None`.
fn find_hint_entry_in_zip(zip_path: &Path, story_stem: &str) -> io::Result<Option<String>> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut matches: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = entry.name().to_string();
        // Only the bare filename portion needs to match the pattern.
        let basename = name.rsplit('/').next().unwrap_or(&name);
        if is_hint_sidecar(basename) {
            matches.push(name);
        }
    }
    // Rank by the bare filename, but return the full entry path.
    let basename_of = |n: &str| n.rsplit('/').next().unwrap_or(n).to_string();
    let basenames: Vec<String> = matches.iter().map(|n| basename_of(n)).collect();
    match pick_hint_candidate(story_stem, basenames) {
        Some(chosen) => Ok(matches.into_iter().find(|n| basename_of(n) == chosen)),
        None => Ok(None),
    }
}

// ── Zip helpers ───────────────────────────────────────────────────────────────

/// ZIP magic bytes (local file header signature).
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// A story image classified by the VM engine that runs it.
///
/// `extract_story` returns this so session creation can route a Z-code image to
/// the Z-machine (`GameSession`) and a Glulx image to `GlulxSession`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadedStory {
    /// A Z-machine story image (`.z*` / `ZCOD` Blorb).
    ZCode(Vec<u8>),
    /// A Glulx story image (`.ulx` / `GLUL` Blorb / `.gblorb`).
    Glulx(Vec<u8>),
    /// A Scott Adams story database (`.dat`).
    Scott(Vec<u8>),
}

impl LoadedStory {
    /// The raw executable bytes, regardless of engine.
    pub fn bytes(&self) -> &[u8] {
        match self {
            LoadedStory::ZCode(b) | LoadedStory::Glulx(b) | LoadedStory::Scott(b) => b,
        }
    }
    /// Consume into the raw executable bytes, regardless of engine.
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            LoadedStory::ZCode(b) | LoadedStory::Glulx(b) | LoadedStory::Scott(b) => b,
        }
    }
}

/// Which release medium a story was mounted out of, when it was one at all.
///
/// Re-exported rather than declared here (SQ-0839): the medium implies a machine
/// implies an interpreter number, and `zvm-cli` needs that same conclusion
/// without depending on `app`. `blorb` is where these filesystems are
/// recognised and the only crate both front-ends share, so it owns the type and
/// the mapping — see [`blorb::medium`]. Every existing `app::hints::DiskImage`
/// spelling keeps working.
pub use blorb::medium::DiskImage;

/// Every story a disk image holds, in disk order — the name the volume stores
/// each one under, and its bytes — or `None` when `path` is not a disk image (or
/// will not mount, or holds no story at all).
///
/// This is the enumeration the picker lists one row per (SQ-0859). It is
/// deliberately the SAME question `zvm-cli`'s `--story` menu asks, of the same
/// `blorb::medium` seam, so the two front-ends cannot end up with different
/// ideas of what is on a disk: what the menu offers, the browser offers.
///
/// The names come back exactly as [`blorb::medium::MountedDisk::stories`] gives
/// them — by path on the formats that have directories — which is what makes
/// them usable as the selector [`load_mounted_story_from`] takes.
///
/// # Two mediums, not one (SQ-0876)
///
/// The first answer is the VOLUME's format, which is what it always was. Each
/// story then carries **its own**, because on a hybrid disc those differ: the
/// Masterpieces CD's Macintosh partition holds 50 DOS builds beside its 33
/// Macintosh ones, and a browser that reported the volume's format for all 83
/// told every PC build to advertise itself as a Macintosh. Every other medium
/// answers its one format for every story on it, so the pair is equal there and
/// nothing moves.
pub fn mounted_stories(
    path: &Path,
) -> Option<(DiskImage, Vec<(blorb::medium::DiskStory, DiskImage)>)> {
    let raw = std::fs::read(path).ok()?;
    blorb::medium::DiskImage::detect(&raw)?;
    let disk = mount_disk(path, raw).ok()?;
    let format = disk.format();
    let stories: Vec<_> = disk
        .stories()
        .into_iter()
        .map(|s| {
            let image = disk.image_for(&s.name);
            (s, image)
        })
        .collect();
    (!stories.is_empty()).then_some((format, stories))
}

/// Open the disk image `path`, whose bytes are `raw`, with the other volumes of
/// its multi-disk release available to it (SQ-0864).
///
/// **The one place `app` mounts a disk**, and the reason it is one place: a
/// story can live on no single floppy. The Apple II 5.25-inch presses of
/// *Shogun* and *Zork Zero* page one game across five and four volumes, so
/// opening any one of them and asking what is on it is a question only the whole
/// release can answer.
///
/// Which files are one release is [`crate::disk_set`]'s answer, from their names
/// and without opening anything — the same rule the browser groups on, so a set
/// the picker shows as one shelf is the set the mount reads across. It is
/// deliberately not `blorb`'s: naming is filesystem policy and that crate is
/// given bytes.
///
/// Both are now [`cli_host::disk_set::mount_at`]'s, which is this function's
/// whole body (SQ-0874). `zvm-cli` mounted with no companions at all until it
/// could reach the rule, so *Trinity* opened in the TUI and not at the prompt;
/// it calls this same helper, and neither front-end can drift from the other
/// because there is nothing left to drift.
fn mount_disk(
    path: &Path,
    raw: Vec<u8>,
) -> Result<blorb::medium::MountedDisk, blorb::medium::MountError> {
    cli_host::disk_set::mount_at(path, raw)
}

/// Read a story file's executable bytes, transparently unwrapping a ZIP whose
/// first story entry is the game, or a release disk image — Amiga or
/// Macintosh — whose filesystem holds one. Does not classify the engine — see
/// [`load_story`] / [`extract_story`].
///
/// The flag says the bytes were mounted out of a disk image rather than read as
/// a plain file, and which kind, so callers can name the container the story
/// came off (SQ-0737).
///
/// `want` names which story to take out of a CONTAINER that holds several — a
/// disk image (SQ-0859) or a zip (SQ-1098) — by the name [`mounted_stories`] or
/// [`zipped_stories`] listed it under. `None` is the container's own tiebreak:
/// what a bare path has always opened, and what every single-story disk and
/// single-story archive means whatever is passed.
fn read_story_file(path: &Path, want: Option<&str>) -> io::Result<(Vec<u8>, Option<DiskImage>)> {
    let raw = std::fs::read(path)?;
    // An original release floppy, whichever machine pressed it (SQ-0719,
    // SQ-0837, SQ-0840). One mount path answers for every format: take the file
    // whose CONTENT is a story, because a release disk's names are a tiebreak
    // and never a guarantee — AmigaDOS has no extensions at all, and every Atari
    // ST story is called `STORY.DAT`.
    //
    // `detect` first because `mount` consumes the bytes, and a plain story file
    // has to fall through to the paths below with them intact.
    if blorb::medium::DiskImage::detect(&raw).is_some() {
        let disk = mount_disk(path, raw)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        // A named story is the browser's row (SQ-0859): the picker listed every
        // story on the image and this is the one the player chose, so it is
        // looked up by that same name and NOT quietly replaced by the tiebreak
        // if it has gone — an image edited between the scan and the launch must
        // say so rather than open a different game.
        if let Some(want) = want {
            let found = disk
                .stories()
                .into_iter()
                .find(|s| s.name == want || s.name.eq_ignore_ascii_case(want));
            return match found {
                // `image_for`, not `format`: on a hybrid disc the story's own
                // half of the platter decides, so a DOS build sitting on a
                // Macintosh filesystem reports DOS and advertises the IBM PC's
                // interpreter number rather than the Macintosh's (SQ-0876).
                // Every other medium answers its one format, as before.
                Some(story) => {
                    let image = disk.image_for(&story.name);
                    Ok((story.bytes, Some(image)))
                }
                None => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no story named '{want}' on the disk image {}", path.display()),
                )),
            };
        }
        return match disk.story() {
            Some(story) => {
                let image = disk.image_for(&story.name);
                Ok((story.bytes, Some(image)))
            }
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no story file on the disk image {} ({} files{}; is this the boot disk?)",
                    path.display(),
                    disk.file_count(),
                    // Only some formats keep a volume name; the message says so
                    // when there is one and reads naturally when there is not.
                    disk.volume_name().map(|n| format!(" on {n}")).unwrap_or_default(),
                ),
            )),
        };
    }
    if raw.starts_with(ZIP_MAGIC) {
        // A ZIP is somebody's DOWNLOAD, not a lanthorn container (the `.lanthorn`
        // archive is that, and it is a zip too — see `crate::archive`). So it is
        // opened the way a release floppy is: by asking what is INSIDE each
        // entry, not what its name claims. SQ-1085.
        // A named entry is the browser's row (SQ-1098), exactly as it is on a
        // disk image twenty lines above: the picker listed every story in the
        // archive and this is the one the player chose.
        let scan = zip_story(path, want)?;
        return match scan.story {
            Some((_name, bytes)) => Ok((bytes, None)),
            None if want.is_some() => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no story named '{}' in the zip {}",
                    want.unwrap_or_default(),
                    path.display(),
                ),
            )),
            None => {
                // SQ-1096: the loader still knows exactly four kinds of story
                // and still classifies by content — but "none of them" is a
                // useless thing to say about an archive of floppies, which is
                // the ordinary packaging for C64, Amiga and Apple II releases.
                // Counted by NAME, and only for the message: a `.d64` is raw
                // sectors with no magic, so there is no content answer to give,
                // and none is needed to tell the player what they are holding.
                let images = zip_entry_names(path)
                    .map(|names| {
                        names.iter().filter(|n| crate::story_url::is_disk_image_name(n)).count()
                    })
                    .unwrap_or(0);
                let media = if images == 0 {
                    String::new()
                } else {
                    format!(
                        "; it holds {images} disk image{}, which lanthorn does not run from \
                         inside a zip — unpack them into your story directory and open one \
                         of those",
                        if images == 1 { "" } else { "s" },
                    )
                };
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "no story file inside the zip {} ({} entr{} read; none is a Blorb, \
                         a Z-machine story, a Glulx image or a Scott Adams database{})",
                        path.display(),
                        scan.examined,
                        if scan.examined == 1 { "y" } else { "ies" },
                        media,
                    ),
                ))
            }
        };
    }
    Ok((raw, None))
}

/// Total inflated bytes one scan of a ZIP will read before it gives up
/// (SQ-1085).
///
/// **A different question from [`MAX_ZIP_ENTRY`], and deliberately not a
/// multiple of it.** That one bounds a BUFFER — how much memory a single entry
/// may occupy at once. This one bounds WORK — how long a launch may spend
/// inflating before it concludes there is no game in here. Tying the two
/// together is how the pair goes wrong: raising the buffer to cover
/// *Kerkerkruip* would otherwise have raised the whole-archive budget to half a
/// gigabyte, which is not a launch anybody wants to wait through.
///
/// 128 MiB is set from the work side. The scan STOPS at the first entry whose
/// content is a story, so an ordinary download spends the size of its game and
/// nothing more; the budget is only reached by an archive of large entries that
/// are not stories, and at deflate's throughput that worst case is well under a
/// second. It leaves room for the largest game known plus its resources plus a
/// couple of large decoys ahead of them in archive order, and a zip bomb still
/// cannot exceed it.
const MAX_ZIP_SCAN: u64 = 128 * 1024 * 1024;

/// The two limits above, pinned against what the corpus actually HOLDS — at
/// compile time, because a number that has to cover a real file is a fact about
/// the build and not a thing to discover when a player opens a zip (SQ-1085).
///
/// The figures live in [`crate::corpus`], which is also where the reasoning is:
/// this cap and `ifdb_search::MAX_DOWNLOAD` were BOTH too small, written
/// independently under two different confident sentences about how big games
/// are. The caps stay separate because they bound different things; the floor
/// they are checked against is one fact and now has one home.
const _: () = assert!(
    MAX_ZIP_ENTRY > crate::corpus::LARGEST_GAME,
    "one zip entry must be able to hold the largest game anybody ships",
);
const _: () = assert!(
    MAX_ZIP_ENTRY > crate::corpus::LARGEST_DISC,
    "…and a compilation disc, which the classifier already recognises",
);
const _: () = assert!(
    MAX_ZIP_SCAN > MAX_ZIP_ENTRY,
    "the whole-archive budget must be able to read past one full-size entry",
);

/// Hand every entry of the ZIP at `path` that could plausibly BE a story or a
/// resource file to `visit`, as (stored name, inflated bytes), stopping early
/// when `visit` returns `true`. Answers how many entries were actually read.
///
/// Two things are skipped without inflating: a directory entry, and an entry
/// whose declared size is zero or past [`MAX_ZIP_ENTRY`]. The declared size is
/// the archive's own claim and so is attacker data — it is used only to SKIP,
/// never to size a buffer, and an entry that lies low is still bounded by the
/// capped read below (and then skipped, because a story never overflows it).
fn for_each_zip_entry(
    path: &Path,
    mut visit: impl FnMut(&str, Vec<u8>) -> bool,
) -> io::Result<usize> {
    use std::io::Read as _;

    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let mut budget = MAX_ZIP_SCAN;
    let mut examined = 0usize;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if entry.is_dir() {
            continue;
        }
        let declared = entry.size();
        if declared == 0 || declared > MAX_ZIP_ENTRY || declared > budget {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        (&mut entry).take(MAX_ZIP_ENTRY + 1).read_to_end(&mut buf)?;
        budget = budget.saturating_sub(buf.len() as u64);
        if buf.len() as u64 > MAX_ZIP_ENTRY {
            // The declared size lied — the entry inflates past a cap set to
            // cover the largest game anybody ships (see [`MAX_ZIP_ENTRY`]), so
            // it is not a candidate. The scan carries on rather than erroring,
            // because one hostile entry must not hide a real game beside it.
            continue;
        }
        examined += 1;
        if visit(&name, buf) {
            break;
        }
    }
    Ok(examined)
}

/// The entry of the ZIP at `path` whose CONTENT is a story lanthorn can run,
/// with the name the archive stores it under — and how many entries were read
/// looking for it (SQ-1085). `want` names WHICH entry (SQ-1098); without one it
/// is the first story in archive order, which is what a bare path has always
/// opened.
///
/// **Classified by content, exactly as a release floppy's files are.** The zip
/// branch used to name three extensions, so `.z4`, `.z6`, `.z7`, `.ulx`,
/// `.zblorb` and a Scott Adams `.dat` were all unreachable inside a zip while
/// opening the very same file loose worked — and the one format whose whole
/// point is that it ships artwork, Version 6, was the one a zip could not carry.
/// [`extract_story`] already knows what a story is and says so for every engine;
/// asking it is both broader and stricter than any list of spellings. That is
/// still true of a NAMED entry: the name says which one, never what it holds.
///
/// A named entry is looked up by that name and **not** quietly replaced by the
/// first story if it has gone, for the same reason the disk-image branch
/// refuses: an archive edited between the scan and the launch must say so
/// rather than open a different game. The scan stops at the matching name
/// whether or not its content turned out to be a story, so `story: None` with a
/// `want` in hand means "that name is not a game", never "keep looking".
fn zip_story(path: &Path, want: Option<&str>) -> io::Result<ZipScan> {
    let mut story: Option<(String, Vec<u8>)> = None;
    let examined = for_each_zip_entry(path, |name, bytes| match want {
        Some(want) if !(name == want || name.eq_ignore_ascii_case(want)) => false,
        // Classified by CONTENT even when it was asked for by name, exactly as
        // the unnamed scan is: the name is which entry, never what it holds.
        _ => match extract_story(bytes.clone()) {
            Ok(_) => {
                story = Some((name.to_string(), bytes));
                true
            }
            // A named miss stops here with nothing; an unnamed one carries on.
            Err(_) => want.is_some(),
        },
    })?;
    Ok(ZipScan { story, examined })
}

/// Every story a ZIP holds, in archive order — the name each entry is stored
/// under and its bytes — or `None` when `path` is not a zip, will not open, or
/// holds no story at all (SQ-1098).
///
/// The zip counterpart of [`mounted_stories`], and the enumeration the picker
/// lists one row per. SQ-1085 made a zip a volume — entries classified by
/// content, so it carries any format lanthorn runs — but stopped at the FIRST
/// story, so an archive holding two games played one of them and there was no
/// way to reach the other however long you looked at the list.
///
/// **Classified by [`extract_story`], the same question the single-story scan
/// asks**, so what the browser offers and what a launch opens cannot disagree
/// about what a story is. A resource-only `Journey.blb` beside the game is not
/// one and does not become a row.
///
/// Costs the whole archive rather than stopping at the first story, bounded by
/// the same [`MAX_ZIP_SCAN`] budget — which is what an ordinary download of a
/// game and its Blorb costs anyway, because the Blorb is read either way.
pub fn zipped_stories(path: &Path) -> Option<Vec<(String, Vec<u8>)>> {
    if !is_zip(path) {
        return None;
    }
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for_each_zip_entry(path, |name, bytes| {
        if extract_story(bytes.clone()).is_ok() {
            out.push((name.to_string(), bytes));
        }
        false
    })
    .ok()?;
    (!out.is_empty()).then_some(out)
}

/// What one scan of a ZIP for a story found, and how hard it looked.
struct ZipScan {
    /// The first entry whose content is a story: the name the archive stores it
    /// under, and its bytes.
    story: Option<(String, Vec<u8>)>,
    /// How many entries were actually inflated. The refusal quotes it, so an
    /// archive holding nothing lanthorn runs reads differently from one whose
    /// entries were all too large to be a story.
    examined: usize,
}

/// The stored names of every non-directory entry in the ZIP at `path`, in
/// archive order — read from the central directory, inflating nothing (SQ-1096).
///
/// A name is not a classification and this is not one: it exists so the FETCH
/// can ask what a download would unpack to, and so the refusal below can say how
/// many disk images it is refusing. `hints::extract_story` still decides what a
/// story IS by content alone.
pub fn zip_entry_names(path: &Path) -> io::Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut out = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if !entry.is_dir() {
            out.push(entry.name().to_string());
        }
    }
    Ok(out)
}

/// Does the file at `path` begin with the ZIP local-file-header signature?
///
/// By its bytes rather than by its extension, for the same reason
/// [`read_story_file`] sniffs one: the container a player downloaded is
/// whatever it is, whatever it is called.
pub(crate) fn is_zip(path: &Path) -> bool {
    use std::io::Read as _;
    let mut head = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head == *ZIP_MAGIC
}

/// Resource-container spellings looked for INSIDE a zip. The same four
/// `blorb::RESOURCE_BLORB_EXTS` names, which is `pub(crate)` there.
const ZIP_BLORB_EXTS: [&str; 4] = ["blb", "blorb", "zblorb", "gblorb"];

/// The Blorb of resources the ZIP at `zip_path` carries beside its story, and a
/// display path naming it, or `None` (SQ-1085).
///
/// **The defect this exists for**: the zip branch handed back `None` where the
/// disk-image branch hands back the medium the story was mounted out of, so a
/// `Journey.blb` sitting in the same zip as `journey.z6` was never opened. A
/// zipped Version 6 game loaded and then drew nothing.
///
/// The rule mirrors [`blorb::resolve_resource_blorb`]'s, read across the
/// archive's entries instead of a directory's files: an entry that parses as a
/// Blorb carrying resources is a candidate, the sole candidate wins, and several
/// are ranked by how much of the ZIP's own stem they share — refusing on a tie
/// rather than drawing another game's plates. A `.zblorb` holding both the story
/// and its resources is the sole candidate for the same reason the loose file is
/// its own tier 1: the story and the artwork came out of one file.
pub(crate) fn zip_resource_blorb(zip_path: &Path) -> Option<(blorb::Blorb, std::path::PathBuf)> {
    if !is_zip(zip_path) {
        return None;
    }
    let mut candidates: Vec<(String, blorb::Blorb)> = Vec::new();
    for_each_zip_entry(zip_path, |name, bytes| {
        let base = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
        if !ZIP_BLORB_EXTS.iter().any(|e| base.ends_with(&format!(".{e}"))) {
            return false;
        }
        if !blorb::Blorb::is_blorb(&bytes) {
            return false;
        }
        if let Ok(b) = blorb::Blorb::parse(bytes) {
            if !b.resources().is_empty() {
                candidates.push((name.to_string(), b));
            }
        }
        false
    })
    .ok()?;
    let named = |name: &str| zip_path.join(name);
    if candidates.len() == 1 {
        let (name, b) = candidates.pop()?;
        return Some((b, named(&name)));
    }
    // Several: rank by shared stem with the zip itself, which is the nearest
    // thing an archive has to the story's name.
    let stem = zip_path.file_stem()?.to_string_lossy().to_ascii_lowercase();
    let mut best: Option<(usize, String, blorb::Blorb)> = None;
    let mut tied = false;
    for (name, b) in candidates {
        let base = name.rsplit('/').next().unwrap_or(&name);
        let cand_stem = Path::new(base)
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let Some(plen) = zip_stem_prefix_match(&stem, &cand_stem) else {
            continue;
        };
        match &best {
            Some((bp, _, _)) if *bp > plen => {}
            Some((bp, _, _)) if *bp == plen => tied = true,
            _ => {
                best = Some((plen, name, b));
                tied = false;
            }
        }
    }
    if tied {
        return None;
    }
    best.map(|(_, name, b)| (b, named(&name)))
}

/// [`blorb::resolve_resource_blorb`]'s stem rule, restated for zip entries
/// because `blorb` keeps its copy private: the shorter stem must be a full
/// prefix of the longer, and at least three characters, so `zork1`↔`zork1-sounds`
/// pairs and `zork0`↔`zork1` does not.
fn zip_stem_prefix_match(story_stem: &str, cand_stem: &str) -> Option<usize> {
    let plen = story_stem.bytes().zip(cand_stem.bytes()).take_while(|(a, b)| a == b).count();
    let shorter = story_stem.len().min(cand_stem.len());
    (plen >= 3 && plen == shorter).then_some(plen)
}

/// Load a story from `path`, classified by engine ([`LoadedStory`]).
///
/// Unwraps a ZIP (first `.z*` entry) and a Blorb container; a raw `.ulx`
/// (`Glul` magic) is recognised as Glulx, and any other raw bytes are treated
/// as Z-code (preserving the historical default).
pub fn load_story(path: &Path) -> io::Result<LoadedStory> {
    Ok(load_mounted_story(path)?.0)
}

/// As [`load_story`], plus which **release disk image** the story was mounted
/// out of, if it was one rather than a plain file (SQ-0737, SQ-0837).
///
/// The answer is the mount's own — an image is recognised by its own filesystem,
/// never by its name — so a disk image with any extension is reported as one and
/// a mis-named ordinary story file is not.
pub fn load_mounted_story(path: &Path) -> io::Result<(LoadedStory, Option<DiskImage>)> {
    load_mounted_story_from(path, None)
}

/// [`load_mounted_story`] for one named story out of a container that holds
/// several — a disk image (SQ-0859) or a zip (SQ-1098) — the browser row's own
/// game, rather than the container's tiebreak.
///
/// `None` is exactly [`load_mounted_story`], so every loose file, every
/// single-story archive and every single-story floppy takes the byte-for-byte
/// path it always did.
pub fn load_mounted_story_from(
    path: &Path,
    disk_entry: Option<&str>,
) -> io::Result<(LoadedStory, Option<DiskImage>)> {
    let (bytes, disk_image) = read_story_file(path, disk_entry)?;
    Ok((extract_story(bytes)?, disk_image))
}

/// Load story bytes from `path`, restricted to **Z-code** images.
///
/// Convenience wrapper over [`load_story`] for the Z-machine-only call sites
/// (the hint companion VM, the picker's legacy path). A Glulx image is rejected
/// with a clear error so those sites behave exactly as before.
pub fn load_story_bytes(path: &Path) -> io::Result<Vec<u8>> {
    match load_story(path)? {
        LoadedStory::ZCode(b) => Ok(b),
        LoadedStory::Glulx(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Glulx story files are not supported on this path".to_string(),
        )),
        LoadedStory::Scott(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Scott Adams story files are not supported on this path".to_string(),
        )),
    }
}

/// Classify `bytes` into a [`LoadedStory`].
///
/// A Blorb yields its embedded executable's kind; a raw image starting with the
/// `Glul` magic is Glulx; a Scott Adams database is content-sniffed; and a
/// Z-machine story **proves itself by its header** like the other three prove
/// themselves, rather than being what is left over.
///
/// # Z-code stopped being the else-branch (SQ-0889)
///
/// It used to be: three formats were tested and everything else was handed to
/// the Z-machine, which is what this doc meant by "the historical pass-through…
/// never errors for a non-Blorb input". The only gate downstream was
/// `zvm::header::parse_header`'s `3..=8` on byte 0 — six of 256 values, so
/// roughly **2.3% of arbitrary containers pass it**, and one of them was an
/// 838 KB Apple II disk image whose DiskCopy 4.2 name-length byte is `0x06`.
/// lanthorn opened the whole image as a Version 6 story, paired it with a
/// sidecar Blorb belonging to a different file, printed
/// "story ended without asking for input", and exited **0** — a message that
/// reads as a game bug and sends the reader looking somewhere else entirely.
///
/// The check is [`blorb::adf::looks_like_zcode`], the same one every disk reader
/// already uses to decide which file on a volume is the game, and it is
/// borrowed rather than restated: it encodes ZMSD §1.1's memory map (dynamic
/// memory ends below `$0e`; the object and global tables are writable so they
/// are inside it; the dictionary is in static memory) and §11.1.6's file-length
/// word, and two of its clauses are corrections that a from-memory rewrite would
/// get wrong. A second copy is a second place for that to go stale.
///
/// Errors with [`io::ErrorKind::InvalidData`] and a diagnostic that says what
/// the bytes are, so the exit status is non-zero and the message names the
/// container instead of the game.
pub fn extract_story(bytes: Vec<u8>) -> io::Result<LoadedStory> {
    if !blorb::Blorb::is_blorb(&bytes) {
        // Raw image: distinguish Glulx by its `Glul` magic; a Scott Adams `.dat`
        // is content-sniffed (it has no fixed magic); a Z-machine story by its
        // header.
        if bytes.starts_with(b"Glul") {
            return Ok(LoadedStory::Glulx(bytes));
        }
        if let Ok(s) = std::str::from_utf8(&bytes) {
            if scott::looks_like_scott(s) {
                return Ok(LoadedStory::Scott(bytes));
            }
        }
        if !blorb::adf::looks_like_zcode(&bytes) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, unrunnable(&bytes)));
        }
        return Ok(LoadedStory::ZCode(bytes));
    }
    let b = blorb::Blorb::parse(bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("invalid Blorb: {e:?}")))?;
    match b.executable() {
        Ok((blorb::ExecKind::ZCode, data)) => Ok(LoadedStory::ZCode(data.to_vec())),
        Ok((blorb::ExecKind::Glulx, data)) => Ok(LoadedStory::Glulx(data.to_vec())),
        Ok((blorb::ExecKind::Scott, data)) => {
            // SQ-0629: gate the SAAI payload behind the same content sniff the
            // raw-`.dat` path above uses — a hostile blorb must not reach
            // scott's loader with arbitrary bytes just by claiming an SAAI
            // exec chunk.
            if !std::str::from_utf8(data).is_ok_and(scott::looks_like_scott) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Blorb SAAI executable does not look like a Scott Adams database",
                ));
            }
            Ok(LoadedStory::Scott(data.to_vec()))
        }
        Err(e) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Blorb has no executable: {e:?}"),
        )),
    }
}

/// Why `bytes` are not a story lanthorn can run — the diagnostic that replaced
/// running them anyway (SQ-0889).
///
/// Deliberately factual rather than clever, because the failure this exists for
/// is a file the player *thinks* is a game and is not, and the only useful thing
/// to say is what it actually is. Two facts, both always available: the length
/// and the head of the file, which is where a container writes its magic and
/// very often its name — `Shogun.po` opens `06 53 48 4f 47 55 4e 20  |.SHOGUN |`,
/// and that line alone identifies it — and, when `blorb` recognises the bytes as
/// a disk image, the format's own label, because "this is a container, not a
/// game" is the whole answer in that case.
fn unrunnable(bytes: &[u8]) -> String {
    let head = &bytes[..bytes.len().min(8)];
    let hex: Vec<String> = head.iter().map(|b| format!("{b:02x}")).collect();
    let text: String =
        head.iter().map(|b| if (0x20..0x7f).contains(b) { char::from(*b) } else { '.' }).collect();
    let what = match blorb::medium::DiskImage::detect(bytes) {
        Some(image) => format!("it is a {} disk image — a container, not a story", image.label()),
        None => "no format lanthorn reads claims it".to_string(),
    };
    format!(
        "not a story file: {} bytes beginning {}  |{text}| — {what}. It is not a Blorb, \
         not a Glulx image, not a Scott Adams database, and its first 64 bytes are not a \
         Z-machine header (ZMSD §11.1)",
        bytes.len(),
        hex.join(" "),
    )
}

/// Cap on one extracted ZIP entry (SQ-0660, raised SQ-1085) — the largest
/// buffer one entry may inflate into.
///
/// **It has to cover the largest thing lanthorn RUNS, and that is no longer a
/// Z-code image.** The cap was 4 MiB against a zip branch that accepted `.z3`,
/// `.z5` and `.z8` only, and a Z-machine story tops out at 512 KiB, so it was
/// eight times the format's own ceiling. Opening a zip by CONTENT changed what
/// the constant has to cover and the constant did not move with it — and neither
/// Glulx nor Blorb has a ceiling to appeal to (a Blorb chunk length is a 32-bit
/// word, Glulx's address space is 32-bit), so the number can only come from what
/// people actually ship.
///
/// What this repo's own `stories/` holds, largest first:
///
/// | file | size |
/// |---|---|
/// | `Kerkerkruip.gblorb` | 21.1 MB |
/// | `Kerkerkruip.b10.gblorb` | 13.6 MB |
/// | `InfocomMasterpieces.img` (a disc, not a story) | 12.0 MB |
/// | `Never Gives Up Her Dead.gblorb` | 11.1 MB |
/// | `CounterfeitMonkey-11.gblorb` | 10.8 MB |
/// | `cragne.gblorb` | 8.5 MB |
///
/// 32 MiB clears the largest of those by half again, and clears the compilation
/// disc too — a zipped disk image is a plausible next case, since the zip branch
/// hands its bytes to the same classifier that already recognises one. It is
/// still a single bounded allocation, and a hostile zip claiming multiple GB is
/// stopped exactly as it was.
const MAX_ZIP_ENTRY: u64 = 32 * 1024 * 1024;

/// Return the bytes of the first ZIP entry whose name satisfies `pred`.
///
/// Returns `Ok(None)` if no entry matches.  Returns `Err` if the file cannot
/// be opened, is not a valid ZIP, or the matched entry inflates beyond
/// [`MAX_ZIP_ENTRY`].
pub fn read_zip_entry(
    zip_path: &Path,
    pred: impl Fn(&str) -> bool,
) -> io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;

    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if pred(entry.name()) {
            let name = entry.name().to_string();
            // Capped read (one byte past the cap detects overflow without ever
            // buffering the excess): the entry's declared size is attacker
            // data, so the *inflated* stream itself is what gets bounded.
            let mut buf = Vec::new();
            (&mut entry).take(MAX_ZIP_ENTRY + 1).read_to_end(&mut buf)?;
            if buf.len() as u64 > MAX_ZIP_ENTRY {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("zip entry '{name}' inflates beyond {MAX_ZIP_ENTRY} bytes"),
                ));
            }
            return Ok(Some(buf));
        }
    }
    Ok(None)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-guidance"))]
mod tests {
    use super::*;

    #[test]
    fn hint_name_matches_patterns() {
        assert!(hint_name_matches("zork1.invisiclues.z5"));
        assert!(hint_name_matches("MyGame-hints.z5"));
        assert!(hint_name_matches("clues.z3"));
        assert!(!hint_name_matches("zork1.z5"));     // the story itself
        assert!(!hint_name_matches("hints.txt"));    // wrong extension
    }

    #[test]
    fn has_release_serial_detects_solid_gold_marker() {
        assert!(has_release_serial("zork1-invclues-r52-s871125.z5"));
        assert!(has_release_serial("deadline-r27-s851006.z3"));
        assert!(!has_release_serial("deadlineinv.z5"));
        assert!(!has_release_serial("zork1_hints.z5"));
    }

    #[test]
    fn is_hint_sidecar_recognizes_slag_and_excludes_solid_gold() {
        // SLAG + user hint files are sidecars.
        assert!(is_hint_sidecar("deadlineinv.z5"));
        assert!(is_hint_sidecar("stuga_hints.z5"));
        assert!(is_hint_sidecar("zork1-invisiclues.z5"));
        assert!(is_hint_sidecar("zork1_hints.z5"));
        // The story itself is not a sidecar.
        assert!(!is_hint_sidecar("zork1.z5"));
        // Solid Gold full games (with release/serial) are not sidecars.
        assert!(!is_hint_sidecar("zork1-invclues-r52-s871125.z5"));
        assert!(!is_hint_sidecar("enchanter-r29-s860820.z3"));
    }

    #[test]
    fn hint_game_key_curated_and_derived() {
        assert_eq!(hint_game_key("deadlineinv.z5").as_deref(), Some("deadline"));
        assert_eq!(hint_game_key("hhgginv.z5").as_deref(), Some("hitchhiker"));
        assert_eq!(hint_game_key("lgopinv.z5").as_deref(), Some("leather"));
        // Curated must beat the (wrong) derived `enchan`.
        assert_eq!(hint_game_key("enchaninv.z5").as_deref(), Some("enchanter"));
        assert_eq!(hint_game_key("zork1_hints.z5").as_deref(), Some("zork1"));
        assert_eq!(hint_game_key("stuga_hints.z5").as_deref(), Some("stuga"));
    }

    #[test]
    fn hint_matches_story_associates_by_key() {
        assert!(hint_matches_story(
            "hhgginv.z5",
            "The Hitchhiker's Guide to the Galaxy"
        ));
        assert!(hint_matches_story("hhgginv.z5", "hitchhiker-r59-s851108"));
        assert!(hint_matches_story("deadlineinv.z5", "deadline-r27-s851006"));
        assert!(!hint_matches_story("deadlineinv.z5", "zork1-r88-s840726"));
    }

    /// The seven SLAG games added beyond the Phase-1 table must resolve.
    #[test]
    fn slag_new_entries_resolve() {
        for (file, key) in [
            ("suspendedinv.z5", "suspended"),
            ("trininv.z5", "trinity"),
            ("wishbrinv.z5", "wishbringer"),
            ("zork1inv.z5", "zork1"),
            ("zork2inv.z5", "zork2"),
            ("zork3inv.z5", "zork3"),
            ("ztuuinv.z5", "ztuu"),
        ] {
            assert_eq!(hint_game_key(file).as_deref(), Some(key), "{file}");
            assert!(is_hint_sidecar(file), "{file} must be a sidecar");
        }
    }

    /// A locally-present `*izm.z5` file (no `inv` suffix, no keyword) is only
    /// recognised via the curated izm table — and then hides/associates like SLAG.
    #[test]
    fn izm_local_files_are_detected_and_keyed() {
        assert!(is_invisiclues_name("deadlizm.z5"));
        assert!(is_hint_sidecar("deadlizm.z5"));
        assert_eq!(hint_game_key("deadlizm.z5").as_deref(), Some("deadline"));
        assert_eq!(hint_game_key("witnizm.z5").as_deref(), Some("witness"));
        assert_eq!(hint_game_key("bzorkizm.z5").as_deref(), Some("beyondzork"));
        assert!(hint_matches_story("witnizm.z5", "The Witness"));
        // A bare `izm` suffix that isn't in the table is NOT a hint file.
        assert!(!is_invisiclues_name("mechanizm.z5"));
    }

    /// An empty IFID means "identity says nothing", so every case here exercises
    /// the filename/title fallback — the common path for the many stories that
    /// are only ever bare files, which must keep working (SQ-0767).
    #[test]
    fn hint_download_prefers_slag_then_izm() {
        // A SLAG-covered game: prefer the live IF Archive file.
        let d = hint_download_for("", "deadline", "Deadline").expect("deadline has a hint");
        assert_eq!(d.filename, "deadlineinv.z5");
        assert!(d.url.contains("ifarchive.org/if-archive/solutions/slag/deadlineinv.z5"), "{}", d.url);

        // A game only the izm set covers: fall back to the Internet Archive.
        let w = hint_download_for("", "witness", "The Witness").expect("witness has an izm hint");
        assert_eq!(w.filename, "witnizm.z5");
        assert!(w.url.contains("web.archive.org"), "{}", w.url);
        assert!(w.url.ends_with("witnizm.z5"), "{}", w.url);

        // Match on title when the stem is opaque.
        assert!(hint_download_for("", "hhgg", "The Hitchhiker's Guide to the Galaxy").is_some());

        // A game with no hint anywhere.
        assert!(hint_download_for("", "adventure", "Colossal Cave").is_none());
    }

    /// Beyond Zork keys on "beyond", never bare "zork", so it must not collide
    /// with zork1/2/3 (and vice-versa).
    #[test]
    fn hint_download_zork_variants_dont_collide() {
        assert_eq!(hint_download_for("", "zork1", "Zork I").unwrap().filename, "zork1inv.z5");
        assert_eq!(hint_download_for("", "beyondzork", "Beyond Zork").unwrap().filename, "bzorkizm.z5");
        // Canonical multi-word/underscored names still match via normalisation.
        assert_eq!(hint_download_for("", "beyond_zork", "").unwrap().filename, "bzorkizm.z5");
        assert_eq!(hint_download_for("", "", "Beyond Zork").unwrap().filename, "bzorkizm.z5");
        assert_eq!(hint_download_for("", "zork0", "Zork Zero").unwrap().filename, "zork0izm.z5");
    }

    /// Regression: a stray common word in a title must not match a compound-word
    /// game key. "Brain Guzzlers from Beyond" contains "beyond" but is not
    /// Beyond Zork, so it gets no hint (badge stays dark).
    #[test]
    fn hint_download_rejects_stray_word_match() {
        assert!(hint_download_for("", "Brain_Guzzlers_from_Beyond!.gblorb", "Brain Guzzlers from Beyond!").is_none());
        assert!(!hint_matches_story("bzorkizm.z5", "Brain Guzzlers from Beyond!"));
    }

    // ── SQ-0767: identity, not filename ─────────────────────────────────────

    /// The four Amiga floppies the bug was reported on, by the release+serial
    /// their disks actually carry (pinned in `tests/real_media_releases.rs`).
    /// The container is named for the box, so its stem and the title derived
    /// from it contain no catalog key — only the IFID can say which game it is.
    #[test]
    fn a_disk_image_finds_its_invisiclues_by_identity_not_filename() {
        let cases = [
            ("ZCODE-88-840726-A129", "Zork I - The Great Underground Empire", "zork1inv.z5"),
            ("ZCODE-48-840904-D899", "Zork II - The Wizard of Frobozz", "zork2inv.z5"),
            ("ZCODE-17-840727-2E7A", "Zork III - The Dungeon Master", "zork3inv.z5"),
            ("ZCODE-16-970828-1185", "Zork - The Undiscovered Underground", "ztuuinv.z5"),
            ("ZCODE-366-890323-C5CD", "Zork Zero - The Revenge of Megaboz", "zork0izm.z5"),
        ];
        for (ifid, stem, want) in cases {
            // The title the picker shows for a container is the canonical one,
            // which spells the number the way the box does — also no key.
            let title = stem.replace(" - ", ": ");
            assert!(
                !normalize_ident(stem).contains("zork1") && !normalize_ident(&title).contains("ztuu"),
                "the premise: no catalog key is in the container's name ({stem})"
            );
            let dl = hint_download_for(ifid, stem, &title)
                .unwrap_or_else(|| panic!("{stem}: identity {ifid} must find its InvisiClues"));
            assert_eq!(dl.filename, want, "{stem}");
        }
    }

    /// The identity is authoritative and exclusive: knowing a build is Zork II
    /// must not let Zork I's or Beyond Zork's clues match it.
    #[test]
    fn an_identified_story_matches_only_its_own_key() {
        let dl = hint_download_for("ZCODE-48-840904-D899", "zork1", "Beyond Zork").unwrap();
        assert_eq!(dl.filename, "zork2inv.z5", "identity beats a misleading stem AND title");
    }

    /// A local sidecar sitting beside a disk image is associated by identity —
    /// comparing the two filenames cannot do it.
    #[test]
    fn a_local_sidecar_is_associated_with_a_disk_image_by_identity() {
        assert!(hint_matches_identity("zork1inv.z5", "ZCODE-88-840726-A129"));
        assert!(hint_matches_identity("zork3izm.z5", "ZCODE-17-840727-2E7A"));
        // Beyond Zork's title spells its own key, so the canonical title path
        // (no TITLE_KEYS row needed) carries it.
        assert!(hint_matches_identity("bzorkizm.z5", "ZCODE-57-871221-C5AD"));
        // Wrong game, and an unknown identity, both stay unmatched.
        assert!(!hint_matches_identity("zork2inv.z5", "ZCODE-88-840726-A129"));
        assert!(!hint_matches_identity("zork1inv.z5", "ZCODE-1-000000-0000"));
    }

    /// `Mini-Zork I` and `Zork I Demo` are separate entries in the title table
    /// and separate games; matching the WHOLE canonical title keeps Zork I's
    /// clues off them.
    #[test]
    fn title_keys_match_whole_titles_so_near_namesakes_dont_collide() {
        assert_eq!(identity_hint_key("ZCODE-88-840726-A129"), Some("zork1"));
        assert_eq!(identity_hint_key("ZCODE-34-871124-0000"), None, "Mini-Zork I");
        assert_eq!(identity_hint_key("ZCODE-15-840330-0000"), None, "Zork I Demo");
    }

    #[test]
    fn narrow_screen_warning_matches_any_width() {
        // The real banner, at two different advertised widths.
        assert!(is_narrow_screen_warning(
            "WARNING: Your screen is only 80 characters wide. The names of some menu items contain up to 512 characters"
        ));
        assert!(is_narrow_screen_warning("your screen is only 255 characters wide."));
        // Ordinary clue/menu text must not match.
        assert!(!is_narrow_screen_warning("Type the number of the topic you want a hint for."));
        assert!(!is_narrow_screen_warning(""));
    }

    #[test]
    fn story_supports_hint_detects_dictionary_word() {
        assert!(story_supports_hint(["look", "hint", "take"].map(String::from)));
        assert!(!story_supports_hint(["look", "take"].map(String::from)));
    }

    #[test]
    fn hint_index_round_trips() {
        let dir = crate::scratch_dir("hintidx");
        save_hint_assoc(&dir, "ZCODE-1", std::path::Path::new("/x/h.z5")).unwrap();
        let idx = load_hint_index(&dir);
        assert_eq!(idx.get("ZCODE-1"), Some(std::path::PathBuf::from("/x/h.z5")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_story_bytes_handles_raw_and_zip() {
        use std::io::Write as _;

        let base = crate::scratch_dir("lsb");

        // A real v5 header, distinguished by a byte the checks do not read, so
        // "the same bytes came back" stays the thing being asserted. Twelve
        // bytes starting `5` used to be enough; since SQ-0889 a story has to
        // look like one.
        let mut story_bytes = sample_zcode(5);
        story_bytes.extend_from_slice(&[1, 2, 3, 4]);

        // --- raw path: a plain .z5 file, no zip magic ---
        let raw_path = base.join("game.z5");
        std::fs::write(&raw_path, &story_bytes).unwrap();
        let loaded_raw = load_story_bytes(&raw_path).expect("raw load");
        assert_eq!(loaded_raw, story_bytes, "raw bytes must be returned as-is");

        // --- zip path: pack the same bytes as "game.z5" inside a zip ---
        let zip_path = base.join("game.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zw.start_file("game.z5", opts).unwrap();
            zw.write_all(&story_bytes).unwrap();
            zw.finish().unwrap();
        }
        let loaded_zip = load_story_bytes(&zip_path).expect("zip load");
        assert_eq!(loaded_zip, story_bytes, "zip entry bytes must match the original");

        let _ = std::fs::remove_dir_all(&base);
    }

    // Build a minimal Blorb wrapping a single Exec chunk of the given type.
    // Mirrors the blorb crate's builder shape: FORM/IFRS + RIdx + Exec/0 chunk.
    fn make_blorb(exec_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        make_blorb_indexed(b"Exec", exec_type, payload)
    }

    // The same builder with the RIdx usage spelled out, so a RESOURCE-only
    // Blorb — one `Pict`, no executable — can be built too. That is the shape
    // `Journey.blb` has, and the one SQ-1085's zip tier has to find.
    fn make_blorb_indexed(usage: &[u8; 4], chunk_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        make_blorb_multi(&[(usage, chunk_type, payload)])
    }

    // …and with several index entries, which is how a `.zblorb` carries its
    // story and its pictures in one file.
    fn make_blorb_multi(entries: &[(&[u8; 4], &[u8; 4], &[u8])]) -> Vec<u8> {
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
        // RIdx first; the resource chunks follow it in order. Offsets are from
        // the start of the FORM: 12 bytes of FORM+IFRS header, 8 of RIdx chunk
        // header, then the index itself (padded to even).
        let ridx_data_len = 4 + 12 * entries.len();
        let mut off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        let mut body = Vec::new();
        for (n, (usage, chunk_type, payload)) in entries.iter().enumerate() {
            ridx.extend_from_slice(*usage);
            ridx.extend_from_slice(&(n as u32).to_be_bytes());
            ridx.extend_from_slice(&(off as u32).to_be_bytes());
            let c = chunk(chunk_type, payload);
            off += c.len();
            body.extend_from_slice(&c);
        }
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    #[test]
    fn load_story_bytes_extracts_zblorb_executable() {
        let base = crate::scratch_dir("zblorb");

        let zcode = b"ZCODE-PAYLOAD";
        let path = base.join("game.zblorb");
        std::fs::write(&path, make_blorb(b"ZCOD", zcode)).unwrap();
        let out = load_story_bytes(&path).expect("zblorb load");
        assert_eq!(out, zcode);

        let _ = std::fs::remove_dir_all(&base);
    }

    // ── A zip is a volume, not a wrapper around one file (SQ-1085) ───────────

    /// A scratch directory of this test's own, unique per CALL so two callers who
    /// happen to pass the same `tag` still get two directories (SQ-1163).
    fn scratch(tag: &str) -> std::path::PathBuf {
        crate::scratch_dir(tag)
    }

    /// Write a zip at `path` holding each `(entry name, bytes)` in order,
    /// STORED so what comes back out is what went in.
    fn write_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
        use std::io::Write as _;
        let file = std::fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            zw.start_file(*name, opts).unwrap();
            zw.write_all(bytes).unwrap();
        }
        zw.finish().unwrap();
    }

    /// A Scott Adams database small enough to inline — the header the loader's
    /// own sniff reads, and nothing more.
    fn sample_scott() -> Vec<u8> {
        b"32767 1 0 1 2 6 1 0 3 125 0 1\n".to_vec()
    }

    /// **The whole of limit one**: the zip branch named three extensions, so
    /// every other format lanthorn opens perfectly well as a loose file was
    /// refused inside a zip — `.z6` above all, the one format whose point is
    /// that it ships artwork.
    ///
    /// Classified by CONTENT, so the name each entry is stored under is not what
    /// is being asserted: the same bytes under a nonsense extension still open.
    #[test]
    fn a_zip_yields_every_format_a_loose_file_does() {
        let base = scratch("zip-formats");

        // (stored name, bytes, the engine it must come back as)
        type Case = (&'static str, Vec<u8>, fn(&LoadedStory) -> bool);
        let cases: Vec<Case> = vec![
            ("journey.z6", sample_zcode(6), |l| matches!(l, LoadedStory::ZCode(_))),
            ("trinity.z4", sample_zcode(4), |l| matches!(l, LoadedStory::ZCode(_))),
            ("shogun.z7", sample_zcode(7), |l| matches!(l, LoadedStory::ZCode(_))),
            ("cm.ulx", b"Glul-and-then-some".to_vec(), |l| matches!(l, LoadedStory::Glulx(_))),
            ("adventureland.dat", sample_scott(), |l| matches!(l, LoadedStory::Scott(_))),
            ("curses.zblorb", make_blorb(b"ZCOD", b"ZCODE-PAYLOAD"), |l| {
                matches!(l, LoadedStory::ZCode(_))
            }),
            ("glulx.gblorb", make_blorb(b"GLUL", b"GLULPAYLOAD"), |l| {
                matches!(l, LoadedStory::Glulx(_))
            }),
            // Content, not spelling: a v6 story stored under a name that claims
            // nothing at all.
            ("STORY.DATA", sample_zcode(6), |l| matches!(l, LoadedStory::ZCode(_))),
        ];

        for (name, bytes, is_right) in cases {
            let zip_path = base.join(format!("{}.zip", name.replace('.', "_")));
            write_zip(&zip_path, &[(name, bytes)]);
            let loaded = load_story(&zip_path)
                .unwrap_or_else(|e| panic!("a zip holding {name} must open: {e}"));
            assert!(is_right(&loaded), "{name} came out of the zip as the wrong engine");
        }

        let _ = std::fs::remove_dir_all(&base);
    }

    /// A story nested in a folder inside the zip — how every download from the
    /// IF Archive is actually laid out — and the non-story entries beside it.
    #[test]
    fn a_zip_finds_the_story_past_a_folder_and_its_readme() {
        let base = scratch("zip-nested");
        let story = sample_zcode(5);
        let zip_path = base.join("zork1.zip");
        write_zip(
            &zip_path,
            &[
                ("Zork I/readme.txt", b"Zork I, from the IF Archive.\n".to_vec()),
                ("Zork I/cover.png", vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13]),
                ("Zork I/zork1.z5", story.clone()),
            ],
        );
        assert_eq!(load_story(&zip_path).unwrap(), LoadedStory::ZCode(story));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// **The whole of limit two**: the zip branch handed back `None` where the
    /// disk-image branch hands back the medium, so a `Journey.blb` in the same
    /// zip as `journey.z6` was never opened and a zipped Version 6 game drew
    /// nothing.
    #[test]
    fn a_blorb_inside_the_zip_is_the_storys_resource_source() {
        let base = scratch("zip-blorb");
        let zip_path = base.join("journey.zip");
        write_zip(
            &zip_path,
            &[
                ("journey.z6", sample_zcode(6)),
                ("Journey.blb", make_blorb_indexed(b"Pict", b"PNG ", b"pixels")),
            ],
        );

        // The story still opens…
        assert!(matches!(load_story(&zip_path), Ok(LoadedStory::ZCode(_))));
        // …and its artwork comes with it.
        let found = crate::graphics::resource_blorb(&zip_path).found;
        let (blorb, path) = found.expect("the Blorb in the zip is the story's resource source");
        assert_eq!(blorb.resources().len(), 1, "the Pict resource is indexed");
        // `Path::ends_with`, not `to_string_lossy().ends_with`: the former compares
        // whole COMPONENTS and reads `/` as a separator on Windows too, while the
        // latter compares bytes and so demanded a separator the platform does not
        // write. `named()` builds this with `PathBuf::join`, which yields `\` on
        // Windows — correct for a path a Windows player reads, and the reason the
        // string form was red on CI and green on every developer machine.
        assert!(
            path.ends_with("journey.zip/Journey.blb"),
            "the display path names the entry inside the archive: {}",
            path.display(),
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// A `.zblorb` carrying the story AND its resources is its own source, the
    /// same way a loose one is `blorb::resolve_resource_blorb`'s tier 1.
    #[test]
    fn a_zipped_self_blorb_is_its_own_resource_source() {
        let base = scratch("zip-selfblorb");
        let zip_path = base.join("curses.zip");
        let both =
            make_blorb_multi(&[(b"Exec", b"ZCOD", b"ZCODE-PAYLOAD"), (b"Pict", b"PNG ", b"pix")]);
        write_zip(&zip_path, &[("curses.zblorb", both)]);
        assert_eq!(load_story(&zip_path).unwrap(), LoadedStory::ZCode(b"ZCODE-PAYLOAD".to_vec()));
        assert!(
            crate::graphics::resource_blorb(&zip_path).found.is_some(),
            "a zipped self-blorb must reach its own resources",
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Two games' archives in one zip: ranked by how much of the zip's own stem
    /// they share, and REFUSED on a tie rather than drawing the other game's
    /// plates — `blorb::resolve_resource_blorb`'s rule, over entries.
    #[test]
    fn two_blorbs_in_one_zip_are_ranked_by_stem_and_refused_on_a_tie() {
        let base = scratch("zip-twoblorbs");
        let pict = |tag: &[u8]| make_blorb_indexed(b"Pict", b"PNG ", tag);

        // Named for one of them: that one wins.
        let named = base.join("journey.zip");
        write_zip(
            &named,
            &[
                ("journey.z6", sample_zcode(6)),
                ("journey.blb", pict(b"journeys")),
                ("arthur.blb", pict(b"arthurs")),
            ],
        );
        let (_, path) = crate::graphics::resource_blorb(&named).found.expect("the stem decides");
        assert!(path.to_string_lossy().ends_with("journey.blb"), "{}", path.display());

        // Named for neither: nothing is drawn, rather than the first one found.
        let anon = base.join("infocom-graphics.zip");
        write_zip(
            &anon,
            &[
                ("journey.z6", sample_zcode(6)),
                ("journey.blb", pict(b"journeys")),
                ("arthur.blb", pict(b"arthurs")),
            ],
        );
        assert!(
            crate::graphics::resource_blorb(&anon).found.is_none(),
            "an ambiguous pair must draw nothing, not the wrong game's plates",
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// The clues a download packs beside the game are found in the game's own
    /// container, not only in a sibling zip.
    #[test]
    fn hints_are_found_inside_the_storys_own_zip() {
        let base = scratch("zip-hints");
        let zip_path = base.join("deadline.zip");
        write_zip(
            &zip_path,
            &[("deadline.z3", sample_zcode(3)), ("deadlineinv.z5", sample_zcode(5))],
        );
        let got = resolve_hint_source(&zip_path, "ZCODE-NOTHING", &load_hint_index(&base));
        assert_eq!(
            got,
            HintResolution::ZipEntry {
                zip_path: zip_path.clone(),
                entry: "deadlineinv.z5".to_string()
            },
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// **The cap that rejects a zip has to cover what lanthorn RUNS.** SQ-1085
    /// widened the classifier to accept Glulx and Blorb by content while
    /// [`MAX_ZIP_ENTRY`] still stood at the 4 MiB a Z-code-only branch needed —
    /// so every real Glulx game was skipped WITHOUT being inflated, and the
    /// launch reported "no story file inside the zip", which reads as a
    /// legitimately empty archive. Half the fix did not work, silently.
    ///
    /// A synthetic Glulx Blorb past the old cap, because the real ones
    /// (`Kerkerkruip.gblorb`, `CounterfeitMonkey-11.gblorb`) live in gitignored
    /// `stories/` and a case built on those goes vacuous in CI — which is how
    /// this would have escaped a second time.
    #[test]
    fn a_glulx_game_past_the_old_four_mib_cap_still_opens_out_of_a_zip() {
        const OLD_CAP: usize = 4 * 1024 * 1024;
        let base = scratch("zip-bigglulx");

        // Six MiB of payload: half again the cap that used to reject it, and
        // small enough that the deflate costs nothing.
        let payload = vec![0x47u8; 6 * 1024 * 1024];
        let gblorb = make_blorb(b"GLUL", &payload);
        assert!(gblorb.len() > OLD_CAP, "the fixture must cross the old cap");

        let zip_path = base.join("kerkerkruip.zip");
        {
            use std::io::Write as _;
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("kerkerkruip.gblorb", opts).unwrap();
            zw.write_all(&gblorb).unwrap();
            zw.finish().unwrap();
        }

        match load_story(&zip_path) {
            Ok(LoadedStory::Glulx(bytes)) => assert_eq!(
                bytes.len(),
                payload.len(),
                "the whole executable comes back, not a truncated prefix",
            ),
            other => panic!("a zipped Glulx game past the old cap must open: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&base);
    }

    /// **A zip is a convenience for what somebody downloaded; it is not a
    /// lanthorn container.** The `.lanthorn` archive is that, and it is a zip
    /// too — so the one thing this must never do is open one as a game. It
    /// carries a map, a Quetzal save and some PNGs, and not one of them is a
    /// story; the refusal says so instead of running a save file.
    #[test]
    fn a_lanthorn_archive_is_not_a_story_container() {
        let base = scratch("zip-archive");
        let path = base.join("ZCODE-12345.lanthorn");
        write_zip(
            &path,
            &[
                ("meta.json", br#"{"name":"Zork I","turns":42}"#.to_vec()),
                ("map.json", br#"{"rooms":[]}"#.to_vec()),
                ("game.qzl", b"FORMIFZSjunk-quetzal-bytes".to_vec()),
                ("pictures/win1.png", vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10]),
            ],
        );
        let err = load_story(&path).expect_err("a save archive is not a game");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("no story file inside the zip"), "{err}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// SQ-0660: a zip entry that inflates beyond the cap must error instead of
    /// allocating unbounded memory; a small entry still extracts.
    #[test]
    fn read_zip_entry_caps_a_huge_inflated_entry() {
        use std::io::Write as _;

        let base = crate::scratch_dir("zipcap");

        // A deflated entry of MAX_ZIP_ENTRY + 128 KiB of zeros: tiny on disk,
        // huge inflated — exactly the zip-bomb shape the cap exists for.
        let zip_path = base.join("bomb.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("huge.z5", opts).unwrap();
            let chunk = vec![0u8; 64 * 1024];
            for _ in 0..(MAX_ZIP_ENTRY / chunk.len() as u64 + 2) {
                zw.write_all(&chunk).unwrap();
            }
            zw.finish().unwrap();
        }
        let err = read_zip_entry(&zip_path, |n| n.ends_with(".z5"))
            .expect_err("an over-cap entry must error");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("huge.z5"), "error names the entry: {err}");

        // A normal-sized entry is unaffected.
        let ok_path = base.join("ok.zip");
        {
            let file = std::fs::File::create(&ok_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("small.z5", opts).unwrap();
            zw.write_all(&[5u8; 128]).unwrap();
            zw.finish().unwrap();
        }
        let bytes = read_zip_entry(&ok_path, |n| n.ends_with(".z5")).unwrap().unwrap();
        assert_eq!(bytes, vec![5u8; 128]);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// SQ-0629: a blorb claiming a Scott Adams (`SAAI`) executable only loads
    /// when its payload passes the same `looks_like_scott` sniff the raw-`.dat`
    /// path uses — arbitrary bytes must never reach scott's loader.
    #[test]
    fn extract_story_gates_a_blorb_saai_payload_behind_the_scott_sniff() {
        // Binary junk behind an SAAI exec chunk: rejected.
        let hostile = make_blorb(b"SAAI", &[0xFF, 0x00, 0xC3, 0x28, 0x9A, 0x01]);
        let err = extract_story(hostile).expect_err("non-Scott SAAI payload must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // Text that fails the header sniff behind SAAI: also rejected.
        let not_scott = make_blorb(b"SAAI", b"just some readme text, not a database");
        assert!(extract_story(not_scott).is_err());

        // A genuine Scott header (12 sane ints) still loads.
        const MINI: &str = "\n32767 1 0 1 2 6 1 0 3 125 0 1\n150 1 0 0 0 0 0 0\n";
        let genuine = make_blorb(b"SAAI", MINI.as_bytes());
        assert_eq!(
            extract_story(genuine).unwrap(),
            LoadedStory::Scott(MINI.as_bytes().to_vec())
        );
    }

    #[test]
    fn extract_story_classifies_engine() {
        // ZCOD Blorb → ZCode(payload).
        let zblorb = make_blorb(b"ZCOD", b"ZCODE");
        assert_eq!(extract_story(zblorb).unwrap(), LoadedStory::ZCode(b"ZCODE".to_vec()));
        // GLUL Blorb → Glulx(payload).
        let gblorb = make_blorb(b"GLUL", b"GLULX");
        assert_eq!(extract_story(gblorb).unwrap(), LoadedStory::Glulx(b"GLULX".to_vec()));
        // Raw Z-code → ZCode. This used to be eight bytes, `[5, 0, 0, …]`, and
        // that it sufficed was the whole defect (SQ-0889): a version byte was
        // the entire claim a file had to make. It now has to carry a header.
        let raw_z = sample_zcode(5);
        assert_eq!(extract_story(raw_z.clone()).unwrap(), LoadedStory::ZCode(raw_z));
        // Raw .ulx (Glul magic) → Glulx pass-through.
        let mut raw_ulx = b"Glul".to_vec();
        raw_ulx.extend_from_slice(&[0, 3, 1, 2]);
        assert_eq!(extract_story(raw_ulx.clone()).unwrap(), LoadedStory::Glulx(raw_ulx));
    }

    /// A structurally valid story image, small but real: a memory map that
    /// obeys ZMSD §1.1 (dynamic memory below `$0e`, the writable object and
    /// global tables inside it, the dictionary in static memory) and a serial
    /// of six printable bytes.
    fn sample_zcode(version: u8) -> Vec<u8> {
        let mut b = vec![0u8; 0x400];
        b[0x00] = version;
        b[0x04..0x06].copy_from_slice(&0x0100u16.to_be_bytes()); // high memory base
        b[0x06..0x08].copy_from_slice(&0x0100u16.to_be_bytes()); // initial PC
        b[0x08..0x0a].copy_from_slice(&0x0200u16.to_be_bytes()); // dictionary
        b[0x0a..0x0c].copy_from_slice(&0x0040u16.to_be_bytes()); // object table
        b[0x0c..0x0e].copy_from_slice(&0x0080u16.to_be_bytes()); // globals
        b[0x0e..0x10].copy_from_slice(&0x0100u16.to_be_bytes()); // static memory base
        b[0x12..0x18].copy_from_slice(b"890101"); // serial
        b
    }

    /// **A version byte is no longer a story** (SQ-0889).
    ///
    /// The reported shape, in miniature: a container whose first byte is a legal
    /// Z-machine version and whose remaining 4,095 bytes are nothing of the
    /// kind. It used to load, run, print nothing and exit 0.
    #[test]
    fn extract_story_refuses_a_container_wearing_a_version_byte() {
        let mut junk = vec![0xa5u8; 4096];
        junk[0] = 6;
        let err = extract_story(junk).expect_err("a version byte is not a header");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("not a story file"), "{err}");

        // …and every version the Z-machine runs still loads when the header is
        // real, so the gate is on the header and not on the version byte.
        for version in 3..=8u8 {
            assert!(
                matches!(extract_story(sample_zcode(version)), Ok(LoadedStory::ZCode(_))),
                "a real v{version} header must still load"
            );
        }
    }

    #[test]
    fn load_story_routes_gblorb_and_ulx() {
        let base = crate::scratch_dir("route");

        // A .gblorb (GLUL) routes to Glulx.
        let gpath = base.join("game.gblorb");
        std::fs::write(&gpath, make_blorb(b"GLUL", b"GLULPAYLOAD")).unwrap();
        assert_eq!(load_story(&gpath).unwrap(), LoadedStory::Glulx(b"GLULPAYLOAD".to_vec()));

        // A raw .ulx routes to Glulx by its magic.
        let upath = base.join("game.ulx");
        let mut ulx = b"Glul".to_vec();
        ulx.extend_from_slice(&[0, 3, 1, 2, 9, 9]);
        std::fs::write(&upath, &ulx).unwrap();
        assert_eq!(load_story(&upath).unwrap(), LoadedStory::Glulx(ulx));

        // A .zblorb (ZCOD) still routes to Z-code.
        let zpath = base.join("game.zblorb");
        std::fs::write(&zpath, make_blorb(b"ZCOD", b"ZP")).unwrap();
        assert_eq!(load_story(&zpath).unwrap(), LoadedStory::ZCode(b"ZP".to_vec()));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn load_story_bytes_rejects_glulx_blorb() {
        let base = crate::scratch_dir("gblorb");

        let path = base.join("game.gblorb");
        std::fs::write(&path, make_blorb(b"GLUL", b"GLULPAYLOAD")).unwrap();
        let err = load_story_bytes(&path).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("glulx"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn detects_scott_dat() {
        let dat = include_bytes!("../../scott/tests/tiny_cave.dat").to_vec();
        match extract_story(dat).unwrap() {
            LoadedStory::Scott(_) => {}
            o => panic!("{o:?}"),
        }
    }

    /// **Z-code is claimed, not defaulted to** (SQ-0889).
    ///
    /// This case used to be called `zcode_still_defaults` and asserted that
    /// `[3, 0, 0, 0, 0, 0, 0, 0]` — a version byte and seven zeroes — came back
    /// as a story. That was the rule, and it is the rule this quest removed: a
    /// Z-machine image now proves itself by its header exactly as Blorb, Glulx
    /// and Scott Adams prove themselves by theirs.
    #[test]
    fn zcode_is_claimed_by_a_header_not_defaulted_to() {
        let err = extract_story(vec![3, 0, 0, 0, 0, 0, 0, 0])
            .expect_err("eight bytes are not a story");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(matches!(extract_story(sample_zcode(3)).unwrap(), LoadedStory::ZCode(_)));
    }

    #[test]
    fn resolve_finds_sibling_then_asks() {
        // Set up a temp dir with a story file and a sibling hints file.
        let dir = crate::scratch_dir("resolve");

        let story = dir.join("story.z5");
        let hints = dir.join("story.hints.z5");
        std::fs::write(&story, b"fake story").unwrap();
        std::fs::write(&hints, b"fake hints").unwrap();

        let empty_index = HintIndex { map: HashMap::new() };

        // With sibling hints file present: should return File(hints).
        let result = resolve_hint_source(&story, "ZCODE-TEST", &empty_index);
        assert_eq!(result, HintResolution::File(hints));

        // Without any hint sibling: should return AskUser.
        let no_hints_dir = crate::scratch_dir("resolve-nosibling");
        let story2 = no_hints_dir.join("story.z5");
        std::fs::write(&story2, b"fake story").unwrap();

        let result2 = resolve_hint_source(&story2, "ZCODE-TEST", &empty_index);
        assert_eq!(result2, HintResolution::AskUser);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&no_hints_dir);
    }

    // Create a fresh temp dir with the given files (empty contents), returning it.
    fn scratch_dir(tag: &str, files: &[&str]) -> PathBuf {
        let dir = crate::scratch_dir(tag);
        for f in files {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn resolve_is_story_aware_in_multi_story_dir() {
        let dir = scratch_dir(
            "resolve-multistory",
            &["zork1.z5", "zork1_hints.z5", "zork2.z5", "zork2-invisiclues.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let r1 = resolve_hint_source(&dir.join("zork1.z5"), "IFID-1", &empty);
        assert_eq!(r1, HintResolution::File(dir.join("zork1_hints.z5")));

        let r2 = resolve_hint_source(&dir.join("zork2.z5"), "IFID-2", &empty);
        assert_eq!(r2, HintResolution::File(dir.join("zork2-invisiclues.z5")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_uses_lone_generic() {
        let dir = scratch_dir("resolve-lonegeneric", &["story.z5", "invisiclues.z5"]);
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("story.z5"), "IFID", &empty);
        assert_eq!(r, HintResolution::File(dir.join("invisiclues.z5")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_ambiguous_generics_asks_user() {
        let dir = scratch_dir("resolve-ambiguous", &["story.z5", "hintsA.z5", "hintsB.z5"]);
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("story.z5"), "IFID", &empty);
        assert_eq!(r, HintResolution::AskUser);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_story_stem_beats_generic() {
        let dir = scratch_dir(
            "resolve-stembeats",
            &["zork1.z5", "zork1_hints.z5", "invisiclues.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("zork1.z5"), "IFID", &empty);
        assert_eq!(r, HintResolution::File(dir.join("zork1_hints.z5")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_multistory_is_deterministic() {
        let dir = scratch_dir(
            "resolve-determinism",
            &["zork1.z5", "zork1_hints.z5", "zork2.z5", "zork2-invisiclues.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let expected = HintResolution::File(dir.join("zork1_hints.z5"));
        for _ in 0..8 {
            let r = resolve_hint_source(&dir.join("zork1.z5"), "IFID-1", &empty);
            assert_eq!(r, expected);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_slag_hint_via_game_key() {
        // Curated SLAG hint (no keyword, no stem prefix) resolves via game key.
        let dir = scratch_dir(
            "resolve-slag",
            &["deadline-r27-s851006.z3", "deadlineinv.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("deadline-r27-s851006.z3"), "IFID", &empty);
        assert_eq!(r, HintResolution::File(dir.join("deadlineinv.z5")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_skips_solid_gold_sibling() {
        // A Solid Gold full game (with -rNN-sNNNNNN) is not a hint sidecar, so
        // it is never offered as another story's hints.
        let dir = scratch_dir(
            "resolve-solidgold",
            &["story.z5", "zork1-invclues-r52-s871125.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("story.z5"), "IFID", &empty);
        assert_eq!(r, HintResolution::AskUser);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stem_matches_story_is_case_insensitive_prefix() {
        assert!(stem_matches_story("zork1", "zork1_hints.z5"));
        assert!(stem_matches_story("zork1", "Zork1.hints.z5"));
        assert!(stem_matches_story("zork1", "ZORK1-invisiclues.z5"));
        assert!(!stem_matches_story("zork1", "zork2_hints.z5"));
    }

    #[test]
    fn stem_matches_story_requires_a_word_boundary() {
        // A bare prefix must not cross-wire: story `zork` ≠ `zork2_hints`.
        assert!(!stem_matches_story("zork", "zork2_hints.z5"));
        assert!(!stem_matches_story("zork1", "zork10_hints.z5"));
        // Exact stem, or prefix + separator, does match.
        assert!(stem_matches_story("zork", "zork_hints.z5"));
        assert!(stem_matches_story("zork", "zork.hints.z5"));
        assert!(stem_matches_story("zork", "zork-invisiclues.z5"));
        // An empty story stem never matches (pathological path).
        assert!(!stem_matches_story("", "invisiclues.z5"));
    }
}
