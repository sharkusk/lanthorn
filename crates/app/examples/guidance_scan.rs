//! Scan a corpus of stories through Lanthorn's Guiding Light (SQ-1206).
//!
//! ```sh
//! cargo run -p app --example guidance_scan                      # stories/ + unit_tests/
//! cargo run -p app --example guidance_scan -- --only curses.z5,vespers.z8
//! cargo run -p app --example guidance_scan -- --corpus stories --json
//! ```
//!
//! For every story it can read it boots the real engine, clears whatever gate
//! stands before the first prompt, and types a fixed battery of
//! player-plausible-but-absent verbs at a noun the story has just printed —
//! driving `vocab::offer_vocabulary` and the shadow probe exactly as
//! `tests/suites/vocabulary_vetting.rs` does. One row per story: the file, the
//! engine and format, the parser family, and every offer the light made with
//! its vetting verdict (`try instead` = vetted in a silent copy of the game,
//! `this story knows` = the dictionary claim the offer can still support).
//!
//! **It is bounded on purpose.** Per story: at most 12 keypresses to clear the
//! opening gate, at most 40 turns, and a 60-second wall clock, after which the
//! row is marked `capped`. Blue Lacuna and King of Shreds and Patches open on a
//! menu that wants a specific letter and never reach a prompt at all; they cost
//! their cap and nothing more.
//!
//! The battery is typed in the opening room and again one room on. A word is
//! offered once per session, so the second room usually adds nothing — what it
//! is there to catch is a candidate the vetting drops in one room and keeps in
//! the next, which is the whole point of the probe.
//!
//! Nothing is written and nothing is committed — the output is the artefact.
//! When the corpus grows, this is the second of the three steps in
//! `crates/verb-synonyms-gen/README.md`: the harvest diff says which verbs the
//! tables have never seen, and this says whether the offers built on them are
//! any good.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::engine::{Engine, KeyInput};
use app::probe::ShadowRecipe;
use app::session::InputKind;
use app::state::{AppState, TranscriptKind};

/// Keypresses spent clearing the gate before the first line prompt.
const MAX_KEYS: usize = 12;
/// Turns typed into one story, battery included.
const MAX_TURNS: usize = 40;
/// Wall clock for one story, boot included.
const TIME_CAP: Duration = Duration::from_secs(60);

/// Verbs a player might reasonably type that most parsers do not hold, plus one
/// whose only near neighbour is a different verb entirely (`hasten`/`fasten`).
const BATTERY: [&str; 7] =
    ["inspect", "illuminate", "obtain", "shove", "gaze at", "peruse", "hasten"];

fn main() -> std::process::ExitCode {
    let mut corpora: Vec<PathBuf> = Vec::new();
    let mut only: Vec<String> = Vec::new();
    let mut json = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> String {
            args.get(i + 1).cloned().unwrap_or_else(|| {
                eprintln!("guidance_scan: `{}` needs a value", args[i]);
                std::process::exit(2);
            })
        };
        match args[i].as_str() {
            "--corpus" => {
                corpora.push(PathBuf::from(need(i)));
                i += 1;
            }
            "--only" => {
                only.extend(need(i).split(',').map(|s| s.trim().to_string()));
                i += 1;
            }
            "--json" => json = true,
            "--help" | "-h" => {
                eprintln!(
                    "usage: guidance_scan [--corpus DIR]... [--only a,b] [--json]\n\
                     defaults to --corpus stories --corpus unit_tests"
                );
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("guidance_scan: unexpected argument `{other}`");
                return std::process::ExitCode::FAILURE;
            }
        }
        i += 1;
    }
    if corpora.is_empty() {
        corpora = vec![PathBuf::from("stories"), PathBuf::from("unit_tests")];
    }

    let mut rows: Vec<Row> = Vec::new();
    for dir in &corpora {
        let Ok(entries) = std::fs::read_dir(dir) else {
            eprintln!("SKIP: no corpus directory at {} (gitignored?)", dir.display());
            continue;
        };
        let mut paths: Vec<PathBuf> =
            entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect();
        paths.sort();
        for p in paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if !only.is_empty() && !only.iter().any(|o| o == &name) {
                continue;
            }
            if let Some(row) = scan(&p, &name) {
                rows.push(row);
            }
        }
    }

    if rows.is_empty() {
        println!("nothing to scan (no readable story in the corpus, or --only matched none)");
        return std::process::ExitCode::SUCCESS;
    }
    if json {
        print_json(&rows);
    } else {
        print_table(&rows);
    }
    std::process::ExitCode::SUCCESS
}

/// One story's answer.
struct Row {
    file: String,
    /// `Z-code v8`, `Glulx 3.1.2`, `Scott Adams`.
    format: String,
    /// `I6`, `I7`, or `other` — see [`family`].
    family: &'static str,
    /// Verb entries the story's grammar declares, `None` when it cannot be read.
    verbs: Option<usize>,
    /// Where the battery was typed, when the engine could say.
    rooms: Vec<String>,
    /// `(command, offer, vetted)`, in the order the light spoke.
    offers: Vec<(String, String, bool)>,
    /// What stood between boot and the first prompt, if anything.
    note: String,
}

