//! SQ-1042: the words a thing can be CALLED, from the live object tree, through
//! the `Introspect` seam and out into the band's object columns and completion.
//!
//! `Introspect` used to answer every object question with a display name, so a
//! panel offering one was offering something the parser had never agreed to.
//! SQ-1118 built the reader (`ObjectWords` — id, printed name, parse names, all
//! one value) and nothing could reach it; this is the seam that carries it.
//!
//! # The parser is the oracle
//!
//! Every claim below was settled by driving the real story under `zvm-cli` and
//! reading what its own parser said. From Zork I r88 (`stories/`), Up a Tree
//! after `n / n / climb tree` — the printed name is on the left, and it is what
//! the band used to put on the input line:
//!
//! ```text
//!   take bird's nest            → I don't know the word "bird'".
//!   take nest                   → Taken.
//!   take jewel-encrusted egg    → You can't see any jewel-encrusted egg here!
//!   take egg                    → Taken.
//! ```
//!
//! Both objects in that room, refused. And in the Living Room after
//! `n / e / open window / enter window / w`, every word this quest offers for
//! the lantern:
//!
//! ```text
//!   take lamp → Taken.   take lantern → Taken.   take light → Taken.
//!   take brass lantern → Taken.
//! ```
//!
//! # The specimens
//!
//! | fixture | release | turns in | what it shows |
//! |---|---|---|---|
//! | `minizork-r34-s871124.z3` (fetched) | r34/s871124 | 0 and 4 | the whole path, in CI |
//! | `stories/zork1-r88-s840726.z3` | r88/s840726 | 3 | the two names the parser refuses |
//!
//! Mini-Zork is a fetched fixture (SQ-1453), which CI always populates, so
//! every case that matters runs there; the Zork I case skips vacuously
//! without `stories/`.

use app::engine::Engine;
use app::session::GameSession;
use app::state::AppState;

use crate::fixture_paths::fixture_path;

// ── Booting ─────────────────────────────────────────────────────────────────

fn boot(bytes: Vec<u8>) -> GameSession {
    GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
        .expect("a Version 3 story should load and boot")
}

/// Mini-Zork I, a fetched fixture (SQ-1453).
fn minizork() -> GameSession {
    let path = fixture_path("minizork-r34-s871124.z3");
    boot(std::fs::read(&path).unwrap_or_else(|e| panic!("fetched fixture at {}: {e}", path.display())))
}

/// A gitignored commercial story, or `None` so the case can skip.
fn story(name: &str) -> Option<GameSession> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    }
    Some(boot(std::fs::read(&path).ok()?))
}

fn drive(session: &mut GameSession, cmds: &[&str]) {
    for c in cmds {
        session.submit(c);
    }
    let _ = session.take_transcript();
}

/// The objects in scope, exactly as the app reads them: the room's visible
/// contents with the player excluded, plus what the player carries.
fn in_scope(session: &GameSession) -> Vec<app::engine::ObjectWords> {
    let intro = session.introspect().expect("a Z-machine story has an object tree");
    let player = intro.player_object();
    let room = session.current_location().expect("a located room").number;
    let mut out = intro.room_objects_excluding(room, player);
    if let Some(p) = player {
        out.extend(intro.contents(p));
    }
    out
}

/// What the band's object columns would offer for everything in scope.
fn offered(session: &GameSession) -> Vec<String> {
    let vocab = <GameSession as Engine>::story_vocabulary(session);
    in_scope(session)
        .iter()
        .filter_map(|o| app::vocab::typeable_name(o, vocab.as_ref()))
        .collect()
}

/// An `AppState` with the scope words refreshed the way `turn.rs` refreshes them.
fn scoped(session: &GameSession) -> AppState {
    let mut state = AppState::default();
    app::input::refresh_scope_words(&mut state, session);
    state
}

// ── The seam ────────────────────────────────────────────────────────────────

/// Every object question now answers with the id, the printed name and the
/// parse names as ONE value. Falsify by having `Introspect` hand back display
/// names again: nothing below can be asked at all.
#[test]
fn the_seam_carries_what_a_thing_is_called_beside_what_it_is_printed_as() {
    let session = minizork();
    let here = in_scope(&session);
    assert!(!here.is_empty(), "West of House holds objects");

    let mailbox = here
        .iter()
        .find(|o| o.printed_name == "small mailbox")
        .expect("the mailbox is on the lawn");
    assert_eq!(mailbox.words, ["mailbo", "box"], "the words its parser accepts, as stored");
    assert_eq!(mailbox.truncated_at, Some(6), "a Version 3 dictionary keeps six Z-characters");
    assert!(mailbox.property.is_some(), "and the property they were read from");
    // The identity travels with them, which is what lets the *here* column drop
    // the player object by id rather than by name.
    assert!(mailbox.id > 0);
}

