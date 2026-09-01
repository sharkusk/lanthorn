//! Cover-art decoding + terminal-protocol caching for the story picker.
//!
//! `load_cover` pulls a blorb's `Fspc` frontispiece image and decodes it,
//! falling back to a fetched `cover.png` sidecar (SQ-0348) when the story has
//! none of its own; `CoverState` holds a bounded LRU cache of decoded images
//! and lazily builds
//! (and caches) a `ratatui-image` protocol scaled to the panel's cover region
//! for the currently-selected story. `CoverDecoder` owns a background worker
//! thread that runs `load_cover` off the main loop so scrolling never stalls.
//! Every failure resolves to `None` — the picker simply shows no cover.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;

use crate::render::graphics::KittyDeleteQueue;

/// Decode PNG/JPEG/GIF bytes into a `DynamicImage`. `None` on any decode failure.
pub fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    image::load_from_memory(bytes).ok()
}

/// Read `path`; if it is a blorb declaring an `Fspc` frontispiece, fetch and
/// decode that Pict. `None` when the file isn't a blorb, has no frontispiece,
/// the referenced Pict is missing, or the image doesn't decode.
fn frontispiece_cover(path: &Path) -> Option<image::DynamicImage> {
    let bytes = std::fs::read(path).ok()?;
    if !blorb::Blorb::is_blorb(&bytes) {
        return None;
    }
    let b = blorb::Blorb::parse(bytes).ok()?;
    let n = b.frontispiece()?;
    let (_ty, data) = b.resource(b"Pict", n)?;
    decode(data)
}

/// `path`'s cover, by precedence: the story's own `Fspc` frontispiece always
/// wins; a fetched `<game_dir>/cover.png` (written by the fetch worker,
/// SQ-0348) is used only when the story has none. `game_dir` is `None` when
/// no fallback source is available (e.g. the IFDB-precedence check in
/// `fetch_worker`, which only cares whether a story already has its own
/// cover). `None` when neither source yields a decodable image.
pub fn load_cover(path: &Path, game_dir: Option<&Path>) -> Option<image::DynamicImage> {
    if let Some(img) = frontispiece_cover(path) {
        return Some(img);
    }
    let bytes = std::fs::read(game_dir?.join("cover.png")).ok()?;
    decode(&bytes)
}

/// Cache capacity: how many decoded covers `CoverState` retains before the
/// least-recently-inserted one is evicted. Sized to hold a whole screenful of
/// gallery tiles (SQ-0374) — even a very wide terminal shows well under this —
/// so scrolling/paging the cover grid never evicts a still-visible cover and
/// forces a re-decode. The list/info-panel path only ever needs one at a time.
const CAP: usize = 128;

/// How many built tile protocols `CoverState` keeps for the gallery view
/// (SQ-0374). Keyed by `(path, cols, rows)`; least-recently-used evicted first.
/// Matches `CAP` so a screenful of rasters survives alongside their images.
const TILE_CAP: usize = 128;

/// Selection-scoped cover state: a bounded LRU map of decoded images (one entry
/// per visited story; `None` records a coverless story so it isn't re-decoded),
/// a single protocol cached by `(path, cols, rows)` for the info panel, and a
/// bounded LRU of tile protocols for the cover-gallery grid (many on screen at
/// once).
#[derive(Default)]
pub struct CoverState {
    decoded: HashMap<PathBuf, Option<image::DynamicImage>>,
    order: VecDeque<PathBuf>,
    /// The trailing `Option<u32>` is the kitty image id [`place_protocol`]
    /// returned the last time this entry was placed (`None` off-kitty, or
    /// before the first placement) — [`Self::note_proto_placed`] fills it in.
    /// One id per entry: `proto` holds at most one, and each `tiles` slot is
    /// its own separate build (SQ-1190).
    ///
    /// [`place_protocol`]: crate::render::graphics::place_protocol
    proto: Option<(PathBuf, u16, u16, Protocol, Option<u32>)>,
    tiles: VecDeque<(PathBuf, u16, u16, Protocol, Option<u32>)>,
    /// Uploads an eviction/replacement here abandoned, queued for the terminal
    /// to free (SQ-1190) — this loop runs before any `AppState`/`GraphicsRender`
    /// exists, so it keeps its own queue rather than sharing that one.
    deletes: KittyDeleteQueue,
}

impl CoverState {
    /// SQ-0988: the terminal's CELL changed size, so every built protocol was
    /// fitted against the wrong aspect ratio. The decoded IMAGES are unaffected —
    /// they are pixels, not geometry — so only the rasters go, and the next draw
    /// rebuilds them from the images already in hand.
    ///
    /// Both caches are keyed `(path, cols, rows)`, which is exactly the key a
    /// font-size change cannot move: the browser can keep the same cell rect
    /// while every cell in it becomes a different box in pixels.
    ///
    /// Every dropped entry's upload is freed in the terminal, not merely
    /// forgotten (SQ-1190) — a font-size change is exactly the kind of frame
    /// this abandons wholesale.
    pub fn invalidate_cell_geometry(&mut self) {
        if let Some(old) = self.proto.take() {
            self.deletes.queue(old.4);
        }
        for t in self.tiles.drain(..) {
            self.deletes.queue(t.4);
        }
    }

