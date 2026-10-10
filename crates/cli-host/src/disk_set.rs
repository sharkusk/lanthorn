//! **A multi-disk release is one collection, not several disks** (SQ-0844).
//!
//! *The Lost Treasures of Infocom* ships as seven Apple II volumes, the Atari ST
//! compilations as nine, the DOS press as `floppy1.ima`…`floppy5.ima`. Every one
//! of those is a shelf of games that happens to have been cut across media, and
//! the browser had no idea: naming `disk1.img` opened whatever single story that
//! one image offered, and the twenty-nine games on its siblings were reachable
//! only by naming each sibling in turn.
//!
//! This module answers exactly one question — **which files are volumes of one
//! set** — from their names. What the answer is *used* for lives elsewhere:
//! `app::picker::StorySource` scans a set, `app::picker::scan_stories` folds a
//! set's duplicate builds together, and [`mount_at`] hands the set to
//! [`blorb::medium::MountedDisk::mount_set`] for both front-ends.
//!
//! # Why it is here and not in `app` (SQ-0874)
//!
//! It lived in `app` until the CLI needed it, and the CLI cannot depend on `app`.
//! `zvm-cli` opened a disk with [`blorb::medium::MountedDisk::mount`] — that is
//! `mount_set` with no companions — so **no multi-volume release opened in the
//! CLI at all**: *Trinity* played in the TUI and not at the prompt, and the
//! Apple II presses reported "no story file on this disk image" off a disk whose
//! game is simply on the next floppy.
//!
//! The alternative was a second copy of the rule in `zvm-cli`, which is exactly
//! what [`blorb::medium`]'s module doc exists to prevent: two front-ends with two
//! ideas of what a release is disagree eventually, and the disagreement is
//! invisible until a game goes missing from one of them. `cli-host` is the crate
//! both front-ends already share and it already depends on `blorb`, so the rule
//! moved down rather than being copied sideways — the same trade SQ-0850 made for
//! the per-game save key. `app::disk_set` is now a re-export of this module, so
//! there is one implementation and every existing call site still spells it the
//! way it always did.
//!
//! # The rule
//!
//! Two files are volumes of one set when they sit in one directory, share a
//! **disk-image extension**, and their stems are identical except at **one run
//! of decimal digits** whose values across the group are distinct, include `1`,
//! and are none of them greater than [`MAX_INDEX`]. Exactly one digit run may
//! qualify; a stem where two of them do forms no set at all.
//!
//! # …and the volume that names its own games (SQ-0961)
//!
//! "Identical except at one digit run" describes every set above and cannot
//! describe this one, a DiskCopy 4.2 press of *The Lost Treasures of Infocom*:
//!
//! ```text
//! The Lost Treasures of Infocom - Disk 1 - Beyond Zork, Lurking Horror.dc42
//! The Lost Treasures of Infocom - Disk 2 - Hitchhiker's, Infidel, Planetfall, …
//! The Lost Treasures of Infocom - Disk 3 - Ballyhoo, Deadline, Moonmist, …
//! The Lost Treasures of Infocom - Disk 4 - Enchanter, Sorcerer, Spellbreaker, …
//! The Lost Treasures of Infocom - Disk 5 - Zork Zero.dc42
//! ```
//!
//! The index is in the middle and there is **no common suffix at all**, because
//! each volume lists the games it carries. The tail is not noise a rule should
//! tolerate; it is different information on every platter, and a prefix-plus-
//! index-plus-suffix rule groups none of them.
//!
//! So the suffix is dropped from the key — **but only when the digit run is
//! introduced by a word that says it is a disk number** ([`DISK_WORDS`]). That
//! qualifier is the whole safety of it. Prefix alone would fold `Ultima 1.dsk`,
//! `Ultima 2 - Revenge.dsk`, `Ultima 3.dsk` into one release, which is the
//! false positive this module exists to refuse; `Disk 1 - …`, `Disk 2 - …` says
//! in so many words what the number means. Nothing already recognised changes:
//! a set with a common suffix agrees on it whether or not the key carries it.
//!
//! It does not loosen the numbering test either. Zork Zero's DOS presses spell
//! their volumes `(360K) (Disk 1)` and `(720K) (Disk 1)`, and the capacity sits
//! in the *prefix*, so they stay the two separate sets they are — while the
//! `{360, 720}` run itself is introduced by no disk word and is keyed, and
//! refused, exactly as before.
//!
//! # What it refuses, and why each one matters
//!
//! Recognition is deliberately conservative, because the two failure modes are
//! not symmetric. Missing a set costs the player a menu they can rebuild by
//! naming the other disks. A *false* set silently folds unrelated games into one
//! collection and, through the IFID dedupe, can hide a row. So:
//!
//! - **`adv01.dat` … `adv13.dat`** — thirteen unrelated Scott Adams games, and a
//!   textbook prefix-plus-index. `.dat` is not a disk-image extension, so they
//!   are thirteen games and never a set. This is the case that makes the
//!   extension test load-bearing rather than decorative.
//! - **Zork Zero's `(360K)` and `(720K)` DOS presses.** Both spell their disks
//!   `(Disk 1)`, `(Disk 2)`, so `360K (Disk 1)` and `720K (Disk 1)` differ at
//!   exactly one digit run — and that run's values are `{360, 720}`, which
//!   contain no `1` and exceed [`MAX_INDEX`]. Refused, and the two presses stay
//!   the two separate three- and two-disk sets they are. This is the corpus's
//!   sharpest false positive and the reason the index test is not merely
//!   "the values differ".
//! - **Years.** `(1993)`, `(1989)` and the `19` of `(19xx)` are constant within
//!   their sets so they never even vary; a run that varied only in years would
//!   fail the same `1`-and-[`MAX_INDEX`] test that stops `{360, 720}`.
//! - **`disk*.img` against `floppy*.ima`.** Different stem text *and* different
//!   extension: two sets, which is what they are.
//! - **Roman numerals** — `Zork I/II/III…adf` carry no digits at all.
//! - **A set whose disk 1 is absent.** Requiring the index to reach `1` is what
//!   makes `{360, 720}` fall over, and it is honest about the cost: hand this
//!   `floppy2.ima`…`floppy5.ima` with `floppy1.ima` deleted and it reports no
//!   set. The games are all still listed by an ordinary directory scan; only the
//!   dedupe and the launch-from-one-member menu are lost.
//!
//! # What it accepted the day a format learned a spelling
//!
//! **`shogun_s1.dsk`…`s5` and `zork_zero_1.dsk`…`4`.** These were refused for
//! one reason only — `.dsk` was not a spelling [`blorb::medium`] claimed.
//! SQ-0864 gave it to the ProDOS row (a 5.25-inch dump is a ProDOS volume with
//! its sectors in the drive's order), and **this module needed no change at
//! all**: the extension census is read off that table, so the two presses became
//! sets the same day the reader landed. That is the whole argument for
//! [`has_image_ext`] asking `blorb::medium` rather than keeping a list, and it
//! is now a measured one rather than a stated one.
//!
//! It also matters more here than for any set before it. The compilations are a
//! convenience — miss one and the games are still listed, disk by disk. These
//! two releases page **one story across every volume of the set**, so a set that
//! is not recognised is not an inconvenience; it is a game that cannot be
//! opened at all.
//!
//! # Why the filename and not the volume label
//!
//! Every mountable format here *could* be asked its volume name, and on the
//! Apple II press the answer is beautiful — `INFOCOM1`…`INFOCOM7`, inside the
//! disk where no rename can reach it. It is still the wrong signal, on measured
//! grounds:
//!
//! - **Nine of the corpus's volumes have no label at all.** Every
//!   `Infocom Compilation N (19xx)(-).st` reports `volume_name() == None`, so a
//!   label rule would cover the Apple II and DOS families and leave the entire
//!   Atari ST shelf — 39 of the 100 rows — ungrouped.
//! - **Labels are not unique.** Zork Zero's 360K and 720K DOS presses both label
//!   their first disk `ZORK0 1` and their second `ZORK0 2`. Grouping on the label
//!   merges the two presses, which is precisely the false positive the filename
//!   rule refuses.
//! - **It would cost a mount.** The name is the one thing already in hand.
//!
//! So the label is not consulted. It is a good corroborating signal and a bad
//! primary one, and a second signal that only sometimes applies is a second
//! answer waiting to disagree with the first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The largest disk number a set may carry. Real multi-disk releases are small —
/// the biggest here is nine — and the bound is what tells a *disk index* apart
/// from a year, a release number or a `360K`/`720K` capacity that happens to sit
/// in the same place in two filenames.
pub const MAX_INDEX: u64 = 32;

