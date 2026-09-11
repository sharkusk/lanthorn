//! Reader for Infocom's **packed Apple volume** — the segmented `.D1`…`.D5`
//! container the Apple II releases of the graphical Version 6 games ship their
//! story in (SQ-0852).
//!
//! # What it is, and why it is not a filesystem
//!
//! *Arthur* and *Journey* do not put a story FILE on their ProDOS disks. What
//! they put there is a handful of opaque segments — `JOURNEY.D1`…`JOURNEY.D4`,
//! `ARTHUR.1/ARTHUR.D1`…`ARTHUR.5/ARTHUR.D5` — and the story is a **paging
//! image scattered across them by 512-byte block**. There is no filename to
//! find, no length, no contiguity: page 34 of the story can sit on the fourth
//! floppy while page 35 sits on the first. This module is the map that puts
//! them back in order.
//!
//! The container is a second container *inside* the filesystem volume, and it
//! is not the filesystem's business: the same index, byte for byte, addresses
//! the raw 5.25-inch pressings of *Shogun* and *Zork Zero*, which carry no
//! filesystem at all. So it lives on its own and takes a set of named byte
//! blobs rather than a [`crate::medium::Volume`].
//!
//! # The index
//!
//! Block 0 of the FIRST segment is an index, big-endian throughout:
//!
//! ```text
//!   0..2   unidentified (0x0083 Journey, 0x00fb Arthur, 0x00b9 Shogun,
//!                        0x0109 Zork Zero) — not a count, length or release
//!   2..4   number of segments, i.e. how many floppies the release was pressed on
//!   4..20  reserved, zero on every image in the corpus
//!  20..    one entry per segment, in segment order
//! ```
//!
//! and each entry is an 8-byte header followed by its runs:
//!
//! ```text
//!   0..2   SGTCHKS  a 16-bit checksum: the sum of the bytes of every block this
//!                   segment contributes (see below — corroboration, not a key)
//!   2..4   SGTPICOF the BLOCK this segment's picture archive starts at, or 0
//!                   for a segment carrying no artwork
//!   4..6   SGTNSEG  number of runs
//!   6..8   SGTGPOF  the block of the global picture directory, or 0
//!   8..    SGTSEG   `runs` records of six bytes: FIRST logical page, LAST
//!                   logical page, FIRST physical block, inclusive, big-endian
//! ```
//!
//! Those names are Infocom's, from `apple/yzip/rel.15/zip.equ`:
//!
//! ```text
//! SGTCHKS  EQU 0  ; check sum for file
//! SGTPICOF EQU 2  ; picture data offset
//! SGTNSEG  EQU 4  ; # of segments in this list
//! SGTGPOF  EQU 6  ; Global Directory Offset
//! SGTSEG   EQU 8  ; start of segments
//! ```
//!
//! Fields 2 and 6 were previously read here as page counts that "nothing
//! depends on" — they are not counts at all, and correcting that is what let
//! the artwork be found (SQ-0863). `pic.asm`'s `READ_IN_PDATA` turns `SGTPICOF`
//! into a file position by shifting it left nine, which is what makes it a
//! block number, and treats zero as "no picture data":
//!
//! ```text
//! lda (DSEGS),Y           ; get MSB
//! sta PFSEEK+SM_FPOS+2    ; Byte 2
//! iny                     ; point to LSB
//! ora (DSEGS),Y           ; is there any pic data?
//! bne GTPD00              ; yes
//! ...                     ; nope
//! GTPD00:
//! lda (DSEGS),Y           ; get it for shifting
//! asl A                   ; *2
//! sta PFSEEK+SM_FPOS+1    ; stash away
//! rol PFSEEK+SM_FPOS+2    ; pick up carry
//! ```
//!
//! *Arthur*'s five segments read 0, 209, 60, 67 and 38, and an archive header
//! sits at exactly those blocks of segments 2..=5 — disk 1 carries the story
//! preload and no art. See [`picture_offsets`] and [`pictures`].
//!
//! A run says "story pages `first..=last` live on this segment starting at
//! block `physical`", so reading is a scatter-gather: walk every entry's runs,
//! fill a page table, then concatenate the pages in logical order. **The runs
//! tile the story's pages exactly** — every page from 0 to the last is named
//! once, which is the structural check this reader leans on hardest, because a
//! file that is not a packed volume has no reason to produce a gapless tiling.
//!
//! The index itself lives in the volume's own block space: on the ProDOS
//! releases it occupies block 0 of segment 1 and the story's page 0 is at block
//! 1, so the physical numbers are segment-file block numbers directly.
//!
//! # Why the checksum is required
//!
//! Nothing in a reassembled story says it was reassembled correctly — the pages
//! are opaque and a wrong map produces a plausible-looking file. The Z-machine
//! header carries the one oracle that settles it: `$1C` is the sum of every
//! byte from `$40` to the declared length (ZMSD §11.1.6), so this reader
//! ASSEMBLES and then VERIFIES, and hands back nothing that does not check out.
//! That is also what keeps a `.D1` full of something else from being misread
//! rather than refused.
//!
//! It is a strict test and it is meant to be: it is how *Arthur* release 63 was
//! proven readable, and it is what would catch a segment silently swapped for
//! another release's.
//!
//! # The entry checksum
//!
//! Entry `k`'s first word is the 16-bit sum of the bytes of the blocks segment
//! `k` contributes — verified on four of *Arthur*'s five segments and on
//! *Journey*'s. `ARTHUR.2/ARTHUR.D2` on the image in `stories/` does **not**
//! match it while still reassembling into a story whose header checksum is
//! exact, so that segment is a patched press and the word cannot be used to
//! identify a segment. It is documented here because it is real, and declined
//! as a key for the same reason.
//!
//! # Segments are paired by name, and then proven by content
//!
//! Recognising the FORMAT is by content, as everywhere in this crate: a segment
//! is the first segment because its block 0 parses as an index. Pairing the
//! REST is by name — `JOURNEY.D1`'s siblings are `JOURNEY.D2`…, `ARTHUR.D1`'s
//! are `ARTHUR.D2`… under their own directories — because the segments are
//! opaque page stores with nothing in them to sort by. The name only proposes;
//! the header checksum disposes.

