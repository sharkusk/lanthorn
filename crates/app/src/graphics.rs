//! Graphics-window canvases + Blorb Pict resolution for in-game Glulx graphics.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use image::{DynamicImage, Rgba, RgbaImage};


/// Unpack a Glk 24-bit `0xRRGGBB` color into an opaque RGBA pixel.
fn rgb(color: u32) -> Rgba<u8> {
    Rgba([(color >> 16) as u8, (color >> 8) as u8, color as u8, 0xFF])
}

/// A graphics window's pixel canvas.
///
/// `img` is an `Arc` so [`arc`](Canvas::arc) — called for every graphics window
/// on every screen refresh (once per timer tick during an animation) — is a
/// cheap reference-count bump, not a full-bitmap deep copy. Mutations go through
/// `Arc::make_mut`, which copies-on-write only when a previously-handed-out clone
/// is still alive, so a static canvas is never copied. (SQ-0343)
/// Process-global draw sequence: every v6 picture draw stamps the target
/// canvas with the next value, so the renderer can z-order overlapping v6
/// windows by DRAW ORDER (later draw = on top) instead of window number — the
/// order the game actually painted them (e.g. Zork0 draws its banner, then the
/// compass overlays, then the room illustration on top). (SQ-0186)
static DRAW_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The next global draw-sequence stamp.
pub fn next_draw_seq() -> u64 {
    DRAW_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Cloning is cheap: the pixels ride an `Arc` and every mutator goes through
/// `Arc::make_mut`, so a clone is a refcount bump that only copies if one of the
/// two is painted afterwards. That is what makes the v6 picture pacer's
/// intermediate frames (SQ-0708) affordable — it snapshots the whole canvas map
/// at each step of a turn's picture sequence.
#[derive(Clone)]
pub struct Canvas {
    pub img: Arc<RgbaImage>,
    bg: Rgba<u8>,
    /// Bumped on every draw so the renderer can cache the built protocol.
    pub version: u64,
    /// Global draw-order stamp of this canvas's most recent picture draw
    /// (0 = never drawn). The v6 compositor sorts overlapping windows by this,
    /// so later-drawn windows paint on top. Set only on the v6 picture path.
    pub z_seq: u64,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        // Default background is TRANSPARENT, not opaque black: a graphics window's
        // pixels that the game hasn't painted (a fresh canvas, or one just cleared
        // by a resize before the game's Arrange redraw lands) must show the pane
        // underneath, never a solid black block. Games that want an opaque
        // background set it via glk_window_set_background_color. (SQ-0332)
        Canvas { img: Arc::new(RgbaImage::new(w.max(1), h.max(1))), bg: Rgba([0, 0, 0, 0x00]), version: 1, z_seq: 0 }
    }

    /// Resize (preserving nothing — Glk redraws) if the pixel dims changed. Cleared
    /// to `bg` (transparent unless the game set one), so an un-redrawn window shows
    /// the pane, not a black block.
    pub fn resize(&mut self, w: u32, h: u32) {
        if (self.img.width(), self.img.height()) != (w.max(1), h.max(1)) {
            self.img = Arc::new(RgbaImage::from_pixel(w.max(1), h.max(1), self.bg));
            self.version += 1;
        }
    }

    /// Grow the canvas to at least `w × h`, PRESERVING existing content (a v6
    /// window can receive several stacked pictures, and a picture may extend past
    /// the window's nominal pixel size — e.g. Zork0's 45×40 compass into a 320×5
    /// banner). Never shrinks; a no-op when already big enough. Unlike `resize`,
    /// this keeps what was already drawn. (SQ-0186)
    pub fn grow_to(&mut self, w: u32, h: u32) {
        let (cw, ch) = (self.img.width(), self.img.height());
        let (nw, nh) = (cw.max(w.max(1)), ch.max(h.max(1)));
        if (nw, nh) == (cw, ch) {
            return;
        }
        let mut grown = RgbaImage::from_pixel(nw, nh, self.bg);
        image::imageops::replace(&mut grown, &*self.img, 0, 0);
        self.img = Arc::new(grown);
        self.version += 1;
    }

    pub fn set_background(&mut self, color: u32) { self.bg = rgb(color); }

    fn paint(&mut self, px: Rgba<u8>, left: i32, top: i32, w: u32, h: u32) {
        let (cw, ch) = (self.img.width() as i64, self.img.height() as i64);
        let x0 = left.max(0) as i64;
        let y0 = top.max(0) as i64;
        let x1 = (left as i64 + w as i64).min(cw);
        let y1 = (top as i64 + h as i64).min(ch);
        let img = Arc::make_mut(&mut self.img);
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x as u32, y as u32, px);
            }
        }
        self.version += 1;
    }

    pub fn fill_rect(&mut self, color: u32, left: i32, top: i32, w: u32, h: u32) {
        self.paint(rgb(color), left, top, w, h);
    }

    pub fn erase_rect(&mut self, left: i32, top: i32, w: u32, h: u32) {
        let bg = self.bg;
        self.paint(bg, left, top, w, h);
    }

    /// Composite `src` at `(x, y)`, optionally scaled to `(sw, sh)`, honoring alpha.
    ///
    /// `(sw, sh)` come from the game (`glk_image_draw_scaled`) and are clamped
    /// to the canvas dimensions before allocating the scaled bitmap — anything
    /// larger is clipped by `overlay` anyway, and clamping bounds the
    /// allocation against a malicious/buggy game requesting e.g. a
    /// 0x40000000 x 0x40000000 image.
    ///
    /// The resample is [`crate::render::graphics::resize_directional`] (SQ-0829),
    /// not a fixed filter. A game names both axes independently here, so this one
    /// call can grow one and shrink the other, and it used to run Triangle at every
    /// size: a Glulx title card blown up 3× came back blurred — the exact opposite
    /// of the "crisp integer upscale" pixel art is drawn at — while a cut-out card
    /// (fmvpoker's deck is stencilled on colour 1) bled its transparent
    /// `(0, 0, 0)` into every edge it was shrunk past.
    pub fn draw_image(&mut self, src: &DynamicImage, x: i32, y: i32, scale: Option<(u32, u32)>) {
        let scaled;
        let view: &DynamicImage = match scale {
            Some((sw, sh)) if sw > 0 && sh > 0 => {
                let sw = sw.min(self.img.width());
                let sh = sh.min(self.img.height());
                scaled = DynamicImage::ImageRgba8(crate::render::graphics::resize_directional(
                    &src.to_rgba8(),
                    sw,
                    sh,
                ));
                &scaled
            }
            _ => src,
        };
        image::imageops::overlay(Arc::make_mut(&mut self.img), view, x as i64, y as i64);
        self.version += 1;
    }

    /// Like [`Canvas::draw_image`] (unscaled), but clipped to the `clip`
    /// pixel box `(w, h)` anchored at the canvas origin — ZMSD §8's "all
    /// plotting is always clipped to the current window" for a canvas that may
    /// be larger than the window's current box (a window that shrank keeps its
    /// old pixels; only new plotting is bounded by the new size).
    pub fn draw_image_clipped(&mut self, src: &DynamicImage, x: i32, y: i32, clip: (u32, u32)) {
        if x < 0 || y < 0 {
            // v6 draw coords are 1-based-positive by the time they reach the
            // canvas; anything else is clamped upstream.
            return;
        }
        let (cx, cy) = (x as u32, y as u32);
        let (cw, ch) = clip;
        if cx >= cw || cy >= ch {
            return;
        }
        let allow_w = (cw - cx).min(src.width());
        let allow_h = (ch - cy).min(src.height());
        if allow_w == 0 || allow_h == 0 {
            return;
        }
        if allow_w < src.width() || allow_h < src.height() {
            let cropped = src.crop_imm(0, 0, allow_w, allow_h);
            image::imageops::overlay(Arc::make_mut(&mut self.img), &cropped, x as i64, y as i64);
        } else {
            image::imageops::overlay(Arc::make_mut(&mut self.img), src, x as i64, y as i64);
        }
        self.version += 1;
    }

    /// A cheap clone of the canvas bitmap (an `Arc` ref-count bump — see the type
    /// docs), handed to the renderer each frame.
    pub fn arc(&self) -> Arc<RgbaImage> { Arc::clone(&self.img) }
}

/// Scale a decoded picture into unit space, nearest-neighbour (the DOS-authentic
/// crisp pixel double) — the resample [`PictSource::scaled_cached`] runs once
/// per distinct decode and caches, formerly `session::v6_scaled_art`, which ran
/// it fresh on every draw and every replay op (SQ-1196).
///
/// `scale == (1, 1)` is the identity: no resize, and — the other half of
/// SQ-1196 — no copy either. The old function `.clone()`d a full `DynamicImage`
/// here even though the pixels are untouched; this hands back the SOURCE `Arc`
/// itself.
fn scale_art(img: &Arc<DynamicImage>, scale: (u32, u32)) -> Arc<DynamicImage> {
    use image::GenericImageView;
    if scale == (1, 1) {
        return Arc::clone(img);
    }
    let (w, h) = img.dimensions();
    Arc::new(DynamicImage::ImageRgba8(image::imageops::resize(
        img.as_ref(),
        w * scale.0,
        h * scale.1,
        image::imageops::FilterType::Nearest,
    )))
}

/// Resolves + caches decoded images by Blorb `Pict` resource number.
///
/// Adaptive palettes (Blorb spec §11.3): pictures listed in the container's
/// `APal` chunk carry a PLACEHOLDER palette. When one is drawn it must be
/// plotted with the "Current Palette" — the palette (PLTE) of the most recently
/// drawn NON-adaptive picture — not its own. We track the current palette as raw
/// PLTE bytes and, when decoding an adaptive picture, splice those bytes into a
/// copy of its PNG's PLTE chunk (fixing the CRC) before handing it to the
/// decoder. Only the PLTE is substituted: the spec derives the Current Palette
/// from "PLTE, gAMA, cHRM and sRGB/iCCP" but NOT tRNS, and Infocom's adaptive
/// overlays rely on their OWN tRNS for the transparent index, so tRNS is left
/// intact. (Since the decoder reads palette RGB verbatim, copying PLTE alone
/// reproduces exactly the colours the base picture renders with.)
///
/// A story mounted out of an Amiga `.adf` disk image has no Blorb at all: its
/// art is the native `Pic.data` archive that shipped on the same floppy, held
/// here as `native` (SQ-0719 / SQ-0734 tier 2). That format carries no `APal`
/// chunk, but it states the same thing per picture: a directory record whose
/// palette offset is ZERO has no colours of its own and must be drawn through
/// the Current Palette. For Zork Zero those records are, id for id, exactly the
/// 172 numbers `Zork0.blb` lists in `APal` — the Blorb's chunk was derived from
/// this field — so a native archive feeds the machinery below through the same
/// `adaptive` set and the same Current Palette, expressed in the same RGB
/// triples a `PLTE` holds. (SQ-0743)
#[derive(Debug)]
pub struct PictSource {
    blorb: Option<blorb::Blorb>,
    /// Native Infocom picture archive, used when there is no Blorb.
    native: Option<blorb::infocom_pics::InfocomPics>,
    cache: HashMap<u32, Option<Arc<DynamicImage>>>,
    /// Pict numbers declared adaptive by the Blorb `APal` chunk (§11.3). Empty
    /// for the overwhelmingly common no-`APal` case, where `image` takes the
    /// original palette-agnostic fast path.
    adaptive: HashSet<u32>,
    /// Raw PLTE bytes (RGB triples) of the most recently drawn non-adaptive
    /// indexed picture — the "Current Palette". `None` until one is drawn; per
    /// §11.3 an adaptive picture drawn before any non-adaptive one is undefined,
    /// and we fall back to its own placeholder palette.
    current_plte: Option<Vec<u8>>,
    /// Bumped whenever `current_plte` actually changes. Adaptive decodes are
    /// cached per `(resnum, palette_gen)` so a palette change re-decodes them
    /// (the same overlay is legally drawn under different base palettes over a
    /// game's life).
    palette_gen: u64,
    /// Adaptive decodes keyed by `(resnum, palette_gen)`.
    adaptive_cache: HashMap<(u32, u64), Option<Arc<DynamicImage>>>,
    /// [`Self::scaled_image`]/[`Self::scaled_image_under_current_palette`]
    /// results for non-palette-dependent pictures, keyed like `cache` (SQ-1196):
    /// `session::v6_scaled_art` used to re-run the resize on every draw and every
    /// replay op, even though a source picture's scaled pixels never change once
    /// decoded. Cleared everywhere `cache` is (today, just [`Self::set_fuse_dither`]).
    scaled_cache: HashMap<u32, Arc<DynamicImage>>,
    /// The same cache for palette-dependent pictures, keyed like `adaptive_cache`
    /// — evicted the same generation-boundary way, by
    /// [`Self::evict_stale_adaptive_cache`], so a stale palette's scaled pixels
    /// never outlive the decode they were resampled from.
    adaptive_scaled_cache: HashMap<(u32, u64), Arc<DynamicImage>>,
    /// Decoded palette-INDEX planes of a NATIVE archive, keyed by resnum
    /// (SQ-1197) — the expensive half of a native draw, and the half a palette
    /// change does not touch.
    ///
    /// `blorb::infocom_pics::Picture` is already exactly this: `width`,
    /// `height`, one index per pixel, the picture's own table and its
    /// transparent index. Producing one costs a Huffman/LZW/Apple decompress
    /// plus the run-length and per-line XOR stages
    /// ([`InfocomPics::decode`](blorb::infocom_pics::InfocomPics::decode));
    /// turning one into RGBA costs a table lookup per pixel
    /// ([`Picture::rgba_with`](blorb::infocom_pics::Picture::rgba_with)).
    /// A palette bump only changes the second, so retaining the first makes a
    /// palette swap — and the display-list replay
    /// `session::replay_under_current_palette` runs on one, up to
    /// `session::V6_OPS_CAP` (512) ops per window — a RE-MAP rather than a
    /// re-decode.
    ///
    /// Unbounded, like [`Self::cache`], and for a smaller price: a plane is one
    /// byte per pixel where a decode is four, so this pins at most a quarter of
    /// what the RGBA cache beside it already pins for the same pictures — and
    /// only for pictures a draw actually asked for, since `dims`/`info` answer
    /// from the directory header and never reach here (SQ-1194). The ceiling is
    /// the archive: the widest native picture space is the Macintosh's 480x300
    /// (144 KB) and a v6 session draws a few dozen distinct pictures.
    ///
    /// `None` for a resnum the archive cannot decode, so a failure is
    /// remembered rather than retried on every draw — the same shape `cache`
    /// uses.
    index_planes: HashMap<u32, Option<Arc<blorb::infocom_pics::Picture>>>,
    /// The colour table this source's video hardware fixed, when it had one
    /// (SQ-0794) — `blorb::infocom_pics::InfocomPics::hardware_palette`. `None`
    /// for a Blorb, an Amiga/Mac `Pic.data` and an MCGA `.MG1` alike, all of
    /// which carry their colours per picture.
    hw_palette: Option<[blorb::infocom_pics::Rgb; 16]>,
    /// Does this source's art need [`blend_half_width_columns`] on the way out —
    /// i.e. is it a SIXTEEN-colour 640-wide rendition, whose pixels are half as
    /// wide as the unit screen's and whose dithers the card fused (SQ-0797)?
    ///
    /// Set once from [`PictSource::art_scale`] and
    /// [`PictSource::is_monochrome`], because both answers come off the archive's
    /// own contents and neither ever changes for a loaded source.
    blend_columns: bool,
    /// Does this run's MACHINE show the whole screen through one palette, so a
    /// picture's colours recolour everything already drawn (SQ-0887)?
    ///
    /// `zvm::interpreter::MachineProfile::one_screen_palette`, handed down by
    /// the app because only the app resolves which machine this is. It is a
    /// hardware fact and not an archive one, which is why it arrives from
    /// outside rather than being read off the file: Shogun's Amiga `Pic.data`
    /// and its DOS `.MG1` both give every picture a palette, and only one of the
    /// two machines lets the last one loaded repaint the border.
    ///
    /// `false` is the behaviour every source had before, so a machine that has
    /// not been measured changes nothing.
    screen_palette: bool,
}

impl PictSource {
    pub fn new(blorb: Option<blorb::Blorb>) -> PictSource {
        let adaptive = blorb
            .as_ref()
            .map(|b| b.adaptive_pictures().iter().copied().collect())
            .unwrap_or_default();
        PictSource {
            blorb,
            native: None,
            cache: HashMap::new(),
            adaptive,
            current_plte: None,
            palette_gen: 0,
            adaptive_cache: HashMap::new(),
            scaled_cache: HashMap::new(),
            adaptive_scaled_cache: HashMap::new(),
            index_planes: HashMap::new(),
            hw_palette: None,
            blend_columns: false,
            screen_palette: false,
        }
    }

    /// Tell this source the machine shows one palette at a time (SQ-0887).
    ///
    /// Set from the interpreter profile at boot, and re-asserted every launch so
    /// a picker→play loop cannot carry one story's machine into the next.
    pub fn set_screen_palette(&mut self, one: bool) {
        self.screen_palette = one;
    }

    /// A source backed by a native Infocom picture archive (Amiga `Pic.data`)
    /// rather than a Blorb — the artwork that shipped on the same disk image as
    /// the story (SQ-0719).
    ///
    /// An EGA or CGA archive has **no** adaptive pictures, however its directory
    /// reads (SQ-0794). Its records are 12 bytes with nowhere to keep a palette,
    /// so every one of them answers "no palette of my own" — but that is the
    /// opposite of Blorb §11.3's adaptive, which means *defer to the
    /// interpreter*. These defer to nothing: their colours were fixed in the
    /// video card, and `hardware_palette` is that card's table. Emptying the
    /// adaptive set here is what keeps the Current-Palette machinery from
    /// splicing a `PLTE` into pictures that have no say in their own colours.
    pub fn from_native(pics: blorb::infocom_pics::InfocomPics) -> PictSource {
        let hw_palette = pics.hardware_palette();
        let adaptive = match hw_palette {
            Some(_) => HashSet::new(),
            None => pics.adaptive_pictures().iter().map(|&id| u32::from(id)).collect(),
        };
        let mut src =
            PictSource { native: Some(pics), adaptive, hw_palette, ..PictSource::new(None) };
        // A 640-wide SIXTEEN-colour rendition dithers, and its dither is the
        // card's business, not the terminal's — see `blend_half_width_columns`.
        src.blend_columns = src.art_scale().is_some_and(|(sx, _)| sx == 1) && !src.is_monochrome();
        src
    }

