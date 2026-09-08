//! Basic DOS-style screen model for zvm-cli: pure formatting/SGR/terminal
//! helpers (this module) plus the stateful `ScreenView` (Task 3).

use zvm::io::TextAttrs;
use zvm::screen::{Palette, StatusLine, StatusRight, UpperWindow, ZColour, grey_rgb, rgb15_to_888};

pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// Push SGR parameters for one colour channel. `fg` selects 3x vs 4x codes.
fn push_colour_sgr(params: &mut Vec<String>, c: ZColour, fg: bool, palette: Palette) {
    let (base_std, base_true) = if fg { (30u16, 38u16) } else { (40u16, 48u16) };
    match c {
        ZColour::Default => {}
        ZColour::Standard(n @ 2..=9) => params.push((base_std + (n as u16 - 2)).to_string()),
        ZColour::Standard(n) => {
            let (r, g, b) = grey_rgb(palette, n);
            params.push(format!("{};2;{};{};{}", base_true, r, g, b));
        }
        ZColour::True(v) => {
            let (r, g, b) = rgb15_to_888(v);
            params.push(format!("{};2;{};{};{}", base_true, r, g, b));
        }
        ZColour::True24(v) => {
            let (r, g, b) = (((v >> 16) & 0xFF), ((v >> 8) & 0xFF), (v & 0xFF));
            params.push(format!("{};2;{};{};{}", base_true, r, g, b));
        }
        _ => {}
    }
}

/// Resolve a `ZColour` to a 24-bit RGB triple for painting the terminal's
/// *page* background (OSC 11), mirroring `push_colour_sgr`'s per-variant
/// colour sources. `Default` (no game colour) is `None`. `Standard(2..=9)`
/// has no RGB source here either — `push_colour_sgr` emits a bare ANSI base
/// code for it and lets the terminal's own colour scheme resolve the actual
/// colour, so there is no concrete triple to paint OSC 11 with; that case is
/// also `None` rather than an invented palette value.
pub fn zcolour_rgb(c: ZColour, palette: Palette) -> Option<(u8, u8, u8)> {
    match c {
        ZColour::Default => None,
        ZColour::Standard(2..=9) => None,
        ZColour::Standard(n) => Some(grey_rgb(palette, n)),
        ZColour::True(v) => Some(rgb15_to_888(v)),
        ZColour::True24(v) => Some((((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8)),
        _ => None,
    }
}

// The page-background OSC and cursor-shape escapes live in `cli_host::term` —
// they were byte-identical here and in gvm-cli (SQ-0605).

/// SGR set-sequence (`ESC[...m`, no trailing reset) for `attrs`, or `""` when
/// no style/colour is active. Shared by `style_wrap` and the raw-mode input
/// editor (to echo typed input in the game's current style/colour).
pub fn sgr_open(attrs: TextAttrs, palette: Palette) -> String {
    let mut params: Vec<String> = Vec::new();
    if attrs.style & 0x01 != 0 { params.push("7".into()); }
    if attrs.style & 0x02 != 0 { params.push("1".into()); }
    if attrs.style & 0x04 != 0 { params.push("3".into()); }
    push_colour_sgr(&mut params, attrs.fg, true, palette);
    push_colour_sgr(&mut params, attrs.bg, false, palette);
    if params.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", params.join(";"))
    }
}

/// SGR that sets only the background colour (`ESC[..m`, no reset), or `""` when
/// the background is Default or colours are not honoured. Used to paint clears
/// and line padding with the game's chosen background.
pub fn bg_sgr(bg: ZColour, honor: bool, palette: Palette) -> String {
    if !honor {
        return String::new();
    }
    let mut params: Vec<String> = Vec::new();
    push_colour_sgr(&mut params, bg, false, palette);
    if params.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", params.join(";"))
    }
}

/// Wrap lower-window text in SGR when on a TTY and any style/colour is active; else plain.
pub fn style_wrap(s: &str, attrs: TextAttrs, is_tty: bool, palette: Palette) -> String {
    if !is_tty {
        return s.to_string();
    }
    let open = sgr_open(attrs, palette);
    if open.is_empty() {
        s.to_string()
    } else {
        format!("{}{}\x1b[0m", open, s)
    }
}

/// Terminal BEL per bleep, TTY-gated.
pub fn bleep_bytes(count: usize, is_tty: bool) -> String {
    if is_tty {
        "\x07".repeat(count)
    } else {
        String::new()
    }
}

fn right_field(right: &StatusRight) -> String {
    match right {
        StatusRight::ScoreTurns { score, turns } => format!("Score: {score}  Moves: {turns}"),
        StatusRight::Time { hours, minutes } => format!("Time: {hours:02}:{minutes:02}"),
        _ => String::new(),
    }
}

/// Plain v3 status row: " `location` ... `right` ", padded to exactly `cols`.
pub fn status_text(st: &StatusLine, cols: u16) -> String {
    let cols = cols as usize;
    if cols < 2 {
        return " ".repeat(cols);
    }
    let inner = cols - 2; // one border space each side
    let right = right_field(&st.right);
    let right_w = right.chars().count().min(inner);
    let right: String = right.chars().take(right_w).collect();
    let left_max = inner - right_w;
    let left: String = st.location.chars().take(left_max).collect();
    let fill = inner - left.chars().count() - right_w;
    format!(" {}{}{} ", left, " ".repeat(fill), right)
}

/// The v1-v3 status row, dressed as `look`'s machine dressed its own (SQ-0873).
///
/// `None` — every ordinary run — is the plain full-width reverse this has always
/// emitted, and so is [`StatusBand::FullReverse`]: the page and the ink are the
/// terminal's own defaults by the time this is called, so `\x1b[7m` swaps exactly
/// the machine's pair.
///
/// The other three are the reason this function exists.
/// [`StatusBand::Ruled`] is the Macintosh, which does not distinguish the band by
/// ground at all and puts solid rules above and below it — one row of underline
/// is a terminal's whole horizontal-rule vocabulary.
/// [`StatusBand::Own`] states a pair outright, being neither the body pair nor
/// its reverse.
/// [`StatusBand::PerRun`] is the Amiga, where the reversal is applied behind each
/// RUN of text and the page shows between: measured on `amiga-spellbreaker.png`
/// as 376 px of page between "Council Chamber" and "Score: 0/0". A run breaks on
/// two or more spaces, so the single space inside a room name stays inside it —
/// which is what the capture shows.
///
/// Version 4 is inside the period look's range and never reaches here: a v4 story
/// writes its own upper window and this row is only ever the v1-v3 status line
/// the interpreter synthesizes.
pub fn status_band_ansi(text: &str, look: Option<zvm::interpreter::PeriodLook>) -> String {
    use zvm::interpreter::StatusBand;
    let reverse = |t: &str| format!("\x1b[7m{t}\x1b[0m");
    match look.map(|l| l.status) {
        None | Some(StatusBand::FullReverse) => reverse(text),
        Some(StatusBand::Ruled) => format!("\x1b[4m{text}\x1b[0m"),
        Some(StatusBand::Own { ground, ink }) => format!(
            "\x1b[38;2;{};{};{};48;2;{};{};{}m{text}\x1b[0m",
            ink.0, ink.1, ink.2, ground.0, ground.1, ground.2
        ),
        Some(StatusBand::PerRun) => {
            let mut out = String::new();
            let mut rest = text;
            while !rest.is_empty() {
                // A gap is THREE or more spaces. Two is what `status_text` puts
                // between "Score:" and "Moves:", and the Amiga capture reverses
                // those as one run — the page shows between the LOCATION and the
                // score, which is the wide fill, and nowhere else.
                let gap_at = rest.match_indices("   ").map(|(i, _)| i).next();
                let (run, tail) = match gap_at {
                    Some(0) => {
                        let end = rest.find(|c| c != ' ').unwrap_or(rest.len());
                        out.push_str(&rest[..end]);
                        rest = &rest[end..];
                        continue;
                    }
                    Some(i) => (&rest[..i], &rest[i..]),
                    None => (rest, ""),
                };
                out.push_str(&reverse(run));
                rest = tail;
            }
            out
        }
        Some(_) => reverse(text),
    }
}

