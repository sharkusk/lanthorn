//! SQ-0789 / SQ-0791 — the three doors into the picture-override mechanism, and
//! the boot-time choices only a launch can make.
//!
//! SQ-0734 landed the machinery: a named native archive beats a Blorb and beats
//! an `.adf`'s own `Pic.data`, its flavour picks the machine, and a name that
//! will not decode is loud. What was missing was any way to *reach* it without
//! hand-editing a file first — which is backwards for a choice you can only
//! judge by looking at it. Three doors, one mechanism:
//!
//!     story on the command line  ->  --pictures                  (SQ-0791)
//!     picked from the browser    ->  the launch-options dialog   (SQ-0789)
//!     persistent preference      ->  pictures = "…" in the game's config.toml
//!
//! Two properties matter more than the plumbing and are what this suite pins.
//!
//! **Discovery for DISPLAY is safe; discovery for PAIRING is not.** Enumerating
//! the archives that carry a story's name and showing them to a person is safe
//! for exactly the reason auto-pairing is unsafe: the person knows which game
//! they own and supplies the assertion the format cannot make. So the list
//! exists, and nothing consumes it automatically. The name filter narrows which
//! rows a person is *shown* and decides nothing — an archive under an unrelated
//! name stays reachable by being named, through `--pictures` or the `pictures`
//! key, which is where it always belonged.
//!
//! **The sidecar's "absent key = inherit" contract survives the checkbox.** A key
//! written at the value it already inherits is not the same as a key left absent
//! — it converts an inheritance into a pin — so the dialog writes only what the
//! user actually changed, and an untouched dialog writes nothing at all.
//!
//! Real archives are gitignored, so every case using one skips vacuously.

use std::path::PathBuf;

use app::launch_options::{
    derived_interpreter, discover_art_candidates, InterpreterSource, LaunchOptionsState,
    LaunchOverrides,
};
use app::styles::{per_game_config_path, read_per_game_interpreter_number, read_per_game_pictures};

use crate::fixture_paths::fixture_path;


fn tmp(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("launchopt-it-{tag}"))
}

/// Zork Zero is the case the whole feature exists for: one game, five renditions
/// of its art sitting side by side, and no way to tell them apart from a
/// filename. The dialog's list is what makes that a choice instead of a guess.
#[test]
fn every_rendition_of_zork_zero_is_offered_with_enough_to_choose_by() {
    let z0 = fixture_path("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return; // gitignored fixture
    }
    let found = discover_art_candidates(&z0, None);
    assert!(!found.is_empty(), "Zork Zero's archives sit beside it");

    for c in &found {
        // A bare filename is not enough to choose by; the point of parsing each
        // candidate is that the list can state what it actually is.
        assert!(c.pictures > 0, "{} lists no pictures", c.filename);
        assert!(c.part >= 1, "{} has no part number", c.filename);
        assert!(
            matches!(c.rendition, "Amiga" | "MCGA" | "EGA" | "CGA" | "EGA/CGA"),
            "{} got an unrecognised rendition label {:?}",
            c.filename,
            c.rendition
        );
        assert!(c.space_width == 320 || c.space_width == 640);
        // NO rendition carries a caveat any more (SQ-0815). Both of the ones
        // that stood here were fixed and then outlived their fix in this dialog:
        // SQ-0790's per-axis art scale drew 640-wide art at true aspect, and
        // SQ-0797 fused EGA's column dither. A warning nobody deletes is a
        // warning that goes on being read, so this asserts the absence rather
        // than the shape.
        assert!(
            c.caveat().is_none(),
            "{} still warns about something lanthorn has fixed: {:?}",
            c.filename,
            c.caveat()
        );
    }

    // The Blorb beside it is tier 1 and is never a pickable native archive; the
    // story file is not one either.
    assert!(found.iter().all(|c| !c.filename.eq_ignore_ascii_case("Zork0.blb")));
    assert!(found.iter().all(|c| !c.filename.eq_ignore_ascii_case("zork0.z6")));
}

/// A story library is usually one flat folder — `stories/` holds Arthur,
/// Journey, Shogun and Zork Zero together — so "every archive beside this story"
/// is mostly other games' art, and offering it all was a dialog that made the
/// user do the filtering. The list is now what its heading says: the archives
/// detected **for this story**.
///
/// This is the table the whole change rests on, pinned against the real library.
/// Every pairing here is one the normalised name test finds, and each names the
/// direction it needed: a story stem that contains the archive's (`zork0` inside
/// `zork0-r393-s890714`), an archive stem that is only a prefix of a longer game
/// name (`beyondzo`), a spaced disk-image title that a prefix rule could never
/// have matched (`James Clavell's Shogun`), and a pair that differ only in case.
#[test]
fn each_game_in_the_library_detects_its_own_archives_and_no_others() {
    // (story, must detect, must NOT detect)
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "zork0-r393-s890714.z6",
            &["zork0.cg1", "zork0.eg1", "zork0.mg1", "zork0.pic"],
            &["arthur.mg1", "journey.mg1", "shogun.mg1", "FMVPOKER.EG1", "beyondzo.mg1"],
        ),
        // `.eg2` is deliberately in the "must NOT" column for both multi-part
        // games: it is the back half of `.eg1`, which now loads it, and offering
        // it alone would be offering 101 of Arthur's 171 ids as if it were the
        // set (SQ-0798).
        (
            "arthur-r74-s890714.z6",
            &["arthur.cg1", "arthur.eg1", "arthur.mg1", "arthur.pic"],
            &["zork0.mg1", "journey.mg1", "shogun.mg1", "arthur.eg2"],
        ),
        (
            "journey-r83-s890706.z6",
            &["journey.cg1", "journey.eg1", "journey.mg1"],
            &["zork0.mg1", "arthur.mg1", "shogun.mg1", "journey.eg2"],
        ),
        (
            "shogun-r322-s890706.z6",
            &["shogun.cg1", "shogun.eg1", "shogun.mg1"],
            &["zork0.mg1", "arthur.mg1", "journey.mg1"],
        ),
        // `beyondzo` is the DOS 8.3 truncation of the game's name: a prefix of
        // the story stem, never the whole of it.
        ("beyondzork-r57-s871221.z5", &["beyondzo.mg1"], &["zork0.mg1", "arthur.mg1"]),
        // The renamed archive the escape hatch exists for — for the fan game it
        // was renamed FOR, the name now matches outright.
        ("fmvpoker.z6", &["FMVPOKER.EG1"], &["zork0.mg1", "arthur.mg1"]),
        // Disk images: a spaced, punctuated, article-carrying title on one side
        // and an 8.3 stem on the other. Only normalising both and testing for
        // containment in either direction connects them.
        ("James Clavell's Shogun.adf", &["shogun.mg1"], &["zork0.mg1", "journey.mg1"]),
        (
            "Beyond Zork - The Coconut of Quendor.adf",
            &["beyondzo.mg1"],
            &["zork0.mg1", "arthur.mg1"],
        ),
    ];

    for (story, wanted, unwanted) in cases {
        let path = fixture_path(story);
        if !path.is_file() {
            continue; // gitignored fixture
        }
        let found = discover_art_candidates(&path, None);
        let names: Vec<&str> = found.iter().map(|c| c.filename.as_str()).collect();
        for w in *wanted {
            // Only assert on archives this library actually has.
            if fixture_path(w).is_file() {
                assert!(
                    found.iter().any(|c| c.filename.eq_ignore_ascii_case(w)),
                    "{story} must detect {w}; got {names:?}"
                );
            }
        }
        for u in *unwanted {
            assert!(
                !found.iter().any(|c| c.filename.eq_ignore_ascii_case(u)),
                "{story} must NOT be offered {u}; got {names:?}"
            );
        }
    }
}

