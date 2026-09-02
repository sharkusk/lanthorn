//! Cover-art decoding + terminal-protocol caching for the story picker.
//!
//! `load_cover` pulls a blorb's `Fspc` frontispiece image and decodes it,
//! falling back to a fetched `cover.png` sidecar (SQ-0348) when the story has
//! none of its own; `CoverState` holds a bounded LRU cache of decoded images
//! and lazily builds
//! (and caches) a `ratatui-image` protocol scaled to the panel's cover region
//! for the currently-selected story. `CoverDecoder` owns a background worker
//! thread that runs `load_cover` off the main loop so scrolling never stalls,
//! and `TileEncoder` (SQ-1199) owns a second one that does the same for the
//! gallery grid's per-tile resize + protocol encode.
//! Every failure resolves to `None` — the picker simply shows no cover.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// Byte budget for `CoverState::decoded`: the sum of every cached decoded
/// cover's pixel-buffer size (`DynamicImage::as_bytes().len()`), evicting the
/// least-recently-used entry until the total is back within budget after each
/// insert (SQ-1195). Replaces a count-based cap (128 entries) that bounded
/// nothing about actual memory: a decoded jacket's size depends on the source
/// image, and 128 of the largest real ones would run to hundreds of MB.
///
/// Sized from measurement, not guesswork — a scan of every cover in
/// `stories/` (`cover::scratch_measure`, since removed) found decoded sizes
/// from a few hundred KB up to 4 MiB (`Toby's Nose.gblorb`, 1024×1024 RGBA),
/// and the gallery shows at most `TILE_CAP` (128) tiles on one screen
/// (SQ-0374). 96 MiB holds a full gallery screen of typical (~1 MiB) covers
/// with headroom for several of the largest ones seen, while capping
/// worst-case memory to under a third of the old count cap's.
const COVER_BYTE_BUDGET: usize = 96 * 1024 * 1024;

/// How many built tile protocols `CoverState` keeps for the gallery view
/// (SQ-0374). Keyed by `(path, cols, rows)`; least-recently-used evicted first.
/// A tile raster is fitted to its small on-screen cell box, not the source
/// image, so its footprint doesn't scale with jacket resolution the way a
/// decoded cover's does — a count cap is the right bound for this cache, sized
/// to a screenful of gallery tiles so scrolling never evicts a still-visible
/// one.
const TILE_CAP: usize = 128;

/// Selection-scoped cover state: a byte-budgeted LRU map of decoded images (one
/// entry per visited story; `None` records a coverless story so it isn't
/// re-decoded), a single protocol cached by `(path, cols, rows)` for the info
/// panel, and a bounded LRU of tile protocols for the cover-gallery grid (many
/// on screen at once).
#[derive(Default)]
pub struct CoverState {
    /// The decoded image is behind an `Arc` (SQ-1199) so a gallery tile's
    /// resize + encode can be handed to [`TileEncoder`]'s worker without
    /// copying a jacket that runs to megabytes — the cache keeps its own
    /// reference and the worker borrows a second one for the length of one
    /// encode. Nothing mutates a decoded cover in place, so sharing it is free.
    decoded: HashMap<PathBuf, Option<Arc<image::DynamicImage>>>,
    order: VecDeque<PathBuf>,
    /// Running total of `decoded`'s pixel-buffer bytes (`None` entries count as
    /// 0) — kept alongside `decoded` rather than recomputed, since summing every
    /// entry's `as_bytes().len()` on every insert would be the same O(n) cost
    /// the LRU eviction already pays, just for a number `insert` needs to check
    /// against [`COVER_BYTE_BUDGET`] before it can decide whether to evict.
    decoded_bytes: usize,
    /// The trailing `Option<u32>` is the kitty image id [`place_protocol`]
    /// returned the last time this entry was placed (`None` off-kitty, or
    /// before the first placement) — [`Self::note_proto_placed`] fills it in.
    /// One id per entry: `proto` holds at most one, and each `tiles` slot is
    /// its own separate build (SQ-1190).
    ///
    /// [`place_protocol`]: crate::render::graphics::place_protocol
    proto: Option<(PathBuf, u16, u16, Protocol, Option<u32>)>,
    tiles: VecDeque<TileEntry>,
    /// Uploads an eviction/replacement here abandoned, queued for the terminal
    /// to free (SQ-1190) — this loop runs before any `AppState`/`GraphicsRender`
    /// exists, so it keeps its own queue rather than sharing that one.
    deletes: KittyDeleteQueue,
}