/// Plain text of one upper-window row, trailing blanks trimmed.
pub fn upper_row_text(upper: &UpperWindow, row: u16) -> String {
    let mut s = String::new();
    for c in 1..=upper.cols {
        s.push(upper.cell(row, c).ch);
    }
    s.trim_end().to_string()
}

/// One upper-window row with per-cell SGR runs (for the pinned TTY region).
/// Carries style AND colour (fg/bg); colour is suppressed when `honor` is false.
pub fn upper_row_ansi(
    upper: &UpperWindow,
    row: u16,
    honor: bool,
    current_fg: ZColour,
    current_bg: ZColour,
    palette: Palette,
) -> String {
    // Last column with any non-default attribute (blank cell = ' ' at style 0,
    // Default/Default); trailing defaults are dropped so the row closes reset,
    // matching the `ESC[2K` line-clear done before each row is written.
    let last = (1..=upper.cols)
        .rev()
        .find(|&c| {
            let cell = upper.cell(row, c);
            cell.ch != ' '
                || cell.style != 0
                || (honor && !matches!(cell.fg, ZColour::Default))
                || (honor && !matches!(cell.bg, ZColour::Default))
        })
        .unwrap_or(0);
    let mut out = String::new();
    let mut cur: Option<(u8, ZColour, ZColour)> = None;
    for c in 1..=last {
        let cell = upper.cell(row, c);
        let (style, fg, bg) = if honor {
            // The upper window's "default" colour is the screen's current
            // colour (what the row-fill paints); resolve Default cells to it so
            // blank/leading cells match the painted background instead of
            // resetting to terminal-default.
            let fg = if matches!(cell.fg, ZColour::Default) { current_fg } else { cell.fg };
            let bg = if matches!(cell.bg, ZColour::Default) { current_bg } else { cell.bg };
            (cell.style, fg, bg)
        } else {
            (cell.style, ZColour::Default, ZColour::Default)
        };
        if cur != Some((style, fg, bg)) {
            out.push_str("\x1b[0m");
            out.push_str(&sgr_open(TextAttrs { style, fg, bg }, palette));
            cur = Some((style, fg, bg));
        }
        out.push(cell.ch);
    }
    if cur.is_some() {
        out.push_str("\x1b[0m");
    }
    out
}

// The scroll-region helpers and the `Pin` placement live in `cli_host::pin`,
// shared with `gvm-cli`, which pins Glk grid windows the same way and had its own
// copy of the same three lines. The measurement behind the placement — a region
// discards what scrolls out of it unless its top margin is row 1 — is that module's
// tests (SQ-0909).
pub use cli_host::{Pin, leave_region};
use cli_host::{enter_region, leave_and_park, pinned_origin};

// ---------------------------------------------------------------------------
// ScreenView — stateful top-region rendering
// ---------------------------------------------------------------------------

use zvm::cpu::exec::Machine;

/// The one block source zvm-cli has: the upper window (or the v1–v3 status
/// line, which is the same region by another name).
const SOURCE_UPPER: u32 = 0;

/// Tracks the pinned top-region state and produces the bytes to emit before an
/// input prompt: an ANSI scroll-region update on a TTY, or a deduped inline
/// plain-text block when piped.
pub struct ScreenView {
    is_tty: bool,
    /// `--story-only`: suppress the pinned region entirely — status line,
    /// menus, forms, everything the game draws above the story. Stronger than
    /// plain mode's `quiet_status_line`, which only quietens one-row chrome.
    story_only: bool,
    /// Plain mode without `--show-status`: don't narrate a one-row status
    /// region on every turn (SQ-0612).
    ///
    /// It is one row that makes this safe to do by size. A v3 status line is
    /// exactly one row by definition, and a v4+ game that splits off a single
    /// row is drawing the same thing; anything taller is a menu, a form, or a
    /// panel the game means you to read. The measured cases are the Infocom
    /// releases with integrated InvisiClues — Planetfall's hint menu is twelve
    /// chapter headings and a `RETURN = See hint / Q = Resume story` legend, all
    /// drawn in the upper window, and a blanket suppression loses every line of
    /// it. Lost Pig's HELP menu and Bureaucracy's licence-application form are
    /// the same shape. Chrome is what gets quietened, not content.
    ///
    /// And it is the one-row case that is actually noisy: a v3 status line
    /// carries `Moves: N`, so it differs every turn and the dedupe never fires.
    /// Measured on Ballyhoo, that is a status line narrated on four turns out of
    /// four. `/status` still answers on demand.
    quiet_status_line: bool,
    /// Screen-reader mode: recognise a repainted menu in the inline block and
    /// announce the marker move instead of reading the block out again
    /// (SQ-0609). Off everywhere else — on a TTY the block is painted in place
    /// and nothing is repeated, and a plain pipe is a transcript that must stay
    /// byte-identical.
    menus: bool,
    /// The menu state behind `menus`. Owned here because this is the only place
    /// that sees the inline block; `main` reaches it through
    /// [`ScreenView::menu`](Self::menu) and friends for `/menu` and number jumps.
    menu: cli_host::MenuTracker,
    /// Where the pinned rows go, and so whether the terminal keeps history.
    pin: Pin,
    term_rows: u16,
    /// Tracked terminal width, kept current on resize. The v3 status bar is
    /// padded to exactly this: hard-coding `DEFAULT_COLS` made the reverse-video
    /// bar wrap out of its 1-row pinned region into the story text on any
    /// terminal narrower than 80 columns (SQ-0636).
    term_cols: u16,
    active_rows: u16,           // current scroll-region top height (TTY)
    last_block: Option<String>, // last inline block emitted (non-TTY dedupe)
    // Set by `erase`; consumed by the next `render`. When set, a subsequent
    // upper-window growth shifts the freshly-streamed lower prompt down below the
    // new upper region (see the shift logic in `render`). Only meaningful right
    // after an `erase_window`, so continuous flow (no erase) is never shifted.
    pending_erase_shift: bool,
    /// The machine's period look for this run, when `--period-look` asked for one
    /// and the story is v1-v4 (SQ-0873). `None` everywhere else, which is the
    /// default and every behaviour below unchanged.
    ///
    /// The page and the ink are the TERMINAL's own defaults by then (OSC 11/10,
    /// set once in `main`), so what is left here is the one thing they cannot
    /// carry: how that machine set its status line apart. Four of the five
    /// measured do it four different ways and only the Apple II, the Commodore
    /// 128 and the Solid Gold Commodore 64 do what this view already did.
    period_look: Option<zvm::interpreter::PeriodLook>,
}

