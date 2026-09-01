//! The bottom command band (SQ-0664): the properties that only show up against
//! a real engine and a real render pass.
//!
//! The unit tests in `render/command_band.rs`, `state.rs`, `input.rs`,
//! `layout.rs` and `config.rs` cover the grammar, the focus ladder and the
//! geometry. What is left — and what the old verb menu got wrong — needs a
//! story running:
//!
//! * the object columns are LIVE (take something and it moves *here* → *carried*)
//! * the band is not a modal, so the story prompt line stays drawn
//! * …and graphical v6 keeps its pixel path while the band is open
//!
//! Real commercial stories are gitignored, so every test that needs one skips
//! vacuously when it is absent (the `any_v6_story_present` pattern).


use app::config::Config;
use app::engine::Engine;
use app::graphics::PictSource;
use app::render::command_band::{
    default_quick, default_verbs, refresh_objects, refresh_verbs, verbs_from_grammar, VerbSource,
    VerbTable, COL_CARRIED, COL_HERE, COL_SECOND, COL_VERB,
};
use app::session::GameSession;
use app::state::{AppState, CommandBandState};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;


fn open_band(state: &mut AppState) {
    let band = CommandBandState::new(default_verbs(), default_quick());
    state.overlays.command_band = Some(band);
    state.band_dock.toggle_to(true, true);
}

fn dump(buf: &Buffer) -> String {
    buf.content().iter().map(|c| c.symbol().to_owned()).collect()
}

// ── Live objects ─────────────────────────────────────────────────────────────