/// One piece of a stem: a run of decimal digits, or the literal text between
/// runs.
///
/// A numeric run keeps its **raw spelling** as well as its value. Constant runs
/// are compared as written, so `Disk 1 of 07` and `Disk 2 of 7` are not volumes
/// of one set, and a run too long to be a number cannot collide with a short one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Token {
    Text(String),
    /// The digits exactly as spelled.
    Num(String),
}

impl Token {
    /// A numeric run's value, or `None` when it is text or too long to be one.
    fn value(&self) -> Option<u64> {
        match self {
            Token::Text(_) => None,
            Token::Num(d) => d.parse().ok(),
        }
    }
}

/// Split a stem into alternating text and digit runs.
fn tokens(stem: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut digits = String::new();
    for ch in stem.chars() {
        if ch.is_ascii_digit() {
            if !text.is_empty() {
                out.push(Token::Text(std::mem::take(&mut text)));
            }
            digits.push(ch);
        } else {
            if !digits.is_empty() {
                out.push(Token::Num(std::mem::take(&mut digits)));
            }
            text.push(ch);
        }
    }
    if !text.is_empty() {
        out.push(Token::Text(text));
    }
    if !digits.is_empty() {
        out.push(Token::Num(digits));
    }
    out
}

/// Is this a spelling [`blorb::medium`] gives one of its disk formats?
///
/// A pre-filter and never evidence of a format — exactly as
/// [`blorb::medium::DiskImage::extensions`] requires. Here it is doing a
/// different job from the picker's: it is what keeps a numbered run of ordinary
/// **story files** from being read as a run of disks.
fn has_image_ext(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    blorb::medium::image_extensions().any(|e| e == ext)
}

/// The stem and lowercased extension a grouping key is built from.
fn parts(path: &Path) -> Option<(Vec<Token>, String)> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    let ext = path.extension().and_then(|e| e.to_str())?.to_ascii_lowercase();
    Some((tokens(stem), ext))
}

/// The words a release spells immediately before a volume's number.
///
/// Deliberately three, and deliberately the three that mean *this number is
/// which platter*. See the module docs for what the qualifier is holding back:
/// it is the only thing separating "the volume names its own games after the
/// index" from "these are sequels".
///
/// `part` is **not** among them, and the corpus says why: `v1 part1.img`,
/// `v1 part2.img`, `v2 part1.img` is the ambiguous stem that
/// `an_ambiguous_stem_forms_no_set` refuses, and admitting the word would give
/// its second run a prefix key and resolve the ambiguity by guessing.
const DISK_WORDS: [&str; 3] = ["disk", "disc", "side"];

/// Does the text immediately before the digit run at `pos` say the number is a
/// **disk number** (SQ-0961)?
///
/// Trailing separators are ignored, so `- Disk `, `_Disk` and `(Disk ` all
/// qualify and `journey_s` does not.
fn introduced_by_a_disk_word(toks: &[Token], pos: usize) -> bool {
    let Some(Token::Text(before)) = pos.checked_sub(1).and_then(|i| toks.get(i)) else {
        return false;
    };
    let word = before.trim_end_matches(|c: char| !c.is_ascii_alphanumeric()).to_ascii_lowercase();
    DISK_WORDS.iter().any(|w| word.ends_with(w))
}

/// Do these index values describe a run of **disks**?
///
/// Distinct, reaching `1`, and none above [`MAX_INDEX`]. See the module docs for
/// what each clause is holding back.
fn is_index_run(values: &[u64]) -> bool {
    if values.len() < 2 {
        return false;
    }
    let mut seen: Vec<u64> = values.to_vec();
    seen.sort_unstable();
    seen.dedup();
    seen.len() == values.len()
        && seen.first() == Some(&1)
        && seen.last().is_some_and(|&m| m <= MAX_INDEX)
}

/// Partition `files` into multi-disk sets, each **ordered by disk number**.
///
/// Only sets are returned: a file in no set appears in no group, and a group
/// always holds at least two files. Deterministic for a given input, and
/// independent of the order `files` arrives in — the caller may hand over a
/// directory listing in whatever order the filesystem produced.
pub fn group(files: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    group_indexed(files)
        .into_iter()
        .map(|g| g.into_iter().map(|(_, p)| p).collect())
        .collect()
}