impl ScreenView {
    pub fn new(
        is_tty: bool,
        story_only: bool,
        quiet_status_line: bool,
        term_rows: u16,
        term_cols: u16,
    ) -> Self {
        ScreenView {
            is_tty,
            story_only,
            quiet_status_line,
            menus: false,
            menu: cli_host::MenuTracker::new(),
            pin: Pin::default(),
            term_rows,
            term_cols,
            active_rows: 0,
            last_block: None,
            pending_erase_shift: false,
            period_look: None,
        }
    }

    /// Dress the status band as `look`'s machine did (SQ-0873); `None` restores
    /// the plain reverse-video bar.
    pub fn set_period_look(&mut self, look: Option<zvm::interpreter::PeriodLook>) {
        self.period_look = look;
    }

    /// Choose where the pinned rows sit. The next frame acts on it: the live region
    /// is torn down and rebuilt through `render`'s ordinary path, so this is usable
    /// mid-session and not only at launch.
    pub fn set_pin(&mut self, pin: Pin) {
        if pin != self.pin {
            self.pin = pin;
            self.active_rows = 0; // force the region to be re-established
        }
    }

    /// Where the pinned rows currently sit.
    pub fn pin(&self) -> Pin {
        self.pin
    }

    /// Turn menu recognition on (screen-reader mode only — SQ-0609).
    pub fn set_menus(&mut self, on: bool) {
        self.menus = on;
    }

    /// The open menu re-listed on demand (`/menu`), or `None` when none is open.
    pub fn menu_listing(&self) -> Option<String> {
        self.menu.listing()
    }

    /// What a line typed at a char prompt means to the open menu.
    pub fn typed_at_menu(&mut self, line: &str) -> cli_host::Typed {
        // The legend lives in the block itself — the same rows the items do — so
        // the menu's own text is the legend text.
        let legend = self.last_block.clone().unwrap_or_default();
        self.menu.typed(line, &legend)
    }

    /// The next synthesized navigation keystroke, as a ZSCII code (ZMSD §3.8:
    /// cursor up 129, cursor down 130).
    pub fn next_menu_key(&mut self) -> Option<u8> {
        self.menu.next_key().map(|k| match k {
            cli_host::NavKey::Char(c) => c as u8,
            cli_host::NavKey::Up => 129,
            cli_host::NavKey::Down => 130,
        })
    }


    /// Number of pinned top rows for the current machine state.
    fn top_rows(machine: &Machine) -> u16 {
        if machine.mem.version() < 4 {
            1 // v1-v3: a status line is always shown
        } else {
            // The grid's own row count: equals `upper_window_rows` normally, but
            // may be larger when a game drew in the upper window below the split
            // (e.g. LostPig's HELP menu). Render every row the game wrote.
            machine.screen.upper.rows
        }
    }

    /// Plain-text rows of the top region (status row for v3, the upper grid for
    /// v4+). Empty vec when there is no region. `cols` is the width the v3
    /// status bar is padded to — the *tracked* terminal width, not
    /// `DEFAULT_COLS` (SQ-0636).
    fn rows_plain(machine: &Machine, top: u16, cols: u16) -> Vec<String> {
        if top == 0 {
            return Vec::new();
        }
        if machine.mem.version() < 4 {
            vec![status_text(&machine.status_line(), cols)]
        } else {
            (1..=top).map(|r| upper_row_text(&machine.screen.upper, r)).collect()
        }
    }

    /// ANSI rows of the top region (reverse-video bar for v3, per-cell SGR runs
    /// for v4+).
    fn rows_ansi(
        machine: &Machine,
        top: u16,
        cols: u16,
        look: Option<zvm::interpreter::PeriodLook>,
    ) -> Vec<String> {
        if top == 0 {
            return Vec::new();
        }
        if machine.mem.version() < 4 {
            vec![status_band_ansi(&status_text(&machine.status_line(), cols), look)]
        } else {
            (1..=top)
                .map(|r| {
                    upper_row_ansi(
                        &machine.screen.upper,
                        r,
                        machine.honor_game_colours,
                        machine.screen.current_fg,
                        machine.screen.current_bg,
                        machine.palette(),
                    )
                })
                .collect()
        }
    }

    /// Bytes to emit just before an input prompt.
    pub fn frame(&mut self, machine: &Machine) -> String {
        if self.story_only {
            return String::new();
        }
        let top = Self::top_rows(machine);
        let plain = Self::rows_plain(machine, top, self.term_cols);
        let ansi = Self::rows_ansi(machine, top, self.term_cols, self.period_look);
        let bg_paint = bg_sgr(machine.screen.current_bg, machine.honor_game_colours, machine.palette());
        self.render(top, &plain, &ansi, &bg_paint)
    }

    /// Pure-core renderer: given the pinned-row count and the already-formatted
    /// plain/ANSI rows, advance the view's state and return the bytes to write.
    fn render(&mut self, top: u16, rows_plain: &[String], rows_ansi: &[String], bg_paint: &str) -> String {
        if self.story_only {
            return String::new();
        }
        if self.quiet_status_line && top <= 1 {
            return String::new();
        }
        if self.is_tty {
            let mut out = String::new();
            // The shift below is a one-shot consumed on the first frame after an
            // erase; read and clear it now.
            let erase_shift = self.pending_erase_shift;
            self.pending_erase_shift = false;
            let want = top;
            if want != self.active_rows {
                let delta = want as i32 - self.active_rows as i32;
                // Only the top placement needs this: with the rows pinned at the
                // BOTTOM the freshly-streamed lower-window text is already in the
                // story area, so there is nothing to push out of the way.
                if erase_shift && delta > 0 && top >= 2 && self.pin == Pin::Top {
                    // A game cleared the screen, then redrew a multi-row upper
                    // window (e.g. BeyondZork's stats + compass) and streamed its
                    // short lower-window prompt right after the clear — before
                    // this frame pins the upper region. That prompt (and the
                    // input cursor) would otherwise sit inside the soon-to-be-
                    // painted upper region, on top of the compass. Push the lower
                    // window — its streamed text AND the cursor — down by the
                    // delta so the cursor lands in the lower window and the prompt
                    // stays visible (ZMSD §8.7.2.2).
                    //
                    // Sequence: DECSC to capture the real prompt cursor, reset to
                    // full screen, scroll the display down (SD), DECRC to restore
                    // the cursor, then a relative cursor-down (CUD) by the delta.
                    // Wrapping SD in DECSC/DECRC normalises the terminal's SD
                    // cursor side-effect, which is NOT portable (tmux homes the
                    // cursor on SD; xterm leaves it put) — so the relative CUD then
                    // lands on the shifted prompt with its real column intact.
                    //
                    // Gated on BOTH the just-erased flag and a multi-row upper
                    // window (top >= 2). In continuous flow (no erase) the content
                    // is already positioned correctly. And a v3 game also erases
                    // at startup but then streams a full screen of narrative under
                    // a 1-row status line (top == 1) — scrolling that would garble
                    // it — so the 1-row status case is excluded.
                    out.push_str("\x1b7"); // DECSC: save real prompt cursor
                    out.push_str(&leave_region());
                    out.push_str(&format!("\x1b[{delta}T")); // SD: content down
                    out.push_str("\x1b8"); // DECRC: restore prompt cursor
                    out.push_str(&format!("\x1b[{delta}B")); // CUD: follow content
                    out.push_str(&enter_region(top, self.term_rows, self.pin));
                } else {
                    out.push_str(&if want == 0 {
                        leave_region()
                    } else {
                        enter_region(want, self.term_rows, self.pin)
                    });
                }
                self.active_rows = want;
            }
            if top > 0 {
                let origin = pinned_origin(self.pin, top, self.term_rows);
                out.push_str("\x1b7"); // DECSC save cursor
                for (i, row) in rows_ansi.iter().enumerate() {
                    // position, paint bg, clear-to-EOL (fills with bg), then row
                    out.push_str(&format!("\x1b[{};1H{}\x1b[2K", origin + i as u16, bg_paint));
                    out.push_str(row);
                }
                out.push_str("\x1b8"); // DECRC restore cursor
            }
            out
        } else {
            if top == 0 {
                return String::new();
            }
            let block = {
                let mut rows: Vec<String> =
                    rows_plain.iter().map(|r| r.trim_end().to_string()).collect();
                while rows.last().map(|r| r.is_empty()).unwrap_or(false) {
                    rows.pop();
                }
                if rows.is_empty() {
                    String::new()
                } else {
                    format!("{}\n", rows.join("\n"))
                }
            };
            if block.is_empty() || self.last_block.as_deref() == Some(block.as_str()) {
                // Unchanged. Still an event when a number jump has just landed
                // on the item it started from (SQ-0609).
                return match self.menus {
                    true => match self.menu.unchanged(SOURCE_UPPER) {
                        cli_host::Emission::Line(l) => format!("{l}\n"),
                        _ => String::new(),
                    },
                    false => String::new(),
                };
            }
            self.last_block = Some(block.clone());
            if !self.menus {
                return block;
            }
            // SOURCE_UPPER: the Z-machine has exactly one place a menu can be
            // drawn from this path — the upper window — so one source id.
            match self.menu.observe(SOURCE_UPPER, &block) {
                cli_host::Emission::Block(b) => b,
                cli_host::Emission::Line(l) => format!("{l}\n"),
                cli_host::Emission::Nothing => String::new(),
            }
        }
    }

