//! SQ-1104 — the first-run font check, end to end through the real files.
//!
//! The unit tests in `render::font_check_dialog` pin the QUESTION (two rows,
//! same slots, one glyph per source family); these pin the ANSWER, which has to
//! survive a round trip lanthorn does not control in one place: `toml_edit`
//! writes into the seeded `style.toml`, `style::parse_style_toml` reads it back,
//! and `SymbolSet::resolve` turns it into the glyphs the map draws. Any of the
//! three could be individually right and the chain still wrong.

use app::render::font_check_dialog::{ASSIST_LAMP, NERD_ARROWS, NERD_PORTALS};
use app::style::{load_style, style_write_path, write_font_check_answer};
use app::symbols::SymbolSet;

/// A throwaway lanthorn home, seeded exactly as a first launch seeds it.
fn seeded_home(name: &str) -> std::path::PathBuf {
    let dir = app::scratch_dir(&format!("fontcheck-{name}"));
    app::theme::template::auto_seed(&dir);
    assert!(dir.join("style.toml").is_file(), "the seed writes a style.toml");
    dir
}

/// Read the home's style file back the way startup does, and resolve the glyphs.
fn glyphs(dir: &std::path::Path) -> SymbolSet {
    SymbolSet::resolve(&badges(dir))
}

/// The same round trip, stopped one stage earlier: the picker's row badges are
/// `SymbolConfig` strings and never reach `SymbolSet` (SQ-1159).
fn badges(dir: &std::path::Path) -> app::config::SymbolConfig {
    let (doc, warnings) = load_style(None, dir);
    assert!(warnings.is_empty(), "the file we just wrote must parse clean: {warnings:?}");
    app::style::finalize_symbols(&doc.symbols)
}

/// The affirmative answer reaches the map: nerdfont arrows, the four distinct
/// stairs/door portal icons, and the lamp as the Guiding Light's mark.
#[test]
fn a_yes_installs_every_family_the_prompt_sampled() {
    let dir = seeded_home("yes");
    let path = style_write_path(None, &dir).expect("no pointer means the personal file");
    write_font_check_answer(&path, true, None).expect("writing the answer");

    let set = glyphs(&dir);
    let want_arrows = app::symbols::Arrows::preset(NERD_ARROWS).unwrap();
    let want_portal = app::symbols::PortalGlyphs::preset(NERD_PORTALS).unwrap();
    assert_eq!(set.arrows, want_arrows, "the arrow preset arrived");
    assert_eq!(set.portal.up, want_portal.up, "the stairs arrived");
    assert_eq!(set.portal.marker, want_portal.marker);
    assert_eq!(set.assist_gutter, ASSIST_LAMP, "the Guiding Light's mark is the lamp");
    assert_eq!(
        set.controls,
        app::symbols::ControlGlyphs::preset(app::render::font_check_dialog::NERD_CONTROLS).unwrap(),
        "the border toggle controls came with the rest (SQ-1123)",
    );

    // PRESET NAMES, not forty expanded per-slot overrides: the file has to stay
    // something a person can read and re-decide, and a later improvement to the
    // preset has to keep reaching them. Asked of the LIVE lines only — the seeded
    // template documents every slot name in comments, `arrow.north` included.
    let text = std::fs::read_to_string(&path).unwrap();
    let live: Vec<&str> = text.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
    assert!(
        live.iter().any(|l| l.contains(&format!("arrow_set = \"{NERD_ARROWS}\""))),
        "{text}"
    );
    assert!(
        live.iter().any(|l| l.contains(&format!("portal_icons = \"{NERD_PORTALS}\""))),
        "{text}"
    );
    assert!(
        live.iter().any(|l| l.contains("control_icons = \"nerdfont\"")),
        "{text}"
    );
    assert!(
        !live.iter().any(|l| l.contains("arrow.")),
        "the answer must not expand into per-slot overrides:\n{live:#?}"
    );
    assert_eq!(
        live.iter().filter(|l| l.contains("gutter.assist")).count(),
        1,
        "the one glyph with no preset of its own is the one override written"
    );
}

/// The negative answer is written too, not merely left unwritten — so a
/// re-check after a font change lands on the same two keys instead of leaving a
/// stale pair behind. And it takes the lamp back out with it.
#[test]
fn a_later_no_undoes_an_earlier_yes() {
    let dir = seeded_home("no-after-yes");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true, None).unwrap();
    write_font_check_answer(&path, false, None).unwrap();

    let set = glyphs(&dir);
    let plain = SymbolSet::default();
    assert_eq!(set.arrows, plain.arrows, "back to the filled triangles");
    assert_eq!(set.portal.up, plain.portal.up, "back to the plain portal icons");
    assert_eq!(
        set.assist_gutter, plain.assist_gutter,
        "and back to ● — NOT `*`, which Infocom games spend on footnotes"
    );
    assert_eq!(set.controls, plain.controls, "and back to the plain border controls");
}

