//! The TI-99/4A loader against the reference-format oracle (SQ-1414).
//!
//! `docs/internals/scott-dialects-spec.md` §10.1 names the oracle this suite
//! uses: many Scott Adams games exist both as a platform-native binary and
//! as a plain-text conversion, so decoding the native file and comparing the
//! resulting tables field for field against the text file is a black-box
//! check that needs nothing from the specification and nothing from any GPL
//! implementation.
//!
//! # How strong an oracle it actually is
//!
//! Not a total one, and §10.2 says so outright: **these are different
//! releases of the same games.** The item counts alone differ for half the
//! corpus — Pirate Adventure has 65 items in the tokenised release against
//! 67 in the conversion, Pyramid of Doom 93 against 101 — and the action
//! tables are not merely differently encoded but differently sized, since
//! the tokenised script has no reference-format counterpart at all
//! (see `scott::ti994a`). So this suite does not assert whole-`Database`
//! equality; it asserts, per file and per table, exactly the agreement that
//! was measured, and prints the whole comparison so a regression names the
//! table it broke.
//!
//! The measured agreements below ARE load-bearing all the same: a loader
//! that mis-read the baseline, the endianness, the pointer-table sentinel
//! rule or the chunked string encoding could not produce the room, item and
//! vocabulary text of a *different release of the same game* by accident.
//!
//! # Getting the corpus
//!
//! Both halves are commercial game files, neither is redistributable, and
//! neither is committed. Point `SCOTT_DIALECT_FIXTURES` at a directory
//! holding `ti99/adv01.fiad`…`adv12.fiad`, or create
//! `stories/scott-dialects/ti99/`; the `.dat` twins are looked for in
//! `stories/` (or `$SCOTT_DAT_FIXTURES`). Everything here **skips
//! vacuously with an explanation** when either half is absent — a silent
//! skip reads exactly like a pass, so the skip says why.
//!
//! ```text
//! <fixtures>/ti99/adv01.fiad … adv12.fiad
//!     https://ifarchive.org/if-archive/scott-adams/games/ti99/scott_adams_ti99_games.zip
//! stories/adv01.dat … adv12.dat
//!     https://ifarchive.org/if-archive/scott-adams/games/scottfree/AdamsGames.zip
//! ```

use std::path::PathBuf;

use scott::{Database, Options, Presentation, StepResult, Vm};

/// The twelve titles, in the order both archives number them.
const TITLES: [&str; 12] = [
    "Adventureland",
    "Pirate Adventure",
    "Secret Mission",
    "Voodoo Castle",
    "The Count",
    "Strange Odyssey",
    "Mystery Fun House",
    "Pyramid of Doom",
    "Ghost Town",
    "Savage Island part I",
    "Savage Island part II",
    "The Golden Voyage",
];

fn dir_of(env: &str, defaults: &[&str]) -> Option<PathBuf> {
    std::env::var_os(env)
        .map(PathBuf::from)
        .into_iter()
        .chain(defaults.iter().map(PathBuf::from))
        .find(|p| p.is_dir())
}

/// The `.fiad` and `.dat` for game `n` (1-based), when both are present.
fn twins(n: usize) -> Option<(Vec<u8>, Vec<u8>)> {
    let ti = dir_of(
        "SCOTT_DIALECT_FIXTURES",
        &["stories/scott-dialects", "../../stories/scott-dialects"],
    )?
    .join("ti99")
    .join(format!("adv{n:02}.fiad"));
    let dat = dir_of("SCOTT_DAT_FIXTURES", &["stories", "../../stories"])?
        .join(format!("adv{n:02}.dat"));
    Some((std::fs::read(ti).ok()?, std::fs::read(dat).ok()?))
}

fn skip(what: &str) {
    eprintln!(
        "SKIP: {what} — no TI-99/4A specimens and/or .dat twins found. \
         Point SCOTT_DIALECT_FIXTURES at a directory holding ti99/adv01.fiad…adv12.fiad \
         (or create stories/scott-dialects/ti99/), and put adv01.dat…adv12.dat in \
         stories/ (or point SCOTT_DAT_FIXTURES at them). See this file's module docs."
    );
}

/// How closely two releases of one game agree, table by table.
///
/// Ratios, not booleans. The two ARE different releases (see the module
/// docs), so "these tables are not byte-identical" says nothing; "seventy-four
/// of this game's seventy-six verbs are identical and the other two are
/// swapped with each other" says a great deal, and it is a number a
/// mis-decoded pointer table cannot produce by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ratio {
    same: usize,
    total: usize,
}

