//! Corpus sweep: room/player/inventory detection for every Z-machine story
//! (SQ-1259).
//!
//! ```sh
//! cargo run -p lanthorn --example location_scan                      # stories/
//! cargo run -p lanthorn --example location_scan -- --only LostPig.z8,anchor.z8
//! cargo run -p lanthorn --example location_scan -- --corpus stories --json
//! ```
//!
//! For every Z-machine story under `--corpus` (default `stories/`) it boots
//! headless the way the real-game test suites do
//! (`app::session::GameSession::new_with_trace`, a 25x80 screen), submits
//! `""` then `look` (draining a further `Char` gate with a blank keypress if
//! the game is still asking a question in between — checked via
//! `pending_input()`), and prints one row:
//!
//! `file | version | room id | room name | LocationMethod | player obj |
//! player short name | inventory count | exit convention | inventory names
//! (first 4)`, with a second line naming the opening room's own declared
//! exits when the derivation (`zvm::world::WorldModel::declared_exit`,
//! SQ-1257/SQ-1260) found anything to say about it.
//!
//! The room and player columns come straight off the booted machine via
//! `zvm::location::detect_location` and `zvm::location::find_player_object`;
//! the inventory comes from the session's own `Introspect::contents(player)`
//! — the same source the inventory panel reads.
//!
//! A story that panics or fails to boot gets a row saying so instead of
//! aborting the sweep (each story's work runs under `catch_unwind`).
//! Non-Z-machine files — Glulx, Scott Adams, disk images and blorbs
//! `app::hints::extract_story` cannot read as Z-code — are skipped without a
//! row, exactly as `guidance_scan` skips whatever it cannot boot.
//!
//! `--only a,b` filters by filename, as in `guidance_scan`. `--json` emits
//! one object per row instead of the table.
//!
//! **Run this before and after any change to `zvm::location`'s heuristics**
//! (see `docs/internals/location-heuristics.md`) and diff the two outputs —
//! a rule that looks right on the story you were fixing can silently rewire
//! another's room or player detection.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use app::engine::Engine;
use app::session::{GameSession, InputKind};

fn main() -> std::process::ExitCode {
    let mut corpora: Vec<PathBuf> = Vec::new();
    let mut only: Vec<String> = Vec::new();
    let mut json = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> String {
            args.get(i + 1).cloned().unwrap_or_else(|| {
                eprintln!("location_scan: `{}` needs a value", args[i]);
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
                    "usage: location_scan [--corpus DIR]... [--only a,b] [--json]\n\
                     defaults to --corpus stories"
                );
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("location_scan: unexpected argument `{other}`");
                return std::process::ExitCode::FAILURE;
            }
        }
        i += 1;
    }
    if corpora.is_empty() {
        corpora = vec![PathBuf::from("stories")];
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
            if let Some(row) = scan_one(&p, &name) {
                rows.push(row);
            }
        }
    }

    if rows.is_empty() {
        println!("nothing to scan (no Z-machine story in the corpus, or --only matched none)");
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
    version: u8,
    outcome: Outcome,
}

enum Outcome {
    Ok {
        room_id: Option<u16>,
        room_name: Option<String>,
        method: Option<String>,
        player_id: Option<u16>,
        player_name: Option<String>,
        inventory: Vec<String>,
        /// SQ-1260: which room-exit convention `zvm::world::WorldModel` found on
        /// this story — `"inform"` (`door_dir`), `"zil"` (the `DIRECTIONS`
        /// dictionary-flag convention), or `"none"`.
        exit_convention: &'static str,
        /// The opening room's own declared exits, one `DIR:answer` token per
        /// direction the derivation says anything OTHER than `Unknown`/`Absent`
        /// for (those two are the overwhelming majority on any story with no
        /// convention, or on a room most of whose directions are simply
        /// unset — printing them would drown the row in noise for no signal).
        declared_exits: String,
    },
    Failed(String),
}