/// [`group`], keeping each volume's **disk number** — the value of the digit run
/// that varies across the set (SQ-0865).
///
/// The number is the one the release itself spells, not the position in the
/// list: `is_index_run` requires the values to be distinct, to reach `1` and to
/// stay under [`MAX_INDEX`], but never to be contiguous, so a set missing its
/// middle platter still reports `3` for the disk labelled 3. That is the whole
/// reason this is exposed rather than left to a caller counting positions — a
/// dialog that says "from disk 2" about disk 3 is worse than one that says
/// nothing.
pub fn group_indexed(files: &[PathBuf]) -> Vec<Vec<(u64, PathBuf)>> {
    // Candidates: disk-image spellings only, sorted so the result never depends
    // on `read_dir` order.
    let mut cands: Vec<&PathBuf> = files.iter().filter(|p| has_image_ext(p)).collect();
    cands.sort();
    cands.dedup();

    // Bucket by "everything except digit run `pos`". A file with three digit runs
    // is offered under three keys; a key collects the files that agree on all the
    // OTHER runs, which is exactly the candidate set for varying that one.
    type Key = (String, usize, Vec<Token>);
    let mut buckets: HashMap<Key, Vec<(u64, PathBuf)>> = HashMap::new();
    for path in &cands {
        let Some((toks, ext)) = parts(path) else { continue };
        for (pos, tok) in toks.iter().enumerate() {
            let Some(value) = tok.value() else { continue };
            let shape = if introduced_by_a_disk_word(&toks, pos) {
                // The prefix alone (SQ-0961): a volume that names its own games
                // after the index shares no suffix with its siblings. Safe here
                // and nowhere else — see the module docs.
                toks[..pos].to_vec()
            } else {
                let mut holed = toks.clone();
                // The hole must not be confusable with any literal text.
                holed[pos] = Token::Text("\u{0}".to_string());
                holed
            };
            // The two shapes can never collide: a holed key always runs past
            // `pos` and a prefix key always stops at it, so they differ in
            // length before they differ in anything else.
            buckets.entry((ext.clone(), pos, shape)).or_default().push((value, (*path).clone()));
        }
    }

    // Keep the buckets whose varying run really is a disk index.
    let mut groups: Vec<Vec<(u64, PathBuf)>> = buckets
        .into_values()
        .filter(|members| {
            let values: Vec<u64> = members.iter().map(|(v, _)| *v).collect();
            is_index_run(&values)
        })
        .collect();

    // Ambiguity is refused, not resolved: a file that two different digit runs
    // each make a plausible set out of has no unambiguous set, and guessing which
    // is the disk number is exactly the false positive this module exists to
    // avoid. Both candidate groups go.
    let mut times_seen: HashMap<PathBuf, usize> = HashMap::new();
    for g in &groups {
        for (_, p) in g {
            *times_seen.entry(p.clone()).or_default() += 1;
        }
    }
    groups.retain(|g| g.iter().all(|(_, p)| times_seen.get(p) == Some(&1)));

    let mut out: Vec<Vec<(u64, PathBuf)>> = groups
        .into_iter()
        .map(|mut g| {
            g.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            g
        })
        .collect();
    // Ordered by the paths alone, exactly as the `Vec<Vec<PathBuf>>` this used to
    // build sorted itself — the disk numbers must not become a second sort key.
    out.sort_by(|a, b| a.iter().map(|(_, p)| p).cmp(b.iter().map(|(_, p)| p)));
    out
}

/// Every volume of the set `path` belongs to — `path` included — in disk order,
/// or `None` when it is not part of one.
///
/// Reads `path`'s directory and nothing else: the rule is entirely a question
/// about names, so this never opens a disk.
pub fn members(path: &Path) -> Option<Vec<PathBuf>> {
    Some(members_indexed(path)?.into_iter().map(|(_, p)| p).collect())
}

/// [`members`], each volume paired with the **disk number** its name carries
/// (SQ-0865).
pub fn members_indexed(path: &Path) -> Option<Vec<(u64, PathBuf)>> {
    if !has_image_ext(path) {
        return None;
    }
    let dir = path.parent()?;
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    group_indexed(&files).into_iter().find(|g| g.iter().any(|(_, m)| m == path))
}

/// Open the disk image `path`, whose bytes are `raw`, with the other volumes of
/// its multi-disk release available to it (SQ-0864).
///
/// **The one way a front-end mounts a named disk**, and the reason it is one way:
/// a story can live on no single floppy. The Apple II 5.25-inch presses of
/// *Shogun*, *Journey* and *Zork Zero* page one game across five, five and four
/// volumes, and the Commodore *Trinity* pages one across two sides — so opening
/// any one of them and asking what is on it is a question only the whole release
/// can answer.
///
/// Which files are one release is [`members`]'s answer, from their names and
/// without opening anything. It is deliberately not `blorb`'s: naming is
/// filesystem policy and that crate is given bytes.
///
/// **Nothing is read eagerly.** [`blorb::medium::MountedDisk::mount_set`] calls
/// the closure only when the named volume turns out to have no story of its own,
/// so an ordinary floppy and every volume of a compilation cost exactly the one
/// read they always did — which is what keeps a library scan from reading seven
/// 800 KB floppies per row.
///
/// # And when the release keeps the story on a volume of its own (SQ-0941)
///
/// `mount_set` answers for a story the release *pages* across its volumes, which
/// is the Apple II and Commodore case above. It cannot answer for a release
/// whose volumes are independent filesystems holding distinct files, because
/// there is no container spanning them to reassemble — and that is the DOS
/// press, where the story sits whole on one floppy and the others carry the
/// installer and the artwork. Measured on the 360K *Zork Zero* (release 393 /
/// serial 890714), three separate FAT12 volumes:
///
/// | volume | files | opened |
/// | --- | --- | --- |
/// | Disk 1 | `INSTALL.EXE`, `EZR.EXE`, `IZORK0.RUN`, `ZORK0.CG1` | nothing |
/// | Disk 2 | `ZORK0.ZIP`, `ZORKZERO.EXE` | the game |
/// | Disk 3 | `ZORK0.EG1` | nothing |
///
/// **Disk 1 is the disk a player opens** — it is the one with the installer on
/// it — and it was the one that could not work. So a volume with no story of its
/// own now asks [`members_indexed`] for its release's other volumes and takes
/// the story off whichever one has it. The set is already known here and the
/// siblings are already reachable; this is the same question asked one call site
/// later.
///
/// **Only when the release carries exactly one game**, which is
/// `app::assets::volumes`'s threshold and is here for the same reason: widening
/// across *The Lost Treasures of Infocom* would hand whoever opened its launcher
/// disk one of thirty unrelated games. A shelf is a browser's job, not a
/// mount's, and the count stops at the second story rather than mounting the
/// rest of the set.
///
/// The siblings are mounted **plainly**, not across the set, and that is exact
/// rather than thrifty: a story the release pages across its volumes was already
/// found above, from whichever volume was named, so the only thing left to look
/// for is a story a sibling holds on its own.
pub fn mount_at(
    path: &Path,
    raw: Vec<u8>,
) -> Result<blorb::medium::MountedDisk, blorb::medium::MountError> {
    widen_to_the_release(path, mount_one(path, raw)?)
}

