//! SQ-1586: `app::host::hints` — resolving and booting a story's InvisiClues
//! hint session from the library, with no terminal.
//!
//! Extracted from what used to be the TUI-only `open_hints`/`hint_opening` in
//! `main.rs`; the acceptance here is that an embedding host reaches the same
//! outcome the TUI always has: `available` agrees with what `open` actually
//! does, `open` boots the hint VM and skips the InvisiClues narrow-screen
//! banner, and a story with no hint sidecar answers "no" / `Ok(None)` with the
//! TUI's own message text.
//!
//! | fixture | role |
//! |---|---|
//! | `zork1-r88-s840726.z3` + sibling `zork1izm.z5` | waitingforgo naming, banner-skip case |
//! | `zork2-r48-s840904.z3` + sibling `zork2inv.z5` | SLAG naming |
//! | `zork1-invclues-r52-s871125.z5` | Solid Gold release — NOT a hint sidecar |
//!
//! All three live only in the gitignored `stories/` (not the fetched-fixtures
//! manifest — they are Infocom hint files, not freely redistributable), so
//! every case here skips vacuously without a local `stories/` checkout, the
//! same CI-safe pattern `fixture_paths` documents.

use std::path::{Path, PathBuf};

use app::config::Config;
use app::hints;
use app::hints::HintStory;
use app::host::hints::{available, no_hint_message, open, HintAvailability};
use app::ifid::compute_ifid;

/// The real `stories/` directory (not the fetched-fixtures one): the hint
/// files this suite needs are Infocom copyrighted material, never fetched by
/// `scripts/fetch-fixtures.sh`, and only ever present via a developer's own
/// `stories/` checkout (symlinked into a worktree per `CLAUDE.md`).
fn stories_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Read `name` from `stories/`, or `None` if it is not there — the case skips.
fn read_story(name: &str) -> Option<(PathBuf, Vec<u8>)> {
    let path = stories_dir().join(name);
    let bytes = std::fs::read(&path).ok()?;
    Some((path, bytes))
}

fn empty_index() -> hints::HintIndex {
    // An empty per-IFID association table: `load_hint_index` on a scratch dir
    // that holds no `hints/index.toml` yields exactly this, with no disk I/O
    // needed to construct one directly.
    let home = app::scratch_dir("sq1586-empty-hint-index");
    hints::load_hint_index(&home)
}

#[test]
fn available_and_open_agree_on_zork1_izm_and_skip_its_banner() {
    let Some((story_path, story_bytes)) = read_story("zork1-r88-s840726.z3") else {
        eprintln!("SKIP: stories/zork1-r88-s840726.z3 absent");
        return;
    };
    if !stories_dir().join("zork1izm.z5").is_file() {
        eprintln!("SKIP: stories/zork1izm.z5 absent");
        return;
    }
    let ifid = compute_ifid(&story_bytes);
    let index = empty_index();

    assert_eq!(
        available(&story_path, HintStory::new(&ifid, ""), &index),
        HintAvailability::Available,
        "zork1izm.z5 sits beside zork1-r88-s840726.z3, so a hint source resolves"
    );

    let cfg = Config::default();
    let session = open(&story_path, HintStory::new(&ifid, ""), &index, &[], &cfg, None)
        .expect("a resolved hint source boots")
        .expect("available() said yes, so open() must find the same source");

    let opening = session.transcript.join("\n");
    assert!(
        !hints::is_narrow_screen_warning(&opening),
        "the narrow-screen banner should have been auto-skipped, got: {opening}"
    );
    assert_eq!(session.label, "zork1izm.z5");

    // zork1izm.z5's topic menu is a GRID (upper-window) screen, not lower-window
    // text — `hint_opening`'s own doc says so ("the menu lives in the upper
    // window") and `hints_tab` draws it separately from `transcript`
    // for exactly that reason. So "the first screen is the hint menu" is read
    // off the companion VM's live grid, not off `session.transcript`.
    let app::state::HintSource::Zcode(vm) = &session.source;
    let model = app::session::screen_model_from_machine(&vm.machine);
    let grid = model.grid().expect("the izm menu is drawn in a grid (upper) window");
    let nonblank = grid.cells.iter().filter(|c| c.ch != ' ').count();
    assert!(
        grid.active_rows > 0 && nonblank > 0,
        "the hint topic menu is drawn on screen (active_rows={}, nonblank cells={})",
        grid.active_rows,
        nonblank,
    );
    assert_eq!(
        vm.pending_input(),
        app::session::InputKind::Char,
        "the izm menu navigates by single keypress"
    );
}