/// A multi-part EGA set is ONE row, and it advertises the whole set (SQ-0798).
///
/// Before this, Arthur offered `arthur.eg1` and `arthur.eg2` as two choices, and
/// both were wrong: the first loaded 97 of 137 pictures and the second is not an
/// archive at all, just the back half of one. Now the first carries the second
/// and the second is not offered.
///
/// The row's picture count is the merged directory — 171 for Arthur, which is
/// exactly what `arthur.mg1` holds, the same artwork shipped undivided. The
/// panel and the dialog both render from this list, so pinning it here pins both.
#[test]
fn a_split_ega_archive_is_offered_as_one_entry_carrying_both_files() {
    for (story_file, head, tail, want_entries, want_mcga) in [
        ("arthur-r74-s890714.z6", "arthur.eg1", "arthur.eg2", 171usize, "arthur.mg1"),
        ("journey-r83-s890706.z6", "journey.eg1", "journey.eg2", 135, "journey.mg1"),
    ] {
        let path = fixture_path(story_file);
        if !path.is_file() || !fixture_path(tail).is_file() {
            continue; // gitignored fixtures
        }
        let found = discover_art_candidates(&path, None);
        let names: Vec<&str> = found.iter().map(|c| c.filename.as_str()).collect();

        let c = found
            .iter()
            .find(|c| c.filename.eq_ignore_ascii_case(head))
            .unwrap_or_else(|| panic!("{head} must be listed; got {names:?}"));
        assert_eq!(c.parts, 2, "{head} is two files");
        assert_eq!(c.part, 1, "and the row names the first of them");
        assert_eq!(c.pictures, want_entries, "{head} advertises the WHOLE set");
        assert!(
            !found.iter().any(|x| x.filename.eq_ignore_ascii_case(tail)),
            "{tail} is half an archive and must not be a separate choice; got {names:?}",
        );

        // The undivided MCGA release of the same artwork is the oracle: if the
        // merged count were wrong, the two renditions would stop agreeing on how
        // many pictures this game has. (Journey's EGA set carries one extra —
        // id 59, a solid-colour blanking plate no other rendition has.)
        if let Some(m) = found.iter().find(|x| x.filename.eq_ignore_ascii_case(want_mcga)) {
            assert_eq!(m.parts, 1, "{want_mcga} is a single disk");
            let slack = usize::from(story_file.starts_with("journey"));
            assert_eq!(
                c.pictures,
                m.pictures + slack,
                "{head} and {want_mcga} are the same game's art",
            );
        }
    }
}

