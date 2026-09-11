//! Zero-dependency parser for the IFF "Blorb" interactive-fiction resource
//! container. Exposes the embedded executable and a generic resource accessor.

pub mod adf;
pub mod amiga_font;
pub mod bitmap_font;
pub mod mac_font;
pub mod resource_fork;
pub mod bpal;
/// The container a disc's filesystem sits inside — raw CD sectors and the Apple
/// Partition Map. Private because it is a wrapper rather than a reader: what it
/// finds is handed to [`hfs`], and nothing outside this crate asks for it.
mod cd;
pub mod atr;
pub mod d64;
pub mod dos33;
pub mod dos_order;
pub mod fat12;
pub mod g64;
pub mod hfs;
pub mod infocom_boot;
pub mod infocom_packed;
pub mod infocom_pics;
pub mod infocom_sound;
pub mod iso9660;
pub mod medium;
pub mod prodos;
pub mod xex;

/// Errors that can arise while parsing a Blorb container.
#[derive(Debug, PartialEq, Eq)]
pub enum BlorbError {
    /// The bytes are not a Blorb (no `FORM`…`IFRS` magic).
    NotBlorb,
    /// A length or offset ran past the end of the data.
    Truncated,
    /// No `RIdx` resource-index chunk was found.
    NoResourceIndex,
    /// A resource-index entry points at an offset outside the file.
    BadOffset,
    /// No usable `Exec` executable chunk was found.
    NoExecutable,
}

/// One parsed resource-index entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceEntry {
    /// Usage tag (e.g. `b"Exec"`, `b"Pict"`, `b"Snd "`, `b"Data"`).
    pub usage: [u8; 4],
    /// Resource number within its usage class.
    pub number: u32,
    /// File offset of the resource's chunk header.
    pub start: usize,
    /// The 4-byte chunk type at `start`.
    pub chunk_type: [u8; 4],
    /// Chunk data length (excludes the 8-byte header and any pad byte).
    pub len: usize,
}

/// The kind of executable embedded in a Blorb's `Exec` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecKind {
    /// Z-machine story (`ZCOD` chunk).
    ZCode,
    /// Glulx story (`GLUL` chunk).
    Glulx,
    /// Scott Adams / ScottFree adventure (`SAAI` chunk).
    Scott,
}

/// The kind of a Blorb `Snd ` sound resource, detected from its chunk type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    /// AIFF sampled sound (`FORM` chunk).
    Aiff,
    /// Ogg Vorbis sampled sound (`OGGV` chunk).
    Ogg,
    /// Amiga ProTracker module (`MOD ` chunk).
    Mod,
    /// A sound resource whose chunk type we do not decode.
    Other,
}

/// Which build of a story a Blorb's resources were made for — the Blorb spec's
/// optional `IFhd` **Game Identifier** chunk (SQ-0866).
///
/// The spec defers the layout: *"For Z-code, the contents of the game identifier
/// chunk are defined in the common save file format specification, section 5."*
/// Quetzal §5.4 spells that out as release number (story offset `$02`, 2 bytes),
/// serial number (`$12`, 6 bytes), checksum (`$1C`, 2 bytes) and a 3-byte initial
/// PC — 13 bytes in all. Blorb adds that the PC *"has no meaning for resource
/// files. It should be set to zero"*, so it is not kept here.
///
/// # What it is for
///
/// The spec states the use outright: *"If it is present, and the interpreter is
/// given a game file along with a resource file, the interpreter can check that
/// the game matches the IFhd chunk. If they don't, the interpreter should display
/// an error."* [`crate::Blorb::game_identifier`] is that check's left-hand side;
/// the policy about what to do with a mismatch is `app`'s, in
/// `app::graphics::resource_blorb`, because it turns on how the two files came to
/// be considered together in the first place.
///
/// The chunk is **optional**, and most of the corpus omits it — every modern
/// `.zblorb`, `advent.blb`, `Sherlock.blb`, all eleven Mysterious Adventures
/// sidecars. An absent identifier is not a mismatch; it is a Blorb declining to
/// say, and callers must treat the two differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameIdentifier {
    /// Release number, story header `$02`.
    pub release: u16,
    /// Serial number, story header `$12` — six ASCII digits, conventionally
    /// `YYMMDD`. Normalised: see [`GameIdentifier::of_story`].
    pub serial: [u8; 6],
    /// Checksum, story header `$1C`.
    pub checksum: u16,
}

impl GameIdentifier {
    /// The identifier a Z-machine story file's own header states, or `None` when
    /// the bytes are too short to carry one.
    ///
    /// # Why the serial is masked
    ///
    /// Bit 7 is stripped from each serial byte. Apple II releases store their
    /// header text in high ASCII, so the *same* serial reads as `890622` off a
    /// `.z6` and as `b8b9b0b6b2b2` off the platter — a difference in character
    /// encoding, not in identity. `cli_host::storage::DiskBuild::of` already
    /// masks for exactly this reason (SQ-0856), and a comparison that did not
    /// would report every Apple II story as a different build from itself.
    ///
    /// Applied on both sides, including a Blorb's own `IFhd`, so the two can
    /// never be normalised differently. No Blorb in the corpus writes high ASCII,
    /// which is precisely why masking it costs nothing.
    pub fn of_story(story: &[u8]) -> Option<GameIdentifier> {
        let s = story.get(..0x1e)?;
        let mut serial = [0u8; 6];
        for (dst, &src) in serial.iter_mut().zip(&s[0x12..0x18]) {
            *dst = src & 0x7f;
        }
        Some(GameIdentifier {
            release: u16::from_be_bytes([s[0x02], s[0x03]]),
            serial,
            checksum: u16::from_be_bytes([s[0x1c], s[0x1d]]),
        })
    }

    /// The serial as text, for a message a person reads.
    pub fn serial_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.serial)
    }
}

impl std::fmt::Display for GameIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "release {}, serial {}", self.release, self.serial_str())
    }
}

/// A parsed Blorb container: owns the file bytes and the resource index.
#[derive(Debug)]
pub struct Blorb {
    bytes: Vec<u8>,
    index: Vec<ResourceEntry>,
    /// Frontispiece (cover) Pict resource number from the top-level `Fspc` chunk.
    fspc: Option<u32>,
    /// `(start, len)` of the top-level `IFmd` chunk's data, if present.
    ifmd: Option<(usize, usize)>,
    /// Which build these resources are for, from the top-level `IFhd` chunk.
    /// `None` for the many containers that carry none — see [`GameIdentifier`].
    ifhd: Option<GameIdentifier>,
    /// Standard window `(width, height)` in pixels from the top-level `Reso`
    /// (resolution) chunk — the display size the pictures were authored for.
    /// A v6 interpreter advertises this as the screen size so the game's
    /// hardcoded pixel layout lines up with its art (e.g. Zork0 → 320×200).
    reso_std: Option<(u16, u16)>,
    /// Picture resource numbers listed in the top-level `APal` (adaptive
    /// palette) chunk — Blorb spec §11.3. Each such picture carries a
    /// PLACEHOLDER palette; the interpreter must plot it with the "Current
    /// Palette" (the palette of the most recently drawn non-adaptive picture)
    /// rather than its own. Empty when the container has no `APal` chunk.
    apal: Vec<u32>,
}

/// The identifier an `IFhd` chunk BODY states — Quetzal §5.4's packed layout
/// (release, serial, checksum, then the initial PC), which is not the story
/// header's spacing, so this cannot share [`GameIdentifier::of_story`].
///
/// Ten bytes are enough: the trailing PC is the one field Blorb declares
/// meaningless in a resource file, so a writer that stops short of it has still
/// said everything that identifies the build.
fn ifhd_identifier(body: &[u8]) -> Option<GameIdentifier> {
    let b = body.get(..10)?;
    let mut serial = [0u8; 6];
    for (dst, &src) in serial.iter_mut().zip(&b[2..8]) {
        *dst = src & 0x7f;
    }
    Some(GameIdentifier {
        release: u16::from_be_bytes([b[0], b[1]]),
        serial,
        checksum: u16::from_be_bytes([b[8], b[9]]),
    })
}