    /// The pinned region's current text, unconditionally — no dedupe, no TTY
    /// gate, no `--story-only` gate.
    ///
    /// [`frame`](Self::frame) answers "what changed since last time", which is
    /// right for streaming and wrong for `/status`: the player asked *because*
    /// nothing changed and it has scrolled away (SQ-0610). Empty when the game
    /// has no status region at all.
    pub fn status_now(machine: &Machine) -> String {
        let top = Self::top_rows(machine);
        // Inline plain text, so the padding width is cosmetic (trailing blanks
        // are trimmed below); the default is fine without threading a view in.
        let mut rows = Self::rows_plain(machine, top, DEFAULT_COLS);
        while rows.last().is_some_and(|r| r.trim_end().is_empty()) {
            rows.pop();
        }
        rows.iter().map(|r| r.trim_end()).collect::<Vec<_>>().join("\n")
    }

    /// Update the row count used for scroll-region sizing (call on terminal resize).
    pub fn set_term_rows(&mut self, rows: u16) {
        self.term_rows = rows;
    }

    /// Update the column count the v3 status bar is padded to (call on resize).
    pub fn set_term_cols(&mut self, cols: u16) {
        self.term_cols = cols;
    }

    /// Clear+home the screen at startup (interactive only), so existing
    /// scrollback is not overwritten by the pinned region.
    pub fn start(&self) -> String {
        if self.is_tty && !self.story_only {
            "\x1b[2J\x1b[H".to_string()
        } else {
            String::new()
        }
    }

    /// Bytes to clear the lower window in response to an `erase_window`
    /// request (ZMSD §8.7.3): leave any scroll region, clear the whole screen,
    /// home the cursor, and reset the pinned-region state so the next `frame`
    /// re-establishes the region. On a piped/non-TTY sink there is no screen to
    /// clear (streaming scrollback), so it is a no-op.
    pub fn erase(&mut self, bg: ZColour, honor: bool, palette: Palette) -> String {
        if !self.is_tty {
            return String::new();
        }
        self.active_rows = 0;
        // Arm the one-shot lower-window shift: if the game now redraws a
        // multi-row upper window, the next `render` pushes the freshly-streamed
        // lower prompt down below it (see `render`).
        self.pending_erase_shift = true;
        let paint = bg_sgr(bg, honor, palette);
        if paint.is_empty() {
            format!("{}\x1b[2J\x1b[H", leave_region())
        } else {
            // Set bg, clear (fills with bg), home, then reset so subsequent
            // prompt/text is not forced onto the painted background run.
            format!("{}{}\x1b[2J\x1b[H\x1b[0m", leave_region(), paint)
        }
    }