    /// Resolve the picture source for `story_path` (SQ-0734's tiers 1 and 2).
    ///
    /// A release disk image supplies its own art: the story and the picture
    /// archive came off the same floppy, so the pairing is guaranteed by the
    /// medium and needs no configuration. **Whichever format the disk is** —
    /// this asks `blorb`'s one mount path (SQ-0840) rather than naming Amiga or
    /// Macintosh, so a format that lands in `blorb::medium::FORMATS` brings its
    /// artwork here without a line changing. Everything else — including a disk
    /// image that carries no readable archive — resolves the story's resource
    /// Blorb exactly as before.
    ///
    /// The Macintosh shipped **two** archives per game, one per screen it sold:
    /// a colour one and a monochrome one, and this decoder reads both (SQ-0838).
    /// The mount hands back the COLOUR one, because that is what every other
    /// medium here supplies and choosing two-colour art for a terminal with
    /// sixteen million of them would need a reason the disk does not give.
    /// The monochrome archive is reached by naming it — `--pictures Pic.data`,
    /// through [`PictureOverride`], which now looks inside the medium for a name
    /// that is not on the host filesystem.
    ///
    /// **The medium is the release, not the platter** (SQ-0862). A multi-disk
    /// press can put the story on one disk and its artwork on another — the DOS
    /// 360K Zork Zero puts CGA on disk 1, the story alone on disk 2 and EGA on
    /// disk 3 — so this asks [`crate::assets::volumes`] rather than mounting one
    /// image. See that function for which siblings are allowed to speak for a
    /// story and which are not.
    /// **A Blorb that names a different build does not speak for a disk-mounted
    /// story** (SQ-0866). That is [`resource_blorb`]'s rule, and it is applied
    /// here rather than in `blorb` because it turns on how the two files came to
    /// be considered together, which is app policy.
    ///
    /// `disk_entry` names WHICH story on the medium, so a compilation pairs each
    /// game with its own archive; see [`release_art`] (SQ-0876).
    pub fn resolve(story_path: &std::path::Path, disk_entry: Option<&str>) -> PictSource {
        if let Some(art) = release_art(story_path, disk_entry) {
            return PictSource::from_native(art.pictures);
        }
        PictSource::new(resource_blorb(story_path).found.map(|(b, _)| b))
    }

    /// Resolve the picture source across all three tiers (SQ-0734).
    ///
    /// `over` is the already-resolved tier-3 override, taken here by value
    /// because a loaded archive moves straight into the source. It is resolved
    /// separately and earlier by [`PictureOverride::resolve`] because its
    /// FLAVOUR also picks the interpreter profile, which has to be settled
    /// before the engine is built.
    ///
    /// A loaded override wins outright — over a resource Blorb beside the story
    /// and over an `.adf`'s own `Pic.data` alike. Everything else, including a
    /// named file that is missing or will not decode, falls through to
    /// [`PictSource::resolve`]; the caller is responsible for surfacing
    /// [`PictureOverride::warning`] in those cases rather than letting the
    /// player believe they are looking at native art.
    pub fn resolve_with_override(
        story_path: &std::path::Path,
        over: PictureOverride,
        disk_entry: Option<&str>,
    ) -> PictSource {
        match over {
            PictureOverride::Loaded { pics, .. } => PictSource::from_native(pics),
            PictureOverride::Unset
            | PictureOverride::Missing { .. }
            | PictureOverride::Unusable { .. } => PictSource::resolve(story_path, disk_entry),
        }
    }

    /// Generation counter of the Current Palette — bumped whenever a non-adaptive
    /// draw establishes a DIFFERENT palette (§11.3). A caller that has already
    /// plotted adaptive pictures watches this: when it moves, everything drawn
    /// with the old palette is now showing the wrong colours and must be replotted.
    pub fn palette_gen(&self) -> u64 {
        self.palette_gen
    }

    /// How many decoded pixel buffers the unbounded [`Self::cache`](field)
    /// currently pins, for a test to assert a size-query path never grew it
    /// (SQ-1194).
    #[cfg(all(test, feature = "t-render"))]
    pub(crate) fn decode_cache_len(&self) -> usize {
        self.cache.len()
    }

    /// The `(resnum, palette_gen)` keys currently pinned in the adaptive
    /// decode cache, for a test to assert a stale generation was evicted
    /// (SQ-1193).
    #[cfg(all(test, feature = "t-render"))]
    pub(crate) fn adaptive_cache_keys(&self) -> Vec<(u32, u64)> {
        self.adaptive_cache.keys().copied().collect()
    }

    /// The index plane currently pinned for `resnum`, WITHOUT decoding one —
    /// for a test to assert, by `Arc::ptr_eq`, that a palette bump re-mapped an
    /// existing plane rather than decoding a fresh one (SQ-1197). `None` both
    /// for "not decoded yet" and for "decoded and failed"; a test that cares
    /// distinguishes them by ordering.
    #[cfg(all(test, feature = "t-render"))]
    pub(crate) fn cached_index_plane(
        &self,
        resnum: u32,
    ) -> Option<Arc<blorb::infocom_pics::Picture>> {
        self.index_planes.get(&resnum).and_then(|o| o.clone())
    }

    /// Keep this source's dither UNFUSED, or fuse it (SQ-0816).
    ///
    /// `fuse` is the player's `fuse_art_dither` preference, and it can only ever
    /// turn the filter OFF: whether an archive is eligible at all stays entirely
    /// [`from_native`](PictSource::from_native)'s business, read off the archive's
    /// own contents. A `.CG1` is not fused because a person said so, and cannot be
    /// made to be — [`blend_half_width_columns`] would only make its one-bit line
    /// work grey (SQ-0806, SQ-0808).
    ///
    /// Both caches are dropped when the answer changes, because every image in
    /// them was decoded under the old one. Call it before the first draw and the
    /// caches are empty anyway; call it later — a settings edit would — and the
    /// next `image()` re-decodes rather than serving a stale blend.
    pub fn set_fuse_dither(&mut self, fuse: bool) {
        let eligible = self.art_scale().is_some_and(|(sx, _)| sx == 1) && !self.is_monochrome();
        let want = fuse && eligible;
        if want == self.blend_columns {
            return;
        }
        self.blend_columns = want;
        self.cache.clear();
        self.adaptive_cache.clear();
        // Every scaled pixel was resampled from a decode this just invalidated.
        self.scaled_cache.clear();
        self.adaptive_scaled_cache.clear();
        // The INDICES do not depend on the fuse — `blend_half_width_columns`
        // runs on the RGBA, after colourisation — so this clear buys nothing
        // today. It is here so that every decode cache on this source has ONE
        // invalidation story ("cleared wherever `cache` is"), rather than one
        // cache with a footnote; a fuse toggle is a settings edit, and paying a
        // re-decode for it once is not a cost worth reasoning about. SQ-1197.
        self.index_planes.clear();
    }

    /// Is this source fusing a 640-wide rendition's dither on the way out?
    pub fn fuses_dither(&self) -> bool {
        self.blend_columns
    }

    /// Is the artwork a TWO-COLOUR rendition? See
    /// [`blorb::infocom_pics::InfocomPics::is_monochrome`]. `false` for a Blorb,
    /// which is never one.
    pub fn is_monochrome(&self) -> bool {
        self.native.as_ref().is_some_and(blorb::infocom_pics::InfocomPics::is_monochrome)
    }

    /// Is the artwork a two-colour rendition of a **video card** — a display with
    /// an ink, a page and no third state (SQ-0956)?
    ///
    /// `is_monochrome` above answers `true` for the Macintosh's mono `Pic.data`
    /// under exactly the same `EF_MONO` test, and that machine's screen is not a
    /// two-state display: its interpreter names ordinary §8.3.1 colours and
    /// `mac/xzip.lst` sets a white page under black ink like any other pair. The
    /// CARD is the one that collapses, and the archive's container is what says
    /// which machine drew it — see
    /// [`blorb::infocom_pics::InfocomPics::two_colour_palette`], whose two tables
    /// each carry their own capture.
    ///
    /// This is what installs [`zvm::screen::Palette::IbmCga`] at boot, and through
    /// it what `zvm::screen::two_colour_card_request` reads.
    pub fn two_colour_card(&self) -> bool {
        self.native
            .as_ref()
            .and_then(blorb::infocom_pics::InfocomPics::two_colour_palette)
            .is_some_and(|p| p == blorb::infocom_pics::CGA_PALETTE)
    }

    /// Should this launch declare the interpreter COLOURLESS to the story
    /// (`honor_game_colours = false`), because the artwork in hand is a
    /// two-colour rendition and no machine is present to say what that display
    /// shows? SQ-0806, refined by SQ-0846 and SQ-0956.
    ///
    /// **The archive's half is what the story cannot see, and that is the whole
    /// point.** A two-colour rendition is a stencil whose transparency reveals a
    /// ground the artwork never had to store, and the story cannot see which
    /// archive was loaded — Zork Zero issues `set_colour(fg=2, bg=9)` for every
    /// video card alike, so honouring it paints the white pillars out against
    /// the white page it asked for. The `EF_MONO` flag is the evidence that a
    /// stencil is what is in hand, and bocfel says as much ("the flags always
    /// *seem* to equal 0xe if the graphics are monochrome"). Declaring the
    /// interpreter colourless hands the ground back to the host theme and the
    /// stencil reads again.
    ///
    /// **A machine that already presents that page is not being guessed at, and
    /// outranks the guess.** Where a launch names a machine, the answer came off
    /// that machine's own interpreter: the Macintosh's white page under black ink
    /// is `mac/xzip.lst`'s `SetColor := (zWHITE*256) + zBLACK`, and a mono Mac
    /// `Pic.data` is the archive that same interpreter chose *for* that page, in
    /// one decision (SQ-0838). Turning colours off there does not save a stencil
    /// from a colour the game asked for; it throws away the one machine whose
    /// colours are known, and it cost SQ-0846's status banner its ink.
    ///
    /// **And the IBM PC is now a machine too**, which is what SQ-0956 turns on.
    /// SQ-0928 gave it blue under white and `ProfileSource::Medium` licenses it, so
    /// a real DOS press stopped reaching this rule — reported as Zork Zero's white
    /// page bleeding into its own CGA artwork. The answer is not to decline harder:
    /// a CGA card HAS a screen, it is black under light grey, and a story gets to
    /// choose which side of it is the ink. That lives in
    /// [`Self::two_colour_card`] and `zvm::screen::two_colour_card_request`, with
    /// the colour flag left SET so the `color` command still works.
    ///
    /// | launch                          | machine | declines |
    /// |---------------------------------|---------|----------|
    /// | Mac HFS volume, mono `Pic.data` | (9, 2)  | no — SQ-0846, the Mac's own page |
    /// | DOS press, `.CG1`               | (6, 9)  | no — the card states its own, SQ-0956 |
    /// | bare `.z6` + `--pictures *.cg1` | none    | **yes** — SQ-0806 unmoved |
    ///
    /// The last row is the whole of what is left, and it is what the rule was
    /// written for: a stencil with no machine behind it, where the host theme is
    /// the only ground there is.
    pub fn declines_game_colours(&self, machine_pair: Option<(u8, u8)>) -> bool {
        self.is_monochrome() && machine_pair.is_none()
    }

    /// **The screen this launch is showing, when a two-colour CARD is what it is
    /// showing it on** — the palette to install and the §8.3.3 pair to report, or
    /// `None` for every other launch (SQ-0956).
    ///
    /// One function because three callers must not drift: `startup.rs` at boot,
    /// `reset.rs` on a `@restart` (which may have re-resolved a different
    /// rendition), and `v6_cga_stencil_page`, which measures the frame that comes
    /// out. A harness that re-derived this instead of calling it would keep
    /// passing while the shipped path regressed — the hazard CLAUDE.md names as
    /// "boot a harness the way `startup.rs` boots".
    ///
    /// Three things have to be true together, and each is a different kind of fact:
    ///
    /// - the ARCHIVE is a video card's two colours ([`Self::two_colour_card`],
    ///   read off the container — a `.CG1` and not a Macintosh `Pic.data`);
    /// - the LAUNCH may present its machine ([`Config::machine_colours_licensed`](crate::config::Config::machine_colours_licensed),
    ///   SQ-0928: a medium always, an asked-for machine on request, a bare story
    ///   file never — and SQ-1154: never at all under `--colour theme|terminal`,
    ///   whatever the medium, because that regime is the raw path), which is also
    ///   what stops a `.cg1` opened beside a plain `.z6` from reaching here —
    ///   that launch keeps SQ-0806's decline;
    /// - the PLAYER has not declined game colours, since a card that cannot be
    ///   told to the story has nothing to say.
    pub fn two_colour_card_screen(
        &self,
        cfg: &crate::config::Config,
    ) -> Option<(zvm::screen::Palette, (u8, u8))> {
        if !cfg.honor_game_colours || !self.two_colour_card() {
            return None;
        }
        let pair = cfg.machine_two_colour_colours()?;
        Some((zvm::screen::Palette::IbmCga, pair))
    }

    /// Is this Pict declared adaptive by the container's `APal` chunk (§11.3)?
    pub fn is_adaptive(&self, resnum: u32) -> bool {
        self.adaptive.contains(&resnum)
    }

    /// The Current Palette's raw `PLTE` bytes, for host Save State (SQ-0588).
    ///
    /// Blorb §11.3 makes this live interpreter state, not game state: it is
    /// established by whichever non-adaptive picture was drawn last, and every
    /// adaptive picture drawn after it decodes through it. A save that carries the
    /// display list but not the palette replays those pictures under whatever
    /// palette the restoring session happens to hold — the wrong colours, or none.
    pub fn current_palette(&self) -> Option<&[u8]> {
        self.current_plte.as_deref()
    }

    /// Reinstate a saved Current Palette (SQ-0588). Bumps `palette_gen` on a real
    /// change, exactly as a non-adaptive draw does, so any adaptive decode cached
    /// against the old generation is recomputed rather than reused.
    pub fn set_current_palette(&mut self, plte: Option<Vec<u8>>) {
        if self.current_plte != plte {
            self.current_plte = plte;
            self.palette_gen += 1;
            self.evict_stale_adaptive_cache();
        }
    }

    /// Drop every adaptive decode cached against a palette generation OTHER
    /// than the current one. Call immediately after `palette_gen` bumps
    /// (SQ-1193): `adaptive_cache` is keyed `(resnum, palette_gen)` and the
    /// generation only ever climbs, so on a `screen_palette` machine — where
    /// every drawn picture routes through the adaptive path (SQ-0887) — a
    /// long session otherwise accumulates a full RGBA per (pic, scene
    /// palette) it will never be asked to decode through again.
    fn evict_stale_adaptive_cache(&mut self) {
        let gen = self.palette_gen;
        self.adaptive_cache.retain(|&(_, g), _| g == gen);
        self.adaptive_scaled_cache.retain(|&(_, g), _| g == gen);
    }

    /// Decode `resnum` for a replay WITHOUT establishing a new Current Palette —
    /// the replay path (SQ-0567).
    ///
    /// This is [`Self::image`] with its one side effect removed. An **adaptive**
    /// picture has no colours of its own and decodes through the live palette,
    /// which is the whole point of the replay: Arthur's frame — Picts 54, 170 and
    /// 171, the archive's only `APal` entries — has to follow the scene that
    /// established it. A picture that CARRIES a palette decodes through its own,
    /// exactly as the draw that first put it on the canvas did; what must not
    /// happen is `image`'s reload of the Current Palette from it, which would
    /// undo the change being replayed for.
    ///
    /// # Why a base picture keeps its own palette — SQ-0881
    ///
    /// This used to send base pictures through the live palette too, on the
    /// reading that a v6 framebuffer holds indices and so recolours wholesale
    /// when a palette is loaded. That is true of a machine with ONE palette, and
    /// the corpus says the MCGA is not one. Measured on Arthur's map screen —
    /// `F2` from the churchyard, `arthur-r74-s890714.z6` at 165x50 — the pictures
    /// the game lays down and how many distinct palettes they carry:
    ///
    /// | rendition | map pictures | distinct palettes |
    /// |---|---|---|
    /// | Amiga `Pic.data` | 137, 108, 115, 112, 138, 147, 140 | **1** |
    /// | DOS `.MG1` | the same seven | **3** |
    /// | DOS `.EG1`/`.CG1` | the same seven | none — a hardware table |
    ///
    /// A single-palette machine's archive gives one palette to a screenful
    /// because it has no choice; the MCGA's DAC has 256 entries and Infocom used
    /// them, so its archive gives the scroll, the room box and the compass rose
    /// palettes of their own. Forcing the last one onto all three left the
    /// parchment showing [`blorb::infocom_pics::DEFAULT_PALETTE`] where the
    /// borrowed table ran out — entry 8 grey for the ground, 9 and 10 for the
    /// scroll rods, which is the grey field and rainbow scrolls reported. The
    /// Amiga and the Blorb were never wrong because their palettes agree with
    /// each other, so the same bug drew the same picture.
    ///
    /// Blorb §11.3 already says this without reference to any machine: the
    /// Current Palette is what an ADAPTIVE picture is drawn through. A picture
    /// with a palette of its own was never asking.
    pub fn image_under_current_palette(&mut self, resnum: u32) -> Option<Arc<DynamicImage>> {
        // SQ-0887: on a one-palette machine EVERY picture on the screen is shown
        // through the live table, which is the reading SQ-0881 removed — rightly,
        // for the MCGA, whose DAC holds 256 entries and whose pictures therefore
        // keep their own. Keyed on the machine, the two findings stop competing:
        // Shogun's border follows the scene on the Amiga and does not on DOS,
        // which is the same story and the same border on two machines.
        if self.screen_palette || self.adaptive.contains(&resnum) {
            return self.adaptive_image(resnum);
        }
        self.get(resnum).cloned()
    }

