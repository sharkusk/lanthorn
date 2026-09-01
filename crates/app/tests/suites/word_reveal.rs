//! SQ-1107 / SQ-1207: the momentary reveal — the nouns and named things on
//! screen the story really knows, and nothing else.
//!
//! # The parser is the oracle
//!
//! Every claim below was settled by driving the real story under `zvm-cli` and
//! reading what its own parser said. Mini-Zork I r34/s871124, at the opening
//! screen (West of House, 0 moves in):
//!
//! ```text
//!   examine mailbox → The small mailbox is closed.
//!   examine house   → The house is a beautiful white colonial. …
//!   examine door    → The door is closed.
//!   examine field   → [I don't know the word "field".]
//!   examine window  → You can't see any window here!
//! ```
//!
//! Five nouns printed in three lines of prose, and the line the reveal draws
//! runs between two of the three groups, not three (SQ-1135):
//!
//! - **`mailbox`, `house`, `door` — words this story knows.** They light.
//! - **`window` — a word this story knows, for something elsewhere.** It would
//!   light too, wherever the prose printed it; nothing here does.
//! - **`field` — not a word at all.** The story has never heard it, so nothing
//!   lights it. It is also the word the room description leads with, which is
//!   exactly why the feature exists: the prose opens with a noun that does not
//!   exist.
//!
//! And one move later, the mailbox is a room away while its sentence is still on
//! screen:
//!
//! ```text
//!   north            → North of House …
//!   examine mailbox  → You can't see any mailbox here!
//! ```
//!
//! `There is a small mailbox here.` is still on screen and `mailbox` **keeps
//! lighting**, because the claim is about the dictionary and not about scope.
//! That case is [`a_word_the_story_knows_lights_wherever_the_thing_is`], and it
//! asserted the reverse until SQ-1135: the scope test made the engines with the
//! most introspection light the least, and a description naming something in the
//! next room — Arthur's crystal in the torque — lit nothing at all.
//!
//! # The specimens
//!
//! | fixture | release | turns in | what it shows |
//! |---|---|---|---|
//! | `crates/zvm/tests/fixtures/minizork.z3` | r34/s871124 | 0 | the whole path, in CI |
//! | `crates/zvm/tests/fixtures/minizork.z3` | r34/s871124 | 1 (`north`) | scope moving under old text |
//! | `stories/zork1-invclues-r52-s871125.z5` | r52/s871125 | 0 | the noun-AND-adjective contract (SQ-1207); Mini-Zork's Version 3 dictionary cannot report adjectives |
//! | `stories/Dr Ludwig and the Devil.gblorb` | r2/s250306 | 3 intro keys, 0 commands | the same contract on Glulx (SQ-1210) |
//! | `stories/King_of_Shreds_and_Patches.gblorb` | — | boot | Glulx fail-safe: answer truly or refuse honestly (SQ-1210) |
//!
//! Mini-Zork is tracked, so every case built on it runs on CI; nothing there
//! skips. The Version 5 specimen is gitignored (CLAUDE.md's `stories/`) and
//! skips vacuously without it — chosen over Mini-Zork specifically because a
//! Version 1-3 story keeps no readable adjective property at all. The two
//! Glulx specimens are gitignored likewise and skip the same way.

use app::engine::Engine;
use app::reveal::Armed;
use app::session::GameSession;
use app::state::{AppState, TranscriptKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::fixture_paths::fixture_path;

// ── Booting and drawing ─────────────────────────────────────────────────────

/// Mini-Zork I, tracked in the checkout.
fn minizork() -> GameSession {
    let path = fixture_path("minizork-r34-s871124.z3");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("tracked at {}: {e}", path.display()));
    GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
        .expect("a Version 3 story should load and boot")
}

const AREA: Rect = Rect { x: 0, y: 0, width: 72, height: 20 };

/// The state a player is looking at: the story's own output in the transcript,
/// the Guiding Light on (this lives under its switch), and one frame drawn —
/// which is what fills the wrap cache and the viewport geometry the reveal reads.
/// `&mut dyn Engine`, not `GameSession`: the Glulx cases below (SQ-1210) build
/// the same screen from the same seam.
fn screen(session: &mut dyn Engine) -> AppState {
    screen_of(&session.take_transcript())
}