use crate::adf::looks_like_story;
use crate::infocom_pics::InfocomPics;

/// The block size the container pages in. ProDOS's, and the unit every physical
/// and logical number in the index counts.
const BLOCK: usize = 512;

/// How many segments an index may claim. Five is the most any release used
/// (*Arthur*, *Shogun*); the ceiling is a sanity bound on arbitrary bytes.
const MAX_SEGMENTS: usize = 8;

/// How many runs one segment's entry may hold. *Arthur*'s second segment uses
/// 35 and *Zork Zero*'s second uses 31; the bound keeps a nonsense count from
/// making this reader walk a whole disk image.
const MAX_RUNS: usize = 512;

/// One run: a contiguous range of story pages living at a contiguous range of
/// blocks on one segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    /// First story page this run supplies, inclusive.
    first_page: usize,
    /// Last story page this run supplies, inclusive.
    last_page: usize,
    /// Block on the segment holding `first_page`.
    block: usize,
}

/// The parsed index: one list of runs per segment, in segment order, and where
/// each segment keeps its artwork.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Index {
    segments: Vec<Vec<Run>>,
    /// `SGTPICOF` per segment: the block its picture archive starts at, or 0 for
    /// a segment that carries none. See [`picture_offsets`].
    pic_blocks: Vec<usize>,
}

/// Parse the index at the start of `first`, or `None` when these bytes are not
/// one.
///
/// Deliberately strict — see the module header. Everything a packed volume
/// states about itself has to be consistent before a single page is read.
fn parse_index(first: &[u8]) -> Option<Index> {
    if first.len() < BLOCK {
        return None;
    }
    let word = |o: usize| -> Option<usize> {
        let b = first.get(o..o + 2)?;
        Some(usize::from(u16::from_be_bytes([b[0], b[1]])))
    };
    // Bytes 4..20 are reserved and zero on every image in the corpus. It is the
    // cheapest thing that tells an index apart from arbitrary data.
    if first[4..20].iter().any(|&b| b != 0) {
        return None;
    }
    let count = word(2)?;
    if count == 0 || count > MAX_SEGMENTS {
        return None;
    }
    let mut at = 20;
    let mut segments = Vec::with_capacity(count);
    let mut pic_blocks = Vec::with_capacity(count);
    for _ in 0..count {
        let runs = word(at + 4)?;
        if runs == 0 || runs > MAX_RUNS {
            return None;
        }
        pic_blocks.push(word(at + 2)?);
        at += 8;
        let mut out = Vec::with_capacity(runs);
        for _ in 0..runs {
            let (first_page, last_page, block) = (word(at)?, word(at + 2)?, word(at + 4)?);
            if first_page > last_page {
                return None;
            }
            out.push(Run { first_page, last_page, block });
            at += 6;
        }
        segments.push(out);
    }
    Some(Index { segments, pic_blocks })
}

/// The page table the index describes: for each story page, which segment and
/// which block on it.
///
/// `None` unless the runs tile `0..len` exactly — no gap anywhere below the
/// highest page named. Overlaps are allowed and the FIRST claim wins: both
/// *Journey* and *Arthur* genuinely name one page twice, and both reassemble to
/// a story whose header checksum is exact either way.
fn page_table(index: &Index) -> Option<Vec<(usize, usize)>> {
    let pages = index
        .segments
        .iter()
        .flat_map(|runs| runs.iter())
        .map(|r| r.last_page + 1)
        .max()
        .filter(|&n| n > 0)?;
    let mut table = vec![None; pages];
    for (segment, runs) in index.segments.iter().enumerate() {
        for run in runs {
            for (step, page) in (run.first_page..=run.last_page).enumerate() {
                table[page].get_or_insert((segment, run.block + step));
            }
        }
    }
    table.into_iter().collect()
}

/// The name of segment `n` (1-based) of a set whose first segment is stored as
/// `first`.
///
/// The releases spell it two ways and this covers both: `JOURNEY.D1` sits beside
/// `JOURNEY.D2` at the volume root, and `ARTHUR.1/ARTHUR.D1` beside
/// `ARTHUR.2/ARTHUR.D2`, so the match is on the trailing digit of the BASENAME
/// and the directory is not consulted at all.
fn sibling_of(first: &str, n: usize) -> Option<String> {
    let base = first.rsplit('/').next()?;
    let stem = base.strip_suffix('1')?;
    Some(format!("{stem}{n}"))
}