    /// Restore the terminal at quit.
    pub fn leave(&mut self) -> String {
        if !self.is_tty {
            return String::new();
        }
        let was_pinned = self.active_rows > 0;
        self.active_rows = 0;
        // The region reset is UNCONDITIONAL (SQ-0913). It was gated on having
        // pinned something, which is the wrong question twice over: a region set
        // by a path this view did not record survives the gate, and `CSI r` is a
        // no-op when none is set, so there is nothing to save by asking.
        let region = leave_region();
        if was_pinned {
            // Shared with every other way out, including the Ctrl-C paths that never
            // reach `main` — see `cli_host::pin::leave_and_park`.
            let _ = &region;
            leave_and_park(self.term_rows)
        } else {
            // Nothing was pinned, so the cursor is sitting wherever the game left
            // its prompt — mid-screen, mid-paragraph. Returning without moving it
            // hands the shell that spot, and the next prompt is drawn INTO the
            // story text. Start a fresh line instead. This is the reported symptom
            // in SQ-0913, and it got commoner with SQ-0909: a one-row status bar no
            // longer pins anything, so `was_pinned` is now false for most games.
            format!("{region}\r\n")
        }
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use zvm::screen::{StatusLine, StatusRight};

    /// SQ-0873. Four machines, four bands, and only one of them is what this
    /// view emitted for everybody.
    #[test]
    fn the_status_band_is_dressed_as_its_machine_dressed_it() {
        // Asked the way the front-end asks (SQ-0983): one row stores no pair, so
        // reading the row would be reading a value that never reaches a screen.
        let look = |n| zvm::interpreter::period_look_for(n, None).unwrap();
        let bar = " Council Chamber   Score: 0/0 ";

        // No look, and the Solid Gold C64: the plain full-width reverse, which is
        // exactly right once the terminal's own defaults ARE the machine's pair.
        assert_eq!(status_band_ansi(bar, None), format!("\x1b[7m{bar}\x1b[0m"));
        assert_eq!(
            status_band_ansi(bar, Some(look(zvm::interpreter::COMMODORE_64_INTERPRETER_NUMBER))),
            format!("\x1b[7m{bar}\x1b[0m")
        );

        // The Macintosh rules its band instead of grounding it.
        assert_eq!(
            status_band_ansi(bar, Some(look(zvm::interpreter::MACINTOSH_INTERPRETER_NUMBER))),
            format!("\x1b[4m{bar}\x1b[0m")
        );

        // SQ-0873: the Amiga measures per run — 376 px of page between "Council
        // Chamber" and "Score: 0/0" — and draws a full-width reverse, on the
        // user's ruling that a band in pieces reads as damage in a terminal.
        assert_eq!(
            status_band_ansi(bar, Some(look(zvm::interpreter::AMIGA_INTERPRETER_NUMBER))),
            format!("\x1b[7m{bar}\x1b[0m"),
            "drawn whole, measured per run"
        );
        // The IBM PC measures the same way and gets the same treatment.
        assert_eq!(
            status_band_ansi(bar, Some(look(zvm::interpreter::IBM_PC_INTERPRETER_NUMBER))),
            format!("\x1b[7m{bar}\x1b[0m")
        );
        // The per-run RENDERING still exists and is still correct — nothing
        // reaches it from the table, so it is exercised directly.
        use zvm::interpreter::{CursorShape, PeriodLook, StatusBand};
        let per_run = PeriodLook {
            page: (0, 0, 0),
            ink: (0xFF, 0xFF, 0xFF),
            status: StatusBand::PerRun,
            cursor_shape: CursorShape::Block,
            cursor_colour: (0xFF, 0xFF, 0xFF),
        };
        assert_eq!(
            status_band_ansi(bar, Some(per_run)),
            "\x1b[7m Council Chamber\x1b[0m   \x1b[7mScore: 0/0 \x1b[0m",
            "and a run breaks on three spaces, not the two inside a field"
        );

        // Whatever the dressing, the visible text is untouched.
        assert_eq!(strip_sgr(&status_band_ansi(bar, None)), bar);
    }

    /// The one variant no row uses today, kept because the 1984 Commodore 64
    /// build is the evidence that a band need not derive from the body pair.
    #[test]
    fn an_own_band_states_its_pair_outright() {
        use zvm::interpreter::{CursorShape, PeriodLook, StatusBand};
        let look = PeriodLook {
            page: (0x6C, 0x6C, 0x6C),
            ink: (0xFF, 0xFF, 0xFF),
            status: StatusBand::Own { ground: (0, 0, 0), ink: (0x6C, 0x6C, 0x6C) },
            cursor_shape: CursorShape::Underscore,
            cursor_colour: (0, 0, 0),
        };
        assert_eq!(
            status_band_ansi("x", Some(look)),
            "\x1b[38;2;108;108;108;48;2;0;0;0mx\x1b[0m"
        );
    }

    /// Drop every SGR sequence, leaving the text a player actually sees.
    fn strip_sgr(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// A hand-built v3 status region (plain + reverse-video ANSI rows).
    fn v3_rows() -> (Vec<String>, Vec<String>) {
        let st = StatusLine {
            location: "West of House".into(),
            right: StatusRight::ScoreTurns { score: 0, turns: 1 },
        };
        let plain = vec![status_text(&st, DEFAULT_COLS)];
        let ansi = vec![format!("\x1b[7m{}\x1b[0m", status_text(&st, DEFAULT_COLS))];
        (plain, ansi)
    }

    #[test]
    fn no_status_suppresses_everything() {
        let (p, a) = v3_rows();
        let mut piped = ScreenView::new(false, true, false, 24, 80);
        assert_eq!(piped.render(1, &p, &a, ""), "");
        let mut tty = ScreenView::new(true, true, false, 24, 80);
        assert_eq!(tty.render(1, &p, &a, ""), "");
    }

    #[test]
    fn piped_emits_inline_block_once_then_dedupes() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(false, false, false, 24, 80);
        let first = v.render(1, &p, &a, "");
        assert!(first.contains("West of House"), "first frame emits block: {first:?}");
        assert!(!first.contains('\x1b'), "inline block carries no ANSI: {first:?}");
        let second = v.render(1, &p, &a, "");
        assert_eq!(second, "", "unchanged region dedupes to empty");
    }

    /// The DEFAULT is unchanged from before SQ-0909: pinned at the top, where
    /// Infocom put it, region below.
    #[test]
    fn tty_pins_at_the_top_by_default_and_resets_on_leave() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, false, 24, 80);
        assert_eq!(v.pin(), Pin::Top);
        let f = v.render(1, &p, &a, "");
        assert!(f.contains("\x1b[2;24r"), "region starts below the one pinned row: {f:?}");
        assert!(f.contains("\x1b[1;1H"), "and the bar is painted on row 1: {f:?}");
        assert!(f.contains("\x1b[7m"), "v3 status bar is reverse-video: {f:?}");
        assert!(v.leave().contains("\x1b[r"), "leave resets region");
    }

    /// `--pin bottom` moves the SAME rows to the bottom and starts the region at
    /// row 1, which is what lets the terminal archive what scrolls past.
    #[test]
    fn pinning_at_the_bottom_starts_the_region_at_row_one() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, false, 24, 80);
        v.set_pin(Pin::Bottom);
        let f = v.render(1, &p, &a, "");
        assert!(f.contains("\x1b[1;23r"), "region is rows 1..23, above the pinned row: {f:?}");
        assert!(f.contains("\x1b[24;1H"), "the bar is painted on the last row: {f:?}");
        assert!(f.contains("West of House"), "and it is still the status bar: {f:?}");
        // A six-row window takes the last six rows and leaves eighteen to scroll.
        let rows = vec!["  Chapter One".to_string(); 6];
        let g = v.render(6, &rows, &rows, "");
        assert!(g.contains("\x1b[1;18r"), "region shrinks to rows 1..18: {g:?}");
        assert!(g.contains("\x1b[19;1H"), "six rows pinned from row 19 down: {g:?}");
    }

    /// Switching placement mid-session rebuilds the region rather than waiting for
    /// the window to change size, so `/pin` takes effect on the next frame.
    #[test]
    fn changing_the_placement_re_establishes_the_region() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, false, 24, 80);
        assert!(v.render(1, &p, &a, "").contains("\x1b[2;24r"), "top to begin with");
        assert_eq!(v.render(1, &p, &a, ""), v.render(1, &p, &a, ""), "a settled frame repaints only");
        v.set_pin(Pin::Bottom);
        let out = v.render(1, &p, &a, "");
        assert!(out.contains("\x1b[1;23r"), "the region moves on the very next frame: {out:?}");
    }

    /// A menu-shaped upper window: a header, then items with `>` on one of them.
    fn menu_rows(selected: usize) -> Vec<String> {
        let items = ["Credits", "Documentation", "Sample Transcript"];
        let mut rows = vec![" N = next item    P = previous item".to_string()];
        for (i, item) in items.iter().enumerate() {
            rows.push(format!("{} {item}", if i == selected { '>' } else { ' ' }));
        }
        rows
    }

    #[test]
    fn quiet_mode_drops_a_one_row_status_but_keeps_a_menu() {
        // SQ-0612. A v3 status line carries `Moves: N`, so it differs every turn
        // and the dedupe never fires — measured, Ballyhoo narrated it on four
        // turns out of four. Plain mode quietens that. But the suppression is by
        // SIZE: the games with integrated InvisiClues draw their hint menus in
        // the upper window — Planetfall's twelve chapter headings and its
        // `RETURN = See hint` legend all vanish under a blanket suppression
        // (measured), as does Lost Pig's HELP menu.
        let (p, a) = v3_rows();
        let mut quiet = ScreenView::new(false, false, true, 24, 80);
        assert_eq!(quiet.render(1, &p, &a, ""), "", "a one-row status is chrome — quietened");

        let menu = menu_rows(0);
        let out = quiet.render(4, &menu, &menu, "");
        assert!(out.contains("> Credits"), "a four-row menu is content — kept: {out:?}");

        // Asking for it back restores the status line.
        let mut loud = ScreenView::new(false, false, false, 24, 80);
        assert!(loud.render(1, &p, &a, "").contains("West of House"), "--show-status restores it");
    }

    #[test]
    fn no_status_still_beats_show_status() {
        // `--story-only` is the stronger, already-documented switch: it suppresses
        // the upper window outright, menu or not.
        let menu = menu_rows(0);
        let mut v = ScreenView::new(false, true, false, 24, 80);
        assert_eq!(v.render(4, &menu, &menu, ""), "");
    }

    #[test]
    fn a_piped_transcript_still_repeats_the_whole_menu() {
        // A plain pipe is a transcript, not a reading. SQ-0607 and zvm-cli's
        // inline block made a grid-drawn menu (Arthur's, Shogun's startup menu)
        // come through as text at all, and it repeats whenever anything in it
        // changes — so the transcript records where the marker went. Menu
        // recognition is screen-reader mode only, and this is what it must NOT
        // change.
        let mut v = ScreenView::new(false, false, false, 24, 80);
        let first = menu_rows(0);
        let out = v.render(4, &first, &first, "");
        assert!(out.contains("> Credits"), "menu comes through as text: {out:?}");
        assert!(out.contains("N = next item"), "and so does its key legend: {out:?}");

        // Unchanged: silence, or every turn would replay the whole menu.
        assert_eq!(v.render(4, &first, &first, ""), "");

        // Marker moves: the block repeats, with the new selection.
        let second = menu_rows(1);
        let moved = v.render(4, &second, &second, "");
        assert!(moved.contains("> Documentation"), "new selection announced: {moved:?}");
        assert!(!moved.contains("> Credits"), "old selection is gone: {moved:?}");
    }

    /// SQ-0609. The pin above, in screen-reader mode, deliberately flipped: what
    /// was a whole block re-read on every keypress is now one line.
    ///
    /// Measured before this, `n` at Planetfall's InvisiClues menu read out
    /// sixteen lines and Arthur's read out twenty-three, every single press.
    #[test]
    fn screen_reader_mode_announces_the_move_instead_of_the_menu() {
        let mut v = ScreenView::new(false, false, true, 24, 80);
        v.set_menus(true);
        let first = menu_rows(0);
        let out = v.render(4, &first, &first, "");
        assert!(out.contains(cli_host::menu::MENU_HINT), "opens host-numbered: {out:?}");
        assert!(out.contains(">1. Credits"), "the marked item keeps its marker: {out:?}");
        assert!(out.contains(" 2. Documentation"), "and the rest are numbered: {out:?}");
        assert!(
            out.contains(" N = next item    P = previous item"),
            "the legend is not an item and is not numbered: {out:?}"
        );

        // The move: one line, and none of the other fifteen.
        let moved = v.render(4, &menu_rows(1), &menu_rows(1), "");
        assert_eq!(moved, ">2. Documentation (2 of 3)\n");
        assert_eq!(v.render(4, &menu_rows(2), &menu_rows(2), ""), ">3. Sample Transcript (3 of 3)\n");
        // Unchanged is still silence.
        assert_eq!(v.render(4, &menu_rows(2), &menu_rows(2), ""), "");
    }

    /// SQ-0609. The guard that makes the above safe: a region that changes for
    /// any reason other than the marker is content, and content is emitted.
    #[test]
    fn screen_reader_mode_does_not_eat_a_region_that_really_changed() {
        let mut v = ScreenView::new(false, false, true, 24, 80);
        v.set_menus(true);
        v.render(4, &menu_rows(0), &menu_rows(0), "");

        // An item's text changed — the menu scrolled, or it is a different menu.
        let renamed: Vec<String> =
            menu_rows(1).iter().map(|r| r.replace("Documentation", "Manual")).collect();
        let out = v.render(4, &renamed, &renamed, "");
        assert!(out.contains("Manual"), "a changed item is read out, not announced: {out:?}");
        assert!(out.contains("Sample Transcript"), "with the rest of the block: {out:?}");

        // A status line is not a menu at all and never was.
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(false, false, false, 24, 80);
        v.set_menus(true);
        let out = v.render(1, &p, &a, "");
        assert!(out.contains("West of House"), "verbatim: {out:?}");
        assert!(!out.contains(cli_host::menu::MENU_HINT), "and not numbered: {out:?}");
        assert!(v.menu_listing().is_none(), "/menu has nothing to re-read");
    }

    #[test]
    fn status_now_answers_even_when_nothing_changed() {
        // `/status` (SQ-0610) is asked precisely because nothing changed and the
        // status has scrolled away, so it must not go through the dedupe that
        // `render` applies.
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(false, false, false, 24, 80);
        assert!(!v.render(1, &p, &a, "").is_empty(), "first frame emits");
        assert_eq!(v.render(1, &p, &a, ""), "", "second is deduped away");
        // ScreenView::status_now goes to the machine, which these row-level tests
        // do not build; what is pinned here is that the dedupe it must bypass is
        // real. The end-to-end behaviour is covered by the CLI's own runs.
    }

    #[test]
    fn start_clears_screen_only_when_interactive() {
        assert_eq!(ScreenView::new(true, false, false, 24, 80).start(), "\x1b[2J\x1b[H");
        assert_eq!(ScreenView::new(false, false, false, 24, 80).start(), ""); // piped
        assert_eq!(ScreenView::new(true, true, false, 24, 80).start(), ""); // --story-only
    }

    #[test]
    fn erase_clears_screen_and_resets_region_on_tty() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, false, 24, 80);
        let _ = v.render(1, &p, &a, ""); // activate a region (active_rows = 1)
        let out = v.erase(ZColour::Default, true, Palette::Standard);
        assert!(out.contains("\x1b[r"), "erase leaves the scroll region: {out:?}");
        assert!(out.contains("\x1b[2J"), "erase clears the screen: {out:?}");
        assert!(out.ends_with("\x1b[H"), "erase homes the cursor: {out:?}");
        // After erase the region is considered inactive, so the next frame
        // re-establishes it.
        let re = v.render(1, &p, &a, "");
        assert!(re.contains("\x1b[2;24r"), "next frame re-enters the region: {re:?}");
    }

    #[test]
    fn erase_is_noop_when_piped() {
        let mut v = ScreenView::new(false, false, false, 24, 80);
        assert_eq!(v.erase(ZColour::Default, true, Palette::Standard), "", "piped erase emits nothing");
    }

    #[test]
    fn tty_dropping_to_zero_rows_resets_region() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, false, 24, 80);
        let _ = v.render(1, &p, &a, ""); // activate region
        let out = v.render(0, &[], &[], "");
        assert!(out.contains("\x1b[r"), "dropping to 0 rows resets region: {out:?}");
    }

    /// **A hint menu opens and closes without confusing either placement**
    /// (SQ-0909).
    ///
    /// This is the case that actually exercises the pin: an InvisiClues menu, Lost
    /// Pig's HELP, Bureaucracy's form — the upper window grows from a one-row status
    /// bar to a dozen rows and back, mid-session, several times. Under a bottom pin
    /// the region has to shrink from the TOP edge staying put, which is the opposite
    /// of what a top pin does, so it is worth asserting rather than assuming.
    #[test]
    fn a_hint_menu_opening_and_closing_resizes_the_region_either_way() {
        let (p1, a1) = v3_rows();
        let menu: Vec<String> = (0..12).map(|i| format!("  {i} Chapter {i}")).collect();

        for (pin, opened, closed) in [
            // Top: the region's TOP edge moves, the bottom stays at 24.
            (Pin::Top, "\x1b[13;24r", "\x1b[2;24r"),
            // Bottom: the region starts at 1 throughout — which is what keeps the
            // history working — and its BOTTOM edge moves instead.
            (Pin::Bottom, "\x1b[1;12r", "\x1b[1;23r"),
        ] {
            let mut v = ScreenView::new(true, false, false, 24, 80);
            v.set_pin(pin);
            let _ = v.render(1, &p1, &a1, "");
            let open = v.render(12, &menu, &menu, "");
            assert!(open.contains(opened), "{pin:?}: menu opens: {open:?}");
            assert!(open.contains("Chapter 0"), "{pin:?}: and is drawn: {open:?}");
            // Re-rendering the same menu must not re-enter the region — that would
            // yank the cursor every frame while the player reads.
            let again = v.render(12, &menu, &menu, "");
            assert!(!again.contains(opened), "{pin:?}: settled menu does not re-pin: {again:?}");
            let shut = v.render(1, &p1, &a1, "");
            assert!(shut.contains(closed), "{pin:?}: menu closes: {shut:?}");
        }
    }

    /// A menu taller than the screen leaves a usable region rather than an inverted
    /// one — `enter_region` clamps instead of emitting `\x1b[1;0r`.
    #[test]
    fn a_full_screen_menu_does_not_invert_the_region() {
        let rows: Vec<String> = (0..30).map(|i| format!("row {i}")).collect();
        for pin in [Pin::Top, Pin::Bottom] {
            let mut v = ScreenView::new(true, false, false, 24, 80);
            v.set_pin(pin);
            let out = v.render(30, &rows, &rows, "");
            assert!(!out.contains(";0r"), "{pin:?}: no zero-height region: {out:?}");
            // Whatever it pins, the rows it paints must start on the screen.
            assert!(out.contains("\x1b[1;1H"), "{pin:?}: painting starts on row 1: {out:?}");
        }
    }

    #[test]
    fn erase_then_multirow_upper_shifts_lower_window() {
        // BeyondZork case: erase, then a game redraws a 12-row upper window with
        // a short lower prompt already streamed at the top. The next frame must
        // scroll the lower window down and follow the cursor below the region.
        let rows = vec![String::new(); 12];
        let mut v = ScreenView::new(true, false, false, 24, 80);
        let _ = v.erase(ZColour::Default, true, Palette::Standard); // arms the one-shot shift
        let out = v.render(12, &rows, &rows, "");
        assert!(out.contains("\x1b[12T"), "scrolls the display down by 12 (SD): {out:?}");
        assert!(out.contains("\x1b[12B"), "follows the cursor down by 12 (CUD): {out:?}");
        assert!(out.contains("\x1b[13;24r"), "pins the region below the upper window: {out:?}");
        // SD is wrapped in DECSC/DECRC so the terminal's SD cursor side-effect is
        // normalised before the relative CUD.
        assert!(
            out.contains("\x1b7\x1b[r\x1b[12T\x1b8\x1b[12B"),
            "SD is wrapped in DECSC/DECRC, then CUD: {out:?}"
        );
        // One-shot: a subsequent height change does NOT shift again.
        let again = v.render(6, &rows[..6], &rows[..6], "");
        assert!(!again.contains("\x1b[6T") && !again.contains("\x1b[6S"), "shift is one-shot: {again:?}");
    }

    #[test]
    fn erase_then_single_row_status_does_not_shift() {
        // A v3 game also erases at startup, but its 1-row status line (top == 1)
        // must NOT trigger a shift — it streams a full screen of narrative that
        // scrolling would garble.
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, false, 24, 80);
        let _ = v.erase(ZColour::Default, true, Palette::Standard);
        let out = v.render(1, &p, &a, "");
        assert!(!out.contains("\x1b[1T"), "1-row status line is not shifted: {out:?}");
        assert!(out.contains("\x1b[2;24r"), "still pins the status region: {out:?}");
    }

    #[test]
    fn multirow_upper_without_erase_does_not_shift() {
        // Continuous flow (no preceding erase): the content is already positioned
        // correctly, so a multi-row upper window must NOT scroll the lower window.
        let rows = vec![String::new(); 12];
        let mut v = ScreenView::new(true, false, false, 24, 80);
        let out = v.render(12, &rows, &rows, "");
        assert!(!out.contains("\x1b[12T"), "no erase means no shift: {out:?}");
        assert!(out.contains("\x1b[13;24r"), "still pins the region: {out:?}");
    }
}