/// A mark the user chose themselves is not ours to remove. The answer is about a
/// FONT; only the glyph the font check itself wrote is cleared by a later "no".
#[test]
fn a_no_leaves_a_gutter_mark_the_user_chose() {
    let dir = seeded_home("keeps-user-mark");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true, None).unwrap();

    // The user then picks their own mark, by hand, the way the file invites.
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text.replace(&ASSIST_LAMP.to_string(), "☼");
    std::fs::write(&path, text).unwrap();

    write_font_check_answer(&path, false, None).unwrap();
    assert_eq!(glyphs(&dir).assist_gutter, '☼', "their mark survives an answer about fonts");
}

/// Repeatable: answering twice the same way must not stack up duplicate keys or
/// a second `[map]` table. (`/run-font-check` exists to be run whenever a font
/// changes, so "again" is the normal case, not the edge one.)
#[test]
fn answering_twice_rewrites_rather_than_appends() {
    let dir = seeded_home("idempotent");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true, None).unwrap();
    let once = std::fs::read_to_string(&path).unwrap();
    write_font_check_answer(&path, true, None).unwrap();
    let twice = std::fs::read_to_string(&path).unwrap();
    assert_eq!(once, twice, "the second identical answer is a no-op on the text");
    let live = twice
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter(|l| l.contains("arrow_set = "))
        .count();
    assert_eq!(live, 1, "one live key, not two:\n{twice}");
}

/// The seed's comments are documentation the user needs; a settings write must
/// not eat them, exactly as `write_config_at` must not eat `config.toml`'s.
#[test]
fn the_seeded_commentary_survives_the_write() {
    let dir = seeded_home("comments");
    let path = style_write_path(None, &dir).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    write_font_check_answer(&path, true, None).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();

    let comments = |t: &str| t.lines().filter(|l| l.trim_start().starts_with('#')).count();
    assert_eq!(comments(&after), comments(&before), "every comment line survives");
    assert!(after.contains("[map.overrides]"), "and the override table's header with them");
}

/// **SQ-1148: a "yes" reaches the MAP pane's control cluster too**, end to end —
/// the dialog's answer, through `write_font_check_answer`, into `style.toml`, out
/// through the style loader and into the resolved `SymbolSet` the renderer reads.
///
/// The cluster deliberately has NO key of its own: it rides `control_icons`,
/// which the answer already writes for the border controls and the tooltip
/// pointer. So there is nothing new in the file to assert — and that absence IS
/// the assertion, because a key of its own is exactly how a config ends up
/// half-patched, with the story pane's controls iconised and the map pane's not,
/// and no way for the player to tell which question they answered.
///
/// This walks the whole chain rather than calling `MapControlGlyphs::preset`
/// directly: the unit case in `symbols.rs` already pins that the preset holds
/// the right codepoints, and what can still break is a link between here and
/// there.
#[test]
fn a_yes_reaches_the_map_controls_too() {
    use app::render::font_check_dialog::NERD_MAP_CONTROLS;

    let dir = seeded_home("map-controls");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true, None).unwrap();

    let want = app::symbols::MapControlGlyphs::preset(NERD_MAP_CONTROLS)
        .expect("the preset the answer names");
    let got = glyphs(&dir).map_controls;
    assert_eq!(got, want, "the map cluster is patched by the same yes");
    assert_eq!(got.room_numbers_on, '\u{F03A0}', "md-numeric reached the renderer");
    assert_eq!(got.room_numbers_off, '\u{F19D3}', "and so did its off-shape");

    // One key, not two: the map cluster must NOT have grown a
    // `map_control_icons` of its own on the way through.
    let text = std::fs::read_to_string(&path).unwrap();
    let live: Vec<&str> = text.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
    assert!(
        live.iter().any(|l| l.contains("control_icons = \"nerdfont\"")),
        "the one key the answer writes:\n{text}"
    );
    assert!(
        !live.iter().any(|l| l.trim_start().starts_with("map_control_icons")),
        "the map cluster grew a key of its own:\n{live:#?}"
    );

    // …and the plain answer leaves it plain, so this is a switch and not a
    // one-way door.
    let dir = seeded_home("map-controls-plain");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, false, None).unwrap();
    assert_eq!(
        glyphs(&dir).map_controls,
        app::symbols::SymbolSet::default().map_controls,
        "a no keeps the ASCII cluster"
    );
}