/// Reassemble the story a packed Apple volume holds, out of the segment files
/// `files` — or `None` when these files are not one, or do not hold all of it.
///
/// The returned name is the FIRST segment's, the one carrying the index: it is
/// where the volume begins, and it is a name the caller was shown and can ask
/// for again.
///
/// A set missing a segment is `None` rather than a truncated story. That is not
/// hypothetical — `stories/Journey.2mg` declares five segments and carries four,
/// so 92 of its 552 pages are simply not on the image.
pub fn story(files: &[(String, Vec<u8>)]) -> Option<(String, Vec<u8>)> {
    files.iter().find_map(|(name, bytes)| assemble(name, bytes, files))
}

/// The **header page** of the story a packed volume pages across, with the name
/// of the segment carrying the index (SQ-0867).
///
/// # Why a page rather than the story
///
/// A build's whole name is in the first 30 bytes. Quetzal §5.4 defines the Game
/// Identifier as release (`$02`), serial (`$12`) and checksum (`$1C`), and all
/// three sit in page 0 — so *which build is this release* is a question one
/// block answers, and reassembling 344 KB to read 30 bytes of it would be an
/// absurd way to ask. [`crate::GameIdentifier::of_story`] takes what this
/// returns.
///
/// Unlike [`story`] it therefore does not need every segment — only the one the
/// index names as page 0, which on every release in the corpus is segment 1, the
/// segment the index itself came off. [`picture_offsets`] already answers for a
/// partial set on the same footing and for the same reason.
///
/// That is what makes `stories/Journey.2mg` answerable at all. Its index
/// declares five segments and the image carries four, so 92 of its 552 pages are
/// absent and [`story`] rightly refuses it — but page 0 is on `JOURNEY.D1` and
/// intact, and it says release 77, serial 890616. A release that cannot be
/// *played* off this image can still say what it is.
///
/// # What stands in for the checksum
///
/// [`story`] leans on the header checksum because reassembly is the risk: a
/// wrong page map produces a plausible-looking file out of correct bytes, and
/// only the checksum catches it. Reading ONE page the index points at is not
/// that operation and cannot fail that way — there is no map to get wrong.
///
/// What has to be excluded instead is a block of arbitrary data being read as a
/// header, and `states_a_story_this_index_tiles` is that test: the page must
/// be a Z-machine header whose own declared story length lands inside the LAST
/// block the index tiles. Two independent structures then have to agree about
/// how long this story is, to within one block, which arbitrary data has no
/// reason to do. Measured slack on the three packed releases here is 448, 352
/// and 56 bytes — all inside one 512-byte page, none of them zero.
///
/// It is corroborated where corroboration is available: for
/// `Arthur Quest 4 Excalibur.2mg` and the five-volume `shogun_s*.dsk` press,
/// which [`story`] CAN reassemble and verify, this page reports the identical
/// build — release 63 / serial 890622 and release 311 / serial 890510.
pub fn story_header(files: &[(String, Vec<u8>)]) -> Option<(String, Vec<u8>)> {
    files.iter().find_map(|(name, bytes)| header_page(name, bytes, files))
}

/// [`story_header`], taking `name`/`first` as the segment holding the index.
fn header_page(name: &str, first: &[u8], files: &[(String, Vec<u8>)]) -> Option<(String, Vec<u8>)> {
    let index = parse_index(first)?;
    let table = page_table(&index)?;
    let &(segment, block) = table.first()?;
    // Segment 1 is the file the index came off; the rest are its namesakes,
    // paired exactly as `assemble` pairs them.
    let bytes: &[u8] = if segment == 0 {
        first
    } else {
        let want = sibling_of(name, segment + 1)?;
        files
            .iter()
            .find(|(other, _)| {
                other.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case(&want))
            })
            .map(|(_, b)| b.as_slice())?
    };
    let at = block.checked_mul(BLOCK)?;
    let page = bytes.get(at..at + BLOCK)?;
    states_a_story_this_index_tiles(page, table.len() * BLOCK)
        .then(|| (name.to_string(), page.to_vec()))
}

/// Is `page` a Z-machine header whose story is the length `tiled` bytes of
/// pages could hold? See [`story_header`] for why this is the test.
///
/// The structural checks are [`looks_like_story`]'s, minus the ones that need
/// the whole file present and re-bounded by the story's DECLARED length instead
/// of by the slice — which is the entire difference, and the reason this cannot
/// simply call it: that function bounds every table by `bytes.len()`, and here
/// `bytes` is one page of a story three hundred times longer.
fn states_a_story_this_index_tiles(page: &[u8], tiled: usize) -> bool {
    if page.len() < 64 {
        return false;
    }
    let word = |o: usize| usize::from(u16::from_be_bytes([page[o], page[o + 1]]));
    // ZMSD §11.1.6, as in `verified`: the length field counts in the version's
    // packed-address scale.
    let scale = match page[0] {
        3 => 2,
        4 | 5 => 4,
        6..=8 => 8,
        _ => return false,
    };
    // The story must end inside the pages the index tiles, and inside the LAST
    // of them — a page table tiles the story it maps and not a byte more, so a
    // declared length that stops a whole block short means these two structures
    // are not describing one story. Zero ("not recorded") cannot be tolerated
    // here the way `looks_like_story` tolerates it: it is the check.
    let length = word(0x1a) * scale;
    if length < 64 || length > tiled || tiled - length >= BLOCK {
        return false;
    }
    let (high, dict, objects, globals, static_base) =
        (word(0x04), word(0x08), word(0x0a), word(0x0c), word(0x0e));
    if !(64..=length).contains(&static_base) {
        return false;
    }
    // Object and global tables are writable, so they live in dynamic memory.
    if !(64..static_base).contains(&objects) || !(64..static_base).contains(&globals) {
        return false;
    }
    // High memory begins at or after static memory; the dictionary is in static.
    if high < static_base || high > length || dict < static_base || dict >= length {
        return false;
    }
    // Serial is six printable characters, in ASCII or the Apple II's high ASCII
    // — the mask is `looks_like_story`'s and is argued there (SQ-0856).
    page[0x12..0x18].iter().all(|c| (0x20..0x7f).contains(&(c & 0x7f)))
}

