//! Configurable keymap — `KeySpec`, `Context`, and `KeyMap`.
//!
//! Commands are identified by their registry command-string (see `crate::slash`).
//! `KeySpec` is a parsed keystroke (key code + modifier flags). `Context`
//! partitions bindings into Global, Map, Anim and Browser layers — the first
//! three are in-game, and Browser is the pre-game story browser, which runs
//! before there is an `AppState` and so shares nothing with them (SQ-0796).
//! `KeyMap` holds the full binding table and exposes lookup / resolve /
//! primary-key queries.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ── Context ────────────────────────────────────────────────────────────────────

/// Which dispatch layer a binding belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Context {
    /// Reached in any focus when no prompt or anim sub-mode is active.
    Global,
    /// Map-focus bindings (also fall through to Global on miss).
    Map,
    /// Tidy-animation sub-mode (does NOT fall through).
    Anim,
    /// The pre-game story browser (SQ-0796). Its own world: it runs before there
    /// is an `AppState`, so nothing here falls through to Global and no Global
    /// binding reaches it — a game command has nothing to act on there.
    Browser,
}

// ── KeySpec ────────────────────────────────────────────────────────────────────

/// A parsed keystroke: key code plus modifier flags.
///
/// Equality is **canonical**, not field-by-field (SQ-0653): a spec parsed from
/// config text and the live crossterm event it is meant to match describe the
/// same keystroke differently, and a raw field comparison made whole classes of
/// binding unmatchable. See [`KeySpec::canonical`].
#[derive(Clone, Copy, Debug)]
pub struct KeySpec {
    pub code: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Uppercase `c` when that is a single character; otherwise leave it alone.
///
/// `char::to_uppercase` is a 1→N mapping ('ß' → "SS"), and a key spec holds one
/// `char` — so a multi-char expansion has no canonical form and stays as it is.
fn upper1(c: char) -> char {
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

impl KeySpec {
    /// Normalize a live crossterm `KeyEvent` into a `KeySpec` for lookups.
    pub fn from_key_event(k: KeyEvent) -> KeySpec {
        KeySpec {
            code: k.code,
            ctrl: k.modifiers.contains(KeyModifiers::CONTROL),
            shift: k.modifiers.contains(KeyModifiers::SHIFT),
            alt: k.modifiers.contains(KeyModifiers::ALT),
        }
        .canonical()
    }

    /// The canonical form of this keystroke — the single representation both a
    /// config-parsed spec and a live terminal event reduce to (SQ-0653).
    ///
    /// Three rules, applied symmetrically to *both* sides of every comparison:
    ///
    /// - **Letters carry their shift in the case.** `FromStr` lowercases the
    ///   whole spec, so `"shift+s"` parsed to `Char('s')` + shift while every
    ///   terminal delivers `Char('S')` + SHIFT — the binding could never match.
    ///   Canonically a shifted letter is uppercase *and* keeps `shift`, so both
    ///   spellings (and a bare `"S"`) meet at `Char('S')` + shift.
    /// - **Other characters drop shift.** A terminal reports the *produced*
    ///   character, so Shift+`=` arrives as `Char('+')` + SHIFT while the spec
    ///   `"+"` has no modifier; the shift is already encoded in the glyph.
    /// - **Tab encodes shift as `BackTab`.** `"shift+tab"` and `"backtab"` both
    ///   parse to `BackTab`, but terminals deliver `BackTab` *with* SHIFT set —
    ///   so the flag is stripped, and a `Tab` + shift event folds to `BackTab`.
    ///
    /// Ctrl and Alt are untouched: they never change the character a terminal
    /// reports.
    pub fn canonical(self) -> KeySpec {
        let KeySpec { code, ctrl, shift, alt } = self;
        let (code, shift) = match code {
            KeyCode::Char(c) if c.is_alphabetic() => {
                if shift || c.is_uppercase() {
                    (KeyCode::Char(upper1(c)), true)
                } else {
                    (KeyCode::Char(c), false)
                }
            }
            KeyCode::Char(c) => (KeyCode::Char(c), false),
            KeyCode::BackTab => (KeyCode::BackTab, false),
            KeyCode::Tab if shift => (KeyCode::BackTab, false),
            other => (other, shift),
        };
        KeySpec { code, ctrl, shift, alt }
    }

    /// Human-readable label for the hint bar / help screen.
    /// Examples: "Ctrl+S", "Shift+←", "h", "F1", "Space", "Shift+Tab".
    pub fn label(&self) -> String {
        // BackTab is always "Shift+Tab" regardless of modifier flags.
        if self.code == KeyCode::BackTab {
            let mut s = String::new();
            if self.ctrl { s.push_str("Ctrl+"); }
            if self.alt { s.push_str("Alt+"); }
            s.push_str("Shift+Tab");
            return s;
        }
        let mut s = String::new();
        if self.ctrl { s.push_str("Ctrl+"); }
        if self.alt { s.push_str("Alt+"); }
        if self.shift { s.push_str("Shift+"); }
        let key_str = match self.code {
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => unreachable!("handled above"),
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PgUp".to_string(),
            KeyCode::PageDown => "PgDn".to_string(),
            KeyCode::F(n) => format!("F{n}"),
            KeyCode::Char(c) => c.to_uppercase().to_string(),
            _ => format!("{:?}", self.code),
        };
        s.push_str(&key_str);
        s
    }

    /// The tuple two specs are compared on: canonical code + modifier flags.
    fn eq_key(&self) -> (KeyCode, bool, bool, bool) {
        let c = self.canonical();
        (c.code, c.ctrl, c.shift, c.alt)
    }
}

/// Canonical equality (SQ-0653). Implemented by hand rather than derived so that
/// EVERY comparison site — `KeyMap::lookup`, the `resolve` de-dup `retain`, the
/// run loop's hotkey-prefix check — normalizes both sides, instead of each site
/// having to remember to. It is a proper equivalence relation (equality of
/// `canonical()`), so the `Eq` below is sound. `KeySpec` deliberately derives no
/// `Hash`: a hash would have to hash the canonical form to stay consistent.
impl PartialEq for KeySpec {
    fn eq(&self, other: &Self) -> bool {
        self.eq_key() == other.eq_key()
    }
}

impl Eq for KeySpec {}

impl std::str::FromStr for KeySpec {
    type Err = String;

    /// Parse a key spec string like "ctrl+s", "shift+left", "+", "f1", "space".
    ///
    /// Modifiers (ctrl, shift, alt) may appear in any order before the key
    /// token, separated by '+'. A lone '+' character parses as Char('+').
    fn from_str(s: &str) -> Result<KeySpec, String> {
        let lower = s.trim().to_lowercase();

        // Special case: a lone "+" (would otherwise produce empty tokens when split).
        if lower == "+" {
            return Ok(KeySpec { code: KeyCode::Char('+'), ctrl: false, shift: false, alt: false });
        }

        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key_token: Option<String> = None;

        let parts: Vec<&str> = lower.split('+').collect();
        let n = parts.len();

        // Walk tokens: modifier keywords consume early slots; the last
        // non-empty, non-modifier token is the key. Handle "+" embedded in
        // the split: a trailing empty part after '+' means '+' was the last char.
        let mut i = 0;
        while i < n {
            let p = parts[i].trim();
            match p {
                "ctrl" | "control" => { ctrl = true; i += 1; }
                "shift" => { shift = true; i += 1; }
                "alt" => { alt = true; i += 1; }
                "" => {
                    // An empty segment from split means a literal '+' was there.
                    // E.g. "shift++" splits as ["shift", "", ""] — the key is '+'.
                    // We treat the first empty token after modifiers as the '+' key.
                    key_token = Some("+".to_string());
                    i += 1;
                }
                other => {
                    key_token = Some(other.to_string());
                    i += 1;
                }
            }
        }

        let tok = key_token.ok_or_else(|| format!("empty key spec: '{s}'"))?;

        let code = match tok.as_str() {
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "tab" => {
                // "shift+tab" parses as shift=true + token "tab"; map to BackTab.
                if shift {
                    shift = false; // BackTab encodes the shift itself
                    KeyCode::BackTab
                } else {
                    KeyCode::Tab
                }
            }
            "backtab" => KeyCode::BackTab,
            "space" => KeyCode::Char(' '),
            "esc" | "escape" => KeyCode::Esc,
            "enter" | "return" => KeyCode::Enter,
            "backspace" => KeyCode::Backspace,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" | "pgdown" => KeyCode::PageDown,
            "f1" => KeyCode::F(1),
            "f2" => KeyCode::F(2),
            "f3" => KeyCode::F(3),
            "f4" => KeyCode::F(4),
            "f5" => KeyCode::F(5),
            "f6" => KeyCode::F(6),
            "f7" => KeyCode::F(7),
            "f8" => KeyCode::F(8),
            "f9" => KeyCode::F(9),
            "f10" => KeyCode::F(10),
            "f11" => KeyCode::F(11),
            "f12" => KeyCode::F(12),
            s if s.chars().count() == 1 => KeyCode::Char(s.chars().next().unwrap()),
            other => return Err(format!("unknown key token: '{other}'")),
        };

        // Canonicalize on the way out (SQ-0653): the spec text was lowercased
        // wholesale above, so "shift+s" would otherwise store Char('s') and never
        // match the Char('S') + SHIFT the terminal delivers. Storing the canonical
        // form also keeps `label()` honest ("Shift+S", not "Shift+s").
        Ok(KeySpec { code, ctrl, shift, alt }.canonical())
    }
}

// ── KeyMap ─────────────────────────────────────────────────────────────────────

/// The full binding table. Each entry is `(KeySpec, String, Context)`.
/// Multiple specs may map to the same command string (multi-bind defaults).
#[derive(Debug)]
pub struct KeyMap {
    pub bindings: Vec<(KeySpec, String, Context)>,
}

impl Default for KeyMap {
    /// Build the default keymap from today's `key_to_action` dispatch.
    ///
    /// This is the single source of truth for back-compat. Every binding here
    /// must match `input.rs` exactly.
    fn default() -> Self {
        use KeyCode::*;

        // Shorthand constructors.
        let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
        let plain = |code| g(code, false, false);
        let ctrl = |code| g(code, true, false);

        let mut b: Vec<(KeySpec, String, Context)> = Vec::new();

        macro_rules! bind {
            ($spec:expr, $cmd:expr, $ctx:expr) => {
                b.push(($spec, $cmd.to_string(), $ctx));
            };
        }

        // ── Global ────────────────────────────────────────────────────────────
        // Tab → toggle-focus (the Tab KEY itself stays hardwired in key_to_action;
        // this entry lets the keymap advertise it for hints/help).
        bind!(plain(Tab), "toggle-focus", Context::Global);

        // NO F-KEY HAS A DEFAULT BINDING, and that is the whole rule (SQ-1142).
        //
        // F2 opened the command panel (SQ-0664), F3 entered pane-resize mode
        // (SQ-0669) and F4 lit the word reveal (SQ-1107). All three are gone as
        // DEFAULTS because they were never ours to claim: a v4+ story may
        // declare a terminating-characters table at header $2E, and Infocom's
        // V6 titles use it — Arthur lists F1-F6, so pressing F2 for its map is
        // a read the STORY handles. A host that intercepts the key eats input
        // the game explicitly asked for, and the player sees a command panel
        // instead of the map they asked their game for.
        //
        // What this does NOT do: `KeySpec` still parses "f1".."f12", so a player
        // who wants one of these keys may bind it in their own config and
        // accept the trade knowingly. The three commands are untouched in
        // `slash::COMMANDS` — `toggle-command-panel`, `resize-panes` and
        // `reveal-words` are all still reachable by name, by the Ctrl+P leader
        // panel where one has a letter, and by the pane-border controls that
        // click them (SQ-1123). Only the default keymap gave them up.
        //
        // Two alternatives were weighed and rejected: letting the story win only
        // while it is READING, keyed off $2E — the highest-fidelity answer, the
        // most machinery, and it makes one keystroke mean different things in
        // different stories; and rebinding the three to other keys, which keeps
        // a default shortcut and spends three more keys on it.

        bind!(ctrl(Char('s')), "save-state", Context::Global);
        bind!(ctrl(Char('r')), "restore-state", Context::Global);

        // ── Map ───────────────────────────────────────────────────────────────
        // Deliberately EMPTY of defaults since SQ-0599. `Context::Map` used to
        // carry the map pane's own key set — plain arrows/hjkl to pan, +/-/0 to
        // zoom, c/n/p to centre and select — reachable only while the map held
        // the keyboard. That focus mode is gone: the same keystroke meaning two
        // different things with no on-screen cue was the whole complaint.
        //
        // The context itself stays, because `[keymap.map]` is a documented
        // config surface and a user may still bind to it; it is now reached
        // only while the debug inspector holds the right-hand pane. Everything
        // the map needs is modeless instead — Shift+Arrow pans from anywhere
        // (see `game_key_to_action`), the mouse pans/zooms/selects, and zoom
        // and centring live in the leader panel's "Map" group below.

        // ── Anim ──────────────────────────────────────────────────────────────
        // Pan in anim: hjkl + shift-arrows. Plain arrows step the animation stage,
        // so shift+Arrow is the arrow-key path for panning during playback
        // (SQ-0416), matching the Map-focus shift-arrow pan.
        bind!(plain(Char('h')), "pan-map -1 0", Context::Anim);
        bind!(plain(Char('l')), "pan-map 1 0", Context::Anim);
        bind!(plain(Char('k')), "pan-map 0 -1", Context::Anim);
        bind!(plain(Char('j')), "pan-map 0 1", Context::Anim);

        bind!(g(Left, false, true), "pan-map -1 0", Context::Anim);
        bind!(g(Right, false, true), "pan-map 1 0", Context::Anim);
        bind!(g(Up, false, true), "pan-map 0 -1", Context::Anim);
        bind!(g(Down, false, true), "pan-map 0 1", Context::Anim);

        // Zoom in anim
        bind!(plain(Char('+')), "zoom-map in", Context::Anim);
        bind!(plain(Char('=')), "zoom-map in", Context::Anim);
        bind!(plain(Char('-')), "zoom-map out", Context::Anim);

        // Step / play / exit
        bind!(plain(Left), "anim-step back", Context::Anim);
        bind!(plain(Right), "anim-step forward", Context::Anim);
        bind!(plain(Char(' ')), "anim-play", Context::Anim);
        bind!(plain(Esc), "anim-exit", Context::Anim);
        bind!(plain(Enter), "anim-exit", Context::Anim);

        // ── Browser ───────────────────────────────────────────────────────────
        // The pre-game story browser (SQ-0796). These are the keys the picker
        // used to match on directly; they are data now, so `[keymap.browser]`
        // can move any of them and the footer hints follow (`crate::browser`).
        //
        // ORDER MATTERS for those hints: a command's first binding is its
        // rank-0 key, which is what a one-rank hint shows and what leads a
        // multi-rank one — hence arrows before hjkl, and Shift-Enter before `o`.
        bind!(plain(Up), "move-selection 0 -1", Context::Browser);
        bind!(plain(Char('k')), "move-selection 0 -1", Context::Browser);
        bind!(plain(Down), "move-selection 0 1", Context::Browser);
        bind!(plain(Char('j')), "move-selection 0 1", Context::Browser);
        // Horizontal movement means something only in the cover gallery; in the
        // list these are bound but inert, exactly as they were.
        bind!(plain(Left), "move-selection -1 0", Context::Browser);
        bind!(plain(Char('h')), "move-selection -1 0", Context::Browser);
        bind!(plain(Right), "move-selection 1 0", Context::Browser);
        bind!(plain(Char('l')), "move-selection 1 0", Context::Browser);
        bind!(plain(PageUp), "page-selection -1", Context::Browser);
        bind!(plain(PageDown), "page-selection 1", Context::Browser);
        // Half-page paging, the vim Ctrl-U/Ctrl-D convention (SQ-1228).
        bind!(ctrl(Char('u')), "half-page-selection -1", Context::Browser);
        bind!(ctrl(Char('d')), "half-page-selection 1", Context::Browser);
        bind!(plain(Home), "select-edge first", Context::Browser);
        bind!(plain(End), "select-edge last", Context::Browser);

        // Enter plays; Shift modifies the default action rather than introducing
        // a mode (SQ-0789), and `o` is the same command on a key every terminal
        // can deliver — Shift-Enter needs the kitty keyboard protocol to be
        // distinguishable from Enter at all. `o` LEADS since SQ-1227: it is the
        // key the story menu's own row advertises, and a menu that names a
        // gesture half its readers' terminals cannot produce is worse than one
        // that names the plain letter.
        bind!(plain(Enter), "play-story", Context::Browser);
        bind!(plain(Char('o')), "open-launch-options", Context::Browser);
        bind!(g(Enter, false, true), "open-launch-options", Context::Browser);
        // Space opens the per-story menu — everything that acts on ONE story,
        // in one place, instead of five separate footer hints (SQ-1227).
        bind!(plain(Char(' ')), "open-story-menu", Context::Browser);
        // `?` shows the browser's own key reference, which is what lets the
        // footer stop advertising the keys it no longer has room for.
        bind!(plain(Char('?')), "show-browser-keys", Context::Browser);

        // Tab LEADS (SQ-1227): the footer names one key per hint, and `Tab` is
        // the one every other pane in lanthorn already uses to swap a panel in.
        bind!(plain(Tab), "toggle-info-panel", Context::Browser);
        bind!(plain(Char('i')), "toggle-info-panel", Context::Browser);
        bind!(plain(Char('g')), "toggle-gallery", Context::Browser);
        bind!(plain(Char('f')), "fetch-story", Context::Browser);
        bind!(plain(Char('r')), "refresh-library", Context::Browser);
        bind!(plain(Char('u')), "set-ifdb-url", Context::Browser);
        bind!(plain(Char('/')), "search-ifdb", Context::Browser);
        bind!(g(Char('U'), false, true), "open-url", Context::Browser);
        bind!(g(Char('H'), false, true), "download-hints", Context::Browser);
        bind!(plain(Char('s')), "sort-library", Context::Browser);
        bind!(plain(Char('d')), "reverse-sort", Context::Browser);
        // Ctrl+F filters the library's in-memory index; Backspace climbs out of
        // a sub-folder. Both are inert with nothing to act on (a flat library,
        // the root), and neither collides with a letter the picker already uses.
        bind!(g(Char('f'), true, false), "find-story", Context::Browser);
        bind!(plain(Backspace), "parent-folder", Context::Browser);
        bind!(plain(Char('q')), "quit-browser", Context::Browser);
        bind!(plain(Esc), "cancel-browser", Context::Browser);

        KeyMap { bindings: b }
    }
}

impl KeyMap {
    /// Look up a key in the given context.
    ///
    /// - `Context::Map` also searches `Context::Global` on miss (fall-through).
    /// - `Context::Global` and `Context::Anim` do not fall through.
    pub fn lookup(&self, spec: &KeySpec, ctx: Context) -> Option<&str> {
        // Exact context match first.
        for (s, cmd, c) in &self.bindings {
            if c == &ctx && s == spec {
                return Some(cmd.as_str());
            }
        }
        // Map falls through to Global.
        if ctx == Context::Map {
            for (s, cmd, c) in &self.bindings {
                if c == &Context::Global && s == spec {
                    return Some(cmd.as_str());
                }
            }
        }
        None
    }

    /// Return the first (primary) `KeySpec` whose command string starts with `command_name`.
    pub fn primary_key(&self, command_name: &str) -> Option<KeySpec> {
        self.bindings.iter()
            .find(|(_, s, _)| s.split_whitespace().next() == Some(command_name))
            .map(|(spec, _, _)| *spec)
    }

    /// The first `KeySpec` bound to this WHOLE entry, argument and all (SQ-1148).
    ///
    /// [`Keymap::primary_key`] matches on the command's name, which is what a
    /// caller asking "is `zoom-map` reachable at all" wants and is a wrong answer
    /// for a caller asking "what key runs `zoom-map out`" — `+` is bound to
    /// `zoom-map in` and comes first, so the name-matching lookup answers `+` for
    /// both directions. A border control that supplies an argument asks this one.
    pub fn primary_key_exact(&self, entry: &str) -> Option<KeySpec> {
        self.bindings.iter().find(|(_, s, _)| s == entry).map(|(spec, _, _)| *spec)
    }

    /// Iterate all `(KeySpec, &str)` pairs that belong to `ctx`
    /// (for the help screen's per-context listing).
    pub fn for_context(&self, ctx: Context) -> impl Iterator<Item = (&KeySpec, &str)> {
        self.bindings.iter()
            .filter(move |(_, _, c)| *c == ctx)
            .map(|(s, cmd, _)| (s, cmd.as_str()))
    }

    /// The registry command `token` names, if it names one (SQ-0759).
    ///
    /// Used only to explain a rejected `[keymap.*]` entry: a token that is a
    /// command name sitting where a key belongs means the line is inverted, not
    /// that the key is unspellable. Underscores are accepted as well as hyphens
    /// because the config template used to advertise the snake_case spelling.
    fn command_name_hint(token: &str) -> Option<&'static str> {
        let kebab = token.replace('_', "-");
        crate::slash::find_command(&kebab).map(|c| c.name)
    }

    /// Build a keymap from config overrides.
    ///
    /// Returns the resolved `KeyMap` and a list of warning strings for
    /// overrides that were rejected (unknown name, parse error, conflict).
    pub fn resolve(cfg: &crate::config::KeymapConfig) -> (KeyMap, Vec<String>) {
        let mut km = if cfg.use_defaults { KeyMap::default() } else { KeyMap { bindings: Vec::new() } };
        let mut warnings = Vec::new();
        for (ctx, section) in [
            (Context::Global, &cfg.global),
            (Context::Map, &cfg.map),
            (Context::Anim, &cfg.anim),
            (Context::Browser, &cfg.browser),
        ] {
            for (key, command) in section {
                let spec = match key.parse::<KeySpec>() {
                    Ok(s) => s,
                    Err(e) => {
                        // SQ-0759: the left-hand side is the KEY and the right-hand
                        // side the command, and a user who writes the pair the other
                        // way round was told their *key* was unparseable — true, but
                        // it names the wrong half of the line. Say what actually
                        // happened when the token is recognisably a command name.
                        warnings.push(match Self::command_name_hint(key) {
                            Some(name) => format!(
                                "keymap: '{key} = \"{command}\"' is written backwards — the entry is \
                                 key = \"command\", so write '{command} = \"{name}\"'; skipped"
                            ),
                            None => format!("keymap: cannot parse key '{key}': {e}; skipped"),
                        });
                        continue;
                    }
                };
                let cmd_name = command.split_whitespace().next().unwrap_or("");
                // The browser's commands and the game's cannot be bound into each
                // other's context — the dispatcher refuses the pairing either way
                // (SQ-0796), so binding one would be a key that silently does
                // nothing. Say so here instead.
                if let Some(spec) = crate::slash::find_command(cmd_name) {
                    let in_browser = ctx == Context::Browser;
                    if (spec.context == Context::Browser) != in_browser {
                        warnings.push(if in_browser {
                            format!("keymap: '{command}' is a game command and cannot be bound in [keymap.browser]; skipped")
                        } else {
                            format!("keymap: '{command}' is a story-browser command — bind it in [keymap.browser]; skipped")
                        });
                        continue;
                    }
                }
                if crate::slash::find_command(cmd_name).is_none() {
                    // The registry spells its names with hyphens; the template used
                    // to say snake_case, so say which spelling to use rather than
                    // leaving the user to guess (SQ-0759).
                    warnings.push(match Self::command_name_hint(cmd_name) {
                        Some(name) => format!(
                            "keymap: unknown command '{command}' — the registry spells it \
                             '{name}'; skipped"
                        ),
                        None => format!("keymap: unknown command '{command}'; skipped"),
                    });
                    continue;
                }
                km.bindings.retain(|(s, _, c)| !(*s == spec && *c == ctx));
                km.bindings.push((spec, command.clone(), ctx));
            }
        }
        (km, warnings)
    }
}

// ── HotkeyLayout ──────────────────────────────────────────────────────────────

/// Default full command-strings for the direct (always-available) command set.
const DEFAULT_DIRECT_COMMANDS: &[&str] = &[
    "quit",
    "save-state",
    "restore-state",
    "pan-map -1 0",
    "pan-map 1 0",
    "pan-map 0 -1",
    "pan-map 0 1",
    "zoom-map in",
    "zoom-map out",
    "zoom-map reset",
    "select-room next",
    "select-room prev",
    "center-map",
    "toggle-focus",
    // SQ-0759: a diagnostic that can only be reached through a modal reports the
    // modal. Opening the palette drops a v6 pane off its pixel path, and coming
    // back re-uploads every chrome band — so the palette route churns the render
    // history and the upload count that `/dump-windows` exists to print. It has no
    // default binding (it is a debugging command, not a play key), but a user who
    // binds one needs it to actually fire: `is_direct_name` gates every Ctrl
    // binding and the whole Map context, and Ctrl is the only class of key the
    // run loop's char-mode gate lets past a story waiting on a keypress.
    "dump-windows",
    // SQ-0761, and for exactly the same reason: a cell dump taken through the
    // palette describes the palette. Worse here than for `/dump-windows`, because
    // this one reports the CELLS — the modal is drawn over them, so its frame is
    // not a stale answer but a different picture entirely.
    "dump-cells",
    // SQ-0994, and the rationale bites hardest here: `/dump-terminal` REPORTS the
    // traffic counters, and reaching it through the palette is itself traffic —
    // a modal drops a v6 pane off the pixel path, and coming back re-uploads every
    // chrome band. Bytes-per-frame taken that way describes the palette's frame.
    // Like its two siblings it has no default binding; this only lets a Ctrl one
    // the user writes actually fire past a story waiting on a keypress.
    "dump-terminal",
];

/// Default groups for the hotkey dialog (title, authored leader-key + full command-string).
///
/// Mnemonic-first layout (SQ-0446, "Proposal B"): 15 frequent map-editing verbs,
/// each on its natural letter, in five function-named groups. `q` is deliberately
/// unassigned so a bare `q` closes the dialog (universal quit/close convention).
/// The long tail (exports, rename-layer, toggle-map, toggle-inspector,
/// toggle-alignment, pane sizing, …) is reachable through the `/` command palette.
/// Entries are `(leader letter, command-string, panel label)`. An empty label
/// falls back to the slash command's own description.
///
/// Every default entry carries a label, for two reasons. A registry description
/// documents every argument form the *slash* command accepts ("zoom the map
/// in/out, reset, or step by signed n") — identical across sibling entries and
/// untrue of each, since a panel entry runs one fixed command and cannot take
/// an argument. And descriptions are written as full sentences that run well
/// past the panel's width, so they were being cut off mid-word at the border.
/// These are sized to fit: keep them short enough to survive the panel's
/// 60-column cap (see `render::hotkeys`).
/// One authored default entry: `(leader letter, command-string, panel label)`.
type DefaultEntry = (char, &'static str, &'static str);
/// One authored default group: title plus its entries.
type DefaultGroup = (&'static str, &'static [DefaultEntry]);

// ── The panel's ORDER is the panel's argument ────────────────────────────────
//
// Session first, because it is what a reader who has just opened this panel is
// most often after — settings, or starting over — and it is the one group that
// is not about the map. Everything after it IS about the map, which is why the
// four map groups run together and say so in their titles: the renderer draws a
// flat `## title` per group with no nesting (`render::hotkeys`), so a hierarchy
// can only be spelled, not indented.
//
// `Layout` is GONE, and with it the panel's only rows for `tidy-map` and
// `animate-tidy`. They were not earning a heading of their own — the layout
// re-tidies itself continuously, so asking for a pass by hand is a thing almost
// nobody does. Both remain commands: `/tidy-map` and `/animate-tidy` still work,
// and `t`/`a` are now free letters if something wants them.
//
// `open-history` is gone from Session for a sharper reason. `record_turn_history`
// is opt-in and defaults to false, and `Action::OpenHistory` is a deliberate
// no-op when the history is empty (`input.rs`) — so for every player who has not
// turned recording on, that row did nothing at all and said nothing about why.
// A menu entry that cannot work should not be offered. `/open-history` still
// opens it for anyone who has the setting on.
const DEFAULT_GROUPS: &[DefaultGroup] = &[
    ("Session", &[('s', "open-settings", "global settings"), ('g', "reset-game", "restart game")]),
    // SQ-0599: zoom and centring used to be plain +/- and c while the map held
    // the keyboard. With that focus mode gone they would otherwise be
    // mouse-only, so they live here — on the keys they always used, which keeps
    // the muscle memory intact now that a leader press precedes them.
    ("Map", &[('+', "zoom-map in", "zoom in"), ('-', "zoom-map out", "zoom out"), ('0', "center-map", "centre on selection")]),
    ("Map · Layers", &[('p', "move-region new", "region into a new layer"), ('m', "move-region parent", "region into the parent layer"), ('c', "cycle-layer next", "next map layer"), ('z', "mark-maze-layer", "flag layer as a maze")]),
    ("Map · Edit", &[('r', "rename-room", "rename room"), ('n', "edit-notes", "edit room notes"), ('d', "delete-connection", "delete connection"), ('e', "relabel-edge", "relabel edge")]),
    ("Map · View", &[('i', "toggle-inventory-panel", "inventory panel"), ('l', "toggle-portal-labels", "portal labels"), ('v', "toggle-command-panel", "command panel"), ('u', "view-map", "drawn / matrix view"), ('k', "toggle-room-panel", "room panel")]),
];

/// One leader-panel entry: `(leader letter, command-string, optional label)`.
///
/// The label overrides the command's registry description when the panel draws
/// the row; `None` falls back to that description. See [`DEFAULT_GROUPS`] for
/// why an override is needed at all.
pub type HotkeyEntry = (char, String, Option<String>);

/// One leader-panel group: its title plus its entries.
pub type HotkeyGroup = (String, Vec<HotkeyEntry>);

/// Runtime layout for the hotkey dialog.
///
/// Controls which key opens the dialog (`prefix`), which commands are always
/// reachable without the dialog (`direct`), and how commands are grouped inside
/// the dialog (`groups`).
#[derive(Debug)]
pub struct HotkeyLayout {
    /// The key that opens (and closes) the dialog.
    pub prefix: KeySpec,
    /// Full command-strings that are always available without opening the dialog.
    pub direct: std::collections::HashSet<String>,
    /// Groups of commands shown in the dialog: (group title, [(leader letter, command-string)]).
    /// The panel's groups. See [`HotkeyEntry`] and [`DEFAULT_GROUPS`].
    pub groups: Vec<HotkeyGroup>,
}

impl Default for HotkeyLayout {
    /// Build the built-in default layout.
    fn default() -> Self {
        let prefix: KeySpec = "ctrl+p".parse().expect("ctrl+p must parse");

        let direct = DEFAULT_DIRECT_COMMANDS.iter().map(|s| s.to_string()).collect();

        let groups = DEFAULT_GROUPS
            .iter()
            .map(|(title, entries)| {
                let entries = entries
                    .iter()
                    .map(|(letter, cmd, label)| {
                        let label = (!label.is_empty()).then(|| label.to_string());
                        (*letter, cmd.to_string(), label)
                    })
                    .collect();
                (title.to_string(), entries)
            })
            .collect();

        HotkeyLayout { prefix, direct, groups }
    }
}

impl HotkeyLayout {
    /// Resolve a `HotkeyLayout` from config, producing warnings for unknown command names.
    ///
    /// Fields that are `None` in the config use the built-in defaults.
    pub fn resolve(cfg: &crate::config::HotkeysConfig) -> (HotkeyLayout, Vec<String>) {
        let mut layout = HotkeyLayout::default();
        let mut warnings: Vec<String> = Vec::new();

        // Override prefix if specified.
        if let Some(prefix_str) = &cfg.prefix {
            match prefix_str.parse::<KeySpec>() {
                Ok(spec) => layout.prefix = spec,
                Err(e) => warnings.push(format!("hotkeys: prefix '{}': {e}; using default", prefix_str)),
            }
        }

        // Override direct set if specified. Each entry is a full command-string;
        // its first token is validated against the registry.
        if let Some(direct_cmds) = &cfg.direct {
            let mut direct_set = std::collections::HashSet::new();
            for cmd in direct_cmds {
                let name = cmd.split_whitespace().next().unwrap_or("");
                if crate::slash::find_command(name).is_some() {
                    direct_set.insert(cmd.clone());
                } else {
                    warnings.push(format!("hotkeys: direct: unknown command '{cmd}'; skipped"));
                }
            }
            layout.direct = direct_set;
        }

        // Override groups if any are specified.
        if !cfg.group.is_empty() {
            let mut groups: Vec<HotkeyGroup> = Vec::new();
            let mut used_letters: std::collections::HashSet<char> = std::collections::HashSet::new();

            for g in &cfg.group {
                // User-authored entries carry no label — the panel falls back
                // to each command's registry description for them.
                let mut cmds: Vec<HotkeyEntry> = Vec::new();
                for entry in &g.commands {
                    let tokens: Vec<&str> = entry.split_whitespace().collect();
                    if tokens.is_empty() {
                        continue;
                    }

                    // Try letter-prefixed form, e.g. "t tidy-map".
                    let mut parsed: Option<(char, String)> = None;
                    if tokens[0].chars().count() == 1 && tokens.len() > 1 {
                        let letter = tokens[0].chars().next().unwrap();
                        if crate::slash::find_command(tokens[1]).is_some() {
                            parsed = Some((letter, tokens[1..].join(" ")));
                        }
                    }

                    let (letter, cmd) = if let Some(lc) = parsed {
                        lc
                    } else {
                        // Whole entry is the command-string; auto-assign a free letter.
                        if crate::slash::find_command(tokens[0]).is_none() {
                            warnings.push(format!("hotkeys: group '{}': unknown command '{entry}'; dropped", g.title));
                            continue;
                        }
                        match ('a'..='z').find(|c| !used_letters.contains(c)) {
                            Some(letter) => (letter, entry.clone()),
                            None => {
                                warnings.push(format!("hotkeys: group '{}': no free letter for '{entry}'; dropped", g.title));
                                continue;
                            }
                        }
                    };

                    if used_letters.contains(&letter) {
                        warnings.push(format!("hotkeys: group '{}': letter '{}' already used; dropped '{}'", g.title, letter, cmd));
                        continue;
                    }
                    used_letters.insert(letter);
                    cmds.push((letter, cmd, None));
                }
                groups.push((g.title.clone(), cmds));
            }
            layout.groups = groups;
        }

        (layout, warnings)
    }