/// Boot a plain (non-v6) Z-machine story from `stories/`, or `None` when the
/// gitignored fixture is absent.
fn boot_zmachine(file: &str) -> Option<GameSession> {
    let path = fixture_path(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
            .expect("story should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// Boot a graphical Version 6 story off its bare `.z6`, or `None` when the
/// gitignored fixture is absent.
///
/// The release and serial are asserted, not assumed: a medium is a different
/// BUILD of the game, and a case that names a frame has to be sure it booted the
/// build it is describing. Nothing here measures geometry or colour — the band's
/// columns are the object tree and the transcript — so this deliberately stops
/// short of the full `startup.rs` chain and never touches the palette.
fn boot_v6(file: &str, release: u16, serial: &str) -> Option<GameSession> {
    let path = fixture_path(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    assert_eq!(bytes[0], 6, "{file}: Z-machine version");
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release, "{file}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{file}: serial");
    let mut picts = PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
            .expect("a v6 story should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

/// The columns come from the engine's object tree, not a transcript scrape —
/// and they are refreshed per turn, so taking an object moves it from the
/// *here* column to the *carried* one. This is the defect the whole redesign
/// exists to fix: `build_verb_menu_nouns` tokenized the last 20 transcript
/// lines once, at open, and never looked again.
#[test]
fn taking_an_object_moves_it_from_here_to_carried() {
    // Mini-Zork opens in the West of House with a mailbox and a mat; the leaflet
    // inside the mailbox is the classic takeable.
    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);

    // Turn 0: whatever is in the opening room is HERE, and nothing is carried.
    session.submit("look");
    seed_player_obj(&mut state, &session);
    refresh_objects(&mut state, &session);
    let here0 = state.overlays.command_band.as_ref().unwrap().here.clone();
    let carried0 = state.overlays.command_band.as_ref().unwrap().carried.clone();
    assert!(
        !state.overlays.command_band.as_ref().unwrap().here.is_empty(),
        "a Z-machine story has a real object tree, so the scope block is not empty"
    );
    assert!(!here0.is_empty(), "the opening room has objects in it: {here0:?}");
    assert!(carried0.is_empty(), "nothing carried at the start: {carried0:?}");

    // Take something that is here, then refresh: it must have MOVED, not been
    // added — the two columns are the object tree, not two independent lists.
    let target = here0
        .iter()
        .find(|o| {
            let l = o.to_lowercase();
            l.contains("mailbox") || l.contains("mat") || l.contains("leaflet")
        })
        .cloned()
        .unwrap_or_else(|| here0[0].clone());
    session.submit("open mailbox");
    session.submit("take leaflet");
    // Mirror the run loop: every turn finisher bumps `turn_epoch`, which is
    // what tells the epoch-gated refresh the VM has run (SQ-1175).
    state.begin_turn();
    refresh_objects(&mut state, &session);

    let band = state.overlays.command_band.as_ref().unwrap();
    assert!(
        !band.carried.is_empty(),
        "the taken object shows up in *carried* on the next refresh (was here: {here0:?}, target {target:?})"
    );
    let taken = band.carried[0].clone();
    assert!(
        !band.here.iter().any(|h| h == &taken),
        "…and is gone from *here*: here={:?} carried={:?}",
        band.here,
        band.carried
    );

    // And the columns really are the band's pick sources.
    let carried_items = band.items(COL_CARRIED);
    assert!(carried_items.contains(&taken), "the carried column offers it: {carried_items:?}");
}

/// Mirror the turn loop's player-object lock, which the real run loop does in
/// `turn.rs` before the band ever refreshes.
fn seed_player_obj(state: &mut AppState, session: &GameSession) {
    if state.player_obj.is_none() {
        state.player_obj = session.introspect().and_then(|i| i.player_object());
    }
}

/// The player object is a child of whatever room they're in, so without an
/// explicit id-based exclusion it would show up in every room's *here*
/// column (Zork 1's player object prints as "cretin"). SQ-0667:
/// `refresh_objects` excludes it by id via `Introspect::room_objects_excluding`
/// — falsify by reverting the `refresh_objects` call site back to the plain
/// `room_objects` it used before.
#[test]
fn the_player_object_is_excluded_from_here() {
    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    session.submit("look");
    seed_player_obj(&mut state, &session);
    assert!(state.player_obj.is_some(), "a Z-machine story always finds the player object");

    refresh_objects(&mut state, &session);
    let here = state.overlays.command_band.as_ref().unwrap().here.clone();

    let loc = session.current_location().unwrap().number;
    let vocab = <GameSession as Engine>::story_vocabulary(&session);
    let unfiltered: Vec<String> = session
        .introspect()
        .unwrap()
        .room_objects(loc)
        .iter()
        .filter_map(|o| app::vocab::typeable_name(o, vocab.as_ref()))
        .collect();
    assert_eq!(
        unfiltered.len(),
        here.len() + 1,
        "exactly the player object should be missing from the filtered list: \
         unfiltered={unfiltered:?} filtered(here)={here:?}"
    );
    let extra: Vec<&String> = unfiltered.iter().filter(|o| !here.contains(o)).collect();
    assert_eq!(extra.len(), 1, "one object — the player's own — is missing from `here`");
}

/// SQ-0676 end to end, against a REAL object tree: with the band open, typing
/// goes to the story prompt, the band highlights the nearest live object, and
/// Tab completes the word to that object's full (often multi-word) name. The
/// unit tests use a hand-made object list; this pins the same behaviour on
/// names the engine actually produces.
///
/// Falsifies against the pre-SQ-0676 band, where those keystrokes never reached
/// `state.input` at all (they filtered a column) and Tab swapped focus.
#[test]
fn typing_at_the_prompt_completes_from_the_live_object_columns() {
    use app::input::{apply_action, key_to_action, Action};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    let mut mapper = mapper::mapper::Mapper::default();
    open_band(&mut state);
    session.submit("look");
    seed_player_obj(&mut state, &session);
    refresh_objects(&mut state, &session);

    // Pick a real object from the room and type the first three characters of
    // its first word — whatever the story happens to call it.
    let here = state.overlays.command_band.as_ref().unwrap().here.clone();
    let target = here
        .iter()
        .find(|o| o.split_whitespace().next().is_some_and(|w| w.chars().count() > 3))
        .cloned()
        .unwrap_or_else(|| here[0].clone());
    let first_word = target.split_whitespace().next().unwrap().to_string();
    let typed: String = first_word.chars().take(3).collect();

    for c in format!("take {typed}").chars() {
        let a = key_to_action(&state, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        assert_eq!(a, Action::InputChar(c), "`{c}` must reach the story prompt");
        apply_action(a, &mut state, &mut mapper);
    }
    assert_eq!(state.input.value, format!("take {typed}"));

    let band = state.overlays.command_band.as_ref().unwrap();
    let (col, idx) = band.nearest_match(&state.input.value).expect("a live object matches");
    assert!(col == COL_HERE || col == COL_CARRIED, "after a verb, the object columns match");
    assert_eq!(band.items(col)[idx], target, "the nearest match is the object typed toward");

    let a = key_to_action(&state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(a, Action::BandTabPick(col, idx), "the open band owns Tab completion");
    apply_action(a, &mut state, &mut mapper);
    assert_eq!(
        state.input.value,
        format!("take {target}"),
        "Tab completed to the live object's full name"
    );
}

/// A closed band costs nothing: the refresh is a no-op that never touches the
/// engine's object tree.
#[test]
fn refresh_is_a_noop_while_the_band_is_closed() {
    let Some(session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    assert!(!refresh_objects(&mut state, &session));
    assert!(state.overlays.command_band.is_none());
}

/// The refresh reports "unchanged" on a second call with no turn in between, so
/// an idle frame does not force a repaint.
#[test]
fn an_unchanged_object_tree_does_not_force_a_repaint() {
    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    session.submit("look");
    seed_player_obj(&mut state, &session);
    assert!(refresh_objects(&mut state, &session), "the first fill is a change");
    assert!(!refresh_objects(&mut state, &session), "…and the second is not");
}

/// SQ-1175: with no turn in between, the second refresh does not merely report
/// "unchanged" — it never re-reads the engine at all. Objects only move when
/// the VM runs, and every path that runs it bumps `turn_epoch`, so the ~20 Hz
/// loop tick must not repeat the object-tree walk (on v4+ the location
/// detection behind it decodes every short name in the game per call).
///
/// Falsify by removing the `objects_epoch` gate in `refresh_objects`: the
/// second call then recomputes, finds the real list differs from the planted
/// sentinel, overwrites it and returns true.
#[test]
fn an_unchanged_epoch_skips_the_recompute_entirely() {
    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    session.submit("look");
    seed_player_obj(&mut state, &session);
    assert!(refresh_objects(&mut state, &session), "the first fill is a change");

    // Plant a sentinel where the engine's answer would go: a gated refresh must
    // not even look, so the sentinel survives.
    state.overlays.command_band.as_mut().unwrap().here = vec!["sentinel".to_string()];
    assert!(!refresh_objects(&mut state, &session), "same epoch: nothing to re-read");
    assert_eq!(
        state.overlays.command_band.as_ref().unwrap().here,
        vec!["sentinel".to_string()],
        "the cached columns were not recomputed"
    );

    // …and the gate INVALIDATES: a new turn re-reads the engine and the real
    // list replaces the sentinel (the stale-cache half of the bargain).
    state.begin_turn();
    assert!(refresh_objects(&mut state, &session), "a bumped epoch recomputes");
    let here = &state.overlays.command_band.as_ref().unwrap().here;
    assert!(!here.iter().any(|h| h == "sentinel"), "the sentinel is gone: {here:?}");
    assert!(!here.is_empty(), "…replaced by the room's real objects");
}

// ── Not a modal ──────────────────────────────────────────────────────────────

/// The story prompt line is gated on `!any_modal_overlay_open()`. The old verb
/// menu counted as a modal, so opening it HID the prompt — a half-typed command
/// with no sign it was still buffered. The band must not.
#[test]
fn the_story_prompt_line_is_still_drawn_while_the_band_is_open() {
    for honor in [true, false] {
        let Some(session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
        let area = Rect::new(0, 0, 80, 24);

        let render = |state: &AppState| -> String {
            let model = session.screen();
            let mut buf = Buffer::empty(area);
            app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
            dump(&buf)
        };

        let mut state = AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = honor;
        state.focus = app::state::Focus::Game;
        state.transcript = vec!["West of House".to_string(), "You are standing here.".to_string()];
        state.input.set("half a comm".to_string(), true);

        let closed = render(&state);
        assert!(closed.contains("half a comm"), "honor={honor}: sanity, the prompt draws closed");

        open_band(&mut state);
        let open = render(&state);
        assert!(
            open.contains("half a comm"),
            "honor={honor}: the band is a dock — the story prompt line stays live"
        );
        assert!(!state.any_modal_overlay_open(), "honor={honor}: and it is not a modal");
    }
}

/// Graphical v6 drops to the cell path while a MODAL overlay is up, because
/// image placements draw over terminal cells. The band is not a modal and lives
/// outside the story pane entirely, so the pixel path must survive it — the old
/// verb menu killed Arthur's and Zork Zero's artwork for as long as it was open.
///
/// Pinned in both `honor_game_colours` modes, per the colour-suite rule.
#[test]
fn v6_keeps_the_pixel_path_while_the_band_is_open() {
    for honor in [true, false] {
        let story_path = fixture_path("zork0-r393-s890714.z6");
        let Ok(story_bytes) = std::fs::read(&story_path) else {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return;
        };
        let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
        let picture_dims = picts.all_pict_dims();
        let std_window = picts.std_window();
        let mut session = GameSession::new_with_trace(
            story_bytes, honor, false, None, false, picture_dims, std_window, None, None
        )
        .expect("Zork0 (v6) should load and boot");
        session.set_pict_source(Some(picts));
        session.flush_boot_pictures();
        let _ = session.take_transcript();

        let mut state = AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        // A real kitty cell size: the pixel path only exists with a picker.
        state.game_picker = Some(app::render::graphics::kitty_picker(14, 28));
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        state.config.honor_game_colours = honor;
        let area = Rect::new(0, 0, 100, 40);

        let frame = |state: &AppState| {
            let model = session.screen();
            let mut buf = Buffer::empty(area);
            app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
        };
        let last_path = |state: &AppState| -> String {
            state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default()
        };

        frame(&state);
        assert_eq!(last_path(&state), "hybrid-ring", "honor={honor}: sanity, the ring runs");

        open_band(&mut state);
        frame(&state);
        assert_eq!(
            last_path(&state),
            "hybrid-ring",
            "honor={honor}: the command band must NOT drop v6 off the pixel path"
        );

        // Falsification anchor: a genuine modal still does drop it, so this test
        // is measuring the gate rather than a path that never changes.
        state.overlays.command_band = None;
        state.overlays.hotkey_dialog = true;
        frame(&state);
        assert_ne!(
            last_path(&state),
            "hybrid-ring",
            "honor={honor}: a real modal still drops the ring (the gate is live)"
        );
    }
}

// ── Geometry against a real frame ────────────────────────────────────────────

/// The band claims a bottom band of the frame and the story pane shrinks — it
/// never overlaps the help row, and it leaves the story pane usable.
#[test]
fn the_band_carves_a_bottom_strip_without_eating_the_help_row() {
    let mut state = AppState::default();
    let area = Rect::new(0, 0, 100, 40);

    let closed = app::layout::compute_pane_layout(area, &state, 0);
    open_band(&mut state);
    let open = app::layout::compute_pane_layout(area, &state, 0);

    assert_eq!(open.help_row, closed.help_row, "the help row does not move");
    assert!(open.command_band.height > 0);
    assert_eq!(open.command_band.y + open.command_band.height, open.help_row.y);
    assert_eq!(
        open.story.height + open.command_band.height,
        closed.story.height,
        "the band's rows come out of the story pane"
    );
    assert!(open.story.height > 0, "and the story pane survives");
}

/// Everything visible in the band is clickable, and the rects it emits land
/// inside the band's own area (so `band_mouse_action` can claim exactly them).
#[test]
fn every_band_element_emits_a_hit_rect_inside_the_band() {
    use app::render::command_band::{draw_command_band, CommandBandHits};

    let mut state = AppState::default();
    open_band(&mut state);
    {
        let b = state.overlays.command_band.as_mut().unwrap();
        b.here = vec!["iron door".to_string()];
        b.carried = vec!["brass key".to_string()];
        b.pick_word("unlock");
    }
    let area = Rect::new(0, 30, 100, 8);
    let mut buf = Buffer::empty(Rect::new(0, 0, 100, 40));
    let mut hits = CommandBandHits::default();
    draw_command_band(&state, area, &mut buf, &mut 0, &mut hits);

    assert_eq!(hits.area, area);
    assert!(!hits.headers.is_empty(), "column headers are clickable");
    assert!(!hits.quick.is_empty(), "quick words are clickable");
    assert!(!hits.rows.is_empty(), "rows are clickable");
    assert!(!hits.columns.is_empty(), "columns are wheel targets");

    let inside = |r: &Rect| {
        r.x >= area.x && r.right() <= area.right() && r.y >= area.y && r.bottom() <= area.bottom()
    };
    for (_, r) in &hits.headers {
        assert!(inside(r), "header rect {r:?} escapes the band");
    }
    for (_, _, r) in &hits.rows {
        assert!(inside(r), "row rect {r:?} escapes the band");
    }
    for (_, r) in &hits.quick {
        assert!(inside(r), "quick rect {r:?} escapes the band");
    }
    // The object columns are reachable now, so their rows are real pick targets.
    assert!(hits.rows.iter().any(|(c, _, _)| *c == COL_HERE));
}

// ── The VERB column is the story's own grammar (SQ-1111) ─────────────────────

/// The defect, proved in BOTH directions against a real story.
///
/// The band's whole job is to say what is possible, and until SQ-1111 its VERB
/// column was a hardcoded 36-verb generic set that nothing fed the running
/// story's grammar. Zork I release 88 falsifies it either way, and both are
/// checked here against `zvm-cli` transcripts taken by hand:
///
/// * `show` is in the built-in table, and Zork I answers
///   `I don't know the word "show".` — a verb the panel offered and the game
///   does not have;
/// * `dig`, `count` and `pray` are Zork I verbs (`> dig` → *What do you want to
///   dig in?*) that the built-in table never named.
///
/// Falsify by reverting `refresh_verbs` to a no-op: the column stays on the
/// fallback and every assertion below flips.
#[test]
fn the_verb_column_is_the_running_story_s_own_grammar() {
    let Some(session) = boot_zmachine("zork1-r88-s840726.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);

    let before = state.overlays.command_band.as_ref().unwrap().items(COL_VERB);
    assert!(before.contains(&"show".to_string()), "the fallback offers `show`");
    assert!(!before.contains(&"dig".to_string()), "…and never named `dig`");

    assert!(refresh_verbs(&mut state, &session), "the story's grammar replaces the fallback");
    let band = state.overlays.command_band.as_ref().unwrap();
    assert_eq!(band.verb_source, VerbSource::Story);
    let after = band.items(COL_VERB);

    // Direction one: a verb the panel offered that this game rejects, gone.
    assert!(!after.contains(&"show".to_string()), "Zork I has no `show` verb");
    // Direction two: verbs this game really has, now offered.
    for word in ["dig", "count", "pray", "plugh", "wave", "burn", "tie"] {
        assert!(after.contains(&word.to_string()), "Zork I knows `{word}`: {}", after.len());
    }
    // …and the everyday spellings survive, even though Zork I's own verb
    // records are named `carry`, `gaze`, `hide` and `chuck` (Infocom lists a
    // verb's synonyms in dictionary order, so the first is merely the
    // alphabetically-earliest). This is why the column is every SPELLING.
    // Asked of the TABLE rather than the column, because `items(COL_VERB)`
    // drops what the quick row can finish in one click — SQ-1128 narrowed that
    // to words that cannot take an object, which is the case below.
    for word in ["take", "look", "put", "throw", "open", "read", "turn"] {
        assert!(
            band.verb_by_word(word).is_some(),
            "the player's own word `{word}` reaches the story's verb"
        );
    }
    assert!(after.len() > 200, "the whole grammar, not a curated slice: {}", after.len());
    let mut sorted = after.clone();
    sorted.sort();
    assert_eq!(after, sorted, "alphabetical — the only order a list this long can be scanned in");
}

/// SQ-1128, on the story that raised it: the column jumped from `lock` to
/// `lose` because `look` was on the quick row, and the user concluded the
/// feature was broken.
///
/// A quick pick fires the BARE word, so a quick row is only a substitute for a
/// word that IS complete bare. Zork I's look-verb has twelve syntax lines,
/// eleven of them `gaze at/in/under/behind/… OBJ`; not one of them survives
/// into `VerbEntry::lines`, which is why a rule asked through `max_nouns()`
/// would still drop it. `takes_object` is asked of the raw grammar instead.
///
/// The same argument returns two more Zork I words nobody noticed missing:
/// `enter` and `exit`, excluded as direction-equivalents of the quick row's
/// `in`/`out` and both really `enter OBJ` / `exit OBJ` here. Across the corpus
/// it also returns `bow`, which the band read as north because the MAPPER's
/// parser does, in the twelve stories that have it (Sherlock, Trinity,
/// Plundered Hearts, …); Zork I is not one of them. SQ-1130 took the reuse out
/// from under all three — `enter`, `exit` and `bow` are ordinary words to the
/// band now, and pass this rule on their own merits.
///
/// Falsify by reverting `items(COL_VERB)` to the flat quick exclusion: every
/// word in the first loop disappears from the column with the reported symptom.
#[test]
fn quick_words_that_take_an_object_stay_in_zork_i_s_column() {
    let Some(session) = boot_zmachine("zork1-r88-s840726.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    assert!(refresh_verbs(&mut state, &session));
    let band = state.overlays.command_band.as_ref().unwrap();
    let items = band.items(COL_VERB);

    // Non-vacuity: this really is the story's own 200-odd word column.
    assert_eq!(band.verb_source, VerbSource::Story);
    assert!(items.len() > 200, "the whole grammar: {}", items.len());

    for word in ["look", "enter", "exit"] {
        assert!(
            items.contains(&word.to_string()),
            "`{word}` takes an object in Zork I, so one click cannot finish it"
        );
    }
    // The reported symptom, exactly: no gap between `lock` and `lose`.
    let lock = items.iter().position(|w| w == "lock").expect("Zork I has `lock`");
    let lose = items.iter().position(|w| w == "lose").expect("Zork I has `lose`");
    assert!(
        items[lock..lose].contains(&"look".to_string()),
        "the column no longer jumps `lock` → `lose`: {:?}",
        &items[lock..=lose]
    );

    // …and the words the quick row really does finish are still excluded.
    // Zork I's `wait` has one bare line and nothing else (Deadline's has
    // `wait for OBJ`, which is why this is asked of the grammar, not of a list).
    for word in ["wait", "inventory", "again", "north", "south"] {
        assert!(
            !items.contains(&word.to_string()),
            "`{word}` is complete on its own and stays on the quick row"
        );
    }
    assert!(band.verb_by_word("wait").is_some(), "…still in the TABLE, just not the column");
}

// ── SQ-1126: Infocom's own test rig is not something to try ──────────────────

/// Zork I r52 ships five sigil verbs in its retail grammar, and alphabetical
/// order put every one of them at the TOP of the column: `#command`, `#random`,
/// `#record`, `#unrecor`, `$verif`. They are Infocom's regression rig (record a
/// playthrough, replay it with the RNG pinned) plus the §15 checksum check —
/// not part of the game, and the first thing a browsing player met.
///
/// The rule is structural, so this asks it of the production path: the story's
/// own grammar through `Config::layer_band_verbs`, which is the one place a
/// column is assembled. Falsify by dropping `without_sigil_verbs` from
/// `Config::for_display` — the five words come back, in the same first five
/// rows.
#[test]
fn zork_i_r52_s_column_drops_the_test_harness_verbs() {
    let Some(session) = boot_zmachine("zork1-invclues-r52-s871125.z5") else { return };
    let vocab = session.story_vocabulary().expect("Zork I r52's grammar reads");
    let entries = verbs_from_grammar(vocab.verbs());

    let unfiltered: Vec<String> = entries.iter().map(|e| e.word.clone()).collect();
    for word in ["#command", "#random", "#record", "#unrecor", "$verif"] {
        assert!(unfiltered.contains(&word.to_string()), "r52 really holds `{word}`");
    }
    assert_eq!(
        unfiltered[..5],
        ["#command", "#random", "#record", "#unrecor", "$verif"],
        "…at the very top of an alphabetical column, which is the whole complaint"
    );

    let column = |cfg: &Config| -> Vec<String> {
        cfg.layer_band_verbs(VerbTable::new(entries.clone(), VerbSource::Story))
            .entries
            .into_iter()
            .map(|e| e.word)
            .collect()
    };

    // With the adult list off, the sigil rule is the ONLY thing removing rows,
    // so the delta is exactly the five — and it is not on that list, which is
    // the point of keeping a rule and a judgement apart.
    let sigils_only = column(&Config { hide_adult_words: false, ..Config::default() });
    assert_eq!(
        unfiltered.len() - sigils_only.len(),
        5,
        "five words out of {}: a rule, not a scrub",
        unfiltered.len()
    );

    let filtered = column(&Config::default());
    for shown in [&sigils_only, &filtered] {
        assert!(
            !shown.iter().any(|w| w.starts_with('#') || w.starts_with('$')),
            "no sigil word survives: {:?}",
            &shown[..8]
        );
        assert_eq!(shown[0], "activate", "the column now opens on a verb a player would try");
        for kept in ["take", "look", "pray", "dig", "count"] {
            assert!(shown.contains(&kept.to_string()), "`{kept}` is untouched");
        }
    }
}

/// DISPLAY ONLY, against the real parser (the SQ-1122 pin, applied to the other
/// rule): `$verify` is a genuinely useful diagnostic — the easiest way to see
/// which interpreter number lanthorn reports to a game without a debug build —
/// so it must still work when typed. Filter the column, not the parser.
#[test]
fn typing_a_sigil_command_still_reaches_zork_i_s_parser() {
    let Some(mut session) = boot_zmachine("zork1-invclues-r52-s871125.z5") else { return };
    let reply = session.submit("$verify").transcript.to_lowercase();
    assert!(!reply.is_empty(), "the turn produced a reply");
    assert!(
        !reply.contains("don't know the word"),
        "the story still knows the word it always knew: {reply:?}"
    );
    assert!(
        reply.contains("verif") || reply.contains("disk") || reply.contains("interpreter"),
        "and it is the checksum check answering: {reply:?}"
    );
}

/// The shapes come off the story's syntax lines too, not off a declared arity —
/// including the alternation a single arity could never hold.
#[test]
fn the_column_s_shapes_are_the_story_s_syntax_lines() {
    let Some(session) = boot_zmachine("zork1-r88-s840726.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    assert!(refresh_verbs(&mut state, &session));
    let band = state.overlays.command_band.as_ref().unwrap();

    // `take noun`, `take noun from noun`, `take noun off noun` — three lines,
    // one verb. The old model had room for exactly one shape per verb.
    let take = band.verb_by_word("take").expect("take is offered");
    assert_eq!(take.max_nouns(), 2);
    assert!(take.accepts(1) && take.accepts(2), "finished at one object AND at two");
    assert!(take.joiners().contains(&"from"), "{:?}", take.joiners());

    // `unlock noun with noun` and nothing else: two objects, always.
    let unlock = band.verb_by_word("unlock").expect("unlock is offered");
    assert_eq!(unlock.joiner(), Some("with"));
    assert!(!unlock.accepts(1), "Zork I has no bare `unlock noun`");

    // Zork I's look-verb takes NO bare object — every one of its one-object
    // lines needs a preposition first (`look at noun`, `look in noun`), so the
    // band must not offer `look lamp`, a command the story really refuses.
    let look = band.verb_by_word("look").expect("look is offered");
    assert_eq!(look.max_nouns(), 0);
    assert!(look.accepts(0));
}

/// Picking a verb opens exactly the columns that verb's own lines reach, and
/// ANY of its joining words moves the band to the second-object column — the
/// alternation `Arity` could not express.
#[test]
fn the_columns_follow_the_story_s_own_shapes() {
    let Some(session) = boot_zmachine("zork1-r88-s840726.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    assert!(refresh_verbs(&mut state, &session));

    let band = state.overlays.command_band.as_mut().unwrap();
    band.here = vec!["mailbox".to_string()];
    band.carried = vec!["lamp".to_string()];

    band.sync_from_input("look");
    assert!(!band.col_reachable(COL_HERE), "Zork I's `look` takes no bare object");

    band.sync_from_input("take ");
    assert!(band.col_reachable(COL_HERE), "`take noun` opens the object columns");
    assert!(!band.col_reachable(COL_SECOND), "…and not the second one yet");

    // `from` and `off` are both Zork I take-lines; either one advances.
    for joiner in ["from", "off"] {
        band.sync_from_input(&format!("take lamp {joiner} "));
        assert!(
            band.col_reachable(COL_SECOND),
            "`take … {joiner} …` is a real Zork I line and must open the second column"
        );
        assert_eq!(band.column_label(COL_SECOND), "FROM…", "the header names the FIRST joiner");
    }
}

/// The fallback survives, and says so. A story whose grammar cannot be read
/// keeps a usable column rather than an empty one — and the column relabels
/// itself, the way `here_is_seen` already relabels the object column, instead
/// of passing a generic list off as this story's own.
#[test]
fn a_story_with_no_readable_grammar_keeps_the_fallback_and_labels_it() {
    // Journey is menu-driven: `zvm::grammar::Grammar::load` answers `Absent`,
    // so there is nothing to read and nothing to offer.
    let Some(session) = boot_zmachine("journey-r83-s890706.z6") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    assert!(!refresh_verbs(&mut state, &session), "no grammar, no change");

    let band = state.overlays.command_band.as_ref().unwrap();
    assert_eq!(band.verb_source, VerbSource::Builtin);
    assert!(!band.items(COL_VERB).is_empty(), "the column is never empty");
    assert_eq!(band.column_label(COL_VERB), "VERB — generic", "it admits what it is");

    // …and the label costs a header row, which the story's own column does not
    // spend: the header is the ONE thing worth the row (SQ-0675 reclaimed it).
    use app::render::command_band::{draw_command_band, CommandBandHits};
    let mut buf = Buffer::empty(Rect::new(0, 0, 100, 40));
    let mut hits = CommandBandHits::default();
    draw_command_band(&state, Rect::new(0, 30, 100, 8), &mut buf, &mut 0, &mut hits);
    assert!(
        hits.headers.iter().any(|(c, _)| *c == COL_VERB),
        "the generic label is drawn as VERB's header"
    );
    assert!(dump(&buf).contains("VERB — generic"), "…and reaches the screen");
}

/// Read once per open, not once per tick: the grammar table is static.
#[test]
fn the_grammar_is_read_once_per_open() {
    let Some(session) = boot_zmachine("zork1-r88-s840726.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    assert!(refresh_verbs(&mut state, &session), "the first read is a change");
    assert!(!refresh_verbs(&mut state, &session), "…and an idle tick forces no repaint");
}

/// `[command_band] verbs` still replaces the column wholesale — the story's own
/// grammar included — and says whose list it is. Existing configs keep working.
#[test]
fn a_configured_verb_list_outranks_the_story_s_grammar() {
    let Some(session) = boot_zmachine("zork1-r88-s840726.z3") else { return };
    let mut state = AppState::default();
    state.config.command_band.verbs = vec![app::config::VerbConfig {
        word: "polish".to_string(),
        arity: "object".to_string(),
        prep: None,
    }];
    let (table, _) = state.config.command_band.resolve_verbs();
    state.overlays.command_band = Some(CommandBandState::new(table, default_quick()));
    state.band_dock.toggle_to(true, true);

    assert!(!refresh_verbs(&mut state, &session), "the player's own list is never overwritten");
    let band = state.overlays.command_band.as_ref().unwrap();
    assert_eq!(band.verb_source, VerbSource::Configured);
    assert_eq!(band.items(COL_VERB), vec!["polish".to_string()]);
    assert_eq!(band.column_label(COL_VERB), "VERB — yours");
}

/// `extra_verbs` now patches the STORY's list rather than a constant.
#[test]
fn extra_verbs_extend_the_story_s_own_column() {
    let Some(session) = boot_zmachine("zork1-r88-s840726.z3") else { return };
    let mut state = AppState::default();
    state.config.command_band.extra_verbs = vec![app::config::VerbConfig {
        word: "frotz".to_string(),
        arity: "object".to_string(),
        prep: None,
    }];
    let (table, _) = state.config.command_band.resolve_verbs();
    state.overlays.command_band = Some(CommandBandState::new(table, default_quick()));
    state.band_dock.toggle_to(true, true);

    assert!(refresh_verbs(&mut state, &session));
    let band = state.overlays.command_band.as_ref().unwrap();
    assert_eq!(band.verb_source, VerbSource::Story, "still the story's, with one added");
    let items = band.items(COL_VERB);
    assert!(items.contains(&"frotz".to_string()), "the extra survives the grammar arriving");
    assert!(items.contains(&"dig".to_string()), "…and so does the story's own list");
}

/// A Scott Adams database has no grammar module and does not need one: its
/// whole grammar is `VERB` / `VERB NOUN`, which is a fact about Scott rather
/// than a gap. The column must still be the game's own words — and the fixture
/// is redistributable, so this one never skips.
#[test]
fn scott_adams_answers_with_its_own_two_line_grammar() {
    let bytes = include_bytes!("../../../scott/tests/tiny_cave.dat").to_vec();
    let session = app::scott_session::ScottSession::new(bytes, None).expect("tiny_cave.dat loads");
    let mut state = AppState::default();
    open_band(&mut state);
    assert!(refresh_verbs(&mut state, &session), "Scott's vocabulary drives the column too");

    let band = state.overlays.command_band.as_ref().unwrap();
    assert_eq!(band.verb_source, VerbSource::Story);
    let items = band.items(COL_VERB);
    assert!(!items.is_empty());
    assert!(items.iter().any(|w| w == "take"), "a synonym gets its own row: {items:?}");
    // Every Scott verb has exactly the two lines, so every one of them takes an
    // object and is also complete alone — uniform, and read rather than assumed.
    for word in &items {
        let v = band.verb_by_word(word).expect("every listed word is in the table");
        assert_eq!(v.max_nouns(), 1, "`{word}`: Scott's grammar is VERB NOUN");
        assert!(v.accepts(0) && v.accepts(1), "`{word}`: the noun is optional");
        assert!(v.joiner().is_none(), "`{word}`: a two-word parser has no prepositions");
    }
}

/// Glulx answers the same seam — `gvm::grammar` reads Inform's tables out of a
/// Glulx image, so the column is that story's own verbs with their own syntax
/// lines, and nothing about the band knows which engine spoke.
#[test]
fn a_glulx_story_drives_the_column_through_the_same_seam() {
    let path = fixture_path("Dr Ludwig and the Devil.gblorb");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let b = blorb::Blorb::parse(bytes).expect("a gblorb parses");
    let exec = b.executable().expect("an Exec chunk").1.to_vec();
    let session =
        app::glulx_session::GlulxSession::new(exec, 80, 24, true, false, false, (1, 1), Some(b), &[])
            .expect("Dr Ludwig boots");

    let mut state = AppState::default();
    open_band(&mut state);
    assert!(refresh_verbs(&mut state, &session), "a readable Glulx grammar drives the column");

    let band = state.overlays.command_band.as_ref().unwrap();
    assert_eq!(band.verb_source, VerbSource::Story);
    let items = band.items(COL_VERB);
    assert!(items.len() > 50, "a whole Inform verb table, not a slice: {}", items.len());
    // Inform declares its verbs' first word canonically, so unlike Infocom the
    // everyday spelling is already the leading one — both still get a row.
    let take = band.verb_by_word("take").expect("Inform's library take");
    assert!(take.accepts(1), "`take noun`");
    assert!(take.joiners().contains(&"from"), "`take noun from noun`: {:?}", take.joiners());
    let put = band.verb_by_word("put").expect("Inform's library put");
    assert!(
        put.joiners().contains(&"in") && put.joiners().contains(&"on"),
        "`put … in …` and `put … on …` are two lines of one verb: {:?}",
        put.joiners()
    );
}

/// SQ-1133: the CARRIED column reads the same nesting walk the *here* column
/// does, so an opened sack in your hands offers its lunch and a shut one does
/// not.
///
/// Mini-Zork r34/s871124, Kitchen, 5 then 6 turns in. The band's carried column
/// used to be `Introspect::contents` — the inventory dock's flat list of what is
/// in your hands — which is a different question, and answering it here meant
/// the word for a thing the parser accepts had no row anywhere.
///
/// Falsify by restoring `intro.contents(p)` in `refresh_objects`: the second
/// half fails with `lunch` absent from the column.
#[test]
fn the_carried_column_reaches_into_an_opened_container() {
    let Some(mut session) = boot_zmachine("minizork-r34-s871124.z3") else { return };
    let mut state = AppState::default();
    open_band(&mut state);
    for c in ["n", "e", "open window", "west", "take sack"] {
        session.submit(c);
    }
    seed_player_obj(&mut state, &session);
    refresh_objects(&mut state, &session);
    // The column carries the name the band would TYPE — Mini-Zork prints
    // "brown sack" — so match on the noun inside it rather than on equality.
    let holds = |col: &[String], w: &str| col.iter().any(|c| c.to_lowercase().contains(w));
    let shut = state.overlays.command_band.as_ref().unwrap().items(COL_CARRIED);
    assert!(holds(&shut, "sack"), "non-vacuity: {shut:?}");
    assert!(!holds(&shut, "lunch"), "a shut sack's lunch is not a word the band may offer: {shut:?}");

    session.submit("open sack");
    // Mirror the run loop: the turn finisher bumps `turn_epoch`, which is what
    // lets the epoch-gated refresh re-read the tree (SQ-1175).
    state.begin_turn();
    refresh_objects(&mut state, &session);
    let open = state.overlays.command_band.as_ref().unwrap().items(COL_CARRIED);
    assert!(
        holds(&open, "lunch"),
        "an opened sack's lunch is one the parser takes, so the column offers it: {open:?}"
    );
}

// ── The printed-word block (SQ-1135) ─────────────────────────────────────────

/// **Arthur's crystal, which is the reported defect.**
///
/// | fixture | release / serial | turns in | the frame |
/// |---|---|---|---|
/// | `stories/arthur-r74-s890714.z6` | 74 / 890714 | 12 taps + `n` to the restore question, then `x torque` | the churchyard, the torque on the ground |
///
/// The story prints, in answer to `x torque`:
///
/// ```text
///   The torque is an open neckband made of twisted metal, and it looks like
///   it's about your size. It ends in two knobs, and imbedded in one of the
///   knobs is a sliver of crystal that gives off a faint glow.
/// ```
///
/// `x crystal` works from here — the crystal is the hint menu — and before
/// SQ-1135 the word was in NO column. The *here* column is the object tree,
/// which stops at the torque; the printed-word block was the fallback for an
/// engine with no object tree at all, so the Z-machine, which can say the most,
/// offered the least.
///
/// Falsify by putting the `None =>` arm back in `refresh_objects` (the block
/// only for engines with no tree): `crystal` leaves the column and the last
/// assertion fails.
#[test]
fn arthurs_crystal_reaches_the_band_once_the_story_has_named_it() {
    let Some(mut session) = boot_v6("arthur-r74-s890714.z6", 74, "890714") else { return };
    let mut state = AppState::default();
    open_band(&mut state);

    // Everything the story has said so far, in the transcript, the way
    // `turn.rs` leaves it — the scrape reads the transcript, not the last reply.
    let say = |state: &mut AppState, text: &str| {
        for line in text.split('\n') {
            state.push_transcript_kind(line, app::state::TranscriptKind::Story);
        }
    };
    say(&mut state, &session.take_transcript());
    for _ in 0..12 {
        let r = match session.pending_input() {
            app::session::InputKind::Line => session.submit(""),
            app::session::InputKind::Char => session.submit_char(13),
            app::session::InputKind::Event => session.submit(""),
        };
        let t = r.transcript.clone();
        say(&mut state, &t);
        if t.to_lowercase().contains("y or n") {
            let t2 = session.submit_char(b'n').transcript;
            say(&mut state, &t2);
        }
    }
    app::input::refresh_seen_words(&mut state, &session);

    // Non-vacuity, twice over: this is the churchyard, and the crystal has NOT
    // been named yet. Without both, the assertions below could pass on a frame
    // the case is not about.
    assert_eq!(
        session.current_location().map(|l| l.name),
        Some("churchyard".to_string()),
        "the specimen frame is the churchyard",
    );
    assert!(
        !state.seen_nouns.contains(&"crystal".to_string()),
        "nothing has printed `crystal` yet: {:?}",
        state.seen_nouns,
    );

    let described = session.submit("x torque").transcript;
    println!("x torque →{described}");
    assert!(
        described.contains("sliver of crystal"),
        "the description that names the crystal is what this case is about: {described:?}",
    );
    say(&mut state, &described);
    app::input::refresh_seen_words(&mut state, &session);
    seed_player_obj(&mut state, &session);
    refresh_objects(&mut state, &session);

    // The story knows the word, and it is a THING — an object answers to it —
    // so it reaches the printed-word block, newest first.
    assert_eq!(
        state.seen_nouns.first().map(String::as_str),
        Some("crystal"),
        "the word just printed leads the block: {:?}",
        state.seen_nouns,
    );

    let band = state.overlays.command_band.as_ref().expect("the band is open");
    println!("here (scope): {:?}", band.here);
    println!("seen: {:?}", band.here_seen);
    assert!(
        band.here.iter().any(|w| w.to_lowercase().contains("torque")),
        "non-vacuity: the object tree still reaches the torque itself: {:?}",
        band.here,
    );
    assert!(
        !band.here.iter().any(|w| w.to_lowercase().contains("crystal")),
        "…and does NOT reach the crystal inside it, which is why the block exists: {:?}",
        band.here,
    );
    assert_eq!(
        band.here_source,
        app::state::HereSource::Mixed,
        "scope rows and printed rows in one column, so the header claims neither",
    );

    let items = band.items(COL_HERE);
    assert!(items.contains(&"crystal".to_string()), "the crystal is offerable: {items:?}");
    // …and in the WITH… column too, which is the other noun slot.
    let second = band.items(COL_SECOND);
    assert!(second.contains(&"crystal".to_string()), "the other noun slot too: {second:?}");
    // The rows carry their provenance, so the crystal draws dimmed and the
    // torque does not.
    let rows = band.rows(COL_HERE);
    let seen_of = |w: &str| rows.iter().find(|r| r.text.to_lowercase().contains(w)).map(|r| r.seen);
    assert_eq!(seen_of("crystal"), Some(true), "a printed word is a weaker claim");
    assert_eq!(seen_of("torque"), Some(false), "an object the tree reports is not");
}

/// **The dictionary's noun bit now agrees with the story's own objects on a
/// Version 6 Infocom story** — and this case is the record of the day it
/// started to (SQ-1153), because it used to assert the opposite.
///
/// `zvm::grammar::decode_roles` read `GrammarFormat::InfocomV6` with Inform's
/// layout — `$01` verb, `$80` noun — and Infocom's V6 games keep neither there.
/// `$80` selected `are is was were will` on Arthur and missed the crystal, the
/// torque and the sword outright, which is why the band's block asks the
/// story's own OBJECTS instead (SQ-1135). The layout is now measured against
/// all three V6 titles' parsers — `$01` verb, `$02` noun, `$04` adjective, in
/// the LAST byte of the entry (`zvm::grammar`'s `F_INFOCOM_V6_VERB`, and
/// `crates/zvm/tests/v6_word_roles.rs` for the evidence) — so both routes now
/// answer, and this case pins that they agree.
///
/// **The object route stays** regardless: `all_object_words` is what the band
/// actually reads, and a word array is the only thing that can follow Arthur's
/// `password` object as it rewrites its own parse names mid-puzzle.
#[test]
fn the_v6_noun_bit_and_the_objects_name_the_same_things() {
    let Some(session) = boot_v6("arthur-r74-s890714.z6", 74, "890714") else { return };
    let vocab = <GameSession as Engine>::story_vocabulary(&session).expect("a readable dictionary");
    for w in ["crystal", "torque", "sword", "is", "was", "were"] {
        println!("{w}: {:?}", vocab.roles(w));
    }
    // Every one of these is a thing the parser takes, and the bit now has them.
    for thing in ["crystal", "torque", "sword", "knob"] {
        assert_eq!(session.knows_word(thing), Some(true), "{thing} is in the dictionary");
        assert!(
            vocab.roles(thing).is_some_and(|r| r.noun),
            "{thing:?} is a thing this story names and the noun bit has to have it",
        );
    }
    // …and the verbs of being it used to pick out, which name nothing at all,
    // are no longer nouns. `was` is a DESCRIPTOR on Arthur ($84) and stays one;
    // what matters is that none of the three is offered as a thing.
    for not_a_thing in ["is", "was", "were"] {
        assert!(
            !vocab.roles(not_a_thing).is_some_and(|r| r.noun),
            "{not_a_thing:?} used to carry the noun bit on Arthur — the whole of SQ-1153",
        );
    }
    // The objects answer correctly, and they are what the block reads.
    let objects =
        session.introspect().and_then(|i| i.all_object_words()).expect("a v6 object table");
    let names = |w: &str| objects.iter().any(|o| o.refers_to(w));
    for thing in ["crystal", "torque", "sword", "knob"] {
        assert!(names(thing), "an object answers to {thing:?}");
    }
    for not_a_thing in ["is", "were"] {
        assert!(!names(not_a_thing), "no object answers to {not_a_thing:?}");
    }
    // `was` is the exception, and it is the STORY's own answer rather than a
    // slip: Arthur's password object rewrites its own parse names as the puzzle
    // runs, and at boot they read `password passwords word words fair begot
    // there lot`. Asking the objects inherits whatever the objects say, exactly
    // as asking the dictionary inherits whatever the flags say — the difference
    // is that this one is right about the crystal.
    let holders: Vec<Option<String>> =
        objects.iter().filter(|o| o.refers_to("was")).map(|o| o.display_name()).collect();
    assert_eq!(
        holders,
        vec![Some("password".to_string())],
        "the one object that answers to `was` is the password, mid-puzzle",
    );
}

/// SQ-1151: the column offers no word a player cannot type.
///
/// **The reported defect**, seen in Arthur's command band: it listed both `be` and
/// `be?`. The `?` is genuinely in the dictionary entry rather than a ZSCII
/// fallback — `be` and `be?` differ in their second word (`0x14a5` against
/// `0x54a5`, Z-char 21 being a literal `?` in alphabet A2) and hold different
/// verb numbers, `$c7` and `$f7`. But `?` is one of the six input separators
/// Arthur declares at its dictionary header, so the tokeniser splits `be?` into
/// `be` and `?` before the parser looks anything up. **No sequence of keystrokes
/// reaches that entry**, and clicking it composes a line the parser will split.
/// Infocom used the trick deliberately: a separator inside a word makes a slot
/// the game's own code can name without a player stumbling into it, which is
/// what Arthur's `int.num`, `int.tim`, `l.g` and `no.word` are too.
///
/// [`VerbTable::without_sigil_verbs`] cannot catch this — it tests the FIRST
/// character against two fixed sigils, and here the offending character is last
/// and comes from this story's own separator table. The rule that does is the
/// same shape sourced from the story:
/// `StoryVocabulary::without_untypeable_words`, applied at the one vocabulary
/// seam so the offer and the reveal are spared it too.
///
/// | fixture | release / serial | turns |
/// |---|---|---|
/// | `stories/arthur-r74-s890714.z6` | 74 / 890714 | 0 — the grammar is static |
/// | `stories/shogun-r322-s890706.z6` | 322 / 890706 | 0 — the grammar is static |
///
/// Falsify by lifting the filter out of `VocabState::get`: the raw dictionary
/// still holds `be?` (asserted below, so this case cannot go vacuous), and the
/// column shows it again.
///
/// [`VerbTable::without_sigil_verbs`]: app::render::command_band::VerbTable::without_sigil_verbs
#[test]
fn the_verb_column_drops_a_word_the_storys_own_tokeniser_would_split() {
    for (file, release, serial) in [
        ("arthur-r74-s890714.z6", 74, "890714"),
        ("shogun-r322-s890706.z6", 322, "890706"),
    ] {
        let Some(session) = boot_v6(file, release, serial) else { continue };

        // Non-vacuity, and the whole premise in two assertions: the entry is
        // real, and the story's own tokeniser will not hand it over whole.
        let raw = <GameSession as Engine>::story_vocabulary(&session)
            .expect("a readable Version 6 dictionary");
        assert!(
            raw.verbs().iter().any(|v| v.words.iter().any(|w| w == "be?")),
            "{file}: `be?` is a real verb spelling in the story's own grammar"
        );
        assert_eq!(
            session.split_like_parser("be?"),
            Some(vec!["be".to_string(), "?".to_string()]),
            "{file}: `?` is one of this story's declared input separators"
        );

        let mut state = AppState::default();
        state.overlays.command_band =
            Some(CommandBandState::new(default_verbs(), default_quick()));
        state.band_dock.toggle_to(true, true);
        assert!(refresh_verbs(&mut state, &session), "{file}: the story drives its own column");

        let band = state.overlays.command_band.as_ref().unwrap();
        assert_eq!(band.verb_source, VerbSource::Story, "{file}");
        let items = band.items(COL_VERB);
        let offered: Vec<&str> = items.iter().map(String::as_str).collect();
        assert!(!offered.contains(&"be?"), "{file}: the column offers no word that splits");
        assert!(
            offered.contains(&"be"),
            "{file}: and the reachable verb of the pair is still there"
        );
        // The rest of the column is untouched — this is a rule about structure,
        // not a list of words, and it must not have eaten anything else.
        for kept in ["look", "take", "open"] {
            assert!(offered.contains(&kept), "{file}: {kept} is unaffected");
        }
    }
}
