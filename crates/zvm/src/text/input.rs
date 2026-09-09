//! ZSCII *input* codes — the keyboard side of ZSCII (ZMSD §3.8), kept apart
//! from [`crate::text::decode`]'s *output* concerns (decoding story text,
//! encoding dictionary words).
//!
//! [`ZsciiInput`] is a newtype so an embedder cannot hand
//! [`crate::cpu::exec::Machine::supply_char`] a ZSCII code the standard never
//! defines for input at all — a byte outside §3.8's Table 2 used to reach
//! `supply_char`, get validated at runtime, and get dropped with a
//! diagnostic. Now the invalid value is simply not constructible, so the
//! check that used to live at runtime lives in the type instead (2026-09
//! zvm reference audit; SQ-1426).
//!
//! # The §3.8 input set, verified against the Z-Machine Standards Document
//! (<https://inform-fiction.org/zmachine/standards/z1point1/sect03.html>),
//! Table 2 and its surrounding prose
//!
//! | codes | meaning | input? |
//! |---|---|---|
//! | 8 | delete | yes |
//! | 13 | newline / carriage return | yes |
//! | 27 | escape | yes |
//! | 32–126 | standard ASCII | yes |
//! | 129–132 | cursor up/down/left/right | yes |
//! | 133–144 | function keys f1–f12 | yes |
//! | 145–154 | keypad 0–9 | yes |
//! | 155–251 | extra characters (via the Unicode translation table, §3.8.5.4) | yes |
//! | 252 | menu click (v6) | yes |
//! | 253 | mouse double-click | yes |
//! | 254 | mouse single-click | yes |
//!
//! Everything else — 0–7, 9–12, 10 (line feed) included, 14–26, 28–31,
//! 127–128, 255 — has no input meaning. 10 is the one surprise worth calling
//! out: it prints as a sentence-space in v6 output (§3.8.2.3) but the
//! standard never defines it as a keystroke, which is why Return is ZSCII 13
//! and not 10 (SQ-1419, SQ-1423). 155–251 *are* legal input, contrary to an
//! earlier reading of this table in `cpu::exec` — §3.8.5.4 says a story's
//! extra characters "are entirely normal ZSCII characters" once the header's
//! Unicode translation table defines them, on both the input and the output
//! side; nothing in the standard restricts them to output only.
use crate::text::decode::char_to_zscii_default;

/// A ZSCII code legal as *input* to [`crate::cpu::exec::Machine::supply_char`]
/// (ZMSD §3.8, Table 2). See the module docs for the verified range table.
///
/// Constructible only through [`Self::new`], [`Self::from_char`], or one of
/// the named constants — so a `ZsciiInput` in hand is always one of the
/// codes the standard defines for a keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ZsciiInput(u8);

impl ZsciiInput {
    /// ZSCII 8: delete / backspace.
    pub const DELETE: ZsciiInput = ZsciiInput(8);
    /// ZSCII 13: newline (Return/Enter on input). 10 (line feed) is NOT a
    /// legal input code — see [`Self::from_char`] for the convenience that
    /// normalises a typed `'\n'` to this instead.
    pub const NEWLINE: ZsciiInput = ZsciiInput(13);
    /// ZSCII 27: escape.
    pub const ESCAPE: ZsciiInput = ZsciiInput(27);

    /// ZSCII 129: cursor up.
    pub const UP: ZsciiInput = ZsciiInput(129);
    /// ZSCII 130: cursor down.
    pub const DOWN: ZsciiInput = ZsciiInput(130);
    /// ZSCII 131: cursor left.
    pub const LEFT: ZsciiInput = ZsciiInput(131);
    /// ZSCII 132: cursor right.
    pub const RIGHT: ZsciiInput = ZsciiInput(132);