    /// Return the command-string bound to leader letter `key`, if any.
    pub fn leader_command(&self, key: char) -> Option<&str> {
        self.groups.iter()
            .flat_map(|(_, cmds)| cmds.iter())
            .find(|(letter, _, _)| *letter == key)
            .map(|(_, cmd, _)| cmd.as_str())
    }

    /// Check whether a full keymap command-string resolves to a direct command.
    ///
    /// `cmd_str` is the full binding string as returned by `KeyMap::lookup`
    /// (e.g. `"zoom-map in"`, `"save-state"`). Matched as a whole against the
    /// direct set, so a command with arguments is matched exactly (e.g.
    /// `"zoom-map in"` is direct but `"zoom-map out"` is matched separately).
    pub fn is_direct_name(&self, cmd_str: &str) -> bool {
        self.direct.contains(cmd_str)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Action;

    // Task 2: KeySpec parsing and labels
    #[test]
    fn keyspec_parse_and_label_roundtrip() {
        let s: KeySpec = "ctrl+s".parse().unwrap();
        assert_eq!((s.ctrl, s.code), (true, KeyCode::Char('s')));
        assert_eq!("shift+left".parse::<KeySpec>().unwrap().code, KeyCode::Left);
        assert_eq!("+".parse::<KeySpec>().unwrap().code, KeyCode::Char('+'));
        assert_eq!("f1".parse::<KeySpec>().unwrap().code, KeyCode::F(1));
        assert_eq!("space".parse::<KeySpec>().unwrap().code, KeyCode::Char(' '));
        assert!("nope".parse::<KeySpec>().is_err());
        assert_eq!("ctrl+s".parse::<KeySpec>().unwrap().label(), "Ctrl+S");
    }

    // ── SQ-0653: shifted bindings must match the events terminals deliver ──────

    /// `FromStr` lowercases the whole spec, so "shift+d" used to store
    /// `Char('d')` + shift while crossterm delivers `Char('D')` + SHIFT — the
    /// binding could never fire. Canonical equality folds both to `Char('D')`.
    #[test]
    fn shift_letter_spec_matches_the_uppercase_event_a_terminal_sends() {
        let spec: KeySpec = "shift+d".parse().unwrap();
        let event = KeySpec::from_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
        assert_eq!(spec, event, "'shift+d' must match Char('D') + SHIFT");
        // Some terminals (and Caps Lock) send the capital with no SHIFT flag.
        let bare_capital = KeySpec::from_key_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::NONE));
        assert_eq!(spec, bare_capital, "'shift+d' must match a bare Char('D')");
        // …and a plain lowercase 'd' is still a DIFFERENT key.
        let plain = KeySpec::from_key_event(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_ne!(spec, plain, "'shift+d' must not swallow a plain 'd'");
        // The binding resolves through a real KeyMap lookup, not just ==.
        let km = KeyMap {
            bindings: vec![(spec, "tidy-map".to_string(), Context::Global)],
        };
        assert_eq!(km.lookup(&event, Context::Global), Some("tidy-map"));
        assert_eq!(spec.label(), "Shift+D");
    }