/// [`screen`] for a caller that accumulated the text itself — the Glulx boot
/// taps through intro pauses and gathers several turns' transcript first.
fn screen_of(text: &str) -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.guidance = true;
    for line in text.split('\n') {
        state.push_transcript_kind(line, TranscriptKind::Story);
    }
    draw(&state);
    state
}

/// Render one frame into a throwaway buffer, for its side effects on the wrap
/// cache and `transcript_geom`.
fn draw(state: &AppState) -> Buffer {
    let mut buf = Buffer::empty(AREA);
    app::render::transcript::render_transcript(
        &app::engine::StatusModel::HostManaged,
        None,
        state,
        AREA,
        &mut buf,
        None,
    );
    buf
}

/// Every cell of the drawn frame that the reveal lit, as `(row, text)`.
fn lit_rows(buf: &Buffer) -> Vec<(u16, String)> {
    (AREA.y..AREA.bottom())
        .filter_map(|y| {
            let s: String = (AREA.x..AREA.right())
                .map(|x| {
                    let c = buf.cell((x, y)).unwrap();
                    if c.modifier.contains(Modifier::UNDERLINED) {
                        c.symbol().chars().next().unwrap_or(' ')
                    } else {
                        ' '
                    }
                })
                .collect();
            (!s.trim().is_empty()).then(|| (y, s.trim_end().to_string()))
        })
        .collect()
}

/// The whole drawn frame, for a failure message worth reading.
fn frame(buf: &Buffer) -> String {
    (AREA.y..AREA.bottom())
        .map(|y| {
            let s: String =
                (AREA.x..AREA.right()).map(|x| buf.cell((x, y)).unwrap().symbol()).collect();
            format!("\n  |{}|", s.trim_end())
        })
        .collect()
}

fn words(state: &AppState) -> Vec<String> {
    state.reveal.as_ref().map(|r| r.words.iter().cloned().collect()).unwrap_or_default()
}

// ── The reveal ──────────────────────────────────────────────────────────────

/// The opening screen, lit. The reveal's question is *"is this one of your
/// objects' parse names?"*, asked of Mini-Zork's own objects with no scope walk
/// in it (SQ-1135, SQ-1207).
#[test]
fn the_opening_screen_lights_the_words_the_story_knows() {
    let mut session = minizork();
    let mut state = screen(&mut session);

    let armed = app::reveal::arm(&mut state, &session);
    assert_eq!(armed, Armed::Lit { words: words(&state).len() });

    let lit = words(&state);
    println!("lit: {lit:?}");
    for here in ["mailbox", "house", "door"] {
        assert!(lit.contains(&here.to_string()), "{here:?} is here and must light: {lit:?}");
    }
    // `field` is not in the dictionary at all — the parser answers
    // `[I don't know the word "field".]` — and it is the noun the description
    // opens with. The dictionary IS the test, so this is what it excludes.
    assert!(!lit.contains(&"field".to_string()), "the story has never heard of `field`: {lit:?}");
    // `window` is in the dictionary (Mini-Zork's kitchen window) and is not on
    // this screen. The viewport is still the bound: a reveal lights words the
    // story PRINTED, never the dictionary at large.
    assert!(!lit.contains(&"window".to_string()), "nothing printed it here: {lit:?}");

    // …and it reaches the screen. The lit words are drawn underlined
    // (`transcript_reveal`), over the story's own prose, without moving it.
    let buf = draw(&state);
    let painted = lit_rows(&buf);
    println!("the screen:{}", frame(&buf));
    println!("what lit:{}", painted.iter().map(|(_, s)| format!("\n  |{s}|")).collect::<String>());
    let all: String = painted.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join(" ");
    for here in ["mailbox", "house", "door"] {
        assert!(all.contains(here), "{here:?} is not underlined on screen:{}", frame(&buf));
    }
    assert!(!all.contains("field"), "`field` must not be underlined:{}", frame(&buf));
}