fn be_u32(b: &[u8], off: usize) -> Result<u32, BlorbError> {
    let s = b
        .get(off..off + 4)
        .ok_or(BlorbError::Truncated)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

impl Blorb {
    /// Cheap magic check: `bytes[0..4] == b"FORM" && bytes[8..12] == b"IFRS"`.
    ///
    /// Returns `false` (rather than parsing) so callers can fall back to
    /// treating the bytes as a raw story file.
    pub fn is_blorb(b: &[u8]) -> bool {
        b.len() >= 12 && &b[0..4] == b"FORM" && &b[8..12] == b"IFRS"
    }

    /// Parse the bytes as a Blorb, eagerly building the resource index.
    ///
    /// Returns [`BlorbError::NotBlorb`] when the data isn't a Blorb, and other
    /// [`BlorbError`] variants for malformed/truncated containers. All offsets
    /// and lengths are bounds-checked, so malformed input never panics.
    pub fn parse(bytes: Vec<u8>) -> Result<Blorb, BlorbError> {
        if !Self::is_blorb(&bytes) {
            return Err(BlorbError::NotBlorb);
        }
        let end = bytes.len();
        // Walk top-level chunks (start at 12, after FORM+len+IFRS) to find RIdx.
        let mut ridx: Option<(usize, usize, usize)> = None; // (entries_start, count, chunk_len)
        let mut fspc: Option<u32> = None;
        let mut ifmd: Option<(usize, usize)> = None;
        let mut ifhd: Option<GameIdentifier> = None;
        let mut reso_std: Option<(u16, u16)> = None;
        let mut apal: Vec<u32> = Vec::new();
        let mut pos = 12;
        while pos + 8 <= end {
            let ctype = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
            let clen = be_u32(&bytes, pos + 4)? as usize;
            let data_start = pos + 8;
            if data_start + clen > end {
                break;
            }
            if &ctype == b"RIdx" {
                let count = be_u32(&bytes, data_start)? as usize;
                ridx = Some((data_start + 4, count, clen));
            } else if &ctype == b"Fspc" && clen >= 4 {
                fspc = be_u32(&bytes, data_start).ok();
            } else if &ctype == b"IFmd" {
                ifmd = Some((data_start, clen));
            } else if &ctype == b"IFhd" {
                ifhd = ifhd_identifier(&bytes[data_start..data_start + clen]);
            } else if &ctype == b"Reso" && clen >= 8 {
                // First two words are the standard window width/height (px);
                // min/max and per-image ratios follow (ignored here).
                let w = be_u32(&bytes, data_start).unwrap_or(0);
                let h = be_u32(&bytes, data_start + 4).unwrap_or(0);
                if w > 0 && h > 0 {
                    reso_std = Some((w.min(u16::MAX as u32) as u16, h.min(u16::MAX as u32) as u16));
                }
            } else if &ctype == b"APal" {
                // Blorb spec §11.3 (Adaptive Palettes): the body is `num*4`
                // bytes, each a big-endian picture resource number. Every listed
                // picture's own palette is a placeholder to be ignored at plot
                // time in favour of the Current Palette. A malformed length (not
                // a multiple of 4) degrades to "no adaptive set" — a corrupt
                // chunk must never fail the whole load. (`BPal`, seen beside
                // `APal` in Infocom's own blorbs, is a converter extension NOT
                // in the Blorb spec and is ignored here.)
                if clen.is_multiple_of(4) {
                    apal = (0..clen / 4)
                        .filter_map(|i| be_u32(&bytes, data_start + i * 4).ok())
                        .collect();
                }
            }
            pos = data_start + clen + (clen & 1);
        }
        let (mut p, count, ridx_len) = ridx.ok_or(BlorbError::NoResourceIndex)?;
        // The declared entry count is untrusted (a 0xFFFFFFFF count would ask
        // `with_capacity` for ~170GB). Each entry is 12 bytes and must lie
        // INSIDE the RIdx chunk itself — bounding reads only against EOF would
        // let a malformed count walk into the chunks that follow (SQ-0629).
        let need = count
            .checked_mul(12)
            .and_then(|n| n.checked_add(4))
            .ok_or(BlorbError::Truncated)?;
        if ridx_len < need {
            return Err(BlorbError::Truncated);
        }
        let mut index = Vec::with_capacity(count);
        for _ in 0..count {
            let u = bytes.get(p..p + 4).ok_or(BlorbError::Truncated)?;
            let usage = [u[0], u[1], u[2], u[3]];
            let number = be_u32(&bytes, p + 4)?;
            let start = be_u32(&bytes, p + 8)? as usize;
            // Read the pointed-at chunk header.
            let t = bytes.get(start..start + 4).ok_or(BlorbError::BadOffset)?;
            let chunk_type = [t[0], t[1], t[2], t[3]];
            let len = be_u32(&bytes, start + 4).map_err(|_| BlorbError::BadOffset)? as usize;
            if start + 8 + len > end {
                return Err(BlorbError::BadOffset);
            }
            index.push(ResourceEntry { usage, number, start, chunk_type, len });
            p += 12;
        }
        Ok(Blorb { bytes, index, fspc, ifmd, ifhd, reso_std, apal })
    }

    /// The parsed resource index (for enumeration).
    pub fn resources(&self) -> &[ResourceEntry] {
        &self.index
    }

    /// The frontispiece (cover) Pict resource number, if the blorb declares one
    /// via a top-level `Fspc` chunk. Combine with [`Blorb::resource`] to fetch
    /// the image bytes: `blorb.resource(b"Pict", blorb.frontispiece()?)`.
    pub fn frontispiece(&self) -> Option<u32> {
        self.fspc
    }

    /// The raw `IFmd` metadata chunk body (a Treaty of Babel `ifindex` XML
    /// document), if the blorb declares one via a top-level `IFmd` chunk.
    /// Returned uninterpreted — parsing is a caller's concern.
    pub fn metadata(&self) -> Option<&[u8]> {
        self.ifmd.map(|(s, l)| &self.bytes[s..s + l])
    }

    /// Which build of a story these resources were made for, from the optional
    /// top-level `IFhd` chunk — `None` when the container does not say.
    ///
    /// See [`GameIdentifier`] for the layout, the spec's own instruction to
    /// check it, and why "does not say" must never be read as "does not match".
    pub fn game_identifier(&self) -> Option<GameIdentifier> {
        self.ifhd
    }

    /// The standard window `(width, height)` in pixels from the `Reso`
    /// (resolution) chunk — the display size the pictures were designed for.
    /// A v6 story advertises this to the game as its screen size so hardcoded
    /// pixel art (e.g. Zork0's 320×200 frame) lines up. `None` when the blorb
    /// carries no `Reso` chunk.
    pub fn std_window(&self) -> Option<(u16, u16)> {
        self.reso_std
    }

    /// The picture resource numbers declared adaptive-palette by the top-level
    /// `APal` chunk (Blorb spec §11.3). These pictures carry a placeholder
    /// palette; an interpreter must plot them with the Current Palette (the
    /// palette of the most recently drawn non-adaptive picture) instead of
    /// their own. Empty when the container has no `APal` chunk.
    pub fn adaptive_pictures(&self) -> &[u32] {
        &self.apal
    }

    /// Whether Pict resource `number` is listed in the `APal` chunk (Blorb spec
    /// §11.3) — i.e. its palette is a placeholder to be replaced at plot time.
    pub fn is_adaptive_picture(&self, number: u32) -> bool {
        self.apal.contains(&number)
    }

    fn chunk_data(&self, e: &ResourceEntry) -> &[u8] {
        // A `FORM` resource (a nested IFF FORM — e.g. an AIFF sound, or a `Data`
        // FORM resource) keeps its 8-byte `FORM`+length header: per the Blorb
        // spec the header is part of the resource data. Every other chunk type
        // yields just its payload (the bytes after the 8-byte chunk header).
        if &e.chunk_type == b"FORM" {
            &self.bytes[e.start..e.start + 8 + e.len]
        } else {
            &self.bytes[e.start + 8..e.start + 8 + e.len]
        }
    }

    /// Public counterpart of `Blorb::chunk_data`: the raw data bytes for
    /// resource `e` (post 8-byte chunk header, pre pad byte — but a `FORM`
    /// resource retains its `FORM`+length header). For callers that need to
    /// inspect a resource's raw bytes (e.g. format-detail parsing).
    pub fn resource_data(&self, e: &ResourceEntry) -> &[u8] {
        self.chunk_data(e)
    }

    /// The embedded executable: its [`ExecKind`] plus a slice of its chunk data.
    ///
    /// Finds the `Exec` resource, mapping a `ZCOD` chunk to [`ExecKind::ZCode`],
    /// `GLUL` to [`ExecKind::Glulx`], and `SAAI` to [`ExecKind::Scott`]. Returns
    /// [`BlorbError::NoExecutable`] when there is no `Exec` entry or its chunk
    /// type is none of those.
    pub fn executable(&self) -> Result<(ExecKind, &[u8]), BlorbError> {
        let e = self
            .index
            .iter()
            .find(|r| &r.usage == b"Exec")
            .ok_or(BlorbError::NoExecutable)?;
        let kind = match &e.chunk_type {
            b"ZCOD" => ExecKind::ZCode,
            b"GLUL" => ExecKind::Glulx,
            b"SAAI" => ExecKind::Scott,
            _ => return Err(BlorbError::NoExecutable),
        };
        Ok((kind, self.chunk_data(e)))
    }

    /// A resource by `usage` + `number` (e.g. `(b"Snd ", 3)`): its chunk type
    /// and data slice, or `None` when no such resource exists.
    pub fn resource(&self, usage: &[u8; 4], number: u32) -> Option<(&[u8; 4], &[u8])> {
        let e = self
            .index
            .iter()
            .find(|r| &r.usage == usage && r.number == number)?;
        Some((&e.chunk_type, self.chunk_data(e)))
    }

    /// Data bytes + detected [`SoundKind`] for sound resource `number`
    /// (`usage == b"Snd "`), or `None` when no such resource exists. An AIFF
    /// (`FORM`) sound retains its `FORM`+length header. The kind is
    /// detected from the chunk type: `FORM` → AIFF, `OGGV` → Ogg, `MOD ` → Mod,
    /// anything else → Other.
    pub fn sound(&self, number: u32) -> Option<(&[u8], SoundKind)> {
        let e = self
            .index
            .iter()
            .find(|r| &r.usage == b"Snd " && r.number == number)?;
        let kind = match &e.chunk_type {
            b"FORM" => SoundKind::Aiff,
            b"OGGV" => SoundKind::Ogg,
            b"MOD " => SoundKind::Mod,
            _ => SoundKind::Other,
        };
        Some((self.chunk_data(e), kind))
    }

    /// True if this Blorb carries any `Snd ` sound resource.
    pub fn has_sounds(&self) -> bool {
        self.resources().iter().any(|r| &r.usage == b"Snd ")
    }

    /// True if this Blorb carries any `Pict` image resource.
    pub fn has_pictures(&self) -> bool {
        self.resources().iter().any(|r| &r.usage == b"Pict")
    }

    /// True if this Blorb embeds its own `Exec` executable — i.e. it is a
    /// self-contained game rather than a resource-only sidecar.
    pub fn has_executable(&self) -> bool {
        self.resources().iter().any(|r| &r.usage == b"Exec")
    }
}

/// Read + parse `path` into a Blorb only if it is a story's resource *sidecar*:
/// a Blorb that carries resources but embeds no `Exec` of its own. We don't
/// filter by resource type — sounds, images, and data are all shown — but a
/// blorb that is itself a game (has an `Exec`) is a different story, not this
/// one's sidecar, so it is excluded from the sibling scan.
/// The filename extensions a RESOURCE blorb wears (SQ-1067).
///
/// One list, consulted by both steps of both [`resolve_resource_blorb`] and
/// [`sibling_blorb_by_name`]. They used to carry four literal lists between them
/// and the sets differed in BOTH steps, in opposite directions, so neither
/// function was a superset of the other — under a doc sentence promising "same
/// match order and rule". Measured before the fix, with resource-only sidecars:
/// a Glulx story beside a `.gblorb` resolved its resources while the row never
/// lit `(blorb)`, and a same-stem `.zblorb` in a prefix tie lit `(blorb)` while
/// resolution returned nothing. Two opposite lies in the story list.
///
/// **`.gblorb` and `.zblorb` are safe in step 1** because the two functions guard
/// it differently and both guards are right: `resolve` reads the file through
/// [`read_resource_blorb`], which requires resources AND no executable — so a
/// `.gblorb` that IS the story is rejected there and picked up by step 0 instead.
/// `sibling` cannot read the file and does not claim to; filename agreement is
/// the whole signal it offers, as its own doc says.
pub(crate) const RESOURCE_BLORB_EXTS: [&str; 4] = ["blb", "blorb", "zblorb", "gblorb"];

fn read_resource_blorb(path: &std::path::Path) -> Option<Blorb> {
    let bytes = std::fs::read(path).ok()?;
    if !Blorb::is_blorb(&bytes) {
        return None;
    }
    let b = Blorb::parse(bytes).ok()?;
    if !b.resources().is_empty() && !b.has_executable() {
        Some(b)
    } else {
        None
    }
}

/// Length of the shared leading run of bytes between two ascii-lowercased stems.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

/// From `(prefix_len, value)` candidates, the single value with the longest
/// prefix — but only if that prefix is >=3 chars and no other candidate ties
/// it (a tie for the longest is a real collision, so return `None` rather than
/// pick by directory order). `None` if nothing reaches 3. Shared by
/// [`resolve_resource_blorb`]'s directory scan and [`sibling_blorb_by_name`] so the
/// prefix/ambiguity rule lives in one place.
fn best_unambiguous_prefix<T>(candidates: Vec<(usize, T)>) -> Option<T> {
    let max_plen = candidates.iter().map(|(plen, _)| *plen).filter(|plen| *plen >= 3).max()?;
    let mut winners = candidates.into_iter().filter(|(plen, _)| *plen == max_plen);
    let first = winners.next().map(|(_, value)| value);
    if winners.next().is_none() {
        first
    } else {
        None // tie → ambiguous
    }
}

/// If `cand_stem` names a plausible resource sidecar for `story_stem`, the
/// length of their shared prefix; else `None`. Requires the SHORTER stem to be a
/// full prefix of the longer (and at least 3 chars): `lurking`↔`lurkinghorror…`
/// and `zork1`↔`zork1-sounds` qualify, but `zork0`↔`zork1` does not — they only
/// share the `zork` series prefix and diverge at the game number, so neither
/// stem fully prefixes the other. This stops one game's sidecar (e.g.
/// `zork0.blb`) from being grabbed by every sibling in a numbered series.
fn stem_prefix_match(story_stem: &str, cand_stem: &str) -> Option<usize> {
    let plen = common_prefix_len(story_stem, cand_stem);
    let shorter = story_stem.len().min(cand_stem.len());
    (plen >= 3 && plen == shorter).then_some(plen)
}

/// Resolve the Blorb holding a story's media resources (sounds and/or
/// pictures), plus the path it was read from, or `None`.
///
/// Order: (1) the story file itself if it is a Blorb; (2) a same-stem
/// `<story>.blb`/`.blorb` sibling with resources; (3) a directory-scan fallback
/// over blorb-extension files that carry `Snd `/`Pict` resources, picking the
/// best stem-prefix match (>=3 chars) or the sole candidate — else `None`
/// (ambiguous ⇒ no resources rather than the wrong game's). Not sound-only:
/// image-only sidecars (e.g. Beyond Zork's `beyondzork.blb`) resolve too.
pub fn resolve_resource_blorb(
    story_path: &std::path::Path,
) -> Option<(Blorb, std::path::PathBuf)> {
    // 1. Story file is itself a Blorb.
    if let Ok(bytes) = std::fs::read(story_path) {
        if Blorb::is_blorb(&bytes) {
            if let Ok(b) = Blorb::parse(bytes) {
                return Some((b, story_path.to_path_buf()));
            }
        }
    }
    // 2. Same-stem sibling.
    for ext in RESOURCE_BLORB_EXTS {
        let cand = story_path.with_extension(ext);
        if cand != story_path && cand.exists() {
            if let Some(b) = read_resource_blorb(&cand) {
                return Some((b, cand));
            }
        }
    }
    // 3. Directory-scan fallback (blorb-extension files with resources only).
    let dir = story_path.parent()?;
    let story_stem = story_path
        .file_stem()?
        .to_string_lossy()
        .to_ascii_lowercase();
    let mut candidates: Vec<(usize, (Blorb, std::path::PathBuf))> = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path == story_path {
            continue;
        }
        let ext_ok = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                let e = e.to_ascii_lowercase();
                RESOURCE_BLORB_EXTS.contains(&e.as_str())
            })
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        let Some(b) = read_resource_blorb(&path) else {
            continue;
        };
        let cand_stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if let Some(plen) = stem_prefix_match(&story_stem, &cand_stem) {
            candidates.push((plen, (b, path)));
        }
    }
    // Require an unambiguous full-prefix match — no sole-candidate fallback,
    // which would grab the wrong game's sidecar (e.g. zork0.blb) whenever it is
    // the only resource blorb beside a story it doesn't actually belong to.
    best_unambiguous_prefix(candidates)
}