/// Where each segment of a packed volume keeps its picture archive: the byte
/// offset of the archive's 16-byte header, or `None` for a segment carrying no
/// artwork. Indexed by segment, so entry 0 is `…D1`.
///
/// `None` overall when `files` are not a packed volume. Unlike [`story`] this
/// does not need every segment to be present — it reads the index and nothing
/// else, so it answers for a volume that is missing a floppy.
///
/// The offsets come from `SGTPICOF`; see the module header for the source and
/// for why a zero means "none" rather than "block zero".
pub fn picture_offsets(files: &[(String, Vec<u8>)]) -> Option<(String, Vec<Option<usize>>)> {
    files.iter().find_map(|(name, bytes)| {
        let index = parse_index(bytes)?;
        // The index alone is not proof; `story` leans on the page tiling for
        // exactly this reason, and it costs nothing to demand it here too.
        page_table(&index)?;
        let offsets = index
            .pic_blocks
            .iter()
            .map(|&b| (b != 0).then(|| b * BLOCK))
            .collect::<Vec<_>>();
        Some((name.clone(), offsets))
    })
}

/// Every picture a packed Apple volume carries, merged into one archive.
///
/// The artwork is spread over the segments — *Arthur* keeps 51, 26, 53 and 38
/// pictures on floppies 2 to 5 — and the parts partition the id space rather
/// than overlapping it, so a caller wants all of them or none. Each part is
/// parsed against the segment that carries it (its offsets are positions in the
/// SEGMENT — see [`crate::infocom_pics::InfocomPics::parse_apple`]) and then
/// folded together with `append_part`, which numbers Apple parts by floppy and
/// so runs 2, 3, 4, 5.
///
/// `None` when these files are not a packed volume, when no segment carries
/// artwork, or when a segment that should carry some is missing — a partial
/// picture set would silently lose whole rooms.
///
/// The story's own artwork is NOT the same thing as the story: [`story`] will
/// refuse a volume this accepts and vice versa, because they lean on different
/// parts of the index. `stories/Journey.2mg` is the case in hand — four segments
/// where the index declares five, so it yields no story; its `SGTPICOF` fields
/// name artwork on segments 2 to 5, and the missing segment is 5, so it yields
/// no artwork either. What it does still yield is its BUILD, which needs only
/// the one page segment 1 carries — see [`story_header`].
pub fn pictures(files: &[(String, Vec<u8>)]) -> Option<(String, InfocomPics)> {
    let (first, offsets) = picture_offsets(files)?;
    let mut set: Option<InfocomPics> = None;
    for (n, offset) in offsets.iter().enumerate() {
        let Some(offset) = *offset else { continue };
        // Segment 1 is the file the index came off; the rest are its namesakes,
        // paired exactly as `assemble` pairs them.
        let bytes = if n == 0 {
            files.iter().find(|(other, _)| *other == first).map(|(_, b)| b)?
        } else {
            let want = sibling_of(&first, n + 1)?;
            files
                .iter()
                .find(|(other, _)| {
                    other.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case(&want))
                })
                .map(|(_, b)| b)?
        };
        let part = InfocomPics::parse_apple(bytes.clone(), offset).ok()?;
        match &mut set {
            None => set = Some(part),
            Some(held) => held.append_part(part).ok()?,
        }
    }
    // Named for the segment carrying the index, exactly as `story` is: it is
    // where the volume begins, and it is a name the caller was shown.
    set.map(|pics| (first, pics))
}

/// Reassemble, taking `first` as the segment holding the index.
fn assemble(name: &str, first: &[u8], files: &[(String, Vec<u8>)]) -> Option<(String, Vec<u8>)> {
    let index = parse_index(first)?;
    let table = page_table(&index)?;

    // Segment 1 is the file the index came off; the rest are its namesakes.
    let mut segments: Vec<&[u8]> = Vec::with_capacity(index.segments.len());
    segments.push(first);
    for n in 2..=index.segments.len() {
        let want = sibling_of(name, n)?;
        let found = files
            .iter()
            .find(|(other, _)| other.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case(&want)))?;
        segments.push(&found.1);
    }

    let mut story = Vec::with_capacity(table.len() * BLOCK);
    for &(segment, block) in &table {
        let at = block.checked_mul(BLOCK)?;
        story.extend_from_slice(segments[segment].get(at..at + BLOCK)?);
    }
    verified(story).map(|bytes| (name.to_string(), bytes))
}