#[cfg(test)]
mod colour_tests {
    use super::*;
    use zvm::io::TextAttrs;
    use zvm::screen::ZColour;

    #[test]
    fn sgr_open_builds_prefix_without_reset() {
        assert_eq!(sgr_open(TextAttrs::default(), Palette::Standard), "", "no attrs → empty");
        assert_eq!(sgr_open(TextAttrs { style: 2, ..Default::default() }, Palette::Standard), "\x1b[1m", "bold, no reset");
        let c = TextAttrs { style: 0, fg: ZColour::Standard(3), bg: ZColour::Default };
        assert_eq!(sgr_open(c, Palette::Standard), "\x1b[31m", "fg only, no trailing reset");
        // style_wrap composes sgr_open + reset.
        assert_eq!(style_wrap("x", c, true, Palette::Standard), "\x1b[31mx\x1b[0m");
    }

    #[test]
    fn bg_sgr_sets_background_only() {
        use zvm::screen::ZColour;
        assert_eq!(bg_sgr(ZColour::Standard(2), true, Palette::Standard), "\x1b[40m", "black bg");
        assert_eq!(bg_sgr(ZColour::Default, true, Palette::Standard), "", "default = no SGR");
        assert_eq!(bg_sgr(ZColour::Standard(2), false, Palette::Standard), "", "honor off = no SGR");
    }