/// **A verb never lights.** The command band answers "what can I do"; this answers
/// "what does the story know about". `open` and `take` are all over Mini-Zork's
/// grammar and its opening prose says `open field` — lighting the verb would
/// leave the prose saying nothing at all.
///
/// The compass words are NOT part of this claim, and never were a filter's to
/// make: Mini-Zork files `west` with the DESC bit, exactly as it files `white`
/// — see `the_reveal_asks_the_objects_not_the_flag_byte`.
#[test]
fn verbs_do_not_light() {
    let mut session = minizork();
    let vocab = <GameSession as Engine>::story_vocabulary(&session).expect("a readable dictionary");
    let mut state = screen(&mut session);
    app::reveal::arm(&mut state, &session);
    let lit = words(&state);
    for verb in ["open", "take"] {
        let r = vocab.roles(verb).expect("in the dictionary");
        assert!(
            !r.noun && !r.adjective,
            "{verb:?} must be filed as a verb and nothing else, or this proves nothing: {r:?}"
        );
        assert!(!lit.contains(&verb.to_string()), "{verb:?} is a verb: {lit:?}");
    }
}

/// **A word for something in another room lights, and that is intended**
/// (SQ-1135).
///
/// After `north`, `There is a small mailbox here.` is still on screen — the
/// player can read it — and the parser now answers `You can't see any mailbox
/// here!`. The word still lights, because the claim the highlight makes is about
/// the STORY's OBJECTS globally: `mailbox` is a real object's parse name,
/// full stop, with no scope walk asking whether that object is HERE. Lighting a
/// word the story has already printed on the player's own screen tells them
/// nothing they were not told.
///
/// This case used to assert the opposite, back when the reveal walked the object
/// tree wherever it could. That is the inversion SQ-1135 removes, and the reason
/// Arthur's crystal reaches a player at all: the description naming it is right
/// there on screen, and a scope-tested reveal lit nothing in it.
#[test]
fn a_word_the_story_knows_lights_wherever_the_thing_is() {
    let mut session = minizork();
    let mut state = screen(&mut session);
    app::reveal::arm(&mut state, &session);
    assert!(words(&state).contains(&"mailbox".to_string()), "lit at West of House");

    session.submit("north");
    for line in session.take_transcript().split('\n') {
        state.push_transcript_kind(line, TranscriptKind::Story);
    }
    draw(&state);
    app::reveal::arm(&mut state, &session);
    let lit = words(&state);
    println!("after `north`, lit: {lit:?}");

    assert!(
        state.transcript.iter().any(|l| l.contains("small mailbox")),
        "the sentence naming the mailbox must still be on screen, or this proves nothing",
    );
    assert!(lit.contains(&"mailbox".to_string()), "the story still knows the word: {lit:?}");
    assert!(lit.contains(&"house".to_string()), "…and the house with it: {lit:?}");
}

// ── Momentary ───────────────────────────────────────────────────────────────

/// One press lights it; the next keystroke, the next turn or the hold puts it
/// out. There is no fourth way and no way to leave it on.
#[test]
fn it_goes_out_on_a_keystroke_on_a_turn_and_on_the_clock() {
    let mut session = minizork();
    let mut state = screen(&mut session);

    // The keystroke path (`main.rs` clears ahead of every dispatch arm).
    app::reveal::arm(&mut state, &session);
    assert!(state.reveal.is_some());
    assert!(app::reveal::clear(&mut state), "a lit reveal goes out");
    assert!(state.reveal.is_none());
    assert!(!app::reveal::clear(&mut state), "…and clearing nothing changes nothing");

    // The turn path. `begin_turn` is what every finished command runs through,
    // including one no key was pressed for (a timed read firing).
    app::reveal::arm(&mut state, &session);
    state.begin_turn();
    assert!(state.reveal.is_none(), "a turn ends the moment the reveal was about");

    // The clock. `expire` is the loop's tick; a reveal whose hold has passed is
    // dropped there and nowhere else, so a player who presses and then does
    // nothing still watches it go out.
    app::reveal::arm(&mut state, &session);
    assert!(!app::reveal::expire(&mut state), "not yet — the hold has not passed");
    state.reveal.as_mut().unwrap().until = std::time::Instant::now();
    assert!(app::reveal::expire(&mut state), "the hold passed");
    assert!(state.reveal.is_none());
}

