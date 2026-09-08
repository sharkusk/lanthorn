//! SQ-0847: the input echo must stand on the same ground as the prose it is
//! typed into.
//!
//! Reported by the player on `stories/Zork Zero Disk.image` — **release 296,
//! serial 881019** — the moment SQ-0846 made the Macintosh's white page real:
//! *"the type prompt color when typing is white so i can't see what i'm typing,
//! after i press enter the text turns black."* And a second breath later, on the
//! disk's colour archive too: *"on mac zork 0 with default graphics (color) we
//! also have the same input text can't be seen."*
//!
//! **The same characters, rendered two ways, disagreeing.** Zork Zero on the
//! Macintosh never calls `set_colour` at all (SQ-0846 measured it, on both
//! archives and in both colour modes), so its white page comes from the MACHINE —
//! header `$2C`/`$2D`, `mac/xzip.lst`'s `SetColor := (zWHITE*256) + zBLACK`,
//! published to the renderer by `session::machine_screen_pair`. The prose path
//! reads that pair (`v6_machine_page` lays it under the transcript style), so the
//! game's own echo of a command is black on white. The live echo did not: it went
//! through `render::screen::game_input_style`, which asked the STORY WINDOW for a
//! pair, got `Default`/`Default` because no window ever declared one, and handed
//! the input line back to the theme — whose `input_text` derives from the `text`
//! role, which on the shipped dark palette is **white**. White ink, white page.
//!
//! So the deliverable here is not "the echo is black". It is that **the two paths
//! agree**: the identical characters at the identical columns must render
//! identically whether they are being typed or already committed. That is the
//! property that was violated, and unlike a colour constant it cannot drift back
//! without a test noticing.
//!
//! Everything below drives real gitignored media and skips vacuously without it.

use std::path::PathBuf;

use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The Macintosh volume, pinned to the build it carries — a disk image is a
/// different RELEASE, not the same story on other media.
const MAC_RELEASE: u16 = 296;
const MAC_SERIAL: &[u8] = b"881019";

fn mac_disk() -> Option<PathBuf> {
    let path = stories_dir().join("Zork Zero Disk.image");
    if !path.exists() {
        eprintln!("SKIP: gitignored Macintosh medium missing at {}", path.display());
        return None;
    }
    Some(path)
}

/// A booted session parked at its first ordinary line prompt, with everything the
/// story printed on the way there.
struct AtPrompt {
    session: GameSession,
    lines: Vec<String>,
    /// What `startup.rs` actually resolved for `honor_game_colours`, after the
    /// archive has had its say (SQ-0806/SQ-0846) — not what the caller asked for.
    honoured: bool,
}

/// Boot the Macintosh disk exactly as `startup.rs` does — optionally naming one of
/// its two archives by hand, as `--pictures` does — and drive it to the first
/// prompt.
fn mac_at_prompt(pictures: Option<&str>, honor_game_colours: bool) -> Option<AtPrompt> {
    let path = mac_disk()?;
    let bytes = match app::hints::load_story(&path).expect("Story.data mounts") {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code off the Macintosh volume, got {other:?}"),
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), MAC_RELEASE, "this disk carries r{MAC_RELEASE}");
    assert_eq!(&bytes[0x12..0x18], MAC_SERIAL, "…and that release's serial");

    // Four of this helper's five callers pass the same `pictures`, so the archive
    // name is no more a discriminator than the pid is (SQ-1131).
    let dir = app::scratch_dir(&format!("mac-echo-{}", pictures.unwrap_or("default")));
    let over = match pictures {
        Some(name) => PictureOverride::resolve_with_session(&path, &dir, Some(name)),
        None => PictureOverride::Unset,
    };
    let named_art_std_window = over.std_window();
    let profile = InterpreterProfile::resolve(&path, None, over.flavour(), None);
    assert_eq!(profile, InterpreterProfile::Macintosh, "an HFS volume is Apple's and nobody else's");
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let picture_dims = picts.all_pict_dims();
    let honoured = honor_game_colours
        && !picts.declines_game_colours(profile.default_colours());
    // SQ-1021/SQ-1022: every per-machine fact in one value, so this
    // harness cannot omit one — it was omitting the CELL.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        named_art_std_window,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut session = GameSession::new_for_machine(bytes, honoured, false, false, picture_dims, None, None, &boot)
    .expect("Zork Zero boots off the Macintosh disk");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    // Inline-prompt mode, which is what `command_bar = false` (the shipped
    // default, and the mode the report was filed in) makes `main.rs` do: keep the
    // game's own `>` in the transcript so the typed command can be appended to it.
    app::engine::Engine::set_strip_prompt(&mut session, false);
    let _ = std::fs::remove_dir_all(&dir);

    let lines = drive_to_prompt(&mut session, "Zork Zero r296 (Macintosh)");
    Some(AtPrompt { session, lines, honoured })
}

