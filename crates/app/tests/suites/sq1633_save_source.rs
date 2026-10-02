//! SQ-1633: which physical copy of a story release a session was booted from
//! (the file/disk-image entry, and the machine when known), recorded in
//! `Meta::source` for DISPLAY only — e.g. "Continue on the Amiga version".
//!
//! **Two different disk images of the SAME Z-machine build (release+serial)
//! already share ONE save folder** via `storage::game_dir`'s `disk_story_key`
//! (SQ-0850) — for a non-v6 story, the key is `<slug>-r<release>-s<serial>`
//! with no per-medium suffix, so any medium carrying that exact build lands
//! in the same directory regardless of which physical disk it came off.
//! `real_media_releases.rs`'s own table names the fixture this suite drives:
//! *Beyond Zork* release 57 / serial 871221 (v5) "the SAME build as the
//! Amiga floppy, the Apple IIgs volume and the bare `.z5`, four media
//! agreeing".
//!
//! This quest adds a fact to `Meta` for display and must NOT change that
//! grouping, or which archive `resume_source` picks, in any way — the point
//! this suite exists to nail down. (A LOOSE story FILE of the same build is
//! keyed by its own FILENAME instead — `cli_host::storage`'s own doc: "A
//! loose story file is keyed by its basename, exactly as it always was" — so
//! it is a SEPARATE save folder from the disk images today, unrelated to
//! this quest, and this suite tests the grouping that actually exists rather
//! than asserting a wider claim.)
//!
//! `stories/` is gitignored (commercial media); every case here skips
//! vacuously when its fixture is absent, like every other real-media suite.

use std::path::{Path, PathBuf};

use app::archive::{MachineDto, SaveSource};
use app::config::Config;
use app::host::persist::save_state_now;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// A config rooted in a scratch home, so nothing here reads or writes the
/// real `~/.lanthorn` — mirrors `host_boot.rs`'s own `headless_config`.
fn headless_config(home: &Path) -> Config {
    Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        ..Config::default()
    }
}

/// Boot `story` exactly as a headless host would — mirrors `host_boot.rs`'s
/// own `boot()`. Both fixtures this suite drives are single-story images, so
/// `disk_entry: None` is exactly what a real launch would pass for either.
fn boot(story: PathBuf, cfg: Config, data_base: &Path) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg,
        roots: app::data_roots::DataRoots::single(data_base.to_path_buf()),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: false,
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

#[test]
fn two_disk_copies_of_one_release_share_one_game_dir_and_each_save_names_its_own_source() {
    let amiga_path = stories_dir().join("Beyond Zork - The Coconut of Quendor.adf");
    let iigs_path = stories_dir().join("Beyond Zork (1988)(Infocom).2mg");
    if !amiga_path.is_file() || !iigs_path.is_file() {
        eprintln!(
            "SKIP: {} and/or {} absent",
            amiga_path.display(),
            iigs_path.display()
        );
        return;
    }

    let home = app::scratch_dir("sq1633-two-copies");
    let data_base = home.join("saves");

    // Both copies booted into the SAME data_base, as two launches of the
    // player's library would be.
    let mut amiga = boot(amiga_path.clone(), headless_config(&home), &data_base);
    let mut iigs = boot(iigs_path.clone(), headless_config(&home), &data_base);

    // The whole point: one release, one shared save folder, unaffected by
    // which medium either copy was booted from.
    assert_eq!(
        amiga.game_dir, iigs.game_dir,
        "two disk images of the SAME build (r57/s871221) must land in ONE game_dir, exactly as before this quest"
    );

    // Each boot recorded its OWN source — the Amiga floppy...
    assert_eq!(
        amiga.state.source,
        SaveSource {
            story_file: Some("Beyond Zork - The Coconut of Quendor.adf".to_string()),
            disk_entry: None,
            machine: Some(MachineDto::Amiga),
        },
        "the Amiga boot must record its own file and machine"
    );
    // ...and the standalone Apple IIgs volume, a different filename and a
    // different machine, off the very same build.
    assert_eq!(
        iigs.state.source,
        SaveSource {
            story_file: Some("Beyond Zork (1988)(Infocom).2mg".to_string()),
            disk_entry: None,
            machine: Some(MachineDto::AppleII),
        },
        "the Apple IIgs boot must record its own file and machine"
    );

    // Save from each — into the two RESERVED slots (`arc_file`/
    // `quick_save_file`) their own `game_dir` computes, which — since the
    // folder is shared — are two files inside the SAME directory.
    assert_eq!(amiga.arc_file.parent(), iigs.quick_save_file.parent(), "both writes land under the one shared folder");
    let saved = save_state_now(&mut *amiga.session, &amiga.mapper, &amiga.state, &amiga.ifid, &amiga.arc_file);
    assert!(matches!(saved, app::host::persist::ExitSave::Saved), "the Amiga copy's save must succeed: {saved:?}");
    let saved = save_state_now(&mut *iigs.session, &iigs.mapper, &iigs.state, &iigs.ifid, &iigs.quick_save_file);
    assert!(matches!(saved, app::host::persist::ExitSave::Saved), "the Apple IIgs copy's save must succeed: {saved:?}");

    // Each save, read back, still names its OWN source — not the other copy's.
    let amiga_meta = app::archive::read_archive_meta(&amiga.arc_file).expect("Amiga save readable");
    assert_eq!(amiga_meta.source, amiga.state.source, "the Amiga save names the Amiga copy");
    let iigs_meta = app::archive::read_archive_meta(&iigs.quick_save_file).expect("Apple IIgs save readable");
    assert_eq!(iigs_meta.source, iigs.state.source, "the Apple IIgs save names the Apple IIgs copy");
    assert_ne!(amiga_meta.source, iigs_meta.source, "the two copies' sources must not collide");

    // And resume selection, which reads only `saved_at` and whether a slot
    // carries a resume point (`app::host::boot::resume_source`'s own doc),
    // still runs cleanly over a folder whose two slots now carry DIFFERENT
    // `source` values — it must not error, and must still resolve to one of
    // the two real saves just written.
    let picked = app::host::resume_source(&amiga.game_dir);
    assert!(picked.is_some(), "two real saves are on disk — resume_source must offer one of them");

    let _ = std::fs::remove_dir_all(&home);
}
