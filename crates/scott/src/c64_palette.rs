//! The sixteen colours a Commodore 64 shows, in VIC-II index order.
//!
//! Every Commodore 64 picture family in this crate — Family B's line-drawn
//! *Mysterious Adventures* artwork ([`crate::c64`]) and Family C's US S.A.G.A.
//! bitmaps ([`crate::saga_pictures`]) — ends up naming a **VIC-II colour
//! index**, and both resolve it here so the two cannot disagree about what
//! "red" is.
//!
//! # Provenance
//!
//! These are Philip "Pepto" Timmermann's measured VIC-II colours (the analysis
//! published at <https://www.pepto.de/projects/colorvic/>, which VICE ships as
//! its default palette) — a public measurement of the chip's composite output,
//! not code, and nothing here is derived from any interpreter.
//!
//! **Eight of the sixteen are confirmed against real-machine captures in this
//! repository** (SQ-1491): `machine-screenshots/c64-hulk-{splash,start,
//! transform,chamber}.png` are the *Hulk*'s own Commodore 64 release under
//! VICE, and every pixel of all four resolves to one of these triples exactly
//! — black, white, red, purple, green, blue and orange between them, plus
//! yellow off `c64-hulk-colorbars.png`. The remaining eight (cyan, brown,
//! light red, dark grey, grey, light green, light blue, light grey) are the
//! published palette's and no capture in this repository exercises them yet.
//!
//! Before SQ-1491 the two families carried two different, brighter, hand-made
//! tables; see the appendix item named in
//! [`docs/internals/scott-dialects-spec.md`](../../../docs/internals/scott-dialects-spec.md)
//! §8.3.

use crate::saga_pictures::Rgb;

/// The VIC-II palette, indexed by hardware colour number 0-15.
///
/// See the module documentation for provenance and for which entries a
/// real-machine capture confirms.
pub const PEPTO_PALETTE: [Rgb; 16] = [
    (0x00, 0x00, 0x00), // 0  black
    (0xFF, 0xFF, 0xFF), // 1  white
    (0x68, 0x37, 0x2B), // 2  red
    (0x70, 0xA4, 0xB2), // 3  cyan
    (0x6F, 0x3D, 0x86), // 4  purple
    (0x58, 0x8D, 0x43), // 5  green
    (0x35, 0x28, 0x79), // 6  blue
    (0xB8, 0xC7, 0x6F), // 7  yellow
    (0x6F, 0x4F, 0x25), // 8  orange
    (0x43, 0x39, 0x00), // 9  brown
    (0x9A, 0x67, 0x59), // 10 light red
    (0x44, 0x44, 0x44), // 11 dark grey
    (0x6C, 0x6C, 0x6C), // 12 grey
    (0x9A, 0xD2, 0x84), // 13 light green
    (0x6C, 0x5E, 0xB5), // 14 light blue
    (0x95, 0x95, 0x95), // 15 light grey
];

#[cfg(test)]
mod tests {
    use super::PEPTO_PALETTE;

    #[test]
    fn the_palette_is_peptos_measured_vic_ii_table() {
        // Asserted by value because every other Commodore 64 colour claim in
        // this crate is now relative to it: if this table moves, so does the
        // meaning of every picture, and the four real-machine captures named
        // in the module header stop matching.
        assert_eq!(
            PEPTO_PALETTE,
            [
                (0x00, 0x00, 0x00),
                (0xFF, 0xFF, 0xFF),
                (0x68, 0x37, 0x2B),
                (0x70, 0xA4, 0xB2),
                (0x6F, 0x3D, 0x86),
                (0x58, 0x8D, 0x43),
                (0x35, 0x28, 0x79),
                (0xB8, 0xC7, 0x6F),
                (0x6F, 0x4F, 0x25),
                (0x43, 0x39, 0x00),
                (0x9A, 0x67, 0x59),
                (0x44, 0x44, 0x44),
                (0x6C, 0x6C, 0x6C),
                (0x9A, 0xD2, 0x84),
                (0x6C, 0x5E, 0xB5),
                (0x95, 0x95, 0x95),
            ]
        );
    }

    #[test]
    fn the_eight_entries_the_captures_confirm() {
        // SQ-1491: these eight are the ones a pixel of a real-machine frame
        // resolves to; see the module header for which frame.
        assert_eq!(PEPTO_PALETTE[0], (0, 0, 0), "black");
        assert_eq!(PEPTO_PALETTE[1], (255, 255, 255), "white");
        assert_eq!(PEPTO_PALETTE[2], (104, 55, 43), "red");
        assert_eq!(PEPTO_PALETTE[4], (111, 61, 134), "purple");
        assert_eq!(PEPTO_PALETTE[5], (88, 141, 67), "green");
        assert_eq!(PEPTO_PALETTE[6], (53, 40, 121), "blue");
        assert_eq!(PEPTO_PALETTE[7], (184, 199, 111), "yellow");
        assert_eq!(PEPTO_PALETTE[8], (111, 79, 37), "orange");
    }
}
