// scott-cli — dumb-terminal Scott Adams (ScottFree `.dat`) host driver.
//
// The zero-Glk counterpart to `zvm-cli`/`gvm-cli`: it loads a real Adventure
// International `.dat` and plays it via stdin/stdout, so a published game can be
// smoke-tested headlessly by piping a command script in and diffing the
// transcript. Scott is line-only — no char input, windows, colour or sound — so
// the whole host loop is: describe → prompt → read a line → step → print.
//
// **A Scott game has no save protocol of its own; the HOST still saves** (SQ-0919).
// Those are two different statements and this header used to run them together,
// which is how the gap stayed invisible for so long: there is no `@save` opcode to
// intercept because Scott has no such opcode, but `scott::Vm` has carried
// `snapshot`/`restore` all along and the TUI has used them all along. Classic
// ScottFree did the same thing — its SAVE GAME and LOAD GAME are the interpreter's
// commands, not the adventure's — so `/save` and `/restore` here are period-correct
// as well as useful.
//
// Usage: scott-cli <adv.dat> [--seed <n>] [--max-turns <n>] [--data-dir <path>]
//   --seed <n>       seed the VM's occurrence-roll PRNG for reproducible runs
//   --max-turns <n>  stop after N commands (a safety cap for scripted input)
//   --data-dir <p>   where saves live (default: beside the .dat)
//   --story <n|name> which program to take off a disk image that holds several

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

use cli_host::{HostMode, StoryOrigin, TerminalGuard};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use scott::{looks_like_scottfree_save, Database, Vm};

/// The canonical Scott Adams input prompt (mirrors `ScottSession::PROMPT` in the
/// app). ScottFree prints it from its input routine; the VM stays input-agnostic,
/// so the host layer owns it. Scott used this phrase, never the Infocom-style `>`.
const PROMPT: &str = "\nTell me what to do ? ";

/// The authentic Scott divider drawn between the room "window" and the command
/// area: `<` and `>` bracketing a run of em-dashes, sized to the room block.
///
/// `None` in plain mode. The rule is pure decoration — it stands in for the
/// boundary a real Scott terminal drew between its two windows — and a screen
/// reader has no way to take it as one: it is thirty-odd em-dashes, announced
/// one at a time or swallowed entirely depending on the reader's punctuation
/// settings, and either way it says nothing. The blank line before the room
/// block already separates the two for a listener (SQ-0608).
fn separator(block: &str, plain: bool) -> Option<String> {
    if plain {
        return None;
    }
    let width = block.lines().map(|l| l.chars().count()).max().unwrap_or(0).max(20);
    Some(format!("<{}>", "\u{2014}".repeat(width.saturating_sub(2))))
}

/// Write `text`, pausing at the bottom of each page.
///
/// Scott prints in blocks — a room description, a turn's response — rather than
/// a character at a time, so the newlines are counted as the block goes out and
/// the pause lands at the line that filled the page.
fn page_out(out: &mut impl Write, pager: &mut cli_host::Pager, interactive: bool, text: &str) {
    for (i, piece) in text.split_inclusive('\n').enumerate() {
        let _ = write!(out, "{piece}");
        if piece.ends_with('\n') && pager.line() {
            let _ = out.flush();
            pager.pause(out, interactive);
        }
        let _ = i;
    }
}

/// Read one command line. Returns `None` at end of input (EOF, or Ctrl-C/Ctrl-D
/// when interactive).
///
/// Piped input (non-TTY): a plain line read, echoed so a captured transcript
/// reads naturally. Interactive (TTY): a minimal raw-mode line editor that echoes
/// typed characters, handles Backspace, and — crucially — swallows arrow keys and
/// other escape sequences instead of letting the terminal spew `^[[A` garbage into
/// the line. It deliberately does not implement history or cursor movement; this
/// is a play/smoke harness, not a full readline.
fn read_command(interactive: bool, out: &mut impl Write) -> Option<String> {
    if !interactive {
        let line = cli_host::read_line_stdin()?;
        let line = line.trim_end_matches(['\n', '\r']).to_string();
        let _ = writeln!(out, "{line}"); // echo for a readable piped transcript
        return Some(line);
    }
    // Raw mode disables canonical line editing and echo; on failure, fall back to a
    // cooked read (arrow keys may still echo, but input still works).
    if terminal::enable_raw_mode().is_err() {
        let line = cli_host::read_line_stdin()?;
        return Some(line.trim_end_matches(['\n', '\r']).to_string());
    }
    let mut buf = String::new();
    let result = loop {
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                // Ctrl-C / Ctrl-D quit (raw mode routes them as keys, not signals).
                KeyCode::Char('c' | 'd') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    break None;
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    let _ = write!(out, "{c}");
                    let _ = out.flush();
                }
                KeyCode::Backspace => {
                    if buf.pop().is_some() {
                        let _ = write!(out, "\u{8} \u{8}"); // erase the last glyph
                        let _ = out.flush();
                    }
                }
                KeyCode::Enter => {
                    let _ = write!(out, "\r\n");
                    let _ = out.flush();
                    break Some(buf.clone());
                }
                // Arrows, Home/End, function keys, etc.: ignored (no garbage).
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    let _ = terminal::disable_raw_mode();
    result
}