/// The associated resource-blorb sibling of `story_path`, matched by FILENAME
/// ONLY (no file read), for the cheap per-row "(blorb)" tag which can't afford
/// to parse every blorb. Same match order and rule as [`resolve_resource_blorb`],
/// over the same `RESOURCE_BLORB_EXTS` (SQ-1067 — this sentence used to name a
/// third set, matching neither function): an exact same-stem sibling first, else
/// the best unambiguous `stem_prefix_match` among the directory's blorbs —
/// `None` if there is none or the longest prefix is a tie. Unlike
/// `resolve_resource_blorb` it does not read the file, so it can't require the
/// blorb to actually carry resources — filename agreement is the whole signal.
pub fn sibling_blorb_by_name(
    story_path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    // 1. Exact same-stem sibling.
    for ext in RESOURCE_BLORB_EXTS {
        let cand = story_path.with_extension(ext);
        if cand != story_path && cand.exists() {
            return Some(cand);
        }
    }
    // 2. Best unambiguous stem-prefix match in the same directory.
    let dir = story_path.parent()?;
    let story_stem = story_path.file_stem()?.to_string_lossy().to_ascii_lowercase();
    let mut candidates: Vec<(usize, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path == story_path {
            continue;
        }
        let ext_ok = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                let e = e.to_ascii_lowercase();
                RESOURCE_BLORB_EXTS.contains(&e.as_str())
            })
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        let cand_stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if let Some(plen) = stem_prefix_match(&story_stem, &cand_stem) {
            candidates.push((plen, path));
        }
    }
    best_unambiguous_prefix(candidates)
}