/// A story that keeps no parse names anywhere still answers — with its printed
/// name and an empty word list, which is the whole truth about it rather than a
/// failure to look. (`inventory::object_words` with no reader is that story.)
#[test]
fn a_story_with_no_parse_names_answers_with_the_printed_name_alone() {
    let session = minizork();
    let obj = app::inventory::object_words(&session.machine.mem, None, 1);
    assert!(obj.words.is_empty());
    assert_eq!(
        app::vocab::typeable_name(&obj, None),
        obj.display_name(),
        "with nothing better to say, the printed name stands — the old behaviour, kept for \
         exactly the stories that can support nothing else"
    );
}

// ── What a column offers ────────────────────────────────────────────────────

/// The adjective survives where the story marks one. Infocom keeps adjectives in
/// a property of their own, so `small` is nowhere in the mailbox's `SYNONYM`
/// list — but `take small mailbox` works and two knives in one room need it.
#[test]
fn the_column_keeps_the_adjective_the_story_marks() {
    let session = minizork();
    let words = offered(&session);
    assert!(words.contains(&"small mailbox".to_string()), "{words:?}");
    assert!(words.contains(&"white house".to_string()), "{words:?}");
}

/// The two names Zork I's own parser refuses, and what the column offers
/// instead. See this module's header for the `zvm-cli` transcript.
///
/// Falsify by reverting `refresh_objects` to the printed name: the column then
/// offers `bird's nest`, which the story answers with `I don't know the word
/// "bird'"`.
#[test]
fn a_name_the_parser_refuses_is_never_the_one_offered() {
    let Some(mut session) = story("zork1-r88-s840726.z3") else { return };
    drive(&mut session, &["n", "n", "climb tree"]);
    assert_eq!(
        session.current_location().map(|l| l.name),
        Some("Up a Tree".to_string()),
        "three turns in, on the branch — the frame every claim here is about"
    );

    let printed: Vec<String> = in_scope(&session).iter().map(|o| o.printed_name.clone()).collect();
    assert!(printed.contains(&"bird's nest".to_string()), "{printed:?}");
    assert!(printed.contains(&"jewel-encrusted egg".to_string()), "{printed:?}");

    let words = offered(&session);
    assert!(words.contains(&"nest".to_string()), "{words:?}");
    assert!(words.contains(&"egg".to_string()), "{words:?}");
    assert!(!words.iter().any(|w| w.contains('\'')), "no apostrophe survives tokenising: {words:?}");
    assert!(!words.contains(&"jewel-encrusted egg".to_string()), "{words:?}");
}

/// `clove of garlic` is `garlic`: the run of qualifying words stops at the
/// preposition, which is what keeps `of` off the input line.
#[test]
fn a_preposition_inside_a_printed_name_stops_the_run() {
    let mut session = minizork();
    drive(&mut session, &["n", "e", "open window", "west", "open sack"]);
    let words = offered(&session);
    assert!(words.contains(&"garlic".to_string()), "{words:?}");
    assert!(!words.iter().any(|w| w.split_whitespace().any(|t| t == "of")), "{words:?}");
}

/// An Inform 7 object has no hardware short name at all — Counterfeit Monkey
/// has 2,222 named objects and most of them print nothing — so the word list is
/// the only text in the image identifying it. A display shows the whole list; a
/// command line gets the story's own first spelling, which the parser accepts on
/// its own.
#[test]
fn a_printed_nameless_object_is_named_by_the_words_it_answers_to() {
    let o = app::engine::ObjectWords::new(
        0x2008c,
        String::new(),
        vec!["monkey".to_string(), "counterfeit".to_string()],
        Some(1),
        Some(9),
    );
    assert_eq!(o.display_name().as_deref(), Some("monkey counterfeit"));
    assert_eq!(app::vocab::typeable_name(&o, None).as_deref(), Some("monkey"));

    // Nothing at all is the one case with no answer: an empty row is worse than
    // no row.
    let nothing = app::engine::ObjectWords::new(7, String::new(), Vec::new(), None, None);
    assert_eq!(nothing.display_name(), None);
    assert_eq!(app::vocab::typeable_name(&nothing, None), None);
}

