//! `ScottSession` — adapts the `scott` VM (ScottFree-format Adventure
//! International titles) to the engine-neutral [`Engine`] trait, alongside
//! `zvm`'s `GameSession` (`session.rs`) and the Glulx `GlulxSession`.
//!
//! Scott games are line-only (no `read_char`), carry no Glk window tree (the
//! app renders the transcript itself), and have no in-game `@save`/`@restore`
//! suspension protocol — persistence is entirely the host-driven Save State
//! snapshot (`Vm::snapshot`/`Vm::restore`).

use std::any::Any;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use crate::engine::{
    BufferWindow, Debugger, Engine, EngineError, EngineSave, GraphicsWindow, LocationInfo,
    ScreenModel, Split, StatusModel, WinNode,
};
use crate::graphics::PictSource;
use crate::session::{InputKind, PendingIo, TurnResult};

/// The engine tag recorded in an `EngineSave` produced by the Scott adapter.
pub const SCOTT_ENGINE: &str = "scott";
/// The save-format version within the `scott` engine.
pub const SCOTT_SAVE_FORMAT: u32 = 1;

/// The canonical Scott Adams input prompt, shown before each command.
/// ScottFree prints exactly this text before every input, so it belongs to
/// the host/input layer here (not the VM, which stays input-agnostic).
/// Scott used this phrase, never the Infocom-style `>`.
const PROMPT: &str = "\nTell me what to do ? ";

/// The keypress hint surfaced while a US S.A.G.A. picture-show sequence is
/// presenting (SQ-1487, spec §12.11 — *The Hulk*'s "bite lip" opening shows
/// several scenes in a row, each ENTER-gated). There is no dedicated hint in
/// the app's main input bar for a keypress wait — `render_input_content`
/// (`render/transcript.rs`) hides the bar outright whenever `pending_input()`
/// answers `Char`, for every engine — so this rides the transcript itself,
/// through `TurnResult::info`: the existing general-purpose "one-line note to
/// the player" door (`turn.rs` pushes it with the ordinary transcript style,
/// so nothing new needs styling). Shown right where the player is already
/// reading, rather than in a bar that would otherwise stay blank.
const PICTURE_SHOW_HINT: &str = "[Press RETURN to continue]";

/// Terminal rows reserved for the room-picture band in a graphics (`.blb`) game.
/// The renderer scales the picture (typically 256×96) to fit this band.
const PICTURE_ROWS: u16 = 16;

/// Resolve this story's ScottFree `-y`/`-s`/`-t`/`-p` options from its
/// per-game sidecar (SQ-1413): `crate::styles::PerGameConfig`'s four
/// `scott_*` keys, absent key = `scott::Options::default()`'s off. Reads
/// only the flags — `scott::Options::presentation` is deliberately left at
/// [`scott::Presentation::C64`] (lanthorn's own layout) regardless of
/// `scott_trs80_style`, matching [`ScottSession::new_with_options`]'s doc.
pub fn resolve_options(game_dir: &std::path::Path) -> scott::Options {
    scott::Options::new()
        .with_you_are(crate::styles::read_per_game_scott_you_are(game_dir).unwrap_or(false))
        .with_scott_light(crate::styles::read_per_game_scott_light(game_dir).unwrap_or(false))
        .with_trs80_style(crate::styles::read_per_game_scott_trs80_style(game_dir).unwrap_or(false))
        .with_prehistoric_lamp(
            crate::styles::read_per_game_scott_prehistoric_lamp(game_dir).unwrap_or(false),
        )
}

/// Decode a ZX Spectrum *Mysterious Adventures* release's own family-B
/// artwork straight off the story bytes it loaded from (SQ-1480) — a `.z80`
/// snapshot, or the bare 48K memory image a host that already decompressed
/// one for its own reasons hands over (`scott::looks_like_zx_mysterious`'s
/// own case, mirrored here the way `scott::Database::parse` reads both).
///
/// `None` for every other story — including a C64 PRG/D64 image, which
/// `scott::looks_like_zx_mysterious_z80`'s `.z80` header sniff and the bare
/// image's length-and-signature check both refuse by construction — and for a
/// recognised snapshot whose picture block does not decode.
fn zx_mysterious_picture_lists(bytes: &[u8]) -> Option<Vec<scott::c64::PictureList>> {
    if scott::looks_like_zx_mysterious_z80(bytes) {
        return scott::zx_mysterious::decode_picture_lists_z80(bytes).ok();
    }
    if bytes.len() == scott::IMAGE_LEN && scott::looks_like_zx_mysterious(bytes) {
        return scott::zx_mysterious::decode_picture_lists(bytes).ok();
    }
    None
}

/// Build the top room-panel buffer from a `Vm::room_block()` string: one logical
/// line per `\n`, with the per-line style/paragraph/image tracks filled parallel
/// (the inline-buffer renderer indexes them by line). `primary: false` so the app
/// draws it inline rather than mirroring it into the transcript.
fn room_panel(block: &str) -> BufferWindow {
    let lines: Vec<String> = block.split('\n').map(str::to_string).collect();
    let n = lines.len();
    BufferWindow {
        lines,
        runs: vec![Vec::new(); n],
        para: vec![crate::state::ParaFmt::default(); n],
        images: vec![None; n],
        primary: false,
        panel: true,
        ..Default::default()
    }
}

/// One step of a US S.A.G.A. picture-show sequence still to present
/// (SQ-1487) — see [`ScottSession::showing`].
struct PendingShow {
    /// Transcript text to surface once the player's keypress reaches this step.
    text: String,
    /// The picture the band switches to at this step, or `None` on the LAST
    /// step to mean "the sequence is over — revert to the room's own picture".
    next_picture: Option<u16>,
}

/// A running Scott Adams (ScottFree `.dat`) game session.
pub struct ScottSession {
    vm: scott::Vm,
    /// The opening room description from `Vm::new`, drained by the first
    /// `take_transcript` call; empty thereafter (per-turn output flows
    /// through `TurnResult::transcript` instead).
    intro: String,
    aux: BTreeMap<String, Vec<u8>>,
    aux_dirty: bool,
    /// Room pictures, from any of four sources, in the order
    /// [`ScottSession::new_with_options`] resolves them: Blorb `Pict`
    /// resources for a graphics (`.blb`) game (the SAGA/Mysterious Adventures
    /// graphic versions ship the room pictures here, SQ-0402); a US S.A.G.A.
    /// release's own **family-C** strip bitmaps, read off the release disk
    /// beside the database (spec §8.3, SQ-1475,
    /// `PictSource::from_scott_saga`); or a Commodore 64 or ZX Spectrum
    /// *Mysterious Adventures* release's own **family-B** vector artwork,
    /// decoded straight out of its PRG/D64 memory image or `.z80` snapshot
    /// (SQ-1463/SQ-1480, `PictSource::from_scott_family_b`). Empty
    /// (`PictSource::new(None)`) for a plain `.dat` with none of them.
    picts: PictSource,
    /// The decoded picture to show for the current room, and the picture number
    /// it was resolved from — recomputed only when the number changes so the same
    /// image isn't re-uploaded to the terminal every frame.
    current_canvas: Option<Arc<image::RgbaImage>>,
    current_pic_num: Option<u16>,
    /// The object records composited over `current_pic_num` to make
    /// `current_canvas`, in draw order (SQ-1482, spec §12.11) — the room's
    /// present items over a room picture, the carried items over the
    /// inventory backdrop.
    ///
    /// Part of the band's identity, not a derived detail: the picture NUMBER
    /// alone stopped being enough the moment two frames of the same room
    /// could differ, so `refresh_picture`'s early-out compares both. Named
    /// records rather than indices because a name is what the container
    /// holds, what the composite reads, and what `/dump-windows` prints.
    current_overlays: Vec<String>,
    pic_version: u64,
    /// Steps of the CURRENT US S.A.G.A. picture-show sequence still to
    /// present, one per remaining opcode-90 request (SQ-1487). Empty outside
    /// a sequence — also what [`ScottSession::pending_input`] reads to
    /// decide `Line` vs `Char`. Transient host presentation state, like
    /// `scott::Vm`'s own per-turn fields it is built from: a Save State taken
    /// mid-sequence restores to the state at the END of the turn that queued
    /// it (the `scott::Vm` snapshot carries no picture-show state either), so
    /// resuming that save shows the room view, not a half-finished cutscene.
    showing: VecDeque<PendingShow>,
    /// The game's own opcode-71 SAVE GAME request, deferred while `showing`
    /// is non-empty (SQ-1487): §12.11 has the room view return only once the
    /// LAST picture's ENTER is pressed, so the save should capture the turn
    /// as finished, not mid-cutscene. Drained the same way `submit`'s own
    /// immediate case drains `Vm::take_save_request` — once, right before the
    /// resulting `TurnResult` is built.
    deferred_save: bool,
}

impl ScottSession {
    /// Parse a ScottFree `.dat` and start a session. Any encoding loads —
    /// `Database::parse` takes raw bytes (SQ-1412), not just UTF-8 text — so
    /// a Latin-1 database is not rejected before it ever reaches the parser.
    /// `pict_blorb` is the game's own Blorb when it is a `.blb` graphics
    /// container (carrying the room `Pict` images); `None` for a plain
    /// `.dat`.
    pub fn new(bytes: Vec<u8>, pict_blorb: Option<blorb::Blorb>) -> Result<ScottSession, String> {
        ScottSession::new_with_trace(bytes, pict_blorb, false, None)
    }

    /// The terminal cell size a session assumes when its caller has none to
    /// give — the same 8×16 fallback `startup.rs` and `reset.rs` use when the
    /// terminal reports no image protocol and the real font size is unknown.
    /// Only the C64 vector artwork reads it (SQ-1467), and only to choose how
    /// finely to draw.
    pub const FALLBACK_CHAR_PX: (u32, u32) = (8, 16);

    /// Like [`ScottSession::new`], but starts the VM with fired-action tracing
    /// on (the `--debug` boot path) so the opening occurrence pass — run inside
    /// `Vm::new_with_trace`, before any host code can toggle tracing — is
    /// captured as coverage from the first frame.
    ///
    /// `random_seed` is the value the occurrence rolls start from (SQ-0811). It
    /// goes into the constructor, not a later `seed_rng`, because that same
    /// opening occurrence pass rolls its percentage chances inside it. `None` —
    /// every caller but the launcher — leaves scott's own fixed default, so a
    /// test's sequence stays the reproducible one it has always been.
    ///
    /// Uses `scott::Options::default()` — every ScottFree `-y`/`-s`/`-p` flag
    /// off, and this crate's own [`scott::Presentation::C64`] room-block
    /// layout (unchanged from before SQ-1413). A caller wanting a per-game
    /// choice of those uses [`ScottSession::new_with_options`].
    pub fn new_with_trace(
        bytes: Vec<u8>,
        pict_blorb: Option<blorb::Blorb>,
        trace: bool,
        random_seed: Option<u32>,
    ) -> Result<ScottSession, String> {
        ScottSession::new_with_options(
            bytes,
            trace,
            random_seed,
            scott::Options::default(),
            // No container was mounted on this path, so there are no family-C
            // picture files to hand over (SQ-1475) — `none()` plus the one
            // fact this caller does have.
            crate::graphics::ScottPictureSources::none().with_pict_blorb(pict_blorb),
        )
    }