/// It lives under the Guiding Light's switch, like every other assist — and says
/// so, instead of appearing to be broken.
#[test]
fn with_the_guiding_light_out_it_does_nothing_and_admits_it() {
    let mut session = minizork();
    let mut state = screen(&mut session);
    state.config.guidance = false;

    assert_eq!(app::reveal::arm(&mut state, &session), Armed::GuidanceOff);
    assert!(state.reveal.is_none(), "nothing lights with the light out");

    state.config.guidance = true;
    assert!(matches!(app::reveal::arm(&mut state, &session), Armed::Lit { .. }));
}

/// Before a frame has been drawn there is no viewport to read, and the reveal
/// says that rather than lighting the whole scrollback. This is also the v6
/// RASTER answer: raster's text never passes through the cell wrap cache, so it
/// takes this branch and the feature is honestly absent there rather than
/// silently wrong.
#[test]
fn with_no_drawn_text_there_is_nothing_to_light() {
    let mut session = minizork();
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.guidance = true;
    // The transcript is full; the SCREEN is not, because nothing has drawn yet.
    for line in session.take_transcript().split('\n') {
        state.push_transcript_kind(line, TranscriptKind::Story);
    }
    assert_eq!(app::reveal::arm(&mut state, &session), Armed::NoText);
    assert!(state.reveal.is_none());
}

/// The viewport is the answer to "how far back?", so a reveal only ever knows
/// about words that were on screen when it was lit.
#[test]
fn only_what_is_on_screen_is_considered() {
    let mut session = minizork();
    let mut state = screen(&mut session);
    app::reveal::arm(&mut state, &session);
    let at_opening = words(&state);
    assert!(at_opening.contains(&"mailbox".to_string()));

    // Push enough plain lines to scroll the opening description off a 20-row
    // pane, then draw so the viewport moves with them.
    for _ in 0..40 {
        state.push_transcript_kind("Time passes.", TranscriptKind::Story);
    }
    draw(&state);
    app::reveal::arm(&mut state, &session);
    let scrolled = words(&state);
    println!("scrolled off: {scrolled:?}");
    assert!(
        !scrolled.contains(&"mailbox".to_string()),
        "the mailbox is still in scope and still in the transcript — but not on screen: {scrolled:?}",
    );
}

// ── The claim, and the label on it ──────────────────────────────────────────

/// **The reveal asks Mini-Zork's OBJECTS, not its dictionary's flag byte**
/// (SQ-1207) — and it has to be an object question, because the flag byte
/// cannot settle this one.
///
/// `west`'s flag byte is `0x33`, which sets the same DESC bit `white` and
/// `boarded` carry: nothing in Mini-Zork's *dictionary* distinguishes the
/// compass word from an adjective. A filter built on that bit would have to
/// light both or neither — which is exactly what the old, dictionary-only
/// tier this engine no longer uses did (see `arm`'s `None` arm, still taken by
/// Glulx and Scott today). Asked of the OBJECTS instead, the question has a
/// real answer: `west` is nobody's parse name at all, so it never lights,
/// regardless of what the flag byte says about `white`.
#[test]
fn the_reveal_asks_the_objects_not_the_flag_byte() {
    let mut session = minizork();
    let vocab = <GameSession as Engine>::story_vocabulary(&session).expect("a readable dictionary");
    for w in ["west", "north", "white", "boarded", "mailbox", "the"] {
        println!("{w}: {:?}", vocab.roles(w));
    }
    // The compass word and the colour are indistinguishable in the dictionary —
    // the fact that makes this a meaningful test and not a coincidence.
    let west = vocab.roles("west").expect("in the dictionary");
    let white = vocab.roles("white").expect("in the dictionary");
    assert_eq!(
        (west.noun, west.adjective, west.special),
        (white.noun, white.adjective, white.special),
        "Mini-Zork files `west` exactly as it files `white`, so a flag-byte \
         filter could not tell them apart even if `arm` still used one",
    );

    // Mini-Zork answers for its own objects, so `arm` never falls back to that
    // ambiguous flag byte in the first place — this IS the assertion, not
    // scaffolding for one.
    let set = session.introspect().and_then(|i| i.object_word_set());
    assert!(set.is_some(), "Mini-Zork has a readable object table; the fallback below is untested here");

    let mut state = screen(&mut session);
    app::reveal::arm(&mut state, &session);
    let lit = words(&state);
    // Neither is one of Mini-Zork's objects' parse names — `west` because a
    // compass direction is not an object, `white` because Version 1-3 stores no
    // readable adjective property at all (`ParseNames::detect`'s own doc). Both
    // stay dark, and for a reason that has nothing to do with the flag byte
    // they happen to share.
    for word in ["west", "white"] {
        assert!(!lit.contains(&word.to_string()), "{word:?} is not a Mini-Zork object: {lit:?}");
    }
    // Articles never carry an object's parse name on any engine.
    for buzz in ["the", "a"] {
        assert!(!lit.contains(&buzz.to_string()), "{buzz:?} names no object: {lit:?}");
    }
}