/// [`mount_at`] for an image already opened through [`blorb::image`] — the
/// door a launch uses, so the file is read once however many steps mount it
/// (SQ-1762).
pub fn mount_file_at(
    path: &Path,
    file: &blorb::image::DiskFile,
) -> Result<blorb::medium::MountedDisk, blorb::medium::MountError> {
    widen_to_the_release(
        path,
        blorb::medium::MountedDisk::mount_file(file, || sibling_images(path))?,
    )
}

/// The widening [`mount_at`] documents: a volume with no story of its own asks
/// its release's other volumes.
fn widen_to_the_release(
    path: &Path,
    disk: blorb::medium::MountedDisk,
) -> Result<blorb::medium::MountedDisk, blorb::medium::MountError> {
    if !disk.stories().is_empty() {
        return Ok(disk);
    }
    // Nothing here and nothing paged across the set: the release's other volumes
    // are the last place to look, and the mount this volume gave is what we keep
    // when they have nothing either — so an error message still describes the
    // disk the player named.
    Ok(story_elsewhere_in_the_release(path).unwrap_or(disk))
}

/// [`mount_at`] without the widening: one named volume, opened with its set's
/// other volumes offered to the container that spans them.
fn mount_one(
    path: &Path,
    raw: Vec<u8>,
) -> Result<blorb::medium::MountedDisk, blorb::medium::MountError> {
    blorb::medium::MountedDisk::mount_set(raw, || sibling_images(path))
}

/// The rest of `path`'s release, as whole images and in disk order — read only
/// when the mount asks for them.
fn sibling_images(path: &Path) -> Vec<Vec<u8>> {
    members(path)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m != path)
        .filter_map(|m| blorb::image::open_disk(&m).map(|f| f.bytes().to_vec()))
        .collect()
}

/// The volume of `path`'s release that carries the one story the release holds,
/// or `None` when it is in no set, when no sibling has a story, or when the set
/// turns out to be a shelf of several (SQ-0941).
///
/// [`members_indexed`] is in disk order, so the volume that answers is the
/// lowest-numbered one that has a story — but the single-game rule makes that a
/// property rather than a preference: there is only ever one to find.
fn story_elsewhere_in_the_release(path: &Path) -> Option<blorb::medium::MountedDisk> {
    let mut found: Option<blorb::medium::MountedDisk> = None;
    for (_, m) in members_indexed(path)? {
        if m == path {
            continue;
        }
        let Some(file) = blorb::image::open_disk(&m) else { continue };
        let Ok(disk) = blorb::medium::MountedDisk::mount_file(&file, Vec::new) else { continue };
        match disk.stories().len() {
            0 => {}
            1 if found.is_none() => found = Some(disk),
            // A second game: this is a compilation, and no volume of it speaks
            // for another. Refused whole rather than resolved by picking one.
            _ => return None,
        }
    }
    found
}

/// One story a path can reach, and the volume it is actually stored on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reachable {
    /// The volume holding it: the one that was named, or a sibling of it.
    pub volume: PathBuf,
    /// How that volume spells it — [`blorb::medium::DiskStory::name`].
    pub name: String,
    /// The story image, byte-exact off the disk.
    pub bytes: Vec<u8>,
    /// The medium THIS story came off, which on a hybrid disc is not the
    /// volume's own format — [`blorb::medium::MountedDisk::image_for`].
    pub image: blorb::medium::DiskImage,
}

/// **Every story `path` can reach across its release**, in disk order (SQ-0961).
///
/// The story-side answer to the question `app::assets::volumes` already answers
/// for artwork, and it exists for the same reason: without one, each front-end
/// decides for itself how far to look and they disagree. They did. `zvm-cli`
/// pointed at `treasures/Lost Treasures of Infocom, The_Disk1.adf` offered the
/// six games on **that platter** — Enchanter, Sorcerer, Spellbreaker and Zork
/// I–III — while lanthorn, pointed at the same file, listed all twenty across
/// the six-volume release. Nothing was wrong with the CLI's mount; it simply
/// asked a narrower question, because there was no wider one to ask.
///
/// `disk` is what [`mount_at`] returned for `path`, so the named volume costs no
/// second read and the caller keeps the mount for its own diagnostics. Its
/// stories come first, whole and in the order it reported them: **a compilation
/// volume's own menu does not move because the release around it is now
/// visible.** The siblings follow in disk order.
///
/// **A build already offered by an earlier volume is dropped**, keyed on release,
/// serial **and machine/image** — the same principle `app::picker`'s
/// `dedupe_within_a_volume` doc comment already states (SQ-0878): build-identity
/// alone is not enough, because two different machines can legitimately ship the
/// identical build. So two rows fold into one only when they'd also share a save
/// directory ([`crate::storage::disk_story_key`]) *and* a machine. That is not
/// tidiness: SQ-0941's widening means the 360K *Zork Zero* press answers
/// `ZORK0.ZIP` from Disk 1 *and* from Disk 2, and without the fold a
/// three-floppy single-game release would grow a menu — and that case still
/// folds under the machine-aware key, because same build *and* same image/machine
/// still collide.
///
/// Duplicates **within** one volume are left exactly as they are. The DiskCopy
/// *Lost Treasures* Disk 1 stores *The Lurking Horror* three times over —
/// `Lurking Horror`, `Trash/Lurking Horror`, `Trash/The Lurking Horror` — and
/// the CLI has always shown all of them, told apart by the only thing that tells
/// them apart, which is the name. Folding a platter's own list is a separate
/// question from reaching the platters beside it.
pub fn stories_across_the_release(path: &Path, disk: &blorb::medium::MountedDisk) -> Vec<Reachable> {
    let mut out: Vec<Reachable> = disk
        .stories()
        .into_iter()
        .map(|s| Reachable {
            volume: path.to_path_buf(),
            image: disk.image_for(&s.name),
            name: s.name,
            bytes: s.bytes,
        })
        .collect();
    let Some(members) = members_indexed(path) else { return out };
    // What the named volume already offers, so a sibling repeating it adds no row.
    // Keyed on the story's own image as well as its build (SQ-1523): two machines
    // can ship the identical release+serial, and dropping one because the other
    // was seen first silently loses a real, separate story.
    let mut seen: Vec<(blorb::medium::DiskImage, u16, String)> =
        out.iter().filter_map(|r| build_of(r.image, &r.bytes)).collect();
    for (_, m) in members {
        if m == path {
            continue;
        }
        // Plainly, not across the set: a story the release PAGES across its
        // volumes was already reassembled above from whichever volume was named,
        // so the only thing left to look for is a story a sibling holds on its
        // own — the same argument `story_elsewhere_in_the_release` makes.
        let Some(file) = blorb::image::open_disk(&m) else { continue };
        let Ok(sibling) = blorb::medium::MountedDisk::mount_file(&file, Vec::new) else { continue };
        for s in sibling.stories() {
            let image = sibling.image_for(&s.name);
            match build_of(image, &s.bytes) {
                Some(b) if seen.contains(&b) => continue,
                Some(b) => seen.push(b),
                None => {}
            }
            out.push(Reachable { volume: m.clone(), image, name: s.name, bytes: s.bytes });
        }
    }
    out
}