struct Args {
    path: String,
    seed: Option<u32>,
    max_turns: Option<u64>,
    /// `--pager on|off`: [MORE] paging. This was `--no-more`/`--no-page` before
    /// SQ-1082, which could only ever turn it off — and named the PROMPT rather
    /// than the feature. `pager` is the noun the code already used
    /// (`cli_host::pager`), the one the help already used ("[MORE] paging"), and
    /// the one a terminal user already has; `--more on|off` reads as a
    /// comparative and `--page on|off` as a verb missing its object. One noun
    /// across all four front-ends now.
    pager: bool,
    /// `--data-dir`: where saves live. `None` puts them beside the `.dat`, which
    /// is what `cli_host::game_dir` does for the other two hosts.
    data_dir: Option<String>,
    /// `--story <n|name>`: which program to take off a disk image that holds
    /// several — the Commodore 64 *Mysterious Adventures* compilation disks
    /// (SQ-1414). Mirrors `zvm-cli`'s flag of the same name and matches through
    /// the same `cli_host::story_pick` rule, so the two front-ends cannot
    /// disagree about what a number or a name picks out.
    story: Option<String>,
    /// ScottFree's four runtime option flags (SQ-1413), mirroring its own
    /// `-y`/`-s`/`-t`/`-p` — see `scott::Options`.
    options: scott::Options,
}

/// Every option `scott-cli` accepts; `cli_host::args` applies the rules.
const OPTS: &[cli_host::Opt] = &[
    cli_host::Opt::flag(&["--screen-reader", "--plain"]),
    cli_host::Opt::flag(&["--help", "-h"]),
    cli_host::Opt::flag(&["--version", "-V"]),
    cli_host::Opt::valued(&["--pager"]),
    cli_host::Opt::valued(&["--seed"]),
    cli_host::Opt::valued(&["--max-turns"]),
    cli_host::Opt::valued(&["--data-dir"]),
    cli_host::Opt::valued(&["--story"]),
    cli_host::Opt::flag(&["--you-are", "-y"]),
    cli_host::Opt::flag(&["--scott-light", "-s"]),
    cli_host::Opt::flag(&["--trs80", "-t"]),
    cli_host::Opt::flag(&["--prehistoric-lamp", "-p"]),
];

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let m = cli_host::scan(argv, OPTS)?;
    if m.positional.len() > 1 {
        return Err(format!("unexpected extra argument: {}", m.positional[1]));
    }
    // Strict, unlike zvm-cli's lenient interpreter number: a seed or turn cap
    // that does not parse is a typo in a reproducibility knob, and silently
    // ignoring it would make a run that looks reproducible but is not.
    let num = |flag: &str| -> Result<Option<u64>, String> {
        match m.value(flag) {
            None => Ok(None),
            Some(v) => v.parse().map(Some).map_err(|_| format!("bad {flag} value: {v}")),
        }
    };
    // `-t`/`--trs80` sets BOTH the flag and this crate's own `Presentation`
    // (see `Options::trs80_style`'s doc for why they're separate knobs) —
    // a CLI user asking for the TRS-80 flag wants the TRS-80 room-block
    // layout too. Off, `scott-cli` keeps `Options::default`'s own
    // `Presentation::C64` layout unchanged (SQ-1413) — the flag adds the
    // TRS-80 choice, it does not switch the baseline to plain ScottFree.
    let trs80 = m.has("--trs80");
    let mut options = scott::Options::new()
        .with_you_are(m.has("--you-are"))
        .with_scott_light(m.has("--scott-light"))
        .with_trs80_style(trs80)
        .with_prehistoric_lamp(m.has("--prehistoric-lamp"));
    if trs80 {
        options = options.with_presentation(scott::Presentation::Trs80);
    }
    Ok(Args {
        path: m.first_positional().ok_or("no story file given")?.to_string(),
        seed: num("--seed")?.map(|v| v as u32),
        max_turns: num("--max-turns")?,
        pager: cli_host::on_off("--pager", m.value("--pager"))?.unwrap_or(true),
        data_dir: m.value("--data-dir").map(str::to_string),
        story: m.value("--story").map(str::to_string),
        options,
    })
}

// ── disk images (SQ-1414) ───────────────────────────────────────────────────

/// One Scott Adams program file found on a mounted disk image.
struct DiskCandidate {
    /// The name the volume stores it under — the CBM filename on a Commodore
    /// disk (`BATON`, `TIME MACHINE`, …).
    name: String,
    /// The program file's bytes, load-address bytes included — exactly what
    /// [`scott::c64::parse_c64_mysterious_prg`] takes.
    bytes: Vec<u8>,
}

/// The canonical title for a Commodore 64 *Mysterious Adventures* program
/// file, by the CBM name the disk stores it under (`BATON` → "The Golden
/// Baton"), matched case-insensitively.
///
/// `scott::c64::RELEASES` is derived from the disks themselves and keyed on
/// their own spelling, unlike `app`'s `scott_titles.tsv` — see that crate's
/// module docs.
fn scott_c64_title(disk_name: &str) -> Option<&'static str> {
    scott::c64::RELEASES
        .iter()
        .find(|r| r.file_name.eq_ignore_ascii_case(disk_name))
        .map(|r| r.title)
}

/// The canonical title for a US S.A.G.A. release, by the database's own
/// content identity rather than by whatever the container happens to call it
/// (SQ-1470) — mirrors `app::picker`'s `scott_release_title`, which has the
/// fuller reasoning. `scott::Database::parse` is cheap enough over these
/// small files to call again here rather than plumb the parsed `Database`
/// through; `main` re-parses the chosen candidate's bytes anyway.
fn saga_us_title(bytes: &[u8]) -> Option<&'static str> {
    scott::Database::parse(bytes).ok()?.saga_us?.display_title()
}

impl DiskCandidate {
    /// How this candidate reads in the menu: the US S.A.G.A. content-identified
    /// title (already platform-qualified — "Voodoo Castle (Atari 8-bit)"), else
    /// the Commodore *Mysterious Adventures* table's title beside the disk's own
    /// name, else the bare name.
    fn label(&self) -> String {
        if let Some(title) = saga_us_title(&self.bytes) {
            return title.to_string();
        }
        match scott_c64_title(&self.name) {
            Some(title) => format!("{title}  ({})", self.name),
            None => self.name.clone(),
        }
    }
}

