//! Infocom's Commodore releases as a **raw GCR bitstream** — the `.g64`
//! container, decoded down to the sectors [`crate::d64`] already reads
//! (SQ-1095).
//!
//! # The medium, and the one difference that matters
//!
//! A `.d64` is a dump of a 1541's *decoded* 256-byte sectors. A `.g64` is a
//! dump of what the read head actually sees: sync marks, GCR-encoded header and
//! data blocks, the gaps between them, and whatever the mastering house did to
//! the parts of the disk that are not data. Decode the bitstream to sectors and
//! it **is** a D64 — so this module ends at `sector_image`, which hands
//! [`crate::d64::D64`] a 174,848-byte image and takes no further interest in
//! what a story is.
//!
//! # Two specifications, both read rather than recalled
//!
//! The container is Peter Schepers' `G64.TXT`, revision 1.9 (Feb 19 2008), from
//! contributions by Markus Brenner, Wolfgang Moser and Immers/Neufeld's *Inside
//! Commodore DOS* — the document that defines the format:
//!
//! ```text
//!   $0000-0007  "GCR-1541"
//!        0008   version ($00 is the only one defined)
//!        0009   number of half-tracks (usually $54, 84)
//!   $000A-000B  maximum track size, LO/HI
//!   $000C-…     one 4-byte LO/HI file offset per half-track; 0 = no data
//!    …          one 4-byte LO/HI speed entry per half-track; <4 is a constant
//!               speed for the whole track, >4 a file offset to a speed block
//! ```
//!
//! and each stored track is a 2-byte LO/HI length followed by that many bytes of
//! GCR, padded out to the maximum with filler. The same document gives the
//! low-level layout of a standard sector — header sync, ten GCR bytes of header,
//! header gap, data sync, 325 GCR bytes of data, tail gap — and the algorithm
//! this module follows verbatim: *"Search for SYNC (at least 10 or more 1 bits);
//! check for header id after SYNC (GCR 0x52); … check for data id after SYNC
//! (GCR 0x55)."*
//!
//! The **encoding table** is not in that document, and is taken from two
//! independent sources that agree byte for byte: VICE's `src/gcr.c`
//! (`GCR_conv_data`, Boose/Sladic/Kajtar) and Linus Åkesson's *GCR decoding on
//! the fly*, which prints the nybble-to-quintuple table in binary. `GCR` below
//! is that table; its inverse is *computed* from it at compile time rather than
//! transcribed, because a hand-written inverse is a second copy of the same fact
//! and the sort of thing a unit test written by the same hand agrees with.
//!
//! # Speed zones are read and then ignored, deliberately
//!
//! Nothing here needs them. A speed zone says how fast the drive wrote, which
//! determines how many bytes fit on a track — and the track's own 2-byte length
//! prefix already states that, exactly, for the disk in hand. How many *sectors*
//! a track carries is the 1541's geometry, which [`crate::d64`] owns and this
//! module borrows. Modelling bit timing would be an emulator, and this is a
//! reader.
//!
//! # What "decode what decodes" means, measured
//!
//! `stories/plundered_hearts[infocom_1987](r26)(!).g64` is 317,884 bytes: version
//! 0, 84 half-tracks declared, 40 whole tracks carrying data, **no half-tracks at
//! all**, tracks 1-17 stored at 7,692 bytes. Decoding it:
//!
//! ```text
//!   tracks  1-35   682 of the 683 sectors a 35-track 1541 has
//!                  (track 24 sector 18 will not decode)
//!   tracks 36-40   0 sectors of 85 — five tracks of bitstream that is not
//!                  sectors at all
//! ```
//!
//! Those six tracks are the copy protection, and lanthorn never executes the
//! loader that would check them: the story is ordinary sectors, and the scan in
//! [`crate::d64`] confirms a run of them against **the story's own header
//! checksum** rather than against anything a mastering house could forge. So an
//! undecodable track is not a failure of this reader — it is skipped, its
//! sectors left zero, and the checksum decides. The story that comes off this
//! image is byte-identical to `stories/plunderedhearts-r26-s870730.z3`, all
//! 128,962 bytes of it.
//!
//! **Tracks 36-40 are dropped rather than carried**, because a `.d64` is 35
//! tracks and the 40-track extension is a different geometry that nothing in the
//! corpus is (see [`crate::d64::D64_LEN`]). That costs nothing here and is
//! measured rather than assumed: those five tracks decode to no sectors
//! whatever.
//!
//! # The one place a block is allowed to be broken
//!
//! Both block types end in **"off" bytes** — `$0F $0F` on the header, `$00 $00`
//! on the data block, present only to make the block a multiple of five so it
//! can be GCR-encoded in whole groups. They carry nothing. And the data block's
//! last off-nibble is exactly where a drive's write splice lands: on *Plundered
//! Hearts* six sectors have a corrupt final GCR byte and are otherwise perfect,
//! which is why `block` takes the number of bytes that must decode cleanly and
//! lets an invalid quintuple past that mark through. The checksum still has to
//! pass, over the bytes that mean something.

use crate::d64::{D64, D64Error, SECTOR, TRACKS, linear, sectors_per_track};
use crate::infocom_pics::InfocomPics;

/// The container's magic, and the whole of the cheap sniff.
const SIGNATURE: [u8; 8] = *b"GCR-1541";

/// The only G64 version ever defined.
const VERSION: u8 = 0;