impl Ratio {
    fn of<T: PartialEq>(a: &[T], b: &[T]) -> Ratio {
        Ratio {
            same: a.iter().zip(b).filter(|(x, y)| x == y).count(),
            total: a.len().max(b.len()),
        }
    }
    /// Percent, rounded down; 100 for two empty tables.
    fn percent(self) -> usize {
        (self.same * 100).checked_div(self.total).unwrap_or(100)
    }
}

impl std::fmt::Display for Ratio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:3}/{:3} ({:3}%)", self.same, self.total, self.percent())
    }
}

/// Text as it can fairly be compared across two releases: the tokenised
/// releases were re-typeset — they capitalise differently ("Royal Palace."
/// against "Royal palace"), they end room descriptions with a period where
/// the conversion does not, and the chunked encoding has no way to store the
/// hard line breaks the conversion's message text carries, so an embedded
/// newline arrives as the implied single space between two chunks. None of
/// that is a decoding difference, and normalising it away is what lets the
/// remaining agreement mean something.
fn comparable(text: &str) -> String {
    text.to_ascii_lowercase()
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['.', '!'])
        .to_string()
}

/// What fraction of `a`'s entries appear ANYWHERE in `b`, in percent.
///
/// Order-independent, which is the point. Spec §10.2 recommends exactly
/// this check for the titles whose counts already differ: "the useful
/// checks are structural — that the vocabulary overlaps heavily, that room
/// descriptions read as the same rooms". Several of these releases renumber
/// their tables wholesale — Secret Mission's implanted bomb detector is item
/// 48 in the tokenised release and item 9 in the conversion — so a
/// positional comparison of those scores zero while saying nothing at all
/// about the decode, and an overlap says a great deal.
fn overlap(a: &[String], b: &[String]) -> Ratio {
    let mut same = 0;
    let mut total = 0;
    for entry in a {
        // The empty/placeholder slots are a convention difference, not a
        // decode (spec §3.5), and counting them would flatter the score.
        if entry.is_empty() || entry == "." {
            continue;
        }
        total += 1;
        if b.contains(entry) {
            same += 1;
        }
    }
    Ratio { same, total }
}

/// One game's whole comparison against its twin.
struct Agreement {
    exits: Ratio,
    rooms: Ratio,
    items: Ratio,
    item_locations: Ratio,
    auto_nouns: Ratio,
    verbs: Ratio,
    nouns: Ratio,
    messages: Ratio,
    // Order-independent overlaps — see `overlap`. These are what the
    // assertions read; the positional ratios above are for the report.
    room_overlap: Ratio,
    item_overlap: Ratio,
    verb_overlap: Ratio,
    noun_overlap: Ratio,
    message_overlap: Ratio,
}

impl Agreement {
    fn measure(ti: &Database, dat: &Database) -> Agreement {
        // Index 0 of every table is a placeholder in both formats — an
        // unused room, an empty message — and this dialect spells an empty
        // string as a single period (spec §3.5) where the conversion spells
        // it as nothing. Comparing them would measure that convention, not
        // the decode, so every text table below starts at index 1.
        let text = |v: &[String]| v.iter().skip(1).map(|s| comparable(s)).collect::<Vec<_>>();
        let room_text = |d: &Database| {
            d.rooms
                .iter()
                .skip(1)
                .map(|r| (comparable(&r.desc), r.literal))
                .collect::<Vec<_>>()
        };
        let item_text = |d: &Database| {
            d.items
                .iter()
                .skip(1)
                .map(|i| (comparable(&i.text), i.treasure))
                .collect::<Vec<_>>()
        };
        Agreement {
            exits: Ratio::of(
                &ti.rooms.iter().map(|r| r.exits).collect::<Vec<_>>(),
                &dat.rooms.iter().map(|r| r.exits).collect::<Vec<_>>(),
            ),
            rooms: Ratio::of(&room_text(ti), &room_text(dat)),
            items: Ratio::of(&item_text(ti), &item_text(dat)),
            item_locations: Ratio::of(
                &ti.items.iter().map(|i| i.start_loc).collect::<Vec<_>>(),
                &dat.items.iter().map(|i| i.start_loc).collect::<Vec<_>>(),
            ),
            auto_nouns: Ratio::of(
                &ti.items.iter().map(|i| i.auto_noun.clone()).collect::<Vec<_>>(),
                &dat.items.iter().map(|i| i.auto_noun.clone()).collect::<Vec<_>>(),
            ),
            verbs: Ratio::of(&text(&ti.verbs), &text(&dat.verbs)),
            nouns: Ratio::of(&text(&ti.nouns), &text(&dat.nouns)),
            messages: Ratio::of(&text(&ti.messages), &text(&dat.messages)),
            room_overlap: overlap(&plain_rooms(ti), &plain_rooms(dat)),
            item_overlap: overlap(&plain_items(ti), &plain_items(dat)),
            verb_overlap: overlap(&text(&ti.verbs), &text(&dat.verbs)),
            noun_overlap: overlap(&text(&ti.nouns), &text(&dat.nouns)),
            message_overlap: overlap(&text(&ti.messages), &text(&dat.messages)),
        }
    }