/// Read the story, skip anything that is not a Z-machine file, and run it
/// under `catch_unwind` so one bad story cannot take the sweep down with it.
fn scan_one(path: &Path, name: &str) -> Option<Row> {
    let bytes = std::fs::read(path).ok()?;
    let loaded = app::hints::extract_story(bytes).ok()?;
    let app::hints::LoadedStory::ZCode(zbytes) = loaded else {
        return None; // Glulx / Scott Adams — a different engine entirely.
    };
    let version = zbytes.first().copied().unwrap_or(0);
    if !(3..=8).contains(&version) {
        return None; // not a readable Z-machine header
    }
    let outcome = match std::panic::catch_unwind(AssertUnwindSafe(|| run_story(zbytes))) {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => Outcome::Failed(e),
        Err(payload) => Outcome::Failed(panic_message(&payload)),
    };
    Some(Row { file: name.to_string(), version, outcome })
}

/// Boot, clear the opening gate, play `""` then `look`, and read the three
/// signals off the resulting machine/session.
fn run_story(bytes: Vec<u8>) -> Result<Outcome, String> {
    let mut session = GameSession::new_with_trace(
        bytes,
        true,
        false,
        None,
        false,
        Vec::new(),
        None,
        None,
        Some((25, 80)),
    )
    .map_err(|e| format!("boot failed: {e:?}"))?;

    drain_char_gate(&mut session);
    if session.pending_input() != InputKind::Line {
        return Err("never reached a line prompt".to_string());
    }
    let _ = session.submit("");
    drain_char_gate(&mut session); // still asking a question? clear it before "look".
    let _ = session.submit("look");

    let loc = zvm::location::detect_location(&session.machine);
    let (room_id, room_name, method) = match &loc {
        Some(l) => (
            l.object().map(|o| o.number),
            l.object().map(|o| o.name.clone()),
            Some(format!("{:?}", l.method())),
        ),
        None => (None, None, None),
    };

    let player_id = zvm::location::find_player_object(&session.machine);
    let player_name = player_id.map(|p| zvm::objects::short_name(&session.machine.mem, p));
    let inventory: Vec<String> = player_id
        .and_then(|p| session.introspect().map(|i| i.contents(p)))
        .map(|items| items.iter().filter_map(|o| o.display_name()).collect())
        .unwrap_or_default();

    // SQ-1260: which room-exit convention this story exhibits, and what the
    // opening room's own compiled exit table says — read the same way
    // `crates/app/src/session.rs`'s `declared_exit` does, straight off the
    // booted machine.
    let model = zvm::world::WorldModel::discover_at_boot(&session.machine);
    let exit_convention = if model.exit_props.iter().any(Option::is_some) {
        "inform"
    } else if model.zil_exit_props.iter().any(Option::is_some) {
        "zil"
    } else {
        "none"
    };
    let declared_exits = room_id
        .map(|room| {
            zvm::world::Compass::ALL
                .iter()
                .filter_map(|&dir| {
                    let d = model.declared_exit(&session.machine.mem, room, dir);
                    (!matches!(d, zvm::world::DeclaredExit::Unknown | zvm::world::DeclaredExit::Absent))
                        .then(|| format!("{dir:?}:{d:?}"))
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    Ok(Outcome::Ok { room_id, room_name, method, player_id, player_name, inventory, exit_convention, declared_exits })
}

/// Clear a `Char` prompt (an intro splash, or the game still asking a
/// question) with blank keypresses, up to a small cap.
fn drain_char_gate(session: &mut GameSession) {
    let mut n = 0;
    while session.pending_input() == InputKind::Char && n < 10 {
        let _ = session.submit_char(13);
        n += 1;
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        format!("panicked: {s}")
    } else if let Some(s) = payload.downcast_ref::<String>() {
        format!("panicked: {s}")
    } else {
        "panicked".to_string()
    }
}

fn print_table(rows: &[Row]) {
    println!(
        "{:<38} {:>3} {:>6} {:<22} {:<14} {:>4} {:<16} {:>3} {:<6}  inventory / declared exits",
        "story", "ver", "room", "room name", "method", "plyr", "player name", "inv", "exits"
    );
    for r in rows {
        match &r.outcome {
            Outcome::Ok {
                room_id, room_name, method, player_id, player_name, inventory, exit_convention, declared_exits,
            } => {
                println!(
                    "{:<38} {:>3} {:>6} {:<22} {:<14} {:>4} {:<16} {:>3} {:<6}  {}",
                    trunc(&r.file, 38),
                    r.version,
                    room_id.map(|n| n.to_string()).unwrap_or_else(|| "-".into()),
                    trunc(room_name.as_deref().unwrap_or("-"), 22),
                    method.as_deref().unwrap_or("-"),
                    player_id.map(|n| n.to_string()).unwrap_or_else(|| "-".into()),
                    trunc(player_name.as_deref().unwrap_or("-"), 16),
                    inventory.len(),
                    exit_convention,
                    inventory.iter().take(4).cloned().collect::<Vec<_>>().join(" | "),
                );
                if !declared_exits.is_empty() {
                    println!("{:<38}   declared exits: {declared_exits}", "");
                }
            }
            Outcome::Failed(reason) => {
                println!("{:<38} {:>3}  FAILED: {reason}", trunc(&r.file, 38), r.version);
            }
        }
    }
    let ok = rows.iter().filter(|r| matches!(r.outcome, Outcome::Ok { .. })).count();
    let with_player = rows
        .iter()
        .filter(|r| matches!(&r.outcome, Outcome::Ok { player_id: Some(_), .. }))
        .count();
    let with_inventory = rows
        .iter()
        .filter(|r| matches!(&r.outcome, Outcome::Ok { inventory, .. } if !inventory.is_empty()))
        .count();
    let inform = rows
        .iter()
        .filter(|r| matches!(&r.outcome, Outcome::Ok { exit_convention, .. } if *exit_convention == "inform"))
        .count();
    let zil = rows
        .iter()
        .filter(|r| matches!(&r.outcome, Outcome::Ok { exit_convention, .. } if *exit_convention == "zil"))
        .count();
    println!(
        "\n{} stories, {ok} booted, {with_player} with a player object, {with_inventory} with a non-empty \
         inventory, {inform} Inform (door_dir), {zil} ZIL (DIRECTIONS)",
        rows.len()
    );
}

fn print_json(rows: &[Row]) {
    println!("[");
    for (i, r) in rows.iter().enumerate() {
        println!("  {{");
        println!("    \"file\": \"{}\",", esc(&r.file));
        println!("    \"version\": {},", r.version);
        match &r.outcome {
            Outcome::Ok {
                room_id, room_name, method, player_id, player_name, inventory, exit_convention, declared_exits,
            } => {
                println!("    \"room_id\": {},", opt_num(*room_id));
                println!("    \"room_name\": {},", opt_str(room_name.as_deref()));
                println!("    \"method\": {},", opt_str(method.as_deref()));
                println!("    \"player_id\": {},", opt_num(*player_id));
                println!("    \"player_name\": {},", opt_str(player_name.as_deref()));
                println!(
                    "    \"inventory\": [{}],",
                    inventory.iter().map(|s| format!("\"{}\"", esc(s))).collect::<Vec<_>>().join(", ")
                );
                println!("    \"exit_convention\": \"{}\",", esc(exit_convention));
                println!("    \"declared_exits\": \"{}\"", esc(declared_exits));
            }
            Outcome::Failed(reason) => {
                println!("    \"failed\": \"{}\"", esc(reason));
            }
        }
        println!("  }}{}", if i + 1 == rows.len() { "" } else { "," });
    }
    println!("]");
}

fn opt_num(n: Option<u16>) -> String {
    n.map(|v| v.to_string()).unwrap_or_else(|| "null".to_string())
}

fn opt_str(s: Option<&str>) -> String {
    match s {
        Some(s) => format!("\"{}\"", esc(s)),
        None => "null".to_string(),
    }
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
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}
