//! What lanthorn actually WRITES to the terminal, not what it computed (SQ-0762).
//!
//! Every other harness here renders into a `Buffer` and asserts on cells. That
//! cannot see a defect that lives between the model and the wire, and this
//! session produced several that were "not in the model" and plainly visible on
//! screen. This one boots the real binary under a pty, answers the terminal
//! queries a kitty terminal answers, feeds it keys, and decodes the bytes that
//! come back.
//!
//! WHAT IT ASSERTS, AND WHAT IT DELIBERATELY DOES NOT. It asserts that the
//! harness is measuring the right thing: that kitty negotiated (a run that fell
//! back to half-blocks exercises the wrong backend and every number it produces
//! is worthless), that a real upload happened, and that the placement rect can be
//! read back off the stream. It does NOT pin any particular defect's presence:
//! a test that fails when a bug is FIXED is a trap for the next person. The
//! defect reading — image extent versus painted fill — is printed as a finding
//! and written to the report file instead.
//!
//! Unix only; a pty is. Windows gets an explicit skip, below.

// Not gated: the decoder half is portable and its unit tests are worth having on
// every platform. Only the pty half inside it is `#[cfg(unix)]`.
//
// The harness module is declared ONCE by this suite's group binary (`tests/pty.rs`)
// and shared by every pty suite in it; declaring it here as well would compile a
// second copy of ~2900 lines into the same binary.
use super::pty_stream;

#[cfg(not(unix))]
#[test]
fn the_pty_capture_is_unix_only() {
    eprintln!("SKIP: capturing the stream needs a pty, which this platform does not have; \
               the decoder's own tests still ran");
}

#[cfg(unix)]
mod unix {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::pty_stream::{self, driver};

    /// Journey release 30, the Amiga disk image. NOT `journey.z6` — that is
    /// release 83 and a genuinely different build, and a finding measured on one
    /// does not transfer to the other (SQ-0760).
    const STORY: &str = "Journey - The Quest Begins.adf";

    /// 117x64 terminal, map pane hidden: the frame border takes a column each
    /// side and the help row takes the bottom, which leaves the story pane the
    /// 115x61 the defect was measured at.
    const COLS: u16 = 117;
    const ROWS: u16 = 64;

    fn out_dir() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pty-capture");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn the_emitted_stream_shows_which_rows_an_image_covers_and_which_are_painted() {
        let story = driver::stories_dir().join(STORY);
        if !story.is_file() {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
            return;
        }
        let user_dir = out_dir().join("user-dir");
        let _ = std::fs::remove_dir_all(&user_dir);

        let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_lanthorn"), &story, &user_dir);
        spec.cols = COLS;
        spec.rows = ROWS;
        // Journey's intro wants a few keypresses before the party menu — the frame
        // with the picture column, the prose beside it and the menu along the
        // bottom, which is the frame the defect lives on.
        spec.keys = vec![
            driver::Key::Wait(Duration::from_millis(1200)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(600)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(600)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(600)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(900)),
        ];

        let cap = driver::run(spec).expect("the pty harness should boot lanthorn");
        let term = pty_stream::decode_capture(&cap);
        let report = pty_stream::report(&cap, &term);
        let path = out_dir().join("journey-r30-115x61.txt");
        let _ = std::fs::write(&path, &report);

        let neg = cap.negotiated();
        assert!(
            neg.is_kitty(),
            "the capture must exercise the kitty path or it measures nothing: {}\n\
             (full report at {})",
            neg.explain(),
            path.display()
        );
        assert!(
            !term.apc.is_empty(),
            "a kitty run uploads at least one image; none was decoded (report at {})",
            path.display()
        );
        let placements = term.placements();
        assert!(
            !placements.is_empty(),
            "the stream carried APC uploads but no U+10EEEE placeholder cells, so nothing was \
             actually placed on screen (report at {})",
            path.display()
        );
        assert!(
            term.printed_cells > 1000,
            "only {} cells were ever printed — the app did not get as far as drawing a frame \
             (report at {})",
            term.printed_cells,
            path.display()
        );