/// The reveal cannot tell "implemented HERE" from "implemented SOMEWHERE" — by
/// design (SQ-1135) — and its claim must say so SOMEWHERE the player meets it.
/// Since SQ-1214 a lit reveal itself says nothing (the lighting is the answer),
/// so the claim lives in the control's description: the canonical wording stays
/// pinned here, and the description must make the weaker claim — the words the
/// story KNOWS — never the stronger one, that the things are here.
#[test]
fn the_reveal_admits_what_it_cannot_tell_apart() {
    assert!(
        app::reveal::CAVEAT.contains("not necessarily"),
        "the canonical wording has to say what it cannot promise: {:?}",
        app::reveal::CAVEAT,
    );
    let desc = app::slash::COMMANDS
        .iter()
        .find(|c| c.name == "reveal-words")
        .expect("the reveal command exists")
        .description;
    assert!(
        desc.contains("knows"),
        "the description carries the weak claim now that no status line does: {desc:?}"
    );
    assert!(
        !desc.contains("things that are here"),
        "and must not promise presence: {desc:?}"
    );
}

// ── The contract, on a real story with real adjectives (SQ-1207) ───────────

/// Retail Zork I, release 52 (Version 5) — not Mini-Zork, because a Version
/// 1-3 story keeps no readable adjective property at all
/// (`zvm::objects::ParseNames::detect`'s own doc: adjectives are answerable
/// "only from V4"), so Mini-Zork cannot prove the noun-AND-adjective half of
/// this contract no matter how the reveal classifies.
///
/// Gitignored (CLAUDE.md's `stories/`), so this skips vacuously without it —
/// see [`a_noun_and_its_adjective_light_while_articles_and_verbs_do_not`].
fn zork1_invclues() -> Option<GameSession> {
    let path = fixture_path("zork1-invclues-r52-s871125.z5");
    let bytes = std::fs::read(&path).ok()?;
    Some(
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
            .expect("a Version 5 story should load and boot"),
    )
}