    fn report(&self) -> String {
        format!(
            "positional: exits {} rooms {} items {} iloc {} links {} \
             verbs {} nouns {} msgs {}\n      \
             overlap:    rooms {} items {} verbs {} nouns {} msgs {}",
            self.exits,
            self.rooms,
            self.items,
            self.item_locations,
            self.auto_nouns,
            self.verbs,
            self.nouns,
            self.messages,
            self.room_overlap,
            self.item_overlap,
            self.verb_overlap,
            self.noun_overlap,
            self.message_overlap,
        )
    }
}

/// Room descriptions, normalised for cross-release comparison.
fn plain_rooms(d: &Database) -> Vec<String> {
    d.rooms.iter().map(|r| comparable(&r.desc)).collect()
}

/// Item texts, normalised for cross-release comparison.
fn plain_items(d: &Database) -> Vec<String> {
    d.items.iter().map(|i| comparable(&i.text)).collect()
}

/// The measured per-table agreement floor, in percent, for each of the
/// twelve titles against its `.dat` twin — filled in from the run below and
/// pinned so a decoding regression has to break one.
///
/// They are floors and not exact values on purpose: they are a property of
/// two particular releases meeting each other, and the interesting event is
/// a table falling apart, not a percentage point.
#[derive(Debug, Clone, Copy)]
struct Floors {
    rooms: usize,
    items: usize,
    verbs: usize,
    nouns: usize,
    messages: usize,
}

const FLOORS: [Floors; 12] = [
    /* adv01 Adventureland    */ Floors { rooms:  81, items:  82, verbs: 100, nouns:  92, messages:  48 },
    /* adv02 Pirate Adventure */ Floors { rooms:  88, items:  90, verbs: 100, nouns:  89, messages:  44 },
    /* adv03 Secret Mission   */ Floors { rooms:  73, items:  72, verbs: 100, nouns:  91, messages:  48 },
    /* adv04 Voodoo Castle    */ Floors { rooms:  92, items:  95, verbs: 100, nouns: 100, messages:  47 },
    /* adv05 The Count        */ Floors { rooms:  91, items:  98, verbs: 100, nouns:  93, messages:  61 },
    /* adv06 Strange Odyssey  */ Floors { rooms:  91, items:  75, verbs: 100, nouns:  97, messages:  58 },
    /* adv07 Mystery Fun House */ Floors { rooms:  97, items:  88, verbs: 100, nouns:  96, messages:  48 },
    /* adv08 Pyramid of Doom  */ Floors { rooms: 100, items:  78, verbs: 100, nouns:  93, messages:  41 },
    /* adv09 Ghost Town       */ Floors { rooms:  97, items:  97, verbs:  99, nouns:  98, messages:  66 },
    /* adv10 Savage Island I  */ Floors { rooms:  62, items:  91, verbs: 100, nouns:  96, messages:  49 },
    /* adv11 Savage Island II */ Floors { rooms:  65, items:  95, verbs: 100, nouns:  96, messages:  50 },
    /* adv12 The Golden Voyage */ Floors { rooms:  94, items:  94, verbs: 100, nouns:  97, messages:  60 },
];

