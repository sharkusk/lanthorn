//! Parses the ScottFree `.dat` text format into a [`Database`]: a lexer over
//! whitespace-separated integers and `"`-quoted strings, a header of table
//! sizes, then rooms, messages, actions, verbs/nouns, and items in that
//! fixed order. [`looks_like_scott`] does the cheap header-shape sniff a
//! multi-engine host uses to decide whether a file is worth handing to
//! [`Database::parse`] at all.
//!
//! Every count and index in the header is bounds-checked before it sizes an
//! allocation or indexes a table (SQ-0629): a hostile or truncated file
//! returns a [`LoadError`] rather than exhausting memory or panicking.
//!
//! [`Database::parse`] takes any `AsRef<[u8]>` (a `&str` still works
//! unchanged) and lexes over raw bytes rather than `char`s, so a Latin-1 or
//! otherwise non-UTF-8 `.dat` loads instead of failing a UTF-8 check before
//! it ever reaches the parser (SQ-1412). Bytes outside ASCII inside a quoted
//! string become `?`.

use crate::database::CARRIED;
use crate::*;

/// Everything [`Database::parse`] can fail with: a malformed or truncated
/// `.dat` file, or one that fails to parse but whose raw bytes match a known
/// OTHER Scott Adams dialect's signature. `#[non_exhaustive]` so a future
/// parse-failure mode can add a variant without breaking a caller's `match`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LoadError {
    /// The file ran out of bytes before a value the parser still expected
    /// (mid-header, mid-table, or a missing trailing token), or the parsed
    /// start-room field doesn't index a real room.
    Truncated,
    /// A whitespace-delimited token that should have been a decimal integer
    /// wasn't one; the string is that token's own literal text, kept for the
    /// error message.
    BadInt(String),
    /// A quoted string is malformed: the parser expected an opening `"`
    /// where a string token began, or reached end of file before finding
    /// the closing one.
    Unterminated,
    /// A header count is negative or implausibly large — a hostile file can
    /// otherwise request gigabytes of `Vec` capacity before a single body
    /// token is read (SQ-0629).
    BadCount(&'static str, i32),
    /// A room exit points outside the room table (negative or > NumRooms).
    BadExit(i32),
    /// The file is not this crate's ScottFree text format, but its bytes
    /// match a KNOWN other Scott Adams dialect's signature — see [`Dialect`]
    /// — and that dialect is one this crate does not yet read.
    ///
    /// A refusal that can name what it refused, so a host can tell the
    /// player "this is a Commodore 64 memory snapshot" instead of showing a
    /// generic parse failure that gives no hint what went wrong.
    /// [`Dialect::Ti994aBytecode`] no longer reaches here — it is LOADED
    /// (SQ-1414, [`crate::parse_ti994a`]); the remaining memory-image
    /// dialects are still gated on the clean-room specification work that
    /// covers them.
    UnsupportedDialect(Dialect),
    /// The file matched a [`Dialect`] this crate DOES read, but that
    /// dialect's own structures did not check out: the string names what —
    /// a table pointer resolving outside the file, a record running past the
    /// end, a starting room that indexes nothing.
    ///
    /// Distinct from [`LoadError::UnsupportedDialect`] because the two mean
    /// opposite things to a host: that one says "a format I cannot read",
    /// this one says "a format I can read, in a file that is damaged".
    BadDialectData(Dialect, &'static str),
}

/// A Scott Adams / Adventure International game-data format this crate's
/// [`Database::parse`] does not read, recognised by a fixed byte signature
/// rather than by attempting (and failing) the normal text parse — see
/// [`detect_dialect`].
///
/// # Where these signatures come from
///
/// Each one is the opening of the game's own **verb dictionary**, and each
/// is stated below as a fact about the on-disk layout, re-derived by
/// measurement over the specimen corpus catalogued on [`detect_dialect`]
/// rather than taken from any interpreter's source. lanthorn is
/// BSD-3-Clause and the established Scott Adams interpreters are GPL, so a
/// signature is carried into this crate only once it has been measured off
/// real game files (SQ-1414; the pre-existing citations of one such
/// interpreter's source were replaced by the measurements below).
///
/// Every Adventure International dictionary begins with the same two verbs
/// in the same two slots — `AUTO` (verb 0, the occurrence pseudo-verb) then
/// `GO` (verb 1) — stored in **fixed-width fields**, so the first two
/// entries are a stable byte pattern that varies only with the field width
/// and with how the format marks a synonym. That is the whole of what these
/// signatures are, and it is why each is some spelling of "AUTO, GO".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Dialect {
    /// A TI-99/4A game image (a cartridge, cassette or `.fiad` disk-file
    /// dump) — a self-contained binary with its own header and its own
    /// action bytecode, not text at all, so nothing in it resembles the
    /// `.dat` this crate reads.
    ///
    /// Recognised by the fixed 10-byte sequence
    /// `30 30 30 30 00 30 30 00 28 28`. Measured: that sequence occurs
    /// exactly once, at file offset **1417** (`0x589`), in all twelve
    /// TI-99/4A releases of Adventures 1-12 (`adv01.fiad`-`adv12.fiad`),
    /// whose sizes range from 10,170 to 12,616 bytes. A fixed offset in
    /// those twelve is not evidence of a fixed offset in general, so the
    /// scan below stays unanchored.
    Ti994aBytecode,
    /// A C64 / ZX Spectrum / Atari 8-bit / Apple II memory snapshot
    /// carrying a binary Adventure International dictionary whose entries
    /// are NUL-padded to a fixed width — the plain, unpacked layout.
    ///
    /// Three field widths are known, and each gives a different opening
    /// pattern. Measured on `m1goldba.z80` and `supergra.z80` (see
    /// [`detect_dialect`]), whose companion `.dat` conversions declare word
    /// length 4: entries are **5 bytes** wide — a word of up to 5
    /// characters, NUL-padded — and a leading `*` marks a synonym of the
    /// preceding canonical word. The table there opens
    /// `AUTO\0` `GO\0\0\0` `SWIM\0` `SQUE\0` …, of which the first 8 bytes,
    /// `b"AUTO\0GO\0"`, are the signature.
    ///
    /// The same construction at the other two widths gives the other two
    /// signatures: a 4-byte field (word length 3) opens `AUT\0` `GO\0`,
    /// i.e. `b"AUT\0GO\0"`; a 6-byte field (word length 5) opens
    /// `GO\0\0\0\0` `*CROSS` `*RUN\0`, i.e. `b"GO\0\0\0\0*CROSS*RUN\0"` —
    /// the wide-dictionary layout The Sorcerer of Claymorgue Castle uses,
    /// which begins at `GO` rather than `AUTO`. Neither of those two
    /// appears in the corpus measured here; both are carried forward from
    /// the pre-existing detection (SQ-1413) **unverified**, and
    /// [`detect_dialect`]'s own doc says so.
    C64OrZxSnapshot,
    /// The same style of memory-image dictionary, **packed**: the trailing
    /// NUL pad is dropped and the synonym marker moves into the letter
    /// casing, so each entry is one byte narrower than the equivalent
    /// unpacked table.
    ///
    /// Measured on `seablood.z80` and `sherwood.z80`, the only two of the
    /// twenty ZX Spectrum snapshots in the corpus that carry it. Their
    /// dictionaries open
    /// `aUTO` `gO\0\0` `CLIM` `ENTE` `RUN\0` `BOAR` `MOVE` `sAIL` … —
    /// **4-byte** fields with no separator, in which a **lower-case first
    /// letter marks a canonical word and an upper-case one marks a synonym**
    /// of the entry before it. That is exactly the role the leading `*`
    /// plays in the unpacked tables: compare `supergra.z80`'s unpacked
    /// `GO\0\0\0` `*ENTE` `*WALK` `*CATC` `*CLIM` with the packed
    /// `gO\0\0` `CLIM` `ENTE` `RUN\0` above. The first 7 bytes,
    /// `b"aUTOgO\0"`, are the signature — mixed-case for that reason, and a
    /// byte shorter than the unpacked four-letter signature for the other.
    ///
    /// A game that packs its dictionary this way packs its action table
    /// too, which is why this is a [`Dialect`] of its own rather than a
    /// spelling variant of [`Dialect::C64OrZxSnapshot`]: reading it needs a
    /// different table decoder, not merely a different dictionary reader.
    CompressedActionTable,
    /// The **US S.A.G.A. binary database** — the American "Scott Adams
    /// Graphic Adventure" disk releases for the Atari 8-bit, the Apple II and
    /// the Commodore 64, plus the Questprobe *Hulk*
    /// ([`crate::saga_us`], SQ-1414/SQ-1464). This crate READS it.
    ///
    /// **The odd one out: this variant has no byte signature.** Every other
    /// [`Dialect`] above is the opening of a fixed-width verb dictionary, and
    /// this format's verb block spells one — the plain `AUTO\0GO\0` is 645
    /// bytes into the Commodore 64 *Hulk*'s array — but its dictionary proper
    /// begins at the **noun** block, so matching that signature and reading
    /// from there "produces a self-consistent parse of nothing"
    /// (`docs/internals/scott-dialects-spec.md` §12.14). It is therefore
    /// recognised **structurally** instead, by §12.2's version/adventure scan
    /// over the first 0x38 bytes plus §12.4's header limits plus the presence
    /// of §12.5's `ANY` — [`crate::saga_us::detect_saga_us`] — and
    /// [`detect_dialect`] checks for it BEFORE the signature scan for exactly
    /// that reason.
    SagaUsDatabase,
}