/// Everything that decides what a gallery tile's raster IS (SQ-1199): whose
/// cover, the aspect-fitted box it is built for, and the terminal cell that box
/// was measured in. It is both the tile cache's key and the encode request's,
/// so a response can be matched back to the layout that asked for it.
///
/// The cell is in the key because it is the tile grid's whole geometry
/// generation: a tile's cover band is `TILE_W x TILE_COVER_H` **constants**
/// (`cover_gallery`), so the only thing that can move the fitted box for a
/// given jacket is the cell changing shape — which is exactly what
/// [`CoverState::invalidate_cell_geometry`] already drops the built rasters
/// for (SQ-0988). Carrying it means a reply that was already in flight when
/// the font size moved can be recognised as stale and dropped, instead of
/// landing in the cache fitted to a cell that no longer exists.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub path: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub cell: (u16, u16),
}

impl TileKey {
    /// The key for `path`'s cover fitted into `area`, as measured against
    /// `picker`'s cell.
    pub fn new(path: &Path, area: Rect, picker: &Picker) -> Self {
        let fs = picker.font_size();
        Self {
            path: path.to_path_buf(),
            cols: area.width,
            rows: area.height,
            cell: (fs.width, fs.height),
        }
    }
}

/// One built gallery tile: the geometry it was built for, the raster, and the
/// kitty image id it was last placed under (`None` off-kitty, or before the
/// first placement) so an eviction can free the upload (SQ-1190).
struct TileEntry {
    key: TileKey,
    proto: Protocol,
    placed_id: Option<u32>,
}

/// A gallery-tile encode request: the geometry, the shared decoded jacket, and
/// a copy of the `Picker` to encode with (SQ-1199).
///
/// `Picker` is `Clone` and holds nothing but plain data — a font size, a
/// protocol type, a background colour, a tmux flag and a capability list — so
/// it is `Send` and a copy per request is a handful of bytes. That is why the
/// worker gets a copy rather than the facts to rebuild one from: no
/// reconstruction can go out of step with what the UI thread is actually
/// drawing with. (Kitty image ids come from `rand::random()` inside the crate,
/// not from any counter the `Picker` owns, so two threads encoding at once
/// cannot collide over one.)
pub struct TileRequest {
    pub key: TileKey,
    pub img: Arc<image::DynamicImage>,
    pub picker: Picker,
}

/// A finished gallery-tile encode: the key it was asked for under, and the
/// raster (`None` when the encode failed).
pub type TileResponse = (TileKey, Option<Protocol>);

