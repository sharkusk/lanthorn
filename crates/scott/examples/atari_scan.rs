//! Report what `scott::saga_atari` finds on an Atari companion picture side.
//!
//! Development scaffolding for SQ-1483: `cargo run -p lanthorn-scott --example
//! atari_scan -- <side-b.atr> [nl|std] [<file-offset> <out.ppm>]` prints every
//! record's offset, size, geometry and colour bytes, and with the last two
//! arguments writes one record out as a binary PPM. The specimen suite pins
//! the numbers; this is how they were read off, and the PPM is how a decode
//! gets looked at.
//!
//! PPM rather than PNG because `scott` takes no dependencies and a PNG needs
//! a deflate stream; any image viewer or `sips` will convert one.

use scott::saga_atari::{decode_record, scan_picture_side, splice_vtoc};
use scott::saga_pictures::{FamilyCScheme, CANVAS_HEIGHT, CANVAS_WIDTH};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: atari_scan <side-b.atr> [nl|std] [<file-offset> <out.ppm>]");
        std::process::exit(2);
    };
    let scheme = match args.next().as_deref() {
        Some("std") => FamilyCScheme::Standard,
        _ => FamilyCScheme::NoLiteral,
    };
    let raw = std::fs::read(&path).expect("read the side");
    let spliced = splice_vtoc(&raw);
    let found = scan_picture_side(&raw, scheme);

    let want: Option<usize> = args.next().and_then(|a| {
        let a = a.trim_start_matches("0x");
        usize::from_str_radix(a, 16).ok()
    });
    if let (Some(at), Some(out)) = (want, args.next()) {
        let r = found
            .iter()
            .find(|r| r.file_offset() == at)
            .unwrap_or_else(|| panic!("no record at 0x{at:05X}"));
        let pic = decode_record(&spliced, r, scheme).expect("decodes");
        // A doubled-height PPM, because a family-C pixel row is half as tall
        // as it is wide on the machines this artwork was drawn for.
        let mut ppm = format!("P6\n{} {}\n255\n", CANVAS_WIDTH, CANVAS_HEIGHT).into_bytes();
        for y in 0..CANVAS_HEIGHT {
            for x in 0..CANVAS_WIDTH {
                let (r, g, b) = pic.palette[usize::from(pic.pixels[y * CANVAS_WIDTH + x])];
                ppm.extend_from_slice(&[r, g, b]);
            }
        }
        std::fs::write(&out, &ppm).expect("write the ppm");
        println!("wrote {out} from the record at 0x{at:05X}");
        return;
    }
    println!("{path}: {} records under {scheme:?}", found.len());
    let mut prev_end = None;
    for (n, r) in found.iter().enumerate() {
        let gap = prev_end.map_or(0, |e| r.offset - e);
        let pic = decode_record(&spliced, r, scheme).expect("decodes");
        let mut seen = [0usize; 4];
        for y in r.layout.top..r.layout.top + r.layout.pairs * 2 {
            for x in r.layout.left..r.layout.left + r.layout.cols * 8 {
                if (0..CANVAS_WIDTH as i32).contains(&x) && (0..160).contains(&y) {
                    seen[usize::from(pic.pixels[y as usize * CANVAS_WIDTH + x as usize])] += 1;
                }
            }
        }
        println!(
            "  #{n:3} 0x{:05X} size={:5} slack={} gap={gap:2} {:2}x{:2} at ({:4},{:3}) col={:02X?} hist={seen:?}",
            r.file_offset(),
            r.size,
            r.size - r.decoded_len,
            r.layout.cols,
            r.layout.pairs,
            r.layout.left,
            r.layout.top,
            r.colour_bytes,
        );
        prev_end = Some(r.offset + r.size);
    }
}
