//! What the NON-primary Glk windows hold, as a recipe a host Save State can carry
//! (SQ-1712).
//!
//! A Glulx snapshot (`gvm::Machine::save_state`) saves the window STRUCTURE (the
//! `Glk ` chunk) and nothing a window displays, because Glulx spec §1.8.5 leans on
//! the game to repaint after a restore. That holds for the game's own `@restore`,
//! where a real Glk library keeps the windows' contents. A host Save State swaps
//! memory under a game that never learns it happened — and `Machine::restore_state`
//! closes and reopens every backend window — so the host has to put the screen
//! back as of the save. Narcolepsy's bubble (text window 2, reprinted only when
//! the room changes) stayed empty until the next turn.
//!
//! The chunk is **the recipe, never the result** (CLAUDE.md, "Persist the recipe"):
//! a text buffer's styled log (inline images by Blorb resource number), a text
//! grid's cells in the game's own grid coordinates, and a graphics window's
//! draw-op list. No canvas pixels, no terminal cell geometry — a snapshot moves
//! between panes of any size and between graphics backends.
//!
//! It rides inside the engine-save bytes as one extra IFF chunk ([`CHUNK_ID`])
//! after the chunks gvm writes; a foreign reader skips an unknown chunk, and a
//! snapshot without one (older, or a window-less game) restores exactly as before.
//! The PRIMARY text buffer is never in it: the app transcript already mirrors
//! that window and the archive carries the transcript.

use crate::inline_image::ImageAlign;
use crate::state::ParaFmt;

/// The IFF chunk id appended to a Glulx host snapshot ("Lanthorn Window Contents").
pub(crate) const CHUNK_ID: &[u8; 4] = b"LtWc";
/// Wire version inside the chunk's JSON.
pub(crate) const VERSION: u32 = 1;

/// Per-buffer cap on retained log entries (newest kept). A non-primary buffer is a
/// panel the game reprints (bubbles, side lists), but a game that never clears one
/// would otherwise grow every snapshot — and the rewind history keeps one per turn.
pub(crate) const MAX_BUFFER_ELEMS: usize = 256;
/// Per-buffer cap on retained text bytes (newest kept).
pub(crate) const MAX_BUFFER_TEXT_BYTES: usize = 32 * 1024;
/// Per-graphics-window cap on retained draw ops (newest kept). A full-canvas
/// erase already collapses the list (see `AppGlk::note_gfx_op`).
pub(crate) const MAX_GFX_OPS: usize = 1024;

/// One graphics-window drawing operation, in the game's own pixel space.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum GfxOp {
    Fill { color: u32, left: i32, top: i32, w: u32, h: u32 },
    Erase { left: i32, top: i32, w: u32, h: u32 },
    Background(u32),
    Image { resnum: u32, x: i32, y: i32, scale: Option<(u32, u32)> },
}

/// A text-grid cell: `(row, col, char, style-bits, fg, bg, link, glk_style)`.
pub(crate) type GridCellDto = (u32, u32, char, u8, u32, u32, u32, u8);

/// One entry of a text buffer's log.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum ElemDto {
    Text { bits: u8, fg: u32, bg: u32, link: u32, para: ParaFmt, glk_style: u8, text: String },
    /// An inline picture by Blorb `Pict` resource number; the pixels are re-read
    /// from the story's own resources on reinstatement.
    Image {
        resource: u32,
        align: ImageAlign,
        scaled: Option<(u32, u32)>,
        margin_px: Option<u32>,
        /// `(rule, width, height, maxwidth)` of a standing `imagerule`.
        rule: Option<(u32, u32, u32, u32)>,
        link: u32,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Body {
    Buffer(Vec<ElemDto>),
    Grid(Vec<GridCellDto>),
    Graphics(Vec<GfxOp>),
}

/// One window's contents, keyed by the Glk window id gvm restores.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct WindowContent {
    pub id: u32,
    pub body: Body,
}

/// Everything the chunk carries.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Contents {
    pub v: u32,
    pub windows: Vec<WindowContent>,
}

/// Append `contents` to a `FORM IFZS` blob as an extra chunk, fixing up the FORM
/// length. A blob that is not a well-formed FORM, or empty contents, is returned
/// untouched (the snapshot then behaves as it always did).
pub(crate) fn append_chunk(mut form: Vec<u8>, contents: &Contents) -> Vec<u8> {
    if contents.windows.is_empty() || form.len() < 12 || &form[0..4] != b"FORM" {
        return form;
    }
    let Ok(json) = serde_json::to_vec(contents) else { return form };
    form.extend_from_slice(CHUNK_ID);
    form.extend_from_slice(&(json.len() as u32).to_be_bytes());
    form.extend_from_slice(&json);
    if json.len() % 2 == 1 {
        form.push(0);
    }
    let body_len = (form.len() - 8) as u32;
    form[4..8].copy_from_slice(&body_len.to_be_bytes());
    form
}

/// Read the contents chunk out of a `FORM IFZS` blob; `None` when absent or
/// unreadable (an older snapshot, a future version).
pub(crate) fn find_chunk(form: &[u8]) -> Option<Contents> {
    if form.len() < 12 || &form[0..4] != b"FORM" {
        return None;
    }
    let mut i = 12usize;
    while i + 8 <= form.len() {
        let len = u32::from_be_bytes([form[i + 4], form[i + 5], form[i + 6], form[i + 7]]) as usize;
        let start = i + 8;
        let end = start.checked_add(len)?;
        if end > form.len() {
            return None;
        }
        if &form[i..i + 4] == CHUNK_ID {
            let c: Contents = serde_json::from_slice(&form[start..end]).ok()?;
            return (c.v == VERSION).then_some(c);
        }
        i = end + (len % 2);
    }
    None
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    fn form() -> Vec<u8> {
        let mut v = b"FORM".to_vec();
        v.extend_from_slice(&4u32.to_be_bytes());
        v.extend_from_slice(b"IFZS");
        v
    }

    #[test]
    fn chunk_round_trips_and_keeps_the_form_length_honest() {
        let c = Contents {
            v: VERSION,
            windows: vec![WindowContent { id: 3, body: Body::Graphics(vec![GfxOp::Background(7)]) }],
        };
        let blob = append_chunk(form(), &c);
        let len = u32::from_be_bytes([blob[4], blob[5], blob[6], blob[7]]) as usize;
        assert_eq!(len + 8, blob.len(), "FORM length covers the appended chunk");
        let back = find_chunk(&blob).expect("chunk found");
        assert_eq!(back.windows.len(), 1);
        assert_eq!(back.windows[0].id, 3);
    }

    #[test]
    fn a_snapshot_without_the_chunk_reads_as_none() {
        assert!(find_chunk(&form()).is_none());
        assert!(find_chunk(b"junk").is_none());
    }

    #[test]
    fn empty_contents_add_nothing() {
        let blob = append_chunk(form(), &Contents { v: VERSION, windows: vec![] });
        assert_eq!(blob, form());
    }
}