    /// Flush any deletes queued by eviction/replacement into `buf` (SQ-1190),
    /// mirroring `GraphicsRender::flush_kitty_deletes` for a loop with no
    /// `GraphicsRender` of its own — see [`KittyDeleteQueue`].
    pub fn flush_kitty_deletes(&mut self, area: Rect, buf: &mut Buffer) {
        self.deletes.flush(area, buf);
    }

    /// Queue a delete for an upload owned by ANOTHER cache in the same picker
    /// loop — the resource-preview modal (`picker_ui.rs`), which has no
    /// `GraphicsRender` of its own either and would rather share this queue
    /// (already flushed every frame) than keep and flush a second one that
    /// would also lose whatever it held whenever the modal closes and its
    /// struct is dropped (SQ-1190).
    pub fn queue_external_delete(&mut self, id: Option<u32>) {
        self.deletes.queue(id);
    }

    /// Record the kitty image id [`Self::protocol`]'s caller placed it under,
    /// so a later replace/evict can free it (SQ-1190). A no-op if there is no
    /// cached entry to attach it to.
    pub fn note_proto_placed(&mut self, id: Option<u32>) {
        if let Some(entry) = self.proto.as_mut() {
            entry.4 = id;
        }
    }

    /// Record the kitty image id [`Self::tile_protocol`]'s caller placed it
    /// under — always the most-recently-used tile, since a cache hit promotes
    /// to the back and a miss pushes there (SQ-1190).
    pub fn note_tile_placed(&mut self, id: Option<u32>) {
        if let Some(entry) = self.tiles.back_mut() {
            entry.4 = id;
        }
    }

    /// True when `path` has already been decoded (`Some` or `None`) — skip the
    /// re-read/decode. A cached `None` (coverless story) still counts.
    pub fn has(&self, path: &Path) -> bool {
        self.decoded.contains_key(path)
    }

    /// Record the decode result for `path` (`Some(img)` or a coverless `None`)
    /// in the LRU cache, evicting the oldest entry once capacity is exceeded.
    ///
    /// Re-inserting an existing path refreshes its recency without duplicating it
    /// in `order` (so `order` stays 1:1 with `decoded` — no leak, no premature
    /// eviction of a live key). A replaced image also drops any stale built
    /// protocol for that path so `protocol()` rebuilds from the new image.
    pub fn insert(&mut self, path: PathBuf, img: Option<image::DynamicImage>) {
        if self.decoded.insert(path.clone(), img).is_some() {
            // Existing key: move it to most-recent, and invalidate a stale raster
            // (both the info-panel proto and any gallery tiles for this path),
            // freeing each dropped upload in the terminal (SQ-1190).
            self.order.retain(|p| p != &path);
            if matches!(&self.proto, Some((p, _, _, _, _)) if *p == path) {
                let old = self.proto.take();
                self.deletes.queue(old.and_then(|t| t.4));
            }
            let stale_tiles: Vec<Option<u32>> =
                self.tiles.iter().filter(|(p, ..)| *p == path).map(|(.., id)| *id).collect();
            self.tiles.retain(|(p, ..)| *p != path);
            for id in stale_tiles {
                self.deletes.queue(id);
            }
        }
        self.order.push_back(path);
        while self.decoded.len() > CAP {
            match self.order.pop_front() {
                Some(oldest) => { self.decoded.remove(&oldest); }
                None => break,
            }
        }
    }

    /// Drop any cached decode (and built protocol) for `path`, so the next
    /// request re-reads and re-decodes it. Used after a fetch writes a
    /// `cover.png` for a story previously cached as coverless (`None`) —
    /// without this the stale `None` would hide the freshly fetched cover
    /// until the picker is reopened (SQ-0348).
    pub fn forget(&mut self, path: &Path) {
        if self.decoded.remove(path).is_some() {
            self.order.retain(|p| p != path);
        }
        if matches!(&self.proto, Some((p, _, _, _, _)) if p == path) {
            let old = self.proto.take();
            self.deletes.queue(old.and_then(|t| t.4));
        }
        let stale_tiles: Vec<Option<u32>> =
            self.tiles.iter().filter(|(p, ..)| p == path).map(|(.., id)| *id).collect();
        self.tiles.retain(|(p, ..)| p != path);
        for id in stale_tiles {
            self.deletes.queue(id);
        }
    }