// ── Adjectives, where the story keeps a list of them — SQ-1120 ──────────────

/// An Infocom V4+ story keeps its adjectives in a property of their own, and
/// `zvm` reads it: they reach completion as words, and the printed name is no
/// longer the only route to them.
///
/// Verified against the parser on `stories/zork1-invclues-r52-s871125.z5` (the
/// v5 Solid Gold release, in the Living Room after
/// `n / e / open window / enter window / w`): `take brass lantern → Taken.`
#[test]
fn adjectives_the_story_states_are_offered_beside_the_nouns() {
    // Zork I r52's lantern, as `zvm::objects::ParseNames` reads it.
    let lantern = app::engine::ObjectWords::new(
        153,
        "brass lantern".to_string(),
        vec!["lamp".to_string(), "lantern".to_string(), "light".to_string()],
        Some(46),
        Some(9),
    )
    .with_adjectives(vec!["brass".to_string()], 44);
    assert_eq!(
        app::vocab::typeable_words(&lantern, None),
        ["brass", "lantern", "lamp", "light"]
    );
    assert_eq!(app::vocab::typeable_name(&lantern, None).as_deref(), Some("brass lantern"));

    // Beyond Zork r57 object 133, as the reader answers it: printed `key` and
    // nothing more, with four adjectives the screen never shows. The printed
    // name cannot supply those, so the property is the only route to them.
    let key = app::engine::ObjectWords::new(
        133,
        "key".to_string(),
        vec!["key".to_string(), "keys".to_string()],
        Some(49),
        Some(9),
    )
    .with_adjectives(
        ["mauve", "second", "gray", "grey"].map(String::from).to_vec(),
        48,
    );
    assert_eq!(
        app::vocab::typeable_words(&key, None),
        ["key", "keys", "mauve", "second", "gray", "grey"]
    );

    // And a story that cannot be asked offers exactly what it did before: the
    // nouns, and nothing invented.
    let v3 = app::engine::ObjectWords::new(
        164,
        "brass lantern".to_string(),
        vec!["lamp".to_string(), "lanter".to_string(), "light".to_string()],
        Some(18),
        Some(6),
    );
    assert!(!v3.adjectives.is_available());
    // `lanter` never appears: the printed name spells it out and the two cut to
    // the same six-character key. `brass` does not appear either, which is the
    // gap — with no adjective list and no vocabulary to consult, nothing in this
    // story says it is a word.
    assert_eq!(app::vocab::typeable_words(&v3, None), ["lantern", "lamp", "light"]);
}

/// **The census behind the fallback** (SQ-1120's first question).
///
/// SQ-1042 recovers Infocom adjectives through the dictionary's DESC bit ($20)
/// because the property was unreadable, and a story that never sets that bit
/// would lose its adjectives silently. Measured over every Z-machine story in
/// `stories/`, the answer is in two halves:
///
/// * **Every V1–V5 Infocom story sets it**, 34 of 34, generously — 101 words in
///   Zork III up to 658 in A Mind Forever Voyaging, a fifth to a third of each
///   dictionary. The fallback is safe exactly where it is now the only source.
/// * **The three Infocom V6 games effectively do not** — Zork Zero 11 of 1624,
///   Shogun 17 of 1389, Arthur 9 of 1059 — and `zvm::grammar` does not even
///   decode `adjective` for their flag layout, which moved. So on those three
///   the fallback yields NOTHING, and they are precisely the games the
///   adjective property now answers for.
///
/// Inform stories are not in either half: they have no DESC bit and keep
/// adjectives in the `name` array beside the nouns, where `refers_to` already
/// finds them.
#[test]
fn every_pre_v6_infocom_story_sets_the_desc_bit_the_fallback_depends_on() {
    let pre_v6 = [
        "zork1-r88-s840726.z3",
        "zork2-r48-s840904.z3",
        "zork3-r17-s840727.z3",
        "hitchhiker-r59-s851108.z3",
        "planetfall-r37-s851003.z3",
        "trinity-r12-s860926.z4",
        "amfv-r77-s850814.z4",
        "beyondzork-r57-s871221.z5",
        "sherlock-r26-s880127.z5",
    ];
    let v6 = ["zork0-r393-s890714.z6", "shogun-r322-s890706.z6", "arthur-r74-s890714.z6"];

    let marked = |bytes: Vec<u8>| -> (usize, usize) {
        let mem = zvm::memory::Memory::new(bytes).expect("a story that boots");
        let words = zvm::grammar::dictionary_words(&mem);
        let raw = words.iter().filter(|w| w.roles.raw & 0x20 != 0).count();
        (words.len(), raw)
    };

    // Raw bytes, not a booted session: three of these are graphical V6 and this
    // suite's `boot` is a Version 3 harness.
    let bytes_of = |name: &str| -> Option<Vec<u8>> {
        let path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
        match std::fs::read(&path) {
            Ok(b) => Some(b),
            Err(_) => {
                eprintln!("SKIP: {} absent", path.display());
                None
            }
        }
    };

    let mut checked = 0;
    for name in pre_v6 {
        let Some(bytes) = bytes_of(name) else { continue };
        let (total, desc) = marked(bytes);
        assert!(
            desc * 8 >= total,
            "{name}: only {desc} of {total} words carry the DESC bit — the SQ-1042 \
             fallback is the only adjective source below V6 and needs it"
        );
        checked += 1;
    }
    for name in v6 {
        let Some(bytes) = bytes_of(name) else { continue };
        let (total, desc) = marked(bytes);
        assert!(
            desc * 20 < total,
            "{name}: {desc} of {total} words carry $20 — if V6 really marked its \
             adjectives, the fallback would work there and this claim is wrong"
        );
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no Infocom media present");
    }
}