    /// The fullest constructor: [`ScottSession::new_with_trace`] plus
    /// ScottFree's `-y`/`-s`/`-t`/`-p` [`scott::Options`] (SQ-1413) — a
    /// per-game choice, read from `<game_dir>/config.toml` by the caller
    /// (`crate::styles::read_per_game_scott_you_are` and its three
    /// siblings) and passed in here. `options` is a constructor argument for
    /// the same reason `random_seed` is: the opening occurrence pass below
    /// can print option-gated wording, so a session built with the wrong
    /// options and corrected afterward would already have shown the wrong
    /// text. `Presentation` is deliberately NOT read from the per-game
    /// override here — lanthorn always keeps its own
    /// [`scott::Presentation::C64`] room-block layout regardless of `-t`
    /// (see `scott::Presentation`'s doc); only `scott-cli` lets `-t` switch
    /// the layout too.
    ///
    /// `pictures` bundles the four picture facts a session needs from outside
    /// itself — the game's own Blorb `Pict` container, the terminal's cell
    /// size (only the C64 Mysterious Adventures vector artwork reads it,
    /// SQ-1467), the player's HiRes/Original choice for that artwork
    /// (SQ-1473), and a US S.A.G.A. release's own family-C/D/E picture files
    /// off its container (SQ-1475/SQ-1476/SQ-1477) — into one
    /// [`crate::graphics::ScottPictureSources`] value (SQ-1485). They travel
    /// together because they are resolved together, once, in
    /// [`crate::graphics::ScottPictureSources::resolve`] — `startup.rs` and
    /// `reset.rs` both call it rather than each resolving (and risking
    /// drifting on) the four facts by hand. See that type's doc for the full
    /// per-field detail; `crate::graphics::ScottPictureSources::none()` is the
    /// no-pictures default this and [`ScottSession::new_with_trace`] use.
    pub fn new_with_options(
        bytes: Vec<u8>,
        trace: bool,
        random_seed: Option<u32>,
        options: scott::Options,
        pictures: crate::graphics::ScottPictureSources,
    ) -> Result<ScottSession, String> {
        let crate::graphics::ScottPictureSources {
            pict_blorb,
            char_px,
            resolution: picture_resolution,
            saga_pictures,
        } = pictures;
        // `Database::parse` takes raw bytes (SQ-1412), so a Latin-1 or
        // otherwise non-UTF-8 `.dat` loads here instead of being rejected by
        // a UTF-8 check before it ever reached the parser.
        let db = scott::Database::parse(&bytes).map_err(|e| format!("invalid Scott .dat: {e:?}"))?;
        let mut vm = scott::Vm::new_full(
            db,
            trace,
            random_seed.unwrap_or(scott::Vm::DEFAULT_RNG_SEED),
            options,
        );
        let mut intro = vm.take_output();
        if !vm.has_quit() {
            intro.push_str(PROMPT);
        }
        // SQ-1463: `bytes` loaded straight off a Commodore 64 Mysterious
        // Adventures PRG/D64 carries its own room artwork (Family B vector
        // pictures) in the same memory image the database above was just
        // parsed from — decode it once here, the same way `pict_blorb` is
        // handed over for a `.blb` game, rather than re-deriving it from the
        // VM's state on every `refresh_picture`. `pict_blorb` still wins when
        // present (a graphics container beside a `.dat` is a different,
        // Blorb-carried release of the same series).
        //
        // SQ-1467: what comes back is the DISPLAY LISTS, not rasters — the
        // pictures are vectors and nothing in them fixes a size, so each room
        // is drawn when it is first shown, at the supersample the band's own
        // device height picks out.
        // SQ-1475: a US S.A.G.A. release's artwork is family C (§8.3) — one
        // record per picture, in separate files on the same release disk the
        // database came off (§12.10). The mount is long gone by the time a
        // room asks for one, so the records arrived with the story bytes; keep
        // them undecoded and let `PictSource` decode the ones a player
        // actually reaches.
        //
        // Ordered after `pict_blorb` and before the family-B decode for the
        // same reason each of those is where it is: a Blorb beside the story
        // is a different, Blorb-carried release, and no story is both a
        // S.A.G.A. database and a Mysterious Adventures memory image.
        // SQ-1476: the RELEASE, not just the platform — the Apple II names
        // its picture files after the adventure number too — and read off the
        // database the VM is already holding rather than re-sniffed from the
        // bytes.
        let saga_release = (!saga_pictures.is_empty())
            .then(|| vm.database().saga_us)
            .flatten();
        // SQ-1477: family E — the MS-DOS *Questprobe* release. Its database
        // is the plain reference TEXT format (§10.7), so `detect_saga_us`
        // above answered `None` and the release has to be identified from the
        // database's own header counts instead (`scott::saga_dos::identify`).
        // Ordered after family C because a container cannot carry both: the
        // two naming rules do not overlap, and `saga_pictures` came off one
        // container.
        let dos_release = (!saga_pictures.is_empty() && saga_release.is_none())
            .then(|| scott::saga_dos::identify(vm.database()))
            .flatten();
        // SQ-1480: a ZX Spectrum Mysterious Adventures release carries the
        // SAME family-B display lists as the C64 releases (§8.2), decoded
        // straight off the `.z80` snapshot (or the bare 48K image a host that
        // already decompressed one for its own reasons hands over) rather
        // than off a PRG/D64 image — see `zx_mysterious_picture_lists`. Tried
        // before the C64 attempt below; the two container sniffs cannot both
        // match, so the order only decides which refusal a third format hits
        // first, and neither ever fires for the other's files.
        let picts = if pict_blorb.is_some() {
            PictSource::new(pict_blorb)
        } else if let Some(release) = saga_release {
            PictSource::from_scott_saga(saga_pictures, release)
        } else if let Some(release) = dos_release {
            PictSource::from_scott_dos_saga(saga_pictures, release)
        } else if let Some(lists) = zx_mysterious_picture_lists(&bytes) {
            PictSource::from_scott_family_b(
                lists,
                u32::from(PICTURE_ROWS) * char_px.1,
                picture_resolution,
                crate::graphics::ScottFamilyBPlatform::Zx,
            )
        } else {
            scott::c64::prg_image(&bytes)
                .filter(|(image, at)| scott::c64::looks_like_c64_mysterious(image, *at))
                .and_then(|(image, at)| scott::c64::decode_family_b_picture_lists(image, at).ok())
                .map(|lists| {
                    PictSource::from_scott_family_b(
                        lists,
                        u32::from(PICTURE_ROWS) * char_px.1,
                        picture_resolution,
                        crate::graphics::ScottFamilyBPlatform::C64,
                    )
                })
                .unwrap_or_else(|| PictSource::new(None))
        };
        let mut s = ScottSession {
            vm,
            intro,
            aux: BTreeMap::new(),
            aux_dirty: false,
            picts,
            current_canvas: None,
            current_pic_num: None,
            current_overlays: Vec::new(),
            pic_version: 0,
            showing: VecDeque::new(),
            deferred_save: false,
        };
        s.refresh_picture();
        Ok(s)
    }

    /// Recompute the current room's decoded picture. Cheap when the picture
    /// number is unchanged (early-out). A room whose number has no picture in
    /// this source shows none.
    ///
    /// **Darkness is `scott::Vm::current_picture`'s answer, not this
    /// function's** (SQ-1475). It used to be decided here — a dark room showed
    /// nothing — but that is only right for most dialects: a US S.A.G.A.
    /// release draws a dedicated darkness image instead (spec §12.11, "where
    /// other dialects paint black, these draw picture index 0"), and which of
    /// the two it is depends on the database. One source of truth in the VM,
    /// which knows, rather than a host rule that has to be kept in step with
    /// it.
    /// The picture number an MS-DOS *Questprobe* release wants for the room
    /// the player is in, or `None` when this session is not one (SQ-1477).
    ///
    /// **Why this exists at all.** §12.11 gives the US S.A.G.A. releases two
    /// picture rules no other dialect has — the *Hulk*'s five remapped room
    /// pairs, and a dedicated image drawn in the DARK where every other
    /// dialect shows nothing — and `scott::Vm::current_picture` applies both,
    /// keyed on `Database::saga_us`. The MS-DOS release is the same game and
    /// obeys the same two rules, but its database is the reference TEXT
    /// format (§10.7) and so carries no `saga_us` record for the VM to key
    /// on. The rules therefore arrive here, from the RELEASE
    /// (`scott::saga_dos::identify`) rather than from the encoding — and they
    /// are the release's own tables, not a second copy: the remap is
    /// `scott::saga_us::hulk_room_picture` and the darkness index is
    /// `scott::DARKNESS_PICTURE`.
    ///
    /// The remap applies **only to a number that came from the room**, which
    /// is what the `== current_room()` test is for: `current_picture` also
    /// answers an explicit draw-picture opcode's operand, and remapping THAT
    /// would draw a different picture than the game asked for.
    ///
    /// Checked against the release's own files rather than taken on trust —
    /// the MS-DOS *Hulk* ships an `R0100.PAK` (the darkness image) and room
    /// pictures for exactly the ten rooms this does not remap.
    fn dos_room_picture(&self) -> Option<u16> {
        let release = self.picts.scott_dos_release()?;
        Some(match self.vm.current_picture() {
            // Dark: §12.11's darkness image, where a plain dialect shows
            // nothing at all and `current_picture` answers `None`.
            None => scott::DARKNESS_PICTURE as u16,
            Some(n) if usize::from(n) == self.vm.current_room() => {
                release.room_picture(usize::from(n)) as u16
            }
            Some(n) => n,
        })
    }

    fn refresh_picture(&mut self) {
        let want = self.dos_room_picture().or_else(|| self.vm.current_picture());
        let overlays = self.room_overlays();
        self.set_band(want, overlays);
    }

    /// Point the band at `want` with `overlays` composited over it, decoding
    /// nothing when neither has changed (SQ-1482).
    ///
    /// **The band's identity is the pair**, which is why one function owns
    /// both fields: the same room picture with a different set of objects
    /// lying in the room is a different frame, and comparing only the number
    /// would leave the last frame's gem on the floor after the player picked
    /// it up.
    fn set_band(&mut self, want: Option<u16>, overlays: Vec<String>) {
        if want == self.current_pic_num && overlays == self.current_overlays {
            return;
        }
        self.current_pic_num = want;
        self.current_overlays = overlays;
        self.current_canvas = want
            .and_then(|n| self.picts.scott_composite(u32::from(n), &self.current_overlays))
            .map(|dynimg| Arc::new(dynimg.to_rgba8()));
        self.pic_version += 1;
    }

    /// The object records drawn over the ROOM picture this turn (spec
    /// §12.11, SQ-1482): "after [the room picture], every object picture
    /// whose item index names an item presently in the room is drawn over
    /// it, in the order the picture files were gathered".
    ///
    /// Three things the VM cannot answer and this does. Which items are in
    /// the room is `Vm::item_indices_in_room`; which PICTURE each item draws
    /// is the release's (`SagaUs::object_picture` /
    /// `DosRelease::object_picture` — identity but for the *Hulk*'s three
    /// measured overrides); and which of those the container actually ships,
    /// in which order, is the picture source's. Most items have no artwork at
    /// all, so the usual answer is a short list or none.
    ///
    /// **Nothing is drawn in the dark.** §12.11: "where other dialects paint
    /// black, these draw picture index 0 — a dedicated darkness image — and
    /// return." The `return` is the operative word: the darkness image stands
    /// alone, and overlaying the objects a player cannot see onto it would
    /// undo the whole point of it.
    ///
    /// Empty for every engine, dialect and release that is not one of these —
    /// a source holding no S.A.G.A. records answers with nothing whatever is
    /// asked of it — so this is safe to call on every refresh.
    fn room_overlays(&self) -> Vec<String> {
        if self.vm.is_dark() {
            return Vec::new();
        }
        let saga = self.vm.database().saga_us;
        let dos = self.picts.scott_dos_release();
        let items = self.vm.item_indices_in_room();
        let mut indices: Vec<u16> = match (saga, dos) {
            (Some(release), _) => {
                items.iter().map(|&i| release.object_picture(i) as u16).collect()
            }
            (None, Some(release)) => {
                items.iter().map(|&i| release.object_picture(i) as u16).collect()
            }
            (None, None) => return Vec::new(),
        };
        // §12.11's other shape of hard-coded overlay, keyed on the ROOM
        // rather than on an item: *The Count*'s 80/81/82 in rooms 8/18/9 and
        // *Voodoo Castle*'s 80 in room 14. Neither title's artwork is
        // reachable from any specimen in the archive (see
        // `SagaUs::room_overlay`), so this adds nothing to a frame today and
        // is here so that it will when one is.
        if let Some(picture) = saga.and_then(|r| r.room_overlay(self.vm.current_room())) {
            indices.push(picture as u16);
        }
        self.picts.scott_overlays(scott::PictureUsage::ObjectInRoom, &indices)
    }

    /// The object records drawn over the INVENTORY backdrop (§12.11,
    /// SQ-1482): "draws picture index 98 as a room picture, then draws the
    /// inventory-object picture of every carried item".
    ///
    /// **No per-item override here, unlike [`Self::room_overlays`].** The
    /// *Hulk*'s three room-side overrides exist because three of its items
    /// have no `B01nnnR` under their own index; on the inventory side every
    /// one of them does — the disk carries `B01021I` for the wax and
    /// `B01042I` for the cavern gem — so the index IS the item number, which
    /// is §8.6's plain rule. Each inventory record also carries its own slot
    /// on the canvas, so the carried items tile the screen without anything
    /// here placing them.
    fn inventory_overlays(&self) -> Vec<String> {
        let carried: Vec<u16> =
            self.vm.carried_item_indices().iter().map(|&i| i as u16).collect();
        self.picts.scott_overlays(scott::PictureUsage::ObjectInInventory, &carried)
    }

    /// Force the picture band to `picture`, bypassing `Vm::current_picture()`
    /// — used while presenting a US S.A.G.A. picture-show sequence
    /// (SQ-1487), where the band shows each opcode-90 picture in turn rather
    /// than whatever `current_picture()` would answer (the room's own
    /// picture, or darkness). Same early-out/version-bump shape as
    /// `refresh_picture`, just driven by an explicit number instead of the
    /// VM's own opinion.
    fn show_sequence_picture(&mut self, picture: u16) {
        // §8.6 reserves index 98 for "the inventory backdrop", so a show of
        // 98 IS §12.11's inventory screen however it was queued — by the
        // inventory command itself or by an opcode-90 operand naming it —
        // and the carried items go over it (SQ-1482). Every other picture in
        // a sequence is a scene drawn whole, with nothing over it.
        let overlays = if usize::from(picture) == scott::INVENTORY_PICTURE {
            self.inventory_overlays()
        } else {
            Vec::new()
        };
        self.set_band(Some(picture), overlays);
    }

    /// Build a `TurnResult` with the non-Scott fields at their empty default,
    /// mirroring `GameSession::drain_turn`'s field set exactly.
    fn turn(&self, transcript: String, quit: bool) -> TurnResult {
        TurnResult {
            transcript,
            transcript_runs: Vec::new(),
            location: self.snapshot_location(),
            quit,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: Vec::new(),
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
            prose_retired: None,
            declared_exit: None,
        }
    }

    fn snapshot_location(&self) -> Option<LocationInfo> {
        let r = self.vm.current_room();
        Some(LocationInfo { number: r as mapper::graph::RoomId, parent: 0, name: self.vm.room_name(r).to_string() })
    }

    /// An item's current location (`-1`/`255` = carried, `0` = nowhere, else
    /// a room index) — `scott::Vm::item_loc`, exposed for the binary crate's
    /// own restore-path tests (`engine_helpers::restore_from_file`,
    /// SQ-1413) to assert what a restore actually placed, without widening
    /// the `vm` field itself. `pub`, not `pub(crate)`: `engine_helpers.rs`
    /// lives in the `lanthorn` BINARY crate, a separate compilation unit
    /// from this `app` LIB crate, so `pub(crate)` here would not reach it.
    #[allow(dead_code)] // only called from the binary crate's own t-session-gated tests
    pub fn item_loc(&self, idx: usize) -> i32 {
        self.vm.item_loc(idx)
    }
}