/// Bytes before the half-track offset table.
const HEADER: usize = 12;

/// A speed entry below this is a constant speed for the whole track; at or above
/// it, the entry is a file offset to a per-byte speed block. Neither is read
/// here — see the module header.
const CONSTANT_SPEED: u32 = 4;

/// The 1541 writes a 40-bit sync; the specification's own rule for finding one
/// is "at least 10 or more 1 bits", which is what this reader uses.
const SYNC_BITS: u32 = 10;

/// The first GCR byte of a header block — `$08` encodes to `01010 01001 …`.
const HEADER_LEAD: u8 = 0x52;

/// The first GCR byte of a data block — `$07` encodes to `01010 10111 …`.
const DATA_LEAD: u8 = 0x55;

/// GCR bytes in a header block, decoding to 8.
const HEADER_GCR: usize = 10;

/// Bytes of a decoded header that must be intact: the `$08` id, its checksum,
/// the sector, the track and the two format-id bytes. The two after them are
/// `$0F` off bytes.
const HEADER_MEANS: usize = 6;

/// GCR bytes in a data block, decoding to 260.
const DATA_GCR: usize = 325;

/// Bytes of a decoded data block that must be intact: the `$07` id, 256 bytes of
/// sector and the checksum. The two after them are `$00` off bytes.
const DATA_MEANS: usize = SECTOR + 2;

/// Block ids, from the specification's own breakdown of the two block types.
const HEADER_ID: u8 = 0x08;
const DATA_ID: u8 = 0x07;

/// The 1541's 4-bit → 5-bit GCR table, indexed by nybble.
///
/// VICE `src/gcr.c`'s `GCR_conv_data`, and identical to the binary table in
/// Linus Åkesson's *GCR decoding on the fly*: `0000`→`01010`, `0001`→`01011`,
/// `0010`→`10010`, `0011`→`10011`, `0100`→`01110`, `0101`→`01111`,
/// `0110`→`10110`, `0111`→`10111`, `1000`→`01001`, `1001`→`11001`,
/// `1010`→`11010`, `1011`→`11011`, `1100`→`01101`, `1101`→`11101`,
/// `1110`→`11110`, `1111`→`10101`. No code has more than two zero bits in a row
/// and none is ten ones, which is what keeps a sync mark unambiguous.
pub(crate) const GCR: [u8; 16] = [
    0x0a, 0x0b, 0x12, 0x13, 0x0e, 0x0f, 0x16, 0x17, 0x09, 0x19, 0x1a, 0x1b, 0x0d, 0x1d, 0x1e, 0x15,
];

/// What [`UNGCR`] holds for the sixteen 5-bit codes the table above never emits.
const INVALID: u8 = 0xff;

/// The inverse of [`GCR`], **computed from it** rather than written out. A
/// transcribed inverse is the same fact stated twice, and the second statement
/// is the one nobody checks.
const UNGCR: [u8; 32] = {
    let mut table = [INVALID; 32];
    let mut nybble = 0;
    while nybble < 16 {
        table[GCR[nybble] as usize] = nybble as u8;
        nybble += 1;
    }
    table
};

/// Errors that can arise while mounting a Commodore GCR bitstream image.
#[derive(Debug, PartialEq, Eq)]
pub enum G64Error {
    /// Not a G64 container, or nothing in it decodes to an Infocom press.
    NotAG64,
}

/// A mounted `.g64`, which is a mounted [`D64`] and nothing more.
///
/// Every accessor delegates. The decode happens once, at [`G64::mount`], and
/// what comes out of it is a sector image with no memory of having been a
/// bitstream — so there is no second story-finding path here to drift away from
/// the first.
#[derive(Debug)]
pub struct G64 {
    disk: D64,
}

impl G64 {
    /// Cheap sniff: is this a G64 of an Infocom Commodore release?
    ///
    /// Cheap in the only way that matters for a directory scan — the eight-byte
    /// signature turns away every file in a library but this one. Past that it
    /// costs a whole decode, because [`crate::medium::Volume::looks_like`] must
    /// never claim bytes [`G64::mount`] would then refuse, and what makes these
    /// bytes an Infocom press is a story that verifies. The same bargain
    /// [`D64::looks_like_d64`] strikes, for the same reason.
    pub fn looks_like_g64(raw: &[u8]) -> bool {
        sector_image(raw).is_some_and(|image| D64::looks_like_d64(&image))
    }

    /// Decode the bitstream and open the disk inside it.
    pub fn mount(raw: &[u8]) -> Result<G64, G64Error> {
        let image = sector_image(raw).ok_or(G64Error::NotAG64)?;
        match D64::mount(image) {
            Ok(disk) => Ok(G64 { disk }),
            Err(D64Error::NotAD64) => Err(G64Error::NotAG64),
        }
    }

    /// The name in the BAM, when it reads as one.
    pub fn volume_name(&self) -> Option<&str> {
        self.disk.volume_name()
    }

    /// The story on this disk, named `T5/S0` for where its header sits.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        self.disk.story()
    }

    /// Everything the disk can be shown to hold: the story, and nothing else.
    pub fn contents(&self) -> Vec<(String, Vec<u8>)> {
        self.disk.contents()
    }

    /// One entry by the name a caller was shown, case-insensitively.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        self.disk.read_named(name)
    }

    /// How many entries the mount found.
    pub fn file_count(&self) -> usize {
        self.disk.file_count()
    }

    /// No artwork — Infocom pressed no Version 6 game for the Commodore, so
    /// there is no evidence about where one would keep an archive.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        self.disk.pictures()
    }
}

