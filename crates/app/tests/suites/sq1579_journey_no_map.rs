//! SQ-1579: the Amiga release of Journey must never gain a false room on the
//! automap, and — more broadly — a menu-driven v6 story with no grammar must
//! never be mapped at all.
//!
//! Two independent defects combined to produce the reported bug. `Journey -
//! The Quest Begins.adf` (Amiga release 30) paints a CENTERED "JOURNEY" title
//! banner as its only v6 status-band candidate — the module doc in
//! `location.rs` assumed Journey paints no band at all, which is true of the
//! bare `journey.z6` (release 83) but not of this disk image. Before the fix,
//! two things let that banner through:
//!
//! 1. `global_room_by_shown_text` (rung 3 of `detect_location_v6`) was handed
//!    every status-band candidate, not just the LEFT-ANCHORED ones rung 2
//!    restricts itself to — so a centered banner could reach it exactly like
//!    a real room-name field.
//! 2. `object_text_property` accepted "journey" as a mere PREFIX of Praxix's
//!    own property text ("journey, the following was written in…"), which
//!    happens to be a perfectly ordinary word-boundary prefix match.
//!
//! Together, the "JOURNEY" banner corroborated an unrelated global and minted
//! a false room, "journey, the following was written in", that never cleared
//! once the global did (the party member Praxix is only selected some of the
//! time). Separately — and this is the deeper fix — Journey has no grammar
//! table at all (`Grammar::load` answers `Absent`, since it is menu-driven),
//! so it has no verb-driven navigation to build a map out of in the first
//! place, and should not be mapped regardless of what any status-band
//! detector says.
//!
//! Repro (from the quest body): boot the Amiga floppy, tap Enter through the
//! intro vignettes and the Praxix -> Cast -> Elevation menu sequence (11 key
//! reads), then two more Enters ("Praxix thought to cast the 'Elevation'
//! spell…"). The map must stay completely empty throughout.
//!
//! `stories/` is gitignored, so this skips vacuously when the floppy is
//! absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::engine_helpers::zmachine_story_has_no_grammar;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::machine_boot::MachineBoot;
use app::session::{apply_turn, DeathWatch, GameSession, InputKind};
use mapper::mapper::Mapper;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot the Amiga Journey floppy exactly as `startup.rs` boots any disk
/// image — profile from the mount, artwork from the same medium — per
/// CLAUDE.md's "Boot a harness the way `startup.rs` boots" rule.
fn boot_amiga_journey() -> Option<GameSession> {
    let path = stories_dir().join("Journey - The Quest Begins.adf");
    let (loaded, _mounted) = match app::hints::load_mounted_story(&path) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    let bytes = loaded.bytes().to_vec();
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let boot = MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &boot)
        .unwrap_or_else(|e| panic!("Amiga Journey should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

#[test]
fn amiga_journey_has_no_grammar_and_stays_unmapped() {
    let Some(mut session) = boot_amiga_journey() else { return };

    // Journey is menu-driven: it must carry no grammar table at all. This is
    // the fact the boot-time gate (`host::boot`/`host::reset`) reads to call
    // `Mapper::disable_mapping` before the first turn is ever applied.
    assert!(
        zmachine_story_has_no_grammar(&session),
        "the Amiga Journey floppy must have no grammar table (Grammar::load == Absent)"
    );

    let mut mapper = Mapper::default();
    mapper.disable_mapping(); // exactly what `host::boot`/`host::reset` do for this story
    let mut death = DeathWatch::default();

    // Tap Enter through the intro vignettes and the Praxix -> Cast ->
    // Elevation menu sequence (11 key reads), then a couple more turns past
    // it, watching `current_location` directly the whole way — bypassing the
    // mapper gate — so this also falsifies the `location.rs` detector fix on
    // its own, not just the mapper-disable path.
    for i in 0..20 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        assert!(!session.quit, "quit at turn {i}");
        assert!(session.machine.fault_trace.is_none(), "faulted at turn {i}");
        apply_turn(&mut mapper, "", &r, &mut death);

        if let Some(loc) = session.current_location() {
            assert!(
                !loc.name.starts_with("journey, the following was written in"),
                "turn {i}: the centered \"JOURNEY\" title banner corroborated Praxix's own \
                 property text again — detect_location_v6's rung 3 must reject a \
                 non-left-anchored candidate and a mere text prefix (SQ-1579); got {:?}",
                loc.name
            );
        }
    }

    assert!(
        mapper.graph.rooms().next().is_none(),
        "a menu-driven story with no grammar must never gain a mapped room: {:?}",
        mapper.graph.rooms().map(|r| r.name.clone()).collect::<Vec<_>>()
    );
}

/// The bare PC build (release 83) already had no map before this fix — it
/// paints no status band at all (its story window owns the top of the
/// screen), so `detect_location` was already `None` on every turn. This
/// confirms that stays true, for a DIFFERENT reason than the Amiga press
/// above: no grammar, gated the same way as every other v6 title with none.
#[test]
fn pc_r83_journey_has_no_grammar_and_stays_unmapped() {
    let path = stories_dir().join("journey-r83-s890706.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        bytes, true, false, None, false, picture_dims, picts.std_window(), None, None,
    )
    .expect("Journey r83 should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    assert!(
        zmachine_story_has_no_grammar(&session),
        "journey-r83-s890706.z6 must have no grammar table (Grammar::load == Absent)"
    );

    let mut mapper = Mapper::default();
    mapper.disable_mapping();
    let mut death = DeathWatch::default();

    for i in 0..20 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        assert!(!session.quit, "quit at turn {i}");
        assert!(session.machine.fault_trace.is_none(), "faulted at turn {i}");
        apply_turn(&mut mapper, "", &r, &mut death);
    }

    assert!(
        mapper.graph.rooms().next().is_none(),
        "the PC r83 press must stay unmapped: {:?}",
        mapper.graph.rooms().map(|r| r.name.clone()).collect::<Vec<_>>()
    );
}