/// Background gallery-tile encoder (SQ-1199), the same shape as [`CoverDecoder`]
/// one stage further along the pipeline: one long-lived worker thread, a request
/// channel, and a non-blocking drain of finished work.
///
/// Before this, `CoverState::tile_protocol` resized the decoded jacket to the
/// tile box and encoded the terminal protocol (kitty/sixel/iTerm2/half-blocks)
/// **synchronously, inside the draw**, once per newly visible tile — so one
/// scroll notch exposing a row of tiles stalled the picker's event loop for the
/// whole row's worth of resamples and encodes. Now the draw enqueues and paints
/// the letterbox footprint it was already painting for an undecoded cover, and
/// the tile appears when its raster lands.
///
/// In-flight keys are tracked here, so a redraw while a tile is still encoding
/// re-requests nothing — the picker redraws every 16ms while any tile is
/// pending, which without the dedupe would queue the same encode dozens of
/// times over.
pub struct TileEncoder {
    req_tx: std::sync::mpsc::Sender<TileRequest>,
    /// Kept only in the worker-less [`Self::detached`] form, where the harness
    /// drains it instead of a thread. `None` once a worker owns it.
    req_rx: Option<std::sync::mpsc::Receiver<TileRequest>>,
    res_tx: std::sync::mpsc::Sender<TileResponse>,
    res_rx: std::sync::mpsc::Receiver<TileResponse>,
    in_flight: HashSet<TileKey>,
    /// Keys whose encode came back empty. They are never retried: the encode is
    /// a pure function of the jacket, the box and the picker, so a second
    /// attempt would fail identically — and since the draw asks again on every
    /// frame, retrying would pin the picker's tick loop at 16ms re-encoding a
    /// tile that cannot be built. The synchronous build had the same property
    /// for free, by falling straight through to the titled placeholder.
    failed: HashSet<TileKey>,
    _worker: Option<std::thread::JoinHandle<()>>,
}

impl TileEncoder {
    /// Spawn the encode worker. It exits cleanly when the `TileEncoder` is
    /// dropped (dropping `req_tx` makes the worker's `recv()` err).
    pub fn new() -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<TileRequest>();
        let (res_tx, res_rx) = std::sync::mpsc::channel::<TileResponse>();
        let tx = res_tx.clone();
        let worker = std::thread::spawn(move || {
            while let Ok(r) = req_rx.recv() {
                let built = crate::render::graphics::fitted_protocol(
                    &r.picker,
                    &r.img,
                    Size::new(r.key.cols, r.key.rows),
                    false,
                );
                if tx.send((r.key, built)).is_err() {
                    break;
                }
            }
        });
        Self {
            req_tx,
            req_rx: None,
            res_tx,
            res_rx,
            in_flight: HashSet::new(),
            failed: HashSet::new(),
            _worker: Some(worker),
        }
    }

    /// A `TileEncoder` with NO worker thread: requests pile up on the request
    /// channel for the caller to read with [`Self::take_requests`], and results
    /// are whatever the caller feeds back with [`Self::deliver`].
    ///
    /// This is the harness seam (mirroring `GraphicsRender::drive_v6_encode`,
    /// SQ-0469): it makes "the draw enqueued and did not encode" and "this
    /// reply is stale" assertable without racing a thread or waiting on a
    /// clock. The picker itself always uses [`Self::new`].
    pub fn detached() -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<TileRequest>();
        let (res_tx, res_rx) = std::sync::mpsc::channel::<TileResponse>();
        Self {
            req_tx,
            req_rx: Some(req_rx),
            res_tx,
            res_rx,
            in_flight: HashSet::new(),
            failed: HashSet::new(),
            _worker: None,
        }
    }

    /// Queue `key`'s encode, unless the same key is already in flight. Returns
    /// whether it was queued. Silently dropped if the worker has already exited.
    pub fn request(&mut self, key: TileKey, img: Arc<image::DynamicImage>, picker: &Picker) -> bool {
        if self.failed.contains(&key) || !self.in_flight.insert(key.clone()) {
            return false;
        }
        // A copy of the picker, not the facts to rebuild one — see [`TileRequest`].
        let _ = self.req_tx.send(TileRequest { key, img, picker: picker.clone() });
        true
    }

    /// True once `key`'s encode has come back empty, and so will never be
    /// retried (see the `failed` field). The caller draws its no-cover
    /// placeholder rather than waiting for a raster that is not coming.
    pub fn failed(&self, key: &TileKey) -> bool {
        self.failed.contains(key)
    }

    /// True while ANY tile encode is outstanding — the picker's "keep ticking"
    /// condition, so the redraw that paints a landed tile fires without a
    /// keypress (the same role `!requested.is_empty()` plays for decodes).
    pub fn pending(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// Non-blocking drain of every finished encode. A key leaves the in-flight
    /// set whether or not its raster survives the caller's staleness check, so
    /// a reply for a geometry nobody wants any more cannot pin the tick loop on.
    pub fn drain(&mut self) -> Vec<TileResponse> {
        let done: Vec<TileResponse> = self.res_rx.try_iter().collect();
        for (key, proto) in &done {
            self.in_flight.remove(key);
            if proto.is_none() {
                self.failed.insert(key.clone());
            }
        }
        done
    }

    /// Block until every in-flight encode has come back, then drain (test
    /// helper — the deterministic counterpart to sleeping, mirroring SQ-0469's
    /// `drive_v6_encode`). Gives up if the worker has gone away.
    pub fn drain_blocking(&mut self) -> Vec<TileResponse> {
        let mut done = Vec::new();
        while !self.in_flight.is_empty() {
            match self.res_rx.recv() {
                Ok(r) => {
                    self.in_flight.remove(&r.0);
                    if r.1.is_none() {
                        self.failed.insert(r.0.clone());
                    }
                    done.push(r);
                }
                Err(_) => break,
            }
        }
        done.extend(self.drain());
        done
    }

    /// Every request queued so far, taken off the channel (harness seam; only
    /// meaningful on a [`Self::detached`] encoder, where nothing else reads it).
    pub fn take_requests(&self) -> Vec<TileRequest> {
        self.req_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default()
    }

    /// Feed a result back as though the worker had produced it (harness seam).
    pub fn deliver(&self, key: TileKey, proto: Option<Protocol>) {
        let _ = self.res_tx.send((key, proto));
    }
}

