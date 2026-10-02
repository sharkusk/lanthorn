//! SQ-1676: several players, one install.
//!
//! One shared catalogue (IFDB info and covers), per-player saves and settings.
//! Lanthorn takes a player NAME and never authenticates it. These cases hold the
//! split to what the quest decided:
//!
//! * the default player's layout is byte-for-byte today's;
//! * a named player's saves and sidecars live under `<user>/users/<name>/saves/`
//!   while metadata, covers and learned addresses stay in `<user>/saves/`;
//! * `config.toml` and `style.toml` layer shared -> player, and a player's write
//!   stores only what differs from what they inherit;
//! * nothing a player does removes catalogue files.
//!
//! Every case that boots a story uses the fetched `Tangle.z5` fixture and skips
//! when it is absent, like every real-media suite.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use app::config::{self, Cli, Config};
use app::data_roots::DataRoots;
use app::fetch_worker::{FetchOrder, FetchTarget, Fetcher};
use app::host::persist::save_state_now;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::ifdb::{FetchError, FetchOutcome, MetadataSource};
use app::ifiction::{IFiction, IfdbExt};
use app::launch_options::LaunchOverrides;
use clap::Parser;

use crate::fixture_paths::fixture_path;

fn story() -> Option<PathBuf> {
    let p = fixture_path("Tangle.z5");
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: {} absent", p.display());
        None
    }
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([9, 8, 7, 255]));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// IFDB, answering the same record for every story and a real PNG for its cover.
struct Ifdb;

impl Ifdb {
    fn record() -> IFiction {
        IFiction {
            title: Some("Shared Title".into()),
            author: Some("An Author".into()),
            ifdb: Some(IfdbExt {
                tuid: "abc123".into(),
                link: None,
                cover_url: Some("https://ifdb.example/cover".into()),
                average_rating: None,
                rating_count: None,
            }),
            ..Default::default()
        }
    }
}

impl MetadataSource for Ifdb {
    fn fetch(&self, _ifid: &str) -> Result<FetchOutcome, FetchError> {
        Ok(FetchOutcome::Found(Box::new(Self::record())))
    }
    fn fetch_by_id(&self, _tuid: &str) -> Result<FetchOutcome, FetchError> {
        Ok(FetchOutcome::Found(Box::new(Self::record())))
    }
    fn fetch_cover(&self, _url: &str) -> Result<Vec<u8>, FetchError> {
        Ok(png())
    }
}

/// A metadata pass over one story the way the picker's `f` key does it: the
/// worker is handed the CATALOGUE base. Waits for the result.
fn fetch_via_worker(story: &Path, roots: &DataRoots) {
    let entry = app::picker::resolve_entry(story, roots).expect("the story is listable");
    let fetcher = Fetcher::new(Box::new(Ifdb), roots.catalogue().to_path_buf(), Duration::ZERO);
    fetcher.request(FetchOrder { stories: vec![FetchTarget::row(&entry)], forced: true, id_override: None });
    for _ in 0..4000 {
        if !fetcher.drain().is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the fetch worker never answered");
}

/// A metadata pass the way `--import-metadata` does it: through `roots`, which is
/// what has to pick the catalogue folder out of the pair.
fn fetch_via_import(story: &Path, roots: &DataRoots) {
    let row = app::metadata_import::ImportRow {
        path: story.to_path_buf(),
        ifdb_tuid: Some("abc123".into()),
        ..Default::default()
    };
    let outcome = app::metadata_import::import_row(&row, roots, &Ifdb);
    assert!(matches!(outcome, app::metadata_import::RowOutcome::FetchedById { .. }), "{outcome:?}");
}

fn boot(story: &Path, home: &Path, roots: DataRoots) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let cfg = Config {
        user_dir: home.to_path_buf(),
        config_file: home.join("config.toml"),
        random_seed: Some(1),
        ..Config::default()
    };
    let req = BootRequest {
        story_path: story.to_path_buf(),
        disk_entry: None,
        overrides: &overrides,
        cfg,
        roots,
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: false,
    };
    boot_story(req, &mut QuietBoot).expect("the story boots headlessly")
}

