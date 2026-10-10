//! The renderer-neutral colour decision for the player's typed command in the
//! transcript (SQ-1758, exposed for embedding hosts by SQ-1759).
//!
//! A transcript line's typed command carries a `StyleRun` whose `glk_style` is
//! [`crate::state::GLK_STYLE_TYPED_INPUT`]; `bits`, `fg`, `bg` and `ink` are packed
//! ZColours (`state::pack_zcolour`). [`typed_input_colours`] answers WHICH SOURCE
//! each part of the final style comes from; the host resolves a source to its own
//! colour type. The terminal renderer is a thin adapter over this call, so a
//! non-terminal host that calls it draws the same thing.
//!
//! The decision, in order:
//! * `bits` are the run's, in both `honor_game_colours` modes.
//! * foreground: the story's `ink` when honor is on and it is not Default, else
//!   the theme's input colour.
//! * background: the prompt line's own `bg` when honor is on and it is not Default
//!   (keeping the band continuous), else the theme/line background.
//! * with honor on, if the chosen foreground against the background has a
//!   contrast below [`crate::colors::MIN_INPUT_CONTRAST`], the foreground becomes
//!   the prompt's (`run.fg` if set, else the line's base foreground).

use crate::colors::{contrast_ratio_rgb, MIN_INPUT_CONTRAST};
use crate::state::{unpack_zcolour, StyleRun};
use zvm::screen::ZColour;

/// Where the typed command's foreground comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InkSource {
    /// The theme's `transcript_input` foreground (over the line style).
    Theme,
    /// The story's own input colour (packed ZColour, never Default).
    Game(u32),
    /// The prompt line's own foreground (packed ZColour, never Default).
    Prompt(u32),
    /// The line's base foreground.
    LineBase,
}

/// Where the typed command's background comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgSource {
    /// The theme/line background (nothing is painted over it).
    Theme,
    /// The prompt line's game background (packed ZColour, never Default).
    Game(u32),
}

/// A source the host is asked to resolve to RGB for the contrast check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Fg(InkSource),
    Bg(BgSource),
}

/// The resolved decision: sources, not a renderer's style type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedInputColours {
    pub fg: InkSource,
    pub bg: BgSource,
    /// Style bits (1=reverse, 2=bold, 4=italic, 8=fixed), the run's own.
    pub bits: u8,
}

/// Decide the typed command's colours for `run`.
///
/// `rgb_of` resolves a [`Probe`] to sRGB for the contrast check; `None` means
/// unknown (a terminal-palette colour, or no colour at all), which disables the
/// fallback. `line_base_fg` says whether the line has a base foreground at all;
/// without one there is nothing to fall back to.
pub fn typed_input_colours(
    run: &StyleRun,
    honor_game_colours: bool,
    line_base_fg: bool,
    rgb_of: impl Fn(Probe) -> Option<(u8, u8, u8)>,
) -> TypedInputColours {
    let set = |packed: u32| (!matches!(unpack_zcolour(packed), ZColour::Default)).then_some(packed);
    let mut out = TypedInputColours { fg: InkSource::Theme, bg: BgSource::Theme, bits: run.bits };
    if !honor_game_colours {
        return out;
    }
    if let Some(p) = set(run.ink) {
        out.fg = InkSource::Game(p);
    }
    if let Some(p) = set(run.bg) {
        out.bg = BgSource::Game(p);
    }
    let fallback = match set(run.fg) {
        Some(p) => Some(InkSource::Prompt(p)),
        None => line_base_fg.then_some(InkSource::LineBase),
    };
    if let (Some(fg), Some(bg), Some(fallback)) = (rgb_of(Probe::Fg(out.fg)), rgb_of(Probe::Bg(out.bg)), fallback) {
        if contrast_ratio_rgb(fg, bg) < MIN_INPUT_CONTRAST {
            out.fg = fallback;
        }
    }
    out
}

#[cfg(all(test, feature = "t-theme"))]
mod tests {
    use super::*;
    use crate::state::pack_zcolour;

    const BLACK: (u8, u8, u8) = (0, 0, 0);
    const WHITE: (u8, u8, u8) = (255, 255, 255);
    fn z(r: u8, g: u8, b: u8) -> u32 {
        pack_zcolour(ZColour::True24(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)))
    }
    fn run() -> StyleRun {
        StyleRun { bits: 2, ..StyleRun::default() }
    }
    /// Theme fg white, theme bg black, everything else unknown.
    fn rgb(p: Probe) -> Option<(u8, u8, u8)> {
        match p {
            Probe::Fg(InkSource::Theme) => Some(WHITE),
            Probe::Bg(BgSource::Theme) => Some(BLACK),
            _ => None,
        }
    }

    #[test]
    fn honor_off_is_theme_with_bits() {
        let r = StyleRun { ink: z(1, 2, 3), bg: z(4, 5, 6), fg: z(7, 8, 9), ..run() };
        let c = typed_input_colours(&r, false, true, rgb);
        assert_eq!(c, TypedInputColours { fg: InkSource::Theme, bg: BgSource::Theme, bits: 2 });
    }

    #[test]
    fn honor_on_story_ink_is_game() {
        let r = StyleRun { ink: z(1, 2, 3), ..run() };
        assert_eq!(typed_input_colours(&r, true, true, rgb).fg, InkSource::Game(z(1, 2, 3)));
    }

    #[test]
    fn honor_on_no_ink_is_theme() {
        assert_eq!(typed_input_colours(&run(), true, true, rgb).fg, InkSource::Theme);
    }

    #[test]
    fn honor_on_game_bg_is_kept() {
        let r = StyleRun { bg: z(4, 5, 6), ..run() };
        assert_eq!(typed_input_colours(&r, true, true, |_| None).bg, BgSource::Game(z(4, 5, 6)));
    }

    #[test]
    fn low_contrast_falls_back_to_prompt_fg() {
        let r = StyleRun { fg: z(9, 9, 9), ..run() };
        let c = typed_input_colours(&r, true, true, |_| Some(WHITE));
        assert_eq!(c.fg, InkSource::Prompt(z(9, 9, 9)));
    }

    #[test]
    fn low_contrast_without_prompt_fg_falls_back_to_line_base() {
        let c = typed_input_colours(&run(), true, true, |_| Some(WHITE));
        assert_eq!(c.fg, InkSource::LineBase);
        // No line base foreground either: nothing to fall back to.
        assert_eq!(typed_input_colours(&run(), true, false, |_| Some(WHITE)).fg, InkSource::Theme);
    }

    #[test]
    fn unknown_rgb_means_no_fallback() {
        let r = StyleRun { fg: z(9, 9, 9), ..run() };
        assert_eq!(typed_input_colours(&r, true, true, |_| None).fg, InkSource::Theme);
    }
}