/// Scans `bytes` for one of [`Dialect`]'s fixed signatures, anywhere in the
/// file. `None` means no known signature was found; the file may still be
/// unreadable for some other reason, just not one this crate can name.
///
/// The scan is deliberately **unanchored**. Within one dialect the same
/// signature lands at very different offsets across releases — measured
/// across the corpus below, the four-letter dictionary signature appears at
/// offsets from 796 to 14,761 — so a fixed-offset check would recognise
/// some releases of a dialect and not others.
///
/// # Specimen corpus
///
/// Every claim in [`Dialect`]'s per-variant docs, and the two ordering
/// decisions below, were measured over these files. None of them is
/// redistributable, so none is committed; the hand-built fixtures in this
/// module's tests stand in for them on CI, and
/// `crates/scott/tests/dialect_specimens.rs` re-runs the measurement over
/// the real files when they are present, and skips vacuously when they are
/// not.
///
/// | archive | files | measured |
/// |---|---|---|
/// | IF Archive `if-archive/scott-adams/games/ti99/scott_adams_ti99_games.zip` (82,552 bytes) | `adv01.fiad`-`adv12.fiad`, Adventures 1-12 | [`Dialect::Ti994aBytecode`] in all 12, at offset 1417 in each |
/// | IF Archive `if-archive/games/spectrum/mystsoft.zip` (546,758 bytes) | 20 ZX Spectrum `.z80` snapshots | 13 [`Dialect::C64OrZxSnapshot`], 2 [`Dialect::CompressedActionTable`], 5 undetected |
///
/// The ZX Spectrum breakdown is the useful part, because it shows the
/// detector separating Scott Adams games from their shelf-mates rather than
/// merely firing:
///
/// * `b"AUTO\0GO\0"` in the eleven Mysterious Adventures
///   (`m1goldba`-`m11waxwo`), `gremlins` and `supergra`.
/// * `b"aUTOgO\0"` in `seablood` and `sherwood`, and in nothing else.
/// * Nothing in `blizzard`, `heman`, `kayleth`, `rbplanet` and `temple` —
///   the five titles in that archive that are not Adventure International
///   games. A detector that fired on those would be worse than useless,
///   since [`Database::parse`] consults this only after a parse has already
///   failed and would then blame the wrong format.
///
/// **Not covered by any specimen measured here:** `b"AUT\0GO\0"` (the
/// three-letter dictionary) and `b"GO\0\0\0\0*CROSS*RUN\0"` (Claymorgue's
/// five-letter one). Both are carried forward from SQ-1413 unverified. They
/// are consistent with the fixed-width construction [`Dialect`] describes,
/// which is why they are kept, but no file in the corpus above demonstrates
/// either.
pub fn detect_dialect(bytes: &[u8]) -> Option<Dialect> {
    const TI99: &[u8] = b"\x30\x30\x30\x30\x00\x30\x30\x00\x28\x28";
    const C64_4: &[u8] = b"AUTO\0GO\0";
    const C64_3: &[u8] = b"AUT\0GO\0";
    const C64_5: &[u8] = b"GO\0\0\0\0*CROSS*RUN\0";
    const COMPRESSED: &[u8] = b"aUTOgO\0";
    let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if crate::saga_us::detect_saga_us(bytes).is_some() {
        // Checked FIRST, and structurally rather than by signature — see
        // [`Dialect::SagaUsDatabase`]. These databases spell the plain
        // four-letter (or three-letter) dictionary signature in their VERB
        // block, 645 bytes past where their dictionary actually starts, so a
        // signature scan reaching them first would name the wrong dialect and
        // a loader following it would read nothing useful (§12.14).
        Some(Dialect::SagaUsDatabase)
    } else if contains(TI99) {
        Some(Dialect::Ti994aBytecode)
    } else if contains(COMPRESSED) {
        // Checked before the unpacked signatures. The packed and unpacked
        // dictionaries are different byte strings, so a file cannot match
        // both by construction — but a packed dictionary's later entries
        // are unpadded 4-byte fields, which is exactly the shape that could
        // spell an unpacked signature by accident, and only this ordering
        // makes that harmless.
        Some(Dialect::CompressedActionTable)
    } else if contains(C64_4) || contains(C64_3) || contains(C64_5) {
        // `AUT\0GO\0` is NOT a substring of `AUTO\0GO\0` (the four-letter
        // table pads `AUT`+`O`, the three-letter one pads `AUT`+NUL), so
        // these three are genuinely alternatives and their order among
        // themselves does not matter.
        Some(Dialect::C64OrZxSnapshot)
    } else if crate::z80::looks_like_z80(bytes) {
        // A ZX Spectrum `.z80` snapshot whose RLE happened to break a
        // signature up — an `ED ED` run landing inside `AUTO\0GO\0` — so the
        // scan above saw nothing (SQ-1478). Decompressing and scanning the
        // 48K image the tables actually live in is the reliable answer, and
        // §7.1's whole point; the raw scan comes first only because it is
        // free and because every one of the twenty §10.3 snapshots is caught
        // by it.
        //
        // Recursion is not a risk: the branch is taken only for a buffer
        // `looks_like_z80` accepts, and a decompressed image is 49,152 bytes
        // whose own offset-30 word is game data — a second decompression
        // attempt is one level deep at most and its result is discarded
        // unless it carries a signature.
        crate::z80::decompress_z80(bytes).ok().and_then(|image| {
            let contains = |needle: &[u8]| image.windows(needle.len()).any(|w| w == needle);
            if contains(COMPRESSED) {
                Some(Dialect::CompressedActionTable)
            } else if contains(C64_4) || contains(C64_3) || contains(C64_5) {
                Some(Dialect::C64OrZxSnapshot)
            } else {
                None
            }
        })
    } else {
        None
    }
}