/// Tap through the boot ceremony until the story wants a typed line, keeping every
/// row it printed. That transcript is the reading surface the echo lands on.
fn drive_to_prompt(s: &mut GameSession, who: &str) -> Vec<String> {
    let mut lines: Vec<String> = s.take_transcript().split('\n').map(str::to_owned).collect();
    for _ in 0..40 {
        if matches!(s.pending_input(), InputKind::Line) {
            break;
        }
        let r = match s.pending_input() {
            InputKind::Char => s.submit_char(13),
            _ => s.submit(""),
        };
        lines.extend(r.transcript.split('\n').map(str::to_owned));
        assert!(!s.quit, "{who}: quit while driving to the prompt");
        assert!(s.machine.fault_trace.is_none(), "{who}: faulted while driving to the prompt");
    }
    assert!(matches!(s.pending_input(), InputKind::Line), "{who}: must reach a line prompt");
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    assert!(!lines.is_empty(), "{who}: the story must have printed something to type under");
    // Inline mode appends the typed command to the game's OWN `>` line, which is
    // why it keeps it (`main.rs` sets `strip_prompt` from `command_bar`). Without
    // that line there is nothing for either echo path to land on.
    assert_eq!(lines.last().map(String::as_str), Some(">"), "{who}: the game's prompt must be the last line");
    lines
}