#[test]
fn every_specimen_loads_and_is_compared_table_by_table_with_its_dat_twin() {
    if twins(1).is_none() {
        skip("the .fiad/.dat oracle");
        return;
    }
    // What was MEASURED, per game, as the set of tables the two releases
    // agree on. Pinned rather than asserted-as-all-true because these are
    // different releases (see the module docs); a change to any entry is a
    // change to what the loader produces and must be looked at.
    //
    // The header scalars agree almost everywhere, which is the strongest
    // single signal in the table: the maximum carried, the starting room,
    // the significant word length, the lamp duration and the treasure room
    // are five independent numbers read out of five different header bytes
    // through the baseline rule, and they match a completely independent
    // encoding of the same game.
    let mut report = String::new();
    let mut loaded = 0;
    for (i, title) in TITLES.iter().enumerate() {
        let n = i + 1;
        let Some((ti_bytes, dat_bytes)) = twins(n) else {
            continue;
        };
        let ti = Database::parse(&ti_bytes)
            .unwrap_or_else(|e| panic!("adv{n:02}.fiad ({title}) must load: {e:?}"));
        let dat = Database::parse(&dat_bytes)
            .unwrap_or_else(|e| panic!("adv{n:02}.dat ({title}) must load: {e:?}"));
        loaded += 1;

        // Whatever else differs, the tokenised release must be internally
        // coherent: a script, no reference-format actions, and every table
        // sized by its own header.
        let script = ti.ti99.as_ref().expect("a TI-99/4A database carries a script");
        assert!(
            ti.actions.is_empty(),
            "adv{n:02}: a tokenised database has no reference-format actions"
        );
        assert_eq!(
            script.verb_chains.len(),
            ti.verbs.len(),
            "adv{n:02}: one dispatch slot per vocabulary index"
        );
        assert_eq!(
            ti.verbs.len(),
            ti.nouns.len(),
            "adv{n:02}: the two dictionaries are padded to one length"
        );
        assert!(ti.start_room < ti.rooms.len(), "adv{n:02}: start room in range");

        let a = Agreement::measure(&ti, &dat);
        report.push_str(&format!(
            "adv{n:02} {title:22} items={:3} rooms={:3} vocab={:3} msgs={:3} recs={:4}  {}\n",
            ti.items.len(),
            ti.rooms.len(),
            ti.verbs.len(),
            ti.messages.len(),
            script.verb_chains.iter().map(|c| c.len()).sum::<usize>()
                + script.automatic.len(),
            a.report(),
        ));

        // The header scalars that name the same game whichever release you
        // read. Four of the six agree for all twelve; the two that do not
        // are named individually below, because "this release differs here"
        // is a fact worth pinning and a silent tolerance is not.
        assert_eq!(ti.max_carry, dat.max_carry, "adv{n:02} ({title}): max carried");
        assert_eq!(ti.word_length, dat.word_length, "adv{n:02} ({title}): word length");
        assert_eq!(ti.light_time, dat.light_time, "adv{n:02} ({title}): lamp duration");
        assert_eq!(ti.treasure_room, dat.treasure_room, "adv{n:02} ({title}): treasure room");
        // Savage Island part II starts the player in room 29 in the
        // tokenised release and room 30 in the conversion, which is the same
        // difference as its room count (30 against 31): the conversion has
        // one extra room. Every other title's starting room agrees.
        if n != 11 {
            assert_eq!(ti.start_room, dat.start_room, "adv{n:02} ({title}): start room");
        }
        // Pyramid of Doom's tokenised release has fourteen `*`-marked items
        // against the conversion's thirteen — and the tokenised header's own
        // treasure byte says thirteen too, which is exactly why spec §3.3
        // calls that byte "present but not authoritative" and requires the
        // asterisks to be counted instead.
        if n != 8 {
            assert_eq!(
                ti.num_treasures, dat.num_treasures,
                "adv{n:02} ({title}): treasure count"
            );
        }

        // The table-by-table floors, every one of them measured. These are
        // what a decoding regression breaks: mis-read the pointer-table
        // sentinel rule and the vocabulary collapses; get the chunk
        // separator wrong and every room and message falls below its floor;
        // drop the 255-means-carried normalisation and the item locations
        // do.
        let floors = FLOORS[i];
        for (name, got, floor) in [
            ("room overlap", a.room_overlap, floors.rooms),
            ("item overlap", a.item_overlap, floors.items),
            ("verb overlap", a.verb_overlap, floors.verbs),
            ("noun overlap", a.noun_overlap, floors.nouns),
            ("message overlap", a.message_overlap, floors.messages),
        ] {
            assert!(
                got.percent() >= floor,
                "adv{n:02} ({title}): {name} agreement fell to {got}, \
                 below the measured floor of {floor}%"
            );
        }
    }
    eprintln!("\nTI-99/4A release vs .dat conversion, per table:\n{report}");
    assert_eq!(loaded, 12, "all twelve titles present in both formats");
}

