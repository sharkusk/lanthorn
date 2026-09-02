//! SQ-0961: **how far to look for stories is one question with one answer.**
//!
//! `zvm-cli` pointed at `treasures/Lost Treasures of Infocom, The_Disk1.adf`
//! offered the six games on that platter; lanthorn pointed at the same file
//! listed all twenty across the six-volume release. Nothing was wrong with the
//! CLI's mount — it asked a narrower question, because there was no wider one to
//! ask. `app::assets::volumes` has been the single answer for *artwork* since
//! SQ-0874 ("the seam that knows disks exist so that no caller has to"); stories
//! had `cli_host::disk_set::mount_at` for the platter and nothing at all for the
//! release, so each front-end decided for itself and they drifted.
//!
//! Three earlier bugs are the same seam at three call sites — SQ-0941 (a disk
//! with no story of its own gave up instead of asking its release), SQ-0952 (the
//! path-only save key mounted the platter while the launch path mounted the set)
//! and this one — which is why the last case here is a **source-level rule**
//! rather than a fourth fix.
//!
//! # The specimens
//!
//! Two presses of one compilation, and CLAUDE.md's rule applies in full: they
//! are different *releases*, not one release on two media, and no result carries
//! from either to the other.
//!
//! | fixture | volumes | naming | games |
//! | --- | --- | --- | --- |
//! | `treasures/Lost Treasures of Infocom, The_Disk1.adf` … `_Disk6` | 6 | identical stem, index last | 20 |
//! | `treasures/The Lost Treasures of Infocom - Disk 1 - ….dc42` … `Disk 5` | 5 | index in the middle, **no common suffix** | 20 |
//!
//! They agree on the twenty games and on almost nothing else: the Amiga press
//! carries *Enchanter* r16/831118, *Hitchhiker's* r58/851002 and *Zork Zero*
//! r366/890323, the Macintosh press r29/860820, r59/851108 and r296/881019.
//! Measured 2026-08-21.
//!
//! The DiskCopy naming is why this suite exists on both: `disk_set::members`
//! returned **0** for it, because each volume names the games it carries after
//! the index and a prefix-plus-index-plus-suffix rule can group nothing that
//! shape. Without the grouping fix the enumeration seam would have listed one
//! platter there and nobody would have noticed.
//!
//! `treasures/` is gitignored (commercial media), so every real-media case skips
//! vacuously when its fixture is missing and every `ran > 0` guard is gated on a
//! presence check. The last case needs no fixture at all.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn treasures_dir() -> PathBuf {
    repo_root().join("treasures")
}

fn stories_dir() -> PathBuf {
    repo_root().join("stories")
}

/// One press of *The Lost Treasures of Infocom*: a volume to name, how many
/// volumes the release has, and how many games it offers.
struct Press {
    member: &'static str,
    volumes: usize,
    games: usize,
}

const PRESSES: &[Press] = &[
    // The Amiga press. Named by disk 3 rather than disk 1, because the claim is
    // about the release and not about a privileged volume.
    Press { member: "Lost Treasures of Infocom, The_Disk3.adf", volumes: 6, games: 20 },
    // The Macintosh DiskCopy 4.2 press — the naming that grouped as nothing.
    Press {
        member: "The Lost Treasures of Infocom - Disk 1 - Beyond Zork, Lurking Horror.dc42",
        volumes: 5,
        games: 20,
    },
];

fn any_press_present() -> bool {
    PRESSES.iter().any(|p| treasures_dir().join(p.member).exists())
}

/// A scratch storage base, unique per test process.
fn data_base(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("sq0961-{tag}"))
}

/// Every menu row **`zvm-cli`** would build for `path` — the seam's own answer,
/// duplicates and all.
fn rows_the_cli_offers(path: &Path) -> Vec<cli_host::disk_set::Reachable> {
    let Ok(raw) = std::fs::read(path) else { return Vec::new() };
    let Ok(disk) = cli_host::disk_set::mount_at(path, raw) else { return Vec::new() };
    cli_host::disk_set::stories_across_the_release(path, &disk)
}

/// …identified by build, which is the count of *games*.
fn builds_the_cli_offers(path: &Path) -> BTreeSet<(u16, String)> {
    rows_the_cli_offers(path)
        .iter()
        .filter_map(|r| cli_host::DiskBuild::header_of(&r.bytes))
        .map(|(_, release, serial)| (release, serial))
        .collect()
}

/// What **lanthorn** would offer for `path`: the browser's rows, identified the
/// same way.
fn builds_the_browser_offers(path: &Path, base: &Path) -> BTreeSet<(u16, String)> {
    let Some(source) = app::picker::StorySource::of(path, base) else { return BTreeSet::new() };
    source
        .scan(base)
        .iter()
        .filter_map(|e| Some((e.meta.release?, e.meta.serial.clone()?)))
        .collect()
}