    /// The Blorb `Reso` standard window `(width, height)` in pixels — the
    /// resolution the pictures were authored for. A v6 story advertises this
    /// as its screen size so its hardcoded pixel art lines up (SQ-0186).
    /// `None` when there's no Blorb or no `Reso` chunk.
    ///
    /// A native archive has no equivalent field — Infocom's Amiga interpreter
    /// knew its own 320×200 lores screen — so it answers `None` and lands on
    /// exactly that fallback downstream, which is also what the Blorb `Reso`
    /// chunks of these games say.
    pub fn std_window(&self) -> Option<(u16, u16)> {
        self.blorb.as_ref().and_then(|b| b.std_window())
    }

    /// The standard window a NATIVE archive implies — the picture space every
    /// Infocom archive draws into, whichever machine wrote it (SQ-0837).
    ///
    /// This is the same constant, for the same reason,
    /// [`PictureOverride::std_window`] hands back when a user names an archive
    /// outright: mounting an archive off the disk it shipped on must not produce
    /// different geometry from pointing at that very file by hand.
    ///
    /// It sits AHEAD of the machine in `startup.rs`'s chain, and that ordering
    /// is Infocom's rather than ours (SQ-0838). Their Macintosh interpreter
    /// picked its window and its picture file in one decision — *"for a small
    /// window use mono gfx, for a big window use color gfx"* — so the archive in
    /// hand is the better evidence about the screen, not the worse. SQ-0837 put
    /// this link last, when it answered one fixed pair and could only ever
    /// restate what the machine already said; now that it answers per archive it
    /// has to come first, or a mono-only Mac volume would be laid out on the
    /// colour Mac's screen. **Nothing else moves**: for a `.adf` this is
    /// (320, 200) and so is the Amiga profile, and a Blorb-less story that is
    /// not a disk image still has no native archive at all and still draws its
    /// art 1:1 (SQ-0715/SQ-0718).
    ///
    /// # The screen is the picture space, at the scale the machine drew it
    ///
    /// | rendition                     | picture space | scale | screen  |
    /// |-------------------------------|---------------|-------|---------|
    /// | Amiga/Mac colour, MCGA `.MG1` | 320×200       | (2,2) | 640×400 |
    /// | EGA `.EG1`/`.EG2`, CGA `.CG1` | 640×200       | (1,2) | 640×400 |
    /// | Macintosh mono `Pic.data`     | 480×300       | (1,1) | 480×300 |
    /// | Apple II `ARTHUR.D*`          | 140×192       | (4,2) | 560×384 |
    ///
    /// The first two rows are the corpus as it already stood: SQ-0790's reading
    /// that a 320-wide and a 640-wide rendition are *two drawings of one screen*
    /// survives intact, because the denser one's half-width pixels put it back
    /// on the same 640×400. What that reading could not express is a third
    /// rendition drawn for a genuinely different screen, and the standard
    /// Macintosh's monochrome plate is one: `mac/gfx.p` calls it "scaled for a
    /// 480x300 screen (std Mac)" and Infocom's interpreter really did open a
    /// 480×300 window for it. Multiplying the space by the scale gives every
    /// old rendition the screen it already had and the new one the screen it
    /// asks for — see [`Self::art_scale`] and
    /// [`crate::interpreter::InterpreterProfile::std_window`].
    ///
    /// The fourth row is the Apple II's, and it is the same statement on a
    /// fourth machine (SQ-0863). Its space is stated by Infocom in the dots it
    /// is counted in — `apple/yzip/rel.15/apple.equ`'s `MAXWIDTH EQU 140 ; 560 /
    /// 4 = max "pixels"` and `MAXHEIGHT EQU 192 ; 192 screen lines` — so 140×192
    /// art fills all 560×192 dots of the double-hi-res display, and the screen it
    /// asks for is the one that display has always been shown through. Nothing
    /// here is a preference: the archive says 140 wide, the machine says a pixel
    /// is four dots, and 560×384 is what the two of them multiply to.
    ///
    /// **A caveat this cannot hide**: 300 is not a multiple of the 16-pixel v6
    /// cell, so `session.rs` rounds the screen to 19 rows — 304 pixels, four
    /// more than the Mac's, where a real Mac fitted 20 rows of its own 15-pixel
    /// Geneva into exactly 300. Rounding the other way would hand the game a
    /// 288-pixel screen and clip the bottom twelve pixels off its own artwork.
    pub fn native_std_window(&self) -> Option<(u16, u16)> {
        let pics = self.native.as_ref()?;
        Some((pics.picture_space_width(), pics.picture_space_height()))
    }

    /// The per-axis factor this source's art is scaled by on its way into the
    /// 640×400 unit screen, or `None` when the source has no opinion and the
    /// uniform [`crate::session::V6_ART_SCALE`] rule stands (SQ-0790).
    ///
    /// Only a NATIVE archive answers, because only a native archive states a
    /// picture space. Every Blorb is answered by its `Reso` chunk upstream, and
    /// a Blorb-less story (scopa) is answered by Blorb §11's non-scalable rule.
    ///
    /// The unit screen is the same 640×400 for every rendition — it is
    /// lanthorn's presentation space, not the card's — so the factor is simply
    /// how many unit pixels one art pixel covers on each axis:
    ///
    /// | rendition                    | picture space | scale |
    /// |------------------------------|---------------|-------|
    /// | Amiga `Pic.data`, MCGA `.MG1`| 320×200       | (2, 2)|
    /// | EGA `.EG1`/`.EG2`, CGA `.CG1`| 640×200       | (1, 2)|
    /// | Macintosh mono `Pic.data`    | 480×300       | (1, 1)|
    /// | Apple II `ARTHUR.D*`         | 140×192       | (4, 2)|
    ///
    /// The last row is why the vertical factor is derived rather than fixed at
    /// [`crate::session::V6_ART_SCALE`] (SQ-0838). Every rendition Infocom
    /// shipped is 200 lines tall except the standard Macintosh's monochrome one,
    /// which `mac/gfx.p` calls a "480x300 screen (std Mac)" and displays 1:1
    /// where it scales the colour art by 1.5 or 2 (`IF ge.mono OR myTiny THEN {
    /// scale 1x for display }`). 480×300 is the one picture space that does not
    /// double onto the 640×400 unit screen, and doubling it anyway would put a
    /// 960×600 plate on a 640×400 screen. Deriving both axes the same way leaves
    /// all four existing renditions exactly where they were — 400/200 is 2 — and
    /// is a change of reasoning rather than of behaviour for them.
    ///
    /// The 640-wide row is the whole of SQ-0790, and it is not a compromise: an
    /// EGA pixel really is half as wide as an MCGA one. Bocfel encodes exactly
    /// this as `pixelwidth` — 1.0 at `hw_screenwidth` 320, **0.5** at 640 — and
    /// its final blit (`draw_image.cpp:251`) works out to `gscreenw * H / 320`
    /// for both, i.e. the two picture spaces cover the same rectangle. Frotz
    /// says the same thing as `x_scale = (flags & 0x08) ? 640 : 320`
    /// (`src/curses/ux_pic.c:126`).
    ///
    /// And the corpus agrees, which is what makes this a measurement rather than
    /// a reading: under this rule `arthur.mg1` and `arthur.eg1` produce
    /// **identical** unit-space dimensions for all 125 pictures they share, and
    /// `zork0.mg1`/`zork0.eg1` for 446 of 503 (the remainder differ by a pixel
    /// or two because the two renditions are separately drawn artwork, not one
    /// scaled copy).
    ///
    /// # The Apple's (4, 2), sourced from the machine (SQ-0863)
    ///
    /// The last row is the widest factor in the table and the only one where the
    /// derivation and the machine had to be checked against each other, because
    /// 640/140 is 4.57 and an integer division landing on 4 is not an argument.
    ///
    /// **Horizontally it is Infocom's own number.** `apple/yzip/rel.15/apple.equ`
    /// does not merely state the width, it states the arithmetic:
    ///
    /// ```text
    ///   MAXWIDTH   EQU 140   ; 560 / 4 = max "pixels"
    ///   MAXHEIGHT  EQU 192   ; 192 screen lines
    /// ```
    ///
    /// The Apple II's double-hi-res screen is 560 dots across and a colour pixel
    /// is four of them, so 140 art pixels cover the display exactly and the
    /// horizontal factor is 4 by definition rather than by fit. The directory
    /// agrees from the other end: the widest picture in all four of *Arthur*'s
    /// archives is exactly 140, and the tallest exactly 192, so the artwork uses
    /// every dot the machine had.
    ///
    /// **Vertically the machine states a count and not a size.** `MAXHEIGHT EQU
    /// 192 ; 192 screen lines` is one art row per scan line, so the factor has to
    /// come from what a scan line MEASURES, which is the display's business
    /// rather than the archive's. The Apple II's 192 active lines fill the
    /// visible raster of a 4:3 monitor while 560 dots fill its width, so one line
    /// is (3/4)·(560/192) — 2.19 — dots tall. At a horizontal 4, the vertical
    /// factor that preserves that shape is 2 to the nearest whole pixel. The
    /// alternative, 1, would put 140×192 art on a 560×192 screen and squash the
    /// picture to less than half its height.
    ///
    /// Two independent checks land on the same pair. 560×384 is exactly 70×24
    /// whole [`crate::interpreter::InterpreterProfile::v6_font_cell`]s, where
    /// 560×192 is 70×12 and would tell the story it has twelve rows; and the
    /// uniform rule below — unit space divided by picture space — computes (4, 2)
    /// unaided, so the Apple needs no special case in the arithmetic, only this
    /// paragraph saying why the arithmetic is right here.
    pub fn art_scale(&self) -> Option<(u32, u32)> {
        let unit_w = u32::from(INFOCOM_V6_STD_WINDOW.0) * crate::session::V6_ART_SCALE;
        let unit_h = u32::from(INFOCOM_V6_STD_WINDOW.1) * crate::session::V6_ART_SCALE;
        let (space_w, space_h) = match self.native.as_ref() {
            Some(pics) => (
                u32::from(pics.picture_space_width()).max(1),
                u32::from(pics.picture_space_height()).max(1),
            ),
            // SQ-0936: a BLORB has no native archive to ask, so the density comes
            // from the same place the DOUBLING decision already takes it — the
            // `Reso` chunk's standard window.
            //
            // Blorb §11 is explicit that a resource file without one has no
            // scalable images: "non-scalable images are always displayed at their
            // actual size. (One image pixel per screen pixel.)" Every Infocom v6
            // blorb declares 320x200 and doubles; scopa.blb declares NOTHING
            // because its card art is already drawn for the 640x400 screen, which
            // is why doubling it once told the game its cards were 104x168 and its
            // sample cards overlapped and hung off the bottom.
            //
            // So an undeclared blorb is 1:1, not the doubled default — and it
            // matters beyond bookkeeping now, because the magnification ladder is
            // derived from this. Handing scopa (2, 2) would lock its 1:1 art onto
            // half-steps, putting one art pixel on 1.5 device pixels: the exact
            // blur the lock exists to remove.
            None => match self.std_window() {
                Some((w, h)) => (u32::from(w).max(1), u32::from(h).max(1)),
                None => (unit_w, unit_h),
            },
        };
        Some(((unit_w / space_w).max(1), (unit_h / space_h).max(1)))
    }

    /// `resnum`'s decoded PALETTE-INDEX plane from the native archive, decoding
    /// it once and retaining it (SQ-1197). `None` for a Blorb source (whose
    /// pictures are PNGs — see [`Self::index_planes`](field)), an unknown id, a
    /// size-only placeholder, or a compression variant `blorb` does not decode.
    ///
    /// This is the only caller of `InfocomPics::decode` under `crates/app/src`
    /// (test suites call it directly as an oracle), so a plane cached here is a
    /// decompress that never runs again for the life of the source.
    fn index_plane(&mut self, resnum: u32) -> Option<Arc<blorb::infocom_pics::Picture>> {
        if !self.index_planes.contains_key(&resnum) {
            let decoded = self.native.as_ref().and_then(|pics| {
                let id = u16::try_from(resnum).ok()?;
                pics.decode(id).ok()
            });
            self.index_planes.insert(resnum, decoded.map(Arc::new));
        }
        self.index_planes.get(&resnum).and_then(|o| o.clone())
    }

    fn get(&mut self, resnum: u32) -> Option<&Arc<DynamicImage>> {
        if !self.cache.contains_key(&resnum) {
            let decoded = match &self.blorb {
                Some(b) => b
                    .resource(b"Pict", resnum)
                    .and_then(|(_ty, bytes)| crate::cover::decode(bytes)),
                None => self
                    .index_plane(resnum)
                    .and_then(|pic| native_image(&pic, self.hw_palette.as_ref(), self.blend_columns)),
            };
            self.cache.insert(resnum, decoded.map(Arc::new));
        }
        self.cache.get(&resnum).and_then(|o| o.as_ref())
    }

    /// `(width, height)` of a Pict, or `None`. Answers from the header sniffer
    /// ([`Self::dims`]) rather than a full decode: `image_info` (Glk selector
    /// 8) is the sole caller, and a game can sweep it over its whole picture
    /// catalog at boot, which used to decode and pin every one of those images
    /// in the unbounded `cache` forever — pixels no caller here ever asked for
    /// (SQ-1194). A caller that genuinely needs the DECODED pixels' dimensions
    /// should call [`Self::image`] and measure the result directly, not this.
    pub fn info(&mut self, resnum: u32) -> Option<(u32, u32)> {
        self.dims(resnum)
    }

    /// The decoded image for a Pict about to be DRAWN, or `None`. Returns a
    /// cheap `Arc` clone rather than deep-copying the `DynamicImage`.
    ///
    /// This is the adaptive-palette establishment point (Blorb §11.3): drawing a
    /// NON-adaptive picture updates the Current Palette from its PLTE; drawing an
    /// ADAPTIVE picture decodes it with that Current Palette spliced in. Size
    /// queries (`info`/`dims`) deliberately do NOT go through here, so querying a
    /// picture's dimensions never counts as "drawing" for palette purposes.
    pub fn image(&mut self, resnum: u32) -> Option<Arc<DynamicImage>> {
        // No APal chunk → no adaptive pictures: keep the original fast path
        // (and never touch palette state) for every non-v6 / non-adaptive blorb.
        //
        // SQ-0887: unless the MACHINE shows one palette at a time, in which case
        // there is palette state to keep even with nothing declared adaptive —
        // Shogun's Amiga archive declares none and gives all 48 pictures their
        // own colours, and this early return is what left `current_plte` at
        // `None` forever, so `palette_gen` never moved and the SQ-0567 replay
        // never ran. The border kept the gold of its own table through every
        // scene.
        if self.adaptive.is_empty() && !self.screen_palette {
            return self.get(resnum).cloned();
        }
        if self.adaptive.contains(&resnum) {
            return self.adaptive_image(resnum);
        }
        // A non-adaptive draw establishes the Current Palette for later adaptive
        // draws, then resolves normally.
        let arc = self.get(resnum).cloned();
        if arc.is_some() {
            self.set_current_palette_from(resnum);
        }
        arc
    }

    /// Is `resnum`'s decode ANSWERED BY the Current Palette — the same test
    /// [`Self::image`] and [`Self::image_under_current_palette`] each make before
    /// deciding whether to route through [`Self::adaptive_image`] — an adaptive
    /// Blorb picture, or (SQ-0887) any picture at all on a one-screen-palette
    /// machine. Shared here so the scaled-image cache below keys itself the same
    /// way its source does.
    fn is_palette_dependent(&self, resnum: u32) -> bool {
        self.screen_palette || self.adaptive.contains(&resnum)
    }

    /// [`Self::image`] scaled into unit space (`session::v6_scaled_art`'s job),
    /// cached so the resize runs once per distinct decode rather than on every
    /// draw (SQ-1196): every v6 window refresh and every timer tick re-requests
    /// the same picture at the same `scale`, and a Nearest resize into a fresh
    /// ~1&nbsp;MB buffer is not free to redo each time.
    ///
    /// `scale == (1, 1)` is the identity — no resample, no copy, the *source*
    /// `Arc` is the answer — which is also the common case for a Blorb-less
    /// story (`art_scale` degenerates to (1, 1) exactly when a source has no
    /// scaling opinion; see [`Self::art_scale`]).
    ///
    /// Keyed and invalidated exactly like the source cache it wraps: a
    /// palette-dependent picture by `(resnum, palette_gen)`, evicted the moment
    /// the generation moves on ([`Self::evict_stale_adaptive_cache`]); anything
    /// else by `resnum` alone, cleared wherever [`Self::cache`] is
    /// ([`Self::set_fuse_dither`]). `art_scale` itself never changes for a
    /// source's lifetime (it is the archive's own density — see
    /// `session::GameSession::art_scale`), so it is not part of either key: a
    /// caller that changed it mid-session would need its own cache flush, and
    /// none does.
    pub fn scaled_image(&mut self, resnum: u32, scale: (u32, u32)) -> Option<Arc<DynamicImage>> {
        self.scaled_cached(resnum, scale, |s| s.image(resnum))
    }

    /// [`Self::image_under_current_palette`] scaled into unit space and cached
    /// the same way [`Self::scaled_image`] is — a replay op resolves the same
    /// `(resnum, palette_gen)` pixels a live draw would, so it shares that
    /// cache rather than resampling a second time.
    pub fn scaled_image_under_current_palette(
        &mut self,
        resnum: u32,
        scale: (u32, u32),
    ) -> Option<Arc<DynamicImage>> {
        self.scaled_cached(resnum, scale, |s| s.image_under_current_palette(resnum))
    }