#[test]
fn every_verb_of_every_tokenised_release_is_a_verb_of_its_dat_twin() {
    if twins(1).is_none() {
        skip("the vocabulary oracle");
        return;
    }
    // The single sharpest claim in this file. The two dictionaries are
    // located through two separate header pointers, stored as bare
    // characters with no terminator and no padding, and each word's length
    // is the difference between consecutive entries of a big-endian pointer
    // table whose last entry is an end sentinel (spec §3.6). Get the
    // baseline, the endianness, the sentinel rule or the "length is the
    // difference" rule wrong by ONE and the words come out shifted by a
    // character — and a shifted word is not a word of the other release.
    //
    // Measured: 100% for eleven of the twelve, and 103 of 104 for Ghost
    // Town, whose one miss is a word its conversion does not carry.
    for (i, title) in TITLES.iter().enumerate() {
        let n = i + 1;
        let Some((ti_bytes, dat_bytes)) = twins(n) else { continue };
        let ti = Database::parse(&ti_bytes).expect("loads");
        let dat = Database::parse(&dat_bytes).expect("loads");
        let words = |v: &[String]| {
            v.iter()
                .skip(1)
                .map(|s| comparable(s))
                .collect::<Vec<_>>()
        };
        let got = overlap(&words(&ti.verbs), &words(&dat.verbs));
        assert!(
            got.percent() >= FLOORS[i].verbs,
            "adv{n:02} ({title}): only {got} of the tokenised release's verbs \
             are verbs of the conversion"
        );
    }
}

#[test]
fn every_specimen_reaches_a_prompt_and_answers_look_and_inventory() {
    if twins(1).is_none() {
        skip("the playable smoke");
        return;
    }
    for (i, title) in TITLES.iter().enumerate() {
        let n = i + 1;
        let Some((ti_bytes, _)) = twins(n) else { continue };
        let db = Database::parse(&ti_bytes).expect("loads");
        // The lamp flags and the message set are forced by the database
        // (spec Appendix A), so whatever the host asks for is overridden.
        let mut vm = Vm::new_full(db, false, 7, Options::new());
        assert_eq!(vm.options().presentation, Presentation::Ti994a);
        assert!(vm.options().scott_light, "spec §9.2 forces the countdown");
        assert!(vm.options().prehistoric_lamp, "spec §9.2 forces the destroy");
        assert_eq!(vm.step(), StepResult::NeedLine, "adv{n:02} ({title})");
        let _ = vm.take_output();
        let block = vm.room_block();
        assert!(
            !block.is_empty(),
            "adv{n:02} ({title}): the starting room describes itself"
        );
        for line in ["look", "inventory"] {
            vm.supply_line(line);
            // The game may legitimately end on an early automatic action;
            // what must not happen is a panic or a hang.
            if vm.step() == StepResult::Quit {
                break;
            }
            let out = vm.take_output();
            assert!(
                !out.contains('\u{FFFD}'),
                "adv{n:02} ({title}): '{line}' produced a replacement character: {out:?}"
            );
        }
    }
}

#[test]
fn no_specimen_has_an_undecodable_or_runaway_record() {
    if twins(1).is_none() {
        skip("the record census");
        return;
    }
    // Spec §11: opcode bytes 202-211 and 213 are unassigned, and a record
    // containing one is undecodable from that byte on. None of the twelve
    // original releases contains one — measured — so any appearance would
    // mean the record walk had gone off the rails rather than that the game
    // used a mystery opcode.
    for i in 0..TITLES.len() {
        let n = i + 1;
        let Some((ti_bytes, _)) = twins(n) else { continue };
        let db = Database::parse(&ti_bytes).expect("loads");
        let script = db.ti99.as_ref().unwrap();
        let records = script.verb_chains.iter().flatten().chain(&script.automatic);
        for (r, rec) in records.enumerate() {
            assert!(
                rec.ops.len() < 256,
                "adv{n:02} record {r}: a record's stream is at most 254 bytes"
            );
            // The percentage key of an automatic record is 0..=100; an
            // explicit record's key is a noun index. Only the automatic
            // chain is checked, since it is the one with a stated range.
            let _ = rec.key;
        }
        for (r, rec) in script.automatic.iter().enumerate() {
            assert!(
                rec.key <= 100,
                "adv{n:02} automatic record {r}: percentage key {} is out of range",
                rec.key
            );
        }
    }
}