/// **The defect, as one assertion**: name a volume and both front-ends offer the
/// same games.
///
/// Compared by BUILD rather than by row count, because the two lists are not
/// obliged to be the same length and should not be made so: the DiskCopy disk 1
/// stores *The Lurking Horror* three times over (`Lurking Horror`,
/// `Trash/Lurking Horror`, `Trash/The Lurking Horror`) and the CLI menu has
/// always shown all three, told apart by the only thing that tells them apart.
/// Twenty-two candidates, twenty games, and the games are the claim.
///
/// FALSIFICATION: make `stories_across_the_release` return only the named
/// volume's own stories and this fails with the reported symptom — six builds
/// against the browser's twenty on the Amiga press, four against twenty on the
/// Macintosh one. `zvm-cli` is a binary and cannot be linked from here, so the
/// half of the claim that says the CLI *uses* the seam is pinned in that crate,
/// by `media::tests::the_menu_lists_the_whole_release`.
#[test]
fn both_front_ends_offer_the_same_release() {
    let mut ran = 0;
    for press in PRESSES {
        let path = treasures_dir().join(press.member);
        if !path.exists() {
            continue;
        }
        ran += 1;
        let base = data_base(&format!("agree-{ran}"));
        let cli = builds_the_cli_offers(&path);
        let tui = builds_the_browser_offers(&path, &base);
        assert_eq!(
            cli.len(),
            press.games,
            "{}: zvm-cli offers {} games, not {}",
            press.member,
            cli.len(),
            press.games
        );
        assert_eq!(cli, tui, "{}: the two front-ends disagree about the release", press.member);
        let _ = std::fs::remove_dir_all(&base);
    }
    assert!(ran > 0 || !any_press_present(), "a press is present but none was enumerated");
}

/// The prerequisite (SQ-0961's third note): both presses are recognised as one
/// release, however each spells its volumes.
///
/// FALSIFICATION: drop the disk-word branch from `disk_set::group_indexed` and
/// the DiskCopy row reports `None` — which is what it did before the fix, and
/// what would have made the case above pass vacuously on it.
#[test]
fn each_press_is_one_release() {
    let mut ran = 0;
    for press in PRESSES {
        let path = treasures_dir().join(press.member);
        if !path.exists() {
            continue;
        }
        ran += 1;
        let members = app::disk_set::members(&path)
            .unwrap_or_else(|| panic!("{}: not recognised as a set at all", press.member));
        assert_eq!(members.len(), press.volumes, "{}: {members:?}", press.member);
        assert!(members.contains(&path), "{}: the named volume is not in its own set", press.member);
        // In disk order, and every member is that press's own spelling.
        let ext = path.extension().unwrap();
        assert!(members.iter().all(|m| m.extension() == Some(ext)), "{}: {members:?}", press.member);
    }
    assert!(ran > 0 || !any_press_present(), "a press is present but no set was checked");
}

/// Releases that carry **one** game must keep offering exactly one, whatever
/// reaching across the release now costs them.
///
/// Two shapes here and both matter. The Apple II and Commodore presses page one
/// story across every volume, so the siblings hold no story of their own and add
/// nothing. The DOS *Zork Zero* set is the SQ-0941 shape — disk 1 is a launcher,
/// disk 2 has `ZORK0.ZIP` — and the widening answers it from disk 2 while the
/// sibling sweep finds the SAME file there again. That is the case the
/// cross-volume fold exists for.
///
/// FALSIFICATION: remove the `seen` fold from `stories_across_the_release` and
/// the DOS row becomes two candidates, i.e. a three-floppy single-game release
/// grows a menu.
#[test]
fn a_one_game_release_still_opens_without_a_menu() {
    const ONE_GAME: &[&str] = &[
        "journey_s1.dsk",
        "shogun_s1.dsk",
        "zork_zero_1.dsk",
        "TRINITY1.D64",
        "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 1) [!].ima",
    ];
    let mut ran = 0;
    for name in ONE_GAME {
        let path = stories_dir().join(name);
        if !path.exists() {
            continue;
        }
        ran += 1;
        // The ROW count, not the count of distinct builds: a duplicate that
        // folded itself away in a set would be exactly the defect going unseen.
        let rows = rows_the_cli_offers(&path);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(rows.len(), 1, "{name}: a one-game release offers a menu of {names:?}");
    }
    assert!(
        ran > 0 || !ONE_GAME.iter().any(|n| stories_dir().join(n).exists()),
        "a one-game release is present but none was enumerated",
    );
}

// ── the source-level rule ─────────────────────────────────────────────────────

/// Crates whose `src/` may name [`blorb::medium::MountedDisk::mount`] freely.
///
/// Only `blorb`, which defines it. Every other crate is a caller, and a caller
/// asking about one platter is asking the wrong question.
const OWNS_THE_MOUNT: &str = "blorb";