/// The story picker's row badges follow the answer too (SQ-1159).
///
/// They did not, for as long as the font check has existed: `arrow_set`,
/// `portal_icons` and `control_icons` were written from the answer and the
/// `badge_*` keys were not, so a player who said yes got patched glyphs
/// everywhere EXCEPT the picker. It is one key, in `[elements]` rather than
/// `[map]` — that is where the badges live, beside the selector that colours
/// them — and it round-trips through the same three stages as the rest.
#[test]
fn a_yes_reaches_the_picker_badges_too() {
    use app::render::font_check_dialog::NERD_BADGES;

    let dir = seeded_home("badges");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true, None).unwrap();

    let want = app::symbols::StoryBadges::preset(NERD_BADGES).expect("the preset the answer names");
    let cfg = badges(&dir);
    assert_eq!(cfg.badge_save, want.save.to_string(), "the save badge is patched");
    assert_eq!(cfg.badge_hint, want.hint.to_string());
    assert_eq!(cfg.badge_hint_available, want.hint_available.to_string());

    // A PRESET NAME, and one of them — not a glyph per badge. Expanding them
    // would freeze today's codepoints into the user's file and stop a later
    // improvement to the set from ever reaching them, which is the same reason
    // `[map]`'s three keys are names.
    let text = std::fs::read_to_string(&path).unwrap();
    let live: Vec<&str> = text.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
    assert!(
        live.iter().any(|l| l.contains(&format!("badge_icons = \"{NERD_BADGES}\""))),
        "{text}"
    );
    for key in ["badge_save", "badge_hint", "badge_hint_available"] {
        assert!(
            !live.iter().any(|l| l.trim_start().starts_with(key)),
            "the answer expanded into a per-badge key ({key}):\n{live:#?}"
        );
    }

    // …and a later "no" takes them back to the letters, like every other key the
    // answer writes.
    write_font_check_answer(&path, false, None).unwrap();
    let plain = app::symbols::StoryBadges::PLAIN;
    assert_eq!(badges(&dir).badge_save, plain.save.to_string(), "back to the letters");
    assert_eq!(badges(&dir).badge_hint_available, plain.hint_available.to_string());
}

/// A badge the user spelled themselves is not ours to move. The answer chooses a
/// SET; a key names one badge, and it outranks the set either way.
#[test]
fn a_badge_the_user_chose_survives_both_answers() {
    let dir = seeded_home("badge-kept");
    let path = style_write_path(None, &dir).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    // The seeded template documents the key commented out; the user uncomments it.
    let text = text.replace("# badge_save = \"S\"", "badge_save = \"★\"");
    std::fs::write(&path, &text).unwrap();

    write_font_check_answer(&path, true, None).unwrap();
    assert_eq!(badges(&dir).badge_save, "★", "their badge survives a yes");
    write_font_check_answer(&path, false, None).unwrap();
    assert_eq!(badges(&dir).badge_save, "★", "and a no");
}

/// A file that does not parse is the text the user has to READ to fix it.
/// Refuse, the way `config::write_config_at` refuses, rather than rewriting it.
#[test]
fn a_broken_style_file_is_refused_not_overwritten() {
    let dir = seeded_home("broken");
    let path = style_write_path(None, &dir).unwrap();
    std::fs::write(&path, "[map\narrow_set = oops\n").unwrap();
    let err = write_font_check_answer(&path, true, None).expect_err("a broken file must be refused");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[map\narrow_set = oops\n",
        "and left exactly as it was"
    );
}

/// `style = "default"` names the built-in style, which lives in the binary.
/// There is no file, so there is nothing to write and the caller must be told.
#[test]
fn the_builtin_style_has_no_file_to_write() {
    let dir = seeded_home("builtin");
    assert!(style_write_path(Some("default"), &dir).is_none());
    // Any other pointer resolves to a real path, relative to the home.
    assert_eq!(
        style_write_path(Some("mine.toml"), &dir),
        Some(dir.join("mine.toml")),
    );
}