// ── The container ─────────────────────────────────────────────────────────────

/// A 35-track 1541 sector image decoded out of `raw`, or `None` when `raw` is
/// not a G64 container at all.
///
/// **`Some` is not a claim that anything decoded.** A track whose bitstream is
/// not sectors — the copy protection, on the one specimen here — leaves its
/// sectors zero and the caller carries on; only a container that will not parse
/// is refused. What makes the result a *story* is [`crate::d64`]'s checksum
/// scan, which is the only thing in this crate that can tell a run of sectors
/// from a plausible one.
pub(crate) fn sector_image(raw: &[u8]) -> Option<Vec<u8>> {
    let half_tracks = container(raw)?;
    let mut image = vec![0u8; crate::d64::D64_LEN];
    for track in 1..=TRACKS {
        // Whole tracks only: index 0 is track 1.0, 1 is track 1.5, and a
        // half-track is a protection artefact with no sector on it that a
        // 35-track image has room for.
        let Some(gcr) = half_tracks.get((track - 1) * 2).copied().flatten() else { continue };
        for (sector, bytes) in read_track(gcr, track) {
            let at = linear(track, sector) * SECTOR;
            image[at..at + SECTOR].copy_from_slice(&bytes);
        }
    }
    Some(image)
}

/// The stored bitstream of each declared half-track, `None` where the image has
/// no data for one — or `None` overall when `raw` is not a G64.
fn container(raw: &[u8]) -> Option<Vec<Option<&[u8]>>> {
    if raw.len() < HEADER || raw[..8] != SIGNATURE || raw[8] != VERSION {
        return None;
    }
    let declared = usize::from(raw[9]);
    // The offset table and the speed table are one 4-byte entry per half-track
    // each, back to back, and a file too short to hold both is not one.
    let tables = HEADER + 8 * declared;
    if declared == 0 || raw.len() < tables {
        return None;
    }
    let word = |at: usize| u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
    // Read the speed table for the sole purpose of refusing a file whose speed
    // OFFSETS point outside it — a structural check on the container. Which
    // speed a track was written at is not something this reader needs; see the
    // module header.
    for entry in 0..declared {
        let speed = word(HEADER + 4 * declared + 4 * entry);
        if speed >= CONSTANT_SPEED && speed as usize >= raw.len() {
            return None;
        }
    }
    let mut tracks = Vec::with_capacity(declared);
    for entry in 0..declared {
        let at = word(HEADER + 4 * entry) as usize;
        if at == 0 {
            tracks.push(None);
            continue;
        }
        // A 2-byte LO/HI length, then that many bytes of GCR — both of which
        // have to be inside the file.
        if at + 2 > raw.len() {
            return None;
        }
        let len = usize::from(u16::from_le_bytes([raw[at], raw[at + 1]]));
        if at + 2 + len > raw.len() {
            return None;
        }
        tracks.push(Some(&raw[at + 2..at + 2 + len]));
    }
    Some(tracks)
}

// ── The bitstream ─────────────────────────────────────────────────────────────

/// Bit `at` of `gcr`, MSB first — the order the head reads them in.
fn bit(gcr: &[u8], at: usize) -> u32 {
    u32::from(gcr[at >> 3] >> (7 - (at & 7)) & 1)
}

/// The GCR byte starting at bit `at`, or `None` past the end of the track.
fn gcr_byte(gcr: &[u8], at: usize) -> Option<u8> {
    let end = at + 8;
    if end > gcr.len() * 8 {
        return None;
    }
    Some((at..end).fold(0u8, |byte, i| byte << 1 | bit(gcr, i) as u8))
}

/// The `count` GCR bytes at bit `at`, decoded four data bytes per five.
///
/// `means` is how many of the decoded bytes carry information. An invalid
/// 5-bit code at or beyond it is the block's trailing "off" bytes and is passed
/// over as a zero; before it, the block is refused. See the module header for
/// why that distinction is not a leniency: on real media the write splice lands
/// in exactly those pad nybbles, and the checksum over the meaningful bytes is
/// what actually decides.
fn block(gcr: &[u8], at: usize, count: usize, means: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(count / 5 * 4);
    for group in 0..count / 5 {
        // Five GCR bytes are forty bits and eight 5-bit codes, MSB first.
        let mut bits: u64 = 0;
        for byte in 0..5 {
            bits = bits << 8 | u64::from(gcr_byte(gcr, at + (group * 5 + byte) * 8)?);
        }
        let mut nybbles = [0u8; 8];
        for (which, nybble) in nybbles.iter_mut().enumerate() {
            let code = (bits >> (35 - 5 * which)) & 0x1f;
            *nybble = match UNGCR[code as usize] {
                INVALID if out.len() + which / 2 >= means => 0,
                INVALID => return None,
                value => value,
            };
        }
        for pair in 0..4 {
            out.push(nybbles[2 * pair] << 4 | nybbles[2 * pair + 1]);
        }
    }
    Some(out)
}

