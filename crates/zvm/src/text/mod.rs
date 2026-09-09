//! Z-machine text subsystem.

// Default alphabet tables (ZMSD §3.5.3).
// Each table covers Z-chars 6–31 (26 entries; index = Z-char − 6).
//
// A0: lowercase a–z
pub(crate) const A0: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";
// A1: uppercase A–Z
pub(crate) const A1: &[u8; 26] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
// A2: Z-char 6 = 10-bit ZSCII escape (0x00 placeholder, handled specially)
//     Z-char 7 = newline, Z-chars 8–17 = 0–9, Z-chars 18–31 = punctuation
pub(crate) const A2: &[u8; 26] = b"\x00\n0123456789.,!?_#'\"/\\-:()";
// A2 in Version 1 ONLY (ZMSD §3.5.4): "Version 1 has a slightly different A2
// row in its alphabet table (new-line is not needed, making room for the `<`
// character)". Version 1 prints a newline from Z-char 1 in any alphabet
// (§3.5.2) instead, so the whole row shifts one place left and `<` joins the
// punctuation at what is Z-char 27 here. Z-char 6 is still the 10-bit ZSCII
// escape (§3.4 is not qualified by version, and both Frotz's `text.c` and
// Bocfel's `screen.cpp` take it in every version), so index 0 stays a
// placeholder; index 1 is the digit `0`, where every later version has `^`.
pub(crate) const A2_V1: &[u8; 26] = b"\x000123456789.,!?_#'\"/\\<-:()";

/// The A2 row this `version` reads (ZMSD §3.5.3 / §3.5.4). A0 and A1 are the
/// same in every version, so only A2 needs asking.
pub(crate) fn a2_row(version: u8) -> &'static [u8; 26] {
    if version == 1 { A2_V1 } else { A2 }
}

pub mod cp437;

pub mod decode;
pub use decode::{decode_string, decode_string_words};

pub mod encode;
pub use encode::{encode_word, encode_word_mem};

pub mod input;
pub use input::ZsciiInput;