fn scan(path: &Path, name: &str) -> Option<Row> {
    let deadline = Instant::now() + TIME_CAP;
    let bytes = std::fs::read(path).ok()?;
    let loaded = app::hints::extract_story(bytes.clone()).ok()?;
    let format = format_of(&loaded);
    let mut engine = boot(loaded)?;

    let mut note = String::new();
    let vocab = engine.story_vocabulary();
    if vocab.is_none() {
        note.push_str("grammar unreadable; ");
    }
    let family = family(vocab.as_ref());
    let verbs = vocab.as_ref().map(|v| v.verbs().len());

    let mut state = AppState::default();
    state.assist_preamble_shown = true;
    state.probe.arm(ShadowRecipe {
        story_bytes: Arc::new(bytes),
        store: PathBuf::new(),
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    });

    // Clear whatever gate stands before the first line prompt.
    let mut keys = 0;
    while engine.pending_input() == InputKind::Char && keys < MAX_KEYS {
        if Instant::now() >= deadline {
            break;
        }
        let _ = engine.submit_key(KeyInput::Char(' '));
        keys += 1;
    }
    if keys > 0 {
        note.push_str(&format!("{keys} keypress gate; "));
    }
    if engine.pending_input() == InputKind::Char {
        note.push_str("never reached a prompt (menu?); ");
    }

    let mut play = Play { state, engine, last: String::new(), turns: 0, deadline };
    play.turn("look");
    let mut rooms = Vec::new();
    let mut offers = Vec::new();
    for round in 0..2 {
        if play.spent() {
            note.push_str("capped; ");
            break;
        }
        if round == 1 {
            // One step on, so the second battery is asked somewhere else.
            let before = play.engine.current_location().map(|l| l.number);
            for dir in ["north", "east", "west", "south", "out"] {
                play.turn(dir);
                if play.engine.current_location().map(|l| l.number) != before {
                    break;
                }
                if play.spent() {
                    break;
                }
            }
            play.turn("look");
        }
        let room = play.engine.current_location().map(|l| l.name).unwrap_or_default();
        if !room.is_empty() && !rooms.contains(&room) {
            rooms.push(room);
        }
        let noun = noun_here(&play, &vocab);
        for verb in BATTERY {
            if play.spent() {
                break;
            }
            let cmd = if verb == "hasten" {
                "hasten north".to_string()
            } else {
                format!("{verb} {noun}")
            };
            for line in play.turn(&cmd) {
                let vetted = line.starts_with(app::vocab::LEAD_VETTED);
                let body = line
                    .trim_start_matches(app::vocab::LEAD_VETTED)
                    .trim_start_matches(app::vocab::LEAD_DICTIONARY)
                    .to_string();
                offers.push((cmd.clone(), body, vetted));
            }
        }
    }
    Some(Row { file: name.to_string(), format, family, verbs, rooms, offers, note })
}

fn boot(loaded: app::hints::LoadedStory) -> Option<Box<dyn Engine>> {
    match loaded {
        app::hints::LoadedStory::ZCode(b) => {
            let mut s = app::session::GameSession::new_with_trace(
                b,
                true,
                false,
                None,
                false,
                Vec::new(),
                None,
                None,
                Some((25, 80)),
            )
            .ok()?;
            s.set_strip_prompt(true);
            Some(Box::new(s))
        }
        app::hints::LoadedStory::Glulx(b) => {
            let s =
                app::glulx_session::GlulxSession::new(b, 80, 24, true, false, false, (1, 1), None, &[])
                    .ok()?;
            Some(Box::new(s))
        }
        app::hints::LoadedStory::Scott(b) => {
            Some(Box::new(app::scott_session::ScottSession::new_with_trace(b, None, false, None).ok()?))
        }
    }
}

fn format_of(loaded: &app::hints::LoadedStory) -> String {
    let b = loaded.bytes();
    match loaded {
        app::hints::LoadedStory::ZCode(_) => format!("Z-code v{}", b.first().copied().unwrap_or(0)),
        app::hints::LoadedStory::Glulx(_) => {
            let v = b.get(4..8).map_or(0u32, |s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]));
            format!("Glulx {}.{}.{}", v >> 16, (v >> 8) & 0xff, v & 0xff)
        }
        app::hints::LoadedStory::Scott(_) => "Scott Adams".to_string(),
    }
}

/// Inform 6 or Inform 7, from the library verbs that separate them.
///
/// `noscript` and `unscript` are Inform 6 library verbs that the Inform 7
/// Standard Rules do not declare; measured across this corpus they are present
/// in every I6 game and in none of the I7 ones. A story with no readable
/// grammar, or one that is not Inform at all, answers `other`.
fn family(vocab: Option<&app::vocab::StoryVocabulary>) -> &'static str {
    let Some(v) = vocab else { return "other" };
    if v.verb_named("noscript").is_some() || v.verb_named("unscript").is_some() {
        "I6"
    } else if v.verb_named("examine").is_some() {
        "I7"
    } else {
        "other"
    }
}