/// A hybrid render at real kitty-ish cell metrics (8x18) — the shipped mode, and
/// the one the report was filed against — with the host transcript in it and a
/// command either being TYPED at the live prompt or already COMMITTED onto the
/// game's own `>` line, which is what `turn.rs` does with the echo in inline mode.
///
/// `colors` is the player's theme, so a case can hand in one that names the input
/// line's own colours.
#[allow(deprecated)]
fn render_echo(
    at: &AtPrompt,
    honour: bool,
    typed: &str,
    committed: bool,
    colors: app::colors::ColorScheme,
    command_bar: bool,
) -> (Rect, Buffer) {
    use app::engine::Engine as _;
    let model = at.session.screen();
    let mut state = app::state::AppState::default();
    state.colors = colors;
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honour;
    state.config.command_bar = command_bar;
    for line in &at.lines {
        state.push_transcript_kind(line, app::state::TranscriptKind::Story);
    }
    if committed {
        state.append_to_last_transcript_line(typed);
    } else {
        state.input.set(typed, true);
    }
    let area = Rect::new(0, 0, 120, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    (area, buf)
}

/// One drawn cell, as a string so a failure prints what it measured.
type CellLook = (String, String, String);

fn look(buf: &Buffer, x: u16, y: u16) -> CellLook {
    let c = buf.cell((x, y)).expect("in-bounds cell");
    (format!("{:?}", c.fg), format!("{:?}", c.bg), format!("{:?}", c.modifier))
}

/// The look of every cell of `needle` where it was drawn, plus the one cell after
/// it (the caret). Panics if the text never reached the pane, because a case that
/// measures nothing must fail loudly rather than pass vacuously.
fn span_look(area: Rect, buf: &Buffer, needle: &str) -> Vec<CellLook> {
    let row = |y: u16| -> String {
        (0..area.width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect()
    };
    for y in 0..area.height {
        let text = row(y);
        if let Some(byte_at) = text.find(needle) {
            let x0 = text[..byte_at].chars().count() as u16;
            let x1 = (x0 + needle.chars().count() as u16).min(area.width - 1);
            return (x0..=x1).map(|x| look(buf, x, y)).collect();
        }
    }
    // Dump the pane before dying: "never drawn" is the most expensive kind of
    // render failure to diagnose from a bare message.
    for y in 0..area.height {
        eprintln!("{y:>3} |{}|", row(y));
    }
    panic!("{needle:?} was never drawn into the pane");
}

/// A theme with the input line's own colours named by hand, as a player's
/// `style.toml` would name them.
fn theme_naming_the_input_line(fg: ratatui::style::Color) -> app::colors::ColorScheme {
    use app::theme::registry::Delta;
    let mut colors = app::colors::ColorScheme::terminal_default();
    let mut decls: app::theme::resolve::Decls = Default::default();
    for sel in ["input_text", "input_prompt"] {
        decls.insert(sel.to_string(), Delta { fg: Some(fg), ..Delta::EMPTY });
    }
    colors.theme = app::theme::resolve::resolve(
        &app::theme::resolve::Roles::terminal_default(),
        &decls,
        &Default::default(),
        &Default::default(),
    );
    colors
}

const TYPED: &str = "look";

// ── The deliverable ──────────────────────────────────────────────────────────

/// **THE DELIVERABLE — SQ-0847, as reported.** On the Macintosh, the characters
/// being typed and the same characters once committed must render identically.
///
/// Both of the disk's archives, because the player hit it on both: the two-colour
/// `Pic.data` he reported first and the colour `CPic.data` the disk loads on its
/// own. The page is the machine's either way — it comes off header `$2C`/`$2D`,
/// not out of the artwork — so the archive must make no difference at all, and
/// this case says so by measuring both.
///
/// The assertion is the RELATION, not a colour: `>look` typed and `>look`
/// committed are the same glyphs at the same columns, and every cell of the span
/// (the game's `>`, the four typed letters, the caret past them) must carry the
/// same ink, page and modifiers in both renders.
///
/// FALSIFIED by reverting the machine-ground fallback in
/// `render::screen::game_input_style`: the live span comes back as the theme's
/// `input_text` — white, on the machine's white page — while the committed span
/// stays black, which is the report verbatim.
#[test]
fn the_macintosh_types_in_the_same_ink_it_commits_in() {
    if mac_disk().is_none() {
        return;
    }
    for archive in [None, Some("Pic.data"), Some("CPic.data")] {
        let Some(at) = mac_at_prompt(archive, true) else { return };
        assert!(at.honoured, "{archive:?}: this machine keeps its own colours (SQ-0846)");

        let (area, live) = render_echo(&at, true, TYPED, false, app::colors::ColorScheme::terminal_default(), false);
        let (_, done) = render_echo(&at, true, TYPED, true, app::colors::ColorScheme::terminal_default(), false);
        let needle = format!(">{TYPED}");
        let live_span = span_look(area, &live, &needle);
        let done_span = span_look(area, &done, &needle);
        assert_eq!(
            live_span, done_span,
            "{archive:?}: the echo and the committed text must render the same characters the same way",
        );

        // …and what they agree ON is the machine's own pair, so the case cannot be
        // satisfied by both paths going equally wrong. Black ink on a white page,
        // as `mac/xzip.lst` states them.
        let palette = app::colors::ColorScheme::terminal_default().palette;
        let white = palette[usize::from(app::interpreter::MAC_DEFAULT_BACKGROUND) - 2];
        let black = palette[usize::from(app::interpreter::MAC_DEFAULT_FOREGROUND) - 2];
        // The typed letters are span[1..=4]; span[0] is the game's `>` and the
        // last is the caret, which reverses the pair rather than carrying it flat.
        for cell in &live_span[1..=TYPED.chars().count()] {
            assert_eq!(
                (cell.0.as_str(), cell.1.as_str()),
                (format!("{black:?}").as_str(), format!("{white:?}").as_str()),
                "{archive:?}: a typed character must be the machine's black on its white, got {cell:?}",
            );
        }
    }
}

/// The symptom itself, stated as the thing that must be false: not one cell of the
/// live echo may be drawn in the theme's `text` ink on the machine's page.
///
/// This is the report in the player's own terms — *"white so i can't see what i'm
/// typing"* — and it is worth its own case because the agreement above would also
/// be satisfied by both paths turning white together, which would be a different
/// and equally broken screen.
#[test]
fn nothing_typed_on_the_machines_page_is_drawn_in_the_themes_ink() {
    if mac_disk().is_none() {
        return;
    }
    let colors = app::colors::ColorScheme::terminal_default();
    let theme_ink = format!("{:?}", colors.theme.get("input_text").style.fg.expect("the theme names an ink"));
    for archive in [Some("Pic.data"), Some("CPic.data")] {
        let Some(at) = mac_at_prompt(archive, true) else { return };
        let (area, buf) = render_echo(&at, true, TYPED, false, colors.clone(), false);
        let span = span_look(area, &buf, &format!(">{TYPED}"));
        assert!(
            span[1..=TYPED.chars().count()].iter().all(|c| c.0 != theme_ink),
            "{archive:?}: the typed line is still the theme's {theme_ink} on the machine's page — {span:?}",
        );
    }

    // …and the command-bar mode draws its own input row through a second call
    // site, so it gets the same guarantee rather than inheriting one by luck.
    let Some(at) = mac_at_prompt(Some("Pic.data"), true) else { return };
    let (area, buf) = render_echo(&at, true, TYPED, false, colors, true);
    let span = span_look(area, &buf, &format!("> {TYPED}"));
    assert!(
        span[2..].iter().take(TYPED.chars().count()).all(|c| c.0 != theme_ink),
        "the command bar's typed line is still the theme's {theme_ink} on the machine's page — {span:?}",
    );
}

// ── The guards ───────────────────────────────────────────────────────────────

/// `honor_game_colours = false` must be a no-op here, exactly as SQ-0846 made it
/// at the source: a machine's own page is still a game colour, and the switch that
/// declines them declines this too. With colours off the typed line goes back to
/// the theme's `input_text` and no machine page reaches it.
///
/// Pinned per the project's colour convention, and load-bearing rather than
/// ceremonial: the whole of SQ-0846's design was keeping that switch meaningful.
#[test]
fn with_game_colours_declined_the_typed_line_is_the_themes_own() {
    if mac_disk().is_none() {
        return;
    }
    let colors = app::colors::ColorScheme::terminal_default();
    let theme_ink = format!("{:?}", colors.theme.get("input_text").style.fg.expect("the theme names an ink"));
    for archive in [Some("Pic.data"), Some("CPic.data")] {
        let Some(at) = mac_at_prompt(archive, false) else { return };
        assert!(!at.honoured, "{archive:?}: the player's own switch still turns them off");
        let (area, buf) = render_echo(&at, false, TYPED, false, colors.clone(), false);
        let span = span_look(area, &buf, &format!(">{TYPED}"));
        for cell in &span[1..=TYPED.chars().count()] {
            assert_eq!(
                cell.0, theme_ink,
                "{archive:?}: colours declined — the theme owns the input line, got {span:?}",
            );
        }
    }
}

/// **A player who named the input line's colours still gets them.** The machine's
/// page is a DEFAULT — the ground under a channel nobody claimed — and a
/// `style.toml` that claims it outranks a default. `set_colour` is the other
/// case and keeps winning: there the game asked, and an honoured game colour has
/// always beaten the theme's input fields.
///
/// FALSIFY by dropping the `input_line_is_themed` gate: the machine's black
/// overwrites the magenta the player asked for.
#[test]
fn an_explicitly_themed_input_line_wins_over_the_machines_page() {
    if mac_disk().is_none() {
        return;
    }
    let magenta = ratatui::style::Color::Magenta;
    let colors = theme_naming_the_input_line(magenta);
    assert_ne!(
        app::colors::ColorScheme::terminal_default().theme.get("input_text").style.fg,
        Some(magenta),
        "premise: magenta is not what the shipped theme would have chosen",
    );
    for archive in [Some("Pic.data"), Some("CPic.data")] {
        let Some(at) = mac_at_prompt(archive, true) else { return };
        let (area, buf) = render_echo(&at, true, TYPED, false, colors.clone(), false);
        let span = span_look(area, &buf, &format!(">{TYPED}"));
        for cell in &span[1..=TYPED.chars().count()] {
            assert_eq!(
                cell.0,
                format!("{magenta:?}"),
                "{archive:?}: the player named the input line's ink and must keep it, got {span:?}",
            );
        }
    }
}