        // The finding, not an assertion: which rows are image and which are paint.
        eprintln!("emitted-stream report: {}", path.display());
        eprint!("{}", pty_stream::overrun_finding(&term));
    }

    /// One `key=value` out of an APC control block. `ApcCmd`'s own accessor is
    /// private to the decoder, and `params` is the raw block it parsed.
    fn param<'a>(params: &'a str, key: &str) -> Option<&'a str> {
        params.split(',').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
    }

    /// SQ-0753: every image lanthorn stops showing must be FREED in the terminal.
    ///
    /// Deletes were **zero in every capture ever taken** of this app — the quest's
    /// headline fact, and the reason it mattered: kitty evicts by LRU and evicts
    /// images that are CURRENTLY PLACED, so an unbounded pile of orphans can blank a
    /// live one. Only the graphics WINDOWS, whose ids lanthorn allocates itself, were
    /// ever deleted (SQ-0637); everything drawn through a `ratatui-image` `Protocol`
    /// — every chrome band, the whole-pane raster composite — was uploaded and
    /// abandoned. Journey release 30 over five keystrokes: 4.1 MB up, 0 bytes freed.
    ///
    /// The invariant asserted is the leak-free one, not a defect's presence: at the
    /// end of the run every image the app uploaded is either still on the screen or
    /// has been deleted, and nothing on the screen has been deleted out from under
    /// its own placeholders. Only the emitted stream can say this — the cell buffer
    /// cannot see an upload at all.
    #[test]
    fn every_abandoned_upload_is_deleted_and_no_live_one_is() {
        let story = driver::stories_dir().join(STORY);
        if !story.is_file() {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
            return;
        }
        let user_dir = out_dir().join("user-dir-deletes");
        let _ = std::fs::remove_dir_all(&user_dir);

        let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_lanthorn"), &story, &user_dir);
        spec.cols = COLS;
        spec.rows = ROWS;
        // Journey boots through the raster path and then hands the screen to the
        // hybrid ring, so this walk abandons a whole-pane composite AND re-encodes a
        // band — the two ways a `ratatui-image` upload is orphaned.
        spec.keys = vec![
            driver::Key::Wait(Duration::from_millis(1500)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(800)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(800)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(800)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(1200)),
        ];

        let cap = driver::run(spec).expect("the pty harness should boot lanthorn");
        let term = pty_stream::decode_capture(&cap);
        let report = pty_stream::report(&cap, &term);
        let path = out_dir().join("journey-r30-deletes.txt");
        let _ = std::fs::write(&path, &report);

        let neg = cap.negotiated();
        assert!(
            neg.is_kitty(),
            "half-blocks uploads nothing, so this measures nothing: {}\n(report at {})",
            neg.explain(),
            path.display()
        );

        // Transmits and deletes carry the FULL 32-bit id; a placeholder cell can only
        // carry the low 24 bits (the high byte is a diacritic the decoder does not
        // fold back in), so the comparison happens down there.
        let low24 = |id: u32| id & 0x00FF_FFFF;
        let ids = |action: &str| -> std::collections::BTreeSet<u32> {
            term.apc
                .iter()
                .filter(|a| param(&a.params, "a") == Some(action))
                .filter_map(|a| param(&a.params, "i")?.parse::<u32>().ok())
                // lanthorn's own graphics-window ids are deliberately KEPT uploaded
                // and re-placed (`KITTY_CACHE`, SQ-0564), so an unplaced one is a
                // cache entry, not a leak. Only the `ratatui-image` uploads — random
                // ids, no cache of their own — are in scope here.
                .filter(|id| id & 0xFFF0_0000 != 0x00B0_0000)
                .collect()
        };
        let uploaded: std::collections::BTreeSet<u32> = ids("T").into_iter().map(low24).collect();
        let deleted: std::collections::BTreeSet<u32> = ids("d").into_iter().map(low24).collect();
        let on_screen: std::collections::BTreeSet<u32> =
            term.placements().iter().map(|p| low24(p.image_id)).collect();

        assert!(!uploaded.is_empty(), "no image was uploaded at all (report at {})", path.display());
        assert!(
            !deleted.is_empty(),
            "the run uploaded {} image(s) and deleted none — this is exactly the state SQ-0753 \
             measured, where every image lanthorn ever sent stays in the terminal for good \
             (report at {})",
            uploaded.len(),
            path.display()
        );

        let leaked: Vec<u32> = uploaded.difference(&on_screen).filter(|i| !deleted.contains(i)).copied().collect();
        assert!(
            leaked.is_empty(),
            "image(s) {leaked:?} were uploaded, are not on the final screen, and were never \
             deleted — the terminal is holding pixels nothing can ever show again \
             (report at {})",
            path.display()
        );

        let freed_but_shown: Vec<u32> = on_screen.intersection(&deleted).copied().collect();
        assert!(
            freed_but_shown.is_empty(),
            "image(s) {freed_but_shown:?} are still placed on the final screen and were DELETED \
             — `d=I` frees the data and its placements, so those cells now draw nothing \
             (report at {})",
            path.display()
        );

        eprintln!(
            "emitted-stream deletes: {} uploaded, {} still on screen, {} freed — report at {}",
            uploaded.len(),
            on_screen.len(),
            deleted.len(),
            path.display()
        );
    }

    /// SQ-1201: the wire-hygiene counter (`app::render::graphics::measure_traffic`)
    /// this quest built to catch the whole class SQ-1190 fixed by hand — a cache
    /// that replaces or evicts a `Protocol` without ever emitting the `a=d` for its
    /// upload. Same story, terminal size and key walk as
    /// `every_abandoned_upload_is_deleted_and_no_live_one_is` above (which already
    /// drives Journey through a raster-composite abandon AND a band re-encode, the
    /// two ways this app orphans a `ratatui-image` upload) — reused rather than
    /// invented, so this measures the same real traffic that test's own BTreeSet
    /// arithmetic already vouches for, just through the counter a future eviction
    /// site would actually be caught by.
    ///
    /// THE BOUND: at capture end, `stranded_uploads` (ids this capture transmitted
    /// that no later `a=d` in it named) can be no more than the count of DISTINCT
    /// image ids still placed on the final screen. Every one of those is legitimately
    /// stranded — it is still visible, so freeing it would blank the screen, exactly
    /// `queue_protocol_delete_after_place`'s reason to exist. Anything stranded
    /// BEYOND that bound is transmitted, no longer on screen, and never freed —
    /// which is precisely the leak SQ-1190 fixed four instances of and the reason
    /// this counter exists at all.
    ///
    /// Falsified by reverting `GraphicsRender::invalidate_v6`'s
    /// `self.queue_protocol_delete(old.placed_id)` (crates/app/src/render/graphics.rs)
    /// — the wholesale-abandon delete for the raster composite Journey drops two
    /// frames into its boot, when the hybrid ring takes over. NOT literally one of
    /// SQ-1190's four `cover.rs`/`picker_ui.rs` call sites: those guard the
    /// pre-game PICKER (a byte-budgeted cover cache and a 128-tile gallery LRU,
    /// both needing far more images/navigation than a short pty walk can drive
    /// reliably), while this walk never leaves a single game. It is the SAME
    /// underlying seam, though — `queue_external_deletes` (SQ-1190's fix) funnels
    /// into this struct's `queue_protocol_delete`, the exact function reverted
    /// here — so a regression in that shared machinery is exactly what this
    /// falsification exercises. With the call commented out: `deletes` dropped
    /// from 1 to 0, `freed_pixels` from 2,119,680 to 0, and `stranded_uploads`
    /// grew from 1 (449,280 pixel bytes — the one band legitimately still on
    /// screen) to 2 (2,568,960 pixel bytes = that same band PLUS the abandoned
    /// composite, never freed) — tripping the bound of 1 distinct id on screen.
    /// See the quest report for the full numbers.
    #[test]
    fn stranded_uploads_never_exceeds_what_is_still_on_screen() {
        let story = driver::stories_dir().join(STORY);
        if !story.is_file() {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
            return;
        }
        let user_dir = out_dir().join("user-dir-wire-hygiene");
        let _ = std::fs::remove_dir_all(&user_dir);

        let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_lanthorn"), &story, &user_dir);
        spec.cols = COLS;
        spec.rows = ROWS;
        spec.keys = vec![
            driver::Key::Wait(Duration::from_millis(1500)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(800)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(800)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(800)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(1200)),
        ];

        let cap = driver::run(spec).expect("the pty harness should boot lanthorn");
        let term = pty_stream::decode_capture(&cap);
        let report = pty_stream::report(&cap, &term);
        let path = out_dir().join("journey-r30-wire-hygiene.txt");
        let _ = std::fs::write(&path, &report);

        let neg = cap.negotiated();
        assert!(
            neg.is_kitty(),
            "half-blocks uploads nothing, so this measures nothing: {}\n(report at {})",
            neg.explain(),
            path.display()
        );

        let text = String::from_utf8_lossy(&cap.bytes);
        let m = app::render::graphics::measure_traffic(&text);
        assert!(m.uploads > 0, "no image was uploaded at all (report at {})", path.display());

        // The placeholder foreground only ever carries the low 24 bits of an id
        // (the decoder does not fold the diacritic high byte back in), which is
        // the same truncation `measure_traffic`'s `i=` ids are compared against —
        // see `every_abandoned_upload_is_deleted_and_no_live_one_is` above.
        let on_screen: std::collections::BTreeSet<u32> =
            term.placements().iter().map(|p| p.image_id).collect();

        eprintln!(
            "wire hygiene: {} upload(s) ({} pixel bytes) · {} delete(s) · {} pixel bytes freed · \
             {} stranded ({} pixel bytes) · {} distinct id(s) on the final screen — report at {}",
            m.uploads,
            m.pixels,
            m.deletes,
            m.freed_pixels,
            m.stranded_uploads,
            m.stranded_pixels,
            on_screen.len(),
            path.display()
        );

        assert!(
            m.stranded_uploads <= on_screen.len() as u64,
            "{} upload(s) are stranded (transmitted, never freed by a later a=d in this capture) \
             but only {} distinct image id(s) are still on the final screen — the difference is \
             transmitted, off-screen, and never freed: exactly the class SQ-1190 fixed \
             (report at {})",
            m.stranded_uploads,
            on_screen.len(),
            path.display()
        );
    }
}