    /// Build-or-reuse a protocol for `path`'s cover, fitted (aspect-preserved)
    /// into `area`. `None` when `path` has no decoded cover or the build fails.
    ///
    /// While `animating` is true and a protocol for `path` is already cached
    /// (at any size), that stale raster is reused rather than rebuilt — the
    /// panel width changes every frame during a slide, and re-resizing the
    /// image on each tick is expensive for no visible benefit mid-motion. The
    /// geometry catches up on the next non-animating (settled) frame.
    pub fn protocol(
        &mut self,
        picker: &Picker,
        path: &Path,
        area: Rect,
        animating: bool,
    ) -> Option<&Protocol> {
        let img = self.decoded.get(path).and_then(|o| o.as_ref())?;
        let cached_for_path = matches!(&self.proto, Some((p, _, _, _, _)) if p == path);
        if animating && cached_for_path {
            return self.proto.as_ref().map(|(_, _, _, p, _)| p);
        }
        let fresh = matches!(
            &self.proto,
            Some((p, w, h, _, _)) if p == path && *w == area.width && *h == area.height
        );
        if !fresh {
            // Direction-aware + alpha-correct, then an identity `Fit` (SQ-0829): a
            // cover is a full-resolution jacket scan being reduced several-fold into
            // a panel, and the crate's default filter for `Fit(None)` is Nearest.
            // On half-blocks that pre-scale is a device-pixel intermediate the
            // backend throws away, so there the reduction is one pass (SQ-0979).
            let built = crate::render::graphics::fitted_protocol(
                picker,
                img,
                Size::new(area.width, area.height),
                false,
            )?;
            // Freed only once the rebuild has actually succeeded — the `?` above
            // must leave a surviving entry (and its terminal upload) alone
            // (SQ-1190).
            let old = self.proto.take();
            self.deletes.queue(old.and_then(|t| t.4));
            self.proto = Some((path.to_path_buf(), area.width, area.height, built, None));
        }
        self.proto.as_ref().map(|(_, _, _, p, _)| p)
    }

    /// Build-or-reuse a gallery-tile protocol for `path`'s cover, fitted into
    /// `area`. Unlike [`protocol`], many of these coexist (one per visible tile),
    /// so they live in a bounded LRU keyed by `(path, cols, rows)` rather than a
    /// single slot. `None` when `path` has no decoded cover or the build fails.
    ///
    /// [`protocol`]: Self::protocol
    pub fn tile_protocol(
        &mut self,
        picker: &Picker,
        path: &Path,
        area: Rect,
    ) -> Option<&Protocol> {
        if let Some(pos) = self
            .tiles
            .iter()
            .position(|(p, w, h, _, _)| p == path && *w == area.width && *h == area.height)
        {
            // Cache hit: promote to most-recently-used and hand it back. Same
            // `Protocol`, so the id `note_tile_placed` already recorded is still
            // right — nothing to free here.
            let entry = self.tiles.remove(pos).unwrap();
            self.tiles.push_back(entry);
            return self.tiles.back().map(|(_, _, _, p, _)| p);
        }
        let img = self.decoded.get(path).and_then(|o| o.as_ref())?;
        // Same reduction as [`protocol`], only harder: a gallery tile is smaller
        // still, so Nearest was discarding a larger share of every jacket (SQ-0829)
        // — and this is the heaviest site in the app for it, one build per visible
        // tile and a row of them on every scroll notch, which is why half-blocks
        // stopped building a device-pixel intermediate per tile (SQ-0979).
        let built = crate::render::graphics::fitted_protocol(
            picker,
            img,
            Size::new(area.width, area.height),
            false,
        )?;
        self.tiles.push_back((path.to_path_buf(), area.width, area.height, built, None));
        // LRU eviction beyond capacity frees each dropped tile's upload, not
        // merely the struct that named it (SQ-1190).
        while self.tiles.len() > TILE_CAP {
            if let Some(evicted) = self.tiles.pop_front() {
                self.deletes.queue(evicted.4);
            }
        }
        self.tiles.back().map(|(_, _, _, p, _)| p)
    }

    /// The aspect-fitted, centred sub-rect of `area` for `path`'s cover, computed
    /// from the image's pixel dimensions and the picker's cell size. Building the
    /// protocol at — and rendering into — this rect centres the cover on BOTH axes
    /// regardless of how a given render protocol reports its own size. Returns
    /// `area` unchanged when the cover isn't decoded.
    pub fn fitted_tile_rect(&self, picker: &Picker, path: &Path, area: Rect) -> Rect {
        let Some(img) = self.decoded.get(path).and_then(|o| o.as_ref()) else {
            return area;
        };
        let fs = picker.font_size();
        let (fw, fh) = (fs.width.max(1) as f32, fs.height.max(1) as f32);
        let (iw, ih) = (img.width().max(1) as f32, img.height().max(1) as f32);
        let scale = (area.width as f32 * fw / iw).min(area.height as f32 * fh / ih);
        let cols = ((iw * scale / fw).round() as u16).clamp(1, area.width);
        let rows = ((ih * scale / fh).round() as u16).clamp(1, area.height);
        Rect::new(
            area.x + (area.width - cols) / 2,
            area.y + (area.height - rows) / 2,
            cols,
            rows,
        )
    }
}

/// Background cover decoder: owns one long-lived worker thread that runs
/// `load_cover` off the main loop. Requests are queued on `req_tx`; decoded
/// results are drained (non-blocking) from `res_rx`. The worker exits cleanly
/// when the `CoverDecoder` is dropped (dropping `req_tx` makes the worker's
/// `recv()` err, ending its loop).
pub struct CoverDecoder {
    req_tx: std::sync::mpsc::Sender<(PathBuf, PathBuf)>,
    res_rx: std::sync::mpsc::Receiver<(PathBuf, Option<image::DynamicImage>)>,
    _worker: std::thread::JoinHandle<()>,
}