/// `story` truncated to its declared length, if it really is a story and its
/// header checksum agrees with the bytes that were reassembled.
///
/// The whole point of the module's strictness lands here; see the header for
/// why nothing weaker will do.
///
/// **Crate-visible** since SQ-0868, because the question it answers — *are these
/// the story's own bytes, in the story's own order, all of them* — is the same
/// question a raw self-booting disk has to answer about a run of sectors, and
/// [`crate::infocom_boot`] must not have a second copy of the checksum rule to
/// get subtly wrong. It takes a `Vec` because it truncates; callers scanning for
/// a start offset should test [`looks_like_story`] on a borrow first and only
/// pay for the copy on a structural hit.
pub(crate) fn verified(mut story: Vec<u8>) -> Option<Vec<u8>> {
    if !looks_like_story(&story) {
        return None;
    }
    let word = |o: usize| usize::from(u16::from_be_bytes([story[o], story[o + 1]]));
    // ZMSD §11.1.6: the length field counts in the version's packed-address
    // scale, which is 8 for every Version 6 story — and Version 6 is the only
    // version this container was ever used for.
    let scale = match story[0] {
        3 => 2,
        4 | 5 => 4,
        6..=8 => 8,
        _ => return None,
    };
    let (length, checksum) = (word(0x1a) * scale, word(0x1c));
    if length < 64 || length > story.len() {
        return None;
    }
    let sum = story[64..length].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
    if checksum != 0 && usize::from(sum) != checksum {
        return None;
    }
    story.truncate(length);
    Some(story)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A story whose header checksum is correct for its own bytes, so the
    /// reader's one hard test can pass without a real game.
    fn story_bytes(len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[0] = 6;
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x1a, (len / 8) as u16);
        b[0x12..0x18].copy_from_slice(b"890616");
        for (i, slot) in b.iter_mut().enumerate().skip(64) {
            *slot = (i % 251) as u8;
        }
        let sum = b[64..len].iter().fold(0u16, |a, &c| a.wrapping_add(u16::from(c)));
        b[0x1c..0x1e].copy_from_slice(&sum.to_be_bytes());
        b
    }

    /// Build a two-segment packed volume holding `story`, with the pages
    /// deliberately out of order and split across both segments so that a
    /// reader which merely concatenates cannot pass.
    fn packed(story: &[u8]) -> Vec<(String, Vec<u8>)> {
        let pages = story.len().div_ceil(BLOCK);
        let half = pages / 2;
        // Segment 1 holds the index in block 0 and the SECOND half of the story
        // from block 1; segment 2 holds the first half, reversed.
        let mut d1 = vec![0u8; BLOCK * (1 + (pages - half))];
        for (i, page) in (half..pages).enumerate() {
            let at = BLOCK * (1 + i);
            let src = page * BLOCK;
            let end = (src + BLOCK).min(story.len());
            d1[at..at + (end - src)].copy_from_slice(&story[src..end]);
        }
        let mut d2 = vec![0u8; BLOCK * half];
        for page in 0..half {
            let at = BLOCK * (half - 1 - page);
            d2[at..at + BLOCK].copy_from_slice(&story[page * BLOCK..(page + 1) * BLOCK]);
        }
        let mut index: Vec<u8> = Vec::new();
        index.extend_from_slice(&0x0083u16.to_be_bytes()); // the unidentified word
        index.extend_from_slice(&2u16.to_be_bytes()); // two segments
        index.resize(20, 0);
        // Segment 1: one run, pages half..pages at block 1.
        index.extend_from_slice(&0u16.to_be_bytes());
        index.extend_from_slice(&((pages - half) as u16).to_be_bytes());
        index.extend_from_slice(&1u16.to_be_bytes());
        index.extend_from_slice(&0u16.to_be_bytes());
        index.extend_from_slice(&(half as u16).to_be_bytes());
        index.extend_from_slice(&((pages - 1) as u16).to_be_bytes());
        index.extend_from_slice(&1u16.to_be_bytes());
        // Segment 2: one run per page, descending on the disk.
        index.extend_from_slice(&0u16.to_be_bytes());
        index.extend_from_slice(&(half as u16).to_be_bytes());
        index.extend_from_slice(&(half as u16).to_be_bytes());
        index.extend_from_slice(&0u16.to_be_bytes());
        for page in 0..half {
            index.extend_from_slice(&(page as u16).to_be_bytes());
            index.extend_from_slice(&(page as u16).to_be_bytes());
            index.extend_from_slice(&((half - 1 - page) as u16).to_be_bytes());
        }
        assert!(index.len() <= BLOCK, "the sample index must fit block 0");
        d1[..index.len()].copy_from_slice(&index);
        vec![("GAME.D1".into(), d1), ("GAME.D2".into(), d2)]
    }

    #[test]
    fn a_scattered_story_is_put_back_in_logical_page_order() {
        let want = story_bytes(BLOCK * 9);
        let files = packed(&want);
        let (name, got) = story(&files).expect("the packed volume reassembles");
        assert_eq!(name, "GAME.D1", "the name is the segment carrying the index");
        assert_eq!(got, want);
    }

    /// The container's own proof of correctness. Swapping two pages leaves every
    /// structural check intact — the tiling is still gapless, every block still
    /// exists — and only the header checksum notices.
    #[test]
    fn a_map_that_puts_one_page_in_the_wrong_place_is_refused() {
        let want = story_bytes(BLOCK * 9);
        let mut files = packed(&want);
        let d1 = &mut files[0].1;
        // Entry 2's first two runs name pages 0 and 1; trade their blocks.
        let a = 20 + 8 + 6 + 8 + 4;
        let b = a + 6;
        d1.swap(a, b);
        d1.swap(a + 1, b + 1);
        assert_eq!(story(&files), None, "a mis-assembled story must not be handed out");
    }

    #[test]
    fn a_set_missing_a_segment_is_refused_rather_than_truncated() {
        let want = story_bytes(BLOCK * 9);
        let mut files = packed(&want);
        files.truncate(1);
        assert_eq!(story(&files), None, "Journey.2mg's exact defect, in two lines");
    }

    #[test]
    fn ordinary_files_are_not_packed_volumes() {
        assert_eq!(story(&[("STORY.DAT".into(), story_bytes(BLOCK * 4))]), None);
        assert_eq!(story(&[("junk".into(), vec![0u8; BLOCK * 4])]), None);
        assert_eq!(story(&[("short".into(), vec![7u8; 12])]), None);
        assert_eq!(story(&[]), None);
    }

    /// The reserved words are what a random file trips over first.
    #[test]
    fn a_first_block_with_dirt_in_its_reserved_words_is_not_an_index() {
        let want = story_bytes(BLOCK * 9);
        let mut files = packed(&want);
        files[0].1[7] = 1;
        assert_eq!(story(&files), None);
    }

    // ── SQ-0867: the header page on its own ──────────────────────────────────

    /// Build a two-segment volume with the pages IN order — segment 1 carrying
    /// the index and the first half, segment 2 the rest. The shape every real
    /// press has, and the one where dropping the last floppy still leaves page 0
    /// in hand.
    fn packed_in_order(story: &[u8]) -> Vec<(String, Vec<u8>)> {
        let pages = story.len().div_ceil(BLOCK);
        let half = pages / 2;
        let mut d1 = vec![0u8; BLOCK * (1 + half)];
        d1[BLOCK..BLOCK + half * BLOCK].copy_from_slice(&story[..half * BLOCK]);
        let mut d2 = vec![0u8; BLOCK * (pages - half)];
        d2[..story.len() - half * BLOCK].copy_from_slice(&story[half * BLOCK..]);
        let mut index: Vec<u8> = Vec::new();
        index.extend_from_slice(&0x0083u16.to_be_bytes());
        index.extend_from_slice(&2u16.to_be_bytes());
        index.resize(20, 0);
        for (runs, first_page, last_page, block) in
            [(1u16, 0u16, half as u16 - 1, 1u16), (1, half as u16, pages as u16 - 1, 0)]
        {
            index.extend_from_slice(&0u16.to_be_bytes()); // SGTCHKS
            index.extend_from_slice(&0u16.to_be_bytes()); // SGTPICOF
            index.extend_from_slice(&runs.to_be_bytes());
            index.extend_from_slice(&0u16.to_be_bytes()); // SGTGPOF
            index.extend_from_slice(&first_page.to_be_bytes());
            index.extend_from_slice(&last_page.to_be_bytes());
            index.extend_from_slice(&block.to_be_bytes());
        }
        d1[..index.len()].copy_from_slice(&index);
        vec![("GAME.D1".into(), d1), ("GAME.D2".into(), d2)]
    }

    /// The header page is the story's own first page, whichever segment the
    /// index puts it on — here, deliberately, the second one.
    #[test]
    fn the_header_page_is_read_off_whichever_segment_holds_page_zero() {
        let want = story_bytes(BLOCK * 9);
        let files = packed(&want);
        let (name, page) = story_header(&files).expect("the volume states a header");
        assert_eq!(name, "GAME.D1", "named for the segment carrying the index, as `story` is");
        assert_eq!(page, want[..BLOCK], "and it is page 0 of the story, byte for byte");
    }

    /// **`Journey.2mg`'s case, in two lines.** A set missing its last segment
    /// yields no story — and still yields the page a build is named from.
    #[test]
    fn an_incomplete_set_still_states_its_header_when_page_zero_survives() {
        let want = story_bytes(BLOCK * 9);
        let mut files = packed_in_order(&want);
        assert!(story(&files).is_some(), "the premise: complete, it reassembles");
        files.truncate(1);
        assert_eq!(story(&files), None, "incomplete, it is refused as a story");
        let (_, page) = story_header(&files).expect("but page 0 is still on segment 1");
        assert_eq!(page, want[..BLOCK]);
    }

    /// …and when page 0 is on the segment that went missing, nothing is invented.
    #[test]
    fn an_incomplete_set_states_nothing_when_page_zero_is_on_the_absent_segment() {
        let want = story_bytes(BLOCK * 9);
        let mut files = packed(&want); // this fixture puts page 0 on segment 2
        files.truncate(1);
        assert_eq!(story_header(&files), None);
    }

    /// The check that stands in for the checksum: the header's own declared
    /// length has to agree with the index's tiling to within one block. Move it
    /// a whole block and the page is no longer this index's story.
    #[test]
    fn a_header_whose_length_disagrees_with_the_tiling_is_refused() {
        let len = BLOCK * 9;
        let mut want = story_bytes(len);
        assert!(story_header(&packed_in_order(&want)).is_some(), "the premise");
        // A story one whole page shorter than the pages the index tiles.
        let short = ((len - BLOCK) / 8) as u16;
        want[0x1a..0x1c].copy_from_slice(&short.to_be_bytes());
        assert_eq!(story_header(&packed_in_order(&want)), None);
    }

    /// A block of arbitrary bytes under a well-formed index is not a header.
    #[test]
    fn arbitrary_pages_are_not_read_as_a_build() {
        let want = story_bytes(BLOCK * 9);
        let mut files = packed_in_order(&want);
        // Page 0 sits at block 1 of segment 1; fill it with dirt.
        files[0].1[BLOCK..2 * BLOCK].iter_mut().for_each(|b| *b = 0xa5);
        assert_eq!(story_header(&files), None);
        assert_eq!(story_header(&[("junk".into(), vec![0u8; BLOCK * 4])]), None);
        assert_eq!(story_header(&[]), None);
    }

    #[test]
    fn segments_are_paired_by_the_basename_under_any_directory() {
        assert_eq!(sibling_of("JOURNEY.D1", 3).as_deref(), Some("JOURNEY.D3"));
        assert_eq!(sibling_of("ARTHUR.1/ARTHUR.D1", 5).as_deref(), Some("ARTHUR.D5"));
        assert_eq!(sibling_of("STORY.DAT", 2), None);
    }

    // ── Real media ───────────────────────────────────────────────────────────

    /// The user's `stories/` directory, gitignored — every test over it skips
    /// vacuously, and CI has none of it at all.
    fn stories_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    /// Every file on a ProDOS image in `stories/`, or `None` when it is absent.
    fn volume_files(name: &str) -> Option<Vec<(String, Vec<u8>)>> {
        let path = stories_dir().join(name);
        let Ok(raw) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        };
        let fs = crate::prodos::ProDos::mount(raw).expect("a ProDOS image in the corpus mounts");
        Some(fs.files().iter().filter_map(|e| fs.read(e).map(|b| (e.path(), b))).collect())
    }

    /// **The headline.** *Arthur*'s Apple press keeps its story in five segments
    /// and no file on the volume is a story; this is the whole container,
    /// end to end, proven by the game's own header checksum.
    #[test]
    fn real_arthur_reassembles_out_of_five_segments() {
        let Some(files) = volume_files("Arthur Quest 4 Excalibur.2mg") else { return };
        assert!(
            files.iter().all(|(_, b)| !looks_like_story(b)),
            "no FILE on this volume is a story — that is what makes it a packed volume"
        );
        let (name, bytes) = story(&files).expect("the packed volume reassembles");
        assert_eq!(name, "ARTHUR.1/ARTHUR.D1", "named for the segment carrying the index");
        assert_eq!(bytes.len(), 271_304, "the story's own declared length");
        assert_eq!(bytes[0], 6, "Version 6");
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 63, "release 63");
        assert_eq!(&bytes[0x12..0x18], b"890622", "serial 890622");
        // `story` already required this; asserted again because it is the one
        // fact that says the page map was right rather than merely plausible.
        let sum = bytes[64..].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
        assert_eq!(sum, u16::from_be_bytes([bytes[0x1c], bytes[0x1d]]), "ZMSD §11.1.6 checksum");
    }

    /// **`Journey.2mg` is an incomplete pressing, and is refused as one.**
    ///
    /// Its index declares five segments; the volume carries `JOURNEY.D1` …
    /// `JOURNEY.D4` and nothing else, so 92 of the story's 552 pages are simply
    /// not on the image. A reader that concatenated what it had would produce
    /// 460 pages of plausible-looking Z-code; this one produces nothing.
    #[test]
    fn real_journey_is_refused_because_its_fifth_segment_is_absent() {
        let Some(files) = volume_files("Journey.2mg") else { return };
        let (name, first) = files
            .iter()
            .find(|(n, b)| n.ends_with("D1") && parse_index(b).is_some())
            .expect("JOURNEY.D1 carries an index");
        let index = parse_index(first).expect("…and it parses");
        assert_eq!(index.segments.len(), 5, "the index declares five segments");
        assert_eq!(
            (2..=5).filter(|&n| {
                let want = sibling_of(name, n).unwrap();
                files.iter().any(|(o, _)| o.rsplit('/').next() == Some(want.as_str()))
            })
            .count(),
            3,
            "and only D2, D3 and D4 are on the volume"
        );
        assert_eq!(story(&files), None, "so no story is handed out at all");
    }

    /// …and it still says which build it is (SQ-0867).
    ///
    /// The story cannot be reassembled and the build is not in question: page 0
    /// is on `JOURNEY.D1`, which the image has, and it reads release 77 / serial
    /// 890616 — a different build from the release 83 / serial 890706
    /// `stories/journey.z6` carries and from the release 30 / serial 890322 of
    /// the Amiga floppy.
    #[test]
    fn real_journey_states_release_77_even_though_it_cannot_be_reassembled() {
        let Some(files) = volume_files("Journey.2mg") else { return };
        let (name, page) = story_header(&files).expect("page 0 survives on JOURNEY.D1");
        assert_eq!(name, "JOURNEY.D1", "named for the segment carrying the index");
        assert_eq!(page.len(), BLOCK);
        assert_eq!(page[0], 6, "Version 6");
        let id = crate::GameIdentifier::of_story(&page).expect("a header names a build");
        assert_eq!(id.release, 77);
        assert_eq!(id.serial_str(), "890616");
    }

    /// The complete Apple press of *Arthur* is where the cheap answer can be
    /// checked against the expensive one: `story` reassembles 530 pages and
    /// verifies them against the header checksum, `story_header` reads one page,
    /// and the two must name the same build. This is what licenses trusting the
    /// page alone on *Journey*, where no checksum can be taken.
    #[test]
    fn real_arthur_states_the_same_build_from_one_page_as_from_all_of_them() {
        let Some(files) = volume_files("Arthur Quest 4 Excalibur.2mg") else { return };
        let (_, whole) = story(&files).expect("the packed volume reassembles");
        let (_, page) = story_header(&files).expect("and states a header");
        assert_eq!(page, whole[..BLOCK], "the same page, byte for byte");
        assert_eq!(
            crate::GameIdentifier::of_story(&page),
            crate::GameIdentifier::of_story(&whole),
            "release 63, serial 890622, checksum $45EB either way"
        );
    }

    /// SQ-0863: `SGTPICOF` names where each segment keeps its artwork.
    ///
    /// This field was read here as a page count "nothing depends on"; it is the
    /// picture archive's block. The check is that an archive header really is at
    /// each of the blocks it names — `PHFID` equal to the disk number, and the
    /// 8-byte record `PLDSIZE` — which is a fact about the bytes, not about this
    /// reader's arithmetic.
    #[test]
    fn real_arthur_names_a_picture_archive_on_four_of_its_five_segments() {
        let Some(files) = volume_files("Arthur Quest 4 Excalibur.2mg") else { return };
        let (name, offsets) = picture_offsets(&files).expect("the packed volume is read");
        assert_eq!(name, "ARTHUR.1/ARTHUR.D1");
        assert_eq!(
            offsets,
            vec![None, Some(209 * BLOCK), Some(60 * BLOCK), Some(67 * BLOCK), Some(38 * BLOCK)],
            "disk 1 carries the story preload and no art"
        );

        for (n, offset) in offsets.iter().enumerate() {
            let Some(offset) = *offset else { continue };
            let want = sibling_of(name.as_str(), n + 1).unwrap();
            let seg = files
                .iter()
                .find(|(o, _)| o.rsplit('/').next() == Some(want.as_str()))
                .map(|(_, b)| b)
                .unwrap();
            assert_eq!(seg[offset], (n + 1) as u8, "{want}: PHFID is the disk number");
            assert_eq!(seg[offset + 8], 8, "{want}: PLDSIZE, the 8-byte record");
        }
    }

    /// SQ-0863: the four archives fold into one set of 168 pictures.
    #[test]
    fn real_arthur_merges_its_four_picture_archives() {
        let Some(files) = volume_files("Arthur Quest 4 Excalibur.2mg") else { return };
        let (name, pics) = pictures(&files).expect("the volume carries artwork");
        assert_eq!(name, "ARTHUR.1/ARTHUR.D1", "named for the segment carrying the index");
        assert_eq!(pics.flavour(), crate::infocom_pics::Flavour::Apple);
        assert_eq!(pics.part(), 2, "numbered by floppy, so the set starts at 2");
        assert_eq!(pics.parts(), 4);
        assert_eq!(pics.entries().len(), 168);
    }

    /// SQ-0863: `Journey.2mg` gains no artwork, as it gains no story — and the
    /// reason is worth pinning, because it is not that there is none.
    ///
    /// Its index names a picture archive on all four of segments 2..=5, and
    /// three of those archives are really there, with valid headers. The fifth
    /// segment is the one the pressing is missing, so a reader that took what it
    /// could get would hand back a picture set with a floppy's worth of rooms
    /// silently absent. This one hands back nothing, for the same reason
    /// [`story`] does.
    #[test]
    fn real_journey_gains_no_artwork_because_a_segment_is_missing() {
        let Some(files) = volume_files("Journey.2mg") else { return };
        assert_eq!(story(&files), None, "still no story");

        let (name, offsets) = picture_offsets(&files).expect("its index still parses");
        assert_eq!(offsets[0], None, "disk 1 carries no art, as on Arthur");
        assert_eq!(offsets.iter().filter(|o| o.is_some()).count(), 4, "disks 2..=5 all name one");

        // Three of the four are present and really are archives; the fourth
        // segment is not on the volume at all.
        let mut present = 0;
        let mut absent = 0;
        for (n, offset) in offsets.iter().enumerate() {
            let Some(offset) = *offset else { continue };
            let want = sibling_of(name.as_str(), n + 1).unwrap();
            match files.iter().find(|(o, _)| o.rsplit('/').next() == Some(want.as_str())) {
                Some((_, seg)) => {
                    assert_eq!(seg[offset], (n + 1) as u8, "{want}: PHFID is the disk number");
                    assert_eq!(seg[offset + 8], 8, "{want}: PLDSIZE, the 8-byte record");
                    present += 1;
                }
                None => absent += 1,
            }
        }
        assert_eq!((present, absent), (3, 1), "D5 is the segment the pressing lacks");
        assert!(pictures(&files).is_none(), "so no artwork is handed out at all");
    }

    /// Nothing else in the corpus is mistaken for a packed volume — including
    /// the six *Lost Treasures* volumes, which are ProDOS disks full of ordinary
    /// story files, and the standalone *Beyond Zork*.
    #[test]
    fn no_other_prodos_image_in_the_corpus_holds_a_packed_volume() {
        let Ok(dir) = std::fs::read_dir(stories_dir()) else {
            eprintln!("SKIP: no stories directory");
            return;
        };
        let mut ran = 0;
        for entry in dir.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if !name.to_ascii_lowercase().ends_with(".2mg") {
                continue;
            }
            let Some(files) = volume_files(&name) else { continue };
            ran += 1;
            let packed = story(&files);
            let expected = name == "Arthur Quest 4 Excalibur.2mg";
            assert_eq!(packed.is_some(), expected, "{name}: packed volume found = {expected}");
        }
        if ran == 0 {
            eprintln!("SKIP: no ProDOS media present");
        }
    }
}