    /// ZSCII 133–144: function keys f1–f12.
    pub const F1: ZsciiInput = ZsciiInput(133);
    pub const F2: ZsciiInput = ZsciiInput(134);
    pub const F3: ZsciiInput = ZsciiInput(135);
    pub const F4: ZsciiInput = ZsciiInput(136);
    pub const F5: ZsciiInput = ZsciiInput(137);
    pub const F6: ZsciiInput = ZsciiInput(138);
    pub const F7: ZsciiInput = ZsciiInput(139);
    pub const F8: ZsciiInput = ZsciiInput(140);
    pub const F9: ZsciiInput = ZsciiInput(141);
    pub const F10: ZsciiInput = ZsciiInput(142);
    pub const F11: ZsciiInput = ZsciiInput(143);
    pub const F12: ZsciiInput = ZsciiInput(144);

    /// ZSCII 145–154: keypad 0–9.
    pub const KEYPAD_0: ZsciiInput = ZsciiInput(145);
    pub const KEYPAD_1: ZsciiInput = ZsciiInput(146);
    pub const KEYPAD_2: ZsciiInput = ZsciiInput(147);
    pub const KEYPAD_3: ZsciiInput = ZsciiInput(148);
    pub const KEYPAD_4: ZsciiInput = ZsciiInput(149);
    pub const KEYPAD_5: ZsciiInput = ZsciiInput(150);
    pub const KEYPAD_6: ZsciiInput = ZsciiInput(151);
    pub const KEYPAD_7: ZsciiInput = ZsciiInput(152);
    pub const KEYPAD_8: ZsciiInput = ZsciiInput(153);
    pub const KEYPAD_9: ZsciiInput = ZsciiInput(154);

    /// ZSCII 252: menu click (v6).
    pub const MENU_CLICK: ZsciiInput = ZsciiInput(252);
    /// ZSCII 253: mouse double-click.
    pub const MOUSE_DOUBLE_CLICK: ZsciiInput = ZsciiInput(253);
    /// ZSCII 254: mouse single-click.
    pub const MOUSE_CLICK: ZsciiInput = ZsciiInput(254);

    /// Construct a `ZsciiInput` from a raw ZSCII byte, or `None` if `code` is
    /// not one of the codes ZMSD §3.8's Table 2 defines for input. See the
    /// module docs for the exact ranges.
    pub const fn new(code: u8) -> Option<Self> {
        match code {
            8 | 13 | 27 | 32..=126 | 129..=132 | 133..=144 | 145..=154 | 155..=251 | 252..=254 => {
                Some(ZsciiInput(code))
            }
            _ => None,
        }
    }

    /// Construct a `ZsciiInput` from a typed character, via the default ZSCII
    /// translation (ZMSD §3.8) — the same rule
    /// [`crate::text::decode::char_to_zscii_default`] uses for dictionary
    /// word encoding, so a typed character resolves to the same ZSCII code
    /// wherever it is turned into one.
    ///
    /// `'\n'` maps to [`Self::NEWLINE`] (ZSCII 13) as a convenience: a host
    /// forwarding an unprocessed line-feed byte from piped stdin or a
    /// platform newline gets Return rather than nothing (SQ-1419's rule,
    /// moved here from `supply_char` itself). A character with no ZSCII
    /// mapping at all falls back to `'?'` (63), matching
    /// `char_to_zscii_default`'s own fallback — the same "no such glyph"
    /// answer a story would give back from its own dictionary encoder.
    pub fn from_char(c: char) -> Option<Self> {
        Self::new(char_to_zscii_default(c))
    }