#[test]
fn available_and_open_agree_on_zork2_inv_slag_naming() {
    let Some((story_path, story_bytes)) = read_story("zork2-r48-s840904.z3") else {
        eprintln!("SKIP: stories/zork2-r48-s840904.z3 absent");
        return;
    };
    if !stories_dir().join("zork2inv.z5").is_file() {
        eprintln!("SKIP: stories/zork2inv.z5 absent");
        return;
    }
    let ifid = compute_ifid(&story_bytes);
    let index = empty_index();

    assert_eq!(
        available(&story_path, HintStory::new(&ifid, ""), &index),
        HintAvailability::Available,
        "zork2inv.z5 sits beside zork2-r48-s840904.z3, so a hint source resolves"
    );

    let cfg = Config::default();
    let session = open(&story_path, HintStory::new(&ifid, ""), &index, &[], &cfg, None)
        .expect("a resolved hint source boots")
        .expect("available() said yes, so open() must find the same source");

    let opening = session.transcript.join("\n");
    assert!(!opening.trim().is_empty(), "the hint program's first screen is not blank");
    assert!(
        opening.contains("InvisiClues") && opening.contains("ZORK"),
        "the first screen names the InvisiClues booklet, got: {opening}"
    );
    assert_eq!(session.label, "zork2inv.z5");
}

#[test]
fn a_story_with_no_hint_sidecar_says_no_and_returns_ok_none() {
    // A fixture directory with no hint file at all: fixture_paths' fetched
    // copy of a freely redistributable story has no InvisiClues sibling.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let story_path = manifest.join("tests/fixtures/stories/Tangle.z5");
    if !story_path.is_file() {
        eprintln!("SKIP: {} absent", story_path.display());
        return;
    }
    let story_bytes = std::fs::read(&story_path).expect("read the story");
    let ifid = compute_ifid(&story_bytes);
    let index = empty_index();

    assert_eq!(
        available(&story_path, HintStory::new(&ifid, ""), &index),
        HintAvailability::None,
        "Tangle.z5 has no hint sidecar anywhere lanthorn looks"
    );

    let cfg = Config::default();
    let result = open(&story_path, HintStory::new(&ifid, ""), &index, &[], &cfg, None).expect("no hint source is not an error");
    assert!(result.is_none(), "open() finds nothing, exactly as available() said");
}

#[test]
fn solid_gold_release_is_not_treated_as_a_hint_sidecar() {
    // zork1-invclues-r52-s871125.z5 carries a `-r<digits>-s<digits>` marker, so
    // `hints::is_hint_sidecar` excludes it (it is a full Solid Gold GAME with
    // built-in clues, not a standalone hint file) — and `resolve_hint_source`,
    // which `available`/`open` both run through, uses that same exclusion.
    let Some((_, invclues_bytes)) = read_story("zork1-invclues-r52-s871125.z5") else {
        eprintln!("SKIP: stories/zork1-invclues-r52-s871125.z5 absent");
        return;
    };
    let Some((_, story_bytes)) = read_story("zork1-r88-s840726.z3") else {
        eprintln!("SKIP: stories/zork1-r88-s840726.z3 absent");
        return;
    };
    assert!(
        !hints::is_hint_sidecar("zork1-invclues-r52-s871125.z5"),
        "the excluded-by-release-serial rule this case depends on"
    );

    // Isolated scratch dir whose ONLY sibling of the plain Zork I story is the
    // Solid Gold release: if it were (wrongly) treated as a hint sidecar,
    // `available`/`open` would find it.
    let home = app::scratch_dir("sq1586-solid-gold-not-sidecar");
    std::fs::create_dir_all(&home).expect("create scratch dir");
    let plain_story = home.join("zork1-r88-s840726.z3");
    std::fs::write(&plain_story, &story_bytes).expect("write plain story");
    std::fs::write(home.join("zork1-invclues-r52-s871125.z5"), &invclues_bytes)
        .expect("write Solid Gold sibling");

    let ifid = compute_ifid(&story_bytes);
    let index = empty_index();
    assert_eq!(
        available(&plain_story, HintStory::new(&ifid, ""), &index),
        HintAvailability::None,
        "a Solid Gold release beside the story must not be picked up as its hint sidecar"
    );
    let cfg = Config::default();
    let result =
        open(&plain_story, HintStory::new(&ifid, ""), &index, &[], &cfg, None).expect("no hint source is not an error");
    assert!(result.is_none(), "open() must not open the Solid Gold release as a hint VM");

    let _ = std::fs::remove_dir_all(&home);
}