/// **The contract SQ-1207 exists for.** West of House's opening line names a
/// house and a door with real adjectives (`white`, `boarded`) sitting beside
/// articles, prepositions and a verb that are printed just as plainly — the
/// reveal has to tell a thing from the words around it.
///
/// Falsified by reverting `arm`'s `Some(set)` arm to a bare dictionary-acceptance
/// test (SQ-1207's before-state): `the`, `a`, `of`, `an`, `open`, `standing`,
/// `with`, `in` and `here` are all real Zork I dictionary entries and would
/// light right alongside `house`, exactly the bug this quest was filed against.
/// (`standing` and `here` are just as printed; the point is that none of these
/// nine is any object's parse name.)
#[test]
fn a_noun_and_its_adjective_light_while_articles_and_verbs_do_not() {
    let Some(mut session) = zork1_invclues() else {
        eprintln!("SKIP: stories/zork1-invclues-r52-s871125.z5 not present");
        return;
    };
    let mut state = screen(&mut session);
    let armed = app::reveal::arm(&mut state, &session);
    assert_eq!(armed, Armed::Lit { words: words(&state).len() });
    let lit = words(&state);
    println!("lit: {lit:?}");
    assert!(
        state.transcript.iter().any(|l| l.contains("white house")),
        "the specimen text must actually be on screen, or this proves nothing: {:?}",
        state.transcript,
    );

    // Nouns, and the adjectives that describe them — all real Zork I object
    // parse names.
    for thing in ["house", "white", "door", "boarded", "mailbox", "small", "front"] {
        assert!(lit.contains(&thing.to_string()), "{thing:?} names or describes a real object: {lit:?}");
    }
    // Articles, a preposition, a verb — on the same two lines, none of them a
    // parse name of anything.
    for not_a_thing in ["the", "a", "of", "an", "open", "standing", "with", "in", "here"] {
        assert!(!lit.contains(&not_a_thing.to_string()), "{not_a_thing:?} names nothing: {lit:?}");
    }
}

// ── The same contract on Glulx (SQ-1210) ────────────────────────────────────
//
// The bug this section pins: with no object-word answer on Glulx, `arm` fell
// back to the dictionary's flag bits, and an Inform dictionary files `a`, `an`
// and `the` in noun position — so the reveal lit the articles of every Glulx
// game. `Engine::object_word_set` now answers from the story's own objects
// (`gvm::objects::ParseNames`), and these cases hold it to the Zork I
// specimen's exact contract.
//
// Specimens (both gitignored, skip vacuously; oracle: the game's own parser
// under `gvm-cli`, 2026-09-01):
//
// | fixture | release | turns in | what it shows |
// |---|---|---|---|
// | `stories/Dr Ludwig and the Devil.gblorb` | r2/s250306, I7 6M62 | 3 intro keys, 0 commands | the SQ-1210 symptom carrier |
// | `stories/King_of_Shreds_and_Patches.gblorb` | whatever it answers | boot | fail-safe: answer truly or refuse honestly |

/// Boot a Glulx story from `stories/` (bare image or Blorb), or skip.
fn glulx_session(name: &str) -> Option<app::glulx_session::GlulxSession> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let image = if blorb::Blorb::is_blorb(&bytes) {
        let b = blorb::Blorb::parse(bytes).expect("story parses as a Blorb");
        match b.executable() {
            Ok((blorb::ExecKind::Glulx, data)) => data.to_vec(),
            _ => return None,
        }
    } else {
        bytes
    };
    Some(
        app::glulx_session::GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[])
            .expect("the story boots"),
    )
}

/// Dr Ludwig's opening screen: tap through the intro's keypress pauses to the
/// first line prompt (three of them; the bound is just a hang guard) and hand
/// back everything printed on the way. 0 commands typed.
fn dr_ludwig_opening() -> Option<(app::glulx_session::GlulxSession, String)> {
    let mut session = glulx_session("Dr Ludwig and the Devil.gblorb")?;
    let mut text = session.take_transcript();
    for _ in 0..8 {
        if session.pending_input() != app::session::InputKind::Char {
            break;
        }
        if let Some(turn) = session.submit_key(app::engine::KeyInput::Enter) {
            text.push('\n');
            text.push_str(&turn.transcript);
        }
    }
    assert_eq!(
        session.pending_input(),
        app::session::InputKind::Line,
        "the intro ends at the game's own command prompt, or the frame is not the one measured"
    );
    Some((session, text))
}