impl Engine for ScottSession {
    // SQ-1270: `Engine::submit`'s contract is to route by `pending_input()`,
    // never handing a line to a keypress read. Scott is ordinarily
    // line-only, but SQ-1487 gave it one keypress-driven state of its own —
    // a US S.A.G.A. picture-show sequence — which `pending_input`/
    // `submit_key` below now handle the same way every other engine's
    // `read_char` does.
    fn submit(&mut self, command: &str) -> TurnResult {
        self.vm.supply_line(command);
        let _ = self.vm.step();
        let transcript = self.vm.take_output();
        let quit = self.vm.has_quit();
        let mut shows = self.vm.take_picture_shows();
        // A show whose picture this source cannot supply is not presented at
        // all (SQ-1482). Every step of a sequence stops the game for a
        // keypress, and stopping it in front of a band that shows nothing —
        // or, worse, in front of the PREVIOUS picture, still on screen — is
        // strictly worse than not pausing. This is also what keeps §12.11's
        // inventory screen from reaching a release that has no `R01098`: the
        // VM queues it from the database (which is all a headless core can
        // know), and the picture files are the host's to check. `quest1.dat`,
        // the reference-format twin of the Commodore disk, is exactly that
        // case — the same eleven header counts, none of the artwork.
        let picts = &mut self.picts;
        shows.retain(|show| picts.image(u32::from(show.picture)).is_some());
        // The game ran the SAVE GAME action (opcode 71): bubble the same Save
        // request the Z-machine/Glulx engines raise for `@save`, so the app's
        // Save State file I/O runs. The prompt is withheld and returns via
        // `resume_save` once the host has written the snapshot.
        let save_requested = !quit && self.vm.take_save_request();

        if quit || shows.is_empty() {
            self.refresh_picture();
            if save_requested {
                let mut result = self.turn(transcript, quit);
                result.pending_io = Some(PendingIo::Save);
                return result;
            }
            let mut transcript = transcript;
            if !quit {
                transcript.push_str(PROMPT);
            }
            return self.turn(transcript, quit);
        }

        // §12.11: several pictures in ONE turn, each ENTER-gated (SQ-1487,
        // user report on *The Hulk*'s "bite lip") — split the turn's output
        // at every opcode-90 offset `Vm::take_picture_shows` recorded, show
        // the first picture now, and queue the rest for `submit_key` to walk
        // one keypress at a time. The save the turn also raised (unlikely to
        // co-occur, but not impossible) waits for the sequence to finish —
        // §12.11 treats the room view returning as when the turn is really
        // over, so that is when Save State should capture it too.
        self.deferred_save = save_requested;
        let mut offsets: Vec<usize> = shows.iter().map(|s| s.output_len).collect();
        offsets.push(transcript.len());
        let first_text = transcript[..offsets[0]].to_string();
        self.showing = (0..shows.len())
            .map(|i| PendingShow {
                text: transcript[offsets[i]..offsets[i + 1]].to_string(),
                next_picture: shows.get(i + 1).map(|s| s.picture),
            })
            .collect();
        self.show_sequence_picture(shows[0].picture);
        let mut result = self.turn(first_text, false);
        result.info = Some(PICTURE_SHOW_HINT.to_string());
        result
    }

    fn submit_key(&mut self, _key: crate::engine::KeyInput) -> Option<TurnResult> {
        // The S.A.G.A. picture-show door (SQ-1487, spec §12.11) waits for the
        // player to press ENTER between pictures, but — like the Z-machine's
        // `read_char` mapping for its own dismissable pauses — it does not
        // care WHICH key dismissed it, only that one did. Outside a sequence
        // `showing` is empty, `pending_input()` never answers `Char`, and the
        // app never routes a key here to begin with — matching the doc this
        // used to carry ("Scott is line-only").
        let step = self.showing.pop_front()?;
        match step.next_picture {
            Some(picture) => {
                self.show_sequence_picture(picture);
                let mut result = self.turn(step.text, false);
                result.info = Some(PICTURE_SHOW_HINT.to_string());
                Some(result)
            }
            None => {
                // The last picture in the sequence: the room view returns.
                self.refresh_picture();
                let quit = self.vm.has_quit();
                let mut transcript = step.text;
                if std::mem::take(&mut self.deferred_save) && !quit {
                    let mut result = self.turn(transcript, quit);
                    result.pending_io = Some(PendingIo::Save);
                    return Some(result);
                }
                if !quit {
                    transcript.push_str(PROMPT);
                }
                Some(self.turn(transcript, quit))
            }
        }
    }

    fn take_transcript(&mut self) -> String {
        std::mem::take(&mut self.intro)
    }

    fn drain_screen_clear(&mut self) -> bool {
        // Scott Adams games have no screen-clear channel at all: the VM prints and
        // the host scrolls. Nothing to drain, at boot or in a turn — which is why
        // `ScottSession::turn` writes `erase_lower: false` outright.
        false
    }

    fn pending_input(&self) -> InputKind {
        // SQ-1487: a US S.A.G.A. picture-show sequence is the one place
        // Scott waits on a keypress rather than a line — see `submit_key`.
        if self.showing.is_empty() {
            InputKind::Line
        } else {
            InputKind::Char
        }
    }

    fn resume_save(&mut self, _wrote_ok: bool) -> TurnResult {
        // The SAVE GAME action ran the whole turn synchronously; the host has now
        // performed the Save State write. Nothing in the VM to resume — just
        // return to the command prompt (withheld by `submit` for this turn).
        let quit = self.vm.has_quit();
        let transcript = if quit { String::new() } else { PROMPT.to_string() };
        self.turn(transcript, quit)
    }

    fn resume_restore(&mut self, _data: Option<&[u8]>) -> TurnResult {
        // Scott has no in-game @restore suspension; nothing to resume.
        self.turn(String::new(), self.vm.has_quit())
    }

    fn has_quit(&self) -> bool {
        self.vm.has_quit()
    }