/// The written file must ANSWER the prompt's own question: whatever the two
/// sample rows showed is what the two answers install. Pinned against the rows
/// rather than against literals, so the dialog and the writer cannot drift.
#[test]
fn the_rows_the_prompt_shows_are_the_glyphs_the_answers_install() {
    use app::render::font_check_dialog::sample_row;

    let dir = seeded_home("rows-match");
    let path = style_write_path(None, &dir).unwrap();

    // What the map draws after an answer. The diagonal corner stubs are NOT
    // here (SQ-1245) — they moved to their own question with their own writer
    // parameter, so `write_font_check_answer`'s `nerdfont` argument does not
    // install or change them and stage one's row no longer shows them; see
    // `stage_two_rows_match_what_each_answer_writes` for the diagonal
    // question's own version of this invariant.
    //
    // The map pane's own control cluster joins the rest at SQ-1148, and is
    // read off the RESOLVED set like everything else here — which is what
    // makes this the exact inverse of `font_check_dialog`'s
    // `every_map_glyph_a_yes_installs_appears_in_the_row_the_player_judges`.
    // That one says every glyph the answer INSTALLS is shown; this one says
    // every glyph SHOWN is installed. Neither implies the other, and the pair
    // is what holds the rule that the prompt asks about exactly what it
    // applies.
    let on_the_map = |set: &app::symbols::SymbolSet| {
        let m = &set.map_controls;
        [
            set.arrows.north, set.arrows.south, set.arrows.east, set.arrows.west,
            set.portal.marker, set.portal.up, set.portal.down,
            set.portal.in_, set.portal.out, set.portal.unknown,
            set.assist_gutter,
            m.room_numbers_on, m.room_numbers_off, m.centre,
            m.zoom_out, m.zoom_in, m.view_matrix, m.view_drawn,
        ]
    };

    write_font_check_answer(&path, true, None).unwrap();
    let set = glyphs(&dir);
    for ch in sample_row(true).chars().filter(|c| !c.is_whitespace()) {
        assert!(
            on_the_map(&set).contains(&ch),
            "row 1 showed {ch:?}, which the yes answer does not put on the map"
        );
    }

    write_font_check_answer(&path, false, None).unwrap();
    let set = glyphs(&dir);
    for ch in sample_row(false).chars().filter(|c| !c.is_whitespace()) {
        assert!(
            on_the_map(&set).contains(&ch),
            "row 2 showed {ch:?}, which the no answer does not put on the map"
        );
    }
}

// ── SQ-1245: the diagonal answer is a second, independent question ─────────

/// **The answer matrix.** Icons and diagonals are answered independently, so
/// BOTH crossed combinations must round-trip — not just "yes" agreeing with
/// "yes". This is the falsifying case for the whole feature: before the
/// write-side of SQ-1245 existed, `diagonal_corners` had no writer at all, so
/// `set.diagonal_corners` could only ever read back the compiled-in default
/// (`true`) — the "diagonals no" half below fails on that code every time,
/// which is what proves this case is actually exercising the new path rather
/// than passing by construction.
#[test]
fn the_diagonal_answer_is_independent_of_the_icon_answer() {
    // icons yes, diagonals no.
    let dir = seeded_home("icons-yes-diag-no");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true, Some(false)).unwrap();
    let set = glyphs(&dir);
    let want_arrows = app::symbols::Arrows::preset(NERD_ARROWS).unwrap();
    assert_eq!(set.arrows, want_arrows, "the icon presets still installed on a yes");
    assert!(!set.diagonal_corners, "diagonal_corners is off despite the icon yes");

    // icons no, diagonals yes.
    let dir = seeded_home("icons-no-diag-yes");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, false, Some(true)).unwrap();
    let set = glyphs(&dir);
    let plain = SymbolSet::default();
    assert_eq!(set.arrows, plain.arrows, "no icon presets installed on a no");
    assert!(set.diagonal_corners, "diagonal_corners is on despite the icon no");
}

/// A skipped stage two (Esc or the close box, `diagonal = None`) leaves
/// `map.diagonal_corners` exactly as it was — not reset to the compiled-in
/// default, and not touched at all — while the icon answer beside it still
/// applies normally.
#[test]
fn skipping_stage_two_leaves_diagonal_corners_untouched() {
    let dir = seeded_home("diag-skip-after-off");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true, Some(false)).unwrap();
    assert!(!glyphs(&dir).diagonal_corners, "set up: off");

    write_font_check_answer(&path, false, None).unwrap();
    assert!(!glyphs(&dir).diagonal_corners, "a skipped stage two must not flip it back on");
    assert_eq!(
        glyphs(&dir).arrows,
        SymbolSet::default().arrows,
        "the icon answer beside the skip still applies"
    );
}

/// The very first answer, with stage two skipped: no `diagonal_corners` key is
/// written at all — an absent key, not a written `true` — so the file states
/// only what was actually decided, matching how every other font-check key
/// behaves when its question goes unanswered.
#[test]
fn a_first_answer_that_skips_stage_two_writes_no_diagonal_key() {
    let dir = seeded_home("diag-never-answered");
    let path = style_write_path(None, &dir).unwrap();
    write_font_check_answer(&path, true, None).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let live: Vec<&str> = text.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
    assert!(
        !live.iter().any(|l| l.trim_start().starts_with("diagonal_corners")),
        "no key written when stage two is skipped:\n{live:#?}"
    );
    assert!(glyphs(&dir).diagonal_corners, "and the compiled-in default (on) still applies");
}

