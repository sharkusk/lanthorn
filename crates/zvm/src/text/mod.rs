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

pub mod cp437;

pub mod decode;
pub use decode::{decode_string, decode_string_words};

pub mod encode;
pub use encode::{encode_word, encode_word_mem};