    /// Both spellings of Shift+Tab must match `BackTab` whether or not the
    /// terminal also sets SHIFT (most do; the spec forms never did).
    #[test]
    fn shift_tab_and_backtab_specs_match_backtab_with_or_without_shift() {
        let with_shift = KeySpec::from_key_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        let without = KeySpec::from_key_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE));
        for text in ["shift+tab", "backtab"] {
            let spec: KeySpec = text.parse().unwrap();
            assert_eq!(spec, with_shift, "'{text}' must match BackTab + SHIFT");
            assert_eq!(spec, without, "'{text}' must match a bare BackTab");
            let km = KeyMap { bindings: vec![(spec, "toggle-focus".to_string(), Context::Global)] };
            assert_eq!(km.lookup(&with_shift, Context::Global), Some("toggle-focus"));
            assert_eq!(spec.label(), "Shift+Tab");
        }
        // A terminal that reports Tab + SHIFT instead of BackTab lands there too.
        let tab_shift = KeySpec::from_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT));
        assert_eq!("shift+tab".parse::<KeySpec>().unwrap(), tab_shift);
        // Plain Tab stays plain Tab.
        let tab = KeySpec::from_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_ne!("shift+tab".parse::<KeySpec>().unwrap(), tab);
    }

    /// A punctuation key already encodes its shift in the glyph the terminal
    /// reports, so the stray SHIFT flag must not defeat a plain "+" binding.
    #[test]
    fn punctuation_specs_ignore_the_shift_flag_the_terminal_adds() {
        let spec: KeySpec = "+".parse().unwrap();
        let plus_shifted = KeySpec::from_key_event(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::SHIFT));
        assert_eq!(spec, plus_shifted, "Shift+= delivers Char('+') + SHIFT");
        let km = KeyMap::default();
        assert_eq!(km.lookup(&plus_shifted, Context::Anim), Some("zoom-map in"));
    }

    // Task 3a: default keymap matches today's bindings
    #[test]
    fn default_keymap_matches_todays_bindings() {
        let km = KeyMap::default();
        let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
        use KeyCode::*;
        assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Global), Some("save-state"));
        // Map falls through to Global:
        assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Map), Some("save-state"));
    }

    /// SQ-0599: the map pane no longer takes the keyboard, so the key set that
    /// only worked while it held focus is gone. These keys are ordinary typing
    /// now — a bare `h` belongs in the command line, not panning the map.
    #[test]
    fn the_map_context_ships_no_default_bindings() {
        let km = KeyMap::default();
        let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
        use KeyCode::*;
        for spec in [
            g(Char('h'), false, false),
            g(Char('j'), false, false),
            g(Char('n'), false, false),
            g(Char('p'), false, false),
            g(Char('c'), false, false),
            g(Char('+'), false, false),
            g(Char('-'), false, false),
            g(Char('0'), false, false),
            g(Left, false, false),
            g(Esc, false, false),
        ] {
            assert_eq!(
                km.lookup(&spec, Context::Map),
                None,
                "{spec:?} must not be a map-context default any more"
            );
        }
        // Global bindings still reach through the map context's fallthrough.
        assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Map), Some("save-state"));
        assert_eq!(km.lookup(&g(Char('r'), true, false), Context::Map), Some("restore-state"));
    }

    /// SQ-0796: the browser is its own context and does NOT fall through to
    /// Global, because nothing Global does has anything to act on before a story
    /// is loaded. Its own keys resolve; the game's do not reach it, and its keys
    /// do not leak into the game.
    #[test]
    fn the_browser_context_stands_alone() {
        let km = KeyMap::default();
        let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
        use KeyCode::*;
        assert_eq!(km.lookup(&g(Enter, false, false), Context::Browser), Some("play-story"));
        assert_eq!(
            km.lookup(&g(Enter, false, true), Context::Browser),
            Some("open-launch-options")
        );
        // Ctrl+S saves in the game; in the browser there is nothing to save.
        assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Browser), None);
        // …and `g` opens the cover gallery only there.
        assert_eq!(km.lookup(&g(Char('g'), false, false), Context::Global), None);
        assert_eq!(km.lookup(&g(Char('g'), false, false), Context::Map), None);
    }

    /// SQ-1228: Ctrl-U/Ctrl-D are the vim half-page convention, bound by default
    /// in the story picker's list view (Browser context) only — the picker has
    /// no readline prompt to conflict with, unlike the story transcript's
    /// Ctrl-D (hardwired in `input.rs`; see `ctrl_d_half_pages_the_transcript_in_game_focus`).
    #[test]
    fn ctrl_u_and_ctrl_d_half_page_the_browser_list() {
        let km = KeyMap::default();
        let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
        use KeyCode::*;
        assert_eq!(
            km.lookup(&g(Char('u'), true, false), Context::Browser),
            Some("half-page-selection -1")
        );
        assert_eq!(
            km.lookup(&g(Char('d'), true, false), Context::Browser),
            Some("half-page-selection 1")
        );
        // Not bound in any other context.
        assert_eq!(km.lookup(&g(Char('u'), true, false), Context::Global), None);
        assert_eq!(km.lookup(&g(Char('d'), true, false), Context::Global), None);
        assert_eq!(km.lookup(&g(Char('u'), true, false), Context::Map), None);
        assert_eq!(km.lookup(&g(Char('d'), true, false), Context::Map), None);
    }

    /// SQ-0796: binding across the two worlds is refused with a warning rather
    /// than accepted into a key that could only ever do nothing.
    #[test]
    fn resolve_refuses_a_command_from_the_other_world() {
        let mut cfg = crate::config::KeymapConfig::default();
        cfg.browser.insert("ctrl+w".into(), "quit".into());
        cfg.global.insert("ctrl+w".into(), "sort-library".into());
        let (km, warnings) = KeyMap::resolve(&cfg);
        assert_eq!(warnings.len(), 2, "both directions warn: {warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("game command")), "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("[keymap.browser]")), "{warnings:?}");
        let spec: KeySpec = "ctrl+w".parse().unwrap();
        assert_eq!(km.lookup(&spec, Context::Browser), None, "the binding was skipped");
        assert_eq!(km.lookup(&spec, Context::Global), None, "and so was the other one");
    }

    /// SQ-0796: a browser binding from config layers onto the defaults exactly as
    /// a Global one does.
    #[test]
    fn resolve_layers_browser_overrides_onto_the_defaults() {
        let mut cfg = crate::config::KeymapConfig::default();
        cfg.browser.insert("f5".into(), "refresh-library".into());
        let (km, warnings) = KeyMap::resolve(&cfg);
        assert!(warnings.is_empty(), "{warnings:?}");
        let f5: KeySpec = "f5".parse().unwrap();
        assert_eq!(km.lookup(&f5, Context::Browser), Some("refresh-library"));
        // The shipped `r` survives — an override adds, it does not replace.
        let r = KeySpec { code: KeyCode::Char('r'), ctrl: false, shift: false, alt: false };
        assert_eq!(km.lookup(&r, Context::Browser), Some("refresh-library"));
    }

    // ── HotkeyLayout tests ────────────────────────────────────────────────────

    #[test]
    fn hotkey_layout_default_direct_and_indirect() {
        let layout = HotkeyLayout::default();
        // Direct commands
        assert!(layout.is_direct_name("center-map"), "center-map should be direct");
        assert!(layout.is_direct_name("quit"), "quit should be direct");
        assert!(layout.is_direct_name("toggle-focus"), "toggle-focus should be direct");
        // Non-direct (dialog-only) commands
        assert!(!layout.is_direct_name("tidy-map"), "tidy-map should NOT be direct");
        assert!(!layout.is_direct_name("open-settings"), "open-settings should NOT be direct");
        // Groups
        assert_eq!(layout.groups.len(), 5, "Layout was removed; Session + Map + its three sub-groups remain");
        assert_eq!(layout.groups[0].0, "Session", "Session leads the panel — it is the group that is not about the map");
    }

    #[test]
    fn hotkey_layout_resolve_custom_direct_and_unknown_name() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: Some(vec!["save-state".into(), "quit".into(), "not-a-command".into()]),
            group: vec![HotkeyGroupConfig { title: "T".into(), commands: vec!["tidy-map".into()] }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        // Specified direct commands are direct
        assert!(layout.is_direct_name("save-state"), "save-state should be direct");
        assert!(layout.is_direct_name("quit"), "quit should be direct");
        // center-map is NOT in custom direct list
        assert!(!layout.is_direct_name("center-map"), "center-map should NOT be direct with custom list");
        // Unknown command produces a warning
        assert!(!warnings.is_empty(), "unknown command in direct should produce warning");
        assert!(warnings.iter().any(|w| w.contains("not-a-command")), "warning should mention not-a-command");
    }

    #[test]
    fn hotkey_layout_resolve_unknown_group_command_dropped() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: None,
            group: vec![HotkeyGroupConfig {
                title: "MyGroup".into(),
                commands: vec!["tidy-map".into(), "totally-fake-cmd".into()],
            }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        assert_eq!(layout.groups.len(), 1);
        assert_eq!(layout.groups[0].1.len(), 1, "unknown command should be dropped from group");
        assert_eq!(layout.groups[0].1[0].1, "tidy-map");
        assert!(!warnings.is_empty(), "unknown group command should produce warning");
        assert!(warnings.iter().any(|w| w.contains("totally-fake-cmd")));
    }

    #[test]
    fn backtab_cycles_focus_back_by_default() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use crate::input::{key_to_action, Action};
        use crate::state::AppState;

        let mut state = AppState::default();
        // BackTab is typically delivered with no modifiers.
        let backtab = KeyEvent {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        // With no mid-word suggestions the autocomplete intercept doesn't apply, so
        // Shift-Tab reverses the per-window focus cycle (the mirror of Tab).
        state.focus = crate::state::Focus::Game;
        let action = key_to_action(&state, backtab);
        assert!(
            matches!(action, Action::CycleFocusBack),
            "BackTab in Game focus should reverse focus, got {:?}",
            action
        );

        state.focus = crate::state::Focus::Map;
        let action_map = key_to_action(&state, backtab);
        assert!(
            matches!(action_map, Action::CycleFocusBack),
            "BackTab in Map focus should reverse focus, got {:?}",
            action_map
        );
    }

    #[test]
    fn backtab_keyspec_label_is_shift_tab() {
        let spec = KeySpec { code: KeyCode::BackTab, ctrl: false, shift: false, alt: false };
        assert_eq!(spec.label(), "Shift+Tab");
    }

    #[test]
    fn shift_tab_parses_to_backtab() {
        let spec: KeySpec = "shift+tab".parse().unwrap();
        assert_eq!(spec.code, KeyCode::BackTab);
        // shift flag should be false (BackTab encodes the shift itself)
        assert!(!spec.shift);
    }

    #[test]
    fn backtab_token_parses_to_backtab() {
        let spec: KeySpec = "backtab".parse().unwrap();
        assert_eq!(spec.code, KeyCode::BackTab);
    }

    #[test]
    fn reset_game_key_f5_unbound_by_default() {
        // reset-game is leader-only now (SQ-0202); F5 has no default binding.
        let km = KeyMap::default();
        let f5 = KeySpec { code: KeyCode::F(5), ctrl: false, shift: false, alt: false };
        assert_eq!(km.lookup(&f5, Context::Global), None);
    }

    /// `open-history` is reachable, and is NOT offered in the leader panel.
    ///
    /// It was in Session until it was noticed that it does nothing for almost
    /// everyone: `record_turn_history` defaults to false, and `OpenHistory` is a
    /// silent no-op on an empty history. A row that cannot work and cannot say
    /// why is worse than no row. The command itself is untouched — `/open-history`
    /// still opens the modal once recording is on.
    #[test]
    fn open_history_is_a_command_but_not_a_panel_row() {
        // Leader-only since SQ-0202, so no key reaches it. This used to assert
        // that F4 was unbound, which was never the point — F4 was merely where
        // `open-history` did not live, and SQ-1107 put the word reveal there.
        // What matters is that no default binding reaches `open-history` at all.
        let km = KeyMap::default();
        let f4 = KeySpec { code: KeyCode::F(4), ctrl: false, shift: false, alt: false };
        assert_ne!(km.lookup(&f4, Context::Global), Some("open-history"));
        assert_eq!(
            km.primary_key("open-history"),
            None,
            "open-history is leader-only: no default key may reach it",
        );
        let layout = HotkeyLayout::default();
        assert!(
            !layout.groups.iter().any(|(_, cmds)| cmds.iter().any(|c| c.1 == "open-history")),
            "open-history must not be offered in the leader panel: it is a no-op unless \
             record_turn_history is on, and the panel gives no way to say so"
        );
        assert!(
            crate::slash::COMMANDS.iter().any(|c| c.name == "open-history"),
            "the command itself must remain — this removed a panel row, not a feature"
        );
    }

    #[test]
    fn reset_game_in_session_dialog_group() {
        // SQ-0446: reset-game moved from Files to the Session group, letter 'g'.
        let layout = HotkeyLayout::default();
        let session_group = layout.groups.iter().find(|(title, _)| title == "Session");
        assert!(session_group.is_some(), "Session group should exist");
        let (_, cmds) = session_group.unwrap();
        assert!(cmds.iter().any(|c| c.1 == "reset-game"), "reset-game should be in Session group");
    }

    #[test]
    fn toggle_inventory_key_v_unbound_by_default() {
        // toggle-inventory is leader-only now (SQ-0202); v has no default binding.
        let km = KeyMap::default();
        let spec = KeySpec { code: KeyCode::Char('v'), ctrl: false, shift: false, alt: false };
        let cmd = km.lookup(&spec, Context::Global);
        assert_eq!(cmd, None, "v should not be bound to toggle-inventory by default");
    }

    #[test]
    fn toggle_inventory_in_view_group() {
        let layout = HotkeyLayout::default();
        let view_group = layout.groups.iter().find(|(title, _)| title == "Map \u{b7} View");
        assert!(view_group.is_some(), "View group should exist");
        let (_, cmds) = view_group.unwrap();
        assert!(cmds.iter().any(|c| c.1 == "toggle-inventory-panel"), "toggle-inventory-panel should be in View group");
        // SQ-0692: the room panel is a View-group toggle too — the popups it replaced
        // were mouse-only, which is why nobody found the diagnostics view.
        assert!(cmds.iter().any(|c| c.1 == "toggle-room-panel"), "toggle-room-panel should be in View group");
    }

    #[test]
    fn apply_action_toggle_inventory_flips_bool() {
        use mapper::mapper::Mapper;
        use crate::input::apply_action;
        use crate::state::AppState;
        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        assert!(!state.show_inventory);
        apply_action(Action::ToggleInventory, &mut state, &mut mapper);
        assert!(state.show_inventory);
        apply_action(Action::ToggleInventory, &mut state, &mut mapper);
        assert!(!state.show_inventory);
    }

    // ── Item 2: ZoomReset command wiring ─────────────────────────────────────

    /// Zoom and centring kept their keys but moved behind the leader (SQ-0599):
    /// the map pane no longer takes focus, so a bare `0` is typing, and these
    /// commands would otherwise be mouse-only.
    #[test]
    fn zoom_and_centre_are_reachable_through_the_leader_panel() {
        let layout = HotkeyLayout::default();
        assert_eq!(layout.leader_command('+'), Some("zoom-map in"));
        assert_eq!(layout.leader_command('-'), Some("zoom-map out"));
        assert_eq!(layout.leader_command('0'), Some("center-map"));
        assert_eq!(layout.leader_command('f'), None, "centring moved onto '0'");

        let km = KeyMap::default();
        let zero = KeySpec { code: KeyCode::Char('0'), ctrl: false, shift: false, alt: false };
        assert_eq!(
            km.lookup(&zero, Context::Map),
            None,
            "'0' is no longer a map-focus binding — there is no map focus"
        );
    }

    #[test]
    fn zoom_reset_is_in_direct_set() {
        let layout = HotkeyLayout::default();
        assert!(
            layout.is_direct_name("zoom-map reset"),
            "zoom-map reset must be in the direct set (accessible without the hotkey dialog)"
        );
    }

    #[test]
    fn zoom_reset_action_resets_level() {
        use mapper::mapper::Mapper;
        use crate::input::apply_action;
        use crate::state::{AppState, Zoom};
        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        // Zoom all the way out first.
        for _ in 0..8 {
            apply_action(Action::ZoomOut, &mut state, &mut mapper);
        }
        assert!(matches!(state.zoom, Zoom::Overview));
        // Reset
        apply_action(Action::ZoomReset, &mut state, &mut mapper);
        assert_eq!(state.zoom_level, 7, "ZoomReset must restore zoom_level to 7");
        assert!(matches!(state.zoom, Zoom::Boxes), "ZoomReset must restore Zoom::Boxes");
    }

    /// Every default binding for a DIRECT command must have ctrl=false and
    /// shift=false, so direct commands stay reachable with plain (unmodified)
    /// keystrokes. Two deliberate exceptions:
    ///   - save-state / restore-state intentionally use Ctrl.
    ///   - the Shift+Arrow pan aliases (SQ-0416): pan-map keeps its plain-arrow
    ///     and hjkl bindings too, so the shift-arrow alias is additive, not the
    ///     only way in. The invariant still meaningfully guards every other
    ///     direct binding (and pan-map's non-shift forms).
    #[test]
    fn direct_default_bindings_have_no_modifiers() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();

        // Commands excluded from this invariant by design.
        let excluded = ["save-state", "restore-state"];

        // Arrow key codes — used to allow the additive Shift+Arrow pan aliases.
        let is_arrow = |c| matches!(c, KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down);

        let mut violations: Vec<String> = Vec::new();
        for (spec, cmd_str, _ctx) in &km.bindings {
            let first = cmd_str.split_whitespace().next().unwrap_or("");
            if excluded.contains(&first) {
                continue;
            }
            if !layout.is_direct_name(cmd_str) {
                continue;
            }
            // The Shift+Arrow pan alias is an intentional, additive exception.
            if first == "pan-map" && spec.shift && !spec.ctrl && is_arrow(spec.code) {
                continue;
            }
            if spec.ctrl || spec.shift {
                violations.push(format!(
                    "{} ({}): ctrl={} shift={}",
                    cmd_str,
                    spec.label(),
                    spec.ctrl,
                    spec.shift,
                ));
            }
        }

        assert!(
            violations.is_empty(),
            "direct bindings with modifier keys found:\n  {}",
            violations.join("\n  ")
        );
    }

    #[test]
    fn hotkey_defaults_use_registry_names() {
        // DEFAULT_DIRECT_COMMANDS are full command-strings; validate the first token.
        for cmd in DEFAULT_DIRECT_COMMANDS {
            let name = cmd.split_whitespace().next().unwrap_or("");
            assert!(crate::slash::find_command(name).is_some(), "direct command not in registry: {cmd}");
        }
        // DEFAULT_GROUPS hold (letter, full command-string) pairs; validate the first token.
        for (_title, entries) in DEFAULT_GROUPS {
            for (_letter, cmd, _label) in *entries {
                let name = cmd.split_whitespace().next().unwrap_or("");
                assert!(crate::slash::find_command(name).is_some(), "group command not in registry: {cmd}");
            }
        }
    }

    #[test]
    fn keymap_default_and_resolve_command_strings() {
        use crate::config::KeymapConfig;
        let km = KeyMap::default();
        let cs: KeySpec = "ctrl+s".parse().unwrap();
        assert_eq!(km.lookup(&cs, Context::Global), Some("save-state"));
        // Reached through the map context's fallthrough to Global.
        assert_eq!(km.lookup(&cs, Context::Map), Some("save-state"));

        // A user CAN still bind into the map context even though it ships no
        // defaults — `[keymap.map]` remains a documented config surface.
        let mut cfg0 = KeymapConfig::default();
        cfg0.map.insert("ctrl+g".into(), "center-map".into());
        let (km0, warns0) = KeyMap::resolve(&cfg0);
        let cg: KeySpec = "ctrl+g".parse().unwrap();
        assert_eq!(km0.lookup(&cg, Context::Map), Some("center-map"));
        assert!(warns0.is_empty());

        // use_defaults=false → empty base; only the user binding exists.
        let mut cfg = KeymapConfig { use_defaults: false, ..Default::default() };
        cfg.global.insert("ctrl+s".into(), "save-state".into());
        let (km2, warns) = KeyMap::resolve(&cfg);
        let cs: KeySpec = "ctrl+s".parse().unwrap();
        assert_eq!(km2.lookup(&cs, Context::Global), Some("save-state"));
        assert!(km2.lookup(&"f6".parse().unwrap(), Context::Global).is_none(), "no defaults loaded");
        assert!(warns.is_empty());

        // Unknown command name → skip + warn.
        let mut cfg3 = KeymapConfig::default();
        cfg3.global.insert("ctrl+z".into(), "frobnicate".into());
        let (_km3, warns3) = KeyMap::resolve(&cfg3);
        assert!(warns3.iter().any(|w| w.contains("frobnicate")));
    }

    // ── SQ-0202: authored leader letters ─────────────────────────────────────

    #[test]
    fn default_leader_letters_are_unique() {
        let layout = HotkeyLayout::default();
        let letters: Vec<char> = layout.groups.iter()
            .flat_map(|(_, cmds)| cmds.iter().map(|(letter, _, _)| *letter))
            .collect();
        let unique: std::collections::HashSet<char> = letters.iter().copied().collect();
        assert_eq!(letters.len(), unique.len(), "leader letters must be unique");
        assert_eq!(
            letters.len(),
            18,
            "expected 18 authored leader letters. It was 21 until three rows left the panel: \
             `t` and `a` with the Layout group (the layout re-tidies itself, so a by-hand pass \
             was not earning a heading) and `h` with open-history (a no-op unless \
             record_turn_history is on, and the panel cannot say so). All three are still \
             commands; only the rows are gone, and the three letters are now free"
        );
    }

    #[test]
    fn leader_command_resolves_authored_letter() {
        let layout = HotkeyLayout::default();
        assert_eq!(layout.leader_command('t'), None, "`t` was tidy-map; the Layout group is gone");
        assert_eq!(layout.leader_command('r'), Some("rename-room"));
        assert_eq!(layout.leader_command('c'), Some("cycle-layer next"));
        // SQ-0446 Proposal B mnemonics:
        assert_eq!(layout.leader_command('n'), Some("edit-notes"));
        assert_eq!(layout.leader_command('i'), Some("toggle-inventory-panel"));
        assert_eq!(layout.leader_command('l'), Some("toggle-portal-labels"));
        assert_eq!(layout.leader_command('v'), Some("toggle-command-panel"));
        assert_eq!(layout.leader_command('s'), Some("open-settings"));
        assert_eq!(layout.leader_command('g'), Some("reset-game"));
        // 'q' is deliberately unassigned (bare q closes the dialog):
        assert_eq!(layout.leader_command('q'), None);
        // moved to the '/' palette — no longer leader letters:
        // ('z' was resize-panes' letter; SQ-0666 reclaimed the free slot for maZe.)
        assert_eq!(layout.leader_command('z'), Some("mark-maze-layer"));
        // 'k' was free after reset-pane-size left; SQ-0692 gave it the room panel.
        assert_eq!(layout.leader_command('k'), Some("toggle-room-panel"));
        assert_eq!(layout.leader_command('x'), None); // reset-game moved to 'g'
        assert_eq!(layout.leader_command('1'), None);
    }

    /// SQ-1142: lanthorn claims NO function key by default, in any context.
    ///
    /// A v4+ story may declare a terminating-characters table at header $2E and
    /// Infocom's V6 titles do — Arthur lists F1-F6 — so a default binding on an
    /// F-key eats a read the story explicitly asked for. F2/F3/F4 carried
    /// `toggle-command-panel`/`resize-panes`/`reveal-words` until this; they are
    /// leader-, palette- and border-control-reachable now, and only the default
    /// keymap gave them up.
    ///
    /// This is deliberately the WHOLE range rather than the three that were
    /// bound: the next person reaching for a free key must not find one here.
    #[test]
    fn no_function_key_carries_a_default_binding() {
        let km = KeyMap::default();
        for n in 1..=12u8 {
            let spec = KeySpec { code: KeyCode::F(n), ctrl: false, shift: false, alt: false };
            for ctx in [Context::Global, Context::Map, Context::Anim, Context::Browser] {
                assert_eq!(
                    km.lookup(&spec, ctx),
                    None,
                    "F{n} carries a default binding in {ctx:?}: the story may have asked \
                     for that key through its own $2E terminating-characters table",
                );
            }
        }
        // …and the real key path from the story prompt agrees: nothing resolves.
        let s = crate::state::AppState::default();
        for n in [2u8, 3, 4] {
            let ev = crossterm::event::KeyEvent::new(KeyCode::F(n), KeyModifiers::NONE);
            assert!(
                matches!(crate::input::key_to_command(&s, ev), crate::input::KeyResolve::None),
                "F{n} still resolves to a command",
            );
        }
    }

    /// The unbind is not a removal of the SPEC: a player who wants an F-key may
    /// still write one in their own `[keymap]`, and the parser has to keep
    /// accepting the token for that to be true.
    #[test]
    fn f_key_specs_still_parse_so_a_player_can_bind_one_by_hand() {
        for n in 1..=12u8 {
            let spec: KeySpec = format!("f{n}").parse().unwrap_or_else(|e| panic!("f{n}: {e}"));
            assert_eq!(spec.code, KeyCode::F(n));
        }
    }

    #[test]
    fn resolve_parses_letter_prefixed_config_entry() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: None,
            group: vec![HotkeyGroupConfig {
                title: "MyGroup".into(),
                commands: vec!["t tidy-map".into()],
            }],
        };
        let (layout, _warnings) = HotkeyLayout::resolve(&cfg);
        assert_eq!(layout.groups.len(), 1);
        assert!(layout.groups[0].1.iter().any(|(letter, cmd, _)| *letter == 't' && cmd == "tidy-map"));
    }

    #[test]
    fn resolve_autoassigns_when_letter_omitted() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: None,
            group: vec![HotkeyGroupConfig {
                title: "MyGroup".into(),
                commands: vec!["tidy-map".into()],
            }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        assert!(warnings.is_empty(), "auto-assigning a free letter should not warn: {warnings:?}");
        assert_eq!(layout.groups.len(), 1);
        assert_eq!(layout.groups[0].1.len(), 1);
        assert_eq!(layout.groups[0].1[0].1, "tidy-map");
    }

    #[test]
    fn resolve_warns_on_duplicate_letter() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: None,
            group: vec![HotkeyGroupConfig {
                title: "MyGroup".into(),
                commands: vec!["t tidy-map".into(), "t animate-tidy".into()],
            }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        assert!(!warnings.is_empty(), "duplicate letter should produce a warning");
        assert_eq!(layout.leader_command('t'), Some("tidy-map"), "first occurrence wins");
    }

    #[test]
    fn toggle_map_not_direct() {
        // Layout is not always-active anymore (SQ-0202); leader panel only.
        assert!(!HotkeyLayout::default().is_direct_name("toggle-map"));
    }

    #[test]
    fn leader_commands_unbound_in_defaults() {
        // Commands trimmed out of the always-active default keymap (SQ-0202) have
        // no default binding at all; they're reachable only through the leader panel.
        assert_eq!(KeyMap::default().lookup(&"ctrl+e".parse().unwrap(), Context::Global), None);
        assert_eq!(KeyMap::default().lookup(&"r".parse().unwrap(), Context::Map), None);
    }
}
