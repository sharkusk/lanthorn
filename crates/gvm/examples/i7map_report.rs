//! SQ-1303 measurement driver: what `gvm::i7map` recovers from a story file
//! alone, with no turn played. `cargo run -p lanthorn-gvm --example i7map_report -- <story>`
use gvm::i7map::{I7Exit, I7World};
use gvm::memory::Memory;
use gvm::objects::ParseNames;

fn glulx_image(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.starts_with(b"Glul") {
        return Some(bytes);
    }
    if !(bytes.starts_with(b"FORM") && bytes.get(8..12) == Some(b"IFRS")) {
        return None;
    }
    let be32 = |a: usize| -> usize {
        u32::from_be_bytes([bytes[a], bytes[a + 1], bytes[a + 2], bytes[a + 3]]) as usize
    };
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let len = be32(i + 4);
        if &bytes[i..i + 4] == b"GLUL" {
            return bytes.get(i + 8..i + 8 + len).map(<[u8]>::to_vec);
        }
        i += 8 + len + (len & 1);
    }
    None
}

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else {
            println!("{path}: unreadable");
            continue;
        };
        let Some(image) = glulx_image(bytes) else {
            println!("{path}: not Glulx");
            continue;
        };
        let Ok(mem) = Memory::new(image) else {
            println!("{path}: bad image");
            continue;
        };
        let Ok(pn) = ParseNames::detect(&mem) else {
            println!("{path}: no object tree");
            continue;
        };
        let t = std::time::Instant::now();
        let Some(w) = I7World::detect(&mem, &pn) else {
            println!("{path}: no I7 map ({:?})", t.elapsed());
            continue;
        };
        let named = w
            .rooms()
            .iter()
            .filter(|&&r| w.printed_name(&mem, &pn, r).is_some())
            .count();
        let mut exits = 0;
        let mut resolved = 0;
        let mut compassed = 0;
        for &r in w.rooms() {
            for (c, _, e) in w.exits(&mem, &pn, r) {
                exits += 1;
                if c.is_some() {
                    compassed += 1;
                }
                if e.destination().is_some() {
                    resolved += 1;
                }
            }
        }
        println!(
            "{path}: {} rooms, {} named, {} directions, {exits} exits ({resolved} to a room, {compassed} on a compass point), props {:?}, Map_Storage {:#x}, {:?}",
            w.rooms().len(),
            named,
            w.directions().len(),
            w.properties(),
            w.map_storage(),
            t.elapsed()
        );
        // An INDEPENDENT room set, for cross-checking the one the map gives:
        // Inform's property 2 is the class-inheritance list (`Inform6/objects.c`
        // seeds only properties 1, 2 and 3), and every I7 room carries the
        // `K1_room` class in it. Whichever class the map's rooms agree on should
        // be carried by exactly those objects and no others.
        let classes = |o: u32| -> Vec<u32> {
            let Some((data, len)) = pn.property(&mem, o, 2) else { return vec![] };
            (0..len).filter_map(|i| mem.read32(data + i * 4)).collect()
        };
        let mut tally: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for &r in w.rooms() {
            for c in classes(r) {
                *tally.entry(c).or_default() += 1;
            }
        }
        // Every object also inherits the generic `Object` class, so the room
        // kind is the TIGHTEST class every room carries, not the commonest.
        let universal: Vec<u32> =
            tally.iter().filter(|(_, &n)| n == w.rooms().len()).map(|(&c, _)| c).collect();
        if let Some((class, carriers)) = universal
            .iter()
            .map(|&c| (c, pn.objects().filter(|&o| classes(o).contains(&c)).count()))
            .min_by_key(|&(_, n)| n)
        {
            println!(
                "  class cross-check: class {class:#x} is on all {} rooms and on {carriers} objects in all",
                w.rooms().len()
            );
        }

        if std::env::var("I7_DUMP").is_ok() {
            for (i, &d) in w.directions().iter().enumerate() {
                println!("  dir {i}: {:?}", w.printed_name(&mem, &pn, d));
            }
            for &r in w.rooms() {
                println!("ROOM {r:#x} {:?}", w.printed_name(&mem, &pn, r));
                for (c, d, e) in w.exits(&mem, &pn, r) {
                    let dn = c
                        .map(|c| format!("{c:?}"))
                        .unwrap_or_else(|| format!("{:?}", w.printed_name(&mem, &pn, d)));
                    let to = match e {
                        I7Exit::Room(x) => format!("{:?}", w.printed_name(&mem, &pn, x)),
                        I7Exit::ThroughDoor { to, .. } => {
                            format!("door -> {:?}", w.printed_name(&mem, &pn, to))
                        }
                        I7Exit::Door(x) => format!("door {x:#x} (unresolved)"),
                    };
                    println!("   {dn} -> {to}");
                }
            }
        }
    }
}