impl CoverDecoder {
    pub fn new() -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<(PathBuf, PathBuf)>();
        let (res_tx, res_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            while let Ok((path, game_dir)) = req_rx.recv() {
                // ends when req_tx drops (picker exits)
                let img = load_cover(&path, Some(&game_dir));
                if res_tx.send((path, img)).is_err() {
                    break;
                }
            }
        });
        Self { req_tx, res_rx, _worker: worker }
    }

    /// Queue `path` for background decoding, with `game_dir` as the fetched-cover
    /// fallback source when `path` has no `Fspc` of its own. Silently dropped if
    /// the worker has already exited.
    pub fn request(&self, path: PathBuf, game_dir: PathBuf) {
        let _ = self.req_tx.send((path, game_dir));
    }

    /// Non-blocking drain of all decoded results ready so far.
    pub fn drain(&self) -> impl Iterator<Item = (PathBuf, Option<image::DynamicImage>)> + '_ {
        self.res_rx.try_iter()
    }
}

impl Default for CoverDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Pure debounce predicate: request a cover only when it isn't already cached,
/// isn't already in flight, and the selection has been settled at least
/// `debounce` (avoids decoding covers you scroll straight past).
pub fn should_request_cover(
    cached: bool,
    requested: bool,
    since_change: std::time::Duration,
    debounce: std::time::Duration,
) -> bool {
    !cached && !requested && since_change >= debounce
}