    /// Shared cache lookup/populate for [`Self::scaled_image`] and
    /// [`Self::scaled_image_under_current_palette`]: `decode` is whichever of
    /// the two source methods the caller wants, so both share one cache and one
    /// eviction story instead of duplicating it.
    ///
    /// **`decode` runs on a cache HIT too, and that is the point (SQ-1288).**
    /// [`Self::image`] is not a pure function: drawing a NON-adaptive picture
    /// establishes the Current Palette from its PLTE (§11.3, see
    /// [`Self::set_current_palette_from`]), which is the whole mechanism by
    /// which a later adaptive picture follows the scene. SQ-1196 returned early
    /// on a `scaled_cache` hit and so skipped that call for any base picture the
    /// session had already drawn once — and a game revisits its scenes. Arthur
    /// kept the church's brown frame after walking back out to the blue
    /// churchyard, and its F1 picture screen no longer came back the way it went
    /// away, because the second draw of Pict 4 established nothing.
    ///
    /// Only the RESAMPLE is cached here, which is the cost SQ-1196 set out to
    /// remove: `decode` itself is already cached ([`Self::cache`] /
    /// [`Self::adaptive_cache`]), so a repeat draw pays a hash lookup and an
    /// `Arc` clone, not a decompress.
    fn scaled_cached(
        &mut self,
        resnum: u32,
        scale: (u32, u32),
        decode: impl FnOnce(&mut Self) -> Option<Arc<DynamicImage>>,
    ) -> Option<Arc<DynamicImage>> {
        let source = decode(self)?;
        // Read `palette_gen` AFTER the decode: a base picture drawn on a
        // one-screen-palette machine (SQ-0887) is palette-dependent by
        // `is_palette_dependent` and yet bumps the generation on its way
        // through, so the pixels just decoded belong to the NEW generation.
        if self.is_palette_dependent(resnum) {
            let key = (resnum, self.palette_gen);
            if let Some(img) = self.adaptive_scaled_cache.get(&key) {
                return Some(Arc::clone(img));
            }
            let scaled = scale_art(&source, scale);
            self.adaptive_scaled_cache.insert(key, Arc::clone(&scaled));
            return Some(scaled);
        }
        if let Some(img) = self.scaled_cache.get(&resnum) {
            return Some(Arc::clone(img));
        }
        let scaled = scale_art(&source, scale);
        self.scaled_cache.insert(resnum, Arc::clone(&scaled));
        Some(scaled)
    }

    /// Remember Pict `resnum`'s PLTE as the Current Palette (§11.3). No-op for a
    /// non-indexed picture (no PLTE); bumps `palette_gen` only on a real change.
    ///
    /// A native archive names its palette in the directory record instead of a
    /// `PLTE` chunk, and answers here without decoding the picture's pixels; the
    /// 16 RGB triples it yields are the same shape a `PLTE` holds, so the
    /// Current Palette — including the copy a host Save State carries — is one
    /// representation across both archives.
    fn set_current_palette_from(&mut self, resnum: u32) {
        let plte = match (&self.blorb, &self.native) {
            (Some(b), _) => b
                .resource(b"Pict", resnum)
                .and_then(|(_ty, bytes)| png_plte(bytes)),
            (None, Some(pics)) => u16::try_from(resnum)
                .ok()
                .and_then(|id| pics.palette_of(id))
                .map(|pal| pal.concat()),
            (None, None) => None,
        };
        let Some(plte) = plte else {
            return;
        };
        if self.current_plte.as_deref() != Some(plte.as_slice()) {
            self.current_plte = Some(plte);
            self.palette_gen += 1;
            self.evict_stale_adaptive_cache();
        }
    }

    /// Decode an adaptive picture with the Current Palette spliced into its PLTE
    /// (§11.3), caching per `(resnum, palette_gen)`. With no base picture drawn
    /// yet the palette is undefined per spec; we fall back to the placeholder.
    fn adaptive_image(&mut self, resnum: u32) -> Option<Arc<DynamicImage>> {
        let key = (resnum, self.palette_gen);
        if !self.adaptive_cache.contains_key(&key) {
            // SQ-1197: a native picture's INDICES are the same under every
            // palette, so this miss costs a re-MAP off the retained index plane
            // (a table lookup per pixel) rather than a fresh decompress — which
            // is what a palette bump asks for, once per display-list op, when
            // `session::replay_under_current_palette` replots every window.
            let plane = self.blorb.is_none().then(|| self.index_plane(resnum)).flatten();
            let decoded = match (&self.blorb, &plane) {
                // Clone the raw PNG bytes so the immutable blorb borrow ends
                // before we mutate the cache.
                (Some(b), _) => b
                    .resource(b"Pict", resnum)
                    .map(|(_ty, bytes)| bytes.to_vec())
                    .and_then(|raw| {
                        let spliced = self
                            .current_plte
                            .as_ref()
                            .and_then(|plte| splice_plte(&raw, plte));
                        crate::cover::decode(spliced.as_deref().unwrap_or(&raw))
                    }),
                // A native picture is palette indices already: there is no PLTE
                // to splice, the Current Palette IS the colour table it expands
                // through. With none loaded yet it falls back to its own — §11.3
                // leaves that case undefined and the Blorb path does the same.
                //
                // A hardware table outranks the Current Palette outright: an EGA
                // or CGA picture's colours were the video card's, so there is
                // nothing for a loaded palette to adapt (SQ-0794). No archive
                // reaches here with one — `from_native` leaves such a source with
                // an empty adaptive set — but a restored Current Palette must not
                // be able to reach in through the replay path either.
                (None, Some(pic)) => {
                    let pal = self
                        .hw_palette
                        .or_else(|| self.current_plte.as_deref().map(colour_table));
                    native_image(pic, pal.as_ref(), self.blend_columns)
                }
                (None, None) => None,
            };
            self.adaptive_cache.insert(key, decoded.map(Arc::new));
        }
        self.adaptive_cache.get(&key).and_then(|o| o.clone())
    }

    /// The bytes + text-flag of Blorb `Data` resource `resnum`, for
    /// `glk_stream_open_resource`. `is_text` is true for a `TEXT` chunk, false
    /// for `BINA`/`FORM` (binary). `None` when there is no Blorb or no such Data
    /// resource. (The `PictSource` is AppGlk's sole Blorb holder, so Data lookup
    /// lives here alongside `Pict` lookup.)
    pub fn data_resource(&self, resnum: u32) -> Option<(Vec<u8>, bool)> {
        let (ty, bytes) = self.blorb.as_ref()?.resource(b"Data", resnum)?;
        Some((bytes.to_vec(), ty == b"TEXT"))
    }

    /// `(width, height)` of Pict `resnum`, sniffed from the image header only —
    /// no full decode. Used by the v6 Z-machine `picture_data` dimension table
    /// (Plan 1a), where only the size is needed at boot, not the pixels.
    ///
    /// A `Rect` chunk (Blorb §Rect: 8 bytes, width then height, big-endian) is a
    /// dimension-only placeholder with no pixels — Infocom v6 games (Zork Zero,
    /// Shogun, Arthur) query these via `picture_data` as invisible *placement*
    /// pictures whose (height, width) encode screen (y, x) layout coordinates.
    pub fn dims(&mut self, resnum: u32) -> Option<(u32, u32)> {
        if self.blorb.is_none() {
            // A native directory record carries the size whether or not the
            // entry has pixels, so a placeholder answers here just as a Blorb
            // `Rect` does — which is what those placeholders became on
            // conversion.
            let pics = self.native.as_ref()?;
            let e = pics.entry(u16::try_from(resnum).ok()?)?;
            return Some((u32::from(e.width), u32::from(e.height)));
        }
        let (ty, bytes) = self.blorb.as_ref()?.resource(b"Pict", resnum)?;
        if ty == b"Rect" {
            let b: &[u8] = bytes;
            if b.len() < 8 {
                return None;
            }
            let w = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            let h = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
            return Some((w, h));
        }
        image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()
    }

    /// `(number, width, height)` for every `Pict` resource in the Blorb, header-
    /// sniffed via [`PictSource::dims`]. Feeds the v6 `Machine::set_picture_dims`
    /// injection at session construction (Plan 1a). Empty when there is neither
    /// a Blorb nor a native archive.
    pub fn all_pict_dims(&mut self) -> Vec<(u16, u16, u16)> {
        let numbers: Vec<u32> = match (&self.blorb, &self.native) {
            (Some(b), _) => b.resources().iter().filter(|r| &r.usage == b"Pict").map(|r| r.number).collect(),
            (None, Some(pics)) => pics.entries().iter().map(|e| u32::from(e.id)).collect(),
            (None, None) => Vec::new(),
        };
        numbers
            .into_iter()
            .filter_map(|n| self.dims(n).map(|(w, h)| (n as u16, w as u16, h as u16)))
            .collect()
    }
}

/// The Infocom Version 6 standard window: the ART resolution every v6 release
/// was laid out against, which lanthorn presents doubled as the 640×400 unit
/// screen (SQ-0479). It is the same on every machine that shipped one of these
/// games, and it is what every Infocom Blorb's `Reso` chunk declares — a native
/// archive has no chunk to declare it with, so this stands in.
///
/// It is NOT the archive's picture space, which is 320 or 640 depending on how
/// wide the card's pixels were; see [`PictSource::art_scale`].
pub const INFOCOM_V6_STD_WINDOW: (u16, u16) = (320, 200);

/// Tier 3 of the picture-resource policy (SQ-0734): the user names a native
/// Infocom archive in the per-game sidecar `<game_dir>/config.toml` and thereby
/// ASSERTS that it belongs to this story.
///
/// ```toml
/// pictures = "FMVPOKER.EG1"
/// ```
///
/// # Why the user has to say it
///
/// The three tiers are ordered by confidence in the PAIRING, and nothing below
/// tier 3 can be guessed. A Blorb validates its own contents. A disk image ties
/// story and archive together by the medium they shipped on. A loose archive
/// beside a story ties them together by *nothing*: the format carries no release
/// number and no serial, every Infocom Amiga release names its archive
/// `Pic.data`, and the PC names are a DOS 8.3 convention that survives neither a
/// renamed story nor a renamed archive. A stem rule would therefore have to be
/// wrong sometimes, and being wrong here is INVISIBLE — Arthur's plates drawn
/// into Zork Zero look like art, not like an error. So there is no
/// auto-discovery, and this is deliberate rather than unfinished.
///
/// **Discovery for DISPLAY is safe; discovery for PAIRING is not.** Those look
/// contradictory and are not, and the difference is the whole policy. Listing
/// the archives that happen to sit beside a story, and showing that list to a
/// person, is fine — better than fine, because the person knows which game they
/// own and can supply the assertion the file format cannot make. Taking the same
/// list and *picking from it* is what has no evidence behind it. If a future
/// feature enumerates candidates (SQ-0789 proposes exactly that, for a picker),
/// it must hand them to a human and end there; wiring an enumerator into this
/// function would reintroduce precisely the failure the tiers exist to prevent.
/// Nothing here enumerates anything today, on purpose.
///
/// # What naming one buys
///
/// It wins outright. A named archive that loads beats a resource Blorb beside
/// the story and beats the `Pic.data` an `.adf` carries: naming it is an
/// instruction, not a hint. It also picks the machine — see
/// [`crate::interpreter::InterpreterProfile::resolve`], which takes
/// [`PictureOverride::flavour`] as its second-most-specific input.
///
/// # What a bad name costs
///
/// Nothing silently. A file that is absent, or present and undecodable, leaves
/// the Blorb in charge but produces a [`PictureOverride::warning`] the host must
/// show. Falling back quietly would recreate the exact failure the policy exists
/// to prevent, inverted: the player believing they are seeing native art when
/// they are not, with nothing on screen to say otherwise.
#[derive(Debug)]
pub enum PictureOverride {
    /// No `pictures` key. Tiers 1 and 2 decide, exactly as before.
    Unset,
    /// The key names a file that is not there. "If the file exists" was the
    /// user's condition for the override winning, so it does not apply — but the
    /// user asked for something they did not get, and hears about it.
    Missing { path: std::path::PathBuf },
    /// The named file exists and cannot be used: unreadable, the wrong container
    /// flavour, corrupt, or truncated. Loud on purpose.
    Unusable { path: std::path::PathBuf, reason: String },
    /// The named file is a native archive, and it wins.
    ///
    /// `refused` carries the complaint when a file sat under the next part's
    /// name and turned out not to be one (SQ-0798). The archive still loads —
    /// part 1 is exactly what the user named — but the continuation is never
    /// dropped in silence.
    Loaded {
        path: std::path::PathBuf,
        pics: blorb::infocom_pics::InfocomPics,
        refused: Option<String>,
    },
}

impl PictureOverride {
    /// Read and validate the `pictures` key of `<game_dir>/config.toml`.
    ///
    /// A relative value resolves against the STORY's own directory — "beside the
    /// story" is where these archives sit, and the sidecar lives elsewhere, in
    /// the per-game save directory. An absolute value is used as given.
    ///
    /// The file is parsed here, not merely stat'ed, for two reasons: the flavour
    /// it turns out to be selects the interpreter profile, and a file that will
    /// not decode must be reported before the story boots rather than discovered
    /// picture by picture as blanks.
    pub fn resolve(story_path: &std::path::Path, game_dir: &std::path::Path) -> PictureOverride {
        PictureOverride::resolve_with_session(story_path, game_dir, None)
    }

    /// [`resolve`](PictureOverride::resolve), with a name supplied for THIS
    /// launch taking precedence over the sidecar's key.
    ///
    /// The session name is the other two doors into this mechanism (SQ-0791 /
    /// SQ-0789): `--pictures` on the command line, and the launch-options
    /// dialog's un-persisted choice. Both outrank the config key, matching how an
    /// explicit interpreter number already outranks the inferred one and the
    /// general rule that the more specific and more recent instruction wins.
    ///
    /// Everything downstream is unchanged: the archive still has to parse, it
    /// still beats a Blorb and an `.adf`'s own `Pic.data`, its flavour still
    /// picks the machine, and a name that is absent or will not decode is still
    /// loud. A door is not a policy.
    ///
    /// # Naming a file that is INSIDE the medium
    ///
    /// A story mounted out of a disk image has no directory to put a loose
    /// archive next to, and the archive a user would want to name is already on
    /// the volume. So when the bare name does not exist on the host filesystem
    /// and the story is a disk image, the name is looked up on the volume
    /// instead — see [`read_off_the_medium`].
    ///
    /// This is what makes the Macintosh's **monochrome** `Pic.data` reachable
    /// (SQ-0838). Its disk carries two archives, `Hfs::pictures` hands back the
    /// colour one, and choosing the other is a preference nothing on the disk
    /// states — so it is the user's to state, with `--pictures Pic.data`, and
    /// there is nowhere else for that name to point. An Amiga `.adf` gains the
    /// same door by the same code, which is the point: a door, not a policy.
    pub fn resolve_with_session(
        story_path: &std::path::Path,
        game_dir: &std::path::Path,
        session: Option<&str>,
    ) -> PictureOverride {
        let Some(name) = session
            .map(str::to_string)
            .or_else(|| crate::styles::read_per_game_pictures(game_dir))
        else {
            return PictureOverride::Unset;
        };
        let named = std::path::Path::new(&name);
        let path = if named.is_absolute() {
            named.to_path_buf()
        } else {
            story_path.parent().unwrap_or(std::path::Path::new(".")).join(named)
        };
        let raw = match std::fs::read(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match read_off_the_medium(story_path, named) {
                    Some(raw) => raw,
                    None => return PictureOverride::Missing { path },
                }
            }
            Err(e) => return PictureOverride::Unusable { path, reason: e.to_string() },
        };
        match blorb::infocom_pics::InfocomPics::parse(raw) {
            Ok(mut pics) => {
                let refused = absorb_continuations(&mut pics, &path);
                PictureOverride::Loaded { path, pics, refused }
            }
            Err(e) => PictureOverride::Unusable { path, reason: e.to_string() },
        }
    }

    /// The flavour of the archive the user named, for interpreter-profile
    /// selection. `None` whenever no usable archive was named, which leaves the
    /// medium and then the default to decide.
    pub fn flavour(&self) -> Option<blorb::infocom_pics::Flavour> {
        match self {
            PictureOverride::Loaded { pics, .. } => Some(pics.flavour()),
            PictureOverride::Unset
            | PictureOverride::Missing { .. }
            | PictureOverride::Unusable { .. } => None,
        }
    }

    /// The standard window — the machine's native ART resolution — that the
    /// named archive implies, for the `v6_screen_px` chain at boot.
    ///
    /// A native archive has no `Reso` chunk, because **the format has no such
    /// concept** (SQ-0736). Blorb §11 makes the ABSENCE of `Reso` meaningful —
    /// non-scalable art, drawn one image pixel per screen pixel — so reading a
    /// native archive's silence as that declaration is what left Zork Zero's
    /// 320×200 art at half size on a 640×400 screen. The archive is not silent:
    /// the standard window is the machine's, and every machine that wrote one of
    /// these archives drew v6 on the same one.
    ///
    /// Nearly every rendition answers with the SAME standard window, and that is
    /// the point (SQ-0790). A 320-wide archive (Amiga/Mac colour, MCGA `.MG1`)
    /// and a 640-wide one (EGA `.EG1`/`.EG2`, CGA `.CG1`) are two drawings of
    /// one screen, not two screens: the EGA plate has twice as many pixels
    /// across because each is half as wide, so both cover the same rectangle.
    /// What differs between them is the DENSITY of the art, which is
    /// [`PictSource::art_scale`]'s business — 320-wide doubles onto the 640×400
    /// unit screen, 640-wide is x1 across and x2 down. Answering with the
    /// picture space and letting the scale close the gap says exactly that, and
    /// says it for the one rendition that really is a second screen: the
    /// standard Macintosh's 480×300 monochrome plate, which lands 1:1 on a
    /// 480×300 screen. See [`PictSource::native_std_window`] for the table and
    /// the sources; mounting an archive off the disk it shipped on and naming
    /// that very file by hand must not produce different geometry, so the two
    /// are deliberately one answer.
    ///
    /// SQ-0734 shipped `None` here for a 640-wide archive as an explicit
    /// deferral, on the reading that its true presentation was a 640×200 screen
    /// on an 8×8 cell that `V6_ART_SCALE` could not express. The screen half of
    /// that turned out to be wrong: 640×200 on an 8×8 cell is 80×25 characters,
    /// which is the same character grid the 640×400 unit screen already provides
    /// on its 8×16 cell. Only the art density was ever different, and a per-axis
    /// art scale is the whole of it.
    pub fn std_window(&self) -> Option<(u16, u16)> {
        match self {
            // The archive's own picture space, rather than
            // `interpreter::AMIGA_STD_WINDOW`: for an MCGA `.MG1` the two are
            // the same numbers, but naming the Amiga's constant here would read
            // as a claim that an MCGA archive is Amiga media, and it is not.
            PictureOverride::Loaded { pics, .. } => {
                Some((pics.picture_space_width(), pics.picture_space_height()))
            }
            _ => None,
        }
    }

    /// The complaint to show the user, naming the file and the reason. `None`
    /// when there is nothing to complain about — no key, or a key that worked.
    ///
    /// A key that worked can still have one thing to say: an archive loaded
    /// whose *continuation* was refused draws with fewer pictures than the set it
    /// claims to be, which is the SQ-0798 defect wearing a different hat.
    pub fn warning(&self) -> Option<String> {
        match self {
            PictureOverride::Unset => None,
            PictureOverride::Loaded { refused, .. } => refused.clone(),
            PictureOverride::Missing { path } => Some(format!(
                "pictures = \"{}\" names a picture archive that is not there — \
                 falling back to this story's Blorb art",
                path.display(),
            )),
            PictureOverride::Unusable { path, reason } => Some(format!(
                "pictures = \"{}\" cannot be used: {reason} — \
                 falling back to this story's Blorb art",
                path.display(),
            )),
        }
    }
}