impl Default for TileEncoder {
    fn default() -> Self {
        Self::new()
    }
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
            self.deletes.queue(t.placed_id);
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

    /// Record the kitty image id [`Self::tile`]'s caller placed it under —
    /// always the most-recently-used tile, since a cache hit promotes to the
    /// back and a worker result is installed there (SQ-1190).
    pub fn note_tile_placed(&mut self, id: Option<u32>) {
        if let Some(entry) = self.tiles.back_mut() {
            entry.placed_id = id;
        }
    }

    /// True when `path` has already been decoded (`Some` or `None`) — skip the
    /// re-read/decode. A cached `None` (coverless story) still counts.
    pub fn has(&self, path: &Path) -> bool {
        self.decoded.contains_key(path)
    }

    /// A cover's contribution to [`Self::decoded_bytes`]: its decoded
    /// pixel-buffer size, or 0 for a coverless (`None`) entry.
    fn image_bytes(img: &Option<Arc<image::DynamicImage>>) -> usize {
        img.as_ref().map_or(0, |i| i.as_bytes().len())
    }

    /// Move `path` to most-recently-used in the decoded-image LRU, if it's
    /// cached at all. Called on every access (a "hit"), not just on insert, so
    /// a cover the picker is actively showing — built from `decoded` on every
    /// draw via [`Self::protocol`]/[`Self::tile_protocol`] — is never the
    /// least-recently-used entry `insert`'s eviction picks next, however long
    /// ago it was first decoded (SQ-1195).
    fn touch(&mut self, path: &Path) {
        if self.decoded.contains_key(path) {
            self.order.retain(|p| p != path);
            self.order.push_back(path.to_path_buf());
        }
    }