/// Tokenizer over a `.dat` source: whitespace-separated ints and `"`-delimited
/// strings, over raw bytes (not `char`s) so non-UTF-8 input never panics or
/// fails up front — see the module doc.
struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a [u8]) -> Self {
        Lexer { bytes: src, pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn next_int(&mut self) -> Result<i32, LoadError> {
        self.skip_ws();
        if self.pos >= self.bytes.len() {
            return Err(LoadError::Truncated);
        }
        let start = self.pos;
        while self.pos < self.bytes.len() && !self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        // Header/action/exit tokens are always plain ASCII digits (optionally
        // signed), so a lossy-to-ASCII read is exact here; a non-numeric token
        // (including one containing non-ASCII bytes) fails to parse below.
        let word = String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned();
        word.parse::<i32>().map_err(|_| LoadError::BadInt(word))
    }

    /// Reads one `"`-delimited string token, per
    /// `docs/internals/scott-dialects-spec.md` §2.6 ("Quoted string
    /// syntax"): the opening `"` (after skipping whitespace) starts the
    /// token; a doubled `""` inside it is an escape for one literal `"`
    /// (checked before backtick substitution, so a backtick can never pair
    /// with a following `"` to form one); any other backtick byte becomes a
    /// `"` character; a `\r` byte is dropped so CRLF- and LF-authored files
    /// read the same, while an embedded `\n` is preserved; a byte outside
    /// ASCII becomes `?`. The string ends at the first `"` not immediately
    /// followed by another; end of file before that, or a non-`"` byte where
    /// the token should start, is [`LoadError::Unterminated`].
    fn next_str(&mut self) -> Result<String, LoadError> {
        // §2.6 enumerates the pre-quote whitespace set explicitly (space,
        // tab, CR, LF, vertical tab, form feed) — a superset of
        // `u8::is_ascii_whitespace`, which omits vertical tab (0x0B) — so
        // this is spelled out here rather than shared with `skip_ws`.
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
        {
            self.pos += 1;
        }
        if self.pos >= self.bytes.len() || self.bytes[self.pos] != b'"' {
            return Err(LoadError::Unterminated);
        }
        self.pos += 1; // consume the opening quote
        let mut out = String::new();
        loop {
            if self.pos >= self.bytes.len() {
                return Err(LoadError::Unterminated);
            }
            let b = self.bytes[self.pos];
            if b == b'"' {
                if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'"' {
                    out.push('"');
                    self.pos += 2;
                    continue;
                }
                self.pos += 1; // consume the closing quote
                return Ok(out);
            }
            if b == b'`' {
                out.push('"');
            } else if b == b'\r' {
                // dropped entirely, per §2.6
            } else if b.is_ascii() {
                out.push(b as char);
            } else {
                out.push('?');
            }
            self.pos += 1;
        }
    }
}

/// Splits an item's trailing `/WORD/` auto-noun marker out of its display
/// text, per `docs/internals/scott-dialects-spec.md` §2.5 ("Item auto-noun:
/// the trailing `/WORD/` marker"). `text` is truncated in place to the
/// item's displayed text (marker removed) and the auto-noun is returned; if
/// there is no first `/`, or the remainder from the first `/` onward is
/// exactly `//` or exactly `/*`, `text` is left untouched and `None` is
/// returned. Otherwise the auto-noun runs from just after the first `/` to
/// the next `/`, or to the end of the string if there is no second `/`.
///
/// `pub(crate)` so [`crate::c64`] applies the identical rule: §6.3 says the
/// memory-image dialects spell `/WORD/` "identically", and two copies of this
/// would be two places for that to drift.
pub(crate) fn extract_auto_noun(text: &mut String) -> Option<String> {
    let first = text.find('/')?;
    let remainder = &text[first..];
    if remainder == "//" || remainder == "/*" {
        return None;
    }
    let after = &text[first + 1..];
    let noun = match after.find('/') {
        Some(rel) => after[..rel].to_string(),
        None => after.to_string(),
    };
    text.truncate(first);
    Some(noun)
}

impl Database {
    /// Parse a ScottFree `.dat` file. Accepts anything byte-like — `&str`,
    /// `&[u8]`, `&Vec<u8>` — so a caller holding raw file bytes (a Latin-1 or
    /// otherwise non-UTF-8 `.dat`, which a `&str` conversion would reject
    /// outright) can hand them over directly; see the module doc.
    ///
    /// On failure, checks the raw bytes against [`detect_dialect`]'s known
    /// signatures before giving up. A file that fails the text parse and
    /// matches [`Dialect::Ti994aBytecode`] is handed to
    /// [`crate::parse_ti994a`], which reads it (SQ-1414) — so this one entry
    /// point answers for both the text format and the TI-99/4A tokenised
    /// one, which is what the dialect specification asks of a loader: "a
    /// dialect loader belongs behind the same entry point as the text parser
    /// and must produce the same model, not a parallel one"
    /// (`docs/internals/scott-dialects-spec.md`, Appendix A).
    ///
    /// A file matching one of the OTHER signatures is still refused as
    /// [`LoadError::UnsupportedDialect`] rather than as whatever token-level
    /// error (`BadInt`, `Truncated`, …) the text lexer happened to hit first
    /// — a host can then say "this is a Commodore 64 snapshot" instead of
    /// "invalid data" (SQ-1413).
    pub fn parse<S: AsRef<[u8]> + ?Sized>(src: &S) -> Result<Database, LoadError> {
        let bytes = src.as_ref();
        let text_error = match Self::parse_scottfree(bytes) {
            Ok(db) => return Ok(db),
            Err(e) => e,
        };
        match detect_dialect(bytes) {
            Some(Dialect::Ti994aBytecode) => crate::ti994a::parse_ti994a(bytes),
            // A Commodore 64 program file carrying one of the eleven
            // *Mysterious Adventures* (SQ-1414): `crate::c64` identifies it by
            // the low sixteen bits of the file's own byte sum, so a snapshot
            // of any OTHER game answering the same dictionary signature falls
            // through to the refusal below exactly as it did before.
            Some(Dialect::C64OrZxSnapshot) if crate::c64::looks_like_c64_mysterious_prg(bytes) => {
                crate::c64::parse_c64_mysterious_prg(bytes)
            }
            // A ZX Spectrum *Mysterious Adventures* release (SQ-1478),
            // either as a `.z80` snapshot or as the 48K image a host that
            // decompressed for its own reasons already holds. The container
            // step comes first — §7.1's "decompress first" rule, and the
            // reason the raw bytes reached `detect_dialect` at all is that
            // the RLE passes literal text through — and then
            // `crate::zx_mysterious` locates the tables with no catalogue.
            //
            // A file that only LOOKS like a snapshot (the check is a header
            // shape, not a magic number) refuses as the dialect it is rather
            // than as a container failure, which is what a memory image of
            // some other Scott Adams game does too.
            Some(Dialect::C64OrZxSnapshot) => {
                let decompressed;
                let image: Option<&[u8]> = if crate::z80::looks_like_z80(bytes) {
                    decompressed = crate::z80::decompress_z80(bytes).ok();
                    decompressed.as_deref()
                } else if bytes.len() == crate::z80::IMAGE_LEN {
                    Some(bytes)
                } else {
                    None
                };
                match image {
                    Some(image) => crate::zx_mysterious::parse_zx_mysterious(image),
                    None => Err(LoadError::UnsupportedDialect(Dialect::C64OrZxSnapshot)),
                }
            }
            // A US S.A.G.A. disk release's database, as the container handed
            // it over (SQ-1414/SQ-1464): `crate::saga_us` recognises it
            // structurally and answers which platform's array offset fitted,
            // so one entry point still reads every dialect this crate loads.
            Some(Dialect::SagaUsDatabase) => match crate::saga_us::detect_saga_us(bytes) {
                Some(platform) => crate::saga_us::parse_saga_us(bytes, platform),
                None => Err(LoadError::UnsupportedDialect(Dialect::SagaUsDatabase)),
            },
            Some(d) => Err(LoadError::UnsupportedDialect(d)),
            None => Err(text_error),
        }
    }

    fn parse_scottfree(bytes: &[u8]) -> Result<Database, LoadError> {
        let mut lex = Lexer::new(bytes);

        let _unknown = lex.next_int()?;
        let num_items = lex.next_int()?;
        let num_actions = lex.next_int()?;
        let num_words = lex.next_int()?;
        let num_rooms = lex.next_int()?;
        let max_carry = lex.next_int()?;
        let player_room = lex.next_int()?;
        let num_treasures = lex.next_int()?;
        let word_length = lex.next_int()?;
        let light_time = lex.next_int()?;
        let num_messages = lex.next_int()?;
        let treasure_room = lex.next_int()?;

        // Bound every count that sizes a pre-reserved Vec. Real Scott
        // databases have counts in the low hundreds; `looks_like_scott` uses
        // the same <10000 ceiling for its sniff. Without this, an unvalidated
        // header int (num_actions = 2_000_000_000) asks `with_capacity` for
        // tens of gigabytes before any body token is read (SQ-0629).
        const MAX_COUNT: i32 = 10_000;
        for (name, v) in [
            ("NumItems", num_items),
            ("NumActions", num_actions),
            ("NumWords", num_words),
            ("NumRooms", num_rooms),
            ("NumMessages", num_messages),
        ] {
            if !(0..=MAX_COUNT).contains(&v) {
                return Err(LoadError::BadCount(name, v));
            }
        }

        let mut actions = Vec::with_capacity(num_actions as usize + 1);
        for _ in 0..=num_actions {
            let w0 = lex.next_int()?;
            let verb = (w0 / 150) as u16;
            let noun = (w0 % 150) as u16;
            let mut conditions = [Condition { code: 0, value: 0 }; 5];
            for c in conditions.iter_mut() {
                let w = lex.next_int()?;
                c.code = (w % 20) as u8;
                c.value = (w / 20) as u16;
            }
            let w6 = lex.next_int()?;
            let w7 = lex.next_int()?;
            let commands = [
                (w6 / 150) as u16,
                (w6 % 150) as u16,
                (w7 / 150) as u16,
                (w7 % 150) as u16,
            ];
            actions.push(Action {
                verb,
                noun,
                conditions,
                commands,
            });
        }

        let mut verbs: Vec<String> = Vec::with_capacity(num_words as usize + 1);
        let mut nouns: Vec<String> = Vec::with_capacity(num_words as usize + 1);
        for _ in 0..=num_words {
            let verb = lex.next_str()?;
            let noun = lex.next_str()?;
            verbs.push(verb);
            nouns.push(noun);
        }

        let mut rooms = Vec::with_capacity(num_rooms as usize + 1);
        for _ in 0..=num_rooms {
            let mut exits = [0usize; 6];
            for e in exits.iter_mut() {
                let v = lex.next_int()?;
                // Exits index the room table at move time; a negative value
                // would wrap to a huge usize and an over-large one soft-locks
                // the player in a nonexistent room (SQ-0629). 0 = no exit.
                if !(0..=num_rooms).contains(&v) {
                    return Err(LoadError::BadExit(v));
                }
                *e = v as usize;
            }
            let mut desc = lex.next_str()?;
            let literal = if let Some(stripped) = desc.strip_prefix('*') {
                desc = stripped.to_string();
                true
            } else {
                false
            };
            rooms.push(Room {
                exits,
                desc,
                literal,
            });
        }

        let mut messages = Vec::with_capacity(num_messages as usize + 1);
        for _ in 0..=num_messages {
            messages.push(lex.next_str()?);
        }

        let mut items = Vec::with_capacity(num_items as usize + 1);
        for _ in 0..=num_items {
            let mut text = lex.next_str()?;
            let mut start_loc = lex.next_int()?;
            // ScottFree stores an item's location (and start location) as an
            // `unsigned char` with `CARRIED` defined as 255 (`Scott.h`); this
            // crate represents "carried" as -1 (`database::CARRIED`)
            // throughout `Vm`. Normalise at load so conditions 17/18 ("item
            // still/not in its initial room") still hold after a
            // programmatic take moves an item to -1 — comparing a live -1
            // against a start_loc left at 255 would never match (SQ-1412).
            if start_loc == 255 {
                start_loc = CARRIED;
            }
            let treasure = text.starts_with('*');
            let auto_noun = extract_auto_noun(&mut text);
            items.push(Item {
                text,
                treasure,
                auto_noun,
                start_loc,
            });
        }

        // Trailer (optional metadata, best-effort): NumActions+1 comment strings,
        // then a version int, then the adventure number. Any failure here just
        // leaves adventure_number at 0 — the trailer is not required to parse.
        let mut adventure_number = 0;
        (|| -> Result<(), LoadError> {
            for _ in 0..=num_actions {
                lex.next_str()?;
            }
            let _version = lex.next_int()?;
            adventure_number = lex.next_int()?;
            Ok(())
        })()
        .ok();

        // Guard against a malformed start-room field that would index past the
        // rooms table at runtime.
        if player_room < 0 || player_room as usize >= rooms.len() {
            return Err(LoadError::Truncated);
        }

        Ok(Database {
            max_carry,
            start_room: player_room as usize,
            num_treasures,
            word_length: word_length as usize,
            light_time,
            treasure_room: treasure_room as usize,
            actions,
            verbs,
            nouns,
            rooms,
            messages,
            items,
            adventure_number,
            // The text format IS the reference shape; only a TI-99/4A
            // release carries a tokenised script instead (SQ-1414).
            ti99: None,
            // …and the text format carries no series marker either: §6.1 says
            // a reference-format Mysterious database is recognised "only by
            // its header counts", which is a catalogue this crate does not
            // keep. A `.dat` conversion of one of the eleven therefore loads
            // as an ordinary database, and a host wanting the series' lamp
            // behaviour for it sets the two options itself.
            mysterious: false,
            second_person: false,
            // …nor any S.A.G.A. release identity: §12.2's version/adventure
            // pair lives in the binary database's front matter, and a `.dat`
            // conversion of one of those titles is an ordinary text database
            // with an optional trailer and no platform (SQ-1414).
            saga_us: None,
        })
    }
}

/// Cheap content sniff for engine detection, over RAW BYTES: true for a
/// ScottFree `.dat` (the text format, via [`looks_like_scott`]), a TI-99/4A
/// tokenised release (via [`crate::ti994a::looks_like_ti994a`]), a
/// Commodore 64 *Mysterious Adventures* program file (via
/// [`crate::c64::looks_like_c64_mysterious_prg`]) **or** a US S.A.G.A.
/// binary database as its container handed it over (via
/// [`crate::saga_us::detect_saga_us`]).
///
/// This is the sniff a multi-engine host wants, because those are exactly
/// the things [`Database::parse`] reads. [`looks_like_scott`] takes a
/// `&str` and so can never answer for a binary dialect at all: a host that
/// spells its check `from_utf8(bytes).is_ok_and(looks_like_scott)` rejects
/// every TI-99/4A file before the loader ever sees it, which is what it did
/// before SQ-1414.
pub fn looks_like_scott_bytes(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(looks_like_scott)
        || crate::ti994a::looks_like_ti994a(bytes)
        || crate::c64::looks_like_c64_mysterious_prg(bytes)
        || crate::zx_mysterious::looks_like_zx_mysterious_z80(bytes)
        || crate::saga_us::detect_saga_us(bytes).is_some()
}

/// Cheap content sniff for engine detection: parse the 12 header ints and sanity-check.
///
/// TEXT format only. For a host deciding whether a file is a Scott Adams
/// game at all, [`looks_like_scott_bytes`] is the one to reach for — it also
/// answers for the TI-99/4A tokenised releases, which are binary and so can
/// never reach a `&str` in the first place.
pub fn looks_like_scott(src: &str) -> bool {
    let mut lex = Lexer::new(src.as_bytes());
    let mut ints = [0i32; 12];
    for slot in ints.iter_mut() {
        match lex.next_int() {
            Ok(v) => *slot = v,
            Err(_) => return false,
        }
    }
    let in_range = |v: i32, lo: i32, hi: i32| v >= lo && v < hi;
    in_range(ints[1], 0, 10000)
        && in_range(ints[2], 0, 10000)
        && in_range(ints[3], 0, 10000)
        && in_range(ints[4], 0, 10000)
        && in_range(ints[10], 0, 10000)
        && in_range(ints[5], 0, 1000)
        && in_range(ints[8], 1, 10)
        // Start room (field 6) must be a valid room index: 0..=num_rooms (field 4).
        && in_range(ints[6], 0, ints[4] + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    // NumItems=1, NumActions=0, NumWords=1, NumRooms=2, NumMessages=0
    const MINI: &str = r#"
32767 1 0 1 2 6 1 0 3 125 0 1
150 1 0 0 0 0 0 0
"" ""
"GO" "NORTH"
0 0 0 0 0 0 "*limbo"
2 0 0 0 0 0 "*forest clearing"
0 0 0 0 0 0 "*swamp"
""
"" 0
"*a brass lamp/LAMP/" 1
"action comment"
0
1
1
"#;
    #[test]
    fn parses_header_rooms_items() {
        let db = Database::parse(MINI).expect("parse");
        assert_eq!(db.start_room, 1);
        assert_eq!(db.light_time, 125);
        assert_eq!(db.rooms.len(), 3); // NumRooms=2 -> 3 slots
        assert_eq!(db.rooms[1].desc, "forest clearing");
        assert!(db.rooms[1].literal);
        assert_eq!(db.rooms[1].exits[0], 2); // north -> room 2
        assert_eq!(db.items.len(), 2); // NumItems=1 -> 2 slots
        assert_eq!(db.items[1].auto_noun.as_deref(), Some("LAMP"));
        assert!(!db.items[1].text.contains('/'));
        assert!(db.items[1].text.starts_with('*')); // treasure keeps its asterisk
    }
    #[test]
    fn action_decodes_verb_noun() {
        let db = Database::parse(MINI).unwrap();
        assert_eq!(db.actions.len(), 1); // NumActions=0 -> 1 slot
        assert_eq!(db.actions[0].verb, 1);
        assert_eq!(db.actions[0].noun, 0);
    }
    #[test]
    fn parses_trailer_adventure_number() {
        // MINI's trailer: 1 comment string ("action comment"), version=0,
        // adventure_number=1, checksum=1.
        let db = Database::parse(MINI).unwrap();
        assert_eq!(db.adventure_number, 1);
    }
    #[test]
    fn missing_trailer_leaves_adventure_number_zero() {
        // Same header/body as MINI, but with the trailer entirely removed.
        const NO_TRAILER: &str = r#"
32767 1 0 1 2 6 1 0 3 125 0 1
150 1 0 0 0 0 0 0
"" ""
"GO" "NORTH"
0 0 0 0 0 0 "*limbo"
2 0 0 0 0 0 "*forest clearing"
0 0 0 0 0 0 "*swamp"
""
"" 0
"*a brass lamp/LAMP/" 1
"#;
        let db = Database::parse(NO_TRAILER).expect("parse without trailer");
        assert_eq!(db.adventure_number, 0);
    }
    #[test]
    fn tiny_cave_fixture_reports_adventure_number() {
        let db = Database::parse(include_str!("../tests/tiny_cave.dat")).expect("parse fixture");
        assert_eq!(db.adventure_number, 99);
    }
    #[test]
    fn sniff_accepts_scott_rejects_garbage() {
        assert!(looks_like_scott(MINI));
        assert!(!looks_like_scott("This is a plain english sentence."));
        assert!(!looks_like_scott("\x01\x02\x03 not text"));
    }
    #[test]
    fn negative_count_errors_not_panics() {
        // NumActions = -1 (field index 2) must yield a LoadError, not a panic
        let bad = "32767 1 -1 1 2 6 1 0 3 125 0 1\n";
        assert!(Database::parse(bad).is_err());
    }

    #[test]
    fn out_of_range_start_room_errors_not_panics() {
        // MINI has NumRooms=2 (3 slots: 0..2). A start room of 9 (field index 6)
        // is out of range and must yield a LoadError rather than panic at runtime.
        const BAD_START: &str = r#"
32767 1 0 1 2 6 9 0 3 125 0 1
150 1 0 0 0 0 0 0
"" ""
"GO" "NORTH"
0 0 0 0 0 0 "*limbo"
2 0 0 0 0 0 "*forest clearing"
0 0 0 0 0 0 "*swamp"
""
"" 0
"*a brass lamp/LAMP/" 1
"#;
        assert!(Database::parse(BAD_START).is_err());
        // The sniff should also reject it (start room outside 0..=num_rooms).
        assert!(!looks_like_scott(BAD_START));
    }

    // --- Dialect detection (SQ-1414).
    //
    // Each fixture below is BUILT FROM THE FORMAT, not copied from a game
    // file: `dictionary` lays out fixed-width dictionary entries exactly as
    // [`Dialect`]'s doc describes them, so a test failing here means either
    // the detector or that description is wrong. The real game files the
    // description was measured from are not redistributable and are not
    // committed — `tests/dialect_specimens.rs` re-measures them when they
    // are present.

    /// An Adventure International dictionary: `words` laid out in
    /// fixed-width `field`-byte slots, NUL-padded. That one rule covers
    /// both layouts [`Dialect`] describes — they differ in the field width
    /// (word length + 1 where a NUL pad is reserved, word length where it
    /// is not) and in how a synonym is marked (a leading `*`, or an
    /// upper-case first letter), neither of which this helper needs to know.
    fn dictionary(field: usize, words: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for w in words {
            assert!(w.len() <= field, "{w:?} does not fit a {field}-byte field");
            out.extend_from_slice(w.as_bytes());
            out.resize(out.len() + field - w.len(), 0);
        }
        out
    }

    /// Wrap a table in plausible surrounding memory, so the test exercises
    /// the unanchored scan rather than a match at offset 0.
    fn in_memory_image(table: &[u8]) -> Vec<u8> {
        let mut image = vec![0xC9u8; 4096];
        image.extend_from_slice(table);
        image.extend_from_slice(&[0x00; 512]);
        image
    }

    #[test]
    fn detects_the_four_letter_dictionary_by_its_first_two_entries() {
        // Word length 4 -> 5-byte fields, the layout measured on
        // m1goldba.z80 and supergra.z80.
        let table = dictionary(5, &["AUTO", "GO", "SWIM", "*SQUE", "BURN"]);
        assert_eq!(&table[..8], b"AUTO\0GO\0");
        assert_eq!(
            detect_dialect(&in_memory_image(&table)),
            Some(Dialect::C64OrZxSnapshot)
        );
    }

    #[test]
    fn detects_the_three_letter_dictionary() {
        // Word length 3 -> 4-byte fields.
        let table = dictionary(4, &["AUT", "GO", "SWI", "*SQU"]);
        assert_eq!(&table[..7], b"AUT\0GO\0");
        assert_eq!(
            detect_dialect(&in_memory_image(&table)),
            Some(Dialect::C64OrZxSnapshot)
        );
    }

    #[test]
    fn detects_the_five_letter_claymorgue_dictionary() {
        // Word length 5 -> 6-byte fields, and this table opens at GO
        // rather than AUTO.
        let table = dictionary(6, &["GO", "*CROSS", "*RUN", "CLIMB"]);
        assert_eq!(&table[..17], b"GO\0\0\0\0*CROSS*RUN\0");
        assert_eq!(
            detect_dialect(&in_memory_image(&table)),
            Some(Dialect::C64OrZxSnapshot)
        );
    }

    #[test]
    fn detects_the_packed_dictionary_and_prefers_it_over_the_unpacked_ones() {
        // Word length 4 -> 4-byte fields with no pad; a lower-case first
        // letter is the canonical word, upper-case a synonym of it. The
        // layout measured on seablood.z80 and sherwood.z80.
        let table = dictionary(4, &["aUTO", "gO", "CLIM", "ENTE", "RUN", "BOAR"]);
        assert_eq!(&table[..7], b"aUTOgO\0");
        assert_eq!(
            detect_dialect(&in_memory_image(&table)),
            Some(Dialect::CompressedActionTable)
        );
    }

    #[test]
    fn detects_a_ti994a_image_by_its_ten_byte_marker() {
        // Real images carry this 1417 bytes into the file; the scan is
        // unanchored, so the fixture puts it somewhere else on purpose.
        let mut image = vec![0u8; 3000];
        image[900..910].copy_from_slice(b"\x30\x30\x30\x30\x00\x30\x30\x00\x28\x28");
        assert_eq!(detect_dialect(&image), Some(Dialect::Ti994aBytecode));
    }

    #[test]
    fn ti994a_marker_wins_over_a_dictionary_in_the_same_file() {
        // Ordering is load-bearing: a TI-99/4A image is a whole different
        // container, so its marker settles the question even if some byte
        // run elsewhere spells a dictionary.
        let mut image = in_memory_image(&dictionary(5, &["AUTO", "GO", "SWIM"]));
        let at = image.len();
        image.extend_from_slice(b"\x30\x30\x30\x30\x00\x30\x30\x00\x28\x28");
        assert!(at > 0);
        assert_eq!(detect_dialect(&image), Some(Dialect::Ti994aBytecode));
    }

    #[test]
    fn ordinary_files_are_not_claimed_by_any_dialect() {
        // The detector exists to NAME a refusal, so a false positive is
        // worse than no answer: it would blame the wrong format for a
        // failure that has some other cause.
        assert_eq!(detect_dialect(b""), None);
        assert_eq!(detect_dialect(MINI.as_bytes()), None);
        assert_eq!(detect_dialect(b"AUTO GO SWIM SQUEEZE"), None);
        assert_eq!(detect_dialect(&vec![0u8; 8192]), None);
        assert_eq!(detect_dialect(include_bytes!("../tests/tiny_cave.dat")), None);
    }

    #[test]
    fn a_dialect_image_is_refused_by_name_rather_than_by_lexer_accident() {
        // The whole point of the variant: `parse` must not report whatever
        // token error the text lexer happened to trip over first.
        let image = in_memory_image(&dictionary(5, &["AUTO", "GO", "SWIM"]));
        assert_eq!(
            Database::parse(&image),
            Err(LoadError::UnsupportedDialect(Dialect::C64OrZxSnapshot))
        );
    }

    #[test]
    fn a_ti99_signature_in_a_file_that_is_not_one_is_refused_as_damaged_data() {
        // SQ-1414 changed this half of the contract: the TI-99/4A dialect is
        // LOADED now, so `parse` hands a signature match to
        // `parse_ti994a` and reports what that finds. Here the signature
        // sits too near the start of the file for the baseline subtraction
        // to stay non-negative, so it is `BadDialectData` — "a format I can
        // read, in a file that is damaged" — and no longer
        // `UnsupportedDialect`, which now means only "a format I cannot
        // read".
        let mut ti = vec![0u8; 3000];
        ti[900..910].copy_from_slice(b"\x30\x30\x30\x30\x00\x30\x30\x00\x28\x28");
        assert!(matches!(
            Database::parse(&ti),
            Err(LoadError::BadDialectData(Dialect::Ti994aBytecode, _))
        ));
    }

    #[test]
    fn a_valid_dat_is_never_diverted_into_a_dialect_refusal() {
        // Detection runs only after `parse_scottfree` fails, so a good file
        // cannot be refused however its bytes read.
        assert!(Database::parse(MINI).is_ok());
        assert!(Database::parse(include_bytes!("../tests/tiny_cave.dat")).is_ok());
    }

    // --- `next_str` (SQ-1447), one case per rule in spec §2.6. Expected
    // values are taken from the spec text itself, not from running any prior
    // implementation.

    #[test]
    fn next_str_empty_string() {
        let mut lex = Lexer::new(b"\"\"");
        assert_eq!(lex.next_str().unwrap(), "");
    }

    #[test]
    fn next_str_plain_content() {
        let mut lex = Lexer::new(b"\"hello world\"");
        assert_eq!(lex.next_str().unwrap(), "hello world");
    }

    #[test]
    fn next_str_skips_leading_whitespace_of_every_kind() {
        // Spaces, tabs, CRs, LFs, vertical tabs and form feeds are all
        // "whitespace before the opening quote" per §2.6.
        let mut lex = Lexer::new(b" \t\r\n\x0b\x0c\"x\"");
        assert_eq!(lex.next_str().unwrap(), "x");
    }

    #[test]
    fn next_str_rejects_a_non_quote_byte_where_a_string_must_start() {
        let mut lex = Lexer::new(b"nope\"");
        assert_eq!(lex.next_str(), Err(LoadError::Unterminated));
    }

    #[test]
    fn next_str_rejects_end_of_file_with_no_opening_quote() {
        let mut lex = Lexer::new(b"   ");
        assert_eq!(lex.next_str(), Err(LoadError::Unterminated));
    }

    #[test]
    fn next_str_doubled_quote_is_an_escaped_literal_quote() {
        let mut lex = Lexer::new(b"\"say \"\"hi\"\" now\"");
        assert_eq!(lex.next_str().unwrap(), "say \"hi\" now");
    }

    #[test]
    fn next_str_backtick_becomes_a_double_quote_character() {
        // "Rusty axe (Magic word BUNYON on it)/AXE/" with backticks around
        // the magic word displays with real quotes, per §2.6's own example.
        let mut lex = Lexer::new(b"\"Magic word `BUNYON` on it\"");
        assert_eq!(lex.next_str().unwrap(), "Magic word \"BUNYON\" on it");
    }

    #[test]
    fn next_str_order_of_rules_backtick_then_doubled_quote() {
        // The spec's own worked example: backtick, ", ", X inside a string
        // must read as `""X` (a quote from the backtick, then one escaped
        // quote, then X) — not as an escaped quote followed by a fresh
        // opening delimiter.
        let mut lex = Lexer::new(b"\"`\"\"X\"");
        assert_eq!(lex.next_str().unwrap(), "\"\"X");
    }

    #[test]
    fn next_str_embedded_newline_is_preserved() {
        let mut lex = Lexer::new(b"\"line one\nline two\"");
        assert_eq!(lex.next_str().unwrap(), "line one\nline two");
    }

    #[test]
    fn next_str_carriage_return_is_dropped() {
        let mut lex = Lexer::new(b"\"line one\r\nline two\"");
        assert_eq!(lex.next_str().unwrap(), "line one\nline two");
    }

    #[test]
    fn next_str_non_ascii_byte_becomes_a_question_mark() {
        let mut bytes = vec![b'"'];
        bytes.extend_from_slice(b"before ");
        bytes.push(0xE9);
        bytes.extend_from_slice(b" after");
        bytes.push(b'"');
        let mut lex = Lexer::new(&bytes);
        assert_eq!(lex.next_str().unwrap(), "before ? after");
    }

    #[test]
    fn next_str_unterminated_at_eof_is_an_error() {
        let mut lex = Lexer::new(b"\"never closes");
        assert_eq!(lex.next_str(), Err(LoadError::Unterminated));
    }

    #[test]
    fn next_str_leaves_the_byte_after_closing_quote_for_the_next_token() {
        let mut lex = Lexer::new(b"\"a\" 42");
        assert_eq!(lex.next_str().unwrap(), "a");
        assert_eq!(lex.next_int().unwrap(), 42);
    }

    // --- `extract_auto_noun` (SQ-1445), one case per edge in spec §2.5.
    // Expected values are taken from the spec text itself.

    #[test]
    fn auto_noun_no_slash_leaves_text_untouched() {
        let mut text = "plain description".to_string();
        assert_eq!(extract_auto_noun(&mut text), None);
        assert_eq!(text, "plain description");
    }

    #[test]
    fn auto_noun_splits_at_the_first_slash_not_the_last() {
        // §2.5's own distinguishing example.
        let mut text = "Luger/LUGER/GUN/".to_string();
        assert_eq!(extract_auto_noun(&mut text).as_deref(), Some("LUGER"));
        assert_eq!(text, "Luger");
    }

    #[test]
    fn auto_noun_exact_double_slash_suffix_is_no_marker_and_stays_on_screen() {
        let mut text = "lamp//".to_string();
        assert_eq!(extract_auto_noun(&mut text), None);
        assert_eq!(text, "lamp//");
    }

    #[test]
    fn auto_noun_slash_slash_x_falls_through_to_an_empty_auto_noun() {
        // Remainder "//X" is not exactly "//", so this is a marker after
        // all: displayed text loses the suffix, but the zero-length span
        // between the two slashes yields an empty (unmatchable) auto-noun.
        let mut text = "lamp//X".to_string();
        assert_eq!(extract_auto_noun(&mut text).as_deref(), Some(""));
        assert_eq!(text, "lamp");
    }

    #[test]
    fn auto_noun_exact_slash_star_suffix_is_no_marker() {
        let mut text = "foo/*".to_string();
        assert_eq!(extract_auto_noun(&mut text), None);
        assert_eq!(text, "foo/*");
    }

    #[test]
    fn auto_noun_missing_closing_slash_takes_the_rest_of_the_string() {
        // §2.5's real-database example: a first slash with no second one.
        let mut text = "combination torture chamber/rec room".to_string();
        assert_eq!(extract_auto_noun(&mut text).as_deref(), Some("rec room"));
        assert_eq!(text, "combination torture chamber");
    }

    #[test]
    fn auto_noun_treasure_asterisk_is_untouched_by_the_split() {
        // The `*` marker is examined on the text before the auto-noun
        // split (by the caller, not this function), so it must survive
        // here as plain leading/trailing text.
        let mut text = "*Pot of RUBIES*/RUB/".to_string();
        assert_eq!(extract_auto_noun(&mut text).as_deref(), Some("RUB"));
        assert_eq!(text, "*Pot of RUBIES*");
    }

    #[test]
    fn auto_noun_no_case_conversion_is_applied() {
        let mut text = "thing/MiXed/".to_string();
        assert_eq!(extract_auto_noun(&mut text).as_deref(), Some("MiXed"));
    }

    #[test]
    fn auto_noun_no_whitespace_is_trimmed_from_either_side_of_the_slash() {
        let mut text = "thing / padded noun /".to_string();
        assert_eq!(extract_auto_noun(&mut text).as_deref(), Some(" padded noun "));
        assert_eq!(text, "thing ");
    }
}