/// A noun this story has just printed that its own object list answers to —
/// the battery needs one, and a word the game never heard makes every candidate
/// fail for the wrong reason.
fn noun_here(play: &Play, vocab: &Option<app::vocab::StoryVocabulary>) -> String {
    if let Some(scope) = app::vocab::objects_in_scope(&*play.engine) {
        for o in &scope {
            for w in app::vocab::typeable_words(o, vocab.as_ref()) {
                if w.len() >= 3 {
                    return w;
                }
            }
        }
    }
    let set = play.engine.object_word_set();
    for tok in play.last.split(|c: char| !c.is_ascii_alphabetic()) {
        let t = tok.to_lowercase();
        if t.len() < 4 {
            continue;
        }
        let known = set.as_ref().is_some_and(|s| s.contains(&t))
            || vocab.as_ref().is_some_and(|v| v.roles(&t).is_some_and(|r| r.noun));
        if known {
            return t;
        }
    }
    "door".to_string()
}

struct Play {
    state: AppState,
    engine: Box<dyn Engine>,
    last: String,
    turns: usize,
    deadline: Instant,
}

impl Play {
    fn spent(&self) -> bool {
        self.turns >= MAX_TURNS || Instant::now() >= self.deadline
    }

    /// One turn the way `finish_command_turn` takes it: the game's reply, then
    /// the offer, then the beat in which the shadow answers. Returns the assist
    /// lines this turn added.
    fn turn(&mut self, cmd: &str) -> Vec<String> {
        if self.spent() {
            return Vec::new();
        }
        self.turns += 1;
        let before = assists(&self.state).len();
        let r = self.engine.submit(cmd);
        self.last = r.transcript.clone();
        self.state.push_transcript_kind(&format!("> {cmd}"), TranscriptKind::Input);
        self.state.push_transcript_kind(r.transcript.trim_end_matches('\n'), TranscriptKind::Story);
        let printed = !r.transcript.trim().is_empty();
        app::vocab::offer_vocabulary(&mut self.state, &*self.engine, cmd, printed);
        app::vocab::settle_vocabulary_offer(&mut self.state);
        assists(&self.state).into_iter().skip(before).collect()
    }
}

fn assists(state: &AppState) -> Vec<String> {
    state
        .transcript
        .iter()
        .zip(&state.transcript_kinds)
        .filter(|(_, k)| **k == TranscriptKind::Assist)
        .map(|(l, _)| l.clone())
        .collect()
}

fn print_table(rows: &[Row]) {
    println!(
        "{:<42} {:<13} {:<6} {:>5} {:>7}  notes",
        "story", "format", "family", "verbs", "vet/off"
    );
    for r in rows {
        let ok = r.offers.iter().filter(|(_, _, v)| *v).count();
        println!(
            "{:<42} {:<13} {:<6} {:>5} {:>3}/{:<3}  {}{}",
            trunc(&r.file, 42),
            r.format,
            r.family,
            r.verbs.map(|n| n.to_string()).unwrap_or_else(|| "-".into()),
            ok,
            r.offers.len(),
            r.note,
            if r.rooms.is_empty() { String::new() } else { format!("rooms: {}", r.rooms.join(" → ")) }
        );
        for (cmd, offer, v) in &r.offers {
            println!("      {:<24} {} {}", cmd, if *v { "vetted  " } else { "unvetted" }, offer);
        }
    }
    let total: usize = rows.iter().map(|r| r.offers.len()).sum();
    let vetted: usize = rows.iter().map(|r| r.offers.iter().filter(|(_, _, v)| *v).count()).sum();
    println!("\n{} stories, {total} offers, {vetted} of them vetted", rows.len());
}

fn print_json(rows: &[Row]) {
    println!("[");
    for (i, r) in rows.iter().enumerate() {
        println!("  {{");
        println!("    \"file\": \"{}\",", esc(&r.file));
        println!("    \"format\": \"{}\",", esc(&r.format));
        println!("    \"family\": \"{}\",", r.family);
        match r.verbs {
            Some(n) => println!("    \"verbs\": {n},"),
            None => println!("    \"verbs\": null,"),
        }
        println!(
            "    \"rooms\": [{}],",
            r.rooms.iter().map(|s| format!("\"{}\"", esc(s))).collect::<Vec<_>>().join(", ")
        );
        println!("    \"note\": \"{}\",", esc(r.note.trim_end_matches([';', ' '])));
        println!("    \"offers\": [");
        for (j, (cmd, offer, v)) in r.offers.iter().enumerate() {
            println!(
                "      {{ \"command\": \"{}\", \"offer\": \"{}\", \"vetted\": {} }}{}",
                esc(cmd),
                esc(offer),
                v,
                if j + 1 == r.offers.len() { "" } else { "," }
            );
        }
        println!("    ]");
        println!("  }}{}", if i + 1 == rows.len() { "" } else { "," });
    }
    println!("]");
}

fn esc(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' | '\r' | '\t' => vec![' '],
            c => vec![c],
        })
        .collect()
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}