/// A continuation whose part 1 is NOT on disk is still listed — it is all there
/// is, and it says so.
///
/// The collapse rule is "hide a part whose predecessor is present", not "hide
/// anything numbered above 1". Someone with only disk two should see disk two,
/// flagged, rather than an empty dialog.
#[test]
fn a_lone_continuation_is_still_offered_and_says_which_part_it_is() {
    let eg2 = fixture_path("arthur.eg2");
    if !eg2.is_file() {
        return; // gitignored fixture
    }
    let dir = tmp("lone-part2");
    std::fs::copy(&eg2, dir.join("arthur.eg2")).unwrap();
    std::fs::write(dir.join("arthur.z6"), b"x").unwrap();

    let found = discover_art_candidates(&dir.join("arthur.z6"), None);
    let c = found
        .iter()
        .find(|c| c.filename.eq_ignore_ascii_case("arthur.eg2"))
        .expect("with no part 1 beside it, part 2 is the only choice there is");
    assert_eq!((c.part, c.parts), (2, 1));
    assert_eq!(c.pictures, 101, "and it advertises only what it actually holds");

    // Put part 1 back and the row disappears into it.
    std::fs::copy(fixture_path("arthur.eg1"), dir.join("arthur.eg1")).unwrap();
    let found = discover_art_candidates(&dir.join("arthur.z6"), None);
    assert!(!found.iter().any(|c| c.filename.eq_ignore_ascii_case("arthur.eg2")));
    assert_eq!(
        found.iter().find(|c| c.filename.eq_ignore_ascii_case("arthur.eg1")).map(|c| c.pictures),
        Some(171),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The list is sorted and stable, and it never carries the story itself or a
/// Blorb — the two files most likely to be sitting right beside it.
#[test]
fn the_detected_list_is_sorted_and_holds_only_native_archives() {
    let z0 = fixture_path("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let found = discover_art_candidates(&z0, None);
    let names: Vec<String> = found.iter().map(|c| c.filename.to_lowercase()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "the list is alphabetical");
    assert_eq!(found, discover_art_candidates(&z0, None), "and stable across calls");
    assert!(found.iter().all(|c| !c.filename.eq_ignore_ascii_case("Zork0.blb")));
    assert!(found.iter().all(|c| !c.filename.to_lowercase().ends_with(".z6")));
}

/// The dialog is a door into SQ-0734's mechanism, not a second one: the name it
/// hands over is exactly what `pictures = "…"` would have said, and the
/// resolution that follows is the one already tested by `picture_override.rs`.
#[test]
fn a_chosen_rendition_becomes_the_same_override_the_config_key_would_have() {
    let z0 = fixture_path("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let mut st = LaunchOptionsState::new("Zork Zero", &z0, None, None, Some(6), None);
    let Some(i) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.mg1"))
    else {
        return; // this library has no MCGA rendition
    };
    st.art = i + 1;
    let ov = st.overrides();
    assert_eq!(ov.pictures.as_deref(), Some("zork0.mg1"));

    // And that name resolves through the very same entry point boot uses.
    let over = app::graphics::PictureOverride::resolve_with_session(
        &z0,
        &tmp("session"),
        ov.pictures.as_deref(),
    );
    assert!(
        matches!(over, app::graphics::PictureOverride::Loaded { .. }),
        "the session name must load exactly as the config key does, got {over:?}"
    );
    // …and it beats an empty sidecar and the Blorb both, which is the precedence
    // SQ-0734 set: naming an archive is an instruction, not a hint.
    assert_eq!(over.flavour(), Some(blorb::infocom_pics::Flavour::Pc));
}

/// The session name outranks the config key. A user with `pictures` set who
/// passes `--pictures` (or picks something else in the dialog) needs the more
/// specific, more recent instruction to win — and the help text says so.
#[test]
fn a_launch_time_name_outranks_the_games_own_config_key() {
    let z0 = fixture_path("zork0-r393-s890714.z6");
    let pic = fixture_path("zork0.pic");
    let mg1 = fixture_path("zork0.mg1");
    if !z0.is_file() || !pic.is_file() || !mg1.is_file() {
        return;
    }
    let game_dir = tmp("outrank");
    std::fs::write(per_game_config_path(&game_dir), "pictures = \"zork0.pic\"\n").unwrap();

    // With no session name, the sidecar decides: the Amiga archive, hence Amiga.
    let sidecar = app::graphics::PictureOverride::resolve(&z0, &game_dir);
    assert_eq!(sidecar.flavour(), Some(blorb::infocom_pics::Flavour::AmigaMac));

    // With one, it wins outright — a different flavour, so a different machine.
    let session =
        app::graphics::PictureOverride::resolve_with_session(&z0, &game_dir, Some("zork0.mg1"));
    assert_eq!(session.flavour(), Some(blorb::infocom_pics::Flavour::Pc));

    // The session choice does not touch the file: it applies to this launch only,
    // which is the whole try-before-you-commit idea.
    assert_eq!(read_per_game_pictures(&game_dir), Some("zork0.pic".to_string()));
    let _ = std::fs::remove_dir_all(&game_dir);
}

/// The inherit contract. A dialog that wrote every field it displayed would pin
/// settings the user never touched, and the story would stop tracking later
/// changes to the global config.
#[test]
fn the_checkbox_writes_only_the_keys_the_user_changed() {
    let dir = tmp("inherit");
    let story = dir.join("story.z6");
    std::fs::write(&story, b"x").unwrap();
    let game_dir = dir.join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    // A story that already inherits a global interpreter number of 6.
    let mut st = LaunchOptionsState::new("Story", &story, None, Some(6), Some(6), None);

    // Ticking the box without changing anything writes nothing — an untouched
    // dialog must be indistinguishable from never opening it.
    st.persist = true;
    st.persist_to(&game_dir).unwrap();
    assert!(
        !per_game_config_path(&game_dir).exists(),
        "an untouched dialog must not create a sidecar"
    );
    assert_eq!(st.overrides(), LaunchOverrides::default());

    // Changing one key writes one key. Not two, and not the inherited value of
    // the other — `interpreter_number = 6` present is NOT the same as absent.
    st.interpreter = Some(4);
    st.persist_to(&game_dir).unwrap();
    let body = std::fs::read_to_string(per_game_config_path(&game_dir)).unwrap();
    assert_eq!(read_per_game_interpreter_number(&game_dir), Some(4));
    assert_eq!(read_per_game_pictures(&game_dir), None, "untouched key stays absent: {body:?}");
    assert_eq!(body.lines().filter(|l| !l.trim().is_empty()).count(), 1, "{body:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An existing hand-written `pictures` key survives the dialog writing its
/// sibling, which is the SQ-0734 carry-through rule extended to the new writers.
#[test]
fn writing_one_launch_key_never_deletes_the_other() {
    let dir = tmp("carry");
    let story = dir.join("story.z6");
    std::fs::write(&story, b"x").unwrap();
    let game_dir = dir.join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    std::fs::write(
        per_game_config_path(&game_dir),
        "pictures = \"FMVPOKER.EG1\"\nhonor_game_colours = false\n",
    )
    .unwrap();

    let mut st = LaunchOptionsState::new("Story", &story, Some("FMVPOKER.EG1"), None, Some(6), None);
    st.interpreter = Some(6);
    st.persist_to(&game_dir).unwrap();
    assert_eq!(read_per_game_pictures(&game_dir), Some("FMVPOKER.EG1".to_string()));
    assert_eq!(read_per_game_interpreter_number(&game_dir), Some(6));
    assert_eq!(app::styles::read_per_game_honor(&game_dir), Some(false));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Picking prettier art MOVES the emulated machine unless a number is set
/// outright, and header byte 0x1E is not inert — `crates/zvm/src/cpu/exec.rs`
/// branches on it. The dialog derives the number the same way boot does and
/// reports where it came from, because doing that silently is the surprise the
/// dialog exists to prevent.
#[test]
fn the_art_choice_moves_the_machine_and_the_dialog_can_name_the_source() {
    let z0 = fixture_path("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let mut st = LaunchOptionsState::new("Zork Zero", &z0, None, None, Some(6), None);

    // No art named: Frotz's rule, and it says so.
    assert_eq!(st.derived(), Some((6, InterpreterSource::Default)));

    // The Amiga rendition asks for the Amiga — a different number, from the art.
    if let Some(i) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.pic")) {
        st.art = i + 1;
        assert_eq!(st.derived(), Some((4, InterpreterSource::Artwork)));
        // The same answer `InterpreterProfile::resolve` reaches at boot, so the
        // dialog is reporting the real chain rather than a parallel guess.
        let profile = app::interpreter::InterpreterProfile::resolve(&z0, None, st.chosen_art().map(|c| c.flavour), None);
        assert_eq!(profile.interpreter_number(), Some(4));
    }
    // A DOS rendition asks for the IBM PC, which defers to zvm's own rule.
    if let Some(i) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.mg1")) {
        st.art = i + 1;
        assert_eq!(st.derived(), Some((6, InterpreterSource::Artwork)));
    }
    // An explicit number beats the art, and reports itself as explicit.
    st.interpreter = Some(3);
    assert_eq!(st.derived(), Some((3, InterpreterSource::Explicit)));
}

/// The exact chain `boot_story` runs, for the CLI door: `--pictures` →
/// `resolve_with_session` → `InterpreterProfile::resolve`. Naming the Amiga
/// archive on the command line moves header byte 0x1E to 4, which is not a
/// cosmetic change — `crates/zvm/src/cpu/exec.rs` branches on that byte.
#[test]
fn the_cli_door_reaches_the_machine_the_same_way_boot_does() {
    let z0 = fixture_path("zork0-r393-s890714.z6");
    if !z0.is_file() || !fixture_path("zork0.pic").is_file() {
        return;
    }
    let empty = tmp("cli-chain");

    // Baseline: no flag, no sidecar → the Blorb, and the default machine.
    let none = app::graphics::PictureOverride::resolve_with_session(&z0, &empty, None);
    let base = app::interpreter::InterpreterProfile::resolve(&z0, None, none.flavour(), None);
    assert_eq!(base, app::interpreter::InterpreterProfile::IbmPc);
    assert_eq!(base.interpreter_number(), None, "IBM PC defers to zvm's own rule");

    // With the flag naming the Amiga archive, the machine follows the art.
    let named =
        app::graphics::PictureOverride::resolve_with_session(&z0, &empty, Some("zork0.pic"));
    assert!(matches!(named, app::graphics::PictureOverride::Loaded { .. }), "{named:?}");
    let amiga = app::interpreter::InterpreterProfile::resolve(&z0, None, named.flavour(), None);
    assert_eq!(amiga, app::interpreter::InterpreterProfile::Amiga);
    assert_eq!(amiga.interpreter_number(), Some(4));

    // …unless a number is set outright, which still wins over the art.
    let pinned = app::interpreter::InterpreterProfile::resolve(&z0, Some(6), named.flavour(), None);
    assert_eq!(pinned, app::interpreter::InterpreterProfile::IbmPc);
    let _ = std::fs::remove_dir_all(&empty);
}

/// A per-game or per-launch interpreter number must never reach the GLOBAL
/// config. `write_config_at` persists `interpreter_number` unless the value is
/// marked as belonging to this run — so a story whose sidecar pins the Amiga,
/// played once with the settings screen opened, would otherwise hand every other
/// story machine 4 from then on.
#[test]
fn a_per_launch_interpreter_number_never_leaks_into_the_global_config() {
    let dir = tmp("leak");
    let cfg_path = dir.join("config.toml");
    std::fs::write(&cfg_path, "interpreter_number = 6\n").unwrap();

    // What `boot_story` does for a sidecar / dialog value: in force this run,
    // and flagged as such.
    let mut cfg = app::config::Config {
        config_file: cfg_path.clone(),
        interpreter_number: Some(4),
        ..Default::default()
    };
    cfg.one_run.pin(app::config::keys::INTERPRETER_NUMBER, 4u8);
    assert!(cfg.interpreter_number_from_cli(), "the value is marked one-run");
    app::config::write_config_file(&cfg).unwrap();
    let after = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(after.contains("interpreter_number = 6"), "the global key is untouched: {after:?}");

    // A deliberate settings-screen edit is a different act and DOES persist.
    cfg.set_interpreter_number(Some(4));
    app::config::write_config_file(&cfg).unwrap();
    let after = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(after.contains("interpreter_number = 4"), "an explicit edit persists: {after:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Glulx or Scott story has no header 0x1E at all, so there is no number to
/// report and the dialog must not invent one.
#[test]
fn a_non_z_story_has_no_interpreter_number_to_report() {
    assert_eq!(derived_interpreter(None, None, None, None), None);
    assert_eq!(derived_interpreter(None, None, Some(app::hints::DiskImage::Adf), None), None);
}

/// Reopening the dialog on a story whose sidecar already names an archive lands
/// on that archive, and overrides nothing — the baseline is what it inherits.
#[test]
fn the_dialog_opens_on_what_the_story_already_inherits() {
    let z0 = fixture_path("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let all = discover_art_candidates(&z0, None);
    let Some(i) = all.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.eg1")) else {
        return;
    };
    let st = LaunchOptionsState::new("Zork Zero", &z0, Some("zork0.eg1"), Some(4), Some(6), None);
    assert_eq!(st.art, i + 1, "the sidecar's archive is the selected row");
    assert_eq!(st.interpreter, Some(4));
    assert!(st.overrides().is_empty(), "opening and playing changes nothing");
    // Backing out to "use this story's own art" cannot be an override — there is
    // no "override with nothing" — so the dialog flags that only the checkbox
    // can carry it, rather than appearing to act and then not acting.
    let mut cleared = st.clone();
    cleared.art = 0;
    assert!(cleared.clears_inherited_art());
    assert!(cleared.overrides().is_empty());
}

/// The rendered dialog, as the user sees it. Kept as an assertion rather than a
/// snapshot so it survives cosmetic edits, but it prints the frame so a reviewer
/// can read it (`cargo nextest run -p app launch_options -- --nocapture`).
#[test]
fn the_dialog_renders_its_list_its_derived_number_and_its_checkbox() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let z0 = fixture_path("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let mut st = LaunchOptionsState::new("Zork Zero", &z0, None, None, Some(6), None);
    if let Some(i) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.eg1")) {
        st.art = i + 1;
        st.cursor = app::launch_options::Row::Art(i + 1);
    }
    st.persist = true;

    let cs = app::colors::ColorScheme::terminal_default();
    let mut term = Terminal::new(TestBackend::new(100, 26)).unwrap();
    term.draw(|f| {
        app::render::launch_options_dialog::draw_launch_options(&st, f.area(), &cs, f.buffer_mut());
    })
    .unwrap();
    let buf = term.backend().buffer();
    let mut frame = String::new();
    for y in 0..buf.area().height {
        for x in 0..buf.area().width {
            frame.push_str(buf.cell((x, y)).unwrap().symbol());
        }
        frame.push('\n');
    }
    println!("{frame}");

    assert!(frame.contains("Launch options"), "{frame}");
    assert!(frame.contains("Zork Zero"));
    assert!(frame.contains("Artwork"));
    assert!(frame.contains("zork0.eg1"), "every candidate is listed by name");
    assert!(frame.contains("pictures"), "with its picture count");
    // Zork Zero's renditions are all single-disk, so no row carries a
    // multi-part note — the column that used to say "part 1" on every line is
    // gone, because a constant is not information (SQ-0798).
    assert!(!frame.contains("part 1"), "a single-part archive says nothing about parts: {frame}");
    assert!(!frame.contains("disks"), "and claims no second disk: {frame}");
    assert!(frame.contains("header 0x1E"), "the derived interpreter number");
    assert!(frame.contains("from the artwork"), "and where it came from");
    // SQ-0815, reported as *"we also still have the dither warning in the dialog
    // box"*: the EGA caveat is GONE, because SQ-0797 fused the dither it warned
    // about. This is the assertion that would have caught it — a dialog string is
    // only ever checked by the eye that reads it, and this one was true when it
    // was written and false by the next morning.
    assert!(
        !frame.contains("dithered colours do not fuse"),
        "the EGA dithering caveat outlived its defect and is still in the dialog: {frame}",
    );
    assert!(!frame.contains("⚠"), "no rendition warns about anything today: {frame}");
    assert!(frame.contains("[x] Save as this game's default"), "the checkbox, ticked");
}


// ── The artwork INSIDE the medium (SQ-0843) ──────────────────────────────────
//
// Reported by the user against the browser: *"the story list dialog box doesn't
// allow me to choose the b/w mac format."* The enumeration was a `read_dir` of
// the story's own directory, written before lanthorn could open a disk at all,
// so a story mounted out of `stories/Zork Zero Disk.image` could be offered
// neither of the two archives that disk carries. `--pictures Pic.data` and the
// per-game `pictures` key were the only doors to the Macintosh's two-colour art.
//
// The list now unions both sources through `app::assets::files` — loose files
// beside the story, and the files on the volume when the story came off one —
// and everything below is that union seen from the dialog.

/// The Macintosh disk offers **both** of its archives, told apart by the only
/// thing that distinguishes them.
///
/// `CPic.data` and `Pic.data` are one codec (`Flavour::AmigaMac`), one picture
/// count (483) and two names that say nothing, so the row a person reads has to
/// carry the discriminator: `is_monochrome`, off the archive's own `EF_MONO`
/// flags, and the 480×300 picture space Infocom's own Macintosh interpreter
/// calls `GFXMAC_X`/`GFXMAC_Y`.
///
/// FALSIFIED by dropping the medium arm from `assets::files`: the Mac disk's
/// candidate list collapses to the one loose `zorkzero.mg1` that happens to
/// carry its name, and there is no way to choose the b/w Mac format — the
/// reported symptom, exactly.
#[test]
fn both_macintosh_archives_are_offered_from_inside_the_disk_image() {
    let image = fixture_path("Zork Zero Disk.image");
    if !image.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium missing");
        return;
    }
    let found = discover_art_candidates(&image, None);
    let names: Vec<&str> = found.iter().map(|c| c.filename.as_str()).collect();

    let colour = found
        .iter()
        .find(|c| c.filename == "CPic.data")
        .unwrap_or_else(|| panic!("the disk's colour archive must be offered; got {names:?}"));
    let mono = found
        .iter()
        .find(|c| c.filename == "Pic.data")
        .unwrap_or_else(|| panic!("the disk's two-colour archive must be offered; got {names:?}"));

    assert!(colour.on_medium && mono.on_medium, "both live on the volume, not beside the story");
    assert_eq!(colour.path, image, "…and the only path either has is the image itself");
    assert_eq!(mono.path, image);

    // The labels a human reads, which is the whole point of listing two rows.
    assert_eq!(colour.rendition, "Amiga", "a colour AmigaMac archive stays honestly ambiguous");
    assert_eq!(mono.rendition, "Mac B&W", "the two-colour one is the standard Macintosh's");
    assert_ne!(colour.rendition, mono.rendition, "two rows a person cannot tell apart is the bug");

    // …and the facts under the labels, read off each file rather than its name.
    assert_eq!(colour.space_width, 320, "CPic.data is the big Mac's doubled 320×200");
    assert_eq!(mono.space_width, 480, "Pic.data is GFXMAC_X — '1.5 x Amiga sizes'");
    assert_eq!(colour.pictures, 483, "one directory, both archives");
    assert_eq!(mono.pictures, 483);
    assert_eq!((colour.parts, mono.parts), (1, 1), "neither is a multi-part set");

    // Nothing else on that volume is artwork: the story, the application and the
    // desktop database are all present in `contents()` and all decline to parse.
    let on_disk: Vec<&str> = found.iter().filter(|c| c.on_medium).map(|c| c.filename.as_str()).collect();
    assert_eq!(on_disk, ["CPic.data", "Pic.data"], "identified by parsing, not by name");
    assert!(
        !names.iter().any(|n| n.eq_ignore_ascii_case("Story.data")),
        "a story is not artwork however it parses: {names:?}"
    );
}

/// The Amiga floppy gains the same door from the same code — a door, not a
/// policy. Its `Pic.data` was always loaded automatically (tier 2); now it can
/// also be chosen, which is what makes "inherit" and "pick the disk's own art"
/// two ways of saying one thing rather than one working and one missing.
#[test]
fn an_amiga_floppy_offers_its_own_archive_through_the_same_seam() {
    let adf = fixture_path("Zork Zero - The Revenge of Megaboz.adf");
    if !adf.is_file() {
        eprintln!("SKIP: gitignored Amiga medium missing");
        return;
    }
    let found = discover_art_candidates(&adf, None);
    let names: Vec<&str> = found.iter().map(|c| c.filename.as_str()).collect();
    let pic = found
        .iter()
        .find(|c| c.filename.eq_ignore_ascii_case("Pic.data"))
        .unwrap_or_else(|| panic!("the floppy's own archive must be offered; got {names:?}"));
    assert!(pic.on_medium);
    assert_eq!(pic.rendition, "Amiga");
    assert_eq!(pic.space_width, 320);
    assert!(pic.pictures > 100, "a real archive holds many pictures, got {}", pic.pictures);
}

/// **Picking it must LOAD it.** The dialog writes a bare filename into
/// `pictures = "…"`, and that name exists nowhere on the host filesystem — so
/// the two halves only meet if `PictureOverride` looks inside the medium
/// (SQ-0838). This drives the whole chain the dialog does: choose the row, take
/// the override it produces, resolve it, and ask the resulting picture source
/// what it actually is.
#[test]
fn picking_the_monochrome_row_loads_the_monochrome_art() {
    let image = fixture_path("Zork Zero Disk.image");
    if !image.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium missing");
        return;
    }
    let empty = tmp("mac-pick");
    let base = LaunchOptionsState::new(
        "Zork Zero",
        &image,
        None,
        None,
        Some(6),
        Some(blorb::medium::DiskImage::Hfs),
    );
    // The name really is absent from the story's directory: this is the case a
    // host-filesystem-only override would report as `Missing`.
    assert!(!fixture_path("Pic.data").exists(), "the archive is on the volume, not beside it");

    for (filename, want_mono, want_space) in
        [("Pic.data", true, (480u16, 300u16)), ("CPic.data", false, (320, 200))]
    {
        let mut st = base.clone();
        // The colour archive is what row 0 already resolves to, so it is not
        // listed a second time (SQ-0876) and "pick it" means accepting the
        // automatic row — which writes no `pictures` key and draws the very
        // same file. The monochrome one is a row of its own.
        let is_default =
            base.default_art.as_ref().is_some_and(|d| d.filename.eq_ignore_ascii_case(filename));
        st.art = match st.candidates.iter().position(|c| c.filename == filename) {
            Some(i) => i + 1,
            None => {
                assert!(is_default, "{filename} is neither a row nor the automatic choice");
                0
            }
        };

        // What the dialog hands the launch: one override, one bare name — and
        // nothing at all when the automatic row is the one accepted.
        let ov = st.overrides();
        let want = (!is_default).then_some(filename);
        assert_eq!(ov.pictures.as_deref(), want);
        assert_eq!(ov.interpreter_number, None, "an untouched interpreter row overrides nothing");

        // …and what that name resolves to, off the medium.
        let over = app::graphics::PictureOverride::resolve_with_session(
            &image,
            &empty,
            ov.pictures.as_deref(),
        );
        if !is_default {
            assert!(
                matches!(over, app::graphics::PictureOverride::Loaded { .. }),
                "{filename} was offered and did not load: {over:?}"
            );
            assert_eq!(over.warning(), None, "{filename}: a row offered must load quietly");
            assert_eq!(over.std_window(), Some(want_space), "{filename}");
        }
        let src = app::graphics::PictSource::resolve_with_override(&image, over, None);
        assert_eq!(src.is_monochrome(), want_mono, "{filename}: the art actually drawn");
        assert_eq!(src.native_std_window(), Some(want_space), "{filename}");
    }
    let _ = std::fs::remove_dir_all(&empty);
}

/// The provenance line must not contradict the row above it.
///
/// `Flavour::AmigaMac` is one container written by two machines, so the archive
/// names a codec and not a machine — and until SQ-0843 the named-archive step of
/// `InterpreterProfile::resolve` returned before the medium was ever consulted.
/// Picking `CPic.data`, the very archive the story loads on its own, therefore
/// demoted a Macintosh to an Amiga, and the dialog printed "header 0x1E = 4
/// (Amiga) — from the artwork" two lines under a row labelled `Mac B&W`.
///
/// Both surfaces are checked, because a dialog that reports one number while
/// boot applies another is worse than either alone.
#[test]
fn choosing_a_macintosh_archive_leaves_the_machine_a_macintosh() {
    let image = fixture_path("Zork Zero Disk.image");
    if !image.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium missing");
        return;
    }
    let empty = tmp("mac-machine");
    let mut st = LaunchOptionsState::new(
        "Zork Zero",
        &image,
        None,
        None,
        Some(6),
        Some(blorb::medium::DiskImage::Hfs),
    );
    // Inherit: the medium's own answer, ZMSD §11.1.3's 3.
    assert_eq!(st.derived(), Some((3, InterpreterSource::DiskImage)));

    for filename in ["CPic.data", "Pic.data"] {
        // `CPic.data` is the automatic choice and so is not listed twice
        // (SQ-0876); accepting row 0 is how it is picked.
        st.art = match st.candidates.iter().position(|c| c.filename == filename) {
            Some(i) => i + 1,
            None => 0,
        };
        assert_eq!(
            st.derived(),
            Some((3, InterpreterSource::DiskImage)),
            "{filename}: the disk settled it, and the line says the disk"
        );
        // The same answer boot reaches, through the same function.
        let over = app::graphics::PictureOverride::resolve_with_session(&image, &empty, Some(filename));
        let profile = app::interpreter::InterpreterProfile::resolve(&image, None, over.flavour(), None);
        assert_eq!(profile, app::interpreter::InterpreterProfile::Macintosh, "{filename}");
        assert_eq!(profile.interpreter_number(), Some(3), "{filename}");
    }

    // An UNAMBIGUOUS flavour still beats the medium outright, which is what
    // keeps naming an archive an instruction: the loose MCGA archive on the host
    // asks for the IBM PC and gets it, on a Macintosh disk like anywhere else.
    if let Some(i) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zorkzero.mg1")) {
        st.art = i + 1;
        assert_eq!(st.derived(), Some((6, InterpreterSource::Artwork)));
    }
    // …and an explicit number beats everything, as always.
    st.interpreter = Some(4);
    assert_eq!(st.derived(), Some((4, InterpreterSource::Explicit)));
    let _ = std::fs::remove_dir_all(&empty);
}

/// **The loose-file path is unmoved.** The medium arm is additive: a story that
/// is not release media mounts nothing, and one that is keeps every loose
/// candidate it had. Pinned as an exact list, because "still contains" would not
/// notice a row that appeared.
#[test]
fn adding_the_medium_arm_moved_nothing_beside_the_story() {
    for (story, want) in [
        ("zork0-r393-s890714.z6", &["zork0.cg1", "zork0.eg1", "zork0.mg1", "zork0.pic"][..]),
        // Arthur's `.eg2` is absorbed into the `.eg1` row and never listed —
        // the SQ-0798 rule, unchanged by any of this.
        ("arthur-r74-s890714.z6", &["arthur.cg1", "arthur.eg1", "arthur.mg1", "arthur.pic"]),
    ] {
        let path = fixture_path(story);
        if !path.is_file() {
            continue; // gitignored fixtures
        }
        let found = discover_art_candidates(&path, None);
        let names: Vec<String> = found.iter().map(|c| c.filename.to_lowercase()).collect();
        assert_eq!(names, want, "{story}");
        assert!(found.iter().all(|c| !c.on_medium), "{story} is not release media");
        assert!(
            found.iter().all(|c| c.path.is_file() && c.path != path),
            "{story}: a loose candidate is its own file on disk"
        );
    }
}

/// **The dialog the user is judging this by.** The frame is printed, so the row
/// a person reads can be read here too (`cargo nextest run -p app
/// both_macintosh -- --nocapture`).
#[test]
fn the_dialog_shows_the_disks_own_archives_and_says_where_they_live() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let image = fixture_path("Zork Zero Disk.image");
    if !image.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium missing");
        return;
    }
    let mut st = LaunchOptionsState::new(
        "Zork Zero",
        &image,
        None,
        None,
        Some(6),
        Some(blorb::medium::DiskImage::Hfs),
    );
    let i = st.candidates.iter().position(|c| c.filename == "Pic.data").expect("a candidate");
    st.art = i + 1;
    st.cursor = app::launch_options::Row::Art(i + 1);

    let cs = app::colors::ColorScheme::terminal_default();
    let mut term = Terminal::new(TestBackend::new(100, 26)).unwrap();
    term.draw(|f| {
        app::render::launch_options_dialog::draw_launch_options(&st, f.area(), &cs, f.buffer_mut());
    })
    .unwrap();
    let buf = term.backend().buffer();
    let mut frame = String::new();
    for y in 0..buf.area().height {
        for x in 0..buf.area().width {
            frame.push_str(buf.cell((x, y)).unwrap().symbol());
        }
        frame.push('\n');
    }
    println!("{frame}");

    // Both rows, both labels, and the marker that explains a filename the
    // story's own folder plainly does not contain.
    assert!(frame.contains("CPic.data"), "the colour archive is offered: {frame}");
    assert!(frame.contains("Pic.data"), "…and the two-colour one: {frame}");
    assert!(frame.contains("Mac B&W"), "labelled so a person can tell them apart: {frame}");
    // SQ-0865: WHICH disk. This release is a single image, so there is no disk
    // number to give and the phrase is the user's own wording — never the bare
    // "on disk" it replaced, which said neither which disk nor even that there
    // was only one.
    assert!(frame.contains("from game disk"), "and told where they live: {frame}");
    assert!(!frame.contains("on disk"), "the vague marker is gone: {frame}");
    // The provenance line must agree with the row selected above it: this disk
    // is a Macintosh whichever of its two archives you pick (SQ-0843).
    assert!(
        frame.contains("header 0x1E = 3 (Macintosh)"),
        "picking the Mac's own art must not demote the machine: {frame}"
    );
    assert!(frame.contains("from the disk image"), "and the line names what settled it: {frame}");
}

// ── SQ-0865: the default row says what it will open, and where it lives ──────
//
// Reported against the panel: *"the defaul artwork selection is 'Use this
// story's own art (Blorb / disk image)'. We should be more specific on the
// default format and match the text/formatting of the other options. In terms of
// the other options, 'on disk' is confusing."*
//
// The old row was prose among columns and named nothing, which made it the one
// row you could not judge before accepting it. It is columnar now and it names
// the archive the boot will really open — resolved through the boot's own path
// (`graphics::release_art`, then the resource Blorb), never re-derived here.

/// A **single-image release** has no disk number to give, and says so in the
/// user's own words. Both media in the corpus that are one platter: the
/// Macintosh HFS disk and the Amiga floppy.
///
/// The default row is the interesting half. The Macintosh ships two archives and
/// the automatic choice is the COLOUR one (`blorb::medium` tiebreak, SQ-0838), so
/// the row must say `CPic.data` — naming `Pic.data` there would be a row
/// promising two-colour art to someone who is about to get colour.
#[test]
fn a_single_image_release_says_from_game_disk_and_names_its_default() {
    // (image, the archive that boots when nothing is overridden)
    for (name, default_archive, rendition) in [
        ("Zork Zero Disk.image", "CPic.data", "Amiga"),
        ("Zork Zero - The Revenge of Megaboz.adf", "Pic.data", "Amiga"),
    ] {
        let image = fixture_path(name);
        if !image.is_file() {
            eprintln!("SKIP: gitignored medium missing: {name}");
            continue;
        }
        let st = LaunchOptionsState::new("Zork Zero", &image, None, None, Some(6), None);

        // No candidate off this medium carries a disk number, because the
        // release is one disk — and each says the phrase that means exactly
        // that, rather than the "on disk" that meant nothing in particular.
        // The list holds the volume's archives OTHER than the one row 0 already
        // names, so the Amiga floppy — which carries exactly one — lists none
        // and the Macintosh disk lists its monochrome `Pic.data` beside the
        // colour default. Both are correct; what must never happen is an
        // archive being offered twice.
        let on_medium: Vec<_> = st.candidates.iter().filter(|c| c.on_medium).collect();
        assert!(
            on_medium.iter().all(|c| !c.filename.eq_ignore_ascii_case(default_archive)),
            "{name}: the automatic row already names {default_archive}"
        );
        for c in &on_medium {
            assert_eq!(c.disk_number, None, "{name}/{}: one platter, no number", c.filename);
            assert_eq!(
                app::launch_options::medium_note(c),
                "from game disk",
                "{name}/{}",
                c.filename,
            );
        }

        let d = st.default_art.as_ref().unwrap_or_else(|| panic!("{name}: a default to name"));
        assert_eq!(d.filename, default_archive, "{name}: the archive that actually boots");
        assert_eq!(d.rendition, rendition, "{name}");
        assert_eq!(d.medium_note(), "from game disk", "{name}");
        assert!(d.on_medium && d.disk_number.is_none(), "{name}");

        // …and it agrees with the boot. Same oracle as the DOS press's:
        // naming what the row shows must reach the artwork accepting the
        // default reaches.
        let empty = tmp("single-agree");
        let mut auto = app::graphics::PictSource::resolve(&image, None);
        let over =
            app::graphics::PictureOverride::resolve_with_session(&image, &empty, Some(&d.filename));
        let mut named = app::graphics::PictSource::resolve_with_override(&image, over, None);
        assert_eq!(
            named.is_monochrome(),
            auto.is_monochrome(),
            "{name}: the row names {default_archive}, which is not what boots",
        );
        assert_eq!(named.native_std_window(), auto.native_std_window(), "{name}");
        assert_eq!(named.dims(1), auto.dims(1), "{name}");
        let _ = std::fs::remove_dir_all(&empty);
    }
}

/// An **ordinary file beside the story** keeps saying nothing about disks —
/// there is no disk — and the default row names the resource Blorb, which is
/// what tier 1 will actually open.
///
/// `Zork0.blb` is never a *pickable* row (it is not a native archive and the
/// candidate list rightly refuses it), which is exactly why the default row has
/// to name it: it is otherwise the one thing the dialog can boot and cannot show.
#[test]
fn a_loose_story_names_its_blorb_and_mentions_no_disk() {
    let z0 = fixture_path("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return; // gitignored fixture
    }
    let st = LaunchOptionsState::new("Zork Zero", &z0, None, None, Some(6), None);
    for c in &st.candidates {
        assert!(!c.on_medium, "{}: nothing here is on a medium", c.filename);
        assert_eq!(c.disk_number, None, "{}", c.filename);
        assert_eq!(app::launch_options::medium_note(c), "", "{}", c.filename);
    }

    let d = st.default_art.as_ref().expect("the Blorb beside it is what boots");
    assert!(d.filename.eq_ignore_ascii_case("Zork0.blb"), "got {:?}", d.filename);
    assert_eq!(d.rendition, "Blorb", "a Blorb is a container, not a video card");
    assert_eq!(d.medium_note(), "", "a file in a folder explains nothing");
    // The count is the Blorb's own `Pict` population — the same enumeration
    // `PictSource::all_pict_dims` draws from, not the native archives' 503
    // directory entries. `Zork0.blb`'s RIdx really does list 2213 of them, every
    // one a `Pict`, which is why the two numbers differ by so much: a native
    // archive holds one entry per picture id, a Blorb one per image.
    assert_eq!(d.pictures, 2213, "the Blorb's Pict resources, whole");

    // The row is not a claim about a native archive: `PictSource::resolve` on
    // this story takes tier 1, and there is no release art to take instead.
    assert!(app::graphics::release_art(&z0, None).is_none(), "no disk image, no release art");
}

/// **The narrow panel.** The dialog is not full-screen and its width follows the
/// terminal's, so the widened name column has to be checked at the bottom of its
/// range as well as the top.
///
/// What must survive clipping is the part that identifies the row — the mark,
/// the archive's name and its rendition. The picture count and the disk note are
/// the first things off the end, and that is the right order: a person at 48
/// columns can still tell the rows apart, which is what the list is for. Pinned
/// so that a future column change cannot quietly cost the rendition instead.
#[test]
fn a_narrow_dialog_keeps_the_name_and_the_rendition_of_every_row() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let image = fixture_path("Zork Zero Disk.image");
    if !image.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium missing");
        return;
    }
    let st = LaunchOptionsState::new("Zork Zero", &image, None, None, Some(6), None);
    let cs = app::colors::ColorScheme::terminal_default();
    for width in [48u16, 60, 100] {
        let mut term = Terminal::new(TestBackend::new(width, 26)).unwrap();
        let mut drew = false;
        term.draw(|f| {
            drew = app::render::launch_options_dialog::draw_launch_options(
                &st,
                f.area(),
                &cs,
                f.buffer_mut(),
            )
            .is_some();
        })
        .unwrap();
        assert!(drew, "{width}: the dialog fits and must draw");
        let buf = term.backend().buffer();
        let mut frame = String::new();
        for y in 0..buf.area().height {
            for x in 0..buf.area().width {
                frame.push_str(buf.cell((x, y)).unwrap().symbol());
            }
            frame.push('\n');
        }
        println!("── {width} columns\n{frame}");
        for want in ["CPic.data", "Pic.data", "Mac B&W", "Automatic — "] {
            assert!(frame.contains(want), "{width} columns lost {want:?}:\n{frame}");
        }
    }
}