    /// Record the decode result for `path` (`Some(img)` or a coverless `None`)
    /// in the LRU cache, evicting the least-recently-used entries until the
    /// total decoded bytes are back within [`COVER_BYTE_BUDGET`] (SQ-1195). A
    /// single cover larger than the whole budget is still inserted and kept —
    /// it is the one just requested — only OTHER entries are evicted to make
    /// room for it.
    ///
    /// Re-inserting an existing path refreshes its recency without duplicating it
    /// in `order` (so `order` stays 1:1 with `decoded` — no leak, no premature
    /// eviction of a live key). A replaced image also drops any stale built
    /// protocol for that path so `protocol()` rebuilds from the new image.
    pub fn insert(&mut self, path: PathBuf, img: Option<image::DynamicImage>) {
        let img = img.map(Arc::new);
        let new_bytes = Self::image_bytes(&img);
        if let Some(prev) = self.decoded.insert(path.clone(), img) {
            self.decoded_bytes = self.decoded_bytes + new_bytes - Self::image_bytes(&prev);
            // Existing key: move it to most-recent, and invalidate a stale raster
            // (both the info-panel proto and any gallery tiles for this path),
            // freeing each dropped upload in the terminal (SQ-1190).
            self.order.retain(|p| p != &path);
            if matches!(&self.proto, Some((p, _, _, _, _)) if *p == path) {
                let old = self.proto.take();
                self.deletes.queue(old.and_then(|t| t.4));
            }
            let stale_tiles: Vec<Option<u32>> = self
                .tiles
                .iter()
                .filter(|t| t.key.path == path)
                .map(|t| t.placed_id)
                .collect();
            self.tiles.retain(|t| t.key.path != path);
            for id in stale_tiles {
                self.deletes.queue(id);
            }
        } else {
            self.decoded_bytes += new_bytes;
        }
        self.order.push_back(path);
        // Evict oldest-first, but never the entry just pushed to the back —
        // `order.len() > 1` guarantees at least that one survives regardless
        // of its own size.
        while self.decoded_bytes > COVER_BYTE_BUDGET && self.order.len() > 1 {
            let Some(oldest) = self.order.pop_front() else { break };
            if let Some(removed) = self.decoded.remove(&oldest) {
                self.decoded_bytes -= Self::image_bytes(&removed);
            }
        }
    }