/// The path of part `part` of the multi-part set `path` belongs to, or `None`
/// when the name cannot express one (SQ-0798).
///
/// **The format states the part number, and the filename carries it.** Header
/// byte 0 is the part; Frotz's DOS port turns that number straight back into a
/// filename — `extension[3] = '0' + number`, under the comment *"EGA pictures
/// may be stored in two separate graphics files"* (`src/dos/bcpic.c`) — and its
/// `open_graphics_file(int number)` takes the part as a parameter for exactly
/// this reason. So the rule here is Infocom's own: replace the final character
/// of the extension with the part's digit, leaving everything else, case
/// included, untouched. `Pic.data` has no trailing digit and therefore no
/// continuation, which is correct — the Amiga releases ship one file.
///
/// # This is NOT the stem-based discovery SQ-0734 rejected
///
/// Those look alike and are opposites, and the difference is the whole of the
/// tier policy. What SQ-0734 forbids is pairing an archive to a **story** on the
/// evidence of a name: `arthur.z6` sitting beside `arthur.mg1` proves nothing,
/// every Amiga release calls its archive `Pic.data`, and a wrong pairing draws
/// Arthur's plates into Zork Zero with nothing on screen to say so.
///
/// Nothing of that kind happens here. The pairing has **already been asserted**
/// — by a user naming the archive outright, which is tier 3 — and this only
/// follows that one archive's own in-band part number to the rest of itself. The
/// story is not consulted and could not be. What is guessed is a filename; what
/// is then *verified* is the header, by
/// [`blorb::infocom_pics::InfocomPics::append_part`], which refuses a file whose
/// part byte, codec or picture ids say it is not the continuation. A stem rule
/// has nothing to verify against; this one does.
/// [`part_path`] over a NAME rather than a host path — what a file on a medium
/// has, where `PC/ARTHUR/ARTHUR.EG1` is a name the volume spells and not
/// anywhere on this machine (SQ-0881).
///
/// The two share their whole rule, because they are one rule: only the last
/// character of the extension changes, and only `1..=9` is a part.
pub fn part_name(name: &str, part: u8) -> Option<String> {
    if !(1..=9).contains(&part) {
        return None;
    }
    let (stem, ext) = name.rsplit_once('.')?;
    let mut ext: Vec<u8> = ext.as_bytes().to_vec();
    if !ext.last()?.is_ascii_digit() {
        return None;
    }
    *ext.last_mut()? = b'0' + part;
    Some(format!("{stem}.{}", String::from_utf8(ext).ok()?))
}

pub fn part_path(path: &std::path::Path, part: u8) -> Option<std::path::PathBuf> {
    // One digit is all a DOS 8.3 extension can hold, and it is all `bcpic.c`
    // writes; 0 is not a part number.
    if !(1..=9).contains(&part) {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    let (stem, ext) = name.rsplit_once('.')?;
    let mut ext: Vec<u8> = ext.as_bytes().to_vec();
    if !ext.last()?.is_ascii_digit() {
        return None;
    }
    *ext.last_mut()? = b'0' + part;
    let ext = String::from_utf8(ext).ok()?;
    Some(path.with_file_name(format!("{stem}.{ext}")))
}

/// The artwork the release `story_path` came off supplies for it, or `None` when
/// it supplies none (SQ-0862).
///
/// Tier 2 of the picture policy, widened from the platter to the release. Which
/// volumes may answer at all is [`crate::assets::volumes`]'s question, and the
/// interesting half of the argument is there; this only decides which of the
/// answers to take when more than one volume has one.
///
/// # The order, and the one preference in it
///
/// **The story's own volume wins outright.** It is the strongest pairing on
/// offer and it is what every single-image release already resolved to, so
/// nothing that worked before this function existed moves — the 720K Zork Zero
/// press keeps the `ZORK0.MG1` sharing its story's disk and does not start
/// preferring disk 2's CGA.
///
/// Among the siblings, **colour beats monochrome**, and then the disk order the
/// set is already in. `blorb`'s per-volume `pictures()` deliberately expresses no
/// preference between video cards — and it never had to, because no image in the
/// corpus carries two renditions. A release does: the DOS 360K press offers CGA
/// on disk 1 and EGA on disk 3, and taking the earlier disk would hand a terminal
/// with sixteen million colours a two-colour Zork Zero. CGA's two colours are a
/// 1989 hardware constraint, not an authorial choice, so a rendition that kept
/// its colour is the better default. It is only a default: every rendition on the
/// release is listed in the launch dialog and reachable by name, which is where
/// a person who wants the CGA plates says so.
///
/// **MCGA against EGA is decided now** (SQ-0880), and by `blorb::medium::art_preference`
/// rather than here, so one volume's folders and a release's volumes rank alike.
/// It was left open while no release put two colour renditions where a choice
/// had to be made; *The Lost Treasures of Infocom II* puts `ARTHUR.MG1` and
/// `ARTHUR.EG1` in one folder for three games. See that function for why the
/// picture count cannot settle it and why 256 colours beats 640 pixels.
///
/// # Why it also reports WHICH archive it took (SQ-0865)
///
/// The launch dialog's default row has to name the archive accepting it will
/// open, and the only way for that row to be trustworthy is for it to come from
/// this function rather than from a second copy of the rule above. So the answer
/// carries the archive's stored name and the volume it came off, and the choice
/// itself is untouched: the sort key below is the same key, applied to the same
/// stable sort, so every release resolves to exactly the archive it did before.
/// # Which story, on a disc that holds several (SQ-0876)
///
/// `disk_entry` is the story's own name on the volume — the browser row's, the
/// same selector [`crate::hints::load_mounted_story_from`] takes. A volume that
/// keeps its games in folders answers for that story alone, and a story with no
/// artwork beside it gets none rather than a stranger's.
///
/// `None` means "whatever this release's artwork is", which is every single-game
/// press and is exactly the old behaviour.
pub fn release_art(
    story_path: &std::path::Path,
    disk_entry: Option<&str>,
) -> Option<ReleaseArt> {
    let volumes = crate::assets::volumes(story_path);
    let (own, siblings) = volumes.split_first()?;
    // The story's own volume wins outright, as ever — but WHICH archive on it is
    // now the story's question when the volume can tell games apart.
    let own_art = match disk_entry {
        Some(entry) => own.disk.pictures_for(entry),
        None => own.disk.pictures(),
    };
    if let Some(art) = own_art {
        return Some(ReleaseArt {
            pictures: art.pictures,
            name: art.name,
            disk_number: own.disk_number,
        });
    }
    let mut found: Vec<ReleaseArt> = siblings
        .iter()
        .filter_map(|v| {
            v.disk.pictures().map(|a| ReleaseArt {
                pictures: a.pictures,
                name: a.name,
                disk_number: v.disk_number,
            })
        })
        .collect();
    // Stable, so disk order survives as the last tiebreak.
    found.sort_by_key(|a| {
        // The same preference `blorb` applies within one volume, so a sibling
        // volume cannot be ranked by a different rule from a sibling folder
        // (SQ-0880).
        (blorb::medium::art_preference(&a.pictures), std::cmp::Reverse(a.pictures.entries().len()))
    });
    found.into_iter().next()
}

/// The artwork a release supplies for a story, and where on the release it is
/// (SQ-0865).
#[derive(Debug)]
pub struct ReleaseArt {
    /// The parsed archive — what [`PictSource::resolve`] draws with.
    pub pictures: blorb::infocom_pics::InfocomPics,
    /// The archive's name as its volume spells it, e.g. `ZORK0.EG1`.
    pub name: String,
    /// The disk number of the volume it came off, or `None` when the release is
    /// a single image. See [`crate::assets::MountedVolume::disk_number`].
    pub disk_number: Option<u64>,
}

/// The resource Blorb that may speak for `story_path`'s artwork, and the
/// complaint when one was found and refused (SQ-0866).
#[derive(Debug)]
pub struct ResourceBlorb {
    /// The Blorb to draw from, and where it was read. `None` when the story has
    /// none — **or when the one it has was refused**, which is why the two
    /// fields are not an either/or enum: a caller that only wants to draw reads
    /// `found` and needs no arm for the refusal.
    pub found: Option<(blorb::Blorb, std::path::PathBuf)>,
    /// Why a Blorb that WAS found is not being used, in the words the host shows
    /// the player. Always `None` when `found` is `Some`.
    pub refused: Option<String>,
}

/// Tier 1 of the picture policy: the story's own resource Blorb, **unless it
/// says it belongs to a different build** (SQ-0866).
///
/// # The defect this exists to end
///
/// `Arthur Quest 4 Excalibur.2mg` is the Apple IIgs press of *Arthur*, release
/// 63 / serial 890622, and it carries 168 pictures. Its own volume offers
/// lanthorn no artwork it can read yet, so tier 1 ran, and
/// [`blorb::resolve_resource_blorb`]'s directory scan matched `Arthur.blb` on a
/// six-character stem prefix — a Blorb built for release 74 / serial 890714, the
/// DOS press, holding **326** pictures. The game asked for its picture numbers
/// and got another build's, which is the corruption the user reported.
///
/// # The rule, and where its line falls
///
/// A Blorb is refused when it **contradicts** the story: it carries the Blorb
/// spec's optional `IFhd` Game Identifier, the story came off a disk image, and
/// the identifier matches no build on that release. The spec asks for exactly
/// this — *"the interpreter can check that the game matches the IFhd chunk. If
/// they don't, the interpreter should display an error"* — and refusing is that
/// error, made quiet on screen and loud in the message.
///
/// Three ways to escape it, and each is a deliberate *absence of contradiction*
/// rather than an exception:
///
/// - **The Blorb states no build.** Most of the corpus: every modern `.zblorb`,
///   `advent.blb`, `Sherlock.blb`, `beyondzork.blb`, all eleven Mysterious
///   Adventures sidecars. There is nothing to contradict, and reading silence as
///   disagreement would strip artwork from nearly every story that has any. "Does
///   not say" and "says something else" are different facts and are kept so.
/// - **The story is not on a medium.** A loose story file sits in a directory a
///   *person* assembled, and that placement is itself the pairing assertion. The
///   corpus states the case outright: `fmvpoker.blb` is a byte-for-byte copy of
///   `Zork0.blb`, so its `IFhd` names Zork Zero while `fmvpoker.z6` is release 60
///   / serial 001227 — and *Frobozz Magic Video Poker*'s own readme instructs the
///   player to do that ("Obtain one of the Zork Zero graphics files… rename the
///   graphics file to FMVPOKER"). Borrowing another game's plates is the whole
///   design of that game. lanthorn does not overrule a person who has already
///   answered the question.
///
///   A disk image is the opposite case and that is the whole distinction: nobody
///   put `Arthur.blb` *beside* `Arthur Quest 4 Excalibur.2mg`. Both are in a
///   library folder next to two hundred unrelated files, and a shared stem is a
///   coincidence of naming, not an act. `crate::assets::AssetOrigin` already
///   draws this line — a file on the medium "shipped in the box with the story…
///   which is exactly what a loose file's name has to be tested for".
/// - **No story on the release could be identified.** Nothing to compare against,
///   so nothing is proven, and the Blorb keeps drawing. The rule tightens by
///   itself as the readers improve, rather than guessing now, and SQ-0867 is that
///   happening: [`release_builds`] could once only ask each volume on its own, so
///   the Apple II presses that page a story across a whole SET — `Journey.2mg`
///   and the five-volume `shogun_s*.dsk` — were unidentifiable however plainly
///   they stated their build. Both are identified now, and `Shogun.blb`'s release
///   322 against that press's release 311 is refused on the same evidence
///   `Arthur.blb` was.
///
/// # What it never touches
///
/// The medium's own artwork ([`release_art`]) is resolved first and is never
/// consulted here, so every disk that draws today keeps drawing from the same
/// archive. A named archive (`--pictures`, the per-game `pictures` key) outranks
/// this whole function — that is the user asserting the pairing, and a mismatch
/// they asked for is theirs to make. And a Blorb that IS the story file is its
/// own container by construction and is never tested.
pub fn resource_blorb(story_path: &std::path::Path) -> ResourceBlorb {
    // A ZIP the player downloaded is asked FIRST, and only when the story came
    // out of one (SQ-1085). The archive holding both the story and its `.blb` is
    // one download, which is the relation `blorb::resolve_resource_blorb`'s tier
    // 1 already treats as conclusive for a `.zblorb`; a same-stem file sitting
    // beside the zip is a person's own filing and keeps the tier it always had.
    // `zip_resource_blorb` opens with a four-byte magic check, so a loose story
    // pays nothing for this.
    let found = crate::hints::zip_resource_blorb(story_path)
        .or_else(|| blorb::resolve_resource_blorb(story_path));
    let Some((blorb, path)) = found else {
        return ResourceBlorb { found: None, refused: None };
    };
    match build_mismatch(story_path, &blorb, &path) {
        Some(refused) => ResourceBlorb { found: None, refused: Some(refused) },
        None => ResourceBlorb { found: Some((blorb, path)), refused: None },
    }
}

/// The refused-Blorb complaint a player needs to SEE, or `None` (SQ-0882).
///
/// [`resource_blorb`]'s refusal is a fact about a sidecar. Whether it is news
/// depends on something it deliberately does not look at: whether the medium
/// carries artwork of its own. [`PictSource::resolve`] takes that first and only
/// falls through to a sidecar when there is none, so on a disk that draws from
/// its own archive the refusal changed nothing — it declined a file that was
/// never going to be reached.
///
/// SQ-0866 put the warning in for one reason, stated in its own words: *"it is
/// only honest if the player is told why their disk has no pictures"*. A disk
/// that HAS pictures is outside that warrant, and telling it anyway is worse
/// than silence, because the only sentence the player gets ends "a different
/// build's pictures are not being drawn" and reads as *your artwork is missing*.
/// Both Amiga and Apple II presses of Arthur are in that position — `Pic.data`
/// and `ARTHUR.1/ARTHUR.D1` draw, while `Arthur.blb` (release 74, serial 890714)
/// is refused against release 54/890606 and 63/890622 — and both were reporting
/// it at every boot.
///
/// Asking what WON rather than what was declined also keeps this honest as the
/// readers improve: a medium lanthorn learns to read artwork off stops warning
/// by itself, with nothing here to update.
pub fn unpaired_art_warning(
    story_path: &std::path::Path,
    disk_entry: Option<&str>,
) -> Option<String> {
    if release_art(story_path, disk_entry).is_some() {
        return None;
    }
    resource_blorb(story_path).refused
}

/// Why the Blorb at `path` may not speak for `story_path`, or `None` when
/// nothing contradicts it. The rule, and the argument for where its line falls,
/// are [`resource_blorb`]'s.
fn build_mismatch(
    story_path: &std::path::Path,
    blorb: &blorb::Blorb,
    path: &std::path::Path,
) -> Option<String> {
    // A `.zblorb`/`.gblorb` holding its own story: tautologically its own build.
    if path == story_path {
        return None;
    }
    let stated = blorb.game_identifier()?; // states no build: nothing to contradict
    let on_release = release_builds(story_path);
    if on_release.is_empty() || on_release.contains(&stated) {
        return None;
    }
    Some(format!(
        "{} is the artwork for {stated}, but this disk is {} — \
         a different build's pictures are not being drawn",
        path.file_name().unwrap_or(path.as_os_str()).to_string_lossy(),
        on_release.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", "),
    ))
}