#[cfg(test)]
mod identifier_tests {
    use super::*;

    /// A Z-machine header stating `release` / `serial` / `checksum`.
    fn story(release: u16, serial: &[u8; 6], checksum: u16) -> Vec<u8> {
        let mut s = vec![0u8; 0x40];
        s[0] = 6;
        s[0x02..0x04].copy_from_slice(&release.to_be_bytes());
        s[0x12..0x18].copy_from_slice(serial);
        s[0x1c..0x1e].copy_from_slice(&checksum.to_be_bytes());
        s
    }

    /// Quetzal §5.4's field offsets, read off a real story header.
    #[test]
    fn a_story_header_states_release_serial_and_checksum() {
        let id = GameIdentifier::of_story(&story(63, b"890622", 0x45eb)).unwrap();
        assert_eq!(id.release, 63);
        assert_eq!(id.serial_str(), "890622");
        assert_eq!(id.checksum, 0x45eb);
        assert_eq!(id.to_string(), "release 63, serial 890622");
        // Too short to carry one.
        assert_eq!(GameIdentifier::of_story(&[0u8; 0x1d]), None);
    }

    /// Apple II releases store header text in high ASCII. The SAME serial must
    /// not read as two different builds depending on the medium (SQ-0856's
    /// masking, applied to identity).
    #[test]
    fn a_high_ascii_serial_is_the_same_build_as_its_plain_one() {
        let plain = GameIdentifier::of_story(&story(63, b"890622", 0x45eb)).unwrap();
        let high: [u8; 6] = [0xb8, 0xb9, 0xb0, 0xb6, 0xb2, 0xb2];
        let apple = GameIdentifier::of_story(&story(63, &high, 0x45eb)).unwrap();
        assert_eq!(apple, plain, "high ASCII is an encoding, not a different release");
        assert_eq!(apple.serial_str(), "890622");
    }

    /// The `IFhd` chunk's own packing (release, serial, checksum, then the PC the
    /// Blorb spec calls meaningless) is NOT the story header's spacing.
    #[test]
    fn a_blorbs_ifhd_states_which_build_its_resources_are_for() {
        let mut ifhd = Vec::new();
        ifhd.extend_from_slice(&74u16.to_be_bytes());
        ifhd.extend_from_slice(b"890714");
        ifhd.extend_from_slice(&0xd526u16.to_be_bytes());
        ifhd.extend_from_slice(&[0, 0, 0]); // "should be set to zero"
        assert_eq!(ifhd.len(), 13, "Quetzal §5.4.2");
        let bytes = tests::build_blorb_with_top(
            &[(b"Pict", 1, b"PNG ", &[1, 2, 3, 4])],
            &[(b"IFhd", &ifhd)],
        );
        let id = Blorb::parse(bytes).unwrap().game_identifier().unwrap();
        assert_eq!((id.release, id.serial_str().into_owned(), id.checksum), (74, "890714".into(), 0xd526));
    }

    /// The chunk is optional, and most of the corpus omits it. An absent
    /// identifier must read as "does not say", never as a mismatch.
    #[test]
    fn a_blorb_without_the_chunk_states_no_build_at_all() {
        let bytes = tests::build_blorb(&[(b"Pict", 1, b"PNG ", &[1, 2, 3, 4])]);
        assert_eq!(Blorb::parse(bytes).unwrap().game_identifier(), None);
    }