    /// Drop any cached decode (and built protocol) for `path`, so the next
    /// request re-reads and re-decodes it. Used after a fetch writes a
    /// `cover.png` for a story previously cached as coverless (`None`) —
    /// without this the stale `None` would hide the freshly fetched cover
    /// until the picker is reopened (SQ-0348).
    pub fn forget(&mut self, path: &Path) {
        if let Some(removed) = self.decoded.remove(path) {
            self.decoded_bytes -= Self::image_bytes(&removed);
            self.order.retain(|p| p != path);
        }
        if matches!(&self.proto, Some((p, _, _, _, _)) if p == path) {
            let old = self.proto.take();
            self.deletes.queue(old.and_then(|t| t.4));
        }
        let stale_tiles: Vec<Option<u32>> =
            self.tiles.iter().filter(|t| t.key.path == path).map(|t| t.placed_id).collect();
        self.tiles.retain(|t| t.key.path != path);
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
        self.touch(path);
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

    /// The already-built gallery-tile protocol for `key`, or `None` when it has
    /// not been encoded yet. Unlike [`protocol`], many of these coexist (one per
    /// visible tile), so they live in a bounded LRU keyed by [`TileKey`] rather
    /// than a single slot.
    ///
    /// **This never builds anything** (SQ-1199). The resize + encode is the
    /// heaviest synchronous site in the app — a gallery tile is a smaller box
    /// than the info panel's, so a jacket is reduced further and by a
    /// direction-aware filter (SQ-0829), once per newly visible tile, a whole
    /// row of them per scroll notch — and it now runs on [`TileEncoder`]'s
    /// worker. A miss means "ask the worker and draw the letterbox"; the tile
    /// arrives via [`Self::insert_tile`] and paints on the next frame.
    ///
    /// [`protocol`]: Self::protocol
    pub fn tile(&mut self, key: &TileKey) -> Option<&Protocol> {
        // A tile on screen means its underlying decoded image is in active use,
        // whether or not its raster is built yet — so the LRU is touched on the
        // miss too, exactly as the synchronous build used to.
        self.touch(&key.path);
        let pos = self.tiles.iter().position(|t| t.key == *key)?;
        // Cache hit: promote to most-recently-used and hand it back. Same
        // `Protocol`, so the id `note_tile_placed` already recorded is still
        // right — nothing to free here.
        let entry = self.tiles.remove(pos).unwrap();
        self.tiles.push_back(entry);
        self.tiles.back().map(|t| &t.proto)
    }

    /// The decoded jacket for `path`, shared with [`TileEncoder`]'s worker
    /// (SQ-1199). `None` for an undecoded or coverless story.
    pub fn image(&self, path: &Path) -> Option<Arc<image::DynamicImage>> {
        self.decoded.get(path).and_then(|o| o.clone())
    }

    /// Install a worker-built tile raster, unless it is stale.
    ///
    /// Stale means the terminal's cell no longer has the shape the encode was
    /// fitted against — the reply was already in flight when the font size
    /// moved and [`Self::invalidate_cell_geometry`] threw the rest away. Such a
    /// raster is DISCARDED rather than cached: the current layout will never
    /// ask for its key again, so keeping it would only burn a slot of
    /// [`TILE_CAP`] until the LRU walked it out. Returns whether it was kept.
    pub fn insert_tile(&mut self, key: TileKey, proto: Protocol, cell: (u16, u16)) -> bool {
        if key.cell != cell {
            return false;
        }
        // A second reply for a key already held (a request re-issued across an
        // eviction, say) replaces it — and frees the upload the old one left in
        // the terminal (SQ-1190).
        if let Some(pos) = self.tiles.iter().position(|t| t.key == key) {
            if let Some(old) = self.tiles.remove(pos) {
                self.deletes.queue(old.placed_id);
            }
        }
        self.tiles.push_back(TileEntry { key, proto, placed_id: None });
        // LRU eviction beyond capacity frees each dropped tile's upload, not
        // merely the struct that named it (SQ-1190).
        while self.tiles.len() > TILE_CAP {
            if let Some(evicted) = self.tiles.pop_front() {
                self.deletes.queue(evicted.placed_id);
            }
        }
        true
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

#[cfg(all(test, feature = "t-picker"))]
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

    /// A blank RGBA image whose decoded buffer is exactly `bytes` long
    /// (`w * h * 4`, one row). `insert` takes a `DynamicImage` directly and
    /// never re-encodes it, so this gives byte-exact control over what
    /// `CoverState::image_bytes` reads — no PNG round-trip, and no dependence
    /// on the `image` crate's choice of pixel format for a given encoding.
    fn image_of_bytes(bytes: usize) -> image::DynamicImage {
        assert_eq!(bytes % 4, 0, "RGBA is 4 bytes/pixel");
        image::DynamicImage::ImageRgba8(image::RgbaImage::new((bytes / 4) as u32, 1))
    }

    /// SQ-1195 (Part B): three covers a little over a third of the budget each
    /// — two fit comfortably, the third pushes the running total past
    /// `COVER_BYTE_BUDGET`, so the least-recently-used (the first inserted)
    /// must be evicted to bring the total back within budget.
    ///
    /// Falsified by the old count-based `CAP`: three entries is nowhere near
    /// 128, so nothing would be evicted and `decoded_bytes` would sit over
    /// budget.
    #[test]
    fn insert_evicts_least_recently_used_once_the_byte_budget_is_exceeded() {
        let mut st = CoverState::default();
        let third = COVER_BYTE_BUDGET / 3 + 8;
        let (a, b, c) = (PathBuf::from("a.gblorb"), PathBuf::from("b.gblorb"), PathBuf::from("c.gblorb"));
        st.insert(a.clone(), Some(image_of_bytes(third)));
        st.insert(b.clone(), Some(image_of_bytes(third)));
        assert!(st.has(&a) && st.has(&b), "two thirds of the budget is still under it");

        st.insert(c.clone(), Some(image_of_bytes(third)));
        assert!(!st.has(&a), "the least-recently-used entry is evicted to make room");
        assert!(st.has(&b) && st.has(&c), "the two most recent survive");
        assert!(
            st.decoded_bytes <= COVER_BYTE_BUDGET,
            "total decoded bytes must be back within budget after insert: {}",
            st.decoded_bytes
        );
    }

    /// SQ-1195 (Part B): a "hit" — the picker actually drawing an already-cached
    /// cover via `protocol()` — must move it to most-recently-used, or a cover
    /// still on screen could be the next thing evicted purely because it was
    /// decoded first.
    #[test]
    fn touching_an_entry_makes_it_recent_and_it_survives_the_next_eviction() {
        let mut st = CoverState::default();
        let third = COVER_BYTE_BUDGET / 3 + 8;
        let (a, b, c) = (PathBuf::from("a.gblorb"), PathBuf::from("b.gblorb"), PathBuf::from("c.gblorb"));
        st.insert(a.clone(), Some(image_of_bytes(third)));
        st.insert(b.clone(), Some(image_of_bytes(third)));

        // Touch `a` (a picker draw), making it MORE recent than `b`.
        let picker = ratatui_image::picker::Picker::halfblocks();
        assert!(st.protocol(&picker, &a, Rect::new(0, 0, 4, 4), false).is_some());

        st.insert(c.clone(), Some(image_of_bytes(third)));
        assert!(st.has(&a), "a was touched by the hit, so it is not the least-recently-used");
        assert!(!st.has(&b), "b, never touched since its insert, is evicted instead");
        assert!(st.has(&c));
    }

    /// SQ-1195 (Part B): a cover larger than the WHOLE budget is still the one
    /// just requested — it must be cached, not silently dropped.
    #[test]
    fn a_single_cover_larger_than_the_whole_budget_is_still_cached() {
        let mut st = CoverState::default();
        let huge = COVER_BYTE_BUDGET + 4 * 1024 * 1024;
        let p = PathBuf::from("huge.gblorb");
        st.insert(p.clone(), Some(image_of_bytes(huge)));
        assert!(st.has(&p), "the cover just requested is kept even though it alone exceeds the budget");
        assert_eq!(st.decoded_bytes, huge);
    }

    /// SQ-1195 (Part B): the old 128-entry count cap is gone — many TINY
    /// covers, nowhere near the byte budget, all stay regardless of count.
    #[test]
    fn the_old_count_cap_of_128_is_no_longer_a_bound() {
        let mut st = CoverState::default();
        let paths: Vec<PathBuf> = (0..129).map(|i| PathBuf::from(format!("game{i}.gblorb"))).collect();
        for p in &paths {
            st.insert(p.clone(), Some(image_of_bytes(64)));
        }
        assert_eq!(st.decoded.len(), 129, "no count bound remains: all 129 tiny covers stay");
        for p in &paths {
            assert!(st.has(p));
        }
    }

    #[test]
    fn reinsert_refreshes_recency_and_byte_accounting_without_corrupting_lru() {
        let mut st = CoverState::default();
        let third = COVER_BYTE_BUDGET / 3 + 8;
        let (a, b) = (PathBuf::from("a.gblorb"), PathBuf::from("b.gblorb"));
        st.insert(a.clone(), Some(image_of_bytes(third)));
        st.insert(b.clone(), Some(image_of_bytes(third)));
        assert_eq!(st.order.len(), 2, "two distinct entries so far");

        // Re-insert `a` with a DIFFERENT decoded size — `order` must not gain a
        // duplicate (else a later eviction would drop a live key and
        // `order`/`decoded` would diverge), and `decoded_bytes` must reflect
        // `a`'s NEW size, not the sum of old and new.
        let a_new_bytes = third / 2;
        st.insert(a.clone(), Some(image_of_bytes(a_new_bytes)));
        assert_eq!(st.order.len(), 2, "order must stay 1:1 with decoded — no duplicate");
        assert_eq!(st.decoded_bytes, a_new_bytes + third, "b's size plus a's NEW size, not a's stale one too");

        // `a` was just refreshed (now most-recent); `b` was not touched since
        // its own insert. Pushing the total over budget must evict `b`, not
        // the refreshed `a`.
        st.insert(PathBuf::from("c.gblorb"), Some(image_of_bytes(third)));
        st.insert(PathBuf::from("d.gblorb"), Some(image_of_bytes(third)));
        assert!(st.has(&a), "the refreshed entry must survive eviction");
        assert!(!st.has(&b), "b, the genuine least-recently-used, is evicted");
    }

    /// Build `path`'s tile the way [`TileEncoder`]'s worker and the picker
    /// loop's drain do between them — synchronously, since a unit test has no
    /// loop to tick. `None` for a coverless/undecoded path, exactly as the old
    /// synchronous `tile_protocol` returned for one.
    fn build_tile(st: &mut CoverState, picker: &Picker, path: &Path, area: Rect) -> Option<TileKey> {
        let key = TileKey::new(path, area, picker);
        let img = st.image(path)?;
        let proto = crate::render::graphics::fitted_protocol(
            picker,
            &img,
            Size::new(key.cols, key.rows),
            false,
        )?;
        let cell = key.cell;
        st.insert_tile(key.clone(), proto, cell).then_some(key)
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

        assert!(build_tile(&mut st, &picker, a, area).is_some());
        assert!(build_tile(&mut st, &picker, b, area).is_some());
        // Both remain cached (2 distinct tiles held at once).
        assert_eq!(st.tiles.len(), 2);
        // A coverless / undecoded path yields nothing.
        assert!(build_tile(&mut st, &picker, Path::new("missing.gblorb"), area).is_none());
    }

    #[test]
    fn tile_protocol_dropped_when_image_is_replaced_or_forgotten() {
        let mut st = CoverState::default();
        let picker = ratatui_image::picker::Picker::halfblocks();
        let area = Rect::new(0, 0, 16, 8);
        let p = Path::new("game.gblorb");

        st.insert(p.to_path_buf(), decode(&png_bytes()));
        assert!(build_tile(&mut st, &picker, p, area).is_some());
        assert_eq!(st.tiles.len(), 1);

        // Re-decoding the same path (e.g. after a fetch writes a new cover)
        // must invalidate its stale tile raster.
        st.insert(p.to_path_buf(), decode(&png_bytes()));
        assert_eq!(st.tiles.len(), 0, "replacing the image drops its tile raster");

        build_tile(&mut st, &picker, p, area);
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
    /// returns for it, exactly as `picker_ui.rs`'s gallery draw does — with the
    /// encode inlined (see `build_tile`) where the draw would wait for the worker.
    fn place_tile(st: &mut CoverState, picker: &Picker, path: &Path, area: Rect, buf: &mut Buffer) -> Option<u32> {
        let key = TileKey::new(path, area, picker);
        if st.tile(&key).is_none() {
            build_tile(st, picker, path, area)?;
        }
        let proto = st.tile(&key)?;
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
    /// Falsified by reverting the `self.deletes.queue(evicted.placed_id)` call
    /// in `insert_tile`'s eviction loop: the flush below then writes nothing.
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
        assert!(st.tiles.iter().all(|t| t.key.path != first), "the oldest tile was evicted");

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
        assert!(build_tile(&mut state, &picker, path, area).is_some(), "…and a gallery tile");

        state.invalidate_cell_geometry();
        assert!(state.has(path), "the decode survives — only the geometry was wrong");
        assert_eq!(
            (state.proto.is_some(), state.tiles.len()),
            (false, 0),
            "every raster was fitted to the old cell and must be rebuilt"
        );
    }
}