/// Every build the release `story_path` came off carries, in disk order.
///
/// Empty when the story is not a disk image at all — which is the ordinary case
/// and the one that costs nothing, because [`crate::assets::volumes`] refuses a
/// story file before mounting anything.
///
/// Also empty when the release mounts and lanthorn can identify nothing on it. An
/// empty answer is treated as "no evidence" by [`resource_blorb`] and never as
/// "no match"; see its docs.
///
/// # A story on no single volume (SQ-0867)
///
/// Asking each volume on its own is right for every release that puts a story
/// FILE on a platter, and blind to the ones that do not. The Apple II presses of
/// the graphical Version 6 games page one story across the whole set as opaque
/// `.D1`…`.D5` segments — the five-volume `shogun_s*.dsk` is five floppies of
/// which not one carries a story, and `Journey.2mg` is one image of the same
/// shape — so every volume mounted, every volume answered "nothing here", and a
/// release whose build is written plainly on it read as unidentifiable.
///
/// So when no volume speaks for itself, the release is asked as a whole.
/// [`blorb::infocom_packed::story_header`] reads the segment index and the one
/// page it names as page 0, which is where release, serial and checksum live;
/// see that function for why one page is enough and what stands in for the
/// checksum [`blorb::infocom_packed::story`] would have verified.
///
/// # Cost
///
/// The second arm runs only when the first found nothing, and in the release it
/// is for it adds **no disk read at all**: [`crate::assets::volumes`] has already
/// mounted every volume of the set — it has to, since SQ-0862, so that a sibling
/// floppy's artwork can be found — and this reads their already-open contents.
/// What it adds is one index parse and one 512-byte page.
///
/// Above it sits a stronger gate still: production reaches this only through
/// [`build_mismatch`], which returns before asking when the Blorb states no build
/// of its own. A release with no stem-matching Blorb, or one whose Blorb is
/// silent, never pays even the mount.
pub fn release_builds(story_path: &std::path::Path) -> Vec<blorb::GameIdentifier> {
    let volumes = crate::assets::volumes(story_path);
    let on_a_volume: Vec<blorb::GameIdentifier> = volumes
        .iter()
        .flat_map(|v| v.disk.stories())
        .filter_map(|s| blorb::GameIdentifier::of_story(&s.bytes))
        .collect();
    if !on_a_volume.is_empty() {
        return on_a_volume;
    }
    packed_across_the_set(&volumes).into_iter().collect()
}

/// The build a release pages ACROSS its volumes states in its own header, or
/// `None` when these volumes are not one of those releases (SQ-0867).
fn packed_across_the_set(
    volumes: &[crate::assets::MountedVolume],
) -> Option<blorb::GameIdentifier> {
    // The story's own volume first, so which floppy a person opened cannot
    // change the answer — `blorb::medium`'s reassembly orders them the same way
    // and for the same reason.
    let files: Vec<(String, Vec<u8>)> =
        volumes.iter().flat_map(|v| v.disk.contents()).collect();
    let (_, header) = blorb::infocom_packed::story_header(&files)?;
    blorb::GameIdentifier::of_story(&header)
}

/// Read a bare filename off the release `story_path` was mounted from, for a
/// user naming an archive that lives INSIDE the medium (SQ-0838).
///
/// The Macintosh release is the case that needs it: its disk carries a colour
/// `CPic.data` and a monochrome `Pic.data`, the automatic choice is colour, and
/// the other one exists nowhere on the host filesystem for `--pictures` to point
/// at. An Amiga `.adf` is read by the same three lines.
///
/// **Only after the host filesystem has been tried and come up empty**, and
/// never for an absolute path, which is an instruction about the host and is
/// honoured as one. A name that resolves to a real file beside the story still
/// wins, so nothing that worked before moves. The lookup is by name because that
/// is what the user typed — the CONTENT tiers ([`PictSource::resolve`]) are what
/// identify an archive nobody named.
///
/// # Every volume of the release, not just the story's own (SQ-0862)
///
/// A name the launch dialog showed must be a name this can find, and the dialog
/// lists the whole release's artwork — booting the DOS 360K Zork Zero's disk 2
/// offers `ZORK0.EG1`, which is on disk 3. Both doors ask
/// [`crate::assets::volumes`] so they cannot answer differently; the story's own
/// volume is searched first, so a name that resolved before still resolves to the
/// same file.
///
/// # One mount path, and why this had to become one (SQ-0833)
///
/// This read `if looks_like_adf … else if looks_like_hfs` until the DOS and
/// Atari ST formats arrived — the last copy of the two-reader chain SQ-0840
/// replaced everywhere else, missed because it predates `MountedDisk`. It was
/// merely stale while two formats existed and became a **defect** the moment a
/// third registered: `crate::assets::files` enumerates a disk's artwork through
/// `blorb::medium` and so offered a FAT12 disk's `ZORK0.EG1` in the launch
/// dialog, while this function had no arm that could load it. Offered, picked,
/// and nothing drawn. It goes through the one table now, so the next format
/// cannot reintroduce the gap.
///
/// # A name may carry a directory now
///
/// It used to be rejected outright, on the reasonable ground that a directory is
/// an instruction about where to look — reasonable while no volume HAD
/// directories. FAT12 does: every story on an Atari ST compilation is called
/// `STORY.DAT` and the folder is the only thing that tells four of them apart,
/// so `HITCHHIK/STORY.DAT` is the volume's own spelling and `contents` shows it
/// that way. What a caller is shown, it must be able to ask for. The components
/// are re-joined with `/` so a Windows user's backslashes reach the same file;
/// AmigaDOS and HFS are flat, so a name with a separator simply matches nothing
/// there, exactly as before.
fn read_off_the_medium(story_path: &std::path::Path, named: &std::path::Path) -> Option<Vec<u8>> {
    if named.is_absolute() {
        return None;
    }
    let parts: Option<Vec<&str>> =
        named.components().map(|c| c.as_os_str().to_str()).collect();
    let name = parts?.join("/");
    crate::assets::volumes(story_path).iter().find_map(|v| v.disk.read_named(&name))
}

/// Merge every continuation of `path` into `pics`, and return the complaint if
/// one was found and refused (SQ-0798).
///
/// # How far it looks
///
/// **Until a part is missing.** Arthur and Journey stop at two, but the format
/// does not: the part byte is a whole byte and `open_graphics_file(int number)`
/// takes any number, so a title we do not have could ship three. Stopping at 2
/// would be a corpus fact hardcoded as a rule, and it would fail silently — the
/// symptom is missing pictures, which is precisely the defect this fixes.
/// Scanning instead costs one `open` of a file that is not there per set, once
/// at boot, and the naming bounds the walk on its own: a DOS extension holds one
/// digit, so [`part_path`] answers `None` past part 9 and the loop cannot run
/// away.
///
/// An absent next part is the ordinary end of the walk and says nothing. A part
/// that is *present* and does not check out is refused and reported: SQ-0734's
/// rule that an unusable named archive must be loud applies to its continuation,
/// because a half-loaded set looks exactly like a complete one until the game
/// draws the picture that is not there.
pub fn absorb_continuations(
    pics: &mut blorb::infocom_pics::InfocomPics,
    path: &std::path::Path,
) -> Option<String> {
    while let Some(next) = part_path(path, pics.next_part()) {
        let Ok(raw) = std::fs::read(&next) else {
            return None; // no such part: the set ends here, as most do.
        };
        let outcome = blorb::infocom_pics::InfocomPics::parse(raw)
            .and_then(|part| pics.append_part(part));
        if let Err(e) = outcome {
            return Some(format!(
                "{} sits under this archive's next part name but {e} — \
                 using only the parts that check out",
                next.display(),
            ));
        }
    }
    None
}

/// Colourise one already-decoded native picture into the same `DynamicImage`
/// the Blorb path yields (SQ-0719).
///
/// `Picture::rgba` already expands the palette indices and gives the
/// transparent index alpha 0, which is exactly what `Canvas`'s alpha-honoring
/// overlay wants. `None` is the one case left after the decode: an index plane
/// whose length disagrees with its own `width * height`.
///
/// `palette`, when given, overrides the picture's own — the Current Palette an
/// adaptive picture is drawn through (SQ-0743).
///
/// This used to take the ARCHIVE and a resnum and decompress the picture itself,
/// which made a palette change an O(decode) event. It takes the decoded index
/// plane instead (SQ-1197), so a palette change is O(pixels): the plane comes
/// from [`PictSource::index_plane`], which decodes each picture once, and
/// everything below this line — the table lookup, the transparency, the dither
/// fuse — is what actually varies with the palette.
fn native_image(
    pic: &blorb::infocom_pics::Picture,
    palette: Option<&[blorb::infocom_pics::Rgb; 16]>,
    blend_columns: bool,
) -> Option<DynamicImage> {
    let rgba = match palette {
        Some(pal) => pic.rgba_with(pal),
        None => pic.rgba(),
    };
    let mut buf = image::RgbaImage::from_raw(u32::from(pic.width), u32::from(pic.height), rgba)?;
    if blend_columns {
        blend_half_width_columns(&mut buf);
    }
    Some(DynamicImage::ImageRgba8(buf))
}

/// Fuse a 640-wide rendition's column dither, because its pixels are half as wide
/// as the unit screen's (SQ-0797).
///
/// # Why there is anything to fuse
///
/// EGA has no bronze, so Zork Zero's artist made one. The proscenium arch is a
/// column-by-column dither — bright red (index 12) against brown (index 6) for
/// the lit stone, light grey against bright red for the highlights, brown against
/// black for the shadow — and on a 640×200 EGA screen those columns are half as
/// wide as an MCGA pixel, so the card and the eye fused each pair into a colour
/// the palette does not contain. Bocfel says the same of Zork Zero's EGA hint
/// background (`z6/draw_border.cpp:745`): "no single pixel of the artwork is the
/// colour the eye actually sees". lanthorn keeps all 640 columns — geometrically
/// right, [`PictSource::art_scale`] maps them onto exactly the rectangle a
/// 320-wide plate covers — so without this the dither arrives at full contrast
/// and the arch reads as salmon-and-olive speckle.
///
/// # Why here and not in the renderer
///
/// Because the unit-space→pane scale MAGNIFIES at every pane worth playing on, and
/// magnification is `FilterType::Nearest`, deliberately: crisp DOS pixels are the
/// house style, and a nearest resample never blends. Above 640 px it replicates the
/// columns, so the dither arrives at full contrast however wide the pane is; below
/// 640 the resampler now takes the area arm (SQ-0824,
/// [`resize_directional`](crate::render::graphics::resize_directional)) and would
/// fuse the pair itself — but that was never a reason to leave the artwork alone,
/// because it would make the fused colour a function
/// of how wide the player's terminal happens to be. Measured horizontal speckle on
/// Zork Zero's EGA border ran 22.3 / 40.3 / 49.1 / 39.2 / 24.4 at pane widths of
/// 320 / 480 / 640 / 800 / 1280, against 4.3 for the same frame in MCGA. Fusing
/// at the archive boundary instead makes the answer a property of the artwork.
///
/// # The filter
///
/// A three-tap tent, `[1, 2, 1] / 4`, across columns only. It is chosen over the
/// obvious two-tap box on fixed column pairs because it is PHASE-INDEPENDENT: for
/// any period-2 alternation `…a b a b…` the tent yields `(a + b) / 2` at *both*
/// phases and so collapses the dither exactly, wherever it starts, while a fixed
/// pairing only collapses the half of it that happens to be aligned. It is also
/// symmetric (no half-pixel shift) and leaves flat colour untouched. Measured on
/// Zork Zero's EGA border, mean per-channel distance to the MCGA frame: 44.5 raw,
/// 29.2 with the box, **27.8** with the tent.
///
/// Alpha is never touched and never blended. A transparent pixel is left exactly
/// as it is, and an opaque one at a transparency edge folds the missing tap's
/// weight back onto itself — a native archive's art is a stencil (fmvpoker's
/// cards are cut out on colour 1, SQ-0801), and a fringe of half-transparent
/// pixels around every cut-out is not what the card did.
///
/// # What is deliberately NOT fused
///
/// CGA. A `.CG1`'s 640-wide art is genuine one-bit line work — Zork Zero's CGA
/// pillar is a lit column of mirrored tiles (SQ-0808), and SQ-0806 hands its two
/// colours to the terminal — and blending one-bit line work only makes it grey.
/// [`PictSource::is_monochrome`] is the test, read off the archive's own
/// `EF_MONO` flags rather than off a filename. MCGA and the Amiga are 320-wide,
/// have no dither at this frequency, and never reach here.
fn blend_half_width_columns(img: &mut image::RgbaImage) {
    let (w, h) = img.dimensions();
    if w < 2 {
        return;
    }
    let w = w as usize;
    let stride = w * 4;
    let mut row = vec![0u8; stride];
    let buf: &mut [u8] = img;
    for y in 0..h as usize {
        let base = y * stride;
        row.copy_from_slice(&buf[base..base + stride]);
        for x in 0..w {
            let c: [u8; 4] = row[x * 4..x * 4 + 4].try_into().expect("4 bytes per pixel");
            if c[3] != 255 {
                continue; // a cut-out stays cut out, at its own colour
            }

            let tap = |i: usize| -> [u8; 4] {
                let p: [u8; 4] = row[i * 4..i * 4 + 4].try_into().expect("4 bytes per pixel");
                if p[3] == 255 { p } else { c }
            };
            let l = if x > 0 { tap(x - 1) } else { c };
            let r = if x + 1 < w { tap(x + 1) } else { c };
            for k in 0..3 {
                let sum = u32::from(l[k]) + 2 * u32::from(c[k]) + u32::from(r[k]);
                buf[base + x * 4 + k] = ((sum + 2) / 4) as u8;
            }
        }
    }
}

/// The Current Palette's raw RGB triples as a native 16-entry colour table.
/// Entries the palette does not reach keep the archive's default, so no pixel
/// index is ever left without a colour.
fn colour_table(plte: &[u8]) -> [blorb::infocom_pics::Rgb; 16] {
    let mut pal = blorb::infocom_pics::DEFAULT_PALETTE;
    for (slot, c) in pal.iter_mut().zip(plte.as_chunks::<3>().0.iter()) {
        *slot = *c;
    }
    pal
}

/// PNG 8-byte signature.
const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// The raw PLTE chunk data (palette RGB triples) of a PNG byte stream, or `None`
/// when the bytes aren't a PNG or carry no PLTE (e.g. a truecolor picture, which
/// therefore never becomes the Current Palette — Blorb §11.3 tracks PLTE).
fn png_plte(png: &[u8]) -> Option<Vec<u8>> {
    if png.len() < 8 || &png[0..8] != PNG_SIG {
        return None;
    }
    let mut q = 8;
    while q + 8 <= png.len() {
        let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
        let ty = &png[q + 4..q + 8];
        let ds = q + 8;
        if ds + len + 4 > png.len() {
            break;
        }
        if ty == b"PLTE" {
            return Some(png[ds..ds + len].to_vec());
        }
        q = ds + len + 4; // data + 4-byte CRC
    }
    None
}

/// The bit depth from a PNG's IHDR (always the first chunk), used to cap the
/// spliced palette to the `2^bitdepth`-entry PLTE maximum. `None` if `png` isn't
/// a PNG opening with an IHDR chunk.
fn png_bit_depth(png: &[u8]) -> Option<u8> {
    // [sig 8][len 4][IHDR 4][width 4][height 4][bit_depth 1]… → offset 24.
    if png.len() <= 24 || &png[0..8] != PNG_SIG || &png[12..16] != b"IHDR" {
        return None;
    }
    Some(png[24])
}

/// A copy of PNG `png` with its PLTE chunk data replaced by the Current Palette
/// `new_plte` (CRC recomputed), or `None` if `png` isn't an indexed PNG carrying
/// a PLTE. Entry-count differences (Blorb §11.3): the replacement is capped to
/// the picture's bit-depth maximum (`2^bitdepth` entries); when the Current
/// Palette is SHORTER than the placeholder it replaces, the placeholder's
/// trailing entries are kept so no pixel index is left without a colour. Only
/// PLTE is touched — every other chunk (crucially tRNS, which carries the
/// overlay's transparent index) is copied verbatim.
fn splice_plte(png: &[u8], new_plte: &[u8]) -> Option<Vec<u8>> {
    if png.len() < 8 || &png[0..8] != PNG_SIG {
        return None;
    }
    let max_bytes = (1usize << png_bit_depth(png)?).saturating_mul(3);
    let mut out = Vec::with_capacity(png.len());
    out.extend_from_slice(&png[0..8]);
    let mut q = 8;
    let mut replaced = false;
    while q + 8 <= png.len() {
        let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
        let ty = &png[q + 4..q + 8];
        let ds = q + 8;
        if ds + len + 4 > png.len() {
            return None; // truncated chunk → don't hand a corrupt stream on
        }
        if ty == b"PLTE" {
            let orig = &png[ds..ds + len];
            let mut pal = new_plte[..new_plte.len().min(max_bytes)].to_vec();
            if pal.len() < orig.len() {
                pal.extend_from_slice(&orig[pal.len()..]); // keep trailing indices in range
            }
            pal.truncate(max_bytes);
            pal.truncate(pal.len() - pal.len() % 3); // whole RGB triples only
            out.extend_from_slice(&(pal.len() as u32).to_be_bytes());
            out.extend_from_slice(b"PLTE");
            out.extend_from_slice(&pal);
            out.extend_from_slice(&png_crc(b"PLTE", &pal).to_be_bytes());
            replaced = true;
        } else {
            out.extend_from_slice(&png[q..ds + len + 4]);
        }
        q = ds + len + 4;
    }
    replaced.then_some(out)
}