fn save(b: &mut BootedStory) {
    let out = save_state_now(&mut *b.session, &b.mapper, &b.state, &b.ifid, &b.arc_file);
    assert!(matches!(out, app::host::persist::ExitSave::Saved), "{out:?}");
}

fn badges(story: &Path, roots: &DataRoots, home: &Path) -> app::picker::RowBadges {
    let entry = app::picker::resolve_entry(story, roots).unwrap();
    app::picker::compute_row_badges(&entry, roots, &app::hints::load_hint_index(home))
}

const KEY: &str = "Tangle.z5.save";

// ── the default player ───────────────────────────────────────────────────────

#[test]
fn the_default_player_keeps_todays_layout() {
    let Some(story) = story() else { return };
    let home = app::scratch_dir("mp-default");
    let roots = DataRoots::resolve(&home, None, None);
    let mut b = boot(&story, &home, roots.clone());

    // Saves, and metadata fetched through either door, all in the one folder.
    let folder = home.join("saves").join(KEY);
    assert_eq!(b.game_dir, folder);
    save(&mut b);
    assert!(folder.join("default.lanthorn").is_file());
    fetch_via_worker(&story, &roots);
    assert!(folder.join("info.json").is_file());
    assert!(folder.join("cover.png").is_file());
    assert!(!home.join("users").exists(), "no player tree appears for the default player");

    // A settings write lands in <user>/config.toml, stamped as before.
    let mut cfg = config::resolve_at(&home);
    cfg.volume = 33;
    config::write_config_file(&cfg).unwrap();
    let written = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(written.contains("volume = 33"), "{written}");
    assert!(written.contains("version"), "the default player's file is stamped as ever: {written}");
    let _ = std::fs::remove_dir_all(&home);
}

// ── a named player ───────────────────────────────────────────────────────────

