//! Count what `scott::saga_atari` finds on every Atari companion side, under
//! both schemes.
//!
//! Development scaffolding for SQ-1483, one line per side, so that the
//! specimen suite's thresholds are measured rather than guessed at:
//! `cargo run -p lanthorn-scott --example atari_survey -- <dir>`.

use scott::saga_atari::scan_picture_side;
use scott::saga_pictures::FamilyCScheme;

const SIDES: [&str; 7] = [
    "SAGA #1 - Adventureland [side B].atr",
    "SAGA #2 - Pirate Adventure [side B].atr",
    "SAGA #3 - Mission Impossible [side B].atr",
    "SAGA #4 - Voodoo Castle [side B].atr",
    "SAGA #5 - The Count [side B].atr",
    "SAGA #6 - Strange Odyssey [side B].atr",
    "SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr",
];

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "stories/scott-dialects/atari".into());
    for name in SIDES {
        let path = std::path::Path::new(&dir).join(name);
        let Ok(raw) = std::fs::read(&path) else {
            println!("  (absent) {name}");
            continue;
        };
        let std_n = scan_picture_side(&raw, FamilyCScheme::Standard).len();
        let nl_n = scan_picture_side(&raw, FamilyCScheme::NoLiteral).len();
        println!("  standard {std_n:3}   no-literal {nl_n:3}   {name}");
    }
}