/// The image, release and serial a story's header carries, or `None` when it has
/// no Z-machine header to read — the identity the cross-volume fold is keyed on.
fn build_of(image: blorb::medium::DiskImage, bytes: &[u8]) -> Option<(blorb::medium::DiskImage, u16, String)> {
    let (_, release, serial) = crate::storage::DiskBuild::header_of(bytes)?;
    Some((image, release, serial))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|n| PathBuf::from("/stories").join(n)).collect()
    }

    fn names_of(g: &[PathBuf]) -> Vec<String> {
        g.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    /// The set the user named: `diskN.img`.
    #[test]
    fn a_trailing_index_on_a_disk_image_is_a_set() {
        let g = group(&paths(&["disk1.img", "disk2.img", "disk3.img", "disk4.img"]));
        assert_eq!(g.len(), 1);
        assert_eq!(names_of(&g[0]), ["disk1.img", "disk2.img", "disk3.img", "disk4.img"]);
    }

    /// …and it is ordered by disk number, not by filename, so a ten-disk set does
    /// not run 1, 10, 2.
    #[test]
    fn members_are_ordered_by_disk_number() {
        let g = group(&paths(&["d10.img", "d2.img", "d1.img"]));
        assert_eq!(names_of(&g[0]), ["d1.img", "d2.img", "d10.img"]);
    }

    /// Two families in one directory are two sets — the stem text and the
    /// extension both differ, and either alone would be enough.
    #[test]
    fn dos_floppies_and_dos_disks_are_two_sets() {
        let g = group(&paths(&["disk1.img", "disk2.img", "floppy1.ima", "floppy2.ima"]));
        assert_eq!(g.len(), 2, "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
        let all: Vec<Vec<String>> = g.iter().map(|x| names_of(x)).collect();
        assert!(all.contains(&vec!["disk1.img".to_string(), "disk2.img".to_string()]));
        assert!(all.contains(&vec!["floppy1.ima".to_string(), "floppy2.ima".to_string()]));
    }

    /// A constant digit run elsewhere in the stem is no obstacle: the index is
    /// the run that varies, wherever it sits.
    #[test]
    fn an_index_in_the_middle_of_a_stem_is_found() {
        let g = group(&paths(&[
            "Lost Treasures of Infocom, The (1993)(BRCC)(Disk 1 of 7).2mg",
            "Lost Treasures of Infocom, The (1993)(BRCC)(Disk 2 of 7).2mg",
            "Lost Treasures of Infocom, The (1993)(BRCC)(Disk 3 of 7).2mg",
        ]));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].len(), 3);
        // The `1993` and the `of 7` never varied, so neither was ever a candidate.
    }

    #[test]
    fn the_atari_st_compilations_are_one_set() {
        let names: Vec<String> =
            (1..=9).map(|n| format!("Infocom Compilation {n} (19xx)(-).st")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let g = group(&paths(&refs));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].len(), 9, "one shelf of nine, not three shelves of three");
    }

    // ── What it refuses ──────────────────────────────────────────────────────

    /// **The false positive that matters.** Thirteen Scott Adams games named
    /// `advNN.dat` are a textbook prefix-plus-index and must never be a set,
    /// because merging them would put twelve unrelated games behind one of them.
    #[test]
    fn a_numbered_run_of_story_files_is_not_a_set() {
        let names: Vec<String> = (1..=13).map(|n| format!("adv{n:02}.dat")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        assert!(group(&paths(&refs)).is_empty(), ".dat is not a disk-image spelling");
    }

    /// **The free consequence** (SQ-0864). The Apple II 5.25-inch presses were
    /// refused here for exactly one reason — `.dsk` was not a spelling
    /// `blorb::medium` claimed — and the moment the ProDOS row claimed it they
    /// became sets, with not a line of this module changed.
    ///
    /// It is the sharpest case in the corpus for grouping at all: each of these
    /// floppies carries a fifth or a quarter of one story and nothing else, so a
    /// set that is not recognised is a game that cannot be opened. (*Journey*'s
    /// five joined *Shogun*'s and *Zork Zero*'s in SQ-0863, again with not a
    /// line of this module changed.)
    #[test]
    fn the_apple_five_and_a_quarter_inch_presses_are_two_sets() {
        let mut names: Vec<String> = (1..=5).map(|n| format!("shogun_s{n}.dsk")).collect();
        names.extend((1..=4).map(|n| format!("zork_zero_{n}.dsk")));
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let g = group(&paths(&refs));
        assert_eq!(g.len(), 2, "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
        let sizes: Vec<usize> = {
            let mut s: Vec<usize> = g.iter().map(|x| x.len()).collect();
            s.sort_unstable();
            s
        };
        assert_eq!(sizes, [4, 5], "Zork Zero on four floppies, Shogun on five");
        // …and in disk order, which is the order the segments pair in.
        let shogun = g.iter().find(|x| x.len() == 5).expect("the five-disk set");
        assert_eq!(names_of(shogun), [
            "shogun_s1.dsk",
            "shogun_s2.dsk",
            "shogun_s3.dsk",
            "shogun_s4.dsk",
            "shogun_s5.dsk",
        ]);
    }

    /// **The free consequence, a second time** (SQ-0869) — and the first set in
    /// the corpus whose members SHOUT their extension.
    ///
    /// `TRINITY1.D64` and `TRINITY2.D64` are the two sides of Infocom's
    /// Commodore *Trinity*, a Version 4 story of 262,064 bytes on a pair of
    /// 174,848-byte floppies. They became a set the moment `blorb::medium`
    /// claimed `.d64`, with not a line of this module changed — including the
    /// case-folding, since `parts` lowercases and the census is lowercase.
    ///
    /// **And the lone *Hitchhiker's* disk is not a set**, which is the other
    /// half and the one worth asserting: its stem carries a four-digit year, so
    /// a rule that grouped on "there is a number in the name" would have made a
    /// one-member set out of it. `is_index_run` refuses a run of one and refuses
    /// values that do not start at 1, and 1984 is both.
    #[test]
    fn the_two_sides_of_the_commodore_trinity_are_one_set_and_hitchhikers_is_none() {
        let lone = "Hitchhikers_Guide_to_the_Galaxy_The_1984_Infocom.d64";
        let g = group(&paths(&["TRINITY1.D64", "TRINITY2.D64", lone]));
        assert_eq!(g.len(), 1, "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
        assert_eq!(names_of(&g[0]), ["TRINITY1.D64", "TRINITY2.D64"], "in side order");
        assert!(
            !g[0].iter().any(|p| p.ends_with(lone)),
            "a single disk with a year in its name is not a set"
        );
        // …and on its own it forms no group at all, rather than a group of one.
        assert!(group(&paths(&[lone])).is_empty());
    }

    /// …and the census is genuinely where that came from: nothing in this
    /// module names a spelling, so `.dsk` and `.d64` had to arrive through the
    /// table.
    #[test]
    fn the_extension_census_is_the_tables_and_not_this_modules() {
        assert!(has_image_ext(Path::new("/stories/shogun_s1.dsk")));
        assert!(blorb::medium::image_extensions().any(|e| e == "dsk"));
        assert!(has_image_ext(Path::new("/stories/TRINITY1.D64")), "and case does not decide it");
        assert!(blorb::medium::image_extensions().any(|e| e == "d64"));
        assert!(!has_image_ext(Path::new("/stories/adv01.dat")));
    }

    /// **The corpus's sharpest case.** Zork Zero's two DOS presses spell their
    /// disks alike, so `(360K) (Disk 1)` and `(720K) (Disk 1)` differ at exactly
    /// one digit run — and `{360, 720}` is a capacity, not a disk index. The two
    /// presses must come out as two sets of three and two.
    #[test]
    fn two_presses_of_one_game_are_two_sets() {
        let g = group(&paths(&[
            "Zork Zero (1989) (r393, Serial 890714) (360K) (Disk 1) [!].ima",
            "Zork Zero (1989) (r393, Serial 890714) (360K) (Disk 2) [!].ima",
            "Zork Zero (1989) (r393, Serial 890714) (360K) (Disk 3) [!].ima",
            "Zork Zero (1989) (r393, Serial 890714) (720K) (Disk 1) [!].ima",
            "Zork Zero (1989) (r393, Serial 890714) (720K) (Disk 2) [!].ima",
        ]));
        assert_eq!(g.len(), 2, "the capacity run must not group the two presses");
        let sizes: Vec<usize> = {
            let mut s: Vec<usize> = g.iter().map(|x| x.len()).collect();
            s.sort_unstable();
            s
        };
        assert_eq!(sizes, [2, 3]);
        for set in &g {
            let caps: Vec<bool> =
                set.iter().map(|p| p.to_string_lossy().contains("360K")).collect();
            assert!(caps.iter().all(|c| *c == caps[0]), "a set mixed the two capacities");
        }
    }

    /// **A volume that names its own games after the index is still a set**
    /// (SQ-0961) — the DiskCopy 4.2 press of *The Lost Treasures of Infocom*,
    /// whose five stems agree on nothing after `Disk N`.
    ///
    /// FALSIFICATION: drop the [`introduced_by_a_disk_word`] branch from
    /// `group_indexed` and this reports no set at all, which is exactly what
    /// `app::disk_set::members` answered for this press before the fix.
    #[test]
    fn a_volume_that_names_its_own_games_is_still_a_set() {
        let g = group(&paths(&[
            "The Lost Treasures of Infocom - Disk 1 - Beyond Zork, Lurking Horror.dc42",
            "The Lost Treasures of Infocom - Disk 2 - Hitchhiker's, Infidel, Planetfall.dc42",
            "The Lost Treasures of Infocom - Disk 3 - Ballyhoo, Deadline, Moonmist.dc42",
            "The Lost Treasures of Infocom - Disk 4 - Enchanter, Sorcerer, Zork I.dc42",
            "The Lost Treasures of Infocom - Disk 5 - Zork Zero.dc42",
        ]));
        assert_eq!(g.len(), 1, "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
        assert_eq!(g[0].len(), 5);
        assert!(names_of(&g[0])[0].contains("Disk 1"), "and in disk order");
    }

    /// …and the qualifier is what keeps that from swallowing a shelf of sequels,
    /// which is the same shape with no word saying the number is a platter.
    #[test]
    fn sequels_that_subtitle_themselves_are_not_a_set() {
        let g = group(&paths(&[
            "Ultima 1.dsk",
            "Ultima 2 - Revenge of the Enchantress.dsk",
            "Ultima 3 - Exodus.dsk",
        ]));
        assert!(g.is_empty(), "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
    }

    /// A run of years is not a run of disks, however tidily it varies.
    #[test]
    fn a_varying_year_is_not_an_index() {
        let g = group(&paths(&["Zork (1988)(Infocom).adf", "Zork (1989)(Infocom).adf"]));
        assert!(g.is_empty(), "1988/1989 reach neither 1 nor MAX_INDEX");
    }

    /// The index must reach 1. Stated as a test because it is the clause doing
    /// the work above, and because its cost — a set whose first disk is missing
    /// is no set — should fail loudly here if anyone relaxes it.
    #[test]
    fn a_set_without_its_first_disk_is_not_recognised() {
        assert!(group(&paths(&["floppy2.ima", "floppy3.ima", "floppy4.ima"])).is_empty());
        assert!(!group(&paths(&["floppy1.ima", "floppy2.ima"])).is_empty());
    }

    /// A lone volume is not a set, and neither is a repeated index.
    #[test]
    fn a_set_needs_two_distinct_disks() {
        assert!(group(&paths(&["floppy1.ima"])).is_empty());
        assert!(group(&paths(&["floppy1.ima", "floppy01.ima"])).is_empty(), "1 twice is not 1..2");
    }

    /// Roman numerals are text; three Zork floppies are three games.
    #[test]
    fn roman_numerals_are_not_indices() {
        let g = group(&paths(&[
            "Zork I - The Great Underground Empire.adf",
            "Zork II - The Wizard of Frobozz.adf",
            "Zork III - The Dungeon Master.adf",
        ]));
        assert!(g.is_empty());
    }

    /// Two digit runs that each make a plausible set leave the file in neither:
    /// guessing which is the disk number is the failure mode, so it is refused.
    #[test]
    fn an_ambiguous_stem_forms_no_set() {
        let g = group(&paths(&["v1 part1.img", "v1 part2.img", "v2 part1.img"]));
        assert!(g.is_empty(), "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
    }

    /// Extensions are matched case-insensitively, and stems are not.
    #[test]
    fn extensions_are_case_insensitive() {
        let g = group(&paths(&["DISK1.IMG", "DISK2.IMG"]));
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn members_of_a_non_image_is_none() {
        assert!(members(Path::new("/stories/zork1.z5")).is_none());
    }

    /// **A single raw disk is not a set of one** (SQ-0868), even sitting in a
    /// directory full of `.dsk` volumes that are.
    ///
    /// `Planetfall r29 (clean copy from retail disk).dsk` is the corpus's first
    /// `.dsk` that is a whole game on one disk, and its stem carries a digit run
    /// (`r29`) — so the two things that could go wrong are that it forms a
    /// degenerate set of its own, or that it is dragged into Shogun's or Zork
    /// Zero's. Neither can: a set needs two files whose stems differ at ONE digit
    /// run, and no other file in the corpus shares this stem at all.
    ///
    /// This matters more than a lone-volume test usually would, because the
    /// consequence is not cosmetic. `MountedDisk::mount_set` consults a set's
    /// companions, and `dedupe_within_sets` folds rows sharing an IFID *within a
    /// set* — so a false set here is how a game silently disappears from the
    /// browser.
    #[test]
    fn a_lone_raw_disk_forms_no_set_among_the_sets_beside_it() {
        const RAW: &str = "Planetfall r29 (clean copy from retail disk).dsk";
        let files = paths(&[
            RAW,
            "shogun_s1.dsk",
            "shogun_s2.dsk",
            "zork_zero_1.dsk",
            "zork_zero_2.dsk",
        ]);
        let sets = group(&files);
        assert_eq!(sets.len(), 2, "the two presses are sets and the raw disk is not");
        for set in &sets {
            assert!(
                !names_of(set).iter().any(|n| n == RAW),
                "the raw disk was folded into {:?}",
                names_of(set)
            );
        }
        // …and on its own it is still nothing, digit run notwithstanding.
        assert!(group(&paths(&[RAW])).is_empty());
    }

    // ── Opening from a volume that carries no story (SQ-0941) ────────────────
    //
    // The real case is the DOS *Zork Zero* press, whose three volumes are three
    // independent FAT12 filesystems — see [`mount_at`] for the measurement. The
    // media are gitignored, so the shape is reproduced here on synthetic
    // AmigaDOS floppies, which is the cheapest image this crate can mount.

    /// AmigaDOS block size.
    const BSIZE: usize = 512;
    /// Blocks on a DD floppy.
    const DD_BLOCKS: usize = 1760;

    /// The smallest AmigaDOS (FFS) image `blorb` will mount and list: a
    /// bootblock, then one header block per file naming its data blocks in the
    /// reverse-order table at `BSIZE-204`. Files are small enough never to need
    /// an extension block, and the reader finds headers by scanning, so no root
    /// block is needed.
    fn floppy(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut image = vec![0u8; DD_BLOCKS * BSIZE];
        image[0..3].copy_from_slice(b"DOS");
        image[3] = 1; // FFS: a data block is its raw payload
        let mut next = 881;
        let put32 = |image: &mut Vec<u8>, block: usize, off: usize, v: u32| {
            let at = block * BSIZE + off;
            image[at..at + 4].copy_from_slice(&v.to_be_bytes());
        };
        for (name, data) in files {
            let header = next;
            next += 1;
            put32(&mut image, header, 0, 2); // T_HEADER
            put32(&mut image, header, 4, header as u32);
            put32(&mut image, header, BSIZE - 4, 0xFFFF_FFFD); // ST_FILE
            put32(&mut image, header, BSIZE - 188, data.len() as u32);
            let at = header * BSIZE + BSIZE - 80;
            image[at] = name.len() as u8;
            image[at + 1..at + 1 + name.len()].copy_from_slice(name.as_bytes());
            for (i, chunk) in data.chunks(BSIZE).enumerate() {
                let db = next;
                next += 1;
                let at = db * BSIZE;
                image[at..at + chunk.len()].copy_from_slice(chunk);
                put32(&mut image, header, BSIZE - 204 - 4 * i, db as u32);
            }
            put32(&mut image, header, 8, data.len().div_ceil(BSIZE) as u32);
        }
        image
    }

    /// A Version 3 story, only as far as `blorb`'s own sniff looks: header
    /// fields consistent enough for `blorb::adf::looks_like_zcode`, with the
    /// release word carrying `release` so two of these can be told apart.
    fn zcode(release: u16) -> Vec<u8> {
        let mut b = vec![0u8; 0x400];
        b[0x00] = 3;
        b[0x02..0x04].copy_from_slice(&release.to_be_bytes());
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x06, 0x0040); // initial PC
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x18, 0x0060); // abbreviations
        word(0x1a, 0x0200); // declared length, in v3's two-byte units
        b[0x12..0x18].copy_from_slice(b"890714");
        b
    }

    /// One synthetic volume of a press: the filename it is written under, and
    /// the files on it.
    type Platter<'a> = (&'a str, Vec<(&'a str, Vec<u8>)>);

    /// A directory of its own, so `members_indexed`'s `read_dir` sees exactly
    /// the volumes a case put there.
    fn press(tag: &str, volumes: &[Platter<'_>]) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NTH: AtomicUsize = AtomicUsize::new(0);
        let nth = NTH.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("lanthorn-sq0941-{tag}-{}-{nth}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, files) in volumes {
            std::fs::write(dir.join(name), floppy(files)).unwrap();
        }
        dir
    }

    fn opened(path: &Path) -> Vec<String> {
        let raw = std::fs::read(path).expect("the volume reads");
        let disk = mount_at(path, raw).expect("the volume mounts");
        disk.stories().into_iter().map(|s| s.name).collect()
    }

    /// **The defect.** Disk 1 of a DOS press carries the installer and one art
    /// rendition; the story is whole on disk 2. Naming disk 1 — the disk a
    /// player opens — must reach the game, not fail silently.
    ///
    /// FALSIFICATION: drop the `story_elsewhere_in_the_release` arm from
    /// `mount_at` and disk 1 and disk 3 both report no story at all, which is
    /// the symptom as measured on the 360K *Zork Zero* (r393/s890714).
    #[test]
    fn a_volume_with_no_story_opens_its_releases_only_game() {
        let dir = press("one-game", &[
            ("zork0_1.adf", vec![("Install", b"not a story".to_vec())]),
            ("zork0_2.adf", vec![("Story.data", zcode(393))]),
            ("zork0_3.adf", vec![("Art.eg1", b"not a story either".to_vec())]),
        ]);
        assert_eq!(opened(&dir.join("zork0_2.adf")), ["Story.data"], "the story's own disk");
        assert_eq!(opened(&dir.join("zork0_1.adf")), ["Story.data"], "the installer disk");
        assert_eq!(opened(&dir.join("zork0_3.adf")), ["Story.data"], "the artwork disk");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The guard.** A launcher disk on a *compilation* must stay a launcher
    /// disk: the release holds several games and no volume of it speaks for
    /// another, whether the second game is on the same sibling or on a later
    /// one. Reaching for one would hand whoever opened *The Lost Treasures of
    /// Infocom*'s disk 1 whichever of thirty games the scan happened to find
    /// first; that shelf is the browser's to present.
    #[test]
    fn a_launcher_disk_on_a_shelf_reaches_for_nothing() {
        let two_on_one = press("shelf-one-disk", &[
            ("ltoi1.adf", vec![("Launcher", b"not a story".to_vec())]),
            ("ltoi2.adf", vec![("Zork.I", zcode(88)), ("Zork.II", zcode(48))]),
        ]);
        assert!(opened(&two_on_one.join("ltoi1.adf")).is_empty(), "two games on one sibling");
        let _ = std::fs::remove_dir_all(&two_on_one);

        let one_each = press("shelf-two-disks", &[
            ("shelf1.adf", vec![("Launcher", b"not a story".to_vec())]),
            ("shelf2.adf", vec![("Zork.I", zcode(88))]),
            ("shelf3.adf", vec![("Zork.II", zcode(48))]),
        ]);
        assert!(opened(&one_each.join("shelf1.adf")).is_empty(), "one game per sibling");
        let _ = std::fs::remove_dir_all(&one_each);
    }

    /// **What does not move.** A volume that carries a game of its own answers
    /// with it and never consults a sibling, and a lone disk with nothing on it
    /// still has nothing on it — which is the message `zvm-cli` and the TUI both
    /// print, and it must go on describing the disk the player named.
    #[test]
    fn a_volume_that_has_a_game_and_a_disk_that_is_in_no_set_are_unchanged() {
        let dir = press("own-game", &[
            ("press1.adf", vec![("Story.data", zcode(88))]),
            ("press2.adf", vec![("Story.data", zcode(48))]),
        ]);
        // Two games, so nothing is widened anyway — but each volume answering
        // with its OWN build is the property that says the sibling was never
        // preferred to it.
        for (vol, release) in [("press1.adf", 88u16), ("press2.adf", 48)] {
            let path = dir.join(vol);
            let raw = std::fs::read(&path).unwrap();
            let disk = mount_at(&path, raw).unwrap();
            let story = disk.story().expect("its own game");
            assert_eq!(u16::from_be_bytes([story.bytes[2], story.bytes[3]]), release, "{vol}");
        }
        let _ = std::fs::remove_dir_all(&dir);

        let lone = press("lone", &[("boot.adf", vec![("Startup-Sequence", b"echo".to_vec())])]);
        assert!(opened(&lone.join("boot.adf")).is_empty(), "a boot disk in no set");
        let _ = std::fs::remove_dir_all(&lone);
    }

    // ── The cross-volume fold's key (SQ-1523) ─────────────────────────────────

    /// **The fix, isolated from disk I/O.** `build_of` is the whole of the key
    /// `stories_across_the_release`'s `seen` fold uses. Two machines that ship
    /// the identical release+serial (several late-1980s Infocom titles on the
    /// Lost Treasures compilations, e.g. Trinity r12/s860926 on both its DOS and
    /// Mac volumes) must not collide — and the SAME machine with the same build
    /// still must, which is the SQ-0941 case this fold exists for.
    ///
    /// FALSIFICATION: drop the `DiskImage` from `build_of`'s key and `dos_a` and
    /// `mac` compare equal.
    #[test]
    fn build_key_distinguishes_machine_not_only_build() {
        let story = zcode(12); // one release+serial, as if shipped for two machines
        let dos_a = build_of(blorb::medium::DiskImage::Fat12Dos, &story);
        let dos_b = build_of(blorb::medium::DiskImage::Fat12Dos, &story);
        let mac = build_of(blorb::medium::DiskImage::Hfs, &story);
        assert!(dos_a.is_some() && mac.is_some());
        assert_ne!(dos_a, mac, "two machines sharing a build must not collide in the fold");
        assert_eq!(dos_a, dos_b, "the same machine with the same build must still collide (SQ-0941)");
    }

    /// **The full seam, same machine.** Two AmigaDOS siblings paging the
    /// identical build (SQ-0941's shape) must still fold to one row through
    /// `stories_across_the_release` itself, not only at the key level above.
    #[test]
    fn same_machine_same_build_still_folds_across_siblings() {
        let dir = press("sq1523-fold", &[
            ("v1.adf", vec![("Story.data", zcode(393))]),
            ("v2.adf", vec![("Story.data", zcode(393))]),
        ]);
        let path = dir.join("v1.adf");
        let raw = std::fs::read(&path).unwrap();
        let disk = mount_one(&path, raw).unwrap();
        let rows = stories_across_the_release(&path, &disk);
        assert_eq!(rows.len(), 1, "same machine, same build, must still fold to one row: {rows:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