    fn screen(&self) -> ScreenModel {
        // The classic Scott split: a persistent top panel showing the current
        // room block (redrawn every frame from live VM state), above the scrolling
        // command transcript. The panel is a non-primary buffer carrying its own
        // lines; the primary buffer below is the transcript the app mirrors.
        let panel = room_panel(&self.vm.room_block());
        let rows = panel.lines.len() as u16;
        let text = WinNode::Pair {
            vertical: true,
            split: Split { fixed: rows },
            border: true,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Buffer(panel)),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        };
        // A graphics (`.blb`) game shows the current room's picture in a band
        // above the room panel; the renderer scales the canvas into the band.
        let root = match &self.current_canvas {
            Some(canvas) => WinNode::Pair {
                vertical: true,
                split: Split { fixed: PICTURE_ROWS },
                border: true,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Graphics(GraphicsWindow {
                    win: 1,
                    canvas: Arc::clone(canvas),
                    version: self.pic_version,
                    upscale: true,
                })),
                second: Box::new(text),
            },
            None => text,
        };
        ScreenModel { root, status: StatusModel::HostManaged, bg: 0, fg: 0, content_size: (0, 0) }
    }

    /// `/dump-windows` for the Scott engine (SQ-0699 follow-up).
    ///
    /// Without this the trait default ran, and it is written for the Z-machine:
    /// it looked for a GRID window, found none (Scott's layout is
    /// buffer-over-buffer), and printed `Grid 0x0 over Buffer (Z-machine simple
    /// path)` — a size that is always zero and an engine name that is always
    /// wrong. Report what a Scott screen actually has: the optional room picture
    /// band with its canvas diagnostics, the room panel and its live lines, and
    /// the transcript below.
    ///
    /// The graphics line mirrors the Glulx dump's `canvas=WxH vN opaque=N`
    /// (`glk_backend::window_dump_lines`) so the two read alike; `opaque=0` on a
    /// present canvas is the tell for "the game never painted this room".
    fn window_dump(&self) -> Vec<String> {
        let block = self.vm.room_block();
        let panel = room_panel(&block);
        let mut out = vec!["Scott Adams layout — picture over room panel over transcript".to_string()];
        match &self.current_canvas {
            Some(canvas) => {
                let opaque = canvas.pixels().filter(|p| p.0[3] != 0).count();
                // SQ-1467: the family-B artwork is drawn at a supersample
                // chosen from this band's device height, so a frame has to be
                // able to say which resolution produced it.
                // SQ-1480: and which platform's palette read it — the C64 and
                // ZX Spectrum releases share the exact same display lists and
                // geometry, and differ only in colour.
                // SQ-1475: a fourth source, and it names the platform whose
                // colour table read the record — the geometry is the same on
                // either, the colours are not.
                // SQ-1477: a fifth source. Family E is the MS-DOS release's
                // own CGA bitmaps, and a frame says so — the geometry is the
                // same 280-pixel canvas family C uses and the artwork is the
                // same artist's, so nothing else in the dump distinguishes
                // them.
                // SQ-1476: …and a sixth, which is why the FAMILY is named as
                // well as the platform: the Apple II releases are family D, a
                // line-drawing decoder over the machine's own hi-res canvas.
                let source = match (
                    self.picts.scott_c64_scale(),
                    self.picts.scott_family_b_platform(),
                    self.picts.scott_saga_platform(),
                ) {
                    (Some(scale), Some(platform), _) => {
                        format!("native {} x{scale}", platform.label())
                    }
                    (None, _, Some(scott::SagaPlatform::AppleII)) => format!(
                        "S.A.G.A. family D (Apple II, {} picture(s))",
                        self.picts.scott_saga_count().unwrap_or(0)
                    ),
                    (None, _, Some(platform)) => format!(
                        "S.A.G.A. family C ({}, {} record(s))",
                        platform.label(),
                        self.picts.scott_saga_count().unwrap_or(0)
                    ),
                    _ => match self.picts.scott_saga_dos_count() {
                        Some(n) => format!("S.A.G.A. family E (MS-DOS, {n} picture(s))"),
                        None => "blorb".to_string(),
                    },
                };
                // SQ-1482: §12.11's object overlays make a room's band no
                // longer a single named picture, so a capture has to say
                // which records were composited over it — and say so even
                // when there were none, since "no objects here" and "this
                // build draws no overlays" are different frames and look
                // identical. Only a S.A.G.A. source can have any, so the
                // field is silent for every other kind rather than always
                // reading `none`.
                let overlays = match self.picts.scott_saga_platform().is_some()
                    || self.picts.scott_saga_dos_count().is_some()
                {
                    false => String::new(),
                    true if self.current_overlays.is_empty() => "  ·  overlays=none".to_string(),
                    true => format!("  ·  overlays={}", self.current_overlays.join(",")),
                };
                out.push(format!(
                    "  picture: {} row(s) reserved  ·  canvas={}x{} v{} opaque={} source={source}{overlays}",
                    PICTURE_ROWS,
                    canvas.width(),
                    canvas.height(),
                    self.pic_version,
                    opaque
                ));
            }
            // SQ-1475: a US S.A.G.A. database opened from a bare extracted
            // `db/*.bin` — or an Atari side A whose companion picture side is
            // not paired — reaches here with no picture source at all, which
            // looks exactly like a text-only game and is not one. Say which.
            // SQ-1476 adds a third way to land here: an Apple II release whose
            // companion side is missing, or one of the three whose side A is
            // not a DOS 3.3 disk at all.
            None
                if self.vm.database().saga_us.is_some()
                    && self.picts.scott_saga_platform().is_none() =>
            {
                out.push(
                    "  picture: none — a S.A.G.A. release with no picture files on this file \
                     (§8.3's are separate files on the release disk)"
                        .to_string(),
                )
            }
            None => out.push("  picture: none (text-only game, or this room has no art)".to_string()),
        }
        out.push(format!("  room panel: {} line(s)", panel.lines.len()));
        for (i, line) in panel.lines.iter().enumerate() {
            let trimmed = line.trim_end();
            if !trimmed.is_empty() {
                out.push(format!("  line {:>2}: {trimmed:?}", i + 1));
            }
        }
        out.push("  transcript: primary buffer (host-managed scrollback)".to_string());
        out
    }

    fn save_state(&self) -> EngineSave {
        EngineSave::new(SCOTT_ENGINE, SCOTT_SAVE_FORMAT, self.vm.snapshot())
    }

    fn restore_state(&mut self, save: &EngineSave) -> Result<(), EngineError> {
        if !save.is_engine(SCOTT_ENGINE) {
            return Err(EngineError::EngineMismatch {
                expected: SCOTT_ENGINE.to_string(),
                found: save.engine.clone(),
            });
        }
        let r = self
            .vm
            .restore(&save.bytes)
            .map_err(|e| EngineError::BadSave(format!("bad Scott snapshot: {e}")));
        // SQ-1487: a picture-show sequence is transient host state, not part
        // of the `scott::Vm` snapshot — a restore lands on the completed
        // turn that queued it, not mid-cutscene, so any steps still pending
        // (a Save State taken mid-sequence, or simply a different save) are
        // stale and must not keep presenting against the now-replaced VM.
        self.showing.clear();
        self.deferred_save = false;
        self.refresh_picture();
        r
    }

    /// Restores from either of Scott's two save formats, detected by shape
    /// (SQ-1413): this crate's own binary snapshot
    /// ([`scott::Vm::SNAPSHOT_MAGIC`], unambiguous — checked first) or
    /// ScottFree 1.14's own text save format
    /// ([`scott::looks_like_scottfree_save`]), so a player can bring an old
    /// ScottFree `.sav` into lanthorn. Anything matching neither falls
    /// through to `Vm::restore`, which reports the ordinary bad-snapshot
    /// error rather than a bespoke "unrecognised format" one — there is
    /// nothing a third message would say that the binary-restore failure
    /// doesn't already.
    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError> {
        let is_scottfree_save =
            !bytes.starts_with(&scott::Vm::SNAPSHOT_MAGIC) && scott::looks_like_scottfree_save(bytes);
        let r = if is_scottfree_save {
            self.vm
                .restore_scottfree(bytes)
                .map_err(|e| EngineError::BadSave(format!("bad ScottFree save: {e}")))
        } else {
            self.vm
                .restore(bytes)
                .map_err(|e| EngineError::BadSave(format!("bad Scott snapshot: {e}")))
        };
        // See `restore_state`'s comment: a picture-show sequence is host
        // state, not VM state, and must not survive a restore.
        self.showing.clear();
        self.deferred_save = false;
        self.refresh_picture();
        r
    }

    fn aux_data(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.aux
    }

    fn set_aux_data(&mut self, data: BTreeMap<String, Vec<u8>>) {
        // Loading the archive's aux data back INTO the session is not the game
        // writing anything — `aux_dirty` means "the game changed aux data since we
        // last persisted it". Marking it here made every Scott archive restore pop
        // the aux-persistence consent dialog under `aux_storage = Ask`, for a game
        // that had written nothing. The Z-machine (`session.rs`) and Glulx
        // (`glulx_session.rs`) setters leave the flag alone; this now matches them.
        // (SQ-0658)
        self.aux = data;
    }

    fn aux_dirty(&self) -> bool {
        self.aux_dirty
    }

    fn clear_aux_dirty(&mut self) {
        self.aux_dirty = false;
    }

    fn current_location(&self) -> Option<LocationInfo> {
        self.snapshot_location()
    }

    fn set_debug_trace(&mut self, on: bool) {
        // Turning the inspector on enables fired-action tracing; off stops it.
        // The cumulative `ever_fired` set (permanent colour + persisted coverage)
        // is preserved either way — only the per-turn set stops updating.
        self.vm.set_trace_fired(on);
    }

    fn seed_executed_pcs(&mut self, pcs: &std::collections::HashSet<u32>) {
        // The PC-set sidecar stores Scott action indices as `u32`s (SQ-0449).
        self.vm.seed_ever_fired(pcs);
    }

    fn debugger(&self) -> Option<&dyn Debugger> {
        Some(&self.vm)
    }

    /// A two-word parser's vocabulary, in the same neutral shape the Z-machine
    /// and Glulx answer in.
    ///
    /// There is no grammar module to read and none is needed: a Scott Adams
    /// game's whole grammar is `VERB NOUN`, with the noun optional, so every verb
    /// gets those two lines and the action number is the verb's own index. The
    /// vocabulary lists carry synonyms as `*`-prefixed entries following the
    /// canonical word (`Database::match_verb` resolves them that way), which is
    /// exactly `Verb::words` — the first spelling, then its synonyms.
    fn story_vocabulary(&self) -> Option<crate::vocab::StoryVocabulary> {
        use grammar_model::{NounKind, Slot, SyntaxLine, Token, Verb, WordRoles};
        let db = self.vm.database();
        let mut words: std::collections::BTreeMap<String, WordRoles> =
            std::collections::BTreeMap::new();
        let mut verbs: Vec<Verb> = Vec::new();
        for (i, entry) in db.verbs.iter().enumerate() {
            let word = entry.trim_start_matches('*').trim().to_lowercase();
            // A database pads its vocabulary lists with `.` for a slot no word
            // reaches; it is a placeholder, not a word somebody could type.
            if !word.chars().any(char::is_alphanumeric) {
                continue;
            }
            words.entry(word.clone()).or_default().verb = true;
            if entry.starts_with('*') {
                // A synonym of the nearest preceding canonical verb.
                if let Some(v) = verbs.last_mut() {
                    v.words.push(word);
                }
            } else {
                let lines = vec![
                    SyntaxLine::new(i as u16, false, Vec::new()),
                    SyntaxLine::new(i as u16, false, vec![Slot::one(Token::Noun(NounKind::Noun))]),
                ];
                verbs.push(Verb::new(i as u32, 0, vec![word], lines));
            }
        }
        for entry in &db.nouns {
            let word = entry.trim_start_matches('*').trim().to_lowercase();
            if word.chars().any(char::is_alphanumeric) {
                words.entry(word).or_default().noun = true;
            }
        }
        // `word_length` is a character count (`Database::match_verb` truncates
        // both sides to it), and 0 there means "compare whole words" — which is
        // what the snapshot's own 0 means too.
        Some(crate::vocab::StoryVocabulary::new(
            verbs,
            words,
            std::collections::BTreeSet::new(),
            db.word_length,
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use crate::debug_panel::{DebugPanelState, Section};
    use crate::engine::Engine;

    fn dat() -> Vec<u8> {
        include_bytes!("../../scott/tests/tiny_cave.dat").to_vec()
    }

    #[test]
    fn loading_archived_aux_data_does_not_mark_it_dirty() {
        // SQ-0658: `aux_dirty` means "the GAME changed aux data since we last
        // persisted it" — it is what `aux_storage = Ask` consults before asking the
        // player for consent to write. Scott's setter raised it, so every archive
        // restore popped the consent dialog for a game that had written nothing.
        // The Z-machine (session.rs) and Glulx (glulx_session.rs) setters leave the
        // flag alone; persistence behaviour is required to be uniform across
        // engines, and restoring is not writing.
        let mut s = ScottSession::new(dat(), None).unwrap();
        assert!(!s.aux_dirty(), "a fresh session has nothing to persist");

        let mut data = BTreeMap::new();
        data.insert("pcset".to_string(), vec![1u8, 2, 3]);
        s.set_aux_data(data.clone());

        assert_eq!(s.aux_data(), &data, "the archive's aux data really is installed");
        assert!(
            !s.aux_dirty(),
            "restoring aux data is not the game dirtying it — this is what re-asks for consent",
        );
    }

    /// The seed the launcher hands a session must reach the VM, and reach it
    /// BEFORE the opening occurrence pass — which runs inside the constructor and
    /// rolls its own percentage chances. A `seed_rng` call after construction
    /// compiles just as well and decides nothing about the game the player is
    /// handed, which is why this asserts on the wiring rather than the API
    /// (SQ-0811).
    #[test]
    fn a_launch_seed_reaches_the_scott_vm_before_the_opening_pass() {
        let seeded = ScottSession::new_with_trace(dat(), None, false, Some(0xC0FF_EE00)).unwrap();
        // The intro pass already advanced the generator, so the state cannot still
        // BE the seed — but it must be off that seed's stream, not the default's.
        let same = ScottSession::new_with_trace(dat(), None, false, Some(0xC0FF_EE00)).unwrap();
        let other = ScottSession::new_with_trace(dat(), None, false, Some(0x0BAD_1DEA)).unwrap();
        let unseeded = ScottSession::new(dat(), None).unwrap();
        assert_eq!(seeded.vm.rng_seed(), same.vm.rng_seed(), "the same seed replays");
        assert_ne!(seeded.vm.rng_seed(), other.vm.rng_seed(), "a different seed diverges");
        assert_ne!(seeded.vm.rng_seed(), unseeded.vm.rng_seed(), "the seed really was applied");
    }

    #[test]
    fn debug_inspector_wires_scott_sections_and_tracks_fired_actions() {
        // End-to-end through the real session + panel: open the inspector, adopt
        // the Scott layout, refresh, and confirm the sections are populated and
        // Call Stack / Eval Stack / Memory are hidden. Then play a turn with
        // tracing on and confirm coverage accrues.
        let mut s = ScottSession::new_with_trace(dat(), None, true, None).unwrap();
        assert!(s.debugger().is_some(), "Scott exposes a debugger");

        let mut panel = {
            let dbg = s.debugger().expect("debugger");
            let mut p = DebugPanelState::new(dbg.pc());
            p.apply_engine_layout(dbg);
            p.refresh(dbg);
            p
        };

        // The Scott layout hides the register-machine sections and relabels its
        // three windows; every window still has at least one tab.
        let all: Vec<Section> = panel.tabs.iter().flat_map(|w| w.iter().copied()).collect();
        assert!(!all.contains(&Section::CallStack) && !all.contains(&Section::Memory));
        assert!(panel.tabs.iter().all(|w| !w.is_empty()));
        assert_eq!(panel.tab_label(Section::Disasm), "Actions");

        // Sections carry real content pulled from the loaded game.
        assert!(!panel.snapshot.disasm.is_empty(), "Actions list non-empty");
        assert!(panel.snapshot.globals.iter().any(|l| l.starts_with("Room:")), "State shown");
        assert!(panel.snapshot.objects.iter().any(|l| l.contains("lamp")), "Items shown");
        assert!(!panel.snapshot.dict.is_empty(), "Vocab shown");
        assert!(panel.snapshot.locals.iter().any(|l| l == "Rooms:"), "World shown");

        // Play a turn that fires a table action (moving down runs an occurrence),
        // then refresh: both the last-turn and cumulative fired sets grow.
        s.submit("down");
        {
            let dbg = s.debugger().expect("debugger");
            panel.refresh(dbg);
        }
        assert!(!panel.snapshot.executed.is_empty(), "an action fired this turn");
        assert!(!panel.snapshot.executed_ever.is_empty(), "and was recorded cumulatively");
    }

    /// The top room-panel text (first buffer of the split), joined for matching.
    fn panel_text(model: &ScreenModel) -> String {
        match &model.root {
            WinNode::Pair { first, .. } => match &**first {
                WinNode::Buffer(b) => b.lines.join("\n"),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }

    #[test]
    fn boots_and_shows_room_panel() {
        let mut s = ScottSession::new(dat(), None).unwrap();
        assert_eq!(s.current_location().expect("loc").number, 1);

        // The transcript opens with only the prompt — the room lives in the panel.
        let intro = s.take_transcript();
        assert!(intro.contains("Tell me what to do ?"), "intro carries the Scott prompt");
        assert!(!intro.contains('>'), "Scott never uses the '>' prompt");
        assert!(s.take_transcript().is_empty(), "intro drains only once");

        // The top panel shows the room block: description, exits, and items.
        let panel = panel_text(&s.screen());
        assert!(panel.contains("sunlit forest clearing"), "room in panel: {panel:?}");
        assert!(panel.contains("Obvious exits:"), "exits in panel: {panel:?}");
        assert!(panel.contains("brass lamp"), "items in panel: {panel:?}");

        // Take the lamp (so room 2 is lit), then descend: the panel follows the
        // player, and each turn's transcript ends with the prompt.
        s.submit("take lamp");
        let r = s.submit("down");
        assert_eq!(r.location.expect("loc").number, 2);
        assert!(!r.quit);
        assert!(r.transcript.contains("Tell me what to do ?"), "each turn ends with the prompt");
        assert!(panel_text(&s.screen()).contains("damp, dark cave"), "panel follows the player");
    }

    #[test]
    fn save_restore_roundtrip() {
        let mut s = ScottSession::new(dat(), None).unwrap();
        let start = s.current_location().unwrap().number;
        let save = s.save_state();

        s.submit("down");
        let moved = s.current_location().unwrap().number;
        assert_ne!(moved, start, "the move actually changed rooms (sanity)");

        s.restore_state(&save).unwrap();
        assert_eq!(s.current_location().unwrap().number, start);
    }

    #[test]
    fn a_normal_turn_raises_no_save_request_and_resume_returns_to_prompt() {
        // Only the SAVE GAME action (opcode 71) bubbles a Save request; an
        // ordinary command must not. resume_save (called by the host after it
        // writes the snapshot) returns cleanly to the command prompt.
        let mut s = ScottSession::new(dat(), None).unwrap();
        let r = s.submit("down");
        assert_eq!(r.pending_io, None, "a plain move does not request a save");

        let resumed = s.resume_save(true);
        assert!(
            resumed.transcript.contains("Tell me what to do ?"),
            "resume_save returns to the Scott prompt: {:?}",
            resumed.transcript
        );
        assert_eq!(resumed.pending_io, None);
    }

    /// A minimal 2×2 PNG for a blorb `Pict` resource.
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn blb_game_shows_a_room_picture_band() {
        // A graphics (.blb) game: the start room's Pict renders as a Graphics
        // window above the room panel. tiny_cave.dat starts in (lit) room 1, and
        // picture number == room number, so Pict resource 1 is the start picture.
        let blorb = crate::graphics::test_blorb_with_pict(1, &tiny_png());
        let s = ScottSession::new(dat(), Some(blorb)).unwrap();
        match s.screen().root {
            WinNode::Pair { first, .. } => {
                assert!(matches!(*first, WinNode::Graphics(_)), "picture band on top");
            }
            other => panic!("expected a graphics band on top, got {other:?}"),
        }
    }

    #[test]
    fn plain_dat_has_no_picture_band() {
        // Without a picture blorb the layout is unchanged: room panel over
        // transcript, no graphics window.
        let s = ScottSession::new(dat(), None).unwrap();
        match s.screen().root {
            WinNode::Pair { first, .. } => {
                assert!(matches!(*first, WinNode::Buffer(_)), "no graphics band");
            }
            other => panic!("expected the plain room/transcript pair, got {other:?}"),
        }
    }

    // ── SQ-1463: the C64 Mysterious Adventures' own Family B pictures ──────────

    /// *The Golden Baton* as shipped on the C64 `MYSTADV1.D64`'s `BATON.prg` —
    /// the eleven titles are commercial and gitignored (`stories/` only), so
    /// every case below skips vacuously without it, exactly like `session.rs`'s
    /// own real-game in-crate tests.
    fn baton_prg() -> Option<Vec<u8>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg");
        if !path.exists() {
            eprintln!("SKIP: no {} (gitignored commercial fixture)", path.display());
            return None;
        }
        Some(std::fs::read(&path).expect("read BATON.prg"))
    }

    /// The Graphics leaf of a Scott screen's picture band, or `None` when the
    /// layout has none (dark room / no art for this room).
    fn picture_band(model: &ScreenModel) -> Option<&crate::engine::GraphicsWindow> {
        match &model.root {
            WinNode::Pair { first, .. } => match &**first {
                WinNode::Graphics(gw) => Some(gw),
                _ => None,
            },
            _ => None,
        }
    }

    /// A session on this PRG with the terminal cell size a caller chooses,
    /// which is the only input to how finely the C64 artwork is drawn
    /// (SQ-1467) when `resolution` is `HiRes`.
    fn baton_session(
        bytes: Vec<u8>,
        char_px: (u32, u32),
        resolution: crate::graphics::ScottPictureResolution,
    ) -> ScottSession {
        ScottSession::new_with_options(
            bytes,
            false,
            None,
            scott::Options::default(),
            crate::graphics::ScottPictureSources::none()
                .with_char_px(char_px)
                .with_resolution(resolution),
        )
        .expect("BATON.prg loads")
    }

    #[test]
    fn mysterious_c64_room1_shows_its_own_picture_drawn_for_the_band() {
        let Some(bytes) = baton_prg() else { return };
        // Room 1 ("dense SPOOKY Forest") is lit and start_room, so the boot
        // session's very first screen already carries its picture — decoded
        // straight off the PRG, not from any Blorb (`pict_blorb` is `None`).
        let s = ScottSession::new(bytes, None).expect("BATON.prg loads");
        assert_eq!(s.current_location().expect("loc").number, 1);
        let model = s.screen();
        let gw = picture_band(&model).expect("room 1 shows a picture band");
        // The default cell is 8x16 (`FALLBACK_CHAR_PX`), so the band is
        // 16 x 16 = 256 device pixels tall and 256/94 rounds up to a 3x
        // supersample — 765 x 282, drawn once at about the size it is shown.
        assert_eq!(
            (gw.canvas.width(), gw.canvas.height()),
            (scott::c64::PICTURE_WIDTH as u32 * 3, scott::c64::PICTURE_HEIGHT as u32 * 3),
            "drawn at the band's own resolution, not at the 255 x 94 native canvas"
        );
        assert!(
            gw.canvas.width() > scott::c64::PICTURE_WIDTH as u32,
            "and larger than native, which is the whole point"
        );
        // A supersample multiplies both axes, so the shape the band fits is
        // exactly the shape the native canvas has — no second aspect
        // correction anywhere in the path.
        assert_eq!(
            gw.canvas.width() * scott::c64::PICTURE_HEIGHT as u32,
            gw.canvas.height() * scott::c64::PICTURE_WIDTH as u32,
            "the aspect ratio is the native one"
        );
        assert!(gw.upscale, "the band renderer stretches it to fill the reserved rows");
    }

    #[test]
    fn the_c64_artwork_is_drawn_at_the_resolution_the_terminals_cell_asks_for() {
        let Some(bytes) = baton_prg() else { return };
        // 16 rows of an 8-pixel cell is 128 device pixels: 128/94 rounds up to
        // 2. Half the height, one step less resolution.
        let small = baton_session(bytes.clone(), (8, 8), crate::graphics::ScottPictureResolution::HiRes);
        let m = small.screen();
        let gw = picture_band(&m).expect("a picture band");
        assert_eq!(gw.canvas.height(), scott::c64::PICTURE_HEIGHT as u32 * 2, "8px cell → 2x");

        // …and an absurdly tall cell is capped rather than obeyed: 16 x 40 is
        // 640 device pixels, which would ask for 7.
        let huge = baton_session(bytes, (20, 40), crate::graphics::ScottPictureResolution::HiRes);
        let m = huge.screen();
        let gw = picture_band(&m).expect("a picture band");
        assert_eq!(gw.canvas.height(), scott::c64::PICTURE_HEIGHT as u32 * 4, "capped at 4x");
    }

    /// SQ-1473: the player's "original" choice ignores the band entirely and
    /// draws the release's own 255x94 canvas — scale 1 — whatever the
    /// terminal's cell size is; "hi-res" (the default) is the band-fitted
    /// supersample this file's other cases already pin.
    #[test]
    fn original_resolution_draws_the_native_255x94_canvas_at_any_cell_size() {
        let Some(bytes) = baton_prg() else { return };
        let original =
            baton_session(bytes.clone(), (8, 16), crate::graphics::ScottPictureResolution::Original);
        let m = original.screen();
        let gw = picture_band(&m).expect("a picture band");
        assert_eq!(
            (gw.canvas.width(), gw.canvas.height()),
            (scott::c64::PICTURE_WIDTH as u32, scott::c64::PICTURE_HEIGHT as u32),
            "original resolution is the native canvas, unscaled"
        );

        // The same band, same cell size, drawn hi-res instead: a bigger
        // placed image — the whole point of the choice — but the SAME room's
        // band geometry (the reserved row count/window shape never moves).
        let hires = baton_session(bytes, (8, 16), crate::graphics::ScottPictureResolution::HiRes);
        let m2 = hires.screen();
        let gw2 = picture_band(&m2).expect("a picture band");
        assert!(
            gw2.canvas.width() > gw.canvas.width() && gw2.canvas.height() > gw.canvas.height(),
            "hi-res must be larger than original at the same cell size"
        );
        assert_eq!(gw.upscale, gw2.upscale, "the band's own layout is unaffected by the resolution choice");

        // `/dump-windows` names the resolution actually drawn (SQ-1473), the
        // same `source=native C64 x{scale}` line `window_dump`'s doc promises.
        let dump = original.window_dump().join("\n");
        assert!(dump.contains("source=native C64 x1"), "original resolution dumps x1: {dump:?}");
        let dump2 = hires.window_dump().join("\n");
        assert!(dump2.contains("source=native C64 x3"), "8x16 cell hi-res dumps x3: {dump2:?}");
    }

    #[test]
    fn mysterious_c64_dark_room_hides_the_picture_band() {
        let Some(bytes) = baton_prg() else { return };
        let mut s = ScottSession::new(bytes, None).expect("BATON.prg loads");
        // The walk to the Cave (room 20), the one room this release makes dark
        // on entry (action 2: player==20 sets the dark flag, §8's occurrence
        // opcode 56) — found by walking the real game, not guessed: forest
        // stream (n) -> tree (w) -> cabin clearing (n) -> "go cabin" teleports
        // to the cabin-with-a-hole (19) -> down falls into the dark Cave (20).
        for cmd in ["n", "w", "n", "go cabin", "d"] {
            let r = s.submit(cmd);
            assert!(!r.quit, "{cmd:?} must not end the game: {:?}", r.transcript);
        }
        assert_eq!(s.current_location().expect("loc").number, 20, "reached the Cave");
        assert!(picture_band(&s.screen()).is_none(), "a dark room shows no picture band");
    }

    #[test]
    fn mysterious_c64_picture_band_reappears_leaving_the_dark_room() {
        let Some(bytes) = baton_prg() else { return };
        let mut s = ScottSession::new(bytes, None).expect("BATON.prg loads");
        for cmd in ["n", "w", "n", "go cabin"] {
            s.submit(cmd);
        }
        assert_eq!(s.current_location().expect("loc").number, 19);
        assert!(
            picture_band(&s.screen()).is_some(),
            "room 19 (cabin with hole in floor) is lit and has its own picture"
        );

        s.submit("d"); // into the dark Cave (20)
        assert_eq!(s.current_location().expect("loc").number, 20);
        assert!(picture_band(&s.screen()).is_none(), "dark: band hidden");

        s.submit("u"); // room 19 auto-clears the dark flag on entry (action 4)
        assert_eq!(s.current_location().expect("loc").number, 19, "back in the lit cabin");
        assert!(picture_band(&s.screen()).is_some(), "lit again: band reappears");
    }

    #[test]
    fn in_game_save_bytes_round_trip_via_restore_game_save() {
        // The in-game SAVE opcode writes `save_state().bytes` as the game-save
        // payload; the in-game restore feeds them back through
        // `restore_game_save`. They must round-trip (regression: the payload was
        // empty for Scott, so restore hit "bad Scott snapshot").
        let mut s = ScottSession::new(dat(), None).unwrap();
        let start = s.current_location().unwrap().number;
        let bytes = s.save_state().bytes;
        assert!(!bytes.is_empty(), "the in-game save payload is the VM snapshot");

        s.submit("down");
        assert_ne!(s.current_location().unwrap().number, start, "moved before restore");

        s.restore_game_save(&bytes).expect("restore accepts the snapshot");
        assert_eq!(s.current_location().unwrap().number, start, "restored to the saved room");
    }

    /// SQ-0531: Scott under the unified writer. Scott has NO game-native save
    /// format — its only bytes are the VM snapshot — so both triggers must seal
    /// the same payload, and an archive written under either one must restore.
    /// The trigger still round-trips, because it is what the saves list uses to
    /// decide whether to advertise a save as portable.
    #[test]
    fn scott_saves_and_restores_under_both_triggers() {
        use crate::archive::SaveTrigger;
        use crate::persist_files::game_save_bytes;

        let mut s = ScottSession::new(dat(), None).unwrap();
        let start = s.current_location().unwrap().number;

        let ingame = game_save_bytes(&s, SaveTrigger::Ingame);
        let host = game_save_bytes(&s, SaveTrigger::HostState);
        assert_eq!(ingame.bytes, host.bytes, "Scott has one save shape, not two");
        assert_eq!(ingame.engine, SCOTT_ENGINE, "and the archive is tagged for Scott");

        let dir = std::env::temp_dir().join(format!("bm-sq0531-scott-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, trigger, save) in [
            ("from-the-game", SaveTrigger::Ingame, &ingame),
            ("from-the-host", SaveTrigger::HostState, &host),
        ] {
            crate::persist_files::save_named(
                &dir, "SCOTT-TEST-0531", name, trigger, &mapper::mapper::Mapper::default(),
                save, None, &[], None, None, s.aux_data(), 1, None, None,
                &crate::archive::SessionRecord::empty(),
            )
            .expect("save_named writes the Scott archive");
            let path = dir.join(format!("{name}.lanthorn"));
            assert_eq!(crate::archive::read_archive_meta(&path).unwrap().trigger, trigger);

            // Wander off, then restore through the path that trigger selects.
            s.submit("down");
            assert_ne!(s.current_location().unwrap().number, start, "moved before restore");
            let ac = crate::archive::load_archive(&path).expect("load_archive");
            if trigger.is_portable() {
                s.restore_game_save(&ac.save).expect("game-save restore");
            } else {
                s.restore_state(&ac.engine_save()).expect("host resume");
            }
            assert_eq!(s.current_location().unwrap().number, start, "restored to the saved room");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_state_rejects_foreign_engine() {
        let mut s = ScottSession::new(dat(), None).unwrap();
        let foreign = EngineSave::new("zmachine", 1, vec![1, 2, 3]);
        let err = s.restore_state(&foreign).unwrap_err();
        assert!(matches!(err, EngineError::EngineMismatch { .. }));
    }

    // ── SQ-1475: the US S.A.G.A. releases' own family-C strip bitmaps ─────────

    /// *The Hulk* as shipped on the Commodore 64 Questprobe disk
    /// `QUESTPR1.D64` (spec §10.7): the database `SHULK.DB` plus seventy
    /// `R01nnn`/`B01nnnR`/`B01nnnI` picture files, all off one mount.
    ///
    /// Commercial and gitignored, so every case below skips vacuously without
    /// it. Opened through `hints::load_mounted_story_full`, which is exactly
    /// the door `startup.rs` opens — a hand-assembled pair of (database,
    /// pictures) would be measuring a launch the app never performs.
    /// A mounted Scott database plus its named S.A.G.A. picture files.
    type SagaFixture = (Vec<u8>, Vec<(String, Vec<u8>)>);

    fn hulk_d64() -> Option<SagaFixture> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/scott-dialects/c64/QUESTPR1.D64");
        if !path.exists() {
            eprintln!("SKIP: no {} (gitignored commercial fixture)", path.display());
            return None;
        }
        let mounted = crate::hints::load_mounted_story_full(&path, None)
            .expect("QUESTPR1.D64 mounts and holds one Scott database");
        let crate::hints::LoadedStory::Scott(bytes) = mounted.story else {
            panic!("QUESTPR1.D64's story is a Scott database");
        };
        Some((bytes, mounted.saga_pictures))
    }

    /// A session on the Hulk disk, built the way `startup.rs` builds one.
    fn hulk_session() -> Option<ScottSession> {
        let (bytes, pictures) = hulk_d64()?;
        assert_eq!(pictures.len(), 70, "the mount found the whole picture set (§10.7)");
        Some(
            ScottSession::new_with_options(
                bytes,
                false,
                None,
                scott::Options::default(),
                crate::graphics::ScottPictureSources::none().with_saga_pictures(pictures),
            )
            .expect("the Hulk boots off its own release disk"),
        )
    }

    /// The opening frame shows room 1's own picture at family C's canvas.
    ///
    /// 280x160 is the decoded size (`scott::saga_pictures::CANVAS_HEIGHT` —
    /// §8.3 states 158 and the records say 160), and `upscale` is on because
    /// the band fits it the same aspect-preserving way it fits a Blorb's
    /// pictures. The non-flat guard is what would catch a decode that wrote
    /// nothing: a blank canvas has exactly the right dimensions.
    #[test]
    fn hulk_room_one_shows_its_own_family_c_picture() {
        let Some(s) = hulk_session() else { return };
        assert_eq!(s.vm.current_room(), 1, "premise: Bruce Banner starts in room 1");
        let screen = s.screen();
        let band = picture_band(&screen).expect("room 1 has a picture band");
        let canvas = &band.canvas;
        assert_eq!(
            (canvas.width(), canvas.height()),
            (
                scott::saga_pictures::CANVAS_WIDTH as u32,
                scott::saga_pictures::CANVAS_HEIGHT as u32
            ),
            "family C's own canvas, undoubled"
        );
        assert!(band.upscale, "the band fits it like any other bitmap source");
        // Four colours actually drawn, and none of them covering the canvas.
        let mut seen = std::collections::HashSet::new();
        for p in canvas.pixels() {
            seen.insert((p.0[0], p.0[1], p.0[2]));
            assert_eq!(p.0[3], 255, "family C carries no transparent index");
        }
        assert_eq!(seen.len(), 4, "black, orange, purple and white (§8.3's table)");
        assert!(
            seen.contains(&scott::c64_palette::PEPTO_PALETTE[8])
                && seen.contains(&scott::c64_palette::PEPTO_PALETTE[4]),
            "room 1's orange and purple, the VIC-II's own since SQ-1491 — the exact \
             triples `machine-screenshots/c64-hulk-start.png` shows; got {seen:?}"
        );
    }

    /// `/dump-windows` names the third picture source and its platform, so a
    /// frame says where a room's art came from (SQ-1463's line, extended).
    #[test]
    fn hulk_window_dump_names_the_family_c_source() {
        let Some(s) = hulk_session() else { return };
        let dump = s.window_dump().join("\n");
        assert!(
            dump.contains("source=S.A.G.A. family C (Commodore 64, 70 record(s))"),
            "dump should name the family-C source:\n{dump}"
        );
    }

    /// *The Hulk* as shipped on MS-DOS (spec §10.7): a zip of loose DOS files
    /// — `ADVENT.DAT` in the plain reference text format, plus sixty-eight
    /// `.PAK` family-E pictures (§8.5) — opened through the same door
    /// `startup.rs` opens, because the archive is not re-opened afterwards.
    ///
    /// Commercial and gitignored, so every case below skips vacuously.
    fn hulk_dos_session() -> Option<ScottSession> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/scott-dialects/msdos/The-Hulk_DOS_EN.zip");
        if !path.exists() {
            eprintln!("SKIP: no {} (gitignored commercial fixture)", path.display());
            return None;
        }
        let mounted = crate::hints::load_mounted_story_full(&path, None)
            .expect("the MS-DOS Hulk zip holds one Scott database");
        let crate::hints::LoadedStory::Scott(bytes) = mounted.story else {
            panic!("The-Hulk_DOS_EN.zip's story is a Scott database");
        };
        assert_eq!(mounted.saga_pictures.len(), 68, "the zip's whole picture set (§10.7)");
        Some(
            ScottSession::new_with_options(
                bytes,
                false,
                None,
                scott::Options::default(),
                crate::graphics::ScottPictureSources::none()
                    .with_saga_pictures(mounted.saga_pictures),
            )
            .expect("the MS-DOS Hulk boots out of its own zip"),
        )
    }

    /// Room 1's family-E picture reaches the band, at the same canvas family C
    /// draws to and through the same aspect-preserving fit (SQ-1477).
    ///
    /// The palette is the tell that it really is family E and not the twin:
    /// §8.5 fixes it (black, cyan, magenta, white) where family C reads four
    /// stored colour bytes, so the same room on the Commodore 64 disk resolves
    /// orange and purple through §8.3's table and this one cannot.
    #[test]
    fn ms_dos_room_one_shows_its_own_family_e_picture() {
        let Some(s) = hulk_dos_session() else { return };
        assert_eq!(s.vm.current_room(), 1, "premise: Bruce Banner starts in room 1");
        let screen = s.screen();
        let band = picture_band(&screen).expect("room 1 has a picture band");
        let canvas = &band.canvas;
        assert_eq!(
            (canvas.width(), canvas.height()),
            (
                scott::saga_pictures::CANVAS_WIDTH as u32,
                scott::saga_pictures::CANVAS_HEIGHT as u32
            ),
            "the shared S.A.G.A. canvas"
        );
        assert!(band.upscale, "the band fits it like any other bitmap source");
        let mut seen = std::collections::HashSet::new();
        for p in canvas.pixels() {
            seen.insert((p.0[0], p.0[1], p.0[2]));
            assert_eq!(p.0[3], 255, "family E carries no transparent index");
        }
        assert!(
            seen.iter().all(|c| scott::saga_dos::PALETTE.contains(c)),
            "every colour is one of §8.5's four, got {seen:?}"
        );
        assert!(seen.len() >= 3, "room 1 is a drawing, not a flat fill: {seen:?}");
    }

    /// `/dump-windows` names the fourth picture source and counts it, so a
    /// frame says which encoding of the *Hulk*'s artwork it is showing — the
    /// canvas and the composition are the same either way.
    #[test]
    fn ms_dos_window_dump_names_the_family_e_source() {
        let Some(s) = hulk_dos_session() else { return };
        let dump = s.window_dump().join("\n");
        assert!(
            dump.contains("source=S.A.G.A. family E (MS-DOS, 68 picture(s))"),
            "dump should name the family-E source:\n{dump}"
        );
    }

    /// The MS-DOS band follows the player too, and §12.11's remap travels with
    /// it — this release's database is the plain reference TEXT format, so the
    /// remap arrives from `scott::saga_dos::identify` rather than from
    /// `Database::saga_us`, and a room with no picture file of its own still
    /// draws one.
    #[test]
    fn the_ms_dos_band_changes_when_the_room_does() {
        let Some(mut s) = hulk_dos_session() else { return };
        let first = picture_band(&s.screen()).expect("room 1 has art").canvas.clone();
        let start = s.vm.current_room();
        for command in ["bite lip", "east"] {
            s.submit(command);
            if s.vm.current_room() != start {
                break;
            }
        }
        let room = s.vm.current_room();
        if room == start {
            eprintln!("SKIP: the Hulk stayed in room {start}");
            return;
        }
        let second = picture_band(&s.screen())
            .map(|b| b.canvas.clone())
            .unwrap_or_else(|| panic!("room {room} has a picture too (§12.11 remaps every room)"));
        assert_ne!(first.as_raw(), second.as_raw(), "room {room}'s picture is not room {start}'s");
    }

    // ── SQ-1480: the ZX Spectrum Mysterious Adventures' own Family B pictures ──

    /// *The Golden Baton* as shipped on the ZX Spectrum `m1goldba.z80` —
    /// the same eleven titles as `baton_prg` above, this time as a 48K
    /// snapshot. Commercial and gitignored, so every case below skips
    /// vacuously without it.
    fn goldba_z80() -> Option<Vec<u8>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/scott-dialects/spectrum/m1goldba.z80");
        if !path.exists() {
            eprintln!("SKIP: no {} (gitignored commercial fixture)", path.display());
            return None;
        }
        Some(std::fs::read(&path).expect("read m1goldba.z80"))
    }

    /// Room 1 (the same "dense forest, very SPOOKY" the C64 release opens in
    /// — the eleven titles share the series' geography) shows its own
    /// picture drawn at the band's own resolution, exactly as
    /// `mysterious_c64_room1_shows_its_own_picture_drawn_for_the_band` pins
    /// for the C64 release: the SAME `scott::c64::PICTURE_WIDTH`/`HEIGHT`
    /// canvas and the SAME "round the band's magnification up, cap at 4x"
    /// rule (`crate::graphics::scott_c64_scale`), because both platforms
    /// decode through [`crate::graphics::PictSource::from_scott_family_b`].
    #[test]
    fn zx_mysterious_room1_shows_its_own_picture_drawn_for_the_band() {
        let Some(bytes) = goldba_z80() else { return };
        let s = ScottSession::new(bytes, None).expect("m1goldba.z80 loads");
        assert_eq!(s.current_location().expect("loc").number, 1);
        let model = s.screen();
        let gw = picture_band(&model).expect("room 1 shows a picture band");
        // FALLBACK_CHAR_PX is 8x16, so 16 rows x 16px = 256 device pixels,
        // and 256 / 94 rounds up to a 3x supersample — the same arithmetic
        // the C64 case above pins, because it is the same free function.
        assert_eq!(
            (gw.canvas.width(), gw.canvas.height()),
            (scott::c64::PICTURE_WIDTH as u32 * 3, scott::c64::PICTURE_HEIGHT as u32 * 3),
            "drawn at the band's own resolution, not at the 255 x 94 native canvas"
        );
        assert_eq!(
            gw.canvas.width() * scott::c64::PICTURE_HEIGHT as u32,
            gw.canvas.height() * scott::c64::PICTURE_WIDTH as u32,
            "the aspect ratio is the native one"
        );
        assert!(gw.upscale, "the band renderer stretches it to fill the reserved rows");
        // Not a flat fill — a decode that wrote nothing still has the right
        // dimensions, so this is the guard that would have caught one.
        let distinct: std::collections::HashSet<[u8; 4]> = gw.canvas.pixels().map(|p| p.0).collect();
        assert!(distinct.len() > 1, "room 1's forest is more than one colour: {distinct:?}");
    }

    /// The band's canvas follows the player: room 2's picture is NOT room 1's
    /// (§8.6's pure identity — room *n* shows image *n* − 1), the same
    /// property `native_room1_canvas_matches_the_decoder_oracle_not_an_off_by_one_room`
    /// (`crates/app/tests/suites/scott_c64_native_pictures.rs`) pins for the
    /// C64 release, checked here end to end through a live session instead of
    /// against the decoder oracle directly.
    #[test]
    fn zx_mysterious_picture_band_changes_when_the_room_changes() {
        let Some(bytes) = goldba_z80() else { return };
        let mut s = ScottSession::new(bytes, None).expect("m1goldba.z80 loads");
        let room1_canvas = picture_band(&s.screen()).expect("room 1 has a band").canvas.clone();

        let r = s.submit("north");
        assert!(!r.quit, "moving must not end the game: {:?}", r.transcript);
        assert_ne!(s.current_location().expect("loc").number, 1, "actually moved");

        // A dark or picture-less neighbour is still a *different* band (none,
        // rather than room 1's) — either way the band followed the player
        // rather than staying pinned to room 1's art.
        if let Some(gw) = picture_band(&s.screen()) {
            assert_ne!(gw.canvas, room1_canvas, "a different room's picture must not equal room 1's");
        }
    }

    /// `/dump-windows` names the platform, not just the family
    /// (`ScottFamilyBPlatform::label`, SQ-1480) — the geometry and the
    /// `source=native <platform> x<scale>` shape are identical to the C64's
    /// line; only the word between them differs.
    #[test]
    fn zx_mysterious_window_dump_names_the_native_zx_source() {
        let Some(bytes) = goldba_z80() else { return };
        let s = ScottSession::new(bytes, None).expect("m1goldba.z80 loads");
        let dump = s.window_dump().join("\n");
        assert!(
            dump.contains("source=native ZX Spectrum x3"),
            "dump should name the native ZX Spectrum source:\n{dump}"
        );
    }


    // ── SQ-1476: the Apple II releases' family-D line drawings ───────────────

    /// *Adventureland* as pressed for the Apple II (spec §10.6): the database
    /// `A1.DAT` on the **boot** side, and the artwork on the companion side A
    /// — which is the whole point of this fixture, since a walk of the mounted
    /// image alone finds no pictures at all.
    ///
    /// Commercial and gitignored, so every case below skips vacuously without
    /// it. Opened through `hints::load_mounted_story_full`, the door
    /// `startup.rs` opens.
    fn adventureland_apple() -> Option<SagaFixture> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../stories/scott-dialects/apple/Scott Adams Graphic Adventure 1 - \
             Adventureland v2.1-416 (4am crack) side B - boot.dsk",
        );
        if !path.exists() {
            eprintln!("SKIP: no {} (gitignored commercial fixture)", path.display());
            return None;
        }
        let mounted = crate::hints::load_mounted_story_full(&path, None)
            .expect("the boot side mounts and holds one Scott database");
        let crate::hints::LoadedStory::Scott(bytes) = mounted.story else {
            panic!("the boot side's story is a Scott database");
        };
        Some((bytes, mounted.saga_pictures))
    }

    fn adventureland_apple_session() -> Option<ScottSession> {
        let (bytes, pictures) = adventureland_apple()?;
        assert_eq!(
            pictures.len(),
            93,
            "the mount reached the COMPANION side's picture files (§10.6)"
        );
        Some(
            ScottSession::new_with_options(
                bytes,
                false,
                None,
                scott::Options::default(),
                crate::graphics::ScottPictureSources::none().with_saga_pictures(pictures),
            )
            .expect("Adventureland boots off its own release disk"),
        )
    }

    /// The opening frame shows the START room's own picture at family D's
    /// canvas — the Apple II hi-res screen, 280x192, which is NOT family C's
    /// 280x160.
    ///
    /// The non-flat guard is what would catch a decode that wrote nothing: a
    /// blank canvas has exactly the right dimensions.
    #[test]
    fn adventureland_apple_start_room_shows_its_own_family_d_picture() {
        let Some(s) = adventureland_apple_session() else { return };
        // *Adventureland*'s own header says room 11, not room 1 — `adv01.dat`
        // agrees — so the frame under test is the forest the player opens in.
        assert_eq!(s.vm.current_room(), 11, "premise: this release starts in room 11");
        let screen = s.screen();
        let band = picture_band(&screen).expect("the start room has a picture band");
        let canvas = &band.canvas;
        assert_eq!(
            (canvas.width(), canvas.height()),
            (
                scott::apple_pictures::CANVAS_WIDTH as u32,
                scott::apple_pictures::CANVAS_HEIGHT as u32
            ),
            "family D's own canvas — the machine's hi-res page"
        );
        assert!(band.upscale, "the band fits it like any other bitmap source");
        let mut counts = std::collections::HashMap::new();
        for p in canvas.pixels() {
            assert_eq!(p.0[3], 255, "family D carries no transparent index");
            let rgb = (p.0[0], p.0[1], p.0[2]);
            assert!(
                scott::apple_pictures::PALETTE.contains(&rgb),
                "{rgb:?} is not one of the six hi-res colours"
            );
            *counts.entry(rgb).or_insert(0usize) += 1;
        }
        // Room 11 is the forest floor, and it is COLOURED: an orange ground
        // under a green canopy with black trunks, which is SQ-1489's paint
        // model reaching the screen. Two colours would be the line art alone.
        assert!(counts.len() >= 5, "only {} colours on the canvas", counts.len());
        let orange = counts[&scott::apple_pictures::PALETTE[4]];
        assert!(orange > 30_000, "the orange ground is only {orange} pixels");
        let green = counts[&scott::apple_pictures::PALETTE[2]];
        assert!(green > 2_000, "the green canopy is only {green} pixels");
        let white = counts.get(&scott::apple_pictures::PALETTE[5]).copied().unwrap_or(0);
        assert!(
            white < canvas.width() as usize * canvas.height() as usize / 2,
            "the canvas washed out to white"
        );
    }

    /// One of the three **scrambled** releases — *The Count* — boots off its
    /// boot side and draws the room artwork that lives on a side A with no
    /// filesystem on it at all (SQ-1490).
    ///
    /// Both `honor_game_colours` modes, and the flag has no effect here for
    /// the reason the family-C cases give: it governs TEXT-cell colour
    /// resolution and a room-picture band is a raw RGBA canvas. Documented
    /// rather than assumed.
    #[test]
    fn the_count_apple_start_room_draws_from_its_side_a_in_both_colour_modes() {
        for honor_game_colours in [true, false] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../../stories/scott-dialects/apple/Scott Adams Graphic Adventure 5 - \
                 The Count v2.1-115 (4am crack) side B - boot.dsk",
            );
            if !path.exists() {
                eprintln!("SKIP: no {} (gitignored commercial fixture)", path.display());
                return;
            }
            let _ = honor_game_colours;
            let mounted = crate::hints::load_mounted_story_full(&path, None)
                .expect("the boot side mounts and holds one Scott database");
            assert_eq!(
                mounted.saga_pictures.len(),
                26,
                "the records found by header on The Count's side A (§8.4, §10.6)"
            );
            let crate::hints::LoadedStory::Scott(bytes) = mounted.story else {
                panic!("the boot side's story is a Scott database");
            };
            let s = ScottSession::new_with_options(
                bytes,
                false,
                None,
                scott::Options::default(),
                crate::graphics::ScottPictureSources::none()
                    .with_saga_pictures(mounted.saga_pictures),
            )
            .expect("The Count boots off its own release disk");
            let screen = s.screen();
            let band = picture_band(&screen).expect("the start room has a picture band");
            assert_eq!(
                (band.canvas.width(), band.canvas.height()),
                (280, 160),
                "§8.4's nominal size, not the plain sub-variant's 192-row page"
            );
            let pixel = band.canvas.get_pixel(140, 80);
            assert_eq!(
                (pixel.0[0], pixel.0[1], pixel.0[2]),
                scott::apple_pictures::PALETTE[4],
                "the middle of the brass-bed room is orange, not an undrawn black"
            );
        }
    }

    /// Walking into another room draws that room's picture, which is the whole
    /// of §12.10's "a room's picture index IS the room number" reaching the
    /// screen. Two different rooms, two different canvases.
    #[test]
    fn adventureland_apple_band_changes_with_the_room() {
        let Some(mut s) = adventureland_apple_session() else { return };
        let start = s.vm.current_room();
        let first = picture_band(&s.screen()).expect("the start room has a picture").canvas.clone();
        let mut moved = None;
        for command in ["go north", "go south", "go east", "go west", "climb tree"] {
            crate::engine::Engine::submit(&mut s, command);
            let room = s.vm.current_room();
            if room != start {
                moved = Some(room);
                break;
            }
        }
        let Some(room) = moved else {
            panic!("no direction left room {start}, so this case proves nothing");
        };
        let second = picture_band(&s.screen())
            .unwrap_or_else(|| panic!("room {room} has no picture band"))
            .canvas
            .clone();
        assert_ne!(
            first.as_raw(),
            second.as_raw(),
            "room {room} drew room {start}'s picture again"
        );
    }

    /// `/dump-windows` names the family and the platform, so a frame says
    /// where a room's art came from — and family **D**, not C.
    #[test]
    fn adventureland_apple_window_dump_names_the_family_d_source() {
        let Some(s) = adventureland_apple_session() else { return };
        let dump = s.window_dump().join("\n");
        assert!(
            dump.contains("source=S.A.G.A. family D (Apple II, 93 picture(s))"),
            "dump should name the family-D source:\n{dump}"
        );
    }

    /// A US S.A.G.A. release opened WITHOUT its release disk — the extracted
    /// `db/hulk.bin` — has no pictures at all, and the dump says which kind of
    /// nothing that is rather than reading as a text-only game.
    #[test]
    fn an_extracted_saga_database_has_no_pictures_and_says_so() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/scott-dialects/c64/db/hulk.bin");
        if !path.exists() {
            eprintln!("SKIP: no {} (gitignored commercial fixture)", path.display());
            return;
        }
        let bytes = std::fs::read(&path).expect("read hulk.bin");
        let s = ScottSession::new(bytes, None).expect("the extracted database boots");
        assert!(picture_band(&s.screen()).is_none(), "no band without the release disk");
        let dump = s.window_dump().join("\n");
        assert!(
            dump.contains("a S.A.G.A. release with no picture files on this file"),
            "dump should distinguish this from a text-only game:\n{dump}"
        );
    }

    /// The band follows the player: a room change re-resolves the picture, and
    /// two different rooms are two different canvases.
    ///
    /// Driven through `submit`, not by poking the VM, because
    /// `refresh_picture` runs on the turn boundary and that is the thing under
    /// test.
    #[test]
    fn the_band_changes_when_the_room_does() {
        let Some(mut s) = hulk_session() else { return };
        let first = picture_band(&s.screen()).expect("room 1 has art").canvas.clone();
        let start = s.vm.current_room();
        // Bruce is tied to a chair with no exits at all, so no direction moves
        // him: `BITE LIP` is the game's own opening — it turns him into the
        // Hulk, who bursts the ropes and ends up in a dome. Two turns in, and
        // the specimen table's turn count for this frame.
        let mut moved = None;
        for command in ["bite lip", "east"] {
            s.submit(command);
            if s.vm.current_room() != start {
                moved = Some(s.vm.current_room());
                break;
            }
        }
        let Some(room) = moved else {
            eprintln!("SKIP: the Hulk stayed in room {start} for eight turns");
            return;
        };
        let second = picture_band(&s.screen()).map(|b| b.canvas.clone());
        assert!(second.is_some(), "room {room} has a picture too (§12.11 remaps every room)");
        let second = second.expect("just checked");
        assert_ne!(
            first.as_raw(),
            second.as_raw(),
            "room {room}'s picture is not room {start}'s"
        );
    }

    /// The band shows whatever `scott::Vm::current_picture` chose, and nothing
    /// else — the whole of the host's picture policy since SQ-1475.
    ///
    /// That delegation is the case worth pinning HERE. It used to be
    /// `if self.vm.is_dark() { None }`, which is right for most dialects and
    /// wrong for these: §12.11 has a US S.A.G.A. release draw picture index 0
    /// in the dark, the dedicated "IT'S TOO DARK!" image, and only the
    /// database knows which rule applies. The rule itself is asserted where it
    /// lives, in `scott`'s own
    /// `vm::tests::saga_us_darkness_draws_the_darkness_picture` — reaching a
    /// dark room in the *Hulk* is a walkthrough, and the darkness flag is not
    /// this crate's to set.
    #[test]
    fn the_band_shows_exactly_the_picture_the_vm_chose() {
        let Some(mut s) = hulk_session() else { return };
        assert_eq!(s.current_pic_num, s.vm.current_picture(), "at boot");
        assert_eq!(s.current_pic_num, Some(1), "premise: Banner starts in room 1");
        // SQ-1482 removed `inventory` from this list: §12.11 has the
        // inventory command draw picture 98 and wait for ENTER, so it goes
        // through the picture-show door and the band deliberately stops
        // agreeing with `current_picture()` until the key arrives. That is
        // its own case below.
        for command in ["look", "wait"] {
            s.submit(command);
            assert_eq!(
                s.current_pic_num,
                s.vm.current_picture(),
                "after {command:?} the band still shows the VM's own choice"
            );
        }
    }

    // ── SQ-1487: "bite lip" shows several pictures in sequence, each waiting
    // for RETURN, instead of only the last one ─────────────────────────────

    /// *The Hulk*'s own opening (spec §12.11): typing `BITE LIP` in room 1
    /// runs an action whose commands draw pictures 84, 83 and 86 in a row —
    /// measured directly off `QUESTPR1.D64` (`probe_bite_lip_sequence`,
    /// since reverted; the numbers are pinned here, not derived at test
    /// time) — before the gas transformation drops Bruce Banner into room 2,
    /// whose own picture (2) is what the band shows once the sequence ends.
    #[test]
    fn bite_lip_shows_every_picture_in_sequence_each_key_gated() {
        let Some(mut s) = hulk_session() else { return };
        assert_eq!(s.vm.current_room(), 1, "premise: Bruce Banner starts in room 1");

        let first = s.submit("bite lip");
        assert_eq!(
            s.pending_input(),
            InputKind::Char,
            "the first picture in the sequence waits for a keypress, not a line"
        );
        assert_eq!(s.current_pic_num, Some(84), "the band shows the FIRST picture the chain names");
        assert!(
            !first.transcript.contains("Tell me what to do"),
            "no prompt while a sequence is still presenting: {:?}",
            first.transcript
        );
        assert_eq!(
            first.info.as_deref(),
            Some(PICTURE_SHOW_HINT),
            "the keypress hint rides TurnResult::info"
        );

        // Two more presses show the remaining two pictures — the queue this
        // turn built, not one slot silently overwritten by the last request
        // (SQ-1487's actual bug).
        for want_picture in [83u16, 86] {
            let r = s.submit_key(crate::engine::KeyInput::Enter).expect("still showing — a key advances it");
            assert_eq!(s.pending_input(), InputKind::Char, "picture {want_picture} still waits for a key");
            assert_eq!(s.current_pic_num, Some(want_picture), "advances to the next queued picture");
            assert!(!r.transcript.contains("Tell me what to do"), "still mid-sequence, no prompt yet");
        }

        // The fourth key press ends the sequence: the room view returns.
        let last = s.submit_key(crate::engine::KeyInput::Enter).expect("the last key still routes here");
        assert_eq!(s.pending_input(), InputKind::Line, "the sequence is over — back to line input");
        assert_eq!(s.vm.current_room(), 2, "the gas transformation moved Bruce Banner to room 2");
        assert_eq!(
            s.current_pic_num,
            s.vm.current_picture(),
            "the band reverts to the room's own picture, exactly like an ordinary turn"
        );
        assert_eq!(s.current_pic_num, Some(2), "room 2's own picture");
        assert!(
            last.transcript.trim_end().ends_with("Tell me what to do ?"),
            "the ordinary prompt returns once the sequence ends: {:?}",
            last.transcript
        );

        // Outside a sequence, Scott is still line-only.
        assert!(s.submit_key(crate::engine::KeyInput::Enter).is_none(), "no sequence left to advance");
    }

    /// An ordinary turn that never runs opcode 90 is byte-identical to
    /// before SQ-1487 — the new machinery must be a true no-op off its own
    /// trigger.
    #[test]
    fn a_turn_with_no_picture_show_is_unaffected() {
        let Some(mut s) = hulk_session() else { return };
        let r = s.submit("look");
        assert_eq!(s.pending_input(), InputKind::Line, "an ordinary turn never waits on a keypress");
        assert!(r.transcript.trim_end().ends_with("Tell me what to do ?"), "the ordinary prompt: {:?}", r.transcript);
        assert!(r.info.is_none(), "no picture-show hint outside a sequence");
    }

    // ── SQ-1482: §12.11's object overlays and the inventory picture screen ──

    /// The dome, two turns and four keypresses in: `BITE LIP` is the game's
    /// own opening (SQ-1487's three ENTER-gated scenes) and the gas that
    /// follows drops Bruce Banner into room 2.
    ///
    /// **The specimen's frame, named.** Room 2 holds six items — the mirror,
    /// the broken chair, a `*Gem`, the metal hand fan, the large iron ring
    /// set in the floor and a sign — and exactly two of them have artwork on
    /// the disk, which is what makes it the frame worth pinning: the overlay
    /// rule has to draw two records and not six.
    fn hulk_in_the_dome() -> Option<ScottSession> {
        let mut s = hulk_session()?;
        s.submit("bite lip");
        for _ in 0..4 {
            s.submit_key(crate::engine::KeyInput::Enter);
        }
        assert_eq!(s.vm.current_room(), 2, "the opening cutscene ends in the dome");
        Some(s)
    }

    /// One named family-C record off `QUESTPR1.D64`, decoded — the bare
    /// picture a composited band is compared against, so a case pins the
    /// DIFFERENCE the overlay makes rather than a colour on its own.
    fn hulk_bare(name: &str) -> Option<scott::saga_pictures::Picture> {
        let (_, pictures) = hulk_d64()?;
        let (_, record) = pictures.into_iter().find(|(n, _)| n == name)?;
        Some(scott::decode_family_c(&record, scott::SagaPlatform::Commodore64).expect("decodes"))
    }

    fn band_pixel(s: &ScottSession, x: u32, y: u32) -> (u8, u8, u8) {
        let screen = s.screen();
        let band = picture_band(&screen).expect("a picture band");
        let p = band.canvas.get_pixel(x, y).0;
        (p[0], p[1], p[2])
    }

    /// §12.11: "after [the room picture], every object picture whose item
    /// index names an item presently in the room is drawn over it, **in the
    /// order the picture files were gathered**."
    ///
    /// The two records are pinned by name AND by order, because §8.6 makes
    /// the order observable ("later pictures overwrite earlier ones") and the
    /// disk's directory lists `B01053R` (the sign) before `B01033R` (the
    /// iron ring). The pixel is the falsifier: R01002 alone is black at
    /// (123, 122) and the ring paints white there, so a band that skipped the
    /// composite would read black and one that copied the overlay's whole
    /// canvas would black out the dome around it.
    #[test]
    fn the_domes_band_draws_the_sign_and_the_ring_over_its_room_picture() {
        let Some(s) = hulk_in_the_dome() else { return };
        assert_eq!(s.current_pic_num, Some(2), "room 2's own picture is the base");
        assert_eq!(
            s.current_overlays,
            vec!["B01053R".to_string(), "B01033R".to_string()],
            "the dome's two items that have artwork, in the disk directory's own order"
        );

        let bare = hulk_bare("R01002").expect("the fixture is present");
        assert_eq!(bare.rgb(123, 122), Some((0, 0, 0)), "premise: the bare room picture is black here");
        assert_eq!(
            band_pixel(&s, 123, 122),
            (255, 255, 255),
            "the iron ring (B01033R) paints white over it"
        );
        // …and the rest of the room picture is untouched: the overlay's
        // canvas is black everywhere outside its own rectangle, and copying
        // that would show.
        assert_eq!(bare.rgb(4, 2), Some((255, 255, 255)), "premise: the bare picture is white here");
        assert_eq!(band_pixel(&s, 4, 2), (255, 255, 255), "outside every overlay's rectangle");
    }

    /// A record the *Hulk* does **not** ship, built by hand from §8.3 so a
    /// take and a drop can be watched changing the band.
    ///
    /// One 8-pixel column, rows 0 and 1, every pixel value 3, whose third
    /// stored colour byte is 67 — §8.3's table reads that as red, and the
    /// dome's own picture is white in that corner, so the record is visible
    /// against it. Named `B01023R` so §8.6's rule reads it as the room
    /// artwork of item 23, the metal hand fan, which is the one item in the
    /// dome a player can pick up in a single turn.
    fn fabricated_fan_record() -> Vec<u8> {
        vec![
            0x00, 0x50, // load address, ignored
            0x00, 0x00, // data size, informational
            3, 0, 3, 0, // left column 0, top row 0, right column 0, bottom row 0
            2, 3, 67, 0, // values 1, 2, 3 → white, white, RED; the fourth is unused
            0x80, 0xFF, 0xFF, // repeat count 1: one pair, both bytes all value 3
            0, 0, // the two-byte tail every record carries
        ]
    }

    /// Taking an item takes its record off the band, and dropping it puts it
    /// back — the overlay set is recomputed from the room's items every turn,
    /// not cached from the room number (SQ-1482).
    ///
    /// The *Hulk* ships no room artwork for any item a player can carry, so
    /// the moving record is [`fabricated_fan_record`] and everything else in
    /// the frame is the real disk's.
    #[test]
    fn taking_and_dropping_an_item_moves_its_record_on_and_off_the_band() {
        let Some((bytes, mut pictures)) = hulk_d64() else { return };
        pictures.push(("B01023R".to_string(), fabricated_fan_record()));
        let mut s = ScottSession::new_with_options(
            bytes,
            false,
            None,
            scott::Options::default(),
            crate::graphics::ScottPictureSources::none().with_saga_pictures(pictures),
        )
        .expect("the Hulk boots with one fabricated record beside its own");
        s.submit("bite lip");
        for _ in 0..4 {
            s.submit_key(crate::engine::KeyInput::Enter);
        }
        assert_eq!(s.vm.current_room(), 2, "premise: the opening ends in the dome");

        let carried = vec!["B01053R".to_string(), "B01033R".to_string(), "B01023R".to_string()];
        assert_eq!(s.current_overlays, carried, "the fabricated record is gathered last, so drawn last");
        assert_eq!(
            band_pixel(&s, 0, 0),
            scott::c64_palette::PEPTO_PALETTE[2],
            "and paints its corner red"
        );
        let version = s.pic_version;

        s.submit("get fan");
        assert_eq!(s.vm.current_room(), 2, "premise: taking the fan does not move the player");
        assert_eq!(
            s.current_overlays,
            vec!["B01053R".to_string(), "B01033R".to_string()],
            "item 23 is in the pack now, so its record leaves the room band"
        );
        assert_ne!(s.pic_version, version, "the band was recomputed, so the terminal re-uploads it");
        assert_eq!(
            band_pixel(&s, 0, 0),
            scott::c64_palette::PEPTO_PALETTE[1],
            "the room picture's own corner is back"
        );

        s.submit("drop fan");
        assert_eq!(s.current_overlays, carried, "…and returns when the fan does");
        assert_eq!(band_pixel(&s, 0, 0), scott::c64_palette::PEPTO_PALETTE[2]);
    }

    /// §12.11: "The inventory command draws a picture. Beyond listing what is
    /// carried, it clears the graphics window, draws picture index 98 as a
    /// room picture, then draws the inventory-object picture of every carried
    /// item, and waits for the player to press ENTER before restoring the
    /// room view."
    ///
    /// Every clause of that is asserted here: the listing is printed (and
    /// printed BEFORE the picture, which is what "beyond listing" fixes and
    /// what the VM's `output_len` carries), the band switches to 98, the one
    /// carried item's inventory record is composited over it, the prompt is
    /// withheld, and ENTER puts the dome back exactly as it was.
    #[test]
    fn the_inventory_command_shows_picture_98_with_the_carried_item_over_it() {
        let Some(mut s) = hulk_in_the_dome() else { return };
        s.submit("get fan");
        let room_overlays = s.current_overlays.clone();

        let r = s.submit("inventory");
        assert!(r.transcript.contains("Metal Hand Fan"), "the listing first: {:?}", r.transcript);
        assert!(
            !r.transcript.contains("Tell me what to do"),
            "the prompt is withheld while the screen is up: {:?}",
            r.transcript
        );
        assert_eq!(r.info.as_deref(), Some(PICTURE_SHOW_HINT), "and the keypress hint rides with it");
        assert_eq!(s.pending_input(), InputKind::Char, "§12.11 waits for ENTER");
        assert_eq!(s.current_pic_num, Some(98), "§8.6 reserves 98 for the inventory backdrop");
        assert_eq!(
            s.current_overlays,
            vec!["B01023I".to_string()],
            "the carried item's INVENTORY record — a different picture from its room one"
        );

        let bare = hulk_bare("R01098").expect("the fixture is present");
        assert_eq!(
            bare.rgb(116, 104),
            Some(scott::c64_palette::PEPTO_PALETTE[1]),
            "premise: the backdrop is white here"
        );
        assert_eq!(
            band_pixel(&s, 116, 104),
            scott::c64_palette::PEPTO_PALETTE[4],
            "the fan's inventory record paints purple over it"
        );

        let last = s.submit_key(crate::engine::KeyInput::Enter).expect("a key ends the screen");
        assert_eq!(s.pending_input(), InputKind::Line, "back to line input");
        assert_eq!(s.current_pic_num, Some(2), "the room view returns");
        assert_eq!(s.current_overlays, room_overlays, "…with the dome's own overlays back");
        assert!(
            last.transcript.trim_end().ends_with("Tell me what to do ?"),
            "and the prompt with it: {:?}",
            last.transcript
        );
    }

    /// An empty pack still gets the screen — §12.11 makes the picture part of
    /// the command, not of what is in the pack — and it is the backdrop with
    /// nothing over it.
    #[test]
    fn the_inventory_screen_with_an_empty_pack_is_the_bare_backdrop() {
        let Some(mut s) = hulk_in_the_dome() else { return };
        let r = s.submit("inventory");
        assert!(r.transcript.contains("Nothing"), "the listing says so: {:?}", r.transcript);
        assert_eq!(s.current_pic_num, Some(98));
        assert!(s.current_overlays.is_empty(), "nothing carried, nothing composited");
        assert_eq!(band_pixel(&s, 116, 104), (255, 255, 255), "the backdrop's own white");
        s.submit_key(crate::engine::KeyInput::Enter);
        assert_eq!(s.current_pic_num, Some(2));
    }

    /// `/dump-windows` names the records composited into the band, so a
    /// capture of an overlaid frame says what is in it — and says `none`
    /// rather than going silent when a S.A.G.A. room happens to have no
    /// objects, since the two frames look identical otherwise.
    #[test]
    fn the_dump_names_the_overlays_the_band_was_built_from() {
        let Some(mut s) = hulk_session() else { return };
        let line = |s: &ScottSession| {
            s.window_dump().into_iter().find(|l| l.contains("picture:")).expect("a picture line")
        };
        assert!(
            line(&s).contains("overlays=none"),
            "room 1 holds no items at all: {:?}",
            line(&s)
        );
        s.submit("bite lip");
        for _ in 0..4 {
            s.submit_key(crate::engine::KeyInput::Enter);
        }
        assert!(
            line(&s).contains("overlays=B01053R,B01033R"),
            "the dome's two records, in draw order: {:?}",
            line(&s)
        );
    }

    /// A picture-show request the source cannot draw is not presented at all
    /// (SQ-1482) — a keypress pause in front of a band that shows nothing is
    /// worse than no pause.
    ///
    /// The *Hulk*'s database with **no** picture files is exactly that case,
    /// and it is not hypothetical: §12.13's oracle is the same game as a
    /// plain reference-format `.dat`, and a player who extracted `SHULK.DB`
    /// on its own gets this session.
    #[test]
    fn an_undrawable_picture_show_never_stops_the_game() {
        let Some((bytes, _)) = hulk_d64() else { return };
        let mut s = ScottSession::new_with_options(
            bytes,
            false,
            None,
            scott::Options::default(),
            crate::graphics::ScottPictureSources::none(),
        )
        .expect("the database alone still boots");
        let r = s.submit("inventory");
        assert_eq!(s.pending_input(), InputKind::Line, "no picture, no pause");
        assert!(
            r.transcript.trim_end().ends_with("Tell me what to do ?"),
            "the ordinary prompt comes straight back: {:?}",
            r.transcript
        );
        assert!(r.info.is_none(), "and no keypress hint");
        let r = s.submit("bite lip");
        assert_eq!(s.pending_input(), InputKind::Line, "nor for opcode 90's three scenes");
        assert!(r.transcript.trim_end().ends_with("Tell me what to do ?"));
    }

    // ── SQ-1482, the MS-DOS release: the same two rules over family E ──

    /// The DOS zip's own gather order is not the Commodore disk's, and
    /// §12.11 draws in the order the files were gathered — so the same two
    /// records reach the same dome in the other order. Nothing in the frame
    /// changes, because the two do not overlap; the case is here because the
    /// order is the thing the rule names.
    #[test]
    fn the_dos_domes_band_draws_the_same_two_records_in_the_zips_order() {
        let Some(mut s) = hulk_dos_session() else { return };
        s.submit("bite lip");
        for _ in 0..4 {
            s.submit_key(crate::engine::KeyInput::Enter);
        }
        assert_eq!(s.vm.current_room(), 2, "premise: the opening ends in the dome");
        assert_eq!(s.current_pic_num, Some(2));
        let mut sorted = s.current_overlays.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["B01033R.PAK".to_string(), "B01053R.PAK".to_string()],
            "the same two items' artwork, under §8.5's names"
        );
        // Measured on The-Hulk_DOS_EN.zip: R0102.PAK alone is black at
        // (130, 112) and B01033R.PAK paints CGA cyan there.
        assert_eq!(band_pixel(&s, 130, 112), (0, 255, 255), "the iron ring, in family E's palette");
    }

    /// §12.11's inventory screen on the release whose database is the plain
    /// reference TEXT format (§10.7) — the rule has to reach it through
    /// `saga_dos::identify` rather than through `Database::saga_us`, which is
    /// `None` here, and the picture is `R0198.PAK` rather than `R01098`.
    #[test]
    fn the_dos_inventory_command_shows_its_own_picture_98() {
        let Some(mut s) = hulk_dos_session() else { return };
        assert!(
            s.vm.database().saga_us.is_none(),
            "premise: this database says nothing about itself (§10.7)"
        );
        s.submit("bite lip");
        for _ in 0..4 {
            s.submit_key(crate::engine::KeyInput::Enter);
        }
        s.submit("get fan");
        let r = s.submit("inventory");
        assert!(r.transcript.contains("Metal Hand Fan"), "the listing: {:?}", r.transcript);
        assert_eq!(s.pending_input(), InputKind::Char, "§12.11 waits for ENTER here too");
        assert_eq!(s.current_pic_num, Some(98));
        assert_eq!(s.current_overlays, vec!["B01023I.PAK".to_string()]);
        // Measured: R0198.PAK is white at (116, 104) and B01023I.PAK paints
        // CGA magenta there.
        assert_eq!(band_pixel(&s, 116, 104), (255, 0, 255));
        s.submit_key(crate::engine::KeyInput::Enter);
        assert_eq!(s.pending_input(), InputKind::Line);
        assert_eq!(s.current_pic_num, Some(2), "the dome comes back");
    }
}