/// Every sector of `track` that decodes, as `(sector, bytes)`.
///
/// The specification's own algorithm: find a sync, look at the first GCR byte
/// after it, decode a header block or a data block accordingly, and pair a data
/// block with the header that preceded it. A block that will not decode, whose
/// checksum does not verify, or whose header names a different track than the
/// one it is physically on, is dropped — the last of those because writing a
/// sector into a track it does not claim to be on would be a fabrication, and a
/// header that disagrees with its own track is a protection artefact.
fn read_track(gcr: &[u8], track: usize) -> Vec<(usize, [u8; SECTOR])> {
    let bits = gcr.len() * 8;
    let mut found = Vec::new();
    let mut sector: Option<usize> = None;
    let mut ones = 0;
    for at in 0..bits {
        if bit(gcr, at) == 1 {
            ones += 1;
            continue;
        }
        if ones < SYNC_BITS {
            ones = 0;
            continue;
        }
        ones = 0;
        match gcr_byte(gcr, at) {
            Some(HEADER_LEAD) => {
                sector = block(gcr, at, HEADER_GCR, HEADER_MEANS).filter(|h| {
                    // `$00` id, `$01` checksum, then sector, track and the two
                    // format-id bytes — the specification's own field order,
                    // and the one VICE writes.
                    h[0] == HEADER_ID
                        && h[1] == h[2] ^ h[3] ^ h[4] ^ h[5]
                        && usize::from(h[3]) == track
                        && usize::from(h[2]) < sectors_per_track(track)
                }).map(|h| usize::from(h[2]));
            }
            Some(DATA_LEAD) => {
                let Some(at_sector) = sector.take() else { continue };
                let Some(data) = block(gcr, at, DATA_GCR, DATA_MEANS) else { continue };
                let checksum = data[1..=SECTOR].iter().fold(0u8, |sum, &b| sum ^ b);
                if data[0] != DATA_ID || checksum != data[SECTOR + 1] {
                    continue;
                }
                let mut bytes = [0u8; SECTOR];
                bytes.copy_from_slice(&data[1..=SECTOR]);
                found.push((at_sector, bytes));
            }
            _ => {}
        }
    }
    found
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The gap of `$55` bytes the 1541 leaves between a header block and the
    /// data block that follows it. Nine on every drive since ROM 901225-3.
    const HEADER_GAP: usize = 9;

    /// The gap between one sector and the next sync. Real disks vary from 4 to
    /// 19 bytes; the writer below picks one and the reader never looks at it.
    const TAIL_GAP: usize = 8;

    /// The sync the 1541 writes: 40 `1` bits.
    const SYNC: [u8; 5] = [0xff; 5];

    /// The maximum track size a 1541 G64 declares, from the specification.
    const MAX_TRACK: u16 = 7928;

    /// The format id a synthetic disk is mastered with — `"00"` in PETSCII,
    /// which is what the *Plundered Hearts* press carries.
    const FORMAT_ID: (u8, u8) = (0x30, 0x30);

    /// Four bytes as the five GCR bytes that encode them: eight nybbles through
    /// [`GCR`], packed MSB first. VICE's `gcr_convert_4bytes_to_GCR`.
    fn encode_group(source: &[u8]) -> [u8; 5] {
        let mut bits: u64 = 0;
        for &byte in source {
            bits = bits << 5 | u64::from(GCR[usize::from(byte >> 4)]);
            bits = bits << 5 | u64::from(GCR[usize::from(byte & 0x0f)]);
        }
        let mut out = [0u8; 5];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = (bits >> (32 - 8 * i)) as u8;
        }
        out
    }

    /// A whole block, GCR encoded four bytes at a time.
    fn encode(block: &[u8]) -> Vec<u8> {
        block.chunks(4).flat_map(encode_group).collect()
    }

    /// **A G64 written around an existing 1541 sector image** — the other half
    /// of the reader, kept in the tests because nothing ships a G64 writer.
    ///
    /// It is what makes the round trip below an oracle rather than a second
    /// opinion: the sectors that go in are known, and CI can run it with no
    /// commercial media anywhere near it.
    pub(crate) fn g64_of(image: &[u8]) -> Vec<u8> {
        assert_eq!(image.len(), crate::d64::D64_LEN);
        let half_tracks = TRACKS * 2;
        let mut out = vec![0u8; HEADER + 8 * half_tracks];
        out[..8].copy_from_slice(&SIGNATURE);
        out[8] = VERSION;
        out[9] = half_tracks as u8;
        out[10..12].copy_from_slice(&MAX_TRACK.to_le_bytes());
        for track in 1..=TRACKS {
            let at = (out.len() as u32).to_le_bytes();
            out[HEADER + 4 * (track - 1) * 2..HEADER + 4 * (track - 1) * 2 + 4]
                .copy_from_slice(&at);
            // The four 1541 speed zones, as a constant per track.
            let speed: u32 = match track {
                1..=17 => 3,
                18..=24 => 2,
                25..=30 => 1,
                _ => 0,
            };
            let speed_at = HEADER + 4 * half_tracks + 4 * (track - 1) * 2;
            out[speed_at..speed_at + 4].copy_from_slice(&speed.to_le_bytes());

            let mut gcr = Vec::new();
            for sector in 0..sectors_per_track(track) {
                let (id2, id1) = FORMAT_ID;
                let checksum = sector as u8 ^ track as u8 ^ id2 ^ id1;
                let header =
                    [HEADER_ID, checksum, sector as u8, track as u8, id2, id1, 0x0f, 0x0f];
                gcr.extend_from_slice(&SYNC);
                gcr.extend(encode(&header));
                gcr.extend(std::iter::repeat_n(0x55, HEADER_GAP));

                let at = linear(track, sector) * SECTOR;
                let mut data = Vec::with_capacity(260);
                data.push(DATA_ID);
                data.extend_from_slice(&image[at..at + SECTOR]);
                data.push(image[at..at + SECTOR].iter().fold(0u8, |sum, &b| sum ^ b));
                data.extend_from_slice(&[0, 0]);
                gcr.extend_from_slice(&SYNC);
                gcr.extend(encode(&data));
                gcr.extend(std::iter::repeat_n(0x55, TAIL_GAP));
            }
            assert!(gcr.len() <= usize::from(MAX_TRACK), "track {track} overflows the max");
            out.extend_from_slice(&(gcr.len() as u16).to_le_bytes());
            out.extend_from_slice(&gcr);
            // Filler out to the declared maximum, as a real dump carries. It is
            // `$FF` — sync bits — precisely so that a reader honouring the
            // length prefix and one honouring the maximum size behave
            // differently, which is what the round trip below then pins.
            out.extend(std::iter::repeat_n(0xff, usize::from(MAX_TRACK) - gcr.len()));
        }
        out
    }

    /// A G64 of the synthetic 1541 disk [`crate::d64`] builds — the census's
    /// sample for this format.
    pub(crate) fn sample_disk(story: &[u8]) -> Vec<u8> {
        g64_of(&crate::d64::tests::sample_disk(story))
    }

    /// A Version 3 story whose header checksum is correct for its own bytes.
    fn fake_story(len: usize) -> Vec<u8> {
        let mut story = vec![0u8; len];
        story[0] = 3;
        let mut word = |o: usize, v: u16| story[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400);
        word(0x06, 0x0500);
        word(0x08, 0x0300);
        word(0x0a, 0x0100);
        word(0x0c, 0x0200);
        word(0x0e, 0x0280);
        word(0x1a, (len / 2) as u16);
        story[0x12..0x18].copy_from_slice(b"870730");
        for (i, byte) in story.iter_mut().enumerate().skip(64) {
            *byte = (i % 251) as u8;
        }
        let sum = story[64..].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
        story[0x1c..0x1e].copy_from_slice(&sum.to_be_bytes());
        story
    }

    /// Where a track's stored bitstream begins in a G64 built above.
    fn track_at(raw: &[u8], track: usize) -> usize {
        let at = HEADER + 4 * (track - 1) * 2;
        u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]) as usize + 2
    }

    // ── The encoding, against the specification ──────────────────────────────

    /// [`GCR`] is the table VICE's `gcr.c` and Åkesson's article both print,
    /// stated here in the binary the article gives so a transposed hex digit
    /// cannot hide, and [`UNGCR`] is exactly its inverse.
    ///
    /// The properties below are the ones the encoding exists for, and they are
    /// asserted rather than recited: no code has three zero bits in a row, so
    /// the drive's clock recovery never loses the beat, and no run of ones
    /// reaches [`SYNC_BITS`] however the codes are ordered — which is what makes
    /// a sync mark unambiguous and this reader's scan possible at all.
    #[test]
    fn the_gcr_table_is_the_one_the_specifications_print() {
        let from_the_article = [
            0b01010, 0b01011, 0b10010, 0b10011, 0b01110, 0b01111, 0b10110, 0b10111, 0b01001,
            0b11001, 0b11010, 0b11011, 0b01101, 0b11101, 0b11110, 0b10101,
        ];
        assert_eq!(GCR, from_the_article, "the 4-bit to 5-bit table");
        for (nybble, &code) in GCR.iter().enumerate() {
            assert_eq!(UNGCR[code as usize], nybble as u8, "code {code:05b}");
        }
        assert_eq!(UNGCR.iter().filter(|&&v| v != INVALID).count(), 16, "sixteen codes are used");

        // Two codes back to back is the worst case for either run, since a
        // sequence is only ever codes.
        for &a in &GCR {
            for &b in &GCR {
                let pair = u32::from(a) << 5 | u32::from(b);
                let (mut zeros, mut ones, mut worst_zeros, mut worst_ones) = (0, 0, 0, 0);
                for bit in (0..10).rev() {
                    if pair >> bit & 1 == 1 {
                        ones += 1;
                        zeros = 0;
                    } else {
                        zeros += 1;
                        ones = 0;
                    }
                    worst_zeros = worst_zeros.max(zeros);
                    worst_ones = worst_ones.max(ones);
                }
                assert!(worst_zeros <= 2, "{a:05b}{b:05b} has {worst_zeros} zeros in a row");
                assert!(
                    worst_ones < SYNC_BITS,
                    "{a:05b}{b:05b} would read as a sync mark"
                );
            }
        }
    }

    /// The bitstream a 1541 writes for one sector, decoded back to the bytes it
    /// was made from — the round trip, at the smallest scale it exists.
    #[test]
    fn five_gcr_bytes_are_four_data_bytes_both_ways() {
        for value in 0..=u32::MAX / 0x10001 {
            let source = value.to_be_bytes();
            let gcr = encode_group(&source);
            let back = block(&gcr, 0, 5, 4).expect("it decodes");
            assert_eq!(back, source, "{source:02x?}");
        }
        // The specification's own worked example: `$08` leads a header block
        // with GCR `$52`, and `$07` leads a data block with GCR `$55`.
        assert_eq!(encode_group(&[HEADER_ID, 0, 0, 0])[0], HEADER_LEAD);
        assert_eq!(encode_group(&[DATA_ID, 0, 0, 0])[0], DATA_LEAD);
    }

    // ── The container and the whole disk ─────────────────────────────────────

    /// **The round trip that needs no commercial media**, and the reason this
    /// module can be trusted at all: a known sector image is GCR-encoded into a
    /// G64 and decoded back, and every one of the 683 sectors comes out
    /// identical.
    #[test]
    fn a_synthetic_bitstream_decodes_to_the_sectors_it_was_written_from() {
        let image = crate::d64::tests::sample_disk(&fake_story(4096));
        let raw = g64_of(&image);
        assert_eq!(&raw[..8], b"GCR-1541");
        assert_eq!(sector_image(&raw).as_deref(), Some(image.as_slice()), "683 sectors, exactly");
    }

    /// …and the disk inside it mounts and hands back its story, through the same
    /// [`crate::d64`] the sector dump goes through.
    #[test]
    fn a_synthetic_g64_mounts_and_hands_back_its_story() {
        let story = fake_story(4096);
        let raw = g64_of(&crate::d64::tests::sample_disk(&story));
        assert!(G64::looks_like_g64(&raw));
        let disk = G64::mount(&raw).expect("it mounts");
        assert_eq!(disk.volume_name(), Some("SAMPLE"));
        assert_eq!(disk.file_count(), 1);
        assert_eq!(disk.story().expect("a story"), ("T3/S0".to_string(), story.clone()));
        assert_eq!(disk.read_named("t3/s0"), Some(story), "case-insensitively, like every format");
        assert_eq!(disk.pictures().map(|(name, _)| name), None, "no Commodore v6 press exists");
    }

    /// The length prefix is what bounds a track, not the maximum track size in
    /// the file header.
    ///
    /// [`g64_of`] pads every track out to the declared maximum with `$FF`, which
    /// is forty sync bits per five bytes — so a reader that took the maximum as
    /// the track length would run its scan through hundreds of bytes of sync and
    /// find blocks that are not there. The test above only passes because this
    /// one is true, and this states it directly.
    #[test]
    fn a_track_ends_where_its_length_prefix_says() {
        let raw = g64_of(&crate::d64::tests::sample_disk(&fake_story(4096)));
        let at = track_at(&raw, 1) - 2;
        let stored = usize::from(u16::from_le_bytes([raw[at], raw[at + 1]]));
        assert!(stored < usize::from(MAX_TRACK), "the sample really is padded");
        assert!(raw[at + 2 + stored..at + 2 + usize::from(MAX_TRACK)].iter().all(|&b| b == 0xff));
    }

    /// **The behaviour the scope decision asks for** (SQ-1095): a track that is
    /// not sectors costs that track and nothing else.
    ///
    /// Copy protection on these disks lives in the loader, which lanthorn never
    /// executes, so a track full of something that is not a standard GCR sector
    /// layout is not a failure — it is skipped. *Plundered Hearts* has six such
    /// tracks; here the same shape is built deliberately, and the non-vacuity
    /// guard is the second assertion: the ruined track really did decode to
    /// nothing, so the story below was found in spite of it and not beside it.
    #[test]
    fn a_track_that_will_not_decode_costs_only_that_track() {
        let story = fake_story(4096);
        let mut raw = g64_of(&crate::d64::tests::sample_disk(&story));
        let at = track_at(&raw, 20);
        let len = usize::from(u16::from_le_bytes([raw[at - 2], raw[at - 1]]));
        // Not noise, which might decode by accident: a track of `$55` has no
        // sync mark anywhere in it, which is the one thing this scan needs.
        raw[at..at + len].fill(0x55);

        let image = sector_image(&raw).expect("the container still parses");
        let ruined = linear(20, 0) * SECTOR;
        assert!(
            image[ruined..ruined + SECTOR * sectors_per_track(20)].iter().all(|&b| b == 0),
            "the whole of track 20 is missing, which is what makes this test mean anything"
        );
        assert_eq!(
            G64::mount(&raw).expect("it still mounts").story(),
            Some(("T3/S0".to_string(), story)),
            "the story is on tracks 3 and 4 and is untouched"
        );
    }

    /// FALSIFICATION, and the one this feature could most easily ship broken: a
    /// decoder whose checksums are computed and not compared.
    ///
    /// One nybble of one sector's data block is changed to a different valid GCR
    /// code — so the block still *decodes*, cleanly, into 256 plausible bytes —
    /// and the sector must be dropped on its own checksum. Remove the
    /// `checksum != data[SECTOR + 1]` clause in [`read_track`] and this fails
    /// with a story that is one byte wrong and looks entirely fine.
    #[test]
    fn a_flipped_gcr_nybble_is_caught_by_the_sectors_own_checksum() {
        let story = fake_story(4096);
        let clean = g64_of(&crate::d64::tests::sample_disk(&story));
        let mut raw = clean.clone();
        // Track 3 sector 0 is where the story's header sits. Its data block
        // follows the header block and the gap that separates them.
        let at = track_at(&raw, 3) + SYNC.len() + HEADER_GCR + HEADER_GAP + SYNC.len();
        // Decode the block's first group, change one nybble of one data byte to
        // a different VALID code, and write it back — so what lands on the disk
        // is a block that decodes perfectly and is one byte away from the
        // checksum it carries.
        assert_eq!(raw[at], DATA_LEAD);
        let mut group = block(&raw[at..], 0, 5, 4).expect("the first group decodes");
        assert_eq!(group[0], DATA_ID, "…and it is the data block");
        group[1] ^= 0x01;
        raw[at..at + 5].copy_from_slice(&encode_group(&group));
        assert!(
            block(&raw[at..], 0, DATA_GCR, DATA_MEANS).is_some(),
            "the damaged block still decodes cleanly, into 256 plausible bytes"
        );

        let image = sector_image(&raw).expect("the container still parses");
        let dropped = linear(3, 0) * SECTOR;
        assert!(
            image[dropped..dropped + SECTOR].iter().all(|&b| b == 0),
            "the sector is refused, not repaired and not passed on"
        );
        assert_eq!(G64::mount(&raw).map(|d| d.story()), Err(G64Error::NotAG64), "and no story");
        assert!(G64::mount(&clean).is_ok(), "…which is not how it behaves undamaged");
    }

    /// The one place a block is allowed to be broken: its trailing "off" bytes.
    ///
    /// A data block ends in two `$00` bytes that exist only to make it a
    /// multiple of five, and a drive's write splice lands in exactly those
    /// nybbles — six sectors of the *Plundered Hearts* press have a corrupt
    /// final GCR byte and are otherwise perfect. So the last GCR byte here is
    /// made invalid and the sector must still come back, while the same damage
    /// one group earlier — which reaches the checksum byte — must not.
    #[test]
    fn a_broken_off_byte_at_the_end_of_a_block_is_not_a_broken_sector() {
        let story = fake_story(4096);
        let image = crate::d64::tests::sample_disk(&story);
        let start = track_at(&image_g64(&image), 3);
        let data = start + SYNC.len() + HEADER_GCR + HEADER_GAP + SYNC.len();

        let mut splice = image_g64(&image);
        splice[data + DATA_GCR - 1] = 0x00;
        assert!(
            block(&splice[data..], 0, DATA_GCR, DATA_MEANS).is_some(),
            "an invalid code in the pad is passed over"
        );
        assert_eq!(
            G64::mount(&splice).expect("it mounts").story(),
            Some(("T3/S0".to_string(), story)),
            "and the sector is whole"
        );

        let mut deeper = image_g64(&image);
        deeper[data + DATA_GCR - 6] = 0x00;
        assert!(
            block(&deeper[data..], 0, DATA_GCR, DATA_MEANS).is_none(),
            "the same damage over the checksum byte refuses the block"
        );
    }

    /// [`g64_of`] under another name, so the test above can build three copies
    /// without shadowing itself.
    fn image_g64(image: &[u8]) -> Vec<u8> {
        g64_of(image)
    }

    /// A container that is not one, said in each of the ways it can fail.
    #[test]
    fn an_alien_or_truncated_container_is_refused_rather_than_guessed_at() {
        let good = g64_of(&crate::d64::tests::sample_disk(&fake_story(4096)));
        assert!(sector_image(&good).is_some());

        let mut wrong_magic = good.clone();
        wrong_magic[..8].copy_from_slice(b"GCR-1571");
        assert_eq!(sector_image(&wrong_magic), None, "another drive's bitstream");

        let mut wrong_version = good.clone();
        wrong_version[8] = 1;
        assert_eq!(sector_image(&wrong_version), None, "a version we have never seen");

        let mut no_tracks = good.clone();
        no_tracks[9] = 0;
        assert_eq!(sector_image(&no_tracks), None, "no half-tracks declared");

        // An offset table that runs past the end of the file.
        assert_eq!(sector_image(&good[..HEADER + 8]), None, "truncated before its tables");

        let mut off_the_end = good.clone();
        off_the_end[HEADER..HEADER + 4].copy_from_slice(&(good.len() as u32 + 1).to_le_bytes());
        assert_eq!(sector_image(&off_the_end), None, "a track outside the file");

        // The LAST track, because every other one has thirty-four tracks of
        // bitstream behind it to overrun into.
        let mut long_track = good.clone();
        let at = track_at(&long_track, TRACKS) - 2;
        long_track[at..at + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(sector_image(&long_track), None, "a track longer than the file holds");

        assert_eq!(sector_image(b""), None);
        assert_eq!(sector_image(&vec![0u8; crate::d64::D64_LEN]), None, "a `.d64` is not a `.g64`");
    }

    /// A side of a multi-disk Commodore release reaches `crate::d64`'s
    /// `story_across` in whichever container it was dumped in, and
    /// the two answer alike.
    ///
    /// No G64 set is in the corpus, so what this pins is the seam rather than a
    /// press: a `.g64` side is decoded to sectors *before* the reassembly scan,
    /// which is the one thing that could silently not happen — the filter used
    /// to be `len() == D64_LEN`, which drops a bitstream without a word.
    #[test]
    fn a_bitstream_side_reaches_the_multi_disk_scan_as_sectors() {
        let story = fake_story(4096);
        let sectors = crate::d64::tests::sample_disk(&story);
        let bitstream = g64_of(&sectors);
        let expected = Some(("T3/S0".to_string(), story));
        assert_eq!(
            crate::d64::story_across(&[sectors.clone(), sectors]),
            expected,
            "two sector dumps"
        );
        assert_eq!(
            crate::d64::story_across(&[bitstream.clone(), bitstream]),
            expected,
            "…and two bitstream dumps of the same disk"
        );
    }

    // ── Real media ───────────────────────────────────────────────────────────

    /// `stories/` is gitignored, so every case below skips vacuously in CI.
    fn fixture(name: &str) -> Option<Vec<u8>> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
        let path = dir.join(name);
        std::fs::read(&path)
            .inspect_err(|_| eprintln!("SKIP: gitignored medium missing at {}", path.display()))
            .ok()
    }

    /// The specimen, structurally — what a real nibble of a 1987 Infocom disk
    /// actually contains, so a later G64 that differs is a fact and not a
    /// surprise.
    #[test]
    fn the_plundered_hearts_bitstream_is_forty_whole_tracks_and_no_half_ones() {
        let Some(raw) = fixture("plundered_hearts[infocom_1987](r26)(!).g64") else { return };
        assert_eq!(raw.len(), 317_884);
        assert_eq!(&raw[..8], b"GCR-1541");
        assert_eq!(raw[8], VERSION);
        assert_eq!(raw[9], 84, "84 half-tracks declared, as the specification's own dump has");
        assert_eq!(u16::from_le_bytes([raw[10], raw[11]]), MAX_TRACK);

        let tracks = container(&raw).expect("it parses");
        let with_data: Vec<usize> = (0..tracks.len()).filter(|&i| tracks[i].is_some()).collect();
        assert_eq!(with_data.len(), 40, "an extended forty-track disk");
        assert!(with_data.iter().all(|i| i % 2 == 0), "no half-track: no track-level protection");
        assert_eq!(with_data.last(), Some(&78), "track 40 is the last with data");
        assert_eq!(tracks[0].expect("track 1").len(), 7692, "the ordinary speed-zone-3 length");

        // What decodes and what does not, measured. Tracks 36-40 are the
        // protection and hold no standard sector at all; track 24 sector 18 is
        // one block the nibbler could not read, and is outside the story.
        //
        // Counted by asking the decoder, not by looking for non-zero bytes in
        // the assembled image: three sectors of this press are legitimately all
        // zero, and "it decoded" and "it has something in it" are different
        // questions.
        let decoded: usize =
            (1..=TRACKS).map(|t| read_track(tracks[(t - 1) * 2].expect("stored"), t).len()).sum();
        assert_eq!(decoded, 682, "682 of the 683 sectors a 35-track 1541 has");
        assert!(
            read_track(tracks[23 * 2].expect("track 24"), 24).iter().all(|&(s, _)| s != 18),
            "and the one that does not is track 24 sector 18, outside the story"
        );
        for track in 36..=40 {
            let gcr = tracks[(track - 1) * 2].expect("it is stored");
            assert!(read_track(gcr, track).is_empty(), "track {track} is not sectors");
        }
    }

    /// **The oracle.** What comes off the bitstream must be the release the
    /// corpus already holds as a bare story file, byte for byte — a GCR decode
    /// that is subtly wrong produces plausible bytes, and nothing else here can
    /// tell the difference.
    #[test]
    fn plundered_hearts_off_the_bitstream_is_the_release_the_corpus_already_has() {
        let Some(raw) = fixture("plundered_hearts[infocom_1987](r26)(!).g64") else { return };
        let Some(reference) = fixture("plunderedhearts-r26-s870730.z3") else { return };

        assert!(G64::looks_like_g64(&raw));
        let disk = G64::mount(&raw).expect("the bitstream mounts");
        assert_eq!(disk.volume_name(), Some("PLUNDERED HEARTS"));
        let (name, story) = disk.story().expect("a story");
        assert_eq!(name, "T5/S0", "where the header sits, as this medium names things");

        assert_eq!(story.len(), 128_962);
        assert_eq!(story[0], 3, "Version 3");
        assert_eq!(u16::from_be_bytes([story[2], story[3]]), 26, "release 26");
        assert_eq!(&story[0x12..0x18], b"870730", "serial 870730");
        assert_eq!(story, reference, "byte-identical to plunderedhearts-r26-s870730.z3");
    }

    /// The layout the 1987 press spends its sectors in — the third in three
    /// presses, and the one `crate::d64`'s third `Plan` was added for.
    ///
    /// Stated as a property of the *story* rather than of the plan, so it would
    /// still fail if the plan were changed to something that happened to
    /// reassemble: every 256-byte block of the release, in order, is read back
    /// out of the sector this claims it is on.
    #[test]
    fn the_1987_press_spends_seventeen_sectors_a_track() {
        let Some(raw) = fixture("plundered_hearts[infocom_1987](r26)(!).g64") else { return };
        let Some(reference) = fixture("plunderedhearts-r26-s870730.z3") else { return };
        let image = sector_image(&raw).expect("it decodes");

        let mut plan = Vec::new();
        for track in 5..=TRACKS {
            if track == 17 {
                continue;
            }
            // Track 18's first two sectors are the BAM and the one directory
            // sector behind it; the press starts after them and still takes
            // seventeen.
            let base = if track == 18 { 2 } else { 0 };
            plan.extend((base..base + 17).map(|sector| linear(track, sector)));
        }
        let assembled: Vec<u8> = plan
            .iter()
            .flat_map(|&i| image[i * SECTOR..(i + 1) * SECTOR].iter().copied())
            .take(reference.len())
            .collect();
        assert_eq!(assembled, reference, "T5/S0 .. T16/S16, T18/S2 .. T18/S18, T19/S0 .. T35/S9");
        assert_eq!(plan.len(), 510, "thirty tracks of seventeen");
        assert_eq!(reference.len().div_ceil(SECTOR), 504, "…six more than the story spends");
    }
}