/// The one production file allowed to call it: the seam itself, which mounts a
/// release's siblings plainly on purpose — mounting THEM across the set would
/// recurse.
const THE_SEAM: &str = "disk_set.rs";

/// Every `.rs` under `crates/*/src/`, as (`crate/relative/path`, source with
/// `#[cfg(test)] mod …` blocks removed).
fn production_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let crates = repo_root().join("crates");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&crates)
        .expect("the crates directory is part of the checkout")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()) != Some(OWNS_THE_MOUNT))
        .collect();
    dirs.sort();
    for dir in dirs {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
        collect_rs(&dir.join("src"), &name, &mut out);
    }
    out.sort();
    out
}

fn collect_rs(dir: &Path, label: &str, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            let sub = format!("{label}/{}", p.file_name().and_then(|n| n.to_str()).unwrap_or("?"));
            collect_rs(&p, &sub, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
            let Ok(src) = std::fs::read_to_string(&p) else { continue };
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            out.push((format!("{label}/{name}"), without_test_modules(&src)));
        }
    }
}

/// `src` with every `#[cfg(test)] mod … { … }` block cut out.
///
/// A unit test inside `src/` is a test, and several of them legitimately mount a
/// platter to establish a premise — `zvm-cli::media`'s
/// `this_front_end_claims_every_disk_blorb_can_open` mounts exactly what `blorb`
/// detected, which is the whole point of it. Only a `mod` is cut: a
/// `#[cfg(test)]` on a bare `fn` or `static` keeps its lines, and the rule erring
/// toward noise is what makes it safe to leave alone (the same trade
/// `palette_lock_discipline` makes).
fn without_test_modules(src: &str) -> String {
    let mut out = String::new();
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        // SQ-1242 put `app`'s in-crate `mod tests` blocks behind `t-*` Cargo
        // features, spelled `#[cfg(all(test, feature = "t-<group>"))]` (or
        // `any(feature = …)` for the couple shared across two groups) — both
        // prefixes are checked, or this scan stops recognising the boundary in
        // every file SQ-1242 rewrote and starts reading test code as production.
        let t = line.trim();
        if t != "#[cfg(test)]" && !t.starts_with("#[cfg(all(test,") {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let Some(head) = lines.peek() else { break };
        if !(head.contains("mod ") && head.contains('{')) {
            continue; // the attribute itself carries no call
        }
        // Skip the module by counting braces from its own line.
        let mut depth = 0i32;
        for l in lines.by_ref() {
            depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
            if depth <= 0 {
                break;
            }
        }
    }
    out
}

/// **No production code outside the seam mounts one platter** (SQ-0961).
///
/// Asserted over the source because there is no runtime signal for it: a call
/// site that mounts a platter works perfectly on every single-volume release,
/// which is most of the corpus, and goes wrong only on a set — and then quietly,
/// by offering fewer games or none rather than by failing. Three bugs came out
/// of it and the reasoning that was supposed to prevent the fourth is exactly
/// what was not written down anywhere a new call site would meet it.
///
/// Falsified by pointing any of the fixed call sites back at
/// `MountedDisk::mount`, which names that file here.
#[test]
fn no_production_code_mounts_the_platter_alone() {
    let (mut scanned, mut offenders) = (0usize, Vec::new());
    for (name, src) in production_sources() {
        scanned += 1;
        if name.ends_with(THE_SEAM) {
            continue;
        }
        // A plain substring scan: it cannot tell a call from a mention, so a file
        // naming the function in prose is reported. Reword the prose rather than
        // routing around the rule.
        if src.contains("MountedDisk::mount(") {
            offenders.push(name);
        }
    }
    assert!(
        offenders.is_empty(),
        "these production files mount ONE PLATTER: {offenders:?}\n\
         `blorb::medium::MountedDisk::mount` opens the named image and nothing else, and a \
         release is not always one image: the Apple II and Commodore presses page a single story \
         across four and five volumes, and a compilation's games sit on volumes nobody named. \
         Every front-end goes through `cli_host::disk_set::mount_at` (one story off a release) or \
         `cli_host::disk_set::stories_across_the_release` (all of them) — SQ-0941, SQ-0952, \
         SQ-0961 were three separate bugs from three call sites that did not.",
    );
    assert!(
        scanned >= 50,
        "only {scanned} production files were scanned — this case is looking in the wrong place \
         and would pass vacuously",
    );
}

/// The rule's own escape hatch is real: the seam does call it, and the check must
/// be seeing that file rather than passing because it never read it.
#[test]
fn the_seam_itself_is_what_the_rule_exempts() {
    let seam = production_sources()
        .into_iter()
        .find(|(n, _)| n.ends_with(THE_SEAM))
        .expect("cli-host/src/disk_set.rs is part of the checkout");
    assert!(
        seam.1.contains("MountedDisk::mount("),
        "the exempt file no longer calls the platter mount — if that is deliberate, delete the \
         exemption rather than leaving a rule with a hole in it",
    );
}
