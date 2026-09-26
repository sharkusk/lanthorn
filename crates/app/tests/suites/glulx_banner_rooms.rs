//! An Inform 7 game's front matter must not mint rooms (SQ-0732).
//!
//! Glulx room detection reads the `Subheader`-styled line a game prints above a room
//! description. A title page prints its title, its author and its act list the same way,
//! and a prologue prints its own strapline — so THE BAT handed the automap three rooms
//! before play began: an act list, a newspaper masthead, and only then the real one.
//!
//! The discriminator is the shape of the page, not the words on it. Inform joins a room
//! heading to the description below it, and hands the player the command prompt when the
//! turn is over. A banner stands alone above a blank line, on a page that ends by asking
//! for a keypress. Both halves matter, and the second test here is why: Adventure in
//! `superbrief` prints a room heading, a blank line and nothing but its object list, so
//! "detached from the prose below it" alone would throw real rooms away.

use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

fn glulx_image(name: &str) -> Option<Vec<u8>> {
    let p = fixture_path(name);
    let Ok(bytes) = std::fs::read(&p) else {
        eprintln!("SKIP: gitignored story missing at {}", p.display());
        return None;
    };
    if !blorb::Blorb::is_blorb(&bytes) {
        return Some(bytes);
    }
    let b = blorb::Blorb::parse(bytes).expect("story parses as a Blorb");
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Some(data.to_vec()),
        _ => None,
    }
}

fn boot(name: &str) -> Option<GlulxSession> {
    let image = glulx_image(name)?;
    Some(
        GlulxSession::new(image, 80, 24, true, false, false, (1.0, 1.0), None, &[])
            .expect("the story boots"),
    )
}

#[test]
fn the_bats_title_page_and_prologue_are_not_rooms() {
    let Some(mut s) = boot("THE BAT.gblorb.blorb") else { return };

    // Every distinct room the game reports from boot up to and including the turn that
    // finally puts the player at a command prompt in a room.
    let mut seen: Vec<String> = Vec::new();
    let note = |loc: Option<app::engine::LocationInfo>, seen: &mut Vec<String>| {
        if let Some(r) = loc {
            if !seen.contains(&r.name) {
                seen.push(r.name);
            }
        }
    };
    note(s.current_location(), &mut seen);

    // THE BAT opens by asking whether you have played IF before, then shows its title
    // page and its prologue — each a "press any key to continue" page — before the game
    // proper starts in the Master's Bedroom.
    let mut t = Engine::submit(&mut s, "yes");
    note(t.location.clone(), &mut seen);
    for _ in 0..8 {
        if s.pending_input() != InputKind::Char {
            break;
        }
        let Some(next) = s.submit_key(KeyInput::Enter) else { break };
        t = next;
        note(t.location.clone(), &mut seen);
    }

    assert_eq!(
        s.pending_input(),
        InputKind::Line,
        "precondition: the intro pages are done and the game wants a command"
    );
    assert_eq!(
        seen,
        vec!["Master's Bedroom".to_string()],
        "the only room THE BAT has at its first command prompt is the Master's Bedroom; \
         its act list and its prologue's newspaper strapline are `Subheader` banners, not \
         rooms the player can stand in"
    );
}

#[test]
fn cragnes_warning_pages_are_not_rooms() {
    // cragne Manor opens on two front-matter pages -- CONTENT WARNING and CONCEPT
    // WARNING -- each an own-line `Subheader` above a blank line, and each ending by
    // reading a LINE ("Please type yes or no."), not a keypress. So the shape test and
    // the keypress test both pass them, and both became rooms. Neither page ever prints
    // the parser's command prompt, because the game reads the answer itself: only a
    // completed turn hands back a ">". (SQ-0733)
    let Some(mut s) = boot("cragne.gblorb") else { return };

    let mut seen: Vec<String> = Vec::new();
    let note = |loc: Option<app::engine::LocationInfo>, seen: &mut Vec<String>| {
        if let Some(r) = loc {
            if !seen.contains(&r.name) {
                seen.push(r.name);
            }
        }
    };
    note(s.current_location(), &mut seen);

    // Boot ends on a keypress; then "yes" to each warning page, then a "press any key
    // to begin" page, and only then the first real room.
    for _ in 0..6 {
        let t = match s.pending_input() {
            InputKind::Char => match s.submit_key(KeyInput::Enter) {
                Some(t) => t,
                None => break,
            },
            InputKind::Line => Engine::submit(&mut s, "yes"),
            _ => break,
        };
        note(t.location, &mut seen);
        if !seen.is_empty() {
            break;
        }
    }

    assert_eq!(
        s.pending_input(),
        InputKind::Line,
        "precondition: the warning pages are answered and the game wants a command"
    );
    assert_eq!(
        seen,
        vec!["Railway Platform (Naomi Hinchen)".to_string()],
        "cragne's first room is the Railway Platform; CONTENT WARNING and CONCEPT \
         WARNING are pages the game reads an answer from, not places on the map"
    );
}

#[test]
fn a_superbrief_room_with_only_an_object_list_is_still_a_room() {
    // Adventure in `superbrief` prints "Inside Building", a blank line, and then only the
    // things lying about — the same detached shape as a banner. It is a room because the
    // turn ends at the command prompt, which a "press any key" page never does.
    let Some(mut s) = boot("advent.blb") else { return };
    let start = s.current_location().expect("Adventure boots into a room");
    assert_eq!(start.name, "At End Of Road", "precondition: Adventure's opening room");

    Engine::submit(&mut s, "superbrief");
    let inside = Engine::submit(&mut s, "in").location.expect("`in` reports a room");
    assert_eq!(
        inside.name, "Inside Building",
        "a superbrief room heading followed by a blank line and an object list is a room"
    );

    // And a superbrief room with nothing in it at all — heading, blank line, prompt.
    let back = Engine::submit(&mut s, "out").location.expect("`out` reports a room");
    assert_eq!(back.name, "At End Of Road");
    assert_eq!(back.number, start.number, "and it is the same node we started on");
}