/// These candidates as the shared chooser sees them — the same `Row` shape
/// `zvm-cli` builds for its own disk menu, so `--story` matches identically in
/// both front-ends.
fn disk_rows(cands: &[DiskCandidate]) -> Vec<cli_host::story_pick::Row> {
    cands
        .iter()
        .map(|c| cli_host::story_pick::Row {
            name: c.name.clone(),
            title: saga_us_title(&c.bytes)
                .map(str::to_string)
                .or_else(|| scott_c64_title(&c.name).map(str::to_string)),
            label: c.label(),
        })
        .collect()
}

/// Mount `raw` (the disk image at `path`) and return every Scott Adams
/// program file it holds — the Commodore 64 *Mysterious Adventures*
/// compilation disks read through the one disk seam every front-end shares
/// (SQ-1414).
///
/// Filtered by [`scott::looks_like_scott_bytes`] over
/// [`blorb::medium::MountedDisk::contents`], which lists every file on the
/// volume regardless of engine — a Z-machine or Glulx sibling on the same
/// disk (there are none in the corpus, but nothing here assumes it) is simply
/// not a Scott Adams program and does not pass the sniff.
///
/// **The US S.A.G.A. Atari 8-bit database is not a directory entry**
/// (SQ-1470): the release masters its database straight over Atari DOS 2's
/// own volume table of contents, so `contents()` lists at most
/// `DOS.SYS`/`AUTORUN.SYS` on these sides and never the database itself —
/// see `blorb::atr`'s module docs. The one extra candidate the format offers
/// beyond its directory is the whole image, through the reserved
/// `blorb::atr::IMAGE_ENTRY` door. The Apple II and Commodore 64 releases
/// need no such door: their databases ARE ordinary catalogue files.
fn story_candidates(path: &Path, raw: Vec<u8>) -> Result<Vec<DiskCandidate>, String> {
    let disk = cli_host::disk_set::mount_at(path, raw)
        .map_err(|e| format!("Error: cannot mount the disk image: {e}"))?;
    let mut contents = disk.contents();
    if disk.format() == blorb::medium::DiskImage::AtariDos2 {
        if let Some(image) = disk.read_named(blorb::atr::IMAGE_ENTRY) {
            contents.push((blorb::atr::IMAGE_ENTRY.to_string(), image));
        }
    }
    let found: Vec<DiskCandidate> = contents
        .iter()
        .filter(|(_, bytes)| scott::looks_like_scott_bytes(bytes))
        .map(|(name, bytes)| DiskCandidate {
            name: saga_us_keyed_name(name, bytes),
            bytes: bytes.clone(),
        })
        .collect();
    if found.is_empty() {
        let names: Vec<String> = contents.iter().map(|(n, _)| n.clone()).collect();
        let named = disk.volume_name().map(|n| format!(" on {n}")).unwrap_or_default();
        return Err(format!(
            "Error: no Scott Adams program on this disk image{named} ({} file{}: {})",
            names.len(),
            if names.len() == 1 { "" } else { "s" },
            names.join(", "),
        ));
    }
    Ok(found)
}

/// A save-key-safe name for a disk-mounted Scott candidate (SQ-1470) — mirrors
/// `app::hints`'s helper of the same name, which has the fuller reasoning.
///
/// **A US S.A.G.A. release's own container entry is not a distinguishing
/// name.** `blorb::atr::IMAGE_ENTRY` is the literal `"IMAGE"` on all seven
/// Atari sides, and the Apple II boot disks spell two DIFFERENT titles
/// `DATABASE` — and this binary's own save key is the entry name alone
/// (`cli_host::story_key_for`; Scott bytes carry no Z-machine header to build
/// a `DiskBuild` from), so two different games under either literal name
/// would share one save directory. The (version, adventure, platform) triple
/// is this release's actual identity and is appended to the real container
/// name — a no-op for every other Scott source, whose names are already
/// unique and already shipped.
fn saga_us_keyed_name(name: &str, bytes: &[u8]) -> String {
    match scott::Database::parse(bytes).ok().and_then(|db| db.saga_us) {
        Some(saga) => {
            format!("{name}-{}-{}-{}", saga.version, saga.adventure, saga_us_platform_slug(saga.platform))
        }
        None => name.to_string(),
    }
}

/// A short, filename-safe token per [`scott::SagaPlatform`], for
/// [`saga_us_keyed_name`].
fn saga_us_platform_slug(platform: scott::SagaPlatform) -> &'static str {
    match platform {
        scott::SagaPlatform::Atari8Bit => "atari8bit",
        scott::SagaPlatform::AppleII => "appleii",
        scott::SagaPlatform::Commodore64 => "c64",
        _ => "unknown",
    }
}