// ── Completion ──────────────────────────────────────────────────────────────

/// The quest's own subject: complete from the objects actually present.
///
/// And spell them out — a Version 3 dictionary stores `mailbo`, which is a
/// fragment to offer a player, while the object's printed name holds `mailbox`
/// in full. Both reach the same entry, because the parser truncates the typed
/// word exactly as the dictionary truncated its own.
#[test]
fn completion_offers_the_things_that_are_here_spelled_out() {
    let session = minizork();
    let state = scoped(&session);
    assert!(state.scope_words.contains(&"mailbox".to_string()), "{:?}", state.scope_words);
    assert!(state.scope_words.contains(&"box".to_string()), "the other word it answers to");
    assert!(
        !state.scope_words.contains(&"mailbo".to_string()),
        "the stored fragment is not what a player is shown: {:?}",
        state.scope_words
    );
    // The adjective is completable too — a player reads "small mailbox" on the
    // screen and may type either half.
    assert!(state.scope_words.contains(&"small".to_string()), "{:?}", state.scope_words);
}

/// Scope outranks the recent prose and the flat dictionary: the thing in front
/// of you comes first.
#[test]
fn what_is_here_is_ranked_above_the_dictionary() {
    use app::input::{apply_action, key_to_action, Action};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let session = minizork();
    let mut state = scoped(&session);
    state.dict_words =
        session.introspect().map(|i| i.vocabulary()).expect("a v3 story has a dictionary");
    let mut mapper = mapper::mapper::Mapper::default();
    for c in "mail".chars() {
        let a = key_to_action(&state, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        assert_eq!(a, Action::InputChar(c));
        apply_action(a, &mut state, &mut mapper);
    }
    assert_eq!(
        state.suggestions.first().map(String::as_str),
        Some("mailbox"),
        "the mailbox on the lawn beats the dictionary's own `mailbo`: {:?}",
        state.suggestions
    );
}

/// **The line between a convenience and a puzzle solver**, and the first thing
/// this quest asked for: a thing inside a CLOSED container must not complete.
///
/// Mini-Zork's Kitchen holds a shut brown sack with a lunch and a clove of
/// garlic in it. Neither may be completable until the player opens it — and the
/// second half of this case is the falsifier, because a filter that hid them
/// always would pass the first half alone.
#[test]
fn a_closed_containers_contents_never_complete() {
    let mut session = minizork();
    drive(&mut session, &["n", "e", "open window", "west"]);
    assert_eq!(session.current_location().map(|l| l.name), Some("Kitchen".to_string()));

    let shut = scoped(&session);
    assert!(shut.scope_words.contains(&"sack".to_string()), "the sack itself is here");
    for secret in ["garlic", "clove", "lunch", "food", "sandwi"] {
        assert!(
            !shut.scope_words.contains(&secret.to_string()),
            "`{secret}` is inside a shut sack and completing it would be a hint: {:?}",
            shut.scope_words
        );
    }

    drive(&mut session, &["open sack"]);
    let open = scoped(&session);
    for now_visible in ["garlic", "clove", "lunch"] {
        assert!(
            open.scope_words.contains(&now_visible.to_string()),
            "an opened sack shows what the game itself now lists: {:?}",
            open.scope_words
        );
    }
}

/// The player's own object never completes and never fills a column — it is
/// structurally a child of whatever room they are in, and Zork's prints
/// `cretin`.
#[test]
fn the_avatar_is_not_a_thing_in_the_room() {
    let mut session = minizork();
    drive(&mut session, &["look"]);
    let state = scoped(&session);
    assert!(!state.scope_words.contains(&"cretin".to_string()), "{:?}", state.scope_words);
    assert!(!offered(&session).iter().any(|w| w.contains("cretin")));
}

/// SQ-1133: **the carried half of scope nests too, and stops at the same lid.**
///
/// The room half has descended into open holders since SQ-0678 — the shut sack
/// on Mini-Zork's kitchen table is the case above — while the carried half read
/// the player's DIRECT children and nothing else. So the very same sack hid its
/// lunch the instant you picked it up, and the two surfaces whose job is to say
/// what is real disagreed about one object depending on whose hands it was in.
///
/// | fixture | release | turns in | what it shows |
/// |---|---|---|---|
/// | `minizork-r34-s871124.z3` (fetched) | r34/s871124 | 5 then 6 | shut, then opened, in the player's hands |
///
/// The parser is the oracle, and it was asked: driven to the Kitchen and
/// `take sack` / `open sack`, Mini-Zork answers `i` with
/// "The brown sack contains: A clove of garlic / A lunch" and
/// `examine lunch` with "There is nothing special about the lunch."
///
/// **Measured depth: one.** Zork I r88 and Mini-Zork r34 both put the lunch and
/// the garlic exactly one level below the opened sack, and an unbounded,
/// openness-respecting walk of the corpus in `stories/` finds nothing at all
/// below the player at boot (62 stories booted; every one 0 deep, because the
/// openness attribute is identified in only 4 of them and nobody is carrying an
/// opened container on turn 0). The cap taken is `MAX_NEST_DEPTH`, shared with
/// the room walk rather than chosen again here.
///
/// Falsify by putting `Introspect::contents` back in `vocab::scope_split`: the
/// second half fails with the garlic and the lunch missing — the reported
/// symptom — while the first half still passes, which is exactly why the first
/// half alone proves nothing.
#[test]
fn a_carried_container_shows_its_contents_only_once_it_is_opened() {
    let mut session = minizork();
    drive(&mut session, &["n", "e", "open window", "west", "take sack"]);
    assert_eq!(session.current_location().map(|l| l.name), Some("Kitchen".to_string()));

    let shut = scoped(&session);
    assert!(shut.scope_words.contains(&"sack".to_string()), "the sack is in hand");
    for secret in ["garlic", "clove", "lunch", "food"] {
        assert!(
            !shut.scope_words.contains(&secret.to_string()),
            "`{secret}` is inside a shut sack — carrying it does not open it: {:?}",
            shut.scope_words
        );
    }

    drive(&mut session, &["open sack"]);
    let open = scoped(&session);
    for now_visible in ["garlic", "clove", "lunch"] {
        assert!(
            open.scope_words.contains(&now_visible.to_string()),
            "an opened sack in your hands reads exactly like one on the table: {:?}",
            open.scope_words
        );
    }
}

/// The same pair on the commercial release the report came from, skipping
/// vacuously without `stories/`: Zork I r88/s840726, Kitchen, 6 turns in
/// (`n`, `e`, `open window`, `enter window`, `take sack`, `open sack`).
#[test]
fn zork1s_carried_sack_reads_the_same_way() {
    let Some(mut session) = story("zork1-r88-s840726.z3") else { return };
    drive(&mut session, &["n", "e", "open window", "enter window", "take sack"]);
    let shut = scoped(&session);
    assert!(
        shut.scope_words.contains(&"sack".to_string()),
        "non-vacuity: the sack really is in hand at this turn count: {:?}",
        shut.scope_words
    );
    assert!(!shut.scope_words.contains(&"lunch".to_string()), "{:?}", shut.scope_words);

    drive(&mut session, &["open sack"]);
    let open = scoped(&session);
    for w in ["lunch", "garlic"] {
        assert!(open.scope_words.contains(&w.to_string()), "{:?}", open.scope_words);
    }
}