/// SQ-1689: a story played from a disk image is named for the box, so only its
/// MOUNTED identity can find its own hint file among many in the folder.
/// Specimen: `stories/Zork I - The Great Underground Empire.adf` (Zork I r88,
/// IFID ZCODE-88-840726) beside `zork1izm.z5`, `bzorkizm.z5`, `zork3inv.z5`.
#[test]
fn zork1_adf_finds_its_own_hint_file_in_game() {
    let adf = stories_dir().join("Zork I - The Great Underground Empire.adf");
    if !adf.is_file() || !stories_dir().join("zork1izm.z5").is_file() {
        eprintln!("SKIP: Zork I ADF specimen or zork1izm.z5 absent");
        return;
    }
    let (loaded, _disk) = hints::load_mounted_story_from(&adf, None).expect("the ADF mounts");
    let ifid = compute_ifid(loaded.bytes());
    assert_eq!(ifid, "ZCODE-88-840726-A129", "mounted story IFID, not the filename");
    let index = empty_index();

    assert_eq!(available(&adf, HintStory::new(&ifid, ""), &index), HintAvailability::Available);
    let session = open(&adf, HintStory::new(&ifid, ""), &index, &[], &Config::default(), None)
        .expect("a resolved hint source boots")
        .expect("available() said yes, so open() must find the same source");
    assert_eq!(session.label, "zork1izm.z5");
    eprintln!("RAN: zork1 ADF -> {}", session.label);
}

// ── SQ-1690: the per-game documents folder, the chooser, the message ─────────

/// A scratch library: a story, a bootable stand-in hint program (the fetched
/// `minizork` fixture under a hint-file name) and an empty documents folder.
/// `None` without the fixture (CI fetches it).
struct DocsLib {
    story: PathBuf,
    docs: PathBuf,
    user: PathBuf,
    hint_bytes: Vec<u8>,
}

fn docs_lib(tag: &str) -> Option<DocsLib> {
    let hint_bytes = std::fs::read(crate::fixture_paths::fixture_path("minizork-r34-s871124.z3")).ok()?;
    let lib = app::scratch_dir(tag);
    let story = lib.join("zork1.z3");
    std::fs::write(&story, &hint_bytes).unwrap();
    let docs = lib.join("Zork [tuid]");
    std::fs::create_dir_all(&docs).unwrap();
    let user = app::scratch_dir("sq1690-user");
    Some(DocsLib { story, docs, user, hint_bytes })
}

fn label_of(session: &app::state::HintSession) -> &str {
    &session.label
}

#[test]
fn sq1690_the_documents_folder_is_searched_first_and_a_lone_odd_name_is_accepted() {
    let Some(l) = docs_lib("sq1690-host-first") else { return };
    // The old home also has a sidecar; the folder must win. Its name matches
    // nothing about the story: the TUID proves the game.
    std::fs::write(l.story.with_file_name("zork1inv.z3"), &l.hint_bytes).unwrap();
    std::fs::write(l.docs.join("clues-from-the-web-hints.z3"), &l.hint_bytes).unwrap();
    let index = hints::load_hint_index(&l.user);
    let story = HintStory::new("IFID", "Zork").with_documents(Some(&l.docs));
    assert_eq!(available(&l.story, story, &index), HintAvailability::Available);
    let session = open(&l.story, story, &index, &[], &Config::default(), None).unwrap().expect("opens");
    assert_eq!(label_of(&session), "clues-from-the-web-hints.z3", "documents first");
}

#[test]
fn sq1690_an_empty_documents_folder_falls_back_to_the_sidecar_beside_the_story() {
    let Some(l) = docs_lib("sq1690-host-fallback") else { return };
    std::fs::write(l.story.with_file_name("zork1inv.z3"), &l.hint_bytes).unwrap();
    let index = hints::load_hint_index(&l.user);
    let story = HintStory::new("IFID", "Zork").with_documents(Some(&l.docs));
    let session = open(&l.story, story, &index, &[], &Config::default(), None).unwrap().expect("opens");
    assert_eq!(label_of(&session), "zork1inv.z3");
}

#[test]
fn sq1690_tied_candidates_are_offered_and_a_pick_is_remembered() {
    let Some(l) = docs_lib("sq1690-host-choose") else { return };
    for n in ["aaa-inv.z3", "bbb-inv.z3"] {
        std::fs::write(l.docs.join(n), &l.hint_bytes).unwrap();
    }
    let index = hints::load_hint_index(&l.user);
    let story = HintStory::new("IFID", "Zork").with_documents(Some(&l.docs));
    let HintAvailability::Choose(candidates) = available(&l.story, story, &index) else {
        panic!("two unrankable hint programs are a choice")
    };
    assert_eq!(candidates, vec![l.docs.join("aaa-inv.z3"), l.docs.join("bbb-inv.z3")]);
    assert!(open(&l.story, story, &index, &[], &Config::default(), None).unwrap().is_none(), "nothing opens until picked");

    app::host::hints::remember(&l.user, "IFID", &candidates[1]).unwrap();
    let index = hints::load_hint_index(&l.user); // a fresh load, as the next run does
    assert_eq!(available(&l.story, story, &index), HintAvailability::Available);
    let session = open(&l.story, story, &index, &[], &Config::default(), None).unwrap().expect("opens");
    assert_eq!(label_of(&session), "bbb-inv.z3");
}