    #[test]
    fn erase_paints_current_bg() {
        use zvm::screen::ZColour;
        let mut v = ScreenView::new(true, false, false, 24, 80);
        let out = v.erase(ZColour::Standard(2), true, Palette::Standard);
        assert!(out.contains("\x1b[40m"), "bg SGR before clear: {out:?}");
        assert!(out.contains("\x1b[2J"), "screen clear present: {out:?}");
        assert!(out.find("\x1b[40m").unwrap() < out.find("\x1b[2J").unwrap(),
            "bg set before the clear: {out:?}");
    }

    #[test]
    fn style_wrap_emits_colour_sgr() {
        // Pins an exact RGB for a Standard colour, which the process-wide
        // palette decides. No lock, because this binary's TEST build has no
        // writer to race with: the one `set_palette` call is on the boot path,
        // and printing the machine table stopped touching the global when it
        // moved to `zvm::machines` (SQ-0960). §8.3.1's own table is what resolves
        // here. The moment a case in this crate sets the palette, every reader
        // like this one needs a lock again — SQ-0904/0958.
        // standard fg=red(3)->31, bg=blue(6)->44
        let a = TextAttrs { style: 0, fg: ZColour::Standard(3), bg: ZColour::Standard(6) };
        assert_eq!(style_wrap("x", a, true, Palette::Standard), "\x1b[31;44mx\x1b[0m");
        // default channels emit nothing; no attrs → no wrap
        let d = TextAttrs { style: 0, fg: ZColour::Default, bg: ZColour::Default };
        assert_eq!(style_wrap("x", d, true, Palette::Standard), "x");
        // true colour fg
        let t = TextAttrs { style: 0, fg: ZColour::True(0x7FFF), bg: ZColour::Default };
        assert_eq!(style_wrap("x", t, true, Palette::Standard), "\x1b[38;2;255;255;255mx\x1b[0m");
        // grey 11 -> 8C8C8C (ZMSD §8.3.1: medium grey is true colour $4631;
        // this used to pin the invented #808080)
        let g = TextAttrs { style: 0, fg: ZColour::Standard(11), bg: ZColour::Default };
        assert_eq!(style_wrap("x", g, true, Palette::Standard), "\x1b[38;2;140;140;140mx\x1b[0m");
        // non-tty stays plain
        assert_eq!(style_wrap("x", a, false, Palette::Standard), "x");
    }
}