/// **SQ-1210's contract, on the story it was filed against.** The Laboratory
/// description names a desk, an operating table, a grandfather clock, a
/// staircase, a summoning circle drawn in chalk, and the Devil — every one
/// confirmed against the game's own parser (`x desk` → the Grand Grimoire,
/// `x grandfather` → "a few minutes past six", `x summoning` → the circle,
/// `x the` / `x an` → "There was no such thing in sight!"). The articles and
/// the hint bar's verbs (`type`, `ask`) sit in the same screenful and must
/// stay dark — under the pre-SQ-1210 dictionary fallback they lit.
///
/// Falsified by forcing `GlulxSession::object_word_set` to `None` (the
/// pre-quest state): `the`, `a` and `an` light and this case fails on them.
#[test]
fn glulx_nouns_and_adjectives_light_while_articles_and_verbs_do_not() {
    let Some((session, text)) = dr_ludwig_opening() else { return };
    // The seam must ANSWER here, or everything below would be exercising the
    // dictionary fallback — the exact bug this quest closed.
    assert!(
        session.object_word_set().is_some(),
        "Dr Ludwig is Inform-compiled Glulx; ParseNames must read its object list"
    );

    let mut state = screen_of(&text);
    let armed = app::reveal::arm(&mut state, &session);
    assert_eq!(armed, Armed::Lit { words: words(&state).len() });
    let lit = words(&state);
    println!("lit: {lit:?}");
    assert!(
        state.transcript.iter().any(|l| l.contains("summoning circle")),
        "the specimen text must actually be on screen, or this proves nothing: {:?}",
        state.transcript,
    );

    // Things, and the adjectives that describe them — all parser-confirmed
    // parse names of Dr Ludwig's objects.
    for thing in
        ["devil", "circle", "chalk", "summoning", "desk", "table", "clock", "grandfather", "staircase"]
    {
        assert!(lit.contains(&thing.to_string()), "{thing:?} names or describes a real object: {lit:?}");
    }
    // Articles and two of the hint bar's verbs, printed in the same screenful,
    // none of them any object's parse name.
    for not_a_thing in ["the", "a", "an", "type", "ask"] {
        assert!(!lit.contains(&not_a_thing.to_string()), "{not_a_thing:?} names nothing: {lit:?}");
    }
}

/// The Glulx set is cached for a turn and rebuilt after the VM runs — the same
/// SQ-1176 discipline `GameSession` proves in `session.rs`'s unit test, held
/// here at the adapter's seam because the invalidation lives in its drive
/// paths.
#[test]
fn the_glulx_object_word_set_is_cached_for_a_turn_and_dropped_when_the_vm_runs() {
    use std::sync::Arc;
    let Some((mut session, _)) = dr_ludwig_opening() else { return };

    let first = session.object_word_set().expect("Dr Ludwig answers");
    assert!(
        first.contains("devil") && !first.contains("the"),
        "the set answers as the story's own objects answer"
    );
    let again = session.object_word_set().expect("still answerable");
    assert!(Arc::ptr_eq(&first, &again), "within a turn, one build serves every caller");

    session.submit("look");
    let fresh = session.object_word_set().expect("still answerable");
    assert!(
        !Arc::ptr_eq(&first, &fresh),
        "after a turn the set is rebuilt from live RAM, not served stale"
    );
}

/// Fail-safe on a second, unrelated Glulx story: either the object list reads
/// and the set carries no article, or detection refuses and the reveal keeps
/// its documented dictionary fallback. Refusal is a PASS — a wrong set would
/// silently unlight real nouns, which is worse than the bug (SQ-1210's prime
/// rule). What this case forbids is the middle ground: a set that "answers"
/// with words no object owns.
#[test]
fn a_second_glulx_story_answers_truly_or_refuses_honestly() {
    let Some(mut session) = glulx_session("King_of_Shreds_and_Patches.gblorb") else { return };
    match session.object_word_set() {
        Some(set) => {
            for article in ["the", "a", "an"] {
                assert!(!set.contains(article), "{article:?} can be no object's parse name");
            }
            println!("King of Shreds: object list read");
        }
        None => {
            // The documented fallback must still arm the reveal off the
            // dictionary tier rather than going dark or panicking.
            let mut state = screen(&mut session);
            let armed = app::reveal::arm(&mut state, &session);
            println!("King of Shreds: ParseNames refused; fallback armed as {armed:?}");
            assert!(
                matches!(armed, Armed::Lit { .. } | Armed::Nothing),
                "with no object answer the dictionary tier still runs: {armed:?}"
            );
        }
    }
}