#[cfg(test)]
mod tests {
    /// A GIF cover decodes: IFDB and the IFComp archive serve some covers as
    /// GIF, and every one of them was dropped before the decoder was enabled.
    #[test]
    fn gif_covers_decode() {
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 200, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Gif).unwrap();
        let bytes = out.into_inner();
        assert_eq!(&bytes[..6], b"GIF89a");
        let decoded = super::decode(&bytes).expect("a GIF decodes");
        assert_eq!((decoded.width(), decoded.height()), (3, 2));
    }

    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// A solid-color 2x2 PNG, encoded via the `image` crate.
    fn png_bytes_colored(rgb: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb(rgb));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// A minimal, structurally valid blorb (`RIdx` with zero entries, no
    /// `Fspc`) — a story that carries no frontispiece of its own, so the
    /// fetched-cover fallback is the only source.
    fn minimal_blorb_no_fspc() -> Vec<u8> {
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        let ridx_body = 0u32.to_be_bytes(); // count = 0
        inner.extend_from_slice(b"RIdx");
        inner.extend_from_slice(&(ridx_body.len() as u32).to_be_bytes());
        inner.extend_from_slice(&ridx_body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    /// A blorb declaring its own `Fspc` frontispiece pointing at a `Pict`
    /// resource holding `png`. Mirrors `fetch_worker::tests::blorb_with_fspc_and_cover`
    /// (duplicated here, test-only, to keep this module's fixtures self-contained).
    fn blorb_with_fspc(png: &[u8]) -> Vec<u8> {
        blorb_with_fspc_naming(png, b"Pict", 7, 7)
    }

    /// [`blorb_with_fspc`] with the dangling cases spelled out: the single
    /// resource is indexed under `usage`/`res_number`, and the `Fspc` chunk
    /// names `fspc_number`. Pass a `fspc_number` no resource has, or a `usage`
    /// that isn't `Pict`, to build a container whose frontispiece cannot
    /// resolve — the case the Blorb spec does not legislate for (SQ-0985).
    fn blorb_with_fspc_naming(
        png: &[u8],
        usage: &[u8; 4],
        res_number: u32,
        fspc_number: u32,
    ) -> Vec<u8> {
        fn iff_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // count
        ridx.extend_from_slice(usage);
        ridx.extend_from_slice(&res_number.to_be_bytes()); // number
        let ridx_chunk_len = 8 + (4 + 12);
        let fspc_chunk_len = 8 + 4;
        let pict_off = 12 + ridx_chunk_len + fspc_chunk_len;
        ridx.extend_from_slice(&(pict_off as u32).to_be_bytes()); // start
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&iff_chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&iff_chunk(b"Fspc", &fspc_number.to_be_bytes()));
        inner.extend_from_slice(&iff_chunk(b"PNG ", png));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    /// A valid 2x2 red PNG, encoded via the `image` crate.
    fn png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn decode_accepts_png_rejects_garbage() {
        assert!(decode(&png_bytes()).is_some());
        assert!(decode(b"not an image").is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn cover_state_caches_by_path_and_builds_protocol() {
        let mut st = CoverState::default();
        let path = Path::new("game.gblorb");
        assert!(!st.has(path));

        st.insert(path.to_path_buf(), decode(&png_bytes()));
        assert!(st.has(path));

        // halfblocks() needs no terminal query — deterministic in tests.
        let picker = ratatui_image::picker::Picker::halfblocks();
        let area = ratatui::layout::Rect::new(0, 0, 10, 5);
        assert!(st.protocol(&picker, path, area, false).is_some());

        // A different path has no cover until inserted.
        let other = Path::new("other.gblorb");
        assert!(!st.has(other));
        assert!(st.protocol(&picker, other, area, false).is_none());
    }

    #[test]
    fn insert_evicts_oldest_beyond_cap() {
        let mut st = CoverState::default();
        // Insert CAP + 3 distinct paths; the 3 oldest must be evicted.
        let paths: Vec<PathBuf> =
            (0..CAP + 3).map(|i| PathBuf::from(format!("game{i}.gblorb"))).collect();
        for p in &paths {
            st.insert(p.clone(), decode(&png_bytes()));
        }
        assert_eq!(st.decoded.len(), CAP, "cache is bounded to CAP");
        // Oldest 3 evicted.
        for p in &paths[..3] {
            assert!(!st.has(p), "oldest entries should be evicted");
        }
        // Newest present (the just-inserted current is never evicted).
        for p in &paths[3..] {
            assert!(st.has(p), "recent entries should remain cached");
        }
    }

    #[test]
    fn reinsert_refreshes_recency_without_corrupting_lru() {
        let mut st = CoverState::default();
        // Fill exactly to CAP.
        let paths: Vec<PathBuf> =
            (0..CAP).map(|i| PathBuf::from(format!("game{i}.gblorb"))).collect();
        for p in &paths {
            st.insert(p.clone(), decode(&png_bytes()));
        }
        // Re-insert the OLDEST (game0) — it must move to most-recent, and `order`
        // must not gain a duplicate (else a later eviction would drop a live key
        // and `order`/`decoded` would diverge).
        st.insert(paths[0].clone(), decode(&png_bytes()));
        assert_eq!(st.decoded.len(), CAP, "re-insert must not change the count");
        assert_eq!(st.order.len(), CAP, "order must stay 1:1 with decoded (no dup)");

        // Now insert one NEW path: the oldest survivor (game1, since game0 was
        // refreshed) is evicted — NOT the just-refreshed game0.
        st.insert(PathBuf::from("new.gblorb"), decode(&png_bytes()));
        assert_eq!(st.decoded.len(), CAP);
        assert!(st.has(&paths[0]), "refreshed key must survive eviction");
        assert!(!st.has(&paths[1]), "the genuine oldest must be evicted");
        assert!(st.has(Path::new("new.gblorb")));
    }

    #[test]
    fn tile_protocol_caches_multiple_covers_at_once() {
        // The gallery needs several covers rastered simultaneously — unlike the
        // single-slot info-panel proto, tile protocols coexist.
        let mut st = CoverState::default();
        let picker = ratatui_image::picker::Picker::halfblocks();
        let area = Rect::new(0, 0, 16, 8);
        let a = Path::new("a.gblorb");
        let b = Path::new("b.gblorb");
        st.insert(a.to_path_buf(), decode(&png_bytes()));
        st.insert(b.to_path_buf(), decode(&png_bytes()));

        assert!(st.tile_protocol(&picker, a, area).is_some());
        assert!(st.tile_protocol(&picker, b, area).is_some());
        // Both remain cached (2 distinct tiles held at once).
        assert_eq!(st.tiles.len(), 2);
        // A coverless / undecoded path yields nothing.
        assert!(st.tile_protocol(&picker, Path::new("missing.gblorb"), area).is_none());
    }

    #[test]
    fn tile_protocol_dropped_when_image_is_replaced_or_forgotten() {
        let mut st = CoverState::default();
        let picker = ratatui_image::picker::Picker::halfblocks();
        let area = Rect::new(0, 0, 16, 8);
        let p = Path::new("game.gblorb");

        st.insert(p.to_path_buf(), decode(&png_bytes()));
        assert!(st.tile_protocol(&picker, p, area).is_some());
        assert_eq!(st.tiles.len(), 1);

        // Re-decoding the same path (e.g. after a fetch writes a new cover)
        // must invalidate its stale tile raster.
        st.insert(p.to_path_buf(), decode(&png_bytes()));
        assert_eq!(st.tiles.len(), 0, "replacing the image drops its tile raster");

        st.tile_protocol(&picker, p, area);
        assert_eq!(st.tiles.len(), 1);
        st.forget(p);
        assert_eq!(st.tiles.len(), 0, "forget drops the tile raster too");
    }

    /// A real kitty picker so `place_protocol` has an id to hand back — the
    /// tests above use `halfblocks()`, which never names one and so cannot
    /// exercise the delete path at all.
    fn test_kitty_picker() -> Picker {
        crate::render::graphics::kitty_picker(8, 16)
    }

    /// Build (or reuse) `path`'s tile and record the id `place_protocol`
    /// returns for it, exactly as `picker_ui.rs`'s gallery draw does.
    fn place_tile(st: &mut CoverState, picker: &Picker, path: &Path, area: Rect, buf: &mut Buffer) -> Option<u32> {
        let proto = st.tile_protocol(picker, path, area)?;
        let id = crate::render::graphics::place_protocol(proto, area, buf);
        st.note_tile_placed(id);
        id
    }

    /// SQ-1190: an image replaced under the SAME path (`insert` re-decoding a
    /// freshly fetched cover) must free the tile upload it drops, not merely
    /// forget it — `tile_protocol_dropped_when_image_is_replaced_or_forgotten`
    /// above only checks the cache count, which is silent about the terminal
    /// leak. Falsified by reverting the `self.deletes.queue(id)` calls this
    /// quest added to `insert`/`forget`: the flush below then writes nothing.
    #[test]
    fn replacing_or_forgetting_a_cover_queues_a_delete_for_its_tile_upload() {
        let mut st = CoverState::default();
        let picker = test_kitty_picker();
        let area = Rect::new(0, 0, 4, 4);
        let p = Path::new("game.gblorb");
        let mut buf = Buffer::empty(area);

        st.insert(p.to_path_buf(), decode(&png_bytes()));
        let id = place_tile(&mut st, &picker, p, area, &mut buf).expect("kitty picker names an id");

        // Re-decoding the same path drops the stale tile raster — and must
        // queue its upload for deletion.
        st.insert(p.to_path_buf(), decode(&png_bytes()));
        let mut flush_buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        st.flush_kitty_deletes(Rect::new(0, 0, 4, 1), &mut flush_buf);
        let cell = flush_buf.cell((0, 0)).expect("a plain cell to carry the delete");
        assert!(
            cell.symbol().contains(crate::render::graphics::kitty_delete_escape(id).as_str()),
            "insert() must queue a delete for the replaced tile's upload"
        );

        // Same for `forget`.
        let id2 = place_tile(&mut st, &picker, p, area, &mut buf).expect("kitty picker names an id");
        st.forget(p);
        let mut flush_buf2 = Buffer::empty(Rect::new(0, 0, 4, 1));
        st.flush_kitty_deletes(Rect::new(0, 0, 4, 1), &mut flush_buf2);
        let cell2 = flush_buf2.cell((0, 0)).expect("a plain cell to carry the delete");
        assert!(
            cell2.symbol().contains(crate::render::graphics::kitty_delete_escape(id2).as_str()),
            "forget() must queue a delete for the forgotten tile's upload"
        );
    }

    /// SQ-1190: the gallery-tile LRU's capacity eviction must free the oldest
    /// tile's upload, not merely pop the struct that named it — kitty evicts
    /// by ITS OWN LRU too and evicts images that are currently placed
    /// (`GraphicsRender`'s SQ-0753 comment), so an unbounded pile of orphaned
    /// gallery-tile uploads can blank a tile still on screen.
    ///
    /// Falsified by reverting the `self.deletes.queue(evicted.4)` call in
    /// `tile_protocol`'s eviction loop: the flush below then writes nothing.
    #[test]
    fn tile_lru_eviction_queues_a_delete_for_the_evicted_upload() {
        let mut st = CoverState::default();
        let picker = test_kitty_picker();
        let area = Rect::new(0, 0, 4, 4);
        let mut buf = Buffer::empty(area);

        let first = PathBuf::from("game0.gblorb");
        st.insert(first.clone(), decode(&png_bytes()));
        let id = place_tile(&mut st, &picker, &first, area, &mut buf).expect("kitty picker names an id");

        // Push TILE_CAP more distinct tiles: the first push past capacity
        // evicts `first` (the oldest), and the LRU stays at capacity after.
        for i in 1..=TILE_CAP {
            let path = PathBuf::from(format!("game{i}.gblorb"));
            st.insert(path.clone(), decode(&png_bytes()));
            place_tile(&mut st, &picker, &path, area, &mut buf);
        }
        assert_eq!(st.tiles.len(), TILE_CAP, "capacity is enforced");
        assert!(st.tiles.iter().all(|(p, ..)| p != &first), "the oldest tile was evicted");

        let mut flush_buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        st.flush_kitty_deletes(Rect::new(0, 0, 4, 1), &mut flush_buf);
        let cell = flush_buf.cell((0, 0)).expect("a plain cell to carry the delete");
        assert!(
            cell.symbol().contains(crate::render::graphics::kitty_delete_escape(id).as_str()),
            "LRU eviction must queue a delete for the evicted tile's upload"
        );
    }

    #[test]
    fn none_is_cached_and_not_redecoded() {
        let mut st = CoverState::default();
        let path = Path::new("coverless.z5");
        st.insert(path.to_path_buf(), None);
        // A cached `None` still counts as "known" so it isn't re-requested.
        assert!(st.has(path));
        // ...and yields no protocol without touching the disk.
        let picker = ratatui_image::picker::Picker::halfblocks();
        let area = ratatui::layout::Rect::new(0, 0, 10, 5);
        assert!(st.protocol(&picker, path, area, false).is_none());
    }

    /// A large (300x300) solid PNG — big enough that `Resize::Fit` actually
    /// scales it down differently for different target areas (unlike the tiny
    /// 2x2 `png_bytes()` fixture, which is already smaller than any halfblocks
    /// cell box and so never changes fitted size regardless of area).
    fn large_png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(300, 300, image::Rgb([0, 255, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn protocol_reuses_cached_raster_when_animating() {
        let mut st = CoverState::default();
        let path = Path::new("game.gblorb");
        st.insert(path.to_path_buf(), decode(&large_png_bytes()));

        let picker = ratatui_image::picker::Picker::halfblocks();

        let size_a = st
            .protocol(&picker, path, Rect::new(0, 0, 10, 6), false)
            .unwrap()
            .size();

        // Different area, but animating: reuse the stale raster, no rebuild.
        let size_animating = st
            .protocol(&picker, path, Rect::new(0, 0, 20, 10), true)
            .unwrap()
            .size();
        assert_eq!(size_animating, size_a, "animating reuse should not resize");

        // Same area, not animating: settle by rebuilding at the new size.
        let size_settled = st
            .protocol(&picker, path, Rect::new(0, 0, 20, 10), false)
            .unwrap()
            .size();
        assert_ne!(size_settled, size_a, "settled frame should rebuild at the new area");
    }

    #[test]
    fn fitted_tile_rect_centers_portrait_and_landscape() {
        let mut st = CoverState::default();
        let picker = ratatui_image::picker::Picker::halfblocks();
        let area = Rect::new(0, 0, 20, 11);

        // Portrait (tall/narrow): fills the height, centred horizontally.
        let pp = Path::new("portrait.png");
        let p_img = image::RgbImage::from_pixel(100, 300, image::Rgb([0, 0, 255]));
        let mut pb = Vec::new();
        image::DynamicImage::ImageRgb8(p_img)
            .write_to(&mut std::io::Cursor::new(&mut pb), image::ImageFormat::Png)
            .unwrap();
        st.insert(pp.to_path_buf(), decode(&pb));
        let fp = st.fitted_tile_rect(&picker, pp, area);
        assert!(fp.width < area.width, "portrait fits narrower than the tile: {fp:?}");
        let (lm, rm) = (fp.x - area.x, area.right() - fp.right());
        assert!(lm >= 1 && rm >= 1 && lm.abs_diff(rm) <= 1, "portrait centred horizontally: lm={lm} rm={rm}");

        // Landscape (short/wide): fills the width, centred vertically.
        let lp = Path::new("landscape.png");
        let l_img = image::RgbImage::from_pixel(300, 100, image::Rgb([0, 255, 0]));
        let mut lb = Vec::new();
        image::DynamicImage::ImageRgb8(l_img)
            .write_to(&mut std::io::Cursor::new(&mut lb), image::ImageFormat::Png)
            .unwrap();
        st.insert(lp.to_path_buf(), decode(&lb));
        let fl = st.fitted_tile_rect(&picker, lp, area);
        assert!(fl.height < area.height, "landscape fits shorter than the tile: {fl:?}");
        let (tm, bm) = (fl.y - area.y, area.bottom() - fl.bottom());
        assert!(tm >= 1 && bm >= 1 && tm.abs_diff(bm) <= 1, "landscape centred vertically: tm={tm} bm={bm}");
    }

    /// Set up a temp story file + its `<key>.save/` game dir, cleaned up by the
    /// caller via the returned dir's parent.
    fn temp_story_and_game_dir(name: &str, story_bytes: &[u8]) -> (PathBuf, PathBuf) {
        let base = crate::scratch_dir(&format!("cover-fallback-{name}"));
        let story_path = base.join("game.gblorb");
        std::fs::write(&story_path, story_bytes).unwrap();
        let game_dir = base.join("game.gblorb.save");
        std::fs::create_dir_all(&game_dir).unwrap();
        (story_path, game_dir)
    }

    #[test]
    fn load_cover_falls_back_to_fetched_cover_png_when_no_frontispiece() {
        let (story_path, game_dir) =
            temp_story_and_game_dir("fallback", &minimal_blorb_no_fspc());
        std::fs::write(game_dir.join("cover.png"), png_bytes_colored([1, 2, 3])).unwrap();

        // No fallback source offered: nothing to show.
        assert!(load_cover(&story_path, None).is_none(), "no Fspc and no fallback source");

        // Fallback source offered: the fetched cover.png is used.
        let img = load_cover(&story_path, Some(&game_dir)).expect("fetched cover should load");
        let px = img.to_rgb8().get_pixel(0, 0).0;
        assert_eq!(px, [1, 2, 3], "fallback cover's pixels should decode");

        let _ = std::fs::remove_dir_all(story_path.parent().unwrap());
    }

    #[test]
    fn load_cover_prefers_its_own_frontispiece_over_a_fetched_cover_png() {
        let own = png_bytes_colored([200, 50, 50]);
        let fetched = png_bytes_colored([1, 2, 3]);
        let (story_path, game_dir) = temp_story_and_game_dir("precedence", &blorb_with_fspc(&own));
        std::fs::write(game_dir.join("cover.png"), &fetched).unwrap();

        let img = load_cover(&story_path, Some(&game_dir)).expect("own frontispiece should load");
        let px = img.to_rgb8().get_pixel(0, 0).0;
        assert_eq!(px, [200, 50, 50], "the story's own Fspc must win over a fetched cover.png");

        let _ = std::fs::remove_dir_all(story_path.parent().unwrap());
    }

    /// An `Fspc` naming a Pict that isn't in the index is a container the Blorb
    /// spec does not legislate for — it says the chunk holds the "number of a
    /// Pict resource" and stops there. The picker's only sane reading is *no
    /// cover*, silently, and specifically **not** a claim on the cover slot:
    /// the fetched sidecar must still get its turn, exactly as if the broken
    /// chunk weren't there (SQ-0985).
    #[test]
    fn a_dangling_fspc_falls_through_to_the_fetched_cover() {
        // Indexed as Pict 7; the Fspc names Pict 9, which does not exist.
        let blorb = blorb_with_fspc_naming(&png_bytes_colored([200, 50, 50]), b"Pict", 7, 9);
        let (story_path, game_dir) = temp_story_and_game_dir("dangling", &blorb);
        std::fs::write(game_dir.join("cover.png"), png_bytes_colored([1, 2, 3])).unwrap();

        assert!(load_cover(&story_path, None).is_none(), "an unresolvable Fspc is no cover");
        let img = load_cover(&story_path, Some(&game_dir)).expect("the sidecar still applies");
        assert_eq!(
            img.to_rgb8().get_pixel(0, 0).0,
            [1, 2, 3],
            "a dangling Fspc must not shadow the fetched cover.png"
        );

        let _ = std::fs::remove_dir_all(story_path.parent().unwrap());
    }

    /// `Fspc` names a **Pict** resource. A container whose only resource with
    /// that number is a sound resolves to no cover — `Blorb::resource` matches
    /// on usage as well as number, so the picture lookup simply misses, and it
    /// must miss rather than decode whatever bytes happen to sit there.
    #[test]
    fn an_fspc_naming_a_non_pict_resource_is_no_cover() {
        let blorb = blorb_with_fspc_naming(&png_bytes_colored([200, 50, 50]), b"Snd ", 7, 7);
        let (story_path, _game_dir) = temp_story_and_game_dir("wrong-usage", &blorb);

        assert!(
            load_cover(&story_path, None).is_none(),
            "Fspc resolves through Pict; a Snd of the same number is not the frontispiece"
        );

        let _ = std::fs::remove_dir_all(story_path.parent().unwrap());
    }

    #[test]
    fn decoder_round_trips_a_non_blorb_as_none() {
        // A real file that isn't a blorb: `load_cover` returns `None`, so the
        // worker delivers `(path, None)` — exercises spawn → request → decode →
        // deliver with no cover fixture needed.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("lanthorn-cover-test-{}.txt", std::process::id()));
        std::fs::write(&path, b"not a blorb").unwrap();

        let d = CoverDecoder::new();
        d.request(path.clone(), dir.join("no-such-game-dir"));

        let mut got = None;
        // Bounded poll: worker is near-instant, but don't spin forever.
        for _ in 0..1000 {
            if let Some(res) = d.drain().next() {
                got = Some(res);
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = std::fs::remove_file(&path);

        let (rp, img) = got.expect("worker should deliver a result");
        assert_eq!(rp, path);
        assert!(img.is_none(), "a non-blorb has no cover");
    }

    #[test]
    fn should_request_cover_truth_table() {
        let zero = Duration::ZERO;
        let debounce = Duration::from_millis(100);
        let past = Duration::from_millis(150);

        // Not cached, not requested, debounce elapsed → request.
        assert!(should_request_cover(false, false, past, debounce));
        // Time gate: not yet debounced → hold off.
        assert!(!should_request_cover(false, false, zero, debounce));
        // Already cached → never request.
        assert!(!should_request_cover(true, false, past, debounce));
        // Already in flight → never re-request.
        assert!(!should_request_cover(false, true, past, debounce));
        // Boundary: exactly at the debounce is enough (>=).
        assert!(should_request_cover(false, false, debounce, debounce));
    }

    /// SQ-0988: a cover is aspect-fitted against the terminal's CELL, so the same
    /// rect on the same image lands differently once the cell changes shape.
    ///
    /// 4x7 and 4x9 are FiraCode at 6 px and 7 px — real cells of 1.750 and 2.250
    /// from a face whose design ratio is 2.002, because the width and the height
    /// round at different rates. If these two fits were equal there would be
    /// nothing for the browser's resize hook to invalidate.
    #[test]
    fn the_same_area_fits_a_different_cover_rect_once_the_cell_changes_shape() {
        #[allow(deprecated)]
        fn picker(w: u16, h: u16) -> Picker {
            Picker::from_fontsize(ratatui_image::FontSize::new(w, h))
        }
        let path = Path::new("/cover.png");
        let mut state = CoverState::default();
        state.insert(
            path.to_path_buf(),
            Some(image::DynamicImage::ImageRgba8(image::RgbaImage::new(600, 800))),
        );
        let area = Rect::new(0, 0, 30, 20);
        let tall = state.fitted_tile_rect(&picker(4, 7), path, area);
        let taller = state.fitted_tile_rect(&picker(4, 9), path, area);
        assert_ne!(
            (tall.width, tall.height),
            (taller.width, taller.height),
            "a 600x800 cover in 30x20 cells fits {}x{} at a 1.750 cell and {}x{} at a 2.250 one",
            tall.width,
            tall.height,
            taller.width,
            taller.height
        );
    }

    /// And the built rasters are keyed in CELLS, which a font change does not
    /// move — so they have to be dropped explicitly. The decoded IMAGE stays:
    /// it is pixels, not geometry, and re-reading it from disk would be the
    /// browser stuttering for no reason.
    #[test]
    fn invalidating_the_cell_geometry_drops_the_rasters_and_keeps_the_decoded_image() {
        let path = Path::new("/cover.png");
        let mut state = CoverState::default();
        state.insert(
            path.to_path_buf(),
            Some(image::DynamicImage::ImageRgba8(image::RgbaImage::new(60, 80))),
        );
        #[allow(deprecated)]
        let picker = Picker::from_fontsize(ratatui_image::FontSize::new(8, 18));
        let area = Rect::new(0, 0, 6, 4);
        assert!(state.protocol(&picker, path, area, false).is_some(), "the fixture must build a raster");
        assert!(state.tile_protocol(&picker, path, area).is_some(), "…and a gallery tile");

        state.invalidate_cell_geometry();
        assert!(state.has(path), "the decode survives — only the geometry was wrong");
        assert_eq!(
            (state.proto.is_some(), state.tiles.len()),
            (false, 0),
            "every raster was fitted to the old cell and must be rebuilt"
        );
    }
}