/// Pick a Scott Adams program off a mounted image — the same shape as
/// `zvm-cli`'s `media::choose`, matched through `cli_host::story_pick` so a
/// number or a name resolves identically in both front-ends.
///
/// One candidate opens without asking. `--story` decides it outright.
/// Otherwise a terminal on stdin gets the menu and answers it; without one
/// this refuses to block and names the flag to pass instead.
fn choose_disk_story(
    cands: &[DiskCandidate],
    want: Option<&str>,
    stdin_is_tty: bool,
    mut announce: impl FnMut(&str),
    mut read_line: impl FnMut() -> Option<String>,
) -> Result<usize, String> {
    let rows = disk_rows(cands);
    if let Some(w) = want {
        return cli_host::story_pick::find(&rows, w, "this disk");
    }
    if cands.len() == 1 {
        return Ok(0);
    }
    if !stdin_is_tty {
        return Err(format!(
            "Error: this disk image holds {} Scott Adams programs; pass --story <n|name> to \
             pick one:\n{}",
            cands.len(),
            cli_host::story_pick::menu(&rows)
        ));
    }
    announce(&format!(
        "This disk holds {} Scott Adams programs:\n{}",
        cands.len(),
        cli_host::story_pick::menu(&rows)
    ));
    loop {
        announce(&format!("Which one? [1-{}] ", cands.len()));
        let Some(line) = read_line() else {
            return Err("Error: no story chosen.".to_string());
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match cli_host::story_pick::find(&rows, line, "this disk") {
            Ok(i) => return Ok(i),
            Err(e) => announce(&format!("{e}\n")),
        }
    }
}

// ── host commands ─────────────────────────────────────────────────────────────

/// A command the HOST answers, never passed to the game.
///
/// The leading slash is what makes interception safe, and the reasoning is
/// `cli_host::input::is_status_request`'s: any bare word risks shadowing a verb
/// the adventure defines, and a host that silently eats a real command is worse
/// than no feature. Scott's parser is two words deep and assigns no meaning to
/// `/` at all.
#[derive(Debug, PartialEq, Eq)]
enum HostCommand {
    /// Repeat the room block.
    Status,
    /// Save; the name may be empty, meaning "ask me".
    Save(String),
    /// Restore; likewise.
    Restore(String),
}

/// Parse `line` as a host command, or `None` for a game command.
///
/// A bare `/save` is not an error — it prompts, which is where the numbered list
/// of what already exists gets shown. `/save cellar` skips straight to the name,
/// for a scripted transcript that cannot answer a prompt.
fn host_command(line: &str) -> Option<HostCommand> {
    let t = line.trim();
    if cli_host::input::is_status_request(t) {
        return Some(HostCommand::Status);
    }
    let (verb, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    let rest = rest.trim().to_string();
    match verb.to_ascii_lowercase().as_str() {
        "/save" => Some(HostCommand::Save(rest)),
        "/restore" | "/load" => Some(HostCommand::Restore(rest)),
        _ => None,
    }
}

// ── save / restore ────────────────────────────────────────────────────────────

/// Prompt for a save name, listing what already exists above it (SQ-0918).
///
/// Reads through [`read_command`] rather than stdin directly, so the raw-mode
/// editor, the piped echo and the EOF rule are the same ones the game prompt uses
/// — a filename typed at a TTY must behave like every other line this host reads.
/// EOF yields an empty string, which every caller treats as a cancel.
fn prompt_line(
    out: &mut impl Write,
    interactive: bool,
    prompt: &str,
    saves: &[String],
) -> String {
    if let Some(line) = cli_host::save_list_line(saves) {
        let _ = writeln!(out, "\n{line}");
    }
    let _ = write!(out, "{prompt}");
    let _ = out.flush();
    read_command(interactive, out).unwrap_or_default()
}

/// Write `vm`'s snapshot to `name` (or to a name the player is asked for).
///
/// The bytes are `scott::Vm::snapshot`'s — item locations, the player's room, the
/// flags, the counters and the lamp — and they carry `SCOTT_EXT` rather than
/// `.qzl` because they are not Quetzal and it would be a lie to say they were.
fn do_save(
    out: &mut impl Write,
    interactive: bool,
    vm: &Vm,
    game_dir: &Path,
    name: &str,
) {
    let saves = cli_host::existing_saves(game_dir, cli_host::SCOTT_EXT);
    let typed = if name.is_empty() {
        prompt_line(out, interactive, "Save as ? ", &saves)
    } else {
        name.to_string()
    };
    if typed.trim().is_empty() {
        let _ = writeln!(out, "Save cancelled.");
        return;
    }
    // Deliberately NOT `pick_save`: at a save prompt a number would mean
    // "overwrite that one", and silently clobbering a save the player named is the
    // defect SQ-0648 fixed in the TUI. The list is a reminder of what you would
    // collide with, not a way to choose a target.
    let path = cli_host::resolve_save_input(&typed, game_dir, cli_host::SCOTT_EXT);
    if let Some(warning) = cli_host::overwrite_warning(&path) {
        if !cli_host::is_yes(&prompt_line(out, interactive, &warning, &[])) {
            let _ = writeln!(out, "Save cancelled.");
            return;
        }
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    match fs::write(&path, vm.snapshot()) {
        Ok(()) => {
            let _ = writeln!(out, "Saved to '{}'.", path.display());
        }
        Err(e) => {
            let _ = writeln!(out, "Save failed: {e}");
        }
    }
}

/// Restore `vm` from `name` (or from a name the player is asked for).
///
/// `true` when the VM actually changed, so the caller knows to redraw the room —
/// a restore that lands you somewhere else and says nothing about it is the one
/// way this could be worse than not having it.
///
/// **A save from a different adventure is refused, not half-applied.**
/// `Vm::restore`/`Vm::restore_scottfree` both check the loaded database
/// before writing anything, so pointing this at another game's save fails
/// cleanly instead of scattering one adventure's item locations through
/// another's.
///
/// **Accepts either save format, detected by shape (SQ-1413):** this
/// binary's own `Vm::snapshot` (`Vm::SNAPSHOT_MAGIC`, unambiguous — checked
/// first) or a ScottFree 1.14 text save (`scott::looks_like_scottfree_save`)
/// — the classic interpreter's own `.sav`, brought in from ScottFree itself
/// or a fork that kept its format. Detection is the crate's, not
/// reimplemented here, so the app host and this CLI can never disagree
/// about what counts as which.
fn do_restore(
    out: &mut impl Write,
    interactive: bool,
    vm: &mut Vm,
    game_dir: &Path,
    name: &str,
) -> bool {
    let saves = cli_host::existing_saves(game_dir, cli_host::SCOTT_EXT);
    let typed = if name.is_empty() {
        prompt_line(out, interactive, "Restore which ? ", &saves)
    } else {
        name.to_string()
    };
    if typed.trim().is_empty() {
        let _ = writeln!(out, "Restore cancelled.");
        return false;
    }
    // Here a number DOES pick from the list, which is safe because restoring is
    // not destructive of anything on disk (SQ-0918).
    let chosen = cli_host::pick_save(&typed, &saves).map_or(typed.clone(), str::to_string);
    let path = cli_host::resolve_save_input(&chosen, game_dir, cli_host::SCOTT_EXT);
    match fs::read(&path) {
        Ok(bytes) => {
            let is_scottfree_save =
                !bytes.starts_with(&Vm::SNAPSHOT_MAGIC) && looks_like_scottfree_save(&bytes);
            let result =
                if is_scottfree_save { vm.restore_scottfree(&bytes) } else { vm.restore(&bytes) };
            match result {
                Ok(()) => {
                    let _ = writeln!(out, "Restored from '{}'.", path.display());
                    true
                }
                Err(e) => {
                    let _ = writeln!(out, "Restore failed: '{}' is not a save for this game ({e}).", path.display());
                    false
                }
            }
        }
        Err(e) => {
            let _ = writeln!(out, "Restore failed: {e}");
            false
        }
    }
}

const HELP: &str = "\
scott-cli — DOS-style Scott Adams (ScottFree) player (no map)

Usage: scott-cli [OPTIONS] <adv.dat>

Arguments:
  <adv.dat>             Scott Adams ScottFree .dat adventure, a TI-99/4A or
                        Commodore 64 release, or a disk image carrying one
                        (the Commodore 64 Mysterious Adventures compilation
                        disks hold several — see --story)

Host commands (typed at any prompt, never passed to the game):
  /status               Repeat the room block (location, exits, what is here)
  /save [name]          Save the game. Bare, it lists what you already have and
                        asks; with a name it goes straight there (and still asks
                        before overwriting). Scott has no save format of its
                        own, so these are the host's own snapshots and carry
                        .sav rather than .qzl.
  /restore [name]       Restore. Bare, it lists your saves and takes a number or
                        a name. A save from a different adventure is refused
                        rather than half-applied. Also accepts a save from the
                        classic ScottFree interpreter itself. Alias: /load

Options:
      --screen-reader   Linear plain text (alias: --plain; also selected by
                        TERM=dumb). Hands line editing and echo back to the
                        terminal and drops the em-dash divider rule, which a
                        reader can only spell out. Scott has no status window to
                        suppress — the room block IS the story — so there is no
                        --story-only here. Ask for the room again any time with
                        /status.
      --pager <on|off>  [MORE] paging on long output. Default on, and off
                        wherever it could not work anyway: --screen-reader, or a
                        piped stdout.
      --seed <n>        Seed the RNG for reproducible play
      --data-dir <path> Where saves live (default: a .save directory beside the
                        .dat, the same rule zvm-cli and gvm-cli follow)
      --story <n|name>  Which program to take off a disk image that holds more
                        than one, by its 1-based menu number or a case-
                        insensitive (sub)name — the Commodore 64 Mysterious
                        Adventures compilation disks (MYSTADV1.D64,
                        MYSTADV2.D64) hold six and five apiece. A single-
                        program disk needs no flag at all.
      --max-turns <n>   Stop after n turns (headless/testing)
  -y, --you-are         Second-person replies (\"You are dead.\", \"You are
                        carrying:\") instead of ScottFree's plain first-person
                        default (\"I am dead.\", \"I'm carrying:\"). Some
                        Brian Howarth titles (Robin of Sherwood among them)
                        were authored assuming this flag.
  -s, --scott-light     The original Adams lamp countdown wording: a running
                        \"Light runs out in N turns.\" every turn under 25,
                        instead of ScottFree's own \"Your light is growing
                        dim.\" every 5th turn.
  -t, --trs80           The TRS-80 room-block layout (items suffixed \". \"
                        and the whole block framed by the TRS-80's rule)
                        in place of this player's own default layout.
  -p, --prehistoric-lamp
                        The light source is destroyed the instant its fuel
                        reaches zero, rather than merely going dark and
                        staying an inert carried item.
  -V, --version         Print version and exit
  -h, --help            Print this help and exit
";

fn main() {
    let argv: Vec<String> = env::args().collect();
    if cli_host::handled_common_flags(&argv, HELP, env!("CARGO_BIN_NAME"), buildinfo::LONG) {
        return;
    }
    let args = match parse_args(&argv) {
        // Already the only CLI that rejected unknown flags; now it shows the
        // help alongside the message, like the other two (SQ-0614).
        Err(e) => cli_host::usage_error(env!("CARGO_BIN_NAME"), &e, HELP),
        Ok(a) => a,
    };

    // Settled before the disk mount below, which may need to ask the player a
    // question — mirrors `zvm-cli`'s own `--story` wiring (SQ-1414).
    let mode = HostMode::detect_with(cli_host::plain_requested(&argv)).install();

    let story_path = Path::new(&args.path);
    let raw = fs::read(story_path).unwrap_or_else(|e| {
        eprintln!("scott-cli: cannot read {}: {e}", args.path);
        process::exit(1);
    });

    // An original release floppy — the Commodore 64 *Mysterious Adventures*
    // compilation disks (SQ-1414): mount it and take a program off it, through
    // the same `blorb::medium` seam every other front-end opens a disk with.
    // `disk_entry` is the CBM name chosen, so saves below can be keyed per
    // program rather than per disk.
    let (bytes, disk_entry): (Vec<u8>, Option<String>) =
        if blorb::medium::DiskImage::detect(&raw).is_some() {
            let cands = story_candidates(story_path, raw).unwrap_or_else(|e| {
                eprintln!("{e}");
                process::exit(1);
            });
            let chosen = choose_disk_story(
                &cands,
                args.story.as_deref(),
                mode.stdin_tty(),
                |s| {
                    print!("{s}");
                    let _ = io::stdout().flush();
                },
                || {
                    let mut line = String::new();
                    match io::stdin().read_line(&mut line) {
                        Ok(0) | Err(_) => None,
                        Ok(_) => Some(line),
                    }
                },
            )
            .unwrap_or_else(|e| {
                eprintln!("{e}");
                process::exit(1);
            });
            // Say which one opened whenever the story came off a disk —
            // including the scripted `--story` path, where nothing else on
            // screen would show it (mirrors `zvm-cli`'s own disk menu).
            //
            // Unconditional since SQ-1470, not just "when there was a choice
            // to get wrong": an Atari 8-bit or Apple II S.A.G.A. side always
            // holds exactly one Scott candidate, and it is the ONE piece of
            // output that says which platform's release just opened — the
            // same title exists on more than one, and nothing else on screen
            // would say so.
            println!("Opening {}) {}", chosen + 1, cands[chosen].label());
            let name = cands[chosen].name.clone();
            let mut cands = cands;
            (cands.swap_remove(chosen).bytes, Some(name))
        } else {
            (raw, None)
        };

    // Bytes, not `&str`: since SQ-1414 this reads two BINARY encodings as well
    // — the TI-99/4A tokenised releases, and the Commodore 64 *Mysterious
    // Adventures* as `.prg` program files — and no UTF-8 conversion survives
    // either. `looks_like_scott_bytes` answers for all three and is the same
    // predicate `Database::parse` then applies. A disk-mounted candidate has
    // already passed this exact sniff in `story_candidates`; repeating it here
    // costs nothing and keeps the loose-file path unchanged.
    if !scott::looks_like_scott_bytes(&bytes) {
        eprintln!(
            "scott-cli: {} does not look like a Scott .dat, a TI-99/4A release \
             or a Commodore 64 Mysterious Adventures .prg",
            args.path
        );
        process::exit(1);
    }
    let db = Database::parse(&bytes).unwrap_or_else(|e| {
        eprintln!("scott-cli: invalid Scott game data: {e:?}");
        process::exit(1);
    });

    // Seed and Options both belong at construction (`Vm::new_full`'s own
    // doc): the opening occurrence pass runs inside the constructor and can
    // roll percentage chances or print option-gated wording, so setting
    // either afterward would be too late for it.
    let mut vm = Vm::new_full(
        db,
        false,
        args.seed.unwrap_or(Vm::DEFAULT_RNG_SEED),
        args.options,
    );
    // Saves live where the other two hosts put theirs, by the same rule, so a
    // player who has learned one has learned all three (SQ-0919) — keyed per
    // ENTRY when the story came off a disk holding several (SQ-1414), so
    // `BATON` and `TIME MACHINE` off the same `MYSTADV1.D64` do not share a
    // save directory. Scott bytes carry no Z-machine header, so `build` is
    // always `None` here; the entry alone decides.
    let key = cli_host::story_key_for(StoryOrigin {
        path: story_path,
        entry: disk_entry.as_deref(),
        build: None,
    });
    let game_dir: PathBuf =
        cli_host::game_dir_with_key(story_path, args.data_dir.as_deref(), &key);

    // scott-cli emits no escape sequences at all, so `rich` never comes up here
    // — the only terminal state it touches is raw mode, and until SQ-0605 it had
    // no teardown of any kind: a panic mid-game left the shell in raw mode.
    // `raw_only` fixes that without putting escapes into a transcript that has
    // never had any.
    //
    // That also makes plain mode nearly free here: the one thing `--plain` has
    // to change is handing line editing back to the terminal, which is exactly
    // what `raw_input` decides (SQ-0606).
    let interactive = mode.raw_input();
    let _guard = TerminalGuard::raw_only();
    let mut out = io::stdout();

    // Any pending output before the first prompt (empty today — the room is shown
    // via the room block below).
    let _ = write!(out, "{}", vm.take_output());

    let mut turns = 0u64;
    let mut last_block = String::new();
    // Scott stores no score: it is the count of treasures deposited in the
    // treasure room, recomputed each turn (SQ-0616). Screen-reader mode only —
    // otherwise the SCORE verb is the way to ask.
    let mut score_watch = cli_host::ScoreWatch::new();
    let announce_scores = mode.plain();
    // `[MORE]` paging (SQ-0617). Needs both ends to be a terminal — a pipe would
    // never answer the prompt — and is off in screen-reader mode by choice.
    let term_rows = terminal::size().map(|(_, r)| r).unwrap_or(24);
    let mut pager = cli_host::Pager::new(
        mode.both_tty() && args.pager && !mode.plain(),
        cli_host::Pager::height_for(term_rows),
    );
    let interactive_pager = mode.both_tty();
    loop {
        if vm.has_quit() {
            break;
        }
        if let Some(max) = args.max_turns {
            if turns >= max {
                eprintln!("\nscott-cli: reached --max-turns {max}, stopping");
                break;
            }
        }
        // The room block is the top "window" in the real game; on a dumb terminal
        // we print it inline whenever the room (or its contents) changes.
        let block = vm.room_block();
        if block != last_block {
            page_out(&mut out, &mut pager, interactive_pager, &format!("\n{block}\n"));
            if let Some(rule) = separator(&block, mode.plain()) {
                page_out(&mut out, &mut pager, interactive_pager, &format!("{rule}\n"));
            }
            last_block = block;
        }
        if announce_scores {
            let (stored, _total) = vm.treasures_stored();
            if let Some(line) = score_watch.update(Some(stored)) {
                let _ = writeln!(out, "{line}");
            }
        }
        // The player is about to be asked something, so they have caught up.
        pager.reset();
        let _ = write!(out, "{PROMPT}");
        let _ = out.flush();

        // `/status` is answered by the host and the game never sees it, so loop
        // until a real command arrives. Scott's "status" is the room block —
        // where you are, the exits, what is here — which the loop above prints
        // only when it *changes*, so after a few turns of conversation it has
        // scrolled away (SQ-0610).
        let read = loop {
            let line = read_command(interactive, &mut out);
            // Both a real command and end-of-input leave the loop; only a HOST
            // command goes round again. Keeping EOF as `None` here matters —
            // turning it into an empty command is the bug that hung the other two
            // CLIs (SQ-0604/0605).
            let Some(l) = line else { break None };
            match host_command(&l) {
                Some(HostCommand::Status) => {
                    let _ = writeln!(out, "{}", vm.room_block());
                }
                Some(HostCommand::Save(name)) => {
                    do_save(&mut out, interactive, &vm, &game_dir, &name);
                }
                Some(HostCommand::Restore(name)) => {
                    if do_restore(&mut out, interactive, &mut vm, &game_dir, &name) {
                        // A restore moves you, and the outer loop only prints the
                        // room when it CHANGES — which it will not notice from in
                        // here. Show it now, and record it, so the player is told
                        // where the restore put them and the outer loop does not
                        // then print it twice.
                        let block = vm.room_block();
                        let _ = writeln!(out, "\n{block}");
                        last_block = block;
                    }
                }
                // Not ours: hand it to the game.
                None => break Some(l),
            }
            let _ = write!(out, "{PROMPT}");
            let _ = out.flush();
        };
        let Some(line) = read else {
            let _ = writeln!(out); // tidy trailing newline on EOF / quit
            break;
        };

        vm.supply_line(&line);
        let _ = vm.step();
        turns += 1;
        // Prints this turn's output; a quitting turn (win/death) is drained here
        // and the loop's top-of-iteration `has_quit` check ends the session.
        page_out(&mut out, &mut pager, interactive_pager, &vm.take_output());

        // On game end, print the final room block: the panel (upper "window")
        // reflects the closing state, but the loop's top-of-iteration block print
        // won't run because `has_quit` breaks first. Mirrors the app, which keeps
        // the final panel on screen at game over.
        if vm.has_quit() {
            let block = vm.room_block();
            if block != last_block {
                let _ = writeln!(out, "\n{block}");
                if let Some(rule) = separator(&block, mode.plain()) {
                let _ = writeln!(out, "{rule}");
            }
            }
        }
    }
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── host commands (SQ-0919) ───────────────────────────────────────────────

    /// The slash is what makes interception safe, so a bare word must NOT be one.
    /// `save` and `restore` are perfectly ordinary things to type at a Scott
    /// prompt — and a host that ate them would be worse than no feature.
    #[test]
    fn only_a_slash_makes_a_host_command() {
        assert_eq!(host_command("/status"), Some(HostCommand::Status));
        assert_eq!(host_command("/save"), Some(HostCommand::Save(String::new())));
        assert_eq!(host_command("/save cellar"), Some(HostCommand::Save("cellar".into())));
        assert_eq!(host_command("/restore"), Some(HostCommand::Restore(String::new())));
        assert_eq!(host_command("/load deep"), Some(HostCommand::Restore("deep".into())));
        // Case and surrounding space are the player's, not ours.
        assert_eq!(host_command("  /SAVE  cellar "), Some(HostCommand::Save("cellar".into())));

        for game in ["save", "restore", "load", "save game", "go north", "", "/", "/quit"] {
            assert_eq!(host_command(game), None, "{game:?} belongs to the adventure");
        }
    }

    // ── save / restore round trip ─────────────────────────────────────────────

    fn tiny_cave() -> Vm {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scott/tests/tiny_cave.dat");
        let src = fs::read_to_string(&path).expect("the redistributable fixture is checked in");
        Vm::new(Database::parse(&src).expect("valid .dat"))
    }

    fn scratch(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NTH: AtomicUsize = AtomicUsize::new(0);
        let nth = NTH.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("scott-cli-{name}-{}-{nth}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// The whole feature, end to end: a snapshot taken before a move restores the
    /// world to before that move.
    ///
    /// Both calls are given a NAME, so neither prompts — a test that read stdin
    /// would hang under nextest, where every case gets its own process and no
    /// terminal.
    #[test]
    fn a_save_restores_the_world_to_where_it_was() {
        let dir = scratch("roundtrip");
        let mut vm = tiny_cave();
        let mut out: Vec<u8> = Vec::new();

        let before = vm.room_block();
        do_save(&mut out, false, &vm, &dir, "here");
        assert!(dir.join("here.sav").is_file(), "and it lands under .sav, not .qzl");

        vm.supply_line("GO DOWN");
        let _ = vm.step();
        assert_ne!(vm.room_block(), before, "the move has to actually move, or this proves nothing");

        assert!(do_restore(&mut out, false, &mut vm, &dir, "here"));
        assert_eq!(vm.room_block(), before, "restored to before the move");

        let _ = fs::remove_dir_all(&dir);
    }

    /// **A save from another adventure is refused rather than half-applied.**
    /// `Vm::restore` checks the item count against the loaded database before it
    /// writes anything; without that check a mismatched save would scatter one
    /// game's item locations through another's.
    #[test]
    fn a_save_from_a_different_game_is_refused() {
        let dir = scratch("foreign");
        let mut vm = tiny_cave();
        let before = vm.room_block();
        let mut out: Vec<u8> = Vec::new();

        // A snapshot shaped like one, for a game with a different item count.
        let mut bogus = 999u32.to_le_bytes().to_vec();
        bogus.extend(std::iter::repeat_n(0u8, 4096));
        fs::write(dir.join("alien.sav"), &bogus).unwrap();

        assert!(!do_restore(&mut out, false, &mut vm, &dir, "alien"));
        assert_eq!(vm.room_block(), before, "and the live game is untouched");
        assert!(
            String::from_utf8_lossy(&out).contains("not a save for this game"),
            "the player is told why: {}",
            String::from_utf8_lossy(&out)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// A hand-authored ScottFree 1.14 save (see the field order documented
    /// on `scott::Vm::restore_scottfree`), shaped for `tiny_cave.dat`
    /// (`NumItems=9`, 10 item slots, 4 rooms 0..=3): 16 `counter room`
    /// pairs, a state line, then one location per item.
    fn tiny_cave_scottfree_save() -> String {
        let mut s = String::new();
        for ct in 0..16 {
            s.push_str(&format!("{} 1\n", ct + 1));
        }
        // BitFlags DarkFlag MyLoc CurrentCounter SavedRoom LightTime
        s.push_str("0 0 3 7 1 42\n");
        // idol (item 1) carried, lamp (item 9, LIGHT_SOURCE) in room 1.
        s.push_str("0\n255\n0\n0\n0\n0\n0\n0\n0\n1\n");
        s
    }

    /// `/restore` also accepts the classic ScottFree interpreter's own save
    /// format (SQ-1413), detected by shape (`scott::looks_like_scottfree_save`)
    /// rather than a reserved extension — this crate's own binary snapshot
    /// and ScottFree's text format both carry `.sav`.
    #[test]
    fn restore_accepts_a_scottfree_save() {
        let dir = scratch("scottfree-import");
        let mut vm = tiny_cave();
        let mut out: Vec<u8> = Vec::new();

        fs::write(dir.join("classic.sav"), tiny_cave_scottfree_save()).unwrap();

        assert!(do_restore(&mut out, false, &mut vm, &dir, "classic"));
        assert_eq!(vm.current_room(), 3, "MyLoc=3 from the save file");
        assert_eq!(vm.item_loc(1), scott::database::CARRIED, "the idol (255) normalises to CARRIED");
        assert_eq!(vm.item_loc(9), 1, "the lamp is in room 1");

        let _ = fs::remove_dir_all(&dir);
    }

    /// Neither this crate's own binary snapshot nor a well-formed ScottFree
    /// save: refused with a message, not a panic, and the live game is
    /// untouched.
    #[test]
    fn restore_rejects_garbage_matching_neither_format() {
        let dir = scratch("garbage-import");
        let mut vm = tiny_cave();
        let before = vm.room_block();
        let mut out: Vec<u8> = Vec::new();

        fs::write(dir.join("junk.sav"), b"this is not a scott save of either format").unwrap();

        assert!(!do_restore(&mut out, false, &mut vm, &dir, "junk"));
        assert_eq!(vm.room_block(), before, "the live game is untouched");

        let _ = fs::remove_dir_all(&dir);
    }

    /// An existing name is not silently clobbered. The answer is read from the
    /// caller, so this pins only that the WARNING is raised for the path
    /// `do_save` would write — the guard `cli_host::overwrite_warning` provides
    /// and `zvm-cli` already uses (SQ-0918).
    #[test]
    fn saving_over_an_existing_name_has_something_to_warn_about() {
        let dir = scratch("overwrite");
        let vm = tiny_cave();
        let mut out: Vec<u8> = Vec::new();
        do_save(&mut out, false, &vm, &dir, "twice");

        let path = cli_host::resolve_save_input("twice", &dir, cli_host::SCOTT_EXT);
        assert!(cli_host::overwrite_warning(&path).is_some(), "the second save must ask first");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn separator_rule_is_sized_to_the_room_block() {
        // `<` + em-dashes + `>`, as wide as the widest line (floor of 20).
        let rule = separator("I'm in a forest", false).expect("a rule outside plain mode");
        assert_eq!(rule.chars().count(), 20, "short block gets the minimum width");
        assert!(rule.starts_with('<') && rule.ends_with('>'), "bracketed: {rule:?}");

        let wide = separator(&"x".repeat(40), false).unwrap();
        assert_eq!(wide.chars().count(), 40, "grows to the block");
    }

    #[test]
    fn separator_is_absent_in_plain_mode() {
        // Thirty-odd em-dashes are pure decoration: a screen reader either
        // announces each one or drops the line entirely, and neither conveys the
        // window boundary it stands for (SQ-0608).
        assert_eq!(separator("I'm in a forest", true), None);
        assert_eq!(separator(&"x".repeat(60), true), None);
    }
}

#[cfg(test)]
mod help_width_tests {
    /// SQ-1093. One wrap authority across all four front-ends. The reported
    /// symptom was a `--help` showing two at once — prose hand-wrapped to about
    /// 83 columns beside a run that nothing measured — so the right margin read
    /// as a rendering fault rather than a layout choice. `cli_host::HELP_WIDTH`
    /// is the number; this is scott-cli's half of the pin.
    #[test]
    fn every_help_line_fits_the_one_width_all_four_front_ends_share() {
        let over = cli_host::overlong_help_lines(super::HELP);
        assert!(
            over.is_empty(),
            "--help must wrap at {}, but {over:?} do not:\n{}",
            cli_host::HELP_WIDTH,
            super::HELP
        );
        assert!(
            super::HELP
                .lines()
                .filter(|l| l.chars().count() > cli_host::HELP_WIDTH - 10)
                .count()
                > 5,
            "the text should be filling the width, not merely short of it"
        );
    }
}