/// Stage two's own version of `the_rows_the_prompt_shows_are_the_glyphs_the_answers_install`:
/// whatever `diagonal_sample_row` showed is what the diagonal answer installs.
#[test]
fn stage_two_rows_match_what_each_answer_writes() {
    use app::render::font_check_dialog::diagonal_sample_row;

    let dir = seeded_home("diag-rows-match");
    let path = style_write_path(None, &dir).unwrap();

    write_font_check_answer(&path, true, Some(true)).unwrap();
    let set = glyphs(&dir);
    assert!(set.diagonal_corners);
    let installed = [set.path.diag_ul, set.path.diag_ur, set.path.diag_ll, set.path.diag_lr];
    for ch in diagonal_sample_row(true).chars().filter(|c| !c.is_whitespace()) {
        assert!(installed.contains(&ch), "row 1 showed {ch:?}, which a diagonal yes does not install");
    }

    write_font_check_answer(&path, true, Some(false)).unwrap();
    let set = glyphs(&dir);
    assert!(!set.diagonal_corners);
    let fallback = [set.path.nw, set.path.ne, set.path.sw, set.path.se];
    for ch in diagonal_sample_row(false).chars().filter(|c| !c.is_whitespace()) {
        assert!(fallback.contains(&ch), "row 2 showed {ch:?}, which a diagonal no does not fall back to");
    }
}

/// `--font-check` is a bare noun with a value, like `--sound` / `--images` /
/// `--accel` / `--guidance` — and it is declared on `Cli` in `app::config`,
/// which is what the TUI parses with. (`cli-host/src/args.rs` is the scanner for
/// the three headless CLI players and knows nothing about this flag.)
#[test]
fn the_flag_has_three_states_and_none_is_the_default() {
    use app::config::{Cli, OnOff};
    use clap::Parser;

    let parse = |args: &[&str]| Cli::try_parse_from(args).expect("parses");
    assert_eq!(parse(&["lanthorn", "story.z5"]).font_check, None, "absent = ask only on a first run");
    assert_eq!(parse(&["lanthorn", "--font-check", "on", "story.z5"]).font_check, Some(OnOff::On));
    assert_eq!(parse(&["lanthorn", "--font-check", "off", "story.z5"]).font_check, Some(OnOff::Off));
    // No `set-` prefix — that belongs to the slash command, whose registry
    // requires a verb.
    assert!(Cli::try_parse_from(["lanthorn", "--set-font-check", "on", "s.z5"]).is_err());
}

/// The slash spelling is verb-noun and takes no argument: the dialog IS the
/// question, so there is no second grammar to keep in step with the buttons.
#[test]
fn the_slash_command_is_verb_noun_and_argument_free() {
    use app::slash::{find_command, parse, Category, SlashOutcome};

    assert!(matches!(parse("run-font-check", '/'), SlashOutcome::RunFontCheck));
    assert!(matches!(parse("run-font-check anything", '/'), SlashOutcome::RunFontCheck));
    let spec = find_command("run-font-check").expect("registered");
    assert_eq!(spec.category, Category::Style);
    assert!(find_command("font-check").is_none(), "a bare noun is not a command name");
}

/// **The harness guard, as a guard rather than a convention** (SQ-1104).
///
/// "There is no config.toml" is the NORMAL state of a throwaway user-dir, and a
/// first run raises a modal that waits for a keypress — under a REAL pty, so no
/// tty check saves it. Falsified by removing the line: `pty_emitted_stream` still
/// reported PASS, and the prompt had quietly eaten one of its four Enters and
/// written `arrow_set = "filled"` into the harness's style.toml. A pty harness
/// can lose a keystroke and stay green, so nothing downstream will report this;
/// the line itself is what has to be defended.
#[test]
fn the_pty_driver_still_writes_a_config_for_every_harness() {
    let src = include_str!("../pty_stream/driver.rs");
    let run = src
        .split_once("pub fn run(spec: Spec)")
        .expect("the driver still has a `run`")
        .1;
    let head = &run[..run.len().min(2000)];
    assert!(
        head.contains("user_dir.join(\"config.toml\")"),
        "pty_stream::driver::run must seed a config.toml into every Spec's user_dir, \
         or a first-run font check blocks every pty harness at once"
    );
}