#[test]
fn sq1690_the_no_hint_message_names_the_folder_or_says_to_link_first() {
    let linked = no_hint_message(Some(Path::new("/lib/documents/Zork [t]")));
    assert!(linked.contains("/lib/documents/Zork [t]"), "{linked}");
    assert!(linked.contains("documents folder"), "{linked}");
    assert!(linked.contains("Download hints"), "{linked}");
    let unlinked = no_hint_message(None);
    assert!(unlinked.contains("link this game to IFDB"), "{unlinked}");
    assert!(unlinked.contains("Download hints"), "{unlinked}");
    for m in [&linked, &unlinked] {
        assert!(!m.contains("/hints"), "the command that never existed is gone: {m}");
    }
}


/// The text of each active row of the hint program's upper (grid) window, trailing blanks kept.
fn grid_rows(session: &app::state::HintSession) -> Vec<String> {
    let app::state::HintSource::Zcode(vm) = &session.source;
    let model = app::session::screen_model_from_machine(&vm.machine);
    let grid = model.grid().expect("the hint program draws a grid (upper) window");
    let cols = grid.cols as usize;
    (0..grid.active_rows as usize)
        .map(|r| grid.cells[r * cols..(r + 1) * cols].iter().map(|c| c.ch).collect())
        .collect()
}

/// SQ-1753: a host can boot a hint program at its panel's real size. `bzorkizm.z5` reads its
/// width at boot, so 40 columns must centre the title inside 40 (not cut it off), while
/// `None` still boots the default 80x24.
#[test]
fn a_hint_program_boots_at_the_size_the_host_gives_it() {
    let Some((story_path, story_bytes)) = read_story("beyondzork-r57-s871221.z5") else {
        eprintln!("SKIP: stories/beyondzork-r57-s871221.z5 absent");
        return;
    };
    if !stories_dir().join("bzorkizm.z5").is_file() {
        eprintln!("SKIP: stories/bzorkizm.z5 absent");
        return;
    }
    let ifid = compute_ifid(&story_bytes);
    let index = empty_index();
    let cfg = Config::default();
    let boot = |screen| {
        open(&story_path, HintStory::new(&ifid, ""), &index, &[], &cfg, screen)
            .expect("the hint file boots")
            .expect("bzorkizm.z5 sits beside the story")
    };

    // At 40 columns the program is told 40 and lays out for it: nothing it draws is a cut-off
    // fragment of the 80-column layout (its 41-character title does not fit, so it omits it).
    let narrow = boot(Some((24, 40)));
    let app::state::HintSource::Zcode(vm) = &narrow.source;
    assert_eq!(
        (vm.machine.mem.read_byte(0x20), vm.machine.mem.read_byte(0x21)),
        (24, 40),
        "the program is told the panel's size at boot"
    );
    let rows = grid_rows(&narrow);
    assert!(rows.iter().all(|r| !r.contains("Beyond Zork: The Coc")), "no clipped title fragment: {rows:?}");
    assert!(rows.iter().any(|r| r.contains("ENTER = select item   Q = quit program")), "the menu fits 40: {rows:?}");

    // At 56 the 41-character title fits, and is centred in 56 (not in 80, and not clipped).
    let mid = boot(Some((24, 56)));
    let rows = grid_rows(&mid);
    let title = rows.iter().find(|r| r.contains("Beyond Zork")).unwrap_or_else(|| panic!("title row in {rows:?}"));
    assert_eq!(title.chars().count(), 56, "the title row is the panel's width: {title:?}");
    assert!(title.contains("Beyond Zork: The Coconut of Quendor"), "the title is not cut off: {title:?}");
    let lead = title.chars().take_while(|c| *c == ' ').count();
    let trail = title.chars().count() - lead - title.trim_end().chars().count() + lead;
    assert!((5..=10).contains(&lead) && trail >= 5, "the title is centred within 56: {title:?}");

    let default = boot(None);
    let app::state::HintSource::Zcode(vm) = &default.source;
    assert_eq!(
        (vm.machine.mem.read_byte(0x20), vm.machine.mem.read_byte(0x21)),
        (24, 80),
        "None still boots the default 80x24"
    );
}