/// CRC-32 (PNG/zlib polynomial `0xEDB88320`) over a chunk's type bytes followed
/// by its data — the value PNG stores after each chunk's payload.
fn png_crc(ty: &[u8], data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in ty.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Test-only: build a minimal Blorb containing one `Pict` resource whose raw
/// bytes are `data`, at resource number `resnum` — for tests that need a
/// resolvable image without a full story file.
#[cfg(all(test, any(feature = "t-render", feature = "t-session")))]
pub(crate) fn test_blorb_with_pict(resnum: u32, data: &[u8]) -> blorb::Blorb {
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
    let ridx_data_len = 4 + 12; // count + one 12-byte entry
    let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
    let pict_chunk = chunk(b"PNG ", data);
    let mut ridx = Vec::new();
    ridx.extend_from_slice(&1u32.to_be_bytes());
    ridx.extend_from_slice(b"Pict");
    ridx.extend_from_slice(&resnum.to_be_bytes());
    ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
    let ridx_chunk = chunk(b"RIdx", &ridx);
    let mut inner = Vec::new();
    inner.extend_from_slice(b"IFRS");
    inner.extend_from_slice(&ridx_chunk);
    inner.extend_from_slice(&pict_chunk);
    let mut file = Vec::new();
    file.extend_from_slice(b"FORM");
    file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
    file.extend_from_slice(&inner);
    blorb::Blorb::parse(file).expect("valid test blorb")
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use image::GenericImageView;

    /// The property that chose the tent over a two-tap box on fixed column pairs
    /// (SQ-0797): a period-2 dither collapses to its exact mean at BOTH phases, so
    /// a run of `a b a b` and a run of `b a b a` come out as the same flat colour.
    /// A fixed pairing only collapses the half of the artwork that happens to be
    /// aligned with it, and nothing in the format aligns a dither to anything.
    #[test]
    fn the_column_tent_collapses_a_dither_at_either_phase() {
        let (a, b) = ([255u8, 85, 85, 255], [170u8, 85, 0, 255]); // EGA 12 and 6
        let mut even = image::RgbaImage::from_fn(8, 1, |x, _| {
            image::Rgba(if x % 2 == 0 { a } else { b })
        });
        let mut odd = image::RgbaImage::from_fn(8, 1, |x, _| {
            image::Rgba(if x % 2 == 0 { b } else { a })
        });
        blend_half_width_columns(&mut even);
        blend_half_width_columns(&mut odd);
        // Bronze: the mean of bright red and brown, which EGA does not hold.
        let bronze = image::Rgba([213u8, 85, 43, 255]);
        for x in 1..7 {
            assert_eq!(*even.get_pixel(x, 0), bronze, "even phase, column {x}");
            assert_eq!(*odd.get_pixel(x, 0), bronze, "odd phase, column {x}");
        }
    }

    /// Flat colour survives untouched, and so does a transparent pixel and the
    /// opaque pixel beside it — the tap that would have reached across a cut-out
    /// folds back onto the centre instead of averaging a fringe into it.
    #[test]
    fn the_column_tent_leaves_flat_colour_and_stencil_edges_alone() {
        let solid = image::Rgba([170u8, 85, 0, 255]);
        let hole = image::Rgba([0u8, 0, 0, 0]);
        let mut img = image::RgbaImage::from_fn(5, 1, |x, _| if x == 2 { hole } else { solid });
        blend_half_width_columns(&mut img);
        for x in [0u32, 1, 3, 4] {
            assert_eq!(*img.get_pixel(x, 0), solid, "column {x} keeps its own colour");
        }
        assert_eq!(*img.get_pixel(2, 0), hole, "a cut-out is never painted in");
    }

    #[test]
    fn arc_shares_while_unchanged_and_copies_on_write() {
        // arc() must be a cheap Arc share when the canvas hasn't changed (the
        // per-tick deep-clone this removes), and a later draw must NOT mutate a
        // frame already handed to the renderer (copy-on-write isolation). (SQ-0343)
        let mut c = Canvas::new(4, 4);
        c.fill_rect(0x00FF_0000, 0, 0, 4, 4); // red
        let snap = c.arc(); // renderer's frame this tick
        assert!(Arc::ptr_eq(&snap, &c.img), "arc() shares the bitmap, no deep copy");
        c.fill_rect(0x0000_00FF, 0, 0, 4, 4); // game draws blue next tick
        assert_eq!(snap.get_pixel(0, 0).0, [0xFF, 0, 0, 0xFF], "handed-out frame stays red");
        assert_eq!(c.img.get_pixel(0, 0).0, [0, 0, 0xFF, 0xFF], "live canvas is now blue");
        assert!(!Arc::ptr_eq(&snap, &c.img), "make_mut copied-on-write for the new draw");
    }

    #[test]
    fn fill_rect_paints_pixels_and_bumps_version() {
        let mut c = Canvas::new(10, 10);
        let v0 = c.version;
        c.fill_rect(0x00FF_0000, 2, 3, 4, 5); // red
        assert!(c.version > v0);
        let px = c.img.get_pixel(2, 3);
        assert_eq!(px.0, [0xFF, 0x00, 0x00, 0xFF]);
        // outside the rect stays transparent/default
        assert_ne!(c.img.get_pixel(9, 9).0, [0xFF, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn fill_rect_clips_out_of_bounds() {
        let mut c = Canvas::new(4, 4);
        c.fill_rect(0x0000_FF00, -2, -2, 100, 100); // green, way oversized
        assert_eq!(c.img.get_pixel(0, 0).0, [0x00, 0xFF, 0x00, 0xFF]);
        // no panic; whole canvas filled
        assert_eq!(c.img.get_pixel(3, 3).0, [0x00, 0xFF, 0x00, 0xFF]);
    }

    #[test]
    fn erase_uses_background_color() {
        let mut c = Canvas::new(4, 4);
        c.set_background(0x0000_00FF); // blue
        c.fill_rect(0x00FF_0000, 0, 0, 4, 4);
        c.erase_rect(0, 0, 2, 2);
        assert_eq!(c.img.get_pixel(0, 0).0, [0x00, 0x00, 0xFF, 0xFF]); // erased → bg
        assert_eq!(c.img.get_pixel(3, 3).0, [0xFF, 0x00, 0x00, 0xFF]); // untouched
    }

    #[test]
    fn draw_image_composites_scaled() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut c = Canvas::new(8, 8);
        c.draw_image(&image::DynamicImage::ImageRgba8(img), 1, 1, Some((4, 4)));
        assert_eq!(c.img.get_pixel(1, 1).0, [10, 20, 30, 255]);
        assert_eq!(c.img.get_pixel(4, 4).0, [10, 20, 30, 255]); // scaled to 4x4
    }

    #[test]
    fn draw_image_clamps_absurd_scale() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut c = Canvas::new(8, 8);
        // A malicious/buggy game could request a ~4 exabyte scaled bitmap;
        // this must clamp to the canvas size instead of allocating it.
        c.draw_image(&image::DynamicImage::ImageRgba8(img), 0, 0, Some((1_000_000_000, 1_000_000_000)));
        assert_eq!(c.img.dimensions(), (8, 8));
        assert_eq!(c.img.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn pict_source_resolves_and_caches() {
        // No blorb → None.
        let mut none = PictSource::new(None);
        assert!(none.info(1).is_none());
        assert!(none.image(1).is_none());
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

    /// Build a minimal Blorb carrying one `Data` resource at `resnum` with the
    /// given chunk type (`b"TEXT"` / `b"BINA"`) and raw bytes.
    fn test_blorb_with_data(resnum: u32, chunk_ty: &[u8; 4], data: &[u8]) -> blorb::Blorb {
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
        let ridx_data_len = 4 + 12;
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let data_chunk = chunk(chunk_ty, data);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes());
        ridx.extend_from_slice(b"Data");
        ridx.extend_from_slice(&resnum.to_be_bytes());
        ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
        let ridx_chunk = chunk(b"RIdx", &ridx);
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&data_chunk);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        blorb::Blorb::parse(file).expect("valid test blorb")
    }

    #[test]
    fn data_resource_reads_text_and_binary_chunks() {
        // A TEXT chunk reports is_text=true; a BINA chunk false; a missing
        // number and a Blorb-less source both yield None.
        let src = PictSource::new(Some(test_blorb_with_data(3, b"TEXT", b"hello")));
        assert_eq!(src.data_resource(3), Some((b"hello".to_vec(), true)));
        assert_eq!(src.data_resource(4), None, "no such Data resource");

        let bin = PictSource::new(Some(test_blorb_with_data(1, b"BINA", &[1, 2, 3])));
        assert_eq!(bin.data_resource(1), Some((vec![1, 2, 3], false)));

        assert_eq!(PictSource::new(None).data_resource(1), None, "no blorb → None");
    }

    #[test]
    fn dims_and_all_pict_dims_header_sniff_without_full_decode() {
        // A tiny in-memory Blorb with one Pict (a 2x2 PNG) at resource number 5.
        let blorb = test_blorb_with_pict(5, &png_bytes());
        let mut src = PictSource::new(Some(blorb));
        assert_eq!(src.dims(5), Some((2, 2)));
        assert_eq!(src.dims(99), None, "no such resource");
        assert_eq!(src.all_pict_dims(), vec![(5u16, 2u16, 2u16)]);

        assert_eq!(PictSource::new(None).all_pict_dims(), Vec::<(u16, u16, u16)>::new());
    }

    #[test]
    fn image_hands_out_cheap_arc_clones_of_one_decode() {
        // SQ-0175 part B: `PictSource::image` must not deep-clone the decoded
        // `DynamicImage` on every draw — repeated calls for the same resnum
        // should return `Arc` clones pointing at the same allocation.
        let blorb = test_blorb_with_pict(1, &png_bytes());
        let mut src = PictSource::new(Some(blorb));
        let a = src.image(1).expect("resolves");
        let b = src.image(1).expect("resolves");
        assert!(Arc::ptr_eq(&a, &b), "both calls must share one cached decode");
        assert_eq!(a.dimensions(), (2, 2));
    }

    #[test]
    fn info_answers_from_header_sniff_without_pinning_the_decode_cache() {
        // SQ-1194: `image_info` (Glk selector 8) is answered by `info()`,
        // which used to route through `get()` — a full decode pinned in the
        // unbounded `cache` forever — even though only the dimensions were
        // ever asked for. An image-heavy game sweeping `glk_image_get_info`
        // over its whole catalog must not decode and pin every picture it
        // merely measures.
        let blorb = test_blorb_with_pict(5, &png_bytes());
        let mut src = PictSource::new(Some(blorb));
        assert_eq!(src.decode_cache_len(), 0, "nothing decoded yet");
        assert_eq!(src.info(5), Some((2, 2)));
        assert_eq!(src.decode_cache_len(), 0, "info() must not populate the decode cache");
        // The decode path still works and still caches, when a caller
        // actually wants the pixels.
        assert!(src.image(5).is_some());
        assert_eq!(src.decode_cache_len(), 1, "image() still decodes and caches normally");
    }

    // ── Adaptive palettes (Blorb spec §11.3, SQ-0485) ───────────────────────

    /// zlib "stored" (uncompressed) wrapper so we can hand-build indexed PNGs
    /// without a compressor: header, one final stored block, adler32 trailer.
    fn zlib_store(raw: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01, 0x01];
        let len = raw.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(raw);
        let (mut a, mut b) = (1u32, 0u32);
        for &x in raw {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    fn png_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(ty);
        v.extend_from_slice(data);
        v.extend_from_slice(&super::png_crc(ty, data).to_be_bytes());
        v
    }

    /// A `w`×`h` 4-bit indexed PNG. `rows[y][x]` = palette index; `palette` is
    /// RGB triples; `trns` optional per-index alpha. Filter-none scanlines.
    fn indexed_png(w: u32, h: u32, palette: &[u8], trns: Option<&[u8]>, rows: &[&[u8]]) -> Vec<u8> {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[4, 3, 0, 0, 0]); // bitdepth=4, indexed, comp/filter/interlace=0
        let mut raw = Vec::new();
        for row in rows {
            raw.push(0); // filter: none
            let mut x = 0usize;
            while x < w as usize {
                let hi = row[x] & 0xf;
                let lo = if x + 1 < w as usize { row[x + 1] & 0xf } else { 0 };
                raw.push((hi << 4) | lo);
                x += 2;
            }
        }
        let mut png = super::PNG_SIG.to_vec();
        png.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
        png.extend_from_slice(&png_chunk(b"PLTE", palette));
        if let Some(t) = trns {
            png.extend_from_slice(&png_chunk(b"tRNS", t));
        }
        png.extend_from_slice(&png_chunk(b"IDAT", &zlib_store(&raw)));
        png.extend_from_slice(&png_chunk(b"IEND", b""));
        png
    }

    /// The stored trailing CRC of the first chunk of type `ty` in a PNG.
    fn stored_crc(png: &[u8], ty: &[u8; 4]) -> Option<u32> {
        let mut q = 8;
        while q + 12 <= png.len() {
            let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
            let t = &png[q + 4..q + 8];
            let ds = q + 8;
            if ds + len + 4 > png.len() {
                break;
            }
            if t == ty {
                let c = &png[ds + len..ds + len + 4];
                return Some(u32::from_be_bytes([c[0], c[1], c[2], c[3]]));
            }
            q = ds + len + 4;
        }
        None
    }

    /// Build a Blorb with the given `(number, png_bytes)` Pict resources and an
    /// `APal` chunk listing `apal` as adaptive.
    fn blorb_apal(picts: &[(u32, &[u8])], apal: &[u32]) -> blorb::Blorb {
        fn iff(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let ridx_data_len = 4 + 12 * picts.len();
        let mut apal_bytes = Vec::new();
        for n in apal {
            apal_bytes.extend_from_slice(&n.to_be_bytes());
        }
        let apal_chunk = iff(b"APal", &apal_bytes);
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2) + apal_chunk.len();
        let mut offsets = Vec::new();
        let mut cursor = first_res_off;
        let mut body = Vec::new();
        for (_n, data) in picts {
            offsets.push(cursor as u32);
            let c = iff(b"PNG ", data);
            cursor += c.len();
            body.extend_from_slice(&c);
        }
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&(picts.len() as u32).to_be_bytes());
        for (i, (n, _d)) in picts.iter().enumerate() {
            ridx.extend_from_slice(b"Pict");
            ridx.extend_from_slice(&n.to_be_bytes());
            ridx.extend_from_slice(&offsets[i].to_be_bytes());
        }
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&iff(b"RIdx", &ridx));
        inner.extend_from_slice(&apal_chunk);
        inner.extend_from_slice(&body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        blorb::Blorb::parse(file).expect("valid apal test blorb")
    }

    fn top_left(img: &DynamicImage) -> [u8; 4] {
        img.to_rgba8().get_pixel(0, 0).0
    }

    #[test]
    fn splice_plte_substitutes_palette_fixes_crc_and_decodes() {
        // 2×1, both pixels index 1. Placeholder idx1 = magenta.
        let png = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]);
        assert_eq!(top_left(&crate::cover::decode(&png).unwrap()), [170, 0, 170, 255], "placeholder is magenta");
        // Current Palette: idx1 = green.
        let current = [0u8, 0, 0, 0, 170, 0];
        let spliced = super::splice_plte(&png, &current).expect("indexed PNG splices");
        assert_eq!(super::png_plte(&spliced).as_deref(), Some(&current[..]), "PLTE now the current palette");
        // CRC was recomputed over the new PLTE (not left stale).
        assert_eq!(stored_crc(&spliced, b"PLTE"), Some(super::png_crc(b"PLTE", &current)));
        // Decodes cleanly (CRC valid) to the substituted colour.
        assert_eq!(top_left(&crate::cover::decode(&spliced).unwrap()), [0, 170, 0, 255], "now green");
    }

    #[test]
    fn splice_plte_keeps_trailing_entries_when_current_is_shorter() {
        // Placeholder has 4 entries; the pixel uses index 3.
        let placeholder = [0, 0, 0, 10, 10, 10, 20, 20, 20, 200, 100, 50];
        let png = indexed_png(2, 1, &placeholder, None, &[&[3, 3]]);
        // Current Palette shorter (2 entries): index 3 would otherwise dangle.
        let spliced = super::splice_plte(&png, &[1, 2, 3, 4, 5, 6]).unwrap();
        let pal = super::png_plte(&spliced).unwrap();
        assert_eq!(pal.len(), placeholder.len(), "length kept so index 3 stays in range");
        assert_eq!(&pal[0..6], &[1, 2, 3, 4, 5, 6], "leading entries from the current palette");
        assert_eq!(&pal[9..12], &[200, 100, 50], "trailing placeholder entry retained");
        assert_eq!(top_left(&crate::cover::decode(&spliced).unwrap()), [200, 100, 50, 255], "index 3 still resolves");
    }

    #[test]
    fn splice_plte_caps_current_palette_to_bit_depth_max() {
        let png = indexed_png(2, 1, &[0, 0, 0, 9, 9, 9], None, &[&[1, 1]]);
        // A 20-entry current palette exceeds the 16-entry (2^4) PLTE cap.
        let current: Vec<u8> = (0..20u8).flat_map(|i| [i, i, i]).collect();
        let spliced = super::splice_plte(&png, &current).unwrap();
        assert_eq!(super::png_plte(&spliced).unwrap().len(), 16 * 3, "capped to 16 entries");
        assert_eq!(top_left(&crate::cover::decode(&spliced).unwrap()), [1, 1, 1, 255], "idx1 → current[1]");
    }

    #[test]
    fn splice_and_plte_reject_non_indexed_png() {
        // A truecolor PNG has no PLTE; §11.3 derives the palette from PLTE, so
        // there is nothing to substitute and the adaptive path decodes it as-is.
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([9, 8, 7]));
        let mut rgb = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut rgb), image::ImageFormat::Png)
            .unwrap();
        assert!(super::png_plte(&rgb).is_none(), "truecolor PNG has no PLTE");
        assert!(super::splice_plte(&rgb, &[1, 2, 3]).is_none(), "nothing to splice");
    }

    #[test]
    fn adaptive_picture_uses_current_palette_and_reacts_to_palette_change() {
        let base_green = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let base_red = indexed_png(2, 1, &[0, 0, 0, 200, 0, 0], None, &[&[1, 1]]);
        let adaptive = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]); // placeholder magenta
        let blorb = blorb_apal(&[(1, &base_green), (2, &adaptive), (3, &base_red)], &[2]);
        let mut src = PictSource::new(Some(blorb));
        assert!(src.adaptive.contains(&2) && !src.adaptive.contains(&1), "APal set parsed");

        // (a) Adaptive drawn before any base is undefined per §11.3 → placeholder.
        assert_eq!(top_left(&src.image(2).unwrap()), [170, 0, 170, 255], "no base yet → own placeholder");

        // (b) Draw the green base, then the adaptive: it takes the green palette.
        src.image(1).unwrap();
        assert_eq!(top_left(&src.image(2).unwrap()), [0, 170, 0, 255], "plotted with current (green) palette");

        // (c) A different base re-establishes the palette; the SAME adaptive
        //     picture re-decodes (cache keyed by palette generation).
        src.image(3).unwrap();
        assert_eq!(top_left(&src.image(2).unwrap()), [200, 0, 0, 255], "palette change re-decodes adaptive");
    }

    #[test]
    fn palette_change_evicts_the_stale_generations_adaptive_cache() {
        // SQ-1193: adaptive_cache is keyed (resnum, palette_gen) and
        // palette_gen only ever climbs, so nothing evicted an old
        // generation's decodes. On a screen_palette machine (SQ-0887) every
        // drawn picture routes through here, so a long session accumulated a
        // full RGBA per (pic, scene palette) it would never be asked to
        // decode through again. A palette bump must retain only the entries
        // decoded under the CURRENT generation.
        let base_green = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let base_red = indexed_png(2, 1, &[0, 0, 0, 200, 0, 0], None, &[&[1, 1]]);
        let adaptive = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]);
        let blorb = blorb_apal(&[(1, &base_green), (2, &adaptive), (3, &base_red)], &[2]);
        let mut src = PictSource::new(Some(blorb));

        src.image(1).unwrap(); // establishes the green palette, generation 1
        src.image(2).unwrap(); // decodes and caches the adaptive under it
        let gen1 = src.palette_gen();
        assert_eq!(src.adaptive_cache_keys(), vec![(2, gen1)], "one entry, the current generation");

        src.image(3).unwrap(); // re-establishes the palette (red) → gen bumps
        let gen2 = src.palette_gen();
        assert!(gen2 > gen1, "a different base bumps the generation");
        assert!(
            src.adaptive_cache_keys().is_empty(),
            "the old generation's entry is evicted on the bump, before anything redraws it"
        );

        src.image(2).unwrap(); // re-decodes under the new generation
        assert_eq!(src.adaptive_cache_keys(), vec![(2, gen2)], "only the current generation survives");
    }

    #[test]
    fn scaled_image_resamples_once_and_shares_the_arc_on_repeat_draws() {
        // SQ-1196: `session::v6_scaled_art` used to re-run a full Nearest resize
        // on EVERY draw and EVERY replay op, even for a picture whose scaled
        // pixels never change. Falsify: before the cache existed, two calls at a
        // non-unit scale each allocated their own `DynamicImage` and this
        // `Arc::ptr_eq` would fail.
        let base = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let blorb = blorb_apal(&[(1, &base)], &[]);
        let mut src = PictSource::new(Some(blorb));

        let first = src.scaled_image(1, (3, 3)).unwrap();
        assert_eq!((first.width(), first.height()), (6, 3), "resampled by the requested scale");
        let second = src.scaled_image(1, (3, 3)).unwrap();
        assert!(Arc::ptr_eq(&first, &second), "second draw hits the cache: the resample ran once");
    }

    #[test]
    fn scaled_image_at_unit_scale_is_the_source_arc_with_no_copy() {
        // The other half of SQ-1196: `v6_scaled_art` `.clone()`d a full
        // `DynamicImage` at (1, 1) even though the pixels are untouched.
        // `scaled_image` must hand back the SOURCE's own `Arc` instead.
        let base = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let blorb = blorb_apal(&[(1, &base)], &[]);
        let mut src = PictSource::new(Some(blorb));

        let source = src.image(1).unwrap();
        let scaled = src.scaled_image(1, (1, 1)).unwrap();
        assert!(Arc::ptr_eq(&source, &scaled), "(1,1) is the identity: no resample, no copy");
    }

    #[test]
    fn palette_change_invalidates_the_scaled_adaptive_cache() {
        // The scaled cache for a palette-dependent picture must not outlive the
        // generation it was resampled under — mirrors
        // `palette_change_evicts_the_stale_generations_adaptive_cache` above,
        // which is the existing invalidation point this hangs off of
        // (`evict_stale_adaptive_cache`, called from the same palette-gen bump).
        let base_green = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let base_red = indexed_png(2, 1, &[0, 0, 0, 200, 0, 0], None, &[&[1, 1]]);
        let adaptive = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]);
        let blorb = blorb_apal(&[(1, &base_green), (2, &adaptive), (3, &base_red)], &[2]);
        let mut src = PictSource::new(Some(blorb));

        src.image(1).unwrap(); // establishes the green palette, generation 1
        let scaled_gen1_a = src.scaled_image(2, (3, 3)).unwrap();
        let scaled_gen1_b = src.scaled_image(2, (3, 3)).unwrap();
        assert!(
            Arc::ptr_eq(&scaled_gen1_a, &scaled_gen1_b),
            "same generation, repeat draw: the resample ran once"
        );

        src.image(3).unwrap(); // re-establishes the palette (red) → gen bumps,
                                // evicting the stale generation's scaled entry
        let scaled_gen2 = src.scaled_image(2, (3, 3)).unwrap();
        assert!(
            !Arc::ptr_eq(&scaled_gen1_a, &scaled_gen2),
            "the palette changed the source pixels: the next draw resamples again"
        );
        assert_eq!(top_left(&scaled_gen2), [200, 0, 0, 255], "recoloured under the new (red) palette");
    }

    #[test]
    fn a_redrawn_base_picture_re_establishes_its_palette_through_the_scaled_cache() {
        // SQ-1288: `scaled_image` is the LIVE DRAW path, and drawing a
        // non-adaptive picture is what establishes the Current Palette (§11.3).
        // SQ-1196's scaled cache returned early on a hit and so never called
        // `image()` for a base picture the session had already drawn — but a
        // game revisits its scenes. Arthur walked out of the brown church back
        // into the blue churchyard and kept the church's frame, because the
        // SECOND draw of the churchyard picture established nothing.
        //
        // Falsify: return early on a `scaled_cache` hit again and the last
        // assertion reads red — the palette Pict 3 left behind.
        let base_green = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let base_red = indexed_png(2, 1, &[0, 0, 0, 200, 0, 0], None, &[&[1, 1]]);
        let adaptive = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]);
        let blorb = blorb_apal(&[(1, &base_green), (2, &adaptive), (3, &base_red)], &[2]);
        let mut src = PictSource::new(Some(blorb));

        src.scaled_image(1, (2, 2)).unwrap(); // the green scene
        assert_eq!(top_left(&src.scaled_image(2, (2, 2)).unwrap()), [0, 170, 0, 255], "green scene");

        src.scaled_image(3, (2, 2)).unwrap(); // the red scene
        assert_eq!(top_left(&src.scaled_image(2, (2, 2)).unwrap()), [200, 0, 0, 255], "red scene");

        // Back to the green scene — a REDRAW of Pict 1, already in the scaled
        // cache. It must still establish green.
        src.scaled_image(1, (2, 2)).unwrap();
        assert_eq!(
            top_left(&src.scaled_image(2, (2, 2)).unwrap()),
            [0, 170, 0, 255],
            "revisiting the green scene re-establishes its palette: the adaptive picture follows back"
        );
    }

    /// A `PictSource` over one of `stories/`'s native Infocom archives, or
    /// `None` (with a printed SKIP) when the gitignored fixture is absent —
    /// the CI-safe pattern every real-game case here uses.
    fn native_fixture(archive: &str) -> Option<(PictSource, blorb::infocom_pics::InfocomPics)> {
        let path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(archive);
        let Ok(raw) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored native archive missing at {}", path.display());
            return None;
        };
        let parse = |bytes: Vec<u8>| {
            blorb::infocom_pics::InfocomPics::parse(bytes).expect("a release archive parses")
        };
        // Two independent parses: one for the source under test, one for the
        // oracle below, so the oracle shares no decode state with it.
        Some((PictSource::from_native(parse(raw.clone())), parse(raw)))
    }

    /// SQ-1197, the correctness bar: a palette change must produce the SAME
    /// pixels a from-scratch decode under that palette produces.
    ///
    /// `base_a`/`base_b` are two pixel-bearing pictures carrying palettes of
    /// their own — drawing one establishes the Current Palette (§11.3) — and
    /// `target` is a picture with no palette of its own, which the archive's
    /// directory marks adaptive by a zero palette offset. Ids are read off the
    /// named release, not guessed.
    fn palette_swap_remaps_rather_than_re_decodes(
        archive: &str,
        base_a: u32,
        base_b: u32,
        target: u32,
    ) {
        let Some((mut src, pics)) = native_fixture(archive) else { return };
        assert!(src.is_adaptive(target), "{archive}: picture {target} has no palette of its own");
        assert!(
            !src.fuses_dither(),
            "{archive}: this case's oracle colourises without the dither fuse"
        );

        // Generation A: the base establishes the palette, the adaptive target
        // decodes through it — and retains its index plane.
        src.image(base_a).expect("base picture A decodes");
        let gen_a = src.palette_gen();
        let under_a = src.image_under_current_palette(target).expect("adaptive picture decodes");
        let plane_a = src.cached_index_plane(target).expect("the decode retained an index plane");

        // Generation B.
        src.image(base_b).expect("base picture B decodes");
        assert_ne!(gen_a, src.palette_gen(), "{archive}: the two bases carry different palettes");
        let under_b = src.image_under_current_palette(target).expect("adaptive picture re-maps");

        // ZERO decodes: the plane the re-map read is the very object the first
        // draw decoded. Falsify by removing `index_planes` and letting
        // `adaptive_image` call `InfocomPics::decode` again — the second plane is
        // then a fresh allocation and this fails.
        let plane_b = src.cached_index_plane(target).expect("the plane is still retained");
        assert!(
            Arc::ptr_eq(&plane_a, &plane_b),
            "{archive}: a palette swap must re-map the retained index plane, not re-decode"
        );

        // The oracle: decompress the picture afresh out of the second parse and
        // expand it through the Current Palette by hand — the arithmetic the old
        // `native_image` did on every palette change, written out here so it
        // shares no cache, no `Arc` and no code path with the source above.
        let plte = src.current_palette().expect("a base draw established the Current Palette");
        let fresh = pics
            .decode(u16::try_from(target).expect("a Pict number fits u16"))
            .expect("the archive decodes the target picture")
            .rgba_with(&colour_table(plte));
        assert_eq!(
            under_b.as_bytes(),
            fresh.as_slice(),
            "{archive}: the re-mapped pixels must be byte-identical to a fresh decode"
        );

        // Non-vacuity: the swap has to have actually recoloured something, or
        // "identical to a fresh decode" is a statement about two identical
        // images and proves nothing.
        assert_ne!(
            under_a.as_bytes(),
            under_b.as_bytes(),
            "{archive}: pictures {base_a} and {base_b} must recolour {target} differently"
        );
    }

    #[test]
    fn dos_mcga_palette_swap_remaps_the_index_plane() {
        // Zork Zero r393/s890714, DOS `.MG1` (MCGA). Its directory marks 172
        // pixel-bearing pictures adaptive — id for id, the numbers `Zork0.blb`
        // lists in `APal` — the lowest of which is 9; pictures 2 and 4 carry
        // palettes of their own and those two palettes differ.
        palette_swap_remaps_rather_than_re_decodes("zork0.mg1", 2, 4, 9);
    }

    #[test]
    fn amiga_palette_swap_remaps_the_index_plane() {
        // The same release's Amiga `.pic`, whose pictures run the Huffman +
        // run-length + per-line XOR path instead of the PC's LZW — the decode
        // this quest is about, and a different one from the case above.
        palette_swap_remaps_rather_than_re_decodes("zork0.pic", 2, 4, 9);
    }

    #[test]
    fn set_fuse_dither_drops_the_retained_index_planes() {
        // The index planes are cleared wherever `cache` is (SQ-1197). The fuse
        // is the only such point today; Zork Zero's `.EG1` is the eligible
        // shape for it — a 640-wide SIXTEEN-colour rendition — so
        // `from_native` turns the fuse ON and this can turn it back off.
        let Some((mut src, _)) = native_fixture("zork0.eg1") else { return };
        assert!(src.fuses_dither(), "a 640-wide EGA rendition is dither-eligible");

        src.image(1).expect("EGA picture 1 decodes");
        let before = src.cached_index_plane(1).expect("the draw retained an index plane");

        src.set_fuse_dither(false);
        assert!(
            src.cached_index_plane(1).is_none(),
            "the fuse change dropped every decode this source held, planes included"
        );

        src.image(1).expect("EGA picture 1 decodes again");
        let after = src.cached_index_plane(1).expect("the redraw decoded a fresh plane");
        assert!(!Arc::ptr_eq(&before, &after), "the next draw decodes rather than serving a stale plane");
    }

    #[test]
    fn size_queries_do_not_establish_the_palette() {
        // `info`/`dims` must not count as "drawing": querying a base picture's
        // size must NOT set the Current Palette that later adaptive draws use.
        let base_green = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let adaptive = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]);
        let blorb = blorb_apal(&[(1, &base_green), (2, &adaptive)], &[2]);
        let mut src = PictSource::new(Some(blorb));
        src.info(1); // size query on the base — not a draw
        src.dims(1);
        assert!(src.current_plte.is_none(), "a size query must not establish the palette");
        assert_eq!(top_left(&src.image(2).unwrap()), [170, 0, 170, 255], "adaptive still on its placeholder");
    }

    /// **`graphics::resource_blorb` is the only way in, and this is what keeps
    /// it so** (SQ-1085).
    ///
    /// A bare `blorb::resolve_resource_blorb` knows the filesystem and nothing
    /// else. Every tier `app` has added since — the build-mismatch refusal
    /// (SQ-0866) and now the ZIP a player downloaded the game in — lives in the
    /// wrapper, so a call that skips it silently resolves from an older set of
    /// rules and produces a plausible answer rather than an error. That is
    /// exactly the shape CLAUDE.md's refactoring policy names, and `reset.rs`
    /// has already been on the wrong side of it once (SQ-1022).
    ///
    /// It cannot be made unreachable — `blorb` is a dependency and its function
    /// is public — so it is made VISIBLE instead. Two exemptions, both narrow:
    /// `graphics.rs` itself, which is the wrapper; and anything below a file's
    /// own `#[cfg(test)] mod tests`, where a harness building its own
    /// `PictSource` from a loose story file is stating its inputs rather than
    /// resolving a launch.
    ///
    /// # A test MODULE, not any test item
    ///
    /// `#[cfg(test)]` sits on individual items too, and saying nothing about
    /// the item AFTER it is the whole point of an item attribute. Latching on
    /// the bare attribute read 59 lines of `render/screen.rs` and skipped the
    /// other 12,251 — the largest and most-edited file in the crate, and the
    /// one where somebody reaching for a Blorb is most likely to reach for the
    /// bare call. Three more files carry an early item attribute:
    /// `render/transcript.rs` (line 13, a `use`), `render/v6_layout.rs` (187, a
    /// `const`) and `config_template.rs` (38, a `use`).
    ///
    /// So the latch requires the attribute to be followed by a `mod`
    /// declaration, which is this tree's one convention for a test module and
    /// always the last thing in the file. Anything else — a test-only `use`,
    /// `const` or helper `fn` — leaves the scan running, and a bare call inside
    /// such a helper would be reported. That is the right way round: a false
    /// report is answered by moving the helper into `mod tests`, where a missed
    /// one is answered by nobody, because it looks exactly like a pass.
    ///
    /// # And the scan proves it reached the code
    ///
    /// A source scanner that silently stops scanning is indistinguishable from
    /// a passing one, which is how the latch bug survived review. So the reach
    /// is asserted too, against the file that exposed it.
    #[test]
    fn resource_blorb_is_resolved_through_this_module_only() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
        }
        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src_root, &mut files);
        files.sort();
        assert!(files.len() > 10, "the walk found the source tree: {}", files.len());

        let mut bad = Vec::new();
        let mut examined_total = 0usize;
        let mut examined_screen = 0usize;
        for file in files {
            if file.file_name().and_then(|n| n.to_str()) == Some("graphics.rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let lines: Vec<&str> = text.lines().collect();
            let mut examined = 0usize;
            for (n, line) in lines.iter().enumerate() {
                // `#[cfg(test)]` on a `mod` ends the production half of the
                // file; on any other item it says nothing about what follows.
                // SQ-1242 put `app`'s in-crate `mod tests` blocks behind t-star
                // Cargo features, gated with an `all(test, …)` predicate — both
                // the old and new spelling are checked, or this scan would stop
                // reading at line 1 of every file it rewrote and pass vacuously
                // on the rest.
                let trimmed = line.trim_start();
                if trimmed.starts_with("#[cfg(test)]") || trimmed.starts_with("#[cfg(all(test,") {
                    let next = lines[n + 1..].iter().find(|l| !l.trim().is_empty());
                    if next.is_some_and(|l| l.trim_start().starts_with("mod ")
                        || l.trim_start().starts_with("pub mod ")
                        || l.trim_start().starts_with("pub(crate) mod "))
                    {
                        break;
                    }
                }
                examined += 1;
                // Comments and doc links name it on purpose; this is about code.
                let code = match line.find("//") {
                    Some(i) => &line[..i],
                    None => line,
                };
                if code.contains("blorb::resolve_resource_blorb(") {
                    bad.push(format!("  {}:{}  {}", file.display(), n + 1, line.trim()));
                }
            }
            examined_total += examined;
            if file.ends_with("render/screen.rs") {
                examined_screen = examined;
            }
        }

        // The reach, stated against the file that caught the latch bug.
        // `render/screen.rs` keeps its `#[cfg(test)] mod tests` at the end and
        // has carried over 8,000 production lines for a long time; the broken
        // latch read 59 of them. Floors rather than exact counts, because both
        // files grow — but a latch that stops early cannot clear them.
        assert!(
            examined_screen > 5_000,
            "the scan must read the BODY of render/screen.rs, not stop at its first \
             `#[cfg(test)]` item — examined {examined_screen} lines",
        );
        assert!(
            examined_total > 80_000,
            "the scan must read the crate, not a prologue of it — examined \
             {examined_total} lines across app/src",
        );

        assert!(
            bad.is_empty(),
            "resolve the resource Blorb through `crate::graphics::resource_blorb`, which \
             carries the zip tier and the build-mismatch refusal:\n{}",
            bad.join("\n"),
        );
    }
}