#[test]
fn a_named_players_saves_go_to_their_own_tree_and_metadata_to_the_catalogue() {
    let Some(story) = story() else { return };
    let home = app::scratch_dir("mp-bob");
    let bob = DataRoots::resolve(&home, None, Some("bob"));
    let mut b = boot(&story, &home, bob.clone());

    let bobs = home.join("users/bob/saves").join(KEY);
    let shared = home.join("saves").join(KEY);
    assert_eq!(b.game_dir, bobs);
    save(&mut b);
    assert!(bobs.join("default.lanthorn").is_file());
    assert!(!shared.join("default.lanthorn").exists(), "a save never lands in the shared tree");

    // A fetch (through either door) lands in the catalogue, not bob's folder.
    fetch_via_worker(&story, &bob);
    fetch_via_import(&story, &bob);
    assert!(shared.join("info.json").is_file());
    assert!(shared.join("cover.png").is_file());
    assert!(!bobs.join("info.json").exists());
    assert!(!bobs.join("cover.png").exists());

    // The default player (and anyone else) sees what bob fetched.
    let default = DataRoots::resolve(&home, None, None);
    let seen = app::picker::resolve_entry(&story, &default).unwrap();
    assert_eq!(seen.title, "Shared Title");
    let amy = DataRoots::resolve(&home, None, Some("amy"));
    assert_eq!(app::picker::resolve_entry(&story, &amy).unwrap().title, "Shared Title");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn players_cannot_see_each_others_saves() {
    let Some(story) = story() else { return };
    let home = app::scratch_dir("mp-isolated");
    let default = DataRoots::resolve(&home, None, None);
    let bob = DataRoots::resolve(&home, None, Some("bob"));

    let mut b = boot(&story, &home, bob.clone());
    save(&mut b);
    assert!(badges(&story, &bob, &home).save, "bob sees his own save");
    assert!(!badges(&story, &default, &home).save, "the default player does not see bob's");
    let entry = app::picker::resolve_entry(&story, &default).unwrap();
    assert!(app::persist_files::list_saves(&entry.game_dir(&default)).is_empty());

    // And the other way round.
    let amy = DataRoots::resolve(&home, None, Some("amy"));
    let mut d = boot(&story, &home, default.clone());
    save(&mut d);
    assert!(badges(&story, &default, &home).save);
    assert!(!badges(&story, &amy, &home).save, "amy sees neither");
    assert_eq!(app::persist_files::list_saves(&entry.game_dir(&bob)).len(), 1, "bob still sees only his own");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_catalogue_only_folder_reads_as_unplayed() {
    let Some(story) = story() else { return };
    let home = app::scratch_dir("mp-unplayed");
    let default = DataRoots::resolve(&home, None, None);
    let bob = DataRoots::resolve(&home, None, Some("bob"));
    // Bob browses and fetches; the catalogue folder now holds metadata and no save.
    fetch_via_worker(&story, &bob);
    assert!(home.join("saves").join(KEY).join("info.json").is_file());
    assert!(!badges(&story, &default, &home).save, "metadata alone is not a played game");
    assert!(!badges(&story, &bob, &home).save);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn nothing_a_player_does_removes_catalogue_files() {
    let Some(story) = story() else { return };
    let home = app::scratch_dir("mp-delete");
    let default = DataRoots::resolve(&home, None, None);
    let mut b = boot(&story, &home, default.clone());
    fetch_via_worker(&story, &default);
    save(&mut b);
    let folder = b.game_dir.clone();
    std::fs::write(folder.join("room-global"), "1 2:3").unwrap();
    std::fs::write(folder.join("quick-save.lanthorn"), b"x").unwrap();
    let catalogue = ["info.json", "cover.png", "room-global"];

    // Every delete the player can reach: auto data, the quick-save, and each save by hand.
    app::storage::delete_auto_persistent(&folder);
    app::storage::delete_quick_save(&folder);
    for s in app::persist_files::list_saves(&folder) {
        let _ = app::persist_files::delete_save(&s.path);
    }
    assert!(!folder.join("default.lanthorn").exists(), "premise: the saves really went");
    for f in catalogue {
        assert!(folder.join(f).is_file(), "{f} survived the deletes");
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// The deletes above are the ones that exist today; this keeps it so. A new
/// `remove_dir_all` in production code would take a whole story folder, catalogue
/// and every other player's... file with it.
#[test]
fn production_code_never_removes_a_directory() {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let production = match text.find("#[cfg(all(test") {
            Some(i) => &text[..i],
            None => &text,
        };
        if f.ends_with("lib.rs") {
            continue; // `scratch_dir`, a test helper, clears its own fresh directory
        }
        assert!(!production.contains("remove_dir_all"), "{} removes a directory", f.display());
    }
}

// ── layered config ───────────────────────────────────────────────────────────

fn cli(home: &Path, player: Option<&str>) -> Cli {
    let mut args = vec!["lanthorn".to_string(), "--user-dir".into(), home.display().to_string()];
    if let Some(p) = player {
        args.push("--player".into());
        args.push(p.into());
    }
    Cli::parse_from(args)
}

fn keys_of(file: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    text.parse::<toml::Table>().unwrap().keys().cloned().collect()
}

#[test]
fn a_player_inherits_the_shared_config_and_writes_only_their_overrides() {
    let home = app::scratch_dir("mp-config");
    let shared = home.join("config.toml");
    std::fs::write(&shared, "volume = 40\nhistory_turns = 7\n").unwrap();

    let mut bob = config::resolve(&cli(&home, Some("bob")));
    assert_eq!(bob.volume, 40, "inherited");
    assert_eq!(bob.history_turns, 7, "inherited");
    assert_eq!(bob.config_file, home.join("users/bob/config.toml"));

    bob.volume = 55;
    config::write_config_file(&bob).unwrap();
    let file = home.join("users/bob/config.toml");
    assert_eq!(keys_of(&file), ["volume"], "only the override, no template, no version stamp");
    assert_eq!(std::fs::read_to_string(&shared).unwrap(), "volume = 40\nhistory_turns = 7\n", "the shared file is untouched");

    // A key bob never touched follows a later change to the shared file.
    std::fs::write(&shared, "volume = 40\nhistory_turns = 9\n").unwrap();
    let again = config::resolve(&cli(&home, Some("bob")));
    assert_eq!(again.history_turns, 9, "follows the shared file");
    assert_eq!(again.volume, 55, "bob's override holds");

    // Putting a key back to the inherited value is not an override: it is not added.
    let mut amy = config::resolve(&cli(&home, Some("amy")));
    amy.volume = 40;
    config::write_config_file(&amy).unwrap();
    assert!(keys_of(&home.join("users/amy/config.toml")).is_empty());

    // The default player is unchanged: their file is the shared one.
    let mut default = config::resolve(&cli(&home, None));
    assert_eq!(default.config_file, shared);
    default.history_turns = 11;
    config::write_config_file(&default).unwrap();
    assert!(std::fs::read_to_string(&shared).unwrap().contains("history_turns = 11"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn keymap_tables_layer_key_by_key() {
    let home = app::scratch_dir("mp-keymap");
    std::fs::write(home.join("config.toml"), "[keymap.global]\n\"ctrl+z\" = \"zoom-in\"\n").unwrap();
    std::fs::create_dir_all(home.join("users/bob")).unwrap();
    std::fs::write(home.join("users/bob/config.toml"), "[keymap.global]\n\"ctrl+x\" = \"zoom-out\"\n").unwrap();
    let bob = config::resolve(&cli(&home, Some("bob")));
    assert_eq!(bob.keymap.global.get("ctrl+z").map(String::as_str), Some("zoom-in"));
    assert_eq!(bob.keymap.global.get("ctrl+x").map(String::as_str), Some("zoom-out"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_players_broken_config_is_reported_and_never_overwritten() {
    let home = app::scratch_dir("mp-broken");
    std::fs::create_dir_all(home.join("users/bob")).unwrap();
    std::fs::write(home.join("users/bob/config.toml"), "volume = [oops\n").unwrap();
    let bob = config::resolve(&cli(&home, Some("bob")));
    assert!(bob.config_error.is_some());
    assert!(config::write_config_file(&bob).is_err());
    assert_eq!(std::fs::read_to_string(home.join("users/bob/config.toml")).unwrap(), "volume = [oops\n");
    let _ = std::fs::remove_dir_all(&home);
}

// ── layered style ────────────────────────────────────────────────────────────

#[test]
fn style_layers_shared_then_player() {
    let home = app::scratch_dir("mp-style");
    std::fs::write(home.join("style.toml"), "[map]\nbox_style = \"rounded\"\narrow_set = \"plain\"\n").unwrap();
    std::fs::create_dir_all(home.join("users/bob")).unwrap();
    std::fs::write(home.join("users/bob/style.toml"), "[map]\narrow_set = \"bold\"\n").unwrap();

    let bob = config::resolve(&cli(&home, Some("bob")));
    let (doc, warnings) = app::style::load_style_for(&bob);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(doc.symbols.arrow_set.as_deref(), Some("bold"), "the player's own value wins");
    assert_eq!(doc.symbols.box_style.as_deref(), Some("rounded"), "everything else inherited");

    // A key bob never set follows a later change to the shared file.
    std::fs::write(home.join("style.toml"), "[map]\nbox_style = \"double\"\narrow_set = \"plain\"\n").unwrap();
    let (doc, _) = app::style::load_style_for(&bob);
    assert_eq!(doc.symbols.box_style.as_deref(), Some("double"));
    assert_eq!(doc.symbols.arrow_set.as_deref(), Some("bold"));

    // The default player reads the shared file alone.
    let default = config::resolve(&cli(&home, None));
    let (doc, _) = app::style::load_style_for(&default);
    assert_eq!(doc.symbols.arrow_set.as_deref(), Some("plain"));
    let _ = std::fs::remove_dir_all(&home);
}

// ── choosing a player on the command line ───────────────────────────────────

fn lanthorn(home: &Path, player_flag: Option<&str>, env: Option<&str>) -> std::process::Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_lanthorn"));
    cmd.arg("--user-dir").arg(home).env_remove("LANTHORN_PLAYER").stdin(std::process::Stdio::null());
    if let Some(p) = player_flag {
        cmd.arg("--player").arg(p);
    }
    if let Some(e) = env {
        cmd.env("LANTHORN_PLAYER", e);
    }
    cmd.output().expect("lanthorn runs")
}

fn stderr(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn an_invalid_player_name_is_rejected_with_a_clear_error() {
    let home = app::scratch_dir("mp-invalid");
    for bad in ["../x", "a/b", ".hidden", "..", "has space", &"x".repeat(30)] {
        let o = lanthorn(&home, Some(bad), None);
        assert_eq!(o.status.code(), Some(2), "--player {bad:?}");
        assert!(stderr(&o).contains("invalid player name"), "--player {bad:?}: {}", stderr(&o));
    }
    // The environment is checked the same way.
    let o = lanthorn(&home, None, Some("../x"));
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("invalid player name"), "{}", stderr(&o));
    assert!(!home.join("users").exists(), "a rejected name creates nothing");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_flag_beats_the_environment_and_empty_means_the_default_player() {
    let home = app::scratch_dir("mp-flag-env");
    // With no story, a valid launch gets as far as "no story given"; that is the
    // proof the name was accepted.
    let accepted = |o: &std::process::Output| stderr(o).contains("no story given");
    assert!(accepted(&lanthorn(&home, Some("bob"), Some("../x"))), "a valid flag overrides a bad environment");
    assert!(accepted(&lanthorn(&home, Some(""), Some("../x"))), "an empty flag is the default player, whatever the environment says");
    assert!(accepted(&lanthorn(&home, None, Some(""))), "an empty environment is the default player");
    assert!(accepted(&lanthorn(&home, None, Some("bob"))), "the environment names a player");
    let _ = std::fs::remove_dir_all(&home);
}

// ── atomic metadata writes ───────────────────────────────────────────────────

#[test]
fn story_info_writes_are_atomic() {
    let dir = app::scratch_dir("mp-atomic");
    // A big record, so a truncating write has a wide window to be caught in.
    let info = |tag: &str| app::story_info::StoryInfo {
        format_version: app::story_info::FORMAT_VERSION,
        ifid: "ZCODE-1-000001".into(),
        fetched: None,
        probe: Some(app::story_info::ProbeMeta { probed_at: Some(format!("{tag}{}", "x".repeat(60_000))) }),
    };
    // Two players refreshing the same story at once: whatever a reader catches, it
    // is a whole file. A truncating write would show an empty or half file here.
    let writers: Vec<_> = (0..4)
        .map(|n| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                for i in 0..150 {
                    let _ = app::story_info::save(&dir, &info(&format!("{n}-{i}")));
                }
            })
        })
        .collect();
    let reader = {
        let dir = dir.clone();
        std::thread::spawn(move || {
            let mut torn = 0;
            for _ in 0..3000 {
                if let Ok(raw) = std::fs::read(app::story_info::info_path(&dir)) {
                    if serde_json::from_slice::<serde_json::Value>(&raw).is_err() {
                        torn += 1;
                    }
                }
            }
            torn
        })
    };
    for w in writers {
        w.join().unwrap();
    }
    assert_eq!(reader.join().unwrap(), 0, "a reader caught a partial info.json");
    assert!(app::story_info::load(&dir, "ZCODE-1-000001").is_some());
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".part"))
        .collect();
    assert!(leftovers.is_empty(), "no temp files left behind");
    let _ = std::fs::remove_dir_all(&dir);
}