#[cfg(test)]
mod page_bg_tests {
    use zvm::screen::{Palette, ZColour};

    // The OSC escapes and their change-detection are covered in
    // `cli_host::term` now; what stays here is the Z-machine-specific question
    // of which ZColours have an RGB triple to paint with at all.

    #[test]
    fn zcolour_rgb_default_is_none_true24_unpacks() {
        assert_eq!(super::zcolour_rgb(ZColour::Default, Palette::Standard), None);
        assert_eq!(super::zcolour_rgb(ZColour::True24(0x123456), Palette::Standard), Some((0x12, 0x34, 0x56)));
    }

    #[test]
    fn zcolour_rgb_standard_and_true() {
        // Pins an exact RGB for a Standard colour, which the process-wide
        // palette decides. No lock, because this binary's TEST build has no
        // writer to race with: the one `set_palette` call is on the boot path,
        // and printing the machine table stopped touching the global when it
        // moved to `zvm::machines` (SQ-0960). §8.3.1's own table is what resolves
        // here. The moment a case in this crate sets the palette, every reader
        // like this one needs a lock again — SQ-0904/0958.
        // 2..=9 are scheme-relative in push_colour_sgr (bare ANSI code, no RGB
        // source) — no invented palette here either.
        assert_eq!(super::zcolour_rgb(ZColour::Standard(3), Palette::Standard), None);
        // 10..=12 resolve via the shared grey_rgb table, same as push_colour_sgr.
        // ZMSD §8.3.1 fixes medium grey (11) at true colour $4631 → #8C8C8C.
        // (This assertion previously pinned the invented #808080.)
        assert_eq!(super::zcolour_rgb(ZColour::Standard(11), Palette::Standard), Some((0x8C, 0x8C, 0x8C)));
        // True colour goes through rgb15_to_888, same as push_colour_sgr.
        assert_eq!(super::zcolour_rgb(ZColour::True(0x001F), Palette::Standard), Some((255, 0, 0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvm::screen::{StatusLine, StatusRight, UpperWindow, ZColour};

    #[test]
    fn style_wrap_only_when_tty_and_styled() {
        use zvm::io::TextAttrs;
        assert_eq!(style_wrap("hi", TextAttrs { style: 0, ..Default::default() }, true, Palette::Standard), "hi");
        assert_eq!(style_wrap("hi", TextAttrs { style: 2, ..Default::default() }, false, Palette::Standard), "hi");
        assert_eq!(style_wrap("hi", TextAttrs { style: 2, ..Default::default() }, true, Palette::Standard), "\x1b[1mhi\x1b[0m");
    }

    #[test]
    fn bleep_bytes_tty_gated() {
        assert_eq!(bleep_bytes(3, true), "\x07\x07\x07");
        assert_eq!(bleep_bytes(3, false), "");
        assert_eq!(bleep_bytes(0, true), "");
    }

    #[test]
    fn status_text_pads_and_right_aligns() {
        let st = StatusLine {
            location: "West of House".into(),
            right: StatusRight::ScoreTurns { score: 0, turns: 1 },
        };
        let row = status_text(&st, 40);
        assert_eq!(row.chars().count(), 40, "padded to width");
        assert!(row.starts_with(" West of House"), "location left: {row:?}");
        assert!(row.trim_end().ends_with("Moves: 1"), "right field: {row:?}");
    }

    #[test]
    fn status_text_truncates_long_location() {
        let st = StatusLine {
            location: "x".repeat(100),
            right: StatusRight::Time { hours: 9, minutes: 5 },
        };
        let row = status_text(&st, 20);
        assert_eq!(row.chars().count(), 20);
        assert!(row.contains("09:05"));
    }

    #[test]
    fn upper_row_text_and_ansi() {
        let mut u = UpperWindow::default();
        u.resize(1, 5);
        u.put(1, 1, 'H', 0, ZColour::Default, ZColour::Default);
        u.put(1, 2, 'i', 2, ZColour::Default, ZColour::Default); // bold
        let text = upper_row_text(&u, 1);
        assert_eq!(text, "Hi"); // trailing blanks trimmed
        let ansi = upper_row_ansi(&u, 1, true, ZColour::Default, ZColour::Default, Palette::Standard);
        assert!(
            ansi.contains("\x1b[1m") && ansi.ends_with("\x1b[0m"),
            "ansi: {ansi:?}"
        );
    }

    #[test]
    fn upper_row_ansi_emits_per_cell_fg_bg() {
        use zvm::screen::ZColour;
        let mut u = UpperWindow::default();
        u.resize(1, 6);
        // "Hi" in red-on-black, honor on.
        u.put(1, 1, 'H', 0, ZColour::Standard(3), ZColour::Standard(2));
        u.put(1, 2, 'i', 0, ZColour::Standard(3), ZColour::Standard(2));
        let on = upper_row_ansi(&u, 1, true, ZColour::Default, ZColour::Default, Palette::Standard);
        assert!(on.contains("31"), "red fg SGR present: {on:?}");
        assert!(on.contains("40"), "black bg SGR present: {on:?}");
        assert!(on.contains("Hi"), "text present: {on:?}");
        // honor off: no colour SGR, text still present.
        let off = upper_row_ansi(&u, 1, false, ZColour::Default, ZColour::Default, Palette::Standard);
        assert!(!off.contains("31") && !off.contains("40"), "no colour when honor off: {off:?}");
        assert!(off.contains("Hi"), "text present when honor off: {off:?}");
    }

    #[test]
    fn upper_row_ansi_paints_default_cells_with_current_bg() {
        use zvm::screen::ZColour;
        let mut u = UpperWindow::default();
        u.resize(1, 6);
        // cols 1-2 blank (Default bg); cols 3-4 'Hi' explicit white-on-black.
        u.put(1, 3, 'H', 0, ZColour::Standard(9), ZColour::Standard(2));
        u.put(1, 4, 'i', 0, ZColour::Standard(9), ZColour::Standard(2));
        // Screen background is black: leading blank cells must be painted black
        // (bg 40) BEFORE the text, not left to reset-to-terminal-default.
        let out = upper_row_ansi(&u, 1, true, ZColour::Default, ZColour::Standard(2), Palette::Standard);
        // The text run at col 3 emits its own bg 40, so a plain "40 before H"
        // check would pass even unfixed. Discriminate: the segment BEFORE the
        // first (leading) space must already carry bg 40, i.e. the leading
        // blank cells are painted with the screen background rather than a bare
        // reset. Unfixed, the leading run is a lone `ESC[0m` with no 40.
        let first_space = out.find(' ').expect("row has leading spaces");
        assert!(
            out[..first_space].contains("40"),
            "leading blank cells carry the screen bg before the spaces: {out:?}"
        );
        assert!(out.contains('H'), "text preserved: {out:?}");
    }

}