    /// The raw ZSCII byte this input represents (ZMSD §3.8).
    pub const fn code(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_accepts_every_table_2_code() {
        let accepted: &[u8] = &[8, 13, 27];
        for &c in accepted {
            assert!(ZsciiInput::new(c).is_some(), "ZSCII {c} must be accepted");
        }
        for c in 32..=126u8 {
            assert!(ZsciiInput::new(c).is_some(), "ASCII {c} must be accepted");
        }
        for c in 129..=132u8 {
            assert!(ZsciiInput::new(c).is_some(), "cursor key {c} must be accepted");
        }
        for c in 133..=144u8 {
            assert!(ZsciiInput::new(c).is_some(), "function key {c} must be accepted");
        }
        for c in 145..=154u8 {
            assert!(ZsciiInput::new(c).is_some(), "keypad key {c} must be accepted");
        }
        for c in 155..=251u8 {
            assert!(ZsciiInput::new(c).is_some(), "extra character {c} must be accepted");
        }
        for c in 252..=254u8 {
            assert!(ZsciiInput::new(c).is_some(), "mouse/menu code {c} must be accepted");
        }
    }

    #[test]
    fn new_rejects_every_gap() {
        // The undefined stretches between and around the accepted ranges
        // (ZMSD §3.8, Table 2), plus the two single-code surprises: 10 (line
        // feed, output-only) and 255.
        let rejected: &[u8] =
            &[0, 1, 7, 9, 10, 11, 12, 14, 20, 26, 28, 31, 127, 128, 255];
        for &c in rejected {
            assert!(ZsciiInput::new(c).is_none(), "ZSCII {c} must be rejected");
        }
    }

    #[test]
    fn every_accepted_range_starts_and_ends_where_the_table_says() {
        // 129–132, 133–144, 145–154, 155–251 and 252–254 all run back-to-back
        // with no gap between them (cursor keys straight into function keys
        // straight into keypad straight into extra characters straight into
        // mouse/menu codes), so only the two truly isolated ranges — 32–126,
        // and the run from 129 to 254 as a whole — have a rejected code on
        // either side.
        for &lo_hi in &[32u8, 126, 129, 132, 133, 144, 145, 154, 155, 251, 252, 254] {
            assert!(ZsciiInput::new(lo_hi).is_some(), "{lo_hi} must be accepted");
        }
        assert!(ZsciiInput::new(31).is_none(), "31 (just below the ASCII range) must be rejected");
        assert!(ZsciiInput::new(127).is_none(), "127 (just above the ASCII range) must be rejected");
        assert!(ZsciiInput::new(128).is_none(), "128 (just below the cursor keys) must be rejected");
        assert!(ZsciiInput::new(255).is_none(), "255 (just above mouse/menu) must be rejected");
    }

    #[test]
    fn from_char_newline_is_zscii_13() {
        assert_eq!(ZsciiInput::from_char('\n'), Some(ZsciiInput::NEWLINE));
    }

    #[test]
    fn from_char_ascii_round_trips() {
        assert_eq!(ZsciiInput::from_char('A'), ZsciiInput::new(b'A'));
        assert_eq!(ZsciiInput::from_char('?'), ZsciiInput::new(b'?'));
    }

    #[test]
    fn from_char_unmapped_falls_back_to_question_mark() {
        // Matches `char_to_zscii_default`'s own fallback (no default Unicode
        // translation table entry for '€').
        assert_eq!(ZsciiInput::from_char('€'), Some(ZsciiInput::new(b'?').unwrap()));
    }

    #[test]
    fn code_round_trips() {
        assert_eq!(ZsciiInput::DELETE.code(), 8);
        assert_eq!(ZsciiInput::NEWLINE.code(), 13);
        assert_eq!(ZsciiInput::ESCAPE.code(), 27);
        assert_eq!(ZsciiInput::UP.code(), 129);
        assert_eq!(ZsciiInput::DOWN.code(), 130);
        assert_eq!(ZsciiInput::LEFT.code(), 131);
        assert_eq!(ZsciiInput::RIGHT.code(), 132);
        assert_eq!(ZsciiInput::F1.code(), 133);
        assert_eq!(ZsciiInput::F12.code(), 144);
        assert_eq!(ZsciiInput::KEYPAD_0.code(), 145);
        assert_eq!(ZsciiInput::KEYPAD_9.code(), 154);
        assert_eq!(ZsciiInput::MENU_CLICK.code(), 252);
        assert_eq!(ZsciiInput::MOUSE_DOUBLE_CLICK.code(), 253);
        assert_eq!(ZsciiInput::MOUSE_CLICK.code(), 254);
    }
}