    /// A truncated chunk says nothing rather than half a build, and never panics.
    #[test]
    fn a_short_ifhd_chunk_is_ignored_rather_than_half_read() {
        let bytes = tests::build_blorb_with_top(
            &[(b"Pict", 1, b"PNG ", &[1, 2, 3, 4])],
            &[(b"IFhd", &[0, 74, b'8', b'9'])],
        );
        assert_eq!(Blorb::parse(bytes).unwrap().game_identifier(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build an IFF chunk: type + BE len + data + pad-to-even.
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    /// Build a Blorb with the given resources. Each resource is
    /// (usage, number, chunk_type, data). Returns the file bytes.
    // `pub(super)`: `identifier_tests` above builds containers with the same two
    // helpers rather than growing a second copy of the layout arithmetic.
    pub(super) type BlorbRes<'a> = (&'a [u8; 4], u32, &'a [u8; 4], &'a [u8]);
    pub(super) fn build_blorb(res: &[BlorbRes]) -> Vec<u8> {
        build_blorb_with_top(res, &[])
    }

    /// `top` = extra top-level chunks as (type, data), emitted after RIdx.
    /// Resource offsets must account for their size — hence the shared body layout.
    pub(super) fn build_blorb_with_top(res: &[BlorbRes], top: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        // Lay out the resource chunks after the RIdx chunk (and any top-level
        // chunks) to compute offsets.
        let count = res.len() as u32;
        let ridx_data_len = 4 + 12 * res.len();
        let mut top_chunks = Vec::new();
        for (ty, data) in top {
            top_chunks.extend_from_slice(&chunk(ty, data));
        }
        // RIdx chunk header (8) sits at file offset 12 (after FORM+len+IFRS);
        // resources follow RIdx AND the top-level chunks.
        let first_res_off =
            12 + 8 + ridx_data_len + (ridx_data_len % 2) + top_chunks.len();
        let mut offsets = Vec::new();
        let mut cursor = first_res_off;
        let mut body = Vec::new();
        for (_u, _n, ty, data) in res {
            offsets.push(cursor as u32);
            let c = chunk(ty, data);
            cursor += c.len();
            body.extend_from_slice(&c);
        }
        // RIdx data
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&count.to_be_bytes());
        for (i, (usage, number, _ty, _d)) in res.iter().enumerate() {
            ridx.extend_from_slice(*usage);
            ridx.extend_from_slice(&number.to_be_bytes());
            ridx.extend_from_slice(&offsets[i].to_be_bytes());
        }
        let ridx_chunk = chunk(b"RIdx", &ridx);
        // Assemble FORM
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&top_chunks);
        inner.extend_from_slice(&body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    #[test]
    fn is_blorb_detects_magic() {
        let b = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")]);
        assert!(Blorb::is_blorb(&b));
        assert!(!Blorb::is_blorb(b"not a blorb at all"));
        assert!(!Blorb::is_blorb(&[]));
    }

    #[test]
    fn parse_indexes_resources() {
        let b = build_blorb(&[
            (b"Exec", 0, b"ZCOD", b"abcd"),
            (b"Snd ", 1, b"FORM", b"xyz"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        assert_eq!(blorb.resources().len(), 2);
        let exec = blorb
            .resources()
            .iter()
            .find(|r| &r.usage == b"Exec")
            .unwrap();
        assert_eq!(&exec.chunk_type, b"ZCOD");
        assert_eq!(exec.len, 4);
    }

    #[test]
    fn parse_rejects_malformed() {
        assert_eq!(Blorb::parse(b"junk".to_vec()).unwrap_err(), BlorbError::NotBlorb);
        // FORM/IFRS but truncated before any chunk
        let mut t = b"FORM".to_vec();
        t.extend_from_slice(&4u32.to_be_bytes());
        t.extend_from_slice(b"IFRS");
        assert_eq!(Blorb::parse(t).unwrap_err(), BlorbError::NoResourceIndex);
    }

    #[test]
    fn executable_returns_zcode_data() {
        let b = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")]);
        let blorb = Blorb::parse(b).unwrap();
        let (kind, data) = blorb.executable().unwrap();
        assert_eq!(kind, ExecKind::ZCode);
        assert_eq!(data, b"abcd");
    }

    #[test]
    fn executable_detects_glulx() {
        let b = build_blorb(&[(b"Exec", 0, b"GLUL", b"glul")]);
        assert_eq!(Blorb::parse(b).unwrap().executable().unwrap().0, ExecKind::Glulx);
    }

    #[test]
    fn executable_detects_scott() {
        // Scott Adams / ScottFree games ship in a blorb `SAAI` Exec chunk.
        let b = build_blorb(&[(b"Exec", 0, b"SAAI", b"scott-dat")]);
        let blorb = Blorb::parse(b).unwrap();
        let (kind, data) = blorb.executable().unwrap();
        assert_eq!(kind, ExecKind::Scott);
        assert_eq!(data, b"scott-dat");
    }

    #[test]
    fn executable_missing_is_error() {
        let b = build_blorb(&[(b"Snd ", 1, b"FORM", b"x")]);
        assert_eq!(
            Blorb::parse(b).unwrap().executable().unwrap_err(),
            BlorbError::NoExecutable
        );
    }

    #[test]
    fn frontispiece_reads_fspc_number() {
        // FORM/IFRS with an RIdx (one Pict) followed by a top-level Fspc chunk
        // whose 4-byte payload is the frontispiece resource number (7).
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // count
        ridx.extend_from_slice(b"Pict");
        ridx.extend_from_slice(&7u32.to_be_bytes()); // number
        // start offset patched below once we know the layout.
        let ridx_chunk_len = 8 + (4 + 12);
        let fspc_chunk_len = 8 + 4;
        let pict_off = 12 + ridx_chunk_len + fspc_chunk_len;
        ridx.extend_from_slice(&(pict_off as u32).to_be_bytes()); // start
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&chunk(b"Fspc", &7u32.to_be_bytes()));
        inner.extend_from_slice(&chunk(b"PNG ", b"\x89PNG")); // dummy Pict payload
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);

        let b = Blorb::parse(file).unwrap();
        assert_eq!(b.frontispiece(), Some(7));
        // The referenced Pict resource is reachable via the generic accessor.
        assert_eq!(b.resource(b"Pict", 7).unwrap().0, b"PNG ");
    }

    #[test]
    fn frontispiece_absent_is_none() {
        let b = Blorb::parse(build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")])).unwrap();
        assert_eq!(b.frontispiece(), None);
    }

    /// The spec gives `Fspc` a length of exactly 4 — one resource number. A
    /// chunk declaring less than that has not said a number, so the container
    /// parses with no frontispiece rather than with a number read off whatever
    /// followed it (SQ-0985; the `clen >= 4` guard in `parse`). The rest of the
    /// blorb must survive: one bad optional chunk is not a bad blorb.
    #[test]
    fn a_short_fspc_chunk_declares_no_frontispiece() {
        let b = Blorb::parse(build_blorb_with_top(
            &[(b"Exec", 0, b"ZCOD", b"abcd")],
            &[(b"Fspc", &[0, 7])], // two bytes: not a resource number
        ))
        .unwrap();
        assert_eq!(b.frontispiece(), None);
        assert_eq!(b.resource(b"Exec", 0).unwrap().0, b"ZCOD");
    }

    #[test]
    fn ifmd_chunk_is_exposed_verbatim() {
        let xml = br#"<ifindex version="1.0"><story><bibliographic><title>T</title></bibliographic></story></ifindex>"#;
        let b = Blorb::parse(build_blorb_with_top(
            &[(b"Exec", 0, b"ZCOD", b"abcd")],
            &[(b"IFmd", xml)],
        ))
        .unwrap();
        assert_eq!(b.metadata(), Some(&xml[..]), "IFmd bytes returned uninterpreted");
    }

    #[test]
    fn blorb_without_ifmd_has_no_metadata() {
        let b = Blorb::parse(build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")])).unwrap();
        assert_eq!(b.metadata(), None);
    }

    /// An odd-length IFmd is padded to even in the container; the returned slice
    /// must be the DECLARED length, not the padded one, or the XML gains a NUL
    /// byte and roxmltree rejects it.
    #[test]
    fn odd_length_ifmd_excludes_its_pad_byte() {
        let odd = b"<ifindex></ifindex>"; // 19 bytes → forces a pad byte
        assert_eq!(odd.len() % 2, 1, "the fixture must be odd for this test to mean anything");
        let b = Blorb::parse(build_blorb_with_top(
            &[(b"Exec", 0, b"ZCOD", b"abcd")],
            &[(b"IFmd", odd)],
        ))
        .unwrap();
        assert_eq!(b.metadata(), Some(&odd[..]), "no trailing pad byte");
    }

    /// Adding a top-level chunk must not break resource offset resolution.
    #[test]
    fn resources_still_resolve_with_an_ifmd_present() {
        let b = Blorb::parse(build_blorb_with_top(
            &[(b"Pict", 1, b"PNG ", b"pngdata")],
            &[(b"IFmd", b"<ifindex/>")],
        ))
        .unwrap();
        assert_eq!(b.resource(b"Pict", 1).map(|r| r.1), Some(&b"pngdata"[..]));
    }

    #[test]
    fn resource_fetches_by_usage_number() {
        let b = build_blorb(&[
            (b"Exec", 0, b"ZCOD", b"abcd"),
            (b"Snd ", 3, b"OGGV", b"oggdata"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        let (ty, data) = blorb.resource(b"Snd ", 3).unwrap();
        assert_eq!(ty, b"OGGV");
        assert_eq!(data, b"oggdata");
        assert!(blorb.resource(b"Snd ", 99).is_none());
    }

    #[test]
    fn form_resource_retains_its_header() {
        // SQ-0323: a FORM resource (nested IFF FORM) must be returned WITH its
        // 8-byte `FORM`+length header — the header is part of the resource data
        // per the Blorb spec — whereas a non-FORM chunk yields only its payload.
        let b = build_blorb(&[
            (b"Data", 8, b"FORM", b"THIS"),   // FORM resource: keep the header
            (b"Data", 9, b"TEXT", b"payload"), // non-FORM: payload only
        ]);
        let blorb = Blorb::parse(b).unwrap();
        let (ty, data) = blorb.resource(b"Data", 8).unwrap();
        assert_eq!(ty, b"FORM");
        // Whole chunk: "FORM" + big-endian length (4) + "THIS" = 12 bytes.
        let mut expected = b"FORM".to_vec();
        expected.extend_from_slice(&4u32.to_be_bytes());
        expected.extend_from_slice(b"THIS");
        assert_eq!(data, &expected[..], "FORM resource keeps its 8-byte header");
        assert_eq!(blorb.resource(b"Data", 9).unwrap().1, b"payload", "non-FORM: payload only");
    }

    #[test]
    fn resource_handles_odd_length_pad_byte() {
        // First chunk has odd-length data ("oggdata" = 7); the pad byte must be
        // skipped so the following resource is still found at its real offset.
        let b = build_blorb(&[
            (b"Snd ", 3, b"OGGV", b"oggdata"),
            (b"Exec", 0, b"ZCOD", b"abcd"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        assert_eq!(blorb.resource(b"Snd ", 3).unwrap().1, b"oggdata");
        let (kind, data) = blorb.executable().unwrap();
        assert_eq!(kind, ExecKind::ZCode);
        assert_eq!(data, b"abcd");
    }

    #[test]
    fn sound_fetches_aiff_by_number() {
        let b = build_blorb(&[
            (b"Exec", 0, b"ZCOD", b"abcd"),
            (b"Snd ", 7, b"FORM", b"aiffbytes"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        let (data, kind) = blorb.sound(7).unwrap();
        // A FORM resource keeps its 8-byte `FORM`+length header (Blorb spec).
        let mut expected = b"FORM".to_vec();
        expected.extend_from_slice(&(b"aiffbytes".len() as u32).to_be_bytes());
        expected.extend_from_slice(b"aiffbytes");
        assert_eq!(data, &expected[..]);
        assert_eq!(kind, SoundKind::Aiff);
        assert!(blorb.sound(99).is_none(), "absent sound number returns None");
    }

    #[test]
    fn sound_detects_ogg_mod_other() {
        let b = build_blorb(&[
            (b"Snd ", 1, b"OGGV", b"ogg"),
            (b"Snd ", 2, b"MOD ", b"mod"),
            (b"Snd ", 3, b"AIFF", b"weird"),
        ]);
        let blorb = Blorb::parse(b).unwrap();
        assert_eq!(blorb.sound(1).unwrap().1, SoundKind::Ogg);
        assert_eq!(blorb.sound(2).unwrap().1, SoundKind::Mod);
        assert_eq!(blorb.sound(3).unwrap().1, SoundKind::Other);
    }

    #[test]
    fn parse_rejects_bad_offset() {
        // A valid RIdx whose single entry points past EOF → BadOffset.
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // count
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes()); // number
        ridx.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // start past EOF
        let ridx_chunk = chunk(b"RIdx", &ridx);
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        assert_eq!(Blorb::parse(file).unwrap_err(), BlorbError::BadOffset);
    }

    /// Build FORM/IFRS bytes with an arbitrary RIdx payload followed by `rest`.
    fn blorb_with_ridx_payload(ridx_payload: &[u8], rest: &[u8]) -> Vec<u8> {
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&chunk(b"RIdx", ridx_payload));
        inner.extend_from_slice(rest);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    /// SQ-0629: an untrusted RIdx count (0xFFFFFFFF would ask `with_capacity`
    /// for ~170GB) must be validated against the RIdx chunk length before any
    /// allocation or entry parsing.
    #[test]
    fn parse_rejects_huge_ridx_count() {
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // count, no entries
        let file = blorb_with_ridx_payload(&ridx, &[]);
        assert_eq!(Blorb::parse(file).unwrap_err(), BlorbError::Truncated);
    }

    /// SQ-0629: a count claiming more entries than the RIdx chunk holds must
    /// error rather than walk into the chunks that follow — entry reads were
    /// previously bounded only against EOF, so a trailing chunk's bytes parsed
    /// as phantom index entries.
    #[test]
    fn parse_rejects_ridx_count_exceeding_its_chunk_length() {
        // RIdx declares TWO entries but its chunk length only covers one
        // (4 + 12 bytes). A well-formed Exec resource chunk follows, so under
        // the old EOF-only bound the second "entry" read the next chunk's bytes.
        let exec_off: u32 = 12 + 8 + 16 + 24; // FORM hdr + RIdx hdr + RIdx data + this Fake chunk
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&2u32.to_be_bytes()); // count = 2 (lie)
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes());
        ridx.extend_from_slice(&exec_off.to_be_bytes());
        let mut rest = Vec::new();
        rest.extend_from_slice(&chunk(b"Fake", &[0u8; 16]));
        rest.extend_from_slice(&chunk(b"ZCOD", b"abcd"));
        let file = blorb_with_ridx_payload(&ridx, &rest);
        assert_eq!(Blorb::parse(file).unwrap_err(), BlorbError::Truncated);
    }

    #[test]
    fn parse_tolerates_malformed_trailing_chunk() {
        // A valid RIdx + Exec (ZCOD) resource, followed by a malformed
        // TRAILING top-level chunk whose declared length overruns EOF. The
        // chunk-walk loop must stop scanning rather than fail the whole
        // parse — the resource index was already fully built from RIdx.
        let mut file = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")]);
        file.extend_from_slice(b"XXXX");
        file.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // declared len far past EOF

        let blorb = Blorb::parse(file).expect("malformed trailing chunk must not fail parse");
        let (kind, data) = blorb.executable().unwrap();
        assert_eq!(kind, ExecKind::ZCode);
        assert_eq!(data, b"abcd");
        assert!(blorb.resources().iter().any(|r| &r.usage == b"Exec"));
    }

    // ── resolve_resource_blorb / has_sounds ────────────────────────────────────

    /// RAII guard that removes a temp directory (and its contents) on drop.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "lanthorn-blorb-test-{}-{}-{n}",
                std::process::id(),
                tag
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn join(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sound_blorb() -> Vec<u8> {
        sound_blorb_tagged(b"aiffdata")
    }

    /// A resource-only sound sidecar (a `Snd ` resource, no `Exec` — a real
    /// sidecar is not a game of its own). The `Snd ` payload is exactly `tag`,
    /// so tests can tell which file the resolver picked among several candidates.
    /// Uses an `OGGV` chunk (not `FORM`) so `sound()` returns the payload
    /// verbatim — a `FORM` resource would retain its 8-byte header (see `sound()`).
    fn sound_blorb_tagged(tag: &[u8]) -> Vec<u8> {
        build_blorb(&[(b"Snd ", 1, b"OGGV", tag)])
    }

    /// A self-contained game Blorb (has an `Exec`) — NOT a sidecar. The sibling
    /// scan must skip these so a story next to an unrelated game isn't paired
    /// with it.
    fn game_blorb() -> Vec<u8> {
        build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd"), (b"Snd ", 1, b"OGGV", b"x")])
    }

    /// A picture-only Blorb (one `Pict` resource, no `Snd `), mirroring a
    /// graphics sidecar like Beyond Zork's `beyondzork.blb`.
    fn pict_blorb() -> Vec<u8> {
        build_blorb(&[(b"Pict", 1, b"PNG ", b"fakepngdata")])
    }

    #[test]
    fn resolve_finds_picture_only_sidecar() {
        // SQ-0372: a no-sound game (Beyond Zork) ships an images-only resource
        // blorb with a different stem; the resolver must accept it rather than
        // gate on has_sounds() and miss it.
        let dir = TempDir::new("pict-only");
        let story = dir.join("beyondzork-r57-s871221.z5");
        std::fs::write(&story, b"not a blorb").unwrap();
        let sibling = dir.join("beyondzork.blb");
        std::fs::write(&sibling, pict_blorb()).unwrap();

        let (b, path) = resolve_resource_blorb(&story).expect("images-only sidecar resolves");
        assert!(!b.has_sounds() && b.has_pictures(), "picture-only blorb");
        assert_eq!(path, sibling);
    }

    #[test]
    fn resolve_prefers_story_that_is_itself_a_blorb() {
        // The story file itself is a self-contained game blorb (has Exec); step 1
        // returns it directly — the not-a-game sibling gate never applies here.
        let dir = TempDir::new("self-blorb");
        let game = dir.join("game.blb");
        std::fs::write(&game, game_blorb()).unwrap();

        let (b, path) = resolve_resource_blorb(&game).expect("resolves the story's own Blorb");
        assert!(b.has_sounds());
        assert_eq!(path, game);
    }

    #[test]
    fn resolve_skips_a_sibling_that_is_itself_a_game() {
        // A prefix-matching sibling that embeds its own Exec is a different game,
        // not this story's resource sidecar — it must not be paired.
        let dir = TempDir::new("game-sibling");
        let story = dir.join("zork1.z3");
        std::fs::write(&story, b"not a blorb").unwrap();
        std::fs::write(dir.join("zork1-sequel.blorb"), game_blorb()).unwrap();
        assert!(
            resolve_resource_blorb(&story).is_none(),
            "an Exec-bearing sibling is a game, not a sidecar"
        );
    }

    #[test]
    fn resolve_finds_same_stem_sibling() {
        let dir = TempDir::new("sibling");
        let story = dir.join("game.z5");
        std::fs::write(&story, b"not a blorb").unwrap();
        let sibling = dir.join("game.blb");
        std::fs::write(&sibling, sound_blorb()).unwrap();

        let (b, path) = resolve_resource_blorb(&story).expect("resolves the same-stem sibling");
        assert!(b.has_sounds());
        assert_eq!(path, sibling);
    }

    #[test]
    fn resolve_scans_directory_for_prefixed_blorb() {
        let dir = TempDir::new("prefix-scan");
        let story = dir.join("lurkinghorror.z5");
        std::fs::write(&story, b"not a blorb").unwrap();
        let prefixed = dir.join("lurkinghorror-sounds.blorb");
        std::fs::write(&prefixed, sound_blorb_tagged(b"prefixed-data")).unwrap();
        let unrelated = dir.join("arthur.blb");
        std::fs::write(&unrelated, sound_blorb_tagged(b"arthur-data")).unwrap();

        let (b, path) = resolve_resource_blorb(&story).expect("resolves the prefix-matching blorb");
        assert!(b.has_sounds());
        assert_eq!(
            b.sound(1).unwrap().0,
            b"prefixed-data",
            "must pick the prefix-matching blorb, not the unrelated one"
        );
        assert_eq!(path, prefixed);
    }

    #[test]
    fn resolve_returns_none_on_prefix_tie() {
        // Two sidecars whose stems each fully prefix the story stem, tied for the
        // longest prefix → ambiguous → None (don't guess by directory order).
        let dir = TempDir::new("prefix-tie");
        let story = dir.join("zork1.z3");
        std::fs::write(&story, b"not a blorb").unwrap();
        std::fs::write(dir.join("zork1-sounds.blorb"), sound_blorb_tagged(b"a")).unwrap();
        std::fs::write(dir.join("zork1-music.blorb"), sound_blorb_tagged(b"b")).unwrap();
        assert!(
            resolve_resource_blorb(&story).is_none(),
            "tie for longest prefix must be ambiguous"
        );
    }

    #[test]
    fn resolve_rejects_a_shared_series_prefix() {
        // SQ-0372: zork0.blb (Zork Zero's resources) must not be paired with
        // Zork I — they share only the "zork" series prefix and diverge at the
        // game number, and zork0's stem does not fully prefix zork1's. This holds
        // even though zork0.blb is the sole resource blorb in the folder.
        let dir = TempDir::new("series-resolve");
        let story = dir.join("zork1-r88-s840726.z3");
        std::fs::write(&story, b"not a blorb").unwrap();
        std::fs::write(dir.join("zork0.blb"), pict_blorb()).unwrap();
        assert!(
            resolve_resource_blorb(&story).is_none(),
            "zork0.blb belongs to Zork Zero, not Zork I"
        );
    }

    /// **The `(blorb)` tag and the resolver agree about every resource extension**
    /// (SQ-1067).
    ///
    /// `sibling_blorb_by_name` promises "same match order and rule as
    /// `resolve_resource_blorb`". The ORDER always matched; the extension SETS did
    /// not, and they differed in BOTH steps in opposite directions, so neither
    /// function was a superset of the other:
    ///
    /// | step | `resolve` | `sibling` |
    /// |---|---|---|
    /// | 1, exact same-stem | `blb`, `blorb` | `blb`, `blorb`, `zblorb` |
    /// | 2, directory scan | `blb`, `blorb`, `zblorb`, `gblorb` | `blb`, `blorb`, `zblorb` |
    ///
    /// The doc sentence described a third set, matching neither. The result was
    /// two opposite lies in the story list: a Glulx story beside a resource-only
    /// `.gblorb` loaded its resources while the row never lit `(blorb)`, and a
    /// same-stem `.zblorb` caught in a prefix TIE lit `(blorb)` while resolution
    /// returned nothing.
    ///
    /// These are the audit's own four measured rows plus a negative, and every
    /// existing `sibling_by_name_*` case uses `.blb`/`.blorb` — the two spellings
    /// where the sets happened to agree — so all 292 were green with the
    /// divergence in place.
    ///
    /// FALSIFY by restoring either literal list: rows 1–3 disagree.
    #[test]
    fn the_blorb_tag_and_the_resolver_agree_on_every_resource_extension() {
        // (story, sidecars, whether a resource sidecar should be found at all)
        let cases: &[(&str, &[&str], bool)] = &[
            // A resource-only `.gblorb`: resolve saw it, the tag could not.
            ("game.ulx", &["game.gblorb"], true),
            // …and the same by prefix rather than by exact stem.
            ("gamestory.ulx", &["game.gblorb"], true),
            // A same-stem `.zblorb` in a prefix tie: the tag saw it, resolve could not.
            ("game.z5", &["game.zblorb", "game2.zblorb"], true),
            // The control — the same shape spelled `.blb`, where the two always agreed.
            ("game.z5", &["game.blb", "game2.blb"], true),
            // A negative, so agreement is not just "both always find something":
            // an ambiguous tie with no same-stem sibling is `None` on both sides.
            ("zork1.z3", &["zork1-a.blorb", "zork1-b.blorb"], false),
        ];
        for (i, (story_name, sidecars, expect_found)) in cases.iter().enumerate() {
            let dir = TempDir::new(&format!("tag-agree-{i}"));
            let story = dir.join(story_name);
            std::fs::write(&story, b"not a blorb").unwrap();
            for name in *sidecars {
                std::fs::write(dir.join(name), sound_blorb()).unwrap();
            }
            let resolved = resolve_resource_blorb(&story).is_some();
            let tagged = sibling_blorb_by_name(&story).is_some();
            assert_eq!(
                resolved, tagged,
                "row {i} ({story_name} beside {sidecars:?}): the row's `(blorb)` tag says \
                 {tagged} and the launch says {resolved} — the tag is a claim about a file, \
                 contradicted by what opening it actually does",
            );
            assert_eq!(
                resolved, *expect_found,
                "row {i} ({story_name} beside {sidecars:?}): the premise — a resource-only \
                 sidecar is {} here",
                if *expect_found { "present and unambiguous" } else { "ambiguous, so neither should claim it" },
            );
        }
    }

    // ── sibling_blorb_by_name (filename-only, powers the "(blorb)" tag) ──────

    #[test]
    fn sibling_by_name_finds_exact_stem() {
        let dir = TempDir::new("byname-exact");
        let story = dir.join("game.z5");
        std::fs::write(&story, b"x").unwrap();
        let sibling = dir.join("game.blb");
        std::fs::write(&sibling, b"x").unwrap(); // content irrelevant: filename-only
        assert_eq!(sibling_blorb_by_name(&story), Some(sibling));
    }

    #[test]
    fn sibling_by_name_finds_prefix_match_case_insensitively() {
        // The real bug (SQ-0372): The Lurking Horror ships as
        // `lurkinghorror-r219-s870912.z3` beside `Lurking.blb` — different stems,
        // different case. The prefix scan must still pair them.
        let dir = TempDir::new("byname-prefix");
        let story = dir.join("lurkinghorror-r219-s870912.z3");
        std::fs::write(&story, b"x").unwrap();
        let sibling = dir.join("Lurking.blb");
        std::fs::write(&sibling, b"x").unwrap();
        assert_eq!(sibling_blorb_by_name(&story), Some(sibling));
    }

    #[test]
    fn sibling_by_name_none_on_prefix_tie() {
        let dir = TempDir::new("byname-tie");
        let story = dir.join("zork1.z3");
        std::fs::write(&story, b"x").unwrap();
        std::fs::write(dir.join("zork1-a.blorb"), b"x").unwrap();
        std::fs::write(dir.join("zork1-b.blorb"), b"x").unwrap();
        assert_eq!(sibling_blorb_by_name(&story), None, "ambiguous tie → None");
    }

    #[test]
    fn sibling_by_name_rejects_a_shared_series_prefix() {
        // SQ-0372: the (blorb) tag must agree with the resolver — zork0.blb must
        // not light for zork1/zork2/zork3, only for Zork Zero's own story.
        let dir = TempDir::new("byname-series");
        for n in ["1", "2", "3"] {
            std::fs::write(dir.join(&format!("zork{n}-r88-s840726.z3")), b"x").unwrap();
        }
        std::fs::write(dir.join("zork0.blb"), b"x").unwrap();
        for n in ["1", "2", "3"] {
            let story = dir.join(&format!("zork{n}-r88-s840726.z3"));
            assert_eq!(sibling_blorb_by_name(&story), None, "zork{n} must not match zork0.blb");
        }
        let z0 = dir.join("zork0-r393-s890714.z3");
        std::fs::write(&z0, b"x").unwrap();
        assert_eq!(
            sibling_blorb_by_name(&z0),
            Some(dir.join("zork0.blb")),
            "Zork Zero's own story pairs with zork0.blb"
        );
    }

    #[test]
    fn sibling_by_name_none_when_prefix_too_short_or_absent() {
        let dir = TempDir::new("byname-miss");
        let story = dir.join("zork.z3");
        std::fs::write(&story, b"x").unwrap();
        // Only a 2-char shared prefix ("zo"), below the >=3 floor.
        std::fs::write(dir.join("zone.blorb"), b"x").unwrap();
        // A blorb with no shared prefix at all.
        std::fs::write(dir.join("arthur.blb"), b"x").unwrap();
        assert_eq!(sibling_blorb_by_name(&story), None);
    }

    #[test]
    fn resolve_ambiguous_scan_returns_none() {
        let dir = TempDir::new("ambiguous");
        let story = dir.join("story.z5");
        std::fs::write(&story, b"not a blorb").unwrap();
        std::fs::write(dir.join("foo.blb"), sound_blorb()).unwrap();
        std::fs::write(dir.join("bar.blorb"), sound_blorb()).unwrap();

        assert!(resolve_resource_blorb(&story).is_none());
    }

    #[test]
    fn resolve_soundless_story_no_blorb_returns_none() {
        let dir = TempDir::new("no-blorb");
        let story = dir.join("plain.z5");
        std::fs::write(&story, b"not a blorb").unwrap();

        assert!(resolve_resource_blorb(&story).is_none());
    }

    /// A well-formed `APal` chunk (Blorb §11.3) lists picture numbers as
    /// big-endian u32s; they surface via `adaptive_pictures`/`is_adaptive_picture`.
    #[test]
    fn apal_lists_adaptive_picture_numbers() {
        let mut apal = Vec::new();
        for n in [9u32, 24, 216] {
            apal.extend_from_slice(&n.to_be_bytes());
        }
        let b = Blorb::parse(build_blorb_with_top(
            &[(b"Pict", 9, b"PNG ", b"x"), (b"Pict", 216, b"PNG ", b"y")],
            &[(b"APal", &apal)],
        ))
        .unwrap();
        assert_eq!(b.adaptive_pictures(), &[9, 24, 216]);
        assert!(b.is_adaptive_picture(9) && b.is_adaptive_picture(216));
        assert!(!b.is_adaptive_picture(1), "a picture not in APal is not adaptive");
    }

    /// No `APal` chunk → empty adaptive set (the common case for non-v6 blorbs).
    #[test]
    fn no_apal_means_no_adaptive_pictures() {
        let b = Blorb::parse(build_blorb(&[(b"Pict", 1, b"PNG ", b"x")])).unwrap();
        assert!(b.adaptive_pictures().is_empty());
        assert!(!b.is_adaptive_picture(1));
    }

    /// A malformed `APal` whose length is not a multiple of 4 must degrade to
    /// "no adaptive set" rather than fail the load (error-tolerant parsing).
    #[test]
    fn malformed_apal_degrades_to_empty() {
        let b = Blorb::parse(build_blorb_with_top(
            &[(b"Pict", 1, b"PNG ", b"x")],
            &[(b"APal", &[0, 0, 0, 9, 0xFF])], // 5 bytes: not a multiple of 4
        ))
        .expect("a corrupt APal must not fail the whole parse");
        assert!(b.adaptive_pictures().is_empty(), "corrupt APal → no adaptive set");
    }

    #[test]
    fn has_sounds_true_only_with_snd() {
        let with_sound = Blorb::parse(sound_blorb()).unwrap();
        assert!(with_sound.has_sounds());

        let without_sound = Blorb::parse(build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")])).unwrap();
        assert!(!without_sound.has_sounds());
    }
}