/// SQ-1532: `startup.rs::boot_story`'s precedence for the per-game
/// `colour_source` sidecar key — this launch's own dialog choice, else the
/// sidecar, else the global default; `--colour` on this launch outranks both.
/// The exact expression is `overrides.colour_source.or_else(|| read_per_game_colour_source(&game_dir)).filter(|_| cli.colour.is_none())`,
/// reproduced here rather than called: `boot_story` lives in the `lanthorn`
/// BINARY crate (`crates/app/src/startup.rs`), not the `app` library this
/// integration suite links against, so it is not reachable from a test file —
/// the same reason `a_per_launch_interpreter_number_never_leaks_into_the_global_config`
/// above models `write_config_at`'s guard by hand instead of calling
/// `boot_story` for the interpreter-number case, and the shape
/// `honor_game_colours`'s own per-game read already has at
/// `startup.rs:~1194-1224` (`.filter(|_| cli.game_colours.is_none())`), which
/// this mirrors for the second CLI flag SQ-1532 adds a sidecar key for.
#[test]
fn colour_source_sidecar_is_honored_unless_the_cli_names_one() {
    let dir = tmp("colour-source-boot-precedence");
    let game_dir = dir.join("game.save");
    app::styles::write_per_game_colour_source(
        &game_dir,
        Some(Some(app::config::ColourSource::Terminal)),
        false,
    )
    .unwrap();

    // No dialog override, no `--colour`: the sidecar decides.
    let no_cli: Option<app::config::ColourSource> = None;
    let resolved = None::<app::config::ColourSource>
        .or_else(|| app::styles::read_per_game_colour_source(&game_dir))
        .filter(|_| no_cli.is_none());
    assert_eq!(
        resolved,
        Some(app::config::ColourSource::Terminal),
        "no CLI flag on this launch: the per-game sidecar's choice is honored"
    );

    // `--colour machine` on this launch: it outranks the sidecar entirely, even
    // though nothing here touched the sidecar's own stored value.
    let cli_colour = Some(app::config::ColourSource::Machine);
    let resolved = None::<app::config::ColourSource>
        .or_else(|| app::styles::read_per_game_colour_source(&game_dir))
        .filter(|_| cli_colour.is_none());
    assert_eq!(resolved, None, "--colour on this launch must outrank the sidecar entirely");
    assert_eq!(
        app::styles::read_per_game_colour_source(&game_dir),
        Some(app::config::ColourSource::Terminal),
        "and must not have touched the sidecar's own stored value either"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
